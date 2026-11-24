//! The game page: a full page for one game, opened from its poster. At the
//! left the live attract preview of the chosen version (its poster while the
//! preview loads); at the right the game's name and facts and a menu: Play,
//! Version, Play together, Saves, Game settings and About this version. Each
//! but Play opens one level down in the same page (B or Esc goes back a
//! level, then to the games). The levels reuse what the launcher had in
//! panels: the Version list, the Link play choices (and the network's
//! host and join), the save slots, and rows that open the menu's own pages
//! of the game's settings (`menu`), so nothing is configured twice.

use super::*;
use romset::VariantKind;

/// What the page is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Level {
    Main,
    Version,
    Together,
    Saves,
    Settings,
    About,
}

impl Level {
    fn label(self) -> &'static str {
        match self {
            Level::Main => "Game",
            Level::Version => "Version",
            Level::Together => "Play together",
            Level::Saves => "Saves",
            Level::Settings => "Game settings",
            Level::About => "About this version",
        }
    }

    /// The key of the main menu's row that opens the level.
    fn row(self) -> &'static str {
        match self {
            Level::Main => "play",
            Level::Version => "version",
            Level::Together => "together",
            Level::Saves => "saves",
            Level::Settings => "settings",
            Level::About => "about",
        }
    }
}

/// The page's state: where it is and what it keeps between frames.
pub struct GamePage {
    pub level: Level,
    pub(super) focus: Focus,
    /// A row to move the focus to once it has been drawn (a try or two).
    pending: Option<(Id, u8)>,
    /// Opened by a click: a press before this is the second of a double
    /// click, and plays.
    pub(super) dbl_until: Option<Instant>,
    /// The slots, read for `saves_for` when the Saves level was entered.
    pub(super) saves: Vec<SaveItem>,
    pub(super) saves_for: Option<&'static str>,
    /// How many of the game's save files there are, for `used_for`.
    used: usize,
    used_for: Option<&'static str>,
}

impl GamePage {
    fn new(level: Level) -> GamePage {
        GamePage {
            level,
            focus: Focus::default(),
            pending: None,
            dbl_until: None,
            saves: Vec::new(),
            saves_for: None,
            used: 0,
            used_for: None,
        }
    }

    pub fn animating(&self) -> bool {
        self.pending.is_some()
    }

    /// Focus the row `key` of `level` once it is drawn.
    pub fn want(&mut self, level: Level, key: &str) {
        self.pending = Some((row_id(level, key), 8));
    }
}

pub(super) fn row_id(level: Level, key: &str) -> Id {
    Id::unique(("page-row", level, key))
}

/// A row of the menu.
struct Row {
    key: String,
    label: String,
    tag: Option<(&'static str, Color32)>,
    sub: String,
    right: String,
    radio: Option<bool>,
    primary: bool,
    enabled: bool,
    chevron: bool,
}

impl Row {
    fn new(key: impl Into<String>, label: impl Into<String>) -> Row {
        Row {
            key: key.into(),
            label: label.into(),
            tag: None,
            sub: String::new(),
            right: String::new(),
            radio: None,
            primary: false,
            enabled: true,
            chevron: false,
        }
    }

    fn sub(mut self, sub: impl Into<String>) -> Self {
        self.sub = sub.into();
        self
    }

    fn right(mut self, right: impl Into<String>) -> Self {
        self.right = right.into();
        self
    }

    fn chevron(mut self) -> Self {
        self.chevron = true;
        self
    }
}

/// Row height and gap, in design points.
const ROW_H: f32 = 56.0;
const ROW_GAP: f32 = 5.0;

/// A row, in the flow of `ui`: true if it was clicked or accepted from the pad.
fn row(ui: &mut Ui, focus: &mut Focus, level: Level, spec: &Row, u: f32) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H * u), Sense::hover());
    let id = row_id(level, &spec.key);
    let r = ui.interact(rect, id, if spec.enabled { Sense::click() } else { Sense::hover() });
    let p = ui.painter().clone();
    let radius = 14.0 * u;
    let hover = r.hovered() && spec.enabled;
    if spec.primary {
        p.add(
            egui::epaint::RectShape::filled(
                rect.translate(vec2(0.0, 6.0 * u)),
                radius,
                Color32::from_rgba_unmultiplied(31, 107, 255, 80),
            )
            .with_blur_width(20.0 * u),
        );
        p.rect_filled(rect, radius, if hover { Color32::from_rgb(60, 130, 255) } else { theme::BLUE });
        p.rect_stroke(
            rect,
            radius,
            Stroke::new(2.0 * u, Color32::from_rgba_unmultiplied(34, 229, 255, 128)),
            StrokeKind::Inside,
        );
    } else {
        let a = if !spec.enabled {
            10
        } else if hover {
            38
        } else {
            24
        };
        p.rect_filled(rect, radius, Color32::from_white_alpha(a));
    }
    if spec.enabled && focus.is_on(id) {
        theme::focus_ring(&p, rect, u, 14.0);
    }
    let (ink, dim) = if spec.enabled {
        (FG, if spec.primary { theme::CYAN_LIGHT } else { DIM })
    } else {
        (DIM, Color32::from_rgb(110, 118, 135))
    };
    let pad = 20.0 * u;
    let mut x = rect.left() + pad;
    if let Some(on) = spec.radio {
        let c = pos2(x + 11.0 * u, rect.center().y);
        let ring = if on { theme::CYAN } else { Color32::from_white_alpha(130) };
        p.circle_stroke(c, 10.0 * u, Stroke::new(2.0 * u, ring));
        if on {
            p.circle_filled(c, 5.5 * u, theme::CYAN);
        }
        x += 36.0 * u;
    }
    // The right: a value, then a chevron.
    let mut right_edge = rect.right() - pad;
    if spec.chevron {
        p.text(
            pos2(right_edge, rect.center().y - 1.0 * u),
            Align2::RIGHT_CENTER,
            "›",
            FontId::new(26.0 * u, theme::bold()),
            dim,
        );
        right_edge -= 20.0 * u;
    }
    let value_font = FontId::new(17.0 * u, theme::semibold());
    let mut left_end = right_edge;
    if !spec.right.is_empty() {
        let room = (rect.width() * 0.4).max(40.0 * u);
        let text = fit_text(&p, &spec.right, &value_font, room);
        let g = p.layout_no_wrap(text, value_font, dim);
        left_end = right_edge - g.size().x - 16.0 * u;
        p.galley(pos2(right_edge - g.size().x, rect.center().y - g.size().y / 2.0), g, dim);
    }
    let room = (left_end - x).max(20.0 * u);
    let label_font = FontId::new(22.0 * u, theme::bold());
    let tag_font = FontId::new(14.0 * u, theme::semibold());
    let tag = spec.tag.map(|(t, c)| (p.layout_no_wrap(t.to_string(), tag_font.clone(), FG), c));
    let tag_w = tag.as_ref().map_or(0.0, |(g, _)| g.size().x + 18.0 * u + 10.0 * u);
    let label = fit_text(&p, &spec.label, &label_font, (room - tag_w).max(20.0 * u));
    let lg = p.layout_no_wrap(label, label_font, ink);
    let sg = (!spec.sub.is_empty()).then(|| {
        let font = FontId::new(15.0 * u, FontFamily::Proportional);
        let text = fit_text(&p, &spec.sub, &font, room);
        p.layout_no_wrap(text, font, dim)
    });
    let total = lg.size().y + sg.as_ref().map_or(0.0, |g| g.size().y + 2.0 * u);
    let top = rect.center().y - total / 2.0;
    let lw = lg.size().x;
    p.galley(pos2(x, top), lg.clone(), ink);
    if let Some((g, fill)) = tag {
        let size = g.size() + vec2(18.0 * u, 6.0 * u);
        let tr = Rect::from_min_size(pos2(x + lw + 10.0 * u, top + (lg.size().y - size.y) / 2.0 + 1.0 * u), size);
        p.rect_filled(tr, 999.0, fill);
        p.galley(tr.min + vec2(9.0 * u, 3.0 * u), g, FG);
    }
    if let Some(g) = sg {
        p.galley(pos2(x, top + lg.size().y + 2.0 * u), g, dim);
    }
    spec.enabled && focus.register(ui, &r)
}

/// A small heading over a group of rows.
fn section(ui: &mut Ui, text: &str, u: f32) {
    ui.add_space(8.0 * u);
    ui.label(egui::RichText::new(text.to_uppercase()).font(FontId::new(15.0 * u, theme::semibold())).color(DIM));
    ui.add_space(2.0 * u);
}

/// The tag a version shows in the Version list.
fn tag_of(g: &romset::Game) -> Option<(&'static str, Color32)> {
    match romset::variants::get(g.id)?.kind {
        VariantKind::Hack => Some(("Hack", Color32::from_rgba_unmultiplied(255, 150, 40, 110))),
        VariantKind::Prototype => Some(("Prototype", Color32::from_rgba_unmultiplied(170, 110, 255, 110))),
        _ => None,
    }
}

/// One line about a version for the Version list.
fn note_of(g: &romset::Game) -> String {
    let mut note = match romset::variants::get(g.id).map(|v| v.kind) {
        None => "The title's main set.",
        Some(VariantKind::Revision) => "Another official revision.",
        Some(VariantKind::Region) => "The same game for another market.",
        Some(VariantKind::CrossBoard) => "The same program on another board.",
        Some(VariantKind::Prototype) => "A prototype or location-test build.",
        Some(VariantKind::Hack) => "A modified set, not an official release.",
    }
    .to_string();
    if crate::link::title_links(g.id) && crate::link::play_for(g.id).is_none() {
        note += " Link play is off for this version.";
    }
    note
}

/// What there is to say about a version, a paragraph each (empty: nothing,
/// so the page has no About row).
fn about_of(g: &romset::Game, has_versions: bool) -> Vec<String> {
    let mut out = Vec::new();
    if has_versions && romset::variants::get(g.id).is_none() {
        out.push("The title's main set, which its other versions are made from.".to_string());
    }
    if let Some(v) = romset::variants::get(g.id) {
        let mut text = note_of(g);
        text = text.replace(" Link play is off for this version.", "");
        if !v.kind_confirmed {
            text += " Which kind it is rests on the set's name and chips; it is not confirmed.";
        }
        out.push(text);
    }
    if crate::link::title_links(g.id) {
        out.push(if crate::link::play_for(g.id).is_some() {
            "Link play works with this version, on one screen and over a network.".to_string()
        } else {
            "Link play is off for this version: its program has no link play, or its settings record has not been \
             checked against the title's, so reModel does not link it."
                .to_string()
        });
    }
    if let Some(v) = romset::variants::get(g.id) {
        out.push(if v.own_textures {
            "It has textures of its own: a texture pack made for another version does not apply to it.".to_string()
        } else {
            "A texture pack made for the title also works on this version.".to_string()
        });
    } else if has_versions {
        out.push("A texture pack made for it also works on the versions that share its textures.".to_string());
    }
    out
}

/// What the page draws from, gathered before it draws.
struct Info {
    card: &'static romset::Game,
    game: &'static romset::Game,
    versions: Vec<&'static romset::Game>,
    links: bool,
    link_ok: bool,
    machines: Option<usize>,
    on_screen: usize,
    hosted: Option<(String, usize, usize)>,
    about: Vec<String>,
    version_label: String,
    /// How many of the game's save files there are.
    used: usize,
}

impl Launcher {
    /// Open the selected game's page at `level` (a level the game lacks
    /// opens the main menu).
    pub(super) fn open_page(&mut self, level: Level, by_click: bool) {
        let Some(card) = self.cur else { return };
        let level = match level {
            Level::Version if self.versions_of(card).len() < 2 => Level::Main,
            Level::Together if !crate::link::title_links(card.id) => Level::Main,
            other => other,
        };
        let mut page = GamePage::new(level);
        if by_click {
            page.dbl_until = Some(Instant::now() + DOUBLE_CLICK);
        }
        if level == Level::Version {
            if let Some(v) = self.version() {
                page.want(Level::Version, v.id);
            }
        }
        self.page = Some(page);
    }

    /// Back a level, or from the main menu to the games.
    pub(super) fn page_back(&mut self) {
        let Some(page) = &self.page else { return };
        if page.level == Level::Main {
            self.page = None;
            self.pending_focus = Some((self.selected_id(), 6));
        } else {
            self.page_leave(page.level);
        }
    }

    /// Back to the main menu from `from`, the row that opened it focused.
    pub(super) fn page_leave(&mut self, from: Level) {
        if let Some(page) = &mut self.page {
            page.level = Level::Main;
            page.focus = Focus::default();
            page.want(Level::Main, from.row());
        }
    }

    /// Down to `level`.
    pub(super) fn page_enter(&mut self, level: Level) {
        let chosen = self.version().map(|g| g.id);
        let Some(page) = &mut self.page else { return };
        page.level = level;
        page.focus = Focus::default();
        page.pending = None;
        if let (Level::Version, Some(id)) = (level, chosen) {
            page.want(Level::Version, id);
        }
    }

    /// The selected game's save files and slots: read when the Saves level is entered.
    fn read_saves(ctx: &egui::Context, game: &'static romset::Game) -> Vec<SaveItem> {
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
        items
    }

    /// Draw the page and act on the input (`navs` are the pad's and the
    /// keyboard's; the page's own clicks go to `clicks`).
    pub(super) fn page_frame(
        &mut self,
        ctx: &egui::Context,
        l: &Layout,
        view: &View,
        navs: Vec<Nav>,
        clicks: &mut Vec<Click>,
    ) {
        let Some(mut page) = self.page.take() else { return };
        let (Some(card), Some(game)) = (self.cur, self.version()) else {
            self.page = Some(page);
            return;
        };
        let now = Instant::now();
        page.focus.show = self.pad_used || self.keys_used;
        // The second press of a double click that opened the page plays.
        if let Some(t) = page.dbl_until {
            if now >= t {
                page.dbl_until = None;
            } else if ctx.input(|i| i.pointer.primary_pressed()) {
                self.last_click = None;
                clicks.push(Click::Play);
                self.page = Some(page);
                return;
            }
        }
        let mut back = false;
        for nav in page.focus.begin(&navs) {
            match nav {
                Nav::Back => back = true,
                Nav::Continue => clicks.push(Click::Continue),
                _ => {}
            }
        }
        if page.used_for != Some(game.id) {
            page.used = std::iter::once(saves::resume_file(game.id))
                .chain((0..menu::SLOTS).map(|n| saves::slot_file(game.id, n)))
                .filter(|p| saves::read_info(p).is_some())
                .count();
            page.used_for = Some(game.id);
        }
        if page.level == Level::Saves && page.saves_for != Some(game.id) {
            page.saves = Self::read_saves(ctx, game);
            page.saves_for = Some(game.id);
        }
        let versions = self.versions_of(card);
        let play = crate::link::play_for(game.id);
        let info = Info {
            card,
            game,
            links: crate::link::title_links(card.id),
            link_ok: play.is_some(),
            machines: crate::link::title_machines(card.id),
            on_screen: play.map_or(2, crate::link::max_players),
            hosted: self.lan_hosts.iter().find(|f| f.game == game.id).map(|f| (f.name.clone(), f.players, f.max)),
            about: about_of(game, versions.len() > 1),
            used: page.used,
            version_label: if versions.len() > 1 { library::version_label(game) } else { String::new() },
            versions,
        };
        let u = l.u;
        let w = l.top.width();
        let side_w = (w * 0.4).clamp(430.0 * u, 640.0 * u).min(w * 0.6);
        let gap = 36.0 * u;
        let top = l.screen.top() + 40.0 * u;
        let bottom = l.bar.top() - 24.0 * u;
        let hero = Rect::from_min_max(pos2(l.top.left(), top), pos2(l.top.right() - side_w - gap, bottom));
        let side = Rect::from_min_max(pos2(l.top.right() - side_w, top), pos2(l.top.right(), bottom));
        let level = page.level;
        let area = egui::Area::new(Id::unique("launcher")).fixed_pos(l.screen.min).order(egui::Order::Background);
        area.show(ctx, |ui| {
            ui.set_min_size(l.screen.size());
            let p = ui.painter().clone();
            self.paint_page_background(&p, l, card);
            self.paint_hero(&p, l, hero, &info, view);
            self.paint_side(ui, &mut page, side, &info, view, u, clicks);
            let hints = self.page_hints(level);
            Self::paint_bar(ui, &p, l, view, &mut page.focus, hints, None, clicks);
        });
        page.focus.end();
        if let Some((id, tries)) = page.pending {
            page.focus.focus_to(id);
            page.pending = (page.focus.focused() != Some(id) && tries > 1).then_some((id, tries - 1));
        }
        self.page = Some(page);
        if back {
            clicks.push(Click::PageBack);
        }
    }

    /// The page's hint bar.
    fn page_hints(&self, level: Level) -> Vec<Hint> {
        let main = level == Level::Main;
        let select = if main { "Select" } else { "Choose" };
        let back = if main { "Back to games" } else { "Back" };
        let hint = |chip, label: &str, click| Hint { chip, label: label.to_string(), click };
        let mut h = if self.pad_used {
            vec![hint(Chip::A, select, None), hint(Chip::B, back, Some(Click::PageBack))]
        } else {
            vec![hint(Chip::Key("Enter"), select, None), hint(Chip::Key("Esc"), back, Some(Click::PageBack))]
        };
        if main {
            h.push(hint(if self.pad_used { Chip::Y } else { Chip::Key("C") }, "Continue", Some(Click::Continue)));
        }
        h
    }

    /// The page's background: the poster blurred, dark, over the backdrop.
    fn paint_page_background(&self, p: &Painter, l: &Layout, card: &romset::Game) {
        let s = l.screen;
        if let Some(tex) = self.blurs.get(card.id) {
            let [w, h] = tex.size();
            let (w, h) = (w as f32, h as f32);
            let k = (s.width() / w).max(s.height() / h);
            let crop = vec2(s.width() / (w * k), s.height() / (h * k));
            p.image(tex.id(), s, Rect::from_center_size(pos2(0.5, 0.5), crop), Color32::from_white_alpha(70));
        }
        vgradient(p, s, Color32::from_black_alpha(150), Color32::from_black_alpha(215));
    }

    /// The hero: the chosen version's attract preview in a frame over the
    /// poster blurred, the poster itself while there is no preview yet.
    fn paint_hero(&self, p: &Painter, l: &Layout, rect: Rect, info: &Info, view: &View) {
        let u = l.u;
        let radius = 18.0 * u;
        p.add(egui::epaint::RectShape::filled(rect, radius, Color32::from_black_alpha(120)).with_blur_width(30.0 * u));
        p.rect_filled(rect, radius, Color32::from_rgb(6, 9, 16));
        if let Some(tex) = self.blurs.get(info.card.id) {
            let [w, h] = tex.size();
            let (w, h) = (w as f32, h as f32);
            let k = (rect.width() / w).max(rect.height() / h);
            let crop = vec2(rect.width() / (w * k), rect.height() / (h * k));
            p.add(
                egui::epaint::RectShape::filled(rect, radius, Color32::from_white_alpha(120))
                    .with_texture(tex.id(), Rect::from_center_size(pos2(0.5, 0.5), crop)),
            );
        }
        let live = view.preview.filter(|v| v.alpha > 0.0);
        // The poster, standing in until the preview has faded in.
        let poster_alpha = live.map_or(1.0, |v| 1.0 - v.alpha);
        if let (Some(tex), true) = (self.posters.get(info.card.id), poster_alpha > 0.01) {
            let [w, h] = tex.size();
            let aspect = w as f32 / h as f32;
            let ph = rect.height() * 0.84;
            let r = Rect::from_center_size(rect.center(), vec2(ph * aspect, ph));
            p.add(
                egui::epaint::RectShape::filled(r, 12.0 * u, Color32::from_white_alpha((poster_alpha * 255.0) as u8))
                    .with_texture(tex.id(), full_uv()),
            );
        }
        if let Some(v) = live {
            let [px, py, pw, ph] = v.picture;
            let screen = l.screen;
            let aspect = (pw * screen.width()) / (ph * screen.height()).max(1.0);
            let avail = rect.shrink(14.0 * u);
            let (mut w, mut h) = (avail.width(), avail.width() / aspect);
            if h > avail.height() {
                (w, h) = (avail.height() * aspect, avail.height());
            }
            let pic = Rect::from_center_size(avail.center(), vec2(w, h));
            p.add(
                egui::epaint::RectShape::filled(pic, 10.0 * u, Color32::from_white_alpha((v.alpha * 255.0) as u8))
                    .with_texture(v.texture, Rect::from_min_size(pos2(px, py), vec2(pw, ph))),
            );
        }
        p.rect_stroke(
            rect,
            radius,
            Stroke::new(1.5 * u, Color32::from_rgba_unmultiplied(34, 229, 255, 90)),
            StrokeKind::Inside,
        );
        // What the preview is doing.
        let label = |t: &str| {
            if info.version_label.is_empty() {
                t.to_string()
            } else {
                format!("{t} · {}", info.version_label)
            }
        };
        let status = match (view.progress, view.no_preview) {
            (Some(f), _) => format!("Preparing the preview… {:.0}%", f * 100.0),
            (None, Some(why)) if why.contains(" not in ") => {
                "No preview: chips missing from the ROM folder".to_string()
            }
            (None, Some(why)) => format!("No preview: {why}"),
            _ if live.is_some() => label("Attract preview"),
            _ => label("Preview loading"),
        };
        let font = FontId::new(16.0 * u, theme::semibold());
        let g = p.layout_no_wrap(fit_text(p, &status, &font, rect.width() - 56.0 * u), font, FG);
        let r = Rect::from_min_size(
            pos2(rect.left() + 18.0 * u, rect.bottom() - 18.0 * u - g.size().y - 14.0 * u),
            g.size() + vec2(24.0 * u, 14.0 * u),
        );
        p.rect_filled(r, 999.0, Color32::from_black_alpha(170));
        p.galley(r.min + vec2(12.0 * u, 7.0 * u), g, FG);
    }

    /// The right of the page: the header, then the level's rows.
    #[allow(clippy::too_many_arguments)]
    fn paint_side(
        &mut self,
        ui: &mut Ui,
        page: &mut GamePage,
        side: Rect,
        info: &Info,
        view: &View,
        u: f32,
        clicks: &mut Vec<Click>,
    ) {
        let p = ui.painter().clone();
        let level = page.level;
        let game = info.game;
        let mut y = side.top();
        let crumb = if level == Level::Main { game.genre.to_string() } else { level.label().to_string() };
        let r = p.text(
            pos2(side.left(), y),
            Align2::LEFT_TOP,
            crumb.to_uppercase(),
            FontId::new(16.0 * u, theme::semibold()),
            theme::CYAN_LIGHT,
        );
        y = r.bottom() + 6.0 * u;
        let size = if level == Level::Main {
            match game.title.chars().count() {
                0..=14 => 46.0,
                15..=22 => 38.0,
                _ => 32.0,
            }
        } else {
            30.0
        };
        let title = p.layout(game.title.to_uppercase(), FontId::new(size * u, theme::display()), FG, side.width());
        p.galley(pos2(side.left(), y), title.clone(), FG);
        y += title.size().y + 12.0 * u;
        let chip_font = FontId::new(16.0 * u, theme::semibold());
        let mut facts = vec![game.board.name().to_string(), game.year.to_string()];
        if let Some(n) = info.machines {
            facts.push(format!("Links 2 to {n}"));
        }
        if !info.version_label.is_empty() {
            facts.push(info.version_label.clone());
        }
        let (mut x, mut row_h) = (side.left(), 0.0);
        for (i, text) in facts.into_iter().enumerate() {
            let last = i >= 2 && text == info.version_label;
            let (fill, ink) = if last {
                (Color32::from_rgba_unmultiplied(31, 107, 255, 150), FG)
            } else {
                (Color32::from_white_alpha(26), theme::TEXT_2)
            };
            let g = p.layout_no_wrap(text, chip_font.clone(), ink);
            let size = g.size() + vec2(24.0 * u, 12.0 * u);
            if x > side.left() && x + size.x > side.right() {
                x = side.left();
                y += row_h + 6.0 * u;
            }
            let r = Rect::from_min_size(pos2(x, y), size);
            p.rect_filled(r, 999.0, fill);
            p.galley(r.min + vec2(12.0 * u, 6.0 * u), g, ink);
            x = r.right() + 8.0 * u;
            row_h = size.y;
        }
        y += row_h + 18.0 * u;
        let body = Rect::from_min_max(pos2(side.left() - 8.0 * u, y), pos2(side.right() + 8.0 * u, side.bottom()));
        let (focus, saves) = (&mut page.focus, &page.saves);
        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
            focus.scroll_area(&format!("page-{level:?}")).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = ROW_GAP * u;
                ui.add_space(6.0 * u);
                // (The rows are inset, so the focus ring beside them isn't clipped.)
                ui.set_width(body.width() - 16.0 * u);
                ui.add_space(0.0);
                self.level_rows(ui, focus, saves, level, info, view, u, clicks);
                ui.add_space(8.0 * u);
            });
        });
    }

    /// The rows of one level.
    #[allow(clippy::too_many_arguments)]
    fn level_rows(
        &mut self,
        ui: &mut Ui,
        focus: &mut Focus,
        saves: &[SaveItem],
        level: Level,
        info: &Info,
        view: &View,
        u: f32,
        clicks: &mut Vec<Click>,
    ) {
        let go = |hit: bool, click: Click, clicks: &mut Vec<Click>| {
            if hit {
                clicks.push(click);
            }
        };
        match level {
            Level::Main => {
                let mut spec = Row::new("play", "Play");
                spec.primary = true;
                spec.right = info.version_label.clone();
                go(row(ui, focus, level, &spec, u), Click::Play, clicks);
                if info.versions.len() > 1 {
                    let spec = Row::new("version", "Version").right(info.version_label.clone()).chevron();
                    go(row(ui, focus, level, &spec, u), Click::Enter(Level::Version), clicks);
                }
                if info.links {
                    let mut spec = Row::new("together", "Play together").chevron();
                    spec.sub = if info.link_ok {
                        format!("2 to {} machines · this screen or over a network", info.machines.unwrap_or(2))
                    } else {
                        "Not for this version".to_string()
                    };
                    spec.enabled = info.link_ok;
                    go(row(ui, focus, level, &spec, u), Click::Enter(Level::Together), clicks);
                }
                let saved = info.used;
                let spec = Row::new("saves", "Saves")
                    .right(if saved == 0 { "None yet".to_string() } else { format!("{saved} saved") })
                    .chevron();
                go(row(ui, focus, level, &spec, u), Click::Enter(Level::Saves), clicks);
                let spec = Row::new("settings", "Game settings").right(view.settings.picture.clone()).chevron();
                go(row(ui, focus, level, &spec, u), Click::Enter(Level::Settings), clicks);
                if !info.about.is_empty() {
                    let spec = Row::new("about", "About this version").chevron();
                    go(row(ui, focus, level, &spec, u), Click::Enter(Level::About), clicks);
                }
            }
            Level::Version => {
                for v in &info.versions {
                    let mut spec = Row::new(v.id, library::version_label(v)).sub(note_of(v));
                    spec.tag = tag_of(v);
                    spec.radio = Some(v.id == info.game.id);
                    go(row(ui, focus, level, &spec, u), Click::SetVersion(v), clicks);
                }
            }
            Level::Together => {
                section(ui, "On this screen", u);
                for n in 2..=info.on_screen {
                    let spec = Row::new(format!("players{n}"), format!("{n} players")).right(if n == 2 {
                        "split"
                    } else {
                        "quadrants"
                    });
                    go(row(ui, focus, level, &spec, u), Click::Together(n), clicks);
                }
                section(ui, "Over the network", u);
                if let Some((name, players, max)) = &info.hosted {
                    let spec = Row::new("join hosted", format!("Join {name}'s game"))
                        .sub(format!("{players} of {max} players"));
                    go(row(ui, focus, level, &spec, u), Click::NetJoinHosted, clicks);
                }
                go(row(ui, focus, level, &Row::new("host", "Host a game"), u), Click::NetHost, clicks);
                let label = if info.hosted.is_some() { "Join another game" } else { "Join a game" };
                go(row(ui, focus, level, &Row::new("join", label), u), Click::NetJoin, clicks);
                section(ui, "Your name", u);
                let edit = egui::TextEdit::singleline(&mut self.player_name)
                    .char_limit(linknet::MAX_NAME)
                    .hint_text(linknet::default_name())
                    .desired_width(f32::INFINITY)
                    .font(FontId::new(18.0 * u, FontFamily::Proportional));
                if ui.add(edit).changed() {
                    self.name_edited = true;
                }
            }
            Level::Saves => {
                for (i, item) in saves.iter().enumerate() {
                    let id = Id::unique(("save-card", info.game.id, i));
                    if menu::slot_card(ui, focus, id, &item.name, false, item.info.as_ref(), item.thumb.as_ref())
                        && item.info.as_ref().is_some_and(|inf| inf.loads)
                    {
                        clicks.push(Click::Load(item.from));
                    }
                }
            }
            Level::Settings => {
                let st = &view.settings;
                let rows = [
                    (
                        "picture",
                        "Picture",
                        st.picture.clone(),
                        Click::SettingsAt(menu::Page::Picture, menu::Tab::Display),
                    ),
                    (
                        "controls",
                        "Controls",
                        String::new(),
                        Click::SettingsAt(menu::Page::Controls, menu::Tab::Display),
                    ),
                    (
                        "display",
                        "Display",
                        st.display.clone(),
                        Click::SettingsAt(menu::Page::Settings, menu::Tab::Display),
                    ),
                    ("sound", "Sound", st.sound.clone(), Click::SettingsAt(menu::Page::Settings, menu::Tab::Sound)),
                    (
                        "speed",
                        "Speed and latency",
                        st.speed.clone(),
                        Click::SettingsAt(menu::Page::Settings, menu::Tab::Motion),
                    ),
                ];
                for (key, label, right, click) in rows {
                    go(row(ui, focus, level, &Row::new(key, label).right(right).chevron(), u), click, clicks);
                }
                if let Some(fp) = &st.free_play {
                    let spec = Row::new("free play", "Free play").right(fp.clone()).chevron();
                    go(
                        row(ui, focus, level, &spec, u),
                        Click::SettingsAt(menu::Page::Settings, menu::Tab::More),
                        clicks,
                    );
                }
            }
            Level::About => {
                for text in &info.about {
                    ui.add_space(4.0 * u);
                    ui.label(egui::RichText::new(text).size(18.0 * u).color(theme::TEXT_2));
                    ui.add_space(6.0 * u);
                }
                ui.add_space(6.0 * u);
                go(row(ui, focus, level, &Row::new("back", "Back").chevron(), u), Click::PageBack, clicks);
            }
        }
    }
}
