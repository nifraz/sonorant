//! The menu model: one tree of items that the menu, the keyboard and the help search
//! all read.
//!
//! Nostalgia+ built its menu in code, and its keyboard handler and help text repeated
//! the same knowledge separately, so the three drifted apart. Here an [`Item`] carries
//! its label, its state, its shortcut, its help line and the [`Action`] it performs, and
//! [`tree`] builds the whole menu from the current state. The egui menu renders the
//! tree, [`action_for_key`] looks a keypress up in it, and [`search`] flattens it for
//! the help window, so a shortcut or a help entry cannot drift away from its item.
//!
//! Applying an action is [`apply`]. Almost everything it does is a write into
//! [`Settings`] or [`Session`]; the few things the model cannot do itself, because they
//! need the settings folder, come back as an [`Effect`] for the app to carry out.

use crate::dsp::{
    AnalysisQuality, BandAggregate, ChannelPairMode, CurveInterpolation, FilteringAmount,
    FreqScale, WindowType,
};
use crate::media::{Controls, Follow, Player, Transport};
use crate::palette::PaletteKind;
use crate::settings::{
    AxisLabelMode, CurveStyle, Flag, FrameCap, GraphBackground, Number, Preset, ScaleLanePosition,
    Settings,
};

/// What capture listens to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Capture {
    /// Whatever the followed player is playing through, falling back to the whole mix
    /// when there is no player to follow.
    #[default]
    FollowPlayer,
    /// Everything the machine is playing, whoever is playing it.
    WholeSystem,
}

impl Capture {
    pub const ALL: [Capture; 2] = [Capture::FollowPlayer, Capture::WholeSystem];

    pub fn label(self) -> &'static str {
        match self {
            Capture::FollowPlayer => "Following the player",
            Capture::WholeSystem => "Everything the system plays",
        }
    }
}

/// How finished frames reach the screen. The app maps these onto the backend's own
/// present modes; the model only needs to name them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Presentation {
    /// Queue every frame for the next refresh. No tearing, and the frame rate is the
    /// refresh rate.
    #[default]
    EveryRefresh,
    /// Keep only the newest frame in the queue: no tearing, and the latest picture.
    Newest,
    /// Present immediately, tearing included.
    Immediate,
}

impl Presentation {
    pub const ALL: [Presentation; 3] = [
        Presentation::EveryRefresh,
        Presentation::Newest,
        Presentation::Immediate,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Presentation::EveryRefresh => "Every refresh",
            Presentation::Newest => "Newest frame",
            Presentation::Immediate => "Immediate",
        }
    }
}

/// One of the settings that takes a value from a fixed list.
///
/// Every variant holds the value it would set, so a radio item is one `Choice` and the
/// model can both apply it and say whether it is the current one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Palette(PaletteKind),
    Scale(FreqScale),
    Pair(ChannelPairMode),
    Style(CurveStyle),
    Interp(CurveInterpolation),
    Filter(FilteringAmount),
    Quality(AnalysisQuality),
    Window(WindowType),
    Aggregate(BandAggregate),
    Background(GraphBackground),
    Labels(AxisLabelMode),
    ScaleLane(ScaleLanePosition),
    Cap(FrameCap),
}

impl Choice {
    /// What this reads as in a menu.
    pub fn label(self) -> &'static str {
        match self {
            Choice::Palette(p) => p.display_name(),
            Choice::Scale(s) => match s {
                FreqScale::Note => "Notes",
                FreqScale::Log => "Logarithmic",
                FreqScale::Linear => "Linear",
            },
            Choice::Pair(m) => match m {
                ChannelPairMode::LeftRight => "Left and right",
                ChannelPairMode::MidSide => "Mid and side",
                ChannelPairMode::LeftOnly => "Left only",
                ChannelPairMode::RightOnly => "Right only",
            },
            Choice::Style(s) => match s {
                CurveStyle::Line => "Line",
                CurveStyle::Bars => "Bars",
                CurveStyle::Led => "LED",
            },
            Choice::Interp(i) => match i {
                CurveInterpolation::PeakFlat => "Flat peaks",
                CurveInterpolation::LinearSmooth => "Linear smooth",
                CurveInterpolation::CubicSpline => "Cubic spline",
            },
            Choice::Filter(f) => match f {
                FilteringAmount::None => "None",
                FilteringAmount::Light => "Light",
                FilteringAmount::Medium => "Medium",
                FilteringAmount::Strong => "Strong",
            },
            Choice::Quality(q) => match q {
                AnalysisQuality::Fast => "Fast",
                AnalysisQuality::Balanced => "Balanced",
                AnalysisQuality::High => "High",
                AnalysisQuality::LowLatency => "Low latency",
            },
            Choice::Window(w) => match w {
                WindowType::Hann => "Hann",
                WindowType::Hamming => "Hamming",
                WindowType::BlackmanHarris => "Blackman-Harris",
                WindowType::Nuttall => "Nuttall",
                WindowType::Gaussian => "Gaussian",
                WindowType::Rectangular => "Rectangular",
            },
            Choice::Aggregate(a) => match a {
                BandAggregate::Peak => "Peak",
                BandAggregate::Energy => "Energy",
            },
            Choice::Background(b) => match b {
                GraphBackground::Plain => "Plain",
                GraphBackground::Lines => "Lines",
                GraphBackground::Grid => "Grid",
                GraphBackground::Chessboard => "Chessboard",
            },
            Choice::Labels(l) => match l {
                AxisLabelMode::Notes => "Notes",
                AxisLabelMode::Frequency => "Frequency",
                AxisLabelMode::Both => "Both",
            },
            Choice::ScaleLane(p) => match p {
                ScaleLanePosition::Top => "Top",
                ScaleLanePosition::Bottom => "Bottom",
            },
            Choice::Cap(c) => match c {
                FrameCap::Display => "Every refresh",
                FrameCap::Fps60 => "60 a second",
                FrameCap::Fps30 => "30 a second",
            },
        }
    }

    /// Whether this is what the settings already hold.
    pub fn is_current(self, s: &Settings) -> bool {
        match self {
            Choice::Palette(p) => s.palette == p,
            Choice::Scale(v) => s.scale == v,
            Choice::Pair(v) => s.pair_mode == v,
            Choice::Style(v) => s.style == v,
            Choice::Interp(v) => s.interp == v,
            Choice::Filter(v) => s.filter == v,
            Choice::Quality(v) => s.quality == v,
            Choice::Window(v) => s.window == v,
            Choice::Aggregate(v) => s.aggregate == v,
            Choice::Background(v) => s.background == v,
            Choice::Labels(v) => s.label_mode == v,
            Choice::ScaleLane(v) => s.scale_lane_pos == v,
            Choice::Cap(v) => s.frame_cap == v,
        }
    }

    fn apply(self, s: &mut Settings) {
        match self {
            Choice::Palette(v) => s.palette = v,
            Choice::Scale(v) => s.scale = v,
            Choice::Pair(v) => s.pair_mode = v,
            Choice::Style(v) => s.style = v,
            Choice::Interp(v) => s.interp = v,
            Choice::Filter(v) => s.filter = v,
            Choice::Quality(v) => s.quality = v,
            Choice::Window(v) => s.window = v,
            Choice::Aggregate(v) => s.aggregate = v,
            Choice::Background(v) => s.background = v,
            Choice::Labels(v) => s.label_mode = v,
            Choice::ScaleLane(v) => s.scale_lane_pos = v,
            Choice::Cap(v) => s.frame_cap = v,
        }
    }
}

/// Everything a menu item, a key or a quick-bar button can do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Flip a switch.
    Toggle(Flag),
    /// Set a number outright.
    Set(Number, f64),
    /// Move a number by whole steps of its own size.
    Step(Number, f64),
    /// Take one of a fixed set of values.
    Choose(Choice),
    /// Move to the next palette, so a key can walk through them.
    NextPalette,
    /// Move to the next curve style.
    NextStyle,
    /// Load a built-in preset.
    UsePreset(Preset),
    /// Load one of the user's saved presets, by name.
    LoadPreset(String),
    /// Open the name dialog and save the current settings under the name it takes.
    SavePreset,
    /// Delete one of the user's saved presets.
    DeletePreset(String),
    /// Put every setting back to its default.
    ResetSettings,
    /// Point capture at the player or at the whole mix.
    SetCapture(Capture),
    /// Choose which player the deck follows.
    SetFollow(Follow),
    /// Hold the picture still while analysis carries on.
    Freeze,
    /// Hold the average spectrum as an amber reference, or drop the one held.
    Reference,
    Fullscreen,
    /// Leave fullscreen, and do nothing when already windowed, so `Esc` can carry it
    /// without swallowing the key in a window.
    LeaveFullscreen,
    /// Show or hide the help window.
    Help,
    SetPresentation(Presentation),
    /// Ask the player to do something.
    Send(Transport),
    Quit,
}

/// Something the app has to carry out, because the model cannot reach the settings
/// folder itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    LoadPreset(String),
    /// Open the name dialog; the app saves once it is answered.
    SavePreset,
    DeletePreset(String),
}

/// The state the menu reads and writes that isn't a saved setting.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session {
    pub frozen: bool,
    pub fullscreen: bool,
    /// Whether an amber reference spectrum is being held.
    pub reference: bool,
    pub help: bool,
    pub quit: bool,
    pub capture: Capture,
    pub follow: Follow,
    pub presentation: Presentation,
    /// What the menu or a deck button asked the player to do, taken by the app each
    /// frame.
    pub command: Option<Transport>,
}

/// What [`tree`] needs to build the menu: the state it shows, and the lists that only
/// the running app knows.
#[derive(Debug)]
pub struct Context<'a> {
    pub settings: &'a Settings,
    pub session: &'a Session,
    /// The players a session can see, for the follow list.
    pub players: &'a [Player],
    /// The user's saved presets, by name.
    pub presets: &'a [String],
    /// What the followed player says it can be asked to do.
    pub controls: Controls,
    /// The presentation modes this GPU and surface offer.
    pub presentations: &'a [Presentation],
}

/// What an item is.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// Does something and closes the menu.
    Command(Action),
    /// A switch, with what it is now.
    Check(Action, bool),
    /// One of a group, with whether it is the one chosen.
    Radio(Action, bool),
    Submenu(Vec<Item>),
    Separator,
}

/// One line of the menu.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub label: String,
    /// One sentence for the help window, and for a tooltip.
    pub help: &'static str,
    /// The key that does this, written as [`action_for_key`] expects it.
    pub shortcut: Option<&'static str>,
    pub kind: Kind,
    pub enabled: bool,
}

impl Item {
    pub fn command(label: impl Into<String>, help: &'static str, action: Action) -> Item {
        Item::of(label, help, Kind::Command(action))
    }

    pub fn check(label: impl Into<String>, help: &'static str, action: Action, on: bool) -> Item {
        Item::of(label, help, Kind::Check(action, on))
    }

    pub fn radio(
        label: impl Into<String>,
        help: &'static str,
        action: Action,
        chosen: bool,
    ) -> Item {
        Item::of(label, help, Kind::Radio(action, chosen))
    }

    pub fn submenu(label: impl Into<String>, help: &'static str, items: Vec<Item>) -> Item {
        Item::of(label, help, Kind::Submenu(items))
    }

    pub fn separator() -> Item {
        Item::of("", "", Kind::Separator)
    }

    /// Gives the item the key that performs it.
    pub fn key(mut self, shortcut: &'static str) -> Item {
        self.shortcut = Some(shortcut);
        self
    }

    /// Greys the item out, for something that exists but cannot be used now.
    pub fn when(mut self, enabled: bool) -> Item {
        self.enabled = enabled;
        self
    }

    /// The action the item performs, if any.
    pub fn action(&self) -> Option<&Action> {
        match &self.kind {
            Kind::Command(a) | Kind::Check(a, _) | Kind::Radio(a, _) => Some(a),
            Kind::Submenu(_) | Kind::Separator => None,
        }
    }

    fn of(label: impl Into<String>, help: &'static str, kind: Kind) -> Item {
        Item {
            label: label.into(),
            help,
            shortcut: None,
            kind,
            enabled: true,
        }
    }
}

/// A switch, named and explained.
fn flag(label: &'static str, help: &'static str, f: Flag, s: &Settings) -> Item {
    Item::check(label, help, Action::Toggle(f), s.flag(f))
}

/// A submenu of one radio group: every value of a choice, with the current one marked.
fn choices<T: Copy>(
    label: &'static str,
    help: &'static str,
    values: impl IntoIterator<Item = T>,
    wrap: impl Fn(T) -> Choice,
    item_help: &'static str,
    s: &Settings,
) -> Item {
    let items = values
        .into_iter()
        .map(|v| {
            let c = wrap(v);
            Item::radio(c.label(), item_help, Action::Choose(c), c.is_current(s))
        })
        .collect();
    Item::submenu(label, help, items)
}

/// A submenu of set sizes for one number, with the nearest marked.
fn sizes(
    label: &'static str,
    help: &'static str,
    n: Number,
    steps: &[f64],
    fmt: impl Fn(f64) -> String,
    s: &Settings,
) -> Item {
    let now = s.number(n);
    // The current value may be one no menu entry offers, from a preset or an old file,
    // so the nearest is marked rather than none of them.
    let nearest = steps
        .iter()
        .copied()
        .min_by(|a, b| (a - now).abs().total_cmp(&(b - now).abs()));
    let items = steps
        .iter()
        .map(|&v| Item::radio(fmt(v), help, Action::Set(n, v), nearest == Some(v)))
        .collect();
    Item::submenu(label, help, items)
}

/// Builds the whole menu for the state in `ctx`.
pub fn tree(ctx: &Context<'_>) -> Vec<Item> {
    let s = ctx.settings;
    let session = ctx.session;
    vec![
        Item::submenu(
            "Presets",
            "Whole sets of settings for a way of listening",
            presets(ctx),
        ),
        Item::separator(),
        Item::submenu(
            "Capture",
            "Which sound is analysed",
            Capture::ALL
                .iter()
                .map(|&c| {
                    Item::radio(
                        c.label(),
                        "Analyse the followed player alone, or the whole mix",
                        Action::SetCapture(c),
                        session.capture == c,
                    )
                })
                .collect(),
        ),
        Item::submenu(
            "Follow player",
            "Which player the deck and capture follow",
            follow(ctx),
        ),
        Item::separator(),
        Item::submenu("Analysis", "How the sound is measured", analysis(s)),
        Item::submenu("Graph", "The curve strip beside each image", graph(s)),
        Item::submenu("Spectrogram", "The scrolling image itself", spectrogram(s)),
        Item::submenu(
            "Axes and labels",
            "The grid, the scales and their text",
            axes(s),
        ),
        Item::submenu("Hover", "What the pointer reads out", hover(s)),
        Item::submenu("Waveform", "The lanes along the bottom", waveform(s)),
        Item::submenu("Centre deck", "The panel between the panes", deck(s)),
        Item::submenu(
            "Quick bar",
            "The row of buttons over the image",
            quick_bar(s),
        ),
        Item::submenu("Immersive", "The full-screen treatment", immersive(s)),
        Item::separator(),
        Item::check(
            "Freeze",
            "Hold the picture still; analysis carries on",
            Action::Freeze,
            session.frozen,
        )
        .key("Space"),
        Item::check(
            "Reference curve",
            "Hold the average spectrum in amber to compare against, or drop it",
            Action::Reference,
            session.reference,
        )
        .key("A"),
        Item::check(
            "Fullscreen",
            "Fill the monitor the window is on",
            Action::Fullscreen,
            session.fullscreen,
        )
        .key("F11"),
        Item::command(
            "Leave fullscreen",
            "Go back to a window",
            Action::LeaveFullscreen,
        )
        .key("Esc")
        .when(session.fullscreen),
        Item::submenu("Player", "Ask the player to do something", transport(ctx)),
        Item::separator(),
        Item::submenu(
            "Frame rate",
            "How often the screen is redrawn",
            vec![
                choices(
                    "Redraw",
                    "How often the screen is redrawn; scrolling follows audio time either way",
                    FrameCap::ALL.iter().copied(),
                    Choice::Cap,
                    "How often the screen is redrawn",
                    s,
                ),
                Item::submenu(
                    "Presentation",
                    "How finished frames reach the screen",
                    Presentation::ALL
                        .iter()
                        .map(|&p| {
                            Item::radio(
                                p.label(),
                                "How finished frames reach the screen",
                                Action::SetPresentation(p),
                                session.presentation == p,
                            )
                            .when(ctx.presentations.contains(&p))
                        })
                        .collect(),
                ),
            ],
        ),
        Item::check(
            "Status line",
            "The line of figures over the image",
            Action::Toggle(Flag::ShowStatus),
            s.flag(Flag::ShowStatus),
        ),
        Item::separator(),
        Item::check(
            "Help",
            "Every command, with its key and what it does",
            Action::Help,
            session.help,
        )
        .key("F1"),
        Item::command(
            "Reset every setting",
            "Put everything back to its default",
            Action::ResetSettings,
        ),
        Item::command("Quit", "Close Sonorant", Action::Quit),
    ]
}

fn presets(ctx: &Context<'_>) -> Vec<Item> {
    let s = ctx.settings;
    let built_in = [
        (
            Preset::Studio,
            "A musical axis and a gentle tilt: the everyday view",
        ),
        (
            Preset::Nostalgia,
            "The original plugin's look, without its mapping bugs",
        ),
        (
            Preset::QC,
            "Flat, wide and linear, for spotting a lossy source",
        ),
        (Preset::Immersive, "For watching rather than measuring"),
        (
            Preset::Vocal,
            "Where voices live, with the detail to separate formants",
        ),
        (
            Preset::Bass,
            "A narrow low range, with the largest transforms",
        ),
        (
            Preset::Percussion,
            "Short windows, so transients land where they are heard",
        ),
        (Preset::Mastering, "Nothing tilted or adaptive: measurement"),
    ];
    let mut items: Vec<Item> = built_in
        .iter()
        .map(|&(p, help)| Item::radio(p.name(), help, Action::UsePreset(p), s.preset == p))
        .collect();
    items.push(Item::separator());
    if ctx.presets.is_empty() {
        items.push(
            Item::command(
                "No saved presets",
                "Presets you save appear here",
                Action::SavePreset,
            )
            .when(false),
        );
    }
    for name in ctx.presets {
        items.push(Item::command(
            name.clone(),
            "One of your saved presets",
            Action::LoadPreset(name.clone()),
        ));
    }
    items.push(Item::separator());
    items.push(Item::command(
        "Save these settings...",
        "Save everything as it is now under a name of your own",
        Action::SavePreset,
    ));
    items.push(Item::submenu(
        "Delete",
        "Remove one of your saved presets",
        if ctx.presets.is_empty() {
            vec![Item::command("Nothing saved", "", Action::SavePreset).when(false)]
        } else {
            ctx.presets
                .iter()
                .map(|n| {
                    Item::command(
                        n.clone(),
                        "Remove this saved preset",
                        Action::DeletePreset(n.clone()),
                    )
                })
                .collect()
        },
    ));
    items
}

fn follow(ctx: &Context<'_>) -> Vec<Item> {
    let mut items = vec![Item::radio(
        "Whichever is playing",
        "Follow whichever player is playing, preferring the one that started last",
        Action::SetFollow(Follow::Whichever),
        ctx.session.follow == Follow::Whichever,
    )];
    if ctx.players.is_empty() {
        items.push(
            Item::command(
                "No players running",
                "Players appear here once one is open",
                Action::SetFollow(Follow::Whichever),
            )
            .when(false),
        );
    }
    for player in ctx.players {
        // Pinning is by id, because two windows of the same app share a name.
        let pinned = Follow::Pinned(player.id.clone());
        let chosen = ctx.session.follow == pinned;
        items.push(Item::radio(
            player.name.clone(),
            "Follow this player, whatever else starts",
            Action::SetFollow(pinned),
            chosen,
        ));
    }
    items
}

fn transport(ctx: &Context<'_>) -> Vec<Item> {
    let c = ctx.controls;
    vec![
        Item::command(
            "Play or pause",
            "Ask the player to play or pause",
            Action::Send(Transport::PlayPause),
        )
        .when(c.play_pause),
        Item::command(
            "Next track",
            "Ask the player for the next track",
            Action::Send(Transport::Next),
        )
        .when(c.next),
        Item::command(
            "Previous track",
            "Ask the player for the previous track",
            Action::Send(Transport::Previous),
        )
        .when(c.previous),
    ]
}

fn analysis(s: &Settings) -> Vec<Item> {
    vec![
        choices(
            "Channels",
            "Which pair of channels the panes show",
            ChannelPairMode::ALL.iter().copied(),
            Choice::Pair,
            "Which pair of channels the panes show",
            s,
        ),
        choices(
            "Resolution",
            "How large the transforms are: detail against delay",
            AnalysisQuality::ALL.iter().copied(),
            Choice::Quality,
            "How large the transforms are",
            s,
        ),
        choices(
            "Window",
            "The window each transform is taken through",
            WindowType::ALL.iter().copied(),
            Choice::Window,
            "The window each transform is taken through",
            s,
        ),
        choices(
            "Band aggregate",
            "Whether a band takes the loudest bin in it or sums their energy",
            BandAggregate::ALL.iter().copied(),
            Choice::Aggregate,
            "Whether a band takes its loudest bin or sums their energy",
            s,
        ),
        Item::separator(),
        sizes(
            "Tilt",
            "Lift the top end so the treble isn't a thin line under the bass",
            Number::Tilt,
            &[0.0, 1.5, 3.0, 4.5, 6.0],
            |v| format!("{v:.1} dB an octave"),
            s,
        ),
        flag(
            "Follow the material",
            "Move the level range with the music rather than holding it fixed",
            Flag::AdaptiveRange,
            s,
        ),
        sizes(
            "Contrast",
            "How much of the quiet end is pushed to black",
            Number::Contrast,
            &[0.0, 0.25, 0.45, 0.6, 0.8],
            |v| format!("{:.0}%", v * 100.0),
            s,
        ),
        sizes(
            "Floor",
            "The level the darkest colour means, when the range is fixed",
            Number::FloorDb,
            &[-140.0, -120.0, -95.0, -80.0, -60.0],
            |v| format!("{v:.0} dB"),
            s,
        ),
        sizes(
            "Ceiling",
            "The level the brightest colour means, when the range is fixed",
            Number::CeilingDb,
            &[0.0, -5.0, -10.0, -20.0],
            |v| format!("{v:.0} dB"),
            s,
        ),
        Item::separator(),
        sizes(
            "Attack",
            "How fast a curve rises to a new level",
            Number::AttackMs,
            &[1.0, 10.0, 20.0, 50.0, 100.0],
            |v| format!("{v:.0} ms"),
            s,
        ),
        sizes(
            "Release",
            "How fast a curve falls away from one",
            Number::ReleaseMs,
            &[80.0, 160.0, 320.0, 640.0, 1200.0],
            |v| format!("{v:.0} ms"),
            s,
        ),
    ]
}

fn graph(s: &Settings) -> Vec<Item> {
    vec![
        sizes(
            "Width",
            "How much of each pane the curve strip takes",
            Number::CurveWidthPct,
            &[0.0, 10.0, 18.0, 26.0, 40.0],
            |v| {
                if v == 0.0 {
                    "Hidden".to_owned()
                } else {
                    format!("{v:.0}% of the pane")
                }
            },
            s,
        ),
        choices(
            "Style",
            "Whether the curve is a line, bars or an LED ladder",
            CurveStyle::ALL.iter().copied(),
            Choice::Style,
            "How the curve is drawn",
            s,
        ),
        Item::command(
            "Next style",
            "Move on to the next curve style",
            Action::NextStyle,
        )
        .key("B"),
        choices(
            "Interpolation",
            "How the curve runs between the bands it is made of",
            CurveInterpolation::ALL.iter().copied(),
            Choice::Interp,
            "How the curve runs between its bands",
            s,
        ),
        choices(
            "Smoothing",
            "How hard neighbouring bands are averaged together",
            FilteringAmount::ALL.iter().copied(),
            Choice::Filter,
            "How hard neighbouring bands are averaged",
            s,
        ),
        choices(
            "Background",
            "The pattern behind the curve",
            GraphBackground::ALL.iter().copied(),
            Choice::Background,
            "The pattern behind the curve",
            s,
        ),
        Item::separator(),
        flag(
            "Peak trace",
            "A held maximum that decays slowly",
            Flag::ShowMax,
            s,
        )
        .key("P"),
        flag(
            "Average trace",
            "The running average of the spectrum",
            Flag::ShowAvg,
            s,
        ),
        flag(
            "Minimum trace",
            "The quietest level each band has reached",
            Flag::ShowMin,
            s,
        ),
        flag(
            "Fill under the curve",
            "Shade the area beneath the line",
            Flag::SolidFill,
            s,
        ),
        Item::separator(),
        flag(
            "Curve beside the gutter",
            "Put the curve strip on the inner edge of each pane",
            Flag::CurveOnLeft,
            s,
        ),
        flag(
            "Mirror the left pane",
            "Both newest columns meet in the middle, so history flows outward",
            Flag::MirrorLeftPane,
            s,
        ),
        sizes(
            "Bar width",
            "How wide one bar is, in the bar and LED styles",
            Number::BarSize,
            &[2.0, 4.0, 6.0, 10.0, 16.0],
            |v| format!("{v:.0} px"),
            s,
        ),
        sizes(
            "LED segment",
            "How tall one lit segment is in the LED style",
            Number::LedSegment,
            &[3.0, 5.0, 8.0, 12.0],
            |v| format!("{v:.0} px"),
            s,
        ),
    ]
}

fn spectrogram(s: &Settings) -> Vec<Item> {
    vec![
        choices(
            "Palette",
            "The colours levels are drawn in",
            PaletteKind::ALL.iter().copied(),
            Choice::Palette,
            "Draw the image in these colours",
            s,
        ),
        Item::command(
            "Next palette",
            "Move on to the next palette",
            Action::NextPalette,
        )
        .key("C"),
        choices(
            "Frequency axis",
            "How frequency is spread across the axis",
            FreqScale::ALL.iter().copied(),
            Choice::Scale,
            "How frequency is spread across the axis",
            s,
        ),
        sizes(
            "Scroll speed",
            "How many rows of history a second of sound becomes",
            Number::RowsPerSecond,
            &[15.0, 30.0, 60.0, 120.0],
            |v| format!("{v:.0} rows a second"),
            s,
        ),
        sizes(
            "Zoom",
            "How many screen pixels one row is drawn across",
            Number::PxPerRow,
            &[1.0, 2.0, 4.0, 8.0],
            |v| format!("{v:.0} px a row"),
            s,
        ),
        flag(
            "Blend between rows",
            "Fade between rows rather than stepping, so slow scrolling stays smooth",
            Flag::SmoothTime,
            s,
        ),
        Item::separator(),
        sizes(
            "Lowest frequency",
            "Where the axis starts",
            Number::Fmin,
            &[0.0, 20.0, 40.0, 80.0, 150.0],
            |v| format!("{v:.0} Hz"),
            s,
        ),
        sizes(
            "Highest frequency",
            "Where the axis ends",
            Number::Fmax,
            &[8000.0, 12000.0, 16000.0, 18000.0, 20000.0, 22050.0],
            |v| format!("{:.0} kHz", v / 1000.0),
            s,
        ),
        sizes(
            "Centre gutter",
            "How wide the labelled strip between the panes is",
            Number::GutterWidth,
            &[0.0, 24.0, 34.0, 48.0, 64.0],
            |v| format!("{v:.0} px"),
            s,
        ),
        flag(
            "Colour bar",
            "The level-to-colour key down the right edge",
            Flag::ShowColourBar,
            s,
        ),
    ]
}

fn axes(s: &Settings) -> Vec<Item> {
    vec![
        flag(
            "Grid",
            "Lines across the image at the frequencies labelled",
            Flag::ShowGrid,
            s,
        )
        .key("G"),
        flag(
            "Axis labels",
            "The frequencies written along the gutter",
            Flag::ShowAxisLabels,
            s,
        ),
        choices(
            "Labels read as",
            "Whether the axis is labelled in notes, hertz or both",
            AxisLabelMode::ALL.iter().copied(),
            Choice::Labels,
            "Whether the axis is labelled in notes, hertz or both",
            s,
        ),
        flag(
            "Repeat at the edges",
            "Label columns at both outer edges as well",
            Flag::ShowOuterLabels,
            s,
        ),
        flag(
            "Semitone lines",
            "A faint line at every semitone, where there is room",
            Flag::ShowSemitones,
            s,
        ),
        flag(
            "Level scale",
            "The dB scale along the curve strip",
            Flag::ShowDbScale,
            s,
        ),
        flag(
            "Time marks",
            "Seconds marked along the image",
            Flag::ShowTimeMarks,
            s,
        ),
        Item::separator(),
        flag(
            "Scale strip",
            "Keep a strip at one end for the scales",
            Flag::ReserveScaleSpace,
            s,
        ),
        choices(
            "Strip at",
            "Which end of the panes the scale strip sits at",
            ScaleLanePosition::ALL.iter().copied(),
            Choice::ScaleLane,
            "Which end the scale strip sits at",
            s,
        ),
        flag(
            "Unit captions",
            "The words under the scales saying what they measure",
            Flag::ShowScaleUnits,
            s,
        ),
        sizes(
            "Text size",
            "How large the axis, scale and readout text is",
            Number::LabelFontSize,
            &[6.0, 7.0, 8.0, 10.0, 12.0],
            |v| format!("{v:.0} pt"),
            s,
        ),
    ]
}

fn hover(s: &Settings) -> Vec<Item> {
    vec![
        flag(
            "On-screen readouts",
            "The hover readout, the quick bar and the status line over the image",
            Flag::ShowOsd,
            s,
        )
        .key("O"),
        flag(
            "Read out both panes",
            "Draw the hover line across both panes and read out both channels",
            Flag::SyncHover,
            s,
        ),
        flag(
            "Pin to the axis",
            "Stamp the hovered frequency onto the axis itself",
            Flag::ShowHoverPin,
            s,
        ),
        flag(
            "Harmonic ruler",
            "Ghost lines at whole multiples of the frequency under the pointer",
            Flag::ShowHarmonics,
            s,
        ),
        flag(
            "Double-click to seek",
            "Double-click a column of the image to send the player to that moment",
            Flag::SeekOnImageClick,
            s,
        ),
    ]
}

fn waveform(s: &Settings) -> Vec<Item> {
    vec![
        flag(
            "Waveform lanes",
            "The level against time along the bottom",
            Flag::ShowWaveform,
            s,
        )
        .key("W"),
        sizes(
            "Height",
            "How much of the view the lanes take",
            Number::WaveHeightPct,
            &[0.0, 6.0, 10.0, 16.0, 24.0],
            |v| {
                if v == 0.0 {
                    "Hidden".to_owned()
                } else {
                    format!("{v:.0}% of the view")
                }
            },
            s,
        ),
    ]
}

fn deck(s: &Settings) -> Vec<Item> {
    vec![
        flag(
            "Centre deck",
            "The panel of figures between the panes",
            Flag::ShowCentreDeck,
            s,
        ),
        sizes(
            "Height",
            "How tall the deck and the lanes beside it are",
            Number::DeckHeightPx,
            &[80.0, 100.0, 120.0, 160.0, 200.0],
            |v| format!("{v:.0} px"),
            s,
        ),
        Item::separator(),
        flag("Artwork", "The album cover", Flag::DeckArtwork, s),
        flag(
            "Track details",
            "Title, artists, album and year",
            Flag::DeckTrackInfo,
            s,
        ),
        flag(
            "Transport",
            "The play button, the seek bar and the clock",
            Flag::DeckTransport,
            s,
        ),
        flag(
            "Goniometer",
            "The stereo field as a figure",
            Flag::DeckGoniometer,
            s,
        ),
        flag(
            "Correlation",
            "How alike the two channels are",
            Flag::DeckCorrelation,
            s,
        ),
        flag("Balance", "Which side is louder", Flag::DeckBalance, s),
        Item::separator(),
        flag(
            "Momentary loudness",
            "LUFS over the last 400 ms",
            Flag::DeckLufsM,
            s,
        ),
        flag(
            "Short-term loudness",
            "LUFS over the last three seconds",
            Flag::DeckLufsS,
            s,
        ),
        flag(
            "Integrated loudness",
            "LUFS over the track so far",
            Flag::DeckLufsI,
            s,
        ),
        flag(
            "Loudness range",
            "How far the loudness moves over the track",
            Flag::DeckLra,
            s,
        ),
        flag(
            "True peak",
            "The highest level between samples, in dBTP",
            Flag::DeckTruePeak,
            s,
        ),
        flag(
            "Crest factor",
            "How far the peaks stand above the average",
            Flag::DeckCrest,
            s,
        ),
        flag(
            "Overs",
            "How many samples went past full scale",
            Flag::DeckOvers,
            s,
        ),
        flag(
            "Tempo",
            "The beats a minute the music is running at",
            Flag::DeckBpm,
            s,
        ),
        flag(
            "Brightness",
            "Where the spectrum's weight sits",
            Flag::DeckBrightness,
            s,
        ),
    ]
}

fn quick_bar(s: &Settings) -> Vec<Item> {
    vec![
        flag(
            "Quick bar",
            "A row of buttons for the things changed most often",
            Flag::ShowQuickButtons,
            s,
        ),
        flag(
            "Compact",
            "Icons alone, without their words",
            Flag::QuickBarCompact,
            s,
        ),
        flag(
            "Split around the gutter",
            "Leave the centre strip clear so the axis runs unbroken",
            Flag::QuickBarSplit,
            s,
        ),
    ]
}

fn immersive(s: &Settings) -> Vec<Item> {
    vec![
        flag(
            "Immersive mode",
            "The full-screen treatment: glow, fading chrome and a backdrop",
            Flag::Immersive,
            s,
        )
        .key("I"),
        flag(
            "Glow",
            "Bright parts of the image bleed light into what is around them",
            Flag::Glow,
            s,
        ),
        flag(
            "Hide the chrome",
            "Fade the furniture and the pointer away while nothing moves",
            Flag::AutoHide,
            s,
        ),
        flag(
            "Cinematic",
            "Quarter the scroll speed, for watching rather than reading",
            Flag::Cinematic,
            s,
        ),
        Item::separator(),
        flag(
            "Artwork backdrop",
            "The album cover, blurred, behind the analysis",
            Flag::Backdrop,
            s,
        ),
        sizes(
            "Backdrop strength",
            "How far forward the backdrop comes",
            Number::BackdropPct,
            &[0.0, 10.0, 18.0, 30.0, 45.0, 60.0],
            |v| format!("{v:.0}%"),
            s,
        ),
        flag(
            "Beat flare",
            "A pulse of light at each beat",
            Flag::BeatReactive,
            s,
        ),
        flag(
            "Colour follows the music",
            "Drift the palette with the spectrum's brightness",
            Flag::ColourFollows,
            s,
        ),
        sizes(
            "Colour drift",
            "How far the palette is allowed to drift",
            Number::ColourFollowDegrees,
            &[0.0, 20.0, 40.0, 80.0, 120.0],
            |v| format!("{v:.0} degrees"),
            s,
        ),
    ]
}

/// Performs `action`, and says what the app has to finish.
///
/// Anything that changes a setting also marks the settings as `Custom`, so the preset
/// list stops claiming a preset the settings have moved away from.
pub fn apply(action: &Action, s: &mut Settings, session: &mut Session) -> Option<Effect> {
    let before = s.preset;
    let mut touched = true;
    match action {
        Action::Toggle(f) => {
            s.toggle(*f);
        }
        Action::Set(n, v) => s.set_number(*n, *v),
        Action::Step(n, by) => s.step_number(*n, *by),
        Action::Choose(c) => c.apply(s),
        Action::NextPalette => {
            let all = PaletteKind::ALL;
            let at = all.iter().position(|&p| p == s.palette).unwrap_or(0);
            s.palette = all[(at + 1) % all.len()];
        }
        Action::NextStyle => {
            let all = CurveStyle::ALL;
            let at = all.iter().position(|&c| c == s.style).unwrap_or(0);
            s.style = all[(at + 1) % all.len()];
        }
        Action::UsePreset(p) => {
            s.apply_preset(*p);
            touched = false;
        }
        Action::ResetSettings => {
            *s = Settings::default();
            touched = false;
        }
        Action::LoadPreset(name) => return Some(Effect::LoadPreset(name.clone())),
        Action::SavePreset => return Some(Effect::SavePreset),
        Action::DeletePreset(name) => return Some(Effect::DeletePreset(name.clone())),
        Action::SetCapture(c) => {
            session.capture = *c;
            touched = false;
        }
        Action::SetFollow(f) => {
            session.follow = f.clone();
            touched = false;
        }
        Action::Freeze => {
            session.frozen = !session.frozen;
            touched = false;
        }
        Action::Reference => {
            session.reference = !session.reference;
            touched = false;
        }
        Action::Fullscreen => {
            session.fullscreen = !session.fullscreen;
            touched = false;
        }
        Action::LeaveFullscreen => {
            session.fullscreen = false;
            touched = false;
        }
        Action::Help => {
            session.help = !session.help;
            touched = false;
        }
        Action::SetPresentation(p) => {
            session.presentation = *p;
            touched = false;
        }
        Action::Send(t) => {
            session.command = Some(*t);
            touched = false;
        }
        Action::Quit => {
            session.quit = true;
            touched = false;
        }
    }
    // A preset the settings have moved away from is no longer that preset.
    if touched && s.preset == before {
        s.preset = Preset::Custom;
    }
    None
}

/// The action a key performs, looked up in the menu itself so the two cannot disagree.
///
/// `key` is a name as the items carry it: a single letter, or `Space`, `Esc`, `F1` or
/// `F11`. An item that is greyed out doesn't answer to its key either, so `Esc` does
/// nothing in a window.
pub fn action_for_key(ctx: &Context<'_>, key: &str) -> Option<Action> {
    find_key(&tree(ctx), key)
}

fn find_key(items: &[Item], key: &str) -> Option<Action> {
    for item in items {
        if !item.enabled {
            continue;
        }
        if let Kind::Submenu(children) = &item.kind {
            if let Some(found) = find_key(children, key) {
                return Some(found);
            }
        } else if item.shortcut.is_some_and(|k| k.eq_ignore_ascii_case(key))
            && let Some(action) = item.action()
        {
            return Some(action.clone());
        }
    }
    None
}

/// One command as the help window lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Where it is, as "Graph > Peak trace".
    pub path: String,
    pub help: &'static str,
    pub shortcut: Option<&'static str>,
    pub action: Action,
}

/// Every command in the tree, with the path that leads to it.
pub fn entries(items: &[Item]) -> Vec<Entry> {
    let mut out = Vec::new();
    walk(items, "", &mut out);
    out
}

fn walk(items: &[Item], prefix: &str, out: &mut Vec<Entry>) {
    for item in items {
        match &item.kind {
            Kind::Separator => {}
            Kind::Submenu(children) => {
                let path = if prefix.is_empty() {
                    item.label.clone()
                } else {
                    format!("{prefix} > {}", item.label)
                };
                walk(children, &path, out);
            }
            Kind::Command(a) | Kind::Check(a, _) | Kind::Radio(a, _) => {
                let path = if prefix.is_empty() {
                    item.label.clone()
                } else {
                    format!("{prefix} > {}", item.label)
                };
                out.push(Entry {
                    path,
                    help: item.help,
                    shortcut: item.shortcut,
                    action: a.clone(),
                });
            }
        }
    }
}

/// The commands whose path, help or key matches `query`, in the order they appear in the
/// menu. An empty query is everything.
pub fn search(items: &[Item], query: &str) -> Vec<Entry> {
    let q = query.trim().to_lowercase();
    entries(items)
        .into_iter()
        .filter(|e| {
            q.is_empty()
                || e.path.to_lowercase().contains(&q)
                || e.help.to_lowercase().contains(&q)
                || e.shortcut.is_some_and(|k| k.to_lowercase() == q)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context<'a>(settings: &'a Settings, session: &'a Session) -> Context<'a> {
        Context {
            settings,
            session,
            players: &[],
            presets: &[],
            controls: Controls {
                play_pause: true,
                next: true,
                previous: true,
                seek: true,
            },
            presentations: &Presentation::ALL,
        }
    }

    #[test]
    fn every_flag_round_trips() {
        let mut s = Settings::default();
        for &f in Flag::ALL {
            let was = s.flag(f);
            assert_eq!(s.toggle(f), !was, "{f:?}");
            assert_eq!(s.flag(f), !was, "{f:?}");
            s.set_flag(f, was);
            assert_eq!(s.flag(f), was, "{f:?}");
        }
        // Every flag reaches a field of its own: flipping one moves nothing else.
        for &f in Flag::ALL {
            let mut a = Settings::default();
            a.toggle(f);
            let changed: Vec<Flag> = Flag::ALL
                .iter()
                .copied()
                .filter(|&g| a.flag(g) != Settings::default().flag(g))
                .collect();
            assert_eq!(changed, vec![f], "{f:?} moved more than itself");
        }
    }

    #[test]
    fn numbers_stay_inside_their_range() {
        for &n in Number::ALL {
            let r = n.range();
            let mut s = Settings::default();
            s.set_number(n, -1e9);
            assert_eq!(s.number(n), r.min, "{n:?} under");
            s.set_number(n, 1e9);
            assert_eq!(s.number(n), r.max, "{n:?} over");
            s.set_number(n, f64::NAN);
            assert_eq!(s.number(n), r.min, "{n:?} not a number");
            // A step from the bottom moves by exactly one step.
            s.set_number(n, r.min);
            s.step_number(n, 1.0);
            let want = (r.min + r.step).min(r.max);
            let want = if r.whole { want.round() } else { want };
            assert!(
                (s.number(n) - want).abs() < 1e-6,
                "{n:?} stepped to {}",
                s.number(n)
            );
        }
    }

    #[test]
    fn menu_items_are_in_step_with_the_settings() {
        let mut s = Settings::default();
        s.set_flag(Flag::ShowGrid, false);
        s.palette = PaletteKind::Turbo;
        let session = Session::default();
        let items = tree(&context(&s, &session));
        let found = entries(&items);
        let grid = found
            .iter()
            .find(|e| e.path == "Axes and labels > Grid")
            .expect("the grid switch");
        assert_eq!(grid.action, Action::Toggle(Flag::ShowGrid));
        // The palette in the settings is the one the menu marks.
        let items_flat = flatten(&items);
        let chosen: Vec<&Item> = items_flat
            .iter()
            .copied()
            .filter(|i| {
                matches!(
                    i.kind,
                    Kind::Radio(Action::Choose(Choice::Palette(_)), true)
                )
            })
            .collect();
        assert_eq!(chosen.len(), 1);
        assert_eq!(chosen[0].label, PaletteKind::Turbo.display_name());
    }

    fn flatten(items: &[Item]) -> Vec<&Item> {
        let mut out = Vec::new();
        for item in items {
            out.push(item);
            if let Kind::Submenu(children) = &item.kind {
                out.extend(flatten(children));
            }
        }
        out
    }

    #[test]
    fn keys_do_what_their_menu_item_does() {
        let s = Settings::default();
        let session = Session::default();
        let ctx = context(&s, &session);
        for (key, want) in [
            ("Space", Action::Freeze),
            ("A", Action::Reference),
            ("F11", Action::Fullscreen),
            ("I", Action::Toggle(Flag::Immersive)),
            ("W", Action::Toggle(Flag::ShowWaveform)),
            ("O", Action::Toggle(Flag::ShowOsd)),
            ("G", Action::Toggle(Flag::ShowGrid)),
            ("P", Action::Toggle(Flag::ShowMax)),
            ("F1", Action::Help),
            ("B", Action::NextStyle),
            ("C", Action::NextPalette),
        ] {
            assert_eq!(action_for_key(&ctx, key), Some(want.clone()), "{key}");
            // Lower case is the same key.
            assert_eq!(
                action_for_key(&ctx, &key.to_lowercase()),
                Some(want),
                "{key}"
            );
        }
        assert_eq!(action_for_key(&ctx, "Z"), None);

        // Esc answers only where it has something to do, so it doesn't swallow the key
        // in a window.
        assert_eq!(action_for_key(&ctx, "Esc"), None);
        let full = Session {
            fullscreen: true,
            ..Session::default()
        };
        assert_eq!(
            action_for_key(&context(&s, &full), "Esc"),
            Some(Action::LeaveFullscreen)
        );
    }

    #[test]
    fn no_key_is_bound_twice() {
        let s = Settings::default();
        let session = Session::default();
        let items = tree(&context(&s, &session));
        let mut seen: Vec<&str> = entries(&items).iter().filter_map(|e| e.shortcut).collect();
        seen.sort_unstable();
        let count = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), count, "a key appears on two items: {seen:?}");
    }

    #[test]
    fn actions_reach_the_settings_and_the_session() {
        let mut s = Settings::default();
        let mut session = Session::default();
        assert!(apply(&Action::Toggle(Flag::Glow), &mut s, &mut session).is_none());
        assert_eq!(s.glow, !Settings::default().glow);
        // Changing anything means the settings are no longer the preset they came from.
        assert_eq!(s.preset, Preset::Custom);

        apply(&Action::UsePreset(Preset::QC), &mut s, &mut session);
        assert_eq!(s.preset, Preset::QC);
        assert_eq!(s.palette, PaletteKind::Viridis);

        apply(&Action::Freeze, &mut s, &mut session);
        assert!(session.frozen);
        // Session state isn't a settings change, so the preset stands.
        assert_eq!(s.preset, Preset::QC);

        apply(&Action::Send(Transport::Next), &mut s, &mut session);
        assert_eq!(session.command, Some(Transport::Next));

        apply(&Action::Quit, &mut s, &mut session);
        assert!(session.quit);
    }

    #[test]
    fn presets_come_back_as_effects() {
        let mut s = Settings::default();
        let mut session = Session::default();
        assert_eq!(
            apply(&Action::LoadPreset("Mine".into()), &mut s, &mut session),
            Some(Effect::LoadPreset("Mine".into()))
        );
        assert_eq!(
            apply(&Action::SavePreset, &mut s, &mut session),
            Some(Effect::SavePreset)
        );
        assert_eq!(
            apply(&Action::DeletePreset("Mine".into()), &mut s, &mut session),
            Some(Effect::DeletePreset("Mine".into()))
        );
        // None of that touched the settings.
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn cycling_walks_every_palette_and_style() {
        let mut s = Settings::default();
        let mut session = Session::default();
        let mut seen = vec![s.palette];
        for _ in 1..PaletteKind::ALL.len() {
            apply(&Action::NextPalette, &mut s, &mut session);
            assert!(
                !seen.contains(&s.palette),
                "{:?} came round early",
                s.palette
            );
            seen.push(s.palette);
        }
        apply(&Action::NextPalette, &mut s, &mut session);
        assert_eq!(s.palette, seen[0]);

        for _ in 0..CurveStyle::ALL.len() {
            apply(&Action::NextStyle, &mut s, &mut session);
        }
        assert_eq!(s.style, Settings::default().style);
    }

    #[test]
    fn help_search_finds_a_command_by_its_words() {
        let s = Settings::default();
        let session = Session::default();
        let items = tree(&context(&s, &session));
        let found = search(&items, "harmonic");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].action, Action::Toggle(Flag::ShowHarmonics));
        // The key is searchable too, and finds the same item the key itself performs.
        let by_key = search(&items, "F11");
        assert_eq!(by_key.len(), 1);
        assert_eq!(by_key[0].action, Action::Fullscreen);
        assert_eq!(search(&items, "").len(), entries(&items).len());
        assert!(search(&items, "zzzz").is_empty());
    }

    #[test]
    fn every_command_explains_itself() {
        let s = Settings::default();
        let session = Session::default();
        let items = tree(&context(&s, &session));
        for entry in entries(&items) {
            assert!(!entry.path.is_empty(), "an item has no label");
            // A disabled placeholder is the one thing with nothing to explain.
            if entry.path.contains("No ") || entry.path.contains("Nothing saved") {
                continue;
            }
            assert!(!entry.help.is_empty(), "{} has no help line", entry.path);
        }
    }

    #[test]
    fn the_nearest_size_is_the_one_marked() {
        // 18% is offered; a preset that moved it to 26 marks 26, and an odd value from
        // an old file marks whichever entry is closest.
        let s = Settings {
            curve_width_pct: 23,
            ..Settings::default()
        };
        let session = Session::default();
        let items = tree(&context(&s, &session));
        let marked: Vec<String> = flatten(&items)
            .iter()
            .filter(|i| {
                matches!(
                    i.kind,
                    Kind::Radio(Action::Set(Number::CurveWidthPct, _), true)
                )
            })
            .map(|i| i.label.clone())
            .collect();
        assert_eq!(marked, vec!["26% of the pane".to_owned()]);
    }

    #[test]
    fn a_player_that_cannot_be_asked_is_greyed_out() {
        let s = Settings::default();
        let session = Session::default();
        let mut ctx = context(&s, &session);
        ctx.controls = Controls::default();
        let items = tree(&ctx);
        let player = flatten(&items)
            .into_iter()
            .find(|i| i.label == "Next track")
            .expect("the next-track item");
        assert!(!player.enabled);
    }
}
