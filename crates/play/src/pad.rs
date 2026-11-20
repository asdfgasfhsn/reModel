//! The gamepad besides the game's own controls: as navigation (`PadNav`:
//! `Nav`s for the launcher and the menu, directions repeating while held),
//! and the gate between the pad and the game (`Gate`): the menu's buttons
//! (Guide, or Start and Select together) open the menu without the game
//! seeing them, and buttons pressed in the menu stay away from the game
//! until they're released.

use crate::launcher::Nav;
use gilrs::{Axis, Button};
use std::time::{Duration, Instant};

/// A gamepad event: from gilrs, or scripted (`--nav`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Pressed(Button),
    Released(Button),
    Axis(Axis, f32),
}

impl Event {
    pub fn from_gilrs(ev: &gilrs::EventType) -> Option<Event> {
        use gilrs::EventType;
        match *ev {
            EventType::ButtonPressed(b, _) => Some(Event::Pressed(b)),
            EventType::ButtonReleased(b, _) => Some(Event::Released(b)),
            EventType::AxisChanged(a, v, _) => Some(Event::Axis(a, v)),
            _ => None,
        }
    }
}

/// The gamepad as `Nav`s: buttons, and the d-pad and left stick repeating
/// while held.
#[derive(Default)]
pub struct PadNav {
    /// The direction held (by the stick or d-pad), since when, and when it
    /// last repeated.
    held: Option<(Nav, Instant, Instant)>,
    stick: (f32, f32),
    dpad: Option<Nav>,
}

/// First repeat of a held direction, then every `REPEAT`.
pub const REPEAT_DELAY: Duration = Duration::from_millis(380);
pub const REPEAT: Duration = Duration::from_millis(110);

impl PadNav {
    /// What `ev` asks for, if anything (directions also repeat: `repeat`).
    pub fn event(&mut self, ev: Event, now: Instant) -> Option<Nav> {
        match ev {
            Event::Pressed(b) => {
                let nav = match b {
                    Button::South => Nav::Accept,
                    Button::Start => Nav::Start,
                    Button::East => Nav::Back,
                    Button::Select => Nav::View,
                    Button::North => Nav::Continue,
                    Button::West => Nav::Saves,
                    Button::LeftTrigger => Nav::PrevGroup,
                    Button::RightTrigger => Nav::NextGroup,
                    Button::LeftTrigger2 => Nav::PrevFilter,
                    Button::RightTrigger2 => Nav::NextFilter,
                    Button::DPadLeft => Nav::Left,
                    Button::DPadRight => Nav::Right,
                    Button::DPadUp => Nav::Up,
                    Button::DPadDown => Nav::Down,
                    _ => return None,
                };
                if matches!(nav, Nav::Left | Nav::Right | Nav::Up | Nav::Down) {
                    self.dpad = Some(nav);
                    self.held = Some((nav, now, now));
                }
                Some(nav)
            }
            Event::Released(b) => {
                let nav = match b {
                    Button::DPadLeft => Nav::Left,
                    Button::DPadRight => Nav::Right,
                    Button::DPadUp => Nav::Up,
                    Button::DPadDown => Nav::Down,
                    _ => return None,
                };
                if self.dpad == Some(nav) {
                    self.dpad = None;
                    self.held = None;
                }
                None
            }
            Event::Axis(axis, v) => {
                match axis {
                    Axis::LeftStickX => self.stick.0 = v,
                    Axis::LeftStickY => self.stick.1 = v,
                    _ => return None,
                }
                let (x, y) = self.stick;
                // Up is positive in gilrs.
                let dir = if x.abs() > 0.6 && x.abs() > y.abs() {
                    Some(if x < 0.0 { Nav::Left } else { Nav::Right })
                } else if y.abs() > 0.6 {
                    Some(if y > 0.0 { Nav::Up } else { Nav::Down })
                } else {
                    None
                };
                let held = self.held.map(|h| h.0);
                if dir.is_some() && dir != held {
                    self.held = dir.map(|d| (d, now, now));
                    return dir;
                }
                if dir.is_none() && x.abs() < 0.4 && y.abs() < 0.4 && self.dpad.is_none() {
                    self.held = None;
                }
                None
            }
        }
    }

    /// The held direction again, when it's due to repeat.
    pub fn repeat(&mut self, now: Instant) -> Option<Nav> {
        let (nav, since, last) = self.held?;
        let due = if last == since { since + REPEAT_DELAY } else { last + REPEAT };
        if now >= due {
            self.held = Some((nav, since, now));
            return Some(nav);
        }
        None
    }
}

/// Start or Select alone waits this long for the other before the game
/// gets it (pressed within it, the two open the menu).
pub const TOGETHER: Duration = Duration::from_millis(100);
/// Start and Select held together this long open the menu.
pub const HOLD: Duration = Duration::from_millis(500);
/// A tap of Start or Select shorter than `TOGETHER` reaches the game
/// afterwards, held for this long.
pub const TAP: Duration = Duration::from_millis(100);

/// Start or Select, as the game sees it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Held {
    #[default]
    Up,
    /// Pressed, not passed on yet (it may be the menu's chord).
    Waiting(Instant),
    /// Passed on while held.
    Game,
    /// Released while waiting: passed on until then.
    Tap(Instant),
}

const PAIR: [Button; 2] = [Button::Start, Button::Select];

/// Between the last used pad and the game: which of its buttons the game
/// sees, and when the pad opens or closes the menu.
#[derive(Default)]
pub struct Gate {
    /// Start and Select.
    pair: [Held; 2],
    /// Since when both have been held (not kept).
    both_since: Option<Instant>,
    /// Kept from the game until released.
    kept: Vec<Button>,
}

fn pair_index(b: Button) -> Option<usize> {
    PAIR.iter().position(|&p| p == b)
}

impl Gate {
    /// `b` was pressed (`open`: with the menu open): true if it opens or
    /// closes the menu. Pressed in the menu, the game doesn't see it until
    /// it's released.
    pub fn press(&mut self, b: Button, now: Instant, open: bool) -> bool {
        if b == Button::Mode {
            self.keep(b);
            return true;
        }
        if open {
            self.keep(b);
            return false;
        }
        let Some(i) = pair_index(b) else { return false };
        if let Held::Waiting(t) = self.pair[1 - i] {
            if now.saturating_duration_since(t) <= TOGETHER {
                self.chord();
                return true;
            }
        }
        self.kept.retain(|k| *k != b);
        self.pair[i] = Held::Waiting(now);
        false
    }

    /// `b` was released.
    pub fn release(&mut self, b: Button, now: Instant) {
        self.kept.retain(|k| *k != b);
        if let Some(i) = pair_index(b) {
            self.pair[i] = match self.pair[i] {
                Held::Waiting(_) => Held::Tap(now + TAP),
                held @ Held::Tap(_) => held,
                _ => Held::Up,
            };
        }
    }

    /// Time passing with the menu closed: true if Start and Select have
    /// been held together long enough to open it.
    pub fn tick(&mut self, now: Instant) -> bool {
        let live = |h: Held| matches!(h, Held::Waiting(_) | Held::Game);
        if live(self.pair[0]) && live(self.pair[1]) {
            let since = *self.both_since.get_or_insert(now);
            if now.saturating_duration_since(since) >= HOLD {
                self.chord();
                return true;
            }
        } else {
            self.both_since = None;
        }
        for i in 0..2 {
            self.pair[i] = match self.pair[i] {
                Held::Waiting(t) if !live(self.pair[1 - i]) && now.saturating_duration_since(t) >= TOGETHER => {
                    Held::Game
                }
                Held::Tap(until) if now >= until => Held::Up,
                held => held,
            };
        }
        false
    }

    /// The menu opened or closed: Start and Select held now are kept from
    /// the game until they're released.
    pub fn menu_changed(&mut self) {
        for (i, b) in PAIR.into_iter().enumerate() {
            if matches!(self.pair[i], Held::Waiting(_) | Held::Game) {
                self.keep(b);
            }
            self.pair[i] = Held::Up;
        }
        self.both_since = None;
    }

    /// Keep `b` from the game until it's released (pressed for something
    /// else: skipping the splash, the menu).
    pub fn keep(&mut self, b: Button) {
        if !self.kept.contains(&b) {
            self.kept.push(b);
        }
        if let Some(i) = pair_index(b) {
            self.pair[i] = Held::Up;
        }
    }

    fn chord(&mut self) {
        PAIR.into_iter().for_each(|b| self.keep(b));
        self.both_since = None;
    }

    /// Whether the game sees `b`, which the pad has `pressed` or not.
    pub fn passes(&self, b: Button, pressed: bool) -> bool {
        if self.kept.contains(&b) {
            return false;
        }
        match pair_index(b) {
            Some(i) => matches!(self.pair[i], Held::Game | Held::Tap(_)),
            None => pressed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn directions_repeat_while_held() {
        let mut p = PadNav::default();
        let t = Instant::now();
        assert_eq!(p.event(Event::Pressed(Button::DPadRight), t), Some(Nav::Right));
        assert_eq!(p.repeat(t + ms(100)), None);
        assert_eq!(p.repeat(t + REPEAT_DELAY), Some(Nav::Right));
        assert_eq!(p.repeat(t + REPEAT_DELAY + REPEAT), Some(Nav::Right));
        assert_eq!(p.event(Event::Released(Button::DPadRight), t), None);
        assert_eq!(p.repeat(t + Duration::from_secs(5)), None);
        // The stick: a direction once past the threshold, none while held.
        assert_eq!(p.event(Event::Axis(Axis::LeftStickX, -0.9), t), Some(Nav::Left));
        assert_eq!(p.event(Event::Axis(Axis::LeftStickX, -1.0), t), None);
        assert_eq!(p.event(Event::Axis(Axis::LeftStickX, 0.0), t), None);
        assert_eq!(p.event(Event::Pressed(Button::South), t), Some(Nav::Accept));
        assert_eq!(p.event(Event::Pressed(Button::Start), t), Some(Nav::Start));
        assert_eq!(p.event(Event::Pressed(Button::East), t), Some(Nav::Back));
        assert_eq!(p.event(Event::Pressed(Button::RightTrigger), t), Some(Nav::NextGroup));
        // The launcher's: Y continues, X the saves, the triggers the filter.
        assert_eq!(p.event(Event::Pressed(Button::North), t), Some(Nav::Continue));
        assert_eq!(p.event(Event::Pressed(Button::West), t), Some(Nav::Saves));
        assert_eq!(p.event(Event::Pressed(Button::LeftTrigger2), t), Some(Nav::PrevFilter));
        assert_eq!(p.event(Event::Pressed(Button::RightTrigger2), t), Some(Nav::NextFilter));
    }

    /// What the game sees of Start, each `step` from `t` to `until`.
    fn start_seen(g: &mut Gate, t: Instant, until: u64) -> Vec<bool> {
        (0..=until / 10)
            .map(|i| {
                let now = t + ms(i * 10);
                assert!(!g.tick(now));
                g.passes(Button::Start, true)
            })
            .collect()
    }

    #[test]
    fn guide_toggles_and_is_kept() {
        let mut g = Gate::default();
        let t = Instant::now();
        assert!(g.press(Button::Mode, t, false), "opens");
        assert!(g.press(Button::Mode, t + ms(500), true), "closes");
        g.release(Button::Mode, t + ms(600));
        assert!(!g.press(Button::South, t, false), "A is the game's");
        assert!(g.passes(Button::South, true));
    }

    #[test]
    fn a_brief_start_reaches_the_game() {
        let mut g = Gate::default();
        let t = Instant::now();
        assert!(!g.press(Button::Start, t, false));
        assert!(!g.passes(Button::Start, true), "waits for Select a moment");
        g.release(Button::Start, t + ms(40));
        assert!(!g.tick(t + ms(50)));
        assert!(g.passes(Button::Start, false), "then the game sees the tap");
        assert!(!g.tick(t + ms(40) + TAP));
        assert!(!g.passes(Button::Start, false), "for a while");
    }

    #[test]
    fn a_held_start_reaches_the_game_while_held() {
        let mut g = Gate::default();
        let t = Instant::now();
        g.press(Button::Start, t, false);
        let seen = start_seen(&mut g, t, 400);
        assert!(!seen[0] && !seen[5]);
        assert!(seen[10..].iter().all(|s| *s), "from TOGETHER on");
        g.release(Button::Start, t + ms(410));
        assert!(!g.passes(Button::Start, false));
        // Select alone too, the coin on many pads.
        g.press(Button::Select, t + ms(500), false);
        assert!(!g.tick(t + ms(700)));
        assert!(g.passes(Button::Select, true));
    }

    #[test]
    fn start_and_select_together_open_the_menu() {
        let mut g = Gate::default();
        let t = Instant::now();
        assert!(!g.press(Button::Select, t, false));
        assert!(g.press(Button::Start, t + ms(60), false), "pressed together");
        // Neither reaches the game, even held after the menu closes.
        assert!(!g.tick(t + Duration::from_secs(2)));
        assert!(!g.passes(Button::Start, true) && !g.passes(Button::Select, true));
        g.release(Button::Start, t + Duration::from_secs(3));
        g.press(Button::Start, t + Duration::from_secs(4), false);
        assert!(!g.tick(t + Duration::from_secs(5)), "Select is still kept: no chord");
        assert!(g.passes(Button::Start, true) && !g.passes(Button::Select, true));
    }

    #[test]
    fn start_then_select_held_open_the_menu() {
        let mut g = Gate::default();
        let t = Instant::now();
        g.press(Button::Start, t, false);
        assert!(!g.tick(t + ms(200)));
        assert!(g.passes(Button::Start, true), "Start alone went to the game");
        assert!(!g.press(Button::Select, t + ms(300), false), "too late to be together");
        assert!(!g.tick(t + ms(310)));
        assert!(!g.passes(Button::Select, true), "Select waits while Start is held");
        assert!(!g.tick(t + ms(700)));
        assert!(g.tick(t + ms(820)), "held together for HOLD");
        assert!(!g.passes(Button::Start, true) && !g.passes(Button::Select, true));
    }

    #[test]
    fn start_held_select_tapped_is_a_coin() {
        let mut g = Gate::default();
        let t = Instant::now();
        g.press(Button::Start, t, false);
        g.tick(t + ms(200));
        g.press(Button::Select, t + ms(300), false);
        g.tick(t + ms(320));
        g.release(Button::Select, t + ms(380));
        assert!(!g.tick(t + ms(390)));
        assert!(g.passes(Button::Select, false), "the tap reaches the game");
        assert!(g.passes(Button::Start, true));
    }

    #[test]
    fn buttons_pressed_in_the_menu_are_kept_until_released() {
        let mut g = Gate::default();
        let t = Instant::now();
        assert!(!g.press(Button::South, t, true));
        assert!(!g.press(Button::Start, t, true));
        g.menu_changed();
        assert!(!g.passes(Button::South, true), "A that resumed stays away");
        assert!(!g.tick(t + Duration::from_secs(1)));
        assert!(!g.passes(Button::Start, true), "so does Start");
        g.release(Button::South, t + Duration::from_secs(1));
        assert!(g.passes(Button::South, true), "pressed again, it's the game's");
        // Start held through the menu opening (by the keyboard) is kept too.
        g.release(Button::Start, t);
        g.press(Button::Start, t, false);
        g.tick(t + ms(200));
        g.menu_changed();
        assert!(!g.passes(Button::Start, true));
    }
}
