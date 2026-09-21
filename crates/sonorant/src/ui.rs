//! The egui layer: the right-click menu, the help window and the name dialog.
//!
//! None of these decide anything. [`sonorant_core::menu`] holds the tree of items, and
//! this draws it: a submenu for a submenu, a checkbox for a switch, a radio button for a
//! choice, each with its key written alongside and its help line as a tooltip. A click
//! hands the item's action back to [`sonorant_core::menu::apply`], which is also where
//! the keyboard sends its keys, so both go through the same door.

use sonorant_core::media::{Controls, Player};
use sonorant_core::menu::{self, Action, Context, Effect, Item, Kind, Presentation, Session};
use sonorant_core::settings::{Settings, sanitise_name};

use crate::pacing::PacingStats;
use crate::present::PresentCounts;

/// Tells a screen reader what the window is and what the picture is showing.
///
/// Everything else here is an egui widget and describes itself. The visuals are not:
/// they are one region of pixels drawn by our own shaders, and what is useful to say
/// about them is not a list of parts but the reading itself, which is what the status
/// line already puts into words. It goes on as a description rather than a label so a
/// reader says the name once and the figures when asked.
///
/// The window is named here too because egui builds the root node with no name of its
/// own, so without this a reader announces an application with an untitled window.
fn describe(ctx: &egui::Context, visuals: &egui::Response, reading: &str) {
    use egui::accesskit::{Node, Role};
    let named = |node: &mut Node, role, label: &str, description: &str| {
        node.set_role(role);
        node.set_label(label.to_owned());
        if !description.is_empty() {
            node.set_description(description.to_owned());
        }
    };
    ctx.accesskit_node_builder(egui::accesskit_root_id(), |node| {
        named(node, Role::Window, "Sonorant", "");
    });
    ctx.accesskit_node_builder(visuals.id, |node| {
        named(node, Role::Image, "Analyser", reading);
    });
}

/// A preset the app has to fetch, write or remove: the model can't reach the settings
/// folder itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    Load(String),
    /// Save the settings as they are under a name the dialog has already taken.
    Save(String),
    Delete(String),
}

/// Everything the status line reports.
#[derive(Clone, Debug, Default)]
pub struct Status {
    pub pacing: PacingStats,
    /// What reached the screen, where the platform counts it.
    pub presented: Option<PresentCounts>,
    pub capture: String,
    /// Audio frames and history rows lost.
    pub dropped: (u64, u64),
}

/// The state the windows keep between frames, and the work they hand back.
#[derive(Debug, Default)]
pub struct Shell {
    /// The state the menu reads and writes that isn't a saved setting.
    pub session: Session,
    /// What the help window's search box holds.
    query: String,
    /// Which row of the help window the keyboard is on, counted over the list as it is
    /// filtered. Clamped to what is showing, because the filter changes under it.
    picked: usize,
    /// The name dialog while it is open: the name typed, and why it can't be used yet.
    naming: Option<(String, Option<String>)>,
    /// Set when the help window opens, so the search box takes the focus once.
    focus_search: bool,
    asks: Vec<Ask>,
}

impl Shell {
    pub fn new(fullscreen: bool) -> Shell {
        Shell {
            session: Session {
                fullscreen,
                ..Session::default()
            },
            ..Shell::default()
        }
    }

    /// Work the app has to carry out, taken once a frame.
    pub fn take_asks(&mut self) -> Vec<Ask> {
        std::mem::take(&mut self.asks)
    }

    /// Performs one action, and holds on to anything the app has to finish.
    ///
    /// This is the one way in: the menu, the keys and the deck's buttons all come
    /// through here.
    pub fn act(&mut self, action: &Action, settings: &mut Settings) {
        match menu::apply(action, settings, &mut self.session) {
            Some(Effect::LoadPreset(name)) => self.asks.push(Ask::Load(name)),
            Some(Effect::DeletePreset(name)) => self.asks.push(Ask::Delete(name)),
            // The name has to be asked for before anything can be saved.
            Some(Effect::SavePreset) => self.naming = Some((String::new(), None)),
            None => {}
        }
        if matches!(action, Action::Help) && self.session.help {
            self.focus_search = true;
        }
    }

    /// Lays out one frame's UI and returns the area left for the visuals, in points.
    pub fn show(&mut self, ui: &mut egui::Ui, settings: &mut Settings, what: &Around<'_>) -> Area {
        let mut area = Area::default();
        let mut chosen: Option<Action> = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                area.rect = ui.max_rect();
                let response = ui.interact(
                    area.rect,
                    egui::Id::new("visuals"),
                    egui::Sense::click_and_drag(),
                );
                area.hovered = response
                    .hovered()
                    .then(|| ui.ctx().pointer_latest_pos())
                    .flatten();
                area.double_clicked = response.double_clicked();
                // A drag over the image pans the history, or orbits the waterfall's
                // camera, so both directions are wanted.
                area.dragged = response.dragged().then(|| response.drag_delta());
                // Smoothed rather than raw: the wheel drives a zoom, and a zoom that
                // jumps a notch at a time is harder to aim than one that glides.
                if response.hovered() {
                    area.scrolled = ui.ctx().input(|i| i.smooth_scroll_delta.y);
                }
                // A drag ends in a click as far as egui is concerned, and a pan is not
                // a press of whatever button happens to be under the pointer.
                area.clicked = response.clicked() && area.dragged.is_none();
                area.menu_open = response.context_menu_opened();
                describe(ui.ctx(), &response, what.reading);
                // The menu stays up while it is being used. egui's default for a menu
                // is to close on any click at all, which put the whole thing away
                // every time a box was ticked, so a switch could only ever be tried
                // one at a time with a right-click in between. It is as much a
                // settings panel as a menu: what a click changes shows in the picture
                // behind it, and the next thing can be tried against the last. A
                // submenu inherits this from its parent, so one line covers the tree.
                egui::Popup::context_menu(&response)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                        let items = menu::tree(&Context {
                            settings,
                            session: &self.session,
                            players: what.players,
                            presets: what.presets,
                            controls: what.controls,
                            presentations: what.presentations,
                            reported_delay_ms: what.reported_delay_ms,
                        });
                        draw(ui, &items, &mut chosen);
                    });
            });
        if let Some(action) = chosen {
            self.act(&action, settings);
        }
        self.help(ui.ctx(), settings, what, area.menu_open);
        self.name_dialog(ui.ctx(), what.presets);
        area
    }

    /// The help window: every command, with its key, what it does and the state it is
    /// in.
    ///
    /// It is a list to act on rather than a page to read. Clicking a row does the
    /// thing, so help is a way to reach a command whose menu you can't remember, and
    /// the ticks and dots down the left are the live settings, so the window doubles
    /// as a reading of how the app is set. The rows are grouped under the submenu each
    /// command lives in, which makes "where was that?" answerable by the same shape
    /// the menu has.
    ///
    /// `menu_open` keeps Escape honest: it takes one thing off at a time, and the
    /// right-click menu is in front of this.
    fn help(
        &mut self,
        ctx: &egui::Context,
        settings: &mut Settings,
        what: &Around<'_>,
        menu_open: bool,
    ) {
        if !self.session.help {
            return;
        }
        let items = menu::tree(&Context {
            settings,
            session: &self.session,
            players: what.players,
            presets: what.presets,
            controls: what.controls,
            presentations: what.presentations,
            reported_delay_ms: what.reported_delay_ms,
        });
        let mut open = true;
        let mut chosen: Option<Action> = None;
        let mut shut = false;
        // The name dialog wants Enter for itself, so the list lets go of the keyboard
        // while it is up rather than answering a key meant for the box in front.
        let driving = self.naming.is_none();
        egui::Window::new("Help")
            .open(&mut open)
            .default_width(720.0)
            .default_height(560.0)
            .show(ctx, |ui| {
                // Taken before the search box is built: a text box swallows Enter and
                // the arrows, and here they belong to the list under it.
                let (up, down, run) = ui.input_mut(|i| {
                    if !driving {
                        return (false, false, false);
                    }
                    (
                        i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                        i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                        i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                    )
                });
                if driving && !menu_open {
                    shut |=
                        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                }

                ui.horizontal(|ui| {
                    ui.label("Search");
                    let box_ = ui.add(
                        egui::TextEdit::singleline(&mut self.query)
                            .desired_width(260.0)
                            .hint_text("a name, a key or a word from the description"),
                    );
                    // The window is opened to be typed into, but only on the frame it
                    // opens: taking the focus back every frame would trap it.
                    if std::mem::take(&mut self.focus_search) {
                        box_.request_focus();
                    }
                    // A different list is a different first row.
                    if box_.changed() {
                        self.picked = 0;
                    }
                    if ui.button("Clear").clicked() {
                        self.query.clear();
                        self.picked = 0;
                    }
                    ui.label(
                        egui::RichText::new("Arrows to move, Enter to do it, Esc to close").weak(),
                    );
                });
                ui.separator();

                let sheet = menu::entries(&items);
                let sheet: Vec<&menu::Entry> =
                    sheet.iter().filter(|e| e.shortcut.is_some()).collect();
                egui::CollapsingHeader::new(format!("Keys ({})", sheet.len()))
                    .default_open(true)
                    .show(ui, |ui| {
                        // Two pairs to a row: the list is short and wide rather than
                        // long and thin, so it fits above the search results.
                        egui::Grid::new("help-keys")
                            .num_columns(4)
                            .spacing([12.0, 4.0])
                            .show(ui, |ui| {
                                for pair in sheet.chunks(2) {
                                    for entry in pair {
                                        ui.label(chip(entry.shortcut));
                                        ui.label(entry.leaf());
                                    }
                                    ui.end_row();
                                }
                            });
                    });
                ui.separator();

                let found = menu::search(&items, &self.query);
                if found.is_empty() {
                    self.picked = 0;
                } else {
                    if down {
                        self.picked += 1;
                    }
                    if up {
                        self.picked = self.picked.saturating_sub(1);
                    }
                    self.picked = self.picked.min(found.len() - 1);
                }
                if run
                    && let Some(entry) = found.get(self.picked)
                    && entry.enabled
                {
                    chosen = Some(entry.action.clone());
                }
                ui.label(match found.len() {
                    0 => "Nothing matches".to_owned(),
                    1 => "1 command".to_owned(),
                    n => format!("{n} commands"),
                });

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Grid::new("help-grid")
                            .num_columns(4)
                            .striped(true)
                            .show(ui, |ui| {
                                let mut section: Option<&str> = None;
                                for (i, entry) in found.iter().enumerate() {
                                    if section != Some(entry.section()) {
                                        section = Some(entry.section());
                                        ui.label(
                                            egui::RichText::new(entry.section()).strong().heading(),
                                        );
                                        ui.label("");
                                        ui.label("");
                                        ui.label("");
                                        ui.end_row();
                                    }
                                    if mark(ui, entry).is_some_and(|r| r.clicked()) {
                                        self.picked = i;
                                        chosen = Some(entry.action.clone());
                                    }
                                    let hit = ui.add_enabled(
                                        entry.enabled,
                                        egui::Button::new(highlight(ui, under(entry), &self.query))
                                            .selected(i == self.picked),
                                    );
                                    // The keyboard's row is brought into view as it
                                    // moves, or moving past the fold would lose it.
                                    if i == self.picked && (up || down) {
                                        hit.scroll_to_me(None);
                                    }
                                    if hit.clicked() {
                                        self.picked = i;
                                        chosen = Some(entry.action.clone());
                                    }
                                    ui.label(chip(entry.shortcut));
                                    ui.label(highlight(ui, entry.help, &self.query));
                                    ui.end_row();
                                }
                            });
                    });
            });
        if let Some(action) = chosen {
            self.act(&action, settings);
            // The row's tick is drawn from the model, which has only now moved.
            ctx.request_repaint();
        }
        if !open || shut {
            self.session.help = false;
        }
    }

    /// The name dialog, for saving the settings as a preset of your own.
    fn name_dialog(&mut self, ctx: &egui::Context, presets: &[String]) {
        let Some((text, error)) = &mut self.naming else {
            return;
        };
        let mut open = true;
        let mut save = false;
        let mut cancel = false;
        egui::Window::new("Save these settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("A name for this preset");
                let box_ = ui.text_edit_singleline(text);
                box_.request_focus();
                if let Some(why) = error.as_deref() {
                    ui.colored_label(ui.visuals().error_fg_color, why);
                }
                let replaces = presets
                    .iter()
                    .any(|p| p.eq_ignore_ascii_case(sanitise_name(text).as_str()));
                ui.horizontal(|ui| {
                    save = ui
                        .button(if replaces { "Replace" } else { "Save" })
                        .clicked();
                    cancel = ui.button("Cancel").clicked();
                });
                save |= box_.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                cancel |= ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
        if save {
            // A preset is a file, so the name has to be one a file can have. Saying so
            // beats writing "My mix 2/3" to a folder that has no such place.
            let name = sanitise_name(text);
            if name.is_empty() {
                *error = Some("A name needs a letter or a digit in it".to_owned());
            } else {
                self.asks.push(Ask::Save(name));
                self.naming = None;
            }
        } else if cancel || !open {
            self.naming = None;
        }
    }
}

/// What the app knows and the model doesn't: the lists the menu offers.
#[derive(Debug)]
pub struct Around<'a> {
    pub players: &'a [Player],
    pub presets: &'a [String],
    pub controls: Controls,
    pub presentations: &'a [Presentation],
    /// What the system says the output path costs, in milliseconds, where it says
    /// anything: the figure the automatic visual delay follows.
    pub reported_delay_ms: Option<f64>,
    /// What the status line last read, which is what a screen reader is told the
    /// picture is showing.
    pub reading: &'a str,
}

/// What the central panel left for the visuals, and what the pointer did in it.
#[derive(Clone, Copy, Debug)]
pub struct Area {
    pub rect: egui::Rect,
    /// Where the pointer is over the visuals, in points, or `None` when it is elsewhere.
    pub hovered: Option<egui::Pos2>,
    pub double_clicked: bool,
    /// Whether it was clicked at all, for the quick bar's buttons.
    pub clicked: bool,
    /// How far a drag moved this frame, in points, or `None` when nothing is being
    /// dragged. The flat views use the time axis; the waterfall uses both.
    pub dragged: Option<egui::Vec2>,
    /// How far the wheel turned over the image this frame, in points.
    pub scrolled: f32,
    /// Whether the right-click menu is open, so the chrome doesn't fade under it.
    pub menu_open: bool,
}

impl Default for Area {
    fn default() -> Area {
        Area {
            rect: egui::Rect::NOTHING,
            hovered: None,
            double_clicked: false,
            clicked: false,
            dragged: None,
            scrolled: 0.0,
            menu_open: false,
        }
    }
}

/// Draws a level of the menu, and says which item was clicked.
fn draw(ui: &mut egui::Ui, items: &[Item], chosen: &mut Option<Action>) {
    for item in items {
        match &item.kind {
            Kind::Separator => {
                ui.separator();
            }
            Kind::Submenu(children) => {
                ui.menu_button(item.label.as_str(), |ui| draw(ui, children, chosen))
                    .response
                    .on_hover_text(item.help);
            }
            Kind::Command(action) => {
                let hit = ui
                    .add_enabled(item.enabled, egui::Button::new(atoms(item)))
                    .on_hover_text(item.help);
                if hit.clicked() {
                    *chosen = Some(action.clone());
                    took(ui, action);
                }
            }
            Kind::Check(action, on) => {
                // The checkbox's own bool is thrown away: the state comes from the model
                // next frame, so a switch can't drift from what it controls.
                let mut shown = *on;
                let hit = ui
                    .add_enabled(item.enabled, egui::Checkbox::new(&mut shown, atoms(item)))
                    .on_hover_text(item.help);
                if hit.clicked() {
                    *chosen = Some(action.clone());
                    took(ui, action);
                }
            }
            Kind::Radio(action, selected) => {
                let hit = ui
                    .add_enabled(item.enabled, egui::RadioButton::new(*selected, atoms(item)))
                    .on_hover_text(item.help);
                if hit.clicked() {
                    *chosen = Some(action.clone());
                    took(ui, action);
                }
            }
        }
    }
}

/// A help row's label: everything below the heading it is grouped under, so a command
/// two levels down still says which level it is on.
fn under(entry: &menu::Entry) -> &str {
    match entry.path.split_once(" > ") {
        Some((_, rest)) => rest,
        None => entry.path.as_str(),
    }
}

/// The state of a switch or a choice, down the left of the help list: the same box or
/// dot the menu draws, so the two read alike.
///
/// Drawn by egui rather than written as a character. Written, a filled dot came out as
/// an empty box on this machine: the fonts egui bundles have no glyph for it, and a
/// list of settings whose "on" mark is a missing-glyph box is worse than no mark.
/// Clicking one does what clicking the row does, so there is nothing here that looks
/// like a second, disagreeing switch.
fn mark(ui: &mut egui::Ui, entry: &menu::Entry) -> Option<egui::Response> {
    match entry.kind {
        menu::EntryKind::Command => {
            ui.label("");
            None
        }
        menu::EntryKind::Check(on) => {
            let mut shown = on;
            Some(ui.add_enabled(entry.enabled, egui::Checkbox::without_text(&mut shown)))
        }
        menu::EntryKind::Radio(chosen) => {
            Some(ui.add_enabled(entry.enabled, egui::RadioButton::new(chosen, "")))
        }
    }
}

/// A key as a chip, or a dash where a command has no key.
fn chip(shortcut: Option<&str>) -> egui::RichText {
    match shortcut {
        Some(key) => egui::RichText::new(key).monospace().strong(),
        None => egui::RichText::new("—").weak(),
    }
}

/// `text` with every occurrence of `query` picked out, so a row says why it matched.
///
/// Matched by ASCII case alone, which is both what the search does and what keeps the
/// byte offsets of the lowercased copy the same as the original's.
fn highlight(ui: &egui::Ui, text: &str, query: &str) -> egui::text::LayoutJob {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let plain = egui::TextFormat {
        font_id: font.clone(),
        color: ui.visuals().text_color(),
        ..Default::default()
    };
    let lit = egui::TextFormat {
        font_id: font,
        color: ui.visuals().strong_text_color(),
        background: ui.visuals().selection.bg_fill.gamma_multiply(0.45),
        ..Default::default()
    };
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        job.append(text, 0.0, plain);
        return job;
    }
    let hay = text.to_ascii_lowercase();
    let mut at = 0;
    while let Some(found) = hay[at..].find(&needle) {
        let from = at + found;
        let to = from + needle.len();
        job.append(&text[at..from], 0.0, plain.clone());
        job.append(&text[from..to], 0.0, lit.clone());
        at = to;
    }
    job.append(&text[at..], 0.0, plain);
    job
}

/// What happens to the menu itself once an item has been chosen.
///
/// [`Action::closes_menu`] decides, and `ui.close()` closes the whole tree from
/// wherever in it the item was, so a command in a submenu puts the lot away. The
/// repaint is asked for because the change shows a frame later, when the tree is built
/// again from the settings: without it, a click while the app is idling at ten frames
/// a second could take a tenth of a second to appear.
fn took(ui: &egui::Ui, action: &Action) {
    if action.closes_menu() {
        ui.close();
    }
    ui.ctx().request_repaint();
}

/// An item's label, with its key pushed out to the right edge.
fn atoms(item: &Item) -> egui::Atoms<'_> {
    let mut atoms = egui::Atoms::new(item.label.as_str());
    if let Some(key) = item.shortcut {
        atoms.push_right(egui::Atom::grow());
        atoms.push_right(egui::RichText::new(key).weak());
    }
    atoms
}

/// The name a key goes by in the menu model, or `None` for a key it doesn't use.
pub fn key_name(key: &winit::keyboard::Key) -> Option<String> {
    use winit::keyboard::{Key, NamedKey};
    match key {
        Key::Named(NamedKey::Space) => Some("Space".to_owned()),
        Key::Named(NamedKey::Escape) => Some("Esc".to_owned()),
        Key::Named(NamedKey::End) => Some("End".to_owned()),
        Key::Named(NamedKey::F1) => Some("F1".to_owned()),
        Key::Named(NamedKey::F11) => Some("F11".to_owned()),
        Key::Character(c) if c.len() == 1 && c.is_ascii() => Some(c.to_uppercase()),
        _ => None,
    }
}
