//! When the furniture fades and the pointer goes away.
//!
//! Immersive mode is for watching rather than reading, so with `auto_hide` on, the
//! scales, the deck, the quick bar and the status line fade out once nothing has moved
//! for a while, and the pointer goes with them. Anything at all — a moved pointer, a
//! key, an open menu — brings them straight back, at once rather than by fading in, so
//! the thing you reached for is already there when you get to it.

use std::time::{Duration, Instant};

use sonorant_core::settings::{Flag, Settings};

/// How long everything stays up after the last thing that happened.
const HOLD: Duration = Duration::from_millis(2500);
/// How long the fade itself takes.
const FADE: Duration = Duration::from_millis(600);

/// The furniture's opacity, and whether the pointer is shown.
#[derive(Clone, Copy, Debug)]
pub struct Chrome {
    /// When something last happened that the chrome should answer.
    stirred: Instant,
    alpha: f64,
}

impl Chrome {
    pub fn new(now: Instant) -> Chrome {
        Chrome {
            stirred: now,
            alpha: 1.0,
        }
    }

    /// Something happened: a pointer moved, a key was pressed, a menu opened.
    pub fn stir(&mut self, now: Instant) {
        self.stirred = now;
        self.alpha = 1.0;
    }

    /// Works out the opacity at `now`. `holding` keeps the chrome up however long it has
    /// been, for a menu or a dialog that is open under the pointer.
    pub fn update(&mut self, now: Instant, s: &Settings, holding: bool) -> f64 {
        let fades = s.flag(Flag::Immersive) && s.flag(Flag::AutoHide) && s.flag(Flag::ShowOsd);
        if !fades || holding {
            self.alpha = 1.0;
            if holding {
                self.stirred = now;
            }
            return self.alpha;
        }
        let idle = now.saturating_duration_since(self.stirred);
        self.alpha = if idle <= HOLD {
            1.0
        } else {
            let into = (idle - HOLD).as_secs_f64() / FADE.as_secs_f64();
            (1.0 - into).clamp(0.0, 1.0)
        };
        self.alpha
    }

    /// Whether the pointer should be drawn. It goes once the furniture has gone, not
    /// with it, so it doesn't vanish from under a hand that is still using it.
    pub fn cursor_visible(&self) -> bool {
        self.alpha > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn immersive() -> Settings {
        Settings {
            immersive: true,
            auto_hide: true,
            ..Settings::default()
        }
    }

    #[test]
    fn nothing_fades_outside_immersive_mode() {
        let t = Instant::now();
        let mut c = Chrome::new(t);
        let s = Settings::default();
        assert_eq!(c.update(t + Duration::from_secs(600), &s, false), 1.0);
        assert!(c.cursor_visible());
    }

    #[test]
    fn auto_hide_can_be_switched_off_on_its_own() {
        let t = Instant::now();
        let mut c = Chrome::new(t);
        let s = Settings {
            auto_hide: false,
            ..immersive()
        };
        assert_eq!(c.update(t + Duration::from_secs(600), &s, false), 1.0);
    }

    #[test]
    fn the_furniture_holds_then_fades_then_goes() {
        let t = Instant::now();
        let mut c = Chrome::new(t);
        let s = immersive();
        assert_eq!(c.update(t + Duration::from_millis(100), &s, false), 1.0);
        assert_eq!(c.update(t + HOLD, &s, false), 1.0);
        let half = c.update(t + HOLD + FADE / 2, &s, false);
        assert!((0.3..0.7).contains(&half), "{half}");
        assert_eq!(c.update(t + HOLD + FADE, &s, false), 0.0);
        assert!(!c.cursor_visible());
        // It stays gone rather than coming back round.
        assert_eq!(c.update(t + Duration::from_secs(60), &s, false), 0.0);
    }

    #[test]
    fn anything_at_all_brings_it_straight_back() {
        let t = Instant::now();
        let mut c = Chrome::new(t);
        let s = immersive();
        let gone = t + HOLD + FADE;
        assert_eq!(c.update(gone, &s, false), 0.0);
        c.stir(gone);
        // Back at once, not faded in: what you reached for is there when you arrive.
        assert_eq!(c.update(gone + Duration::from_millis(10), &s, false), 1.0);
        assert!(c.cursor_visible());
    }

    #[test]
    fn an_open_menu_holds_everything_up() {
        let t = Instant::now();
        let mut c = Chrome::new(t);
        let s = immersive();
        let late = t + Duration::from_secs(60);
        assert_eq!(c.update(late, &s, true), 1.0);
        // And the clock starts again from when it closed, rather than from long ago.
        assert_eq!(c.update(late + HOLD, &s, false), 1.0);
        assert_eq!(c.update(late + HOLD + FADE, &s, false), 0.0);
    }
}
