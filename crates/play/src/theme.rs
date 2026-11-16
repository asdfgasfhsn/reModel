//! The player's shared look: palette, fonts, type scale, and the few widgets
//! every screen is built from (panel, menu row, segmented control, button
//! hint chips, corner toasts). The design is laid out at 1280x720 logical
//! points and scales with the window's height from there (`scale`), so one
//! layout serves a laptop window and a 4K screen alike.
//!
//! Headings are Exo 2 Black Italic in capitals, everything else Inter; both
//! are bundled (`assets/fonts/`, SIL OFL 1.1).
// The screens still to be moved onto this (see docs/play.md) use the rest.
#![allow(dead_code)]

use egui::{
    pos2, vec2, Align2, Color32, Context, FontData, FontDefinitions, FontFamily, FontId, Id, Painter, Rect, Response,
    RichText, Sense, Stroke, StrokeKind, TextStyle, Ui,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The design's height in logical points: scale 1.0.
pub const DESIGN_HEIGHT: f32 = 720.0;
/// The smallest scale: below it a window's text would drop under legible.
pub const MIN_SCALE: f32 = 0.85;

pub const BG: Color32 = Color32::from_rgb(4, 6, 11);
/// A panel's fill (97% opaque) and its edge (cyan at 35%).
pub const PANEL_FILL: Color32 = Color32::from_rgba_unmultiplied_const(8, 12, 22, 247);
pub const PANEL_EDGE: Color32 = Color32::from_rgba_unmultiplied_const(34, 229, 255, 89);
/// A raised element on a panel (a pill, a button): white at 8%.
pub const RAISED: Color32 = Color32::from_rgba_premultiplied(20, 20, 20, 20);
pub const TEXT: Color32 = Color32::from_rgb(255, 255, 255);
pub const TEXT_2: Color32 = Color32::from_rgb(211, 218, 232);
pub const TEXT_DIM: Color32 = Color32::from_rgb(154, 166, 189);
pub const CYAN: Color32 = Color32::from_rgb(34, 229, 255);
pub const CYAN_LIGHT: Color32 = Color32::from_rgb(143, 243, 255);
pub const BLUE: Color32 = Color32::from_rgb(31, 107, 255);
pub const RED: Color32 = Color32::from_rgb(255, 42, 61);
pub const YELLOW: Color32 = Color32::from_rgb(255, 210, 63);
pub const GREEN: Color32 = Color32::from_rgb(47, 211, 107);
/// Dark text on a bright fill (a chip's letter, the selected segment).
pub const INK: Color32 = BG;
/// A focused row's fill: cyan at 12%.
const FOCUS_FILL: Color32 = Color32::from_rgba_unmultiplied_const(34, 229, 255, 31);

/// Radii in points at scale 1.
pub const ROW_RADIUS: f32 = 12.0;
pub const CARD_RADIUS: f32 = 16.0;

/// The heading family (Exo 2 Black Italic); semibold and bold Inter.
pub fn display() -> FontFamily {
    FontFamily::Name("display".into())
}
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}
pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

/// Install the fonts. Inter leads the proportional family (the stock fonts
/// stay behind it for the symbols Inter lacks).
fn install_fonts(ctx: &Context) {
    let mut f = FontDefinitions::default();
    let faces: [(&str, &'static [u8]); 4] = [
        ("inter", include_bytes!("../assets/fonts/inter/Inter-Regular.ttf")),
        ("inter-semibold", include_bytes!("../assets/fonts/inter/Inter-SemiBold.ttf")),
        ("inter-bold", include_bytes!("../assets/fonts/inter/Inter-Bold.ttf")),
        ("exo2-black-italic", include_bytes!("../assets/fonts/exo2/Exo2-BlackItalic.ttf")),
    ];
    for (name, bytes) in faces {
        f.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    }
    let stock = f.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let with = |first: &str| std::iter::once(first.to_owned()).chain(stock.iter().cloned()).collect::<Vec<_>>();
    f.families.insert(FontFamily::Proportional, with("inter"));
    f.families.insert(semibold(), with("inter-semibold"));
    f.families.insert(bold(), with("inter-bold"));
    f.families.insert(display(), with("exo2-black-italic"));
    ctx.set_fonts(f);
}

/// The UI scale for a window `height` logical points tall.
pub fn scale_for(height: f32) -> f32 {
    (height / DESIGN_HEIGHT).max(MIN_SCALE)
}

/// The UI scale now: 1.0 at 720 points tall, with a floor.
pub fn scale(ctx: &Context) -> f32 {
    scale_for(ctx.content_rect().height())
}

/// `v` design points in this window's.
pub fn px(ctx: &Context, v: f32) -> f32 {
    v * scale(ctx)
}

fn shadow(offset_y: i8, blur: u8, alpha: u8) -> egui::Shadow {
    egui::Shadow { offset: [0, offset_y], blur, spread: 0, color: Color32::from_black_alpha(alpha) }
}

/// egui's style at scale `s`: the palette, the type scale (body 18, small
/// 15, heading 28 points at 720p), radii and spacing.
fn style_at(ctx: &Context, s: f32) {
    ctx.all_styles_mut(|st| {
        let v = &mut st.visuals;
        *v = egui::Visuals::dark();
        v.window_fill = PANEL_FILL;
        v.panel_fill = PANEL_FILL;
        v.extreme_bg_color = BG;
        v.faint_bg_color = Color32::from_white_alpha(8);
        v.window_stroke = Stroke::new(1.0, PANEL_EDGE);
        v.window_corner_radius = (CARD_RADIUS * s).into();
        v.menu_corner_radius = (ROW_RADIUS * s).into();
        v.window_shadow = shadow(12, 36, 140);
        v.selection.bg_fill = Color32::from_rgba_unmultiplied(34, 229, 255, 90);
        v.selection.stroke = Stroke::new(1.0, CYAN);
        v.hyperlink_color = CYAN;
        v.override_text_color = Some(TEXT);
        let r: egui::CornerRadius = (ROW_RADIUS * s).into();
        let w = &mut v.widgets;
        w.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_white_alpha(28));
        for x in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
            x.corner_radius = r;
        }
        w.inactive.bg_fill = RAISED;
        w.inactive.weak_bg_fill = RAISED;
        w.hovered.bg_fill = Color32::from_white_alpha(36);
        w.hovered.weak_bg_fill = Color32::from_white_alpha(36);
        w.hovered.bg_stroke = Stroke::new(1.0, CYAN);
        w.active.bg_fill = Color32::from_rgba_unmultiplied(34, 229, 255, 70);
        w.active.weak_bg_fill = Color32::from_rgba_unmultiplied(34, 229, 255, 70);
        w.active.bg_stroke = Stroke::new(1.5, CYAN);
        st.spacing.button_padding = vec2(14.0, 7.0) * s;
        st.spacing.item_spacing = vec2(10.0, 8.0) * s;
        st.spacing.interact_size.y = 30.0 * s;
        st.text_styles = [
            (TextStyle::Heading, FontId::new(28.0 * s, display())),
            (TextStyle::Body, FontId::new(18.0 * s, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(18.0 * s, semibold())),
            (TextStyle::Small, FontId::new(15.0 * s, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(15.0 * s, FontFamily::Monospace)),
        ]
        .into();
    });
}

/// Set the fonts and the style up (once, at start).
pub fn install(ctx: &Context) {
    install_fonts(ctx);
    style_at(ctx, 1.0);
    ctx.data_mut(|d| d.insert_temp(Id::unique("theme-scale"), 1.0f32));
}

/// Follow the window: re-style when the UI scale (for a window `height`
/// logical points tall) has changed. Cheap when it hasn't.
pub fn sync(ctx: &Context, height: f32) {
    let s = scale_for(height);
    let was = ctx.data(|d| d.get_temp::<f32>(Id::unique("theme-scale")));
    if was.is_none_or(|w| (w - s).abs() > 0.005) {
        style_at(ctx, s);
        ctx.data_mut(|d| d.insert_temp(Id::unique("theme-scale"), s));
    }
}

/// A heading's text: capitals in Exo 2 Black Italic, `size` points at 720p.
pub fn heading(ctx: &Context, text: &str, size: f32) -> RichText {
    RichText::new(text.to_uppercase()).font(FontId::new(px(ctx, size), display())).color(TEXT)
}

/// A panel: the dark, nearly opaque card with its cyan edge.
pub fn panel_frame(ctx: &Context) -> egui::Frame {
    let s = scale(ctx);
    egui::Frame::new()
        .fill(PANEL_FILL)
        .stroke(Stroke::new(1.0, PANEL_EDGE))
        .corner_radius(CARD_RADIUS * s)
        .inner_margin(egui::Margin::same((20.0 * s) as i8))
        .shadow(shadow(12, 36, 140))
}

/// The focus ring round `r`: a 4 point cyan outline standing outside it
/// (drawn over any clip, so a row at a list's edge keeps its ring), and a
/// faint cyan fill inside.
pub fn focus_ring(p: &Painter, r: Rect, s: f32, radius: f32) {
    let p = p.with_clip_rect(p.ctx().content_rect());
    p.rect_filled(r, radius * s, FOCUS_FILL);
    p.rect_stroke(r.expand(2.0 * s), (radius + 2.0) * s, Stroke::new(4.0 * s, CYAN), StrokeKind::Outside);
}

/// A menu row's focus: the ring, with a 4 point cyan bar down its left.
pub fn menu_row_focus(p: &Painter, r: Rect, s: f32) {
    focus_ring(p, r, s, ROW_RADIUS);
    let bar = Rect::from_min_size(r.min, vec2(4.0 * s, r.height())).shrink2(vec2(0.0, 6.0 * s));
    p.rect_filled(bar, 2.0 * s, CYAN);
}

/// A full-width menu row: `label` on the left, `hint` (a shortcut, a value)
/// dim on the right. `focused` is whatever the pad or keyboard is on.
pub fn menu_row(ui: &mut Ui, label: &str, hint: &str, focused: bool) -> Response {
    let s = scale(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0 * s), Sense::hover());
    let r = ui.interact(rect, ui.make_persistent_id(("menu-row", label)), Sense::click());
    let p = ui.painter();
    if focused {
        menu_row_focus(p, rect, s);
    } else if r.hovered() {
        p.rect_filled(rect, ROW_RADIUS * s, Color32::from_white_alpha(14));
    }
    let pad = 24.0 * s;
    p.text(rect.left_center() + vec2(pad, 0.0), Align2::LEFT_CENTER, label, FontId::new(21.0 * s, semibold()), TEXT);
    if !hint.is_empty() {
        p.text(
            rect.right_center() - vec2(pad, 0.0),
            Align2::RIGHT_CENTER,
            hint,
            FontId::new(16.0 * s, FontFamily::Proportional),
            TEXT_DIM,
        );
    }
    r
}

/// A segmented control (a pill of choices): the one clicked, if any. The
/// selected segment is filled cyan.
pub fn segmented(ui: &mut Ui, labels: &[&str], selected: usize) -> Option<usize> {
    let s = scale(ui.ctx());
    let font = FontId::new(17.0 * s, semibold());
    let widths: Vec<f32> = labels
        .iter()
        .map(|l| ui.painter().layout_no_wrap(l.to_string(), font.clone(), TEXT).size().x + 36.0 * s)
        .collect();
    let (rect, _) = ui.allocate_exact_size(vec2(widths.iter().sum::<f32>() + 8.0 * s, 44.0 * s), Sense::hover());
    ui.painter().rect_filled(rect, 999.0, RAISED);
    let (mut x, mut clicked) = (rect.left() + 4.0 * s, None);
    let base = ui.make_persistent_id(labels.join("|"));
    for (i, (label, w)) in labels.iter().zip(&widths).enumerate() {
        let seg = Rect::from_min_size(pos2(x, rect.top() + 4.0 * s), vec2(*w, rect.height() - 8.0 * s));
        let r = ui.interact(seg, base.with(i), Sense::click());
        let on = i == selected;
        if on {
            ui.painter().rect_filled(seg, 999.0, CYAN);
        } else if r.hovered() {
            ui.painter().rect_filled(seg, 999.0, Color32::from_white_alpha(18));
        }
        ui.painter().text(seg.center(), Align2::CENTER_CENTER, label, font.clone(), if on { INK } else { TEXT });
        if r.clicked() {
            clicked = Some(i);
        }
        x += w;
    }
    clicked
}

/// A button hint's cap: a face button, the menu button or a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chip<'a> {
    A,
    B,
    X,
    Y,
    /// The pad's menu button (Start), drawn "≡".
    Menu,
    /// Any other cap: "LB RB", "Esc", "D-pad".
    Key(&'a str),
}

impl<'a> Chip<'a> {
    /// The chip for a button's name as the menus spell it.
    pub fn named(name: &'a str) -> Chip<'a> {
        match name {
            "A" => Chip::A,
            "B" => Chip::B,
            "X" => Chip::X,
            "Y" => Chip::Y,
            "Start" | "Menu" | "≡" => Chip::Menu,
            other => Chip::Key(other),
        }
    }

    /// The cap's text and, for a face button, its colour.
    fn face(self) -> (&'a str, Option<Color32>) {
        match self {
            Chip::A => ("A", Some(GREEN)),
            Chip::B => ("B", Some(Color32::from_rgb(255, 77, 94))),
            Chip::X => ("X", Some(Color32::from_rgb(58, 160, 255))),
            Chip::Y => ("Y", Some(YELLOW)),
            Chip::Menu => ("≡", None),
            Chip::Key(k) => (k, None),
        }
    }
}

/// A chip's size: its diameter in design points (its letters follow it).
pub const CHIP: f32 = 26.0;
/// The launcher's: big enough that its labels read at 18 points from a couch.
pub const CHIP_LARGE: f32 = 32.0;

/// How wide `chip` draws.
fn chip_width(p: &Painter, chip: Chip, s: f32) -> f32 {
    chip_width_at(p, chip, s, CHIP)
}

/// How wide `chip` draws, `d` design points across.
pub fn chip_width_at(p: &Painter, chip: Chip, s: f32, d: f32) -> f32 {
    let (d, f) = (d * s, d * 0.58 * s);
    match chip.face() {
        (_, Some(_)) => d,
        (text, None) => {
            let w = p.layout_no_wrap(text.to_owned(), FontId::new(f, bold()), TEXT).size().x + 0.6 * d;
            if chip == Chip::Menu {
                w.max(d)
            } else {
                w
            }
        }
    }
}

/// Paint `chip` with its left edge at `at` (centred on `at.y`).
fn paint_chip(p: &Painter, chip: Chip, at: egui::Pos2, s: f32) {
    paint_chip_at(p, chip, at, s, CHIP);
}

/// Paint `chip` `d` design points across, its left edge at `at` (centred on
/// `at.y`).
pub fn paint_chip_at(p: &Painter, chip: Chip, at: egui::Pos2, s: f32, d: f32) {
    let font = FontId::new(d * 0.58 * s, bold());
    let (text, circle) = chip.face();
    let (w, d) = (chip_width_at(p, chip, s, d), d * s);
    if let Some(fill) = circle {
        let c = pos2(at.x + d / 2.0, at.y);
        p.circle_filled(c, d / 2.0, fill);
        p.text(c, Align2::CENTER_CENTER, text, font, INK);
        return;
    }
    let r = Rect::from_min_size(pos2(at.x, at.y - d / 2.0), vec2(w, d));
    let (fill, ink) = if chip == Chip::Menu {
        (Color32::from_rgb(230, 235, 245), INK)
    } else {
        (Color32::from_white_alpha(36), TEXT)
    };
    p.rect_filled(r, 6.0 * s, fill);
    if chip == Chip::Menu {
        // Three bars: no font has the "≡" to be sure of.
        let (half, gap) = (0.26 * d, 0.2 * d);
        for i in [-1.0, 0.0, 1.0] {
            let y = r.center().y + i * gap;
            p.line_segment([pos2(r.center().x - half, y), pos2(r.center().x + half, y)], Stroke::new(0.085 * d, ink));
        }
        return;
    }
    p.text(r.center(), Align2::CENTER_CENTER, text, font, ink);
}

/// A line of button hints: each chip and what it does ("A  Select").
pub fn chips(ui: &mut Ui, hints: &[(Chip, &str)]) {
    let s = scale(ui.ctx());
    let font = FontId::new(18.0 * s, FontFamily::Proportional);
    let (gap, between) = (8.0 * s, 22.0 * s);
    let p = ui.painter().clone();
    let widths: Vec<(f32, f32)> = hints
        .iter()
        .map(|(chip, what)| {
            (chip_width(&p, *chip, s), p.layout_no_wrap(what.to_string(), font.clone(), TEXT_2).size().x)
        })
        .collect();
    let total: f32 =
        widths.iter().map(|(a, b)| a + gap + b).sum::<f32>() + between * hints.len().saturating_sub(1) as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(total, 28.0 * s), Sense::hover());
    let mut x = rect.left();
    for ((chip, what), (cw, tw)) in hints.iter().zip(&widths) {
        paint_chip(&p, *chip, pos2(x, rect.center().y), s);
        p.text(pos2(x + cw + gap, rect.center().y), Align2::LEFT_CENTER, what, font.clone(), TEXT_2);
        x += cw + gap + tw + between;
    }
}

/// How serious a toast is: its dot's colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warn,
    /// Stays until dismissed.
    Error,
}

impl Level {
    fn colour(self) -> Color32 {
        match self {
            Level::Ok => GREEN,
            Level::Warn => YELLOW,
            Level::Error => RED,
        }
    }
}

struct Toast {
    id: u64,
    text: String,
    level: Level,
    at: Instant,
}

/// Corner notices: a short queue, newest at the bottom, each fading out
/// after `LIFE` (errors stay until dismissed).
#[derive(Default)]
pub struct Toasts {
    list: Vec<Toast>,
    next: u64,
}

impl Toasts {
    /// How long a toast shows, and how much of that it fades over.
    pub const LIFE: Duration = Duration::from_millis(2500);
    const FADE: Duration = Duration::from_millis(400);
    /// The most shown at once.
    const MAX: usize = 3;

    /// Queue a toast (the same text again just restarts the one showing).
    pub fn push(&mut self, level: Level, text: impl Into<String>) {
        self.push_at(level, text.into(), Instant::now());
    }

    fn push_at(&mut self, level: Level, text: String, now: Instant) {
        self.list.retain(|t| t.text != text);
        self.next += 1;
        self.list.push(Toast { id: self.next, text, level, at: now });
        let extra = self.list.len().saturating_sub(Self::MAX);
        self.list.drain(..extra);
    }

    fn live(t: &Toast, now: Instant) -> bool {
        t.level == Level::Error || now.saturating_duration_since(t.at) < Self::LIFE
    }

    /// Something is showing (the screen needs redrawing).
    pub fn active(&self) -> bool {
        let now = Instant::now();
        self.list.iter().any(|t| Self::live(t, now))
    }

    /// Dismiss the errors (they've been seen).
    pub fn dismiss_errors(&mut self) {
        self.list.retain(|t| t.level != Level::Error);
    }

    /// Draw the toasts in the screen's bottom-left corner.
    pub fn show(&mut self, ctx: &Context) {
        let now = Instant::now();
        self.list.retain(|t| Self::live(t, now));
        if self.list.is_empty() {
            return;
        }
        let s = scale(ctx);
        let mut dismissed = None;
        egui::Area::new(Id::unique("toasts"))
            .order(egui::Order::Foreground)
            .anchor(Align2::LEFT_BOTTOM, [24.0 * s, -24.0 * s])
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0 * s;
                for t in &self.list {
                    let left = Self::LIFE.saturating_sub(now.saturating_duration_since(t.at));
                    let fade = if t.level == Level::Error {
                        1.0
                    } else {
                        (left.as_secs_f32() / Self::FADE.as_secs_f32()).min(1.0)
                    };
                    ui.scope(|ui| {
                        ui.set_opacity(fade);
                        let frame = egui::Frame::new()
                            .fill(Color32::from_rgba_unmultiplied(13, 18, 32, 235))
                            .stroke(Stroke::new(1.0, Color32::from_rgba_unmultiplied(34, 229, 255, 102)))
                            .corner_radius(ROW_RADIUS * s)
                            .inner_margin(egui::Margin::symmetric((18.0 * s) as i8, (12.0 * s) as i8))
                            .shadow(shadow(10, 30, 128));
                        let out = frame.show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 12.0 * s;
                                let (dot, _) = ui.allocate_exact_size(vec2(10.0 * s, 10.0 * s), Sense::hover());
                                ui.painter().circle_filled(dot.center(), 5.0 * s, t.level.colour());
                                ui.label(RichText::new(&t.text).font(FontId::new(17.0 * s, FontFamily::Proportional)));
                                if t.level == Level::Error {
                                    ui.label(RichText::new("×").size(20.0 * s).color(TEXT_DIM));
                                }
                            });
                        });
                        if ui.interact(out.response.rect, ui.scope_id().with(("toast", t.id)), Sense::click()).clicked()
                        {
                            dismissed = Some(t.id);
                        }
                    });
                }
            });
        if let Some(id) = dismissed {
            self.list.retain(|t| t.id != id);
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_follows_height_with_a_floor() {
        assert_eq!(scale_for(720.0), 1.0);
        assert_eq!(scale_for(2160.0), 3.0);
        assert_eq!(scale_for(300.0), MIN_SCALE);
    }

    #[test]
    fn chips_are_named_by_their_button() {
        assert_eq!(Chip::named("A"), Chip::A);
        assert_eq!(Chip::named("Start"), Chip::Menu);
        assert_eq!(Chip::named("D-pad"), Chip::Key("D-pad"));
    }

    #[test]
    fn toasts_expire_but_errors_stay() {
        let mut t = Toasts::default();
        let past = Instant::now() - Toasts::LIFE * 2;
        t.push_at(Level::Ok, "saved".into(), past);
        t.push_at(Level::Error, "broke".into(), past);
        assert!(t.active(), "the error is still up");
        t.dismiss_errors();
        assert!(!t.active());
        t.push(Level::Warn, "hm");
        assert!(t.active());
    }

    #[test]
    fn toasts_dedupe_and_cap() {
        let mut t = Toasts::default();
        for i in 0..5 {
            t.push(Level::Ok, format!("n{i}"));
        }
        t.push(Level::Ok, "n4");
        assert_eq!(t.list.len(), Toasts::MAX);
        assert_eq!(t.list.last().unwrap().text, "n4");
    }
}
