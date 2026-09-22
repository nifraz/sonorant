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
    AxisLabelMode, CameraView, CurveStyle, Flag, FrameCap, GraphBackground, Number, Preset,
    RenderQuality, ScaleLanePosition, Settings,
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
    Render(RenderQuality),
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
            Choice::Render(q) => q.name(),
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
                ScaleLanePosition::Both => "Both ends",
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
            Choice::Render(v) => s.render_quality == v,
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
            Choice::Render(v) => s.render_quality = v,
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
    /// Put the image back on now, at the plain zoom, after the wheel or a drag has
    /// taken it back through the history.
    GoLive,
    /// Put the waterfall's camera somewhere to start from.
    SetCamera(CameraView),
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
    /// Put the right-click menu away. It does nothing else: the menu is a thing the UI
    /// holds open, so closing it is the UI's to do, and this is the item that asks.
    CloseMenu,
    Quit,
}

impl Action {
    /// Whether choosing this in the right-click menu should put the menu away.
    ///
    /// The menu is as much a settings panel as a menu: a switch, a palette or a step of
    /// a number leaves it open, so the next thing can be tried against what the last
    /// one did, and the picture changes behind it while it stands there. What closes it
    /// is what takes over from it: a window that would otherwise open behind the menu,
    /// a dialog waiting to be answered, and the two ways out.
    pub fn closes_menu(&self) -> bool {
        matches!(
            self,
            Action::Help
                | Action::SavePreset
                | Action::LoadPreset(_)
                | Action::DeletePreset(_)
                | Action::CloseMenu
                | Action::Quit
        )
    }
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
    /// Whether the image is parked somewhere in the history rather than on now, so the
    /// menu and the quick bar can offer the way back. The app owns where it is looking
    /// and sets this; nothing in here writes it.
    pub parked: bool,
    /// Set when the image should go back to now. The app takes it each frame, the way
    /// it takes `command`, and puts the view and the zoom back itself.
    pub go_live: bool,
    /// Where the waterfall's camera has been asked to go, taken by the app each frame.
    /// A place to start from rather than a state, so nothing here remembers it.
    pub camera: Option<CameraView>,
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
    /// What the system says the sound still has to travel after capture taps it, in
    /// milliseconds, where it says anything at all.
    pub reported_delay_ms: Option<f64>,
}

/// One value a [`Kind::Choice`] or a [`Kind::Number`] offers.
///
/// The menu draws a pick as a radio button in a submenu, which is what it has always
/// looked like. Help draws the whole group as one row instead, because a list of 257
/// commands in which four of them are "Which pair of channels the panes show" is not a
/// list anybody reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub label: String,
    pub help: &'static str,
    pub action: Action,
    pub chosen: bool,
    pub enabled: bool,
}

/// What an item is.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// Does something and closes the menu.
    Command(Action),
    /// A switch, with what it is now.
    Check(Action, bool),
    /// One of a group, with whether it is the one chosen. For a radio that sits among
    /// other kinds of item; a group of its own is a [`Kind::Choice`].
    Radio(Action, bool),
    /// A whole group of values, one of them chosen.
    Choice(Vec<Pick>),
    /// A number: what it is now, and the values worth coming back to.
    ///
    /// The picks are a shortcut, not the whole of it. Anything in [`Number::range`] can
    /// be set, by the slider the menu draws under them or by the steppers beside it,
    /// because a figure that suits one room or one pair of ears is rarely one of five
    /// round numbers.
    Number(Number, f64, Vec<Pick>),
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
    ///
    /// A choice or a number has none of its own: what happens depends on which value is
    /// picked or where the slider is put, so those are on the [`Pick`]s and on
    /// [`Action::Set`].
    pub fn action(&self) -> Option<&Action> {
        match &self.kind {
            Kind::Command(a) | Kind::Check(a, _) | Kind::Radio(a, _) => Some(a),
            Kind::Choice(_) | Kind::Number(..) | Kind::Submenu(_) | Kind::Separator => None,
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

/// One value of a group, ready to draw.
fn pick(label: impl Into<String>, help: &'static str, action: Action, chosen: bool) -> Pick {
    Pick {
        label: label.into(),
        help,
        action,
        chosen,
        enabled: true,
    }
}

/// Every value of a choice, with the current one marked.
fn choices<T: Copy>(
    label: &'static str,
    help: &'static str,
    values: impl IntoIterator<Item = T>,
    wrap: impl Fn(T) -> Choice,
    item_help: &'static str,
    s: &Settings,
) -> Item {
    let picks = values
        .into_iter()
        .map(|v| {
            let c = wrap(v);
            pick(c.label(), item_help, Action::Choose(c), c.is_current(s))
        })
        .collect();
    Item::of(label, help, Kind::Choice(picks))
}

/// A choice whose values carry their own help line, for the ones worth explaining
/// apart: a palette reads differently from the next palette along, and saying so once
/// per value beats one sentence covering all thirteen.
fn choices_each<T: Copy>(
    label: &'static str,
    help: &'static str,
    values: impl IntoIterator<Item = T>,
    wrap: impl Fn(T) -> Choice,
    item_help: impl Fn(T) -> &'static str,
    s: &Settings,
) -> Item {
    let picks = values
        .into_iter()
        .map(|v| {
            let c = wrap(v);
            pick(c.label(), item_help(v), Action::Choose(c), c.is_current(s))
        })
        .collect();
    Item::of(label, help, Kind::Choice(picks))
}

/// A number: the values worth coming back to, and the whole range between them.
///
/// A pick is marked only when the value really is that value. It used to mark whichever
/// was nearest, so that a figure from a preset or an old file didn't leave the group
/// with nothing ticked, and that is no longer the kinder answer: the slider under the
/// picks says where the value is, and a tick on 3.0 while the slider reads 3.4 is a
/// disagreement between two things on the same screen.
fn sizes(
    label: &'static str,
    help: &'static str,
    n: Number,
    steps: &[f64],
    fmt: impl Fn(f64) -> String,
    s: &Settings,
) -> Item {
    let now = s.number(n);
    let nearest = steps
        .iter()
        .copied()
        .min_by(|a, b| (a - now).abs().total_cmp(&(b - now).abs()))
        .filter(|v| (v - now).abs() < f64::EPSILON.max(n.range().step / 2.0));
    let picks = steps
        .iter()
        .map(|&v| pick(fmt(v), help, Action::Set(n, v), nearest == Some(v)))
        .collect();
    Item::of(label, help, Kind::Number(n, now, picks))
}

/// Builds the whole menu for the state in `ctx`.
///
/// The order is what you reach for, not what the settings struct happens to list. The
/// presets come first because one of them is often the whole answer; then what is being
/// listened to; then the seven groups that are the picture itself; then the things done
/// to a running picture, which carry the keys; then the app's own three. `Close menu`
/// is last and alone, because it is the way out rather than another thing the menu does.
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
        Item::of(
            "Capture",
            "Which sound is analysed",
            Kind::Choice(
                Capture::ALL
                    .iter()
                    .map(|&c| {
                        pick(
                            c.label(),
                            "Analyse the followed player alone, or the whole mix",
                            Action::SetCapture(c),
                            session.capture == c,
                        )
                    })
                    .collect(),
            ),
        ),
        Item::submenu(
            "Playback",
            "Which player is followed, and what to ask it to do",
            playback(ctx),
        ),
        Item::separator(),
        Item::submenu("Analysis", "How the sound is measured", analysis(s)),
        Item::submenu(
            "Spectrogram",
            "The scrolling image: its colours, its axis and how fast it runs",
            spectrogram(s),
        ),
        Item::submenu("Spectrum", "The curve strip beside each image", spectrum(s)),
        Item::submenu(
            "Scales and labels",
            "The grid, the scales, their text and what the pointer reads out",
            axes(s),
        ),
        Item::submenu(
            "Panels",
            "The deck, the lanes, the quick bar and the other furniture",
            panels(s),
        ),
        Item::submenu(
            "Look and immersion",
            "The full-screen treatment: glow, backdrop and the beat",
            immersive(s),
        ),
        Item::submenu(
            "Performance",
            "Frame rate, how much the new visuals may do, and the visual delay",
            performance(ctx),
        ),
        Item::separator(),
        Item::check(
            "Freeze",
            "Hold the picture still; analysis carries on",
            Action::Freeze,
            session.frozen,
        )
        .key("Space"),
        Item::command(
            "Live",
            "Put the image back on now, at the plain zoom",
            Action::GoLive,
        )
        .key("End"),
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
        Item::command("Quit Sonorant", "Stop the program", Action::Quit),
        Item::separator(),
        // Last, on its own, because it is the way out of the menu rather than another
        // thing the menu does. Esc does the same; it carries no key here because Esc
        // already belongs to leaving fullscreen, and a key answers to one item.
        Item::command(
            "Close menu",
            "Put the menu away. Esc does the same, as does clicking outside it",
            Action::CloseMenu,
        ),
    ]
}

/// The player: which one is followed, and the three things it can be asked.
///
/// One group, because they are one subject. Following and asking were two top-level
/// submenus with nine others between them, which is a strange place to leave the two
/// items that are about the same player.
fn playback(ctx: &Context<'_>) -> Vec<Item> {
    let mut items = vec![follow(ctx), Item::separator()];
    items.extend(transport(ctx));
    items
}

/// Frame rate, presentation, how much the new visuals may do, and the visual delay.
fn performance(ctx: &Context<'_>) -> Vec<Item> {
    let s = ctx.settings;
    vec![
        choices(
            "Redraw",
            "How often the screen is redrawn; scrolling follows audio time either way",
            FrameCap::ALL.iter().copied(),
            Choice::Cap,
            "How often the screen is redrawn",
            s,
        ),
        Item::of(
            "Presentation",
            "How finished frames reach the screen",
            Kind::Choice(
                Presentation::ALL
                    .iter()
                    .map(|&p| Pick {
                        enabled: ctx.presentations.contains(&p),
                        ..pick(
                            p.label(),
                            match p {
                                Presentation::EveryRefresh => {
                                    "Queue every frame for the next refresh: no tearing"
                                }
                                Presentation::Newest => {
                                    "Keep only the newest frame queued: no tearing, least delay"
                                }
                                Presentation::Immediate => {
                                    "Show a frame the moment it is drawn, tearing included"
                                }
                            },
                            Action::SetPresentation(p),
                            session_presentation(ctx) == p,
                        )
                    })
                    .collect(),
            ),
        ),
        choices(
            "Visual quality",
            "How much work the waterfall, the glow and the backdrop may do",
            RenderQuality::ALL.iter().copied(),
            Choice::Render,
            "Lower is cheaper on a weak GPU; Medium is what the app has always drawn",
            s,
        ),
        Item::separator(),
        Item::submenu(
            "Visual delay",
            "Hold the picture back so it lines up with what you hear",
            visual_delay(ctx),
        ),
    ]
}

fn session_presentation(ctx: &Context<'_>) -> Presentation {
    ctx.session.presentation
}

/// The visual delay: the automatic figure where there is one, and the offsets by hand.
///
/// Capture taps the mix before the hardware plays it, so the picture is early by
/// whatever the output path costs. The automatic item carries the number the system
/// reports, because "Automatic" alone tells you nothing about whether it found
/// anything.
fn visual_delay(ctx: &Context<'_>) -> Vec<Item> {
    let s = ctx.settings;
    let automatic = match ctx.reported_delay_ms {
        Some(ms) => Item::check(
            format!("Automatic ({ms:.0} ms)"),
            "Follow the delay the system reports, and keep following it when the              output changes",
            Action::Toggle(Flag::AutoVisualDelay),
            s.flag(Flag::AutoVisualDelay),
        ),
        None => Item::check(
            "Automatic",
            "Nothing here reports the output's delay, so the offset has to be set by              hand",
            Action::Toggle(Flag::AutoVisualDelay),
            s.flag(Flag::AutoVisualDelay),
        )
        .when(false),
    };
    vec![
        automatic,
        Item::separator(),
        // Set by hand rather than stepped, because the way to find the right figure is
        // to watch a drum against the picture and try another one, not to creep up on
        // it a step at a time.
        sizes(
            "Offset",
            "How long the picture is held back, so it lines up with the sound",
            Number::VisualDelayMs,
            &[0.0, 20.0, 40.0, 80.0, 150.0, 300.0],
            |v| {
                if v == 0.0 {
                    "None".to_owned()
                } else {
                    format!("{v:.0} ms")
                }
            },
            s,
        ),
    ]
}

fn presets(ctx: &Context<'_>) -> Vec<Item> {
    let s = ctx.settings;
    // Grouped by what you would be doing, not by who wrote them: the shipped settings,
    // then the ones for watching, then the ones for a kind of material, then the ones
    // for work. `None` is a rule off between two groups.
    let built_in = [
        Some((
            Preset::Default,
            "The settings the app ships with: the same as resetting everything",
        )),
        None,
        Some((
            Preset::Studio,
            "A musical axis and a gentle tilt: the everyday view",
        )),
        Some((Preset::Immersive, "For watching rather than measuring")),
        Some((
            Preset::Club,
            "Short windows, a fast scroll and every immersive trick: for dancing to",
        )),
        None,
        Some((
            Preset::Vocal,
            "Where voices live, with the detail to separate formants",
        )),
        Some((
            Preset::Speech,
            "Talk: wider at the bottom than Vocal, and quick enough for consonants",
        )),
        Some((
            Preset::Bass,
            "A narrow low range, with the largest transforms",
        )),
        Some((
            Preset::Percussion,
            "Short windows, so transients land where they are heard",
        )),
        Some((
            Preset::Classical,
            "Long, slow and quiet: a floor deep enough to hold a real pianissimo",
        )),
        None,
        Some((
            Preset::QC,
            "Flat, wide and linear, for spotting a lossy source",
        )),
        Some((Preset::Mastering, "Nothing tilted or adaptive: measurement")),
        Some((
            Preset::Broadcast,
            "A fixed window to full scale, with the deck showing the loudness figures",
        )),
        None,
        Some((
            Preset::Nostalgia,
            "The original plugin's look, without its mapping bugs",
        )),
    ];
    let mut items: Vec<Item> = built_in
        .iter()
        .map(|entry| match *entry {
            Some((p, help)) => Item::radio(p.name(), help, Action::UsePreset(p), s.preset == p),
            None => Item::separator(),
        })
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

fn follow(ctx: &Context<'_>) -> Item {
    let mut picks = vec![pick(
        "Whichever is playing",
        "Follow whichever player is playing, preferring the one that started last",
        Action::SetFollow(Follow::Whichever),
        ctx.session.follow == Follow::Whichever,
    )];
    for player in ctx.players {
        // Pinning is by id, because two windows of the same app share a name.
        let pinned = Follow::Pinned(player.id.clone());
        let chosen = ctx.session.follow == pinned;
        picks.push(pick(
            player.name.clone(),
            "Follow this player, whatever else starts",
            Action::SetFollow(pinned),
            chosen,
        ));
    }
    Item::of(
        "Follow player",
        if ctx.players.is_empty() {
            "Which player is followed. None is running, so there is only the one entry"
        } else {
            "Which player the deck and capture follow"
        },
        Kind::Choice(picks),
    )
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

fn spectrum(s: &Settings) -> Vec<Item> {
    vec![
        sizes(
            "Width",
            "How much of each pane the curve strip takes",
            Number::CurveWidthPct,
            &[0.0, 10.0, 18.0, 26.0, 33.0, 40.0, 50.0, 60.0],
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
        sizes(
            "Peak decay",
            "How fast the held maximum lets go of a peak",
            Number::PeakDecay,
            &[0.0, 6.0, 14.0, 24.0, 40.0, 60.0],
            |v| {
                if v == 0.0 {
                    "Never".to_owned()
                } else {
                    format!("{v:.0} dB a second")
                }
            },
            s,
        ),
        flag(
            "Average trace",
            "The running average of the spectrum",
            Flag::ShowAvg,
            s,
        ),
        sizes(
            "Average window",
            "How long the average trace takes to follow a change",
            Number::AverageSeconds,
            &[0.2, 0.6, 1.2, 3.0, 6.0, 10.0],
            |v| format!("{v:.1} s"),
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
        flag(
            "Phosphor curve",
            "Let the line smear where it moves, sharing the scope's persistence",
            Flag::CurvePhosphor,
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
            &[1.0, 2.0, 3.0, 4.0, 6.0, 10.0, 16.0, 24.0, 32.0],
            |v| format!("{v:.0} px"),
            s,
        ),
        sizes(
            "LED segment",
            "How tall one lit segment is in the LED style",
            Number::LedSegment,
            &[2.0, 3.0, 5.0, 8.0, 12.0, 20.0, 32.0],
            |v| format!("{v:.0} px"),
            s,
        ),
    ]
}

fn spectrogram(s: &Settings) -> Vec<Item> {
    vec![
        choices_each(
            "Palette",
            "The colours levels are drawn in",
            PaletteKind::ALL.iter().copied(),
            Choice::Palette,
            PaletteKind::about,
            s,
        ),
        Item::command(
            "Next palette",
            "Move on to the next palette",
            Action::NextPalette,
        )
        .key("C"),
        Item::separator(),
        choices(
            "Frequency axis",
            "How frequency is spread across the axis",
            FreqScale::ALL.iter().copied(),
            Choice::Scale,
            "How frequency is spread across the axis",
            s,
        ),
        sizes(
            "Lowest frequency",
            "Where the axis starts",
            Number::Fmin,
            &[0.0, 20.0, 30.0, 40.0, 60.0, 80.0, 150.0, 300.0],
            |v| format!("{v:.0} Hz"),
            s,
        ),
        sizes(
            "Highest frequency",
            "Where the axis ends",
            Number::Fmax,
            &[
                4000.0, 8000.0, 10000.0, 12000.0, 16000.0, 18000.0, 20000.0, 22050.0,
            ],
            |v| format!("{:.1} kHz", v / 1000.0),
            s,
        ),
        Item::separator(),
        sizes(
            "Scroll speed",
            "How many rows of history a second of sound becomes",
            Number::RowsPerSecond,
            &[10.0, 15.0, 30.0, 45.0, 60.0, 90.0, 120.0, 180.0, 240.0],
            |v| format!("{v:.0} rows a second"),
            s,
        ),
        sizes(
            "Zoom",
            "How many screen pixels one row is drawn across",
            Number::PxPerRow,
            &[1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0],
            |v| format!("{v:.0} px a row"),
            s,
        ),
        sizes(
            "History length",
            "How far back the wheel and a drag can reach, as far as memory allows",
            Number::HistoryMinutes,
            &[1.0, 3.0, 5.0, 10.0, 15.0, 30.0, 60.0],
            |v| format!("{v:.0} min"),
            s,
        ),
        flag(
            "Blend between rows",
            "Fade between rows rather than stepping, so slow scrolling stays smooth",
            Flag::SmoothTime,
            s,
        ),
        Item::separator(),
        flag(
            "Waterfall",
            "Draw the history as a landscape, with a camera you can orbit",
            Flag::Waterfall,
            s,
        )
        .key("3"),
        Item::submenu(
            "Camera",
            "Where to put the waterfall's camera",
            CameraView::ALL
                .iter()
                .map(|&v| {
                    Item::command(
                        v.name(),
                        match v {
                            CameraView::Classic => "Down the history from a little above it",
                            CameraView::Overhead => "Nearly overhead, close to the flat view",
                            CameraView::Side => "From one side, where the ridges show their shape",
                            CameraView::Low => "Along the surface, with the loud rows standing up",
                        },
                        Action::SetCamera(v),
                    )
                })
                .collect(),
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
            "Channel names",
            "The L and R written in the corner of each pane",
            Flag::ShowLabels,
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
        choices_each(
            "Strip at",
            "Which end of the panes the scale strip sits at, or both",
            ScaleLanePosition::ALL.iter().copied(),
            Choice::ScaleLane,
            |v| match v {
                ScaleLanePosition::Top => "Over the image",
                ScaleLanePosition::Bottom => "Under the image",
                ScaleLanePosition::Both => {
                    "At both ends, repeated rather than split, for twice the height"
                }
            },
            s,
        ),
        flag(
            "Unit captions",
            "The words under the scales saying what they measure",
            Flag::ShowScaleUnits,
            s,
        ),
        Item::separator(),
        sizes(
            "Text size",
            "How large the axis, scale and readout text is",
            Number::LabelFontSize,
            &[5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 12.0, 14.0, 18.0, 22.0, 28.0],
            |v| format!("{v:.0} pt"),
            s,
        ),
        sizes(
            "Centre gutter",
            "How wide the labelled strip between the panes is",
            Number::GutterWidth,
            &[0.0, 16.0, 24.0, 34.0, 48.0, 64.0, 90.0, 120.0],
            |v| {
                if v == 0.0 {
                    "None".to_owned()
                } else {
                    format!("{v:.0} px")
                }
            },
            s,
        ),
        Item::separator(),
        Item::submenu(
            "Hover readout",
            "What the pointer reads out of the picture",
            hover(s),
        ),
    ]
}

fn hover(s: &Settings) -> Vec<Item> {
    vec![
        flag(
            "Readout box",
            "The panel of figures beside the pointer. The crosshair and the pin stay",
            Flag::ShowHud,
            s,
        ),
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

/// The furniture around the picture: the master switch, then one submenu per panel.
///
/// The colour bar and the status line have no submenu of their own because they are one
/// switch each, and a submenu holding a single item is a door to a cupboard.
fn panels(s: &Settings) -> Vec<Item> {
    vec![
        flag(
            "On-screen readouts",
            "Everything drawn over the image: the hover readout, the quick bar and the \
             status line",
            Flag::ShowOsd,
            s,
        )
        .key("O"),
        Item::separator(),
        Item::submenu(
            "Centre deck",
            "The panel of figures between the panes",
            deck(s),
        ),
        Item::submenu(
            "Waveform lanes",
            "The level against time along the bottom",
            waveform(s),
        ),
        Item::submenu(
            "Quick bar",
            "The row of buttons over the image",
            quick_bar(s),
        ),
        Item::separator(),
        flag(
            "Colour bar",
            "The level-to-colour key down the right edge",
            Flag::ShowColourBar,
            s,
        ),
        flag(
            "Status line",
            "The line of figures over the image: capture, loudness, frame rate and lag",
            Flag::ShowStatus,
            s,
        ),
    ]
}

fn waveform(s: &Settings) -> Vec<Item> {
    vec![
        flag(
            "Show the lanes",
            "The level against time along the bottom",
            Flag::ShowWaveform,
            s,
        )
        .key("W"),
        sizes(
            "Height",
            "How much of the view the lanes take",
            Number::WaveHeightPct,
            &[0.0, 4.0, 6.0, 10.0, 16.0, 24.0, 32.0, 40.0],
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
            "Show the deck",
            "The panel of figures between the panes",
            Flag::ShowCentreDeck,
            s,
        ),
        sizes(
            "Height",
            "How tall the deck and the lanes beside it are",
            Number::DeckHeightPx,
            &[60.0, 80.0, 100.0, 120.0, 160.0, 200.0, 280.0, 400.0],
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
            "Phosphor scope",
            "Let the goniometer's trace fade like a CRT rather than blink each frame",
            Flag::DeckPhosphor,
            s,
        ),
        sizes(
            "Phosphor persistence",
            "How long the trace takes to fade away",
            Number::PhosphorMs,
            &[60.0, 120.0, 250.0, 500.0, 750.0, 1000.0, 1500.0, 2000.0],
            |v| {
                if v >= 1000.0 {
                    format!("{:.1} s", v / 1000.0)
                } else {
                    format!("{v:.0} ms")
                }
            },
            s,
        ),
        sizes(
            "Phosphor intensity",
            "How hard the trace is written",
            Number::PhosphorIntensity,
            &[10.0, 40.0, 70.0, 100.0, 150.0, 220.0, 300.0],
            |v| format!("{v:.0}%"),
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
            "Show the bar",
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
        Action::Set(n, v) => {
            s.set_number(*n, *v);
            chose_delay_by_hand(*n, s);
        }
        Action::Step(n, by) => {
            s.step_number(*n, *by);
            chose_delay_by_hand(*n, s);
        }
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
        Action::GoLive => {
            // Unfreezing is how the app is told to follow the newest row again, and
            // `go_live` is what puts the zoom back with it.
            session.frozen = false;
            session.go_live = true;
            touched = false;
        }
        Action::SetCamera(view) => {
            session.camera = Some(*view);
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
        // Nothing to apply: the UI closes its own popup when it sees this go past.
        Action::CloseMenu => touched = false,
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

/// Choosing the visual delay by hand is choosing it, so the automatic figure stops
/// writing over it.
///
/// Without this, picking a number from the menu would hold only until the sink's
/// latency next moved, which on a machine that reports it is whenever the output
/// changes. A switch that quietly undoes what the item beside it just did is worse
/// than no switch.
fn chose_delay_by_hand(n: Number, s: &mut Settings) {
    if n == Number::VisualDelayMs {
        s.auto_visual_delay = false;
    }
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

/// What an entry does, so help can show the state beside it rather than only the name.
#[derive(Clone, Debug, PartialEq)]
pub enum EntryKind {
    /// Something that happens once.
    Command,
    /// A switch, and whether it is on.
    Check(bool),
    /// One of a set, and whether it is the one chosen. A radio that sits among other
    /// kinds of item; a whole group is one [`EntryKind::Choice`] row.
    Radio(bool),
    /// A whole group, as one row: help draws the values in a dropdown rather than
    /// spending a row on each of them.
    Choice(Vec<Pick>),
    /// A number, where it is, and the values worth coming back to: a slider, with
    /// those beside it.
    Number(Number, f64, Vec<Pick>),
}

impl EntryKind {
    /// What the row reads as now: a switch's state, a choice's value, a number and its
    /// unit, or nothing for a command.
    pub fn value(&self) -> Option<String> {
        match self {
            EntryKind::Command => None,
            EntryKind::Check(on) => Some(if *on { "on" } else { "off" }.to_owned()),
            EntryKind::Radio(chosen) => {
                Some(if *chosen { "chosen" } else { "not chosen" }.to_owned())
            }
            EntryKind::Choice(picks) => Some(
                picks
                    .iter()
                    .find(|p| p.chosen)
                    .map_or_else(|| "—".to_owned(), |p| p.label.clone()),
            ),
            EntryKind::Number(n, v, _) => Some(written(*n, *v)),
        }
    }
}

/// A number written out with its unit, for a help row and a slider's own label.
pub fn written(n: Number, v: f64) -> String {
    let r = n.range();
    // Enough places to tell one step from the next, and then the noughts that buys
    // trimmed off again: a tilt of three is "3 dB/oct", and one of three and a quarter
    // is "3.25 dB/oct", rather than both carrying two decimals because one of them
    // needs them.
    let places = if r.whole || r.step >= 1.0 {
        0
    } else if r.step >= 0.1 {
        1
    } else {
        2
    };
    let mut figure = format!("{v:.places$}");
    if figure.contains('.') {
        figure = figure
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned();
    }
    if r.unit.is_empty() {
        figure
    } else {
        format!("{figure} {}", r.unit)
    }
}

/// One command as the help window lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Where it is, as "Graph > Peak trace".
    pub path: String,
    pub help: &'static str,
    pub shortcut: Option<&'static str>,
    pub action: Action,
    /// What it is, and the state it is in. Help shows a switch as on or off and a
    /// choice as chosen or not, which makes the window a reading of the settings as
    /// well as a list of what can be done to them.
    pub kind: EntryKind,
    /// Greyed out in the menu, so greyed out here, and its key does nothing.
    pub enabled: bool,
    /// What would put this setting back to the value the app ships with, or `None`
    /// where it is already there or is not a setting at all.
    ///
    /// Held rather than worked out by whatever draws the row, so "what have I changed?"
    /// has one answer and help's filter and its revert button cannot disagree.
    pub reset: Option<Action>,
}

/// The top-level items, which belong to no submenu, grouped under this heading.
pub const GENERAL: &str = "General";

impl Entry {
    /// The submenu it sits in, as a heading: "Graph" for "Graph > Peak trace", and
    /// [`GENERAL`] for an item at the top of the menu.
    pub fn section(&self) -> &str {
        match self.path.split_once(" > ") {
            Some((section, _)) => section,
            None => GENERAL,
        }
    }

    /// Its own label, without the path that leads to it.
    pub fn leaf(&self) -> &str {
        match self.path.rsplit_once(" > ") {
            Some((_, leaf)) => leaf,
            None => self.path.as_str(),
        }
    }

    /// The values this row offers, where it offers any.
    pub fn picks(&self) -> &[Pick] {
        match &self.kind {
            EntryKind::Choice(picks) | EntryKind::Number(_, _, picks) => picks,
            _ => &[],
        }
    }

    /// Whether `query`, already lowercased and trimmed, is anywhere in this row.
    ///
    /// The values are searched as well as the row, because a choice is one row now and
    /// "Viridis" has to find the palette. The key is matched by part rather than whole:
    /// "f1" finds help, and so does "f".
    pub fn matches(&self, query: &str) -> bool {
        query.is_empty()
            || self.path.to_lowercase().contains(query)
            || self.help.to_lowercase().contains(query)
            || self
                .shortcut
                .is_some_and(|k| k.to_lowercase().contains(query))
            || self.picks().iter().any(|p| {
                p.label.to_lowercase().contains(query) || p.help.to_lowercase().contains(query)
            })
    }

    /// Whether this row differs from what the app ships with.
    pub fn changed(&self) -> bool {
        self.reset.is_some()
    }
}

/// Every command in the tree, with the path that leads to it.
///
/// A choice or a number is one entry, not one per value: the menu spends a submenu of
/// radios on it because that is how a menu picks a value, and a flat list of 257 rows
/// in which four of them read "Which pair of channels the panes show" is not a list.
/// The values ride along on the entry, so searching still finds them by name.
pub fn entries(items: &[Item]) -> Vec<Entry> {
    let mut out = Vec::new();
    let shipped = Settings::default();
    walk(items, "", &shipped, &mut out);
    out
}

fn walk(items: &[Item], prefix: &str, shipped: &Settings, out: &mut Vec<Entry>) {
    let under = |label: &str| {
        if prefix.is_empty() {
            label.to_owned()
        } else {
            format!("{prefix} > {label}")
        }
    };
    for item in items {
        match &item.kind {
            Kind::Separator => {}
            Kind::Submenu(children) => walk(children, &under(&item.label), shipped, out),
            Kind::Choice(picks) => {
                // Enter on the row re-picks what is already picked, which is the honest
                // answer: the row is a reading, and the dropdown is what changes it.
                let chosen = picks.iter().find(|p| p.chosen);
                out.push(Entry {
                    path: under(&item.label),
                    help: item.help,
                    shortcut: item.shortcut,
                    action: chosen.map_or(Action::CloseMenu, |p| p.action.clone()),
                    kind: EntryKind::Choice(picks.clone()),
                    enabled: item.enabled,
                    reset: shipped_pick(picks, shipped)
                        .filter(|p| !p.chosen)
                        .map(|p| p.action.clone()),
                });
            }
            // The named values ride along rather than becoming rows of their own, so
            // searching for one by name still lands on the setting that offers it.
            Kind::Number(n, value, picks) => out.push(Entry {
                path: under(&item.label),
                help: item.help,
                shortcut: item.shortcut,
                // Setting it to what it is: running the row does nothing, because the
                // slider beside it is what moves a number.
                action: Action::Set(*n, *value),
                kind: EntryKind::Number(*n, *value, picks.clone()),
                enabled: item.enabled,
                reset: (shipped.number(*n) != *value).then(|| Action::Set(*n, shipped.number(*n))),
            }),
            Kind::Command(a) | Kind::Check(a, _) | Kind::Radio(a, _) => out.push(Entry {
                path: under(&item.label),
                help: item.help,
                shortcut: item.shortcut,
                action: a.clone(),
                kind: match &item.kind {
                    Kind::Check(_, on) => EntryKind::Check(*on),
                    Kind::Radio(_, chosen) => EntryKind::Radio(*chosen),
                    _ => EntryKind::Command,
                },
                enabled: item.enabled,
                reset: match (&item.kind, a) {
                    (Kind::Check(_, on), Action::Toggle(f)) if *on != shipped.flag(*f) => {
                        Some(Action::Toggle(*f))
                    }
                    _ => None,
                },
            }),
        }
    }
}

/// The value of a group the app ships with, where the group is one of the settings.
///
/// Capture, the followed player and the presentation mode are choices too, and none of
/// them is a setting with a shipped value, so they have nothing to be put back to.
fn shipped_pick<'a>(picks: &'a [Pick], shipped: &Settings) -> Option<&'a Pick> {
    picks.iter().find(|p| match &p.action {
        Action::Choose(c) => c.is_current(shipped),
        _ => false,
    })
}

/// The commands whose path, help or key matches `query`, in the order they appear in the
/// menu. An empty query is everything.
///
/// The key is matched the same way as the rest, by part rather than whole: "f1" finds
/// help, and so does "f". Matching a key only when the whole of it was typed made the
/// key column the one part of the window that searching didn't reach.
pub fn search(items: &[Item], query: &str) -> Vec<Entry> {
    let q = query.trim().to_lowercase();
    entries(items)
        .into_iter()
        .filter(|e| e.matches(&q))
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
            reported_delay_ms: None,
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
    fn setting_the_visual_delay_by_hand_stops_it_following_the_system() {
        // Following the system is what this is about, so it starts there whatever the
        // app ships with.
        let mut s = Settings {
            auto_visual_delay: true,
            ..Settings::default()
        };
        let mut session = Session::default();
        // Another number leaves the automatic alone.
        apply(
            &Action::Set(Number::PhosphorMs, 200.0),
            &mut s,
            &mut session,
        );
        assert!(s.auto_visual_delay);
        apply(
            &Action::Set(Number::VisualDelayMs, 80.0),
            &mut s,
            &mut session,
        );
        assert_eq!(s.visual_delay_ms, 80);
        assert!(!s.auto_visual_delay);
        // And so does stepping it, which is the same choice made a nudge at a time.
        s.auto_visual_delay = true;
        apply(
            &Action::Step(Number::VisualDelayMs, 1.0),
            &mut s,
            &mut session,
        );
        assert!(!s.auto_visual_delay);
        // The switch itself is still a switch: it can be put back on.
        apply(&Action::Toggle(Flag::AutoVisualDelay), &mut s, &mut session);
        assert!(s.auto_visual_delay);
    }

    #[test]
    fn the_automatic_delay_says_what_it_found() {
        let s = Settings::default();
        let session = Session::default();
        let labels = |ctx: &Context<'_>| {
            let items = tree(ctx);
            let Some(Kind::Submenu(children)) = flatten(&items)
                .iter()
                .find(|i| i.label == "Visual delay")
                .map(|i| i.kind.clone())
            else {
                panic!("no visual delay menu")
            };
            children
                .iter()
                .map(|c| (c.label.clone(), c.enabled))
                .collect::<Vec<_>>()
        };
        // Nothing reported: the switch is there but greyed, so it is plain that the
        // figure is missing rather than nought.
        let mut ctx = context(&s, &session);
        assert_eq!(labels(&ctx)[0], ("Automatic".to_owned(), false));
        // A figure reported: the item carries it.
        ctx.reported_delay_ms = Some(21.8);
        assert_eq!(labels(&ctx)[0], ("Automatic (22 ms)".to_owned(), true));
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
            .find(|e| e.path == "Scales and labels > Grid")
            .expect("the grid switch");
        assert_eq!(grid.action, Action::Toggle(Flag::ShowGrid));
        // The palette in the settings is the one the menu marks, and the one its help
        // row reads as its value.
        let palette = found
            .iter()
            .find(|e| e.path == "Spectrogram > Palette")
            .expect("the palette choice");
        let chosen: Vec<&Pick> = palette.picks().iter().filter(|p| p.chosen).collect();
        assert_eq!(chosen.len(), 1);
        assert_eq!(chosen[0].label, PaletteKind::Turbo.display_name());
        assert_eq!(
            palette.kind.value().as_deref(),
            Some(PaletteKind::Turbo.display_name())
        );
    }

    #[test]
    fn a_setting_leaves_the_menu_open_and_a_window_closes_it() {
        let s = Settings::default();
        let session = Session::default();
        let items = tree(&context(&s, &session));

        // Everything in the whole tree that closes the menu, and nothing else: a
        // dialog to answer, a window that would open behind it, and the ways out. The
        // two preset placeholders are the greyed-out "nothing here yet" items.
        let closing: Vec<String> = entries(&items)
            .iter()
            .filter(|e| e.action.closes_menu())
            .map(|e| e.leaf().to_owned())
            .collect();
        assert_eq!(
            closing,
            [
                "No saved presets",
                "Save these settings...",
                "Nothing saved",
                "Help",
                "Quit Sonorant",
                "Close menu",
            ]
        );

        // A switch, a choice and a step all stay: the picture changes behind a menu
        // that is still standing, which is the point of them staying.
        for action in [
            Action::Toggle(Flag::ShowGrid),
            Action::Choose(Choice::Palette(PaletteKind::Turbo)),
            Action::Step(Number::BarSize, 1.0),
            Action::NextPalette,
            Action::GoLive,
            Action::ResetSettings,
            Action::SetCapture(Capture::WholeSystem),
        ] {
            assert!(!action.closes_menu(), "{action:?}");
        }
    }

    #[test]
    fn the_way_out_of_the_menu_is_its_own_item() {
        let s = Settings::default();
        let session = Session::default();
        let items = tree(&context(&s, &session));
        let last = items.last().expect("a last item");
        assert_eq!(last.label, "Close menu");
        assert_eq!(last.action(), Some(&Action::CloseMenu));
        // It is not the same item as quitting, and neither is worded as the other.
        let quit = items
            .iter()
            .find(|i| i.action() == Some(&Action::Quit))
            .expect("quit");
        assert_eq!(quit.label, "Quit Sonorant");
        assert!(!quit.help.contains("Close"), "{}", quit.help);

        // Closing the menu is the UI's doing, so the model changes nothing for it.
        let mut settings = Settings::default();
        let mut session = Session::default();
        let before = (settings.clone(), session.clone());
        assert_eq!(apply(&Action::CloseMenu, &mut settings, &mut session), None);
        assert_eq!((settings, session), before);

        // And Esc still belongs to leaving fullscreen: the new item carries no key, so
        // nothing was taken from the one that had it.
        let full = Session {
            fullscreen: true,
            ..Session::default()
        };
        let s = Settings::default();
        assert_eq!(
            action_for_key(&context(&s, &full), "Esc"),
            Some(Action::LeaveFullscreen)
        );
    }

    #[test]
    fn help_entries_carry_their_state_and_where_they_live() {
        let s = Settings {
            show_grid: true,
            palette: PaletteKind::Turbo,
            ..Settings::default()
        };
        let session = Session::default();
        let items = tree(&context(&s, &session));
        let found = entries(&items);

        let grid = found
            .iter()
            .find(|e| e.path == "Scales and labels > Grid")
            .expect("the grid switch");
        assert_eq!(grid.kind, EntryKind::Check(true));
        assert_eq!(grid.section(), "Scales and labels");
        assert_eq!(grid.leaf(), "Grid");
        assert!(grid.enabled);

        // A whole choice is one row, carrying every value it offers and which of them
        // is in use, rather than a row apiece.
        let palettes: Vec<&Entry> = found
            .iter()
            .filter(|e| matches!(e.kind, EntryKind::Choice(_)) && e.leaf() == "Palette")
            .collect();
        assert_eq!(palettes.len(), 1);
        assert_eq!(palettes[0].picks().len(), PaletteKind::ALL.len());
        let chosen: Vec<&str> = palettes[0]
            .picks()
            .iter()
            .filter(|p| p.chosen)
            .map(|p| p.label.as_str())
            .collect();
        assert_eq!(chosen, [PaletteKind::Turbo.display_name()]);
        // And searching still finds it by the value's own name.
        assert!(palettes[0].matches("turbo"));
        assert!(palettes[0].matches("viridis"));

        // An item at the top of the menu belongs to no submenu.
        let freeze = found
            .iter()
            .find(|e| e.action == Action::Freeze)
            .expect("freeze");
        assert_eq!(freeze.section(), GENERAL);
        assert_eq!(freeze.leaf(), "Freeze");
    }

    #[test]
    fn a_key_is_searched_for_by_part() {
        let s = Settings::default();
        let session = Session::default();
        let items = tree(&context(&s, &session));
        for query in ["F1", "f1", "f"] {
            let found = search(&items, query);
            assert!(
                found.iter().any(|e| e.action == Action::Help),
                "{query} found nothing with F1 on it"
            );
        }
        // A greyed-out item is still listed, and says so.
        let leave = search(&items, "Leave fullscreen");
        assert_eq!(leave.len(), 1);
        assert!(!leave[0].enabled);
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
            ("End", Action::GoLive),
            ("3", Action::Toggle(Flag::Waterfall)),
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
    fn a_changed_row_carries_what_puts_it_back() {
        let shipped = Settings::default();
        let mut s = shipped.clone();
        s.set_flag(Flag::ShowGrid, !shipped.show_grid);
        s.palette = if shipped.palette == PaletteKind::Turbo {
            PaletteKind::Viridis
        } else {
            PaletteKind::Turbo
        };
        s.set_number(Number::Tilt, 4.25);
        let session = Session::default();
        let items = tree(&context(&s, &session));
        let found = entries(&items);
        let at = |path: &str| {
            found
                .iter()
                .find(|e| e.path == path)
                .unwrap_or_else(|| panic!("{path}"))
        };

        // Three kinds of setting, each knowing its own way home.
        let back: Vec<Action> = [
            "Scales and labels > Grid",
            "Spectrogram > Palette",
            "Analysis > Tilt",
        ]
        .iter()
        .map(|p| {
            let e = at(p);
            assert!(e.changed(), "{p} should read as changed");
            e.reset
                .clone()
                .unwrap_or_else(|| panic!("{p} has no way back"))
        })
        .collect();
        let mut put_back = s.clone();
        let mut session = Session::default();
        for action in &back {
            apply(action, &mut put_back, &mut session);
        }
        assert_eq!(put_back.show_grid, shipped.show_grid);
        assert_eq!(put_back.palette, shipped.palette);
        assert_eq!(put_back.tilt_db_per_octave, shipped.tilt_db_per_octave);

        // Nothing that is already where it ships has one, and neither has anything that
        // is not a setting at all.
        let untouched = tree(&context(&shipped, &Session::default()));
        assert!(
            entries(&untouched).iter().all(|e| !e.changed()),
            "the shipped settings read as changed: {:?}",
            entries(&untouched)
                .iter()
                .filter(|e| e.changed())
                .map(|e| e.path.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_choice_is_one_row_and_a_number_is_one_row() {
        let s = Settings::default();
        let session = Session::default();
        let found = entries(&tree(&context(&s, &session)));
        // Every value of every group used to be a row of its own, which ran to well
        // over two hundred. One row each keeps the list skimmable.
        assert!(
            found.len() < 160,
            "the help list has grown back to {} rows",
            found.len()
        );
        // And no two rows share a path, which is what a collapsed group must not undo.
        let mut paths: Vec<&str> = found.iter().map(|e| e.path.as_str()).collect();
        paths.sort_unstable();
        let before = paths.len();
        paths.dedup();
        assert_eq!(paths.len(), before, "two rows share a path");
    }

    #[test]
    fn a_size_is_marked_only_when_it_is_the_value() {
        let marked = |pct: i32| {
            let s = Settings {
                curve_width_pct: pct,
                ..Settings::default()
            };
            let session = Session::default();
            let items = tree(&context(&s, &session));
            flatten(&items)
                .iter()
                .find_map(|i| match &i.kind {
                    Kind::Number(Number::CurveWidthPct, _, picks) => Some(
                        picks
                            .iter()
                            .filter(|p| p.chosen)
                            .map(|p| p.label.clone())
                            .collect::<Vec<_>>(),
                    ),
                    _ => None,
                })
                .expect("the curve width item")
        };
        assert_eq!(marked(26), vec!["26% of the pane".to_owned()]);
        // 23 is between two of the named widths, so neither claims it; the slider in
        // the same submenu is what says where it really is.
        assert!(marked(23).is_empty());
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
