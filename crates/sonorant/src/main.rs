//! Sonorant: a real-time spectrogram, spectrum and loudness visualiser for whatever is
//! playing.
//!
//! Capture feeds the analysis thread; the window draws its history and meters with wgpu
//! and egui. `sonorant capture` runs the same pipeline without a window.

// A GUI app on Windows: no console window. Logs still reach a terminal it's started from.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod audio;
mod chrome;
mod gpu;
mod nowplaying;
mod options;
mod pacing;
mod present;
mod probe;
mod timeview;
mod ui;

use std::io::Write;
use std::process::ExitCode;

use winit::event_loop::EventLoop;

use crate::app::{App, UserEvent};
use crate::options::{Options, USAGE};

fn main() -> ExitCode {
    env_logger::Builder::from_env(
        env_logger::Env::default()
            .default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn,egui_wgpu=warn"),
    )
    .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = probe::run(&args) {
        return ExitCode::from(code.clamp(0, 255) as u8);
    }

    let options = match Options::parse(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("sonorant: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if options.help || options.version {
        let text = if options.help {
            USAGE.to_owned()
        } else {
            format!("sonorant {}", env!("CARGO_PKG_VERSION"))
        };
        let _ = writeln!(std::io::stdout(), "{text}");
        return ExitCode::SUCCESS;
    }

    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(l) => l,
        Err(e) => {
            log::error!("cannot start the event loop: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut app = App::new(options, event_loop.create_proxy());
    if let Err(e) = event_loop.run_app(&mut app) {
        log::error!("event loop failed: {e}");
        return ExitCode::FAILURE;
    }
    match app.error() {
        Some(_) => ExitCode::FAILURE,
        None => ExitCode::SUCCESS,
    }
}
