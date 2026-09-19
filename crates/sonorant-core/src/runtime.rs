//! The analysis thread and the queues around it.
//!
//! ```text
//! capture thread --f32 frames--> [rtrb] --> analysis thread --rows--> [rtrb] --> render
//!                                                    \--latest state--> [triple buffer] --> render
//! ```
//!
//! Data only flows forward and nobody waits on anybody: the capture side drops audio if
//! the ring is full rather than block, the analysis side drops rows if the renderer has
//! stopped reading, and the renderer always takes whatever state was published last.
//! Settings reach the analysis thread as commands, the only time it allocates.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use half::f16;

use crate::dsp::LoudnessReadings;
use crate::engine::{AnalysisConfig, Engine, GRID_BINS, Hop, Row, Sink};

/// Frames of audio the capture ring holds: about two seconds at 48 kHz, so a stalled
/// analysis thread has time to catch up before anything is lost.
const AUDIO_RING_FRAMES: usize = 1 << 17;
/// Rows the renderer may fall behind by before rows are dropped: four seconds at 60.
const ROW_RING: usize = 256;
/// Frames of recent audio in each snapshot, for the goniometer and waveform.
pub const SCOPE_FRAMES: usize = 4096;
/// How long the analysis thread sleeps when there's no audio to read.
const IDLE_SLEEP: Duration = Duration::from_millis(1);

/// The capture side: pushes interleaved stereo frames without ever blocking.
#[derive(Debug)]
pub struct AudioInput {
    ring: rtrb::Producer<f32>,
    dropped: Arc<AtomicU64>,
}

impl AudioInput {
    /// Copies interleaved stereo frames into the ring. Safe on a real-time thread: no
    /// allocation, locks or system calls. Frames that don't fit are dropped and counted.
    pub fn push_interleaved(&mut self, frames: &[f32]) {
        let wanted = frames.len() & !1;
        let space = self.ring.slots() & !1;
        let n = wanted.min(space);
        if n > 0
            && let Ok(chunk) = self.ring.write_chunk_uninit(n)
        {
            chunk.fill_from_iter(frames[..n].iter().copied());
        }
        if n < wanted {
            self.dropped
                .fetch_add(((wanted - n) / 2) as u64, Ordering::Relaxed);
        }
    }

    /// Copies as many whole frames as fit and returns how many samples that was. For
    /// sources that can wait, such as a file played faster than real time.
    pub fn push_some(&mut self, frames: &[f32]) -> usize {
        let n = (frames.len() & !1).min(self.ring.slots() & !1);
        if n > 0
            && let Ok(chunk) = self.ring.write_chunk_uninit(n)
        {
            chunk.fill_from_iter(frames[..n].iter().copied());
        }
        n
    }
}

/// One history row, ready for the GPU.
#[derive(Clone, Debug)]
pub struct RowMsg {
    pub index: u64,
    /// The row's timestamp in frames of audio.
    pub frames: u64,
    pub floor_db: f32,
    pub ceiling_db: f32,
    /// Waveform extremes since the previous row: left min, left max, right min, right max.
    pub wave: [f32; 4],
    /// Levels on the fixed grid, low to high frequency.
    pub a: [f16; GRID_BINS],
    pub b: [f16; GRID_BINS],
}

impl RowMsg {
    fn empty() -> RowMsg {
        RowMsg {
            index: 0,
            frames: 0,
            floor_db: 0.0,
            ceiling_db: 0.0,
            wave: [0.0; 4],
            a: [f16::ZERO; GRID_BINS],
            b: [f16::ZERO; GRID_BINS],
        }
    }
}

/// One pane's curves, as `f32` for drawing.
#[derive(Clone, Debug, Default)]
pub struct PaneCurves {
    pub raw: Vec<f32>,
    pub display: Vec<f32>,
    pub max: Vec<f32>,
    pub min: Vec<f32>,
    pub average: Vec<f32>,
}

/// The newest analysis state.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub hop: u64,
    /// Frames of audio analysed: the audio clock.
    pub frames: u64,
    pub sample_rate: f64,
    pub analysed: bool,
    /// What the last hop's analysis took, in seconds, smoothed over a second.
    pub analysis_seconds: f64,
    pub floor_db: f64,
    pub ceiling_db: f64,
    pub panes: Vec<PaneCurves>,
    pub pulse: f64,
    pub bpm: f64,
    pub centroid: f64,
    /// Onsets since the start, so a renderer that missed a hop still sees one happened.
    pub onsets: u64,
    pub loudness: LoudnessReadings,
    /// The newest audio, oldest first, and the frame index of its last sample.
    pub scope_left: Vec<f32>,
    pub scope_right: Vec<f32>,
    pub rows: u64,
    /// Audio frames lost because the ring was full, and rows lost because the renderer
    /// fell behind.
    pub dropped_frames: u64,
    pub dropped_rows: u64,
}

/// Commands for the analysis thread.
#[derive(Debug)]
pub enum Command {
    Config(AnalysisConfig),
    /// The stream's rate changed: analysis starts again at the new rate.
    SampleRate(f64),
    /// A new track: the programme measures start again.
    ResetTrack,
    Stop,
}

/// The render side of the analysis.
#[derive(Debug)]
pub struct Analysis {
    rows: rtrb::Consumer<RowMsg>,
    latest: triple_buffer::Output<Snapshot>,
    commands: mpsc::Sender<Command>,
    thread: Option<JoinHandle<()>>,
}

impl Analysis {
    /// Starts the analysis thread. Returns the render side and the capture side.
    pub fn spawn(
        sample_rate: f64,
        hop_rate: f64,
        config: AnalysisConfig,
    ) -> (Analysis, AudioInput) {
        let (audio_tx, audio_rx) = rtrb::RingBuffer::new(AUDIO_RING_FRAMES * 2);
        let (rows_tx, rows_rx) = rtrb::RingBuffer::new(ROW_RING);
        let (latest_tx, latest_rx) = triple_buffer::triple_buffer(&Snapshot::default());
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let dropped = Arc::new(AtomicU64::new(0));
        let input = AudioInput {
            ring: audio_tx,
            dropped: dropped.clone(),
        };

        let thread = thread::Builder::new()
            .name("sonorant-analysis".into())
            .spawn(move || {
                let engine = Engine::new(sample_rate, hop_rate, config);
                let sink = Publisher {
                    rows: rows_tx,
                    latest: latest_tx,
                    row: Box::new(RowMsg::empty()),
                    onsets: 0,
                    rows_sent: 0,
                    dropped_rows: 0,
                    dropped_frames: dropped,
                    sample_rate,
                };
                run(engine, sink, audio_rx, cmd_rx, hop_rate);
            })
            .expect("the analysis thread starts");

        (
            Analysis {
                rows: rows_rx,
                latest: latest_rx,
                commands: cmd_tx,
                thread: Some(thread),
            },
            input,
        )
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    /// The newest state. Never waits.
    pub fn latest(&mut self) -> &Snapshot {
        self.latest.update();
        self.latest.output_buffer()
    }

    /// Rows cut since the last call, oldest first.
    pub fn take_rows(&mut self, mut each: impl FnMut(&RowMsg)) {
        while let Ok(row) = self.rows.pop() {
            each(&row);
        }
    }
}

impl Drop for Analysis {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(
    mut engine: Engine,
    mut sink: Publisher,
    mut audio: rtrb::Consumer<f32>,
    commands: mpsc::Receiver<Command>,
    hop_rate: f64,
) {
    loop {
        while let Ok(cmd) = commands.try_recv() {
            match cmd {
                Command::Config(c) => engine.set_config(c),
                Command::SampleRate(rate) => {
                    if rate != engine.sample_rate() {
                        engine = Engine::new(rate, hop_rate, engine.config().clone());
                        sink.sample_rate = rate;
                    }
                }
                Command::ResetTrack => engine.reset_track(),
                Command::Stop => return,
            }
        }
        let available = audio.slots() & !1;
        if available == 0 {
            thread::sleep(IDLE_SLEEP);
            continue;
        }
        let Ok(chunk) = audio.read_chunk(available) else {
            continue;
        };
        let (a, b) = chunk.as_slices();
        engine.push_interleaved(a, &mut sink);
        engine.push_interleaved(b, &mut sink);
        chunk.commit_all();
    }
}

struct Publisher {
    rows: rtrb::Producer<RowMsg>,
    latest: triple_buffer::Input<Snapshot>,
    row: Box<RowMsg>,
    onsets: u64,
    rows_sent: u64,
    dropped_rows: u64,
    dropped_frames: Arc<AtomicU64>,
    sample_rate: f64,
}

fn copy_f32(dst: &mut Vec<f32>, src: &[f64]) {
    dst.clear();
    dst.extend(src.iter().map(|&v| v as f32));
}

impl Sink for Publisher {
    fn hop(&mut self, h: &Hop<'_>) {
        if h.features.onset() {
            self.onsets += 1;
        }
        let s = self.latest.input_buffer_mut();
        s.hop = h.index;
        s.frames = h.frames;
        s.sample_rate = self.sample_rate;
        s.analysed = h.analysed;
        // Smoothed, or the status line's figure is unreadable.
        s.analysis_seconds += (h.seconds - s.analysis_seconds) * 0.05;
        s.floor_db = h.floor_db;
        s.ceiling_db = h.ceiling_db;
        s.panes.resize_with(h.panes.len(), PaneCurves::default);
        for (out, p) in s.panes.iter_mut().zip(h.panes) {
            let e = p.extremes();
            copy_f32(&mut out.raw, p.raw());
            copy_f32(&mut out.display, p.display());
            copy_f32(&mut out.max, e.max());
            copy_f32(&mut out.min, e.min());
            copy_f32(&mut out.average, e.average());
        }
        s.pulse = h.features.pulse();
        s.bpm = h.features.bpm();
        s.centroid = h.features.centroid();
        s.onsets = self.onsets;
        s.loudness = h.loudness;
        let n = h.recent_left.len().min(SCOPE_FRAMES);
        s.scope_left.clear();
        s.scope_left
            .extend_from_slice(&h.recent_left[h.recent_left.len() - n..]);
        s.scope_right.clear();
        s.scope_right
            .extend_from_slice(&h.recent_right[h.recent_right.len() - n..]);
        s.rows = self.rows_sent;
        s.dropped_rows = self.dropped_rows;
        s.dropped_frames = self.dropped_frames.load(Ordering::Relaxed);
        self.latest.publish();
    }

    fn row(&mut self, r: &Row<'_>) {
        let m = &mut *self.row;
        m.index = r.index;
        m.frames = r.frames;
        m.floor_db = r.floor_db as f32;
        m.ceiling_db = r.ceiling_db as f32;
        m.wave = r.wave;
        for (d, &v) in m.a.iter_mut().zip(r.a) {
            *d = f16::from_f64(v);
        }
        for (d, &v) in m.b.iter_mut().zip(r.b) {
            *d = f16::from_f64(v);
        }
        if self.rows.is_full() {
            self.dropped_rows += 1;
        } else {
            let _ = self.rows.push((*self.row).clone());
            self.rows_sent += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use std::f64::consts::PI;
    use std::time::Instant;

    #[test]
    fn audio_in_rows_and_state_out() {
        let config = AnalysisConfig::from_settings(&Settings::default(), 256);
        let (mut analysis, mut input) = Analysis::spawn(48000.0, 120.0, config);
        // One second of a tone, in capture-sized pieces.
        let frames: Vec<f32> = (0..48000)
            .flat_map(|i| {
                let v = (0.25 * (2.0 * PI * 1000.0 * i as f64 / 48000.0).sin()) as f32;
                [v, v]
            })
            .collect();
        for chunk in frames.chunks(960) {
            input.push_interleaved(chunk);
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut rows = Vec::new();
        loop {
            analysis.take_rows(|r| rows.push((r.index, r.frames)));
            let s = analysis.latest();
            if s.frames == 48000 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "analysis stalled at {} frames",
                s.frames
            );
            thread::sleep(Duration::from_millis(5));
        }
        analysis.take_rows(|r| rows.push((r.index, r.frames)));
        let s = analysis.latest().clone();
        assert!(s.analysed);
        assert_eq!(s.panes.len(), 2);
        assert_eq!(s.panes[0].display.len(), 256);
        assert_eq!(s.scope_left.len(), SCOPE_FRAMES);
        assert_eq!(s.dropped_frames, 0);
        // Rows are consecutive and timed in audio frames.
        assert!(!rows.is_empty());
        assert!(
            rows.windows(2)
                .all(|w| w[1].0 == w[0].0 + 1 && w[1].1 > w[0].1)
        );
        assert!(s.loudness.momentary > -20.0 && s.loudness.momentary < -10.0);
    }
}
