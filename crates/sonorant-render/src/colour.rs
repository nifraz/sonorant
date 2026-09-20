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

    /// The same colour with its parts decoded to linear light, alpha left alone.
    ///
    /// For the passes that work in light rather than in encoded values: the phosphor
    /// accumulator, which adds and fades real light and encodes once at the end. Putting
    /// an encoded colour in there would be encoded twice over.
    pub fn to_linear(self) -> [f32; 4] {
        let decode = |v: f32| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        [decode(self.r), decode(self.g), decode(self.b), self.a]
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

/// The lightness ink needs to carry over the dark ground the visuals are drawn on.
const INK_LUMINANCE: f64 = 0.55;

/// Lifts a colour until it reads as ink over the visuals.
///
/// The desktop's accent is picked to look right as a window highlight, against the
/// desktop's own background. Windows' default is `#3F3F3F`, which drawn as a hover line
/// or a lit button would be darker than the grid beneath it, so "on" would read as off.
/// This mixes the colour towards white until it is light enough to carry, which keeps
/// its hue while guaranteeing it is seen. A colour already light enough is left alone.
pub fn as_ink(c: Argb) -> Argb {
    let (r, g, b) = (
        f64::from(c.r()) / 255.0,
        f64::from(c.g()) / 255.0,
        f64::from(c.b()) / 255.0,
    );
    let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    if lum >= INK_LUMINANCE {
        return c;
    }
    // Mixing with white by `t` moves the luminance the same way, because luminance is
    // linear in the parts: lum' = lum + t * (1 - lum). Solving for the lightness wanted
    // gives the mix straight out, with no searching.
    let t = (INK_LUMINANCE - lum) / (1.0 - lum);
    let mix = |v: f64| ((v + t * (1.0 - v)) * 255.0).round().clamp(0.0, 255.0) as u32;
    Argb((u32::from(c.a()) << 24) | (mix(r) << 16) | (mix(g) << 8) | mix(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(c: Argb) -> f64 {
        0.2126 * f64::from(c.r()) / 255.0
            + 0.7152 * f64::from(c.g()) / 255.0
            + 0.0722 * f64::from(c.b()) / 255.0
    }

    #[test]
    fn a_dark_accent_is_lifted_until_it_reads() {
        // Windows' default accent is a grey darker than the grid it would be drawn over.
        let lifted = as_ink(Argb(0xFF3F_3F3F));
        assert!(luminance(lifted) > luminance(Argb(0xFF3F_3F3F)));
        assert!((luminance(lifted) - INK_LUMINANCE).abs() < 0.01, "{lifted}");
        // Still grey, and still opaque.
        assert_eq!(lifted.r(), lifted.g());
        assert_eq!(lifted.g(), lifted.b());
        assert_eq!(lifted.a(), 255);
    }

    #[test]
    fn a_colour_keeps_its_hue_when_it_is_lifted() {
        // A vivid blue accent stays blue: still the bluest part, still the least red.
        let lifted = as_ink(Argb(0xFF00_78D4));
        assert!(lifted.b() > lifted.g() && lifted.g() > lifted.r());
        assert!((luminance(lifted) - INK_LUMINANCE).abs() < 0.01, "{lifted}");
    }

    #[test]
    fn a_colour_light_enough_already_is_left_alone() {
        for c in [Argb(0xFFFF_D678), Argb(0xFFFF_FFFF), Argb(0x80EE_EEEE)] {
            assert_eq!(as_ink(c), c, "{c}");
        }
    }

    #[test]
    fn even_black_comes_back_readable() {
        let lifted = as_ink(Argb(0xFF00_0000));
        assert!((luminance(lifted) - INK_LUMINANCE).abs() < 0.01, "{lifted}");
    }

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
