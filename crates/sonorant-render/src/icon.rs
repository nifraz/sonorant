//! The app's icon, drawn from the same palette as the picture it stands for.
//!
//! A desktop asks for the icon in half a dozen raster sizes and in a scalable form,
//! Windows asks for all of them packed into one `.ico`, and the window itself asks for
//! raw pixels. Keeping six hand-drawn files in step is how icons drift, so the shapes
//! are described once here, in a unit square, and every form comes out of that one
//! description: the raster at whatever size is asked for, the SVG from the same
//! numbers, and the `.ico` from the rasters. The files under `packaging/icons` are what
//! the `icon` example writes, and a test redraws them to check the two still agree.
//!
//! The bars are coloured out of [`PaletteKind::Magma`], and named rather than taken
//! from the settings: an icon is a fixed thing a desktop caches, so it cannot follow a
//! palette the person changes, and the one it is drawn from has to be chosen here. The
//! app ships showing Nostalgia Red these days; Magma stays, because these are the
//! colours the icon has always been and the ones the AppStream branding quotes.

use sonorant_core::palette::{self, Lut, PaletteKind};

/// The corner radius of the tile, as a share of its side. GNOME and Windows both round
/// an app tile by roughly this much, so the icon sits among the others.
const TILE_RADIUS: f64 = 0.2237;

/// The heights of the bars, as a share of the drawing area: a spectrum with its peak
/// left of centre and falling away above it, which is the shape most music has.
const BARS: [f64; 5] = [0.42, 0.78, 1.00, 0.58, 0.30];

/// The left and right edges of the block of bars.
const BARS_X: (f64, f64) = (0.17, 0.83);
/// The gap between two bars, as a share of a bar's width.
const BAR_GAP: f64 = 0.45;
/// Where the bars stand, and how high the tallest reaches.
const BASELINE: f64 = 0.80;
const BARS_TOP: f64 = 0.18;

/// Where in the ramp the foot of a bar is: far enough up Magma that the shortest bar
/// is still a visible magenta rather than the near-black the ramp starts at.
const RAMP_FLOOR: f64 = 0.34;

/// The tile itself: the Magma floor, lifted enough to read as a surface rather than as
/// a hole, and shaded from top to bottom so the tile has some depth to it.
const TILE_TOP: [f64; 3] = [0.106, 0.071, 0.212];
const TILE_BOTTOM: [f64; 3] = [0.027, 0.020, 0.059];

/// How many samples a pixel is drawn with, along each axis.
const SUPERSAMPLE: u32 = 4;

/// The sizes packed into the `.ico`, and written as PNGs beside it.
pub const SIZES: [u32; 6] = [16, 32, 48, 64, 128, 256];

/// The icon at `size` by `size` pixels, 8-bit sRGB with straight alpha, rows top down.
///
/// This is what a window is given directly, so it costs no file and no decoder: at 64
/// pixels it is well under a millisecond of arithmetic.
pub fn rgba(size: u32) -> Vec<u8> {
    let lut = palette::build_lut(PaletteKind::Magma);
    let shapes = Shapes::new(size);
    let n = size as usize;
    let mut out = vec![0u8; n * n * 4];
    let step = 1.0 / f64::from(size * SUPERSAMPLE);
    let samples = (SUPERSAMPLE * SUPERSAMPLE) as f64;
    for py in 0..size {
        for px in 0..size {
            let (mut sum, mut covered) = ([0.0f64; 3], 0.0f64);
            for sy in 0..SUPERSAMPLE {
                for sx in 0..SUPERSAMPLE {
                    let x = (f64::from(px * SUPERSAMPLE + sx) + 0.5) * step;
                    let y = (f64::from(py * SUPERSAMPLE + sy) + 0.5) * step;
                    let Some(colour) = shapes.sample(x, y, &lut) else {
                        continue;
                    };
                    covered += 1.0;
                    for c in 0..3 {
                        sum[c] += colour[c];
                    }
                }
            }
            let i = (py as usize * n + px as usize) * 4;
            if covered > 0.0 {
                for c in 0..3 {
                    out[i + c] = encode(sum[c] / covered);
                }
            }
            // Straight alpha: the colour above is the covered part's own colour, not a
            // colour already faded towards the transparent corner.
            out[i + 3] = (covered / samples * 255.0).round() as u8;
        }
    }
    out
}

/// The same icon as an SVG, for the scalable slot an icon theme and Flathub both want.
///
/// Drawn on a 512-unit square: whole numbers for the tile, two decimals for the bars.
pub fn svg() -> String {
    const SIDE: f64 = 512.0;
    let lut = palette::build_lut(PaletteKind::Magma);
    let u = |v: f64| format!("{:.2}", v * SIDE);
    let hex = |c: [f64; 3]| {
        format!(
            "#{:02x}{:02x}{:02x}",
            encode(c[0]),
            encode(c[1]),
            encode(c[2])
        )
    };

    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{SIDE:.0}\" height=\"{SIDE:.0}\" \
         viewBox=\"0 0 {SIDE:.0} {SIDE:.0}\">\n"
    ));
    s.push_str("  <title>Sonorant</title>\n  <defs>\n");
    s.push_str("    <linearGradient id=\"tile\" x1=\"0\" y1=\"0\" x2=\"0\" y2=\"1\">\n");
    s.push_str(&format!(
        "      <stop offset=\"0\" stop-color=\"{}\"/>\n      <stop offset=\"1\" \
         stop-color=\"{}\"/>\n",
        hex(TILE_TOP),
        hex(TILE_BOTTOM)
    ));
    s.push_str("    </linearGradient>\n");
    // In user space, so every bar reads the same ramp: a short bar keeps the colours of
    // the part of the tall one it reaches, which is what the raster does.
    s.push_str(&format!(
        "    <linearGradient id=\"bars\" gradientUnits=\"userSpaceOnUse\" x1=\"0\" y1=\"{}\" \
         x2=\"0\" y2=\"{}\">\n",
        u(BASELINE),
        u(BARS_TOP)
    ));
    for i in 0..=8 {
        let t = f64::from(i) / 8.0;
        s.push_str(&format!(
            "      <stop offset=\"{t:.3}\" stop-color=\"{}\"/>\n",
            {
                let c = ramp(t, &lut);
                format!(
                    "#{:02x}{:02x}{:02x}",
                    encode(c[0]),
                    encode(c[1]),
                    encode(c[2])
                )
            }
        ));
    }
    s.push_str("    </linearGradient>\n  </defs>\n");
    s.push_str(&format!(
        "  <rect x=\"0\" y=\"0\" width=\"{SIDE:.0}\" height=\"{SIDE:.0}\" rx=\"{}\" \
         fill=\"url(#tile)\"/>\n",
        u(TILE_RADIUS)
    ));
    // The scalable form has no pixel grid to snap to, so it takes the unsnapped bars.
    for bar in Shapes::new(0).bars {
        let r = bar.top;
        s.push_str(&format!(
            "  <path fill=\"url(#bars)\" d=\"M{} {}V{}A{r0} {r0} 0 0 1 {} {}H{}A{r0} {r0} 0 0 1 \
             {} {}V{}Z\"/>\n",
            u(bar.x0),
            u(bar.y1),
            u(bar.y0 + r),
            u(bar.x0 + r),
            u(bar.y0),
            u(bar.x1 - r),
            u(bar.x1),
            u(bar.y0 + r),
            u(bar.y1),
            r0 = u(r),
        ));
    }
    s.push_str("</svg>\n");
    s
}

/// Every size in [`SIZES`] packed into one Windows `.ico`.
///
/// The entries are PNGs rather than DIBs, which Windows has read since Vista and which
/// keeps the alpha straightforward: no upside-down rows and no separate AND mask.
pub fn ico() -> Result<Vec<u8>, String> {
    let images: Vec<Vec<u8>> = SIZES
        .iter()
        .map(|&size| crate::readback::encode_png(size, size, &rgba(size)))
        .collect::<Result<_, _>>()?;

    let count = u16::try_from(images.len()).map_err(|_| "too many icon sizes".to_string())?;
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // an icon, not a cursor
    out.extend_from_slice(&count.to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (&size, png) in SIZES.iter().zip(&images) {
        // 256 is written as 0: the field is one byte and 256 doesn't fit in it.
        let side = u8::try_from(size % 256).unwrap_or(0);
        out.extend_from_slice(&[side, side, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits a pixel
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for png in &images {
        out.extend_from_slice(png);
    }
    Ok(out)
}

/// A rectangle with its top and bottom corners rounded by their own radius.
#[derive(Clone, Copy, Debug)]
struct RoundRect {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    top: f64,
    bottom: f64,
}

impl RoundRect {
    fn contains(&self, x: f64, y: f64) -> bool {
        if x < self.x0 || x >= self.x1 || y < self.y0 || y >= self.y1 {
            return false;
        }
        let inside = |cx: f64, cy: f64, r: f64| (x - cx).powi(2) + (y - cy).powi(2) <= r * r;
        if y < self.y0 + self.top {
            if x < self.x0 + self.top {
                return inside(self.x0 + self.top, self.y0 + self.top, self.top);
            }
            if x > self.x1 - self.top {
                return inside(self.x1 - self.top, self.y0 + self.top, self.top);
            }
        }
        if y > self.y1 - self.bottom {
            if x < self.x0 + self.bottom {
                return inside(self.x0 + self.bottom, self.y1 - self.bottom, self.bottom);
            }
            if x > self.x1 - self.bottom {
                return inside(self.x1 - self.bottom, self.y1 - self.bottom, self.bottom);
            }
        }
        true
    }
}

/// The icon's geometry in the unit square, with the bars snapped to whatever pixel grid
/// they are about to be drawn on.
struct Shapes {
    tile: RoundRect,
    bars: Vec<RoundRect>,
}

impl Shapes {
    /// `size` is the pixels a side the bars are snapped to; 0 leaves them unsnapped,
    /// which is what the SVG wants.
    fn new(size: u32) -> Shapes {
        // Snapping matters at 16 pixels, where a bar is a pixel and a half wide: left
        // alone, all five come out as two grey columns of half-covered pixels and the
        // icon reads as a smear. Snapped, they are whole columns of solid colour.
        let snap = |v: f64| {
            if size == 0 {
                v
            } else {
                (v * f64::from(size)).round() / f64::from(size)
            }
        };
        let span = BARS_X.1 - BARS_X.0;
        let n = BARS.len() as f64;
        let width = span / (n + (n - 1.0) * BAR_GAP);
        let pitch = width * (1.0 + BAR_GAP);
        let height = BASELINE - BARS_TOP;
        let bars = BARS
            .iter()
            .enumerate()
            .map(|(i, &h)| {
                let x0 = snap(BARS_X.0 + i as f64 * pitch);
                let x1 = snap(BARS_X.0 + i as f64 * pitch + width);
                let y0 = BASELINE - h * height;
                RoundRect {
                    x0,
                    y0,
                    x1,
                    y1: snap(BASELINE),
                    // A round top and a flat foot: the bars stand on a line, the way
                    // they do in the picture.
                    top: (x1 - x0) / 2.0,
                    bottom: 0.0,
                }
            })
            .collect();
        Shapes {
            tile: RoundRect {
                x0: 0.0,
                y0: 0.0,
                x1: 1.0,
                y1: 1.0,
                top: TILE_RADIUS,
                bottom: TILE_RADIUS,
            },
            bars,
        }
    }

    /// The colour at a point, or `None` where the icon is transparent.
    fn sample(&self, x: f64, y: f64, lut: &Lut) -> Option<[f64; 3]> {
        if !self.tile.contains(x, y) {
            return None;
        }
        if self.bars.iter().any(|b| b.contains(x, y)) {
            let t = (BASELINE - y) / (BASELINE - BARS_TOP);
            return Some(ramp(t.clamp(0.0, 1.0), lut));
        }
        let mut tile = [0.0; 3];
        for (c, out) in tile.iter_mut().enumerate() {
            *out = TILE_TOP[c] + (TILE_BOTTOM[c] - TILE_TOP[c]) * y;
        }
        Some(tile)
    }
}

/// The bar colour `t` of the way from the baseline to the top of the tallest bar.
fn ramp(t: f64, lut: &Lut) -> [f64; 3] {
    let i = ((RAMP_FLOOR + (1.0 - RAMP_FLOOR) * t) * 255.0)
        .round()
        .clamp(0.0, 255.0) as usize;
    let c = lut[i];
    [
        f64::from(c.r) / 255.0,
        f64::from(c.g) / 255.0,
        f64::from(c.b) / 255.0,
    ]
}

fn encode(v: f64) -> u8 {
    (v * 255.0).round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(rgba: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    }

    #[test]
    fn the_corners_are_clear_and_the_middle_is_not() {
        for size in SIZES {
            let px = rgba(size);
            assert_eq!(px.len(), (size * size * 4) as usize);
            assert_eq!(pixel(&px, size, 0, 0)[3], 0, "{size}: top left");
            assert_eq!(
                pixel(&px, size, size - 1, size - 1)[3],
                0,
                "{size}: bottom right"
            );
            assert_eq!(
                pixel(&px, size, size / 2, size / 2)[3],
                255,
                "{size}: middle"
            );
        }
    }

    #[test]
    fn the_bars_run_dark_at_the_foot_to_light_at_the_tip() {
        let size = 256;
        let px = rgba(size);
        // The middle of the tallest bar, which stands at BARS_X.0 + 2 pitches.
        let shapes = Shapes::new(size);
        let tall = shapes.bars[2];
        let x = (((tall.x0 + tall.x1) / 2.0) * f64::from(size)) as u32;
        let luminance = |y: f64| {
            let p = pixel(&px, size, x, (y * f64::from(size)) as u32);
            0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2])
        };
        let foot = luminance(BASELINE - 0.02);
        let tip = luminance(BARS_TOP + 0.02);
        assert!(tip > foot * 2.0, "foot {foot:.1}, tip {tip:.1}");
        // And the tip is the pale end of Magma rather than anything invented here.
        let top = pixel(&px, size, x, ((BARS_TOP + 0.02) * f64::from(size)) as u32);
        assert!(top[0] > 200 && top[1] > 120, "{top:?}");
    }

    #[test]
    fn a_bar_is_brighter_than_the_tile_behind_it() {
        let size = 128;
        let px = rgba(size);
        let shapes = Shapes::new(size);
        let tall = shapes.bars[2];
        let bar_x = (((tall.x0 + tall.x1) / 2.0) * f64::from(size)) as u32;
        let y = ((BASELINE - 0.05) * f64::from(size)) as u32;
        let bar = pixel(&px, size, bar_x, y);
        // Halfway between two bars, where the tile shows through.
        let gap_x = (((tall.x1 + shapes.bars[3].x0) / 2.0) * f64::from(size)) as u32;
        let gap = pixel(&px, size, gap_x, y);
        assert!(bar[0] > gap[0] + 60, "bar {bar:?}, tile {gap:?}");
        assert_eq!(bar[3], 255);
        assert_eq!(gap[3], 255);
    }

    #[test]
    fn small_sizes_keep_their_bars_crisp() {
        // Every bar edge lands on a pixel boundary, so a 16-pixel icon has no column
        // that is half bar and half tile.
        for size in [16, 32, 48] {
            for bar in Shapes::new(size).bars {
                for edge in [bar.x0, bar.x1] {
                    let pixels = edge * f64::from(size);
                    assert!(
                        (pixels - pixels.round()).abs() < 1e-9,
                        "{size}: edge at {pixels}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_svg_draws_the_tile_and_every_bar() {
        let svg = svg();
        assert!(svg.starts_with("<?xml"));
        assert!(svg.contains("viewBox=\"0 0 512 512\""));
        assert_eq!(svg.matches("<path").count(), BARS.len());
        assert!(svg.contains("url(#tile)") && svg.contains("url(#bars)"));
        assert!(svg.ends_with("</svg>\n"));
    }

    #[test]
    fn the_ico_holds_every_size_as_a_png() {
        let ico = ico().expect("ico");
        assert_eq!(&ico[0..4], &[0, 0, 1, 0]);
        let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
        assert_eq!(count, SIZES.len());
        for (i, &size) in SIZES.iter().enumerate() {
            let entry = 6 + i * 16;
            let expected = u8::try_from(size % 256).unwrap_or(0);
            assert_eq!(ico[entry], expected, "{size}: width byte");
            let len = u32::from_le_bytes(ico[entry + 8..entry + 12].try_into().unwrap()) as usize;
            let at = u32::from_le_bytes(ico[entry + 12..entry + 16].try_into().unwrap()) as usize;
            assert_eq!(&ico[at..at + 8], b"\x89PNG\r\n\x1a\n", "{size}: not a PNG");
            assert!(at + len <= ico.len());
        }
    }
}
