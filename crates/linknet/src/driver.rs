//! The frame-boundary exchange of a networked link: what `LinkHub::exchange`
//! does for machines in one process, with the other hosts' records coming over
//! the session. The host runs its frame, calls [`NetLink::boundary`], and if
//! that says [`Step::Stalled`] runs nothing and calls it again (after other
//! work, or a short sleep) until it says done: the others' records for the
//! frame the delay asks for are then in and the machine goes on, exactly as
//! it would in a same-process run with the same delay.

use crate::session::{End, Event, Session};
use crate::wire::Params;
use model2::bus::Model2Bus as Bus;
use model2::link::{HubLog, LinkHub};
use std::time::{Duration, Instant};

pub enum Step {
    /// The boundary was passed; the hub's log lines.
    Done(HubLog),
    /// The others' records for this boundary have not all arrived.
    Stalled,
    /// The session is over.
    End(End),
}

#[derive(Clone, Debug, Default)]
pub struct StallStats {
    /// Boundaries that had to wait, and the time spent waiting (all, and the longest).
    pub stalls: u64,
    pub stall_time: Duration,
    pub longest: Duration,
    pub boundaries: u64,
}

/// Something the player should tell the user about the session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// A player left (`lost`: its connection went silent); the survivors are agreeing the drop frame.
    Left { host: usize, name: String, lost: bool },
    /// The survivors agreed the drop frame of a player who left: from the next frame its node is frozen.
    Dropped { host: usize, name: String, frame: u64 },
}

pub struct NetLink {
    pub hub: LinkHub,
    pub sess: Session,
    /// What has happened since the player last took it ([`NetLink::take_notices`]).
    notices: Vec<Notice>,
    local: Vec<usize>,
    put_frame: u64,
    waiting_since: Option<Instant>,
    pub stalls: StallStats,
}

impl NetLink {
    pub fn new(mut hub: LinkHub, sess: Session) -> NetLink {
        // Records are kept a while for the session's own resends and checks.
        hub.keep = 0;
        let local = sess.local_nodes().to_vec();
        NetLink {
            hub,
            sess,
            notices: Vec::new(),
            local,
            put_frame: 0,
            waiting_since: None,
            stalls: StallStats::default(),
        }
    }

    pub fn params(&self) -> &Params {
        &self.sess.params
    }

    /// The local nodes (in the order `buses` is given to [`NetLink::boundary`]).
    pub fn local_nodes(&self) -> &[usize] {
        &self.local
    }

    /// The machines have run frame `frame - 1` (`frame` counts from 1, as `Machine::frames`
    /// does): make their records, send them, and pass the boundary if the
    /// others' have arrived.
    pub fn boundary(&mut self, frame: u64, buses: &mut [&mut Bus]) -> Step {
        assert_eq!(buses.len(), self.local.len());
        if self.put_frame != frame {
            for (k, &node) in self.local.iter().enumerate() {
                let rec = self.hub.put_local(node, frame, buses[k]);
                self.sess.publish(node, frame, rec);
            }
            self.put_frame = frame;
            self.stalls.boundaries += 1;
        }
        self.take_events();
        // Only an error (a state mismatch) ends the session: a player who leaves does not.
        if let Some(e) = &self.sess.end {
            return Step::End(e.clone());
        }
        if !self.hub.has_all(frame, &self.local) {
            self.waiting_since.get_or_insert_with(Instant::now);
            return Step::Stalled;
        }
        if let Some(t) = self.waiting_since.take() {
            let d = t.elapsed();
            self.stalls.stalls += 1;
            self.stalls.stall_time += d;
            self.stalls.longest = self.stalls.longest.max(d);
        }
        let mut local: Vec<(usize, &mut Bus)> =
            self.local.iter().copied().zip(buses.iter_mut().map(|b| &mut **b)).collect();
        Step::Done(self.hub.apply(frame, &mut local))
    }

    /// While the machine is not running (a menu over the game): keep the session
    /// alive and take what arrives, so the others see a host that is only paused.
    pub fn keepalive(&mut self) {
        self.take_events();
    }

    /// Pump the session and take what it brought: records into the hub, a player's leaving
    /// into a notice, an agreed drop frame into the hub (the node is frozen after it).
    fn take_events(&mut self) {
        for ev in self.sess.pump() {
            match ev {
                Event::Rec { node, frame, rec } => {
                    self.hub.put(node, frame, rec);
                }
                Event::Left { host, name, lost } => self.notices.push(Notice::Left { host, name, lost }),
                Event::Dropped { host, frame } => {
                    for &node in self.sess.nodes_of_host(host) {
                        self.hub.leave(node, frame);
                    }
                    let name = self.sess.name_of(host);
                    self.notices.push(Notice::Dropped { host, name, frame });
                }
            }
        }
    }

    /// The notices since the last call.
    pub fn take_notices(&mut self) -> Vec<Notice> {
        std::mem::take(&mut self.notices)
    }

    /// After the last boundary: keep the session going until the other hosts
    /// hold everything of ours they will need (or `limit` passes), then say goodbye.
    /// Returns when it is time to stop.
    pub fn linger(&mut self, last: u64, limit: Duration) {
        let t = Instant::now();
        while t.elapsed() < limit && self.sess.end.is_none() && self.sess.acked_by_all() < last {
            let _ = self.sess.pump();
            std::thread::sleep(Duration::from_micros(200));
        }
        self.sess.bye("finished");
    }
}
