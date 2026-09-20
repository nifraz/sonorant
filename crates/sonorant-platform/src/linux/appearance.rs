//! What the desktop looks like: the accent colour and whether it is dark or light.
//!
//! Nostalgia+ took its colours from MusicBee's skin. With no host to ask, Sonorant asks
//! the desktop, through the XDG settings portal, which works the same inside a Flatpak
//! and out of one. `color-scheme` has been there since the portal's first version;
//! `accent-color` arrived in version 2, so a desktop that has not got it simply has no
//! accent and the palette keeps its own colours.

use sonorant_core::settings::Argb;
use zbus::blocking::Connection;

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

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const NAMESPACE: &str = "org.freedesktop.appearance";

/// Reads the desktop's appearance, or the default where the portal won't say.
pub fn appearance() -> Appearance {
    match read() {
        Ok(a) => a,
        Err(e) => {
            log::debug!("no appearance from the portal: {e}; using the default");
            Appearance::default()
        }
    }
}

fn read() -> zbus::Result<Appearance> {
    let connection = Connection::session()?;
    let dark = match one::<u32>(&connection, "color-scheme") {
        // 1 is "prefer dark", 2 "prefer light", 0 "no preference".
        Ok(scheme) => scheme != 2,
        Err(e) => {
            log::debug!("no colour scheme: {e}");
            true
        }
    };
    // The accent arrives as three doubles from 0 to 1, in sRGB.
    let accent = match one::<(f64, f64, f64)>(&connection, "accent-color") {
        Ok((r, g, b)) => Some(Argb(u32::from_be_bytes([
            255,
            channel(r),
            channel(g),
            channel(b),
        ]))),
        Err(e) => {
            log::debug!("no accent colour: {e}");
            None
        }
    };
    Ok(Appearance { accent, dark })
}

/// Reads one setting out of the appearance namespace.
///
/// `ReadOne` hands the value straight back, where the older `Read` wrapped it in a
/// second variant. Ubuntu 24.04's portal has `ReadOne`, and a desktop without it simply
/// has no appearance to report: the caller takes the default.
fn one<T>(connection: &Connection, key: &str) -> zbus::Result<T>
where
    T: zbus::zvariant::Type + zbus::export::serde::de::DeserializeOwned,
{
    let proxy = zbus::blocking::Proxy::new(connection, PORTAL, PATH, SETTINGS)?;
    proxy.call("ReadOne", &(NAMESPACE, key))
}

/// One sRGB channel, from the portal's 0-to-1 double.
fn channel(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_land_on_whole_bytes() {
        assert_eq!(channel(0.0), 0);
        assert_eq!(channel(1.0), 255);
        assert_eq!(channel(0.5), 128);
        // A portal that says something silly is held to the range rather than wrapping.
        assert_eq!(channel(-1.0), 0);
        assert_eq!(channel(2.0), 255);
        assert_eq!(channel(f64::NAN), 0);
    }

    #[test]
    fn the_default_is_dark_with_no_accent() {
        let a = Appearance::default();
        assert!(a.dark);
        assert_eq!(a.accent, None);
    }
}
