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

/// What the hub shows of a node that has left, from the frame after its drop frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ghost {
    /// Repeat the node's last record: its car stops, everything else it said stays said.
    #[default]
    Freeze,
    /// The block is a copy of the lowest-numbered node still playing, but for the identity bytes
    /// (`ghost_keep`) and the flags, which stay the leaver's: the leaver keeps pace with the
    /// others in the game's state, its clock and its barriers.
    Mirror,
    /// The leaver's board stops: its record says "not started" (the link goes down for all).
    Down,
}

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
    /// A change of the link delay this record announces: `(from, D)`, D becomes `D` from
    /// the boundary `from` on every node (the session host's records only: `docs/hw/link-board.md`
    /// section 12.7). It is part of the record, so it reaches every host by the same
    /// acknowledged, resent path as the block and is covered by the state check.
    pub change: Option<(u64, u8)>,
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
    /// How a node that has left shows from its drop frame on, and for [`Ghost::Mirror`] the byte
    /// ranges of the block that stay the leaver's and the offset of a word-sum checksum to recompute.
    pub ghost: Ghost,
    pub ghost_keep: Vec<(usize, usize)>,
    pub ghost_sum: Option<usize>,
    /// Per receiving node: ready, the IDs it gave every node, the started
    /// flags it last saw.
    ready: Vec<bool>,
    ids: Vec<Vec<u8>>,
    seen: Vec<Vec<bool>>,
    /// Per node, its records by frame.
    recs: Vec<std::collections::BTreeMap<u64, NodeRec>>,
    /// Nodes that have left: the node's drop frame F (its last real record), and a
    /// copy of that record, which every later frame's record is a repeat of.
    left: Vec<Option<u64>>,
    last: Vec<Option<NodeRec>>,
    /// The delay changes (`LinkHub::schedule_delay`), by the boundary they take effect at.
    sched: Vec<DelayChange>,
}

/// D becomes `d` from boundary `from`, and `floor` is the frame the others' view had reached
/// at `from - 1`: a view never goes back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DelayChange {
    pub from: u64,
    pub d: usize,
    pub floor: u64,
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
            ghost: Ghost::Freeze,
            ghost_keep: Vec::new(),
            ghost_sum: None,
            ready: vec![false; nodes],
            ids: vec![vec![0; nodes]; nodes],
            seen: vec![vec![false; nodes]; nodes],
            recs: vec![Default::default(); nodes],
            left: vec![None; nodes],
            last: vec![None; nodes],
            sched: Vec::new(),
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
        self.ghost = Ghost::Mirror;
        self.ghost_keep = play.ghost_keep.to_vec();
        self.ghost_sum = play.ghost_sum;
    }

    /// The ID the hub gave node `i` (valid once ready).
    pub fn id(&self, i: usize) -> u8 {
        self.ids[i][i]
    }

    /// Whether any node's board is up as it sees things.
    pub fn is_ready(&self) -> bool {
        self.ready.iter().any(|&r| r)
    }

    /// The largest delay of any node, now or scheduled.
    pub fn max_delay(&self) -> usize {
        self.delays.iter().copied().chain(self.sched.iter().map(|c| c.d)).max().unwrap_or(1)
    }

    /// D becomes `d` from boundary `from` (the same for every node: it replaces the per-node
    /// delays from then on). The others' view of boundary `b >= from` is the frame
    /// `max(b + 1 - d, v)`, `v` the frame the view had reached at `from - 1`: a raise
    /// shows the same frame until `b + 1 - d` catches up (nothing new for `d - D` boundaries),
    /// a lower jumps over `D - d` frames (their records are never shown). Every host of a session
    /// must schedule the same changes before it passes `from`. Returns false (and changes nothing)
    /// if `from` is not after the last scheduled change or `d` is out of 1..=60.
    pub fn schedule_delay(&mut self, from: u64, d: usize) -> bool {
        if !(1..=60).contains(&d) || from < 2 || self.sched.last().is_some_and(|c| c.from >= from) {
            return false;
        }
        let floor = self.view_frame_for(None, from - 1).unwrap_or(0);
        self.sched.push(DelayChange { from, d, floor });
        true
    }

    /// The scheduled delay changes.
    pub fn changes(&self) -> &[DelayChange] {
        &self.sched
    }

    /// The delay in force at boundary `frame`: the others' records come from `frame + 1 - D`.
    pub fn delay_at(&self, frame: u64) -> usize {
        self.sched.iter().rev().find(|c| c.from <= frame).map_or(self.delay, |c| c.d)
    }

    /// Whether a change takes effect after boundary `frame` (announced, not yet reached).
    pub fn change_pending(&self, frame: u64) -> bool {
        self.sched.last().is_some_and(|c| c.from > frame)
    }

    /// Node `i`'s record of `frame`, if the hub has it: for a node that has left
    /// ([`LinkHub::leave`]), the record of every frame after its drop frame is a
    /// repeat of the record of the drop frame (the node is frozen, as `--freeze` does).
    pub fn rec(&self, node: usize, frame: u64) -> Option<std::borrow::Cow<'_, NodeRec>> {
        use std::borrow::Cow;
        match self.left.get(node)? {
            Some(fl) if frame > *fl => {
                let last = self.last[node].as_ref().or_else(|| self.recs[node].get(fl))?;
                match self.ghost {
                    Ghost::Freeze => Some(Cow::Borrowed(last)),
                    Ghost::Down => Some(Cow::Owned(NodeRec::default())),
                    Ghost::Mirror => {
                        // The lowest node that is still playing at `frame` (a node is until its drop frame).
                        let m = (0..self.nodes).find(|&n| n != node && self.left[n].is_none_or(|f| f >= frame));
                        let Some(m) = m else { return Some(Cow::Borrowed(last)) };
                        let src = self.rec(m, frame)?;
                        let mut out = src.into_owned();
                        out.master = last.master;
                        out.start_frame = last.start_frame;
                        out.started = out.started && last.started;
                        if out.tx.len() == self.block && last.tx.len() == self.block {
                            for &(a, b) in &self.ghost_keep {
                                out.tx[a..b].copy_from_slice(&last.tx[a..b]);
                            }
                            if let Some(at) = self.ghost_sum {
                                let sum = (0..self.block.min(64)).step_by(4).filter(|&o| o != at).fold(0u32, |a, o| {
                                    a.wrapping_add(u32::from_le_bytes(out.tx[o..o + 4].try_into().unwrap()))
                                });
                                out.tx[at..at + 4].copy_from_slice(&sum.to_le_bytes());
                            }
                        }
                        Some(Cow::Owned(out))
                    }
                }
            }
            // The drop frame's own record is kept after the others are forgotten.
            Some(fl) if frame == *fl => self.recs[node].get(&frame).or(self.last[node].as_ref()).map(Cow::Borrowed),
            _ => self.recs.get(node)?.get(&frame).map(Cow::Borrowed),
        }
    }

    /// Node `node` has left: its last real record is that of frame `drop_frame`, and from the next
    /// frame its records are repeats of it. Every host of a session must give the same frame
    /// (`remodel-linknet` agrees it), and the hub must not have passed a boundary that needed a
    /// record of the node beyond it. Idempotent for the same frame.
    pub fn leave(&mut self, node: usize, drop_frame: u64) {
        if self.left[node].is_some() {
            return;
        }
        self.left[node] = Some(drop_frame);
        self.last[node] = self.recs[node].get(&drop_frame).cloned();
    }

    /// The frame node `node` left at, if it has.
    pub fn left_at(&self, node: usize) -> Option<u64> {
        self.left[node]
    }

    /// Add a node's record of `frame` (a local node's, or one that came from
    /// the network). Returns false and keeps the first if there is one.
    pub fn put(&mut self, node: usize, frame: u64, rec: NodeRec) -> bool {
        if self.left[node] == Some(frame) && self.last[node].is_none() {
            self.last[node] = Some(rec.clone());
        }
        match self.recs[node].entry(frame) {
            std::collections::btree_map::Entry::Occupied(_) => false,
            std::collections::btree_map::Entry::Vacant(v) => {
                let change = rec.change;
                v.insert(rec);
                if let Some((from, d)) = change {
                    self.schedule_delay(from, d as usize);
                }
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
        let rec = NodeRec { started, master, start_frame, tx, change: None };
        self.put(node, frame, rec.clone());
        rec
    }

    /// The frame node `j`'s view takes the others' records from at `frame`.
    pub fn view_frame(&self, j: usize, frame: u64) -> Option<u64> {
        self.view_frame_for(Some(j), frame)
    }

    /// The same with the node's own delay (`None`: the session's base delay) before any change.
    fn view_frame_for(&self, j: Option<usize>, frame: u64) -> Option<u64> {
        match self.sched.iter().rev().find(|c| c.from <= frame) {
            Some(c) => {
                let v = (frame + 1).saturating_sub(c.d as u64).max(c.floor);
                Some(v)
            }
            None => {
                let d = j.map_or(self.delay, |j| self.delays[j]).max(1) as u64;
                (frame + 1).checked_sub(d)
            }
        }
    }

    /// Whether every record the local nodes' views need at boundary `frame`
    /// is here: the others' records of `frame - (delay - 1)` (those before
    /// the first frame need none), and the local nodes' own of `frame`.
    pub fn has_all(&self, frame: u64, local: &[usize]) -> bool {
        self.missing(frame, local).is_empty()
    }

    fn has_rec(&self, node: usize, frame: u64) -> bool {
        self.rec(node, frame).is_some()
    }

    /// What [`LinkHub::has_all`] still waits for: `(node, frame)`.
    pub fn missing(&self, frame: u64, local: &[usize]) -> Vec<(usize, u64)> {
        let mut out = Vec::new();
        for &j in local {
            if !self.has_rec(j, frame) {
                out.push((j, frame));
            }
            let Some(vf) = self.view_frame(j, frame) else { continue };
            for i in (0..self.nodes).filter(|&i| i != j) {
                // Frame numbers start at 1: there is no record of frame 0.
                if vf >= 1 {
                    // A node that left needs its drop frame's record, whatever the frame.
                    let need = self.left[i].map_or(vf, |fl| vf.min(fl));
                    if !self.has_rec(i, need) && !out.contains(&(i, need)) {
                        out.push((i, need));
                    }
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
                    let r = if i == j { self.rec(i, frame) } else { vf.and_then(|f| self.rec(i, f)) };
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
                let rec = if src == j { self.rec(j, frame) } else { vf.and_then(|f| self.rec(src, f)) };
                let frozen =
                    src != j && vf.is_some_and(|f| self.freeze.is_some_and(|(fnode, at)| fnode == src && f >= at));
                if let (Some(r), false) = (rec, frozen) {
                    if r.tx.len() == self.block {
                        bus.link_rx_write_at(self.rx, self.block, k, &r.tx);
                    }
                }
            }
        }
        // The drop frame's record is kept whatever is forgotten.
        for i in 0..n {
            if let (Some(fl), None) = (self.left[i], &self.last[i]) {
                self.last[i] = self.recs[i].get(&fl).cloned();
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
        // Nodes that have left (only written when there are any, so a save without is as it was).
        if self.left.iter().any(|l| l.is_some()) {
            for (l, rec) in self.left.iter().zip(&self.last) {
                match l {
                    Some(f) => {
                        o.push(1);
                        o.extend(f.to_le_bytes());
                        let r = rec.clone().unwrap_or_default();
                        o.push(r.started as u8 | (r.master as u8) << 1 | (rec.is_some() as u8) << 2);
                        o.extend(r.start_frame.to_le_bytes());
                        o.extend((r.tx.len() as u16).to_le_bytes());
                        o.extend(&r.tx);
                    }
                    None => o.push(0),
                }
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
                hub.recs[i].insert(
                    frame,
                    NodeRec { started: fl & 1 != 0, master: fl & 2 != 0, start_frame, tx, change: None },
                );
            }
        }
        // Nodes that have left, if the save has them.
        if at < data.len() {
            for i in 0..nodes {
                if *data.get(at).ok_or_else(short)? == 0 {
                    at += 1;
                    continue;
                }
                let h = data.get(at + 1..at + 1 + 8 + 1 + 8 + 2).ok_or_else(short)?;
                let f = u64::from_le_bytes(h[0..8].try_into().unwrap());
                let fl = h[8];
                let start_frame = u64::from_le_bytes(h[9..17].try_into().unwrap());
                let len = u16::from_le_bytes([h[17], h[18]]) as usize;
                let tx = data.get(at + 20..at + 20 + len).ok_or_else(short)?.to_vec();
                at += 20 + len;
                hub.left[i] = Some(f);
                if fl & 4 != 0 {
                    hub.last[i] =
                        Some(NodeRec { started: fl & 1 != 0, master: fl & 2 != 0, start_frame, tx, change: None });
                }
            }
        }
        Ok(hub)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frame the others' records are taken from at each boundary `from..=to`.
    fn views(h: &LinkHub, from: u64, to: u64) -> Vec<u64> {
        (from..=to).map(|b| h.view_frame(0, b).unwrap()).collect()
    }

    #[test]
    fn a_delay_raise_holds_the_view_and_a_lower_skips_a_frame() {
        let mut h = LinkHub::new(2, 3);
        assert_eq!(views(&h, 10, 14), [8, 9, 10, 11, 12]);
        // D 3 -> 5 from boundary 20: the view stays on frame 17 (the one it showed at 19) for the boundaries 20 and 21,
        // then moves a frame a boundary again; it never goes back.
        assert!(h.schedule_delay(20, 5));
        assert_eq!(views(&h, 17, 26), [15, 16, 17, 17, 17, 18, 19, 20, 21, 22]);
        assert_eq!((h.delay_at(19), h.delay_at(20)), (3, 5));
        // D 5 -> 4 from 30: one record (frame 26) is never shown.
        assert!(h.schedule_delay(30, 4));
        assert_eq!(views(&h, 28, 32), [24, 25, 27, 28, 29]);
        // D 4 -> 4 is a no-change schedule that still must not move the view; 4 -> 1 from 40 jumps ahead.
        assert!(h.schedule_delay(40, 1));
        assert_eq!(views(&h, 38, 41), [35, 36, 40, 41]);
        assert_eq!(h.max_delay(), 5);
        // Changes must come in order, within 1..=60, and never from the first boundary.
        assert!(!h.schedule_delay(40, 2) && !h.schedule_delay(35, 2) && !h.schedule_delay(50, 0));
        assert!(!LinkHub::new(2, 3).schedule_delay(1, 2));
    }

    #[test]
    fn a_change_in_a_record_schedules_it_once_and_the_view_stays_what_lockstep_needs() {
        let mut h = LinkHub::new(2, 2);
        let rec = |change| NodeRec { started: true, master: true, start_frame: 2, tx: vec![1], change };
        assert!(h.put(0, 10, rec(Some((14, 4)))));
        assert!(!h.put(0, 10, rec(Some((15, 5)))), "the first record of a frame is kept");
        assert_eq!(h.changes().len(), 1);
        // The records the hub needs at boundary 14 and 15 (node 1 viewing node 0): frame 12 (what it had reached at 13) on both.
        assert_eq!(h.missing(14, &[1]), vec![(1, 14), (0, 12)]);
        assert_eq!(h.missing(15, &[1]), vec![(1, 15), (0, 12)]);
    }

    #[test]
    fn the_scripted_partner_delays_its_blocks_and_goes_silent() {
        let mut l = Link::new(LinkBoard::Idle).with_fake(0, 2).with_timing(2, 10);
        assert_eq!(l.deliver(0, &[1]), None, "the pipe is still filling");
        assert_eq!(l.deliver(1, &[2]), None);
        assert_eq!(l.deliver(2, &[3]), Some(vec![1]), "two frames late");
        assert_eq!(l.deliver(3, &[4]), Some(vec![2]));
        assert_eq!(l.deliver(10, &[5]), None, "silent from frame 10");
    }

    fn block_of(node: usize, frame: u64) -> NodeRec {
        let mut tx = vec![0u8; BLOCK];
        tx[0] = node as u8 + 1;
        tx[2] = node as u8 + 1;
        tx[10] = frame as u8;
        tx[100] = node as u8;
        NodeRec { started: true, master: node == 0, start_frame: 3, tx, change: None }
    }

    #[test]
    fn a_node_that_left_repeats_its_last_record_and_is_no_longer_waited_for() {
        let mut h = LinkHub::new(2, 2);
        h.leave(1, 5);
        for f in 1..=8 {
            h.put(0, f, block_of(0, f));
            h.put(1, f, block_of(1, f));
        }
        assert_eq!(h.rec(1, 5).unwrap().tx[10], 5);
        assert_eq!(h.rec(1, 6).unwrap().tx[10], 5, "frame 6 is a repeat of 5");
        assert_eq!(h.rec(1, 8).unwrap().tx[10], 5, "even though a record of 8 arrived");
        // Boundary 20 of node 0 needs nothing of node 1 beyond the drop frame, only its own record.
        let mut g = LinkHub::new(2, 2);
        g.leave(1, 5);
        for f in 1..=5 {
            g.put(1, f, block_of(1, f));
        }
        g.put(0, 20, block_of(0, 20));
        assert!(g.has_all(20, &[0]));
        // A node that has left but whose drop-frame record has not arrived is waited for.
        let mut k = LinkHub::new(2, 2);
        k.leave(1, 5);
        k.put(0, 20, block_of(0, 20));
        assert_eq!(k.missing(20, &[0]), vec![(1, 5)]);
        k.put(1, 5, block_of(1, 5));
        assert!(k.has_all(20, &[0]));
        // Before the drop frame the real records are still needed.
        let mut m = LinkHub::new(2, 2);
        m.leave(1, 5);
        m.put(0, 4, block_of(0, 4));
        assert_eq!(m.missing(4, &[0]), vec![(1, 3)]);
    }

    #[test]
    fn a_mirrored_leaver_copies_the_lowest_node_still_playing_but_keeps_its_identity() {
        let mut h = LinkHub::new(3, 2);
        h.ghost = Ghost::Mirror;
        h.ghost_keep = vec![(0, 4), (100, 101)];
        h.ghost_sum = Some(0x3c);
        h.leave(1, 5);
        for f in 1..=9 {
            for n in 0..3 {
                h.put(n, f, block_of(n, f));
            }
        }
        let g = h.rec(1, 7).unwrap().into_owned();
        assert_eq!(g.tx[10], 7, "the state moves with node 0");
        assert_eq!((g.tx[0], g.tx[2]), (2, 2), "node 1's own identity bytes");
        assert_eq!(g.tx[100], 1, "and its car's pose");
        assert!(g.started && !g.master && g.start_frame == 3);
        let sum = (0..16)
            .filter(|&w| w != 15)
            .fold(0u32, |a, w| a.wrapping_add(u32::from_le_bytes(g.tx[w * 4..w * 4 + 4].try_into().unwrap())));
        assert_eq!(u32::from_le_bytes(g.tx[0x3c..0x40].try_into().unwrap()), sum, "checksum recomputed");
        // Up to the drop frame it is the real record.
        assert_eq!(h.rec(1, 5).unwrap().tx[100], 1);
        assert_eq!(h.rec(1, 4).unwrap().tx[10], 4);
        // Node 0 leaves at 6: from 7 the lowest node playing is node 2 (for node 1's ghost too).
        h.leave(0, 6);
        assert_eq!(h.rec(1, 7).unwrap().tx[100], 1);
        assert_eq!(h.rec(1, 8).unwrap().tx[10], 8);
        assert_eq!(h.rec(0, 8).unwrap().tx[100], 0, "node 0's own pose");
        assert_eq!(h.rec(0, 8).unwrap().tx[0], 1);
        // The board of a leaver can also be shown down.
        h.ghost = Ghost::Down;
        assert!(!h.rec(1, 8).unwrap().started);
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
