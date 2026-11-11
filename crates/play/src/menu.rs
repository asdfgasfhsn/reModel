//! The in-game menu (Esc) and short notices, drawn with egui over the game.
//! Its Controls page (also the launcher's) remaps the game's keys and
//! gamepad bindings (`romset::bindings`): click a binding, then press what
//! should replace it.
//!
//! From a gamepad (`focus`): the d-pad or left stick moves between the
//! widgets, A activates, left and right change a slider or radio group, B
//! goes back (closing the menu), Start resumes.

use crate::focus::Focus;
use crate::launcher::Nav;
use crate::settings::GameSettings;
use romset::bindings::{Accepts, Bindings, Slot, Value};
use std::time::{Duration, Instant, SystemTime};

/// Save slots shown in the menu; slot 0 is the quick slot (F5 / F9).
pub const SLOTS: u8 = 5;
/// The Controls page waits this long for a binding, then gives up.
pub const CAPTURE_WAIT: Duration = Duration::from_secs(10);

pub enum Action {
    Resume,
    Save(u8),
    Load(u8),
    /// Leave the game for the launcher.
    Games,
    /// The game's bindings changed (save them).
    Bindings,
    Quit,
}

#[derive(Default)]
pub struct Menu {
    pub open: bool,
    notice: Option<(String, Instant)>,
    /// On the Controls page.
    pub controls: bool,
    /// Waiting for a key, button or stick to bind into a slot: in place of
    /// its nth binding, or added (None).
    pub capture: Option<(Slot, Option<usize>)>,
    /// The Controls page lists inputs the manifest leaves unbound too.
    show_unbound: bool,
    /// Since when the Controls page has waited for `capture`.
    capture_since: Option<((Slot, Option<usize>), Instant)>,
    /// The last input came from a gamepad: its focus glows, hints show.
    pub pad_used: bool,
    /// The pad's focus on the menu's page and on the Controls page.
    focus: Focus,
    controls_focus: Focus,
}

/// Player-wide settings the menu changes besides the game's.
pub struct Prefs<'a> {
    pub fullscreen: &'a mut bool,
    /// Changes are kept for all games (else for the one playing).
    pub for_all: &'a mut bool,
}

/// What the menu shows besides the settings.
pub struct View<'a> {
    /// Title of the game running, if any.
    pub playing: Option<&'a str>,
    /// When each slot was saved, if it has been.
    pub slots: &'a [Option<SystemTime>],
    /// The game's own input lag in frames (`romset::Game::input_lag`).
    pub input_lag: Option<u8>,
    /// Anisotropic filtering the GPU has (`gpu::max_anisotropy`): the
    /// texture filters offered.
    pub max_anisotropy: u8,
    /// The GPU can do temporal anti-aliasing (`gpu::temporal_supported`).
    pub temporal_supported: bool,
    /// How the texture pack is doing (where it is, what it replaced), or
    /// why there is none, while the setting is on.
    pub pack: Option<&'a str>,
    /// What material maps did in the last frame, or why they have no
    /// effect, while the setting is on.
    pub materials: Option<&'a str>,
}

/// Temporal anti-aliasing's choices: render scale in percent, label.
const TEMPORAL: [(u8, &str); 5] = [(0, "off"), (100, "TAA"), (75, "75%"), (67, "67%"), (50, "50%")];

/// Smooth shading's choices.
const SHADING: [(gpu::Shading, &str); 3] =
    [(gpu::Shading::Flat, "flat"), (gpu::Shading::Vertex, "vertex"), (gpu::Shading::Pixel, "pixel")];

/// Material maps' rules: relief strengths and the faithful rule (`off`).
pub const RELIEF: [(&str, &str); 5] = [("1", "1"), ("2", "2"), ("4", "4"), ("8", "8"), ("faithful", "off")];

/// What the shading setting does as set (said beside its choices).
fn shading_hint(v: &GameSettings) -> &'static str {
    match v.shading {
        _ if v.shading != gpu::Shading::Flat && v.crease == 0 => "(crease 0: nothing is smoothed)",
        gpu::Shading::Flat => "(the game's own: one brightness a polygon)",
        gpu::Shading::Vertex => "(brightness blended across curved surfaces)",
        gpu::Shading::Pixel if v.geo_chip => "(with the chip: drawn as vertex; per pixel needs the native geometrizer)",
        gpu::Shading::Pixel => "(the game's lighting worked out for every pixel)",
    }
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

    /// The notice, if one is showing.
    pub fn notices(&self, ctx: &egui::Context) {
        if let Some((text, at)) = &self.notice {
            if at.elapsed() < Duration::from_millis(2500) {
                egui::Area::new(egui::Id::unique("notice"))
                    .order(egui::Order::Foreground)
                    .anchor(egui::Align2::CENTER_TOP, [0.0, 24.0])
                    .show(ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| ui.label(egui::RichText::new(text).size(18.0)));
                    });
                ctx.request_repaint();
            }
        }
    }

    /// Note where this frame's input came from: `navs` (the pad's) turn the
    /// pad's focus and hints on, the keyboard or mouse off.
    fn note_input(&mut self, ctx: &egui::Context, navs: &[Nav]) {
        let other = ctx.input(|i| {
            i.pointer.delta() != egui::Vec2::ZERO
                || i.events.iter().any(|e| matches!(e, egui::Event::Key { .. } | egui::Event::PointerButton { .. }))
        });
        if !navs.is_empty() {
            self.pad_used = true;
        } else if other {
            self.pad_used = false;
        }
    }

    /// Draw the menu (when open) and the notices; `navs` are the pad's
    /// this frame.
    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        settings: &mut GameSettings,
        prefs: Prefs,
        view: &View,
        bindings: Option<&mut Bindings>,
        navs: &[Nav],
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        self.notices(ctx);
        if !self.open {
            self.controls = false;
            self.capture = None;
            self.focus = Focus::default();
            self.controls_focus = Focus::default();
            return actions;
        }
        self.note_input(ctx, navs);
        // Start resumes from either page.
        if self.capture.is_none() && view.playing.is_some() && navs.contains(&Nav::Start) {
            actions.push(Action::Resume);
            return actions;
        }
        if let (true, Some(b)) = (self.controls, bindings) {
            if self.controls_page(ctx, b, view.playing.unwrap_or(""), navs) {
                actions.push(Action::Bindings);
            }
            return actions;
        }
        if self.controls {
            self.controls = false;
        }
        self.focus.show = self.pad_used;
        for nav in self.focus.begin(navs) {
            if nav == Nav::Back && view.playing.is_some() {
                actions.push(Action::Resume);
            }
        }
        let focus = &mut self.focus;
        let mut to_controls = false;
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
                            if focus.button(ui, |ui| ui.button("Resume  (Esc)")) {
                                actions.push(Action::Resume);
                            }
                            if focus.button(ui, |ui| ui.button("Controls…")) {
                                to_controls = true;
                            }
                        });
                    }
                    None => {
                        ui.heading("No game");
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
                            if focus.button(ui, |ui| ui.button("Save")) {
                                actions.push(Action::Save(slot));
                            }
                            if focus.button(ui, |ui| ui.add_enabled(saved.is_some(), egui::Button::new("Load"))) {
                                actions.push(Action::Load(slot));
                            }
                            ui.end_row();
                        }
                    });
                }

                ui.separator();
                ui.strong("Sound");
                let db = |ui: &mut egui::Ui, focus: &mut Focus, v: &mut f32, range: (f32, f32), text: &str| {
                    let r = ui.add(egui::Slider::new(v, range.0..=range.1).step_by(1.0).suffix(" dB").text(text));
                    *v = (*v + focus.slider(ui, &r) as f32).clamp(range.0, range.1);
                };
                db(ui, focus, &mut settings.sfx_db, (-20.0, 20.0), "effects");
                db(ui, focus, &mut settings.music_db, (-30.0, 10.0), "music");

                ui.separator();
                ui.strong("Display");
                ui.horizontal(|ui| {
                    ui.label("Aspect:");
                    focus.radio(ui, "wide", &mut settings.wide, true, "fit the window");
                    focus.radio(ui, "wide", &mut settings.wide, false, "4:3");
                });
                focus.checkbox(ui, &mut settings.bezel, "Bezel  (beside a 4:3 picture in a wider window)");
                ui.horizontal(|ui| {
                    ui.label("HUD on wide screens:");
                    focus.radio(ui, "hud", &mut settings.hud_edges, true, "at the edges");
                    focus.radio(ui, "hud", &mut settings.hud_edges, false, "centred");
                });
                focus.checkbox(
                    ui,
                    &mut settings.exact,
                    "Exact arcade speed (57.5 fps; off: one frame per refresh, smoother)",
                );
                ui.add_enabled_ui(!settings.geo_chip, |ui| {
                    focus.checkbox(
                        ui,
                        &mut settings.smooth,
                        "Smooth motion (in-between frames above 60 Hz, a frame later)",
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Run-ahead:");
                    focus.radio(ui, "run_ahead", &mut settings.run_ahead, 0, "off");
                    for n in 1..=crate::settings::MAX_RUN_AHEAD {
                        let text = if n == 1 { "1 frame".to_string() } else { format!("{n} frames") };
                        focus.radio(ui, "run_ahead", &mut settings.run_ahead, n, text);
                    }
                    // What this game has to hide (more only judders).
                    ui.weak(match view.input_lag {
                        Some(0) => "(this game has no lag to hide: best off)".to_string(),
                        Some(1) => "(this game lags a frame: 1 at most)".to_string(),
                        Some(n) => format!(
                            "(this game lags {n} frames: {} at most)",
                            n.min(crate::settings::MAX_RUN_AHEAD as u8)
                        ),
                        None => "(less input lag, more CPU)".to_string(),
                    });
                });
                // Temporal anti-aliasing replaces multisampling while it's on.
                let temporal = settings.temporal_on() && view.temporal_supported;
                ui.horizontal(|ui| {
                    ui.label("Anti-aliasing:");
                    ui.add_enabled_ui(!temporal, |ui| {
                        focus.radio(ui, "aa", &mut settings.aa, 0, "off");
                        focus.radio(ui, "aa", &mut settings.aa, 4, "4x");
                        focus.radio(ui, "aa", &mut settings.aa, 8, "8x");
                    });
                    ui.weak(if temporal { "(off while temporal is on)" } else { "(8x where the GPU has it)" });
                });
                ui.horizontal(|ui| {
                    ui.label("Temporal:");
                    ui.add_enabled_ui(!settings.geo_chip && view.temporal_supported, |ui| {
                        for (n, text) in TEMPORAL {
                            focus.radio(ui, "temporal", &mut settings.temporal, n, text);
                        }
                    });
                    ui.weak(if !view.temporal_supported {
                        "(this GPU can't: off)"
                    } else if settings.geo_chip {
                        "(off: needs the native geometrizer)"
                    } else {
                        "(a % draws smaller and upscales)"
                    });
                });
                ui.add_enabled_ui(temporal, |ui| {
                    focus.checkbox(
                        ui,
                        &mut settings.stipple_keep,
                        "Keep the checkerboard pattern  (else see-through as an even 50% blend)",
                    );
                });
                focus.checkbox(
                    ui,
                    &mut settings.depth,
                    "Depth buffer (polygons that cross sort right; off: the game's order)",
                );
                ui.horizontal(|ui| {
                    ui.label("Shading:");
                    for (s, text) in SHADING {
                        focus.radio(ui, "shading", &mut settings.shading, s, text);
                    }
                    ui.weak(shading_hint(settings));
                });
                ui.add_enabled_ui(settings.shading != gpu::Shading::Flat, |ui| {
                    focus.checkbox(
                        ui,
                        &mut settings.shading_fine,
                        "Blend brightness levels  (no banding on smooth surfaces; else the game's 64 levels)",
                    );
                    focus.checkbox(
                        ui,
                        &mut settings.shading_balance,
                        "Balance brightness  (each smooth surface as bright as the game's; off: bumpy ones brighter)",
                    );
                });
                focus.checkbox(ui, &mut settings.pack, "Texture pack  (replacement textures)");
                if let (true, Some(text)) = (settings.pack, view.pack) {
                    ui.weak(text);
                }
                ui.horizontal(|ui| {
                    ui.label("Texture filtering:");
                    for (label, v) in crate::settings::tex_filters(view.max_anisotropy) {
                        if let Some(f) = model2::enhance::TexFilter::parse(v) {
                            focus.radio(ui, "tex_filter", &mut settings.tex_filter, f, *label);
                        }
                    }
                });
                ui.weak("(of texture pack images: mipmaps stop the shimmer at a distance, anisotropic adds sharpness)");
                ui.add_enabled_ui(settings.pack, |ui| {
                    focus.checkbox(
                        ui,
                        &mut settings.materials,
                        "Material maps  (the pack's normal maps: bumps lit by the game's lights)",
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Material relief:");
                    ui.add_enabled_ui(settings.materials_on(), |ui| {
                        for (text, v) in RELIEF {
                            if let Some(r) = gpu::Relief::parse(v) {
                                focus.radio(ui, "materials_relief", &mut settings.materials_relief, r, text);
                            }
                        }
                    });
                    ui.weak("(strength, 2 by default; faithful: the game's own lighting sum, subtler)");
                });
                if let (true, Some(text)) = (settings.materials, view.materials) {
                    ui.weak(text);
                }
                ui.horizontal(|ui| {
                    ui.label("Geometrizer:");
                    focus.radio(ui, "geo", &mut settings.geo_chip, false, "native");
                    focus.radio(ui, "geo", &mut settings.geo_chip, true, "the game's chip");
                    ui.weak("(on the next start)");
                });
                focus.checkbox(ui, prefs.fullscreen, "Fullscreen  (F11)");
                ui.horizontal(|ui| {
                    ui.label("Save changes for:");
                    focus.radio(ui, "for_all", prefs.for_all, false, "this game");
                    focus.radio(ui, "for_all", prefs.for_all, true, "all games");
                });

                ui.separator();
                ui.horizontal(|ui| {
                    if focus.button(ui, |ui| ui.button("Back to games")) {
                        actions.push(Action::Games);
                    }
                    if focus.button(ui, |ui| ui.button("Quit")) {
                        actions.push(Action::Quit);
                    }
                });
                if focus.show {
                    ui.separator();
                    hints(
                        ui,
                        &[("D-pad", "choose"), ("A", "select"), ("← →", "change"), ("B", "back"), ("Start", "resume")],
                    );
                }
            });
        self.focus.end();
        if to_controls {
            self.controls = true;
            self.controls_focus = Focus::default();
        }
        actions
    }

    /// The Controls page: every input with its bindings, to remap (`navs`:
    /// the pad's; B goes back). Returns whether they changed (to save
    /// them).
    pub fn controls_page(&mut self, ctx: &egui::Context, b: &mut Bindings, title: &str, navs: &[Nav]) -> bool {
        let mut changed = false;
        self.note_input(ctx, navs);
        // Waiting too long for a binding gives up (a way out for a pad
        // without a free Guide button).
        match (self.capture, self.capture_since) {
            (None, _) => self.capture_since = None,
            (Some(c), Some((was, at))) if c == was => {
                if at.elapsed() >= CAPTURE_WAIT {
                    self.capture = None;
                    self.capture_since = None;
                    self.notify("Nothing bound");
                }
            }
            (Some(c), _) => self.capture_since = Some((c, Instant::now())),
        }
        let navs = if self.capture.is_some() { &[][..] } else { navs };
        let focus = &mut self.controls_focus;
        focus.show = self.pad_used;
        let mut back = focus.begin(navs).contains(&Nav::Back);
        let max_h = ctx.content_rect().height() * 0.6;
        let (capture, show_unbound, pad_used) = (&mut self.capture, &mut self.show_unbound, self.pad_used);
        let waited = self.capture_since.map_or(0, |(_, at)| at.elapsed().as_secs());
        egui::Window::new("Controls")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(520.0);
                ui.horizontal(|ui| {
                    if focus.button(ui, |ui| ui.button("← Back")) {
                        back = true;
                    }
                    ui.heading(title);
                });
                ui.label(
                    "Click a binding, then press a key or gamepad button (for an axis, move a stick or \
                     press a trigger: the way that should read +). + adds one, × removes it. Esc or the \
                     pad's Guide button cancels, as does waiting; Esc, F5, F9 and F11 are the player's \
                     own. Saved for this game.",
                );
                if let Some((slot, _)) = *capture {
                    let what = if b.accepts(slot) == Accepts::Axis {
                        "move a stick or trigger"
                    } else {
                        "press a key or button"
                    };
                    let left = CAPTURE_WAIT.as_secs().saturating_sub(waited);
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        format!("{}: {what}…  (Esc or Guide cancels; {left} s)", b.label(slot)),
                    );
                    ctx.request_repaint();
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
                            if unbound && !*show_unbound {
                                continue;
                            }
                            ui.label(b.label(*slot));
                            ui.horizontal_wrapped(|ui| {
                                // A grid column only grows to what it held
                                // last frame: room to wrap in.
                                ui.set_min_width(300.0);
                                let current = *capture;
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
                                        if focus.button(ui, |ui| ui.button(text)) {
                                            *capture = Some((*slot, None));
                                        }
                                        if a.axis != romset::PadAxis::None {
                                            let mut inv = a.invert;
                                            if focus.checkbox(ui, &mut inv, "invert") {
                                                b.invert(*slot);
                                                changed = true;
                                            }
                                            if focus.button(ui, |ui| ui.small_button("×").on_hover_text("No axis")) {
                                                b.remove(*slot, 0);
                                                changed = true;
                                            }
                                        }
                                    }
                                    Value::Inputs(list) => {
                                        for (i, input) in list.iter().enumerate() {
                                            let text =
                                                if capturing(Some(i)) { "…".to_string() } else { input.text() };
                                            let hover = "Click, then press its replacement";
                                            if focus.button(ui, |ui| ui.button(text).on_hover_text(hover)) {
                                                *capture = Some((*slot, Some(i)));
                                            }
                                            if focus.button(ui, |ui| ui.small_button("×").on_hover_text("Remove")) {
                                                b.remove(*slot, i);
                                                changed = true;
                                            }
                                        }
                                        let add = if capturing(None) { "…" } else { "+" };
                                        if focus.button(ui, |ui| ui.button(add).on_hover_text("Add a binding")) {
                                            *capture = Some((*slot, None));
                                        }
                                    }
                                }
                                if !b.is_default(*slot) && focus.button(ui, |ui| ui.small_button("reset")) {
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
                    if focus.button(ui, |ui| ui.button("Reset to defaults")) {
                        b.reset_all();
                        *capture = None;
                        changed = true;
                    }
                    if hidden > 0 {
                        focus.checkbox(ui, show_unbound, format!("Show unbound inputs ({hidden})"));
                    }
                });
                if pad_used {
                    ui.separator();
                    match capture {
                        Some(_) => hints(ui, &[("any button", "binds it"), ("Guide", "cancel")]),
                        None => hints(ui, &[("D-pad", "choose"), ("A", "rebind"), ("B", "back")]),
                    }
                }
            });
        focus.end();
        if back {
            self.controls = false;
            self.capture = None;
        }
        changed
    }
}

/// A line of pad hints: each button's cap and what it does.
fn hints(ui: &mut egui::Ui, hints: &[(&str, &str)]) {
    use crate::launcher::{keycap, DIM, FG};
    let key_font = egui::FontId::monospace(12.0);
    let font = egui::FontId::proportional(13.5);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (i, (key, what)) in hints.iter().enumerate() {
            if i > 0 {
                ui.add_space(12.0);
            }
            let w = ui.painter().layout_no_wrap(key.to_string(), key_font.clone(), FG).size().x + 12.0;
            let (r, _) = ui.allocate_exact_size(egui::vec2(w, 22.0), egui::Sense::hover());
            keycap(ui.painter(), r, key, &key_font, 1.0);
            ui.label(egui::RichText::new(*what).font(font.clone()).color(DIM));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The menu over a game, headless: a frame per `frames` entry (the
    /// pad's `Nav`s that frame), after one to lay it out; the actions taken.
    fn run(menu: &mut Menu, settings: &mut GameSettings, b: &mut Bindings, frames: &[&[Nav]]) -> Vec<Action> {
        let ctx = egui::Context::default();
        let mut actions = Vec::new();
        let slots = [Some(SystemTime::now()), None, None, None, None];
        let view = View {
            playing: Some("A game"),
            slots: &slots,
            input_lag: Some(2),
            max_anisotropy: 16,
            temporal_supported: true,
            pack: None,
            materials: None,
        };
        let (mut fullscreen, mut for_all) = (false, false);
        for navs in [&[][..]].iter().chain(frames) {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0))),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                let prefs = Prefs { fullscreen: &mut fullscreen, for_all: &mut for_all };
                actions.extend(menu.ui(ui.ctx(), settings, prefs, &view, Some(&mut *b), navs));
            });
            out.textures_delta.clear();
        }
        actions
    }

    fn setup() -> (Menu, GameSettings, Bindings) {
        let game = romset::games::find("topskatr").unwrap();
        let menu = Menu { open: true, ..Default::default() };
        (menu, GameSettings::default(), Bindings::defaults(&game.controls))
    }

    fn names(actions: &[Action]) -> Vec<&'static str> {
        actions
            .iter()
            .map(|a| match a {
                Action::Resume => "resume",
                Action::Save(_) => "save",
                Action::Load(_) => "load",
                Action::Games => "games",
                Action::Bindings => "bindings",
                Action::Quit => "quit",
            })
            .collect()
    }

    #[test]
    fn opens_on_resume() {
        let (mut m, mut s, mut b) = setup();
        assert_eq!(names(&run(&mut m, &mut s, &mut b, &[&[Nav::Accept]])), ["resume"]);
        let (mut m, mut s, mut b) = setup();
        assert_eq!(names(&run(&mut m, &mut s, &mut b, &[&[Nav::Start]])), ["resume"]);
        let (mut m, mut s, mut b) = setup();
        assert_eq!(names(&run(&mut m, &mut s, &mut b, &[&[Nav::Back]])), ["resume"], "B closes it");
    }

    #[test]
    fn down_to_back_to_games() {
        let (mut m, mut s, mut b) = setup();
        let mut frames: Vec<&[Nav]> = vec![&[Nav::Down]; 30];
        frames.push(&[Nav::Left]);
        frames.push(&[Nav::Accept]);
        assert_eq!(names(&run(&mut m, &mut s, &mut b, &frames)), ["games"]);
        // Right of it, at the foot, is Quit.
        let (mut m, mut s, mut b) = setup();
        let mut frames: Vec<&[Nav]> = vec![&[Nav::Down]; 30];
        frames.extend([&[Nav::Left][..], &[Nav::Right], &[Nav::Accept]]);
        assert_eq!(names(&run(&mut m, &mut s, &mut b, &frames)), ["quit"]);
    }

    #[test]
    fn save_and_load_slots() {
        let (mut m, mut s, mut b) = setup();
        // Down to the quick slot's Save, right to its Load (it's saved).
        let acts = run(&mut m, &mut s, &mut b, &[&[Nav::Down], &[Nav::Accept], &[Nav::Right], &[Nav::Accept]]);
        assert!(matches!(&acts[..], [Action::Save(0), Action::Load(0)]));
        // Slot 1 is empty: its Load can't be focused.
        let (mut m, mut s, mut b) = setup();
        let acts = run(&mut m, &mut s, &mut b, &[&[Nav::Down], &[Nav::Down], &[Nav::Right], &[Nav::Accept]]);
        assert!(matches!(&acts[..], [Action::Save(1)]));
    }

    #[test]
    fn sliders_checkboxes_and_radios() {
        let (mut m, mut s, mut b) = setup();
        let (sfx, music, wide, bezel) = (s.sfx_db, s.music_db, s.wide, s.bezel);
        // Resume, five slots, then the effects slider: right three, left one.
        let mut frames: Vec<&[Nav]> = vec![&[Nav::Down]; 6];
        frames.extend([&[Nav::Right][..], &[Nav::Right, Nav::Right], &[Nav::Left]]);
        // The music slider.
        frames.extend([&[Nav::Down][..], &[Nav::Left]]);
        // Aspect: step to the other choice; down to the bezel checkbox, A.
        let step = [if wide { Nav::Right } else { Nav::Left }];
        frames.extend([&[Nav::Down][..], &step, &[Nav::Down], &[Nav::Accept]]);
        run(&mut m, &mut s, &mut b, &frames);
        assert_eq!(s.sfx_db, sfx + 2.0);
        assert_eq!(s.music_db, music - 1.0);
        assert_eq!(s.wide, !wide);
        assert_eq!(s.bezel, !bezel);
    }

    #[test]
    fn temporal_turns_multisampling_off() {
        // From the effects slider down past the display rows to run-ahead;
        // with temporal on the anti-aliasing radios are off (skipped), so
        // the next is temporal's: right to 50%; then keep the stipple.
        let (mut m, mut s, mut b) = setup();
        s.temporal = 67;
        let mut frames: Vec<&[Nav]> = vec![&[Nav::Down]; 6 + 7];
        frames.extend([&[Nav::Down][..], &[Nav::Right], &[Nav::Down], &[Nav::Accept]]);
        run(&mut m, &mut s, &mut b, &frames);
        assert_eq!((s.aa, s.temporal, s.stipple_keep), (0, 50, true));
        // With the chip temporal can't run: its radios are off instead (and
        // smooth motion's checkbox, one row less).
        let (mut m, mut s, mut b) = setup();
        (s.temporal, s.geo_chip) = (67, true);
        let mut frames: Vec<&[Nav]> = vec![&[Nav::Down]; 6 + 6];
        frames.extend([&[Nav::Down][..], &[Nav::Right], &[Nav::Down], &[Nav::Right]]);
        run(&mut m, &mut s, &mut b, &frames);
        assert_eq!((s.aa, s.temporal, s.stipple_keep), (4, 67, false));
    }

    #[test]
    fn shading_row() {
        // From the effects slider down to run-ahead, then anti-aliasing,
        // temporal, (the stipple checkbox off without temporal) the depth
        // buffer and shading: right to vertex, right again to pixel.
        let (mut m, mut s, mut b) = setup();
        let mut frames: Vec<&[Nav]> = vec![&[Nav::Down]; 6 + 7 + 4];
        frames.push(&[Nav::Right]);
        run(&mut m, &mut s, &mut b, &frames);
        assert_eq!((s.shading, s.depth, s.aa, s.temporal), (gpu::Shading::Vertex, false, 0, 0));
        let (mut m, mut s, mut b) = setup();
        frames.push(&[Nav::Right]);
        run(&mut m, &mut s, &mut b, &frames);
        assert_eq!(s.shading, gpu::Shading::Pixel);
        // Pixel is per pixel with the native geometrizer; the chip doesn't
        // turn it off, but draws it as vertex and says so.
        assert!(!shading_hint(&s).contains("as vertex"));
        s.geo_chip = true;
        assert!(shading_hint(&s).contains("as vertex"));
        s.crease = 0;
        assert!(shading_hint(&s).contains("nothing"));
    }

    #[test]
    fn focus_glows_for_the_pad_only() {
        let ctx = egui::Context::default();
        let (mut m, mut s, _) = setup();
        let mut cyan_strokes = |m: &mut Menu, navs: &[Nav]| {
            let (mut f, mut a) = (false, false);
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0))),
                ..Default::default()
            };
            let view = View {
                playing: Some("A game"),
                slots: &[],
                input_lag: None,
                max_anisotropy: 16,
                temporal_supported: true,
                pack: None,
                materials: None,
            };
            let mut out = ctx.run_ui(input, |ui| {
                m.ui(ui.ctx(), &mut s, Prefs { fullscreen: &mut f, for_all: &mut a }, &view, None, navs);
            });
            out.textures_delta.clear();
            out.shapes
                .iter()
                // Cyan (fading in with the window).
                .filter(|c| {
                    matches!(&c.shape, egui::Shape::Rect(r)
                        if r.stroke.width == 1.5 && r.stroke.color.to_opaque().b() > 150 && r.stroke.color.r() < 100)
                })
                .count()
        };
        cyan_strokes(&mut m, &[]);
        assert_eq!(cyan_strokes(&mut m, &[]), 0, "keyboard and mouse: no glow");
        assert_eq!(cyan_strokes(&mut m, &[Nav::Down]), 1, "the pad: one");
    }

    #[test]
    fn controls_page_from_the_pad() {
        let (mut m, mut s, mut b) = setup();
        // Right to Controls…, into it; down to the first binding, A captures.
        let acts = run(&mut m, &mut s, &mut b, &[&[Nav::Right], &[Nav::Accept], &[], &[Nav::Down], &[Nav::Accept]]);
        assert!(acts.is_empty());
        assert!(m.controls);
        let first = b.slots()[0].0;
        assert_eq!(m.capture.map(|c| c.0), Some(first), "waiting for a binding");
        // While it waits, the pad's input is the binding's, not the page's.
        run(&mut m, &mut s, &mut b, &[&[Nav::Back]]);
        assert!(m.controls && m.capture.is_some());
        // Waiting too long gives up.
        m.capture_since = m.capture_since.map(|(c, at)| (c, at - CAPTURE_WAIT));
        run(&mut m, &mut s, &mut b, &[&[]]);
        assert!(m.capture.is_none());
        // B goes back to the menu, on Controls….
        run(&mut m, &mut s, &mut b, &[&[Nav::Back]]);
        assert!(!m.controls);
        let acts = run(&mut m, &mut s, &mut b, &[&[Nav::Left], &[Nav::Accept]]);
        assert_eq!(names(&acts), ["resume"]);
    }
}
