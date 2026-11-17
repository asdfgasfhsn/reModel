//! Whether the game follows the display or the clock: the "behind" test.
//!
//! Following the display (a game frame per one or two refreshes) is the
//! smooth way to play and the only one that keeps frames evenly spaced,
//! but it needs each frame's work to fit in the frame time. When it
//! doesn't, the clock paces the game instead (frames as fast as they can be
//! made, no in-betweens), and run-ahead is dropped if that goes on.
//!
//! The frame cost of a real game wanders by several milliseconds with what
//! is on screen, so a cost near the budget would flip the player between
//! the two every few seconds, each flip a visible change of rhythm. The
//! test therefore has margins and patience both ways: it switches to the
//! clock only when the smoothed cost stays over the budget (and a margin)
//! for [`ENTER`] frames, and back only when it stays well under it for
//! [`LEAVE`] frames, and a cost between the two keeps whichever is on.

/// The smoothed cost over this share of the budget counts as too slow.
pub const OVER: f64 = 1.03;
/// ... and under this share, as fast enough again.
pub const UNDER: f64 = 0.8;
/// Game frames the cost must stay over (about 1.5 s) before the clock paces.
pub const ENTER: u32 = 90;
/// Game frames it must stay under (about 4 s) before the display does again.
pub const LEAVE: u32 = 240;
/// Weight of each redraw's cost in the smoothed cost.
const SMOOTHING: f64 = 0.05;

/// A change of the pacing, for the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Too slow to follow the display: the clock paces.
    Behind,
    /// Following the display again.
    Synced,
}

#[derive(Default, Debug)]
pub struct Sync {
    /// Host time per game frame, smoothed, in seconds (0 until measured).
    pub cost: f64,
    /// Paced by the clock because the cost is too high to sync.
    pub behind: bool,
    /// Frames in a row the cost has been on the wrong side of the margin
    /// for the current mode (over when synced, under when behind).
    streak: u32,
    /// Frames run since `behind` was last clear (run-ahead's grace).
    pub behind_frames: u32,
}

impl Sync {
    /// Take the cost of `ran` (more than 0) frames, `cost` seconds each,
    /// against the display's frame time `budget`; says if the pacing
    /// changed.
    pub fn note(&mut self, cost: f64, ran: u32, budget: f64) -> Option<Change> {
        self.cost = if self.cost == 0.0 { cost } else { self.cost + (cost - self.cost) * SMOOTHING };
        let wrong = if self.behind { self.cost < budget * UNDER } else { self.cost > budget * OVER };
        self.streak = if wrong { self.streak.saturating_add(ran) } else { 0 };
        self.behind_frames = if self.behind { self.behind_frames.saturating_add(ran) } else { 0 };
        let patience = if self.behind { LEAVE } else { ENTER };
        if self.streak < patience {
            return None;
        }
        self.streak = 0;
        self.behind = !self.behind;
        self.behind_frames = 0;
        Some(if self.behind { Change::Behind } else { Change::Synced })
    }

    /// Forget everything measured (a new game, or run-ahead given up).
    pub fn reset(&mut self) {
        *self = Sync::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUDGET: f64 = 1.0 / 60.0;

    /// Feed `n` frames of `cost` (as a share of the budget); the changes.
    fn run(s: &mut Sync, share: f64, n: u32) -> Vec<Change> {
        (0..n).filter_map(|_| s.note(share * BUDGET, 1, BUDGET)).collect()
    }

    #[test]
    fn cheap_frames_stay_synced() {
        let mut s = Sync::default();
        assert!(run(&mut s, 0.6, 1000).is_empty());
        assert!(!s.behind);
    }

    #[test]
    fn a_cost_just_under_the_budget_does_not_switch() {
        // 16.03 ms against 16.67 was enough to switch before.
        let mut s = Sync::default();
        assert!(run(&mut s, 0.96, 3000).is_empty());
        assert!(!s.behind);
    }

    #[test]
    fn sustained_overload_switches_once_after_the_patience() {
        let mut s = Sync::default();
        run(&mut s, 0.7, 100);
        assert!(run(&mut s, 1.3, ENTER - 20).is_empty(), "not at once");
        let changes = run(&mut s, 1.3, 400);
        assert_eq!(changes, vec![Change::Behind]);
        assert!(s.behind);
    }

    #[test]
    fn a_short_spike_is_ignored() {
        let mut s = Sync::default();
        run(&mut s, 0.7, 100);
        assert!(run(&mut s, 2.0, 30).is_empty());
        assert!(run(&mut s, 0.7, 500).is_empty());
        assert!(!s.behind);
    }

    #[test]
    fn behind_stays_until_well_under_the_budget() {
        let mut s = Sync::default();
        run(&mut s, 1.3, 400);
        assert!(s.behind);
        // Under the budget, but not by the margin: still the clock.
        assert!(run(&mut s, 0.9, 2000).is_empty());
        assert!(s.behind);
        // Well under, for long enough: the display again, once.
        assert_eq!(run(&mut s, 0.6, 1000), vec![Change::Synced]);
        assert!(!s.behind);
    }

    #[test]
    fn a_cost_wandering_around_the_budget_never_flaps() {
        // The cost swings 0.85 to 1.15 of the budget every 2 s, around the
        // budget itself: the old test switched at each swing.
        let mut s = Sync::default();
        let mut changes = 0;
        for i in 0..20_000u32 {
            let phase = (i as f64 / 120.0 * std::f64::consts::TAU).sin();
            changes += s.note((1.0 + 0.15 * phase) * BUDGET, 1, BUDGET).is_some() as u32;
        }
        assert_eq!(changes, 0);
    }

    #[test]
    fn a_noisy_cost_with_a_high_mean_switches_once_and_stays() {
        let mut s = Sync::default();
        let mut changes = Vec::new();
        let mut x = 12345u32;
        for _ in 0..20_000 {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            let noise = (x >> 16) as f64 / 65536.0 - 0.5; // -0.5..0.5
            changes.extend(s.note((1.2 + noise) * BUDGET, 1, BUDGET));
        }
        assert_eq!(changes, vec![Change::Behind]);
    }

    #[test]
    fn behind_frames_count_only_while_behind() {
        let mut s = Sync::default();
        run(&mut s, 0.7, 50);
        assert_eq!(s.behind_frames, 0);
        run(&mut s, 1.5, 300);
        assert!(s.behind);
        let n = s.behind_frames;
        assert!(n > 0);
        run(&mut s, 1.5, 10);
        assert_eq!(s.behind_frames, n + 10);
    }
}
