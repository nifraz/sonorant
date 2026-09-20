//! Drawing the bottom band: the waveform lanes, the centre deck's goniometer,
//! transport, meters and readouts, and the colour bar and status line beside them.
//!
//! The geometry is [`crate::band`]; this draws into the [`Overlay`], so everything here
//! blends as GDI+ did. Ported from Nostalgia+'s `BottomBand`, `CenterDeck` and
//! `AnalyzerPanel`.

use sonorant_core::dsp::LoudnessReadings;
use sonorant_core::palette::{self, Lut};
use sonorant_core::settings::Settings;

use crate::band::{BandLayout, DeckLayout};
use crate::colour::{Rgba, pick, pick_keep_alpha};
use crate::layout::{PaneLayout, Rect};
use crate::overlay::{Face, Layer, Overlay};

/// What's playing, as the deck shows it. Phase 4 fills this from the media session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub composer: String,
    pub year: String,
}

impl TrackInfo {
    pub fn is_empty(&self) -> bool {
        *self == TrackInfo::default()
    }
}

/// Everything the deck reads each frame.
#[derive(Clone, Copy, Debug)]
pub struct DeckState<'a> {
    pub loudness: &'a LoudnessReadings,
    pub bpm: f64,
    /// The spectral centroid as a frequency.
    pub brightness_hz: f64,
    /// The newest audio, for the goniometer.
    pub scope_left: &'a [f32],
    pub scope_right: &'a [f32],
    pub track: &'a TrackInfo,
    /// Position and length in seconds, when a player is reporting them.
    pub position: Option<(f64, f64)>,
    pub playing: bool,
}

/// The waveform lanes' history: one pair of extremes per spectrogram row, so a
/// transient lines up with the column that produced it whatever the scroll speed.
#[derive(Clone, Debug, Default)]
pub struct WaveRing {
    /// Left min, left max, right min, right max, newest last.
    rows: std::collections::VecDeque<[f32; 4]>,
    capacity: usize,
}

impl WaveRing {
    pub fn new(capacity: usize) -> WaveRing {
        WaveRing {
            rows: std::collections::VecDeque::with_capacity(capacity.max(1)),
            capacity: capacity.max(1),
        }
    }

    /// Keeps room for `columns` columns, the widest lane on screen.
    pub fn resize(&mut self, columns: usize) {
        self.capacity = columns.max(8) + 4;
        while self.rows.len() > self.capacity {
            self.rows.pop_front();
        }
    }

    pub fn push(&mut self, wave: [f32; 4]) {
        if self.rows.len() == self.capacity {
            self.rows.pop_front();
        }
        self.rows.push_back(wave);
    }

    pub fn clear(&mut self) {
        self.rows.clear();
    }

    /// The extremes `age` rows back from the newest, or silence beyond the history.
    pub fn at(&self, age: usize) -> [f32; 4] {
        if age >= self.rows.len() {
            return [0.0; 4];
        }
        self.rows[self.rows.len() - 1 - age]
    }
}

/// Draws the lanes and the deck.
#[allow(clippy::too_many_arguments)]
pub fn draw_band(
    o: &mut Overlay,
    band: &BandLayout,
    panes: &[PaneLayout],
    s: &Settings,
    lut: &Lut,
    waves: &WaveRing,
    state: &DeckState<'_>,
    alpha: f64,
    px: f32,
) {
    if alpha <= 0.004 {
        return;
    }
    if s.show_waveform && band.wave_a.h > 0 {
        waveforms(o, band, panes, s, lut, waves, alpha);
    }
    if s.show_center_deck {
        deck(o, &band.deck, s, lut, state, alpha, px);
    }
}

fn waveforms(
    o: &mut Overlay,
    band: &BandLayout,
    panes: &[PaneLayout],
    s: &Settings,
    lut: &Lut,
    waves: &WaveRing,
    alpha: f64,
) {
    let lane = band.wave_a;
    let ground = pick_keep_alpha(s.theme.panel, Rgba::argb(255, 10, 10, 12)).faded(alpha);
    o.rect(
        Layer::Over,
        band.band.x as f32,
        lane.y as f32,
        band.band.w as f32,
        lane.h as f32,
        ground,
    );
    let ink = pick(
        s.theme.waveform,
        Rgba::rgb(palette::color_at(lut, 0.86), 255),
    )
    .with_alpha(215)
    .faded(alpha);
    let mid_y = lane.y + lane.h / 2;
    let half = lane.h as f32 / 2.0 - 2.0;
    o.hline(
        Layer::Over,
        band.band.x as f32,
        band.band.right() as f32,
        mid_y as f32,
        Rgba::argb(45, 255, 255, 255).faded(alpha),
    );
    for (i, rect) in [band.wave_a, band.wave_b].into_iter().enumerate() {
        let Some(pane) = panes.get(i) else { continue };
        if rect.w <= 0 {
            continue;
        }
        // Match the spectrogram above: the newest column sits against that pane's curve.
        for age in 0..rect.w {
            let x = if pane.curve_on_left {
                rect.x + age
            } else {
                rect.right() - 1 - age
            };
            let w = waves.at(age as usize);
            let (lo, hi) = if i == 0 { (w[0], w[1]) } else { (w[2], w[3]) };
            let top = mid_y as f32 - hi * half;
            let bottom = mid_y as f32 - lo * half;
            o.rect(
                Layer::Over,
                x as f32,
                top,
                1.0,
                (bottom - top).max(1.0),
                ink,
            );
        }
    }
}

fn deck(
    o: &mut Overlay,
    d: &DeckLayout,
    s: &Settings,
    lut: &Lut,
    state: &DeckState<'_>,
    alpha: f64,
    px: f32,
) {
    if d.bounds.w <= 0 {
        return;
    }
    if d.art.w > 0 {
        artwork(o, d.art, alpha);
    }
    if d.info.w > 0 {
        track_info(o, d.info, state.track, alpha, px);
    }
    if d.goniometer.w > 0 {
        goniometer(o, d.goniometer, lut, state, alpha);
    }
    if d.play.w > 0 || d.seek.w > 0 {
        transport(o, d, state, alpha, px);
    }
    if d.correlation.w > 0 || d.balance.w > 0 {
        meters(o, d, state.loudness, alpha, px);
    }
    if d.loudness.w > 0 {
        readouts(o, d, s, state, alpha, px);
    }
}

/// A placeholder until Phase 4 brings artwork in: the frame it will sit in.
fn artwork(o: &mut Overlay, r: Rect, alpha: f64) {
    o.rect(
        Layer::Over,
        r.x as f32,
        r.y as f32,
        r.w as f32,
        r.h as f32,
        Rgba::argb(120, 20, 20, 26).faded(alpha),
    );
    o.outline(
        Layer::Over,
        r.x as f32,
        r.y as f32,
        r.w as f32,
        r.h as f32,
        Rgba::argb(60, 255, 255, 255).faded(alpha),
    );
}

/// What's playing, as much of it as the block is tall enough to hold: the title, then
/// four labelled lines, or two joined pairs, or one. The captions earn their width only
/// at the tall sizes.
fn track_info(o: &mut Overlay, r: Rect, track: &TrackInfo, alpha: f64, px: f32) {
    let mid_px = px + 1.5 * 96.0 / 72.0;
    let title_h = if track.title.is_empty() {
        0.0
    } else {
        o.measure("Mg", Face::Sans, mid_px).h
    };
    let line_h = o.measure("Mg", Face::Sans, px).h;
    if line_h <= 0.0 {
        return;
    }
    let room = ((r.h as f32 - title_h) / line_h) as i32;
    let join = |a: &str, b: &str| match (a.is_empty(), b.is_empty()) {
        (false, false) => format!("{a}  ·  {b}"),
        (false, true) => a.to_owned(),
        (true, false) => b.to_owned(),
        (true, true) => String::new(),
    };
    let mut rows: Vec<(Option<&str>, String)> = Vec::new();
    if room >= 4 {
        rows.push((Some("COMPOSER"), track.composer.clone()));
        rows.push((Some("ARTISTS"), track.artist.clone()));
        rows.push((Some("ALBUM"), track.album.clone()));
        rows.push((Some("YEAR"), track.year.clone()));
    } else if room >= 2 {
        rows.push((None, join(&track.composer, &track.artist)));
        rows.push((None, join(&track.album, &track.year)));
    } else if room >= 1 {
        rows.push((None, join(&track.artist, &track.album)));
    }
    rows.retain(|(_, v)| !v.is_empty());
    if track.title.is_empty() && rows.is_empty() {
        return;
    }

    let mut cap_w: f32 = 0.0;
    for (cap, _) in &rows {
        if let Some(c) = cap {
            cap_w = cap_w.max(o.measure(c, Face::Sans, px).w);
        }
    }
    if cap_w > 0.0 {
        cap_w += 10.0;
    }
    let ink = Rgba::argb(240, 245, 245, 248).faded(alpha);
    let dim = Rgba::argb(180, 205, 205, 215).faded(alpha);
    let cap_ink = Rgba::argb(130, 175, 175, 188).faded(alpha);
    let block = title_h + rows.len() as f32 * line_h;
    let mut y = r.y as f32 + ((r.h as f32 - block) / 2.0).max(0.0);
    let clip = [r.x as f32, r.y as f32, r.w as f32, r.h as f32];
    if !track.title.is_empty() {
        o.text_clipped(
            Layer::Over,
            &track.title,
            Face::Sans,
            mid_px,
            r.x as f32,
            y,
            ink,
            Some(clip),
        );
        y += title_h;
    }
    for (cap, value) in &rows {
        if let Some(c) = cap {
            o.text(Layer::Over, c, Face::Sans, px, r.x as f32, y, cap_ink);
        }
        o.text_clipped(
            Layer::Over,
            value,
            Face::Sans,
            px,
            r.x as f32 + cap_w,
            y,
            dim,
            Some(clip),
        );
        y += line_h;
    }
}

/// Lissajous plot of the two channels, rotated 45 degrees so mono reads as a vertical
/// line: a circle is a wide image, a horizontal line is out of phase, and a lean to one
/// side is a level imbalance.
fn goniometer(o: &mut Overlay, r: Rect, lut: &Lut, state: &DeckState<'_>, alpha: f64) {
    let (x, y, w, h) = (r.x as f32, r.y as f32, r.w as f32, r.h as f32);
    o.rect(
        Layer::Over,
        x,
        y,
        w,
        h,
        Rgba::argb(200, 10, 10, 13).faded(alpha),
    );
    o.outline(
        Layer::Over,
        x,
        y,
        w,
        h,
        Rgba::argb(50, 255, 255, 255).faded(alpha),
    );
    // The diagonals are the left-only and right-only axes; the vertical is mono.
    let guide = Rgba::argb(28, 255, 255, 255).faded(alpha);
    o.line(Layer::Over, x, y, x + w, y + h, 1.0, guide);
    o.line(Layer::Over, x + w, y, x, y + h, 1.0, guide);
    o.vline(Layer::Over, x + (r.w / 2) as f32, y, y + h, guide);

    let n = state.scope_left.len().min(state.scope_right.len());
    if n < 2 {
        return;
    }
    // The newest few milliseconds, decimated to a trace of a consistent length.
    const POINTS: usize = 1024;
    let take = n.min(POINTS);
    let (l, rr) = (
        &state.scope_left[n - take..],
        &state.scope_right[n - take..],
    );
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let rad = w.min(h) / 2.0 - 3.0;
    // 1/sqrt(2) so a hard-panned full-scale signal reaches the frame rather than
    // spilling out of it.
    const K: f32 = std::f32::consts::FRAC_1_SQRT_2;
    let trace = Rgba::rgb(palette::color_at(lut, 0.80), 150).faded(alpha);
    let point = |i: usize| (cx + (l[i] - rr[i]) * K * rad, cy - (l[i] + rr[i]) * K * rad);
    let mut prev = point(0);
    for i in 1..take {
        let p = point(i);
        // Inside the frame: a loud transient would otherwise draw over the deck.
        if p.0 >= x && p.0 <= x + w && p.1 >= y && p.1 <= y + h {
            o.line(Layer::Over, prev.0, prev.1, p.0, p.1, 1.0, trace);
        }
        prev = p;
    }
}

/// Controls, seek bar and clock on one row, the bar in the column it shares with the
/// meters below.
fn transport(o: &mut Overlay, d: &DeckLayout, state: &DeckState<'_>, alpha: f64, px: f32) {
    let ink = Rgba::argb(215, 235, 235, 242).faded(alpha);
    let dim = Rgba::argb(140, 185, 185, 195).faded(alpha);
    let edge = Rgba::argb(70, 255, 255, 255).faded(alpha);
    if d.prev.w > 0 {
        previous(o, d.prev, ink);
        if state.playing {
            pause(o, d.play, ink);
        } else {
            play(o, d.play, ink);
        }
        next(o, d.next, ink);
    }
    let (pos, dur) = state.position.unwrap_or((0.0, 0.0));
    let text = format!(
        "{} / {}",
        clock(pos),
        if dur > 0.0 {
            clock(dur)
        } else {
            "--:--".to_owned()
        }
    );
    let sz = o.measure(&text, Face::Sans, px);
    if d.clock.w as f32 >= sz.w {
        o.text(
            Layer::Over,
            &text,
            Face::Sans,
            px,
            d.clock.right() as f32 - sz.w,
            d.clock.y as f32 + (d.clock.h as f32 - sz.h) / 2.0,
            dim,
        );
    }
    if d.seek.w <= 0 {
        return;
    }
    let (sx, sy, sw, sh) = (
        d.seek.x as f32,
        d.seek.y as f32,
        d.seek.w as f32,
        d.seek.h as f32,
    );
    o.outline(Layer::Over, sx, sy, sw, sh, edge);
    if dur > 0.0 {
        let t = (pos / dur).clamp(0.0, 1.0) as f32;
        let w = (sw - 2.0) * t;
        if w > 0.0 {
            o.rect(
                Layer::Over,
                sx + 1.0,
                sy + 1.0,
                w,
                sh - 1.0,
                Rgba::argb(200, 235, 235, 242).faded(alpha),
            );
        }
    }
}

fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0) as i64;
    format!("{}:{:02}", total / 60, total % 60)
}

fn play(o: &mut Overlay, r: Rect, ink: Rgba) {
    let i = (r.h / 4) as f32;
    o.triangle(
        Layer::Over,
        [
            (r.x as f32 + i, r.y as f32 + i),
            (r.right() as f32 - i, r.y as f32 + (r.h / 2) as f32),
            (r.x as f32 + i, r.bottom() as f32 - i),
        ],
        ink,
    );
}

fn pause(o: &mut Overlay, r: Rect, ink: Rgba) {
    let i = (r.h / 4) as f32;
    let w = (r.w / 7).max(2) as f32;
    let h = r.h as f32 - i * 2.0;
    o.rect(Layer::Over, r.x as f32 + i, r.y as f32 + i, w, h, ink);
    o.rect(
        Layer::Over,
        r.right() as f32 - i - w,
        r.y as f32 + i,
        w,
        h,
        ink,
    );
}

fn next(o: &mut Overlay, r: Rect, ink: Rgba) {
    let i = (r.h / 4) as f32;
    let w = (r.w / 8).max(2) as f32;
    o.triangle(
        Layer::Over,
        [
            (r.x as f32 + i, r.y as f32 + i),
            (r.right() as f32 - i - w, r.y as f32 + (r.h / 2) as f32),
            (r.x as f32 + i, r.bottom() as f32 - i),
        ],
        ink,
    );
    o.rect(
        Layer::Over,
        r.right() as f32 - i - w,
        r.y as f32 + i,
        w,
        r.h as f32 - i * 2.0,
        ink,
    );
}

fn previous(o: &mut Overlay, r: Rect, ink: Rgba) {
    let i = (r.h / 4) as f32;
    let w = (r.w / 8).max(2) as f32;
    o.triangle(
        Layer::Over,
        [
            (r.right() as f32 - i, r.y as f32 + i),
            (r.x as f32 + i + w, r.y as f32 + (r.h / 2) as f32),
            (r.right() as f32 - i, r.bottom() as f32 - i),
        ],
        ink,
    );
    o.rect(
        Layer::Over,
        r.x as f32 + i,
        r.y as f32 + i,
        w,
        r.h as f32 - i * 2.0,
        ink,
    );
}

/// Correlation and balance: they describe the pair of channels, which is why they live
/// in the deck rather than over one pane.
fn meters(o: &mut Overlay, d: &DeckLayout, l: &LoudnessReadings, alpha: f64, px: f32) {
    let corr_fill = if l.correlation < 0.0 {
        Rgba::argb(255, 255, 120, 90)
    } else {
        Rgba::argb(255, 120, 220, 160)
    };
    bipolar(
        o,
        d.correlation,
        d.bar,
        "CORR",
        l.correlation,
        corr_fill,
        alpha,
        px,
    );
    bipolar(
        o,
        d.balance,
        d.bar,
        "BAL",
        l.balance,
        Rgba::argb(255, 150, 190, 240),
        alpha,
        px,
    );
}

/// One meter on one line: name, a centre-zero bar in the shared column, then the value.
#[allow(clippy::too_many_arguments)]
fn bipolar(
    o: &mut Overlay,
    r: Rect,
    bar: Rect,
    label: &str,
    value: f64,
    fill: Rgba,
    alpha: f64,
    px: f32,
) {
    if r.w <= 0 || r.h <= 0 {
        return;
    }
    let text = signed(value);
    let lbl = Rgba::argb(135, 180, 180, 190).faded(alpha);
    let val = Rgba::argb(225, 240, 240, 246).faded(alpha);
    let edge = Rgba::argb(55, 255, 255, 255).faded(alpha);
    let ls = o.measure(label, Face::Sans, px);
    let vs = o.measure(&text, Face::Sans, px);
    o.text(
        Layer::Over,
        label,
        Face::Sans,
        px,
        r.x as f32,
        r.y as f32 + (r.h as f32 - ls.h) / 2.0,
        lbl,
    );
    o.text(
        Layer::Over,
        &text,
        Face::Sans,
        px,
        r.right() as f32 - vs.w,
        r.y as f32 + (r.h as f32 - vs.h) / 2.0,
        val,
    );

    if bar.w < 24 {
        return;
    }
    let bar_h = (r.h - 6).clamp(4, 9);
    let bar_y = r.y + (r.h - bar_h) / 2;
    o.outline(
        Layer::Over,
        bar.x as f32,
        bar_y as f32,
        bar.w as f32,
        bar_h as f32,
        edge,
    );
    let t = value.clamp(-1.0, 1.0);
    let mid = bar.x + bar.w / 2;
    let end = mid + (t * (bar.w / 2 - 1) as f64) as i32;
    let fill = fill.faded(alpha);
    if end < mid {
        o.rect(
            Layer::Over,
            end as f32,
            bar_y as f32 + 1.0,
            (mid - end) as f32,
            (bar_h - 1) as f32,
            fill,
        );
    } else {
        o.rect(
            Layer::Over,
            mid as f32,
            bar_y as f32 + 1.0,
            (end - mid).max(1) as f32,
            (bar_h - 1) as f32,
            fill,
        );
    }
    o.vline(
        Layer::Over,
        mid as f32,
        bar_y as f32,
        (bar_y + bar_h) as f32,
        edge,
    );
}

/// The readouts, two or three rows deep, filling each column before adding one, in
/// priority order so shedding a column gives up the least useful number first.
fn readouts(
    o: &mut Overlay,
    d: &DeckLayout,
    s: &Settings,
    state: &DeckState<'_>,
    alpha: f64,
    px: f32,
) {
    let l = state.loudness;
    let label = Rgba::argb(150, 190, 190, 200).faded(alpha);
    let value = Rgba::argb(240, 245, 245, 248).faded(alpha);
    // Anything above -1 dBTP will clip a lossy encoder even though the samples never
    // did, which is the whole reason true peak is measured.
    let warn = Rgba::argb(255, 255, 120, 90).faded(alpha);
    let mut cells: Vec<(String, String, Rgba)> = Vec::new();
    let mut add = |on: bool, cap: &str, val: String, ink: Rgba| {
        if on {
            cells.push((cap.to_owned(), val, ink));
        }
    };
    add(s.deck_show_lufs_m, "LUFS-M", db(l.momentary), value);
    add(s.deck_show_lufs_s, "LUFS-S", db(l.short_term), value);
    add(s.deck_show_lufs_i, "LUFS-I", db(l.integrated), value);
    add(s.deck_show_lra, "LRA", format!("{:.1}", l.range), value);
    add(
        s.deck_show_true_peak,
        "TRUE PK",
        db(l.true_peak_db),
        if l.true_peak_db > -1.0 { warn } else { value },
    );
    add(
        s.deck_show_crest,
        "CREST",
        format!("{:.1}", l.crest_db),
        value,
    );
    let overs_cap = if l.overs > 0 {
        format!("OVERS {}", clock(l.last_over_seconds))
    } else {
        "OVERS".to_owned()
    };
    add(
        s.deck_show_overs,
        &overs_cap,
        l.overs.to_string(),
        if l.overs > 0 { warn } else { value },
    );
    add(
        s.deck_show_bpm,
        "BPM",
        if state.bpm > 0.0 {
            format!("{:.0}", state.bpm)
        } else {
            "--".to_owned()
        },
        value,
    );
    add(
        s.deck_show_brightness,
        "BRIGHT",
        hz(state.brightness_hz),
        value,
    );

    let rows = d.loud_rows.max(1);
    let cell_h = d.loudness.h / rows;
    let mid_px = px + 1.5 * 96.0 / 72.0;
    let val_h = o.measure("-00.0", Face::Sans, mid_px).h;
    for (i, (cap, val, ink)) in cells.into_iter().enumerate() {
        let (col, row) = (i as i32 / rows, i as i32 % rows);
        if col >= d.loud_cols {
            break;
        }
        let x = (d.loudness.x + col * d.loud_col_w) as f32;
        let y = (d.loudness.y + row * cell_h) as f32;
        // A caption too wide for its column gives up its tail rather than running into
        // the next one: "OVERS 2:14" falls back to "OVERS".
        let mut cap = cap;
        let mut ls = o.measure(&cap, Face::Sans, px);
        if ls.w > (d.loud_col_w - 4) as f32
            && let Some(cut) = cap.rfind(' ')
        {
            cap.truncate(cut);
            ls = o.measure(&cap, Face::Sans, px);
        }
        let top = y + ((cell_h as f32 - (ls.h + val_h)) / 2.0).max(0.0);
        o.text(Layer::Over, &cap, Face::Sans, px, x, top, label);
        o.text(Layer::Over, &val, Face::Sans, mid_px, x, top + ls.h, ink);
    }
}

/// A signed value with a space where the sign would be when it's zero, so the number
/// doesn't jog sideways as it crosses.
fn signed(v: f64) -> String {
    if v > 0.0 {
        format!("+{v:.2}")
    } else if v < 0.0 {
        format!("-{:.2}", -v)
    } else {
        " 0.00".to_owned()
    }
}

/// A level, or a dash when the meter hasn't settled on one yet.
fn db(v: f64) -> String {
    if v <= -70.0 {
        "--".to_owned()
    } else {
        format!("{v:.1}")
    }
}

/// Short enough for a narrow column: 440, 2.4k, 14k.
fn hz(f: f64) -> String {
    if f <= 0.0 {
        return "--".to_owned();
    }
    if f < 1000.0 {
        return format!("{f:.0}");
    }
    let k = f / 1000.0;
    if k < 10.0 {
        format!("{k:.1}k")
    } else {
        format!("{k:.0}k")
    }
}

/// The colour bar down the edge: the palette's ramp with the range's ends beside it.
#[allow(clippy::too_many_arguments)]
pub fn draw_colour_bar(
    o: &mut Overlay,
    r: Rect,
    s: &Settings,
    lut: &Lut,
    floor_db: f64,
    ceiling_db: f64,
    alpha: f64,
    px: f32,
) {
    if r.w <= 0 || alpha <= 0.004 {
        return;
    }
    let ground = pick_keep_alpha(s.theme.panel, Rgba::argb(255, 14, 14, 16)).faded(alpha);
    o.rect(
        Layer::Over,
        r.x as f32,
        r.y as f32,
        r.w as f32,
        r.h as f32,
        ground,
    );
    let (bar_x, bar_w) = (r.x + 5, 11);
    let (top, bottom) = (r.y + 12, r.bottom() - 12);
    if bottom <= top {
        return;
    }
    let height = bottom - top;
    for i in 0..height {
        let t = 1.0 - i as f64 / height as f64;
        let c = Rgba::rgb(palette::color_at(lut, t), 255).faded(alpha);
        o.rect(
            Layer::Over,
            bar_x as f32,
            (top + i) as f32,
            bar_w as f32,
            1.0,
            c,
        );
    }
    o.outline(
        Layer::Over,
        bar_x as f32,
        top as f32,
        bar_w as f32,
        height as f32,
        Rgba::argb(60, 255, 255, 255).faded(alpha),
    );
    let ink = Rgba::argb(190, 225, 225, 228).faded(alpha);
    // Centred on the end each value belongs to, like every other scale here.
    let h = o.measure("0", Face::Sans, px).h;
    let x = (bar_x + bar_w + 3) as f32;
    o.text(
        Layer::Over,
        &format!("{ceiling_db:.0}"),
        Face::Sans,
        px,
        x,
        top as f32 - h / 2.0,
        ink,
    );
    o.text(
        Layer::Over,
        &format!("{floor_db:.0}"),
        Face::Sans,
        px,
        x,
        bottom as f32 - h / 2.0,
        ink,
    );
}

/// A flare along the view's edges on each onset: four gradient bars rather than a
/// vignette, because the edges are where the eye catches movement without being pulled
/// off the analysis. The band recedes as well as fades, so a decaying beat costs less
/// to draw and reads as a flare rather than a light being dimmed.
pub fn draw_beat_flare(o: &mut Overlay, client: Rect, lut: &Lut, pulse: f64) {
    if pulse <= 0.02 {
        return;
    }
    let band = ((client.h / 22).clamp(20, 48) as f64 * (0.35 + 0.65 * pulse)) as i32;
    if band < 6 {
        return;
    }
    let alpha = (pulse * 90.0) as u8;
    if alpha < 2 {
        return;
    }
    let hot = Rgba::rgb(palette::color_at(lut, 0.9), alpha);
    let gone = hot.with_alpha(0);
    let (x, y) = (client.x as f32, client.y as f32);
    let (w, h, b) = (client.w as f32, client.h as f32, band as f32);
    o.gradient_v(Layer::Top, x, y, w, b, hot, gone);
    o.gradient_v(Layer::Top, x, y + h - b, w, b, gone, hot);
    o.gradient(Layer::Top, x, y, b, h, hot, gone);
    o.gradient(Layer::Top, x + w - b, y, b, h, gone, hot);
}

/// The status line over the top-left of the image.
pub fn draw_status(o: &mut Overlay, s: &Settings, text: &str, top: f32, alpha: f64, px: f32) {
    if alpha <= 0.004 {
        return;
    }
    let ink = pick_keep_alpha(s.theme.axis_text, Rgba::argb(140, 210, 210, 215)).faded(alpha);
    o.text(Layer::Over, text, Face::Sans, px, 4.0, top + 2.0, ink);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_and_frequencies_read_short() {
        assert_eq!(db(-14.26), "-14.3");
        assert_eq!(db(-70.0), "--");
        assert_eq!(hz(440.0), "440");
        assert_eq!(hz(2400.0), "2.4k");
        assert_eq!(hz(14000.0), "14k");
        assert_eq!(hz(0.0), "--");
        assert_eq!(clock(125.0), "2:05");
        assert_eq!(
            (signed(0.19), signed(-0.5), signed(0.0)),
            ("+0.19".to_owned(), "-0.50".to_owned(), " 0.00".to_owned())
        );
        assert_eq!(clock(-1.0), "0:00");
    }

    #[test]
    fn the_wave_ring_reads_back_from_the_newest() {
        let mut ring = WaveRing::new(4);
        for i in 1..=6 {
            ring.push([-(i as f32), i as f32, 0.0, 0.0]);
        }
        assert_eq!(ring.at(0), [-6.0, 6.0, 0.0, 0.0]);
        assert_eq!(ring.at(3), [-3.0, 3.0, 0.0, 0.0]);
        // Beyond what's been pushed reads as silence, not as the oldest row again.
        assert_eq!(ring.at(4), [0.0; 4]);
    }
}
