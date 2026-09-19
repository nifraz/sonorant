//! The scales around the panes: the frequency grid and its labels in the gutter and the
//! outer columns, the time marks along each spectrogram, the level scale along each
//! curve strip, the reserved scale lane, and the channel labels.
//!
//! Ported from Nostalgia+'s `StereoScope.DrawGrid` and `ChannelPane`, keeping their
//! rules: labels are centred on their row or not drawn at all, numbers step round the
//! unit captions, and spacing follows the space available rather than a fixed count.

use sonorant_core::dsp::{FreqScale, FrequencyMap, midi_to_freq};
use sonorant_core::settings::{AxisLabelMode, GraphBackground, Settings};

use crate::colour::{Rgba, pick_keep_alpha};
use crate::curves::db_step;
use crate::layout::{PaneLayout, Rect, ScopeLayout};
use crate::overlay::{Face, Layer, Overlay, TextSize};

/// Everything the scales are drawn from, for one frame.
#[derive(Clone, Copy, Debug)]
pub struct Scales<'a> {
    pub layout: &'a ScopeLayout,
    pub settings: &'a Settings,
    /// The display's frequency axis: one column per spectrogram row.
    pub map: &'a FrequencyMap,
    pub floor_db: f64,
    pub ceiling_db: f64,
    /// Spectrogram pixels per second of audio.
    pub px_per_second: f64,
    /// Pixels per point, so the label size follows the display's scaling.
    pub scale: f32,
    /// The furniture's opacity, 0 to 1.
    pub alpha: f64,
    /// Pixels to drop labels drawn over the image by, to clear a status line.
    pub top_inset: f32,
    /// Outer labels start below this y, clear of chrome over the image's top.
    pub label_floor_y: f32,
}

impl Scales<'_> {
    /// The small label size in pixels: the settings' point size at this scaling.
    pub fn label_px(&self) -> f32 {
        label_px(self.settings, self.scale)
    }
}

/// The small label size in pixels for `settings` at `scale` pixels per point.
pub fn label_px(settings: &Settings, scale: f32) -> f32 {
    settings.label_font_size.clamp(5.0, 20.0) * 96.0 / 72.0 * scale
}

/// Draws every scale. Call before the curve strips are drawn; the scales that belong
/// over them go to [`Layer::Over`].
pub fn draw(o: &mut Overlay, sc: &Scales<'_>) {
    if sc.alpha <= 0.004 {
        return;
    }
    let s = sc.settings;
    let level_unit = (s.show_scale_units && s.show_db_scale).then_some("dBFS");
    let time_unit = (s.show_scale_units && s.show_time_marks).then_some("now");
    for pane in &sc.layout.panes {
        let units = scale_lane(o, sc, pane, level_unit, time_unit);
        if s.show_time_marks {
            time_marks(o, sc, pane, &units);
        }
        level_scale(o, sc, pane, &units);
    }
    if s.show_grid {
        grid(o, sc);
    }
    if s.show_labels {
        pane_labels(o, sc);
    }
}

/// The spans of the scale lane the unit captions own, so numbers step round them.
#[derive(Clone, Copy, Debug, Default)]
struct LaneUnits {
    level: Option<(f32, f32)>,
    time: Option<(f32, f32)>,
}

impl LaneUnits {
    fn free(&self, x0: f32, x1: f32) -> bool {
        [self.level, self.time]
            .into_iter()
            .flatten()
            .all(|(l, r)| x1 <= l || x0 >= r)
    }
}

fn lane_at_top(p: &PaneLayout) -> bool {
    p.lane.y <= p.bounds.y
}

/// The rows a tick inside the lane covers, at its edge facing the image.
fn lane_tick(p: &PaneLayout) -> (f32, f32) {
    let len = (p.lane.h / 4).max(3);
    if lane_at_top(p) {
        (
            (p.lane.bottom() - 1 - len) as f32,
            (p.lane.bottom() - 1) as f32,
        )
    } else {
        (p.lane.y as f32, (p.lane.y + len) as f32)
    }
}

/// Where text of height `h` sits in the lane: centred in the part the ticks leave.
fn lane_text_y(p: &PaneLayout, h: f32) -> f32 {
    let len = (p.lane.h / 4).max(3) as f32;
    let free = p.lane.h as f32 - len;
    if lane_at_top(p) {
        p.lane.y as f32 + (free - h) / 2.0
    } else {
        p.lane.y as f32 + len + (free - h) / 2.0
    }
}

/// The reserved strip's ground and hairline, and each scale's unit at the end it's
/// measured from: level from the curve's base, time from the live edge.
fn scale_lane(
    o: &mut Overlay,
    sc: &Scales<'_>,
    p: &PaneLayout,
    level_unit: Option<&str>,
    time_unit: Option<&str>,
) -> LaneUnits {
    let mut units = LaneUnits::default();
    let lane = p.lane;
    if lane.h <= 0 {
        return units;
    }
    let t = &sc.settings.theme;
    let ground = pick_keep_alpha(t.panel, Rgba::argb(255, 13, 13, 16));
    fill(
        o,
        Layer::Under,
        lane,
        ground.with_alpha((255.0 * sc.alpha) as u8),
    );
    let edge = if lane_at_top(p) {
        lane.bottom() - 1
    } else {
        lane.y
    };
    o.hline(
        Layer::Under,
        lane.x as f32,
        (lane.right() - 1) as f32,
        edge as f32,
        Rgba::argb((60.0 * sc.alpha) as u8, 255, 255, 255),
    );

    let ink = pick_keep_alpha(t.units, Rgba::argb(190, 150, 200, 245)).faded(sc.alpha);
    let px = sc.label_px();
    if let Some(unit) = level_unit
        && p.curve.w >= 40
    {
        let sz = o.measure(unit, Face::Sans, px);
        let x = if p.curve_on_left {
            p.curve.x as f32 + 2.0
        } else {
            p.curve.right() as f32 - sz.w - 2.0
        };
        o.text(
            Layer::Under,
            unit,
            Face::Sans,
            px,
            x,
            lane_text_y(p, sz.h),
            ink,
        );
        units.level = Some((x - 3.0, x + sz.w + 3.0));
    }
    if let Some(unit) = time_unit
        && p.spectro.w >= 60
    {
        let sz = o.measure(unit, Face::Sans, px);
        let x = if p.curve_on_left {
            p.spectro.x as f32 + 2.0
        } else {
            p.spectro.right() as f32 - sz.w - 2.0
        };
        o.text(
            Layer::Under,
            unit,
            Face::Sans,
            px,
            x,
            lane_text_y(p, sz.h),
            ink,
        );
        units.time = Some((x - 3.0, x + sz.w + 3.0));
    }
    units
}

/// The time step: a mark roughly every 85 px, on a round number of seconds.
pub fn time_step(px_per_second: f64) -> f64 {
    const CHOICES: [f64; 8] = [1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0];
    CHOICES
        .into_iter()
        .find(|&c| c * px_per_second >= 85.0)
        .unwrap_or(120.0)
}

/// Ticks back in time along the spectrogram. Age runs away from the curve, so mirrored
/// panes count outwards in opposite directions.
fn time_marks(o: &mut Overlay, sc: &Scales<'_>, p: &PaneLayout, units: &LaneUnits) {
    let r = p.spectro;
    if sc.px_per_second <= 0.0 || r.w < 40 {
        return;
    }
    let t = &sc.settings.theme;
    let visible = r.w as f64 / sc.px_per_second;
    let step = time_step(sc.px_per_second);
    let line = pick_keep_alpha(t.grid_major, Rgba::argb(30, 255, 255, 255));
    let text = pick_keep_alpha(t.axis_text, Rgba::argb(215, 235, 235, 242)).faded(sc.alpha);
    let chip = Rgba::argb((170.0 * sc.alpha) as u8, 8, 8, 11);
    let tick = line.with_alpha((110.0 * sc.alpha) as u8);
    let line = line.faded(sc.alpha);
    let px = sc.label_px();

    let mut secs = step;
    while secs < visible {
        let off = (secs * sc.px_per_second) as i32;
        let x = if p.curve_on_left {
            r.x + off
        } else {
            r.right() - 1 - off
        };
        secs += step;
        if x < r.x || x >= r.right() {
            continue;
        }
        let x = x as f32;
        o.vline(Layer::Under, x, r.y as f32, r.bottom() as f32, line);
        let label = format!("-{:.0}s", secs - step);
        let sz = o.measure(&label, Face::Sans, px);
        let lx = clamp_into(x - sz.w / 2.0, sz.w, r);
        if p.lane.h > 0 {
            let (t0, t1) = lane_tick(p);
            o.vline(Layer::Under, x, t0, t1, tick);
            if units.free(lx - 4.0, lx + sz.w + 4.0) {
                o.text(
                    Layer::Under,
                    &label,
                    Face::Sans,
                    px,
                    lx,
                    lane_text_y(p, sz.h),
                    text,
                );
            }
        } else {
            let ly = r.y as f32 + 3.0 + sc.top_inset;
            chip_behind(o, Layer::Under, lx, ly, sz, chip);
            o.text(Layer::Under, &label, Face::Sans, px, lx, ly, text);
        }
    }
}

/// The level scale's numbers along the curve strip, in the lane or on chips at the
/// strip's quiet (high-frequency) end. The lines are the curve pass's.
fn level_scale(o: &mut Overlay, sc: &Scales<'_>, p: &PaneLayout, units: &LaneUnits) {
    let s = sc.settings;
    let r = p.curve;
    if !matches!(s.background, GraphBackground::Lines | GraphBackground::Grid)
        || !s.show_db_scale
        || r.w < if p.lane.h > 0 { 34 } else { 58 }
    {
        return;
    }
    let t = &s.theme;
    let span = (sc.ceiling_db - sc.floor_db).max(1.0);
    let step = db_step(span, r.w);
    let line = pick_keep_alpha(t.grid_major, Rgba::argb(34, 255, 255, 255));
    let text = pick_keep_alpha(t.axis_text, Rgba::argb(215, 235, 235, 242)).faded(sc.alpha);
    let chip = Rgba::argb((170.0 * sc.alpha) as u8, 8, 8, 11);
    let tick = line.with_alpha((110.0 * sc.alpha) as u8);
    let (base, dir) = if p.curve_on_left {
        (r.x as f64, 1.0)
    } else {
        (r.right() as f64, -1.0)
    };
    let px = sc.label_px();

    let mut d = (sc.floor_db / step).ceil() * step;
    while d <= sc.ceiling_db {
        let tt = ((d - sc.floor_db) / span).clamp(0.0, 1.0);
        let x = (base + dir * tt * r.w as f64) as f32;
        let label = format!("{d:.0}");
        d += step;
        let sz = o.measure(&label, Face::Sans, px);
        let lx = clamp_into(x - sz.w / 2.0, sz.w, r);
        if p.lane.h > 0 {
            let (t0, t1) = lane_tick(p);
            o.vline(Layer::Under, x.floor(), t0, t1, tick);
            if units.free(lx - 4.0, lx + sz.w + 4.0) {
                o.text(
                    Layer::Under,
                    &label,
                    Face::Sans,
                    px,
                    lx,
                    lane_text_y(p, sz.h),
                    text,
                );
            }
        } else {
            let ly = r.y as f32 + 3.0 + sc.top_inset;
            chip_behind(o, Layer::Under, lx, ly, sz, chip);
            o.text(Layer::Under, &label, Face::Sans, px, lx, ly, text);
        }
    }
}

/// One gridline of the frequency axis.
#[derive(Clone, Debug, PartialEq)]
pub struct GridLine {
    pub freq: f64,
    pub label: String,
    /// A second line under the label, in the "both" mode.
    pub sub_label: Option<String>,
    /// An octave (or on a linear axis a multiple of five steps): drawn stronger.
    pub major: bool,
}

/// Gridline frequencies and their labels at a density that suits `axis_pixels`: about
/// one label per 30 to 64 px, subdividing the octave musically (octave, tritone, major
/// third, minor third, tone, semitone) on the note and log axes.
pub fn grid_lines(map: &FrequencyMap, mode: AxisLabelMode, axis_pixels: i32) -> Vec<GridLine> {
    let mut target = if axis_pixels <= 0 {
        30.0
    } else {
        (axis_pixels as f64 / 18.0).clamp(30.0, 64.0)
    };
    if mode == AxisLabelMode::Both {
        target *= 1.5; // two lines of text need two lines' room
    }
    let mut out = Vec::new();

    if map.scale == FreqScale::Linear {
        const STEPS: [f64; 5] = [500.0, 1000.0, 2000.0, 5000.0, 10000.0];
        let step = if axis_pixels <= 0 {
            2000.0
        } else {
            let range = (map.fmax - map.fmin).max(1.0);
            STEPS
                .into_iter()
                .find(|&s| axis_pixels as f64 * s / range >= target)
                .unwrap_or(10000.0)
        };
        let mut f = step;
        while f <= map.fmax {
            if f >= map.fmin {
                out.push(GridLine {
                    freq: f,
                    label: short_hz(f),
                    sub_label: None,
                    major: (f % (step * 5.0)).abs() < 1.0,
                });
            }
            f += step;
        }
        return out;
    }

    let octaves = (map.fmax / map.fmin).log2();
    let px_per_octave = if axis_pixels <= 0 || octaves <= 0.0 {
        0.0
    } else {
        axis_pixels as f64 / octaves
    };
    // The finest subdivision whose labels still fit.
    let step = [1, 2, 3, 4, 6, 12]
        .into_iter()
        .find(|&semis| px_per_octave > 0.0 && px_per_octave * semis as f64 / 12.0 >= target)
        .unwrap_or(12);
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let mut midi = 12;
    while midi <= 132 {
        let f = midi_to_freq(midi as f64);
        if f >= map.fmin && f <= map.fmax {
            let note = format!("{}{}", NAMES[midi % 12], midi / 12 - 1);
            let (label, sub_label) = match mode {
                AxisLabelMode::Frequency => (short_hz(f), None),
                AxisLabelMode::Notes => (note, None),
                AxisLabelMode::Both => (note, Some(short_hz(f))),
            };
            out.push(GridLine {
                freq: f,
                label,
                sub_label,
                major: midi % 12 == 0,
            });
        }
        midi += step;
    }
    out
}

/// One style for the whole axis: "440", "1.5k", "12k".
pub fn short_hz(f: f64) -> String {
    if f >= 1000.0 {
        let k = format!("{:.1}", f / 1000.0);
        format!("{}k", k.strip_suffix(".0").unwrap_or(&k))
    } else {
        format!("{f:.0}")
    }
}

/// The row of the display axis a frequency falls on, in framebuffer pixels.
fn row_of(map: &FrequencyMap, top: i32, h: i32, f: f64) -> i32 {
    top + h - 1 - map.freq_to_x(f).round() as i32
}

/// Gridlines across every pane, the ground under the label columns, and the note or
/// frequency labels centred on their rows in the gutter and the outer columns.
fn grid(o: &mut Overlay, sc: &Scales<'_>) {
    let l = sc.layout;
    let Some(first) = l.panes.first() else {
        return;
    };
    let s = sc.settings;
    let t = &s.theme;
    let (top, h) = (first.spectro.y, first.spectro.h);
    let map = sc.map;
    let lines = grid_lines(map, s.label_mode, h);
    let px = sc.label_px();

    // All three label columns stand on the same ground.
    let ground = pick_keep_alpha(t.panel, Rgba::argb(255, 12, 12, 15)).faded(sc.alpha);
    for r in [l.gutter, l.outer_left, l.outer_right] {
        fill(o, Layer::Over, r, ground);
    }

    let mut unit_floor = 0.0;
    if s.show_axis_labels && s.show_scale_units {
        axis_unit(o, sc, top);
        // With no lane the caption sits on the axis, so the rows start below it.
        if first.lane.h == 0 {
            unit_floor = o.measure("Hz", Face::Sans, px).h + 2.0;
        }
    }

    // Semitone lines, once there's room for them to read as lines.
    if s.show_semitones && map.scale != FreqScale::Linear {
        let px_per_octave = h as f64 / (map.fmax / map.fmin).log2();
        if px_per_octave > 96.0 && lines.len() < 40 {
            let fine = pick_keep_alpha(t.grid_minor, Rgba::argb(16, 255, 255, 255)).faded(sc.alpha);
            for midi in (12..=132).filter(|m| m % 12 != 0) {
                let f = midi_to_freq(midi as f64);
                if f < map.fmin || f > map.fmax {
                    continue;
                }
                let y = row_of(map, top, h, f);
                if y >= top && y < top + h {
                    o.hline(
                        Layer::Over,
                        l.bounds.x as f32,
                        l.bounds.right() as f32,
                        y as f32,
                        fine,
                    );
                }
            }
        }
    }

    let major = pick_keep_alpha(t.grid_major, Rgba::argb(38, 255, 255, 255)).faded(sc.alpha);
    let minor = pick_keep_alpha(t.grid_minor, Rgba::argb(20, 255, 255, 255)).faded(sc.alpha);
    let text = pick_keep_alpha(t.axis_text, Rgba::argb(185, 232, 232, 238)).faded(sc.alpha);
    let text_minor = pick_keep_alpha(t.axis_text, Rgba::argb(120, 210, 210, 220)).faded(sc.alpha);
    for line in &lines {
        let y = row_of(map, top, h, line.freq);
        if y < top || y >= top + h {
            continue;
        }
        let pen = if line.major { major } else { minor };
        let yf = y as f32;
        // Across the image only; the label columns carry a tick instead.
        for p in &l.panes {
            o.hline(
                Layer::Over,
                p.bounds.x as f32,
                p.bounds.right() as f32,
                yf,
                pen,
            );
        }
        if !s.show_axis_labels {
            continue;
        }
        let outer_ok = yf >= sc.label_floor_y;
        let ink = if line.major { text } else { text_minor };
        let sz = o.measure(&line.label, Face::Sans, px);
        let line_h = sz.h - 2.0;
        let block_h = if line.sub_label.is_some() {
            line_h + sz.h
        } else {
            sz.h
        };
        // Centred on its row, or not drawn at all.
        let ly = yf - block_h / 2.0;
        if ly < top as f32 + unit_floor || ly + block_h > (top + h) as f32 {
            continue;
        }
        const TICK: f32 = 4.0;
        let mut columns = Vec::with_capacity(3);
        if l.gutter.w >= 22 {
            let g = l.gutter;
            o.hline(Layer::Over, g.x as f32, g.x as f32 + TICK, yf, pen);
            o.hline(
                Layer::Over,
                g.right() as f32 - TICK,
                g.right() as f32,
                yf,
                pen,
            );
            columns.push(g);
        }
        if l.outer_left.w > 0 && outer_ok {
            let (ol, or) = (l.outer_left, l.outer_right);
            o.hline(
                Layer::Over,
                ol.right() as f32 - TICK,
                ol.right() as f32,
                yf,
                pen,
            );
            o.hline(Layer::Over, or.x as f32, or.x as f32 + TICK, yf, pen);
            columns.extend([ol, or]);
        }
        for c in columns {
            centred(o, Layer::Over, &line.label, px, c, ly, ink);
            if let Some(sub) = &line.sub_label {
                centred(o, Layer::Over, sub, px, c, ly + line_h, ink);
            }
        }
    }
}

/// Names the frequency axis at the top of every column that carries it: in the lane
/// when there is one, on a chip at the top of the axis when not.
fn axis_unit(o: &mut Overlay, sc: &Scales<'_>, axis_top: i32) {
    let s = sc.settings;
    let unit = if s.label_mode == AxisLabelMode::Notes && sc.map.scale != FreqScale::Linear {
        "note"
    } else {
        "Hz"
    };
    let px = sc.label_px();
    let sz = o.measure(unit, Face::Sans, px);
    let lane = sc.layout.panes[0].lane;
    let in_lane = lane.h as f32 >= sz.h;
    let y = if in_lane {
        lane.y as f32 + (lane.h as f32 - sz.h) / 2.0
    } else {
        axis_top as f32 + 1.0
    };
    let chip = Rgba::argb(190, 8, 8, 11).faded(sc.alpha);
    let ink = pick_keep_alpha(s.theme.units, Rgba::argb(190, 150, 200, 245)).faded(sc.alpha);
    let l = sc.layout;
    for column in [l.gutter, l.outer_left, l.outer_right] {
        if (column.w as f32) < sz.w + 2.0 {
            continue;
        }
        let x = column.x as f32 + (column.w as f32 - sz.w) / 2.0;
        if !in_lane {
            chip_behind(o, Layer::Over, x, y, sz, chip);
        }
        o.text(Layer::Over, unit, Face::Sans, px, x, y, ink);
    }
}

/// Each pane's channel name, parked at the past end, opposite the curve, a line below
/// the top so it doesn't read as part of the first axis label.
fn pane_labels(o: &mut Overlay, sc: &Scales<'_>) {
    let back = Rgba::argb(150, 8, 8, 10).faded(sc.alpha);
    let ink = Rgba::argb(225, 240, 240, 245).faded(sc.alpha);
    let px = sc.label_px();
    for p in &sc.layout.panes {
        let r = p.spectro;
        let sz = o.measure(p.label, Face::Sans, px);
        let x = if p.curve_on_left {
            r.right() as f32 - sz.w - 8.0
        } else {
            r.x as f32 + 5.0
        };
        let y = r.y as f32 + 3.0 + sc.top_inset + sz.h + 4.0;
        o.rect(Layer::Over, x - 3.0, y, sz.w + 6.0, sz.h, back);
        o.text(Layer::Over, p.label, Face::Sans, px, x, y - 1.0, ink);
    }
}

fn fill(o: &mut Overlay, layer: Layer, r: Rect, colour: Rgba) {
    o.rect(
        layer, r.x as f32, r.y as f32, r.w as f32, r.h as f32, colour,
    );
}

/// The dark chip that keeps a label readable over the image.
fn chip_behind(o: &mut Overlay, layer: Layer, x: f32, y: f32, sz: TextSize, colour: Rgba) {
    o.rect(layer, x - 2.0, y - 1.0, sz.w + 4.0, sz.h + 1.0, colour);
}

/// `text` centred across `column`, its top at `y`.
fn centred(o: &mut Overlay, layer: Layer, text: &str, px: f32, column: Rect, y: f32, ink: Rgba) {
    let sz = o.measure(text, Face::Sans, px);
    let x = column.x as f32 + (column.w as f32 - sz.w) / 2.0;
    o.text(layer, text, Face::Sans, px, x, y, ink);
}

/// `x` moved so a label `w` wide stays inside `r`.
fn clamp_into(x: f32, w: f32, r: Rect) -> f32 {
    let x = if x + w > r.right() as f32 {
        r.right() as f32 - w
    } else {
        x
    };
    x.max(r.x as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_lines_subdivide_the_octave_to_fit() {
        let map = FrequencyMap::new(FreqScale::Note, 2000, 20.0, 20000.0);
        // 2000 px over ten octaves is 200 px an octave, with labels wanted every 64 px:
        // major thirds, 67 px apart.
        let lines = grid_lines(&map, AxisLabelMode::Notes, 2000);
        let labels: Vec<&str> = lines.iter().map(|l| l.label.as_str()).collect();
        // E0 is 20.6 Hz, just inside the axis.
        assert_eq!(labels[..4], ["E0", "G#0", "C1", "E1"]);
        assert!(lines[2].major && !lines[3].major);
        // A short strip gets octaves only.
        let short = grid_lines(&map, AxisLabelMode::Notes, 200);
        assert!(short.iter().all(|l| l.major));
        assert_eq!(short.len(), 10);
        assert_eq!(short.last().unwrap().label, "C10");
    }

    #[test]
    fn both_mode_adds_frequencies_under_notes() {
        let map = FrequencyMap::new(FreqScale::Note, 400, 20.0, 20000.0);
        let lines = grid_lines(&map, AxisLabelMode::Both, 400);
        let c4 = lines.iter().find(|l| l.label == "C4").unwrap();
        assert_eq!(c4.sub_label.as_deref(), Some("262"));
        let c8 = lines.iter().find(|l| l.label == "C8").unwrap();
        assert_eq!(c8.sub_label.as_deref(), Some("4.2k"));
    }

    #[test]
    fn linear_lines_step_in_round_hertz() {
        let map = FrequencyMap::new(FreqScale::Linear, 600, 0.0, 20000.0);
        let lines = grid_lines(&map, AxisLabelMode::Notes, 600);
        // 600 px over 20 kHz: 1 kHz steps are 30 px, short of the 33 px wanted.
        assert_eq!(lines[0].freq, 2000.0);
        assert_eq!(lines[0].label, "2k");
        assert!(lines.iter().any(|l| l.major && l.freq == 10000.0));
    }

    #[test]
    fn hertz_read_one_way() {
        assert_eq!(short_hz(440.0), "440");
        assert_eq!(short_hz(1500.0), "1.5k");
        assert_eq!(short_hz(12000.0), "12k");
        assert_eq!(short_hz(4186.0), "4.2k");
    }

    #[test]
    fn marks_every_85_px_or_so() {
        assert_eq!(time_step(60.0), 2.0);
        assert_eq!(time_step(100.0), 1.0);
        assert_eq!(time_step(15.0), 10.0);
    }
}
