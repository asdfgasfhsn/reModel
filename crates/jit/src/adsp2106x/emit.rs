//! A region's IR: each word lowered as `Sharc::block_exec` runs it, where
//! the lowering has it (`lowered`), else a call of `block_exec` itself
//! (`h_exec`).
//!
//! A lowered word does what `block_exec` does, in its order: the address
//! of each transfer the word makes (with its condition) classified first,
//! leaving before the word for an IOP register or reserved address as
//! `block_exec` declines it; STKY noted (`Ctx::st`); the word's effects;
//! the bus told of an external access (`Bus::after_access`); and leaving
//! after the word if it wrote watched code. Its effects are `exec`'s, in
//! `exec`'s order where order can be seen: registers a transfer stores and
//! a register move copies are read before the compute runs, the compute's
//! results are written before the transfer's load lands (so the load
//! wins), DM after PM. Compute operations the lowering has (the ALU's
//! fixed-point adds and logic, its float add and subtract, the float
//! multiply, the type 6 shifts) are IR, the float ones on
//! `float::single`'s path (two normal inputs, a result in range, RND32)
//! with anything else through the interpreter's own operation (`jit_alu`,
//! `jit_mul`); the rest of the compute field is the interpreter's
//! (`jit_compute`). Memory is the interpreter's `load` and `store`
//! (`jit_read`, `jit_write`), addresses the DAGs' IR.

use super::{Ctx, BAIL, FAULT, LAST};
use crate::copro::{call, sig, trusted, F32, F64, I32, I64, I8};
use ::adsp2106x::compute::{MfRegs, MulOp, ShiftOp};
use ::adsp2106x::cpu::{jit_layout, layout, word_index, Bus, InBlock, JitPipe, Sharc};
use ::adsp2106x::{codegen::in_block, Addr, AluOp, Compute, Count, Insn, Modifier, Op, Target, TRUE};
use cranelift_codegen::ir::{condcodes::IntCC, Block, BlockArg, InstBuilder, JumpTableData, SigRef, Type, Value};
use cranelift_frontend::FunctionBuilder;
use cranelift_frontend::Variable;
use std::mem::offset_of;

// ASTAT, STKY and MODE1 bits (`adsp2106x::cpu`, UM §E.4-E.7).
const AZ: u32 = 1 << 0;
const AV: u32 = 1 << 1;
const AN: u32 = 1 << 2;
const AC: u32 = 1 << 3;
const AS: u32 = 1 << 4;
const AI: u32 = 1 << 5;
const MN: u32 = 1 << 6;
const MV: u32 = 1 << 7;
const MU: u32 = 1 << 8;
const MI: u32 = 1 << 9;
const AF: u32 = 1 << 10;
const SV: u32 = 1 << 11;
const SZ: u32 = 1 << 12;
const SS: u32 = 1 << 13;
const BTF: u32 = 1 << 18;
const AOS: u32 = 1 << 2;
const CB7S: u32 = 1 << 17;
const CB15S: u32 = 1 << 18;
const IRQ_CB7: u32 = 21;
const IRQ_CB15: u32 = 22;
const BR8: u32 = 1 << 0;
const BR0: u32 = 1 << 1;
const ALUSAT: u32 = 1 << 13;
const TRUNC: u32 = 1 << 15;
const RND32: u32 = 1 << 16;
const MASK40: i64 = 0xff_ffff_ffff;

/// MSGR0-7 (IOP 0x08-0x0f): plain storage to the core (`Sharc::iop_get`,
/// `iop_set`), so an access to them can be in a block, through
/// `jit_read`/`jit_write`.
fn msgr(addr: u32) -> bool {
    (0x08..=0x0f).contains(&addr)
}

/// A universal register the lowering reads and writes in place: the
/// register file and the DAG registers.
fn plain_ureg(r: u8) -> bool {
    r < 0x50
}

/// A universal register the lowering moves between registers: those and
/// PX's views (`PX`, `PX1`, `PX2`; their transfers with memory, 48 bits
/// wide, are `block_exec`'s).
fn reg_ureg(r: u8) -> bool {
    plain_ureg(r) || matches!(r, 0xdb..=0xdd)
}

/// ALU operations lowered to IR (the rest go through `jit_alu`).
fn alu_ir(op: AluOp) -> bool {
    use AluOp::*;
    matches!(op, Add | Sub | Inc | Dec | Pass | And | Or | Xor | Not | FAdd | FSub)
}

/// Does `insn`'s lowering have it (else it runs as `h_exec`)? A word that
/// can be in a block (`codegen::in_block`), whose registers are the
/// register file and the DAGs', and whose memory addresses aren't known
/// to be IOP registers.
pub(super) fn lowered(insn: &Insn) -> bool {
    match insn.op {
        Op::Nop | Op::Compute { .. } | Op::Modify { .. } | Op::Dual { .. } | Op::StoreImm { .. } => true,
        Op::ModifyImm { .. } => true,
        Op::Branch { .. } | Op::Return { .. } | Op::JumpElseMove { .. } => capable(insn),
        Op::Do { .. } | Op::Stacks(_) | Op::CJump { .. } => capable(insn),
        Op::Transfer { mv, .. } => plain_ureg(mv.reg),
        Op::ShiftImm { mv, .. } => mv.is_none_or(|m| plain_ureg(m.reg)),
        Op::RegMove { src, dst, .. } => reg_ureg(src) && reg_ureg(dst),
        Op::LoadImm { reg, .. } => reg_ureg(reg),
        Op::IndirectImm { reg, .. } => plain_ureg(reg),
        Op::Direct { reg, addr, .. } => plain_ureg(reg) && (addr >= 0x2_0000 || msgr(addr)),
        _ => false,
    }
}

/// Can `insn` be in a pipeline block (`SharcJit`'s regions)? The words a
/// block of the interpreter's holds (`codegen::in_block`), and the
/// sequencer's that the lowering runs with the pipeline: jumps, calls and
/// returns (not RTI, nor a jump that ends an interrupt), DOs (not a
/// one-word loop, whose pipeline is rebuilt), the loop and PC stack
/// operations, CJUMP. Not a jump to itself on a FLAG input with nothing
/// else to do, where the board stops stepping the core (`spinning`).
pub(super) fn capable(insn: &Insn) -> bool {
    match insn.op {
        Op::Branch { cond, call, target, delayed, loop_abort, clear_int, compute, .. } => {
            let spin = matches!(cond, 9..=12 | 25..=28)
                && !call
                && !delayed
                && !loop_abort
                && compute == Compute::None
                && target.resolve(insn.addr) == Some(insn.addr);
            !clear_int && !spin && !matches!(compute, Compute::Unknown(_))
        }
        Op::Return { rti, compute, .. } => !rti && !matches!(compute, Compute::Unknown(_)),
        Op::JumpElseMove { compute, mv, .. } => !matches!(compute, Compute::Unknown(_)) && plain_ureg(mv.reg),
        Op::Do { count, end, .. } => {
            end != insn.addr.wrapping_add(1)
                && match count {
                    Some(Count::Ureg(r)) => plain_ureg(r),
                    _ => true,
                }
        }
        Op::Stacks(st) => !st.push_sts && !st.pop_sts,
        Op::CJump { .. } => true,
        // BIT on the user status registers and ASTAT (its FLAG outputs
        // are pins nothing reads within a run), any BIT TST or XOR (which
        // only set BTF): `block_exec` runs them.
        Op::Bit { op, sreg, .. } => {
            matches!(sreg, 0x70 | 0x71 | 0x7c) || matches!(op, ::adsp2106x::BitOp::Tst | ::adsp2106x::BitOp::Xor)
        }
        _ => in_block(insn),
    }
}

/// Is it one of the sequencer's words, which only the lowering runs (not
/// `block_exec`)?
pub(super) fn sequencer(insn: &Insn) -> bool {
    matches!(
        insn.op,
        Op::Branch { .. }
            | Op::Return { .. }
            | Op::JumpElseMove { .. }
            | Op::Do { .. }
            | Op::Stacks(_)
            | Op::CJump { .. }
    )
}

struct Sigs {
    exec: SigRef,
    compute: SigRef,
    op2: SigRef,
    shift: SigRef,
    read: SigRef,
    write: SigRef,
    access: SigRef,
    cond: SigRef,
    fetch: SigRef,
    room: SigRef,
    pipe: SigRef,
    pipe1: SigRef,
    do_: SigRef,
    pop: SigRef,
}

/// The pipeline and the block's counts, as the IR keeps them.
struct Vars {
    x: Variable,
    xf: Variable,
    xs: Variable,
    d: Variable,
    df: Variable,
    ds: Variable,
    f: Variable,
    hold: Variable,
    /// Instructions the block may still run (it leaves at 0), the last
    /// instruction's address. (Bubbles count their cycles in
    /// `Ctx::cycles`; the loop end and short-loop flag stay in
    /// `Ctx::pipe`, where the helpers keep them.)
    left: Variable,
    pc: Variable,
    /// This word's branch: 0 none, 1 delayed, 2 not; and where to.
    rk: Variable,
    rt: Variable,
}

/// What a region is translated for, which no word of a block changes:
/// MODE1 (its rounding, saturation and bit-reverse modes) and SYSCON's
/// IMDW bits (which internal memory block holds 48-bit data). A region
/// runs only while the core's are these (`SharcJit::one`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode {
    pub mode1: u32,
    pub imdw: u32,
}

impl Mode {
    pub fn of(s: &Sharc) -> Mode {
        Mode { mode1: s.mode1, imdw: s.iop[::adsp2106x::cpu::SYSCON as usize] & (3 << 8) }
    }
}

/// The IR of a region.
pub(super) struct Emit<'a, 'b> {
    b: &'a mut FunctionBuilder<'b>,
    ptr: Type,
    /// The core (`*mut Sharc`).
    s: Value,
    ctx: Value,
    /// The exit: everything kept in `v` written back to `Ctx`.
    exit: Block,
    sigs: Sigs,
    v: Vars,
    /// The block that runs the word in the execute slot (or a bubble).
    dispatch: Block,
    /// The exit from plain code: its parameter the word in the execute
    /// slot of the plain sequence.
    exit_plain: Block,
    /// The word being emitted for plain code (else for the variables).
    plain_at: Option<u32>,
    /// Where each word copy notes STKY at the start of its cycle
    /// (`note_st`): blocks filled in once the region is emitted, with the
    /// note if the word can set an exception sticky bit (`changes_stky`),
    /// else nothing; by word copy.
    st_sites: Vec<(Block, Block, usize, u32)>,
    changes_stky: Vec<bool>,
    /// The modes the region is translated for.
    mode: Mode,
    /// The word being emitted, and whether it can change ASTAT and the
    /// FLAG inputs.
    pc_now: u32,
    word_status: (bool, bool),
    status_sites: Vec<(Block, Block, (bool, bool))>,
}

impl<'a, 'b> Emit<'a, 'b> {
    pub fn new(b: &'a mut FunctionBuilder<'b>, ptr: Type, mode: Mode) -> Self {
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let p = b.block_params(entry).to_vec();
        let exit = b.create_block();
        let dispatch = b.create_block();
        let var = |b: &mut FunctionBuilder, t: Type| b.declare_var(t);
        let v = Vars {
            x: var(b, I32),
            xf: var(b, I32),
            xs: var(b, I32),
            d: var(b, I32),
            df: var(b, I32),
            ds: var(b, I32),
            f: var(b, I32),
            hold: var(b, I32),
            left: var(b, I64),
            pc: var(b, I32),
            rk: var(b, I32),
            rt: var(b, I32),
        };
        let sigs = Sigs {
            exec: sig(b, &[ptr, ptr, ptr, I64], Some(I32)),
            compute: sig(b, &[ptr, ptr], None),
            op2: sig(b, &[ptr, ptr, I64, I64], Some(I64)),
            shift: sig(b, &[ptr, ptr, I64, I64, I32], Some(I64)),
            read: sig(b, &[ptr, ptr, I32, I32], Some(I64)),
            write: sig(b, &[ptr, ptr, I32, I32, I64], None),
            access: sig(b, &[ptr, ptr, I64], None),
            cond: sig(b, &[ptr, I32], Some(I32)),
            fetch: sig(b, &[ptr, I32, I32, I32, ptr], Some(I64)),
            room: sig(b, &[ptr], Some(I32)),
            pipe: sig(b, &[ptr, ptr], None),
            pipe1: sig(b, &[ptr, I32, ptr], None),
            do_: sig(b, &[ptr, I32, I32, I64, ptr], None),
            pop: sig(b, &[ptr], Some(I32)),
        };
        let exit_plain = b.create_block();
        b.append_block_param(exit_plain, I32);
        Emit {
            b,
            ptr,
            s: p[0],
            ctx: p[2],
            exit,
            sigs,
            v,
            dispatch,
            exit_plain,
            plain_at: None,
            st_sites: Vec::new(),
            changes_stky: Vec::new(),
            pc_now: 0,
            word_status: (false, false),
            status_sites: Vec::new(),
            mode,
        }
    }

    /// The core (`*mut Sharc`).
    fn s(&mut self) -> Value {
        self.s
    }

    fn k(&mut self, v: u32) -> Value {
        self.b.ins().iconst(I32, v as i64)
    }

    fn k64(&mut self, v: u64) -> Value {
        self.b.ins().iconst(I64, v as i64)
    }

    fn p(&mut self, v: usize) -> Value {
        self.b.ins().iconst(self.ptr, v as i64)
    }

    /// Leave before the word (the pipeline as it stands: the word is
    /// still in the execute slot, for the interpreter's single step).
    fn bail(&mut self) {
        match self.plain_at {
            Some(a) => {
                let a = self.k(a);
                self.b.ins().jump(self.exit_plain, &[BlockArg::Value(a)]);
            }
            None => {
                self.b.ins().jump(self.exit, &[]);
            }
        }
    }

    fn use_(&mut self, v: Variable) -> Value {
        self.b.use_var(v)
    }

    fn def(&mut self, v: Variable, x: Value) {
        self.b.def_var(v, x);
    }

    fn def_k(&mut self, v: Variable, x: u32) {
        let k = self.k(x);
        self.b.def_var(v, k);
    }

    fn pipe_ptr(&mut self) -> Value {
        self.b.ins().iadd_imm_s(self.ctx, offset_of!(Ctx, pipe) as i64)
    }

    /// The innermost loop's end (`JitPipe::loop_end`), as the helpers
    /// keep it in `Ctx::pipe`.
    fn loop_end(&mut self) -> Value {
        self.b.ins().load(I32, trusted(), self.ctx, (offset_of!(Ctx, pipe) + offset_of!(JitPipe, loop_end)) as i32)
    }

    /// MODE1, which no word of a block changes: as the region was
    /// translated for (`Mode`).
    fn mode1(&mut self) -> Value {
        self.k(self.mode.mode1)
    }

    /// One more cycle without an instruction (a bubble, or the cycle a
    /// fault began), in `Ctx::cycles`.
    fn extra_cycle(&mut self) {
        let off = offset_of!(Ctx, cycles) as i32;
        let c = self.b.ins().load(I64, trusted(), self.ctx, off);
        let c = self.b.ins().iadd_imm_s(c, 1);
        self.b.ins().store(trusted(), c, self.ctx, off);
    }

    /// Leave after this word if `go` (I32, 0 or 1): what is left of the
    /// budget becomes 1, the rest noted in `Ctx::cut`.
    fn stop_if(&mut self, go: Value) {
        let left = self.use_(self.v.left);
        let off = offset_of!(Ctx, cut) as i32;
        let cut = self.b.ins().load(I64, trusted(), self.ctx, off);
        let lost = self.b.ins().iadd_imm_s(left, -1);
        let g = self.b.ins().uextend(I64, go);
        let lost = self.b.ins().imul(lost, g);
        let cut = self.b.ins().iadd(cut, lost);
        self.b.ins().store(trusted(), cut, self.ctx, off);
        let left = self.b.ins().isub(left, lost);
        self.def(self.v.left, left);
    }

    fn ld32(&mut self, off: usize) -> Value {
        let s_ = self.s();
        self.b.ins().load(I32, trusted(), s_, off as i32)
    }

    fn st32(&mut self, off: usize, v: Value) {
        if off == layout::ASTAT {
            self.word_status.0 = true;
        }
        let s_ = self.s();
        self.b.ins().store(trusted(), v, s_, off as i32);
    }

    fn rf(&mut self, n: u8) -> Value {
        let s_ = self.s();
        self.b.ins().load(I64, trusted(), s_, (layout::R + 8 * (n as usize & 15)) as i32)
    }

    fn set_rf(&mut self, n: u8, v: Value) {
        let v = self.b.ins().band_imm_u(v, MASK40);
        let s_ = self.s();
        self.b.ins().store(trusted(), v, s_, (layout::R + 8 * (n as usize & 15)) as i32);
    }

    /// `(v >> 8) as u32`: a 40-bit value's fixed-point field.
    fn field(&mut self, v40: Value) -> Value {
        let v = self.b.ins().ushr_imm_u(v40, 8);
        self.b.ins().ireduce(I32, v)
    }

    /// `(v as u64) << 8`.
    fn widen(&mut self, v32: Value) -> Value {
        let v = self.b.ins().uextend(I64, v32);
        self.b.ins().ishl_imm_u(v, 8)
    }

    fn helper(&mut self, s: SigRef, f: usize, args: &[Value]) -> Option<Value> {
        // The interpreter's operations may set exception sticky bits.
        if [h_compute as HCompute as usize, h_alu as HOp2 as usize, h_mul as HOp2 as usize, h_shift as HShift as usize]
            .contains(&f)
        {
            self.sets_stky();
            self.word_status.0 = true;
        }
        // A word on `block_exec` can do anything; the bus may change the
        // FLAG inputs after an access.
        if f == h_exec as HExec as usize {
            self.word_status = (true, true);
        }
        if f == h_after_access as HAccess as usize {
            self.word_status.1 = true;
        }
        call(self.b, self.ptr, s, f, args)
    }

    /// Branch on `c` (an I8 or I32): `yes` when it holds, else on.
    fn if_then(&mut self, c: Value) -> (Block, Block) {
        let (yes, on) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(c, yes, &[], on, &[]);
        (yes, on)
    }

    // ---- the region

    /// The region `words`, run as a pipeline block: from the word in the
    /// execute slot, each cycle as `Sharc::step` runs it (the word, the
    /// fetch stage with the loop logic, the interrupt hold-off), on to the
    /// next word in the execute slot while it is in the region; bubbles
    /// (after a branch that isn't delayed) as `step` runs them. It leaves
    /// before a word it can't run now, after the budget's last word, after
    /// a word that wrote watched code, and before running on into a short
    /// conditional loop.
    ///
    /// Each word but the sequencer's is plain code, for the pipeline as
    /// the plain sequence (the word in the execute slot, the next two
    /// after it, no slot looped back, no hold-off), whose fetch stage is
    /// only the test that its fetch (two words on) isn't the innermost
    /// loop's end. The sequencer's words, and the words in `general` (a
    /// branch's delay slots, a loop's last two), are also code for the
    /// pipeline in variables, as it is after a branch or a loop's end.
    pub fn region(&mut self, words: &[Insn], general: &[u32]) {
        let start = words[0].addr;
        let has_general = |i: &Insn| sequencer(i) || general.contains(&i.addr);
        let pblocks: Vec<_> = words.iter().map(|i| (!sequencer(i)).then(|| self.b.create_block())).collect();
        let gblocks: Vec<_> = words.iter().map(|i| has_general(i).then(|| self.b.create_block())).collect();
        // Entry: the pipeline from `Ctx::pipe`.
        let pf = |f: usize| (offset_of!(Ctx, pipe) + f) as i32;
        for (var, off) in [
            (self.v.x, offset_of!(JitPipe, x)),
            (self.v.xf, offset_of!(JitPipe, x_flag)),
            (self.v.xs, offset_of!(JitPipe, x_some)),
            (self.v.d, offset_of!(JitPipe, d)),
            (self.v.df, offset_of!(JitPipe, d_flag)),
            (self.v.ds, offset_of!(JitPipe, d_some)),
            (self.v.f, offset_of!(JitPipe, f)),
            (self.v.hold, offset_of!(JitPipe, hold)),
        ] {
            let x = self.b.ins().load(I32, trusted(), self.ctx, pf(off));
            self.def(var, x);
        }
        let budget = self.b.ins().load(I64, trusted(), self.ctx, offset_of!(Ctx, budget) as i32);
        self.def(self.v.left, budget);
        let x = self.use_(self.v.x);
        self.def(self.v.pc, x);
        self.def_k(self.v.rk, 0);
        self.def_k(self.v.rt, 0);
        self.b.ins().jump(self.dispatch, &[]);

        // Dispatch (the pipeline in its variables): a bubble, or the word
        // in the execute slot, as plain code if the pipeline is plain.
        self.b.switch_to_block(self.dispatch);
        let xs = self.use_(self.v.xs);
        let (word, bubble) = self.if_then(xs);
        self.b.switch_to_block(bubble);
        self.note_no_insn();
        let (fa, ff) = self.fetch_status();
        self.extra_cycle();
        self.fetch_stage(fa, ff, false);
        self.b.ins().jump(self.dispatch, &[]);
        self.b.switch_to_block(word);
        let short = self.b.ins().load(
            I32,
            trusted(),
            self.ctx,
            (offset_of!(Ctx, pipe) + offset_of!(JitPipe, short_loop)) as i32,
        );
        let (out, go) = self.if_then(short);
        self.b.switch_to_block(out);
        self.b.ins().jump(self.exit, &[]);
        self.b.switch_to_block(go);
        let x = self.use_(self.v.x);
        let plain = self.plain_now(x);
        let idx = self.b.ins().iadd_imm_s(x, -(start as i64));
        let (pt, gt) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(plain, pt, &[], gt, &[]);
        let outside = self.b.create_block();
        for (blk, table) in [(pt, &pblocks), (gt, &gblocks)] {
            self.b.switch_to_block(blk);
            // A plain word without plain code (the sequencer's) runs on
            // the variables; a word without code for them, leaves.
            let calls: Vec<_> = (0..words.len())
                .map(|j| {
                    let t = table[j].or(if blk == pt { gblocks[j] } else { None }).unwrap_or(outside);
                    self.b.func.dfg.block_call(t, &[])
                })
                .collect();
            let def = self.b.func.dfg.block_call(outside, &[]);
            let jt = self.b.create_jump_table(JumpTableData::new(def, &calls));
            self.b.ins().br_table(idx, jt);
        }
        self.b.switch_to_block(outside);
        self.set_kind(super::OUTSIDE);
        self.b.ins().jump(self.exit, &[]);

        for (j, i) in words.iter().enumerate() {
            let a = i.addr;
            let seq = sequencer(i);
            // The fetch stage etc. on the variables, for both copies.
            let gtail = self.b.create_block();
            let gfa = self.b.append_block_param(gtail, I32);
            let gff = self.b.append_block_param(gtail, I32);
            for plain in [true, false] {
                let Some(blk) = (if plain { pblocks[j] } else { gblocks[j] }) else { continue };
                self.b.switch_to_block(blk);
                self.plain_at = plain.then_some(a);
                let site = self.status_site();
                if seq {
                    self.def_k(self.v.rk, 0);
                }
                self.changes_stky.push(false);
                self.pc_now = a;
                self.word_status = (false, false);
                if lowered(i) {
                    self.word(i);
                } else {
                    self.interpreted(i);
                }
                self.plain_at = None;
                // The cycle's end: counted.
                self.count(a);
                let r = self.use_(self.v.left);
                let r = self.b.ins().iadd_imm_s(r, -1);
                self.def(self.v.left, r);
                self.def_k(self.v.pc, a);
                if plain {
                    // The fetch of a + 2: the plain sequence goes on unless
                    // it is the innermost loop's end.
                    let le = self.loop_end();
                    let at_end = self.b.ins().icmp_imm_u(IntCC::Equal, le, a as i64 + 2);
                    let (to_general, on) = self.if_then(at_end);
                    self.b.switch_to_block(to_general);
                    self.b.set_cold_block(to_general);
                    // As the fetch stage finds it: the decode slot a + 1,
                    // the fetch address a + 2.
                    self.def_k(self.v.d, a + 1);
                    self.def_k(self.v.df, 0);
                    self.def_k(self.v.ds, 1);
                    self.def_k(self.v.f, a + 2);
                    self.def_k(self.v.hold, 0);
                    let (fa, ff) = self.status_after(site);
                    self.b.ins().jump(gtail, &[BlockArg::Value(fa), BlockArg::Value(ff)]);
                    self.b.switch_to_block(on);
                    let next = self.k(a + 1);
                    let left = self.use_(self.v.left);
                    let out = self.b.ins().icmp_imm_u(IntCC::Equal, left, 0);
                    let (go, beyond) = match pblocks.get(j + 1).copied().flatten() {
                        Some(n) => (n, None),
                        None => {
                            let leave = self.b.create_block();
                            (leave, Some(leave))
                        }
                    };
                    let (leave, on) = self.if_then(out);
                    self.b.switch_to_block(leave);
                    self.set_kind(super::DONE);
                    self.b.ins().jump(self.exit_plain, &[BlockArg::Value(next)]);
                    self.b.switch_to_block(on);
                    self.b.ins().jump(go, &[]);
                    if let Some(leave) = beyond {
                        // Past the region's plain code: leave, to go on in
                        // the region that has the next word.
                        self.b.switch_to_block(leave);
                        self.set_kind(super::OUTSIDE);
                        self.b.ins().jump(self.exit_plain, &[BlockArg::Value(next)]);
                    }
                } else {
                    let (fa, ff) = self.status_after(site);
                    self.b.ins().jump(gtail, &[BlockArg::Value(fa), BlockArg::Value(ff)]);
                }
            }
            self.b.switch_to_block(gtail);
            self.fetch_stage(gfa, gff, seq);
            let h = self.use_(self.v.hold);
            let h1 = self.b.ins().iadd_imm_s(h, -1);
            let z = self.k(0);
            let h = self.b.ins().select(h, h1, z);
            self.def(self.v.hold, h);
            let left = self.use_(self.v.left);
            let out = self.b.ins().icmp_imm_u(IntCC::Equal, left, 0);
            let (leave, on) = self.if_then(out);
            self.b.switch_to_block(leave);
            self.set_kind(super::DONE);
            self.b.ins().jump(self.exit, &[]);
            self.b.switch_to_block(on);
            self.b.ins().jump(self.dispatch, &[]);
        }

        // The exit from plain code: the pipeline from the word in its
        // execute slot.
        self.b.switch_to_block(self.exit_plain);
        let at = self.b.block_params(self.exit_plain)[0];
        let at1 = self.b.ins().iadd_imm_s(at, 1);
        let at2 = self.b.ins().iadd_imm_s(at, 2);
        self.def(self.v.x, at);
        self.def(self.v.d, at1);
        self.def(self.v.f, at2);
        for v in [self.v.xf, self.v.df, self.v.hold] {
            self.def_k(v, 0);
        }
        self.def_k(self.v.xs, 1);
        self.def_k(self.v.ds, 1);
        self.b.ins().jump(self.exit, &[]);

        self.fill_st();
        self.fill_status();

        // The exit: the pipeline and counts back to `Ctx`.
        self.b.switch_to_block(self.exit);
        for (var, off) in [
            (self.v.x, offset_of!(JitPipe, x)),
            (self.v.xf, offset_of!(JitPipe, x_flag)),
            (self.v.xs, offset_of!(JitPipe, x_some)),
            (self.v.d, offset_of!(JitPipe, d)),
            (self.v.df, offset_of!(JitPipe, d_flag)),
            (self.v.ds, offset_of!(JitPipe, d_some)),
            (self.v.f, offset_of!(JitPipe, f)),
            (self.v.hold, offset_of!(JitPipe, hold)),
        ] {
            let x = self.use_(var);
            self.b.ins().store(trusted(), x, self.ctx, pf(off));
        }
        // Instructions run: the budget less what is left and what was cut;
        // as many cycles more.
        let ran = self.ran();
        self.b.ins().store(trusted(), ran, self.ctx, offset_of!(Ctx, ran) as i32);
        let off = offset_of!(Ctx, cycles) as i32;
        let c = self.b.ins().load(I64, trusted(), self.ctx, off);
        let c = self.b.ins().iadd(c, ran);
        self.b.ins().store(trusted(), c, self.ctx, off);
        let pc = self.use_(self.v.pc);
        self.b.ins().store(trusted(), pc, self.ctx, offset_of!(Ctx, pc) as i32);
        self.b.ins().return_(&[]);
    }

    /// How the block ended (`Ctx::kind`).
    fn set_kind(&mut self, k: u32) {
        let k = self.k(k);
        self.b.ins().store(trusted(), k, self.ctx, offset_of!(Ctx, kind) as i32);
    }

    /// Is the pipeline, with `x` in the execute slot, the plain sequence?
    fn plain_now(&mut self, x: Value) -> Value {
        let d = self.use_(self.v.d);
        let f = self.use_(self.v.f);
        let x1 = self.b.ins().iadd_imm_s(x, 1);
        let x2 = self.b.ins().iadd_imm_s(x, 2);
        let a = self.b.ins().icmp(IntCC::Equal, d, x1);
        let b = self.b.ins().icmp(IntCC::Equal, f, x2);
        let ab = self.b.ins().band(a, b);
        let mut ok = self.b.ins().uextend(I32, ab);
        let ds = self.use_(self.v.ds);
        ok = self.b.ins().band(ok, ds);
        for v in [self.v.xf, self.v.df, self.v.hold] {
            let x = self.use_(v);
            let z = self.b.ins().icmp_imm_u(IntCC::Equal, x, 0);
            let z = self.b.ins().uextend(I32, z);
            ok = self.b.ins().band(ok, z);
        }
        ok
    }

    /// Where a word notes ASTAT and the FLAG inputs at the start of its
    /// cycle, for the loop test of its fetch stage (`fetch_status`): a
    /// block filled in by `fill_status` with what the word can change
    /// (`word_status`); what it can't is read after it (`status_after`).
    fn status_site(&mut self) -> usize {
        let (fl, on) = (self.b.create_block(), self.b.create_block());
        self.b.ins().jump(fl, &[]);
        self.b.switch_to_block(on);
        self.status_sites.push((fl, on, (false, false)));
        self.status_sites.len() - 1
    }

    /// ASTAT and the FLAG inputs as the word at `site` began, from after
    /// the word.
    fn status_after(&mut self, site: usize) -> (Value, Value) {
        let (astat, flag_in) = self.word_status;
        let st = &mut self.status_sites[site].2;
        st.0 |= astat;
        st.1 |= flag_in;
        let a = if astat {
            self.b.ins().load(I32, trusted(), self.ctx, offset_of!(Ctx, fa) as i32)
        } else {
            self.ld32(layout::ASTAT)
        };
        let f = if flag_in {
            self.b.ins().load(I32, trusted(), self.ctx, offset_of!(Ctx, ff) as i32)
        } else {
            let s_ = self.s();
            self.b.ins().uload8(I32, trusted(), s_, jit_layout::FLAG_IN as i32)
        };
        (a, f)
    }

    fn fill_status(&mut self) {
        for (fl, on, (astat, flag_in)) in std::mem::take(&mut self.status_sites) {
            self.b.switch_to_block(fl);
            if astat || flag_in {
                let (a, f) = self.fetch_status();
                if astat {
                    self.b.ins().store(trusted(), a, self.ctx, offset_of!(Ctx, fa) as i32);
                }
                if flag_in {
                    self.b.ins().store(trusted(), f, self.ctx, offset_of!(Ctx, ff) as i32);
                }
            }
            self.b.ins().jump(on, &[]);
        }
    }

    /// ASTAT and the FLAG inputs at the start of the cycle (the loop test
    /// of its fetch sees them: `fetch_status`).
    fn fetch_status(&mut self) -> (Value, Value) {
        let a = self.ld32(layout::ASTAT);
        let s_ = self.s();
        let f = self.b.ins().uload8(I32, trusted(), s_, jit_layout::FLAG_IN as i32);
        (a, f)
    }

    /// `Sharc::fetch_stage`: the fetch of the fetch address, through the
    /// loop logic when it is the innermost loop's end (`jit_fetch`), the
    /// slots moved on, and this word's branch (`redirect`) taken.
    fn fetch_stage(&mut self, fa: Value, ff: Value, redirect: bool) {
        let fetched = self.use_(self.v.f);
        let le = self.loop_end();
        let at_end = self.b.ins().icmp(IntCC::Equal, fetched, le);
        let join = self.b.create_block();
        let succ = self.b.append_block_param(join, I32);
        let looped = self.b.append_block_param(join, I32);
        let (lp, plain) = self.if_then(at_end);
        self.b.switch_to_block(plain);
        let next = self.b.ins().iadd_imm_s(fetched, 1);
        let z = self.k(0);
        self.b.ins().jump(join, &[BlockArg::Value(next), BlockArg::Value(z)]);
        self.b.switch_to_block(lp);
        self.b.set_cold_block(lp);
        let pp = self.pipe_ptr();
        let (s, sg) = (self.s(), self.sigs.fetch);
        let r = self.helper(sg, h_fetch as HFetch as usize, &[s, fetched, fa, ff, pp]).expect("returns");
        let n = self.b.ins().ireduce(I32, r);
        let l = self.b.ins().ushr_imm_u(r, 32);
        let l = self.b.ins().ireduce(I32, l);
        self.b.ins().jump(join, &[BlockArg::Value(n), BlockArg::Value(l)]);
        self.b.switch_to_block(join);
        for (to, from) in [(self.v.x, self.v.d), (self.v.xf, self.v.df), (self.v.xs, self.v.ds)] {
            let v = self.use_(from);
            self.def(to, v);
        }
        self.def(self.v.d, fetched);
        self.def(self.v.df, looped);
        self.def_k(self.v.ds, 1);
        self.def(self.v.f, succ);
        if redirect {
            let rk = self.use_(self.v.rk);
            let (taken, on) = self.if_then(rk);
            self.b.switch_to_block(taken);
            let rt = self.use_(self.v.rt);
            let rt = self.b.ins().band_imm_u(rt, 0xff_ffff);
            self.def(self.v.f, rt);
            // Not delayed: the two words fetched become NOPs.
            let nd = self.b.ins().icmp_imm_u(IntCC::Equal, rk, 2);
            let (sq, on2) = self.if_then(nd);
            self.b.switch_to_block(sq);
            self.def_k(self.v.xs, 0);
            self.def_k(self.v.ds, 0);
            self.b.ins().jump(on2, &[]);
            self.b.switch_to_block(on2);
            self.b.ins().jump(on, &[]);
            self.b.switch_to_block(on);
        }
    }

    /// `block_exec` itself, for a word the lowering doesn't have.
    fn interpreted(&mut self, i: &Insn) {
        let insn = self.p(i as *const Insn as usize);
        let steps = self.steps_after();
        let (s, ctx, sg) = (self.s(), self.ctx, self.sigs.exec);
        // `block_exec` notes STKY itself (unless it declines the word).
        let st_pc = offset_of!(Ctx, st_pc) as i32;
        let before = self.b.ins().load(I32, trusted(), self.ctx, st_pc);
        let r = self.helper(sg, h_exec as HExec as usize, &[s, ctx, insn, steps]).expect("returns");
        let declined = self.b.ins().icmp_imm_u(IntCC::Equal, r, BAIL as i64);
        let here = self.k(i.addr);
        let now = self.b.ins().select(declined, before, here);
        self.b.ins().store(trusted(), now, self.ctx, st_pc);
        let (stopped, on) = self.if_then(r);
        self.b.switch_to_block(stopped);
        // LAST: it ran, and the block ends after it. FAULT: the cycle
        // begun and the word counted, as `step` leaves a fault.
        let last = self.b.ins().icmp_imm_u(IntCC::Equal, r, LAST as i64);
        let (ran, other) = self.if_then(last);
        self.b.switch_to_block(ran);
        let one = self.k(1);
        self.stop_if(one);
        self.b.ins().jump(on, &[]);
        self.b.switch_to_block(other);
        let fault = self.b.ins().icmp_imm_u(IntCC::Equal, r, FAULT as i64);
        let (f, bail) = self.if_then(fault);
        self.b.switch_to_block(bail);
        self.bail();
        self.b.switch_to_block(f);
        self.count(i.addr);
        self.extra_cycle();
        self.def_k(self.v.pc, i.addr);
        if let Some(a) = self.plain_at {
            self.def_k(self.v.d, a + 1);
            self.def_k(self.v.f, a + 2);
            for v in [self.v.xf, self.v.df, self.v.hold, self.v.x] {
                self.def_k(v, 0);
            }
            self.def_k(self.v.ds, 1);
        }
        self.def_k(self.v.xs, 0);
        self.b.ins().jump(self.exit, &[]);
        self.b.switch_to_block(on);
    }

    /// Instructions run so far.
    fn ran(&mut self) -> Value {
        let budget = self.b.ins().load(I64, trusted(), self.ctx, offset_of!(Ctx, budget) as i32);
        let cut = self.b.ins().load(I64, trusted(), self.ctx, offset_of!(Ctx, cut) as i32);
        let left = self.use_(self.v.left);
        let r = self.b.ins().isub(budget, cut);
        self.b.ins().isub(r, left)
    }

    /// The step count once this word has run.
    fn steps_after(&mut self) -> Value {
        let base = self.b.ins().load(I64, trusted(), self.ctx, offset_of!(Ctx, base) as i32);
        let ran = self.ran();
        let s = self.b.ins().iadd(base, ran);
        self.b.ins().iadd_imm_s(s, 1)
    }

    /// The word at `a` counted (`Sharc::counts`).
    fn count(&mut self, a: u32) {
        let k = word_index(a).expect("region words are internal");
        let counts = self.b.ins().load(self.ptr, trusted(), self.ctx, offset_of!(Ctx, counts) as i32);
        let n = self.b.ins().load(I64, trusted(), counts, 8 * k as i32);
        let n = self.b.ins().iadd_imm_s(n, 1);
        self.b.ins().store(trusted(), n, counts, 8 * k as i32);
    }

    /// `*st = self.stky`, as `block_exec` notes it before the word runs:
    /// STKY at the start of the last cycle, for the interrupt latch at
    /// the end (`Sharc::jit_exit`), which only looks at its exception
    /// bits. Only a word that can set one needs it (`Ctx::st_pc`): this
    /// is a place for the note, filled in by `fill_st` once the word is
    /// known to be one.
    fn note_st(&mut self) {
        let (fl, on) = (self.b.create_block(), self.b.create_block());
        self.b.ins().jump(fl, &[]);
        let word = self.changes_stky.len() - 1;
        self.st_sites.push((fl, on, word, self.pc_now));
        self.b.switch_to_block(on);
    }

    /// This word can set an exception sticky bit.
    fn sets_stky(&mut self) {
        *self.changes_stky.last_mut().expect("in a word") = true;
    }

    /// A cycle without an instruction (a bubble): STKY as it ends it.
    fn note_no_insn(&mut self) {
        let m = self.k(u32::MAX);
        self.b.ins().store(trusted(), m, self.ctx, offset_of!(Ctx, st_pc) as i32);
    }

    /// The notes of STKY, now that the words that need them are known.
    fn fill_st(&mut self) {
        for (fl, on, word, pc) in std::mem::take(&mut self.st_sites) {
            self.b.switch_to_block(fl);
            if self.changes_stky[word] {
                let st = self.ld32(layout::STKY);
                self.b.ins().store(trusted(), st, self.ctx, offset_of!(Ctx, st) as i32);
                let pc = self.k(pc);
                self.b.ins().store(trusted(), pc, self.ctx, offset_of!(Ctx, st_pc) as i32);
            }
            self.b.ins().jump(on, &[]);
        }
    }

    /// A lowered word. Leaves the builder in the block that goes on to
    /// the next word.
    fn word(&mut self, i: &Insn) {
        let a = i.addr;
        match &i.op {
            Op::Nop => self.note_st(),
            Op::Compute { cond, compute } => {
                self.note_st();
                let on = self.when(*cond);
                self.compute(compute);
                self.join(on);
            }
            Op::Modify { cond, compute, i: di, m } => {
                self.note_st();
                let on = self.when(*cond);
                self.compute(compute);
                let mv = self.m_value(*m);
                self.modify(*di, mv);
                self.join(on);
            }
            Op::RegMove { cond, compute, src, dst } => {
                self.note_st();
                let on = self.when(*cond);
                let v = self.ureg_read(*src);
                self.compute(compute);
                self.ureg_write(*dst, v);
                self.join(on);
            }
            Op::LoadImm { reg, data } => {
                self.note_st();
                let v = self.k64((*data as u64) << 8);
                self.ureg_write(*reg, v);
            }
            Op::ModifyImm { i: di, data, bitrev: false } => {
                self.note_st();
                let m = self.b.ins().iconst(I64, *data as i64);
                self.modify(*di, m);
            }
            Op::ModifyImm { i: di, data, bitrev: true } => {
                // BITREV(I, data): the sum, bit-reversed in the DAG's width.
                self.note_st();
                let off = layout::I + 4 * *di as usize;
                let iv = self.ld32(off);
                let v = self.b.ins().iadd_imm_s(iv, *data as i64);
                let v = self.b.ins().bitrev(v);
                let v = if *di >= 8 {
                    let v = self.b.ins().ushr_imm_u(v, 8);
                    self.b.ins().band_imm_u(v, 0xff_ffff)
                } else {
                    v
                };
                self.st32(off, v);
            }
            Op::Branch { cond, call, target, delayed, loop_abort, compute, else_, .. } => {
                let on = self.when_else(*cond, |e| {
                    if *call {
                        e.room(0xff);
                    }
                    let t = e.target(*target, a);
                    if !*else_ {
                        e.note_st();
                        e.compute(compute);
                    } else {
                        e.note_st();
                    }
                    if *call {
                        // Delayed: the third word after; else the word in
                        // the decode slot.
                        let ret = if *delayed {
                            e.k(a.wrapping_add(3))
                        } else {
                            let ds = e.use_(e.v.ds);
                            let d = e.use_(e.v.d);
                            let n = e.k(a + 1);
                            e.b.ins().select(ds, d, n)
                        };
                        let (s, sg) = (e.s(), e.sigs.pipe1);
                        let pp = e.pipe_ptr();
                        e.helper(sg, h_push_pc as HPipe1 as usize, &[s, ret, pp]);
                    } else if *loop_abort {
                        let (s, sg) = (e.s(), e.sigs.pipe);
                        let pp = e.pipe_ptr();
                        e.helper(sg, h_abort as HPipe as usize, &[s, pp]);
                    }
                    e.branch(t, *delayed);
                });
                self.otherwise(on, |e| {
                    e.note_st();
                    if *else_ {
                        e.compute(compute);
                    }
                });
            }
            Op::Return { cond, delayed, loop_reentry, compute, else_, .. } => {
                let on = self.when_else(*cond, |e| {
                    e.room(0x00ff_0000);
                    e.note_st();
                    if !*else_ {
                        e.compute(compute);
                    }
                    let (s, sg) = (e.s(), e.sigs.pop);
                    let t = e.helper(sg, h_pop_pc as HPop as usize, &[s]).expect("returns");
                    if *loop_reentry {
                        let (s, sg) = (e.s(), e.sigs.room);
                        e.helper(sg, h_reentry as HRoom as usize, &[s]);
                    }
                    e.branch(t, *delayed);
                });
                self.otherwise(on, |e| {
                    e.note_st();
                    if *else_ {
                        e.compute(compute);
                    }
                });
            }
            Op::JumpElseMove { cond, target, compute, mv } => {
                let on = self.when_else(*cond, |e| {
                    let t = e.target(*target, a);
                    e.note_st();
                    e.branch(t, false);
                });
                self.otherwise(on, |e| e.transfer(a, compute, mv.addr, mv.write, mv.reg));
            }
            Op::Do { count, term, end } => {
                self.room(0xff | 0xff00);
                self.note_st();
                let (has, n) = match count {
                    Some(Count::Imm(n)) => (1, self.k(*n as u32)),
                    Some(Count::Ureg(r)) => {
                        let v = self.ureg_read(*r);
                        (1, self.field(v))
                    }
                    None => (0, self.k(0)),
                };
                // count | has << 32 | term << 40.
                let n = self.b.ins().uextend(I64, n);
                let k = ((has as u64) << 32 | (*term as u64) << 40) as i64;
                let packed = self.b.ins().bor_imm_u(n, k);
                let (s, sg) = (self.s(), self.sigs.do_);
                let (pc, e) = (self.k(a), self.k(*end));
                let pp = self.pipe_ptr();
                self.helper(sg, h_do as HDo as usize, &[s, pc, e, packed, pp]);
            }
            Op::Stacks(st) => {
                let need = (st.push_pcstk as u32 * 0xff) | (st.push_loop as u32 * 0xff00);
                if need != 0 {
                    self.room(need);
                }
                self.note_st();
                let bits = st.push_loop as u32
                    | (st.pop_loop as u32) << 1
                    | (st.push_pcstk as u32) << 2
                    | (st.pop_pcstk as u32) << 3;
                let (s, sg) = (self.s(), self.sigs.pipe1);
                let k = self.k(bits);
                let pp = self.pipe_ptr();
                self.helper(sg, h_stacks as HPipe1 as usize, &[s, k, pp]);
            }
            Op::CJump { target } => {
                let t = self.target(*target, a);
                self.note_st();
                let (i6, i7) = (self.ureg_read(0x16), self.ureg_read(0x17));
                self.ureg_write(0x02, i6);
                self.ureg_write(0x16, i7);
                self.branch(t, true);
            }
            Op::Transfer { cond, compute, mv } => {
                // Not run, it neither accesses memory nor declines.
                let on = if *cond == TRUE {
                    None
                } else {
                    let c = self.cond(*cond);
                    let (go, skip) = self.if_then(c);
                    let on = self.b.create_block();
                    self.b.switch_to_block(skip);
                    self.note_st();
                    self.b.ins().jump(on, &[]);
                    self.b.switch_to_block(go);
                    Some(on)
                };
                self.transfer(a, compute, mv.addr, mv.write, mv.reg);
                self.join(on);
            }
            Op::ShiftImm { cond, op, rn, rx, imm, mv } => {
                let on = if *cond == TRUE {
                    None
                } else {
                    let c = self.cond(*cond);
                    let (go, skip) = self.if_then(c);
                    let on = self.b.create_block();
                    self.b.switch_to_block(skip);
                    self.note_st();
                    self.b.ins().jump(on, &[]);
                    self.b.switch_to_block(go);
                    Some(on)
                };
                match mv {
                    None => {
                        self.note_st();
                        self.shift_imm(op, *rn, *rx, *imm);
                    }
                    Some(mv) => {
                        let (addr, ext) = self.peek_checked(a, &mv.addr);
                        self.note_st();
                        let value = mv.write.then(|| self.ureg_read(mv.reg));
                        self.shift_imm(op, *rn, *rx, *imm);
                        self.access(a, mv.addr, addr, ext, mv.write, mv.reg, value);
                    }
                }
                self.join(on);
            }
            Op::Dual { compute, dm, pm } => {
                let (da, dext) = self.peek_checked(a, &dm.addr);
                let (pa, pext) = self.peek_checked(a, &pm.addr);
                self.note_st();
                let (dv, pv) = (self.rf(dm.reg), self.rf(pm.reg));
                self.compute(compute);
                let (dm_mod, pm_mod) = (self.modifier(&dm.addr), self.modifier(&pm.addr));
                self.modify(dm.addr.i, dm_mod);
                self.modify(pm.addr.i, pm_mod);
                let dl = self.mem(false, da, dm.write, dv);
                let pl = self.mem(true, pa, pm.write, pv);
                if let Some(v) = pl {
                    self.set_rf(pm.reg, v);
                }
                if let Some(v) = dl {
                    self.set_rf(dm.reg, v);
                }
                let ext = self.b.ins().bor(dext, pext);
                self.after(a, ext, dm.write || pm.write);
            }
            Op::Direct { pm, write, reg, addr } => {
                self.note_st();
                let value = write.then(|| self.ureg_read(*reg));
                if msgr(*addr) {
                    // A message register: the IOP register itself.
                    let iop = self.b.ins().load(self.ptr, trusted(), self.ctx, offset_of!(Ctx, iop) as i32);
                    let off = 4 * *addr as i32;
                    match value {
                        Some(v) => {
                            let v = self.field(v);
                            self.b.ins().store(trusted(), v, iop, off);
                        }
                        None => {
                            let v = self.b.ins().load(I32, trusted(), iop, off);
                            let v = self.widen(v);
                            self.ureg_write(*reg, v);
                        }
                    }
                    return;
                }
                let at = self.k(*addr);
                let ext = self.b.ins().iconst(I8, (*addr >= 0x8_0000) as i64);
                let l = self.mem(*pm, at, *write, value.unwrap_or(at));
                if let Some(v) = l {
                    self.ureg_write(*reg, v);
                }
                self.after(a, ext, *write);
            }
            Op::IndirectImm { write, reg, i: di, offset } => {
                let addr = Addr { i: *di, modifier: Modifier::Imm(*offset), post: false };
                let (at, ext) = self.peek_checked(a, &addr);
                self.note_st();
                let value = write.then(|| self.ureg_read(*reg));
                let l = self.mem(*di >= 8, at, *write, value.unwrap_or(at));
                if let Some(v) = l {
                    self.ureg_write(*reg, v);
                }
                self.after(a, ext, *write);
            }
            Op::StoreImm { addr, data } => {
                let (at, ext) = self.peek_checked(a, addr);
                self.note_st();
                let m = self.modifier(addr);
                self.modify(addr.i, m);
                let v = self.k64((*data as u64) << 8);
                self.mem(addr.pm(), at, true, v);
                self.after(a, ext, true);
            }
            _ => unreachable!("not lowered: {i}"),
        }
    }

    /// Leave before the word unless the stacks have room: `mask` picks the
    /// bytes of `jit_stack_room` that must be nonzero (the PC stack's and
    /// the loop stack's room, the PC stack's and loop stack's depth).
    fn room(&mut self, mask: u32) {
        let (s, sg) = (self.s(), self.sigs.room);
        let r = self.helper(sg, h_room as HRoom as usize, &[s]).expect("returns");
        for byte in 0..4 {
            if mask >> (8 * byte) & 0xff != 0 {
                let b = self.b.ins().ushr_imm_u(r, 8 * byte as i64);
                let b = self.b.ins().band_imm_u(b, 0xff);
                let none = self.b.ins().icmp_imm_u(IntCC::Equal, b, 0);
                let (out, on) = self.if_then(none);
                self.b.switch_to_block(out);
                self.bail();
                self.b.switch_to_block(on);
            }
        }
    }

    /// A branch to `t` (I32) as `Sharc::branch` requests it: the fetch
    /// stage takes it, and interrupts wait for it (and its delay slots).
    fn branch(&mut self, t: Value, delayed: bool) {
        self.def(self.v.rt, t);
        self.def_k(self.v.rk, if delayed { 1 } else { 2 });
        self.def_k(self.v.hold, if delayed { 2 } else { 1 });
    }

    /// `Sharc::target` for a branch at `pc`.
    fn target(&mut self, t: Target, pc: u32) -> Value {
        match t {
            Target::Abs(a) => self.k(a),
            Target::Rel(d) => self.k(pc.wrapping_add(d as u32) & 0xff_ffff),
            Target::Indirect { i, m } => {
                let iv = self.ld32(layout::I + 4 * i as usize);
                let mv = self.m_value(m);
                let mv = self.b.ins().ireduce(I32, mv);
                let v = self.b.ins().iadd(iv, mv);
                let v = self.b.ins().band_imm_u(v, 0xff_ffff);
                if i == 8 {
                    let mode1 = self.mode1();
                    let br = self.b.ins().band_imm_u(mode1, BR8 as i64);
                    let rev = self.b.ins().bitrev(v);
                    let rev = self.b.ins().ushr_imm_u(rev, 8);
                    self.b.ins().select(br, rev, v)
                } else {
                    v
                }
            }
        }
    }

    /// The taken side of a conditional word: `then` runs if `cond` holds;
    /// the block for the other side, for [`otherwise`](Emit::otherwise).
    fn when_else(&mut self, cond: u8, then: impl FnOnce(&mut Self)) -> (Option<Block>, Block) {
        let join = self.b.create_block();
        if cond == TRUE {
            then(self);
            self.b.ins().jump(join, &[]);
            return (None, join);
        }
        let c = self.cond(cond);
        let (yes, no) = self.if_then(c);
        self.b.switch_to_block(yes);
        then(self);
        self.b.ins().jump(join, &[]);
        (Some(no), join)
    }

    fn otherwise(&mut self, (no, join): (Option<Block>, Block), els: impl FnOnce(&mut Self)) {
        if let Some(no) = no {
            self.b.switch_to_block(no);
            els(self);
            self.b.ins().jump(join, &[]);
        }
        self.b.switch_to_block(join);
    }

    /// Under condition `cond` (unless TRUE): the block that goes on after,
    /// for [`join`](Emit::join).
    fn when(&mut self, cond: u8) -> Option<Block> {
        if cond == TRUE {
            return None;
        }
        let c = self.cond(cond);
        let (yes, on) = self.if_then(c);
        self.b.switch_to_block(yes);
        Some(on)
    }

    fn join(&mut self, on: Option<Block>) {
        if let Some(on) = on {
            self.b.ins().jump(on, &[]);
            self.b.switch_to_block(on);
        }
    }

    /// A transfer with a compute (types 3, 4, 10's else): its address
    /// checked, STKY noted, the stored value read, the compute, the DAG's
    /// post-modify, the access and the load.
    fn transfer(&mut self, a: u32, compute: &Compute, addr: Addr, write: bool, reg: u8) {
        let (at, ext) = self.peek_checked(a, &addr);
        self.note_st();
        let value = write.then(|| self.ureg_read(reg));
        self.compute(compute);
        self.access(a, addr, at, ext, write, reg, value);
    }

    /// The DAG's post-modify, the access, the load, and what follows an
    /// access (`after`).
    #[allow(clippy::too_many_arguments)]
    fn access(&mut self, a: u32, addr: Addr, at: Value, ext: Value, write: bool, reg: u8, value: Option<Value>) {
        if addr.post {
            let m = self.modifier(&addr);
            self.modify(addr.i, m);
        }
        let l = self.mem(addr.pm(), at, write, value.unwrap_or(at));
        if let Some(v) = l {
            self.ureg_write(reg, v);
        }
        self.after(a, ext, write);
    }

    /// After an access: the bus told if it was external, and the block
    /// left after the word if a write reached watched code.
    fn after(&mut self, a: u32, ext: Value, wrote: bool) {
        let (tell, on) = self.if_then(ext);
        self.b.switch_to_block(tell);
        let _ = a;
        let steps = self.steps_after();
        let (s, e, sg) = (self.s(), self.ctx, self.sigs.access);
        self.helper(sg, h_after_access as HAccess as usize, &[s, e, steps]);
        self.b.ins().jump(on, &[]);
        self.b.switch_to_block(on);
        if wrote {
            let s_ = self.s();
            let d = self.b.ins().load(I8, trusted(), s_, layout::CODE_DIRTY as i32);
            let d = self.b.ins().uextend(I32, d);
            self.stop_if(d);
        }
    }

    /// A read (`None` for a write of `value`) at `at` through the PM bus
    /// (`pm`) or the DM bus: the register's new value.
    fn mem(&mut self, pm: bool, at: Value, write: bool, value: Value) -> Option<Value> {
        let pm = self.k(pm as u32);
        if write {
            self.write(pm, at, value);
            None
        } else {
            Some(self.read(pm, at))
        }
    }

    /// A write of the 40-bit `v`: a 32-bit word of internal memory (normal
    /// addresses, IMDW clear) inline, as `Internal::write32` stores it, and
    /// `invalidate` called only if the words it touches have anything
    /// cached (`Sharc::jit_cached`); anything else through `jit_write`.
    fn write(&mut self, pm: Value, at: Value, v: Value) {
        let join = self.b.create_block();
        let off = self.b.ins().iadd_imm_s(at, -0x2_0000);
        let normal = self.b.ins().icmp_imm_u(IntCC::UnsignedLessThan, off, 0x2_0000);
        let (inl, slow) = self.if_then(normal);
        self.b.switch_to_block(inl);
        let block1 = self.b.ins().icmp_imm_u(IntCC::UnsignedGreaterThanOrEqual, off, 0x8000);
        let b1 = self.b.ins().uextend(I32, block1);
        // IMDW (SYSCON, as the region was translated for): 48-bit data
        // in that block goes the slow way.
        if self.mode.imdw != 0 {
            let syscon = self.k(self.mode.imdw);
            let sh = self.b.ins().iadd_imm_s(b1, 8);
            let imdw = self.b.ins().ushr(syscon, sh);
            let imdw = self.b.ins().band_imm_u(imdw, 1);
            let (wide, plain) = self.if_then(imdw);
            self.b.switch_to_block(wide);
            self.b.ins().jump(slow, &[]);
            self.b.switch_to_block(plain);
        }
        let m0 = self.b.ins().load(self.ptr, trusted(), self.ctx, offset_of!(Ctx, mem) as i32);
        let m1 = self.b.ins().load(self.ptr, trusted(), self.ctx, offset_of!(Ctx, mem) as i32 + 8);
        let col = self.b.ins().select(block1, m1, m0);
        let o = self.b.ins().band_imm_u(off, 0x7fff);
        let g = self.b.ins().ushr_imm_u(o, 12);
        let r = self.b.ins().band_imm_u(o, 0xfff);
        let at2 = self.b.ins().ishl_imm_u(g, 13);
        let at2 = self.b.ins().bor(at2, r);
        let at2 = self.b.ins().ishl_imm_u(at2, 1);
        let at2 = self.b.ins().uextend(self.ptr, at2);
        let p = self.b.ins().iadd(col, at2);
        let v32 = self.field(v);
        let hi = self.b.ins().ushr_imm_u(v32, 16);
        self.b.ins().istore16(trusted(), hi, p, 0);
        self.b.ins().istore16(trusted(), v32, p, 2 * 4096);
        // The 48-bit words of columns 2g and 2g+1: groups 2g/3 and
        // (2g+1)/3 (none past 4), row r.
        let g2 = self.b.ins().ishl_imm_u(g, 1);
        let ga = self.b.ins().imul_imm_u(g2, 11);
        let ga = self.b.ins().ushr_imm_u(ga, 5);
        let g21 = self.b.ins().iadd_imm_s(g2, 1);
        let gb = self.b.ins().imul_imm_u(g21, 11);
        let gb = self.b.ins().ushr_imm_u(gb, 5);
        let gb_ok = self.b.ins().icmp_imm_u(IntCC::UnsignedLessThan, gb, 5);
        let gb = self.b.ins().select(gb_ok, gb, ga);
        let base = self.b.ins().imul_imm_u(b1, ::adsp2106x::mem::WORDS48 as i64);
        let base = self.b.ins().iadd(base, r);
        let cached = self.b.ins().load(self.ptr, trusted(), self.ctx, offset_of!(Ctx, cached) as i32);
        let mut any = None;
        for gg in [ga, gb] {
            let k = self.b.ins().ishl_imm_u(gg, 12);
            let k = self.b.ins().iadd(k, base);
            let k = self.b.ins().uextend(self.ptr, k);
            let p = self.b.ins().iadd(cached, k);
            let c = self.b.ins().uload8(I32, trusted(), p, 0);
            any = Some(match any {
                None => c,
                Some(a) => self.b.ins().bor(a, c),
            });
        }
        let (inv, on) = self.if_then(any.expect("two"));
        self.b.switch_to_block(inv);
        self.b.set_cold_block(inv);
        let (s, sg) = (self.s(), self.sigs.cond);
        self.helper(sg, h_invalidate as HCond as usize, &[s, at]);
        self.b.ins().jump(on, &[]);
        self.b.switch_to_block(on);
        self.b.ins().jump(join, &[]);
        self.b.switch_to_block(slow);
        let (s, e, sg) = (self.s(), self.ctx, self.sigs.write);
        self.helper(sg, h_write as HWrite as usize, &[s, e, pm, at, v]);
        self.b.ins().jump(join, &[]);
        self.b.switch_to_block(join);
    }

    /// A read: a 32-bit word of internal memory (normal addresses, IMDW
    /// clear) inline, as `Internal::read32` finds it; anything else
    /// through `jit_read`.
    fn read(&mut self, pm: Value, at: Value) -> Value {
        let join = self.b.create_block();
        let out = self.b.append_block_param(join, I64);
        // 0x20000-0x3ffff: block 1 from 0x28000 (and its aliases).
        let off = self.b.ins().iadd_imm_s(at, -0x2_0000);
        let normal = self.b.ins().icmp_imm_u(IntCC::UnsignedLessThan, off, 0x2_0000);
        let (inl, slow) = self.if_then(normal);
        self.b.switch_to_block(inl);
        let block1 = self.b.ins().icmp_imm_u(IntCC::UnsignedGreaterThanOrEqual, off, 0x8000);
        let b1 = self.b.ins().uextend(I32, block1);
        // IMDW (SYSCON, as the region was translated for): 48-bit data
        // in that block goes the slow way.
        if self.mode.imdw != 0 {
            let syscon = self.k(self.mode.imdw);
            let sh = self.b.ins().iadd_imm_s(b1, 8);
            let imdw = self.b.ins().ushr(syscon, sh);
            let imdw = self.b.ins().band_imm_u(imdw, 1);
            let (wide, plain) = self.if_then(imdw);
            self.b.switch_to_block(wide);
            self.b.ins().jump(slow, &[]);
            self.b.switch_to_block(plain);
        }
        let m0 = self.b.ins().load(self.ptr, trusted(), self.ctx, offset_of!(Ctx, mem) as i32);
        let m1 = self.b.ins().load(self.ptr, trusted(), self.ctx, offset_of!(Ctx, mem) as i32 + 8);
        let col = self.b.ins().select(block1, m1, m0);
        // Columns 2g and 2g+1 (g: bits 14-12), row: bits 11-0.
        let o = self.b.ins().band_imm_u(off, 0x7fff);
        let g = self.b.ins().ushr_imm_u(o, 12);
        let r = self.b.ins().band_imm_u(o, 0xfff);
        let hi = self.b.ins().ishl_imm_u(g, 13);
        let hi = self.b.ins().bor(hi, r);
        let hi = self.b.ins().ishl_imm_u(hi, 1);
        let hi = self.b.ins().uextend(self.ptr, hi);
        let p = self.b.ins().iadd(col, hi);
        let h = self.b.ins().uload16(I64, trusted(), p, 0);
        let l = self.b.ins().uload16(I64, trusted(), p, 2 * 4096);
        let h = self.b.ins().ishl_imm_u(h, 16);
        let v = self.b.ins().bor(h, l);
        let v = self.b.ins().ishl_imm_u(v, 8);
        self.b.ins().jump(join, &[BlockArg::Value(v)]);
        self.b.switch_to_block(slow);
        let (s, e, sg) = (self.s(), self.ctx, self.sigs.read);
        let v = self.helper(sg, h_read as HRead as usize, &[s, e, pm, at]).expect("returns");
        self.b.ins().jump(join, &[BlockArg::Value(v)]);
        self.b.switch_to_block(join);
        out
    }

    // ---- universal registers

    fn ureg_read(&mut self, r: u8) -> Value {
        let n = (r & 15) as usize;
        match r >> 4 {
            0 => self.rf(r),
            g @ 1..=4 => {
                let off = [layout::I, layout::M, layout::L, layout::B][g as usize - 1] + 4 * n;
                let v = self.ld32(off);
                let v = if g == 2 && n >= 8 {
                    let v = self.b.ins().ishl_imm_u(v, 8);
                    self.b.ins().sshr_imm_u(v, 8)
                } else {
                    v
                };
                self.widen(v)
            }
            _ => {
                let s_ = self.s();
                let px = self.b.ins().load(I64, trusted(), s_, offset_of!(Sharc, px) as i32);
                match r {
                    // PX: bits 47-8 as a 40-bit value.
                    0xdb => self.b.ins().ushr_imm_u(px, 8),
                    0xdc => {
                        let v = self.b.ins().band_imm_u(px, 0xffff);
                        self.b.ins().ishl_imm_u(v, 8)
                    }
                    0xdd => {
                        let v = self.b.ins().ushr_imm_u(px, 16);
                        let v = self.b.ins().band_imm_u(v, 0xffff_ffff);
                        self.b.ins().ishl_imm_u(v, 8)
                    }
                    _ => unreachable!("not a register the lowering moves"),
                }
            }
        }
    }

    fn ureg_write(&mut self, r: u8, v: Value) {
        let n = (r & 15) as usize;
        match r >> 4 {
            0 => self.set_rf(r, v),
            g @ 1..=4 => {
                let v = self.field(v);
                let v = if n >= 8 { self.b.ins().band_imm_u(v, 0xff_ffff) } else { v };
                let off = [layout::I, layout::M, layout::L, layout::B][g as usize - 1] + 4 * n;
                self.st32(off, v);
                if g == 4 {
                    // Loading B also loads I.
                    self.st32(layout::I + 4 * n, v);
                }
            }
            _ => {
                let off = offset_of!(Sharc, px) as i32;
                let s_ = self.s();
                let px = self.b.ins().load(I64, trusted(), s_, off);
                let new = match r {
                    0xdb => {
                        let v = self.b.ins().band_imm_u(v, MASK40);
                        self.b.ins().ishl_imm_u(v, 8)
                    }
                    0xdc => {
                        let v32 = self.b.ins().ushr_imm_u(v, 8);
                        let lo = self.b.ins().band_imm_u(v32, 0xffff);
                        let hi = self.b.ins().band_imm_u(px, !0xffff_i64);
                        self.b.ins().bor(hi, lo)
                    }
                    0xdd => {
                        let v32 = self.b.ins().ushr_imm_u(v, 8);
                        let v32 = self.b.ins().band_imm_u(v32, 0xffff_ffff);
                        let hi = self.b.ins().ishl_imm_u(v32, 16);
                        let lo = self.b.ins().band_imm_u(px, 0xffff);
                        self.b.ins().bor(hi, lo)
                    }
                    _ => unreachable!("not a register the lowering moves"),
                };
                let s_ = self.s();
                self.b.ins().store(trusted(), new, s_, off);
            }
        }
    }

    // ---- DAGs

    /// An M register as a signed modifier (I64).
    fn m_value(&mut self, m: u8) -> Value {
        let v = self.ld32(layout::M + 4 * m as usize);
        let v = if m >= 8 {
            let v = self.b.ins().ishl_imm_u(v, 8);
            self.b.ins().sshr_imm_u(v, 8)
        } else {
            v
        };
        self.b.ins().sextend(I64, v)
    }

    fn modifier(&mut self, a: &Addr) -> Value {
        match a.modifier {
            Modifier::M(m) => self.m_value(m),
            Modifier::Imm(v) => self.b.ins().iconst(I64, v as i64),
        }
    }

    /// The address the DAG outputs for `a` (`Sharc::dag_peek`).
    fn peek(&mut self, a: &Addr) -> Value {
        let i = a.i as usize;
        let dag2 = i >= 8;
        let iv = self.ld32(layout::I + 4 * i);
        let out = if a.post {
            iv
        } else {
            let m = self.modifier(a);
            let m = self.b.ins().ireduce(I32, m);
            let v = self.b.ins().iadd(iv, m);
            if dag2 {
                self.b.ins().band_imm_u(v, 0xff_ffff)
            } else {
                v
            }
        };
        if i == 0 || i == 8 {
            // Bit-reversed in BR0 / BR8 mode.
            let mode1 = self.mode1();
            let br = self.b.ins().band_imm_u(mode1, if dag2 { BR8 } else { BR0 } as i64);
            let rev = self.b.ins().bitrev(out);
            let rev = if dag2 { self.b.ins().ushr_imm_u(rev, 8) } else { rev };
            self.b.ins().select(br, rev, out)
        } else {
            out
        }
    }

    /// [`peek`](Emit::peek), leaving before the word (as `block_exec`
    /// declines it) for an IOP register (but the message registers) or a
    /// reserved address; and whether it is on the external port (I8).
    fn peek_checked(&mut self, at: u32, a: &Addr) -> (Value, Value) {
        let addr = self.peek(a);
        let low = self.b.ins().icmp_imm_u(IntCC::UnsignedLessThan, addr, 0x2_0000);
        let m = self.b.ins().band_imm_u(addr, !7);
        let not_msgr = self.b.ins().icmp_imm_u(IntCC::NotEqual, m, 8);
        let low = self.b.ins().band(low, not_msgr);
        let (bail, on) = self.if_then(low);
        self.b.switch_to_block(bail);
        let _ = at;
        self.bail();
        self.b.switch_to_block(on);
        let ext = self.b.ins().icmp_imm_u(IntCC::UnsignedGreaterThanOrEqual, addr, 0x8_0000);
        (addr, ext)
    }

    /// `Sharc::modify`: I`i` post-modified by `m` (I64), circular when
    /// L`i` is nonzero; I7 and I15 flag wrapping.
    fn modify(&mut self, i: u8, m: Value) {
        let i = i as usize;
        let dag2 = i >= 8;
        let mask: i64 = if dag2 { 0xff_ffff } else { 0xffff_ffff };
        let iv = self.ld32(layout::I + 4 * i);
        let iv = self.b.ins().uextend(I64, iv);
        let l = self.ld32(layout::L + 4 * i);
        let sum = self.b.ins().iadd(iv, m);
        let raw = self.b.ins().band_imm_u(sum, mask);
        let done = self.b.create_block();
        if i != 7 && i != 15 {
            // Not circular: just the sum.
            let (circ, plain) = self.if_then(l);
            self.b.switch_to_block(plain);
            let r = self.b.ins().ireduce(I32, raw);
            self.st32(layout::I + 4 * i, r);
            self.b.ins().jump(done, &[]);
            self.b.switch_to_block(circ);
        }
        let l = self.b.ins().uextend(I64, l);
        let b = self.ld32(layout::B + 4 * i);
        let b = self.b.ins().uextend(I64, b);
        let up = self.b.ins().icmp_imm_s(IntCC::SignedGreaterThanOrEqual, m, 0);
        let end = self.b.ins().iadd(b, l);
        let over = self.b.ins().icmp(IntCC::SignedGreaterThanOrEqual, sum, end);
        let under = self.b.ins().icmp(IntCC::SignedLessThan, sum, b);
        let wraps = self.b.ins().select(up, over, under);
        let down = self.b.ins().isub(raw, l);
        let down = self.b.ins().band_imm_u(down, mask);
        let back = self.b.ins().iadd(raw, l);
        let back = self.b.ins().band_imm_u(back, mask);
        let wrapped = self.b.ins().select(up, down, back);
        let circ = self.b.ins().icmp_imm_u(IntCC::NotEqual, l, 0);
        let take = self.b.ins().band(circ, wraps);
        let new = self.b.ins().select(take, wrapped, raw);
        let new = self.b.ins().ireduce(I32, new);
        self.st32(layout::I + 4 * i, new);
        if i == 7 || i == 15 {
            let (wrap, on) = self.if_then(wraps);
            self.b.switch_to_block(wrap);
            let (sticky, irq) = if i == 7 { (CB7S, IRQ_CB7) } else { (CB15S, IRQ_CB15) };
            let st = self.ld32(layout::STKY);
            let st = self.b.ins().bor_imm_u(st, sticky as i64);
            self.st32(layout::STKY, st);
            // `latch`: unless that interrupt is being serviced.
            let p = self.ld32(layout::IMASKP);
            let busy = self.b.ins().band_imm_u(p, 1i64 << irq);
            let l = self.ld32(layout::IRPTL);
            let set = self.b.ins().bor_imm_u(l, 1i64 << irq);
            let l = self.b.ins().select(busy, l, set);
            self.st32(layout::IRPTL, l);
            self.b.ins().jump(on, &[]);
            self.b.switch_to_block(on);
        }
        self.b.ins().jump(done, &[]);
        self.b.switch_to_block(done);
    }

    // ---- conditions

    fn astat(&mut self) -> Value {
        self.ld32(layout::ASTAT)
    }

    /// `bit` of ASTAT, as an I8.
    fn flag(&mut self, a: Value, bit: u32) -> Value {
        let v = self.b.ins().band_imm_u(a, bit as i64);
        self.b.ins().icmp_imm_u(IntCC::NotEqual, v, 0)
    }

    /// Condition code `c` of an `IF` (`Sharc::cond`), as an I8: ASTAT's
    /// arithmetic flags in IR, the rest (FLAG inputs, BM, the loop
    /// counter) through `Sharc::cond`.
    fn cond(&mut self, c: u8) -> Value {
        let c = c & 31;
        if c == TRUE {
            return self.b.ins().iconst(I8, 1);
        }
        let base = c & 15;
        let v = match (c, base) {
            (_, 9..=12) => {
                // FLAGn_IN as `Sharc::astat` shows it: the pin for an
                // input (MODE2 FLGnO clear), ASTAT's bit for an output.
                let n = (base - 9) as i64;
                let mode2 = self.ld32(jit_layout::MODE2);
                let out = self.b.ins().ushr_imm_u(mode2, 15 + n);
                let out = self.b.ins().band_imm_u(out, 1);
                let a = self.astat();
                let a = self.b.ins().ushr_imm_u(a, 19 + n);
                let a = self.b.ins().band_imm_u(a, 1);
                let s_ = self.s();
                let f = self.b.ins().uload8(I32, trusted(), s_, jit_layout::FLAG_IN as i32);
                let f = self.b.ins().ushr_imm_u(f, n);
                let f = self.b.ins().band_imm_u(f, 1);
                let v = self.b.ins().select(out, a, f);
                self.b.ins().icmp_imm_u(IntCC::NotEqual, v, 0)
            }
            (15, _) | (31, _) | (_, 14) => {
                let (s, sg) = (self.s(), self.sigs.cond);
                let k = self.k(c as u32);
                let r = self.helper(sg, h_cond as HCond as usize, &[s, k]).expect("returns");
                return self.b.ins().icmp_imm_u(IntCC::NotEqual, r, 0);
            }
            (_, 1) | (_, 2) => {
                let a = self.astat();
                let af = self.flag(a, AF);
                let an = self.flag(a, AN);
                let av = self.flag(a, AV);
                let az = self.flag(a, AZ);
                let mode1 = self.mode1();
                let sat = self.b.ins().band_imm_u(mode1, ALUSAT as i64);
                let nosat = self.b.ins().icmp_imm_u(IntCC::Equal, sat, 0);
                let ov = self.b.ins().band(av, nosat);
                let x = self.b.ins().bxor(an, ov);
                let one = self.b.ins().iconst(I8, 1);
                let naf = self.b.ins().bxor(af, one);
                let fixed = self.b.ins().band(naf, x);
                let fneg = self.b.ins().band(af, an);
                if base == 1 {
                    let naz = self.b.ins().bxor(az, one);
                    let f = self.b.ins().band(fneg, naz);
                    self.b.ins().bor(fixed, f)
                } else {
                    let f = self.b.ins().bor(fneg, az);
                    self.b.ins().bor(fixed, f)
                }
            }
            (_, b) => {
                let bit = match b {
                    0 => AZ,
                    3 => AC,
                    4 => AV,
                    5 => MV,
                    6 => MN,
                    7 => SV,
                    8 => SZ,
                    _ => BTF,
                };
                let a = self.astat();
                self.flag(a, bit)
            }
        };
        if c >= 16 {
            self.b.ins().bxor_imm_u(v, 1)
        } else {
            v
        }
    }

    // ---- compute

    /// A compute field: IR for what the lowering has, else the
    /// interpreter's `jit_compute` (which writes its results at once).
    fn compute(&mut self, c: &Compute) {
        match c {
            Compute::None => {}
            Compute::Alu { op, rn, rx, ry } => {
                let (x, y) = (self.rf(*rx), self.rf(*ry));
                if let Some(v) = self.alu(op, x, y) {
                    self.set_rf(*rn, v);
                }
            }
            Compute::Mul { op: op @ MulOp::FMul, rn, rx, ry } => {
                let (x, y) = (self.rf(*rx), self.rf(*ry));
                if let Some(v) = self.mul(op, x, y) {
                    self.set_rf(*rn, v);
                }
            }
            Compute::Multi { mul: op @ MulOp::FMul, alu, rm, ra, regs: MfRegs { xm, ym, xa, ya } } => {
                let (xm, ym, xa, ya) = (self.rf(*xm), self.rf(*ym), self.rf(*xa), self.rf(*ya));
                let m = self.mul(op, xm, ym);
                let a = self.alu(alu, xa, ya);
                if let Some(v) = m {
                    self.set_rf(*rm, v);
                }
                if let Some(v) = a {
                    self.set_rf(*ra, v);
                }
            }
            c => {
                let (s, sg) = (self.s(), self.sigs.compute);
                let p = self.p(c as *const Compute as usize);
                self.helper(sg, h_compute as HCompute as usize, &[s, p]);
            }
        }
    }

    /// ASTAT's ALU flags replaced by `bits` (`set_alu_flags`).
    fn set_alu_flags(&mut self, bits: Value) {
        let a = self.astat();
        let a = self.b.ins().band_imm_u(a, !(AZ | AV | AN | AC | AS | AI | AF) as i64);
        let a = self.b.ins().bor(a, bits);
        self.st32(layout::ASTAT, a);
    }

    /// `bit` if `c` (an I8) holds, else 0 (I32).
    fn bit_if(&mut self, c: Value, bit: u32) -> Value {
        let c = self.b.ins().uextend(I32, c);
        self.b.ins().imul_imm_u(c, bit as i64)
    }

    /// One ALU operation on 40-bit values (`Sharc::alu` with its flags):
    /// the result, if it has one.
    fn alu(&mut self, op: &AluOp, x40: Value, y40: Value) -> Option<Value> {
        use AluOp::*;
        match op {
            Comp => {
                self.comp(x40, y40);
                return None;
            }
            FComp => {
                self.fcomp(op, x40, y40);
                return None;
            }
            FAbs | FNeg | FPass => return Some(self.fsign(op, x40)),
            _ => {}
        }
        if !alu_ir(*op) {
            let (s, sg) = (self.s(), self.sigs.op2);
            let p = self.p(op as *const AluOp as usize);
            let v = self.helper(sg, h_alu as HOp2 as usize, &[s, p, x40, y40]).expect("returns");
            return (!matches!(op, Comp | FComp)).then_some(v);
        }
        if matches!(op, FAdd | FSub) {
            return Some(self.fadd(op, x40, y40));
        }
        let (x, y) = (self.field(x40), self.field(y40));
        let (out, flags) = match op {
            Add | Sub | Inc | Dec => {
                let (b, c) = match op {
                    Add => (y, 0),
                    Sub => (self.b.ins().bnot(y), 1),
                    Inc => (self.k(1), 0),
                    _ => (self.k(u32::MAX), 0),
                };
                let (x64, b64) = (self.b.ins().uextend(I64, x), self.b.ins().uextend(I64, b));
                let s = self.b.ins().iadd(x64, b64);
                let s = self.b.ins().iadd_imm_s(s, c);
                let r = self.b.ins().ireduce(I32, s);
                let carry = self.b.ins().ushr_imm_u(s, 32);
                let carry = self.b.ins().ireduce(I8, carry);
                let (xr, br) = (self.b.ins().bxor(x, r), self.b.ins().bxor(b, r));
                let v = self.b.ins().band(xr, br);
                let v = self.b.ins().icmp_imm_s(IntCC::SignedLessThan, v, 0);
                // ALUSAT: overflow saturates.
                let mode1 = self.mode1();
                let sat = self.b.ins().band_imm_u(mode1, ALUSAT as i64);
                let sat = self.b.ins().icmp_imm_u(IntCC::NotEqual, sat, 0);
                let clip = self.b.ins().band(sat, v);
                let neg = self.b.ins().icmp_imm_s(IntCC::SignedLessThan, r, 0);
                let (hi, lo) = (self.k(0x7fff_ffff), self.k(0x8000_0000));
                let sv = self.b.ins().select(neg, hi, lo);
                let out = self.b.ins().select(clip, sv, r);
                let av = self.bit_if(v, AV);
                let ac = self.bit_if(carry, AC);
                let f = self.b.ins().bor(av, ac);
                (out, Some(f))
            }
            Pass => (x, None),
            And => (self.b.ins().band(x, y), None),
            Or => (self.b.ins().bor(x, y), None),
            Xor => (self.b.ins().bxor(x, y), None),
            _ => (self.b.ins().bnot(x), None),
        };
        let z = self.b.ins().icmp_imm_u(IntCC::Equal, out, 0);
        let az = self.bit_if(z, AZ);
        let n = self.b.ins().ushr_imm_u(out, 31);
        let an = self.b.ins().ishl_imm_u(n, 2);
        let mut bits = self.b.ins().bor(az, an);
        if let Some(f) = flags {
            bits = self.b.ins().bor(bits, f);
        }
        self.set_alu_flags(bits);
        if let Some(f) = flags {
            // AV: AOS sticks.
            self.sets_stky();
            let av = self.b.ins().band_imm_u(f, AV as i64);
            let (stick, on) = self.if_then(av);
            self.b.switch_to_block(stick);
            let st = self.ld32(layout::STKY);
            let st = self.b.ins().bor_imm_u(st, AOS as i64);
            self.st32(layout::STKY, st);
            self.b.ins().jump(on, &[]);
            self.b.switch_to_block(on);
        }
        Some(self.widen(out))
    }

    /// ASTAT's compare accumulation (bits 31-24) shifted down, `gt` (I8)
    /// coming in at the top (`Sharc::cacc`).
    fn cacc(&mut self, gt: Value) {
        let a = self.ld32(layout::ASTAT);
        let c = self.b.ins().ushr_imm_u(a, 25);
        let c = self.b.ins().ishl_imm_u(c, 24);
        let g = self.bit_if(gt, 1 << 31);
        let lo = self.b.ins().band_imm_u(a, 0xff_ffff);
        let a = self.b.ins().bor(lo, c);
        let a = self.b.ins().bor(a, g);
        self.st32(layout::ASTAT, a);
    }

    /// COMP(Rx, Ry): AZ if equal, AN if less (signed), and the compare
    /// accumulation.
    fn comp(&mut self, x40: Value, y40: Value) {
        let (x, y) = (self.field(x40), self.field(y40));
        let gt = self.b.ins().icmp(IntCC::SignedGreaterThan, x, y);
        self.cacc(gt);
        let eq = self.b.ins().icmp(IntCC::Equal, x, y);
        let lt = self.b.ins().icmp(IntCC::SignedLessThan, x, y);
        let az = self.bit_if(eq, AZ);
        let an = self.bit_if(lt, AN);
        let bits = self.b.ins().bor(az, an);
        self.set_alu_flags(bits);
    }

    /// An operand as the float ALU sees it (`float::input`): with RND32
    /// its eight LSBs clear; and whether it is a NaN (I8).
    fn finput(&mut self, x40: Value) -> (Value, Value) {
        let v = if self.mode.mode1 & RND32 != 0 { self.b.ins().band_imm_u(x40, !0xff_i64 & MASK40) } else { x40 };
        let e = self.b.ins().ushr_imm_u(v, 31);
        let e = self.b.ins().band_imm_u(e, 0xff);
        let frac = self.b.ins().band_imm_u(v, 0x7fff_ffff);
        let inf_exp = self.b.ins().icmp_imm_u(IntCC::Equal, e, 0xff);
        let some = self.b.ins().icmp_imm_u(IntCC::NotEqual, frac, 0);
        (v, self.b.ins().band(inf_exp, some))
    }

    /// Off the fast path of a float operation: `jit_alu` itself, its
    /// result (if `ret`) to `join`'s parameter.
    fn alu_slow(&mut self, op: &AluOp, x40: Value, y40: Value, join: Block, ret: bool) {
        let (s, sg) = (self.s(), self.sigs.op2);
        let p = self.p(op as *const AluOp as usize);
        let w = self.helper(sg, h_alu as HOp2 as usize, &[s, p, x40, y40]).expect("returns");
        if ret {
            self.b.ins().jump(join, &[BlockArg::Value(w)]);
        } else {
            self.b.ins().jump(join, &[]);
        }
    }

    /// FCOMP(Fx, Fy) for operands that aren't NaNs (else `jit_alu`): AZ
    /// if equal, AN if less, AF, the compare accumulation. Zeros (and
    /// denormals) of either sign are equal; otherwise the sign and the
    /// magnitude order them.
    fn fcomp(&mut self, op: &AluOp, x40: Value, y40: Value) {
        let (a, an) = self.finput(x40);
        let (b, bn) = self.finput(y40);
        let nan = self.b.ins().bor(an, bn);
        let (slow, fast) = self.if_then(nan);
        let join = self.b.create_block();
        self.b.switch_to_block(slow);
        self.b.set_cold_block(slow);
        self.alu_slow(op, x40, y40, join, false);
        self.b.switch_to_block(fast);
        let key = |e: &mut Self, v: Value| {
            let ex = e.b.ins().ushr_imm_u(v, 31);
            let ex = e.b.ins().band_imm_u(ex, 0xff);
            let zero = e.b.ins().icmp_imm_u(IntCC::Equal, ex, 0);
            let mag = e.b.ins().band_imm_u(v, 0x7f_ffff_ffff);
            let neg = e.b.ins().ineg(mag);
            let sign = e.b.ins().ushr_imm_u(v, 39);
            let k = e.b.ins().select(sign, neg, mag);
            let z = e.k64(0);
            e.b.ins().select(zero, z, k)
        };
        let (ka, kb) = (key(self, a), key(self, b));
        let gt = self.b.ins().icmp(IntCC::SignedGreaterThan, ka, kb);
        self.cacc(gt);
        let eq = self.b.ins().icmp(IntCC::Equal, ka, kb);
        let lt = self.b.ins().icmp(IntCC::SignedLessThan, ka, kb);
        let az = self.bit_if(eq, AZ);
        let an = self.bit_if(lt, AN);
        let bits = self.b.ins().bor(az, an);
        let bits = self.b.ins().bor_imm_u(bits, AF as i64);
        self.set_alu_flags(bits);
        self.b.ins().jump(join, &[]);
        self.b.switch_to_block(join);
    }

    /// FABS, -F and PASS Fx (`float::sign_op`) of an operand that isn't a
    /// NaN (else `jit_alu`): the operand with its sign cleared, flipped or
    /// kept; a zero (or denormal) becomes a zero of that sign. AZ, AN,
    /// AF; ABS's AS if the operand was negative.
    fn fsign(&mut self, op: &AluOp, x40: Value) -> Value {
        let (v, nan) = self.finput(x40);
        let (slow, fast) = self.if_then(nan);
        let join = self.b.create_block();
        let out = self.b.append_block_param(join, I64);
        self.b.switch_to_block(slow);
        self.b.set_cold_block(slow);
        let z = self.k64(0);
        self.alu_slow(op, x40, z, join, true);
        self.b.switch_to_block(fast);
        let sign = self.b.ins().ushr_imm_u(v, 39);
        let ns = match op {
            AluOp::FNeg => self.b.ins().bxor_imm_u(sign, 1),
            AluOp::FAbs => self.k64(0),
            _ => sign,
        };
        let e = self.b.ins().ushr_imm_u(v, 31);
        let e = self.b.ins().band_imm_u(e, 0xff);
        let zero = self.b.ins().icmp_imm_u(IntCC::Equal, e, 0);
        let top = self.b.ins().ishl_imm_u(ns, 39);
        let mag = self.b.ins().band_imm_u(v, 0x7f_ffff_ffff);
        let whole = self.b.ins().bor(top, mag);
        let r = self.b.ins().select(zero, top, whole);
        let az = self.bit_if(zero, AZ);
        let ns32 = self.b.ins().ireduce(I32, ns);
        let an = self.b.ins().ishl_imm_u(ns32, 2);
        let mut bits = self.b.ins().bor(az, an);
        bits = self.b.ins().bor_imm_u(bits, AF as i64);
        if *op == AluOp::FAbs {
            let was = self.b.ins().ireduce(I32, sign);
            let as_ = self.b.ins().ishl_imm_u(was, 4);
            bits = self.b.ins().bor(bits, as_);
        }
        self.set_alu_flags(bits);
        self.b.ins().jump(join, &[BlockArg::Value(r)]);
        self.b.switch_to_block(join);
        out
    }

    /// The 32-bit (RND32) fast path `float::single` takes for two normal
    /// single-precision inputs, `r` computing their exact result in f64:
    /// the 40-bit value and its sign (I32), and whether the path holds
    /// (I8; else the interpreter's operation decides).
    fn single(
        &mut self,
        x40: Value,
        y40: Value,
        r: impl FnOnce(&mut Self, Value, Value) -> Value,
        close: bool,
    ) -> (Value, Value, Value) {
        let (a, b) = (self.field(x40), self.field(y40));
        let ea = self.b.ins().ushr_imm_u(a, 23);
        let ea = self.b.ins().band_imm_u(ea, 0xff);
        let eb = self.b.ins().ushr_imm_u(b, 23);
        let eb = self.b.ins().band_imm_u(eb, 0xff);
        let normal = |e: &mut Self, ex: Value| {
            let lo = e.b.ins().icmp_imm_u(IntCC::NotEqual, ex, 0);
            let hi = e.b.ins().icmp_imm_u(IntCC::NotEqual, ex, 0xff);
            e.b.ins().band(lo, hi)
        };
        let (na, nb) = (normal(self, ea), normal(self, eb));
        let mut ok = self.b.ins().band(na, nb);
        let mode1 = self.mode1();
        let rnd = self.b.ins().band_imm_u(mode1, RND32 as i64);
        let rnd = self.b.ins().icmp_imm_u(IntCC::NotEqual, rnd, 0);
        ok = self.b.ins().band(ok, rnd);
        if close {
            // An add is exact in f64 while the exponents are at most 28
            // apart.
            let d = self.b.ins().isub(ea, eb);
            let d = self.b.ins().iadd_imm_s(d, 28);
            let near = self.b.ins().icmp_imm_u(IntCC::UnsignedLessThanOrEqual, d, 56);
            ok = self.b.ins().band(ok, near);
        }
        let fa = self.b.ins().bitcast(F32, cranelift_codegen::ir::MemFlagsData::new(), a);
        let fa = self.b.ins().fpromote(F64, fa);
        let fb = self.b.ins().bitcast(F32, cranelift_codegen::ir::MemFlagsData::new(), b);
        let fb = self.b.ins().fpromote(F64, fb);
        let res = r(self, fa, fb);
        let bits = self.b.ins().bitcast(I64, cranelift_codegen::ir::MemFlagsData::new(), res);
        let sign = self.b.ins().ushr_imm_u(bits, 63);
        let mag = self.b.ins().band_imm_u(bits, i64::MAX);
        let nz = self.b.ins().icmp_imm_u(IntCC::NotEqual, mag, 0);
        ok = self.b.ins().band(ok, nz);
        // To 23 fraction bits, to nearest even or toward zero; a carry
        // lands in the exponent.
        let mode1 = self.mode1();
        let t = self.b.ins().band_imm_u(mode1, TRUNC as i64);
        let trunc = self.b.ins().icmp_imm_u(IntCC::NotEqual, t, 0);
        let odd = self.b.ins().ushr_imm_u(mag, 29);
        let odd = self.b.ins().band_imm_u(odd, 1);
        let up = self.b.ins().iadd_imm_s(mag, (1 << 28) - 1);
        let up = self.b.ins().iadd(up, odd);
        let up = self.b.ins().ushr_imm_u(up, 29);
        let down = self.b.ins().ushr_imm_u(mag, 29);
        let rounded = self.b.ins().select(trunc, down, up);
        // Exponent (biased for f64) 897..=1150: single's -126..=127.
        let e = self.b.ins().ushr_imm_u(rounded, 23);
        let e = self.b.ins().iadd_imm_s(e, -897);
        let inr = self.b.ins().icmp_imm_u(IntCC::UnsignedLessThanOrEqual, e, 1150 - 897);
        ok = self.b.ins().band(ok, inr);
        let v = self.b.ins().iadd_imm_s(rounded, -(896i64 << 23));
        let s31 = self.b.ins().ishl_imm_u(sign, 31);
        let v = self.b.ins().bor(v, s31);
        let v = self.b.ins().band_imm_u(v, 0xffff_ffff);
        let v40 = self.b.ins().ishl_imm_u(v, 8);
        let sign = self.b.ins().ireduce(I32, sign);
        (v40, sign, ok)
    }

    /// The exact path's results for zero operands (exponent field 0: zero or
    /// a denormal, which reads as zero) with RND32, the other operand zero
    /// or normal: for an add (`sub`, if a subtract) the other operand
    /// rounded to 32 bits (its sign changed for a subtract's `y`), or a zero
    /// signed as `add_exact` signs it; for a multiply (`None`) a zero signed
    /// with the product's sign. The 40-bit value, its sign (I32), AZ (I32,
    /// the AZ bit if zero) and whether this path applies (I8).
    fn zeros(&mut self, x40: Value, y40: Value, sub: Option<bool>) -> (Value, Value, Value, Value) {
        let (a, b) = (self.field(x40), self.field(y40));
        let exp = |e: &mut Self, v: Value| {
            let x = e.b.ins().ushr_imm_u(v, 23);
            e.b.ins().band_imm_u(x, 0xff)
        };
        let (ea, eb) = (exp(self, a), exp(self, b));
        let za = self.b.ins().icmp_imm_u(IntCC::Equal, ea, 0);
        let zb = self.b.ins().icmp_imm_u(IntCC::Equal, eb, 0);
        let fa = self.b.ins().icmp_imm_u(IntCC::NotEqual, ea, 0xff);
        let fb = self.b.ins().icmp_imm_u(IntCC::NotEqual, eb, 0xff);
        let finite = self.b.ins().band(fa, fb);
        let any = self.b.ins().bor(za, zb);
        let mode1 = self.mode1();
        let rnd = self.b.ins().band_imm_u(mode1, RND32 as i64);
        let rnd = self.b.ins().icmp_imm_u(IntCC::NotEqual, rnd, 0);
        let ok = self.b.ins().band(any, finite);
        let ok = self.b.ins().band(ok, rnd);
        let sa = self.b.ins().ushr_imm_u(a, 31);
        let sb = self.b.ins().ushr_imm_u(b, 31);
        let both = self.b.ins().band(za, zb);
        let zero40 = |e: &mut Self, sign: Value| {
            let s = e.b.ins().uextend(I64, sign);
            e.b.ins().ishl_imm_u(s, 39)
        };
        let zero = self.k(0);
        match sub {
            None => {
                let sign = self.b.ins().bxor(sa, sb);
                let v = zero40(self, sign);
                (v, sign, zero, ok)
            }
            Some(sub) => {
                let sb = if sub { self.b.ins().bxor_imm_u(sb, 1) } else { sb };
                // Both zero.
                let zs = self.b.ins().band(sa, sb);
                let zv = zero40(self, zs);
                // x zero: y; y zero: x (to 32 bits, y's sign as added).
                let ym = self.b.ins().band_imm_u(y40, !0xff_i64 & 0xff_ffff_ffff);
                let ym = if sub { self.b.ins().bxor_imm_u(ym, 1 << 39) } else { ym };
                let xm = self.b.ins().band_imm_u(x40, !0xff_i64 & 0xff_ffff_ffff);
                let v = self.b.ins().select(za, ym, xm);
                let s = self.b.ins().select(za, sb, sa);
                let v = self.b.ins().select(both, zv, v);
                let s = self.b.ins().select(both, zs, s);
                let azk = self.k(AZ);
                let az = self.b.ins().select(both, azk, zero);
                (v, s, az, ok)
            }
        }
    }

    /// FADD / FSUB: `float::single`'s path in IR, else `jit_alu`.
    fn fadd(&mut self, op: &AluOp, x40: Value, y40: Value) -> Value {
        let sub = *op == AluOp::FSub;
        let (v, sign, ok) =
            self.single(x40, y40, |e, a, b| if sub { e.b.ins().fsub(a, b) } else { e.b.ins().fadd(a, b) }, true);
        let (fast, zero) = self.if_then(ok);
        let fin = self.b.create_block();
        let (fv, fs, fz) = (
            self.b.append_block_param(fin, I64),
            self.b.append_block_param(fin, I32),
            self.b.append_block_param(fin, I32),
        );
        let join = self.b.create_block();
        let out = self.b.append_block_param(join, I64);
        self.b.switch_to_block(fast);
        let z0 = self.k(0);
        self.b.ins().jump(fin, &[BlockArg::Value(v), BlockArg::Value(sign), BlockArg::Value(z0)]);
        // Off it: a zero operand's path, else the interpreter's.
        self.b.switch_to_block(zero);
        let (zv, zsign, zz, zok) = self.zeros(x40, y40, Some(sub));
        let slow = self.b.create_block();
        self.b.ins().brif(zok, fin, &[BlockArg::Value(zv), BlockArg::Value(zsign), BlockArg::Value(zz)], slow, &[]);
        self.b.switch_to_block(fin);
        // AZ, AN, AF; nothing sticks.
        let an = self.b.ins().ishl_imm_u(fs, 2);
        let bits = self.b.ins().bor(an, fz);
        let bits = self.b.ins().bor_imm_u(bits, AF as i64);
        self.set_alu_flags(bits);
        self.b.ins().jump(join, &[BlockArg::Value(fv)]);
        self.b.switch_to_block(slow);
        self.b.set_cold_block(slow);
        let (s, sg) = (self.s(), self.sigs.op2);
        let p = self.p(op as *const AluOp as usize);
        let w = self.helper(sg, h_alu as HOp2 as usize, &[s, p, x40, y40]).expect("returns");
        self.b.ins().jump(join, &[BlockArg::Value(w)]);
        self.b.switch_to_block(join);
        out
    }

    /// One multiplier operation (`Sharc::mul`): FMUL on `float::single`'s
    /// path in IR, anything else (or off the path) through `jit_mul`. The
    /// register file result, if it has one.
    fn mul(&mut self, op: &MulOp, x40: Value, y40: Value) -> Option<Value> {
        if *op != MulOp::FMul {
            let (s, sg) = (self.s(), self.sigs.op2);
            let p = self.p(op as *const MulOp as usize);
            let v = self.helper(sg, h_mul as HOp2 as usize, &[s, p, x40, y40]).expect("returns");
            let reg = match op {
                MulOp::Mul { dst, .. } | MulOp::Sat { dst, .. } | MulOp::Rnd { dst, .. } => {
                    *dst == ::adsp2106x::compute::MulDst::Reg
                }
                _ => false,
            };
            return reg.then_some(v);
        }
        let (v, sign, ok) = self.single(x40, y40, |e, a, b| e.b.ins().fmul(a, b), false);
        let (fast, zero) = self.if_then(ok);
        let fin = self.b.create_block();
        let (fv, fs) = (self.b.append_block_param(fin, I64), self.b.append_block_param(fin, I32));
        let join = self.b.create_block();
        let out = self.b.append_block_param(join, I64);
        self.b.switch_to_block(fast);
        self.b.ins().jump(fin, &[BlockArg::Value(v), BlockArg::Value(sign)]);
        self.b.switch_to_block(zero);
        let (zv, zsign, _, zok) = self.zeros(x40, y40, None);
        let slow = self.b.create_block();
        self.b.ins().brif(zok, fin, &[BlockArg::Value(zv), BlockArg::Value(zsign)], slow, &[]);
        self.b.switch_to_block(fin);
        let a = self.astat();
        let a = self.b.ins().band_imm_u(a, !(MN | MV | MU | MI) as i64);
        let mn = self.b.ins().ishl_imm_u(fs, 6);
        let a = self.b.ins().bor(a, mn);
        self.st32(layout::ASTAT, a);
        self.b.ins().jump(join, &[BlockArg::Value(fv)]);
        self.b.switch_to_block(slow);
        self.b.set_cold_block(slow);
        let (s, sg) = (self.s(), self.sigs.op2);
        let p = self.p(op as *const MulOp as usize);
        let w = self.helper(sg, h_mul as HOp2 as usize, &[s, p, x40, y40]).expect("returns");
        self.b.ins().jump(join, &[BlockArg::Value(w)]);
        self.b.switch_to_block(join);
        Some(out)
    }

    /// A type 6 shift by its immediate (`Sharc::shift`): LSHIFT and
    /// ASHIFT in IR, the rest through `jit_shift_imm`.
    fn shift_imm(&mut self, op: &ShiftOp, rn: u8, rx: u8, imm: u16) {
        let x40 = self.rf(rx);
        if matches!(op, ShiftOp::Btst | ShiftOp::Bset | ShiftOp::Bclr | ShiftOp::Btgl) {
            // The bit operations, with the bit (`ShiftY::imm`'s pos) known.
            let pos = imm as u32 & 0xff;
            let x = self.field(x40);
            let (out, sz) = if pos > 31 {
                // SV; nothing changes (BTST: SZ).
                let sz = if *op == ShiftOp::Btst {
                    self.b.ins().iconst(I8, 1)
                } else {
                    self.b.ins().icmp_imm_u(IntCC::Equal, x, 0)
                };
                (x, sz)
            } else {
                let bit = 1u32 << pos;
                match op {
                    ShiftOp::Btst => {
                        let t = self.b.ins().band_imm_u(x, bit as i64);
                        (x, self.b.ins().icmp_imm_u(IntCC::Equal, t, 0))
                    }
                    _ => {
                        let o = match op {
                            ShiftOp::Bset => self.b.ins().bor_imm_u(x, bit as i64),
                            ShiftOp::Bclr => self.b.ins().band_imm_u(x, !bit as i64),
                            _ => self.b.ins().bxor_imm_u(x, bit as i64),
                        };
                        (o, self.b.ins().icmp_imm_u(IntCC::Equal, o, 0))
                    }
                }
            };
            let sz = self.bit_if(sz, SZ);
            let sv = if pos > 31 { SV } else { 0 };
            let a = self.ld32(layout::ASTAT);
            let a = self.b.ins().band_imm_u(a, !(SV | SZ | SS) as i64);
            let a = self.b.ins().bor(a, sz);
            let a = self.b.ins().bor_imm_u(a, sv as i64);
            self.st32(layout::ASTAT, a);
            if *op != ShiftOp::Btst {
                let v = self.widen(out);
                self.set_rf(rn, v);
            }
            return;
        }
        if !matches!(op, ShiftOp::Lshift | ShiftOp::Ashift) {
            let n40 = self.rf(rn);
            let (s, sg) = (self.s(), self.sigs.shift);
            let p = self.p(op as *const ShiftOp as usize);
            let imm = self.k(imm as u32);
            let v = self.helper(sg, h_shift as HShift as usize, &[s, p, x40, n40, imm]).expect("returns");
            if *op != ShiftOp::Btst {
                self.set_rf(rn, v);
            }
            return;
        }
        let op = *op;
        let x = self.field(x40);
        let s = imm as u8 as i8 as i32;
        let arith = op == ShiftOp::Ashift;
        let zero = self.k(0);
        let (r, ov) = match s {
            32.. => (zero, self.b.ins().icmp_imm_u(IntCC::NotEqual, x, 0)),
            1..=31 => {
                let r = self.b.ins().ishl_imm_u(x, s as i64);
                let ov = if arith {
                    let back = self.b.ins().sshr_imm_u(r, s as i64);
                    self.b.ins().icmp(IntCC::NotEqual, back, x)
                } else {
                    let out = self.b.ins().ushr_imm_u(x, 32 - s as i64);
                    self.b.ins().icmp_imm_u(IntCC::NotEqual, out, 0)
                };
                (r, ov)
            }
            ..=-32 => (if arith { self.b.ins().sshr_imm_u(x, 31) } else { zero }, self.b.ins().iconst(I8, 0)),
            -31..=-1 => (
                if arith { self.b.ins().sshr_imm_u(x, -s as i64) } else { self.b.ins().ushr_imm_u(x, -s as i64) },
                self.b.ins().iconst(I8, 0),
            ),
            0 => (x, self.b.ins().iconst(I8, 0)),
        };
        let sv = self.bit_if(ov, SV);
        let z = self.b.ins().icmp_imm_u(IntCC::Equal, r, 0);
        let sz = self.bit_if(z, SZ);
        let a = self.astat();
        let a = self.b.ins().band_imm_u(a, !(SV | SZ | SS) as i64);
        let a = self.b.ins().bor(a, sv);
        let a = self.b.ins().bor(a, sz);
        self.st32(layout::ASTAT, a);
        let v = self.widen(r);
        self.set_rf(rn, v);
    }
}

// The helpers: the interpreter's own pieces.
//
// SAFETY (all of them): called only by regions `SharcJit::one` started,
// with the core, the `&mut dyn Bus` and the context it passed, live and
// distinct, and pointers into the region's `words`.

type HExec = unsafe extern "C" fn(*mut Sharc, *mut Ctx, *const Insn, u64) -> u32;
type HFetch = unsafe extern "C" fn(*mut Sharc, u32, u32, u32, *mut JitPipe) -> u64;
type HRoom = unsafe extern "C" fn(*mut Sharc) -> u32;
type HPipe = unsafe extern "C" fn(*mut Sharc, *mut JitPipe);
type HPipe1 = unsafe extern "C" fn(*mut Sharc, u32, *mut JitPipe);
type HDo = unsafe extern "C" fn(*mut Sharc, u32, u32, u64, *mut JitPipe);
type HPop = unsafe extern "C" fn(*mut Sharc) -> u32;
type HCompute = unsafe extern "C" fn(*mut Sharc, *const Compute);
type HOp2 = unsafe extern "C" fn(*mut Sharc, *const u8, u64, u64) -> u64;
type HShift = unsafe extern "C" fn(*mut Sharc, *const ShiftOp, u64, u64, u32) -> u64;
type HRead = unsafe extern "C" fn(*mut Sharc, *mut Ctx, u32, u32) -> u64;
type HWrite = unsafe extern "C" fn(*mut Sharc, *mut Ctx, u32, u32, u64);
type HAccess = unsafe extern "C" fn(*mut Sharc, *mut Ctx, u64);
type HCond = unsafe extern "C" fn(*mut Sharc, u32) -> u32;

/// The block's `&mut dyn Bus` (`Ctx::bus`).
unsafe fn bus<'e>(ctx: *mut Ctx) -> &'e mut &'e mut dyn Bus {
    unsafe { &mut *((*ctx).bus as *mut &mut dyn Bus) }
}

/// One word of a block (`Sharc::block_exec`): 0 if it ran and the block
/// goes on, else how the block ends.
unsafe extern "C" fn h_exec(s: *mut Sharc, ctx: *mut Ctx, insn: *const Insn, steps: u64) -> u32 {
    let ext = unsafe { bus(ctx) };
    let (s, ctx, insn) = unsafe { (&mut *s, &mut *ctx, &*insn) };
    match s.block_exec(insn, &mut ctx.st, steps, ext) {
        InBlock::Ran => {
            ctx.helper += 1;
            0
        }
        InBlock::Last => {
            ctx.helper += 1;
            LAST
        }
        InBlock::Bail => BAIL,
        InBlock::Fault(f) => {
            ctx.fault = Some(f);
            FAULT
        }
    }
}

unsafe extern "C" fn h_compute(s: *mut Sharc, c: *const Compute) {
    unsafe { (*s).jit_compute(&*c) }
}

unsafe extern "C" fn h_alu(s: *mut Sharc, op: *const u8, x: u64, y: u64) -> u64 {
    unsafe { (*s).jit_alu(*(op as *const AluOp), x, y) }
}

unsafe extern "C" fn h_mul(s: *mut Sharc, op: *const u8, x: u64, y: u64) -> u64 {
    unsafe { (*s).jit_mul(*(op as *const MulOp), x, y) }
}

unsafe extern "C" fn h_shift(s: *mut Sharc, op: *const ShiftOp, x: u64, n: u64, imm: u32) -> u64 {
    unsafe { (*s).jit_shift_imm(*op, x, imm as u16, n) }
}

unsafe extern "C" fn h_read(s: *mut Sharc, e: *mut Ctx, pm: u32, addr: u32) -> u64 {
    unsafe { (*s).jit_read(pm != 0, addr, bus(e)) }
}

unsafe extern "C" fn h_write(s: *mut Sharc, e: *mut Ctx, pm: u32, addr: u32, v: u64) {
    unsafe { (*s).jit_write(pm != 0, addr, v, bus(e)) }
}

/// After an external access in a block: the step count it leaves, then
/// the bus's `after_access`.
unsafe extern "C" fn h_after_access(s: *mut Sharc, e: *mut Ctx, steps: u64) {
    let s = unsafe { &mut *s };
    s.steps = steps;
    unsafe { bus(e) }.after_access(s);
}

unsafe extern "C" fn h_cond(s: *mut Sharc, c: u32) -> u32 {
    unsafe { (*s).cond(c as u8) as u32 }
}

unsafe extern "C" fn h_fetch(s: *mut Sharc, fetched: u32, astat: u32, flag_in: u32, p: *mut JitPipe) -> u64 {
    unsafe { (*s).jit_fetch(fetched, astat, flag_in as u8, &mut *p) }
}

unsafe extern "C" fn h_room(s: *mut Sharc) -> u32 {
    unsafe { (*s).jit_stack_room() }
}

unsafe extern "C" fn h_push_pc(s: *mut Sharc, v: u32, p: *mut JitPipe) {
    unsafe { (*s).jit_push_pc(v, &mut *p) }
}

unsafe extern "C" fn h_pop_pc(s: *mut Sharc) -> u32 {
    unsafe { (*s).jit_pop_pc() }
}

unsafe extern "C" fn h_abort(s: *mut Sharc, p: *mut JitPipe) {
    unsafe { (*s).jit_abort_loop(&mut *p) }
}

unsafe extern "C" fn h_reentry(s: *mut Sharc) -> u32 {
    unsafe { (*s).jit_reentry() };
    0
}

/// `packed`: the count (if bit 32) and the termination code (bits 47-40).
unsafe extern "C" fn h_do(s: *mut Sharc, pc: u32, end: u32, packed: u64, p: *mut JitPipe) {
    let count = (packed >> 32 & 1 != 0).then_some(packed as u32);
    let term = (packed >> 40) as u8;
    unsafe { (*s).jit_do(pc, count, term, end, &mut *p) }
}

unsafe extern "C" fn h_stacks(s: *mut Sharc, bits: u32, p: *mut JitPipe) {
    let st = ::adsp2106x::StackOps {
        push_loop: bits & 1 != 0,
        pop_loop: bits & 2 != 0,
        push_sts: false,
        pop_sts: false,
        push_pcstk: bits & 4 != 0,
        pop_pcstk: bits & 8 != 0,
        flush_cache: false,
    };
    unsafe { (*s).jit_stacks(st, &mut *p) }
}

/// `invalidate` for a 32-bit store (`HCond`'s shape: the result unused).
unsafe extern "C" fn h_invalidate(s: *mut Sharc, addr: u32) -> u32 {
    unsafe { (*s).jit_invalidate32(addr) };
    0
}
