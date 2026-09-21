//! How long sound takes to reach the screen, measured rather than reasoned about.
//!
//! The pipeline's own delay is the piece the app is answerable for: from the moment
//! capture hands over a buffer to the moment the frame carrying it is drawn. Capture
//! stamps the wall clock and the frame count each time it pushes, so the age of the
//! newest analysed audio is the age of that stamp plus however many frames arrived
//! after it and haven't been analysed yet.
//!
//! Two things sit outside it at either end, both of them honest to leave out and
//! dishonest to forget. Before: the sound was already a graph cycle old when the
//! system handed it over, about 5 ms at a 256-frame quantum. After: the frame drawn now
//! reaches the photon at the next refresh, and later still where frames are queued
//! ahead. And the visual delay, when it is set, is inside this figure on purpose: the
//! whole point of it is to make the picture later.

use std::time::Duration;

/// Readings kept for the live figure: a few seconds at any frame rate.
const RECENT: usize = 512;

/// What the pipeline's delay has been.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LatencyStats {
    pub readings: usize,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

impl std::fmt::Display for LatencyStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "mean {:.1} ms, p50 {:.1} ms, p99 {:.1} ms, max {:.1} ms over {} frames",
            self.mean_ms, self.p50_ms, self.p99_ms, self.max_ms, self.readings
        )
    }
}

/// Collects the delay a frame at a time.
#[derive(Clone, Debug, Default)]
pub struct Latency {
    recent: Vec<f32>,
    head: usize,
    /// Every reading, for the figure at the end of the run.
    all: Vec<f32>,
}

impl Latency {
    /// Records one frame's reading. Anything impossible is dropped rather than
    /// averaged in: a clock that has not found its footing yet reports a negative age,
    /// and one reading of -40 ms would move a mean further than a hundred good ones.
    pub fn record(&mut self, delay: Duration) {
        let ms = delay.as_secs_f64() * 1000.0;
        if !ms.is_finite() || !(0.0..10_000.0).contains(&ms) {
            return;
        }
        let ms = ms as f32;
        if self.recent.len() < RECENT {
            self.recent.push(ms);
        } else {
            self.recent[self.head] = ms;
            self.head = (self.head + 1) % RECENT;
        }
        // An hour at 240 Hz is 860k readings; stop keeping them long before that.
        if self.all.len() < 4_000_000 {
            self.all.push(ms);
        }
    }

    /// The last few hundred frames, for the status line.
    pub fn recent(&self) -> LatencyStats {
        stats(&self.recent)
    }

    /// Every frame of the run, for the line logged on the way out.
    pub fn overall(&self) -> LatencyStats {
        stats(&self.all)
    }
}

fn stats(readings: &[f32]) -> LatencyStats {
    if readings.is_empty() {
        return LatencyStats::default();
    }
    let mut sorted: Vec<f64> = readings.iter().map(|&v| f64::from(v)).collect();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    let at = |p: f64| sorted[((p * (n - 1) as f64).round() as usize).min(n - 1)];
    LatencyStats {
        readings: n,
        mean_ms: sorted.iter().sum::<f64>() / n as f64,
        p50_ms: at(0.50),
        p99_ms: at(0.99),
        max_ms: sorted[n - 1],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_steady_pipeline_reads_as_one_figure() {
        let mut l = Latency::default();
        for _ in 0..1000 {
            l.record(Duration::from_micros(12_000));
        }
        let s = l.overall();
        assert_eq!(s.readings, 1000);
        assert!((s.mean_ms - 12.0).abs() < 0.01);
        assert!((s.p99_ms - 12.0).abs() < 0.01);
        assert!((s.max_ms - 12.0).abs() < 0.01);
    }

    #[test]
    fn the_worst_frames_show_in_the_tail_and_not_the_middle() {
        let mut l = Latency::default();
        for _ in 0..99 {
            l.record(Duration::from_millis(10));
        }
        l.record(Duration::from_millis(80));
        let s = l.overall();
        assert!((s.p50_ms - 10.0).abs() < 0.01);
        assert!((s.max_ms - 80.0).abs() < 0.01);
        assert!(s.mean_ms > 10.0 && s.mean_ms < 11.0);
    }

    #[test]
    fn readings_that_cannot_be_true_are_left_out() {
        let mut l = Latency::default();
        // Before the audio clock has found its footing the age comes out negative,
        // and a `Duration` cannot hold that, so it arrives as a zero-length one.
        l.record(Duration::ZERO);
        l.record(Duration::from_secs(30));
        l.record(Duration::from_millis(15));
        let s = l.overall();
        assert_eq!(s.readings, 2);
        assert!((s.max_ms - 15.0).abs() < 0.01);
    }

    #[test]
    fn the_live_figure_forgets_what_the_whole_run_keeps() {
        let mut l = Latency::default();
        for _ in 0..RECENT {
            l.record(Duration::from_millis(50));
        }
        for _ in 0..RECENT {
            l.record(Duration::from_millis(10));
        }
        assert!((l.recent().mean_ms - 10.0).abs() < 0.01);
        assert!((l.overall().mean_ms - 30.0).abs() < 0.01);
    }
}
