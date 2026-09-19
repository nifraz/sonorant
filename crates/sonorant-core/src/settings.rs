//! Every setting, the built-in presets, and themes.
//!
//! The fields follow Nostalgia+'s, renamed to Rust style, with the changes the plan
//! lists: the frame-rate target becomes a cap, the scroll divider becomes a scroll
//! speed in rows per second, and the MusicBee-only fields are gone.

use serde::{Deserialize, Serialize};

use crate::dsp::{
    AnalysisQuality, BandAggregate, ChannelPairMode, CurveInterpolation, FilteringAmount,
    FreqScale, WindowType,
};
use crate::palette::PaletteKind;

/// Declares a fieldless enum with its settings-file names, a list of every variant and
/// name lookups.
macro_rules! named_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident = $text:literal),+ $(,)? } default $default:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name { $($(#[$vmeta])* $variant),+ }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn name(self) -> &'static str {
                match self { $($name::$variant => $text),+ }
            }

            pub fn from_name(name: &str) -> Option<$name> {
                Self::ALL.iter().copied().find(|v| v.name().eq_ignore_ascii_case(name))
            }
        }

        impl Default for $name {
            fn default() -> Self { $name::$default }
        }
    };
}

named_enum! {
    /// The built-in starting points. `Custom` means "changed since a preset".
    Preset {
        Nostalgia = "Nostalgia",
        Studio = "Studio",
        QC = "QC",
        Immersive = "Immersive",
        Custom = "Custom",
        Vocal = "Vocal",
        Bass = "Bass",
        Percussion = "Percussion",
        Mastering = "Mastering",
    } default Studio
}

named_enum! {
    /// How the spectrum curve is drawn.
    CurveStyle { Line = "Line", Bars = "Bars", Led = "Led" } default Line
}

named_enum! {
    /// The pattern behind the curve.
    GraphBackground { Plain = "Plain", Lines = "Lines", Grid = "Grid", Chessboard = "Chessboard" } default Lines
}

named_enum! {
    /// What the frequency axis prints at each gridline.
    AxisLabelMode { Notes = "Notes", Frequency = "Frequency", Both = "Both" } default Notes
}

named_enum! {
    /// Which end of the panes the scale strip sits at.
    ScaleLanePosition { Top = "Top", Bottom = "Bottom" } default Top
}

named_enum! {
    /// How often the screen is redrawn. Scrolling follows audio time whatever this is.
    FrameCap {
        /// Every refresh of the display.
        Display = "Display",
        Fps60 = "60",
        Fps30 = "30",
    } default Display
}

named_enum! {
    /// The parts of the display whose colour can be set by hand.
    ///
    /// Every one defaults to unset, meaning "follow the palette", so colours stay
    /// coherent when the palette changes and keep following the music when the hue
    /// tracks the centroid. Setting a slot opts that one element out of all that.
    ThemeSlot {
        Background = "Background",
        Panel = "Panel",
        GridMajor = "GridMajor",
        GridMinor = "GridMinor",
        AxisText = "AxisText",
        Units = "Units",
        Curve = "Curve",
        PeakTrace = "PeakTrace",
        AverageTrace = "AverageTrace",
        MinimumTrace = "MinimumTrace",
        Hover = "Hover",
        Waveform = "Waveform",
        Snapshot = "Snapshot",
    } default Background
}

/// A colour with alpha, packed `0xAARRGGBB` as Nostalgia+ stored it. Written as
/// `#AARRGGBB`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Argb(pub u32);

impl Argb {
    pub fn a(self) -> u8 {
        (self.0 >> 24) as u8
    }
    pub fn r(self) -> u8 {
        (self.0 >> 16) as u8
    }
    pub fn g(self) -> u8 {
        (self.0 >> 8) as u8
    }
    pub fn b(self) -> u8 {
        self.0 as u8
    }

    /// Parses `#AARRGGBB` or `AARRGGBB`, as Nostalgia+'s files hold them.
    pub fn parse(text: &str) -> Option<Argb> {
        let hex = text.trim();
        let hex = hex.strip_prefix('#').unwrap_or(hex).trim();
        if hex.is_empty() || hex.len() > 8 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(hex, 16).ok().map(Argb)
    }
}

impl std::fmt::Display for Argb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{:08X}", self.0)
    }
}

impl Serialize for Argb {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Argb {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Argb, D::Error> {
        let text = String::deserialize(d)?;
        Argb::parse(&text).ok_or_else(|| serde::de::Error::custom(format!("not a colour: {text}")))
    }
}

/// Hand-set colours, one optional colour per [`ThemeSlot`]. Unset slots follow the
/// palette.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Theme {
    pub background: Option<Argb>,
    pub panel: Option<Argb>,
    pub grid_major: Option<Argb>,
    pub grid_minor: Option<Argb>,
    pub axis_text: Option<Argb>,
    pub units: Option<Argb>,
    pub curve: Option<Argb>,
    pub peak_trace: Option<Argb>,
    pub average_trace: Option<Argb>,
    pub minimum_trace: Option<Argb>,
    pub hover: Option<Argb>,
    pub waveform: Option<Argb>,
    pub snapshot: Option<Argb>,
}

impl Theme {
    pub fn get(&self, slot: ThemeSlot) -> Option<Argb> {
        *self.slot(slot)
    }

    pub fn set(&mut self, slot: ThemeSlot, colour: Option<Argb>) {
        *self.slot_mut(slot) = colour;
    }

    pub fn clear(&mut self) {
        *self = Theme::default();
    }

    fn slot(&self, slot: ThemeSlot) -> &Option<Argb> {
        match slot {
            ThemeSlot::Background => &self.background,
            ThemeSlot::Panel => &self.panel,
            ThemeSlot::GridMajor => &self.grid_major,
            ThemeSlot::GridMinor => &self.grid_minor,
            ThemeSlot::AxisText => &self.axis_text,
            ThemeSlot::Units => &self.units,
            ThemeSlot::Curve => &self.curve,
            ThemeSlot::PeakTrace => &self.peak_trace,
            ThemeSlot::AverageTrace => &self.average_trace,
            ThemeSlot::MinimumTrace => &self.minimum_trace,
            ThemeSlot::Hover => &self.hover,
            ThemeSlot::Waveform => &self.waveform,
            ThemeSlot::Snapshot => &self.snapshot,
        }
    }

    fn slot_mut(&mut self, slot: ThemeSlot) -> &mut Option<Argb> {
        match slot {
            ThemeSlot::Background => &mut self.background,
            ThemeSlot::Panel => &mut self.panel,
            ThemeSlot::GridMajor => &mut self.grid_major,
            ThemeSlot::GridMinor => &mut self.grid_minor,
            ThemeSlot::AxisText => &mut self.axis_text,
            ThemeSlot::Units => &mut self.units,
            ThemeSlot::Curve => &mut self.curve,
            ThemeSlot::PeakTrace => &mut self.peak_trace,
            ThemeSlot::AverageTrace => &mut self.average_trace,
            ThemeSlot::MinimumTrace => &mut self.minimum_trace,
            ThemeSlot::Hover => &mut self.hover,
            ThemeSlot::Waveform => &mut self.waveform,
            ThemeSlot::Snapshot => &mut self.snapshot,
        }
    }
}

/// Every setting. Missing fields in a file take their defaults, so older files load.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub preset: Preset,
    pub palette: PaletteKind,
    pub scale: FreqScale,
    pub fmin: f64,
    pub fmax: f64,
    pub quality: AnalysisQuality,
    pub window: WindowType,
    pub aggregate: BandAggregate,
    pub tilt_db_per_octave: f64,
    pub adaptive_range: bool,
    pub floor_db: f64,
    pub ceiling_db: f64,

    pub show_grid: bool,
    pub show_labels: bool,
    pub show_color_bar: bool,
    pub show_hud: bool,
    pub show_status: bool,
    pub frame_cap: FrameCap,
    pub attack_ms: f64,
    pub release_ms: f64,
    /// How fast the held maximum trace lets go.
    pub peak_decay_db_per_sec: f64,
    /// Seconds for the average trace to follow a step change.
    pub average_seconds: f64,
    /// Spectrogram rows per second of audio. Nostalgia+ tied this to its frame rate.
    pub rows_per_second: f64,

    // Per-channel panes, shared by every view.
    pub pair_mode: ChannelPairMode,
    pub style: CurveStyle,
    pub interp: CurveInterpolation,
    pub filter: FilteringAmount,
    pub show_max: bool,
    pub show_min: bool,
    pub show_avg: bool,
    pub solid_fill: bool,
    pub curve_on_left: bool,
    /// Mirror the left pane so both newest slices meet at the centre and history flows
    /// outward: the sound seems to come from the middle.
    pub mirror_left_pane: bool,
    /// Graph strip width as a percentage of each pane; 0 hides the graph.
    pub curve_width_pct: i32,
    /// Waveform lane height as a percentage of the view; 0 hides it.
    pub wave_height_pct: i32,

    // Axes, grid and hover.
    pub background: GraphBackground,
    pub show_db_scale: bool,
    pub show_time_marks: bool,
    pub show_semitones: bool,
    pub show_outer_labels: bool,
    pub show_axis_labels: bool,
    pub label_mode: AxisLabelMode,
    /// Point size for axis, scale and readout text.
    pub label_font_size: f32,
    /// Draw the hover line across both panes and read out both channels.
    pub sync_hover: bool,
    /// Stamp the hovered frequency onto the axis itself.
    pub show_hover_pin: bool,
    /// Ghost lines at whole multiples of the hovered frequency.
    pub show_harmonics: bool,
    /// Double-click a spectrogram column to seek the player to that moment.
    pub seek_on_image_click: bool,
    /// Master switch for on-screen display.
    pub show_osd: bool,

    // Quick action bar.
    pub show_quick_buttons: bool,
    pub quick_bar_compact: bool,

    // Reserved scale strip.
    pub reserve_scale_space: bool,
    pub scale_lane_pos: ScaleLanePosition,
    pub show_scale_units: bool,

    // Centre deck.
    pub show_center_deck: bool,
    pub deck_height_px: i32,
    pub deck_show_goniometer: bool,
    pub deck_show_transport: bool,
    pub deck_show_artwork: bool,
    pub deck_show_track_info: bool,
    pub deck_show_correlation: bool,
    pub deck_show_balance: bool,
    pub deck_show_lufs_m: bool,
    pub deck_show_lufs_s: bool,
    pub deck_show_true_peak: bool,
    pub deck_show_crest: bool,
    pub deck_show_lufs_i: bool,
    pub deck_show_lra: bool,
    pub deck_show_overs: bool,
    pub deck_show_bpm: bool,
    pub deck_show_brightness: bool,
    /// Split the quick bar around the centre gutter so the axis runs unbroken.
    pub quick_bar_split: bool,
    pub bar_size: i32,
    pub led_segment: i32,
    /// Low percentile for auto-range. Raising it blackens more of the noise floor, the
    /// main lever against dense material rendering as haze.
    pub contrast: f64,

    // Fullscreen and immersion.
    pub gutter_width: i32,
    pub show_waveform: bool,
    pub immersive: bool,
    pub glow: bool,
    pub auto_hide: bool,
    pub imm_backdrop: bool,
    pub backdrop_pct: i32,
    pub imm_beat_reactive: bool,
    pub imm_colour_follows: bool,
    pub colour_follow_degrees: i32,
    pub imm_cinematic: bool,

    pub theme: Theme,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            preset: Preset::Studio,
            palette: PaletteKind::Magma,
            scale: FreqScale::Note,
            fmin: 20.0,
            fmax: 20000.0,
            quality: AnalysisQuality::Balanced,
            window: WindowType::Hann,
            aggregate: BandAggregate::Peak,
            tilt_db_per_octave: 3.0,
            adaptive_range: true,
            floor_db: -95.0,
            ceiling_db: -5.0,
            show_grid: true,
            show_labels: true,
            show_color_bar: true,
            show_hud: true,
            show_status: true,
            frame_cap: FrameCap::Display,
            attack_ms: 20.0,
            release_ms: 320.0,
            peak_decay_db_per_sec: 14.0,
            average_seconds: 1.2,
            rows_per_second: 60.0,
            pair_mode: ChannelPairMode::LeftRight,
            style: CurveStyle::Line,
            interp: CurveInterpolation::LinearSmooth,
            filter: FilteringAmount::Light,
            show_max: true,
            show_min: false,
            show_avg: false,
            solid_fill: true,
            curve_on_left: true,
            mirror_left_pane: false,
            curve_width_pct: 18,
            wave_height_pct: 10,
            background: GraphBackground::Lines,
            show_db_scale: true,
            show_time_marks: true,
            show_semitones: true,
            show_outer_labels: true,
            show_axis_labels: true,
            label_mode: AxisLabelMode::Notes,
            label_font_size: 7.0,
            sync_hover: true,
            show_hover_pin: true,
            show_harmonics: false,
            seek_on_image_click: true,
            show_osd: true,
            show_quick_buttons: true,
            quick_bar_compact: false,
            reserve_scale_space: true,
            scale_lane_pos: ScaleLanePosition::Top,
            show_scale_units: true,
            show_center_deck: true,
            deck_height_px: 120,
            deck_show_goniometer: true,
            deck_show_transport: true,
            deck_show_artwork: true,
            deck_show_track_info: true,
            deck_show_correlation: true,
            deck_show_balance: true,
            deck_show_lufs_m: true,
            deck_show_lufs_s: true,
            deck_show_true_peak: true,
            deck_show_crest: true,
            deck_show_lufs_i: true,
            deck_show_lra: true,
            deck_show_overs: true,
            deck_show_bpm: true,
            deck_show_brightness: true,
            quick_bar_split: true,
            bar_size: 6,
            led_segment: 5,
            contrast: 0.25,
            gutter_width: 34,
            show_waveform: true,
            immersive: false,
            glow: true,
            auto_hide: true,
            imm_backdrop: true,
            backdrop_pct: 18,
            imm_beat_reactive: true,
            imm_colour_follows: true,
            colour_follow_degrees: 40,
            imm_cinematic: false,
            theme: Theme::default(),
        }
    }
}

/// The scroll speed Nostalgia+'s presets meant by a scroll divider of 2 at 60 fps.
const HALF_SPEED: f64 = 30.0;

impl Settings {
    /// Height of the reserved scale strip in pixels at 96 dpi, derived from the text
    /// size so a larger font never leaves the strip too small for its labels.
    pub fn scale_lane_height(&self) -> i32 {
        if !self.reserve_scale_space {
            return 0;
        }
        let f = self.label_font_size.clamp(5.0, 20.0);
        (f * 1.9) as i32 + 7
    }

    /// Spectrogram rows per second of audio as drawn: cinematic mode quarters it.
    pub fn effective_rows_per_second(&self) -> f64 {
        let rps = self.rows_per_second.max(0.0);
        if self.imm_cinematic { rps / 4.0 } else { rps }
    }

    /// Applies a built-in preset over the current settings. Only the fields a preset is
    /// about change; everything else is left as it was.
    pub fn apply_preset(&mut self, p: Preset) {
        self.preset = p;
        match p {
            Preset::Nostalgia => {
                // The original look, with the mapping bugs left out.
                self.palette = PaletteKind::NostalgiaRed;
                self.scale = FreqScale::Linear;
                self.fmin = 0.0;
                self.fmax = 22050.0;
                self.quality = AnalysisQuality::Balanced;
                self.tilt_db_per_octave = 0.0;
                self.aggregate = BandAggregate::Peak;
                self.adaptive_range = true;
                self.immersive = false;
            }
            Preset::QC => {
                // Reading masters and spotting lossy sources: flat, wide, no tilt, a
                // linear top end so a codec shelf is obvious.
                self.palette = PaletteKind::Viridis;
                self.scale = FreqScale::Linear;
                self.fmin = 0.0;
                self.fmax = 22050.0;
                self.quality = AnalysisQuality::High;
                self.tilt_db_per_octave = 0.0;
                self.aggregate = BandAggregate::Energy;
                self.adaptive_range = false;
                self.floor_db = -110.0;
                self.ceiling_db = 0.0;
                self.immersive = false;
            }
            Preset::Immersive => {
                // For watching rather than measuring: a musical axis, tilt so the top
                // half isn't haze, and resolution enough for harmonics to be lines.
                self.palette = PaletteKind::Magma;
                self.scale = FreqScale::Note;
                self.fmin = 25.0;
                self.fmax = 18000.0;
                self.quality = AnalysisQuality::High;
                self.tilt_db_per_octave = 1.5;
                self.aggregate = BandAggregate::Peak;
                self.adaptive_range = true;
                self.contrast = 0.55;
                self.rows_per_second = HALF_SPEED;
                self.immersive = true;
                self.glow = true;
                self.auto_hide = true;
                self.show_waveform = true;
            }
            Preset::Vocal => {
                // Where voices live, with the resolution to separate formants.
                self.palette = PaletteKind::Magma;
                self.scale = FreqScale::Note;
                self.fmin = 150.0;
                self.fmax = 9000.0;
                self.quality = AnalysisQuality::Balanced;
                self.tilt_db_per_octave = 1.5;
                self.aggregate = BandAggregate::Peak;
                self.adaptive_range = true;
                self.contrast = 0.45;
                self.rows_per_second = HALF_SPEED;
                self.immersive = false;
            }
            Preset::Bass => {
                // A narrow low range needs the largest transforms: a semitone at E1 is
                // 2.4 Hz wide.
                self.palette = PaletteKind::Inferno;
                self.scale = FreqScale::Note;
                self.fmin = 20.0;
                self.fmax = 800.0;
                self.quality = AnalysisQuality::High;
                self.tilt_db_per_octave = 0.0;
                self.aggregate = BandAggregate::Peak;
                self.adaptive_range = true;
                self.contrast = 0.35;
                self.rows_per_second = HALF_SPEED;
                self.immersive = false;
            }
            Preset::Percussion => {
                // Transients are about timing, so latency and scroll speed matter more
                // than frequency detail.
                self.palette = PaletteKind::Turbo;
                self.scale = FreqScale::Note;
                self.fmin = 40.0;
                self.fmax = 18000.0;
                self.quality = AnalysisQuality::LowLatency;
                self.tilt_db_per_octave = 3.0;
                self.aggregate = BandAggregate::Peak;
                self.adaptive_range = true;
                self.contrast = 0.60;
                self.rows_per_second = 60.0;
                self.immersive = false;
            }
            Preset::Mastering => {
                // Measurement: nothing tilted or adaptive, energy summed so noise floors
                // read at their true level.
                self.palette = PaletteKind::Viridis;
                self.scale = FreqScale::Linear;
                self.fmin = 0.0;
                self.fmax = 22050.0;
                self.quality = AnalysisQuality::High;
                self.tilt_db_per_octave = 0.0;
                self.aggregate = BandAggregate::Energy;
                self.adaptive_range = false;
                self.floor_db = -120.0;
                self.ceiling_db = 0.0;
                self.rows_per_second = HALF_SPEED;
                self.immersive = false;
            }
            Preset::Studio | Preset::Custom => {
                self.palette = PaletteKind::Magma;
                self.scale = FreqScale::Note;
                self.fmin = 20.0;
                self.fmax = 20000.0;
                self.quality = AnalysisQuality::Balanced;
                self.tilt_db_per_octave = 3.0;
                self.aggregate = BandAggregate::Peak;
                self.adaptive_range = true;
                self.immersive = false;
            }
        }
    }
}

/// Keeps a name to letters, digits, spaces, hyphens and underscores, so it can be a
/// file name on any system. Trimmed at both ends.
pub fn sanitise_name(name: &str) -> String {
    name.trim()
        .chars()
        .filter(|&c| c.is_alphanumeric() || c == ' ' || c == '-' || c == '_')
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_parse_and_print() {
        assert_eq!(Argb::parse("#FF102030"), Some(Argb(0xFF10_2030)));
        assert_eq!(Argb::parse("80abcdef"), Some(Argb(0x80AB_CDEF)));
        assert_eq!(Argb::parse("zz"), None);
        assert_eq!(Argb::parse("#123456789"), None);
        assert_eq!(Argb(0xFF0C_1824).to_string(), "#FF0C1824");
    }

    #[test]
    fn cinematic_quarters_the_scroll() {
        let mut s = Settings {
            rows_per_second: 30.0,
            ..Settings::default()
        };
        let plain = s.effective_rows_per_second();
        s.imm_cinematic = true;
        assert!((plain - s.effective_rows_per_second() * 4.0).abs() < 1e-12);
    }

    #[test]
    fn names_are_sanitised() {
        assert_eq!(sanitise_name("bad/name*?"), "badname");
        assert_eq!(sanitise_name("  Round Trip  "), "Round Trip");
    }

    #[test]
    fn theme_slots_are_independent() {
        let mut t = Theme::default();
        for (i, &slot) in ThemeSlot::ALL.iter().enumerate() {
            t.set(slot, Some(Argb(i as u32)));
        }
        for (i, &slot) in ThemeSlot::ALL.iter().enumerate() {
            assert_eq!(t.get(slot), Some(Argb(i as u32)), "{slot:?}");
        }
        t.clear();
        assert!(ThemeSlot::ALL.iter().all(|&s| t.get(s).is_none()));
    }
}
