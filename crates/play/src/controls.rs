//! The keyboard and gamepad as a [`model2::input::PlayerInput`]: the
//! game's manifest names its bindings as W3C key codes and standard-pad
//! buttons and axes, mapped here onto winit and gilrs (and back, for
//! capturing what a player presses to remap a control).

use gilrs::{Axis, Button};
use romset::bindings::Capture;
use romset::{PadAxis, PadButton};
use std::collections::HashSet;
use winit::keyboard::KeyCode;

/// Stick deflections smaller than this read as centred.
const DEAD_ZONE: f32 = 0.15;

pub struct Devices<'a> {
    pub keys: &'a HashSet<KeyCode>,
    pub pad: Option<gilrs::Gamepad<'a>>,
}

impl model2::input::PlayerInput for Devices<'_> {
    fn key(&self, code: &str) -> bool {
        key_code(code).is_some_and(|k| self.keys.contains(&k))
    }

    fn pad_button(&self, b: PadButton) -> bool {
        self.pad.as_ref().is_some_and(|p| p.is_pressed(button(b)))
    }

    fn pad_axis(&self, a: PadAxis) -> f32 {
        let Some(p) = &self.pad else { return 0.0 };
        let stick = |axis: Axis, sign: f32| {
            let v = (p.value(axis) * sign).clamp(-1.0, 1.0);
            if v.abs() > DEAD_ZONE {
                v
            } else {
                0.0
            }
        };
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
pub fn captured(ev: &gilrs::EventType) -> Option<Capture> {
    match *ev {
        gilrs::EventType::ButtonPressed(b, _) => pad_button(b).map(Capture::Button),
        gilrs::EventType::AxisChanged(a, v, _) if v.abs() > 0.6 => {
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
