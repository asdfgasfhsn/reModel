//! What a game's cabinet controls are and where the board reads them, with
//! default bindings for players. Each game's manifest holds one
//! [`Controls`]; the board model, the scripted `--input` syntax and the
//! players all read it, so nothing outside `games/` names a game's inputs.
//!
//! Keys are named by their W3C `KeyboardEvent.code` (`"KeyZ"`,
//! `"ArrowLeft"`, `"Enter"`, ...), which the browser build uses as is and the
//! native player maps to its own key codes. Pad bindings use the W3C
//! "standard" gamepad layout ([`PadButton`], [`PadAxis`]).
//!
//! Light guns (House of the Dead) will need a third kind of control, a
//! pointer read through its own I/O (not the analog channels); it would be
//! a new list here (say `guns: &[Gun]`, each with its port, trigger button
//! and calibration) that players bind to the mouse or a touch.

/// A game's controls.
pub struct Controls {
    /// Digital inputs: switches read active low on the I/O chip's ports.
    pub buttons: &'static [ButtonInput],
    /// Analog inputs: channels of the I/O chip's ADC.
    pub analogs: &'static [AnalogInput],
    /// Coin up and start a game, in the `--input` script syntax: what
    /// `coin-start` in a script stands for (`make capture`, `make bench`).
    pub coin_start: &'static str,
}

/// A digital input.
pub struct ButtonInput {
    /// Name in input scripts (`name@from-to`).
    pub name: &'static str,
    /// Port: index of the I/O register that reads it (0 = `0x1C00002`,
    /// 1 = `0x1C00004`, 2 = `0x1C00006`, 3 = `0x1C0000C`).
    pub port: u8,
    /// Its bit in the port (a mask); pressed reads 0.
    pub bit: u8,
    /// Default keys (`KeyboardEvent.code`).
    pub keys: &'static [&'static str],
    /// Default pad buttons.
    pub pad: &'static [PadButton],
}

/// A [`ButtonInput`], for manifests.
pub(crate) const fn button(
    name: &'static str,
    port: u8,
    bit: u8,
    keys: &'static [&'static str],
    pad: &'static [PadButton],
) -> ButtonInput {
    ButtonInput { name, port, bit, keys, pad }
}

/// An analog input: one ADC channel, driven by a player axis.
pub struct AnalogInput {
    /// Name in input scripts (`name=value@from-to`).
    pub name: &'static str,
    pub channel: u8,
    /// Value at rest.
    pub centre: u8,
    /// Change from `centre` at full positive deflection of the bound axis
    /// (stick right or down, trigger fully pressed); negative deflection
    /// goes the other way, clamped to 0..=255.
    pub span: i16,
    /// The pad axis that drives it.
    pub axis: PadAxis,
    /// Keys and pad buttons that hold it at full negative and full positive
    /// deflection (overriding the axis while held).
    pub keys_neg: &'static [&'static str],
    pub keys_pos: &'static [&'static str],
    pub pad_neg: &'static [PadButton],
    pub pad_pos: &'static [PadButton],
}

impl AnalogInput {
    /// The channel value for an axis deflection (-1 to 1; 0 to 1 for a
    /// trigger).
    pub fn value(&self, deflection: f32) -> u8 {
        (self.centre as f32 + deflection.clamp(-1.0, 1.0) * self.span as f32).round().clamp(0.0, 255.0) as u8
    }
}

/// Buttons of the W3C standard gamepad, in its index order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadButton {
    /// A on Xbox pads, cross on PlayStation.
    South,
    East,
    West,
    North,
    LeftShoulder,
    RightShoulder,
    LeftTrigger,
    RightTrigger,
    Select,
    Start,
    LeftStick,
    RightStick,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
}

impl PadButton {
    /// Index in the W3C standard gamepad's `buttons`.
    pub fn index(self) -> u32 {
        self as u32
    }
}

/// Pad axes an analog input can follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadAxis {
    /// Not bound to an axis (keys and buttons only).
    None,
    /// -1 left to 1 right.
    LeftStickX,
    /// -1 up to 1 down.
    LeftStickY,
    RightStickX,
    RightStickY,
    /// 0 released to 1 pressed.
    LeftTrigger,
    RightTrigger,
    /// Right trigger minus left trigger: -1 to 1.
    Triggers,
}

impl Controls {
    pub fn button(&self, name: &str) -> Option<&ButtonInput> {
        self.buttons.iter().find(|b| b.name == name)
    }

    pub fn analog(&self, name: &str) -> Option<&AnalogInput> {
        self.analogs.iter().find(|a| a.name == name)
    }

    /// All input names, for messages.
    pub fn names(&self) -> String {
        self.buttons.iter().map(|b| b.name).chain(self.analogs.iter().map(|a| a.name)).collect::<Vec<_>>().join(", ")
    }

    /// One line per control with its default keys and pad bindings (for
    /// help text and docs).
    pub fn describe(&self) -> String {
        let mut out = String::new();
        for a in self.analogs {
            let keys: Vec<_> = a.keys_neg.iter().chain(a.keys_pos).map(|k| key_label(k)).collect();
            let mut pad: Vec<String> = Vec::new();
            if a.axis != PadAxis::None {
                pad.push(format!("{:?}", a.axis));
            }
            pad.extend(a.pad_neg.iter().chain(a.pad_pos).map(|b| format!("{b:?}")));
            out += &format!("  {:<10} {} / {}\n", a.name, keys.join(" "), pad.join(" "));
        }
        for b in self.buttons {
            let keys: Vec<_> = b.keys.iter().map(|k| key_label(k)).collect();
            let pad: Vec<_> = b.pad.iter().map(|p| format!("{p:?}")).collect();
            if !keys.is_empty() || !pad.is_empty() {
                out += &format!("  {:<10} {} / {}\n", b.name, keys.join(" "), pad.join(" "));
            }
        }
        out
    }
}

/// Short label for a `KeyboardEvent.code`.
fn key_label(code: &str) -> &str {
    code.strip_prefix("Key").or_else(|| code.strip_prefix("Digit")).unwrap_or(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_game_has_consistent_controls() {
        for g in crate::games::ALL {
            let c = &g.controls;
            let mut names: Vec<_> = c.buttons.iter().map(|b| b.name).chain(c.analogs.iter().map(|a| a.name)).collect();
            names.sort();
            let n = names.len();
            names.dedup();
            assert_eq!(names.len(), n, "{}: duplicate input names", g.id);
            for b in c.buttons {
                assert!(b.port < 4 && b.bit.count_ones() == 1, "{}: {}", g.id, b.name);
            }
            for a in c.analogs {
                assert!(a.channel < 8, "{}: {}", g.id, a.name);
            }
            assert!(c.button("coin1").is_some(), "{}", g.id);
            assert!(c.coin_start.contains("coin1@"), "{}", g.id);
        }
    }

    #[test]
    fn analog_values_follow_span_and_clamp() {
        let a = AnalogInput {
            name: "x",
            channel: 0,
            centre: 0x80,
            span: -112,
            axis: PadAxis::LeftStickX,
            keys_neg: &[],
            keys_pos: &[],
            pad_neg: &[],
            pad_pos: &[],
        };
        assert_eq!(a.value(0.0), 0x80);
        assert_eq!(a.value(1.0), 0x10);
        assert_eq!(a.value(-1.0), 0xf0);
        assert_eq!(a.value(-3.0), 0xf0);
    }
}
