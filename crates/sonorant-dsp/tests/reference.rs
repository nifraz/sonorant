//! Sonorant's DSP against the numbers Nostalgia+ produced for the same inputs.
//!
//! Tolerances follow the plan: spectra within 0.01 dB wherever the reference is above
//! -120 dBFS, loudness within 0.01 LU, and overs, notes and tempo exactly. Everything
//! that is plain arithmetic on the same inputs is held much tighter.

use std::collections::HashMap;
use std::f64::consts::PI;

use sonorant_dsp::fft::{Complex, Fft};
use sonorant_dsp::loudness::{self, LoudnessMeter, LoudnessReadings};
use sonorant_dsp::*;
use sonorant_testdata::{Mismatches, Signal, Value, ValueExt, floats, json, signal};

fn window_type(v: &str) -> WindowType {
    WindowType::from_name(v).unwrap_or_else(|| panic!("unknown window {v}"))
}

fn map_from(v: &Value) -> FrequencyMap {
    FrequencyMap::new(
        FreqScale::from_name(v.s("scale")).unwrap(),
        v.u("width"),
        v.f("fmin"),
        v.f("fmax"),
    )
}

#[test]
fn windows_match() {
    let doc = json("fft.json");
    let mut m = Mismatches::new();
    for w in doc.arr("windows") {
        let (coeffs, gain) = window::build(window_type(w.s("window")), w.u("n"));
        let name = format!("{} {}", w.s("window"), w.u("n"));
        m.close(|| format!("{name} gain"), gain, w.f("coherent_gain"), 1e-14);
        for (i, (&got, want)) in coeffs.iter().zip(w.floats("coefficients")).enumerate() {
            m.close(|| format!("{name} [{i}]"), got, want, 1e-14);
        }
    }
    m.assert_none("windows");
}

#[test]
fn transforms_match() {
    let doc = json("fft.json");
    let mut m = Mismatches::new();
    for t in doc.arr("forward") {
        let n = t.u("n");
        let (re, im) = (t.floats("in_re"), t.floats("in_im"));
        let mut data: Vec<Complex<f64>> = re
            .iter()
            .zip(&im)
            .map(|(&r, &i)| Complex::new(r, i))
            .collect();
        Fft::new(n).forward(&mut data);
        // Different factorisations round differently; hold them to 1e-12 of the scale.
        let scale = n as f64;
        for (i, ((c, wr), wi)) in data
            .iter()
            .zip(t.floats("out_re"))
            .zip(t.floats("out_im"))
            .enumerate()
        {
            m.close(|| format!("n={n} re[{i}]"), c.re / scale, wr / scale, 1e-13);
            m.close(|| format!("n={n} im[{i}]"), c.im / scale, wi / scale, 1e-13);
        }
    }

    let mr = &doc["magnitude_real"];
    let n = mr.u("n");
    let (w, g) = window::build(window_type(mr.s("window")), n);
    let mut out = vec![0.0; n / 2 + 1];
    Fft::new(n).magnitude_real(&mr.floats("input"), &w, g, &mut out);
    for (i, (&got, want)) in out.iter().zip(mr.floats("magnitude")).enumerate() {
        m.close(|| format!("magnitude_real[{i}]"), got, want, 1e-12);
    }

    let mp = &doc["magnitude_real_pair"];
    let n = mp.u("n");
    let (w, g) = window::build(window_type(mp.s("window")), n);
    let (mut ol, mut or) = (vec![0.0; n / 2 + 1], vec![0.0; n / 2 + 1]);
    Fft::new(n).magnitude_real_pair(
        &mp.floats("left"),
        &mp.floats("right"),
        &w,
        g,
        &mut ol,
        &mut or,
    );
    for (i, (&got, want)) in ol.iter().zip(mp.floats("magnitude_left")).enumerate() {
        m.close(|| format!("pair left[{i}]"), got, want, 1e-12);
    }
    for (i, (&got, want)) in or.iter().zip(mp.floats("magnitude_right")).enumerate() {
        m.close(|| format!("pair right[{i}]"), got, want, 1e-12);
    }
    m.assert_none("fft");
}

#[test]
fn frequency_maps_match() {
    let doc = json("frequency_map.json");
    let mut m = Mismatches::new();
    for spec in doc.arr("maps") {
        let map = map_from(spec);
        let name = format!(
            "{} {} {}..{}",
            spec.s("scale"),
            spec.u("width"),
            spec.f("fmin"),
            spec.f("fmax")
        );
        let rel = |x: f64| 1e-12 * x.abs().max(1.0);
        for (i, want) in spec.floats("edges").into_iter().enumerate() {
            m.close(|| format!("{name} edge {i}"), map.edges[i], want, rel(want));
        }
        for (i, want) in spec.floats("centres").into_iter().enumerate() {
            m.close(
                || format!("{name} centre {i}"),
                map.centres[i],
                want,
                rel(want),
            );
        }
        for (f, want) in spec
            .floats("probe_freqs")
            .into_iter()
            .zip(spec.floats("probe_positions"))
        {
            m.close(
                || format!("{name} position of {f}"),
                map.freq_to_position(f),
                want,
                1e-12,
            );
        }
        for (x, want) in spec
            .floats("probe_x")
            .into_iter()
            .zip(spec.floats("x_to_freq"))
        {
            m.close(
                || format!("{name} freq at x={x}"),
                map.x_to_freq(x),
                want,
                rel(want),
            );
        }
    }
    m.assert_none("frequency maps");
}

#[test]
fn notes_match_exactly() {
    let doc = json("notes.json");
    let mut m = Mismatches::new();
    let names = doc.arr("names");
    for ((f, name), cents) in doc
        .floats("freqs")
        .into_iter()
        .zip(names)
        .zip(doc.floats("cents"))
    {
        let want = name.as_str().unwrap();
        match describe_note(f) {
            Some((note, c)) => {
                m.equal(
                    || format!("name at {f} Hz"),
                    note.to_string().as_str(),
                    want,
                );
                m.close(|| format!("cents at {f} Hz"), c, cents, 1e-9);
            }
            None => m.equal(|| format!("name at {f} Hz"), "-", want),
        }
    }
    for (midi, want) in doc.floats("midi_to_freq").into_iter().enumerate() {
        m.close(
            || format!("midi {midi}"),
            midi_to_freq(midi as f64),
            want,
            want * 1e-14,
        );
    }
    m.assert_none("notes");
}

fn load_signals(doc: &Value) -> HashMap<String, Signal> {
    doc.arr("signals")
        .iter()
        .map(|s| {
            (
                s.s("name").to_owned(),
                signal(s.s("name"), s.f("sample_rate")),
            )
        })
        .collect()
}

/// A reference level and ours agree to 0.01 dB where the reference is above -120 dBFS.
/// Below that both only have to be down there too: the last digits of -130 dB noise
/// depend on the FFT's rounding and mean nothing on screen.
fn compare_db(m: &mut Mismatches, label: &str, got: &[f64], want: &[f64]) {
    assert_eq!(got.len(), want.len(), "{label}: length");
    for (i, (&g, &w)) in got.iter().zip(want).enumerate() {
        if w > -120.0 {
            m.close(|| format!("{label} [{i}]"), g, w, 0.01);
        } else {
            m.close(
                || format!("{label} [{i}] (quiet)"),
                g.max(-120.0),
                -120.0,
                0.02,
            );
        }
    }
}

#[test]
fn spectra_match() {
    let doc = json("spectrum.json");
    let signals = load_signals(&doc);
    let mut m = Mismatches::new();
    let mut analyzers: HashMap<String, SpectrumAnalyzer> = HashMap::new();
    for case in doc.arr("cases") {
        let id = case.s("id");
        let sig = &signals[case.s("signal")];
        let quality = AnalysisQuality::from_name(case.s("quality")).unwrap();
        let window = window_type(case.s("window"));
        let an = analyzers.entry(sig.name.clone()).or_default();
        an.configure(case.f("sample_rate"), quality, window);
        assert_eq!(an.largest_fft(), case.u("largest_fft"), "{id}");
        assert_eq!(an.describe_resolution(), case.s("resolution"), "{id}");
        let map = map_from(&case["map"]);
        let aggregate = BandAggregate::from_name(case.s("aggregate")).unwrap();
        let tilt = case.f("tilt_db_per_octave");
        let mut a = vec![0.0; map.width];
        if case.s("kind") == "stereo" {
            let mode = ChannelPairMode::from_name(case.s("pair_mode")).unwrap();
            let mut b = vec![0.0; map.width];
            assert!(an.compute_stereo(
                &sig.left, &sig.right, &map, &mut a, &mut b, aggregate, tilt, mode
            ));
            compare_db(&mut m, &format!("{id} a"), &a, &case.floats("out_a"));
            compare_db(&mut m, &format!("{id} b"), &b, &case.floats("out_b"));
        } else {
            let mode = ChannelMode::from_name(case.s("channel_mode")).unwrap();
            assert!(an.compute(&sig.left, &sig.right, mode, &map, &mut a, aggregate, tilt));
            compare_db(&mut m, &format!("{id} a"), &a, &case.floats("out_a"));
        }
    }
    assert!(
        m.checked() > 100_000,
        "only {} values compared",
        m.checked()
    );
    m.assert_none("spectra");
}

#[test]
fn dynamic_range_matches() {
    let doc = json("dynamic_range.json");
    let mut m = Mismatches::new();
    for case in doc.arr("cases") {
        let name = case.s("name");
        let mut dr = DynamicRange::new();
        dr.low_percentile = case.f("low_percentile");
        let (floors, ceilings) = (case.floats("floor"), case.floats("ceiling"));
        let mut k = 0;
        for step in case.arr("steps") {
            let count = step.u("count");
            let (lo, span) = (step.f("lo"), step.f("span"));
            let db: Vec<f64> = (0..count)
                .map(|i| lo + (i as f64 / count as f64) * span)
                .collect();
            for _ in 0..step.u("frames") {
                dr.observe(&db, 0.94);
                dr.update(1.0 / 60.0);
                m.close(
                    || format!("{name} floor after update {k}"),
                    dr.floor(),
                    floors[k],
                    1e-9,
                );
                m.close(
                    || format!("{name} ceiling after update {k}"),
                    dr.ceiling(),
                    ceilings[k],
                    1e-9,
                );
                k += 1;
            }
        }
        assert_eq!(k, floors.len(), "{name}: update count");
    }
    m.assert_none("dynamic range");
}

#[test]
fn curve_shaping_matches() {
    let doc = json("curve_shaping.json");
    let input = doc.floats("input");
    let mut m = Mismatches::new();
    for s in doc.arr("smooth") {
        let interp = CurveInterpolation::from_name(s.s("interpolation")).unwrap();
        let filter = FilteringAmount::from_name(s.s("filtering")).unwrap();
        assert_eq!(filter.kernel_radius(), s.u("kernel_radius"));
        let mut out = vec![0.0; input.len()];
        smooth(&input, &mut out, interp, filter);
        for (i, (&got, want)) in out.iter().zip(s.floats("output")).enumerate() {
            m.close(|| format!("{interp:?}/{filter:?} [{i}]"), got, want, 1e-9);
        }
    }

    let ext_doc = &doc["extremum"];
    let bins = ext_doc.u("bins");
    let mut ext = ExtremumTracker::new();
    ext.resize(bins);
    let snaps = ext_doc.arr("snapshots");
    let mut next = 0;
    for f in 1..=240usize {
        let frame: Vec<f64> = (0..bins)
            .map(|i| -100.0 + ((i * 7 + f * 13) % 50) as f64)
            .collect();
        ext.update(&frame, 1.0 / 60.0);
        if next < snaps.len() && snaps[next].u("frame") == f {
            let s = &snaps[next];
            for (label, got, want) in [
                ("min", ext.min(), s.floats("min")),
                ("max", ext.max(), s.floats("max")),
                ("average", ext.average(), s.floats("average")),
            ] {
                for (i, (&g, w)) in got.iter().zip(want).enumerate() {
                    m.close(|| format!("frame {f} {label}[{i}]"), g, w, 1e-9);
                }
            }
            next += 1;
        }
    }
    assert_eq!(next, snaps.len());
    m.assert_none("curve shaping");
}

fn generate(spec: &Value, rate: f64, total: usize) -> Vec<f64> {
    let mut x = vec![0.0; total];
    match spec.s("kind") {
        "sine" => {
            let (f, a) = (spec.f("freq"), spec.f("amp"));
            for (i, v) in x.iter_mut().enumerate() {
                *v = a * (2.0 * PI * f * i as f64 / rate).sin();
            }
        }
        "segments" => {
            let f = spec.f("freq");
            let seg = (rate * spec.f("segment_seconds")) as usize;
            let (even, odd) = (spec.f("amp_even"), spec.f("amp_odd"));
            for (i, v) in x.iter_mut().enumerate() {
                let a = if (i / seg).is_multiple_of(2) {
                    even
                } else {
                    odd
                };
                *v = a * (2.0 * PI * f * i as f64 / rate).sin();
            }
        }
        "bursts" => {
            let (f, a) = (spec.f("freq"), spec.f("amp"));
            let burst = (rate * spec.f("burst_seconds")) as usize;
            for b in 0..spec.u("count") {
                let start = (rate * (0.5 + b as f64)) as usize;
                for i in 0..burst {
                    x[start + i] = a * (2.0 * PI * f * i as f64 / rate).sin();
                }
            }
        }
        other => panic!("unknown generator {other}"),
    }
    x
}

fn compare_readings(m: &mut Mismatches, label: &str, r: &LoudnessReadings, want: &Value) {
    m.close(
        || format!("{label} momentary"),
        r.momentary,
        want.f("m"),
        0.01,
    );
    m.close(
        || format!("{label} short-term"),
        r.short_term,
        want.f("s"),
        0.01,
    );
    m.close(
        || format!("{label} integrated"),
        r.integrated,
        want.f("i"),
        0.01,
    );
    m.close(|| format!("{label} range"), r.range, want.f("lra"), 0.01);
    m.close(
        || format!("{label} true peak"),
        r.true_peak_db,
        want.f("tp"),
        0.01,
    );
    m.close(
        || format!("{label} crest"),
        r.crest_db,
        want.f("crest"),
        0.01,
    );
    m.close(
        || format!("{label} correlation"),
        r.correlation,
        want.f("corr"),
        1e-6,
    );
    m.close(
        || format!("{label} balance"),
        r.balance,
        want.f("bal"),
        1e-6,
    );
    m.equal(
        || format!("{label} overs"),
        r.overs as u64,
        want.u("overs") as u64,
    );
    m.close(
        || format!("{label} last over"),
        r.last_over_seconds,
        want.f("last_over"),
        1e-9,
    );
}

#[test]
fn loudness_matches() {
    let doc = json("loudness.json");
    let signals = load_signals(&json("spectrum.json"));
    let mut m = Mismatches::new();

    for f in doc.arr("filters") {
        let rate = f.f("sample_rate");
        let (shelf, hp) = loudness::k_weighting(rate);
        for (got, want, which) in [
            (shelf, f.floats("shelf"), "shelf"),
            (hp, f.floats("high_pass"), "hp"),
        ] {
            for (i, (g, w)) in got.coefficients().into_iter().zip(want).enumerate() {
                m.close(|| format!("{rate} {which}[{i}]"), g, w, 1e-13);
            }
        }
    }
    let phases = loudness::true_peak_phases();
    for (p, row) in doc.arr("true_peak_phases").iter().enumerate() {
        for (k, w) in floats(row).into_iter().enumerate() {
            m.close(|| format!("phase {p} tap {k}"), phases[p][k], w, 1e-15);
        }
    }

    for case in doc.arr("cases") {
        let name = case.s("name");
        if name == "reset_after_sustained_16k" {
            let mut meter = LoudnessMeter::new();
            meter.configure(16000.0);
            let x: Vec<f64> = (0..16000 * 6)
                .map(|i| 0.999 * (2.0 * PI * 500.0 * i as f64 / 16000.0).sin())
                .collect();
            meter.process(&x, &x);
            meter.reset();
            compare_readings(&mut m, name, &meter.readings(), &case["after_reset"]);
            continue;
        }
        let rate = case.f("sample_rate");
        let (l, r) = if let Some(sig) = case["signal"].as_str() {
            let s = &signals[sig];
            (
                s.left.iter().map(|&v| v as f64).collect(),
                s.right.iter().map(|&v| v as f64).collect(),
            )
        } else {
            let x = generate(&case["generator"], rate, case.u("frames"));
            let sum: f64 = x.iter().sum();
            let sq: f64 = x.iter().map(|v| v * v).sum();
            let (want_sum, want_sq) = (case.f("input_sum"), case.f("input_sum_squares"));
            assert!(
                (sum - want_sum).abs() < 1e-6,
                "{name}: generator sum {sum} vs {want_sum}"
            );
            assert!(
                (sq - want_sq).abs() < 1e-6 * want_sq.max(1.0),
                "{name}: generator energy"
            );
            match case.s("channels") {
                "negated" => (x.clone(), x.iter().map(|v| -v).collect::<Vec<_>>()),
                "right_only" => (vec![0.0; x.len()], x),
                _ => (x.clone(), x),
            }
        };
        let mut meter = LoudnessMeter::new();
        meter.configure(rate);
        let chunk = case.u("chunk_frames");
        for (k, (cl, cr)) in l.chunks(chunk).zip(r.chunks(chunk)).enumerate() {
            meter.process(cl, cr);
            compare_readings(
                &mut m,
                &format!("{name} chunk {k}"),
                &meter.readings(),
                &case.arr("readings")[k],
            );
        }
    }
    m.assert_none("loudness");
}

#[test]
fn music_features_match() {
    let doc = json("music_features.json");
    let mut m = Mismatches::new();
    for case in doc.arr("cases") {
        let name = case.s("name");
        let bins = case.u("bins");
        let mut f = MusicFeatures::new(60.0);
        f.octave_check = false;
        assert_eq!(f.pulse_decay_seconds, doc.f("pulse_decay_seconds"));
        let frames = case.arr("frames");
        let mut t = 0.0;
        for (k, want) in frames.iter().enumerate() {
            let (db, dt): (Vec<f64>, f64) = match name {
                "click_120bpm" => {
                    let hit = if k < 900 { k % 30 == 0 } else { k == 900 };
                    (vec![if hit { -30.0 } else { -75.0 }; bins], case.f("dt"))
                }
                "steady" => (vec![-40.0; bins], case.f("dt")),
                "bass_only" => (
                    (0..bins)
                        .map(|i| if i < bins / 8 { -20.0 } else { FLOOR_DB })
                        .collect(),
                    case.f("dt"),
                ),
                "treble_only" => (
                    (0..bins)
                        .map(|i| if i > bins * 7 / 8 { -20.0 } else { FLOOR_DB })
                        .collect(),
                    case.f("dt"),
                ),
                "jittered_clock" => {
                    let dt = if k % 3 == 0 { 0.020 } else { 0.015 };
                    let hit = t % 0.5 < dt;
                    let db = (0..bins)
                        .map(|i| {
                            if hit {
                                -30.0 + (i % 5) as f64
                            } else {
                                -75.0 + (i % 3) as f64
                            }
                        })
                        .collect();
                    t += dt;
                    (db, dt)
                }
                other => panic!("unknown case {other}"),
            };
            f.update(&db, dt);
            let w = floats(want);
            m.close(|| format!("{name} frame {k} flux"), f.flux(), w[0], 1e-9);
            m.equal(|| format!("{name} frame {k} onset"), f.onset(), w[1] == 1.0);
            m.close(|| format!("{name} frame {k} pulse"), f.pulse(), w[2], 1e-9);
            m.equal(|| format!("{name} frame {k} bpm"), f.bpm(), w[3]);
            m.close(
                || format!("{name} frame {k} centroid"),
                f.centroid(),
                w[4],
                1e-9,
            );
        }
    }
    m.assert_none("music features");
}
