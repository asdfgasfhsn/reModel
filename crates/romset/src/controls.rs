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
//! Light guns are a third kind of control ([`Gun`]): a position in the
//! game's view per player, from any pointer (mouse, touch, a stick, a
//! script), which the manifest maps to where and how the board reads it.

/// A game's controls.
pub struct Controls {
    /// Digital inputs: switches read active low on the I/O chip's ports.
    pub buttons: &'static [ButtonInput],
    /// Analog inputs: channels of the I/O chip's ADC.
    pub analogs: &'static [AnalogInput],
    /// Light guns (pointers): where the board reads each player's aim.
    pub guns: &'static [Gun],
    /// Coin up and start a game, in the `--input` script syntax: what
    /// `coin-start` in a script stands for (`make capture`, `make bench`).
    pub coin_start: &'static str,
}

/// A light gun: one player's aim and its buttons. Players and scripts give
/// the aim in view coordinates (0 to 1 across and down the game's 496x384
/// view, or off screen); the manifest says where the board reads it and
/// what raw values the view's edges give, so any pointer (mouse, touch,
/// stick, a tracked hand) can drive it.
pub struct Gun {
    /// Name in input scripts: `gun1=x:y@from-to` with x and y in view
    /// pixels (0-495, 0-383), or `gun1=off@from-to`.
    pub name: &'static str,
    /// Where the board reads the aim.
    pub source: GunSource,
    /// Raw X at the view's left and right edges, and raw Y at its top and
    /// bottom: the calibration a cabinet's gun setting would hold. Values
    /// between follow linearly; either end may be the larger. An end may
    /// lie beyond what the board can read (raw values clamp to it) when the
    /// game's aim can't reach the view's edge.
    pub x_range: (i16, i16),
    pub y_range: (i16, i16),
    /// The button input that is this gun's trigger (fires where it aims).
    pub trigger: &'static str,
    /// How the player reloads.
    pub reload: Reload,
    /// Keys that reload (the trigger's keys are the trigger button's).
    pub keys_reload: &'static [&'static str],
}

/// Where a board reads a gun's position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GunSource {
    /// Model 2C's gun interface (`model2::gun`): X and Y are counter
    /// pairs (a low byte, then the next counter's two high bits), and
    /// `offscreen_bit` of counter 8 reads 0 while the gun points at the
    /// screen and 1 when it sees nothing.
    Counters { x: u8, y: u8, offscreen_bit: u8 },
    /// A mounted gun on two ADC channels (yaw and pitch potentiometers,
    /// bel), read like any analog input. It can't point off the screen.
    Analog { x: u8, y: u8 },
}

/// How a gun game reloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reload {
    /// Pull the trigger aimed off the screen. Players that can't point
    /// off screen offer a reload button that aims away for a moment.
    Offscreen,
    /// No reloading: the players' reload controls (right mouse button, the
    /// pad's reload buttons, the touch pill) press this button instead
    /// (bel's missile button).
    Button(&'static str),
}

impl Gun {
    /// What players see the reload control called: "Reload", or the
    /// label of the button that stands in for it.
    pub fn reload_label(&self, controls: &Controls) -> String {
        match self.reload {
            Reload::Offscreen => "Reload".to_string(),
            Reload::Button(b) => controls.button(b).map_or_else(|| capitalise(b), ButtonInput::label),
        }
    }

    /// The raw X and Y for an aim at `(x, y)` in view coordinates (0 to 1;
    /// clamped to the view; raw values below 0 read 0).
    pub fn raw(&self, x: f32, y: f32) -> (u16, u16) {
        let map = |(a, b): (i16, i16), t: f32| (a as f32 + t.clamp(0.0, 1.0) * (b as f32 - a as f32)).round() as u16;
        (map(self.x_range, x), map(self.y_range, y))
    }
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
    /// What players see it called (help, on-screen buttons), if not its
    /// name capitalised; set with [`ButtonInput::labelled`].
    pub label_text: Option<&'static str>,
}

/// A [`ButtonInput`], for manifests.
pub(crate) const fn button(
    name: &'static str,
    port: u8,
    bit: u8,
    keys: &'static [&'static str],
    pad: &'static [PadButton],
) -> ButtonInput {
    ButtonInput { name, port, bit, keys, pad, label_text: None }
}

impl ButtonInput {
    /// The same input with a label for players (e.g. "Jump" for
    /// `jumpfront`).
    pub const fn labelled(self, label: &'static str) -> ButtonInput {
        ButtonInput { label_text: Some(label), ..self }
    }

    /// What players see it called.
    pub fn label(&self) -> String {
        self.label_text.map_or_else(|| capitalise(self.name), str::to_string)
    }
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
    /// What players see it called: the name, capitalised.
    pub fn label(&self) -> String {
        capitalise(self.name)
    }

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

    /// The gun called `name`, with its player index (0 for the first).
    pub fn gun(&self, name: &str) -> Option<(usize, &Gun)> {
        self.guns.iter().enumerate().find(|(_, g)| g.name == name)
    }

    /// All input names, for messages.
    pub fn names(&self) -> String {
        let names = self.buttons.iter().map(|b| b.name).chain(self.analogs.iter().map(|a| a.name));
        names.chain(self.guns.iter().map(|g| g.name)).collect::<Vec<_>>().join(", ")
    }

    /// One line per control with its default keys and pad bindings (for
    /// help text and docs).
    pub fn describe(&self) -> String {
        let mut out = String::new();
        for (i, g) in self.guns.iter().enumerate() {
            let aim = if i == 0 { "mouse or touch, else pad sticks" } else { "pad sticks (beside a mouse)" };
            let keys: Vec<_> = g.keys_reload.iter().map(|k| key_label(k)).collect();
            let reload = match g.reload {
                Reload::Offscreen => "reload: shoot off screen".to_string(),
                Reload::Button(b) => format!("{b}: right button"),
            };
            out += &format!("  {:<10} aim: {aim}; fire: {}; {reload}", g.name, g.trigger);
            out += &if keys.is_empty() { "\n".to_string() } else { format!(" or {}\n", keys.join(" ")) };
        }
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

    /// The controls a player uses, each with its first default keys (both
    /// directions for an analog), for short help: (key symbols, label).
    /// Coin, test and service are left out, as is anything without keys;
    /// start comes last.
    pub fn summary(&self) -> Vec<(Vec<&'static str>, String)> {
        let analogs = self.analogs.iter().map(|a| {
            let keys: Vec<&'static str> =
                a.keys_neg.iter().take(1).chain(a.keys_pos.iter().take(1)).map(|k| key_symbol(k)).collect();
            (keys, a.label())
        });
        let mut buttons: Vec<&ButtonInput> = self
            .buttons
            .iter()
            .filter(|b| !["coin", "test", "service"].iter().any(|p| b.name.starts_with(p)))
            .collect();
        buttons.sort_by_key(|b| b.name == "start");
        let buttons =
            buttons.into_iter().map(|b| (b.keys.iter().take(1).map(|k| key_symbol(k)).collect::<Vec<_>>(), b.label()));
        let mut out: Vec<(Vec<&'static str>, String)> = Vec::new();
        if let Some(g) = self.guns.first() {
            out.push((vec!["Mouse"], "Aim, click to shoot".to_string()));
            if g.reload == Reload::Offscreen {
                out.push((g.keys_reload.iter().take(1).map(|k| key_symbol(k)).collect(), "Reload".to_string()));
            }
        }
        for (keys, label) in analogs.chain(buttons) {
            // Analogs bound to the same keys (skisuprg's edge and swing) once.
            if !keys.is_empty() && !out.iter().any(|o| o.0 == keys) {
                out.push((keys, label));
            }
        }
        out
    }
}

/// `name` with its first letter in upper case.
fn capitalise(name: &str) -> String {
    let mut c = name.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// A `KeyboardEvent.code` as printed on the key (arrows as arrows).
pub fn key_symbol(code: &str) -> &str {
    match code {
        "ArrowLeft" => "←",
        "ArrowRight" => "→",
        "ArrowUp" => "↑",
        "ArrowDown" => "↓",
        _ => key_label(code),
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
            names.extend(c.guns.iter().map(|g| g.name));
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
            assert!(c.guns.len() <= 2, "{}: at most two guns", g.id);
            for gun in c.guns {
                assert!(c.button(gun.trigger).is_some(), "{}: {} has no trigger button", g.id, gun.name);
                match gun.source {
                    GunSource::Counters { x, y, offscreen_bit } => {
                        assert!(x < 15 && y < 15 && offscreen_bit < 8, "{}: {}", g.id, gun.name)
                    }
                    GunSource::Analog { x, y } => {
                        assert!(x < 8 && y < 8, "{}: {}", g.id, gun.name)
                    }
                }
                if let Reload::Button(b) = gun.reload {
                    assert!(c.button(b).is_some(), "{}: {} reloads with no button {b}", g.id, gun.name);
                }
            }
            assert!(c.button("coin1").is_some(), "{}", g.id);
            assert!(c.coin_start.contains("coin1@"), "{}", g.id);
        }
    }

    #[test]
    fn summary_lists_what_players_press() {
        let s = crate::games::TOPSKATR.controls.summary();
        assert_eq!(s[0], (vec!["←", "→"], "Curving".to_string()));
        assert!(s.iter().any(|c| c == &(vec!["Z"], "Jump".to_string())));
        assert_eq!(s.last().unwrap().1, "Start");
        assert!(!s.iter().any(|c| c.1 == "Coin"));
        for g in crate::games::ALL {
            assert!(!g.controls.summary().is_empty(), "{}", g.id);
        }
    }

    #[test]
    fn gun_aims_map_through_the_calibration() {
        let (_, gun) = crate::games::HOTD.controls.gun("gun1").unwrap();
        let r = |v: i16| v as u16;
        assert_eq!(gun.raw(0.0, 0.0), (r(gun.x_range.0), r(gun.y_range.0)));
        assert_eq!(gun.raw(1.0, 1.0), (r(gun.x_range.1), r(gun.y_range.1)));
        assert_eq!(gun.raw(-1.0, 2.0), (r(gun.x_range.0), r(gun.y_range.1)));
        let (mx, my) = gun.raw(0.5, 0.5);
        assert!(mx > r(gun.x_range.0) && mx < r(gun.x_range.1) && my > r(gun.y_range.0) && my < r(gun.y_range.1));
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
