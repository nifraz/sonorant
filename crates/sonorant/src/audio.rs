//! The app's side of the audio: a source, the analysis thread, and the clock the
//! picture scrolls by.

use std::path::Path;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use sonorant_core::engine::{AnalysisConfig, DEFAULT_HOP_RATE};
use sonorant_core::runtime::{Analysis, Command, RowMsg, Snapshot};
use sonorant_core::source::{AudioSource, SourceEvent, SourceStatus, Wav, WavSource};

/// Where the audio comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    /// The whole system mix.
    System,
    /// One app, by name or process id.
    App(String),
    /// A WAV file, looped in real time.
    File(std::path::PathBuf),
}

#[derive(Debug)]
pub struct Audio {
    source: Box<dyn AudioSource>,
    analysis: Analysis,
    events: Receiver<SourceEvent>,
    /// What capture is pointed at, so it isn't reopened for the target it already has.
    input: Input,
    pub status: SourceStatus,
    pub sample_rate: f64,
    clock: AudioClock,
}

impl Audio {
    pub fn start(input: &Input, config: AnalysisConfig) -> Result<Audio, String> {
        let (source, rate) = open(input)?;
        let (analysis, audio_in) = Analysis::spawn(rate, DEFAULT_HOP_RATE, config);
        let (tx, rx) = channel();
        let mut source = source;
        source.start(audio_in, tx);
        Ok(Audio {
            source,
            analysis,
            events: rx,
            input: input.clone(),
            status: SourceStatus::Starting,
            sample_rate: rate,
            clock: AudioClock::default(),
        })
    }

    pub fn input(&self) -> &Input {
        &self.input
    }

    /// Points capture somewhere else without disturbing the analysis.
    ///
    /// The new source is opened before the old one is stopped, so a target that turns
    /// out to be unreachable leaves the picture running on what it had. The analysis
    /// thread is handed a new ring and stops reading the old one, so nothing either
    /// source pushes in between can be interleaved into the wrong stream.
    pub fn set_input(&mut self, input: &Input) -> Result<(), String> {
        if self.input == *input {
            return Ok(());
        }
        let (mut source, rate) = open(input)?;
        self.source.stop();
        let (tx, rx) = channel();
        source.start(self.analysis.take_input(), tx);
        self.source = source;
        self.events = rx;
        self.input = input.clone();
        self.status = SourceStatus::Starting;
        if rate != self.sample_rate {
            self.sample_rate = rate;
            self.analysis.send(Command::SampleRate(rate));
        }
        // Wall time and audio time have to find each other again from the new stream.
        self.clock = AudioClock::default();
        Ok(())
    }

    pub fn set_config(&self, config: AnalysisConfig) {
        self.analysis.send(Command::Config(config));
    }

    /// A new track started: the programme measures - integrated loudness, range, BPM
    /// and overs - start again rather than averaging the last track into this one.
    pub fn reset_track(&self) {
        self.analysis.send(Command::ResetTrack);
    }

    /// Handles the source's news. Call once a frame.
    pub fn poll(&mut self) {
        while let Ok(ev) = self.events.try_recv() {
            match ev {
                SourceEvent::Format(f) => {
                    if f.sample_rate as f64 != self.sample_rate {
                        self.sample_rate = f.sample_rate as f64;
                        self.analysis.send(Command::SampleRate(self.sample_rate));
                        self.clock = AudioClock::default();
                    }
                    log::info!(
                        "capture format: {} Hz, {} channel(s)",
                        f.sample_rate,
                        f.source_channels
                    );
                }
                SourceEvent::Status(s) => {
                    log::info!("capture: {s}");
                    self.status = s;
                }
            }
        }
    }

    /// The newest analysis, and notes when it arrived for the clock.
    pub fn latest(&mut self, now: Instant) -> &Snapshot {
        let s = self.analysis.latest();
        self.clock.observe(s.frames, s.sample_rate, now);
        s
    }

    pub fn take_rows(&mut self, each: impl FnMut(&RowMsg)) {
        self.analysis.take_rows(each);
    }

    /// Audio time now, in frames, running smoothly between hops.
    pub fn frames_now(&self, now: Instant) -> f64 {
        self.clock.estimate(now)
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.source.stop();
    }
}

fn open(input: &Input) -> Result<(Box<dyn AudioSource>, f64), String> {
    match input {
        Input::File(path) => {
            let wav = Wav::open(Path::new(path))
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            let rate = wav.sample_rate as f64;
            Ok((Box::new(WavSource::new(wav, true, true)), rate))
        }
        Input::System => Ok((system_source()?, 48000.0)),
        Input::App(name) => Ok((app_source(name)?, 48000.0)),
    }
}

#[cfg(windows)]
fn system_source() -> Result<Box<dyn AudioSource>, String> {
    use sonorant_platform::windows::{Target, WasapiSource};
    Ok(Box::new(WasapiSource::new(Target::WholeSystem)))
}

#[cfg(windows)]
fn app_source(want: &str) -> Result<Box<dyn AudioSource>, String> {
    use sonorant_platform::windows::{Target, WasapiSource, audio_apps};
    let apps = audio_apps();
    let a = apps
        .iter()
        .find(|a| {
            want.parse::<u32>().is_ok_and(|pid| pid == a.pid) || a.name.eq_ignore_ascii_case(want)
        })
        .ok_or_else(|| format!("no app called {want} is playing"))?;
    Ok(Box::new(WasapiSource::new(Target::Process {
        pid: a.pid,
        name: a.name.clone(),
    })))
}

#[cfg(target_os = "linux")]
fn system_source() -> Result<Box<dyn AudioSource>, String> {
    use sonorant_platform::linux::{PipeWireSource, Target};
    Ok(Box::new(PipeWireSource::new(Target::WholeSystem)))
}

#[cfg(target_os = "linux")]
fn app_source(want: &str) -> Result<Box<dyn AudioSource>, String> {
    use sonorant_platform::linux::{PipeWireSource, Target, audio_apps};
    let apps = audio_apps();
    let a = apps
        .iter()
        .find(|a| {
            want.parse::<u64>()
                .is_ok_and(|n| n == a.serial || Some(n as u32) == a.pid)
                || a.name.eq_ignore_ascii_case(want)
        })
        .ok_or_else(|| format!("no app called {want} is playing"))?;
    Ok(Box::new(PipeWireSource::new(Target::Node {
        serial: a.serial,
        name: a.name.clone(),
    })))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn system_source() -> Result<Box<dyn AudioSource>, String> {
    Err("capture isn't supported on this platform; try --wav".into())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn app_source(_want: &str) -> Result<Box<dyn AudioSource>, String> {
    system_source()
}

/// Maps the analysis's frame count to wall time, so the picture scrolls smoothly between
/// hops.
///
/// Hops arrive every few milliseconds, a little early or late. The clock keeps a
/// slowly-adjusted offset between wall time and audio time instead of jumping to each
/// hop, so the scroll runs at an even speed. It never runs more than a few hops past the
/// newest analysis, so if the audio stalls the picture stops rather than scrolling on
/// into rows that don't exist, and it never runs backwards.
#[derive(Debug)]
struct AudioClock {
    epoch: Instant,
    frames: u64,
    rate: f64,
    /// Wall seconds minus audio seconds, smoothed.
    offset: Option<f64>,
    last: std::cell::Cell<f64>,
}

impl Default for AudioClock {
    fn default() -> Self {
        AudioClock {
            epoch: Instant::now(),
            frames: 0,
            rate: 48000.0,
            offset: None,
            last: 0.0.into(),
        }
    }
}

/// How far past the newest analysis the clock may run.
const MAX_LEAD: Duration = Duration::from_millis(40);

impl AudioClock {
    fn observe(&mut self, frames: u64, rate: f64, now: Instant) {
        if rate <= 0.0 || (frames == self.frames && self.offset.is_some()) {
            return;
        }
        if frames < self.frames {
            // The analysis restarted.
            *self = AudioClock::default();
        }
        self.frames = frames;
        self.rate = rate;
        let seen = now.duration_since(self.epoch).as_secs_f64() - frames as f64 / rate;
        self.offset = Some(match self.offset {
            // A jump of more than 100 ms is a stall or a restart: follow it at once.
            Some(o) if (seen - o).abs() < 0.1 => o + (seen - o) * 0.05,
            _ => seen,
        });
    }

    fn estimate(&self, now: Instant) -> f64 {
        let Some(offset) = self.offset else {
            return self.frames as f64;
        };
        let audio = (now.duration_since(self.epoch).as_secs_f64() - offset) * self.rate;
        let limit = self.frames as f64 + MAX_LEAD.as_secs_f64() * self.rate;
        let out = audio.min(limit).max(self.last.get());
        self.last.set(out);
        out
    }
}
