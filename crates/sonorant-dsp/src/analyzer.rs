//! The multi-resolution spectrum analyser.
//!
//! One FFT size forces one compromise across the whole band: big enough to separate
//! bass notes means smeared transients up top, and the reverse. This runs several sizes
//! over the same instant and stitches them, so bin density roughly follows the log
//! frequency axis instead of fighting it. At 48 kHz the Balanced profile resolves about
//! 2.9 Hz below 300 Hz, one semitone at 50 Hz, while keeping 47 Hz bins above 3 kHz
//! where transients live.

use crate::fft::Fft;
use crate::frequency_map::{FrequencyMap, log2_ratio};
use crate::window::{self, WindowType};

/// The level reported for bins with no measurable energy, in dBFS.
pub const FLOOR_DB: f64 = -140.0;

/// Width of the crossover blend, as a ratio either side of each corner.
const BLEND_RATIO: f64 = 1.35;

/// FFT profiles. The values are pinned, as in Nostalgia+'s settings files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AnalysisQuality {
    /// One 4096-point transform.
    Fast = 0,
    /// 16384, 4096 and 1024 points, crossing at 300 Hz and 3 kHz.
    #[default]
    Balanced = 1,
    /// 32768, 8192, 2048 and 512 points, crossing at 250 Hz, 2 kHz and 8 kHz.
    High = 2,
    /// 4096, 1024 and 256 points, crossing at 600 Hz and 4 kHz.
    ///
    /// Every window ends at "now", so a band's effective time is half its length ago:
    /// High's 32K bass window is centred about 340 ms back and its 512 treble window
    /// about 5 ms back, so bass visibly trails treble on the same hit. Capping the
    /// largest transform holds the delay near 43 ms at the cost of bass detail.
    LowLatency = 3,
}

impl AnalysisQuality {
    pub const ALL: [AnalysisQuality; 4] = [
        AnalysisQuality::Fast,
        AnalysisQuality::Balanced,
        AnalysisQuality::High,
        AnalysisQuality::LowLatency,
    ];

    pub fn name(self) -> &'static str {
        match self {
            AnalysisQuality::Fast => "Fast",
            AnalysisQuality::Balanced => "Balanced",
            AnalysisQuality::High => "High",
            AnalysisQuality::LowLatency => "LowLatency",
        }
    }

    pub fn from_name(name: &str) -> Option<AnalysisQuality> {
        Self::ALL
            .into_iter()
            .find(|q| q.name().eq_ignore_ascii_case(name))
    }

    /// Transform sizes, largest first, and the corners between them. The last band
    /// runs to Nyquist.
    pub fn profile(self) -> (&'static [usize], &'static [f64]) {
        match self {
            AnalysisQuality::Fast => (&[4096], &[]),
            AnalysisQuality::Balanced => (&[16384, 4096, 1024], &[300.0, 3000.0]),
            AnalysisQuality::High => (&[32768, 8192, 2048, 512], &[250.0, 2000.0, 8000.0]),
            AnalysisQuality::LowLatency => (&[4096, 1024, 256], &[600.0, 4000.0]),
        }
    }
}

/// How the bins inside one display column combine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BandAggregate {
    /// The loudest bin: tones keep their true height.
    #[default]
    Peak,
    /// The summed energy: noise reads at its true level.
    Energy,
}

impl BandAggregate {
    pub const ALL: [BandAggregate; 2] = [BandAggregate::Peak, BandAggregate::Energy];

    pub fn name(self) -> &'static str {
        match self {
            BandAggregate::Peak => "Peak",
            BandAggregate::Energy => "Energy",
        }
    }

    pub fn from_name(name: &str) -> Option<BandAggregate> {
        Self::ALL
            .into_iter()
            .find(|a| a.name().eq_ignore_ascii_case(name))
    }
}

/// One signal made from the two channels, for single-channel analysis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChannelMode {
    #[default]
    Mid,
    Left,
    Right,
    Side,
}

impl ChannelMode {
    pub const ALL: [ChannelMode; 4] = [
        ChannelMode::Mid,
        ChannelMode::Left,
        ChannelMode::Right,
        ChannelMode::Side,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ChannelMode::Mid => "Mid",
            ChannelMode::Left => "Left",
            ChannelMode::Right => "Right",
            ChannelMode::Side => "Side",
        }
    }

    pub fn from_name(name: &str) -> Option<ChannelMode> {
        Self::ALL
            .into_iter()
            .find(|m| m.name().eq_ignore_ascii_case(name))
    }
}

/// Which two signals the two panes show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ChannelPairMode {
    #[default]
    LeftRight,
    MidSide,
    LeftOnly,
    RightOnly,
}

impl ChannelPairMode {
    pub const ALL: [ChannelPairMode; 4] = [
        ChannelPairMode::LeftRight,
        ChannelPairMode::MidSide,
        ChannelPairMode::LeftOnly,
        ChannelPairMode::RightOnly,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ChannelPairMode::LeftRight => "LeftRight",
            ChannelPairMode::MidSide => "MidSide",
            ChannelPairMode::LeftOnly => "LeftOnly",
            ChannelPairMode::RightOnly => "RightOnly",
        }
    }

    pub fn from_name(name: &str) -> Option<ChannelPairMode> {
        Self::ALL
            .into_iter()
            .find(|m| m.name().eq_ignore_ascii_case(name))
    }

    /// Pane labels, outer pane first.
    pub fn pane_labels(self) -> [&'static str; 2] {
        match self {
            ChannelPairMode::MidSide => ["MID", "SIDE"],
            ChannelPairMode::LeftOnly => ["LEFT", "LEFT"],
            ChannelPairMode::RightOnly => ["RIGHT", "RIGHT"],
            ChannelPairMode::LeftRight => ["L", "R"],
        }
    }

    /// Single-channel modes show one full-width pane instead of two.
    pub fn pane_count(self) -> usize {
        match self {
            ChannelPairMode::LeftOnly | ChannelPairMode::RightOnly => 1,
            _ => 2,
        }
    }
}

#[derive(Debug)]
struct SubBand {
    fft: Fft,
    n: usize,
    hi_hz: f64,
    window: Vec<f64>,
    window_gain: f64,
    mag_l: Vec<f64>,
    mag_r: Vec<f64>,
    frame_l: Vec<f64>,
    frame_r: Vec<f64>,
    bin_width: f64,
}

/// Several FFT sizes over the same instant, stitched into one spectrum per display
/// column. Configuring allocates; computing does not.
#[derive(Debug)]
pub struct SpectrumAnalyzer {
    bands: Vec<SubBand>,
    snap_l: Vec<f64>,
    snap_r: Vec<f64>,
    max_n: usize,
    sample_rate: f64,
    window: WindowType,
    quality: AnalysisQuality,
}

impl Default for SpectrumAnalyzer {
    fn default() -> Self {
        SpectrumAnalyzer::new()
    }
}

impl SpectrumAnalyzer {
    /// An unconfigured analyser; [`SpectrumAnalyzer::configure`] before computing.
    pub fn new() -> SpectrumAnalyzer {
        SpectrumAnalyzer {
            bands: Vec::new(),
            snap_l: Vec::new(),
            snap_r: Vec::new(),
            max_n: 0,
            sample_rate: 48000.0,
            window: WindowType::Hann,
            quality: AnalysisQuality::Balanced,
        }
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// Frames of history a compute call needs: the largest transform.
    pub fn largest_fft(&self) -> usize {
        self.max_n
    }

    pub fn is_configured(&self) -> bool {
        !self.bands.is_empty()
    }

    /// The transform sizes, as the status line shows them: "16K / 4K / 1K".
    pub fn describe_resolution(&self) -> String {
        if self.bands.is_empty() {
            return "-".to_owned();
        }
        let parts: Vec<String> = self
            .bands
            .iter()
            .map(|b| {
                if b.n / 1024 > 0 {
                    format!("{}K", b.n / 1024)
                } else {
                    b.n.to_string()
                }
            })
            .collect();
        parts.join(" / ")
    }

    /// Sets the rate, profile and window. Does nothing when none of them changed.
    pub fn configure(&mut self, sample_rate: f64, quality: AnalysisQuality, window: WindowType) {
        let sample_rate = if sample_rate > 0.0 {
            sample_rate
        } else {
            48000.0
        };
        if self.sample_rate == sample_rate
            && self.quality == quality
            && self.window == window
            && !self.bands.is_empty()
        {
            return;
        }
        self.sample_rate = sample_rate;
        self.quality = quality;
        self.window = window;

        let nyquist = sample_rate * 0.5;
        let (sizes, corners) = quality.profile();
        self.bands = sizes
            .iter()
            .enumerate()
            .map(|(i, &n)| {
                let (window, window_gain) = window::build(window, n);
                SubBand {
                    fft: Fft::new(n),
                    n,
                    hi_hz: if i == sizes.len() - 1 {
                        nyquist
                    } else {
                        corners[i]
                    },
                    window,
                    window_gain,
                    mag_l: vec![0.0; n / 2 + 1],
                    mag_r: vec![0.0; n / 2 + 1],
                    frame_l: vec![0.0; n],
                    frame_r: vec![0.0; n],
                    bin_width: sample_rate / n as f64,
                }
            })
            .collect();
        self.max_n = sizes.iter().copied().max().unwrap_or(0);
        self.snap_l = vec![0.0; self.max_n];
        self.snap_r = vec![0.0; self.max_n];
    }

    /// Single-channel analysis of the newest [`largest_fft`](Self::largest_fft) frames.
    /// `left` and `right` are the channel histories, oldest first; only their tails are
    /// read. Fills one dBFS level per display column. False when there is not enough
    /// history yet or the analyser is not configured.
    #[allow(clippy::too_many_arguments)]
    pub fn compute(
        &mut self,
        left: &[f32],
        right: &[f32],
        channel: ChannelMode,
        map: &FrequencyMap,
        out: &mut [f64],
        aggregate: BandAggregate,
        tilt_db_per_octave: f64,
    ) -> bool {
        let n = self.max_n;
        if self.bands.is_empty() || left.len() < n || right.len() < n {
            return false;
        }
        let (l, r) = (&left[left.len() - n..], &right[right.len() - n..]);
        for ((d, &a), &b) in self.snap_l.iter_mut().zip(l).zip(r) {
            // Sums and differences are formed in f32, as the capture ring held them.
            *d = match channel {
                ChannelMode::Left => a as f64,
                ChannelMode::Right => b as f64,
                ChannelMode::Side => (a - b) as f64 * 0.5,
                ChannelMode::Mid => (a + b) as f64 * 0.5,
            };
        }
        for band in &mut self.bands {
            band.frame_l.copy_from_slice(&self.snap_l[n - band.n..]);
            band.fft.magnitude_real(
                &band.frame_l,
                &band.window,
                band.window_gain,
                &mut band.mag_l,
            );
        }
        self.project(map, out, None, aggregate, tilt_db_per_octave);
        true
    }

    /// Two-channel analysis for whichever pair is on display, from one complex FFT per
    /// sub-band. Mid/side is formed before the transform, so it costs what left/right
    /// costs.
    #[allow(clippy::too_many_arguments)]
    pub fn compute_stereo(
        &mut self,
        left: &[f32],
        right: &[f32],
        map: &FrequencyMap,
        out_a: &mut [f64],
        out_b: &mut [f64],
        aggregate: BandAggregate,
        tilt_db_per_octave: f64,
        mode: ChannelPairMode,
    ) -> bool {
        let n = self.max_n;
        if self.bands.is_empty() || left.len() < n || right.len() < n {
            return false;
        }
        let (l, r) = (&left[left.len() - n..], &right[right.len() - n..]);
        for (i, (&a, &b)) in l.iter().zip(r).enumerate() {
            let (a, b) = (a as f64, b as f64);
            (self.snap_l[i], self.snap_r[i]) = match mode {
                ChannelPairMode::MidSide => ((a + b) * 0.5, (a - b) * 0.5),
                ChannelPairMode::LeftOnly => (a, a),
                ChannelPairMode::RightOnly => (b, b),
                ChannelPairMode::LeftRight => (a, b),
            };
        }
        for band in &mut self.bands {
            band.frame_l.copy_from_slice(&self.snap_l[n - band.n..]);
            band.frame_r.copy_from_slice(&self.snap_r[n - band.n..]);
            band.fft.magnitude_real_pair(
                &band.frame_l,
                &band.frame_r,
                &band.window,
                band.window_gain,
                &mut band.mag_l,
                &mut band.mag_r,
            );
        }
        self.project(map, out_a, Some(out_b), aggregate, tilt_db_per_octave);
        true
    }

    /// Projects the spectrum of the last compute call onto another set of columns, such
    /// as the history store's fixed grid, without transforming again. `out_b` is only
    /// meaningful after [`compute_stereo`](Self::compute_stereo).
    pub fn reproject(
        &self,
        map: &FrequencyMap,
        out_a: &mut [f64],
        out_b: Option<&mut [f64]>,
        aggregate: BandAggregate,
        tilt_db_per_octave: f64,
    ) {
        if !self.bands.is_empty() {
            self.project(map, out_a, out_b, aggregate, tilt_db_per_octave);
        }
    }

    fn project(
        &self,
        map: &FrequencyMap,
        out_a: &mut [f64],
        mut out_b: Option<&mut [f64]>,
        aggregate: BandAggregate,
        tilt_db_per_octave: f64,
    ) {
        let mut w = map.width.min(out_a.len());
        if let Some(b) = &out_b {
            w = w.min(b.len());
        }
        for x in 0..w {
            let (f0, f1, fc) = (map.edges[x], map.edges[x + 1], map.centres[x]);
            let tilt = if tilt_db_per_octave != 0.0 && fc > 0.0 {
                tilt_db_per_octave * log2_ratio(fc / 1000.0)
            } else {
                0.0
            };
            out_a[x] = to_db(self.blended_magnitude(f0, f1, fc, aggregate, false), tilt);
            if let Some(b) = out_b.as_deref_mut() {
                b[x] = to_db(self.blended_magnitude(f0, f1, fc, aggregate, true), tilt);
            }
        }
    }

    fn blended_magnitude(
        &self,
        f0: f64,
        f1: f64,
        fc: f64,
        aggregate: BandAggregate,
        right: bool,
    ) -> f64 {
        // The band that owns the centre, blended with a neighbour inside a crossover.
        let last = self.bands.len() - 1;
        let idx = self.bands.iter().position(|b| fc < b.hi_hz).unwrap_or(last);
        let mut primary = band_magnitude(&self.bands[idx], f0, f1, fc, aggregate, right);

        if idx < last {
            let lo = self.bands[idx].hi_hz / BLEND_RATIO;
            if fc > lo {
                let t = (log2_ratio(fc / lo) / log2_ratio(BLEND_RATIO)).min(1.0);
                let other = band_magnitude(&self.bands[idx + 1], f0, f1, fc, aggregate, right);
                primary = primary * (1.0 - t) + other * t;
            }
        }
        if idx > 0 {
            let hi = self.bands[idx - 1].hi_hz * BLEND_RATIO;
            if fc < hi {
                let t = (log2_ratio(hi / fc) / log2_ratio(BLEND_RATIO)).min(1.0);
                let other = band_magnitude(&self.bands[idx - 1], f0, f1, fc, aggregate, right);
                primary = primary * (1.0 - t) + other * t;
            }
        }
        primary
    }
}

fn to_db(mag: f64, tilt: f64) -> f64 {
    let db = if mag > 1e-12 {
        20.0 * mag.log10() + tilt
    } else {
        FLOOR_DB
    };
    db.max(FLOOR_DB)
}

fn band_magnitude(
    b: &SubBand,
    f0: f64,
    f1: f64,
    fc: f64,
    aggregate: BandAggregate,
    right: bool,
) -> f64 {
    let mag = if right { &b.mag_r } else { &b.mag_l };
    let half = (b.n / 2) as i64;
    let k0 = ((f0 / b.bin_width).ceil() as i64).max(0);
    let k1 = ((f1 / b.bin_width).floor() as i64).min(half);

    if k1 >= k0 {
        let bins = &mag[k0 as usize..=k1 as usize];
        return match aggregate {
            BandAggregate::Energy => bins.iter().map(|m| m * m).sum::<f64>().sqrt(),
            BandAggregate::Peak => bins.iter().fold(0.0, |m, &v| if v > m { v } else { m }),
        };
    }

    // A column narrower than one bin: interpolate at its centre so the low end reads as
    // a smooth ridge rather than a staircase.
    let pos = fc / b.bin_width;
    let i0 = (pos.floor() as i64).max(0);
    if i0 >= half {
        return mag[half as usize];
    }
    let i1 = (i0 + 1).min(half);
    let frac = pos - i0 as f64;
    mag[i0 as usize] * (1.0 - frac) + mag[i1 as usize] * frac
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frequency_map::FreqScale;
    use std::f64::consts::PI;

    const SR: f64 = 48000.0;

    fn tone(n: usize, parts: &[(f64, f64)]) -> Vec<f32> {
        (0..n)
            .map(|i| {
                parts
                    .iter()
                    .map(|&(a, f)| a * (2.0 * PI * f * i as f64 / SR).sin())
                    .sum::<f64>() as f32
            })
            .collect()
    }

    fn peak_near(db: &[f64], map: &FrequencyMap, freq: f64, window: usize) -> usize {
        let centre = (0..map.width)
            .min_by(|&a, &b| {
                (map.centres[a] - freq)
                    .abs()
                    .total_cmp(&(map.centres[b] - freq).abs())
            })
            .unwrap();
        if window == 0 {
            return centre;
        }
        let (lo, hi) = (
            centre.saturating_sub(window),
            (centre + window).min(db.len() - 1),
        );
        (lo..=hi).fold(lo, |p, i| if db[i] > db[p] { i } else { p })
    }

    #[test]
    fn tones_land_on_their_columns_at_their_levels() {
        let mut an = SpectrumAnalyzer::new();
        an.configure(SR, AnalysisQuality::Balanced, WindowType::Hann);
        let x = tone(40000, &[(0.25, 100.0), (0.5, 1000.0)]);
        let map = FrequencyMap::new(FreqScale::Note, 1200, 20.0, 20000.0);
        let mut db = vec![0.0; 1200];
        assert!(an.compute(
            &x,
            &x,
            ChannelMode::Mid,
            &map,
            &mut db,
            BandAggregate::Peak,
            0.0
        ));
        let (p1, p2) = (
            peak_near(&db, &map, 100.0, 40),
            peak_near(&db, &map, 1000.0, 40),
        );
        assert!((map.centres[p1] - 100.0).abs() < 3.0);
        assert!((map.centres[p2] - 1000.0).abs() < 12.0);
        assert!((db[p1] + 12.04).abs() < 3.0);
        assert!((db[p2] + 6.02).abs() < 3.0);
        assert!(((db[p2] - db[p1]) - 6.02).abs() < 1.5);
        let wide = db.iter().filter(|&&v| v > db[p2] - 20.0).count();
        assert!(wide < 120, "{wide} columns within 20 dB of the peak");
    }

    #[test]
    fn stereo_channels_do_not_leak() {
        let mut an = SpectrumAnalyzer::new();
        an.configure(SR, AnalysisQuality::Balanced, WindowType::Hann);
        let l = tone(40000, &[(0.5, 1000.0)]);
        let r = tone(40000, &[(0.25, 3000.0)]);
        let map = FrequencyMap::new(FreqScale::Note, 1200, 20.0, 20000.0);
        let (mut a, mut b) = (vec![0.0; 1200], vec![0.0; 1200]);
        assert!(an.compute_stereo(
            &l,
            &r,
            &map,
            &mut a,
            &mut b,
            BandAggregate::Peak,
            0.0,
            ChannelPairMode::LeftRight
        ));
        let (l1, r3) = (
            peak_near(&a, &map, 1000.0, 40),
            peak_near(&b, &map, 3000.0, 40),
        );
        assert!((a[l1] + 6.02).abs() < 3.0 && (b[r3] + 12.04).abs() < 3.0);
        assert!(a[peak_near(&a, &map, 3000.0, 0)] < a[l1] - 60.0);
        assert!(b[peak_near(&b, &map, 1000.0, 0)] < b[r3] - 60.0);
    }

    #[test]
    fn needs_a_full_window_of_history() {
        let mut an = SpectrumAnalyzer::new();
        let map = FrequencyMap::new(FreqScale::Note, 100, 20.0, 20000.0);
        let mut out = vec![0.0; 100];
        let x = vec![0.0f32; 20000];
        assert!(!an.compute(
            &x,
            &x,
            ChannelMode::Mid,
            &map,
            &mut out,
            BandAggregate::Peak,
            0.0
        ));
        an.configure(SR, AnalysisQuality::High, WindowType::Hann);
        assert_eq!(an.largest_fft(), 32768);
        assert!(!an.compute(
            &x,
            &x,
            ChannelMode::Mid,
            &map,
            &mut out,
            BandAggregate::Peak,
            0.0
        ));
        assert_eq!(an.describe_resolution(), "32K / 8K / 2K / 512");
    }
}
