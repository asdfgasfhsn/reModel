//! Same-screen linked play (docs/play.md, "Link play"): two to four
//! machines of one game in one window, joined by a link hub
//! (`model2::link::LinkHub`) and stepped in lockstep, one controller each.
//!
//! Machine 0 is the session's own (`Session::machine`: its sound, saves,
//! catalogue and so on work as in single play); the others are kept here.
//! Each frame the machines run alongside each other, one thread each (the
//! frame is a barrier), then the hub carries their link blocks across
//! (`model2::link::LinkHub::exchange`); the hub's frame-boundary exchange is
//! the only thing the machines share, so the whole set stays deterministic
//! (the same inputs give the same states: `remodel link-run`'s combined
//! hash, which the player's `--frames` run prints too).
//!
//! What a game needs to be linked this way is its manifest's
//! `romset::LinkPlay`; this module knows nothing of any one game.

use crate::pad;
use model2::link::LinkHub;
use model2::Machine;
use std::path::PathBuf;

/// The games the player splits the screen for, and the most machines on
/// one screen (the layout's quadrants).
const SPLIT_GAMES: &[&str] = &["srallyc", "daytona", "manxtt", "indy500", "motoraid", "waverunr"];
pub const MAX_ON_SCREEN: usize = 4;

/// The link data of `id` if it can be played linked on one screen.
pub fn play_for(id: &str) -> Option<&'static romset::LinkPlay> {
    SPLIT_GAMES.contains(&romset::variants::title_of(id)).then(|| romset::games::link_play(id)).flatten()
}

/// The most machines of `play`'s game on one screen.
pub fn max_players(play: &romset::LinkPlay) -> usize {
    play.max_nodes.min(MAX_ON_SCREEN)
}

/// Machine `i`'s settings image (`base`: the settings the session starts from, see
/// `romset::LinkPlay::base_with_free_play`) with its role and car number set, loaded into
/// its bus where the game keeps them.
pub fn load_settings(bus: &mut model2::bus::Model2Bus, play: &romset::LinkPlay, base: Option<&[u8]>, i: usize) {
    let role = *play.roles.get(i).or(play.roles.last()).unwrap();
    let car: Vec<(usize, u8)> = play.car_off.map(|o| (o, i as u8)).into_iter().collect();
    let image = play.settings.image(base, play.role_off, role, &car);
    match play.settings.store {
        romset::SettingsStore::Eeprom => bus.eeprom.load(&image),
        romset::SettingsStore::IoBoard => bus.io_store_load(&image),
    }
    bus.link = Some(model2::link::Link::new(romset::LinkBoard::Idle).joined());
}

/// Whether a game starts on free play (the `free_play` setting, `--free-play`): the
/// game's own FREE PLAY coin setting (`romset::LinkPlay::free_play`), for the games
/// that have it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FreePlay {
    /// On in link play, where nobody wants to feed coins to a screen each, off in a
    /// single game.
    #[default]
    Auto,
    On,
    Off,
}

impl FreePlay {
    pub const ALL: [FreePlay; 3] = [FreePlay::Auto, FreePlay::On, FreePlay::Off];

    pub fn name(self) -> &'static str {
        match self {
            FreePlay::Auto => "auto",
            FreePlay::On => "on",
            FreePlay::Off => "off",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FreePlay::Auto => "Link play only",
            FreePlay::On => "Always",
            FreePlay::Off => "Never",
        }
    }

    pub fn parse(s: &str) -> Option<FreePlay> {
        let t = s.trim().to_lowercase();
        match t.as_str() {
            "true" | "yes" => Some(FreePlay::On),
            "false" | "no" => Some(FreePlay::Off),
            _ => FreePlay::ALL.into_iter().find(|f| f.name() == t),
        }
    }

    /// Free play for a game of `linked` play (two or more machines) or a single game.
    pub fn on(self, linked: bool) -> bool {
        match self {
            FreePlay::Auto => linked,
            FreePlay::On => true,
            FreePlay::Off => false,
        }
    }
}

/// How the views are laid out in the window (the `link_layout` setting).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    /// Two players: side by side or stacked, whichever gives each picture
    /// more room in this window (side by side on a 16:9 window, stacked on
    /// a tall one); three or four: quadrants.
    #[default]
    Auto,
    /// Two players side by side.
    Side,
    /// Two players one above the other.
    Stacked,
}

impl Layout {
    pub const ALL: [Layout; 3] = [Layout::Auto, Layout::Side, Layout::Stacked];

    pub fn name(self) -> &'static str {
        match self {
            Layout::Auto => "auto",
            Layout::Side => "side",
            Layout::Stacked => "stacked",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Layout::Auto => "Auto",
            Layout::Side => "Side by side",
            Layout::Stacked => "Stacked",
        }
    }

    pub fn parse(s: &str) -> Option<Layout> {
        Layout::ALL.into_iter().find(|l| l.name() == s.trim().to_lowercase())
    }
}

/// What the machines' sound does (the `link_audio` setting).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Audio {
    /// Player 1's machine at full level, the others quiet behind it (their
    /// engines and announcers still there, so a pass is heard).
    #[default]
    P1,
    /// Every machine, equally loud.
    Mix,
    /// Every machine, spread from left to right in player order.
    Pan,
    /// Player 1's machine only: the others' sound boards don't run at all
    /// (the cheapest).
    P1Only,
}

impl Audio {
    pub const ALL: [Audio; 4] = [Audio::P1, Audio::Mix, Audio::Pan, Audio::P1Only];

    pub fn name(self) -> &'static str {
        match self {
            Audio::P1 => "p1",
            Audio::Mix => "mix",
            Audio::Pan => "pan",
            Audio::P1Only => "p1only",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Audio::P1 => "Player 1 loudest",
            Audio::Mix => "All equal",
            Audio::Pan => "Spread left to right",
            Audio::P1Only => "Player 1 only",
        }
    }

    pub fn parse(s: &str) -> Option<Audio> {
        Audio::ALL.into_iter().find(|a| a.name() == s.trim().to_lowercase())
    }

    /// The left and right gain of seat `seat` of `n`, or None if its sound
    /// board doesn't run.
    pub fn gains(self, seat: usize, n: usize) -> Option<(f32, f32)> {
        match self {
            Audio::P1Only => (seat == 0).then_some((1.0, 1.0)),
            Audio::P1 => Some(if seat == 0 { (1.0, 1.0) } else { (0.3, 0.3) }),
            Audio::Mix => {
                let g = 1.0 / (n as f32).sqrt();
                Some((g, g))
            }
            Audio::Pan => {
                // Equal power across the stereo field.
                let at = if n > 1 { seat as f32 / (n - 1) as f32 } else { 0.5 };
                let a = at * std::f32::consts::FRAC_PI_2;
                Some((a.cos() * 0.85, a.sin() * 0.85))
            }
        }
    }
}

/// A rectangle in the window, in pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Space left between the views and round the window's edge (pixels).
const GAP: u32 = 3;

/// The window's `w` x `h` pixels divided for `n` machines: a cell for each,
/// and for three the fourth quadrant, empty.
pub fn cells(n: usize, win: (u32, u32), layout: Layout) -> (Vec<Cell>, Option<Cell>) {
    let (w, h) = win;
    let grid = |cols: u32, rows: u32| -> Vec<Cell> {
        let (cw, ch) = (w / cols, h / rows);
        (0..rows * cols)
            .map(|i| {
                let (c, r) = (i % cols, i / cols);
                Cell { x: c * cw + GAP / 2, y: r * ch + GAP / 2, w: cw.saturating_sub(GAP), h: ch.saturating_sub(GAP) }
            })
            .collect()
    };
    let one = |c: &Cell| {
        // The 4:3 picture's height in the cell.
        c.h.min(c.w * 3 / 4)
    };
    match n {
        0 | 1 => (vec![Cell { x: 0, y: 0, w, h }], None),
        2 => {
            let side = grid(2, 1);
            let stacked = grid(1, 2);
            let use_side = match layout {
                Layout::Side => true,
                Layout::Stacked => false,
                Layout::Auto => one(&side[0]) >= one(&stacked[0]),
            };
            (if use_side { side } else { stacked }, None)
        }
        _ => {
            let mut g = grid(2, 2);
            let empty = (n == 3).then(|| g.pop().unwrap());
            g.truncate(n.min(4));
            (g, empty)
        }
    }
}

/// The 4:3 picture's place in `cell`: its height in pixels, and its top-left
/// corner (centred).
pub fn fit(cell: Cell) -> (u32, (u32, u32)) {
    let vh = cell.h.min(cell.w * 3 / 4).max(1);
    let vw = vh * 4 / 3;
    (vh, (cell.x + (cell.w.saturating_sub(vw)) / 2, cell.y + (cell.h - vh.min(cell.h)) / 2))
}

/// One player's place at the controls.
pub struct Seat {
    /// The gamepad that has joined (pressed a button), if any.
    pub pad: Option<gilrs::GamepadId>,
    /// Which of its buttons the game sees, and when it opens the menu.
    pub gate: pad::Gate,
    /// What its sticks hold as directions.
    pub dirs: romset::StickDirs,
}

/// The machines beyond the session's own, the hub that joins them, and the
/// players' seats.
pub struct LinkSet {
    /// Seats 1 to n-1 (seat 0 is `Session::machine`).
    pub machines: Vec<Machine>,
    pub hub: LinkHub,
    /// Their sound boards (None where the audio mode leaves them out), and
    /// seat 0's mode and level.
    pub sounds: Vec<Option<crate::sound::Sound>>,
    pub audio: Audio,
    pub seats: Vec<Seat>,
    /// The keyboard drives seat 0 (pads then take seats 1 up); else the
    /// pads take seats from 0.
    pub keyboard: bool,
    /// Where the set's own EEPROMs and saves are (never the player's own).
    pub dir: PathBuf,
    /// Per seat from 1, a scripted input (`--input`, `--input2`..).
    pub scripts: Vec<Option<model2::input::InputScript>>,
    /// Host time the last frame's machines took running (the slowest of
    /// them, the barrier) and the exchange.
    pub last_step: std::time::Duration,
    pub step_total: std::time::Duration,
    pub steps: u64,
    /// The slowest of the machines' own time in the last frames, summed.
    pub busiest_total: std::time::Duration,
}

impl LinkSet {
    pub fn new(machines: Vec<Machine>, hub: LinkHub, sounds: Vec<Option<crate::sound::Sound>>, dir: PathBuf) -> Self {
        let n = machines.len() + 1;
        LinkSet {
            scripts: (0..n - 1).map(|_| None).collect(),
            machines,
            hub,
            sounds,
            audio: Audio::default(),
            seats: (0..n)
                .map(|_| Seat { pad: None, gate: pad::Gate::default(), dirs: romset::StickDirs::default() })
                .collect(),
            keyboard: true,
            dir,
            last_step: Default::default(),
            step_total: Default::default(),
            steps: 0,
            busiest_total: Default::default(),
        }
    }

    /// How many machines, the session's own included.
    pub fn n(&self) -> usize {
        self.machines.len() + 1
    }

    /// The seat a gamepad plays in, if it has joined.
    pub fn seat_of(&self, id: gilrs::GamepadId) -> Option<usize> {
        self.seats.iter().position(|s| s.pad == Some(id))
    }

    /// A gamepad's first button press: it takes the first free seat the
    /// keyboard doesn't have (the first, with the keyboard off), if there is
    /// one.
    pub fn join(&mut self, id: gilrs::GamepadId) -> Option<usize> {
        let from = usize::from(self.keyboard);
        let at = (from..self.seats.len()).find(|&i| self.seats[i].pad.is_none())?;
        self.seats[at] = Seat { pad: Some(id), gate: pad::Gate::default(), dirs: romset::StickDirs::default() };
        Some(at)
    }

    /// A gamepad went away: its seat is free for the next to join.
    pub fn leave(&mut self, id: gilrs::GamepadId) -> Option<usize> {
        let at = self.seat_of(id)?;
        self.seats[at].pad = None;
        Some(at)
    }

    /// Set every sound board's level for the audio mode.
    pub fn apply_audio(&self, m0: Option<&crate::sound::Sound>) {
        let n = self.n();
        if let (Some(snd), Some((l, r))) = (m0, self.audio.gains(0, n)) {
            snd.set_gain(l, r);
        }
        for (i, s) in self.sounds.iter().enumerate() {
            if let (Some(snd), Some((l, r))) = (s, self.audio.gains(i + 1, n)) {
                snd.set_gain(l, r);
            }
        }
    }

    /// Run one frame of every machine (`m0` is seat 0's) and carry the link
    /// blocks across: each machine on its own thread, the frame a barrier.
    pub fn step(&mut self, m0: &mut Machine) -> Result<(), i960::cpu::Stop> {
        let t = std::time::Instant::now();
        let mut busiest = std::time::Duration::ZERO;
        let result = std::thread::scope(|sc| {
            let handles: Vec<_> = self
                .machines
                .iter_mut()
                .map(|m| {
                    let hand = Hand(m);
                    sc.spawn(move || {
                        let m = hand.into_inner();
                        let t = std::time::Instant::now();
                        run_frame(m).map(|_| t.elapsed())
                    })
                })
                .collect();
            let t0 = std::time::Instant::now();
            let first = run_frame(m0).map(|_| t0.elapsed());
            let mut r = first.map(|d| busiest = busiest.max(d));
            for h in handles {
                match h.join().expect("a machine's thread panicked") {
                    Ok(d) => busiest = busiest.max(d),
                    Err(e) => r = r.and(Err(e)),
                }
            }
            r
        });
        result?;
        let frame = m0.frames;
        let mut buses: Vec<&mut model2::bus::Model2Bus> =
            std::iter::once(&mut m0.bus).chain(self.machines.iter_mut().map(|m| &mut m.bus)).collect();
        for line in self.hub.exchange(frame, &mut buses) {
            println!("link          frame {frame}: {line}");
        }
        self.last_step = t.elapsed();
        self.step_total += self.last_step;
        self.busiest_total += busiest;
        self.steps += 1;
        Ok(())
    }

    /// The machines' sound link bytes to their sound boards (or dropped, for
    /// a machine whose board doesn't run).
    pub fn send_sound(&mut self) {
        for (m, s) in self.machines.iter_mut().zip(&mut self.sounds) {
            match s {
                Some(snd) => snd.send(&mut m.bus),
                None => m.bus.uart_log.clear(),
            }
        }
    }

    /// The whole set for a save: after the session's own machine's state,
    /// as its "live" tail. Seat 0's sound board state comes in as
    /// `sound0`.
    pub fn save(&mut self, m0: &mut Machine, sound0: Option<Vec<u8>>) -> Vec<u8> {
        let mut o = b"LINKSET1".to_vec();
        o.push(self.n() as u8);
        put(&mut o, &sound0.unwrap_or_default());
        for (m, s) in self.machines.iter_mut().zip(&self.sounds) {
            put(&mut o, &m.save_state());
            put(&mut o, &s.as_ref().map(|s| s.save()).unwrap_or_default());
        }
        let buses: Vec<&mut model2::bus::Model2Bus> =
            std::iter::once(&mut m0.bus).chain(self.machines.iter_mut().map(|m| &mut m.bus)).collect();
        let hub = self.hub.save(&buses);
        put(&mut o, &hub);
        o
    }

    /// Load what [`LinkSet::save`] wrote into the other machines and the
    /// hub (seat 0's machine is loaded already); returns seat 0's sound
    /// state.
    pub fn load(&mut self, m0: &mut Machine, data: &[u8]) -> Result<Vec<u8>, String> {
        let short = || "link save truncated".to_string();
        let rest = data.strip_prefix(b"LINKSET1").ok_or("not a link play save (a single machine's?)")?;
        let n = *rest.first().ok_or_else(short)? as usize;
        if n != self.n() {
            return Err(format!("the save is of {n} machines, this game has {}", self.n()));
        }
        let mut at = 1;
        let sound0 = take(rest, &mut at).ok_or_else(short)?.to_vec();
        for (m, s) in self.machines.iter_mut().zip(self.sounds.iter_mut()) {
            m.load_state(take(rest, &mut at).ok_or_else(short)?)?;
            let sound = take(rest, &mut at).ok_or_else(short)?;
            if let (Some(snd), false) = (s, sound.is_empty()) {
                snd.load(sound.to_vec(), &m.bus)?;
            }
        }
        let hub = take(rest, &mut at).ok_or_else(short)?;
        let mut buses: Vec<&mut model2::bus::Model2Bus> =
            std::iter::once(&mut m0.bus).chain(self.machines.iter_mut().map(|m| &mut m.bus)).collect();
        self.hub = LinkHub::load(hub, &mut buses)?;
        Ok(sound0)
    }

    /// FNV of every machine's state hash and the hub's state, the combined
    /// hash `remodel link-run` prints.
    pub fn combined_hash(&mut self, m0: &mut Machine) -> u64 {
        let fold = |mut h: u64, bytes: &[u8]| {
            for &b in bytes {
                h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
            }
            h
        };
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for m in std::iter::once(&mut *m0).chain(self.machines.iter_mut()) {
            m.bus.geo.sync();
            h = fold(h, &m.bus.state_hash().to_le_bytes());
        }
        let buses: Vec<&mut model2::bus::Model2Bus> =
            std::iter::once(&mut m0.bus).chain(self.machines.iter_mut().map(|m| &mut m.bus)).collect();
        fold(h, &self.hub.save(&buses))
    }
}

/// A machine handed to a thread for one frame.
///
/// `Machine` isn't `Send` only because the JIT's fast-page table holds raw
/// pointers into memory the same machine owns, and the interpreter's decode
/// cache holds `Rc`s no other machine sees. Nothing in a machine is tied to
/// the thread that made it (its only thread-local, the JIT pool's inline
/// compiler, is for jobs run where they were asked for and is made by the
/// thread that runs them) and nothing is shared between machines but the
/// compile pool's locked queue, so a machine used by one thread at a time,
/// handed over and back by the scope's spawn and join, is as safe as it is
/// on the thread that made it.
struct Hand<'a>(&'a mut Machine);

// SAFETY: see `Hand`.
unsafe impl Send for Hand<'_> {}

impl<'a> Hand<'a> {
    fn into_inner(self) -> &'a mut Machine {
        self.0
    }
}

/// Run `m` until its frame count moves on.
fn run_frame(m: &mut Machine) -> Result<(), i960::cpu::Stop> {
    let f0 = m.frames;
    while m.frames == f0 {
        m.run_slice(u64::MAX)?;
    }
    Ok(())
}

fn put(o: &mut Vec<u8>, b: &[u8]) {
    o.extend((b.len() as u64).to_le_bytes());
    o.extend(b);
}

fn take<'a>(d: &'a [u8], at: &mut usize) -> Option<&'a [u8]> {
    let n = u64::from_le_bytes(d.get(*at..*at + 8)?.try_into().ok()?) as usize;
    let s = d.get(*at + 8..*at + 8 + n)?;
    *at += 8 + n;
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_split_screen_game_has_link_data_for_two_to_four_machines() {
        for id in SPLIT_GAMES {
            let p = play_for(id).unwrap_or_else(|| panic!("{id}: no link data"));
            assert!((2..=MAX_ON_SCREEN).contains(&max_players(p)), "{id}");
        }
        assert!(play_for("vf2").is_none());
        // A variant links as its parent does where its record is the parent's, and not otherwise.
        for id in ["manxttc", "srallycb", "srallyp", "daytonat", "daytonata"] {
            assert!(play_for(id).is_some(), "{id}");
        }
        assert!(play_for("topskatrj").is_none());
    }

    #[test]
    fn two_players_side_by_side_on_a_wide_window_and_stacked_on_a_tall_one() {
        let (c, empty) = cells(2, (1920, 1080), Layout::Auto);
        assert_eq!((c.len(), empty), (2, None));
        assert!(c[0].x + c[0].w <= c[1].x, "side by side: {c:?}");
        assert_eq!(fit(c[0]).0, (960 - GAP) * 3 / 4, "a 957x1077 cell holds a 957x717 picture");
        let (c, _) = cells(2, (1080, 1920), Layout::Auto);
        assert!(c[0].y + c[0].h <= c[1].y, "stacked on a tall window: {c:?}");
        // The setting overrides.
        let (c, _) = cells(2, (1920, 1080), Layout::Stacked);
        assert!(c[0].y + c[0].h <= c[1].y);
        assert_eq!(fit(c[0]).0, 540 - GAP);
    }

    #[test]
    fn three_and_four_players_take_quadrants_and_three_leave_one_empty() {
        let (c, empty) = cells(3, (1920, 1080), Layout::Auto);
        assert_eq!(c.len(), 3);
        let e = empty.expect("an empty fourth quadrant");
        assert!(e.x > c[0].x && e.y > c[0].y, "bottom right: {e:?}");
        let (c, empty) = cells(4, (1920, 1080), Layout::Auto);
        assert_eq!((c.len(), empty), (4, None));
        // No two cells overlap and all are inside the window.
        for (i, a) in c.iter().enumerate() {
            assert!(a.x + a.w <= 1920 && a.y + a.h <= 1080);
            for b in &c[i + 1..] {
                assert!(a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y);
            }
        }
        assert_eq!(fit(c[0]).0, 540 - GAP, "a 957x537 quadrant holds a 4:3 picture 537 high");
    }

    #[test]
    fn a_picture_is_four_to_three_and_centred_in_its_cell() {
        let cell = Cell { x: 100, y: 50, w: 900, h: 300 };
        let (vh, (ox, oy)) = fit(cell);
        assert_eq!((vh, oy), (300, 50));
        assert_eq!(ox, 100 + (900 - 400) / 2);
    }

    #[test]
    fn audio_modes_set_each_machines_level() {
        assert_eq!(Audio::P1.gains(0, 3), Some((1.0, 1.0)));
        assert!(Audio::P1.gains(1, 3).unwrap().0 < 0.5);
        assert_eq!(Audio::P1Only.gains(1, 2), None);
        let (l0, r0) = Audio::Pan.gains(0, 4).unwrap();
        let (l3, r3) = Audio::Pan.gains(3, 4).unwrap();
        assert!(l0 > r0 && r3 > l3, "spread from the left to the right");
        let (a, b) = (Audio::Mix.gains(0, 4).unwrap(), Audio::Mix.gains(3, 4).unwrap());
        assert_eq!(a, b);
        for x in Audio::ALL {
            assert_eq!(Audio::parse(x.name()), Some(x));
        }
        for x in Layout::ALL {
            assert_eq!(Layout::parse(x.name()), Some(x));
        }
    }
}
