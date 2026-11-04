//! Reference interpreter for the MB86235.
//!
//! Runs the program the i960 uploads, one 64-bit word per step. Semantics
//! follow `docs/hw/mb86235.md`; where it lists a conflict, the choice here
//! is the one the uploaded topskatr program supports, or the simplest
//! reading when the program doesn't exercise it. Notable choices:
//! - Within a word: transfer source, then ALU/MUL sources are read; then
//!   ALU, MUL and transfer destination are written, in that order; POP last.
//! - Branch conditions see the flags from before the current word.
//! - Every branch, call and return has one delay slot.
//! - Reading FI while the input FIFO is empty stalls with no side effects.
//! - Floats are IEEE single; FRCP/FRSQ/FLOG are computed exactly.

use crate::{decode, AluOp, BranchTarget, Control, Insn, Loc, Mem, MulOp, Transfer, PROGRAM_WORDS};
use std::collections::VecDeque;

/// The external (24-bit word-addressed) bus.
pub trait External {
    fn read(&mut self, addr: u32) -> u32;
    fn write(&mut self, addr: u32, v: u32);
}

/// Depth reported by the FIFO-full flags. The real depth is unknown; the
/// model itself never drops data.
pub const FIFO_DEPTH: usize = 8;

// Status register bits.
const AD: u32 = 1 << 0;
const AU: u32 = 1 << 1;
const AV: u32 = 1 << 2;
const AZ: u32 = 1 << 3;
const AN: u32 = 1 << 4;
const ZC: u32 = 1 << 8;
const IL: u32 = 1 << 9;
const NR: u32 = 1 << 10;
const ZD: u32 = 1 << 11;
const RP: u32 = 1 << 14;
const LP: u32 = 1 << 15;
const MD: u32 = 1 << 16;
const MU: u32 = 1 << 17;
const MV: u32 = 1 << 18;
const MZ: u32 = 1 << 19;
const MN: u32 = 1 << 20;
const OFF: u32 = 1 << 24;
const OFE: u32 = 1 << 25;
const IFF: u32 = 1 << 26;
const IFE: u32 = 1 << 27;

const FLOAT_CONSTS: [f32; 8] = [-1.0, 0.0, 0.5, 1.0, 1.5, 2.0, 3.0, 5.0];
const INT_CONSTS: [u32; 8] = [0, 1, u32::MAX, 0, 0, 0, 0, 0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Ran,
    /// Waiting for the input FIFO.
    Stalled,
    /// An instruction this model can't execute.
    Unsupported(u32),
}

pub struct Tgp {
    pub program: Vec<u64>,
    pub aram: Vec<u32>,
    pub bram: Vec<u32>,
    pub aa: [u32; 8],
    pub ab: [u32; 8],
    pub ma: [u32; 8],
    pub mb: [u32; 8],
    pub ar: [u32; 8],
    pub pr: [u32; 24],
    pub prp: u32,
    pub pwp: u32,
    pub eb: u32,
    pub eo: u32,
    pub sp: u32,
    pub st: u32,
    pub md: u32,
    pub lpc: u32,
    pub rpc: u32,
    pub pdr: u32,
    pub ddr: u32,
    pub pc: u32,
    pcs: [u32; 4],
    pcsp: usize,
    /// Target of a delayed branch, taken after the next word.
    pending: Option<u32>,
    pub fifo_in: VecDeque<u32>,
    pub fifo_out: VecDeque<u32>,
    pub steps: u64,
    decoded: Vec<Option<Insn>>,
}

impl Default for Tgp {
    fn default() -> Self {
        Self::new()
    }
}

impl Tgp {
    pub fn new() -> Self {
        Tgp {
            program: vec![0; PROGRAM_WORDS as usize],
            aram: vec![0; 1024],
            bram: vec![0; 1024],
            aa: [0; 8],
            ab: [0; 8],
            ma: [0; 8],
            mb: [0; 8],
            ar: [0; 8],
            pr: [0; 24],
            prp: 0,
            pwp: 0,
            eb: 0,
            eo: 0,
            sp: 0,
            st: 0,
            md: 0,
            lpc: 0,
            rpc: 0,
            pdr: 0,
            ddr: 0,
            pc: 0,
            pcs: [0; 4],
            pcsp: 0,
            pending: None,
            fifo_in: VecDeque::new(),
            fifo_out: VecDeque::new(),
            steps: 0,
            decoded: vec![None; PROGRAM_WORDS as usize],
        }
    }

    /// Load a program and restart at address 0 (the i960 releasing HALT).
    pub fn boot(&mut self, program: &[u64]) {
        self.program.iter_mut().for_each(|w| *w = 0);
        self.program[..program.len().min(PROGRAM_WORDS as usize)].copy_from_slice(program);
        self.decoded.iter_mut().for_each(|d| *d = None);
        self.pc = 0;
        self.pending = None;
        self.pcsp = 0;
        self.rpc = 0;
        self.st &= !(RP | LP);
    }

    fn insn(&mut self, pc: u32) -> Insn {
        let slot = &mut self.decoded[pc as usize];
        slot.get_or_insert_with(|| decode(pc, self.program[pc as usize])).clone()
    }

    fn live_flags(&self) -> u32 {
        let mut f = self.st & !(IFE | IFF | OFE | OFF);
        if self.fifo_in.is_empty() {
            f |= IFE;
        }
        if self.fifo_in.len() >= FIFO_DEPTH {
            f |= IFF;
        }
        if self.fifo_out.is_empty() {
            f |= OFE;
        }
        if self.fifo_out.len() >= FIFO_DEPTH {
            f |= OFF;
        }
        f
    }

    fn cond(&self, c: u32) -> bool {
        const BIT: [u32; 25] =
            [20, 19, 18, 17, 11, 10, 9, 8, 4, 3, 2, 1, 16, 0, 99, 99, 28, 29, 30, 99, 26, 27, 24, 25, 99];
        match BIT.get(c as usize) {
            Some(&b) if b < 32 => self.live_flags() & (1 << b) != 0,
            _ => false,
        }
    }

    // --- registers -------------------------------------------------------

    fn reg_read(&mut self, code: u32) -> u32 {
        let i = (code & 7) as usize;
        match code & 0x3f {
            0x00..=0x07 => self.ma[i],
            0x08..=0x0f => self.aa[i],
            0x10 => self.eb,
            0x11 => self.eb >> 14,
            0x12 => self.eb & 0x3fff,
            0x13 => self.eo,
            0x14 => self.sp,
            0x15 => self.live_flags(),
            0x16 => self.md,
            0x17 => self.lpc,
            0x18..=0x1f => self.ar[i],
            0x20..=0x27 => self.mb[i],
            0x28..=0x2f => self.ab[i],
            0x30 => self.pr[self.prp as usize],
            0x31 => self.fifo_in.pop_front().unwrap_or(0),
            0x34 => self.pdr,
            0x35 => self.ddr,
            0x36 => self.prp,
            0x37 => self.pwp,
            _ => 0,
        }
    }

    fn reg_write(&mut self, code: u32, v: u32) {
        let i = (code & 7) as usize;
        match code & 0x3f {
            0x00..=0x07 => self.ma[i] = v,
            0x08..=0x0f => self.aa[i] = v,
            0x10 => self.eb = v & 0xff_ffff,
            0x11 => self.eb = ((v << 14) | (self.eb & 0x3fff)) & 0xff_ffff,
            0x12 => self.eb = (self.eb & !0x3fff) | (v & 0x3fff),
            0x13 => self.eo = v & 0xff_ffff,
            0x14 => self.sp = v & 0x3ff,
            0x15 => self.st = v,
            0x16 => self.md = v & 0xffff,
            0x17 => self.lpc = v,
            0x18..=0x1f => self.ar[i] = v & 0x3fff,
            0x20..=0x27 => self.mb[i] = v,
            0x28..=0x2f => self.ab[i] = v,
            0x30 => {
                self.pr[self.pwp as usize] = v;
                self.pwp = (self.pwp + 1) % 24;
            }
            0x32 | 0x33 => self.fifo_out.push_back(v),
            0x34 => self.pdr = v,
            0x35 => self.ddr = v,
            0x36 => self.prp = v % 24,
            0x37 => self.pwp = v % 24,
            _ => {}
        }
    }

    /// Arithmetic output code (5 bits).
    fn out_write(&mut self, code: u32, v: u32) {
        let i = (code & 7) as usize;
        match code {
            0x00..=0x07 => self.ma[i] = v,
            0x08..=0x0f => self.mb[i] = v,
            0x10..=0x17 => self.aa[i] = v,
            _ => self.ab[i] = v,
        }
    }

    fn pr_read(&mut self, code: u32) -> u32 {
        let v = self.pr[self.prp as usize];
        match code {
            0x11 => self.prp = (self.prp + 1) % 24,
            0x12 => self.prp = (self.prp + 23) % 24,
            0x13 => self.prp = 0,
            _ => {}
        }
        v
    }

    fn alu_in1(&self, v: u32) -> u32 {
        if v < 8 {
            self.aa[v as usize]
        } else {
            self.ab[(v & 7) as usize]
        }
    }

    fn mul_in1(&self, v: u32) -> u32 {
        if v < 8 {
            self.ma[v as usize]
        } else {
            self.mb[(v & 7) as usize]
        }
    }

    fn in2(&mut self, alu: bool, v: u32, float: bool) -> u32 {
        match v {
            0x00..=0x0f if alu => self.alu_in1(v),
            0x00..=0x0f => self.mul_in1(v),
            0x10..=0x13 => self.pr_read(v),
            0x18..=0x1f if float => FLOAT_CONSTS[(v - 0x18) as usize].to_bits(),
            0x18..=0x1f => INT_CONSTS[(v - 0x18) as usize],
            _ => 0,
        }
    }

    // --- memory ----------------------------------------------------------

    /// Effective address with its post-modify applied.
    fn ea(&mut self, m: &Mem) -> u32 {
        let (x, y) = (m.arx as usize, m.ary as usize);
        let d = m.disp as u32;
        let ax = self.ar[x];
        let ay = self.ar[y];
        let wrap = |v: u32| v & 0x3fff;
        let cbs = if m.b_bus { (self.md >> 8) & 7 } else { (self.md >> 12) & 7 };
        let circ = ay & (0x1ff >> (7 - cbs.min(7)));
        let (hi7, lo7) = (ay >> 7, ay & 0x7f);
        let addr = match m.mode {
            0 => ax,
            1 => {
                self.ar[x] = wrap(ax + 1);
                ax
            }
            2 => {
                self.ar[x] = wrap(ax.wrapping_sub(1));
                ax
            }
            3 => {
                self.ar[x] = wrap(ax.wrapping_add(d));
                ax
            }
            4 => ax + ay,
            5 => {
                self.ar[y] = wrap(ay + 1);
                ax + ay
            }
            6 => {
                self.ar[y] = wrap(ay.wrapping_sub(1));
                ax + ay
            }
            7 => {
                self.ar[y] = wrap(ay.wrapping_add(d));
                ax + ay
            }
            8 => ax + if m.b_bus { lo7 } else { hi7 },
            9 => ax + if m.b_bus { hi7 } else { lo7 },
            0xa => ax.wrapping_add(d),
            0xb => ax.wrapping_add(ay).wrapping_add(d),
            0xc => d,
            0xd => {
                self.ar[y] = wrap(ay + 1);
                ax + circ
            }
            0xe => {
                self.ar[y] = wrap(ay.wrapping_sub(1));
                ax + circ
            }
            _ => {
                self.ar[y] = wrap(ay.wrapping_add(d));
                ax + circ
            }
        };
        wrap(addr)
    }

    fn mem_read(&mut self, m: &Mem, ext: &mut impl External) -> u32 {
        let a = self.ea(m);
        if m.b_bus {
            self.bram[(a & 0x3ff) as usize]
        } else if a < 0x400 {
            self.aram[a as usize]
        } else {
            ext.read((self.eb & 0xff_c000) + a)
        }
    }

    fn mem_write(&mut self, m: &Mem, v: u32, ext: &mut impl External) {
        let a = self.ea(m);
        if m.b_bus {
            self.bram[(a & 0x3ff) as usize] = v;
        } else if a < 0x400 {
            self.aram[a as usize] = v;
        } else {
            ext.write((self.eb & 0xff_c000) + a, v);
        }
    }

    fn loc_read(&mut self, l: &Loc, ext: &mut impl External) -> u32 {
        match l {
            Loc::Reg(r) => self.reg_read(*r),
            Loc::Mem(m) => self.mem_read(m, ext),
            Loc::Imm(v) => *v,
            Loc::Ext { disp } => {
                let v = ext.read((self.eb + self.eo) & 0xff_ffff);
                self.eo = self.eo.wrapping_add(*disp as u32) & 0xff_ffff;
                v
            }
        }
    }

    fn loc_write(&mut self, l: &Loc, v: u32, ext: &mut impl External) {
        match l {
            Loc::Reg(r) => self.reg_write(*r, v),
            Loc::Mem(m) => self.mem_write(m, v, ext),
            Loc::Imm(_) => {}
            Loc::Ext { disp } => {
                ext.write((self.eb + self.eo) & 0xff_ffff, v);
                self.eo = self.eo.wrapping_add(*disp as u32) & 0xff_ffff;
            }
        }
    }

    // --- arithmetic ------------------------------------------------------

    fn alu(&mut self, a: &AluOp) -> Option<(u32, u32)> {
        let float = a.is_float();
        let x = self.alu_in1(a.in1);
        let y = if matches!(a.op, 0x08 | 0x09 | 0x1c..=0x1f) { a.in2 } else { self.in2(true, a.in2, float) };
        let (fx, fy) = (f32::from_bits(x), f32::from_bits(y));
        let fl = |v: f32| v.to_bits();
        let mut extra = 0;
        let clampf = |v: f32, extra: &mut u32| {
            if v < 0.0 {
                *extra |= ZC;
                0.0
            } else {
                v
            }
        };
        let clampi = |v: u32, extra: &mut u32| {
            if (v as i32) < 0 {
                *extra |= ZC;
                0
            } else {
                v
            }
        };
        let r = match a.op {
            0x00 => fl(fx + fy),
            0x01 => fl(clampf(fx + fy, &mut extra)),
            0x02 => fl(fy - fx),
            0x03 => fl(clampf(fy - fx, &mut extra)),
            0x04 => {
                self.set_alu_flags_f(fy - fx, 0);
                return None;
            }
            0x05 => fl(fx.abs()),
            0x06 => {
                self.set_alu_flags_f(fy.abs() - fx.abs(), 0);
                return None;
            }
            0x08 | 0x09 => {
                // Scale by a power of two through the exponent field.
                let exp = ((x >> 23) & 0xff) as i32;
                if exp == 0 {
                    x
                } else {
                    let e = if a.op == 0x08 { exp + y as i32 } else { exp - y as i32 };
                    let e = e.clamp(0, 255) as u32;
                    (x & !(0xff << 23)) | (e << 23)
                }
            }
            0x0a => {
                if fx == 0.0 {
                    extra |= ZD;
                }
                fl(1.0 / fx)
            }
            0x0b => {
                if fx <= 0.0 {
                    extra |= NR;
                }
                fl(1.0 / fx.sqrt())
            }
            0x0c => {
                if fx <= 0.0 {
                    extra |= IL;
                }
                fl(fx.log2())
            }
            0x0d => fl(x as i32 as f32),
            0x0e => (fx.trunc() as i32) as u32,
            0x0f => {
                if fx < 0.0 {
                    extra |= AU;
                } else if fx > 255.0 {
                    extra |= AV;
                }
                fx.clamp(0.0, 255.0) as u32
            }
            0x10 => x.wrapping_add(y),
            0x11 => clampi(x.wrapping_add(y), &mut extra),
            0x12 => y.wrapping_sub(x),
            0x13 => clampi(y.wrapping_sub(x), &mut extra),
            0x14 => {
                self.set_alu_flags_i(y.wrapping_sub(x), 0);
                return None;
            }
            0x15 => x & 0x7fff_ffff,
            0x16 => return Some((a.out, x)),
            0x17 => {
                let v = clampi(x, &mut extra);
                self.st = (self.st & !ZC) | extra;
                return Some((a.out, v));
            }
            0x18 => x & y,
            0x19 => x | y,
            0x1a => x ^ y,
            0x1b => !x,
            0x1c => x.checked_shr(y).unwrap_or(0),
            0x1d | 0x1f => x.checked_shl(y).unwrap_or(0),
            0x1e => ((x as i32) >> y.min(31)) as u32,
            _ => return None,
        };
        if float && !matches!(a.op, 0x0e | 0x0f) {
            self.set_alu_flags_f(f32::from_bits(r), extra);
        } else {
            self.set_alu_flags_i(r, extra);
        }
        Some((a.out, r))
    }

    fn set_alu_flags_f(&mut self, v: f32, extra: u32) {
        let mut f = extra;
        if v < 0.0 {
            f |= AN;
        }
        if v == 0.0 {
            f |= AZ;
        }
        self.st = (self.st & !(AN | AZ | AV | AU | AD | ZC | ZD | NR | IL)) | f;
    }

    fn set_alu_flags_i(&mut self, v: u32, extra: u32) {
        let mut f = extra;
        if (v as i32) < 0 {
            f |= AN;
        }
        if v == 0 {
            f |= AZ;
        }
        self.st = (self.st & !(AN | AZ | AV | AU | AD | ZC | ZD | NR | IL)) | f;
    }

    fn mul(&mut self, m: &MulOp) -> (u32, u32) {
        let x = self.mul_in1(m.in1);
        let y = self.in2(false, m.in2, m.float);
        let (r, neg, zero) = if m.float {
            let v = f32::from_bits(x) * f32::from_bits(y);
            (v.to_bits(), v < 0.0, v == 0.0)
        } else {
            let v = (x as i32).wrapping_mul(y as i32);
            (v as u32, v < 0, v == 0)
        };
        self.st &= !(MN | MZ | MV | MU | MD);
        if neg {
            self.st |= MN;
        }
        if zero {
            self.st |= MZ;
        }
        (m.out, r)
    }

    // --- control ---------------------------------------------------------

    fn target(&mut self, t: BranchTarget, pc: u32) -> u32 {
        let v = match t {
            BranchTarget::Imm(a) => a,
            BranchTarget::Relative(d) => pc.wrapping_add(1).wrapping_add(d as u32),
            BranchTarget::Reg(r) => self.reg_read(r),
            BranchTarget::ARam(a) => self.aram[a as usize & 0x3ff],
            BranchTarget::BRam(a) => self.bram[a as usize & 0x3ff],
            BranchTarget::ARamAr(x) => self.aram[(self.ar[x as usize] & 0x3ff) as usize],
            BranchTarget::BRamAr(x) => self.bram[(self.ar[x as usize] & 0x3ff) as usize],
        };
        v % PROGRAM_WORDS
    }

    /// Does this word read the input FIFO?
    fn reads_fifo(t: &Transfer) -> bool {
        let fi = |l: &Loc| matches!(l, Loc::Reg(0x31));
        match t {
            Transfer::Move { src, .. } => fi(src),
            Transfer::Double { a, b } => fi(&a.0) || fi(&b.0),
            Transfer::Control(Control::Push(0x31)) => true,
            _ => false,
        }
    }

    /// Execute one word.
    pub fn step(&mut self, ext: &mut impl External) -> Step {
        let pc = self.pc;
        let insn = self.insn(pc);
        if let Transfer::Unsupported(_) = insn.transfer {
            return Step::Unsupported(pc);
        }
        if Self::reads_fifo(&insn.transfer) && self.fifo_in.is_empty() {
            return Step::Stalled;
        }
        let delayed = self.pending.take();

        // Control decisions use the flags from before this word.
        let mut branch: Option<u32> = None;
        let mut pop: Option<u32> = None;
        let mut rep_set = false;
        if let Transfer::Control(c) = insn.transfer {
            match c {
                Control::Rep(n) => {
                    self.rpc = n;
                    rep_set = true;
                }
                Control::RepAr(x) => {
                    self.rpc = self.ar[x as usize];
                    rep_set = true;
                }
                Control::SetL(n) => {
                    self.lpc = n;
                    self.st |= LP;
                }
                Control::SetLAr(x) => {
                    self.lpc = self.ar[x as usize];
                    self.st |= LP;
                }
                Control::ClrF { input, output } => {
                    if input {
                        self.fifo_in.clear();
                    }
                    if output {
                        self.fifo_out.clear();
                    }
                }
                Control::Push(r) => {
                    let v = self.reg_read(r);
                    self.sp = (self.sp + 1023) & 0x3ff;
                    self.bram[self.sp as usize] = v;
                }
                Control::Pop(r) => pop = Some(r),
                Control::SetM { op, value } => match op {
                    0x08 => self.md = value,
                    0x09 => self.md = (self.md & !(7 << 12)) | ((value & 7) << 12),
                    0x0a => self.md = (self.md & !(7 << 8)) | ((value & 7) << 8),
                    0x0b => self.md = (self.md & !(1 << 7)) | ((value & 1) << 7),
                    0x0c => self.md = (self.md & !(1 << 4)) | ((value & 1) << 4),
                    _ => self.md = (self.md & !7) | (value & 7),
                },
                Control::Branch { cond, negate, target } => {
                    if self.cond(cond) != negate {
                        branch = Some(self.target(target, pc));
                    }
                }
                Control::Jump(t) => branch = Some(self.target(t, pc)),
                Control::Loop(t) => {
                    if self.st & LP != 0 {
                        branch = Some(t);
                    }
                    self.lpc = self.lpc.wrapping_sub(1);
                    if self.lpc <= 1 {
                        self.st &= !LP;
                    }
                }
                Control::BitBranch { set, arx, bit, target } => {
                    if (self.ar[arx as usize] >> bit) & 1 == set as u32 {
                        branch = Some(target);
                    }
                }
                Control::Call { cond, target } => {
                    let take = cond.is_none_or(|(c, n)| self.cond(c) != n);
                    if take {
                        let t = self.target(target, pc);
                        self.pcs[self.pcsp & 3] = (pc + 2) % PROGRAM_WORDS;
                        self.pcsp = (self.pcsp + 1) & 3;
                        branch = Some(t);
                    }
                }
                Control::Ret => {
                    self.pcsp = (self.pcsp + 3) & 3;
                    branch = Some(self.pcs[self.pcsp]);
                }
                Control::Nop | Control::Unknown(_) => {}
            }
        }

        // Read phase: transfer source, then arithmetic sources.
        let moved: Vec<(Loc, u32)> = match insn.transfer {
            Transfer::Move { src, dst } => {
                let v = self.loc_read(&src, ext);
                vec![(dst, v)]
            }
            Transfer::Double { a, b } => {
                let va = self.loc_read(&a.0, ext);
                let vb = self.loc_read(&b.0, ext);
                vec![(a.1, va), (b.1, vb)]
            }
            _ => vec![],
        };
        let alu = insn.alu.as_ref().and_then(|a| self.alu(a));
        let mul = insn.mul.as_ref().map(|m| self.mul(m));

        // Write phase.
        if let Some((o, v)) = alu {
            self.out_write(o, v);
        }
        if let Some((o, v)) = mul {
            self.out_write(o, v);
        }
        for (dst, v) in moved {
            self.loc_write(&dst, v, ext);
        }
        if let Some(r) = pop {
            let v = self.bram[self.sp as usize];
            self.sp = (self.sp + 1) & 0x3ff;
            self.reg_write(r, v);
        }

        // Next PC: a delayed branch from the previous word wins; REP holds.
        let mut next = (pc + 1) % PROGRAM_WORDS;
        if !rep_set && self.rpc > 0 {
            self.rpc -= 1;
            if self.rpc > 0 {
                next = pc;
                self.st |= RP;
            } else {
                self.st &= !RP;
            }
        }
        if let Some(t) = delayed {
            next = t;
        }
        self.pending = branch;
        self.pc = next;
        self.steps += 1;
        Step::Ran
    }

    /// Run until stalled on input or `max` words have executed.
    pub fn run(&mut self, ext: &mut impl External, max: u64) -> Step {
        for _ in 0..max {
            match self.step(ext) {
                Step::Ran => {}
                other => return other,
            }
        }
        Step::Ran
    }
}

#[cfg(test)]
mod tests;
