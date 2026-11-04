//! Scripted inputs for headless runs.
//!
//! A script is a comma-separated list of `name@from-to` (hold a button for
//! frames `from..=to`) or `name=value@from-to` (set an analog channel).
//! Example: `coin1@1100-1105,start@1300-1305,curving=0x40@1400-1500`.

use crate::bus::Model2Bus;

/// Digital inputs: (name, port index into `Model2Bus::inputs`, bit).
/// Ports read active low. Coin/test/service follow the common Model 2
/// layout. The game buttons are topskatr's; all were confirmed by scripted
/// presses (README, "What's known"). Test and service are not yet checked.
const BUTTONS: &[(&str, usize, u8)] = &[
    ("coin1", 0, 0x01),
    ("coin2", 0, 0x02),
    ("test", 0, 0x04),
    ("service", 0, 0x08),
    ("selright", 0, 0x10),
    ("jumpfront", 0, 0x20),
    ("start", 0, 0x40),
    ("selleft", 0, 0x80),
    ("jumptail", 1, 0x01),
];

/// Analog channels on the I/O chip: topskatr's two board pots. Curving
/// is steering (low = right), slide makes the skater crouch and slide.
const ANALOG: &[(&str, usize)] = &[("curving", 0), ("slide", 1)];

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
    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut events = Vec::new();
        for item in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (what, frames) = item.split_once('@').ok_or_else(|| format!("{item}: missing @frames"))?;
            let (from, to) = match frames.split_once('-') {
                Some((a, b)) => (num(a), num(b)),
                None => (num(frames), num(frames)),
            };
            let (Some(from), Some(to)) = (from, to) else { return Err(format!("{item}: bad frame range")) };
            let action = match what.split_once('=') {
                Some((name, v)) => {
                    let channel = ANALOG
                        .iter()
                        .find(|a| a.0 == name)
                        .map(|a| a.1)
                        .ok_or_else(|| format!("{name}: unknown analog input"))?;
                    let value = num(v).filter(|&v| v <= 0xff).ok_or_else(|| format!("{item}: bad value"))? as u8;
                    Action::Analog { channel, value }
                }
                None => {
                    let &(_, port, bit) = BUTTONS
                        .iter()
                        .find(|b| b.0 == what)
                        .ok_or_else(|| format!("{what}: unknown input (known: {})", Self::names()))?;
                    Action::Button { port, bit }
                }
            };
            events.push(Event { action, from, to });
        }
        Ok(InputScript { events })
    }

    pub fn names() -> String {
        BUTTONS.iter().map(|b| b.0).chain(ANALOG.iter().map(|a| a.0)).collect::<Vec<_>>().join(", ")
    }

    /// Set the bus inputs for `frame`: everything released and centred
    /// except what the script holds.
    pub fn apply(&self, frame: u64, bus: &mut Model2Bus) {
        bus.inputs = [0xff; 4];
        bus.analog = [0x80; 8];
        for e in self.events.iter().filter(|e| (e.from..=e.to).contains(&frame)) {
            match e.action {
                Action::Button { port, bit } => bus.inputs[port] &= !bit,
                Action::Analog { channel, value } => bus.analog[channel] = value,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_buttons_and_analog() {
        let s = InputScript::parse("coin1@10-12, curving=0x40@5").unwrap();
        assert_eq!(s.events.len(), 2);
        assert_eq!(s.events[0], Event { action: Action::Button { port: 0, bit: 1 }, from: 10, to: 12 });
        assert_eq!(s.events[1], Event { action: Action::Analog { channel: 0, value: 0x40 }, from: 5, to: 5 });
    }

    #[test]
    fn rejects_unknown_names() {
        assert!(InputScript::parse("kickflip@1").is_err());
        assert!(InputScript::parse("coin1").is_err());
    }
}
