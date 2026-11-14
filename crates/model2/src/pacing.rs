//! Following a display whose refresh callbacks arrive unevenly (a
//! browser's animation frames, especially on phones): its refresh rate,
//! how many refreshes each callback stands for, and whether to follow it
//! at all (docs/web.md, "Pacing").

/// Model 2 video frame rate (Hz).
pub const FPS: f64 = 57.524;
/// Most emulated frames run per redraw before dropping time.
pub const MAX_CATCH_UP: u32 = 3;

/// Intervals the coarse refresh period is the median of.
const HISTORY: usize = 31;
/// Callbacks the refresh grid is fitted to (about 4 s at 60 Hz).
const MARKS: usize = 256;
/// Callbacks, and refreshes across them, the fit needs at the least.
const MIN_MARKS: usize = 8;
const MIN_SPAN: f64 = 30.0;
/// Callbacks further apart than this (ms) are a gap (a hidden tab), not
/// refreshes to catch up.
const GAP_MS: f64 = 250.0;

/// A display's refreshes, from callbacks that may come early, late or not
/// at all. Each interval between callbacks is a whole number of refreshes
/// (exactly, for callbacks within a quarter of a refresh of theirs: a late
/// one and an early one after it count two, not three), and the period is
/// the slope of a least-squares line through the recent callbacks' times
/// against their refresh numbers, exact to well within their jitter.
pub struct Refresh {
    /// The game's frame rate (frames per second).
    fps: f64,
    /// Recent callback intervals (ms), oldest overwritten first, and their
    /// median: the coarse period, which missed refreshes and jitter barely
    /// move, to start the grid from and to notice another display.
    intervals: [f64; HISTORY],
    len: usize,
    median: f64,
    /// Callbacks in a row the median has been more than `RESET` from the
    /// period.
    away: usize,
    /// The refresh period (ms).
    ms: f64,
    /// Time (ms) since the fit started, and the refresh number of the last
    /// callback in it.
    clock: f64,
    index: u64,
    /// The recent callbacks the grid is fitted to: (refresh number, time).
    marks: [(f64, f64); MARKS],
    marked: usize,
    /// Refreshes counted so far.
    refreshes: u64,
    /// Following the display: game frames per refresh (1, or 1/2 at about
    /// 120 Hz); None to follow the clock.
    follow: Option<f64>,
}

impl Refresh {
    /// Following a display (one game frame per refresh, or per two) when
    /// that is within this of the game's own rate; once following, until
    /// it is outside the wider band.
    const ENTER: (f64, f64) = (0.95, 1.1);
    const LEAVE: (f64, f64) = (0.92, 1.14);
    /// The grid starts over from the coarse period when they differ by
    /// more than this (another display, or a bad start).
    const RESET: f64 = 0.15;

    /// For a game running at `fps` frames per second, starting from a 60 Hz
    /// guess.
    pub fn new(fps: f64) -> Self {
        let mut r = Refresh {
            fps,
            intervals: [0.0; HISTORY],
            len: 0,
            median: 1000.0 / 60.0,
            away: 0,
            ms: 1000.0 / 60.0,
            clock: 0.0,
            index: 0,
            marks: [(0.0, 0.0); MARKS],
            marked: 0,
            refreshes: 0,
            follow: None,
        };
        r.choose();
        r
    }

    /// The refresh period (ms).
    pub fn ms(&self) -> f64 {
        self.ms
    }

    /// The game's speed over its own rate: following the display, the
    /// refresh rate times the frames per refresh; else 1.
    pub fn speed(&self) -> f64 {
        self.follow.map_or(1.0, |k| 1000.0 / self.ms * k / self.fps)
    }

    /// Game frames per refresh when following the display, or None to
    /// follow the clock.
    pub fn per_refresh(&self) -> Option<f64> {
        self.follow
    }

    /// A callback `dt` ms after the last: the refreshes it stands for (1
    /// normally, more after missed ones, 0 for an extra early one).
    pub fn tick(&mut self, dt: f64) -> u64 {
        if dt.is_nan() || dt <= 0.0 || dt > GAP_MS {
            self.restart();
            self.refreshes += 1;
            return 1;
        }
        if dt > 2.0 && dt < 70.0 {
            self.intervals[self.len % HISTORY] = dt;
            self.len += 1;
            let n = self.len.min(HISTORY);
            if n >= 5 {
                let mut v = self.intervals;
                let v = &mut v[..n];
                v.sort_by(f64::total_cmp);
                self.median = v[n / 2];
            }
        }
        // Another display (or a bad start): the median has been well away
        // from the period for as long as it takes to replace it.
        self.away = if (self.ms / self.median - 1.0).abs() > Self::RESET { self.away + 1 } else { 0 };
        if self.away >= HISTORY || self.len == 5 && self.away > 0 {
            self.ms = self.median;
            self.away = 0;
            self.restart();
        }
        self.clock += dt;
        let n = (dt / self.ms).round().max(0.0) as u64;
        self.index += n;
        self.refreshes += n;
        self.mark(self.index as f64, self.clock);
        self.choose();
        n
    }

    /// Game frames that fall due over `n` refreshes just counted by
    /// `tick` (following the display).
    pub fn frames(&self, n: u64) -> u64 {
        match self.follow {
            Some(k) if k < 1.0 => self.refreshes / 2 - (self.refreshes - n) / 2,
            _ => n,
        }
    }

    /// Add a callback at refresh `x`, time `y`, and refit the grid.
    fn mark(&mut self, x: f64, y: f64) {
        self.marks[self.marked % MARKS] = (x, y);
        self.marked += 1;
        let marks = &self.marks[..self.marked.min(MARKS)];
        let oldest = marks.iter().map(|m| m.0).fold(x, f64::min);
        if marks.len() < MIN_MARKS || x - oldest < MIN_SPAN {
            return;
        }
        // Relative to this callback, so the sums stay small.
        let mut s = [0.0f64; 4];
        for &(mx, my) in marks {
            let (dx, dy) = (mx - x, my - y);
            s = [s[0] + dx, s[1] + dy, s[2] + dx * dx, s[3] + dx * dy];
        }
        let n = marks.len() as f64;
        let spread = n * s[2] - s[0] * s[0];
        if spread > 0.0 {
            let ms = (n * s[3] - s[0] * s[1]) / spread;
            if (ms / self.median - 1.0).abs() <= Self::RESET {
                self.ms = ms;
            }
        }
    }

    /// Start the fit over at the next callback.
    fn restart(&mut self) {
        self.clock = 0.0;
        self.index = 0;
        self.marked = 0;
    }

    fn choose(&mut self) {
        let hz = 1000.0 / self.ms;
        let within = |k: f64, (lo, hi): (f64, f64)| (lo..=hi).contains(&(hz * k / self.fps));
        if self.follow.is_some_and(|k| within(k, Self::LEAVE)) {
            return;
        }
        self.follow = [1.0, 0.5].into_iter().find(|&k| within(k, Self::ENTER));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_refreshes() {
        let mut r = Refresh::new(57.524);
        let mut n = 0;
        // 60 Hz with a missed refresh, then a late one and an early one.
        for dt in [16.7, 16.6, 16.7, 16.7, 16.6, 33.4, 16.7, 24.0, 9.4, 16.7] {
            n += r.tick(dt);
        }
        assert_eq!(n, 11);
        assert_eq!(r.per_refresh(), Some(1.0));
        // 120 Hz: a game frame every other refresh.
        let mut r = Refresh::new(57.524);
        let mut frames = 0;
        for _ in 0..240 {
            let n = r.tick(8.33);
            frames += r.frames(n);
        }
        assert_eq!(r.per_refresh(), Some(0.5));
        assert!((115..=121).contains(&frames), "{frames}");
        // 30 Hz: too slow to follow.
        let mut r = Refresh::new(57.524);
        for _ in 0..40 {
            r.tick(33.3);
        }
        assert_eq!(r.per_refresh(), None);
    }

    #[test]
    fn locks_to_the_display() {
        // 59.5 Hz, the callbacks up to 4 ms early or late and one in ten
        // refreshes missed: the period to within 0.1%, and the refreshes
        // counted to within one.
        let period = 1000.0 / 59.5;
        let mut r = Refresh::new(57.524);
        let (mut grid, mut last, mut seed) = (0.0f64, 0.0f64, 1u32);
        let mut random = || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 8) as f64 / (1u32 << 24) as f64
        };
        let mut counted = 0;
        for i in 0..6000 {
            let skip = if random() < 0.1 { 2.0 } else { 1.0 };
            grid += period * skip;
            let t = grid + (random() - 0.5) * 8.0;
            r.tick(t - last);
            last = t;
            if i == 999 {
                counted = r.refreshes as i64 - (grid / period).round() as i64;
            }
        }
        assert!((r.ms() / period - 1.0).abs() < 0.001, "{} ms", r.ms());
        let drift = r.refreshes as i64 - (grid / period).round() as i64 - counted;
        assert!(drift.abs() <= 1, "{drift} refreshes off over 5000 callbacks");
    }
}
