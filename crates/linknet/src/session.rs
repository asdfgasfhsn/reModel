//! A running session: the hosts' records of each frame carried between
//! them over UDP (a mesh: every host talks to every other), with
//! acknowledgements, redundancy, state checks, liveness and a clean end.
//!
//! What a packet carries: every record the sender has made that the
//! receiver has not yet said it holds, oldest first (`acks` says what is
//! held), so a lost packet is repaired by the next one with nothing to
//! ask for; the sender's hashes of the last few checkpoint frames; and the
//! acknowledgements. A packet is sent when a record is made, when one is
//! unacknowledged for a while, and now and then as a heartbeat.

use crate::net::Net;
use crate::wire::*;
use model2::link::NodeRec;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

/// How long before an unacknowledged record is sent again.
const RESEND: Duration = Duration::from_millis(12);
/// How long a received record waits for a packet of ours to carry its acknowledgement.
const ACK_DELAY: Duration = Duration::from_millis(6);
const HEARTBEAT: Duration = Duration::from_millis(100);
const PING_EVERY: Duration = Duration::from_millis(500);
/// Frames of records kept behind the newest, as bases for the deltas.
const RETAIN: u64 = 256;
/// Checkpoint hashes carried in each packet.
const HASHES: usize = 8;

/// What happened that the driver must act on.
#[derive(Debug)]
pub enum Event {
    /// A record from another host's node, validated.
    Rec { node: usize, frame: u64, rec: NodeRec },
}

/// How the session ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum End {
    /// A host finished or left and said so.
    Left { host: usize, name: String, reason: String },
    /// A host went silent.
    Silent { host: usize, name: String, ms: u64 },
    /// The checkpoint hashes differ.
    Desync { frame: u64, ours: u64, theirs: u64, host: usize, name: String },
    /// The host aborted the setup.
    Aborted(String),
}

impl std::fmt::Display for End {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            End::Left { name, reason, .. } if reason.is_empty() || reason == "left" => write!(f, "{name} left"),
            End::Left { name, reason, .. } => write!(f, "{name} left: {reason}"),
            End::Silent { name, ms, .. } => {
                write!(f, "{name} has been silent for {:.1} s: connection lost", *ms as f32 / 1000.0)
            }
            End::Desync { frame, ours, theirs, name, .. } => {
                write!(f, "out of sync with {name} at frame {frame}: our state hash {ours:016x}, theirs {theirs:016x}")
            }
            End::Aborted(s) => write!(f, "session aborted: {s}"),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub bytes_tx: u64,
    pub bytes_rx: u64,
    pub pkts_tx: u64,
    pub pkts_rx: u64,
    /// Datagrams dropped as malformed or out of place.
    pub bad: u64,
    /// Records received again (duplicates, resends).
    pub dup_recs: u64,
    pub lost_by_fault: u64,
    /// Checkpoints compared and found equal.
    pub checks_ok: u64,
}

struct Peer {
    addr: SocketAddr,
    /// What the peer says it holds of ours: records of all our nodes up to this frame.
    acked: u64,
    last_send: Instant,
    last_heard: Instant,
    /// A record of the peer's arrived and its acknowledgement is owed since this time.
    owe_ack: Option<Instant>,
    /// A record of ours the peer has not been sent yet.
    fresh: bool,
    rtt_ms: Option<f32>,
    last_ping: Instant,
    /// The peer's checkpoint hashes not yet compared, and the frames already compared.
    hashes: BTreeMap<u64, u64>,
    compared: std::collections::BTreeSet<u64>,
}

pub struct Session {
    pub net: Net,
    pub me: usize,
    names: Vec<String>,
    pub params: Params,
    peers: Vec<Option<Peer>>,
    nodes_of: Vec<Vec<usize>>,
    n: usize,
    /// Every node's records by frame (ours until acknowledged by all, the others' recent ones).
    store: Vec<BTreeMap<u64, NodeRec>>,
    /// Per node, the frame up to which its records are all held.
    contig: Vec<u64>,
    /// Our newest published frame.
    latest: u64,
    /// The newest frame any peer has reported.
    pub peer_latest: u64,
    ours: BTreeMap<u64, u64>,
    recent: Vec<(u64, u64)>,
    epoch: Instant,
    pub end: Option<End>,
    pub stats: Stats,
    bye_sent: bool,
}

impl Session {
    /// Host `h`'s name as shown (players sharing a name are told apart).
    pub fn name_of(&self, h: usize) -> String {
        self.names.get(h).cloned().unwrap_or_else(|| format!("player {}", h + 1))
    }

    /// The hosts' names as shown, by host.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// A session of `params` as host `me`, on `net`, with the other hosts at `addrs`
    /// (indexed by host; ours is ignored).
    pub fn new(net: Net, me: usize, params: Params, addrs: Vec<SocketAddr>) -> Session {
        let now = Instant::now();
        let n = params.nodes();
        let nodes_of: Vec<Vec<usize>> =
            params.hosts.iter().map(|h| h.nodes.iter().map(|&x| x as usize).collect()).collect();
        let peers = addrs
            .into_iter()
            .enumerate()
            .map(|(h, addr)| {
                (h != me).then_some(Peer {
                    addr,
                    acked: 0,
                    last_send: now - Duration::from_secs(1),
                    last_heard: now,
                    owe_ack: None,
                    fresh: false,
                    rtt_ms: None,
                    last_ping: now,
                    hashes: BTreeMap::new(),
                    compared: Default::default(),
                })
            })
            .collect();
        let names = crate::names::display_names(&params.hosts.iter().map(|h| h.name.clone()).collect::<Vec<_>>());
        Session {
            net,
            me,
            names,
            params,
            peers,
            nodes_of,
            n,
            store: vec![BTreeMap::new(); n],
            contig: vec![0; n],
            latest: 0,
            peer_latest: 0,
            ours: BTreeMap::new(),
            recent: Vec::new(),
            epoch: now,
            end: None,
            stats: Stats::default(),
            bye_sent: false,
        }
    }

    /// The nodes this host runs.
    pub fn local_nodes(&self) -> &[usize] {
        &self.nodes_of[self.me]
    }

    /// The round trip to each other host in milliseconds (None until measured).
    pub fn rtts(&self) -> Vec<Option<f32>> {
        self.peers.iter().map(|p| p.as_ref().and_then(|p| p.rtt_ms)).collect()
    }

    /// The largest measured round trip.
    pub fn worst_rtt(&self) -> Option<f32> {
        self.rtts().into_iter().flatten().fold(None, |a: Option<f32>, b| Some(a.map_or(b, |a| a.max(b))))
    }

    pub fn peers(&self) -> usize {
        self.peers.iter().flatten().count()
    }

    /// The lowest frame every peer has acknowledged of ours.
    pub fn acked_by_all(&self) -> u64 {
        self.peers.iter().flatten().map(|p| p.acked).min().unwrap_or(self.latest)
    }

    /// Our record of local node `node` at `frame`, made at the frame boundary.
    pub fn publish(&mut self, node: usize, frame: u64, rec: NodeRec) {
        self.store[node].insert(frame, rec);
        for p in self.peers.iter_mut().flatten() {
            p.fresh = true;
        }
        if node == *self.nodes_of[self.me].last().unwrap() {
            self.latest = self.latest.max(frame);
        }
        self.advance(node);
        self.checkpoint(frame);
    }

    fn advance(&mut self, node: usize) {
        while self.store[node].contains_key(&(self.contig[node] + 1)) {
            self.contig[node] += 1;
        }
    }

    /// What this host holds of `host`'s nodes: the frame up to which all are held.
    fn held_of(&self, host: usize) -> u64 {
        self.nodes_of[host].iter().map(|&n| self.contig[n]).min().unwrap_or(0)
    }

    /// If every node's record of checkpoint `frame` is held, hash it and compare.
    fn checkpoint(&mut self, frame: u64) {
        let every = self.params.check_every as u64;
        if every == 0 || !frame.is_multiple_of(every) || self.ours.contains_key(&frame) {
            return;
        }
        let recs: Vec<Option<&NodeRec>> = (0..self.n).map(|i| self.store[i].get(&frame)).collect();
        if recs.iter().any(|r| r.is_none()) {
            return;
        }
        let h = hash_recs(&recs.into_iter().flatten().collect::<Vec<_>>());
        self.ours.insert(frame, h);
        self.recent.push((frame, h));
        if self.recent.len() > HASHES {
            self.recent.remove(0);
        }
        self.compare(frame);
        if self.ours.len() > 64 {
            let first = *self.ours.keys().next().unwrap();
            self.ours.remove(&first);
        }
    }

    fn compare(&mut self, frame: u64) {
        let Some(&ours) = self.ours.get(&frame) else { return };
        for h in 0..self.peers.len() {
            let theirs = self.peers[h].as_ref().and_then(|p| p.hashes.get(&frame)).copied();
            if let Some(theirs) = theirs {
                if theirs != ours {
                    let name = self.name_of(h);
                    self.finish(End::Desync { frame, ours, theirs, host: h, name });
                    return;
                }
                self.stats.checks_ok += 1;
                if let Some(p) = self.peers[h].as_mut() {
                    p.hashes.remove(&frame);
                    p.compared.insert(frame);
                    if p.compared.len() > 64 {
                        let first = *p.compared.iter().next().unwrap();
                        p.compared.remove(&first);
                    }
                }
            }
        }
    }

    /// End the session with `end`, telling the others why.
    fn finish(&mut self, end: End) {
        if self.end.is_some() {
            return;
        }
        let reason = match &end {
            End::Desync { .. } => end.to_string(),
            _ => String::new(),
        };
        if !reason.is_empty() {
            self.bye(&reason);
        }
        self.end = Some(end);
    }

    /// Tell every other host that this one is leaving (sent a few times, as
    /// datagrams can be lost), without the fault injection.
    pub fn bye(&mut self, reason: &str) {
        if self.bye_sent {
            return;
        }
        self.bye_sent = true;
        let mut w = Writer::new(kind::BYE);
        w.u32(self.params.session).u8(self.me as u8).str(reason);
        for p in self.peers.iter().flatten() {
            for _ in 0..3 {
                let _ = self.net.sock.send_to(&w.0, p.addr);
            }
        }
    }

    fn now_ms(&self) -> u32 {
        self.epoch.elapsed().as_millis() as u32
    }

    /// Do what is due: read what arrived, send what should go, check the
    /// others are alive. Call often (every frame and while stalled).
    pub fn pump(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        self.net.flush();
        while let Some((bytes, from)) = self.net.recv() {
            self.receive(&bytes, from, &mut events);
        }
        if self.end.is_none() {
            self.send_due();
            self.liveness();
        }
        self.stats.bytes_tx = self.net.bytes_tx;
        self.stats.bytes_rx = self.net.bytes_rx;
        self.stats.pkts_tx = self.net.pkts_tx;
        self.stats.pkts_rx = self.net.pkts_rx;
        self.stats.lost_by_fault = self.net.lost_by_fault;
        events
    }

    fn liveness(&mut self) {
        let timeout = Duration::from_millis(self.params.timeout_ms as u64);
        for h in 0..self.peers.len() {
            if let Some(p) = &self.peers[h] {
                if p.last_heard.elapsed() > timeout {
                    let ms = p.last_heard.elapsed().as_millis() as u64;
                    let name = self.name_of(h);
                    self.finish(End::Silent { host: h, name, ms });
                    return;
                }
            }
        }
    }

    fn send_due(&mut self) {
        let now = Instant::now();
        for h in 0..self.peers.len() {
            let Some(p) = &self.peers[h] else { continue };
            let unacked = p.acked < self.latest;
            let due = (unacked && (p.fresh || now.duration_since(p.last_send) >= RESEND))
                || p.owe_ack.is_some_and(|t| now.duration_since(t) >= ACK_DELAY)
                || now.duration_since(p.last_send) >= HEARTBEAT;
            if due {
                self.send_data(h);
            }
            let Some(p) = &mut self.peers[h] else { continue };
            if now.duration_since(p.last_ping) >= PING_EVERY {
                p.last_ping = now;
                let to = p.addr;
                let mut w = Writer::new(kind::PING);
                w.u32(self.params.session).u8(self.me as u8).u32(self.epoch.elapsed().as_millis() as u32);
                self.net.send(to, &w.0);
            }
        }
    }

    /// Send peer `h` every record of ours it has not acknowledged (as many as fit), with acks and hashes.
    fn send_data(&mut self, h: usize) {
        let mut w = Writer::new(kind::DATA);
        let acked = self.peers[h].as_ref().unwrap().acked;
        w.u32(self.params.session).u8(self.me as u8).u64(self.latest);
        w.u8(self.peers.len() as u8);
        for k in 0..self.peers.len() {
            w.u64(if k == self.me { 0 } else { self.held_of(k) });
        }
        w.u8(self.recent.len() as u8);
        for &(f, hash) in &self.recent {
            w.u64(f).u64(hash);
        }
        let first = acked + 1;
        let count_at = w.0.len() + 8;
        w.u64(first).u8(0);
        let mut count = 0u8;
        let block = self.params.block as usize;
        let zeros = vec![0u8; block];
        let mut f = first;
        while f <= self.latest && (count as usize) < MAX_FRAMES {
            let mark = w.0.len();
            for &node in &self.nodes_of[self.me] {
                let Some(rec) = self.store[node].get(&f) else {
                    w.0.truncate(mark);
                    f = u64::MAX;
                    break;
                };
                let base = self.store[node]
                    .get(&(f - 1))
                    .map(|r| r.tx.as_slice())
                    .filter(|t| t.len() == block)
                    .unwrap_or(&zeros);
                encode_rec(&mut w, rec, base, block);
            }
            if f == u64::MAX || w.0.len() > MAX_PACKET - 200 && count > 0 {
                if f != u64::MAX {
                    w.0.truncate(mark);
                }
                break;
            }
            count += 1;
            f += 1;
        }
        w.0[count_at] = count;
        let p = self.peers[h].as_mut().unwrap();
        p.last_send = Instant::now();
        p.owe_ack = None;
        p.fresh = false;
        let to = p.addr;
        self.net.send(to, &w.0);
    }

    fn receive(&mut self, bytes: &[u8], from: SocketAddr, events: &mut Vec<Event>) {
        let Some((ver, k, mut r)) = header(bytes) else { return self.stats.bad += 1 };
        if ver != PROTO {
            return self.stats.bad += 1;
        }
        let ok = match k {
            kind::DATA => self.on_data(&mut r, from, events),
            kind::PING | kind::PONG => self.on_ping(k, &mut r, from),
            kind::BYE => self.on_bye(&mut r, from),
            kind::START_ACK => {
                // A joiner that has not seen our GO yet: say it again.
                let mut w = Writer::new(kind::GO);
                w.u32(self.params.session);
                self.net.send(from, &w.0);
                Some(())
            }
            kind::ABORT => {
                let (s, reason) = (r.u32(), r.str());
                match (s, reason) {
                    (Some(s), Some(reason)) if s == self.params.session && self.host_of(from).is_some() => {
                        self.finish(End::Aborted(reason));
                        Some(())
                    }
                    _ => None,
                }
            }
            kind::GO
            | kind::START
            | kind::WELCOME
            | kind::LOBBY
            | kind::HELLO
            | kind::INFO
            | kind::DISCOVER
            | kind::REFUSE => Some(()),
            _ => None,
        };
        if ok.is_none() {
            self.stats.bad += 1;
        }
    }

    /// The host whose address is `from`.
    fn host_of(&self, from: SocketAddr) -> Option<usize> {
        self.peers.iter().position(|p| p.as_ref().is_some_and(|p| p.addr == from))
    }

    fn on_ping(&mut self, k: u8, r: &mut Reader, from: SocketAddr) -> Option<()> {
        let (s, h, t) = (r.u32()?, r.u8()? as usize, r.u32()?);
        if s != self.params.session || self.host_of(from) != Some(h) {
            return None;
        }
        if k == kind::PING {
            let mut w = Writer::new(kind::PONG);
            w.u32(self.params.session).u8(self.me as u8).u32(t);
            self.net.send(from, &w.0);
        } else {
            let rtt = self.now_ms().wrapping_sub(t) as f32;
            if rtt < 60_000.0 {
                let p = self.peers[h].as_mut()?;
                p.rtt_ms = Some(p.rtt_ms.map_or(rtt, |old| old * 0.8 + rtt * 0.2));
            }
        }
        self.peers[h].as_mut()?.last_heard = Instant::now();
        Some(())
    }

    fn on_bye(&mut self, r: &mut Reader, from: SocketAddr) -> Option<()> {
        let (s, h, reason) = (r.u32()?, r.u8()? as usize, r.str()?);
        if s != self.params.session || self.host_of(from) != Some(h) {
            return None;
        }
        if self.end.is_none() {
            let name = self.name_of(h);
            self.end = Some(End::Left { host: h, name, reason });
            self.bye_sent = true;
        }
        Some(())
    }

    fn on_data(&mut self, r: &mut Reader, from: SocketAddr, events: &mut Vec<Event>) -> Option<()> {
        let (s, h) = (r.u32()?, r.u8()? as usize);
        if s != self.params.session || h >= self.peers.len() || h == self.me || self.host_of(from) != Some(h) {
            return None;
        }
        self.peers[h].as_mut()?.last_heard = Instant::now();
        let latest = r.u64()?;
        if r.u8()? as usize != self.peers.len() {
            return None;
        }
        let mut acks = Vec::new();
        for _ in 0..self.peers.len() {
            acks.push(r.u64()?);
        }
        let nh = r.u8()? as usize;
        if nh > HASHES {
            return None;
        }
        let mut hashes = Vec::new();
        for _ in 0..nh {
            hashes.push((r.u64()?, r.u64()?));
        }
        let first = r.u64()?;
        let count = r.u8()? as usize;
        if count > MAX_FRAMES || first == 0 || latest > first.saturating_add(1 << 40) {
            return None;
        }
        // The peer cannot be further ahead of us than the link delay and the packets in flight allow.
        let horizon = self.latest + 2 * self.params.delay as u64 + 8 + RETAIN;
        if first > horizon || first + count as u64 > horizon + 1 {
            return None;
        }
        // Everything is parsed against copies first: a packet is taken whole or not at all.
        let block = self.params.block as usize;
        let zeros = vec![0u8; block];
        let mut fresh: Vec<(usize, u64, NodeRec)> = Vec::new();
        let mut prev: BTreeMap<usize, NodeRec> = BTreeMap::new();
        for i in 0..count as u64 {
            let f = first + i;
            for &node in &self.nodes_of[h] {
                let base: Vec<u8> = match prev.get(&node).or_else(|| self.store[node].get(&(f - 1))) {
                    Some(b) if b.tx.len() == block => b.tx.clone(),
                    Some(_) => zeros.clone(),
                    None if f == 1 => zeros.clone(),
                    // Not held (an old acknowledgement): wait for a newer packet.
                    None => return Some(()),
                };
                let rec = decode_rec(r, &base, block)?;
                prev.insert(node, rec.clone());
                fresh.push((node, f, rec));
            }
        }
        if r.left() != 0 {
            return None;
        }
        // Accepted.
        let now = Instant::now();
        self.peer_latest = self.peer_latest.max(latest);
        {
            let p = self.peers[h].as_mut().unwrap();
            p.last_heard = now;
            p.acked = p.acked.max(acks[self.me]);
            for (f, hash) in hashes {
                if f != 0 && !p.compared.contains(&f) {
                    p.hashes.entry(f).or_insert(hash);
                }
            }
            if p.hashes.len() > 64 {
                let first = *p.hashes.keys().next().unwrap();
                p.hashes.remove(&first);
            }
        }
        let mut got_new = false;
        for (node, f, mut rec) in fresh {
            if self.store[node].contains_key(&f) {
                self.stats.dup_recs += 1;
                continue;
            }
            if self.net.faults.corrupt_frame != 0 && f == self.net.faults.corrupt_frame {
                // A test: a record that arrives damaged.
                match rec.tx.first_mut() {
                    Some(b) => *b ^= 0x10,
                    None => rec.start_frame ^= 1,
                }
            }
            self.store[node].insert(f, rec.clone());
            self.advance(node);
            events.push(Event::Rec { node, frame: f, rec });
            got_new = true;
            self.checkpoint(f);
        }
        if got_new {
            let p = self.peers[h].as_mut().unwrap();
            p.owe_ack.get_or_insert(now);
        }
        let frames: Vec<u64> = self.peers[h].as_ref().unwrap().hashes.keys().copied().collect();
        for f in frames {
            self.compare(f);
        }
        self.prune();
        Some(())
    }

    /// Forget records nobody will ask for again.
    fn prune(&mut self) {
        let mine = self.acked_by_all();
        for node in 0..self.n {
            let keep_from = if self.nodes_of[self.me].contains(&node) {
                mine.saturating_sub(1)
            } else {
                self.contig[node].saturating_sub(RETAIN)
            };
            if let Some((&first, _)) = self.store[node].iter().next() {
                if first < keep_from {
                    self.store[node] = self.store[node].split_off(&keep_from);
                }
            }
        }
    }
}
