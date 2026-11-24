//! One cabinet, several players (docs/play.md, "Players"): the seats of
//! `seats::Seats` read onto the game's controls. Seat N's device reads the
//! game's bindings for player N (`seats::player_bindings`: the pad and keys
//! of player 1's layout on player 2's buttons), and a gamepad has its own
//! gate, sticks and steering filter, as in link play. A game with a single
//! player's controls, or seats left as they start (the keyboard in P1, no
//! pad seated), takes the player's usual path (`App::apply_inputs`) and
//! nothing here.

use crate::controls::{self, Devices, Tuning};
use crate::pad::Gate;
use crate::seats::{self, Seats, Who};
use crate::steer::SteerFilter;
use gilrs::{GamepadId, Gilrs};
use model2::input::{apply_gun, apply_player, held, GunState, PlayerInput, StickCursor};
use romset::{Bindings, StickDirs};
use std::collections::HashSet;
use winit::keyboard::KeyCode;

/// What a seated gamepad keeps between frames (seat N's, by index).
#[derive(Default)]
pub struct Cabinet {
    /// Between each seat's pad and the game: the menu's buttons.
    pub gates: [Gate; seats::MAX],
    dirs: [StickDirs; seats::MAX],
    steer: [SteerFilter; seats::MAX],
    sticks: [StickCursor; seats::MAX],
}

/// The mouse, for the gun games: where it aims in the view, its buttons, and whether it
/// has been moved over the window.
#[derive(Clone, Copy, Default)]
pub struct Mouse {
    pub aim: Option<(f32, f32)>,
    pub buttons: [bool; 2],
    pub used: bool,
}

/// What the player's usual path keeps for the devices no seat holds, and what this frame
/// reads.
pub struct Frame<'a> {
    pub keys: &'a HashSet<KeyCode>,
    pub gilrs: Option<&'a Gilrs>,
    pub seats: &'a Seats<GamepadId>,
    /// How many players the game seats.
    pub players: usize,
    /// The pad last used that holds no seat: it plays with the keyboard's P1 layout.
    pub loose_pad: Option<GamepadId>,
    pub loose_gate: &'a Gate,
    pub loose_dirs: &'a mut StickDirs,
    pub loose_steer: &'a mut SteerFilter,
    pub loose_stick: &'a mut StickCursor,
    pub tuning: Tuning,
    /// The menu is open over the game: the controls are the menu's.
    pub in_menu: bool,
    pub mouse: Mouse,
}

/// Whether the seats ask for more than the player's usual path gives: anything but the
/// keyboard in P1 and the other seats empty.
pub fn active<I: Copy + PartialEq>(seats: &Seats<I>, players: usize) -> bool {
    players >= 2 && (0..players).any(|s| seats.held(s) != if s == 0 { Some(Who::Keyboard) } else { None })
}

impl Cabinet {
    /// Set the game's controls from every device: the keyboard in the seat it holds,
    /// the loose pad in P1, and each seated
    /// pad in its seat.
    pub fn apply(&mut self, bus: &mut model2::bus::Model2Bus, base: &Bindings, f: Frame) {
        let none = HashSet::new();
        let kb = f.seats.keyboard_seat().filter(|&s| s < f.players);
        let gil = f.gilrs;
        let pad_of = |id: GamepadId| gil.map(|g| g.gamepad(id));
        let tuning = f.tuning;
        // Each source: the seat it plays for (None: the loose devices, player 1's layout).
        let mut sources: Vec<(Option<usize>, Bindings, Devices)> = Vec::new();
        {
            let pad = if f.in_menu { None } else { f.loose_pad.and_then(pad_of) };
            let dirs = controls::read_sticks(f.loose_dirs, pad.as_ref(), tuning);
            let raw = pad.as_ref().map(controls::raw_axes);
            let steer = f.loose_steer.step(raw.as_ref(), tuning);
            let keys = if f.in_menu || kb != Some(0) { &none } else { f.keys };
            sources.push((None, base.clone(), Devices { keys, pad, gate: f.loose_gate, tuning, dirs, steer }));
        }
        for seat in 0..f.players {
            let (keys, pad) = match f.seats.held(seat) {
                Some(Who::Pad(id)) if !f.in_menu => (&none, pad_of(id)),
                Some(Who::Keyboard) if seat > 0 && !f.in_menu => (f.keys, None),
                _ => continue,
            };
            let dirs = controls::read_sticks(&mut self.dirs[seat], pad.as_ref(), tuning);
            let raw = pad.as_ref().map(controls::raw_axes);
            let steer = self.steer[seat].step(raw.as_ref(), tuning);
            let devices = Devices { keys, pad, gate: &self.gates[seat], tuning, dirs, steer };
            sources.push((Some(seat), seats::player_bindings(base, seat), devices));
        }
        let all: Vec<(&Bindings, &dyn PlayerInput)> =
            sources.iter().map(|(_, b, d)| (b, d as &dyn PlayerInput)).collect();
        apply_player(bus, &all);
        let guns = bus.controls.guns.len().min(f.players);
        if guns == 0 {
            return;
        }
        let source = |seat: Option<usize>| sources.iter().find(|s| s.0 == seat);
        for g in 0..guns {
            match f.seats.held(g) {
                // A pad aims its own gun with the sticks.
                Some(Who::Pad(_)) => {
                    let Some((_, b, d)) = source(Some(g)) else { continue };
                    let mut state = self.sticks[g].update(d, base);
                    state.reload |= held(d, b.reload(g));
                    apply_gun(bus, g, state);
                }
                // The keyboard's seat is the mouse's; with the keyboard out, the first.
                who if who == Some(Who::Keyboard) || (who.is_none() && g == 0) => {
                    let Some((_, b, d)) = source(if who.is_some() { Some(g) } else { None }).or_else(|| source(None))
                    else {
                        continue;
                    };
                    let reload = held(d, b.reload(g));
                    if f.mouse.used && !f.in_menu && (who == Some(Who::Keyboard) || kb.is_none()) {
                        let [trigger, r] = f.mouse.buttons;
                        apply_gun(bus, g, GunState { at: f.mouse.aim, trigger, reload: r || reload });
                    } else if g == 0 {
                        // No mouse: the loose pad's sticks aim, as in a single player's game.
                        let mut state = f.loose_stick.update(d, base);
                        state.reload |= reload;
                        apply_gun(bus, g, state);
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seats::PadKey;

    fn pad(seats: &mut Seats<u32>, id: u32) {
        seats.sight(id, PadKey { name: format!("pad {id}"), uuid: format!("u{id}") }, String::new());
    }

    /// The usual path serves the usual seats: the keyboard in P1, nobody else.
    #[test]
    fn seats_as_they_start_take_the_usual_path() {
        let mut s = Seats::<u32>::default();
        assert!(!super::active(&s, 2));
        pad(&mut s, 1);
        assert!(!super::active(&s, 2));
        s.join_pad(1, 2);
        assert!(super::active(&s, 2), "a pad in P2");
        assert!(!super::active(&s, 1), "a game of one player");
        s.leave(1);
        s.leave(0);
        assert!(super::active(&s, 2), "the keyboard left");
    }
}
