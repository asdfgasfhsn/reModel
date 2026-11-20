//! Setting a session up: one host waits in a lobby, the others join it by
//! address, then the host fixes what everyone runs (the delay, the node
//! assignment, the master's settings) and the session starts. Both sides
//! are state machines polled from the caller's loop (a headless run or a
//! window), nothing here blocks.
//!
//! Refused with a clear message: another game, other ROMs, another build,
//! another protocol version, a full lobby, a session already started, and (at
//! the start) a joiner whose settings are not what the host sent.
//!
//! Hosts are found two ways, merged by the caller ([`crate::mdns::merge`]): the
//! mDNS advert ([`crate::mdns`]) and the broadcast [`Discovery`] here.

use crate::mdns::{Advert, Lobby, Txt};
use crate::names::{display_names, sanitize_name};
use crate::net::{Faults, Net};
use crate::session::Session;
use crate::wire::*;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

/// The port a host listens on unless told another; hosts on it are found by [`Discovery`].
pub const DEFAULT_PORT: u16 = 47890;

/// What both ends must agree on, and what each brings.
#[derive(Clone, Debug)]
pub struct Config {
    pub game: String,
    /// A hash of the program ROMs (the same ROMs on every machine).
    pub rom_hash: u64,
    /// The reModel version (the same build on every machine).
    pub version: String,
    pub name: String,
    /// Bytes in a link block.
    pub block: u16,
    /// The most hosts the game can link.
    pub max_hosts: usize,
    pub faults: Faults,
    /// Silence that ends a session or a lobby.
    pub timeout: Duration,
    /// Frames between state checks.
    pub check_every: u16,
}

const RETRY: Duration = Duration::from_millis(150);

fn send(net: &mut Net, to: SocketAddr, w: Writer) {
    net.send(to, &w.0);
}

struct Joiner {
    addr: SocketAddr,
    name: String,
    last_heard: Instant,
    rtt_ms: Option<f32>,
    /// The setup hash it reported at the start.
    acked: Option<u64>,
}

/// What a host's lobby is doing.
pub enum HostStatus {
    Lobby,
    /// The start is sent, the joiners are checking their settings.
    Starting,
    Ready(Box<Session>, Params),
    Failed(String),
}

enum HostPhase {
    Lobby,
    Starting { params: Params, expect: Vec<u64>, since: Instant, last: Instant },
    Done,
}

pub struct Host {
    net: Net,
    cfg: Config,
    joiners: Vec<Joiner>,
    session: u32,
    phase: HostPhase,
    local: SocketAddr,
    pings: Instant,
    advert: Option<Advert>,
    /// The state last sent as the advert (to send only changes).
    advert_note: Option<String>,
    /// Names of joiners who left or went silent since the last poll's check.
    gone: Vec<String>,
}

impl Host {
    pub fn bind(mut cfg: Config, addr: SocketAddr) -> std::io::Result<Host> {
        cfg.name = crate::names::name_or_default(&cfg.name);
        let sock = UdpSocket::bind(addr)?;
        let local = sock.local_addr()?;
        let net = Net::new(sock, cfg.faults.clone())?;
        let session =
            (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos())
                ^ std::process::id().rotate_left(16))
            .max(1);
        Ok(Host {
            net,
            cfg,
            joiners: Vec::new(),
            session,
            phase: HostPhase::Lobby,
            local,
            pings: Instant::now(),
            advert: None,
            advert_note: None,
            gone: Vec::new(),
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    /// Advertise this lobby over mDNS (`_remodel._udp`) while it is open and has a free
    /// seat. On failure (no multicast) the error is returned and the broadcast answer
    /// carries on alone.
    pub fn advertise(&mut self) -> Result<(), String> {
        self.advert = Some(Advert::start(self.local.port())?);
        self.update_advert();
        Ok(())
    }

    /// Whether the mDNS advert is out now.
    pub fn advertised(&self) -> bool {
        self.advert.as_ref().is_some_and(|a| a.is_advertised())
    }

    /// This lobby's name for itself, as the advert and the broadcast answer give it.
    pub fn txt(&self) -> Txt {
        Txt {
            game: self.cfg.game.clone(),
            version: self.cfg.version.clone(),
            rom_hash: self.cfg.rom_hash,
            players: self.players() as u8,
            max: self.cfg.max_hosts as u8,
            delay: self.suggest_delay().min(60) as u8,
            lobby: match self.phase {
                HostPhase::Lobby if self.players() >= self.cfg.max_hosts => Lobby::Full,
                HostPhase::Lobby => Lobby::Open,
                _ => Lobby::Starting,
            },
            name: self.cfg.name.clone(),
        }
    }

    /// Bring the advert in line with the lobby: out while it is open with a seat free,
    /// withdrawn once it is full or starting.
    fn update_advert(&mut self) {
        let t = self.txt();
        let Some(a) = self.advert.as_mut() else { return };
        let want = (t.lobby == Lobby::Open).then_some(&t);
        let key = format!("{want:?}");
        if self.advert_note.as_deref() == Some(&key) {
            return;
        }
        self.advert_note = Some(key);
        if let Err(e) = a.set(want) {
            eprintln!("link: mDNS advert failed ({e}); hosts are still found by broadcast");
            self.advert = None;
        }
    }

    /// This host's name.
    pub fn name(&self) -> &str {
        &self.cfg.name
    }

    /// The players now: this host and the joiners.
    pub fn players(&self) -> usize {
        1 + self.joiners.len()
    }

    /// The joiners' names (as shown: two the same are told apart) and round trips (ms).
    pub fn joiners(&self) -> Vec<(String, Option<f32>)> {
        let all = self.names();
        self.joiners.iter().zip(&all[1..]).map(|(j, n)| (n.clone(), j.rtt_ms)).collect()
    }

    /// Every player's name as shown, the host first.
    pub fn names(&self) -> Vec<String> {
        let raw: Vec<String> =
            std::iter::once(self.cfg.name.clone()).chain(self.joiners.iter().map(|j| j.name.clone())).collect();
        display_names(&raw)
    }

    /// The delay D to start with: frames for half the worst round trip between two
    /// hosts (each joiner's half to the host, two of them in a row), plus one; 1 at least.
    pub fn suggest_delay(&self) -> usize {
        let mut halves: Vec<f32> = self.joiners.iter().map(|j| j.rtt_ms.unwrap_or(0.0) / 2.0).collect();
        halves.sort_by(|a, b| b.partial_cmp(a).unwrap());
        let one_way = match halves.as_slice() {
            [] => 0.0,
            [a] => *a,
            [a, b, ..] => a + b,
        };
        delay_for(one_way)
    }

    fn welcome(&mut self, to: SocketAddr, idx: usize) {
        let mut w = Writer::new(kind::WELCOME);
        w.u32(self.session).u8(idx as u8);
        send(&mut self.net, to, w);
    }

    fn refuse(&mut self, to: SocketAddr, why: &str) {
        let mut w = Writer::new(kind::REFUSE);
        w.str(why);
        send(&mut self.net, to, w);
    }

    /// Fix the session and send it to the joiners. `delay`: D, or None for the
    /// suggestion; `base`: the master's settings record; `setup_hash(params, host)`:
    /// what host `host`'s settings must hash to for these params (checked against what
    /// each joiner reports).
    pub fn start(
        &mut self,
        delay: Option<usize>,
        base: Option<Vec<u8>>,
        setup_hash: &dyn Fn(&Params, usize) -> u64,
    ) -> Result<(), String> {
        if !matches!(self.phase, HostPhase::Lobby) {
            return Err("already started".into());
        }
        if self.joiners.is_empty() {
            return Err("no one has joined".into());
        }
        let delay = delay.unwrap_or_else(|| self.suggest_delay()).clamp(1, 60);
        let mut hosts = vec![HostEntry { addr: self.local.to_string(), name: self.cfg.name.clone(), nodes: vec![0] }];
        for (i, j) in self.joiners.iter().enumerate() {
            hosts.push(HostEntry { addr: j.addr.to_string(), name: j.name.clone(), nodes: vec![(i + 1) as u8] });
        }
        let params = Params {
            session: self.session,
            game: self.cfg.game.clone(),
            delay: delay as u8,
            check_every: self.cfg.check_every,
            timeout_ms: self.cfg.timeout.as_millis() as u32,
            block: self.cfg.block,
            hosts,
            base,
        };
        let expect = (0..params.hosts.len()).map(|h| setup_hash(&params, h)).collect();
        self.phase = HostPhase::Starting { params, expect, since: Instant::now(), last: Instant::now() - RETRY };
        Ok(())
    }

    pub fn poll(&mut self) -> HostStatus {
        self.update_advert();
        self.net.flush();
        while let Some((bytes, from)) = self.net.recv() {
            self.on_packet(&bytes, from);
        }
        let now = Instant::now();
        let timeout = self.cfg.timeout;
        for j in self.joiners.iter().filter(|j| now.duration_since(j.last_heard) >= timeout) {
            self.gone.push(j.name.clone());
        }
        self.joiners.retain(|j| now.duration_since(j.last_heard) < timeout);
        let lost = !self.gone.is_empty();
        if matches!(self.phase, HostPhase::Lobby) {
            if now.duration_since(self.pings) >= Duration::from_millis(300) {
                self.pings = now;
                let n = self.joiners.len() + 1;
                let names = self.names();
                for k in 0..self.joiners.len() {
                    let to = self.joiners[k].addr;
                    let mut w = Writer::new(kind::PING);
                    w.u32(self.session).u8(0).u32(self.epoch_ms());
                    send(&mut self.net, to, w);
                    let mut w = Writer::new(kind::LOBBY);
                    w.u32(self.session).u8(n as u8);
                    for name in &names {
                        w.str(name);
                    }
                    send(&mut self.net, to, w);
                }
            }
            return HostStatus::Lobby;
        }
        let names = {
            let raw: Vec<String> =
                std::iter::once(self.cfg.name.clone()).chain(self.joiners.iter().map(|j| j.name.clone())).collect();
            display_names(&raw)
        };
        let HostPhase::Starting { params, expect, since, last } = &mut self.phase else { return HostStatus::Lobby };
        if lost {
            let why = format!("{} left while the session was starting", self.gone.join(", "));
            return self.abort(&why);
        }
        if now.duration_since(*since) > Duration::from_secs(10).max(timeout) {
            return self.abort("the players did not all answer the start in time");
        }
        // Check what has been reported.
        for (i, j) in self.joiners.iter().enumerate() {
            if let Some(h) = j.acked {
                if h != expect[i + 1] {
                    let why = format!(
                        "{}'s game settings differ from the ones sent (hash {h:016x}, expected {:016x}): refusing to start",
                        names[i + 1],
                        expect[i + 1]
                    );
                    return self.abort(&why);
                }
            }
        }
        if now.duration_since(*last) >= RETRY {
            *last = now;
            let mut w = Writer::new(kind::START);
            params.encode(&mut w);
            for j in self.joiners.iter().filter(|j| j.acked.is_none()) {
                self.net.send(j.addr, &w.0);
            }
        }
        if self.joiners.iter().all(|j| j.acked.is_some()) {
            let HostPhase::Starting { params, .. } = std::mem::replace(&mut self.phase, HostPhase::Done) else {
                unreachable!()
            };
            let addrs: Vec<SocketAddr> =
                std::iter::once(self.local).chain(self.joiners.iter().map(|j| j.addr)).collect();
            let mut w = Writer::new(kind::GO);
            w.u32(self.session);
            for j in &self.joiners {
                for _ in 0..3 {
                    self.net.send(j.addr, &w.0);
                }
            }
            let sock = self.net.sock.try_clone().expect("socket");
            let net = Net::new(sock, self.cfg.faults.clone()).expect("socket");
            return HostStatus::Ready(Box::new(Session::new(net, 0, params.clone(), addrs)), params);
        }
        HostStatus::Starting
    }

    /// Close the lobby: the joiners are told, the advert withdrawn and the port is let go.
    pub fn close(&mut self) {
        self.advert = None;
        if matches!(self.phase, HostPhase::Lobby | HostPhase::Starting { .. }) {
            self.abort("the host closed the lobby");
        }
    }

    fn epoch_ms(&self) -> u32 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u32)
    }

    fn abort(&mut self, why: &str) -> HostStatus {
        let mut w = Writer::new(kind::ABORT);
        w.u32(self.session).str(why);
        for j in &self.joiners {
            for _ in 0..3 {
                let _ = self.net.sock.send_to(&w.0, j.addr);
            }
        }
        self.phase = HostPhase::Done;
        HostStatus::Failed(why.to_string())
    }

    fn on_packet(&mut self, bytes: &[u8], from: SocketAddr) {
        let Some((ver, k, mut r)) = header(bytes) else { return };
        if k == kind::DISCOVER {
            if let HostPhase::Lobby = self.phase {
                let t = self.txt();
                let mut w = Writer::new(kind::INFO);
                w.str(&t.game)
                    .str(&t.name)
                    .u8(t.players)
                    .u8(t.max)
                    .u16(self.local.port())
                    .str(&t.version)
                    .u64(t.rom_hash)
                    .u8(t.delay);
                send(&mut self.net, from, w);
            }
            return;
        }
        if k == kind::HELLO {
            if ver != PROTO {
                return self.refuse(
                    from,
                    &format!(
                        "your reModel speaks link protocol {ver}, this host's speaks {PROTO}: use the same reModel version ({} here)",
                        self.cfg.version
                    ),
                );
            }
            return self.on_hello(&mut r, from);
        }
        if ver != PROTO {
            return;
        }
        let (now_ms, session) = (self.epoch_ms(), self.session);
        let Some(j) = self.joiners.iter_mut().find(|j| j.addr == from) else { return };
        j.last_heard = Instant::now();
        match k {
            kind::PING => {
                let (Some(s), Some(_), Some(t)) = (r.u32(), r.u8(), r.u32()) else { return };
                if s == self.session {
                    let mut w = Writer::new(kind::PONG);
                    w.u32(s).u8(0).u32(t);
                    send(&mut self.net, from, w);
                }
            }
            kind::PONG => {
                let (Some(s), Some(_), Some(t)) = (r.u32(), r.u8(), r.u32()) else { return };
                let rtt = now_ms.wrapping_sub(t) as f32;
                if s == session && rtt < 60_000.0 {
                    j.rtt_ms = Some(j.rtt_ms.map_or(rtt, |o| o * 0.7 + rtt * 0.3));
                }
            }
            kind::START_ACK => {
                let (Some(s), Some(_), Some(h)) = (r.u32(), r.u8(), r.u64()) else { return };
                if s == session && j.acked.is_none() {
                    j.acked = Some(h);
                }
            }
            kind::BYE => {
                // A joiner leaving the lobby.
                if matches!(self.phase, HostPhase::Starting { .. }) {
                    self.gone.push(j.name.clone());
                }
                self.joiners.retain(|j| j.addr != from);
            }
            _ => {}
        }
    }

    fn on_hello(&mut self, r: &mut Reader, from: SocketAddr) {
        let (Some(game), Some(rom), Some(version), Some(name)) = (r.str(), r.u64(), r.str(), r.str()) else { return };
        if let Some(i) = self.joiners.iter().position(|j| j.addr == from) {
            self.joiners[i].last_heard = Instant::now();
            return self.welcome(from, i + 1);
        }
        let why = if !matches!(self.phase, HostPhase::Lobby) {
            Some("the session has already started".to_string())
        } else if game != self.cfg.game {
            Some(format!("this host is playing {}, you chose {game}", self.cfg.game))
        } else if rom != self.cfg.rom_hash {
            Some("your ROMs are not the same as the host's (the program ROM hashes differ)".to_string())
        } else if version != self.cfg.version {
            Some(format!("reModel {version} here, {} on the host: use the same version", self.cfg.version))
        } else if self.players() >= self.cfg.max_hosts {
            Some(format!("{} can link at most {} cabinets and they are all taken", self.cfg.game, self.cfg.max_hosts))
        } else {
            None
        };
        if let Some(why) = why {
            return self.refuse(from, &why);
        }
        let now = Instant::now();
        self.joiners.push(Joiner {
            addr: from,
            name: crate::names::name_or_default(&name),
            last_heard: now,
            rtt_ms: None,
            acked: None,
        });
        let idx = self.joiners.len();
        self.welcome(from, idx);
    }
}

/// D for a one-way delay of `ms`: frames of it, plus one for the frame's own length.
pub fn delay_for(one_way_ms: f32) -> usize {
    ((one_way_ms / (1000.0 / 60.0)).ceil() as usize + 1).max(1)
}

/// What a joiner is doing.
pub enum JoinStatus {
    Connecting,
    /// In the host's lobby, with this many players so far.
    Lobby {
        players: usize,
        /// Everyone's names as shown, the host first (empty until the host has sent them).
        names: Vec<String>,
    },
    /// The host has started: check the settings and call [`Join::accept`] with their hash.
    Params(Box<Params>),
    /// Accepted; waiting for the host's go.
    Waiting,
    Ready(Box<Session>, Params),
    Failed(String),
}

enum JoinPhase {
    Hello,
    Lobby { idx: usize, players: usize, names: Vec<String> },
    Params(Params),
    Accepted { params: Params, hash: u64, last: Instant },
    Done,
}

pub struct Join {
    net: Net,
    cfg: Config,
    host: SocketAddr,
    session: u32,
    phase: JoinPhase,
    since: Instant,
    last: Instant,
    last_heard: Instant,
    reported: bool,
    /// Our index among the hosts, as the welcome said.
    idx: usize,
}

impl Join {
    pub fn connect(mut cfg: Config, host: SocketAddr) -> std::io::Result<Join> {
        cfg.name = crate::names::name_or_default(&cfg.name);
        let any: SocketAddr = if host.is_ipv4() { "0.0.0.0:0".parse().unwrap() } else { "[::]:0".parse().unwrap() };
        let sock = UdpSocket::bind(any)?;
        let net = Net::new(sock, cfg.faults.clone())?;
        let now = Instant::now();
        Ok(Join {
            net,
            cfg,
            host,
            session: 0,
            phase: JoinPhase::Hello,
            since: now,
            last: now - RETRY,
            last_heard: now,
            reported: false,
            idx: 0,
        })
    }

    /// Our index among the hosts (valid once welcomed).
    pub fn index(&self) -> usize {
        self.idx
    }

    pub fn host_addr(&self) -> SocketAddr {
        self.host
    }

    /// Settings checked: report their hash and wait for the go.
    pub fn accept(&mut self, hash: u64) {
        if let JoinPhase::Params(p) = &self.phase {
            self.phase = JoinPhase::Accepted { params: p.clone(), hash, last: Instant::now() - RETRY };
        }
    }

    /// Leave (in the lobby, politely).
    pub fn leave(&mut self) {
        let mut w = Writer::new(kind::BYE);
        w.u32(self.session).u8(0).str("left");
        let _ = self.net.sock.send_to(&w.0, self.host);
        self.phase = JoinPhase::Done;
    }

    pub fn poll(&mut self) -> JoinStatus {
        self.net.flush();
        let mut go = false;
        while let Some((bytes, from)) = self.net.recv() {
            if from != self.host {
                continue;
            }
            let Some((ver, k, mut r)) = header(&bytes) else { continue };
            if ver != PROTO {
                if matches!(self.phase, JoinPhase::Hello) {
                    self.phase = JoinPhase::Done;
                    return JoinStatus::Failed(format!(
                        "the host runs another version of reModel (link protocol {ver}, yours is {PROTO}): use the same version on every machine"
                    ));
                }
                continue;
            }
            self.last_heard = Instant::now();
            match k {
                kind::WELCOME => {
                    if let (Some(s), Some(idx)) = (r.u32(), r.u8()) {
                        if matches!(self.phase, JoinPhase::Hello) {
                            self.session = s;
                            self.idx = idx as usize;
                            self.phase =
                                JoinPhase::Lobby { idx: idx as usize, players: idx as usize + 1, names: Vec::new() };
                        }
                    }
                }
                kind::REFUSE => {
                    if let Some(why) = r.str() {
                        self.phase = JoinPhase::Done;
                        return JoinStatus::Failed(format!("the host refused: {why}"));
                    }
                }
                kind::LOBBY => {
                    if let (Some(s), Some(n)) = (r.u32(), r.u8()) {
                        if let JoinPhase::Lobby { players, names, .. } = &mut self.phase {
                            if s == self.session && (1..=MAX_HOSTS as u8).contains(&n) {
                                *players = n as usize;
                                let list: Option<Vec<String>> = (0..n).map(|_| r.str()).collect();
                                if let Some(l) = list {
                                    *names = l.iter().map(|x| sanitize_name(x)).collect();
                                }
                            }
                        }
                    }
                }
                kind::PING => {
                    if let (Some(s), Some(_), Some(t)) = (r.u32(), r.u8(), r.u32()) {
                        if s == self.session {
                            let mut w = Writer::new(kind::PONG);
                            w.u32(s).u8(0).u32(t);
                            send(&mut self.net, from, w);
                        }
                    }
                }
                kind::START => {
                    if let JoinPhase::Lobby { idx, .. } = &self.phase {
                        let idx = *idx;
                        if let Some(p) = Params::decode(&mut r) {
                            let ours = p.hosts.get(idx);
                            if p.session == self.session
                                && ours.is_some()
                                && p.game == self.cfg.game
                                && p.block == self.cfg.block
                            {
                                self.phase = JoinPhase::Params(p);
                                self.reported = false;
                            }
                        }
                    }
                }
                kind::GO => {
                    if let Some(s) = r.u32() {
                        go |= s == self.session;
                    }
                }
                kind::DATA => go |= matches!(self.phase, JoinPhase::Accepted { .. }),
                kind::ABORT => {
                    if let (Some(_), Some(why)) = (r.u32(), r.str()) {
                        self.phase = JoinPhase::Done;
                        return JoinStatus::Failed(format!("the host aborted the start: {why}"));
                    }
                }
                _ => {}
            }
        }
        let now = Instant::now();
        match &mut self.phase {
            JoinPhase::Hello => {
                if now.duration_since(self.since) > Duration::from_secs(5) {
                    self.phase = JoinPhase::Done;
                    return JoinStatus::Failed(format!(
                        "no answer from {}: is the host running, the address and port right, and its firewall open for UDP?",
                        self.host
                    ));
                }
                if now.duration_since(self.last) >= RETRY {
                    self.last = now;
                    let mut w = Writer::new(kind::HELLO);
                    w.str(&self.cfg.game).u64(self.cfg.rom_hash).str(&self.cfg.version).str(&self.cfg.name);
                    send(&mut self.net, self.host, w);
                }
                JoinStatus::Connecting
            }
            JoinPhase::Lobby { players, names, .. } => {
                let (players, names) = (*players, names.clone());
                if now.duration_since(self.last_heard) > self.cfg.timeout {
                    self.phase = JoinPhase::Done;
                    return JoinStatus::Failed("the host went away".into());
                }
                JoinStatus::Lobby { players, names }
            }
            JoinPhase::Params(p) => {
                if self.reported {
                    return JoinStatus::Waiting;
                }
                self.reported = true;
                JoinStatus::Params(Box::new(p.clone()))
            }
            JoinPhase::Accepted { params, hash, last } => {
                if go {
                    let params = params.clone();
                    return self.finish(params);
                }
                if now.duration_since(self.last_heard) > self.cfg.timeout {
                    self.phase = JoinPhase::Done;
                    return JoinStatus::Failed("the host went away before the start".into());
                }
                if now.duration_since(*last) >= RETRY {
                    *last = now;
                    let mut w = Writer::new(kind::START_ACK);
                    w.u32(self.session).u8(self.idx as u8).u64(*hash);
                    let hashv = w.0.clone();
                    self.net.send(self.host, &hashv);
                }
                JoinStatus::Waiting
            }
            JoinPhase::Done => JoinStatus::Failed("not connected".into()),
        }
    }

    fn finish(&mut self, params: Params) -> JoinStatus {
        let idx = self.idx;
        let mut addrs: Vec<SocketAddr> = Vec::new();
        for (h, e) in params.hosts.iter().enumerate() {
            if h == 0 {
                addrs.push(self.host);
            } else {
                match e.addr.parse() {
                    Ok(a) => addrs.push(a),
                    Err(_) => {
                        self.phase = JoinPhase::Done;
                        return JoinStatus::Failed("the host sent a bad address".into());
                    }
                }
            }
        }
        self.phase = JoinPhase::Done;
        let sock = self.net.sock.try_clone().expect("socket");
        let net = Net::new(sock, self.cfg.faults.clone()).expect("socket");
        JoinStatus::Ready(Box::new(Session::new(net, idx, params.clone(), addrs)), params)
    }
}

/// A host found on the network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// Where to join it.
    pub addr: SocketAddr,
    pub game: String,
    /// The host's user name.
    pub name: String,
    pub players: usize,
    pub max: usize,
    pub version: String,
    /// The program ROM hash the host's agreement compares (None if only the
    /// join itself can tell).
    pub rom_hash: Option<u64>,
    /// The delay D the host would start with.
    pub delay: u8,
    pub lobby: Lobby,
    /// All the addresses the host was heard at (the one in `addr` included).
    pub addrs: Vec<IpAddr>,
    /// By which way it was heard.
    pub mdns: bool,
    pub broadcast: bool,
}

impl Found {
    /// Whether `o` is this same host: the same port and the same address, or an
    /// address the other one lists too.
    pub fn same_host(&self, o: &Found) -> bool {
        self.addr.port() == o.addr.port()
            && (self.addr.ip() == o.addr.ip() || self.addrs.contains(&o.addr.ip()) || o.addrs.contains(&self.addr.ip()))
    }
}

/// Looking for hosts: a datagram to the broadcast address (or a given one) that
/// every host in a lobby on the default port answers.
pub struct Discovery {
    sock: UdpSocket,
    target: SocketAddr,
    last: Option<Instant>,
    seen: Vec<(Found, Instant)>,
    pub found: Vec<Found>,
}

impl Discovery {
    /// Look on the local network (hosts on [`DEFAULT_PORT`]).
    pub fn broadcast() -> std::io::Result<Discovery> {
        Discovery::to(SocketAddr::from(([255, 255, 255, 255], DEFAULT_PORT)))
    }

    pub fn to(target: SocketAddr) -> std::io::Result<Discovery> {
        let sock = UdpSocket::bind("0.0.0.0:0")?;
        sock.set_broadcast(true)?;
        sock.set_nonblocking(true)?;
        Ok(Discovery { sock, target, last: None, seen: Vec::new(), found: Vec::new() })
    }

    /// Ask again every half second and collect the answers.
    pub fn poll(&mut self) -> &[Found] {
        if self.last.is_none_or(|t| t.elapsed() >= Duration::from_millis(500)) {
            self.last = Some(Instant::now());
            let _ = self.sock.send_to(Writer::new(kind::DISCOVER).0.as_slice(), self.target);
        }
        let mut buf = [0u8; 512];
        while let Ok((n, from)) = self.sock.recv_from(&mut buf) {
            let Some((ver, k, mut r)) = header(&buf[..n]) else { continue };
            if ver != PROTO || k != kind::INFO {
                continue;
            }
            if let (
                Some(game),
                Some(name),
                Some(players),
                Some(max),
                Some(port),
                Some(version),
                Some(rom),
                Some(delay),
            ) = (r.str(), r.str(), r.u8(), r.u8(), r.u16(), r.str(), r.u64(), r.u8())
            {
                let addr = SocketAddr::new(from.ip(), port);
                let f = Found {
                    addr,
                    game,
                    name: sanitize_name(&name),
                    players: players as usize,
                    max: max as usize,
                    version,
                    rom_hash: Some(rom),
                    delay,
                    lobby: if players >= max { Lobby::Full } else { Lobby::Open },
                    addrs: vec![from.ip()],
                    mdns: false,
                    broadcast: true,
                };
                self.seen.retain(|(o, _)| o.addr != f.addr);
                self.seen.push((f, Instant::now()));
            }
        }
        self.seen.retain(|(_, t)| t.elapsed() < Duration::from_millis(1800));
        self.found = self.seen.iter().map(|(f, _)| f.clone()).collect();
        &self.found
    }
}
