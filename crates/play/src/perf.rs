//! `--perf-log`: what the game's thread was doing when it ran slow. Each
//! redraw whose emulation took longer than its frames' share of real time
//! prints a `slow` line with what the JITs did in it (blocks installed and
//! the time installing them, which is on this thread; code run on the
//! interpreter because no translation was ready; translations dropped or
//! thrown away; jobs waiting on the workers), so a slowdown can be told
//! apart as the JIT's or not. Leaving the game prints a `frame times`
//! summary like the CLI's `run` and `bench`.

use std::time::Duration;

/// The JITs' counters at one point, to take differences of.
#[derive(Clone, Copy, Default)]
struct Snapshot {
    /// i960: blocks installed, host time installing them (this thread),
    /// instructions interpreted outside blocks, blocks dropped because
    /// their code changed, finished translations thrown away.
    blocks: u64,
    link: Duration,
    interpreted: u64,
    dropped: u64,
    discarded: u64,
    /// The copro JITs, summed: blocks, words interpreted outside blocks,
    /// translations thrown away.
    copro_blocks: u64,
    copro_interpreted: u64,
    copro_discarded: u64,
}

impl Snapshot {
    fn of(m: &model2::Machine) -> Self {
        let mut s = Snapshot::default();
        if let Some(j) = m.jit_stats() {
            (s.blocks, s.link, s.interpreted, s.dropped, s.discarded) =
                (j.blocks, j.link, j.interpreted, j.dropped, j.discarded);
        }
        for (_, c, _) in m.copro_jit_stats() {
            s.copro_blocks += c.blocks;
            s.copro_interpreted += c.interpreted;
            s.copro_discarded += c.discarded;
        }
        s
    }
}

pub struct PerfLog {
    /// The game's set, at the start of every line (games started from the
    /// launcher one after another share the terminal).
    set: &'static str,
    last: Snapshot,
    /// Host milliseconds of emulation per frame, a redraw's time shared
    /// among its frames.
    frames: Vec<f32>,
    slow: u32,
}

impl PerfLog {
    pub fn new(set: &'static str, m: &model2::Machine) -> Self {
        PerfLog { set, last: Snapshot::of(m), frames: Vec::new(), slow: 0 }
    }

    /// One redraw: `ran` frames emulated in `emulate`, `queued` the audio
    /// waiting to play (seconds) if there is sound.
    pub fn redraw(&mut self, m: &model2::Machine, ran: u32, emulate: Duration, queued: Option<f64>) {
        if ran == 0 {
            return;
        }
        let now = Snapshot::of(m);
        let ms = emulate.as_secs_f64() * 1000.0;
        let per = ms / ran as f64;
        self.frames.extend(std::iter::repeat_n(per as f32, ran as usize));
        let budget = 1000.0 / model2::pacing::FPS;
        if per > budget {
            self.slow += 1;
            let d = |a: u64, b: u64| a.saturating_sub(b);
            let (l, n) = (self.last, now);
            let jit = if m.jit_on() {
                format!(
                    "; i960 JIT: +{} blocks ({:.2} ms installing), {} instructions interpreted, {} dropped, {} thrown away; \
                     copro JITs: +{} blocks, {} words interpreted, {} thrown away; {} jobs on the workers",
                    d(n.blocks, l.blocks),
                    (n.link.saturating_sub(l.link)).as_secs_f64() * 1000.0,
                    d(n.interpreted, l.interpreted),
                    d(n.dropped, l.dropped),
                    d(n.discarded, l.discarded),
                    d(n.copro_blocks, l.copro_blocks),
                    d(n.copro_interpreted, l.copro_interpreted),
                    d(n.copro_discarded, l.copro_discarded),
                    model2::jit::busy(),
                )
            } else {
                "; no JIT".to_string()
            };
            let audio = queued.map_or(String::new(), |q| format!("; audio {:.0} ms queued", q * 1000.0));
            println!(
                "slow          {} frame {}: {ran} frames in {ms:.1} ms ({per:.1} ms each, {budget:.1} due){jit}{audio}",
                self.set, m.frames
            );
        }
        self.last = now;
    }

    /// The `frame times` line: host milliseconds of emulation per frame.
    pub fn summary(&self) -> String {
        let mut v = self.frames.clone();
        if v.is_empty() {
            return format!("frame times   {}: no frames", self.set);
        }
        v.sort_by(f32::total_cmp);
        let at = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
        let over = |ms: f32| v.iter().filter(|&&x| x > ms).count();
        format!(
            "frame times   {}: {} frames emulated, ms: median {:.2}, p99 {:.2}, p99.9 {:.2}, worst {:.1}; over 8 ms {}, over 16.7 ms {}; {} slow redraws",
            self.set,
            v.len(),
            at(0.5),
            at(0.99),
            at(0.999),
            v[v.len() - 1],
            over(8.0),
            over(16.7),
            self.slow
        )
    }
}
