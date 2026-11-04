//! Motorola 68000: interpreter and disassembler.
//!
//! Used for the Model 2 sound CPUs (the sound board's 68EC000 and the DSB2
//! MPEG board's 68000). Written from the M68000 Programmer's Reference
//! Manual and checked against the public SingleStepTests 68000 suite
//! (`make m68k-tests`).

pub mod cpu;
mod disasm;

pub use cpu::{Bus, Cpu};
pub use disasm::disasm;

// Addressing-mode classes as bitmasks over `mode_id`: bit 0 Dn, 1 An,
// 2 (An), 3 (An)+, 4 -(An), 5 d16(An), 6 d8(An,Xn), 7 abs.w, 8 abs.l,
// 9 d16(PC), 10 d8(PC,Xn), 11 #imm.
pub(crate) const ALL: u16 = 0xfff;
pub(crate) const DATA: u16 = ALL & !(1 << 1);
pub(crate) const MEMORY: u16 = ALL & !3;
pub(crate) const CONTROL: u16 = 0x7e4;
pub(crate) const ALTERABLE: u16 = 0x1ff;
pub(crate) const DATA_ALT: u16 = DATA & ALTERABLE;
pub(crate) const MEM_ALT: u16 = MEMORY & ALTERABLE;
pub(crate) const CTRL_ALT: u16 = CONTROL & ALTERABLE;
pub(crate) const POSTINC: u16 = 1 << 3;
pub(crate) const PREDEC: u16 = 1 << 4;
pub(crate) const IMM: u16 = 1 << 11;

/// Index of an effective-address mode/register pair in the class masks.
pub(crate) fn mode_id(mode: u16, reg: u16) -> Option<u16> {
    match (mode & 7, reg & 7) {
        (7, r) if r <= 4 => Some(7 + r),
        (7, _) => None,
        (m, _) => Some(m),
    }
}

/// True if the mode/register pair is in `class`.
pub(crate) fn ea_ok(mode: u16, reg: u16, class: u16) -> bool {
    mode_id(mode, reg).is_some_and(|i| class & (1 << i) != 0)
}

/// Operand size.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Sz {
    B,
    W,
    L,
}

impl Sz {
    /// The usual two-bit size field: 00 byte, 01 word, 10 long.
    pub(crate) fn from_bits(b: u16) -> Option<Sz> {
        match b & 3 {
            0 => Some(Sz::B),
            1 => Some(Sz::W),
            2 => Some(Sz::L),
            _ => None,
        }
    }
    pub(crate) fn bytes(self) -> u32 {
        match self {
            Sz::B => 1,
            Sz::W => 2,
            Sz::L => 4,
        }
    }
    pub(crate) fn mask(self) -> u32 {
        match self {
            Sz::B => 0xff,
            Sz::W => 0xffff,
            Sz::L => 0xffff_ffff,
        }
    }
    pub(crate) fn msb(self) -> u32 {
        match self {
            Sz::B => 0x80,
            Sz::W => 0x8000,
            Sz::L => 0x8000_0000,
        }
    }
    pub(crate) fn suffix(self) -> &'static str {
        match self {
            Sz::B => ".b",
            Sz::W => ".w",
            Sz::L => ".l",
        }
    }
    /// Sign-extend the low `self` bits of `v` to 32.
    pub(crate) fn sext(self, v: u32) -> u32 {
        match self {
            Sz::B => v as u8 as i8 as u32,
            Sz::W => v as u16 as i16 as u32,
            Sz::L => v,
        }
    }
}
