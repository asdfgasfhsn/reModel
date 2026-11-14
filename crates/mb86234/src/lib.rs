//! Fujitsu MB86234, the geometry processor ("TGP") of Sega's Model 2 and
//! 2A boards: an interpreter for its 32-bit instruction words and a
//! disassembler.
//!
//! No manual for the chip is public and nothing here comes from an
//! emulator: the encodings were read from the programs the games upload
//! (`srallyc`'s, then `vf2`'s) and pinned by running their routines and
//! checking the mathematics they compute and what their callers expect
//! (`docs/hw/mb86234.md` lists every encoding with how sure it is). Where an encoding is a hypothesis
//! the code says so; words of a form never seen are counted in
//! [`Cpu::unknown`] and executed as no-ops.
//!
//! The model: one word a step (no delay slots, no cycle timing); a word's
//! ALU operation and its transfer read their operands at the start of the
//! word and write at the end. Two RAMs of 512 words (RAM0, RAM1), one
//! address space for indirect accesses (RAM0 at 0, RAM1 at 0x200) except
//! where the transfer type fixes the RAM, program memory readable as
//! tables, an
//! input and an output FIFO, a 24-bit external bus that reaches either the
//! board's shared RAM or the function unit's ports ([`funcs`]), and in both
//! modes the board's data ROM from [`ROM_BASE`].

pub mod disasm;
pub mod funcs;

use std::collections::{BTreeMap, VecDeque};

pub use funcs::Funcs;

/// Words of internal RAM (RAM0 then RAM1, as indirect addresses see them).
pub const RAM_WORDS: usize = 0x400;
/// Program memory: the largest program the 16-bit jump targets reach is
/// more than any board loads; `srallyc`'s is 1,665 words.
pub const PROGRAM_WORDS: usize = 0x1000;
/// The length of a circular indirect operand's ring, in words (`vf2`'s
/// ring in shared RAM is 64 words: the i960 wraps its writes there; what
/// sets the length on the chip, if anything, is unknown).
pub const RING: u32 = 64;
/// Return addresses kept (calls nest at most three deep in `srallyc`).
pub const STACK_DEPTH: usize = 16;

/// Register numbers of the transfer fields (`docs/hw/mb86234.md`).
pub mod reg {
    pub const AR0: u8 = 0x00;
    pub const AR1: u8 = 0x01;
    pub const AR2: u8 = 0x02;
    pub const AR3: u8 = 0x03;
    /// The loop counter `BF00` counts down (8 bits).
    pub const LC: u8 = 0x0c;
    /// A second loop counter, which `BF10` counts down.
    pub const LC2: u8 = 0x0d;
    /// The multiplier's first input and the ALU's second operand.
    pub const R10: u8 = 0x10;
    /// The multiplier's second input.
    pub const R13: u8 = 0x13;
    pub const ACC: u8 = 0x19;
    /// The product.
    pub const P: u8 = 0x1c;
    /// Shift count of the shift operations.
    pub const SH: u8 = 0x1f;
    /// An output status register (the program copies its flag word there).
    pub const ST: u8 = 0x20;
    /// Reading takes the next word of the input FIFO.
    pub const FIFO_IN: u8 = 0x21;
    /// Writing gives the host a word.
    pub const FIFO_OUT: u8 = 0x22;
    /// The external bus's mode: bit 22 set reaches the shared RAM, clear
    /// the function unit (`63400000` / `63000000` set it).
    pub const BUS: u8 = 0x23;
}

/// The board side of the chip's external bus.
/// Where the board's data ROM starts on the external bus (words).
pub const ROM_BASE: u32 = 0x80_0000;

pub trait Bus {
    /// A read of the board's shared RAM (on Model 2A the geometrizer's
    /// buffer RAM, in 32-bit words).
    fn read(&mut self, addr: u32) -> u32;
    fn write(&mut self, addr: u32, v: u32);
}

/// A bus with nothing on it (tests, programs that don't use it).
pub struct NoBus;

impl Bus for NoBus {
    fn read(&mut self, _: u32) -> u32 {
        0
    }
    fn write(&mut self, _: u32, _: u32) {}
}

/// Why [`Cpu::run`] returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// The next word reads the input FIFO and it is empty.
    Input,
    /// The step budget ran out.
    Limit,
}

fn sext(v: u32, bits: u32) -> u32 {
    let s = 32 - bits;
    (((v << s) as i32) >> s) as u32
}

/// An operand's address: an index and the base added to it (0 unless the
/// operand is indexed).
#[derive(Clone, Copy)]
struct Ea {
    index: u32,
    base: u32,
}

impl Ea {
    /// The address in space `s`: the 24-bit sum (internally, 10 bits:
    /// RAM0 then RAM1).
    fn at(self, s: Space) -> u32 {
        let sum = self.index.wrapping_add(self.base);
        match s {
            Space::Int => sum & 0x3ff,
            _ => sum & 0xff_ffff,
        }
    }
}

/// A float word's value. The chip has no infinities or NaNs (*hypothesis*,
/// `docs/hw/mb86234.md`, "Special values"): an exponent field of 255 is an
/// ordinary binade, `1.m * 2^128`.
pub(crate) fn f(v: u32) -> f64 {
    if (v >> 23) & 0xff == 0xff {
        f32::from_bits(v - (1 << 23)) as f64 * 2.0
    } else {
        f32::from_bits(v) as f64
    }
}

/// `x` rounded to a float word: single precision (computing in `f64` and
/// rounding once gives IEEE single's results, so in range nothing changes),
/// using exponent 255 as an ordinary binade and saturating beyond it.
pub(crate) fn word(x: f64) -> u32 {
    let r = x as f32;
    if r.is_finite() {
        return r.to_bits();
    }
    let h = (x / 2.0) as f32;
    if h.is_finite() && (h.to_bits() >> 23) & 0xff == 0xfe {
        h.to_bits() + (1 << 23)
    } else {
        (if x.is_sign_negative() { 0x8000_0000 } else { 0 }) | 0x7fff_ffff
    }
}

/// A float register and its two views: `+1` is its exponent field
/// (bits 23-30), `+2` its mantissa field with the sign in bit 23.
fn view_base(r: u8) -> Option<(u8, u8)> {
    match r {
        0x10..=0x1e => Some((0x10 + (r - 0x10) / 3 * 3, (r - 0x10) % 3)),
        _ => None,
    }
}

#[derive(Clone)]
pub struct Cpu {
    pub program: Vec<u32>,
    /// RAM0 (`0..0x200`) and RAM1 (`0x200..0x400`).
    pub ram: Vec<u32>,
    pub regs: [u32; 64],
    /// What the ALU reads as the accumulator. It is `regs[ACC]` except
    /// after a word whose ALU operation and transfer both wrote the
    /// accumulator: then a transfer reading it sees the ALU's result and
    /// the next ALU operation the transferred value (the program's square
    /// root and command check rely on this).
    pub acc_in: u32,
    pub pc: u32,
    pub stack: Vec<u32>,
    /// Executions left of a word repeated by `3C04 n`.
    pub repeat: u32,
    repeat_armed: bool,
    pub negative: bool,
    pub zero: bool,
    /// Control registers written by `3C`/`3D` (4 bits select one): `4` is
    /// the repeat count; the others' meaning is unknown.
    pub ctrl: [u32; 16],
    /// Mode registers written by `36`: `0xA` selects the rounding of
    /// float-to-integer conversion (hypothesis: 4 truncates, 7 rounds).
    pub mode: [u32; 16],
    /// External accesses go to the shared RAM (after `63400000`) or to the
    /// function unit (after `63000000`, and at reset).
    pub shared_bus: bool,
    pub fifo_in: VecDeque<u32>,
    pub fifo_out: VecDeque<u32>,
    pub funcs: Funcs,

    // Diagnostics, not state.
    /// Instruction words of a form this model doesn't know, by word, with
    /// how often each ran (as a no-op).
    pub unknown: BTreeMap<u32, u64>,
    /// External addresses the function unit has nothing at, by address,
    /// with how often each was read (as 0) or written.
    pub unknown_ext: BTreeMap<u32, u64>,
    pub steps: u64,
}

impl Default for Cpu {
    fn default() -> Self {
        Cpu::new(Vec::new(), Vec::new())
    }
}

/// Pending register writes of one word.
struct Writes {
    regs: [(u8, u32); 3],
    n: usize,
}

impl Writes {
    fn new() -> Self {
        Writes { regs: [(0, 0); 3], n: 0 }
    }
    fn push(&mut self, r: u8, v: u32) {
        if self.n < 3 {
            self.regs[self.n] = (r, v);
            self.n += 1;
        }
    }
}

/// Where an operand of a transfer or multiplier load is.
#[derive(Clone, Copy)]
enum Space {
    /// Internal RAM (RAM0, then RAM1 from 0x200).
    Int,
    /// The external bus.
    Ext,
    /// Program memory (read only: tables).
    Prog,
}

/// What an ALU operation produced.
#[derive(Default)]
struct AluOut {
    acc: Option<u32>,
    product: Option<u32>,
}

impl Cpu {
    /// A chip with `program` loaded and the function unit's `tables` (the
    /// board's lookup ROM as 32-bit words; may be empty), at reset.
    pub fn new(program: Vec<u32>, tables: Vec<u32>) -> Self {
        Cpu {
            program,
            ram: vec![0; RAM_WORDS],
            regs: [0; 64],
            acc_in: 0,
            pc: 0,
            stack: Vec::new(),
            repeat: 0,
            repeat_armed: false,
            negative: false,
            zero: false,
            ctrl: [0; 16],
            mode: [0; 16],
            shared_bus: false,
            fifo_in: VecDeque::new(),
            fifo_out: VecDeque::new(),
            funcs: Funcs::new(tables),
            unknown: BTreeMap::new(),
            unknown_ext: BTreeMap::new(),
            steps: 0,
        }
    }

    /// Run until the program needs input it hasn't got, or for `limit`
    /// words.
    pub fn run(&mut self, bus: &mut dyn Bus, limit: u64) -> Stop {
        for _ in 0..limit {
            let w = self.program.get(self.pc as usize).copied().unwrap_or(0);
            if reads_fifo(w) && self.fifo_in.is_empty() {
                return Stop::Input;
            }
            self.step(bus, w);
        }
        Stop::Limit
    }

    fn unknown_word(&mut self, w: u32) {
        *self.unknown.entry(w).or_default() += 1;
    }

    fn step(&mut self, bus: &mut dyn Bus, w: u32) {
        self.steps += 1;
        let mut next = self.pc + 1;
        let top = w >> 24;
        match w >> 26 {
            0x00 | 0x07 => self.op(bus, w),
            _ => match top {
                0x36 => self.mode[((w >> 16) & 0xf) as usize] = w & 0xffff,
                // P = sign-extended imm (`skytargt` loads 0 or 1 into R13
                // and P on the two sides of a compare and stores whichever
                // its branch took as the index of the larger value).
                0x38 => self.write_regs(&[(reg::P, sext(w, 24))], None),
                0x39 => self.write_regs(&[(reg::R10, sext(w, 24))], None),
                // R13 = sign-extended imm (hypothesis: `vf2` loads an
                // all-ones mask into R13 this way and stores R13 next).
                0x3a => self.write_regs(&[(reg::R13, sext(w, 24))], None),
                0x3b => self.write_regs(&[(reg::ACC, sext(w, 24))], None),
                0x3c | 0x3d => {
                    let alu = ((w >> 20) & 0x1f) as u8;
                    if alu != 0 {
                        let out = self.alu(w, alu);
                        self.commit(&Writes::new(), out);
                    }
                    let c = ((w >> 16) & 0xf) as usize;
                    self.ctrl[c] = w & 0xffff;
                    if c == 4 {
                        self.repeat = w & 0xffff;
                        self.repeat_armed = true;
                    }
                }
                // Register (top bits 0-5) = imm24: AR0-AR3 (`40`-`43`), LC
                // (`4C`), the float views (`5r`), the bus register (`63`).
                0x40..=0x7f => {
                    let r = (top & 0x3f) as u8;
                    let v = match view_base(r) {
                        // A whole float register (R10, R13, R16, ACC, P):
                        // only its low 16 bits (hypothesis: `vf2` compares
                        // a counter of 0x400000 + i with `R10 = 3` after
                        // loading R10 with the counter).
                        Some((_, 0)) => (self.regs[r as usize] & 0xffff_0000) | (w & 0xffff),
                        _ => w & 0xff_ffff,
                    };
                    self.write_regs(&[(r, v)], None)
                }
                0xbe | 0xfe => {
                    let cond = (w >> 16) & 0xff;
                    let taken = match cond & !0x0c {
                        0x00 => Some(self.zero),
                        0x10 => Some(!self.negative),
                        0x20 => Some(self.negative || self.zero),
                        // The function unit's arctangent: |x| < |y|.
                        0xa0 => Some(self.funcs.atan_steep()),
                        _ => None,
                    };
                    match taken {
                        None => self.unknown_word(w),
                        Some(t) => {
                            let t = t != (top == 0xfe);
                            if cond & 0x0c == 0x04 {
                                // A conditional call.
                                if t {
                                    self.push(next);
                                    next = w & 0xffff;
                                }
                            } else if cond & 0x0c != 0 {
                                // A conditional transfer instead of a jump: a
                                // load of RAM0 into a register.
                                if t {
                                    let r = ((w >> 9) & 0x3f) as u8;
                                    let v = self.ram[(w & 0xff) as usize];
                                    self.write_regs(&[(r, v)], None);
                                }
                            } else if t {
                                next = w & 0xffff;
                            }
                        }
                    }
                }
                0xbf => match (w >> 16) & 0xff {
                    // Loop: count the loop counter (register 0x0C) down and
                    // jump while it is not zero; the flags are left alone.
                    // The course data search loads the counter with a
                    // cell's polygon count and closes its loop with this.
                    // `BF10`: the same on register 0x0D (`vf2` nests it
                    // outside an LC loop).
                    0x00 | 0x10 => {
                        let r = if w & 0x0010_0000 != 0 { reg::LC2 } else { reg::LC };
                        let mask = if r == reg::LC { 0xff } else { u32::MAX };
                        let c = &mut self.regs[r as usize];
                        *c = c.wrapping_sub(1) & mask;
                        if *c != 0 {
                            next = w & 0xffff;
                        }
                    }
                    0x60 => next = w & 0xffff,
                    0x64 => {
                        self.push(next);
                        next = w & 0xffff;
                    }
                    // Jump to the address in register r (`BF62 4019`: the
                    // dispatch through a table of jumps), or with bit 14
                    // clear in RAM0 at a direct address (`daytona`'s
                    // `BF66 0035` calls the routine whose address it
                    // stored at RAM0 0x35).
                    0x62 => next = self.jump_target(w),
                    0x66 => {
                        self.push(next);
                        next = self.jump_target(w);
                    }
                    0x6a => match self.stack.pop() {
                        Some(a) => next = a,
                        None => self.unknown_word(w),
                    },
                    // Return from interrupt (hypothesis); interrupts aren't modelled.
                    0x6e => {}
                    _ => self.unknown_word(w),
                },
                _ => self.unknown_word(w),
            },
        }
        if self.repeat_armed {
            self.repeat_armed = false;
        } else if self.repeat > 0 {
            self.repeat -= 1;
            if self.repeat > 0 {
                return;
            }
        }
        self.pc = next & 0xffff;
    }

    /// The target of an indirect jump or call: register `w & 0x3F` with
    /// bit 14 set, else RAM0's direct address `w & 0x7F`.
    fn jump_target(&mut self, w: u32) -> u32 {
        let v = if w & 0x4000 != 0 { self.read_reg((w & 0x3f) as u8) } else { self.ram_read(w & 0x7f) };
        v & 0xffff
    }

    fn push(&mut self, a: u32) {
        if self.stack.len() == STACK_DEPTH {
            self.stack.remove(0);
        }
        self.stack.push(a);
    }

    // ---- registers

    fn read_reg(&mut self, r: u8) -> u32 {
        let r = r & 0x3f;
        if r == reg::FIFO_IN {
            return self.fifo_in.pop_front().unwrap_or(0);
        }
        match view_base(r) {
            Some((b, 1)) => (self.regs[b as usize] >> 23) & 0xff,
            Some((b, 2)) => {
                let v = self.regs[b as usize];
                (v & 0x7f_ffff) | ((v >> 31) << 23)
            }
            _ => self.regs[r as usize],
        }
    }

    /// Apply a word's register writes, `alu` being what its ALU operation
    /// wrote (the accumulator rule is on [`Cpu::acc_in`]).
    fn write_regs(&mut self, writes: &[(u8, u32)], alu_acc: Option<u32>) {
        let mut moved_acc = false;
        for &(r, v) in writes {
            let r = r & 0x3f;
            if r == reg::FIFO_OUT {
                self.fifo_out.push_back(v);
                continue;
            }
            let (base, part) = view_base(r).unwrap_or((r, 0));
            let old = if base == reg::ACC && alu_acc.is_some() { self.acc_in } else { self.regs[base as usize] };
            let new = match part {
                1 => (old & 0x807f_ffff) | ((v & 0xff) << 23),
                2 => (old & 0x7f80_0000) | (v & 0x7f_ffff) | (((v >> 23) & 1) << 31),
                _ => v,
            };
            let new = match base {
                // 8 bits: `vf2`'s dispatch moves a command word
                // (`F << 23 | F << 8 | F`) here and reads back F.
                reg::LC => new & 0xff,
                reg::AR0..=reg::AR3 => new & 0xff_ffff,
                reg::BUS => {
                    self.shared_bus = new & 0x0040_0000 != 0;
                    new
                }
                _ => new,
            };
            if base == reg::ACC {
                moved_acc = true;
                self.acc_in = new;
                if alu_acc.is_some() {
                    continue;
                }
            }
            self.regs[base as usize] = new;
        }
        if let Some(a) = alu_acc {
            self.regs[reg::ACC as usize] = a;
            if !moved_acc {
                self.acc_in = a;
            }
        }
    }

    fn commit(&mut self, w: &Writes, out: AluOut) {
        self.write_regs(&w.regs[..w.n], out.acc);
        if let Some(p) = out.product {
            self.regs[reg::P as usize] = p;
        }
    }

    // ---- memory

    fn ram_read(&self, a: u32) -> u32 {
        self.ram[(a as usize) & (RAM_WORDS - 1)]
    }

    /// The 24-bit external address of an access through `a`: the bus
    /// register's bits 16-21 and 23 complete it (bit 22 is the mode).
    /// `daytona`'s program puts a data ROM address in the bus register and
    /// its low 16 bits in an address register (docs/hw/mb86234.md); the
    /// 2A programs set only bit 22 there and carry the ROM's address in
    /// the address register itself.
    fn ext_addr(&self, a: u32) -> u32 {
        (a & 0xff_ffff) | (self.regs[reg::BUS as usize] & 0xbf_0000)
    }

    fn ext_read(&mut self, bus: &mut dyn Bus, a: u32) -> u32 {
        let a = self.ext_addr(a);
        // The data ROM (from 0x800000) is there in both bus modes.
        if self.shared_bus || a >= ROM_BASE {
            bus.read(a)
        } else {
            match self.funcs.read(a & 0xffff) {
                Some(v) => v,
                None => {
                    *self.unknown_ext.entry(a & 0xffff).or_default() += 1;
                    0
                }
            }
        }
    }

    fn ext_write(&mut self, bus: &mut dyn Bus, a: u32, v: u32) {
        let a = self.ext_addr(a);
        if self.shared_bus || a >= ROM_BASE {
            bus.write(a, v);
        } else if !self.funcs.write(a & 0xffff, v) {
            *self.unknown_ext.entry(a & 0xffff).or_default() += 1;
        }
    }

    /// An indirect operand through `ar` (AR2 or AR3), whose field `m`
    /// holds the signed 5-bit step in bits 0-4 and the mode in bits 5-6;
    /// the register is stepped after. Bit 5 set: the register's value;
    /// clear (`docs/hw/mb86234.md`): indexed, the register plus its base
    /// (AR0 for AR2, AR1 for AR3), and with bit 6 also set, **circular**:
    /// the step wraps within [`RING`] words, so the index counts round a
    /// ring (`vf2` reads a 64-word ring the i960 fills in shared RAM this
    /// way).
    fn post(&mut self, ar: u8, m: u32) -> Ea {
        let a = self.regs[ar as usize];
        let next = a.wrapping_add(sext(m & 0x1f, 5));
        self.regs[ar as usize] =
            if m & 0x60 == 0x40 { (a & !(RING - 1)) | (next & (RING - 1)) } else { next } & 0xff_ffff;
        let base = if m & 0x20 == 0 { self.base(ar) } else { 0 };
        Ea { index: a, base }
    }

    /// The base register of `ar`: AR0 for AR2, AR1 for AR3.
    fn base(&self, ar: u8) -> u32 {
        self.regs[(ar & 1) as usize]
    }

    /// A 9-bit operand field: direct (`0x00-0x7F`), relative to `ar` and
    /// its base (`0x80-0xFF`) or indirect (`0x100` set) through `ar`
    /// ([`Cpu::post`]).
    fn c9(&mut self, c9: u32, ar: u8) -> Ea {
        if c9 & 0x100 != 0 {
            self.post(ar, c9)
        } else if c9 & 0x80 != 0 {
            self.relative(ar, c9 & 0x7f)
        } else {
            Ea { index: c9, base: 0 }
        }
    }

    /// `ar` and its base, plus `n`.
    fn relative(&self, ar: u8, n: u32) -> Ea {
        Ea { index: self.regs[ar as usize].wrapping_add(n), base: self.base(ar) }
    }

    // ---- the ALU + transfer words

    fn op(&mut self, bus: &mut dyn Bus, w: u32) {
        let alu = ((w >> 21) & 0x1f) as u8;
        let kind = (w >> 16) & 0x1f;
        let op1 = (w >> 9) & 0x7f;
        let c9 = w & 0x1ff;
        // The ALU reads its operands before the transfer changes them.
        let out = self.alu(w, alu);
        let mut wr = Writes::new();
        if w >> 26 == 0x07 && kind >= 0x1c {
            // The single-address transfers: a register and an operand
            // through AR3 (`1E` with op1 bit 6: through AR2, from the
            // program).
            let mut store: Option<(Space, u32, u32)> = None;
            match kind {
                // Register to RAM (or the external bus when op1 bit 6 is set).
                0x1c => {
                    let v = self.read_reg(op1 as u8);
                    let s = if op1 & 0x40 != 0 { Space::Ext } else { Space::Int };
                    let a = self.c9(c9, reg::AR3).at(s);
                    store = Some((s, a, v));
                }
                // Internal RAM to register.
                0x1d => {
                    let a = self.c9(c9, reg::AR3).at(Space::Int);
                    wr.push(op1 as u8, self.ram_read(a));
                }
                // External bus (op1 bit 6: program memory) to register.
                0x1e => {
                    let v = if op1 & 0x40 != 0 {
                        let a = self.c9(c9, reg::AR2).at(Space::Prog);
                        self.prog_read(a)
                    } else {
                        let a = self.c9(c9, reg::AR3).at(Space::Ext);
                        self.ext_read(bus, a)
                    };
                    wr.push(op1 as u8, v);
                }
                // Register to register.
                _ => {
                    let v = self.read_reg(c9 as u8);
                    wr.push(op1 as u8, v);
                }
            }
            self.commit(&wr, out);
            if let Some((s, a, v)) = store {
                self.store(bus, s, a, v);
            }
            return;
        }
        // The two-operand forms: kind = group (bits 2-4) and op1's mode
        // (bits 0-1). The group gives the spaces: op1's (the destination of
        // a transfer, R13 of a multiplier load) and c9's (the source, R10).
        let group = kind >> 2;
        let (s1, s9) = match group {
            0 | 1 => (Space::Ext, Space::Int),
            2 => (Space::Int, Space::Ext),
            3 | 4 => (Space::Int, Space::Int),
            5 => (Space::Int, Space::Prog),
            _ => {
                self.unknown_word(w);
                self.commit(&wr, out);
                return;
            }
        };
        let is_mult = w >> 26 == 0;
        // op1's address: direct, relative to AR3 and AR1, or through AR3.
        let a1 = match kind & 3 {
            0 => op1,
            1 => self.relative(reg::AR3, op1).at(s1),
            3 => self.post(reg::AR3, op1).at(s1),
            _ => {
                self.unknown_word(w);
                self.commit(&wr, out);
                return;
            }
        };
        // c9's: direct, relative to AR2 and AR0, or through AR2.
        let mut a9 = self.c9(c9, reg::AR2).at(s9);
        // Groups 3 and 4 fix the RAM of each operand, direct or not:
        // op1's is RAM1 and c9's RAM0 in group 3, the other way round in
        // group 4. A multiplier load reads both in one word, one from each
        // RAM; both games keep their matrix stack in RAM1 and copy it
        // through RAM0 with these (`docs/hw/mb86234.md`).
        let mut a1 = a1;
        match group {
            3 => {
                a1 = 0x200 | (a1 & 0x1ff);
                a9 &= 0x1ff;
            }
            4 => {
                a1 &= 0x1ff;
                a9 = 0x200 | (a9 & 0x1ff);
            }
            _ => {}
        }
        let v9 = self.load(bus, s9, a9);
        if is_mult {
            let v1 = self.load(bus, s1, a1);
            wr.push(reg::R13, v1);
            wr.push(reg::R10, v9);
            self.commit(&wr, out);
            return;
        }
        self.commit(&wr, out);
        self.store(bus, s1, a1, v9);
    }

    fn prog_read(&self, a: u32) -> u32 {
        self.program.get((a & 0xffff) as usize).copied().unwrap_or(0)
    }

    fn load(&mut self, bus: &mut dyn Bus, s: Space, a: u32) -> u32 {
        match s {
            Space::Int => self.ram_read(a),
            Space::Ext => self.ext_read(bus, a),
            Space::Prog => self.prog_read(a),
        }
    }

    fn store(&mut self, bus: &mut dyn Bus, s: Space, a: u32, v: u32) {
        match s {
            Space::Int => self.ram[(a as usize) & (RAM_WORDS - 1)] = v,
            Space::Ext => self.ext_write(bus, a, v),
            Space::Prog => {}
        }
    }

    fn set_flags_f(&mut self, v: u32) {
        self.zero = v & 0x7fff_ffff == 0;
        self.negative = v >> 31 != 0 && !self.zero;
    }

    fn set_flags_i(&mut self, v: u32) {
        self.zero = v == 0;
        self.negative = v >> 31 != 0;
    }

    /// The ALU operation `code` of word `w` (`docs/hw/mb86234.md` has the
    /// table and how each code was pinned).
    fn alu(&mut self, w: u32, code: u8) -> AluOut {
        let a = self.acc_in;
        let r10 = self.regs[reg::R10 as usize];
        let r13 = self.regs[reg::R13 as usize];
        let p = self.regs[reg::P as usize];
        let product = || word(f(r10) * f(r13));
        let fl = |s: &mut Self, v: f64| {
            let b = word(v);
            s.set_flags_f(b);
            Some(b)
        };
        let int = |s: &mut Self, v: u32| {
            s.set_flags_i(v);
            Some(v)
        };
        let sh = self.regs[reg::SH as usize] & 31;
        match code {
            0 => AluOut::default(),
            1 => AluOut { acc: int(self, a & r10), product: None },
            2 => AluOut { acc: int(self, a | r10), product: None },
            // XOR (hypothesis: `skytargt` keeps a product only when its
            // exponent's bits 4-6 equal the mask 0x70, else flushes it).
            3 => AluOut { acc: int(self, a ^ r10), product: None },
            // NOT (hypothesis: `vf2` clears a mask's bit with it and 1).
            4 => AluOut { acc: int(self, !a), product: None },
            5 => {
                self.set_flags_f(word(f(a) - f(r10)));
                AluOut::default()
            }
            6 => AluOut { acc: fl(self, f(a) + f(r10)), product: None },
            7 => AluOut { acc: fl(self, f(a) - f(r10)), product: None },
            8 => AluOut { acc: None, product: Some(product()) },
            9 => AluOut { acc: fl(self, f(a) + f(p)), product: Some(product()) },
            10 => AluOut { acc: fl(self, f(a) - f(p)), product: Some(product()) },
            11 => AluOut { acc: fl(self, f(a).abs()), product: None },
            12 => AluOut { acc: fl(self, f(a) + f(p)), product: None },
            13 => {
                self.set_flags_f(p);
                AluOut { acc: Some(p), product: Some(product()) }
            }
            14 => AluOut { acc: fl(self, a as i32 as f32 as f64), product: None },
            15 => {
                let x = f(a);
                let v = if self.mode[0xa] & 3 == 0 { x.trunc() } else { x.round_ties_even() };
                AluOut { acc: int(self, v as i32 as u32), product: None }
            }
            16 => AluOut { acc: fl(self, f(a) / f(r10)), product: None },
            17 => AluOut { acc: fl(self, -f(a)), product: None },
            // R13 + R10 (hypothesis: `vf2` pairs it with 20 to widen a
            // range, centre - r and centre + r).
            19 => AluOut { acc: fl(self, f(r13) + f(r10)), product: None },
            20 => AluOut { acc: fl(self, f(r13) - f(r10)), product: None },
            22 => AluOut { acc: int(self, a >> sh), product: None },
            23 => AluOut { acc: int(self, a << sh), product: None },
            24 => AluOut { acc: int(self, ((a as i32) >> sh) as u32), product: None },
            26 => AluOut { acc: int(self, a.wrapping_add(r10)), product: None },
            27 => AluOut { acc: int(self, a.wrapping_sub(r10)), product: None },
            _ => {
                self.unknown_word(w);
                AluOut::default()
            }
        }
    }
}

/// Does word `w` read the input FIFO (so it must wait for a word)?
pub fn reads_fifo(w: u32) -> bool {
    if w >> 26 != 0x07 {
        return false;
    }
    let kind = (w >> 16) & 0x1f;
    (kind == 0x1c && (w >> 9) & 0x3f == reg::FIFO_IN as u32) || (kind == 0x1f && w & 0x1ff == reg::FIFO_IN as u32)
}

state::fields!(Cpu {
    ram,
    regs,
    acc_in,
    pc,
    stack,
    repeat,
    repeat_armed,
    negative,
    zero,
    ctrl,
    mode,
    shared_bus,
    fifo_in,
    fifo_out,
    funcs
});

#[cfg(test)]
mod tests;
