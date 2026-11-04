//! Reference interpreter for the i960KB.
//!
//! Correctness over speed: this is the oracle the recompiler is checked
//! against, and the tracer that tells the analyzer which code really runs.
//!
//! Simplifications, chosen so observable behaviour matches what software can
//! see:
//! - The local-register cache is write-through with zero entries: every call
//!   spills the caller's r0-r15 to its frame, every return reloads them. Code
//!   can only tell the difference by reading a frame without `flushreg`.
//! - fp0-fp3 hold `f64` rather than 80-bit extended values.
//! - Faults stop execution with `Stop::Fault` instead of entering the fault
//!   table; the handlers in this ROM only print a message and halt.

use crate::{decode, Flow, Insn, MemRef, Operand};
use std::rc::Rc;

/// Memory and I/O as seen by the CPU.
pub trait Bus {
    fn read8(&mut self, addr: u32) -> u8;
    fn read16(&mut self, addr: u32) -> u16;
    fn read32(&mut self, addr: u32) -> u32;
    fn write8(&mut self, addr: u32, v: u8);
    fn write16(&mut self, addr: u32, v: u16);
    fn write32(&mut self, addr: u32, v: u32);
    /// Side-effect-free instruction fetch. `None` for unmapped/non-code.
    fn fetch(&self, addr: u32) -> Option<u32>;
    /// True if `addr` is read-only memory, so decoded instructions there
    /// can be cached.
    fn is_rom(&self, _addr: u32) -> bool {
        false
    }
}

/// Decoded-instruction cache covers the low 2 MB (Model 2 program ROM).
const ICACHE_BYTES: u32 = 0x20_0000;

/// IAC (inter-agent communication) port: `synmovq` here messages the CPU.
pub const IAC_PORT: u32 = 0xff00_0010;
/// Interrupt control register: one vector byte per external line IRQ0-3.
pub const ICR_PORT: u32 = 0xff00_0004;
const ICR_RESET: u32 = 0xff00_0000;

// Register indices: r0-r15 are 0-15, g0-g15 are 16-31.
pub const PFP: usize = 0;
pub const SP: usize = 1;
pub const RIP: usize = 2;
pub const G14: usize = 30;
pub const FP: usize = 31;

// Process controls.
const PC_TRACE: u32 = 1 << 0;
const PC_SUPERVISOR: u32 = 1 << 1;
const PC_INTERRUPTED: u32 = 1 << 13;
const PC_PRIORITY_SHIFT: u32 = 16;
const PC_PRIORITY_MASK: u32 = 0x1f << PC_PRIORITY_SHIFT;
const PC_RESET: u32 = PC_PRIORITY_MASK | PC_INTERRUPTED | PC_SUPERVISOR;

// Arithmetic controls: condition code lives in bits 0-2.
const CC_GREATER: u32 = 0b001;
const CC_EQUAL: u32 = 0b010;
const CC_LESS: u32 = 0b100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultKind {
    Operation,
    ZeroDivide,
    Constraint,
    Unimplemented(&'static str),
    /// A `fault*` instruction whose condition held.
    Conditional,
    /// `ret` with a return type this model doesn't handle.
    BadReturn(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    Fault { kind: FaultKind, ip: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Real {
    Single,
    Double,
}

pub struct Cpu {
    pub r: [u32; 32],
    pub fpr: [f64; 4],
    pub ip: u32,
    pub ac: u32,
    pub pc: u32,
    pub tc: u32,
    pub sat: u32,
    pub prcb: u32,
    pub icr: u32,
    /// Pending interrupt vectors, one bit each.
    pending: [u32; 8],
    pub icount: u64,
    /// IAC messages other than the ones handled here, for diagnostics.
    pub unhandled_iac: Vec<[u32; 4]>,
    icache: Vec<Option<Rc<Insn>>>,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    pub fn new() -> Self {
        Cpu {
            r: [0; 32],
            fpr: [0.0; 4],
            ip: 0,
            ac: 0,
            pc: PC_RESET,
            tc: 0,
            sat: 0,
            prcb: 0,
            icr: ICR_RESET,
            pending: [0; 8],
            icount: 0,
            unhandled_iac: Vec::new(),
            icache: Vec::new(),
        }
    }

    /// Hardware reset: SAT at 0, PRCB pointer at 4, first IP at 12.
    pub fn reset(&mut self, bus: &mut impl Bus) {
        let sat = 0;
        let prcb = bus.read32(4);
        let ip = bus.read32(12);
        self.reinit(bus, sat, prcb, ip);
    }

    /// Initialise from a SAT/PRCB pair, as on reset or an IAC reinit.
    /// The first frame sits at the interrupt stack (PRCB+24).
    fn reinit(&mut self, bus: &mut impl Bus, sat: u32, prcb: u32, ip: u32) {
        self.sat = sat;
        self.prcb = prcb;
        self.ip = ip;
        self.pc = PC_RESET;
        self.ac = 0;
        self.r[FP] = bus.read32(prcb + 24);
        self.r[SP] = self.r[FP] + 64;
        self.r[PFP] = 0;
    }

    pub fn priority(&self) -> u32 {
        (self.pc & PC_PRIORITY_MASK) >> PC_PRIORITY_SHIFT
    }

    fn cc(&self) -> u32 {
        self.ac & 7
    }

    fn set_cc(&mut self, cc: u32) {
        self.ac = (self.ac & !7) | cc;
    }

    /// Raise interrupt `vector` (8-255). Its priority is `vector / 8`.
    pub fn post_interrupt(&mut self, vector: u8) {
        self.pending[(vector >> 5) as usize] |= 1 << (vector & 31);
    }

    /// Assert external interrupt line 0-3; the ICR picks its vector.
    /// A line programmed with vector 0 is ignored.
    pub fn raise_line(&mut self, line: u32) {
        let vector = (self.icr >> (8 * (line & 3))) as u8;
        if vector >= 8 {
            self.post_interrupt(vector);
        }
    }

    pub fn has_pending(&self) -> bool {
        self.pending.iter().any(|&w| w != 0)
    }

    fn highest_pending(&self) -> Option<u8> {
        (0..8)
            .rev()
            .find(|&i| self.pending[i] != 0)
            .map(|i| (i as u32 * 32 + 31 - self.pending[i].leading_zeros()) as u8)
    }

    fn check_interrupts(&mut self, bus: &mut impl Bus) {
        let Some(v) = self.highest_pending() else { return };
        let level = (v >> 3) as u32;
        if level == 31 || level > self.priority() {
            self.pending[(v >> 5) as usize] &= !(1 << (v & 31));
            self.take_interrupt(bus, v, level);
        }
    }

    fn take_interrupt(&mut self, bus: &mut impl Bus, vector: u8, level: u32) {
        let table = bus.read32(self.prcb + 20);
        let handler = bus.read32(table + 36 + (vector as u32 - 8) * 4);
        // Not already interrupted: switch to the interrupt stack.
        let sp = if self.pc & PC_INTERRUPTED == 0 { bus.read32(self.prcb + 24) } else { self.r[SP] };
        // Room for the 16-byte interrupt record just below the new frame.
        let fp = ((sp + 63) & !63) + 64;
        self.push_frame(bus, self.ip, fp, 7);
        bus.write32(fp - 16, self.pc);
        bus.write32(fp - 12, self.ac);
        bus.write32(fp - 8, vector as u32);
        self.pc =
            (self.pc & !(PC_PRIORITY_MASK | PC_TRACE)) | (level << PC_PRIORITY_SHIFT) | PC_INTERRUPTED | PC_SUPERVISOR;
        self.ip = handler;
    }

    /// Spill the current frame and open a new one at `fp`.
    fn push_frame(&mut self, bus: &mut impl Bus, ret: u32, fp: u32, ret_type: u32) {
        self.r[RIP] = ret;
        let old = self.r[FP];
        for i in 0..16 {
            bus.write32(old + 4 * i as u32, self.r[i]);
        }
        self.r[PFP] = old | ret_type;
        self.r[FP] = fp;
        self.r[SP] = fp + 64;
    }

    fn call(&mut self, bus: &mut impl Bus, target: u32, ret: u32) {
        let fp = (self.r[SP] + 63) & !63;
        self.push_frame(bus, ret, fp, 0);
        self.ip = target;
    }

    fn calls(&mut self, bus: &mut impl Bus, n: u32, ret: u32) -> Result<(), FaultKind> {
        if n > 259 {
            return Err(FaultKind::Constraint);
        }
        let table = bus.read32(self.sat + 152);
        let entry = bus.read32(table + 48 + 4 * n);
        let target = entry & !3;
        if entry & 3 == 2 && self.pc & PC_SUPERVISOR == 0 {
            let fp = (bus.read32(table + 12) + 63) & !63;
            let ret_type = 2 | (self.pc & PC_TRACE);
            self.push_frame(bus, ret, fp, ret_type);
            self.pc |= PC_SUPERVISOR;
        } else {
            let fp = (self.r[SP] + 63) & !63;
            self.push_frame(bus, ret, fp, 0);
        }
        self.ip = target;
        Ok(())
    }

    fn ret(&mut self, bus: &mut impl Bus) -> Result<(), FaultKind> {
        let ret_type = self.r[PFP] & 7;
        let frame = self.r[FP];
        let restore = |cpu: &mut Self, bus: &mut dyn FnMut(u32) -> u32| {
            cpu.r[FP] = cpu.r[PFP] & !63;
            for i in 0..16 {
                cpu.r[i] = bus(cpu.r[FP] + 4 * i as u32);
            }
            cpu.ip = cpu.r[RIP];
        };
        match ret_type {
            0 => restore(self, &mut |a| bus.read32(a)),
            1 | 7 => {
                let pc = bus.read32(frame - 16);
                let ac = bus.read32(frame - 12);
                restore(self, &mut |a| bus.read32(a));
                self.ac = ac;
                // Only supervisor code may restore the full process controls.
                if self.pc & PC_SUPERVISOR != 0 {
                    self.pc = pc;
                }
                self.check_interrupts(bus);
            }
            2 | 3 => {
                restore(self, &mut |a| bus.read32(a));
                if self.pc & PC_SUPERVISOR != 0 {
                    self.pc = (self.pc & !(PC_SUPERVISOR | PC_TRACE)) | (ret_type & 1);
                }
            }
            t => return Err(FaultKind::BadReturn(t)),
        }
        Ok(())
    }

    fn get(&self, op: &Operand) -> u32 {
        match *op {
            Operand::Reg(r) => self.r[r as usize],
            Operand::Lit(v) => v,
            _ => 0,
        }
    }

    fn reg_index(op: &Operand) -> usize {
        match *op {
            Operand::Reg(r) => r as usize,
            _ => unreachable!("destination must be a register"),
        }
    }

    fn set(&mut self, op: &Operand, v: u32) {
        self.r[Self::reg_index(op)] = v;
    }

    fn ea(&self, m: &MemRef) -> u32 {
        let r = |i: u8| self.r[i as usize];
        match *m {
            MemRef::Abs(a) => a,
            MemRef::Base { abase, disp } => r(abase).wrapping_add(disp),
            MemRef::IpRel { target } => target,
            MemRef::Index { disp, index, scale } => disp.wrapping_add(r(index).wrapping_mul(scale as u32)),
            MemRef::BaseIndex { abase, disp, index, scale } => {
                r(abase).wrapping_add(disp).wrapping_add(r(index).wrapping_mul(scale as u32))
            }
        }
    }

    fn get_real(&self, op: &Operand, kind: Real) -> f64 {
        match *op {
            Operand::FpReg(i) => self.fpr[i as usize],
            Operand::FpLit(s) => {
                if s == "1.0" {
                    1.0
                } else {
                    0.0
                }
            }
            Operand::Reg(r) => {
                let r = r as usize;
                match kind {
                    Real::Single => f32::from_bits(self.r[r]) as f64,
                    Real::Double => f64::from_bits(((self.r[(r + 1) & 31] as u64) << 32) | self.r[r] as u64),
                }
            }
            _ => 0.0,
        }
    }

    fn set_real(&mut self, op: &Operand, v: f64, kind: Real) {
        match *op {
            Operand::FpReg(i) => self.fpr[i as usize] = v,
            Operand::Reg(r) => {
                let r = r as usize;
                match kind {
                    Real::Single => self.r[r] = (v as f32).to_bits(),
                    Real::Double => {
                        let b = v.to_bits();
                        self.r[r] = b as u32;
                        self.r[(r + 1) & 31] = (b >> 32) as u32;
                    }
                }
            }
            _ => {}
        }
    }

    fn compare<T: PartialOrd>(&mut self, a: T, b: T) {
        self.set_cc(if a < b {
            CC_LESS
        } else if a == b {
            CC_EQUAL
        } else if a > b {
            CC_GREATER
        } else {
            0
        });
    }

    /// Branch condition for the low three opcode bits (`bno` is mask 0).
    fn cond(&self, mask: u32) -> bool {
        if mask == 0 {
            self.cc() == 0
        } else {
            self.cc() & mask != 0
        }
    }

    pub fn decode_at(&self, bus: &impl Bus) -> Insn {
        decode(self.ip, |a| bus.fetch(a))
    }

    fn fetch_insn(&mut self, bus: &impl Bus) -> Rc<Insn> {
        let ip = self.ip;
        if ip < ICACHE_BYTES && ip & 3 == 0 && bus.is_rom(ip) {
            if self.icache.is_empty() {
                self.icache = vec![None; (ICACHE_BYTES / 4) as usize];
            }
            let slot = &mut self.icache[(ip / 4) as usize];
            return slot.get_or_insert_with(|| Rc::new(decode(ip, |a| bus.fetch(a)))).clone();
        }
        Rc::new(self.decode_at(bus))
    }

    /// Execute one instruction (taking any interrupt that's due first).
    /// Returns the executed instruction's address and control flow.
    pub fn step(&mut self, bus: &mut impl Bus) -> Result<(u32, Flow), Stop> {
        if self.has_pending() {
            self.check_interrupts(bus);
        }
        let insn = self.fetch_insn(bus);
        let ip = self.ip;
        self.exec(bus, &insn).map_err(|kind| Stop::Fault { kind, ip })?;
        self.icount += 1;
        Ok((ip, insn.flow))
    }

    fn exec(&mut self, bus: &mut impl Bus, insn: &Insn) -> Result<(), FaultKind> {
        let next = insn.next();
        let ops = &insn.operands;
        let op = insn.word >> 24;
        self.ip = next;

        if insn.flow == Flow::Invalid {
            return Err(FaultKind::Operation);
        }

        match op {
            0x00..=0x1f => match (op, insn.flow) {
                (_, Flow::Jump(t)) => self.ip = t,
                (_, Flow::Call(t)) => self.call(bus, t, next),
                (_, Flow::Link(t)) => {
                    self.r[G14] = next;
                    self.ip = t;
                }
                (_, Flow::Return) => self.ret(bus)?,
                (_, Flow::Branch(t)) => {
                    if self.cond(op & 7) {
                        self.ip = t;
                    }
                }
                (_, Flow::CondFault) => {
                    if self.cond(op & 7) {
                        return Err(FaultKind::Conditional);
                    }
                }
                _ => return Err(FaultKind::Operation),
            },
            0x20..=0x27 => {
                let v = self.cond(op & 7) as u32;
                self.set(&ops[0], v);
            }
            0x30..=0x3f => {
                let Flow::Branch(t) = insn.flow else { return Err(FaultKind::Operation) };
                let src1 = self.get(&ops[0]);
                let src2 = self.get(&ops[1]);
                let take = match op {
                    0x30 | 0x37 => {
                        let set = src2 & (1 << (src1 & 31)) != 0;
                        self.set_cc(if set { CC_EQUAL } else { 0 });
                        set == (op == 0x37)
                    }
                    0x31..=0x36 => {
                        self.compare(src1, src2);
                        self.cond(op & 7)
                    }
                    _ => {
                        self.compare(src1 as i32, src2 as i32);
                        self.cond(op & 7)
                    }
                };
                if take {
                    self.ip = t;
                }
            }
            0x58..=0x7f => self.exec_reg(bus, insn, next)?,
            0x80..=0xcf => self.exec_mem(bus, insn, next)?,
            _ => return Err(FaultKind::Operation),
        }
        Ok(())
    }

    fn exec_reg(&mut self, bus: &mut impl Bus, insn: &Insn, next: u32) -> Result<(), FaultKind> {
        let full = ((insn.word >> 24) << 4) | ((insn.word >> 7) & 0xf);
        let ops = &insn.operands;
        let s1 = || self.get(&ops[0]);
        let s2 = || self.get(&ops[1]);
        let bit = |v: u32| 1u32 << (v & 31);
        macro_rules! dst3 {
            ($v:expr) => {{
                let v = $v;
                self.set(&ops[2], v)
            }};
        }

        match full {
            0x580 => dst3!(s2() ^ bit(s1())),
            0x581 => dst3!(s2() & s1()),
            0x582 => dst3!(s2() & !s1()),
            0x583 => dst3!(s2() | bit(s1())),
            0x584 => dst3!(!s2() & s1()),
            0x586 => dst3!(s2() ^ s1()),
            0x587 => dst3!(s2() | s1()),
            0x588 => dst3!(!(s2() | s1())),
            0x589 => dst3!(!(s2() ^ s1())),
            0x58a => {
                let v = !s1();
                self.set(&ops[1], v)
            }
            0x58b => dst3!(s2() | !s1()),
            0x58c => dst3!(s2() & !bit(s1())),
            0x58d => dst3!(!s2() | s1()),
            0x58e => dst3!(!(s2() & s1())),
            0x58f => {
                let v = if self.cc() & CC_EQUAL != 0 { s2() | bit(s1()) } else { s2() & !bit(s1()) };
                dst3!(v)
            }
            0x590 | 0x591 => dst3!(s2().wrapping_add(s1())),
            0x592 | 0x593 => dst3!(s2().wrapping_sub(s1())),
            0x598 => dst3!(if s1() >= 32 { 0 } else { s2() >> s1() }),
            0x59a => {
                // Shift right "dividing": rounds toward zero for negatives.
                let (n, v) = (s1().min(32), s2() as i32);
                let r = if n >= 32 {
                    0
                } else {
                    let q = v >> n;
                    if v < 0 && (v & ((1i64 << n) - 1) as i32) != 0 {
                        q + 1
                    } else {
                        q
                    }
                };
                dst3!(r as u32)
            }
            0x59b => dst3!(((s2() as i32) >> s1().min(31)) as u32),
            0x59c | 0x59e => dst3!(if s1() >= 32 { 0 } else { s2() << s1() }),
            0x59d => dst3!(s2().rotate_left(s1() & 31)),
            0x5a0 => self.compare(s1(), s2()),
            0x5a1 => self.compare(s1() as i32, s2() as i32),
            0x5a2 | 0x5a3 => {
                if self.cc() & CC_LESS == 0 {
                    let le = if full == 0x5a2 { s1() <= s2() } else { (s1() as i32) <= (s2() as i32) };
                    self.set_cc(if le { CC_EQUAL } else { CC_GREATER });
                }
            }
            0x5a4..=0x5a7 => {
                let (a, b) = (s1(), s2());
                if full & 1 == 0 {
                    self.compare(a, b)
                } else {
                    self.compare(a as i32, b as i32)
                }
                dst3!(if full < 0x5a6 { b.wrapping_add(1) } else { b.wrapping_sub(1) })
            }
            0x5ac => {
                let (a, b) = (s1(), s2());
                let any = (0..4).any(|i| (a >> (i * 8)) & 0xff == (b >> (i * 8)) & 0xff);
                self.set_cc(if any { CC_EQUAL } else { 0 });
            }
            0x5ae => {
                let set = s2() & bit(s1()) != 0;
                self.set_cc(if set { CC_EQUAL } else { 0 });
            }
            0x5b0 | 0x5b2 => {
                let carry_in = (self.cc() >> 1) & 1;
                let (a, b) = (s1() as u64, s2() as u64);
                let r = if full == 0x5b0 { a + b + carry_in as u64 } else { b + (!a & 0xffff_ffff) + carry_in as u64 };
                let res = r as u32;
                let (sa, sb) = if full == 0x5b0 { (a as u32, b as u32) } else { (!(a as u32), b as u32) };
                let overflow = ((sa ^ res) & (sb ^ res)) >> 31;
                self.set_cc((((r >> 32) as u32 & 1) << 1) | overflow);
                dst3!(res)
            }
            0x5cc | 0x5dc | 0x5ec | 0x5fc => {
                let n = match full {
                    0x5cc => 1,
                    0x5dc => 2,
                    0x5ec => 3,
                    _ => 4,
                };
                let d = Self::reg_index(&ops[1]);
                match ops[0] {
                    Operand::Reg(s) => {
                        let vals: Vec<u32> = (0..n).map(|i| self.r[(s as usize + i) & 31]).collect();
                        for (i, v) in vals.into_iter().enumerate() {
                            self.r[(d + i) & 31] = v;
                        }
                    }
                    Operand::Lit(v) => {
                        self.r[d] = v;
                        for i in 1..n {
                            self.r[(d + i) & 31] = 0;
                        }
                    }
                    _ => {}
                }
            }
            0x600..=0x602 => {
                let (dst, src) = (s1(), s2());
                let words = [1, 2, 4][(full - 0x600) as usize];
                let msg: Vec<u32> = (0..words).map(|i| bus.read32(src + 4 * i)).collect();
                if dst == IAC_PORT && words == 4 {
                    self.iac(bus, [msg[0], msg[1], msg[2], msg[3]]);
                } else if dst == ICR_PORT && words == 1 {
                    self.icr = msg[0];
                } else {
                    for (i, w) in msg.into_iter().enumerate() {
                        bus.write32(dst + 4 * i as u32, w);
                    }
                }
                self.set_cc(CC_EQUAL);
            }
            0x610 | 0x612 => {
                let addr = s1() & !3;
                let old = bus.read32(addr);
                let d = Self::reg_index(&ops[2]);
                let new = if full == 0x612 { old.wrapping_add(s2()) } else { (self.r[d] & s2()) | (old & !s2()) };
                bus.write32(addr, new);
                self.r[d] = old;
            }
            0x615 => {
                let v = bus.read32(s1() & !3);
                self.set(&ops[1], v);
                self.set_cc(CC_EQUAL);
            }
            0x640 | 0x641 => {
                let v = if full == 0x641 { s1() } else { !s1() };
                if v == 0 {
                    self.set(&ops[1], u32::MAX);
                    self.set_cc(0);
                } else {
                    self.set(&ops[1], 31 - v.leading_zeros());
                    self.set_cc(CC_EQUAL);
                }
            }
            0x645 => {
                let (mask, src) = (s1(), s2());
                let old = self.ac;
                self.ac = (src & mask) | (old & !mask);
                dst3!(old)
            }
            0x650 => {
                let (mask, src) = (s1(), s2());
                let d = Self::reg_index(&ops[2]);
                self.r[d] = (src & mask) | (self.r[d] & !mask);
            }
            0x651 => {
                let (pos, len) = (s1() & 31, s2());
                let d = Self::reg_index(&ops[2]);
                let mask = if len >= 32 { u32::MAX } else { (1u32 << len) - 1 };
                self.r[d] = (self.r[d] >> pos) & mask;
            }
            0x654 => {
                let (mask, src) = (s1(), s2());
                let old = self.tc;
                self.tc = (src & mask) | (old & !mask);
                dst3!(old)
            }
            0x655 => {
                let mask = s2();
                let d = Self::reg_index(&ops[2]);
                let old = self.pc;
                if mask != 0 {
                    if self.pc & PC_SUPERVISOR == 0 {
                        return Err(FaultKind::Unimplemented("modpc in user mode"));
                    }
                    self.pc = (self.r[d] & mask) | (old & !mask);
                }
                self.r[d] = old;
                self.check_interrupts(bus);
            }
            0x660 => {
                let n = s1();
                self.calls(bus, n, next)?
            }
            0x66b | 0x66c | 0x66d | 0x66f => {}
            0x670 => {
                let v = s1() as u64 * s2() as u64;
                let d = Self::reg_index(&ops[2]);
                self.r[d] = v as u32;
                self.r[(d + 1) & 31] = (v >> 32) as u32;
            }
            0x671 => {
                let divisor = s1() as u64;
                if divisor == 0 {
                    return Err(FaultKind::ZeroDivide);
                }
                let lo = match ops[1] {
                    Operand::Reg(r) => r as usize,
                    _ => return Err(FaultKind::Operation),
                };
                let dividend = ((self.r[(lo + 1) & 31] as u64) << 32) | self.r[lo] as u64;
                let d = Self::reg_index(&ops[2]);
                self.r[d] = (dividend % divisor) as u32;
                self.r[(d + 1) & 31] = (dividend / divisor) as u32;
            }
            0x701 | 0x741 => dst3!(s2().wrapping_mul(s1())),
            0x708 | 0x70b => {
                let (a, b) = (s1(), s2());
                if a == 0 {
                    return Err(FaultKind::ZeroDivide);
                }
                dst3!(if full == 0x70b { b / a } else { b % a })
            }
            0x748 | 0x749 | 0x74b => {
                let (a, b) = (s1() as i32, s2() as i32);
                if a == 0 {
                    return Err(FaultKind::ZeroDivide);
                }
                let v = match full {
                    0x74b => b.wrapping_div(a),
                    0x748 => b.wrapping_rem(a),
                    _ => {
                        // modi: result takes the divisor's sign.
                        let r = b.wrapping_rem(a);
                        if r != 0 && ((r < 0) != (a < 0)) {
                            r + a
                        } else {
                            r
                        }
                    }
                };
                dst3!(v as u32)
            }
            0x674..=0x6e3 | 0x78b..=0x79f => self.exec_float(insn, full)?,
            _ => return Err(FaultKind::Unimplemented(insn.mnemonic)),
        }
        Ok(())
    }

    fn exec_float(&mut self, insn: &Insn, full: u32) -> Result<(), FaultKind> {
        use Real::*;
        let ops = &insn.operands;
        let k = if (full >> 4) & 1 == 1 && full < 0x6c0 || matches!(full, 0x79b..=0x79f | 0x6d9 | 0x676) {
            Double
        } else {
            Single
        };
        let r1 = |c: &Self| c.get_real(&ops[0], k);
        let r2 = |c: &Self| c.get_real(&ops[1], k);
        match full {
            0x674 => {
                let v = self.get(&ops[0]) as i32 as f64;
                self.set_real(&ops[1], v, Single)
            }
            0x675 => {
                let lo = Self::reg_index(&ops[0]);
                let v = (((self.r[(lo + 1) & 31] as u64) << 32) | self.r[lo] as u64) as i64 as f64;
                self.set_real(&ops[1], v, Single)
            }
            0x676 | 0x677 => {
                let n = self.get(&ops[0]) as i32;
                let v = self.get_real(&ops[1], k) * 2f64.powi(n);
                self.set_real(&ops[2], v, k)
            }
            0x684 | 0x685 | 0x694 | 0x695 => {
                let (a, b) = (r1(self), r2(self));
                self.compare(a, b)
            }
            0x68f | 0x69f => return Err(FaultKind::Unimplemented(insn.mnemonic)),
            0x680..=0x69e => {
                let base = full & !0x10;
                let unary = matches!(base, 0x688..=0x68e);
                let a = r1(self);
                let v = if unary {
                    match base {
                        0x688 => a.sqrt(),
                        0x689 => a.exp2() - 1.0,
                        0x68a => a.abs().log2().floor(),
                        0x68b => a.round_ties_even(),
                        0x68c => a.sin(),
                        0x68d => a.cos(),
                        _ => a.tan(),
                    }
                } else {
                    let b = r2(self);
                    match base {
                        0x680 => b.atan2(a),
                        0x681 => b * (a + 1.0).log2(),
                        0x682 => b * a.log2(),
                        _ => b % a,
                    }
                };
                let dst = if unary { &ops[1] } else { &ops[2] };
                self.set_real(dst, v, k)
            }
            0x6c0..=0x6c3 => {
                let v = self.get_real(&ops[0], Single);
                let v = if full & 2 == 0 { v.round_ties_even() } else { v.trunc() };
                let d = Self::reg_index(&ops[1]);
                if full & 1 == 0 {
                    self.r[d] = v as i32 as u32;
                } else {
                    let w = v as i64 as u64;
                    self.r[d] = w as u32;
                    self.r[(d + 1) & 31] = (w >> 32) as u32;
                }
            }
            0x6c9 | 0x6d9 => {
                let v = r1(self);
                self.set_real(&ops[1], v, k)
            }
            0x6e1 => {
                let v = match ops[0] {
                    Operand::Reg(r) => {
                        let r = r as usize;
                        ext80_to_f64(self.r[r], self.r[(r + 1) & 31], self.r[(r + 2) & 31])
                    }
                    _ => self.get_real(&ops[0], Double),
                };
                match ops[1] {
                    Operand::Reg(r) => {
                        let (a, b, c) = f64_to_ext80(v);
                        let r = r as usize;
                        self.r[r] = a;
                        self.r[(r + 1) & 31] = b;
                        self.r[(r + 2) & 31] = c;
                    }
                    _ => self.set_real(&ops[1], v, Double),
                }
            }
            0x6e2 | 0x6e3 => {
                let (a, b) = (self.get_real(&ops[0], Double), self.get_real(&ops[1], Double));
                let sign_neg = if full == 0x6e2 { b.is_sign_negative() } else { !b.is_sign_negative() };
                let v = if sign_neg { -a.abs() } else { a.abs() };
                self.set_real(&ops[2], v, Double)
            }
            0x78b..=0x79f => {
                let (a, b) = (r1(self), r2(self));
                let v = match full & 0xf {
                    0xb => b / a,
                    0xc => b * a,
                    0xd => b - a,
                    _ => b + a,
                };
                self.set_real(&ops[2], v, k)
            }
            _ => return Err(FaultKind::Unimplemented(insn.mnemonic)),
        }
        Ok(())
    }

    fn exec_mem(&mut self, bus: &mut impl Bus, insn: &Insn, next: u32) -> Result<(), FaultKind> {
        let op = insn.word >> 24;
        let ops = &insn.operands;
        let (Some(Operand::Mem(m)), reg) = (ops.iter().find(|o| matches!(o, Operand::Mem(_))), ops.first()) else {
            return Err(FaultKind::Operation);
        };
        let ea = self.ea(m);
        let ri = |o: Option<&Operand>| match o {
            Some(Operand::Reg(r)) => *r as usize,
            _ => 0,
        };
        match op {
            0x84 => self.ip = ea,
            0x85 => {
                self.set(&ops[1], next);
                self.ip = ea;
            }
            0x86 => self.call(bus, ea, next),
            0x8c => self.set(&ops[1], ea),
            0x80 => self.r[ri(ops.get(1))] = bus.read8(ea) as u32,
            0xc0 => self.r[ri(ops.get(1))] = bus.read8(ea) as i8 as i32 as u32,
            0x88 => self.r[ri(ops.get(1))] = bus.read16(ea) as u32,
            0xc8 => self.r[ri(ops.get(1))] = bus.read16(ea) as i16 as i32 as u32,
            0x82 | 0xc2 => bus.write8(ea, self.r[ri(reg)] as u8),
            0x8a | 0xca => bus.write16(ea, self.r[ri(reg)] as u16),
            0x90 | 0x98 | 0xa0 | 0xb0 => {
                let n = match op {
                    0x90 => 1,
                    0x98 => 2,
                    0xa0 => 3,
                    _ => 4,
                };
                let d = ri(ops.get(1));
                for i in 0..n {
                    self.r[(d + i) & 31] = bus.read32(ea.wrapping_add(4 * i as u32));
                }
            }
            0x92 | 0x9a | 0xa2 | 0xb2 => {
                let n = match op {
                    0x92 => 1,
                    0x9a => 2,
                    0xa2 => 3,
                    _ => 4,
                };
                let s = ri(reg);
                for i in 0..n {
                    bus.write32(ea.wrapping_add(4 * i as u32), self.r[(s + i) & 31]);
                }
            }
            _ => return Err(FaultKind::Operation),
        }
        Ok(())
    }

    fn iac(&mut self, bus: &mut impl Bus, msg: [u32; 4]) {
        match msg[0] >> 24 {
            // Reinitialise processor: new SAT, PRCB and first IP.
            0x93 => self.reinit(bus, msg[1], msg[2], msg[3]),
            // Interrupt: vector in the low byte.
            0x40 => self.post_interrupt(msg[0] as u8),
            // Purge instruction cache, test pending interrupts.
            0x89 => {}
            0x41 => self.check_interrupts(bus),
            _ => self.unhandled_iac.push(msg),
        }
    }
}

/// 80-bit extended real (in three registers: mantissa lo, hi, sign/exp) to f64.
fn ext80_to_f64(lo: u32, hi: u32, se: u32) -> f64 {
    let mant = ((hi as u64) << 32) | lo as u64;
    let sign = if se & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = (se & 0x7fff) as i32;
    if exp == 0 && mant == 0 {
        return 0.0 * sign;
    }
    if exp == 0x7fff {
        return if mant << 1 == 0 { sign * f64::INFINITY } else { f64::NAN };
    }
    sign * (mant as f64) * 2f64.powi(exp - 16383 - 63)
}

fn f64_to_ext80(v: f64) -> (u32, u32, u32) {
    let sign = if v.is_sign_negative() { 0x8000 } else { 0 };
    if v == 0.0 {
        return (0, 0, sign);
    }
    if v.is_infinite() {
        return (0, 0x8000_0000, sign | 0x7fff);
    }
    if v.is_nan() {
        return (0, 0xc000_0000, 0x7fff);
    }
    let a = v.abs();
    let e = a.log2().floor() as i32;
    let mut mant = (a / 2f64.powi(e) * 2f64.powi(63)) as u64;
    let mut exp = e + 16383;
    if mant >> 63 == 0 {
        mant <<= 1;
        exp -= 1;
    }
    (mant as u32, (mant >> 32) as u32, sign | exp as u32)
}

#[cfg(test)]
mod tests;
