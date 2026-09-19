//! Adaptive floor and ceiling for the colour scale.
//!
//! A fixed window renders a modern master near -12 dBFS as a solid wash: almost every
//! value lands at the top of the ramp and the structure disappears. Following
//! percentiles of what is actually playing keeps the ramp spread across the material,
//! and slew-limiting the movement stops the picture pumping on transients.

const MIN_DB: i32 = -140;
const MAX_DB: i32 = 12;
const BINS: usize = (MAX_DB - MIN_DB) as usize;

/// A decaying 1 dB histogram of recent levels and the floor and ceiling it implies.
#[derive(Clone, Debug)]
pub struct DynamicRange {
    // f32, as in Nostalgia+: the percentiles have to fall on the same bins.
    hist: [f32; BINS],
    floor: f64,
    ceiling: f64,
    primed: bool,
    /// Percentile taken as the floor. Raising it blackens more of the noise floor.
    pub low_percentile: f64,
    /// Percentile taken as the ceiling.
    pub high_percentile: f64,
    /// Fastest the floor and ceiling may move, in dB per second.
    pub slew_db_per_second: f64,
    /// Headroom above the measured ceiling.
    pub ceiling_pad_db: f64,
    /// Narrowest span, so quiet passages don't blow the contrast up.
    pub min_span_db: f64,
    /// Widest span. Without it a mostly empty top end drags the low percentile to the
    /// numeric floor and flattens the picture.
    pub max_span_db: f64,
    /// The ceiling never tracks below this, so digital silence reads as silence rather
    /// than showing the noise floor at full brightness.
    pub min_ceiling_db: f64,
}

impl Default for DynamicRange {
    fn default() -> Self {
        DynamicRange {
            hist: [0.0; BINS],
            floor: -100.0,
            ceiling: -10.0,
            primed: false,
            low_percentile: 0.25,
            high_percentile: 0.999,
            slew_db_per_second: 9.0,
            ceiling_pad_db: 3.0,
            min_span_db: 35.0,
            max_span_db: 90.0,
            min_ceiling_db: -55.0,
        }
    }
}

impl DynamicRange {
    pub fn new() -> DynamicRange {
        DynamicRange::default()
    }

    pub fn floor(&self) -> f64 {
        self.floor
    }

    pub fn ceiling(&self) -> f64 {
        self.ceiling
    }

    /// Forgets the history; the next update jumps straight to its targets.
    pub fn reset(&mut self) {
        self.hist = [0.0; BINS];
        self.primed = false;
    }

    /// Decays the histogram by `decay`, then counts every level in `db`.
    pub fn observe(&mut self, db: &[f64], decay: f64) {
        let d = decay as f32;
        for h in &mut self.hist {
            *h *= d;
        }
        for &v in db {
            let b = ((v - MIN_DB as f64) as i32).clamp(0, BINS as i32 - 1);
            self.hist[b as usize] += 1.0;
        }
    }

    /// Moves the floor and ceiling towards the histogram's percentiles, by at most the
    /// slew limit over `dt_seconds`.
    pub fn update(&mut self, dt_seconds: f64) {
        let total: f64 = self.hist.iter().map(|&h| h as f64).sum();
        if total <= 0.0 {
            return;
        }
        let mut low = self.percentile(total, self.low_percentile);
        let mut high = self.percentile(total, self.high_percentile) + self.ceiling_pad_db;

        if high < self.min_ceiling_db {
            high = self.min_ceiling_db;
        }
        if high - low > self.max_span_db {
            low = high - self.max_span_db;
        }
        if high - low < self.min_span_db {
            let mid = (high + low) * 0.5;
            low = mid - self.min_span_db * 0.5;
            high = mid + self.min_span_db * 0.5;
        }

        if !self.primed {
            self.floor = low;
            self.ceiling = high;
            self.primed = true;
            return;
        }
        let step = self.slew_db_per_second * dt_seconds;
        self.floor += limit(low - self.floor, step);
        self.ceiling += limit(high - self.ceiling, step);
    }

    fn percentile(&self, total: f64, p: f64) -> f64 {
        let want = total * p;
        let mut acc = 0.0;
        for (i, &h) in self.hist.iter().enumerate() {
            acc += h as f64;
            if acc >= want {
                return (MIN_DB + i as i32) as f64;
            }
        }
        MAX_DB as f64
    }
}

/// `v` limited to ±`step`. Unlike `f64::clamp` it tolerates a negative step.
fn limit(v: f64, step: f64) -> f64 {
    if v < -step {
        -step
    } else if v > step {
        step
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_the_material() {
        let mut dr = DynamicRange::new();
        let db: Vec<f64> = (0..1000)
            .map(|i| -80.0 + (i as f64 / 1000.0) * 60.0)
            .collect();
        for _ in 0..200 {
            dr.observe(&db, 0.94);
            dr.update(1.0 / 60.0);
        }
        assert!(
            dr.floor() > -85.0 && dr.floor() < -50.0,
            "floor {}",
            dr.floor()
        );
        assert!(
            dr.ceiling() > -30.0 && dr.ceiling() < 5.0,
            "ceiling {}",
            dr.ceiling()
        );
        assert!(dr.ceiling() - dr.floor() > 20.0);
    }

    #[test]
    fn silence_pins_the_ceiling() {
        let mut dr = DynamicRange::new();
        let db = vec![-140.0; 500];
        dr.observe(&db, 0.94);
        dr.update(1.0 / 60.0);
        assert!(dr.ceiling() >= -55.0 - 1e-9);
    }
}
