//! The in-game menu (Esc) and short notices, drawn with egui over the game.

use crate::settings::Settings;
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
    Quit,
}

#[derive(Default)]
pub struct Menu {
    pub open: bool,
    notice: Option<(String, Instant)>,
    /// The ROM folder being edited.
    pub rom_text: String,
}

/// What the menu shows besides the settings.
pub struct View<'a> {
    /// Title of the game running, if any.
    pub playing: Option<&'a str>,
    /// Playable games in the ROM folder: (set, title).
    pub games: &'a [(String, &'static str)],
    /// Sets in the ROM folder Remodel doesn't support yet.
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

    pub fn ui(&mut self, ctx: &egui::Context, settings: &mut Settings, view: &View) -> Vec<Action> {
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
            return actions;
        }
        egui::Window::new("Remodel")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(380.0);
                match view.playing {
                    Some(title) => {
                        ui.heading(title);
                        if ui.button("Resume  (Esc)").clicked() {
                            actions.push(Action::Resume);
                        }
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
}
