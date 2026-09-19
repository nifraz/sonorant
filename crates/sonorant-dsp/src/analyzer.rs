//! The multi-resolution spectrum analyser.
//!
//! One FFT size forces one compromise across the whole band: big enough to separate
//! bass notes means smeared transients up top, and the reverse. This runs several sizes
//! over the same instant and stitches them, so bin density roughly follows the log
//! frequency axis instead of fighting it. At 48 kHz the Balanced profile resolves about
//! 2.9 Hz below 300 Hz, one semitone at 50 Hz, while keeping 47 Hz bins above 3 kHz
//! where transients live.

use crate::fft::{Complex, Fft};
use crate::frequency_map::{FreqScale, FrequencyMap, log2_ratio};
use crate::math;
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

/// Windows at least this long (8K and up at 48 kHz) are "long": see
/// [`SpectrumAnalyzer::set_hop_frames`].
const LONG_BAND_SECONDS: f64 = 0.15;
/// How often a long band refreshes, at most, in calls per second of audio.
const LONG_BAND_RATE: f64 = 60.0;
/// Projection plans kept at once: the display's, the history grid's, and one spare so a
/// resize doesn't evict the grid's.
const PLAN_SLOTS: usize = 3;

#[derive(Debug)]
struct SubBand {
    fft: Fft,
    n: usize,
    hi_hz: f64,
    window: Vec<f64>,
    window_gain: f64,
    /// Unscaled power per bin, 0..=n/2: a bin's magnitude is `pow.sqrt() * scale`.
    pow_l: Vec<f64>,
    pow_r: Vec<f64>,
    scale: f64,
    bin_width: f64,
    /// The band transforms on calls where `calls % stride == phase`.
    stride: u64,
    phase: u64,
    due: bool,
}

/// What a compute call transformed. A change refreshes every band at once, so no band
/// shows the old signal after a switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Input {
    Single(ChannelMode),
    Pair(ChannelPairMode),
}

/// Where a column reads one band's spectrum.
#[derive(Clone, Copy, Debug)]
enum Read {
    /// Bins `k0..=k1`, combined by the aggregate.
    Bins { band: u32, k0: u32, k1: u32 },
    /// A column narrower than one bin: interpolated at its centre, so the low end reads
    /// as a smooth ridge rather than a staircase.
    Lerp {
        band: u32,
        i0: u32,
        i1: u32,
        frac: f64,
    },
}

/// One column of a projection: the band that owns its centre, blended with a neighbour
/// inside a crossover, as `v * (1 - t) + other * t`, upper neighbour first.
#[derive(Clone, Copy, Debug)]
struct Column {
    primary: Read,
    upper: Option<(Read, f64)>,
    lower: Option<(Read, f64)>,
}

/// What a projection was built for. Maps with the same axis, size and range have the
/// same edges, so these identify one.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PlanKey {
    scale: FreqScale,
    width: usize,
    fmin: f64,
    fmax: f64,
    tilt: f64,
    bands: u64,
}

/// A map's columns resolved against the bands: which bins each reads, the blend
/// weights and the tilt. Built when the axis, size, tilt or profile changes, so a hop
/// does no edge arithmetic and takes one logarithm per column and channel.
#[derive(Clone, Debug, Default)]
struct Projection {
    key: Option<PlanKey>,
    columns: Vec<Column>,
    /// Each column's tilt in dB, apart so the conversion to dB is one tight loop.
    tilt: Vec<f64>,
    last_used: u64,
}

/// Several FFT sizes over the same instant, stitched into one spectrum per display
/// column. Configuring allocates; computing does not, except to plan a map it hasn't
/// seen.
#[derive(Debug)]
pub struct SpectrumAnalyzer {
    bands: Vec<SubBand>,
    max_n: usize,
    sample_rate: f64,
    window: WindowType,
    quality: AnalysisQuality,
    hop_frames: f64,
    calls: u64,
    last_input: Option<Input>,
    /// Bumped whenever the bands are rebuilt, which makes every plan stale.
    generation: u64,
    plans: [Projection; PLAN_SLOTS],
    plan_clock: u64,
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
            max_n: 0,
            sample_rate: 48000.0,
            window: WindowType::Hann,
            quality: AnalysisQuality::Balanced,
            hop_frames: 0.0,
            calls: 0,
            last_input: None,
            generation: 0,
            plans: Default::default(),
            plan_clock: 0,
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
                    pow_l: vec![0.0; n / 2 + 1],
                    pow_r: vec![0.0; n / 2 + 1],
                    scale: 0.0,
                    bin_width: sample_rate / n as f64,
                    stride: 1,
                    phase: 0,
                    due: true,
                }
            })
            .collect();
        self.max_n = sizes.iter().copied().max().unwrap_or(0);
        self.last_input = None;
        self.generation += 1;
        self.assign_strides();
    }

    /// Tells the analyser how many frames pass between compute calls, so the long
    /// transforms can skip calls. At 120 hops a second a window of 150 ms or more moves
    /// by about 5% of its length or less per hop (the 16K window by 2.4%), so
    /// transforming it every hop shows nothing new but costs most of the hop. Such bands
    /// refresh at most 60 times a second of audio instead, taking turns when there are
    /// two. At 60 hops a second or slower, and by default, every band refreshes on every
    /// call.
    pub fn set_hop_frames(&mut self, frames: f64) {
        if self.hop_frames != frames {
            self.hop_frames = frames;
            self.assign_strides();
        }
    }

    fn assign_strides(&mut self) {
        let stride = if self.hop_frames > 0.0 {
            ((self.sample_rate / LONG_BAND_RATE / self.hop_frames + 1e-9).floor() as u64).max(1)
        } else {
            1
        };
        let mut turn = 0;
        for band in &mut self.bands {
            if band.n as f64 >= LONG_BAND_SECONDS * self.sample_rate {
                band.stride = stride;
                band.phase = turn % stride;
                turn += 1;
            } else {
                band.stride = 1;
                band.phase = 0;
            }
        }
    }

    /// Starts a compute call: marks the bands that transform this time.
    fn begin(&mut self, input: Input) {
        let all = self.last_input != Some(input);
        self.last_input = Some(input);
        let call = self.calls;
        self.calls += 1;
        for band in &mut self.bands {
            band.due = all || call % band.stride == band.phase;
        }
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
        self.begin(Input::Single(channel));
        for band in self.bands.iter_mut().filter(|b| b.due) {
            let (l, r) = (&left[left.len() - band.n..], &right[right.len() - band.n..]);
            let input = band.fft.input_mut();
            let w = &band.window;
            // Sums and differences are formed in f32, as the capture ring held them.
            match channel {
                ChannelMode::Left => fill(input, w, l, r, |a, _| (a as f64, 0.0)),
                ChannelMode::Right => fill(input, w, l, r, |_, b| (b as f64, 0.0)),
                ChannelMode::Side => fill(input, w, l, r, |a, b| ((a - b) as f64 * 0.5, 0.0)),
                ChannelMode::Mid => fill(input, w, l, r, |a, b| ((a + b) as f64 * 0.5, 0.0)),
            }
            band.scale = band.fft.transform_power(band.window_gain, &mut band.pow_l);
        }
        let plan = self.plan(map, tilt_db_per_octave);
        self.project(plan, out, None, aggregate);
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
        self.begin(Input::Pair(mode));
        for band in self.bands.iter_mut().filter(|b| b.due) {
            let (l, r) = (&left[left.len() - band.n..], &right[right.len() - band.n..]);
            let input = band.fft.input_mut();
            let w = &band.window;
            let f = |a: f32| a as f64;
            match mode {
                ChannelPairMode::MidSide => fill(input, w, l, r, |a, b| {
                    ((f(a) + f(b)) * 0.5, (f(a) - f(b)) * 0.5)
                }),
                ChannelPairMode::LeftOnly => fill(input, w, l, r, |a, _| (f(a), f(a))),
                ChannelPairMode::RightOnly => fill(input, w, l, r, |_, b| (f(b), f(b))),
                ChannelPairMode::LeftRight => fill(input, w, l, r, |a, b| (f(a), f(b))),
            }
            band.scale =
                band.fft
                    .transform_power_pair(band.window_gain, &mut band.pow_l, &mut band.pow_r);
        }
        let plan = self.plan(map, tilt_db_per_octave);
        self.project(plan, out_a, Some(out_b), aggregate);
        true
    }

    /// Projects the spectrum of the last compute call onto another set of columns, such
    /// as the history store's fixed grid, without transforming again. `out_b` is only
    /// meaningful after [`compute_stereo`](Self::compute_stereo).
    pub fn reproject(
        &mut self,
        map: &FrequencyMap,
        out_a: &mut [f64],
        out_b: Option<&mut [f64]>,
        aggregate: BandAggregate,
        tilt_db_per_octave: f64,
    ) {
        if !self.bands.is_empty() {
            let plan = self.plan(map, tilt_db_per_octave);
            self.project(plan, out_a, out_b, aggregate);
        }
    }

    /// The slot holding the plan for `map` at this tilt, building it if need be in the
    /// least recently used slot.
    fn plan(&mut self, map: &FrequencyMap, tilt_db_per_octave: f64) -> usize {
        let key = PlanKey {
            scale: map.scale,
            width: map.width,
            fmin: map.fmin,
            fmax: map.fmax,
            tilt: tilt_db_per_octave,
            bands: self.generation,
        };
        self.plan_clock += 1;
        let slot = match self.plans.iter().position(|p| p.key == Some(key)) {
            Some(slot) => slot,
            None => {
                let slot = (0..PLAN_SLOTS)
                    .min_by_key(|&i| self.plans[i].last_used)
                    .unwrap_or(0);
                let mut plan = std::mem::take(&mut self.plans[slot]);
                plan.key = Some(key);
                plan.columns.clear();
                plan.columns
                    .extend((0..map.width).map(|x| self.plan_column(map, x)));
                plan.tilt.clear();
                plan.tilt.extend(map.centres.iter().map(|&fc| {
                    if tilt_db_per_octave != 0.0 && fc > 0.0 {
                        tilt_db_per_octave * log2_ratio(fc / 1000.0)
                    } else {
                        0.0
                    }
                }));
                self.plans[slot] = plan;
                slot
            }
        };
        self.plans[slot].last_used = self.plan_clock;
        slot
    }

    fn plan_column(&self, map: &FrequencyMap, x: usize) -> Column {
        let (f0, f1, fc) = (map.edges[x], map.edges[x + 1], map.centres[x]);
        // The band that owns the centre, blended with a neighbour inside a crossover.
        let last = self.bands.len() - 1;
        let idx = self.bands.iter().position(|b| fc < b.hi_hz).unwrap_or(last);
        let read = |i: usize| plan_read(&self.bands[i], i as u32, f0, f1, fc);
        let mut upper = None;
        if idx < last {
            let lo = self.bands[idx].hi_hz / BLEND_RATIO;
            if fc > lo {
                let t = (log2_ratio(fc / lo) / log2_ratio(BLEND_RATIO)).min(1.0);
                upper = Some((read(idx + 1), t));
            }
        }
        let mut lower = None;
        if idx > 0 {
            let hi = self.bands[idx - 1].hi_hz * BLEND_RATIO;
            if fc < hi {
                let t = (log2_ratio(hi / fc) / log2_ratio(BLEND_RATIO)).min(1.0);
                lower = Some((read(idx - 1), t));
            }
        }
        Column {
            primary: read(idx),
            upper,
            lower,
        }
    }

    fn project(
        &self,
        plan: usize,
        out_a: &mut [f64],
        out_b: Option<&mut [f64]>,
        aggregate: BandAggregate,
    ) {
        let plan = &self.plans[plan];
        let w = plan.columns.len().min(out_a.len());
        match out_b {
            Some(out_b) => {
                let w = w.min(out_b.len());
                let (out_a, out_b) = (&mut out_a[..w], &mut out_b[..w]);
                for ((c, a), b) in plan
                    .columns
                    .iter()
                    .zip(out_a.iter_mut())
                    .zip(out_b.iter_mut())
                {
                    (*a, *b) = self.column::<true>(c, aggregate);
                }
                to_db(out_a, &plan.tilt);
                to_db(out_b, &plan.tilt);
            }
            None => {
                let out_a = &mut out_a[..w];
                for (c, a) in plan.columns.iter().zip(out_a.iter_mut()) {
                    *a = self.column::<false>(c, aggregate).0;
                }
                to_db(out_a, &plan.tilt);
            }
        }
    }

    /// Both channels' magnitudes for one column, in linear amplitude. Without `TWO`
    /// only the left is read and the right is zero.
    #[inline]
    fn column<const TWO: bool>(&self, c: &Column, aggregate: BandAggregate) -> (f64, f64) {
        let (mut a, mut b) = self.read::<TWO>(c.primary, aggregate);
        for (r, t) in [c.upper, c.lower].into_iter().flatten() {
            let (oa, ob) = self.read::<TWO>(r, aggregate);
            a = a * (1.0 - t) + oa * t;
            b = b * (1.0 - t) + ob * t;
        }
        (a, b)
    }

    #[inline]
    fn read<const TWO: bool>(&self, r: Read, aggregate: BandAggregate) -> (f64, f64) {
        match r {
            Read::Bins { band, k0, k1 } => {
                let b = &self.bands[band as usize];
                let range = k0 as usize..=k1 as usize;
                let (pl, pr) = (&b.pow_l[range.clone()], &b.pow_r[range]);
                // The root of the largest power is the largest magnitude, to the bit.
                let (x, y) = match aggregate {
                    BandAggregate::Peak if TWO => {
                        pl.iter().zip(pr).fold((0.0, 0.0), |(x, y), (&p, &q)| {
                            (if p > x { p } else { x }, if q > y { q } else { y })
                        })
                    }
                    BandAggregate::Peak => {
                        (pl.iter().fold(0.0, |x, &p| if p > x { p } else { x }), 0.0)
                    }
                    BandAggregate::Energy if TWO => pl
                        .iter()
                        .zip(pr)
                        .fold((0.0, 0.0), |(x, y), (&p, &q)| (x + p, y + q)),
                    BandAggregate::Energy => (pl.iter().sum::<f64>(), 0.0),
                };
                (
                    x.sqrt() * b.scale,
                    if TWO { y.sqrt() * b.scale } else { 0.0 },
                )
            }
            Read::Lerp { band, i0, i1, frac } => {
                let b = &self.bands[band as usize];
                let (i0, i1) = (i0 as usize, i1 as usize);
                let lerp = |pow: &[f64]| {
                    (pow[i0].sqrt() * b.scale) * (1.0 - frac) + (pow[i1].sqrt() * b.scale) * frac
                };
                (lerp(&b.pow_l), if TWO { lerp(&b.pow_r) } else { 0.0 })
            }
        }
    }
}

/// Windows two channels into a transform's input: the real and imaginary parts are what
/// `pick` makes of each pair of samples.
#[inline]
fn fill(
    input: &mut [Complex<f64>],
    window: &[f64],
    left: &[f32],
    right: &[f32],
    pick: impl Fn(f32, f32) -> (f64, f64),
) {
    for (((d, &w), &a), &b) in input.iter_mut().zip(window).zip(left).zip(right) {
        let (x, y) = pick(a, b);
        *d = Complex::new(x * w, y * w);
    }
}

/// Magnitudes to dBFS in place, plus each column's tilt, floored at [`FLOOR_DB`].
fn to_db(values: &mut [f64], tilt: &[f64]) {
    for (v, &t) in values.iter_mut().zip(tilt) {
        let db = if *v > 1e-12 {
            20.0 * math::log10(*v) + t
        } else {
            FLOOR_DB
        };
        *v = db.max(FLOOR_DB);
    }
}

/// Where the column `f0..f1` centred on `fc` reads band `b` (number `index`).
fn plan_read(b: &SubBand, index: u32, f0: f64, f1: f64, fc: f64) -> Read {
    let half = (b.n / 2) as i64;
    let k0 = ((f0 / b.bin_width).ceil() as i64).max(0);
    let k1 = ((f1 / b.bin_width).floor() as i64).min(half);
    if k1 >= k0 {
        return Read::Bins {
            band: index,
            k0: k0 as u32,
            k1: k1 as u32,
        };
    }
    let pos = fc / b.bin_width;
    let i0 = (pos.floor() as i64).max(0);
    if i0 >= half {
        let half = half as u32;
        return Read::Bins {
            band: index,
            k0: half,
            k1: half,
        };
    }
    Read::Lerp {
        band: index,
        i0: i0 as u32,
        i1: (i0 + 1).min(half) as u32,
        frac: pos - i0 as f64,
    }
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
