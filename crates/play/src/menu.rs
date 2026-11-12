//! The in-game menu (Esc) and short notices, drawn with egui over the game.
//!
//! The pause menu is a short list: Resume, Save / Load, Settings, Controls,
//! Back to games, Quit. Save / Load is a small screen of its own (the
//! slots). Settings come in tabs (Display, Picture, Motion, Sound, More),
//! each row "label ◂ value ▸": left and right change it, and one help line
//! at the bottom says what the focused (or pointed-at) row does. Options
//! that do nothing as things are set are left out, not greyed: a
//! picture preset (Original, Sharp, Enhanced, Custom) sets the picture
//! rows, and only Custom lists them. Every settings tab ends with where
//! the changes are kept (this game or all games).
//!
//! Its Controls page (also the launcher's) remaps the game's keys and
//! gamepad bindings (`romset::bindings`): click a binding, then press what
//! should replace it.
//!
//! From a gamepad (`focus`): the d-pad or left stick moves between the
//! widgets, A activates, left and right change a row, LB and RB switch
//! tabs, B goes back (closing the menu from the first screen), Start
//! resumes. The keyboard does the same: the arrow keys, Enter, Q and E (or
//! Tab and Shift+Tab) for tabs, Backspace to go back; Esc closes the menu.

use crate::focus::Focus;
use crate::launcher::{keycap, Nav, CYAN, DIM, FG};
use crate::settings::{GameSettings, Preset};
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, Sense, Shape};
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

/// The screens other than Controls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Pause,
    SaveLoad,
    Settings,
}

/// The settings tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Display,
    Picture,
    Motion,
    Sound,
    More,
}

impl Tab {
    pub const ALL: [Tab; 5] = [Tab::Display, Tab::Picture, Tab::Motion, Tab::Sound, Tab::More];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Display => "Display",
            Tab::Picture => "Picture",
            Tab::Motion => "Motion",
            Tab::Sound => "Sound",
            Tab::More => "More",
        }
    }

    /// The tab `d` places on, wrapping round.
    pub fn step(self, d: i32) -> Tab {
        let i = Tab::ALL.iter().position(|t| *t == self).unwrap_or(0) as i32;
        Tab::ALL[(i + d).rem_euclid(Tab::ALL.len() as i32) as usize]
    }
}

#[derive(Default)]
pub struct Menu {
    pub open: bool,
    notice: Option<(String, Instant)>,
    /// Which screen the menu is on (besides Controls) and which settings tab.
    pub page: Page,
    pub tab: Tab,
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
    /// The last input came from the keyboard's navigation keys: the focus
    /// glows.
    keys_used: bool,
    /// The focus on each screen.
    pause_focus: Focus,
    slots_focus: Focus,
    settings_focus: Focus,
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
    /// The game running, for what its manifest says (input lag, the most
    /// run-ahead worth having, the preset's choices).
    pub game: Option<&'static romset::Game>,
    /// When each slot was saved, if it has been.
    pub slots: &'a [Option<SystemTime>],
    /// Anisotropic filtering the GPU has (`gpu::max_anisotropy`): the
    /// texture filters offered.
    pub max_anisotropy: u8,
    /// The GPU can do temporal anti-aliasing (`gpu::temporal_supported`).
    pub temporal_supported: bool,
    /// The game has bezel art.
    pub has_bezel: bool,
    /// Run-ahead was switched off because the machine could not keep up.
    pub run_ahead_dropped: bool,
    /// How the texture pack is doing (where it is, what it replaced), or
    /// why there is none, while the setting is on.
    pub pack: Option<&'a str>,
    /// What material maps did in the last frame, or why they have no
    /// effect, while the setting is on.
    pub materials: Option<&'a str>,
}

/// Temporal anti-aliasing's choices: render scale in percent, label.
const TEMPORAL: [(u8, &str); 5] = [(0, "Off"), (100, "TAA"), (75, "75%"), (67, "67%"), (50, "50%")];

/// Smooth shading's choices.
const SHADING: [(gpu::Shading, &str); 3] =
    [(gpu::Shading::Flat, "Flat"), (gpu::Shading::Vertex, "Vertex"), (gpu::Shading::Pixel, "Pixel")];

/// Smooth shading's crease angles besides the game's own, in degrees.
const CREASES: [u8; 4] = [20, 30, 40, 60];

/// Material maps' rules: relief strengths and the faithful rule (`off`).
pub const RELIEF: [(&str, &str); 5] = [("1", "1"), ("2", "2"), ("4", "4"), ("8", "8"), ("faithful", "off")];

/// The label of one more relief choice when `r` is none of [`RELIEF`]'s
/// ("custom (1.5)"), so the menu shows the strength in effect.
fn custom_relief(r: gpu::Relief) -> Option<String> {
    (!RELIEF.iter().any(|(_, v)| gpu::Relief::parse(v) == Some(r))).then(|| format!("custom ({})", r.name()))
}

/// What the shading setting does as set (the help line of its row).
fn shading_hint(v: &GameSettings) -> &'static str {
    match v.shading {
        _ if v.shading != gpu::Shading::Flat && v.crease == 0 => "Crease 0: nothing is smoothed.",
        gpu::Shading::Flat => "The game's own: one brightness a polygon.",
        gpu::Shading::Vertex => "Brightness blended across curved surfaces.",
        gpu::Shading::Pixel if v.geo_chip => "With the chip drawn as vertex: per pixel needs the native geometrizer.",
        gpu::Shading::Pixel => "The game's lighting worked out for every pixel.",
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

/// The frames of run-ahead worth offering for `game` (none: the most any
/// game takes), and the most the row lists.
fn run_ahead_max(game: Option<&romset::Game>) -> u32 {
    game.map_or(crate::settings::MAX_RUN_AHEAD, |g| g.run_ahead_max as u32)
}

/// The help line of the run-ahead row.
fn run_ahead_help(game: Option<&romset::Game>, dropped: bool) -> String {
    let mut text = match game {
        None => "Hides input lag at the cost of CPU time.".to_string(),
        Some(g) if g.run_ahead_max > g.input_lag => format!(
            "Hides a frame of lag from the punches, and makes the stick judder: the stick has none to hide. \
             Costs about {}x the game's CPU time.",
            g.run_ahead_max + 1
        ),
        Some(g) => format!(
            "Hides {} of this game's {} of input lag, at the cost of CPU time (about {}x the game's); more only \
             judders.",
            frames(g.run_ahead_max as u32),
            frames(g.input_lag as u32),
            g.run_ahead_max + 1
        ),
    };
    if dropped {
        text += " Switched off: this machine could not keep up. Choosing it again tries again.";
    }
    text
}

fn frames(n: u32) -> String {
    if n == 1 {
        "1 frame".into()
    } else {
        format!("{n} frames")
    }
}

/// What a preset does for `game`, as the help line of the preset row.
fn preset_help(p: Preset, view: &View, geo_chip: bool) -> String {
    match p {
        Preset::Original => "The arcade look: nothing that changes the picture.".into(),
        Preset::Sharp => "4x anti-aliasing and mipmapped texture filtering, nothing else.".into(),
        Preset::Custom => "Choose each picture setting yourself.".into(),
        Preset::Enhanced => {
            let mut d = GameSettings::default();
            d.apply_preset(Preset::Enhanced, view.game);
            let mut parts = Vec::new();
            parts.push(match d.temporal {
                _ if d.temporal > 0 && geo_chip => "4x anti-aliasing (temporal needs the native geometrizer)",
                _ if d.temporal > 0 && !view.temporal_supported => "4x anti-aliasing (this GPU can't do temporal)",
                0 => "4x anti-aliasing",
                _ => "temporal anti-aliasing",
            });
            if d.depth {
                parts.push("the depth buffer");
            }
            if d.shading != gpu::Shading::Flat {
                parts.push("smooth shading");
            }
            if !geo_chip {
                parts.push("smooth motion");
            }
            let mut text = format!("For this game: {}", parts.join(", "));
            if d.run_ahead > 0 {
                text += &format!(", run-ahead {}", d.run_ahead);
            }
            text + "."
        }
    }
}

/// How a row was changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    None,
    Dec,
    Inc,
    /// On to the next, from the first again after the last (A, a click).
    Cycle,
}

/// The rows of a settings screen as they are drawn: the pad's focus, and
/// the help line of the row it's on (or the pointer).
struct Rows<'a> {
    focus: &'a mut Focus,
    /// The focus is what picks the help line, not the pointer.
    nav_mode: bool,
    focused: Option<String>,
    hovered: Option<String>,
}

impl Rows<'_> {
    /// One "label ◂ value ▸" row; `can`: whether there is a choice before
    /// and after.
    fn row(&mut self, ui: &mut egui::Ui, label: &str, value: &str, can: (bool, bool), help: &str) -> Step {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
        let r = ui.interact(rect, ui.make_persistent_id(("row", label)), Sense::CLICK);
        let (pad, accepted) = self.focus.stepper(ui, &r);
        let (arrow, value_w) = (26.0, 190.0);
        let right = Rect::from_min_max(pos2(rect.right() - arrow, rect.top()), rect.right_bottom());
        let mid = Rect::from_min_max(pos2(right.left() - value_w, rect.top()), pos2(right.left(), rect.bottom()));
        let left = Rect::from_min_max(pos2(mid.left() - arrow, rect.top()), pos2(mid.left(), rect.bottom()));
        let p = ui.painter();
        if r.hovered() && !self.nav_mode {
            p.rect_filled(rect, 4.0, Color32::from_white_alpha(10));
        }
        let font = FontId::proportional(15.5);
        p.text(rect.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, label, font.clone(), FG);
        p.text(mid.center(), Align2::CENTER_CENTER, value, font, FG);
        for (rect, dir, on) in [(left, -1.0, can.0), (right, 1.0, can.1)] {
            let c = rect.center();
            let color = if on { CYAN } else { DIM.gamma_multiply(0.35) };
            let tip = c + vec2(5.0 * dir, 0.0);
            let back = c - vec2(3.0 * dir, 0.0);
            p.add(Shape::convex_polygon(
                vec![tip, back + vec2(0.0, -6.0), back + vec2(0.0, 6.0)],
                color,
                egui::Stroke::NONE,
            ));
        }
        if self.focus.focused() == Some(r.id) {
            self.focused = Some(help.to_string());
        }
        if r.hovered() {
            self.hovered = Some(help.to_string());
        }
        if r.clicked() {
            return match r.interact_pointer_pos() {
                Some(at) if left.contains(at) => Step::Dec,
                Some(at) if right.contains(at) => Step::Inc,
                _ => Step::Cycle,
            };
        }
        if accepted {
            Step::Cycle
        } else if pad < 0 {
            Step::Dec
        } else if pad > 0 {
            Step::Inc
        } else {
            Step::None
        }
    }

    /// A row choosing `value` among `options`: whether it changed.
    fn choose<T: Copy + PartialEq>(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        value: &mut T,
        options: &[(T, &str)],
        help: &str,
    ) -> bool {
        let at = options.iter().position(|o| o.0 == *value);
        let n = options.len();
        let text = at.map_or("", |i| options[i].1);
        let can = at.map_or((false, false), |i| (i > 0, i + 1 < n));
        let to = match (self.row(ui, label, text, can, help), at) {
            (Step::Dec, Some(i)) if i > 0 => i - 1,
            (Step::Inc, Some(i)) if i + 1 < n => i + 1,
            (Step::Cycle, Some(i)) => (i + 1) % n,
            _ => return false,
        };
        *value = options[to].0;
        true
    }

    /// An on/off row.
    fn toggle(&mut self, ui: &mut egui::Ui, label: &str, value: &mut bool, help: &str) -> bool {
        self.choose(ui, label, value, &[(false, "Off"), (true, "On")], help)
    }

    /// A level in dB, a step at a time.
    fn db(&mut self, ui: &mut egui::Ui, label: &str, value: &mut f32, range: (f32, f32), help: &str) {
        let text = format!("{}{} dB", if *value > 0.0 { "+" } else { "" }, *value);
        let step = match self.row(ui, label, &text, (*value > range.0, *value < range.1), help) {
            Step::Dec => -1.0,
            Step::Inc | Step::Cycle => 1.0,
            Step::None => 0.0,
        };
        *value = (*value + step).clamp(range.0, range.1);
    }

    /// The help line to show.
    fn help(&self) -> String {
        let (first, second) =
            if self.nav_mode { (&self.focused, &self.hovered) } else { (&self.hovered, &self.focused) };
        first.as_ref().or(second.as_ref()).cloned().unwrap_or_default()
    }
}

/// The keyboard's navigation keys this frame as `Nav`s, taken out of egui's
/// input (so its own focus and the widgets don't see them again).
fn menu_keys(ctx: &egui::Context) -> Vec<Nav> {
    use egui::{Key, Modifiers};
    let map = [
        (Modifiers::SHIFT, Key::Tab, Nav::PrevGroup),
        (Modifiers::NONE, Key::ArrowLeft, Nav::Left),
        (Modifiers::NONE, Key::ArrowRight, Nav::Right),
        (Modifiers::NONE, Key::ArrowUp, Nav::Up),
        (Modifiers::NONE, Key::ArrowDown, Nav::Down),
        (Modifiers::NONE, Key::Enter, Nav::Accept),
        (Modifiers::NONE, Key::Space, Nav::Accept),
        (Modifiers::NONE, Key::Backspace, Nav::Back),
        (Modifiers::NONE, Key::Tab, Nav::NextGroup),
        (Modifiers::NONE, Key::E, Nav::NextGroup),
        (Modifiers::NONE, Key::Q, Nav::PrevGroup),
        (Modifiers::NONE, Key::PageDown, Nav::NextGroup),
        (Modifiers::NONE, Key::PageUp, Nav::PrevGroup),
    ];
    let mut out = Vec::new();
    ctx.input_mut(|i| {
        for (modifiers, key, nav) in map {
            while i.consume_key(modifiers, key) {
                out.push(nav);
            }
        }
    });
    out
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
    /// pad's focus and hints on, `keys` (the keyboard's navigation keys)
    /// the focus alone, the pointer or other keys both off.
    fn note_input(&mut self, ctx: &egui::Context, navs: &[Nav], keys: &[Nav]) {
        let other = ctx.input(|i| {
            i.pointer.delta() != egui::Vec2::ZERO
                || i.events.iter().any(|e| matches!(e, egui::Event::Key { .. } | egui::Event::PointerButton { .. }))
        });
        if !navs.is_empty() {
            (self.pad_used, self.keys_used) = (true, false);
        } else if !keys.is_empty() {
            (self.pad_used, self.keys_used) = (false, true);
        } else if other {
            (self.pad_used, self.keys_used) = (false, false);
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
            self.page = Page::Pause;
            self.pause_focus = Focus::default();
            self.slots_focus = Focus::default();
            self.settings_focus = Focus::default();
            self.controls_focus = Focus::default();
            return actions;
        }
        let keys = if self.capture.is_none() { menu_keys(ctx) } else { Vec::new() };
        self.note_input(ctx, navs, &keys);
        let all: Vec<Nav> = navs.iter().chain(&keys).copied().collect();
        let navs = &all[..];
        // Start resumes from any page.
        if self.capture.is_none() && view.playing.is_some() && navs.contains(&Nav::Start) {
            actions.push(Action::Resume);
            return actions;
        }
        if let (true, Some(b)) = (self.controls, bindings) {
            if self.controls_inner(ctx, b, view.playing.unwrap_or(""), navs) {
                actions.push(Action::Bindings);
            }
            return actions;
        }
        if self.controls {
            self.controls = false;
        }
        match self.page {
            Page::Pause => self.pause_page(ctx, view, navs, &mut actions),
            Page::SaveLoad => self.save_page(ctx, view, navs, &mut actions),
            Page::Settings => self.settings_page(ctx, settings, prefs, view, navs),
        }
        actions
    }

    /// The pause menu: a short list.
    fn pause_page(&mut self, ctx: &egui::Context, view: &View, navs: &[Nav], actions: &mut Vec<Action>) {
        let focus = &mut self.pause_focus;
        focus.show = self.pad_used || self.keys_used;
        for nav in focus.begin(navs) {
            if nav == Nav::Back && view.playing.is_some() {
                actions.push(Action::Resume);
            }
        }
        let mut go = None;
        let mut to_controls = false;
        egui::Window::new("reModel")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(340.0);
                ui.heading(view.playing.unwrap_or("No game"));
                ui.add_space(6.0);
                let w = ui.available_width();
                let mut big = |ui: &mut egui::Ui, text: &str| {
                    focus.button(ui, |ui| ui.add_sized([w, 34.0], egui::Button::new(text)))
                };
                if view.playing.is_some() {
                    if big(ui, "Resume") {
                        actions.push(Action::Resume);
                    }
                    if big(ui, "Save / Load") {
                        go = Some(Page::SaveLoad);
                    }
                }
                if big(ui, "Settings") {
                    go = Some(Page::Settings);
                }
                if view.playing.is_some() && big(ui, "Controls") {
                    to_controls = true;
                }
                if big(ui, "Back to games") {
                    actions.push(Action::Games);
                }
                if big(ui, "Quit") {
                    actions.push(Action::Quit);
                }
                if focus.show && self.pad_used {
                    ui.separator();
                    hints(ui, &[("D-pad", "choose"), ("A", "select"), ("B", "resume"), ("Start", "resume")]);
                }
            });
        self.pause_focus.end();
        if let Some(page) = go {
            self.page = page;
            self.slots_focus = Focus::default();
            self.settings_focus = Focus::default();
        }
        if to_controls {
            self.controls = true;
            self.controls_focus = Focus::default();
        }
    }

    /// The save slots, on a screen of their own.
    fn save_page(&mut self, ctx: &egui::Context, view: &View, navs: &[Nav], actions: &mut Vec<Action>) {
        let focus = &mut self.slots_focus;
        focus.show = self.pad_used || self.keys_used;
        let mut back = focus.begin(navs).contains(&Nav::Back);
        egui::Window::new("Save / Load")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(340.0);
                if ui.button("< Back").clicked() {
                    back = true;
                }
                ui.add_space(4.0);
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
                if self.pad_used {
                    ui.separator();
                    hints(ui, &[("D-pad", "choose"), ("A", "select"), ("B", "back"), ("Start", "resume")]);
                }
            });
        self.slots_focus.end();
        if back {
            self.page = Page::Pause;
            self.pause_focus = Focus::default();
        }
    }

    /// The settings: tabs of rows, the preset first on Picture.
    fn settings_page(
        &mut self,
        ctx: &egui::Context,
        settings: &mut GameSettings,
        mut prefs: Prefs,
        view: &View,
        navs: &[Nav],
    ) {
        let rest = self.settings_focus.begin(navs);
        let mut back = rest.contains(&Nav::Back);
        let tab_step = rest.iter().filter(|n| **n == Nav::NextGroup).count() as i32
            - rest.iter().filter(|n| **n == Nav::PrevGroup).count() as i32;
        if tab_step != 0 {
            self.tab = self.tab.step(tab_step);
            self.settings_focus = Focus::default();
        }
        let nav_mode = self.pad_used || self.keys_used;
        let pad_hints = self.pad_used;
        let mut tab = self.tab;
        let max_h = (ctx.content_rect().height() * 0.5).max(160.0);
        let focus = &mut self.settings_focus;
        focus.show = nav_mode;
        let mut rows = Rows { focus, nav_mode, focused: None, hovered: None };
        egui::Window::new("Settings")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_min_width(520.0);
                ui.horizontal(|ui| {
                    if ui.button("< Back").clicked() {
                        back = true;
                    }
                    ui.add_space(8.0);
                    for t in Tab::ALL {
                        if ui.selectable_label(tab == t, t.label()).clicked() {
                            tab = t;
                        }
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
                    ui.set_min_height(max_h.min(6.0 * 30.0));
                    match tab {
                        Tab::Display => display_rows(ui, &mut rows, settings, &mut prefs, view),
                        Tab::Picture => picture_rows(ui, &mut rows, settings, view),
                        Tab::Motion => motion_rows(ui, &mut rows, settings, view),
                        Tab::Sound => sound_rows(ui, &mut rows, settings),
                        Tab::More => more_rows(ui, &mut rows, settings, view),
                    }
                });
                ui.separator();
                rows.choose(
                    ui,
                    "Applies to",
                    &mut *prefs.for_all,
                    &[(false, "This game"), (true, "All games")],
                    "Where changes on these screens are kept: for this game only, or as the settings of every \
                     game that has none of its own.",
                );
                ui.separator();
                let help = rows.help();
                ui.allocate_ui_with_layout(
                    vec2(ui.available_width(), 54.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_height(54.0);
                        ui.add(egui::Label::new(egui::RichText::new(help).color(DIM)).wrap());
                    },
                );
                if pad_hints {
                    ui.separator();
                    hints(
                        ui,
                        &[
                            ("D-pad", "choose"),
                            ("Left Right", "change"),
                            ("LB RB", "tabs"),
                            ("B", "back"),
                            ("Start", "resume"),
                        ],
                    );
                }
            });
        self.settings_focus.end();
        if tab != self.tab {
            self.tab = tab;
            self.settings_focus = Focus::default();
        }
        // A picture preset whose rows no longer all match is Custom.
        *settings = settings.clone().settled(view.game);
        if back {
            self.page = Page::Pause;
            self.pause_focus = Focus::default();
        }
    }

    /// The Controls page: every input with its bindings, to remap (`navs`:
    /// the pad's; B goes back). Returns whether they changed (to save
    /// them).
    pub fn controls_page(&mut self, ctx: &egui::Context, b: &mut Bindings, title: &str, navs: &[Nav]) -> bool {
        self.note_input(ctx, navs, &[]);
        self.controls_inner(ctx, b, title, navs)
    }

    /// The Controls page, the input already noted.
    fn controls_inner(&mut self, ctx: &egui::Context, b: &mut Bindings, title: &str, navs: &[Nav]) -> bool {
        let mut changed = false;
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
        focus.show = self.pad_used || self.keys_used;
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
                    if focus.button(ui, |ui| ui.button("< Back")) {
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

fn display_rows(ui: &mut egui::Ui, rows: &mut Rows, s: &mut GameSettings, prefs: &mut Prefs, view: &View) {
    rows.choose(
        ui,
        "Aspect",
        &mut s.wide,
        &[(true, "Fit the window"), (false, "4:3")],
        "Fit the window draws the game wider than its 4:3 frame to fill the window. 4:3 keeps the original shape, \
         with the bezel beside it in a wider window.",
    );
    if s.wide {
        rows.choose(
            ui,
            "HUD position",
            &mut s.hud_edges,
            &[(true, "At the edges"), (false, "Centred")],
            "Where the score, speed and other displays go on a wide screen: out at the edges, or together in the \
             middle as in the original.",
        );
    } else if view.has_bezel {
        rows.toggle(ui, "Bezel", &mut s.bezel, "The cabinet's artwork beside a 4:3 picture in a wider window.");
    }
    rows.toggle(ui, "Fullscreen", &mut *prefs.fullscreen, "Fills the screen (F11 does the same, any time).");
}

fn picture_rows(ui: &mut egui::Ui, rows: &mut Rows, s: &mut GameSettings, view: &View) {
    let mut p = s.preset;
    let help = preset_help(p, view, s.geo_chip);
    if rows.choose(
        ui,
        "Preset",
        &mut p,
        &[
            (Preset::Original, "Original"),
            (Preset::Sharp, "Sharp"),
            (Preset::Enhanced, "Enhanced"),
            (Preset::Custom, "Custom"),
        ],
        &help,
    ) {
        s.apply_preset(p, view.game);
    }
    if s.preset != Preset::Custom {
        return;
    }
    // Temporal anti-aliasing replaces multisampling while it runs.
    let temporal = s.temporal_on() && view.temporal_supported;
    if !temporal {
        rows.choose(
            ui,
            "Anti-aliasing",
            &mut s.aa,
            &[(0, "Off"), (4, "4x"), (8, "8x")],
            "Smooths polygon edges by multisampling (8x where the GPU has it).",
        );
    }
    if !s.geo_chip && view.temporal_supported {
        rows.choose(
            ui,
            "Temporal anti-aliasing",
            &mut s.temporal,
            &TEMPORAL,
            "Smooth edges that stay smooth in motion, and replaces anti-aliasing. A percentage draws the 3D layer \
             smaller and upscales it: cheaper.",
        );
    }
    if temporal {
        rows.choose(
            ui,
            "See-through polygons",
            &mut s.stipple_keep,
            &[(false, "Even blend"), (true, "Checkerboard")],
            "How the games' checkerboard shadows and glass look under temporal anti-aliasing: an even 50% blend, \
             or the original pattern.",
        );
    }
    rows.toggle(
        ui,
        "Depth buffer",
        &mut s.depth,
        "Polygons that cross sort right. Off draws them in the game's order.",
    );
    let hint = shading_hint(s);
    rows.choose(ui, "Shading", &mut s.shading, &SHADING, hint);
    if s.shading != gpu::Shading::Flat {
        let game_crease = view.game.map_or(romset::Shading::DEFAULT.crease, |g| g.shading.crease);
        let mut list = vec![(gpu::CREASE_AUTO, format!("The game's ({game_crease}°)"))];
        list.extend(CREASES.iter().map(|c| (*c, format!("{c}°"))));
        if !list.iter().any(|o| o.0 == s.crease) {
            list.push((
                s.crease,
                if s.crease == 0 { "0° (nothing smoothed)".into() } else { format!("{}°", s.crease) },
            ));
        }
        let list: Vec<(u8, &str)> = list.iter().map(|(v, t)| (*v, t.as_str())).collect();
        rows.choose(
            ui,
            "Crease angle",
            &mut s.crease,
            &list,
            "Faces meeting at a sharper angle than this keep a hard edge: boxes and limbs stay faceted, bodies \
             smooth.",
        );
    }
    if s.light_on() {
        rows.toggle(
            ui,
            "Blend brightness levels",
            &mut s.shading_fine,
            "No banding on large smooth surfaces. Off keeps the game's 64 levels.",
        );
        rows.toggle(
            ui,
            "Brightness balance",
            &mut s.shading_balance,
            "Each smooth surface as bright as the game's. Off, bumpy surfaces come out brighter.",
        );
    }
    let pack_help = match (s.pack, view.pack) {
        (true, Some(text)) => text,
        _ => "Replacement textures from the game's texpack folder, if it has one.",
    };
    rows.toggle(ui, "Texture pack", &mut s.pack, pack_help);
    if s.pack && !s.geo_chip {
        let help = match (s.materials, view.materials) {
            (true, Some(text)) => text,
            _ => "The pack's normal maps: bumps and grooves lit by the game's own lights.",
        };
        rows.toggle(ui, "Material maps", &mut s.materials, help);
    }
    if s.materials_on() && !s.geo_chip {
        let custom = custom_relief(s.materials_relief);
        let mut list: Vec<(gpu::Relief, &str)> =
            RELIEF.iter().filter_map(|(text, v)| gpu::Relief::parse(v).map(|r| (r, *text))).collect();
        if let Some(text) = &custom {
            list.push((s.materials_relief, text.as_str()));
        }
        rows.choose(
            ui,
            "Material relief",
            &mut s.materials_relief,
            &list,
            "How strongly the bumps shade, 2 by default. Faithful keeps the game's own lighting sum: subtler.",
        );
    }
}

fn motion_rows(ui: &mut egui::Ui, rows: &mut Rows, s: &mut GameSettings, view: &View) {
    rows.toggle(
        ui,
        "Exact arcade speed",
        &mut s.exact,
        "The arcade board runs at 57.5 frames a second. Off shows one game frame per screen refresh: about 4% \
         fast, but smooth.",
    );
    if !s.geo_chip {
        rows.toggle(
            ui,
            "Smooth motion",
            &mut s.smooth,
            "Draws in-between frames on screens faster than 60 Hz, a frame later.",
        );
    }
    let max = run_ahead_max(view.game);
    if max > 0 || s.run_ahead > 0 {
        const LABELS: [&str; 4] = ["Off", "1 frame", "2 frames", "3 frames"];
        let mut list: Vec<(u32, &str)> = (0..=max).map(|n| (n, LABELS[n as usize])).collect();
        if s.run_ahead > max {
            list.push((s.run_ahead, LABELS[s.run_ahead as usize]));
        }
        let help = run_ahead_help(view.game, view.run_ahead_dropped);
        rows.choose(ui, "Run-ahead", &mut s.run_ahead, &list, &help);
    }
}

fn sound_rows(ui: &mut egui::Ui, rows: &mut Rows, s: &mut GameSettings) {
    rows.db(ui, "Effects", &mut s.sfx_db, (-20.0, 20.0), "The level of the sound board: effects and announcer, in dB.");
    rows.db(ui, "Music", &mut s.music_db, (-30.0, 10.0), "The level of the music board, in dB.");
}

fn more_rows(ui: &mut egui::Ui, rows: &mut Rows, s: &mut GameSettings, view: &View) {
    rows.choose(
        ui,
        "Geometrizer",
        &mut s.geo_chip,
        &[(false, "Native"), (true, "The game's chip")],
        "Takes effect the next time a game starts. Native is faster and needed for smooth motion, temporal \
         anti-aliasing and smooth lighting; the chip runs the game's own program.",
    );
    if s.pack {
        let mut list: Vec<(model2::enhance::TexFilter, &str)> = crate::settings::tex_filters(view.max_anisotropy)
            .iter()
            .filter_map(|(label, v)| model2::enhance::TexFilter::parse(v).map(|f| (f, *label)))
            .collect();
        if !list.iter().any(|o| o.0 == s.tex_filter) {
            list.push((s.tex_filter, "Not on this GPU"));
        }
        rows.choose(
            ui,
            "Texture filtering",
            &mut s.tex_filter,
            &list,
            "Of the texture pack's images: mipmaps stop the shimmer at a distance, anisotropic filtering adds \
             sharpness.",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relief_off_the_list_is_a_custom_choice() {
        assert_eq!(custom_relief(gpu::Relief::default()), None);
        assert_eq!(custom_relief(gpu::Relief::Faithful), None);
        let r = gpu::Relief::parse("1.5").unwrap();
        assert_eq!(custom_relief(r).as_deref(), Some("custom (1.5)"));
    }

    /// The menu over a game, headless, a frame at a time.
    struct Rig {
        ctx: egui::Context,
        menu: Menu,
        s: GameSettings,
        b: Bindings,
        game: &'static romset::Game,
        actions: Vec<Action>,
        fullscreen: bool,
        for_all: bool,
        dropped: bool,
        size: egui::Vec2,
    }

    impl Rig {
        fn new(id: &str) -> Rig {
            let game = romset::games::playable(id).unwrap();
            let mut rig = Rig {
                ctx: egui::Context::default(),
                menu: Menu { open: true, ..Default::default() },
                s: GameSettings::default(),
                b: Bindings::defaults(&game.controls),
                game,
                actions: Vec::new(),
                fullscreen: false,
                for_all: false,
                dropped: false,
                size: egui::vec2(1920.0, 1080.0),
            };
            // One frame to lay it out.
            rig.frame(&[], Vec::new());
            rig
        }

        fn frame(&mut self, navs: &[Nav], events: Vec<egui::Event>) {
            let slots = [Some(SystemTime::now()), None, None, None, None];
            let view = View {
                playing: Some(self.game.title),
                game: Some(self.game),
                slots: &slots,
                max_anisotropy: 16,
                temporal_supported: true,
                has_bezel: true,
                run_ahead_dropped: self.dropped,
                pack: None,
                materials: None,
            };
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
                events,
                ..Default::default()
            };
            let Rig { menu, s, b, actions, fullscreen, for_all, .. } = self;
            let mut out = self.ctx.run_ui(input, |ui| {
                let prefs = Prefs { fullscreen, for_all };
                actions.extend(menu.ui(ui.ctx(), s, prefs, &view, Some(&mut *b), navs));
            });
            out.textures_delta.clear();
        }

        /// A frame for each entry, the pad's `Nav`s that frame.
        fn run(&mut self, frames: &[&[Nav]]) -> &mut Rig {
            for navs in frames {
                self.frame(navs, Vec::new());
            }
            self
        }

        /// `n` presses of `nav`, a frame each.
        fn press(&mut self, n: usize, nav: Nav) -> &mut Rig {
            for _ in 0..n {
                self.frame(&[nav], Vec::new());
            }
            self
        }

        /// Into the Settings screen, onto its first row.
        fn open_settings(&mut self) -> &mut Rig {
            self.run(&[&[Nav::Down], &[Nav::Down], &[Nav::Accept], &[]])
        }

        /// Into the settings tab `t`.
        fn open_tab(&mut self, t: Tab) -> &mut Rig {
            self.open_settings();
            let n = Tab::ALL.iter().position(|x| *x == t).unwrap();
            self.press(n, Nav::NextGroup);
            self.frame(&[], Vec::new());
            assert_eq!(self.menu.tab, t);
            self
        }

        fn names(&self) -> Vec<&'static str> {
            names(&self.actions)
        }
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
        for nav in [Nav::Accept, Nav::Start, Nav::Back] {
            let mut r = Rig::new("topskatr");
            r.run(&[&[nav]]);
            assert_eq!(r.names(), ["resume"], "{nav:?}");
        }
    }

    #[test]
    fn the_pause_menu_is_a_short_list() {
        // Resume, Save / Load, Settings, Controls, Back to games, Quit, in
        // that order: down to the foot is Quit, one up Back to games.
        let mut r = Rig::new("topskatr");
        r.press(30, Nav::Down).run(&[&[Nav::Accept]]);
        assert_eq!(r.names(), ["quit"]);
        let mut r = Rig::new("topskatr");
        r.press(30, Nav::Down).press(1, Nav::Up).run(&[&[Nav::Accept]]);
        assert_eq!(r.names(), ["games"]);
        // Nothing else on it: the controls of the settings aren't here.
        let mut r = Rig::new("topskatr");
        r.press(5, Nav::Down).run(&[&[Nav::Accept]]);
        assert_eq!(r.names(), ["quit"], "five steps from Resume is the last");
    }

    #[test]
    fn save_and_load_slots() {
        let mut r = Rig::new("topskatr");
        // Down to Save / Load, in; the quick slot's Save, right to its Load
        // (it's saved).
        r.run(&[&[Nav::Down], &[Nav::Accept], &[]]);
        assert_eq!(r.menu.page, Page::SaveLoad);
        r.run(&[&[Nav::Accept], &[Nav::Right], &[Nav::Accept]]);
        assert!(matches!(&r.actions[..], [Action::Save(0), Action::Load(0)]));
        // Slot 1 is empty: its Load can't be focused.
        let mut r = Rig::new("topskatr");
        r.run(&[&[Nav::Down], &[Nav::Accept], &[], &[Nav::Down], &[Nav::Right], &[Nav::Accept]]);
        assert!(matches!(&r.actions[..], [Action::Save(1)]));
        // B goes back to the pause menu, which B then closes.
        r.run(&[&[Nav::Back], &[]]);
        assert_eq!(r.menu.page, Page::Pause);
        r.run(&[&[Nav::Back]]);
        assert_eq!(r.names(), ["save", "resume"]);
    }

    #[test]
    fn rows_change_with_left_and_right() {
        let mut r = Rig::new("topskatr");
        let (wide, bezel) = (r.s.wide, r.s.bezel);
        assert!(wide);
        // Aspect: right to 4:3 (left again at the end of the choices does
        // nothing); then the bezel shows in place of the HUD row: A turns it.
        r.open_settings().run(&[&[Nav::Right], &[Nav::Right]]);
        assert!(!r.s.wide);
        r.run(&[&[Nav::Down], &[Nav::Accept]]);
        assert_eq!(r.s.bezel, !bezel);
        // Left back to Fit the window: the HUD row replaces the bezel's.
        r.run(&[&[Nav::Up], &[Nav::Left], &[Nav::Left]]);
        assert!(r.s.wide);
        r.run(&[&[Nav::Down], &[Nav::Right]]);
        assert!(!r.s.hud_edges, "At the edges is the first choice: right goes on to Centred");
    }

    #[test]
    fn hud_and_bezel_rows_follow_the_aspect() {
        let mut r = Rig::new("topskatr");
        // Fit the window: HUD position is the second row (centred with
        // right); the third is Fullscreen.
        r.open_settings().run(&[&[Nav::Down], &[Nav::Right], &[Nav::Down], &[Nav::Accept]]);
        assert!(!r.s.hud_edges);
        assert!(r.fullscreen);
        // 4:3 with no bezel art: neither row; Fullscreen is the second.
        let mut r = Rig::new("topskatr");
        r.open_settings().run(&[&[Nav::Right], &[Nav::Down], &[Nav::Accept]]);
        assert!(!r.s.wide && r.s.hud_edges);
    }

    #[test]
    fn tabs_switch_with_the_shoulder_buttons_and_keys() {
        let mut r = Rig::new("topskatr");
        r.open_settings();
        assert_eq!(r.menu.tab, Tab::Display);
        r.run(&[&[Nav::NextGroup], &[Nav::NextGroup]]);
        assert_eq!(r.menu.tab, Tab::Motion);
        r.run(&[&[Nav::PrevGroup]]);
        assert_eq!(r.menu.tab, Tab::Picture);
        // Round the ends.
        r.run(&[&[Nav::PrevGroup], &[Nav::PrevGroup]]);
        assert_eq!(r.menu.tab, Tab::More);
        r.run(&[&[Nav::NextGroup]]);
        assert_eq!(r.menu.tab, Tab::Display);
        // The keyboard: E and Tab forward, Q and Shift+Tab back.
        let key = |k: egui::Key, shift: bool| egui::Event::Key {
            key: k,
            physical_key: Some(k),
            pressed: true,
            repeat: false,
            modifiers: if shift { egui::Modifiers::SHIFT } else { egui::Modifiers::NONE },
        };
        r.frame(&[], vec![key(egui::Key::E, false)]);
        assert_eq!(r.menu.tab, Tab::Picture);
        r.frame(&[], vec![key(egui::Key::Tab, false)]);
        assert_eq!(r.menu.tab, Tab::Motion);
        r.frame(&[], vec![key(egui::Key::Q, false)]);
        r.frame(&[], vec![key(egui::Key::Tab, true)]);
        assert_eq!(r.menu.tab, Tab::Display);
        // The arrow keys move and change a row like the pad's d-pad.
        r.frame(&[], vec![key(egui::Key::ArrowRight, false)]);
        assert!(!r.s.wide);
        assert!(!r.menu.pad_used, "the keyboard isn't the pad");
        assert!(r.menu.keys_used);
    }

    #[test]
    fn sound_rows_are_in_decibels() {
        let mut r = Rig::new("topskatr");
        let (sfx, music) = (r.s.sfx_db, r.s.music_db);
        // Effects: right three, left one; music: left one.
        r.open_tab(Tab::Sound).press(3, Nav::Right).press(1, Nav::Left);
        r.run(&[&[Nav::Down], &[Nav::Left]]);
        assert_eq!((r.s.sfx_db, r.s.music_db), (sfx + 2.0, music - 1.0));
    }

    #[test]
    fn applies_to_is_at_the_foot_of_each_tab() {
        for t in Tab::ALL {
            let mut r = Rig::new("topskatr");
            r.open_tab(t);
            assert!(!r.for_all);
            // However many rows the tab has, the last is "Applies to".
            r.press(40, Nav::Down).run(&[&[Nav::Right]]);
            assert!(r.for_all, "{t:?}");
        }
    }

    #[test]
    fn picture_presets_set_the_rows() {
        let mut r = Rig::new("topskatr");
        assert_eq!(r.s.preset, Preset::Original);
        // Picture's first row is the preset; right to Sharp.
        r.open_tab(Tab::Picture).press(1, Nav::Right);
        assert_eq!((r.s.preset, r.s.aa, r.s.tex_filter), (Preset::Sharp, 4, model2::enhance::TexFilter::Mip));
        assert_eq!((r.s.temporal, r.s.depth, r.s.shading, r.s.smooth), (0, false, gpu::Shading::Flat, false));
        // Enhanced: for topskatr temporal at full size, the depth buffer,
        // smooth shading, smooth motion; its input lag is 0.
        r.press(1, Nav::Right);
        assert_eq!(r.s.preset, Preset::Enhanced);
        assert_eq!((r.s.temporal, r.s.depth, r.s.shading), (100, true, gpu::Shading::Vertex));
        assert_eq!((r.s.smooth, r.s.run_ahead), (true, 0));
        // Back to Original: everything off again, smooth motion too.
        r.press(2, Nav::Left);
        assert_eq!(r.s.preset, Preset::Original);
        assert_eq!(GameSettings { sfx_db: r.s.sfx_db, music_db: r.s.music_db, ..r.s.clone() }, GameSettings::default());
        // Custom keeps the rows as they are.
        r.press(2, Nav::Right).press(2, Nav::Right);
        assert_eq!((r.s.preset, r.s.aa, r.s.temporal, r.s.depth), (Preset::Custom, 4, 100, true));
    }

    #[test]
    fn enhanced_is_per_game() {
        let preset = |id: &str| {
            let mut s = GameSettings::default();
            s.apply_preset(Preset::Enhanced, romset::games::playable(id));
            s
        };
        // Temporal everywhere but vstriker, which gets 4x multisampling.
        let s = preset("vstriker");
        assert_eq!((s.aa, s.temporal, s.depth, s.shading), (4, 0, false, gpu::Shading::Flat));
        let s = preset("stcc");
        assert_eq!((s.temporal, s.depth, s.shading, s.run_ahead), (100, false, gpu::Shading::Vertex, 1));
        // Run-ahead at the useful maximum: sgt24h measured 4, offered 3; the
        // fighters whose stick has no lag get none.
        assert_eq!(preset("sgt24h").run_ahead, 3);
        assert_eq!(preset("schamp").run_ahead, 0);
        assert_eq!(preset("lastbrnx").run_ahead, 0);
        assert_eq!(preset("hotd").run_ahead, 0);
        assert!(preset("bel").depth && !preset("dynabb97").depth && !preset("stcc").depth);
    }

    #[test]
    fn changing_a_row_makes_the_preset_custom() {
        let mut s = GameSettings::default();
        let topskatr = romset::games::playable("topskatr");
        s.apply_preset(Preset::Enhanced, topskatr);
        assert_eq!(s.clone().settled(topskatr).preset, Preset::Enhanced);
        // Smooth motion and run-ahead don't count against a preset...
        s.smooth = false;
        s.run_ahead = 2;
        assert_eq!(s.clone().settled(topskatr).preset, Preset::Enhanced);
        // ... the picture rows do, and keep their values.
        s.depth = false;
        let t = s.clone().settled(topskatr);
        assert_eq!((t.preset, t.temporal, t.shading), (Preset::Custom, 100, gpu::Shading::Vertex));
        // Another game's Enhanced isn't this one's.
        let mut s = GameSettings::default();
        s.apply_preset(Preset::Enhanced, topskatr);
        assert_eq!(s.settled(romset::games::playable("stcc")).preset, Preset::Custom);
    }

    #[test]
    fn only_custom_lists_the_picture_rows() {
        let mut r = Rig::new("topskatr");
        // Original: the preset row, then "Applies to": a step down from the
        // first is the last, and right on it sets that.
        r.open_tab(Tab::Picture).press(1, Nav::Down).press(1, Nav::Right);
        assert!(r.for_all);
        // Custom (rows as Original): preset, anti-aliasing, temporal, depth
        // buffer, shading, texture pack. Four down is shading; right: vertex.
        let mut r = Rig::new("topskatr");
        r.s.preset = Preset::Custom;
        r.open_tab(Tab::Picture).press(4, Nav::Down).press(1, Nav::Right);
        assert_eq!(r.s.shading, gpu::Shading::Vertex);
        assert_eq!(r.s.preset, Preset::Custom);
    }

    #[test]
    fn rows_that_do_nothing_are_left_out() {
        // Temporal on: no anti-aliasing row (the first below the preset is
        // temporal's, and its checkerboard row follows).
        let mut r = Rig::new("topskatr");
        (r.s.preset, r.s.temporal) = (Preset::Custom, 67);
        r.open_tab(Tab::Picture).press(1, Nav::Down).press(1, Nav::Right);
        assert_eq!((r.s.aa, r.s.temporal), (0, 50));
        r.press(1, Nav::Down).press(1, Nav::Accept);
        assert!(r.s.stipple_keep);
        // With the chip temporal can't run: no temporal row, no checkerboard
        // row; anti-aliasing is the first and the depth buffer the second.
        let mut r = Rig::new("topskatr");
        (r.s.preset, r.s.temporal, r.s.geo_chip) = (Preset::Custom, 67, true);
        r.open_tab(Tab::Picture).press(1, Nav::Down).press(1, Nav::Right);
        assert_eq!((r.s.aa, r.s.temporal), (4, 67));
        r.press(1, Nav::Down).press(1, Nav::Accept);
        assert!(r.s.depth && !r.s.stipple_keep);
        // Flat shading: no crease, brightness levels or balance rows; the
        // texture pack follows shading, and its maps appear once it's on.
        let mut r = Rig::new("topskatr");
        r.s.preset = Preset::Custom;
        r.open_tab(Tab::Picture).press(5, Nav::Down).press(1, Nav::Accept);
        assert!(r.s.pack && !r.s.materials);
        r.press(1, Nav::Down).press(1, Nav::Accept);
        assert!(r.s.materials);
        // ... and relief once the maps are on.
        r.press(1, Nav::Down).press(1, Nav::Right);
        assert_ne!(r.s.materials_relief, gpu::Relief::default());
        // Smooth shading brings its three: crease, levels, balance.
        let mut r = Rig::new("topskatr");
        (r.s.preset, r.s.shading) = (Preset::Custom, gpu::Shading::Vertex);
        r.open_tab(Tab::Picture).press(8, Nav::Down).press(1, Nav::Accept);
        assert!(r.s.pack, "preset, aa, temporal, depth, shading, crease, levels, balance, pack");
        // Crease 0 smooths nothing: no levels or balance rows.
        let mut r = Rig::new("topskatr");
        (r.s.preset, r.s.shading, r.s.crease) = (Preset::Custom, gpu::Shading::Vertex, 0);
        r.open_tab(Tab::Picture).press(6, Nav::Down).press(1, Nav::Accept);
        assert!(r.s.pack);
    }

    #[test]
    fn motion_rows_and_the_run_ahead_cap() {
        // topskatr has no lag to hide: exact speed, smooth motion, nothing
        // else; the last row is "Applies to".
        let mut r = Rig::new("topskatr");
        r.open_tab(Tab::Motion).press(1, Nav::Down).press(1, Nav::Down).press(1, Nav::Right);
        assert!(r.for_all && r.s.run_ahead == 0);
        // stcc lags a frame: run-ahead is the third row, off or 1.
        let mut r = Rig::new("stcc");
        r.open_tab(Tab::Motion).press(2, Nav::Down).press(5, Nav::Right);
        assert_eq!(r.s.run_ahead, 1);
        // sgt24h measures 4: offered up to 3.
        let mut r = Rig::new("sgt24h");
        r.open_tab(Tab::Motion).press(2, Nav::Down).press(5, Nav::Right);
        assert_eq!(r.s.run_ahead, 3);
        // With the chip there are no in-betweens: smooth motion's row goes,
        // run-ahead is the second.
        let mut r = Rig::new("stcc");
        r.s.geo_chip = true;
        r.open_tab(Tab::Motion).press(1, Nav::Down).press(1, Nav::Right);
        assert_eq!(r.s.run_ahead, 1);
        // A value above the cap (the command line's) keeps its row to turn
        // it off.
        let mut r = Rig::new("topskatr");
        r.s.run_ahead = 2;
        r.open_tab(Tab::Motion).press(2, Nav::Down).press(5, Nav::Left);
        assert_eq!(r.s.run_ahead, 0);
    }

    #[test]
    fn run_ahead_help_says_what_it_does_for_the_game() {
        let help = |id: &str, dropped| run_ahead_help(romset::games::playable(id), dropped);
        let h = help("sgt24h", false);
        assert!(h.contains("3 frames") && h.contains("4 frames"), "{h}");
        let h = help("stcc", false);
        assert!(h.contains("1 frame") && !h.contains("1 frames"), "{h}");
        for id in ["schamp", "lastbrnx"] {
            let h = help(id, false);
            assert!(h.contains("punches") && h.contains("judder"), "{id}: {h}");
        }
        assert!(!help("stcc", false).contains("too slow") && !help("stcc", false).contains("could not keep up"));
        assert!(help("stcc", true).contains("could not keep up"));
        assert!(run_ahead_help(None, false).contains("input lag"));
        assert_eq!(run_ahead_max(romset::games::playable("sgt24h")), 3);
        assert_eq!(run_ahead_max(romset::games::playable("hotd")), 0);
    }

    #[test]
    fn the_help_line_says_what_the_preset_does() {
        // The text the preset row's help line shows, per game.
        let view = View {
            playing: Some("A game"),
            game: romset::games::playable("stcc"),
            slots: &[],
            max_anisotropy: 16,
            temporal_supported: true,
            has_bezel: true,
            run_ahead_dropped: false,
            pack: None,
            materials: None,
        };
        assert!(preset_help(Preset::Enhanced, &view, false).contains("smooth shading"));
        assert!(!preset_help(Preset::Enhanced, &view, false).contains("depth buffer"));
        assert!(preset_help(Preset::Enhanced, &view, false).contains("run-ahead 1"));
        assert!(preset_help(Preset::Enhanced, &view, true).contains("native geometrizer"));
        let bel = View { game: romset::games::playable("bel"), ..view };
        assert!(preset_help(Preset::Enhanced, &bel, false).contains("depth buffer"));
    }

    #[test]
    fn shading_row() {
        let mut s = GameSettings { shading: gpu::Shading::Pixel, ..Default::default() };
        // Pixel is per pixel with the native geometrizer; the chip doesn't
        // turn it off, but draws it as vertex and says so.
        assert!(!shading_hint(&s).contains("vertex"));
        s.geo_chip = true;
        assert!(shading_hint(&s).contains("as vertex"));
        s.crease = 0;
        assert!(shading_hint(&s).contains("nothing"));
    }

    #[test]
    fn focus_glows_for_the_pad_and_keys_only() {
        let mut r = Rig::new("topskatr");
        let glow = |r: &mut Rig, navs: &[Nav], events: Vec<egui::Event>| {
            let before = r.ctx.cumulative_frame_nr();
            let slots = [None; 5];
            let view = View {
                playing: Some("A game"),
                game: None,
                slots: &slots,
                max_anisotropy: 16,
                temporal_supported: true,
                has_bezel: false,
                run_ahead_dropped: false,
                pack: None,
                materials: None,
            };
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0))),
                events,
                ..Default::default()
            };
            let Rig { menu, s, fullscreen, for_all, .. } = r;
            let mut out = r.ctx.run_ui(input, |ui| {
                menu.ui(ui.ctx(), s, Prefs { fullscreen, for_all }, &view, None, navs);
            });
            out.textures_delta.clear();
            assert!(r.ctx.cumulative_frame_nr() > before);
            out.shapes
                .iter()
                // Cyan (fading in with the window).
                .filter(|c| {
                    matches!(&c.shape, egui::Shape::Rect(r)
                        if r.stroke.width == 1.5 && r.stroke.color.to_opaque().b() > 150 && r.stroke.color.r() < 100)
                })
                .count()
        };
        glow(&mut r, &[], Vec::new());
        assert_eq!(glow(&mut r, &[], Vec::new()), 0, "keyboard and mouse: no glow");
        assert_eq!(glow(&mut r, &[Nav::Down], Vec::new()), 1, "the pad: one");
        let key = egui::Event::Key {
            key: egui::Key::ArrowDown,
            physical_key: Some(egui::Key::ArrowDown),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        r.menu.pad_used = false;
        r.menu.keys_used = false;
        assert_eq!(glow(&mut r, &[], vec![key]), 1, "the navigation keys: one");
        let click = egui::Event::PointerButton {
            pos: egui::pos2(5.0, 5.0),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        };
        assert_eq!(glow(&mut r, &[], vec![click]), 0, "the mouse turns it off");
    }

    #[test]
    fn controls_page_from_the_pad() {
        let mut r = Rig::new("topskatr");
        // Down to Controls (the fourth), into it; down to the first
        // binding, A captures.
        r.press(3, Nav::Down).run(&[&[Nav::Accept], &[], &[Nav::Down], &[Nav::Accept]]);
        assert!(r.actions.is_empty());
        assert!(r.menu.controls);
        let first = r.b.slots()[0].0;
        assert_eq!(r.menu.capture.map(|c| c.0), Some(first), "waiting for a binding");
        // While it waits, the pad's input is the binding's, not the page's.
        r.run(&[&[Nav::Back]]);
        assert!(r.menu.controls && r.menu.capture.is_some());
        // Waiting too long gives up.
        r.menu.capture_since = r.menu.capture_since.map(|(c, at)| (c, at - CAPTURE_WAIT));
        r.run(&[&[]]);
        assert!(r.menu.capture.is_none());
        // B goes back to the pause menu, on Controls; B there resumes.
        r.run(&[&[Nav::Back]]);
        assert!(!r.menu.controls);
        r.run(&[&[Nav::Back]]);
        assert_eq!(r.names(), ["resume"]);
    }
}
