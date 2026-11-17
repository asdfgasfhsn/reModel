//! The keyboard and gamepad as a [`model2::input::PlayerInput`]: the
//! game's manifest names its bindings as W3C key codes and standard-pad
//! buttons and axes, mapped here onto winit and gilrs (and back, for
//! capturing what a player presses to remap a control).

use gilrs::{Axis, Button};
use romset::bindings::Capture;
use romset::{PadAxis, PadButton, StickDirs};
use std::collections::HashSet;
use winit::keyboard::KeyCode;

/// How a deflection reads: its curve, and a dead zone and sensitivity in
/// percent. The player's tuning of the analog inputs (Controls page).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Curve {
    /// What the stick does, one to one.
    #[default]
    Linear,
    /// Gentle: small movements count for less (finer control near the middle).
    Gentle,
    /// Quick: small movements count for more.
    Quick,
}

impl Curve {
    pub const ALL: [Curve; 3] = [Curve::Linear, Curve::Gentle, Curve::Quick];

    pub fn name(self) -> &'static str {
        match self {
            Curve::Linear => "linear",
            Curve::Gentle => "gentle",
            Curve::Quick => "quick",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Curve::Linear => "Linear",
            Curve::Gentle => "Gentle",
            Curve::Quick => "Quick",
        }
    }

    pub fn parse(s: &str) -> Option<Curve> {
        Curve::ALL.into_iter().find(|c| c.name() == s)
    }

    /// The curve's shape on 0 to 1.
    fn shape(self, u: f32) -> f32 {
        match self {
            Curve::Linear => u,
            Curve::Gentle => u * u,
            Curve::Quick => u.sqrt(),
        }
    }
}

/// Tuning of one kind of analog input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Response {
    /// Deflections up to this (percent of full) read as none.
    pub dead: u8,
    /// Percent of full deflection the largest deflection reaches (over 100
    /// reaches full before the end of the travel).
    pub sens: u8,
    pub curve: Curve,
}

impl Response {
    /// The most the dead zone can be, and the range of the sensitivity.
    pub const MAX_DEAD: u8 = 40;
    pub const SENS: std::ops::RangeInclusive<u8> = 50..=150;

    /// What sticks have always had: a 15% dead zone, nothing else.
    pub const STICK: Response = Response { dead: 15, sens: 100, curve: Curve::Linear };
    /// Triggers and pedals: read as they are.
    pub const PEDAL: Response = Response { dead: 0, sens: 100, curve: Curve::Linear };

    /// The reading for a deflection `v` (-1 to 1, or 0 to 1): none inside
    /// the dead zone, beyond it the curve over what's left of the travel
    /// (the reading steps from none to the dead zone's size at its edge, as
    /// a stick's always has; a curve other than Linear bends what comes
    /// after) and the sensitivity. The defaults give `v` itself.
    pub fn apply(&self, v: f32) -> f32 {
        let dead = self.dead as f32 / 100.0;
        let a = v.abs();
        if a <= dead {
            return 0.0;
        }
        let shaped = match self.curve {
            Curve::Linear => a,
            c => dead + (1.0 - dead) * c.shape(((a - dead) / (1.0 - dead)).clamp(0.0, 1.0)),
        };
        (shaped * (self.sens as f32 / 100.0)).min(1.0).copysign(v)
    }
}

/// The player's tuning of the sticks and of the triggers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tuning {
    pub stick: Response,
    pub pedal: Response,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning { stick: Response::STICK, pedal: Response::PEDAL }
    }
}

/// A gamepad that looks like a PlayStation one (by name or vendor) is
/// drawn with its sticks side by side, as its own are; any other as an
/// Xbox-style pad, the sticks offset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PadStyle {
    #[default]
    Offset,
    SideBySide,
}

/// Sony's USB vendor id.
const SONY: u16 = 0x054c;

pub fn pad_style(name: &str, vendor: Option<u16>) -> PadStyle {
    let n = name.to_ascii_lowercase();
    let sony = vendor == Some(SONY)
        || n == "wireless controller"
        || ["playstation", "dualshock", "dualsense", "ps3", "ps4", "ps5", "sony"].iter().any(|k| n.contains(k));
    if sony {
        PadStyle::SideBySide
    } else {
        PadStyle::Offset
    }
}

/// What the pad reads now, for the Controls page's live meters and its
/// picture of the pad.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PadLive {
    pub connected: bool,
    pub style: PadStyle,
    /// Each axis' raw deflection (before tuning), in `PadAxis::ALL` order.
    pub axes: [f32; 8],
}

impl PadLive {
    /// A pad as gilrs has it (`None`: none connected).
    pub fn of(pad: Option<&gilrs::Gamepad>) -> PadLive {
        let Some(p) = pad else { return PadLive::default() };
        let mut axes = [0.0; 8];
        for (v, a) in axes.iter_mut().zip(PadAxis::ALL) {
            *v = raw_axis(p, a);
        }
        PadLive { connected: true, style: pad_style(p.name(), p.vendor_id()), axes }
    }

    pub fn axis(&self, a: PadAxis) -> f32 {
        PadAxis::ALL.iter().position(|x| *x == a).map_or(0.0, |i| self.axes[i])
    }
}

/// A pad axis as the manifest reads it, before any tuning.
pub fn raw_axis(p: &gilrs::Gamepad, a: PadAxis) -> f32 {
    let stick = |axis: Axis, sign: f32| (p.value(axis) * sign).clamp(-1.0, 1.0);
    let trigger = |b: Button| p.button_data(b).map_or(0.0, |d| d.value());
    match a {
        PadAxis::None => 0.0,
        PadAxis::LeftStickX => stick(Axis::LeftStickX, 1.0),
        // gilrs reads up as positive; the manifest's down is.
        PadAxis::LeftStickY => stick(Axis::LeftStickY, -1.0),
        PadAxis::RightStickX => stick(Axis::RightStickX, 1.0),
        PadAxis::RightStickY => stick(Axis::RightStickY, -1.0),
        PadAxis::LeftTrigger => trigger(Button::LeftTrigger2),
        PadAxis::RightTrigger => trigger(Button::RightTrigger2),
        PadAxis::Triggers => trigger(Button::RightTrigger2) - trigger(Button::LeftTrigger2),
    }
}

/// Whether `a` is a stick's axis (else a trigger's, or none).
pub fn is_stick(a: PadAxis) -> bool {
    matches!(a, PadAxis::LeftStickX | PadAxis::LeftStickY | PadAxis::RightStickX | PadAxis::RightStickY)
}

pub struct Devices<'a> {
    pub keys: &'a HashSet<KeyCode>,
    pub pad: Option<gilrs::Gamepad<'a>>,
    /// Which of the pad's buttons the game sees (not the menu's).
    pub gate: &'a crate::pad::Gate,
    /// The player's tuning of the sticks and triggers.
    pub tuning: Tuning,
    /// What the sticks hold as directions ([`read_sticks`]).
    pub dirs: StickDirs,
}

/// Move `dirs` by one frame of `pad`'s sticks, past the player's dead zone.
pub fn read_sticks(dirs: &mut StickDirs, pad: Option<&gilrs::Gamepad>, tuning: Tuning) -> StickDirs {
    let Some(p) = pad else { return dirs.update((0.0, 0.0), (0.0, 0.0), 0.0) };
    let xy = |x, y| (raw_axis(p, x), raw_axis(p, y));
    let (l, r) = (xy(PadAxis::LeftStickX, PadAxis::LeftStickY), xy(PadAxis::RightStickX, PadAxis::RightStickY));
    dirs.update(l, r, tuning.stick.dead as f32 / 100.0)
}

impl model2::input::PlayerInput for Devices<'_> {
    fn key(&self, code: &str) -> bool {
        key_code(code).is_some_and(|k| self.keys.contains(&k))
    }

    fn pad_button(&self, b: PadButton) -> bool {
        if b.is_stick_dir() {
            return self.dirs.held(b);
        }
        self.pad.as_ref().is_some_and(|p| self.gate.passes(button(b), p.is_pressed(button(b))))
    }

    fn pad_axis(&self, a: PadAxis) -> f32 {
        let Some(p) = &self.pad else { return 0.0 };
        let tune = if is_stick(a) { self.tuning.stick } else { self.tuning.pedal };
        match a {
            PadAxis::None => 0.0,
            a => tune.apply(raw_axis(p, a)),
        }
    }
}

/// gilrs names the shoulder buttons triggers, and the triggers trigger 2.
fn button(b: PadButton) -> Button {
    match b {
        PadButton::South => Button::South,
        PadButton::East => Button::East,
        PadButton::West => Button::West,
        PadButton::North => Button::North,
        PadButton::LeftShoulder => Button::LeftTrigger,
        PadButton::RightShoulder => Button::RightTrigger,
        PadButton::LeftTrigger => Button::LeftTrigger2,
        PadButton::RightTrigger => Button::RightTrigger2,
        PadButton::Select => Button::Select,
        PadButton::Start => Button::Start,
        PadButton::LeftStick => Button::LeftThumb,
        PadButton::RightStick => Button::RightThumb,
        PadButton::DPadUp => Button::DPadUp,
        PadButton::DPadDown => Button::DPadDown,
        PadButton::DPadLeft => Button::DPadLeft,
        PadButton::DPadRight => Button::DPadRight,
        // Not buttons: read from the axes (`StickDirs`).
        _ => Button::Unknown,
    }
}

/// The keys a game can be bound to (and the player's own, so capturing
/// one can say it's taken): winit names them as `KeyboardEvent.code` does.
macro_rules! keys {
    ($($k:ident)*) => {
        /// winit's key for a `KeyboardEvent.code`.
        pub fn key_code(code: &str) -> Option<KeyCode> {
            match code {
                $(stringify!($k) => Some(KeyCode::$k),)*
                _ => None,
            }
        }

        /// The `KeyboardEvent.code` of a winit key, if it's one of these.
        pub fn code_name(key: KeyCode) -> Option<&'static str> {
            match key {
                $(KeyCode::$k => Some(stringify!($k)),)*
                _ => None,
            }
        }
    };
}

keys!(
    KeyA KeyB KeyC KeyD KeyE KeyF KeyG KeyH KeyI KeyJ KeyK KeyL KeyM KeyN KeyO KeyP KeyQ KeyR KeyS KeyT KeyU
    KeyV KeyW KeyX KeyY KeyZ Digit0 Digit1 Digit2 Digit3 Digit4 Digit5 Digit6 Digit7 Digit8 Digit9
    ArrowLeft ArrowRight ArrowUp ArrowDown Enter Space Tab Backspace ShiftLeft ShiftRight ControlLeft
    ControlRight AltLeft AltRight Comma Period Slash Semicolon Quote BracketLeft BracketRight Minus Equal
    Backquote Backslash CapsLock Insert Delete Home End PageUp PageDown Escape
    F1 F2 F3 F4 F5 F6 F7 F8 F9 F10 F11 F12
    Numpad0 Numpad1 Numpad2 Numpad3 Numpad4 Numpad5 Numpad6 Numpad7 Numpad8 Numpad9
    NumpadAdd NumpadSubtract NumpadMultiply NumpadDivide NumpadDecimal NumpadEnter
);

/// What a gamepad event gives when capturing a binding: a button pressed,
/// or a stick moved well off centre (in the manifest's sense: down is
/// positive).
pub fn captured(ev: crate::pad::Event) -> Option<Capture> {
    use crate::pad::Event;
    match ev {
        Event::Pressed(b) => pad_button(b).map(Capture::Button),
        Event::Axis(a, v) if v.abs() > 0.6 => {
            let (axis, v) = match a {
                Axis::LeftStickX => (PadAxis::LeftStickX, v),
                Axis::LeftStickY => (PadAxis::LeftStickY, -v),
                Axis::RightStickX => (PadAxis::RightStickX, v),
                Axis::RightStickY => (PadAxis::RightStickY, -v),
                _ => return None,
            };
            Some(Capture::Axis(axis, v < 0.0))
        }
        _ => None,
    }
}

/// The standard-pad button gilrs reports as `b`.
fn pad_button(b: Button) -> Option<PadButton> {
    PadButton::ALL.into_iter().find(|&p| button(p) == b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bound_key_maps() {
        for g in romset::games::ALL {
            let c = &g.controls;
            let keys = c
                .buttons
                .iter()
                .flat_map(|b| b.keys)
                .chain(c.analogs.iter().flat_map(|a| a.keys_neg.iter().chain(a.keys_pos)));
            for k in keys {
                assert!(key_code(k).is_some(), "{}: key {k}", g.id);
            }
        }
    }

    /// Remapping from the pad: buttons and sticks bind; Guide isn't a
    /// game's (it cancels instead).
    #[test]
    fn pad_events_capture() {
        use crate::pad::Event;
        assert_eq!(captured(Event::Pressed(Button::South)), Some(Capture::Button(PadButton::South)));
        assert_eq!(captured(Event::Pressed(Button::Start)), Some(Capture::Button(PadButton::Start)));
        assert_eq!(captured(Event::Pressed(Button::Mode)), None);
        assert_eq!(captured(Event::Released(Button::South)), None);
        // gilrs reads up as positive: up is the manifest's negative.
        assert_eq!(captured(Event::Axis(Axis::LeftStickY, 0.9)), Some(Capture::Axis(PadAxis::LeftStickY, true)));
        assert_eq!(captured(Event::Axis(Axis::LeftStickX, 0.3)), None);
    }

    /// The defaults read exactly as the old fixed dead zone did.
    #[test]
    fn default_tuning_is_the_old_reading() {
        let old = |v: f32| if v.abs() > 0.15 { v } else { 0.0 };
        for i in -1000..=1000 {
            let v = i as f32 / 1000.0;
            assert_eq!(Response::STICK.apply(v), old(v), "{v}");
            assert_eq!(Response::PEDAL.apply(v.abs()), v.abs(), "{v}");
        }
    }

    #[test]
    fn tuning_curves() {
        let r = |dead, sens, curve| Response { dead, sens, curve };
        assert_eq!(r(20, 100, Curve::Linear).apply(0.2), 0.0);
        assert_eq!(r(20, 100, Curve::Linear).apply(-0.19), 0.0);
        assert_eq!(r(20, 100, Curve::Linear).apply(-0.5), -0.5);
        assert!((r(0, 50, Curve::Linear).apply(0.8) - 0.4).abs() < 1e-6);
        assert_eq!(r(0, 150, Curve::Linear).apply(0.9), 1.0);
        assert_eq!(r(0, 150, Curve::Linear).apply(-0.9), -1.0);
        let (g, q) = (r(0, 100, Curve::Gentle), r(0, 100, Curve::Quick));
        assert!(g.apply(0.5) < 0.5 && q.apply(0.5) > 0.5);
        assert!((g.apply(1.0) - 1.0).abs() < 1e-6 && (q.apply(1.0) - 1.0).abs() < 1e-6);
        let g = r(15, 100, Curve::Gentle);
        let mut last = 0.0;
        for i in 0..=100 {
            let y = g.apply(i as f32 / 100.0);
            assert!(y >= last, "{i}");
            last = y;
        }
    }

    #[test]
    fn pads_are_told_apart_by_name_and_vendor() {
        assert_eq!(pad_style("Xbox Wireless Controller", Some(0x045e)), PadStyle::Offset);
        assert_eq!(pad_style("DualSense Wireless Controller", None), PadStyle::SideBySide);
        assert_eq!(pad_style("Some Pad", Some(0x054c)), PadStyle::SideBySide);
        assert_eq!(pad_style("PS4 Controller", None), PadStyle::SideBySide);
        assert_eq!(pad_style("8BitDo Pro 2", Some(0x2dc8)), PadStyle::Offset);
    }

    #[test]
    fn keys_and_buttons_map_both_ways() {
        for k in ["KeyZ", "ArrowLeft", "Numpad5", "F5", "Escape"] {
            assert_eq!(key_code(k).and_then(code_name), Some(k));
        }
        for k in romset::bindings::RESERVED_KEYS {
            assert!(key_code(k).is_some(), "{k}");
        }
        for b in PadButton::ALL {
            assert_eq!(pad_button(button(b)), Some(b));
        }
    }
}
