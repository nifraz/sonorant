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
    /// Screen pixels each history row is drawn across: the time zoom.
    pub px_per_row: i32,
    /// Blend between rows rather than stepping, so slow scrolling doesn't stair-step.
    pub smooth_time: bool,

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
            px_per_row: 1,
            smooth_time: false,
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

named_enum! {
    /// Every switch in [`Settings`], so the menu model, the keys and the quick bar can
    /// name one without a field of their own for each.
    Flag {
        ShowGrid = "ShowGrid",
        ShowLabels = "ShowLabels",
        ShowColourBar = "ShowColourBar",
        ShowHud = "ShowHud",
        ShowStatus = "ShowStatus",
        AdaptiveRange = "AdaptiveRange",
        SmoothTime = "SmoothTime",
        ShowMax = "ShowMax",
        ShowMin = "ShowMin",
        ShowAvg = "ShowAvg",
        SolidFill = "SolidFill",
        CurveOnLeft = "CurveOnLeft",
        MirrorLeftPane = "MirrorLeftPane",
        ShowDbScale = "ShowDbScale",
        ShowTimeMarks = "ShowTimeMarks",
        ShowSemitones = "ShowSemitones",
        ShowOuterLabels = "ShowOuterLabels",
        ShowAxisLabels = "ShowAxisLabels",
        SyncHover = "SyncHover",
        ShowHoverPin = "ShowHoverPin",
        ShowHarmonics = "ShowHarmonics",
        SeekOnImageClick = "SeekOnImageClick",
        ShowOsd = "ShowOsd",
        ShowQuickButtons = "ShowQuickButtons",
        QuickBarCompact = "QuickBarCompact",
        QuickBarSplit = "QuickBarSplit",
        ReserveScaleSpace = "ReserveScaleSpace",
        ShowScaleUnits = "ShowScaleUnits",
        ShowCentreDeck = "ShowCentreDeck",
        DeckGoniometer = "DeckGoniometer",
        DeckTransport = "DeckTransport",
        DeckArtwork = "DeckArtwork",
        DeckTrackInfo = "DeckTrackInfo",
        DeckCorrelation = "DeckCorrelation",
        DeckBalance = "DeckBalance",
        DeckLufsM = "DeckLufsM",
        DeckLufsS = "DeckLufsS",
        DeckTruePeak = "DeckTruePeak",
        DeckCrest = "DeckCrest",
        DeckLufsI = "DeckLufsI",
        DeckLra = "DeckLra",
        DeckOvers = "DeckOvers",
        DeckBpm = "DeckBpm",
        DeckBrightness = "DeckBrightness",
        ShowWaveform = "ShowWaveform",
        Immersive = "Immersive",
        Glow = "Glow",
        AutoHide = "AutoHide",
        Backdrop = "Backdrop",
        BeatReactive = "BeatReactive",
        ColourFollows = "ColourFollows",
        Cinematic = "Cinematic",
    } default ShowGrid
}

named_enum! {
    /// Every setting that is a number rather than a switch or a choice.
    Number {
        Fmin = "Fmin",
        Fmax = "Fmax",
        Tilt = "Tilt",
        FloorDb = "FloorDb",
        CeilingDb = "CeilingDb",
        Contrast = "Contrast",
        AttackMs = "AttackMs",
        ReleaseMs = "ReleaseMs",
        PeakDecay = "PeakDecay",
        AverageSeconds = "AverageSeconds",
        RowsPerSecond = "RowsPerSecond",
        PxPerRow = "PxPerRow",
        CurveWidthPct = "CurveWidthPct",
        WaveHeightPct = "WaveHeightPct",
        DeckHeightPx = "DeckHeightPx",
        GutterWidth = "GutterWidth",
        BarSize = "BarSize",
        LedSegment = "LedSegment",
        LabelFontSize = "LabelFontSize",
        BackdropPct = "BackdropPct",
        ColourFollowDegrees = "ColourFollowDegrees",
    } default RowsPerSecond
}

/// What a [`Number`] accepts: the ends of its range, how far one step moves it, whether
/// it is a whole number, and what it is measured in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub whole: bool,
    pub unit: &'static str,
}

impl Number {
    pub fn range(self) -> Range {
        let (min, max, step, whole, unit) = match self {
            Number::Fmin => (0.0, 1000.0, 5.0, false, "Hz"),
            Number::Fmax => (1000.0, 24000.0, 500.0, false, "Hz"),
            Number::Tilt => (0.0, 6.0, 0.5, false, "dB/oct"),
            Number::FloorDb => (-140.0, -20.0, 5.0, false, "dB"),
            Number::CeilingDb => (-60.0, 0.0, 5.0, false, "dB"),
            Number::Contrast => (0.0, 0.95, 0.05, false, ""),
            Number::AttackMs => (1.0, 400.0, 5.0, false, "ms"),
            Number::ReleaseMs => (20.0, 2000.0, 20.0, false, "ms"),
            Number::PeakDecay => (0.0, 60.0, 2.0, false, "dB/s"),
            Number::AverageSeconds => (0.1, 10.0, 0.1, false, "s"),
            Number::RowsPerSecond => (1.0, 240.0, 5.0, false, "rows/s"),
            Number::PxPerRow => (1.0, 8.0, 1.0, true, "px"),
            Number::CurveWidthPct => (0.0, 60.0, 2.0, true, "%"),
            Number::WaveHeightPct => (0.0, 40.0, 1.0, true, "%"),
            Number::DeckHeightPx => (60.0, 400.0, 10.0, true, "px"),
            Number::GutterWidth => (0.0, 90.0, 2.0, true, "px"),
            Number::BarSize => (1.0, 24.0, 1.0, true, "px"),
            Number::LedSegment => (2.0, 20.0, 1.0, true, "px"),
            Number::LabelFontSize => (5.0, 20.0, 0.5, false, "pt"),
            Number::BackdropPct => (0.0, 60.0, 2.0, true, "%"),
            Number::ColourFollowDegrees => (0.0, 180.0, 5.0, true, "deg"),
        };
        Range {
            min,
            max,
            step,
            whole,
            unit,
        }
    }
}

impl Settings {
    pub fn flag(&self, flag: Flag) -> bool {
        match flag {
            Flag::ShowGrid => self.show_grid,
            Flag::ShowLabels => self.show_labels,
            Flag::ShowColourBar => self.show_color_bar,
            Flag::ShowHud => self.show_hud,
            Flag::ShowStatus => self.show_status,
            Flag::AdaptiveRange => self.adaptive_range,
            Flag::SmoothTime => self.smooth_time,
            Flag::ShowMax => self.show_max,
            Flag::ShowMin => self.show_min,
            Flag::ShowAvg => self.show_avg,
            Flag::SolidFill => self.solid_fill,
            Flag::CurveOnLeft => self.curve_on_left,
            Flag::MirrorLeftPane => self.mirror_left_pane,
            Flag::ShowDbScale => self.show_db_scale,
            Flag::ShowTimeMarks => self.show_time_marks,
            Flag::ShowSemitones => self.show_semitones,
            Flag::ShowOuterLabels => self.show_outer_labels,
            Flag::ShowAxisLabels => self.show_axis_labels,
            Flag::SyncHover => self.sync_hover,
            Flag::ShowHoverPin => self.show_hover_pin,
            Flag::ShowHarmonics => self.show_harmonics,
            Flag::SeekOnImageClick => self.seek_on_image_click,
            Flag::ShowOsd => self.show_osd,
            Flag::ShowQuickButtons => self.show_quick_buttons,
            Flag::QuickBarCompact => self.quick_bar_compact,
            Flag::QuickBarSplit => self.quick_bar_split,
            Flag::ReserveScaleSpace => self.reserve_scale_space,
            Flag::ShowScaleUnits => self.show_scale_units,
            Flag::ShowCentreDeck => self.show_center_deck,
            Flag::DeckGoniometer => self.deck_show_goniometer,
            Flag::DeckTransport => self.deck_show_transport,
            Flag::DeckArtwork => self.deck_show_artwork,
            Flag::DeckTrackInfo => self.deck_show_track_info,
            Flag::DeckCorrelation => self.deck_show_correlation,
            Flag::DeckBalance => self.deck_show_balance,
            Flag::DeckLufsM => self.deck_show_lufs_m,
            Flag::DeckLufsS => self.deck_show_lufs_s,
            Flag::DeckTruePeak => self.deck_show_true_peak,
            Flag::DeckCrest => self.deck_show_crest,
            Flag::DeckLufsI => self.deck_show_lufs_i,
            Flag::DeckLra => self.deck_show_lra,
            Flag::DeckOvers => self.deck_show_overs,
            Flag::DeckBpm => self.deck_show_bpm,
            Flag::DeckBrightness => self.deck_show_brightness,
            Flag::ShowWaveform => self.show_waveform,
            Flag::Immersive => self.immersive,
            Flag::Glow => self.glow,
            Flag::AutoHide => self.auto_hide,
            Flag::Backdrop => self.imm_backdrop,
            Flag::BeatReactive => self.imm_beat_reactive,
            Flag::ColourFollows => self.imm_colour_follows,
            Flag::Cinematic => self.imm_cinematic,
        }
    }

    pub fn set_flag(&mut self, flag: Flag, on: bool) {
        *self.flag_at(flag) = on;
    }

    /// Flips `flag` and says what it is now.
    pub fn toggle(&mut self, flag: Flag) -> bool {
        let at = self.flag_at(flag);
        *at = !*at;
        *at
    }

    fn flag_at(&mut self, flag: Flag) -> &mut bool {
        match flag {
            Flag::ShowGrid => &mut self.show_grid,
            Flag::ShowLabels => &mut self.show_labels,
            Flag::ShowColourBar => &mut self.show_color_bar,
            Flag::ShowHud => &mut self.show_hud,
            Flag::ShowStatus => &mut self.show_status,
            Flag::AdaptiveRange => &mut self.adaptive_range,
            Flag::SmoothTime => &mut self.smooth_time,
            Flag::ShowMax => &mut self.show_max,
            Flag::ShowMin => &mut self.show_min,
            Flag::ShowAvg => &mut self.show_avg,
            Flag::SolidFill => &mut self.solid_fill,
            Flag::CurveOnLeft => &mut self.curve_on_left,
            Flag::MirrorLeftPane => &mut self.mirror_left_pane,
            Flag::ShowDbScale => &mut self.show_db_scale,
            Flag::ShowTimeMarks => &mut self.show_time_marks,
            Flag::ShowSemitones => &mut self.show_semitones,
            Flag::ShowOuterLabels => &mut self.show_outer_labels,
            Flag::ShowAxisLabels => &mut self.show_axis_labels,
            Flag::SyncHover => &mut self.sync_hover,
            Flag::ShowHoverPin => &mut self.show_hover_pin,
            Flag::ShowHarmonics => &mut self.show_harmonics,
            Flag::SeekOnImageClick => &mut self.seek_on_image_click,
            Flag::ShowOsd => &mut self.show_osd,
            Flag::ShowQuickButtons => &mut self.show_quick_buttons,
            Flag::QuickBarCompact => &mut self.quick_bar_compact,
            Flag::QuickBarSplit => &mut self.quick_bar_split,
            Flag::ReserveScaleSpace => &mut self.reserve_scale_space,
            Flag::ShowScaleUnits => &mut self.show_scale_units,
            Flag::ShowCentreDeck => &mut self.show_center_deck,
            Flag::DeckGoniometer => &mut self.deck_show_goniometer,
            Flag::DeckTransport => &mut self.deck_show_transport,
            Flag::DeckArtwork => &mut self.deck_show_artwork,
            Flag::DeckTrackInfo => &mut self.deck_show_track_info,
            Flag::DeckCorrelation => &mut self.deck_show_correlation,
            Flag::DeckBalance => &mut self.deck_show_balance,
            Flag::DeckLufsM => &mut self.deck_show_lufs_m,
            Flag::DeckLufsS => &mut self.deck_show_lufs_s,
            Flag::DeckTruePeak => &mut self.deck_show_true_peak,
            Flag::DeckCrest => &mut self.deck_show_crest,
            Flag::DeckLufsI => &mut self.deck_show_lufs_i,
            Flag::DeckLra => &mut self.deck_show_lra,
            Flag::DeckOvers => &mut self.deck_show_overs,
            Flag::DeckBpm => &mut self.deck_show_bpm,
            Flag::DeckBrightness => &mut self.deck_show_brightness,
            Flag::ShowWaveform => &mut self.show_waveform,
            Flag::Immersive => &mut self.immersive,
            Flag::Glow => &mut self.glow,
            Flag::AutoHide => &mut self.auto_hide,
            Flag::Backdrop => &mut self.imm_backdrop,
            Flag::BeatReactive => &mut self.imm_beat_reactive,
            Flag::ColourFollows => &mut self.imm_colour_follows,
            Flag::Cinematic => &mut self.imm_cinematic,
        }
    }

    pub fn number(&self, n: Number) -> f64 {
        match n {
            Number::Fmin => self.fmin,
            Number::Fmax => self.fmax,
            Number::Tilt => self.tilt_db_per_octave,
            Number::FloorDb => self.floor_db,
            Number::CeilingDb => self.ceiling_db,
            Number::Contrast => self.contrast,
            Number::AttackMs => self.attack_ms,
            Number::ReleaseMs => self.release_ms,
            Number::PeakDecay => self.peak_decay_db_per_sec,
            Number::AverageSeconds => self.average_seconds,
            Number::RowsPerSecond => self.rows_per_second,
            Number::PxPerRow => f64::from(self.px_per_row),
            Number::CurveWidthPct => f64::from(self.curve_width_pct),
            Number::WaveHeightPct => f64::from(self.wave_height_pct),
            Number::DeckHeightPx => f64::from(self.deck_height_px),
            Number::GutterWidth => f64::from(self.gutter_width),
            Number::BarSize => f64::from(self.bar_size),
            Number::LedSegment => f64::from(self.led_segment),
            Number::LabelFontSize => f64::from(self.label_font_size),
            Number::BackdropPct => f64::from(self.backdrop_pct),
            Number::ColourFollowDegrees => f64::from(self.colour_follow_degrees),
        }
    }

    /// Sets `n`, held inside its range and rounded where it is a whole number.
    pub fn set_number(&mut self, n: Number, value: f64) {
        let r = n.range();
        let v = if value.is_finite() { value } else { r.min };
        let v = v.clamp(r.min, r.max);
        let v = if r.whole { v.round() } else { v };
        match n {
            Number::Fmin => self.fmin = v,
            Number::Fmax => self.fmax = v,
            Number::Tilt => self.tilt_db_per_octave = v,
            Number::FloorDb => self.floor_db = v,
            Number::CeilingDb => self.ceiling_db = v,
            Number::Contrast => self.contrast = v,
            Number::AttackMs => self.attack_ms = v,
            Number::ReleaseMs => self.release_ms = v,
            Number::PeakDecay => self.peak_decay_db_per_sec = v,
            Number::AverageSeconds => self.average_seconds = v,
            Number::RowsPerSecond => self.rows_per_second = v,
            Number::PxPerRow => self.px_per_row = v as i32,
            Number::CurveWidthPct => self.curve_width_pct = v as i32,
            Number::WaveHeightPct => self.wave_height_pct = v as i32,
            Number::DeckHeightPx => self.deck_height_px = v as i32,
            Number::GutterWidth => self.gutter_width = v as i32,
            Number::BarSize => self.bar_size = v as i32,
            Number::LedSegment => self.led_segment = v as i32,
            Number::LabelFontSize => self.label_font_size = v as f32,
            Number::BackdropPct => self.backdrop_pct = v as i32,
            Number::ColourFollowDegrees => self.colour_follow_degrees = v as i32,
        }
    }

    /// Moves `n` by `steps` of its own step size.
    pub fn step_number(&mut self, n: Number, steps: f64) {
        let r = n.range();
        self.set_number(n, self.number(n) + steps * r.step);
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
