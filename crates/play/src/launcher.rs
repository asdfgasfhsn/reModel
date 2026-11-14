//! The launcher: the player's start screen, drawn with egui over the
//! backdrop (`backdrop`).
//!
//! A row of game cards (each game's poster and title) slides along the
//! bottom, grouped by board with the recently played first; the selected
//! card's game shows above it: its live attract mode (`preview`, once the
//! card has been selected for `SETTLE`), title, year, board and controls,
//! and what it can do (play, continue where it was left, settings). The
//! settings slide out from the right over the preview, which shows what
//! they change. The selected game's bezel art fades in behind everything.
//!
//! Keyboard: arrows choose (up and down step through the game's buttons),
//! Enter plays, Tab the settings, Page Up / Page Down jump between the
//! groups, Esc goes back. Gamepad (`pad::PadNav`): d-pad or left stick, A, Y,
//! the shoulder buttons, B. Mouse: click a card to select it, again (or a
//! button) to play; the wheel scrolls the row.
//!
//! Everything is laid out in units of the screen (`Layout`): a 1600x900
//! design scaled to fit, on top of the display's own scaling, so it reads
//! the same at 1080p, 1440p ultrawide and 4K.

use crate::hints;
use crate::settings::{GameSettings, Preset, Source, KEYS};
use egui::{pos2, vec2, Align2, Color32, FontId, Id, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

/// The logo's colours.
pub const CYAN: Color32 = Color32::from_rgb(47, 230, 239);
pub const RED: Color32 = Color32::from_rgb(255, 36, 54);
pub const FG: Color32 = Color32::from_rgb(242, 244, 248);
pub const DIM: Color32 = Color32::from_rgb(139, 147, 163);
pub const PANEL: Color32 = Color32::from_rgb(7, 11, 18);
pub const LINE: Color32 = Color32::from_rgb(22, 58, 68);
const KEY_BG: Color32 = Color32::from_rgb(13, 20, 29);
const KEY_LINE: Color32 = Color32::from_rgb(44, 58, 72);
const AMBER: Color32 = Color32::from_rgb(255, 212, 121);

/// How long a card stays selected before its preview starts.
pub const SETTLE: Duration = Duration::from_millis(500);
/// Most recently played games shown first.
const RECENT: usize = 6;
/// The preview's fade in and out.
pub const FADE_IN: Duration = Duration::from_millis(450);

/// What the player asked for, from any device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Left,
    Right,
    Up,
    Down,
    Accept,
    /// The pad's Start: plays here, resumes in the menu.
    Start,
    Back,
    Settings,
    PrevGroup,
    NextGroup,
}

pub enum Action {
    Play {
        set: String,
        resume: bool,
    },
    /// Look for games in this ROM folder.
    Rescan(String),
    /// Drop the game's own settings.
    ResetGame(String),
    Quit,
}

/// Which page slides out over the preview.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Closed,
    Settings,
    /// The game's controls, to remap (`menu::Menu::controls_page`).
    Controls,
}

/// A run of cards under one heading.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub name: String,
    pub start: usize,
    pub len: usize,
}

/// Where everything goes, in points.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    /// Points per design unit.
    pub u: f32,
    pub screen: Rect,
    /// The top bar (logo, groups).
    pub top: Rect,
    /// The preview, the window's shape.
    pub hero: Rect,
    /// Title, details and buttons, right of the preview.
    pub info: Rect,
    /// Centre line of the cards, and a selected card's size.
    pub cards_y: f32,
    pub card: Vec2,
    /// The hints along the bottom (centre line).
    pub hints_y: f32,
}

/// The launcher's layout for a screen `screen` points in size: a 1600x900
/// design scaled by `u` to fit, centred.
pub fn layout(screen: Rect) -> Layout {
    let u = (screen.width() / 1600.0).min(screen.height() / 900.0).max(0.2);
    let y0 = screen.top() + (screen.height() - 900.0 * u) / 2.0;
    let margin = 56.0 * u;
    let top =
        Rect::from_min_max(pos2(screen.left() + margin, y0 + 24.0 * u), pos2(screen.right() - margin, y0 + 80.0 * u));
    // The preview has the window's shape (what the game will look like),
    // leaving room for the details beside it.
    let aspect = (screen.width() / screen.height().max(1.0)).clamp(crate::ASPECTS.0, crate::ASPECTS.1);
    let (slot_top, slot_h) = (y0 + 104.0 * u, 424.0 * u);
    let room = screen.width() - 2.0 * margin - 40.0 * u - 440.0 * u;
    let w = (slot_h * aspect).min(room.max(200.0 * u));
    let h = w / aspect;
    let hero = Rect::from_min_size(pos2(screen.left() + margin, slot_top + (slot_h - h) / 2.0), vec2(w, h));
    let info =
        Rect::from_min_max(pos2(hero.right() + 40.0 * u, slot_top), pos2(screen.right() - margin, slot_top + slot_h));
    Layout {
        u,
        screen,
        top,
        hero,
        info,
        cards_y: y0 + 704.0 * u,
        card: vec2(160.0, 240.0) * u,
        hints_y: y0 + 878.0 * u,
    }
}

/// A game's card: what it shows besides the poster.
pub struct Card {
    pub game: &'static romset::Game,
    pub group: usize,
}

/// What the launcher shows of the selected game besides what it keeps.
pub struct View<'a> {
    /// The live preview, once it has a frame.
    pub preview: Option<PreviewView>,
    /// Making its preview state: how far (0..1).
    pub progress: Option<f32>,
    /// Why it can't be previewed.
    pub no_preview: Option<&'a str>,
    /// When its resume state was saved, if it has one.
    pub resume: Option<SystemTime>,
    /// Sets in the ROM folder reModel doesn't support yet.
    pub unsupported: usize,
    /// The frames of run-ahead this computer has time for, once measured
    /// (`hints::FrameCost::run_ahead_room`), and the selected game's
    /// run-ahead was switched off as too slow.
    pub run_ahead_room: Option<u32>,
    pub run_ahead_dropped: bool,
}

/// The preview as a texture, and where the game's picture is in it.
#[derive(Clone, Copy, Debug)]
pub struct PreviewView {
    pub texture: egui::TextureId,
    /// Opacity (fading in or out).
    pub alpha: f32,
    /// The picture in the texture, as fractions (x, y, width, height): a
    /// 4:3 picture is narrower than the texture, the bezel beside it.
    pub picture: [f32; 4],
    /// The bezel shows beside the picture.
    pub bezel: bool,
}

/// The settings panel's rows.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Row {
    Scope,
    Choice(&'static str, &'static str, &'static str, &'static [(&'static str, &'static str)]),
    Db(&'static str, &'static str, f32, f32),
    Controls,
    Reset,
}

/// The run-ahead row's choices (label, value), as many as the game offers.
static RUN_AHEAD_OPTIONS: [(&str, &str); 4] = [("Off", "0"), ("1", "1"), ("2", "2"), ("3", "3")];

/// What the settings list shows of a game besides its settings.
struct Shown<'a> {
    game: &'static romset::Game,
    values: &'a GameSettings,
    has_own: bool,
    max_anisotropy: u8,
    temporal_supported: bool,
}

/// The rows that apply to the game as set, as the in-game menu lists them
/// (`menu::settings_page`): the preset first among the picture rows, the
/// rest of the picture only under Custom, and rows with nothing to do (the
/// chip's, a run-ahead the game has no lag for) left out.
fn shown_rows(c: &Shown) -> Vec<Row> {
    let v = c.values;
    let temporal = v.temporal_on() && c.temporal_supported;
    let custom = v.preset == Preset::Custom;
    let max = hints::run_ahead_max(Some(c.game));
    rows(c.has_own, c.max_anisotropy)
        .into_iter()
        .filter_map(|row| {
            let Row::Choice(key, label, hint, options) = row else { return Some(row) };
            let show = match key {
                "hud_edges" => v.wide,
                "bezel" => !v.wide && crate::bezel::art_set(c.game).is_some(),
                "aa" => custom && !temporal,
                "temporal" => custom && !v.geo_chip && c.temporal_supported,
                "stipple" => custom && temporal,
                "depth" | "shading" | "pack" => custom,
                "bloom" => custom && c.temporal_supported,
                "shading_fine" | "shading_balance" => custom && v.light_on(),
                "materials" => custom && v.pack && !v.geo_chip,
                "materials_relief" => custom && v.materials_on() && !v.geo_chip,
                "smooth" => !v.geo_chip,
                "run_ahead" => hints::run_ahead_shown(Some(c.game), v.run_ahead),
                "tex_filter" => v.pack,
                _ => true,
            };
            let options = if key == "run_ahead" { &RUN_AHEAD_OPTIONS[..=(max as usize).min(3)] } else { options };
            show.then_some(Row::Choice(key, label, hint, options))
        })
        .collect()
}

/// Change setting `key`: a preset sets the rows it describes.
fn change(values: &mut GameSettings, key: &str, value: &str, game: &'static romset::Game) {
    match (key, Preset::parse(value)) {
        ("preset", Some(p)) => values.apply_preset(p, Some(game)),
        _ => {
            values.set(key, value);
        }
    }
}

/// `values` with a preset whose rows no longer all match made Custom.
fn settle(values: &mut GameSettings, game: &'static romset::Game) {
    *values = std::mem::take(values).settled(Some(game));
}

/// The settings panel's rows: key, label, hint, and choices (label, value);
/// texture filters up to `max_anisotropy` (`gpu::max_anisotropy`).
fn rows(has_own: bool, max_anisotropy: u8) -> Vec<Row> {
    let mut rows = vec![
        Row::Scope,
        Row::Choice("wide", "Aspect", "", &[("Fit the window", "true"), ("4:3", "false")]),
        Row::Choice("bezel", "Bezel", "beside a 4:3 picture in a wider window", &[("On", "true"), ("Off", "false")]),
        Row::Choice("hud_edges", "HUD", "in wide views", &[("At the edges", "true"), ("Centred", "false")]),
        Row::Choice(
            "preset",
            "Preset",
            "",
            &[("Original", "original"), ("Sharp", "sharp"), ("Enhanced", "enhanced"), ("Custom", "custom")],
        ),
        Row::Choice(
            "aa",
            "Anti-aliasing",
            "smooths polygon edges; 8x needs a GPU that has it, else 4x",
            &[("Off", "0"), ("4x", "4"), ("8x", "8")],
        ),
        Row::Choice(
            "temporal",
            "Temporal AA",
            "edges smoothed over frames; a % draws smaller, then upscales",
            &[("Off", "off"), ("TAA", "taa"), ("75%", "75"), ("67%", "67"), ("50%", "50")],
        ),
        Row::Choice(
            "stipple",
            "Checkerboard see-through",
            "with temporal: the games' screen-door translucency as an even blend, or the pattern kept",
            &[("Blend", "blend"), ("Keep", "keep")],
        ),
        Row::Choice(
            "depth",
            "Depth buffer",
            "polygons that cross or are sorted wrong come out right; off draws in the game's order",
            &[("Off", "false"), ("On", "true")],
        ),
        Row::Choice(
            "shading",
            "Shading",
            "flat is the game's own; vertex blends brightness across curved surfaces",
            &[("Flat", "flat"), ("Vertex", "vertex"), ("Pixel", "pixel")],
        ),
        Row::Choice(
            "shading_fine",
            "Brightness levels",
            "with shading: blended, so smooth surfaces don't band; or the game's 64",
            &[("Game's", "false"), ("Blended", "true")],
        ),
        Row::Choice(
            "shading_balance",
            "Brightness balance",
            "with shading: each smooth surface as bright as the game's; off, bumpy ones come out brighter",
            &[("On", "true"), ("Off", "false")],
        ),
        Row::Choice(
            "pack",
            "Texture pack",
            "replacement textures from the game's texpack folder",
            &[("Off", "false"), ("On", "true")],
        ),
        Row::Choice(
            "materials",
            "Material maps",
            "the pack's normal maps: bumps and grooves lit by the game's own lights",
            &[("Off", "false"), ("On", "true")],
        ),
        Row::Choice(
            "materials_relief",
            "Material relief",
            "how strongly maps shade (2 by default); faithful keeps the game's own lighting sum, subtler",
            &[("1", "1"), ("2", "2"), ("4", "4"), ("8", "8"), ("Faithful", "off")],
        ),
        Row::Choice(
            "bloom",
            "Bloom",
            "bright parts of the 3D picture glow; the HUD stays sharp",
            &[("Off", "off"), ("Light", "25"), ("Medium", "50"), ("Strong", "100")],
        ),
        Row::Choice(
            "exact",
            "Speed",
            "exact is 57.5 frames a second; else one a 60 or 120 Hz refresh, 4.3% fast",
            &[("One frame a refresh", "false"), ("Exact arcade", "true")],
        ),
        Row::Choice(
            "smooth",
            "Smooth motion",
            "in-between frames on displays faster than 60 Hz, a frame later",
            &[("Off", "false"), ("On", "true")],
        ),
        Row::Choice(
            "run_ahead",
            "Run-ahead",
            "frames shown ahead of the game: less input lag for that much more work",
            &RUN_AHEAD_OPTIONS,
        ),
        Row::Db("sfx_db", "Effects", -20.0, 20.0),
        Row::Db("music_db", "Music", -30.0, 10.0),
        Row::Choice(
            "geo",
            "Geometrizer",
            "the chip runs the game's own program (the reference); applies on starting",
            &[("Native", "native"), ("The game's chip", "chip")],
        ),
        Row::Choice(
            "tex_filter",
            "Texture filtering",
            "of pack images: less shimmer far off",
            crate::settings::tex_filters(max_anisotropy),
        ),
        Row::Controls,
    ];
    if has_own {
        rows.push(Row::Reset);
    }
    rows
}

pub struct Launcher {
    pub open: bool,
    cards: Vec<Card>,
    groups: Vec<Group>,
    sel: usize,
    /// When the selection last changed.
    pub selected_at: Instant,
    /// The row's scroll (points, where it starts), where it's going, and each
    /// card's scale, animated towards their targets.
    scroll: Option<f32>,
    scroll_target: f32,
    scales: Vec<f32>,
    last_frame: Option<Instant>,
    /// The highlighted button beside the preview.
    button: usize,
    pub panel: Panel,
    /// The panel's slide, 0 (closed) to 1.
    panel_t: f32,
    row: usize,
    /// Mouse wheel travel not yet turned into a step.
    wheel: f32,
    /// The last input came from a gamepad (the hints say its buttons).
    pub pad_used: bool,
    posters: HashMap<String, egui::TextureHandle>,
    /// Bezel art (a few, by set), the one fading in, and since when.
    bezels: Vec<(String, egui::TextureHandle)>,
    backdrop: Option<(String, Instant)>,
    backdrop_before: Option<String>,
    art: Option<crate::art::Loader>,
    logo: Option<egui::TextureHandle>,
    /// The ROM folder being edited (no games found).
    pub rom_text: String,
    /// Anisotropic filtering the GPU has (`gpu::max_anisotropy`): the
    /// texture filters offered.
    pub max_anisotropy: u8,
    /// The GPU can do temporal anti-aliasing (`gpu::temporal_supported`).
    pub temporal_supported: bool,
}

/// The hint of the run-ahead row: `hint`, what this computer has time for
/// (amber when `chosen` is more), and whether it was switched off.
fn run_ahead_hint(hint: &str, chosen: u32, game: &romset::Game, view: &View) -> (String, Color32) {
    let max = hints::run_ahead_max(Some(game));
    let mut text = hint.to_string();
    if let Some(note) = hints::run_ahead_note(view.run_ahead_room, max, chosen) {
        text += &format!(". {note}");
    }
    if view.run_ahead_dropped {
        text += ". Switched off: too slow. Choosing it again tries again.";
    }
    let warn = hints::run_ahead_warns(view.run_ahead_room, max, chosen) || view.run_ahead_dropped;
    (text, if warn { AMBER } else { DIM })
}

/// Why setting `key` doesn't apply as set (said in place of its hint):
/// temporal anti-aliasing needs the native geometrizer and a GPU that has
/// it, and turns multisampling off; per-pixel shading needs the native
/// geometrizer (the chip draws it as vertex), and a crease angle of 0
/// smooths nothing; material maps need the texture pack and the native
/// geometrizer.
fn overridden(key: &str, v: &GameSettings, temporal_supported: bool) -> Option<&'static str> {
    let temporal = v.temporal_on() && temporal_supported;
    match key {
        "aa" if temporal && v.aa > 0 => Some("off while temporal anti-aliasing is on"),
        "temporal" if v.temporal > 0 && !temporal_supported => Some("off: this GPU can't"),
        "temporal" if v.temporal > 0 && v.geo_chip => Some("off: it needs the native geometrizer"),
        "stipple" if v.stipple_keep && !temporal => Some("only with temporal anti-aliasing on"),
        "shading" if v.shading != gpu::Shading::Flat && v.crease == 0 => Some("crease 0: nothing is smoothed"),
        "shading" if v.shading == gpu::Shading::Pixel && v.geo_chip => {
            Some("drawn as vertex with the chip: per pixel needs the native geometrizer")
        }
        "shading_fine" if v.shading_fine && !v.light_on() => Some("only with shading on"),
        "shading_balance" if !v.shading_balance && !v.light_on() => Some("only with shading on"),
        "materials" if v.materials && !v.pack => Some("no effect without the texture pack on"),
        "materials" if v.materials && v.geo_chip => Some("no effect with the chip: needs the native geometrizer"),
        "materials_relief" if v.materials_relief != gpu::Relief::default() && !v.materials_on() => {
            Some("only with material maps on")
        }
        _ => None,
    }
}

impl Default for Launcher {
    fn default() -> Self {
        Launcher {
            open: false,
            cards: Vec::new(),
            groups: Vec::new(),
            sel: 0,
            selected_at: Instant::now(),
            scroll: None,
            scroll_target: 0.0,
            scales: Vec::new(),
            last_frame: None,
            button: 0,
            panel: Panel::Closed,
            panel_t: 0.0,
            row: 0,
            wheel: 0.0,
            pad_used: false,
            posters: HashMap::new(),
            bezels: Vec::new(),
            backdrop: None,
            backdrop_before: None,
            art: None,
            logo: None,
            rom_text: String::new(),
            max_anisotropy: 16,
            temporal_supported: true,
        }
    }
}

/// "2 h ago" and so on.
pub fn age(t: SystemTime) -> String {
    let s = t.elapsed().unwrap_or_default().as_secs();
    match s {
        0..60 => "just now".into(),
        60..120 => "a minute ago".into(),
        120..3600 => format!("{} minutes ago", s / 60),
        3600..7200 => "an hour ago".into(),
        7200..86400 => format!("{} hours ago", s / 3600),
        86400..172800 => "yesterday".into(),
        _ => format!("{} days ago", s / 86400),
    }
}

/// Smoothing towards a target: the share of the way to go in `dt`.
fn ease(dt: f32, rate: f32) -> f32 {
    1.0 - (-dt * rate).exp()
}

impl Launcher {
    /// Set the games (playable sets in the ROM folder) and the recently
    /// played (most recent first), keeping the selection if it's still
    /// there, else selecting the most recent.
    pub fn set_games(&mut self, games: &[&'static romset::Game], recent: &[String]) {
        let selected = self.selected().map(|g| g.id);
        let mut cards = Vec::new();
        let mut groups = Vec::new();
        let recents: Vec<&'static romset::Game> =
            recent.iter().filter_map(|r| games.iter().find(|g| g.id == r).copied()).take(RECENT).collect();
        if !recents.is_empty() {
            groups.push(Group { name: "Recently played".into(), start: 0, len: recents.len() });
            cards.extend(recents.iter().map(|&game| Card { game, group: 0 }));
        }
        let mut boards: Vec<&str> = games.iter().map(|g| g.board.name()).collect();
        boards.sort();
        boards.dedup();
        for board in boards {
            let mut list: Vec<&'static romset::Game> =
                games.iter().copied().filter(|g| g.board.name() == board).collect();
            list.sort_by_key(|g| (g.title, g.id));
            groups.push(Group { name: board.to_string(), start: cards.len(), len: list.len() });
            let group = groups.len() - 1;
            cards.extend(list.into_iter().map(|game| Card { game, group }));
        }
        self.cards = cards;
        self.groups = groups;
        self.scales = self.cards.iter().map(|_| 0.8).collect();
        self.scroll = None;
        let keep = selected.and_then(|id| self.cards.iter().position(|c| c.game.id == id));
        self.select(keep.unwrap_or(0));
        let all: Vec<&'static romset::Game> = games.to_vec();
        if !all.is_empty() {
            self.art = Some(crate::art::Loader::start(all));
        }
    }

    /// Select the card for `set` (the first, if it's in more than one group).
    pub fn select_set(&mut self, set: &str) {
        if let Some(i) = self.cards.iter().position(|c| c.game.id == set) {
            self.select(i);
        }
    }

    fn select(&mut self, i: usize) {
        let i = i.min(self.cards.len().saturating_sub(1));
        if i != self.sel || self.cards.is_empty() {
            self.selected_at = Instant::now();
        }
        let before = self.selected().map(|g| g.id);
        self.sel = i;
        if self.selected().map(|g| g.id) != before {
            self.button = 0;
            if self.panel == Panel::Controls {
                self.panel = Panel::Settings;
            }
        }
    }

    pub fn selected(&self) -> Option<&'static romset::Game> {
        self.cards.get(self.sel).map(|c| c.game)
    }

    #[cfg(test)]
    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    #[cfg(test)]
    pub fn selection(&self) -> usize {
        self.sel
    }

    /// The buttons beside the preview: (label, play, resume).
    fn buttons(resume: bool) -> &'static [(&'static str, Option<bool>)] {
        if resume {
            &[("Continue", Some(true)), ("New game", Some(false)), ("Settings", None)]
        } else {
            &[("Play", Some(false)), ("Settings", None)]
        }
    }

    /// Act on `nav`: move, open or close the panel, play. `resume`: the
    /// selected game has a resume state.
    pub fn nav(
        &mut self,
        nav: Nav,
        resume: bool,
        values: &mut GameSettings,
        for_all: &mut bool,
        has_own: bool,
        actions: &mut Vec<Action>,
    ) {
        let Some(game) = self.selected() else {
            return;
        };
        let nav = if nav == Nav::Start { Nav::Accept } else { nav };
        match self.panel {
            Panel::Controls => {
                if nav == Nav::Back {
                    self.panel = Panel::Settings;
                }
            }
            Panel::Settings => {
                let rows = shown_rows(&Shown {
                    game,
                    values,
                    has_own,
                    max_anisotropy: self.max_anisotropy,
                    temporal_supported: self.temporal_supported,
                });
                self.row = self.row.min(rows.len() - 1);
                match nav {
                    Nav::Back | Nav::Settings => self.panel = Panel::Closed,
                    Nav::Up => self.row = self.row.saturating_sub(1),
                    Nav::Down => self.row = (self.row + 1).min(rows.len() - 1),
                    Nav::Left | Nav::Right | Nav::Accept => {
                        let step = match nav {
                            Nav::Left => -1,
                            Nav::Right => 1,
                            _ => 0,
                        };
                        match rows[self.row] {
                            Row::Scope => *for_all = if step == 0 { !*for_all } else { step > 0 },
                            Row::Choice(key, _, _, options) => {
                                // A value off the list is one more choice
                                // after the others: staying on it keeps it.
                                let (labels, at) = choice_labels(options, &values.get(key));
                                let (at, n) = (at as i32, labels.len() as i32);
                                let to = if step == 0 { (at + 1) % n } else { (at + step).clamp(0, n - 1) };
                                if let Some(o) = options.get(to as usize) {
                                    change(values, key, o.1, game);
                                }
                            }
                            Row::Db(key, _, min, max) => {
                                let v: f32 = values.get(key).parse().unwrap_or(0.0);
                                values.set(key, &(v + step as f32).clamp(min, max).to_string());
                            }
                            Row::Controls if step == 0 => self.panel = Panel::Controls,
                            Row::Reset if step == 0 => {
                                actions.push(Action::ResetGame(game.id.to_string()));
                                self.row = self.row.saturating_sub(1);
                            }
                            _ => {}
                        }
                        settle(values, game);
                    }
                    Nav::PrevGroup | Nav::NextGroup | Nav::Start => {}
                }
            }
            Panel::Closed => {
                let buttons = Self::buttons(resume);
                self.button = self.button.min(buttons.len() - 1);
                match nav {
                    Nav::Left => self.select(self.sel.saturating_sub(1)),
                    Nav::Right => self.select(self.sel + 1),
                    Nav::Up => self.button = self.button.saturating_sub(1),
                    Nav::Down => self.button = (self.button + 1).min(buttons.len() - 1),
                    Nav::Settings => self.open_settings(),
                    Nav::Accept => match buttons[self.button].1 {
                        Some(resume) => actions.push(Action::Play { set: game.id.to_string(), resume }),
                        None => self.open_settings(),
                    },
                    Nav::PrevGroup | Nav::NextGroup => {
                        let g = self.cards[self.sel].group;
                        let to = if nav == Nav::PrevGroup {
                            // To the start of this group, or the one before.
                            if self.sel > self.groups[g].start || g == 0 {
                                self.groups[g].start
                            } else {
                                self.groups[g - 1].start
                            }
                        } else {
                            self.groups.get(g + 1).map_or(self.cards.len() - 1, |n| n.start)
                        };
                        self.select(to);
                    }
                    Nav::Back | Nav::Start => {}
                }
            }
        }
    }

    fn open_settings(&mut self) {
        self.panel = Panel::Settings;
        self.row = 1;
    }

    /// The keyboard's `Nav`s this frame (taken from egui's input, so the
    /// controls page's widgets don't see them twice).
    pub fn keys(ctx: &egui::Context) -> Vec<Nav> {
        use egui::{Key, Modifiers};
        let map = [
            (Key::ArrowLeft, Nav::Left),
            (Key::ArrowRight, Nav::Right),
            (Key::ArrowUp, Nav::Up),
            (Key::ArrowDown, Nav::Down),
            (Key::Enter, Nav::Accept),
            (Key::Space, Nav::Accept),
            (Key::Escape, Nav::Back),
            (Key::Backspace, Nav::Back),
            (Key::Tab, Nav::Settings),
            (Key::PageUp, Nav::PrevGroup),
            (Key::PageDown, Nav::NextGroup),
        ];
        let mut out = Vec::new();
        ctx.input_mut(|i| {
            for (key, nav) in map {
                while i.consume_key(Modifiers::NONE, key) {
                    out.push(nav);
                }
            }
        });
        out
    }

    /// Upload decoded art, and ask for the selected game's bezel.
    fn art(&mut self, ctx: &egui::Context) {
        let Some(loader) = &self.art else { return };
        for (set, kind, image) in loader.take() {
            let tex = ctx.load_texture(format!("{kind:?} {set}"), image, egui::TextureOptions::LINEAR);
            match kind {
                crate::art::Kind::Poster => {
                    self.posters.insert(set, tex);
                }
                crate::art::Kind::Bezel => {
                    self.bezels.retain(|b| b.0 != set);
                    self.bezels.push((set, tex));
                    // The art is full HD: keep the last few.
                    if self.bezels.len() > 3 {
                        drop(self.bezels.remove(0));
                    }
                }
            }
        }
        if let Some(game) = self.selected() {
            let shown = self.backdrop.as_ref().map(|b| b.0.as_str());
            if let Some(set) = crate::bezel::art_set(game) {
                if shown != Some(set) {
                    if self.bezel_tex(set).is_none() {
                        loader.bezel(game);
                    }
                    self.backdrop_before = self.backdrop.take().map(|b| b.0);
                    self.backdrop = Some((set.to_string(), Instant::now()));
                }
            }
        }
    }

    /// The bezel art of set `set` (a set with art: `bezel::art_set`).
    fn bezel_tex(&self, set: &str) -> Option<&egui::TextureHandle> {
        self.bezels.iter().find(|b| b.0 == set).map(|b| &b.1)
    }

    /// Draw the launcher and act on the input. `navs` are the gamepad's;
    /// the keyboard's and mouse's are read here. `values` are the selected
    /// game's settings, edited in place (`sources` says where each came
    /// from); `for_all` is where edits go.
    #[allow(clippy::too_many_arguments)]
    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        mut navs: Vec<Nav>,
        view: &View,
        values: &mut GameSettings,
        sources: &[Source; KEYS.len()],
        for_all: &mut bool,
        has_own: bool,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        let now = Instant::now();
        let dt = self.last_frame.map_or(0.0, |t| (now - t).as_secs_f32()).min(0.1);
        self.last_frame = Some(now);
        self.art(ctx);
        let l = layout(ctx.content_rect());
        if self.logo.is_none() {
            if let Ok(image) = png::decode(crate::splash::LOGO) {
                let image = egui::ColorImage::from_rgba_unmultiplied([image.width, image.height], &image.rgba);
                self.logo = Some(ctx.load_texture("launcher logo", image, egui::TextureOptions::LINEAR));
            }
        }
        let resume = view.resume.is_some();
        if self.panel != Panel::Controls {
            if !navs.is_empty() {
                self.pad_used = true;
            }
            let keys = Self::keys(ctx);
            if !keys.is_empty() {
                self.pad_used = false;
            }
            navs.extend(keys);
        }
        if self.cards.is_empty() {
            self.empty(ctx, &l, view, &mut actions);
            return actions;
        }
        for nav in navs {
            self.nav(nav, resume, values, for_all, has_own, &mut actions);
        }

        // Animation targets.
        for (i, s) in self.scales.iter_mut().enumerate() {
            let target = if i == self.sel { 1.0 } else { 0.78 };
            *s += (target - *s) * ease(dt, 16.0);
        }
        let open = if self.panel == Panel::Closed { 0.0 } else { 1.0 };
        self.panel_t += (open - self.panel_t) * ease(dt, 18.0);
        if (self.panel_t - open).abs() < 0.002 {
            self.panel_t = open;
        }

        let area = egui::Area::new(Id::unique("launcher")).fixed_pos(l.screen.min).order(egui::Order::Background);
        area.show(ctx, |ui| {
            ui.set_min_size(l.screen.size());
            let painter = ui.painter().clone();
            self.paint_backdrop(&painter, &l, now);
            self.paint_top(ui, &painter, &l, &mut actions);
            self.paint_hero(&painter, &l, view, now);
            self.paint_info(ui, &painter, &l, view, &mut actions);
            self.paint_cards(ui, &painter, &l, dt, &mut actions);
            self.paint_hints(&painter, &l);
        });
        if self.panel_t > 0.0 {
            self.paint_panel(ctx, &l, view, values, sources, for_all, has_own, &mut actions);
        }
        actions
    }

    /// Whether anything is still moving (else redraws needn't animate).
    pub fn animating(&self) -> bool {
        let settled =
            self.scales.iter().enumerate().all(|(i, s)| (s - if i == self.sel { 1.0 } else { 0.78 }).abs() < 0.002);
        let scrolling = self.scroll.is_some_and(|s| s != self.scroll_target);
        !settled || scrolling || (self.panel_t > 0.0 && self.panel_t < 1.0)
    }

    fn paint_backdrop(&self, p: &Painter, l: &Layout, now: Instant) {
        // The selected game's bezel, faded in over the grid, the one before
        // fading out.
        let fade = self.backdrop.as_ref().map_or(1.0, |b| ((now - b.1).as_secs_f32() / 0.6).min(1.0));
        let cover = |tex: &egui::TextureHandle, alpha: f32| {
            let [w, h] = tex.size();
            let s = (l.screen.width() / w as f32).max(l.screen.height() / h as f32);
            let r = Rect::from_center_size(l.screen.center(), vec2(w as f32, h as f32) * s);
            p.image(
                tex.id(),
                r,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::from_white_alpha((alpha * 255.0) as u8),
            );
        };
        const LEVEL: f32 = 0.30;
        if fade < 1.0 {
            if let Some(tex) = self.backdrop_before.as_deref().and_then(|s| self.bezel_tex(s)) {
                cover(tex, LEVEL * (1.0 - fade));
            }
        }
        if let Some(tex) = self.backdrop.as_ref().and_then(|b| self.bezel_tex(&b.0)) {
            cover(tex, LEVEL * fade);
        }
        // Darker behind the cards and the details, for reading.
        let s = l.screen;
        gradient(
            p,
            Rect::from_min_max(pos2(s.left(), l.cards_y - l.card.y), s.max),
            Color32::TRANSPARENT,
            Color32::from_black_alpha(200),
        );
        gradient(
            p,
            Rect::from_min_max(s.min, pos2(s.right(), l.top.bottom() + 20.0 * l.u)),
            Color32::from_black_alpha(150),
            Color32::TRANSPARENT,
        );
    }

    fn paint_top(&mut self, ui: &mut egui::Ui, p: &Painter, l: &Layout, actions: &mut Vec<Action>) {
        let u = l.u;
        let mut x = l.top.left();
        if let Some(logo) = &self.logo {
            let [w, h] = logo.size();
            let lh = l.top.height() * 0.92;
            let r = Rect::from_min_size(pos2(x, l.top.center().y - lh / 2.0), vec2(lh * w as f32 / h as f32, lh));
            p.image(logo.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            x = r.right() + 40.0 * u;
        }
        // The groups, the current one underlined; click to jump.
        let current = self.cards.get(self.sel).map_or(0, |c| c.group);
        for (i, g) in self.groups.clone().iter().enumerate() {
            let font = FontId::proportional(17.0 * u);
            let galley = p.layout_no_wrap(g.name.to_uppercase(), font, FG);
            let r = Rect::from_min_size(pos2(x, l.top.center().y - galley.size().y / 2.0), galley.size());
            let hit = ui.interact(r.expand(8.0 * u), Id::unique(("group", i)), Sense::click());
            let on = i == current;
            let color = if on {
                FG
            } else if hit.hovered() {
                Color32::from_rgb(200, 206, 216)
            } else {
                DIM
            };
            p.galley(r.min, galley.clone(), color);
            if on {
                p.galley(r.min + vec2(0.6 * u, 0.0), galley, color);
                p.rect_filled(
                    Rect::from_min_size(pos2(r.left(), r.bottom() + 6.0 * u), vec2(r.width(), 3.0 * u)),
                    1.5 * u,
                    CYAN,
                );
            }
            if hit.clicked() {
                self.select(g.start);
            }
            x = r.right() + 32.0 * u;
        }
        // Quit, top right.
        let font = FontId::proportional(16.0 * u);
        let galley = p.layout_no_wrap("Quit".into(), font, DIM);
        let r = Rect::from_min_size(
            pos2(l.top.right() - galley.size().x, l.top.center().y - galley.size().y / 2.0),
            galley.size(),
        );
        let hit = ui.interact(r.expand(8.0 * u), Id::unique("quit"), Sense::click());
        p.galley(r.min, galley, if hit.hovered() { FG } else { DIM });
        if hit.clicked() {
            actions.push(Action::Quit);
        }
    }

    fn paint_hero(&self, p: &Painter, l: &Layout, view: &View, now: Instant) {
        let u = l.u;
        let r = l.hero;
        let radius = 12.0 * u;
        // Glow and frame.
        p.add(
            egui::epaint::RectShape::filled(r, radius, Color32::from_rgba_unmultiplied(47, 230, 239, 40))
                .with_blur_width(28.0 * u),
        );
        p.rect_filled(r, radius, Color32::BLACK);
        let Some(game) = self.selected() else { return };
        let live = view.preview.filter(|v| v.alpha > 0.0);
        // The poster until the preview is live (and fading into it).
        if live.is_none_or(|v| v.alpha < 1.0) {
            if let Some(tex) = self.posters.get(game.id) {
                let a = 1.0 - live.map_or(0.0, |v| v.alpha);
                let [w, h] = tex.size();
                let (w, h) = (w as f32, h as f32);
                // Cover the frame, dimmed, with the whole poster in the middle.
                let s = (r.width() / w).max(r.height() / h);
                let crop = vec2(r.width() / (w * s), r.height() / (h * s));
                let uv = Rect::from_center_size(pos2(0.5, 0.5), crop);
                p.add(
                    egui::epaint::RectShape::filled(r, radius, Color32::from_white_alpha((a * 60.0) as u8))
                        .with_texture(tex.id(), uv),
                );
                let fit = Rect::from_center_size(r.center(), vec2(w, h) * (r.height() / h));
                p.image(
                    tex.id(),
                    fit,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::from_white_alpha((a * 255.0) as u8),
                );
            }
        }
        if let Some(v) = live {
            let a = (v.alpha * 255.0) as u8;
            let [px, py, pw, ph] = v.picture;
            let pic = Rect::from_min_size(
                pos2(r.left() + px * r.width(), r.top() + py * r.height()),
                vec2(pw * r.width(), ph * r.height()),
            );
            p.add(
                egui::epaint::RectShape::filled(r, radius, Color32::from_white_alpha(a))
                    .with_texture(v.texture, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))),
            );
            // The bezel over the black beside the picture.
            if v.bezel {
                if let Some(tex) = crate::bezel::art_set(game).and_then(|s| self.bezel_tex(s)) {
                    let [bw, bh] = tex.size();
                    let w = pic.height() * bw as f32 / bh as f32;
                    let art = Rect::from_center_size(pic.center(), vec2(w, pic.height()));
                    for (x0, x1) in [(art.left().max(r.left()), pic.left()), (pic.right(), art.right().min(r.right()))]
                    {
                        if x1 > x0 {
                            let part = Rect::from_min_max(pos2(x0, pic.top()), pos2(x1, pic.bottom()));
                            let uv =
                                Rect::from_min_max(pos2((x0 - art.left()) / w, 0.0), pos2((x1 - art.left()) / w, 1.0));
                            p.image(tex.id(), part, uv, Color32::from_white_alpha(a));
                        }
                    }
                }
            }
        }
        p.rect_stroke(r, radius, Stroke::new(1.5 * u, CYAN.gamma_multiply(0.8)), StrokeKind::Outside);
        // Status along the foot of the frame.
        let status = match (view.progress, view.no_preview) {
            (Some(f), _) => Some(format!("Preparing the attract mode preview… {:.0}%", f * 100.0)),
            (None, Some(why)) => Some(if why.contains(" not in ") {
                "No preview: chips missing from the ROM folder".to_string()
            } else {
                format!("No preview: {why}")
            }),
            _ => None,
        };
        if let Some(text) = status {
            let bar = Rect::from_min_max(pos2(r.left(), r.bottom() - 44.0 * u), r.max);
            p.add(egui::epaint::RectShape::filled(
                bar,
                egui::CornerRadius { nw: 0, ne: 0, sw: radius as u8, se: radius as u8 },
                Color32::from_black_alpha(190),
            ));
            p.with_clip_rect(bar.shrink(4.0 * u)).text(
                pos2(bar.left() + 18.0 * u, bar.center().y),
                Align2::LEFT_CENTER,
                text,
                FontId::proportional(15.0 * u),
                FG,
            );
            if let Some(f) = view.progress {
                let track = Rect::from_min_size(pos2(bar.left(), bar.top()), vec2(r.width(), 3.0 * u));
                p.rect_filled(track, 0.0, LINE);
                p.rect_filled(Rect::from_min_size(track.min, vec2(r.width() * f.clamp(0.0, 1.0), 3.0 * u)), 0.0, CYAN);
            }
        }
        let _ = now;
    }

    fn paint_info(&mut self, ui: &mut egui::Ui, p: &Painter, l: &Layout, view: &View, actions: &mut Vec<Action>) {
        let u = l.u;
        let Some(game) = self.selected() else { return };
        let r = l.info;
        let mut y = r.top() + 6.0 * u;
        // Board and year, small capitals in cyan.
        let meta = format!("{}  ·  {}", game.board.name().to_uppercase(), game.year);
        let m = p.text(pos2(r.left(), y), Align2::LEFT_TOP, meta, FontId::proportional(15.0 * u), CYAN);
        y = m.bottom() + 10.0 * u;
        // The title, as large as fits.
        let size = if game.title.len() > 22 { 40.0 } else { 48.0 } * u;
        let galley = p.layout(game.title.to_string(), FontId::proportional(size), FG, r.width());
        let t = Rect::from_min_size(pos2(r.left(), y), galley.size());
        p.galley(t.min, galley.clone(), FG);
        p.galley(t.min + vec2(0.9 * u, 0.0), galley, FG);
        y = t.bottom() + 12.0 * u;
        // The logo's red stripe.
        let bar = [
            pos2(r.left() + 8.0 * u, y),
            pos2(r.left() + 64.0 * u, y),
            pos2(r.left() + 56.0 * u, y + 5.0 * u),
            pos2(r.left(), y + 5.0 * u),
        ];
        p.add(egui::Shape::convex_polygon(bar.to_vec(), RED, Stroke::NONE));
        y += 22.0 * u;
        let set = match game.parent {
            Some(parent) => format!("{}  ·  a version of {parent}", game.id),
            None => game.id.to_string(),
        };
        let s = p.text(pos2(r.left(), y), Align2::LEFT_TOP, set, FontId::monospace(13.0 * u), DIM);
        y = s.bottom() + 18.0 * u;
        // Controls: keycaps and what they do, wrapped.
        let font = FontId::proportional(14.5 * u);
        let key_font = FontId::monospace(12.5 * u);
        let (mut x, line_h) = (r.left(), 30.0 * u);
        for (keys, label) in controls(game) {
            let key_w: Vec<f32> = keys
                .iter()
                .map(|k| p.layout_no_wrap(k.to_string(), key_font.clone(), FG).size().x.max(9.0 * u) + 12.0 * u)
                .collect();
            let label_g = p.layout_no_wrap(label, font.clone(), Color32::from_rgb(201, 208, 220));
            let w = key_w.iter().sum::<f32>() + 4.0 * u * keys.len() as f32 + label_g.size().x + 22.0 * u;
            if x > r.left() && x + w > r.right() {
                x = r.left();
                y += line_h;
            }
            if y + line_h > r.bottom() - 150.0 * u {
                break;
            }
            for (k, kw) in keys.iter().zip(&key_w) {
                let kr = Rect::from_min_size(pos2(x, y), vec2(*kw, 22.0 * u));
                keycap(p, kr, k, &key_font, u);
                x = kr.right() + 4.0 * u;
            }
            p.galley(
                pos2(x + 2.0 * u, y + 11.0 * u - label_g.size().y / 2.0),
                label_g.clone(),
                Color32::from_rgb(201, 208, 220),
            );
            x += label_g.size().x + 22.0 * u;
        }
        // The buttons, from the foot of the column up.
        let buttons = Self::buttons(view.resume.is_some());
        self.button = self.button.min(buttons.len() - 1);
        let bh = 50.0 * u;
        let gap = 12.0 * u;
        let mut by = r.bottom() - buttons.len() as f32 * (bh + gap) + gap;
        let bw = r.width().min(420.0 * u);
        for (i, (label, play)) in buttons.iter().enumerate() {
            let br = Rect::from_min_size(pos2(r.left(), by), vec2(bw, bh));
            let hit = ui.interact(br, Id::unique(("button", i)), Sense::click());
            if hit.hovered() && ui.input(|i| i.pointer.delta() != Vec2::ZERO) {
                self.button = i;
            }
            let focused = i == self.button && self.panel == Panel::Closed;
            let primary = i == 0;
            let fill = if focused {
                Color32::from_rgba_unmultiplied(47, 230, 239, 46)
            } else {
                Color32::from_rgba_unmultiplied(13, 20, 29, 220)
            };
            if focused {
                p.add(
                    egui::epaint::RectShape::filled(br, bh / 2.0, Color32::from_rgba_unmultiplied(47, 230, 239, 60))
                        .with_blur_width(18.0 * u),
                );
            }
            p.rect_filled(br, bh / 2.0, fill);
            let stroke = if focused {
                CYAN
            } else if primary {
                CYAN.gamma_multiply(0.5)
            } else {
                KEY_LINE
            };
            p.rect_stroke(br, bh / 2.0, Stroke::new(2.0 * u, stroke), StrokeKind::Inside);
            let ic = if focused { CYAN } else { DIM };
            icon(p, pos2(br.left() + 26.0 * u, br.center().y), play.is_some(), ic, u);
            let g = p.layout_no_wrap(label.to_string(), FontId::proportional(19.0 * u), FG);
            let tp = pos2(br.left() + 48.0 * u, br.center().y - g.size().y / 2.0);
            p.galley(tp, g.clone(), FG);
            p.galley(tp + vec2(0.6 * u, 0.0), g.clone(), FG);
            if *play == Some(true) {
                if let Some(t) = view.resume {
                    p.text(
                        pos2(tp.x + g.size().x + 12.0 * u, br.center().y),
                        Align2::LEFT_CENTER,
                        format!("from {}", age(t)),
                        FontId::proportional(15.0 * u),
                        DIM,
                    );
                }
            }
            if hit.clicked() {
                self.button = i;
                match play {
                    Some(resume) => actions.push(Action::Play { set: game.id.to_string(), resume: *resume }),
                    None => self.open_settings(),
                }
            }
            by += bh + gap;
        }
    }

    fn paint_cards(&mut self, ui: &mut egui::Ui, p: &Painter, l: &Layout, dt: f32, actions: &mut Vec<Action>) {
        let u = l.u;
        let gap = 18.0 * u;
        let group_gap = 56.0 * u;
        // Each card's left edge along the row, at its current scale.
        let mut xs = Vec::with_capacity(self.cards.len());
        let mut x = 0.0;
        for (i, c) in self.cards.iter().enumerate() {
            if i > 0 && c.group != self.cards[i - 1].group {
                x += group_gap;
            }
            xs.push(x);
            x += l.card.x * self.scales[i] + gap;
        }
        // The selected card in the middle, but the row doesn't leave the
        // margins empty: it starts at the left one, ends at the right one.
        let end = x - gap;
        let margin = l.hero.left() - l.screen.left();
        let centred = l.screen.center().x - (xs[self.sel] + l.card.x * self.scales[self.sel] / 2.0);
        let lowest = (l.screen.right() - margin - end).min(l.screen.left() + margin);
        let target = centred.clamp(lowest, l.screen.left() + margin);
        let shift = self.scroll.get_or_insert(target);
        *shift += (target - *shift) * ease(dt, 12.0);
        if (target - *shift).abs() < 0.3 {
            *shift = target;
        }
        let shift = *shift;
        self.scroll_target = target;
        // The mouse wheel steps through the cards.
        let row = Rect::from_min_max(
            pos2(l.screen.left(), l.cards_y - l.card.y / 2.0 - 30.0 * u),
            pos2(l.screen.right(), l.cards_y + l.card.y / 2.0 + 30.0 * u),
        );
        if ui.rect_contains_pointer(row) && self.panel == Panel::Closed {
            let d = ui.input(|i| i.smooth_scroll_delta);
            self.wheel += d.y + d.x;
            let step = 40.0;
            while self.wheel >= step {
                self.wheel -= step;
                self.select(self.sel.saturating_sub(1));
            }
            while self.wheel <= -step {
                self.wheel += step;
                self.select(self.sel + 1);
            }
        }
        // Group headings over their first card.
        for g in &self.groups {
            let gx = xs[g.start] + shift;
            if gx > l.screen.right() || xs[g.start + g.len - 1] + shift + l.card.x < l.screen.left() {
                continue;
            }
            let y = l.cards_y - l.card.y / 2.0 - 30.0 * u;
            let stripe = [
                pos2(gx + 6.0 * u, y - 2.0 * u),
                pos2(gx + 30.0 * u, y - 2.0 * u),
                pos2(gx + 24.0 * u, y + 2.0 * u),
                pos2(gx, y + 2.0 * u),
            ];
            p.add(egui::Shape::convex_polygon(stripe.to_vec(), RED, Stroke::NONE));
            p.text(
                pos2(gx + 40.0 * u, y),
                Align2::LEFT_CENTER,
                g.name.to_uppercase(),
                FontId::proportional(13.0 * u),
                CYAN,
            );
        }
        let mut clicked = None;
        for (i, c) in self.cards.iter().enumerate() {
            let s = self.scales[i];
            let size = l.card * s;
            let left = xs[i] + shift;
            if left > l.screen.right() + 10.0 || left + size.x < l.screen.left() - 10.0 {
                continue;
            }
            let r = Rect::from_min_size(pos2(left, l.cards_y - size.y / 2.0), size);
            let selected = i == self.sel;
            let radius = 10.0 * u * s;
            // How lit it is: the selected card fully, the rest dimmed.
            let lit = ((s - 0.78) / 0.22).clamp(0.0, 1.0);
            if selected {
                p.add(
                    egui::epaint::RectShape::filled(
                        r,
                        radius,
                        Color32::from_rgba_unmultiplied(47, 230, 239, (70.0 * lit) as u8),
                    )
                    .with_blur_width(26.0 * u),
                );
            }
            p.add(
                egui::epaint::RectShape::filled(
                    r.translate(vec2(0.0, 8.0 * u)),
                    radius,
                    Color32::from_black_alpha(160),
                )
                .with_blur_width(18.0 * u),
            );
            let shade = (150.0 + 105.0 * lit) as u8;
            match self.posters.get(c.game.id) {
                Some(tex) => {
                    p.add(
                        egui::epaint::RectShape::filled(r, radius, Color32::from_gray(shade))
                            .with_texture(tex.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))),
                    );
                }
                None => {
                    p.rect_filled(r, radius, PANEL);
                }
            }
            let stroke = if selected { Stroke::new(2.5 * u, CYAN) } else { Stroke::new(1.0 * u, LINE) };
            p.rect_stroke(r, radius, stroke, StrokeKind::Outside);
            // Its title underneath, and a mark for one to continue.
            let tc = if selected { FG } else { DIM };
            let title =
                p.layout(c.game.title.to_string(), FontId::proportional(13.5 * u), tc, (l.card.x * 1.0).max(size.x));
            p.galley(pos2(r.center().x - title.size().x / 2.0, r.bottom() + 10.0 * u), title, tc);
            let hit = ui.interact(r, Id::unique(("card", i)), Sense::click());
            if hit.clicked() {
                clicked = Some(i);
            }
        }
        if let Some(i) = clicked {
            if i == self.sel && self.panel == Panel::Closed {
                if let Some(game) = self.selected() {
                    actions.push(Action::Play { set: game.id.to_string(), resume: false });
                }
            } else {
                self.select(i);
            }
        }
    }

    fn paint_hints(&self, p: &Painter, l: &Layout) {
        let u = l.u;
        let hints: &[(&str, &str)] = match (self.pad_used, self.panel) {
            (true, Panel::Closed) => &[("D-pad", "choose"), ("A", "play"), ("Y", "settings"), ("LB RB", "board")],
            (true, _) => &[("D-pad", "choose and change"), ("A", "select"), ("B", "back")],
            (false, Panel::Closed) => &[
                ("← →", "choose"),
                ("Enter", "play"),
                ("Tab", "settings"),
                ("PgUp PgDn", "board"),
                ("F11", "fullscreen"),
            ],
            (false, _) => &[("↑ ↓", "choose"), ("← →", "change"), ("Esc", "back")],
        };
        let key_font = FontId::monospace(12.0 * u);
        let font = FontId::proportional(13.5 * u);
        let parts: Vec<(f32, f32)> = hints
            .iter()
            .map(|(k, t)| {
                (
                    p.layout_no_wrap(k.to_string(), key_font.clone(), FG).size().x + 12.0 * u,
                    p.layout_no_wrap(t.to_string(), font.clone(), DIM).size().x,
                )
            })
            .collect();
        let total: f32 =
            parts.iter().map(|(a, b)| a + b + 8.0 * u).sum::<f32>() + 28.0 * u * (parts.len() as f32 - 1.0);
        let mut x = l.screen.center().x - total / 2.0;
        for ((k, t), (kw, tw)) in hints.iter().zip(parts) {
            let r = Rect::from_min_size(pos2(x, l.hints_y - 11.0 * u), vec2(kw, 22.0 * u));
            keycap(p, r, k, &key_font, u);
            p.text(pos2(r.right() + 8.0 * u, l.hints_y), Align2::LEFT_CENTER, *t, font.clone(), DIM);
            x = r.right() + 8.0 * u + tw + 28.0 * u;
        }
        // The version, small in the corner.
        let about = format!("reModel {}", remodel_version::VERSION);
        let corner = pos2(l.screen.right() - 14.0 * u, l.screen.bottom() - 10.0 * u);
        p.text(corner, Align2::RIGHT_BOTTOM, about, FontId::proportional(11.0 * u), DIM.gamma_multiply(0.7));
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_panel(
        &mut self,
        ctx: &egui::Context,
        l: &Layout,
        view: &View,
        values: &mut GameSettings,
        sources: &[Source; KEYS.len()],
        for_all: &mut bool,
        has_own: bool,
        actions: &mut Vec<Action>,
    ) {
        let u = l.u;
        let Some(game) = self.selected() else { return };
        let w = (520.0 * u).min(l.screen.width() * 0.48);
        let t = self.panel_t;
        let x = l.screen.right() - w * (1.0 - (1.0 - t).powi(3));
        let rect = Rect::from_min_size(pos2(x, l.screen.top()), vec2(w, l.screen.height()));
        egui::Area::new(Id::unique("launcher panel")).fixed_pos(rect.min).order(egui::Order::Middle).show(ctx, |ui| {
            ui.set_min_size(rect.size());
            let p = ui.painter().clone();
            p.add(egui::epaint::RectShape::filled(rect, 0.0, Color32::from_black_alpha(140)).with_blur_width(40.0 * u));
            p.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(7, 11, 18, 246));
            p.vline(rect.left(), rect.y_range(), Stroke::new(1.5 * u, CYAN.gamma_multiply(0.7)));
            let pad = 36.0 * u;
            let inner = rect.shrink2(vec2(pad, 0.0));
            let mut y = l.top.top() + 4.0 * u;
            let h = p.text(pos2(inner.left(), y), Align2::LEFT_TOP, "SETTINGS", FontId::proportional(14.0 * u), CYAN);
            y = h.bottom() + 6.0 * u;
            let g = p.layout(game.title.to_string(), FontId::proportional(28.0 * u), FG, inner.width());
            p.galley(pos2(inner.left(), y), g.clone(), FG);
            p.galley(pos2(inner.left() + 0.7 * u, y), g.clone(), FG);
            y += g.size().y + 14.0 * u;
            p.hline(inner.x_range(), y, Stroke::new(1.0 * u, LINE));
            y += 10.0 * u;
            let rows = shown_rows(&Shown {
                game,
                values,
                has_own,
                max_anisotropy: self.max_anisotropy,
                temporal_supported: self.temporal_supported,
            });
            self.row = self.row.min(rows.len() - 1);
            let row_h = 66.0 * u;
            // Keep the focused row on screen.
            let room = rect.bottom() - 60.0 * u - y;
            let scroll = ((self.row as f32 + 1.0) * row_h - room).max(0.0);
            let clip = Rect::from_min_max(pos2(rect.left(), y), pos2(rect.right(), rect.bottom() - 52.0 * u));
            let p = p.with_clip_rect(clip);
            for (i, row) in rows.iter().enumerate() {
                let r = Rect::from_min_size(
                    pos2(inner.left() - 14.0 * u, y + i as f32 * row_h - scroll),
                    vec2(inner.width() + 28.0 * u, row_h - 6.0 * u),
                );
                if !clip.intersects(r) {
                    continue;
                }
                let hit = ui.interact(r.intersect(clip), Id::unique(("row", i)), Sense::click_and_drag());
                if hit.hovered() && ui.input(|i| i.pointer.delta() != Vec2::ZERO) {
                    self.row = i;
                }
                let focused = i == self.row && self.panel == Panel::Settings;
                if focused {
                    p.rect_filled(r, 10.0 * u, Color32::from_rgba_unmultiplied(47, 230, 239, 22));
                    p.rect_stroke(r, 10.0 * u, Stroke::new(1.5 * u, CYAN.gamma_multiply(0.8)), StrokeKind::Inside);
                }
                let lx = r.left() + 14.0 * u;
                let (label, hint, key) = match row {
                    Row::Scope => ("Save for", "where changes here are kept", None),
                    Row::Choice(k, label, hint, _) => (*label, *hint, Some(*k)),
                    Row::Db(k, label, _, _) => (*label, "", Some(*k)),
                    Row::Controls => ("Controls…", "remap the keys and gamepad", None),
                    Row::Reset => ("Use the settings for all games", "forget this game's own", None),
                };
                // The hint wraps short of the control on the right; the
                // label and hint are centred on the row together.
                let control_w = match row {
                    Row::Scope => pills_width(&p, &["This game", "All games"], u),
                    Row::Choice(k, _, _, options) => {
                        let labels = choice_labels(options, &values.get(k)).0;
                        pills_width(&p, &labels.iter().map(String::as_str).collect::<Vec<_>>(), u)
                    }
                    Row::Db(..) => 330.0 * u,
                    Row::Controls | Row::Reset => 30.0 * u,
                };
                let wrap = (r.right() - 12.0 * u - control_w - 18.0 * u - lx).max(80.0 * u);
                // A setting another overrides says so in place of its hint.
                let (hint, hint_c) = match key.and_then(|k| overridden(k, values, self.temporal_supported)) {
                    Some(why) => (why.to_string(), AMBER),
                    None => match key {
                        Some("preset") => (
                            hints::preset_help(values.preset, Some(game), self.temporal_supported, values.geo_chip),
                            DIM,
                        ),
                        Some("run_ahead") => run_ahead_hint(hint, values.run_ahead, game, view),
                        _ => (hint.to_string(), DIM),
                    },
                };
                let hint_g = (!hint.is_empty())
                    .then(|| p.layout(hint.to_string(), FontId::proportional(11.5 * u), hint_c, wrap));
                let label_h = 20.0 * u;
                let block = label_h + hint_g.as_ref().map_or(0.0, |g| g.size().y + 2.0 * u);
                let label_y = r.center().y - block / 2.0 + label_h / 2.0;
                let lr = p.text(pos2(lx, label_y), Align2::LEFT_CENTER, label, FontId::proportional(16.5 * u), FG);
                if let Some(g) = hint_g {
                    p.galley(pos2(lx, label_y + label_h / 2.0 + 2.0 * u), g, hint_c);
                }
                if let Some(k) = key {
                    let src = sources[KEYS.iter().position(|x| *x == k).unwrap_or(0)];
                    let tag = match src {
                        Source::Game => Some(("this game", CYAN)),
                        Source::CommandLine => Some(("command line", AMBER)),
                        _ => None,
                    };
                    if let Some((text, color)) = tag {
                        let tg = p.layout_no_wrap(text.to_string(), FontId::proportional(10.5 * u), color);
                        let tr = Rect::from_min_size(
                            pos2(lr.right() + 8.0 * u, label_y - 8.0 * u),
                            tg.size() + vec2(10.0 * u, 4.0 * u),
                        );
                        p.rect_stroke(tr, 8.0 * u, Stroke::new(1.0 * u, color.gamma_multiply(0.7)), StrokeKind::Inside);
                        p.galley(tr.min + vec2(5.0 * u, 2.0 * u), tg, color);
                    }
                }
                let right = r.right() - 12.0 * u;
                match *row {
                    Row::Scope => {
                        let opts = [("This game", false), ("All games", true)];
                        if let Some(v) = pills(
                            ui,
                            &p,
                            right,
                            r.center().y,
                            &opts.map(|o| o.0),
                            opts.iter().position(|o| o.1 == *for_all).unwrap_or(0),
                            u,
                            ("scope", i),
                        ) {
                            *for_all = opts[v].1;
                            self.row = i;
                        }
                    }
                    Row::Choice(k, _, _, options) => {
                        let (labels, at) = choice_labels(options, &values.get(k));
                        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
                        if let Some(v) = pills(ui, &p, right, r.center().y, &labels, at, u, ("choice", i)) {
                            if let Some(o) = options.get(v) {
                                change(values, k, o.1, game);
                            }
                            self.row = i;
                        }
                    }
                    Row::Db(k, _, min, max) => {
                        let v: f32 = values.get(k).parse().unwrap_or(0.0);
                        let text = format!("{}{v:.0} dB", if v > 0.0 { "+" } else { "" });
                        let vr = p.text(
                            pos2(right, r.center().y),
                            Align2::RIGHT_CENTER,
                            text,
                            FontId::monospace(14.0 * u),
                            FG,
                        );
                        let track = Rect::from_min_max(
                            pos2(right - 250.0 * u, r.center().y - 2.0 * u),
                            pos2(vr.left() - 16.0 * u, r.center().y + 2.0 * u),
                        );
                        let f = (v - min) / (max - min);
                        p.rect_filled(track, 2.0 * u, KEY_LINE);
                        p.rect_filled(
                            Rect::from_min_max(track.min, pos2(track.left() + track.width() * f, track.bottom())),
                            2.0 * u,
                            CYAN,
                        );
                        p.circle_filled(pos2(track.left() + track.width() * f, track.center().y), 8.0 * u, FG);
                        if hit.is_pointer_button_down_on() || hit.dragged() {
                            if let Some(pos) = hit.interact_pointer_pos() {
                                if pos.x >= track.left() - 12.0 * u {
                                    let f = ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0);
                                    values.set(k, &(min + f * (max - min)).round().to_string());
                                }
                            }
                            self.row = i;
                        }
                    }
                    Row::Controls | Row::Reset => {
                        p.text(
                            pos2(right, r.center().y),
                            Align2::RIGHT_CENTER,
                            "›",
                            FontId::proportional(24.0 * u),
                            if focused { CYAN } else { DIM },
                        );
                        if hit.clicked() {
                            self.row = i;
                            if *row == Row::Controls {
                                self.panel = Panel::Controls;
                            } else {
                                actions.push(Action::ResetGame(game.id.to_string()));
                            }
                        }
                    }
                }
            }
            settle(values, game);
            // Close, at the foot.
            let p = ui.painter().clone();
            let foot = pos2(inner.left(), rect.bottom() - 28.0 * u);
            let g = p.layout_no_wrap(
                if self.pad_used { "B  close" } else { "Esc  close" }.into(),
                FontId::proportional(13.5 * u),
                DIM,
            );
            let cr = Rect::from_min_size(pos2(foot.x, foot.y - g.size().y / 2.0), g.size());
            let hit = ui.interact(cr.expand(8.0 * u), Id::unique("panel close"), Sense::click());
            p.galley(cr.min, g, if hit.hovered() { FG } else { DIM });
            if hit.clicked() {
                self.panel = Panel::Closed;
            }
        });
    }

    /// No games: say where reModel looked, and let another folder be typed in.
    fn empty(&mut self, ctx: &egui::Context, l: &Layout, view: &View, actions: &mut Vec<Action>) {
        let u = l.u;
        egui::Area::new(Id::unique("launcher empty")).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.set_max_width(640.0 * u);
            ui.vertical_centered(|ui| {
                if let Some(logo) = &self.logo {
                    let [w, h] = logo.size();
                    let lh = 90.0 * u;
                    ui.add(egui::Image::new((logo.id(), vec2(lh * w as f32 / h as f32, lh))));
                }
                ui.add_space(24.0 * u);
                ui.label(egui::RichText::new("No games found").size(28.0 * u).color(FG));
                ui.add_space(8.0 * u);
                ui.label(
                    egui::RichText::new(
                        "reModel looks for MAME sets (zips or folders, e.g. topskatr.zip) in this folder:",
                    )
                    .size(15.0 * u)
                    .color(DIM),
                );
                ui.add_space(12.0 * u);
                ui.horizontal(|ui| {
                    let edit = egui::TextEdit::singleline(&mut self.rom_text)
                        .font(FontId::monospace(15.0 * u))
                        .desired_width(460.0 * u);
                    let r = ui.add(edit);
                    let scan = ui.add(egui::Button::new(egui::RichText::new("Scan").size(16.0 * u)));
                    if scan.clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                        actions.push(Action::Rescan(self.rom_text.trim().to_string()));
                    }
                });
                if view.unsupported > 0 {
                    ui.add_space(8.0 * u);
                    ui.label(
                        egui::RichText::new(format!("{} other sets there aren't supported yet", view.unsupported))
                            .size(14.0 * u)
                            .color(DIM),
                    );
                }
                ui.add_space(20.0 * u);
                if ui.add(egui::Button::new(egui::RichText::new("Quit").size(16.0 * u))).clicked() {
                    actions.push(Action::Quit);
                }
            });
        });
    }
}

/// A play triangle, or a settings mark (three sliders), centred on `c`.
fn icon(p: &Painter, c: Pos2, play: bool, color: Color32, u: f32) {
    if play {
        let pts = vec![c + vec2(-5.0, -7.0) * u, c + vec2(8.0, 0.0) * u, c + vec2(-5.0, 7.0) * u];
        p.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
        return;
    }
    for (i, knob) in [(-1.0f32, 3.0f32), (0.0, -4.0), (1.0, 1.0)] {
        let y = c.y + i * 6.0 * u;
        p.hline((c.x - 8.0 * u)..=(c.x + 8.0 * u), y, Stroke::new(1.5 * u, color));
        p.circle_filled(pos2(c.x + knob * u, y), 2.6 * u, color);
    }
}

/// The game's controls for its card: the manifest's summary, with
/// direction buttons bound to the arrow keys shown as one stick, and the
/// start buttons last.
fn controls(game: &romset::Game) -> Vec<(Vec<&'static str>, String)> {
    let mut out = game.controls.summary();
    let arrows = ["←", "→", "↑", "↓"];
    let dirs: Vec<usize> = (0..out.len()).filter(|&i| out[i].0.len() == 1 && arrows.contains(&out[i].0[0])).collect();
    let is_dir = |label: &str| ["up", "down", "left", "right"].contains(&label.to_lowercase().as_str());
    if dirs.len() == 4 && dirs.iter().all(|&i| is_dir(&out[i].1)) {
        let at = dirs[0];
        for &i in dirs.iter().rev() {
            out.remove(i);
        }
        out.insert(at, (arrows.to_vec(), "Move".to_string()));
    }
    out.sort_by_key(|(_, label)| label.starts_with("Start"));
    out
}

/// A keyboard key or pad button's cap with `text` on it.
pub fn keycap(p: &Painter, r: Rect, text: &str, font: &FontId, u: f32) {
    p.rect_filled(r, 4.0 * u, KEY_BG);
    p.rect_stroke(r, 4.0 * u, Stroke::new(1.0 * u, KEY_LINE), StrokeKind::Inside);
    p.hline(r.x_range().shrink(2.0 * u), r.bottom() - 1.0 * u, Stroke::new(1.5 * u, KEY_LINE));
    p.text(r.center() - vec2(0.0, 1.0 * u), Align2::CENTER_CENTER, text, font.clone(), FG);
}

/// A choice row's pill labels and the selected one: a value off the list
/// (a `materials_relief` of 1.5 from the settings file or the command line)
/// gets a pill of its own after the others, "Custom (1.5)", so the row
/// shows what is in effect instead of highlighting the first choice.
fn choice_labels(options: &[(&str, &str)], value: &str) -> (Vec<String>, usize) {
    let mut labels: Vec<String> = options.iter().map(|o| o.0.to_string()).collect();
    let at = options.iter().position(|o| o.1 == value).unwrap_or_else(|| {
        labels.push(format!("Custom ({value})"));
        options.len()
    });
    (labels, at)
}

/// How wide `pills` draws `labels`.
fn pills_width(p: &Painter, labels: &[&str], u: f32) -> f32 {
    let font = FontId::proportional(14.0 * u);
    labels.iter().map(|l| p.layout_no_wrap(l.to_string(), font.clone(), FG).size().x + 24.0 * u).sum::<f32>()
        + 6.0 * u * (labels.len() as f32 - 1.0)
}

/// Choice pills ending at `right`, centred on `y`, `at` chosen; returns one
/// clicked.
#[allow(clippy::too_many_arguments)]
fn pills(
    ui: &egui::Ui,
    p: &Painter,
    right: f32,
    y: f32,
    labels: &[&str],
    at: usize,
    u: f32,
    id: (&str, usize),
) -> Option<usize> {
    let font = FontId::proportional(14.0 * u);
    let h = 32.0 * u;
    let widths: Vec<f32> =
        labels.iter().map(|l| p.layout_no_wrap(l.to_string(), font.clone(), FG).size().x + 24.0 * u).collect();
    let mut x = right - widths.iter().sum::<f32>() - 6.0 * u * (labels.len() as f32 - 1.0);
    let mut clicked = None;
    for (i, (label, w)) in labels.iter().zip(widths).enumerate() {
        let r = Rect::from_min_size(pos2(x, y - h / 2.0), vec2(w, h));
        let hit = ui.interact(r, Id::unique((id.0, id.1, i)), Sense::click());
        let on = i == at;
        let fill = if on { Color32::from_rgba_unmultiplied(47, 230, 239, 40) } else { KEY_BG };
        p.rect_filled(r, h / 2.0, fill);
        let stroke = if on {
            CYAN
        } else if hit.hovered() {
            DIM
        } else {
            KEY_LINE
        };
        p.rect_stroke(r, h / 2.0, Stroke::new(1.5 * u, stroke), StrokeKind::Inside);
        p.text(
            r.center(),
            Align2::CENTER_CENTER,
            *label,
            font.clone(),
            if on { FG } else { Color32::from_rgb(201, 208, 220) },
        );
        if hit.clicked() {
            clicked = Some(i);
        }
        x += w + 6.0 * u;
    }
    clicked
}

/// A vertical gradient over `r`.
fn gradient(p: &Painter, r: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(r.left_top(), top);
    mesh.colored_vertex(r.right_top(), top);
    mesh.colored_vertex(r.left_bottom(), bottom);
    mesh.colored_vertex(r.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    p.add(egui::Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn games(ids: &[&str]) -> Vec<&'static romset::Game> {
        ids.iter().map(|id| romset::games::find(id).unwrap()).collect()
    }

    #[test]
    fn grouped_by_board_recent_first() {
        let mut l = Launcher::default();
        l.set_games(&games(&["topskatr", "hotd", "fvipers", "doa", "stcc"]), &["hotd".into(), "gone".into()]);
        let names: Vec<&str> = l.groups().iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Recently played", "Model 2B", "Model 2C"]);
        assert_eq!(l.selected().unwrap().id, "hotd");
        // Titles in order within a board.
        let b: Vec<&str> = l.cards[l.groups[1].start..][..l.groups[1].len].iter().map(|c| c.game.title).collect();
        assert_eq!(b, ["Dead or Alive", "Fighting Vipers"]);
        assert_eq!(l.groups[2].len, 3);
    }

    #[test]
    fn model_2a_has_its_own_group_before_2b() {
        let mut l = Launcher::default();
        l.set_games(&games(&["doa", "srallyc", "topskatr"]), &[]);
        let names: Vec<&str> = l.groups().iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Model 2A", "Model 2B", "Model 2C"]);
        assert_eq!(l.selected().unwrap().id, "srallyc");
    }

    #[test]
    fn original_model_2_has_its_own_group_first() {
        let mut l = Launcher::default();
        l.set_games(&games(&["srallyc", "daytona", "topskatr"]), &[]);
        let names: Vec<&str> = l.groups().iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Model 2", "Model 2A", "Model 2C"]);
        assert_eq!(l.selected().unwrap().id, "daytona");
    }

    #[test]
    fn settings_temporal_overrides() {
        let mut v = GameSettings { aa: 4, ..Default::default() };
        assert!(["aa", "temporal", "stipple"].iter().all(|k| overridden(k, &v, true).is_none()));
        v.temporal = 100;
        assert!(overridden("aa", &v, true).is_some() && overridden("temporal", &v, true).is_none());
        assert!(overridden("aa", &v, false).is_none() && overridden("temporal", &v, false).is_some());
        v.geo_chip = true;
        v.stipple_keep = true;
        assert!(overridden("aa", &v, true).is_none());
        assert!(overridden("temporal", &v, true).is_some() && overridden("stipple", &v, true).is_some());
        // Shading: the chip doesn't turn it off; pixel is drawn as vertex
        // with it, and says so (per pixel with the native geometrizer).
        let mut v = GameSettings { geo_chip: true, shading: gpu::Shading::Vertex, ..Default::default() };
        assert!(overridden("shading", &v, true).is_none());
        v.shading = gpu::Shading::Pixel;
        assert!(overridden("shading", &v, true).is_some_and(|s| s.contains("as vertex")));
        assert!(overridden("shading", &GameSettings { geo_chip: false, ..v.clone() }, true).is_none());
        v.crease = 0;
        assert!(overridden("shading", &v, true).is_some_and(|s| s.contains("nothing")));
        // Material maps: no effect without the pack or with the chip.
        let mut v = GameSettings { materials: true, ..Default::default() };
        assert!(overridden("materials", &v, true).is_some_and(|s| s.contains("texture pack")));
        v.pack = true;
        assert!(overridden("materials", &v, true).is_none());
        v.geo_chip = true;
        assert!(overridden("materials", &v, true).is_some_and(|s| s.contains("native")));
        let v = GameSettings { materials_relief: gpu::Relief::Faithful, ..Default::default() };
        assert!(overridden("materials_relief", &v, true).is_some());
        assert!(overridden("materials_relief", &GameSettings { materials: true, pack: true, ..v }, true).is_none());
        // The rows hold every value the settings take for them.
        for key in ["temporal", "stipple", "shading", "materials", "materials_relief", "bloom"] {
            let Some(Row::Choice(_, _, _, options)) =
                rows(false, 16).into_iter().find(|r| matches!(r, Row::Choice(k, ..) if *k == key))
            else {
                panic!("no {key} row")
            };
            for (_, value) in options {
                let mut v = GameSettings::default();
                assert!(v.set(key, value) && v.get(key) == *value, "{key} = {value}");
            }
        }
    }

    #[test]
    fn a_value_off_the_list_shows_as_custom() {
        let Some(Row::Choice(_, _, _, options)) =
            rows(false, 16).into_iter().find(|r| matches!(r, Row::Choice("materials_relief", ..)))
        else {
            panic!("no materials_relief row")
        };
        let (labels, at) = choice_labels(options, "2");
        assert_eq!((labels.len(), labels[at].as_str()), (options.len(), "2"));
        let mut v = GameSettings::default();
        assert!(v.set("materials_relief", "1.5"));
        let (labels, at) = choice_labels(options, &v.get("materials_relief"));
        assert_eq!((labels.len(), at, labels[at].as_str()), (options.len() + 1, options.len(), "Custom (1.5)"));
    }

    #[test]
    fn navigation() {
        let mut l = Launcher::default();
        l.set_games(&games(&["topskatr", "hotd", "fvipers", "doa", "stcc"]), &[]);
        let (mut v, mut all, mut acts) = (GameSettings::default(), true, Vec::new());
        let mut nav = |l: &mut Launcher, n: Nav, v: &mut GameSettings| l.nav(n, false, v, &mut all, false, &mut acts);
        assert_eq!(l.selection(), 0);
        nav(&mut l, Nav::Left, &mut v);
        assert_eq!(l.selection(), 0, "stops at the first");
        nav(&mut l, Nav::NextGroup, &mut v);
        assert_eq!(l.selected().unwrap().board.name(), "Model 2C");
        nav(&mut l, Nav::Right, &mut v);
        nav(&mut l, Nav::PrevGroup, &mut v);
        assert_eq!(l.selection(), l.groups[1].start, "to the group's start");
        nav(&mut l, Nav::PrevGroup, &mut v);
        assert_eq!(l.selection(), 0, "then the one before");
        // Settings: down to the aspect row, change it, back out.
        nav(&mut l, Nav::Settings, &mut v);
        assert_eq!(l.panel, Panel::Settings);
        nav(&mut l, Nav::Right, &mut v);
        assert!(!v.wide, "4:3");
        nav(&mut l, Nav::Down, &mut v);
        nav(&mut l, Nav::Accept, &mut v);
        assert!(!v.bezel);
        nav(&mut l, Nav::Back, &mut v);
        assert_eq!(l.panel, Panel::Closed);
        nav(&mut l, Nav::Accept, &mut v);
        let game = l.selected().unwrap().id;
        assert!(matches!(&acts[..], [Action::Play { set, resume: false }] if set == game));
    }

    #[test]
    fn continue_or_new_game() {
        let mut l = Launcher::default();
        l.set_games(&games(&["topskatr"]), &[]);
        let (mut v, mut all, mut acts) = (GameSettings::default(), true, Vec::new());
        l.nav(Nav::Accept, true, &mut v, &mut all, false, &mut acts);
        l.nav(Nav::Down, true, &mut v, &mut all, false, &mut acts);
        l.nav(Nav::Accept, true, &mut v, &mut all, false, &mut acts);
        let got: Vec<bool> = acts
            .iter()
            .map(|a| match a {
                Action::Play { resume, .. } => *resume,
                _ => panic!(),
            })
            .collect();
        assert_eq!(got, [true, false]);
    }

    #[test]
    fn layout_fits_every_shape() {
        for (w, h) in
            [(1280.0, 720.0), (1920.0, 1080.0), (3440.0, 1440.0), (1024.0, 768.0), (1920.0, 1200.0), (800.0, 600.0)]
        {
            let s = Rect::from_min_size(Pos2::ZERO, vec2(w, h));
            let l = layout(s);
            for r in [l.top, l.hero, l.info] {
                assert!(s.contains_rect(r), "{w}x{h}: {r:?}");
            }
            assert!(l.hero.right() < l.info.left() && l.info.width() >= 300.0 * l.u, "{w}x{h}");
            assert!(l.hero.bottom() < l.cards_y - l.card.y / 2.0 - 30.0 * l.u, "{w}x{h}: cards clear the preview");
            assert!(l.cards_y + l.card.y / 2.0 + 30.0 * l.u < l.hints_y, "{w}x{h}: titles clear the hints");
            assert!(l.hints_y < h, "{w}x{h}");
            // The preview has the window's shape, within the views' range.
            let a = l.hero.width() / l.hero.height();
            assert!((a - (w / h).clamp(crate::ASPECTS.0, crate::ASPECTS.1)).abs() < 0.01, "{w}x{h}: {a}");
        }
    }

    /// Headless: the launcher lays out at 1080p and 4K (2x), every card
    /// and button inside the screen, and the keyboard moves it.
    #[test]
    fn draws_headless() {
        let mut l = Launcher::default();
        l.set_games(&games(&["topskatr", "hotd", "fvipers", "doa", "stcc", "overrevb", "sgt24h"]), &["stcc".into()]);
        let ctx = egui::Context::default();
        let (mut v, mut all) = (GameSettings::default(), true);
        let sources = [Source::Default; KEYS.len()];
        let view = View {
            preview: None,
            progress: Some(0.4),
            no_preview: None,
            resume: None,
            unsupported: 0,
            run_ahead_room: None,
            run_ahead_dropped: false,
        };
        for (size, ppp) in [(vec2(1920.0, 1080.0), 1.0), (vec2(1920.0, 1080.0), 2.0), (vec2(3440.0, 1440.0), 1.0)] {
            for key in [None, Some(egui::Key::ArrowRight), Some(egui::Key::Tab), Some(egui::Key::Escape)] {
                let before = l.selection();
                // The key, then frames a tenth of a second apart until the
                // animations have settled.
                let mut out = None;
                for frame in 0..12 {
                    let mut input = egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size / ppp)),
                        ..Default::default()
                    };
                    input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
                    if let (Some(key), 0) = (key, frame) {
                        input.events.push(egui::Event::Key {
                            key,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: Default::default(),
                        });
                    }
                    l.last_frame = Some(Instant::now() - Duration::from_millis(100));
                    let mut o = ctx.run_ui(input, |ui| {
                        l.ui(ui.ctx(), Vec::new(), &view, &mut v, &sources, &mut all, false);
                    });
                    o.textures_delta.clear();
                    out = Some(o);
                }
                let out = out.unwrap();
                assert!(!l.animating(), "{size:?}@{ppp} {key:?}: settled");
                if key == Some(egui::Key::ArrowRight) {
                    assert_eq!(l.selection(), before + 1);
                }
                if key == Some(egui::Key::Tab) {
                    assert_eq!(l.panel, Panel::Settings);
                }
                let screen = Rect::from_min_size(Pos2::ZERO, size / ppp);
                for s in &out.shapes {
                    if let egui::Shape::Text(t) = &s.shape {
                        let r = t.visual_bounding_rect();
                        // Cards' titles run off the sides; everything else
                        // is on screen.
                        if r.center().y < screen.height() * 0.75 {
                            assert!(screen.expand(1.0).contains_rect(r), "{size:?}@{ppp}: {r:?}");
                        }
                    }
                }
            }
        }
        assert_eq!(l.panel, Panel::Closed);
    }

    fn shown(id: &str, v: &GameSettings) -> Vec<Row> {
        let game = romset::games::find(id).unwrap();
        shown_rows(&Shown { game, values: v, has_own: false, max_anisotropy: 16, temporal_supported: true })
    }

    fn keys(rows: &[Row]) -> Vec<&'static str> {
        rows.iter().filter_map(|r| if let Row::Choice(k, ..) = r { Some(*k) } else { None }).collect()
    }

    #[test]
    fn the_list_follows_the_preset_like_the_menu() {
        let mut v = GameSettings::default();
        let k = keys(&shown("topskatr", &v));
        assert!(k.contains(&"preset") && k.contains(&"exact") && k.contains(&"smooth"));
        assert!(!k.iter().any(|k| ["aa", "temporal", "depth", "shading", "pack"].contains(k)), "{k:?}");
        // The preset comes before the rest of the picture, which Custom adds.
        v.preset = Preset::Custom;
        let k = keys(&shown("topskatr", &v));
        let at = |key| k.iter().position(|x| *x == key).unwrap();
        assert!(at("preset") < at("aa") && at("aa") < at("temporal") && at("depth") < at("shading"), "{k:?}");
        assert!(at("shading") < at("exact"), "picture before motion");
        // Rows with nothing to do are left out, as in the menu: no
        // anti-aliasing under temporal, no lighting rows with flat shading,
        // none of the native geometrizer's with the chip.
        v.temporal = 100;
        let k = keys(&shown("topskatr", &v));
        assert!(!k.contains(&"aa") && k.contains(&"stipple") && !k.contains(&"shading_fine"), "{k:?}");
        v.shading = gpu::Shading::Vertex;
        assert!(keys(&shown("topskatr", &v)).contains(&"shading_fine"));
        v.geo_chip = true;
        let k = keys(&shown("topskatr", &v));
        assert!(!k.contains(&"temporal") && !k.contains(&"smooth") && !k.contains(&"stipple") && k.contains(&"aa"));
        // The wide view has the HUD row, a 4:3 one the bezel.
        let v = GameSettings { wide: true, ..Default::default() };
        let k = keys(&shown("topskatr", &v));
        assert!(k.contains(&"hud_edges") && !k.contains(&"bezel"));
        let k = keys(&shown("topskatr", &GameSettings { wide: false, ..Default::default() }));
        assert!(!k.contains(&"hud_edges"));
    }

    #[test]
    fn run_ahead_is_capped_per_game() {
        let options = |id: &str, v: &GameSettings| {
            shown(id, v).into_iter().find_map(|r| match r {
                Row::Choice("run_ahead", _, _, o) => Some(o.len()),
                _ => None,
            })
        };
        let v = GameSettings::default();
        assert_eq!(options("sgt24h", &v), Some(4));
        assert_eq!(options("stcc", &v), Some(2));
        assert_eq!(options("hotd", &v), None, "a zero-lag game has no row");
        // Set anyway, it can be turned off.
        assert_eq!(options("hotd", &GameSettings { run_ahead: 1, ..v }), Some(1));
    }

    #[test]
    fn choosing_in_the_list_is_choosing_in_the_menu() {
        let game = romset::games::find("stcc").unwrap();
        let mut v = GameSettings::default();
        change(&mut v, "preset", "enhanced", game);
        let mut menu = GameSettings::default();
        menu.apply_preset(Preset::Enhanced, Some(game));
        assert_eq!(v, menu);
        assert_eq!(v.get("preset"), "enhanced");
        // Smooth motion and run-ahead don't count against the preset...
        change(&mut v, "run_ahead", "0", game);
        settle(&mut v, game);
        assert_eq!(v.preset, Preset::Enhanced);
        // ...a picture row does.
        change(&mut v, "aa", "8", game);
        settle(&mut v, game);
        assert_eq!((v.preset, v.aa), (Preset::Custom, 8));
        // The preset row offers what the menu does, by the files' names.
        let rows = shown("stcc", &v);
        let Some(Row::Choice(_, _, _, options)) = rows.iter().find(|r| keys(&[**r]) == ["preset"]) else {
            panic!("no preset row")
        };
        let names: Vec<_> = options.iter().map(|o| o.1).collect();
        assert_eq!(names, hints::PRESETS.map(|(p, _)| p.name()));
        assert_eq!(options.iter().map(|o| o.0).collect::<Vec<_>>(), hints::PRESETS.map(|(_, l)| l));
    }

    #[test]
    fn the_run_ahead_hint_says_what_the_computer_can_do() {
        let game = romset::games::find("sgt24h").unwrap();
        let view = |room, dropped| View {
            preview: None,
            progress: None,
            no_preview: None,
            resume: None,
            unsupported: 0,
            run_ahead_room: room,
            run_ahead_dropped: dropped,
        };
        let base = "base";
        assert_eq!(run_ahead_hint(base, 2, game, &view(None, false)), ("base".to_string(), DIM), "unmeasured: as is");
        let (t, c) = run_ahead_hint(base, 1, game, &view(Some(2), false));
        assert!(t.contains("about 2") && c == DIM, "{t}");
        let (t, c) = run_ahead_hint(base, 3, game, &view(Some(1), false));
        assert!(t.contains("3 is more") && c == AMBER, "{t}");
        let (t, _) = run_ahead_hint(base, 0, game, &view(Some(0), false));
        assert!(t.contains("too slow for run-ahead"), "{t}");
        let (t, c) = run_ahead_hint(base, 0, game, &view(None, true));
        assert!(t.contains("Switched off") && c == AMBER);
    }
}
