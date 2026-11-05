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
}

impl Drive {
    pub fn new(kind: DriveBoard) -> Self {
        match kind {
            DriveBoard::SkiPlatform => Drive::Ski(SkiPlatform::default()),
        }
    }

    /// The game wrote a command byte.
    pub fn command(&mut self, v: u8) {
        match self {
            Drive::Ski(b) => b.command(v),
        }
    }

    /// The reply the game reads.
    pub fn reply(&self) -> u8 {
        match self {
            Drive::Ski(b) => b.reply,
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
        }
    }

    /// The last command written: what the board was last told to do.
    pub fn last_command(&self) -> u8 {
        match self {
            Drive::Ski(b) => b.last,
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

// The kind comes from the manifest, so a state only holds the board's own.
impl state::State for Drive {
    fn save(&self, w: &mut state::Writer) {
        match self {
            Drive::Ski(b) => b.save(w),
        }
    }
    fn load(&mut self, r: &mut state::Reader) -> Result<(), state::Error> {
        match self {
            Drive::Ski(b) => b.load(r),
        }
    }
}

state::fields!(SkiPlatform { reply, last, centre });

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
        let Drive::Ski(b) = &d;
        assert_eq!(b.centre, Some(0x7c));
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
