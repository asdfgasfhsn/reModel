//! The in-game menu (Esc) and short notices, drawn with egui over the game.
//! Its Controls page remaps the game's keys and gamepad bindings
//! (`romset::bindings`): click a binding, then press what should replace it.

use crate::settings::Settings;
use romset::bindings::{Accepts, Bindings, Slot, Value};
use std::time::{Duration, Instant, SystemTime};

/// Save slots shown in the menu; slot 0 is the quick slot (F5 / F9).
pub const SLOTS: u8 = 5;

pub enum Action {
    Resume,
    Save(u8),
    Load(u8),
    /// Switch to the game with this set name.
    Game(String),
    /// Look for games in the ROM folder in the settings again.
    Rescan,
    /// The game's bindings changed (save them).
    Bindings,
    Quit,
}

#[derive(Default)]
pub struct Menu {
    pub open: bool,
    notice: Option<(String, Instant)>,
    /// The ROM folder being edited.
    pub rom_text: String,
    /// On the Controls page.
    pub controls: bool,
    /// Waiting for a key, button or stick to bind into a slot: in place of
    /// its nth binding, or added (None).
    pub capture: Option<(Slot, Option<usize>)>,
    /// The Controls page lists inputs the manifest leaves unbound too.
    show_unbound: bool,
}

/// What the menu shows besides the settings.
pub struct View<'a> {
    /// Title of the game running, if any.
    pub playing: Option<&'a str>,
    /// Playable games in the ROM folder: (set, title).
    pub games: &'a [(String, &'static str)],
    /// Sets in the ROM folder reModel doesn't support yet.
    pub unsupported: usize,
    /// When each slot was saved, if it has been.
    pub slots: &'a [Option<SystemTime>],
}

fn age(t: SystemTime) -> String {
    let s = t.elapsed().unwrap_or_default().as_secs();
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86400 => format!("{} h ago", s / 3600),
        _ => format!("{} days ago", s / 86400),
    }
}

impl Menu {
    /// Show `text` briefly at the top of the screen.
    pub fn notify(&mut self, text: impl Into<String>) {
        self.notice = Some((text.into(), Instant::now()));
    }

    /// True while there's something to draw.
    pub fn visible(&self) -> bool {
        self.open || self.notice.as_ref().is_some_and(|n| n.1.elapsed() < Duration::from_millis(2500))
    }

    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        settings: &mut Settings,
        view: &View,
        bindings: Option<&mut Bindings>,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        if let Some((text, at)) = &self.notice {
            if at.elapsed() < Duration::from_millis(2500) {
                egui::Area::new(egui::Id::unique("notice")).anchor(egui::Align2::CENTER_TOP, [0.0, 24.0]).show(
                    ctx,
                    |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| ui.label(egui::RichText::new(text).size(18.0)));
                    },
                );
            }
        }
        if !self.open {
            self.controls = false;
            self.capture = None;
            return actions;
        }
        if let (true, Some(b)) = (self.controls, bindings) {
            self.controls_page(ctx, b, view.playing.unwrap_or(""), &mut actions);
            return actions;
        }
        self.controls = false;
        egui::Window::new("reModel")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(380.0);
                match view.playing {
                    Some(title) => {
                        ui.heading(title);
                        ui.horizontal(|ui| {
                            if ui.button("Resume  (Esc)").clicked() {
                                actions.push(Action::Resume);
                            }
                            if ui.button("Controls…").clicked() {
                                self.controls = true;
                            }
                        });
                    }
                    None => {
                        ui.heading("No game");
                        ui.label("Choose a ROM folder with a supported set below.");
                    }
                }

                if view.playing.is_some() {
                    ui.separator();
                    ui.strong("Save states");
                    ui.label("F5 quick save, F9 quick load");
                    egui::Grid::new("slots").num_columns(4).spacing([12.0, 4.0]).show(ui, |ui| {
                        for slot in 0..SLOTS {
                            ui.label(if slot == 0 { "Quick".to_string() } else { format!("Slot {slot}") });
                            let saved = view.slots.get(slot as usize).copied().flatten();
                            ui.label(saved.map_or("empty".into(), age));
                            if ui.button("Save").clicked() {
                                actions.push(Action::Save(slot));
                            }
                            if ui.add_enabled(saved.is_some(), egui::Button::new("Load")).clicked() {
                                actions.push(Action::Load(slot));
                            }
                            ui.end_row();
                        }
                    });
                }

                ui.separator();
                ui.strong("Sound");
                ui.add(
                    egui::Slider::new(&mut settings.sfx_db, -20.0..=20.0).step_by(1.0).suffix(" dB").text("effects"),
                );
                ui.add(
                    egui::Slider::new(&mut settings.music_db, -30.0..=10.0).step_by(1.0).suffix(" dB").text("music"),
                );

                ui.separator();
                ui.strong("Display");
                ui.horizontal(|ui| {
                    ui.label("Aspect:");
                    ui.radio_value(&mut settings.wide, true, "fit the window");
                    ui.radio_value(&mut settings.wide, false, "4:3");
                });
                ui.checkbox(&mut settings.bezel, "Bezel  (beside a 4:3 picture in a wider window)");
                ui.horizontal(|ui| {
                    ui.label("HUD on wide screens:");
                    ui.radio_value(&mut settings.hud_edges, true, "at the edges");
                    ui.radio_value(&mut settings.hud_edges, false, "centred");
                });
                ui.checkbox(&mut settings.exact, "Exact arcade speed (57.5 fps; off: one frame per refresh, smoother)");
                ui.checkbox(&mut settings.fullscreen, "Fullscreen  (F11)");

                ui.separator();
                ui.strong("Game");
                ui.horizontal(|ui| {
                    ui.label("ROM folder:");
                    ui.text_edit_singleline(&mut self.rom_text);
                    if ui.button("Scan").clicked() {
                        settings.rom = self.rom_text.trim().into();
                        actions.push(Action::Rescan);
                    }
                });
                if view.games.is_empty() {
                    ui.label("No supported sets there (MAME zips or folders, e.g. topskatr.zip).");
                }
                for (set, title) in view.games {
                    let current = view.playing.is_some() && *set == settings.game;
                    if ui.selectable_label(current, format!("{title}  ({set})")).clicked() && !current {
                        actions.push(Action::Game(set.clone()));
                    }
                }
                if view.unsupported > 0 {
                    ui.weak(format!("{} other sets found, not supported yet", view.unsupported));
                }

                ui.separator();
                if ui.button("Quit").clicked() {
                    actions.push(Action::Quit);
                }
            });
        actions
    }

    /// The Controls page: every input with its bindings, to remap.
    fn controls_page(&mut self, ctx: &egui::Context, b: &mut Bindings, title: &str, actions: &mut Vec<Action>) {
        let mut changed = false;
        let max_h = ctx.content_rect().height() * 0.6;
        egui::Window::new("Controls")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(520.0);
                ui.horizontal(|ui| {
                    if ui.button("← Back").clicked() {
                        self.controls = false;
                        self.capture = None;
                    }
                    ui.heading(title);
                });
                ui.label(
                    "Click a binding, then press a key or gamepad button (for an axis, move a stick or \
                     press a trigger: the way that should read +). + adds one, × removes it. Esc cancels; \
                     Esc, F5, F9 and F11 are the player's own. Saved for this game.",
                );
                if let Some((slot, _)) = self.capture {
                    let what = if b.accepts(slot) == Accepts::Axis {
                        "move a stick or trigger"
                    } else {
                        "press a key or button"
                    };
                    ui.colored_label(egui::Color32::YELLOW, format!("{}: {what}…  (Esc cancels)", b.label(slot)));
                }
                ui.separator();
                let slots: Vec<(Slot, Value)> = b.slots().to_vec();
                let hidden = slots
                    .iter()
                    .filter(|(s, v)| *v == Value::Inputs(Vec::new()) && b.default_of(*s) == Some(v))
                    .count();
                egui::ScrollArea::vertical().max_height(max_h).show(ui, |ui| {
                    egui::Grid::new("bindings").num_columns(3).spacing([12.0, 6.0]).striped(true).show(ui, |ui| {
                        for (slot, value) in &slots {
                            let unbound = *value == Value::Inputs(Vec::new()) && b.default_of(*slot) == Some(value);
                            if unbound && !self.show_unbound {
                                continue;
                            }
                            ui.label(b.label(*slot));
                            ui.horizontal_wrapped(|ui| {
                                let current = self.capture;
                                let capturing = |at| current == Some((*slot, at));
                                match value {
                                    Value::Axis(a) => {
                                        let text = if capturing(None) {
                                            "…".to_string()
                                        } else if a.axis == romset::PadAxis::None {
                                            "none".to_string()
                                        } else {
                                            a.axis.label().to_string()
                                        };
                                        if ui.button(text).clicked() {
                                            self.capture = Some((*slot, None));
                                        }
                                        if a.axis != romset::PadAxis::None {
                                            let mut inv = a.invert;
                                            if ui.checkbox(&mut inv, "invert").changed() {
                                                b.invert(*slot);
                                                changed = true;
                                            }
                                            if ui.small_button("×").on_hover_text("No axis").clicked() {
                                                b.remove(*slot, 0);
                                                changed = true;
                                            }
                                        }
                                    }
                                    Value::Inputs(list) => {
                                        for (i, input) in list.iter().enumerate() {
                                            let text =
                                                if capturing(Some(i)) { "…".to_string() } else { input.text() };
                                            if ui
                                                .button(text)
                                                .on_hover_text("Click, then press its replacement")
                                                .clicked()
                                            {
                                                self.capture = Some((*slot, Some(i)));
                                            }
                                            if ui.small_button("×").on_hover_text("Remove").clicked() {
                                                b.remove(*slot, i);
                                                changed = true;
                                            }
                                        }
                                        let add = if capturing(None) { "…" } else { "+" };
                                        if ui.button(add).on_hover_text("Add a binding").clicked() {
                                            self.capture = Some((*slot, None));
                                        }
                                    }
                                }
                                if !b.is_default(*slot) && ui.small_button("reset").clicked() {
                                    b.reset(*slot);
                                    changed = true;
                                }
                            });
                            let others: Vec<String> = b
                                .conflicts_of(*slot)
                                .iter()
                                .map(|c| {
                                    let with: Vec<String> =
                                        c.slots.iter().filter(|s| *s != slot).map(|&s| b.label(s)).collect();
                                    format!("{} also: {}", c.input.text(), with.join(", "))
                                })
                                .collect();
                            if others.is_empty() {
                                ui.label("");
                            } else {
                                ui.colored_label(
                                    egui::Color32::from_rgb(255, 170, 60),
                                    format!("⚠ {}", others.join("; ")),
                                );
                            }
                            ui.end_row();
                        }
                    });
                });
                ui.separator();
                if !b.conflicts().is_empty() {
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 170, 60),
                        "⚠ Some keys or buttons are bound twice: both inputs get them.",
                    );
                }
                ui.horizontal(|ui| {
                    if ui.button("Reset to defaults").clicked() {
                        b.reset_all();
                        self.capture = None;
                        changed = true;
                    }
                    if hidden > 0 {
                        ui.checkbox(&mut self.show_unbound, format!("Show unbound inputs ({hidden})"));
                    }
                });
            });
        if changed {
            actions.push(Action::Bindings);
        }
    }
}
