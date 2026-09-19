//! What one analysis hop costs. The target is under 0.5 ms for a Balanced stereo hop at
//! 48 kHz (docs/plan.md, "Targets").

use std::f64::consts::PI;
use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use sonorant_dsp::*;

fn signal(n: usize) -> (Vec<f32>, Vec<f32>) {
    let l = (0..n)
        .map(|i| {
            let t = i as f64 / 48000.0;
            (0.3 * (2.0 * PI * 220.0 * t).sin() + 0.1 * (2.0 * PI * 3520.0 * t).sin()) as f32
        })
        .collect();
    let r = (0..n)
        .map(|i| (0.25 * (2.0 * PI * 330.0 * i as f64 / 48000.0).sin()) as f32)
        .collect();
    (l, r)
}

fn hop(c: &mut Criterion) {
    let (l, r) = signal(40000);
    let map = FrequencyMap::new(FreqScale::Note, 1080, 20.0, 20000.0);
    let grid = FrequencyMap::new(FreqScale::Log, 2048, 10.0, 24000.0);
    let (mut a, mut b) = (vec![0.0; map.width], vec![0.0; map.width]);
    let (mut ga, mut gb) = (vec![0.0; 2048], vec![0.0; 2048]);

    for q in AnalysisQuality::ALL {
        let mut an = SpectrumAnalyzer::new();
        an.configure(48000.0, q, WindowType::Hann);
        c.bench_function(
            &format!("stereo spectrum, {} profile, 1080 columns", q.name()),
            |bench| {
                bench.iter(|| {
                    an.compute_stereo(
                        black_box(&l),
                        black_box(&r),
                        &map,
                        &mut a,
                        &mut b,
                        BandAggregate::Peak,
                        3.0,
                        ChannelPairMode::LeftRight,
                    )
                })
            },
        );
    }

    // The whole hop: the Balanced transform, the display columns and the history grid,
    // ballistics on both panes, features, range and 400 samples of loudness metering.
    let mut an = SpectrumAnalyzer::new();
    an.configure(48000.0, AnalysisQuality::Balanced, WindowType::Hann);
    let mut panes = [ChannelCurves::new(map.width), ChannelCurves::new(map.width)];
    let mut features = MusicFeatures::new(120.0);
    let mut range = DynamicRange::new();
    let mut meter = LoudnessMeter::new();
    meter.configure(48000.0);
    let ml: Vec<f64> = l[..400].iter().map(|&v| v as f64).collect();
    let mr: Vec<f64> = r[..400].iter().map(|&v| v as f64).collect();
    c.bench_function("whole hop, Balanced, 120 hops per second", |bench| {
        bench.iter(|| {
            let [pa, pb] = &mut panes;
            an.compute_stereo(
                &l,
                &r,
                &map,
                pa.raw_mut(),
                pb.raw_mut(),
                BandAggregate::Peak,
                3.0,
                ChannelPairMode::LeftRight,
            );
            an.reproject(&grid, &mut ga, Some(&mut gb), BandAggregate::Peak, 3.0);
            for p in &mut panes {
                p.update(
                    1.0 / 120.0,
                    CurveInterpolation::LinearSmooth,
                    FilteringAmount::Light,
                    20.0,
                    320.0,
                    14.0,
                    1.2,
                );
            }
            features.update(panes[0].raw(), 1.0 / 120.0);
            range.observe(panes[0].raw(), 0.94);
            range.observe(panes[1].raw(), 1.0);
            range.update(1.0 / 120.0);
            meter.process(&ml, &mr);
            black_box(range.floor())
        })
    });
}

criterion_group!(benches, hop);
criterion_main!(benches);
