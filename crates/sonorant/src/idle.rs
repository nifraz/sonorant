//! Redrawing slowly while there is nothing to see.
//!
//! A covered window draws nothing at all, which the frame loop handles. This is the
//! other half: the window is visible, but nothing is playing, so every frame is the
//! same black scroll under the same resting meters. Drawing that 165 times a second is
//! the whole cost of the app for none of its value, and on a laptop it is the
//! difference between a machine that idles cool and one that does not.
//!
//! The rate only governs frames the app asks for itself. A key, a click, a pointer
//! moving or a menu opening all request a redraw of their own, so nothing feels slow to
//! use while idling; what goes slowly is the part that has nothing new in it.
//!
//! Silence is held for a few seconds before the rate drops, so a gap between tracks or
//! a rest in the music is not mistaken for a pause. Coming back is immediate: one frame
//! with sound in it and the next is due at once.

use std::time::{Duration, Instant};

/// How often the screen is redrawn while nothing is playing.
pub const IDLE_INTERVAL: Duration = Duration::from_millis(100);
/// How long it has to stay quiet before the rate drops.
const QUIET_FIRST: Duration = Duration::from_secs(3);
/// Momentary loudness at or below this is silence rather than quiet music.
///
/// Digital silence measures about -140 LUFS and the quietest passage anyone would call
/// music is nearer -50, so this sits well clear of both: it answers "is the stream
/// carrying anything at all", not "is this quiet".
const SILENT_LUFS: f64 = -90.0;

/// Whether the picture has anything new in it, and for how long it hasn't.
#[derive(Clone, Copy, Debug, Default)]
pub struct Idle {
    /// When the quiet started, while it lasts.
    since: Option<Instant>,
}

impl Idle {
    /// Takes this frame's evidence and says whether to redraw slowly from here.
    ///
    /// `capturing` is false when the source has stopped, suspended or failed, which is
    /// silence for a reason rather than silence in the music, and is treated the same
    /// way: there is nothing arriving to draw either way.
    pub fn update(&mut self, now: Instant, momentary_lufs: f64, capturing: bool) -> bool {
        // `is_none_or` rather than a negated comparison, so a loudness that is not a
        // number - nothing measured yet - reads as silence, which is what it is.
        let quiet = !capturing
            || momentary_lufs
                .partial_cmp(&SILENT_LUFS)
                .is_none_or(|o| o != std::cmp::Ordering::Greater);
        if !quiet {
            self.since = None;
            return false;
        }
        let since = *self.since.get_or_insert(now);
        now.duration_since(since) >= QUIET_FIRST
    }

    /// Forgets the quiet, so the next frame starts counting again.
    ///
    /// For anything that makes the picture worth watching again whatever the sound is
    /// doing: a new track, a source being pointed somewhere else.
    pub fn stir(&mut self) {
        self.since = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_has_to_last_before_the_rate_drops() {
        let mut idle = Idle::default();
        let t = Instant::now();
        assert!(!idle.update(t, -140.0, true));
        assert!(!idle.update(t + Duration::from_secs(2), -140.0, true));
        assert!(idle.update(t + QUIET_FIRST, -140.0, true));
        assert!(idle.update(t + Duration::from_secs(60), -140.0, true));
    }

    #[test]
    fn one_frame_with_sound_in_it_ends_it() {
        let mut idle = Idle::default();
        let t = Instant::now();
        idle.update(t, -140.0, true);
        assert!(idle.update(t + Duration::from_secs(10), -140.0, true));
        // Music again: back at once, and the count starts over rather than resuming
        // where it was, so the next silence gets the full wait of its own.
        assert!(!idle.update(t + Duration::from_secs(11), -20.0, true));
        assert!(!idle.update(t + Duration::from_secs(13), -140.0, true));
        assert!(!idle.update(t + Duration::from_secs(15), -140.0, true));
        assert!(idle.update(t + Duration::from_secs(16), -140.0, true));
    }

    #[test]
    fn quiet_music_is_not_silence() {
        let mut idle = Idle::default();
        let t = Instant::now();
        // Well below anything comfortable and still not silence.
        assert!(!idle.update(t + Duration::from_secs(60), -60.0, true));
        // Nothing measured yet reads as silence, which is what it is.
        assert!(!idle.update(t, f64::NAN, true));
        assert!(idle.update(t + QUIET_FIRST, f64::NAN, true));
    }

    #[test]
    fn a_source_that_has_stopped_is_silence_whatever_it_last_measured() {
        let mut idle = Idle::default();
        let t = Instant::now();
        assert!(!idle.update(t, -20.0, false));
        assert!(idle.update(t + QUIET_FIRST, -20.0, false));
        // And stirring it puts the count back to the start.
        idle.stir();
        assert!(!idle.update(t + QUIET_FIRST, -20.0, false));
    }
}
