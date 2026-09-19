//! Frame pacing: how evenly frames reach the screen.
//!
//! Every frame records the moment it got its swapchain image. With vsync that moment
//! follows the display, but only loosely: with frames queued ahead, the swapchain hands
//! buffers back in bursts, and on the reference PC the intervals alternate around
//! 15.5 and 31 ms while every refresh still gets a new frame. A long interval is not a
//! missed refresh by itself. What is: fewer frames than the display refreshed in the
//! same time, so that's what "missed" counts, from the exact refresh rate.

use std::fmt;
use std::io::{self, Write};
use std::path::Path;
use std::time::{Duration, Instant};

/// Frames ignored at start-up, while shaders compile and the swapchain settles.
const WARM_UP_FRAMES: u64 = 30;
/// Intervals kept for the live readout: a few seconds at any refresh rate.
const RECENT: usize = 512;

#[derive(Clone, Debug)]
pub struct FramePacing {
    last: Option<Instant>,
    frames: u64,
    refresh_hz: Option<f64>,
    recent: Vec<f32>,
    recent_head: usize,
    all_ms: Vec<f32>,
}

/// Statistics over a set of frame intervals.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PacingStats {
    pub frames: usize,
    pub fps: f64,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    /// Refreshes without a new frame: the refreshes in the time the frames spanned,
    /// less the frames.
    pub missed: u64,
    pub refresh_hz: Option<f64>,
}

impl FramePacing {
    pub fn new(refresh_hz: Option<f64>) -> FramePacing {
        FramePacing {
            last: None,
            frames: 0,
            refresh_hz,
            recent: Vec::with_capacity(RECENT),
            recent_head: 0,
            all_ms: Vec::new(),
        }
    }

    pub fn set_refresh_hz(&mut self, hz: Option<f64>) {
        self.refresh_hz = hz;
    }

    /// Records a frame at `now` and returns the interval since the previous one.
    pub fn record(&mut self, now: Instant) -> Option<Duration> {
        let interval = self.last.map(|t| now.saturating_duration_since(t));
        self.last = Some(now);
        self.frames += 1;
        let dt = interval?;
        // The first interval comes with the second frame.
        if self.frames - 1 <= WARM_UP_FRAMES {
            return interval;
        }
        let ms = dt.as_secs_f64() * 1000.0;
        if self.recent.len() < RECENT {
            self.recent.push(ms as f32);
        } else {
            self.recent[self.recent_head] = ms as f32;
            self.recent_head = (self.recent_head + 1) % RECENT;
        }
        // A day at 240 Hz is 20M frames; stop keeping individual intervals long before.
        if self.all_ms.len() < 4_000_000 {
            self.all_ms.push(ms as f32);
        }
        interval
    }

    /// Forgets the timing of the last frame, so a pause isn't measured as a slow frame.
    pub fn break_sequence(&mut self) {
        self.last = None;
    }

    /// Statistics over the last few hundred frames.
    pub fn recent(&self) -> PacingStats {
        stats(&self.recent, self.refresh_hz)
    }

    /// Statistics over every frame since the warm-up.
    pub fn overall(&self) -> PacingStats {
        stats(&self.all_ms, self.refresh_hz)
    }

    /// Writes every recorded interval as CSV: frame number and milliseconds.
    pub fn write_csv(&self, path: &Path) -> io::Result<()> {
        let mut w = io::BufWriter::new(std::fs::File::create(path)?);
        writeln!(w, "frame,interval_ms")?;
        for (i, ms) in self.all_ms.iter().enumerate() {
            writeln!(w, "{},{ms:.4}", i as u64 + WARM_UP_FRAMES + 1)?;
        }
        w.flush()
    }
}

/// Refreshes in `span_ms` that didn't get one of `frames` frames.
fn missed_refreshes(span_ms: f64, frames: usize, refresh_hz: Option<f64>) -> u64 {
    let Some(hz) = refresh_hz.filter(|&h| h > 0.0) else {
        return 0;
    };
    let refreshes = span_ms * hz / 1000.0;
    (refreshes - frames as f64).round().max(0.0) as u64
}

fn stats(intervals: &[f32], refresh_hz: Option<f64>) -> PacingStats {
    if intervals.is_empty() {
        return PacingStats {
            refresh_hz,
            ..PacingStats::default()
        };
    }
    let mut sorted: Vec<f64> = intervals.iter().map(|&v| v as f64).collect();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    let span: f64 = sorted.iter().sum();
    let mean = span / n as f64;
    let at = |p: f64| sorted[((p * (n - 1) as f64).round() as usize).min(n - 1)];
    PacingStats {
        frames: n,
        fps: if mean > 0.0 { 1000.0 / mean } else { 0.0 },
        mean_ms: mean,
        p50_ms: at(0.50),
        p99_ms: at(0.99),
        max_ms: sorted[n - 1],
        missed: missed_refreshes(span, n, refresh_hz),
        refresh_hz,
    }
}

impl fmt::Display for PacingStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:.1} fps over {} frames, interval mean {:.2} ms, p50 {:.2} ms, p99 {:.2} ms, max {:.2} ms",
            self.fps, self.frames, self.mean_ms, self.p50_ms, self.p99_ms, self.max_ms
        )?;
        match self.refresh_hz {
            Some(hz) => write!(f, ", {} missed at {hz:.2} Hz", self.missed),
            None => write!(f, ", refresh rate unknown"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(p: &mut FramePacing, intervals_ms: &[f64]) {
        let mut t = Instant::now();
        p.record(t);
        for _ in 0..WARM_UP_FRAMES {
            t += Duration::from_micros(16_667);
            p.record(t);
        }
        for &ms in intervals_ms {
            t += Duration::from_secs_f64(ms / 1000.0);
            p.record(t);
        }
    }

    #[test]
    fn steady_60hz_misses_nothing() {
        let mut p = FramePacing::new(Some(60.0));
        feed(&mut p, &[16.667; 600]);
        let s = p.overall();
        assert_eq!(s.frames, 600);
        assert_eq!(s.missed, 0);
        assert!((s.fps - 60.0).abs() < 0.1);
        assert!((s.p99_ms - 16.667).abs() < 0.01);
    }

    #[test]
    fn long_intervals_count_the_refreshes_they_skipped() {
        let mut p = FramePacing::new(Some(60.0));
        let mut v = vec![16.667; 100];
        v[10] = 33.3; // one refresh missed
        v[50] = 50.0; // two
        v[70] = 20.0; // late, but not a whole refresh
        feed(&mut p, &v);
        assert_eq!(p.overall().missed, 3);
        assert_eq!(p.recent().missed, 3);
    }

    #[test]
    fn late_frames_made_up_by_early_ones_miss_nothing() {
        // What the swapchain hands back on the reference PC: bursts, but a frame for
        // every refresh.
        let mut p = FramePacing::new(Some(59.94));
        let mut v = Vec::new();
        for _ in 0..40 {
            v.extend([15.5; 14]);
            v.push(16.683 * 15.0 - 15.5 * 14.0);
        }
        feed(&mut p, &v);
        assert_eq!(p.overall().missed, 0);
    }

    #[test]
    fn warm_up_is_ignored() {
        let mut p = FramePacing::new(Some(144.0));
        let mut t = Instant::now();
        for _ in 0..=WARM_UP_FRAMES {
            p.record(t);
            t += Duration::from_millis(100);
        }
        assert_eq!(p.overall().frames, 0);
        assert_eq!(p.overall().missed, 0);
    }
}
