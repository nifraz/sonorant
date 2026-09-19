//! Headless checks of capture and analysis, from the terminal.
//!
//! `sonorant capture` starts a source and the analysis thread without a window and
//! prints what arrives once a second: the source's status, frames per second, loudness,
//! tempo and anything dropped. `sonorant apps` lists the apps that can be captured on
//! their own.

use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use sonorant_core::engine::{AnalysisConfig, DEFAULT_HOP_RATE};
use sonorant_core::runtime::Analysis;
use sonorant_core::settings::Settings;
use sonorant_core::source::{AudioSource, SourceEvent, Wav, WavSource};

pub const USAGE: &str = "\
Usage: sonorant capture [--app <name or pid>] [--wav <file>] [--seconds <s>]
       sonorant apps

  capture   capture and analyse without a window, printing once a second
  apps      list the apps playing audio, for --app";

#[derive(Debug, Default)]
struct CaptureArgs {
    app: Option<String>,
    wav: Option<PathBuf>,
    seconds: Option<f64>,
}

/// Runs a probe command if `args` names one. Returns the exit code, or `None` when the
/// arguments are for the normal app.
pub fn run(args: &[String]) -> Option<i32> {
    match args.first().map(String::as_str) {
        Some("capture") => Some(match parse(&args[1..]) {
            Ok(a) => capture(a),
            Err(e) => {
                eprintln!("sonorant: {e}\n\n{USAGE}");
                2
            }
        }),
        Some("apps") => Some(apps()),
        _ => None,
    }
}

fn parse(args: &[String]) -> Result<CaptureArgs, String> {
    let mut a = CaptureArgs::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--app" => a.app = Some(value()?),
            "--wav" => a.wav = Some(PathBuf::from(value()?)),
            "--seconds" => {
                let v = value()?;
                a.seconds = Some(
                    v.parse()
                        .map_err(|_| format!("--seconds takes a number, not {v}"))?,
                );
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(a)
}

fn say(text: &str) {
    let _ = writeln!(std::io::stdout(), "{text}");
}

#[cfg(windows)]
fn apps() -> i32 {
    let apps = sonorant_platform::windows::audio_apps();
    if apps.is_empty() {
        say("No app is playing through the default output device.");
    }
    for a in apps {
        say(&format!("{:>7}  {}", a.pid, a.name));
    }
    0
}

#[cfg(not(windows))]
fn apps() -> i32 {
    say("Listing apps needs the PipeWire backend, which isn't built here yet.");
    1
}

#[cfg(windows)]
fn live_source(app: Option<&str>) -> Result<Box<dyn AudioSource>, String> {
    use sonorant_platform::windows::{Target, WasapiSource, audio_apps};
    let target = match app {
        None => Target::WholeSystem,
        Some(want) => {
            let apps = audio_apps();
            let found = apps.iter().find(|a| {
                want.parse::<u32>().is_ok_and(|pid| pid == a.pid)
                    || a.name.eq_ignore_ascii_case(want)
            });
            let a = found
                .ok_or_else(|| format!("no app called {want} is playing; try `sonorant apps`"))?;
            Target::Process {
                pid: a.pid,
                name: a.name.clone(),
            }
        }
    };
    Ok(Box::new(WasapiSource::new(target)))
}

#[cfg(target_os = "linux")]
fn live_source(app: Option<&str>) -> Result<Box<dyn AudioSource>, String> {
    use sonorant_platform::linux::{PipeWireSource, Target, audio_apps};
    let target = match app {
        None => Target::WholeSystem,
        Some(want) => {
            let apps = audio_apps();
            let found = apps.iter().find(|a| {
                want.parse::<u64>()
                    .is_ok_and(|n| n == a.serial || Some(n as u32) == a.pid)
                    || a.name.eq_ignore_ascii_case(want)
            });
            let a = found
                .ok_or_else(|| format!("no app called {want} is playing; try `sonorant apps`"))?;
            Target::Node {
                serial: a.serial,
                name: a.name.clone(),
            }
        }
    };
    Ok(Box::new(PipeWireSource::new(target)))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn live_source(_app: Option<&str>) -> Result<Box<dyn AudioSource>, String> {
    Err("capture isn't supported on this platform".into())
}

fn capture(a: CaptureArgs) -> i32 {
    let (mut source, rate): (Box<dyn AudioSource>, f64) = match &a.wav {
        Some(path) => match Wav::open(path) {
            Ok(w) => {
                let rate = w.sample_rate as f64;
                (Box::new(WavSource::new(w, true, false)), rate)
            }
            Err(e) => {
                eprintln!("sonorant: cannot read {}: {e}", path.display());
                return 1;
            }
        },
        None => match live_source(a.app.as_deref()) {
            Ok(s) => (s, 48000.0),
            Err(e) => {
                eprintln!("sonorant: {e}");
                return 1;
            }
        },
    };

    let config = AnalysisConfig::from_settings(&Settings::default(), 512);
    let (mut analysis, input) = Analysis::spawn(rate, DEFAULT_HOP_RATE, config);
    let (tx, rx) = mpsc::channel();
    source.start(input, tx);

    let started = Instant::now();
    let mut last_print = started;
    let mut last_frames = 0u64;
    let mut rows = 0u64;
    loop {
        while let Ok(ev) = rx.try_recv() {
            match ev {
                SourceEvent::Format(f) => {
                    say(&format!(
                        "format: {} Hz, {} channel(s)",
                        f.sample_rate, f.source_channels
                    ));
                    analysis.send(sonorant_core::runtime::Command::SampleRate(
                        f.sample_rate as f64,
                    ));
                }
                SourceEvent::Status(s) => say(&format!("status: {s}")),
            }
        }
        analysis.take_rows(|_| rows += 1);
        let now = Instant::now();
        if now.duration_since(last_print) >= Duration::from_secs(1) {
            let s = analysis.latest();
            let per_second =
                (s.frames - last_frames) as f64 / now.duration_since(last_print).as_secs_f64();
            let l = &s.loudness;
            say(&format!(
                "{:6.1}s  {:7.0} frames/s  M {:6.1}  S {:6.1}  I {:6.1} LUFS  TP {:6.1} dBTP  corr {:+.2}  BPM {:5.1}  rows {}  dropped {}/{}",
                now.duration_since(started).as_secs_f64(),
                per_second,
                l.momentary,
                l.short_term,
                l.integrated,
                l.true_peak_db,
                l.correlation,
                s.bpm,
                rows,
                s.dropped_frames,
                s.dropped_rows,
            ));
            last_frames = s.frames;
            last_print = now;
        }
        if a.seconds
            .is_some_and(|limit| now.duration_since(started).as_secs_f64() >= limit)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    source.stop();
    0
}
