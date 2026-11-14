//! Intel i960 KA/KB instruction decoder.
//!
//! Formats (from the 80960KB programmer's reference):
//!   CTRL  op[31:24] 0x00-0x1f, disp[23:2] signed, T[1]
//!   COBR  op[31:24] 0x20-0x3f, src1[23:19] src2[18:14] M1[13] disp[12:2] T[1] S2[0]
//!   REG   op[31:24] 0x58-0x7f, dst[23:19] src2[18:14] M3[13] M2[12] M1[11]
//!         opx[10:7] S2[6] S1[5] src1[4:0]; full opcode = op<<4 | opx
//!   MEM   op[31:24] 0x80-0xcf, reg[23:19] abase[18:14]
//!         MEMA (bit12=0): mode[13] offset[11:0]
//!         MEMB (bit12=1): mode[13:10] scale[9:7] index[4:0] (+ 32-bit disp word)

pub mod codegen;
pub mod cpu;
pub mod lower;

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operand {
    Reg(u8),
    /// Floating-point register fp0-fp3.
    FpReg(u8),
    Lit(u32),
    FpLit(&'static str),
    Mem(MemRef),
    Target(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemRef {
    Abs(u32),
    Base { abase: u8, disp: u32 },
    IpRel { target: u32 },
    Index { disp: u32, index: u8, scale: u8 },
    BaseIndex { abase: u8, disp: u32, index: u8, scale: u8 },
}

impl MemRef {
    /// Absolute address, if the operand doesn't depend on registers.
    pub fn static_addr(self) -> Option<u32> {
        match self {
            MemRef::Abs(a) | MemRef::IpRel { target: a } => Some(a),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    /// Falls through to the next instruction.
    Next,
    Jump(u32),
    /// Conditional branch: target or fall through.
    Branch(u32),
    Call(u32),
    /// `bal`: branch and link (leaf call, returns through a register).
    Link(u32),
    /// `calls n`: system procedure call through the sysproc table.
    SysCall,
    IndirectJump,
    IndirectCall,
    IndirectLink,
    Return,
    /// Conditional fault: may trap, otherwise falls through.
    CondFault,
    Invalid,
}

#[derive(Clone, Debug)]
pub struct Insn {
    pub addr: u32,
    pub size: u32,
    pub word: u32,
    pub mnemonic: &'static str,
    pub operands: Vec<Operand>,
    pub flow: Flow,
}

impl Insn {
    pub fn next(&self) -> u32 {
        self.addr.wrapping_add(self.size)
    }

    pub fn view(&self) -> InsnRef<'_> {
        InsnRef {
            addr: self.addr,
            size: self.size,
            word: self.word,
            mnemonic: self.mnemonic,
            operands: &self.operands,
            flow: self.flow,
        }
    }
}

/// A borrowed [`Insn`], what the interpreter executes. Recompiled code
/// writes these as constants.
#[derive(Clone, Copy, Debug)]
pub struct InsnRef<'a> {
    pub addr: u32,
    pub size: u32,
    pub word: u32,
    pub mnemonic: &'static str,
    pub operands: &'a [Operand],
    pub flow: Flow,
}

impl InsnRef<'_> {
    pub fn next(&self) -> u32 {
        self.addr.wrapping_add(self.size)
    }
}

pub fn reg_name(r: u8) -> &'static str {
    const N: [&str; 32] = [
        "pfp", "sp", "rip", "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10", "r11", "r12", "r13", "r14", "r15", "g0",
        "g1", "g2", "g3", "g4", "g5", "g6", "g7", "g8", "g9", "g10", "g11", "g12", "g13", "g14", "fp",
    ];
    N[(r & 31) as usize]
}

fn sext(v: u32, bits: u32) -> u32 {
    let s = 32 - bits;
    (((v << s) as i32) >> s) as u32
}

fn ctrl_name(op: u32) -> Option<&'static str> {
    Some(match op {
        0x08 => "b",
        0x09 => "call",
        0x0a => "ret",
        0x0b => "bal",
        0x10 => "bno",
        0x11 => "bg",
        0x12 => "be",
        0x13 => "bge",
        0x14 => "bl",
        0x15 => "bne",
        0x16 => "ble",
        0x17 => "bo",
        0x18 => "faultno",
        0x19 => "faultg",
        0x1a => "faulte",
        0x1b => "faultge",
        0x1c => "faultl",
        0x1d => "faultne",
        0x1e => "faultle",
        0x1f => "faulto",
        _ => return None,
    })
}

fn cobr_name(op: u32) -> Option<&'static str> {
    Some(match op {
        0x20 => "testno",
        0x21 => "testg",
        0x22 => "teste",
        0x23 => "testge",
        0x24 => "testl",
        0x25 => "testne",
        0x26 => "testle",
        0x27 => "testo",
        0x30 => "bbc",
        0x31 => "cmpobg",
        0x32 => "cmpobe",
        0x33 => "cmpobge",
        0x34 => "cmpobl",
        0x35 => "cmpobne",
        0x36 => "cmpoble",
        0x37 => "bbs",
        0x38 => "cmpibno",
        0x39 => "cmpibg",
        0x3a => "cmpibe",
        0x3b => "cmpibge",
        0x3c => "cmpibl",
        0x3d => "cmpibne",
        0x3e => "cmpible",
        0x3f => "cmpibo",
        _ => return None,
    })
}

fn mem_name(op: u32) -> Option<&'static str> {
    Some(match op {
        0x80 => "ldob",
        0x82 => "stob",
        0x84 => "bx",
        0x85 => "balx",
        0x86 => "callx",
        0x88 => "ldos",
        0x8a => "stos",
        0x8c => "lda",
        0x90 => "ld",
        0x92 => "st",
        0x98 => "ldl",
        0x9a => "stl",
        0xa0 => "ldt",
        0xa2 => "stt",
        0xb0 => "ldq",
        0xb2 => "stq",
        0xc0 => "ldib",
        0xc2 => "stib",
        0xc8 => "ldis",
        0xca => "stis",
        _ => return None,
    })
}

/// Operand shape of a REG-format instruction.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// src1, src2, dst
    S1S2D,
    /// src1, dst
    S1D,
    /// src1, src2
    S1S2,
    /// src1
    S1,
    /// no operands
    NoOps,
}

/// Returns (mnemonic, shape, is_float).
fn reg_info(op: u32) -> Option<(&'static str, Shape, bool)> {
    use Shape::*;
    let i = |n, s| Some((n, s, false));
    let f = |n, s| Some((n, s, true));
    match op {
        0x580 => i("notbit", S1S2D),
        0x581 => i("and", S1S2D),
        0x582 => i("andnot", S1S2D),
        0x583 => i("setbit", S1S2D),
        0x584 => i("notand", S1S2D),
        0x586 => i("xor", S1S2D),
        0x587 => i("or", S1S2D),
        0x588 => i("nor", S1S2D),
        0x589 => i("xnor", S1S2D),
        0x58a => i("not", S1D),
        0x58b => i("ornot", S1S2D),
        0x58c => i("clrbit", S1S2D),
        0x58d => i("notor", S1S2D),
        0x58e => i("nand", S1S2D),
        0x58f => i("alterbit", S1S2D),
        0x590 => i("addo", S1S2D),
        0x591 => i("addi", S1S2D),
        0x592 => i("subo", S1S2D),
        0x593 => i("subi", S1S2D),
        0x598 => i("shro", S1S2D),
        0x59a => i("shrdi", S1S2D),
        0x59b => i("shri", S1S2D),
        0x59c => i("shlo", S1S2D),
        0x59d => i("rotate", S1S2D),
        0x59e => i("shli", S1S2D),
        0x5a0 => i("cmpo", S1S2),
        0x5a1 => i("cmpi", S1S2),
        0x5a2 => i("concmpo", S1S2),
        0x5a3 => i("concmpi", S1S2),
        0x5a4 => i("cmpinco", S1S2D),
        0x5a5 => i("cmpinci", S1S2D),
        0x5a6 => i("cmpdeco", S1S2D),
        0x5a7 => i("cmpdeci", S1S2D),
        0x5ac => i("scanbyte", S1S2),
        0x5ae => i("chkbit", S1S2),
        0x5b0 => i("addc", S1S2D),
        0x5b2 => i("subc", S1S2D),
        0x5cc => i("mov", S1D),
        0x5dc => i("movl", S1D),
        0x5ec => i("movt", S1D),
        0x5fc => i("movq", S1D),
        0x600 => i("synmov", S1S2),
        0x601 => i("synmovl", S1S2),
        0x602 => i("synmovq", S1S2),
        0x610 => i("atmod", S1S2D),
        0x612 => i("atadd", S1S2D),
        0x615 => i("synld", S1D),
        0x640 => i("spanbit", S1D),
        0x641 => i("scanbit", S1D),
        0x642 => i("daddc", S1S2D),
        0x643 => i("dsubc", S1S2D),
        0x644 => i("dmovt", S1D),
        0x645 => i("modac", S1S2D),
        0x650 => i("modify", S1S2D),
        0x651 => i("extract", S1S2D),
        0x654 => i("modtc", S1S2D),
        0x655 => i("modpc", S1S2D),
        0x660 => i("calls", S1),
        0x66b => i("mark", NoOps),
        0x66c => i("fmark", NoOps),
        0x66d => i("flushreg", NoOps),
        0x66f => i("syncf", NoOps),
        0x670 => i("emul", S1S2D),
        0x671 => i("ediv", S1S2D),
        0x674 => f("cvtir", S1D),
        0x675 => f("cvtilr", S1D),
        0x676 => f("scalerl", S1S2D),
        0x677 => f("scaler", S1S2D),
        0x680 => f("atanr", S1S2D),
        0x681 => f("logepr", S1S2D),
        0x682 => f("logr", S1S2D),
        0x683 => f("remr", S1S2D),
        0x684 => f("cmpor", S1S2),
        0x685 => f("cmpr", S1S2),
        0x688 => f("sqrtr", S1D),
        0x689 => f("expr", S1D),
        0x68a => f("logbnr", S1D),
        0x68b => f("roundr", S1D),
        0x68c => f("sinr", S1D),
        0x68d => f("cosr", S1D),
        0x68e => f("tanr", S1D),
        0x68f => f("classr", S1),
        0x690 => f("atanrl", S1S2D),
        0x691 => f("logeprl", S1S2D),
        0x692 => f("logrl", S1S2D),
        0x693 => f("remrl", S1S2D),
        0x694 => f("cmporl", S1S2),
        0x695 => f("cmprl", S1S2),
        0x698 => f("sqrtrl", S1D),
        0x699 => f("exprl", S1D),
        0x69a => f("logbnrl", S1D),
        0x69b => f("roundrl", S1D),
        0x69c => f("sinrl", S1D),
        0x69d => f("cosrl", S1D),
        0x69e => f("tanrl", S1D),
        0x69f => f("classrl", S1),
        0x6c0 => f("cvtri", S1D),
        0x6c1 => f("cvtril", S1D),
        0x6c2 => f("cvtzri", S1D),
        0x6c3 => f("cvtzril", S1D),
        0x6c9 => f("movr", S1D),
        0x6d9 => f("movrl", S1D),
        0x6e1 => f("movre", S1D),
        0x6e2 => f("cpysre", S1S2D),
        0x6e3 => f("cpyrsre", S1S2D),
        0x701 => i("mulo", S1S2D),
        0x708 => i("remo", S1S2D),
        0x70b => i("divo", S1S2D),
        0x741 => i("muli", S1S2D),
        0x748 => i("remi", S1S2D),
        0x749 => i("modi", S1S2D),
        0x74b => i("divi", S1S2D),
        0x78b => f("divr", S1S2D),
        0x78c => f("mulr", S1S2D),
        0x78d => f("subr", S1S2D),
        0x78f => f("addr", S1S2D),
        0x79b => f("divrl", S1S2D),
        0x79c => f("mulrl", S1S2D),
        0x79d => f("subrl", S1S2D),
        0x79f => f("addrl", S1S2D),
        _ => None,
    }
}

/// Integer conversions between int and fp registers: only one side is float.
fn float_src_only(op: u32) -> bool {
    matches!(op, 0x6c0..=0x6c3)
}
fn float_dst_only(op: u32) -> bool {
    matches!(op, 0x674 | 0x675)
}

fn reg_operand(field: u8, m: bool, float: bool) -> Operand {
    match (m, float) {
        (false, _) => Operand::Reg(field),
        (true, false) => Operand::Lit(field as u32),
        (true, true) => match field {
            0..=3 => Operand::FpReg(field),
            16 => Operand::FpLit("0.0"),
            22 => Operand::FpLit("1.0"),
            _ => Operand::Lit(field as u32),
        },
    }
}

/// Decode one instruction. `fetch(addr)` returns the 32-bit word at `addr`.
pub fn decode(addr: u32, fetch: impl Fn(u32) -> Option<u32>) -> Insn {
    let invalid =
        |word| Insn { addr, size: 4, word, mnemonic: ".word", operands: vec![Operand::Lit(word)], flow: Flow::Invalid };
    let Some(word) = fetch(addr) else {
        return Insn { addr, size: 4, word: 0, mnemonic: "<oob>", operands: vec![], flow: Flow::Invalid };
    };
    let op = word >> 24;
    let r_dst = ((word >> 19) & 31) as u8;
    let r_s2 = ((word >> 14) & 31) as u8;

    match op {
        0x00..=0x1f => {
            let Some(name) = ctrl_name(op) else { return invalid(word) };
            let target = addr.wrapping_add(sext(word & 0x00ff_fffc, 24));
            let (operands, flow) = match op {
                0x08 => (vec![Operand::Target(target)], Flow::Jump(target)),
                0x09 => (vec![Operand::Target(target)], Flow::Call(target)),
                0x0a => (vec![], Flow::Return),
                0x0b => (vec![Operand::Target(target)], Flow::Link(target)),
                0x10..=0x17 => (vec![Operand::Target(target)], Flow::Branch(target)),
                _ => (vec![], Flow::CondFault),
            };
            Insn { addr, size: 4, word, mnemonic: name, operands, flow }
        }
        0x20..=0x3f => {
            let Some(name) = cobr_name(op) else { return invalid(word) };
            let m1 = word & (1 << 13) != 0;
            if op < 0x28 {
                // test*: src1 field is the destination register.
                return Insn {
                    addr,
                    size: 4,
                    word,
                    mnemonic: name,
                    operands: vec![Operand::Reg(r_dst)],
                    flow: Flow::Next,
                };
            }
            let target = addr.wrapping_add(sext(word & 0x1ffc, 13));
            let src1 = if m1 { Operand::Lit(r_dst as u32) } else { Operand::Reg(r_dst) };
            Insn {
                addr,
                size: 4,
                word,
                mnemonic: name,
                operands: vec![src1, Operand::Reg(r_s2), Operand::Target(target)],
                flow: Flow::Branch(target),
            }
        }
        0x58..=0x7f => {
            let full = (op << 4) | ((word >> 7) & 0xf);
            let Some((name, shape, float)) = reg_info(full) else { return invalid(word) };
            let m1 = word & (1 << 11) != 0;
            let m2 = word & (1 << 12) != 0;
            let m3 = word & (1 << 13) != 0;
            let s1f = float && !float_dst_only(full);
            let df = float && !float_src_only(full);
            let s1 = reg_operand((word & 31) as u8, m1, s1f);
            let s2 = reg_operand(r_s2, m2, float);
            let d = if df && m3 && r_dst < 4 { Operand::FpReg(r_dst) } else { Operand::Reg(r_dst) };
            let operands = match shape {
                Shape::S1S2D => vec![s1, s2, d],
                Shape::S1D => vec![s1, d],
                Shape::S1S2 => vec![s1, s2],
                Shape::S1 => vec![s1],
                Shape::NoOps => vec![],
            };
            let flow = if full == 0x660 { Flow::SysCall } else { Flow::Next };
            Insn { addr, size: 4, word, mnemonic: name, operands, flow }
        }
        0x80..=0xcf => {
            let Some(name) = mem_name(op) else { return invalid(word) };
            let abase = r_s2;
            let (mem, size) = if word & (1 << 12) == 0 {
                let off = word & 0xfff;
                if word & (1 << 13) == 0 {
                    (MemRef::Abs(off), 4)
                } else {
                    (MemRef::Base { abase, disp: off }, 4)
                }
            } else {
                let mode = (word >> 10) & 0xf;
                let scale = 1u8 << ((word >> 7) & 7);
                let index = (word & 31) as u8;
                let needs_disp = mode >= 0xc || mode == 0x5;
                let disp = if needs_disp {
                    match fetch(addr.wrapping_add(4)) {
                        Some(d) => d,
                        None => return invalid(word),
                    }
                } else {
                    0
                };
                let size = if needs_disp { 8 } else { 4 };
                let m = match mode {
                    0x4 => MemRef::Base { abase, disp: 0 },
                    0x5 => MemRef::IpRel { target: addr.wrapping_add(disp).wrapping_add(8) },
                    0x7 => MemRef::BaseIndex { abase, disp: 0, index, scale },
                    0xc => MemRef::Abs(disp),
                    0xd => MemRef::Base { abase, disp },
                    0xe => MemRef::Index { disp, index, scale },
                    0xf => MemRef::BaseIndex { abase, disp, index, scale },
                    _ => return invalid(word),
                };
                (m, size)
            };
            let reg = Operand::Reg(r_dst);
            let m = Operand::Mem(mem);
            let is_store = matches!(op, 0x82 | 0x8a | 0x92 | 0x9a | 0xa2 | 0xb2 | 0xc2 | 0xca);
            let (operands, flow) = match op {
                0x84 => (vec![m], branch_flow(mem, Flow::Jump, Flow::IndirectJump)),
                0x85 => (vec![m, reg], branch_flow(mem, Flow::Link, Flow::IndirectLink)),
                0x86 => (vec![m], branch_flow(mem, Flow::Call, Flow::IndirectCall)),
                _ if is_store => (vec![reg, m], Flow::Next),
                _ => (vec![m, reg], Flow::Next),
            };
            Insn { addr, size, word, mnemonic: name, operands, flow }
        }
        _ => invalid(word),
    }
}

fn branch_flow(mem: MemRef, direct: fn(u32) -> Flow, indirect: Flow) -> Flow {
    // For bx/balx/callx the effective address *is* the target.
    match mem.static_addr() {
        Some(t) => direct(t),
        None => indirect,
    }
}

impl fmt::Display for Operand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Operand::Reg(r) => f.write_str(reg_name(r)),
            Operand::FpReg(r) => write!(f, "fp{r}"),
            Operand::Lit(v) => write!(f, "{v}"),
            Operand::FpLit(s) => f.write_str(s),
            Operand::Target(t) => write!(f, "{t:#x}"),
            Operand::Mem(m) => write!(f, "{m}"),
        }
    }
}

impl fmt::Display for MemRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            MemRef::Abs(a) => write!(f, "{a:#x}"),
            MemRef::Base { abase, disp: 0 } => write!(f, "({})", reg_name(abase)),
            MemRef::Base { abase, disp } => write!(f, "{disp:#x}({})", reg_name(abase)),
            MemRef::IpRel { target } => write!(f, "{target:#x}(ip)"),
            MemRef::Index { disp, index, scale } => write!(f, "{disp:#x}[{}*{scale}]", reg_name(index)),
            MemRef::BaseIndex { abase, disp, index, scale } => {
                if disp != 0 {
                    write!(f, "{disp:#x}")?;
                }
                write!(f, "({})[{}*{scale}]", reg_name(abase), reg_name(index))
            }
        }
    }
}

impl fmt::Display for Insn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.mnemonic)?;
        for (i, o) in self.operands.iter().enumerate() {
            f.write_str(if i == 0 { " " } else { ", " })?;
            write!(f, "{o}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(words: &[u32]) -> Insn {
        decode(0x1000, |a| words.get(((a - 0x1000) / 4) as usize).copied())
    }

    #[test]
    fn ctrl_branch_backwards() {
        // b -8
        let i = one(&[0x08ff_fff8]);
        assert_eq!(i.to_string(), "b 0xff8");
        assert_eq!(i.flow, Flow::Jump(0xff8));
    }

    #[test]
    fn reg_literal_add() {
        // addo 4, sp, sp: op 0x59 opx 0 m1 src1=4 src2=sp(1) dst=sp(1)
        let w = (0x59 << 24) | (1 << 19) | (1 << 14) | (1 << 11) | 4;
        assert_eq!(one(&[w]).to_string(), "addo 4, sp, sp");
    }

    #[test]
    fn memb_absolute_load() {
        // ld 0x500000, g0: op 0x90, dst g0=16, MEMB mode 0xc
        let w = (0x90 << 24) | (16 << 19) | (0xc << 10) | (1 << 12);
        let i = one(&[w, 0x0050_0000]);
        assert_eq!(i.size, 8);
        assert_eq!(i.to_string(), "ld 0x500000, g0");
    }

    #[test]
    fn cobr_compare_branch() {
        // cmpobe 0, g0, +16 (m1 literal)
        let w = (0x32 << 24) | (16 << 14) | (1 << 13) | 16;
        assert_eq!(one(&[w]).to_string(), "cmpobe 0, g0, 0x1010");
    }
}
