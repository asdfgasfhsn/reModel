//! The in-game menu (Esc) and short notices, drawn with egui over the game.
//!
//! The pause menu is a short list: Resume, Save / Load, Picture, Controls,
//! Sound, Speed and latency, Photo mode, Back to games, Quit. Save / Load is
//! a small screen of its own (the slots). The picture has two pages of their
//! own (`picture`): a list of styles (Arcade original, Sharp, Smooth modern,
//! CRT, Custom) applied live to the paused game, with a before/after
//! Compare, and an Advanced page with a row for each picture setting. The
//! other settings come in tabs (Display, Motion, Sound, More), each row
//! "label ◂ value ▸": left and right change it, and one help line at the
//! bottom says what the focused (or pointed-at) row does. Options that do
//! nothing as things are set are left out, not greyed. Every settings tab
//! ends with where the changes are kept (this game or all games).
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
use crate::hints::{run_ahead_help, run_ahead_max, run_ahead_shown, RUN_AHEAD};
use crate::launcher::{Nav, DIM};
use crate::saves::{self, SlotInfo};
use crate::settings::{GameSettings, Preset};

mod controls;
pub use controls::conflict_text;
mod figure;
mod picture;
use crate::theme::{self, Chip, Level, Toasts};
use egui::{pos2, vec2, Align2, Color32, FontFamily, FontId, Rect, Sense, Shape, Stroke};
use romset::bindings::{Bindings, Slot};
use std::collections::HashMap;
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
    /// From the launcher: drop the selected game's own settings, so it
    /// takes the ones for all games.
    ResetOwn,
    /// Start the game again from power-on.
    Reset,
    /// Open photo mode.
    Photo,
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
    /// The picture styles, and Compare.
    Picture,
    /// Each picture setting.
    Advanced,
    /// The game's controls: bindings, the device's picture, the tuning.
    Controls,
}

/// The settings tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Display,
    Motion,
    Sound,
    More,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Display, Tab::Motion, Tab::Sound, Tab::More];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Display => "Display",
            Tab::Motion => "Motion",
            Tab::Sound => "Sound",
            Tab::More => "More",
        }
    }

    /// The page's heading: the Motion tab is "Speed and latency" there.
    pub fn heading(self) -> &'static str {
        match self {
            Tab::Motion => "Speed and latency",
            other => other.label(),
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
    /// The corner notices.
    toasts: Toasts,
    /// The menu was open last frame (opening it dismisses error toasts).
    was_open: bool,
    /// Which screen the menu is on (besides Controls) and which settings tab.
    pub page: Page,
    pub tab: Tab,
    /// Waiting for a key, button or stick to bind into a slot: in place of
    /// its nth binding, or added (None).
    pub capture: Option<(Slot, Option<usize>)>,
    /// The Controls page lists inputs the manifest leaves unbound too.
    show_unbound: bool,
    /// Since when the Controls page has waited for `capture`.
    capture_since: Option<((Slot, Option<usize>), Instant)>,
    /// The last input came from a gamepad: its focus ring shows, hints show.
    pub pad_used: bool,
    /// The last input came from the keyboard's navigation keys: the focus
    /// ring shows.
    keys_used: bool,
    /// The focus on each screen.
    pause_focus: Focus,
    slots_focus: Focus,
    settings_focus: Focus,
    /// The Picture and Advanced pages' state.
    picture: picture::State,
    /// The Controls page's state.
    ctl: controls::State,
    /// The slot the slots page has selected.
    slot_sel: u8,
    /// The slots' thumbnails as textures, loaded as the slots page opens,
    /// and pictures just taken, waiting to become textures.
    thumbs: HashMap<u8, Thumb>,
    fresh: HashMap<u8, egui::ColorImage>,
    /// Reset game was selected once, and wants a second.
    reset_armed: Option<Instant>,
}

/// A slot's thumbnail on the GPU: of the save made at `saved`, read from
/// the PNG written at `stamp` (None: taken just now, the PNG not yet seen).
struct Thumb {
    saved: SystemTime,
    stamp: Option<SystemTime>,
    tex: egui::TextureHandle,
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
    /// Each slot, if it has been saved: when, by what, its thumbnail.
    pub saves: &'a [Option<SlotInfo>],
    /// Anisotropic filtering the GPU has (`gpu::max_anisotropy`): the
    /// texture filters offered.
    pub max_anisotropy: u8,
    /// The GPU can do temporal anti-aliasing (`gpu::temporal_supported`).
    pub temporal_supported: bool,
    /// The game has bezel art.
    pub has_bezel: bool,
    /// The game has settings of its own (the launcher offers to drop them).
    pub has_own: bool,
    /// Run-ahead was switched off because the machine could not keep up.
    pub run_ahead_dropped: bool,
    /// The frames of run-ahead this computer has time for, once measured
    /// (`hints::FrameCost::run_ahead_room`).
    pub run_ahead_room: Option<u32>,
    /// The game's picture in the window, in pixels (x, y, width, height):
    /// where Compare's divider goes. None: no game picture to compare.
    pub picture: Option<[f32; 4]>,
    /// What the picture pages say about where changes are kept.
    pub keeps: Keeps,
    /// How the texture pack is doing (where it is, what it replaced), or
    /// why there is none, while the setting is on.
    pub pack: Option<&'a str>,
    /// What material maps did in the last frame, or why they have no
    /// effect, while the setting is on.
    pub materials: Option<&'a str>,
    /// The look drawn now (its file with the player's tuning), for the
    /// Advanced page's rows; None with the look off.
    pub look: Option<&'a model2::look::Look>,
    /// What the pad reads now (the Controls page's picture and meters).
    pub pad: crate::controls::PadLive,
}

/// What the picture pages tell a player about where a change is kept
/// (`Settings::all_games_preset`, `Settings::has_own_picture`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Keeps {
    /// The style games with no settings of their own play with.
    pub all_games: Preset,
    /// The game has picture settings of its own.
    pub own_picture: bool,
}

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
    /// One "label ◂ value ▸" row, the label over the value; `can`: whether
    /// there is a choice before and after.
    fn row(&mut self, ui: &mut egui::Ui, label: &str, value: &str, can: (bool, bool), help: &str) -> Step {
        let s = theme::scale(ui.ctx());
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0 * s), Sense::hover());
        let r = ui.interact(rect, ui.make_persistent_id(("row", label)), Sense::CLICK);
        let (pad, accepted) = self.focus.stepper(ui, &r);
        let arrow = 28.0 * s;
        let right = Rect::from_min_max(
            pos2(rect.right() - arrow - 8.0 * s, rect.top()),
            pos2(rect.right() - 8.0 * s, rect.bottom()),
        );
        let left = Rect::from_min_max(pos2(right.left() - arrow, rect.top()), pos2(right.left(), rect.bottom()));
        let p = ui.painter();
        if r.hovered() && !self.nav_mode {
            p.rect_filled(rect, theme::ROW_RADIUS * s, Color32::from_white_alpha(14));
        }
        let x = rect.left() + 14.0 * s;
        p.text(
            pos2(x, rect.top() + 6.0 * s),
            Align2::LEFT_TOP,
            label,
            FontId::new(14.0 * s, FontFamily::Proportional),
            theme::TEXT_DIM,
        );
        p.text(
            pos2(x, rect.top() + 24.0 * s),
            Align2::LEFT_TOP,
            value,
            FontId::new(17.0 * s, theme::semibold()),
            theme::TEXT,
        );
        for (rect, dir, on) in [(left, -1.0, can.0), (right, 1.0, can.1)] {
            let c = rect.center() + vec2(0.0, 4.0 * s);
            let color = if on { theme::CYAN } else { DIM.gamma_multiply(0.35) };
            let tip = c + vec2(6.0 * dir, 0.0) * s;
            let back = c - vec2(4.0 * dir, 0.0) * s;
            p.add(Shape::convex_polygon(
                vec![tip, back + vec2(0.0, -7.0 * s), back + vec2(0.0, 7.0 * s)],
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

/// "Paused", a pill at the game's top left while the menu is up.
fn paused_pill(ctx: &egui::Context) {
    let s = theme::scale(ctx);
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::unique("remodel-paused-pill")));
    let galley = p.layout_no_wrap("Paused".into(), FontId::new(17.0 * s, theme::semibold()), theme::TEXT);
    let size = vec2(galley.size().x + 52.0 * s, galley.size().y + 16.0 * s);
    let rect = Rect::from_min_size(pos2(24.0 * s, 24.0 * s), size);
    p.rect_filled(rect, 999.0, Color32::from_rgba_unmultiplied(4, 6, 11, 204));
    let dot = Rect::from_center_size(pos2(rect.left() + 21.0 * s, rect.center().y), vec2(10.0 * s, 10.0 * s));
    p.rect_filled(dot, 2.0 * s, theme::YELLOW);
    p.galley(pos2(rect.left() + 36.0 * s, rect.center().y - galley.size().y / 2.0), galley, theme::TEXT);
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
    /// Show `text` briefly in a corner of the screen.
    pub fn notify(&mut self, text: impl Into<String>) {
        self.toast(Level::Ok, text);
    }

    /// Like `notify`, as a warning.
    pub fn warn(&mut self, text: impl Into<String>) {
        self.toast(Level::Warn, text);
    }

    /// Like `notify`, as an error: it stays until dismissed (a click, or
    /// opening the menu).
    pub fn error(&mut self, text: impl Into<String>) {
        self.toast(Level::Error, text);
    }

    fn toast(&mut self, level: Level, text: impl Into<String>) {
        let text = text.into();
        // The log has every notice, as the screen shows it only briefly.
        println!("notice        {text}");
        self.toasts.push(level, text);
    }

    /// True while there's something to draw.
    pub fn visible(&self) -> bool {
        self.open || self.toasts.active()
    }

    /// The notices, if any are showing.
    pub fn notices(&mut self, ctx: &egui::Context) {
        self.toasts.show(ctx);
    }

    /// `notices` for photo mode: at the top middle, the dock having the
    /// bottom.
    pub fn photo_notices(&mut self, ctx: &egui::Context) {
        let s = theme::scale(ctx);
        self.toasts.show_at(ctx, egui::Align2::CENTER_TOP, [0.0, 20.0 * s]);
    }

    /// Note where this frame's input came from: `navs` (the pad's) turn the
    /// pad's focus and hints on, `keys` (the keyboard's navigation keys)
    /// the focus alone, the pointer or other keys both off.
    fn note_input(&mut self, ctx: &egui::Context, navs: &[Nav], keys: &[Nav]) {
        let other = crate::focus::pointer_or_other_key(ctx);
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
        if self.open && !self.was_open {
            self.toasts.dismiss_errors();
        }
        self.was_open = self.open;
        self.notices(ctx);
        if !self.open {
            self.capture = None;
            self.page = Page::Pause;
            self.pause_focus = Focus::default();
            self.slots_focus = Focus::default();
            self.settings_focus = Focus::default();
            self.ctl = controls::State::default();
            self.picture = picture::State::default();
            self.thumbs.clear();
            self.reset_armed = None;
            return actions;
        }
        self.picture.wipe_rect = None;
        if view.playing.is_some() && !self.picture.comparing(self.page) && self.page != Page::Controls {
            paused_pill(ctx);
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
        if self.page == Page::Controls {
            match bindings {
                Some(b) => {
                    if self.controls_page(ctx, settings, prefs, view, b, navs) {
                        actions.push(Action::Bindings);
                    }
                    return actions;
                }
                // Nothing to bind: back to the pause menu.
                None => self.page = Page::Pause,
            }
        }
        match self.page {
            Page::Pause => self.pause_page(ctx, settings, view, navs, &mut actions),
            Page::SaveLoad => self.save_page(ctx, view, navs, &mut actions),
            Page::Settings => self.settings_page(ctx, settings, prefs, view, navs),
            Page::Picture => self.picture_page(ctx, settings, prefs, view, navs),
            Page::Advanced => self.advanced_page(ctx, settings, prefs, view, navs),
            Page::Controls => {}
        }
        actions
    }

    /// The pause menu: what to do (the top rows), then where the settings
    /// are, each with its value.
    fn pause_page(
        &mut self,
        ctx: &egui::Context,
        settings: &GameSettings,
        view: &View,
        navs: &[Nav],
        actions: &mut Vec<Action>,
    ) {
        let focus = &mut self.pause_focus;
        focus.show = self.pad_used || self.keys_used;
        for nav in focus.begin(navs) {
            // B resumes the game, or from the launcher closes the menu.
            if nav == Nav::Back {
                actions.push(Action::Resume);
            }
        }
        if self.reset_armed.is_some_and(|t| t.elapsed() > RESET_CONFIRM) {
            self.reset_armed = None;
        }
        let playing = view.playing.is_some();
        let armed = self.reset_armed.is_some();
        let used = view.saves.iter().flatten().count();
        let picture = settings.preset.label();
        let sound = format!("Music {}{} dB", if settings.music_db > 0.0 { "+" } else { "" }, settings.music_db);
        let speed = if settings.run_ahead > 0 {
            format!("Run-ahead {}", settings.run_ahead)
        } else if settings.exact {
            "Exact speed".to_string()
        } else {
            "Smooth".to_string()
        };
        let mut go: Option<(Page, Tab)> = None;
        let mut arm = false;
        let s = theme::scale(ctx);
        let way_out = if playing { "Resume" } else { "Close" };
        // The way out is the pad's B or the keyboard's Esc, by what is in use.
        let hints = if self.pad_used {
            [(Chip::A, "Select"), (Chip::B, way_out)]
        } else {
            [(Chip::Key("Enter"), "Select"), (Chip::Key("Esc"), way_out)]
        };
        let spec = PanelSpec {
            game: view.playing.or(view.game.map(|g| g.title)).unwrap_or("reModel"),
            heading: if playing { "Paused" } else { "Menu" },
            back: false,
            extra_h: 0.0,
        };
        side_panel(ctx, &spec, &hints, |ui, reg| {
            ui.scope_builder(egui::UiBuilder::new().max_rect(reg.body.shrink2(vec2(12.0 * s, 0.0))), |ui| {
                focus.scroll_area("pause").show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0 * s;
                    ui.add_space(8.0 * s);
                    if playing {
                        if focus.menu_row(ui, "Resume", "Esc") {
                            actions.push(Action::Resume);
                        }
                        if focus.menu_row(ui, "Quick save", "F5") {
                            actions.push(Action::Save(0));
                        }
                        if focus.menu_row(ui, "Quick load", "F9") {
                            actions.push(Action::Load(0));
                        }
                        if focus.menu_row(ui, "Save slots", &format!("{used} used ›")) {
                            go = Some((Page::SaveLoad, Tab::Display));
                        }
                        if focus.menu_row(ui, "Photo mode", "F8") {
                            actions.push(Action::Photo);
                        }
                        if focus.menu_row(ui, "Reset game", if armed { "again to confirm" } else { "" }) {
                            if armed {
                                actions.push(Action::Reset);
                            } else {
                                arm = true;
                            }
                        }
                    }
                    if playing && focus.menu_row(ui, "Back to games", "") {
                        actions.push(Action::Games);
                    }
                    ui.add_space(10.0 * s);
                    ui.horizontal(|ui| {
                        ui.add_space(20.0 * s);
                        ui.label(
                            egui::RichText::new("SETTINGS")
                                .font(FontId::new(13.0 * s, theme::semibold()))
                                .color(theme::TEXT_DIM),
                        );
                    });
                    ui.add_space(4.0 * s);
                    if focus.menu_row(ui, "Picture", &format!("{picture} ›")) {
                        go = Some((Page::Picture, Tab::Display));
                    }
                    // (From the launcher the game is the one selected.)
                    if (playing || view.game.is_some()) && focus.menu_row(ui, "Controls", "›") {
                        go = Some((Page::Controls, Tab::Display));
                    }
                    if focus.menu_row(ui, "Sound", &format!("{sound} ›")) {
                        go = Some((Page::Settings, Tab::Sound));
                    }
                    if focus.menu_row(ui, "Speed and latency", &format!("{speed} ›")) {
                        go = Some((Page::Settings, Tab::Motion));
                    }
                    if !playing && view.has_own && focus.menu_row(ui, "Use the settings for all games", "") {
                        actions.push(Action::ResetOwn);
                    }
                    if !playing && focus.menu_row(ui, "Quit reModel", "") {
                        actions.push(Action::Quit);
                    }
                    ui.add_space(6.0 * s);
                    ui.horizontal(|ui| {
                        ui.add_space(20.0 * s);
                        ui.label(
                            egui::RichText::new(format!("reModel {}", remodel_version::VERSION))
                                .font(FontId::new(14.0 * s, FontFamily::Proportional))
                                .color(theme::TEXT_DIM),
                        );
                    });
                });
            });
            false
        });
        self.pause_focus.end();
        if arm {
            self.reset_armed = Some(Instant::now());
            self.warn("Select Reset game again to confirm");
        }
        if let Some((page, tab)) = go {
            self.reset_armed = None;
            self.page = page;
            if page == Page::Settings {
                self.tab = tab;
            }
            self.slots_focus = Focus::default();
            self.settings_focus = Focus::default();
            self.picture = picture::State::default();
            self.ctl = controls::State::default();
        }
    }

    /// Bring the slots' thumbnails up to date: a picture just taken, or
    /// the PNG beside a state read when it is new to the menu (a page
    /// at a time, as the slots page opens, not every frame).
    fn sync_thumbs(&mut self, ctx: &egui::Context, saves: &[Option<SlotInfo>]) {
        for slot in 0..SLOTS {
            let Some(info) = saves.get(slot as usize).and_then(|i| i.as_ref()) else {
                self.thumbs.remove(&slot);
                self.fresh.remove(&slot);
                continue;
            };
            let stamp = info.thumb.as_ref().map(|t| t.1);
            let image = if let Some(img) = self.fresh.remove(&slot) {
                Some(img)
            } else if let Some((path, _)) =
                info.thumb.as_ref().filter(|(_, t)| self.thumbs.get(&slot).is_none_or(|k| k.stamp != Some(*t)))
            {
                saves::read_thumb(path).map(|(size, rgba)| egui::ColorImage::from_rgba_unmultiplied(size, &rgba))
            } else {
                // Nothing new; a picture of an older save with none now.
                if self.thumbs.get(&slot).is_some_and(|k| k.saved != info.time && stamp.is_none()) {
                    self.thumbs.remove(&slot);
                }
                continue;
            };
            match image {
                Some(img) => {
                    let tex = ctx.load_texture(format!("slot-{slot}"), img, egui::TextureOptions::LINEAR);
                    self.thumbs.insert(slot, Thumb { saved: info.time, stamp, tex });
                }
                None => {
                    self.thumbs.remove(&slot);
                }
            }
        }
    }

    /// The picture of the state just saved to `slot`, for the slots page
    /// (its PNG is being written beside the state).
    pub fn fresh_thumb(&mut self, slot: u8, size: [usize; 2], rgba: Vec<u8>) {
        if rgba.len() == size[0] * size[1] * 4 {
            self.fresh.insert(slot, egui::ColorImage::from_rgba_unmultiplied(size, &rgba));
        }
    }

    /// The save slots: a card each, then Load, Save here and Back.
    fn save_page(&mut self, ctx: &egui::Context, view: &View, navs: &[Nav], actions: &mut Vec<Action>) {
        self.sync_thumbs(ctx, view.saves);
        let focus = &mut self.slots_focus;
        focus.show = self.pad_used || self.keys_used;
        let mut back = focus.begin(navs).contains(&Nav::Back);
        let mut sel = self.slot_sel.min(SLOTS - 1);
        let thumbs = &self.thumbs;
        let s = theme::scale(ctx);
        let hints = [(Chip::A, "Select"), (Chip::B, "Back"), (Chip::Key("Esc"), "Resume")];
        let spec =
            PanelSpec { game: view.playing.unwrap_or("reModel"), heading: "Save slots", back: true, extra_h: 68.0 };
        let mut jump = None;
        back |= side_panel(ctx, &spec, &hints, |ui, reg| {
            let (load_id, save_id) =
                (ui.make_persistent_id(("pill", "Load")), ui.make_persistent_id(("pill", "Save here")));
            ui.scope_builder(egui::UiBuilder::new().max_rect(reg.body.shrink2(vec2(24.0 * s, 0.0))), |ui| {
                focus.scroll_area("slots").show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0 * s;
                    ui.add_space(12.0 * s);
                    for slot in 0..SLOTS {
                        let id = ui.make_persistent_id(("slot-card", slot));
                        let info = view.saves.get(slot as usize).and_then(|i| i.as_ref());
                        let name = if slot == 0 { "Quick save".to_string() } else { format!("Slot {slot}") };
                        if slot_card(ui, focus, id, &name, sel == slot, info, thumbs.get(&slot).map(|t| &t.tex)) {
                            // Selected: on to what's done with it.
                            sel = slot;
                            jump = Some(if info.is_some_and(|i| i.loads) { load_id } else { save_id });
                        }
                    }
                    ui.add_space(16.0 * s);
                });
            });
            // Load, Save here, Back.
            let foot = reg.extra;
            ui.painter().hline(foot.x_range(), foot.top(), Stroke::new(1.0, Color32::from_white_alpha(20)));
            let (pad, gap, h) = (20.0 * s, 8.0 * s, 44.0 * s);
            let top = foot.center().y - h / 2.0;
            let back_w = 84.0 * s;
            let w = (foot.width() - 2.0 * pad - 2.0 * gap - back_w) / 2.0;
            let at = |x: f32, w: f32| Rect::from_min_size(pos2(x, top), vec2(w, h));
            let info = view.saves.get(sel as usize).and_then(|i| i.as_ref());
            let can_load = info.is_some_and(|i| i.loads);
            let x = foot.left() + pad;
            if pill(ui, focus, at(x, w), "Load", Some(theme::BLUE), can_load) {
                actions.push(Action::Load(sel));
            }
            if pill(ui, focus, at(x + w + gap, w), "Save here", Some(theme::RAISED), true) {
                actions.push(Action::Save(sel));
            }
            let mut back = false;
            if pill(ui, focus, at(x + 2.0 * (w + gap), back_w), "Back", None, true) {
                back = true;
            }
            back
        });
        self.slot_sel = sel;
        if let Some(id) = jump {
            self.slots_focus.focus_to(id);
        }
        self.slots_focus.end();
        if back {
            self.page = Page::Pause;
            self.pause_focus = Focus::default();
        }
    }

    /// The settings: tabs of rows.
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
        let mut tab = self.tab;
        let s = theme::scale(ctx);
        let focus = &mut self.settings_focus;
        focus.show = nav_mode;
        let mut rows = Rows { focus, nav_mode, focused: None, hovered: None };
        let hints = [(Chip::LeftRight, "Change"), (Chip::Key("LB RB"), "Tabs"), (Chip::B, "Back")];
        let spec =
            PanelSpec { game: view.playing.unwrap_or("reModel"), heading: tab.heading(), back: true, extra_h: 120.0 };
        back |= side_panel(ctx, &spec, &hints, |ui, reg| {
            ui.scope_builder(egui::UiBuilder::new().max_rect(reg.body.shrink2(vec2(16.0 * s, 0.0))), |ui| {
                tabs(ui, &mut tab);
                rows.focus.scroll_area("settings").show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0 * s;
                    match tab {
                        Tab::Display => display_rows(ui, &mut rows, settings, &mut prefs, view),
                        Tab::Motion => motion_rows(ui, &mut rows, settings, view),
                        Tab::Sound => sound_rows(ui, &mut rows, settings),
                        Tab::More => more_rows(ui, &mut rows, settings, view),
                    }
                });
            });
            ui.painter().hline(reg.extra.x_range(), reg.extra.top(), Stroke::new(1.0, Color32::from_white_alpha(20)));
            ui.scope_builder(egui::UiBuilder::new().max_rect(reg.extra.shrink2(vec2(16.0 * s, 6.0 * s))), |ui| {
                ui.spacing_mut().item_spacing.y = 4.0 * s;
                rows.choose(
                    ui,
                    "Applies to",
                    &mut *prefs.for_all,
                    &[(false, "This game"), (true, "All games")],
                    "Where changes on these screens are kept: for this game only, or as the settings of every \
                     game that has none of its own.",
                );
                let help = rows.help();
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(help)
                            .font(FontId::new(14.0 * s, FontFamily::Proportional))
                            .color(theme::TEXT_DIM),
                    )
                    .wrap(),
                );
            });
            false
        });
        self.settings_focus.end();
        if tab != self.tab {
            self.tab = tab;
            self.settings_focus = Focus::default();
        }
        // A picture style whose rows no longer all match (texture
        // filtering, on More) is Custom.
        *settings = settings.clone().settled(view.game);
        if back {
            self.page = Page::Pause;
            self.pause_focus = Focus::default();
        }
    }
}

/// The side panel's width, and its header's and footer's heights, in
/// design points (720 high).
const PANEL_W: f32 = 380.0;
const HEADER_H: f32 = 84.0;
const FOOTER_H: f32 = 48.0;
/// Reset game wants a second selection within this long.
const RESET_CONFIRM: Duration = Duration::from_secs(4);

/// The panel's rectangle and the regions in it, tiling it top to bottom:
/// header, body, extra (a page's own footer, `extra_h` design points, maybe
/// none) and the hint chips' footer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Regions {
    pub panel: Rect,
    pub header: Rect,
    pub body: Rect,
    pub extra: Rect,
    pub foot: Rect,
}

fn regions(screen: Rect, s: f32, extra_h: f32) -> Regions {
    let panel = Rect::from_min_max(
        pos2(screen.right() - (PANEL_W * s).min(screen.width()), screen.top()),
        screen.right_bottom(),
    );
    let y = |v: f32| (panel.top() + v).clamp(panel.top(), panel.bottom());
    let (header_b, foot_t) = (y(HEADER_H * s), y(panel.height() - FOOTER_H * s));
    let extra_t = y(foot_t - panel.top() - extra_h * s).max(header_b);
    let band = |top: f32, bottom: f32| Rect::from_x_y_ranges(panel.x_range(), top..=bottom);
    Regions {
        panel,
        header: band(panel.top(), header_b),
        body: band(header_b, extra_t),
        extra: band(extra_t, foot_t),
        foot: band(foot_t, panel.bottom()),
    }
}

/// What a page tells the panel.
pub(crate) struct PanelSpec<'a> {
    /// The game (small, above the heading).
    pub game: &'a str,
    pub heading: &'a str,
    /// A back arrow in the header.
    pub back: bool,
    /// Height of the page's own footer region (design points).
    pub extra_h: f32,
}

/// The side panel the menu pages are in, docked on the right edge over the
/// paused game: its shadow, edge, header and button hints; `content` fills
/// the rest (its regions) and returns whether the page was asked to go back.
/// True if it was, or the header's back arrow was clicked.
pub(crate) fn side_panel(
    ctx: &egui::Context,
    spec: &PanelSpec,
    hints: &[(Chip, &str)],
    content: impl FnOnce(&mut egui::Ui, &Regions) -> bool,
) -> bool {
    let s = theme::scale(ctx);
    let reg = regions(ctx.content_rect(), s, spec.extra_h);
    let mut back = false;
    egui::Area::new(egui::Id::unique("remodel-menu-panel"))
        .order(egui::Order::Middle)
        .fixed_pos(reg.panel.min)
        .constrain(false)
        .fade_in(false)
        .show(ctx, |ui| {
            // The panel takes the pointer, so the game's picture behind it
            // doesn't.
            ui.allocate_rect(reg.panel, Sense::click_and_drag());
            let p = ui.painter().clone();
            p.add(
                egui::Shadow { offset: [-10, 0], blur: 60, spread: 0, color: Color32::from_black_alpha(140) }
                    .as_shape(reg.panel, 0.0),
            );
            p.rect_filled(reg.panel, 0.0, theme::PANEL_FILL);
            p.vline(reg.panel.left(), reg.panel.y_range(), Stroke::new(1.0, theme::PANEL_EDGE));
            let rule = Stroke::new(1.0, Color32::from_white_alpha(20));
            p.hline(reg.header.x_range(), reg.header.bottom(), rule);
            p.hline(reg.foot.x_range(), reg.foot.top(), rule);
            // The header: the game, then the page's heading.
            let x = reg.header.left() + 26.0 * s;
            p.text(
                pos2(x, reg.header.top() + 20.0 * s),
                Align2::LEFT_TOP,
                spec.game.to_uppercase(),
                FontId::new(13.0 * s, theme::semibold()),
                theme::CYAN_LIGHT,
            );
            let size = if spec.heading.len() > 12 { 22.0 } else { 26.0 };
            p.text(
                pos2(x, reg.header.top() + 40.0 * s),
                Align2::LEFT_TOP,
                spec.heading.to_uppercase(),
                FontId::new(size * s, theme::display()),
                theme::TEXT,
            );
            if spec.back {
                let r = Rect::from_center_size(
                    pos2(reg.header.right() - 26.0 * s - 16.0 * s, reg.header.top() + 30.0 * s),
                    vec2(32.0 * s, 32.0 * s),
                );
                let resp = ui.interact(r, egui::Id::unique("remodel-menu-back"), Sense::click());
                let fill = if resp.hovered() { Color32::from_white_alpha(36) } else { theme::RAISED };
                p.circle_filled(r.center(), r.width() / 2.0, fill);
                p.text(
                    r.center() - vec2(1.0, 2.0 * s),
                    Align2::CENTER_CENTER,
                    "‹",
                    FontId::new(22.0 * s, theme::bold()),
                    theme::TEXT,
                );
                back |= resp.clicked();
            }
            back |= content(ui, &reg);
            let chips = Rect::from_min_size(
                pos2(reg.foot.left() + 26.0 * s, reg.foot.top() + 12.0 * s),
                vec2(reg.foot.width() - 52.0 * s, 24.0 * s),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(chips), |ui| theme::chips(ui, hints));
        });
    back
}

/// The settings' tab strip: the one clicked is chosen.
fn tabs(ui: &mut egui::Ui, tab: &mut Tab) {
    let s = theme::scale(ui.ctx());
    let font = FontId::new(15.0 * s, theme::semibold());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0 * s), Sense::hover());
    let mut x = rect.left() + 8.0 * s;
    for (i, t) in Tab::ALL.iter().enumerate() {
        let w = ui.painter().layout_no_wrap(t.label().to_string(), font.clone(), theme::TEXT).size().x;
        let r = Rect::from_min_size(pos2(x - 6.0 * s, rect.top()), vec2(w + 12.0 * s, rect.height()));
        let resp = ui.interact(r, ui.make_persistent_id(("settings-tab", i)), Sense::click());
        let on = *t == *tab;
        let colour = if on {
            theme::TEXT
        } else if resp.hovered() {
            theme::TEXT_2
        } else {
            theme::TEXT_DIM
        };
        ui.painter().text(pos2(x, rect.center().y - 1.0 * s), Align2::LEFT_CENTER, t.label(), font.clone(), colour);
        if on {
            let y = rect.bottom() - 4.0 * s;
            ui.painter().rect_filled(Rect::from_min_size(pos2(x, y), vec2(w, 3.0 * s)), 1.5 * s, theme::CYAN);
        }
        if resp.clicked() {
            *tab = *t;
        }
        x += w + 18.0 * s;
    }
}

/// A footer button in `rect`: `fill` None is an outline. Clicked, or
/// activated from the pad. Disabled, it can't be focused.
fn pill(ui: &mut egui::Ui, focus: &mut Focus, rect: Rect, text: &str, fill: Option<Color32>, enabled: bool) -> bool {
    let s = theme::scale(ui.ctx());
    let id = ui.make_persistent_id(("pill", text));
    ui.add_enabled_ui(enabled, |ui| {
        let r = ui.interact(rect, id, Sense::click());
        let hover = r.hovered() && enabled;
        let p = ui.painter();
        let radius = theme::ROW_RADIUS * s;
        let dim = if enabled { 1.0 } else { 0.4 };
        match fill {
            Some(f) => {
                let f = if hover { f.gamma_multiply(1.15) } else { f };
                p.rect_filled(rect, radius, f.gamma_multiply(dim));
            }
            None => {
                if hover {
                    p.rect_filled(rect, radius, Color32::from_white_alpha(18));
                }
                p.rect_stroke(rect, radius, Stroke::new(1.0, Color32::from_white_alpha(51)), egui::StrokeKind::Inside);
            }
        }
        let ink = if enabled { theme::TEXT } else { theme::TEXT_DIM };
        p.text(rect.center(), Align2::CENTER_CENTER, text, FontId::new(16.0 * s, theme::bold()), ink);
        if focus.is_on(id) {
            theme::focus_ring(p, rect, s, theme::ROW_RADIUS);
        }
        focus.register(ui, &r)
    })
    .inner
}

/// A slot's card: its thumbnail (or "Empty"), name, when and by what. The
/// selected one has a cyan border and a faint fill. Clicked, or activated
/// from the pad.
pub(crate) fn slot_card(
    ui: &mut egui::Ui,
    focus: &mut Focus,
    id: egui::Id,
    name: &str,
    selected: bool,
    info: Option<&SlotInfo>,
    thumb: Option<&egui::TextureHandle>,
) -> bool {
    let s = theme::scale(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 88.0 * s), Sense::hover());
    let r = ui.interact(rect, id, Sense::click());
    let radius = theme::ROW_RADIUS * s;
    let p = ui.painter().clone();
    if selected {
        p.rect_filled(rect, radius, Color32::from_rgba_unmultiplied(34, 229, 255, 26));
        p.rect_stroke(rect, radius, Stroke::new(2.0 * s, theme::CYAN), egui::StrokeKind::Inside);
    } else {
        let white = if r.hovered() { 14 } else { 8 };
        p.rect_filled(rect, radius, Color32::from_white_alpha(white));
    }
    let pic = Rect::from_min_size(rect.min + vec2(8.0 * s, 8.0 * s), vec2(96.0 * s, 72.0 * s));
    match thumb {
        Some(tex) => egui::Image::from_texture((tex.id(), pic.size())).corner_radius(8.0 * s).paint_at(ui, pic),
        None => {
            p.rect_filled(pic, 8.0 * s, Color32::from_rgb(20, 27, 44));
            let text = if info.is_some() { "No preview" } else { "Empty" };
            p.text(
                pic.center(),
                Align2::CENTER_CENTER,
                text,
                FontId::new(14.0 * s, FontFamily::Proportional),
                Color32::from_rgb(109, 122, 148),
            );
        }
    }
    let x = pic.right() + 14.0 * s;
    let wrap = (rect.right() - 10.0 * s - x).max(40.0);
    let mut y = rect.top() + 10.0 * s;
    p.text(pos2(x, y), Align2::LEFT_TOP, name, FontId::new(17.0 * s, theme::bold()), theme::TEXT);
    y += 23.0 * s;
    if let Some(info) = info {
        for (text, size, colour) in
            [(saves::when(info.time), 14.0, theme::TEXT_2), (saves::meta(info), 14.0, theme::TEXT_DIM)]
        {
            let galley = p.layout(text, FontId::new(size * s, FontFamily::Proportional), colour, wrap);
            let h = galley.size().y;
            p.galley(pos2(x, y), galley, colour);
            y += h + 3.0 * s;
        }
    }
    if focus.is_on(id) {
        theme::focus_ring(&p, rect, s, theme::ROW_RADIUS);
    }
    focus.register(ui, &r)
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
    if run_ahead_shown(view.game, s.run_ahead) {
        // What the game takes, and a setting past that (to turn it off).
        let list: Vec<(u32, &str)> =
            RUN_AHEAD.iter().copied().filter(|(n, _)| *n <= max || *n == s.run_ahead).collect();
        let help = run_ahead_help(view.game, view.run_ahead_dropped, view.run_ahead_room, s.run_ahead);
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
        /// The text drawn in the last frame.
        texts: Vec<String>,
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
                texts: Vec::new(),
            };
            // One frame to lay it out.
            rig.frame(&[], Vec::new());
            rig
        }

        fn frame(&mut self, navs: &[Nav], events: Vec<egui::Event>) {
            let saved = SlotInfo { time: SystemTime::now(), made_by: Some("v0.9.0".into()), loads: true, thumb: None };
            let saves = [Some(saved), None, None, None, None];
            let view = View {
                playing: Some(self.game.title),
                game: Some(self.game),
                saves: &saves,
                max_anisotropy: 16,
                temporal_supported: true,
                has_bezel: true,
                has_own: false,
                run_ahead_dropped: self.dropped,
                run_ahead_room: None,
                pack: None,
                materials: None,
                look: None,
                pad: Default::default(),
                picture: Some([0.0, 0.0, self.size.x, self.size.y]),
                keeps: Keeps { all_games: Preset::Sharp, own_picture: false },
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
            self.texts = out
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                    _ => None,
                })
                .collect();
        }

        /// Whether the last frame drew text containing `needle`.
        fn shows(&self, needle: &str) -> bool {
            self.texts.iter().any(|t| t.contains(needle))
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

        /// Into the Settings screen (by Sound, then back two tabs), onto its
        /// first row.
        fn open_settings(&mut self) -> &mut Rig {
            self.press(9, Nav::Down).run(&[&[Nav::Accept], &[], &[Nav::PrevGroup], &[Nav::PrevGroup], &[]]);
            assert_eq!(self.menu.tab, Tab::Display);
            self
        }

        /// Into the Picture page, onto its first widget (the scope switch).
        fn open_picture(&mut self) -> &mut Rig {
            self.press(7, Nav::Down).run(&[&[Nav::Accept], &[]]);
            assert_eq!(self.menu.page, Page::Picture);
            self
        }

        /// Into the Advanced page, onto its first row.
        fn open_advanced(&mut self) -> &mut Rig {
            // The scope switch, five styles, Compare, Advanced.
            self.open_picture().press(8, Nav::Down).run(&[&[Nav::Accept], &[], &[]]);
            assert_eq!(self.menu.page, Page::Advanced);
            self
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
                Action::ResetOwn => "reset-own",
                Action::Reset => "reset",
                Action::Photo => "photo",
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
    fn the_pause_menu_lists_what_to_do_then_the_settings() {
        // Resume, Quick save, Quick load, Save slots, Photo mode, Reset
        // game, Back to games; then Picture, Controls, Sound, Speed and
        // latency.
        let at = |downs: usize| {
            let mut r = Rig::new("topskatr");
            r.press(downs, Nav::Down).run(&[&[Nav::Accept], &[]]);
            r
        };
        assert_eq!(at(1).names(), ["save"]);
        assert_eq!(at(2).names(), ["load"]);
        assert_eq!(at(3).menu.page, Page::SaveLoad);
        assert_eq!(at(4).names(), ["photo"]);
        assert_eq!(at(6).names(), ["games"]);
        assert_eq!(at(7).menu.page, Page::Picture);
        assert!(at(8).menu.on_controls());
        let r = at(9);
        assert_eq!((r.menu.page, r.menu.tab), (Page::Settings, Tab::Sound));
        // Speed and latency is the last row.
        for downs in [10, 30] {
            let r = at(downs);
            assert_eq!((r.menu.page, r.menu.tab), (Page::Settings, Tab::Motion), "{downs}");
        }
    }

    #[test]
    fn reset_game_asks_twice() {
        let mut r = Rig::new("topskatr");
        r.press(5, Nav::Down).run(&[&[Nav::Accept], &[]]);
        assert!(r.actions.is_empty(), "the first only asks");
        r.run(&[&[Nav::Accept]]);
        assert_eq!(r.names(), ["reset"]);
        // Moving the focus about does not disarm it.
        let mut r = Rig::new("topskatr");
        r.press(5, Nav::Down).run(&[&[Nav::Accept], &[Nav::Up], &[Nav::Down], &[Nav::Accept]]);
        assert_eq!(r.names(), ["reset"], "moving the focus doesn't");
        let mut r = Rig::new("topskatr");
        r.press(5, Nav::Down).run(&[&[Nav::Accept]]);
        r.menu.reset_armed = r.menu.reset_armed.map(|t| t - RESET_CONFIRM * 2);
        r.run(&[&[Nav::Accept]]);
        assert!(r.actions.is_empty(), "too late: it asks again");
    }

    #[test]
    fn save_and_load_slots() {
        let mut r = Rig::new("topskatr");
        // Down to Save slots, in. A on the quick slot (saved) selects it
        // and goes on to Load.
        r.press(3, Nav::Down).run(&[&[Nav::Accept], &[]]);
        assert_eq!(r.menu.page, Page::SaveLoad);
        r.run(&[&[Nav::Accept], &[Nav::Accept]]);
        assert!(matches!(&r.actions[..], [Action::Load(0)]));
        // Slot 1 is empty: A selects it and goes on to Save here (Load
        // can't be focused).
        let mut r = Rig::new("topskatr");
        r.press(3, Nav::Down).run(&[&[Nav::Accept], &[], &[Nav::Down], &[Nav::Accept], &[Nav::Accept]]);
        assert!(matches!(&r.actions[..], [Action::Save(1)]));
        assert_eq!(r.menu.slot_sel, 1);
        // B goes back to the pause menu, which B then closes.
        r.run(&[&[Nav::Back], &[]]);
        assert_eq!(r.menu.page, Page::Pause);
        r.run(&[&[Nav::Back]]);
        assert_eq!(r.names(), ["save", "resume"]);
    }

    #[test]
    fn thumbnails_become_textures_when_the_slots_page_opens() {
        let mut r = Rig::new("topskatr");
        // A picture just taken for the quick slot (saved), another for slot
        // 3 (not saved: dropped).
        r.menu.fresh_thumb(0, [2, 2], vec![200; 16]);
        r.menu.fresh_thumb(3, [2, 2], vec![200; 16]);
        r.menu.fresh_thumb(1, [2, 2], vec![200; 3]);
        assert!(r.menu.thumbs.is_empty(), "nothing is loaded before the page opens");
        r.press(3, Nav::Down).run(&[&[Nav::Accept], &[]]);
        assert_eq!(r.menu.page, Page::SaveLoad);
        assert!(r.menu.thumbs.contains_key(&0));
        assert!(!r.menu.thumbs.contains_key(&3) && !r.menu.thumbs.contains_key(&1));
        // Closing the menu frees them.
        r.menu.open = false;
        r.run(&[&[]]);
        assert!(r.menu.thumbs.is_empty());
    }

    #[test]
    fn the_panel_regions_tile_its_edge() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 720.0));
        let r = regions(screen, 1.0, 80.0);
        assert_eq!(r.panel, Rect::from_min_max(pos2(900.0, 0.0), pos2(1280.0, 720.0)));
        assert_eq!(r.header.top(), 0.0);
        assert_eq!(r.header.bottom(), r.body.top());
        assert_eq!(r.body.bottom(), r.extra.top());
        assert_eq!(r.extra.bottom(), r.foot.top());
        assert_eq!(r.foot.bottom(), 720.0);
        assert_eq!(r.extra.height(), 80.0);
        // At 4K the panel is as much of the screen as at 720p.
        let big = regions(Rect::from_min_size(pos2(0.0, 0.0), vec2(3840.0, 2160.0)), 3.0, 0.0);
        assert_eq!(big.panel.width(), 1140.0);
        assert_eq!(big.extra.height(), 0.0);
        // A window too short for its regions squeezes the body to nothing.
        let tiny = regions(Rect::from_min_size(pos2(0.0, 0.0), vec2(500.0, 200.0)), 1.0, 80.0);
        assert!(tiny.body.height() >= 0.0 && tiny.panel.width() == 380.0);
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
        assert_eq!(r.menu.tab, Tab::Sound);
        r.run(&[&[Nav::PrevGroup]]);
        assert_eq!(r.menu.tab, Tab::Motion);
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
        assert_eq!(r.menu.tab, Tab::Motion);
        r.frame(&[], vec![key(egui::Key::Tab, false)]);
        assert_eq!(r.menu.tab, Tab::Sound);
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

    /// The styles, from the pad: the scope switch, then a card each (Arcade
    /// original, Sharp, Smooth modern, CRT, Custom), Compare, Advanced.
    #[test]
    fn choosing_a_style_sets_its_rows() {
        let mut r = Rig::new("topskatr");
        assert_eq!(r.s.preset, Preset::Original);
        r.open_picture().press(2, Nav::Down).press(1, Nav::Accept);
        assert_eq!((r.s.preset, r.s.aa, r.s.tex_filter), (Preset::Sharp, 4, model2::enhance::TexFilter::Mip));
        assert_eq!((r.s.temporal, r.s.depth, r.s.shading), (0, false, gpu::Shading::Flat));
        assert_eq!((r.s.smooth, r.s.crt), (false, 0));
        // Smooth modern: for topskatr temporal at full size, the depth
        // buffer, smooth shading, in-between frames.
        r.press(1, Nav::Down).press(1, Nav::Accept);
        assert_eq!(r.s.preset, Preset::Enhanced);
        assert_eq!((r.s.temporal, r.s.depth, r.s.shading), (100, true, gpu::Shading::Vertex));
        assert_eq!((r.s.smooth, r.s.run_ahead, r.s.crt), (true, 0, 0));
        // CRT: Sharp with scanlines; nothing of Smooth modern left.
        r.press(1, Nav::Down).press(1, Nav::Accept);
        assert_eq!((r.s.preset, r.s.aa, r.s.crt), (Preset::Crt, 4, crate::settings::CRT_STYLE_STRENGTH));
        assert_eq!((r.s.temporal, r.s.depth, r.s.smooth), (0, false, false));
        // Back to Arcade original: everything off again, CRT too.
        r.press(3, Nav::Up).press(1, Nav::Accept);
        assert_eq!(r.s.preset, Preset::Original);
        assert_eq!(GameSettings { sfx_db: r.s.sfx_db, music_db: r.s.music_db, ..r.s.clone() }, GameSettings::default());
        // Custom with nothing made before keeps the rows as they are.
        r.press(5, Nav::Down).press(1, Nav::Accept);
        assert_eq!((r.s.preset, r.s.aa, r.s.crt), (Preset::Custom, 0, 0));
        // B goes back to the pause menu.
        r.run(&[&[Nav::Back], &[]]);
        assert_eq!(r.menu.page, Page::Pause);
    }

    /// Choosing Custom again brings back the last mix the player made.
    #[test]
    fn custom_restores_the_last_custom_values() {
        let mut r = Rig::new("topskatr");
        r.s.preset = Preset::Custom;
        (r.s.aa, r.s.bloom, r.s.crt) = (8, 50, 30);
        r.open_picture().press(2, Nav::Down).press(1, Nav::Accept);
        assert_eq!((r.s.preset, r.s.aa, r.s.bloom, r.s.crt), (Preset::Sharp, 4, 0, 0));
        r.press(4, Nav::Down).press(1, Nav::Accept);
        assert_eq!((r.s.preset, r.s.aa, r.s.bloom, r.s.crt), (Preset::Custom, 8, 50, 30));
    }

    /// Where it's saved: the switch is the settings' scope, and the footer
    /// says what the other games keep.
    #[test]
    fn the_switch_says_where_it_is_saved() {
        let mut r = Rig::new("topskatr");
        r.open_picture();
        assert!(!r.for_all);
        assert!(r.shows("Saved for Top Skater. Other games keep Sharp."), "{:?}", r.texts);
        r.press(1, Nav::Right).press(1, Nav::Up);
        assert!(r.for_all);
        assert!(r.shows("Saved for all games (Top Skater has no style of its own)."), "{:?}", r.texts);
        r.press(1, Nav::Left);
        assert!(!r.for_all);
        // A is the switch too.
        r.press(1, Nav::Accept);
        assert!(r.for_all);
    }

    /// Compare: A switches it, then left and right move the divider (from
    /// Compare itself or any other widget); the part of the picture left of
    /// it goes to `wipe_rect`.
    #[test]
    fn compare_moves_with_the_pad() {
        let mut r = Rig::new("topskatr");
        r.open_picture();
        assert_eq!(r.menu.wipe_rect(), None, "off");
        // The scope switch, five styles, then Compare.
        r.press(7, Nav::Down).press(1, Nav::Accept).run(&[&[], &[]]);
        assert!(r.menu.picture.compare);
        // The picture is the window (1920x1080: 1.5x), the panel 570 wide.
        let visible = 1920.0 - 570.0;
        let rect = |wipe: f32| Some((0, 0, (wipe * visible).round() as u32, 1080));
        assert_eq!(r.menu.wipe_rect(), rect(0.5));
        r.press(3, Nav::Right);
        assert_eq!(r.menu.wipe_rect(), rect(0.5 + 3.0 * picture::WIPE_STEP));
        // From a card too (nothing is beside it): left.
        r.press(2, Nav::Up).press(2, Nav::Left);
        assert_eq!(r.menu.wipe_rect(), rect(0.5 + picture::WIPE_STEP));
        // To the ends and no further.
        r.press(40, Nav::Left);
        assert_eq!(r.menu.wipe_rect(), Some((0, 0, 0, 1080)));
        r.press(60, Nav::Right);
        assert_eq!(r.menu.wipe_rect(), rect(1.0));
        // On the other page it's gone.
        r.press(3, Nav::Down).press(1, Nav::Accept);
        assert_eq!(r.menu.page, Page::Advanced);
        assert_eq!(r.menu.wipe_rect(), None);
    }

    /// The pointer drags the divider.
    #[test]
    fn compare_drags_with_the_pointer() {
        let mut r = Rig::new("topskatr");
        r.open_picture().press(7, Nav::Down).press(1, Nav::Accept).run(&[&[], &[]]);
        let at = egui::pos2(300.0, 200.0);
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        r.frame(&[], vec![egui::Event::PointerMoved(at)]);
        r.frame(&[], vec![button(true)]);
        r.frame(&[], vec![button(false)]);
        // The window is 1920x1080 pixels at 1 point a pixel here: the picture
        // shows 1920 - 380 * 1.5 = 1350 points of it.
        let w = r.menu.picture.wipe;
        assert!((w - 300.0 / 1350.0).abs() < 0.01, "{w}");
    }

    /// Advanced: only the focused row says what it does.
    #[test]
    fn only_the_focused_row_shows_its_help() {
        let mut r = Rig::new("topskatr");
        r.open_advanced();
        assert!(r.shows("Softens jagged edges"));
        assert!(!r.shows("Blends frames to remove shimmer"));
        r.press(1, Nav::Down);
        assert!(r.shows("Blends frames to remove shimmer"));
        assert!(!r.shows("Softens jagged edges"));
        // The style line.
        assert!(r.shows("Arcade original") && r.shows(" · Top Skater only"), "{:?}", r.texts);
    }

    /// A row changed under a style makes it Custom, from that style; the
    /// button goes back to its values.
    #[test]
    fn changing_a_row_in_advanced_makes_the_style_custom() {
        let mut r = Rig::new("topskatr");
        r.open_picture().press(3, Nav::Down).press(1, Nav::Accept);
        assert_eq!(r.s.preset, Preset::Enhanced);
        // Advanced: with temporal on the rows are Resolution, Temporal
        // smoothing, Lighting, Glow, ...: right on Glow (the fourth).
        r.press(5, Nav::Down).press(1, Nav::Accept).run(&[&[], &[]]);
        assert_eq!(r.menu.page, Page::Advanced);
        r.press(3, Nav::Down).press(1, Nav::Right);
        assert_eq!((r.s.preset, r.s.bloom), (Preset::Custom, 25));
        assert_eq!(r.menu.picture.base, Some(Preset::Enhanced));
        r.run(&[&[]]);
        assert!(r.shows("Custom (from Smooth modern)"), "{:?}", r.texts);
        // Another change: still from Smooth modern.
        r.press(1, Nav::Up).press(1, Nav::Right);
        assert_eq!(r.menu.picture.base, Some(Preset::Enhanced));
        // Back to the style's values: the last widget.
        r.press(60, Nav::Down).press(1, Nav::Accept).run(&[&[]]);
        assert_eq!((r.s.preset, r.s.bloom), (Preset::Enhanced, 0));
        assert_eq!(r.s.shading, gpu::Shading::Vertex);
        assert!(r.shows("Smooth modern") && !r.shows("Custom (from"));
    }

    /// In-between frames are part of the style's values (a dot) but, as
    /// before, don't make it Custom; wide-screen HUD placement isn't the
    /// picture's. CRT is a row.
    #[test]
    fn the_other_rows() {
        let mut r = Rig::new("topskatr");
        r.s.preset = Preset::Custom;
        r.open_advanced();
        // Original rows: Edge smoothing, Temporal smoothing, Lighting,
        // Glow, In-between frames, Wide screens, Texture pack, CRT.
        r.press(4, Nav::Down).press(1, Nav::Right);
        assert!(r.s.smooth);
        r.press(1, Nav::Down).press(1, Nav::Right);
        assert!(!r.s.hud_edges);
        r.press(2, Nav::Down).press(1, Nav::Right);
        assert_eq!(r.s.crt, 30);
        // More: the depth buffer.
        r.press(2, Nav::Down).press(1, Nav::Accept);
        assert!(r.s.depth);
        // With the chip: no temporal smoothing or in-between frames.
        let mut r = Rig::new("topskatr");
        (r.s.preset, r.s.geo_chip) = (Preset::Custom, true);
        r.open_advanced();
        assert!(r.shows("Edge smoothing") && !r.shows("Temporal smoothing") && !r.shows("In-between frames"));
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
            let saves = [None, None, None, None, None];
            let view = View {
                playing: Some("A game"),
                game: None,
                saves: &saves,
                max_anisotropy: 16,
                temporal_supported: true,
                has_bezel: false,
                has_own: false,
                run_ahead_dropped: false,
                run_ahead_room: None,
                pack: None,
                materials: None,
                look: None,
                pad: Default::default(),
                picture: None,
                keeps: Keeps::default(),
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
                        if r.stroke.width == 4.0 && r.stroke.color.to_opaque().b() > 150 && r.stroke.color.r() < 100)
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
        assert_eq!(glow(&mut r, &[], vec![key.clone()]), 1, "the navigation keys: one");
        let up = egui::Event::Key {
            key: egui::Key::ArrowDown,
            physical_key: Some(egui::Key::ArrowDown),
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        assert_eq!(glow(&mut r, &[], vec![up]), 1, "letting the key go keeps it");
        assert_eq!(glow(&mut r, &[], Vec::new()), 1, "and it stays");
        let click = egui::Event::PointerButton {
            pos: egui::pos2(5.0, 5.0),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        };
        assert_eq!(glow(&mut r, &[], vec![click]), 0, "the mouse turns it off");
    }

    /// The menu over the launcher: no game running, one selected.
    #[test]
    fn from_the_launcher_the_menu_has_no_game_rows() {
        let game = romset::games::find("topskatr").unwrap();
        let mut menu = Menu { open: true, ..Default::default() };
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut s = GameSettings::default();
        let (mut fullscreen, mut for_all) = (false, false);
        let mut run = |menu: &mut Menu, has_own: bool, navs: &[Nav]| {
            let view = View {
                playing: None,
                game: Some(game),
                saves: &[],
                max_anisotropy: 16,
                temporal_supported: true,
                has_bezel: false,
                has_own,
                run_ahead_dropped: false,
                run_ahead_room: None,
                pack: None,
                materials: None,
                look: None,
                pad: Default::default(),
                picture: None,
                keeps: Keeps::default(),
            };
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0))),
                ..Default::default()
            };
            let mut actions = Vec::new();
            let mut out = ctx.run_ui(input, |ui| {
                let prefs = Prefs { fullscreen: &mut fullscreen, for_all: &mut for_all };
                actions = menu.ui(ui.ctx(), &mut s, prefs, &view, None, navs);
            });
            out.textures_delta.clear();
            let texts: Vec<String> = out
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                    _ => None,
                })
                .collect();
            (names(&actions), texts)
        };
        run(&mut menu, false, &[]);
        let (_, texts) = run(&mut menu, false, &[]);
        let has = |t: &[String], n: &str| t.iter().any(|x| x.contains(n));
        assert!(has(&texts, "Picture") && has(&texts, "Controls") && has(&texts, "Quit reModel"), "{texts:?}");
        assert!(!has(&texts, "Back to games") && !has(&texts, "Resume"), "{texts:?}");
        assert!(!has(&texts, "Use the settings for all games"));
        assert!(has(&texts, "TOP SKATER"), "the selected game's name heads it: {texts:?}");
        // A game with settings of its own can drop them.
        let (_, texts) = run(&mut menu, true, &[]);
        assert!(has(&texts, "Use the settings for all games"), "{texts:?}");
        // B closes it (as Resume does over a game).
        let (acts, _) = run(&mut menu, false, &[Nav::Back]);
        assert_eq!(acts, ["resume"]);
    }

    impl Rig {
        /// Into the Controls page, onto its first action.
        fn open_controls(&mut self) -> &mut Rig {
            self.press(8, Nav::Down).run(&[&[Nav::Accept], &[], &[]]);
            assert_eq!(self.menu.page, Page::Controls);
            self
        }

        /// The first action row's slot (topskatr, on the keyboard: Coin).
        fn first_slot(&self) -> romset::bindings::Slot {
            controls::rows(&self.b, controls::Device::Keys, crate::controls::PadStyle::Offset, false).0[0].slot
        }
    }

    #[test]
    fn controls_page_from_the_pad() {
        let mut r = Rig::new("topskatr");
        r.open_controls();
        assert!(r.actions.is_empty());
        assert!(r.shows("CONTROLS") && r.shows("Keyboard"), "{:?}", r.texts);
        // A on the first action waits for a binding.
        let first = r.first_slot();
        r.run(&[&[Nav::Accept]]);
        assert_eq!(r.menu.capture.map(|c| c.0), Some(first), "waiting for a binding");
        r.run(&[&[]]);
        assert!(r.shows("Press a key or button"), "{:?}", r.texts);
        // While it waits, the pad's input is the binding's, not the page's.
        r.run(&[&[Nav::Back]]);
        assert_eq!(r.menu.page, Page::Controls);
        assert!(r.menu.capture.is_some());
        // Waiting too long gives up.
        r.menu.capture_since = r.menu.capture_since.map(|(c, at)| (c, at - CAPTURE_WAIT));
        r.run(&[&[]]);
        assert!(r.menu.capture.is_none());
        // B goes back to the pause menu, on Controls; B there resumes.
        r.run(&[&[Nav::Back]]);
        assert_eq!(r.menu.page, Page::Pause);
        r.run(&[&[Nav::Back]]);
        assert_eq!(r.names(), ["resume"]);
    }

    #[test]
    fn x_adds_y_clears_and_y_again_restores() {
        let mut r = Rig::new("topskatr");
        r.open_controls();
        let first = r.first_slot();
        let before = r.b.get(first).cloned().unwrap();
        // X waits to add another.
        r.run(&[&[Nav::Saves]]);
        assert_eq!(r.menu.capture, Some((first, None)));
        r.menu.capture = None;
        r.run(&[&[]]);
        // Y takes the keyboard's bindings off, and says the game changed.
        r.run(&[&[Nav::Continue]]);
        assert!(r.names().contains(&"bindings"));
        assert!(r.b.get(first).unwrap().inputs().iter().all(|i| matches!(i, romset::bindings::Input::Pad(_))));
        assert!(r.shows("not bound here"));
        // Y on the empty row brings the game's own back.
        r.run(&[&[], &[Nav::Continue]]);
        assert_eq!(r.b.get(first), Some(&before));
    }

    #[test]
    fn reset_asks_twice_and_puts_everything_back() {
        let mut r = Rig::new("topskatr");
        r.open_controls();
        let first = r.first_slot();
        r.run(&[&[Nav::Continue]]);
        assert!(!r.b.is_default(first));
        r.s.tuning.stick.dead = 30;
        // Down to the last widget: the reset button.
        r.press(40, Nav::Down).run(&[&[Nav::Accept]]);
        assert!(r.menu.ctl.armed.is_some() && !r.b.is_default(first), "the first press only asks");
        r.run(&[&[], &[Nav::Accept]]);
        assert!(r.b.is_default(first));
        assert_eq!(r.s.tuning, Default::default());
    }

    #[test]
    fn a_doubled_binding_shows_on_the_page() {
        let mut r = Rig::new("topskatr");
        let jump = r.b.slot_named("jumpfront").unwrap();
        let tail = r.b.slot_named("jumptail").unwrap();
        r.b.assign(tail, None, romset::bindings::Input::Key("KeyZ".into()), romset::bindings::RESERVED_KEYS).unwrap();
        let _ = jump;
        r.open_controls();
        assert!(r.shows("also "), "{:?}", r.texts);
        assert!(r.shows("on two actions"));
    }

    #[test]
    fn the_shoulder_buttons_change_device() {
        let mut r = Rig::new("topskatr");
        r.open_controls();
        assert_eq!(r.menu.ctl.device, None);
        r.run(&[&[Nav::PrevGroup], &[]]);
        assert_eq!(r.menu.ctl.device, Some(controls::Device::Pad));
        assert!(r.shows("Jump front") || r.shows("Jump"));
        r.run(&[&[Nav::NextGroup], &[]]);
        assert_eq!(r.menu.ctl.device, Some(controls::Device::Keys));
        // A wheel game starts on its wheel.
        let mut r = Rig::new("daytona");
        r.open_controls();
        assert!(r.shows("Wheel"));
        assert_eq!(r.menu.device(controls::cabinet_art(Some(r.game)), &Default::default()), controls::Device::Cabinet);
    }

    #[test]
    fn tuning_rows_step_with_the_pad_and_follow_the_settings() {
        let mut r = Rig::new("daytona");
        r.open_controls();
        let dead = egui::Id::unique(("remodel-controls-tune", 0));
        r.menu.ctl.focus.focus_to(dead);
        let was = r.s.tuning.stick.dead;
        r.run(&[&[Nav::Right], &[Nav::Right], &[Nav::Left]]);
        assert_eq!(r.s.tuning.stick.dead, was + 1);
        // The curve is a choice: A moves on to the next.
        r.menu.ctl.focus.focus_to(egui::Id::unique(("remodel-controls-tune", 2)));
        r.run(&[&[], &[Nav::Accept]]);
        assert_eq!(r.s.tuning.stick.curve, crate::controls::Curve::Gentle);
        assert!(r.shows("Dead zone") && r.shows("Sensitivity") && r.shows("Curve"));
        // The pedals are the other block.
        r.menu.ctl.block = 1;
        r.run(&[&[], &[]]);
        assert!(r.shows("Pedals") && r.shows("Steering"));
    }
}
