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
                response.context_menu(|ui| {
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
        self.help(ui.ctx(), settings, what);
        self.name_dialog(ui.ctx(), what.presets);
        area
    }

    /// The help window: every command, with its key and what it does, filtered by the
    /// search box. Clicking one performs it, so help is also a way to reach a command
    /// whose menu you can't remember.
    fn help(&mut self, ctx: &egui::Context, settings: &mut Settings, what: &Around<'_>) {
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
        egui::Window::new("Help")
            .open(&mut open)
            .default_width(520.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Search");
                    let box_ = ui.text_edit_singleline(&mut self.query);
                    // The window is opened to be typed into, but only on the frame it
                    // opens: taking the focus back every frame would trap it.
                    if std::mem::take(&mut self.focus_search) {
                        box_.request_focus();
                    }
                    if ui.button("Clear").clicked() {
                        self.query.clear();
                    }
                });
                ui.separator();
                let found = menu::search(&items, &self.query);
                ui.label(match found.len() {
                    0 => "Nothing matches".to_owned(),
                    1 => "1 command".to_owned(),
                    n => format!("{n} commands"),
                });
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .show(ui, |ui| {
                        egui::Grid::new("help-grid")
                            .num_columns(3)
                            .striped(true)
                            .show(ui, |ui| {
                                for entry in found {
                                    if ui.button(&entry.path).clicked() {
                                        chosen = Some(entry.action.clone());
                                    }
                                    ui.label(entry.shortcut.unwrap_or(""));
                                    ui.label(entry.help);
                                    ui.end_row();
                                }
                            });
                    });
            });
        if let Some(action) = chosen {
            self.act(&action, settings);
        }
        if !open {
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
                    ui.close();
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
                }
            }
            Kind::Radio(action, selected) => {
                let hit = ui
                    .add_enabled(item.enabled, egui::RadioButton::new(*selected, atoms(item)))
                    .on_hover_text(item.help);
                if hit.clicked() {
                    *chosen = Some(action.clone());
                }
            }
        }
    }
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
