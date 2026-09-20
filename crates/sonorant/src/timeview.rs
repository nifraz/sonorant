//! Where along the history the image is looking.
//!
//! Two numbers: how far the wheel has stretched a row, and how many rows the image's
//! newest edge sits behind the newest row there is. Zero rows behind is the live view,
//! following the audio; anything else is parked, which the app carries as the freeze
//! anchor it already had.
//!
//! The arithmetic lives here rather than in the frame loop because the one thing that
//! has to be right is easy to get subtly wrong: zooming about the pointer has to leave
//! the row under the pointer where it was, or the picture slides away from under the
//! hand doing the zooming.

/// How far one notch of the wheel zooms, as a fraction of an octave. An eighth is fine
/// enough to aim with and still crosses an octave in eight notches.
const NOTCH: f64 = 1.0 / 8.0;

/// The zoom and the place in the history the image is looking at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeView {
    /// Multiplies the pixels-a-row setting. 1 is the setting as it stands.
    pub zoom: f64,
    /// Rows the newest edge of the image is behind the newest row there is.
    pub offset: f64,
}

impl Default for TimeView {
    fn default() -> TimeView {
        TimeView {
            zoom: 1.0,
            offset: 0.0,
        }
    }
}

/// The pane the view is being moved over: how wide it is in pixels, and how many pixels
/// a row is drawn across before the zoom.
#[derive(Clone, Copy, Debug)]
pub struct Strip {
    pub width: f64,
    pub px_per_row: f64,
}

impl Strip {
    /// Rows across the pane at this zoom.
    fn visible(&self, zoom: f64) -> f64 {
        self.width / (self.px_per_row * zoom).max(0.05)
    }
}

impl TimeView {
    /// Zooms by `notches` of the wheel about `along`, keeping the row that was under
    /// that point under it.
    pub fn zoom_about(&mut self, notches: f64, along: f64, strip: Strip, range: (f64, f64)) {
        let before = strip.visible(self.zoom);
        self.zoom = (self.zoom * 2f64.powf(notches * NOTCH)).clamp(range.0, range.1);
        let after = strip.visible(self.zoom);
        // The near edge takes up whatever the span between it and the pointer gained or
        // lost, which is what leaves the pointer's own row where it was.
        self.offset += along * (before - after);
    }

    /// Pans by a drag of `pixels` along the pane. `newest_left` says which way time
    /// runs; the image follows the pointer either way, so dragging towards the newest
    /// edge brings older rows in behind it.
    pub fn pan(&mut self, pixels: f64, newest_left: bool, strip: Strip) {
        let rows = pixels / (strip.px_per_row * self.zoom).max(0.05);
        self.offset += if newest_left { -rows } else { rows };
    }

    /// Holds the view inside the history: not past now, and not past the oldest row the
    /// store still has. A screen of rows is kept in view, so a drag to the end leaves a
    /// full image rather than an empty one.
    pub fn clamp_to(&mut self, written: f64, capacity: f64, strip: Strip) {
        let reach = (capacity - strip.visible(self.zoom)).max(0.0);
        self.offset = self.offset.clamp(0.0, reach.min(written).max(0.0));
    }

    /// Whether the image is parked in the history rather than following the newest row.
    /// Half a row is under a pixel at any zoom worth having, so it counts as live.
    pub fn parked(&self) -> bool {
        self.offset >= 0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STRIP: Strip = Strip {
        width: 800.0,
        px_per_row: 2.0,
    };
    const RANGE: (f64, f64) = (0.05, 20.0);

    /// The age in rows of the column at `along`, as `spectrogram.wgsl` works it out:
    /// the view's near edge plus however far across the pane the column is. The
    /// invariants below are checked against this rather than against the fields, so
    /// they say something about the picture rather than about the arithmetic.
    fn age_at(view: &TimeView, along: f64, strip: Strip) -> f64 {
        view.offset + along * strip.visible(view.zoom)
    }

    /// The whole point of zooming about the pointer: whatever row was under it is still
    /// under it afterwards, wherever on the pane it was and whichever way it zoomed.
    #[test]
    fn zooming_leaves_the_row_under_the_pointer_where_it_was() {
        for along in [0.0, 0.25, 0.5, 0.9, 1.0] {
            for notches in [-6.0, -1.0, 1.0, 6.0] {
                let mut view = TimeView {
                    zoom: 1.0,
                    offset: 300.0,
                };
                let before = age_at(&view, along, STRIP);
                view.zoom_about(notches, along, STRIP, RANGE);
                let after = age_at(&view, along, STRIP);
                assert!(
                    (before - after).abs() < 1e-9,
                    "{along} at {notches} notches: row {before} became {after}"
                );
            }
        }
    }

    #[test]
    fn zooming_in_and_back_out_returns_to_where_it_started() {
        let mut view = TimeView {
            zoom: 1.0,
            offset: 120.0,
        };
        view.zoom_about(5.0, 0.7, STRIP, RANGE);
        view.zoom_about(-5.0, 0.7, STRIP, RANGE);
        assert!((view.zoom - 1.0).abs() < 1e-9, "{}", view.zoom);
        assert!((view.offset - 120.0).abs() < 1e-9, "{}", view.offset);
    }

    #[test]
    fn the_zoom_stops_at_the_ends_of_its_range() {
        let mut view = TimeView::default();
        view.zoom_about(1000.0, 0.5, STRIP, RANGE);
        assert_eq!(view.zoom, RANGE.1);
        view.zoom_about(-1000.0, 0.5, STRIP, RANGE);
        assert_eq!(view.zoom, RANGE.0);
    }

    /// The image follows the pointer: dragging towards the newest edge brings older
    /// rows in behind it, and which edge that is depends on the pane.
    #[test]
    fn a_drag_takes_the_image_with_it() {
        let mut left = TimeView {
            offset: 500.0,
            ..TimeView::default()
        };
        // Newest on the left, so older is off the right edge: dragging left fetches it.
        left.pan(-100.0, true, STRIP);
        assert_eq!(left.offset, 550.0);
        left.pan(100.0, true, STRIP);
        assert_eq!(left.offset, 500.0);

        let mut right = TimeView {
            offset: 500.0,
            ..TimeView::default()
        };
        // Newest on the right: older is off the left edge, so dragging right fetches it.
        right.pan(100.0, false, STRIP);
        assert_eq!(right.offset, 550.0);
    }

    /// A drag is in pixels, so the same drag walks further through the history the
    /// further out the view is zoomed.
    #[test]
    fn a_drag_covers_more_history_when_the_view_is_zoomed_out() {
        let mut near = TimeView {
            zoom: 4.0,
            ..TimeView::default()
        };
        let mut far = TimeView {
            zoom: 0.25,
            ..TimeView::default()
        };
        near.pan(-100.0, true, STRIP);
        far.pan(-100.0, true, STRIP);
        assert!(
            far.offset > near.offset * 15.0,
            "{} {}",
            far.offset,
            near.offset
        );
    }

    #[test]
    fn the_view_cannot_leave_the_history() {
        let strip = STRIP;
        let visible = strip.visible(1.0);
        let mut view = TimeView {
            offset: -50.0,
            ..TimeView::default()
        };
        view.clamp_to(10_000.0, 18_000.0, strip);
        assert_eq!(view.offset, 0.0, "the view cannot get ahead of now");

        view.offset = 1e9;
        view.clamp_to(10_000.0, 18_000.0, strip);
        assert_eq!(view.offset, 10_000.0, "nor past the rows there are");

        // With a full store, the last screen of rows stays in view.
        view.offset = 1e9;
        view.clamp_to(1e9, 18_000.0, strip);
        assert_eq!(view.offset, 18_000.0 - visible);
    }

    #[test]
    fn under_half_a_row_back_is_still_live() {
        let mut view = TimeView::default();
        assert!(!view.parked());
        view.offset = 0.4;
        assert!(!view.parked());
        view.offset = 0.6;
        assert!(view.parked());
    }
}
