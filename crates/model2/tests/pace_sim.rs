//! A deterministic model of a browser page's audio path (docs/web.md,
//! "Audio rate"): the page's frame loop as `crates/web/src/lib.rs` runs it,
//! following the display (`model2::pacing::Refresh`) or the clock, the
//! audio rate control (`model2::pacing::Pace`) setting the sound worker's
//! resampler step, and the AudioWorklet's queue
//! (`crates/web/audio-worklet.js`) draining at the device's clock, driven
//! by synthetic animation-frame timings. The controller before the phone
//! fix is kept here (`Before`) to compare against. `make pace-sim` prints
//! the report; the tests hold the controller to it.

use model2::audio::RATE;
use model2::pacing::{Pace, Refresh};

const FPS: f64 = 57.524;
const MAX_CATCH_UP: u32 = 3;
/// Frames a game may run late, before and after: the page now catches up
/// from a stall of up to 100 ms.
const MAX_OWED: u64 = 2;
const MAX_OWED_AFTER: u64 = 6;
const STALE_MS: f64 = 250.0;
/// Audio render quantum (frames) and the worklet's level report interval
/// (quanta).
const QUANTUM: f64 = 128.0;
const LEVEL_EVERY: u64 = 8;

/// Animation frame intervals (ms), each from the generator.
type Intervals = Box<dyn FnMut(&mut Rng) -> f64>;

/// A small deterministic generator (xorshift64*).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.next()
    }
}

/// The audio buffering a page uses (seconds): the rate control's target,
/// the worklet's priming and its most.
#[derive(Clone, Copy)]
pub struct Buffer {
    target: f64,
    prime: f64,
    max: f64,
}
const DESKTOP: Buffer = Buffer { target: 0.07, prime: 0.06, max: 0.25 };
const PHONE: Buffer = Buffer { target: 0.15, prime: 0.13, max: 0.4 };

/// How a page paces the game and its audio.
trait Control {
    /// An animation frame `dt` ms after the last, at `t`: the frames due
    /// (all told, given `frames` run and `owed`), whether it follows the
    /// display, the frames that fell due in `dt`, and the speed the game
    /// is meant to run at.
    fn plan(&mut self, t: f64, dt: f64, frames: u64, owed: &mut u64, epoch: f64) -> (u64, bool, f64, f64);
    /// Whether frames left due after a long interval are dropped time
    /// rather than the game falling behind.
    fn drops_stalls(&self) -> bool {
        false
    }
    /// The resampler step after the frame ran `ran`.
    fn step(&mut self, dt: f64, speed: f64, ran: f64, due: f64, queued: Option<f64>, target: f64) -> f64;
}

/// The page and `Pace` before the phone fix: the refresh as an average of
/// intervals, one frame per animation frame when following it, and the
/// step its speed times a pace over 150 ms with a queue correction up to
/// 20%.
struct Before {
    refresh_ms: f64,
    redraws: u64,
    ran: [f64; 2],
    due: [f64; 2],
    level: Option<f64>,
}

impl Before {
    fn new() -> Self {
        Before { refresh_ms: 1000.0 / 60.0, redraws: 0, ran: [60.0; 2], due: [60.0; 2], level: None }
    }
}

impl Control for Before {
    fn plan(&mut self, t: f64, dt: f64, frames: u64, owed: &mut u64, epoch: f64) -> (u64, bool, f64, f64) {
        if dt > 2.0 && dt < 50.0 {
            self.refresh_ms += (dt - self.refresh_ms) * 0.05;
        }
        self.redraws += 1;
        let hz = 1000.0 / self.refresh_ms;
        match [1.0, 0.5].into_iter().find(|k| (0.95..=1.1).contains(&(hz * k / FPS))) {
            Some(k) => {
                let next = (k == 1.0 || self.redraws.is_multiple_of(2)) as u64;
                *owed = (*owed + next).min(MAX_OWED);
                let speed = hz * k / FPS;
                (frames + *owed, true, dt * FPS * speed / 1000.0, speed)
            }
            None => (((t - epoch) * FPS / 1000.0) as u64, false, dt * FPS / 1000.0, 1.0),
        }
    }

    fn step(&mut self, dt: f64, speed: f64, ran: f64, due: f64, queued: Option<f64>, target: f64) -> f64 {
        let gap = dt > 0.1;
        let dt = dt.clamp(0.001, 0.1);
        let a = (dt / 0.15f64).min(1.0);
        if !gap {
            for (s, n) in [(&mut self.ran, ran), (&mut self.due, due)] {
                s[0] += (n / dt - s[0]) * a;
                s[1] += (s[0] - s[1]) * a;
            }
        }
        let pace = if self.due[1] > 0.0 { (self.ran[1] / self.due[1]).clamp(0.25, 1.5) } else { 1.0 };
        self.level = queued.map(|q| match self.level {
            Some(l) => l + (q - l) * (dt / 0.15).min(1.0),
            None => q,
        });
        let factor = match self.level {
            None => pace,
            Some(level) => {
                let e = ((level - target) / target).clamp(-1.0, 1.0);
                let beyond = (e.abs() - 0.2).max(0.0) / 0.8;
                let extra = if e < 0.0 { -beyond * 0.2 } else { beyond * 0.03 };
                pace * (1.0 + e * 0.005 + extra)
            }
        };
        speed * factor
    }
}

/// The page now (`App::tick`).
struct After {
    refresh: Refresh,
    pace: Pace,
}

impl Control for After {
    fn plan(&mut self, t: f64, dt: f64, frames: u64, owed: &mut u64, epoch: f64) -> (u64, bool, f64, f64) {
        let n = self.refresh.tick(dt);
        match self.refresh.per_refresh() {
            Some(_) => {
                let next = self.refresh.frames(n);
                // Frames past what may be owed after a stall (a game that
                // was keeping up) are time dropped, not the game falling
                // behind.
                let over = (*owed + next).saturating_sub(MAX_OWED_AFTER);
                let counted = if *owed == 0 { next - over } else { next };
                *owed = (*owed + next).min(MAX_OWED_AFTER);
                (frames + *owed, true, counted as f64, self.refresh.speed())
            }
            None => (((t - epoch) * FPS / 1000.0) as u64, false, dt * FPS / 1000.0, 1.0),
        }
    }

    fn drops_stalls(&self) -> bool {
        true
    }

    fn step(&mut self, dt: f64, speed: f64, ran: f64, due: f64, queued: Option<f64>, target: f64) -> f64 {
        self.pace.update(dt, speed, ran, due, queued, target)
    }
}

struct Scenario {
    name: &'static str,
    seconds: f64,
    /// The next animation frame interval (ms), from a fresh generator.
    interval: fn() -> Intervals,
    /// The fraction of real time the game can run at, at time `t` (s): the
    /// geometry worker's throughput.
    capacity: fn(f64) -> f64,
    /// Render quanta the device runs back to back (1 on desktop; phones
    /// render larger hardware buffers in bursts).
    burst: u64,
    /// The device's audio clock against the page's (1 + this).
    drift: f64,
    /// Whether this is a phone (the phone buffer, after the fix).
    phone: bool,
    /// Where to measure pitch (s).
    measure: (f64, f64),
}

#[derive(Default, Debug)]
struct Report {
    /// Pitch against its mean over the measured part (cents).
    max_cents: f64,
    rms_cents: f64,
    /// Pitch movement over 100 ms (cents a second): fastest and 99th
    /// percentile.
    max_slew: f64,
    p99_slew: f64,
    /// The audio breaking off (the queue running out while playing).
    underruns_per_min: f64,
    /// The mean step in the measured part, and the game's real speed there
    /// (frames run a second over its own rate).
    mean_step: f64,
    game_speed: f64,
}

fn simulate(s: &Scenario, control: &mut dyn Control, buf: Buffer) -> Report {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut interval = (s.interval)();
    let rate = RATE as f64;
    // The page.
    let (mut t, mut redraws, mut owed, mut frames, mut epoch) = (0.0f64, 0u64, 0u64, 0u64, 0.0f64);
    let (mut step, mut sent_since) = (1.0f64, 0.0f64);
    let mut audio_level: Option<(f64, f64)> = None;
    // Frames the geometry worker has ready.
    let mut geo = 3.0f64;
    // Audio on its way to the worklet (arrival ms, frames), and reports
    // on their way back (arrival ms, level).
    let mut to_worklet: std::collections::VecDeque<(f64, f64)> = Default::default();
    let mut to_page: std::collections::VecDeque<(f64, f64)> = Default::default();
    // The worklet.
    let (mut queue, mut primed, mut dropouts, mut quanta, mut next_burst) = (0.0f64, false, 0u64, 0u64, 0.0f64);
    let quantum_ms = QUANTUM / rate * 1000.0 / (1.0 + s.drift);
    let mut started_at = None;
    // Measurements.
    let mut samples: Vec<(f64, f64)> = Vec::new();
    let mut ran_measured = 0u64;

    while t < s.seconds * 1000.0 {
        let dt = interval(&mut rng);
        let t_next = t + dt;
        // The worklet's render quanta until the next animation frame.
        while next_burst < t_next {
            while to_worklet.front().is_some_and(|f| f.0 <= next_burst) {
                queue += to_worklet.pop_front().unwrap().1;
                queue = queue.min(buf.max * rate);
            }
            for _ in 0..s.burst {
                if !primed && queue >= buf.prime * rate {
                    primed = true;
                    started_at.get_or_insert(next_burst);
                }
                if primed {
                    if queue >= QUANTUM {
                        queue -= QUANTUM;
                    } else {
                        queue = 0.0;
                        primed = false;
                        dropouts += 1;
                    }
                }
                quanta += 1;
                if quanta.is_multiple_of(LEVEL_EVERY) {
                    to_page.push_back((next_burst + 1.0, queue));
                }
            }
            next_burst += quantum_ms * s.burst as f64;
        }
        t = t_next;
        while to_page.front().is_some_and(|r| r.0 <= t) {
            let (at, level) = to_page.pop_front().unwrap();
            audio_level = Some((level, at));
            sent_since = 0.0;
        }

        // The animation frame.
        redraws += 1;
        let (due, follow, fell_due, speed) = control.plan(t, dt, frames, &mut owed, epoch);
        let capacity = (s.capacity)(t / 1000.0);
        geo = if capacity >= 1.0 { 3.0 } else { (geo + dt / 1000.0 * FPS * 1.05 * capacity).min(2.0) };
        let mut ran = 0u32;
        while frames < due && ran < MAX_CATCH_UP && geo >= 1.0 {
            geo -= 1.0;
            frames += 1;
            ran += 1;
            // The frame's audio, resampled with the step it was sent with,
            // reaches the worklet once the sound worker has rendered it.
            let out = rate / FPS / step;
            to_worklet.push_back((t + 3.0, out));
            sent_since += out;
        }
        owed = owed.saturating_sub(ran as u64);
        let mut fell_due = fell_due;
        if control.drops_stalls() && !follow && frames < due && dt > 2000.0 / FPS {
            fell_due = (fell_due - (due - frames) as f64).max(ran as f64);
        }
        if redraws > 1 {
            let queued = audio_level.and_then(|(level, at)| {
                let age = t - at;
                (age < STALE_MS).then(|| (level + sent_since) / rate - age / 1000.0)
            });
            step = control.step(dt / 1000.0, speed, ran as f64, fell_due, queued, buf.target);
        }
        if follow || frames < due {
            epoch = t - frames as f64 * 1000.0 / FPS;
        }
        let secs = t / 1000.0;
        if secs >= s.measure.0 && secs < s.measure.1 {
            samples.push((secs, step));
            ran_measured += ran as u64;
        }
    }

    let n = samples.len().max(1) as f64;
    let mean = samples.iter().map(|s| s.1).sum::<f64>() / n;
    let cents = |r: f64| 1200.0 * (r / mean).log2();
    let mut slews = Vec::new();
    let mut j = 0;
    for i in 0..samples.len() {
        while j < samples.len() && samples[j].0 < samples[i].0 + 0.1 {
            j += 1;
        }
        if j < samples.len() {
            let d = 1200.0 * (samples[j].1 / samples[i].1).log2();
            slews.push(d.abs() / (samples[j].0 - samples[i].0));
        }
    }
    slews.sort_by(f64::total_cmp);
    let pick = |q: f64| slews.get(((slews.len() as f64 - 1.0) * q) as usize).copied().unwrap_or(0.0);
    let minutes = (t - started_at.unwrap_or(0.0)) / 60_000.0;
    Report {
        max_cents: samples.iter().map(|s| cents(s.1).abs()).fold(0.0, f64::max),
        rms_cents: (samples.iter().map(|s| cents(s.1).powi(2)).sum::<f64>() / n).sqrt(),
        max_slew: pick(1.0),
        p99_slew: pick(0.99),
        underruns_per_min: dropouts as f64 / minutes.max(1e-9),
        mean_step: mean,
        game_speed: ran_measured as f64 / (s.measure.1 - s.measure.0) / FPS,
    }
}

/// About `HZ`, with a little noise.
fn steady<const HZ: u32>() -> Intervals {
    Box::new(|r| 1000.0 / HZ as f64 + r.range(-0.3, 0.3))
}

/// A phone's animation frames (WebKit on iOS): 60 Hz refreshes delivered
/// up to 4 ms early or late (8 to 33 ms apart), 8% of them missed, and a
/// 50 to 100 ms stall every 2 to 6 seconds (the page busy, thermal
/// throttling), after which the page catches up.
fn phone() -> Intervals {
    let refresh = 1000.0 / 60.0;
    let (mut at, mut stall_at, mut offset) = (0.0f64, 3000.0f64, 0.0f64);
    Box::new(move |r| {
        let before = at + offset;
        at += if r.next() < 0.08 { 2.0 } else { 1.0 } * refresh;
        if at > stall_at {
            // Refreshes go on: it resumes on one.
            at += (r.range(50.0, 100.0) / refresh).round() * refresh;
            stall_at = at + r.range(2000.0, 6000.0);
        }
        offset = r.range(-4.0, 4.0);
        (at + offset - before).max(8.0)
    })
}

fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "steady 60 Hz",
            seconds: 120.0,
            interval: steady::<60>,
            capacity: |_| 1.0,
            burst: 1,
            drift: 0.0003,
            phone: false,
            measure: (5.0, 120.0),
        },
        Scenario {
            name: "phone jitter",
            seconds: 300.0,
            interval: phone,
            capacity: |_| 1.0,
            burst: 4,
            drift: 0.0003,
            phone: true,
            measure: (5.0, 300.0),
        },
        Scenario {
            name: "85% for a minute",
            seconds: 100.0,
            interval: steady::<60>,
            capacity: |t| if (10.0..70.0).contains(&t) { 0.85 } else { 1.0 },
            burst: 1,
            drift: 0.0003,
            phone: false,
            measure: (25.0, 70.0),
        },
        Scenario {
            name: "30 Hz low power",
            seconds: 120.0,
            interval: steady::<30>,
            capacity: |_| 1.0,
            burst: 4,
            drift: 0.0003,
            phone: true,
            measure: (5.0, 120.0),
        },
        Scenario {
            name: "varying 80-100%",
            seconds: 180.0,
            interval: steady::<60>,
            capacity: varying,
            burst: 1,
            drift: 0.0003,
            phone: false,
            measure: (5.0, 180.0),
        },
        Scenario {
            name: "phone, 85%",
            seconds: 100.0,
            interval: phone,
            capacity: |t| if (10.0..70.0).contains(&t) { 0.85 } else { 1.0 },
            burst: 4,
            drift: 0.0003,
            phone: true,
            measure: (25.0, 70.0),
        },
    ]
}

/// A game whose cost changes with the scene (sgt24h in a race): for 2 to
/// 6 s at a time, 80%, 88%, 95% or all of real time.
fn varying(t: f64) -> f64 {
    let (mut at, mut k) = (0.0, 0u64);
    loop {
        let h = k.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 40;
        at += 2.0 + (h % 5) as f64;
        if t < at {
            return [1.0, 0.95, 0.88, 0.8][(h >> 8) as usize % 4];
        }
        k += 1;
    }
}

fn before(s: &Scenario) -> Report {
    simulate(s, &mut Before::new(), DESKTOP)
}

fn after(s: &Scenario) -> Report {
    let mut c = After { refresh: Refresh::new(FPS), pace: Pace::default() };
    simulate(s, &mut c, if s.phone { PHONE } else { DESKTOP })
}

#[test]
#[ignore = "a report: make pace-sim"]
fn pace_report() {
    println!(
        "{:<18} {:<7} {:>7} {:>7} {:>8} {:>8} {:>11} {:>6} {:>6}",
        "scenario", "", "max ¢", "rms ¢", "max ¢/s", "p99 ¢/s", "underruns/m", "step", "game"
    );
    for s in scenarios() {
        for (which, r) in [("before", before(&s)), ("after", after(&s))] {
            println!(
                "{:<18} {:<7} {:>7.1} {:>7.1} {:>8.0} {:>8.0} {:>11.1} {:>6.3} {:>6.3}",
                s.name,
                which,
                r.max_cents,
                r.rms_cents,
                r.max_slew,
                r.p99_slew,
                r.underruns_per_min,
                r.mean_step,
                r.game_speed
            );
        }
    }
}

fn scenario(name: &str) -> Scenario {
    scenarios().into_iter().find(|s| s.name == name).unwrap()
}

#[test]
fn steady_pitch_while_keeping_up() {
    // A steady display, a phone's uneven one and a phone in low power mode:
    // the pitch within a few cents, never breaking up.
    for name in ["steady 60 Hz", "phone jitter", "30 Hz low power"] {
        let r = after(&scenario(name));
        assert!(r.max_cents < 5.0 && r.rms_cents < 2.0, "{name}: {r:?}");
        assert!(r.p99_slew < 10.0, "{name}: {r:?}");
        assert_eq!(r.underruns_per_min, 0.0, "{name}: {r:?}");
    }
}

#[test]
fn follows_a_game_that_cant_keep_up() {
    // 85% of real time for a minute: the audio slows to match, breaking up
    // at most once or twice as it does.
    for name in ["85% for a minute", "phone, 85%"] {
        let r = after(&scenario(name));
        assert!((r.mean_step / r.game_speed - 1.0).abs() < 0.01, "{name}: {r:?}");
        assert!(r.underruns_per_min < 1.5, "{name}: {r:?}");
    }
    let r = after(&scenario("varying 80-100%"));
    assert!(r.underruns_per_min < 3.0, "{r:?}");
}
