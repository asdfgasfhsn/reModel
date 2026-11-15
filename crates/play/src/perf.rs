//! `--perf-log`: what the game's thread was doing when it ran slow. Each
//! redraw whose emulation took longer than its frames' share of real time
//! prints a `slow` line with what the JITs did in it (blocks installed and
//! the time installing them, which is on this thread; code run on the
//! interpreter because no translation was ready; translations dropped or
//! thrown away; jobs waiting on the workers), so a slowdown can be told
//! apart as the JIT's or not. Leaving the game prints a `frame times`
//! summary like the CLI's `run` and `bench`.
//!
//! It also times the redraws themselves ([`spans`]): each one that comes
//! more than one and a half refreshes after the one before prints a `gap`
//! line saying where the time went (the surface's wait for a texture, the
//! present, drawing, the event loop, emulation, the geometrizer), and
//! leaving the game a `redraw gaps` summary, so a stutter with a fast
//! emulation can be told apart as the display's, the draw thread's or the
//! event loop's.

use std::time::{Duration, Instant};

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
    gaps: Gaps,
}

impl PerfLog {
    pub fn new(set: &'static str, m: &model2::Machine) -> Self {
        spans::start();
        let gaps = Gaps { geo_waited: m.bus.geo.waited, ..Gaps::default() };
        PerfLog { set, last: Snapshot::of(m), frames: Vec::new(), slow: 0, gaps }
    }

    /// One redraw: `ran` frames emulated in `emulate`, `queued` the audio
    /// waiting to play (seconds) if there is sound.
    pub fn redraw(&mut self, m: &model2::Machine, ran: u32, emulate: Duration, queued: Option<f64>) {
        // The geometrizer's waits in it (for its worker, from the frames'
        // ends): the geometrizer's time, not emulation's.
        self.gaps.geo_emulating += m.bus.geo.waited.saturating_sub(self.gaps.geo_waited);
        self.gaps.geo_waited = m.bus.geo.waited;
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

    /// A redraw was handed to drawing (`refresh`: the display's period,
    /// or the game's frame period if not known; `inline`: drawn on this
    /// thread, `--no-overlap`): a `gap` line if it came late.
    pub fn submitted(&mut self, m: &model2::Machine, refresh: Duration, inline: bool) {
        let now = Instant::now();
        let spans = spans::take();
        let g = &mut self.gaps;
        let geo = std::mem::take(&mut g.geo_emulating);
        let Some(prev) = g.last.replace(now) else { return };
        g.redraws += 1;
        let gap = now - prev;
        if gap.as_secs_f64() <= 1.5 * refresh.as_secs_f64() {
            return;
        }
        // Each span is logged as it ends, so all of those that overlap the
        // gap are here (the redraw before's start, logged with the gap
        // before, is taken to be at its start).
        let all = spans;
        let t = attribute(&all, prev, now, inline, geo);
        g.count += 1;
        g.worst = g.worst.max(gap);
        for (sum, v) in g.time.iter_mut().zip(t) {
            *sum += v;
        }
        let cause = cause(&t);
        g.by_cause[cause] += 1;
        for s in &all {
            match s.kind {
                Kind::RedrawStart => g.qos_main = s.qos,
                // Drawn inline, the surface's wait is on the game's thread.
                Kind::Surface if !inline => g.qos_draw = s.qos,
                _ => {}
            }
        }
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let parts: Vec<String> =
            CAUSES.iter().zip(t).filter(|(_, v)| ms(*v) >= 0.05).map(|(c, v)| format!("{c} {:.1}", ms(v))).collect();
        println!(
            "gap           {} frame {}: redraw {:.1} ms after the last ({:.1} refreshes); ms: {}; mostly {}",
            self.set,
            m.frames,
            ms(gap),
            gap.as_secs_f64() / refresh.as_secs_f64(),
            parts.join(", "),
            CAUSES[cause],
        );
    }

    /// Redraws that aren't the game's (photo mode): the next one handed
    /// over starts timing again.
    pub fn pause(&mut self) {
        self.gaps.last = None;
        spans::take();
    }

    /// The `redraw gaps` line: how many came late, and what took the time.
    pub fn gaps_summary(&self) -> String {
        let g = &self.gaps;
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let mut out =
            format!("redraw gaps   {}: {} of {} redraws came over 1.5 refreshes late", self.set, g.count, g.redraws);
        if g.count > 0 {
            let causes: Vec<String> =
                CAUSES.iter().zip(g.by_cause).filter(|(_, n)| *n > 0).map(|(c, n)| format!("{c} {n}")).collect();
            let time: Vec<String> = CAUSES
                .iter()
                .zip(g.time)
                .map(|(c, v)| (c, ms(v) / g.count as f64))
                .filter(|(_, v)| *v >= 0.05)
                .map(|(c, v)| format!("{c} {v:.1}"))
                .collect();
            out += &format!(
                ", worst {:.1} ms; mostly: {}; ms in each: {}",
                ms(g.worst),
                causes.join(", "),
                time.join(", ")
            );
        }
        if let Some(main) = qos_name(g.qos_main) {
            out += &format!("; QoS: game's thread {main}");
            if let Some(draw) = qos_name(g.qos_draw) {
                out += &format!(", draw thread {draw}");
            }
        }
        out
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

impl Drop for PerfLog {
    fn drop(&mut self) {
        spans::stop();
    }
}

/// Where a gap's time went, in [`CAUSES`]' order.
const CAUSES: [&str; 7] = ["surface", "present", "drawing", "event loop", "emulation", "geometrizer", "other"];
const SURFACE: usize = 0;
const PRESENT: usize = 1;
const DRAWING: usize = 2;
const EVENT_LOOP: usize = 3;
const EMULATION: usize = 4;
const GEOMETRIZER: usize = 5;
const OTHER: usize = 6;

/// The redraw gaps so far.
#[derive(Default)]
struct Gaps {
    /// When the last redraw was handed to drawing.
    last: Option<Instant>,
    /// The geometrizer's `waited` when last seen, and its waits since in
    /// emulation (until the next hand-over).
    geo_waited: Duration,
    geo_emulating: Duration,
    redraws: u64,
    count: u64,
    worst: Duration,
    /// Time in each cause, and the gaps each was most of.
    time: [Duration; 7],
    by_cause: [u64; 7],
    /// The QoS classes the threads ran at (macOS).
    qos_main: u32,
    qos_draw: u32,
}

use spans::Kind;

/// Split the time from `from` to `to` (between two hand-overs) by what
/// the game's thread was doing (`spans`): emulating (less the
/// geometrizer's waits in it, `geo`), waiting for the geometrizer's
/// frame, waiting for the draw thread (the frame before still drawing:
/// the surface's wait for a texture, the present or the drawing), idle in
/// the event loop until it delivered the redraw, or other work in it.
/// Drawn `inline`, the surface's and present's waits are the game
/// thread's own.
fn attribute(spans: &[spans::Span], from: Instant, to: Instant, inline: bool, geo: Duration) -> [Duration; 7] {
    // The redraws, and the waits between them that were for the draw
    // thread (the redraw before deferred itself: `next_start`).
    let mut events: Vec<&spans::Span> = spans.iter().collect();
    events.sort_by_key(|s| s.a);
    let (mut busy, mut deferred) = (Vec::new(), Vec::new());
    let (mut open, mut defer, mut idle_from): (Option<Instant>, bool, Option<(Instant, bool)>) = (None, false, None);
    for s in &events {
        match s.kind {
            Kind::RedrawStart => {
                if let Some((at, true)) = idle_from.take() {
                    deferred.push((at, s.a));
                }
                open = Some(s.a);
                defer = false;
            }
            Kind::Defer => defer = true,
            Kind::RedrawEnd => {
                busy.push((open.take().unwrap_or(from), s.a));
                idle_from = Some((s.a, defer));
            }
            _ => {}
        }
    }
    if let Some(at) = open {
        busy.push((at, to));
    }
    if let Some((at, true)) = idle_from {
        deferred.push((at, to));
    }
    let of =
        |k: Kind| -> Vec<(Instant, Instant)> { spans.iter().filter(|s| s.kind == k).map(|s| (s.a, s.b)).collect() };
    let (emulate, geometry, finish, surface, present) =
        (of(Kind::Emulate), of(Kind::Geometry), of(Kind::Finish), of(Kind::Surface), of(Kind::Present));
    let mut cuts: Vec<Instant> = vec![from, to];
    for &(a, b) in
        busy.iter().chain(&deferred).chain(&emulate).chain(&geometry).chain(&finish).chain(&surface).chain(&present)
    {
        cuts.extend([a, b].into_iter().filter(|t| (from..=to).contains(t)));
    }
    cuts.sort();
    cuts.dedup();
    let within = |l: &[(Instant, Instant)], t: Instant| l.iter().any(|&(a, b)| a <= t && t < b);
    let mut out = [Duration::ZERO; 7];
    for w in cuts.windows(2) {
        let (a, d) = (w[0], w[1] - w[0]);
        let on_draw = |out: &mut [Duration; 7]| {
            let i = if within(&surface, a) {
                SURFACE
            } else if within(&present, a) {
                PRESENT
            } else {
                DRAWING
            };
            out[i] += d;
        };
        if within(&emulate, a) {
            out[EMULATION] += d;
        } else if within(&geometry, a) {
            out[GEOMETRIZER] += d;
        } else if within(&finish, a) || within(&deferred, a) {
            on_draw(&mut out);
        } else if !within(&busy, a) {
            out[EVENT_LOOP] += d;
        } else if inline && (within(&surface, a) || within(&present, a)) {
            on_draw(&mut out);
        } else {
            out[OTHER] += d;
        }
    }
    let geo = geo.min(out[EMULATION]);
    out[EMULATION] -= geo;
    out[GEOMETRIZER] += geo;
    out
}

/// The cause that took most of a gap.
fn cause(t: &[Duration; 7]) -> usize {
    (0..t.len()).max_by_key(|&i| t[i]).unwrap_or(OTHER)
}

/// A macOS quality-of-service class's name (`qos_class_t`).
fn qos_name(q: u32) -> Option<&'static str> {
    Some(match q {
        0x21 => "user-interactive",
        0x19 => "user-initiated",
        0x15 => "default",
        0x11 => "utility",
        0x09 => "background",
        _ => return None,
    })
}

/// The redraws' timing, from the game's thread and the draw thread: off
/// (a check of a flag) unless `--perf-log` has a game running.
pub mod spans {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    use std::time::Instant;

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Kind {
        /// A redraw's handler, on the game's thread.
        RedrawStart,
        RedrawEnd,
        /// The redraw waits for the draw thread instead (`next_start`).
        Defer,
        /// Emulating the frames due.
        Emulate,
        /// Waiting for the geometrizer's frame, to draw it.
        Geometry,
        /// Waiting for the draw thread to finish the frame before.
        Finish,
        /// On the thread drawing: the surface's wait for a texture, and
        /// the present.
        Surface,
        Present,
    }

    #[derive(Clone, Copy)]
    pub struct Span {
        pub kind: Kind,
        pub a: Instant,
        pub b: Instant,
        /// The thread's QoS class (macOS; 0 elsewhere).
        pub qos: u32,
    }

    static ON: AtomicBool = AtomicBool::new(false);
    static LOG: Mutex<Vec<Span>> = Mutex::new(Vec::new());
    /// More than this many spans without a hand-over to take them (the
    /// launcher, a pause): start again.
    const MAX: usize = 4096;

    pub(super) fn start() {
        LOG.lock().unwrap().clear();
        ON.store(true, Ordering::Relaxed);
    }

    pub(super) fn stop() {
        ON.store(false, Ordering::Relaxed);
        LOG.lock().unwrap().clear();
    }

    pub(super) fn take() -> Vec<Span> {
        std::mem::take(&mut *LOG.lock().unwrap())
    }

    /// The start of a span, when timing.
    pub fn begin() -> Option<Instant> {
        ON.load(Ordering::Relaxed).then(Instant::now)
    }

    /// The span from `begin` until now.
    pub fn end(kind: Kind, begin: Option<Instant>) {
        let Some(a) = begin else { return };
        let span = Span { kind, a, b: Instant::now(), qos: qos() };
        let mut log = LOG.lock().unwrap();
        if log.len() >= MAX {
            log.clear();
        }
        log.push(span);
    }

    /// An instant.
    pub fn mark(kind: Kind) {
        end(kind, begin());
    }

    #[cfg(target_os = "macos")]
    fn qos() -> u32 {
        extern "C" {
            fn qos_class_self() -> u32;
        }
        // SAFETY: reads the calling thread's class; no arguments.
        unsafe { qos_class_self() }
    }

    #[cfg(not(target_os = "macos"))]
    fn qos() -> u32 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spans::Span;

    fn span(kind: Kind, t0: Instant, a: u64, b: u64) -> Span {
        let ms = |m| t0 + Duration::from_millis(m);
        Span { kind, a: ms(a), b: ms(b), qos: 0 }
    }

    #[test]
    fn a_gap_is_split_by_what_the_game_thread_waited_on() {
        let t0 = Instant::now();
        let at = |m| t0 + Duration::from_millis(m);
        // The redraw before ends at 1 ms after its hand-over at 0; the next
        // defers itself at 2 (the draw thread is waiting for the surface,
        // 0 to 15, then draws to 18); the one after starts at 18, emulates
        // for 3 ms, waits 1 for the geometrizer and hands over at 23.
        let spans = [
            span(Kind::RedrawEnd, t0, 1, 1),
            span(Kind::Surface, t0, 0, 15),
            span(Kind::RedrawStart, t0, 2, 2),
            span(Kind::Defer, t0, 2, 2),
            span(Kind::RedrawEnd, t0, 2, 2),
            span(Kind::RedrawStart, t0, 18, 18),
            span(Kind::Emulate, t0, 18, 21),
            span(Kind::Geometry, t0, 21, 22),
        ];
        let t = attribute(&spans, at(0), at(23), false, Duration::ZERO);
        let ms = |i: usize| t[i].as_millis();
        assert_eq!(ms(SURFACE), 13); // 2 to 15
        assert_eq!(ms(DRAWING), 3); // 15 to 18
        assert_eq!(ms(EVENT_LOOP), 1); // 1 to 2
        assert_eq!(ms(EMULATION), 3);
        assert_eq!(ms(GEOMETRIZER), 1);
        assert_eq!(ms(OTHER), 2); // 0 to 1, 22 to 23
        assert_eq!(cause(&t), SURFACE);
        // The geometrizer's waits inside emulation are its own.
        let t = attribute(&spans, at(0), at(23), false, Duration::from_millis(2));
        assert_eq!((t[EMULATION].as_millis(), t[GEOMETRIZER].as_millis()), (1, 3));
    }
}
