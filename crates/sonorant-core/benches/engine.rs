//! What a hop costs in the engine itself, at the app's 120 hops a second: the transforms
//! (the long ones every other hop), the display curves, a history row every other hop,
//! features, range and loudness. The target is under 0.5 ms for Balanced stereo at
//! 48 kHz (docs/plan.md, "Targets").

use std::f64::consts::PI;
use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use sonorant_core::dsp::AnalysisQuality;
use sonorant_core::engine::{AnalysisConfig, DEFAULT_HOP_RATE, Engine, NullSink};
use sonorant_core::settings::Settings;

const RATE: f64 = 48000.0;

/// Ten seconds of two tones and a little noise per channel, looped by the benchmark.
fn signal() -> (Vec<f32>, Vec<f32>) {
    let n = RATE as usize * 10;
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
    };
    let mut l = Vec::with_capacity(n);
    let mut r = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f64 / RATE;
        l.push(
            (0.3 * (2.0 * PI * 220.0 * t).sin()
                + 0.1 * (2.0 * PI * 3520.0 * t).sin()
                + 0.02 * noise()) as f32,
        );
        r.push((0.25 * (2.0 * PI * 330.0 * t).sin() + 0.02 * noise()) as f32);
    }
    (l, r)
}

fn engine(c: &mut Criterion) {
    let (l, r) = signal();
    let hop = (RATE / DEFAULT_HOP_RATE) as usize;
    for quality in AnalysisQuality::ALL {
        let settings = Settings {
            quality,
            ..Settings::default()
        };
        let mut engine = Engine::new(
            RATE,
            DEFAULT_HOP_RATE,
            AnalysisConfig::from_settings(&settings, 1080),
        );
        // Past the first second, so every transform has a full window.
        let mut at = 0;
        while at < RATE as usize {
            engine.push(&l[at..at + hop], &r[at..at + hop], &mut NullSink);
            at += hop;
        }
        c.bench_function(
            &format!("engine hop, {} profile, 1080 columns", quality.name()),
            |b| {
                b.iter(|| {
                    if at + hop > l.len() {
                        at = 0;
                    }
                    engine.push(black_box(&l[at..at + hop]), &r[at..at + hop], &mut NullSink);
                    at += hop;
                })
            },
        );
    }
}

criterion_group!(benches, engine);
criterion_main!(benches);
