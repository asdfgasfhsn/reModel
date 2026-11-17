//! The communication (link) board some cabinets carry in an expansion slot
//! at `0x1A10000` (shared RAM from there, status registers at
//! `0x1A14000`). Which board a game has fitted, if any, is in its manifest
//! (`romset::LinkBoard`); with none the slot reads as all ones, "no board".
//!
//! The model answers what the game's own code checks and nothing of the
//! link protocol itself (`docs/games/von.md`): the register reads that tell
//! a game the board is fitted and idle. There is no other cabinet on the
//! link, so a game set to a linked ID (master, slave or relay) waits for
//! one as it would on the real board with nothing plugged in.

use romset::LinkBoard;

/// A fitted link board.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    kind: LinkBoard,
}

impl Link {
    pub fn new(kind: LinkBoard) -> Self {
        Link { kind }
    }

    /// The word at `0x1A14000`: byte 0 is the status the game polls after
    /// writing 0 to it (bit 0 set means no board), byte 2 `0x1A14002`
    /// (bit 7 is the board's busy flag, waited on after the game writes 1
    /// to `0x1A14000`). Idle: both read 0.
    pub fn status(&self) -> u32 {
        match self.kind {
            LinkBoard::Idle => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_idle_board_is_fitted_and_not_busy() {
        let s = Link::new(LinkBoard::Idle).status();
        assert_eq!(s & 0x01, 0, "bit 0 of 0x1A14000: board present");
        assert_eq!(s >> 16 & 0x80, 0, "bit 7 of 0x1A14002: not busy");
    }
}
