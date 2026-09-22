//! The hover readout, the harmonic ruler and the axis pin.
//!
//! Where the pointer sits over a pane, the row under it is a frequency and the column is
//! a moment. This reads both back out: the frequency in hertz and as a note with its
//! deviation in cents, the level each channel is showing at that frequency, and how far
//! back in time the column is. Nostalgia+ drew the same three things, and the settings
//! that govern them carry over: `sync_hover` reads out both panes at once,
//! `show_hover_pin` stamps the frequency onto the axis itself, and `show_harmonics`
//! draws ghost lines at whole multiples of the frequency under the pointer.

use sonorant_core::dsp::{FrequencyMap, describe_note};
use sonorant_core::settings::Settings;

use crate::axes::short_hz;
use crate::colour::{Rgba, pick_keep_alpha};
use crate::layout::{PaneLayout, Rect, ScopeLayout};
use crate::overlay::{Face, Layer, Overlay};

/// How many multiples of the hovered frequency the ruler marks.
const HARMONICS: u32 = 8;

/// What the pointer is over.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hover {
    /// Which pane the pointer is in.
    pub pane: usize,
    /// Where it is, in physical pixels.
    pub x: i32,
    pub y: i32,
    /// The frequency of the row under it.
    pub freq: f64,
    /// Which row of the display that is, so a caller can read a level out of its own
    /// curve without mapping the frequency again.
    pub bin: usize,
    /// How far back the column under it is, in seconds. Zero over the curve strip,
    /// where there is no history to point at.
    pub age: f64,
    /// Whether the pointer is over the image rather than the curve strip.
    pub on_image: bool,
}

/// What the pointer is over, or `None` when it is outside every pane.
///
/// `at` is in physical pixels, as the panes are laid out.
/// `behind` is how many seconds the pane's newest edge is behind now, which is zero
/// while the image is live and grows as the wheel or a drag parks it back in the
/// history. It is added to the age, so the readout and a double-click to seek both
/// name the moment that is actually drawn there rather than one measured from an edge
/// that has moved.
pub fn locate(
    layout: &ScopeLayout,
    map: &FrequencyMap,
    at: (i32, i32),
    px_per_row: f64,
    rows_per_second: f64,
    behind: f64,
) -> Option<Hover> {
    let (x, y) = at;
    let (pane_index, pane) = layout
        .panes
        .iter()
        .enumerate()
        .find(|(_, p)| p.bounds.contains(x, y))?;
    // A scale strip is chrome, not the axis: a frequency read from one would be wrong.
    // Both ends are checked, because the strip can be reserved at both.
    if pane.lanes().any(|lane| lane.contains(x, y)) {
        return None;
    }
    let body = pane.spectro;
    if y < body.y || y >= body.bottom() {
        return None;
    }
    let bin = (body.bottom() - 1 - y).max(0) as usize;
    let freq = map.x_to_freq(bin as f64);
    let on_image = pane.spectro.contains(x, y);
    let age = if on_image {
        pane.age_at(x, px_per_row).unwrap_or(0.0) / rows_per_second.max(1e-9) + behind
    } else {
        0.0
    };
    Some(Hover {
        pane: pane_index,
        x,
        y,
        freq,
        bin,
        age,
        on_image,
    })
}

/// A level to read out beside a pane's label, in dBFS.
#[derive(Clone, Copy, Debug)]
pub struct Reading<'a> {
    pub label: &'a str,
    pub db: f64,
}

/// Everything one frame's readout is drawn from.
#[derive(Clone, Copy, Debug)]
pub struct Readout<'a> {
    pub layout: &'a ScopeLayout,
    pub settings: &'a Settings,
    pub map: &'a FrequencyMap,
    pub hover: Hover,
    /// One per pane when the hover is synced, otherwise the hovered pane alone.
    pub levels: &'a [Reading<'a>],
    /// The furniture's opacity, 0 to 1.
    pub alpha: f64,
    /// The label size in pixels.
    pub label_px: f32,
}

/// Draws the hover lines, the harmonic ruler, the axis pin and the readout box.
pub fn draw(o: &mut Overlay, r: &Readout<'_>) {
    let s = r.settings;
    if !s.show_osd || r.alpha <= 0.004 {
        return;
    }
    let ink = pick_keep_alpha(s.theme.hover, Rgba::argb(220, 255, 214, 120)).faded(r.alpha);
    let panes: Vec<&PaneLayout> = if s.sync_hover {
        r.layout.panes.iter().collect()
    } else {
        r.layout.panes.get(r.hover.pane).into_iter().collect()
    };

    // The frequency, across every pane it is read out of.
    for pane in &panes {
        o.hline(
            Layer::Top,
            pane.bounds.x as f32,
            pane.bounds.right() as f32,
            r.hover.y as f32 + 0.5,
            ink,
        );
    }
    // The moment, down the pane the pointer is actually in: the same column of another
    // pane is the same moment, but the pointer is only over one of them.
    if r.hover.on_image
        && let Some(pane) = r.layout.panes.get(r.hover.pane)
    {
        o.vline(
            Layer::Top,
            r.hover.x as f32 + 0.5,
            pane.spectro.y as f32,
            pane.spectro.bottom() as f32,
            ink.halved(),
        );
    }

    if s.show_harmonics {
        harmonics(o, r, &panes, ink);
    }
    if s.show_hover_pin {
        pin(o, r, ink);
    }
    // The box is the one part that covers the picture, so it has a switch of its own:
    // the crosshair, the ruler and the pin all read against the image rather than over
    // it, and someone who wants to see what is under the pointer wants them kept.
    if s.show_hud {
        box_(o, r, ink);
    }
}

/// Ghost lines at whole multiples of the hovered frequency, fading as they climb: a
/// harmonic series lines up with them when what you are pointing at is a fundamental.
fn harmonics(o: &mut Overlay, r: &Readout<'_>, panes: &[&PaneLayout], ink: Rgba) {
    let Some(first) = r.layout.panes.first() else {
        return;
    };
    let body = first.spectro;
    for n in 2..=HARMONICS {
        let f = r.hover.freq * f64::from(n);
        if f > r.map.fmax {
            break;
        }
        let y = row_y(r.map, body, f);
        if y < body.y || y >= body.bottom() {
            continue;
        }
        // Each one fainter than the last, so the fundamental stays the brightest line
        // and a tall stack doesn't turn the pane into a grid.
        let fade = 0.55 / f64::from(n - 1);
        for pane in panes {
            o.hline(
                Layer::Top,
                pane.bounds.x as f32,
                pane.bounds.right() as f32,
                y as f32 + 0.5,
                ink.faded(fade),
            );
        }
        // The multiple itself, small, against the gutter, where there is room.
        if !r.layout.gutter.is_empty() && r.layout.gutter.w > 14 {
            let text = format!("{n}x");
            let px = r.label_px * 0.85;
            let sz = o.measure(&text, Face::Sans, px);
            let x = r.layout.gutter.x as f32 + (r.layout.gutter.w as f32 - sz.w) / 2.0;
            o.text(
                Layer::Top,
                &text,
                Face::Sans,
                px,
                x,
                y as f32 - sz.h / 2.0,
                ink.faded(fade),
            );
        }
    }
}

/// The hovered frequency stamped onto the axis columns, so the reading can be compared
/// with the labels around it rather than only with the box by the pointer.
fn pin(o: &mut Overlay, r: &Readout<'_>, ink: Rgba) {
    let text = short_hz(r.hover.freq);
    let px = r.label_px;
    let sz = o.measure(&text, Face::Sans, px);
    let columns = [r.layout.gutter, r.layout.outer_left, r.layout.outer_right];
    for column in columns {
        if column.w < sz.w.ceil() as i32 {
            continue;
        }
        let x = column.x as f32 + (column.w as f32 - sz.w) / 2.0;
        let y = r.hover.y as f32 - sz.h / 2.0;
        o.rect(
            Layer::Top,
            x - 2.0,
            y - 1.0,
            sz.w + 4.0,
            sz.h + 1.0,
            Rgba::argb(210, 10, 10, 12).faded(r.alpha),
        );
        o.text(Layer::Top, &text, Face::Sans, px, x, y, ink);
    }
}

/// The readout itself, beside the pointer: the frequency, the note, each channel's level
/// and how far back the column is.
fn box_(o: &mut Overlay, r: &Readout<'_>, ink: Rgba) {
    let px = r.label_px;
    let mut lines = vec![hz_line(r.hover.freq)];
    if let Some((note, cents)) = describe_note(r.hover.freq) {
        lines.push(format!("{note} {cents:+.0} cents"));
    }
    for level in r.levels {
        lines.push(if level.db.is_finite() {
            format!("{}  {:.1} dB", level.label, level.db)
        } else {
            format!("{}  --", level.label)
        });
    }
    if r.hover.on_image {
        lines.push(if r.hover.age < 0.05 {
            "now".to_owned()
        } else {
            format!("-{:.2} s", r.hover.age)
        });
    }

    let pad = (px * 0.5).max(3.0);
    let line_h = px * 1.35;
    let w = lines
        .iter()
        .map(|l| o.measure(l, Face::Sans, px).w)
        .fold(0.0_f32, f32::max)
        + pad * 2.0;
    let h = line_h * lines.len() as f32 + pad * 2.0;

    // Beside the pointer, and on whichever side keeps it inside the panes.
    let gap = px;
    let bounds = r.layout.bounds;
    let mut x = r.hover.x as f32 + gap;
    if x + w > bounds.right() as f32 {
        x = r.hover.x as f32 - gap - w;
    }
    let x = x.clamp(
        bounds.x as f32,
        (bounds.right() as f32 - w).max(bounds.x as f32),
    );
    let y = (r.hover.y as f32 - h / 2.0).clamp(
        bounds.y as f32,
        (bounds.bottom() as f32 - h).max(bounds.y as f32),
    );

    o.rect(
        Layer::Top,
        x,
        y,
        w,
        h,
        Rgba::argb(216, 10, 10, 12).faded(r.alpha),
    );
    o.outline(Layer::Top, x, y, w, h, ink.faded(0.5));
    for (i, line) in lines.iter().enumerate() {
        let colour = if i == 0 { ink } else { ink.faded(0.85) };
        o.text(
            Layer::Top,
            line,
            Face::Sans,
            px,
            x + pad,
            y + pad + line_h * i as f32,
            colour,
        );
    }
}

/// The frequency written out in full, rather than the axis's short form: a readout is
/// read, not scanned.
fn hz_line(f: f64) -> String {
    if f >= 10000.0 {
        format!("{:.2} kHz", f / 1000.0)
    } else if f >= 1000.0 {
        format!("{:.3} kHz", f / 1000.0)
    } else if f >= 100.0 {
        format!("{f:.1} Hz")
    } else {
        format!("{f:.2} Hz")
    }
}

/// The y a frequency sits at, in a body that runs low at the bottom.
fn row_y(map: &FrequencyMap, body: Rect, f: f64) -> i32 {
    body.bottom() - 1 - map.freq_to_x(f).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use sonorant_core::dsp::FreqScale;
    use sonorant_core::settings::ScaleLanePosition;

    fn layout() -> ScopeLayout {
        ScopeLayout::new(Rect::new(0, 0, 1200, 600), &Settings::default(), 1.0)
    }

    #[test]
    fn the_row_under_the_pointer_is_its_frequency() {
        let l = layout();
        let pane = &l.panes[0];
        let map = FrequencyMap::new(FreqScale::Note, pane.spectro.h as usize, 20.0, 20000.0);
        // The bottom row of the image is the lowest frequency, the top row the highest.
        let bottom = locate(
            &l,
            &map,
            (pane.spectro.x + 5, pane.spectro.bottom() - 1),
            1.0,
            60.0,
            0.0,
        )
        .expect("inside the pane");
        assert_eq!(bottom.bin, 0);
        assert!((bottom.freq - 20.0).abs() < 0.5, "{}", bottom.freq);
        let top = locate(
            &l,
            &map,
            (pane.spectro.x + 5, pane.spectro.y),
            1.0,
            60.0,
            0.0,
        )
        .expect("inside the pane");
        assert_eq!(top.bin, pane.spectro.h as usize - 1);
        assert!(top.freq > 19000.0, "{}", top.freq);
    }

    #[test]
    fn the_column_under_the_pointer_is_a_moment() {
        let l = layout();
        let pane = &l.panes[0];
        let map = FrequencyMap::new(FreqScale::Note, pane.spectro.h as usize, 20.0, 20000.0);
        let y = pane.spectro.y + 10;
        // The newest column is now; 60 columns back at 60 rows a second is a second ago.
        let newest = if pane.curve_on_left {
            pane.spectro.x
        } else {
            pane.spectro.right() - 1
        };
        let now = locate(&l, &map, (newest, y), 1.0, 60.0, 0.0).expect("inside");
        assert!(now.age.abs() < 1e-9);
        let back = if pane.curve_on_left {
            newest + 60
        } else {
            newest - 60
        };
        let then = locate(&l, &map, (back, y), 1.0, 60.0, 0.0).expect("inside");
        assert!((then.age - 1.0).abs() < 1e-9, "{}", then.age);
        // Zooming in spreads the same second over twice the pixels.
        let zoomed = locate(&l, &map, (back, y), 2.0, 60.0, 0.0).expect("inside");
        assert!((zoomed.age - 0.5).abs() < 1e-9, "{}", zoomed.age);

        // Parked ten seconds back, every column is ten seconds older than it reads from
        // the edge, including the edge itself.
        let parked = locate(&l, &map, (newest, y), 1.0, 60.0, 10.0).expect("inside");
        assert!((parked.age - 10.0).abs() < 1e-9, "{}", parked.age);
        let parked_back = locate(&l, &map, (back, y), 1.0, 60.0, 10.0).expect("inside");
        assert!((parked_back.age - 11.0).abs() < 1e-9, "{}", parked_back.age);

        // Off the image there is no moment to name, parked or not.
        let off = locate(&l, &map, (pane.curve.x + 1, y), 1.0, 60.0, 10.0).expect("inside");
        assert_eq!(off.age, 0.0);
    }

    #[test]
    fn the_curve_strip_reads_frequency_but_not_time() {
        let l = layout();
        let pane = &l.panes[0];
        let map = FrequencyMap::new(FreqScale::Note, pane.spectro.h as usize, 20.0, 20000.0);
        let at = (
            pane.curve.x + pane.curve.w / 2,
            pane.curve.y + pane.curve.h / 2,
        );
        let h = locate(&l, &map, at, 1.0, 60.0, 0.0).expect("over the curve strip");
        assert!(!h.on_image);
        assert_eq!(h.age, 0.0);
        assert!(h.freq > 20.0);
    }

    #[test]
    fn outside_the_panes_is_nothing() {
        let l = layout();
        let map = FrequencyMap::new(FreqScale::Note, 500, 20.0, 20000.0);
        assert!(locate(&l, &map, (-5, 10), 1.0, 60.0, 0.0).is_none());
        assert!(locate(&l, &map, (600, 10_000), 1.0, 60.0, 0.0).is_none());
        // The gutter carries labels, not an image.
        let gutter = l.gutter;
        if !gutter.is_empty() {
            assert!(locate(&l, &map, (gutter.x + gutter.w / 2, 300), 1.0, 60.0, 0.0).is_none());
        }
    }

    #[test]
    fn the_scale_lane_is_chrome_rather_than_axis() {
        for &pos in ScaleLanePosition::ALL {
            let s = Settings {
                scale_lane_pos: pos,
                ..Settings::default()
            };
            let l = ScopeLayout::new(Rect::new(0, 0, 1200, 600), &s, 1.0);
            let pane = &l.panes[0];
            let map = FrequencyMap::new(FreqScale::Note, pane.spectro.h as usize, 20.0, 20000.0);
            let lanes: Vec<Rect> = pane.lanes().collect();
            assert_eq!(
                lanes.len(),
                if pos == ScaleLanePosition::Both { 2 } else { 1 },
                "{pos:?} reserves the wrong number of strips"
            );
            for lane in lanes {
                let at = (pane.spectro.x + 5, lane.y + lane.h / 2);
                assert!(locate(&l, &map, at, 1.0, 60.0, 0.0).is_none(), "{pos:?}");
            }
        }
    }

    #[test]
    fn frequencies_are_written_to_a_readable_precision() {
        assert_eq!(hz_line(43.65), "43.65 Hz");
        assert_eq!(hz_line(440.0), "440.0 Hz");
        assert_eq!(hz_line(1234.5), "1.234 kHz");
        assert_eq!(hz_line(15000.0), "15.00 kHz");
    }
}
