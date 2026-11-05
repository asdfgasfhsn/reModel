//! The keyboard and gamepad as a [`model2::input::PlayerInput`]: the
//! game's manifest names its bindings as W3C key codes and standard-pad
//! buttons and axes, mapped here onto winit and gilrs.

use gilrs::{Axis, Button};
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

/// winit's key for a `KeyboardEvent.code` (winit uses the same names).
pub fn key_code(code: &str) -> Option<KeyCode> {
    use KeyCode::*;
    macro_rules! keys {
        ($($k:ident)*) => {
            match code {
                $(stringify!($k) => Some($k),)*
                _ => None,
            }
        };
    }
    keys!(
        KeyA KeyB KeyC KeyD KeyE KeyF KeyG KeyH KeyI KeyJ KeyK KeyL KeyM KeyN KeyO KeyP KeyQ KeyR KeyS KeyT KeyU
        KeyV KeyW KeyX KeyY KeyZ Digit0 Digit1 Digit2 Digit3 Digit4 Digit5 Digit6 Digit7 Digit8 Digit9
        ArrowLeft ArrowRight ArrowUp ArrowDown Enter Space Tab Backspace ShiftLeft ShiftRight ControlLeft
        ControlRight AltLeft AltRight Comma Period Slash Semicolon Quote BracketLeft BracketRight Minus Equal
    )
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
}
