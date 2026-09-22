//! Colour ramps for the spectrogram, as 256-entry lookup tables.
//!
//! Magma, Inferno, Viridis, Plasma and Cividis are perceptually uniform: equal steps in
//! level read as equal steps in brightness, so two points in the picture can be compared
//! by eye. Cividis is the one that survives colour-vision deficiency: it is built from
//! blue and yellow alone, which no common deficiency confuses. The Cool Edit ramp
//! survives as Nostalgia Red; it saturates early by design, which is charming and
//! imprecise, and Ember, Ocean and Phosphor are of the same family: chosen to look like
//! something rather than to measure.
//!
//! **New ramps go on the end.** Nostalgia+ wrote the palette as a number as well as a
//! name, and the importer still reads both, so inserting one in the middle would change
//! what an old settings file means.

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
    Plasma,
    Cividis,
    Twilight,
    Ocean,
    Phosphor,
    Ember,
}

impl PaletteKind {
    pub const ALL: [PaletteKind; 13] = [
        PaletteKind::Magma,
        PaletteKind::Inferno,
        PaletteKind::Viridis,
        PaletteKind::Turbo,
        PaletteKind::NostalgiaRed,
        PaletteKind::Ice,
        PaletteKind::Grey,
        PaletteKind::Plasma,
        PaletteKind::Cividis,
        PaletteKind::Twilight,
        PaletteKind::Ocean,
        PaletteKind::Phosphor,
        PaletteKind::Ember,
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
            PaletteKind::Plasma => "Plasma",
            PaletteKind::Cividis => "Cividis",
            PaletteKind::Twilight => "Twilight",
            PaletteKind::Ocean => "Ocean",
            PaletteKind::Phosphor => "Phosphor",
            PaletteKind::Ember => "Ember",
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
            PaletteKind::Phosphor => "Phosphor Green",
            other => other.name(),
        }
    }

    /// One line on what the ramp is for, as the menu and help explain it.
    pub fn about(self) -> &'static str {
        match self {
            PaletteKind::Magma => "Perceptually even, black through purple to cream",
            PaletteKind::Inferno => "Perceptually even, and hotter than Magma at the top",
            PaletteKind::Viridis => "Perceptually even, blue through green to yellow",
            PaletteKind::Turbo => "The rainbow, made even. Loud, and easy to read levels off",
            PaletteKind::NostalgiaRed => "Cool Edit's ramp: saturates early, by design",
            PaletteKind::Ice => "Black through blue to white",
            PaletteKind::Grey => "Black to white, and nothing else",
            PaletteKind::Plasma => "Perceptually even, blue through pink to yellow",
            PaletteKind::Cividis => {
                "Perceptually even, and readable with any colour-vision deficiency"
            }
            PaletteKind::Twilight => "Dark at both ends, so the middle of the range stands out",
            PaletteKind::Ocean => "Black through teal to a pale sky",
            PaletteKind::Phosphor => "The green of an old analyser screen",
            PaletteKind::Ember => "A fire seen in the dark: deep red through orange to white",
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
            PaletteKind::Plasma => PLASMA,
            PaletteKind::Cividis => CIVIDIS,
            PaletteKind::Twilight => TWILIGHT,
            PaletteKind::Ocean => OCEAN,
            PaletteKind::Phosphor => PHOSPHOR,
            PaletteKind::Ember => EMBER,
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

const PLASMA: &[Stop] = &[
    s(0.00, 13, 8, 135),
    s(0.06, 43, 4, 142),
    s(0.13, 65, 4, 146),
    s(0.19, 86, 1, 145),
    s(0.25, 106, 0, 138),
    s(0.31, 125, 3, 128),
    s(0.38, 143, 13, 116),
    s(0.44, 159, 29, 104),
    s(0.50, 174, 44, 93),
    s(0.56, 188, 60, 83),
    s(0.63, 201, 76, 73),
    s(0.69, 213, 93, 63),
    s(0.75, 224, 111, 53),
    s(0.81, 234, 131, 43),
    s(0.88, 242, 152, 33),
    s(0.94, 248, 176, 24),
    s(0.97, 251, 201, 27),
    s(1.00, 240, 249, 33),
];

const CIVIDIS: &[Stop] = &[
    s(0.00, 0, 34, 78),
    s(0.06, 0, 42, 92),
    s(0.13, 0, 51, 104),
    s(0.19, 22, 60, 106),
    s(0.25, 44, 69, 105),
    s(0.31, 60, 78, 105),
    s(0.38, 74, 87, 106),
    s(0.44, 86, 96, 108),
    s(0.50, 98, 105, 111),
    s(0.56, 110, 114, 114),
    s(0.63, 122, 123, 117),
    s(0.69, 135, 133, 116),
    s(0.75, 148, 143, 113),
    s(0.81, 162, 154, 108),
    s(0.88, 177, 165, 100),
    s(0.94, 193, 177, 90),
    s(0.97, 210, 190, 77),
    s(1.00, 253, 231, 55),
];

/// Dark at both ends and bright through the middle, so a band sitting in the middle of
/// the range is the thing the eye lands on. The only ramp here that is not monotonic in
/// brightness, which is the point of it and also why it is no good for reading a level.
const TWILIGHT: &[Stop] = &[
    s(0.00, 18, 12, 32),
    s(0.10, 36, 32, 76),
    s(0.20, 52, 62, 118),
    s(0.30, 72, 98, 150),
    s(0.40, 114, 138, 172),
    s(0.50, 176, 176, 184),
    s(0.60, 198, 152, 150),
    s(0.70, 196, 114, 110),
    s(0.80, 172, 74, 82),
    s(0.90, 118, 42, 66),
    s(1.00, 42, 16, 38),
];

const OCEAN: &[Stop] = &[
    s(0.00, 0, 6, 12),
    s(0.18, 0, 34, 52),
    s(0.36, 0, 68, 88),
    s(0.54, 0, 110, 118),
    s(0.70, 16, 156, 148),
    s(0.83, 88, 200, 186),
    s(0.93, 168, 228, 222),
    s(1.00, 236, 252, 252),
];

const PHOSPHOR: &[Stop] = &[
    s(0.00, 0, 4, 0),
    s(0.20, 0, 32, 8),
    s(0.40, 0, 72, 16),
    s(0.60, 16, 128, 28),
    s(0.76, 64, 184, 48),
    s(0.88, 138, 226, 96),
    s(0.96, 204, 246, 168),
    s(1.00, 244, 255, 232),
];

const EMBER: &[Stop] = &[
    s(0.00, 4, 0, 0),
    s(0.16, 40, 4, 4),
    s(0.32, 88, 10, 6),
    s(0.48, 140, 26, 8),
    s(0.62, 190, 52, 8),
    s(0.74, 226, 92, 12),
    s(0.84, 245, 140, 28),
    s(0.92, 252, 190, 74),
    s(0.97, 254, 226, 150),
    s(1.00, 255, 248, 230),
];

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
