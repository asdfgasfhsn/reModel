//! Network link play in the player (docs/play.md, "Network link play"): this
//! window runs one cabinet of a linked game, the others run on other
//! machines, and the link board's blocks travel over UDP in lockstep
//! (`remodel-linknet`). The launcher's panel sets the session up; this is
//! the running side: the frame boundary that may have to wait, and the
//! status line.

use linknet::{Config, NetLink, Params, Step};
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
}

impl NetPlay {
    pub fn new(sess: linknet::Session, params: &Params, play: &romset::LinkPlay) -> NetPlay {
        let node = params.hosts[sess.me].nodes[0] as usize;
        let hub = LinkHub::for_game(params.nodes(), params.delay as usize, play);
        NetPlay {
            link: NetLink::new(hub, sess),
            node,
            nodes: params.nodes(),
            delay: params.delay as usize,
            names: params.hosts.iter().map(|h| h.name.clone()).collect(),
            pending: false,
            ended: None,
            held: 0,
            waiting_since: None,
            recent: Vec::new(),
        }
    }

    /// The machine has run frame `frame - 1`: pass the boundary if the others'
    /// records are in. Returns true when it was passed (the next frame may
    /// run), false if the frame is held or the session ended.
    pub fn boundary(&mut self, m: &mut model2::Machine) -> bool {
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
        format!(
            "Link  {} players  D {}  round trip {rtt}  held {}x ({:.1} s){}{}",
            self.nodes,
            self.delay,
            st.stalls,
            st.stall_time.as_secs_f64(),
            if worst > Duration::from_millis(50) {
                format!("  worst hold {} ms", worst.as_millis())
            } else {
                String::new()
            },
            if waiting { "  WAITING for the other player" } else { "" },
        )
    }
}

/// The session's configuration for `game` on this machine: what a host and a
/// joiner must agree on (docs/hw/link-board.md section 12).
pub fn config(game: &romset::Game, roms: &romset::RomSet, play: &romset::LinkPlay, name: String) -> Config {
    Config {
        game: game.id.to_string(),
        rom_hash: linknet::rom_hash(&roms.program),
        version: env!("CARGO_PKG_VERSION").to_string(),
        name,
        block: play.block as u16,
        max_hosts: play.max_nodes,
        faults: linknet::Faults::default(),
        timeout: Duration::from_secs(5),
        check_every: 60,
    }
}

/// A name for this player in the lobby.
pub fn player_name() -> String {
    std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "player".into())
}
