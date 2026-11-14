//! The 23-bit compute field (UM Appendix B): single-function ALU,
//! multiplier and shifter operations, MR transfers and the multifunction
//! computations.

use state::bits;
use std::fmt;

/// ALU operations (UM Tables B.1 and B.2). Names follow the syntax; `F`
/// variants are floating point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AluOp {
    Add,
    Sub,
    AddCi,
    SubCi,
    Avg,
    Comp,
    PassCi,
    PassCiMinus1,
    Inc,
    Dec,
    Neg,
    Abs,
    Pass,
    And,
    Or,
    Xor,
    Not,
    Min,
    Max,
    Clip,
    FAdd,
    FSub,
    FAbsAdd,
    FAbsSub,
    FAvg,
    FComp,
    FNeg,
    FAbs,
    FPass,
    FRnd,
    FScalb,
    FMant,
    FLogb,
    FFixBy,
    FFix,
    FTruncBy,
    FTrunc,
    FFloatBy,
    FFloat,
    FRecips,
    FRsqrts,
    FCopysign,
    FMin,
    FMax,
    FClip,
}

impl AluOp {
    /// The 8-bit ALU opcode (CU=00).
    pub fn from_opcode(o: u8) -> Option<AluOp> {
        use AluOp::*;
        Some(match o {
            0x01 => Add,
            0x02 => Sub,
            0x05 => AddCi,
            0x06 => SubCi,
            0x09 => Avg,
            0x0a => Comp,
            0x25 => PassCi,
            0x26 => PassCiMinus1,
            0x29 => Inc,
            0x2a => Dec,
            0x22 => Neg,
            0x30 => Abs,
            0x21 => Pass,
            0x40 => And,
            0x41 => Or,
            0x42 => Xor,
            0x43 => Not,
            0x61 => Min,
            0x62 => Max,
            0x63 => Clip,
            0x81 => FAdd,
            0x82 => FSub,
            0x91 => FAbsAdd,
            0x92 => FAbsSub,
            0x89 => FAvg,
            0x8a => FComp,
            0xa2 => FNeg,
            0xb0 => FAbs,
            0xa1 => FPass,
            0xa5 => FRnd,
            0xbd => FScalb,
            0xad => FMant,
            0xc1 => FLogb,
            0xd9 => FFixBy,
            0xc9 => FFix,
            0xdd => FTruncBy,
            0xcd => FTrunc,
            0xda => FFloatBy,
            0xca => FFloat,
            0xc4 => FRecips,
            0xc5 => FRsqrts,
            0xe0 => FCopysign,
            0xe1 => FMin,
            0xe2 => FMax,
            0xe3 => FClip,
            _ => return None,
        })
    }

    /// Takes floating-point operands or yields a floating-point result,
    /// so sets AF (UM §2.5.3.8).
    pub fn is_float(self) -> bool {
        self as u8 >= AluOp::FAdd as u8
    }

    /// Format with operand registers.
    pub fn format(self, n: u8, x: u8, y: u8) -> String {
        use AluOp::*;
        let (r, f) = (|i: u8| format!("R{i}"), |i: u8| format!("F{i}"));
        match self {
            Add => format!("{} = {} + {}", r(n), r(x), r(y)),
            Sub => format!("{} = {} - {}", r(n), r(x), r(y)),
            AddCi => format!("{} = {} + {} + CI", r(n), r(x), r(y)),
            SubCi => format!("{} = {} - {} + CI - 1", r(n), r(x), r(y)),
            Avg => format!("{} = ({} + {})/2", r(n), r(x), r(y)),
            Comp => format!("COMP({}, {})", r(x), r(y)),
            PassCi => format!("{} = {} + CI", r(n), r(x)),
            PassCiMinus1 => format!("{} = {} + CI - 1", r(n), r(x)),
            Inc => format!("{} = {} + 1", r(n), r(x)),
            Dec => format!("{} = {} - 1", r(n), r(x)),
            Neg => format!("{} = -{}", r(n), r(x)),
            Abs => format!("{} = ABS {}", r(n), r(x)),
            Pass => format!("{} = PASS {}", r(n), r(x)),
            And => format!("{} = {} AND {}", r(n), r(x), r(y)),
            Or => format!("{} = {} OR {}", r(n), r(x), r(y)),
            Xor => format!("{} = {} XOR {}", r(n), r(x), r(y)),
            Not => format!("{} = NOT {}", r(n), r(x)),
            Min => format!("{} = MIN({}, {})", r(n), r(x), r(y)),
            Max => format!("{} = MAX({}, {})", r(n), r(x), r(y)),
            Clip => format!("{} = CLIP {} BY {}", r(n), r(x), r(y)),
            FAdd => format!("{} = {} + {}", f(n), f(x), f(y)),
            FSub => format!("{} = {} - {}", f(n), f(x), f(y)),
            FAbsAdd => format!("{} = ABS ({} + {})", f(n), f(x), f(y)),
            FAbsSub => format!("{} = ABS ({} - {})", f(n), f(x), f(y)),
            FAvg => format!("{} = ({} + {})/2", f(n), f(x), f(y)),
            FComp => format!("COMP({}, {})", f(x), f(y)),
            FNeg => format!("{} = -{}", f(n), f(x)),
            FAbs => format!("{} = ABS {}", f(n), f(x)),
            FPass => format!("{} = PASS {}", f(n), f(x)),
            FRnd => format!("{} = RND {}", f(n), f(x)),
            FScalb => format!("{} = SCALB {} BY {}", f(n), f(x), r(y)),
            FMant => format!("{} = MANT {}", r(n), f(x)),
            FLogb => format!("{} = LOGB {}", r(n), f(x)),
            FFixBy => format!("{} = FIX {} BY {}", r(n), f(x), r(y)),
            FFix => format!("{} = FIX {}", r(n), f(x)),
            FTruncBy => format!("{} = TRUNC {} BY {}", r(n), f(x), r(y)),
            FTrunc => format!("{} = TRUNC {}", r(n), f(x)),
            FFloatBy => format!("{} = FLOAT {} BY {}", f(n), r(x), r(y)),
            FFloat => format!("{} = FLOAT {}", f(n), r(x)),
            FRecips => format!("{} = RECIPS {}", f(n), f(x)),
            FRsqrts => format!("{} = RSQRTS {}", f(n), f(x)),
            FCopysign => format!("{} = {} COPYSIGN {}", f(n), f(x), f(y)),
            FMin => format!("{} = MIN({}, {})", f(n), f(x), f(y)),
            FMax => format!("{} = MAX({}, {})", f(n), f(x), f(y)),
            FClip => format!("{} = CLIP {} BY {}", f(n), f(x), f(y)),
        }
    }
}

/// One of the two 80-bit multiplier result registers: MRF (foreground) or
/// MRB (background), UM §2.6.2.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mr {
    F,
    B,
}

impl Mr {
    fn name(self) -> &'static str {
        match self {
            Mr::F => "MRF",
            Mr::B => "MRB",
        }
    }
}

/// Where a fixed-point multiplier result goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MulDst {
    /// Rn: bits 63-32 of a fractional result, 31-0 of an integer one.
    Reg,
    Mr(Mr),
}

/// Multiplier operations (UM Table B.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MulOp {
    /// `dst = [MR ±] Rx * Ry (mods)`: `acc` is the MR register added to
    /// (`sub`: subtracted from); `xs`/`ys` signed inputs, `frac`
    /// fractional, `round` round to nearest at bit 32.
    Mul { acc: Option<Mr>, sub: bool, dst: MulDst, xs: bool, ys: bool, frac: bool, round: bool },
    /// `dst = SAT MR (mod1)`.
    Sat { src: Mr, dst: MulDst, signed: bool, frac: bool },
    /// `dst = RND MR (mod1)`, fractional only.
    Rnd { src: Mr, dst: MulDst, signed: bool },
    /// `MR = 0`.
    Clear(Mr),
    /// `Fn = Fx * Fy`.
    FMul,
}

impl MulOp {
    /// The 8-bit multiplier opcode (CU=01).
    pub fn from_opcode(o: u8) -> Option<MulOp> {
        let o = o as u32;
        let dst2 = |d: u32| match d {
            0 => Some((MulDst::Reg, Mr::F)),
            1 => Some((MulDst::Reg, Mr::B)),
            2 => Some((MulDst::Mr(Mr::F), Mr::F)),
            _ => Some((MulDst::Mr(Mr::B), Mr::B)),
        };
        let (y, x, f, d, r) =
            (bits(o, 5, 5) == 1, bits(o, 4, 4) == 1, bits(o, 3, 3) == 1, bits(o, 2, 1), bits(o, 0, 0) == 1);
        Some(match o >> 6 {
            0b01 => {
                // 01yx f d r: Rn, MRF or MRB = Rx * Ry; d=01 undefined.
                let dst = match d {
                    0 => MulDst::Reg,
                    2 => MulDst::Mr(Mr::F),
                    3 => MulDst::Mr(Mr::B),
                    _ => return None,
                };
                if r && !f {
                    return None;
                }
                MulOp::Mul { acc: None, sub: false, dst, xs: x, ys: y, frac: f, round: r }
            }
            0b10 | 0b11 => {
                if r && !f {
                    return None;
                }
                let (dst, acc) = dst2(d)?;
                MulOp::Mul { acc: Some(acc), sub: o >> 6 == 0b11, dst, xs: x, ys: y, frac: f, round: r }
            }
            _ => match o {
                0x30 => MulOp::FMul,
                0x14 => MulOp::Clear(Mr::F),
                0x16 => MulOp::Clear(Mr::B),
                0x00..=0x0f => {
                    let (dst, src) = dst2(d)?;
                    MulOp::Sat { src, dst, signed: o & 1 == 1, frac: f }
                }
                0x18..=0x1f => {
                    let (dst, src) = dst2(d)?;
                    MulOp::Rnd { src, dst, signed: o & 1 == 1 }
                }
                _ => return None,
            },
        })
    }

    fn mods(xs: bool, ys: bool, frac: bool, round: bool) -> String {
        let s = |b: bool| if b { 'S' } else { 'U' };
        format!("({}{}{}{})", s(xs), s(ys), if frac { "F" } else { "I" }, if round { "R" } else { "" })
    }

    pub fn format(self, n: u8, x: u8, y: u8) -> String {
        let dst = |d: MulDst| match d {
            MulDst::Reg => format!("R{n}"),
            MulDst::Mr(m) => m.name().to_string(),
        };
        match self {
            MulOp::Mul { acc, sub, dst: d, xs, ys, frac, round } => {
                let acc = match acc {
                    Some(m) => format!("{} {} ", m.name(), if sub { "-" } else { "+" }),
                    None => String::new(),
                };
                format!("{} = {acc}R{x} * R{y} {}", dst(d), Self::mods(xs, ys, frac, round))
            }
            MulOp::Sat { src, dst: d, signed, frac } => {
                format!(
                    "{} = SAT {} ({}{})",
                    dst(d),
                    src.name(),
                    if signed { 'S' } else { 'U' },
                    if frac { 'F' } else { 'I' }
                )
            }
            MulOp::Rnd { src, dst: d, signed } => {
                format!("{} = RND {} ({}F)", dst(d), src.name(), if signed { 'S' } else { 'U' })
            }
            MulOp::Clear(m) => format!("{} = 0", m.name()),
            MulOp::FMul => format!("F{n} = F{x} * F{y}"),
        }
    }
}

/// Shifter operations (UM Table B.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShiftOp {
    Lshift,
    OrLshift,
    Ashift,
    OrAshift,
    Rot,
    Bclr,
    Bset,
    Btgl,
    Btst,
    Fdep,
    OrFdep,
    FdepSe,
    OrFdepSe,
    Fext,
    FextSe,
    Exp,
    ExpEx,
    Leftz,
    Lefto,
    Fpack,
    Funpack,
}

impl ShiftOp {
    /// The 8-bit shifter opcode (CU=10).
    pub fn from_opcode(o: u8) -> Option<ShiftOp> {
        use ShiftOp::*;
        Some(match o {
            0x00 => Lshift,
            0x20 => OrLshift,
            0x04 => Ashift,
            0x24 => OrAshift,
            0x08 => Rot,
            0xc4 => Bclr,
            0xc0 => Bset,
            0xc8 => Btgl,
            0xcc => Btst,
            0x44 => Fdep,
            0x64 => OrFdep,
            0x4c => FdepSe,
            0x6c => OrFdepSe,
            0x40 => Fext,
            0x48 => FextSe,
            0x80 => Exp,
            0x84 => ExpEx,
            0x88 => Leftz,
            0x8c => Lefto,
            0x90 => Fpack,
            0x94 => Funpack,
            _ => return None,
        })
    }

    /// Two-input operations, which have an immediate form (type 6).
    pub fn takes_immediate(self) -> bool {
        use ShiftOp::*;
        !matches!(self, Exp | ExpEx | Leftz | Lefto | Fpack | Funpack)
    }

    /// Field deposit/extract: the Y operand is bit6 and len6.
    pub fn is_field(self) -> bool {
        use ShiftOp::*;
        matches!(self, Fdep | OrFdep | FdepSe | OrFdepSe | Fext | FextSe)
    }

    fn template(self) -> (&'static str, &'static str) {
        use ShiftOp::*;
        // (prefix, suffix) around "Rx BY y".
        match self {
            Lshift => ("LSHIFT", ""),
            OrLshift => ("OR LSHIFT", ""),
            Ashift => ("ASHIFT", ""),
            OrAshift => ("OR ASHIFT", ""),
            Rot => ("ROT", ""),
            Bclr => ("BCLR", ""),
            Bset => ("BSET", ""),
            Btgl => ("BTGL", ""),
            Btst => ("BTST", ""),
            Fdep => ("FDEP", ""),
            OrFdep => ("OR FDEP", ""),
            FdepSe => ("FDEP", " (SE)"),
            OrFdepSe => ("OR FDEP", " (SE)"),
            Fext => ("FEXT", ""),
            FextSe => ("FEXT", " (SE)"),
            Exp => ("EXP", ""),
            ExpEx => ("EXP", " (EX)"),
            Leftz => ("LEFTZ", ""),
            Lefto => ("LEFTO", ""),
            Fpack => ("FPACK", ""),
            Funpack => ("FUNPACK", ""),
        }
    }

    fn format_with(self, n: u8, x: u8, y: &str) -> String {
        use ShiftOp::*;
        let (op, suffix) = self.template();
        match self {
            Btst => format!("BTST R{x} BY {y}"),
            Exp | ExpEx | Leftz | Lefto => format!("R{n} = {op} R{x}{suffix}"),
            Fpack => format!("R{n} = FPACK F{x}"),
            Funpack => format!("F{n} = FUNPACK R{x}"),
            OrLshift | OrAshift | OrFdep | OrFdepSe => format!("R{n} = R{n} {op} R{x} BY {y}{suffix}"),
            _ => format!("R{n} = {op} R{x} BY {y}{suffix}"),
        }
    }

    pub fn format(self, n: u8, x: u8, y: u8) -> String {
        self.format_with(n, x, &format!("R{y}"))
    }

    /// Immediate form (type 6): `imm` is data8, or bit6:len6 for fields.
    pub fn format_imm(self, n: u8, x: u8, imm: u16) -> String {
        let y = if self.is_field() {
            format!("{}:{}", imm & 0x3f, (imm >> 6) & 0x3f)
        } else if matches!(self, ShiftOp::Bclr | ShiftOp::Bset | ShiftOp::Btgl | ShiftOp::Btst) {
            format!("{}", imm & 0xff)
        } else {
            format!("{}", imm as u8 as i8)
        };
        self.format_with(n, x, &y)
    }
}

/// Multiplier X and Y, ALU X and Y operand registers of a multifunction
/// computation: R0-R3, R4-R7, R8-R11 and R12-R15 (UM Figure B.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MfRegs {
    pub xm: u8,
    pub ym: u8,
    pub xa: u8,
    pub ya: u8,
}

/// A compute operation (UM Appendix B).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compute {
    /// No compute operation (a zero field).
    None,
    Alu {
        op: AluOp,
        rn: u8,
        rx: u8,
        ry: u8,
    },
    /// Dual add/subtract: `Ra = Rx + Ry, Rs = Rx - Ry`.
    AddSub {
        float: bool,
        ra: u8,
        rs: u8,
        rx: u8,
        ry: u8,
    },
    Mul {
        op: MulOp,
        rn: u8,
        rx: u8,
        ry: u8,
    },
    Shift {
        op: ShiftOp,
        rn: u8,
        rx: u8,
        ry: u8,
    },
    /// MR transfer (UM B-52): `mr` is the AI code (0-2 MR0F-MR2F, 4-6
    /// MR0B-MR2B); `to_mr`: register file to MR.
    MrMove {
        to_mr: bool,
        mr: u8,
        rk: u8,
    },
    /// Parallel multiplier and ALU (UM Table B.7).
    Multi {
        mul: MulOp,
        alu: AluOp,
        rm: u8,
        ra: u8,
        regs: MfRegs,
    },
    /// Parallel multiplier and dual add/subtract.
    MultiAddSub {
        float: bool,
        rm: u8,
        ra: u8,
        rs: u8,
        regs: MfRegs,
    },
    /// An undefined encoding of the 23-bit field.
    Unknown(u32),
}

impl Compute {
    pub fn decode(cf: u32) -> Compute {
        if cf == 0 {
            return Compute::None;
        }
        let (n, x, y) = (bits(cf, 11, 8) as u8, bits(cf, 7, 4) as u8, bits(cf, 3, 0) as u8);
        if bits(cf, 22, 22) == 0 {
            let opcode = bits(cf, 19, 12) as u8;
            return match bits(cf, 21, 20) {
                0 if opcode >> 4 == 0x7 || opcode >> 4 == 0xf => {
                    Compute::AddSub { float: opcode >> 4 == 0xf, ra: n, rs: opcode & 15, rx: x, ry: y }
                }
                0 => match AluOp::from_opcode(opcode) {
                    Some(op) => Compute::Alu { op, rn: n, rx: x, ry: y },
                    None => Compute::Unknown(cf),
                },
                1 => match MulOp::from_opcode(opcode) {
                    Some(op) => Compute::Mul { op, rn: n, rx: x, ry: y },
                    None => Compute::Unknown(cf),
                },
                2 => match ShiftOp::from_opcode(opcode) {
                    Some(op) => Compute::Shift { op, rn: n, rx: x, ry: y },
                    None => Compute::Unknown(cf),
                },
                _ => Compute::Unknown(cf),
            };
        }
        let regs = MfRegs {
            xm: bits(cf, 7, 6) as u8,
            ym: 4 + bits(cf, 5, 4) as u8,
            xa: 8 + bits(cf, 3, 2) as u8,
            ya: 12 + bits(cf, 1, 0) as u8,
        };
        let (rm, ra) = (bits(cf, 15, 12) as u8, bits(cf, 11, 8) as u8);
        let opcode = bits(cf, 21, 16);
        if opcode >> 4 >= 2 {
            return Compute::MultiAddSub { float: opcode >> 4 == 3, rm, ra, rs: (opcode & 15) as u8, regs };
        }
        if opcode >> 1 == 0 {
            // MR transfer: 1 00000 T AI RK; bits 7-0 are unspecified, so
            // ignored (UM §A.3).
            let mr = bits(cf, 15, 12) as u8;
            if mr & 3 == 3 || mr > 6 {
                return Compute::Unknown(cf);
            }
            return Compute::MrMove { to_mr: opcode & 1 == 1, mr, rk: n };
        }
        let ssfr = MulOp::Mul { acc: None, sub: false, dst: MulDst::Reg, xs: true, ys: true, frac: true, round: true };
        let mac = |sub: bool, dst: MulDst| MulOp::Mul {
            acc: Some(Mr::F),
            sub,
            dst,
            xs: true,
            ys: true,
            frac: true,
            round: dst == MulDst::Reg,
        };
        let mul = match opcode >> 2 {
            0b0001 => ssfr,
            0b0010 => mac(false, MulDst::Mr(Mr::F)),
            0b0011 => mac(false, MulDst::Reg),
            0b0100 => mac(true, MulDst::Mr(Mr::F)),
            0b0101 => mac(true, MulDst::Reg),
            0b0110 | 0b0111 => MulOp::FMul,
            _ => return Compute::Unknown(cf),
        };
        let alu = if mul == MulOp::FMul {
            [
                AluOp::FAdd,
                AluOp::FSub,
                AluOp::FFloatBy,
                AluOp::FFixBy,
                AluOp::FAvg,
                AluOp::FAbs,
                AluOp::FMax,
                AluOp::FMin,
            ][(opcode & 7) as usize]
        } else {
            match opcode & 3 {
                0 => AluOp::Add,
                1 => AluOp::Sub,
                2 => AluOp::Avg,
                _ => return Compute::Unknown(cf),
            }
        };
        Compute::Multi { mul, alu, rm, ra, regs }
    }

    pub fn is_unknown(&self) -> bool {
        matches!(self, Compute::Unknown(_))
    }
}

fn mr_name(mr: u8) -> String {
    format!("MR{}{}", mr & 3, if mr & 4 != 0 { 'B' } else { 'F' })
}

impl fmt::Display for Compute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Compute::None => f.write_str("NOP"),
            Compute::Alu { op, rn, rx, ry } => f.write_str(&op.format(rn, rx, ry)),
            Compute::AddSub { float, ra, rs, rx, ry } => {
                let p = if float { 'F' } else { 'R' };
                write!(f, "{p}{ra} = {p}{rx} + {p}{ry}, {p}{rs} = {p}{rx} - {p}{ry}")
            }
            Compute::Mul { op, rn, rx, ry } => f.write_str(&op.format(rn, rx, ry)),
            Compute::Shift { op, rn, rx, ry } => f.write_str(&op.format(rn, rx, ry)),
            Compute::MrMove { to_mr, mr, rk } => {
                if to_mr {
                    write!(f, "{} = R{rk}", mr_name(mr))
                } else {
                    write!(f, "R{rk} = {}", mr_name(mr))
                }
            }
            Compute::Multi { mul, alu, rm, ra, regs } => {
                write!(f, "{}, {}", mul.format(rm, regs.xm, regs.ym), alu.format(ra, regs.xa, regs.ya))
            }
            Compute::MultiAddSub { float, rm, ra, rs, regs } => {
                let p = if float { 'F' } else { 'R' };
                let m = if float {
                    format!("F{rm} = F{} * F{}", regs.xm, regs.ym)
                } else {
                    format!("R{rm} = R{} * R{} (SSFR)", regs.xm, regs.ym)
                };
                write!(f, "{m}, {p}{ra} = {p}{} + {p}{}, {p}{rs} = {p}{} - {p}{}", regs.xa, regs.ya, regs.xa, regs.ya)
            }
            Compute::Unknown(cf) => write!(f, "?compute {cf:#08x}"),
        }
    }
}
