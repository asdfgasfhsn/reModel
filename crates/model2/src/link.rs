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
    /// Bit 7 of `0x1A14002` flips every frame up to this frame (0: for ever): a test.
    pub beat_until: u64,
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
            beat_until: 0,
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

/// What one node showed the link at one frame boundary: the board flags
/// the game set (`started` is the write of 1 to `0x1A14000`, `master` of 1
/// to `0x1A14002`, and the frame it started at) and, while started, the
/// game's own block. This is all that crosses between machines, so it is
/// also what a network carries (`remodel-linknet`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NodeRec {
    pub started: bool,
    pub master: bool,
    pub start_frame: u64,
    /// The block (`LinkPlay::block` bytes), empty while the node's board is not started.
    pub tx: Vec<u8>,
}

/// The link board of a cabinet group, stepped by the host in lockstep
/// (experimental, `docs/hw/link-board.md` section 8a). After every machine
/// has run a frame, each node's record of the frame ([`LinkHub::put_local`])
/// is added and [`LinkHub::apply`] fills each local machine's received
/// blocks in the order [`source`] gives: the node's own block last, at once,
/// the others `delay` frames after they were sent (1: as soon as the echo, so
/// at boundary `F` a node sees the others' records of frame `F - (delay - 1)`).
/// The ready byte, the node's ID (1-based: the master is 1) and the node
/// count at `0x1A10000`, `0x1A10002` and `0x1A10003` come from the nodes'
/// board starts, **as each node sees them**: its own at once, the others'
/// `delay - 1` frames late like their blocks. So the hub works the same
/// whether every node is in this process ([`LinkHub::exchange`]) or the
/// records of some come over a network, a node needing only the records its
/// view asks for ([`LinkHub::has_all`]).
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
    /// The link RAM geometry (a game's `romset::LinkPlay`, defaults `srallyc`'s):
    /// offsets of the own block and of the first received block, the block
    /// size, and what the ID register holds beyond the 0-based node number
    /// (with layout 3).
    pub tx: usize,
    pub rx: usize,
    pub block: usize,
    pub id_base: u8,
    /// Frames of records kept beyond what the views need (a network resends from them).
    pub keep: u64,
    /// Per receiving node: ready, the IDs it gave every node, the started
    /// flags it last saw.
    ready: Vec<bool>,
    ids: Vec<Vec<u8>>,
    seen: Vec<Vec<bool>>,
    /// Per node, its records by frame.
    recs: Vec<std::collections::BTreeMap<u64, NodeRec>>,
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
        assert!((1..=8).contains(&nodes), "1 to 8 nodes");
        LinkHub {
            nodes,
            delay: delay.max(1),
            delays: vec![delay.max(1); nodes],
            layout: 3,
            drop_at: 0,
            freeze: None,
            fixed_ids: None,
            tx: TX,
            rx: RX,
            block: BLOCK,
            id_base: 1,
            keep: 0,
            ready: vec![false; nodes],
            ids: vec![vec![0; nodes]; nodes],
            seen: vec![vec![false; nodes]; nodes],
            recs: vec![Default::default(); nodes],
        }
    }

    /// A hub for the machines of a game, with its link RAM geometry.
    pub fn for_game(nodes: usize, delay: usize, play: &romset::LinkPlay) -> Self {
        assert!(nodes <= play.max_nodes, "{nodes} nodes: the game links at most {}", play.max_nodes);
        let mut h = LinkHub::new(nodes, delay);
        h.set_play(play);
        h
    }

    /// Take the geometry of a game's link RAM.
    pub fn set_play(&mut self, play: &romset::LinkPlay) {
        (self.tx, self.rx, self.block, self.id_base) = (play.tx, play.rx, play.block, play.id_base);
    }

    /// The ID the hub gave node `i` (valid once ready).
    pub fn id(&self, i: usize) -> u8 {
        self.ids[i][i]
    }

    /// Whether any node's board is up as it sees things.
    pub fn is_ready(&self) -> bool {
        self.ready.iter().any(|&r| r)
    }

    /// The largest delay of any node.
    pub fn max_delay(&self) -> usize {
        self.delays.iter().copied().max().unwrap_or(1)
    }

    /// Node `i`'s record of `frame`, if the hub has it.
    pub fn rec(&self, node: usize, frame: u64) -> Option<&NodeRec> {
        self.recs.get(node)?.get(&frame)
    }

    /// Add a node's record of `frame` (a local node's, or one that came from
    /// the network). Returns false and keeps the first if there is one.
    pub fn put(&mut self, node: usize, frame: u64, rec: NodeRec) -> bool {
        match self.recs[node].entry(frame) {
            std::collections::btree_map::Entry::Occupied(_) => false,
            std::collections::btree_map::Entry::Vacant(v) => {
                v.insert(rec);
                true
            }
        }
    }

    /// Record local node `node`'s machine at the boundary `frame`: its board
    /// flags and, while the board is started, its own block.
    pub fn put_local(&mut self, node: usize, frame: u64, bus: &Bus) -> NodeRec {
        let (started, master, start_frame) =
            bus.link.as_ref().map_or((false, false, 0), |l| (l.started, l.master, l.start_frame));
        let tx = match bus.link_ram() {
            Some(r) if started => r[self.tx..self.tx + self.block].to_vec(),
            _ => Vec::new(),
        };
        // The start frame matters only while started (and is what crosses the wire only then).
        let start_frame = if started { start_frame } else { 0 };
        let rec = NodeRec { started, master, start_frame, tx };
        self.put(node, frame, rec.clone());
        rec
    }

    /// The frame node `j`'s view takes the others' records from at `frame`.
    fn view_frame(&self, j: usize, frame: u64) -> Option<u64> {
        let d = self.delays[j].max(1) as u64;
        (frame + 1).checked_sub(d)
    }

    /// Whether every record the local nodes' views need at boundary `frame`
    /// is here: the others' records of `frame - (delay - 1)` (those before
    /// the first frame need none), and the local nodes' own of `frame`.
    pub fn has_all(&self, frame: u64, local: &[usize]) -> bool {
        self.missing(frame, local).is_empty()
    }

    /// What [`LinkHub::has_all`] still waits for: `(node, frame)`.
    pub fn missing(&self, frame: u64, local: &[usize]) -> Vec<(usize, u64)> {
        let mut out = Vec::new();
        for &j in local {
            if !self.recs[j].contains_key(&frame) {
                out.push((j, frame));
            }
            let Some(vf) = self.view_frame(j, frame) else { continue };
            for i in (0..self.nodes).filter(|&i| i != j) {
                // Frame numbers start at 1: there is no record of frame 0.
                if vf >= 1 && !self.recs[i].contains_key(&vf) && !out.contains(&(i, vf)) {
                    out.push((i, vf));
                }
            }
        }
        out
    }

    /// One frame boundary with every node here: every machine has run frame `frame - 1`.
    pub fn exchange(&mut self, frame: u64, buses: &mut [&mut Bus]) -> HubLog {
        assert_eq!(buses.len(), self.nodes);
        for (i, b) in buses.iter().enumerate() {
            self.put_local(i, frame, b);
        }
        let mut local: Vec<(usize, &mut Bus)> = buses.iter_mut().enumerate().map(|(i, b)| (i, &mut **b)).collect();
        self.apply(frame, &mut local)
    }

    /// Boundary `frame` for the local machines `(node, bus)`, whose records
    /// of it ([`LinkHub::put_local`]) and the others' needed ([`LinkHub::has_all`]) are in.
    pub fn apply(&mut self, frame: u64, local: &mut [(usize, &mut Bus)]) -> HubLog {
        let mut log = HubLog::new();
        let n = self.nodes;
        for (slot, (j, bus)) in local.iter_mut().enumerate() {
            let j = *j;
            let vf = self.view_frame(j, frame);
            let flags: Vec<(bool, bool, u64)> = (0..n)
                .map(|i| {
                    let r = if i == j { self.recs[i].get(&frame) } else { vf.and_then(|f| self.recs[i].get(&f)) };
                    r.map_or((false, false, 0), |r| (r.started, r.master, r.start_frame))
                })
                .collect();
            for (i, f) in flags.iter().enumerate() {
                if f.0 != self.seen[j][i] {
                    if slot == 0 {
                        log.push(format!(
                            "node {i} {} the board{}",
                            if f.0 { "starts" } else { "stops" },
                            if f.1 && f.0 { " (as master)" } else { "" }
                        ));
                    }
                    self.seen[j][i] = f.0;
                }
            }
            let all = flags.iter().all(|f| f.0) && !(self.drop_at != 0 && frame >= self.drop_at);
            if all && !self.ready[j] {
                // The master is 0, the others in the order they started.
                let mut order: Vec<usize> = (0..n).collect();
                order.sort_by_key(|&i| (!flags[i].1, flags[i].2, i));
                for (id, &i) in order.iter().enumerate() {
                    self.ids[j][i] = id as u8;
                }
                if let Some(f) = &self.fixed_ids {
                    self.ids[j].copy_from_slice(f);
                }
                if slot == 0 {
                    if flags.iter().filter(|f| f.1).count() != 1 {
                        log.push("warning: not exactly one master among the started nodes".into());
                    }
                    log.push(format!("link ready: IDs {:?}, {} nodes", self.ids[j], n));
                }
                self.ready[j] = true;
            } else if !all && self.ready[j] {
                self.ready[j] = false;
                if slot == 0 {
                    log.push("link down".into());
                }
            }
            let id = self.ids[j][j] + if self.layout == 3 { self.id_base } else { 0 };
            bus.link_set_ready(self.ready[j], id, n as u8);
            if !self.ready[j] {
                continue;
            }
            let me = self.ids[j][j] as usize;
            for k in 0..n {
                let want = source(self.layout, me, k, n);
                let src = (0..n).find(|&x| self.ids[j][x] as usize == want).unwrap();
                let rec = if src == j { self.recs[j].get(&frame) } else { vf.and_then(|f| self.recs[src].get(&f)) };
                let frozen =
                    src != j && vf.is_some_and(|f| self.freeze.is_some_and(|(fnode, at)| fnode == src && f >= at));
                if let (Some(r), false) = (rec, frozen) {
                    if r.tx.len() == self.block {
                        bus.link_rx_write_at(self.rx, self.block, k, &r.tx);
                    }
                }
            }
        }
        // Forget what no view will ask for again.
        let lo = frame.saturating_sub(self.max_delay() as u64 + self.keep);
        for r in &mut self.recs {
            *r = r.split_off(&lo);
        }
        log
    }

    /// The hub's state and the local nodes' board flags, for a save together
    /// with the machines' (the flags live in each bus's [`Link`], outside the
    /// machine's own state). `buses` is every node's, in node order.
    pub fn save(&self, buses: &[&mut Bus]) -> Vec<u8> {
        let mut o = vec![self.nodes as u8, self.delay as u8];
        o.extend((self.block as u16).to_le_bytes());
        o.extend(self.delays.iter().map(|&d| d as u8));
        o.extend(self.drop_at.to_le_bytes());
        for (i, b) in buses.iter().enumerate() {
            let l = b.link.as_ref();
            o.extend([self.ready[i] as u8]);
            o.extend(&self.ids[i]);
            o.extend(self.seen[i].iter().map(|&s| s as u8));
            o.extend([l.is_some_and(|l| l.started) as u8, l.is_some_and(|l| l.master) as u8]);
            o.extend(l.map_or(0, |l| l.start_frame).to_le_bytes());
        }
        for r in &self.recs {
            o.extend((r.len() as u32).to_le_bytes());
            for (f, rec) in r {
                o.extend(f.to_le_bytes());
                o.push(rec.started as u8 | (rec.master as u8) << 1);
                o.extend(rec.start_frame.to_le_bytes());
                o.extend((rec.tx.len() as u16).to_le_bytes());
                o.extend(&rec.tx);
            }
        }
        o
    }

    pub fn load(data: &[u8], buses: &mut [&mut Bus]) -> Result<Self, String> {
        let short = || "link hub state truncated".to_string();
        let nodes = *data.first().ok_or_else(short)? as usize;
        let delay = *data.get(1).ok_or_else(short)? as usize;
        if nodes != buses.len() || !(1..=8).contains(&nodes) {
            return Err(format!("the save has {nodes} nodes, the run {}", buses.len()));
        }
        let block = u16::from_le_bytes(data.get(2..4).ok_or_else(short)?.try_into().unwrap()) as usize;
        let mut hub = LinkHub::new(nodes, delay);
        hub.block = block;
        for (i, d) in data.get(4..4 + nodes).ok_or_else(short)?.iter().enumerate() {
            hub.delays[i] = *d as usize;
        }
        let at0 = 4 + nodes;
        hub.drop_at = u64::from_le_bytes(data.get(at0..at0 + 8).ok_or_else(short)?.try_into().unwrap());
        let mut at = at0 + 8;
        for (i, b) in buses.iter_mut().enumerate() {
            let len = 1 + 2 * nodes + 2 + 8;
            let f = data.get(at..at + len).ok_or_else(short)?;
            at += len;
            hub.ready[i] = f[0] != 0;
            hub.ids[i].copy_from_slice(&f[1..1 + nodes]);
            for (s, v) in hub.seen[i].iter_mut().zip(&f[1 + nodes..1 + 2 * nodes]) {
                *s = *v != 0;
            }
            let g = &f[1 + 2 * nodes..];
            if let Some(l) = b.link.as_mut() {
                (l.started, l.master) = (g[0] != 0, g[1] != 0);
                l.start_frame = u64::from_le_bytes(g[2..10].try_into().unwrap());
            }
        }
        for i in 0..nodes {
            let count = u32::from_le_bytes(data.get(at..at + 4).ok_or_else(short)?.try_into().unwrap());
            at += 4;
            for _ in 0..count {
                let h = data.get(at..at + 19).ok_or_else(short)?;
                let (frame, fl) = (u64::from_le_bytes(h[0..8].try_into().unwrap()), h[8]);
                let start_frame = u64::from_le_bytes(h[9..17].try_into().unwrap());
                let len = u16::from_le_bytes([h[17], h[18]]) as usize;
                let tx = data.get(at + 19..at + 19 + len).ok_or_else(short)?.to_vec();
                at += 19 + len;
                hub.recs[i].insert(frame, NodeRec { started: fl & 1 != 0, master: fl & 2 != 0, start_frame, tx });
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
        for n in 1..=8usize {
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
    fn every_games_blocks_fit_the_link_ram() {
        for id in ["srallyc", "daytona"] {
            let p = romset::games::link_play(id).unwrap();
            assert!(p.rx + p.max_nodes * p.block <= 0x4000, "{id}: {} received blocks fit the 16 KB", p.max_nodes);
            assert_eq!(p.tx + p.block, p.rx, "{id}: the received blocks follow the own block");
        }
    }

    #[test]
    fn an_idle_board_is_fitted_and_not_busy() {
        let s = Link::new(LinkBoard::Idle).status();
        assert_eq!(s & 0x01, 0, "bit 0 of 0x1A14000: board present");
        assert_eq!(s >> 16 & 0x80, 0, "bit 7 of 0x1A14002: not busy");
    }
}
