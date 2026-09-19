//! Curve shaping across frequency, ballistics over time, and the extremum traces.

use crate::analyzer::FLOOR_DB;

/// How the curve is drawn between bins.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CurveInterpolation {
    /// Untouched, so peaks keep their true height.
    PeakFlat,
    /// A moving average.
    #[default]
    LinearSmooth,
    /// The moving average plus a Catmull-Rom pass that rounds the shoulders.
    CubicSpline,
}

impl CurveInterpolation {
    pub const ALL: [CurveInterpolation; 3] = [
        CurveInterpolation::PeakFlat,
        CurveInterpolation::LinearSmooth,
        CurveInterpolation::CubicSpline,
    ];

    pub fn name(self) -> &'static str {
        match self {
            CurveInterpolation::PeakFlat => "PeakFlat",
            CurveInterpolation::LinearSmooth => "LinearSmooth",
            CurveInterpolation::CubicSpline => "CubicSpline",
        }
    }

    pub fn from_name(name: &str) -> Option<CurveInterpolation> {
        Self::ALL
            .into_iter()
            .find(|c| c.name().eq_ignore_ascii_case(name))
    }
}

/// How hard the curve is smoothed across frequency.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FilteringAmount {
    None,
    #[default]
    Light,
    Medium,
    Strong,
}

impl FilteringAmount {
    pub const ALL: [FilteringAmount; 4] = [
        FilteringAmount::None,
        FilteringAmount::Light,
        FilteringAmount::Medium,
        FilteringAmount::Strong,
    ];

    pub fn name(self) -> &'static str {
        match self {
            FilteringAmount::None => "None",
            FilteringAmount::Light => "Light",
            FilteringAmount::Medium => "Medium",
            FilteringAmount::Strong => "Strong",
        }
    }

    pub fn from_name(name: &str) -> Option<FilteringAmount> {
        Self::ALL
            .into_iter()
            .find(|f| f.name().eq_ignore_ascii_case(name))
    }

    /// Half-width of the moving-average kernel, in columns.
    pub fn kernel_radius(self) -> usize {
        match self {
            FilteringAmount::None => 0,
            FilteringAmount::Light => 1,
            FilteringAmount::Medium => 3,
            FilteringAmount::Strong => 6,
        }
    }
}

/// Smooths a dB curve across frequency into `dst`.
pub fn smooth(src: &[f64], dst: &mut [f64], interp: CurveInterpolation, amount: FilteringAmount) {
    let count = src.len().min(dst.len());
    if count == 0 {
        return;
    }
    let (src, dst) = (&src[..count], &mut dst[..count]);
    let r = amount.kernel_radius() as isize;
    let at = |i: isize| src[i.clamp(0, count as isize - 1) as usize];

    if interp == CurveInterpolation::PeakFlat || r == 0 {
        dst.copy_from_slice(src);
        if interp != CurveInterpolation::CubicSpline {
            return;
        }
    } else {
        let inv = 1.0 / (2 * r + 1) as f64;
        let mut acc = 0.0;
        for i in -r..=r {
            acc += at(i);
        }
        for (i, d) in dst.iter_mut().enumerate() {
            let i = i as isize;
            *d = acc * inv;
            acc -= at(i - r);
            acc += at(i + r + 1);
        }
    }

    if interp == CurveInterpolation::CubicSpline {
        // One Catmull-Rom evaluation at each sample's own position: it keeps the samples
        // but softens the joins.
        let mut prev = dst[0];
        for i in 1..count.saturating_sub(1) {
            let (p0, p1, p2) = (prev, dst[i], dst[i + 1]);
            prev = dst[i];
            dst[i] = 0.125 * p0 + 0.75 * p1 + 0.125 * p2;
        }
    }
}

/// Per-column maximum, minimum and average over time: a decaying peak hold, a rising
/// valley hold and an exponential mean. Cheap enough for every hop, and correct the
/// moment playback changes.
#[derive(Clone, Debug)]
pub struct ExtremumTracker {
    min: Vec<f64>,
    max: Vec<f64>,
    avg: Vec<f64>,
    /// Seconds for the average to follow a step change.
    pub average_seconds: f64,
    /// How fast the held extremes let go, in dB per second.
    pub hold_decay_db_per_second: f64,
}

impl Default for ExtremumTracker {
    fn default() -> Self {
        ExtremumTracker {
            min: Vec::new(),
            max: Vec::new(),
            avg: Vec::new(),
            average_seconds: 1.2,
            hold_decay_db_per_second: 14.0,
        }
    }
}

impl ExtremumTracker {
    pub fn new() -> ExtremumTracker {
        ExtremumTracker::default()
    }

    pub fn min(&self) -> &[f64] {
        &self.min
    }

    pub fn max(&self) -> &[f64] {
        &self.max
    }

    pub fn average(&self) -> &[f64] {
        &self.avg
    }

    pub fn resize(&mut self, n: usize) {
        if self.min.len() == n {
            return;
        }
        self.min = vec![0.0; n];
        self.max = vec![0.0; n];
        self.avg = vec![0.0; n];
        self.reset();
    }

    pub fn reset(&mut self) {
        self.min.fill(0.0);
        self.max.fill(FLOOR_DB);
        self.avg.fill(FLOOR_DB);
    }

    pub fn update(&mut self, db: &[f64], dt: f64) {
        if self.min.len() < db.len() {
            self.resize(db.len());
        }
        let drift = self.hold_decay_db_per_second * dt;
        let a = 1.0 - (-dt / self.average_seconds.max(0.01)).exp();
        for (i, &v) in db.iter().enumerate() {
            let mx = self.max[i] - drift;
            self.max[i] = if v > mx { v } else { mx };
            let mn = self.min[i] + drift;
            self.min[i] = if v < mn { v } else { mn };
            self.avg[i] += (v - self.avg[i]) * a;
        }
    }
}

/// One pane's curves: the raw spectrum, the smoothed one, the displayed one after
/// attack and release, and the extremum traces.
#[derive(Clone, Debug, Default)]
pub struct ChannelCurves {
    raw: Vec<f64>,
    shaped: Vec<f64>,
    display: Vec<f64>,
    extremes: ExtremumTracker,
}

impl ChannelCurves {
    pub fn new(n: usize) -> ChannelCurves {
        let mut c = ChannelCurves::default();
        c.resize(n);
        c
    }

    /// Resizes to `n` columns. A change of size starts the display from the floor.
    pub fn resize(&mut self, n: usize) {
        if self.raw.len() == n {
            return;
        }
        self.raw = vec![0.0; n];
        self.shaped = vec![0.0; n];
        self.display = vec![FLOOR_DB; n];
        self.extremes.resize(n);
    }

    pub fn len(&self) -> usize {
        self.raw.len()
    }

    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    /// The analyser writes here.
    pub fn raw_mut(&mut self) -> &mut [f64] {
        &mut self.raw
    }

    pub fn raw(&self) -> &[f64] {
        &self.raw
    }

    pub fn shaped(&self) -> &[f64] {
        &self.shaped
    }

    pub fn display(&self) -> &[f64] {
        &self.display
    }

    pub fn extremes(&self) -> &ExtremumTracker {
        &self.extremes
    }

    /// Returns the display and extremes to their starting state.
    pub fn reset(&mut self) {
        self.extremes.reset();
        self.display.fill(FLOOR_DB);
    }

    /// Shapes the newest raw spectrum, runs attack and release over `dt` seconds, and
    /// updates the extremum traces from the shaped curve.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        dt: f64,
        interp: CurveInterpolation,
        filter: FilteringAmount,
        attack_ms: f64,
        release_ms: f64,
        hold_decay_db_per_second: f64,
        average_seconds: f64,
    ) {
        self.extremes.hold_decay_db_per_second = hold_decay_db_per_second;
        self.extremes.average_seconds = average_seconds;
        if self.raw.is_empty() {
            return;
        }
        smooth(&self.raw, &mut self.shaped, interp, filter);
        let a = 1.0 - (-dt / (attack_ms / 1000.0).max(0.001)).exp();
        let r = 1.0 - (-dt / (release_ms / 1000.0).max(0.001)).exp();
        for (d, &v) in self.display.iter_mut().zip(&self.shaped) {
            let c = *d;
            *d = c + (v - c) * if v > c { a } else { r };
        }
        self.extremes.update(&self.shaped, dt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_flat_is_untouched_and_smoothing_keeps_the_mean() {
        let src: Vec<f64> = (0..50).map(|i| if i == 25 { 0.0 } else { -60.0 }).collect();
        let mut dst = vec![0.0; 50];
        smooth(
            &src,
            &mut dst,
            CurveInterpolation::PeakFlat,
            FilteringAmount::Strong,
        );
        assert_eq!(src, dst);
        smooth(
            &src,
            &mut dst,
            CurveInterpolation::LinearSmooth,
            FilteringAmount::Medium,
        );
        assert!(dst[25] < 0.0 && dst[25] > -60.0);
        let before: f64 = src.iter().sum();
        let after: f64 = dst.iter().sum();
        assert!((before - after).abs() < 1e-9);
    }

    #[test]
    fn attack_is_faster_than_release() {
        let mut c = ChannelCurves::new(4);
        c.raw_mut().fill(-20.0);
        c.update(
            1.0 / 60.0,
            CurveInterpolation::PeakFlat,
            FilteringAmount::None,
            20.0,
            320.0,
            14.0,
            1.2,
        );
        let risen = c.display()[0] - FLOOR_DB;
        c.raw_mut().fill(-140.0);
        let top = c.display()[0];
        c.update(
            1.0 / 60.0,
            CurveInterpolation::PeakFlat,
            FilteringAmount::None,
            20.0,
            320.0,
            14.0,
            1.2,
        );
        let fallen = top - c.display()[0];
        assert!(risen / 120.0 > fallen / (top - FLOOR_DB));
    }
}
