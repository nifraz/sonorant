//! The analysis engine: audio in, spectra, meters and history rows out.
//!
//! Audio arrives in whatever chunks the capture API delivers. The engine cuts it into
//! hops of a fixed length in audio time and analyses once per hop, so results never
//! depend on the chunking or the display. Every sample also passes through the loudness
//! meter exactly once, in order.
//!
//! Per hop it publishes the display curves, the dynamic range, the music features and
//! the loudness readings. Spectrogram rows are cut at their own rate in audio time,
//! also on hop boundaries, and hold levels on a fixed log-spaced grid so the renderer
//! can apply any axis, range or palette to the whole history.
//!
//! Nothing here allocates once configured, except when the configuration changes.

use crate::dsp::{
    AnalysisQuality, BandAggregate, ChannelCurves, ChannelPairMode, CurveInterpolation,
    DynamicRange, FilteringAmount, FreqScale, FrequencyMap, LoudnessMeter, LoudnessReadings,
    MusicFeatures, SpectrumAnalyzer, WindowType,
};
use crate::settings::Settings;

/// Levels per history row, log-spaced from [`GRID_FMIN`] to [`GRID_FMAX`].
pub const GRID_BINS: usize = 2048;
pub const GRID_FMIN: f64 = 10.0;
pub const GRID_FMAX: f64 = 24000.0;
/// Hops per second in the app. The reference tests run at 60, Nostalgia+'s frame rate.
pub const DEFAULT_HOP_RATE: f64 = 120.0;
/// The largest transform any profile uses.
const MAX_FFT: usize = 32768;
/// Samples metered per call, so the meter's scratch buffers stay a fixed size.
const METER_CHUNK: usize = 1024;

/// Everything the analysis needs from the settings and the layout.
#[derive(Clone, Debug, PartialEq)]
pub struct AnalysisConfig {
    pub quality: AnalysisQuality,
    pub window: WindowType,
    pub aggregate: BandAggregate,
    pub tilt_db_per_octave: f64,
    pub pair_mode: ChannelPairMode,
    pub interp: CurveInterpolation,
    pub filter: FilteringAmount,
    pub attack_ms: f64,
    pub release_ms: f64,
    pub peak_decay_db_per_sec: f64,
    pub average_seconds: f64,
    pub adaptive_range: bool,
    pub contrast: f64,
    pub floor_db: f64,
    pub ceiling_db: f64,
    /// History rows per second of audio.
    pub rows_per_second: f64,
    /// The display axis the curves are computed on.
    pub scale: FreqScale,
    pub columns: usize,
    pub fmin: f64,
    pub fmax: f64,
}

impl AnalysisConfig {
    /// The analysis for `settings`, with curves `columns` wide (the pane's height).
    pub fn from_settings(s: &Settings, columns: usize) -> AnalysisConfig {
        AnalysisConfig {
            quality: s.quality,
            window: s.window,
            aggregate: s.aggregate,
            tilt_db_per_octave: s.tilt_db_per_octave,
            pair_mode: s.pair_mode,
            interp: s.interp,
            filter: s.filter,
            attack_ms: s.attack_ms,
            // Cinematic lengthens the fall, not the rise: hits still land sharply but
            // let go slowly, which reads as a trail.
            release_ms: if s.imm_cinematic {
                s.release_ms * 3.0
            } else {
                s.release_ms
            },
            peak_decay_db_per_sec: s.peak_decay_db_per_sec,
            average_seconds: s.average_seconds,
            adaptive_range: s.adaptive_range,
            contrast: s.contrast,
            floor_db: s.floor_db,
            ceiling_db: s.ceiling_db,
            rows_per_second: s.effective_rows_per_second(),
            scale: s.scale,
            columns,
            fmin: s.fmin,
            fmax: s.fmax,
        }
    }

    /// The display map at a sample rate: the top clamped to Nyquist, and a log axis kept
    /// clear of 0 Hz.
    pub fn map(&self, sample_rate: f64) -> FrequencyMap {
        let fmax = self.fmax.min(sample_rate * 0.5);
        let fmin = if self.scale == FreqScale::Linear {
            self.fmin.max(0.0)
        } else {
            self.fmin.max(10.0)
        };
        FrequencyMap::new(self.scale, self.columns.max(1), fmin, fmax)
    }
}

/// What one hop produced. Borrowed from the engine; copy what you keep.
#[derive(Debug)]
pub struct Hop<'a> {
    /// Hops since the engine started, from 0.
    pub index: u64,
    /// Frames of audio consumed when this hop ran.
    pub frames: u64,
    /// False until there's enough audio for the largest transform.
    pub analysed: bool,
    /// What this hop's analysis took, in seconds.
    pub seconds: f64,
    pub floor_db: f64,
    pub ceiling_db: f64,
    /// The panes: one for single-channel modes, two otherwise.
    pub panes: &'a [ChannelCurves],
    pub map: &'a FrequencyMap,
    pub features: &'a MusicFeatures,
    pub loudness: LoudnessReadings,
    /// True when this hop also produced a history row.
    pub pushed_row: bool,
    /// The newest audio, oldest first: everything the engine keeps, at least as long as
    /// the largest transform once there's been that much.
    pub recent_left: &'a [f32],
    pub recent_right: &'a [f32],
}

/// One spectrogram row.
#[derive(Debug)]
pub struct Row<'a> {
    /// Rows since the engine started, from 0.
    pub index: u64,
    /// Frames of audio consumed when the row was cut: its timestamp.
    pub frames: u64,
    /// The colour range in force when the row was cut.
    pub floor_db: f64,
    pub ceiling_db: f64,
    /// Levels on the fixed grid, dBFS. `b` equals `a` for single-channel modes.
    pub a: &'a [f64],
    pub b: &'a [f64],
    /// Waveform extremes since the previous row: left min, left max, right min, right max.
    pub wave: [f32; 4],
}

/// Receives the engine's output.
pub trait Sink {
    fn hop(&mut self, hop: &Hop<'_>);
    fn row(&mut self, row: &Row<'_>);
}

/// A sink that ignores everything.
#[derive(Debug, Default)]
pub struct NullSink;

impl Sink for NullSink {
    fn hop(&mut self, _hop: &Hop<'_>) {}
    fn row(&mut self, _row: &Row<'_>) {}
}

#[derive(Debug)]
pub struct Engine {
    sample_rate: f64,
    hop_rate: f64,
    hop_frames: f64,
    config: AnalysisConfig,

    analyzer: SpectrumAnalyzer,
    map: FrequencyMap,
    grid: FrequencyMap,
    panes: Vec<ChannelCurves>,
    spare: Vec<f64>,
    grid_a: Vec<f64>,
    grid_b: Vec<f64>,
    features: MusicFeatures,
    range: DynamicRange,
    meter: LoudnessMeter,
    meter_l: Vec<f64>,
    meter_r: Vec<f64>,
    floor_db: f64,
    ceiling_db: f64,

    // The newest audio, contiguous: [start, end) of buffers twice the size kept.
    hist_l: Vec<f32>,
    hist_r: Vec<f32>,
    start: usize,
    end: usize,

    frames: u64,
    next_hop: f64,
    hops: u64,
    rows: u64,
    // Rows are timed from here, so the first row lands a whole row after the first
    // analysed hop, as Nostalgia+'s scroll divider did.
    row_origin: Option<u64>,
    rows_at_origin: u64,
    wave: [f32; 4],
}

impl Engine {
    pub fn new(sample_rate: f64, hop_rate: f64, config: AnalysisConfig) -> Engine {
        let sample_rate = if sample_rate > 0.0 {
            sample_rate
        } else {
            48000.0
        };
        let hop_rate = if hop_rate > 0.0 {
            hop_rate
        } else {
            DEFAULT_HOP_RATE
        };
        let keep = MAX_FFT + (sample_rate / hop_rate).ceil() as usize + METER_CHUNK;
        let mut meter = LoudnessMeter::new();
        meter.configure(sample_rate);
        let mut e = Engine {
            sample_rate,
            hop_rate,
            hop_frames: sample_rate / hop_rate,
            map: config.map(sample_rate),
            grid: FrequencyMap::new(FreqScale::Log, GRID_BINS, GRID_FMIN, GRID_FMAX),
            config,
            analyzer: SpectrumAnalyzer::new(),
            panes: Vec::new(),
            spare: Vec::new(),
            grid_a: vec![0.0; GRID_BINS],
            grid_b: vec![0.0; GRID_BINS],
            features: MusicFeatures::new(hop_rate),
            range: DynamicRange::new(),
            meter,
            meter_l: vec![0.0; METER_CHUNK],
            meter_r: vec![0.0; METER_CHUNK],
            floor_db: -95.0,
            ceiling_db: -5.0,
            hist_l: vec![0.0; keep * 2],
            hist_r: vec![0.0; keep * 2],
            start: 0,
            end: 0,
            frames: 0,
            next_hop: sample_rate / hop_rate,
            hops: 0,
            rows: 0,
            row_origin: None,
            rows_at_origin: 0,
            wave: EMPTY_WAVE,
        };
        e.fit_panes();
        e
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    pub fn hop_rate(&self) -> f64 {
        self.hop_rate
    }

    pub fn config(&self) -> &AnalysisConfig {
        &self.config
    }

    /// Frames consumed so far: the engine's clock.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    pub fn features_mut(&mut self) -> &mut MusicFeatures {
        &mut self.features
    }

    /// Takes a new configuration. Only what changed is rebuilt; the history and meters
    /// carry on.
    pub fn set_config(&mut self, config: AnalysisConfig) {
        if config == self.config {
            return;
        }
        if config.rows_per_second != self.config.rows_per_second {
            // Carry on from the rows already cut instead of jumping.
            self.row_origin = self.row_origin.map(|_| self.frames);
            self.rows_at_origin = self.rows;
        }
        let remap = config.scale != self.config.scale
            || config.columns != self.config.columns
            || config.fmin != self.config.fmin
            || config.fmax != self.config.fmax;
        self.config = config;
        if remap {
            self.map = self.config.map(self.sample_rate);
        }
        self.fit_panes();
    }

    /// A new track: the programme measures start again, as do the range and features.
    pub fn reset_track(&mut self) {
        self.meter.reset();
        self.range.reset();
        self.features.reset();
    }

    fn fit_panes(&mut self) {
        let count = self.config.pair_mode.pane_count();
        let width = self.map.width;
        self.panes.resize_with(count, ChannelCurves::default);
        for p in &mut self.panes {
            p.resize(width);
        }
        self.spare.resize(width, 0.0);
    }

    /// Feeds interleaved stereo frames.
    pub fn push_interleaved(&mut self, frames: &[f32], sink: &mut impl Sink) {
        // Split into small planar batches through the history buffer itself.
        for chunk in frames.chunks(2 * METER_CHUNK) {
            let n = chunk.len() / 2;
            let mut l = [0.0f32; METER_CHUNK];
            let mut r = [0.0f32; METER_CHUNK];
            for (i, f) in chunk.as_chunks::<2>().0.iter().enumerate() {
                l[i] = f[0];
                r[i] = f[1];
            }
            self.push(&l[..n], &r[..n], sink);
        }
    }

    /// Feeds planar stereo frames: `left` and `right` of equal length.
    pub fn push(&mut self, left: &[f32], right: &[f32], sink: &mut impl Sink) {
        let n = left.len().min(right.len());
        let mut i = 0;
        while i < n {
            let hop_at = self.next_hop.round() as u64;
            let until = hop_at.saturating_sub(self.frames).max(1) as usize;
            let take = until.min(n - i).min(METER_CHUNK);
            self.append(&left[i..i + take], &right[i..i + take]);
            i += take;
            if self.frames >= hop_at {
                self.next_hop += self.hop_frames;
                self.hop(sink);
            }
        }
    }

    fn append(&mut self, l: &[f32], r: &[f32]) {
        let n = l.len();
        let keep = self.hist_l.len() / 2;
        if self.end + n > self.hist_l.len() {
            // Slide the newest `keep` frames back to the front.
            let from = self.end.saturating_sub(keep).max(self.start);
            self.hist_l.copy_within(from..self.end, 0);
            self.hist_r.copy_within(from..self.end, 0);
            self.end -= from;
            self.start = 0;
        }
        self.hist_l[self.end..self.end + n].copy_from_slice(l);
        self.hist_r[self.end..self.end + n].copy_from_slice(r);
        self.end += n;
        if self.end - self.start > keep {
            self.start = self.end - keep;
        }

        for (k, (&a, &b)) in l.iter().zip(r).enumerate() {
            self.meter_l[k] = a as f64;
            self.meter_r[k] = b as f64;
            self.wave[0] = self.wave[0].min(a);
            self.wave[1] = self.wave[1].max(a);
            self.wave[2] = self.wave[2].min(b);
            self.wave[3] = self.wave[3].max(b);
        }
        self.meter.process(&self.meter_l[..n], &self.meter_r[..n]);
        self.frames += n as u64;
    }

    fn hop(&mut self, sink: &mut impl Sink) {
        let started = std::time::Instant::now();
        let index = self.hops;
        self.hops += 1;
        let dt = 1.0 / self.hop_rate;
        let c = &self.config;
        self.analyzer
            .configure(self.sample_rate, c.quality, c.window);
        self.analyzer.set_hop_frames(self.hop_frames);

        let (left, right) = (
            &self.hist_l[self.start..self.end],
            &self.hist_r[self.start..self.end],
        );
        let two = self.panes.len() > 1;
        let analysed = {
            // A single pane takes the second output, as Nostalgia+'s did when both
            // outputs were the same array: the pair transform rounds its two halves
            // differently in the last bit.
            let (first, rest) = self.panes.split_at_mut(1);
            let (out_a, out_b): (&mut [f64], &mut [f64]) = if two {
                (first[0].raw_mut(), rest[0].raw_mut())
            } else {
                (&mut self.spare, first[0].raw_mut())
            };
            self.analyzer.compute_stereo(
                left,
                right,
                &self.map,
                out_a,
                out_b,
                c.aggregate,
                c.tilt_db_per_octave,
                c.pair_mode,
            )
        };

        let mut pushed_row = false;
        if analysed {
            for p in &mut self.panes {
                p.update(
                    dt,
                    c.interp,
                    c.filter,
                    c.attack_ms,
                    c.release_ms,
                    c.peak_decay_db_per_sec,
                    c.average_seconds,
                );
            }
            // One channel drives the features: both share their onsets.
            self.features.update(self.panes[0].raw(), dt);

            if c.adaptive_range {
                self.range.low_percentile = c.contrast.clamp(0.01, 0.95);
                self.range.observe(self.panes[0].raw(), 0.94);
                if two {
                    self.range.observe(self.panes[1].raw(), 1.0);
                }
                self.range.update(dt);
                self.floor_db = self.range.floor();
                self.ceiling_db = self.range.ceiling();
            } else {
                self.floor_db = c.floor_db;
                self.ceiling_db = c.ceiling_db;
            }
            if self.ceiling_db - self.floor_db < 1.0 {
                self.ceiling_db = self.floor_db + 1.0;
            }

            let origin = *self
                .row_origin
                .get_or_insert(self.frames - self.hop_frames.round() as u64);
            let elapsed = (self.frames - origin) as f64 / self.sample_rate;
            let due = self.rows_at_origin + (elapsed * c.rows_per_second + 1e-9).floor() as u64;
            if due > self.rows {
                pushed_row = true;
                self.cut_row(sink);
            }
        }

        let n = self.end - self.start;
        sink.hop(&Hop {
            index,
            frames: self.frames,
            analysed,
            seconds: started.elapsed().as_secs_f64(),
            floor_db: self.floor_db,
            ceiling_db: self.ceiling_db,
            panes: &self.panes,
            map: &self.map,
            features: &self.features,
            loudness: self.meter.readings(),
            pushed_row,
            recent_left: &self.hist_l[self.end - n..self.end],
            recent_right: &self.hist_r[self.end - n..self.end],
        });
    }

    fn cut_row(&mut self, sink: &mut impl Sink) {
        let c = &self.config;
        let two = self.panes.len() > 1;
        self.analyzer.reproject(
            &self.grid,
            &mut self.grid_a,
            Some(&mut self.grid_b),
            c.aggregate,
            c.tilt_db_per_octave,
        );
        let a: &[f64] = if two { &self.grid_a } else { &self.grid_b };
        sink.row(&Row {
            index: self.rows,
            frames: self.frames,
            floor_db: self.floor_db,
            ceiling_db: self.ceiling_db,
            a,
            b: &self.grid_b,
            wave: if self.wave[0] > self.wave[1] {
                [0.0; 4]
            } else {
                self.wave
            },
        });
        self.rows += 1;
        self.wave = EMPTY_WAVE;
    }
}

const EMPTY_WAVE: [f32; 4] = [
    f32::INFINITY,
    f32::NEG_INFINITY,
    f32::INFINITY,
    f32::NEG_INFINITY,
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[derive(Default)]
    struct Collect {
        hops: Vec<(u64, u64, bool, bool)>,
        rows: Vec<(u64, u64)>,
    }

    impl Sink for Collect {
        fn hop(&mut self, h: &Hop<'_>) {
            self.hops
                .push((h.index, h.frames, h.analysed, h.pushed_row));
        }
        fn row(&mut self, r: &Row<'_>) {
            assert_eq!(r.a.len(), GRID_BINS);
            self.rows.push((r.index, r.frames));
        }
    }

    fn tone(n: usize, rate: f64) -> Vec<f32> {
        (0..n)
            .map(|i| (0.3 * (2.0 * PI * 440.0 * i as f64 / rate).sin()) as f32)
            .collect()
    }

    #[test]
    fn hops_do_not_depend_on_chunking() {
        let config = AnalysisConfig::from_settings(&Settings::default(), 300);
        let x = tone(48000 * 2, 48000.0);
        let mut whole = Collect::default();
        Engine::new(48000.0, 120.0, config.clone()).push(&x, &x, &mut whole);
        let mut bits = Collect::default();
        let mut e = Engine::new(48000.0, 120.0, config);
        for (k, c) in x.chunks(37).enumerate() {
            if k % 3 == 0 {
                let inter: Vec<f32> = c.iter().flat_map(|&v| [v, v]).collect();
                e.push_interleaved(&inter, &mut bits);
            } else {
                e.push(c, c, &mut bits);
            }
        }
        assert_eq!(whole.hops, bits.hops);
        assert_eq!(whole.rows, bits.rows);
        assert_eq!(whole.hops.len(), 240);
        assert!(whole.hops.iter().all(|h| h.1 % 400 == 0));
    }

    #[test]
    fn rows_follow_audio_time() {
        // 60 rows per second at 120 hops: a row every other analysed hop.
        let config = AnalysisConfig::from_settings(&Settings::default(), 200);
        let x = tone(48000 * 3, 48000.0);
        let mut out = Collect::default();
        Engine::new(48000.0, 120.0, config).push(&x, &x, &mut out);
        let analysed = out.hops.iter().filter(|h| h.2).count();
        assert!(
            (out.rows.len() as i64 - analysed as i64 / 2).abs() <= 1,
            "{} rows",
            out.rows.len()
        );
        // At 44.1 kHz hops are 367.5 frames long, so they alternate 367 and 368.
        let mut out = Collect::default();
        let config = AnalysisConfig::from_settings(&Settings::default(), 200);
        Engine::new(44100.0, 120.0, config).push(
            &tone(44100, 44100.0),
            &tone(44100, 44100.0),
            &mut out,
        );
        assert_eq!(out.hops.len(), 120);
        assert_eq!(out.hops.last().unwrap().1, 44100);
    }
}
