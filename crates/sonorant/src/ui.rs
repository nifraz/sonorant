//! The egui layer: the right-click menu, the help window and the name dialog.
//!
//! None of these decide anything. [`sonorant_core::menu`] holds the tree of items, and
//! this draws it: a submenu for a submenu, a checkbox for a switch, a radio button for a
//! choice, each with its key written alongside and its help line as a tooltip. A click
//! hands the item's action back to [`sonorant_core::menu::apply`], which is also where
//! the keyboard sends its keys, so both go through the same door.

use sonorant_core::media::{Controls, Player};
use sonorant_core::menu::{
    self, Action, Context, Effect, Entry, EntryKind, Item, Kind, Pick, Presentation, Session,
};
use sonorant_core::settings::{Number, Settings, sanitise_name};

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
    /// The settings as they were when they last came from somewhere they can be got
    /// back from: a preset, a save, a reset, or the settings file at start-up.
    ///
    /// What [`Shell::unsaved`] compares against. `Settings::preset` cannot answer this
    /// on its own, because loading a preset of your own replaces the whole struct and
    /// brings whatever `preset` was written into that file along with it.
    saved: Option<Settings>,
    /// A switch the user has to answer before it happens, because it would throw the
    /// changes away: the action, and the name of what is being switched to.
    confirming: Option<(Action, String)>,
    /// What to do once the name dialog has saved, where saving was the answer to that
    /// warning rather than something asked for on its own.
    after_saving: Option<Action>,
    /// Set where something opened a window the menu would be in front of.
    close_the_menu: bool,
    /// Which of the help window's two pages is showing.
    page: Page,
    /// The help window's filters, none of which are on to begin with.
    filter: Filter,
}

/// The help window's two pages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Commands,
    HowItWorks,
}

/// What the help window is narrowed to, beside the search box.
///
/// Three switches rather than one setting, because they answer different questions and
/// asking two at once is reasonable: "what have I changed that has a key?" is a fair
/// thing to want.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Filter {
    /// Only what differs from the settings the app ships with.
    changed: bool,
    /// Only switches that are on.
    on: bool,
    /// Only what a key reaches.
    keyed: bool,
}

impl Filter {
    fn any(self) -> bool {
        self.changed || self.on || self.keyed
    }

    fn keeps(self, e: &Entry) -> bool {
        (!self.changed || e.changed())
            && (!self.on || matches!(e.kind, EntryKind::Check(true)))
            && (!self.keyed || e.shortcut.is_some())
    }
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

    /// Puts the right-click menu away, wherever in its tree it is open.
    ///
    /// The menu is egui's to hold, not the model's, so this is the one way to close it
    /// from outside a click on one of its items. The right-click menu and its submenus
    /// are the only popups the app raises: help and the name dialog are windows, and
    /// closing every popup is how a submenu goes with its parent.
    pub fn close_menu(&self, ctx: &egui::Context) {
        egui::Popup::close_all(ctx);
    }

    /// Takes the settings as a place that can be got back to.
    ///
    /// Called at start-up and after a preset is loaded or written, which are the two
    /// the app carries out rather than the model. Everything else that lands on a known
    /// set of settings does it in [`Shell::run`].
    pub fn settled(&mut self, settings: &Settings) {
        self.saved = Some(settings.clone());
    }

    /// Whether the settings have moved since they last came from somewhere.
    pub fn unsaved(&self, settings: &Settings) -> bool {
        self.saved.as_ref().is_some_and(|was| was != settings)
    }

    /// Performs one action, and holds on to anything the app has to finish.
    ///
    /// This is the one way in: the menu, the keys, the help window and the deck's
    /// buttons all come through here, which is why the warning below can sit here and
    /// cover every one of them.
    pub fn act(&mut self, action: &Action, settings: &mut Settings) {
        if let Some(what) = self.would_lose_changes(action, settings) {
            self.confirming = Some((action.clone(), what));
            // The dialog is a window, and the menu would be in front of it.
            self.close_the_menu = true;
            return;
        }
        self.run(action, settings);
    }

    /// What `action` would switch to, where doing it would throw away changes.
    ///
    /// Only the three that replace every setting at once: the built-in presets, a
    /// preset of your own, and the reset. Quitting is not among them, because the
    /// settings are written on the way out, so nothing is lost by leaving.
    fn would_lose_changes(&self, action: &Action, settings: &Settings) -> Option<String> {
        if !self.unsaved(settings) {
            return None;
        }
        match action {
            Action::UsePreset(p) => Some(p.name().to_owned()),
            Action::LoadPreset(name) => Some(name.clone()),
            Action::ResetSettings => Some("the settings the app ships with".to_owned()),
            _ => None,
        }
    }

    /// Performs an action with no questions asked: the warning has been answered, or
    /// there was nothing to warn about.
    fn run(&mut self, action: &Action, settings: &mut Settings) {
        match menu::apply(action, settings, &mut self.session) {
            Some(Effect::LoadPreset(name)) => self.asks.push(Ask::Load(name)),
            Some(Effect::DeletePreset(name)) => self.asks.push(Ask::Delete(name)),
            // The name has to be asked for before anything can be saved.
            Some(Effect::SavePreset) => self.naming = Some((String::new(), None)),
            None => {}
        }
        // A preset or a reset is a place that can be got back to, so the settings are
        // no longer unsaved the moment one lands.
        if matches!(action, Action::UsePreset(_) | Action::ResetSettings) {
            self.saved = Some(settings.clone());
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
        self.confirm(ui.ctx(), settings, what);
        self.help(ui.ctx(), settings, what, area.menu_open);
        self.name_dialog(ui.ctx(), what.presets, settings);
        if std::mem::take(&mut self.close_the_menu) {
            self.close_menu(ui.ctx());
            area.menu_open = false;
        }
        area
    }

    /// The warning before a switch that would throw away changes.
    ///
    /// Three ways out, because there are three things a person means by "wait": keep
    /// these first, go ahead anyway, or never mind. Saving is offered rather than only
    /// warned about, because a warning you can only obey by cancelling, finding the
    /// save item yourself and coming back is a warning that teaches people to click
    /// past it.
    fn confirm(&mut self, ctx: &egui::Context, settings: &mut Settings, what: &Around<'_>) {
        let Some((action, target)) = self.confirming.clone() else {
            return;
        };
        let changed = self.changed_since_saved(settings, what);
        let mut save = false;
        let mut go = false;
        let mut cancel = false;
        let answer = egui::Modal::new(egui::Id::new("unsaved")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.heading("These settings are not saved");
            ui.add_space(6.0);
            ui.label(match changed {
                0 | 1 => format!("One setting has changed. Switching to {target} will lose it."),
                n => format!("{n} settings have changed. Switching to {target} will lose them."),
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                save = ui.button("Save as a preset...").clicked();
                go = ui.button("Switch anyway").clicked();
                cancel = ui.button("Cancel").clicked();
            });
            cancel |= ui.input(|i| i.key_pressed(egui::Key::Escape));
        });
        if answer.should_close() {
            cancel = true;
        }
        if save {
            // The switch waits for the name dialog: saving is what was asked for, and
            // it would be a poor answer to write the preset and then not go.
            self.confirming = None;
            self.after_saving = Some(action);
            self.naming = Some((String::new(), None));
        } else if go {
            self.confirming = None;
            self.run(&action, settings);
        } else if cancel {
            self.confirming = None;
        }
    }

    /// How many settings have moved since they last came from somewhere.
    ///
    /// Counted off the menu model, one reading against the other, because that is the
    /// question the warning is about: not how far these settings are from the ones the
    /// app ships with, but how much would be thrown away. Both readings come from the
    /// same tree shape, so they line up row for row.
    fn changed_since_saved(&self, settings: &Settings, what: &Around<'_>) -> usize {
        let Some(was) = &self.saved else {
            return 0;
        };
        let reading = |s: &Settings| {
            menu::entries(&menu::tree(&Context {
                settings: s,
                session: &self.session,
                players: what.players,
                presets: what.presets,
                controls: what.controls,
                presentations: what.presentations,
                reported_delay_ms: what.reported_delay_ms,
            }))
            .iter()
            .map(|e| (e.path.clone(), e.kind.value()))
            .collect::<Vec<_>>()
        };
        let (now, then) = (reading(settings), reading(was));
        now.iter()
            .zip(&then)
            .filter(|(a, b)| a.0 == b.0 && a.1 != b.1)
            .count()
    }

    /// The help window: every command, with its key, what it does and the state it is
    /// in.
    ///
    /// It is a list to act on rather than a page to read. A row is the setting itself,
    /// not a link to it: a switch has its box, a choice its dropdown, a number its
    /// slider, so the window is a settings sheet that happens to be searchable, and
    /// "where was that?" and "what have I changed?" have the same answer in it.
    ///
    /// A choice is one row and not one per value. It used to be one per value, which
    /// made 257 rows of which four read "Which pair of channels the panes show", and a
    /// list nobody can skim is a list nobody reads.
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
        let driving = self.naming.is_none() && self.confirming.is_none();
        egui::Window::new("Help")
            .open(&mut open)
            .default_width(860.0)
            .default_height(600.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.page, Page::Commands, "Commands");
                    ui.selectable_value(&mut self.page, Page::HowItWorks, "How it works");
                });
                ui.separator();
                match self.page {
                    Page::Commands => {
                        shut |= self.commands(ui, &items, driving, menu_open, &mut chosen);
                    }
                    Page::HowItWorks => {
                        if driving && !menu_open {
                            shut |= ui.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                            });
                        }
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, how_it_works);
                    }
                }
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

    /// The commands page: the keys, the search box and its filters, then the rows.
    ///
    /// Returns whether Escape was pressed for this window.
    fn commands(
        &mut self,
        ui: &mut egui::Ui,
        items: &[Item],
        driving: bool,
        menu_open: bool,
        chosen: &mut Option<Action>,
    ) -> bool {
        // Taken before the search box is built: a text box swallows Enter and the
        // arrows, and here they belong to the list under it.
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
        let mut shut = false;
        if driving && !menu_open {
            shut |= ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        }

        ui.horizontal(|ui| {
            ui.label("Search");
            let box_ = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .desired_width(240.0)
                    .hint_text("a name, a value, a key or a word from the description"),
            );
            // The window is opened to be typed into, but only on the frame it opens:
            // taking the focus back every frame would trap it.
            if std::mem::take(&mut self.focus_search) {
                box_.request_focus();
            }
            // A different list is a different first row.
            if box_.changed() {
                self.picked = 0;
            }
            let was = self.filter;
            ui.toggle_value(&mut self.filter.changed, "Changed")
                .on_hover_text("Only what differs from the settings the app ships with");
            ui.toggle_value(&mut self.filter.on, "On")
                .on_hover_text("Only switches that are on");
            ui.toggle_value(&mut self.filter.keyed, "Has a key")
                .on_hover_text("Only what a key reaches");
            if ui
                .add_enabled(
                    !self.query.is_empty() || self.filter.any(),
                    egui::Button::new("Clear"),
                )
                .clicked()
            {
                self.query.clear();
                self.filter = Filter::default();
                self.picked = 0;
            }
            if was != self.filter {
                self.picked = 0;
            }
        });
        ui.label(
            egui::RichText::new(
                "Arrows to move, Enter to do it, Esc to close. A row is the setting \
                 itself, so it can be changed here.",
            )
            .weak(),
        );
        ui.separator();

        let sheet = menu::entries(items);
        let keys: Vec<&Entry> = sheet.iter().filter(|e| e.shortcut.is_some()).collect();
        egui::CollapsingHeader::new(format!("Keys ({})", keys.len()))
            .default_open(true)
            .show(ui, |ui| {
                // Two pairs to a row: the list is short and wide rather than long and
                // thin, so it fits above the search results.
                egui::Grid::new("help-keys")
                    .num_columns(4)
                    .spacing([12.0, 4.0])
                    .show(ui, |ui| {
                        for pair in keys.chunks(2) {
                            for entry in pair {
                                ui.label(chip(entry.shortcut));
                                ui.label(entry.leaf());
                            }
                            ui.end_row();
                        }
                    });
            });
        ui.separator();

        let needle = self.query.trim().to_lowercase();
        let found: Vec<&Entry> = sheet
            .iter()
            .filter(|e| e.matches(&needle) && self.filter.keeps(e))
            .collect();
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
            *chosen = Some(entry.action.clone());
        }
        let changed = sheet.iter().filter(|e| e.changed()).count();
        ui.horizontal(|ui| {
            ui.label(match found.len() {
                0 => "Nothing matches".to_owned(),
                1 => "1 setting".to_owned(),
                n => format!("{n} settings"),
            });
            if changed > 0 && !self.filter.changed {
                ui.label(
                    egui::RichText::new(format!("{changed} changed from the defaults")).weak(),
                );
            }
        });

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("help-grid")
                    .num_columns(5)
                    .striped(true)
                    .spacing([10.0, 3.0])
                    .show(ui, |ui| {
                        let mut section: Option<&str> = None;
                        for (i, entry) in found.iter().enumerate() {
                            if section != Some(entry.section()) {
                                section = Some(entry.section());
                                ui.label("");
                                ui.label(egui::RichText::new(entry.section()).strong());
                                for _ in 0..3 {
                                    ui.label("");
                                }
                                ui.end_row();
                            }
                            let hit = row(ui, entry, &self.query, i == self.picked, chosen);
                            if let Some(hit) = hit {
                                // The keyboard's row is brought into view as it moves,
                                // or moving past the fold would lose it.
                                if i == self.picked && (up || down) {
                                    hit.scroll_to_me(None);
                                }
                                if hit.clicked() {
                                    self.picked = i;
                                }
                            }
                            ui.end_row();
                        }
                    });
            });
        shut
    }

    /// The name dialog, for saving the settings as a preset of your own.
    fn name_dialog(&mut self, ctx: &egui::Context, presets: &[String], settings: &mut Settings) {
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
                // Saving was the answer to the warning, so the switch it held up goes
                // ahead now the settings are somewhere they can be got back from.
                if let Some(action) = self.after_saving.take() {
                    self.saved = Some(settings.clone());
                    self.run(&action, settings);
                }
            }
        } else if cancel || !open {
            self.naming = None;
            // Backing out of the name dialog backs out of the switch as well: the
            // person asked to save first, and there has been no save.
            self.after_saving = None;
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
            // A choice reads as a submenu of radios, which is what it has always
            // looked like; the model groups it so that help can draw it as one row.
            Kind::Choice(picks) => {
                ui.menu_button(with_value(item, value_of(picks)), |ui| {
                    radios(ui, picks, chosen)
                })
                .response
                .on_hover_text(item.help);
            }
            // A number is the same submenu of named values, and under them the slider
            // and the steppers that reach everything between them.
            Kind::Number(n, value, picks) => {
                ui.menu_button(with_value(item, Some(menu::written(*n, *value))), |ui| {
                    if !picks.is_empty() {
                        radios(ui, picks, chosen);
                        ui.separator();
                    }
                    slider(ui, *n, *value, chosen);
                })
                .response
                .on_hover_text(item.help);
            }
        }
    }
}

/// The values of a group, as radio buttons.
fn radios(ui: &mut egui::Ui, picks: &[Pick], chosen: &mut Option<Action>) {
    for p in picks {
        let hit = ui
            .add_enabled(
                p.enabled,
                egui::RadioButton::new(p.chosen, p.label.as_str()),
            )
            .on_hover_text(p.help);
        if hit.clicked() {
            *chosen = Some(p.action.clone());
            took(ui, &p.action);
        }
    }
}

/// A number's slider, with a stepper either side of it.
///
/// The steppers are there because a slider is bad at the last two pixels: nudging a
/// visual delay by five milliseconds or a bar by one is exactly the kind of thing this
/// menu is for, and dragging to it is luck.
fn slider(ui: &mut egui::Ui, n: Number, value: f64, chosen: &mut Option<Action>) {
    let r = n.range();
    let mut v = value;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(value > r.min, egui::Button::new("−"))
            .on_hover_text(format!("Down by {}", menu::written(n, r.step)))
            .clicked()
        {
            *chosen = Some(Action::Step(n, -1.0));
        }
        // The value shows, and shows as a box that can be typed into: a slider is the
        // wrong tool for "exactly 150 ms", and inside the submenu the label that
        // carries the figure is out of sight.
        //
        // No `step_by`. egui snaps the value it is given onto the step grid and reports
        // that as a change, so a slider that steps would move any figure not already on
        // its grid the moment it was drawn: opening this menu once turned a top of the
        // axis of 22,050 Hz into 22,100. The step belongs to the buttons either side,
        // which are the ones asked to move by a step; the bar itself goes anywhere the
        // setting allows, and `Settings::set_number` rounds the whole ones.
        let drag = ui.add(
            egui::Slider::new(&mut v, r.min..=r.max).custom_formatter(|x, _| menu::written(n, x)),
        );
        if drag.changed() {
            *chosen = Some(Action::Set(n, v));
        }
        if ui
            .add_enabled(value < r.max, egui::Button::new("+"))
            .on_hover_text(format!("Up by {}", menu::written(n, r.step)))
            .clicked()
        {
            *chosen = Some(Action::Step(n, 1.0));
        }
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(menu::written(n, r.min)).weak().small());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(menu::written(n, r.max)).weak().small());
        });
    });
    // A slider drag is one long gesture; without this the picture behind only catches
    // up when the pointer stops.
    ui.ctx().request_repaint();
}

/// Which value of a group is in use.
fn value_of(picks: &[Pick]) -> Option<String> {
    picks.iter().find(|p| p.chosen).map(|p| p.label.clone())
}

/// An item's label with what it is set to written after it, so a submenu says what is
/// inside it without being opened.
fn with_value(item: &Item, value: Option<String>) -> egui::Atoms<'_> {
    let mut atoms = egui::Atoms::new(item.label.as_str());
    if let Some(value) = value {
        atoms.push_right(egui::Atom::grow());
        atoms.push_right(egui::RichText::new(value).weak());
    }
    atoms
}

/// A help row's label: everything below the heading it is grouped under, so a command
/// two levels down still says which level it is on.
fn under(entry: &Entry) -> &str {
    match entry.path.split_once(" > ") {
        Some((_, rest)) => rest,
        None => entry.path.as_str(),
    }
}

/// One row of the help list: the state, the name, the control, the key and the help.
///
/// Returns the row's own button where it has one, so the keyboard can scroll to it. A
/// number has none: it is a slider the whole way across, and there is nothing sensible
/// for Enter to do to it.
fn row(
    ui: &mut egui::Ui,
    entry: &Entry,
    query: &str,
    picked: bool,
    chosen: &mut Option<Action>,
) -> Option<egui::Response> {
    if mark(ui, entry).is_some_and(|r| r.clicked()) {
        *chosen = Some(entry.action.clone());
    }
    // Five cells every time, whatever the row is, or the columns stop lining up and
    // the grid reads as a heap: the state, the name, the control, the key, the words.
    let name = highlight(ui, under(entry), query);
    let hit = match &entry.kind {
        // A number is set here rather than reached from here: the row is the slider.
        EntryKind::Number(n, value, _) => {
            ui.label(name);
            let r = n.range();
            let mut v = *value;
            // No `step_by` here either, and for the same reason: see `slider`.
            let drag = ui.add_enabled(
                entry.enabled,
                egui::Slider::new(&mut v, r.min..=r.max)
                    .custom_formatter(|x, _| menu::written(*n, x)),
            );
            if drag.changed() {
                *chosen = Some(Action::Set(*n, v));
            }
            None
        }
        // A choice is picked here too, from the values the row carries.
        EntryKind::Choice(picks) => {
            ui.label(name);
            let current = picks
                .iter()
                .find(|p| p.chosen)
                .map_or("—", |p| p.label.as_str());
            ui.add_enabled_ui(entry.enabled, |ui| {
                egui::ComboBox::from_id_salt(&entry.path)
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for p in picks {
                            let hit = ui.add_enabled(
                                p.enabled,
                                egui::Button::selectable(p.chosen, p.label.as_str()),
                            );
                            if hit.on_hover_text(p.help).clicked() {
                                *chosen = Some(p.action.clone());
                            }
                        }
                    });
            });
            None
        }
        _ => {
            let hit = ui.add_enabled(entry.enabled, egui::Button::new(name).selected(picked));
            if hit.clicked() {
                *chosen = Some(entry.action.clone());
            }
            // Nothing in the control column: a command is done by its own row, and a
            // switch by the box at the head of it.
            ui.label("");
            Some(hit)
        }
    };
    ui.label(chip(entry.shortcut));
    ui.horizontal(|ui| {
        // The revert sits with the help rather than in a column of its own, because
        // most rows have none and an empty column down the whole list reads as damage.
        if let Some(back) = &entry.reset {
            if ui
                .small_button("↺")
                .on_hover_text("Put this back to what the app ships with")
                .clicked()
            {
                *chosen = Some(back.clone());
            }
        } else {
            ui.label(egui::RichText::new("  ").small());
        }
        ui.label(highlight(ui, entry.help, query));
    });
    hit
}

/// The state of a switch or a choice, down the left of the help list: the same box or
/// dot the menu draws, so the two read alike.
///
/// Drawn by egui rather than written as a character. Written, a filled dot came out as
/// an empty box on this machine: the fonts egui bundles have no glyph for it, and a
/// list of settings whose "on" mark is a missing-glyph box is worse than no mark.
/// Clicking one does what clicking the row does, so there is nothing here that looks
/// like a second, disagreeing switch.
fn mark(ui: &mut egui::Ui, entry: &Entry) -> Option<egui::Response> {
    match entry.kind {
        EntryKind::Command | EntryKind::Choice(_) | EntryKind::Number(..) => {
            ui.label("");
            None
        }
        EntryKind::Check(on) => {
            let mut shown = on;
            Some(ui.add_enabled(entry.enabled, egui::Checkbox::without_text(&mut shown)))
        }
        EntryKind::Radio(chosen) => {
            Some(ui.add_enabled(entry.enabled, egui::RadioButton::new(chosen, "")))
        }
    }
}

/// The page for everything the menu cannot say, because none of it is a menu item: what
/// the parts of the picture are, and what the mouse does to them.
fn how_it_works(ui: &mut egui::Ui) {
    let para = |ui: &mut egui::Ui, text: &str| {
        ui.label(text);
        ui.add_space(6.0);
    };
    // A grid rather than a row of two labels: the second column has to wrap, and a
    // label that wraps inside a horizontal layout takes the whole width for itself.
    let pair = |ui: &mut egui::Ui, id: &str, rows: &[(&str, &str)]| {
        egui::Grid::new(id)
            .num_columns(2)
            .spacing([16.0, 8.0])
            .show(ui, |ui| {
                for (name, text) in rows {
                    ui.with_layout(
                        egui::Layout::top_down(egui::Align::LEFT).with_main_wrap(false),
                        |ui| {
                            ui.set_min_width(150.0);
                            ui.label(egui::RichText::new(*name).strong());
                        },
                    );
                    ui.label(*text);
                    ui.end_row();
                }
            });
    };

    ui.heading("The picture");
    ui.add_space(4.0);
    para(
        ui,
        "Sonorant listens to whatever the machine is playing and draws it. Nothing here \
         changes the sound; it is a window onto it.",
    );
    pair(
        ui,
        "how-picture",
        &[
            (
                "The panes",
                "One per channel, left and right. Frequency runs up the pane and time runs \
                 across it, so a held note is a horizontal line and a drum is a vertical \
                 one. Colour is level.",
            ),
            (
                "The gutter",
                "The labelled strip between the panes, carrying the frequency axis. With \
                 room to spare the same labels repeat at both outer edges.",
            ),
            (
                "The spectrum",
                "The strip beside each image: the level in every band right now, measured \
                 from the edge nearest the newest column.",
            ),
            (
                "The scale strip",
                "The reserved band at the end of the panes carrying the time marks and the \
                 level numbers. It can sit at either end, or at both.",
            ),
            (
                "The deck",
                "The panel between the panes: artwork, the track, the transport, the \
                 goniometer, and the loudness figures.",
            ),
            (
                "The lanes",
                "The waveform along the bottom: level against time, on the same time axis \
                 as the image above it.",
            ),
            (
                "The quick bar",
                "The row of buttons over the image, for the switches reached most often. \
                 Every one of them is a menu item as well.",
            ),
            (
                "The status line",
                "What is being captured, loudness and tempo, the frame rate, the refreshes \
                 missed, the GPU's time, and how far the picture is behind the sound.",
            ),
        ],
    );

    ui.add_space(14.0);
    ui.heading("The mouse");
    ui.add_space(4.0);
    pair(
        ui,
        "how-mouse",
        &[
            (
                "Point at a pane",
                "Reads out the frequency under the pointer as hertz and as a note with its \
                 deviation in cents, each channel's level there, and how far back in time \
                 the column is.",
            ),
            (
                "Wheel over the image",
                "Zooms time about the pointer, so the history spreads out or packs in. The \
                 status line says the zoom while it is not 1x.",
            ),
            (
                "Drag across the image",
                "Walks back through the history. The image parks where it is left; End, or \
                 Live in the menu, brings it back to now.",
            ),
            (
                "Drag with the waterfall on",
                "Orbits the camera instead. The wheel moves the eye in and out.",
            ),
            (
                "Double-click a column",
                "Sends the player to that moment, where the player can be seeked.",
            ),
            (
                "Right-click anywhere",
                "The menu: presets, what is captured, and every setting. It stays open \
                 while you use it, so a switch can be tried against the last.",
            ),
        ],
    );

    ui.add_space(14.0);
    ui.heading("Settings and presets");
    ui.add_space(4.0);
    para(
        ui,
        "Settings are written when the app closes, so they are as you left them next \
         time. A preset is a whole set of them saved under a name: the built-in ones are \
         in the Presets menu, and Save these settings puts yours beside them. Switching \
         away from changes you have not saved asks first.",
    );
    para(
        ui,
        "Default in that menu, and Reset every setting, both put back what the app ships \
         with. On the Commands page, Changed lists everything that differs from it, and \
         the arrow on a row puts that one setting back on its own.",
    );
}

/// A key as a chip, or a dash where a command has no key./// A key as a chip, or a dash where a command has no key.
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

#[cfg(test)]
mod tests {
    use super::*;
    use sonorant_core::settings::{Flag, Preset};

    /// A shell that has just read its settings off disk: nothing unsaved yet.
    fn shell_for(settings: &Settings) -> Shell {
        let mut shell = Shell::default();
        shell.settled(settings);
        shell
    }

    #[test]
    fn a_switch_that_would_lose_changes_asks_first() {
        let mut s = Settings::default();
        let mut shell = shell_for(&s);
        assert!(!shell.unsaved(&s));

        // Nothing to lose, so the preset lands without a word.
        shell.act(&Action::UsePreset(Preset::QC), &mut s);
        assert_eq!(s.preset, Preset::QC);
        assert!(shell.confirming.is_none());
        // And a preset is itself a place to get back to.
        assert!(!shell.unsaved(&s));

        // Now there is something to lose.
        shell.act(&Action::Toggle(Flag::Glow), &mut s);
        assert!(shell.unsaved(&s));
        let before = s.clone();
        shell.act(&Action::UsePreset(Preset::Bass), &mut s);
        assert_eq!(s, before, "the switch waits for an answer");
        assert!(shell.confirming.is_some());
        assert!(shell.close_the_menu, "the dialog would be behind the menu");
    }

    #[test]
    fn only_the_three_that_replace_everything_are_guarded() {
        let mut s = Settings::default();
        let mut shell = shell_for(&s);
        shell.act(&Action::Toggle(Flag::Glow), &mut s);
        assert!(shell.unsaved(&s));

        for action in [
            Action::UsePreset(Preset::QC),
            Action::LoadPreset("mine".to_owned()),
            Action::ResetSettings,
        ] {
            assert!(
                shell.would_lose_changes(&action, &s).is_some(),
                "{action:?} should ask"
            );
        }
        // Quitting writes the settings on the way out, so nothing is lost by leaving,
        // and everything else is one setting rather than all of them.
        for action in [
            Action::Quit,
            Action::Toggle(Flag::ShowGrid),
            Action::SavePreset,
            Action::Freeze,
        ] {
            assert!(
                shell.would_lose_changes(&action, &s).is_none(),
                "{action:?} should not ask"
            );
        }
    }

    #[test]
    fn saving_first_carries_the_switch_out_afterwards() {
        let mut s = Settings::default();
        let mut shell = shell_for(&s);
        shell.act(&Action::Toggle(Flag::Glow), &mut s);
        shell.act(&Action::UsePreset(Preset::Bass), &mut s);

        // What the Save button does: the name dialog opens, holding the switch.
        let (action, _) = shell.confirming.take().expect("the warning");
        shell.after_saving = Some(action);
        shell.naming = Some((String::new(), None));

        // What answering the name dialog does.
        shell.asks.push(Ask::Save("mine".to_owned()));
        shell.naming = None;
        let held = shell.after_saving.take().expect("the held switch");
        shell.saved = Some(s.clone());
        shell.run(&held, &mut s);

        assert_eq!(s.preset, Preset::Bass, "the switch went ahead");
        assert!(!shell.unsaved(&s));
    }

    #[test]
    fn a_reset_leaves_nothing_unsaved() {
        let mut s = Settings::default();
        let mut shell = shell_for(&s);
        shell.act(&Action::Toggle(Flag::Glow), &mut s);
        assert!(shell.unsaved(&s));
        // The warning, then the answer.
        shell.act(&Action::ResetSettings, &mut s);
        shell.confirming = None;
        shell.run(&Action::ResetSettings, &mut s);
        assert_eq!(s, Settings::default());
        assert!(!shell.unsaved(&s));
    }

    /// The popup memory is egui's, so this is what closing the menu really comes down
    /// to. The window event that calls it is winit's and can't be raised from a test;
    /// what can go wrong here is reaching for the wrong door, not the door sticking.
    #[test]
    fn closing_the_menu_clears_the_popup_egui_is_holding() {
        let ctx = egui::Context::default();
        let shell = Shell::default();
        let id = egui::Id::new("a menu");
        egui::Popup::open_id(&ctx, id);
        assert!(egui::Popup::is_id_open(&ctx, id));
        shell.close_menu(&ctx);
        assert!(!egui::Popup::is_any_open(&ctx));
    }
}
