//! The launcher: the player's start screen, drawn with egui over the
//! backdrop (`backdrop`), in the theme's look (`theme`).
//!
//! Two views of the games found in the ROM folder (never the ones missing),
//! switched from the top bar or with LB and RB, and remembered:
//!
//! * the **carousel**: the selected game's poster large at the right, the
//!   next games beside it, and at the left its name, kind, board and year
//!   and one line about the player and the game; the game's live attract
//!   mode (`preview`, once it has been selected for `SETTLE`) plays dimmed
//!   behind everything, its poster blurred there until it does;
//! * the **grid**: every game as a poster, with filter chips (All, Recent
//!   and the kinds there are games of: `library::Filter`), a sort
//!   (`library::Sort`) and "N of M games".
//!
//! Both have the same top bar (the logo, the quick resume pill, the view
//! switch) and the same hint bar along the bottom (the pad's buttons, or the
//! keys when the keyboard was used last), whose entries can be clicked too.
//! Everything is reachable from a pad (`focus`, whose ring shows while the
//! pad or the keyboard is in use) and from the mouse. A game's poster opens
//! its **game page** (`page`): the live attract preview of the chosen
//! version beside Play, Version, Play together, Saves and Game settings,
//! each opening one level down; the settings open the menu's side panel
//! (`menu`) on the page they name.
//!
//! Keyboard: arrows choose, Enter opens the game's page, C continues, Q and
//! E (or Page Up and Down) switch the view, [ and ] the filter; S, L and V
//! open the page at Saves, Play together and Version, Tab the player's
//! settings. Pad: d-pad or stick, A, Y, LB RB, LT RT (X, View and Start do
//! what S, L and Tab do). Mouse: click a poster to select it, again to open
//! its page, twice quickly to play; the wheel steps through the carousel
//! and scrolls the grid.
//!
//! Everything is laid out in design points at 1280x720 and scaled to the
//! window (`layout`), so it reads the same at 1080p, ultrawide and 4K.

mod page;
mod players;
pub use players::{Goes as PlayersGoes, Outcome as PlayersOutcome, PlayersScreen, PlayersView};

use crate::focus::Focus;
use crate::library::{self, Continue, Filter, Sort, Start, ViewMode};
use crate::menu;
use crate::saves::{self, SlotInfo};
use crate::theme::{self, Chip};
use egui::{
    pos2, vec2, Align2, Color32, FontFamily, FontId, Id, Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui,
    Vec2,
};
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant, SystemTime};

/// Two clicks on a poster within this are a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(450);

/// The shared palette (`theme`).
pub use crate::theme::CYAN;
pub const FG: Color32 = crate::theme::TEXT;
pub const DIM: Color32 = crate::theme::TEXT_DIM;
const PANEL: Color32 = Color32::from_rgb(8, 12, 22);

/// How long a game stays selected before its preview starts.
pub const SETTLE: Duration = Duration::from_millis(500);
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
    /// The pad's Start (the menu button).
    Start,
    Back,
    Settings,
    /// LB (and Q, Page Up): the menu's previous tab; the launcher's view.
    PrevGroup,
    /// RB (and E, Page Down).
    NextGroup,
    /// X: the selected game's save slots.
    Saves,
    /// Y: the quick resume.
    Continue,
    /// LT and RT (and [ and ]): the grid's previous and next filter.
    PrevFilter,
    NextFilter,
    /// The pad's View (Select) button: Link play on the launcher's page,
    /// Back everywhere else ([`Nav::view_as_back`]).
    View,
}

impl Nav {
    /// View is Back in the panels and menus.
    pub fn view_as_back(navs: &[Nav]) -> Vec<Nav> {
        navs.iter().map(|&n| if n == Nav::View { Nav::Back } else { n }).collect()
    }
}

pub enum Action {
    /// Start `set`, from power-on or a saved state.
    Play {
        set: String,
        from: Start,
    },
    /// Start `set` as link play: this many machines on one screen.
    PlayLink {
        set: String,
        players: usize,
    },
    /// Open the Players screen for `set`: one cabinet, or (`link`) this many machines on
    /// this screen, whose Start plays.
    Players {
        set: String,
        link: Option<usize>,
    },
    /// Host a network link session of `set` (the player makes the lobby's socket).
    NetHost {
        set: String,
    },
    /// Join the host at `addr` (a game found on the network, from the launcher's banner).
    NetJoinAt {
        set: String,
        addr: String,
    },
    /// Join one.
    NetJoin {
        set: String,
    },
    /// A network session is agreed: start `set` as one cabinet of it.
    PlayNet {
        set: String,
        session: Box<linknet::Session>,
        params: Box<linknet::Params>,
    },
    /// The player chose a version of a title (its card shows `set`): remember it.
    Version {
        title: String,
        set: String,
    },
    /// Open the menu over the launcher (its settings).
    Settings,
    /// Open it with Quit reModel chosen (Esc or B on the home screen): Enter
    /// quits, Esc backs out.
    AskQuit,
    /// Open it on one of the selected game's pages (the game page's
    /// Game settings rows); going back closes it.
    SettingsAt(menu::Page, menu::Tab),
    /// Close it (Esc, Start).
    CloseSettings,
    /// Drop the game's own settings (the menu's "Reset all to system").
    ResetGame(String),
    /// Look for games in this ROM folder.
    Rescan(String),
    /// The player's name for network link play was edited.
    SetPlayerName(String),
    Quit,
}

/// Where everything goes, in points. The 1280x720 design scaled by `u` to
/// fit, its content centred and no wider than a 3440x1440 screen takes.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    /// Points per design point.
    pub u: f32,
    pub screen: Rect,
    /// The top bar (logo, pill, view switch).
    pub top: Rect,
    /// Between the top bar and the hint bar.
    pub body: Rect,
    /// The hint bar, across the foot of the screen.
    pub bar: Rect,
}

pub fn layout(screen: Rect) -> Layout {
    let u = (screen.width() / 1280.0).min(screen.height() / 720.0).max(theme::MIN_SCALE);
    let width = (screen.width() - 96.0 * u).min(1744.0 * u).max(120.0 * u);
    let left = screen.center().x - width / 2.0;
    let bar = Rect::from_min_max(pos2(screen.left(), screen.bottom() - 64.0 * u), screen.max);
    Layout {
        u,
        screen,
        top: Rect::from_min_size(pos2(left, screen.top() + 28.0 * u), vec2(width, 44.0 * u)),
        body: Rect::from_min_max(pos2(left, screen.top() + 100.0 * u), pos2(left + width, bar.top())),
        bar,
    }
}

impl Layout {
    /// The preview's size, for the texture it's drawn into: the screen's
    /// shape at 0.6 of its size (shown dimmed behind the carousel, and in a
    /// frame on the game page).
    pub fn hero(&self) -> Rect {
        Rect::from_center_size(self.screen.center(), self.screen.size() * 0.6)
    }

    /// Where the backdrop's horizon is, as a fraction of the screen's
    /// height, and how bright it is, for `mode`: the carousel's floor lies
    /// under the posters, the grid's is faint at the foot.
    pub fn backdrop(mode: ViewMode) -> (f32, f32) {
        match mode {
            ViewMode::Carousel => (0.64, 1.0),
            ViewMode::Grid => (0.92, 0.3),
        }
    }
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
    /// The quick resume: the last game played and its latest state.
    pub cont: Option<Continue>,
    /// The menu is open over the launcher (Settings).
    pub menu_open: bool,
    /// The menu is waiting for a key or button to bind (Esc is the
    /// binding's, not the way out).
    pub capturing: bool,
    /// What the selected game's settings are now, for the game page's rows.
    pub settings: SettingsLine,
}

/// The selected game's settings in a few words each.
#[derive(Clone, Debug, Default)]
pub struct SettingsLine {
    pub picture: String,
    pub sound: String,
    pub speed: String,
    /// The Free play setting's label, for a game that has one.
    pub free_play: Option<String>,
    /// How the game is shown: its aspect.
    pub display: String,
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
}

/// A save slot's entry in the Saves panel.
struct SaveItem {
    from: Start,
    name: String,
    info: Option<SlotInfo>,
    thumb: Option<egui::TextureHandle>,
}

/// An entry of the hint bar: a button (or key), what it does, and the click
/// it makes (none for the ones that only say how to move).
pub(crate) struct Hint {
    pub chip: Chip<'static>,
    pub label: String,
    pub click: Option<Click>,
}

/// A click (or a pad press) on the launcher's page, applied after it is
/// drawn.
#[derive(Clone, Copy)]
pub(crate) enum Click {
    View(ViewMode),
    Filter(Filter),
    NextFilter,
    Sort,
    Continue,
    /// The player's settings (the menu's pause page).
    Settings,
    /// Ask to quit (System settings, at Quit reModel).
    Quit,
    /// Open the selected game's page (at its main menu).
    OpenPage,
    /// Open it at a level (the S, L and V keys, the pad's X and View).
    OpenAt(page::Level),
    /// A game's poster: selects it, or opens its page if it is; clicked
    /// (the flag) twice quickly it plays.
    Game(&'static romset::Game, bool),
    /// The banner's game hosted on the network: its page at Play together.
    LanJoin,
    /// On the page: back a level, or to the games.
    PageBack,
    /// Go down to a level of the page.
    Enter(page::Level),
    /// Start the chosen version.
    Play,
    /// A version chosen in the page's list.
    SetVersion(&'static romset::Game),
    /// Open the Players screen for the chosen version (one cabinet).
    Players,
    /// Open it for this many machines on this screen: Start plays.
    Together(usize),
    NetHost,
    NetJoin,
    /// Join the host the banner names.
    NetJoinHosted,
    /// Start from a save slot.
    Load(Start),
    /// Open the menu on a page of the game's settings.
    SettingsAt(menu::Page, menu::Tab),
}

pub struct Launcher {
    pub open: bool,
    /// The playable games (titles and their variants), and the recently
    /// played (latest first).
    all: Vec<&'static romset::Game>,
    /// The cards: one per title (a variant whose title isn't there stands
    /// alone), the carousel's and grid's lists draw from these.
    cards: Vec<&'static romset::Game>,
    /// How many versions each card has (its own set and every variant there).
    versions_n: HashMap<&'static str, usize>,
    recent: Vec<String>,
    /// The version of each title chosen (title to set id), kept by the
    /// player's settings.
    chosen: BTreeMap<String, String>,
    pub mode: ViewMode,
    pub filter: Filter,
    pub sort: Sort,
    chips: Vec<Filter>,
    /// The carousel's games (everything, the played first) and the grid's
    /// (filtered, sorted).
    carousel: Vec<&'static romset::Game>,
    grid: Vec<&'static romset::Game>,
    /// The selected game.
    cur: Option<&'static romset::Game>,
    /// When the selection last changed.
    pub selected_at: Instant,
    /// The carousel's slide, in cards, towards 0 after a step; each card's
    /// growth (0 small, 1 selected).
    slide: f32,
    grow: HashMap<&'static str, f32>,
    last_frame: Option<Instant>,
    wheel: f32,
    /// The last input came from a gamepad (or none yet: the couch's
    /// buttons are the default).
    pub pad_used: bool,
    keys_used: bool,
    focus: Focus,
    /// Where the focus goes once the page has drawn it.
    pending_focus: Option<(Id, u8)>,
    /// The selected game's page, when it is open.
    page: Option<page::GamePage>,
    /// The last poster clicked and when (a second click on it soon plays).
    last_click: Option<(&'static str, Instant)>,
    /// The player's name was edited on the page this frame.
    name_edited: bool,
    /// The network link play panel (lobby or join), over everything else.
    net_page: Option<crate::netpage::NetPage>,
    /// Games hosted on this network (mDNS), for the banner: the browse
    /// (started on the first frame; never in tests), the hosts this player
    /// could join, and when to look again.
    lan: Option<linknet::Browser>,
    lan_off: bool,
    lan_hosts: Vec<linknet::Found>,
    lan_next: Option<std::time::Instant>,
    posters: HashMap<String, egui::TextureHandle>,
    /// Each poster blurred (a small copy, stretched), the carousel's
    /// background while the preview isn't there.
    blurs: HashMap<String, egui::TextureHandle>,
    art: Option<crate::art::Loader>,
    logo: Option<egui::TextureHandle>,
    /// The ROM folder being edited (no games found).
    pub rom_text: String,
    /// The player's name for network link play, as edited on the Link play page.
    pub player_name: String,
}

/// The id the carousel's selected card has whichever game it is, so the
/// pad's focus stays on it as the games pass through.
fn selected_card() -> Id {
    Id::unique("carousel-selected")
}

fn tile_id(game: &romset::Game) -> Id {
    Id::unique(("tile", game.id))
}

fn button_id(name: &str) -> Id {
    Id::unique(("launcher-button", name))
}

impl Default for Launcher {
    fn default() -> Self {
        Launcher {
            open: false,
            all: Vec::new(),
            cards: Vec::new(),
            versions_n: HashMap::new(),
            recent: Vec::new(),
            chosen: BTreeMap::new(),
            mode: ViewMode::default(),
            filter: Filter::All,
            sort: Sort::default(),
            chips: vec![Filter::All],
            carousel: Vec::new(),
            grid: Vec::new(),
            cur: None,
            selected_at: Instant::now(),
            slide: 0.0,
            grow: HashMap::new(),
            last_frame: None,
            wheel: 0.0,
            pad_used: true,
            keys_used: false,
            focus: Focus::default(),
            pending_focus: None,
            page: None,
            last_click: None,
            name_edited: false,
            net_page: None,
            lan: None,
            lan_off: false,
            lan_hosts: Vec::new(),
            lan_next: None,
            posters: HashMap::new(),
            blurs: HashMap::new(),
            art: None,
            logo: None,
            rom_text: String::new(),
            player_name: String::new(),
        }
    }
}

/// "2 hours ago" and so on.
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

fn full_uv() -> Rect {
    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))
}

/// `image` as a small copy (24 texels across): drawn stretched with linear
/// filtering it is a blur.
fn blur_image(image: &egui::ColorImage) -> egui::ColorImage {
    const W: usize = 24;
    let (w0, h0) = (image.width().max(1), image.height().max(1));
    let h = (h0 * W / w0).max(1);
    let mut pixels = Vec::with_capacity(W * h);
    for y in 0..h {
        for x in 0..W {
            let (x0, x1) = (x * w0 / W, ((x + 1) * w0 / W).clamp(x * w0 / W + 1, w0));
            let (y0, y1) = (y * h0 / h, ((y + 1) * h0 / h).clamp(y * h0 / h + 1, h0));
            let (mut sum, mut n) = ([0u32; 3], 0u32);
            // Every few texels is enough of a cell for a blur.
            for yy in (y0..y1).step_by(((y1 - y0) / 4).max(1)) {
                for xx in (x0..x1).step_by(((x1 - x0) / 4).max(1)) {
                    let c = image.pixels[yy * w0 + xx];
                    sum = [sum[0] + c.r() as u32, sum[1] + c.g() as u32, sum[2] + c.b() as u32];
                    n += 1;
                }
            }
            let n = n.max(1);
            pixels.push(Color32::from_rgb((sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8));
        }
    }
    egui::ColorImage::new([W, h], pixels)
}

/// A control: its hit area, registered with the pad's focus. True if it
/// was clicked or A was pressed on it.
fn control(focus: &mut Focus, ui: &Ui, id: Id, rect: Rect) -> (Response, bool) {
    let r = ui.interact(rect, id, Sense::click());
    let hit = focus.register(ui, &r);
    (r, hit)
}

/// `text` cut with an ellipsis to fit `width`.
fn fit_text(p: &Painter, text: &str, font: &FontId, width: f32) -> String {
    let wide = |t: &str| p.layout_no_wrap(t.to_string(), font.clone(), FG).size().x;
    if wide(text) <= width {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while chars.len() > 1 {
        chars.pop();
        let t: String = chars.iter().collect::<String>().trim_end().to_string() + "…";
        if wide(&t) <= width {
            return t;
        }
    }
    "…".to_string()
}

impl Launcher {
    /// Set the games (playable sets in the ROM folder) and the recently
    /// played (most recent first), keeping the selection if it's still
    /// there, else selecting the first.
    pub fn set_games(&mut self, games: &[&'static romset::Game], recent: &[String]) {
        self.all = games.to_vec();
        self.cards = library::cards(games);
        self.versions_n = self.cards.iter().map(|c| (c.id, library::versions(c, games).len())).collect();
        self.recent = recent.to_vec();
        self.chips = Filter::chips(&self.cards, recent);
        if !self.chips.contains(&self.filter) {
            self.filter = Filter::All;
        }
        self.relist();
        let keep = self.cur.filter(|c| self.cards.iter().any(|g| g.id == c.id));
        let first = self.carousel.first().copied();
        self.cur = keep.or(first);
        self.selected_at = Instant::now();
        self.grow.clear();
        self.slide = 0.0;
        self.pending_focus = Some((self.selected_id(), 6));
        if !games.is_empty() {
            self.art = Some(crate::art::Loader::start(games.to_vec()));
        }
    }

    /// The lists, from the games, the recent, the filter and the sort.
    fn relist(&mut self) {
        self.carousel = library::list(&self.cards, &self.recent, Filter::All, Sort::Recent);
        self.grid = library::list(&self.cards, &self.recent, self.filter, self.sort);
    }

    /// The games the current view lists.
    fn view_list(&self) -> &[&'static romset::Game] {
        match self.mode {
            ViewMode::Carousel => &self.carousel,
            ViewMode::Grid => &self.grid,
        }
    }

    /// The selected game's place in the current view.
    fn index(&self) -> Option<usize> {
        let cur = self.cur?;
        self.view_list().iter().position(|g| g.id == cur.id)
    }

    /// The id the focus takes to rest on the selected game.
    fn selected_id(&self) -> Id {
        match (self.mode, self.cur) {
            (ViewMode::Grid, Some(g)) => tile_id(g),
            _ => selected_card(),
        }
    }

    /// Select the game for `set`: its title's card, showing that version.
    pub fn select_set(&mut self, set: &str) {
        let card = self.cards.iter().copied().find(|c| library::is_version_of(set, c));
        if let Some(g) = card {
            if set != g.id {
                self.chosen.insert(g.id.to_string(), set.to_string());
            }
            self.select(g);
            self.pending_focus = Some((self.selected_id(), 6));
        }
    }

    /// The player's chosen versions (title to set id), from the settings.
    pub fn set_versions(&mut self, chosen: &BTreeMap<String, String>) {
        self.chosen = chosen.clone();
    }

    /// The versions the card `card` lists: the title, then every variant
    /// whose ROMs are there, hacks and prototypes too.
    fn versions_of(&self, card: &'static romset::Game) -> Vec<&'static romset::Game> {
        library::versions(card, &self.all)
    }

    /// The version of the selected card that is shown and played.
    fn version(&self) -> Option<&'static romset::Game> {
        let card = self.cur?;
        Some(library::chosen_version(card, &self.versions_of(card), &self.chosen))
    }

    /// Choose version `to` of the selected card.
    fn choose_version(&mut self, to: &'static romset::Game, actions: &mut Vec<Action>) {
        let Some(card) = self.cur else { return };
        if self.version().map(|g| g.id) == Some(to.id) {
            return;
        }
        self.chosen.insert(card.id.to_string(), to.id.to_string());
        actions.push(Action::Version { title: card.id.to_string(), set: to.id.to_string() });
        self.selected_at = Instant::now();
    }

    fn select(&mut self, game: &'static romset::Game) {
        if self.cur.map(|c| c.id) != Some(game.id) {
            self.selected_at = Instant::now();
        }
        self.cur = Some(game);
    }

    /// The game the selected card plays: its chosen version.
    pub fn selected(&self) -> Option<&'static romset::Game> {
        self.version()
    }

    /// The selected card (the title).
    #[cfg(test)]
    fn card(&self) -> Option<&'static romset::Game> {
        self.cur
    }

    /// Step the selection through the view's list by `d` (the carousel wraps
    /// round, the grid stops at its ends).
    fn step(&mut self, d: i32) {
        let n = self.view_list().len() as i32;
        let Some(at) = self.index().map(|i| i as i32).filter(|_| n > 0) else { return };
        let to = match self.mode {
            ViewMode::Carousel => (at + d).rem_euclid(n),
            ViewMode::Grid => (at + d).clamp(0, n - 1),
        };
        if to != at {
            let game = self.view_list()[to as usize];
            self.select(game);
            if self.mode == ViewMode::Carousel && d.abs() == 1 {
                self.slide = (self.slide + d as f32).clamp(-1.5, 1.5);
            }
        }
    }

    /// Show `mode`, keeping the selected game.
    pub fn set_mode(&mut self, mode: ViewMode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        self.slide = 0.0;
        if self.index().is_none() {
            if let Some(first) = self.view_list().first().copied() {
                self.select(first);
            }
        }
        self.pending_focus = Some((self.selected_id(), 6));
    }

    /// Show the games `filter` keeps (when it is among the chips).
    pub fn set_filter(&mut self, filter: Filter) {
        if !self.chips.contains(&filter) {
            return;
        }
        self.filter = filter;
        self.relist();
        if self.mode == ViewMode::Grid && self.index().is_none() {
            if let Some(first) = self.grid.first().copied() {
                self.select(first);
            }
        }
        self.pending_focus = Some((self.selected_id(), 6));
    }

    /// The filter `d` chips on, wrapping round.
    fn cycle_filter(&mut self, d: i32) {
        let n = self.chips.len() as i32;
        let at = self.chips.iter().position(|c| *c == self.filter).unwrap_or(0) as i32;
        self.set_filter(self.chips[(at + d).rem_euclid(n) as usize]);
    }

    pub fn set_sort(&mut self, sort: Sort) {
        self.sort = sort;
        self.relist();
    }

    /// The selected game's preview is wanted: the carousel and the game
    /// page show it, the grid doesn't.
    pub fn wants_preview(&self) -> bool {
        self.mode == ViewMode::Carousel || self.page.is_some()
    }

    /// The view whose backdrop is under the screen: the game page's is the
    /// grid's, faint.
    pub fn backdrop_view(&self) -> ViewMode {
        if self.page.is_some() {
            ViewMode::Grid
        } else {
            self.mode
        }
    }

    /// The games the grid lists now, and how many there are in all.
    pub fn counts(&self) -> (usize, usize) {
        (self.grid.len(), self.cards.len())
    }

    #[cfg(test)]
    fn grid_ids(&self) -> Vec<&'static str> {
        self.grid.iter().map(|g| g.id).collect()
    }

    #[cfg(test)]
    fn carousel_ids(&self) -> Vec<&'static str> {
        self.carousel.iter().map(|g| g.id).collect()
    }

    /// The game page or the network panel is open.
    pub fn page_open(&self) -> bool {
        self.page.is_some() || self.net_page.is_some()
    }

    /// The keyboard's `Nav`s this frame (taken from egui's input, so the
    /// widgets don't see them twice).
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
            (Key::S, Nav::Saves),
            (Key::C, Nav::Continue),
            (Key::Q, Nav::PrevGroup),
            (Key::E, Nav::NextGroup),
            (Key::PageUp, Nav::PrevGroup),
            (Key::PageDown, Nav::NextGroup),
            (Key::OpenBracket, Nav::PrevFilter),
            (Key::CloseBracket, Nav::NextFilter),
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

    /// Upload decoded art.
    fn art(&mut self, ctx: &egui::Context) {
        let Some(loader) = &self.art else { return };
        for (set, image) in loader.take() {
            let blur = ctx.load_texture(format!("blur {set}"), blur_image(&image), egui::TextureOptions::LINEAR);
            self.blurs.insert(set.clone(), blur);
            let tex = ctx.load_texture(format!("poster {set}"), image, egui::TextureOptions::LINEAR);
            self.posters.insert(set, tex);
        }
    }

    /// Note where this frame's input came from, as the menu does: the
    /// pad's turns the focus ring and the pad's buttons on, the keyboard's
    /// navigation keys the ring and the keys' hints, the pointer or other
    /// keys the ring off.
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

    /// Draw the launcher and act on the input. `navs` are the gamepad's;
    /// the keyboard's and mouse's are read here.
    pub fn ui(&mut self, ctx: &egui::Context, navs: Vec<Nav>, view: &View) -> Vec<Action> {
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
        if self.cur.is_none() {
            self.empty(ctx, &l, view, &mut actions);
            return actions;
        }

        // The menu over the launcher takes the input; Esc and Start close it.
        let keys = if view.menu_open {
            if !view.capturing {
                let esc = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                if esc || navs.contains(&Nav::Start) {
                    actions.push(Action::CloseSettings);
                }
            }
            Vec::new()
        } else if ctx.egui_wants_keyboard_input() {
            // Typing an address or a name: the keys are the text's.
            Vec::new()
        } else {
            Self::keys(ctx)
        };
        self.note_input(ctx, &navs, &keys);
        let all: Vec<Nav> = navs.iter().chain(&keys).copied().collect();
        // The menu and the network panel take the input; the page behind
        // gets none (its focus just holds).
        let blocked = view.menu_open || self.net_page.is_some();
        // View opens Link play from the home screen; over a page or a panel it's Back.
        let page_view = !blocked && self.page.is_none() && all.contains(&Nav::View);
        // (Used up here, so the page it opens doesn't see a Back.)
        let all =
            if page_view { all.into_iter().filter(|&n| n != Nav::View).collect() } else { Nav::view_as_back(&all) };
        let mut clicks = Vec::new();
        self.poll_lan(ctx);
        if self.page.is_some() {
            let page_navs = if blocked { Vec::new() } else { all.clone() };
            self.page_frame(ctx, &l, view, page_navs, &mut clicks);
            if std::mem::take(&mut self.name_edited) {
                actions.push(Action::SetPlayerName(self.player_name.clone()));
            }
        } else {
            let page_navs = if blocked { Vec::new() } else { all.clone() };
            let page_navs = self.override_navs(page_navs);
            self.focus.show = self.pad_used || self.keys_used;
            if !blocked {
                let links = self.cur.is_some_and(|c| crate::link::title_links(c.id));
                let versions = self.cur.is_some_and(|c| self.versions_of(c).len() > 1);
                let key = |k: egui::Key| ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, k));
                if links && key(egui::Key::L) {
                    clicks.push(Click::OpenAt(page::Level::Together));
                }
                if versions && key(egui::Key::V) {
                    clicks.push(Click::OpenAt(page::Level::Version));
                }
                if !self.lan_hosts.is_empty() && key(egui::Key::J) {
                    clicks.push(Click::LanJoin);
                }
                if page_view && links {
                    clicks.push(Click::OpenAt(page::Level::Together));
                }
            }
            for nav in self.focus.begin(&page_navs) {
                match nav {
                    Nav::Start | Nav::Settings => clicks.push(Click::Settings),
                    Nav::Back => clicks.push(Click::Quit),
                    Nav::Saves => clicks.push(Click::OpenAt(page::Level::Saves)),
                    Nav::Continue => clicks.push(Click::Continue),
                    Nav::PrevGroup | Nav::NextGroup => clicks.push(Click::View(self.mode.other())),
                    Nav::PrevFilter if self.mode == ViewMode::Grid => self.cycle_filter(-1),
                    Nav::NextFilter if self.mode == ViewMode::Grid => self.cycle_filter(1),
                    _ => {}
                }
            }

            // Animation.
            self.slide -= self.slide * ease(dt, 14.0);
            if self.slide.abs() < 0.01 {
                self.slide = 0.0;
            }

            let area = egui::Area::new(Id::unique("launcher")).fixed_pos(l.screen.min).order(egui::Order::Background);
            area.show(ctx, |ui| {
                ui.set_min_size(l.screen.size());
                let p = ui.painter().clone();
                self.paint_background(&p, &l, view);
                match self.mode {
                    ViewMode::Carousel => self.paint_carousel(ui, &p, &l, view, dt, &mut clicks),
                    ViewMode::Grid => self.paint_grid(ui, &p, &l, view, &mut clicks),
                }
                self.paint_top(ui, &p, &l, view, &mut clicks);
                let hints = self.home_hints();
                let count = (self.mode == ViewMode::Grid).then(|| self.counts());
                Self::paint_bar(ui, &p, &l, view, &mut self.focus, hints, count, &mut clicks);
            });
            self.focus.end();
            self.apply_focus();
        }
        for click in clicks {
            self.apply(click, view, &mut actions);
        }
        if let Some(page) = &mut self.net_page {
            let show = self.pad_used || self.keys_used;
            if page.paint(ctx, &all, show, &mut actions) {
                self.net_page = None;
            }
        }
        actions
    }

    /// The carousel's own moves: left and right on the selected card step
    /// through the games (round, as the strip is). The rest is for the
    /// focus to place by where things are.
    fn override_navs(&mut self, navs: Vec<Nav>) -> Vec<Nav> {
        if self.mode != ViewMode::Carousel || self.focus.focused() != Some(selected_card()) {
            return navs;
        }
        let mut rest = Vec::new();
        for nav in navs {
            match nav {
                Nav::Left => self.step(-1),
                Nav::Right => self.step(1),
                other => rest.push(other),
            }
        }
        rest
    }

    /// Move the focus where it was asked to go, once the page has been
    /// drawn with that widget in it (a try or two); then take the
    /// selection from where the focus is in the grid.
    fn apply_focus(&mut self) {
        if let Some((id, tries)) = self.pending_focus {
            self.focus.focus_to(id);
            self.pending_focus = (self.focus.focused() != Some(id) && tries > 1).then_some((id, tries - 1));
        }
        if self.mode == ViewMode::Grid {
            if let Some(id) = self.focus.focused() {
                if let Some(game) = self.grid.iter().copied().find(|g| tile_id(g) == id) {
                    self.select(game);
                }
            }
        }
    }

    /// Do what a click (or a pad press) on the page asked.
    fn apply(&mut self, click: Click, view: &View, actions: &mut Vec<Action>) {
        let set = self.version().map(|g| g.id.to_string());
        match click {
            Click::View(mode) => self.set_mode(mode),
            Click::Filter(f) => self.set_filter(f),
            Click::NextFilter => self.cycle_filter(1),
            Click::Sort => self.set_sort(self.sort.other()),
            Click::Continue => {
                if let Some(c) = view.cont {
                    actions.push(Action::Play { set: c.game.id.to_string(), from: c.from });
                }
            }
            Click::Settings => actions.push(Action::Settings),
            Click::Quit => actions.push(Action::AskQuit),
            Click::OpenPage => self.open_page(page::Level::Main, false),
            Click::OpenAt(level) => self.open_page(level, false),
            Click::Game(g, mouse) => {
                let now = Instant::now();
                let double = mouse && self.last_click.is_some_and(|(id, t)| id == g.id && now - t < DOUBLE_CLICK);
                self.last_click = mouse.then_some((g.id, now));
                if double {
                    self.last_click = None;
                    self.select(g);
                    if let Some(set) = self.version().map(|v| v.id.to_string()) {
                        actions.push(Action::Play { set, from: Start::New });
                    }
                } else if self.cur.map(|c| c.id) == Some(g.id) {
                    self.open_page(page::Level::Main, mouse);
                } else {
                    self.select(g);
                    self.pending_focus = Some((self.selected_id(), 6));
                }
            }
            Click::LanJoin => {
                let Some(f) = self.lan_hosts.first().cloned() else { return };
                let Some(card) = self.cards.iter().copied().find(|c| library::is_version_of(&f.game, c)) else {
                    return;
                };
                self.select(card);
                if let Some(v) = self.all.iter().copied().find(|g| g.id == f.game) {
                    self.choose_version(v, actions);
                }
                self.open_page(page::Level::Together, false);
                if let Some(page) = &mut self.page {
                    page.want(page::Level::Together, "join hosted");
                }
            }
            Click::PageBack => self.page_back(),
            Click::Enter(level) => self.page_enter(level),
            Click::Play => actions.extend(set.map(|set| Action::Play { set, from: Start::New })),
            Click::SetVersion(g) => {
                self.choose_version(g, actions);
                if self.page.is_some() {
                    self.page_leave(page::Level::Version);
                    if let Some(page) = &mut self.page {
                        page.want(page::Level::Main, "play");
                    }
                }
            }
            Click::Players => actions.extend(set.map(|set| Action::Players { set, link: None })),
            Click::Together(players) => actions.extend(set.map(|set| Action::Players { set, link: Some(players) })),
            Click::NetHost => actions.extend(set.map(|set| Action::NetHost { set })),
            Click::NetJoin => actions.extend(set.map(|set| Action::NetJoin { set })),
            Click::NetJoinHosted => {
                let host = set.and_then(|set| self.lan_hosts.iter().find(|f| f.game == set).cloned());
                if let Some(f) = host {
                    actions.push(Action::NetJoinAt { set: f.game.clone(), addr: f.addr.to_string() });
                }
            }
            Click::Load(from) => actions.extend(set.map(|set| Action::Play { set, from })),
            Click::SettingsAt(page, tab) => actions.push(Action::SettingsAt(page, tab)),
        }
    }

    /// Whether anything is still moving (else redraws needn't animate).
    pub fn animating(&self) -> bool {
        // (The posters only grow and slide in the carousel.)
        let growing = self.mode == ViewMode::Carousel
            && self.grow.iter().any(|(id, g)| {
                let target = if self.cur.map(|c| c.id) == Some(*id) { 1.0 } else { 0.0 };
                (g - target).abs() > 0.01
            });
        growing
            || (self.mode == ViewMode::Carousel && self.slide != 0.0)
            || self.pending_focus.is_some()
            || self.page.as_ref().is_some_and(|p| p.animating())
    }

    fn paint_background(&self, p: &Painter, l: &Layout, view: &View) {
        let s = l.screen;
        match self.mode {
            ViewMode::Carousel => {
                let cover = |tex: egui::TextureId, uv: Rect, alpha: u8| {
                    p.image(tex, s, uv, Color32::from_white_alpha(alpha));
                };
                // The poster blurred, brightness .38 over the backdrop's black.
                if let Some(tex) = self.cur.and_then(|g| self.blurs.get(g.id)) {
                    let [w, h] = tex.size();
                    let (w, h) = (w as f32, h as f32);
                    let k = (s.width() / w).max(s.height() / h);
                    let crop = vec2(s.width() / (w * k), s.height() / (h * k));
                    cover(tex.id(), Rect::from_center_size(pos2(0.5, 0.5), crop), 100);
                }
                // The attract mode, dimmed, over it: the square of the
                // texture's uv that is the picture's middle (the texture has
                // the screen's shape, so a square in uv is the screen's).
                if let Some(v) = view.preview.filter(|v| v.alpha > 0.0) {
                    let [px, py, pw, ph] = v.picture;
                    let side = pw.min(ph);
                    let uv = Rect::from_center_size(pos2(px + pw / 2.0, py + ph / 2.0), vec2(side, side));
                    cover(v.texture, uv, (v.alpha * 105.0) as u8);
                }
                // Dark behind the text at the left, lighter towards the right.
                let (a, b, c) =
                    (Color32::from_black_alpha(245), Color32::from_black_alpha(184), Color32::from_black_alpha(64));
                let mid = s.left() + s.width() * 0.42;
                hgradient(p, Rect::from_min_max(s.min, pos2(mid, s.bottom())), a, b);
                hgradient(p, Rect::from_min_max(pos2(mid, s.top()), s.max), b, c);
            }
            ViewMode::Grid => {
                // A cool glow at the top right fading to black.
                let glow = Color32::from_rgba_unmultiplied(15, 42, 68, 200);
                vgradient(
                    p,
                    Rect::from_min_max(s.min, pos2(s.right(), s.top() + s.height() * 0.6)),
                    glow,
                    Color32::TRANSPARENT,
                );
            }
        }
    }

    /// The top bar: the logo, the quick resume pill (in the grid; the
    /// carousel's is bigger, under it) and the view switch.
    /// Look for games hosted on this network about once a second: the ones
    /// this player could join (a game they have, the same reModel, a free seat).
    fn poll_lan(&mut self, ctx: &egui::Context) {
        if self.lan_off {
            return;
        }
        let now = std::time::Instant::now();
        if self.lan_next.is_some_and(|t| now < t) {
            return;
        }
        self.lan_next = Some(now + std::time::Duration::from_secs(1));
        if self.lan.is_none() {
            match linknet::Browser::start() {
                Ok(b) => self.lan = Some(b),
                Err(e) => {
                    eprintln!("launcher: no network game discovery ({e})");
                    self.lan_off = true;
                    return;
                }
            }
        }
        let Some(b) = self.lan.as_mut() else { return };
        let all = &self.all;
        let before = self.lan_hosts.len();
        self.lan_hosts = b
            .poll()
            .into_iter()
            .filter(|f| {
                f.version == env!("CARGO_PKG_VERSION")
                    && f.lobby == linknet::Lobby::Open
                    && f.players < f.max
                    && all.iter().any(|g| g.id == f.game)
            })
            .collect();
        if self.lan_hosts.len() != before {
            ctx.request_repaint();
        }
    }

    /// The banner for a game hosted on this network: who, what, and Join.
    /// It ends at `right`; returns its left edge.
    fn paint_lan(&mut self, ui: &mut Ui, p: &Painter, l: &Layout, right: f32, clicks: &mut Vec<Click>) -> f32 {
        let Some(f) = self.lan_hosts.first() else { return right };
        let u = l.u;
        let title = romset::games::find(&f.game).map_or_else(|_| f.game.clone(), |g| g.title.to_string());
        let more = self.lan_hosts.len() - 1;
        let text = format!(
            "{} is hosting {title} · {}/{}{}",
            f.name,
            f.players,
            f.max,
            if more > 0 { format!(" (+{more} more)") } else { String::new() }
        );
        let font = FontId::new(17.0 * u, theme::semibold());
        let g = p.layout_no_wrap(text, font, FG);
        let chip = self.chip(Chip::None, "J");
        let d = 30.0;
        let cw = theme::chip_width_at(p, chip, u, d);
        let join = p.layout_no_wrap("Join".into(), FontId::new(17.0 * u, theme::bold()), theme::CYAN);
        let w = 18.0 * u + g.size().x + 16.0 * u + cw + 8.0 * u + join.size().x + 18.0 * u;
        let h = 46.0 * u;
        let rect = Rect::from_min_size(pos2(right - w, l.top.center().y - h / 2.0), vec2(w, h));
        let id = button_id("lan join");
        let (r, hit) = control(&mut self.focus, ui, id, rect);
        p.rect_filled(rect, 999.0, Color32::from_rgba_unmultiplied(20, 60, 72, if r.hovered() { 235 } else { 200 }));
        p.rect_stroke(rect, 999.0, Stroke::new(1.5 * u, theme::CYAN), egui::StrokeKind::Inside);
        let mut x = rect.left() + 18.0 * u;
        p.galley(pos2(x, rect.center().y - g.size().y / 2.0), g.clone(), FG);
        x += g.size().x + 16.0 * u;
        theme::paint_chip_at(p, chip, pos2(x, rect.center().y), u, d);
        x += cw + 8.0 * u;
        p.galley(pos2(x, rect.center().y - join.size().y / 2.0), join, theme::CYAN);
        if self.focus.is_on(id) {
            theme::focus_ring(p, rect, u, 23.0);
        }
        if hit {
            clicks.push(Click::LanJoin);
        }
        rect.left()
    }

    fn paint_top(&mut self, ui: &mut Ui, p: &Painter, l: &Layout, view: &View, clicks: &mut Vec<Click>) {
        let u = l.u;
        let mut x = l.top.left();
        if let Some(logo) = &self.logo {
            let [w, h] = logo.size();
            let r =
                Rect::from_min_size(pos2(x, l.top.top()), vec2(l.top.height() * w as f32 / h as f32, l.top.height()));
            p.image(logo.id(), r, full_uv(), Color32::WHITE);
            x = r.right() + 24.0 * u;
        }
        if self.mode == ViewMode::Grid {
            if let Some(c) = view.cont {
                self.pill(ui, p, l, c, true, pos2(x, l.top.center().y), clicks);
            }
        }
        // The view switch, at the right (the carousel's count beside it).
        let font = FontId::new(18.0 * u, theme::bold());
        let labels = ViewMode::ALL.map(|m| m.label());
        let widths: Vec<f32> =
            labels.iter().map(|t| p.layout_no_wrap(t.to_string(), font.clone(), FG).size().x + 40.0 * u).collect();
        let total = widths.iter().sum::<f32>() + 16.0 * u;
        let mut right = l.top.right();
        if self.mode == ViewMode::Carousel {
            let n = self.cards.len();
            let text = format!("{n} game{}", if n == 1 { "" } else { "s" });
            let r = p.text(
                pos2(right, l.top.center().y),
                Align2::RIGHT_CENTER,
                text,
                FontId::new(18.0 * u, theme::semibold()),
                Color32::from_rgb(185, 195, 214),
            );
            right = r.left() - 24.0 * u;
        }
        let capsule = Rect::from_min_size(pos2(right - total, l.top.center().y - 25.0 * u), vec2(total, 50.0 * u));
        p.rect_filled(capsule, 999.0, Color32::from_white_alpha(20));
        let mut sx = capsule.left() + 8.0 * u;
        for (mode, w) in ViewMode::ALL.into_iter().zip(widths) {
            let seg = Rect::from_min_size(pos2(sx, capsule.top() + 5.0 * u), vec2(w, capsule.height() - 10.0 * u));
            let id = button_id(mode.name());
            let (r, hit) = control(&mut self.focus, ui, id, seg);
            let on = mode == self.mode;
            if on {
                p.rect_filled(seg, 999.0, FG);
            } else if r.hovered() {
                p.rect_filled(seg, 999.0, Color32::from_white_alpha(24));
            }
            p.text(seg.center(), Align2::CENTER_CENTER, mode.label(), font.clone(), if on { theme::INK } else { FG });
            if self.focus.is_on(id) {
                theme::focus_ring(p, seg, u, 22.0);
            }
            if hit && !on {
                clicks.push(Click::View(mode));
            }
            sx += w;
        }
        self.paint_lan(ui, p, l, capsule.left() - 16.0 * u, clicks);
    }

    /// The quick resume pill: the last game played and where it picks up.
    /// `compact`: one line in the top bar, `at` the left edge's middle;
    /// else the carousel's card, `at` its top left.
    #[allow(clippy::too_many_arguments)]
    fn pill(
        &mut self,
        ui: &mut Ui,
        p: &Painter,
        l: &Layout,
        c: Continue,
        compact: bool,
        at: Pos2,
        clicks: &mut Vec<Click>,
    ) {
        let u = l.u;
        // A variant says which: "Top Skater (Japan)".
        let named = if romset::variants::get(c.game.id).is_some() {
            romset::variants::display_name(c.game.id)
        } else {
            c.game.title.to_string()
        };
        let when = match c.saved {
            Some(t) => format!("{}, {}", c.from.label(), age(t)),
            None => "from the start".to_string(),
        };
        let chip = self.chip(Chip::Y, "C");
        let d = if compact { 32.0 } else { 36.0 } * u;
        let (thumb, pad) = if compact { (40.0 * u, 6.0 * u) } else { (56.0 * u, 12.0 * u) };
        let caption_font = FontId::new(18.0 * u, theme::semibold());
        let line_font = FontId::new(if compact { 18.0 } else { 22.0 } * u, theme::semibold());
        let (caption, line) = if compact {
            ("Continue".to_string(), format!("{} · {}", named, c.saved.map_or("new".to_string(), age)))
        } else {
            ("CONTINUE".to_string(), format!("{} · {when}", named))
        };
        let cg = p.layout_no_wrap(caption, caption_font.clone(), theme::CYAN_LIGHT);
        let lg = p.layout_no_wrap(line, line_font, FG);
        let chip_w = theme::chip_width_at(p, chip, u, d / u);
        let gap = 14.0 * u;
        let (text_w, text_h) = if compact {
            (cg.size().x + 8.0 * u + lg.size().x, lg.size().y)
        } else {
            (cg.size().x.max(lg.size().x), cg.size().y + 2.0 * u + lg.size().y)
        };
        let size = vec2(
            pad + thumb + gap + text_w + gap + chip_w + pad + if compact { 2.0 * u } else { 8.0 * u },
            thumb + 2.0 * pad,
        );
        let rect = if compact {
            Rect::from_min_size(pos2(at.x, at.y - size.y / 2.0), size)
        } else {
            Rect::from_min_size(at, size)
        };
        let radius = if compact { 12.0 } else { 14.0 };
        let id = button_id("continue");
        let (r, hit) = control(&mut self.focus, ui, id, rect);
        let fill = if r.hovered() { Color32::from_white_alpha(32) } else { Color32::from_white_alpha(20) };
        p.rect_filled(rect, radius * u, fill);
        p.rect_stroke(
            rect,
            radius * u,
            Stroke::new(1.0 * u, Color32::from_rgba_unmultiplied(34, 229, 255, 90)),
            StrokeKind::Inside,
        );
        let tr = Rect::from_min_size(pos2(rect.left() + pad, rect.center().y - thumb / 2.0), vec2(thumb, thumb));
        match self.posters.get(c.game.id) {
            Some(tex) => {
                let [w, h] = tex.size();
                let frac = (w as f32 / h as f32).min(1.0);
                let uv = Rect::from_center_size(pos2(0.5, 0.5), vec2(1.0, frac));
                p.add(egui::epaint::RectShape::filled(tr, 8.0 * u, Color32::WHITE).with_texture(tex.id(), uv));
            }
            None => {
                p.rect_filled(tr, 8.0 * u, PANEL);
            }
        }
        let tx = tr.right() + gap;
        if compact {
            let y = rect.center().y;
            p.galley(pos2(tx, y - cg.size().y / 2.0), cg.clone(), theme::CYAN_LIGHT);
            p.galley(pos2(tx + cg.size().x + 8.0 * u, y - lg.size().y / 2.0), lg, FG);
        } else {
            let ty = rect.center().y - text_h / 2.0;
            p.galley(pos2(tx, ty), cg.clone(), theme::CYAN_LIGHT);
            p.galley(pos2(tx, ty + cg.size().y + 2.0 * u), lg, FG);
        }
        theme::paint_chip_at(p, chip, pos2(rect.right() - pad - 2.0 * u - chip_w, rect.center().y), u, d / u);
        if self.focus.is_on(id) {
            theme::focus_ring(p, rect, u, radius);
        }
        if hit {
            clicks.push(Click::Continue);
        }
    }

    /// A chip as the last input names its button: the pad's, or the key.
    fn chip<'a>(&self, pad: Chip<'a>, key: &'a str) -> Chip<'a> {
        if self.pad_used {
            pad
        } else {
            Chip::Key(key)
        }
    }

    fn paint_carousel(&mut self, ui: &mut Ui, p: &Painter, l: &Layout, view: &View, dt: f32, clicks: &mut Vec<Click>) {
        let u = l.u;
        let Some(card) = self.cur else { return };
        // The version of the title that Play starts: its own title, year and board.
        let versions = self.versions_of(card);
        let game = self.version().unwrap_or(card);
        if let Some(c) = view.cont {
            self.pill(ui, p, l, c, false, pos2(l.body.left(), l.body.top() + 4.0 * u), clicks);
        }
        // The game: its name large, what kind it is, and what can be done.
        let info_w = (520.0 * u).min(l.body.width() * 0.45);
        let left = l.body.left();
        let mut y = l.body.top() + 112.0 * u;
        let size = match game.title.chars().count() {
            0..=14 => 60.0,
            15..=22 => 48.0,
            _ => 40.0,
        };
        let title = p.layout(game.title.to_uppercase(), FontId::new(size * u, theme::display()), FG, info_w);
        p.galley(pos2(left, y), title.clone(), FG);
        y += title.size().y + 14.0 * u;
        let chip_font = FontId::new(18.0 * u, theme::semibold());
        let mut x = left;
        let facts = [
            (game.genre.to_string(), Color32::from_rgba_unmultiplied(34, 229, 255, 41), theme::CYAN_LIGHT),
            (game.board.name().to_string(), Color32::from_white_alpha(26), theme::TEXT_2),
            (game.year.to_string(), Color32::from_white_alpha(26), theme::TEXT_2),
        ];
        let mut row_h = 0.0;
        for (text, fill, ink) in facts {
            let g = p.layout_no_wrap(text, chip_font.clone(), ink);
            let r = Rect::from_min_size(pos2(x, y), g.size() + vec2(28.0 * u, 14.0 * u));
            if x > left && r.right() > left + info_w {
                x = left;
                y += row_h + 8.0 * u;
            }
            let r = Rect::from_min_size(pos2(x, y), r.size());
            p.rect_filled(r, 999.0, fill);
            p.galley(r.min + vec2(14.0 * u, 7.0 * u), g, ink);
            x = r.right() + 10.0 * u;
            row_h = r.height();
        }
        y += row_h + 14.0 * u;
        let line = match view.resume {
            Some(t) => format!("You left off {}", age(t)),
            None if library::played_rank(&self.recent, card).is_some() => "Played before".to_string(),
            None => "Not played yet".to_string(),
        };
        let para = p.layout(line, FontId::new(22.0 * u, theme::semibold()), FG, info_w);
        p.galley(pos2(left, y), para.clone(), FG);
        y += para.size().y + 6.0 * u;
        // Under it, dim: the versions, and linking where the title does.
        let mut extra = Vec::new();
        if versions.len() > 1 {
            extra.push(format!("{} versions", versions.len()));
        }
        if let Some(n) = crate::link::title_machines(card.id) {
            extra.push(format!("Link play, 2 to {n} machines"));
        }
        if !extra.is_empty() {
            let more = p.layout(extra.join(" · "), FontId::new(18.0 * u, FontFamily::Proportional), DIM, info_w);
            p.galley(pos2(left, y), more, DIM);
        }
        self.paint_strip(ui, p, l, dt, clicks);
        // The preview's own status along the foot of the info column.
        let status = match (view.progress, view.no_preview) {
            (Some(f), _) => Some(format!("Preparing the preview… {:.0}%", f * 100.0)),
            (None, Some(why)) if why.contains(" not in ") => {
                Some("No preview: chips missing from the ROM folder".to_string())
            }
            (None, Some(why)) => Some(format!("No preview: {why}")),
            _ => None,
        };
        if let Some(text) = status {
            let at = pos2(left, l.bar.top() - 18.0 * u);
            p.with_clip_rect(Rect::from_min_max(pos2(left, at.y - 30.0 * u), pos2(left + info_w, l.bar.top()))).text(
                at,
                Align2::LEFT_BOTTOM,
                text,
                FontId::new(18.0 * u, FontFamily::Proportional),
                theme::TEXT_DIM,
            );
        }
    }

    /// The posters at the right: the selected game large, the next ones
    /// beside it (round the list), a step sliding them along.
    fn paint_strip(&mut self, ui: &mut Ui, p: &Painter, l: &Layout, dt: f32, clicks: &mut Vec<Click>) {
        let u = l.u;
        let n = self.carousel.len();
        let Some(at) = self.index() else { return };
        let info_w = (520.0 * u).min(l.body.width() * 0.45);
        let x0 = l.body.left() + info_w + 32.0 * u;
        let cy = l.body.center().y;
        let (small, big) = (vec2(168.0, 252.0) * u, vec2(236.0, 354.0) * u);
        let gap = 26.0 * u;
        let room = ((l.screen.right() - x0) / (small.x + gap)).ceil() as usize + 1;
        let count = room.clamp(5.min(n), n);
        let clip = Rect::from_min_max(pos2(x0 - 12.0 * u, l.body.top()), l.screen.max);
        let pc = p.with_clip_rect(clip);
        // The wheel steps through them.
        let strip = Rect::from_min_max(pos2(x0, l.body.top()), pos2(l.screen.right(), l.body.bottom()));
        if ui.rect_contains_pointer(strip) && !self.page_open() {
            let d = ui.input(|i| i.smooth_scroll_delta);
            self.wheel += d.y + d.x;
            while self.wheel >= 40.0 {
                self.wheel -= 40.0;
                self.step(-1);
            }
            while self.wheel <= -40.0 {
                self.wheel += 40.0;
                self.step(1);
            }
        }
        let first = if n > count && n >= 2 { -1 } else { 0 };
        let order: Vec<(i32, &'static romset::Game)> =
            (first..count as i32).map(|k| (k, self.carousel[(at as i32 + k).rem_euclid(n as i32) as usize])).collect();
        let sizes: Vec<Vec2> = order
            .iter()
            .map(|(_, g)| {
                let t = self.grow.entry(g.id).or_insert(if self.cur.map(|c| c.id) == Some(g.id) { 1.0 } else { 0.0 });
                small + (big - small) * *t
            })
            .collect();
        // Their left edges: the selected one's at `x0`, moved by the slide.
        let k0 = order.iter().position(|(k, _)| *k == 0).unwrap_or(0);
        let mut xs = vec![0.0; order.len()];
        xs[k0] = x0 + self.slide * (small.x + gap);
        for i in k0 + 1..order.len() {
            xs[i] = xs[i - 1] + sizes[i - 1].x + gap;
        }
        for i in (0..k0).rev() {
            xs[i] = xs[i + 1] - gap - sizes[i].x;
        }
        for (i, (k, game)) in order.iter().enumerate() {
            let selected = *k == 0;
            let target = if selected { 1.0 } else { 0.0 };
            let grow = self.grow.entry(game.id).or_insert(target);
            *grow += (target - *grow) * ease(dt, 16.0);
            if (target - *grow).abs() < 0.01 {
                *grow = target;
            }
            let t = *grow;
            let r = Rect::from_min_size(pos2(xs[i], cy - sizes[i].y / 2.0), sizes[i]);
            if r.right() < clip.left() || r.left() > clip.right() {
                continue;
            }
            let alpha = match *k {
                -1 => (self.slide.max(0.0) * 0.8).min(0.8),
                0 => 1.0,
                1..=3 => 0.8,
                _ => 0.45,
            };
            let radius = 14.0 * u;
            if selected {
                pc.add(
                    egui::epaint::RectShape::filled(
                        r,
                        radius,
                        Color32::from_rgba_unmultiplied(34, 229, 255, (140.0 * t) as u8),
                    )
                    .with_blur_width(40.0 * u),
                );
            }
            pc.add(
                egui::epaint::RectShape::filled(
                    r.translate(vec2(0.0, 14.0 * u)),
                    radius,
                    Color32::from_black_alpha((140.0 * alpha) as u8),
                )
                .with_blur_width(34.0 * u),
            );
            let tint = Color32::from_white_alpha((alpha * 255.0) as u8);
            match self.posters.get(game.id) {
                Some(tex) => {
                    pc.add(egui::epaint::RectShape::filled(r, radius, tint).with_texture(tex.id(), full_uv()));
                }
                None => {
                    pc.rect_filled(r, radius, PANEL.gamma_multiply(alpha));
                }
            }
            if selected {
                pc.rect_stroke(r, radius, Stroke::new(4.0 * u * t, CYAN), StrokeKind::Outside);
            }
            if *k >= 0 && r.intersects(clip) {
                let id = if selected { selected_card() } else { Id::unique(("card", game.id)) };
                let (resp, hit) = control(&mut self.focus, ui, id, r.intersect(clip));
                if resp.hovered() && !selected {
                    pc.rect_stroke(
                        r,
                        radius,
                        Stroke::new(2.0 * u, Color32::from_white_alpha(120)),
                        StrokeKind::Outside,
                    );
                }
                if self.focus.is_on(id) && !selected {
                    theme::focus_ring(&pc, r, u, 14.0);
                }
                if hit {
                    clicks.push(Click::Game(game, resp.clicked()));
                }
            }
        }
        // The selected card's focus is the ring it wears (the focus ring is
        // the same cyan, drawn brighter while the pad is in use).
        if self.focus.is_on(selected_card()) {
            if let Some((i, _)) = order.iter().enumerate().find(|(_, (k, _))| *k == 0) {
                let r = Rect::from_min_size(pos2(xs[i], cy - sizes[i].y / 2.0), sizes[i]);
                theme::focus_ring(&pc, r, u, 14.0);
            }
        }
    }

    /// The grid: the filters and sort, then the posters.
    fn paint_grid(&mut self, ui: &mut Ui, p: &Painter, l: &Layout, view: &View, clicks: &mut Vec<Click>) {
        let _ = view;
        let u = l.u;
        // The filters.
        let font = FontId::new(18.0 * u, theme::semibold());
        let row_y = l.body.top() + 6.0 * u;
        let h = 42.0 * u;
        let wide: Vec<(Filter, f32)> = self
            .chips
            .iter()
            .map(|f| (*f, p.layout_no_wrap(f.label().to_string(), font.clone(), FG).size().x + 36.0 * u))
            .collect();
        let sort_label = self.sort.label();
        let sort_w = p.layout_no_wrap(sort_label.to_string(), font.clone(), FG).size().x + 62.0 * u;
        let chips_w = wide.iter().map(|c| c.1 + 10.0 * u).sum::<f32>();
        let labels = chips_w + sort_w + 160.0 * u < l.body.width();
        let mut x = l.body.left();
        if labels {
            let r = p.text(
                pos2(x, row_y + h / 2.0),
                Align2::LEFT_CENTER,
                "Show",
                FontId::new(18.0 * u, FontFamily::Proportional),
                theme::TEXT_DIM,
            );
            x = r.right() + 14.0 * u;
        }
        for (f, w) in wide {
            let r = Rect::from_min_size(pos2(x, row_y), vec2(w, h));
            let id = button_id(f.label());
            let (resp, hit) = control(&mut self.focus, ui, id, r);
            let on = f == self.filter;
            if on {
                p.rect_filled(r, 999.0, CYAN);
            } else {
                p.rect_filled(r, 999.0, Color32::from_white_alpha(if resp.hovered() { 28 } else { 15 }));
                p.rect_stroke(r, 999.0, Stroke::new(1.0 * u, Color32::from_white_alpha(51)), StrokeKind::Inside);
            }
            p.text(r.center(), Align2::CENTER_CENTER, f.label(), font.clone(), if on { theme::INK } else { FG });
            if self.focus.is_on(id) {
                theme::focus_ring(p, r, u, 21.0);
            }
            if hit {
                clicks.push(Click::Filter(f));
            }
            x = r.right() + 10.0 * u;
        }
        let sr = Rect::from_min_size(pos2(l.body.right() - sort_w, row_y), vec2(sort_w, h));
        if labels {
            p.text(
                pos2(sr.left() - 14.0 * u, row_y + h / 2.0),
                Align2::RIGHT_CENTER,
                "Sort",
                FontId::new(18.0 * u, FontFamily::Proportional),
                theme::TEXT_DIM,
            );
        }
        let id = button_id("sort");
        let (resp, hit) = control(&mut self.focus, ui, id, sr);
        p.rect_filled(sr, 10.0 * u, Color32::from_white_alpha(if resp.hovered() { 28 } else { 15 }));
        p.rect_stroke(sr, 10.0 * u, Stroke::new(1.0 * u, Color32::from_white_alpha(51)), StrokeKind::Inside);
        p.text(pos2(sr.left() + 16.0 * u, sr.center().y), Align2::LEFT_CENTER, sort_label, font, FG);
        // (Its down arrow is drawn: the fonts have no ▾.)
        let tip = pos2(sr.right() - 22.0 * u, sr.center().y + 4.0 * u);
        let arrow = vec![tip + vec2(-7.0, -4.0) * u, tip + vec2(7.0, -4.0) * u, tip + vec2(0.0, 4.0) * u];
        p.add(egui::Shape::convex_polygon(arrow, FG, Stroke::NONE));
        if self.focus.is_on(id) {
            theme::focus_ring(p, sr, u, 10.0);
        }
        if hit {
            clicks.push(Click::Sort);
        }
        // The posters, scrolling.
        let area = Rect::from_min_max(
            pos2(l.body.left() - 10.0 * u, l.body.top() + 62.0 * u),
            pos2(l.body.right() + 10.0 * u, l.bar.top() - 6.0 * u),
        );
        if self.grid.is_empty() {
            p.text(
                area.center(),
                Align2::CENTER_CENTER,
                "No games here",
                FontId::new(22.0 * u, theme::semibold()),
                theme::TEXT_DIM,
            );
            return;
        }
        let games = self.grid.clone();
        let sel = self.cur.map(|c| c.id);
        let gap = 22.0 * u;
        let inner_w = l.body.width();
        let cols = (((inner_w + gap) / (170.0 * u + gap)).floor() as usize).clamp(2, 12);
        let tw = (inner_w - gap * (cols - 1) as f32) / cols as f32;
        let (ph, label_h) = (tw * 1.5, 62.0 * u);
        let th = ph + label_h;
        let rows = games.len().div_ceil(cols);
        let total = rows as f32 * th + (rows - 1) as f32 * gap + 20.0 * u;
        let name_font = FontId::new(18.0 * u, theme::semibold());
        let genre_font = FontId::new(18.0 * u, FontFamily::Proportional);
        let (posters, focus) = (&self.posters, &mut self.focus);
        ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let (_, space) = ui.allocate_space(vec2(area.width(), total));
                let origin = space.min + vec2(10.0 * u, 8.0 * u);
                let p = ui.painter().clone();
                for (i, game) in games.iter().enumerate() {
                    let (col, row) = (i % cols, i / cols);
                    let r = Rect::from_min_size(
                        origin + vec2(col as f32 * (tw + gap), row as f32 * (th + gap)),
                        vec2(tw, th),
                    );
                    let pr = Rect::from_min_size(r.min, vec2(tw, ph));
                    let id = tile_id(game);
                    let (resp, hit) = control(focus, ui, id, r);
                    if hit {
                        clicks.push(Click::Game(game, resp.clicked()));
                    }
                    if !r.expand(20.0 * u).intersects(ui.clip_rect()) {
                        continue;
                    }
                    let selected = sel == Some(game.id);
                    match posters.get(game.id) {
                        Some(tex) => {
                            p.add(
                                egui::epaint::RectShape::filled(pr, 12.0 * u, Color32::WHITE)
                                    .with_texture(tex.id(), full_uv()),
                            );
                        }
                        None => {
                            p.rect_filled(pr, 12.0 * u, PANEL);
                        }
                    }
                    if focus.is_on(id) {
                        theme::focus_ring(&p, pr, u, 12.0);
                    } else if selected {
                        p.add(
                            egui::epaint::RectShape::filled(
                                pr,
                                12.0 * u,
                                Color32::from_rgba_unmultiplied(34, 229, 255, 70),
                            )
                            .with_blur_width(30.0 * u),
                        );
                        p.rect_stroke(pr, 12.0 * u, Stroke::new(4.0 * u, CYAN), StrokeKind::Outside);
                    } else if resp.hovered() {
                        p.rect_stroke(
                            pr,
                            12.0 * u,
                            Stroke::new(2.0 * u, Color32::from_white_alpha(110)),
                            StrokeKind::Outside,
                        );
                    }
                    let name = fit_text(&p, game.title, &name_font, tw - 8.0 * u);
                    p.text(
                        pos2(r.left() + 4.0 * u, pr.bottom() + 8.0 * u),
                        Align2::LEFT_TOP,
                        name,
                        name_font.clone(),
                        FG,
                    );
                    let versions = self.versions_n.get(game.id).copied().unwrap_or(1);
                    let second = if versions > 1 { format!("{versions} versions") } else { game.genre.to_string() };
                    let genre = fit_text(&p, &second, &genre_font, tw - 8.0 * u);
                    p.text(
                        pos2(r.left() + 4.0 * u, pr.bottom() + 8.0 * u + 28.0 * u),
                        Align2::LEFT_TOP,
                        genre,
                        genre_font.clone(),
                        theme::TEXT_DIM,
                    );
                }
            });
        });
    }

    /// The home screen's hints: Choose, Open, Continue, View (the grid adds
    /// the filter), System settings and Quit, the pad's buttons or the keys by what
    /// was used last.
    fn home_hints(&self) -> Vec<Hint> {
        let grid = self.mode == ViewMode::Grid;
        // Text, not the four-way-arrow glyph: none of the bundled fonts has it.
        let choose = "D-pad";
        let hint = |chip, label: &str, click| Hint { chip, label: label.to_string(), click };
        if self.pad_used {
            let mut h = vec![
                hint(Chip::Key(choose), "Choose", None),
                hint(Chip::A, "Open", Some(Click::OpenPage)),
                hint(Chip::Y, "Continue", Some(Click::Continue)),
                hint(Chip::Key("LB RB"), "View", Some(Click::View(self.mode.other()))),
            ];
            if grid {
                h.push(hint(Chip::Key("LT RT"), "Filter", Some(Click::NextFilter)));
            }
            h.push(hint(Chip::Menu, "System settings", Some(Click::Settings)));
            h.push(hint(Chip::B, "Quit", Some(Click::Quit)));
            h
        } else {
            let mut h = vec![
                hint(Chip::Key("← → ↑ ↓"), "Choose", None),
                hint(Chip::Key("Enter"), "Open", Some(Click::OpenPage)),
                hint(Chip::Key("C"), "Continue", Some(Click::Continue)),
                hint(Chip::Key("Q E"), "View", Some(Click::View(self.mode.other()))),
            ];
            if grid {
                h.push(hint(Chip::Key("[ ]"), "Filter", Some(Click::NextFilter)));
            }
            h.push(hint(Chip::Key("Tab"), "System settings", Some(Click::Settings)));
            h.push(hint(Chip::Key("Esc"), "Quit", Some(Click::Quit)));
            h
        }
    }

    /// The hint bar along the foot: what each button does, each one
    /// clickable. `count` ("N of M games") is the grid's, at the right.
    #[allow(clippy::too_many_arguments)]
    fn paint_bar(
        ui: &mut Ui,
        p: &Painter,
        l: &Layout,
        view: &View,
        focus: &mut Focus,
        hints: Vec<Hint>,
        count: Option<(usize, usize)>,
        clicks: &mut Vec<Click>,
    ) {
        let u = l.u;
        p.rect_filled(l.bar, 0.0, Color32::from_rgba_unmultiplied(4, 6, 11, 230));
        p.hline(l.bar.x_range(), l.bar.top(), Stroke::new(1.0 * u, Color32::from_white_alpha(15)));
        let font = FontId::new(16.0 * u, FontFamily::Proportional);
        let d = theme::CHIP_LARGE;
        let mut x = l.top.left();
        let y = l.bar.center().y;
        for (i, Hint { chip, label, click }) in hints.into_iter().enumerate() {
            let cw = theme::chip_width_at(p, chip, u, d);
            let g = p.layout_no_wrap(label, font.clone(), theme::TEXT_2);
            let r = Rect::from_min_size(pos2(x, y - 19.0 * u), vec2(cw + 8.0 * u + g.size().x + 18.0 * u, 38.0 * u));
            let dim = matches!(click, Some(Click::Continue)) && view.cont.is_none();
            let shown = Color32::from_white_alpha(if dim { 90 } else { 255 });
            if let Some(c) = click {
                let id = Id::unique(("hint", i));
                let (resp, hit) = control(focus, ui, id, r);
                if resp.hovered() && !dim {
                    p.rect_filled(r, 10.0 * u, Color32::from_white_alpha(20));
                }
                if focus.is_on(id) {
                    theme::focus_ring(p, r, u, 10.0);
                }
                if hit && !dim {
                    clicks.push(c);
                }
            }
            theme::paint_chip_at(p, chip, pos2(r.left() + 6.0 * u, y), u, d);
            p.galley(pos2(r.left() + 6.0 * u + cw + 8.0 * u, y - g.size().y / 2.0), g, shown);
            x = r.right() + 8.0 * u;
        }
        if let Some((shown, of)) = count {
            p.text(
                pos2(l.top.right(), y),
                Align2::RIGHT_CENTER,
                format!("{shown} of {of} games"),
                FontId::new(16.0 * u, FontFamily::Proportional),
                theme::TEXT_DIM,
            );
        }
    }

    /// The poster of `set` once it is loaded (the Players screen shows it).
    pub fn poster(&self, set: &str) -> Option<egui::TextureId> {
        self.posters.get(set).map(|t| t.id())
    }

    /// Show the network link play panel (the player made its lobby or its join).
    pub fn show_net(&mut self, page: crate::netpage::NetPage) {
        self.net_page = Some(page);
    }

    /// No games: say where reModel looked, and let another folder be typed in.
    fn empty(&mut self, ctx: &egui::Context, l: &Layout, view: &View, actions: &mut Vec<Action>) {
        let u = l.u;
        egui::Area::new(Id::unique("launcher empty")).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.set_max_width(700.0 * u);
            ui.vertical_centered(|ui| {
                if let Some(logo) = &self.logo {
                    let [w, h] = logo.size();
                    let lh = 90.0 * u;
                    ui.add(egui::Image::new((logo.id(), vec2(lh * w as f32 / h as f32, lh))));
                }
                ui.add_space(24.0 * u);
                ui.label(theme::heading(ctx, "No games found", 34.0));
                ui.add_space(10.0 * u);
                ui.label(
                    egui::RichText::new(
                        "reModel looks for MAME sets (zips or folders, e.g. topskatr.zip) in this folder:",
                    )
                    .size(18.0 * u)
                    .color(theme::TEXT_2),
                );
                ui.add_space(14.0 * u);
                ui.horizontal(|ui| {
                    let edit = egui::TextEdit::singleline(&mut self.rom_text)
                        .font(FontId::new(18.0 * u, FontFamily::Monospace))
                        .desired_width(480.0 * u);
                    let r = ui.add(edit);
                    let scan = ui.add(egui::Button::new(egui::RichText::new("Scan").size(18.0 * u)));
                    if scan.clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                        actions.push(Action::Rescan(self.rom_text.trim().to_string()));
                    }
                });
                if view.unsupported > 0 {
                    ui.add_space(8.0 * u);
                    ui.label(
                        egui::RichText::new(format!("{} other sets there aren't supported yet", view.unsupported))
                            .size(18.0 * u)
                            .color(DIM),
                    );
                }
                ui.add_space(20.0 * u);
                if ui.add(egui::Button::new(egui::RichText::new("Quit").size(18.0 * u))).clicked() {
                    actions.push(Action::Quit);
                }
            });
        });
    }
}

/// A horizontal gradient over `r`.
fn hgradient(p: &Painter, r: Rect, left: Color32, right: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(r.left_top(), left);
    mesh.colored_vertex(r.right_top(), right);
    mesh.colored_vertex(r.left_bottom(), left);
    mesh.colored_vertex(r.right_bottom(), right);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    p.add(egui::Shape::mesh(mesh));
}

/// A vertical gradient over `r`.
fn vgradient(p: &Painter, r: Rect, top: Color32, bottom: Color32) {
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
    use romset::Kind;

    fn games(ids: &[&str]) -> Vec<&'static romset::Game> {
        ids.iter().map(|id| romset::games::find(id).unwrap()).collect()
    }

    const SET: [&str; 8] = ["topskatr", "hotd", "vf2", "stcc", "srallyc", "vcop", "doa", "sgt24h"];

    fn view(cont: Option<Continue>, menu_open: bool) -> View<'static> {
        View {
            preview: None,
            progress: None,
            no_preview: None,
            resume: None,
            unsupported: 0,
            cont,
            menu_open,
            capturing: false,
            settings: SettingsLine::default(),
        }
    }

    /// A launcher drawn headless, a frame at a time, the way `ui` is called.
    struct Rig {
        ctx: egui::Context,
        l: Launcher,
        size: Vec2,
        cont: Option<Continue>,
        resume: Option<SystemTime>,
        menu_open: bool,
        actions: Vec<Action>,
        shapes: Vec<egui::epaint::ClippedShape>,
        /// Seconds of the frames so far (a tenth each).
        time: f64,
    }

    impl Rig {
        fn new(ids: &[&str], recent: &[&str]) -> Rig {
            let ctx = egui::Context::default();
            theme::install(&ctx);
            let mut l = Launcher { lan_off: true, ..Default::default() };
            let recent: Vec<String> = recent.iter().map(|s| s.to_string()).collect();
            l.set_games(&games(ids), &recent);
            let mut rig = Rig {
                ctx,
                l,
                size: vec2(1280.0, 720.0),
                cont: None,
                resume: None,
                menu_open: false,
                actions: Vec::new(),
                shapes: Vec::new(),
                time: 0.0,
            };
            // One frame to lay it out, one for the focus to settle in.
            rig.frame(&[], Vec::new());
            rig.frame(&[], Vec::new());
            rig.actions.clear();
            rig
        }

        fn frame(&mut self, navs: &[Nav], events: Vec<egui::Event>) {
            let mut view = view(self.cont, self.menu_open);
            view.resume = self.resume;
            self.time += 0.1;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            theme::sync(&self.ctx, self.size.y);
            self.l.last_frame = Some(Instant::now() - Duration::from_millis(100));
            let Rig { ctx, l, actions, .. } = self;
            let mut out = ctx.run_ui(input, |ui| {
                actions.extend(l.ui(ui.ctx(), navs.to_vec(), &view));
            });
            out.textures_delta.clear();
            self.shapes = out.shapes;
        }

        /// `n` presses of `nav`, a frame each, then frames for the scrolling
        /// and sliding to settle.
        fn press(&mut self, n: usize, nav: Nav) -> &mut Rig {
            for _ in 0..n {
                self.frame(&[nav], Vec::new());
            }
            for _ in 0..6 {
                self.frame(&[], Vec::new());
            }
            self
        }

        fn key(&mut self, key: egui::Key) {
            let event = egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            };
            self.frame(&[], vec![event]);
            self.frame(&[], Vec::new());
        }

        /// The text drawn last frame, and its sizes in points.
        fn texts(&self) -> Vec<(String, f32)> {
            self.shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Text(t) => {
                        let size = t.galley.job.sections.first().map_or(0.0, |s| s.format.font_id.size);
                        Some((t.galley.text().to_string(), size))
                    }
                    _ => None,
                })
                .collect()
        }

        fn shows(&self, needle: &str) -> bool {
            self.texts().iter().any(|(t, _)| t.contains(needle))
        }

        fn selected(&self) -> &'static str {
            self.l.selected().unwrap().id
        }

        fn played(&self) -> Vec<(String, Start)> {
            self.actions
                .iter()
                .filter_map(|a| match a {
                    Action::Play { set, from } => Some((set.clone(), *from)),
                    _ => None,
                })
                .collect()
        }

        /// The focus is on a tile of the grid.
        fn on_a_tile(&self) -> bool {
            self.l.focus.focused().is_some_and(|id| self.l.grid.iter().any(|g| tile_id(g) == id))
        }
    }

    fn continuing(set: &str, from: Start) -> Option<Continue> {
        Some(Continue {
            game: romset::games::find(set).unwrap(),
            from,
            saved: Some(SystemTime::now() - Duration::from_secs(12 * 60)),
        })
    }

    #[test]
    fn the_symbols_the_launcher_draws_have_glyphs() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        // Fonts take effect with the first frame.
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        for (family, chars) in [(theme::bold(), "←→↑↓·…"), (theme::semibold(), "·…"), (FontFamily::Proportional, "·…")]
        {
            let font = FontId::new(18.0, family);
            for c in chars.chars() {
                assert!(ctx.fonts_mut(|f| f.has_glyph(&font, c)), "no glyph for {c}");
            }
        }
    }

    #[test]
    fn carousel_steps_round_the_games() {
        let mut r = Rig::new(&SET, &["stcc", "hotd"]);
        // The played first (latest first), then A to Z by title.
        assert_eq!(r.l.carousel_ids()[..3], ["stcc", "hotd", "doa"]);
        assert_eq!(r.selected(), "stcc");
        r.press(1, Nav::Right);
        assert_eq!(r.selected(), "hotd");
        r.press(1, Nav::Left).press(1, Nav::Left);
        assert_eq!(r.selected(), *r.l.carousel_ids().last().unwrap(), "left from the first goes round");
        r.press(1, Nav::Right);
        assert_eq!(r.selected(), "stcc", "and right from the last");
        assert!(r.l.wants_preview());
    }

    #[test]
    fn y_goes_back_to_the_last_game_at_its_latest_state() {
        let mut r = Rig::new(&SET, &["stcc"]);
        r.press(2, Nav::Right);
        assert_eq!(r.selected(), "srallyc");
        r.press(1, Nav::Continue);
        assert!(r.played().is_empty(), "nothing to continue");
        r.cont = continuing("stcc", Start::Slot(2));
        r.press(1, Nav::Continue);
        assert_eq!(r.played(), [("stcc".to_string(), Start::Slot(2))], "not the game selected");
        // The pill says what and when.
        assert!(r.shows("CONTINUE") && r.shows("slot 2, 12 minutes ago"), "{:?}", r.texts());
        r.key(egui::Key::C);
        assert_eq!(r.played().len(), 2, "the keyboard's C does the same");
        // In the grid it is one line in the top bar.
        r.l.set_mode(ViewMode::Grid);
        r.frame(&[], Vec::new());
        assert!(r.shows("Continue") && r.shows("12 minutes ago") && !r.shows("CONTINUE"), "{:?}", r.texts());
    }

    /// The page's level, if it is open.
    fn level(r: &Rig) -> Option<page::Level> {
        r.l.page.as_ref().map(|p| p.level)
    }

    /// The page's focus is on the row `key` of `level`.
    fn on_row(r: &Rig, level: page::Level, key: &str) -> bool {
        r.l.page.as_ref().and_then(|p| p.focus.focused()) == Some(page::row_id(level, key))
    }

    #[test]
    fn a_opens_the_page_with_play_focused_and_a_again_plays() {
        let mut r = Rig::new(&SET, &["stcc"]);
        r.press(1, Nav::Accept);
        assert!(r.played().is_empty(), "A on a poster opens the page, it does not play");
        assert_eq!(level(&r), Some(page::Level::Main));
        assert!(on_row(&r, page::Level::Main, "play"));
        assert!(r.shows("PLAY") || r.shows("Play"), "{:?}", r.texts());
        r.press(1, Nav::Accept);
        assert_eq!(r.played(), [("stcc".to_string(), Start::New)]);
    }

    #[test]
    fn a_click_selects_then_opens_and_two_quick_clicks_play() {
        let mut r = Rig::new(&SET, &["stcc"]);
        let hotd = romset::games::find("hotd").unwrap();
        // A click on another poster selects it; on the selected one, opens its page.
        r.l.apply(Click::Game(hotd, false), &view(None, false), &mut r.actions);
        assert_eq!((r.selected(), level(&r)), ("hotd", None));
        r.l.apply(Click::Game(hotd, false), &view(None, false), &mut r.actions);
        assert_eq!(level(&r), Some(page::Level::Main));
        assert!(r.actions.is_empty());
        r.l.page = None;
        // Two quick mouse clicks on a poster that is not selected: select, then play.
        let vcop = romset::games::find("vcop").unwrap();
        r.l.apply(Click::Game(vcop, true), &view(None, false), &mut r.actions);
        assert!(r.actions.is_empty() && level(&r).is_none());
        r.l.apply(Click::Game(vcop, true), &view(None, false), &mut r.actions);
        assert_eq!(r.played(), [("vcop".to_string(), Start::New)]);
        // On the selected one the first click opens the page, and a press soon after plays.
        r.actions.clear();
        r.l.apply(Click::Game(vcop, true), &view(None, false), &mut r.actions);
        assert_eq!(level(&r), Some(page::Level::Main));
        let press = vec![
            egui::Event::PointerMoved(pos2(300.0, 300.0)),
            egui::Event::PointerButton {
                pos: pos2(300.0, 300.0),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
        ];
        r.frame(&[], press);
        assert_eq!(r.played(), [("vcop".to_string(), Start::New)], "the second press of the double click");
        // Slowly, it is just a click on the page.
        r.actions.clear();
        r.l.page = None;
        r.l.last_click = None;
        r.l.open_page(page::Level::Main, true);
        r.l.page.as_mut().unwrap().dbl_until = Some(Instant::now() - Duration::from_millis(1));
        let press = vec![egui::Event::PointerButton {
            pos: pos2(40.0, 600.0),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        }];
        r.frame(&[], press);
        assert!(r.played().is_empty());
    }

    #[test]
    fn the_card_has_no_buttons_and_says_one_thing_about_the_game() {
        let mut r = Rig::new(&["daytona", "dayton93", "daytonat", "topskatr", "hotd"], &["hotd"]);
        while r.l.card().unwrap().id != "daytona" {
            r.press(1, Nav::Right);
        }
        for gone in ["Saves", "Settings", "Link play", "Version", "Play"] {
            assert!(!r.texts().iter().any(|(t, _)| t == gone), "{gone}: {:?}", r.texts());
        }
        assert!(r.shows("Not played yet"), "{:?}", r.texts());
        // Under it, dim: the versions and the linking.
        assert!(r.shows("3 versions · Link play, 2 to 8 machines"), "{:?}", r.texts());
        while r.l.card().unwrap().id != "hotd" {
            r.press(1, Nav::Right);
        }
        assert!(r.shows("Played before") && !r.shows("versions"), "{:?}", r.texts());
        // The resume state's age, when there is one.
        r.resume = Some(SystemTime::now() - Duration::from_secs(2 * 3600 + 60));
        r.frame(&[], Vec::new());
        assert!(r.shows("You left off 2 hours ago"), "{:?}", r.texts());
    }

    #[test]
    fn the_saves_level_lists_the_slots_and_a_card_starts_the_game_from_it() {
        let mut r = Rig::new(&SET, &["stcc"]);
        r.press(1, Nav::Saves);
        r.frame(&[], Vec::new());
        assert_eq!(level(&r), Some(page::Level::Saves), "X opens the page at Saves");
        assert!(r.shows("Where you left off") && r.shows("Quick save") && r.shows("Slot 4"), "{:?}", r.texts());
        // Empty slots load nothing.
        r.press(1, Nav::Accept);
        assert!(r.played().is_empty() && level(&r) == Some(page::Level::Saves));
        // Give the resume state and slot 2 a state this build loads.
        let saved = |t: u64| SlotInfo {
            time: SystemTime::UNIX_EPOCH + Duration::from_secs(t),
            made_by: None,
            loads: true,
            thumb: None,
        };
        let page = r.l.page.as_mut().unwrap();
        page.saves[0].info = Some(saved(100));
        page.saves[3].info = Some(saved(200));
        r.press(1, Nav::Accept);
        assert_eq!(r.played(), [("stcc".to_string(), Start::Resume)]);
        // Slot 2 is three cards down.
        r.actions.clear();
        r.press(3, Nav::Down).press(1, Nav::Accept);
        assert_eq!(r.played(), [("stcc".to_string(), Start::Slot(2))]);
        // B goes back to the main menu with Saves focused, then to the games.
        r.actions.clear();
        r.press(1, Nav::Back);
        assert_eq!(level(&r), Some(page::Level::Main));
        assert!(on_row(&r, page::Level::Main, "saves"));
        r.press(1, Nav::Back);
        assert!(level(&r).is_none() && r.played().is_empty());
        // The keyboard's S opens it again.
        r.key(egui::Key::S);
        assert_eq!(level(&r), Some(page::Level::Saves));
    }

    #[test]
    fn b_goes_back_level_by_level() {
        let mut r = Rig::new(&["daytona", "dayton93", "hotd"], &[]);
        while r.l.card().unwrap().id != "daytona" {
            r.press(1, Nav::Right);
        }
        r.press(1, Nav::Accept);
        assert_eq!(level(&r), Some(page::Level::Main));
        // Down to Version, in.
        r.press(1, Nav::Down).press(1, Nav::Accept);
        assert_eq!(level(&r), Some(page::Level::Version));
        r.press(1, Nav::Back);
        assert_eq!(level(&r), Some(page::Level::Main));
        assert!(on_row(&r, page::Level::Main, "version"), "back to the row that opened it");
        r.press(1, Nav::Back);
        assert!(level(&r).is_none(), "and out to the games");
        assert_eq!(r.l.focus.focused(), Some(selected_card()), "the poster has the focus again");
        // The keyboard's Esc does the same.
        r.press(1, Nav::Accept);
        r.key(egui::Key::Escape);
        assert!(level(&r).is_none());
    }

    #[test]
    fn the_page_is_reached_from_the_keyboard_and_the_pad_and_never_loses_the_focus() {
        let mut r = Rig::new(&["daytona", "dayton93", "hotd"], &[]);
        while r.l.card().unwrap().id != "daytona" {
            r.press(1, Nav::Right);
        }
        r.key(egui::Key::Enter);
        assert_eq!(level(&r), Some(page::Level::Main));
        // A long wander over every level: the focus is always on something.
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..400 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let nav =
                [Nav::Up, Nav::Down, Nav::Left, Nav::Right, Nav::Accept, Nav::Back, Nav::Accept][(x % 7) as usize];
            r.frame(&[nav], Vec::new());
            r.actions.clear();
            r.frame(&[], Vec::new());
            let on = match &r.l.page {
                Some(p) => p.focus.focused().is_some() || level(&r) == Some(page::Level::About),
                None => r.l.focus.focused().is_some(),
            };
            assert!(on, "the focus was lost at {:?}", level(&r));
        }
    }
    #[test]
    fn start_opens_the_settings_and_esc_or_start_close_them() {
        let mut r = Rig::new(&SET, &[]);
        r.press(1, Nav::Start);
        assert!(r.actions.iter().any(|a| matches!(a, Action::Settings)));
        r.actions.clear();
        // With the menu open the launcher takes nothing but the way out.
        r.menu_open = true;
        let at = r.selected();
        r.press(3, Nav::Right);
        assert_eq!(r.selected(), at);
        r.press(1, Nav::Start);
        assert!(r.actions.iter().any(|a| matches!(a, Action::CloseSettings)));
        r.actions.clear();
        r.key(egui::Key::Escape);
        assert!(r.actions.iter().any(|a| matches!(a, Action::CloseSettings)));
        // Tab opens them from the keyboard.
        r.actions.clear();
        r.menu_open = false;
        r.key(egui::Key::Tab);
        assert!(r.actions.iter().any(|a| matches!(a, Action::Settings)));
        // And the hint bar says so.
        r.actions.clear();
        r.frame(&[], Vec::new());
        assert!(r.shows("System settings"), "{:?}", r.texts());
        // Esc on the home screen asks to quit (System settings, at Quit reModel).
        assert!(r.shows("Quit"), "{:?}", r.texts());
        r.key(egui::Key::Escape);
        assert!(r.actions.iter().any(|a| matches!(a, Action::AskQuit)), "{:?}", r.actions.len());
    }

    #[test]
    fn the_shoulder_buttons_switch_the_view_and_it_stays() {
        let mut r = Rig::new(&SET, &["vf2"]);
        assert_eq!(r.l.mode, ViewMode::Carousel);
        r.press(1, Nav::NextGroup);
        assert_eq!(r.l.mode, ViewMode::Grid);
        assert!(!r.l.wants_preview(), "the grid has no preview to run");
        assert_eq!(r.selected(), "vf2", "the same game");
        r.press(1, Nav::PrevGroup);
        assert_eq!(r.l.mode, ViewMode::Carousel);
        // A new list of games (back from a game) keeps the view and the game.
        r.l.set_mode(ViewMode::Grid);
        r.l.set_games(&games(&SET), &["vf2".to_string(), "hotd".to_string()]);
        assert_eq!((r.l.mode, r.l.selected().unwrap().id), (ViewMode::Grid, "vf2"));
        // Q and E, from the keyboard.
        r.key(egui::Key::Q);
        assert_eq!(r.l.mode, ViewMode::Carousel);
    }

    #[test]
    fn the_view_switch_is_clickable_and_pad_reachable() {
        let mut r = Rig::new(&SET, &[]);
        let view = view(None, false);
        r.l.apply(Click::View(ViewMode::Grid), &view, &mut r.actions);
        assert_eq!(r.l.mode, ViewMode::Grid);
        // Up from a poster reaches the filters, up again the top bar.
        r.frame(&[], Vec::new());
        r.frame(&[], Vec::new());
        r.press(3, Nav::Up);
        assert!(!r.on_a_tile());
    }

    #[test]
    fn the_grid_filters_by_kind_with_the_triggers() {
        let mut r = Rig::new(&SET, &["vf2"]);
        r.l.set_mode(ViewMode::Grid);
        r.frame(&[], Vec::new());
        assert_eq!(r.l.counts(), (8, 8));
        let chips: Vec<&str> = r.l.chips.iter().map(|f| f.label()).collect();
        assert_eq!(chips, ["All", "Recent", "Racing", "Fighting", "Shooting", "Sports"]);
        // RT steps on (Recent, then Racing), LT back.
        r.press(1, Nav::NextFilter);
        assert_eq!(r.l.filter, Filter::Recent);
        assert_eq!(r.l.grid_ids(), ["vf2"]);
        r.press(1, Nav::NextFilter);
        assert_eq!(r.l.filter, Filter::Kind(Kind::Racing));
        assert_eq!(r.l.grid_ids(), ["srallyc", "stcc", "sgt24h"], "A to Z: Sega Rally, Sega Touring, Super GT");
        assert_eq!(r.l.counts(), (3, 8));
        assert!(r.shows("3 of 8 games"), "{:?}", r.texts());
        assert!(r.l.selected().is_some_and(|g| g.kind == Kind::Racing), "the selection follows into the list");
        r.press(2, Nav::PrevFilter);
        assert_eq!(r.l.filter, Filter::All);
        r.press(1, Nav::PrevFilter);
        assert_eq!(r.l.filter, Filter::Kind(Kind::Sports), "round the other way");
        // The carousel doesn't filter.
        assert_eq!(r.l.carousel_ids().len(), 8);
        // The keyboard's [ and ].
        r.key(egui::Key::CloseBracket);
        assert_eq!(r.l.filter, Filter::All);
    }

    #[test]
    fn the_grid_sorts_recent_first_or_a_to_z() {
        let mut r = Rig::new(&SET, &["vcop", "doa"]);
        r.l.set_mode(ViewMode::Grid);
        assert_eq!(r.l.grid_ids()[..2], ["vcop", "doa"]);
        r.l.set_sort(Sort::AtoZ);
        let order = r.l.grid_ids();
        let mut sorted = order.clone();
        sorted.sort_by_key(|id| romset::games::find(id).unwrap().title);
        assert_eq!(order, sorted);
        // The sort button says which, and a click flips it.
        r.frame(&[], Vec::new());
        assert!(r.shows("A to Z"), "{:?}", r.texts());
        r.l.apply(Click::Sort, &view(None, false), &mut r.actions);
        assert_eq!(r.l.sort, Sort::Recent);
        assert_eq!(r.l.grid_ids()[..2], ["vcop", "doa"]);
    }

    #[test]
    fn the_pad_moves_over_the_grid_and_up_to_the_filters() {
        let mut r = Rig::new(&SET, &[]);
        r.l.set_mode(ViewMode::Grid);
        r.frame(&[], Vec::new());
        r.frame(&[], Vec::new());
        let order = r.l.grid_ids();
        assert_eq!(r.selected(), order[0]);
        r.press(1, Nav::Right);
        assert_eq!(r.selected(), order[1]);
        // Six to a row at 1280x720: down is the second row, under it.
        r.press(1, Nav::Down);
        assert_eq!(r.selected(), order[7]);
        r.press(1, Nav::Up);
        assert_eq!(r.selected(), order[1]);
        // Up again leaves the posters for the filters; the selection stays.
        r.press(1, Nav::Up);
        assert!(!r.on_a_tile());
        assert_eq!(r.selected(), order[1]);
        r.press(1, Nav::Down);
        assert!(r.on_a_tile());
        // A opens the selected one's page, and A again plays it.
        r.press(1, Nav::Accept);
        assert!(r.played().is_empty() && level(&r).is_some());
        r.press(1, Nav::Accept);
        assert_eq!(r.played(), [(order[1].to_string(), Start::New)]);
    }

    #[test]
    fn every_control_is_reachable_from_the_pad() {
        for mode in ViewMode::ALL {
            let mut r = Rig::new(&SET, &["stcc"]);
            r.cont = continuing("stcc", Start::Resume);
            r.l.set_mode(mode);
            r.frame(&[], Vec::new());
            r.frame(&[], Vec::new());
            let mut seen = std::collections::HashSet::new();
            // A long, repeatable wander over the d-pad.
            let mut x = 0x2545_f491_4f6c_dd1du64;
            for _ in 0..600 {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                let nav = [Nav::Up, Nav::Down, Nav::Left, Nav::Right][(x % 4) as usize];
                r.frame(&[nav], Vec::new());
                seen.extend(r.l.focus.focused());
            }
            let must: Vec<(&str, Id)> = match mode {
                ViewMode::Carousel => vec![
                    ("the pill", button_id("continue")),
                    ("Carousel", button_id("carousel")),
                    ("Grid", button_id("grid")),
                    ("the selected poster", selected_card()),
                    ("a hint", Id::unique(("hint", 1))),
                ],
                ViewMode::Grid => vec![
                    ("the pill", button_id("continue")),
                    ("Carousel", button_id("carousel")),
                    ("Grid", button_id("grid")),
                    ("a chip", button_id("Racing")),
                    ("Sort", button_id("sort")),
                    ("a hint", Id::unique(("hint", 1))),
                ],
            };
            for (what, id) in must {
                assert!(seen.contains(&id), "{mode:?}: the pad never reached {what}");
            }
        }
    }

    #[test]
    fn both_views_have_the_same_hints_and_the_grid_adds_the_filter() {
        for pad in [true, false] {
            let mut r = Rig::new(&SET, &["stcc"]);
            r.cont = continuing("stcc", Start::New);
            if pad {
                r.press(1, Nav::Right);
            } else {
                r.key(egui::Key::ArrowRight);
            }
            let hints = |r: &Rig| -> Vec<String> {
                ["Choose", "Open", "Play", "Saves", "Settings", "Link play", "Version", "Continue", "View", "Filter"]
                    .iter()
                    .filter(|h| r.texts().iter().any(|(t, _)| t == *h))
                    .map(|h| h.to_string())
                    .collect()
            };
            let carousel = hints(&r);
            assert_eq!(carousel, ["Choose", "Open", "Continue", "View"], "pad {pad}");
            // The keys' caps when the keyboard was last used, the pad's else.
            assert_eq!(r.shows("Enter"), !pad, "pad {pad}");
            assert_eq!(r.shows("LB RB"), pad, "pad {pad}");
            r.l.set_mode(ViewMode::Grid);
            r.frame(&[], Vec::new());
            let grid = hints(&r);
            let mut want = carousel.clone();
            want.push("Filter".into());
            assert_eq!(grid, want, "pad {pad}");
            assert_eq!(r.shows("LT RT"), pad, "pad {pad}");
        }
    }

    #[test]
    fn text_is_never_under_14_points_at_720p() {
        for mode in ViewMode::ALL {
            for cont in [None, continuing("stcc", Start::Resume)] {
                let mut r = Rig::new(&SET, &["stcc"]);
                r.cont = cont;
                r.l.set_mode(mode);
                for _ in 0..3 {
                    r.frame(&[], Vec::new());
                }
                let small: Vec<_> = r.texts().into_iter().filter(|(_, size)| *size < 13.99).collect();
                assert!(small.is_empty(), "{mode:?}: {small:?}");
                assert!(r.texts().len() > 10, "{mode:?}");
            }
        }
    }

    #[test]
    fn only_the_games_there_are_listed() {
        let mut r = Rig::new(&["hotd", "vcop", "topskatr"], &["hotd"]);
        r.l.set_mode(ViewMode::Grid);
        r.frame(&[], Vec::new());
        assert_eq!(r.l.counts(), (3, 3));
        let chips: Vec<&str> = r.l.chips.iter().map(|f| f.label()).collect();
        assert_eq!(chips, ["All", "Recent", "Shooting", "Sports"], "no Racing chip without a racing game");
        assert!(r.shows("3 of 3 games"));
        r.l.set_games(&games(&["hotd"]), &[]);
        assert_eq!(r.l.counts(), (1, 1));
        assert!(!r.l.grid_ids().contains(&"vcop"));
        // The count at the foot of the carousel's top bar.
        r.l.set_mode(ViewMode::Carousel);
        r.frame(&[], Vec::new());
        assert!(r.shows("1 game") && !r.shows("1 games"));
        // No games: the ROM folder prompt.
        r.l.set_games(&[], &[]);
        r.frame(&[], Vec::new());
        r.frame(&[], Vec::new());
        assert!(r.shows("NO GAMES FOUND"));
    }

    #[test]
    fn the_selection_survives_the_list_changing() {
        let mut r = Rig::new(&SET, &[]);
        r.l.select_set("vf2");
        assert_eq!(r.selected(), "vf2");
        let recent = vec!["hotd".to_string()];
        r.l.set_games(&games(&SET), &recent);
        assert_eq!(r.selected(), "vf2", "kept");
        r.l.set_games(&games(&["hotd", "doa"]), &recent);
        assert_eq!(r.selected(), "hotd", "else the first: the one played last");
    }

    #[test]
    fn posters_blur_to_a_small_copy() {
        let mut pixels = vec![Color32::from_rgb(200, 0, 0); 40 * 60 / 2];
        pixels.extend(vec![Color32::from_rgb(0, 0, 200); 40 * 60 / 2]);
        let img = egui::ColorImage::new([40, 60], pixels);
        let blur = blur_image(&img);
        assert_eq!(blur.width(), 24);
        assert_eq!(blur.height(), 36, "the poster's shape");
        assert!(blur.pixels[0].r() > 150 && blur.pixels[0].b() < 50, "red at the top");
        assert!(blur.pixels.last().unwrap().b() > 150, "blue at the foot");
    }

    #[test]
    fn layout_fits_every_shape() {
        for (w, h) in
            [(1280.0, 720.0), (1920.0, 1080.0), (3440.0, 1440.0), (1024.0, 768.0), (1920.0, 1200.0), (800.0, 600.0)]
        {
            let s = Rect::from_min_size(Pos2::ZERO, vec2(w, h));
            let l = layout(s);
            for r in [l.top, l.body, l.bar] {
                assert!(s.expand(0.5).contains_rect(r), "{w}x{h}: {r:?}");
            }
            assert!(l.top.bottom() < l.body.top() && l.body.bottom() <= l.bar.top() + 0.5, "{w}x{h}");
            assert!(l.body.height() > 400.0 * l.u * 0.8, "{w}x{h}: room for the posters");
            assert!(l.u >= theme::MIN_SCALE);
            assert!(s.contains_rect(l.hero()));
        }
        // At the design size the design's own numbers.
        let l = layout(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 720.0)));
        assert_eq!((l.u, l.top.left(), l.top.top(), l.bar.top()), (1.0, 48.0, 28.0, 656.0));
    }

    /// Headless: both views at 1080p, 4K (2x), ultrawide, 4:3 and small,
    /// every text on screen where it should be, the animations settling,
    /// and the keyboard moving it.
    #[test]
    fn draws_headless() {
        for mode in ViewMode::ALL {
            for (size, ppp) in [
                (vec2(1920.0, 1080.0), 1.0),
                (vec2(3840.0, 2160.0), 2.0),
                (vec2(3440.0, 1440.0), 1.0),
                (vec2(1024.0, 768.0), 1.0),
                (vec2(800.0, 600.0), 1.0),
            ] {
                let mut r = Rig::new(&SET, &["stcc"]);
                r.size = size / ppp;
                r.cont = continuing("stcc", Start::Slot(1));
                r.l.set_mode(mode);
                let before = r.selected();
                for _ in 0..12 {
                    r.frame(&[], Vec::new());
                }
                r.key(egui::Key::ArrowRight);
                for _ in 0..12 {
                    r.frame(&[], Vec::new());
                }
                assert!(!r.l.animating(), "{mode:?} {size:?}: settled");
                assert!(r.selected() != before || mode == ViewMode::Grid, "{mode:?} {size:?}: the key moved it");
                let screen = Rect::from_min_size(Pos2::ZERO, r.size);
                let l = layout(screen);
                for c in &r.shapes {
                    if let egui::Shape::Text(t) = &c.shape {
                        let b = t.visual_bounding_rect();
                        // The top bar and the hint bar are never cut off.
                        if c.clip_rect.intersects(b) && (b.center().y < l.top.bottom() || b.center().y > l.bar.top()) {
                            assert!(
                                screen.expand(1.0).contains_rect(b),
                                "{mode:?} {size:?}: {:?} at {b:?}",
                                t.galley.text()
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_empty_launcher_asks_for_a_rom_folder() {
        let mut r = Rig::new(&[], &[]);
        r.l.rom_text = "/roms".into();
        r.frame(&[], Vec::new());
        assert!(r.shows("NO GAMES FOUND"));
        assert!(r.texts().iter().all(|(_, size)| *size >= 17.99), "{:?}", r.texts());
    }

    fn versions(r: &Rig) -> Vec<&'static str> {
        let card = r.l.card().unwrap();
        r.l.versions_of(card).iter().map(|g| g.id).collect()
    }

    fn chosen(r: &Rig) -> Vec<(String, String)> {
        r.actions
            .iter()
            .filter_map(|a| match a {
                Action::Version { title, set } => Some((title.clone(), set.clone())),
                _ => None,
            })
            .collect()
    }

    /// Select the card `id`, by the carousel.
    fn go_to(r: &mut Rig, id: &str) {
        while r.l.card().unwrap().id != id {
            r.press(1, Nav::Right);
        }
    }

    #[test]
    fn a_title_with_versions_is_one_card_and_lists_what_is_installed() {
        let r = Rig::new(&["topskatr", "topskatrj", "topskatru", "hotd", "vcopa"], &[]);
        // vcopa's title (vcop) is not here, so it stands alone; topskatr's versions are one card.
        let mut cards = r.l.carousel_ids();
        cards.sort();
        assert_eq!(cards, ["hotd", "topskatr", "vcopa"]);
        assert_eq!(r.l.counts().1, 3);
        let mut r = r;
        go_to(&mut r, "topskatr");
        assert_eq!(versions(&r), ["topskatr", "topskatrj", "topskatru"], "the title first");
        assert_eq!(r.selected(), "topskatr");
        assert!(r.shows("3 versions"), "{:?}", r.texts());
        // The page's Version row opens the list, every version a radio row with the chosen one lit.
        r.press(1, Nav::Accept);
        assert!(r.shows("Version"), "{:?}", r.texts());
        r.press(1, Nav::Down).press(1, Nav::Accept);
        for text in ["Export", "Japan", "USA"] {
            assert!(r.shows(text), "{text}: {:?}", r.texts());
        }
        assert!(on_row(&r, page::Level::Version, "topskatr"), "the chosen one has the focus");
        // A title without variants has no row.
        r.l.page = None;
        go_to(&mut r, "hotd");
        r.press(1, Nav::Accept);
        assert!(!r.shows("Version") && !r.shows("Japan"), "{:?}", r.texts());
    }

    #[test]
    fn the_version_list_switches_what_play_starts_and_the_choice_is_remembered() {
        let mut r = Rig::new(&["topskatr", "topskatrj", "topskatru"], &[]);
        assert_eq!(r.selected(), "topskatr");
        r.press(1, Nav::Accept);
        // Play is first; Version is the row under it.
        assert!(on_row(&r, page::Level::Main, "play"));
        r.press(1, Nav::Down);
        assert!(on_row(&r, page::Level::Main, "version"));
        r.press(1, Nav::Accept);
        assert_eq!(level(&r), Some(page::Level::Version));
        r.press(1, Nav::Down).press(1, Nav::Accept);
        assert_eq!(r.selected(), "topskatrj");
        assert_eq!(chosen(&r), [("topskatr".to_string(), "topskatrj".to_string())], "kept for the title");
        assert_eq!(level(&r), Some(page::Level::Main), "choosing goes back to the main menu");
        assert!(on_row(&r, page::Level::Main, "play"));
        assert!(r.shows("Japan"), "Play says which version it starts: {:?}", r.texts());
        // The preview follows: the selection time restarts for it.
        r.actions.clear();
        r.press(1, Nav::Accept);
        assert_eq!(r.played(), [("topskatrj".to_string(), Start::New)]);
        // Choosing the same one again is no change.
        r.actions.clear();
        let j = romset::games::find("topskatrj").unwrap();
        r.l.apply(Click::SetVersion(j), &view(None, false), &mut r.actions);
        assert!(chosen(&r).is_empty());
        // The Saves level is the chosen version's.
        r.l.page = None;
        r.key(egui::Key::S);
        r.frame(&[], Vec::new());
        assert_eq!(r.l.version().unwrap().id, "topskatrj");
        assert_eq!(r.l.page.as_ref().unwrap().saves_for, Some("topskatrj"));
        // The keyboard's V opens the list.
        r.l.page = None;
        r.key(egui::Key::V);
        assert_eq!(level(&r), Some(page::Level::Version));
    }

    #[test]
    fn a_choice_is_remembered_and_a_missing_one_falls_back_to_the_title() {
        let mut r = Rig::new(&["srallyc", "srallycb"], &[]);
        let b = romset::games::find("srallycb").unwrap();
        r.l.apply(Click::SetVersion(b), &view(None, false), &mut r.actions);
        assert_eq!(r.selected(), "srallycb");
        assert_eq!(chosen(&r), [("srallyc".to_string(), "srallycb".to_string())]);
        // The settings give it back after a restart (the launcher is built afresh).
        let mut again = Launcher { lan_off: true, ..Default::default() };
        let saved = BTreeMap::from([("srallyc".to_string(), "srallycb".to_string())]);
        again.set_versions(&saved);
        again.set_games(&games(&["srallyc", "srallycb"]), &[]);
        assert_eq!(again.selected().unwrap().id, "srallycb");
        // A choice that is no longer installed falls back to the title.
        again.set_games(&games(&["srallyc"]), &[]);
        assert_eq!(again.selected().unwrap().id, "srallyc");
        // A double click on the card plays the version.
        let mut r = Rig::new(&["srallyc", "srallycb"], &[]);
        r.l.set_versions(&saved);
        r.l.set_games(&games(&["srallyc", "srallycb"]), &[]);
        let card = r.l.card().unwrap();
        r.l.apply(Click::Game(card, true), &view(None, false), &mut r.actions);
        r.l.page = None;
        r.l.apply(Click::Game(card, true), &view(None, false), &mut r.actions);
        assert_eq!(r.played(), [("srallycb".to_string(), Start::New)]);
    }

    #[test]
    fn hacks_and_prototypes_are_listed_with_their_tags() {
        let ids = ["daytona", "daytonat", "daytonata", "srallyc", "srallycb", "srallyp"];
        let mut r = Rig::new(&ids, &[]);
        go_to(&mut r, "daytona");
        assert_eq!(versions(&r), ["daytona", "daytonat", "daytonata"], "the hacks are versions like the others");
        r.press(1, Nav::Accept).press(1, Nav::Down).press(1, Nav::Accept);
        assert!(r.shows("Turbo") && r.shows("Hack"), "{:?}", r.texts());
        r.l.page = None;
        go_to(&mut r, "srallyc");
        assert_eq!(versions(&r), ["srallyc", "srallycb", "srallyp"]);
        r.press(1, Nav::Accept).press(1, Nav::Down).press(1, Nav::Accept);
        assert!(r.shows("Pro Drivin") && r.shows("Prototype"), "{:?}", r.texts());
        assert!(!r.shows("Hack"), "{:?}", r.texts());
        // Choosing one starts it.
        let t = romset::games::find("srallyp").unwrap();
        r.l.apply(Click::SetVersion(t), &view(None, false), &mut r.actions);
        assert_eq!(r.selected(), "srallyp");
        r.actions.clear();
        r.frame(&[], Vec::new());
        r.frame(&[], Vec::new());
        r.press(1, Nav::Accept);
        assert_eq!(r.played(), [("srallyp".to_string(), Start::New)]);
    }

    #[test]
    fn play_together_is_for_games_that_link_and_greyed_for_versions_that_do_not() {
        let mut r = Rig::new(&["daytona", "dayton93", "topskatr", "hotd"], &[]);
        // A game that does not link: no row, and the keys and the pad's View do nothing.
        go_to(&mut r, "topskatr");
        r.press(1, Nav::Accept);
        assert!(!r.shows("Play together"), "{:?}", r.texts());
        r.l.page = None;
        r.key(egui::Key::L);
        assert_eq!(level(&r), None, "L does nothing here");
        r.press(1, Nav::View);
        assert_eq!(level(&r), None);
        // Daytona links: the row is there and opens the level, the pad's View too.
        go_to(&mut r, "daytona");
        r.press(1, Nav::Accept);
        assert!(r.shows("Play together") && !r.shows("Not for this version"), "{:?}", r.texts());
        r.press(2, Nav::Down).press(1, Nav::Accept);
        assert_eq!(level(&r), Some(page::Level::Together));
        for text in ["On this screen", "2 players", "3 players", "4 players", "Host a game", "Join a game", "Your name"]
        {
            assert!(r.shows(&text.to_uppercase()) || r.shows(text), "{text}: {:?}", r.texts());
        }
        // Choosing two players opens the Players screen for two machines (its Start plays them linked).
        r.press(1, Nav::Accept);
        assert!(r.actions.iter().any(|a| matches!(a, Action::Players { set, link: Some(2) } if set == "daytona")));
        r.actions.clear();
        // Host and Join are the network's.
        r.press(3, Nav::Down).press(1, Nav::Accept);
        assert!(r.actions.iter().any(|a| matches!(a, Action::NetHost { set } if set == "daytona")), "host");
        r.actions.clear();
        r.press(1, Nav::Down).press(1, Nav::Accept);
        assert!(r.actions.iter().any(|a| matches!(a, Action::NetJoin { set } if set == "daytona")), "join");
        r.l.page = None;
        r.press(1, Nav::View);
        assert_eq!(level(&r), Some(page::Level::Together), "the pad's View on the home screen");
        r.press(1, Nav::View);
        assert_eq!(level(&r), Some(page::Level::Main), "and Back in the page");
        // A version with link play off: the row is greyed, says so, and cannot be entered.
        r.l.page = None;
        let v = romset::games::find("dayton93").unwrap();
        assert!(crate::link::play_for(v.id).is_none(), "the test's premise");
        r.l.apply(Click::SetVersion(v), &view(None, false), &mut r.actions);
        r.l.open_page(page::Level::Main, false);
        r.frame(&[], Vec::new());
        assert!(r.shows("Play together") && r.shows("Not for this version"), "{:?}", r.texts());
        r.press(2, Nav::Down);
        assert!(!on_row(&r, page::Level::Main, "together"), "the greyed row is skipped by the focus");
        r.press(1, Nav::Accept);
        assert_ne!(level(&r), Some(page::Level::Together));
    }

    #[test]
    fn the_lan_banner_opens_the_page_at_play_together_with_the_join() {
        let mut r = Rig::new(&["daytona", "hotd"], &[]);
        go_to(&mut r, "hotd");
        let found = linknet::Found {
            addr: "192.168.1.20:47890".parse().unwrap(),
            game: "daytona".into(),
            name: "Sam".into(),
            players: 1,
            max: 4,
            version: env!("CARGO_PKG_VERSION").into(),
            rom_hash: None,
            delay: 2,
            lobby: linknet::Lobby::Open,
            addrs: Vec::new(),
            mdns: true,
            broadcast: false,
        };
        r.l.lan_hosts = vec![found];
        r.frame(&[], Vec::new());
        assert!(r.shows("Sam is hosting Daytona USA · 1/4"), "{:?}", r.texts());
        r.l.apply(Click::LanJoin, &view(None, false), &mut r.actions);
        assert_eq!(r.selected(), "daytona");
        assert_eq!(level(&r), Some(page::Level::Together));
        r.frame(&[], Vec::new());
        r.frame(&[], Vec::new());
        assert!(on_row(&r, page::Level::Together, "join hosted"), "{:?}", r.l.page.as_ref().map(|p| p.focus.focused()));
        assert!(r.shows("Join Sam's game") && r.shows("1 of 4 players"), "{:?}", r.texts());
        r.press(1, Nav::Accept);
        assert!(
            r.actions.iter().any(
                |a| matches!(a, Action::NetJoinAt { set, addr } if set == "daytona" && addr == "192.168.1.20:47890")
            ),
            "the join goes to that host"
        );
    }

    /// A game with a second player's controls has a Players row on its page, which opens the
    /// Players screen for one cabinet; a game of one player's has none.
    #[test]
    fn the_page_has_a_players_row_for_a_cabinet_of_two() {
        let mut r = Rig::new(&SET, &[]);
        go_to(&mut r, "topskatr");
        r.press(1, Nav::Accept);
        assert!(!r.shows("Players"), "{:?}", r.texts());
        r.l.page = None;
        go_to(&mut r, "vf2");
        r.press(1, Nav::Accept);
        assert!(r.shows("Players"), "{:?}", r.texts());
        assert!(r.actions.is_empty());
        // Play, then Players.
        r.press(1, Nav::Down).press(1, Nav::Accept);
        assert!(
            r.actions.iter().any(|a| matches!(a, Action::Players { set, link: None } if set == "vf2")),
            "{:?}",
            r.texts()
        );
        assert!(r.played().is_empty(), "opening Players does not play");
    }

    #[test]
    fn game_settings_rows_open_the_menu_pages() {
        let mut r = Rig::new(&SET, &["hotd"]);
        r.press(1, Nav::Accept);
        // (Play, Players, Saves, Game settings: hotd has no versions and does not link.)
        r.press(3, Nav::Down).press(1, Nav::Accept);
        assert_eq!(level(&r), Some(page::Level::Settings));
        for text in ["Picture", "Controls", "Display", "Sound", "Speed and latency"] {
            assert!(r.shows(text), "{text}: {:?}", r.texts());
        }
        assert!(!r.shows("Use the settings for all games"), "{:?}", r.texts());
        r.press(1, Nav::Accept);
        assert!(r.actions.iter().any(|a| matches!(a, Action::SettingsAt(menu::Page::Picture, _))));
        r.actions.clear();
        r.press(2, Nav::Down).press(1, Nav::Accept);
        assert!(r.actions.iter().any(|a| matches!(a, Action::SettingsAt(menu::Page::Settings, menu::Tab::Display))));
        r.actions.clear();
        r.press(1, Nav::Down).press(1, Nav::Accept);
        assert!(r.actions.iter().any(|a| matches!(a, Action::SettingsAt(menu::Page::Settings, menu::Tab::Sound))));
    }

    #[test]
    fn about_this_version_says_what_there_is_to_say() {
        let mut r = Rig::new(&["topskatr", "topskatrj", "hotd"], &[]);
        go_to(&mut r, "hotd");
        r.press(1, Nav::Accept);
        assert!(!r.shows("About this version"), "nothing to say: {:?}", r.texts());
        r.l.page = None;
        go_to(&mut r, "topskatr");
        r.press(1, Nav::Accept);
        assert!(r.shows("About this version"));
        r.l.page = None;
        let j = romset::games::find("topskatrj").unwrap();
        r.l.apply(Click::SetVersion(j), &view(None, false), &mut r.actions);
        r.l.open_page(page::Level::About, false);
        r.frame(&[], Vec::new());
        assert!(r.shows("same game for another market") && r.shows("texture pack"), "{:?}", r.texts());
        // Back is a row, so the pad can leave it.
        r.press(1, Nav::Accept);
        assert_eq!(level(&r), Some(page::Level::Main));
    }

    #[test]
    fn the_page_hints_are_select_back_and_continue() {
        for pad in [true, false] {
            let mut r = Rig::new(&SET, &["stcc"]);
            r.cont = continuing("stcc", Start::New);
            if pad {
                r.press(1, Nav::Accept);
            } else {
                r.key(egui::Key::Enter);
            }
            let has = |r: &Rig, t: &str| r.texts().iter().any(|(x, _)| x == t);
            assert!(has(&r, "Select") && has(&r, "Back to games") && has(&r, "Continue"), "pad {pad}: {:?}", r.texts());
            assert_eq!(r.shows("Esc"), !pad);
            // A level down: Choose and Back, no Continue.
            r.l.page_enter(page::Level::Settings);
            r.frame(&[], Vec::new());
            assert!(
                has(&r, "Choose") && has(&r, "Back") && !has(&r, "Continue") && !has(&r, "Select"),
                "{:?}",
                r.texts()
            );
        }
    }

    #[test]
    fn the_page_shows_the_title_and_its_facts_at_every_size() {
        for (size, ppp) in [
            (vec2(1280.0, 720.0), 1.0),
            (vec2(1920.0, 1080.0), 1.0),
            (vec2(3840.0, 2160.0), 2.0),
            (vec2(1024.0, 768.0), 1.0),
            (vec2(800.0, 600.0), 1.0),
        ] {
            for id in ["daytona", "topskatr"] {
                let mut r = Rig::new(&["daytona", "dayton93", "daytonat", "topskatr"], &[]);
                r.size = size / ppp;
                go_to(&mut r, id);
                r.press(1, Nav::Accept);
                for _ in 0..3 {
                    r.frame(&[], Vec::new());
                }
                assert!(r.shows(&r.l.version().unwrap().title.to_uppercase()), "{id} {size:?}: {:?}", r.texts());
                let u = layout(Rect::from_min_size(Pos2::ZERO, r.size)).u;
                let small: Vec<_> = r.texts().into_iter().filter(|(_, s)| *s < 13.99 * u).collect();
                assert!(small.is_empty(), "{id} {size:?}: {small:?}");
                for level in [
                    page::Level::Version,
                    page::Level::Together,
                    page::Level::Saves,
                    page::Level::Settings,
                    page::Level::About,
                ] {
                    r.l.page_enter(level);
                    for _ in 0..3 {
                        r.frame(&[], Vec::new());
                    }
                    let screen = Rect::from_min_size(Pos2::ZERO, r.size);
                    let l = layout(screen);
                    for c in &r.shapes {
                        if let egui::Shape::Text(t) = &c.shape {
                            let b = t.visual_bounding_rect();
                            // Nothing under the hint bar, and the header is inside the screen.
                            if c.clip_rect.intersects(b) && b.center().y > l.bar.top() && b.center().y < l.bar.bottom()
                            {
                                assert!(
                                    screen.expand(1.0).contains_rect(b),
                                    "{id} {level:?} {size:?}: {:?} at {b:?}",
                                    t.galley.text()
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_variant_played_last_brings_its_card_forward_and_is_selected_again() {
        let mut ids = SET.to_vec();
        ids.push("topskatrj");
        let mut r = Rig::new(&ids, &["topskatrj"]);
        // The card of its title is first in the carousel, as the recent sort has it.
        assert_eq!(r.l.carousel_ids()[0], "topskatr");
        assert_eq!(r.l.card().unwrap().id, "topskatr");
        // Coming back to the launcher from the game selects that version.
        r.l.select_set("topskatrj");
        assert_eq!(r.selected(), "topskatrj");
        // The quick resume names the version.
        let c = library::quick_resume(&r.l.all, &r.l.recent, |_| Vec::new()).unwrap();
        assert_eq!(c.game.id, "topskatrj");
    }

    #[test]
    fn the_grid_card_says_how_many_versions_there_are() {
        let mut r = Rig::new(&["topskatr", "topskatrj", "hotd"], &[]);
        r.l.set_mode(ViewMode::Grid);
        r.frame(&[], Vec::new());
        assert!(r.shows("2 versions"), "{:?}", r.texts());
        // A or Enter on a poster opens the page here too, and the grid's hints are the home's and Filter.
        r.press(1, Nav::Accept);
        assert!(level(&r).is_some());
        assert!(r.l.wants_preview(), "the page shows the preview whichever view is under it");
        r.press(1, Nav::Back);
        assert!(level(&r).is_none() && r.on_a_tile(), "back on the poster");
    }

    #[test]
    fn the_pad_shows_no_key_cap_for_the_keyboards_version_and_join_keys() {
        let mut r = Rig::new(&["topskatr", "topskatrj"], &[]);
        r.l.pad_used = true;
        assert_eq!(r.l.chip(Chip::None, "V"), Chip::None);
        assert_eq!(r.l.chip(Chip::None, "J"), Chip::None);
        r.l.pad_used = false;
        assert_eq!(r.l.chip(Chip::None, "V"), Chip::Key("V"));
        assert_eq!(r.l.chip(Chip::None, "J"), Chip::Key("J"));
    }
}
