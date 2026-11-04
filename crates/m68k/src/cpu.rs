//! 68000 interpreter.
//!
//! Instruction-at-a-time: no prefetch queue is modelled, so code that
//! modifies the next one or two words after the current instruction sees the
//! new bytes immediately (the real chip would already have fetched them).
//!
//! Cycle counts are approximate: four per bus word, plus the internal time
//! from the manual's tables for the common cases. Not implemented: trace
//! mode (the T bit is stored but never raises an exception) and bus errors
//! (the `Bus` trait has no way to signal one).

use crate::{ea_ok, Sz, ALL, ALTERABLE, CONTROL, CTRL_ALT, DATA, DATA_ALT, IMM, MEMORY, MEM_ALT, POSTINC, PREDEC};

/// Memory and devices as seen by the CPU: a 24-bit, big-endian address
/// space. Long accesses are made as two word accesses, high word first, and
/// word accesses are always to even addresses.
pub trait Bus {
    fn read8(&mut self, addr: u32) -> u8;
    fn read16(&mut self, addr: u32) -> u16;
    fn write8(&mut self, addr: u32, v: u8);
    fn write16(&mut self, addr: u32, v: u16);
    /// Interrupt acknowledge for `level`: a vector number, or `None` to
    /// autovector (vector 24 + level).
    fn int_ack(&mut self, _level: u8) -> Option<u8> {
        None
    }
    /// The RESET instruction pulsed the reset line to external devices.
    fn reset_devices(&mut self) {}
}

const ADDR_MASK: u32 = 0x00ff_ffff;
const SR_MASK: u16 = 0xa71f;

pub const VEC_ADDRESS_ERROR: u8 = 3;
pub const VEC_ILLEGAL: u8 = 4;
pub const VEC_ZERO_DIVIDE: u8 = 5;
pub const VEC_CHK: u8 = 6;
pub const VEC_TRAPV: u8 = 7;
pub const VEC_PRIVILEGE: u8 = 8;
pub const VEC_LINE_A: u8 = 10;
pub const VEC_LINE_F: u8 = 11;
/// Autovectors are 25-31 for levels 1-7; 24 is the spurious interrupt.
pub const VEC_AUTOVECTOR: u8 = 24;
pub const VEC_TRAP: u8 = 32;

/// Why an instruction stopped early.
#[derive(Clone, Copy, Debug)]
enum Fault {
    /// Word or long access to an odd address.
    Address { addr: u32, write: bool, prog: bool },
    /// A group 1/2 exception through this vector.
    Vector(u8),
}

type R<T> = Result<T, Fault>;

const ILLEGAL: Fault = Fault::Vector(VEC_ILLEGAL);
const PRIVILEGE: Fault = Fault::Vector(VEC_PRIVILEGE);

/// A resolved effective address.
#[derive(Clone, Copy, Debug)]
enum Ea {
    D(usize),
    A(usize),
    M(u32),
    I(u32),
}

/// 68000 register state.
#[derive(Clone, Debug)]
pub struct Cpu {
    pub d: [u32; 8],
    /// A0-A7; `a[7]` is the stack pointer for the current mode.
    pub a: [u32; 8],
    /// The other mode's stack pointer (USP in supervisor mode, else SSP).
    other_sp: u32,
    /// Address of the next instruction word to fetch.
    pub pc: u32,
    t: bool,
    s: bool,
    mask: u8,
    x: bool,
    n: bool,
    z: bool,
    v: bool,
    c: bool,
    stopped: bool,
    halted: bool,
    irq: u8,
    nmi: bool,
    cyc: u32,
    /// Address and opcode of the instruction being executed.
    ppc: u32,
    ir: u16,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    pub fn new() -> Self {
        Cpu {
            d: [0; 8],
            a: [0; 8],
            other_sp: 0,
            pc: 0,
            t: false,
            s: true,
            mask: 7,
            x: false,
            n: false,
            z: false,
            v: false,
            c: false,
            stopped: false,
            halted: false,
            irq: 0,
            nmi: false,
            cyc: 0,
            ppc: 0,
            ir: 0,
        }
    }

    /// Reset exception: supervisor mode, interrupts masked, SSP from address
    /// 0 and PC from address 4.
    pub fn reset<B: Bus>(&mut self, b: &mut B) {
        self.set_sr(0x2700);
        self.stopped = false;
        self.halted = false;
        self.nmi = false;
        let sp = (b.read16(0) as u32) << 16 | b.read16(2) as u32;
        let pc = (b.read16(4) as u32) << 16 | b.read16(6) as u32;
        self.a[7] = sp;
        self.pc = pc;
        self.halted = pc & 1 != 0;
    }

    pub fn sr(&self) -> u16 {
        (self.t as u16) << 15
            | (self.s as u16) << 13
            | (self.mask as u16) << 8
            | (self.x as u16) << 4
            | (self.n as u16) << 3
            | (self.z as u16) << 2
            | (self.v as u16) << 1
            | self.c as u16
    }

    /// Write SR; changing the S bit swaps A7 with the other stack pointer.
    pub fn set_sr(&mut self, v: u16) {
        let v = v & SR_MASK;
        let s = v & 0x2000 != 0;
        if s != self.s {
            std::mem::swap(&mut self.a[7], &mut self.other_sp);
            self.s = s;
        }
        self.t = v & 0x8000 != 0;
        self.mask = (v >> 8) as u8 & 7;
        self.set_ccr(v as u8);
    }

    fn set_ccr(&mut self, v: u8) {
        self.x = v & 0x10 != 0;
        self.n = v & 8 != 0;
        self.z = v & 4 != 0;
        self.v = v & 2 != 0;
        self.c = v & 1 != 0;
    }

    pub fn usp(&self) -> u32 {
        if self.s {
            self.other_sp
        } else {
            self.a[7]
        }
    }

    pub fn ssp(&self) -> u32 {
        if self.s {
            self.a[7]
        } else {
            self.other_sp
        }
    }

    pub fn set_usp(&mut self, v: u32) {
        if self.s {
            self.other_sp = v
        } else {
            self.a[7] = v
        }
    }

    pub fn set_ssp(&mut self, v: u32) {
        if self.s {
            self.a[7] = v
        } else {
            self.other_sp = v
        }
    }

    /// True while waiting in STOP for an interrupt.
    pub fn stopped(&self) -> bool {
        self.stopped
    }

    /// True after a double fault (address error while processing one).
    /// Only reset leaves this state.
    pub fn halted(&self) -> bool {
        self.halted
    }

    /// Drive the interrupt priority inputs (0 = none, 7 = non-maskable).
    /// Levels 1-6 are level-sensitive; level 7 triggers on its rising edge.
    pub fn set_irq(&mut self, level: u8) {
        let level = level & 7;
        if level == 7 && self.irq != 7 {
            self.nmi = true;
        }
        self.irq = level;
    }

    /// Execute one instruction (or take one interrupt); returns the cycles
    /// used. While stopped or halted this just burns four cycles.
    pub fn step<B: Bus>(&mut self, b: &mut B) -> u32 {
        self.cyc = 0;
        if self.halted {
            return 4;
        }
        let level = if self.nmi {
            7
        } else if self.irq > self.mask && self.irq < 7 {
            self.irq
        } else {
            0
        };
        if level != 0 {
            self.nmi = false;
            self.stopped = false;
            if let Err(f) = self.interrupt(b, level) {
                self.fault(b, f);
            }
            return self.cyc;
        }
        if self.stopped {
            return 4;
        }
        self.ppc = self.pc;
        let op = self.fetch16(b);
        self.ir = op;
        if let Err(f) = self.exec(b, op) {
            self.fault(b, f);
        }
        self.cyc
    }

    // ---- bus access ----

    fn fetch16<B: Bus>(&mut self, b: &mut B) -> u16 {
        let v = b.read16(self.pc & ADDR_MASK);
        self.pc = self.pc.wrapping_add(2);
        self.cyc += 4;
        v
    }

    fn fetch32<B: Bus>(&mut self, b: &mut B) -> u32 {
        let hi = self.fetch16(b) as u32;
        hi << 16 | self.fetch16(b) as u32
    }

    fn imm<B: Bus>(&mut self, b: &mut B, sz: Sz) -> u32 {
        match sz {
            Sz::B => self.fetch16(b) as u32 & 0xff,
            Sz::W => self.fetch16(b) as u32,
            Sz::L => self.fetch32(b),
        }
    }

    fn read<B: Bus>(&mut self, b: &mut B, addr: u32, sz: Sz) -> R<u32> {
        if sz != Sz::B && addr & 1 != 0 {
            return Err(Fault::Address { addr, write: false, prog: false });
        }
        Ok(match sz {
            Sz::B => {
                self.cyc += 4;
                b.read8(addr & ADDR_MASK) as u32
            }
            Sz::W => {
                self.cyc += 4;
                b.read16(addr & ADDR_MASK) as u32
            }
            Sz::L => {
                self.cyc += 8;
                let hi = b.read16(addr & ADDR_MASK) as u32;
                hi << 16 | b.read16(addr.wrapping_add(2) & ADDR_MASK) as u32
            }
        })
    }

    fn write<B: Bus>(&mut self, b: &mut B, addr: u32, sz: Sz, v: u32) -> R<()> {
        if sz != Sz::B && addr & 1 != 0 {
            return Err(Fault::Address { addr, write: true, prog: false });
        }
        match sz {
            Sz::B => {
                self.cyc += 4;
                b.write8(addr & ADDR_MASK, v as u8)
            }
            Sz::W => {
                self.cyc += 4;
                b.write16(addr & ADDR_MASK, v as u16)
            }
            Sz::L => {
                self.cyc += 8;
                b.write16(addr & ADDR_MASK, (v >> 16) as u16);
                b.write16(addr.wrapping_add(2) & ADDR_MASK, v as u16);
            }
        }
        Ok(())
    }

    fn push<B: Bus>(&mut self, b: &mut B, sz: Sz, v: u32) -> R<()> {
        let sp = self.a[7].wrapping_sub(sz.bytes());
        self.a[7] = sp;
        self.write(b, sp, sz, v)
    }

    fn pop<B: Bus>(&mut self, b: &mut B, sz: Sz) -> R<u32> {
        let sp = self.a[7];
        let v = self.read(b, sp, sz)?;
        self.a[7] = sp.wrapping_add(sz.bytes());
        Ok(v)
    }

    /// Continue execution at `target`; an odd target faults on the fetch.
    fn jump(&mut self, target: u32) -> R<()> {
        if target & 1 != 0 {
            return Err(Fault::Address { addr: target, write: false, prog: true });
        }
        self.pc = target;
        Ok(())
    }

    // ---- effective addresses ----

    fn need(mode: u16, reg: u16, class: u16) -> R<()> {
        if ea_ok(mode, reg, class) {
            Ok(())
        } else {
            Err(ILLEGAL)
        }
    }

    /// Brief extension word: base + Xn.w/.l + d8.
    fn indexed<B: Bus>(&mut self, b: &mut B, base: u32) -> u32 {
        let ext = self.fetch16(b);
        let r = (ext >> 12) as usize & 7;
        let xn = if ext & 0x8000 != 0 { self.a[r] } else { self.d[r] };
        let xn = if ext & 0x800 != 0 { xn } else { xn as i16 as u32 };
        self.cyc += 2;
        base.wrapping_add(xn).wrapping_add(ext as i8 as u32)
    }

    /// Resolve an effective address, applying (An)+/-(An) side effects and
    /// fetching extension words. The caller has checked the mode is valid.
    fn ea<B: Bus>(&mut self, b: &mut B, mode: u16, reg: u16, sz: Sz) -> R<Ea> {
        let r = reg as usize & 7;
        // Byte pushes and pops through A7 keep the stack word-aligned.
        let step = if sz == Sz::B && r == 7 { 2 } else { sz.bytes() };
        Ok(match mode & 7 {
            0 => Ea::D(r),
            1 => Ea::A(r),
            2 => Ea::M(self.a[r]),
            3 => {
                let a = self.a[r];
                self.a[r] = a.wrapping_add(step);
                Ea::M(a)
            }
            4 => {
                self.cyc += 2;
                let a = self.a[r].wrapping_sub(step);
                self.a[r] = a;
                Ea::M(a)
            }
            5 => {
                let d = self.fetch16(b) as i16 as u32;
                Ea::M(self.a[r].wrapping_add(d))
            }
            6 => {
                let base = self.a[r];
                Ea::M(self.indexed(b, base))
            }
            _ => match r {
                0 => Ea::M(self.fetch16(b) as i16 as u32),
                1 => Ea::M(self.fetch32(b)),
                2 => {
                    let base = self.pc;
                    Ea::M(base.wrapping_add(self.fetch16(b) as i16 as u32))
                }
                3 => {
                    let base = self.pc;
                    Ea::M(self.indexed(b, base))
                }
                4 => Ea::I(self.imm(b, sz)),
                _ => return Err(ILLEGAL),
            },
        })
    }

    /// Address of a control-mode effective address.
    fn ea_addr<B: Bus>(&mut self, b: &mut B, mode: u16, reg: u16) -> R<u32> {
        match self.ea(b, mode, reg, Sz::L)? {
            Ea::M(a) => Ok(a),
            _ => Err(ILLEGAL),
        }
    }

    fn rd<B: Bus>(&mut self, b: &mut B, ea: Ea, sz: Sz) -> R<u32> {
        match ea {
            Ea::D(r) => Ok(self.d[r] & sz.mask()),
            Ea::A(r) => Ok(self.a[r] & sz.mask()),
            Ea::M(a) => self.read(b, a, sz),
            Ea::I(v) => Ok(v),
        }
    }

    fn wr<B: Bus>(&mut self, b: &mut B, ea: Ea, sz: Sz, v: u32) -> R<()> {
        match ea {
            Ea::D(r) => {
                let m = sz.mask();
                self.d[r] = (self.d[r] & !m) | (v & m);
                Ok(())
            }
            Ea::A(r) => {
                self.a[r] = v;
                Ok(())
            }
            Ea::M(a) => self.write(b, a, sz, v),
            Ea::I(_) => Err(ILLEGAL),
        }
    }

    /// Resolve and read a source operand from the low six opcode bits.
    fn src<B: Bus>(&mut self, b: &mut B, op: u16, sz: Sz, class: u16) -> R<u32> {
        let (mode, reg) = ((op >> 3) & 7, op & 7);
        Self::need(mode, reg, class)?;
        let ea = self.ea(b, mode, reg, sz)?;
        self.rd(b, ea, sz)
    }

    // ---- flags and arithmetic ----

    fn set_nz(&mut self, v: u32, sz: Sz) {
        self.n = v & sz.msb() != 0;
        self.z = v & sz.mask() == 0;
    }

    fn logic(&mut self, v: u32, sz: Sz) -> u32 {
        self.set_nz(v, sz);
        self.v = false;
        self.c = false;
        v & sz.mask()
    }

    /// `d + s + x` setting all of XNZVC; with `sticky` Z is only cleared.
    fn add(&mut self, s: u32, d: u32, x: bool, sz: Sz, sticky: bool) -> u32 {
        let (s, d) = (s & sz.mask(), d & sz.mask());
        let r = d.wrapping_add(s).wrapping_add(x as u32) & sz.mask();
        let msb = sz.msb();
        self.c = ((s & d) | (!r & (s | d))) & msb != 0;
        self.v = ((s ^ r) & (d ^ r)) & msb != 0;
        self.x = self.c;
        self.n = r & msb != 0;
        self.z = if sticky { self.z && r == 0 } else { r == 0 };
        r
    }

    /// `d - s - x` setting all of XNZVC; with `sticky` Z is only cleared.
    fn sub(&mut self, s: u32, d: u32, x: bool, sz: Sz, sticky: bool) -> u32 {
        let z = self.z;
        let r = self.cmp(s, d, x, sz);
        self.x = self.c;
        if sticky {
            self.z = z && r == 0;
        }
        r
    }

    /// `d - s - x` setting NZVC (X untouched).
    fn cmp(&mut self, s: u32, d: u32, x: bool, sz: Sz) -> u32 {
        let (s, d) = (s & sz.mask(), d & sz.mask());
        let r = d.wrapping_sub(s).wrapping_sub(x as u32) & sz.mask();
        let msb = sz.msb();
        self.c = ((s & !d) | (r & (s | !d))) & msb != 0;
        self.v = ((s ^ d) & (r ^ d)) & msb != 0;
        self.n = r & msb != 0;
        self.z = r == 0;
        r
    }

    /// Packed-BCD `d + s + X`.
    fn abcd(&mut self, s: u32, d: u32) -> u32 {
        let x = self.x as u32;
        let bin = s + d + x;
        // Per-digit carries of the binary sum, and digits that exceed 9.
        let carries = ((s & d) | (!bin & (s | d))) & 0x88;
        let low = bin & 0xff;
        let over9 = ((low + 0x66) ^ low) & 0x110;
        let adj = carries | (over9 >> 1);
        let corr = adj - (adj >> 2);
        let r = (bin + corr) & 0xff;
        self.c = bin + corr > 0xff;
        self.x = self.c;
        self.v = !low & r & 0x80 != 0;
        self.n = r & 0x80 != 0;
        self.z &= r == 0;
        r
    }

    /// Packed-BCD `d - s - X`.
    fn sbcd(&mut self, s: u32, d: u32) -> u32 {
        let x = self.x as u32;
        let bin = d.wrapping_sub(s).wrapping_sub(x) & 0xff;
        let borrows = ((!d & s) | (bin & !d) | (bin & s)) & 0x88;
        let corr = borrows - (borrows >> 2);
        let r = bin.wrapping_sub(corr) & 0xff;
        self.c = (borrows | (!bin & r)) & 0x80 != 0;
        self.x = self.c;
        self.v = bin & !r & 0x80 != 0;
        self.n = r & 0x80 != 0;
        self.z &= r == 0;
        r
    }

    fn cond(&self, cc: u16) -> bool {
        match cc & 15 {
            0 => true,
            1 => false,
            2 => !self.c && !self.z,
            3 => self.c || self.z,
            4 => !self.c,
            5 => self.c,
            6 => !self.z,
            7 => self.z,
            8 => !self.v,
            9 => self.v,
            10 => !self.n,
            11 => self.n,
            12 => self.n == self.v,
            13 => self.n != self.v,
            14 => !self.z && self.n == self.v,
            _ => self.z || self.n != self.v,
        }
    }

    // ---- exceptions ----

    fn enter_supervisor(&mut self) -> u16 {
        let sr = self.sr();
        self.set_sr((sr | 0x2000) & 0x7fff);
        sr
    }

    /// Group 1/2 exception: stack PC and SR, then vector.
    fn exception<B: Bus>(&mut self, b: &mut B, vector: u8, pc: u32) -> R<()> {
        let sr = self.enter_supervisor();
        self.push(b, Sz::L, pc)?;
        self.push(b, Sz::W, sr as u32)?;
        let target = self.read(b, vector as u32 * 4, Sz::L)?;
        self.jump(target)
    }

    fn interrupt<B: Bus>(&mut self, b: &mut B, level: u8) -> R<()> {
        self.cyc += 44 - 20;
        let vector = b.int_ack(level).unwrap_or(VEC_AUTOVECTOR + level);
        let pc = self.pc;
        let r = self.exception(b, vector, pc);
        self.mask = level;
        r
    }

    /// Take the exception for a fault; a second address error while
    /// handling an address error halts the CPU.
    fn fault<B: Bus>(&mut self, b: &mut B, f: Fault) {
        let mut f = f;
        let mut in_address_error = false;
        loop {
            let r = match f {
                Fault::Vector(v) => {
                    let pc = match v {
                        VEC_ILLEGAL | VEC_PRIVILEGE | VEC_LINE_A | VEC_LINE_F => self.ppc,
                        _ => self.pc,
                    };
                    self.cyc += match v {
                        VEC_CHK => 40 - 16,
                        VEC_ZERO_DIVIDE => 38 - 16,
                        _ => 34 - 16,
                    };
                    self.exception(b, v, pc)
                }
                Fault::Address { addr, write, prog } => {
                    if in_address_error {
                        self.halted = true;
                        return;
                    }
                    in_address_error = true;
                    self.cyc += 50 - 28;
                    self.address_error(b, addr, write, prog)
                }
            };
            match r {
                Ok(()) => return,
                Err(e @ Fault::Address { .. }) => f = e,
                Err(_) => {
                    self.halted = true;
                    return;
                }
            }
        }
    }

    /// Group 0 frame: status word, access address, IR, SR, PC.
    fn address_error<B: Bus>(&mut self, b: &mut B, addr: u32, write: bool, prog: bool) -> R<()> {
        let fc = (self.s as u16) << 2 | if prog { 2 } else { 1 };
        let status = (self.ir & 0xffe0) | (!write as u16) << 4 | (prog as u16) << 3 | fc;
        // The stacked PC as the suite records it: for a jump to an odd
        // address, four bytes before the target; otherwise two bytes before
        // the next unfetched word.
        let pc = if prog { addr.wrapping_sub(4) } else { self.pc.wrapping_sub(2) };
        let sr = self.enter_supervisor();
        self.push(b, Sz::L, pc)?;
        self.push(b, Sz::W, sr as u32)?;
        self.push(b, Sz::W, self.ir as u32)?;
        self.push(b, Sz::L, addr)?;
        self.push(b, Sz::W, status as u32)?;
        let target = self.read(b, VEC_ADDRESS_ERROR as u32 * 4, Sz::L)?;
        self.jump(target)
    }

    // ---- instructions ----

    fn exec<B: Bus>(&mut self, b: &mut B, op: u16) -> R<()> {
        match op >> 12 {
            0x0 => self.group0(b, op),
            0x1 => self.op_move(b, op, Sz::B),
            0x2 => self.op_move(b, op, Sz::L),
            0x3 => self.op_move(b, op, Sz::W),
            0x4 => self.group4(b, op),
            0x5 => self.group5(b, op),
            0x6 => self.op_bcc(b, op),
            0x7 => {
                if op & 0x100 != 0 {
                    return Err(ILLEGAL);
                }
                let v = op as i8 as u32;
                self.d[(op >> 9) as usize & 7] = self.logic(v, Sz::L);
                Ok(())
            }
            0x8 => self.group8(b, op),
            0x9 => self.addsub(b, op, true),
            0xa => Err(Fault::Vector(VEC_LINE_A)),
            0xb => self.group_b(b, op),
            0xc => self.group_c(b, op),
            0xd => self.addsub(b, op, false),
            0xe => self.shifts(b, op),
            _ => Err(Fault::Vector(VEC_LINE_F)),
        }
    }

    /// Immediate ops, bit ops and MOVEP.
    fn group0<B: Bus>(&mut self, b: &mut B, op: u16) -> R<()> {
        let (mode, reg) = ((op >> 3) & 7, op & 7);
        if op & 0x100 != 0 {
            if mode == 1 {
                return self.movep(b, op);
            }
            let bit = self.d[(op >> 9) as usize & 7];
            return self.bitop(b, op, bit, true);
        }
        let k = (op >> 9) & 7;
        if k == 4 {
            Self::need(mode, reg, if (op >> 6) & 3 == 0 { DATA & !IMM } else { DATA_ALT })?;
            let bit = self.fetch16(b) as u32 & 0xff;
            return self.bitop(b, op, bit, false);
        }
        let sz = Sz::from_bits(op >> 6).ok_or(ILLEGAL)?;
        if k == 7 {
            return Err(ILLEGAL);
        }
        if mode == 7 && reg == 4 {
            // ORI/ANDI/EORI to CCR (byte) or SR (word).
            if !matches!(k, 0 | 1 | 5) || sz == Sz::L {
                return Err(ILLEGAL);
            }
            if sz == Sz::W && !self.s {
                return Err(PRIVILEGE);
            }
            let imm = self.fetch16(b);
            let cur = self.sr();
            let v = match k {
                0 => cur | imm,
                1 => cur & imm,
                _ => cur ^ imm,
            };
            if sz == Sz::W {
                self.set_sr(v);
            } else {
                self.set_ccr(v as u8);
            }
            self.cyc += 12;
            return Ok(());
        }
        Self::need(mode, reg, DATA_ALT)?;
        let imm = self.imm(b, sz);
        let ea = self.ea(b, mode, reg, sz)?;
        let d = self.rd(b, ea, sz)?;
        let r = match k {
            0 => self.logic(d | imm, sz),
            1 => self.logic(d & imm, sz),
            2 => self.sub(imm, d, false, sz, false),
            3 => self.add(imm, d, false, sz, false),
            5 => self.logic(d ^ imm, sz),
            _ => {
                self.cmp(imm, d, false, sz);
                if mode == 0 && sz == Sz::L {
                    self.cyc += 2;
                }
                return Ok(());
            }
        };
        if mode == 0 && sz == Sz::L {
            self.cyc += 4;
        }
        self.wr(b, ea, sz, r)
    }

    /// BTST/BCHG/BCLR/BSET: long on a data register, byte in memory.
    fn bitop<B: Bus>(&mut self, b: &mut B, op: u16, bit: u32, dynamic: bool) -> R<()> {
        let (mode, reg) = ((op >> 3) & 7, op & 7);
        let kind = (op >> 6) & 3;
        if dynamic {
            Self::need(mode, reg, if kind == 0 { DATA } else { DATA_ALT })?;
        }
        if mode == 0 {
            let r = reg as usize;
            let m = 1u32 << (bit & 31);
            self.z = self.d[r] & m == 0;
            match kind {
                0 => self.cyc += 2,
                1 => self.d[r] ^= m,
                2 => self.d[r] &= !m,
                _ => self.d[r] |= m,
            }
            if kind != 0 {
                let extra = if bit & 31 >= 16 { 4 } else { 2 };
                self.cyc += extra + (kind == 2) as u32 * 2;
            }
            return Ok(());
        }
        let ea = self.ea(b, mode, reg, Sz::B)?;
        let v = self.rd(b, ea, Sz::B)?;
        let m = 1u32 << (bit & 7);
        self.z = v & m == 0;
        let r = match kind {
            0 => return Ok(()),
            1 => v ^ m,
            2 => v & !m,
            _ => v | m,
        };
        self.wr(b, ea, Sz::B, r)
    }

    fn movep<B: Bus>(&mut self, b: &mut B, op: u16) -> R<()> {
        let rx = (op >> 9) as usize & 7;
        let disp = self.fetch16(b) as i16 as u32;
        let addr = self.a[op as usize & 7].wrapping_add(disp);
        let n = if op & 0x40 != 0 { 4 } else { 2 };
        if op & 0x80 != 0 {
            let v = self.d[rx];
            for i in 0..n {
                let byte = v >> (8 * (n - 1 - i));
                self.write(b, addr.wrapping_add(2 * i), Sz::B, byte)?;
            }
        } else {
            let mut v = 0u32;
            for i in 0..n {
                v = v << 8 | self.read(b, addr.wrapping_add(2 * i), Sz::B)?;
            }
            if n == 4 {
                self.d[rx] = v;
            } else {
                self.d[rx] = (self.d[rx] & 0xffff_0000) | v;
            }
        }
        Ok(())
    }

    fn op_move<B: Bus>(&mut self, b: &mut B, op: u16, sz: Sz) -> R<()> {
        let (dmode, dreg) = ((op >> 6) & 7, (op >> 9) & 7);
        if dmode == 1 {
            if sz == Sz::B {
                return Err(ILLEGAL);
            }
            let v = self.src(b, op, sz, ALL)?;
            self.a[dreg as usize] = sz.sext(v);
            return Ok(());
        }
        Self::need(dmode, dreg, DATA_ALT)?;
        let v = self.src(b, op, sz, if sz == Sz::B { DATA } else { ALL })?;
        let v = self.logic(v, sz);
        if dmode == 3 {
            // (An)+ is only bumped once the write has succeeded.
            let r = dreg as usize;
            let addr = self.a[r];
            self.wr(b, Ea::M(addr), sz, v)?;
            self.a[r] = addr.wrapping_add(if sz == Sz::B && r == 7 { 2 } else { sz.bytes() });
            return Ok(());
        }
        let r = if dmode == 4 && sz == Sz::L && self.a[dreg as usize] & 1 != 0 {
            // The low word is written first, after a two-byte decrement.
            let r = dreg as usize;
            self.a[r] = self.a[r].wrapping_sub(2);
            Err(Fault::Address { addr: self.a[r], write: true, prog: false })
        } else {
            let ea = self.ea(b, dmode, dreg, sz)?;
            if dmode == 4 {
                self.cyc -= 2;
            }
            self.wr(b, ea, sz, v)
        };
        if r.is_err() {
            // Where the prefetch had got to when the write faulted, as the
            // stacked PC shows it.
            match (dmode, dreg) {
                (4, _) => self.pc = self.pc.wrapping_add(2),
                (7, 1) if ea_ok((op >> 3) & 7, op & 7, MEMORY & !IMM) => self.pc = self.pc.wrapping_sub(2),
                _ => {}
            }
        }
        r
    }

    fn group4<B: Bus>(&mut self, b: &mut B, op: u16) -> R<()> {
        let (mode, reg) = ((op >> 3) & 7, op & 7);
        let rx = (op >> 9) as usize & 7;
        if op & 0x100 != 0 {
            return match (op >> 6) & 7 {
                6 => self.chk(b, op),
                7 => {
                    Self::need(mode, reg, CONTROL)?;
                    self.a[rx] = self.ea_addr(b, mode, reg)?;
                    Ok(())
                }
                _ => Err(ILLEGAL),
            };
        }
        match (op >> 8) & 0xf {
            k @ (0x0 | 0x2 | 0x4 | 0x6) => match Sz::from_bits(op >> 6) {
                Some(sz) => {
                    Self::need(mode, reg, DATA_ALT)?;
                    let ea = self.ea(b, mode, reg, sz)?;
                    let d = self.rd(b, ea, sz)?;
                    let r = match k {
                        0x0 => {
                            let x = self.x;
                            self.sub(d, 0, x, sz, true)
                        }
                        0x2 => self.logic(0, sz),
                        0x4 => self.sub(d, 0, false, sz, false),
                        _ => self.logic(!d, sz),
                    };
                    if mode == 0 && sz == Sz::L {
                        self.cyc += 2;
                    }
                    self.wr(b, ea, sz, r)
                }
                None => match k {
                    0x0 => {
                        Self::need(mode, reg, DATA_ALT)?;
                        let ea = self.ea(b, mode, reg, Sz::W)?;
                        if let Ea::M(_) = ea {
                            self.rd(b, ea, Sz::W)?;
                        } else {
                            self.cyc += 2;
                        }
                        let sr = self.sr() as u32;
                        self.wr(b, ea, Sz::W, sr)
                    }
                    0x4 => {
                        let v = self.src(b, op, Sz::W, DATA)?;
                        self.set_ccr(v as u8);
                        self.cyc += 8;
                        Ok(())
                    }
                    0x6 => {
                        if !self.s {
                            return Err(PRIVILEGE);
                        }
                        let v = self.src(b, op, Sz::W, DATA)?;
                        self.set_sr(v as u16);
                        self.cyc += 8;
                        Ok(())
                    }
                    _ => Err(ILLEGAL),
                },
            },
            0x8 => match (op >> 6) & 3 {
                0 => {
                    Self::need(mode, reg, DATA_ALT)?;
                    let ea = self.ea(b, mode, reg, Sz::B)?;
                    let d = self.rd(b, ea, Sz::B)?;
                    let r = self.sbcd(d, 0);
                    if mode == 0 {
                        self.cyc += 2;
                    }
                    self.wr(b, ea, Sz::B, r)
                }
                1 if mode == 0 => {
                    let r = reg as usize;
                    let v = self.d[r].rotate_left(16);
                    self.d[r] = self.logic(v, Sz::L);
                    Ok(())
                }
                1 => {
                    Self::need(mode, reg, CONTROL)?;
                    let addr = self.ea_addr(b, mode, reg)?;
                    self.push(b, Sz::L, addr)
                }
                k if mode == 0 => {
                    let r = reg as usize;
                    if k == 2 {
                        let v = self.d[r] as u8 as i8 as u16 as u32;
                        let v = self.logic(v, Sz::W);
                        self.d[r] = (self.d[r] & 0xffff_0000) | v;
                    } else {
                        let v = self.d[r] as u16 as i16 as u32;
                        self.d[r] = self.logic(v, Sz::L);
                    }
                    Ok(())
                }
                _ => self.movem_store(b, op),
            },
            0xa => match Sz::from_bits(op >> 6) {
                Some(sz) => {
                    let v = self.src(b, op, sz, DATA_ALT)?;
                    self.logic(v, sz);
                    Ok(())
                }
                None if op == 0x4afc => Err(ILLEGAL),
                None => {
                    Self::need(mode, reg, DATA_ALT)?;
                    let ea = self.ea(b, mode, reg, Sz::B)?;
                    let v = self.rd(b, ea, Sz::B)?;
                    self.logic(v, Sz::B);
                    if let Ea::M(_) = ea {
                        self.cyc += 2;
                    }
                    self.wr(b, ea, Sz::B, v | 0x80)
                }
            },
            0xc if op & 0x80 != 0 => self.movem_load(b, op),
            0xe => self.group4e(b, op),
            _ => Err(ILLEGAL),
        }
    }

    fn chk<B: Bus>(&mut self, b: &mut B, op: u16) -> R<()> {
        let bound = self.src(b, op, Sz::W, DATA)? as u16 as i16;
        let dn = self.d[(op >> 9) as usize & 7] as u16 as i16;
        self.cyc += 6;
        self.z = dn == 0;
        self.v = false;
        self.c = false;
        if dn < 0 {
            self.n = true;
            Err(Fault::Vector(VEC_CHK))
        } else if dn > bound {
            self.n = false;
            Err(Fault::Vector(VEC_CHK))
        } else {
            Ok(())
        }
    }

    /// MOVEM registers to memory. Mask bit 0 is D0 (A7 for -(An)).
    fn movem_store<B: Bus>(&mut self, b: &mut B, op: u16) -> R<()> {
        let (mode, reg) = ((op >> 3) & 7, op & 7);
        Self::need(mode, reg, CTRL_ALT | PREDEC)?;
        let sz = if op & 0x40 != 0 { Sz::L } else { Sz::W };
        let mask = self.fetch16(b);
        if mode == 4 {
            let r = reg as usize;
            let mut addr = self.a[r];
            for i in 0..16 {
                if mask & (1 << i) != 0 {
                    let n = 15 - i;
                    let v = if n < 8 { self.d[n] } else { self.a[n - 8] };
                    addr = addr.wrapping_sub(sz.bytes());
                    if addr & 1 != 0 {
                        // Longs go out low word first.
                        let at = addr.wrapping_add(sz.bytes() - 2);
                        return Err(Fault::Address { addr: at, write: true, prog: false });
                    }
                    self.write(b, addr, sz, v)?;
                }
            }
            self.a[r] = addr;
            return Ok(());
        }
        let mut addr = self.ea_addr(b, mode, reg)?;
        for i in 0..16 {
            if mask & (1 << i) != 0 {
                let v = if i < 8 { self.d[i] } else { self.a[i - 8] };
                self.write(b, addr, sz, v)?;
                addr = addr.wrapping_add(sz.bytes());
            }
        }
        Ok(())
    }

    /// MOVEM memory to registers; words are sign-extended to 32 bits.
    fn movem_load<B: Bus>(&mut self, b: &mut B, op: u16) -> R<()> {
        let (mode, reg) = ((op >> 3) & 7, op & 7);
        Self::need(mode, reg, CONTROL | POSTINC)?;
        let sz = if op & 0x40 != 0 { Sz::L } else { Sz::W };
        let mask = self.fetch16(b);
        let start = if mode == 3 { self.a[reg as usize] } else { self.ea_addr(b, mode, reg)? };
        let mut addr = start;
        let r = (|| {
            for i in 0..16 {
                if mask & (1 << i) != 0 {
                    let v = sz.sext(self.read(b, addr, sz)?);
                    if i < 8 {
                        self.d[i] = v;
                    } else {
                        self.a[i - 8] = v;
                    }
                    addr = addr.wrapping_add(sz.bytes());
                }
            }
            // The 68000 reads one more word past the end.
            self.read(b, addr, Sz::W).map(|_| ())
        })();
        if mode == 3 {
            // A faulting (An)+ is left one word on, as the suite records it.
            self.a[reg as usize] = if r.is_ok() { addr } else { start.wrapping_add(2) };
        }
        r
    }

    fn group4e<B: Bus>(&mut self, b: &mut B, op: u16) -> R<()> {
        let r = op as usize & 7;
        match op {
            0x4e40..=0x4e4f => Err(Fault::Vector(VEC_TRAP + (op & 15) as u8)),
            0x4e50..=0x4e57 => {
                let disp = self.fetch16(b) as i16 as u32;
                let v = self.a[r];
                self.push(b, Sz::L, if r == 7 { self.a[7].wrapping_sub(4) } else { v })?;
                self.a[r] = self.a[7];
                self.a[7] = self.a[7].wrapping_add(disp);
                Ok(())
            }
            0x4e58..=0x4e5f => {
                let addr = self.a[r];
                let v = self.read(b, addr, Sz::L)?;
                self.a[7] = addr.wrapping_add(4);
                self.a[r] = v;
                Ok(())
            }
            0x4e60..=0x4e6f => {
                if !self.s {
                    return Err(PRIVILEGE);
                }
                if op & 8 == 0 {
                    self.other_sp = self.a[r];
                } else {
                    self.a[r] = self.other_sp;
                }
                Ok(())
            }
            0x4e70 => {
                if !self.s {
                    return Err(PRIVILEGE);
                }
                b.reset_devices();
                self.cyc += 128;
                Ok(())
            }
            0x4e71 => Ok(()),
            0x4e72 => {
                if !self.s {
                    return Err(PRIVILEGE);
                }
                let v = self.fetch16(b);
                self.set_sr(v);
                self.stopped = true;
                Ok(())
            }
            0x4e73 => {
                if !self.s {
                    return Err(PRIVILEGE);
                }
                let sp = self.a[7];
                let sr = self.read(b, sp, Sz::W)?;
                let pc = self.read(b, sp.wrapping_add(2), Sz::L)?;
                self.a[7] = sp.wrapping_add(6);
                self.set_sr(sr as u16);
                self.cyc += 8;
                self.jump(pc)
            }
            0x4e75 => {
                let pc = self.pop(b, Sz::L)?;
                self.cyc += 4;
                self.jump(pc)
            }
            0x4e76 => {
                if self.v {
                    Err(Fault::Vector(VEC_TRAPV))
                } else {
                    Ok(())
                }
            }
            0x4e77 => {
                let ccr = self.pop(b, Sz::W)?;
                self.set_ccr(ccr as u8);
                let pc = self.pop(b, Sz::L)?;
                self.cyc += 4;
                self.jump(pc)
            }
            0x4e80..=0x4eff => {
                let (mode, reg) = ((op >> 3) & 7, op & 7);
                Self::need(mode, reg, CONTROL)?;
                let target = self.ea_addr(b, mode, reg)?;
                let ret = self.pc;
                // JSR faults on an odd target before pushing (BSR after).
                self.jump(target)?;
                if op & 0x40 == 0 {
                    self.push(b, Sz::L, ret)?;
                }
                Ok(())
            }
            _ => Err(ILLEGAL),
        }
    }

    /// ADDQ/SUBQ, Scc, DBcc.
    fn group5<B: Bus>(&mut self, b: &mut B, op: u16) -> R<()> {
        let (mode, reg) = ((op >> 3) & 7, op & 7);
        let Some(sz) = Sz::from_bits(op >> 6) else {
            let cc = (op >> 8) & 15;
            if mode == 1 {
                let base = self.pc;
                let disp = self.fetch16(b) as i16 as u32;
                if self.cond(cc) {
                    self.cyc += 4;
                    return Ok(());
                }
                let r = reg as usize;
                let cnt = (self.d[r] as u16).wrapping_sub(1);
                self.d[r] = (self.d[r] & 0xffff_0000) | cnt as u32;
                self.cyc += 2;
                if cnt != 0xffff {
                    return self.jump(base.wrapping_add(disp));
                }
                self.cyc += 4;
                return Ok(());
            }
            Self::need(mode, reg, DATA_ALT)?;
            let ea = self.ea(b, mode, reg, Sz::B)?;
            if let Ea::M(_) = ea {
                self.rd(b, ea, Sz::B)?;
            }
            let v = if self.cond(cc) { 0xff } else { 0 };
            if mode == 0 && v != 0 {
                self.cyc += 2;
            }
            return self.wr(b, ea, Sz::B, v);
        };
        let q = match (op >> 9) & 7 {
            0 => 8,
            n => n as u32,
        };
        let sub = op & 0x100 != 0;
        if mode == 1 {
            if sz == Sz::B {
                return Err(ILLEGAL);
            }
            let r = reg as usize;
            self.a[r] = if sub { self.a[r].wrapping_sub(q) } else { self.a[r].wrapping_add(q) };
            self.cyc += 4;
            return Ok(());
        }
        Self::need(mode, reg, ALTERABLE)?;
        let ea = self.ea(b, mode, reg, sz)?;
        let d = self.rd(b, ea, sz)?;
        let r = if sub { self.sub(q, d, false, sz, false) } else { self.add(q, d, false, sz, false) };
        if mode == 0 && sz == Sz::L {
            self.cyc += 4;
        }
        self.wr(b, ea, sz, r)
    }

    fn op_bcc<B: Bus>(&mut self, b: &mut B, op: u16) -> R<()> {
        let cc = (op >> 8) & 15;
        let base = self.pc;
        let disp = match op & 0xff {
            0 => self.fetch16(b) as i16 as u32,
            d => d as i8 as u32,
        };
        let target = base.wrapping_add(disp);
        if cc == 1 {
            let ret = self.pc;
            self.push(b, Sz::L, ret)?;
            return self.jump(target);
        }
        if self.cond(cc) {
            self.cyc += 6;
            if op & 0xff == 0 {
                self.cyc -= 4;
            }
            self.jump(target)
        } else {
            self.cyc += 4;
            Ok(())
        }
    }

    /// OR/AND in both directions (`and` picks which).
    fn logic_op<B: Bus>(&mut self, b: &mut B, op: u16, and: bool) -> R<()> {
        let (mode, reg) = ((op >> 3) & 7, op & 7);
        let rx = (op >> 9) as usize & 7;
        let sz = Sz::from_bits(op >> 6).ok_or(ILLEGAL)?;
        let f = |a: u32, b: u32| if and { a & b } else { a | b };
        if op & 0x100 == 0 {
            let s = self.src(b, op, sz, DATA)?;
            let r = self.logic(f(s, self.d[rx]), sz);
            if sz == Sz::L {
                self.cyc += if mode == 0 || (mode == 7 && reg == 4) { 4 } else { 2 };
            }
            self.wr(b, Ea::D(rx), sz, r)
        } else {
            Self::need(mode, reg, MEM_ALT)?;
            let ea = self.ea(b, mode, reg, sz)?;
            let d = self.rd(b, ea, sz)?;
            let r = self.logic(f(d, self.d[rx]), sz);
            self.wr(b, ea, sz, r)
        }
    }

    /// OR, DIVU, DIVS, SBCD.
    fn group8<B: Bus>(&mut self, b: &mut B, op: u16) -> R<()> {
        match (op >> 6) & 7 {
            3 => self.divu(b, op),
            7 => self.divs(b, op),
            4 if op & 0x30 == 0 => self.bcd(b, op, false),
            _ => self.logic_op(b, op, false),
        }
    }

    /// ABCD/SBCD, register or -(An) form.
    fn bcd<B: Bus>(&mut self, b: &mut B, op: u16, add: bool) -> R<()> {
        let (rx, ry) = ((op >> 9) as usize & 7, op as usize & 7);
        let (s, d, dst) = if op & 8 == 0 {
            self.cyc += 2;
            (self.d[ry] & 0xff, self.d[rx] & 0xff, Ea::D(rx))
        } else {
            let sa = self.ea(b, 4, ry as u16, Sz::B)?;
            let s = self.rd(b, sa, Sz::B)?;
            let da = self.ea(b, 4, rx as u16, Sz::B)?;
            let d = self.rd(b, da, Sz::B)?;
            (s, d, da)
        };
        let r = if add { self.abcd(s, d) } else { self.sbcd(s, d) };
        self.wr(b, dst, Sz::B, r)
    }

    fn divu<B: Bus>(&mut self, b: &mut B, op: u16) -> R<()> {
        let rx = (op >> 9) as usize & 7;
        let s = self.src(b, op, Sz::W, DATA)?;
        if s == 0 {
            self.c = false;
            self.v = false;
            self.n = false;
            self.z = false;
            return Err(Fault::Vector(VEC_ZERO_DIVIDE));
        }
        let dvd = self.d[rx];
        let q = dvd / s;
        self.c = false;
        self.cyc += 136;
        if q > 0xffff {
            // Overflow leaves N and Z alone.
            self.v = true;
            return Ok(());
        }
        self.d[rx] = (dvd % s) << 16 | q;
        self.v = false;
        self.set_nz(q, Sz::W);
        Ok(())
    }

    fn divs<B: Bus>(&mut self, b: &mut B, op: u16) -> R<()> {
        let rx = (op >> 9) as usize & 7;
        let s = self.src(b, op, Sz::W, DATA)? as u16 as i16 as i64;
        if s == 0 {
            self.c = false;
            self.v = false;
            self.n = false;
            self.z = false;
            return Err(Fault::Vector(VEC_ZERO_DIVIDE));
        }
        let dvd = self.d[rx] as i32 as i64;
        let q = dvd / s;
        self.c = false;
        self.cyc += 154;
        if !(-0x8000..=0x7fff).contains(&q) {
            self.v = true;
            return Ok(());
        }
        let rem = dvd % s;
        self.d[rx] = (rem as u32) << 16 | (q as u32 & 0xffff);
        self.v = false;
        self.set_nz(q as u32, Sz::W);
        Ok(())
    }

    /// ADD/SUB, ADDA/SUBA, ADDX/SUBX.
    fn addsub<B: Bus>(&mut self, b: &mut B, op: u16, sub: bool) -> R<()> {
        let (mode, reg) = ((op >> 3) & 7, op & 7);
        let rx = (op >> 9) as usize & 7;
        let opmode = (op >> 6) & 7;
        if opmode == 3 || opmode == 7 {
            let sz = if opmode == 3 { Sz::W } else { Sz::L };
            let s = sz.sext(self.src(b, op, sz, ALL)?);
            self.a[rx] = if sub { self.a[rx].wrapping_sub(s) } else { self.a[rx].wrapping_add(s) };
            self.cyc += if sz == Sz::W || mode <= 1 || (mode == 7 && reg == 4) { 4 } else { 2 };
            return Ok(());
        }
        let sz = Sz::from_bits(opmode).unwrap_or(Sz::L);
        if opmode >= 4 && mode <= 1 {
            // ADDX/SUBX.
            let ry = reg as usize;
            let (s, d, dst) = if mode == 0 {
                if sz == Sz::L {
                    self.cyc += 4;
                }
                (self.d[ry], self.d[rx], Ea::D(rx))
            } else {
                let sa = self.predec_x(b, ry, sz)?;
                let s = self.rd(b, sa, sz)?;
                let da = self.predec_x(b, rx, sz)?;
                let d = self.rd(b, da, sz)?;
                (s, d, da)
            };
            let x = self.x;
            let r = if sub { self.sub(s, d, x, sz, true) } else { self.add(s, d, x, sz, true) };
            return self.wr(b, dst, sz, r);
        }
        if opmode < 4 {
            let class = if sz == Sz::B { DATA } else { ALL };
            let s = self.src(b, op, sz, class)?;
            let d = self.d[rx];
            let r = if sub { self.sub(s, d, false, sz, false) } else { self.add(s, d, false, sz, false) };
            if sz == Sz::L {
                self.cyc += if mode <= 1 || (mode == 7 && reg == 4) { 4 } else { 2 };
            }
            self.wr(b, Ea::D(rx), sz, r)
        } else {
            Self::need(mode, reg, MEM_ALT)?;
            let ea = self.ea(b, mode, reg, sz)?;
            let d = self.rd(b, ea, sz)?;
            let s = self.d[rx];
            let r = if sub { self.sub(s, d, false, sz, false) } else { self.add(s, d, false, sz, false) };
            self.wr(b, ea, sz, r)
        }
    }

    /// -(An) for ADDX/SUBX. A long operand is read low word first, so an
    /// odd An faults after only one two-byte decrement.
    fn predec_x<B: Bus>(&mut self, b: &mut B, r: usize, sz: Sz) -> R<Ea> {
        if sz == Sz::L && self.a[r] & 1 != 0 {
            self.a[r] = self.a[r].wrapping_sub(2);
            return Err(Fault::Address { addr: self.a[r], write: false, prog: false });
        }
        self.ea(b, 4, r as u16, sz)
    }

    /// CMP, CMPA, CMPM, EOR.
    fn group_b<B: Bus>(&mut self, b: &mut B, op: u16) -> R<()> {
        let (mode, reg) = ((op >> 3) & 7, op & 7);
        let rx = (op >> 9) as usize & 7;
        let opmode = (op >> 6) & 7;
        if opmode == 3 || opmode == 7 {
            let sz = if opmode == 3 { Sz::W } else { Sz::L };
            let s = sz.sext(self.src(b, op, sz, ALL)?);
            let d = self.a[rx];
            self.cmp(s, d, false, Sz::L);
            self.cyc += 2;
            return Ok(());
        }
        let sz = Sz::from_bits(opmode).unwrap_or(Sz::L);
        if opmode < 4 {
            let s = self.src(b, op, sz, if sz == Sz::B { DATA } else { ALL })?;
            let d = self.d[rx];
            self.cmp(s, d, false, sz);
            if sz == Sz::L {
                self.cyc += 2;
            }
            return Ok(());
        }
        if mode == 1 {
            let sa = self.ea(b, 3, reg, sz)?;
            let s = self.rd(b, sa, sz)?;
            let da = self.ea(b, 3, rx as u16, sz)?;
            let d = self.rd(b, da, sz)?;
            self.cmp(s, d, false, sz);
            return Ok(());
        }
        Self::need(mode, reg, DATA_ALT)?;
        let ea = self.ea(b, mode, reg, sz)?;
        let d = self.rd(b, ea, sz)?;
        let r = self.logic(d ^ self.d[rx], sz);
        if mode == 0 && sz == Sz::L {
            self.cyc += 4;
        }
        self.wr(b, ea, sz, r)
    }

    /// AND, MULU, MULS, ABCD, EXG.
    fn group_c<B: Bus>(&mut self, b: &mut B, op: u16) -> R<()> {
        let rx = (op >> 9) as usize & 7;
        let ry = op as usize & 7;
        match (op >> 6) & 7 {
            3 => {
                let s = self.src(b, op, Sz::W, DATA)?;
                let r = (self.d[rx] & 0xffff) * s;
                self.d[rx] = self.logic(r, Sz::L);
                self.cyc += 34 + 2 * s.count_ones();
                Ok(())
            }
            7 => {
                let s = self.src(b, op, Sz::W, DATA)?;
                let r = (self.d[rx] as u16 as i16 as i32).wrapping_mul(s as u16 as i16 as i32) as u32;
                self.d[rx] = self.logic(r, Sz::L);
                self.cyc += 34 + 2 * (((s << 1) ^ s) & 0xffff).count_ones();
                Ok(())
            }
            _ => match op & 0x1f8 {
                0x100 | 0x108 => self.bcd(b, op, true),
                0x140 => {
                    self.d.swap(rx, ry);
                    self.cyc += 2;
                    Ok(())
                }
                0x148 => {
                    self.a.swap(rx, ry);
                    self.cyc += 2;
                    Ok(())
                }
                0x188 => {
                    std::mem::swap(&mut self.d[rx], &mut self.a[ry]);
                    self.cyc += 2;
                    Ok(())
                }
                _ => self.logic_op(b, op, true),
            },
        }
    }

    fn shifts<B: Bus>(&mut self, b: &mut B, op: u16) -> R<()> {
        let (mode, reg) = ((op >> 3) & 7, op & 7);
        let left = op & 0x100 != 0;
        let Some(sz) = Sz::from_bits(op >> 6) else {
            if op & 0x800 != 0 {
                return Err(ILLEGAL);
            }
            Self::need(mode, reg, MEM_ALT)?;
            let ea = self.ea(b, mode, reg, Sz::W)?;
            let v = self.rd(b, ea, Sz::W)?;
            let r = self.shift((op >> 9) & 3, left, v, 1, Sz::W);
            return self.wr(b, ea, Sz::W, r);
        };
        let rx = (op >> 9) as usize & 7;
        let count = if op & 0x20 != 0 {
            self.d[rx] & 63
        } else {
            match rx {
                0 => 8,
                n => n as u32,
            }
        };
        let r = reg as usize;
        let v = self.shift((op >> 3) & 3, left, self.d[r], count, sz);
        self.wr(b, Ea::D(r), sz, v)?;
        self.cyc += 2 * count + if sz == Sz::L { 4 } else { 2 };
        Ok(())
    }

    /// One of AS/LS/ROX/RO (`kind` 0-3) by `count`, setting flags.
    fn shift(&mut self, kind: u16, left: bool, v: u32, count: u32, sz: Sz) -> u32 {
        let (mask, msb) = (sz.mask(), sz.msb());
        let mut v = v & mask;
        self.v = false;
        if count == 0 {
            self.c = kind == 2 && self.x;
            self.set_nz(v, sz);
            return v;
        }
        for _ in 0..count {
            let out = if left { v & msb != 0 } else { v & 1 != 0 };
            // Bit shifted in: the sign for ASR, X for ROX, the bit out for RO.
            let entry = if left { 1 } else { msb };
            let fill = match kind {
                0 if !left => v & msb,
                2 if self.x => entry,
                3 if out => entry,
                _ => 0,
            };
            let nv = if left { (v << 1) & mask } else { v >> 1 } | fill;
            if kind == 0 && left && (nv ^ v) & msb != 0 {
                self.v = true;
            }
            v = nv;
            self.c = out;
            if kind != 3 {
                self.x = out;
            }
        }
        if kind == 0 && !left && count > sz.bytes() * 8 {
            // ASR past the operand width: the suite has C and X clear.
            self.c = false;
            self.x = false;
        }
        self.set_nz(v, sz);
        v
    }
}

#[cfg(test)]
mod tests;
