//! Loudness and stereo metering to ITU-R BS.1770-4.
//!
//! Two K-weighting stages (a +4 dB high shelf and a 38 Hz high-pass) run per channel,
//! then mean square is integrated over sliding windows: 400 ms for momentary and 3 s
//! for short-term. True peak uses 4x polyphase oversampling, which catches the
//! inter-sample peaks a sample-peak reading misses: the usual reason a "0 dBFS" master
//! still clips a converter. Correlation and balance come from the unweighted signal
//! over 400 ms.
//!
//! Integrated loudness and loudness range are gated over the whole programme:
//! momentary blocks for the first and three-second blocks for the second, each through
//! an absolute -70 LUFS gate and then a relative one.

use std::f64::consts::PI;
use std::sync::OnceLock;

/// Direct-form-II transposed biquad.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Biquad {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    pub fn new(b0: f64, b1: f64, b2: f64, a1: f64, a2: f64) -> Biquad {
        Biquad {
            b0,
            b1,
            b2,
            a1,
            a2,
            z1: 0.0,
            z2: 0.0,
        }
    }

    /// `[b0, b1, b2, a1, a2]`, with `a0` normalised to 1.
    pub fn coefficients(&self) -> [f64; 5] {
        [self.b0, self.b1, self.b2, self.a1, self.a2]
    }

    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

// The K-weighting prototype, as published in BS.1770.
const SHELF_F0: f64 = 1681.974450955533;
const SHELF_GAIN_DB: f64 = 3.999843853973347;
const SHELF_Q: f64 = 0.7071752369554196;
const HP_F0: f64 = 38.13547087602444;
const HP_Q: f64 = 0.5003270373238773;

/// The K-weighting shelf and high-pass for a sample rate.
///
/// At 48 kHz these are the coefficients BS.1770-4 publishes. Anywhere else they are
/// designed from the prototype, which lands about 0.2 dB away from the published
/// response: shared-mode mixes are almost always 48 kHz, so that rate gets the exact
/// values.
pub fn k_weighting(sample_rate: f64) -> (Biquad, Biquad) {
    if (sample_rate - 48000.0).abs() < 0.5 {
        return (
            Biquad::new(
                1.53512485958697,
                -2.69169618940638,
                1.19839281085285,
                -1.69065929318241,
                0.73248077421585,
            ),
            Biquad::new(1.0, -2.0, 1.0, -1.99004745483398, 0.99007225036621),
        );
    }
    (
        high_shelf(sample_rate, SHELF_F0, SHELF_GAIN_DB, SHELF_Q),
        high_pass(sample_rate, HP_F0, HP_Q),
    )
}

/// RBJ cookbook high shelf.
fn high_shelf(fs: f64, f0: f64, gain_db: f64, q: f64) -> Biquad {
    let a = 10.0f64.powf(gain_db / 40.0);
    let w0 = 2.0 * PI * f0 / fs;
    let cosw = w0.cos();
    let alpha = w0.sin() / (2.0 * q);
    let sqrt_a = a.sqrt();

    let nb0 = a * ((a + 1.0) + (a - 1.0) * cosw + 2.0 * sqrt_a * alpha);
    let nb1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cosw);
    let nb2 = a * ((a + 1.0) + (a - 1.0) * cosw - 2.0 * sqrt_a * alpha);
    let na0 = (a + 1.0) - (a - 1.0) * cosw + 2.0 * sqrt_a * alpha;
    let na1 = 2.0 * ((a - 1.0) - (a + 1.0) * cosw);
    let na2 = (a + 1.0) - (a - 1.0) * cosw - 2.0 * sqrt_a * alpha;
    Biquad::new(nb0 / na0, nb1 / na0, nb2 / na0, na1 / na0, na2 / na0)
}

/// RBJ high-pass poles with BS.1770's numerator, `[1, -2, 1]`, rather than the
/// unity-gain cookbook one.
fn high_pass(fs: f64, f0: f64, q: f64) -> Biquad {
    let w0 = 2.0 * PI * f0 / fs;
    let cosw = w0.cos();
    let alpha = w0.sin() / (2.0 * q);
    let na0 = 1.0 + alpha;
    Biquad::new(1.0, -2.0, 1.0, (-2.0 * cosw) / na0, (1.0 - alpha) / na0)
}

/// The 4-phase, 8-tap windowed-sinc interpolator used for true peak.
pub fn true_peak_phases() -> &'static [[f64; 8]; 4] {
    static PHASES: OnceLock<[[f64; 8]; 4]> = OnceLock::new();
    PHASES.get_or_init(|| {
        const PHASES: usize = 4;
        const TAPS: usize = 8;
        let mut result = [[0.0; TAPS]; PHASES];
        for (p, h) in result.iter_mut().enumerate() {
            let mut sum = 0.0;
            for (k, tap) in h.iter_mut().enumerate() {
                let t = (k as f64 - (TAPS / 2 - 1) as f64) - p as f64 / PHASES as f64;
                let s = if t.abs() < 1e-9 {
                    1.0
                } else {
                    (PI * t).sin() / (PI * t)
                };
                // Blackman across the tap span.
                let wpos =
                    ((k as f64 + p as f64 / PHASES as f64) / (TAPS as f64 - 1.0)).clamp(0.0, 1.0);
                let w = 0.42 - 0.5 * (2.0 * PI * wpos).cos() + 0.08 * (4.0 * PI * wpos).cos();
                *tap = s * w;
                sum += *tap;
            }
            if sum.abs() > 1e-12 {
                for tap in h.iter_mut() {
                    *tap /= sum;
                }
            }
        }
        result
    })
}

/// Everything the meter reports, as one copyable value to publish.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoudnessReadings {
    /// LUFS over the last 400 ms.
    pub momentary: f64,
    /// LUFS over the last 3 s.
    pub short_term: f64,
    /// Gated LUFS since the last reset: the figure you quote.
    pub integrated: f64,
    /// LU between the 10th and 95th percentile of the gated three-second blocks: how
    /// much the track moves, rather than how loud it is.
    pub range: f64,
    /// dBTP, held with a 2 dB/s decay.
    pub true_peak_db: f64,
    /// True peak over RMS, in dB.
    pub crest_db: f64,
    /// +1 identical, 0 unrelated, -1 inverted.
    pub correlation: f64,
    /// -1 fully left, 0 centred, +1 fully right.
    pub balance: f64,
    /// True-peak excursions past -1 dBTP since the last reset.
    pub overs: u32,
    /// When the last over happened, in seconds of audio since the reset.
    pub last_over_seconds: f64,
}

impl Default for LoudnessReadings {
    fn default() -> Self {
        LoudnessReadings {
            momentary: -70.0,
            short_term: -70.0,
            integrated: -70.0,
            range: 0.0,
            true_peak_db: -70.0,
            crest_db: 0.0,
            correlation: 0.0,
            balance: 0.0,
            overs: 0,
            last_over_seconds: 0.0,
        }
    }
}

/// Above this a lossy encoder will clip even though the samples did not.
pub const OVER_THRESHOLD_DB: f64 = -1.0;
/// Excursions closer together than this are one event, or a loud master sitting near
/// the ceiling counts an over on every half cycle.
const OVER_HOLDOFF_SECONDS: f64 = 0.200;

// Gated block loudness goes into a 0.1 LU histogram rather than a list: gating needs a
// second pass over every block there has ever been, and a histogram answers that in
// fixed memory however long the programme runs.
const HIST_BINS: usize = 751; // -70.0 to +5.0 LUFS
const HIST_LO: f64 = -70.0;
const HIST_STEP: f64 = 0.1;

/// The meter. Configuring allocates; processing does not.
#[derive(Clone, Debug)]
pub struct LoudnessMeter {
    shelf: [Biquad; 2],
    hp: [Biquad; 2],
    sample_rate: f64,
    configured: bool,

    ms_window: Vec<f64>,
    ms_write: usize,
    ms_filled: usize,
    momentary_len: usize,
    short_len: usize,
    sum_momentary: f64,
    sum_short: f64,

    sum_cross: f64,
    sum_ll: f64,
    sum_rr: f64,
    corr_l: Vec<f64>,
    corr_r: Vec<f64>,
    corr_write: usize,
    corr_len: usize,
    corr_filled: usize,

    hist_m: Box<[u32; HIST_BINS]>,
    hist_s: Box<[u32; HIST_BINS]>,
    block_step: usize,
    block_countdown: usize,
    long_step: usize,
    long_countdown: usize,
    gates_dirty: bool,

    over_hold_samples: usize,
    over_holdoff: usize,
    clock_samples: u64,

    true_peak: f64,
    peak_decay_per_sample: f64,
    tp_hist_l: [f64; 8],
    tp_hist_r: [f64; 8],
    tp_pos: usize,

    readings: LoudnessReadings,
}

impl Default for LoudnessMeter {
    fn default() -> Self {
        LoudnessMeter::new()
    }
}

impl LoudnessMeter {
    /// An unconfigured meter; [`LoudnessMeter::configure`] before processing.
    pub fn new() -> LoudnessMeter {
        LoudnessMeter {
            shelf: [Biquad::default(); 2],
            hp: [Biquad::default(); 2],
            sample_rate: 0.0,
            configured: false,
            ms_window: Vec::new(),
            ms_write: 0,
            ms_filled: 0,
            momentary_len: 0,
            short_len: 0,
            sum_momentary: 0.0,
            sum_short: 0.0,
            sum_cross: 0.0,
            sum_ll: 0.0,
            sum_rr: 0.0,
            corr_l: Vec::new(),
            corr_r: Vec::new(),
            corr_write: 0,
            corr_len: 0,
            corr_filled: 0,
            hist_m: Box::new([0; HIST_BINS]),
            hist_s: Box::new([0; HIST_BINS]),
            block_step: 1,
            block_countdown: 1,
            long_step: 1,
            long_countdown: 1,
            gates_dirty: false,
            over_hold_samples: 1,
            over_holdoff: 0,
            clock_samples: 0,
            true_peak: 0.0,
            peak_decay_per_sample: 1.0,
            tp_hist_l: [0.0; 8],
            tp_hist_r: [0.0; 8],
            tp_pos: 0,
            readings: LoudnessReadings::default(),
        }
    }

    pub fn readings(&self) -> LoudnessReadings {
        self.readings
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// Builds the filters and windows for a sample rate. Does nothing if the rate is
    /// unchanged; a new rate starts every measure afresh.
    pub fn configure(&mut self, sample_rate: f64) {
        let sample_rate = if sample_rate > 0.0 {
            sample_rate
        } else {
            48000.0
        };
        if self.configured && self.sample_rate == sample_rate {
            return;
        }
        self.sample_rate = sample_rate;
        self.configured = true;

        let (shelf, hp) = k_weighting(sample_rate);
        self.shelf = [shelf; 2];
        self.hp = [hp; 2];

        self.momentary_len = (sample_rate * 0.400) as usize;
        self.short_len = (sample_rate * 3.000) as usize;
        self.ms_window = vec![0.0; self.short_len];
        self.ms_write = 0;
        self.ms_filled = 0;
        self.sum_momentary = 0.0;
        self.sum_short = 0.0;

        self.corr_len = (sample_rate * 0.400) as usize;
        self.corr_l = vec![0.0; self.corr_len];
        self.corr_r = vec![0.0; self.corr_len];
        self.corr_write = 0;
        self.corr_filled = 0;
        self.sum_cross = 0.0;
        self.sum_ll = 0.0;
        self.sum_rr = 0.0;

        // A block every 100 ms and every second: 400 ms blocks overlapping by 75% and
        // 3 s blocks by two thirds, which is what the two measures are defined on.
        self.block_step = ((sample_rate * 0.100) as usize).max(1);
        self.long_step = ((sample_rate * 1.000) as usize).max(1);
        self.block_countdown = self.block_step;
        self.long_countdown = self.long_step;
        self.hist_m.fill(0);
        self.hist_s.fill(0);
        self.over_hold_samples = ((sample_rate * OVER_HOLDOFF_SECONDS) as usize).max(1);
        self.over_holdoff = 0;
        self.clock_samples = 0;
        self.readings.overs = 0;
        self.readings.last_over_seconds = 0.0;
        self.readings.integrated = -70.0;
        self.readings.range = 0.0;

        self.tp_hist_l = [0.0; 8];
        self.tp_hist_r = [0.0; 8];
        self.tp_pos = 0;
        self.true_peak = 0.0;
        self.peak_decay_per_sample = 10.0f64.powf(-(2.0 / 20.0) / sample_rate); // ~2 dB/s
    }

    /// Clears the programme measures: a new track starts from nothing.
    pub fn reset(&mut self) {
        if !self.configured {
            return;
        }
        for f in self.shelf.iter_mut().chain(self.hp.iter_mut()) {
            f.reset();
        }
        self.ms_window.fill(0.0);
        self.ms_write = 0;
        self.ms_filled = 0;
        self.sum_momentary = 0.0;
        self.sum_short = 0.0;
        self.corr_l.fill(0.0);
        self.corr_r.fill(0.0);
        self.corr_write = 0;
        self.corr_filled = 0;
        self.sum_cross = 0.0;
        self.sum_ll = 0.0;
        self.sum_rr = 0.0;
        self.true_peak = 0.0;
        self.hist_m.fill(0);
        self.hist_s.fill(0);
        self.block_countdown = self.block_step;
        self.long_countdown = self.long_step;
        self.clock_samples = 0;
        self.over_holdoff = 0;
        self.readings.overs = 0;
        self.readings.last_over_seconds = 0.0;
        self.readings.integrated = -70.0;
        self.readings.range = 0.0;
    }

    /// Meters every sample of `left` and `right` once, in order.
    pub fn process(&mut self, left: &[f64], right: &[f64]) {
        let count = left.len().min(right.len());
        if !self.configured || count == 0 {
            return;
        }
        let over_level = 10.0f64.powf(OVER_THRESHOLD_DB / 20.0);
        let phases = true_peak_phases();

        for i in 0..count {
            let (l, r) = (left[i], right[i]);

            // K-weighted mean square, summed across channels (G = 1.0 each).
            let kl = self.hp[0].process(self.shelf[0].process(l));
            let kr = self.hp[1].process(self.shelf[1].process(r));
            let ms = kl * kl + kr * kr;

            if self.ms_filled == self.short_len {
                self.sum_short -= self.ms_window[self.ms_write];
                let mi = if self.ms_write >= self.momentary_len {
                    self.ms_write - self.momentary_len
                } else {
                    self.ms_write + self.short_len - self.momentary_len
                };
                self.sum_momentary -= self.ms_window[mi];
            } else if self.ms_write >= self.momentary_len {
                self.sum_momentary -= self.ms_window[self.ms_write - self.momentary_len];
            }
            self.ms_window[self.ms_write] = ms;
            self.sum_short += ms;
            self.sum_momentary += ms;
            self.ms_write += 1;
            if self.ms_write >= self.short_len {
                self.ms_write = 0;
            }
            if self.ms_filled < self.short_len {
                self.ms_filled += 1;
            }

            // A block is only taken once its window is full, or the first few would be
            // quiet by accident and drag the gated mean down.
            self.block_countdown -= 1;
            if self.block_countdown == 0 {
                self.block_countdown = self.block_step;
                if self.ms_filled >= self.momentary_len {
                    add_block(
                        &mut self.hist_m,
                        self.sum_momentary / self.momentary_len as f64,
                    );
                    self.gates_dirty = true;
                }
            }
            self.long_countdown -= 1;
            if self.long_countdown == 0 {
                self.long_countdown = self.long_step;
                if self.ms_filled >= self.short_len {
                    add_block(&mut self.hist_s, self.sum_short / self.short_len as f64);
                    self.gates_dirty = true;
                }
            }

            // Correlation and balance, on the unweighted signal.
            if self.corr_filled == self.corr_len {
                let (ol, or) = (self.corr_l[self.corr_write], self.corr_r[self.corr_write]);
                self.sum_cross -= ol * or;
                self.sum_ll -= ol * ol;
                self.sum_rr -= or * or;
            }
            self.corr_l[self.corr_write] = l;
            self.corr_r[self.corr_write] = r;
            self.sum_cross += l * r;
            self.sum_ll += l * l;
            self.sum_rr += r * r;
            self.corr_write += 1;
            if self.corr_write >= self.corr_len {
                self.corr_write = 0;
            }
            if self.corr_filled < self.corr_len {
                self.corr_filled += 1;
            }

            // True peak, 4x oversampled.
            self.tp_hist_l[self.tp_pos] = l;
            self.tp_hist_r[self.tp_pos] = r;
            self.tp_pos = (self.tp_pos + 1) & 7;
            let mut local_peak = 0.0f64;
            for h in phases {
                let (mut al, mut ar) = (0.0, 0.0);
                for (k, &tap) in h.iter().enumerate() {
                    let idx = (self.tp_pos + k) & 7;
                    al += self.tp_hist_l[idx] * tap;
                    ar += self.tp_hist_r[idx] * tap;
                }
                let m = al.abs().max(ar.abs());
                if m > local_peak {
                    local_peak = m;
                }
            }
            self.true_peak *= self.peak_decay_per_sample;
            if local_peak > self.true_peak {
                self.true_peak = local_peak;
            }

            // One over per moment, not per sample: a clipped passage is one event to the
            // ear and thousands to a naive counter.
            if self.over_holdoff > 0 {
                self.over_holdoff -= 1;
            }
            if local_peak > over_level && self.over_holdoff == 0 {
                self.readings.overs += 1;
                self.readings.last_over_seconds = self.clock_samples as f64 / self.sample_rate;
                self.over_holdoff = self.over_hold_samples;
            }
            self.clock_samples += 1;
        }

        if self.gates_dirty {
            self.gates_dirty = false;
            self.readings.integrated = gated(&self.hist_m, 10.0).0;
            let (_, gate) = gated(&self.hist_s, 20.0);
            // The spread of what clears the gate, not of everything: otherwise one
            // silent passage sets the bottom of the range.
            let hi = percentile(&self.hist_s, gate, 0.95);
            let lo = percentile(&self.hist_s, gate, 0.10);
            self.readings.range = if hi > -300.0 && lo > -300.0 {
                (hi - lo).max(0.0)
            } else {
                0.0
            };
        }

        let m_count = self.ms_filled.min(self.momentary_len);
        if m_count > 0 {
            self.readings.momentary =
                -0.691 + 10.0 * (self.sum_momentary / m_count as f64).max(1e-14).log10();
        }
        if self.ms_filled > 0 {
            self.readings.short_term =
                -0.691 + 10.0 * (self.sum_short / self.ms_filled as f64).max(1e-14).log10();
        }
        self.readings.true_peak_db = 20.0 * self.true_peak.max(1e-9).log10();

        if self.corr_filled > 0 {
            let n = self.corr_filled as f64;
            let denom = (self.sum_ll * self.sum_rr).sqrt();
            self.readings.correlation = if denom > 1e-12 {
                self.sum_cross / denom
            } else {
                0.0
            };
            let rms_l = (self.sum_ll / n).sqrt();
            let rms_r = (self.sum_rr / n).sqrt();
            let total = rms_l + rms_r;
            self.readings.balance = if total > 1e-9 {
                (rms_r - rms_l) / total
            } else {
                0.0
            };
            let rms = ((self.sum_ll + self.sum_rr) / (2.0 * n)).sqrt();
            self.readings.crest_db = if rms > 1e-9 {
                20.0 * (self.true_peak.max(1e-9) / rms).log10()
            } else {
                0.0
            };
        }
    }
}

/// One block's loudness into a histogram. Anything under -70 LUFS is silence as far as
/// the measure goes and is dropped here: that is the absolute gate.
fn add_block(hist: &mut [u32; HIST_BINS], mean_square: f64) {
    if mean_square <= 0.0 {
        return;
    }
    let l = -0.691 + 10.0 * mean_square.log10();
    if l < HIST_LO {
        return;
    }
    let i = ((l - HIST_LO) / HIST_STEP + 0.5).floor() as i64;
    hist[i.clamp(0, HIST_BINS as i64 - 1) as usize] += 1;
}

fn bin_lufs(i: usize) -> f64 {
    HIST_LO + i as f64 * HIST_STEP
}

/// The gated mean and the relative threshold it used: the mean of every block, then the
/// mean of the blocks above a threshold `relative_lu` below it.
fn gated(hist: &[u32; HIST_BINS], relative_lu: f64) -> (f64, f64) {
    let first = mean_above(hist, f64::NEG_INFINITY);
    if first <= -300.0 {
        return (-70.0, -300.0);
    }
    let threshold = first - relative_lu;
    let second = mean_above(hist, threshold);
    (if second <= -300.0 { -70.0 } else { second }, threshold)
}

/// Mean loudness of the blocks above a threshold, averaged as power: the mean of two
/// dB figures is not the dB of their mean, and the difference is the measurement.
fn mean_above(hist: &[u32; HIST_BINS], above: f64) -> f64 {
    let (mut sum, mut n) = (0.0, 0u64);
    for (i, &c) in hist.iter().enumerate() {
        if c == 0 {
            continue;
        }
        let l = bin_lufs(i);
        if l <= above {
            continue;
        }
        sum += c as f64 * 10.0f64.powf((l + 0.691) / 10.0);
        n += c as u64;
    }
    if n == 0 {
        -300.0
    } else {
        -0.691 + 10.0 * (sum / n as f64).log10()
    }
}

/// Loudness at a percentile of the blocks above a threshold.
fn percentile(hist: &[u32; HIST_BINS], above: f64, p: f64) -> f64 {
    let total: u64 = hist
        .iter()
        .enumerate()
        .filter(|&(i, &c)| c != 0 && bin_lufs(i) > above)
        .map(|(_, &c)| c as u64)
        .sum();
    if total == 0 {
        return -300.0;
    }
    let want = ((total as f64 * p).ceil() as u64).max(1);
    let mut run = 0u64;
    for (i, &c) in hist.iter().enumerate() {
        if c == 0 {
            continue;
        }
        let l = bin_lufs(i);
        if l <= above {
            continue;
        }
        run += c as u64;
        if run >= want {
            return l;
        }
    }
    -300.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f64 = 48000.0;

    fn sine(n: usize, rate: f64, f: f64, amp: f64) -> Vec<f64> {
        (0..n)
            .map(|i| amp * (2.0 * PI * f * i as f64 / rate).sin())
            .collect()
    }

    #[test]
    fn calibration_tone_reads_minus_23() {
        let mut m = LoudnessMeter::new();
        m.configure(SR);
        let x = sine(SR as usize * 4, SR, 1000.0, 10.0f64.powf(-23.0 / 20.0));
        m.process(&x, &x);
        let r = m.readings();
        assert!((r.short_term + 23.0).abs() < 0.35, "{}", r.short_term);
        assert!((r.momentary + 23.0).abs() < 0.35);
        assert!((r.correlation - 1.0).abs() < 0.01);
        assert!(r.balance.abs() < 0.01);
    }

    #[test]
    fn inverted_and_one_sided() {
        let x = sine(SR as usize * 4, SR, 300.0, 0.3);
        let neg: Vec<f64> = x.iter().map(|v| -v).collect();
        let zero = vec![0.0; x.len()];
        let mut m = LoudnessMeter::new();
        m.configure(SR);
        m.process(&x, &neg);
        assert!((m.readings().correlation + 1.0).abs() < 0.01);
        let mut m = LoudnessMeter::new();
        m.configure(SR);
        m.process(&zero, &x);
        assert!(m.readings().balance > 0.98);
    }

    #[test]
    fn full_scale_sine_is_near_zero_dbtp() {
        let mut m = LoudnessMeter::new();
        m.configure(SR);
        let x = sine(SR as usize * 4, SR, 997.0, 1.0);
        m.process(&x, &x);
        assert!(m.readings().true_peak_db.abs() < 1.0);
    }

    #[test]
    fn published_coefficients_at_48k_designed_elsewhere() {
        let (s, h) = k_weighting(48000.0);
        assert_eq!(s.b0, 1.53512485958697);
        assert_eq!(h.a2, 0.99007225036621);
        let (s, h) = k_weighting(44100.0);
        assert!(s.b0 > 1.0 && h.b1 == -2.0);
    }
}
