//! Colour ramps for the spectrogram, as 256-entry lookup tables.
//!
//! Magma, Inferno and Viridis are perceptually uniform: equal steps in level read as
//! equal steps in brightness, so two points in the picture can be compared by eye. The
//! Cool Edit ramp survives as Nostalgia Red; it saturates early by design, which is
//! charming and imprecise.

use std::fmt;

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum PaletteKind {
    #[default]
    Magma,
    Inferno,
    Viridis,
    Turbo,
    NostalgiaRed,
    Ice,
    Grey,
}

impl PaletteKind {
    pub const ALL: [PaletteKind; 7] = [
        PaletteKind::Magma,
        PaletteKind::Inferno,
        PaletteKind::Viridis,
        PaletteKind::Turbo,
        PaletteKind::NostalgiaRed,
        PaletteKind::Ice,
        PaletteKind::Grey,
    ];

    /// The name used in settings files.
    pub fn name(self) -> &'static str {
        match self {
            PaletteKind::Magma => "Magma",
            PaletteKind::Inferno => "Inferno",
            PaletteKind::Viridis => "Viridis",
            PaletteKind::Turbo => "Turbo",
            PaletteKind::NostalgiaRed => "NostalgiaRed",
            PaletteKind::Ice => "Ice",
            PaletteKind::Grey => "Grey",
        }
    }

    pub fn from_name(name: &str) -> Option<PaletteKind> {
        Self::ALL
            .into_iter()
            .find(|p| p.name().eq_ignore_ascii_case(name))
    }

    /// The name shown in menus.
    pub fn display_name(self) -> &'static str {
        match self {
            PaletteKind::NostalgiaRed => "Nostalgia Red",
            other => other.name(),
        }
    }

    fn stops(self) -> &'static [Stop] {
        match self {
            PaletteKind::Magma => MAGMA,
            PaletteKind::Inferno => INFERNO,
            PaletteKind::Viridis => VIRIDIS,
            PaletteKind::Turbo => TURBO,
            PaletteKind::NostalgiaRed => NOSTALGIA_RED,
            PaletteKind::Ice => ICE,
            PaletteKind::Grey => GREY,
        }
    }
}

impl fmt::Display for PaletteKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

/// An opaque sRGB colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }

    /// Packed as `0xFFRRGGBB`, as Nostalgia+ stored it.
    pub fn to_argb(self) -> u32 {
        0xFF00_0000 | (self.r as u32) << 16 | (self.g as u32) << 8 | self.b as u32
    }

    pub fn to_rgba8(self) -> [u8; 4] {
        [self.r, self.g, self.b, 255]
    }
}

/// A 256-entry colour ramp: entry 0 is silence, entry 255 the ceiling.
pub type Lut = [Rgb; 256];

struct Stop {
    p: f64,
    r: u8,
    g: u8,
    b: u8,
}

const fn s(p: f64, r: u8, g: u8, b: u8) -> Stop {
    Stop { p, r, g, b }
}

const VIRIDIS: &[Stop] = &[
    s(0.00, 68, 1, 84),
    s(0.06, 72, 21, 103),
    s(0.13, 72, 40, 120),
    s(0.19, 69, 55, 129),
    s(0.25, 64, 70, 136),
    s(0.31, 57, 85, 140),
    s(0.38, 51, 99, 141),
    s(0.44, 45, 112, 142),
    s(0.50, 40, 125, 142),
    s(0.56, 35, 138, 141),
    s(0.63, 31, 150, 139),
    s(0.69, 32, 163, 134),
    s(0.75, 41, 175, 127),
    s(0.81, 60, 187, 117),
    s(0.88, 109, 205, 89),
    s(0.94, 170, 220, 50),
    s(1.00, 253, 231, 37),
];

const MAGMA: &[Stop] = &[
    s(0.00, 0, 0, 4),
    s(0.06, 8, 6, 30),
    s(0.13, 20, 14, 54),
    s(0.19, 35, 18, 81),
    s(0.25, 55, 17, 108),
    s(0.31, 76, 17, 122),
    s(0.38, 97, 25, 127),
    s(0.44, 118, 33, 129),
    s(0.50, 139, 41, 129),
    s(0.56, 161, 48, 126),
    s(0.63, 183, 56, 120),
    s(0.69, 205, 65, 111),
    s(0.75, 225, 79, 98),
    s(0.81, 241, 101, 86),
    s(0.88, 251, 132, 86),
    s(0.94, 254, 176, 120),
    s(0.97, 253, 205, 148),
    s(1.00, 252, 253, 191),
];

const INFERNO: &[Stop] = &[
    s(0.00, 0, 0, 4),
    s(0.06, 10, 7, 35),
    s(0.13, 24, 11, 63),
    s(0.19, 42, 11, 91),
    s(0.25, 62, 9, 113),
    s(0.31, 81, 15, 124),
    s(0.38, 100, 24, 128),
    s(0.44, 120, 32, 128),
    s(0.50, 140, 41, 124),
    s(0.56, 160, 50, 117),
    s(0.63, 180, 60, 107),
    s(0.69, 199, 72, 94),
    s(0.75, 216, 87, 78),
    s(0.81, 231, 105, 59),
    s(0.88, 243, 128, 37),
    s(0.94, 249, 193, 31),
    s(0.97, 251, 225, 86),
    s(1.00, 252, 255, 164),
];

const TURBO: &[Stop] = &[
    s(0.00, 48, 18, 59),
    s(0.05, 60, 45, 124),
    s(0.10, 66, 72, 174),
    s(0.15, 68, 98, 211),
    s(0.20, 66, 124, 236),
    s(0.25, 58, 149, 250),
    s(0.30, 44, 173, 250),
    s(0.35, 30, 193, 235),
    s(0.40, 26, 209, 210),
    s(0.45, 37, 221, 180),
    s(0.50, 62, 231, 148),
    s(0.55, 96, 238, 116),
    s(0.60, 133, 243, 88),
    s(0.65, 169, 246, 66),
    s(0.70, 202, 243, 53),
    s(0.75, 227, 232, 50),
    s(0.80, 246, 213, 49),
    s(0.85, 253, 186, 45),
    s(0.90, 251, 153, 38),
    s(0.95, 226, 84, 17),
    s(1.00, 122, 4, 3),
];

const NOSTALGIA_RED: &[Stop] = &[
    s(0.00, 0, 0, 0),
    s(0.15, 32, 0, 8),
    s(0.30, 96, 0, 16),
    s(0.45, 160, 8, 8),
    s(0.60, 216, 40, 0),
    s(0.72, 248, 96, 0),
    s(0.82, 255, 160, 16),
    s(0.90, 255, 208, 64),
    s(0.96, 255, 240, 160),
    s(1.00, 255, 255, 255),
];

const ICE: &[Stop] = &[
    s(0.00, 0, 0, 8),
    s(0.20, 8, 24, 72),
    s(0.40, 12, 64, 140),
    s(0.60, 22, 120, 196),
    s(0.78, 70, 182, 226),
    s(0.90, 150, 224, 240),
    s(1.00, 240, 253, 255),
];

const GREY: &[Stop] = &[s(0.00, 0, 0, 0), s(1.00, 255, 255, 255)];

/// Rounds half to even, as .NET's `Math.Round` did, so every entry matches Nostalgia+.
fn round8(v: f64) -> u8 {
    v.round_ties_even().clamp(0.0, 255.0) as u8
}

/// The ramp for a palette.
pub fn build_lut(kind: PaletteKind) -> Lut {
    let stops = kind.stops();
    let mut lut = [Rgb::default(); 256];
    for (i, out) in lut.iter_mut().enumerate() {
        let t = i as f64 / 255.0;
        let mut k = 0;
        while k < stops.len() - 2 && t > stops[k + 1].p {
            k += 1;
        }
        let (a, b) = (&stops[k], &stops[k + 1]);
        let span = b.p - a.p;
        let f = if span <= 0.0 {
            0.0
        } else {
            ((t - a.p) / span).clamp(0.0, 1.0)
        };
        let lerp = |x: u8, y: u8| round8(x as f64 + (y as f64 - x as f64) * f);
        *out = Rgb::new(lerp(a.r, b.r), lerp(a.g, b.g), lerp(a.b, b.b));
    }
    lut
}

/// The ramp with every entry's hue rotated by `degrees`, keeping saturation and
/// lightness. A rotation moves where the colours sit on the wheel without changing how
/// far apart they are, so the picture stays readable while its colour follows the
/// music. Under half a degree the ramp comes back unchanged.
pub fn build_lut_shifted(kind: PaletteKind, degrees: f64) -> Lut {
    let mut lut = build_lut(kind);
    if degrees.abs() < 0.5 {
        return lut;
    }
    for c in &mut lut {
        let (mut h, s, l) = to_hsl(*c);
        h += degrees;
        while h < 0.0 {
            h += 360.0;
        }
        while h >= 360.0 {
            h -= 360.0;
        }
        *c = from_hsl(h, s, l);
    }
    lut
}

/// The colour at `t` (0..1) along a ramp.
pub fn color_at(lut: &Lut, t: f64) -> Rgb {
    let i = ((t * 255.0) as i32).clamp(0, 255);
    lut[i as usize]
}

/// The colour a palette gives silence: the panel's background.
pub fn background(lut: &Lut) -> Rgb {
    color_at(lut, 0.0)
}

fn to_hsl(c: Rgb) -> (f64, f64, f64) {
    let (r, g, b) = (c.r as f64 / 255.0, c.g as f64 / 255.0, c.b as f64 / 255.0);
    let max = r.max(g.max(b));
    let min = r.min(g.min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d < 1e-9 {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    // `%` on f64 keeps the dividend's sign, like C#'s.
    let mut h = if max == r {
        60.0 * (((g - b) / d) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    if h < 0.0 {
        h += 360.0;
    }
    (h, s, l)
}

fn from_hsl(h: f64, s: f64, l: f64) -> Rgb {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    Rgb::new(
        round8((r + m) * 255.0),
        round8((g + m) * 255.0),
        round8((b + m) * 255.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramps_run_from_their_first_stop_to_their_last() {
        for kind in PaletteKind::ALL {
            let lut = build_lut(kind);
            let (first, last) = (&kind.stops()[0], kind.stops().last().unwrap());
            assert_eq!(lut[0], Rgb::new(first.r, first.g, first.b), "{kind}");
            assert_eq!(lut[255], Rgb::new(last.r, last.g, last.b), "{kind}");
        }
    }

    #[test]
    fn a_full_turn_is_nearly_the_identity() {
        let a = build_lut(PaletteKind::Turbo);
        let b = build_lut_shifted(PaletteKind::Turbo, 360.0);
        for (x, y) in a.iter().zip(&b) {
            assert!((x.r as i32 - y.r as i32).abs() <= 1);
        }
    }
}
