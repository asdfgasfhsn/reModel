//! What a JIT's pipeline blocks run on (crates/jit, docs/jit.md "The
//! SHARC"): runs of instructions, branches and loops included, executed
//! with the pipeline modelled by the translated code and the interpreter's
//! state brought up to date when the run ends.
//!
//! A pipeline block is exact where `step` would do nothing per cycle but
//! count, fetch and move the two pipeline slots on: interrupts off (so
//! none can be taken), the timer off (so a tick only latches IRQ lines and
//! STKY's exceptions, which only gain bits in a block: the last cycle's
//! tick is all of them), no MODE1 write pending, nothing watched
//! overwritten, and not in a loop `spinning` could hold inside. The
//! translated code runs each cycle as `step` does (`begin_cycle`, the
//! instruction, `fetch_stage`, `end_insn`), and leaves the sequencer's
//! stacks and the loop logic to the methods here, so that only what it
//! keeps in registers (the pipeline, the cycle and step counts, the
//! interrupt hold-off) is written back at the end ([`Sharc::jit_exit`]).

use super::*;

/// The pipeline as a pipeline block keeps it (C layout: the translated
/// code reads and writes it in place).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JitPipe {
    /// The execute slot (`x_some` 0: empty), the decode slot, and the
    /// fetch address. A slot's flag is set when its fetch looped back.
    pub x: u32,
    pub x_flag: u32,
    pub x_some: u32,
    pub d: u32,
    pub d_flag: u32,
    pub d_some: u32,
    pub f: u32,
    /// Instructions to finish before an interrupt can be taken.
    pub hold: u32,
    /// The innermost loop's last word (`u32::MAX`: no loop).
    pub loop_end: u32,
    /// The innermost loop ends on a condition within four words, where
    /// `spinning` could hold: a block mustn't run on into it.
    pub short_loop: u32,
}

impl Sharc {
    /// The pipeline, if a pipeline block can start now (see the module
    /// docs); `compiled_pc` has run any bubbles.
    #[doc(hidden)]
    pub fn jit_enter(&self) -> Option<JitPipe> {
        if self.code_dirty
            || self.booting
            || self.idle
            || self.mode1_pending.is_some()
            || self.mode1 & IRPTEN != 0
            || self.mode2 & TIMEN != 0
        {
            return None;
        }
        let (x, x_flag) = self.exec_slot?;
        let (d_some, (d, d_flag)) = match self.decode_slot {
            Some(s) => (1, s),
            None => (0, (0, false)),
        };
        let short_loop = self.jit_short_loop();
        if short_loop {
            return None;
        }
        Some(JitPipe {
            x,
            x_flag: x_flag as u32,
            x_some: 1,
            d,
            d_flag: d_flag as u32,
            d_some,
            f: self.faddr,
            hold: self.irq_hold as u32,
            loop_end: self.jit_loop_end(),
            short_loop: 0,
        })
    }

    /// The innermost loop's last word, or `u32::MAX`.
    fn jit_loop_end(&self) -> u32 {
        self.loop_stack.last().map_or(u32::MAX, |&(l, _)| laddr_end(l))
    }

    /// The innermost loop ends on a condition and is under four words
    /// long (as `block_start` declines).
    fn jit_short_loop(&self) -> bool {
        let Some(&(laddr, _)) = self.loop_stack.last() else { return false };
        !laddr_counter(laddr) && self.pc_stack.last().is_some_and(|&top| laddr_end(laddr).wrapping_sub(top) < 4)
    }

    /// The end of a pipeline block that ran `cycles` cycles (`ran` of them
    /// instructions, the last at `pc`) to `steps`: the pipeline as `p`
    /// leaves it, and each cycle's tick latched with `st`, STKY at the
    /// start of the last one.
    #[doc(hidden)]
    pub fn jit_exit(&mut self, p: &JitPipe, pc: u32, cycles: u64, steps: u64, st: u32) {
        if cycles == 0 {
            return;
        }
        self.exec_slot = (p.x_some != 0).then_some((p.x, p.x_flag != 0));
        self.decode_slot = (p.d_some != 0).then_some((p.d, p.d_flag != 0));
        self.faddr = p.f;
        self.irq_hold = p.hold as u8;
        self.pc = pc;
        self.cycles += cycles;
        self.steps = steps;
        let stky = std::mem::replace(&mut self.stky, st);
        self.tick();
        self.stky = stky;
    }

    /// A fetch stage's loop logic for the fetch of `fetched`, the
    /// innermost loop's end (`next_after`), the cycle having begun with
    /// ASTAT `astat` (as stored) and FLAG inputs `flag_in`: the next fetch
    /// address, bit 32 set if the loop went back; and the pipe's loop end
    /// and short-loop flag updated (the loop may have ended).
    #[doc(hidden)]
    pub fn jit_fetch(&mut self, fetched: u32, astat: u32, flag_in: u8, p: &mut JitPipe) -> u64 {
        let (a, f) = (self.astat, self.flag_in);
        (self.astat, self.flag_in) = (astat, flag_in);
        self.fetch_status = (self.astat(), self.mode1);
        (self.astat, self.flag_in) = (a, f);
        let (succ, looped) = self.next_after(fetched);
        p.loop_end = self.jit_loop_end();
        p.short_loop = self.jit_short_loop() as u32;
        succ as u64 | (looped as u64) << 32
    }

    /// The loop and PC stacks for a word of a pipeline block: these can't
    /// fault there (the block leaves before a word that would overflow or
    /// empty a stack, which then runs as a single step).
    #[doc(hidden)]
    pub fn jit_stack_room(&self) -> u32 {
        (PC_STACK - self.pc_stack.len()) as u32
            | ((LOOP_STACK - self.loop_stack.len()) as u32) << 8
            | (self.pc_stack.len() as u32) << 16
            | (self.loop_stack.len() as u32) << 24
    }

    /// What a 32-bit store to `addr` (a normal internal address) must clear
    /// of what is cached of the words it touches (`invalidate`), after an
    /// inline store found something there (`jit_cached`).
    #[doc(hidden)]
    pub fn jit_invalidate32(&mut self, addr: u32) {
        self.invalidate(addr, false);
    }

    #[doc(hidden)]
    pub fn jit_push_pc(&mut self, v: u32, p: &mut JitPipe) {
        let _ = self.push_pc(v);
        p.loop_end = self.jit_loop_end();
    }

    /// Pop the PC stack: the address.
    #[doc(hidden)]
    pub fn jit_pop_pc(&mut self) -> u32 {
        self.pop_pc().unwrap_or(0)
    }

    /// A loop abort (`JUMP (LA)`): the loop and PC stacks popped.
    #[doc(hidden)]
    pub fn jit_abort_loop(&mut self, p: &mut JitPipe) {
        self.pop_loop();
        self.pop_pc();
        p.loop_end = self.jit_loop_end();
        p.short_loop = self.jit_short_loop() as u32;
    }

    /// RTS (LR): the loop re-entry.
    #[doc(hidden)]
    pub fn jit_reentry(&mut self) {
        self.reentry = self.last_loopback.is_some();
    }

    /// A DO at `pc` (type 12 or 13; not a one-word loop, which rebuilds the
    /// pipeline): `count` the new LCNTR if it loads one.
    #[doc(hidden)]
    pub fn jit_do(&mut self, pc: u32, count: Option<u32>, term: u8, end: u32, p: &mut JitPipe) {
        if let Some(n) = count {
            self.lcntr = n;
        }
        let len = end.wrapping_sub(pc);
        let kind = match (term == LCE, len) {
            (false, _) => LoopKind::Cond,
            (true, 1) => LoopKind::Counter1,
            (true, 2) => LoopKind::Counter2,
            _ => LoopKind::Counter,
        };
        let _ = self.push_pc(pc.wrapping_add(1));
        let _ = self.push_loop(laddr_pack(end, term, kind), self.lcntr);
        p.loop_end = self.jit_loop_end();
        p.short_loop = self.jit_short_loop() as u32;
    }

    /// Type 20's loop and PC stack operations (not the status stack's).
    #[doc(hidden)]
    pub fn jit_stacks(&mut self, s: crate::StackOps, p: &mut JitPipe) {
        if s.push_loop {
            let _ = self.push_loop(0, self.lcntr);
        }
        if s.pop_loop {
            self.pop_loop();
        }
        if s.push_pcstk {
            let _ = self.push_pc(0);
        }
        if s.pop_pcstk {
            self.pop_pc();
        }
        p.loop_end = self.jit_loop_end();
        p.short_loop = self.jit_short_loop() as u32;
    }
}

/// Offsets for pipeline blocks' translated code.
#[doc(hidden)]
pub mod jit_layout {
    use super::Sharc;
    use std::mem::offset_of;
    pub const MODE2: usize = offset_of!(Sharc, mode2);
    pub const FLAG_IN: usize = offset_of!(Sharc, flag_in);
    pub const LCNTR: usize = offset_of!(Sharc, lcntr);
}
