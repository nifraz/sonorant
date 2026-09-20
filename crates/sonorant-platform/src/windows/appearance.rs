//! What the desktop looks like: the accent colour and whether it is dark or light.
//!
//! Nostalgia+ took its colours from MusicBee's skin. With no host to ask, Sonorant asks
//! the desktop instead. `UISettings` is the one place Windows will say both, and it is
//! the same answer whether the accent came from a wallpaper or was chosen by hand.

use sonorant_core::settings::Argb;
use windows::UI::ViewManagement::{UIColorType, UISettings};

/// The desktop's accent colour and whether its windows are dark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Appearance {
    pub accent: Option<Argb>,
    pub dark: bool,
}

impl Default for Appearance {
    fn default() -> Appearance {
        // Dark is the assumption: an analyser is looked at in the dark, and a wrong
        // guess towards dark costs less than a white window in a dim room.
        Appearance {
            accent: None,
            dark: true,
        }
    }
}

/// Reads the desktop's appearance, or the default where Windows won't say.
pub fn appearance() -> Appearance {
    let Ok(settings) = UISettings::new() else {
        log::debug!("no UISettings; using the default appearance");
        return Appearance::default();
    };
    let accent = settings
        .GetColorValue(UIColorType::Accent)
        .ok()
        .map(|c| Argb(u32::from_be_bytes([c.A, c.R, c.G, c.B])));
    // Windows has no "is it dark" flag here. The colour it would paint a window's
    // background is the answer, and its luminance says which way round the theme is.
    let dark = settings
        .GetColorValue(UIColorType::Background)
        .ok()
        .map(|c| luminance(c.R, c.G, c.B) < 0.5)
        .unwrap_or(true);
    Appearance { accent, dark }
}

/// Rough relative luminance, enough to tell a dark theme from a light one.
fn luminance(r: u8, g: u8, b: u8) -> f64 {
    (0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b)) / 255.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luminance_tells_dark_from_light() {
        assert!(luminance(0, 0, 0) < 0.5);
        assert!(luminance(255, 255, 255) > 0.5);
        assert!(luminance(32, 32, 32) < 0.5);
    }

    #[test]
    fn reading_the_desktop_never_fails() {
        // On a runner with no desktop this takes the default rather than panicking.
        let a = appearance();
        if let Some(accent) = a.accent {
            assert_eq!(accent.a(), 255, "an accent colour is opaque");
        }
    }
}
