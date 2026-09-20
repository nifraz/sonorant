//! Reading Nostalgia+'s settings, preset and theme files.
//!
//! Nostalgia+ wrote plain `Key=Value` lines. [`LegacySettings::parse`] reads them
//! exactly as Nostalgia+ did - keys in any case, the last of a repeated key winning,
//! values it couldn't parse falling back to the default, renamed keys still honoured -
//! and [`LegacySettings::into_settings`] carries the result over to Sonorant's model.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::dsp::{
    AnalysisQuality, BandAggregate, ChannelPairMode, CurveInterpolation, FilteringAmount,
    FreqScale, WindowType,
};
use crate::palette::PaletteKind;
use crate::settings::{
    Argb, AxisLabelMode, CurveStyle, FrameCap, GraphBackground, Preset, ScaleLanePosition,
    Settings, Theme, ThemeSlot,
};

/// A Nostalgia+ settings file, field for field as Nostalgia+ held it in memory.
#[derive(Clone, Debug, PartialEq)]
pub struct LegacySettings {
    /// Everything Sonorant kept, in Sonorant's model.
    pub settings: Settings,
    /// The frame rate the scroll was tied to.
    pub target_fps: i32,
    /// Rows were pushed every this many frames.
    pub scroll_divider: i32,
    /// Capture on or off. Sonorant always captures.
    pub use_loopback: bool,
    /// The docked panel's height in MusicBee. Sonorant's window keeps its own size.
    pub dock_panel_height: i32,
}

impl Default for LegacySettings {
    fn default() -> Self {
        let settings = Settings::default();
        LegacySettings {
            settings,
            target_fps: 60,
            scroll_divider: 1,
            use_loopback: true,
            dock_panel_height: 320,
        }
    }
}

/// Parsed `Key=Value` lines: case-insensitive keys, the last duplicate winning.
struct Lines(HashMap<String, String>);

impl Lines {
    fn parse(text: &str) -> Lines {
        let mut map = HashMap::new();
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // A key has to have at least one character before the '='.
            let Some(eq) = line.find('=').filter(|&i| i > 0) else {
                continue;
            };
            map.insert(
                line[..eq].trim().to_ascii_lowercase(),
                line[eq + 1..].trim().to_owned(),
            );
        }
        Lines(map)
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(&key.to_ascii_lowercase()).map(String::as_str)
    }

    fn bool(&self, key: &str, fallback: bool) -> bool {
        match self.get(key).map(|v| v.trim().to_ascii_lowercase()) {
            Some(v) if v == "true" => true,
            Some(v) if v == "false" => false,
            _ => fallback,
        }
    }

    /// .NET's `double.TryParse` with the invariant culture.
    fn f64(&self, key: &str, fallback: f64) -> f64 {
        self.get(key).and_then(parse_double).unwrap_or(fallback)
    }

    /// An integer setting: parsed as a double, then truncated, as Nostalgia+ did.
    fn i32(&self, key: &str, fallback: i32) -> i32 {
        self.get(key)
            .and_then(parse_double)
            .map_or(fallback, |d| d as i32)
    }

    fn f32(&self, key: &str, fallback: f32) -> f32 {
        self.get(key)
            .and_then(parse_double)
            .map_or(fallback, |d| d as f32)
    }

    /// .NET's `Enum.Parse(type, value, ignoreCase: true)`: a member name or a member's
    /// number.
    fn en<T: Copy>(
        &self,
        key: &str,
        all: &[T],
        name: impl Fn(T) -> &'static str,
        fallback: T,
    ) -> T {
        let Some(v) = self.get(key).map(str::trim) else {
            return fallback;
        };
        if let Some(&found) = all.iter().find(|&&x| name(x).eq_ignore_ascii_case(v)) {
            return found;
        }
        match v.parse::<usize>() {
            Ok(i) if i < all.len() => all[i],
            _ => fallback,
        }
    }

    /// `#AARRGGBB`, or `auto` or nothing for "follow the palette".
    fn colour(&self, key: &str, fallback: Option<Argb>) -> Option<Argb> {
        match self.get(key) {
            None => fallback,
            Some(v) if v.trim().is_empty() || v.trim().eq_ignore_ascii_case("auto") => None,
            Some(v) => Argb::parse(v).map_or(fallback, Some),
        }
    }
}

fn parse_double(text: &str) -> Option<f64> {
    let t = text.trim();
    match t {
        "Infinity" => return Some(f64::INFINITY),
        "-Infinity" => return Some(f64::NEG_INFINITY),
        "NaN" => return Some(f64::NAN),
        _ => {}
    }
    // Rust accepts "inf" and "nan" spellings .NET doesn't; .NET accepts nothing Rust
    // rejects in the float style Nostalgia+ used.
    if t.chars()
        .any(|c| c.is_ascii_alphabetic() && c != 'e' && c != 'E')
    {
        return None;
    }
    t.parse().ok()
}

// Enum names, as Nostalgia+ wrote them.
fn name_quality(q: AnalysisQuality) -> &'static str {
    q.name()
}

impl LegacySettings {
    /// Parses a settings or preset file's text. Never fails: a value it can't read keeps
    /// its default, as in Nostalgia+.
    pub fn parse(text: &str) -> LegacySettings {
        let m = Lines::parse(text);
        let d = LegacySettings::default();
        let s = &d.settings;
        let mut theme = Theme::default();
        for &slot in ThemeSlot::ALL {
            theme.set(
                slot,
                m.colour(&format!("Col{}", slot.name()), s.theme.get(slot)),
            );
        }
        // DeckShowMeters used to cover both meters; an old file's value still sets the
        // pair, so switching them off doesn't come back on after an upgrade.
        let both_meters = m.bool("DeckShowMeters", true);

        let settings = Settings {
            preset: m.en("Preset", Preset::ALL, Preset::name, s.preset),
            palette: m.en("Palette", &PaletteKind::ALL, PaletteKind::name, s.palette),
            scale: m.en("Scale", &FreqScale::ALL, FreqScale::name, s.scale),
            quality: m.en("Quality", &AnalysisQuality::ALL, name_quality, s.quality),
            window: m.en("Window", &WindowType::ALL, WindowType::name, s.window),
            aggregate: m.en(
                "Aggregate",
                &BandAggregate::ALL,
                BandAggregate::name,
                s.aggregate,
            ),
            fmin: m.f64("FMin", s.fmin),
            fmax: m.f64("FMax", s.fmax),
            tilt_db_per_octave: m.f64("Tilt", s.tilt_db_per_octave),
            floor_db: m.f64("FloorDb", s.floor_db),
            ceiling_db: m.f64("CeilingDb", s.ceiling_db),
            attack_ms: m.f64("AttackMs", s.attack_ms),
            release_ms: m.f64("ReleaseMs", s.release_ms),
            peak_decay_db_per_sec: m.f64("PeakDecay", s.peak_decay_db_per_sec),
            average_seconds: m.f64("AverageSeconds", s.average_seconds),
            adaptive_range: m.bool("AdaptiveRange", s.adaptive_range),
            show_grid: m.bool("ShowGrid", s.show_grid),
            show_labels: m.bool("ShowLabels", s.show_labels),
            show_color_bar: m.bool("ShowColorBar", s.show_color_bar),
            show_hud: m.bool("ShowHud", s.show_hud),
            show_status: m.bool("ShowStatus", s.show_status),
            pair_mode: m.en(
                "PairMode",
                &ChannelPairMode::ALL,
                ChannelPairMode::name,
                s.pair_mode,
            ),
            style: m.en("Style", CurveStyle::ALL, CurveStyle::name, s.style),
            interp: m.en(
                "Interp",
                &CurveInterpolation::ALL,
                CurveInterpolation::name,
                s.interp,
            ),
            filter: m.en(
                "Filter",
                &FilteringAmount::ALL,
                FilteringAmount::name,
                s.filter,
            ),
            show_max: m.bool("ShowMax", s.show_max),
            show_min: m.bool("ShowMin", s.show_min),
            show_avg: m.bool("ShowAvg", s.show_avg),
            solid_fill: m.bool("SolidFill", s.solid_fill),
            curve_on_left: m.bool("CurveOnLeft", s.curve_on_left),
            mirror_left_pane: m.bool("MirrorLeftPane", s.mirror_left_pane),
            curve_width_pct: m.i32("CurveWidthPct", s.curve_width_pct),
            wave_height_pct: m.i32("WaveHeightPct", s.wave_height_pct),
            background: m.en(
                "Background",
                GraphBackground::ALL,
                GraphBackground::name,
                s.background,
            ),
            show_db_scale: m.bool("ShowDbScale", s.show_db_scale),
            show_time_marks: m.bool("ShowTimeMarks", s.show_time_marks),
            show_semitones: m.bool("ShowSemitones", s.show_semitones),
            show_outer_labels: m.bool("ShowOuterLabels", s.show_outer_labels),
            show_axis_labels: m.bool("ShowAxisLabels", s.show_axis_labels),
            label_mode: m.en(
                "LabelMode",
                AxisLabelMode::ALL,
                AxisLabelMode::name,
                s.label_mode,
            ),
            label_font_size: m.f32("LabelFontSize", s.label_font_size),
            sync_hover: m.bool("SyncHover", s.sync_hover),
            show_hover_pin: m.bool("ShowHoverPin", s.show_hover_pin),
            show_harmonics: m.bool("ShowHarmonics", s.show_harmonics),
            seek_on_image_click: m.bool("SeekOnImageClick", s.seek_on_image_click),
            show_osd: m.bool("ShowOsd", m.bool("FsShowOsd", s.show_osd)),
            bar_size: m.i32("BarSize", s.bar_size),
            led_segment: m.i32("LedSegment", s.led_segment),
            contrast: m.f64("Contrast", s.contrast),
            gutter_width: m.i32("GutterWidth", s.gutter_width),
            show_quick_buttons: m.bool("ShowQuickButtons", s.show_quick_buttons),
            quick_bar_compact: m.bool("QuickBarCompact", s.quick_bar_compact),
            reserve_scale_space: m.bool("ReserveScaleSpace", s.reserve_scale_space),
            scale_lane_pos: m.en(
                "ScaleLanePos",
                ScaleLanePosition::ALL,
                ScaleLanePosition::name,
                s.scale_lane_pos,
            ),
            show_scale_units: m.bool("ShowScaleUnits", s.show_scale_units),
            show_center_deck: m.bool("ShowCenterDeck", s.show_center_deck),
            deck_height_px: m.i32("DeckHeightPx", s.deck_height_px),
            deck_show_goniometer: m.bool("DeckShowGoniometer", s.deck_show_goniometer),
            deck_show_transport: m.bool("DeckShowTransport", s.deck_show_transport),
            deck_show_artwork: m.bool("DeckShowArtwork", s.deck_show_artwork),
            deck_show_track_info: m.bool("DeckShowTrackInfo", s.deck_show_track_info),
            deck_show_correlation: m.bool("DeckShowCorrelation", both_meters),
            deck_show_balance: m.bool("DeckShowBalance", both_meters),
            deck_show_lufs_m: m.bool("DeckShowLufsM", s.deck_show_lufs_m),
            deck_show_lufs_s: m.bool("DeckShowLufsS", s.deck_show_lufs_s),
            deck_show_true_peak: m.bool("DeckShowTruePeak", s.deck_show_true_peak),
            deck_show_crest: m.bool("DeckShowCrest", s.deck_show_crest),
            deck_show_lufs_i: m.bool("DeckShowLufsI", s.deck_show_lufs_i),
            deck_show_lra: m.bool("DeckShowLra", s.deck_show_lra),
            deck_show_overs: m.bool("DeckShowOvers", s.deck_show_overs),
            deck_show_bpm: m.bool("DeckShowBpm", s.deck_show_bpm),
            deck_show_brightness: m.bool("DeckShowBrightness", s.deck_show_brightness),
            quick_bar_split: m.bool("QuickBarSplit", s.quick_bar_split),
            // These lost their Fs prefix when the docked panel gained them; old files
            // still carry the old keys.
            show_waveform: m.bool("ShowWaveform", m.bool("FsShowWaveform", s.show_waveform)),
            immersive: m.bool("Immersive", m.bool("FsImmersive", s.immersive)),
            glow: m.bool("Glow", m.bool("FsGlow", s.glow)),
            auto_hide: m.bool("AutoHide", m.bool("FsAutoHide", s.auto_hide)),
            imm_backdrop: m.bool("ImmBackdrop", s.imm_backdrop),
            backdrop_pct: m.i32("BackdropPct", s.backdrop_pct),
            imm_beat_reactive: m.bool("ImmBeatReactive", s.imm_beat_reactive),
            imm_colour_follows: m.bool("ImmColourFollows", s.imm_colour_follows),
            colour_follow_degrees: m.i32("ColourFollowDegrees", s.colour_follow_degrees),
            imm_cinematic: m.bool("ImmCinematic", s.imm_cinematic),
            theme,
            // Carried over from the two legacy fields below by `into_settings`.
            frame_cap: s.frame_cap,
            rows_per_second: s.rows_per_second,
            // New here: Nostalgia+ drew one row a frame, with no zoom and no blending.
            px_per_row: s.px_per_row,
            smooth_time: s.smooth_time,
            // New here too: Nostalgia+ redrew the goniometer from scratch every frame,
            // so it had nothing to persist and no key for any of this.
            deck_phosphor: s.deck_phosphor,
            curve_phosphor: s.curve_phosphor,
            waterfall: s.waterfall,
            render_quality: s.render_quality,
            history_minutes: s.history_minutes,
            phosphor_ms: s.phosphor_ms,
            phosphor_intensity: s.phosphor_intensity,
        };
        LegacySettings {
            settings,
            target_fps: m.i32("TargetFps", d.target_fps),
            scroll_divider: m.i32("ScrollDivider", d.scroll_divider),
            use_loopback: m.bool("UseLoopback", d.use_loopback),
            dock_panel_height: m.i32("DockPanelHeight", d.dock_panel_height),
        }
    }

    /// Sonorant's settings. The scroll speed Nostalgia+ implied by its frame rate and
    /// divider becomes rows per second; a 30 fps target stays a 30 fps cap, and
    /// anything else runs at the display's rate.
    pub fn into_settings(self) -> Settings {
        let mut s = self.settings;
        let fps = self.target_fps.max(1) as f64;
        s.rows_per_second = fps / self.scroll_divider.max(1) as f64;
        s.frame_cap = if self.target_fps <= 30 {
            FrameCap::Fps30
        } else {
            FrameCap::Display
        };
        s
    }
}

/// Reads a Nostalgia+ theme file: colour keys only, so it applies over any preset.
pub fn parse_theme(text: &str) -> Theme {
    let m = Lines::parse(text);
    let mut theme = Theme::default();
    for &slot in ThemeSlot::ALL {
        theme.set(slot, m.colour(&format!("Col{}", slot.name()), None));
    }
    theme
}

/// Where Nostalgia+ may have left its settings on this machine, most likely first.
///
/// MusicBee's installer build keeps them under `%APPDATA%\MusicBee`, and the Microsoft
/// Store build under its package's roaming folder. Only meaningful on Windows.
pub fn nostalgia_plus_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        dirs.push(Path::new(&appdata).join("MusicBee").join("NostalgiaPlus"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let packages = Path::new(&local).join("Packages");
        if let Ok(entries) = std::fs::read_dir(&packages) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_ascii_lowercase();
                if name.contains("musicbee") {
                    dirs.push(
                        e.path()
                            .join("LocalCache")
                            .join("Roaming")
                            .join("MusicBee")
                            .join("NostalgiaPlus"),
                    );
                }
            }
        }
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_ignore_case_and_last_one_wins() {
        let l = LegacySettings::parse("fmin=35.5\nFMIN=40\n# FMin=1\n=orphan\nnoequals\n");
        assert_eq!(l.settings.fmin, 40.0);
    }

    #[test]
    fn bad_values_keep_defaults() {
        let l =
            LegacySettings::parse("FMax=not-a-number\nPalette=Rainbow\nColCurve=zz\nShowGrid=yes");
        let d = Settings::default();
        assert_eq!(l.settings.fmax, d.fmax);
        assert_eq!(l.settings.palette, d.palette);
        assert_eq!(l.settings.theme.curve, None);
        assert_eq!(l.settings.show_grid, d.show_grid);
    }

    #[test]
    fn enums_accept_numbers_like_dotnet() {
        let l = LegacySettings::parse("Quality=2\nWindow=blackmanharris");
        assert_eq!(l.settings.quality, AnalysisQuality::High);
        assert_eq!(l.settings.window, WindowType::BlackmanHarris);
    }

    #[test]
    fn scroll_speed_comes_from_rate_and_divider() {
        let s = LegacySettings::parse("TargetFps=60\nScrollDivider=2").into_settings();
        assert_eq!(s.rows_per_second, 30.0);
        assert_eq!(s.frame_cap, FrameCap::Display);
        let s = LegacySettings::parse("TargetFps=30").into_settings();
        assert_eq!(s.frame_cap, FrameCap::Fps30);
        assert_eq!(s.rows_per_second, 30.0);
    }
}
