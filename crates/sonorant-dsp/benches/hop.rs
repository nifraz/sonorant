//! What one analysis hop costs. The target is under 0.5 ms for a Balanced stereo hop at
//! 48 kHz (docs/plan.md, "Targets").

use std::f64::consts::PI;
use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use sonorant_dsp::fft::Fft;
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

    // The parts of a Balanced hop on their own: the transforms, then each projection.
    let mut ffts: Vec<(Fft, Vec<f64>, f64)> = [16384, 4096, 1024]
        .into_iter()
        .map(|n| {
            let (w, g) = window::build(WindowType::Hann, n);
            (Fft::new(n), w, g)
        })
        .collect();
    let (fl, fr): (Vec<f64>, Vec<f64>) = (
        l[..16384].iter().map(|&v| v as f64).collect(),
        r[..16384].iter().map(|&v| v as f64).collect(),
    );
    let (mut ml, mut mr) = (vec![0.0; 8193], vec![0.0; 8193]);
    c.bench_function("Balanced transforms only", |bench| {
        bench.iter(|| {
            for (fft, w, g) in &mut ffts {
                let n = fft.size();
                fft.power_real_pair(&fl[..n], &fr[..n], w, *g, &mut ml, &mut mr);
            }
            black_box(ml[100])
        })
    });
    let mut an = SpectrumAnalyzer::new();
    an.configure(48000.0, AnalysisQuality::Balanced, WindowType::Hann);
    an.compute_stereo(
        &l,
        &r,
        &map,
        &mut a,
        &mut b,
        BandAggregate::Peak,
        3.0,
        ChannelPairMode::LeftRight,
    );
    c.bench_function("Balanced projection only, 1080 columns", |bench| {
        bench.iter(|| {
            an.reproject(&map, &mut a, Some(&mut b), BandAggregate::Peak, 3.0);
            black_box(a[100])
        })
    });
    c.bench_function("Balanced projection only, 2048-row history grid", |bench| {
        bench.iter(|| {
            an.reproject(&grid, &mut ga, Some(&mut gb), BandAggregate::Peak, 3.0);
            black_box(ga[100])
        })
    });

    // The worst hop: every Balanced transform, the display columns and a history row,
    // ballistics on both panes, features, range and 400 samples of loudness metering.
    // At 120 hops a second the 16K transform and the row come every other hop; the
    // engine benchmark in sonorant-core measures the average.
    let mut an = SpectrumAnalyzer::new();
    an.configure(48000.0, AnalysisQuality::Balanced, WindowType::Hann);
    let mut panes = [ChannelCurves::new(map.width), ChannelCurves::new(map.width)];
    let mut features = MusicFeatures::new(120.0);
    let mut range = DynamicRange::new();
    let mut meter = LoudnessMeter::new();
    meter.configure(48000.0);
    let ml: Vec<f64> = l[..400].iter().map(|&v| v as f64).collect();
    let mr: Vec<f64> = r[..400].iter().map(|&v| v as f64).collect();
    c.bench_function("worst hop, Balanced, 120 hops per second", |bench| {
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
