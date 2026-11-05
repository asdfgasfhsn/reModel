//! Drive boards: the cabinet boards that move a platform or push back on a
//! wheel, which a game commands through the I/O chip. Which one a game has,
//! if any, is in its manifest (`romset::DriveBoard`); each kind speaks its
//! own protocol and is modelled here as far as the game's code shows what
//! it expects. Nothing physical moves, so the models answer as a healthy
//! board at rest would; the commands are kept for players that might want
//! to turn them into gamepad rumble.
//!
//! The I/O chip side is the same for every kind seen so far: the game
//! writes a command byte to register 4 (`0x1C00008`) and reads a reply
//! byte from register 3 (`0x1C00006`).

use romset::DriveBoard;

/// A drive board's state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Drive {
    Ski(SkiPlatform),
    Wheel(Wheel),
}

impl Drive {
    pub fn new(kind: DriveBoard) -> Self {
        match kind {
            DriveBoard::SkiPlatform => Drive::Ski(SkiPlatform::default()),
            DriveBoard::Wheel => Drive::Wheel(Wheel::default()),
        }
    }

    /// The game wrote a command byte.
    pub fn command(&mut self, v: u8) {
        match self {
            Drive::Ski(b) => b.command(v),
            Drive::Wheel(b) => b.command(v),
        }
    }

    /// The reply the game reads.
    pub fn reply(&self) -> u8 {
        match self {
            Drive::Ski(b) => b.reply,
            Drive::Wheel(_) => Wheel::REPLY,
        }
    }

    /// One line on the board's state, for reports.
    pub fn describe(&self) -> String {
        match self {
            Drive::Ski(b) => format!(
                "ski platform: last command {:#04x}, reply {:#04x}, centre {}",
                b.last,
                b.reply,
                b.centre.map_or("not sent".into(), |c| format!("{c:#04x}"))
            ),
            Drive::Wheel(b) => {
                format!("wheel: last command {:#04x}, recent {:02x?}, {} commands", b.last(), b.recent(), b.count)
            }
        }
    }

    /// The last command written: what the board was last told to do.
    pub fn last_command(&self) -> u8 {
        match self {
            Drive::Ski(b) => b.last,
            Drive::Wheel(b) => b.last(),
        }
    }
}

/// Sega Ski Super G's platform drive board. What the game's code shows
/// (README, "What's known about skisuprg"):
///
/// - Boot: the game sends `0x00` until the reply is `0xCE`, then a byte
///   from its settings (the swing channel's calibrated centre) until the
///   reply is `0xFF`, then `0x00`; after 479 frames without that it stops
///   with "DRIVE BOARD TROUBLE CODE" and the last reply.
/// - Replies `0xB0`-`0xB7` are faults; `0xB8`/`0xB9` start a two-second
///   warning. After command `0xDC` (input test) each nibble of the reply is
///   a switch, all set reading "on"; after `0xA3`/`0xA4` (drive test) bit 4
///   is a status the test shows and follows.
/// - Commands seen: `0x10`-`0x1F` and `0x40` (during a run, with `0xD1`,
///   in turn each frame; the `0x1x` ones depend on the skis' angles),
///   `0x80` (at rest), `0xA0`-`0xA6` and `0xD0` (drive tests), `0xC2` /
///   `0xC3` (test mode, attract), `0xDC` (input test).
///
/// The model answers `0xCE` while it is told `0x00` and `0xFF`, idle with
/// no fault and every switch on, to everything else: that passes every
/// check the game makes. What the real board replies to the motion
/// commands is unknown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkiPlatform {
    /// Reply to the last command.
    pub reply: u8,
    /// Last command.
    pub last: u8,
    /// The byte the game sent after the `0x00`/`0xCE` handshake (the
    /// calibrated centre), once it has.
    pub centre: Option<u8>,
}

impl Default for SkiPlatform {
    fn default() -> Self {
        // Before any command: nothing a game treats as a fault.
        SkiPlatform { reply: 0xff, last: 0, centre: None }
    }
}

impl SkiPlatform {
    fn command(&mut self, v: u8) {
        if self.last == 0 && v != 0 && self.reply == 0xce {
            self.centre = Some(v);
        }
        self.last = v;
        self.reply = if v == 0 { 0xce } else { 0xff };
    }
}

/// A force feedback wheel board (Sega Touring Car Championship's). What
/// the game's code and runs show (README, "What's known about stcc"):
///
/// - Each frame the I/O routine reads a reply byte from `0x1C00006` and
///   writes the frame's command to `0x1C00008`, as `skisuprg` does, but the
///   game only stores the reply: nothing reads it back, so there is no
///   boot handshake and no fault check, and the game runs the same with
///   no board attached.
/// - Commands seen in runs: `0x00` through boot, `0x98` a while after it,
///   then `0x30` (attract, menus), `0x34` (selection screens), `0x38`
///   (racing, centred) and, while racing, `0x5x`/`0x6x` (by the run with
///   steering and collisions, the force effects). What each does on the
///   board is unconfirmed.
///
/// The model replies `0xFF` (what the port reads with nothing attached,
/// so runs are as without a board) and keeps the last commands, for a
/// player to turn into rumble.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Wheel {
    /// The last [`Wheel::HISTORY`] commands, oldest first.
    history: Vec<u8>,
    /// Commands received.
    pub count: u64,
}

impl Wheel {
    pub const REPLY: u8 = 0xff;
    pub const HISTORY: usize = 8;

    fn command(&mut self, v: u8) {
        if self.history.len() == Self::HISTORY {
            self.history.remove(0);
        }
        self.history.push(v);
        self.count += 1;
    }

    /// The last command, `0x00` before any.
    pub fn last(&self) -> u8 {
        self.history.last().copied().unwrap_or(0)
    }

    /// The last few commands, oldest first.
    pub fn recent(&self) -> &[u8] {
        &self.history
    }
}

// The kind comes from the manifest, so a state only holds the board's own.
impl state::State for Drive {
    fn save(&self, w: &mut state::Writer) {
        match self {
            Drive::Ski(b) => b.save(w),
            Drive::Wheel(b) => b.save(w),
        }
    }
    fn load(&mut self, r: &mut state::Reader) -> Result<(), state::Error> {
        match self {
            Drive::Ski(b) => b.load(r),
            Drive::Wheel(b) => b.load(r),
        }
    }
}

state::fields!(SkiPlatform { reply, last, centre });
state::fields!(Wheel { history, count });

#[cfg(test)]
mod tests {
    use super::*;

    /// The game's boot handshake, one command a frame with the reply read
    /// before each command is written, as its vblank handler does.
    #[test]
    fn ski_platform_passes_the_boot_handshake() {
        let mut d = Drive::new(DriveBoard::SkiPlatform);
        let mut cmd = 0u8;
        let mut step = 0;
        for _ in 0..10 {
            let reply = d.reply();
            d.command(cmd);
            match step {
                0 => (cmd, step) = (0, 1),
                1 if reply == 0xce => step = 2,
                2 => {
                    cmd = 0x7c;
                    if reply == 0xff {
                        step = 3;
                    }
                }
                3 => (cmd, step) = (0, 4),
                _ => {}
            }
        }
        assert_eq!(step, 4);
        let Drive::Ski(b) = &d else { panic!() };
        assert_eq!(b.centre, Some(0x7c));
    }

    #[test]
    fn wheel_keeps_recent_commands() {
        let mut d = Drive::new(DriveBoard::Wheel);
        assert_eq!(d.last_command(), 0);
        for v in 0..20u8 {
            d.command(v);
            assert_eq!(d.reply(), 0xff);
        }
        assert_eq!(d.last_command(), 19);
        let Drive::Wheel(b) = &d else { panic!() };
        assert_eq!(b.recent(), &[12, 13, 14, 15, 16, 17, 18, 19]);
        assert_eq!(b.count, 20);
        let bytes = state::to_bytes(&d);
        let mut e = Drive::new(DriveBoard::Wheel);
        state::State::load(&mut e, &mut state::Reader::new(&bytes)).unwrap();
        assert_eq!(d, e);
    }

    #[test]
    fn state_round_trips() {
        let mut d = Drive::new(DriveBoard::SkiPlatform);
        d.command(0);
        d.command(0x55);
        let bytes = state::to_bytes(&d);
        let mut e = Drive::new(DriveBoard::SkiPlatform);
        state::State::load(&mut e, &mut state::Reader::new(&bytes)).unwrap();
        assert_eq!(d, e);
    }
}
