//! A player's bindings: the manifest's default keys and pad bindings
//! ([`Controls`]) with the player's own changes on top. Both players (the
//! native window and the browser) remap through this, so they agree on
//! what can be bound, how it's stored and how conflicts are found; scripted
//! `--input` doesn't use bindings at all (it names the cabinet's inputs).
//!
//! The bindable things are [`Slot`]s: a button's list of keys and pad
//! buttons, an analog input's axis (a stick or trigger, maybe inverted) and
//! the keys and buttons that hold it at full negative and positive
//! deflection, a light gun's reload keys, and the pad buttons that fire and
//! reload the gun the pad's crosshair aims. Only the slots that differ from
//! the manifest are stored ([`Bindings::overrides`]), one per line:
//!
//! ```text
//! jumpfront = KeyX pad:South
//! curving.axis = -RightStickX
//! slide+ =
//! ```
//!
//! A key is its `KeyboardEvent.code`, a pad button `pad:` and its name
//! ([`PadButton`]; the stick directions `LeftStickUp` ... are buttons read
//! from the sticks), an axis its [`PadAxis`] name (`-` inverts, `None` for no
//! axis); an empty list clears the slot. Lines that don't parse (an unknown
//! slot, a bad or reserved key, the wrong kind of binding) are ignored, so
//! stored data from another version or a hand edit can't break a game.

use crate::controls::{key_label, key_symbol, Controls, PadAxis, PadButton, Reload};

/// The players' own keys (menu, quick save and load, fullscreen): never
/// bound to a game's input. The browser adds its own (`KeyF`, `KeyH`).
pub const RESERVED_KEYS: &[&str] = &["Escape", "F5", "F8", "F9", "F11"];

/// Default pad buttons that fire and reload the gun the pad's crosshair
/// aims (both sticks move it).
pub const PAD_GUN_FIRE: &[PadButton] = &[PadButton::RightTrigger, PadButton::South];
pub const PAD_GUN_RELOAD: &[PadButton] = &[PadButton::LeftTrigger, PadButton::East];

/// A digital binding: a key (`KeyboardEvent.code`) or a pad button.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Input {
    Key(String),
    Pad(PadButton),
}

impl Input {
    /// As stored: the key code, or `pad:` and the button's name.
    pub fn token(&self) -> String {
        match self {
            Input::Key(k) => k.clone(),
            Input::Pad(b) => format!("pad:{b:?}"),
        }
    }

    /// Parse a stored token; None if it isn't one.
    pub fn parse(s: &str) -> Option<Input> {
        match s.strip_prefix("pad:") {
            Some(b) => PadButton::from_name(b).map(Input::Pad),
            None => valid_key(s).then(|| Input::Key(s.to_string())),
        }
    }

    /// What players see: the key as printed on it, or the pad button as an
    /// Xbox pad labels it.
    pub fn text(&self) -> String {
        match self {
            Input::Key(k) => key_symbol(k).to_string(),
            Input::Pad(b) => b.label().to_string(),
        }
    }
}

/// A `KeyboardEvent.code`-like name: letters and digits, not too long.
fn valid_key(s: &str) -> bool {
    !s.is_empty() && s.len() <= 32 && s.bytes().all(|c| c.is_ascii_alphanumeric())
}

/// A pad axis bound to an analog input, maybe inverted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Axis {
    pub axis: PadAxis,
    pub invert: bool,
}

impl Axis {
    pub const NONE: Axis = Axis { axis: PadAxis::None, invert: false };

    pub fn token(&self) -> String {
        format!("{}{:?}", if self.invert && self.axis != PadAxis::None { "-" } else { "" }, self.axis)
    }

    pub fn parse(s: &str) -> Option<Axis> {
        let (invert, name) = s.strip_prefix('-').map_or((false, s), |n| (true, n));
        let axis = PadAxis::from_name(name)?;
        Some(Axis { axis, invert: invert && axis != PadAxis::None })
    }

    pub fn text(&self) -> String {
        match self.axis {
            PadAxis::None => String::new(),
            a if self.invert => format!("{} (inverted)", a.label()),
            a => a.label().to_string(),
        }
    }

    /// The deflection it gives, from the pad's reading of its axis.
    pub fn read(&self, pad_axis: impl Fn(PadAxis) -> f32) -> f32 {
        match self.axis {
            PadAxis::None => 0.0,
            a if self.invert => -pad_axis(a),
            a => pad_axis(a),
        }
    }
}

/// Something bindable. Indices are into the manifest's `buttons`,
/// `analogs` and `guns`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    /// A button's keys and pad buttons.
    Button(usize),
    /// An analog input's axis.
    Axis(usize),
    /// Keys and pad buttons holding an analog at full negative deflection
    /// (and positive).
    Neg(usize),
    Pos(usize),
    /// A gun's reload keys and pad buttons (guns that reload by shooting
    /// off the screen).
    Reload(usize),
    /// Pad buttons that fire and reload the gun the pad's crosshair aims.
    PadFire,
    PadReload,
}

/// What a slot takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accepts {
    /// Keys and pad buttons.
    Inputs,
    /// Pad buttons only.
    Pad,
    /// A pad axis (a stick or a trigger).
    Axis,
}

impl Accepts {
    pub fn name(self) -> &'static str {
        match self {
            Accepts::Inputs => "inputs",
            Accepts::Pad => "pad",
            Accepts::Axis => "axis",
        }
    }
}

/// A slot's bindings.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Inputs(Vec<Input>),
    Axis(Axis),
}

impl Value {
    pub fn inputs(&self) -> &[Input] {
        match self {
            Value::Inputs(v) => v,
            Value::Axis(_) => &[],
        }
    }

    pub fn axis(&self) -> Axis {
        match self {
            Value::Axis(a) => *a,
            Value::Inputs(_) => Axis::NONE,
        }
    }

    fn token(&self) -> String {
        match self {
            Value::Inputs(v) => v.iter().map(Input::token).collect::<Vec<_>>().join(" "),
            Value::Axis(a) => a.token(),
        }
    }
}

/// What a player pressed or moved to bind it ([`Bindings::capture`]).
#[derive(Clone, Debug, PartialEq)]
pub enum Capture {
    /// A key (`KeyboardEvent.code`).
    Key(String),
    Button(PadButton),
    /// A stick axis moved, towards its negative end if `true` (sticks read
    /// -1 left or up to 1 right or down).
    Axis(PadAxis, bool),
}

/// A key or button bound in more than one slot.
#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    pub input: Input,
    pub slots: Vec<Slot>,
}

/// A game's bindings: the manifest's, with a player's changes.
#[derive(Clone)]
pub struct Bindings {
    controls: &'static Controls,
    /// Every slot, in the order players see them: each gun's trigger and
    /// reload, the pad crosshair's, the analogs, then the other buttons.
    slots: Vec<(Slot, Value)>,
    defaults: Vec<Value>,
}

fn inputs(keys: &[&str], pad: &[PadButton]) -> Value {
    let mut v: Vec<Input> = keys.iter().map(|k| Input::Key(k.to_string())).collect();
    v.extend(pad.iter().map(|&b| Input::Pad(b)));
    Value::Inputs(v)
}

impl Bindings {
    /// The manifest's bindings.
    pub fn defaults(controls: &'static Controls) -> Bindings {
        let c = controls;
        let mut slots = Vec::new();
        let mut placed = Vec::new();
        for (i, g) in c.guns.iter().enumerate() {
            if let Some(t) = c.buttons.iter().position(|b| b.name == g.trigger) {
                slots.push((Slot::Button(t), inputs(c.buttons[t].keys, c.buttons[t].pad)));
                placed.push(t);
            }
            if g.reload == Reload::Offscreen {
                slots.push((Slot::Reload(i), inputs(g.keys_reload, &[])));
            }
        }
        if !c.guns.is_empty() {
            slots.push((Slot::PadFire, inputs(&[], PAD_GUN_FIRE)));
            slots.push((Slot::PadReload, inputs(&[], PAD_GUN_RELOAD)));
        }
        for (i, a) in c.analogs.iter().enumerate() {
            slots.push((Slot::Axis(i), Value::Axis(Axis { axis: a.axis, invert: false })));
            slots.push((Slot::Neg(i), inputs(a.keys_neg, a.pad_neg)));
            slots.push((Slot::Pos(i), inputs(a.keys_pos, a.pad_pos)));
        }
        // A digital game's d-pad is the left stick's too: its directions
        // also read the stick (`StickDirs`), unless the game steers or aims
        // with that stick's axis, or has a gun the sticks move.
        let on_axis = |x: PadAxis| c.analogs.iter().any(|a| a.axis == x);
        let stick_free = |b: PadButton| {
            c.guns.is_empty()
                && match b {
                    PadButton::DPadUp | PadButton::DPadDown => !on_axis(PadAxis::LeftStickY),
                    _ => !on_axis(PadAxis::LeftStickX),
                }
        };
        for (i, b) in c.buttons.iter().enumerate() {
            if !placed.contains(&i) {
                let mut v = inputs(b.keys, b.pad);
                if let Value::Inputs(list) = &mut v {
                    for &p in b.pad {
                        let Some(stick) = p.left_stick_of().filter(|_| stick_free(p)) else { continue };
                        if !list.contains(&Input::Pad(stick)) {
                            list.push(Input::Pad(stick));
                        }
                    }
                }
                slots.push((Slot::Button(i), v));
            }
        }
        let defaults = slots.iter().map(|s| s.1.clone()).collect();
        Bindings { controls, slots, defaults }
    }

    /// The manifest's bindings with the stored `overrides` (see the module
    /// docs) on top; lines that don't parse, or that bind a key in
    /// `reserved`, are ignored.
    pub fn with_overrides(controls: &'static Controls, overrides: &str, reserved: &[&str]) -> Bindings {
        let mut b = Bindings::defaults(controls);
        for line in overrides.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((name, value)) = line.split_once('=') else { continue };
            let Some(slot) = b.slot_named(name.trim()) else { continue };
            if let Some(v) = b.parse_value(slot, value.trim(), reserved) {
                b.set(slot, v);
            }
        }
        b
    }

    fn parse_value(&self, slot: Slot, text: &str, reserved: &[&str]) -> Option<Value> {
        match self.accepts(slot) {
            Accepts::Axis => Axis::parse(text).map(Value::Axis),
            accepts => {
                let mut v: Vec<Input> = Vec::new();
                for t in text.split_whitespace() {
                    let input = Input::parse(t)?;
                    if matches!(&input, Input::Key(k) if accepts == Accepts::Pad || reserved.contains(&k.as_str())) {
                        return None;
                    }
                    if !v.contains(&input) {
                        v.push(input);
                    }
                }
                Some(Value::Inputs(v))
            }
        }
    }

    pub fn controls(&self) -> &'static Controls {
        self.controls
    }

    /// The changes from the manifest, as stored (empty: none).
    pub fn overrides(&self) -> String {
        let mut out = String::new();
        for ((slot, v), d) in self.slots.iter().zip(&self.defaults) {
            if v != d {
                let t = v.token();
                out += &format!("{} ={}{t}\n", self.slot_name(*slot), if t.is_empty() { "" } else { " " });
            }
        }
        out
    }

    /// Every slot with its bindings, in the order players see them.
    pub fn slots(&self) -> &[(Slot, Value)] {
        &self.slots
    }

    fn index(&self, slot: Slot) -> Option<usize> {
        self.slots.iter().position(|s| s.0 == slot)
    }

    pub fn get(&self, slot: Slot) -> Option<&Value> {
        self.index(slot).map(|i| &self.slots[i].1)
    }

    pub fn default_of(&self, slot: Slot) -> Option<&Value> {
        self.index(slot).map(|i| &self.defaults[i])
    }

    pub fn is_default(&self, slot: Slot) -> bool {
        self.index(slot).is_some_and(|i| self.slots[i].1 == self.defaults[i])
    }

    /// Set a slot's bindings (of the kind it takes); false if it can't be.
    pub fn set(&mut self, slot: Slot, v: Value) -> bool {
        let ok = matches!((&v, self.accepts(slot)), (Value::Axis(_), Accepts::Axis))
            || matches!((&v, self.accepts(slot)), (Value::Inputs(_), Accepts::Inputs | Accepts::Pad));
        match self.index(slot) {
            Some(i) if ok => {
                self.slots[i].1 = v;
                true
            }
            _ => false,
        }
    }

    pub fn reset(&mut self, slot: Slot) {
        if let Some(i) = self.index(slot) {
            self.slots[i].1 = self.defaults[i].clone();
        }
    }

    pub fn reset_all(&mut self) {
        for (s, d) in self.slots.iter_mut().zip(&self.defaults) {
            s.1 = d.clone();
        }
    }

    /// Bind `input` in `slot`: in place of its `at`th binding, or added
    /// after the others (`None`, or past the end). The same input elsewhere
    /// in the slot goes. Errors say why it can't be, for players: a key in
    /// `reserved` (the player's own), or the wrong kind for the slot.
    pub fn assign(&mut self, slot: Slot, at: Option<usize>, input: Input, reserved: &[&str]) -> Result<(), String> {
        if let Input::Key(k) = &input {
            if reserved.contains(&k.as_str()) {
                return Err(format!("{} is the player's own key: pick another", input.text()));
            }
            if !valid_key(k) {
                return Err(format!("{k} can't be bound"));
            }
        }
        match (self.accepts(slot), &input) {
            (Accepts::Axis, _) => return Err("This takes a stick or trigger: move one".into()),
            (Accepts::Pad, Input::Key(_)) => return Err("This takes gamepad buttons only".into()),
            _ => {}
        }
        let Some(i) = self.index(slot) else { return Err("No such input".into()) };
        let Value::Inputs(list) = &mut self.slots[i].1 else { unreachable!() };
        let keep = match at.filter(|&a| a < list.len()) {
            Some(a) => {
                list[a] = input.clone();
                a
            }
            None if list.contains(&input) => return Ok(()),
            None => {
                list.push(input.clone());
                list.len() - 1
            }
        };
        let mut n = 0;
        list.retain(|x| {
            n += 1;
            n - 1 == keep || *x != input
        });
        Ok(())
    }

    /// Bind what the player pressed or moved: a key or pad button into a
    /// list ([`Bindings::assign`]), or a stick or trigger as an analog's
    /// axis. The player moves a stick the way that should read as `+` (an
    /// axis moved towards its negative end binds inverted); a trigger binds
    /// as itself.
    pub fn capture(&mut self, slot: Slot, at: Option<usize>, c: Capture, reserved: &[&str]) -> Result<(), String> {
        if self.accepts(slot) == Accepts::Axis {
            let axis = match c {
                Capture::Axis(axis, negative) if axis != PadAxis::None => Axis { axis, invert: negative },
                Capture::Button(PadButton::LeftTrigger) => Axis { axis: PadAxis::LeftTrigger, invert: false },
                Capture::Button(PadButton::RightTrigger) => Axis { axis: PadAxis::RightTrigger, invert: false },
                Capture::Key(k) if reserved.contains(&k.as_str()) => {
                    return Err(format!("{} is the player's own key", key_symbol(&k)))
                }
                _ => return Err("This takes a stick or trigger: move one (keys go in − and +)".into()),
            };
            self.set(slot, Value::Axis(axis));
            return Ok(());
        }
        match c {
            Capture::Key(k) => self.assign(slot, at, Input::Key(k), reserved),
            Capture::Button(b) => self.assign(slot, at, Input::Pad(b), reserved),
            // A stick pushed a way is that direction (a lever on a stick).
            Capture::Axis(axis, negative) => match PadButton::stick_dir(axis, negative) {
                Some(b) if self.accepts(slot) == Accepts::Inputs => self.assign(slot, at, Input::Pad(b), reserved),
                _ => Err("Sticks go in an analog's axis row: press a key or button".into()),
            },
        }
    }

    /// Invert an analog's axis (or turn inversion off).
    pub fn invert(&mut self, slot: Slot) {
        if let Some(Value::Axis(a)) = self.get(slot).cloned() {
            if a.axis != PadAxis::None {
                self.set(slot, Value::Axis(Axis { invert: !a.invert, ..a }));
            }
        }
    }

    /// Remove a slot's `at`th binding (a list), or its axis.
    pub fn remove(&mut self, slot: Slot, at: usize) {
        if let Some(i) = self.index(slot) {
            match &mut self.slots[i].1 {
                Value::Inputs(list) if at < list.len() => {
                    list.remove(at);
                }
                Value::Axis(a) => *a = Axis::NONE,
                _ => {}
            }
        }
    }

    /// What `slot` takes.
    pub fn accepts(&self, slot: Slot) -> Accepts {
        match slot {
            Slot::Axis(_) => Accepts::Axis,
            Slot::PadFire | Slot::PadReload => Accepts::Pad,
            _ => Accepts::Inputs,
        }
    }

    /// The slot's stored name: the input's name in scripts (`jumpfront`),
    /// an analog's with `.axis`, `-` or `+`, a gun's with `.reload`, and
    /// `pad.fire`, `pad.reload` for the pad's crosshair.
    pub fn slot_name(&self, slot: Slot) -> String {
        let c = self.controls;
        match slot {
            Slot::Button(i) => c.buttons[i].name.to_string(),
            Slot::Axis(i) => format!("{}.axis", c.analogs[i].name),
            Slot::Neg(i) => format!("{}-", c.analogs[i].name),
            Slot::Pos(i) => format!("{}+", c.analogs[i].name),
            Slot::Reload(i) => format!("{}.reload", c.guns[i].name),
            Slot::PadFire => "pad.fire".into(),
            Slot::PadReload => "pad.reload".into(),
        }
    }

    /// The slot called `name` ([`Bindings::slot_name`]), if this game has it.
    pub fn slot_named(&self, name: &str) -> Option<Slot> {
        self.slots.iter().map(|s| s.0).find(|&s| self.slot_name(s) == name)
    }

    /// What players see the slot called.
    pub fn label(&self, slot: Slot) -> String {
        let c = self.controls;
        let player = |i: usize| if c.guns.len() > 1 && i > 0 { format!(" (player {})", i + 1) } else { String::new() };
        match slot {
            Slot::Button(i) => c.buttons[i].label(),
            Slot::Axis(i) => format!("{} (axis)", c.analogs[i].label()),
            Slot::Neg(i) => format!("{} −", c.analogs[i].label()),
            Slot::Pos(i) => format!("{} +", c.analogs[i].label()),
            Slot::Reload(i) => format!("{}{}", c.guns[i].reload_label(c), player(i)),
            Slot::PadFire => "Fire (pad crosshair)".into(),
            Slot::PadReload => format!("{} (pad crosshair)", c.guns[0].reload_label(c)),
        }
    }

    /// Keys and pad buttons bound in more than one slot, other than those
    /// the manifest itself binds so (skisuprg's two ski angles share the
    /// arrows on purpose).
    pub fn conflicts(&self) -> Vec<Conflict> {
        let mut out: Vec<Conflict> = Vec::new();
        for (slot, v) in &self.slots {
            for input in v.inputs() {
                match out.iter_mut().find(|c| c.input == *input) {
                    Some(c) => c.slots.push(*slot),
                    None => out.push(Conflict { input: input.clone(), slots: vec![*slot] }),
                }
            }
        }
        out.retain(|c| {
            c.slots.len() > 1
                && !c.slots.iter().all(|&s| self.default_of(s).is_some_and(|d| d.inputs().contains(&c.input)))
        });
        out
    }

    /// The conflicts `slot` is in.
    pub fn conflicts_of(&self, slot: Slot) -> Vec<Conflict> {
        self.conflicts().into_iter().filter(|c| c.slots.contains(&slot)).collect()
    }

    // For the front ends' input mapping.

    pub fn button(&self, i: usize) -> &[Input] {
        self.get(Slot::Button(i)).map_or(&[], Value::inputs)
    }

    pub fn axis(&self, i: usize) -> Axis {
        self.get(Slot::Axis(i)).map_or(Axis::NONE, Value::axis)
    }

    pub fn neg(&self, i: usize) -> &[Input] {
        self.get(Slot::Neg(i)).map_or(&[], Value::inputs)
    }

    pub fn pos(&self, i: usize) -> &[Input] {
        self.get(Slot::Pos(i)).map_or(&[], Value::inputs)
    }

    /// Gun `i`'s reload keys and buttons (none for a gun that reloads with
    /// a button: that button's own).
    pub fn reload(&self, i: usize) -> &[Input] {
        self.get(Slot::Reload(i)).map_or(&[], Value::inputs)
    }

    pub fn pad_fire(&self) -> &[Input] {
        self.get(Slot::PadFire).map_or(&[], Value::inputs)
    }

    pub fn pad_reload(&self) -> &[Input] {
        self.get(Slot::PadReload).map_or(&[], Value::inputs)
    }

    /// One line per control with its keys and pad bindings (help text,
    /// `make inputs`).
    pub fn describe(&self) -> String {
        let c = self.controls;
        let keys = |v: &[Input]| -> Vec<String> {
            v.iter().filter_map(|i| if let Input::Key(k) = i { Some(key_label(k).to_string()) } else { None }).collect()
        };
        let pads = |v: &[Input]| -> Vec<String> {
            v.iter().filter_map(|i| if let Input::Pad(b) = i { Some(format!("{b:?}")) } else { None }).collect()
        };
        let mut out = String::new();
        for (i, g) in c.guns.iter().enumerate() {
            let aim = if i == 0 { "mouse or touch, else pad sticks" } else { "pad sticks (beside a mouse)" };
            let keys = keys(self.reload(i));
            let reload = match g.reload {
                Reload::Offscreen => "reload: shoot off screen".to_string(),
                Reload::Button(b) => format!("{b}: right button"),
            };
            out += &format!("  {:<10} aim: {aim}; fire: {}; {reload}", g.name, g.trigger);
            out += &if keys.is_empty() { "\n".to_string() } else { format!(" or {}\n", keys.join(" ")) };
        }
        for (i, a) in c.analogs.iter().enumerate() {
            let (neg, pos) = (self.neg(i), self.pos(i));
            let k: Vec<String> = keys(neg).into_iter().chain(keys(pos)).collect();
            let mut pad: Vec<String> = Vec::new();
            let axis = self.axis(i);
            if axis.axis != PadAxis::None {
                pad.push(axis.token());
            }
            pad.extend(pads(neg).into_iter().chain(pads(pos)));
            out += &format!("  {:<10} {} / {}\n", a.name, k.join(" "), pad.join(" "));
        }
        for (i, b) in c.buttons.iter().enumerate() {
            let v = self.button(i);
            let (k, p) = (keys(v), pads(v));
            if !k.is_empty() || !p.is_empty() {
                out += &format!("  {:<10} {} / {}\n", b.name, k.join(" "), p.join(" "));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games;

    fn topskatr() -> Bindings {
        Bindings::defaults(&games::TOPSKATR.controls)
    }

    fn slot(b: &Bindings, name: &str) -> Slot {
        b.slot_named(name).unwrap_or_else(|| panic!("no slot {name}"))
    }

    #[test]
    fn defaults_are_the_manifests() {
        for g in games::ALL {
            let b = Bindings::defaults(&g.controls);
            assert_eq!(b.overrides(), "", "{}", g.id);
            assert!(b.conflicts().is_empty(), "{}: {:?}", g.id, b.conflicts());
            for (i, x) in g.controls.buttons.iter().enumerate() {
                // The manifest's, and the left stick's directions that stand in
                // for its d-pad's.
                let sticks = b.button(i).iter().filter(|i| matches!(i, Input::Pad(p) if p.is_stick_dir())).count();
                let own = x.pad.iter().filter(|p| p.is_stick_dir()).count();
                assert_eq!(b.button(i).len(), x.keys.len() + x.pad.len() + sticks - own, "{}", g.id);
                assert!(x.pad.iter().all(|p| b.button(i).contains(&Input::Pad(*p))), "{}", g.id);
            }
            // Every slot's name finds it again.
            for &(s, _) in b.slots() {
                assert_eq!(b.slot_named(&b.slot_name(s)), Some(s), "{}", g.id);
            }
        }
        let b = topskatr();
        let jump = slot(&b, "jumpfront");
        assert_eq!(
            b.get(jump).unwrap().inputs(),
            [Input::Key("KeyZ".into()), Input::Key("ArrowUp".into()), Input::Pad(PadButton::South)]
        );
        assert_eq!(b.axis(0), Axis { axis: PadAxis::LeftStickX, invert: false });
    }

    #[test]
    fn overrides_round_trip_and_store_only_changes() {
        let mut b = topskatr();
        let jump = slot(&b, "jumpfront");
        b.assign(jump, Some(0), Input::Key("KeyJ".into()), RESERVED_KEYS).unwrap();
        b.assign(slot(&b, "jumptail"), None, Input::Pad(PadButton::North), RESERVED_KEYS).unwrap();
        b.set(slot(&b, "curving.axis"), Value::Axis(Axis { axis: PadAxis::RightStickX, invert: true }));
        b.set(slot(&b, "slide+"), Value::Inputs(Vec::new()));
        let text = b.overrides();
        assert_eq!(
            text,
            "curving.axis = -RightStickX\nslide+ =\njumpfront = KeyJ ArrowUp pad:South\njumptail = KeyX pad:West pad:North\n"
        );
        let c = Bindings::with_overrides(&games::TOPSKATR.controls, &text, RESERVED_KEYS);
        assert_eq!(c.slots(), b.slots());
        assert_eq!(c.overrides(), text);
        // Set back to the manifest's: nothing stored.
        let mut d = c.clone();
        d.reset(jump);
        assert!(!d.overrides().contains("jumpfront"));
        d.reset_all();
        assert_eq!(d.overrides(), "");
    }

    #[test]
    fn bad_stored_data_is_ignored() {
        let text = "garbage\n= x\nnosuch = KeyA\njumpfront = Key-Z\njumptail = pad:Nope\nstart = Escape\n\
                    pad.fire = pad:South\ncurving.axis = Sideways\nslide.axis = KeyA\n# comment\n\
                    selleft = Ké\nselright = KeyR KeyR\n";
        let b = Bindings::with_overrides(&games::TOPSKATR.controls, text, RESERVED_KEYS);
        // Only selright's line was good (its duplicate dropped).
        assert_eq!(b.overrides(), "selright = KeyR\n");
        // pad.fire exists only in gun games; there, keys aren't taken.
        let hotd = &games::HOTD.controls;
        let b = Bindings::with_overrides(hotd, "pad.fire = KeyA\npad.reload = pad:North\n", RESERVED_KEYS);
        assert_eq!(b.overrides(), "pad.reload = pad:North\n");
        // Anything at all, without panicking.
        for s in
            ["", "=", "==", "\u{0}", "a=b=c", "jumpfront", "jumpfront=", "curving.axis=-", "curving.axis=--LeftStickX"]
        {
            let _ = Bindings::with_overrides(&games::TOPSKATR.controls, s, RESERVED_KEYS).overrides();
        }
        let b = Bindings::with_overrides(&games::TOPSKATR.controls, "curving.axis = -None", RESERVED_KEYS);
        assert_eq!(b.axis(0), Axis::NONE);
    }

    #[test]
    fn assign_replaces_adds_and_refuses() {
        let mut b = topskatr();
        let jump = slot(&b, "jumpfront");
        // Adding one it has already moves nothing and adds nothing.
        b.assign(jump, None, Input::Key("KeyZ".into()), RESERVED_KEYS).unwrap();
        assert_eq!(b.get(jump).unwrap().inputs().len(), 3);
        // Replacing the second with the first keeps one of it, in place.
        b.assign(jump, Some(1), Input::Key("KeyZ".into()), RESERVED_KEYS).unwrap();
        assert_eq!(b.get(jump).unwrap().inputs(), [Input::Key("KeyZ".into()), Input::Pad(PadButton::South)]);
        for k in RESERVED_KEYS {
            assert!(b.assign(jump, None, Input::Key(k.to_string()), RESERVED_KEYS).is_err());
        }
        assert!(b.assign(jump, None, Input::Key("KeyF".into()), &["KeyF"]).is_err());
        assert!(b.assign(slot(&b, "curving.axis"), None, Input::Key("KeyA".into()), RESERVED_KEYS).is_err());
        b.remove(jump, 0);
        assert_eq!(b.get(jump).unwrap().inputs(), [Input::Pad(PadButton::South)]);
        b.remove(slot(&b, "curving.axis"), 0);
        assert_eq!(b.axis(0), Axis::NONE);
        let hotd = &games::HOTD.controls;
        let mut g = Bindings::defaults(hotd);
        assert!(g.assign(Slot::PadFire, None, Input::Key("KeyA".into()), RESERVED_KEYS).is_err());
        g.assign(Slot::PadFire, None, Input::Pad(PadButton::North), RESERVED_KEYS).unwrap();
        assert_eq!(g.pad_fire().len(), 3);
    }

    #[test]
    fn conflicts_are_found_but_not_the_manifests_own() {
        // skisuprg binds the arrows to both ski angles: not a conflict.
        let ski = Bindings::defaults(&games::SKISUPRG.controls);
        assert!(ski.conflicts().is_empty());
        let mut b = topskatr();
        let tail = slot(&b, "jumptail");
        b.assign(tail, None, Input::Key("KeyZ".into()), RESERVED_KEYS).unwrap();
        let c = b.conflicts();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].input, Input::Key("KeyZ".into()));
        assert_eq!(c[0].slots, [slot(&b, "jumpfront"), tail]);
        assert_eq!(b.conflicts_of(tail).len(), 1);
        assert!(b.conflicts_of(slot(&b, "start")).is_empty());
    }

    #[test]
    fn gun_games_have_their_slots() {
        let b = Bindings::defaults(&games::HOTD.controls);
        let names: Vec<String> = b.slots().iter().take(6).map(|s| b.slot_name(s.0)).collect();
        assert_eq!(names, ["trigger1", "gun1.reload", "trigger2", "gun2.reload", "pad.fire", "pad.reload"]);
        assert_eq!(b.reload(0), [Input::Key("KeyR".into())]);
        assert_eq!(b.label(Slot::Reload(1)), "Reload (player 2)");
        // bel reloads with its missile buttons: no reload slots.
        let bel = Bindings::defaults(&games::BEL.controls);
        assert!(bel.slot_named("gun1.reload").is_none());
        assert_eq!(bel.label(Slot::PadReload), "Missile (pad crosshair)");
    }

    #[test]
    fn captures_bind_by_kind() {
        let mut b = topskatr();
        let axis = slot(&b, "curving.axis");
        let jump = slot(&b, "jumpfront");
        b.capture(axis, None, Capture::Axis(PadAxis::RightStickY, true), RESERVED_KEYS).unwrap();
        assert_eq!(b.axis(0), Axis { axis: PadAxis::RightStickY, invert: true });
        b.invert(axis);
        assert_eq!(b.axis(0), Axis { axis: PadAxis::RightStickY, invert: false });
        b.capture(axis, None, Capture::Button(PadButton::LeftTrigger), RESERVED_KEYS).unwrap();
        assert_eq!(b.axis(0).axis, PadAxis::LeftTrigger);
        assert!(b.capture(axis, None, Capture::Key("KeyA".into()), RESERVED_KEYS).is_err());
        assert!(b.capture(axis, None, Capture::Button(PadButton::South), RESERVED_KEYS).is_err());
        // A trigger isn't a direction (a stick pushed a way is: see
        // `stick_bindings_are_stored_by_name_and_old_files_load`).
        assert!(b.capture(jump, None, Capture::Axis(PadAxis::LeftTrigger, false), RESERVED_KEYS).is_err());
        assert!(b.capture(jump, None, Capture::Key("F11".into()), RESERVED_KEYS).is_err());
        b.capture(jump, Some(2), Capture::Button(PadButton::North), RESERVED_KEYS).unwrap();
        assert_eq!(b.button(jump_index()).last(), Some(&Input::Pad(PadButton::North)));
        assert_eq!(b.overrides(), "curving.axis = LeftTrigger\njumpfront = KeyZ ArrowUp pad:North\n");
    }

    fn von() -> Bindings {
        Bindings::defaults(&games::VON.controls)
    }

    #[test]
    fn a_digital_games_dpad_is_the_left_stick_too() {
        let b = Bindings::defaults(&games::VF2.controls);
        let left = slot(&b, "left");
        assert_eq!(
            b.get(left).unwrap().inputs(),
            [Input::Key("ArrowLeft".into()), Input::Pad(PadButton::DPadLeft), Input::Pad(PadButton::LeftStickLeft)]
        );
        // Not one that steers with the stick's axis, or aims a gun with it.
        for g in games::ALL {
            let steers_x = g.controls.analogs.iter().any(|a| a.axis == PadAxis::LeftStickX);
            let steers_y = g.controls.analogs.iter().any(|a| a.axis == PadAxis::LeftStickY);
            let b = Bindings::defaults(&g.controls);
            for (i, x) in g.controls.buttons.iter().enumerate() {
                for &(p, blocked) in &[
                    (PadButton::LeftStickLeft, steers_x),
                    (PadButton::LeftStickRight, steers_x),
                    (PadButton::LeftStickUp, steers_y),
                    (PadButton::LeftStickDown, steers_y),
                ] {
                    let bound = b.button(i).contains(&Input::Pad(p));
                    if (blocked || !g.controls.guns.is_empty()) && !x.pad.contains(&p) {
                        assert!(!bound, "{}: {} reads {p:?}", g.id, x.name);
                    }
                }
            }
        }
    }

    #[test]
    fn von_has_a_stick_for_each_lever() {
        let b = von();
        let pad = |name: &str| {
            b.get(slot(&b, name))
                .unwrap()
                .inputs()
                .iter()
                .filter_map(|i| match i {
                    Input::Pad(p) => Some(*p),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(pad("lup"), [PadButton::DPadUp, PadButton::LeftStickUp]);
        assert_eq!(pad("rleft"), [PadButton::RightStickLeft]);
        assert_eq!(pad("rshot"), [PadButton::RightShoulder, PadButton::East]);
        assert!(b.conflicts().is_empty());
    }

    #[test]
    fn stick_bindings_are_stored_by_name_and_old_files_load() {
        let mut b = von();
        let rup = slot(&b, "rup");
        b.capture(rup, None, Capture::Axis(PadAxis::LeftStickY, true), RESERVED_KEYS).unwrap();
        assert_eq!(
            b.get(rup).unwrap().inputs(),
            [Input::Key("ArrowUp".into()), Input::Pad(PadButton::RightStickUp), Input::Pad(PadButton::LeftStickUp)]
        );
        let text = b.overrides();
        assert_eq!(text, "rup = ArrowUp pad:RightStickUp pad:LeftStickUp\n");
        assert_eq!(Bindings::with_overrides(&games::VON.controls, &text, RESERVED_KEYS).slots(), b.slots());
        // A file from before the stick directions: the d-pad and buttons
        // it names, and a name this version doesn't know, as before.
        let old = "lup = KeyW pad:DPadUp\nrup = ArrowUp pad:North\nlshot = pad:Nonsense\n";
        let c = Bindings::with_overrides(&games::VON.controls, old, RESERVED_KEYS);
        assert_eq!(
            c.get(slot(&c, "lup")).unwrap().inputs(),
            [Input::Key("KeyW".into()), Input::Pad(PadButton::DPadUp)]
        );
        assert_eq!(
            c.get(slot(&c, "rup")).unwrap().inputs(),
            [Input::Key("ArrowUp".into()), Input::Pad(PadButton::North)]
        );
        assert_eq!(c.get(slot(&c, "lshot")).unwrap(), von().get(slot(&c, "lshot")).unwrap());
    }

    fn jump_index() -> usize {
        games::TOPSKATR.controls.buttons.iter().position(|b| b.name == "jumpfront").unwrap()
    }

    #[test]
    fn axes_read_with_invert() {
        let a = Axis { axis: PadAxis::LeftStickX, invert: true };
        assert_eq!(a.read(|x| if x == PadAxis::LeftStickX { 0.5 } else { 0.0 }), -0.5);
        assert_eq!(Axis::NONE.read(|_| 1.0), 0.0);
        assert_eq!(Axis::parse(&a.token()), Some(a));
        assert_eq!(Axis::parse("None"), Some(Axis::NONE));
    }
}
