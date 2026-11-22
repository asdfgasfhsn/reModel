//! The decision rule for an adaptive link delay D (docs/hw/link-board.md section 12.7): the
//! session host watches how often frame boundaries are held and how late the other hosts'
//! packets run, raises D when stalls cluster and lowers it, one frame at a time, after a long
//! calm spell. It only decides; the change itself is agreed at a frame by the records
//! (`NodeRec::change`) and applied by the hub, the same on every host.
//!
//! The rule works on a clock the caller passes in (milliseconds), so tests run it on a
//! made-up one.

use std::collections::VecDeque;

/// The longest D the rule picks, and the shortest it lowers to (a fixed D can be any 1 to 60).
pub const AUTO_MAX: usize = 8;
pub const AUTO_MIN: usize = 2;

/// A frame, ms.
const FRAME_MS: f32 = 1000.0 / 60.0;
/// The window the raise looks at, and the fewest boundaries it needs to say anything.
const RAISE_WINDOW_S: u64 = 4;
const RAISE_MIN_BOUNDARIES: u32 = 120;
/// Held share (percent) over the window above which D goes up; and the lower share above which
/// it goes up when the pings also say the slack is too small.
const RAISE_PCT: f32 = 10.0;
const WEAK_RAISE_PCT: f32 = 3.0;
/// After a change nothing is decided for this long (the window starts empty then too).
const COOLDOWN_MS: u64 = 4_000;
/// D is lowered after this long without a raise, over a window this calm (percent held).
const LOWER_AFTER_MS: u64 = 25_000;
const LOWER_WINDOW_S: u64 = 20;
const LOWER_PCT: f32 = 0.5;
/// A raise within this long of a lowering doubles the wait before the next lowering (up to x8);
/// this long without one forgets the doubling.
const FLAP_MS: u64 = 60_000;
const FORGET_MS: u64 = 150_000;

/// The held share of frame boundaries, by whole seconds of the caller's clock.
#[derive(Clone, Debug, Default)]
pub struct Meter {
    /// (second, boundaries, held).
    buckets: VecDeque<(u64, u32, u32)>,
}

impl Meter {
    /// One boundary passed at `now_ms`; `held` if it had to wait for another host.
    pub fn record(&mut self, now_ms: u64, held: bool) {
        let sec = now_ms / 1000;
        match self.buckets.back_mut() {
            Some(b) if b.0 == sec => {
                b.1 += 1;
                b.2 += held as u32;
            }
            _ => self.buckets.push_back((sec, 1, held as u32)),
        }
        while self.buckets.front().is_some_and(|b| b.0 + 60 < sec) {
            self.buckets.pop_front();
        }
    }

    /// Forget everything (after a change of D: what came before says nothing about the new one).
    pub fn clear(&mut self) {
        self.buckets.clear();
    }

    /// The percent of boundaries held over the last `secs` whole seconds up to `now_ms`, if
    /// they held at least `min` boundaries.
    pub fn share(&self, now_ms: u64, secs: u64, min: u32) -> Option<f32> {
        let sec = now_ms / 1000;
        let (mut b, mut h) = (0u32, 0u32);
        for k in self.buckets.iter().filter(|k| k.0 + secs > sec) {
            b += k.1;
            h += k.2;
        }
        (b >= min).then(|| h as f32 * 100.0 / b as f32)
    }

    /// The share the session reports to the other hosts (the last few seconds), percent.
    pub fn report(&self, now_ms: u64) -> u8 {
        self.share(now_ms, RAISE_WINDOW_S, 30).map_or(0, |p| p.round().min(100.0) as u8)
    }
}

/// What D the pings alone ask for: the 95th percentile of the one-way delay, in frames, plus one.
pub fn delay_for_p95(one_way_ms: f32) -> usize {
    (one_way_ms / FRAME_MS).ceil().max(0.0) as usize + 1
}

/// The session host's decision.
#[derive(Clone, Debug)]
pub struct Auto {
    pub min: usize,
    pub max: usize,
    /// When D last changed (the effect's boundary reached) and when it last went down.
    last_change_ms: Option<u64>,
    last_lower_ms: Option<u64>,
    /// How long to wait after a change before lowering, times the flap backoff.
    backoff: u64,
    last_raise_ms: Option<u64>,
}

impl Default for Auto {
    fn default() -> Self {
        Auto::new(AUTO_MIN, AUTO_MAX)
    }
}

impl Auto {
    pub fn new(min: usize, max: usize) -> Auto {
        Auto {
            min: min.max(1),
            max: max.clamp(min.max(1), 60),
            last_change_ms: None,
            last_lower_ms: None,
            backoff: 1,
            last_raise_ms: None,
        }
    }

    /// D changed at `now_ms` from `old` to `new` (as the boundary reached it).
    pub fn changed(&mut self, now_ms: u64, old: usize, new: usize) {
        self.last_change_ms = Some(now_ms);
        if new < old {
            self.last_lower_ms = Some(now_ms);
        } else if new > old {
            // Raised soon after a lowering: that lowering was a mistake, wait longer for the next.
            if self.last_lower_ms.is_some_and(|t| now_ms.saturating_sub(t) < FLAP_MS) {
                self.backoff = (self.backoff * 2).min(8);
            }
            self.last_raise_ms = Some(now_ms);
        }
    }

    /// The D to change to now, if any, with D = `d` in force: from the held share of this
    /// host (`meter`), the largest share a peer reports and the pings' one-way 95th percentile.
    pub fn decide(
        &mut self,
        now_ms: u64,
        d: usize,
        meter: &Meter,
        peer_pct: u8,
        p95_one_way_ms: Option<f32>,
    ) -> Option<usize> {
        if self.last_change_ms.is_some_and(|t| now_ms.saturating_sub(t) < COOLDOWN_MS) {
            return None;
        }
        // A long calm forgets the doubled wait.
        if self.last_raise_ms.is_none_or(|t| now_ms.saturating_sub(t) > FORGET_MS)
            && self.last_lower_ms.is_none_or(|t| now_ms.saturating_sub(t) > FORGET_MS)
        {
            self.backoff = 1;
        }
        let need = p95_one_way_ms.map(delay_for_p95);
        let recent = meter.share(now_ms, RAISE_WINDOW_S, RAISE_MIN_BOUNDARIES).map(|p| p.max(peer_pct as f32));
        if let Some(p) = recent {
            if d < self.max {
                // Stalls cluster: up one frame, or two when more than 30% of the boundaries are held;
                // a smaller share raises it only when the pings say the slack is short too.
                let step = if p > 30.0 { 2 } else { 1 };
                let weak = p > WEAK_RAISE_PCT && need.is_some_and(|n| n > d);
                if p > RAISE_PCT || weak {
                    let want = if p > RAISE_PCT { (d + step).max(need.unwrap_or(0).min(d + 2)) } else { d + 1 };
                    return Some(want.min(self.max));
                }
            }
        }
        // Lowering: a long spell without a change or a stall, one frame at a time.
        if d > self.min {
            let since = self.last_change_ms.unwrap_or(0);
            let wait = LOWER_AFTER_MS * self.backoff;
            if now_ms.saturating_sub(since) >= wait && peer_pct == 0 {
                let calm = meter.share(now_ms, LOWER_WINDOW_S, 60 * LOWER_WINDOW_S as u32 * 3 / 4);
                if calm.is_some_and(|p| p <= LOWER_PCT) && need.is_none_or(|n| n < d) {
                    return Some(d - 1);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `secs` seconds of 60 boundaries a second from `t0`, holding every `every`th
    /// (0: none), through `auto` at D = `d`; returns the first decision and its time.
    fn run(
        auto: &mut Auto,
        meter: &mut Meter,
        t0: u64,
        secs: u64,
        d: usize,
        every: u32,
        p95: Option<f32>,
    ) -> Option<(u64, usize)> {
        for i in 0..secs * 60 {
            let now = t0 + i * 1000 / 60;
            meter.record(now, every > 0 && (i as u32).is_multiple_of(every));
            if i % 6 == 0 {
                if let Some(n) = auto.decide(now, d, meter, 0, p95) {
                    return Some((now, n));
                }
            }
        }
        None
    }

    #[test]
    fn a_clean_network_never_raises_d() {
        let (mut a, mut m) = (Auto::default(), Meter::default());
        // Pings of 14 ms one way look like a short slack at D = 2, but nothing is held.
        assert_eq!(run(&mut a, &mut m, 0, 120, 2, 0, Some(18.0)), None);
        // One held boundary in 200 is noise.
        assert_eq!(run(&mut a, &mut m, 200_000, 60, 2, 200, Some(18.0)), None);
    }

    #[test]
    fn clustered_stalls_raise_d_within_seconds() {
        let (mut a, mut m) = (Auto::default(), Meter::default());
        let (at, d) = run(&mut a, &mut m, 0, 20, 2, 3, None).expect("a third held");
        assert_eq!(d, 4, "over 30% held: two frames");
        assert!(at <= 4_100, "{at}");
        let (mut a, mut m) = (Auto::default(), Meter::default());
        let (_, d) = run(&mut a, &mut m, 0, 20, 2, 6, None).expect("a sixth held");
        assert_eq!(d, 3, "10 to 30% held: one frame");
        // Up to the pings' D when more is asked, but at most two at once.
        let (mut a, mut m) = (Auto::default(), Meter::default());
        let (_, d) = run(&mut a, &mut m, 0, 20, 2, 5, Some(80.0)).unwrap();
        assert_eq!(d, 4);
        // A few stalls with short slack by the pings: one frame.
        let (mut a, mut m) = (Auto::default(), Meter::default());
        let (_, d) = run(&mut a, &mut m, 0, 30, 2, 25, Some(40.0)).unwrap();
        assert_eq!(d, 3);
        // The same few stalls with slack to spare: nothing.
        let (mut a, mut m) = (Auto::default(), Meter::default());
        assert_eq!(run(&mut a, &mut m, 0, 30, 4, 25, Some(40.0)), None);
    }

    #[test]
    fn a_peers_stalls_count_and_d_is_clamped() {
        let (mut a, mut m) = (Auto::default(), Meter::default());
        for i in 0..300u64 {
            m.record(i * 1000 / 60, false);
        }
        assert_eq!(a.decide(5000, 3, &m, 25, None), Some(4), "the peer is held a quarter of the time");
        assert_eq!(a.decide(5000, 8, &m, 60, None), None, "already at the top");
        assert_eq!(Auto::new(2, 99).max, 60);
        assert_eq!(Auto::new(0, 0).min, 1);
    }

    #[test]
    fn it_waits_after_a_change_and_does_not_flap() {
        let (mut a, mut m) = (Auto::default(), Meter::default());
        let mut t = 0u64;
        let mut d = 2usize;
        let mut changes: Vec<(u64, usize)> = Vec::new();
        // Stormy for 20 s, then calm for 10 minutes, then stormy again for 20 s, then calm.
        for (secs, every) in [(20u64, 3u32), (600, 0), (20, 3), (600, 0)] {
            for i in 0..secs * 60 {
                let now = t + i * 1000 / 60;
                // The storm's effect: held boundaries fall off as D grows (a made-up network).
                let held = every > 0 && (i as u32).is_multiple_of(every) && d < 5;
                m.record(now, held);
                if i % 6 == 0 {
                    if let Some(n) = a.decide(now, d, &m, 0, None) {
                        a.changed(now, d, n);
                        m.clear();
                        changes.push((now, n));
                        d = n;
                    }
                }
            }
            t += secs * 1000;
        }
        // Never twice within the cooldown, only by one down, never beyond the clamp.
        for w in changes.windows(2) {
            assert!(w[1].0 - w[0].0 >= COOLDOWN_MS, "{changes:?}");
            if w[1].1 < w[0].1 {
                assert_eq!(w[1].1 + 1, w[0].1, "{changes:?}");
            }
        }
        assert!(changes.iter().all(|c| (AUTO_MIN..=AUTO_MAX).contains(&c.1)), "{changes:?}");
        // It went up in the first storm, came back down one frame at a time, went up again, and ended low.
        assert!(changes.len() >= 6 && changes.len() <= 12, "{changes:?}");
        assert_eq!(d, AUTO_MIN, "{changes:?}");
    }

    #[test]
    fn a_lowering_that_is_undone_makes_the_next_wait_longer() {
        let mut a = Auto::default();
        a.changed(0, 4, 3);
        a.changed(10_000, 3, 4);
        assert_eq!(a.backoff, 2);
        a.changed(30_000, 4, 3);
        a.changed(40_000, 3, 4);
        assert_eq!(a.backoff, 4);
        // A raise long after a lowering is a new storm, not a flap.
        let mut b = Auto::default();
        b.changed(0, 4, 3);
        b.changed(200_000, 3, 4);
        assert_eq!(b.backoff, 1);
        // With the doubled wait, a calm minute does not lower yet.
        let mut m = Meter::default();
        for i in 0..60 * 40u64 {
            m.record(40_000 + i * 1000 / 60, false);
        }
        assert_eq!(a.decide(80_000, 4, &m, 0, None), None);
        for i in 0..60 * 80u64 {
            m.record(80_000 + i * 1000 / 60, false);
        }
        assert_eq!(a.decide(40_000 + 25_000 * 4 + 1000, 4, &m, 0, None), Some(3));
    }
}
