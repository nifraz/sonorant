//! Where the panes go: the geometry rules of Nostalgia+'s stereo view.
//!
//! Two panes sit either side of a centre gutter that carries the frequency labels, or
//! one full-width pane for single-channel modes. Each pane has a curve strip on one side
//! and the spectrogram on the other, and optionally a scale strip at the top or bottom.
//! With room to spare, the axis labels repeat in reserved columns at both outer edges.

use sonorant_core::dsp::ChannelPairMode;
use sonorant_core::settings::{ScaleLanePosition, Settings};

/// A rectangle in pixels: x, y, width, height. Integer, like the layouts it reproduces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const EMPTY: Rect = Rect {
        x: 0,
        y: 0,
        w: 0,
        h: 0,
    };

    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    /// As `[x, y, w, h]` floats, for GPU viewports.
    pub fn to_f32(self) -> [f32; 4] {
        [self.x as f32, self.y as f32, self.w as f32, self.h as f32]
    }
}

/// One pane's parts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaneLayout {
    pub label: &'static str,
    pub bounds: Rect,
    pub curve: Rect,
    pub spectro: Rect,
    /// The reserved scale strip, or empty. With a strip at both ends this is the top
    /// one, so everything that asks "is there a strip over the image?" still reads
    /// this field and gets the same answer.
    pub lane: Rect,
    /// The second strip when the scales are repeated at both ends, or empty.
    pub lane_bottom: Rect,
    pub curve_on_left: bool,
}

impl PaneLayout {
    fn new(
        label: &'static str,
        bounds: Rect,
        curve_width: i32,
        curve_on_left: bool,
        lane_height: i32,
        lane_pos: ScaleLanePosition,
    ) -> PaneLayout {
        let mut b = bounds;
        b.w = b.w.max(8);
        b.h = b.h.max(8);
        let curve_width = curve_width.max(0).min((b.w - 16).max(0));
        // Never let the strips eat the image: on a short panel the scales are worth
        // less than the pixels they would cost. Asking for both ends and getting one
        // is better than asking for both and getting none, so the second is dropped
        // first and only then the first.
        let mut wanted = match lane_pos {
            _ if lane_height <= 0 => 0,
            ScaleLanePosition::Both => 2,
            _ => 1,
        };
        while wanted > 0 && wanted * lane_height > b.h / 4 {
            wanted -= 1;
        }
        let top = Rect::new(b.x, b.y, b.w, lane_height);
        let bottom = Rect::new(b.x, b.bottom() - lane_height, b.w, lane_height);
        let (lane, lane_bottom) = match (wanted, lane_pos) {
            (0, _) => (Rect::EMPTY, Rect::EMPTY),
            (1, ScaleLanePosition::Bottom) => (bottom, Rect::EMPTY),
            (1, _) => (top, Rect::EMPTY),
            (_, _) => (top, bottom),
        };
        let used = wanted * lane_height;
        let body = Rect::new(
            b.x,
            if lane.is_empty() || lane.y > b.y {
                b.y
            } else {
                b.y + lane_height
            },
            b.w,
            b.h - used,
        );
        let spec_w = (body.w - curve_width).max(1);
        let (curve, spectro) = if curve_on_left {
            (
                Rect::new(body.x, body.y, curve_width, body.h),
                Rect::new(body.x + curve_width, body.y, spec_w, body.h),
            )
        } else {
            (
                Rect::new(body.x + spec_w, body.y, curve_width, body.h),
                Rect::new(body.x, body.y, spec_w, body.h),
            )
        };
        PaneLayout {
            label,
            bounds: b,
            curve,
            spectro,
            lane,
            lane_bottom,
            curve_on_left,
        }
    }

    /// The reserved strips, top first, skipping the ends that have none.
    pub fn lanes(&self) -> impl Iterator<Item = Rect> {
        [self.lane, self.lane_bottom]
            .into_iter()
            .filter(|r| !r.is_empty())
    }

    /// Rows back from the newest at pixel column `x`, or `None` off the spectrogram. The
    /// newest row is at the edge nearest the curve strip.
    pub fn age_at(&self, x: i32, px_per_row: f64) -> Option<f64> {
        let s = self.spectro;
        if x < s.x || x >= s.right() {
            return None;
        }
        let from_newest = if self.curve_on_left {
            x - s.x
        } else {
            s.right() - 1 - x
        };
        Some(from_newest as f64 / px_per_row.max(1e-9))
    }
}

/// The stereo view's geometry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScopeLayout {
    pub bounds: Rect,
    pub outer_left: Rect,
    pub outer_right: Rect,
    pub gutter: Rect,
    pub panes: Vec<PaneLayout>,
}

/// Width of the outer label columns at the default label size.
pub const AXIS_MARGIN: i32 = 30;

impl ScopeLayout {
    /// Lays out `bounds` for `s` at `scale` pixels per point. Single-channel modes get
    /// one full-width pane.
    ///
    /// The panes themselves are in physical pixels, so the image keeps a row to a pixel
    /// and loses no detail at 150%; the fixed sizes around them are scaled, so the
    /// label columns and the scale strip stay the size they look on screen.
    pub fn new(bounds: Rect, s: &Settings, scale: f32) -> ScopeLayout {
        let px = |n: i32| (n as f32 * scale).round() as i32;
        // The repeated axis at both edges needs space of its own: overlaid on the graph
        // fill it's unreadable at these widths.
        let wanted = px(AXIS_MARGIN + ((s.label_font_size - 7.0) * 3.0).max(0.0) as i32);
        let margin = if s.show_outer_labels && s.show_axis_labels && bounds.w > 6 * wanted {
            wanted
        } else {
            0
        };
        let outer_left = Rect::new(bounds.x, bounds.y, margin, bounds.h);
        let outer_right = Rect::new(bounds.right() - margin, bounds.y, margin, bounds.h);
        let inner = Rect::new(
            bounds.x + margin,
            bounds.y,
            (bounds.w - 2 * margin).max(16),
            bounds.h,
        );

        let pane_count = s.pair_mode.pane_count() as i32;
        let gutter = if pane_count == 2 {
            s.gutter_width.clamp(0, px(90))
        } else {
            0
        };
        let lane_h = px(s.scale_lane_height());
        let labels = s.pair_mode.pane_labels();
        let pane_w = ((inner.w - gutter) / pane_count.max(1)).max(px(8));
        let curve_width = pane_w * s.curve_width_pct.clamp(0, 60) / 100;

        // Mirroring the left pane puts both curves and both newest columns against the
        // centre, so history flows outward from the middle.
        let left_on_left = if pane_count == 2 && s.mirror_left_pane {
            !s.curve_on_left
        } else {
            s.curve_on_left
        };

        let (panes, gutter_rect) = if pane_count == 2 {
            let left = PaneLayout::new(
                labels[0],
                Rect::new(inner.x, inner.y, pane_w, inner.h),
                curve_width,
                left_on_left,
                lane_h,
                s.scale_lane_pos,
            );
            let gx = inner.x + pane_w;
            let right_x = gx + gutter;
            let right = PaneLayout::new(
                labels[1],
                Rect::new(right_x, inner.y, (inner.right() - right_x).max(8), inner.h),
                curve_width,
                s.curve_on_left,
                lane_h,
                s.scale_lane_pos,
            );
            (vec![left, right], Rect::new(gx, inner.y, gutter, inner.h))
        } else {
            (
                vec![PaneLayout::new(
                    labels[0],
                    inner,
                    curve_width,
                    s.curve_on_left,
                    lane_h,
                    s.scale_lane_pos,
                )],
                Rect::EMPTY,
            )
        };
        ScopeLayout {
            bounds,
            outer_left,
            outer_right,
            gutter: gutter_rect,
            panes,
        }
    }

    /// Spectrogram rows (frequency columns of the curves) the panes show: their height.
    pub fn columns(&self) -> usize {
        self.panes
            .first()
            .map_or(1, |p| p.spectro.h.max(1) as usize)
    }
}

/// Which pair the panes show, for callers that only have the mode.
pub fn pane_count(mode: ChannelPairMode) -> usize {
    mode.pane_count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrored_panes_meet_at_the_centre() {
        let s = Settings {
            mirror_left_pane: true,
            ..Settings::default()
        };
        let l = ScopeLayout::new(Rect::new(0, 0, 1200, 600), &s, 1.0);
        assert!(!l.panes[0].curve_on_left && l.panes[1].curve_on_left);
        assert_eq!(l.panes[0].bounds.right(), l.gutter.x);
        assert_eq!(l.gutter.right(), l.panes[1].bounds.x);
    }

    #[test]
    fn a_strip_at_both_ends_repeats_it_rather_than_splitting_it() {
        let at = |pos| {
            let s = Settings {
                scale_lane_pos: pos,
                ..Settings::default()
            };
            ScopeLayout::new(Rect::new(0, 0, 1200, 600), &s, 1.0)
        };
        let top = at(ScaleLanePosition::Top);
        let both = at(ScaleLanePosition::Both);
        let (a, b) = (&top.panes[0], &both.panes[0]);
        let h = a.lane.h;
        assert!(h > 0, "the default settings reserve a strip");

        // The top strip is the same strip, at the same size: `Both` adds one rather
        // than halving what was there.
        assert_eq!(b.lane, a.lane);
        assert_eq!(
            b.lane_bottom,
            Rect::new(b.bounds.x, b.bounds.bottom() - h, b.bounds.w, h)
        );
        assert_eq!(b.lanes().count(), 2);
        // The image starts where it did and pays for the second strip once.
        assert_eq!(b.spectro.y, a.spectro.y);
        assert_eq!(b.spectro.h, a.spectro.h - h);
        assert_eq!(b.spectro.bottom(), b.lane_bottom.y);

        // At one end, whichever end, there is one strip and `lane` is it.
        let bottom = at(ScaleLanePosition::Bottom);
        let c = &bottom.panes[0];
        assert_eq!(c.lane, b.lane_bottom);
        assert_eq!(c.lane_bottom, Rect::EMPTY);
        assert_eq!(c.lanes().count(), 1);
        assert_eq!(c.spectro.y, c.bounds.y);
    }

    #[test]
    fn a_short_pane_gives_up_the_second_strip_before_the_first() {
        let s = Settings {
            scale_lane_pos: ScaleLanePosition::Both,
            ..Settings::default()
        };
        let h = s.scale_lane_height();
        let count = |view_h| {
            ScopeLayout::new(Rect::new(0, 0, 1200, view_h), &s, 1.0).panes[0]
                .lanes()
                .count()
        };
        // The strips may have a quarter of the pane between them, no more.
        assert_eq!(count(h * 8), 2);
        assert_eq!(count(h * 6), 1);
        assert_eq!(count(h * 3), 0);
    }

    #[test]
    fn age_runs_away_from_the_curve() {
        // Unmirrored, which is the plain rule; the mirrored pair has a test of its own.
        let s = Settings {
            mirror_left_pane: false,
            ..Settings::default()
        };
        let l = ScopeLayout::new(Rect::new(0, 0, 1200, 600), &s, 1.0);
        let p = &l.panes[0];
        assert_eq!(p.age_at(p.spectro.x, 1.0), Some(0.0));
        assert_eq!(p.age_at(p.spectro.x + 120, 1.0), Some(120.0));
        assert_eq!(p.age_at(p.curve.x, 1.0), None);
    }
}
