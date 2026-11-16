//! Gamepad focus over a page of egui widgets (the menu's): the d-pad moves
//! it between them as they're laid out on screen, A activates the focused
//! one, left and right change a stepper row ("label ◂ value ▸"), and
//! the focused one gets the theme's focus ring. Widgets register as they're drawn
//! (`Focus::button` and so on); the moves apply to where they were the
//! frame before.

use crate::launcher::Nav;
use egui::{Id, Rect, Response};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Button,
    /// A "label ◂ value ▸" row: left and right change it, A steps on.
    Stepper,
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
    /// Left (-1) and right (+1) presses this frame that moved nothing: no
    /// stepper took them and no widget lay beside the focused one (a page
    /// can use them for itself: `take_loose`).
    loose: i32,
    /// Draw the ring (the pad or keyboard is in use).
    pub show: bool,
}

/// Whether this frame's input hands the page back to the mouse: the
/// pointer moved more than a nudge, a mouse button, or a key pressed (a
/// page's own navigation keys are counted before this, and releasing a key
/// is not new input: counting it turned the keyboard's focus ring off as
/// soon as an arrow key came up).
pub fn pointer_or_other_key(ctx: &egui::Context) -> bool {
    ctx.input(|i| {
        i.pointer.delta().length() > 2.0
            || i.events.iter().any(|e| {
                matches!(e, egui::Event::Key { pressed: true, .. } | egui::Event::PointerButton { pressed: true, .. })
            })
    })
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
        self.loose = 0;
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
                        Kind::Stepper if item.enabled => self.step.push((item.id, d)),
                        _ => match self.across(at, d as f32) {
                            Some(to) => self.go(to),
                            None => self.loose += d,
                        },
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

    /// The net left and right presses this frame that nothing took (see
    /// `loose`), once.
    pub fn take_loose(&mut self) -> i32 {
        std::mem::take(&mut self.loose)
    }

    /// Move the `n` widgets registered last by `dy` for the d-pad's moves
    /// up and down only (not for drawing or scrolling): a page whose list
    /// scrolls under a pinned footer puts the footer after the whole list
    /// (a big `dy`), where the clipped list entries' real places would
    /// put them beyond it.
    pub fn shift_last(&mut self, n: usize, dy: f32) {
        for item in self.items.iter_mut().rev().take(n) {
            item.rect = item.rect.translate(egui::vec2(0.0, dy));
        }
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
    /// most in line with it.
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
        Some(best)
    }

    fn item(&mut self, ui: &egui::Ui, r: &Response, kind: Kind) -> bool {
        self.item_with(ui, r, kind, true)
    }

    /// Move the focus to `id`, a widget of the page as last drawn.
    pub fn focus_to(&mut self, id: Id) {
        if let Some(i) = self.last.iter().position(|x| x.id == id) {
            self.on = Some((id, i));
            self.moved = true;
        }
    }

    /// Whether the pad or keyboard's focus is on `id` (and shows).
    pub fn is_on(&self, id: Id) -> bool {
        self.show && self.focused() == Some(id)
    }

    /// A widget that paints its own focus (ask `is_on` with its id first):
    /// clicked, or activated from the pad.
    pub fn register(&mut self, ui: &egui::Ui, r: &Response) -> bool {
        self.item_with(ui, r, Kind::Button, false) || r.clicked()
    }

    /// A menu row (`theme::menu_row`): clicked, or activated from the pad.
    pub fn menu_row(&mut self, ui: &mut egui::Ui, label: &str, hint: &str) -> bool {
        let id = ui.make_persistent_id(("menu-row", label));
        let r = crate::theme::menu_row(ui, label, hint, self.is_on(id));
        self.register(ui, &r)
    }

    /// `item`, with `ring` off for a widget that paints its own focus.
    fn item_with(&mut self, ui: &egui::Ui, r: &Response, kind: Kind, ring: bool) -> bool {
        // A window's first frame sizes it, its widgets disabled.
        let enabled = r.enabled() || ui.is_sizing_pass();
        self.items.push(Item { id: r.id, rect: r.rect, kind, enabled });
        if self.focused() == Some(r.id) {
            if self.moved {
                r.scroll_to_me(None);
            }
            if self.show && ring {
                glow(ui.painter(), r.rect);
            }
        }
        self.click.contains(&r.id)
    }
    /// A stepper row: how many steps the pad moved it this frame (left
    /// and right), and whether A was pressed on it.
    pub fn stepper(&mut self, ui: &egui::Ui, r: &Response) -> (i32, bool) {
        let accepted = self.item(ui, r, Kind::Stepper);
        (self.step.iter().filter(|s| s.0 == r.id).map(|s| s.1).sum(), accepted)
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

/// The focus highlight: the theme's cyan ring round `r`.
pub fn glow(p: &egui::Painter, r: Rect) {
    crate::theme::focus_ring(p, r, crate::theme::scale(p.ctx()), crate::theme::ROW_RADIUS);
}
