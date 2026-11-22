//! Network link play in the player (docs/play.md, "Network link play"): this
//! window runs one cabinet of a linked game, the others run on other
//! machines, and the link board's blocks travel over UDP in lockstep
//! (`remodel-linknet`). The launcher's panel sets the session up; this is
//! the running side: the frame boundary that may have to wait, and the
//! status line.

use linknet::{Config, NetLink, Notice, Params, Step};
use model2::link::LinkHub;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// The share of frame boundaries that had to wait for another machine over
/// the last `SPAN` seconds, for the status line: sampled now and then from
/// the session's running counts.
#[derive(Default)]
pub struct StallWindow {
    /// (seconds since the session began, boundaries passed, of them held).
    samples: VecDeque<(f64, u64, u64)>,
}

impl StallWindow {
    /// Seconds the share covers.
    pub const SPAN: f64 = 10.0;
    /// Samples are kept this far apart (seconds).
    const EVERY: f64 = 0.25;
    /// The least the window must hold before it says anything: boundaries.
    const MIN: u64 = 60;

    /// The counts at `at` seconds.
    pub fn sample(&mut self, at: f64, boundaries: u64, stalls: u64) {
        if self.samples.back().is_some_and(|b| at - b.0 < Self::EVERY) {
            return;
        }
        self.samples.push_back((at, boundaries, stalls));
        // Keep one sample from before the span, to measure it from.
        while self.samples.len() > 1 && at - self.samples[1].0 >= Self::SPAN {
            self.samples.pop_front();
        }
    }

    /// The fraction of boundaries held over the window, once there are enough.
    pub fn share(&self) -> Option<f64> {
        let (old, new) = (self.samples.front()?, self.samples.back()?);
        let boundaries = new.1.saturating_sub(old.1);
        (boundaries >= Self::MIN).then(|| new.2.saturating_sub(old.2) as f64 / boundaries as f64)
    }
}

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
    began: Instant,
    window: StallWindow,
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
            began: Instant::now(),
            window: StallWindow::default(),
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
                Notice::DelayChange { from, old, d, announced_at } => {
                    println!("net link play: delay {old} -> {d} frames from frame {from} (announced with frame {announced_at})");
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
                self.delay = self.link.delay();
                if let Some(t) = self.waiting_since.take() {
                    self.recent.push((Instant::now(), t.elapsed()));
                }
                self.recent.retain(|(at, _)| at.elapsed() < Duration::from_secs(5));
                let st = &self.link.stalls;
                self.window.sample(self.began.elapsed().as_secs_f64(), st.boundaries, st.stalls);
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

    /// How long the boundary now held has been waiting, if it is.
    pub fn held_for(&self) -> Option<Duration> {
        self.waiting_since.map(|t| t.elapsed())
    }

    /// What the session cost in waiting, for the log when it ends.
    pub fn summary(&self) -> String {
        let st = &self.link.stalls;
        format!(
            "net link play held {} of {} frame boundaries ({:.1}%), {:.0} ms in all, longest {:.0} ms",
            st.stalls,
            st.boundaries,
            st.stalls as f64 * 100.0 / st.boundaries.max(1) as f64,
            st.stall_time.as_secs_f64() * 1000.0,
            st.longest.as_secs_f64() * 1000.0
        )
    }

    /// One line for the status display: players, D, round trip, holds.
    pub fn status(&self) -> String {
        let rtt = self.link.sess.worst_rtt().map_or("-".to_string(), |r| format!("{r:.0} ms"));
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
        // D follows the network unless it was pinned: a change announced and not yet reached shows as the step.
        let delay = match self.link.delay_next() {
            Some(n) => format!("{} -> {n}", self.delay),
            None => self.delay.to_string(),
        };
        format!(
            "{notice}Link  {head} ({})  Delay {delay} frames  round trip {rtt}  {}{}{}",
            who,
            match self.window.share() {
                Some(p) => format!("held {:.0}% of frames (last {:.0} s)", p * 100.0, StallWindow::SPAN),
                None => "held 0% of frames so far".to_string(),
            },
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
        faults: faults_from_env(),
        timeout: Duration::from_secs(5),
        check_every: 60,
    }
}

/// Faults for testing the player's session over loopback, from
/// `REMODEL_NET_FAULTS` (`latency=5,jitter=10,loss=1,dup=0,seed=7`: what this
/// machine sends, as `remodel link-net`'s flags); none when unset.
fn faults_from_env() -> linknet::Faults {
    parse_faults(&std::env::var("REMODEL_NET_FAULTS").unwrap_or_default())
}

fn parse_faults(spec: &str) -> linknet::Faults {
    let mut f = linknet::Faults::default();
    for (k, v) in spec.split(',').filter_map(|kv| kv.split_once('=')) {
        let n: f32 = v.trim().parse().unwrap_or(0.0);
        match k.trim() {
            "latency" => f.latency_ms = n,
            "jitter" => f.jitter_ms = n,
            "loss" => f.loss_pct = n,
            "dup" => f.dup_pct = n,
            "seed" => f.seed = n as u64,
            _ => {}
        }
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stall_share_covers_the_last_ten_seconds() {
        let mut w = StallWindow::default();
        assert_eq!(w.share(), None);
        // 60 boundaries a second; the first 10 s held one in four, then none.
        let (mut b, mut st) = (0u64, 0u64);
        let mut run = |w: &mut StallWindow, from: u32, to: u32, every: u64| {
            for i in from..to {
                b += 1;
                if every > 0 && b % every == 0 {
                    st += 1;
                }
                w.sample(i as f64 / 60.0, b, st);
            }
        };
        run(&mut w, 0, 600, 4);
        let p = w.share().unwrap();
        assert!((p - 0.25).abs() < 0.02, "{p}");
        run(&mut w, 600, 1200, 0);
        assert!(w.share().unwrap() < 0.02, "old stalls fall out of the window");
        // Too little to say anything yet.
        let mut w = StallWindow::default();
        w.sample(0.0, 0, 0);
        w.sample(0.5, 30, 10);
        assert_eq!(w.share(), None);
    }

    #[test]
    fn fault_specs_parse() {
        let f = parse_faults("latency=5, jitter=10,loss=1,seed=7,bogus=1");
        assert_eq!((f.latency_ms, f.jitter_ms, f.loss_pct, f.seed), (5.0, 10.0, 1.0, 7));
        assert!(!parse_faults("").any());
    }
}
