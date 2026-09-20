//! The quick bar: the switches worth reaching without opening a menu.
//!
//! Nostalgia+ put a strip of buttons over the top of the image, with `Reserve` taking
//! the height off the view before the panes were laid out and `HeightFor` deciding how
//! much that was. Both carry over, and the buttons themselves are [`Action`]s from the
//! menu model, so pressing one and picking the same thing out of the menu are the same
//! act and a button can't come to mean something its menu item doesn't.

use sonorant_core::menu::Action;
use sonorant_core::settings::{Flag, Settings};

use crate::band::MeasureWidth;
use crate::colour::{Rgba, pick_keep_alpha};
use crate::layout::Rect;
use crate::overlay::{Face, Layer, Overlay};

/// Air inside a button, either side of its text, at 100% scaling.
const PAD: i32 = 8;
/// Air between buttons, at 100% scaling.
const GAP: i32 = 4;

/// One button: what it does, what it says, and whether it is on.
#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    pub rect: Rect,
    /// The short form, shown on its own when the bar is compact.
    pub short: &'static str,
    /// The long form, shown beside the short one when there is room.
    pub long: &'static str,
    /// Whether what it controls is on, for a switch; always false for a command.
    pub on: bool,
    pub action: Action,
}

impl Button {
    /// What the button reads as at this setting.
    pub fn text(&self, compact: bool) -> String {
        if compact || self.long.is_empty() {
            self.short.to_owned()
        } else {
            format!("{}  {}", self.short, self.long)
        }
    }
}

/// The bar and the buttons in it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QuickBar {
    pub bounds: Rect,
    pub buttons: Vec<Button>,
}

impl QuickBar {
    /// How tall the bar is for `s` at `text_px`, or zero when it is switched off.
    pub fn height_for(s: &Settings, text_px: f32) -> i32 {
        if !s.flag(Flag::ShowQuickButtons) || !s.flag(Flag::ShowOsd) {
            return 0;
        }
        let pad = if s.flag(Flag::QuickBarCompact) {
            6.0
        } else {
            9.0
        };
        (text_px + pad * 2.0).round() as i32
    }

    /// Takes the bar's height off the top of `view` and hands back both parts.
    ///
    /// The bar is reserved rather than drawn over the panes, so nothing it covers is
    /// analysis you wanted to see, and the pointer over a button is never also over a
    /// row of the image.
    pub fn reserve(view: Rect, s: &Settings, text_px: f32) -> (Rect, Rect) {
        let h = QuickBar::height_for(s, text_px);
        // On a short panel the buttons are worth less than the image they would cost.
        if h <= 0 || h > view.h / 4 {
            return (Rect::EMPTY, view);
        }
        (
            Rect::new(view.x, view.y, view.w, h),
            Rect::new(view.x, view.y + h, view.w, view.h - h),
        )
    }

    /// Lays the buttons out in `bounds`.
    ///
    /// With `quick_bar_split` and a gutter to split around, they go in two groups either
    /// side of it, so the frequency axis runs from the top of the window unbroken.
    /// `parked` is whether the image is somewhere back in the history rather than on
    /// now, which puts the way back at the head of the bar.
    pub fn new(
        bounds: Rect,
        gutter: Rect,
        s: &Settings,
        parked: bool,
        scale: f32,
        measure: MeasureWidth<'_>,
    ) -> QuickBar {
        let mut bar = QuickBar {
            bounds,
            buttons: Vec::new(),
        };
        if bounds.is_empty() {
            return bar;
        }
        let px = |n: i32| (n as f32 * scale).round() as i32;
        let (pad, gap) = (px(PAD), px(GAP));
        let compact = s.flag(Flag::QuickBarCompact);
        let mut buttons = buttons(s, parked);
        let widths: Vec<i32> = buttons
            .iter()
            .map(|b| measure(&b.text(compact)).ceil() as i32 + pad * 2)
            .collect();

        let split = s.flag(Flag::QuickBarSplit) && !gutter.is_empty();
        let (left, right) = if split {
            (
                Rect::new(bounds.x, bounds.y, gutter.x - bounds.x, bounds.h),
                Rect::new(
                    gutter.right(),
                    bounds.y,
                    bounds.right() - gutter.right(),
                    bounds.h,
                ),
            )
        } else {
            (bounds, Rect::EMPTY)
        };

        // Half in each group when split, and the left group is pushed up against the
        // gutter so both groups meet in the middle where the eye already is.
        let first = if split {
            buttons.len().div_ceil(2)
        } else {
            buttons.len()
        };
        let left_w: i32 = widths[..first].iter().sum::<i32>() + gap * (first.max(1) as i32 - 1);
        let mut x = if split {
            (left.right() - left_w).max(left.x)
        } else {
            left.x
        };
        for (i, (button, w)) in buttons.iter_mut().zip(&widths).enumerate() {
            if i == first {
                x = right.x;
            }
            let area = if i < first { left } else { right };
            // A button that would hang off its group's end is dropped rather than
            // drawn half over the axis.
            if area.is_empty() || x + w > area.right() {
                button.rect = Rect::EMPTY;
            } else {
                button.rect = Rect::new(x, bounds.y, *w, bounds.h);
            }
            x += w + gap;
        }
        buttons.retain(|b| !b.rect.is_empty());
        bar.buttons = buttons;
        bar
    }

    /// The button under a point, if any.
    pub fn hit(&self, x: i32, y: i32) -> Option<&Button> {
        self.buttons.iter().find(|b| b.rect.contains(x, y))
    }
}

/// What the bar offers: the switches reached most often, plus the two that walk through
/// a list. Each is the same action its menu item performs.
///
/// "Live" is the exception: it is only there while the image is parked back in the
/// history, and then it comes first. A way back matters most when it is needed, and a
/// button that does nothing the rest of the time is taking room from one that does.
fn buttons(s: &Settings, parked: bool) -> Vec<Button> {
    let switch = |short, long, flag: Flag| Button {
        rect: Rect::EMPTY,
        short,
        long,
        on: s.flag(flag),
        action: Action::Toggle(flag),
    };
    let mut out = Vec::new();
    if parked {
        out.push(Button {
            rect: Rect::EMPTY,
            short: "LIVE",
            long: "Live",
            on: true,
            action: Action::GoLive,
        });
    }
    out.extend([
        switch("IMM", "Immersive", Flag::Immersive),
        switch("WAV", "Waveform", Flag::ShowWaveform),
        switch("GRD", "Grid", Flag::ShowGrid),
        switch("PK", "Peaks", Flag::ShowMax),
        switch("HRM", "Harmonics", Flag::ShowHarmonics),
        Button {
            rect: Rect::EMPTY,
            short: "COL",
            long: "Palette",
            on: false,
            action: Action::NextPalette,
        },
        Button {
            rect: Rect::EMPTY,
            short: "CRV",
            long: "Style",
            on: false,
            action: Action::NextStyle,
        },
        switch("DECK", "Deck", Flag::ShowCentreDeck),
    ]);
    out
}

/// Draws the bar. `under` is the button the pointer is over, if any.
pub fn draw(
    o: &mut Overlay,
    bar: &QuickBar,
    s: &Settings,
    under: Option<&Rect>,
    alpha: f64,
    text_px: f32,
) {
    if bar.bounds.is_empty() || alpha <= 0.004 {
        return;
    }
    let t = &s.theme;
    let ground = pick_keep_alpha(t.panel, Rgba::argb(190, 12, 12, 15)).faded(alpha);
    let ink = pick_keep_alpha(t.axis_text, Rgba::argb(210, 214, 214, 222)).faded(alpha);
    let lit = pick_keep_alpha(t.hover, Rgba::argb(235, 255, 214, 120)).faded(alpha);
    let compact = s.flag(Flag::QuickBarCompact);

    for button in &bar.buttons {
        let r = button.rect;
        let hovered = under == Some(&r);
        let face = if button.on {
            lit.faded(0.22)
        } else if hovered {
            ground.faded(1.6)
        } else {
            ground
        };
        o.rect(
            Layer::Top,
            r.x as f32,
            r.y as f32,
            r.w as f32,
            r.h as f32,
            face,
        );
        if button.on || hovered {
            o.outline(
                Layer::Top,
                r.x as f32,
                r.y as f32,
                r.w as f32,
                r.h as f32,
                if button.on { lit } else { ink.faded(0.6) },
            );
        }
        let text = button.text(compact);
        let sz = o.measure(&text, Face::Sans, text_px);
        o.text(
            Layer::Top,
            &text,
            Face::Sans,
            text_px,
            r.x as f32 + (r.w as f32 - sz.w) / 2.0,
            r.y as f32 + (r.h as f32 - sz.h) / 2.0,
            if button.on { lit } else { ink },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for the real font: every character the same width, so the tests are
    /// about the layout rather than the glyphs.
    fn measure(text: &str) -> f32 {
        text.chars().count() as f32 * 7.0
    }

    fn settings() -> Settings {
        Settings::default()
    }

    #[test]
    fn the_bar_is_reserved_off_the_top() {
        let s = settings();
        let view = Rect::new(0, 0, 1200, 600);
        let (bar, rest) = QuickBar::reserve(view, &s, 12.0);
        assert!(bar.h > 0);
        assert_eq!(bar, Rect::new(0, 0, 1200, bar.h));
        assert_eq!(rest, Rect::new(0, bar.h, 1200, 600 - bar.h));
        assert_eq!(bar.h, QuickBar::height_for(&s, 12.0));
    }

    #[test]
    fn a_bar_nobody_asked_for_costs_nothing() {
        let view = Rect::new(0, 0, 1200, 600);
        let mut s = settings();
        s.set_flag(Flag::ShowQuickButtons, false);
        assert_eq!(QuickBar::height_for(&s, 12.0), 0);
        assert_eq!(QuickBar::reserve(view, &s, 12.0), (Rect::EMPTY, view));

        // The on-screen readouts are a master switch over the bar as well.
        let mut s = settings();
        s.set_flag(Flag::ShowOsd, false);
        assert_eq!(QuickBar::height_for(&s, 12.0), 0);
    }

    #[test]
    fn a_short_view_keeps_its_image() {
        let s = settings();
        // A quarter of a 60 px view is 15 px, less than the bar wants.
        let view = Rect::new(0, 0, 1200, 60);
        assert_eq!(QuickBar::reserve(view, &s, 12.0), (Rect::EMPTY, view));
    }

    #[test]
    fn compact_is_shorter_and_narrower() {
        let mut s = settings();
        let tall = QuickBar::height_for(&s, 12.0);
        s.set_flag(Flag::QuickBarCompact, true);
        assert!(QuickBar::height_for(&s, 12.0) < tall);

        let bounds = Rect::new(0, 0, 1200, 30);
        let wide = QuickBar::new(bounds, Rect::EMPTY, &settings(), false, 1.0, &mut measure);
        let thin = QuickBar::new(bounds, Rect::EMPTY, &s, false, 1.0, &mut measure);
        let width = |b: &QuickBar| b.buttons.iter().map(|x| x.rect.w).sum::<i32>();
        assert!(width(&thin) < width(&wide));
    }

    #[test]
    fn buttons_sit_in_a_row_without_overlapping() {
        let s = Settings {
            quick_bar_split: false,
            ..settings()
        };
        let bounds = Rect::new(10, 4, 1200, 30);
        let bar = QuickBar::new(bounds, Rect::EMPTY, &s, false, 1.0, &mut measure);
        assert!(!bar.buttons.is_empty());
        for pair in bar.buttons.windows(2) {
            assert!(
                pair[0].rect.right() <= pair[1].rect.x,
                "{:?} runs into {:?}",
                pair[0].rect,
                pair[1].rect
            );
        }
        for b in &bar.buttons {
            assert_eq!(b.rect.y, bounds.y);
            assert_eq!(b.rect.h, bounds.h);
            assert!(b.rect.x >= bounds.x && b.rect.right() <= bounds.right());
        }
    }

    #[test]
    fn splitting_leaves_the_gutter_clear() {
        let s = settings();
        assert!(s.quick_bar_split);
        let bounds = Rect::new(0, 0, 1200, 30);
        let gutter = Rect::new(580, 0, 40, 30);
        let bar = QuickBar::new(bounds, gutter, &s, false, 1.0, &mut measure);
        assert!(!bar.buttons.is_empty());
        for b in &bar.buttons {
            assert!(
                b.rect.right() <= gutter.x || b.rect.x >= gutter.right(),
                "{:?} crosses the gutter",
                b.rect
            );
        }
        // Both sides are used, and the left group ends against the gutter.
        let left: Vec<&Button> = bar.buttons.iter().filter(|b| b.rect.x < gutter.x).collect();
        let right: Vec<&Button> = bar
            .buttons
            .iter()
            .filter(|b| b.rect.x >= gutter.right())
            .collect();
        assert!(!left.is_empty() && !right.is_empty());
        assert_eq!(left.last().unwrap().rect.right(), gutter.x);
        assert_eq!(right[0].rect.x, gutter.right());
    }

    #[test]
    fn a_narrow_bar_drops_what_will_not_fit() {
        let s = Settings {
            quick_bar_split: false,
            ..settings()
        };
        let all = QuickBar::new(
            Rect::new(0, 0, 1200, 30),
            Rect::EMPTY,
            &s,
            false,
            1.0,
            &mut measure,
        );
        let few = QuickBar::new(
            Rect::new(0, 0, 200, 30),
            Rect::EMPTY,
            &s,
            false,
            1.0,
            &mut measure,
        );
        assert!(few.buttons.len() < all.buttons.len());
        assert!(!few.buttons.is_empty());
        for b in &few.buttons {
            assert!(b.rect.right() <= 200);
        }
    }

    /// The way back only takes room on the bar while there is somewhere to come back
    /// from, and then it takes the first place.
    #[test]
    fn the_live_button_is_there_only_while_the_image_is_parked() {
        let bounds = Rect::new(0, 0, 1200, 30);
        let s = settings();
        let live = QuickBar::new(bounds, Rect::EMPTY, &s, true, 1.0, &mut measure);
        assert_eq!(live.buttons[0].short, "LIVE");
        assert_eq!(live.buttons[0].action, Action::GoLive);
        assert!(live.buttons[0].on, "the way back reads as lit, not as off");

        let normal = QuickBar::new(bounds, Rect::EMPTY, &s, false, 1.0, &mut measure);
        assert!(normal.buttons.iter().all(|b| b.short != "LIVE"));
        assert_eq!(live.buttons.len(), normal.buttons.len() + 1);
    }

    #[test]
    fn a_button_says_what_its_switch_says() {
        let mut s = settings();
        s.set_flag(Flag::ShowGrid, false);
        s.set_flag(Flag::Immersive, true);
        let bar = QuickBar::new(
            Rect::new(0, 0, 1200, 30),
            Rect::EMPTY,
            &s,
            false,
            1.0,
            &mut measure,
        );
        let grid = bar.buttons.iter().find(|b| b.short == "GRD").unwrap();
        assert!(!grid.on);
        assert_eq!(grid.action, Action::Toggle(Flag::ShowGrid));
        let imm = bar.buttons.iter().find(|b| b.short == "IMM").unwrap();
        assert!(imm.on);
    }

    #[test]
    fn the_pointer_finds_the_button_under_it() {
        let s = settings();
        let bar = QuickBar::new(
            Rect::new(0, 0, 1200, 30),
            Rect::EMPTY,
            &s,
            false,
            1.0,
            &mut measure,
        );
        let first = bar.buttons[0].clone();
        let at = (first.rect.x + first.rect.w / 2, first.rect.y + 15);
        assert_eq!(bar.hit(at.0, at.1).map(|b| b.short), Some(first.short));
        assert!(bar.hit(first.rect.x, 400).is_none());
    }
}
