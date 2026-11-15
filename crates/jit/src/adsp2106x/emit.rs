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

use super::{Ctx, BAIL, BEFORE, FAULT, LAST};
use crate::copro::{call, sig, trusted, F32, F64, I32, I64, I8};
use ::adsp2106x::compute::{MfRegs, MulOp, ShiftOp};
use ::adsp2106x::cpu::{layout, Bus, InBlock, Sharc};
use ::adsp2106x::{Addr, AluOp, Compute, Insn, Modifier, Op, TRUE};
use cranelift_codegen::ir::{condcodes::IntCC, Block, BlockArg, InstBuilder, JumpTableData, SigRef, Type, Value};
use cranelift_frontend::FunctionBuilder;
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

/// A universal register the lowering reads and writes in place: the
/// register file and the DAG registers.
fn plain_ureg(r: u8) -> bool {
    r < 0x50
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
        Op::ModifyImm { bitrev, .. } => !bitrev,
        Op::Branch { cond, .. } | Op::Return { cond, .. } | Op::JumpElseMove { cond, .. } => cond != TRUE,
        Op::Transfer { mv, .. } => plain_ureg(mv.reg),
        Op::ShiftImm { mv, .. } => mv.is_none_or(|m| plain_ureg(m.reg)),
        Op::RegMove { src, dst, .. } => plain_ureg(src) && plain_ureg(dst),
        Op::LoadImm { reg, .. } | Op::IndirectImm { reg, .. } => plain_ureg(reg),
        Op::Direct { reg, addr, .. } => plain_ureg(reg) && addr >= 0x2_0000,
        _ => false,
    }
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
}

/// The IR of a region.
pub(super) struct Emit<'a, 'b> {
    b: &'a mut FunctionBuilder<'b>,
    ptr: Type,
    s: Value,
    ext: Value,
    ctx: Value,
    /// MODE1, which no word of a block changes.
    mode1: Value,
    /// The exit: (kind, word).
    exit: Block,
    sigs: Sigs,
}

impl<'a, 'b> Emit<'a, 'b> {
    pub fn new(b: &'a mut FunctionBuilder<'b>, ptr: Type) -> Self {
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let p = b.block_params(entry).to_vec();
        let exit = b.create_block();
        b.append_block_param(exit, I32);
        b.append_block_param(exit, I32);
        let mode1 = b.ins().load(I32, trusted(), p[0], layout::MODE1 as i32);
        let sigs = Sigs {
            exec: sig(b, &[ptr, ptr, ptr, ptr], Some(I32)),
            compute: sig(b, &[ptr, ptr], None),
            op2: sig(b, &[ptr, ptr, I64, I64], Some(I64)),
            shift: sig(b, &[ptr, ptr, I64, I64, I32], Some(I64)),
            read: sig(b, &[ptr, ptr, I32, I32], Some(I64)),
            write: sig(b, &[ptr, ptr, I32, I32, I64], None),
            access: sig(b, &[ptr, ptr, I64], None),
            cond: sig(b, &[ptr, I32], Some(I32)),
        };
        Emit { b, ptr, s: p[0], ext: p[1], ctx: p[2], mode1, exit, sigs }
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

    fn leave(&mut self, kind: u32, at: u32) {
        let (k, a) = (self.k(kind), self.k(at));
        self.b.ins().jump(self.exit, &[BlockArg::Value(k), BlockArg::Value(a)]);
    }

    fn ld32(&mut self, off: usize) -> Value {
        self.b.ins().load(I32, trusted(), self.s, off as i32)
    }

    fn st32(&mut self, off: usize, v: Value) {
        self.b.ins().store(trusted(), v, self.s, off as i32);
    }

    fn rf(&mut self, n: u8) -> Value {
        self.b.ins().load(I64, trusted(), self.s, (layout::R + 8 * (n as usize & 15)) as i32)
    }

    fn set_rf(&mut self, n: u8, v: Value) {
        let v = self.b.ins().band_imm_u(v, MASK40);
        self.b.ins().store(trusted(), v, self.s, (layout::R + 8 * (n as usize & 15)) as i32);
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
        call(self.b, self.ptr, s, f, args)
    }

    /// Branch on `c` (an I8 or I32): `yes` when it holds, else on.
    fn if_then(&mut self, c: Value) -> (Block, Block) {
        let (yes, on) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(c, yes, &[], on, &[]);
        (yes, on)
    }

    // ---- the region

    /// The region `words`: a jump table to the word it is entered at,
    /// then each word in turn, leaving at the block's stop or as a word
    /// says.
    pub fn region(&mut self, words: &[Insn]) {
        let start = words[0].addr;
        let blocks: Vec<_> = words.iter().map(|_| self.b.create_block()).collect();
        let outside = self.b.create_block();
        let at = self.b.ins().load(I32, trusted(), self.ctx, offset_of!(Ctx, at) as i32);
        let idx = self.b.ins().iadd_imm_s(at, -(start as i64));
        let calls: Vec<_> = blocks.iter().map(|&w| self.b.func.dfg.block_call(w, &[])).collect();
        let def = self.b.func.dfg.block_call(outside, &[]);
        let jt = self.b.create_jump_table(JumpTableData::new(def, &calls));
        self.b.ins().br_table(idx, jt);
        self.b.switch_to_block(outside);
        let before = self.k(BEFORE);
        self.b.ins().jump(self.exit, &[BlockArg::Value(before), BlockArg::Value(at)]);
        for (j, i) in words.iter().enumerate() {
            self.b.switch_to_block(blocks[j]);
            let a = i.addr;
            if j > 0 {
                let stop = self.b.ins().load(I32, trusted(), self.ctx, offset_of!(Ctx, stop) as i32);
                let here = self.b.ins().icmp_imm_u(IntCC::Equal, stop, a as i64);
                let (out, on) = self.if_then(here);
                self.b.switch_to_block(out);
                self.leave(BEFORE, a);
                self.b.switch_to_block(on);
            }
            if lowered(i) {
                self.word(i);
            } else {
                self.interpreted(i);
            }
            match blocks.get(j + 1) {
                Some(&next) => {
                    self.b.ins().jump(next, &[]);
                }
                None => self.leave(BEFORE, a + 1),
            }
        }
        self.b.switch_to_block(self.exit);
        let p = self.b.block_params(self.exit).to_vec();
        self.b.ins().store(trusted(), p[0], self.ctx, offset_of!(Ctx, kind) as i32);
        self.b.ins().store(trusted(), p[1], self.ctx, offset_of!(Ctx, end) as i32);
        self.b.ins().return_(&[]);
    }

    /// `block_exec` itself, for a word the lowering doesn't have.
    fn interpreted(&mut self, i: &Insn) {
        let insn = self.p(i as *const Insn as usize);
        let (s, ext, ctx, sg) = (self.s, self.ext, self.ctx, self.sigs.exec);
        let r = self.helper(sg, h_exec as HExec as usize, &[s, ext, ctx, insn]).expect("returns");
        let (stopped, on) = self.if_then(r);
        self.b.switch_to_block(stopped);
        let at = self.k(i.addr);
        self.b.ins().jump(self.exit, &[BlockArg::Value(r), BlockArg::Value(at)]);
        self.b.switch_to_block(on);
    }

    /// `*st = self.stky`, as `block_exec` notes it before the word runs.
    fn note_st(&mut self) {
        let st = self.ld32(layout::STKY);
        self.b.ins().store(trusted(), st, self.ctx, offset_of!(Ctx, st) as i32);
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
            Op::ModifyImm { i: di, data, .. } => {
                self.note_st();
                let m = self.b.ins().iconst(I64, *data as i64);
                self.modify(*di, m);
            }
            Op::Branch { cond, else_, compute, .. } | Op::Return { cond, else_, compute, .. } => {
                // Taken, it ends the block before it.
                let c = self.cond(*cond);
                let (taken, on) = self.if_then(c);
                self.b.switch_to_block(taken);
                self.leave(BAIL, a);
                self.b.switch_to_block(on);
                self.note_st();
                if *else_ {
                    self.compute(compute);
                }
            }
            Op::JumpElseMove { cond, compute, mv, .. } => {
                let c = self.cond(*cond);
                let (taken, on) = self.if_then(c);
                self.b.switch_to_block(taken);
                self.leave(BAIL, a);
                self.b.switch_to_block(on);
                self.transfer(a, compute, mv.addr, mv.write, mv.reg);
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
        let base = self.b.ins().load(I64, trusted(), self.ctx, offset_of!(Ctx, base) as i32);
        let steps = self.b.ins().iadd_imm_s(base, a as i64 + 1);
        let (s, e, sg) = (self.s, self.ext, self.sigs.access);
        self.helper(sg, h_after_access as HAccess as usize, &[s, e, steps]);
        self.b.ins().jump(on, &[]);
        self.b.switch_to_block(on);
        if wrote {
            let d = self.b.ins().load(I8, trusted(), self.s, layout::CODE_DIRTY as i32);
            let (dirty, on) = self.if_then(d);
            self.b.switch_to_block(dirty);
            self.leave(LAST, a);
            self.b.switch_to_block(on);
        }
    }

    /// A read (`None` for a write of `value`) at `at` through the PM bus
    /// (`pm`) or the DM bus: the register's new value.
    fn mem(&mut self, pm: bool, at: Value, write: bool, value: Value) -> Option<Value> {
        let (s, e) = (self.s, self.ext);
        let pm = self.k(pm as u32);
        if write {
            let sg = self.sigs.write;
            self.helper(sg, h_write as HWrite as usize, &[s, e, pm, at, value]);
            None
        } else {
            Some(self.read(pm, at))
        }
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
        let iop = self.b.ins().load(self.ptr, trusted(), self.ctx, offset_of!(Ctx, iop) as i32);
        let syscon = self.b.ins().load(I32, trusted(), iop, 0);
        let b1 = self.b.ins().uextend(I32, block1);
        let sh = self.b.ins().iadd_imm_s(b1, 8);
        let imdw = self.b.ins().ushr(syscon, sh);
        let imdw = self.b.ins().band_imm_u(imdw, 1);
        let (wide, plain) = self.if_then(imdw);
        self.b.switch_to_block(wide);
        self.b.ins().jump(slow, &[]);
        self.b.switch_to_block(plain);
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
        let (s, e, sg) = (self.s, self.ext, self.sigs.read);
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
            _ => unreachable!("not a plain register"),
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
            _ => unreachable!("not a plain register"),
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
            let br = self.b.ins().band_imm_u(self.mode1, if dag2 { BR8 } else { BR0 } as i64);
            let rev = self.b.ins().bitrev(out);
            let rev = if dag2 { self.b.ins().ushr_imm_u(rev, 8) } else { rev };
            self.b.ins().select(br, rev, out)
        } else {
            out
        }
    }

    /// [`peek`](Emit::peek), leaving before the word (as `block_exec`
    /// declines it) for an IOP register or reserved address; and whether
    /// it is on the external port (I8).
    fn peek_checked(&mut self, at: u32, a: &Addr) -> (Value, Value) {
        let addr = self.peek(a);
        let low = self.b.ins().icmp_imm_u(IntCC::UnsignedLessThan, addr, 0x2_0000);
        let (bail, on) = self.if_then(low);
        self.b.switch_to_block(bail);
        self.leave(BAIL, at);
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
        let b = self.ld32(layout::B + 4 * i);
        let b = self.b.ins().uextend(I64, b);
        let l = self.ld32(layout::L + 4 * i);
        let l = self.b.ins().uextend(I64, l);
        let sum = self.b.ins().iadd(iv, m);
        let raw = self.b.ins().band_imm_u(sum, mask);
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
            (15, _) | (31, _) | (_, 9..=12) | (_, 14) => {
                let (s, sg) = (self.s, self.sigs.cond);
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
                let sat = self.b.ins().band_imm_u(self.mode1, ALUSAT as i64);
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
                let (s, sg) = (self.s, self.sigs.compute);
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
        if !alu_ir(*op) {
            let (s, sg) = (self.s, self.sigs.op2);
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
                let sat = self.b.ins().band_imm_u(self.mode1, ALUSAT as i64);
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
        let rnd = self.b.ins().band_imm_u(self.mode1, RND32 as i64);
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
        let t = self.b.ins().band_imm_u(self.mode1, TRUNC as i64);
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

    /// FADD / FSUB: `float::single`'s path in IR, else `jit_alu`.
    fn fadd(&mut self, op: &AluOp, x40: Value, y40: Value) -> Value {
        let sub = *op == AluOp::FSub;
        let (v, sign, ok) =
            self.single(x40, y40, |e, a, b| if sub { e.b.ins().fsub(a, b) } else { e.b.ins().fadd(a, b) }, true);
        let (fast, slow) = self.if_then(ok);
        let join = self.b.create_block();
        let out = self.b.append_block_param(join, I64);
        self.b.switch_to_block(fast);
        // AN, AF; nothing sticks.
        let an = self.b.ins().ishl_imm_u(sign, 2);
        let bits = self.b.ins().bor_imm_u(an, AF as i64);
        self.set_alu_flags(bits);
        self.b.ins().jump(join, &[BlockArg::Value(v)]);
        self.b.switch_to_block(slow);
        self.b.set_cold_block(slow);
        let (s, sg) = (self.s, self.sigs.op2);
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
            let (s, sg) = (self.s, self.sigs.op2);
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
        let (fast, slow) = self.if_then(ok);
        let join = self.b.create_block();
        let out = self.b.append_block_param(join, I64);
        self.b.switch_to_block(fast);
        let a = self.astat();
        let a = self.b.ins().band_imm_u(a, !(MN | MV | MU | MI) as i64);
        let mn = self.b.ins().ishl_imm_u(sign, 6);
        let a = self.b.ins().bor(a, mn);
        self.st32(layout::ASTAT, a);
        self.b.ins().jump(join, &[BlockArg::Value(v)]);
        self.b.switch_to_block(slow);
        self.b.set_cold_block(slow);
        let (s, sg) = (self.s, self.sigs.op2);
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
        if !matches!(op, ShiftOp::Lshift | ShiftOp::Ashift) {
            let n40 = self.rf(rn);
            let (s, sg) = (self.s, self.sigs.shift);
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

type HExec = unsafe extern "C" fn(*mut Sharc, *mut u8, *mut Ctx, *const Insn) -> u32;
type HCompute = unsafe extern "C" fn(*mut Sharc, *const Compute);
type HOp2 = unsafe extern "C" fn(*mut Sharc, *const u8, u64, u64) -> u64;
type HShift = unsafe extern "C" fn(*mut Sharc, *const ShiftOp, u64, u64, u32) -> u64;
type HRead = unsafe extern "C" fn(*mut Sharc, *mut u8, u32, u32) -> u64;
type HWrite = unsafe extern "C" fn(*mut Sharc, *mut u8, u32, u32, u64);
type HAccess = unsafe extern "C" fn(*mut Sharc, *mut u8, u64);
type HCond = unsafe extern "C" fn(*mut Sharc, u32) -> u32;

unsafe fn bus<'e>(e: *mut u8) -> &'e mut &'e mut dyn Bus {
    unsafe { &mut *(e as *mut &mut dyn Bus) }
}

/// One word of a block (`Sharc::block_exec`): 0 if it ran and the block
/// goes on, else how the block ends.
unsafe extern "C" fn h_exec(s: *mut Sharc, e: *mut u8, ctx: *mut Ctx, insn: *const Insn) -> u32 {
    let (s, ctx, insn) = unsafe { (&mut *s, &mut *ctx, &*insn) };
    let steps = ctx.base.wrapping_add(insn.addr as u64 + 1);
    match s.block_exec(insn, &mut ctx.st, steps, unsafe { bus(e) }) {
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

unsafe extern "C" fn h_read(s: *mut Sharc, e: *mut u8, pm: u32, addr: u32) -> u64 {
    unsafe { (*s).jit_read(pm != 0, addr, bus(e)) }
}

unsafe extern "C" fn h_write(s: *mut Sharc, e: *mut u8, pm: u32, addr: u32, v: u64) {
    unsafe { (*s).jit_write(pm != 0, addr, v, bus(e)) }
}

/// After an external access in a block: the step count it leaves, then
/// the bus's `after_access`.
unsafe extern "C" fn h_after_access(s: *mut Sharc, e: *mut u8, steps: u64) {
    let s = unsafe { &mut *s };
    s.steps = steps;
    unsafe { bus(e) }.after_access(s);
}

unsafe extern "C" fn h_cond(s: *mut Sharc, c: u32) -> u32 {
    unsafe { (*s).cond(c as u8) as u32 }
}
