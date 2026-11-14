//! Analog Devices ADSP-2106x ("SHARC") DSP: instruction decoder,
//! disassembler and interpreter, for the ADSP-21062s of Sega's Model 2B.
//!
//! Written clean-room from Analog Devices' *ADSP-2106x SHARC User's
//! Manual*, Revision 2.1 (March 2004). References such as "UM A-28" are to
//! that manual's page numbers, "UM §3.4.1" to its sections. What is
//! implemented, the open questions and every place the manual was ambiguous
//! (and how that was resolved) are in `docs/hw/adsp2106x.md`.
//!
//! Decoding is separate from execution, as in the `mb86235` crate: [`decode`]
//! turns a 48-bit word into an [`Insn`], which [`cpu::Sharc`] executes and
//! which a recompiler can later translate. Encodings the manual doesn't
//! define decode to [`Op::Unknown`] (or [`Compute::Unknown`]) and are
//! reported by the interpreter, never skipped.

pub mod codegen;
pub mod compute;
pub mod cpu;
pub mod float;
pub mod mem;

pub use compute::{AluOp, Compute, MulOp, ShiftOp};
use state::bits;
use state::sext;
use std::fmt;

/// Condition code that always holds in an `IF` (and never terminates a
/// `DO UNTIL`): TRUE / FOREVER (UM Table 3.2).
pub const TRUE: u8 = 31;
/// Loop counter expired: LCE in `DO UNTIL`, NOT LCE in `IF` (UM Table 3.2).
pub const LCE: u8 = 15;

/// Condition mnemonic (UM Table 3.2). Code 15 reads NOT LCE and code 31
/// TRUE in an `IF`; in a `DO UNTIL` (`term`) they are LCE and FOREVER.
pub fn cond_name(c: u8, term: bool) -> &'static str {
    const N: [&str; 32] = [
        "EQ",
        "LT",
        "LE",
        "AC",
        "AV",
        "MV",
        "MS",
        "SV",
        "SZ",
        "FLAG0_IN",
        "FLAG1_IN",
        "FLAG2_IN",
        "FLAG3_IN",
        "TF",
        "BM",
        "NOT LCE",
        "NE",
        "GE",
        "GT",
        "NOT AC",
        "NOT AV",
        "NOT MV",
        "NOT MS",
        "NOT SV",
        "NOT SZ",
        "NOT FLAG0_IN",
        "NOT FLAG1_IN",
        "NOT FLAG2_IN",
        "NOT FLAG3_IN",
        "NOT TF",
        "NBM",
        "TRUE",
    ];
    match (c & 31, term) {
        (15, true) => "LCE",
        (31, true) => "FOREVER",
        (c, _) => N[c as usize],
    }
}

/// Universal register name (UM §A.4, Figures A.1 and A.2), or `None` for a
/// code the manual leaves undefined.
pub fn ureg_name(code: u8) -> Option<String> {
    let n = code & 15;
    Some(match code >> 4 {
        0 => format!("R{n}"),
        1 => format!("I{n}"),
        2 => format!("M{n}"),
        3 => format!("L{n}"),
        4 => format!("B{n}"),
        6 => match n {
            0 => "FADDR".into(),
            1 => "DADDR".into(),
            3 => "PC".into(),
            4 => "PCSTK".into(),
            5 => "PCSTKP".into(),
            6 => "LADDR".into(),
            7 => "CURLCNTR".into(),
            8 => "LCNTR".into(),
            _ => return None,
        },
        7 => match n {
            0 => "USTAT1".into(),
            1 => "USTAT2".into(),
            9 => "IRPTL".into(),
            10 => "MODE2".into(),
            11 => "MODE1".into(),
            12 => "ASTAT".into(),
            13 => "IMASK".into(),
            14 => "STKY".into(),
            15 => "IMASKP".into(),
            _ => return None,
        },
        0xd => match n {
            0xb => "PX".into(),
            0xc => "PX1".into(),
            0xd => "PX2".into(),
            0xe => "TPERIOD".into(),
            0xf => "TCOUNT".into(),
            _ => return None,
        },
        _ => return None,
    })
}

/// Read-only universal registers: FADDR, DADDR and PC (UM Table 3.1).
pub fn ureg_writable(code: u8) -> bool {
    ureg_name(code).is_some() && !matches!(code, 0x60 | 0x61 | 0x63)
}

fn ureg(code: u8) -> String {
    ureg_name(code).unwrap_or_else(|| format!("?ureg{code:02x}"))
}

/// Address modifier: an M register (same DAG as the I register) or an
/// immediate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    M(u8),
    Imm(i32),
}

/// A DAG address: I register 0-15 (I0-I7 are DAG1 and address data
/// memory, I8-I15 are DAG2 and address program memory), its modifier, and
/// post-modify with update (`DM(I,M)`) or pre-modify without update
/// (`DM(M,I)`), UM §4.3.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Addr {
    pub i: u8,
    pub modifier: Modifier,
    pub post: bool,
}

impl Addr {
    /// DAG2, i.e. a program memory access.
    pub fn pm(&self) -> bool {
        self.i >= 8
    }
}

/// A transfer between memory and a register: `reg` is a universal
/// register code (register file locations R0-R15 are codes 0-15).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Move {
    pub addr: Addr,
    /// Register to memory.
    pub write: bool,
    pub reg: u8,
}

/// Branch target (UM A-28 to A-33).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Abs(u32),
    /// Added to the address of the branch instruction itself.
    Rel(i32),
    /// `(Md,Ic)`: DAG2 pre-modify, the I register isn't updated.
    Indirect {
        i: u8,
        m: u8,
    },
}

/// Loop counter load of a `DO UNTIL LCE` (type 12).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Count {
    Imm(u16),
    Ureg(u8),
}

/// System register bit operation (type 18, UM A-46).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BitOp {
    Set,
    Clr,
    Tgl,
    Tst,
    Xor,
}

/// Stack and cache operations of type 20 (UM A-50).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StackOps {
    pub push_loop: bool,
    pub pop_loop: bool,
    pub push_sts: bool,
    pub pop_sts: bool,
    pub push_pcstk: bool,
    pub pop_pcstk: bool,
    pub flush_cache: bool,
}

/// One decoded instruction. The type numbers are the manual's (UM §A.2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    /// Type 1: compute with parallel DM and PM register-file transfers.
    Dual { compute: Compute, dm: Move, pm: Move },
    /// Type 2: conditional compute.
    Compute { cond: u8, compute: Compute },
    /// Type 3 (any universal register, M-register modifier) and type 4
    /// (register file, 6-bit immediate modifier).
    Transfer { cond: u8, compute: Compute, mv: Move },
    /// Type 5: universal register to universal register.
    RegMove { cond: u8, compute: Compute, src: u8, dst: u8 },
    /// Type 6: shifter operation with immediate Y operand (8 bits, or
    /// bit6 in bits 5-0 and len6 in bits 11-6), optional DM/PM transfer.
    ShiftImm { cond: u8, op: ShiftOp, rn: u8, rx: u8, imm: u16, mv: Option<Move> },
    /// Type 7: `MODIFY(I,M)`.
    Modify { cond: u8, compute: Compute, i: u8, m: u8 },
    /// Types 8 and 9: jump or call. `else_`: the compute runs only when
    /// the condition is false.
    Branch {
        cond: u8,
        call: bool,
        target: Target,
        delayed: bool,
        loop_abort: bool,
        clear_int: bool,
        compute: Compute,
        else_: bool,
    },
    /// Type 10: `IF cond JUMP target, ELSE compute, DM transfer`.
    JumpElseMove { cond: u8, target: Target, compute: Compute, mv: Move },
    /// Type 11: RTS or RTI.
    Return { cond: u8, rti: bool, delayed: bool, loop_reentry: bool, compute: Compute, else_: bool },
    /// Types 12 (`count` given, terminates on LCE) and 13. `end` is the
    /// absolute address of the last instruction of the loop.
    Do { count: Option<Count>, term: u8, end: u32 },
    /// Type 14: direct address (DM 32-bit, PM 24-bit).
    Direct { pm: bool, write: bool, reg: u8, addr: u32 },
    /// Type 15: `DM(data32,I)` pre-modify with an immediate, no update.
    IndirectImm { write: bool, reg: u8, i: u8, offset: i32 },
    /// Type 16: immediate data to memory, post-modify.
    StoreImm { addr: Addr, data: u32 },
    /// Type 17: immediate data to a universal register.
    LoadImm { reg: u8, data: u32 },
    /// Type 18: `BIT op sreg data`; `sreg` is the universal register code.
    Bit { op: BitOp, sreg: u8, data: u32 },
    /// Type 19: `MODIFY(I,data)` or `BITREV(I,data)`.
    ModifyImm { i: u8, data: i32, bitrev: bool },
    /// Type 20.
    Stacks(StackOps),
    /// Type 21.
    Nop,
    /// Type 22.
    Idle,
    /// Type 23 (ADSP-21061 only).
    Idle16,
    /// Type 24: `CJUMP` (compiler call: `JUMP target (DB), R2=I6, I6=I7`).
    CJump { target: Target },
    /// Type 24: `RFRAME` (`I7=I6, I6=DM(0,I6)`).
    RFrame,
    /// An encoding the manual doesn't define.
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Insn {
    pub addr: u32,
    pub word: u64,
    pub op: Op,
}

/// Mask of a 48-bit instruction word.
pub const WORD_MASK: u64 = (1 << 48) - 1;

fn compute(w: u64) -> Compute {
    Compute::decode(bits(w, 22, 0))
}

/// The DAG half (0 = DAG1, 8 = DAG2) for a G bit.
fn dag(g: u32) -> u8 {
    (g as u8) << 3
}

fn ureg_ok(code: u32, write: bool) -> bool {
    let c = code as u8;
    if write {
        ureg_writable(c)
    } else {
        ureg_name(c).is_some()
    }
}

/// Decode the 48-bit word at `addr` (UM Appendix A, compute field per
/// Appendix B).
pub fn decode(addr: u32, word: u64) -> Insn {
    let w = word & WORD_MASK;
    let op = decode_op(addr, w);
    Insn { addr, word: w, op }
}

fn decode_op(addr: u32, w: u64) -> Op {
    let cond = bits(w, 37, 33) as u8;
    match bits(w, 47, 45) {
        // Type 1 (UM A-16).
        0b001 => {
            let dm = Move {
                addr: Addr { i: bits(w, 43, 41) as u8, modifier: Modifier::M(bits(w, 40, 38) as u8), post: true },
                write: bits(w, 44, 44) == 1,
                reg: bits(w, 36, 33) as u8,
            };
            let pm = Move {
                addr: Addr {
                    i: 8 | bits(w, 32, 30) as u8,
                    modifier: Modifier::M(8 | bits(w, 29, 27) as u8),
                    post: true,
                },
                write: bits(w, 37, 37) == 1,
                reg: bits(w, 26, 23) as u8,
            };
            Op::Dual { compute: compute(w), dm, pm }
        }
        // Type 3 (UM A-19).
        0b010 => {
            let d = dag(bits(w, 32, 32));
            let write = bits(w, 31, 31) == 1;
            let reg = bits(w, 30, 23);
            // Data written to memory comes from the register (read), and
            // vice versa.
            if !ureg_ok(reg, !write) {
                return Op::Unknown;
            }
            let addr = Addr {
                i: d | bits(w, 43, 41) as u8,
                modifier: Modifier::M(d | bits(w, 40, 38) as u8),
                post: bits(w, 44, 44) == 1,
            };
            Op::Transfer { cond, compute: compute(w), mv: Move { addr, write, reg: reg as u8 } }
        }
        0b011 if bits(w, 44, 44) == 0 => {
            // Type 4 (UM A-21).
            let d = dag(bits(w, 40, 40));
            let addr = Addr {
                i: d | bits(w, 43, 41) as u8,
                modifier: Modifier::Imm(sext(bits(w, 32, 27), 6)),
                post: bits(w, 38, 38) == 1,
            };
            let mv = Move { addr, write: bits(w, 39, 39) == 1, reg: bits(w, 26, 23) as u8 };
            Op::Transfer { cond, compute: compute(w), mv }
        }
        0b011 => {
            // Type 5 (UM A-23).
            let (src, dst) = (bits(w, 43, 36), bits(w, 30, 23));
            if !ureg_ok(src, false) || !ureg_ok(dst, true) {
                return Op::Unknown;
            }
            Op::RegMove { cond: bits(w, 35, 31) as u8, compute: compute(w), src: src as u8, dst: dst as u8 }
        }
        0b100 if bits(w, 44, 44) == 0 => {
            // Type 6 with a data access (UM A-25).
            let d = dag(bits(w, 32, 32));
            let mv = Move {
                addr: Addr {
                    i: d | bits(w, 43, 41) as u8,
                    modifier: Modifier::M(d | bits(w, 40, 38) as u8),
                    post: true,
                },
                write: bits(w, 31, 31) == 1,
                reg: bits(w, 26, 23) as u8,
            };
            shift_imm(w, cond, Some(mv))
        }
        0b100 => {
            // Type 16 (UM A-42).
            let d = dag(bits(w, 37, 37));
            let addr =
                Addr { i: d | bits(w, 43, 41) as u8, modifier: Modifier::M(d | bits(w, 40, 38) as u8), post: true };
            Op::StoreImm { addr, data: w as u32 }
        }
        0b101 => {
            // Type 15 (UM A-41).
            let pm = bits(w, 44, 44) == 1;
            let write = bits(w, 40, 40) == 1;
            let reg = bits(w, 39, 32);
            if !ureg_ok(reg, !write) {
                return Op::Unknown;
            }
            let offset = if pm { sext(w as u32 & 0xff_ffff, 24) } else { w as u32 as i32 };
            Op::IndirectImm { write, reg: reg as u8, i: dag(pm as u32) | bits(w, 43, 41) as u8, offset }
        }
        // Type 10 (UM A-33): 110 indirect, 111 PC-relative.
        t @ (0b110 | 0b111) => {
            let target = if t == 0b110 {
                Target::Indirect { i: 8 | bits(w, 32, 30) as u8, m: 8 | bits(w, 29, 27) as u8 }
            } else {
                Target::Rel(sext(bits(w, 32, 27), 6))
            };
            let mv = Move {
                addr: Addr { i: bits(w, 43, 41) as u8, modifier: Modifier::M(bits(w, 40, 38) as u8), post: true },
                write: bits(w, 44, 44) == 1,
                reg: bits(w, 26, 23) as u8,
            };
            Op::JumpElseMove { cond, target, compute: compute(w), mv }
        }
        _ => decode_group0(addr, w, cond),
    }
}

/// Type 6: the shifter operation field. SHIFTOP holds bits 7-2 of the
/// shifter opcode of UM Table B.6 (whose bits 1-0 are always zero);
/// docs/hw/adsp2106x.md explains how that was confirmed.
fn shift_imm(w: u64, cond: u8, mv: Option<Move>) -> Op {
    if bits(w, 22, 22) != 0 {
        return Op::Unknown;
    }
    match ShiftOp::from_opcode((bits(w, 21, 16) << 2) as u8) {
        Some(op) if op.takes_immediate() => Op::ShiftImm {
            cond,
            op,
            rn: bits(w, 7, 4) as u8,
            rx: bits(w, 3, 0) as u8,
            imm: (bits(w, 15, 8) | bits(w, 30, 27) << 8) as u16,
            mv,
        },
        _ => Op::Unknown,
    }
}

fn decode_group0(addr: u32, w: u64, cond: u8) -> Op {
    let rel24 = || sext(w as u32 & 0xff_ffff, 24);
    match bits(w, 44, 40) {
        0b00000 => match bits(w, 39, 32) {
            0 => Op::Nop,
            0x80 => Op::Idle,
            0xa0 => Op::Idle16,
            _ => Op::Unknown,
        },
        // Type 2 (UM A-17).
        0b00001 => Op::Compute { cond, compute: compute(w) },
        // Type 6 without a data access.
        0b00010 => shift_imm(w, cond, None),
        // Type 7 (UM A-26).
        0b00100 => {
            let d = dag(bits(w, 38, 38));
            Op::Modify { cond, compute: compute(w), i: d | bits(w, 32, 30) as u8, m: d | bits(w, 29, 27) as u8 }
        }
        // Type 8 (UM A-29).
        t @ (0b00110 | 0b00111) => Op::Branch {
            cond,
            call: bits(w, 39, 39) == 1,
            target: if t == 0b00110 { Target::Abs(w as u32 & 0xff_ffff) } else { Target::Rel(rel24()) },
            delayed: bits(w, 26, 26) == 1,
            loop_abort: bits(w, 38, 38) == 1,
            clear_int: bits(w, 24, 24) == 1,
            compute: Compute::None,
            else_: false,
        },
        // Type 9 (UM A-31).
        t @ (0b01000 | 0b01001) => Op::Branch {
            cond,
            call: bits(w, 39, 39) == 1,
            target: if t == 0b01000 {
                Target::Indirect { i: 8 | bits(w, 32, 30) as u8, m: 8 | bits(w, 29, 27) as u8 }
            } else {
                Target::Rel(sext(bits(w, 32, 27), 6))
            },
            delayed: bits(w, 26, 26) == 1,
            loop_abort: bits(w, 38, 38) == 1,
            clear_int: bits(w, 24, 24) == 1,
            compute: compute(w),
            else_: bits(w, 25, 25) == 1,
        },
        // Type 11 (UM A-35).
        t @ (0b01010 | 0b01011) => Op::Return {
            cond,
            rti: t == 0b01011,
            delayed: bits(w, 26, 26) == 1,
            loop_reentry: t == 0b01010 && bits(w, 24, 24) == 1,
            compute: compute(w),
            else_: bits(w, 25, 25) == 1,
        },
        // Type 12 (UM A-36, A-37).
        0b01100 => Op::Do {
            count: Some(Count::Imm(bits(w, 39, 24) as u16)),
            term: LCE,
            end: addr.wrapping_add(rel24() as u32) & 0xff_ffff,
        },
        0b01101 => {
            let reg = bits(w, 39, 32);
            if !ureg_ok(reg, false) {
                return Op::Unknown;
            }
            Op::Do {
                count: Some(Count::Ureg(reg as u8)),
                term: LCE,
                end: addr.wrapping_add(rel24() as u32) & 0xff_ffff,
            }
        }
        // Type 13 (UM A-38).
        0b01110 => Op::Do { count: None, term: cond, end: addr.wrapping_add(rel24() as u32) & 0xff_ffff },
        // Type 17 (UM A-43).
        0b01111 => {
            let reg = bits(w, 39, 32);
            if !ureg_ok(reg, true) {
                return Op::Unknown;
            }
            Op::LoadImm { reg: reg as u8, data: w as u32 }
        }
        // Type 14 (UM A-40): 000 100 G D.
        0b10000..=0b10011 => {
            let pm = bits(w, 41, 41) == 1;
            let write = bits(w, 40, 40) == 1;
            let reg = bits(w, 39, 32);
            if !ureg_ok(reg, !write) {
                return Op::Unknown;
            }
            Op::Direct { pm, write, reg: reg as u8, addr: if pm { w as u32 & 0xff_ffff } else { w as u32 } }
        }
        // Type 18 (UM A-47).
        0b10100 => {
            let op = match bits(w, 39, 37) {
                0b000 => BitOp::Set,
                0b001 => BitOp::Clr,
                0b010 => BitOp::Tgl,
                0b100 => BitOp::Tst,
                0b101 => BitOp::Xor,
                _ => return Op::Unknown,
            };
            let sreg = 0x70 | bits(w, 35, 32) as u8;
            if ureg_name(sreg).is_none() {
                return Op::Unknown;
            }
            Op::Bit { op, sreg, data: w as u32 }
        }
        // Type 19 (UM A-48, A-49).
        0b10110 => {
            let pm = bits(w, 38, 38) == 1;
            let data = if pm { sext(w as u32 & 0xff_ffff, 24) } else { w as u32 as i32 };
            Op::ModifyImm { i: dag(pm as u32) | bits(w, 34, 32) as u8, data, bitrev: bits(w, 39, 39) == 1 }
        }
        // Type 20 (UM A-50).
        0b10111 => Op::Stacks(StackOps {
            push_loop: bits(w, 39, 39) == 1,
            pop_loop: bits(w, 38, 38) == 1,
            push_sts: bits(w, 37, 37) == 1,
            pop_sts: bits(w, 36, 36) == 1,
            push_pcstk: bits(w, 35, 35) == 1,
            pop_pcstk: bits(w, 34, 34) == 1,
            flush_cache: bits(w, 33, 33) == 1,
        }),
        // Type 24 (UM A-54, A-55): only the encodings shown there.
        0b11000 => match bits(w, 39, 24) {
            0x0400 => Op::CJump { target: Target::Abs(w as u32 & 0xff_ffff) },
            0x4400 => Op::CJump { target: Target::Rel(rel24()) },
            _ => Op::Unknown,
        },
        0b11001 if bits(w, 39, 0) == 0 => Op::RFrame,
        _ => Op::Unknown,
    }
}

/// Control flow of an instruction, for code discovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Next,
    /// Unconditional jump; `delayed`: the next two words run first.
    Jump {
        target: Option<u32>,
        delayed: bool,
    },
    /// Conditional jump: may also fall through.
    Branch {
        target: Option<u32>,
        delayed: bool,
    },
    Call {
        target: Option<u32>,
        delayed: bool,
    },
    Return {
        cond: bool,
        delayed: bool,
    },
    /// `DO ... UNTIL`: falls through into the loop body, which ends at `end`.
    Do {
        end: u32,
    },
    Idle,
    Invalid,
}

impl Target {
    /// The absolute address, when the instruction alone determines it.
    pub fn resolve(&self, pc: u32) -> Option<u32> {
        match *self {
            Target::Abs(a) => Some(a),
            Target::Rel(d) => Some(pc.wrapping_add(d as u32) & 0xff_ffff),
            Target::Indirect { .. } => None,
        }
    }
}

impl Insn {
    pub fn flow(&self) -> Flow {
        match self.op {
            Op::Branch { cond, call, target, delayed, .. } => {
                let target = target.resolve(self.addr);
                if call {
                    Flow::Call { target, delayed }
                } else if cond == TRUE {
                    Flow::Jump { target, delayed }
                } else {
                    Flow::Branch { target, delayed }
                }
            }
            Op::JumpElseMove { target, .. } => Flow::Branch { target: target.resolve(self.addr), delayed: false },
            Op::CJump { target } => Flow::Call { target: target.resolve(self.addr), delayed: true },
            Op::Return { cond, delayed, .. } => Flow::Return { cond: cond != TRUE, delayed },
            Op::Do { end, .. } => Flow::Do { end },
            Op::Idle | Op::Idle16 => Flow::Idle,
            Op::Unknown => Flow::Invalid,
            Op::Dual { compute, .. }
            | Op::Compute { compute, .. }
            | Op::Transfer { compute, .. }
            | Op::RegMove { compute, .. }
            | Op::Modify { compute, .. }
                if compute.is_unknown() =>
            {
                Flow::Invalid
            }
            _ => Flow::Next,
        }
    }

    /// True if the word is an encoding the manual doesn't define, in the
    /// instruction or its compute field.
    pub fn is_unknown(&self) -> bool {
        self.flow() == Flow::Invalid
            || matches!(self.op, Op::Branch { compute, .. } | Op::JumpElseMove { compute, .. } | Op::Return { compute, .. } if compute.is_unknown())
    }
}

impl fmt::Display for Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let space = if self.pm() { "PM" } else { "DM" };
        let i = format!("I{}", self.i);
        let m = match self.modifier {
            Modifier::M(m) => format!("M{m}"),
            Modifier::Imm(v) => format!("{v}"),
        };
        if self.post {
            write!(f, "{space}({i},{m})")
        } else {
            write!(f, "{space}({m},{i})")
        }
    }
}

impl fmt::Display for Move {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.write {
            write!(f, "{} = {}", self.addr, ureg(self.reg))
        } else {
            write!(f, "{} = {}", ureg(self.reg), self.addr)
        }
    }
}

/// `target` as the manual writes it; a PC-relative target is followed by
/// the absolute address in braces (the assembler's comment syntax).
fn target_str(t: Target, pc: u32) -> String {
    match t {
        Target::Abs(a) => format!("{a:#x}"),
        Target::Rel(d) => format!("(PC,{d}) {{{:#x}}}", pc.wrapping_add(d as u32) & 0xff_ffff),
        Target::Indirect { i, m } => format!("(M{m},I{i})"),
    }
}

fn if_prefix(cond: u8) -> String {
    if cond == TRUE {
        String::new()
    } else {
        format!("IF {} ", cond_name(cond, false))
    }
}

impl fmt::Display for Insn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts: Vec<String> = Vec::new();
        let push_compute = |parts: &mut Vec<String>, c: Compute| {
            if c != Compute::None {
                parts.push(c.to_string());
            }
        };
        let s = match self.op {
            Op::Dual { compute, dm, pm } => {
                push_compute(&mut parts, compute);
                parts.push(dm.to_string());
                parts.push(pm.to_string());
                parts.join(", ")
            }
            Op::Compute { cond, compute } => {
                let c = if compute == Compute::None { "NOP".to_string() } else { compute.to_string() };
                format!("{}{c}", if_prefix(cond))
            }
            Op::Transfer { cond, compute, mv } => {
                push_compute(&mut parts, compute);
                parts.push(mv.to_string());
                format!("{}{}", if_prefix(cond), parts.join(", "))
            }
            Op::RegMove { cond, compute, src, dst } => {
                push_compute(&mut parts, compute);
                parts.push(format!("{} = {}", ureg(dst), ureg(src)));
                format!("{}{}", if_prefix(cond), parts.join(", "))
            }
            Op::ShiftImm { cond, op, rn, rx, imm, mv } => {
                parts.push(op.format_imm(rn, rx, imm));
                if let Some(mv) = mv {
                    parts.push(mv.to_string());
                }
                format!("{}{}", if_prefix(cond), parts.join(", "))
            }
            Op::Modify { cond, compute, i, m } => {
                push_compute(&mut parts, compute);
                parts.push(format!("MODIFY (I{i},M{m})"));
                format!("{}{}", if_prefix(cond), parts.join(", "))
            }
            Op::Branch { cond, call, target, delayed, loop_abort, clear_int, compute, else_ } => {
                let mut mods = Vec::new();
                if delayed {
                    mods.push("DB");
                }
                if !call && loop_abort {
                    mods.push("LA");
                }
                if !call && clear_int {
                    mods.push("CI");
                }
                let mods = if mods.is_empty() { String::new() } else { format!(" ({})", mods.join(",")) };
                let mut s = format!(
                    "{}{} {}{mods}",
                    if_prefix(cond),
                    if call { "CALL" } else { "JUMP" },
                    target_str(target, self.addr)
                );
                if compute != Compute::None {
                    s += &format!(", {}{compute}", if else_ { "ELSE " } else { "" });
                }
                s
            }
            Op::JumpElseMove { cond, target, compute, mv } => {
                push_compute(&mut parts, compute);
                parts.push(mv.to_string());
                format!("{}JUMP {}, ELSE {}", if_prefix(cond), target_str(target, self.addr), parts.join(", "))
            }
            Op::Return { cond, rti, delayed, loop_reentry, compute, else_ } => {
                let mut mods = Vec::new();
                if delayed {
                    mods.push("DB");
                }
                if loop_reentry {
                    mods.push("LR");
                }
                let mods = if mods.is_empty() { String::new() } else { format!(" ({})", mods.join(",")) };
                let mut s = format!("{}{}{mods}", if_prefix(cond), if rti { "RTI" } else { "RTS" });
                if compute != Compute::None {
                    s += &format!(", {}{compute}", if else_ { "ELSE " } else { "" });
                }
                s
            }
            Op::Do { count, term, end } => {
                let lc = match count {
                    Some(Count::Imm(n)) => format!("LCNTR = {n}, "),
                    Some(Count::Ureg(r)) => format!("LCNTR = {}, ", ureg(r)),
                    None => String::new(),
                };
                format!("{lc}DO {end:#x} UNTIL {}", cond_name(term, true))
            }
            Op::Direct { pm, write, reg, addr } => {
                let m = format!("{}({addr:#x})", if pm { "PM" } else { "DM" });
                if write {
                    format!("{m} = {}", ureg(reg))
                } else {
                    format!("{} = {m}", ureg(reg))
                }
            }
            Op::IndirectImm { write, reg, i, offset } => {
                let off =
                    if i >= 8 { format!("{:#x}", offset as u32 & 0xff_ffff) } else { format!("{:#x}", offset as u32) };
                let m = format!("{}({off},I{i})", if i >= 8 { "PM" } else { "DM" });
                if write {
                    format!("{m} = {}", ureg(reg))
                } else {
                    format!("{} = {m}", ureg(reg))
                }
            }
            Op::StoreImm { addr, data } => format!("{addr} = {data:#x}"),
            Op::LoadImm { reg, data } => format!("{} = {data:#x}", ureg(reg)),
            Op::Bit { op, sreg, data } => {
                let o = match op {
                    BitOp::Set => "SET",
                    BitOp::Clr => "CLR",
                    BitOp::Tgl => "TGL",
                    BitOp::Tst => "TST",
                    BitOp::Xor => "XOR",
                };
                format!("BIT {o} {} {data:#x}", ureg(sreg))
            }
            Op::ModifyImm { i, data, bitrev } => {
                let d = if i >= 8 { format!("{:#x}", data as u32 & 0xff_ffff) } else { format!("{:#x}", data as u32) };
                format!("{} (I{i},{d})", if bitrev { "BITREV" } else { "MODIFY" })
            }
            Op::Stacks(s) => {
                let all = [
                    (s.push_loop, "PUSH LOOP"),
                    (s.pop_loop, "POP LOOP"),
                    (s.push_sts, "PUSH STS"),
                    (s.pop_sts, "POP STS"),
                    (s.push_pcstk, "PUSH PCSTK"),
                    (s.pop_pcstk, "POP PCSTK"),
                    (s.flush_cache, "FLUSH CACHE"),
                ];
                let v: Vec<&str> = all.iter().filter(|p| p.0).map(|p| p.1).collect();
                if v.is_empty() {
                    "NOP {type 20, no operation}".into()
                } else {
                    v.join(", ")
                }
            }
            Op::Nop => "NOP".into(),
            Op::Idle => "IDLE".into(),
            Op::Idle16 => "IDLE16".into(),
            Op::CJump { target } => format!("CJUMP {} (DB)", target_str(target, self.addr)),
            Op::RFrame => "RFRAME".into(),
            Op::Unknown => format!(".word {:#014x} {{undefined}}", self.word),
        };
        f.write_str(&s)
    }
}

#[cfg(test)]
mod tests;
