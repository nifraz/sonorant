//! What reached the screen, from the swapchain's own counters.
//!
//! Timing frames when the swapchain hands back a buffer shows when the CPU got one, and
//! with frames queued ahead that wanders by a whole refresh either way while every
//! frame still reaches the screen on time. DXGI counts presents and refreshes itself:
//! when its refresh counter moves on further than its present counter, some refresh
//! showed the previous frame again. That is a dropped frame as the viewer sees it.
//!
//! Only Direct3D 12 on Windows has these counters; elsewhere the monitor stays empty and
//! the pacing figures come from the acquire timings alone.

/// Presents and refreshes counted since the last reset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PresentCounts {
    /// Frames that reached the screen.
    pub presents: u64,
    /// Refreshes those frames spanned.
    pub refreshes: u64,
    /// Refreshes that showed a frame again because the next one wasn't ready.
    pub repeated: u64,
}

impl std::fmt::Display for PresentCounts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} presents over {} refreshes, {} repeated",
            self.presents, self.refreshes, self.repeated
        )
    }
}

#[derive(Debug, Default)]
pub struct PresentMonitor {
    /// The last sample: present count and the refresh it was shown on.
    last: Option<(u32, u32)>,
    counts: PresentCounts,
    /// False once the counters turned out to be unavailable.
    unsupported: bool,
}

impl PresentMonitor {
    /// The counts so far, or `None` where the platform has none. A composited window
    /// on some systems (a laptop whose discrete GPU draws for the integrated one) gets
    /// counters that never move; that's none too.
    pub fn counts(&self) -> Option<PresentCounts> {
        (!self.unsupported && self.counts.presents > 0).then_some(self.counts)
    }

    /// Starts counting afresh, after a pause or a change of mode.
    pub fn reset(&mut self) {
        self.last = None;
        self.counts = PresentCounts::default();
    }

    /// Reads the counters. Call after each present.
    pub fn sample(&mut self, surface: &wgpu::Surface<'_>) {
        if self.unsupported {
            return;
        }
        match read(surface) {
            Some(Some(now)) => self.observe(now),
            // The sequence broke (a mode change, say); pick it up from here.
            Some(None) => self.last = None,
            None => self.unsupported = true,
        }
    }

    fn observe(&mut self, (presents, refresh): (u32, u32)) {
        if let Some((p0, r0)) = self.last {
            let dp = presents.wrapping_sub(p0) as u64;
            let dr = refresh.wrapping_sub(r0) as u64;
            // Nothing new reached the screen yet; wait for it.
            if dp == 0 {
                return;
            }
            // Samples are taken every frame, so a jump of more than a few seconds is a
            // pause or a reset, not a stutter.
            if dp < 1000 && dr < 1000 {
                self.counts.presents += dp;
                self.counts.refreshes += dr;
                self.counts.repeated += dr.saturating_sub(dp);
            }
        }
        self.last = Some((presents, refresh));
    }
}

/// `None` when the platform has no counters, `Some(None)` when they're momentarily
/// unavailable, otherwise the present count and the refresh the last present was shown on.
#[cfg(windows)]
fn read(surface: &wgpu::Surface<'_>) -> Option<Option<(u32, u32)>> {
    use windows::Win32::Graphics::Dxgi::DXGI_FRAME_STATISTICS;

    // SAFETY: the swapchain is only read, and the guard holds the surface for the call.
    let hal = unsafe { surface.as_hal::<wgpu::hal::api::Dx12>() }?;
    let Some(swap_chain) = hal.swap_chain() else {
        return Some(None);
    };
    let mut stats = DXGI_FRAME_STATISTICS::default();
    // SAFETY: a plain query on a live swapchain into a struct we own.
    match unsafe { swap_chain.GetFrameStatistics(&mut stats) } {
        Ok(()) => Some(Some((stats.PresentCount, stats.PresentRefreshCount))),
        Err(_) => Some(None),
    }
}

#[cfg(not(windows))]
fn read(_surface: &wgpu::Surface<'_>) -> Option<Option<(u32, u32)>> {
    None
}

/// The display's refresh rate as the compositor runs it, exactly: 59.94 rather than the
/// 59 that monitor enumeration rounds it down to, which matters when counting refreshes
/// over a minute. `None` where there's no compositor to ask.
#[cfg(windows)]
pub fn compositor_refresh_hz() -> Option<f64> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{DWM_TIMING_INFO, DwmGetCompositionTimingInfo};

    let mut info = DWM_TIMING_INFO {
        cbSize: size_of::<DWM_TIMING_INFO>() as u32,
        ..Default::default()
    };
    // SAFETY: a query into a struct we own, sized as the call requires.
    unsafe { DwmGetCompositionTimingInfo(HWND::default(), &mut info) }.ok()?;
    let r = info.rateRefresh;
    (r.uiDenominator > 0 && r.uiNumerator > 0)
        .then(|| r.uiNumerator as f64 / r.uiDenominator as f64)
}

#[cfg(not(windows))]
pub fn compositor_refresh_hz() -> Option<f64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_refreshes_are_counted() {
        let mut m = PresentMonitor::default();
        m.observe((100, 500));
        m.observe((101, 501)); // on time
        m.observe((101, 501)); // nothing new yet
        m.observe((103, 504)); // two frames over three refreshes: one repeat
        m.observe((104, 505));
        let c = m.counts().unwrap();
        assert_eq!((c.presents, c.refreshes, c.repeated), (4, 5, 1));
        m.observe((u32::MAX, 0)); // wrapped and far apart: a pause, not counted
        assert_eq!(m.counts().unwrap().presents, 4);
    }
}
