//! Reference interpreter for the ADSP-21062 core, its internal memory and
//! the I/O-processor registers a host uses to boot it and talk to it.
//!
//! Semantics follow the User's Manual (see the crate docs); the choices
//! where it is silent or ambiguous are listed in `docs/hw/adsp2106x.md`.
//! The main modelling decisions:
//! - The fetch/decode/execute pipeline is modelled at the level the
//!   program can observe (UM §3.1.1): a branch executes with the next two
//!   words already fetched, which then run (delayed branch) or become
//!   NOPs; a loop's termination is tested when its last word is fetched,
//!   i.e. while the word two before it executes (UM §3.5), which gives the
//!   manual's short-loop behaviour without special cases.
//! - Within an instruction, register reads happen before any register
//!   write (UM §2.9); writes to the same register file location land in
//!   the order shifter, multiplier, ALU, PM, DM/universal register, so the
//!   higher-precedence source wins.
//! - Effect latencies of system register writes (UM §E.2.1) and extra
//!   cycles (DAG stalls, PM bus conflicts, wait states) aren't modelled;
//!   `cycles` counts instructions plus the two NOPs of each non-delayed
//!   branch or interrupt.

use crate::mem::{self, Internal};
use crate::{decode, Addr, Insn, Modifier, Op, LCE, TRUE};
use std::collections::VecDeque;

mod exec;
mod recomp;
pub use recomp::{code_hash, program_hash, program_image, Block, BlockEnd, InBlock};

/// Width of an external port access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    /// A 32-bit data word (DM bus register transfers).
    Data32,
    /// A 48-bit word: an instruction, PX, or 40-bit data left-justified
    /// (PM bus transfers).
    Word48,
}

/// The external port: addresses 0x400000 and up, and the multiprocessor
/// space 0x80000-0x3FFFFF, which this model leaves to the board (UM §5.2).
pub trait Bus {
    fn read(&mut self, addr: u32, width: Width) -> u64;
    fn write(&mut self, addr: u32, width: Width, v: u64);
    /// Called by [`Sharc::run_bus`] (and recompiled programs' `run`)
    /// after each instruction that ran: false stops the run there. The
    /// host can look at or change the core here (its FLAG inputs, say),
    /// as it could between calls to [`Sharc::step`].
    #[inline(always)]
    fn after_step(&mut self, _cpu: &mut Sharc) -> bool {
        true
    }
    /// How many instructions recompiled code may run as one block from
    /// here, calling [`after_step`](Bus::after_step) only after the last
    /// of them (and [`after_access`](Bus::after_access) after each that
    /// used the external port). 1, the default, is a call after every
    /// instruction. A bus that allows more promises that, for every
    /// instruction of the block but the last, `after_step` would have
    /// returned true and done nothing but what `after_access` does,
    /// given what the core guarantees of a block's instructions before
    /// its last (`Sharc::block_start`): none is a taken branch, changes
    /// the loop, PC or status stacks or a sequencer or system register,
    /// takes an interrupt or idles, and [`Sharc::spinning`] is false
    /// after each of them; `steps` counts them as usual.
    #[inline(always)]
    fn block_budget(&mut self, _cpu: &Sharc) -> u64 {
        1
    }
    /// Within a block, after an instruction that read or wrote the
    /// external port, with the core's `steps` counting it. The rest of the
    /// core's state may lag (the pipeline, cycles, interrupt latches); this
    /// may read `steps` and change the FLAG inputs, nothing else.
    #[inline(always)]
    fn after_access(&mut self, _cpu: &mut Sharc) {}
}

/// Why the core stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// An encoding the manual doesn't define.
    Undefined { pc: u32, word: u64 },
    /// Something defined but not implemented by this model.
    Unimplemented { pc: u32, what: &'static str },
    /// A core access to an IOP register this model doesn't implement.
    Iop { pc: u32, addr: u32, write: bool },
    /// A data access to reserved internal addresses (0x100-0x1FFFF), or
    /// an instruction fetch from somewhere instructions can't be.
    BadAddress { pc: u32, addr: u32 },
    /// PC, loop or status stack overflow, or a return with an empty PC
    /// stack.
    Stack { pc: u32, what: &'static str },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Ran,
    /// In IDLE (or waiting to be booted): nothing ran.
    Idle,
    Fault(Fault),
}

/// Errors of the host interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostError {
    /// An IOP register or mode this model doesn't implement.
    Unimplemented { addr: u32 },
    /// A write to a full external port buffer (the host would hang).
    BufferFull { addr: u32 },
}

// ASTAT bits (UM §E.6).
pub const AZ: u32 = 1 << 0;
pub const AV: u32 = 1 << 1;
pub const AN: u32 = 1 << 2;
pub const AC: u32 = 1 << 3;
pub const AS: u32 = 1 << 4;
pub const AI: u32 = 1 << 5;
pub const MN: u32 = 1 << 6;
pub const MV: u32 = 1 << 7;
pub const MU: u32 = 1 << 8;
pub const MI: u32 = 1 << 9;
pub const AF: u32 = 1 << 10;
pub const SV: u32 = 1 << 11;
pub const SZ: u32 = 1 << 12;
pub const SS: u32 = 1 << 13;
pub const BTF: u32 = 1 << 18;
const FLG_SHIFT: u32 = 19;
const FLG_BITS: u32 = 0xf << FLG_SHIFT;

// STKY bits (UM §E.7).
pub const AUS: u32 = 1 << 0;
pub const AVS: u32 = 1 << 1;
pub const AOS: u32 = 1 << 2;
pub const AIS: u32 = 1 << 5;
pub const MOS: u32 = 1 << 6;
pub const MVS: u32 = 1 << 7;
pub const MUS: u32 = 1 << 8;
pub const MIS: u32 = 1 << 9;
pub const CB7S: u32 = 1 << 17;
pub const CB15S: u32 = 1 << 18;
const PCFL: u32 = 1 << 21;
const PCEM: u32 = 1 << 22;
const SSOV: u32 = 1 << 23;
const SSEM: u32 = 1 << 24;
const LSOV: u32 = 1 << 25;
const LSEM: u32 = 1 << 26;
const STKY_RO: u32 = 0x3f << 21;

// MODE1 bits (UM §E.4).
pub const BR8: u32 = 1 << 0;
pub const BR0: u32 = 1 << 1;
pub const SRCU: u32 = 1 << 2;
pub const SRD1H: u32 = 1 << 3;
pub const SRD1L: u32 = 1 << 4;
pub const SRD2H: u32 = 1 << 5;
pub const SRD2L: u32 = 1 << 6;
pub const SRRFH: u32 = 1 << 7;
pub const SRRFL: u32 = 1 << 10;
pub const NESTM: u32 = 1 << 11;
pub const IRPTEN: u32 = 1 << 12;
pub const ALUSAT: u32 = 1 << 13;
pub const SSE: u32 = 1 << 14;
pub const TRUNC: u32 = 1 << 15;
pub const RND32: u32 = 1 << 16;

// MODE2 bits (UM §E.5).
const TIMEN: u32 = 1 << 5;
const FLG_OUT_SHIFT: u32 = 15;
/// ADSP-21062 processor ID in MODE2 bits 31-30 (UM §E.5).
const MODE2_ID: u32 = 0b10 << 30;

// Interrupt latch bits (UM Table 3.3).
pub const IRQ_TMZH: u32 = 4;
pub const IRQ_VIRPT: u32 = 5;
pub const IRQ_EP0: u32 = 16;
const IRQ_CB7: u32 = 21;
const IRQ_CB15: u32 = 22;
pub const IRQ_TMZL: u32 = 23;

// IOP register addresses (UM Table E.7).
pub const SYSCON: u32 = 0x00;
pub const VIRPT: u32 = 0x01;
pub const WAIT: u32 = 0x02;
pub const SYSTAT: u32 = 0x03;
pub const EPB0: u32 = 0x04;
pub const MSGR0: u32 = 0x08;
pub const DMAC6: u32 = 0x1c;
pub const II6: u32 = 0x40;
pub const IM6: u32 = 0x41;
pub const C6: u32 = 0x42;

/// IOP addresses Table E.7 marks reserved on the ADSP-21062 [UM E-9 to
/// E-13]. The manual doesn't say what they do; the core ignores writes and
/// reads them as 0. `schamp`'s copro program writes 0x20 (through an
/// index register it hasn't set yet, docs/hw/adsp2106x.md).
fn reserved_iop(a: u32) -> bool {
    matches!(
        a,
        0x10..=0x17 | 0x1a | 0x20..=0x2f | 0x3f | 0x67 | 0x6f | 0x77 | 0x7f | 0xa0..=0xbf | 0xd0..=0xdf
            | 0xe5 | 0xe7 | 0xec | 0xed | 0xef | 0xf5 | 0xf7 | 0xfc | 0xfd | 0xff
    )
}

// DMAC bits (UM §E.12).
const DEN: u32 = 1 << 0;
const CHEN: u32 = 1 << 1;
const TRAN: u32 = 1 << 2;
const DTYPE: u32 = 1 << 5;
const MSWF: u32 = 1 << 8;
const MASTER: u32 = 1 << 9;
const HSHAKE: u32 = 1 << 10;
const FLSH: u32 = 1 << 13;

/// External port buffer depth (UM §E.3.3).
pub const EPB_DEPTH: usize = 6;
/// PC stack depth (UM §3.4.2), loop stacks (§3.5.2), status stack (§3.6.6).
pub const PC_STACK: usize = 30;
pub const LOOP_STACK: usize = 6;
pub const STATUS_STACK: usize = 5;

/// Interrupts that push ASTAT and MODE1 on the status stack: IRQ2-0, the
/// timer and VIRPT (UM §3.6.6).
fn pushes_status(irq: u32) -> bool {
    matches!(irq, 4..=8 | 23)
}

/// Bit-reverse the low `width` bits.
pub fn bitrev(v: u32, width: u32) -> u32 {
    v.reverse_bits() >> (32 - width)
}

/// A pipeline slot: the address fetched, and whether that fetch tested a
/// loop end and looped back (so re-fetching it would test again).
type Slot = Option<(u32, bool)>;

/// How a loop terminates (UM §3.5.2 loop type code).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoopKind {
    Cond = 0,
    Counter1 = 1,
    Counter2 = 2,
    Counter = 3,
}

/// One loop stack entry as LADDR shows it: end address in bits 23-0,
/// termination code in 28-24, loop type in 31-30.
fn laddr_pack(end: u32, term: u8, kind: LoopKind) -> u32 {
    end & 0xff_ffff | (term as u32 & 31) << 24 | (kind as u32) << 30
}

fn laddr_end(l: u32) -> u32 {
    l & 0xff_ffff
}

fn laddr_counter(l: u32) -> bool {
    l >> 30 != 0
}

fn laddr_term(l: u32) -> u8 {
    (l >> 24 & 31) as u8
}

pub struct Sharc {
    pub mem: Internal,
    /// Register file R0-R15/F0-F15, 40 bits each; `r_alt` holds the
    /// inactive halves of the alternate set (UM §2.9.1).
    pub r: [u64; 16],
    pub r_alt: [u64; 16],
    /// DAG registers (DAG2's are 24 bits) and their inactive alternates.
    pub i: [u32; 16],
    pub m: [u32; 16],
    pub l: [u32; 16],
    pub b: [u32; 16],
    dag_alt: [[u32; 16]; 4],
    /// The two 80-bit multiplier result registers, sign-extended; MRF is
    /// `mr[SRCU]` (UM §2.6.2.1).
    pub mr: [i128; 2],
    /// PX: PX2 in bits 47-16, PX1 in 15-0.
    pub px: u64,
    pub mode1: u32,
    mode2: u32,
    astat: u32,
    stky: u32,
    pub irptl: u32,
    pub imask: u32,
    pub imaskp: u32,
    pub ustat1: u32,
    pub ustat2: u32,
    pub tperiod: u32,
    pub tcount: u32,
    pc_stack: Vec<u32>,
    /// Loop address stack (LADDR format) with the loop counter stack.
    loop_stack: Vec<(u32, u32)>,
    pub lcntr: u32,
    /// Saved (ASTAT, MODE1) pairs.
    status_stack: Vec<(u32, u32)>,

    /// Address of the instruction executing (PC).
    pub pc: u32,
    /// Next instruction to execute (the decode stage).
    exec_slot: Slot,
    /// The instruction after that (fetched last cycle).
    decode_slot: Slot,
    /// Next address to fetch (FADDR).
    pub faddr: u32,
    /// Branch requested by the executing instruction: (target, delayed).
    redirect: Option<(u32, bool)>,
    /// A DO UNTIL ran: re-run the loop test on the words already fetched.
    rebuild: bool,
    /// RTS (LR): the next fetch of the loop end that last looped back
    /// loops back again without a second test (UM A-34).
    reentry: bool,
    /// The loop end whose last test looped back, while that loop runs.
    last_loopback: Option<u32>,
    /// ASTAT and MODE1 at the start of the cycle, for its loop test.
    fetch_status: (u32, u32),
    /// Per interrupt nesting level: the interrupt squashed a fetch that
    /// had already looped back, so its RTI must re-enter like RTS (LR).
    irq_reentry: Vec<bool>,
    /// Instructions to finish before an interrupt can be taken (branches
    /// and their delay slots, UM §3.6.1).
    irq_hold: u8,
    pub idle: bool,
    /// Reset in host boot mode until the first interrupt: the channel 6
    /// DMA completion vectors even though IRPTEN is clear (UM §11.6.3).
    booting: bool,

    /// IOP registers 0x00-0xFF, as stored; see `iop_read`.
    pub iop: Vec<u32>,
    /// EPB0, the external port buffer of DMA channel 6.
    pub epb0: VecDeque<u64>,
    /// Host or DMA packing of EPB0: partial word and halves so far.
    epb0_pack: (u64, u32),
    /// Host interface 16-bit packing (UM §8): first half of a write, and
    /// the latched value of a read.
    host_wlatch: Option<u16>,
    host_rlatch: Option<u16>,
    /// IRQ2-0 input levels, and FLAG3-0 input pins.
    irq_lines: [bool; 3],
    pub flag_in: u8,

    pub cycles: u64,
    pub steps: u64,
    /// Executions per internal 48-bit word, block 0 then block 1.
    pub counts: Vec<u64>,
    decoded: Vec<Option<Insn>>,
    /// Recompiled code (`recomp.rs`): per internal word, 0 if none, 1 if
    /// compiled, 2 if compiled and since overwritten.
    code: Vec<u8>,
    /// Some compiled word has been overwritten.
    code_dirty: bool,
    /// Instructions recompiled code had none for, by address.
    pub misses: std::collections::BTreeMap<u32, u64>,
}

impl Default for Sharc {
    fn default() -> Self {
        Self::new()
    }
}

/// Index of an internal 48-bit word in `counts` and the decode cache.
pub fn word_index(addr: u32) -> Option<usize> {
    let (b, g, r) = mem::locate(addr)?;
    (g < 5).then_some(b * mem::WORDS48 as usize + g * mem::ROWS + r)
}

impl Sharc {
    pub fn new() -> Self {
        let mut s = Sharc {
            mem: Internal::default(),
            r: [0; 16],
            r_alt: [0; 16],
            i: [0; 16],
            m: [0; 16],
            l: [0; 16],
            b: [0; 16],
            dag_alt: [[0; 16]; 4],
            mr: [0; 2],
            px: 0,
            mode1: 0,
            mode2: 0,
            astat: 0,
            stky: 0,
            irptl: 0,
            imask: 0,
            imaskp: 0,
            ustat1: 0,
            ustat2: 0,
            tperiod: 0,
            tcount: 0,
            pc_stack: Vec::new(),
            loop_stack: Vec::new(),
            lcntr: 0,
            status_stack: Vec::new(),
            pc: 0,
            exec_slot: None,
            decode_slot: None,
            faddr: 0,
            redirect: None,
            rebuild: false,
            reentry: false,
            last_loopback: None,
            fetch_status: (0, 0),
            irq_reentry: Vec::new(),
            irq_hold: 0,
            idle: false,
            booting: false,
            iop: vec![0; 0x100],
            epb0: VecDeque::new(),
            epb0_pack: (0, 0),
            host_wlatch: None,
            host_rlatch: None,
            irq_lines: [false; 3],
            flag_in: 0,
            cycles: 0,
            steps: 0,
            counts: vec![0; 2 * mem::WORDS48 as usize],
            decoded: vec![None; 2 * mem::WORDS48 as usize],
            code: Vec::new(),
            code_dirty: false,
            misses: Default::default(),
        };
        s.reset();
        s
    }

    /// Reset in host boot mode (UM §11.6.3): the core idles with the PC at
    /// 0x20004 while DMA channel 6 waits for 256 instruction words in
    /// EPB0, packed from 16-bit host writes, least significant first. When
    /// the count runs out, the EP0I interrupt vectors to 0x20040, whose RTI
    /// resumes at 0x20005. Memory, the register file and the timer
    /// registers are not affected.
    pub fn reset(&mut self) {
        self.mode1 = 0;
        self.set_mode1(0);
        self.mode2 = 0;
        self.astat = 0;
        self.stky = 0;
        self.irptl = 0;
        self.imask = 0x3 | 1 << IRQ_EP0;
        self.imaskp = 0;
        self.pc_stack.clear();
        self.loop_stack.clear();
        self.status_stack.clear();
        self.irq_reentry.clear();
        self.update_stack_flags();
        self.iop.iter_mut().for_each(|r| *r = 0);
        self.iop[SYSCON as usize] = 0x10;
        self.iop[VIRPT as usize] = 0x2_0014;
        self.iop[WAIT as usize] = 0x21ad_6b5a;
        // ID 0 (single processor): current bus master reads as 1.
        self.iop[SYSTAT as usize] = 1 << 4;
        self.iop[DMAC6 as usize] = 0xa1;
        self.iop[II6 as usize] = 0x2_0000;
        // "Uninitialized (increment by 1 is automatic)" for booting.
        self.iop[IM6 as usize] = 1;
        self.iop[C6 as usize] = 0x100;
        self.iop[0xc8] = 0x2_c688;
        self.iop[0xee] = 1;
        self.iop[0xfe] = 1;
        self.epb0.clear();
        self.epb0_pack = (0, 0);
        self.host_wlatch = None;
        self.host_rlatch = None;
        self.pc = 0x2_0004;
        self.exec_slot = Some((0x2_0005, false));
        self.decode_slot = None;
        self.faddr = 0x2_0006;
        self.redirect = None;
        self.rebuild = false;
        self.reentry = false;
        self.last_loopback = None;
        self.irq_hold = 0;
        self.idle = true;
        self.booting = true;
    }

    /// Start executing at `addr` with an empty pipeline, as if jumped to
    /// (for tests and tools that load memory directly).
    pub fn start_at(&mut self, addr: u32) {
        self.pc = addr;
        self.idle = false;
        self.booting = false;
        self.exec_slot = None;
        self.decode_slot = None;
        self.faddr = addr;
    }

    /// Load 48-bit words into internal memory from `addr` on.
    pub fn load48(&mut self, addr: u32, words: &[u64]) {
        for (k, &w) in words.iter().enumerate() {
            let a = addr + k as u32;
            self.mem.write48(a, w);
            self.invalidate(a, true);
        }
    }

    #[inline]
    fn invalidate(&mut self, addr: u32, width48: bool) {
        for a in Internal::words48_touched(addr, width48).into_iter().flatten() {
            if let Some(k) = word_index(a) {
                self.decoded[k] = None;
                if let Some(c @ 1..) = self.code.get_mut(k) {
                    // Self-modifying code: that word runs on the
                    // interpreter from now on.
                    *c = 2;
                    self.code_dirty = true;
                }
            }
        }
    }

    // --- status ----------------------------------------------------------

    /// MODE2 as read: bits 31-30 hold the processor ID.
    pub fn mode2(&self) -> u32 {
        self.mode2 | MODE2_ID
    }

    /// ASTAT as read: FLAG3-0 configured as inputs show the pins.
    #[inline(always)]
    pub fn astat(&self) -> u32 {
        let outputs = (self.mode2 >> FLG_OUT_SHIFT) & 0xf;
        let pins = (self.flag_in as u32 & !outputs) << FLG_SHIFT;
        (self.astat & !(!(outputs << FLG_SHIFT) & FLG_BITS)) | pins
    }

    /// FLAG3-0 pins configured as outputs, as the program drives them.
    pub fn flag_outputs(&self) -> u8 {
        let outputs = (self.mode2 >> FLG_OUT_SHIFT) & 0xf;
        ((self.astat >> FLG_SHIFT) & outputs) as u8
    }

    pub fn stky(&self) -> u32 {
        self.stky
    }

    #[inline(always)]
    fn update_stack_flags(&mut self) {
        let mut s = self.stky & !(PCFL | PCEM | SSEM | LSEM);
        if self.pc_stack.len() >= PC_STACK {
            s |= PCFL;
        }
        if self.pc_stack.is_empty() {
            s |= PCEM;
        }
        if self.status_stack.is_empty() {
            s |= SSEM;
        }
        if self.loop_stack.is_empty() {
            s |= LSEM;
        }
        self.stky = s;
    }

    /// Write MODE1, swapping register file, DAG and halves whose
    /// alternate-select bit changed (UM §2.9.1, §4.2.1).
    pub fn set_mode1(&mut self, v: u32) {
        let changed = self.mode1 ^ v;
        if changed & SRRFL != 0 {
            for k in 0..8 {
                std::mem::swap(&mut self.r[k], &mut self.r_alt[k]);
            }
        }
        if changed & SRRFH != 0 {
            for k in 8..16 {
                std::mem::swap(&mut self.r[k], &mut self.r_alt[k]);
            }
        }
        for (bit, range) in [(SRD1L, 0..4), (SRD1H, 4..8), (SRD2L, 8..12), (SRD2H, 12..16)] {
            if changed & bit != 0 {
                for k in range {
                    std::mem::swap(&mut self.i[k], &mut self.dag_alt[0][k]);
                    std::mem::swap(&mut self.m[k], &mut self.dag_alt[1][k]);
                    std::mem::swap(&mut self.l[k], &mut self.dag_alt[2][k]);
                    std::mem::swap(&mut self.b[k], &mut self.dag_alt[3][k]);
                }
            }
        }
        self.mode1 = v;
    }

    #[inline(always)]
    fn mrf(&self) -> usize {
        (self.mode1 & SRCU != 0) as usize
    }

    /// Evaluate condition code `c` of an `IF` (UM Table 3.2).
    #[inline(always)]
    pub fn cond(&self, c: u8) -> bool {
        self.cond_with(c, self.astat(), self.mode1)
    }

    /// `cond` against a given ASTAT and MODE1.
    #[inline(always)]
    fn cond_with(&self, c: u8, a: u32, mode1: u32) -> bool {
        let f = |bit: u32| a & bit != 0;
        let base = |c: u8| -> bool {
            let lt = |le: bool| {
                let fixed = !f(AF) && (f(AN) ^ (f(AV) && mode1 & ALUSAT == 0));
                if le {
                    fixed || (f(AF) && f(AN)) || f(AZ)
                } else {
                    fixed || (f(AF) && f(AN) && !f(AZ))
                }
            };
            match c {
                0 => f(AZ),
                1 => lt(false),
                2 => lt(true),
                3 => f(AC),
                4 => f(AV),
                5 => f(MV),
                6 => f(MN),
                7 => f(SV),
                8 => f(SZ),
                9..=12 => a >> (FLG_SHIFT + (c - 9) as u32) & 1 != 0,
                13 => f(BTF),
                // Bus master: this model is a single-processor system, so
                // always the master while CSEL (MODE1 17-18) is zero.
                _ => mode1 >> 17 & 3 == 0,
            }
        };
        match c & 31 {
            15 => self.curlcntr() != 1,
            31 => true,
            c if c < 15 => base(c),
            c => !base(c - 16),
        }
    }

    /// Does loop termination code `t` hold (UM Table 3.2: LCE, FOREVER)?
    /// Arithmetic conditions are those at the start of the cycle.
    #[inline(always)]
    fn term(&self, t: u8) -> bool {
        let (a, m) = self.fetch_status;
        match t & 31 {
            LCE => self.curlcntr() == 1,
            TRUE => false,
            c => self.cond_with(c, a, m),
        }
    }

    #[inline(always)]
    pub fn curlcntr(&self) -> u32 {
        self.loop_stack.last().map_or(u32::MAX, |e| e.1)
    }

    pub fn pc_stack(&self) -> &[u32] {
        &self.pc_stack
    }

    pub fn loop_depth(&self) -> usize {
        self.loop_stack.len()
    }

    // --- registers --------------------------------------------------------

    /// The 32-bit fixed-point field of a register file location.
    #[inline(always)]
    pub fn rf(&self, n: u8) -> u32 {
        (self.r[n as usize & 15] >> 8) as u32
    }

    pub fn set_rf(&mut self, n: u8, v: u32) {
        self.r[n as usize & 15] = (v as u64) << 8;
    }

    /// A universal register as the 40-bit DM data bus carries it: 32-bit
    /// registers in bits 39-8 (UM §4.4, §5.1.4).
    #[inline(always)]
    pub fn ureg_read(&self, code: u8) -> u64 {
        let n = (code & 15) as usize;
        let sext24 = |v: u32| ((v << 8) as i32 >> 8) as u32;
        let v32 = match code >> 4 {
            0 => return self.r[n],
            1 => self.i[n],
            2 if n >= 8 => sext24(self.m[n]),
            2 => self.m[n],
            3 => self.l[n],
            4 => self.b[n],
            6 => match n {
                0 => self.faddr,
                1 => self.decode_slot.map_or(0, |s| s.0),
                3 => self.pc,
                4 => self.pc_stack.last().copied().unwrap_or(0),
                5 => self.pc_stack.len() as u32,
                6 => self.loop_stack.last().map_or(u32::MAX, |e| e.0),
                7 => self.curlcntr(),
                _ => self.lcntr,
            },
            7 => match n {
                0 => self.ustat1,
                1 => self.ustat2,
                9 => self.irptl,
                10 => self.mode2(),
                11 => self.mode1,
                12 => self.astat(),
                13 => self.imask,
                14 => self.stky,
                _ => self.imaskp,
            },
            _ => match n {
                0xb => return self.px >> 8,
                0xc => self.px as u32 & 0xffff,
                0xd => (self.px >> 16) as u32,
                0xe => self.tperiod,
                _ => self.tcount,
            },
        };
        (v32 as u64) << 8
    }

    /// Write a universal register from the 40-bit DM bus.
    #[inline(always)]
    pub fn ureg_write(&mut self, code: u8, v: u64) {
        let n = (code & 15) as usize;
        let v32 = (v >> 8) as u32;
        let dag = |n: usize, v: u32| if n >= 8 { v & 0xff_ffff } else { v };
        match code >> 4 {
            0 => self.r[n] = v & 0xff_ffff_ffff,
            1 => self.i[n] = dag(n, v32),
            2 => self.m[n] = dag(n, v32),
            3 => self.l[n] = dag(n, v32),
            4 => {
                // Loading B also loads I (UM §4.3.2.2).
                self.b[n] = dag(n, v32);
                self.i[n] = self.b[n];
            }
            6 => match n {
                4 => {
                    if let Some(t) = self.pc_stack.last_mut() {
                        *t = v32 & 0xff_ffff;
                    }
                }
                5 => {
                    let depth = (v32 as usize).min(PC_STACK);
                    self.pc_stack.resize(depth, 0);
                    self.update_stack_flags();
                }
                6 => {
                    if let Some(t) = self.loop_stack.last_mut() {
                        t.0 = v32;
                    }
                }
                7 => {
                    // No effect unless a counter-based loop is running.
                    if let Some(t) = self.loop_stack.last_mut() {
                        if laddr_counter(t.0) {
                            t.1 = v32;
                        }
                    }
                }
                8 => self.lcntr = v32,
                _ => {}
            },
            7 => match n {
                0 => self.ustat1 = v32,
                1 => self.ustat2 = v32,
                9 => self.irptl = v32 & !2,
                10 => self.mode2 = v32 & 0x0fff_ffff,
                11 => self.set_mode1(v32),
                12 => self.astat = v32,
                13 => self.imask = v32,
                14 => self.stky = (self.stky & STKY_RO) | (v32 & !STKY_RO),
                _ => self.imaskp = v32,
            },
            _ => match n {
                0xb => self.px = (v & 0xff_ffff_ffff) << 8,
                0xc => self.px = (self.px & !0xffff) | (v32 & 0xffff) as u64,
                0xd => self.px = (self.px & 0xffff) | (v32 as u64) << 16,
                0xe => self.tperiod = v32,
                _ => self.tcount = v32,
            },
        }
    }

    // --- DAGs --------------------------------------------------------------

    /// An M register as a signed modifier (DAG2's are 24-bit).
    #[inline(always)]
    fn m_value(&self, m: u8) -> i64 {
        let v = self.m[m as usize];
        if m >= 8 {
            ((v << 8) as i32 >> 8) as i64
        } else {
            v as i32 as i64
        }
    }

    /// Post-modify update of I`i` by `m`, with circular buffering when
    /// L`i` is nonzero (UM §4.3.2.1); I7 and I15 flag wrap-around
    /// (§4.3.2.3).
    #[inline(always)]
    fn modify(&mut self, i: usize, m: i64) {
        let dag2 = i >= 8;
        let mask: u64 = if dag2 { 0xff_ffff } else { 0xffff_ffff };
        let (iv, b, l) = (self.i[i] as u64, self.b[i] as u64, self.l[i] as u64);
        let raw = (iv as i64 + m) as u64 & mask;
        let wraps = if m >= 0 { iv as i64 + m >= (b + l) as i64 } else { (iv as i64 + m) < b as i64 };
        let new = if l != 0 && wraps {
            if m >= 0 {
                (raw.wrapping_sub(l)) & mask
            } else {
                (raw + l) & mask
            }
        } else {
            raw
        };
        self.i[i] = new as u32;
        if (i == 7 || i == 15) && wraps {
            let (sticky, irq) = if i == 7 { (CB7S, IRQ_CB7) } else { (CB15S, IRQ_CB15) };
            self.stky |= sticky;
            self.latch(irq);
        }
    }

    /// The address a DAG outputs for `a`, updating the I register for
    /// post-modify. I0/I8 outputs are bit-reversed in BR0/BR8 mode (UM
    /// §4.3.3.1).
    #[inline(always)]
    fn dag_output(&mut self, a: &Addr) -> u32 {
        let out = self.dag_peek(a);
        if a.post {
            self.modify(a.i as usize, self.dag_modifier(a));
        }
        out
    }

    #[inline(always)]
    fn dag_modifier(&self, a: &Addr) -> i64 {
        match a.modifier {
            Modifier::M(m) => self.m_value(m),
            Modifier::Imm(v) => v as i64,
        }
    }

    /// The address [`dag_output`](Sharc::dag_output) gives for `a`,
    /// without the post-modify.
    #[inline(always)]
    fn dag_peek(&self, a: &Addr) -> u32 {
        let i = a.i as usize;
        let dag2 = i >= 8;
        let width = if dag2 { 24 } else { 32 };
        let mask = if dag2 { 0xff_ffff } else { u32::MAX };
        let out = if a.post { self.i[i] } else { (self.i[i] as i64 + self.dag_modifier(a)) as u32 & mask };
        let br = if dag2 { BR8 } else { BR0 };
        if (i == 0 || i == 8) && self.mode1 & br != 0 {
            bitrev(out, width)
        } else {
            out
        }
    }

    // --- memory -------------------------------------------------------------

    /// IMDW of the block `addr` is in: 40-bit data (UM §E.9).
    #[inline(always)]
    fn data48(&self, addr: u32) -> bool {
        let block1 = addr >= mem::BLOCK1;
        self.iop[SYSCON as usize] >> (8 + block1 as u32) & 1 != 0
    }

    /// Read for a register transfer: the register's new 40-bit value, or
    /// for PX all 48 bits (`px48`).
    #[inline(always)]
    fn load(&mut self, pm: bool, addr: u32, px: bool, ext: &mut impl Bus) -> Result<Loaded, Fault> {
        Ok(match addr {
            0..=0xff => Loaded::Bus((self.iop_read(addr)? as u64) << 8),
            0x100..=0x1_ffff => return Err(Fault::BadAddress { pc: self.pc, addr }),
            0x2_0000..=0x3_ffff => {
                if px || self.data48(addr) {
                    let w = self.mem.read48(addr);
                    if px && pm {
                        Loaded::Px48(w)
                    } else {
                        Loaded::Bus(w >> 8)
                    }
                } else {
                    Loaded::Bus((self.mem.read32(addr) as u64) << 8)
                }
            }
            0x4_0000..=0x7_ffff => {
                let v = self.mem.read16(addr) as u32;
                let v = if !px && self.mode1 & SSE != 0 { v as u16 as i16 as i32 as u32 } else { v };
                Loaded::Bus((v as u64) << 8)
            }
            _ => {
                if pm || px {
                    let w = ext.read(addr, Width::Word48) & crate::WORD_MASK;
                    if px {
                        Loaded::Px48(w)
                    } else {
                        Loaded::Bus(w >> 8)
                    }
                } else {
                    Loaded::Bus((ext.read(addr, Width::Data32) as u32 as u64) << 8)
                }
            }
        })
    }

    /// Store a register transfer: `v` is the 40-bit DM bus value, `px48`
    /// the whole of PX when PX is the register.
    #[inline(always)]
    fn store(&mut self, pm: bool, addr: u32, v: u64, px48: Option<u64>, ext: &mut impl Bus) -> Result<(), Fault> {
        match addr {
            0..=0xff => self.iop_write(addr, (v >> 8) as u32)?,
            0x100..=0x1_ffff => return Err(Fault::BadAddress { pc: self.pc, addr }),
            0x2_0000..=0x3_ffff => {
                if px48.is_some() || self.data48(addr) {
                    // DM bus transfers of PX are 40 bits (UM §5.3).
                    let w = match px48 {
                        Some(p) if pm => p,
                        Some(p) => p & !0xff,
                        None => v << 8,
                    };
                    self.mem.write48(addr, w);
                    self.invalidate(addr, true);
                } else {
                    self.mem.write32(addr, (v >> 8) as u32);
                    self.invalidate(addr, false);
                }
            }
            0x4_0000..=0x7_ffff => {
                self.mem.write16(addr, (v >> 8) as u16);
                self.invalidate(mem::short_to_normal(addr), false);
            }
            _ => match px48 {
                Some(p) => ext.write(addr, Width::Word48, p),
                None if pm => ext.write(addr, Width::Word48, v << 8),
                None => ext.write(addr, Width::Data32, (v >> 8) & 0xffff_ffff),
            },
        }
        Ok(())
    }

    /// Transfer between memory and universal register `reg` through the
    /// DM bus (DAG1, `pm` false) or PM bus. For a store, `value` is the
    /// register's value read at the start of the instruction.
    #[inline(always)]
    fn transfer(
        &mut self,
        pm: bool,
        addr: u32,
        write: bool,
        reg: u8,
        value: u64,
        ext: &mut impl Bus,
    ) -> Result<Option<Loaded>, Fault> {
        let px = reg == PX;
        if write {
            self.store(pm, addr, value, px.then_some(self.px), ext)?;
            Ok(None)
        } else {
            self.load(pm, addr, px, ext).map(Some)
        }
    }

    #[inline(always)]
    fn apply_load(&mut self, reg: u8, l: Loaded) {
        match l {
            Loaded::Bus(v) => self.ureg_write(reg, v),
            Loaded::Px48(v) => self.px = v & crate::WORD_MASK,
        }
    }

    /// The decoded instruction at `addr`.
    fn fetch(&mut self, addr: u32, ext: &mut impl Bus) -> Result<Insn, Fault> {
        match addr {
            0x2_0000..=0x3_ffff => {
                let Some(k) = word_index(addr) else { return Err(Fault::BadAddress { pc: addr, addr }) };
                self.counts[k] += 1;
                if let Some(i) = self.decoded[k] {
                    return Ok(i);
                }
                let i = decode(addr, self.mem.read48(addr));
                self.decoded[k] = Some(i);
                Ok(i)
            }
            0x40_0000.. => Ok(decode(addr, ext.read(addr, Width::Word48))),
            _ => Err(Fault::BadAddress { pc: addr, addr }),
        }
    }

    // --- sequencer ------------------------------------------------------------

    #[inline(always)]
    fn push_pc(&mut self, v: u32) -> Result<(), Fault> {
        if self.pc_stack.len() >= PC_STACK {
            return Err(Fault::Stack { pc: self.pc, what: "PC stack overflow" });
        }
        self.pc_stack.push(v & 0xff_ffff);
        self.update_stack_flags();
        Ok(())
    }

    #[inline(always)]
    fn pop_pc(&mut self) -> Option<u32> {
        let v = self.pc_stack.pop();
        self.update_stack_flags();
        v
    }

    #[inline(always)]
    fn push_loop(&mut self, laddr: u32, count: u32) -> Result<(), Fault> {
        if self.loop_stack.len() >= LOOP_STACK {
            self.stky |= LSOV;
            return Err(Fault::Stack { pc: self.pc, what: "loop stack overflow" });
        }
        self.loop_stack.push((laddr, count));
        self.update_stack_flags();
        Ok(())
    }

    #[inline(always)]
    fn pop_loop(&mut self) {
        self.last_loopback = None;
        self.loop_stack.pop();
        self.update_stack_flags();
    }

    fn push_status(&mut self) -> Result<(), Fault> {
        if self.status_stack.len() >= STATUS_STACK {
            self.stky |= SSOV;
            return Err(Fault::Stack { pc: self.pc, what: "status stack overflow" });
        }
        self.status_stack.push((self.astat, self.mode1));
        self.update_stack_flags();
        Ok(())
    }

    /// Pop ASTAT and MODE1; the FLAG bits of ASTAT carry over (UM §3.6.6).
    fn pop_status(&mut self) {
        if let Some((a, m)) = self.status_stack.pop() {
            self.astat = (a & !FLG_BITS) | (self.astat & FLG_BITS);
            self.set_mode1(m);
        }
        self.update_stack_flags();
    }

    /// The address fetched after `addr`, with the loop logic (UM §3.5): if
    /// `addr` ends the innermost loop, its termination is tested now and
    /// the loop either goes back to its top (on the PC stack) or ends
    /// (popping the loop and PC stacks). The bool is true if it looped
    /// back.
    #[inline(always)]
    fn next_after(&mut self, addr: u32) -> (u32, bool) {
        let Some(&(laddr, count)) = self.loop_stack.last() else { return (addr.wrapping_add(1), false) };
        if laddr_end(laddr) != addr {
            return (addr.wrapping_add(1), false);
        }
        let top = self.pc_stack.last().copied().unwrap_or(0);
        if self.reentry && self.last_loopback == Some(addr) {
            // Re-entry after RTS (LR): this fetch's test already happened.
            self.reentry = false;
            return (top, true);
        }
        let done = if laddr_counter(laddr) {
            if count == 1 {
                true
            } else {
                self.loop_stack.last_mut().unwrap().1 = count.wrapping_sub(1);
                false
            }
        } else {
            self.term(laddr_term(laddr))
        };
        if done {
            self.pop_loop();
            self.pop_pc();
            (addr.wrapping_add(1), false)
        } else {
            self.last_loopback = Some(addr);
            (top, true)
        }
    }

    /// Set an interrupt latch bit, unless that interrupt is being serviced
    /// (UM §3.6.3).
    pub fn latch(&mut self, irq: u32) {
        if self.imaskp & (1 << irq) == 0 {
            self.irptl |= 1 << irq;
        }
    }

    /// Drive IRQ`line` (0-2); IRQ0 latches bit 8, IRQ2 bit 6. MODE2 bits
    /// 2-0 select edge sensitivity (UM §3.6.9).
    pub fn set_irq(&mut self, line: usize, asserted: bool) {
        let edge = self.mode2 >> line & 1 != 0;
        if asserted && (!edge || !self.irq_lines[line]) {
            self.latch(8 - line as u32);
        }
        self.irq_lines[line] = asserted;
    }

    /// One clock of the timer, level-sensitive IRQs and the arithmetic
    /// exception interrupts, which follow STKY (UM §3.6.4, §3.7).
    #[inline(always)]
    fn tick(&mut self) {
        if self.mode2 & TIMEN != 0 {
            if self.tcount <= 1 {
                self.tcount = self.tperiod;
                self.latch(IRQ_TMZH);
                self.latch(IRQ_TMZL);
            } else {
                self.tcount -= 1;
            }
        }
        // Latch, as `latch` does for each, every request below at once.
        let mut req = 0;
        if self.irq_lines != [false; 3] {
            for line in 0..3 {
                if self.irq_lines[line] && self.mode2 >> line & 1 == 0 {
                    req |= 1 << (8 - line);
                }
            }
        }
        let s = self.stky;
        if s & (AOS | MOS | AVS | MVS | AUS | MUS | AIS | MIS) != 0 {
            req |= ((s & (AOS | MOS) != 0) as u32) << 24
                | ((s & (AVS | MVS) != 0) as u32) << 25
                | ((s & (AUS | MUS) != 0) as u32) << 26
                | ((s & (AIS | MIS) != 0) as u32) << 27;
        }
        self.irptl |= req & !self.imaskp;
    }

    /// The highest-priority interrupt that can be taken now (UM §3.6.5).
    fn pending_irq(&self) -> Option<u32> {
        let mut cand = self.irptl & self.imask;
        if self.booting {
            cand &= 1 << IRQ_EP0;
        } else if self.mode1 & IRPTEN == 0 {
            return None;
        }
        if self.imaskp != 0 {
            if self.mode1 & NESTM == 0 {
                return None;
            }
            let current = self.imaskp.trailing_zeros();
            cand &= (1u32 << current) - 1;
        }
        (cand != 0).then(|| cand.trailing_zeros())
    }

    /// Take a pending interrupt if there is one (UM §3.6): push the next
    /// instruction's address, and ASTAT/MODE1 for IRQ2-0, timer and
    /// VIRPT; the two words in the pipeline become NOPs.
    #[inline(always)]
    fn take_interrupt(&mut self) -> Result<bool, Fault> {
        // Nothing to take: nothing latched and unmasked, or interrupts off
        // (all a pending_irq would find).
        if self.irptl & self.imask == 0 || (!self.booting && self.mode1 & IRPTEN == 0) {
            return Ok(false);
        }
        self.take_pending()
    }

    #[inline(never)]
    fn take_pending(&mut self) -> Result<bool, Fault> {
        let Some(k) = self.pending_irq() else { return Ok(false) };
        let ret = self.exec_slot.map_or(self.faddr, |s| s.0);
        // A squashed fetch that already looped back must not test again.
        let squashed_loopback = self.exec_slot.is_some_and(|s| s.1) || self.decode_slot.is_some_and(|s| s.1);
        self.push_pc(ret)?;
        if pushes_status(k) {
            self.push_status()?;
        }
        self.irptl &= !(1 << k);
        self.imaskp |= 1 << k;
        self.irq_reentry.push(squashed_loopback);
        self.exec_slot = None;
        self.decode_slot = None;
        self.faddr = if k == IRQ_VIRPT { self.iop[VIRPT as usize] & 0xff_ffff } else { 0x2_0000 + 4 * k };
        self.idle = false;
        self.booting = false;
        self.cycles += 2;
        Ok(true)
    }

    /// Leave the current interrupt (RTI, or JUMP (CI)): clear its IMASKP
    /// bit and pop the status stack if it pushed it.
    fn end_interrupt(&mut self) {
        if self.imaskp == 0 {
            return;
        }
        let k = self.imaskp.trailing_zeros();
        self.imaskp &= !(1 << k);
        if pushes_status(k) {
            self.pop_status();
        }
        if self.irq_reentry.pop() == Some(true) {
            self.reentry = self.last_loopback.is_some();
        }
    }

    /// Execute up to one instruction: bubbles (the NOPs after non-delayed
    /// branches and interrupts) are run through until a real instruction
    /// has executed.
    pub fn step(&mut self, ext: &mut impl Bus) -> Step {
        loop {
            if self.idle {
                self.tick();
                self.cycles += 1;
                match self.take_interrupt() {
                    Ok(true) => continue,
                    Ok(false) => return Step::Idle,
                    Err(f) => return Step::Fault(f),
                }
            }
            self.begin_cycle();
            let Some((addr, _)) = self.exec_slot.take() else {
                self.fetch_stage(false, false);
                continue;
            };
            self.pc = addr;
            let r = self.fetch(addr, ext).and_then(|insn| self.exec(&insn, ext));
            if let Err(f) = r {
                return Step::Fault(f);
            }
            self.steps += 1;
            self.fetch_stage(true, true);
            return self.end_insn();
        }
    }

    /// The start of a cycle: the timer and interrupt latches tick, and the
    /// loop test of this cycle's fetch will see the status from before
    /// this cycle's instruction, but the loop stack after it (a DO's new
    /// loop applies to the word fetched as it executes, UM Figure 3.8).
    #[inline(always)]
    fn begin_cycle(&mut self) {
        self.tick();
        self.cycles += 1;
        self.fetch_status = (self.astat(), self.mode1);
    }

    /// The fetch stage, after the cycle's instruction (if any) executed. A
    /// non-delayed branch still fetches (and tests a loop end) before
    /// squashing the word. `redirect` and `rebuild` say whether the
    /// instruction could have branched or started a loop (they are false
    /// for bubbles, and recompiled code knows them per instruction).
    #[inline(always)]
    fn fetch_stage(&mut self, redirect: bool, rebuild: bool) {
        let fetched = self.faddr;
        let (succ, looped) = self.next_after(fetched);
        self.exec_slot = self.decode_slot;
        self.decode_slot = Some((fetched, looped));
        self.faddr = succ;
        if redirect {
            if let Some((target, delayed)) = self.redirect.take() {
                self.faddr = target & 0xff_ffff;
                if !delayed {
                    self.exec_slot = None;
                    self.decode_slot = None;
                }
            }
        }
        if rebuild && std::mem::take(&mut self.rebuild) {
            // A one-instruction loop: its only word was fetched before
            // the loop existed; fetch again through the loop logic.
            if let Some((first, _)) = self.exec_slot {
                let (second, l1) = self.next_after(first);
                let (next, l2) = self.next_after(second);
                self.exec_slot = Some((first, l1));
                self.decode_slot = Some((second, l2));
                self.faddr = next;
            }
        }
    }

    /// After an instruction: interrupts, unless a branch holds them off.
    #[inline(always)]
    fn end_insn(&mut self) -> Step {
        if self.irq_hold > 0 {
            self.irq_hold -= 1;
        } else if let Err(f) = self.take_interrupt() {
            return Step::Fault(f);
        }
        Step::Ran
    }

    /// How many times the internal word at `addr` has executed.
    pub fn count_at(&self, addr: u32) -> Option<u64> {
        word_index(addr).map(|k| self.counts[k])
    }

    /// True while the core spins on a flag input: the last instruction
    /// was a non-delayed `IF [NOT] FLAGn_IN JUMP` to itself, with no
    /// compute, and it was taken. Until a FLAG input changes (or an
    /// interrupt is latched that can be taken), stepping only repeats it,
    /// so a host may stop stepping. The timer must be off: it would count
    /// meanwhile.
    /// Also true inside a `DO ... UNTIL [NOT] FLAGn_IN` (or `FOREVER`)
    /// loop of at most four NOPs.
    pub fn spinning(&self) -> bool {
        if self.idle || self.mode2 & TIMEN != 0 || self.pending_irq().is_some() {
            return false;
        }
        // (A NOP loop's words are all NOPs, so the two cases don't overlap.)
        if self.nop_loop() {
            return true;
        }
        if self.exec_slot.is_some() || self.decode_slot.is_some() || self.faddr != self.pc {
            return false;
        }
        // Recompiled code doesn't fill the decode cache.
        let Some(k) = word_index(self.pc) else { return false };
        let insn = self.decoded[k].unwrap_or_else(|| decode(self.pc, self.mem.read48(self.pc)));
        matches!(insn.op, Op::Branch {
                cond: 9..=12 | 25..=28,
                call: false,
                target,
                delayed: false,
                loop_abort: false,
                compute: crate::Compute::None,
                ..
            } if target.resolve(self.pc) == Some(self.pc))
    }

    /// The innermost loop is a short run of NOPs ending on a flag input
    /// (or never), and the pipeline holds only its words.
    fn nop_loop(&self) -> bool {
        let Some(&(laddr, _)) = self.loop_stack.last() else { return false };
        let term = laddr_term(laddr) & 31;
        if laddr_counter(laddr) || !matches!(term, 9..=12 | 25..=28 | TRUE) {
            return false;
        }
        let (Some(&start), end) = (self.pc_stack.last(), laddr_end(laddr)) else { return false };
        if end < start || end - start >= 4 {
            return false;
        }
        let body = start..=end;
        let in_body = |s: Slot| s.is_none_or(|(a, _)| body.contains(&a));
        if !body.contains(&self.pc) || !in_body(self.exec_slot) || !in_body(self.decode_slot) {
            return false;
        }
        body.into_iter().all(|a| decode(a, self.mem.read48(a)).op == Op::Nop)
    }

    /// [`step`](Sharc::step) until the bus's
    /// [`after_step`](Bus::after_step) says to stop, or a step doesn't
    /// run (IDLE or a fault).
    pub fn run_bus<B: Bus>(&mut self, ext: &mut B) -> Step {
        loop {
            let r = self.step(ext);
            if r != Step::Ran || !ext.after_step(self) {
                return r;
            }
        }
    }

    /// Run up to `max` instructions; stops early on a fault or in IDLE.
    pub fn run(&mut self, ext: &mut impl Bus, max: u64) -> Step {
        for _ in 0..max {
            match self.step(ext) {
                Step::Ran => {}
                other => return other,
            }
        }
        Step::Ran
    }

    // --- IOP registers and the host interface ----------------------------------

    fn iop_read(&mut self, addr: u32) -> Result<u32, Fault> {
        self.iop_get(addr).ok_or(Fault::Iop { pc: self.pc, addr, write: false })
    }

    fn iop_write(&mut self, addr: u32, v: u32) -> Result<(), Fault> {
        self.iop_set(addr, v).map_err(|_| Fault::Iop { pc: self.pc, addr, write: true })
    }

    /// An implemented IOP register as either side reads it.
    fn iop_get(&mut self, addr: u32) -> Option<u32> {
        let a = addr as usize;
        Some(match addr {
            SYSCON => self.iop[a] & !(1 << 7),
            VIRPT | WAIT | SYSTAT => self.iop[a],
            EPB0 => {
                let w = self.epb0.pop_front()?;
                self.run_dma();
                w as u32
            }
            0x08..=0x0f | 0x18 | 0x19 | 0x1b => self.iop[a],
            DMAC6 => {
                let fs = match self.epb0.len() {
                    0 => 0,
                    EPB_DEPTH => 3,
                    _ => 2,
                };
                self.iop[a] & !(FLSH | 3 << 14) | fs << 14
            }
            0x1d..=0x1f | 0x40..=0x47 => self.iop[a],
            a if reserved_iop(a) => 0,
            _ => return None,
        })
    }

    /// Write an implemented IOP register; `Err` for anything else, or for
    /// a DMA mode this model doesn't run.
    fn iop_set(&mut self, addr: u32, v: u32) -> Result<(), ()> {
        let a = addr as usize;
        match addr {
            SYSCON => {
                if v & 1 != 0 {
                    // SRST: software reset.
                    self.reset();
                    return Ok(());
                }
                if v & (1 << 7) != 0 {
                    self.host_wlatch = None;
                    self.host_rlatch = None;
                }
                self.iop[a] = v & !(1 << 7);
            }
            VIRPT => {
                // A vector interrupt; ignored while one is being serviced
                // (UM §3.6.10).
                if self.imaskp & (1 << IRQ_VIRPT) == 0 {
                    self.iop[a] = v;
                    self.latch(IRQ_VIRPT);
                }
            }
            WAIT | 0x08..=0x0f | 0x18 | 0x19 | 0x1b => self.iop[a] = v,
            DMAC6 => {
                if v & FLSH != 0 {
                    self.epb0.clear();
                    self.epb0_pack = (0, 0);
                }
                let v = v & 0x1fff & !(3 << 3);
                // Implemented: slave-mode receive (the host writes EPB0),
                // packing modes 00-10.
                if v & DEN != 0 && (v & (CHEN | TRAN | MASTER | HSHAKE) != 0 || v >> 6 & 3 == 3) {
                    return Err(());
                }
                self.iop[a] = v;
                self.run_dma();
            }
            0x1d..=0x1f => {
                if v & DEN != 0 {
                    return Err(());
                }
                self.iop[a] = v;
            }
            0x40..=0x47 => {
                self.iop[a] = v;
                self.run_dma();
            }
            a if reserved_iop(a) => {}
            _ => return Err(()),
        }
        Ok(())
    }

    /// Move words from EPB0 into internal memory while DMA channel 6 is
    /// enabled and its count is nonzero; at zero, latch EP0I (UM §6, §11.6).
    fn run_dma(&mut self) {
        let ctl = self.iop[DMAC6 as usize];
        if ctl & DEN == 0 || ctl & TRAN != 0 {
            return;
        }
        while self.iop[C6 as usize] & 0xffff != 0 {
            let Some(w) = self.epb0.pop_front() else { break };
            let addr = self.iop[II6 as usize];
            if ctl & DTYPE != 0 || self.data48(addr) {
                self.mem.write48(addr, w);
                self.invalidate(addr, true);
            } else {
                self.mem.write32(addr, w as u32);
                self.invalidate(addr, false);
            }
            let step = self.iop[IM6 as usize] as u16 as i16 as i32;
            self.iop[II6 as usize] = addr.wrapping_add(step as u32) & 0x3_ffff;
            self.iop[C6 as usize] = (self.iop[C6 as usize] & 0xffff) - 1;
            if self.iop[C6 as usize] == 0 {
                self.latch(IRQ_EP0);
            }
        }
    }

    /// The host's bus width and word for SYSCON HPM (UM §E.9): true for a
    /// 16-bit bus.
    fn host16(&self) -> bool {
        matches!(self.iop[SYSCON as usize] >> 4 & 3, 1 | 2)
    }

    fn host_msw_first(&self) -> bool {
        self.iop[SYSCON as usize] & (1 << 6) != 0
    }

    /// A host write to IOP register `addr` (the board maps its window onto
    /// these addresses). `data` is one bus transfer: with a 16-bit host bus
    /// (SYSCON HPM 01 or 10, the reset value) two transfers make one
    /// 32-bit register write, least significant half first unless HMSWF.
    /// Writes to EPB0 feed DMA channel 6, packed as DMAC6 PMODE says.
    pub fn host_write(&mut self, addr: u32, data: u32) -> Result<(), HostError> {
        if addr == EPB0 {
            return self.epb0_host_write(data);
        }
        if (0x05..=0x07).contains(&addr) || addr > 0xff {
            return Err(HostError::Unimplemented { addr });
        }
        let v = if self.host16() {
            let half = data as u16;
            match self.host_wlatch.take() {
                None => {
                    self.host_wlatch = Some(half);
                    return Ok(());
                }
                Some(first) if self.host_msw_first() => (first as u32) << 16 | half as u32,
                Some(first) => (half as u32) << 16 | first as u32,
            }
        } else {
            data
        };
        self.iop_set(addr, v).map_err(|_| HostError::Unimplemented { addr })
    }

    fn epb0_host_write(&mut self, data: u32) -> Result<(), HostError> {
        let ctl = self.iop[DMAC6 as usize];
        let (halves, word) = match ctl >> 6 & 3 {
            0 => (1, data as u64),
            1 => (2, data as u64 & 0xffff),
            2 => (3, data as u64 & 0xffff),
            _ => return Err(HostError::Unimplemented { addr: EPB0 }),
        };
        if self.epb0.len() >= EPB_DEPTH {
            return Err(HostError::BufferFull { addr: EPB0 });
        }
        let (acc, n) = self.epb0_pack;
        let acc = if halves == 1 {
            word
        } else if ctl & MSWF != 0 {
            acc << 16 | word
        } else {
            acc | word << (16 * n)
        };
        if n + 1 == halves {
            self.epb0_pack = (0, 0);
            self.epb0.push_back(acc);
            self.run_dma();
        } else {
            self.epb0_pack = (acc, n + 1);
        }
        Ok(())
    }

    /// A host read of IOP register `addr`; with a 16-bit host bus each
    /// register takes two reads, as for `host_write`.
    pub fn host_read(&mut self, addr: u32) -> Result<u32, HostError> {
        if addr > 0xff || addr == EPB0 {
            return Err(HostError::Unimplemented { addr });
        }
        if self.host16() {
            if let Some(second) = self.host_rlatch.take() {
                return Ok(second as u32);
            }
            let v = self.iop_get(addr).ok_or(HostError::Unimplemented { addr })?;
            let (first, second) = if self.host_msw_first() { (v >> 16, v) } else { (v, v >> 16) };
            self.host_rlatch = Some(second as u16);
            return Ok(first & 0xffff);
        }
        self.iop_get(addr).ok_or(HostError::Unimplemented { addr })
    }
}

/// Code of the PX universal register.
const PX: u8 = 0xdb;

/// A value loaded for a register.
#[derive(Clone, Copy, Debug)]
enum Loaded {
    /// The 40-bit DM bus value.
    Bus(u64),
    /// All 48 bits for PX (PM bus or external).
    Px48(u64),
}

/// Save states: everything but the execution counts; the decode cache is
/// rebuilt from memory.
impl state::State for Sharc {
    fn save(&self, w: &mut state::Writer) {
        self.mem.cols[0].save(w);
        self.mem.cols[1].save(w);
        self.r.save(w);
        self.r_alt.save(w);
        for v in [&self.i, &self.m, &self.l, &self.b] {
            v.save(w);
        }
        self.dag_alt.save(w);
        for mr in self.mr {
            (mr as u64, (mr >> 64) as u64).save(w);
        }
        self.px.save(w);
        [
            self.mode1,
            self.mode2,
            self.astat,
            self.stky,
            self.irptl,
            self.imask,
            self.imaskp,
            self.ustat1,
            self.ustat2,
            self.tperiod,
            self.tcount,
            self.lcntr,
            self.pc,
            self.faddr,
        ]
        .save(w);
        self.pc_stack.save(w);
        self.loop_stack.save(w);
        self.status_stack.save(w);
        self.exec_slot.save(w);
        self.decode_slot.save(w);
        self.reentry.save(w);
        self.last_loopback.save(w);
        self.irq_reentry.save(w);
        self.irq_hold.save(w);
        self.idle.save(w);
        self.booting.save(w);
        self.iop.save(w);
        self.epb0.save(w);
        self.epb0_pack.save(w);
        self.host_wlatch.save(w);
        self.host_rlatch.save(w);
        self.irq_lines.save(w);
        self.flag_in.save(w);
        self.cycles.save(w);
        self.steps.save(w);
    }

    #[inline(always)]
    fn load(&mut self, r: &mut state::Reader) -> Result<(), state::Error> {
        self.load_fields(r)?;
        self.redirect = None;
        self.rebuild = false;
        self.decoded.iter_mut().for_each(|d| *d = None);
        // Memory changed under any recompiled code: the host attaches it
        // again.
        self.code.clear();
        self.code_dirty = false;
        Ok(())
    }
}

impl Sharc {
    /// The saved fields ([`state::State::load`] without resetting what's
    /// derived from them).
    fn load_fields(&mut self, r: &mut state::Reader) -> Result<(), state::Error> {
        use state::State as _;
        self.mem.cols[0].load(r)?;
        self.mem.cols[1].load(r)?;
        self.r.load(r)?;
        self.r_alt.load(r)?;
        for v in [&mut self.i, &mut self.m, &mut self.l, &mut self.b] {
            v.load(r)?;
        }
        self.dag_alt.load(r)?;
        for k in 0..2 {
            let mut p = (0u64, 0u64);
            p.load(r)?;
            self.mr[k] = ((p.1 as u128) << 64 | p.0 as u128) as i128;
        }
        self.px.load(r)?;
        let mut regs = [0u32; 14];
        regs.load(r)?;
        [
            self.mode1,
            self.mode2,
            self.astat,
            self.stky,
            self.irptl,
            self.imask,
            self.imaskp,
            self.ustat1,
            self.ustat2,
            self.tperiod,
            self.tcount,
            self.lcntr,
            self.pc,
            self.faddr,
        ] = regs;
        self.pc_stack.load(r)?;
        self.loop_stack.load(r)?;
        self.status_stack.load(r)?;
        self.exec_slot.load(r)?;
        self.decode_slot.load(r)?;
        self.reentry.load(r)?;
        self.last_loopback.load(r)?;
        self.irq_reentry.load(r)?;
        self.irq_hold.load(r)?;
        self.idle.load(r)?;
        self.booting.load(r)?;
        self.iop.load(r)?;
        self.epb0.load(r)?;
        self.epb0_pack.load(r)?;
        self.host_wlatch.load(r)?;
        self.host_rlatch.load(r)?;
        self.irq_lines.load(r)?;
        self.flag_in.load(r)?;
        self.cycles.load(r)?;
        self.steps.load(r)?;
        Ok(())
    }

    /// Make this core's state `src`'s, as saving `src`'s state and loading
    /// it here would, but keep what this core has decoded where the
    /// instruction words are the same (a load forgets all of it), and take
    /// `src`'s recompiled-code watch with its memory (which a load leaves
    /// for the host to attach again). `buf` is kept for the state between
    /// calls. For run-ahead's copy of a machine.
    pub fn copy_from(&mut self, src: &Sharc, buf: &mut Vec<u8>) {
        use state::State as _;
        let words = mem::WORDS48 as usize;
        for k in 0..2 * words {
            if self.decoded[k].is_none() {
                continue;
            }
            let (block, o) = (k / words, (k % words) as u32);
            let addr = if block == 0 { mem::BLOCK0 } else { mem::BLOCK1 } + o;
            if self.mem.read48(addr) != src.mem.read48(addr) {
                self.decoded[k] = None;
            }
        }
        let mut w = state::Writer { buf: std::mem::take(buf) };
        w.buf.clear();
        src.save(&mut w);
        self.load_fields(&mut state::Reader::new(&w.buf)).expect("a core's own state loads");
        *buf = w.buf;
        self.redirect = src.redirect;
        self.rebuild = src.rebuild;
        self.fetch_status = src.fetch_status;
        self.code.clone_from(&src.code);
        self.code_dirty = src.code_dirty;
    }
}

#[cfg(test)]
mod tests;
