//! Who plays which seat (docs/play.md, "Players"): the launcher-level model
//! of seats P1 to P4, each held by a device (the keyboard, or a gamepad),
//! and how a held seat reaches the game.
//!
//! * One cabinet: seat N drives the game's player-N controls. The game's
//!   bindings stay one set (a keyboard layout and a pad layout, made for
//!   player 1); the device in seat N reads that layout for player N
//!   ([`player_bindings`]): the manifest's player-2 buttons (`punch2`,
//!   `start2`, `coin2`, ...) take the pad and keys of player 1's
//!   counterparts ([`counterpart`]).
//! * Linked split screen: seat N plays machine N.
//!
//! The model is generic over the gamepad's id (gilrs' `GamepadId` in the
//! player) so it tests without a pad. A pad is remembered by its name and
//! gilrs' UUID, with the seat it held, so a known pad returns to its seat
//! when it is connected again; the memory is kept in `seats.txt` in the
//! data directory.

use romset::bindings::{Bindings, Slot, Value};
use romset::Controls;
use std::time::{Duration, Instant};

/// The most seats: the most machines on one screen.
pub const MAX: usize = 4;

/// How long a seat's lamp stays lit after its device was used.
pub const LAMP: Duration = Duration::from_millis(250);

/// A gamepad as remembered between runs: gilrs' name and UUID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PadKey {
    pub name: String,
    pub uuid: String,
}

/// A device that can hold a seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who<I> {
    Keyboard,
    Pad(I),
}

/// A gamepad connected now.
#[derive(Clone, Debug)]
struct Live<I> {
    id: I,
    key: PadKey,
    /// "Wired", "Wireless · 80%": a line under the name.
    detail: String,
}

pub struct Seats<I> {
    held: [Option<Who<I>>; MAX],
    live: Vec<Live<I>>,
    /// The seat each known pad held last, for when it comes back.
    memory: Vec<(PadKey, usize)>,
    /// The seat the keyboard returns to; None: it has left (and stays out
    /// until it takes a seat again).
    kb_pref: Option<usize>,
    /// The keyboard can take seats at all (`--link-keyboard off` for a
    /// session). Not remembered.
    kb_enabled: bool,
    used: [Option<Instant>; MAX],
    dirty: bool,
}

impl<I: Copy + PartialEq> Default for Seats<I> {
    fn default() -> Self {
        Seats {
            held: [Some(Who::Keyboard), None, None, None],
            live: Vec::new(),
            memory: Vec::new(),
            kb_pref: Some(0),
            kb_enabled: true,
            used: [None; MAX],
            dirty: false,
        }
    }
}

impl<I: Copy + PartialEq> Seats<I> {
    /// A pad the model has seen: connected, with its name and a line of detail. If it is
    /// remembered at a seat and that is free, it takes it (returned).
    pub fn sight(&mut self, id: I, key: PadKey, detail: String) -> Option<usize> {
        if let Some(l) = self.live.iter_mut().find(|l| l.id == id) {
            l.detail = detail;
            return None;
        }
        let at = self
            .memory
            .iter()
            .filter(|(k, _)| *k == key)
            .map(|&(_, s)| s)
            .find(|&s| self.held[s].is_none() && !self.kb_holds(s));
        self.live.push(Live { id, key, detail });
        if let Some(s) = at {
            self.held[s] = Some(Who::Pad(id));
        }
        at
    }

    fn kb_holds(&self, s: usize) -> bool {
        self.held[s] == Some(Who::Keyboard)
    }

    /// Whether the model has seen `id`.
    pub fn knows(&self, id: I) -> bool {
        self.live.iter().any(|l| l.id == id)
    }

    /// A pad went away: its seat is free for the next to join (its memory stays,
    /// so it returns there).
    pub fn unplug(&mut self, id: I) -> Option<usize> {
        self.live.retain(|l| l.id != id);
        let at = self.seat_of(Who::Pad(id))?;
        self.held[at] = None;
        Some(at)
    }

    pub fn held(&self, seat: usize) -> Option<Who<I>> {
        self.held.get(seat).copied().flatten()
    }

    pub fn seat_of(&self, who: Who<I>) -> Option<usize> {
        self.held.iter().position(|h| *h == Some(who))
    }

    /// The seat the keyboard holds.
    pub fn keyboard_seat(&self) -> Option<usize> {
        self.seat_of(Who::Keyboard)
    }

    /// How many of the first `n` seats are held.
    pub fn taken(&self, n: usize) -> usize {
        self.held.iter().take(n).filter(|h| h.is_some()).count()
    }

    /// The first free seat of the first `n`, or the one `pref` if that's free.
    fn free_seat(&self, n: usize, pref: Option<usize>) -> Option<usize> {
        let n = n.min(MAX);
        pref.filter(|&p| p < n && self.held[p].is_none()).or_else(|| (0..n).find(|&i| self.held[i].is_none()))
    }

    /// An unseated pad's button press takes the seat it had (if it is free) or the next
    /// free one of the first `n`. None: it is seated already, or the seats are full.
    pub fn join_pad(&mut self, id: I, n: usize) -> Option<usize> {
        if self.seat_of(Who::Pad(id)).is_some() {
            return None;
        }
        let key = self.live.iter().find(|l| l.id == id).map(|l| l.key.clone());
        let pref = key.as_ref().and_then(|k| {
            self.memory.iter().filter(|(m, _)| m == k).map(|&(_, s)| s).find(|&s| s < n && self.held[s].is_none())
        });
        let at = self.free_seat(n, pref)?;
        self.held[at] = Some(Who::Pad(id));
        self.remember(Who::Pad(id), at);
        Some(at)
    }

    /// The keyboard takes `seat` if it is free (and the keyboard is out).
    pub fn join_keyboard_at(&mut self, seat: usize, n: usize) -> bool {
        if !self.kb_enabled || self.keyboard_seat().is_some() || seat >= n.min(MAX) || self.held[seat].is_some() {
            return false;
        }
        self.held[seat] = Some(Who::Keyboard);
        self.remember(Who::Keyboard, seat);
        true
    }

    /// Whoever holds `seat` leaves it, and is not remembered there.
    pub fn leave(&mut self, seat: usize) -> Option<Who<I>> {
        let who = self.held.get_mut(seat)?.take()?;
        match who {
            Who::Keyboard => self.kb_pref = None,
            Who::Pad(_) => self.memory.retain(|&(_, s)| s != seat),
        }
        self.dirty = true;
        Some(who)
    }

    /// Whoever holds `from` moves to `to` of the first `n` seats, swapping with whoever
    /// holds that. False if there is nobody at `from` or `to` is out of range.
    pub fn move_seat(&mut self, from: usize, to: usize, n: usize) -> bool {
        let n = n.min(MAX);
        if from >= n || to >= n || from == to || self.held[from].is_none() {
            return false;
        }
        self.held.swap(from, to);
        for s in [from, to] {
            if let Some(who) = self.held[s] {
                self.remember(who, s);
            }
        }
        // (A seat that is now free forgets its old holder.)
        for s in [from, to] {
            if self.held[s].is_none() {
                self.memory.retain(|&(_, m)| m != s);
            }
        }
        true
    }

    /// `who` holds `seat` now: remember it there.
    fn remember(&mut self, who: Who<I>, seat: usize) {
        self.dirty = true;
        match who {
            Who::Keyboard => self.kb_pref = Some(seat),
            Who::Pad(id) => {
                let Some(key) = self.live.iter().find(|l| l.id == id).map(|l| l.key.clone()) else { return };
                // The seat's old claimant goes, and this pad's old seat.
                let old = self.memory.iter().position(|(k, s)| *k == key && self.held[*s] != Some(Who::Pad(id)));
                self.memory.retain(|&(_, s)| s != seat);
                if let Some(i) = old.filter(|&i| i < self.memory.len()) {
                    self.memory.remove(i);
                }
                self.memory.push((key, seat));
            }
        }
    }

    /// Let the keyboard take seats (or not, for a session: `--link-keyboard off`). Its
    /// seat is freed while it is off and given back when it is on again.
    pub fn set_keyboard(&mut self, on: bool) {
        if on == self.kb_enabled {
            return;
        }
        self.kb_enabled = on;
        if !on {
            if let Some(s) = self.keyboard_seat() {
                self.held[s] = None;
            }
        } else if let Some(p) = self.kb_pref {
            if self.held[p].is_none() {
                self.held[p] = Some(Who::Keyboard);
            }
        }
    }

    /// The device in `seat` was used now (its lamp lights).
    pub fn used(&mut self, who: Who<I>) {
        if let Some(s) = self.seat_of(who) {
            self.used[s] = Some(Instant::now());
        }
    }

    /// Whether `seat`'s device was used lately.
    pub fn lit(&self, seat: usize) -> bool {
        self.used.get(seat).copied().flatten().is_some_and(|t| t.elapsed() < LAMP)
    }

    /// The name and detail of the device in `seat`.
    pub fn describe(&self, seat: usize) -> Option<(String, String)> {
        match self.held(seat)? {
            Who::Keyboard => Some(("Keyboard".to_string(), String::new())),
            Who::Pad(id) => self.live.iter().find(|l| l.id == id).map(|l| (l.key.name.clone(), l.detail.clone())),
        }
    }

    /// The seats changed since last asked (to be saved).
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// As kept in `seats.txt`: the keyboard's seat (or `off`), then a line per remembered
    /// pad.
    pub fn to_text(&self) -> String {
        let mut t = format!("keyboard {}\n", self.kb_pref.map_or("off".to_string(), |s| (s + 1).to_string()));
        for (k, s) in &self.memory {
            t += &format!("pad {} {} {}\n", s + 1, k.uuid, k.name);
        }
        t
    }

    /// The seats as remembered (nobody connected yet; the keyboard's seat is taken).
    pub fn from_text(text: &str) -> Self {
        let mut seats = Seats::default();
        for line in text.lines() {
            let mut words = line.splitn(2, ' ');
            let rest = words.next().zip(words.next());
            match rest {
                Some(("keyboard", v)) => {
                    seats.kb_pref = v.trim().parse::<usize>().ok().filter(|s| (1..=MAX).contains(s)).map(|s| s - 1);
                }
                Some(("pad", v)) => {
                    let mut p = v.splitn(3, ' ');
                    let (Some(s), Some(uuid), Some(name)) = (p.next(), p.next(), p.next()) else { continue };
                    if let Some(s) = s.parse::<usize>().ok().filter(|s| (1..=MAX).contains(s)) {
                        seats.memory.push((PadKey { name: name.trim().to_string(), uuid: uuid.to_string() }, s - 1));
                    }
                }
                _ => {}
            }
        }
        seats.held = [None; MAX];
        if let Some(p) = seats.kb_pref {
            seats.held[p] = Some(Who::Keyboard);
        }
        seats
    }
}

impl Seats<gilrs::GamepadId> {
    fn file() -> std::path::PathBuf {
        crate::settings::data_dir().join("seats.txt")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::file()).map(|t| Self::from_text(&t)).unwrap_or_default()
    }

    pub fn save(&mut self) {
        if self.take_dirty() {
            let path = Self::file();
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, self.to_text());
        }
    }
}

/// A gamepad's key: gilrs' name and UUID.
pub fn pad_key(p: &gilrs::Gamepad) -> PadKey {
    let uuid: String = p.uuid().iter().map(|b| format!("{b:02x}")).collect();
    PadKey { name: p.name().to_string(), uuid }
}

/// A line of detail under a gamepad's name: how it is connected, and its charge.
pub fn pad_detail(p: &gilrs::Gamepad) -> String {
    use gilrs::PowerInfo;
    match p.power_info() {
        PowerInfo::Wired => "Wired".to_string(),
        PowerInfo::Discharging(pc) | PowerInfo::Charging(pc) => format!("Wireless · {pc}%"),
        PowerInfo::Charged => "Wireless · charged".to_string(),
        PowerInfo::Unknown => "Gamepad".to_string(),
    }
}

/// The keys of the game's keyboard layout in a few words ("Arrows, X V Z"), for the
/// keyboard's line on a seat.
pub fn keys_summary(base: &Bindings) -> String {
    let c = base.controls();
    let mut arrows = false;
    let mut keys: Vec<String> = Vec::new();
    for (i, b) in c.buttons.iter().enumerate() {
        if counterpart(c, b.name).is_none() && !is_second(c, b.name) || matches!(b.name, "coin1" | "start") {
            continue;
        }
        for input in base.button(i) {
            let romset::bindings::Input::Key(k) = input else { continue };
            if k.starts_with("Arrow") {
                arrows = true;
            } else {
                let short = k.strip_prefix("Key").or_else(|| k.strip_prefix("Digit")).unwrap_or(k).to_string();
                if !keys.contains(&short) && !is_second(c, b.name) {
                    keys.push(short);
                }
            }
        }
    }
    keys.truncate(4);
    match (arrows, keys.is_empty()) {
        (true, false) => format!("Arrows, {}", keys.join(" ")),
        (true, true) => "Arrows".to_string(),
        (false, _) => keys.join(" "),
    }
}

/// The button of player 2 that stands for `name`, a button of player 1's, if the game
/// has one: `punch` and `punch2`, `sw1` and `sw1_2`, `coin1` and `coin2`.
pub fn counterpart(controls: &Controls, name: &str) -> Option<&'static str> {
    let has = |n: &str| controls.buttons.iter().find(|b| b.name == n).map(|b| b.name);
    if let Some(n) = has(&format!("{name}_2")) {
        return Some(n);
    }
    if let Some(n) = has(&format!("{name}2")) {
        return Some(n);
    }
    // (Only these count down from 1: `view1` and `view2` are two views, not two players.)
    name.strip_suffix('1').filter(|stem| matches!(*stem, "coin" | "trigger")).and_then(|stem| has(&format!("{stem}2")))
}

/// Whether `name` is the player-2 half of a pair.
fn is_second(controls: &Controls, name: &str) -> bool {
    controls.buttons.iter().any(|b| counterpart(controls, b.name) == Some(name))
}

/// How many players one cabinet of the game seats: 2 if it has a second player's own
/// controls (more than a coin and a start button, or a second gun), else 1.
pub fn cabinet_players(controls: &Controls) -> usize {
    let extra = controls
        .buttons
        .iter()
        .filter_map(|b| counterpart(controls, b.name).map(|c| (b.name, c)))
        .any(|(n, _)| !matches!(n, "coin1" | "start"));
    if extra || controls.guns.len() > 1 {
        2
    } else {
        1
    }
}

/// The bindings the device in seat `player` (0-based) reads: the game's own with every
/// player-1 button's keys and pad buttons given to its player-`player+1` counterpart
/// (the player's own bindings of that counterpart, if they changed any, are kept),
/// and the player-1 buttons themselves let go. Seat 0 reads the bindings as they are.
/// Only two players are in the manifests; seats beyond them read nothing.
pub fn player_bindings(base: &Bindings, player: usize) -> Bindings {
    let mut b = base.clone();
    if player == 0 {
        return b;
    }
    let c = base.controls();
    let none = || Value::Inputs(Vec::new());
    for (i, button) in c.buttons.iter().enumerate() {
        let slot = Slot::Button(i);
        if is_second(c, button.name) {
            if player > 1 {
                b.set(slot, none());
            }
            continue;
        }
        let two = counterpart(c, button.name).filter(|_| player == 1);
        if let Some(j) = two.and_then(|t| c.buttons.iter().position(|x| x.name == t)) {
            if base.is_default(Slot::Button(j)) {
                if let Some(v) = base.get(slot).cloned() {
                    b.set(Slot::Button(j), v);
                }
            }
        }
        // (What has no player-2 half, a test or service switch, is player 1's.)
        b.set(slot, none());
    }
    for i in 0..c.analogs.len() {
        b.set(Slot::Axis(i), Value::Axis(romset::bindings::Axis::NONE));
        b.set(Slot::Neg(i), none());
        b.set(Slot::Pos(i), none());
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(name: &str) -> PadKey {
        PadKey { name: name.to_string(), uuid: format!("uuid-{}", name.replace(' ', "_")) }
    }

    fn model() -> Seats<u32> {
        Seats::default()
    }

    /// The keyboard starts in P1; pads take the next free seats in order of their first press.
    #[test]
    fn pads_join_in_order_of_their_first_press() {
        let mut s = model();
        s.sight(7, pad("a"), String::new());
        s.sight(8, pad("b"), String::new());
        assert_eq!(s.join_pad(8, 4), Some(1));
        assert_eq!(s.join_pad(7, 4), Some(2));
        assert_eq!(s.join_pad(7, 4), None, "seated already");
        assert_eq!(s.held(0), Some(Who::Keyboard));
        assert_eq!(s.taken(4), 3);
        assert_eq!(s.taken(2), 2);
    }

    #[test]
    fn full_seats_take_nobody_and_n_limits_them() {
        let mut s = model();
        for id in 1..=4 {
            s.sight(id, pad(&id.to_string()), String::new());
        }
        assert_eq!(s.join_pad(1, 2), Some(1));
        assert_eq!(s.join_pad(2, 2), None, "two seats, both held");
        assert_eq!(s.join_pad(2, 4), Some(2));
    }

    #[test]
    fn leaving_frees_a_seat_and_forgets_it() {
        let mut s = model();
        s.sight(1, pad("a"), String::new());
        assert_eq!(s.join_pad(1, 4), Some(1));
        assert_eq!(s.leave(1), Some(Who::Pad(1)));
        assert_eq!(s.held(1), None);
        // Known, but it left: it is not put back by itself, and it joins the first free.
        assert_eq!(s.join_pad(1, 4), Some(1));
        assert_eq!(s.leave(0), Some(Who::Keyboard));
        assert!(s.join_keyboard_at(0, 4));
    }

    #[test]
    fn moving_swaps_with_whoever_is_there() {
        let mut s = model();
        s.sight(1, pad("a"), String::new());
        s.join_pad(1, 4);
        assert!(s.move_seat(0, 1, 2));
        assert_eq!((s.held(0), s.held(1)), (Some(Who::Pad(1)), Some(Who::Keyboard)));
        assert!(s.move_seat(1, 3, 4), "to an empty seat");
        assert_eq!((s.held(1), s.held(3)), (None, Some(Who::Keyboard)));
        assert!(!s.move_seat(3, 4, 4) && !s.move_seat(3, 2, 2), "outside the seats");
        assert!(!s.move_seat(2, 0, 4), "nobody there");
        assert_eq!(s.keyboard_seat(), Some(3));
    }

    /// An unplugged pad frees its seat; the same pad (name and UUID) plugged in again
    /// returns to it, a different one doesn't.
    #[test]
    fn a_known_pad_returns_to_its_seat() {
        let mut s = model();
        s.sight(1, pad("a"), String::new());
        s.join_pad(1, 4);
        s.move_seat(1, 2, 4);
        assert_eq!(s.unplug(1), Some(2));
        assert_eq!(s.held(2), None);
        // Another pad meanwhile: not in seat 3.
        assert_eq!(s.sight(9, pad("other"), String::new()), None);
        // Back, under a new id.
        assert_eq!(s.sight(5, pad("a"), String::new()), Some(2));
        assert_eq!(s.held(2), Some(Who::Pad(5)));
    }

    #[test]
    fn a_remembered_seat_that_is_taken_is_not_stolen() {
        let mut s = model();
        s.sight(1, pad("a"), String::new());
        s.join_pad(1, 4);
        s.unplug(1);
        s.sight(2, pad("b"), String::new());
        assert_eq!(s.join_pad(2, 4), Some(1));
        assert_eq!(s.sight(3, pad("a"), String::new()), None);
        assert_eq!(s.join_pad(3, 4), Some(2));
    }

    #[test]
    fn the_seats_are_kept_between_runs() {
        let mut s = model();
        s.sight(1, pad("Xbox Wireless Controller"), String::new());
        s.sight(2, pad("DualSense"), String::new());
        s.join_pad(1, 4);
        s.join_pad(2, 4);
        s.leave(0);
        let text = s.to_text();
        let mut t = Seats::<u32>::from_text(&text);
        assert_eq!(t.keyboard_seat(), None, "the keyboard left");
        assert_eq!(t.sight(10, pad("DualSense"), String::new()), Some(2));
        assert_eq!(t.sight(11, pad("Xbox Wireless Controller"), String::new()), Some(1));
        assert_eq!(Seats::<u32>::from_text("garbage\nkeyboard 9\npad x y").to_text(), "keyboard off\n");
    }

    #[test]
    fn the_keyboard_can_be_kept_out_of_a_session() {
        let mut s = model();
        s.sight(1, pad("a"), String::new());
        s.set_keyboard(false);
        assert_eq!(s.keyboard_seat(), None);
        assert!(!s.join_keyboard_at(0, 4));
        assert_eq!(s.join_pad(1, 4), Some(0), "the pad takes seats from P1");
        s.set_keyboard(true);
        assert_eq!(s.keyboard_seat(), None, "the seat is taken");
        s.leave(0);
        assert!(s.join_keyboard_at(0, 4));
        assert!(!s.to_text().contains("off"));
    }

    #[test]
    fn a_lamp_lights_for_the_device_used() {
        let mut s = model();
        assert!(!s.lit(0));
        s.used(Who::Keyboard);
        assert!(s.lit(0) && !s.lit(1));
        s.used(Who::Pad(3));
        assert!(!s.lit(1));
    }

    /// Every game with a second player's buttons pairs them one to one, and a pair's
    /// second is never itself the first of another.
    #[test]
    fn players_two_buttons_pair_with_players_ones() {
        for g in romset::games::ALL {
            let c = &g.controls;
            let mut seen = Vec::new();
            for b in c.buttons {
                let Some(two) = counterpart(c, b.name) else { continue };
                assert!(!seen.contains(&two), "{}: {two} is the counterpart of two buttons", g.id);
                assert!(counterpart(c, two).is_none(), "{}: {two} pairs on", g.id);
                assert!(b.port <= 3, "{}", g.id);
                seen.push(two);
            }
        }
        let vf = &romset::games::find("vf2").unwrap().controls;
        assert_eq!(counterpart(vf, "punch"), Some("punch2"));
        assert_eq!(counterpart(vf, "coin1"), Some("coin2"));
        assert_eq!(counterpart(vf, "start"), Some("start2"));
        assert_eq!(counterpart(vf, "test"), None);
        let bb = &romset::games::find("dynabb97").unwrap().controls;
        assert_eq!(counterpart(bb, "sw1"), Some("sw1_2"));
        assert_eq!(counterpart(bb, "sw2"), Some("sw2_2"));
    }

    #[test]
    fn how_many_a_cabinet_seats() {
        let n = |id: &str| cabinet_players(&romset::games::find(id).unwrap().controls);
        assert_eq!((n("vf2"), n("doa"), n("dynabb97"), n("lastbrnx"), n("vcop"), n("hotd")), (2, 2, 2, 2, 2, 2));
        assert_eq!((n("daytona"), n("topskatr")), (1, 1));
        let two: Vec<&str> =
            romset::games::ALL.iter().filter(|g| cabinet_players(&g.controls) == 2).map(|g| g.id).collect();
        let want = ["hotd", "bel", "dynabb97", "doa", "fvipers", "schamp", "vstriker", "lastbrnx", "vf2", "vcop"];
        assert!(want.iter().all(|w| two.contains(w)), "{two:?}");
    }

    fn held_by(b: &Bindings, name: &str) -> Vec<romset::bindings::Input> {
        let c = b.controls();
        let i = c.buttons.iter().position(|x| x.name == name).unwrap();
        b.button(i).to_vec()
    }

    /// Seat 2 of Virtua Fighter 2 reads the pad and keys of player 1's controls for
    /// player 2's, and nothing for player 1's.
    #[test]
    fn seat_two_drives_player_twos_controls_with_player_ones_layout() {
        use romset::bindings::Input;
        let vf = &romset::games::find("vf2").unwrap().controls;
        let base = Bindings::defaults(vf);
        let seat1 = player_bindings(&base, 1);
        assert_eq!(held_by(&seat1, "punch2"), held_by(&base, "punch"));
        assert!(held_by(&seat1, "punch2").contains(&Input::Pad(romset::PadButton::South)));
        assert_eq!(held_by(&seat1, "left2"), held_by(&base, "left"));
        assert_eq!(held_by(&seat1, "start2"), held_by(&base, "start"));
        assert_eq!(held_by(&seat1, "coin2"), held_by(&base, "coin1"));
        assert!(held_by(&seat1, "punch").is_empty() && held_by(&seat1, "start").is_empty());
        // Seat 1 is the game's bindings as they are.
        let seat0 = player_bindings(&base, 0);
        assert_eq!(held_by(&seat0, "punch"), held_by(&base, "punch"));
        assert!(held_by(&seat0, "punch2").is_empty());
        // The player's own player-2 bindings win.
        let mut own = base.clone();
        own.set(
            Slot::Button(vf.buttons.iter().position(|b| b.name == "punch2").unwrap()),
            Value::Inputs(vec![Input::Key("KeyU".into())]),
        );
        assert_eq!(held_by(&player_bindings(&own, 1), "punch2"), vec![Input::Key("KeyU".into())]);
    }

    /// A seat beyond the manifest's two players reads nothing.
    #[test]
    fn a_third_seat_in_a_cabinet_reads_nothing() {
        let vf = &romset::games::find("vf2").unwrap().controls;
        let seat2 = player_bindings(&Bindings::defaults(vf), 2);
        for (slot, v) in seat2.slots() {
            if let Slot::Button(_) = slot {
                assert!(v.inputs().is_empty());
            }
        }
    }

    #[test]
    fn dynabb97_second_player_pairs_its_split_names() {
        let bb = &romset::games::find("dynabb97").unwrap().controls;
        let base = Bindings::defaults(bb);
        let seat1 = player_bindings(&base, 1);
        assert_eq!(held_by(&seat1, "sw1_2"), held_by(&base, "sw1"));
        assert_eq!(held_by(&seat1, "sw2_2"), held_by(&base, "sw2"));
    }

    /// A device: the keys held, and the pad buttons.
    struct Held(Vec<&'static str>, Vec<romset::PadButton>);

    impl model2::input::PlayerInput for Held {
        fn key(&self, code: &str) -> bool {
            self.0.contains(&code)
        }
        fn pad_button(&self, b: romset::PadButton) -> bool {
            self.1.contains(&b)
        }
        fn pad_axis(&self, _: romset::PadAxis) -> f32 {
            0.0
        }
    }

    /// What the game reads of a seat's device: `(player 1's punch, player 2's punch)`.
    fn punches(device: &Held, seat: usize) -> (bool, bool) {
        let vf = &romset::games::find("vf2").unwrap().controls;
        let b = player_bindings(&Bindings::defaults(vf), seat);
        let at = |n: &str| model2::input::held(device, b.button(vf.buttons.iter().position(|x| x.name == n).unwrap()));
        (at("punch"), at("punch2"))
    }

    /// Seat 1's device plays player 1 and seat 2's player 2, each with the game's one layout
    /// (the keyboard's X, the pad's South), so a pad in P2 and the keyboard in P1 play a
    /// cabinet together, and swapping the seats swaps the players.
    #[test]
    fn a_seats_device_plays_its_player_with_the_games_layout() {
        use romset::PadButton::South;
        let keyboard = Held(vec!["KeyX"], vec![]);
        let pad = Held(vec![], vec![South]);
        assert_eq!(punches(&keyboard, 0), (true, false));
        assert_eq!(punches(&pad, 0), (true, false));
        assert_eq!(punches(&pad, 1), (false, true), "the pad in P2");
        assert_eq!(punches(&keyboard, 1), (false, true), "the keyboard in P2");
        // A seat past the cabinet's two plays nothing.
        assert_eq!(punches(&pad, 2), (false, false));
    }
}
