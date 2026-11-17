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
    /// A gear lever the players latch ([`Shifter`]), if the game has one.
    pub shifter: Option<Shifter>,
    /// Coin up and start a game, in the `--input` script syntax: what
    /// `coin-start` in a script stands for (`make capture`, `make bench`).
    pub coin_start: &'static str,
}

/// A gear lever that stays where it is put: an H gate (or any lever whose
/// position the game reads from held bits). Players have keys and buttons,
/// not a lever, so the lever's position is **latched**: pressing a gear's
/// button puts the lever in that gear until another is chosen, and
/// [`up`](Shifter::up) and [`down`](Shifter::down) step through the gate
/// (neutral, then the gears in order). The game sees the latched gear's
/// button held (its bits as the manifest gives them; they may be several)
/// and nothing in neutral. The buttons of `neutral`, `up` and `down` are
/// virtual: bit mask 0, so they set nothing on the board and exist for
/// players to bind. Scripted inputs hold the gear buttons directly.
pub struct Shifter {
    /// Button names of the gears, lowest first.
    pub gears: &'static [&'static str],
    /// Button that puts the lever in neutral.
    pub neutral: &'static str,
    /// Buttons that step to the next gear up and down.
    pub up: &'static str,
    pub down: &'static str,
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
    /// screen and 1 when it sees nothing. The original Model 2's gun
    /// registers at `0x1C00100` (`vcop`) have the same layout, read
    /// directly (docs/hw/model2.md).
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
    /// How fast those keys and buttons move it (`None`: at once to full
    /// deflection and back, as a switch). A game whose own code steers
    /// straight from the position, as a cabinet's wheel can't jump, wants
    /// one (`daytona`, docs/play.md "Digital steering").
    pub key_ramp: Option<KeyRamp>,
}

/// Keys and buttons moving an analog input as a hand would: frames from
/// rest to full deflection while one is held, and from full deflection
/// back to rest once it's let go (or on the way to the other side).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyRamp {
    pub attack: u8,
    pub release: u8,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    // Virtual buttons (after the standard pad's, so stored indices and
    // names keep their meaning): a stick pushed one way. See `StickDirs`.
    LeftStickUp,
    LeftStickDown,
    LeftStickLeft,
    LeftStickRight,
    RightStickUp,
    RightStickDown,
    RightStickLeft,
    RightStickRight,
}

impl PadButton {
    /// Every button, in index order.
    pub const ALL: [PadButton; 16] = [
        PadButton::South,
        PadButton::East,
        PadButton::West,
        PadButton::North,
        PadButton::LeftShoulder,
        PadButton::RightShoulder,
        PadButton::LeftTrigger,
        PadButton::RightTrigger,
        PadButton::Select,
        PadButton::Start,
        PadButton::LeftStick,
        PadButton::RightStick,
        PadButton::DPadUp,
        PadButton::DPadDown,
        PadButton::DPadLeft,
        PadButton::DPadRight,
    ];

    /// The stick directions: virtual buttons, read from the sticks' axes
    /// (left stick up, down, left, right, then the right stick's).
    pub const STICK_DIRS: [PadButton; 8] = [
        PadButton::LeftStickUp,
        PadButton::LeftStickDown,
        PadButton::LeftStickLeft,
        PadButton::LeftStickRight,
        PadButton::RightStickUp,
        PadButton::RightStickDown,
        PadButton::RightStickLeft,
        PadButton::RightStickRight,
    ];

    /// Whether this is a stick direction rather than a button of the
    /// standard pad.
    pub fn is_stick_dir(self) -> bool {
        self as usize >= Self::ALL.len()
    }

    /// The stick direction an axis pushed to its negative end (left or up)
    /// or positive end (right or down) is; None for a trigger.
    pub fn stick_dir(axis: PadAxis, negative: bool) -> Option<PadButton> {
        use PadButton::*;
        Some(match (axis, negative) {
            (PadAxis::LeftStickX, true) => LeftStickLeft,
            (PadAxis::LeftStickX, false) => LeftStickRight,
            (PadAxis::LeftStickY, true) => LeftStickUp,
            (PadAxis::LeftStickY, false) => LeftStickDown,
            (PadAxis::RightStickX, true) => RightStickLeft,
            (PadAxis::RightStickX, false) => RightStickRight,
            (PadAxis::RightStickY, true) => RightStickUp,
            (PadAxis::RightStickY, false) => RightStickDown,
            _ => return None,
        })
    }

    /// The left stick direction that stands in for a d-pad direction.
    pub fn left_stick_of(self) -> Option<PadButton> {
        Some(match self {
            PadButton::DPadUp => PadButton::LeftStickUp,
            PadButton::DPadDown => PadButton::LeftStickDown,
            PadButton::DPadLeft => PadButton::LeftStickLeft,
            PadButton::DPadRight => PadButton::LeftStickRight,
            _ => return None,
        })
    }

    /// The stick whose axes a stick direction reads (the part of the pad it
    /// is, for pictures); a standard button is itself.
    pub fn part(self) -> PadButton {
        match self {
            PadButton::LeftStickUp
            | PadButton::LeftStickDown
            | PadButton::LeftStickLeft
            | PadButton::LeftStickRight => PadButton::LeftStick,
            PadButton::RightStickUp
            | PadButton::RightStickDown
            | PadButton::RightStickLeft
            | PadButton::RightStickRight => PadButton::RightStick,
            b => b,
        }
    }

    /// Index in the W3C standard gamepad's `buttons` (for a stick
    /// direction, past them: it isn't one).
    pub fn index(self) -> u32 {
        self as u32
    }

    /// The button at a W3C standard gamepad index.
    pub fn from_index(i: u32) -> Option<PadButton> {
        Self::ALL.get(i as usize).copied()
    }

    /// The button named `name` (its variant's name, as stored).
    pub fn from_name(name: &str) -> Option<PadButton> {
        Self::ALL.into_iter().chain(Self::STICK_DIRS).find(|b| format!("{b:?}") == name)
    }

    /// As an Xbox pad labels it.
    pub fn label(self) -> &'static str {
        match self {
            PadButton::South => "A",
            PadButton::East => "B",
            PadButton::West => "X",
            PadButton::North => "Y",
            PadButton::LeftShoulder => "LB",
            PadButton::RightShoulder => "RB",
            PadButton::LeftTrigger => "LT",
            PadButton::RightTrigger => "RT",
            PadButton::Select => "Back",
            PadButton::Start => "Start",
            PadButton::LeftStick => "L stick press",
            PadButton::RightStick => "R stick press",
            PadButton::DPadUp => "D-pad ↑",
            PadButton::DPadDown => "D-pad ↓",
            PadButton::DPadLeft => "D-pad ←",
            PadButton::DPadRight => "D-pad →",
            PadButton::LeftStickUp => "L stick ↑",
            PadButton::LeftStickDown => "L stick ↓",
            PadButton::LeftStickLeft => "L stick ←",
            PadButton::LeftStickRight => "L stick →",
            PadButton::RightStickUp => "R stick ↑",
            PadButton::RightStickDown => "R stick ↓",
            PadButton::RightStickLeft => "R stick ←",
            PadButton::RightStickRight => "R stick →",
        }
    }
}

/// A stick read as an 8-way lever: which of the four directions each stick
/// holds now, as the virtual [`PadButton`]s (`LeftStickUp` ...). Pushed
/// past a threshold the stick holds the direction its angle is in, and a
/// diagonal both of its two, like a lever's microswitches. It remembers
/// what it held, so a stick resting on the edge of a threshold or of a
/// sector doesn't chatter: it takes a push to start and lets go lower.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StickDirs {
    /// One bit per [`PadButton::STICK_DIRS`].
    held: u8,
}

impl StickDirs {
    /// Push (of full, past the dead zone) that starts a direction. A
    /// digital lever wants a real push, not a stick's slightest lean.
    pub const PUSH: f32 = 0.5;
    /// A held direction lets go this far (of full) below that, or at the
    /// dead zone if that's higher.
    pub const LET_GO: f32 = 0.1;
    /// A direction starts when the stick's angle is within 67.5 degrees of
    /// it (the diagonals' 45 are both, the axes' 22.5 are alone), and holds
    /// to about 73. As the ratio of the side across to the side along.
    const START: f32 = 0.414;
    const HOLD: f32 = 0.30;

    /// Read the sticks, each as (x right, y down) from -1 to 1, with the
    /// player's dead zone (0 to 1).
    pub fn update(&mut self, left: (f32, f32), right: (f32, f32), dead: f32) -> StickDirs {
        let lo = self.held & 0x0f;
        let hi = self.held >> 4;
        self.held = Self::stick(lo, left, dead) | Self::stick(hi, right, dead) << 4;
        *self
    }

    /// One stick's four bits (up, down, left, right) from its last.
    fn stick(was: u8, (x, y): (f32, f32), dead: f32) -> u8 {
        let push = Self::PUSH.max(dead);
        let limit = if was != 0 { (push - Self::LET_GO).max(dead) } else { push };
        if x.hypot(y) <= limit {
            return 0;
        }
        let dir = |bit: u8, along: f32, across: f32| {
            let ratio = if was & bit != 0 { Self::HOLD } else { Self::START };
            if along > 0.0 && along > ratio * across.abs() {
                bit
            } else {
                0
            }
        };
        dir(1, -y, x) | dir(2, y, x) | dir(4, -x, y) | dir(8, x, y)
    }

    /// Whether `b` is held (false for anything but a stick direction).
    pub fn held(&self, b: PadButton) -> bool {
        PadButton::STICK_DIRS.iter().position(|&d| d == b).is_some_and(|i| self.held >> i & 1 != 0)
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

impl PadAxis {
    pub const ALL: [PadAxis; 8] = [
        PadAxis::None,
        PadAxis::LeftStickX,
        PadAxis::LeftStickY,
        PadAxis::RightStickX,
        PadAxis::RightStickY,
        PadAxis::LeftTrigger,
        PadAxis::RightTrigger,
        PadAxis::Triggers,
    ];

    /// The axis named `name` (its variant's name, as stored).
    pub fn from_name(name: &str) -> Option<PadAxis> {
        Self::ALL.into_iter().find(|a| format!("{a:?}") == name)
    }

    /// For players.
    pub fn label(self) -> &'static str {
        match self {
            PadAxis::None => "",
            PadAxis::LeftStickX => "left stick ↔",
            PadAxis::LeftStickY => "left stick ↕",
            PadAxis::RightStickX => "right stick ↔",
            PadAxis::RightStickY => "right stick ↕",
            PadAxis::LeftTrigger => "LT",
            PadAxis::RightTrigger => "RT",
            PadAxis::Triggers => "RT − LT",
        }
    }
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
pub(crate) fn key_label(code: &str) -> &str {
    code.strip_prefix("Key").or_else(|| code.strip_prefix("Digit")).unwrap_or(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Polar position of a stick: degrees clockwise from up, and a push.
    fn at(deg: f32, push: f32) -> (f32, f32) {
        let r = deg.to_radians();
        (r.sin() * push, -r.cos() * push)
    }

    /// What `StickDirs` holds from a fresh start for one push of the left
    /// stick, as "udlr" letters.
    fn dirs(deg: f32, push: f32) -> String {
        let mut d = StickDirs::default();
        d.update(at(deg, push), (0.0, 0.0), 0.15);
        let names = [
            (PadButton::LeftStickUp, 'u'),
            (PadButton::LeftStickDown, 'd'),
            (PadButton::LeftStickLeft, 'l'),
            (PadButton::LeftStickRight, 'r'),
        ];
        names.iter().filter(|(b, _)| d.held(*b)).map(|(_, c)| *c).collect()
    }

    #[test]
    fn a_stick_is_an_eight_way_lever() {
        assert_eq!(dirs(0.0, 1.0), "u");
        assert_eq!(dirs(90.0, 1.0), "r");
        assert_eq!(dirs(180.0, 1.0), "d");
        assert_eq!(dirs(270.0, 1.0), "l");
        assert_eq!(dirs(45.0, 1.0), "ur");
        assert_eq!(dirs(135.0, 1.0), "dr");
        assert_eq!(dirs(225.0, 1.0), "dl");
        assert_eq!(dirs(315.0, 1.0), "ul");
        // The sectors' edges: an axis owns 22.5 degrees each side of it.
        assert_eq!(dirs(15.0, 1.0), "u");
        assert_eq!(dirs(30.0, 1.0), "ur");
        assert_eq!(dirs(60.0, 1.0), "ur");
        assert_eq!(dirs(75.0, 1.0), "r");
    }

    #[test]
    fn a_stick_needs_a_push() {
        assert_eq!(dirs(0.0, 0.2), "");
        assert_eq!(dirs(0.0, 0.49), "");
        assert_eq!(dirs(0.0, 0.51), "u");
        // A dead zone over the push raises it.
        let mut d = StickDirs::default();
        d.update((0.0, -0.55), (0.0, 0.0), 0.6);
        assert!(!d.held(PadButton::LeftStickUp));
        d.update((0.0, -0.65), (0.0, 0.0), 0.6);
        assert!(d.held(PadButton::LeftStickUp));
    }

    #[test]
    fn a_stick_lets_go_lower_and_holds_its_sector() {
        let mut d = StickDirs::default();
        let free = (0.0, 0.0);
        d.update(at(0.0, 0.6), free, 0.15);
        assert!(d.held(PadButton::LeftStickUp));
        // Down to the let-go point: still held; past it: let go.
        d.update(at(0.0, 0.45), free, 0.15);
        assert!(d.held(PadButton::LeftStickUp));
        d.update(at(0.0, 0.35), free, 0.15);
        assert!(!d.held(PadButton::LeftStickUp));
        // And it takes the full push to start again.
        d.update(at(0.0, 0.45), free, 0.15);
        assert!(!d.held(PadButton::LeftStickUp));
        // Held up, drifting round to 70 degrees: right joins but up stays
        // until about 73; from a fresh start up wouldn't start there.
        d.update(at(0.0, 1.0), free, 0.15);
        d.update(at(70.0, 1.0), free, 0.15);
        assert!(d.held(PadButton::LeftStickUp) && d.held(PadButton::LeftStickRight));
        d.update(at(80.0, 1.0), free, 0.15);
        assert!(!d.held(PadButton::LeftStickUp));
        assert_eq!(dirs(70.0, 1.0), "r");
    }

    #[test]
    fn the_sticks_are_read_apart() {
        let mut d = StickDirs::default();
        d.update((0.0, 0.0), at(270.0, 1.0), 0.15);
        assert!(d.held(PadButton::RightStickLeft));
        assert!(!d.held(PadButton::LeftStickLeft) && !d.held(PadButton::DPadLeft) && !d.held(PadButton::South));
    }

    #[test]
    fn stick_buttons_are_named_and_append_to_the_pad() {
        assert_eq!(PadButton::DPadRight.index(), 15);
        assert_eq!(PadButton::LeftStickUp.index(), 16);
        assert_eq!(PadButton::from_index(16), None);
        for b in PadButton::STICK_DIRS {
            assert!(b.is_stick_dir());
            assert_eq!(PadButton::from_name(&format!("{b:?}")), Some(b));
        }
        for b in PadButton::ALL {
            assert!(!b.is_stick_dir());
        }
        assert_eq!(PadButton::stick_dir(PadAxis::RightStickY, true), Some(PadButton::RightStickUp));
        assert_eq!(PadButton::stick_dir(PadAxis::LeftTrigger, true), None);
    }

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
                let in_shifter = c
                    .shifter
                    .as_ref()
                    .is_some_and(|s| s.gears.contains(&b.name) || [s.neutral, s.up, s.down].contains(&b.name));
                // A gear's bits may be several; the shifter's virtual buttons have none.
                assert!(b.port < 4 && (b.bit.count_ones() == 1 || in_shifter), "{}: {}", g.id, b.name);
            }
            if let Some(s) = &c.shifter {
                for n in s.gears.iter().chain([&s.neutral, &s.up, &s.down]) {
                    assert!(c.button(n).is_some(), "{}: shifter button {n}", g.id);
                }
                for n in [s.neutral, s.up, s.down] {
                    assert_eq!(c.button(n).unwrap().bit, 0, "{}: {n} is virtual", g.id);
                }
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
            key_ramp: None,
        };
        assert_eq!(a.value(0.0), 0x80);
        assert_eq!(a.value(1.0), 0x10);
        assert_eq!(a.value(-1.0), 0xf0);
        assert_eq!(a.value(-3.0), 0xf0);
    }
}
