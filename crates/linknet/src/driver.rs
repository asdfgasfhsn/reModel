//! The frame-boundary exchange of a networked link: what `LinkHub::exchange`
//! does for machines in one process, with the other hosts' records coming over
//! the session. The host runs its frame, calls [`NetLink::boundary`], and if
//! that says [`Step::Stalled`] runs nothing and calls it again (after other
//! work, or a short sleep) until it says done: the others' records for the
//! frame the delay asks for are then in and the machine goes on, exactly as
//! it would in a same-process run with the same delay.

use crate::adapt::{Auto, Meter};
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
    /// The session host announced that D becomes `d` from boundary `from` (every host switches
    /// there); `old` is the D in force before the change.
    DelayChange { from: u64, old: usize, d: usize, announced_at: u64 },
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
    /// The session host's rule for an adaptive D (None: D is fixed, or this host does not decide),
    /// and a scripted plan of `(frame, D)` announcements (a test).
    pub auto: Option<Auto>,
    pub plan: Vec<(u64, usize)>,
    /// The held share of boundaries, for the rule and for the other hosts.
    pub meter: Meter,
    began: Instant,
    /// The D in force at the last boundary passed.
    cur_d: usize,
    /// The delay changes the hub knows, as `(from, d)`, for the session's log.
    pub history: Vec<(u64, usize)>,
    seen_changes: usize,
}

impl NetLink {
    pub fn new(mut hub: LinkHub, sess: Session) -> NetLink {
        // Records are kept a while for the session's own resends and checks.
        hub.keep = 0;
        let local = sess.local_nodes().to_vec();
        let cur_d = hub.delay;
        let sess_auto = sess.adaptive.filter(|_| sess.me == 0).map(|(lo, hi)| Auto::new(lo, hi));
        NetLink {
            hub,
            sess,
            notices: Vec::new(),
            local,
            put_frame: 0,
            waiting_since: None,
            stalls: StallStats::default(),
            auto: sess_auto,
            plan: Vec::new(),
            meter: Meter::default(),
            began: Instant::now(),
            cur_d,
            history: Vec::new(),
            seen_changes: 0,
        }
    }

    fn now_ms(&self) -> u64 {
        self.began.elapsed().as_millis() as u64
    }

    /// The delay D in force at the last boundary this host passed.
    pub fn delay(&self) -> usize {
        self.cur_d
    }

    /// The D it will be, when a change is announced and not yet reached.
    pub fn delay_next(&self) -> Option<usize> {
        self.hub.changes().last().filter(|c| c.from > self.put_frame).map(|c| c.d)
    }

    /// Whether this host is the one that decides D (the session host).
    pub fn decides(&self) -> bool {
        self.sess.me == 0
    }

    /// The change this host announces with its record of `frame`, if any: `(from, D)`.
    fn pick_change(&mut self, frame: u64) -> Option<(u64, u8)> {
        // One change at a time: the last one has taken effect and this host has noticed (the
        // boundary that reaches it passes after this record is made, so wait a frame more).
        if !self.decides() || self.hub.change_pending(frame) || self.cur_d != self.hub.delay_at(frame) || frame < 2 {
            return None;
        }
        let d = self.hub.delay_at(frame);
        let want = match self.plan.first().copied() {
            Some((at, n)) if at <= frame => {
                self.plan.remove(0);
                Some(n)
            }
            Some(_) => None,
            None => {
                let now = self.now_ms();
                let (peer, p95) = (self.sess.peer_held_pct(), self.sess.one_way_p95());
                match self.auto.as_mut() {
                    Some(a) => a.decide(now, d, &self.meter, peer, p95),
                    None => None,
                }
            }
        };
        let n = want?.clamp(1, 60);
        // Every other host holds this record before it passes the boundary D - 1 frames on, so
        // `from` a frame or two beyond that reaches no host too late.
        (n != d).then_some((frame + d as u64 + 1, n as u8))
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
            let change = self.pick_change(frame);
            for (k, &node) in self.local.clone().iter().enumerate() {
                let mut rec = self.hub.put_local(node, frame, buses[k]);
                if let (0, Some((from, d))) = (k, change) {
                    rec.change = Some((from, d));
                    self.hub.schedule_delay(from, d as usize);
                    self.note_changes(frame);
                }
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
        let held = self.waiting_since.take().map(|t| {
            let d = t.elapsed();
            self.stalls.stalls += 1;
            self.stalls.stall_time += d;
            self.stalls.longest = self.stalls.longest.max(d);
        });
        let now = self.now_ms();
        let d_now = self.hub.delay_at(frame);
        if d_now != self.cur_d {
            // The change takes effect at this boundary: what came before says nothing of the new D.
            if let Some(a) = self.auto.as_mut() {
                a.changed(now, self.cur_d, d_now);
            }
            self.meter.clear();
            self.cur_d = d_now;
        }
        self.meter.record(now, held.is_some());
        self.sess.held_pct = self.meter.report(now);
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
                    self.note_changes(frame);
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

    /// Notice the delay changes the hub has learnt since last looked.
    fn note_changes(&mut self, announced_at: u64) {
        while self.seen_changes < self.hub.changes().len() {
            let c = self.hub.changes()[self.seen_changes];
            let old = if self.seen_changes == 0 { self.hub.delay } else { self.hub.changes()[self.seen_changes - 1].d };
            self.seen_changes += 1;
            self.history.push((c.from, c.d));
            self.notices.push(Notice::DelayChange { from: c.from, old, d: c.d, announced_at });
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
