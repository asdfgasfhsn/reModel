//! The launcher: the player's start screen, drawn with egui over the
//! backdrop (`backdrop`), in the theme's look (`theme`).
//!
//! Two views of the games found in the ROM folder (never the ones missing),
//! switched from the top bar or with LB and RB, and remembered:
//!
//! * the **carousel**: the selected game's poster large at the right, the
//!   next games beside it, and at the left its name, kind, board and year
//!   and what can be done (play, saves, settings); the game's live attract
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
//! pad or the keyboard is in use) and from the mouse. Settings open the
//! menu's side panel (`menu`), Saves a panel of the game's save slots.
//!
//! Keyboard: arrows choose, Enter plays, S saves, Tab settings, C
//! continues, Q and E (or Page Up and Down) switch the view, [ and ] the
//! filter. Pad: d-pad or stick, A, X, Start (the menu button), Y, LB RB, LT
//! RT. Mouse: click a poster to select it, again to play; the wheel steps
//! through the carousel and scrolls the grid.
//!
//! Everything is laid out in design points at 1280x720 and scaled to the
//! window (`layout`), so it reads the same at 1080p, ultrawide and 4K.

use crate::focus::Focus;
use crate::library::{self, Continue, Filter, Sort, Start, ViewMode};
use crate::menu;
use crate::saves::{self, SlotInfo};
use crate::theme::{self, Chip};
use egui::{
    pos2, vec2, Align2, Color32, FontFamily, FontId, Id, Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui,
    Vec2,
};
use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

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
    /// Host a network link session of `set` (the player makes the lobby's socket).
    NetHost {
        set: String,
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
    /// Open the menu over the launcher (its settings).
    Settings,
    /// Close it (Esc, Start).
    CloseSettings,
    /// Drop the game's own settings (the menu's "Use the settings for all
    /// games").
    ResetGame(String),
    /// Look for games in this ROM folder.
    Rescan(String),
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
    /// shape at half its size (it is only ever shown dimmed behind the
    /// carousel).
    pub fn hero(&self) -> Rect {
        Rect::from_center_size(self.screen.center(), self.screen.size() * 0.5)
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

/// The Saves panel: the selected game's states.
struct SavesPage {
    game: &'static romset::Game,
    items: Vec<SaveItem>,
    focus: Focus,
}

/// The Link play panel: how many machines to start the selected game with.
struct LinkPage {
    game: &'static romset::Game,
    focus: Focus,
}

/// A click (or a pad press) on the launcher's page, applied after it is
/// drawn.
#[derive(Clone, Copy)]
enum Click {
    View(ViewMode),
    Filter(Filter),
    NextFilter,
    Sort,
    Continue,
    Play,
    Saves,
    Settings,
    /// Link play (same screen): the player count panel.
    Link,
    /// A game's poster: selects it, or plays it if it is.
    Game(&'static romset::Game),
}

pub struct Launcher {
    pub open: bool,
    /// The playable games, and the recently played (latest first).
    all: Vec<&'static romset::Game>,
    recent: Vec<String>,
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
    saves: Option<SavesPage>,
    want_saves: bool,
    link_page: Option<LinkPage>,
    /// The network link play panel (lobby or join), over everything else.
    net_page: Option<crate::netpage::NetPage>,
    posters: HashMap<String, egui::TextureHandle>,
    /// Each poster blurred (a small copy, stretched), the carousel's
    /// background while the preview isn't there.
    blurs: HashMap<String, egui::TextureHandle>,
    art: Option<crate::art::Loader>,
    logo: Option<egui::TextureHandle>,
    /// The glyph for "choose" in the hint bar, if the fonts have it.
    choose: Option<&'static str>,
    /// The ROM folder being edited (no games found).
    pub rom_text: String,
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
            recent: Vec::new(),
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
            saves: None,
            want_saves: false,
            link_page: None,
            net_page: None,
            posters: HashMap::new(),
            blurs: HashMap::new(),
            art: None,
            logo: None,
            choose: None,
            rom_text: String::new(),
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
        self.recent = recent.to_vec();
        self.chips = Filter::chips(games, recent);
        if !self.chips.contains(&self.filter) {
            self.filter = Filter::All;
        }
        self.relist();
        let keep = self.cur.filter(|c| self.all.iter().any(|g| g.id == c.id));
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
        self.carousel = library::list(&self.all, &self.recent, Filter::All, Sort::Recent);
        self.grid = library::list(&self.all, &self.recent, self.filter, self.sort);
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

    /// Select the game for `set`.
    pub fn select_set(&mut self, set: &str) {
        if let Some(g) = self.all.iter().copied().find(|g| g.id == set) {
            self.select(g);
            self.pending_focus = Some((self.selected_id(), 6));
        }
    }

    fn select(&mut self, game: &'static romset::Game) {
        if self.cur.map(|c| c.id) != Some(game.id) {
            self.selected_at = Instant::now();
        }
        self.cur = Some(game);
    }

    pub fn selected(&self) -> Option<&'static romset::Game> {
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

    /// The selected game's preview is wanted: the grid doesn't show it.
    pub fn wants_preview(&self) -> bool {
        self.mode == ViewMode::Carousel
    }

    /// The games the grid lists now, and how many there are in all.
    pub fn counts(&self) -> (usize, usize) {
        (self.grid.len(), self.all.len())
    }

    #[cfg(test)]
    fn grid_ids(&self) -> Vec<&'static str> {
        self.grid.iter().map(|g| g.id).collect()
    }

    #[cfg(test)]
    fn carousel_ids(&self) -> Vec<&'static str> {
        self.carousel.iter().map(|g| g.id).collect()
    }

    /// The Saves panel is open.
    pub fn saves_open(&self) -> bool {
        self.saves.is_some() || self.want_saves || self.link_page.is_some() || self.net_page.is_some()
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
        if self.choose.is_none() {
            let font = FontId::new(18.0, theme::bold());
            let has = ctx.fonts_mut(|f| f.has_glyph(&font, '✥'));
            self.choose = Some(if has { "✥" } else { "D-pad" });
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
        } else if self.net_page.is_some() && ctx.egui_wants_keyboard_input() {
            // Typing an address: the keys are the text's.
            Vec::new()
        } else {
            Self::keys(ctx)
        };
        self.note_input(ctx, &navs, &keys);
        let all: Vec<Nav> = navs.iter().chain(&keys).copied().collect();
        if self.want_saves {
            self.want_saves = false;
            self.open_saves(ctx);
        }
        // The Saves panel and the menu take the input; the page behind
        // gets none (its focus just holds).
        let page_navs = if view.menu_open || self.saves.is_some() || self.link_page.is_some() || self.net_page.is_some()
        {
            Vec::new()
        } else {
            all.clone()
        };
        let page_navs = self.override_navs(page_navs);
        self.focus.show = self.pad_used || self.keys_used;
        let mut clicks = Vec::new();
        if !(view.menu_open || self.saves.is_some() || self.link_page.is_some() || self.net_page.is_some())
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::L))
        {
            clicks.push(Click::Link);
        }
        for nav in self.focus.begin(&page_navs) {
            match nav {
                Nav::Start | Nav::Settings => clicks.push(Click::Settings),
                Nav::Saves => clicks.push(Click::Saves),
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
            self.paint_bar(ui, &p, &l, view, &mut clicks);
        });
        self.focus.end();
        self.apply_focus();
        for click in clicks {
            self.apply(click, view, &mut actions);
        }
        if self.saves.is_some() {
            self.paint_saves(ctx, &all, view, &mut actions);
        }
        if self.link_page.is_some() {
            self.paint_link(ctx, &all, &mut actions);
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
    /// through the games (round, as the strip is), and down goes to the
    /// buttons. The rest is for the focus to place by where things are.
    fn override_navs(&mut self, navs: Vec<Nav>) -> Vec<Nav> {
        if self.mode != ViewMode::Carousel || self.focus.focused() != Some(selected_card()) {
            return navs;
        }
        let mut rest = Vec::new();
        for nav in navs {
            match nav {
                Nav::Left => self.step(-1),
                Nav::Right => self.step(1),
                Nav::Down => self.pending_focus = Some((button_id("play"), 6)),
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
            Click::Play => {
                if let Some(g) = self.cur {
                    actions.push(Action::Play { set: g.id.to_string(), from: Start::New });
                }
            }
            Click::Saves => {
                if self.cur.is_some() {
                    self.want_saves = true;
                }
            }
            Click::Settings => actions.push(Action::Settings),
            Click::Link => {
                if let Some(game) = self.cur.filter(|g| crate::link::play_for(g.id).is_some()) {
                    self.link_page = Some(LinkPage { game, focus: Focus::default() });
                }
            }
            Click::Game(g) => {
                if self.cur.map(|c| c.id) == Some(g.id) {
                    actions.push(Action::Play { set: g.id.to_string(), from: Start::New });
                } else {
                    self.select(g);
                    self.pending_focus = Some((self.selected_id(), 6));
                }
            }
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
        growing || (self.mode == ViewMode::Carousel && self.slide != 0.0) || self.pending_focus.is_some()
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
            let n = self.all.len();
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
            ("Continue".to_string(), format!("{} · {}", c.game.title, c.saved.map_or("new".to_string(), age)))
        } else {
            ("CONTINUE".to_string(), format!("{} · {when}", c.game.title))
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
        let Some(game) = self.cur else { return };
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
        y += row_h + 18.0 * u;
        let line = match view.resume {
            Some(t) => format!("You left off {}. Saves has it.", age(t)),
            None if self.recent.iter().any(|r| r == game.id) => "Played before.".to_string(),
            None => "Not played yet.".to_string(),
        };
        let para = p.layout(line, FontId::new(20.0 * u, FontFamily::Proportional), theme::TEXT_2, info_w);
        p.galley(pos2(left, y), para.clone(), theme::TEXT_2);
        y += para.size().y + 22.0 * u;
        // Play, Saves, Settings.
        let mut buttons = vec![
            ("play", "Play", self.chip(Chip::A, "Enter"), Click::Play, true),
            ("saves", "Saves", self.chip(Chip::X, "S"), Click::Saves, false),
            ("settings", "Settings", self.chip(Chip::Menu, "Tab"), Click::Settings, false),
        ];
        // Games that can link cabinets on one screen: that, too.
        if crate::link::play_for(game.id).is_some() {
            buttons.insert(2, ("link", "Link play", Chip::Key("L"), Click::Link, false));
        }
        let font = FontId::new(22.0 * u, theme::bold());
        let mut bx = left;
        for (name, label, chip, click, primary) in buttons {
            let g = p.layout_no_wrap(label.to_string(), font.clone(), FG);
            let d = 34.0;
            let cw = theme::chip_width_at(p, chip, u, d);
            let w = 18.0 * u + cw + 12.0 * u + g.size().x + 26.0 * u;
            // (A narrow window wraps them.)
            if bx > left && bx + w > left + info_w.max(w) {
                bx = left;
                y += 78.0 * u;
            }
            let r = Rect::from_min_size(pos2(bx, y), vec2(w, 66.0 * u));
            let id = button_id(name);
            let (resp, hit) = control(&mut self.focus, ui, id, r);
            if primary {
                p.add(
                    egui::epaint::RectShape::filled(
                        r.translate(vec2(0.0, 8.0 * u)),
                        14.0 * u,
                        Color32::from_rgba_unmultiplied(31, 107, 255, 90),
                    )
                    .with_blur_width(24.0 * u),
                );
                p.rect_filled(r, 14.0 * u, if resp.hovered() { Color32::from_rgb(60, 130, 255) } else { theme::BLUE });
                p.rect_stroke(
                    r,
                    14.0 * u,
                    Stroke::new(2.0 * u, Color32::from_rgba_unmultiplied(34, 229, 255, 128)),
                    StrokeKind::Inside,
                );
            } else {
                p.rect_filled(r, 14.0 * u, Color32::from_white_alpha(if resp.hovered() { 36 } else { 26 }));
            }
            theme::paint_chip_at(p, chip, pos2(r.left() + 18.0 * u, r.center().y), u, d);
            p.galley(pos2(r.left() + 18.0 * u + cw + 12.0 * u, r.center().y - g.size().y / 2.0), g, FG);
            if self.focus.is_on(id) {
                theme::focus_ring(p, r, u, 14.0);
            }
            if hit {
                clicks.push(click);
            }
            bx = r.right() + 14.0 * u;
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
        if ui.rect_contains_pointer(strip) && !self.saves_open() {
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
                    clicks.push(Click::Game(game));
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
                        clicks.push(Click::Game(game));
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
                    let genre = fit_text(&p, game.genre, &genre_font, tw - 8.0 * u);
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

    /// The hint bar along the foot: what each button does, the same in both
    /// views (the grid adds the filters), each one clickable.
    fn paint_bar(&mut self, ui: &mut Ui, p: &Painter, l: &Layout, view: &View, clicks: &mut Vec<Click>) {
        let u = l.u;
        p.rect_filled(l.bar, 0.0, Color32::from_rgba_unmultiplied(4, 6, 11, 230));
        p.hline(l.bar.x_range(), l.bar.top(), Stroke::new(1.0 * u, Color32::from_white_alpha(15)));
        let grid = self.mode == ViewMode::Grid;
        let choose = self.choose.unwrap_or("D-pad");
        let hints: Vec<(Chip, &str, Option<Click>)> = if self.pad_used {
            let mut h = vec![
                (Chip::Key(choose), "Choose", None),
                (Chip::A, "Play", Some(Click::Play)),
                (Chip::X, "Saves", Some(Click::Saves)),
                (Chip::Menu, "Settings", Some(Click::Settings)),
                (Chip::Y, "Continue", Some(Click::Continue)),
                (Chip::Key("LB RB"), "View", Some(Click::View(self.mode.other()))),
            ];
            if grid {
                h.push((Chip::Key("LT RT"), "Filter", Some(Click::NextFilter)));
            }
            h
        } else {
            let mut h = vec![
                (Chip::Key("← → ↑ ↓"), "Choose", None),
                (Chip::Key("Enter"), "Play", Some(Click::Play)),
                (Chip::Key("S"), "Saves", Some(Click::Saves)),
                (Chip::Key("Tab"), "Settings", Some(Click::Settings)),
                (Chip::Key("C"), "Continue", Some(Click::Continue)),
                (Chip::Key("Q E"), "View", Some(Click::View(self.mode.other()))),
            ];
            if grid {
                h.push((Chip::Key("[ ]"), "Filter", Some(Click::NextFilter)));
            }
            h
        };
        let font = FontId::new(16.0 * u, FontFamily::Proportional);
        let d = theme::CHIP_LARGE;
        let mut x = l.top.left();
        let y = l.bar.center().y;
        for (i, (chip, label, click)) in hints.into_iter().enumerate() {
            let cw = theme::chip_width_at(p, chip, u, d);
            let g = p.layout_no_wrap(label.to_string(), font.clone(), theme::TEXT_2);
            let r = Rect::from_min_size(pos2(x, y - 19.0 * u), vec2(cw + 8.0 * u + g.size().x + 18.0 * u, 38.0 * u));
            let dim = matches!(click, Some(Click::Continue)) && view.cont.is_none();
            let mut shown = Color32::from_white_alpha(if dim { 90 } else { 255 });
            if let Some(c) = click {
                let id = Id::unique(("hint", i));
                let (resp, hit) = control(&mut self.focus, ui, id, r);
                if resp.hovered() && !dim {
                    p.rect_filled(r, 10.0 * u, Color32::from_white_alpha(20));
                }
                if self.focus.is_on(id) {
                    theme::focus_ring(p, r, u, 10.0);
                }
                if hit && !dim {
                    clicks.push(c);
                }
            }
            if dim {
                shown = Color32::from_white_alpha(90);
            }
            theme::paint_chip_at(p, chip, pos2(r.left() + 6.0 * u, y), u, d);
            p.galley(pos2(r.left() + 6.0 * u + cw + 8.0 * u, y - g.size().y / 2.0), g, shown);
            x = r.right() + 8.0 * u;
        }
        if grid {
            let (shown, of) = self.counts();
            p.text(
                pos2(l.top.right(), y),
                Align2::RIGHT_CENTER,
                format!("{shown} of {of} games"),
                FontId::new(16.0 * u, FontFamily::Proportional),
                theme::TEXT_DIM,
            );
        }
        let _ = view;
    }

    /// Read the selected game's states and show the Saves panel.
    fn open_saves(&mut self, ctx: &egui::Context) {
        let Some(game) = self.cur else { return };
        let mut items = Vec::new();
        let entries = std::iter::once((Start::Resume, "Where you left off".to_string(), saves::resume_file(game.id)))
            .chain((0..menu::SLOTS).map(|n| {
                (
                    Start::Slot(n),
                    if n == 0 { "Quick save".to_string() } else { format!("Slot {n}") },
                    saves::slot_file(game.id, n),
                )
            }));
        for (from, name, path) in entries {
            let info = saves::read_info(&path);
            let thumb = info.as_ref().and_then(|i| i.thumb.as_ref()).and_then(|(png, _)| saves::read_thumb(png)).map(
                |(size, rgba)| {
                    ctx.load_texture(
                        format!("save {} {name}", game.id),
                        egui::ColorImage::from_rgba_unmultiplied(size, &rgba),
                        egui::TextureOptions::LINEAR,
                    )
                },
            );
            items.push(SaveItem { from, name, info, thumb });
        }
        self.saves = Some(SavesPage { game, items, focus: Focus::default() });
    }

    /// The Saves panel over the launcher: a card per state; one chosen
    /// starts the game from it.
    fn paint_saves(&mut self, ctx: &egui::Context, navs: &[Nav], view: &View, actions: &mut Vec<Action>) {
        let _ = view;
        let (pad, keys) = (self.pad_used, self.keys_used);
        let Some(page) = &mut self.saves else { return };
        let s = theme::scale(ctx);
        page.focus.show = pad || keys;
        let rest = page.focus.begin(navs);
        let mut back = rest.iter().any(|n| matches!(n, Nav::Back | Nav::Saves));
        if rest.contains(&Nav::Start) || rest.contains(&Nav::Settings) {
            back = true;
        }
        let hints = [(Chip::A, "Load"), (Chip::B, "Back"), (Chip::Key("Esc"), "Close")];
        let spec = menu::PanelSpec { game: page.game.title, heading: "Saves", back: true, extra_h: 0.0 };
        let mut chosen = None;
        let SavesPage { game, items, focus } = page;
        back |= menu::side_panel(ctx, &spec, &hints, |ui, reg| {
            ui.scope_builder(egui::UiBuilder::new().max_rect(reg.body.shrink2(vec2(24.0 * s, 0.0))), |ui| {
                focus.scroll_area("saves").show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0 * s;
                    ui.add_space(12.0 * s);
                    for (i, item) in items.iter().enumerate() {
                        let id = Id::unique(("save-card", game.id, i));
                        if menu::slot_card(ui, focus, id, &item.name, false, item.info.as_ref(), item.thumb.as_ref()) {
                            chosen = Some(i);
                        }
                    }
                    ui.add_space(12.0 * s);
                });
            });
            false
        });
        page.focus.end();
        if let Some(i) = chosen {
            let item = &page.items[i];
            if item.info.as_ref().is_some_and(|inf| inf.loads) {
                actions.push(Action::Play { set: page.game.id.to_string(), from: item.from });
                back = true;
            }
        }
        if back {
            self.saves = None;
        }
    }

    /// Show the network link play panel (the player made its lobby or its join).
    pub fn show_net(&mut self, page: crate::netpage::NetPage) {
        self.net_page = Some(page);
    }

    /// The Link play panel over the launcher: a row for each number of
    /// machines the game can link; one chosen starts the game with them.
    fn paint_link(&mut self, ctx: &egui::Context, navs: &[Nav], actions: &mut Vec<Action>) {
        let (pad, keys) = (self.pad_used, self.keys_used);
        let Some(page) = &mut self.link_page else { return };
        page.focus.show = pad || keys;
        let rest = page.focus.begin(navs);
        let mut back = rest.iter().any(|n| matches!(n, Nav::Back | Nav::Start | Nav::Settings));
        let hints = [(Chip::A, "Start"), (Chip::B, "Back"), (Chip::Key("Esc"), "Close")];
        let spec = menu::PanelSpec { game: page.game.title, heading: "Link play", back: true, extra_h: 0.0 };
        let max = crate::link::play_for(page.game.id).map_or(2, crate::link::max_players);
        let mut chosen = None;
        let mut net: Option<bool> = None;
        let LinkPage { game, focus } = page;
        back |= menu::side_panel(ctx, &spec, &hints, |ui, reg| {
            let s = theme::scale(ui.ctx());
            ui.scope_builder(egui::UiBuilder::new().max_rect(reg.body.shrink2(vec2(24.0 * s, 0.0))), |ui| {
                ui.add_space(12.0 * s);
                ui.label(
                    egui::RichText::new(format!(
                        "Linked cabinets on one screen: {} machines side by side, one controller each. \
                         The keyboard plays player 1; each gamepad joins with a button press.",
                        game.title
                    ))
                    .size(15.0 * s)
                    .color(theme::TEXT_2),
                );
                ui.add_space(12.0 * s);
                for n in 2..=max {
                    if focus.menu_row(ui, &format!("{n} players"), if n == 2 { "split" } else { "quadrants" }) {
                        chosen = Some(n);
                    }
                }
                ui.add_space(14.0 * s);
                ui.label(
                    egui::RichText::new("Or each on its own machine, over the network (a LAN):")
                        .size(15.0 * s)
                        .color(theme::TEXT_2),
                );
                ui.add_space(6.0 * s);
                if focus.menu_row(ui, "Host a network game", "this machine is player 1") {
                    net = Some(true);
                }
                if focus.menu_row(ui, "Join a network game", "an address, or a host found") {
                    net = Some(false);
                }
            });
            false
        });
        page.focus.end();
        if let Some(players) = chosen {
            actions.push(Action::PlayLink { set: page.game.id.to_string(), players });
            back = true;
        }
        if let Some(host) = net {
            let set = page.game.id.to_string();
            actions.push(if host { Action::NetHost { set } } else { Action::NetJoin { set } });
            back = true;
        }
        if back {
            self.link_page = None;
        }
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
        }
    }

    /// A launcher drawn headless, a frame at a time, the way `ui` is called.
    struct Rig {
        ctx: egui::Context,
        l: Launcher,
        size: Vec2,
        cont: Option<Continue>,
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
            let mut l = Launcher::default();
            let recent: Vec<String> = recent.iter().map(|s| s.to_string()).collect();
            l.set_games(&games(ids), &recent);
            let mut rig = Rig {
                ctx,
                l,
                size: vec2(1280.0, 720.0),
                cont: None,
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
            let view = view(self.cont, self.menu_open);
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
    fn a_plays_from_the_start_and_a_poster_plays_when_clicked_twice() {
        let mut r = Rig::new(&SET, &["stcc"]);
        r.press(1, Nav::Accept);
        assert_eq!(r.played(), [("stcc".to_string(), Start::New)]);
        r.actions.clear();
        // A click on another poster selects it; on the selected, plays.
        let hotd = romset::games::find("hotd").unwrap();
        r.l.apply(Click::Game(hotd), &view(None, false), &mut r.actions);
        assert_eq!(r.selected(), "hotd");
        assert!(r.actions.is_empty());
        r.l.apply(Click::Game(hotd), &view(None, false), &mut r.actions);
        assert_eq!(r.played(), [("hotd".to_string(), Start::New)]);
    }

    #[test]
    fn down_from_the_poster_reaches_the_buttons() {
        let mut r = Rig::new(&SET, &["stcc"]);
        r.press(1, Nav::Down);
        assert_eq!(r.l.focus.focused(), Some(button_id("play")));
        r.press(1, Nav::Accept);
        assert_eq!(r.played(), [("stcc".to_string(), Start::New)]);
        r.actions.clear();
        r.press(1, Nav::Right).press(1, Nav::Accept);
        assert!(r.l.saves.is_some(), "right of Play is Saves");
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

    #[test]
    fn x_opens_the_saves_and_a_card_starts_the_game_from_it() {
        let mut r = Rig::new(&SET, &["stcc"]);
        r.press(1, Nav::Saves);
        r.frame(&[], Vec::new());
        assert!(r.l.saves.is_some(), "the panel opens");
        assert!(r.shows("Where you left off") && r.shows("Quick save") && r.shows("Slot 4"), "{:?}", r.texts());
        // Empty slots load nothing.
        r.press(1, Nav::Accept);
        assert!(r.played().is_empty() && r.l.saves.is_some());
        // Give the resume state and slot 2 a state this build loads.
        let saved = |t: u64| SlotInfo {
            time: SystemTime::UNIX_EPOCH + Duration::from_secs(t),
            made_by: None,
            loads: true,
            thumb: None,
        };
        let page = r.l.saves.as_mut().unwrap();
        page.items[0].info = Some(saved(100));
        page.items[3].info = Some(saved(200));
        r.press(1, Nav::Accept);
        assert_eq!(r.played(), [("stcc".to_string(), Start::Resume)]);
        assert!(r.l.saves.is_none(), "it closes");
        // Slot 2 is three cards down.
        r.actions.clear();
        r.press(1, Nav::Saves);
        r.l.saves.as_mut().unwrap().items[3].info = Some(saved(200));
        r.press(3, Nav::Down).press(1, Nav::Accept);
        assert_eq!(r.played(), [("stcc".to_string(), Start::Slot(2))]);
        // B closes it without playing; the keyboard's S opens it.
        r.actions.clear();
        r.press(1, Nav::Saves).press(1, Nav::Back);
        assert!(r.l.saves.is_none() && r.played().is_empty());
        r.key(egui::Key::S);
        assert!(r.l.saves.is_some());
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
        // A plays the selected one.
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
                    ("Play", button_id("play")),
                    ("Saves", button_id("saves")),
                    ("Settings", button_id("settings")),
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
                ["Choose", "Play", "Saves", "Settings", "Continue", "View", "Filter"]
                    .iter()
                    .filter(|h| r.texts().iter().any(|(t, _)| t == *h))
                    .map(|h| h.to_string())
                    .collect()
            };
            let carousel = hints(&r);
            assert_eq!(carousel, ["Choose", "Play", "Saves", "Settings", "Continue", "View"], "pad {pad}");
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
}
