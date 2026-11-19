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
    /// Experimental scripted partner (`REMODEL_LINK=fake,ID,TOTAL`): the
    /// node ID and node total reported once the game starts the board.
    fake: Option<(u8, u8)>,
    /// The scripted partner's delay in frames, the frame it goes silent at
    /// (0: never), and the blocks sent so far (`REMODEL_LINK_DELAY`,
    /// `REMODEL_LINK_FREEZE`).
    delay: usize,
    freeze_at: u64,
    /// The frame the board stops reporting itself ready at (0: never).
    pub drop_at: u64,
    history: std::collections::VecDeque<Vec<u8>>,
}

impl Link {
    pub fn new(kind: LinkBoard) -> Self {
        Link { kind, fake: None, delay: 0, freeze_at: 0, drop_at: 0, history: Default::default() }
    }

    /// An idle board that, once the game writes 1 to `0x1A14000`, reports
    /// itself ready with this node ID and total (experimental).
    pub fn with_fake(mut self, id: u8, total: u8) -> Self {
        self.fake = Some((id, total));
        self
    }

    /// Frames of extra delay on what the partner sends, and the frame it
    /// stops sending at (0: never). Experimental.
    pub fn with_timing(mut self, delay: usize, freeze_at: u64) -> Self {
        (self.delay, self.freeze_at) = (delay, freeze_at);
        self
    }

    /// The partner's next delivery for the block just sent at `frame`:
    /// what was sent `delay` frames ago, or `None` while the pipe fills or
    /// after the partner went silent.
    pub fn deliver(&mut self, frame: u64, sent: &[u8]) -> Option<Vec<u8>> {
        self.history.push_back(sent.to_vec());
        if self.history.len() > self.delay + 1 {
            self.history.pop_front();
        }
        if self.freeze_at != 0 && frame >= self.freeze_at || self.history.len() < self.delay + 1 {
            return None;
        }
        self.history.front().cloned()
    }

    pub fn fake(&self) -> Option<(u8, u8)> {
        self.fake
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
    fn the_scripted_partner_delays_its_blocks_and_goes_silent() {
        let mut l = Link::new(LinkBoard::Idle).with_fake(0, 2).with_timing(2, 10);
        assert_eq!(l.deliver(0, &[1]), None, "the pipe is still filling");
        assert_eq!(l.deliver(1, &[2]), None);
        assert_eq!(l.deliver(2, &[3]), Some(vec![1]), "two frames late");
        assert_eq!(l.deliver(3, &[4]), Some(vec![2]));
        assert_eq!(l.deliver(10, &[5]), None, "silent from frame 10");
    }

    #[test]
    fn an_idle_board_is_fitted_and_not_busy() {
        let s = Link::new(LinkBoard::Idle).status();
        assert_eq!(s & 0x01, 0, "bit 0 of 0x1A14000: board present");
        assert_eq!(s >> 16 & 0x80, 0, "bit 7 of 0x1A14002: not busy");
    }
}
