//! Gamepad focus over a page of egui widgets (the menu's): the d-pad moves
//! it between them as they're laid out on screen, A activates the focused
//! one, left and right change a slider or step through a radio group, and
//! the focused one glows. Widgets register as they're drawn
//! (`Focus::button` and so on); the moves apply to where they were the
//! frame before.

use crate::launcher::{Nav, CYAN};
use egui::{Color32, Id, Rect, Response, Stroke, StrokeKind};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Button,
    Slider,
    Radio { group: &'static str, selected: bool },
}

#[derive(Clone, Copy, Debug)]
struct Item {
    id: Id,
    rect: Rect,
    kind: Kind,
    enabled: bool,
}

#[derive(Default)]
pub struct Focus {
    /// The focused widget, and where it came in the page.
    on: Option<(Id, usize)>,
    /// The page's widgets last frame, and so far this frame.
    last: Vec<Item>,
    items: Vec<Item>,
    /// Widgets activated this frame, and sliders stepped.
    click: Vec<Id>,
    step: Vec<(Id, i32)>,
    /// The focus moved this frame (scroll it into view).
    moved: bool,
    /// Draw the glow (the pad is in use).
    pub show: bool,
}

/// Two rects on the same line: their heights mostly overlap.
fn same_row(a: Rect, b: Rect) -> bool {
    a.y_range().intersection(b.y_range()).span() >= 0.5 * a.height().min(b.height())
}

/// The horizontal gap between two rects, 0 if they overlap.
fn x_gap(a: Rect, b: Rect) -> f32 {
    (a.left() - b.right()).max(b.left() - a.right()).max(0.0)
}

impl Focus {
    /// The focused widget.
    pub fn focused(&self) -> Option<Id> {
        self.on.map(|o| o.0)
    }

    /// Start a frame: apply `navs` (directions and Accept) to the page as
    /// it was last drawn; the others are returned for the page.
    pub fn begin(&mut self, navs: &[Nav]) -> Vec<Nav> {
        self.items.clear();
        self.click.clear();
        self.step.clear();
        self.moved = false;
        let mut rest = Vec::new();
        for &nav in navs {
            let at = self.on.and_then(|(id, _)| self.last.iter().position(|i| i.id == id));
            let is_move = matches!(nav, Nav::Up | Nav::Down | Nav::Left | Nav::Right | Nav::Accept);
            let Some(at) = at.filter(|_| is_move) else {
                if !is_move {
                    rest.push(nav);
                }
                continue;
            };
            let item = self.last[at];
            match nav {
                Nav::Accept if item.enabled => self.click.push(item.id),
                Nav::Left | Nav::Right => {
                    let d = if nav == Nav::Left { -1 } else { 1 };
                    match item.kind {
                        Kind::Slider if item.enabled => self.step.push((item.id, d)),
                        Kind::Radio { group, .. } => {
                            let radios: Vec<usize> = (0..self.last.len())
                                .filter(|&i| {
                                    matches!(self.last[i].kind, Kind::Radio { group: g, .. } if g == group)
                                        && self.last[i].enabled
                                })
                                .collect();
                            let pos = radios.iter().position(|&i| i == at).unwrap_or(0) as i32;
                            if let Some(&to) = usize::try_from(pos + d).ok().and_then(|i| radios.get(i)) {
                                self.go(to);
                                self.click.push(self.last[to].id);
                            }
                        }
                        _ => {
                            if let Some(to) = self.across(at, d as f32) {
                                self.go(to);
                            }
                        }
                    }
                }
                Nav::Up | Nav::Down => {
                    if let Some(to) = self.vertical(at, if nav == Nav::Up { -1.0 } else { 1.0 }) {
                        self.go(to);
                    }
                }
                _ => {}
            }
        }
        rest
    }

    fn go(&mut self, to: usize) {
        self.on = Some((self.last[to].id, to));
        self.moved = true;
    }

    /// The nearest widget beside `at` on its line, towards `d` (-1 left).
    fn across(&self, at: usize, d: f32) -> Option<usize> {
        let c = self.last[at].rect;
        (0..self.last.len())
            .filter(|&i| i != at && self.last[i].enabled)
            .filter(|&i| same_row(self.last[i].rect, c) && (self.last[i].rect.center().x - c.center().x) * d > 0.0)
            .min_by(|&a, &b| {
                let dx = |i: usize| (self.last[i].rect.center().x - c.center().x).abs();
                dx(a).total_cmp(&dx(b))
            })
    }

    /// The widget on the nearest line above (`d` -1) or below `at`: the one
    /// most in line with it (a radio group's chosen one).
    fn vertical(&self, at: usize, d: f32) -> Option<usize> {
        let c = self.last[at].rect;
        let cands: Vec<usize> = (0..self.last.len())
            .filter(|&i| i != at && self.last[i].enabled)
            .filter(|&i| {
                let r = self.last[i].rect;
                !same_row(r, c) && (r.center().y - c.center().y) * d > 0.0
            })
            .collect();
        let dy = |i: usize| (self.last[i].rect.center().y - c.center().y).abs();
        let nearest = *cands.iter().min_by(|&&a, &&b| dy(a).total_cmp(&dy(b)))?;
        let row: Vec<usize> =
            cands.into_iter().filter(|&i| same_row(self.last[i].rect, self.last[nearest].rect)).collect();
        let key = |i: usize| {
            let r = self.last[i].rect;
            (x_gap(r, c), (r.left() - c.left()).abs())
        };
        let best = *row.iter().min_by(|&&a, &&b| key(a).partial_cmp(&key(b)).unwrap())?;
        if let Kind::Radio { group, .. } = self.last[best].kind {
            let chosen = row.iter().copied().find(|&i| self.last[i].kind == Kind::Radio { group, selected: true });
            return chosen.or(Some(best));
        }
        Some(best)
    }

    fn item(&mut self, ui: &egui::Ui, r: &Response, kind: Kind) -> bool {
        // A window's first frame sizes it, its widgets disabled.
        let enabled = r.enabled() || ui.is_sizing_pass();
        self.items.push(Item { id: r.id, rect: r.rect, kind, enabled });
        if self.focused() == Some(r.id) {
            if self.moved {
                r.scroll_to_me(None);
            }
            if self.show {
                glow(ui.painter(), r.rect);
            }
        }
        self.click.contains(&r.id)
    }

    /// A button: clicked, or activated from the pad.
    pub fn button(&mut self, ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> Response) -> bool {
        let r = add(ui);
        self.item(ui, &r, Kind::Button) || r.clicked()
    }

    /// A checkbox for `value`: changed?
    pub fn checkbox(&mut self, ui: &mut egui::Ui, value: &mut bool, text: impl Into<egui::WidgetText>) -> bool {
        let mut r = ui.checkbox(value, text);
        if self.item(ui, &r, Kind::Button) {
            *value = !*value;
            r.mark_changed();
        }
        r.changed()
    }

    /// One of radio group `group`'s buttons, setting `value` to `alt`.
    pub fn radio<T: PartialEq + Copy>(
        &mut self,
        ui: &mut egui::Ui,
        group: &'static str,
        value: &mut T,
        alt: T,
        text: impl Into<egui::WidgetText>,
    ) {
        let selected = *value == alt;
        let r = ui.radio_value(value, alt, text);
        if self.item(ui, &r, Kind::Radio { group, selected }) {
            *value = alt;
        }
    }

    /// A slider: how many steps the pad moved it this frame.
    pub fn slider(&mut self, ui: &egui::Ui, r: &Response) -> i32 {
        self.item(ui, r, Kind::Slider);
        self.step.iter().filter(|s| s.0 == r.id).map(|s| s.1).sum()
    }

    /// End the frame: focus the first widget if none is (the page just
    /// opened), or the one in the same place if the focused one has gone.
    pub fn end(&mut self) {
        self.last = std::mem::take(&mut self.items);
        let gone = self.on.is_none_or(|(id, _)| !self.last.iter().any(|i| i.id == id));
        if gone && !self.last.is_empty() {
            let from = self.on.map_or(0, |o| o.1.min(self.last.len() - 1));
            let to = (from..self.last.len()).chain((0..from).rev()).find(|&i| self.last[i].enabled).unwrap_or(from);
            self.on = Some((self.last[to].id, to));
        } else if let Some((id, _)) = self.on {
            self.on = self.last.iter().position(|i| i.id == id).map(|i| (id, i));
        }
    }
}

/// The focus highlight: the launcher's cyan glow round `r`.
pub fn glow(p: &egui::Painter, r: Rect) {
    let r = r.expand(3.0);
    p.add(
        egui::epaint::RectShape::filled(r, 6.0, Color32::from_rgba_unmultiplied(47, 230, 239, 36))
            .with_blur_width(12.0),
    );
    p.rect_stroke(r, 6.0, Stroke::new(1.5, CYAN), StrokeKind::Outside);
}
