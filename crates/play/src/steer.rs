//! Steering smoothing: an adaptive low-pass on a pad stick that steers.
//!
//! A pad stick's reading wanders by a couple of percent even held still, and
//! a game that points its wheels straight at the wheel position every frame
//! (Daytona) turns that into car twitch. The filter is the One Euro filter
//! (Casiez, Roussel and Vogel, "1 Euro Filter: A Simple Speed-based
//! Low-pass Filter for Noisy Input in Interactive Systems", CHI 2012),
//! written from the paper's description: a first-order low-pass whose
//! cutoff rises with the (itself smoothed) speed of the signal, so a still
//! stick is smoothed heavily and a flick is hardly delayed.
//!
//! One step is one emulated frame with a fixed time step, never wall time,
//! so the same inputs always give the same outputs.

use crate::controls::Tuning;
use romset::PadAxis;

/// Seconds a step stands for: one game frame.
const DT: f32 = 1.0 / 60.0;

/// How much steering is smoothed (the Controls page's "Steering smoothing").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Smoothing {
    Off,
    #[default]
    Light,
    Medium,
    Strong,
}

/// The filter's parameters (the paper's names), for the level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// Cutoff in Hz while the stick is still: lower smooths more.
    pub min_cutoff: f32,
    /// How much the cutoff rises with speed (Hz per unit a second): higher
    /// lags less on a fast move.
    pub beta: f32,
    /// Cutoff in Hz of the speed estimate.
    pub d_cutoff: f32,
}

impl Smoothing {
    pub const ALL: [Smoothing; 4] = [Smoothing::Off, Smoothing::Light, Smoothing::Medium, Smoothing::Strong];

    pub fn name(self) -> &'static str {
        match self {
            Smoothing::Off => "off",
            Smoothing::Light => "light",
            Smoothing::Medium => "medium",
            Smoothing::Strong => "strong",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Smoothing::Off => "Off",
            Smoothing::Light => "Light",
            Smoothing::Medium => "Medium",
            Smoothing::Strong => "Strong",
        }
    }

    pub fn parse(s: &str) -> Option<Smoothing> {
        Smoothing::ALL.into_iter().find(|c| c.name() == s)
    }

    /// The parameters, chosen from the synthetic signal in the tests below
    /// (`levels_on_a_synthetic_stick`; numbers in docs/play.md): the jitter
    /// left while holding and the frames of lag on a flick at each level.
    pub fn params(self) -> Option<Params> {
        let p = |min_cutoff, beta| Some(Params { min_cutoff, beta, d_cutoff: 1.0 });
        match self {
            Smoothing::Off => None,
            Smoothing::Light => p(1.5, 2.0),
            Smoothing::Medium => p(1.0, 1.0),
            Smoothing::Strong => p(0.5, 0.5),
        }
    }
}

/// The smoothing factor of a low-pass with cutoff `fc` Hz at one step.
fn alpha(fc: f32) -> f32 {
    let tau = 1.0 / (2.0 * std::f32::consts::PI * fc);
    1.0 / (1.0 + tau / DT)
}

/// One signal's filter.
#[derive(Clone, Copy, Debug, Default)]
pub struct OneEuro {
    /// The last smoothed value and speed; None before the first step.
    last: Option<(f32, f32)>,
}

impl OneEuro {
    pub fn reset(&mut self) {
        self.last = None;
    }

    /// The smoothed `x` after one more frame. The first reading after a
    /// reset passes through.
    pub fn step(&mut self, x: f32, p: Params) -> f32 {
        let Some((prev, dx_prev)) = self.last else {
            self.last = Some((x, 0.0));
            return x;
        };
        let dx = dx_prev + alpha(p.d_cutoff) * ((x - prev) / DT - dx_prev);
        let fc = p.min_cutoff + p.beta * dx.abs();
        let out = prev + alpha(fc) * (x - prev);
        self.last = Some((out, dx));
        out
    }
}

/// The filters of one player's stick axes (the four sticks', in
/// `PadAxis::ALL` order of the stick ones).
#[derive(Clone, Copy, Debug, Default)]
pub struct SteerFilter {
    axes: [OneEuro; 4],
}

/// The stick axes a filter holds.
const STICKS: [PadAxis; 4] = [PadAxis::LeftStickX, PadAxis::LeftStickY, PadAxis::RightStickX, PadAxis::RightStickY];

impl SteerFilter {
    pub fn reset(&mut self) {
        *self = SteerFilter::default();
    }

    /// One frame: each stick axis of `raw` (a pad's readings in
    /// `PadAxis::ALL` order) through the steering tuning, then the
    /// smoothing. `None` for a pad that isn't there lets go of the state.
    /// Returns the smoothed readings in `PadAxis::ALL` order (non-stick
    /// slots 0).
    pub fn step(&mut self, raw: Option<&[f32; 8]>, tuning: Tuning) -> [f32; 8] {
        let mut out = [0.0; 8];
        let Some(raw) = raw else {
            self.reset();
            return out;
        };
        let params = tuning.smooth.params();
        for (i, a) in PadAxis::ALL.into_iter().enumerate() {
            let Some(k) = STICKS.iter().position(|s| *s == a) else { continue };
            let tuned = tuning.steer.apply(raw[i]);
            out[i] = match params {
                Some(p) => self.axes[k].step(tuned, p),
                None => {
                    self.axes[k].reset();
                    tuned
                }
            };
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic noise in -1 to 1.
    struct Noise(u32);
    impl Noise {
        fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(1103515245).wrapping_add(12345) & 0x7fff_ffff;
            self.0 as f32 / 0x7fff_ffff as f32 * 2.0 - 1.0
        }
    }

    const NOISE: f32 = 0.03;

    fn sd(v: &[f32]) -> f32 {
        let m = v.iter().sum::<f32>() / v.len() as f32;
        (v.iter().map(|x| (x - m) * (x - m)).sum::<f32>() / v.len() as f32).sqrt()
    }

    fn first_at(v: &[f32], level: f32) -> usize {
        v.iter().position(|x| *x >= level).unwrap()
    }

    /// What one level does to the three synthetic signals: the jitter left
    /// holding (standard deviation), the frames from the start of a flick to
    /// 90% (`raw` for the unfiltered stick), and the mean and spread of the
    /// error on a slow sweep.
    struct Result {
        hold: f32,
        flick: usize,
        sweep_bias: f32,
        sweep_sd: f32,
    }

    fn run(level: Smoothing) -> Result {
        let go = |sig: &[f32]| -> Vec<f32> {
            let (mut f, mut out) = (OneEuro::default(), Vec::new());
            for x in sig {
                out.push(match level.params() {
                    Some(p) => f.step(*x, p),
                    None => *x,
                });
            }
            out
        };
        let mut n = Noise(12345);
        // Holding at 0.5 for 10 s with 3% noise (the first 2 s settle).
        let hold: Vec<f32> = (0..600).map(|_| 0.5 + NOISE * n.next()).collect();
        let hold = sd(&go(&hold)[120..]);
        // A flick: still at 0 for a second, then to full in 4 frames.
        let mut n = Noise(12345);
        let flick: Vec<f32> = (0..240)
            .map(|i| {
                let t = i - 60;
                let base = if t < 0 { 0.0 } else { ((t + 1) as f32 / 4.0).min(1.0) };
                (base + NOISE * n.next()).clamp(-1.0, 1.0)
            })
            .collect();
        let flick = first_at(&go(&flick)[60..], 0.9);
        // A slow sweep from 0 to full over 5 s.
        let mut n = Noise(12345);
        let sweep: Vec<f32> = (0..300).map(|i| i as f32 / 299.0 + NOISE * n.next()).collect();
        let err: Vec<f32> = go(&sweep).iter().enumerate().map(|(i, o)| o - i as f32 / 299.0).skip(30).collect();
        Result { hold, flick, sweep_bias: err.iter().sum::<f32>() / err.len() as f32, sweep_sd: sd(&err) }
    }

    /// The table in docs/play.md and why the levels are what they are. Run
    /// with `--nocapture` to print it.
    #[test]
    fn levels_on_a_synthetic_stick() {
        let off = run(Smoothing::Off);
        let mut table = String::from(
            "level   min_cutoff  beta  d_cutoff  hold_sd  jitter_cut  flick_frames  extra_lag  sweep_bias  sweep_sd\n",
        );
        for l in Smoothing::ALL {
            let r = run(l);
            let (mc, b, dc) = l.params().map_or((0.0, 0.0, 0.0), |p| (p.min_cutoff, p.beta, p.d_cutoff));
            table += &format!(
                "{:<7} {:>10.2} {:>5.2} {:>9.2} {:>8.4} {:>10.0}% {:>13} {:>10} {:>11.4} {:>9.4}\n",
                l.label(),
                mc,
                b,
                dc,
                r.hold,
                (1.0 - r.hold / off.hold) * 100.0,
                r.flick,
                r.flick - off.flick,
                r.sweep_bias,
                r.sweep_sd
            );
        }
        println!("{table}");
        if let Ok(path) = std::env::var("STEER_TABLE") {
            std::fs::write(path, &table).unwrap();
        }
        // Light: at least half the jitter gone, at most a frame of lag.
        let light = run(Smoothing::Light);
        assert!(light.hold < off.hold * 0.5, "{} vs {}", light.hold, off.hold);
        assert!(light.flick - off.flick <= 1);
        // Each level smooths more and lags no less than the one before.
        let all: Vec<Result> = Smoothing::ALL.into_iter().map(run).collect();
        for w in all.windows(2) {
            assert!(w[1].hold < w[0].hold);
            assert!(w[1].flick >= w[0].flick);
        }
        // Medium and Strong still follow a flick within a few frames.
        assert!(all[3].flick - off.flick <= 3);
    }

    #[test]
    fn steady_input_passes_through() {
        for l in Smoothing::ALL {
            let Some(p) = l.params() else { continue };
            let mut f = OneEuro::default();
            let out = (0..300).map(|_| f.step(0.6, p)).last().unwrap();
            assert!((out - 0.6).abs() < 1e-5, "{l:?}: {out}");
            // The first reading after a reset is itself.
            f.reset();
            assert_eq!(f.step(-0.4, p), -0.4);
        }
    }

    #[test]
    fn noise_is_reduced_and_a_step_follows() {
        let p = Smoothing::Light.params().unwrap();
        let mut n = Noise(7);
        let mut f = OneEuro::default();
        let raw: Vec<f32> = (0..400).map(|_| 0.4 + 0.025 * n.next()).collect();
        let out: Vec<f32> = raw.iter().map(|x| f.step(*x, p)).collect();
        assert!(sd(&out[100..]) < sd(&raw[100..]) * 0.5);
        // A jump from 0 to full in one frame is at 90% within 3.
        let mut f = OneEuro::default();
        (0..60).for_each(|_| {
            f.step(0.0, p);
        });
        let frames = (1..).find(|_| f.step(1.0, p) >= 0.9).unwrap();
        assert!(frames <= 3, "{frames}");
    }

    #[test]
    fn same_inputs_same_outputs() {
        let run = || {
            let mut n = Noise(99);
            let mut f = SteerFilter::default();
            let t = Tuning::default();
            (0..120)
                .map(|_| {
                    let mut raw = [0.0; 8];
                    raw[1] = 0.5 + 0.03 * n.next();
                    f.step(Some(&raw), t)[1].to_bits()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn off_is_the_tuned_value_and_no_pad_resets() {
        let mut t = Tuning { smooth: Smoothing::Off, ..Tuning::default() };
        let mut f = SteerFilter::default();
        let mut raw = [0.0; 8];
        raw[1] = 0.7;
        assert_eq!(f.step(Some(&raw), t)[1], t.steer.apply(0.7));
        t.smooth = Smoothing::Strong;
        f.step(Some(&raw), t);
        f.step(None, t);
        // After the reset the first reading passes through, however strong.
        raw[1] = -0.9;
        assert_eq!(f.step(Some(&raw), t)[1], t.steer.apply(-0.9));
    }
}
