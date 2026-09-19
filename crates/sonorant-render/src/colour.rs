//! Colours for the 2D layers: the curves, the grid, the labels and the chrome.
//!
//! These are sRGB-encoded with straight alpha and are blended as they are, not in
//! linear light: Nostalgia+ drew them with GDI+, which blends encoded values, and its
//! faint gridlines and chips were tuned for that. Blended in linear light, a white line
//! at 34/255 would come out several times brighter.

use sonorant_core::palette::Rgb;
use sonorant_core::settings::Argb;

/// An sRGB-encoded colour with straight alpha, each part 0 to 1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const TRANSPARENT: Rgba = Rgba::argb(0, 0, 0, 0);

    /// From 8-bit parts, alpha first, as `Color.FromArgb` took them.
    pub const fn argb(a: u8, r: u8, g: u8, b: u8) -> Rgba {
        Rgba {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: a as f32 / 255.0,
        }
    }

    /// An opaque palette colour with the given alpha.
    pub fn rgb(c: Rgb, alpha: u8) -> Rgba {
        Rgba::argb(alpha, c.r, c.g, c.b)
    }

    pub fn from_argb(c: Argb) -> Rgba {
        Rgba::argb(c.a(), c.r(), c.g(), c.b())
    }

    /// The same colour with another alpha, 0 to 255.
    pub fn with_alpha(self, alpha: u8) -> Rgba {
        Rgba {
            a: alpha as f32 / 255.0,
            ..self
        }
    }

    /// Alpha scaled by `k` and rounded to 8 bits, as the furniture fades.
    pub fn faded(self, k: f64) -> Rgba {
        let alpha = (self.a as f64 * 255.0 * k).round().clamp(0.0, 255.0);
        Rgba {
            a: alpha as f32 / 255.0,
            ..self
        }
    }

    /// Each colour part halved, as a darker shade of the same hue.
    pub fn halved(self) -> Rgba {
        let h = |v: f32| ((v * 255.0).round() as u8 / 2) as f32 / 255.0;
        Rgba {
            r: h(self.r),
            g: h(self.g),
            b: h(self.b),
            a: self.a,
        }
    }

    pub fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

/// A theme slot's colour if it's set, otherwise `fallback`.
pub fn pick(slot: Option<Argb>, fallback: Rgba) -> Rgba {
    slot.map_or(fallback, Rgba::from_argb)
}

/// A theme slot's hue with `fallback`'s alpha. Most of the furniture is translucent by
/// design and a colour picker offers opaque colours, so an override supplies the hue
/// and the element keeps its transparency.
pub fn pick_keep_alpha(slot: Option<Argb>, fallback: Rgba) -> Rgba {
    slot.map_or(fallback, |c| Rgba {
        a: fallback.a,
        ..Rgba::from_argb(c)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_keep_or_take_alpha() {
        let fallback = Rgba::argb(34, 255, 255, 255);
        let red = Some(Argb(0xFFFF_0000));
        assert_eq!(pick(None, fallback), fallback);
        assert_eq!(pick(red, fallback), Rgba::argb(255, 255, 0, 0));
        assert_eq!(pick_keep_alpha(red, fallback), Rgba::argb(34, 255, 0, 0));
        assert_eq!(
            Rgba::argb(200, 10, 20, 30).faded(0.5),
            Rgba::argb(100, 10, 20, 30)
        );
        assert_eq!(
            Rgba::argb(255, 201, 100, 3).halved(),
            Rgba::argb(255, 100, 50, 1)
        );
    }
}
