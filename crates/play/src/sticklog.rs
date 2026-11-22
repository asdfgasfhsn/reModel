//! `--stick-log FILE`: what a pad's sticks read, a CSV line a game frame, and
//! a summary of how noisy each stick is at rest, for measuring a real pad
//! (docs/play.md "Measuring a pad").

use crate::controls::Tuning;
use romset::PadAxis;
use std::io::Write;

/// The stick axes, with the names the log gives them.
const AXES: [(PadAxis, &str); 4] = [
    (PadAxis::LeftStickX, "lx"),
    (PadAxis::LeftStickY, "ly"),
    (PadAxis::RightStickX, "rx"),
    (PadAxis::RightStickY, "ry"),
];

/// A raw reading under this is the stick at rest.
const REST: f32 = 0.15;

/// What one axis read near rest.
#[derive(Clone, Copy, Debug, Default)]
struct Rest {
    n: u64,
    sum: f64,
    sum_sq: f64,
    /// The largest |raw| seen at rest.
    max: f32,
    /// The smoothed readings at those frames (steering axes).
    smoothed_sum_sq: f64,
}

impl Rest {
    fn add(&mut self, raw: f32, smoothed: Option<f32>) {
        if raw.abs() >= REST {
            return;
        }
        self.n += 1;
        self.sum += raw as f64;
        self.sum_sq += (raw as f64) * (raw as f64);
        self.max = self.max.max(raw.abs());
        if let Some(s) = smoothed {
            self.smoothed_sum_sq += (s as f64) * (s as f64);
        }
    }

    fn mean(&self) -> f64 {
        self.sum / self.n.max(1) as f64
    }

    fn sd(&self) -> f64 {
        let m = self.mean();
        (self.sum_sq / self.n.max(1) as f64 - m * m).max(0.0).sqrt()
    }
}

pub struct StickLog {
    out: std::io::BufWriter<std::fs::File>,
    rest: [Rest; 4],
    /// Which axes steer in this game.
    steering: [bool; 4],
    frames: u64,
    failed: bool,
}

impl StickLog {
    pub fn create(path: &std::path::Path) -> std::io::Result<StickLog> {
        let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
        let mut head = String::from("frame");
        for (_, n) in AXES {
            head += &format!(",{n}_raw,{n}_tuned,{n}_smoothed");
        }
        writeln!(out, "{head}")?;
        Ok(StickLog { out, rest: Default::default(), steering: [false; 4], frames: 0, failed: false })
    }

    /// A game starts: which of the sticks steer in it.
    pub fn game(&mut self, steering: impl Fn(PadAxis) -> bool) {
        self.steering = AXES.map(|(a, _)| steering(a));
    }

    /// One frame: `raw` the pad's readings (`PadAxis::ALL` order), `smoothed`
    /// what steering reads ([`crate::steer::SteerFilter::step`]'s result). The tuned value
    /// is past the steering tuning on an axis that steers, else the stick's;
    /// the smoothed one is left empty on an axis that doesn't steer.
    pub fn frame(&mut self, frame: u64, raw: &[f32; 8], smoothed: &[f32; 8], tuning: Tuning) {
        let mut line = format!("{frame}");
        for (k, (a, _)) in AXES.iter().enumerate() {
            let i = PadAxis::ALL.iter().position(|x| x == a).unwrap_or(0);
            let steers = self.steering[k];
            let tuned = tuning.for_axis(*a, steers).apply(raw[i]);
            let sm = steers.then_some(smoothed[i]);
            line += &format!(",{:.4},{:.4},", raw[i], tuned);
            if let Some(s) = sm {
                line += &format!("{s:.4}");
            }
            self.rest[k].add(raw[i], sm);
        }
        self.frames += 1;
        if writeln!(self.out, "{line}").is_err() && !self.failed {
            self.failed = true;
            eprintln!("--stick-log: can't write");
        }
    }

    /// The summary: per axis near rest, the mean and standard deviation of
    /// the raw reading and the largest offset from zero.
    pub fn summary(&mut self) -> String {
        let _ = self.out.flush();
        let mut s = format!("stick log     {} frames; at rest (raw under {:.0}%):\n", self.frames, REST * 100.0);
        if self.frames == 0 {
            s += "  no gamepad was read (connect one and press a button; keys aren't logged)\n";
            return s;
        }
        for (k, (_, n)) in AXES.iter().enumerate() {
            let r = &self.rest[k];
            if r.n == 0 {
                s += &format!("  {n}  no frames at rest\n");
                continue;
            }
            s += &format!(
                "  {n}  {} frames, mean {:+.2}%, sd {:.2}%, largest offset {:.2}%",
                r.n,
                r.mean() * 100.0,
                r.sd() * 100.0,
                r.max * 100.0
            );
            if self.steering[k] {
                s += &format!(" (steering: smoothed rms {:.2}%)", (r.smoothed_sum_sq / r.n as f64).sqrt() * 100.0);
            }
            s += "\n";
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rest_statistics_ignore_deflected_frames() {
        // Scratch is out/scratch, not /tmp (RAM-backed here).
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/scratch");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("stick-log-test-{}.csv", std::process::id()));
        let mut log = StickLog::create(&path).unwrap();
        log.game(|a| a == PadAxis::LeftStickX);
        let t = Tuning::default();
        for (f, v) in [0.02f32, -0.02, 0.02, -0.02, 0.9, 0.8].into_iter().enumerate() {
            let mut raw = [0.0; 8];
            raw[1] = v;
            log.frame(f as u64, &raw, &raw, t);
        }
        let r = log.rest[0];
        assert_eq!(r.n, 4);
        assert!(r.mean().abs() < 1e-6);
        assert!((r.sd() - 0.02).abs() < 1e-6);
        assert!((r.max - 0.02).abs() < 1e-6);
        let text = log.summary();
        assert!(text.contains("lx  4 frames") && text.contains("steering"), "{text}");
        let csv = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(csv.starts_with("frame,lx_raw,lx_tuned,lx_smoothed,ly_raw"));
        assert_eq!(csv.lines().count(), 7);
        // Non-steering axes have an empty smoothed column.
        assert!(csv.lines().nth(1).unwrap().contains(",,"));
    }
}
