//! Inputs: scripted ones for headless runs, and live ones from players,
//! both by the names in the game's manifest (`romset::Controls`).
//!
//! A script is a comma-separated list of `name@from-to` (hold a button for
//! frames `from..=to`) or `name=value@from-to` (set an analog channel).
//! `coin-start` stands for the game's own coin-up and start (its
//! `Controls::coin_start`). Example for topskatr:
//! `coin1@1100-1105,start@1300-1305,curving=0x40@1400-1500`.

use crate::bus::Model2Bus;
use romset::Controls;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Action {
    Button { port: usize, bit: u8 },
    Analog { channel: usize, value: u8 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Event {
    action: Action,
    from: u64,
    to: u64,
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
            let (from, to) = match frames.split_once('-') {
                Some((a, b)) => (num(a), num(b)),
                None => (num(frames), num(frames)),
            };
            let (Some(from), Some(to)) = (from, to) else { return Err(format!("{item}: bad frame range")) };
            let action = match what.split_once('=') {
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
            events.push(Event { action, from, to });
        }
        Ok(InputScript { events })
    }

    fn analog_names(controls: &Controls) -> String {
        controls.analogs.iter().map(|a| a.name).collect::<Vec<_>>().join(", ")
    }

    /// Set the bus inputs for `frame`: everything released and centred
    /// except what the script holds.
    pub fn apply(&self, frame: u64, bus: &mut Model2Bus) {
        release_all(bus);
        for e in self.events.iter().filter(|e| (e.from..=e.to).contains(&frame)) {
            match e.action {
                Action::Button { port, bit } => bus.inputs[port] &= !bit,
                Action::Analog { channel, value } => bus.analog[channel] = value,
            }
        }
    }
}

/// Every button released and every analog channel at rest.
pub fn release_all(bus: &mut Model2Bus) {
    bus.inputs = [0xff; 4];
    bus.analog = [0x80; 8];
    for a in bus.controls.analogs {
        bus.analog[a.channel as usize] = a.centre;
    }
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

/// Where a player's devices stand, for [`apply_player`]. Keys are
/// `KeyboardEvent.code` names; pad buttons and axes follow the W3C standard
/// gamepad.
pub trait PlayerInput {
    fn key(&self, code: &str) -> bool;
    fn pad_button(&self, b: romset::PadButton) -> bool;
    /// Deflection of an axis: -1 to 1 for sticks, 0 to 1 for triggers.
    fn pad_axis(&self, a: romset::PadAxis) -> f32;
}

/// Set every control of the game from a player's keyboard and pad, with
/// the manifest's default bindings. `extra_button` reports other sources
/// (on-screen touch buttons) by input name.
pub fn apply_player(bus: &mut Model2Bus, p: &impl PlayerInput, extra_button: impl Fn(&str) -> bool) {
    let controls = bus.controls;
    for a in controls.analogs {
        let held = |keys: &[&str], pad: &[romset::PadButton]| {
            keys.iter().any(|k| p.key(k)) || pad.iter().any(|&b| p.pad_button(b))
        };
        let deflection = if held(a.keys_neg, a.pad_neg) {
            -1.0
        } else if held(a.keys_pos, a.pad_pos) {
            1.0
        } else {
            p.pad_axis(a.axis)
        };
        bus.analog[a.channel as usize] = a.value(deflection);
    }
    for b in controls.buttons {
        let down = b.keys.iter().any(|k| p.key(k)) || b.pad.iter().any(|&x| p.pad_button(x)) || extra_button(b.name);
        set_button(bus, b.name, down);
    }
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
        assert_eq!(s.events[0], Event { action: Action::Button { port: 0, bit: 1 }, from: 10, to: 12 });
        assert_eq!(s.events[1], Event { action: Action::Analog { channel: 0, value: 0x40 }, from: 5, to: 5 });
    }

    #[test]
    fn rejects_unknown_names() {
        assert!(InputScript::parse("kickflip@1", topskatr()).is_err());
        assert!(InputScript::parse("coin1", topskatr()).is_err());
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
}
