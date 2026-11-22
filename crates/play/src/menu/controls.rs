//! The menu's Controls page (docs/play.md, "Controls").
//!
//! The side panel lists the game's actions, each with the keys and pad
//! buttons it's bound to (the chips of the device being shown: the pad, the
//! keyboard, or both beside a wheel or gun); beside it, over the paused
//! game, a picture of the device with this game's actions labelled on its
//! parts, and for games with a stick or pedals a tuning card: dead zone,
//! sensitivity and curve, with a live meter of what the stick reads and what
//! the game gets.
//!
//! Rebinding: A (or a click) on a row waits for a key, a pad button or a
//! stick and puts it in place of the row's first binding on this device (a
//! click on a binding replaces that one); X (or +) adds another; Y (or ×)
//! clears the device's bindings from the row, and on an empty row brings the
//! game's own back. A key or button bound to two actions is marked (yellow,
//! "also ...") and the capture says so. The bindings are kept for the game
//! (`settings::save_bindings`); the tuning follows the This game only / All
//! games switch like the Picture page's settings.

use super::figure::{self, Art, GunPart, Part, Placed, Shape, Spot, Tag, WheelPart};
use super::picture::scope_toggle;
use super::{pill, regions, side_panel, Menu, Page, PanelSpec, Prefs, Regions, View};
use crate::controls::{is_stick, Curve, PadLive, PadStyle, Response};
use crate::focus::Focus;
use crate::launcher::Nav;
use crate::settings::GameSettings;
use crate::steer::Smoothing;
use crate::theme::{self, Chip};
use egui::{pos2, vec2, Align2, Color32, FontFamily, FontId, Id, Painter, Pos2, Rect, Sense, Shape as Mesh, Stroke};
use romset::bindings::{Accepts, Bindings, Input, Slot, Value};
use romset::{Game, Kind, PadAxis, PadButton};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The footer's height: the reset button and a line about where the
/// bindings are kept (design points).
const FOOT_H: f32 = 100.0;
/// How far the pad's moves put the pinned footer past the scrolling list.
const SHIFT: f32 = 100_000.0;
/// Reset asks twice within this long.
const RESET_CONFIRM: Duration = Duration::from_secs(4);
/// The picture card's height and the tuning card's top, in design points.
const ART_H: f32 = 360.0;
const MARGIN: f32 = 28.0;
/// Text on this page is never smaller than this (design points).
const TEXT_SIZE: f32 = 16.0;

/// The card behind a picture or the tuning.
const CARD: Color32 = Color32::from_rgba_premultiplied(7, 10, 19, 214);

/// What the page shows pictures and chips of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    /// The wheel and pedals, or the light gun: the game's own controls,
    /// whatever they're bound to.
    Cabinet,
    Pad,
    Keys,
}

impl Device {
    fn label(self, art: Option<Art>) -> &'static str {
        match (self, art) {
            (Device::Cabinet, Some(Art::Gun)) => "Gun",
            (Device::Cabinet, _) => "Wheel",
            (Device::Pad, _) => "Pad",
            (Device::Keys, _) => "Keyboard",
        }
    }
}

/// The art for the game's own controls: a wheel for driving games, a gun
/// for light-gun games, none for the rest.
pub fn cabinet_art(game: Option<&Game>) -> Option<Art> {
    let c = &game?.controls;
    if !c.guns.is_empty() {
        Some(Art::Gun)
    } else if game?.kind == Kind::Racing && !c.analogs.is_empty() {
        Some(Art::Wheel)
    } else {
        None
    }
}

/// What the page keeps between frames.
#[derive(Default)]
pub(super) struct State {
    pub(super) focus: Focus,
    /// The device shown (None: the default for the game and the pad).
    pub(super) device: Option<Device>,
    /// The pad picture chosen by hand (else by the connected pad).
    pub(super) layout: Option<PadStyle>,
    /// The tuning block shown, when a game has a stick's and pedals'.
    pub(super) block: usize,
    /// Reset was selected once, and wants a second.
    pub(super) armed: Option<Instant>,
    textures: HashMap<Art, egui::TextureHandle>,
}

/// One binding on a row.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Cap {
    /// Its place in the slot's list (0 for an axis).
    pub index: usize,
    pub text: String,
    /// Another action has it too.
    pub clash: bool,
}

/// An action on the page.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Row {
    pub slot: Slot,
    pub name: String,
    pub caps: Vec<Cap>,
    /// "also Jump tail": what shares this row's inputs.
    pub also: Option<String>,
    /// Differs from the game's own.
    pub edited: bool,
}

/// What a row asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowAct {
    /// Wait for a binding: in place of the nth, or (None) added.
    Bind(Option<usize>),
    Clear,
    Restore,
    Invert,
}

/// How a pad button reads on a pad of this style.
pub(super) fn pad_text(b: PadButton, style: PadStyle) -> &'static str {
    if style == PadStyle::Offset {
        return b.label();
    }
    match b {
        PadButton::South => "Cross",
        PadButton::East => "Circle",
        PadButton::West => "Square",
        PadButton::North => "Triangle",
        PadButton::LeftShoulder => "L1",
        PadButton::RightShoulder => "R1",
        PadButton::LeftTrigger => "L2",
        PadButton::RightTrigger => "R2",
        PadButton::Select => "Share",
        PadButton::Start => "Options",
        other => other.label(),
    }
}

fn input_text(i: &Input, style: PadStyle) -> String {
    match i {
        Input::Pad(b) => pad_text(*b, style).to_string(),
        key => key.text(),
    }
}

/// An axis as the page says it ("RT", "left stick ↔", "inverted").
fn axis_text(a: romset::bindings::Axis) -> String {
    if a.axis == PadAxis::None {
        String::new()
    } else {
        a.text()
    }
}

/// Whether a binding belongs on `device`'s chips.
fn on_device(i: &Input, device: Device) -> bool {
    matches!((device, i), (Device::Cabinet, _) | (Device::Pad, Input::Pad(_)) | (Device::Keys, Input::Key(_)))
}

/// The action's name on a picture: an analog's own name for its axis (the
/// row says more), the label otherwise.
fn short_label(b: &Bindings, slot: Slot, cabinet: bool) -> String {
    match slot {
        Slot::Axis(i) => b.controls().analogs[i].label(),
        // On the wheel itself the pedal and the wheel are one part each.
        Slot::Neg(i) | Slot::Pos(i) if cabinet => b.controls().analogs[i].label(),
        other => b.label(other),
    }
}

/// "Z is also Jump tail": the toast for binding something already bound to
/// another action; None if `slot` shares nothing.
pub fn conflict_text(b: &Bindings, slot: Slot) -> Option<String> {
    let found: Vec<String> = b
        .conflicts_of(slot)
        .iter()
        .map(|c| {
            let with: Vec<String> = c.slots.iter().filter(|s| **s != slot).map(|&s| b.label(s)).collect();
            format!("{} is also {}", c.input.text(), with.join(", "))
        })
        .collect();
    (!found.is_empty()).then(|| found.join("; "))
}

/// The rows for `device`: every action it can have bindings for, except
/// (unless `show_unbound`) those the manifest leaves unbound and nobody has
/// bound; and how many of those there are.
pub(super) fn rows(b: &Bindings, device: Device, style: PadStyle, show_unbound: bool) -> (Vec<Row>, usize) {
    let mut out = Vec::new();
    let mut hidden = 0;
    for (slot, value) in b.slots() {
        let accepts = b.accepts(*slot);
        if device == Device::Keys && accepts != Accepts::Inputs {
            continue;
        }
        let unbound = *value == Value::Inputs(Vec::new()) && b.default_of(*slot) == Some(value);
        if unbound && !show_unbound {
            hidden += 1;
            continue;
        }
        let caps = match value {
            Value::Axis(a) if a.axis != PadAxis::None => {
                vec![Cap { index: 0, text: axis_text(*a), clash: false }]
            }
            Value::Axis(_) => Vec::new(),
            Value::Inputs(list) => {
                let conflicts = b.conflicts_of(*slot);
                list.iter()
                    .enumerate()
                    .filter(|(_, i)| on_device(i, device))
                    .map(|(index, i)| Cap {
                        index,
                        text: input_text(i, style),
                        clash: conflicts.iter().any(|c| c.input == *i),
                    })
                    .collect()
            }
        };
        let mut with: Vec<String> = Vec::new();
        for c in b.conflicts_of(*slot) {
            for s in c.slots.iter().filter(|s| **s != *slot) {
                let l = b.label(*s);
                if !with.contains(&l) {
                    with.push(l);
                }
            }
        }
        out.push(Row {
            slot: *slot,
            name: b.label(*slot),
            caps,
            also: (!with.is_empty()).then(|| format!("also {}", with.join(", "))),
            edited: !b.is_default(*slot),
        });
    }
    (out, hidden)
}

/// Take `device`'s bindings off `slot` (all of an axis'): whether any went.
pub(super) fn clear_device(b: &mut Bindings, slot: Slot, device: Device) -> bool {
    match b.get(slot).cloned() {
        Some(Value::Inputs(list)) => {
            let mut any = false;
            for (i, input) in list.iter().enumerate().rev() {
                if on_device(input, device) {
                    b.remove(slot, i);
                    any = true;
                }
            }
            any
        }
        Some(Value::Axis(a)) if a.axis != PadAxis::None => {
            b.remove(slot, 0);
            true
        }
        _ => false,
    }
}

/// Which parts of the pad a pad axis is.
fn axis_buttons(a: PadAxis) -> &'static [PadButton] {
    match a {
        PadAxis::None => &[],
        PadAxis::LeftStickX | PadAxis::LeftStickY => &[PadButton::LeftStick],
        PadAxis::RightStickX | PadAxis::RightStickY => &[PadButton::RightStick],
        PadAxis::LeftTrigger => &[PadButton::LeftTrigger],
        PadAxis::RightTrigger => &[PadButton::RightTrigger],
        PadAxis::Triggers => &[PadButton::LeftTrigger, PadButton::RightTrigger],
    }
}

/// An action's part on the wheel or gun picture, by what the manifest calls
/// it (the names are the cabinet's: `wheel`, `gas`, `brake`, `shiftup`,
/// `trigger1`); None for those the picture has no part for.
fn cabinet_part(art: Art, b: &Bindings, slot: Slot) -> Option<Part> {
    let c = b.controls();
    let analog = |i: usize| c.analogs[i].name;
    match (art, slot) {
        (Art::Wheel, Slot::Axis(i) | Slot::Neg(i) | Slot::Pos(i)) => {
            let n = analog(i);
            if ["wheel", "steer", "handle", "curv", "lean", "turn"].iter().any(|k| n.contains(k)) {
                Some(Part::Wheel(WheelPart::Rim))
            } else if ["gas", "accel", "throttle"].iter().any(|k| n.contains(k)) {
                Some(Part::Wheel(WheelPart::Gas))
            } else if n.contains("brake") {
                Some(Part::Wheel(WheelPart::Brake))
            } else {
                None
            }
        }
        (Art::Wheel, Slot::Button(i)) => match c.buttons[i].name {
            "shiftup" => Some(Part::Wheel(WheelPart::PaddleRight)),
            "shiftdown" => Some(Part::Wheel(WheelPart::PaddleLeft)),
            "view" | "view1" => Some(Part::Wheel(WheelPart::Button(2))),
            "start" => Some(Part::Wheel(WheelPart::Button(3))),
            _ => None,
        },
        (Art::Gun, Slot::Button(i)) => {
            let n = c.buttons[i].name;
            if c.guns.iter().any(|g| g.trigger == n) {
                Some(Part::Gun(GunPart::Trigger))
            } else if n == "start" {
                Some(Part::Gun(GunPart::Thumb))
            } else {
                None
            }
        }
        (Art::Gun, Slot::Reload(_)) => Some(Part::Gun(GunPart::Side)),
        _ => None,
    }
}

/// An action on a picture's part (`slot` None: a label of the page's own).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct OnPart {
    pub part: Part,
    pub slot: Option<Slot>,
    pub label: String,
}

/// What's on each part of the picture for `device`, in the order of the
/// manifest, and the actions the picture has no part for.
pub(super) fn part_actions(b: &Bindings, device: Device, art: Art, rows: &[Row]) -> (Vec<OnPart>, Vec<String>) {
    let (mut out, mut loose) = (Vec::new(), Vec::new());
    let gun_game = !b.controls().guns.is_empty();
    for r in rows {
        let value = b.get(r.slot);
        let mut parts: Vec<Part> = Vec::new();
        match device {
            Device::Pad => {
                match value {
                    Some(Value::Inputs(list)) => {
                        // A stick direction is the stick's part, unless it
                        // only stands in for the d-pad's.
                        parts.extend(list.iter().filter_map(|i| match i {
                            Input::Pad(p) if p.is_stick_dir() => {
                                let twin = PadButton::ALL.into_iter().find(|t| t.left_stick_of() == Some(*p));
                                let doubled = twin.is_some_and(|t| list.contains(&Input::Pad(t)));
                                (!doubled).then_some(Part::Pad(p.part()))
                            }
                            Input::Pad(p) => Some(Part::Pad(*p)),
                            _ => None,
                        }));
                        parts.dedup();
                    }
                    Some(Value::Axis(a)) => parts.extend(axis_buttons(a.axis).iter().map(|p| Part::Pad(*p))),
                    None => {}
                }
                if r.caps.is_empty() {
                    continue;
                }
            }
            Device::Cabinet => parts.extend(cabinet_part(art, b, r.slot)),
            Device::Keys => {}
        }
        let label = short_label(b, r.slot, device == Device::Cabinet);
        if parts.is_empty() && device == Device::Cabinet {
            loose.push(label);
            continue;
        }
        for part in parts {
            out.push(OnPart { part, slot: Some(r.slot), label: label.clone() });
        }
    }
    // A pad aims a gun with either stick; a gun points at the screen.
    if gun_game && device == Device::Pad {
        for p in [PadButton::LeftStick, PadButton::RightStick] {
            out.push(OnPart { part: Part::Pad(p), slot: None, label: "Aim".into() });
        }
    }
    if gun_game && device == Device::Cabinet {
        out.push(OnPart { part: Part::Gun(GunPart::Muzzle), slot: None, label: "Aim".into() });
    }
    (out, loose)
}

/// The tuning's blocks: sticks and triggers the game reads, with what it
/// calls them. A game whose analogs aren't bound to any axis has none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tuned {
    /// Sticks that aim or fly.
    Stick,
    /// Sticks that steer (`AnalogInput::steering`).
    Steer,
    /// Triggers.
    Pedal,
}

impl Tuned {
    fn bidirectional(self) -> bool {
        self != Tuned::Pedal
    }

    fn help(self) -> &'static str {
        match self {
            Tuned::Stick => "Nothing is read inside the dead zone; past it the reading rises smoothly.",
            Tuned::Steer => {
                "A small dead zone and a finer curve keep steering steady near the middle. \
                 Smoothing calms a shaky stick; stronger adds a little lag."
            }
            Tuned::Pedal => "Over 100% reaches full before the trigger is all the way down.",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Tuned::Stick => "Stick",
            Tuned::Steer => "Steering",
            Tuned::Pedal => "Pedals",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Block {
    pub kind: Tuned,
    pub title: String,
    /// The first analog's axis: what the live meter reads.
    pub axis: romset::bindings::Axis,
}

pub(super) fn blocks(b: &Bindings) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    for kind in [Tuned::Steer, Tuned::Stick, Tuned::Pedal] {
        let mine: Vec<(usize, romset::bindings::Axis)> = (0..b.controls().analogs.len())
            .map(|i| (i, b.axis(i)))
            .filter(|(i, a)| {
                let steers = b.controls().analogs[*i].steering;
                a.axis != PadAxis::None
                    && match kind {
                        Tuned::Steer => is_stick(a.axis) && steers,
                        Tuned::Stick => is_stick(a.axis) && !steers,
                        Tuned::Pedal => !is_stick(a.axis),
                    }
            })
            .collect();
        let Some(first) = mine.first() else { continue };
        let names: Vec<String> = mine.iter().map(|(i, _)| b.controls().analogs[*i].label()).collect();
        let mut axes: Vec<&str> = Vec::new();
        for (_, a) in &mine {
            if !axes.contains(&a.axis.label()) {
                axes.push(a.axis.label());
            }
        }
        out.push(Block { kind, title: format!("{}: {}", names.join(", "), axes.join(", ")), axis: first.1 });
    }
    out
}

fn response(s: &GameSettings, kind: Tuned) -> Response {
    match kind {
        Tuned::Stick => s.tuning.stick,
        Tuned::Steer => s.tuning.steer,
        Tuned::Pedal => s.tuning.pedal,
    }
}

fn response_mut(s: &mut GameSettings, kind: Tuned) -> &mut Response {
    match kind {
        Tuned::Stick => &mut s.tuning.stick,
        Tuned::Steer => &mut s.tuning.steer,
        Tuned::Pedal => &mut s.tuning.pedal,
    }
}

/// A row's id (the same every frame, for the pad's focus).
fn row_id(b: &Bindings, slot: Slot) -> Id {
    Id::unique(("remodel-controls-row", b.slots().iter().position(|s| s.0 == slot)))
}

/// `text` cut short with "…" to fit `w`.
fn fit(p: &Painter, text: &str, font: &FontId, w: f32) -> String {
    let wide = |t: &str| p.layout_no_wrap(t.to_string(), font.clone(), theme::TEXT).size().x;
    if wide(text) <= w {
        return text.to_string();
    }
    let mut t: String = text.to_string();
    while !t.is_empty() && wide(&format!("{t}…")) > w {
        t.pop();
    }
    format!("{}…", t.trim_end())
}

fn semibold(s: f32, size: f32) -> FontId {
    FontId::new(size * s, theme::semibold())
}

fn plain(s: f32, size: f32) -> FontId {
    FontId::new(size * s, FontFamily::Proportional)
}

impl Menu {
    pub fn on_controls(&self) -> bool {
        self.page == Page::Controls
    }

    /// The device the page shows: the one chosen, else the game's own
    /// controls (a wheel or gun) if it has them, the pad if one is
    /// connected, else the keyboard.
    pub(super) fn device(&self, cabinet: Option<Art>, pad: &PadLive) -> Device {
        let all = devices(cabinet);
        match self.ctl.device.filter(|d| all.contains(d)) {
            Some(d) => d,
            None if cabinet.is_some() => Device::Cabinet,
            None if pad.connected => Device::Pad,
            None => Device::Keys,
        }
    }

    /// Controls: the actions, the picture and the tuning. Returns whether
    /// the bindings changed (to save them).
    pub(super) fn controls_page(
        &mut self,
        ctx: &egui::Context,
        settings: &mut GameSettings,
        prefs: Prefs,
        view: &View,
        b: &mut Bindings,
        navs: &[Nav],
    ) -> bool {
        let s = theme::scale(ctx);
        let mut changed = false;
        // Waiting too long for a binding gives up (a way out for a pad
        // without a free Guide button).
        match (self.capture, self.capture_since) {
            (None, _) => self.capture_since = None,
            (Some(c), Some((was, at))) if c == was => {
                if at.elapsed() >= super::CAPTURE_WAIT {
                    self.capture = None;
                    self.capture_since = None;
                    self.notify("Nothing bound");
                }
            }
            (Some(c), _) => self.capture_since = Some((c, Instant::now())),
        }
        if self.ctl.armed.is_some_and(|t| t.elapsed() > RESET_CONFIRM) {
            self.ctl.armed = None;
        }
        let capturing = self.capture;
        let navs = if capturing.is_some() { &[][..] } else { navs };
        self.ctl.focus.show = self.pad_used || self.keys_used;
        let rest = self.ctl.focus.begin(navs);
        let mut back = rest.contains(&Nav::Back);

        let cabinet = cabinet_art(view.game);
        let all = devices(cabinet);
        let mut device = self.device(cabinet, &view.pad);
        let style = self.ctl.layout.unwrap_or(view.pad.style);
        let (list, hidden) = rows(b, device, style, self.show_unbound);
        let focused = self.ctl.focus.focused().and_then(|f| list.iter().find(|r| row_id(b, r.slot) == f)).cloned();

        // What the pad's other buttons asked for.
        for nav in rest {
            match nav {
                Nav::Saves | Nav::Continue => {
                    let Some(r) = &focused else { continue };
                    let act = if nav == Nav::Continue {
                        if r.caps.is_empty() && r.edited {
                            RowAct::Restore
                        } else {
                            RowAct::Clear
                        }
                    } else if b.accepts(r.slot) == Accepts::Axis {
                        RowAct::Invert
                    } else {
                        RowAct::Bind(None)
                    };
                    changed |= self.row_act(b, r.slot, act, device);
                }
                Nav::NextGroup | Nav::PrevGroup => {
                    let at = all.iter().position(|d| *d == device).unwrap_or(0) as i32;
                    let d = if nav == Nav::NextGroup { 1 } else { -1 };
                    device = all[(at + d).rem_euclid(all.len() as i32) as usize];
                    self.ctl.device = Some(device);
                }
                _ => {}
            }
        }
        // (The rows are for the device as the frame began.)

        let name = view.playing.unwrap_or_else(|| view.game.map_or("This game", |g| g.title));
        let reg = regions(ctx.content_rect(), s, FOOT_H);
        let conflicts = !b.conflicts().is_empty();
        let mut hints =
            vec![(Chip::A, "Rebind".to_string()), (Chip::X, "Add".to_string()), (Chip::Y, "Clear".to_string())];
        if let Some(r) = &focused {
            if b.accepts(r.slot) == Accepts::Axis {
                hints[1].1 = "Invert".into();
            }
            if r.caps.is_empty() && r.edited {
                hints[2].1 = "Restore".into();
            }
        }
        if capturing.is_some() {
            hints = vec![(Chip::Key("Any key or button"), "binds".into()), (Chip::Key("Esc"), "cancels".into())];
        }
        let hint_refs: Vec<(Chip, &str)> = hints.iter().map(|(c, t)| (*c, t.as_str())).collect();

        let spec = PanelSpec { game: name, heading: "Controls", back: true, extra_h: FOOT_H };
        let mut acts: Vec<(Slot, RowAct)> = Vec::new();
        let (mut to_device, mut toggle_unbound, mut reset) = (None, false, false);
        let waited = self.capture_since.map_or(0, |(_, at)| at.elapsed().as_secs());
        let armed = self.ctl.armed.is_some();
        let ctl = &mut self.ctl;
        let focus = &mut ctl.focus;
        let list_ref = &list;
        let b_ref: &Bindings = b;
        back |= side_panel(ctx, &spec, &hint_refs, |ui, reg| {
            let body = reg.body.shrink2(vec2(14.0 * s, 0.0));
            // The device switch above the list.
            let bar = Rect::from_min_size(
                pos2(body.left() + 6.0 * s, body.top() + 8.0 * s),
                vec2(body.width() - 12.0 * s, 40.0 * s),
            );
            let labels: Vec<String> = all.iter().map(|d| d.label(cabinet).to_string()).collect();
            let at = all.iter().position(|d| *d == device).unwrap_or(0);
            let (bar_resp, bar_id) = choice_bar(ui, bar, "controls-device", &labels, at);
            let list_area = Rect::from_min_max(pos2(body.left(), bar.bottom() + 8.0 * s), body.right_bottom());
            ui.scope_builder(egui::UiBuilder::new().max_rect(list_area), |ui| {
                focus.scroll_area("controls").show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0 * s;
                    for r in list_ref {
                        if let Some(act) = action_row(ui, focus, b_ref, r, capturing, waited, device) {
                            acts.push((r.slot, act));
                        }
                    }
                    if hidden > 0 || self.show_unbound {
                        let label = if self.show_unbound {
                            "Hide unbound inputs".to_string()
                        } else {
                            format!("Show {hidden} unbound inputs")
                        };
                        if focus.menu_row(ui, &label, "") {
                            toggle_unbound = true;
                        }
                    }
                });
            });
            // The switch comes last for the pad's focus (it starts on the
            // first action), but sits first on the screen.
            let (pad, accepted) = focus.stepper(ui, &bar_resp);
            if focus.is_on(bar_id) {
                theme::focus_ring(ui.painter(), bar, s, theme::ROW_RADIUS);
            }
            let mut choice = None;
            if bar_resp.clicked() {
                if let Some(p) = bar_resp.interact_pointer_pos() {
                    let w = bar.width() / all.len() as f32;
                    choice = Some((((p.x - bar.left()) / w) as usize).min(all.len() - 1));
                }
            }
            if pad != 0 {
                choice = Some((at as i32 + pad).clamp(0, all.len() as i32 - 1) as usize);
            }
            if accepted {
                choice = Some((at + 1) % all.len());
            }
            to_device = choice.filter(|&c| c != at);
            // The footer.
            let foot = reg.extra;
            ui.painter().hline(foot.x_range(), foot.top(), Stroke::new(1.0, Color32::from_white_alpha(20)));
            let (pad_x, h) = (16.0 * s, 40.0 * s);
            let x = foot.left() + pad_x;
            let w = foot.width() - 2.0 * pad_x;
            let label = if armed { "Again to confirm" } else { "Reset to the game's defaults" };
            if pill(
                ui,
                focus,
                Rect::from_min_size(pos2(x, foot.top() + 10.0 * s), vec2(w, h)),
                label,
                Some(theme::RAISED),
                true,
            ) {
                reset = true;
            }
            focus.shift_last(1, SHIFT);
            let (text, colour) = if conflicts {
                ("A key or button is on two actions: both get it.", theme::YELLOW)
            } else {
                ("Bindings are saved for this game.", theme::TEXT_DIM)
            };
            let p = ui.painter();
            let galley = p.layout(text.to_string(), plain(s, TEXT_SIZE), colour, w);
            p.galley(pos2(x, foot.top() + 58.0 * s), galley, colour);
            false
        });
        if let Some(d) = to_device {
            device = all[d];
            self.ctl.device = Some(device);
        }
        // The page's own acts, from the mouse (the pad's are applied above).
        for (slot, act) in acts {
            changed |= self.row_act(b, slot, act, device);
        }
        if toggle_unbound {
            self.show_unbound = !self.show_unbound;
        }
        if reset {
            if self.ctl.armed.is_some() {
                b.reset_all();
                settings.tuning = Default::default();
                self.capture = None;
                self.ctl.armed = None;
                changed = true;
                self.notify("Controls back to the game's defaults");
            } else {
                self.ctl.armed = Some(Instant::now());
                self.warn("Select Reset again to confirm");
            }
        }

        // The picture and the tuning, over the paused game.
        let (list, _) = rows(b, device, style, self.show_unbound);
        let focused_slot =
            self.ctl.focus.focused().and_then(|f| list.iter().find(|r| row_id(b, r.slot) == f)).map(|r| r.slot);
        self.controls_left(ctx, settings, prefs, view, b, &reg, device, cabinet, style, &list, focused_slot);
        self.ctl.focus.end();
        if back {
            self.back_to_pause();
            self.capture = None;
        }
        changed
    }

    /// Apply an action on a row; whether the bindings changed.
    fn row_act(&mut self, b: &mut Bindings, slot: Slot, act: RowAct, device: Device) -> bool {
        match act {
            RowAct::Bind(at) => {
                self.capture = Some((slot, at));
                false
            }
            RowAct::Clear => {
                let any = clear_device(b, slot, device);
                if !any {
                    self.notify("Nothing to clear");
                }
                any
            }
            RowAct::Restore => {
                b.reset(slot);
                true
            }
            RowAct::Invert => {
                b.invert(slot);
                true
            }
        }
    }

    /// The picture card and the tuning card, left of the panel.
    #[allow(clippy::too_many_arguments)]
    fn controls_left(
        &mut self,
        ctx: &egui::Context,
        settings: &mut GameSettings,
        prefs: Prefs,
        view: &View,
        b: &Bindings,
        reg: &Regions,
        device: Device,
        cabinet: Option<Art>,
        style: PadStyle,
        list: &[Row],
        focused: Option<Slot>,
    ) {
        let s = theme::scale(ctx);
        let screen = ctx.content_rect();
        let left = Rect::from_min_max(screen.min, pos2(reg.panel.left(), screen.bottom()));
        let cards = Rect::from_min_max(
            pos2(left.left() + MARGIN * s, left.top() + MARGIN * s),
            pos2(left.right() - MARGIN * s, left.bottom() - MARGIN * s),
        );
        let art = match device {
            Device::Cabinet => cabinet,
            Device::Pad => Some(Art::pad(style)),
            Device::Keys => None,
        };
        // The textures, loaded once.
        if let Some(a) = art {
            if let std::collections::hash_map::Entry::Vacant(e) = self.ctl.textures.entry(a) {
                if let Some(img) = figure::load(a) {
                    e.insert(ctx.load_texture(format!("controls-{a:?}"), img, egui::TextureOptions::LINEAR));
                }
            }
        }
        let tex = art.and_then(|a| self.ctl.textures.get(&a)).map(|t| t.id());
        let blocks = blocks(b);
        self.ctl.block = self.ctl.block.min(blocks.len().saturating_sub(1));
        let for_all = *prefs.for_all;
        let name = view.playing.unwrap_or_else(|| view.game.map_or("This game", |g| g.title));
        let short = if name.chars().count() > 16 { "This game" } else { name };
        let (mut scope, mut layout_click, mut block_pick) = (None, false, None);
        let mut tweaks: Vec<(Tuned, Tweak, usize)> = Vec::new();
        let ctl = &mut self.ctl;
        let focus = &mut ctl.focus;
        let block_at = ctl.block;
        let layout_text = match style {
            PadStyle::Offset => "Layout: offset sticks",
            PadStyle::SideBySide => "Layout: sticks side by side",
        };
        let pad_view = view.pad;
        egui::Area::new(Id::unique("remodel-controls-left")).order(egui::Order::Middle).fixed_pos(left.min).show(
            ctx,
            |ui| {
                // A scrim over the game's picture, and the pointer's own.
                ui.allocate_rect(left, Sense::click_and_drag());
                ui.painter().rect_filled(left, 0.0, Color32::from_black_alpha(150));
                let art_card = Rect::from_min_size(cards.min, vec2(cards.width(), ART_H * s));
                card(ui.painter(), art_card, s);
                let status = match focused.and_then(|f| list.iter().find(|r| r.slot == f)) {
                    Some(r) if !r.caps.is_empty() => {
                        let caps: Vec<&str> = r.caps.iter().map(|c| c.text.as_str()).collect();
                        Some(format!("{}: {}", r.name, caps.join(" or ")))
                    }
                    Some(r) => Some(format!("{}: not bound here", r.name)),
                    None => None,
                };
                let inner = art_card.shrink2(vec2(12.0 * s, 10.0 * s));
                let foot_h = 40.0 * s;
                let area = Rect::from_min_max(inner.min, pos2(inner.right(), inner.bottom() - foot_h));
                let mut tags: Vec<Tag> = Vec::new();
                let mut loose: Vec<String> = Vec::new();
                let mut picture_rect = None;
                match device {
                    Device::Keys => {
                        keyboard_picture(ui.painter(), area, s, b, list, focused);
                    }
                    _ => {
                        if let (Some(a), Some(id)) = (art, tex) {
                            // Room beside the picture for the labels.
                            let side = 190.0 * s;
                            let room = Rect::from_min_max(
                                pos2(area.left() + side, area.top()),
                                pos2(area.right() - side, area.bottom()),
                            );
                            let at = a.fit(room);
                            egui::Image::from_texture((id, at.size())).uv(a.uv()).paint_at(ui, at);
                            picture_rect = Some((a, at));
                        }
                    }
                }
                if let Some((a, at)) = picture_rect {
                    let (on, l) = part_actions(b, device, a, list);
                    loose = l;
                    for sp in figure::spots(a) {
                        let here: Vec<&OnPart> = on.iter().filter(|o| o.part == sp.part).collect();
                        if here.is_empty() {
                            continue;
                        }
                        let lit = here.iter().any(|o| o.slot.is_some() && o.slot == focused);
                        highlight(ui.painter(), a, at, &sp, lit, s);
                        let mut text: Vec<String> = Vec::new();
                        for o in &here {
                            if !text.contains(&o.label) {
                                text.push(o.label.clone());
                            }
                        }
                        let (x, y) = sp.pin();
                        tags.push(Tag { at: a.point(at, x, y), text: text.join(" / "), lit });
                    }
                }
                if let Some((_, at)) = picture_rect {
                    let placed = figure::place_labels(&tags, inner, at, 30.0 * s, area.top(), area.bottom());
                    draw_labels(ui.painter(), &tags, &placed, s);
                }
                // The foot line: the focused action and its bindings, or what
                // the picture has no part for.
                let line =
                    Rect::from_min_max(pos2(inner.left() + 8.0 * s, inner.bottom() - foot_h), inner.right_bottom());
                let p = ui.painter();
                if let Some(text) = status {
                    let sw =
                        Rect::from_center_size(pos2(line.left() + 6.0 * s, line.center().y), vec2(12.0 * s, 12.0 * s));
                    p.rect_filled(sw, 3.0 * s, theme::CYAN);
                    let font = plain(s, TEXT_SIZE);
                    let room = if device == Device::Pad { 330.0 * s } else { 0.0 };
                    let t = fit(p, &text, &font, line.width() - 28.0 * s - room);
                    p.text(pos2(line.left() + 22.0 * s, line.center().y), Align2::LEFT_CENTER, t, font, theme::TEXT_2);
                } else if !loose.is_empty() {
                    let font = plain(s, TEXT_SIZE);
                    let t = fit(p, &format!("Not on the picture: {}", loose.join(", ")), &font, line.width());
                    p.text(pos2(line.left(), line.center().y), Align2::LEFT_CENTER, t, font, theme::TEXT_DIM);
                }
                if device == Device::Pad {
                    // Which picture, by hand.
                    let font = plain(s, TEXT_SIZE);
                    let w = p.layout_no_wrap(layout_text.to_string(), font.clone(), theme::TEXT).size().x + 28.0 * s;
                    let rect = Rect::from_min_size(
                        pos2(art_card.right() - 12.0 * s - w, art_card.bottom() - 50.0 * s),
                        vec2(w, 40.0 * s),
                    );
                    let id = Id::unique("remodel-controls-layout");
                    let r = ui.interact(rect, id, Sense::click());
                    let p = ui.painter();
                    p.rect_filled(
                        rect,
                        theme::ROW_RADIUS * s,
                        Color32::from_white_alpha(if r.hovered() { 30 } else { 18 }),
                    );
                    p.text(rect.center(), Align2::CENTER_CENTER, layout_text, font, theme::TEXT);
                    if focus.is_on(id) {
                        theme::focus_ring(p, rect, s, theme::ROW_RADIUS);
                    }
                    layout_click = focus.register(ui, &r);
                }
                // The tuning.
                if !blocks.is_empty() {
                    let top = art_card.bottom() + 12.0 * s;
                    let card_rect = Rect::from_min_max(
                        pos2(cards.left(), top),
                        pos2(cards.right(), cards.bottom().max(top + 200.0 * s)),
                    );
                    card(ui.painter(), card_rect, s);
                    let blk = &blocks[block_at];
                    let inner = card_rect.shrink2(vec2(16.0 * s, 10.0 * s));
                    // The title, and where it's kept.
                    let title_row = Rect::from_min_size(inner.min, vec2(inner.width(), 48.0 * s));
                    let switch_w = 360.0 * s;
                    let switch = Rect::from_min_size(
                        pos2(title_row.right() - switch_w, title_row.top() + 2.0 * s),
                        vec2(switch_w, 44.0 * s),
                    );
                    let p = ui.painter();
                    let font = semibold(s, 20.0);
                    let t = fit(p, &blk.title, &font, title_row.width() - switch_w - 16.0 * s);
                    p.text(
                        pos2(title_row.left() + 4.0 * s, title_row.center().y),
                        Align2::LEFT_CENTER,
                        t,
                        font,
                        theme::TEXT,
                    );
                    let labels = [format!("{short} only"), "All games".to_string()];
                    ui.scope_builder(egui::UiBuilder::new().max_rect(switch), |ui| {
                        scope = scope_toggle(ui, focus, &labels, for_all as usize);
                    });
                    let mut y = title_row.bottom();
                    if blocks.len() > 1 {
                        let bar =
                            Rect::from_min_size(pos2(inner.left(), y), vec2(130.0 * s * blocks.len() as f32, 44.0 * s));
                        let labels: Vec<String> = blocks.iter().map(|k| k.kind.label().to_string()).collect();
                        let (resp, id) = choice_bar(ui, bar, "controls-block", &labels, block_at);
                        let (pad, accepted) = focus.stepper(ui, &resp);
                        if focus.is_on(id) {
                            theme::focus_ring(ui.painter(), bar, s, theme::ROW_RADIUS);
                        }
                        if resp.clicked() {
                            block_pick = resp
                                .interact_pointer_pos()
                                .map(|p| ((p.x - bar.left()) / (bar.width() / labels.len() as f32)) as usize);
                        }
                        if pad != 0 {
                            block_pick = Some((block_at as i32 + pad).clamp(0, labels.len() as i32 - 1) as usize);
                        }
                        if accepted {
                            block_pick = Some((block_at + 1) % labels.len());
                        }
                        y = bar.bottom() + 6.0 * s;
                    }
                    let r = response(settings, blk.kind);
                    // Narrow windows give the rows less, to leave the meter room.
                    let rows_w = (inner.width() - 218.0 * s).clamp(300.0 * s, 430.0 * s);
                    // Steering has a fourth row: in a short window the rows
                    // are closer, to fit the card.
                    let n_rows = if blk.kind == Tuned::Steer { 4.0 } else { 3.0 };
                    let row_h = ((inner.bottom() - y) / n_rows).clamp(40.0 * s, 54.0 * s);
                    let mut at_y = y;
                    let smooth = settings.tuning.smooth;
                    let at_smooth = Smoothing::ALL.iter().position(|m| *m == smooth).unwrap_or(0);
                    let mut rows_list = vec![
                        (
                            "Dead zone",
                            format!("{}%", r.dead),
                            Some(r.dead as f32 / Response::MAX_DEAD as f32),
                            (r.dead > 0, r.dead < Response::MAX_DEAD),
                        ),
                        (
                            "Sensitivity",
                            format!("{}%", r.sens),
                            Some(
                                (r.sens - *Response::SENS.start()) as f32
                                    / (*Response::SENS.end() - *Response::SENS.start()) as f32,
                            ),
                            (r.sens > *Response::SENS.start(), r.sens < *Response::SENS.end()),
                        ),
                        (
                            "Curve",
                            r.curve.label().to_string(),
                            None,
                            (r.curve != Curve::ALL[0], r.curve != Curve::ALL[Curve::ALL.len() - 1]),
                        ),
                    ];
                    // Steering only: how much a pad stick's shake is smoothed.
                    if blk.kind == Tuned::Steer {
                        rows_list.push((
                            "Smoothing",
                            smooth.label().to_string(),
                            None,
                            (at_smooth > 0, at_smooth + 1 < Smoothing::ALL.len()),
                        ));
                    }
                    for (i, (label, value, frac, can)) in rows_list.into_iter().enumerate() {
                        let rect = Rect::from_min_size(pos2(inner.left(), at_y), vec2(rows_w, row_h - 4.0 * s));
                        let tw = tune_row(
                            ui,
                            focus,
                            Id::unique(("remodel-controls-tune", i)),
                            rect,
                            label,
                            &value,
                            frac,
                            can,
                        );
                        if tw != Tweak::None {
                            tweaks.push((blk.kind, tw, i));
                        }
                        at_y += row_h;
                    }
                    // The response and the live meter.
                    let right = Rect::from_min_max(pos2(inner.left() + rows_w + 28.0 * s, y), inner.right_bottom());
                    live_meter(ui.painter(), right, s, &pad_view, blk, r, smooth);
                    // One plain sentence on what the tuning is for.
                    let galley = ui.painter().layout(
                        blk.kind.help().to_string(),
                        plain(s, 15.0),
                        theme::TEXT_DIM,
                        right.width(),
                    );
                    let at = pos2(right.left(), right.top() + 116.0 * s);
                    if at.y + galley.size().y <= right.bottom() {
                        ui.painter().galley(at, galley, theme::TEXT_DIM);
                    }
                }
            },
        );
        if let Some(i) = scope {
            *prefs.for_all = i == 1;
        }
        if let Some(i) = block_pick {
            self.ctl.block = i.min(blocks.len().saturating_sub(1));
        }
        if layout_click {
            self.ctl.layout = Some(match style {
                PadStyle::Offset => PadStyle::SideBySide,
                PadStyle::SideBySide => PadStyle::Offset,
            });
        }
        for (kind, tw, row) in tweaks {
            if kind == Tuned::Steer && row == 3 {
                tune_smoothing(&mut settings.tuning.smooth, tw);
            } else {
                tune(response_mut(settings, kind), row, tw);
            }
        }
    }
}

/// The devices the page offers.
fn devices(cabinet: Option<Art>) -> Vec<Device> {
    let mut v = Vec::new();
    if cabinet.is_some() {
        v.push(Device::Cabinet);
    }
    v.extend([Device::Pad, Device::Keys]);
    v
}

/// A card.
fn card(p: &Painter, r: Rect, s: f32) {
    p.rect_filled(r, theme::CARD_RADIUS * s, CARD);
    p.rect_stroke(r, theme::CARD_RADIUS * s, Stroke::new(1.0, Color32::from_white_alpha(20)), egui::StrokeKind::Inside);
}

/// A bar of choices in `rect`, the chosen one filled; returns its response
/// (a click on a segment, `interact_pointer_pos`) and id for the focus.
fn choice_bar(ui: &mut egui::Ui, rect: Rect, name: &str, labels: &[String], selected: usize) -> (egui::Response, Id) {
    let s = theme::scale(ui.ctx());
    let id = Id::unique(("remodel-choice-bar", name));
    let r = ui.interact(rect, id, Sense::click());
    let p = ui.painter();
    let radius = theme::ROW_RADIUS * s;
    p.rect_filled(rect, radius, Color32::from_white_alpha(18));
    let w = rect.width() / labels.len() as f32;
    for (i, label) in labels.iter().enumerate() {
        let seg =
            Rect::from_min_size(pos2(rect.left() + i as f32 * w, rect.top()), vec2(w, rect.height())).shrink(4.0 * s);
        let on = i == selected;
        if on {
            p.rect_filled(seg, radius - 3.0 * s, theme::TEXT);
        }
        let colour = if on { theme::INK } else { theme::TEXT };
        p.text(seg.center(), Align2::CENTER_CENTER, label, semibold(s, TEXT_SIZE), colour);
    }
    (r, id)
}

/// An action's row: its name, its bindings as chips and what shares them.
/// Clicked, or activated from the pad: bind in place of the first; a click
/// on a chip rebinds that one; the buttons on a hovered row add, clear or
/// put back.
fn action_row(
    ui: &mut egui::Ui,
    focus: &mut Focus,
    b: &Bindings,
    row: &Row,
    capturing: Option<(Slot, Option<usize>)>,
    waited: u64,
    device: Device,
) -> Option<RowAct> {
    let s = theme::scale(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 62.0 * s), Sense::hover());
    let id = row_id(b, row.slot);
    let r = ui.interact(rect, id, Sense::click());
    let waiting = capturing.filter(|c| c.0 == row.slot);
    let focused = focus.is_on(id);
    let radius = theme::ROW_RADIUS * s;
    let mut act = None;
    let p = ui.painter().clone();
    if let Some((_, at)) = waiting {
        p.rect_filled(rect, radius, Color32::from_rgba_unmultiplied(34, 229, 255, 22));
        p.rect_stroke(rect, radius, Stroke::new(2.0 * s, theme::CYAN), egui::StrokeKind::Inside);
        let _ = at;
    } else if focused {
        theme::menu_row_focus(&p, rect, s);
    } else if r.hovered() {
        p.rect_filled(rect, radius, Color32::from_white_alpha(14));
    }
    let x = rect.left() + 18.0 * s;
    // Name, and (on a hovered or focused row) its buttons.
    let hot = r.hovered() || focused;
    let mut right = rect.right() - 10.0 * s;
    let top = rect.top() + 8.0 * s;
    if hot && waiting.is_none() {
        let mut button = |text: &str, wide: f32, act_to: RowAct, tip: &str| {
            let rc = Rect::from_min_size(pos2(right - wide, top - 2.0 * s), vec2(wide, 26.0 * s));
            let bid = id.with(text);
            let br = ui.interact(rc, bid, Sense::click());
            let fill = Color32::from_white_alpha(if br.hovered() { 56 } else { 30 });
            p.rect_filled(rc, 8.0 * s, fill);
            p.text(rc.center(), Align2::CENTER_CENTER, text, semibold(s, TEXT_SIZE), theme::TEXT);
            let br = br.on_hover_text(tip);
            right = rc.left() - 6.0 * s;
            br.clicked().then_some(act_to)
        };
        let axis = b.accepts(row.slot) == Accepts::Axis;
        let first = if axis {
            button("Flip", 58.0 * s, RowAct::Invert, "Invert the axis")
        } else {
            button("+", 36.0 * s, RowAct::Bind(None), "Add another binding")
        };
        let second = button("×", 36.0 * s, RowAct::Clear, "Clear this device's bindings");
        let third =
            if row.edited { button("Reset", 70.0 * s, RowAct::Restore, "Back to the game's own") } else { None };
        act = first.or(second).or(third);
    }
    let font = semibold(s, 17.0);
    let name = fit(&p, &row.name, &font, right - x);
    p.text(pos2(x, top), Align2::LEFT_TOP, name, font, theme::TEXT);
    if row.edited {
        p.circle_filled(pos2(rect.left() + 10.0 * s, top + 10.0 * s), 3.5 * s, theme::CYAN);
    }
    // The line below: what it's bound to.
    let cy = rect.bottom() - 19.0 * s;
    if waiting.is_some() {
        let what = if b.accepts(row.slot) == Accepts::Axis {
            "Move a stick or press a trigger"
        } else {
            "Press a key or button"
        };
        let left = super::CAPTURE_WAIT.as_secs().saturating_sub(waited);
        let text = format!("{what}…  {left}");
        let font = semibold(s, TEXT_SIZE);
        let w = (p.layout_no_wrap(text.clone(), font.clone(), theme::INK).size().x + 28.0 * s)
            .min(rect.right() - x - 10.0 * s);
        let pill = Rect::from_min_size(pos2(x, cy - 12.0 * s), vec2(w, 24.0 * s));
        p.rect_filled(pill, 8.0 * s, theme::CYAN);
        p.text(pill.center(), Align2::CENTER_CENTER, fit(&p, &text, &font, w - 16.0 * s), font, theme::INK);
        ui.ctx().request_repaint();
    } else if row.caps.is_empty() {
        p.text(pos2(x, cy), Align2::LEFT_CENTER, "not bound here", plain(s, TEXT_SIZE), theme::TEXT_DIM);
    } else {
        let font = semibold(s, TEXT_SIZE);
        let mut cx = x;
        for cap in &row.caps {
            let w = p.layout_no_wrap(cap.text.clone(), font.clone(), theme::TEXT).size().x + 18.0 * s;
            if cx + w > rect.right() - 8.0 * s {
                break;
            }
            let chip = Rect::from_min_size(pos2(cx, cy - 12.0 * s), vec2(w, 24.0 * s));
            let cid = id.with(("cap", cap.index));
            let cr = ui.interact(chip, cid, Sense::click());
            let (fill, ink) = if cap.clash {
                (Color32::from_rgba_unmultiplied(255, 210, 63, 36), theme::YELLOW)
            } else {
                (Color32::from_white_alpha(if cr.hovered() { 46 } else { 26 }), theme::TEXT)
            };
            p.rect_filled(chip, 8.0 * s, fill);
            let stroke = if cap.clash {
                Stroke::new(1.5 * s, theme::YELLOW)
            } else {
                Stroke::new(1.0, Color32::from_white_alpha(46))
            };
            p.rect_stroke(chip, 8.0 * s, stroke, egui::StrokeKind::Inside);
            p.text(chip.center(), Align2::CENTER_CENTER, &cap.text, font.clone(), ink);
            if cr.on_hover_text("Click to replace this one").clicked() && act.is_none() {
                act = Some(RowAct::Bind(Some(cap.index)));
            }
            cx += w + 6.0 * s;
        }
        if let Some(also) = &row.also {
            let font = plain(s, TEXT_SIZE);
            let t = fit(&p, also, &font, rect.right() - 8.0 * s - cx);
            if !t.is_empty() && t != "…" {
                p.text(pos2(cx + 4.0 * s, cy), Align2::LEFT_CENTER, t, font, theme::YELLOW);
            }
        }
    }
    let _ = device;
    if focused {
        // (The row's own ring is drawn above.)
    }
    let clicked = focus.register(ui, &r);
    if clicked && act.is_none() {
        // The row itself: in place of the device's first binding.
        let first = row.caps.first().filter(|_| b.accepts(row.slot) != Accepts::Axis).map(|c| c.index);
        act = Some(RowAct::Bind(first));
    }
    act
}

/// The picture's lit part: a faint outline for any with an action, the
/// focused action's glowing.
fn highlight(p: &Painter, art: Art, at: Rect, sp: &Spot, lit: bool, s: f32) {
    let c = art.point(at, sp.x, sp.y);
    let c4 = art.crop();
    let size = vec2(sp.w * at.width() / (c4[2] - c4[0]), sp.h * at.height() / (c4[3] - c4[1]));
    let rect = Rect::from_center_size(c, size + vec2(6.0, 6.0) * s);
    let radius = (rect.width().min(rect.height()) / 2.0).min(if sp.w == sp.h { 999.0 } else { 14.0 * s });
    let (fill, stroke) = if lit {
        (Color32::from_rgba_unmultiplied(34, 229, 255, 70), Stroke::new(3.5 * s, theme::CYAN))
    } else {
        (
            Color32::from_rgba_unmultiplied(34, 229, 255, 18),
            Stroke::new(1.5 * s, Color32::from_rgba_unmultiplied(34, 229, 255, 120)),
        )
    };
    match sp.shape {
        Shape::Round => {
            if lit {
                p.rect_stroke(
                    rect.expand(5.0 * s),
                    radius + 5.0 * s,
                    Stroke::new(6.0 * s, Color32::from_rgba_unmultiplied(34, 229, 255, 60)),
                    egui::StrokeKind::Outside,
                );
            }
            p.rect_filled(rect, radius, fill);
            p.rect_stroke(rect, radius, stroke, egui::StrokeKind::Inside);
        }
        Shape::Ring => {
            let r = rect.width() / 2.0 - 2.0 * s;
            if lit {
                p.circle_stroke(c, r, Stroke::new(12.0 * s, Color32::from_rgba_unmultiplied(34, 229, 255, 60)));
            }
            p.circle_stroke(c, r, stroke);
        }
    }
}

/// The labels beside a picture and their leader lines.
fn draw_labels(p: &Painter, tags: &[Tag], placed: &[Placed], s: f32) {
    for pl in placed {
        let tag = &tags[pl.index];
        let colour = if tag.lit { theme::CYAN } else { Color32::from_rgba_unmultiplied(34, 229, 255, 110) };
        let dir = if pl.left { 1.0 } else { -1.0 };
        let stub = pos2(pl.from.x + 14.0 * s * dir, pl.from.y);
        let stroke = Stroke::new(if tag.lit { 2.0 * s } else { 1.2 * s }, colour);
        p.add(Mesh::line(vec![pl.from, stub, pl.to], stroke));
        p.circle_filled(pl.to, if tag.lit { 4.5 * s } else { 3.0 * s }, colour);
        let font = semibold(s, TEXT_SIZE);
        let text = fit(p, &tag.text, &font, pl.label.width() - 4.0 * s);
        let (align, at) = if pl.left {
            (Align2::RIGHT_CENTER, pos2(pl.label.right() - 2.0 * s, pl.label.center().y))
        } else {
            (Align2::LEFT_CENTER, pos2(pl.label.left() + 2.0 * s, pl.label.center().y))
        };
        p.text(at, align, text, font, if tag.lit { theme::TEXT } else { theme::TEXT_2 });
    }
}

/// Where the keyboard picture goes in `area`.
fn keyboard_rect(area: Rect) -> Rect {
    let (uw, uh) = figure::KEYBOARD_UNITS;
    let u = (area.width() / uw).min(area.height() / uh);
    Rect::from_center_size(area.center(), vec2(u * uw, u * uh))
}

/// The keyboard with the game's keys lit, the focused action's brightest.
fn keyboard_picture(p: &Painter, area: Rect, s: f32, b: &Bindings, list: &[Row], focused: Option<Slot>) {
    let whole = keyboard_rect(area);
    let u = whole.width() / figure::KEYBOARD_UNITS.0;
    let conflicts = b.conflicts();
    let bound = |code: &str| -> Vec<Slot> {
        list.iter()
            .filter(|r| matches!(b.get(r.slot), Some(Value::Inputs(l)) if l.iter().any(|i| matches!(i, Input::Key(k) if k == code))))
            .map(|r| r.slot)
            .collect()
    };
    for cap in figure::keyboard() {
        let rect =
            Rect::from_min_size(whole.min + vec2(cap.x * u, cap.row as f32 * u), vec2(cap.w * u, u)).shrink(2.5 * s);
        let slots = bound(cap.code);
        let lit = focused.is_some_and(|f| slots.contains(&f));
        let clash = conflicts.iter().any(|c| matches!(&c.input, Input::Key(k) if k == cap.code));
        let (fill, ink, stroke) = if lit {
            (theme::CYAN, theme::INK, Stroke::NONE)
        } else if clash {
            (Color32::from_rgba_unmultiplied(255, 210, 63, 40), theme::YELLOW, Stroke::new(2.0 * s, theme::YELLOW))
        } else if !slots.is_empty() {
            (
                Color32::from_rgba_unmultiplied(34, 229, 255, 60),
                theme::TEXT,
                Stroke::new(1.5 * s, Color32::from_rgba_unmultiplied(34, 229, 255, 150)),
            )
        } else {
            (Color32::from_rgb(34, 42, 64), theme::TEXT_DIM, Stroke::NONE)
        };
        p.rect_filled(rect, 7.0 * s, fill);
        if stroke != Stroke::NONE {
            p.rect_stroke(rect, 7.0 * s, stroke, egui::StrokeKind::Inside);
        }
        p.text(rect.center(), Align2::CENTER_CENTER, figure::cap_text(cap.code), semibold(s, TEXT_SIZE), ink);
    }
}

/// What a tuning row did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Tweak {
    None,
    /// Steps (left and right on the pad, the arrows).
    By(i32),
    /// To this fraction of the track (the pointer).
    To(f32),
    /// On to the next choice (A).
    Cycle,
}

/// Apply a tuning row's change (0 dead zone, 1 sensitivity, 2 curve).
pub(super) fn tune(r: &mut Response, row: usize, tw: Tweak) {
    match (row, tw) {
        (_, Tweak::None) => {}
        (0, Tweak::By(n)) => r.dead = (r.dead as i32 + n).clamp(0, Response::MAX_DEAD as i32) as u8,
        (0, Tweak::To(f)) => r.dead = (f.clamp(0.0, 1.0) * Response::MAX_DEAD as f32).round() as u8,
        (1, Tweak::By(n)) => {
            r.sens = (r.sens as i32 + 5 * n).clamp(*Response::SENS.start() as i32, *Response::SENS.end() as i32) as u8
        }
        (1, Tweak::To(f)) => {
            let (lo, hi) = (*Response::SENS.start() as f32, *Response::SENS.end() as f32);
            r.sens = ((lo + f.clamp(0.0, 1.0) * (hi - lo)) / 5.0).round() as u8 * 5;
        }
        (2, Tweak::By(n)) => {
            let at = Curve::ALL.iter().position(|c| *c == r.curve).unwrap_or(0) as i32;
            r.curve = Curve::ALL[(at + n).clamp(0, Curve::ALL.len() as i32 - 1) as usize];
        }
        (2, Tweak::Cycle) => {
            let at = Curve::ALL.iter().position(|c| *c == r.curve).unwrap_or(0);
            r.curve = Curve::ALL[(at + 1) % Curve::ALL.len()];
        }
        _ => {}
    }
}

/// Apply a change to the steering smoothing row: a choice, so a step moves
/// to the next level, A cycles round.
pub(super) fn tune_smoothing(m: &mut Smoothing, tw: Tweak) {
    let at = Smoothing::ALL.iter().position(|x| x == m).unwrap_or(0) as i32;
    let last = Smoothing::ALL.len() as i32 - 1;
    let to = match tw {
        Tweak::None | Tweak::To(_) => return,
        Tweak::By(n) => (at + n).clamp(0, last),
        Tweak::Cycle => (at + 1) % (last + 1),
    };
    *m = Smoothing::ALL[to as usize];
}

/// A tuning row: "label ◂ track ▸ value". Left and right on the pad step it
/// (`Tweak::By`), A cycles a choice; a click on an arrow steps, on or a drag
/// along the track sets.
#[allow(clippy::too_many_arguments)]
fn tune_row(
    ui: &mut egui::Ui,
    focus: &mut Focus,
    id: Id,
    rect: Rect,
    label: &str,
    value: &str,
    frac: Option<f32>,
    can: (bool, bool),
) -> Tweak {
    let s = theme::scale(ui.ctx());
    let r = ui.interact(rect, id, Sense::click_and_drag());
    let (pad, accepted) = focus.stepper(ui, &r);
    let p = ui.painter().clone();
    if focus.is_on(id) {
        theme::focus_ring(&p, rect, s, theme::ROW_RADIUS);
    } else if r.hovered() {
        p.rect_filled(rect, theme::ROW_RADIUS * s, Color32::from_white_alpha(14));
    }
    let cy = rect.center().y;
    p.text(pos2(rect.left() + 14.0 * s, cy), Align2::LEFT_CENTER, label, semibold(s, TEXT_SIZE), theme::TEXT_2);
    let arrow = 30.0 * s;
    let value_w = 78.0 * s;
    let left = Rect::from_min_size(pos2(rect.left() + 138.0 * s, rect.top()), vec2(arrow, rect.height()));
    let right =
        Rect::from_min_size(pos2(rect.right() - value_w - arrow - 8.0 * s, rect.top()), vec2(arrow, rect.height()));
    let track =
        Rect::from_min_max(pos2(left.right() + 4.0 * s, cy - 3.0 * s), pos2(right.left() - 4.0 * s, cy + 3.0 * s));
    for (rc, dir, on) in [(left, -1.0, can.0), (right, 1.0, can.1)] {
        let c = rc.center();
        let colour = if on { theme::CYAN } else { theme::TEXT_DIM.gamma_multiply(0.4) };
        let tip = c + vec2(7.0 * dir, 0.0) * s;
        let back = c - vec2(5.0 * dir, 0.0) * s;
        p.add(Mesh::convex_polygon(
            vec![tip, back + vec2(0.0, -8.0 * s), back + vec2(0.0, 8.0 * s)],
            colour,
            Stroke::NONE,
        ));
    }
    match frac {
        Some(f) => {
            p.rect_filled(track, 3.0 * s, Color32::from_white_alpha(30));
            let fill = Rect::from_min_size(track.min, vec2(track.width() * f.clamp(0.0, 1.0), track.height()));
            p.rect_filled(fill, 3.0 * s, theme::CYAN);
            p.circle_filled(pos2(fill.right(), cy), 8.0 * s, theme::TEXT);
        }
        None => {
            // A choice: its name in the middle of the track's place.
            let _ = track;
        }
    }
    p.text(pos2(rect.right() - 10.0 * s, cy), Align2::RIGHT_CENTER, value, semibold(s, 20.0), theme::TEXT);
    let mut out = Tweak::None;
    if r.clicked() || r.dragged() {
        if let Some(at) = r.interact_pointer_pos() {
            let wide = track.expand2(vec2(6.0 * s, 14.0 * s));
            if r.clicked() && left.contains(at) {
                out = Tweak::By(-1);
            } else if r.clicked() && right.contains(at) {
                out = Tweak::By(1);
            } else if frac.is_some() && wide.contains(at) {
                out = Tweak::To((at.x - track.left()) / track.width());
            } else if frac.is_none() && r.clicked() {
                out = Tweak::Cycle;
            }
        }
    }
    if pad != 0 {
        out = Tweak::By(pad);
    } else if accepted && frac.is_none() {
        out = Tweak::Cycle;
    }
    out
}

/// The response curve (the dead zone shaded, the curve, a dot where the
/// stick is now) and the raw and tuned readings as bars.
fn live_meter(p: &Painter, area: Rect, s: f32, pad: &PadLive, block: &Block, resp: Response, smooth: Smoothing) {
    let graph = Rect::from_min_size(area.min, vec2(104.0 * s, 104.0 * s));
    p.rect_filled(graph, 8.0 * s, Color32::from_rgb(13, 18, 32));
    let at = |x: f32, y: f32| pos2(graph.left() + x * graph.width(), graph.bottom() - y * graph.height());
    let dead = resp.dead as f32 / 100.0;
    p.rect_filled(
        Rect::from_min_max(graph.left_top(), pos2(at(dead, 0.0).x, graph.bottom())),
        0.0,
        Color32::from_rgba_unmultiplied(255, 77, 94, 46),
    );
    p.line_segment([at(0.0, 0.0), at(1.0, 1.0)], Stroke::new(1.0, Color32::from_white_alpha(36)));
    let pts: Vec<Pos2> = (0..=48).map(|i| i as f32 / 48.0).map(|x| at(x, resp.apply(x))).collect();
    p.add(Mesh::line(pts, Stroke::new(2.5 * s, theme::CYAN)));
    let raw = block.axis.read(|a| pad.axis(a));
    let tuned = resp.apply(raw);
    // Steering's second bar is what the game reads: past the smoothing too.
    let (read, read_name) = match (block.kind, smooth) {
        (Tuned::Steer, Smoothing::Off) | (Tuned::Stick | Tuned::Pedal, _) => (tuned, "Tuned"),
        (Tuned::Steer, _) => (block.axis.read(|a| pad.smoothed(a)), "Smoothed"),
    };
    if pad.connected {
        let c = at(raw.abs().min(1.0), tuned.abs().min(1.0));
        p.circle_filled(c, 6.0 * s, theme::TEXT);
        p.circle_stroke(c, 8.0 * s, Stroke::new(2.0 * s, theme::CYAN));
    }
    // The bars: -1 to 1 for a stick, 0 to 1 for a trigger.
    let x0 = graph.right() + 16.0 * s;
    let w = area.right() - x0;
    // In a narrow window the graph alone fits.
    if w < 110.0 * s {
        return;
    }
    let font = plain(s, TEXT_SIZE);
    for (i, (name, v, colour)) in
        [("Raw", raw, Color32::from_white_alpha(120)), (read_name, read, theme::CYAN)].into_iter().enumerate()
    {
        let y = area.top() + i as f32 * 54.0 * s;
        let pct = match (pad.connected, block.kind.bidirectional()) {
            (false, _) => "no pad".to_string(),
            (true, true) => format!("{:+.0}%", v * 100.0),
            (true, false) => format!("{:.0}%", v * 100.0),
        };
        p.text(pos2(x0, y), Align2::LEFT_TOP, name, font.clone(), theme::TEXT_DIM);
        p.text(pos2(x0 + w, y), Align2::RIGHT_TOP, pct, font.clone(), theme::TEXT);
        let bar = Rect::from_min_size(pos2(x0, y + 26.0 * s), vec2(w, 14.0 * s));
        p.rect_filled(bar, 7.0 * s, Color32::from_white_alpha(26));
        let (lo, hi) = if block.kind.bidirectional() {
            ((bar.center().x), bar.center().x + v.clamp(-1.0, 1.0) * bar.width() / 2.0)
        } else {
            (bar.left(), bar.left() + v.clamp(0.0, 1.0) * bar.width())
        };
        let fill = Rect::from_x_y_ranges(lo.min(hi)..=lo.max(hi), bar.y_range());
        p.rect_filled(fill, 7.0 * s, colour);
        if block.kind.bidirectional() {
            p.vline(bar.center().x, bar.y_range(), Stroke::new(1.5 * s, Color32::from_white_alpha(90)));
        }
    }
}

#[cfg(test)]
mod tests;
