//! The MB86235 (Model 2C's copro and geometrizer TGP) on Cranelift
//! (docs/jit.md, "Copro JITs"): blocks of the uploaded program translated
//! to machine code, each 64-bit word lowered to IR exactly as
//! `mb86235::cpu::Tgp::exec` defines it: the transfer's source read, then
//! the ALU's and multiplier's, the ALU, multiplier and transfer results
//! written in that order, POP last; conditions on the flags from before
//! the word; one delay slot after every branch, call and return; REP
//! holding the PC on the next word.
//!
//! Timing is the interpreter's word for word: [`Mb86235Jit::run`] has
//! `Tgp::run`'s contract (up to `max` words, stopping at a word that reads
//! the empty input FIFO), and a block counts its words against that budget
//! and leaves after any of them when it runs out, so a run of 2 words (the
//! copro, after each i960 instruction) stops exactly where the
//! interpreter's does, with a delayed branch or a repeat under way as the
//! interpreter would leave it. A stall leaves before the word, as there.
//!
//! What a block can't hold is left to the interpreter's step: words it
//! has no meaning for (MOVI, MOV4: `Step::Unsupported`), a branch in a
//! delay slot, a branch under a repeat, and a block start with a delayed
//! branch pending (the dispatcher runs that one word interpreted).

use crate::copro::{
    block_signature, call, install, module, sig, trusted, Background, CoproStats, OwnedModule, F32, I32, I64, I8,
};
use crate::hotlist::{Chip, HotList};
use crate::pool::{Compiled, Urgency};
use ::mb86235::cpu::{External, Step, Tgp};
use ::mb86235::{decode, AluOp, BranchTarget, Control, Insn, Loc, Mem, MulOp, Transfer, PROGRAM_WORDS};
use cranelift_codegen::ir::{
    condcodes::{FloatCC, IntCC},
    Block as ClifBlock, InstBuilder, MemFlagsData, SigRef, Type, Value,
};
use cranelift_frontend::{FunctionBuilder, Variable};
use std::mem::{offset_of, ManuallyDrop};
use std::sync::Arc;
use std::time::Instant;

/// Words per block at most, by default ([`Mb86235Jit::with_max_block`]).
pub const MAX_BLOCK: usize = 64;

// Status register bits (`mb86235::cpu`; docs/hw/mb86235.md 1.3).
const AD: u32 = 1 << 0;
const AU: u32 = 1 << 1;
const AV: u32 = 1 << 2;
const AZ: u32 = 1 << 3;
const AN: u32 = 1 << 4;
const ZC: u32 = 1 << 8;
const IL: u32 = 1 << 9;
const NR: u32 = 1 << 10;
const ZD: u32 = 1 << 11;
const RP: u32 = 1 << 14;
const LP: u32 = 1 << 15;
const MD: u32 = 1 << 16;
const MU: u32 = 1 << 17;
const MV: u32 = 1 << 18;
const MZ: u32 = 1 << 19;
const MN: u32 = 1 << 20;
const ALU_FLAGS: u32 = AN | AZ | AV | AU | AD | ZC | ZD | NR | IL;

const FLOAT_CONSTS: [f32; 8] = [-1.0, 0.0, 0.5, 1.0, 1.5, 2.0, 3.0, 5.0];
const INT_CONSTS: [u32; 8] = [0, 1, u32::MAX, 0, 0, 0, 0, 0];

/// A block's status: it ran (to its end, or its budget's), the input
/// FIFO stalled it, or (a start it can't take) the dispatcher must
/// interpret one word.
const RAN: u32 = 0;
const STALLED: u32 = 1;
const DECLINED: u32 = 2;

/// A translated block: `(tgp, ext, ctx) -> status`, `ext` pointing at a
/// `&mut dyn External`.
type BlockFn = unsafe extern "C" fn(*mut Tgp, *mut u8, *mut Ctx) -> u32;

/// Passed to every block: the RAMs and counts (their `Vec`s don't move
/// while it runs), the words it may run, and on return the words it ran.
#[repr(C)]
struct Ctx {
    aram: *mut u32,
    bram: *mut u32,
    counts: *mut u64,
    budget: u64,
    ran: u64,
}

struct Block {
    code: BlockFn,
}

/// Table entries: not translated, nothing to translate, being translated
/// (on a worker), else block + `FIRST`.
const COLD: u32 = 0;
const NEVER: u32 = 1;
const QUEUED: u32 = 2;
const FIRST: u32 = 3;

/// The JIT for one uploaded program (found again by its
/// `mb86235::cpu::program_hash`; another program clears it).
pub struct Mb86235Jit {
    module: OwnedModule,
    blocks: Vec<Block>,
    table: Box<[u32]>,
    program: Vec<u64>,
    hash: Option<u64>,
    max_block: usize,
    stats: CoproStats,
    jobs: Background,
    /// The hot-block list (docs/jit.md, "Hot-block list").
    list: Option<Arc<HotList>>,
}

impl Mb86235Jit {
    pub fn new() -> Result<Self, String> {
        Self::with_max_block(MAX_BLOCK)
    }

    /// With blocks of at most `max` words: shorter for a chip run a few
    /// words at a time (the copro, two words an i960 instruction), whose
    /// blocks start almost anywhere and rarely run to their end, so that
    /// translating them costs less.
    pub fn with_max_block(max: usize) -> Result<Self, String> {
        let module = module(false)?;
        Ok(Mb86235Jit {
            module: OwnedModule(ManuallyDrop::new(module)),
            blocks: Vec::new(),
            table: vec![COLD; PROGRAM_WORDS as usize].into_boxed_slice(),
            program: Vec::new(),
            hash: None,
            max_block: max.max(2),
            stats: CoproStats::default(),
            jobs: Background::new(),
            list: None,
        })
    }

    /// Translate on the background workers (the default: a block is
    /// interpreted until its code is ready), or at once, on the thread
    /// that runs the chip.
    pub fn set_background(&mut self, on: bool) {
        self.jobs.on = on;
    }

    /// Note translated blocks in `list`, and translate the blocks it lists
    /// for each program as the chip gets it, behind any the chip is
    /// waiting for.
    pub fn set_hot_list(&mut self, list: Arc<HotList>) {
        self.list = Some(list);
        self.preload();
    }

    pub fn stats(&self) -> CoproStats {
        self.stats
    }

    /// Queue the listed blocks of the program this JIT has (hints: any
    /// start in range that plans to a block).
    fn preload(&mut self) {
        let (Some(list), Some(hash)) = (&self.list, self.hash) else { return };
        for pc in list.get(Chip::Mb86235, hash) {
            if pc < PROGRAM_WORDS && self.table[pc as usize] == COLD {
                self.queue(pc, Urgency::Later);
            }
        }
    }

    /// Translate for the program `t` holds, if it isn't the one this JIT
    /// has (the old blocks' code stays allocated until the JIT is
    /// dropped: programs are uploaded once a boot).
    fn follow(&mut self, t: &Tgp) {
        if self.hash != Some(t.program_hash) {
            self.blocks.clear();
            self.table.fill(COLD);
            self.program.clone_from(&t.program);
            self.hash = Some(t.program_hash);
            self.jobs.clear();
            self.preload();
        }
    }

    /// [`Tgp::run`] on translated code: up to `max` words, stopping before
    /// one that reads the empty input FIFO (`Step::Stalled`) or that the
    /// chip model can't execute (`Step::Unsupported`), exactly where and
    /// as the interpreter does.
    pub fn run<E: External>(&mut self, t: &mut Tgp, ext: &mut E, max: u64) -> Step {
        self.follow(t);
        let mut left = max;
        while left > 0 {
            if t.pending().is_none() {
                let id = match self.table[t.pc as usize] {
                    COLD | QUEUED => self.cold(t.pc),
                    NEVER => None,
                    s => {
                        self.jobs.through = (0, 0);
                        Some(s - FIRST)
                    }
                };
                if let Some(id) = id {
                    let mut ctx = Ctx {
                        aram: t.aram.as_mut_ptr(),
                        bram: t.bram.as_mut_ptr(),
                        counts: t.counts.as_mut_ptr(),
                        budget: left,
                        ran: 0,
                    };
                    let mut e: &mut dyn External = ext;
                    let code = self.blocks[id as usize].code;
                    // SAFETY: made by `translate` for this program; it
                    // touches the TGP by field offsets and through the
                    // helpers below, its RAMs within their 1024 words and
                    // its counts within the program's 4096.
                    let status = unsafe { code(t, &mut e as *mut &mut dyn External as *mut u8, &mut ctx) };
                    t.steps += ctx.ran;
                    left -= ctx.ran;
                    self.stats.runs += 1;
                    match status {
                        RAN => continue,
                        STALLED => return Step::Stalled,
                        _ => {}
                    }
                }
            }
            self.stats.interpreted += 1;
            match t.step(ext) {
                Step::Ran => left -= 1,
                other => return other,
            }
        }
        Step::Ran
    }

    /// A start with no block yet: queue it (unless it is inside a queued
    /// block being interpreted), install what the workers have finished,
    /// and say whether its block is among them.
    #[cold]
    fn cold(&mut self, pc: u32) -> Option<u32> {
        if self.table[pc as usize] == COLD {
            if self.jobs.inside(pc) {
                return None;
            }
            self.queue(pc, Urgency::Now);
        }
        self.install(false, pc);
        match self.table[pc as usize] {
            s if s >= FIRST => Some(s - FIRST),
            QUEUED => {
                self.jobs.through = (pc, self.jobs.reached(pc).unwrap_or(0));
                None
            }
            _ => None,
        }
    }

    /// Wait for the blocks being translated, and install them (tests).
    pub fn wait(&mut self) {
        self.install(true, 0);
    }

    fn insn(&self, pc: u32) -> Insn {
        decode(pc, self.program.get(pc as usize).copied().unwrap_or(0))
    }

    /// The words of the block at `start` (see [`Planned`]).
    fn plan(&self, start: u32) -> Vec<Planned> {
        let mut v: Vec<Planned> = Vec::new();
        let mut pc = start;
        let mut after_rep = true; // the first word may start under a repeat
        while v.len() < self.max_block {
            let i = self.insn(pc);
            if matches!(i.transfer, Transfer::Unsupported(_)) || (after_rep && branches(&i) && !v.is_empty()) {
                break;
            }
            let rep = matches!(i.transfer, Transfer::Control(Control::Rep(_) | Control::RepAr(_)));
            v.push(Planned { insn: i, dynamic_rep: after_rep, delay_slot: false, ends_with_pending: false });
            after_rep = rep;
            pc = (pc + 1) % PROGRAM_WORDS;
            if branches(&i) {
                let d = self.insn(pc);
                if matches!(d.transfer, Transfer::Unsupported(_)) || branches(&d) || v.len() + 1 > self.max_block {
                    // The delay slot to the interpreter: leave with the
                    // branch pending.
                    v.last_mut().expect("just pushed").ends_with_pending = true;
                    break;
                }
                let drep = matches!(d.transfer, Transfer::Control(Control::Rep(_) | Control::RepAr(_)));
                v.push(Planned { insn: d, dynamic_rep: false, delay_slot: true, ends_with_pending: false });
                after_rep = drep;
                pc = (pc + 1) % PROGRAM_WORDS;
                if !conditional(&i) {
                    break;
                }
            }
        }
        v
    }

    /// Queue the block at `start` for translation, or mark it as nothing
    /// to translate.
    fn queue(&mut self, start: u32, urgency: Urgency) {
        let words = self.plan(start);
        if words.is_empty() {
            if urgency == Urgency::Now {
                self.table[start as usize] = NEVER;
            }
            return;
        }
        let end = start + words.len() as u32;
        self.table[start as usize] = QUEUED;
        self.jobs.queue(self.hash.unwrap_or(0), start, end, urgency, Some(I32), move |b, ptr| {
            Emit::new(b, ptr).block(&words);
        });
    }

    /// Put the finished translation for `want` in the table (`wait`:
    /// every one queued), throwing away any for another program.
    fn install(&mut self, wait: bool, want: u32) {
        let done = self.jobs.finished(wait, want);
        if done.is_empty() {
            return;
        }
        let t0 = Instant::now();
        let mut keep = Vec::new();
        for d in done {
            let ours = Some(d.key) == self.hash && self.table[d.start as usize] == QUEUED;
            match d.code {
                Ok(c) if ours => keep.push((d.start, d.end, c)),
                Err(e) if ours => {
                    eprintln!("jit: MB86235 block at {:#x}: {e}", d.start);
                    self.table[d.start as usize] = NEVER;
                }
                _ => self.stats.discarded += 1,
            }
        }
        if let (Some(list), Some(hash)) = (&self.list, self.hash) {
            list.add(Chip::Mb86235, hash, keep.iter().map(|k| k.0));
        }
        let sig = block_signature(&self.module.0, Some(I32));
        let compiled: Vec<&Compiled> = keep.iter().map(|(_, _, c)| c).collect();
        let entries = install(&mut self.module.0, &sig, &compiled);
        for ((start, end, c), entry) in keep.iter().zip(entries) {
            self.stats.time += c.time;
            self.stats.slowest = self.stats.slowest.max(c.time);
            let Some((ptr, size)) = entry else {
                self.table[*start as usize] = NEVER;
                continue;
            };
            // SAFETY: made with the signature of `BlockFn` (the C convention).
            let code = unsafe { std::mem::transmute::<*const u8, BlockFn>(ptr) };
            self.blocks.push(Block { code });
            self.table[*start as usize] = self.blocks.len() as u32 - 1 + FIRST;
            self.stats.blocks += 1;
            self.stats.words += (end - start) as u64;
            self.stats.code_bytes += size as u64;
        }
        self.stats.time += t0.elapsed();
    }
}

/// A word of a block.
struct Planned {
    insn: Insn,
    /// It may run under a repeat (the block's first word, or the word
    /// after a REP): its PC hold is checked as it runs.
    dynamic_rep: bool,
    /// It is the delay slot of the word before (a branch).
    delay_slot: bool,
    /// It branches, and the block ends before its delay slot.
    ends_with_pending: bool,
}

/// Does the word transfer control (after a delay slot)?
fn branches(i: &Insn) -> bool {
    matches!(
        i.transfer,
        Transfer::Control(
            Control::Branch { .. }
                | Control::Jump(_)
                | Control::Loop(_)
                | Control::BitBranch { .. }
                | Control::Call { .. }
                | Control::Ret
        )
    )
}

/// Does it only sometimes?
fn conditional(i: &Insn) -> bool {
    matches!(
        i.transfer,
        Transfer::Control(
            Control::Branch { .. } | Control::Loop(_) | Control::BitBranch { .. } | Control::Call { cond: Some(_), .. }
        )
    )
}

/// A word that does nothing (a repeated one is skipped to its last
/// repetition, as the interpreter does).
fn is_nop(i: &Insn) -> bool {
    i.alu.is_none() && i.mul.is_none() && matches!(i.transfer, Transfer::None | Transfer::Control(Control::Nop))
}

const OFF_AA: i32 = offset_of!(Tgp, aa) as i32;
const OFF_AB: i32 = offset_of!(Tgp, ab) as i32;
const OFF_MA: i32 = offset_of!(Tgp, ma) as i32;
const OFF_MB: i32 = offset_of!(Tgp, mb) as i32;
const OFF_AR: i32 = offset_of!(Tgp, ar) as i32;
const OFF_PR: i32 = offset_of!(Tgp, pr) as i32;
const OFF_PRP: i32 = offset_of!(Tgp, prp) as i32;
const OFF_PWP: i32 = offset_of!(Tgp, pwp) as i32;
const OFF_EB: i32 = offset_of!(Tgp, eb) as i32;
const OFF_EO: i32 = offset_of!(Tgp, eo) as i32;
const OFF_SP: i32 = offset_of!(Tgp, sp) as i32;
const OFF_ST: i32 = offset_of!(Tgp, st) as i32;
const OFF_MD: i32 = offset_of!(Tgp, md) as i32;
const OFF_LPC: i32 = offset_of!(Tgp, lpc) as i32;
const OFF_RPC: i32 = offset_of!(Tgp, rpc) as i32;
const OFF_PDR: i32 = offset_of!(Tgp, pdr) as i32;
const OFF_DDR: i32 = offset_of!(Tgp, ddr) as i32;
const OFF_PC: i32 = offset_of!(Tgp, pc) as i32;
const OFF_PCS: i32 = offset_of!(Tgp, pcs) as i32;
const OFF_PCSP: i32 = offset_of!(Tgp, pcsp) as i32;
const OFF_EXT_FLAGS: i32 = offset_of!(Tgp, ext_flags) as i32;
const OFF_STEPS: i32 = offset_of!(Tgp, steps) as i32;

/// A branch a word takes, or may: whether, and where.
#[derive(Clone, Copy)]
struct Pend {
    taken: Value,
    target: Value,
}

struct Sigs {
    fifo_len: SigRef,
    pop: SigRef,
    push_out: SigRef,
    flags: SigRef,
    ext_read: SigRef,
    ext_write: SigRef,
    clrf: SigRef,
    set_pending: SigRef,
    f2: SigRef,
    f1: SigRef,
}

struct Emit<'a, 'b> {
    b: &'a mut FunctionBuilder<'b>,
    ptr: Type,
    t: Value,
    ext: Value,
    ctx: Value,
    aram: Value,
    bram: Value,
    counts: Value,
    /// Words the budget has left.
    left: Variable,
    /// The shared exit: (pc, a branch pending?, its target, status).
    exit: ClifBlock,
    sigs: Sigs,
}

impl<'a, 'b> Emit<'a, 'b> {
    fn new(b: &'a mut FunctionBuilder<'b>, ptr: Type) -> Self {
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let p = b.block_params(entry).to_vec();
        let aram = b.ins().load(ptr, trusted(), p[2], offset_of!(Ctx, aram) as i32);
        let bram = b.ins().load(ptr, trusted(), p[2], offset_of!(Ctx, bram) as i32);
        let counts = b.ins().load(ptr, trusted(), p[2], offset_of!(Ctx, counts) as i32);
        let budget = b.ins().load(I64, trusted(), p[2], offset_of!(Ctx, budget) as i32);
        let left = b.declare_var(I64);
        b.def_var(left, budget);
        let exit = b.create_block();
        for _ in 0..4 {
            b.append_block_param(exit, I32);
        }
        let sigs = Sigs {
            fifo_len: sig(b, &[ptr], Some(I32)),
            pop: sig(b, &[ptr], Some(I32)),
            push_out: sig(b, &[ptr, I32], None),
            flags: sig(b, &[ptr], Some(I32)),
            ext_read: sig(b, &[ptr, I32], Some(I32)),
            ext_write: sig(b, &[ptr, I32, I32], None),
            clrf: sig(b, &[ptr, I32], None),
            set_pending: sig(b, &[ptr, I32], None),
            f2: sig(b, &[I32, I32], Some(I32)),
            f1: sig(b, &[I32], Some(I32)),
        };
        Emit { b, ptr, t: p[0], ext: p[1], ctx: p[2], aram, bram, counts, left, exit, sigs }
    }

    fn k(&mut self, v: u32) -> Value {
        self.b.ins().iconst(I32, v as i64)
    }

    fn ld(&mut self, off: i32) -> Value {
        self.b.ins().load(I32, trusted(), self.t, off)
    }

    fn st(&mut self, off: i32, v: Value) {
        self.b.ins().store(trusted(), v, self.t, off);
    }

    fn helper(&mut self, s: SigRef, f: usize, args: &[Value]) -> Option<Value> {
        call(self.b, self.ptr, s, f, args)
    }

    /// Jump to the exit (a block param list: pc, pending?, target,
    /// status).
    fn leave(&mut self, pc: Value, pend: Option<Pend>, status: u32) {
        let (taken, target) = match pend {
            Some(p) => (self.b.ins().uextend(I32, p.taken), p.target),
            None => (self.k(0), self.k(0)),
        };
        let s = self.k(status);
        self.b.ins().jump(self.exit, &[pc.into(), taken.into(), target.into(), s.into()]);
    }

    // ---- the block

    fn block(&mut self, words: &[Planned]) {
        let mut pending: Option<Pend> = None;
        let mut first = true;
        for p in words {
            let pc = p.insn.addr;
            if first && branches(&p.insn) {
                // A branch can't start a block under a repeat: decline it.
                let rpc = self.ld(OFF_RPC);
                let (go, no) = (self.b.create_block(), self.b.create_block());
                self.b.ins().brif(rpc, no, &[], go, &[]);
                self.b.switch_to_block(no);
                let here = self.k(pc);
                self.leave(here, None, DECLINED);
                self.b.switch_to_block(go);
            }
            first = false;
            let delayed = if p.delay_slot { pending.take() } else { None };
            let branch = self.word(p, delayed);
            if p.delay_slot {
                // After the delay slot: to the target if taken, else on.
                let d = delayed.expect("a delay slot follows a branch");
                let (go_on, out) = (self.b.create_block(), self.b.create_block());
                self.b.ins().brif(d.taken, out, &[], go_on, &[]);
                self.b.switch_to_block(out);
                self.leave(d.target, None, RAN);
                self.b.switch_to_block(go_on);
            }
            if p.ends_with_pending {
                let next = self.k((pc + 1) % PROGRAM_WORDS);
                self.leave(next, branch, RAN);
                return self.finish();
            }
            pending = branch;
        }
        let last = words.last().expect("a block has words");
        let next = self.k((last.insn.addr + 1) % PROGRAM_WORDS);
        self.leave(next, None, RAN);
        self.finish()
    }

    /// The exit: store the PC, any branch pending, the words run.
    fn finish(&mut self) {
        self.b.switch_to_block(self.exit);
        let p = self.b.block_params(self.exit).to_vec();
        self.st(OFF_PC, p[0]);
        let (t, s) = (self.t, self.sigs.set_pending);
        let (yes, done) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(p[1], yes, &[], done, &[]);
        self.b.switch_to_block(yes);
        self.helper(s, h_set_pending as HSetPending as usize, &[t, p[2]]);
        self.b.ins().jump(done, &[]);
        self.b.switch_to_block(done);
        let budget = self.b.ins().load(I64, trusted(), self.ctx, offset_of!(Ctx, budget) as i32);
        let left = self.b.use_var(self.left);
        let ran = self.b.ins().isub(budget, left);
        self.b.ins().store(trusted(), ran, self.ctx, offset_of!(Ctx, ran) as i32);
        self.b.ins().return_(&[p[3]]);
    }

    /// One word (with its repetitions, if it may be held by a repeat):
    /// what it branches to, if it does.
    fn word(&mut self, p: &Planned, delayed: Option<Pend>) -> Option<Pend> {
        let i = &p.insn;
        let pc = i.addr;
        let head = self.b.create_block();
        self.b.ins().jump(head, &[]);
        self.b.switch_to_block(head);
        // A stall leaves before the word, which then runs again.
        if Tgp::reads_fifo(&i.transfer) {
            let t = self.t;
            let s = self.sigs.fifo_len;
            let n = self.helper(s, h_fifo_len as HFifoLen as usize, &[t]).expect("returns");
            let (go, stall) = (self.b.create_block(), self.b.create_block());
            self.b.ins().brif(n, go, &[], stall, &[]);
            self.b.switch_to_block(stall);
            self.b.set_cold_block(stall);
            let here = self.k(pc);
            self.leave(here, delayed, STALLED);
            self.b.switch_to_block(go);
        }
        if p.dynamic_rep && is_nop(i) {
            // `rpc > 1`: skip to the last repetition (counted, as there,
            // as one word of the budget).
            let rpc = self.ld(OFF_RPC);
            let many = self.b.ins().icmp_imm_u(IntCC::UnsignedGreaterThan, rpc, 1);
            let (skip, go) = (self.b.create_block(), self.b.create_block());
            self.b.ins().brif(many, skip, &[], go, &[]);
            self.b.switch_to_block(skip);
            let extra = self.b.ins().iadd_imm_s(rpc, -1);
            let extra = self.b.ins().uextend(I64, extra);
            let steps = self.b.ins().load(I64, trusted(), self.t, OFF_STEPS);
            let steps = self.b.ins().iadd(steps, extra);
            self.b.ins().store(trusted(), steps, self.t, OFF_STEPS);
            self.count(pc, Some(extra));
            let one = self.k(1);
            self.st(OFF_RPC, one);
            self.b.ins().jump(go, &[]);
            self.b.switch_to_block(go);
        }

        let (branch, rep_set) = self.semantics(i);

        // Next PC: a repeat holds it; else the next word (the block's
        // sequencing), or a delayed branch's target (the caller's).
        let held = if p.dynamic_rep && !rep_set {
            let rpc = self.ld(OFF_RPC);
            let (dec, no) = (self.b.create_block(), self.b.create_block());
            let held = self.b.append_block_param(no, I8);
            let f = self.b.ins().iconst(I8, 0);
            self.b.ins().brif(rpc, dec, &[], no, &[f.into()]);
            self.b.switch_to_block(dec);
            let rpc = self.b.ins().iadd_imm_s(rpc, -1);
            self.st(OFF_RPC, rpc);
            let st = self.ld(OFF_ST);
            let set = self.b.ins().bor_imm_u(st, RP as i64);
            let clear = self.b.ins().band_imm_u(st, !RP as i64);
            let st = self.b.ins().select(rpc, set, clear);
            self.st(OFF_ST, st);
            let h = self.b.ins().icmp_imm_u(IntCC::NotEqual, rpc, 0);
            self.b.ins().jump(no, &[h.into()]);
            self.b.switch_to_block(no);
            Some(held)
        } else {
            None
        };
        self.count(pc, None);
        let left = self.b.use_var(self.left);
        let left = self.b.ins().iadd_imm_s(left, -1);
        self.b.def_var(self.left, left);
        let out = self.b.create_block();
        if let Some(held) = held {
            // Held: again (or leave on it if the budget is spent).
            let (again, on) = (self.b.create_block(), self.b.create_block());
            self.b.ins().brif(held, again, &[], on, &[]);
            self.b.switch_to_block(again);
            let (more, spent) = (self.b.create_block(), self.b.create_block());
            self.b.ins().brif(left, more, &[], spent, &[]);
            self.b.switch_to_block(more);
            self.b.ins().jump(head, &[]);
            self.b.switch_to_block(spent);
            let here = self.k(pc);
            self.leave(here, None, RAN);
            self.b.switch_to_block(on);
        }
        let go_on = self.b.create_block();
        self.b.ins().brif(left, go_on, &[], out, &[]);
        self.b.switch_to_block(out);
        // The budget is spent: leave after this word.
        let next = self.k((pc + 1) % PROGRAM_WORDS);
        match delayed {
            Some(d) => {
                let to = self.b.ins().select(d.taken, d.target, next);
                self.leave(to, None, RAN)
            }
            None => self.leave(next, branch, RAN),
        }
        self.b.switch_to_block(go_on);
        branch
    }

    /// `counts[pc] += 1` (or `n`).
    fn count(&mut self, pc: u32, n: Option<Value>) {
        let off = 8 * pc as i32;
        let c = self.b.ins().load(I64, trusted(), self.counts, off);
        let c = match n {
            Some(n) => self.b.ins().iadd(c, n),
            None => self.b.ins().iadd_imm_s(c, 1),
        };
        self.b.ins().store(trusted(), c, self.counts, off);
    }

    /// The word's effects, in the interpreter's order: its branch (if
    /// any) and whether it set a repeat.
    fn semantics(&mut self, i: &Insn) -> (Option<Pend>, bool) {
        let pc = i.addr;
        let mut branch = None;
        let mut pop = None;
        let mut rep_set = false;
        if let Transfer::Control(c) = i.transfer {
            match c {
                Control::Rep(n) => {
                    let v = self.k(n);
                    self.st(OFF_RPC, v);
                    rep_set = true;
                }
                Control::RepAr(x) => {
                    let v = self.ld(OFF_AR + 4 * x as i32);
                    self.st(OFF_RPC, v);
                    rep_set = true;
                }
                Control::SetL(n) => {
                    let v = self.k(n);
                    self.st(OFF_LPC, v);
                    self.st_or(LP);
                }
                Control::SetLAr(x) => {
                    let v = self.ld(OFF_AR + 4 * x as i32);
                    self.st(OFF_LPC, v);
                    self.st_or(LP);
                }
                Control::ClrF { input, output } => {
                    let (t, s) = (self.t, self.sigs.clrf);
                    let which = self.k(input as u32 | (output as u32) << 1);
                    self.helper(s, h_clrf as HClrF as usize, &[t, which]);
                }
                Control::Push(r) => {
                    let v = self.reg_read(r);
                    let sp = self.ld(OFF_SP);
                    let sp = self.b.ins().iadd_imm_s(sp, 1023);
                    let sp = self.b.ins().band_imm_u(sp, 0x3ff);
                    self.st(OFF_SP, sp);
                    let p = self.ram_addr(self.bram, sp, 0x3ff);
                    self.b.ins().store(trusted(), v, p, 0);
                }
                Control::Pop(r) => pop = Some(r),
                Control::SetM { op, value } => {
                    let md = self.ld(OFF_MD);
                    let v = match op {
                        0x08 => self.k(value),
                        0x09 => self.field(md, 7 << 12, (value & 7) << 12),
                        0x0a => self.field(md, 7 << 8, (value & 7) << 8),
                        0x0b => self.field(md, 1 << 7, (value & 1) << 7),
                        0x0c => self.field(md, 1 << 4, (value & 1) << 4),
                        _ => self.field(md, 7, value & 7),
                    };
                    self.st(OFF_MD, v);
                }
                Control::Branch { cond, negate, target } => {
                    let c = self.cond(cond);
                    let taken = if negate { self.b.ins().bxor_imm_u(c, 1) } else { c };
                    let target = self.target_if(taken, target, pc);
                    branch = Some(Pend { taken, target });
                }
                Control::Jump(t) => {
                    let target = self.target(t, pc);
                    let taken = self.b.ins().iconst(I8, 1);
                    branch = Some(Pend { taken, target });
                }
                Control::Loop(t) => {
                    let st = self.ld(OFF_ST);
                    let lp = self.b.ins().band_imm_u(st, LP as i64);
                    let taken = self.b.ins().icmp_imm_u(IntCC::NotEqual, lp, 0);
                    let lpc = self.ld(OFF_LPC);
                    let lpc = self.b.ins().iadd_imm_s(lpc, -1);
                    self.st(OFF_LPC, lpc);
                    let done = self.b.ins().icmp_imm_u(IntCC::UnsignedLessThanOrEqual, lpc, 1);
                    let st = self.ld(OFF_ST);
                    let cleared = self.b.ins().band_imm_u(st, !LP as i64);
                    let st = self.b.ins().select(done, cleared, st);
                    self.st(OFF_ST, st);
                    let target = self.k(t);
                    branch = Some(Pend { taken, target });
                }
                Control::BitBranch { set, arx, bit, target } => {
                    let a = self.ld(OFF_AR + 4 * arx as i32);
                    let b = self.b.ins().ushr_imm_u(a, bit as i64);
                    let b = self.b.ins().band_imm_u(b, 1);
                    let taken = self.b.ins().icmp_imm_u(IntCC::Equal, b, set as i64);
                    let target = self.k(target);
                    branch = Some(Pend { taken, target });
                }
                Control::Call { cond, target } => {
                    let taken = match cond {
                        None => self.b.ins().iconst(I8, 1),
                        Some((c, n)) => {
                            let c = self.cond(c);
                            if n {
                                self.b.ins().bxor_imm_u(c, 1)
                            } else {
                                c
                            }
                        }
                    };
                    let (yes, done) = (self.b.create_block(), self.b.create_block());
                    let tv = self.b.append_block_param(done, I32);
                    let zero = self.k(0);
                    self.b.ins().brif(taken, yes, &[], done, &[zero.into()]);
                    self.b.switch_to_block(yes);
                    let t = self.target(target, pc);
                    let sp = self.b.ins().load(I64, trusted(), self.t, OFF_PCSP);
                    let slot = self.b.ins().band_imm_u(sp, 3);
                    let off = self.b.ins().ishl_imm_u(slot, 2);
                    let at = self.b.ins().iadd(self.t, off);
                    let ret = self.k((pc + 2) % PROGRAM_WORDS);
                    self.b.ins().store(trusted(), ret, at, OFF_PCS);
                    let sp = self.b.ins().iadd_imm_s(sp, 1);
                    let sp = self.b.ins().band_imm_u(sp, 3);
                    self.b.ins().store(trusted(), sp, self.t, OFF_PCSP);
                    self.b.ins().jump(done, &[t.into()]);
                    self.b.switch_to_block(done);
                    branch = Some(Pend { taken, target: tv });
                }
                Control::Ret => {
                    let sp = self.b.ins().load(I64, trusted(), self.t, OFF_PCSP);
                    let sp = self.b.ins().iadd_imm_s(sp, 3);
                    let sp = self.b.ins().band_imm_u(sp, 3);
                    self.b.ins().store(trusted(), sp, self.t, OFF_PCSP);
                    let off = self.b.ins().ishl_imm_u(sp, 2);
                    let at = self.b.ins().iadd(self.t, off);
                    let target = self.b.ins().load(I32, trusted(), at, OFF_PCS);
                    let taken = self.b.ins().iconst(I8, 1);
                    branch = Some(Pend { taken, target });
                }
                Control::Nop | Control::Unknown(_) => {}
            }
        }

        // Read phase: the transfer's source, then the arithmetic's.
        let mut moved: Vec<(Loc, Value)> = Vec::new();
        match i.transfer {
            Transfer::Move { src, dst } => {
                let v = self.loc_read(&src);
                moved.push((dst, v));
            }
            Transfer::Double { a, b } => {
                let va = self.loc_read(&a.0);
                let vb = self.loc_read(&b.0);
                moved.push((a.1, va));
                moved.push((b.1, vb));
            }
            _ => {}
        }
        let alu = i.alu.as_ref().and_then(|a| self.alu(a));
        let mul = i.mul.as_ref().map(|m| self.mul(m));

        // Write phase.
        if let Some((o, v)) = alu {
            self.out_write(o, v);
        }
        if let Some((o, v)) = mul {
            self.out_write(o, v);
        }
        for (dst, v) in moved {
            self.loc_write(&dst, v);
        }
        if let Some(r) = pop {
            let sp = self.ld(OFF_SP);
            let p = self.ram_addr(self.bram, sp, 0x3ff);
            let v = self.b.ins().load(I32, trusted(), p, 0);
            let sp = self.b.ins().iadd_imm_s(sp, 1);
            let sp = self.b.ins().band_imm_u(sp, 0x3ff);
            self.st(OFF_SP, sp);
            self.reg_write(r, v);
        }
        (branch, rep_set)
    }

    fn field(&mut self, v: Value, mask: u32, bits: u32) -> Value {
        let kept = self.b.ins().band_imm_u(v, !mask as i64);
        self.b.ins().bor_imm_u(kept, bits as i64)
    }

    fn st_or(&mut self, bits: u32) {
        let st = self.ld(OFF_ST);
        let st = self.b.ins().bor_imm_u(st, bits as i64);
        self.st(OFF_ST, st);
    }

    /// Branch condition `c` (`Tgp::cond`) on the flags as the word found
    /// them.
    fn cond(&mut self, c: u32) -> Value {
        const BIT: [u32; 25] =
            [20, 19, 18, 17, 11, 10, 9, 8, 4, 3, 2, 1, 16, 0, 99, 99, 28, 29, 30, 99, 26, 27, 24, 25, 99];
        match BIT.get(c as usize) {
            Some(&b) if b < 32 => {
                let f = match b {
                    24..=27 => {
                        let (t, s) = (self.t, self.sigs.flags);
                        self.helper(s, h_flags as HFlags as usize, &[t]).expect("returns")
                    }
                    28..=30 => {
                        let e = self.ld(OFF_EXT_FLAGS);
                        self.b.ins().ishl_imm_u(e, 28)
                    }
                    _ => self.ld(OFF_ST),
                };
                let x = self.b.ins().band_imm_u(f, 1i64 << b);
                self.b.ins().icmp_imm_u(IntCC::NotEqual, x, 0)
            }
            _ => self.b.ins().iconst(I8, 0),
        }
    }

    /// A branch target (`Tgp::target`), read whether or not it's taken
    /// (the interpreter only reads it when taken: a register read with a
    /// side effect, FI, is read only then).
    fn target_if(&mut self, taken: Value, t: BranchTarget, pc: u32) -> Value {
        match t {
            BranchTarget::Reg(0x31) => {
                let (yes, done) = (self.b.create_block(), self.b.create_block());
                let v = self.b.append_block_param(done, I32);
                let zero = self.k(0);
                self.b.ins().brif(taken, yes, &[], done, &[zero.into()]);
                self.b.switch_to_block(yes);
                let tv = self.target(t, pc);
                self.b.ins().jump(done, &[tv.into()]);
                self.b.switch_to_block(done);
                v
            }
            _ => self.target(t, pc),
        }
    }

    fn target(&mut self, t: BranchTarget, pc: u32) -> Value {
        let v = match t {
            BranchTarget::Imm(a) => return self.k(a % PROGRAM_WORDS),
            BranchTarget::Relative(d) => return self.k(pc.wrapping_add(1).wrapping_add(d as u32) % PROGRAM_WORDS),
            BranchTarget::Reg(r) => self.reg_read(r),
            BranchTarget::ARam(a) => {
                let p = self.aram;
                self.b.ins().load(I32, trusted(), p, 4 * (a & 0x3ff) as i32)
            }
            BranchTarget::BRam(a) => {
                let p = self.bram;
                self.b.ins().load(I32, trusted(), p, 4 * (a & 0x3ff) as i32)
            }
            BranchTarget::ARamAr(x) | BranchTarget::BRamAr(x) => {
                let a = self.ld(OFF_AR + 4 * x as i32);
                let ram = if matches!(t, BranchTarget::ARamAr(_)) { self.aram } else { self.bram };
                let p = self.ram_addr(ram, a, 0x3ff);
                self.b.ins().load(I32, trusted(), p, 0)
            }
        };
        self.b.ins().band_imm_u(v, (PROGRAM_WORDS - 1) as i64)
    }

    // ---- registers

    fn bank(code: u32) -> Option<i32> {
        let i = 4 * (code & 7) as i32;
        Some(match code & 0x3f {
            0x00..=0x07 => OFF_MA + i,
            0x08..=0x0f => OFF_AA + i,
            0x18..=0x1f => OFF_AR + i,
            0x20..=0x27 => OFF_MB + i,
            0x28..=0x2f => OFF_AB + i,
            _ => return None,
        })
    }

    /// `pr[prp]`'s address (`prp` is kept below 24; clamped for safety).
    fn pr_at(&mut self, ptr_off: i32) -> Value {
        let p = self.ld(ptr_off);
        let max = self.k(23);
        let p = self.b.ins().umin(p, max);
        let p = self.b.ins().uextend(self.ptr, p);
        let p = self.b.ins().ishl_imm_u(p, 2);
        self.b.ins().iadd(self.t, p)
    }

    fn reg_read(&mut self, code: u32) -> Value {
        if let Some(off) = Self::bank(code) {
            return self.ld(off);
        }
        match code & 0x3f {
            0x10 => self.ld(OFF_EB),
            0x11 => {
                let eb = self.ld(OFF_EB);
                self.b.ins().ushr_imm_u(eb, 14)
            }
            0x12 => {
                let eb = self.ld(OFF_EB);
                self.b.ins().band_imm_u(eb, 0x3fff)
            }
            0x13 => self.ld(OFF_EO),
            0x14 => self.ld(OFF_SP),
            0x15 => {
                let (t, s) = (self.t, self.sigs.flags);
                self.helper(s, h_flags as HFlags as usize, &[t]).expect("returns")
            }
            0x16 => self.ld(OFF_MD),
            0x17 => self.ld(OFF_LPC),
            0x30 => {
                let at = self.pr_at(OFF_PRP);
                self.b.ins().load(I32, trusted(), at, OFF_PR)
            }
            0x31 => {
                let (t, s) = (self.t, self.sigs.pop);
                self.helper(s, h_pop as HPop as usize, &[t]).expect("returns")
            }
            0x34 => self.ld(OFF_PDR),
            0x35 => self.ld(OFF_DDR),
            0x36 => self.ld(OFF_PRP),
            0x37 => self.ld(OFF_PWP),
            _ => self.k(0),
        }
    }

    fn mod24(&mut self, v: Value) -> Value {
        let n = self.k(24);
        self.b.ins().urem(v, n)
    }

    fn reg_write(&mut self, code: u32, v: Value) {
        if let Some(off) = Self::bank(code) {
            let v = if (0x18..=0x1f).contains(&(code & 0x3f)) { self.b.ins().band_imm_u(v, 0x3fff) } else { v };
            return self.st(off, v);
        }
        match code & 0x3f {
            0x10 => {
                let v = self.b.ins().band_imm_u(v, 0xff_ffff);
                self.st(OFF_EB, v)
            }
            0x11 => {
                let eb = self.ld(OFF_EB);
                let lo = self.b.ins().band_imm_u(eb, 0x3fff);
                let hi = self.b.ins().ishl_imm_u(v, 14);
                let x = self.b.ins().bor(hi, lo);
                let x = self.b.ins().band_imm_u(x, 0xff_ffff);
                self.st(OFF_EB, x)
            }
            0x12 => {
                let eb = self.ld(OFF_EB);
                let hi = self.b.ins().band_imm_u(eb, !0x3fffu32 as i64);
                let lo = self.b.ins().band_imm_u(v, 0x3fff);
                let x = self.b.ins().bor(hi, lo);
                self.st(OFF_EB, x)
            }
            0x13 => {
                let v = self.b.ins().band_imm_u(v, 0xff_ffff);
                self.st(OFF_EO, v)
            }
            0x14 => {
                let v = self.b.ins().band_imm_u(v, 0x3ff);
                self.st(OFF_SP, v)
            }
            0x15 => self.st(OFF_ST, v),
            0x16 => {
                let v = self.b.ins().band_imm_u(v, 0xffff);
                self.st(OFF_MD, v)
            }
            0x17 => self.st(OFF_LPC, v),
            0x30 => {
                let at = self.pr_at(OFF_PWP);
                self.b.ins().store(trusted(), v, at, OFF_PR);
                let w = self.ld(OFF_PWP);
                let w = self.b.ins().iadd_imm_s(w, 1);
                let w = self.mod24(w);
                self.st(OFF_PWP, w);
            }
            0x32 | 0x33 => {
                let (t, s) = (self.t, self.sigs.push_out);
                self.helper(s, h_push_out as HPushOut as usize, &[t, v]);
            }
            0x34 => self.st(OFF_PDR, v),
            0x35 => self.st(OFF_DDR, v),
            0x36 => {
                let v = self.mod24(v);
                self.st(OFF_PRP, v)
            }
            0x37 => {
                let v = self.mod24(v);
                self.st(OFF_PWP, v)
            }
            _ => {}
        }
    }

    /// An arithmetic output (5 bits).
    fn out_write(&mut self, code: u32, v: Value) {
        let i = 4 * (code & 7) as i32;
        let off = match code {
            0x00..=0x07 => OFF_MA,
            0x08..=0x0f => OFF_MB,
            0x10..=0x17 => OFF_AA,
            _ => OFF_AB,
        };
        self.st(off + i, v);
    }

    fn pr_read(&mut self, code: u32) -> Value {
        let at = self.pr_at(OFF_PRP);
        let v = self.b.ins().load(I32, trusted(), at, OFF_PR);
        match code {
            0x11 | 0x12 => {
                let p = self.ld(OFF_PRP);
                let p = self.b.ins().iadd_imm_s(p, if code == 0x11 { 1 } else { 23 });
                let p = self.mod24(p);
                self.st(OFF_PRP, p);
            }
            0x13 => {
                let z = self.k(0);
                self.st(OFF_PRP, z);
            }
            _ => {}
        }
        v
    }

    fn in1(&mut self, alu: bool, v: u32) -> Value {
        let off = match (alu, v < 8) {
            (true, true) => OFF_AA,
            (true, false) => OFF_AB,
            (false, true) => OFF_MA,
            (false, false) => OFF_MB,
        };
        self.ld(off + 4 * (v & 7) as i32)
    }

    fn in2(&mut self, alu: bool, v: u32, float: bool) -> Value {
        match v {
            0x00..=0x0f => self.in1(alu, v),
            0x10..=0x13 => self.pr_read(v),
            0x18..=0x1f if float => self.k(FLOAT_CONSTS[(v - 0x18) as usize].to_bits()),
            0x18..=0x1f => self.k(INT_CONSTS[(v - 0x18) as usize]),
            _ => self.k(0),
        }
    }

    // ---- memory

    /// `ram + 4 * (a & mask)`.
    fn ram_addr(&mut self, ram: Value, a: Value, mask: u32) -> Value {
        let i = self.b.ins().band_imm_u(a, mask as i64);
        let i = self.b.ins().uextend(self.ptr, i);
        let off = self.b.ins().ishl_imm_u(i, 2);
        self.b.ins().iadd(ram, off)
    }

    /// `Tgp::ea`: the address, the post-modify applied.
    fn ea(&mut self, m: &Mem) -> Value {
        let (x, y) = (m.arx as i32, m.ary as i32);
        let d = m.disp as u32;
        let ax = self.ld(OFF_AR + 4 * x);
        let ay = self.ld(OFF_AR + 4 * y);
        let addr = match m.mode {
            0 => ax,
            1..=3 => {
                let step = [0, 1, u32::MAX, d][m.mode as usize];
                let n = self.b.ins().iadd_imm_s(ax, step as i32 as i64);
                let n = self.b.ins().band_imm_u(n, 0x3fff);
                self.st(OFF_AR + 4 * x, n);
                ax
            }
            4 => self.b.ins().iadd(ax, ay),
            5..=7 => {
                let step = [0, 0, 0, 0, 0, 1, u32::MAX, d][m.mode as usize];
                let n = self.b.ins().iadd_imm_s(ay, step as i32 as i64);
                let n = self.b.ins().band_imm_u(n, 0x3fff);
                self.st(OFF_AR + 4 * y, n);
                self.b.ins().iadd(ax, ay)
            }
            8 | 9 => {
                let hi = (m.mode == 8) != m.b_bus;
                let part = if hi { self.b.ins().ushr_imm_u(ay, 7) } else { self.b.ins().band_imm_u(ay, 0x7f) };
                self.b.ins().iadd(ax, part)
            }
            0xa => self.b.ins().iadd_imm_s(ax, d as i32 as i64),
            0xb => {
                let s = self.b.ins().iadd(ax, ay);
                self.b.ins().iadd_imm_s(s, d as i32 as i64)
            }
            0xc => self.k(d),
            _ => {
                // Circular: ay & (0x1ff >> (7 - cbs)), cbs from MOD.
                let md = self.ld(OFF_MD);
                let cbs = self.b.ins().ushr_imm_u(md, if m.b_bus { 8 } else { 12 });
                let cbs = self.b.ins().band_imm_u(cbs, 7);
                let seven = self.k(7);
                let sh = self.b.ins().isub(seven, cbs);
                let ones = self.k(0x1ff);
                let mask = self.b.ins().ushr(ones, sh);
                let circ = self.b.ins().band(ay, mask);
                let step = [1, u32::MAX, d][(m.mode - 0xd) as usize];
                let n = self.b.ins().iadd_imm_s(ay, step as i32 as i64);
                let n = self.b.ins().band_imm_u(n, 0x3fff);
                self.st(OFF_AR + 4 * y, n);
                self.b.ins().iadd(ax, circ)
            }
        };
        self.b.ins().band_imm_u(addr, 0x3fff)
    }

    /// An A-bus address at 0x400 or above reaches external memory (EB's
    /// bank); below, A RAM.
    fn mem_access(&mut self, m: &Mem, write: Option<Value>) -> Option<Value> {
        let a = self.ea(m);
        if m.b_bus {
            let p = self.ram_addr(self.bram, a, 0x3ff);
            return match write {
                Some(v) => {
                    self.b.ins().store(trusted(), v, p, 0);
                    None
                }
                None => Some(self.b.ins().load(I32, trusted(), p, 0)),
            };
        }
        let inside = self.b.ins().icmp_imm_u(IntCC::UnsignedLessThan, a, 0x400);
        let (int, ext, done) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        let r = self.b.append_block_param(done, I32);
        self.b.ins().brif(inside, int, &[], ext, &[]);
        self.b.switch_to_block(int);
        let p = self.ram_addr(self.aram, a, 0x3ff);
        let v = match write {
            Some(v) => {
                self.b.ins().store(trusted(), v, p, 0);
                self.k(0)
            }
            None => self.b.ins().load(I32, trusted(), p, 0),
        };
        self.b.ins().jump(done, &[v.into()]);
        self.b.switch_to_block(ext);
        let eb = self.ld(OFF_EB);
        let bank = self.b.ins().band_imm_u(eb, 0xff_c000);
        let at = self.b.ins().iadd(bank, a);
        let v = match write {
            Some(v) => {
                self.ext_write(at, v);
                self.k(0)
            }
            None => self.ext_read(at),
        };
        self.b.ins().jump(done, &[v.into()]);
        self.b.switch_to_block(done);
        write.is_none().then_some(r)
    }

    fn ext_read(&mut self, a: Value) -> Value {
        let (e, s) = (self.ext, self.sigs.ext_read);
        self.helper(s, h_ext_read as HExtRead as usize, &[e, a]).expect("returns")
    }

    fn ext_write(&mut self, a: Value, v: Value) {
        let (e, s) = (self.ext, self.sigs.ext_write);
        self.helper(s, h_ext_write as HExtWrite as usize, &[e, a, v]);
    }

    /// `EB + EO`, then EO steps by `disp`.
    fn eb_eo(&mut self) -> Value {
        let (eb, eo) = (self.ld(OFF_EB), self.ld(OFF_EO));
        let a = self.b.ins().iadd(eb, eo);
        self.b.ins().band_imm_u(a, 0xff_ffff)
    }

    fn eo_step(&mut self, disp: i32) {
        let eo = self.ld(OFF_EO);
        let eo = self.b.ins().iadd_imm_s(eo, disp as i64);
        let eo = self.b.ins().band_imm_u(eo, 0xff_ffff);
        self.st(OFF_EO, eo);
    }

    fn loc_read(&mut self, l: &Loc) -> Value {
        match *l {
            Loc::Reg(r) => self.reg_read(r),
            Loc::Mem(m) => self.mem_access(&m, None).expect("a read"),
            Loc::Imm(v) => self.k(v),
            Loc::Ext { disp } => {
                let a = self.eb_eo();
                let v = self.ext_read(a);
                self.eo_step(disp);
                v
            }
        }
    }

    fn loc_write(&mut self, l: &Loc, v: Value) {
        match *l {
            Loc::Reg(r) => self.reg_write(r, v),
            Loc::Mem(m) => {
                self.mem_access(&m, Some(v));
            }
            Loc::Imm(_) => {}
            Loc::Ext { disp } => {
                let a = self.eb_eo();
                self.ext_write(a, v);
                self.eo_step(disp);
            }
        }
    }

    // ---- arithmetic

    fn fv(&mut self, v: Value) -> Value {
        self.b.ins().bitcast(F32, MemFlagsData::new(), v)
    }

    fn bits(&mut self, x: Value) -> Value {
        self.b.ins().bitcast(I32, MemFlagsData::new(), x)
    }

    /// `x`'s bits, or for a NaN the interpreter's own function's (`slow`
    /// with `args`): `mb86235::float` defines which NaN.
    fn nan_checked(&mut self, x: Value, slow: usize, args: &[Value]) -> Value {
        let bits = self.bits(x);
        let nan = self.b.ins().fcmp(FloatCC::Unordered, x, x);
        let (cold, done) = (self.b.create_block(), self.b.create_block());
        let r = self.b.append_block_param(done, I32);
        self.b.ins().brif(nan, cold, &[], done, &[bits.into()]);
        self.b.switch_to_block(cold);
        self.b.set_cold_block(cold);
        let s = if args.len() == 2 { self.sigs.f2 } else { self.sigs.f1 };
        let v = self.helper(s, slow, args).expect("returns");
        self.b.ins().jump(done, &[v.into()]);
        self.b.switch_to_block(done);
        r
    }

    fn fadd(&mut self, a: Value, b: Value) -> Value {
        let (x, y) = (self.fv(a), self.fv(b));
        let r = self.b.ins().fadd(x, y);
        self.nan_checked(r, h_add as HF2 as usize, &[a, b])
    }

    fn fsub(&mut self, a: Value, b: Value) -> Value {
        let (x, y) = (self.fv(a), self.fv(b));
        let r = self.b.ins().fsub(x, y);
        self.nan_checked(r, h_sub as HF2 as usize, &[a, b])
    }

    fn fmul(&mut self, a: Value, b: Value) -> Value {
        let (x, y) = (self.fv(a), self.fv(b));
        let r = self.b.ins().fmul(x, y);
        self.nan_checked(r, h_mul as HF2 as usize, &[a, b])
    }

    fn fabs_bits(&mut self, a: Value) -> Value {
        self.b.ins().band_imm_u(a, 0x7fff_ffff)
    }

    /// `f(v) < 0` and `f(v) == 0` as truth values.
    fn fneg_zero(&mut self, v: Value) -> (Value, Value) {
        let x = self.fv(v);
        let z = self.b.ins().f32const(0.0);
        let neg = self.b.ins().fcmp(FloatCC::LessThan, x, z);
        let zero = self.b.ins().fcmp(FloatCC::Equal, x, z);
        (neg, zero)
    }

    /// `flag` if `cond`, else 0.
    fn bit_if(&mut self, cond: Value, flag: u32) -> Value {
        let (f, z) = (self.k(flag), self.k(0));
        self.b.ins().select(cond, f, z)
    }

    /// `clamp_negative`: a negative float to +0, and ZC.
    fn clampf(&mut self, v: Value, extra: &mut Vec<Value>) -> Value {
        let (neg, _) = self.fneg_zero(v);
        let zc = self.bit_if(neg, ZC);
        extra.push(zc);
        let z = self.k(0);
        self.b.ins().select(neg, z, v)
    }

    fn clampi(&mut self, v: Value, extra: &mut Vec<Value>) -> Value {
        let neg = self.b.ins().icmp_imm_s(IntCC::SignedLessThan, v, 0);
        let zc = self.bit_if(neg, ZC);
        extra.push(zc);
        let z = self.k(0);
        self.b.ins().select(neg, z, v)
    }

    fn or_all(&mut self, vals: &[Value]) -> Value {
        let mut acc = self.k(0);
        for &v in vals {
            acc = self.b.ins().bor(acc, v);
        }
        acc
    }

    /// `st = (st & !ALU_FLAGS) | AN | AZ | extra`, for a result whose
    /// negative and zero are `neg`, `zero`.
    fn alu_flags(&mut self, neg: Value, zero: Value, extra: &[Value]) {
        let an = self.bit_if(neg, AN);
        let az = self.bit_if(zero, AZ);
        let mut all = vec![an, az];
        all.extend_from_slice(extra);
        let f = self.or_all(&all);
        let st = self.ld(OFF_ST);
        let st = self.b.ins().band_imm_u(st, !ALU_FLAGS as i64);
        let st = self.b.ins().bor(st, f);
        self.st(OFF_ST, st);
    }

    fn alu_flags_f(&mut self, v: Value, extra: &[Value]) {
        let (neg, zero) = self.fneg_zero(v);
        self.alu_flags(neg, zero, extra);
    }

    fn alu_flags_i(&mut self, v: Value, extra: &[Value]) {
        let neg = self.b.ins().icmp_imm_s(IntCC::SignedLessThan, v, 0);
        let zero = self.b.ins().icmp_imm_u(IntCC::Equal, v, 0);
        self.alu_flags(neg, zero, extra);
    }

    /// `Tgp::alu`: the output and its value, if it writes one.
    fn alu(&mut self, a: &AluOp) -> Option<(u32, Value)> {
        let float = a.is_float();
        let x = self.in1(true, a.in1);
        let imm = matches!(a.op, 0x08 | 0x09 | 0x1c..=0x1f);
        let y = if imm { self.k(a.in2) } else { self.in2(true, a.in2, float) };
        let mut extra: Vec<Value> = Vec::new();
        let r = match a.op {
            0x00 => self.fadd(x, y),
            0x01 => {
                let s = self.fadd(x, y);
                self.clampf(s, &mut extra)
            }
            0x02 => self.fsub(y, x),
            0x03 => {
                let s = self.fsub(y, x);
                self.clampf(s, &mut extra)
            }
            0x04 => {
                let d = self.fsub(y, x);
                self.alu_flags_f(d, &[]);
                return None;
            }
            0x05 => self.fabs_bits(x),
            0x06 => {
                let (ay, ax) = (self.fabs_bits(y), self.fabs_bits(x));
                let d = self.fsub(ay, ax);
                self.alu_flags_f(d, &[]);
                return None;
            }
            0x08 | 0x09 => {
                let exp = self.b.ins().ushr_imm_u(x, 23);
                let exp = self.b.ins().band_imm_u(exp, 0xff);
                let e = if a.op == 0x08 {
                    self.b.ins().iadd_imm_s(exp, a.in2 as i64)
                } else {
                    self.b.ins().iadd_imm_s(exp, -(a.in2 as i64))
                };
                let (lo, hi) = (self.k(0), self.k(255));
                let e = self.b.ins().smax(e, lo);
                let e = self.b.ins().smin(e, hi);
                let e = self.b.ins().ishl_imm_u(e, 23);
                let kept = self.b.ins().band_imm_u(x, !(0xffu32 << 23) as i64);
                let scaled = self.b.ins().bor(kept, e);
                let z = self.b.ins().icmp_imm_u(IntCC::Equal, exp, 0);
                self.b.ins().select(z, x, scaled)
            }
            0x0a => {
                let fx = self.fv(x);
                let zero = self.b.ins().f32const(0.0);
                let z = self.b.ins().fcmp(FloatCC::Equal, fx, zero);
                let zd = self.bit_if(z, ZD);
                extra.push(zd);
                let one = self.b.ins().f32const(1.0);
                let r = self.b.ins().fdiv(one, fx);
                self.nan_checked(r, h_rcp as HF1 as usize, &[x])
            }
            0x0b => {
                let fx = self.fv(x);
                let zero = self.b.ins().f32const(0.0);
                let le = self.b.ins().fcmp(FloatCC::LessThanOrEqual, fx, zero);
                let nr = self.bit_if(le, NR);
                extra.push(nr);
                let s = self.b.ins().sqrt(fx);
                let one = self.b.ins().f32const(1.0);
                let r = self.b.ins().fdiv(one, s);
                self.nan_checked(r, h_rsq as HF1 as usize, &[x])
            }
            0x0c => {
                let fx = self.fv(x);
                let zero = self.b.ins().f32const(0.0);
                let le = self.b.ins().fcmp(FloatCC::LessThanOrEqual, fx, zero);
                let il = self.bit_if(le, IL);
                extra.push(il);
                let s = self.sigs.f1;
                self.helper(s, h_log2 as HF1 as usize, &[x]).expect("returns")
            }
            0x0d => {
                let f = self.b.ins().fcvt_from_sint(F32, x);
                self.bits(f)
            }
            0x0e => {
                let fx = self.fv(x);
                self.b.ins().fcvt_to_sint_sat(I32, fx)
            }
            0x0f => {
                let fx = self.fv(x);
                let (zero, top) = (self.b.ins().f32const(0.0), self.b.ins().f32const(255.0));
                let lt = self.b.ins().fcmp(FloatCC::LessThan, fx, zero);
                let gt = self.b.ins().fcmp(FloatCC::GreaterThan, fx, top);
                let (au, av, z) = (self.k(AU), self.k(AV), self.k(0));
                let v = self.b.ins().select(gt, av, z);
                let v = self.b.ins().select(lt, au, v);
                extra.push(v);
                // clamp(0, 255) then to an integer: NaN gives 0, as the
                // saturating conversion does.
                let u = self.b.ins().fcvt_to_uint_sat(I32, fx);
                let max = self.k(255);
                self.b.ins().umin(u, max)
            }
            0x10 => self.b.ins().iadd(x, y),
            0x11 => {
                let s = self.b.ins().iadd(x, y);
                self.clampi(s, &mut extra)
            }
            0x12 => self.b.ins().isub(y, x),
            0x13 => {
                let s = self.b.ins().isub(y, x);
                self.clampi(s, &mut extra)
            }
            0x14 => {
                let d = self.b.ins().isub(y, x);
                self.alu_flags_i(d, &[]);
                return None;
            }
            0x15 => self.b.ins().band_imm_u(x, 0x7fff_ffff),
            0x16 => return Some((a.out, x)),
            0x17 => {
                let v = self.clampi(x, &mut extra);
                let st = self.ld(OFF_ST);
                let st = self.b.ins().band_imm_u(st, !ZC as i64);
                let st = self.b.ins().bor(st, extra[0]);
                self.st(OFF_ST, st);
                return Some((a.out, v));
            }
            0x18 => self.b.ins().band(x, y),
            0x19 => self.b.ins().bor(x, y),
            0x1a => self.b.ins().bxor(x, y),
            0x1b => self.b.ins().bnot(x),
            0x1c => self.b.ins().ushr_imm_u(x, a.in2 as i64),
            0x1d | 0x1f => self.b.ins().ishl_imm_u(x, a.in2 as i64),
            0x1e => self.b.ins().sshr_imm_u(x, a.in2.min(31) as i64),
            _ => return None,
        };
        if float && !matches!(a.op, 0x0e | 0x0f) {
            self.alu_flags_f(r, &extra);
        } else {
            self.alu_flags_i(r, &extra);
        }
        Some((a.out, r))
    }

    /// `Tgp::mul`.
    fn mul(&mut self, m: &MulOp) -> (u32, Value) {
        let x = self.in1(false, m.in1);
        let y = self.in2(false, m.in2, m.float);
        let (r, neg, zero) = if m.float {
            let v = self.fmul(x, y);
            let (n, z) = self.fneg_zero(v);
            (v, n, z)
        } else {
            let v = self.b.ins().imul(x, y);
            let n = self.b.ins().icmp_imm_s(IntCC::SignedLessThan, v, 0);
            let z = self.b.ins().icmp_imm_u(IntCC::Equal, v, 0);
            (v, n, z)
        };
        let mn = self.bit_if(neg, MN);
        let mz = self.bit_if(zero, MZ);
        let f = self.b.ins().bor(mn, mz);
        let st = self.ld(OFF_ST);
        let st = self.b.ins().band_imm_u(st, !(MN | MZ | MV | MU | MD) as i64);
        let st = self.b.ins().bor(st, f);
        self.st(OFF_ST, st);
        (m.out, r)
    }
}

// The helpers: the interpreter's own pieces.
//
// SAFETY (all of them): called only by blocks `Mb86235Jit::run` started,
// with the TGP and the `&mut dyn External` it passed, live and distinct.

type HFifoLen = unsafe extern "C" fn(*mut Tgp) -> u32;
type HPop = unsafe extern "C" fn(*mut Tgp) -> u32;
type HPushOut = unsafe extern "C" fn(*mut Tgp, u32);
type HFlags = unsafe extern "C" fn(*mut Tgp) -> u32;
type HExtRead = unsafe extern "C" fn(*mut u8, u32) -> u32;
type HExtWrite = unsafe extern "C" fn(*mut u8, u32, u32);
type HClrF = unsafe extern "C" fn(*mut Tgp, u32);
type HSetPending = unsafe extern "C" fn(*mut Tgp, u32);
type HF2 = extern "C" fn(u32, u32) -> u32;
type HF1 = extern "C" fn(u32) -> u32;

/// Nonzero if the input FIFO has a word.
unsafe extern "C" fn h_fifo_len(t: *mut Tgp) -> u32 {
    !unsafe { &*t }.fifo_in.is_empty() as u32
}

unsafe extern "C" fn h_pop(t: *mut Tgp) -> u32 {
    unsafe { &mut *t }.fifo_in.pop_front().unwrap_or(0)
}

unsafe extern "C" fn h_push_out(t: *mut Tgp, v: u32) {
    unsafe { &mut *t }.fifo_out.push_back(v);
}

unsafe extern "C" fn h_flags(t: *mut Tgp) -> u32 {
    unsafe { &*t }.flags()
}

unsafe extern "C" fn h_ext_read(e: *mut u8, a: u32) -> u32 {
    unsafe { &mut *(e as *mut &mut dyn External) }.read(a)
}

unsafe extern "C" fn h_ext_write(e: *mut u8, a: u32, v: u32) {
    unsafe { &mut *(e as *mut &mut dyn External) }.write(a, v)
}

unsafe extern "C" fn h_clrf(t: *mut Tgp, which: u32) {
    let t = unsafe { &mut *t };
    if which & 1 != 0 {
        t.fifo_in.clear();
    }
    if which & 2 != 0 {
        t.fifo_out.clear();
    }
}

unsafe extern "C" fn h_set_pending(t: *mut Tgp, target: u32) {
    unsafe { &mut *t }.set_pending(Some(target));
}

extern "C" fn h_add(a: u32, b: u32) -> u32 {
    ::mb86235::float::add(a, b)
}

extern "C" fn h_sub(a: u32, b: u32) -> u32 {
    ::mb86235::float::sub(a, b)
}

extern "C" fn h_mul(a: u32, b: u32) -> u32 {
    ::mb86235::float::mul(a, b)
}

extern "C" fn h_rcp(a: u32) -> u32 {
    ::mb86235::float::rcp(a)
}

extern "C" fn h_rsq(a: u32) -> u32 {
    ::mb86235::float::rsq(a)
}

extern "C" fn h_log2(a: u32) -> u32 {
    ::mb86235::float::log2(a)
}

/// The first part of two TGPs' state in which they differ, if any:
/// everything the interpreter keeps, its execution counts included (the
/// lockstep's and the tests' comparison).
pub fn differs(a: &Tgp, b: &Tgp) -> Option<&'static str> {
    let checks: [(&'static str, bool); 16] = [
        ("registers", (a.aa, a.ab, a.ma, a.mb, a.ar) == (b.aa, b.ab, b.ma, b.mb, b.ar)),
        ("the PR queue", (a.pr, a.prp, a.pwp) == (b.pr, b.prp, b.pwp)),
        ("EB, EO, SP", (a.eb, a.eo, a.sp) == (b.eb, b.eo, b.sp)),
        ("the status register", a.st == b.st),
        ("MOD", a.md == b.md),
        ("the loop and repeat counters", (a.lpc, a.rpc) == (b.lpc, b.rpc)),
        ("PDR, DDR", (a.pdr, a.ddr) == (b.pdr, b.ddr)),
        ("pc", a.pc == b.pc),
        ("the return stack", (a.pcs, a.pcsp) == (b.pcs, b.pcsp)),
        ("the delayed branch", a.pending() == b.pending()),
        ("the input FIFO", a.fifo_in == b.fifo_in),
        ("the output FIFO", a.fifo_out == b.fifo_out),
        ("A RAM", a.aram == b.aram),
        ("B RAM", a.bram == b.bram),
        ("the step count", a.steps == b.steps),
        ("the execution counts", a.counts == b.counts),
    ];
    checks.iter().find(|c| !c.1).map(|c| c.0)
}

#[cfg(test)]
mod tests;
