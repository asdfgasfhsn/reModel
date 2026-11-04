//! Fujitsu MB86235 ("TGPx4") geometry processor: instruction decoder and
//! disassembler.
//!
//! Encoding facts and their provenance are in `docs/hw/mb86235.md`. Where
//! sources disagree, the choice made here is the one the uploaded Top
//! Skater program supports (see that document's "Checks against topskatr").

use std::fmt;

fn bits(w: u64, hi: u32, lo: u32) -> u32 {
    ((w >> lo) & ((1u64 << (hi - lo + 1)) - 1)) as u32
}

fn sext(v: u32, width: u32) -> i32 {
    let s = 32 - width;
    ((v << s) as i32) >> s
}

/// Program memory size in words; the PC is 12 bits.
pub const PROGRAM_WORDS: u32 = 4096;

/// Register code (6 bits) used by transfers.
pub fn reg_name(code: u32) -> String {
    let i = code & 7;
    match code & 0x3f {
        0x00..=0x07 => format!("MA{i}"),
        0x08..=0x0f => format!("AA{i}"),
        0x10 => "EB".into(),
        0x11 => "EBU".into(),
        0x12 => "EBL".into(),
        0x13 => "EO".into(),
        0x14 => "SP".into(),
        0x15 => "ST".into(),
        0x16 => "MOD".into(),
        0x17 => "LPC".into(),
        0x18..=0x1f => format!("AR{i}"),
        0x20..=0x27 => format!("MB{i}"),
        0x28..=0x2f => format!("AB{i}"),
        0x30 => "PR".into(),
        0x31 => "FI".into(),
        0x32 => "FO0".into(),
        0x33 => "FO1".into(),
        0x34 => "PDR".into(),
        0x35 => "DDR".into(),
        0x36 => "PRP".into(),
        0x37 => "PWP".into(),
        c => format!("?r{c:02x}"),
    }
}

/// Which side of the chip an arithmetic operand belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Alu,
    Mul,
}

/// First arithmetic input (4 bits).
fn in1_name(unit: Unit, v: u32) -> String {
    let (lo, hi) = if unit == Unit::Alu { ("AA", "AB") } else { ("MA", "MB") };
    if v < 8 {
        format!("{lo}{v}")
    } else {
        format!("{hi}{}", v & 7)
    }
}

/// Second arithmetic input (5 bits): registers, the PR queue, or constants.
fn in2_name(unit: Unit, v: u32, float: bool) -> String {
    match v {
        0x00..=0x0f => in1_name(unit, v),
        0x10 => "PR".into(),
        0x11 => "PR++".into(),
        0x12 => "PR--".into(),
        0x13 => "PR#0".into(),
        0x18..=0x1f if float => ["-1.0", "0.0", "0.5", "1.0", "1.5", "2.0", "3.0", "5.0"][(v - 0x18) as usize].into(),
        0x18 => "#0".into(),
        0x19 => "#1".into(),
        0x1a => "#-1".into(),
        _ => format!("?i{v:02x}"),
    }
}

/// Arithmetic output (5 bits).
fn out_name(v: u32) -> String {
    let i = v & 7;
    match v {
        0x00..=0x07 => format!("MA{i}"),
        0x08..=0x0f => format!("MB{i}"),
        0x10..=0x17 => format!("AA{i}"),
        _ => format!("AB{i}"),
    }
}

const ALU_NAMES: [&str; 32] = [
    "FADD", "FADDZ", "FSUB", "FSUBZ", "FCMP", "FABS", "FABC", "NOP", "FEA", "FES", "FRCP", "FRSQ", "FLOG", "CIF",
    "CFI", "CFIB", "ADD", "ADDZ", "SUB", "SUBZ", "CMP", "ABS", "ATR", "ATRZ", "AND", "OR", "XOR", "NOT", "LSR", "LSL",
    "ASR", "ASL",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AluShape {
    /// in1, in2, out
    Binary,
    /// in1, out
    Unary,
    /// in1, in2 (flags only)
    Compare,
    /// in1, #imm5, out
    Immediate,
    None,
}

fn alu_shape(op: u32) -> AluShape {
    use AluShape::*;
    match op {
        0x07 => None,
        0x04 | 0x06 | 0x14 => Compare,
        0x05 | 0x0a..=0x0f | 0x15..=0x17 | 0x1b => Unary,
        0x08 | 0x09 | 0x1c..=0x1f => Immediate,
        _ => Binary,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AluOp {
    pub op: u32,
    pub in1: u32,
    pub in2: u32,
    pub out: u32,
}

impl AluOp {
    /// Float ops are 0x00-0x0f.
    pub fn is_float(&self) -> bool {
        self.op < 0x10
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MulOp {
    pub float: bool,
    pub in1: u32,
    pub in2: u32,
    pub out: u32,
}

/// A memory operand from a 7-bit register/memory code plus mode fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mem {
    /// false = A bus, true = B bus.
    pub b_bus: bool,
    pub arx: u32,
    pub ary: u32,
    pub mode: u32,
    pub disp: i32,
}

/// Transfer operand: a register code, a memory reference or an immediate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loc {
    Reg(u32),
    Mem(Mem),
    Imm(u32),
    /// External memory at EB+EO, then EO += disp.
    Ext {
        disp: i32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchTarget {
    Imm(u32),
    Reg(u32),
    ARam(u32),
    BRam(u32),
    ARamAr(u32),
    BRamAr(u32),
    Relative(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Nop,
    Rep(u32),
    RepAr(u32),
    SetL(u32),
    SetLAr(u32),
    ClrF {
        input: bool,
        output: bool,
    },
    Push(u32),
    Pop(u32),
    SetM {
        op: u32,
        value: u32,
    },
    /// DBcc / DBNcc: `negate` is the N form.
    Branch {
        cond: u32,
        negate: bool,
        target: BranchTarget,
    },
    Jump(BranchTarget),
    /// Loop back while LP is set; target relative to the branch address + 1.
    Loop(u32),
    /// DBBC/DBBS: branch on bit `bit` of ARx.
    BitBranch {
        set: bool,
        arx: u32,
        bit: u32,
        target: u32,
    },
    Call {
        cond: Option<(u32, bool)>,
        target: BranchTarget,
    },
    Ret,
    Unknown(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transfer {
    None,
    Move {
        src: Loc,
        dst: Loc,
    },
    /// Parallel A-bus and B-bus moves.
    Double {
        a: (Loc, Loc),
        b: (Loc, Loc),
    },
    /// MOV4 (to ICDTR) or MVD2/MOVI: decoded only far enough to flag them.
    Unsupported(&'static str),
    Control(Control),
}

#[derive(Clone, Debug)]
pub struct Insn {
    pub addr: u32,
    pub word: u64,
    pub format: u32,
    pub alu: Option<AluOp>,
    pub mul: Option<MulOp>,
    pub transfer: Transfer,
}

/// Condition-code names (EF1).
pub fn cond_name(c: u32) -> String {
    const N: [&str; 25] = [
        "MN", "MZ", "MV", "MU", "ZD", "NR", "IL", "ZC", "AN", "AZ", "AV", "AU", "MD", "AD", "", "", "F0", "F1", "F2",
        "", "IFF", "IFE", "OFF", "OFE", "IF",
    ];
    match N.get(c as usize) {
        Some(n) if !n.is_empty() => (*n).into(),
        _ => format!("?c{c:02x}"),
    }
}

fn mem_or_reg(code: u32, mode: u32, ary: u32, disp: i32) -> Loc {
    if code & 0x40 == 0 {
        Loc::Reg(code)
    } else {
        Loc::Mem(Mem { b_bus: code & 0x20 != 0, arx: code & 7, ary, mode, disp })
    }
}

fn branch_target(ef2: u32) -> BranchTarget {
    let v = ef2 & 0xfff;
    let reg = (ef2 >> 6) & 0x3f;
    if ef2 & 0x1000 != 0 && (ef2 >> 13) == 0 {
        return BranchTarget::Relative(sext(v, 12));
    }
    match ef2 >> 13 {
        0 => BranchTarget::Imm(v),
        1 => BranchTarget::Reg(0x18 | (reg & 7)),
        2 => BranchTarget::Reg(if reg & 0x20 != 0 { 0x28 | (reg & 7) } else { 0x08 | (reg & 7) }),
        3 => BranchTarget::Reg(if reg & 0x20 != 0 { 0x20 | (reg & 7) } else { reg & 7 }),
        4 => BranchTarget::ARam(ef2 & 0x3ff),
        5 => BranchTarget::BRam(ef2 & 0x3ff),
        6 => BranchTarget::ARamAr((ef2 >> 6) & 7),
        _ => BranchTarget::BRamAr((ef2 >> 6) & 7),
    }
}

fn control(w: u64, addr: u32) -> Control {
    let cop = bits(w, 26, 22);
    let ef1 = bits(w, 21, 16);
    let ef2 = bits(w, 15, 0);
    match cop {
        0x00 => Control::Nop,
        // The immediate form is the only one observed; EF1 != 0 selects ARx.
        0x01 if ef1 == 0 => Control::Rep(ef2),
        0x01 => Control::RepAr((ef2 >> 12) & 7),
        0x02 if ef1 == 0 => Control::SetL(ef2),
        0x02 => Control::SetLAr((ef2 >> 12) & 7),
        0x03 => Control::ClrF { input: ef1 & 1 != 0, output: ef1 & 2 != 0 },
        0x04 => Control::Push((ef2 >> 6) & 0x3f),
        0x05 => Control::Pop((ef2 >> 6) & 0x3f),
        0x08..=0x0d => Control::SetM { op: cop, value: ef2 },
        0x10 | 0x11 => Control::Branch { cond: ef1, negate: cop == 0x11, target: branch_target(ef2) },
        0x12 => Control::Jump(branch_target(ef2)),
        0x13 => Control::Loop(addr.wrapping_add(1).wrapping_add(sext(ef2 & 0xfff, 12) as u32) % PROGRAM_WORDS),
        0x14 | 0x15 => Control::BitBranch {
            set: cop == 0x15,
            arx: (ef2 >> 13) & 7,
            bit: ef1 & 0xf,
            target: addr.wrapping_add(1).wrapping_add(sext(ef2 & 0xfff, 12) as u32) % PROGRAM_WORDS,
        },
        0x18 | 0x19 => Control::Call { cond: Some((ef1, cop == 0x19)), target: branch_target(ef2) },
        0x1a => Control::Call { cond: None, target: branch_target(ef2) },
        0x1b => Control::Ret,
        _ => Control::Unknown(cop),
    }
}

pub fn decode(addr: u32, w: u64) -> Insn {
    let format = bits(w, 63, 61);
    let (mut alu, mut mul) = (None, None);
    if format < 7 {
        if w >> 63 == 0 {
            alu = Some(AluOp { op: bits(w, 60, 56), in1: bits(w, 55, 52), in2: bits(w, 51, 47), out: bits(w, 46, 42) });
            mul = Some(MulOp {
                float: bits(w, 41, 41) == 1,
                in1: bits(w, 40, 37),
                in2: bits(w, 36, 32),
                out: bits(w, 31, 27),
            });
        } else if bits(w, 41, 41) == 1 {
            alu = Some(AluOp { op: bits(w, 60, 56), in1: bits(w, 55, 52), in2: bits(w, 51, 47), out: bits(w, 46, 42) });
        } else {
            mul = Some(MulOp {
                float: bits(w, 56, 56) == 1,
                in1: bits(w, 55, 52),
                in2: bits(w, 51, 47),
                out: bits(w, 46, 42),
            });
        }
    }
    // An ALU NOP is no operation at all.
    if alu.is_some_and(|a| a.op == 0x07) {
        alu = None;
    }

    let transfer = match format {
        0 => {
            let sd = bits(w, 26, 25);
            match sd {
                0 => Transfer::Double {
                    a: (Loc::Reg(bits(w, 24, 20)), Loc::Reg(bits(w, 19, 15))),
                    b: (Loc::Reg(bits(w, 14, 10) | 0x20), Loc::Reg(bits(w, 9, 5) | 0x20)),
                },
                1 | 2 => {
                    let md = bits(w, 3, 0);
                    let am =
                        Loc::Mem(Mem { b_bus: false, arx: bits(w, 19, 17), ary: bits(w, 16, 14), mode: md, disp: 0 });
                    let bm = Loc::Mem(Mem { b_bus: true, arx: bits(w, 9, 7), ary: bits(w, 6, 4), mode: md, disp: 0 });
                    let ar = Loc::Reg(bits(w, 24, 20));
                    let br = Loc::Reg(bits(w, 13, 10) | 0x20);
                    if sd == 1 {
                        Transfer::Double { a: (ar, am), b: (br, bm) }
                    } else {
                        Transfer::Double { a: (am, ar), b: (bm, br) }
                    }
                }
                _ => Transfer::Unsupported("MVD1 sd=3"),
            }
        }
        1 => {
            let dr = bits(w, 18, 12);
            if bits(w, 26, 26) == 1 {
                let ext = Loc::Ext { disp: sext(bits(w, 24, 19), 6) };
                let int = mem_or_reg(dr, bits(w, 3, 0), bits(w, 6, 4), sext(bits(w, 11, 7), 5));
                if bits(w, 25, 25) == 1 {
                    Transfer::Move { src: ext, dst: int }
                } else {
                    Transfer::Move { src: int, dst: ext }
                }
            } else {
                let sr = bits(w, 25, 19);
                let src = if sr == 0x58 {
                    Loc::Imm(bits(w, 11, 0))
                } else {
                    mem_or_reg(sr, bits(w, 3, 0), bits(w, 6, 4), sext(bits(w, 11, 7), 5))
                };
                let dst = mem_or_reg(dr, bits(w, 3, 0), bits(w, 6, 4), sext(bits(w, 11, 7), 5));
                Transfer::Move { src, dst }
            }
        }
        2 | 6 => Transfer::Control(control(w, addr)),
        4 => Transfer::Unsupported(if bits(w, 39, 38) == 3 { "MOVI" } else { "MVD2" }),
        5 => {
            if bits(w, 40, 39) == 2 {
                Transfer::Unsupported("MOV4")
            } else {
                let (md, ary, disp) = (bits(w, 3, 0), bits(w, 6, 4), sext(bits(w, 20, 7), 14));
                let dr = bits(w, 30, 24);
                if bits(w, 38, 38) == 1 {
                    let ext = Loc::Ext { disp: sext(bits(w, 36, 31), 6) };
                    let int = mem_or_reg(dr, md, ary, disp);
                    if bits(w, 37, 37) == 1 {
                        Transfer::Move { src: ext, dst: int }
                    } else {
                        Transfer::Move { src: int, dst: ext }
                    }
                } else {
                    let sr = bits(w, 37, 31);
                    let src = if sr == 0x58 { Loc::Imm(bits(w, 23, 0)) } else { mem_or_reg(sr, md, ary, disp) };
                    Transfer::Move { src, dst: mem_or_reg(dr, md, ary, disp) }
                }
            }
        }
        7 => {
            let imm = bits(w, 58, 27);
            let dst = mem_or_reg(bits(w, 25, 19), bits(w, 3, 0), bits(w, 6, 4), sext(bits(w, 18, 7), 12));
            Transfer::Move { src: Loc::Imm(imm), dst }
        }
        _ => Transfer::Unsupported("format 3"),
    };
    Insn { addr, word: w, format, alu, mul, transfer }
}

/// Control-flow summary for analysis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Next,
    /// Delayed transfer: the next word (delay slot) runs first.
    Jump(Option<u32>),
    Branch(Option<u32>),
    Call(Option<u32>),
    Ret,
    Invalid,
}

impl Insn {
    pub fn flow(&self) -> Flow {
        let resolve = |t: BranchTarget| match t {
            BranchTarget::Imm(a) => Some(a),
            BranchTarget::Relative(d) => Some(self.addr.wrapping_add(1).wrapping_add(d as u32) % PROGRAM_WORDS),
            _ => None,
        };
        match self.transfer {
            Transfer::Control(c) => match c {
                Control::Jump(t) => Flow::Jump(resolve(t)),
                Control::Branch { target, .. } => Flow::Branch(resolve(target)),
                Control::Loop(t) | Control::BitBranch { target: t, .. } => Flow::Branch(Some(t)),
                Control::Call { target, .. } => Flow::Call(resolve(target)),
                Control::Ret => Flow::Ret,
                Control::Unknown(_) => Flow::Invalid,
                _ => Flow::Next,
            },
            Transfer::Unsupported("format 3") => Flow::Invalid,
            _ => Flow::Next,
        }
    }
}

impl fmt::Display for Mem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bus = if self.b_bus { "B" } else { "A" };
        let (x, y, d) = (self.arx, self.ary, self.disp);
        let ea = match self.mode {
            0 => format!("@AR{x}"),
            1 => format!("@AR{x}++"),
            2 => format!("@AR{x}--"),
            3 => format!("@AR{x}++{d}"),
            4 => format!("@AR{x}+AR{y}"),
            5 => format!("@AR{x}+AR{y}++"),
            6 => format!("@AR{x}+AR{y}--"),
            7 => format!("@AR{x}+AR{y}++{d}"),
            8 => format!("@AR{x}+AR{y}U"),
            9 => format!("@AR{x}+AR{y}L"),
            0xa => format!("@AR{x}+{d}"),
            0xb => format!("@AR{x}+AR{y}+{d}"),
            0xc => format!("{:#x}", d as u32 & 0x3fff),
            0xd => format!("@AR{x}+[AR{y}++]"),
            0xe => format!("@AR{x}+[AR{y}--]"),
            _ => format!("@AR{x}+[AR{y}++{d}]"),
        };
        write!(f, "{bus}({ea})")
    }
}

impl fmt::Display for Loc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Loc::Reg(r) => f.write_str(&reg_name(*r)),
            Loc::Mem(m) => write!(f, "{m}"),
            Loc::Imm(v) => write!(f, "#{v:#x}"),
            Loc::Ext { disp } => write!(f, "E(@EB+EO++{disp})"),
        }
    }
}

impl fmt::Display for BranchTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            BranchTarget::Imm(a) => write!(f, "{a:#05x}"),
            BranchTarget::Reg(r) => f.write_str(&reg_name(r)),
            BranchTarget::ARam(a) => write!(f, "A({a:#x})"),
            BranchTarget::BRam(a) => write!(f, "B({a:#x})"),
            BranchTarget::ARamAr(x) => write!(f, "A(@AR{x})"),
            BranchTarget::BRamAr(x) => write!(f, "B(@AR{x})"),
            BranchTarget::Relative(d) => write!(f, "rel {d:+}"),
        }
    }
}

impl fmt::Display for Control {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Control::Nop => f.write_str("NOP"),
            Control::Rep(n) => write!(f, "REP {n}"),
            Control::RepAr(x) => write!(f, "REP AR{x}"),
            Control::SetL(n) => write!(f, "SETL {n}"),
            Control::SetLAr(x) => write!(f, "SETL AR{x}"),
            Control::ClrF { input, output } => match (input, output) {
                (true, false) => f.write_str("CLRFI"),
                (false, true) => f.write_str("CLRFO"),
                _ => f.write_str("CLRF"),
            },
            Control::Push(r) => write!(f, "PUSH {}", reg_name(r)),
            Control::Pop(r) => write!(f, "POP {}", reg_name(r)),
            Control::SetM { op, value } => {
                let n = ["SETM", "SETM CBSA", "SETM CBSB", "SETM RF", "SETM RDY", "SETM WAIT"][(op - 8) as usize];
                write!(f, "{n} #{value:#x}")
            }
            Control::Branch { cond, negate, target } => {
                write!(f, "DB{}{} {target}", if negate { "N" } else { "" }, cond_name(cond))
            }
            Control::Jump(t) => write!(f, "DJMP {t}"),
            Control::Loop(t) => write!(f, "DBLP {t:#05x}"),
            Control::BitBranch { set, arx, bit, target } => {
                write!(f, "DBB{} AR{arx}:{bit}, {target:#05x}", if set { "S" } else { "C" })
            }
            Control::Call { cond: None, target } => write!(f, "DCALL {target}"),
            Control::Call { cond: Some((c, n)), target } => {
                write!(f, "DC{}{} {target}", if n { "N" } else { "" }, cond_name(c))
            }
            Control::Ret => f.write_str("DRET"),
            Control::Unknown(c) => write!(f, "?ctrl{c:02x}"),
        }
    }
}

impl fmt::Display for Insn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts: Vec<String> = Vec::new();
        if let Some(a) = self.alu {
            let fl = a.is_float();
            let name = ALU_NAMES[a.op as usize];
            let i1 = in1_name(Unit::Alu, a.in1);
            parts.push(match alu_shape(a.op) {
                AluShape::Binary => format!("{name} {i1}, {}, {}", in2_name(Unit::Alu, a.in2, fl), out_name(a.out)),
                AluShape::Unary => format!("{name} {i1}, {}", out_name(a.out)),
                AluShape::Compare => format!("{name} {i1}, {}", in2_name(Unit::Alu, a.in2, fl)),
                AluShape::Immediate => format!("{name} {i1}, #{}, {}", a.in2, out_name(a.out)),
                AluShape::None => name.into(),
            });
        }
        if let Some(m) = self.mul {
            let name = if m.float { "FMUL" } else { "MUL" };
            parts.push(format!(
                "{name} {}, {}, {}",
                in1_name(Unit::Mul, m.in1),
                in2_name(Unit::Mul, m.in2, m.float),
                out_name(m.out)
            ));
        }
        match self.transfer {
            Transfer::None => {}
            Transfer::Move { src, dst } => parts.push(format!("MOV {src}, {dst}")),
            Transfer::Double { a, b } => parts.push(format!("MVD {}, {} : {}, {}", a.0, a.1, b.0, b.1)),
            Transfer::Unsupported(n) => parts.push(format!("<{n}>")),
            Transfer::Control(Control::Nop) if !parts.is_empty() => {}
            Transfer::Control(c) => parts.push(c.to_string()),
        }
        if parts.is_empty() {
            parts.push("NOP".into());
        }
        f.write_str(&parts.join(" ; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(fields: &[(u32, u32, u64)]) -> u64 {
        fields.iter().fold(0, |w, &(hi, lo, v)| w | (v << lo) & (((1u64 << (hi - lo + 1)) - 1) << lo))
    }

    #[test]
    fn two_op_alu_and_mul() {
        // F2: FADD AA1, AB2, MA3 ; FMUL MA0, MB1, AA4 ; NOP
        let w = put(&[
            (63, 61, 2),
            (60, 56, 0x00),
            (55, 52, 1),
            (51, 47, 0x0a),
            (46, 42, 3),
            (41, 41, 1),
            (40, 37, 0),
            (36, 32, 0x09),
            (31, 27, 0x14),
        ]);
        assert_eq!(decode(0, w).to_string(), "FADD AA1, AB2, MA3 ; FMUL MA0, MB1, AA4");
    }

    #[test]
    fn loop_target_is_relative_to_next_word() {
        // F6, single NOP ALU op (bit 41 set, op 7), DBLP -3 at 0x100.
        let w = put(&[(63, 61, 6), (60, 56, 7), (41, 41, 1), (26, 22, 0x13), (11, 0, 0xffd)]);
        let i = decode(0x100, w);
        assert_eq!(i.to_string(), "DBLP 0x0fe");
        assert_eq!(i.flow(), Flow::Branch(Some(0xfe)));
    }

    #[test]
    fn mov3_immediate_to_register() {
        // MOV3 #0x3f800000 -> AA2 (code 0x0a)
        let w = put(&[(63, 61, 7), (58, 27, 0x3f80_0000), (25, 19, 0x0a)]);
        assert_eq!(decode(0, w).to_string(), "MOV #0x3f800000, AA2");
    }

    #[test]
    fn mov1_fifo_to_memory() {
        // F1: MOV FI, A(@AR1+5) with an ALU NOP and a zero MUL.
        let w = put(&[(63, 61, 1), (60, 56, 7), (25, 19, 0x31), (18, 12, 0x41), (11, 7, 5), (3, 0, 0xa)]);
        let s = decode(0, w).to_string();
        assert!(s.ends_with("MOV FI, A(@AR1+5)"), "{s}");
    }

    #[test]
    fn absolute_call_flow() {
        let w = put(&[(63, 61, 6), (60, 56, 7), (41, 41, 1), (26, 22, 0x1a), (11, 0, 0x234)]);
        assert_eq!(decode(0, w).flow(), Flow::Call(Some(0x234)));
    }
}
