//! Z80 interpreter.
//!
//! Instruction-at-a-time: a prefixed instruction (CB, DD, ED, FD, DDCB,
//! FDCB) runs as one step, so an interrupt can't be taken between a prefix
//! and its opcode. `step` returns the T-states used, from the Zilog manual's
//! tables (conditional branches and repeating block instructions count the
//! taken or repeating case). Bus accesses aren't spread over the T-states:
//! each instruction's reads and writes happen at once.
//!
//! Undocumented behaviour modelled: flag bits 3 (X) and 5 (Y) for every
//! instruction, including the MEMPTR (WZ) register that BIT n,(HL) shows;
//! IXH/IXL/IYH/IYL; SLL; DDCB/FDCB results copied to a register; IN (C) and
//! OUT (C),0; the ED mirrors of NEG, RETN and IM; and the block I/O flags.
//! Not modelled: the X/Y flags left by a block instruction interrupted while
//! repeating, and the NMOS quirk where LD A,I/R read P/V as 0 when an
//! interrupt is accepted right after. SCF and CCF take X/Y from A alone.

/// Memory, I/O and interrupt acknowledge as seen by the CPU.
pub trait Bus {
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, v: u8);
    /// Opcode fetch (an M1 cycle): prefixes and opcodes, not operands,
    /// displacements or the DDCB/FDCB opcode byte. Same as `read` unless
    /// the board treats M1 differently.
    fn fetch(&mut self, addr: u16) -> u8 {
        self.read(addr)
    }
    /// IN: the full 16-bit port address (B or A in the high byte).
    fn input(&mut self, port: u16) -> u8;
    /// OUT: the full 16-bit port address (B or A in the high byte).
    fn output(&mut self, port: u16, v: u8);
    /// Interrupt acknowledge: the byte the device puts on the data bus. In
    /// mode 0 an opcode (usually an RST), in mode 2 the vector's low byte;
    /// unused in mode 1. A bus with nothing driving it reads 0xff.
    fn int_ack(&mut self) -> u8 {
        0xff
    }
    /// RETI was executed (Z80 peripherals watch for it to end an interrupt).
    fn reti(&mut self) {}
}

const C: u8 = 0x01;
const N: u8 = 0x02;
const P: u8 = 0x04;
const X: u8 = 0x08;
const H: u8 = 0x10;
const Y: u8 = 0x20;
const Z: u8 = 0x40;
const S: u8 = 0x80;
const XY: u8 = X | Y;

/// Flag bits for the F register.
pub mod flags {
    pub const C: u8 = super::C;
    pub const N: u8 = super::N;
    /// Parity/overflow.
    pub const PV: u8 = super::P;
    /// Undocumented: bit 3.
    pub const X: u8 = super::X;
    pub const H: u8 = super::H;
    /// Undocumented: bit 5.
    pub const Y: u8 = super::Y;
    pub const Z: u8 = super::Z;
    pub const S: u8 = super::S;
}

/// S, Z, X and Y for a result byte.
static SZXY: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = (i as u8 & (S | XY)) | if i == 0 { Z } else { 0 };
        i += 1;
    }
    t
};

/// S, Z, X, Y and even parity for a result byte.
static SZXYP: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = SZXY[i] | if (i as u8).count_ones() & 1 == 0 { P } else { 0 };
        i += 1;
    }
    t
};

/// T-states for unprefixed opcodes; conditional ones are the not-taken
/// count (the extra is added when taken), prefixes are 0 (counted in their
/// own handler).
#[rustfmt::skip]
static CYCLES: [u8; 256] = [
    4, 10, 7, 6, 4, 4, 7, 4, 4, 11, 7, 6, 4, 4, 7, 4,
    8, 10, 7, 6, 4, 4, 7, 4, 12, 11, 7, 6, 4, 4, 7, 4,
    7, 10, 16, 6, 4, 4, 7, 4, 7, 11, 16, 6, 4, 4, 7, 4,
    7, 10, 13, 6, 11, 11, 10, 4, 7, 11, 13, 6, 4, 4, 7, 4,
    4, 4, 4, 4, 4, 4, 7, 4, 4, 4, 4, 4, 4, 4, 7, 4,
    4, 4, 4, 4, 4, 4, 7, 4, 4, 4, 4, 4, 4, 4, 7, 4,
    4, 4, 4, 4, 4, 4, 7, 4, 4, 4, 4, 4, 4, 4, 7, 4,
    7, 7, 7, 7, 7, 7, 4, 7, 4, 4, 4, 4, 4, 4, 7, 4,
    4, 4, 4, 4, 4, 4, 7, 4, 4, 4, 4, 4, 4, 4, 7, 4,
    4, 4, 4, 4, 4, 4, 7, 4, 4, 4, 4, 4, 4, 4, 7, 4,
    4, 4, 4, 4, 4, 4, 7, 4, 4, 4, 4, 4, 4, 4, 7, 4,
    4, 4, 4, 4, 4, 4, 7, 4, 4, 4, 4, 4, 4, 4, 7, 4,
    5, 10, 10, 10, 10, 11, 7, 11, 5, 10, 10, 0, 10, 17, 7, 11,
    5, 10, 10, 11, 10, 11, 7, 11, 5, 4, 10, 11, 10, 0, 7, 11,
    5, 10, 10, 19, 10, 11, 7, 11, 5, 4, 10, 4, 10, 0, 7, 11,
    5, 10, 10, 4, 10, 11, 7, 11, 5, 6, 10, 4, 10, 0, 7, 11,
];

/// Index into `xy` for HL; IX and IY are 1 and 2.
const HL: usize = 0;

/// Z80 register state.
#[derive(Clone, Debug)]
pub struct Cpu {
    pub a: u8,
    pub f: u8,
    pub bc: u16,
    pub de: u16,
    /// HL, IX and IY, indexed by the prefix in force (none, DD, FD).
    pub xy: [u16; 3],
    pub sp: u16,
    pub pc: u16,
    /// The alternate set, swapped in by EX AF,AF' and EXX.
    pub af_alt: u16,
    pub bc_alt: u16,
    pub de_alt: u16,
    pub hl_alt: u16,
    pub i: u8,
    /// Refresh register: the low seven bits count M1 cycles, bit 7 only
    /// changes by LD R,A.
    pub r: u8,
    pub iff1: bool,
    pub iff2: bool,
    /// Interrupt mode, 0-2.
    pub im: u8,
    /// MEMPTR, the internal address latch some flags leak.
    pub wz: u16,
    halted: bool,
    /// EI was the last instruction: no interrupt before the next one.
    ei_delay: bool,
    int_line: bool,
    nmi_line: bool,
    nmi_pending: bool,
    /// T-states of the instruction in progress.
    cyc: u32,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    /// A CPU in its reset state.
    pub fn new() -> Self {
        let mut c = Cpu {
            a: 0,
            f: 0,
            bc: 0,
            de: 0,
            xy: [0; 3],
            sp: 0,
            pc: 0,
            af_alt: 0,
            bc_alt: 0,
            de_alt: 0,
            hl_alt: 0,
            i: 0,
            r: 0,
            iff1: false,
            iff2: false,
            im: 0,
            wz: 0,
            halted: false,
            ei_delay: false,
            int_line: false,
            nmi_line: false,
            nmi_pending: false,
            cyc: 0,
        };
        c.reset();
        c
    }

    /// The RESET input: PC, I and R cleared, interrupts disabled, mode 0.
    /// The manual leaves the other registers undefined; AF and SP are set to
    /// 0xffff as NMOS parts are commonly found, the rest kept.
    pub fn reset(&mut self) {
        self.pc = 0;
        self.i = 0;
        self.r = 0;
        self.iff1 = false;
        self.iff2 = false;
        self.im = 0;
        self.a = 0xff;
        self.f = 0xff;
        self.sp = 0xffff;
        self.halted = false;
        self.ei_delay = false;
        self.nmi_pending = false;
    }

    pub fn hl(&self) -> u16 {
        self.xy[HL]
    }
    pub fn ix(&self) -> u16 {
        self.xy[1]
    }
    pub fn iy(&self) -> u16 {
        self.xy[2]
    }
    pub fn set_hl(&mut self, v: u16) {
        self.xy[HL] = v;
    }
    pub fn set_ix(&mut self, v: u16) {
        self.xy[1] = v;
    }
    pub fn set_iy(&mut self, v: u16) {
        self.xy[2] = v;
    }
    pub fn af(&self) -> u16 {
        (self.a as u16) << 8 | self.f as u16
    }
    pub fn set_af(&mut self, v: u16) {
        self.a = (v >> 8) as u8;
        self.f = v as u8;
    }

    /// True while in HALT, waiting for an interrupt.
    pub fn halted(&self) -> bool {
        self.halted
    }

    /// Drive the maskable INT line (level-sensitive, active while `on`).
    /// The device should release it once acknowledged (`Bus::int_ack`).
    pub fn set_int(&mut self, on: bool) {
        self.int_line = on;
    }

    /// Drive the NMI line; a non-maskable interrupt is taken on its rising
    /// edge.
    pub fn set_nmi(&mut self, on: bool) {
        if on && !self.nmi_line {
            self.nmi_pending = true;
        }
        self.nmi_line = on;
    }

    /// Run whole instructions until at least `budget` T-states have passed;
    /// returns the T-states used (at most one instruction over the budget).
    pub fn run<B: Bus>(&mut self, b: &mut B, budget: u32) -> u32 {
        let mut used = 0;
        while used < budget {
            used += self.step(b);
        }
        used
    }

    /// Execute one instruction (or accept one interrupt, or idle one HALT
    /// cycle); returns the T-states used.
    pub fn step<B: Bus>(&mut self, b: &mut B) -> u32 {
        if self.ei_delay {
            self.ei_delay = false;
        } else if self.nmi_pending {
            return self.take_nmi(b);
        } else if self.int_line && self.iff1 {
            return self.take_int(b);
        }
        if self.halted {
            // HALT runs NOPs, which refresh like any opcode fetch.
            self.inc_r();
            return 4;
        }
        self.cyc = 0;
        let op = self.fetch_op(b);
        self.exec(b, op, HL);
        self.cyc
    }

    fn take_nmi<B: Bus>(&mut self, b: &mut B) -> u32 {
        self.nmi_pending = false;
        self.halted = false;
        // IFF2 keeps the old IFF1 for RETN to restore.
        self.iff1 = false;
        self.inc_r();
        self.push(b, self.pc);
        self.pc = 0x66;
        self.wz = self.pc;
        11
    }

    fn take_int<B: Bus>(&mut self, b: &mut B) -> u32 {
        self.halted = false;
        self.iff1 = false;
        self.iff2 = false;
        self.inc_r();
        match self.im {
            0 => {
                let op = b.int_ack();
                if op & 0xc7 == 0xc7 {
                    self.push(b, self.pc);
                    self.pc = (op & 0x38) as u16;
                    self.wz = self.pc;
                    13
                } else {
                    // Any other opcode runs as if fetched; operand bytes
                    // come from memory at PC, not from the device.
                    self.cyc = 2;
                    self.exec(b, op, HL);
                    self.cyc
                }
            }
            1 => {
                self.push(b, self.pc);
                self.pc = 0x38;
                self.wz = self.pc;
                13
            }
            _ => {
                let v = b.int_ack();
                self.push(b, self.pc);
                let addr = (self.i as u16) << 8 | v as u16;
                self.pc = self.read16(b, addr);
                self.wz = self.pc;
                19
            }
        }
    }

    #[inline]
    fn inc_r(&mut self) {
        self.r = (self.r & 0x80) | (self.r.wrapping_add(1) & 0x7f);
    }

    #[inline]
    fn fetch_op<B: Bus>(&mut self, b: &mut B) -> u8 {
        let v = b.fetch(self.pc);
        self.pc = self.pc.wrapping_add(1);
        self.inc_r();
        v
    }

    #[inline]
    fn imm8<B: Bus>(&mut self, b: &mut B) -> u8 {
        let v = b.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        v
    }

    #[inline]
    fn imm16<B: Bus>(&mut self, b: &mut B) -> u16 {
        let lo = self.imm8(b) as u16;
        let hi = self.imm8(b) as u16;
        hi << 8 | lo
    }

    #[inline]
    fn read16<B: Bus>(&mut self, b: &mut B, addr: u16) -> u16 {
        let lo = b.read(addr) as u16;
        let hi = b.read(addr.wrapping_add(1)) as u16;
        hi << 8 | lo
    }

    #[inline]
    fn write16<B: Bus>(&mut self, b: &mut B, addr: u16, v: u16) {
        b.write(addr, v as u8);
        b.write(addr.wrapping_add(1), (v >> 8) as u8);
    }

    #[inline]
    fn push<B: Bus>(&mut self, b: &mut B, v: u16) {
        self.sp = self.sp.wrapping_sub(1);
        b.write(self.sp, (v >> 8) as u8);
        self.sp = self.sp.wrapping_sub(1);
        b.write(self.sp, v as u8);
    }

    #[inline]
    fn pop<B: Bus>(&mut self, b: &mut B) -> u16 {
        let v = self.read16(b, self.sp);
        self.sp = self.sp.wrapping_add(2);
        v
    }

    /// 8-bit register by its opcode field (not 6); H and L are IXH/IXL or
    /// IYH/IYL under a prefix.
    #[inline]
    fn r8(&self, r: u8, x: usize) -> u8 {
        match r {
            0 => (self.bc >> 8) as u8,
            1 => self.bc as u8,
            2 => (self.de >> 8) as u8,
            3 => self.de as u8,
            4 => (self.xy[x] >> 8) as u8,
            5 => self.xy[x] as u8,
            _ => self.a,
        }
    }

    #[inline]
    fn set_r8(&mut self, r: u8, x: usize, v: u8) {
        let hi = |w: u16| (w & 0x00ff) | (v as u16) << 8;
        let lo = |w: u16| (w & 0xff00) | v as u16;
        match r {
            0 => self.bc = hi(self.bc),
            1 => self.bc = lo(self.bc),
            2 => self.de = hi(self.de),
            3 => self.de = lo(self.de),
            4 => self.xy[x] = hi(self.xy[x]),
            5 => self.xy[x] = lo(self.xy[x]),
            _ => self.a = v,
        }
    }

    /// Register pair by its opcode field: BC, DE, HL/IX/IY, SP.
    #[inline]
    fn rp(&self, p: u8, x: usize) -> u16 {
        match p {
            0 => self.bc,
            1 => self.de,
            2 => self.xy[x],
            _ => self.sp,
        }
    }

    #[inline]
    fn set_rp(&mut self, p: u8, x: usize, v: u16) {
        match p {
            0 => self.bc = v,
            1 => self.de = v,
            2 => self.xy[x] = v,
            _ => self.sp = v,
        }
    }

    /// The (HL) operand's address: HL, or (IX+d)/(IY+d) with the
    /// displacement fetched (8 more T-states).
    #[inline]
    fn ea<B: Bus>(&mut self, b: &mut B, x: usize) -> u16 {
        if x == HL {
            self.xy[HL]
        } else {
            let d = self.imm8(b) as i8;
            let a = self.xy[x].wrapping_add(d as u16);
            self.wz = a;
            self.cyc += 8;
            a
        }
    }

    #[inline]
    fn cond(&self, c: u8) -> bool {
        let f = self.f;
        match c {
            0 => f & Z == 0,
            1 => f & Z != 0,
            2 => f & C == 0,
            3 => f & C != 0,
            4 => f & P == 0,
            5 => f & P != 0,
            6 => f & S == 0,
            _ => f & S != 0,
        }
    }

    // ---- ALU ----

    #[inline]
    fn add8(&mut self, v: u8, c: u8) {
        let a = self.a;
        let r = a as u16 + v as u16 + c as u16;
        let r8 = r as u8;
        self.f = SZXY[r8 as usize] | ((a ^ v ^ r8) & H) | (((a ^ !v) & (a ^ r8) & 0x80) >> 5) | (r >> 8) as u8;
        self.a = r8;
    }

    /// A - v - c with flags; returns the result without storing it.
    #[inline]
    fn sub8(&mut self, v: u8, c: u8) -> u8 {
        let a = self.a;
        let r = (a as u16).wrapping_sub(v as u16).wrapping_sub(c as u16);
        let r8 = r as u8;
        self.f = SZXY[r8 as usize] | N | ((a ^ v ^ r8) & H) | (((a ^ v) & (a ^ r8) & 0x80) >> 5) | ((r >> 8) as u8 & C);
        r8
    }

    #[inline]
    fn alu(&mut self, op: u8, v: u8) {
        match op {
            0 => self.add8(v, 0),
            1 => self.add8(v, self.f & C),
            2 => self.a = self.sub8(v, 0),
            3 => self.a = self.sub8(v, self.f & C),
            4 => {
                self.a &= v;
                self.f = SZXYP[self.a as usize] | H;
            }
            5 => {
                self.a ^= v;
                self.f = SZXYP[self.a as usize];
            }
            6 => {
                self.a |= v;
                self.f = SZXYP[self.a as usize];
            }
            _ => {
                // CP: X and Y come from the operand, not the result.
                self.sub8(v, 0);
                self.f = (self.f & !XY) | (v & XY);
            }
        }
    }

    #[inline]
    fn inc8(&mut self, v: u8) -> u8 {
        let r = v.wrapping_add(1);
        self.f = (self.f & C) | SZXY[r as usize] | if r & 0xf == 0 { H } else { 0 } | if r == 0x80 { P } else { 0 };
        r
    }

    #[inline]
    fn dec8(&mut self, v: u8) -> u8 {
        let r = v.wrapping_sub(1);
        self.f = (self.f & C) | N | SZXY[r as usize] | if v & 0xf == 0 { H } else { 0 } | if r == 0x7f { P } else { 0 };
        r
    }

    /// CB-prefixed rotate or shift `op` (0-7: RLC RRC RL RR SLA SRA SLL SRL).
    #[inline]
    fn rot(&mut self, op: u8, v: u8) -> u8 {
        let fc = self.f & C;
        let (r, c) = match op {
            0 => (v.rotate_left(1), v >> 7),
            1 => (v.rotate_right(1), v & 1),
            2 => (v << 1 | fc, v >> 7),
            3 => (v >> 1 | fc << 7, v & 1),
            4 => (v << 1, v >> 7),
            5 => (v >> 1 | (v & 0x80), v & 1),
            6 => (v << 1 | 1, v >> 7),
            _ => (v >> 1, v & 1),
        };
        self.f = SZXYP[r as usize] | c;
        r
    }

    /// BIT n: X and Y come from `xy_src` (the register, or for memory
    /// operands the high byte of the address latch).
    #[inline]
    fn bit(&mut self, n: u8, v: u8, xy_src: u8) {
        let t = v & (1 << n);
        self.f = (self.f & C) | H | (xy_src & XY) | if t == 0 { Z | P } else { t & S };
    }

    #[inline]
    fn add16(&mut self, a: u16, v: u16) -> u16 {
        let r = a as u32 + v as u32;
        let r16 = r as u16;
        self.f = (self.f & (S | Z | P)) | ((r16 >> 8) as u8 & XY) | (((a ^ v ^ r16) >> 8) as u8 & H) | (r >> 16) as u8;
        self.wz = a.wrapping_add(1);
        r16
    }

    fn adc16(&mut self, a: u16, v: u16) -> u16 {
        let r = a as u32 + v as u32 + (self.f & C) as u32;
        let r16 = r as u16;
        self.f = ((r16 >> 8) as u8 & (S | XY))
            | if r16 == 0 { Z } else { 0 }
            | (((a ^ v ^ r16) >> 8) as u8 & H)
            | ((((a ^ !v) & (a ^ r16) & 0x8000) >> 13) as u8)
            | (r >> 16) as u8;
        self.wz = a.wrapping_add(1);
        r16
    }

    fn sbc16(&mut self, a: u16, v: u16) -> u16 {
        let r = (a as u32).wrapping_sub(v as u32).wrapping_sub((self.f & C) as u32);
        let r16 = r as u16;
        self.f = ((r16 >> 8) as u8 & (S | XY))
            | N
            | if r16 == 0 { Z } else { 0 }
            | (((a ^ v ^ r16) >> 8) as u8 & H)
            | ((((a ^ v) & (a ^ r16) & 0x8000) >> 13) as u8)
            | ((r >> 16) as u8 & C);
        self.wz = a.wrapping_add(1);
        r16
    }

    fn daa(&mut self) {
        let a = self.a;
        let f = self.f;
        let lo = a & 0xf;
        let mut diff = 0;
        let mut c = 0;
        if f & H != 0 || lo > 9 {
            diff |= 0x06;
        }
        if f & C != 0 || a > 0x99 {
            diff |= 0x60;
            c = C;
        }
        let (r, h) =
            if f & N != 0 { (a.wrapping_sub(diff), f & H != 0 && lo < 6) } else { (a.wrapping_add(diff), lo > 9) };
        self.a = r;
        self.f = SZXYP[r as usize] | (f & N) | c | if h { H } else { 0 };
    }

    // ---- execution ----

    /// Execute `op` (already fetched) under prefix `x` (HL, or 1/2 for
    /// DD/FD).
    fn exec<B: Bus>(&mut self, b: &mut B, op: u8, x: usize) {
        self.cyc += CYCLES[op as usize] as u32;
        let y = (op >> 3) & 7;
        let p = y >> 1;
        match op {
            0x00 => {}
            0x01 | 0x11 | 0x21 | 0x31 => {
                let v = self.imm16(b);
                self.set_rp(p, x, v);
            }
            0x02 | 0x12 => {
                let addr = if op == 0x02 { self.bc } else { self.de };
                b.write(addr, self.a);
                self.wz = (self.a as u16) << 8 | (addr.wrapping_add(1) & 0xff);
            }
            0x0a | 0x1a => {
                let addr = if op == 0x0a { self.bc } else { self.de };
                self.a = b.read(addr);
                self.wz = addr.wrapping_add(1);
            }
            0x22 => {
                let nn = self.imm16(b);
                self.write16(b, nn, self.xy[x]);
                self.wz = nn.wrapping_add(1);
            }
            0x2a => {
                let nn = self.imm16(b);
                self.xy[x] = self.read16(b, nn);
                self.wz = nn.wrapping_add(1);
            }
            0x32 => {
                let nn = self.imm16(b);
                b.write(nn, self.a);
                self.wz = (self.a as u16) << 8 | (nn.wrapping_add(1) & 0xff);
            }
            0x3a => {
                let nn = self.imm16(b);
                self.a = b.read(nn);
                self.wz = nn.wrapping_add(1);
            }
            0x03 | 0x13 | 0x23 | 0x33 => {
                let v = self.rp(p, x).wrapping_add(1);
                self.set_rp(p, x, v);
            }
            0x0b | 0x1b | 0x2b | 0x3b => {
                let v = self.rp(p, x).wrapping_sub(1);
                self.set_rp(p, x, v);
            }
            0x09 | 0x19 | 0x29 | 0x39 => {
                let v = self.rp(p, x);
                self.xy[x] = self.add16(self.xy[x], v);
            }
            0x34 => {
                let addr = self.ea(b, x);
                let v = b.read(addr);
                let r = self.inc8(v);
                b.write(addr, r);
            }
            0x35 => {
                let addr = self.ea(b, x);
                let v = b.read(addr);
                let r = self.dec8(v);
                b.write(addr, r);
            }
            0x04 | 0x0c | 0x14 | 0x1c | 0x24 | 0x2c | 0x3c => {
                let r = self.inc8(self.r8(y, x));
                self.set_r8(y, x, r);
            }
            0x05 | 0x0d | 0x15 | 0x1d | 0x25 | 0x2d | 0x3d => {
                let r = self.dec8(self.r8(y, x));
                self.set_r8(y, x, r);
            }
            0x36 => {
                let addr = self.ea(b, x);
                let n = self.imm8(b);
                b.write(addr, n);
                if x != HL {
                    // The displacement and operand fetches overlap.
                    self.cyc -= 3;
                }
            }
            0x06 | 0x0e | 0x16 | 0x1e | 0x26 | 0x2e | 0x3e => {
                let n = self.imm8(b);
                self.set_r8(y, x, n);
            }
            0x07 => {
                self.a = self.a.rotate_left(1);
                self.f = (self.f & (S | Z | P)) | (self.a & (XY | C));
            }
            0x0f => {
                let c = self.a & 1;
                self.a = self.a.rotate_right(1);
                self.f = (self.f & (S | Z | P)) | (self.a & XY) | c;
            }
            0x17 => {
                let c = self.a >> 7;
                self.a = self.a << 1 | (self.f & C);
                self.f = (self.f & (S | Z | P)) | (self.a & XY) | c;
            }
            0x1f => {
                let c = self.a & 1;
                self.a = self.a >> 1 | (self.f & C) << 7;
                self.f = (self.f & (S | Z | P)) | (self.a & XY) | c;
            }
            0x08 => {
                let af = self.af();
                self.set_af(self.af_alt);
                self.af_alt = af;
            }
            0x10 => {
                let d = self.imm8(b) as i8;
                let bb = ((self.bc >> 8) as u8).wrapping_sub(1);
                self.bc = (self.bc & 0xff) | (bb as u16) << 8;
                if bb != 0 {
                    self.pc = self.pc.wrapping_add(d as u16);
                    self.wz = self.pc;
                    self.cyc += 5;
                }
            }
            0x18 => {
                let d = self.imm8(b) as i8;
                self.pc = self.pc.wrapping_add(d as u16);
                self.wz = self.pc;
            }
            0x20 | 0x28 | 0x30 | 0x38 => {
                let d = self.imm8(b) as i8;
                if self.cond(y - 4) {
                    self.pc = self.pc.wrapping_add(d as u16);
                    self.wz = self.pc;
                    self.cyc += 5;
                }
            }
            0x27 => self.daa(),
            0x2f => {
                self.a = !self.a;
                self.f = (self.f & (S | Z | P | C)) | H | N | (self.a & XY);
            }
            0x37 => self.f = (self.f & (S | Z | P)) | C | (self.a & XY),
            0x3f => {
                let c = self.f & C;
                self.f = (self.f & (S | Z | P)) | (self.a & XY) | if c != 0 { H } else { C };
            }
            0x76 => self.halted = true,
            0x40..=0x7f => {
                let src = op & 7;
                if src == 6 {
                    // LD r,(IX+d): r is the real H or L.
                    let addr = self.ea(b, x);
                    let v = b.read(addr);
                    self.set_r8(y, HL, v);
                } else if y == 6 {
                    let addr = self.ea(b, x);
                    b.write(addr, self.r8(src, HL));
                } else {
                    self.set_r8(y, x, self.r8(src, x));
                }
            }
            0x80..=0xbf => {
                let src = op & 7;
                let v = if src == 6 {
                    let addr = self.ea(b, x);
                    b.read(addr)
                } else {
                    self.r8(src, x)
                };
                self.alu(y, v);
            }
            0xc6 | 0xce | 0xd6 | 0xde | 0xe6 | 0xee | 0xf6 | 0xfe => {
                let n = self.imm8(b);
                self.alu(y, n);
            }
            0xc0 | 0xc8 | 0xd0 | 0xd8 | 0xe0 | 0xe8 | 0xf0 | 0xf8 => {
                if self.cond(y) {
                    self.pc = self.pop(b);
                    self.wz = self.pc;
                    self.cyc += 6;
                }
            }
            0xc9 => {
                self.pc = self.pop(b);
                self.wz = self.pc;
            }
            0xc1 | 0xd1 | 0xe1 => {
                let v = self.pop(b);
                self.set_rp(p, x, v);
            }
            0xf1 => {
                let v = self.pop(b);
                self.set_af(v);
            }
            0xc5 | 0xd5 | 0xe5 => {
                let v = self.rp(p, x);
                self.push(b, v);
            }
            0xf5 => self.push(b, self.af()),
            0xc2 | 0xca | 0xd2 | 0xda | 0xe2 | 0xea | 0xf2 | 0xfa => {
                let nn = self.imm16(b);
                self.wz = nn;
                if self.cond(y) {
                    self.pc = nn;
                }
            }
            0xc3 => {
                let nn = self.imm16(b);
                self.wz = nn;
                self.pc = nn;
            }
            0xc4 | 0xcc | 0xd4 | 0xdc | 0xe4 | 0xec | 0xf4 | 0xfc => {
                let nn = self.imm16(b);
                self.wz = nn;
                if self.cond(y) {
                    self.push(b, self.pc);
                    self.pc = nn;
                    self.cyc += 7;
                }
            }
            0xcd => {
                let nn = self.imm16(b);
                self.wz = nn;
                self.push(b, self.pc);
                self.pc = nn;
            }
            0xc7 | 0xcf | 0xd7 | 0xdf | 0xe7 | 0xef | 0xf7 | 0xff => {
                self.push(b, self.pc);
                self.pc = (op & 0x38) as u16;
                self.wz = self.pc;
            }
            0xd3 => {
                let n = self.imm8(b);
                b.output((self.a as u16) << 8 | n as u16, self.a);
                self.wz = (self.a as u16) << 8 | (n.wrapping_add(1) as u16);
            }
            0xdb => {
                let n = self.imm8(b);
                let port = (self.a as u16) << 8 | n as u16;
                self.a = b.input(port);
                self.wz = port.wrapping_add(1);
            }
            0xd9 => {
                std::mem::swap(&mut self.bc, &mut self.bc_alt);
                std::mem::swap(&mut self.de, &mut self.de_alt);
                std::mem::swap(&mut self.xy[HL], &mut self.hl_alt);
            }
            0xe3 => {
                let v = self.read16(b, self.sp);
                self.write16(b, self.sp, self.xy[x]);
                self.xy[x] = v;
                self.wz = v;
            }
            0xe9 => self.pc = self.xy[x],
            // EX DE,HL ignores DD/FD.
            0xeb => std::mem::swap(&mut self.de, &mut self.xy[HL]),
            0xf3 => {
                self.iff1 = false;
                self.iff2 = false;
            }
            0xfb => {
                self.iff1 = true;
                self.iff2 = true;
                self.ei_delay = true;
            }
            0xf9 => self.sp = self.xy[x],
            0xcb => {
                if x == HL {
                    self.exec_cb(b)
                } else {
                    self.exec_xcb(b, x)
                }
            }
            0xdd | 0xfd => {
                // A prefix costs 4; a later prefix overrides an earlier one.
                self.cyc += 4;
                let nx = if op == 0xdd { 1 } else { 2 };
                let op = self.fetch_op(b);
                self.exec(b, op, nx);
            }
            0xed => {
                let op = self.fetch_op(b);
                self.exec_ed(b, op);
            }
        }
    }

    fn exec_cb<B: Bus>(&mut self, b: &mut B) {
        let op = self.fetch_op(b);
        let r = op & 7;
        let n = (op >> 3) & 7;
        if r == 6 {
            let addr = self.xy[HL];
            let v = b.read(addr);
            match op >> 6 {
                0 => {
                    let v = self.rot(n, v);
                    b.write(addr, v);
                    self.cyc += 15;
                }
                1 => {
                    self.bit(n, v, (self.wz >> 8) as u8);
                    self.cyc += 12;
                }
                2 => {
                    b.write(addr, v & !(1 << n));
                    self.cyc += 15;
                }
                _ => {
                    b.write(addr, v | 1 << n);
                    self.cyc += 15;
                }
            }
        } else {
            let v = self.r8(r, HL);
            self.cyc += 8;
            match op >> 6 {
                0 => {
                    let v = self.rot(n, v);
                    self.set_r8(r, HL, v);
                }
                1 => self.bit(n, v, v),
                2 => self.set_r8(r, HL, v & !(1 << n)),
                _ => self.set_r8(r, HL, v | 1 << n),
            }
        }
    }

    /// DDCB d op / FDCB d op. Neither the displacement nor the opcode is an
    /// M1 fetch. Except for BIT, the result is also copied to the register
    /// the low three bits name (the real H and L), unless that is 6.
    fn exec_xcb<B: Bus>(&mut self, b: &mut B, x: usize) {
        let d = self.imm8(b) as i8;
        let op = self.imm8(b);
        let addr = self.xy[x].wrapping_add(d as u16);
        self.wz = addr;
        let v = b.read(addr);
        let n = (op >> 3) & 7;
        let res = match op >> 6 {
            0 => self.rot(n, v),
            1 => {
                self.bit(n, v, (addr >> 8) as u8);
                self.cyc += 16;
                return;
            }
            2 => v & !(1 << n),
            _ => v | 1 << n,
        };
        b.write(addr, res);
        if op & 7 != 6 {
            self.set_r8(op & 7, HL, res);
        }
        self.cyc += 19;
    }

    fn exec_ed<B: Bus>(&mut self, b: &mut B, op: u8) {
        let y = (op >> 3) & 7;
        let p = y >> 1;
        self.cyc += 8;
        match op {
            0x40 | 0x48 | 0x50 | 0x58 | 0x60 | 0x68 | 0x70 | 0x78 => {
                let v = b.input(self.bc);
                self.wz = self.bc.wrapping_add(1);
                if y != 6 {
                    self.set_r8(y, HL, v);
                }
                self.f = (self.f & C) | SZXYP[v as usize];
                self.cyc += 4;
            }
            0x41 | 0x49 | 0x51 | 0x59 | 0x61 | 0x69 | 0x71 | 0x79 => {
                // OUT (C),0 for the (HL) slot (NMOS parts).
                let v = if y == 6 { 0 } else { self.r8(y, HL) };
                b.output(self.bc, v);
                self.wz = self.bc.wrapping_add(1);
                self.cyc += 4;
            }
            0x42 | 0x52 | 0x62 | 0x72 => {
                let v = self.rp(p, HL);
                self.xy[HL] = self.sbc16(self.xy[HL], v);
                self.cyc += 7;
            }
            0x4a | 0x5a | 0x6a | 0x7a => {
                let v = self.rp(p, HL);
                self.xy[HL] = self.adc16(self.xy[HL], v);
                self.cyc += 7;
            }
            0x43 | 0x53 | 0x63 | 0x73 => {
                let nn = self.imm16(b);
                let v = self.rp(p, HL);
                self.write16(b, nn, v);
                self.wz = nn.wrapping_add(1);
                self.cyc += 12;
            }
            0x4b | 0x5b | 0x6b | 0x7b => {
                let nn = self.imm16(b);
                let v = self.read16(b, nn);
                self.set_rp(p, HL, v);
                self.wz = nn.wrapping_add(1);
                self.cyc += 12;
            }
            0x44 | 0x4c | 0x54 | 0x5c | 0x64 | 0x6c | 0x74 | 0x7c => {
                let v = self.a;
                self.a = 0;
                self.a = self.sub8(v, 0);
            }
            0x45 | 0x4d | 0x55 | 0x5d | 0x65 | 0x6d | 0x75 | 0x7d => {
                // RETN, RETI (0x4d) and their mirrors: all copy IFF2 to IFF1.
                self.pc = self.pop(b);
                self.wz = self.pc;
                self.iff1 = self.iff2;
                if op == 0x4d {
                    b.reti();
                }
                self.cyc += 6;
            }
            0x46 | 0x4e | 0x56 | 0x5e | 0x66 | 0x6e | 0x76 | 0x7e => {
                self.im = match y & 3 {
                    0 | 1 => 0,
                    2 => 1,
                    _ => 2,
                };
            }
            0x47 => {
                self.i = self.a;
                self.cyc += 1;
            }
            0x4f => {
                self.r = self.a;
                self.cyc += 1;
            }
            0x57 | 0x5f => {
                self.a = if op == 0x57 { self.i } else { self.r };
                self.f = (self.f & C) | SZXY[self.a as usize] | if self.iff2 { P } else { 0 };
                self.cyc += 1;
            }
            0x67 => {
                let addr = self.xy[HL];
                let t = b.read(addr);
                b.write(addr, (self.a << 4) | (t >> 4));
                self.a = (self.a & 0xf0) | (t & 0x0f);
                self.f = (self.f & C) | SZXYP[self.a as usize];
                self.wz = addr.wrapping_add(1);
                self.cyc += 10;
            }
            0x6f => {
                let addr = self.xy[HL];
                let t = b.read(addr);
                b.write(addr, (t << 4) | (self.a & 0x0f));
                self.a = (self.a & 0xf0) | (t >> 4);
                self.f = (self.f & C) | SZXYP[self.a as usize];
                self.wz = addr.wrapping_add(1);
                self.cyc += 10;
            }
            0xa0 | 0xa8 | 0xb0 | 0xb8 => self.block_ld(b, op),
            0xa1 | 0xa9 | 0xb1 | 0xb9 => self.block_cp(b, op),
            0xa2 | 0xaa | 0xb2 | 0xba => self.block_in(b, op),
            0xa3 | 0xab | 0xb3 | 0xbb => self.block_out(b, op),
            // Everything else is an 8 T-state NOP.
            _ => {}
        }
    }

    /// Step for a block instruction: +1 (increment) or -1 (decrement).
    #[inline]
    fn block_dir(op: u8) -> u16 {
        if op & 0x08 == 0 {
            1
        } else {
            0xffff
        }
    }

    /// Back up over a repeating block instruction: it runs again.
    #[inline]
    fn block_repeat(&mut self) {
        self.pc = self.pc.wrapping_sub(2);
        self.wz = self.pc.wrapping_add(1);
        self.cyc += 5;
    }

    fn block_ld<B: Bus>(&mut self, b: &mut B, op: u8) {
        let d = Self::block_dir(op);
        let v = b.read(self.xy[HL]);
        b.write(self.de, v);
        self.xy[HL] = self.xy[HL].wrapping_add(d);
        self.de = self.de.wrapping_add(d);
        self.bc = self.bc.wrapping_sub(1);
        let n = v.wrapping_add(self.a);
        self.f = (self.f & (S | Z | C)) | (n & X) | ((n << 4) & Y) | if self.bc != 0 { P } else { 0 };
        self.cyc += 8;
        if op & 0x10 != 0 && self.bc != 0 {
            self.block_repeat();
        }
    }

    fn block_cp<B: Bus>(&mut self, b: &mut B, op: u8) {
        let d = Self::block_dir(op);
        let v = b.read(self.xy[HL]);
        let r = self.a.wrapping_sub(v);
        let h = (self.a ^ v ^ r) & H;
        let n = r.wrapping_sub(h >> 4);
        self.xy[HL] = self.xy[HL].wrapping_add(d);
        self.bc = self.bc.wrapping_sub(1);
        self.wz = self.wz.wrapping_add(d);
        self.f = (self.f & C)
            | N
            | (SZXY[r as usize] & (S | Z))
            | h
            | (n & X)
            | ((n << 4) & Y)
            | if self.bc != 0 { P } else { 0 };
        self.cyc += 8;
        if op & 0x10 != 0 && self.bc != 0 && r != 0 {
            self.block_repeat();
        }
    }

    /// Flags shared by the block I/O instructions: `v` the byte moved, `k`
    /// the sum the carry and parity come from.
    #[inline]
    fn block_io_flags(&mut self, v: u8, k: u16) {
        let bb = (self.bc >> 8) as u8;
        self.f = SZXY[bb as usize]
            | if v & 0x80 != 0 { N } else { 0 }
            | if k > 0xff { H | C } else { 0 }
            | (SZXYP[((k as u8) & 7 ^ bb) as usize] & P);
    }

    fn block_in<B: Bus>(&mut self, b: &mut B, op: u8) {
        let d = Self::block_dir(op);
        let v = b.input(self.bc);
        b.write(self.xy[HL], v);
        self.wz = self.bc.wrapping_add(d);
        self.bc = self.bc.wrapping_sub(0x100);
        self.xy[HL] = self.xy[HL].wrapping_add(d);
        let k = v as u16 + ((self.bc as u8).wrapping_add(d as u8)) as u16;
        self.block_io_flags(v, k);
        self.cyc += 8;
        if op & 0x10 != 0 && self.bc >> 8 != 0 {
            self.block_repeat();
        }
    }

    fn block_out<B: Bus>(&mut self, b: &mut B, op: u8) {
        let d = Self::block_dir(op);
        let v = b.read(self.xy[HL]);
        self.bc = self.bc.wrapping_sub(0x100);
        b.output(self.bc, v);
        self.wz = self.bc.wrapping_add(d);
        self.xy[HL] = self.xy[HL].wrapping_add(d);
        let k = v as u16 + (self.xy[HL] as u8) as u16;
        self.block_io_flags(v, k);
        self.cyc += 8;
        if op & 0x10 != 0 && self.bc >> 8 != 0 {
            self.block_repeat();
        }
    }
}

state::fields!(Cpu {
    a,
    f,
    bc,
    de,
    xy,
    sp,
    pc,
    af_alt,
    bc_alt,
    de_alt,
    hl_alt,
    i,
    r,
    iff1,
    iff2,
    im,
    wz,
    halted,
    ei_delay,
    int_line,
    nmi_line,
    nmi_pending
});

#[cfg(test)]
mod tests;
