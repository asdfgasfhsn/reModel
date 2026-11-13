//! Inputs: scripted ones for headless runs, and live ones from players,
//! both by the names in the game's manifest (`romset::Controls`).
//!
//! A script is a comma-separated list of `name@from-to` (hold a button for
//! frames `from..=to`), `name=value@from-to` (set an analog channel) or,
//! for light guns, `gun1=x:y@from-to` (aim at view pixel x, y of the
//! 496x384 view) and `gun1=off@from-to` (aim off the screen). A range
//! may end in `/n`: then the input is held only for the first two frames
//! of every `n` from `from` (`trigger1@2000-3000/20` fires every 20
//! frames). `coin-start` stands for the game's own coin-up and start (its
//! `Controls::coin_start`). Example for topskatr:
//! `coin1@1100-1105,start@1300-1305,curving=0x40@1400-1500`.

use crate::bus::Model2Bus;
use crate::video;
use romset::Controls;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Action {
    Button {
        port: usize,
        bit: u8,
    },
    Analog {
        channel: usize,
        value: u8,
    },
    /// Aim gun `player` at view pixel (x, y), or off screen.
    Gun {
        player: usize,
        at: Option<(u16, u16)>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Event {
    action: Action,
    from: u64,
    to: u64,
    /// Held only for the first [`PULSE`] frames of every `every`.
    every: Option<u64>,
}

/// Frames a periodic input (`@from-to/n`) is held each time.
const PULSE: u64 = 2;

impl Event {
    fn active(&self, frame: u64) -> bool {
        (self.from..=self.to).contains(&frame) && self.every.is_none_or(|n| (frame - self.from) % n < PULSE)
    }
}

#[derive(Clone, Debug, Default)]
pub struct InputScript {
    events: Vec<Event>,
}

fn num(s: &str) -> Option<u64> {
    match s.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

impl InputScript {
    /// Parse `spec` against a game's controls.
    pub fn parse(spec: &str, controls: &Controls) -> Result<Self, String> {
        let mut events = Vec::new();
        for item in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if item == "coin-start" {
                events.extend(Self::parse(controls.coin_start, controls)?.events);
                continue;
            }
            let (what, frames) = item.split_once('@').ok_or_else(|| format!("{item}: missing @frames"))?;
            let (frames, every) = match frames.split_once('/') {
                Some((f, n)) => match num(n) {
                    Some(n) if n > PULSE => (f, Some(n)),
                    _ => return Err(format!("{item}: repeat every n > {PULSE} frames")),
                },
                None => (frames, None),
            };
            let (from, to) = match frames.split_once('-') {
                Some((a, b)) => (num(a), num(b)),
                None => (num(frames), num(frames)),
            };
            let (Some(from), Some(to)) = (from, to) else { return Err(format!("{item}: bad frame range")) };
            let action = match what.split_once('=') {
                Some((name, v)) if controls.gun(name).is_some() => {
                    let player = controls.gun(name).map_or(0, |g| g.0);
                    let at = match v.split_once(':') {
                        _ if v == "off" => None,
                        Some((x, y)) => match (num(x), num(y)) {
                            (Some(x), Some(y)) if x < video::WIDTH as u64 && y < video::HEIGHT as u64 => {
                                Some((x as u16, y as u16))
                            }
                            _ => return Err(format!("{item}: aim at x:y in view pixels (0-495, 0-383) or off")),
                        },
                        None => return Err(format!("{item}: aim at x:y in view pixels (0-495, 0-383) or off")),
                    };
                    Action::Gun { player, at }
                }
                Some((name, v)) => {
                    let a = controls.analog(name).ok_or_else(|| {
                        format!("{name}: unknown analog input (known: {})", Self::analog_names(controls))
                    })?;
                    let value = num(v).filter(|&v| v <= 0xff).ok_or_else(|| format!("{item}: bad value"))? as u8;
                    Action::Analog { channel: a.channel as usize, value }
                }
                None => {
                    let b = controls
                        .button(what)
                        .ok_or_else(|| format!("{what}: unknown input (known: coin-start, {})", controls.names()))?;
                    Action::Button { port: b.port as usize, bit: b.bit }
                }
            };
            events.push(Event { action, from, to, every });
        }
        Ok(InputScript { events })
    }

    fn analog_names(controls: &Controls) -> String {
        controls.analogs.iter().map(|a| a.name).collect::<Vec<_>>().join(", ")
    }

    /// Set the bus inputs for `frame`: everything released and centred
    /// (guns aimed at the middle of the view) except what the script holds.
    pub fn apply(&self, frame: u64, bus: &mut Model2Bus) {
        release_all(bus);
        for e in self.events.iter().filter(|e| e.active(frame)) {
            match e.action {
                Action::Button { port, bit } => bus.inputs[port] &= !bit,
                Action::Analog { channel, value } => bus.analog[channel] = value,
                Action::Gun { player, at } => {
                    let at = at.map(|(x, y)| {
                        ((x as f32 + 0.5) / video::WIDTH as f32, (y as f32 + 0.5) / video::HEIGHT as f32)
                    });
                    set_gun(bus, player, at);
                }
            }
        }
    }
}

/// Every button released, every analog channel at rest and every gun
/// aimed at the middle of the view.
pub fn release_all(bus: &mut Model2Bus) {
    bus.inputs = [0xff; 4];
    bus.analog = [0x80; 8];
    for a in bus.controls.analogs {
        bus.analog[a.channel as usize] = a.centre;
    }
    for p in 0..bus.controls.guns.len() {
        set_gun(bus, p, Some((0.5, 0.5)));
    }
}

/// Aim player `player`'s gun (0 for the first) at `at` in view
/// coordinates (0 to 1 across and down the 496x384 view), or off the
/// screen with `None`. Every pointer source (mouse, touch, a stick, a
/// script) comes through here. False if the game has no such gun.
pub fn set_gun(bus: &mut Model2Bus, player: usize, at: Option<(f32, f32)>) -> bool {
    let Some(gun) = bus.controls.guns.get(player) else { return false };
    match gun.source {
        romset::GunSource::Counters { x, y, offscreen_bit } => {
            let Some(board) = &mut bus.gun else { return false };
            let (rx, ry) = at.map_or((0, 0), |(ax, ay)| gun.raw(ax, ay));
            board.set_counter_pair(x, rx);
            board.set_counter_pair(y, ry);
            board.set_offscreen(offscreen_bit, at.is_none());
        }
        // A mounted gun stays where it was when the pointer leaves the view.
        romset::GunSource::Analog { x, y } => {
            if let Some((ax, ay)) = at {
                let (rx, ry) = gun.raw(ax, ay);
                bus.analog[x as usize] = rx.min(255) as u8;
                bus.analog[y as usize] = ry.min(255) as u8;
            }
        }
    }
    true
}

/// A gun and its buttons, as a player's pointer reports them for one
/// frame: where it aims (view coordinates 0 to 1, `None` off screen), the
/// trigger, and a reload request. Reload on a game that reloads by
/// shooting off screen aims off screen and pulls the trigger.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GunState {
    pub at: Option<(f32, f32)>,
    pub trigger: bool,
    pub reload: bool,
}

/// Apply a player's [`GunState`]: aim, trigger and reload. The trigger is
/// only ever pressed here, never released, so a key bound to it (set by
/// [`apply_player`] first) still fires. False if the game has no gun for
/// `player`.
pub fn apply_gun(bus: &mut Model2Bus, player: usize, g: GunState) -> bool {
    let Some(gun) = bus.controls.guns.get(player) else { return false };
    let (at, fire) = match gun.reload {
        romset::Reload::Offscreen if g.reload => (None, true),
        romset::Reload::Offscreen => (g.at, g.trigger),
        romset::Reload::Button(b) => {
            if g.reload {
                set_button(bus, b, true);
            }
            (g.at, g.trigger)
        }
    };
    set_gun(bus, player, at);
    if fire {
        set_button(bus, gun.trigger, true);
    }
    true
}

/// Press or release the button `name` (live input). False if the game has
/// no such button.
pub fn set_button(bus: &mut Model2Bus, name: &str, pressed: bool) -> bool {
    let Some(b) = bus.controls.button(name) else { return false };
    let (port, bit) = (b.port as usize, b.bit);
    if pressed {
        bus.inputs[port] &= !bit;
    } else {
        bus.inputs[port] |= bit;
    }
    true
}

/// Set analog channel `name` to a raw value. False if the game has no such
/// channel.
pub fn set_analog(bus: &mut Model2Bus, name: &str, value: u8) -> bool {
    let Some(a) = bus.controls.analog(name) else { return false };
    bus.analog[a.channel as usize] = value;
    true
}

use romset::bindings::{Bindings, Input};
/// Default gamepad bindings for aiming a gun with a stick ([`StickCursor`]):
/// both sticks move it, these fire and reload (players can rebind them,
/// `romset::bindings`).
pub use romset::bindings::{PAD_GUN_FIRE as PAD_GUN_TRIGGER, PAD_GUN_RELOAD};

/// A gun aimed with a gamepad: a cursor the sticks move, for players
/// without a mouse or touch screen (or for player 2 beside a mouse).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StickCursor {
    /// Where it aims, as view fractions.
    pub at: (f32, f32),
}

impl Default for StickCursor {
    fn default() -> Self {
        StickCursor { at: (0.5, 0.5) }
    }
}

impl StickCursor {
    /// View widths a fully deflected stick crosses per frame (about a
    /// second for the whole screen).
    const SPEED: f32 = 1.0 / 50.0;

    /// Move by one frame of the pad's sticks and read its fire and reload
    /// buttons (`b`'s).
    pub fn update(&mut self, p: &dyn PlayerInput, b: &Bindings) -> GunState {
        use romset::PadAxis::*;
        let pick = |a: f32, b: f32| if a.abs() > b.abs() { a } else { b };
        let dx = pick(p.pad_axis(RightStickX), p.pad_axis(LeftStickX));
        let dy = pick(p.pad_axis(RightStickY), p.pad_axis(LeftStickY));
        // Same speed in screen distance both ways (the view is 4:3).
        self.at.0 = (self.at.0 + dx * Self::SPEED).clamp(0.0, 0.999);
        self.at.1 = (self.at.1 + dy * Self::SPEED * 4.0 / 3.0).clamp(0.0, 0.999);
        GunState { at: Some(self.at), trigger: held(p, b.pad_fire()), reload: held(p, b.pad_reload()) }
    }
}

/// Where a player's devices stand, for [`apply_player`]. Keys are
/// `KeyboardEvent.code` names; pad buttons and axes follow the W3C standard
/// gamepad.
pub trait PlayerInput {
    fn key(&self, code: &str) -> bool;
    fn pad_button(&self, b: romset::PadButton) -> bool;
    /// Deflection of an axis: -1 to 1 for sticks, 0 to 1 for triggers.
    fn pad_axis(&self, a: romset::PadAxis) -> f32;
}

/// Whether any of `inputs` is held on `p`.
pub fn held(p: &dyn PlayerInput, inputs: &[Input]) -> bool {
    inputs.iter().any(|i| match i {
        Input::Key(k) => p.key(k),
        Input::Pad(b) => p.pad_button(*b),
    })
}

/// Set every control of the game from a player's devices, each read
/// through its bindings (`romset::Bindings`: the manifest's, or the
/// player's remapped ones; the browser reads its touch controls through
/// the manifest's, whatever the keys and pad are remapped to). A button is
/// down if any source holds it; an analog follows the source deflecting it
/// most, keys and buttons holding it at full deflection over its axis
/// (or turning it there at the manifest's [`romset::KeyRamp`]).
pub fn apply_player(bus: &mut Model2Bus, sources: &[(&Bindings, &dyn PlayerInput)]) {
    let controls = bus.controls;
    let (buttons, analogs) = read_player_ramped(controls, sources, &mut bus.key_ramps);
    for (a, v) in controls.analogs.iter().zip(analogs) {
        bus.analog[a.channel as usize] = v;
    }
    let buttons = latch_gear(&mut bus.gear, controls, buttons);
    for (b, down) in controls.buttons.iter().zip(buttons) {
        set_button(bus, b.name, down);
    }
}

/// A gear lever's position ([`romset::Shifter`]) and which of its buttons
/// were down last frame (for their edges).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GearLatch {
    /// The latched gear's index, `None` in neutral.
    pub gear: Option<usize>,
    held: u16,
}

/// Games with a [`romset::Shifter`]: turn the gear buttons' presses into a
/// lever that stays put. A gear's press selects it; neutral's goes to
/// neutral; up and down step (neutral, then the gears in order, and they
/// stop at the ends). The gear buttons then read down only for the latched
/// gear, so the game sees the lever held there.
fn latch_gear(latch: &mut GearLatch, controls: &Controls, mut buttons: Vec<bool>) -> Vec<bool> {
    let Some(s) = &controls.shifter else { return buttons };
    let index = |n: &str| controls.buttons.iter().position(|b| b.name == n).unwrap_or(0);
    let names: Vec<usize> =
        s.gears.iter().map(|g| index(g)).chain([index(s.neutral), index(s.up), index(s.down)]).collect();
    let n = s.gears.len();
    let pressed = |i: usize| buttons[names[i]] && latch.held & (1 << i) == 0;
    let mut gear = latch.gear;
    for i in 0..n {
        if pressed(i) {
            gear = Some(i);
        }
    }
    if pressed(n) {
        gear = None;
    }
    if pressed(n + 1) {
        gear = Some(gear.map_or(0, |g| (g + 1).min(n - 1)));
    }
    if pressed(n + 2) {
        gear = gear.and_then(|g| g.checked_sub(1));
    }
    latch.held = names.iter().enumerate().fold(0, |m, (i, &b)| m | (buttons[b] as u16) << i);
    latch.gear = gear;
    for (i, &b) in names.iter().enumerate().take(n) {
        buttons[b] = gear == Some(i);
    }
    buttons
}

/// Where the keys and buttons have moved each analog input with a
/// [`romset::KeyRamp`] (-1 to 1, by the manifest's order): they turn it at
/// that rate rather than jump to full deflection.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct KeyRamps([f32; 8]);

impl KeyRamps {
    /// One frame toward `target` (-1, 0 or 1, the keys held) for input `i`:
    /// out from the centre at `attack` frames for the whole way, back to
    /// it (and through it, toward the other side) at `release`.
    fn step(&mut self, i: usize, target: f32, r: romset::KeyRamp) -> f32 {
        let Some(at) = self.0.get_mut(i) else { return target };
        let out = target != 0.0 && (*at == 0.0 || (*at > 0.0) == (target > 0.0)) && target.abs() > at.abs();
        let frames = if out { r.attack } else { r.release };
        let step = 1.0 / frames.max(1) as f32;
        *at += (target - *at).clamp(-step, step);
        // Steps of 1/n add up to the ends only nearly: land on them.
        for end in [-1.0, 0.0, 1.0] {
            if (*at - end).abs() < 1e-4 {
                *at = end;
            }
        }
        *at
    }
}

/// What [`apply_player`] sets: each button's state and each analog's value,
/// in the manifest's order.
#[cfg(test)]
fn read_player(controls: &Controls, sources: &[(&Bindings, &dyn PlayerInput)]) -> (Vec<bool>, Vec<u8>) {
    read_player_ramped(controls, sources, &mut KeyRamps::default())
}

/// [`read_player`], moving the analogs with a key ramp by one frame.
fn read_player_ramped(
    controls: &Controls,
    sources: &[(&Bindings, &dyn PlayerInput)],
    ramps: &mut KeyRamps,
) -> (Vec<bool>, Vec<u8>) {
    let analogs = controls.analogs.iter().enumerate().map(|(i, a)| {
        // The first source holding a key decides; a source's key overrides
        // its axis.
        let (mut keys, mut axis) = (0.0f32, 0.0f32);
        for (b, p) in sources {
            let k = if held(*p, b.neg(i)) {
                -1.0
            } else if held(*p, b.pos(i)) {
                1.0
            } else {
                0.0
            };
            if k != 0.0 {
                if keys == 0.0 {
                    keys = k;
                }
            } else {
                let d = b.axis(i).read(|x| p.pad_axis(x));
                if d.abs() > axis.abs() {
                    axis = d;
                }
            }
        }
        if let Some(r) = a.key_ramp {
            keys = ramps.step(i, keys, r);
        }
        a.value(if keys.abs() >= axis.abs() { keys } else { axis })
    });
    let buttons = (0..controls.buttons.len()).map(|i| sources.iter().any(|(b, p)| held(*p, b.button(i))));
    (buttons.collect(), analogs.collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topskatr() -> &'static Controls {
        &romset::games::find("topskatr").unwrap().controls
    }

    #[test]
    fn parses_buttons_and_analog() {
        let s = InputScript::parse("coin1@10-12, curving=0x40@5", topskatr()).unwrap();
        assert_eq!(s.events.len(), 2);
        assert_eq!(s.events[0], Event { action: Action::Button { port: 0, bit: 1 }, from: 10, to: 12, every: None });
        assert_eq!(
            s.events[1],
            Event { action: Action::Analog { channel: 0, value: 0x40 }, from: 5, to: 5, every: None }
        );
    }

    #[test]
    fn rejects_unknown_names() {
        assert!(InputScript::parse("kickflip@1", topskatr()).is_err());
        assert!(InputScript::parse("coin1", topskatr()).is_err());
    }

    #[test]
    fn parses_gun_aims() {
        let c = &romset::games::find("hotd").unwrap().controls;
        let s = InputScript::parse("gun1=10:20@5-6,gun2=off@7", c).unwrap();
        assert_eq!(s.events[0].action, Action::Gun { player: 0, at: Some((10, 20)) });
        assert_eq!(s.events[1].action, Action::Gun { player: 1, at: None });
        assert!(InputScript::parse("gun1=496:0@1", c).is_err());
        assert!(InputScript::parse("gun1=5@1", c).is_err());
    }

    #[test]
    fn periodic_inputs_pulse() {
        let s = InputScript::parse("coin1@100-200/20", topskatr()).unwrap();
        let on: Vec<u64> = (90..150).filter(|&f| s.events[0].active(f)).collect();
        assert_eq!(on, [100, 101, 120, 121, 140, 141]);
        assert!(InputScript::parse("coin1@1-9/1", topskatr()).is_err());
    }

    #[test]
    fn coin_start_expands_to_each_games_script() {
        for g in romset::games::ALL {
            let s = InputScript::parse("coin-start", &g.controls).unwrap();
            let direct = InputScript::parse(g.controls.coin_start, &g.controls).unwrap();
            assert!(!s.events.is_empty());
            assert_eq!(s.events, direct.events, "{}", g.id);
        }
    }

    /// A keyboard and pad for tests: keys held, pad buttons held, one axis.
    struct Fake {
        keys: Vec<&'static str>,
        pad: Vec<romset::PadButton>,
        axis: (romset::PadAxis, f32),
    }

    impl PlayerInput for Fake {
        fn key(&self, code: &str) -> bool {
            self.keys.contains(&code)
        }
        fn pad_button(&self, b: romset::PadButton) -> bool {
            self.pad.contains(&b)
        }
        fn pad_axis(&self, a: romset::PadAxis) -> f32 {
            if a == self.axis.0 {
                self.axis.1
            } else {
                0.0
            }
        }
    }

    fn fake(keys: &[&'static str], pad: &[romset::PadButton], axis: (romset::PadAxis, f32)) -> Fake {
        Fake { keys: keys.to_vec(), pad: pad.to_vec(), axis }
    }

    #[test]
    fn players_follow_their_bindings() {
        use romset::{PadAxis, PadButton};
        let c = topskatr();
        let jump = c.buttons.iter().position(|b| b.name == "jumpfront").unwrap();
        let tail = c.buttons.iter().position(|b| b.name == "jumptail").unwrap();
        let defaults = Bindings::defaults(c);
        // The manifest's: Z jumps, the left stick steers.
        let p = fake(&["KeyZ"], &[], (PadAxis::LeftStickX, 1.0));
        let (b, a) = read_player(c, &[(&defaults, &p)]);
        assert!(b[jump] && !b[tail]);
        assert_eq!(a[0], c.analogs[0].value(1.0));
        // Remapped: J jumps (not Z), the right stick steers, inverted.
        let text = "jumpfront = KeyJ\ncurving.axis = -RightStickX\n";
        let mine = Bindings::with_overrides(c, text, romset::bindings::RESERVED_KEYS);
        let (b, a) = read_player(c, &[(&mine, &p)]);
        assert!(!b[jump]);
        assert_eq!(a[0], c.analogs[0].value(0.0));
        let q = fake(&["KeyJ"], &[], (PadAxis::RightStickX, 0.5));
        let (b, a) = read_player(c, &[(&mine, &q)]);
        assert!(b[jump]);
        assert_eq!(a[0], c.analogs[0].value(-0.5));
        // A key held overrides the axis; two sources merge (the browser's
        // touch controls through the manifest's bindings).
        let r = fake(&["ArrowRight"], &[], (PadAxis::RightStickX, -1.0));
        assert_eq!(read_player(c, &[(&mine, &r)]).1[0], c.analogs[0].value(1.0));
        let touch = fake(&[], &[PadButton::South], (PadAxis::LeftStickX, -0.25));
        let none = fake(&[], &[], (PadAxis::None, 0.0));
        let (b, a) = read_player(c, &[(&mine, &none), (&defaults, &touch)]);
        assert!(b[jump]);
        assert_eq!(a[0], c.analogs[0].value(-0.25));
    }

    #[test]
    fn keys_turn_a_ramped_wheel_gradually() {
        use romset::PadAxis;
        let c = &romset::games::find("daytona").unwrap().controls;
        let wheel = &c.analogs[0];
        let r = wheel.key_ramp.unwrap();
        let defaults = Bindings::defaults(c);
        let mut ramps = KeyRamps::default();
        let mut frame = |keys: &[&'static str], axis: f32| {
            let p = fake(keys, &[], (PadAxis::LeftStickX, axis));
            read_player_ramped(c, &[(&defaults, &p)], &mut ramps).1[0]
        };
        // Right held: out to full lock over `attack` frames, one step each.
        let out: Vec<u8> = (0..r.attack).map(|_| frame(&["ArrowRight"], 0.0)).collect();
        assert!(out.windows(2).all(|w| w[1] > w[0]));
        assert_eq!(out[0], wheel.value(1.0 / r.attack as f32));
        assert_eq!(*out.last().unwrap(), wheel.value(1.0));
        assert_eq!(frame(&["ArrowRight"], 0.0), wheel.value(1.0));
        // Let go: back to the centre over `release` frames.
        let back: Vec<u8> = (0..r.release).map(|_| frame(&[], 0.0)).collect();
        assert!(back.windows(2).all(|w| w[1] < w[0]));
        assert_eq!(*back.last().unwrap(), wheel.centre);
        // Left after a short tap right: back through the centre at the
        // release rate, then out at the attack rate.
        frame(&["ArrowRight"], 0.0);
        frame(&["ArrowRight"], 0.0);
        let left: Vec<u8> = (0..4).map(|_| frame(&["ArrowLeft"], 0.0)).collect();
        assert!(left.windows(2).all(|w| w[1] < w[0]) && left[3] < wheel.centre);
        // A stick is the player's own hand: it goes straight through.
        let mut ramps = KeyRamps::default();
        let p = fake(&[], &[], (PadAxis::LeftStickX, -1.0));
        assert_eq!(read_player_ramped(c, &[(&defaults, &p)], &mut ramps).1[0], wheel.value(-1.0));
        // Games without a ramp keep the keys a switch.
        let s = &romset::games::find("srallyc").unwrap().controls;
        let p = fake(&["ArrowRight"], &[], (PadAxis::None, 0.0));
        let sb = Bindings::defaults(s);
        assert_eq!(read_player_ramped(s, &[(&sb, &p)], &mut KeyRamps::default()).1[0], s.analogs[0].value(1.0));
    }

    #[test]
    fn the_gear_lever_stays_where_it_is_put() {
        let c = &romset::games::find("srallyc").unwrap().controls;
        let defaults = Bindings::defaults(c);
        let gear_of = |b: &[bool]| {
            (0..4).find(|&i| b[c.buttons.iter().position(|x| x.name == format!("gear{}", i + 1)).unwrap()])
        };
        let mut latch = GearLatch::default();
        let mut step = |keys: &[&'static str]| {
            let p = fake(keys, &[], (romset::PadAxis::None, 0.0));
            let (b, _) = read_player(c, &[(&defaults, &p)]);
            gear_of(&latch_gear(&mut latch, c, b))
        };
        assert_eq!(step(&[]), None);
        assert_eq!(step(&["KeyW"]), Some(1));
        assert_eq!(step(&[]), Some(1), "stays after release");
        assert_eq!(step(&["KeyA"]), Some(2));
        assert_eq!(step(&["KeyA"]), Some(2), "a held key steps once");
        assert_eq!(step(&[]), Some(2));
        assert_eq!(step(&["KeyZ"]), Some(1));
        assert_eq!(step(&[]), Some(1));
        assert_eq!(step(&["KeyZ"]), Some(0));
        assert_eq!(step(&[]), Some(0));
        assert_eq!(step(&["KeyZ"]), None, "down from first is neutral");
        assert_eq!(step(&[]), None);
        assert_eq!(step(&["KeyA"]), Some(0), "up from neutral is first");
        assert_eq!(step(&[]), Some(0));
        assert_eq!(step(&["KeyX"]), None);
        assert_eq!(step(&["KeyR"]), Some(3));
        assert_eq!(step(&[]), Some(3));
        assert_eq!(step(&["KeyA"]), Some(3), "stops at the top");
    }
}
