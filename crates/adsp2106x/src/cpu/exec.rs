//! Instruction execution: the instruction types (UM Appendix A) and the
//! ALU, multiplier and shifter (UM Chapter 2, Appendix B).

use super::*;
use crate::compute::{AluOp, Mr, MulDst, MulOp, ShiftOp};
use crate::float::{self, Class, Mode, ToInt};
use crate::{BitOp, Compute, Count, Move, Op, StackOps, Target};

/// Register file results of a compute operation, written in order (so a
/// later one wins: multiplier before ALU, UM §2.9).
#[derive(Default)]
pub(super) struct Results {
    w: [(u8, u64); 3],
    n: usize,
}

impl Results {
    fn push(&mut self, r: u8, v: u64) {
        self.w[self.n] = (r & 15, v);
        self.n += 1;
    }
}

/// ALU status of one operation (UM §2.5.3).
#[derive(Clone, Copy, Debug, Default)]
struct AluFlags {
    az: bool,
    av: bool,
    an: bool,
    ac: bool,
    as_: bool,
    ai: bool,
    af: bool,
    au: bool,
}

impl AluFlags {
    fn or(self, o: AluFlags) -> AluFlags {
        AluFlags {
            az: self.az || o.az,
            av: self.av || o.av,
            an: self.an || o.an,
            ac: self.ac || o.ac,
            as_: self.as_ || o.as_,
            ai: self.ai || o.ai,
            af: self.af || o.af,
            au: self.au || o.au,
        }
    }

    fn float(f: float::Flags) -> AluFlags {
        AluFlags {
            az: f.zero,
            av: f.overflow,
            an: f.neg,
            ai: f.invalid,
            au: f.underflow,
            af: true,
            ..Default::default()
        }
    }

    fn fixed(v: u32) -> AluFlags {
        AluFlags { az: v == 0, an: v >> 31 != 0, ..Default::default() }
    }
}

/// The shifter's Y operand: a register's 32-bit field or type 6's
/// immediate (UM §2.7.1, Figure 2.5).
#[derive(Clone, Copy, Debug)]
struct ShiftY {
    /// Shift or rotate amount, positive left.
    shift: i64,
    /// Bit position for BSET/BCLR/BTGL/BTST, unsigned (so negative is out
    /// of range, like anything above 31).
    pos: u64,
    bit6: u32,
    len6: u32,
}

impl ShiftY {
    fn reg(y: u32) -> ShiftY {
        ShiftY { shift: y as i32 as i64, pos: y as u64, bit6: y & 0x3f, len6: (y >> 6) & 0x3f }
    }

    fn imm(imm: u16) -> ShiftY {
        let i = imm as u32;
        ShiftY { shift: imm as u8 as i8 as i64, pos: (i & 0xff) as u64, bit6: i & 0x3f, len6: (i >> 6) & 0x3f }
    }
}

fn wrap80(v: i128) -> i128 {
    (v << 48) >> 48
}

/// Round an MR value to nearest (even) at bit 32, clearing MR0 (UM
/// §2.6.3.2).
fn round80(v: i128) -> i128 {
    let low = v & 0xffff_ffff;
    let mut q = v >> 32;
    if low > 0x8000_0000 || low == 0x8000_0000 && q & 1 == 1 {
        q += 1;
    }
    wrap80(q << 32)
}

/// `a + b + c` in 32 bits: result, carry out, signed overflow.
fn add3(a: u32, b: u32, c: u32) -> (u32, bool, bool) {
    let s = a as u64 + b as u64 + c as u64;
    let r = s as u32;
    (r, s >> 32 != 0, ((a ^ r) & (b ^ r)) >> 31 != 0)
}

fn sext_from(v: u64, len: u32) -> u64 {
    if len == 0 || len >= 64 {
        v
    } else {
        (((v << (64 - len)) as i64) >> (64 - len)) as u64
    }
}

impl Sharc {
    fn fmode(&self) -> Mode {
        Mode { trunc: self.mode1 & TRUNC != 0, rnd32: self.mode1 & RND32 != 0 }
    }

    fn apply(&mut self, res: &Results) {
        for &(r, v) in &res.w[..res.n] {
            self.r[r as usize] = v & 0xff_ffff_ffff;
        }
    }

    fn branch(&mut self, target: u32, delayed: bool) {
        self.redirect = Some((target, delayed));
        // No interrupt until the branch (and its delay slots) are done.
        self.irq_hold = if delayed { 2 } else { 1 };
    }

    fn target(&self, t: Target, pc: u32) -> u32 {
        match t {
            Target::Abs(a) => a,
            Target::Rel(d) => pc.wrapping_add(d as u32) & 0xff_ffff,
            Target::Indirect { i, m } => {
                let v = (self.i[i as usize] as i64 + self.m_value(m)) as u32 & 0xff_ffff;
                if i == 8 && self.mode1 & BR8 != 0 {
                    bitrev(v, 24)
                } else {
                    v
                }
            }
        }
    }

    /// A compute operation with an optional memory transfer (types 1, 3, 4,
    /// 6, 10): registers are read, then the compute runs, then memory is
    /// accessed, then results land: compute, then the loaded value.
    fn compute_move(&mut self, res: Results, mv: Option<Move>, ext: &mut impl Bus) -> Result<(), Fault> {
        let loaded = match mv {
            Some(mv) => {
                let value = self.ureg_read(mv.reg);
                let addr = self.dag_output(&mv.addr);
                self.transfer(mv.addr.pm(), addr, mv.write, mv.reg, value, ext)?.map(|l| (mv.reg, l))
            }
            None => None,
        };
        self.apply(&res);
        if let Some((reg, l)) = loaded {
            self.apply_load(reg, l);
        }
        Ok(())
    }

    pub(super) fn exec(&mut self, insn: &Insn, ext: &mut impl Bus) -> Result<(), Fault> {
        let pc = insn.addr;
        let undefined = Fault::Undefined { pc, word: insn.word };
        match insn.op {
            Op::Nop => {}
            Op::Compute { cond, compute } => {
                if self.cond(cond) {
                    let res = self.compute(&compute).ok_or(undefined)?;
                    self.apply(&res);
                }
            }
            Op::Dual { compute, dm, pm } => {
                let (dv, pv) = (self.r[dm.reg as usize], self.r[pm.reg as usize]);
                let res = self.compute(&compute).ok_or(undefined)?;
                let (da, pa) = (self.dag_output(&dm.addr), self.dag_output(&pm.addr));
                let dl = self.transfer(false, da, dm.write, dm.reg, dv, ext)?;
                let pl = self.transfer(true, pa, pm.write, pm.reg, pv, ext)?;
                self.apply(&res);
                // DM has the higher precedence (UM §2.9).
                if let Some(l) = pl {
                    self.apply_load(pm.reg, l);
                }
                if let Some(l) = dl {
                    self.apply_load(dm.reg, l);
                }
            }
            Op::Transfer { cond, compute, mv } => {
                if self.cond(cond) {
                    let value = self.ureg_read(mv.reg);
                    let res = self.compute(&compute).ok_or(undefined)?;
                    let addr = self.dag_output(&mv.addr);
                    let l = self.transfer(mv.addr.pm(), addr, mv.write, mv.reg, value, ext)?;
                    self.apply(&res);
                    if let Some(l) = l {
                        self.apply_load(mv.reg, l);
                    }
                }
            }
            Op::RegMove { cond, compute, src, dst } => {
                if self.cond(cond) {
                    let v = self.ureg_read(src);
                    let res = self.compute(&compute).ok_or(undefined)?;
                    self.apply(&res);
                    self.ureg_write(dst, v);
                }
            }
            Op::ShiftImm { cond, op, rn, rx, imm, mv } => {
                if self.cond(cond) {
                    let mut res = Results::default();
                    let (x, n) = (self.r[rx as usize], self.r[rn as usize]);
                    if let Some(v) = self.shift(op, x, ShiftY::imm(imm), n) {
                        res.push(rn, v);
                    }
                    self.compute_move(res, mv, ext)?;
                }
            }
            Op::Modify { cond, compute, i, m } => {
                if self.cond(cond) {
                    let res = self.compute(&compute).ok_or(undefined)?;
                    let mv = self.m_value(m);
                    self.modify(i as usize, mv);
                    self.apply(&res);
                }
            }
            Op::Branch { cond, call, target, delayed, loop_abort, clear_int, compute, else_ } => {
                if self.cond(cond) {
                    let t = self.target(target, pc);
                    let res = if else_ { Results::default() } else { self.compute(&compute).ok_or(undefined)? };
                    if call {
                        // Delayed: the third word after the call (UM §3.4.1);
                        // otherwise the word in the decode stage.
                        let ret = if delayed { pc.wrapping_add(3) } else { self.decode_slot.map_or(pc + 1, |s| s.0) };
                        self.push_pc(ret)?;
                    } else {
                        if loop_abort {
                            self.pop_loop();
                            self.pop_pc();
                        }
                        if clear_int {
                            self.end_interrupt();
                        }
                    }
                    self.apply(&res);
                    self.branch(t, delayed);
                } else if else_ {
                    let res = self.compute(&compute).ok_or(undefined)?;
                    self.apply(&res);
                }
            }
            Op::JumpElseMove { cond, target, compute, mv } => {
                if self.cond(cond) {
                    let t = self.target(target, pc);
                    self.branch(t, false);
                } else {
                    let res = self.compute(&compute).ok_or(undefined)?;
                    self.compute_move(res, Some(mv), ext)?;
                }
            }
            Op::Return { cond, rti, delayed, loop_reentry, compute, else_ } => {
                if self.cond(cond) {
                    let res = if else_ { Results::default() } else { self.compute(&compute).ok_or(undefined)? };
                    let Some(t) = self.pop_pc() else {
                        return Err(Fault::Stack { pc, what: "return with an empty PC stack" });
                    };
                    self.apply(&res);
                    if rti {
                        self.end_interrupt();
                    }
                    if loop_reentry {
                        self.reentry = self.last_loopback.is_some();
                    }
                    self.branch(t, delayed);
                } else if else_ {
                    let res = self.compute(&compute).ok_or(undefined)?;
                    self.apply(&res);
                }
            }
            Op::Do { count, term, end } => {
                match count {
                    Some(Count::Imm(n)) => self.lcntr = n as u32,
                    Some(Count::Ureg(r)) => self.lcntr = (self.ureg_read(r) >> 8) as u32,
                    None => {}
                }
                let len = end.wrapping_sub(pc);
                let kind = match (term == LCE, len) {
                    (false, _) => LoopKind::Cond,
                    (true, 1) => LoopKind::Counter1,
                    (true, 2) => LoopKind::Counter2,
                    _ => LoopKind::Counter,
                };
                self.push_pc(pc.wrapping_add(1))?;
                self.push_loop(laddr_pack(end, term, kind), self.lcntr)?;
                // A one-instruction loop's end is already in the pipeline.
                self.rebuild = len == 1;
            }
            Op::Direct { pm, write, reg, addr } => {
                let value = self.ureg_read(reg);
                if let Some(l) = self.transfer(pm, addr, write, reg, value, ext)? {
                    self.apply_load(reg, l);
                }
            }
            Op::IndirectImm { write, reg, i, offset } => {
                let value = self.ureg_read(reg);
                let addr = self.dag_output(&Addr { i, modifier: Modifier::Imm(offset), post: false });
                if let Some(l) = self.transfer(i >= 8, addr, write, reg, value, ext)? {
                    self.apply_load(reg, l);
                }
            }
            Op::StoreImm { addr, data } => {
                // The data goes to the upper 32 bits of a 40-bit word (UM A-42).
                let a = self.dag_output(&addr);
                self.store(addr.pm(), a, (data as u64) << 8, None, ext)?;
            }
            Op::LoadImm { reg, data } => self.ureg_write(reg, (data as u64) << 8),
            Op::Bit { op, sreg, data } => {
                let v = (self.ureg_read(sreg) >> 8) as u32;
                let new = match op {
                    BitOp::Set => Some(v | data),
                    BitOp::Clr => Some(v & !data),
                    BitOp::Tgl => Some(v ^ data),
                    BitOp::Tst | BitOp::Xor => {
                        let t = if op == BitOp::Tst { v & data == data } else { v == data };
                        self.astat = (self.astat & !BTF) | if t { BTF } else { 0 };
                        None
                    }
                };
                if let Some(n) = new {
                    self.ureg_write(sreg, (n as u64) << 8);
                }
            }
            Op::ModifyImm { i, data, bitrev: br } => {
                if br {
                    let (width, mask) = if i >= 8 { (24, 0xff_ffff) } else { (32, u32::MAX) };
                    let v = (self.i[i as usize] as i64 + data as i64) as u32 & mask;
                    self.i[i as usize] = bitrev(v, width);
                } else {
                    self.modify(i as usize, data as i64);
                }
            }
            Op::Stacks(s) => self.stacks(s)?,
            Op::Idle => self.idle = true,
            Op::Idle16 => return Err(Fault::Unimplemented { pc, what: "IDLE16 (ADSP-21061 only)" }),
            Op::CJump { target } => {
                // JUMP target (DB), R2=I6, I6=I7 (UM A-54).
                let t = self.target(target, pc);
                let (i6, i7) = (self.ureg_read(0x16), self.ureg_read(0x17));
                self.ureg_write(0x02, i6);
                self.ureg_write(0x16, i7);
                self.branch(t, true);
            }
            Op::RFrame => {
                // I7=I6, I6=DM(0,I6).
                let a = self.i[6];
                let l = self.load(false, a, false, ext)?;
                self.i[7] = a;
                self.apply_load(0x16, l);
            }
            Op::Unknown => return Err(undefined),
        }
        Ok(())
    }

    /// Type 20 (UM A-50). PUSH LOOP pushes LCNTR with an empty loop
    /// address; PUSH PCSTK pushes zero (the program writes the new tops
    /// through LADDR, CURLCNTR and PCSTK). Pops of empty stacks do
    /// nothing.
    fn stacks(&mut self, s: StackOps) -> Result<(), Fault> {
        if s.push_loop {
            self.push_loop(0, self.lcntr)?;
        }
        if s.pop_loop {
            self.pop_loop();
        }
        if s.push_sts {
            self.push_status()?;
        }
        if s.pop_sts {
            self.pop_status();
        }
        if s.push_pcstk {
            self.push_pc(0)?;
        }
        if s.pop_pcstk {
            self.pop_pc();
        }
        Ok(())
    }

    // --- compute -------------------------------------------------------------

    /// Run a compute operation: operands are read first, flags updated, and
    /// the register file results returned for the caller to write. `None`
    /// for an undefined encoding.
    pub(super) fn compute(&mut self, c: &Compute) -> Option<Results> {
        let mut res = Results::default();
        let r = |s: &Self, n: u8| s.r[n as usize & 15];
        match *c {
            Compute::None => {}
            Compute::Alu { op, rn, rx, ry } => {
                let (v, f) = self.alu(op, r(self, rx), r(self, ry));
                self.set_alu_flags(f);
                if let Some(v) = v {
                    res.push(rn, v);
                }
            }
            Compute::AddSub { float, ra, rs, rx, ry } => {
                let (x, y) = (r(self, rx), r(self, ry));
                let (add, sub) = if float { (AluOp::FAdd, AluOp::FSub) } else { (AluOp::Add, AluOp::Sub) };
                let (s, f1) = self.alu(add, x, y);
                let (d, f2) = self.alu(sub, x, y);
                self.set_alu_flags(f1.or(f2));
                res.push(ra, s.unwrap());
                res.push(rs, d.unwrap());
            }
            Compute::Mul { op, rn, rx, ry } => {
                if let Some(v) = self.mul(op, r(self, rx), r(self, ry)) {
                    res.push(rn, v);
                }
            }
            Compute::Shift { op, rn, rx, ry } => {
                let y = ShiftY::reg(self.rf(ry));
                if let Some(v) = self.shift(op, r(self, rx), y, r(self, rn)) {
                    res.push(rn, v);
                }
            }
            Compute::MrMove { to_mr, mr, rk } => {
                let k = if mr & 4 == 0 { self.mrf() } else { 1 - self.mrf() };
                let cur = self.mr[k];
                if to_mr {
                    let v = self.rf(rk) as i128;
                    self.mr[k] = match mr & 3 {
                        0 => (cur & !0xffff_ffff) | (v & 0xffff_ffff),
                        // MR1 is sign-extended into MR2 (UM §2.6.2.1).
                        1 => (cur & 0xffff_ffff) | ((v as u32 as i32 as i128) << 32),
                        _ => wrap80((cur & ((1i128 << 64) - 1)) | (v & 0xffff) << 64),
                    };
                } else {
                    let v = match mr & 3 {
                        0 => cur as u32,
                        1 => (cur >> 32) as u32,
                        // MR2 reads sign-extended.
                        _ => (cur >> 64) as i16 as i32 as u32,
                    };
                    res.push(rk, (v as u64) << 8);
                }
                self.astat &= !(MN | MV | MU | MI);
            }
            Compute::Multi { mul, alu, rm, ra, regs } => {
                let (xm, ym, xa, ya) = (r(self, regs.xm), r(self, regs.ym), r(self, regs.xa), r(self, regs.ya));
                let m = self.mul(mul, xm, ym);
                let (a, f) = self.alu(alu, xa, ya);
                self.set_alu_flags(f);
                if let Some(v) = m {
                    res.push(rm, v);
                }
                if let Some(v) = a {
                    res.push(ra, v);
                }
            }
            Compute::MultiAddSub { float, rm, ra, rs, regs } => {
                let (xm, ym, xa, ya) = (r(self, regs.xm), r(self, regs.ym), r(self, regs.xa), r(self, regs.ya));
                let mul = if float {
                    MulOp::FMul
                } else {
                    MulOp::Mul { acc: None, sub: false, dst: MulDst::Reg, xs: true, ys: true, frac: true, round: true }
                };
                let m = self.mul(mul, xm, ym);
                let (add, sub) = if float { (AluOp::FAdd, AluOp::FSub) } else { (AluOp::Add, AluOp::Sub) };
                let (s, f1) = self.alu(add, xa, ya);
                let (d, f2) = self.alu(sub, xa, ya);
                self.set_alu_flags(f1.or(f2));
                if let Some(v) = m {
                    res.push(rm, v);
                }
                res.push(ra, s.unwrap());
                res.push(rs, d.unwrap());
            }
            Compute::Unknown(_) => return None,
        }
        Some(res)
    }

    fn set_alu_flags(&mut self, f: AluFlags) {
        let bit = |on: bool, bit: u32| if on { bit } else { 0 };
        self.astat = (self.astat & !(AZ | AV | AN | AC | AS | AI | AF))
            | bit(f.az, AZ)
            | bit(f.av, AV)
            | bit(f.an, AN)
            | bit(f.ac, AC)
            | bit(f.as_, AS)
            | bit(f.ai, AI)
            | bit(f.af, AF);
        if f.af {
            self.stky |= bit(f.au, AUS) | bit(f.av, AVS) | bit(f.ai, AIS);
        } else if f.av {
            self.stky |= AOS;
        }
    }

    /// Shift the compare accumulation bits and record `x > y` (UM
    /// §2.5.3.9).
    fn cacc(&mut self, gt: bool) {
        let c = (self.astat >> 24) >> 1 | (gt as u32) << 7;
        self.astat = (self.astat & 0xff_ffff) | c << 24;
    }

    /// One ALU operation on 40-bit register values: the 40-bit result (if
    /// any) and its flags (UM B.2.1, §2.5.4).
    fn alu(&mut self, op: AluOp, x40: u64, y40: u64) -> (Option<u64>, AluFlags) {
        use AluOp::*;
        let (x, y) = ((x40 >> 8) as u32, (y40 >> 8) as u32);
        let fixed = |v: u32| Some((v as u64) << 8);
        let sat = self.mode1 & ALUSAT != 0;
        let ci = (self.astat & AC != 0) as u32;
        // Arithmetic with carry and overflow; saturation in ALUSAT mode.
        let arith = |(r, c, v): (u32, bool, bool)| {
            let out = if sat && v {
                if r >> 31 != 0 {
                    0x7fff_ffff
                } else {
                    0x8000_0000
                }
            } else {
                r
            };
            (fixed(out), AluFlags { av: v, ac: c, ..AluFlags::fixed(out) })
        };
        let m = self.fmode();
        let fres = |(v, f): (u64, float::Flags)| (Some(v), AluFlags::float(f));
        match op {
            Add => arith(add3(x, y, 0)),
            Sub => arith(add3(x, !y, 1)),
            AddCi => arith(add3(x, y, ci)),
            SubCi => arith(add3(x, !y, ci)),
            PassCi => arith(add3(x, 0, ci)),
            PassCiMinus1 => arith(add3(x, u32::MAX, ci)),
            Inc => arith(add3(x, 1, 0)),
            Dec => arith(add3(x, u32::MAX, 0)),
            Neg => arith(add3(!x, 0, 1)),
            Avg => {
                let s = x as i32 as i64 + y as i32 as i64;
                let carry = (x as u64 + y as u64) >> 32 != 0;
                let mut q = s >> 1;
                if self.mode1 & TRUNC == 0 && s & 1 != 0 && q & 1 != 0 {
                    q += 1;
                }
                let out = q as u32;
                (fixed(out), AluFlags { ac: carry, ..AluFlags::fixed(out) })
            }
            Comp => {
                self.cacc((x as i32) > (y as i32));
                (None, AluFlags { az: x == y, an: (x as i32) < (y as i32), ..Default::default() })
            }
            Abs => {
                let neg = (x as i32) < 0;
                let (r, _, v) = if neg { add3(!x, 0, 1) } else { (x, false, false) };
                let out = if sat && v { 0x7fff_ffff } else { r };
                (fixed(out), AluFlags { av: v, as_: neg, ..AluFlags::fixed(out) })
            }
            Pass => (fixed(x), AluFlags::fixed(x)),
            And => (fixed(x & y), AluFlags::fixed(x & y)),
            Or => (fixed(x | y), AluFlags::fixed(x | y)),
            Xor => (fixed(x ^ y), AluFlags::fixed(x ^ y)),
            Not => (fixed(!x), AluFlags::fixed(!x)),
            Min => {
                let v = (x as i32).min(y as i32) as u32;
                (fixed(v), AluFlags::fixed(v))
            }
            Max => {
                let v = (x as i32).max(y as i32) as u32;
                (fixed(v), AluFlags::fixed(v))
            }
            Clip => {
                let (xi, ay) = (x as i32 as i64, (y as i32 as i64).abs());
                let v = if xi.abs() < ay {
                    xi
                } else if xi >= 0 {
                    ay
                } else {
                    -ay
                } as u32;
                (fixed(v), AluFlags::fixed(v))
            }
            FAdd | FSub | FAvg => fres(float::add(x40, y40, op == FSub, op == FAvg, m)),
            FAbsAdd | FAbsSub => {
                let (v, f) = float::add(x40, y40, op == FAbsSub, false, m);
                let v = if f.invalid { v } else { v & !(1 << 39) };
                (Some(v), AluFlags { an: false, ..AluFlags::float(f) })
            }
            FComp => {
                let ord = float::compare(x40, y40, m);
                self.cacc(ord == Some(std::cmp::Ordering::Greater));
                let f = AluFlags {
                    az: ord == Some(std::cmp::Ordering::Equal),
                    an: ord == Some(std::cmp::Ordering::Less),
                    ai: ord.is_none(),
                    af: true,
                    ..Default::default()
                };
                (None, f)
            }
            FNeg | FAbs | FPass => {
                let mut u = float::input(x40, m);
                let sign = u.sign;
                match op {
                    FNeg => u.sign = !u.sign,
                    FAbs => u.sign = false,
                    _ => {}
                }
                let (v, f) = float::pack(u);
                (Some(v), AluFlags { as_: op == FAbs && sign, ..AluFlags::float(f) })
            }
            FRnd => fres(float::rnd(x40, m)),
            FScalb => fres(float::scalb(x40, y as i32, m)),
            FMant => {
                let sign = float::input(x40, m).sign;
                match float::mant(x40, m) {
                    Some(v) => (fixed(v), AluFlags { as_: sign, af: true, az: v == 0, ..Default::default() }),
                    None => (fixed(u32::MAX), AluFlags { as_: sign, af: true, ai: true, ..Default::default() }),
                }
            }
            FLogb => {
                let u = float::input(x40, m);
                let f = AluFlags { af: true, ..Default::default() };
                match u.class {
                    Class::Nan => (Some(float::NAN), AluFlags { ai: true, ..f }),
                    Class::Inf if sat => (fixed(0x7fff_ffff), AluFlags { av: true, ..f }),
                    Class::Inf => (Some(float::pack_inf(false)), AluFlags { av: true, ..f }),
                    Class::Zero if sat => (fixed(0x8000_0000), AluFlags { av: true, an: true, ..f }),
                    Class::Zero => (Some(float::pack_inf(true)), AluFlags { av: true, an: true, ..f }),
                    Class::Normal => {
                        let v = u.exp as u32;
                        (fixed(v), AluFlags { af: true, ..AluFlags::fixed(v) })
                    }
                }
            }
            FFix | FFixBy | FTrunc | FTruncBy => {
                let scale = if matches!(op, FFixBy | FTruncBy) { y as i32 } else { 0 };
                let round = match op {
                    FTrunc | FTruncBy => ToInt::TowardZero,
                    _ if self.mode1 & TRUNC != 0 => ToInt::Floor,
                    _ => ToInt::Nearest,
                };
                let (v, f) = float::to_int(x40, scale, round, sat, m);
                let v = match v {
                    Ok(v) => (v as u64) << 8,
                    Err(nan) => nan,
                };
                (Some(v), AluFlags::float(f))
            }
            FFloat | FFloatBy => {
                let scale = if op == FFloatBy { y as i32 } else { 0 };
                fres(float::float(x, scale, m))
            }
            FRecips => fres(float::recips(x40, m)),
            FRsqrts => fres(float::rsqrts(x40, m)),
            FCopysign | FMin | FMax | FClip => {
                let (a, b) = (float::input(x40, m), float::input(y40, m));
                if a.class == Class::Nan || b.class == Class::Nan {
                    return fres(float::nan());
                }
                let u = match op {
                    FCopysign => float::Unpacked { sign: b.sign, ..a },
                    FMin | FMax if a.class == Class::Zero && b.class == Class::Zero => {
                        // MIN(+0,-0) is -0, MAX(+0,-0) is +0 (UM B-42, B-43).
                        let sign = if op == FMin { a.sign || b.sign } else { a.sign && b.sign };
                        float::Unpacked { sign, ..a }
                    }
                    FMin => {
                        if b.to_f64() < a.to_f64() {
                            b
                        } else {
                            a
                        }
                    }
                    FMax => {
                        if b.to_f64() > a.to_f64() {
                            b
                        } else {
                            a
                        }
                    }
                    _ => {
                        if a.to_f64().abs() < b.to_f64().abs() {
                            a
                        } else {
                            float::Unpacked { sign: a.sign, ..b }
                        }
                    }
                };
                fres(float::pack(u))
            }
        }
    }

    fn mr_index(&self, m: Mr) -> usize {
        match m {
            Mr::F => self.mrf(),
            Mr::B => 1 - self.mrf(),
        }
    }

    /// Fixed-point multiplier flags for the 80-bit result `r` (UM
    /// §2.6.5): MV for the format's overflow bits (`check_mv`), MU for a
    /// fractional result that only has bits in MR0.
    fn mul_fixed_flags(&mut self, r: i128, signed: bool, frac: bool, check_mv: bool) {
        let u = r as u128 & ((1u128 << 80) - 1);
        let top_zero = |n: u32| u >> (80 - n) == 0;
        let top_ones = |n: u32| u >> (80 - n) == (1u128 << n) - 1;
        let mv = check_mv
            && if signed {
                let n = if frac { 17 } else { 49 };
                !(top_zero(n) || top_ones(n))
            } else {
                !top_zero(if frac { 16 } else { 48 })
            };
        let mu = frac && u & 0xffff_ffff != 0 && (top_zero(48) || signed && top_ones(48));
        let mut a = self.astat & !(MN | MV | MU | MI);
        for (on, bit) in [(r < 0, MN), (mv, MV), (mu, MU)] {
            if on {
                a |= bit;
            }
        }
        self.astat = a;
        if mv {
            self.stky |= MOS;
        }
    }

    fn mul_out(&mut self, dst: MulDst, r: i128, frac: bool) -> Option<u64> {
        match dst {
            MulDst::Reg => Some((((if frac { r >> 32 } else { r }) as u32) as u64) << 8),
            MulDst::Mr(m) => {
                let k = self.mr_index(m);
                self.mr[k] = r;
                None
            }
        }
    }

    /// One multiplier operation (UM B.2.2): the register file result, if
    /// any.
    fn mul(&mut self, op: MulOp, x40: u64, y40: u64) -> Option<u64> {
        let (x, y) = ((x40 >> 8) as u32, (y40 >> 8) as u32);
        match op {
            MulOp::FMul => {
                let (v, f) = float::mul(x40, y40, self.fmode());
                let mut a = self.astat & !(MN | MV | MU | MI);
                for (on, bit, sticky) in
                    [(f.neg, MN, 0), (f.overflow, MV, MVS), (f.underflow, MU, MUS), (f.invalid, MI, MIS)]
                {
                    if on {
                        a |= bit;
                        self.stky |= sticky;
                    }
                }
                self.astat = a;
                Some(v)
            }
            MulOp::Mul { acc, sub, dst, xs, ys, frac, round } => {
                let xv = if xs { x as i32 as i128 } else { x as i128 };
                let yv = if ys { y as i32 as i128 } else { y as i128 };
                let mut p = xv * yv;
                // Signed fractional products lose their redundant sign bit
                // (UM §2.6.1).
                if frac && xs && ys {
                    p <<= 1;
                }
                let base = acc.map_or(0, |m| self.mr[self.mr_index(m)]);
                let mut r = wrap80(if sub { base - p } else { base + p });
                if round {
                    r = round80(r);
                }
                self.mul_fixed_flags(r, xs || ys, frac, true);
                self.mul_out(dst, r, frac)
            }
            MulOp::Sat { src, dst, signed, frac } => {
                let cur = self.mr[self.mr_index(src)];
                let (min, max): (i128, i128) = match (signed, frac) {
                    (true, true) => (-(1 << 63), (1 << 63) - 1),
                    (true, false) => (-(1 << 31), (1 << 31) - 1),
                    (false, true) => (0, (1 << 64) - 1),
                    (false, false) => (0, 0xffff_ffff),
                };
                let v = if signed { cur } else { cur & ((1 << 80) - 1) };
                let r = wrap80(v.clamp(min, max));
                self.mul_fixed_flags(r, signed, frac, false);
                self.mul_out(dst, r, frac)
            }
            MulOp::Rnd { src, dst, signed } => {
                let r = round80(self.mr[self.mr_index(src)]);
                self.mul_fixed_flags(r, signed, true, true);
                self.mul_out(dst, r, true)
            }
            MulOp::Clear(m) => {
                let k = self.mr_index(m);
                self.mr[k] = 0;
                self.astat &= !(MN | MV | MU | MI);
                None
            }
        }
    }

    /// One shifter operation (UM B.2.3, §2.7): `n40` is Rn's old value
    /// (the OR forms' third input).
    fn shift(&mut self, op: ShiftOp, x40: u64, y: ShiftY, n40: u64) -> Option<u64> {
        use ShiftOp::*;
        let (x, n) = ((x40 >> 8) as u32, (n40 >> 8) as u32);
        let mut sv = false;
        let mut ss = false;
        let mut sz_override = None;
        let lshift = |v: u32, s: i64| -> (u32, bool) {
            match s {
                32.. => (0, v != 0),
                1..=31 => (v << s, v >> (32 - s) != 0),
                ..=-32 => (0, false),
                -31..=-1 => (v >> -s, false),
                _ => (v, false),
            }
        };
        let ashift = |v: u32, s: i64| -> (u32, bool) {
            match s {
                32.. => (0, v != 0),
                1..=31 => {
                    let r = v << s;
                    (r, ((r as i32) >> s) as u32 != v)
                }
                ..=-32 => (((v as i32) >> 31) as u32, false),
                -31..=-1 => (((v as i32) >> -s) as u32, false),
                _ => (v, false),
            }
        };
        let field = |len6: u32| -> u64 { (1u64 << len6.min(32)) - 1 };
        let out: Option<u32> = match op {
            Lshift | OrLshift | Ashift | OrAshift => {
                let (r, ov) = if matches!(op, Lshift | OrLshift) { lshift(x, y.shift) } else { ashift(x, y.shift) };
                sv = ov;
                Some(if matches!(op, OrLshift | OrAshift) { n | r } else { r })
            }
            Rot => Some(x.rotate_left(y.shift.rem_euclid(32) as u32)),
            Bclr | Bset | Btgl | Btst => {
                if y.pos > 31 {
                    sv = true;
                    if op == Btst {
                        sz_override = Some(true);
                        None
                    } else {
                        Some(x)
                    }
                } else {
                    let bit = 1u32 << y.pos;
                    match op {
                        Bclr => Some(x & !bit),
                        Bset => Some(x | bit),
                        Btgl => Some(x ^ bit),
                        _ => {
                            sz_override = Some(x & bit == 0);
                            None
                        }
                    }
                }
            }
            Fdep | OrFdep | FdepSe | OrFdepSe => {
                let (bit6, len6) = (y.bit6, y.len6);
                let mut f = x as u64 & field(len6);
                // Sign-extend unless the field's MSB lands off-scale left.
                if matches!(op, FdepSe | OrFdepSe) && len6 > 0 && bit6 + len6 <= 32 {
                    f = sext_from(f, len6);
                }
                let dep = (f << bit6) as u32;
                sv = bit6 + len6 > 32;
                Some(if matches!(op, OrFdep | OrFdepSe) { n | dep } else { dep })
            }
            Fext | FextSe => {
                let (bit6, len6) = (y.bit6, y.len6);
                let mut f = (x as u64 >> bit6) & field(len6);
                if op == FextSe && len6 > 0 && bit6 + len6 <= 32 {
                    f = sext_from(f, len6);
                }
                sv = bit6 + len6 > 32;
                Some(f as u32)
            }
            Exp | ExpEx => {
                let neg = (x as i32) < 0;
                let lead = if neg { (!x).leading_zeros() } else { x.leading_zeros() } as i32;
                let av = self.astat & AV != 0;
                let e = if op == ExpEx && av { 1 } else { 1 - lead };
                ss = if op == ExpEx { av ^ neg } else { neg };
                Some(e as u32)
            }
            Leftz | Lefto => {
                let c = if op == Leftz { x.leading_zeros() } else { x.leading_ones() };
                sv = c == 32;
                Some(c)
            }
            Fpack => {
                let (v, ov) = float::fpack(x40);
                sv = ov;
                sz_override = Some(false);
                Some(v)
            }
            Funpack => {
                sz_override = Some(false);
                Some(float::funpack(x & 0xffff))
            }
        };
        let sz = sz_override.unwrap_or(out == Some(0));
        let mut a = self.astat & !(SV | SZ | SS);
        for (on, bit) in [(sv, SV), (sz, SZ), (ss, SS)] {
            if on {
                a |= bit;
            }
        }
        self.astat = a;
        out.map(|v| (v as u64) << 8)
    }
}
