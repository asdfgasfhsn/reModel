//! The communication (link) board some cabinets carry in an expansion slot
//! at `0x1A10000` (shared RAM from there, status registers at
//! `0x1A14000`). Which board a game has fitted, if any, is in its manifest
//! (`romset::LinkBoard`); with none the slot reads as all ones, "no board".
//!
//! The model answers what the game's own code checks and nothing of the
//! link protocol itself (`docs/games/von.md`): the register reads that tell
//! a game the board is fitted and idle. There is no other cabinet on the
//! link, so a game set to a linked ID (master, slave or relay) waits for
//! one as it would on the real board with nothing plugged in. The
//! experimental [`LinkHub`] joins several machines run in lockstep, the
//! way `remodel link-run` does.

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
    /// Joined to a [`LinkHub`] (experimental, `docs/hw/link-board.md`): the
    /// board's registers record what the game does and the hub answers.
    pub hub: bool,
    /// The game has written 1 to `0x1A14000` (the board is started) and,
    /// as the master does, 1 to `0x1A14002`; and the frame it started at.
    pub started: bool,
    pub master: bool,
    pub start_frame: u64,
}

impl Link {
    pub fn new(kind: LinkBoard) -> Self {
        Link {
            kind,
            fake: None,
            delay: 0,
            freeze_at: 0,
            drop_at: 0,
            history: Default::default(),
            hub: false,
            started: false,
            master: false,
            start_frame: 0,
        }
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

    /// A board joined to a [`LinkHub`].
    pub fn joined(mut self) -> Self {
        self.hub = true;
        self
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

/// Where the link RAM starts, the game's own block in it, and the received
/// blocks (`docs/hw/link-board.md` section 4).
pub const RAM: u32 = 0x01a1_0000;
pub const TX: usize = 0x2000;
pub const RX: usize = 0x21c0;
pub const BLOCK: usize = 0x1c0;

/// What the hub did at a frame boundary, for the harness's log.
pub type HubLog = Vec<String>;

/// The link board of a cabinet group, stepped by the host in lockstep
/// (experimental, `docs/hw/link-board.md` section 8a). After every machine
/// has run a frame, [`LinkHub::exchange`] reads each game's own block
/// and fills each machine's received blocks in the order [`source`] gives:
/// the node's own block last, at once, the others `delay` frames after
/// they were sent (1: as soon as the echo). The ready byte, the node's ID
/// (1-based: the master is 1) and the node count at `0x1A10000`,
/// `0x1A10002` and `0x1A10003` come from the nodes' board starts.
#[derive(Clone, Debug)]
pub struct LinkHub {
    pub nodes: usize,
    pub delay: usize,
    /// Per receiving node, the delay of the other nodes' blocks it gets (all `delay`
    /// unless a test sets them apart).
    pub delays: Vec<usize>,
    /// Which node's block goes in received block `k` of node `ID` (a test of the layout):
    /// 0 `(ID + k) mod N` (the first guess, own block first), 1 `(ID - k) mod N`, 2 `k` (by node ID, own included), 3 `(ID + k + 1) mod N` with the ID register 1-based (own block last): the default, the one the game works with.
    pub layout: u8,
    /// Stop reporting ready from this frame (0: never): a test.
    pub drop_at: u64,
    /// From this frame this node's blocks stop reaching the others (a
    /// partner that went silent): a test.
    pub freeze: Option<(usize, u64)>,
    /// Node IDs to hand out in place of the master-first order (a test).
    pub fixed_ids: Option<Vec<u8>>,
    ids: Vec<u8>,
    ready: bool,
    started: Vec<bool>,
    /// Per node, its last `delay` blocks, newest last.
    hist: Vec<std::collections::VecDeque<Vec<u8>>>,
}

type Bus = crate::bus::Model2Bus;

/// The node (by ID, 0-based) whose block goes in received block `k` of the node `me`, among `n`.
/// Layout 3 is the one `srallyc` works with (`docs/hw/link-board.md` section 4): the node's own
/// block comes last and the master (ID 0) is where the game looks for it, at block
/// `(n - (me + 1)) mod n` (the ID register is 1-based).
pub fn source(layout: u8, me: usize, k: usize, n: usize) -> usize {
    match layout {
        1 => (me + n - k) % n,
        2 => k,
        3 => (me + k + 1) % n,
        _ => (me + k) % n,
    }
}

impl LinkHub {
    pub fn new(nodes: usize, delay: usize) -> Self {
        assert!((1..=4).contains(&nodes), "1 to 4 nodes");
        LinkHub {
            nodes,
            delay: delay.max(1),
            delays: vec![delay.max(1); nodes],
            layout: 3,
            drop_at: 0,
            freeze: None,
            fixed_ids: None,
            ids: vec![0; nodes],
            ready: false,
            started: vec![false; nodes],
            hist: vec![Default::default(); nodes],
        }
    }

    /// The ID the hub gave node `i` (valid once ready).
    pub fn id(&self, i: usize) -> u8 {
        self.ids[i]
    }

    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// One frame boundary: every machine has run frame `frame - 1`.
    pub fn exchange(&mut self, frame: u64, buses: &mut [&mut Bus]) -> HubLog {
        let mut log = HubLog::new();
        assert_eq!(buses.len(), self.nodes);
        let flags: Vec<(bool, bool, u64)> = buses
            .iter()
            .map(|b| b.link.as_ref().map_or((false, false, 0), |l| (l.started, l.master, l.start_frame)))
            .collect();
        for (i, f) in flags.iter().enumerate() {
            if f.0 != self.started[i] {
                log.push(format!(
                    "node {i} {} the board{}",
                    if f.0 { "starts" } else { "stops" },
                    if f.1 && f.0 { " (as master)" } else { "" }
                ));
                self.started[i] = f.0;
            }
        }
        let all = flags.iter().all(|f| f.0) && !(self.drop_at != 0 && frame >= self.drop_at);
        if all && !self.ready {
            // The master is 0, the others in the order they started.
            let mut order: Vec<usize> = (0..self.nodes).collect();
            order.sort_by_key(|&i| (!flags[i].1, flags[i].2, i));
            for (id, &i) in order.iter().enumerate() {
                self.ids[i] = id as u8;
            }
            if let Some(f) = &self.fixed_ids {
                self.ids.copy_from_slice(f);
            }
            if flags.iter().filter(|f| f.1).count() != 1 {
                log.push("warning: not exactly one master among the started nodes".into());
            }
            self.ready = true;
            for h in &mut self.hist {
                h.clear();
            }
            log.push(format!("link ready: IDs {:?}, {} nodes", self.ids, self.nodes));
        } else if !all && self.ready {
            self.ready = false;
            log.push("link down".into());
        }
        let total = self.nodes as u8;
        for (i, b) in buses.iter_mut().enumerate() {
            b.link_set_ready(self.ready, self.ids[i] + u8::from(self.layout == 3), total);
        }
        if !self.ready {
            return log;
        }
        let tx: Vec<Vec<u8>> =
            buses.iter().map(|b| b.link_ram().map_or(vec![0; BLOCK], |r| r[TX..TX + BLOCK].to_vec())).collect();
        for (i, t) in tx.iter().enumerate() {
            if self.freeze.is_some_and(|(n, f)| n == i && frame >= f) {
                continue;
            }
            self.hist[i].push_back(t.clone());
            if self.hist[i].len() > self.delays.iter().copied().max().unwrap_or(1) {
                self.hist[i].pop_front();
            }
        }
        for (i, b) in buses.iter_mut().enumerate() {
            let me = self.ids[i] as usize;
            let n = self.nodes;
            for k in 0..n {
                let want = source(self.layout, me, k, n);
                let src = (0..n).find(|&j| self.ids[j] as usize == want).unwrap();
                if src == i {
                    b.link_rx_write(k, &tx[i]);
                    continue;
                }
                let d = self.delays[i];
                if self.hist[src].len() >= d {
                    let blk = self.hist[src][self.hist[src].len() - d].clone();
                    b.link_rx_write(k, &blk);
                }
            }
        }
        log
    }

    /// The hub's state and the nodes' board flags, for a save together with
    /// the machines' (the flags live in each bus's [`Link`], outside the
    /// machine's own state).
    pub fn save(&self, buses: &[&mut Bus]) -> Vec<u8> {
        let mut o = vec![self.nodes as u8, self.delay as u8, self.ready as u8];
        o.extend(self.delays.iter().map(|&d| d as u8));
        o.extend(self.drop_at.to_le_bytes());
        for (i, b) in buses.iter().enumerate() {
            let l = b.link.as_ref();
            o.extend([
                self.ids[i],
                self.started[i] as u8,
                l.is_some_and(|l| l.started) as u8,
                l.is_some_and(|l| l.master) as u8,
            ]);
            o.extend(l.map_or(0, |l| l.start_frame).to_le_bytes());
            o.push(self.hist[i].len() as u8);
            for h in &self.hist[i] {
                o.extend(h);
            }
        }
        o
    }

    pub fn load(data: &[u8], buses: &mut [&mut Bus]) -> Result<Self, String> {
        let short = || "link hub state truncated".to_string();
        let nodes = *data.first().ok_or_else(short)? as usize;
        let delay = *data.get(1).ok_or_else(short)? as usize;
        let ready = *data.get(2).ok_or_else(short)? != 0;
        if nodes != buses.len() {
            return Err(format!("the save has {nodes} nodes, the run {}", buses.len()));
        }
        let mut hub = LinkHub::new(nodes, delay);
        hub.ready = ready;
        for (i, d) in data.get(3..3 + nodes).ok_or_else(short)?.iter().enumerate() {
            hub.delays[i] = *d as usize;
        }
        let at0 = 3 + nodes;
        hub.drop_at = u64::from_le_bytes(data.get(at0..at0 + 8).ok_or_else(short)?.try_into().unwrap());
        let mut at = at0 + 8;
        for (i, b) in buses.iter_mut().enumerate() {
            let f = data.get(at..at + 13).ok_or_else(short)?;
            hub.ids[i] = f[0];
            hub.started[i] = f[1] != 0;
            if let Some(l) = b.link.as_mut() {
                (l.started, l.master) = (f[2] != 0, f[3] != 0);
                l.start_frame = u64::from_le_bytes(f[4..12].try_into().unwrap());
            }
            let n = f[12] as usize;
            at += 13;
            for _ in 0..n {
                hub.hist[i].push_back(data.get(at..at + BLOCK).ok_or_else(short)?.to_vec());
                at += BLOCK;
            }
        }
        Ok(hub)
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
    fn the_received_blocks_are_ordered_as_the_game_expects() {
        for n in 1..=4usize {
            for me in 0..n {
                // The node's own block is the last one received (the "TOTAL - 1" the game compares with).
                assert_eq!(source(3, me, n - 1, n), me);
                // The game looks for the master at block (TOTAL - ID) mod TOTAL, ID the 1-based register.
                let k = (n - (me + 1)) % n;
                assert_eq!(source(3, me, k, n), 0, "node {me} of {n}");
                // Every node appears once.
                let mut seen: Vec<usize> = (0..n).map(|k| source(3, me, k, n)).collect();
                seen.sort();
                assert_eq!(seen, (0..n).collect::<Vec<_>>());
            }
        }
    }

    #[test]
    fn an_idle_board_is_fitted_and_not_busy() {
        let s = Link::new(LinkBoard::Idle).status();
        assert_eq!(s & 0x01, 0, "bit 0 of 0x1A14000: board present");
        assert_eq!(s >> 16 & 0x80, 0, "bit 7 of 0x1A14002: not busy");
    }
}
