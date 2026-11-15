//! What recompiled programs (`crate::codegen`) run on: the interpreter's
//! own cycle, with the instruction known when the code is compiled.
//!
//! A recompiled program's `step` is a `match` on [`Sharc::compiled_pc`]
//! whose arms call [`Sharc::run_compiled`] with that word's decoding as a
//! constant. `run_compiled` is the body of [`Sharc::step`] for one executed
//! instruction, and everything it calls is `#[inline(always)]`, so the
//! compiler specialises each arm: decode, operand and register dispatch
//! fold away, as do the fetch stage's branch and loop set-up paths for
//! instructions that can't take them. The state changes are the
//! interpreter's, in the same order, so a step of recompiled code and a
//! step of the interpreter are indistinguishable (`make lockstep` checks).

use super::*;
use state::State as _;

impl Sharc {
    /// The address of the instruction the next [`step`](Sharc::step)
    /// executes, for recompiled code to dispatch on, after running the
    /// bubbles (squashed words) before it as `step` would. `u32::MAX` when
    /// recompiled code can't run it: in IDLE, or a compiled word the
    /// program has overwritten (see [`watch_code`](Sharc::watch_code)).
    #[inline(always)]
    pub fn compiled_pc(&mut self) -> u32 {
        if self.idle {
            return u32::MAX;
        }
        if self.exec_slot.is_none() {
            self.bubbles();
        }
        match self.exec_slot {
            Some((a, _)) if !self.code_dirty || self.overwritten(a) == Some(false) => a,
            _ => u32::MAX,
        }
    }

    /// Whether the compiled word at `a` was overwritten (None: not one).
    fn overwritten(&self, a: u32) -> Option<bool> {
        match self.code.get(word_index(a)?)? {
            1 => Some(false),
            2 => Some(true),
            _ => None,
        }
    }

    /// The cycles of [`step`](Sharc::step) that execute nothing.
    #[inline(never)]
    fn bubbles(&mut self) {
        while self.exec_slot.is_none() {
            self.begin_cycle();
            self.fetch_stage(false, false);
        }
    }

    /// One [`step`](Sharc::step) of the instruction `insn`, which must be
    /// the word at `insn.addr` and in the execute slot (`compiled_pc`
    /// returned its address). `index` is its [`word_index`].
    #[inline(always)]
    pub fn run_compiled(&mut self, insn: &Insn, index: usize, ext: &mut impl Bus) -> Step {
        self.begin_cycle();
        self.exec_slot = None;
        self.pc = insn.addr;
        self.counts[index] += 1;
        if let Err(f) = self.exec(insn, ext) {
            return Step::Fault(f);
        }
        self.steps += 1;
        let redirect =
            matches!(insn.op, Op::Branch { .. } | Op::JumpElseMove { .. } | Op::Return { .. } | Op::CJump { .. });
        self.fetch_stage(redirect, matches!(insn.op, Op::Do { .. }));
        self.end_insn()
    }

    // --- blocks ----------------------------------------------------------------
    //
    // A recompiled program runs runs of straight-line words as blocks: the
    // instructions one after another, with the pipeline, loop logic,
    // cycle and step counters, timer and interrupt latches brought up to
    // date only when the block ends. That is exact only where those can't
    // do anything but count, which `block_start` checks: the pipeline is
    // the plain sequence (each word fetched two cycles before it runs), no
    // fetch in the block reaches the end of the innermost loop, interrupts
    // are off (so none can be taken) and so is the timer, nothing has
    // overwritten compiled code, and the bus allows it (`Bus::block_budget`).
    // The words themselves (codegen decides) change none of the loop, PC
    // or status stacks or sequencer or system registers, and an
    // instruction that would branch, or reach memory other than internal
    // RAM and the external port (IOP registers, reserved addresses), ends
    // the block before it runs and then runs as a single step.

    /// Whether the next instructions, from `at` (in the execute slot)
    /// up to `end` (exclusive), can run as a block, and how far:
    /// `budget` is the bus's `block_budget`. None: run `at` as a single
    /// step.
    #[inline(always)]
    pub fn block_start(&self, at: u32, end: u32, budget: u64) -> Option<Block> {
        if self.code_dirty
            || self.booting
            || self.mode1_pending.is_some()
            || self.mode1 & IRPTEN != 0
            || self.mode2 & TIMEN != 0
        {
            return None;
        }
        let (next, flag) = self.decode_slot?;
        if next != at.wrapping_add(1) || self.faddr != at.wrapping_add(2) {
            return None;
        }
        let mut n = end.saturating_sub(at).min(budget.min(u32::MAX as u64) as u32);
        if let Some(&(laddr, _)) = self.loop_stack.last() {
            let e = laddr_end(laddr);
            // `spinning` could hold inside a short loop ending on a
            // condition.
            if !laddr_counter(laddr) && self.pc_stack.last().is_some_and(|&top| e.wrapping_sub(top) < 4) {
                return None;
            }
            // Instruction k fetches at + k + 2: stop before the one that
            // fetches the loop's end.
            if let Some(d) = e.checked_sub(at.wrapping_add(2)) {
                n = n.min(d);
            }
        }
        if n == 0 {
            return None;
        }
        Some(Block { at, stop: at + n, steps: self.steps, flag })
    }

    /// One instruction of a block: `insn` is the word at `insn.addr`,
    /// the next the block runs. `st` gets STKY as it was before it ran
    /// (the timer and latches tick on that), `steps` is the step count
    /// once it has.
    #[inline(always)]
    pub fn block_exec(&mut self, insn: &Insn, st: &mut u32, steps: u64, ext: &mut impl Bus) -> InBlock {
        let ext_access = match self.block_access(insn) {
            None => return InBlock::Bail,
            Some(e) => e,
        };
        *st = self.stky;
        if let Err(f) = self.exec(insn, ext) {
            return InBlock::Fault(f);
        }
        if ext_access {
            self.steps = steps;
            ext.after_access(self);
        }
        if self.code_dirty {
            // It wrote compiled code: what follows may have changed.
            return InBlock::Last;
        }
        InBlock::Ran
    }

    /// What `insn` will do in a block: None if it must run as a single
    /// step (it branches, or reaches IOP registers or reserved memory),
    /// else whether it uses the external port. As `exec` would decide,
    /// before it runs.
    #[inline(always)]
    fn block_access(&self, insn: &Insn) -> Option<bool> {
        // Internal RAM (normal and short-word addresses) or the external
        // port; else IOP registers and reserved addresses.
        let class = |a: u32| match a {
            0x2_0000..=0x7_ffff => Some(false),
            0x8_0000.. => Some(true),
            _ => None,
        };
        match insn.op {
            Op::Dual { dm, pm, .. } => {
                let (d, p) = (class(self.dag_peek(&dm.addr))?, class(self.dag_peek(&pm.addr))?);
                Some(d || p)
            }
            Op::Transfer { cond, mv, .. } | Op::ShiftImm { cond, mv: Some(mv), .. } => {
                if self.cond(cond) {
                    class(self.dag_peek(&mv.addr))
                } else {
                    Some(false)
                }
            }
            Op::JumpElseMove { cond, mv, .. } => {
                if self.cond(cond) {
                    None
                } else {
                    class(self.dag_peek(&mv.addr))
                }
            }
            Op::Branch { cond, .. } | Op::Return { cond, .. } => (!self.cond(cond)).then_some(false),
            Op::Direct { addr, .. } => class(addr),
            Op::IndirectImm { i, offset, .. } => {
                class(self.dag_peek(&Addr { i, modifier: Modifier::Imm(offset), post: false }))
            }
            Op::StoreImm { addr, .. } => class(self.dag_peek(&addr)),
            _ => Some(false),
        }
    }

    /// The end of a block: bring the state up to date with the
    /// instructions it ran, as single steps would have left it. `st` is
    /// STKY before the last one ran. None if it ran none (run `b.at` as a
    /// single step).
    #[inline(always)]
    pub fn block_end(&mut self, b: &Block, end: BlockEnd, st: u32) -> Option<Step> {
        let (ran, fault) = match end {
            BlockEnd::Before(a) => (a - b.at, None),
            BlockEnd::Last(a) => (a + 1 - b.at, None),
            BlockEnd::Fault(a, f) => (a - b.at, Some(f)),
        };
        if ran == 0 && fault.is_none() {
            return None;
        }
        self.block_done(b, ran, st, fault)
    }

    #[inline(never)]
    fn block_done(&mut self, b: &Block, ran: u32, st: u32, fault: Option<Fault>) -> Option<Step> {
        let first = word_index(b.at).expect("compiled words are internal");
        if ran > 0 {
            let last = b.at + ran - 1;
            self.cycles += ran as u64;
            self.steps = b.steps + ran as u64;
            self.pc = last;
            self.exec_slot = Some((last + 1, ran == 1 && b.flag));
            self.decode_slot = Some((last + 2, false));
            self.faddr = last + 3;
            self.irq_hold = self.irq_hold.saturating_sub(ran.min(u8::MAX as u32) as u8);
            for c in &mut self.counts[first..first + ran as usize] {
                *c += 1;
            }
        }
        // Every cycle's tick latched the IRQ lines and the exceptions in
        // STKY, which only gains bits in a block: the last tick's is all.
        self.latch_ticks(st);
        let Some(f) = fault else { return Some(Step::Ran) };
        // As `run_compiled` leaves a faulting instruction: its cycle begun,
        // the execute slot taken.
        self.cycles += 1;
        self.exec_slot = None;
        self.pc = b.at + ran;
        self.counts[first + ran as usize] += 1;
        Some(Step::Fault(f))
    }

    /// A [`tick`](Sharc::tick) (in a block, so with the timer off) with
    /// STKY at `st`.
    fn latch_ticks(&mut self, st: u32) {
        let stky = std::mem::replace(&mut self.stky, st);
        self.tick();
        self.stky = stky;
    }

    /// A [`step`](Sharc::step) on the interpreter, for an instruction the
    /// recompiled program has no code for (counted in `misses` unless the
    /// core is idle).
    #[inline(never)]
    pub fn step_uncompiled(&mut self, ext: &mut impl Bus) -> Step {
        if let (false, Some((a, _))) = (self.idle, self.exec_slot) {
            *self.misses.entry(a).or_default() += 1;
        }
        self.step(ext)
    }

    /// Recompiled code for the words at `addrs` of the program `image`
    /// (48-bit words from 0x20000) is in use: a word that doesn't hold
    /// what was compiled, or is written later, runs on the interpreter.
    /// An empty list detaches; so does loading a save state.
    pub fn watch_code(&mut self, addrs: &[u32], image: &[u64]) {
        self.code.clear();
        self.code_dirty = false;
        if addrs.is_empty() {
            return;
        }
        self.code = vec![0; self.decoded.len()];
        for &a in addrs {
            let Some(k) = word_index(a) else { continue };
            let compiled = a.checked_sub(0x2_0000).and_then(|o| image.get(o as usize));
            if compiled == Some(&self.mem.read48(a)) {
                self.code[k] = 1;
            } else {
                self.code[k] = 2;
                self.code_dirty = true;
            }
            self.cached[k] = 1;
        }
    }

    /// Watch the words at `addrs` too, as `watch_code` does (for a JIT,
    /// which translates a program a block at a time): a write to one of
    /// them from now on marks it overwritten, and `block_start` declines
    /// until [`take_overwritten`](Sharc::take_overwritten) has been
    /// called. Words that aren't internal 48-bit words are ignored.
    pub fn watch_words(&mut self, addrs: impl IntoIterator<Item = u32>) {
        if self.code.is_empty() {
            self.code = vec![0; self.decoded.len()];
        }
        for a in addrs {
            if let Some(k) = word_index(a) {
                if self.code[k] == 0 {
                    self.code[k] = 1;
                }
                self.cached[k] = 1;
            }
        }
    }

    /// The watched words written since they were watched, which are no
    /// longer watched (a JIT drops their translations and watches them
    /// again when it translates them again).
    pub fn take_overwritten(&mut self) -> Vec<u32> {
        let mut v = Vec::new();
        if std::mem::take(&mut self.code_dirty) {
            let words = mem::WORDS48 as usize;
            for (k, c) in self.code.iter_mut().enumerate() {
                if *c == 2 {
                    *c = 0;
                    let base = if k < words { mem::BLOCK0 } else { mem::BLOCK1 };
                    v.push(base + (k % words) as u32);
                }
            }
        }
        v
    }

    /// Whether any words are watched (`watch_code`, `watch_words`); a
    /// state load or `watch_code(&[], &[])` stops watching all of them.
    pub fn code_watched(&self) -> bool {
        !self.code.is_empty()
    }

    /// Some watched word has been written.
    #[inline(always)]
    pub fn code_dirty(&self) -> bool {
        self.code_dirty
    }

    /// Compiled words the program has overwritten since `watch_code`.
    pub fn overwritten_code(&self) -> Vec<u32> {
        (0x2_0000..0x4_0000).filter(|&a| self.overwritten(a) == Some(true)).collect()
    }

    /// A hash of everything but memory and the diagnostics: registers,
    /// stacks, pipeline, IOP registers, counters. For comparing a
    /// recompiled run with the interpreter step by step.
    pub fn digest(&self) -> u64 {
        let mut w = state::Writer::default();
        self.r.save(&mut w);
        self.r_alt.save(&mut w);
        for v in [&self.i, &self.m, &self.l, &self.b] {
            v.save(&mut w);
        }
        self.dag_alt.save(&mut w);
        for mr in self.mr {
            (mr as u64, (mr >> 64) as u64).save(&mut w);
        }
        self.px.save(&mut w);
        [self.mode1, self.mode2, self.astat, self.stky, self.irptl, self.imask, self.imaskp, self.lcntr, self.pc]
            .save(&mut w);
        [self.ustat1, self.ustat2, self.tperiod, self.tcount, self.faddr].save(&mut w);
        self.mode1_pending.save(&mut w);
        self.pc_stack.save(&mut w);
        self.loop_stack.save(&mut w);
        self.status_stack.save(&mut w);
        self.exec_slot.save(&mut w);
        self.decode_slot.save(&mut w);
        (self.reentry, self.last_loopback).save(&mut w);
        (self.irq_hold, self.idle).save(&mut w);
        self.flag_in.save(&mut w);
        (self.cycles, self.steps).save(&mut w);
        let bytes = w.buf;
        // FNV-1a.
        bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
    }
}

/// A block of instructions recompiled code runs at once
/// ([`Sharc::block_start`]).
#[derive(Clone, Copy, Debug)]
pub struct Block {
    /// The first instruction's address.
    pub at: u32,
    /// The address it stops before, at the latest.
    pub stop: u32,
    /// `steps` before it.
    pub steps: u64,
    /// The looped-back flag of the decode slot it started with.
    flag: bool,
}

/// What [`Sharc::block_exec`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InBlock {
    /// Ran the instruction; the block can go on.
    Ran,
    /// Ran it, and the block ends after it.
    Last,
    /// Didn't run it: the block ends before it.
    Bail,
    Fault(Fault),
}

/// How a block ended, by instruction address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockEnd {
    /// Before the instruction at this address (not run).
    Before(u32),
    /// After this one.
    Last(u32),
    /// This one faulted.
    Fault(u32, Fault),
}

/// A hash of the words a recompiled program has code for: (address, word)
/// for each of `addrs` in `image` (48-bit words from 0x20000). Programs
/// are looked up by their compiled words being in memory, not by the
/// whole image, so two uploads with the same code and different data
/// share one; this names the crate.
pub fn code_hash(image: &[u64], addrs: &[u32]) -> u64 {
    addrs.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &a| {
        let w = a.checked_sub(0x2_0000).and_then(|k| image.get(k as usize)).copied().unwrap_or(0);
        (0..10).fold(h, |h, k| {
            let byte = if k < 4 { a >> (8 * k) & 0xff } else { (w >> (8 * (k - 4)) & 0xff) as u32 };
            (h ^ byte as u64).wrapping_mul(0x100_0000_01b3)
        })
    })
}

/// A hash of a SHARC program image (48-bit words from 0x20000).
pub fn program_hash(words: &[u64]) -> u64 {
    words.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &w| {
        (0..6).fold(h, |h, k| (h ^ (w >> (8 * k) & 0xff)).wrapping_mul(0x100_0000_01b3))
    })
}

/// The program in internal memory: the 48-bit words from 0x20000 up to the
/// last nonzero one before 0x22800, as an upload leaves them (and as
/// `remodel run` writes them to `sharc-*.bin`).
pub fn program_image(s: &Sharc) -> Vec<u64> {
    let mut words: Vec<u64> = (0..0x2800).map(|k| s.mem.read48(0x2_0000 + k)).collect();
    let used = words.iter().rposition(|&w| w != 0).map_or(0, |k| k + 1);
    words.truncate(used);
    words
}

/// Offsets of the core's fields, for a JIT's translated code (crates/jit),
/// which reads and writes them in place. Private fields of the
/// interpreter: anything else should use the methods.
#[doc(hidden)]
pub mod layout {
    use super::Sharc;
    use std::mem::offset_of;
    pub const R: usize = offset_of!(Sharc, r);
    pub const I: usize = offset_of!(Sharc, i);
    pub const M: usize = offset_of!(Sharc, m);
    pub const L: usize = offset_of!(Sharc, l);
    pub const B: usize = offset_of!(Sharc, b);
    pub const MODE1: usize = offset_of!(Sharc, mode1);
    pub const ASTAT: usize = offset_of!(Sharc, astat);
    pub const STKY: usize = offset_of!(Sharc, stky);
    pub const IRPTL: usize = offset_of!(Sharc, irptl);
    pub const IMASKP: usize = offset_of!(Sharc, imaskp);
    pub const CODE_DIRTY: usize = offset_of!(Sharc, code_dirty);
}
