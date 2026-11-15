//! Idle-loop skipping (docs/idle-skip.md).
//!
//! The games spend most of each frame in a poll loop, waiting for vblank or
//! for an interrupt handler to set a flag. Where a game's manifest lists
//! such a loop (`romset::Game::idle_loops`), the machine runs one pass of
//! it normally while the bus watches what it touches, and if the pass is a
//! fixed point it credits as many more passes as fit before anything the
//! loop reads could change, leaving the machine exactly as running them
//! would:
//!
//! - the pass wrote nothing and every read was of memory only the i960
//!   writes, or of a register that changes only at an event (vblank, a
//!   timer, the sound UART's interrupt); reads of buffer RAM or the
//!   copro's output status count only while the copro is idle
//!   (`Model2Bus::probe_read`);
//! - no event happened during the pass, and none happens during the passes
//!   credited: they end before the next one (`Model2Bus::quiet_retires`);
//! - the CPU came back to the loop's start in the state it left it in, but
//!   for counters: registers the loop only ever adds a constant to (checked
//!   from its code, [`counters`]), which get that constant once per pass;
//! - the copro (2C's MB86235, 2B's SHARC) runs on through the credited
//!   passes in one batch, which steps it exactly as one run per
//!   instruction would (`Model2Bus::idle_advance`).
//!
//! Only whole passes are credited; the rest of the way to the event is
//! run normally, so a partial pass lands where stepping puts it. Nothing
//! here is machine state: save states, run-ahead's copies and the state
//! hash are the same with skipping on or off.

use crate::Machine;
use i960::cpu::Stop;
use i960::{MemRef, Operand};
use std::sync::atomic::{AtomicBool, Ordering};

static DEFAULT_ON: AtomicBool = AtomicBool::new(true);

/// Whether machines built from now on skip their game's idle loops
/// (default on; the front ends' `--no-idle-skip` turns it off).
pub fn set_default(on: bool) {
    DEFAULT_ON.store(on, Ordering::Relaxed);
}

/// See [`set_default`].
pub fn default_on() -> bool {
    DEFAULT_ON.load(Ordering::Relaxed)
}

/// Most instructions one pass of a listed loop may take before the probe
/// gives up on it.
const MAX_PASS: u64 = 64;
/// After this many probes in a row that found no fixed point, wait
/// [`BACKOFF`] instructions before probing that loop again.
const MAX_FAILS: u32 = 2;
const BACKOFF: u64 = 4096;

/// What skipping has done so far (diagnostics, not machine state).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Instructions credited without running them.
    pub skipped: u64,
    /// Times passes were credited.
    pub skips: u64,
    /// Passes run to check for a fixed point, and those that found none.
    pub probes: u64,
    pub refused: u64,
}

/// A machine's idle loops and its skipping's bookkeeping.
pub(crate) struct IdleSkip {
    /// The loops as `[start, end)`.
    loops: Vec<(u32, u32)>,
    /// Skipping is on (it may still have no loops).
    on: bool,
    /// Lowest and highest loop start, or (`u32::MAX`, 0) when off: a cheap
    /// test on every instruction.
    lo: u32,
    hi: u32,
    /// Don't probe before this instruction count (after failed probes).
    retry_at: u64,
    fails: u32,
    /// The instruction addresses of the last probe pass (interpreted), for
    /// the hot-spot profile.
    ips: Vec<u32>,
    pub stats: Stats,
}

impl IdleSkip {
    pub fn new(game: &romset::Game) -> Self {
        let mut s = IdleSkip {
            loops: Vec::new(),
            on: default_on(),
            lo: u32::MAX,
            hi: 0,
            retry_at: 0,
            fails: 0,
            ips: Vec::new(),
            stats: Stats::default(),
        };
        s.set_loops(game.idle_loops);
        s
    }

    /// Where the loops start (where the machine looks for one between
    /// blocks: JIT blocks stop there).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn starts(&self) -> Vec<u32> {
        self.loops.iter().map(|l| l.0).collect()
    }

    pub fn set_loops(&mut self, loops: &[romset::IdleLoop]) {
        self.loops = loops.iter().map(|l| (l.start, l.end)).collect();
        self.set(self.on);
    }

    pub fn set(&mut self, on: bool) {
        self.on = on;
        (self.lo, self.hi) = match self.loops.iter().map(|l| l.0).min() {
            Some(lo) if on => (lo, self.loops.iter().map(|l| l.0).max().unwrap_or(lo)),
            _ => (u32::MAX, 0),
        };
    }

    /// The loop starting at `ip`, if it's worth probing now.
    #[inline(always)]
    fn at(&self, ip: u32, icount: u64) -> Option<(u32, u32)> {
        if ip < self.lo || ip > self.hi || icount < self.retry_at {
            return None;
        }
        self.loops.iter().copied().find(|l| l.0 == ip)
    }

    fn failed(&mut self, icount: u64) {
        self.stats.refused += 1;
        self.fails += 1;
        if self.fails >= MAX_FAILS {
            self.fails = 0;
            self.retry_at = icount + BACKOFF;
        }
    }
}

/// The registers (bit n: register n) that the loop's code in `[start,
/// end)` only counts in: every instruction naming one is `addo` or `subo`
/// of a literal into itself, and no instruction in the loop reads or
/// writes a register it doesn't name (no multi-register moves, loads or
/// stores, no calls, no floating point). `None` if the code isn't all
/// such instructions.
pub fn counters(start: u32, end: u32, fetch: impl Fn(u32) -> Option<u32>) -> Option<u32> {
    const PLAIN: &[&str] = &[
        "ld", "ldob", "ldos", "ldib", "ldis", "lda", "mov", "and", "andnot", "notand", "xor", "or", "nor", "xnor",
        "not", "ornot", "notor", "nand", "setbit", "clrbit", "notbit", "alterbit", "addo", "addi", "subo", "subi",
        "shro", "shri", "shlo", "shli", "rotate", "cmpo", "cmpi", "concmpo", "concmpi", "chkbit", "teste", "testne",
        "testl", "testle", "testg", "testge", "testo", "testno", "b", "be", "bne", "bl", "ble", "bg", "bge", "bo",
        "bno", "bbs", "bbc", "cmpobe", "cmpobne", "cmpobl", "cmpoble", "cmpobg", "cmpobge", "cmpibe", "cmpibne",
        "cmpibl", "cmpible", "cmpibg", "cmpibge", "cmpibo", "cmpibno",
    ];
    let (mut counting, mut other, mut a) = (0u32, 0u32, start);
    while a < end {
        let insn = i960::decode(a, &fetch);
        if !PLAIN.contains(&insn.mnemonic) {
            return None;
        }
        let ops = insn.operands.as_slice();
        let own = match (insn.mnemonic, ops) {
            ("addo" | "subo", [Operand::Reg(s), Operand::Lit(_), Operand::Reg(d)])
            | ("addo" | "subo", [Operand::Lit(_), Operand::Reg(s), Operand::Reg(d)])
                if s == d =>
            {
                Some(*d)
            }
            _ => None,
        };
        match own {
            Some(r) => counting |= 1 << r,
            None => {
                for op in ops {
                    other |= match *op {
                        Operand::Reg(r) => 1 << r,
                        Operand::Mem(m) => match m {
                            MemRef::Base { abase, .. } => 1 << abase,
                            MemRef::Index { index, .. } => 1 << index,
                            MemRef::BaseIndex { abase, index, .. } => 1 << abase | 1 << index,
                            MemRef::Abs(_) | MemRef::IpRel { .. } => 0,
                        },
                        _ => 0,
                    };
                }
            }
        }
        a = insn.next();
    }
    (a == end).then_some(counting & !other)
}

impl Machine {
    /// Skip a game's idle loops (on by default, from [`set_default`]).
    pub fn set_idle_skip(&mut self, on: bool) {
        self.idle.set(on);
    }

    pub fn idle_skip_on(&self) -> bool {
        self.idle.on
    }

    /// Skip these loops instead of the manifest's (tests, experiments).
    pub fn set_idle_loops(&mut self, loops: &[romset::IdleLoop]) {
        self.idle.set_loops(loops);
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(j) = self.jit.as_deref_mut() {
            j.set_stops(self.idle.starts());
        }
    }

    /// What idle skipping has done in this machine.
    pub fn idle_stats(&self) -> Stats {
        self.idle.stats
    }

    /// At a listed loop's start: probe a pass and credit what follows.
    /// False if nothing ran (run an instruction as usual); `left` is how
    /// many instructions the caller still allows.
    #[inline(always)]
    pub(crate) fn idle_skip(&mut self, left: u64) -> Result<bool, Stop> {
        match self.idle.at(self.cpu.ip, self.cpu.icount) {
            Some(l) => self.idle_pass(l, left),
            None => Ok(false),
        }
    }

    #[inline(never)]
    fn idle_pass(&mut self, (start, end): (u32, u32), left: u64) -> Result<bool, Stop> {
        // Not worth a probe unless two more passes could follow it (taking
        // the loop's words for a pass's length).
        let words = ((end - start) / 4).max(1) as u64;
        let quiet = self.bus.quiet_retires();
        if self.cpu.interrupt_due() || quiet < 3 * words || left < 3 * words {
            return Ok(false);
        }
        self.idle.stats.probes += 1;
        let before = self.cpu.arch_state();
        let (i0, f0) = (self.cpu.icount, self.frames);
        let copro_idle = self.bus.copro_idle();
        let limit = left.min(MAX_PASS);
        self.idle.ips.clear();
        self.bus.probe_begin();
        let ran = self.probe_pass(start, end, limit);
        let probe = self.bus.probe_end();
        ran?;
        let p = self.cpu.icount - i0;
        let fixed = p > 0
            && p <= quiet
            && self.cpu.ip == start
            && self.frames == f0
            && !probe.impure
            && (copro_idle || !probe.needs_copro_idle)
            && !self.cpu.interrupt_due();
        let deltas = fixed.then(|| self.cpu.arch_state().changes(&before)).flatten();
        let Some(deltas) = deltas.filter(|d| {
            d.is_empty() || {
                let bus = &self.bus;
                counters(start, end, |a| i960::cpu::Bus::fetch(bus, a))
                    .is_some_and(|c| d.iter().all(|&(r, _)| c & (1 << r) != 0))
            }
        }) else {
            self.idle.failed(self.cpu.icount);
            self.bus.probe_return(probe);
            return Ok(true);
        };
        self.idle.fails = 0;
        let k = (quiet - p).min(left - p) / p;
        if k > 0 {
            let n = k * p;
            self.cpu.icount += n;
            for (r, d) in deltas {
                self.cpu.r[r] = self.cpu.r[r].wrapping_add(d.wrapping_mul(k as u32));
            }
            self.bus.idle_advance(n, k, &probe.io_reads);
            for &ip in &self.idle.ips {
                self.frame_profile.add(ip, k);
            }
            self.idle.stats.skipped += n;
            self.idle.stats.skips += 1;
        }
        self.bus.probe_return(probe);
        Ok(true)
    }

    /// Run one pass of the loop `[start, end)` from its start as usual:
    /// until it is back at the start, leaves the loop, ends a frame, has an
    /// interrupt to take or has run `limit` instructions.
    fn probe_pass(&mut self, start: u32, end: u32, limit: u64) -> Result<(), Stop> {
        let (i0, f0) = (self.cpu.icount, self.frames);
        loop {
            let done = self.cpu.icount - i0;
            let ip = self.cpu.ip;
            if done > 0 && (ip <= start || ip >= end || self.frames != f0) || done >= limit || self.cpu.interrupt_due()
            {
                return Ok(());
            }
            if self.native.is_some() {
                self.dispatch(limit - done)?;
            } else {
                self.idle.ips.push(ip);
                self.step()?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use romset::{IdleLoop, RomSet};

    /// Where the test programs run: work RAM, as `srallyc`'s code does.
    const AT: u32 = 0x50_0100;

    /// A `game` machine (no ROM contents) running `code` from [`AT`], with
    /// `idle` as its idle loops and skipping `on` or off. Vblank and the
    /// timers post interrupts (vectors 0x28 and 0x20) below the CPU's
    /// priority (31 from reset), so they stay pending and change its state
    /// without being taken.
    fn machine(game: &str, code: &[u32], idle: &[IdleLoop], on: bool) -> Machine {
        let game = romset::games::find(game).unwrap();
        let roms = RomSet { game, program: vec![0; 0x1000], chips: vec![Vec::new(); game.files.len()] };
        let mut m = Machine::new(&roms);
        m.bus.geo.set_threaded(false);
        for (i, &w) in code.iter().enumerate() {
            i960::cpu::Bus::write32(&mut m.bus, AT + 4 * i as u32, w);
        }
        m.cpu.ip = AT;
        m.cpu.icr = 0x0020_0028;
        assert_eq!(m.cpu.priority(), 31);
        i960::cpu::Bus::write32(&mut m.bus, 0xe8_0004, 0x3d);
        m.set_idle_loops(idle);
        m.set_idle_skip(on);
        m
    }

    /// Poll the vblank toggle (bit 2 of 0x98000C) until it flips, counting
    /// passes in g0, then go round again: the shape of `lastbrnx`'s and
    /// `srallyc`'s loops.
    const VBLANK: [u32; 9] = [
        0x90a8_3000,
        0x0098_000c, // ld 0x98000c, g5
        0x58a9_1095, // and g5, 4, g5
        0x90a0_3000,
        0x0098_000c, // loop: ld 0x98000c, g4
        0x5980_5010, // addo g0, 1, g0
        0x58a1_1094, // and g4, 4, g4
        0x3aad_1ff0, // cmpibe g5, g4, loop
        0x08ff_ffe0, // b back to the start
    ];
    const VBLANK_LOOP: IdleLoop = IdleLoop { start: AT + 12, end: AT + 32 };

    /// Wait for a byte in work RAM to change (`vf2`'s loop): it never does.
    const BYTE: [u32; 3] = [
        0x8018_3000,
        0x0050_0000, // loop: ldob 0x500000, r3
        0x3a1c_1ff8, // cmpibe r3, g0, loop
    ];
    const BYTE_LOOP: IdleLoop = IdleLoop { start: AT, end: AT + 12 };

    /// A loop that stores: never a fixed point.
    const STORE: [u32; 3] = [
        0x9280_3000,
        0x0050_0200, // loop: st g0, 0x500200
        0x08ff_fff8, // b loop
    ];
    const STORE_LOOP: IdleLoop = IdleLoop { start: AT, end: AT + 12 };

    fn decoded(code: &[u32]) -> Vec<String> {
        let fetch = |a: u32| code.get(((a - AT) / 4) as usize).copied();
        let (mut a, mut v) = (AT, Vec::new());
        while ((a - AT) / 4) < code.len() as u32 {
            let i = i960::decode(a, fetch);
            a = i.next();
            v.push(i.to_string());
        }
        v
    }

    #[test]
    fn test_programs_are_what_they_say() {
        assert_eq!(
            decoded(&VBLANK),
            [
                "ld 0x98000c, g5",
                "and g5, 4, g5",
                "ld 0x98000c, g4",
                "addo g0, 1, g0",
                "and g4, 4, g4",
                "cmpibe g5, g4, 0x50010c",
                "b 0x500100"
            ]
        );
        assert_eq!(decoded(&BYTE), ["ldob 0x500000, r3", "cmpibe r3, g0, 0x500100"]);
        assert_eq!(decoded(&STORE), ["st g0, 0x500200", "b 0x500100"]);
    }

    /// Run `with` (skipping) and `without` in the same slices; they must
    /// agree after every one, whole state included.
    fn same_runs(with: &mut Machine, without: &mut Machine, slices: &[u64]) {
        for (i, &n) in slices.iter().enumerate() {
            let (a, b) = (with.run_slice(n).unwrap(), without.run_slice(n).unwrap());
            assert_eq!(a, b, "slice {i} ({n}) ran a different number of instructions");
            assert_eq!(with.bus.now, without.bus.now, "slice {i}");
            assert!(with.save_state() == without.save_state(), "slice {i} ({n}): states differ");
        }
        assert_eq!(without.idle_stats(), Stats::default());
    }

    /// Slices of every size, from single instructions to whole frames,
    /// so they end in the middle of passes, of skips and at frame ends.
    fn slices() -> Vec<u64> {
        let mut v = Vec::new();
        for f in 0..6 {
            v.extend([1, 2, 3, 4, 5, 7, 11, 100, 1009, 4096 + f, 30_011, u64::MAX]);
        }
        v
    }

    #[test]
    fn fast_forward_lands_where_stepping_does() {
        for game in ["vf2", "topskatr", "overrevb"] {
            for (code, l) in [(&VBLANK[..], VBLANK_LOOP), (&BYTE[..], BYTE_LOOP)] {
                let mut a = machine(game, code, &[l], true);
                let mut b = machine(game, code, &[l], false);
                // Timers 0 and 1 running out mid-frame, and again.
                for m in [&mut a, &mut b] {
                    i960::cpu::Bus::write32(&mut m.bus, 0xf0_0000, 20_000);
                    i960::cpu::Bus::write32(&mut m.bus, 0xf0_0004, 300_001);
                }
                same_runs(&mut a, &mut b, &slices());
                // Most of the time is in the loop, and most of that skipped.
                let skipped = a.idle_stats().skipped;
                assert!(skipped > a.cpu.icount / 2, "{game}: skipped {skipped} of {}", a.cpu.icount);
            }
        }
    }

    /// The counter in g0 gets its passes: after a frame of polling it holds
    /// what stepping counted.
    #[test]
    fn counters_are_credited() {
        let mut a = machine("vf2", &VBLANK, &[VBLANK_LOOP], true);
        let mut b = machine("vf2", &VBLANK, &[VBLANK_LOOP], false);
        same_runs(&mut a, &mut b, &[u64::MAX, u64::MAX, 777, u64::MAX]);
        assert!(a.cpu.r[16] > 100_000, "g0 counted {} passes", a.cpu.r[16]);
        assert!(a.idle_stats().skips > 0);
    }

    #[test]
    fn loops_that_write_are_not_skipped() {
        let mut a = machine("vf2", &STORE, &[STORE_LOOP], true);
        let mut b = machine("vf2", &STORE, &[STORE_LOOP], false);
        same_runs(&mut a, &mut b, &[1000, u64::MAX, u64::MAX]);
        assert_eq!(a.idle_stats().skipped, 0);
        assert!(a.idle_stats().refused > 0);
    }

    /// Off, or with no loops, nothing is probed.
    #[test]
    fn off_switch() {
        let mut a = machine("vf2", &BYTE, &[BYTE_LOOP], false);
        a.run_slice(u64::MAX).unwrap();
        assert_eq!(a.idle_stats(), Stats::default());
        let mut a = machine("vf2", &BYTE, &[], true);
        a.run_slice(u64::MAX).unwrap();
        assert_eq!(a.idle_stats(), Stats::default());
    }

    #[test]
    fn counters_only_count() {
        let fetch = |code: &'static [u32]| move |a: u32| code.get(((a - AT) / 4) as usize).copied();
        // g0 only counts; g4 and g5 are read.
        assert_eq!(counters(AT + 12, AT + 32, fetch(&VBLANK)), Some(1 << 16));
        // Read by `mov g0, g1`: not a counter.
        const READ: [u32; 3] = [0x5980_5010, 0x5c88_0610, 0x08ff_fff8];
        assert_eq!(decoded(&READ)[1], "mov g0, g1");
        assert_eq!(counters(AT, AT + 12, fetch(&READ)), Some(0));
        // A store isn't plain code.
        assert_eq!(counters(AT, AT + 12, fetch(&STORE)), None);
    }
}
