//! Network link play in the player (docs/play.md, "Network link play"): this
//! window runs one cabinet of a linked game, the others run on other
//! machines, and the link board's blocks travel over UDP in lockstep
//! (`remodel-linknet`). The launcher's panel sets the session up; this is
//! the running side: the frame boundary that may have to wait, and the
//! status line.

use linknet::{Config, NetLink, Notice, Params, Step};
use model2::link::LinkHub;
use std::time::{Duration, Instant};

/// The running session of this window's machine.
pub struct NetPlay {
    pub link: NetLink,
    /// This machine's node (0 is the master's), the nodes in all, and D.
    pub node: usize,
    pub nodes: usize,
    pub delay: usize,
    pub names: Vec<String>,
    /// The machine has run its frame and is waiting for the other hosts' records.
    pub pending: bool,
    /// Why the session ended, once it has.
    pub ended: Option<String>,
    /// Redraws that held the frame for the network, and since when the
    /// current wait began.
    pub held: u64,
    waiting_since: Option<Instant>,
    /// The longest wait of the last few seconds, for the status line.
    recent: Vec<(Instant, Duration)>,
    /// The hosts that have left (the others carry on without them), and the latest
    /// notice for the status line with the time it came.
    pub gone: Vec<usize>,
    flash: Option<(String, Instant)>,
}

/// How long a notice stays in the status line.
const FLASH: Duration = Duration::from_secs(8);

impl NetPlay {
    pub fn new(sess: linknet::Session, params: &Params, play: &romset::LinkPlay) -> NetPlay {
        let node = params.hosts[sess.me].nodes[0] as usize;
        let sess_names = sess.names().to_vec();
        let hub = LinkHub::for_game(params.nodes(), params.delay as usize, play);
        NetPlay {
            link: NetLink::new(hub, sess),
            node,
            nodes: params.nodes(),
            delay: params.delay as usize,
            names: sess_names,
            pending: false,
            ended: None,
            held: 0,
            waiting_since: None,
            recent: Vec::new(),
            gone: Vec::new(),
            flash: None,
        }
    }

    /// The machine has run frame `frame - 1`: pass the boundary if the others'
    /// records are in. Returns true when it was passed (the next frame may
    /// run), false if the frame is held or the session ended.
    pub fn boundary(&mut self, m: &mut model2::Machine) -> bool {
        let r = self.boundary_inner(m);
        self.collect_notices();
        r
    }

    /// While the machine is paused (a menu over the game): keep the session alive.
    pub fn keepalive(&mut self) {
        self.link.keepalive();
        self.collect_notices();
        if let Some(e) = self.link.sess.end.clone() {
            self.ended.get_or_insert(e.to_string());
        }
    }

    /// A player who left does not end the game: the status line says so for a few seconds.
    fn collect_notices(&mut self) {
        for n in self.link.take_notices() {
            match n {
                Notice::Left { host, name, lost } => {
                    if !self.gone.contains(&host) {
                        self.gone.push(host);
                    }
                    let alone = self.link.sess.alone();
                    let what = match (lost, alone) {
                        (false, false) => format!("{name} left - racing on"),
                        (false, true) => format!("{name} left - racing on alone"),
                        (true, false) => format!("{name}'s connection was lost - racing on"),
                        (true, true) => format!("{name}'s connection was lost - racing on alone"),
                    };
                    println!("net link play: {what}");
                    self.flash = Some((what, Instant::now()));
                }
                Notice::Dropped { name, frame, .. } => {
                    println!("net link play: {name} dropped at frame {frame} (agreed by the players left)");
                }
            }
        }
    }

    fn boundary_inner(&mut self, m: &mut model2::Machine) -> bool {
        match self.link.boundary(m.frames, &mut [&mut m.bus]) {
            Step::Done(log) => {
                for l in log {
                    println!("link          frame {}: {l}", m.frames);
                }
                self.pending = false;
                if let Some(t) = self.waiting_since.take() {
                    self.recent.push((Instant::now(), t.elapsed()));
                }
                self.recent.retain(|(at, _)| at.elapsed() < Duration::from_secs(5));
                true
            }
            Step::Stalled => {
                self.pending = true;
                self.held += 1;
                self.waiting_since.get_or_insert_with(Instant::now);
                false
            }
            Step::End(e) => {
                self.ended = Some(e.to_string());
                false
            }
        }
    }

    /// One line for the status display: players, D, round trip, holds.
    pub fn status(&self) -> String {
        let rtt = self.link.sess.worst_rtt().map_or("-".to_string(), |r| format!("{r:.0} ms"));
        let st = &self.link.stalls;
        let worst = self.recent.iter().map(|(_, d)| *d).max().unwrap_or_default();
        let waiting = self.waiting_since.is_some_and(|t| t.elapsed() > Duration::from_millis(250));
        let me = self.link.sess.me;
        let others: Vec<&str> = self
            .names
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != me && !self.gone.contains(i))
            .map(|(_, n)| n.as_str())
            .collect();
        let who = if others.is_empty() { "alone".to_string() } else { others.join(", ") };
        let left: Vec<&str> = self.gone.iter().filter_map(|&i| self.names.get(i)).map(|n| n.as_str()).collect();
        let notice = match &self.flash {
            Some((t, at)) if at.elapsed() < FLASH => format!("{t}   "),
            _ => String::new(),
        };
        let players = self.nodes - self.gone.len();
        let head = if players == 1 { "1 player".to_string() } else { format!("{players} players") };
        format!(
            "{notice}Link  {head} ({})  D {}  round trip {rtt}  held {}x ({:.1} s){}{}",
            who,
            self.delay,
            st.stalls,
            st.stall_time.as_secs_f64(),
            if worst > Duration::from_millis(50) {
                format!("  worst hold {} ms", worst.as_millis())
            } else {
                String::new()
            },
            if !waiting {
                if left.is_empty() {
                    String::new()
                } else {
                    format!("  left: {}", left.join(", "))
                }
            } else if others.len() == 1 {
                format!("  WAITING for {who}")
            } else {
                "  WAITING for the other players".to_string()
            },
        )
    }
}

/// The session's configuration for `game` on this machine: what a host and a
/// joiner must agree on (docs/hw/link-board.md section 12).
pub fn config(game: &romset::Game, roms: &romset::RomSet, play: &romset::LinkPlay, name: String) -> Config {
    Config {
        game: game.id.to_string(),
        rom_hash: linknet::rom_hash(roms),
        version: env!("CARGO_PKG_VERSION").to_string(),
        name,
        block: play.block as u16,
        max_hosts: play.max_nodes,
        faults: linknet::Faults::default(),
        timeout: Duration::from_secs(5),
        check_every: 60,
    }
}
