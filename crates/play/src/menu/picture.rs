//! The menu's Picture and Advanced pages (docs/play.md, "Picture styles").
//!
//! Picture lists the styles, each a card with a name, a tag and one plain
//! sentence: Arcade original, Sharp, Smooth modern, CRT and Custom
//! (`Preset`; Smooth modern is the `enhanced` preset in the files). Choosing
//! one applies it at once to the paused game beside the panel. A switch
//! says where the change is kept (this game, or all games, as the settings'
//! layers have it), and Compare shows the game's picture as the arcade
//! original left of a divider and as chosen right of it: the divider
//! follows the pointer (drag) or the pad's left and right. Advanced has a
//! row for each picture setting; changing one makes the style Custom (the
//! one it came from is remembered, for the dots that mark what differs from
//! it and for "Back to the style's values"), and only the focused row says
//! what it does.

use super::{custom_relief, pill, regions, shading_hint, side_panel, Menu, Page, PanelSpec, Prefs, Step, View};
use super::{CREASES, RELIEF};
use crate::focus::Focus;
use crate::hints::{style_line, style_tag, BLOOM, BLOOM_HELP, CRT, CRT_HELP};
use crate::launcher::{Nav, DIM};
use crate::settings::{GameSettings, Preset, PICTURE_KEYS, PRESET_KEYS};
use crate::theme::{self, Chip};
use egui::{pos2, vec2, Align2, Color32, FontFamily, FontId, Rect, Sense, Shape, Stroke, StrokeKind};

/// What the pages keep between frames.
#[derive(Default)]
pub(super) struct State {
    /// The focus on each page.
    focus: Focus,
    adv_focus: Focus,
    /// Compare is on, and the divider's place across the visible picture:
    /// 0 (left edge) to 1.
    pub(super) compare: bool,
    pub(super) wipe: f32,
    /// The last Custom the player made (and the style it came from), for
    /// choosing Custom again after another style.
    stash: Option<(GameSettings, Option<Preset>)>,
    /// The style the Custom settings came from, once it was changed under
    /// a style.
    pub(super) base: Option<Preset>,
    /// Where to show the "before" picture this frame, if Compare is up:
    /// x, y, width, height in window pixels.
    pub wipe_rect: Option<(u32, u32, u32, u32)>,
}

impl State {
    /// Whether Compare's divider is up on `page`.
    pub fn comparing(&self, page: Page) -> bool {
        self.compare && page == Page::Picture
    }
}

/// The picture page's footer: Compare, Advanced and where it's saved, in
/// design points.
const FOOT_H: f32 = 124.0;
/// The Advanced page's footer: its one button.
const ADV_FOOT_H: f32 = 64.0;
/// How far the d-pad's moves put a pinned widget past (or before) a
/// scrolling list (`Focus::shift_last`).
const SHIFT: f32 = 100_000.0;
/// What a press of the pad's left or right moves the divider, of the
/// picture's width.
pub(super) const WIPE_STEP: f32 = 0.04;

impl Menu {
    /// Where the "before" picture goes in the window while Compare is up
    /// (x, y, width, height in pixels): left of the divider.
    pub fn wipe_rect(&self) -> Option<(u32, u32, u32, u32)> {
        self.picture.wipe_rect.filter(|_| self.open)
    }

    /// The style `s` is, or came from: the one Advanced's dots and "Back to
    /// the style's values" go by.
    fn base_style(&self, s: &GameSettings, game: Option<&romset::Game>) -> Preset {
        if s.preset != Preset::Custom {
            return s.preset;
        }
        self.picture.base.unwrap_or_else(|| nearest_style(s, game))
    }

    /// Choose style `p` for `s`: its rows, or for Custom the last ones the
    /// player made (kept when another style was chosen over them).
    fn choose_style(&mut self, s: &mut GameSettings, p: Preset, game: Option<&romset::Game>) {
        if p == s.preset {
            return;
        }
        if s.preset == Preset::Custom {
            self.picture.stash = Some((s.clone(), self.picture.base));
        }
        if p == Preset::Custom {
            let from = s.preset;
            match self.picture.stash.clone() {
                Some((c, base)) => {
                    for k in PRESET_KEYS {
                        s.set(k, &c.get(k));
                    }
                    self.picture.base = base;
                }
                None => self.picture.base = Some(from),
            }
            s.preset = Preset::Custom;
        } else {
            s.apply_preset(p, game);
            self.picture.base = None;
        }
    }

    /// The picture styles.
    pub(super) fn picture_page(
        &mut self,
        ctx: &egui::Context,
        settings: &mut GameSettings,
        prefs: Prefs,
        view: &View,
        navs: &[Nav],
    ) {
        let s = theme::scale(ctx);
        let nav_mode = self.pad_used || self.keys_used;
        self.picture.focus.show = nav_mode;
        let rest = self.picture.focus.begin(navs);
        let mut back = rest.contains(&Nav::Back);
        // Left and right that nothing took move the divider.
        let loose = self.picture.focus.take_loose();
        if self.picture.compare && loose != 0 {
            self.picture.wipe = (self.picture.wipe + WIPE_STEP * loose as f32).clamp(0.0, 1.0);
        }
        let reg = regions(ctx.content_rect(), s, FOOT_H);
        if self.picture.compare {
            self.wipe_overlay(ctx, settings.preset, view, reg.panel.left());
        }
        let name = view.playing.unwrap_or("This game");
        let short = if name.chars().count() > 16 { "This game" } else { name };
        let for_all = *prefs.for_all;
        let note = scope_note(name, for_all, view);
        let mut hints = vec![(Chip::A, "Select"), (Chip::B, "Back"), (Chip::Key("Esc"), "Resume")];
        if self.picture.compare {
            hints.insert(1, (Chip::LeftRight, "Divider"));
        }
        let spec = PanelSpec { game: name, heading: "Picture", back: true, extra_h: FOOT_H };
        let (mut chosen, mut to_advanced, mut scope, mut toggled) = (None, false, None, false);
        let (focus, compare, current) = (&mut self.picture.focus, self.picture.compare, settings.preset);
        let mut steps = 0;
        back |= side_panel(ctx, &spec, &hints, |ui, reg| {
            ui.scope_builder(egui::UiBuilder::new().max_rect(reg.body.shrink2(vec2(16.0 * s, 0.0))), |ui| {
                ui.add_space(4.0 * s);
                let labels = [format!("{short} only"), "All games".to_string()];
                scope = scope_toggle(ui, focus, &labels, for_all as usize);
                focus.shift_last(1, -SHIFT);
                ui.add_space(8.0 * s);
                focus.scroll_area("styles").show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0 * s;
                    for p in Preset::ALL {
                        if style_card(ui, focus, p, p == current, view.game) {
                            chosen = Some(p);
                        }
                    }
                });
            });
            let foot = reg.extra;
            ui.painter().hline(foot.x_range(), foot.top(), Stroke::new(1.0, Color32::from_white_alpha(20)));
            let (pad, h) = (16.0 * s, 38.0 * s);
            let x = foot.left() + pad;
            let w = foot.width() - 2.0 * pad;
            let at = |dy: f32| Rect::from_min_size(pos2(x, foot.top() + dy * s), vec2(w, h));
            let (t, n) = compare_row(ui, focus, at(8.0), compare);
            (toggled, steps) = (t, n);
            if pill(ui, focus, at(50.0), "Advanced ›", Some(theme::RAISED), true) {
                to_advanced = true;
            }
            focus.shift_last(2, SHIFT);
            let p = ui.painter();
            let galley = p.layout(note, FontId::new(14.0 * s, FontFamily::Proportional), theme::TEXT_DIM, w);
            p.galley(pos2(x, foot.top() + 94.0 * s), galley, theme::TEXT_DIM);
            false
        });
        self.picture.focus.end();
        if self.picture.compare && steps != 0 {
            self.picture.wipe = (self.picture.wipe + WIPE_STEP * steps as f32).clamp(0.0, 1.0);
        }
        if self.picture.compare {
            self.set_wipe_rect(ctx, view, reg.panel.left());
        }
        if let Some(i) = scope {
            *prefs.for_all = i == 1;
        }
        if toggled {
            self.picture.compare = !self.picture.compare;
            self.picture.wipe = 0.5;
        }
        if let Some(p) = chosen {
            self.choose_style(settings, p, view.game);
        }
        if to_advanced || back {
            self.picture.wipe_rect = None;
        }
        if to_advanced {
            self.page = Page::Advanced;
            self.picture.adv_focus = Focus::default();
            self.picture.compare = false;
        } else if back {
            self.back_to_pause();
            self.picture.compare = false;
        }
    }

    /// Compare's divider over the game's picture (left of the panel): the
    /// divider and its handle, the labels at the picture's top corners, and
    /// the pointer's drag. Sets `wipe_rect`: the part of the picture left of
    /// the divider, in pixels.
    fn wipe_overlay(&mut self, ctx: &egui::Context, style: Preset, view: &View, panel_left: f32) {
        let Some(pic) = view.picture else { return };
        let (ppp, s) = (ctx.pixels_per_point(), theme::scale(ctx));
        let at = |v: f32| v / ppp;
        let picture = Rect::from_min_size(pos2(at(pic[0]), at(pic[1])), vec2(at(pic[2]), at(pic[3])));
        // The picture's visible part: the panel covers the rest.
        let vis = Rect::from_min_max(picture.min, pos2(picture.right().min(panel_left), picture.bottom()));
        if vis.width() < 60.0 * s || vis.height() < 60.0 * s {
            return;
        }
        let wipe = &mut self.picture.wipe;
        egui::Area::new(egui::Id::unique("remodel-wipe")).order(egui::Order::Background).fixed_pos(vis.min).show(
            ctx,
            |ui| {
                let r = ui.allocate_rect(vis, Sense::click_and_drag());
                if r.hovered() || r.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                if r.dragged() || r.is_pointer_button_down_on() {
                    if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                        *wipe = ((p.x - vis.left()) / vis.width()).clamp(0.0, 1.0);
                    }
                }
                let x = vis.left() + *wipe * vis.width();
                let p = ui.painter();
                p.rect_filled(
                    Rect::from_x_y_ranges(x - 7.0 * s..=x + 7.0 * s, vis.y_range()),
                    0.0,
                    Color32::from_rgba_unmultiplied(34, 229, 255, 46),
                );
                p.rect_filled(Rect::from_x_y_ranges(x - 1.5 * s..=x + 1.5 * s, vis.y_range()), 0.0, theme::TEXT);
                let c = pos2(x, vis.center().y);
                p.circle_filled(c, 22.0 * s, theme::TEXT);
                p.circle_stroke(c, 20.0 * s, Stroke::new(4.0 * s, theme::CYAN));
                p.text(c, Align2::CENTER_CENTER, "◂▸", FontId::new(16.0 * s, theme::bold()), theme::INK);
                let tag = |p: &egui::Painter, pos: egui::Pos2, align: Align2, text: String, colour: Color32| {
                    let g = p.layout_no_wrap(text, FontId::new(16.0 * s, theme::semibold()), colour);
                    let size = g.size() + vec2(24.0 * s, 10.0 * s);
                    let rect = align.anchor_size(pos, size);
                    p.rect_filled(rect, 6.0 * s, Color32::from_black_alpha(178));
                    p.galley(rect.min + vec2(12.0 * s, 5.0 * s), g, colour);
                };
                let top = vis.top() + 14.0 * s;
                tag(
                    p,
                    pos2(vis.left() + 16.0 * s, top),
                    Align2::LEFT_TOP,
                    "Before: Arcade original".into(),
                    theme::TEXT,
                );
                tag(
                    p,
                    pos2(vis.right() - 16.0 * s, top),
                    Align2::RIGHT_TOP,
                    format!("After: {}", style.label()),
                    theme::CYAN_LIGHT,
                );
            },
        );
    }

    /// Set `wipe_rect` from the divider's place (after the pad moved it this
    /// frame).
    fn set_wipe_rect(&mut self, ctx: &egui::Context, view: &View, panel_left: f32) {
        let Some(pic) = view.picture else { return };
        let (ppp, s) = (ctx.pixels_per_point(), theme::scale(ctx));
        let right = (pic[0] + pic[2]).min(panel_left * ppp);
        if right - pic[0] < 60.0 * s * ppp || pic[3] < 60.0 * s * ppp {
            return;
        }
        let split = pic[0] + self.picture.wipe * (right - pic[0]);
        let (x0, y0) = (pic[0] as u32, pic[1] as u32);
        let w = (split.round() as u32).saturating_sub(x0);
        self.picture.wipe_rect = Some((x0, y0, w, pic[3] as u32));
    }

    /// The Advanced page: a row for each picture setting.
    pub(super) fn advanced_page(
        &mut self,
        ctx: &egui::Context,
        settings: &mut GameSettings,
        prefs: Prefs,
        view: &View,
        navs: &[Nav],
    ) {
        let s = theme::scale(ctx);
        let nav_mode = self.pad_used || self.keys_used;
        self.picture.adv_focus.show = nav_mode;
        let rest = self.picture.adv_focus.begin(navs);
        let back = rest.contains(&Nav::Back);
        let was = settings.preset;
        let base_style = self.base_style(settings, view.game);
        let mut base = GameSettings::default();
        base.apply_preset(base_style, view.game);
        let name = if settings.preset == Preset::Custom {
            format!("Custom (from {})", base_style.label())
        } else {
            settings.preset.label().to_string()
        };
        let scope = if *prefs.for_all {
            "All games".to_string()
        } else {
            format!("{} only", view.playing.unwrap_or("This game"))
        };
        let differs = settings.preset == Preset::Custom || PRESET_KEYS.iter().any(|k| settings.get(k) != base.get(k));
        let focus = &mut self.picture.adv_focus;
        let mut adv = Adv { focus, nav_mode, base: &base };
        let hints = [(Chip::LeftRight, "Change"), (Chip::B, "Back"), (Chip::Key("Esc"), "Resume")];
        let spec =
            PanelSpec { game: view.playing.unwrap_or("reModel"), heading: "Advanced", back: true, extra_h: ADV_FOOT_H };
        let mut reset = false;
        let mut back = back;
        back |= side_panel(ctx, &spec, &hints, |ui, reg| {
            ui.scope_builder(egui::UiBuilder::new().max_rect(reg.body.shrink2(vec2(16.0 * s, 0.0))), |ui| {
                // One line, cut short with "…" rather than widening the page.
                let mut job = egui::text::LayoutJob::default();
                let dim = FontId::new(14.0 * s, FontFamily::Proportional);
                let part = |font: &FontId, color: Color32| egui::TextFormat {
                    font_id: font.clone(),
                    color,
                    ..Default::default()
                };
                job.append("Style: ", 14.0 * s, part(&dim, theme::TEXT_DIM));
                job.append(&name, 0.0, part(&FontId::new(14.0 * s, theme::semibold()), theme::TEXT));
                job.append(&format!(" · {scope}"), 0.0, part(&dim, theme::TEXT_DIM));
                ui.add(egui::Label::new(job).truncate());
                ui.add_space(6.0 * s);
                adv.focus.scroll_area("advanced").show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0 * s;
                    advanced_rows(ui, &mut adv, settings, view);
                });
            });
            let foot = reg.extra;
            ui.painter().hline(foot.x_range(), foot.top(), Stroke::new(1.0, Color32::from_white_alpha(20)));
            let r = Rect::from_center_size(foot.center(), vec2(foot.width() - 32.0 * s, 42.0 * s));
            if pill(ui, adv.focus, r, "Back to the style's values", Some(theme::RAISED), differs) {
                reset = true;
            }
            adv.focus.shift_last(1, SHIFT);
            false
        });
        self.picture.adv_focus.end();
        if reset {
            settings.apply_preset(base_style, view.game);
            self.picture.base = None;
        }
        // A style whose picture rows no longer all match is Custom, from
        // the style it was.
        *settings = settings.clone().settled(view.game);
        if was != Preset::Custom && settings.preset == Preset::Custom {
            self.picture.base = Some(was);
        }
        if back {
            self.page = Page::Picture;
            self.picture.focus = Focus::default();
        }
    }
}

/// The style closest to `s`'s picture rows: the one most of them match.
fn nearest_style(s: &GameSettings, game: Option<&romset::Game>) -> Preset {
    let score = |p: Preset| {
        let mut want = s.clone();
        want.apply_preset(p, game);
        PICTURE_KEYS.iter().filter(|k| want.get(k) == s.get(k)).count()
    };
    let all = [Preset::Original, Preset::Sharp, Preset::Enhanced, Preset::Crt, Preset::Comic];
    all.into_iter().max_by_key(|p| (score(*p), std::cmp::Reverse(all.iter().position(|q| q == p)))).unwrap_or_default()
}

/// Where the choice is kept, and what the other games have: the Picture
/// page's last line.
fn scope_note(game: &str, for_all: bool, view: &View) -> String {
    if !for_all {
        return format!("Saved for {game}. Other games keep {}.", view.keeps.all_games.label());
    }
    if view.keeps.own_picture {
        format!("Saved for all games. {game}'s own picture settings are replaced.")
    } else {
        format!("Saved for all games ({game} has no style of its own).")
    }
}

/// The scope switch ("<Game> only", "All games"): the pad moves it as one
/// row (left and right choose, A switches). Returns the new choice.
pub(super) fn scope_toggle(
    ui: &mut egui::Ui,
    focus: &mut Focus,
    labels: &[String; 2],
    selected: usize,
) -> Option<usize> {
    let s = theme::scale(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0 * s), Sense::hover());
    let id = ui.make_persistent_id("picture-scope");
    let r = ui.interact(rect, id, Sense::click());
    let (pad, accepted) = focus.stepper(ui, &r);
    let p = ui.painter().clone();
    let radius = theme::ROW_RADIUS * s;
    p.rect_filled(rect, radius, Color32::from_white_alpha(18));
    let half = (rect.width() - 8.0 * s) / 2.0;
    for (i, label) in labels.iter().enumerate() {
        let seg = Rect::from_min_size(
            pos2(rect.left() + 4.0 * s + i as f32 * half, rect.top() + 4.0 * s),
            vec2(half, rect.height() - 8.0 * s),
        );
        let on = i == selected;
        if on {
            p.rect_filled(seg, radius - 3.0 * s, theme::TEXT);
        }
        let font = FontId::new(15.0 * s, theme::semibold());
        let colour = if on { theme::INK } else { theme::TEXT };
        let mut text = label.clone();
        // A name too long for its half is cut short.
        while p.layout_no_wrap(text.clone(), font.clone(), colour).size().x > half - 12.0 * s && text.len() > 4 {
            text.pop();
        }
        p.text(seg.center(), Align2::CENTER_CENTER, text, font, colour);
    }
    if focus.is_on(id) {
        theme::focus_ring(&p, rect, s, theme::ROW_RADIUS);
    }
    let mut to = None;
    if r.clicked() {
        to = r.interact_pointer_pos().map(|at| (at.x >= rect.center().x) as usize);
    }
    if pad != 0 {
        to = Some((pad > 0) as usize);
    }
    if accepted {
        to = Some(1 - selected);
    }
    to.filter(|t| *t != selected)
}

/// A style's card: name, tag and sentence; the chosen one has a cyan border
/// and a faint fill. Clicked, or activated from the pad.
fn style_card(ui: &mut egui::Ui, focus: &mut Focus, p: Preset, on: bool, game: Option<&romset::Game>) -> bool {
    let s = theme::scale(ui.ctx());
    let w = ui.available_width();
    let painter = ui.painter().clone();
    let name = painter.layout_no_wrap(p.label().to_string(), FontId::new(18.0 * s, theme::bold()), theme::TEXT);
    let tag = style_tag(p);
    let tag = (!tag.is_empty())
        .then(|| painter.layout_no_wrap(tag.to_string(), FontId::new(14.0 * s, theme::semibold()), theme::CYAN_LIGHT));
    let line = painter.layout(
        style_line(p, game),
        FontId::new(14.0 * s, FontFamily::Proportional),
        theme::TEXT_2,
        w - 28.0 * s,
    );
    let h = 9.0 * s + name.size().y + 2.0 * s + line.size().y + 9.0 * s;
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let id = ui.make_persistent_id(("style-card", p.name()));
    let r = ui.interact(rect, id, Sense::click());
    let radius = theme::ROW_RADIUS * s;
    if on {
        painter.rect_filled(rect, radius, Color32::from_rgba_unmultiplied(34, 229, 255, 30));
        painter.rect_stroke(rect, radius, Stroke::new(2.0 * s, theme::CYAN), StrokeKind::Inside);
    } else {
        painter.rect_filled(rect, radius, Color32::from_white_alpha(if r.hovered() { 14 } else { 8 }));
    }
    let x = rect.left() + 14.0 * s;
    let top = rect.top() + 9.0 * s;
    let name_h = name.size().y;
    let name_w = name.size().x;
    painter.galley(pos2(x, top), name, theme::TEXT);
    if let Some(tag) = tag {
        let at = pos2(x + name_w + 10.0 * s, top + (name_h - tag.size().y) / 2.0 + 1.0 * s);
        painter.galley(at, tag, theme::CYAN_LIGHT);
    }
    painter.galley(pos2(x, top + name_h + 2.0 * s), line, theme::TEXT_2);
    if focus.is_on(id) {
        theme::focus_ring(&painter, rect, s, theme::ROW_RADIUS);
    }
    focus.register(ui, &r)
}

/// The Compare row: a switch, and with it on the pad's left and right move
/// the divider. Returns whether it was switched and how far the pad moved
/// the divider.
fn compare_row(ui: &mut egui::Ui, focus: &mut Focus, rect: Rect, on: bool) -> (bool, i32) {
    let s = theme::scale(ui.ctx());
    let id = ui.make_persistent_id("compare-row");
    let r = ui.interact(rect, id, Sense::click());
    let (pad, accepted) = focus.stepper(ui, &r);
    let p = ui.painter().clone();
    let radius = theme::ROW_RADIUS * s;
    let font = FontId::new(16.0 * s, theme::bold());
    if on {
        p.rect_filled(rect, radius, theme::CYAN);
        p.text(rect.center(), Align2::CENTER_CENTER, "Compare", font.clone(), theme::INK);
        for (dir, side) in [(-1.0, rect.left() + 24.0 * s), (1.0, rect.right() - 24.0 * s)] {
            let c = pos2(side, rect.center().y);
            let tip = c + vec2(6.0 * dir, 0.0) * s;
            let back = c - vec2(4.0 * dir, 0.0) * s;
            p.add(Shape::convex_polygon(
                vec![tip, back + vec2(0.0, -7.0 * s), back + vec2(0.0, 7.0 * s)],
                theme::INK,
                Stroke::NONE,
            ));
        }
    } else {
        if r.hovered() {
            p.rect_filled(rect, radius, Color32::from_white_alpha(18));
        }
        p.rect_stroke(rect, radius, Stroke::new(1.0, Color32::from_white_alpha(51)), StrokeKind::Inside);
        p.text(rect.center(), Align2::CENTER_CENTER, "Compare", font, theme::TEXT);
    }
    if focus.is_on(id) {
        theme::focus_ring(&p, rect, s, theme::ROW_RADIUS);
    }
    (r.clicked() || accepted, if on { pad } else { 0 })
}

/// The Advanced page's rows as they are drawn: the focus, and the style's
/// values (a dot marks a row that differs).
struct Adv<'a> {
    focus: &'a mut Focus,
    nav_mode: bool,
    base: &'a GameSettings,
}

impl Adv<'_> {
    /// One row: label, a dot if `changed`, "◀ value ▶". The focused row
    /// alone shows `help` (and the dim `cost` after it). Clicking a row
    /// focuses it; its arrows step it; A steps on.
    #[allow(clippy::too_many_arguments)]
    fn row(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        value: &str,
        can: (bool, bool),
        changed: bool,
        help: &str,
        cost: &str,
    ) -> Step {
        let s = theme::scale(ui.ctx());
        let id = ui.make_persistent_id(("adv-row", label));
        let open = self.focus.focused() == Some(id);
        let width = ui.available_width();
        let painter = ui.painter().clone();
        let galley = open.then(|| {
            let mut job = egui::text::LayoutJob::default();
            job.wrap.max_width = width - 28.0 * s;
            let font = FontId::new(14.0 * s, FontFamily::Proportional);
            job.append(
                help,
                0.0,
                egui::TextFormat { font_id: font.clone(), color: theme::TEXT_2, ..Default::default() },
            );
            if !cost.is_empty() {
                job.append(
                    &format!(" {cost}"),
                    0.0,
                    egui::TextFormat { font_id: font, color: theme::TEXT_DIM, ..Default::default() },
                );
            }
            painter.layout_job(job)
        });
        let head = 46.0 * s;
        let h = head + galley.as_ref().map_or(0.0, |g| g.size().y + 10.0 * s);
        let (rect, _) = ui.allocate_exact_size(vec2(width, h), Sense::hover());
        let r = ui.interact(rect, id, Sense::click());
        let (pad, accepted) = self.focus.stepper(ui, &r);
        let radius = theme::ROW_RADIUS * s;
        let ring = self.focus.is_on(id);
        if open && !ring {
            painter.rect_filled(rect, radius, Color32::from_rgba_unmultiplied(34, 229, 255, 26));
            painter.rect_stroke(rect, radius, Stroke::new(2.0 * s, theme::CYAN), StrokeKind::Inside);
        } else if r.hovered() && !self.nav_mode && !open {
            painter.rect_filled(rect, radius, Color32::from_white_alpha(14));
        }
        let mid = rect.top() + head / 2.0;
        let btn = 32.0 * s;
        let right = Rect::from_center_size(pos2(rect.right() - 12.0 * s - btn / 2.0, mid), vec2(btn, btn));
        let value_w = 96.0 * s;
        let left = Rect::from_center_size(pos2(right.left() - value_w - btn / 2.0, mid), vec2(btn, btn));
        for (b, glyph, on) in [(left, "◀", can.0), (right, "▶", can.1)] {
            painter.rect_filled(b, 9.0 * s, Color32::from_white_alpha(20));
            let c = if on { theme::TEXT } else { DIM.gamma_multiply(0.45) };
            painter.text(b.center(), Align2::CENTER_CENTER, glyph, FontId::new(13.0 * s, FontFamily::Proportional), c);
        }
        painter.text(
            pos2(left.right() + value_w / 2.0, mid),
            Align2::CENTER_CENTER,
            value,
            FontId::new(16.0 * s, theme::bold()),
            theme::CYAN_LIGHT,
        );
        painter.text(
            pos2(rect.left() + 14.0 * s, mid),
            Align2::LEFT_CENTER,
            label,
            FontId::new(16.0 * s, theme::semibold()),
            theme::TEXT,
        );
        if changed {
            painter.circle_filled(pos2(left.left() - 12.0 * s, mid), 4.0 * s, theme::YELLOW);
        }
        if let Some(g) = galley {
            painter.galley(pos2(rect.left() + 14.0 * s, rect.top() + head), g, theme::TEXT_2);
        }
        if ring {
            theme::focus_ring(&painter, rect, s, theme::ROW_RADIUS);
        }
        if r.clicked() {
            match r.interact_pointer_pos() {
                Some(at) if left.contains(at) => return Step::Dec,
                Some(at) if right.contains(at) => return Step::Inc,
                _ => self.focus.focus_to(id),
            }
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

    /// A row choosing `value` among `options` (`keys`: the settings it
    /// stands for, for the dot): whether it changed.
    #[allow(clippy::too_many_arguments)]
    fn choose<T: Copy + PartialEq>(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        changed: bool,
        value: &mut T,
        options: &[(T, &str)],
        help: &str,
        cost: &str,
    ) -> bool {
        let at = options.iter().position(|o| o.0 == *value);
        let n = options.len();
        let text = at.map_or("", |i| options[i].1);
        let can = at.map_or((false, false), |i| (i > 0, i + 1 < n));
        let to = match (self.row(ui, label, text, can, changed, help, cost), at) {
            (Step::Dec, Some(i)) if i > 0 => i - 1,
            (Step::Inc, Some(i)) if i + 1 < n => i + 1,
            (Step::Cycle, Some(i)) => (i + 1) % n,
            _ => return false,
        };
        *value = options[to].0;
        true
    }

    /// An off/on row (`off` and `on` are the words).
    #[allow(clippy::too_many_arguments)]
    fn toggle(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        changed: bool,
        value: &mut bool,
        words: (&str, &str),
        help: &str,
        cost: &str,
    ) -> bool {
        self.choose(ui, label, changed, value, &[(false, words.0), (true, words.1)], help, cost)
    }

    /// Whether any of `keys` differs from the style's value.
    fn differs(&self, s: &GameSettings, keys: &[&str]) -> bool {
        keys.iter().any(|k| s.get(k) != self.base.get(k))
    }
}

/// A heading above a group of rows.
fn group(ui: &mut egui::Ui, text: &str) {
    let s = theme::scale(ui.ctx());
    ui.add_space(10.0 * s);
    ui.horizontal(|ui| {
        ui.add_space(14.0 * s);
        ui.label(egui::RichText::new(text).font(FontId::new(13.0 * s, theme::semibold())).color(theme::TEXT_DIM));
    });
    ui.add_space(4.0 * s);
}

/// The strength rows' choices, with the one set by flag or file that isn't
/// among them.
fn with_current(list: &[(u8, &str)], now: u8) -> Vec<(u8, String)> {
    let mut out: Vec<(u8, String)> = list.iter().map(|(n, t)| (*n, t.to_string())).collect();
    if !out.iter().any(|o| o.0 == now) {
        out.push((now, format!("{now}%")));
    }
    out
}

/// The Advanced page's rows, for the settings that apply as things are set
/// (the rest are left out, as in the other tabs).
fn advanced_rows(ui: &mut egui::Ui, adv: &mut Adv, s: &mut GameSettings, view: &View) {
    let temporal = s.temporal_on() && view.temporal_supported;
    // Resolution: with temporal smoothing, the share of the screen's
    // pixels the 3D is drawn with.
    if temporal {
        let d = adv.differs(s, &["temporal"]);
        let mut list: Vec<(u8, &str)> = vec![(100, "Screen's"), (75, "75%"), (67, "67%"), (50, "50%")];
        let extra = (!list.iter().any(|o| o.0 == s.temporal)).then(|| format!("{}%", s.temporal));
        if let Some(t) = &extra {
            list.push((s.temporal, t));
        }
        adv.choose(
            ui,
            "Resolution",
            d,
            &mut s.temporal,
            &list,
            "How many pixels the 3D is drawn with, then scaled up to your screen.",
            "Fewer is easier on the GPU.",
        );
    }
    if !temporal {
        let d = adv.differs(s, &["aa"]);
        adv.choose(
            ui,
            "Edge smoothing",
            d,
            &mut s.aa,
            &[(0, "Off"), (4, "4×"), (8, "8×")],
            "Softens jagged edges on polygons.",
            "Costs GPU memory.",
        );
    }
    if !s.geo_chip && view.temporal_supported {
        let d = adv.differs(s, &["temporal"]);
        let mut on = s.temporal > 0;
        if adv.toggle(
            ui,
            "Temporal smoothing",
            d,
            &mut on,
            ("Off", "On"),
            "Blends frames to remove shimmer on fine detail, and takes the place of edge smoothing. Can look a \
             little soft.",
            "Costs GPU time.",
        ) {
            s.temporal = if on { 100 } else { 0 };
        }
    }
    let d = adv.differs(s, &["shading"]);
    let note = lighting_note(s);
    let lighting = [(gpu::Shading::Flat, "Flat"), (gpu::Shading::Vertex, "Smooth"), (gpu::Shading::Pixel, "Per pixel")];
    adv.choose(
        ui,
        "Lighting",
        d,
        &mut s.shading,
        &lighting,
        "Flat is the board's own; smooth blends light across each polygon, and per pixel works it out for every \
         pixel.",
        note,
    );
    if view.temporal_supported {
        let d = adv.differs(s, &["bloom"]);
        let list = with_current(&BLOOM, s.bloom);
        let list: Vec<(u8, &str)> = list.iter().map(|(v, t)| (*v, t.as_str())).collect();
        adv.choose(ui, "Glow", d, &mut s.bloom, &list, BLOOM_HELP, "");
    }
    if !s.geo_chip {
        let d = adv.differs(s, &["smooth"]);
        adv.toggle(
            ui,
            "In-between frames",
            d,
            &mut s.smooth,
            ("Off", "On"),
            "Draws extra frames between the game's own for screens faster than 60 Hz. Motion only: the game is \
             unchanged.",
            "Shows each a frame later.",
        );
    }
    if s.wide {
        let mut edges = s.hud_edges;
        if adv.choose(
            ui,
            "Wide screens",
            false,
            &mut edges,
            &[(true, "HUD at edges"), (false, "Centre HUD")],
            "Where the game's 4:3 score and timer go on a wide screen.",
            "",
        ) {
            s.hud_edges = edges;
        }
    }
    let d = adv.differs(s, &["pack"]);
    let pack_help = match (s.pack, view.pack) {
        (true, Some(text)) => text,
        _ => "Replacement textures from this game's texpack folder, if it has one.",
    };
    adv.toggle(ui, "Texture pack", d, &mut s.pack, ("None", "On"), pack_help, "");
    let d = adv.differs(s, &["crt"]);
    let list = with_current(&CRT, s.crt);
    let list: Vec<(u8, &str)> = list.iter().map(|(v, t)| (*v, t.as_str())).collect();
    adv.choose(ui, "CRT", d, &mut s.crt, &list, CRT_HELP, "Looks best on a tall screen.");
    let d = adv.differs(s, &["look"]);
    adv.toggle(
        ui,
        "Comic look",
        d,
        &mut s.look,
        ("Off", "On"),
        "Ink outlines, flat bands of light and halftone dots. Uses the texture pack's look.txt when it has one.",
        "Needs smooth lighting for the bands.",
    );
    if let (true, Some(look)) = (s.look, view.look) {
        look_rows(ui, adv, s, look);
    }

    // More: the picture settings that don't fit above.
    group(ui, "MORE");
    let d = adv.differs(s, &["depth"]);
    adv.toggle(
        ui,
        "Depth buffer",
        d,
        &mut s.depth,
        ("Off", "On"),
        "Polygons that cross sort right. Off draws them in the game's order.",
        "",
    );
    if temporal {
        let d = adv.differs(s, &["stipple"]);
        adv.choose(
            ui,
            "See-through polygons",
            d,
            &mut s.stipple_keep,
            &[(false, "Even blend"), (true, "Checkerboard")],
            "How the games' checkerboard shadows and glass look with temporal smoothing: an even blend, or the \
             original pattern.",
            "",
        );
    }
    if s.shading != gpu::Shading::Flat {
        let d = adv.differs(s, &["crease"]);
        let game_crease = view.game.map_or(romset::Shading::DEFAULT.crease, |g| g.shading.crease);
        let mut list = vec![(gpu::CREASE_AUTO, format!("The game's ({game_crease}°)"))];
        list.extend(CREASES.iter().map(|c| (*c, format!("{c}°"))));
        if !list.iter().any(|o| o.0 == s.crease) {
            list.push((s.crease, if s.crease == 0 { "0° (none)".into() } else { format!("{}°", s.crease) }));
        }
        let list: Vec<(u8, &str)> = list.iter().map(|(v, t)| (*v, t.as_str())).collect();
        adv.choose(
            ui,
            "Crease angle",
            d,
            &mut s.crease,
            &list,
            "Faces meeting at a sharper angle than this keep a hard edge: boxes and limbs stay faceted, bodies \
             smooth.",
            "",
        );
    }
    if s.light_on() {
        let d = adv.differs(s, &["shading_fine"]);
        adv.toggle(
            ui,
            "Blend brightness levels",
            d,
            &mut s.shading_fine,
            ("Off", "On"),
            "No banding on large smooth surfaces. Off keeps the game's 64 levels.",
            "",
        );
        let d = adv.differs(s, &["shading_balance"]);
        adv.toggle(
            ui,
            "Brightness balance",
            d,
            &mut s.shading_balance,
            ("Off", "On"),
            "Each smooth surface as bright as the game's. Off, bumpy surfaces come out brighter.",
            "",
        );
    }
    if s.pack && !s.geo_chip {
        let d = adv.differs(s, &["materials"]);
        let help = match (s.materials, view.materials) {
            (true, Some(text)) => text,
            _ => "The pack's normal maps: bumps and grooves lit by the game's own lights.",
        };
        adv.toggle(ui, "Material maps", d, &mut s.materials, ("Off", "On"), help, "");
    }
    if s.materials_on() && !s.geo_chip {
        let d = adv.differs(s, &["materials_relief"]);
        let custom = custom_relief(s.materials_relief);
        let mut list: Vec<(gpu::Relief, &str)> =
            RELIEF.iter().filter_map(|(text, v)| gpu::Relief::parse(v).map(|r| (r, *text))).collect();
        if let Some(text) = &custom {
            list.push((s.materials_relief, text.as_str()));
        }
        adv.choose(
            ui,
            "Material relief",
            d,
            &mut s.materials_relief,
            &list,
            "How strongly the bumps shade, 2 by default. Faithful keeps the game's own lighting sum: subtler.",
            "",
        );
    }
    if s.pack {
        let d = adv.differs(s, &["tex_filter"]);
        let mut list: Vec<(model2::enhance::TexFilter, &str)> = crate::settings::tex_filters(view.max_anisotropy)
            .iter()
            .filter_map(|(label, v)| model2::enhance::TexFilter::parse(v).map(|f| (f, *label)))
            .collect();
        if !list.iter().any(|o| o.0 == s.tex_filter) {
            list.push((s.tex_filter, "Not on this GPU"));
        }
        adv.choose(
            ui,
            "Texture filtering",
            d,
            &mut s.tex_filter,
            &list,
            "Of the texture pack's images: mipmaps stop the shimmer at a distance, anisotropic filtering adds \
             sharpness.",
            "",
        );
    }
}

/// The look's rows: each parameter as steps over its range; a change goes
/// into the settings' tuning (over the look file's value), live.
fn look_rows(ui: &mut egui::Ui, adv: &mut Adv, s: &mut GameSettings, look: &model2::look::Look) {
    use model2::look::{Kind, PARAMS, SPEED_LINES};
    group(ui, "LOOK");
    for p in PARAMS.iter().chain([&SPEED_LINES]).filter(|p| !p.label.is_empty()) {
        let now = look.params.num(p.name);
        let text = |v: f32| match p.kind {
            Kind::Flag => (if v >= 0.5 { "On" } else { "Off" }).to_string(),
            Kind::Steps => {
                if v < 2.0 {
                    "Off".to_string()
                } else {
                    format!("{}", v as u32)
                }
            }
            _ if p.step >= 1.0 => format!("{v:.0}"),
            _ => format!("{v:.2}"),
        };
        // The grid of values, and the current one if it isn't on it.
        let step = if p.max >= 1000.0 { 1000.0 } else { p.step };
        let mut vals: Vec<f32> = match p.kind {
            Kind::Flag => vec![0.0, 1.0],
            Kind::Steps => std::iter::once(0.0).chain((2..=p.max as u32).map(|n| n as f32)).collect(),
            _ => (0..=((p.max - p.min) / step).round() as u32).map(|i| p.min + i as f32 * step).collect(),
        };
        let key = |v: f32| (v * 1000.0).round() as i64;
        if !vals.iter().any(|v| key(*v) == key(now)) {
            vals.push(now);
            vals.sort_by(|a, b| a.total_cmp(b));
        }
        let list: Vec<(i64, String)> = vals.iter().map(|v| (key(*v), text(*v))).collect();
        let list: Vec<(i64, &str)> = list.iter().map(|(k, t)| (*k, t.as_str())).collect();
        let mut cur = key(now);
        let changed = s.look_tuning.contains_key(p.name);
        if adv.choose(ui, p.label, changed, &mut cur, &list, p.help, "") {
            let v = cur as f32 / 1000.0;
            let value = match p.kind {
                Kind::Flag => (if v >= 0.5 { "on" } else { "off" }).to_string(),
                Kind::Steps => format!("{}", v as u32),
                _ => format!("{v}"),
            };
            s.look_tuning.insert(p.name.to_string(), value);
        }
    }
    let mut reset = false;
    if !s.look_tuning.is_empty()
        && adv.choose(
            ui,
            "Look tuning",
            true,
            &mut reset,
            &[(false, "Changed"), (true, "Reset")],
            "Go back to the look file's own values.",
            "",
        )
        && reset
    {
        s.look_tuning.clear();
    }
}

/// The Lighting row's dim note when the setting isn't doing what it says
/// (`shading_hint`'s cases that matter).
fn lighting_note(s: &GameSettings) -> &'static str {
    match shading_hint(s) {
        h if s.shading != gpu::Shading::Flat && (s.crease == 0 || s.geo_chip) => h,
        _ => "",
    }
}
