//! Audio sources: what a capture backend provides, and a WAV file source.
//!
//! A source pushes interleaved stereo `f32` frames into an [`AudioInput`] from a thread
//! of its own, and reports its format and state on a channel. The analysis never waits
//! on it, and it never waits on the analysis.

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::runtime::AudioInput;

/// The shape of the audio a source delivers after it has made it stereo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamFormat {
    pub sample_rate: u32,
    /// Channels in the source itself; what reaches the ring is always stereo.
    pub source_channels: u16,
}

/// What a source is doing, for the status line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceStatus {
    Starting,
    /// Capturing; the text says what, such as "Whole system, Speakers, 48 kHz".
    Running(String),
    /// No audio server or API to capture from.
    NoAudioServer,
    /// Nothing to capture: no output device, or the chosen app has gone.
    NoDevice,
    /// The system suspended the stream, as for a sleep.
    Suspended,
    /// Another app holds the device in exclusive mode.
    ExclusiveMode,
    Failed(String),
    Stopped,
}

impl fmt::Display for SourceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceStatus::Starting => f.write_str("starting capture"),
            SourceStatus::Running(what) => f.write_str(what),
            SourceStatus::NoAudioServer => f.write_str("no audio server"),
            SourceStatus::NoDevice => f.write_str("no output device"),
            SourceStatus::Suspended => f.write_str("capture suspended"),
            SourceStatus::ExclusiveMode => f.write_str("device in exclusive use"),
            SourceStatus::Failed(why) => write!(f, "capture failed: {why}"),
            SourceStatus::Stopped => f.write_str("capture stopped"),
        }
    }
}

/// News from a source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceEvent {
    Status(SourceStatus),
    /// The stream's format, sent before the first frames and again whenever it changes.
    Format(StreamFormat),
}

/// A capture backend.
pub trait AudioSource: Send + fmt::Debug {
    /// Starts delivering audio into `input` on a thread of the source's own.
    fn start(&mut self, input: AudioInput, events: Sender<SourceEvent>);
    /// Stops and joins the source's thread.
    fn stop(&mut self);
}

/// Turns interleaved frames of any channel count into stereo, as Nostalgia+'s ring did:
/// mono is doubled, and past two channels only the front pair is kept.
pub fn to_stereo(frames: &[f32], channels: usize, out: &mut Vec<f32>) {
    out.clear();
    match channels {
        0 => {}
        1 => out.extend(frames.iter().flat_map(|&v| [v, v])),
        2 => out.extend_from_slice(frames),
        n => out.extend(frames.chunks_exact(n).flat_map(|f| [f[0], f[1]])),
    }
}

// ---------------------------------------------------------------- WAV

/// A decoded WAV file: stereo `f32` frames.
#[derive(Clone, Debug, PartialEq)]
pub struct Wav {
    pub sample_rate: u32,
    pub source_channels: u16,
    /// Interleaved stereo.
    pub frames: Vec<f32>,
}

impl Wav {
    pub fn frame_count(&self) -> usize {
        self.frames.len() / 2
    }

    pub fn open(path: &Path) -> io::Result<Wav> {
        Wav::parse(&fs::read(path)?)
    }

    /// Reads 8, 16, 24 and 32-bit integer PCM and 32 and 64-bit float, including the
    /// extensible header.
    pub fn parse(bytes: &[u8]) -> io::Result<Wav> {
        let bad = |why: &str| io::Error::new(io::ErrorKind::InvalidData, why.to_owned());
        if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            return Err(bad("not a RIFF WAVE file"));
        }
        let u16_at = |b: &[u8], i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        let u32_at = |b: &[u8], i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);

        let mut fmt: Option<(u16, u16, u32, u16)> = None;
        let mut data: Option<&[u8]> = None;
        let mut pos = 12;
        while pos + 8 <= bytes.len() {
            let id = &bytes[pos..pos + 4];
            let len = u32_at(bytes, pos + 4) as usize;
            let body = &bytes[pos + 8..(pos + 8 + len).min(bytes.len())];
            match id {
                b"fmt " if body.len() >= 16 => {
                    let mut tag = u16_at(body, 0);
                    let channels = u16_at(body, 2);
                    let rate = u32_at(body, 4);
                    let bits = u16_at(body, 14);
                    // WAVE_FORMAT_EXTENSIBLE: the real tag is the subformat GUID's first two bytes.
                    if tag == 0xFFFE && body.len() >= 26 {
                        tag = u16_at(body, 24);
                    }
                    fmt = Some((tag, channels, rate, bits));
                }
                b"data" => data = Some(body),
                _ => {}
            }
            pos += 8 + len + (len & 1);
        }
        let (tag, channels, rate, bits) = fmt.ok_or_else(|| bad("no fmt chunk"))?;
        let data = data.ok_or_else(|| bad("no data chunk"))?;
        if channels == 0 || rate == 0 {
            return Err(bad("no channels or no sample rate"));
        }
        let samples: Vec<f32> = match (tag, bits) {
            (1, 8) => data.iter().map(|&b| (b as f32 - 128.0) / 128.0).collect(),
            (1, 16) => data
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
                .collect(),
            (1, 24) => data
                .chunks_exact(3)
                .map(|c| (i32::from_le_bytes([0, c[0], c[1], c[2]]) >> 8) as f32 / 8_388_608.0)
                .collect(),
            (1, 32) => data
                .chunks_exact(4)
                .map(|c| {
                    (i32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f64 / 2_147_483_648.0) as f32
                })
                .collect(),
            (3, 32) => data
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect(),
            (3, 64) => data
                .chunks_exact(8)
                .map(|c| f64::from_le_bytes(c.try_into().expect("8 bytes")) as f32)
                .collect(),
            _ => {
                return Err(bad(&format!(
                    "unsupported WAV encoding: tag {tag}, {bits} bits"
                )));
            }
        };
        let mut frames = Vec::new();
        to_stereo(
            &samples[..samples.len() - samples.len() % channels as usize],
            channels as usize,
            &mut frames,
        );
        Ok(Wav {
            sample_rate: rate,
            source_channels: channels,
            frames,
        })
    }
}

/// Plays a WAV file into the analysis as if it were being captured: in real time, or as
/// fast as the ring takes it. Deterministic, so tests, golden renders and demos use it.
#[derive(Debug)]
pub struct WavSource {
    wav: Arc<Wav>,
    realtime: bool,
    looping: bool,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl WavSource {
    pub fn new(wav: Wav, realtime: bool, looping: bool) -> WavSource {
        WavSource {
            wav: Arc::new(wav),
            realtime,
            looping,
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }
}

impl AudioSource for WavSource {
    fn start(&mut self, mut input: AudioInput, events: Sender<SourceEvent>) {
        self.stop();
        self.stop.store(false, Ordering::Relaxed);
        let (wav, realtime, looping, stop) = (
            self.wav.clone(),
            self.realtime,
            self.looping,
            self.stop.clone(),
        );
        self.thread = Some(
            thread::Builder::new()
                .name("sonorant-wav".into())
                .spawn(move || {
                    let rate = wav.sample_rate;
                    let _ = events.send(SourceEvent::Format(StreamFormat {
                        sample_rate: rate,
                        source_channels: wav.source_channels,
                    }));
                    let _ = events.send(SourceEvent::Status(SourceStatus::Running(format!(
                        "WAV file, {} kHz",
                        rate as f64 / 1000.0
                    ))));
                    // 10 ms pieces, like a capture callback's.
                    let chunk = (rate as usize / 100).max(1) * 2;
                    let started = Instant::now();
                    let mut sent = 0usize;
                    'outer: loop {
                        for piece in wav.frames.chunks(chunk) {
                            if stop.load(Ordering::Relaxed) {
                                break 'outer;
                            }
                            if realtime {
                                let due = Duration::from_secs_f64(sent as f64 / rate as f64);
                                if let Some(wait) = due.checked_sub(started.elapsed()) {
                                    thread::sleep(wait);
                                }
                                input.push_interleaved(piece);
                            } else {
                                let mut rest = piece;
                                while !rest.is_empty() && !stop.load(Ordering::Relaxed) {
                                    let n = input.push_some(rest);
                                    rest = &rest[n..];
                                    if n == 0 {
                                        thread::sleep(Duration::from_millis(1));
                                    }
                                }
                            }
                            sent += piece.len() / 2;
                        }
                        if !looping {
                            break;
                        }
                    }
                    let _ = events.send(SourceEvent::Status(SourceStatus::Stopped));
                })
                .expect("the WAV thread starts"),
        );
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for WavSource {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav_bytes(tag: u16, channels: u16, rate: u32, bits: u16, data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&tag.to_le_bytes());
        v.extend_from_slice(&channels.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        let align = channels * bits / 8;
        v.extend_from_slice(&(rate * align as u32).to_le_bytes());
        v.extend_from_slice(&align.to_le_bytes());
        v.extend_from_slice(&bits.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(data);
        v
    }

    #[test]
    fn reads_the_common_encodings() {
        let pcm16: Vec<u8> = [0i16, 16384, -32768, 32767]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let w = Wav::parse(&wav_bytes(1, 2, 44100, 16, &pcm16)).unwrap();
        assert_eq!(w.sample_rate, 44100);
        assert_eq!(w.frames, [0.0, 0.5, -1.0, 32767.0 / 32768.0]);

        let mono24: Vec<u8> = vec![0x00, 0x00, 0x40, 0x00, 0x00, 0xC0];
        let w = Wav::parse(&wav_bytes(1, 1, 48000, 24, &mono24)).unwrap();
        assert_eq!(w.frames, [0.5, 0.5, -0.5, -0.5]);

        let f32s: Vec<u8> = [0.25f32, -0.75, 0.1, 0.2, 0.3, 0.4]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let w = Wav::parse(&wav_bytes(3, 3, 96000, 32, &f32s)).unwrap();
        assert_eq!(w.source_channels, 3);
        assert_eq!(w.frames, [0.25, -0.75, 0.2, 0.3]);

        assert!(Wav::parse(b"RIFF....WAVEjunk").is_err());
        assert!(Wav::parse(&wav_bytes(2, 2, 44100, 4, &[0; 8])).is_err());
    }

    #[test]
    fn extra_channels_keep_the_front_pair() {
        let mut out = Vec::new();
        to_stereo(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 3, &mut out);
        assert_eq!(out, [1.0, 2.0, 4.0, 5.0]);
        to_stereo(&[1.0, 2.0], 1, &mut out);
        assert_eq!(out, [1.0, 1.0, 2.0, 2.0]);
    }
}
