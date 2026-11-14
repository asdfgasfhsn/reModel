//! The instructions the back ends translate, as operations on the register
//! file: shared by the static recompiler ([`codegen`](crate::codegen),
//! which prints them as Rust) and the JIT (`crates/jit`, which emits them
//! as Cranelift IR). [`lower`] says what one instruction does; `None`
//! leaves it to the interpreter's [`Cpu::exec`](crate::cpu::Cpu::exec).
//!
//! The semantics are the interpreter's (`cpu.rs`), operation for
//! operation: a back end must give the same registers, flags and memory
//! accesses in the same order.

use crate::{Insn, MemRef, Operand};

/// A register or literal source operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Src {
    Reg(u8),
    Lit(u32),
}

/// A two-source register operation, `dst = f(src2, src1)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alu {
    /// `notbit`: src2 ^ bit(src1)
    NotBit,
    And,
    /// src2 & !src1
    AndNot,
    /// `setbit`: src2 | bit(src1)
    SetBit,
    /// !src2 & src1
    NotAnd,
    Xor,
    Or,
    Nor,
    Xnor,
    /// src2 | !src1
    OrNot,
    /// `clrbit`: src2 & !bit(src1)
    ClrBit,
    /// !src2 | src1
    NotOr,
    Nand,
    Add,
    /// src2 - src1
    Sub,
    /// `shro`: src2 >> src1, 0 from 32
    Shro,
    /// `shri`: arithmetic, the count capped at 31
    Shri,
    /// `shlo`/`shli`: src2 << src1, 0 from 32
    Shl,
    /// `rotate`: src2 rotated left by src1 & 31
    Rotate,
    /// `mulo`/`muli`: the low word
    Mul,
}

/// Memory access width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    Byte,
    Short,
}

/// What one instruction does, for the ones the back ends translate.
/// `next` is the address of the instruction after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// `b`: IP = target.
    Jump(u32),
    /// `call`: a local call to target, returning to next.
    Call { target: u32, next: u32 },
    /// `bal`: g14 = next, IP = target.
    Link { target: u32, next: u32 },
    /// `b<cc>`: IP = target if the condition (mask) holds, else next.
    Branch { mask: u32, target: u32, next: u32 },
    /// `ret`.
    Ret,
    /// `test<cc>`: dst = condition (mask) holds.
    Test { mask: u32, dst: u8 },
    /// `bbc`/`bbs`: cc = bit src1 of src2 set ? equal : 0; branch if it
    /// is `set`.
    BitBranch { bit: Src, src: Src, set: bool, target: u32, next: u32 },
    /// `cmpob<cc>`/`cmpib<cc>`: compare src1 with src2, branch on mask.
    CmpBranch { signed: bool, a: Src, b: Src, mask: u32, target: u32, next: u32 },
    /// `bx`: IP = effective address.
    JumpInd(MemRef),
    /// `balx`: dst = next, IP = effective address (computed first).
    LinkInd { ea: MemRef, dst: u8, next: u32 },
    /// `callx`.
    CallInd { ea: MemRef, next: u32 },
    /// `lda`: dst = effective address.
    Lda { ea: MemRef, dst: u8 },
    /// `ldob`/`ldib`/`ldos`/`ldis`.
    LoadNarrow { width: Width, signed: bool, ea: MemRef, dst: u8 },
    /// `stob`/`stib`/`stos`/`stis`.
    StoreNarrow { width: Width, ea: MemRef, src: u8 },
    /// `ld`/`ldl`/`ldt`/`ldq`: `n` words into dst, dst+1, ... (mod 32).
    Load { n: u8, ea: MemRef, dst: u8 },
    /// `st`/`stl`/`stt`/`stq`.
    Store { n: u8, ea: MemRef, src: u8 },
    /// dst = f(src2, src1).
    Alu { op: Alu, a: Src, b: Src, dst: u8 },
    /// `not`: dst = !src1.
    Not { a: Src, dst: u8 },
    /// `cmpo`/`cmpi`: cc from src1 against src2.
    Compare { signed: bool, a: Src, b: Src },
    /// `chkbit`: cc = bit src1 of src2 set ? equal : 0.
    ChkBit { bit: Src, src: Src },
    /// `mov`/`movl`/`movt`/`movq`: `n` registers from src (a literal
    /// fills the first, zeroes the rest).
    Mov { n: u8, src: Src, dst: u8 },
}

fn src(op: Option<&Operand>) -> Option<Src> {
    match op? {
        Operand::Reg(r) => Some(Src::Reg(*r)),
        Operand::Lit(v) => Some(Src::Lit(*v)),
        _ => None,
    }
}

fn reg(op: Option<&Operand>) -> Option<u8> {
    match op? {
        Operand::Reg(r) => Some(*r),
        _ => None,
    }
}

/// What `i` does, or `None` to leave it to the interpreter (the rare
/// instructions, floating point, anything with side effects beyond
/// registers and plain memory accesses).
pub fn lower(i: &Insn) -> Option<Op> {
    let ops = &i.operands;
    let op = i.word >> 24;
    let next = i.next();
    Some(match op {
        0x08 | 0x09 | 0x0b | 0x10..=0x17 => {
            let Some(Operand::Target(t)) = ops.first() else { return None };
            let target = *t;
            match op {
                0x08 => Op::Jump(target),
                0x09 => Op::Call { target, next },
                0x0b => Op::Link { target, next },
                _ => Op::Branch { mask: op & 7, target, next },
            }
        }
        0x0a => Op::Ret,
        0x20..=0x27 => Op::Test { mask: op & 7, dst: reg(ops.first())? },
        0x30..=0x3f => {
            let (a, b) = (src(ops.first())?, src(ops.get(1))?);
            let Some(Operand::Target(t)) = ops.get(2) else { return None };
            let target = *t;
            match op {
                0x30 | 0x37 => Op::BitBranch { bit: a, src: b, set: op == 0x37, target, next },
                0x31..=0x36 => Op::CmpBranch { signed: false, a, b, mask: op & 7, target, next },
                _ => Op::CmpBranch { signed: true, a, b, mask: op & 7, target, next },
            }
        }
        0x58..=0x7f => return lower_reg(i),
        0x80..=0xcf => {
            let ea = match ops.iter().find(|o| matches!(o, Operand::Mem(_))) {
                Some(Operand::Mem(m)) => *m,
                _ => return None,
            };
            let load = || reg(ops.get(1));
            let store = || reg(ops.first());
            match op {
                0x84 => Op::JumpInd(ea),
                0x85 => Op::LinkInd { ea, dst: load()?, next },
                0x86 => Op::CallInd { ea, next },
                0x8c => Op::Lda { ea, dst: load()? },
                0x80 => Op::LoadNarrow { width: Width::Byte, signed: false, ea, dst: load()? },
                0xc0 => Op::LoadNarrow { width: Width::Byte, signed: true, ea, dst: load()? },
                0x88 => Op::LoadNarrow { width: Width::Short, signed: false, ea, dst: load()? },
                0xc8 => Op::LoadNarrow { width: Width::Short, signed: true, ea, dst: load()? },
                0x82 | 0xc2 => Op::StoreNarrow { width: Width::Byte, ea, src: store()? },
                0x8a | 0xca => Op::StoreNarrow { width: Width::Short, ea, src: store()? },
                0x90 | 0x98 | 0xa0 | 0xb0 | 0x92 | 0x9a | 0xa2 | 0xb2 => {
                    let n = match op & !2 {
                        0x90 => 1,
                        0x98 => 2,
                        0xa0 => 3,
                        _ => 4,
                    };
                    if op & 2 == 0 {
                        Op::Load { n, ea, dst: load()? }
                    } else {
                        Op::Store { n, ea, src: store()? }
                    }
                }
                _ => return None,
            }
        }
        _ => return None,
    })
}

fn lower_reg(i: &Insn) -> Option<Op> {
    let ops = &i.operands;
    let full = ((i.word >> 24) << 4) | ((i.word >> 7) & 0xf);
    let alu = |op: Alu| Some(Op::Alu { op, a: src(ops.first())?, b: src(ops.get(1))?, dst: reg(ops.get(2))? });
    match full {
        0x580 => alu(Alu::NotBit),
        0x581 => alu(Alu::And),
        0x582 => alu(Alu::AndNot),
        0x583 => alu(Alu::SetBit),
        0x584 => alu(Alu::NotAnd),
        0x586 => alu(Alu::Xor),
        0x587 => alu(Alu::Or),
        0x588 => alu(Alu::Nor),
        0x589 => alu(Alu::Xnor),
        0x58a => Some(Op::Not { a: src(ops.first())?, dst: reg(ops.get(1))? }),
        0x58b => alu(Alu::OrNot),
        0x58c => alu(Alu::ClrBit),
        0x58d => alu(Alu::NotOr),
        0x58e => alu(Alu::Nand),
        0x590 | 0x591 => alu(Alu::Add),
        0x592 | 0x593 => alu(Alu::Sub),
        0x598 => alu(Alu::Shro),
        0x59b => alu(Alu::Shri),
        0x59c | 0x59e => alu(Alu::Shl),
        0x59d => alu(Alu::Rotate),
        0x5a0 | 0x5a1 => Some(Op::Compare { signed: full == 0x5a1, a: src(ops.first())?, b: src(ops.get(1))? }),
        0x5ae => Some(Op::ChkBit { bit: src(ops.first())?, src: src(ops.get(1))? }),
        0x5cc | 0x5dc | 0x5ec | 0x5fc => {
            let n = ((full - 0x5cc) >> 4) as u8 + 1;
            let dst = reg(ops.get(1))?;
            Some(Op::Mov { n, src: src(ops.first())?, dst })
        }
        0x701 | 0x741 => alu(Alu::Mul),
        _ => None,
    }
}

impl Op {
    /// True if it reads or writes memory or calls into the machine (so the
    /// machine must know the instruction's address first: `Host::at`).
    pub fn touches_bus(&self) -> bool {
        matches!(
            self,
            Op::Call { .. }
                | Op::Ret
                | Op::CallInd { .. }
                | Op::LoadNarrow { .. }
                | Op::StoreNarrow { .. }
                | Op::Load { .. }
                | Op::Store { .. }
        )
    }

    /// True if it writes memory.
    pub fn stores(&self) -> bool {
        matches!(self, Op::Call { .. } | Op::CallInd { .. } | Op::StoreNarrow { .. } | Op::Store { .. })
    }
}
