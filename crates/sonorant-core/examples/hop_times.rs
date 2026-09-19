//! How long each engine hop takes, as a distribution rather than criterion's mean: the
//! long transforms and the history rows land on alternate hops at 120 a second, and a
//! busy machine adds preemption to the tail. Twenty seconds of two tones per profile.
//!
//! `cargo run --release -p sonorant-core --example hop_times`

#![allow(clippy::print_stdout)] // a command-line report

use std::f64::consts::PI;
use std::time::Instant;

use sonorant_core::dsp::AnalysisQuality;
use sonorant_core::engine::{AnalysisConfig, DEFAULT_HOP_RATE, Engine, NullSink};
use sonorant_core::settings::Settings;

fn main() {
    let rate = 48000.0;
    let n = 48000 * 20;
    let tone = |f: f64, a: f64, i: usize| a * (2.0 * PI * f * i as f64 / rate).sin();
    let l: Vec<f32> = (0..n)
        .map(|i| (tone(220.0, 0.3, i) + tone(3520.0, 0.1, i)) as f32)
        .collect();
    let r: Vec<f32> = (0..n).map(|i| tone(330.0, 0.25, i) as f32).collect();
    let hop = (rate / DEFAULT_HOP_RATE) as usize;
    for quality in AnalysisQuality::ALL {
        let settings = Settings {
            quality,
            ..Settings::default()
        };
        let config = AnalysisConfig::from_settings(&settings, 1080);
        let mut engine = Engine::new(rate, DEFAULT_HOP_RATE, config);
        let mut times = Vec::new();
        for at in (0..n - hop).step_by(hop) {
            let start = Instant::now();
            engine.push(&l[at..at + hop], &r[at..at + hop], &mut NullSink);
            // From the second second on, once every transform has a full window.
            if at >= rate as usize {
                times.push(start.elapsed().as_secs_f64() * 1e6);
            }
        }
        let mean = times.iter().sum::<f64>() / times.len() as f64;
        times.sort_by(f64::total_cmp);
        let at = |f: f64| times[((times.len() - 1) as f64 * f) as usize];
        println!(
            "{:<10} mean {mean:6.1}  min {:6.1}  median {:6.1}  p90 {:6.1}  p99 {:6.1} µs",
            quality.name(),
            at(0.0),
            at(0.5),
            at(0.9),
            at(0.99)
        );
    }
}
