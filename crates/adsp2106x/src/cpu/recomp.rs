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
        }
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

/// A hash of a SHARC program image (48-bit words from 0x20000): what
/// recompiled programs are looked up by.
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
