//! Photo mode's dock: a compact translucent strip along the bottom edge,
//! never a panel over the picture. Above, the active tab's settings in one
//! row of cells (each "name, value, < >"); below, the tab bar (Camera,
//! Time, Focus, Look, Effects, Frame) with Hide UI, the thirds grid, Leave
//! and the shutter. LB and RB switch tabs; on the pad the d-pad's up and
//! down walk the cells and left and right change the one focused (or
//! move along the bar); A on a cell takes the photo, as it always did, and
//! on a button presses it. The picture the shot is drawn from never
//! includes any of this.

use super::{Action, Mode, Row};
use crate::focus::Focus as PadFocus;
use crate::launcher::Nav;
use crate::theme::{self, CYAN, INK, RED, TEXT, TEXT_DIM};
use egui::{pos2, vec2, Align2, Color32, FontFamily, FontId, Id, Order, Rect, Sense, Stroke, StrokeKind, Ui};
use std::time::Instant;

/// The dock's size in design points (720p): the inner margins, the row of
/// cells, the gap, the tab bar, and the space left below it. All of it
/// (`FOOTPRINT`) is the most the dock takes of the window's height.
const PAD_X: f32 = 14.0;
const PAD_Y: f32 = 8.0;
const CELLS: f32 = 58.0;
const GAP: f32 = 6.0;
const BAR: f32 = 48.0;
const BOTTOM: f32 = 10.0;
/// The dock's width at most.
const WIDTH: f32 = 1180.0;
/// Columns the cells are laid out in (the most any tab has).
const COLUMNS: usize = 6;

/// The dock and its margin, in design points: the strip it takes.
#[cfg(test)]
pub const FOOTPRINT: f32 = 2.0 * PAD_Y + CELLS + GAP + BAR + BOTTOM;

/// The dock's tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Camera,
    Time,
    Focus,
    Look,
    Effects,
    Frame,
}

pub const TABS: [Tab; 6] = [Tab::Camera, Tab::Time, Tab::Focus, Tab::Look, Tab::Effects, Tab::Frame];

impl Tab {
    pub fn name(self) -> &'static str {
        match self {
            Tab::Camera => "Camera",
            Tab::Time => "Time",
            Tab::Focus => "Focus",
            Tab::Look => "Look",
            Tab::Effects => "Effects",
            Tab::Frame => "Frame",
        }
    }

    /// The settings on this tab, in order (those that do nothing as
    /// things are are left out, and a tab with none is not shown).
    pub fn rows(self) -> &'static [Row] {
        match self {
            Tab::Camera => &[Row::Zoom, Row::Roll, Row::Near, Row::Reset],
            Tab::Time => &[Row::Frame],
            Tab::Focus => &[Row::Dof, Row::Focus, Row::Aperture, Row::Bokeh],
            Tab::Look => &[Row::Exposure, Row::Contrast, Row::Saturation, Row::Warmth, Row::Tint, Row::Shading],
            Tab::Effects => &[Row::Vignette, Row::Grain, Row::Aberration, Row::Bloom, Row::Blur, Row::Shutter],
            Tab::Frame => &[Row::Letterbox, Row::Resolution, Row::Hud, Row::Background, Row::Depth],
        }
    }
}

/// The dock's state between redraws.
#[derive(Default)]
pub struct Dock {
    pub focus: PadFocus,
    /// Navigation presses (pad, keys) for the next redraw.
    pub navs: Vec<Nav>,
    /// The rule-of-thirds grid over the picture.
    pub grid: bool,
    /// The d-pad direction held: pressed at, repeated at.
    pub held: Option<(Nav, Instant, Instant)>,
    /// The dock as last drawn: its top edge and height (points), and the
    /// cells' widgets.
    pub top: f32,
    pub height: f32,
    pub cells: Vec<Id>,
    last_cell: Option<Id>,
    /// A mouse press held on a cell's arrow: its widget and how many
    /// repeats it has made.
    hold: Option<(Id, i64)>,
}

/// One cell: a setting's name and value as the dock says them.
struct Cell {
    row: Row,
    name: String,
    value: String,
    /// A switch: the step a click on its middle makes.
    toggle: Option<i32>,
}

/// What a redraw's clicks and presses asked for.
#[derive(Default)]
struct Out {
    adjust: Vec<(Row, i32)>,
    take: bool,
    leave: bool,
    hide: bool,
    grid: bool,
    tab: Option<Tab>,
    scrub: Option<u64>,
    cells: Vec<Id>,
}

/// The time cell's numbers.
struct TimeInfo {
    /// Where the frame shown is along the scrub, 0 (oldest) to 1 (live).
    at: f32,
    oldest: String,
}

/// The name as the dock writes it (short: a cell is a sixth of the dock).
fn name_of(row: Row, name: &str) -> String {
    match row {
        Row::Aberration => "Aberration",
        Row::Bokeh => "Bokeh",
        Row::Aperture => "Aperture",
        _ => name,
    }
    .to_string()
}

/// The value as the dock writes it.
fn value_of(row: Row, value: String) -> String {
    match row {
        Row::Zoom => value.trim_end_matches(" zoom").to_string(),
        Row::Near => value.trim_end_matches(" of the scene").to_string(),
        Row::Shutter => value.trim_end_matches(" of a frame").to_string(),
        Row::Shading => value.replace("The game's setting", "Game's"),
        _ => value,
    }
}

/// `picture` with the letterbox's bars taken off: where the photo is.
pub fn framed(picture: Rect, letterbox: f32) -> Rect {
    if letterbox <= 0.0 || picture.height() <= 0.0 {
        return picture;
    }
    let (w, h) = (picture.width(), picture.height());
    let size = if letterbox > w / h { vec2(w, w / letterbox) } else { vec2(h * letterbox, h) };
    Rect::from_center_size(picture.center(), size)
}

impl Mode {
    /// A cell for each of the tab's settings that does something now.
    fn cells(&self) -> Vec<Cell> {
        self.tab
            .rows()
            .iter()
            .copied()
            .filter(|r| self.shows(*r))
            .map(|row| {
                let (name, value) = self.text(row);
                let toggle = match row {
                    Row::Dof => Some(if self.dof { -1 } else { 1 }),
                    Row::Hud => Some(if self.opts.hud { -1 } else { 1 }),
                    Row::Background => Some(if self.opts.hide_base { 1 } else { -1 }),
                    Row::Depth => Some(if self.opts.depth { -1 } else { 1 }),
                    _ => None,
                };
                Cell { row, name: name_of(row, name), value: value_of(row, value), toggle }
            })
            .collect()
    }

    fn time_info(&self) -> Option<TimeInfo> {
        let (a, _) = self.range?;
        let span = self.live.saturating_sub(a).max(1) as f32;
        let at = self.wanted.unwrap_or(self.frame);
        Some(TimeInfo {
            at: (at.saturating_sub(a) as f32 / span).clamp(0.0, 1.0),
            oldest: format!("−{:.1} s", self.live.saturating_sub(a) as f64 / model2::pacing::FPS),
        })
    }

    /// The dock over the photo: draws it, and returns what was asked for
    /// (taking a photo, leaving). `picture` is where the picture is in the
    /// window (the thirds grid goes over it), `readout` a line on the
    /// photo about to be taken.
    pub fn ui(&mut self, ctx: &egui::Context, picture: Rect, readout: &str) -> Vec<Action> {
        let mut actions = Vec::new();
        let s = theme::scale(ctx);
        if self.dock.grid {
            paint_grid(ctx, framed(picture, self.look.letterbox));
        }
        let navs = std::mem::take(&mut self.dock.navs);
        if !self.ui {
            self.dock.top = ctx.content_rect().bottom();
            self.dock.height = 0.0;
            if show_pill(ctx, s) {
                self.ui = true;
            }
            return actions;
        }
        readout_chip(ctx, s, readout);
        let tabs = self.tabs();
        if !tabs.contains(&self.tab) {
            self.tab = tabs[0];
        }
        let cells = self.cells();
        let time = self.time_info().filter(|_| self.tab == Tab::Time);
        let tab = self.tab;
        let grid = self.dock.grid;

        let dock = &mut self.dock;
        dock.focus.show = self.pad_used;
        let navs = dock.steer(navs);
        dock.focus.begin(&navs);

        let screen = ctx.content_rect();
        let w = (WIDTH * s).min(screen.width() - 24.0 * s);
        let h = (2.0 * PAD_Y + CELLS + GAP + BAR) * s;
        let pos = pos2(screen.center().x - w / 2.0, screen.bottom() - BOTTOM * s - h);
        let range = self.range;
        let live = self.live;
        let mut out = Out::default();
        egui::Area::new(Id::unique("photo-dock")).order(Order::Middle).fixed_pos(pos).show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::click_and_drag());
            let p = ui.painter().clone();
            p.rect_filled(rect, 16.0 * s, Color32::from_rgba_unmultiplied(6, 9, 16, 208));
            p.rect_stroke(rect, 16.0 * s, Stroke::new(1.0, Color32::from_white_alpha(30)), StrokeKind::Inside);
            let inner = rect.shrink2(vec2(PAD_X, PAD_Y) * s);
            let cells_row = Rect::from_min_size(inner.min, vec2(inner.width(), CELLS * s));
            let bar =
                Rect::from_min_size(pos2(inner.left(), cells_row.bottom() + GAP * s), vec2(inner.width(), BAR * s));
            let gap = 8.0 * s;
            let cw = (cells_row.width() - gap * (COLUMNS - 1) as f32) / COLUMNS as f32;
            if let (Some(t), Some(c)) = (&time, cells.first()) {
                time_cell(ui, &mut dock.focus, cells_row, c, t, (range.map_or(0, |r| r.0), live), &mut out);
            } else {
                for (i, c) in cells.iter().enumerate() {
                    let r = Rect::from_min_size(
                        pos2(cells_row.left() + i as f32 * (cw + gap), cells_row.top()),
                        vec2(cw, cells_row.height()),
                    );
                    cell(ui, &mut dock.focus, &mut dock.hold, r, c, &mut out);
                }
            }
            tab_bar(ui, &mut dock.focus, bar, &tabs, tab, grid, &mut out);
        });
        dock.top = pos.y;
        dock.height = h;
        dock.focus.end();
        dock.cells = std::mem::take(&mut out.cells);
        if let Some(on) = dock.focus.focused().filter(|id| dock.cells.contains(id)) {
            dock.last_cell = Some(on);
        }
        if out.hide {
            self.ui = false;
        }
        if let Some(t) = out.tab {
            self.tab = t;
        }
        if out.grid {
            self.dock.grid = !self.dock.grid;
        }
        if out.take {
            actions.push(Action::Take);
        }
        if out.leave {
            actions.push(Action::Leave);
        }
        if let Some(f) = out.scrub {
            self.wanted = Some(f);
        }
        for (row, d) in out.adjust {
            self.adjust(row, d);
        }
        actions
    }
}

impl Dock {
    /// Up and down walk the cells (left and right change the focused one);
    /// down off the last goes to the bar, up from the bar back to the cell
    /// it came from. The rest is the focus's: the moves along the bar.
    fn steer(&mut self, navs: Vec<Nav>) -> Vec<Nav> {
        let mut rest = Vec::new();
        for nav in navs {
            let on = self.focus.focused();
            let cell = on.and_then(|id| self.cells.iter().position(|c| *c == id));
            match (nav, cell, on) {
                (Nav::Up, Some(i), _) => {
                    if i > 0 {
                        self.focus.focus_to(self.cells[i - 1]);
                    }
                }
                (Nav::Down, Some(i), _) if i + 1 < self.cells.len() => self.focus.focus_to(self.cells[i + 1]),
                (Nav::Up, None, Some(_)) => {
                    let back = self.last_cell.filter(|c| self.cells.contains(c)).or(self.cells.first().copied());
                    if let Some(c) = back {
                        self.focus.focus_to(c);
                    }
                }
                _ => rest.push(nav),
            }
        }
        rest
    }
}

fn font(s: f32) -> FontId {
    FontId::new(18.0 * s, FontFamily::Proportional)
}

fn semibold(s: f32) -> FontId {
    FontId::new(18.0 * s, theme::semibold())
}

/// A triangle pointing `dir` (-1 left) at `c`.
fn arrow(p: &egui::Painter, c: egui::Pos2, dir: f32, s: f32, colour: Color32) {
    let tip = c + vec2(6.0 * dir, 0.0) * s;
    let back = c - vec2(4.0 * dir, 0.0) * s;
    p.add(egui::Shape::convex_polygon(
        vec![tip, back + vec2(0.0, -7.0 * s), back + vec2(0.0, 7.0 * s)],
        colour,
        Stroke::NONE,
    ));
}

/// One setting: name over value, arrows at the right. The arrows step by a
/// click (repeating while held); the middle of a switch toggles it. From
/// the pad, left and right step it, A takes the photo.
fn cell(ui: &mut Ui, focus: &mut PadFocus, hold: &mut Option<(Id, i64)>, rect: Rect, c: &Cell, out: &mut Out) {
    let s = theme::scale(ui.ctx());
    let id = ui.make_persistent_id(("photo-cell", c.name.as_str()));
    let r = ui.interact(rect, id, Sense::click());
    out.cells.push(id);
    let radius = theme::ROW_RADIUS * s;
    let p = ui.painter().clone();
    p.rect_filled(rect, radius, Color32::from_white_alpha(if r.hovered() { 26 } else { 14 }));
    if c.row == Row::Reset {
        if focus.is_on(id) {
            theme::focus_ring(&p, rect, s, theme::ROW_RADIUS);
        }
        p.text(rect.center(), Align2::CENTER_CENTER, &c.name, semibold(s), TEXT);
        if focus.register(ui, &r) {
            out.adjust.push((Row::Reset, 1));
        }
        return;
    }
    let (steps, accepted) = focus.stepper(ui, &r);
    let arrow_w = 28.0 * s;
    let right = Rect::from_min_max(
        pos2(rect.right() - arrow_w - 4.0 * s, rect.top()),
        pos2(rect.right() - 4.0 * s, rect.bottom()),
    );
    let left = Rect::from_min_max(pos2(right.left() - arrow_w, rect.top()), pos2(right.left(), rect.bottom()));
    let text = p.with_clip_rect(Rect::from_min_max(rect.min, pos2(left.left(), rect.bottom())));
    text.text(pos2(rect.left() + 12.0 * s, rect.top() + 5.0 * s), Align2::LEFT_TOP, &c.name, font(s), TEXT_DIM);
    text.text(pos2(rect.left() + 12.0 * s, rect.top() + 29.0 * s), Align2::LEFT_TOP, &c.value, semibold(s), TEXT);
    for (rect, dir) in [(left, -1.0), (right, 1.0)] {
        arrow(&p, pos2(rect.center().x, rect.center().y + 12.0 * s), dir, s, CYAN);
    }
    for _ in 0..steps.abs() {
        out.adjust.push((c.row, steps.signum()));
    }
    out.take |= accepted;
    // The mouse: an arrow acts when pressed, then repeats while held.
    let zone = |at: Option<egui::Pos2>| match at {
        Some(at) if left.contains(at) => -1,
        Some(at) if right.contains(at) => 1,
        _ => 0,
    };
    let was_held = hold.is_some_and(|h| h.0 == id);
    if r.is_pointer_button_down_on() {
        let dir = zone(r.interact_pointer_pos());
        if dir != 0 {
            let t = ui.ctx().input(|i| i.pointer.press_start_time().map(|st| i.time - st)).unwrap_or(0.0);
            let k = if t < 0.4 { -1 } else { ((t - 0.4) / 0.07) as i64 };
            match *hold {
                Some((h, n)) if h == id => {
                    if k > n {
                        out.adjust.push((c.row, dir));
                        *hold = Some((id, k));
                    }
                }
                _ => {
                    out.adjust.push((c.row, dir));
                    *hold = Some((id, -1));
                }
            }
            ui.ctx().request_repaint();
        }
    } else if was_held {
        *hold = None;
    }
    if r.clicked() && !was_held {
        match (zone(r.interact_pointer_pos()), c.toggle) {
            (0, Some(d)) => out.adjust.push((c.row, d)),
            (0, None) => {}
            (d, _) => out.adjust.push((c.row, d)),
        }
    }
}

/// The time scrub as a strip of 20 marks over the history, the frame shown
/// lit: click or drag along it, or left and right from the pad.
fn time_cell(
    ui: &mut Ui,
    focus: &mut PadFocus,
    rect: Rect,
    c: &Cell,
    t: &TimeInfo,
    (oldest, live): (u64, u64),
    out: &mut Out,
) {
    let s = theme::scale(ui.ctx());
    let id = ui.make_persistent_id("photo-time");
    let r = ui.interact(rect, id, Sense::click_and_drag());
    out.cells.push(id);
    let p = ui.painter().clone();
    p.rect_filled(rect, theme::ROW_RADIUS * s, Color32::from_white_alpha(if r.hovered() { 26 } else { 14 }));
    let (steps, accepted) = focus.stepper(ui, &r);
    for _ in 0..steps.abs() {
        out.adjust.push((Row::Frame, steps.signum()));
    }
    out.take |= accepted;
    let top = rect.top() + 4.0 * s;
    let x0 = rect.left() + 14.0 * s;
    let x1 = rect.right() - 14.0 * s;
    p.text(pos2(x0, top), Align2::LEFT_TOP, &t.oldest, font(s), TEXT_DIM);
    p.text(pos2((x0 + x1) / 2.0, top), Align2::CENTER_TOP, &c.value, semibold(s), TEXT);
    p.text(pos2(x1, top), Align2::RIGHT_TOP, "Live", font(s), TEXT_DIM);
    let strip = Rect::from_min_max(pos2(x0, rect.top() + 29.0 * s), pos2(x1, rect.bottom() - 6.0 * s));
    let n = 20;
    let seg = (strip.width() - 3.0 * s * (n - 1) as f32) / n as f32;
    let lit = ((t.at * n as f32) as usize).min(n - 1);
    for i in 0..n {
        let x = strip.left() + i as f32 * (seg + 3.0 * s);
        let colour = if i == lit { CYAN } else { Color32::from_white_alpha(36 + (i % 4) as u8 * 10) };
        p.rect_filled(Rect::from_min_size(pos2(x, strip.top()), vec2(seg, strip.height())), 3.0 * s, colour);
    }
    if r.is_pointer_button_down_on() || r.clicked() {
        if let Some(at) = r.interact_pointer_pos() {
            let f = ((at.x - strip.left()) / strip.width().max(1.0)).clamp(0.0, 1.0);
            out.scrub = Some(oldest + (f * live.saturating_sub(oldest) as f32).round() as u64);
        }
    }
}

/// A button of the bar in `rect`: `on` fills it cyan (the tab shown, the
/// grid on). Clicked, or pressed from the pad.
fn pill(ui: &mut Ui, focus: &mut PadFocus, rect: Rect, id: Id, text: &str, on: bool) -> bool {
    let s = theme::scale(ui.ctx());
    let r = ui.interact(rect, id, Sense::click());
    let p = ui.painter();
    let radius = theme::ROW_RADIUS * s;
    if on {
        p.rect_filled(rect, radius, CYAN);
    } else if r.hovered() {
        p.rect_filled(rect, radius, Color32::from_white_alpha(24));
    }
    p.text(rect.center(), Align2::CENTER_CENTER, text, semibold(s), if on { INK } else { TEXT });
    if focus.is_on(id) {
        theme::focus_ring(p, rect, s, theme::ROW_RADIUS);
    }
    focus.register(ui, &r)
}

fn text_width(ui: &Ui, text: &str, s: f32) -> f32 {
    ui.painter().layout_no_wrap(text.to_string(), semibold(s), TEXT).size().x + 32.0 * s
}

/// The bar: Hide, the tabs, then at the right the grid, Leave and the
/// shutter.
fn tab_bar(ui: &mut Ui, focus: &mut PadFocus, bar: Rect, tabs: &[Tab], tab: Tab, grid: bool, out: &mut Out) {
    let s = theme::scale(ui.ctx());
    let h = 40.0 * s;
    let y = bar.center().y - h / 2.0;
    let gap = 6.0 * s;
    let mut x = bar.left();
    let mut place = |ui: &Ui, text: &str| {
        let w = text_width(ui, text, s);
        let r = Rect::from_min_size(pos2(x, y), vec2(w, h));
        x += w + gap;
        r
    };
    let hide = place(ui, "Hide (Y)");
    let tab_rects: Vec<Rect> = tabs.iter().map(|t| place(ui, t.name())).collect();
    let id = ui.make_persistent_id("photo-hide");
    if pill(ui, focus, hide, id, "Hide (Y)", false) {
        out.hide = true;
    }
    let sep = hide.right() + gap / 2.0;
    ui.painter().vline(
        sep,
        (bar.center().y - 16.0 * s)..=(bar.center().y + 16.0 * s),
        Stroke::new(1.0, Color32::from_white_alpha(40)),
    );
    for (&t, &r) in tabs.iter().zip(&tab_rects) {
        let id = ui.make_persistent_id(("photo-tab", t.name()));
        if pill(ui, focus, r, id, t.name(), t == tab) {
            out.tab = Some(t);
        }
    }
    // From the right.
    let shutter = Rect::from_center_size(pos2(bar.right() - 24.0 * s, bar.center().y), vec2(44.0, 44.0) * s);
    let id = ui.make_persistent_id("photo-shutter");
    let r = ui.interact(shutter, id, Sense::click());
    let p = ui.painter();
    let fill = if r.hovered() { Color32::from_rgb(255, 92, 106) } else { RED };
    p.circle_filled(shutter.center(), 18.0 * s, fill);
    p.circle_stroke(shutter.center(), 20.0 * s, Stroke::new(3.0 * s, Color32::WHITE));
    if focus.is_on(id) {
        theme::focus_ring(p, shutter.expand(2.0 * s), s, 24.0);
    }
    if focus.register(ui, &r) {
        out.take = true;
    }
    let mut right = shutter.left() - gap - 4.0 * s;
    for (text, id) in [("Leave (B)", "photo-leave"), ("Grid", "photo-grid")] {
        let w = text_width(ui, text, s);
        let r = Rect::from_min_size(pos2(right - w, y), vec2(w, h));
        right -= w + gap;
        let on = text == "Grid" && grid;
        let id = ui.make_persistent_id(id);
        if pill(ui, focus, r, id, text, on) {
            if text == "Grid" {
                out.grid = true;
            } else {
                out.leave = true;
            }
        }
    }
}

/// The corner chip: "PHOTO" and the photo's line.
fn readout_chip(ctx: &egui::Context, s: f32, readout: &str) {
    egui::Area::new(Id::unique("photo-readout"))
        .order(Order::Middle)
        .anchor(Align2::LEFT_TOP, [24.0 * s, 20.0 * s])
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(0, 0, 0, 115))
                .corner_radius(10.0 * s)
                .inner_margin(egui::Margin::symmetric((14.0 * s) as i8, (7.0 * s) as i8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 14.0 * s;
                        ui.label(
                            egui::RichText::new("PHOTO").font(FontId::new(18.0 * s, theme::display())).color(CYAN),
                        );
                        ui.label(egui::RichText::new(readout).font(font(s)).color(theme::TEXT_2));
                    });
                });
        });
}

/// With the dock hidden: a small way back (also Y, H).
fn show_pill(ctx: &egui::Context, s: f32) -> bool {
    let text = "Show controls (Y)";
    let galley = ctx.layer_painter(egui::LayerId::debug()).layout_no_wrap(text.to_string(), semibold(s), TEXT);
    let size = vec2(galley.size().x + 32.0 * s, 40.0 * s);
    let screen = ctx.content_rect();
    let pos = pos2(screen.right() - 24.0 * s - size.x, screen.bottom() - 20.0 * s - size.y);
    egui::Area::new(Id::unique("photo-show"))
        .order(Order::Middle)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            let (rect, r) = ui.allocate_exact_size(size, Sense::click());
            let fill = if r.hovered() { 150 } else { 105 };
            ui.painter().rect_filled(rect, theme::ROW_RADIUS * s, Color32::from_black_alpha(fill));
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, semibold(s), theme::TEXT_2);
            r.clicked()
        })
        .inner
}

/// The thirds over `r`.
fn paint_grid(ctx: &egui::Context, r: Rect) {
    let p = ctx.layer_painter(egui::LayerId::new(Order::Background, Id::unique("photo-thirds")));
    let line = Stroke::new(1.0, Color32::from_white_alpha(90));
    for k in 1..3 {
        let f = k as f32 / 3.0;
        let x = r.left() + r.width() * f;
        let y = r.top() + r.height() * f;
        p.vline(x, r.y_range(), line);
        p.hline(r.x_range(), y, line);
    }
}

#[cfg(test)]
mod tests;
