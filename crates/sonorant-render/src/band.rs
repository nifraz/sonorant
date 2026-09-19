//! The strip along the bottom of either view: a scrolling waveform lane under each
//! pane, and the centre deck in the gap they leave between them.
//!
//! Ported from Nostalgia+'s `BottomBand` and `CenterDeck`, rectangle for rectangle (the
//! reference layouts in `tests/reference/layout.json` check it). The deck fills the gap
//! between the two spectrograms with everything that describes the pair rather than one
//! side of it: the goniometer on the axis the display mirrors about, what's playing in
//! the left half, and what it's doing in the right. Whatever doesn't fit is dropped,
//! least useful first, because that gap is only as wide as the graph strips plus the
//! gutter.
//!
//! Some of it depends on how wide text is, so the caller lends a measuring function.

use sonorant_core::settings::Settings;

use crate::layout::{PaneLayout, Rect};

/// Measures a string's width in pixels at the label font size.
pub type MeasureWidth<'a> = &'a mut dyn FnMut(&str) -> f32;

/// Air around the deck's blocks.
const PAD: i32 = 6;
/// The least width the transport block is worth giving.
const MIN_STACK: i32 = 150;
/// Shortest seek bar worth clicking; below it the bar takes a row of its own.
const MIN_SEEK: i32 = 60;
/// Air either side of the shared bar column.
const BAR_GAP: i32 = 10;
/// Enough for a short title at the label font; below it the block says nothing.
const INFO_MIN: i32 = 120;
/// One readout column at the default label size.
const LOUD_COL: i32 = 56;

/// Where each part of the centre deck goes. Empty rectangles are parts that didn't fit
/// or are switched off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeckLayout {
    pub bounds: Rect,
    pub art: Rect,
    pub info: Rect,
    pub goniometer: Rect,
    /// The transport and meters block.
    pub stack: Rect,
    /// The readout grid.
    pub loudness: Rect,
    pub prev: Rect,
    pub play: Rect,
    pub next: Rect,
    pub seek: Rect,
    pub clock: Rect,
    pub correlation: Rect,
    pub balance: Rect,
    /// The x range the seek bar and both meter bars share, so they line up.
    pub bar: Rect,
    pub loud_cols: i32,
    pub loud_rows: i32,
    pub loud_col_w: i32,
}

impl DeckLayout {
    /// The least the deck can usefully be: below this its three rows are shorter than
    /// the text they carry.
    pub const PREFERRED_HEIGHT: i32 = 92;

    /// Lays the deck out in `gap`, measuring text with `measure`.
    pub fn new(gap: Rect, s: &Settings, measure: MeasureWidth<'_>) -> DeckLayout {
        let mut d = DeckLayout {
            bounds: gap,
            ..DeckLayout::default()
        };
        if gap.w <= 40 || gap.h <= 24 {
            return d;
        }
        let h = gap.h - PAD * 2;
        let y = gap.y + PAD;

        // The goniometer first, and in the middle, because it's the one thing here that
        // belongs to both channels at once. A fifth of the width: there are two blocks
        // either side of it that need the room more.
        let square = h.min(gap.w / 5);
        if s.deck_show_goniometer && square >= 24 {
            d.goniometer = Rect::new(
                gap.x + gap.w / 2 - square / 2,
                y + (h - square) / 2,
                square,
                square,
            );
        }
        let mid = gap.x + gap.w / 2;
        let (left_end, right_start) = if d.goniometer.w > 0 {
            (d.goniometer.x - PAD, d.goniometer.right() + PAD)
        } else {
            (mid - PAD, mid + PAD)
        };
        d.layout_left(Rect::new(gap.x + PAD, y, left_end - (gap.x + PAD), h), s);
        d.layout_right(
            Rect::new(right_start, y, gap.right() - PAD - right_start, h),
            s,
            measure,
        );
        d
    }

    /// Artwork then metadata. The artwork goes first when the text is squeezed: half a
    /// cover with no room to say what it's the cover of is the wrong trade.
    fn layout_left(&mut self, r: Rect, s: &Settings) {
        if r.w < 40 || r.h < 20 {
            return;
        }
        let mut square = if s.deck_show_artwork && r.h >= 24 {
            r.h.min(r.w / 2)
        } else {
            0
        };
        if square > 0 && s.deck_show_track_info && r.w - square - PAD < INFO_MIN {
            square = 0;
        }
        let mut x = r.x;
        if square >= 24 {
            self.art = Rect::new(x, r.y, square, r.h);
            x += square + PAD;
        }
        if s.deck_show_track_info && r.right() - x >= INFO_MIN {
            self.info = Rect::new(x, r.y, r.right() - x, r.h);
        }
    }

    /// The transport block, then the readout grid. The grid sheds columns from the
    /// right until the transport has room to stay usable; the readouts are in priority
    /// order, so brightness and tempo go first and the LUFS figures last.
    fn layout_right(&mut self, r: Rect, s: &Settings, measure: MeasureWidth<'_>) {
        if r.w < 40 || r.h < 20 {
            return;
        }
        let player = s.deck_show_transport;
        let (corr, bal) = (s.deck_show_correlation, s.deck_show_balance);
        let meters = corr as i32 + bal as i32;
        let stack = player || meters > 0;

        let loud_col = LOUD_COL + ((s.label_font_size - 7.0).max(0.0) * 5.0) as i32;
        // A caption over a value, plus air. Three rows at a tall deck turns nine
        // readouts into three columns instead of five.
        let cell = (s.label_font_size * 4.6) as i32 + 4;
        let rows = (r.h / cell.max(1)).clamp(2, 3);
        let mut cols = (loud_count(s) + rows - 1) / rows;

        let m = if stack {
            StackMetrics::measure(r.h, measure, player, meters)
        } else {
            StackMetrics::default()
        };
        let reserve = if stack { MIN_STACK + PAD } else { 0 };
        // Measured rather than guessed: three buttons, the longest clock and a bar
        // worth aiming at, all of which depend on the font and the deck's height.
        let want = if stack && player {
            m.cap.max(m.buttons as f32) as i32
                + BAR_GAP * 2
                + MIN_SEEK
                + m.val.max(m.clock) as i32
                + PAD
        } else {
            reserve
        };

        // The single transport row is asked for ahead of the last readout columns, but
        // never down to no readouts at all: one column beats a tidier transport.
        while cols > 1 && r.w - cols * loud_col < want {
            cols -= 1;
        }
        while cols > 0 && r.w - cols * loud_col < reserve {
            cols -= 1;
        }

        let mut x = r.x;
        if stack {
            let w = r.w - if cols > 0 { cols * loud_col + PAD } else { 0 };
            if w >= 60 {
                self.stack = Rect::new(x, r.y, w, r.h);
                self.layout_stack(player, corr, bal, &m);
                x = self.stack.right() + PAD;
            }
        }
        if cols > 0 && r.right() - x >= cols * loud_col {
            self.loud_cols = cols;
            self.loud_rows = rows;
            self.loud_col_w = loud_col;
            self.loudness = Rect::new(x, r.y, cols * loud_col, r.h);
        }
    }

    /// Rows down the block: the transport with its seek bar, then whichever meters are
    /// switched on. Sharing one row with the buttons gives a row back to the metadata
    /// opposite, and puts all three bars in one column so they start and end together.
    fn layout_stack(&mut self, player: bool, corr: bool, bal: bool, m: &StackMetrics) {
        let meters = corr as i32 + bal as i32;
        if !player && meters == 0 {
            return;
        }
        let mut rows = player as i32 + meters;
        let mut row_h = self.stack.h / rows;
        let mut btn = (row_h - 2).clamp(12, 28);
        let mut buttons = if player { (btn + 6) * 3 - 6 } else { 0 };
        let mut widest = m.cap.max(buttons as f32);
        // Half a seek bar is worse than an extra row: you can't drop a cursor on thirty
        // pixels.
        let one_row = !player
            || self.stack.w as f32 - widest - m.val.max(m.clock) - (BAR_GAP * 2) as f32
                >= MIN_SEEK as f32;
        if !one_row {
            rows += 1; // the seek bar gets one of its own
            row_h = self.stack.h / rows;
            btn = (row_h - 2).clamp(12, 28);
            buttons = (btn + 6) * 3 - 6;
            widest = if meters > 0 { m.cap } else { buttons as f32 };
        }
        if row_h < 10 {
            return;
        }

        let mut bar_left = self.stack.x + widest as i32 + BAR_GAP;
        let mut bar_right =
            self.stack.right() - m.val.max(if one_row { m.clock } else { 0.0 }) as i32 - BAR_GAP;
        if bar_right - bar_left < 24 {
            bar_left = 0;
            bar_right = 0;
        } else {
            self.bar = Rect::new(bar_left, self.stack.y, bar_right - bar_left, self.stack.h);
        }

        let mut y = self.stack.y;
        if player {
            let by = y + (row_h - btn) / 2;
            self.prev = Rect::new(self.stack.x, by, btn, btn);
            self.play = Rect::new(self.stack.x + btn + 6, by, btn, btn);
            self.next = Rect::new(self.stack.x + (btn + 6) * 2, by, btn, btn);
            let clock_x = if one_row {
                self.stack.right() - m.clock as i32
            } else {
                self.next.right() + BAR_GAP
            };
            self.clock = Rect::new(clock_x, y, (self.stack.right() - clock_x).max(0), row_h);
            y += row_h;

            let seek_h = (row_h / 3).clamp(5, 10);
            if one_row {
                if bar_right > bar_left {
                    self.seek = Rect::new(
                        bar_left,
                        self.prev.y + (btn - seek_h) / 2,
                        bar_right - bar_left,
                        seek_h,
                    );
                }
            } else {
                self.seek = Rect::new(self.stack.x, y + (row_h - seek_h) / 2, self.stack.w, seek_h);
                y += row_h;
            }
        }
        if corr {
            self.correlation = Rect::new(self.stack.x, y, self.stack.w, row_h);
            y += row_h;
        }
        if bal {
            self.balance = Rect::new(self.stack.x, y, self.stack.w, row_h);
        }
    }
}

/// The text and button widths the transport block's geometry depends on, measured
/// before its width is decided, because that decision needs them.
#[derive(Clone, Copy, Debug, Default)]
struct StackMetrics {
    cap: f32,
    val: f32,
    clock: f32,
    btn: i32,
    buttons: i32,
}

impl StackMetrics {
    fn measure(height: i32, measure: MeasureWidth<'_>, player: bool, meters: i32) -> StackMetrics {
        let mut m = StackMetrics::default();
        let rows = player as i32 + meters;
        let row_h = if rows > 0 { height / rows } else { 0 };
        m.btn = (row_h - 2).clamp(12, 28);
        m.buttons = if player { (m.btn + 6) * 3 - 6 } else { 0 };
        if meters > 0 {
            m.cap = measure("CORR").max(measure("BAL"));
            m.val = measure("+0.00");
        }
        // The widest clock rather than the current one, so the bars don't shift when a
        // track ticks past ten minutes.
        if player {
            m.clock = measure("00:00 / 00:00");
        }
        m
    }
}

/// How many of the nine readouts are switched on.
fn loud_count(s: &Settings) -> i32 {
    [
        s.deck_show_lufs_m,
        s.deck_show_lufs_s,
        s.deck_show_lufs_i,
        s.deck_show_lra,
        s.deck_show_true_peak,
        s.deck_show_crest,
        s.deck_show_overs,
        s.deck_show_bpm,
        s.deck_show_brightness,
    ]
    .into_iter()
    .filter(|&on| on)
    .count() as i32
}

/// The bottom strip: the lanes and the deck.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BandLayout {
    pub band: Rect,
    /// The waveform lane under each pane, lined up with its spectrogram.
    pub wave_a: Rect,
    pub wave_b: Rect,
    pub deck: DeckLayout,
}

impl BandLayout {
    /// How much height the band wants off the bottom of a view `view_height` tall.
    ///
    /// One band carries both the lanes and the deck, so switching the lanes off doesn't
    /// take the deck with them, and a thin waveform doesn't crush it.
    pub fn height_for(s: &Settings, view_height: i32) -> i32 {
        let mut band = wave_height(s, view_height);
        if s.show_center_deck {
            let want = DeckLayout::PREFERRED_HEIGHT.max(s.deck_height_px);
            band = band.max((view_height / 3).min(want));
        }
        // A docked panel is a few hundred pixels tall, and a deck that leaves no
        // spectrogram is not a trade anyone wants.
        band.min((view_height - 60).max(0))
    }

    /// Places the lanes and the deck in `view`. Call after the panes are laid out: the
    /// lanes line up with the spectrograms and the deck fills the gap between them.
    pub fn new(
        view: Rect,
        band_h: i32,
        s: &Settings,
        panes: &[PaneLayout],
        measure: MeasureWidth<'_>,
    ) -> BandLayout {
        let mut b = BandLayout::default();
        if band_h <= 0 {
            return b;
        }
        b.band = Rect::new(view.x, view.bottom() - band_h, view.w, band_h);

        let wave_h = band_h.min(wave_height(s, view.h));
        let wave_top = view.bottom() - wave_h;
        let lanes = s.show_waveform && wave_h > 0;
        if lanes && let Some(p) = panes.first() {
            b.wave_a = Rect::new(p.spectro.x, wave_top, p.spectro.w, wave_h);
        }
        if lanes && let Some(p) = panes.get(1) {
            b.wave_b = Rect::new(p.spectro.x, wave_top, p.spectro.w, wave_h);
        }

        // The deck takes the gap the lanes leave in the middle: the two graph strips
        // plus the gutter. Measured from the panes rather than the lanes, so it's there
        // even when the lanes are off. With one pane it gets a centred slot instead.
        let mut deck = Rect::EMPTY;
        if s.show_center_deck {
            if let (Some(a), Some(c)) = (panes.first(), panes.get(1)) {
                let gl = a.spectro.right().min(c.spectro.right());
                let gr = a.spectro.x.max(c.spectro.x);
                if gr > gl {
                    deck = Rect::new(gl, b.band.y, gr - gl, band_h);
                }
            }
            if deck.w == 0 {
                let dw = 700.min(view.w - 40);
                if dw > 0 {
                    deck = Rect::new(view.x + (view.w - dw) / 2, b.band.y, dw, band_h);
                }
            }
        }
        b.deck = DeckLayout::new(deck, s, measure);
        b
    }
}

fn wave_height(s: &Settings, view_height: i32) -> i32 {
    if !s.show_waveform || s.wave_height_pct <= 0 {
        return 0;
    }
    (view_height * s.wave_height_pct.min(40) / 100).max(24)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Segoe UI at 7 pt, as GDI+ measured the strings the deck asks about when the
    /// reference rectangles were exported.
    fn segoe_7pt(text: &str) -> f32 {
        match text {
            "CORR" | "BAL" => 27.0,
            "+0.00" => 27.0,
            "00:00 / 00:00" => 57.0,
            _ => text.len() as f32 * 6.0,
        }
    }

    #[test]
    fn a_wide_gap_fits_everything_without_overlaps() {
        let s = Settings::default();
        let d = DeckLayout::new(Rect::new(0, 0, 1200, 130), &s, &mut segoe_7pt);
        assert!(d.art.w > 0 && d.info.w > 0 && d.goniometer.w > 0);
        assert!(d.stack.w > 0 && d.loudness.w > 0);
        assert!(d.art.right() <= d.info.x);
        assert!(d.info.right() <= d.goniometer.x);
        assert!(d.goniometer.right() <= d.stack.x);
        assert!(d.stack.right() <= d.loudness.x);
        // The goniometer is the axis the display mirrors about: centred, square, with
        // equal halves either side.
        let centre = d.goniometer.x + d.goniometer.w / 2;
        assert!((centre - 600).abs() <= 1);
        assert_eq!(d.goniometer.w, d.goniometer.h);
        assert!((d.goniometer.x - (1200 - d.goniometer.right())).abs() <= 2);
        // All three bars share one column.
        assert!(d.bar.w > 0);
        assert_eq!(d.seek.x, d.bar.x);
        assert_eq!(d.seek.right(), d.bar.right());
        assert_eq!(d.correlation.w, d.balance.w);
        assert!(d.correlation.bottom() <= d.balance.y);
    }

    #[test]
    fn the_readout_grid_fills_columns_before_adding_them() {
        let mut s = Settings {
            deck_show_track_info: false,
            ..Settings::default()
        };
        for on in [
            &mut s.deck_show_lufs_m,
            &mut s.deck_show_lufs_s,
            &mut s.deck_show_lufs_i,
            &mut s.deck_show_lra,
            &mut s.deck_show_true_peak,
            &mut s.deck_show_crest,
            &mut s.deck_show_overs,
            &mut s.deck_show_bpm,
            &mut s.deck_show_brightness,
        ] {
            *on = false;
        }
        let gap = Rect::new(0, 0, 1200, 130);
        let off = DeckLayout::new(gap, &s, &mut segoe_7pt);
        assert_eq!((off.info.w, off.loudness.w), (0, 0));
        assert!(off.goniometer.w > 0);

        s.deck_show_lufs_m = true;
        s.deck_show_true_peak = true;
        let two = DeckLayout::new(gap, &s, &mut segoe_7pt).loudness.w;
        s.deck_show_lufs_s = true;
        s.deck_show_crest = true;
        let four = DeckLayout::new(gap, &s, &mut segoe_7pt).loudness.w;
        assert!(two > 0 && two * 2 == four, "{two} then {four}");
    }

    #[test]
    fn a_taller_deck_spends_height_instead_of_width() {
        let s = Settings::default();
        let tall = DeckLayout::new(Rect::new(0, 0, 1200, 130), &s, &mut segoe_7pt);
        let short = DeckLayout::new(Rect::new(0, 0, 1200, 84), &s, &mut segoe_7pt);
        assert!(short.loudness.w > tall.loudness.w);
        assert_eq!(tall.loud_rows, 3);
        assert_eq!(short.loud_rows, 2);
    }

    #[test]
    fn the_band_holds_both_the_lanes_and_the_deck() {
        let s = Settings::default();
        // The deck's height wins over a thin waveform.
        let h = BandLayout::height_for(&s, 900);
        assert_eq!(h, s.deck_height_px.max(DeckLayout::PREFERRED_HEIGHT));
        // And on a short view neither takes more than a third of it.
        assert_eq!(BandLayout::height_for(&s, 100), 33);
    }
}
