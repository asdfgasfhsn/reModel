//! Fujitsu MB86234, the geometry processor ("TGP") of Sega's Model 2 and
//! 2A boards: an interpreter for its 32-bit instruction words and a
//! disassembler.
//!
//! No manual for the chip is public and nothing here comes from an
//! emulator: the encodings were read from the one program reModel has
//! (`srallyc`'s, uploaded by the game) and pinned by running its routines
//! and checking the mathematics they compute (`docs/hw/mb86234.md` lists
//! every encoding with how sure it is). Where an encoding is a hypothesis
//! the code says so; words of a form never seen are counted in
//! [`Cpu::unknown`] and executed as no-ops.
//!
//! The model: one word a step (no delay slots, no cycle timing); a word's
//! ALU operation and its transfer read their operands at the start of the
//! word and write at the end. Two RAMs of 512 words (RAM0, RAM1), one
//! address space for indirect accesses (RAM0 at 0, RAM1 at 0x200), an
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
/// Return addresses kept (calls nest at most three deep in `srallyc`).
pub const STACK_DEPTH: usize = 16;

/// Register numbers of the transfer fields (`docs/hw/mb86234.md`).
pub mod reg {
    pub const AR0: u8 = 0x00;
    pub const AR2: u8 = 0x02;
    pub const AR3: u8 = 0x03;
    /// The loop counter `BF00` counts down.
    pub const LC: u8 = 0x0c;
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

fn f(v: u32) -> f32 {
    f32::from_bits(v)
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
                0x39 => self.write_regs(&[(reg::R10, sext(w, 24))], None),
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
                0x40..=0x43 => self.regs[(top & 3) as usize] = w & 0xffff,
                0x50..=0x5f => self.write_regs(&[((0x10 + (top & 0xf)) as u8, w & 0xff_ffff)], None),
                0x63 => self.shared_bus = w & 0x0040_0000 != 0,
                0x7c => {}
                0xbe | 0xfe => {
                    let cond = (w >> 16) & 0xff;
                    let taken = match cond & !0x0c {
                        0x00 => Some(self.zero),
                        0x10 => Some(!self.negative),
                        0x20 => Some(self.negative || self.zero),
                        _ => None,
                    };
                    match taken {
                        None => self.unknown_word(w),
                        Some(t) => {
                            let t = t != (top == 0xfe);
                            if cond & 0x0c != 0 {
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
                    0x00 => {
                        let lc = &mut self.regs[reg::LC as usize];
                        *lc = lc.wrapping_sub(1);
                        if *lc != 0 {
                            next = w & 0xffff;
                        }
                    }
                    0x60 => next = w & 0xffff,
                    0x64 => {
                        self.push(next);
                        next = w & 0xffff;
                    }
                    0x66 => {
                        self.push(next);
                        next = self.read_reg((w & 0x3f) as u8) & 0xffff;
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

    fn ext_read(&mut self, bus: &mut dyn Bus, a: u32) -> u32 {
        // The data ROM (from 0x800000) is there in both bus modes.
        if self.shared_bus || a & 0xff_ffff >= ROM_BASE {
            bus.read(a & 0xff_ffff)
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
        if self.shared_bus || a & 0xff_ffff >= ROM_BASE {
            bus.write(a & 0xff_ffff, v);
        } else if !self.funcs.write(a & 0xffff, v) {
            *self.unknown_ext.entry(a & 0xffff).or_default() += 1;
        }
    }

    /// An address register's value, then the signed 5-bit step added to it.
    fn post(&mut self, ar: u8, modifier: u32) -> u32 {
        let a = self.regs[ar as usize];
        self.regs[ar as usize] = a.wrapping_add(sext(modifier & 0x1f, 5)) & 0xff_ffff;
        a
    }

    /// The internal address of a 9-bit operand field: direct (`0x00-0x7F`),
    /// relative to AR2 (`0x80-0xFF`, hypothesis outside type 0x10) or
    /// indirect (`0x100` set) through `ar` with a post-step.
    fn c9(&mut self, c9: u32, ar: u8) -> u32 {
        if c9 & 0x100 != 0 {
            self.post(ar, c9)
        } else if c9 & 0x80 != 0 {
            self.regs[reg::AR2 as usize].wrapping_add(c9 & 0x7f)
        } else {
            c9
        }
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
        if w >> 26 == 0 {
            // Multiplier loads: R13 from the first operand, R10 from the second.
            let (a, b) = match kind {
                0x0c => {
                    let b = self.c9(c9, reg::AR3);
                    (self.ram_read(0x200 + op1), self.ram_read(b))
                }
                0x0f => {
                    let a = self.post(reg::AR3, op1);
                    let b = self.c9(c9, reg::AR2);
                    (self.ram_read(a), self.ram_read(b))
                }
                0x10 => {
                    let a = if op1 & 0x40 != 0 { op1 } else { 0x200 + op1 };
                    let b = self.c9(c9, reg::AR3);
                    (self.ram_read(a), self.ram_read(b))
                }
                _ => {
                    self.unknown_word(w);
                    self.commit(&wr, out);
                    return;
                }
            };
            wr.push(reg::R13, a);
            wr.push(reg::R10, b);
            self.commit(&wr, out);
            return;
        }
        let mut store: Option<(bool, u32, u32)> = None; // (external, address, value)
        match kind {
            // Register to RAM0/RAM (or the external bus when op1 bit 6 is set).
            0x1c => {
                let v = self.read_reg(op1 as u8);
                let a = self.c9(c9, reg::AR3);
                store = Some((op1 & 0x40 != 0, a, v));
            }
            // Internal RAM to register.
            0x1d => {
                let a = self.c9(c9, reg::AR3);
                wr.push(op1 as u8, self.ram_read(a));
            }
            // External bus to register.
            0x1e => {
                let ar = if op1 & 0x40 != 0 { reg::AR2 } else { reg::AR3 };
                let a = self.c9(c9, ar);
                let v = self.ext_read(bus, a);
                wr.push(op1 as u8, v);
            }
            // Register to register.
            0x1f => {
                let v = self.read_reg(c9 as u8);
                wr.push(op1 as u8, v);
            }
            // RAM0 to RAM1.
            0x0c => {
                let a = self.c9(c9, reg::AR3);
                store = Some((false, 0x200 + op1, self.ram_read(a)));
            }
            // RAM0 to RAM0.
            0x10 => {
                let a = self.c9(c9, reg::AR3);
                store = Some((false, op1, self.ram_read(a)));
            }
            // External to RAM0.
            0x08 => {
                let a = self.c9(c9, reg::AR3);
                store = Some((false, op1, self.ext_read(bus, a)));
            }
            // Two-address transfers: to [AR3] (op1's step), from the second
            // operand ([AR2] when indirect).
            0x03 | 0x07 | 0x0b | 0x0f | 0x13 => {
                let s = self.c9(c9, reg::AR2);
                let v = match kind {
                    // The source of 0x07 sees RAM0 at 0x200 (hypothesis: the
                    // other bus's view; the program adds 0x200 to a RAM0
                    // address for it).
                    0x07 => self.ram_read(s ^ 0x200),
                    0x0b => self.ext_read(bus, s),
                    _ => self.ram_read(s),
                };
                let d = self.post(reg::AR3, op1);
                store = Some((matches!(kind, 0x03 | 0x07), d, v));
            }
            _ => self.unknown_word(w),
        }
        self.commit(&wr, out);
        if let Some((ext, a, v)) = store {
            if ext {
                self.ext_write(bus, a, v);
            } else {
                self.ram[(a as usize) & (RAM_WORDS - 1)] = v;
            }
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
        let product = || (f(r10) * f(r13)).to_bits();
        let fl = |s: &mut Self, v: f32| {
            let b = v.to_bits();
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
            5 => {
                self.set_flags_f((f(a) - f(r10)).to_bits());
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
            14 => AluOut { acc: fl(self, a as i32 as f32), product: None },
            15 => {
                let x = f(a);
                let v = if self.mode[0xa] & 3 == 0 { x.trunc() } else { x.round_ties_even() };
                AluOut { acc: int(self, v as i32 as u32), product: None }
            }
            16 => AluOut { acc: fl(self, f(a) / f(r10)), product: None },
            17 => AluOut { acc: fl(self, -f(a)), product: None },
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
