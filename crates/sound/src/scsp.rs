//! Yamaha YMF292 "SCSP" (Sega 315-5687): 32 PCM/FM slots, timers,
//! interrupts and a MIDI port, run one 44.1 kHz output sample at a time.
//!
//! Written from the SCSP User's Manual with gaps filled from the facts in
//! `docs/hw/sound.md` (which cites MAME, Supermodel and Mednafen). Where they
//! disagree the manual wins and the comment says so.
//!
//! Register addresses are byte offsets into the 4 KB register block, as the
//! sound CPU sees it at `0x100000`. Sound RAM is held big-endian (the 68000's
//! view) and the SCSP reads 16-bit PCM from it as big-endian words.

use crate::dsp::Dsp;
use std::collections::VecDeque;

/// Sound RAM size (MEM4MB = 1: 512 KB).
pub const RAM_SIZE: usize = 0x80000;
const RAM_MASK: u32 = RAM_SIZE as u32 - 1;
/// MIDI input FIFO depth (manual: 4 bytes; MAME uses 32, see MD-13).
pub const MIDI_FIFO: usize = 4;

/// Interrupt source bits in SCIEB/SCIPD/SCIRE.
pub mod int {
    pub const MIDI_IN: u16 = 1 << 3;
    pub const DMA: u16 = 1 << 4;
    pub const CPU: u16 = 1 << 5;
    pub const TIMER_A: u16 = 1 << 6;
    pub const MIDI_OUT: u16 = 1 << 9;
    pub const SAMPLE: u16 = 1 << 10;
}

/// Envelope phase, numbered as the SGC monitor bits report it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Phase {
    Attack = 0,
    Decay1 = 1,
    Decay2 = 2,
    #[default]
    Release = 3,
}

/// EG attenuation at key-on: about -60 dB (Mednafen; MAME agrees).
const EG_START: u16 = 0x280;
/// Silent from this attenuation on (Mednafen).
const EG_SILENT: u16 = 0x3c0;

/// One slot: its 16 register words (offsets 0x00-0x1E) and running state.
#[derive(Clone, Copy, Default)]
pub struct Slot {
    pub regs: [u16; 16],
    /// The key as the last KYONEX latched it from KYONB. Key-on and key-off
    /// happen on its edges only, so a slot that played a loop-off sample to
    /// its end stays keyed on (silent) until the program keys it off.
    pub keyed: bool,
    /// Generating output (key-on until release fades or loop-off ends).
    pub active: bool,
    pub phase: Phase,
    /// EG attenuation, 0 = loudest, 0x3FF = silent; 3/32 dB per step.
    pub eg: u16,
    /// Sample offset from SA (16 bits) and its 14-bit fraction.
    pub pos: u32,
    frac: u32,
    backward: bool,
    lfo: u8,
    lfo_div: u32,
}

impl Slot {
    fn kyonb(&self) -> bool {
        self.regs[0] & 0x0800 != 0
    }
    fn sbctl(&self) -> u16 {
        self.regs[0] >> 9 & 3
    }
    fn ssctl(&self) -> u16 {
        self.regs[0] >> 7 & 3
    }
    fn lpctl(&self) -> u16 {
        self.regs[0] >> 5 & 3
    }
    fn pcm8(&self) -> bool {
        self.regs[0] & 0x10 != 0
    }
    /// Start address in bytes (20 bits).
    pub fn sa(&self) -> u32 {
        (self.regs[0] as u32 & 0xf) << 16 | self.regs[1] as u32
    }
    fn lsa(&self) -> u32 {
        self.regs[2] as u32
    }
    fn lea(&self) -> u32 {
        self.regs[3] as u32
    }
    fn ar(&self) -> u16 {
        self.regs[4] & 0x1f
    }
    fn eghold(&self) -> bool {
        self.regs[4] & 0x20 != 0
    }
    fn d1r(&self) -> u16 {
        self.regs[4] >> 6 & 0x1f
    }
    fn d2r(&self) -> u16 {
        self.regs[4] >> 11
    }
    fn rr(&self) -> u16 {
        self.regs[5] & 0x1f
    }
    fn dl(&self) -> u16 {
        self.regs[5] >> 5 & 0x1f
    }
    fn krs(&self) -> u16 {
        self.regs[5] >> 10 & 0xf
    }
    fn lpslnk(&self) -> bool {
        self.regs[5] & 0x4000 != 0
    }
    fn tl(&self) -> u16 {
        self.regs[6] & 0xff
    }
    fn sdir(&self) -> bool {
        self.regs[6] & 0x100 != 0
    }
    fn stwinh(&self) -> bool {
        self.regs[6] & 0x200 != 0
    }
    fn mdl(&self) -> u16 {
        self.regs[7] >> 12
    }
    fn mdxsl(&self) -> usize {
        (self.regs[7] >> 6 & 0x3f) as usize
    }
    fn mdysl(&self) -> usize {
        (self.regs[7] & 0x3f) as usize
    }
    /// Signed octave, -8..7.
    fn oct(&self) -> i32 {
        ((self.regs[8] >> 11 & 0xf) as i32 ^ 8) - 8
    }
    fn fns(&self) -> u32 {
        self.regs[8] as u32 & 0x3ff
    }
    fn lfore(&self) -> bool {
        self.regs[9] & 0x8000 != 0
    }
    fn lfof(&self) -> u32 {
        (self.regs[9] >> 10 & 0x1f) as u32
    }
    fn plfows(&self) -> u16 {
        self.regs[9] >> 8 & 3
    }
    fn plfos(&self) -> usize {
        (self.regs[9] >> 5 & 7) as usize
    }
    fn alfows(&self) -> u16 {
        self.regs[9] >> 3 & 3
    }
    fn alfos(&self) -> u32 {
        (self.regs[9] & 7) as u32
    }
    fn isel(&self) -> usize {
        (self.regs[10] >> 3 & 0xf) as usize
    }
    fn imxl(&self) -> u32 {
        (self.regs[10] & 7) as u32
    }
    fn disdl(&self) -> u32 {
        (self.regs[11] >> 13) as u32
    }
    fn dipan(&self) -> u32 {
        (self.regs[11] >> 8 & 0x1f) as u32
    }
    fn efsdl(&self) -> u32 {
        (self.regs[11] >> 5 & 7) as u32
    }
    fn efpan(&self) -> u32 {
        (self.regs[11] & 0x1f) as u32
    }

    /// Key scaling added to every EG rate.
    fn key_scale(&self) -> u16 {
        if self.krs() == 0xf {
            0
        } else {
            (self.krs() as i32 + self.oct()).clamp(0, 15) as u16
        }
    }

    fn key_on(&mut self) {
        self.active = true;
        self.phase = Phase::Attack;
        self.pos = 0;
        self.frac = 0;
        self.backward = false;
        // Mednafen: instant when AR + KS >= 0x20. Here also at AR + KS = 0x1F,
        // as in MAME's rate table and the FM-chip convention.
        self.eg = if self.ar() + self.key_scale() >= 0x1f { 0 } else { EG_START };
    }

    /// Phase increment per output sample, 14-bit fraction. `lfo` is the
    /// pitch-LFO multiplier in 1/4096 units.
    fn step(&self, lfo: u32) -> u32 {
        let base = 1024 + self.fns();
        let sh = self.oct() + 4;
        let step = if sh >= 0 { base << sh } else { base >> -sh };
        if lfo == 4096 {
            step
        } else {
            ((step as u64 * lfo as u64) >> 12) as u32
        }
    }

    /// Move the play position by `step` and apply the loop mode.
    fn advance(&mut self, step: u32) {
        let lsa = self.lsa();
        let lea = self.lea().max(lsa);
        let len = lea - lsa;
        let mut p = (self.pos << 14 | self.frac) as i64;
        let (lsa14, lea14) = ((lsa as i64) << 14, (lea as i64) << 14);
        if self.backward {
            p -= step as i64;
            if p <= lsa14 {
                if self.lpctl() == 3 {
                    // Alternating: bounce off LSA and run forward again.
                    p = (lsa14 + (lsa14 - p)).min(lea14);
                    self.backward = false;
                } else if len == 0 {
                    p = lsa14;
                } else {
                    p += ((len as i64) << 14) * (1 + (lsa14 - p) / ((len as i64) << 14));
                }
            }
        } else {
            p += step as i64;
            match self.lpctl() {
                0 if p >= lea14 => {
                    // Loop off: the slot stops at LEA.
                    p = lea14;
                    self.active = false;
                    self.phase = Phase::Release;
                    self.eg = 0x3ff;
                }
                1 if p >= lea14 => {
                    if len == 0 {
                        p = lsa14;
                    } else {
                        p = lsa14 + (p - lsa14) % ((len as i64) << 14);
                    }
                }
                2 if p >= lsa14 => {
                    // Reverse: after the first pass to LSA, loop LEA→LSA backwards.
                    p = (lea14 - (p - lsa14)).max(lsa14);
                    self.backward = true;
                }
                3 if p >= lea14 => {
                    p = (lea14 - (p - lea14)).max(lsa14);
                    self.backward = true;
                }
                _ => {}
            }
        }
        self.pos = (p >> 14) as u32 & 0xffff;
        self.frac = p as u32 & 0x3fff;
    }
}

/// When the EG clocks for effective rate `er` at sample `s`: `Some(k)` gives
/// the step shift, as described in sound.md §2.4 (Mednafen's scheme; MAME
/// uses the manual's time tables with a different octave weighting).
fn eg_clock(er: u16, s: u32) -> Option<u32> {
    if er == 0 {
        return None;
    }
    if er < 0x18 {
        let wbt = ((0x18 - er) >> 1) as u32;
        let m = (2u32 << wbt) - 1;
        let fire = if er & 1 == 0 { s & m == 1 << wbt } else { s & m == 0 && (s >> (wbt + 1)) & 3 != 0 };
        fire.then_some(4)
    } else {
        if s & 1 != 0 {
            return None;
        }
        let e = er.min(0x1e) as u32;
        let k = (0x20 - e) / 2;
        Some(if e & 1 != 0 && s & 2 != 0 { k + 1 } else { k })
    }
}

/// A key-on (`on`) or key-off of `slot` at output sample `sample`, with the
/// slot's registers at that moment.
pub type KeyEvent = (u32, u8, bool, [u16; 12]);

/// One 8-bit up counter timer.
#[derive(Clone, Copy, Default)]
struct Timer {
    ctl: u8,
    count: u8,
}

/// Activity counters for diagnostics.
#[derive(Clone, Default, Debug)]
pub struct ScspStats {
    pub key_ons: u64,
    /// Key-ons per slot.
    pub slot_key_ons: [u64; 32],
    pub midi_in: u64,
    pub midi_overflows: u64,
    pub midi_out: u64,
    pub dma: u64,
    pub dsp_program_writes: u64,
}

pub struct Scsp {
    pub ram: Vec<u8>,
    pub slots: [Slot; 32],
    /// Common registers 0x400-0x42F as written (word index from 0x400).
    common: [u16; 0x18],
    timers: [Timer; 3],
    scieb: u16,
    scipd: u16,
    scilv: [u8; 3],
    mcieb: u16,
    mcipd: u16,
    irq: u8,
    mi: VecDeque<u8>,
    mi_ovf: bool,
    /// Bytes the sound CPU wrote to MOBUF.
    pub midi_out: Vec<u8>,
    /// Sound stack: two generations of the 32 slot outputs.
    stack: [i16; 64],
    pub dsp: Dsp,
    /// Global sample counter.
    pub samples: u32,
    lfsr: u32,
    /// Pitch-LFO multipliers per PLFOS and LFO value, 1/4096 units.
    plfo_table: Vec<u32>,
    pub stats: ScspStats,
    /// When set, every key-on/off.
    pub key_log: Option<Vec<KeyEvent>>,
}

impl Default for Scsp {
    fn default() -> Self {
        Self::new()
    }
}

/// PLFO depth in cents for PLFOS 1-7 (manual p04_26). Mednafen's arithmetic
/// gives about ±190 cents at PLFOS 7 instead of ±494.
const PLFO_CENTS: [f64; 8] = [0.0, 7.0, 13.5, 27.0, 55.0, 112.0, 230.0, 494.0];

impl Scsp {
    pub fn new() -> Self {
        let mut plfo_table = vec![4096; 8 * 256];
        for (s, cents) in PLFO_CENTS.iter().enumerate() {
            for v in 0..256 {
                let x = (v as u8 as i8) as f64 / 128.0;
                plfo_table[s * 256 + v] = (4096.0 * (x * cents / 1200.0).exp2()).round() as u32;
            }
        }
        Scsp {
            ram: vec![0; RAM_SIZE],
            slots: [Slot::default(); 32],
            common: [0; 0x18],
            timers: [Timer::default(); 3],
            scieb: 0,
            scipd: 0,
            scilv: [0; 3],
            mcieb: 0,
            mcipd: 0,
            irq: 0,
            mi: VecDeque::with_capacity(MIDI_FIFO),
            mi_ovf: false,
            midi_out: Vec::new(),
            stack: [0; 64],
            dsp: Dsp::new(),
            samples: 0,
            lfsr: 1,
            plfo_table,
            stats: ScspStats::default(),
            key_log: None,
        }
    }

    /// The interrupt level presented to the sound CPU (0 = none).
    pub fn irq(&self) -> u8 {
        self.irq
    }

    fn update_irq(&mut self) {
        let mut pend = self.scipd & self.scieb;
        if pend & 0x780 != 0 {
            pend = (pend & 0x7f) | 0x80;
        }
        let mut level = 0;
        for b in 0..8 {
            if pend & 1 << b != 0 {
                let l = (self.scilv[0] >> b & 1) | (self.scilv[1] >> b & 1) << 1 | (self.scilv[2] >> b & 1) << 2;
                level = level.max(l);
            }
        }
        self.irq = level;
    }

    fn raise(&mut self, bits: u16) {
        self.scipd |= bits;
        self.mcipd |= bits;
        self.update_irq();
    }

    /// A byte arrives on MIDI-in. Lost (MIOVF set) when the FIFO is full.
    pub fn midi_in(&mut self, b: u8) {
        if self.mi.len() >= MIDI_FIFO {
            self.mi_ovf = true;
            self.stats.midi_overflows += 1;
            return;
        }
        self.mi.push_back(b);
        self.stats.midi_in += 1;
        self.raise(int::MIDI_IN);
    }

    /// Bytes waiting in the MIDI input FIFO.
    pub fn midi_pending(&self) -> usize {
        self.mi.len()
    }

    // ---- registers ----

    /// A common register (0x400-0x42F) as last written, without side effects.
    pub fn common_reg(&self, addr: u32) -> u16 {
        self.common[((addr.clamp(0x400, 0x42e) - 0x400) >> 1) as usize]
    }

    /// Read the register word at `addr` (0-0xFFF). `low` is true when the
    /// access includes the low byte (it pops MIBUF).
    pub fn read_reg(&mut self, addr: u32, low: bool) -> u16 {
        let addr = addr & 0xffe;
        match addr {
            0x000..=0x3ff => {
                let s = &self.slots[(addr >> 5) as usize];
                let w = (addr >> 1 & 0xf) as usize;
                if w < 12 {
                    s.regs[w]
                } else {
                    0
                }
            }
            0x404 => {
                let mut v = 1 << 11; // MOEMP: output is sent at once
                if self.mi_ovf {
                    v |= 1 << 10;
                }
                if self.mi.len() >= MIDI_FIFO {
                    v |= 1 << 9;
                }
                if self.mi.is_empty() {
                    v |= 1 << 8;
                }
                if low {
                    self.mi_ovf = false;
                    v |= self.mi.pop_front().unwrap_or(0) as u16;
                    if self.mi.is_empty() {
                        self.scipd &= !int::MIDI_IN;
                        self.mcipd &= !int::MIDI_IN;
                        self.update_irq();
                    }
                }
                v
            }
            0x406 => 0,
            0x408 => {
                let mslc = self.common[4] >> 11;
                let s = &self.slots[mslc as usize];
                mslc << 11 | ((s.pos >> 12) as u16 & 0xf) << 7 | (s.phase as u16) << 5 | s.eg >> 5
            }
            0x416 => self.common[0xb] & !0x1000,
            0x418 | 0x41a | 0x41c => {
                let t = self.timers[((addr - 0x418) >> 1) as usize];
                (t.ctl as u16) << 8 | t.count as u16
            }
            0x41e => self.scieb,
            0x420 => self.scipd,
            0x422 | 0x42e => 0,
            0x424..=0x428 => self.scilv[((addr - 0x424) >> 1) as usize] as u16,
            0x42a => self.mcieb,
            0x42c => self.mcipd,
            0x400..=0x42f => self.common[((addr - 0x400) >> 1) as usize],
            0x600..=0x67f => self.stack[((addr - 0x600) >> 1) as usize] as u16,
            0x700..=0xeff => self.dsp.read(addr),
            _ => 0,
        }
    }

    /// Write `v` to the register word at `addr`; only the bits in `mask`
    /// (0xFF00 = even byte, 0x00FF = odd byte) are written.
    pub fn write_reg(&mut self, addr: u32, v: u16, mask: u16) {
        let addr = addr & 0xffe;
        let v = v & mask;
        let merge = |old: u16| old & !mask | v;
        match addr {
            0x000..=0x3ff => {
                let n = (addr >> 5) as usize;
                let w = (addr >> 1 & 0xf) as usize;
                let s = &mut self.slots[n];
                s.regs[w] = merge(s.regs[w]);
                if w == 0 {
                    s.regs[0] &= !0x1000;
                    if v & 0x1000 != 0 {
                        self.key_on_execute();
                    }
                }
            }
            0x406 => {
                if mask & 0xff != 0 {
                    self.midi_out.push(v as u8);
                    self.stats.midi_out += 1;
                }
            }
            0x416 => {
                self.common[0xb] = merge(self.common[0xb]);
                if v & 0x1000 != 0 {
                    self.dma();
                }
            }
            0x418 | 0x41a | 0x41c => {
                let t = &mut self.timers[((addr - 0x418) >> 1) as usize];
                if mask & 0xff00 != 0 {
                    t.ctl = (v >> 8) as u8 & 7;
                }
                if mask & 0xff != 0 {
                    t.count = v as u8;
                }
            }
            0x41e => {
                self.scieb = merge(self.scieb) & 0x7ff;
                self.update_irq();
            }
            0x420 => {
                if v & int::CPU != 0 {
                    self.scipd |= int::CPU;
                    self.update_irq();
                }
            }
            0x422 => {
                self.scipd &= !v;
                self.update_irq();
            }
            0x424..=0x428 => {
                let i = ((addr - 0x424) >> 1) as usize;
                self.scilv[i] = merge(self.scilv[i] as u16) as u8;
                self.update_irq();
            }
            0x42a => self.mcieb = merge(self.mcieb) & 0x7ff,
            0x42c => self.mcipd |= v & int::CPU,
            0x42e => self.mcipd &= !v,
            0x400..=0x42f => {
                let i = ((addr - 0x400) >> 1) as usize;
                self.common[i] = merge(self.common[i]);
            }
            0x600..=0x67f => {
                let i = ((addr - 0x600) >> 1) as usize;
                self.stack[i] = merge(self.stack[i] as u16) as i16;
            }
            0x700..=0xeff => {
                if (0x800..0xc00).contains(&addr) {
                    self.stats.dsp_program_writes += 1;
                }
                self.dsp.write(addr, v, mask)
            }
            _ => {}
        }
    }

    /// KYONEX: latch every slot's KYONB as its key. A slot keys on when its
    /// key goes from off to on and releases when it goes from on to off; a
    /// slot whose key stays on is left alone, even if its loop-off sample
    /// has ended. (Re-keying ended slots on every KYONEX replayed srallyc's
    /// streamed voices from stale buffer data: `docs/hw/sound.md` §2.1.)
    fn key_on_execute(&mut self) {
        for (n, s) in self.slots.iter_mut().enumerate() {
            let on = s.kyonb();
            if on == s.keyed {
                continue;
            }
            s.keyed = on;
            if on {
                s.key_on();
                self.stats.key_ons += 1;
                self.stats.slot_key_ons[n] += 1;
            } else if s.active {
                s.phase = Phase::Release;
            } else {
                continue;
            }
            if let Some(log) = &mut self.key_log {
                log.push((self.samples, n as u8, on, s.regs[..12].try_into().unwrap()));
            }
        }
    }

    /// DMA between sound RAM and the registers, done at once.
    fn dma(&mut self) {
        self.stats.dma += 1;
        let mem = ((self.common[0xa] as u32 >> 12) << 16 | self.common[9] as u32) & 0xffffe;
        let reg = self.common[0xa] as u32 & 0xffe;
        let ctl = self.common[0xb];
        let words = (ctl as u32 & 0xffe) >> 1;
        let (gate, to_mem) = (ctl & 0x4000 != 0, ctl & 0x2000 != 0);
        for i in 0..words {
            let m = (mem + 2 * i) as usize & (RAM_SIZE - 2);
            let r = (reg + 2 * i) & 0xffe;
            if to_mem {
                let w = if gate { 0 } else { self.read_reg(r, false) };
                self.ram[m..m + 2].copy_from_slice(&w.to_be_bytes());
            } else {
                let w = if gate { 0 } else { u16::from_be_bytes([self.ram[m], self.ram[m + 1]]) };
                self.write_reg(r, w, 0xffff);
            }
        }
        self.common[0xb] &= !0x1000;
        self.raise(int::DMA);
    }

    // ---- sample generation ----

    fn ram16(&self, a: u32) -> i16 {
        let a = (a & RAM_MASK & !1) as usize;
        i16::from_be_bytes([self.ram[a], self.ram[a + 1]])
    }

    /// The slot's waveform at sample offset `i` from SA.
    fn fetch(&self, s: &Slot, i: u32) -> i32 {
        let v = match s.ssctl() {
            0 if s.pcm8() => (self.ram[((s.sa() + i) & RAM_MASK) as usize] as i8 as i16) << 8,
            0 => self.ram16(s.sa() + 2 * i),
            1 => self.lfsr as i16,
            _ => 0,
        };
        let x = ((s.sbctl() & 1) * 0x7fff) | ((s.sbctl() & 2) << 14);
        (v ^ x as i16) as i32
    }

    /// LFO waveform `ws` at counter `c`: (amplitude 0..255, pitch -128..127).
    fn lfo_wave(&self, ws: u16, c: u8) -> (u32, i32) {
        match ws {
            0 => (c as u32, c as i8 as i32),
            1 => {
                if c < 0x80 {
                    (0, 127)
                } else {
                    (0xff, -128)
                }
            }
            2 => {
                let a = if c < 0x80 { c as u32 * 2 } else { (0xff - c as u32) * 2 };
                let q = (c & 0x3f) as i32 * 2;
                let p = match c >> 6 {
                    0 => q,
                    1 => 127 - q,
                    2 => -q,
                    _ => q - 128,
                };
                (a, p)
            }
            _ => {
                let n = (self.lfsr >> 3) as u8;
                (n as u32, n as i8 as i32)
            }
        }
    }

    /// Advance the chip by one output sample and return (left, right).
    pub fn sample(&mut self) -> (i16, i16) {
        let s = self.samples;
        let (mut l, mut r) = (0i32, 0i32);
        self.dsp.clear_inputs();
        for n in 0..32 {
            let out = self.slot_sample(n, s);
            let slot = &self.slots[n];
            if !slot.stwinh() {
                self.stack[(s.wrapping_mul(32) as usize + n) & 63] = out as i16;
            }
            if out != 0 {
                if slot.disdl() != 0 {
                    let (gl, gr) = pan_gains(slot.disdl(), slot.dipan());
                    l += (out * gl) >> 14;
                    r += (out * gr) >> 14;
                }
                if slot.imxl() != 0 {
                    self.dsp.mix(slot.isel(), (out << 4) >> (7 - slot.imxl()));
                }
            }
        }
        if self.dsp.enabled() {
            let rbp = (self.common[1] & 0x7f) as u32 * 0x1000;
            let rbl = (self.common[1] >> 7 & 3) as u32;
            self.dsp.run(&mut self.ram, rbp, rbl);
            for n in 0..18 {
                let slot = &self.slots[n];
                if slot.efsdl() == 0 {
                    continue;
                }
                let v = if n < 16 { self.dsp.efreg[n] } else { 0 };
                let (gl, gr) = pan_gains(slot.efsdl(), slot.efpan());
                l += (v * gl) >> 14;
                r += (v * gr) >> 14;
            }
        }
        // Master volume (MVOL 15 = unity; MAME uses MVOL/15 linearly).
        let mvol = self.common[0] & 0xf;
        let mv = if mvol == 0 {
            0
        } else {
            let m = 2i32 << (mvol >> 1);
            if mvol & 1 == 0 {
                m * 3 / 4
            } else {
                m
            }
        };
        let l = ((l * mv) >> 8).clamp(-32768, 32767) as i16;
        let r = ((r * mv) >> 8).clamp(-32768, 32767) as i16;
        self.tick(s);
        self.samples = s.wrapping_add(1);
        (l, r)
    }

    /// Timers, the noise LFSR and the per-sample interrupt sources.
    fn tick(&mut self, s: u32) {
        let mut bits = int::SAMPLE | int::MIDI_OUT;
        for (i, t) in self.timers.iter_mut().enumerate() {
            let m = (1 << t.ctl) - 1;
            if s & m == m {
                t.count = t.count.wrapping_add(1);
                if t.count == 0xff {
                    bits |= int::TIMER_A << i;
                }
            }
        }
        if !self.mi.is_empty() {
            bits |= int::MIDI_IN;
        }
        self.lfsr = self.lfsr >> 1 | ((self.lfsr ^ self.lfsr >> 5) & 1) << 16;
        self.raise(bits);
    }

    /// One slot's output for this sample, after EG/TL/ALFO, before pan.
    fn slot_sample(&mut self, n: usize, s: u32) -> i32 {
        let mut slot = self.slots[n];
        if !slot.active {
            return 0;
        }
        // LFO.
        if slot.lfore() {
            slot.lfo = 0;
            slot.lfo_div = 0;
        } else {
            slot.lfo_div += 1;
            let f = slot.lfof();
            let period = (((8 - (f & 3)) << 7) >> (f >> 2)) - 4;
            if slot.lfo_div >= period {
                slot.lfo_div = 0;
                slot.lfo = slot.lfo.wrapping_add(1);
            }
        }
        let alfo = if slot.alfos() != 0 { self.lfo_wave(slot.alfows(), slot.lfo).0 >> (7 - slot.alfos()) } else { 0 };
        let plfo = if slot.plfos() != 0 {
            let p = self.lfo_wave(slot.plfows(), slot.lfo).1;
            self.plfo_table[slot.plfos() * 256 + (p as u8) as usize]
        } else {
            4096
        };

        // Waveform read with FM offset and linear interpolation.
        let mut p = (slot.pos << 14 | slot.frac) as i64;
        if slot.mdl() > 4 {
            let base = (s.wrapping_mul(32) as usize + n) & 63;
            let x = self.stack[(base + slot.mdxsl()) & 63] as i32;
            let y = self.stack[(base + slot.mdysl()) & 63] as i32;
            let m64 = ((x + y) << 6) >> (16 - slot.mdl());
            p += (m64 as i64) << 8;
        }
        let i = (p >> 14) as u32 & 0xffff;
        let f = (p as u32 & 0x3fff) >> 8;
        let s0 = self.fetch(&slot, i);
        let next = if i >= slot.lea() { i } else { i + 1 };
        let s1 = if f != 0 { self.fetch(&slot, next) } else { s0 };
        let smp = s0 + (((s1 - s0) * f as i32) >> 6);

        slot.advance(slot.step(plfo));
        if slot.phase == Phase::Attack && slot.lpslnk() && slot.pos >= slot.lsa() {
            slot.phase = Phase::Decay1;
        }
        self.envelope(&mut slot, s);

        let out = if slot.sdir() {
            smp
        } else {
            let eg = if slot.eghold() && slot.phase == Phase::Attack { 0 } else { slot.eg as u32 };
            let v = (eg + slot.tl() as u32 * 4 + alfo).min(0x3ff);
            if slot.eg >= EG_SILENT {
                0
            } else {
                (smp * (0x7f - (v & 0x3f)) as i32) >> ((v >> 6) + 7)
            }
        };
        if slot.phase == Phase::Release && slot.eg >= EG_SILENT {
            slot.active = false;
        }
        self.slots[n] = slot;
        out
    }

    fn envelope(&self, slot: &mut Slot, s: u32) {
        let rate = match slot.phase {
            Phase::Attack => slot.ar(),
            Phase::Decay1 => slot.d1r(),
            Phase::Decay2 => slot.d2r(),
            Phase::Release => slot.rr(),
        };
        let er = if rate == 0 { 0 } else { (rate + slot.key_scale()).min(0x1f) };
        if let Some(k) = eg_clock(er, s) {
            if slot.phase == Phase::Attack {
                let d = (slot.eg as u32 + 1).div_ceil(1 << k);
                slot.eg = slot.eg.saturating_sub(d as u16);
            } else {
                slot.eg = (slot.eg + (16 >> k)).min(0x3ff);
            }
        }
        match slot.phase {
            Phase::Attack if slot.eg == 0 && !slot.lpslnk() => slot.phase = Phase::Decay1,
            Phase::Decay1 if slot.eg >> 5 >= slot.dl() => slot.phase = Phase::Decay2,
            _ => {}
        }
    }
}

/// Left/right gains (0x4000 = unity) for send level `sdl` and pan `pan`.
fn pan_gains(sdl: u32, pan: u32) -> (i32, i32) {
    let g = 0x80i32 << sdl;
    let att = if pan & 0xf == 0xf {
        0
    } else {
        let a = g >> ((pan & 0xf) >> 1);
        if pan & 1 != 0 {
            a * 3 / 4
        } else {
            a
        }
    };
    if pan & 0x10 == 0 {
        (att, g)
    } else {
        (g, att)
    }
}

// Save states. The PLFO table is constant; statistics and the key log are
// diagnostics.
state::fields!(Scsp {
    ram,
    slots,
    common,
    timers,
    scieb,
    scipd,
    scilv,
    mcieb,
    mcipd,
    irq,
    mi,
    mi_ovf,
    midi_out,
    stack,
    dsp,
    samples,
    lfsr
});
state::fields!(Slot { regs, keyed, active, phase, eg, pos, frac, backward, lfo, lfo_div });
state::fields!(Timer { ctl, count });

state::byte_enum!(Phase, "SCSP envelope phase", { 0 => Attack, 1 => Decay1, 2 => Decay2, 3 => Release });

#[cfg(test)]
mod tests {
    use super::*;

    /// A slot set up on `n` to play a looped 16-bit square wave at RAM 0.
    fn setup(c: &mut Scsp, n: u32, lpctl: u16, lsa: u16, lea: u16) {
        for i in 0..0x1000usize {
            let v: i16 = if (i / 8) % 2 == 0 { 0x4000 } else { -0x4000 };
            c.ram[2 * i..2 * i + 2].copy_from_slice(&v.to_be_bytes());
        }
        let b = n * 0x20;
        c.write_reg(0x400, 0x020f, 0xffff);
        c.write_reg(b, lpctl << 5, 0xffff);
        c.write_reg(b + 2, 0, 0xffff);
        c.write_reg(b + 4, lsa, 0xffff);
        c.write_reg(b + 6, lea, 0xffff);
        c.write_reg(b + 8, 0x001f, 0xffff); // AR max, no decay
        c.write_reg(b + 0xa, 0x3c00 | 0x1f, 0xffff); // KRS off, RR max
        c.write_reg(b + 0x16, 0xe000, 0xffff); // DISDL 7, centre
    }

    fn key(c: &mut Scsp, n: u32, on: bool) {
        let b = n * 0x20;
        let r = c.read_reg(b, false);
        c.write_reg(b, (r & !0x0800) | if on { 0x1800 } else { 0x1000 }, 0xffff);
    }

    #[test]
    fn pitch_step() {
        let mut s = Slot::default();
        assert_eq!(s.step(4096), 1 << 14);
        s.regs[8] = 1 << 11; // OCT +1
        assert_eq!(s.step(4096), 2 << 14);
        s.regs[8] = 0xf << 11 | 0x2ba; // OCT -1, FNS for A4
        assert_eq!(s.step(4096), (1024 + 0x2ba) << 3);
        s.regs[8] = 0x8 << 11; // OCT -8
        assert_eq!(s.step(4096), 1024 >> 4);
    }

    #[test]
    fn plays_and_loops() {
        let mut c = Scsp::new();
        setup(&mut c, 0, 1, 16, 32);
        key(&mut c, 0, true);
        let mut out = Vec::new();
        for _ in 0..200 {
            out.push(c.sample().0);
        }
        // Instant attack, so full level at once; loops between 16 and 32.
        assert!(out[1] > 0x3000 && out[9] < -0x3000, "{:?}", &out[..12]);
        assert!(c.slots[0].active);
        assert!((16..=32).contains(&c.slots[0].pos));
        assert_eq!(c.stats.key_ons, 1);
    }

    #[test]
    fn loop_off_stops() {
        let mut c = Scsp::new();
        setup(&mut c, 3, 0, 0, 40);
        key(&mut c, 3, true);
        for _ in 0..41 {
            c.sample();
        }
        assert!(!c.slots[3].active);
        assert_eq!(c.sample(), (0, 0));
    }

    /// Key-on is an edge of the latched key: a KYONEX for another slot
    /// leaves a slot whose loop-off sample ended (KYONB still set) silent,
    /// and only a key-off then a key-on plays it again.
    #[test]
    fn ended_slot_stays_keyed() {
        let mut c = Scsp::new();
        setup(&mut c, 3, 0, 0, 40);
        key(&mut c, 3, true);
        for _ in 0..50 {
            c.sample();
        }
        assert!(!c.slots[3].active);
        setup(&mut c, 5, 1, 0, 40);
        key(&mut c, 5, true);
        assert!(!c.slots[3].active, "another slot's KYONEX re-keyed an ended slot");
        assert!(c.slots[5].active);
        assert_eq!(c.stats.key_ons, 2);
        // Keying slot 5 on again while its key is on changes nothing.
        key(&mut c, 5, true);
        assert_eq!(c.stats.key_ons, 2);
        key(&mut c, 3, false);
        key(&mut c, 3, true);
        assert!(c.slots[3].active);
        assert_eq!(c.slots[3].pos, 0);
        assert_eq!(c.stats.key_ons, 3);
    }

    #[test]
    fn reverse_and_alternate_loops() {
        let mut c = Scsp::new();
        setup(&mut c, 0, 2, 10, 20);
        key(&mut c, 0, true);
        let mut seen = Vec::new();
        for _ in 0..60 {
            c.sample();
            seen.push(c.slots[0].pos);
        }
        // Forward to LSA, then LEA→LSA backwards.
        assert_eq!(&seen[..9], &[1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert!(c.slots[0].backward && seen[10..].iter().all(|&p| (10..=20).contains(&p)));
        setup(&mut c, 1, 3, 10, 20);
        c.write_reg(0x20 + 0x16, 0, 0xffff);
        key(&mut c, 1, true);
        let mut seen = Vec::new();
        for _ in 0..40 {
            c.sample();
            seen.push(c.slots[1].pos);
        }
        assert_eq!(seen[19], 20);
        assert_eq!(seen[20], 19);
        assert_eq!(seen[29], 10);
        assert_eq!(seen[30], 11);
    }

    #[test]
    fn envelope_attack_decay_release() {
        let mut c = Scsp::new();
        setup(&mut c, 0, 1, 0, 64);
        // AR 0x18, D1R 0x18 to DL 8, D2R 0 (sustain), RR 0x1c.
        c.write_reg(8, 0x18 << 6 | 0x18, 0xffff);
        c.write_reg(0xa, 0x3c00 | 8 << 5 | 0x1c, 0xffff);
        key(&mut c, 0, true);
        assert_eq!(c.slots[0].eg, EG_START);
        let mut n = 0;
        while c.slots[0].phase == Phase::Attack {
            c.sample();
            n += 1;
            assert!(n < 1000, "attack never finished");
        }
        // Exponential attack at 1/16 per clock, a clock every 2 samples.
        assert!((100..400).contains(&n), "attack took {n} samples");
        while c.slots[0].phase == Phase::Decay1 {
            c.sample();
        }
        assert_eq!(c.slots[0].eg >> 5, 8);
        let held = c.slots[0].eg;
        for _ in 0..5000 {
            c.sample();
        }
        assert_eq!(c.slots[0].eg, held, "D2R = 0 sustains");
        key(&mut c, 0, false);
        assert_eq!(c.slots[0].phase, Phase::Release);
        for _ in 0..2000 {
            c.sample();
        }
        assert!(!c.slots[0].active);
    }

    #[test]
    fn eg_rate_ordering() {
        // Clocks per 2^16 samples rise with the rate.
        let count = |er| (0..1u32 << 16).filter(|&s| eg_clock(er, s).is_some()).count();
        let counts: Vec<_> = (1..0x18).map(count).collect();
        assert!(counts.windows(2).all(|w| w[0] < w[1]), "{counts:?}");
        assert_eq!(count(0), 0);
        assert_eq!(count(0x18), 1 << 15);
    }

    #[test]
    fn total_level_attenuates() {
        let mut c = Scsp::new();
        setup(&mut c, 0, 1, 0, 64);
        c.write_reg(0xc, 16, 0xffff); // TL 16 = 64 EG steps = -6 dB
        key(&mut c, 0, true);
        let v = c.sample().0;
        assert!((0x1f00..0x2000).contains(&v), "{v:#x}");
    }

    #[test]
    fn pan_law() {
        assert_eq!(pan_gains(7, 0), (0x4000, 0x4000));
        assert_eq!(pan_gains(7, 0x02), (0x2000, 0x4000));
        assert_eq!(pan_gains(7, 0x1f), (0x4000, 0));
        assert_eq!(pan_gains(6, 0x10), (0x2000, 0x2000));
    }

    #[test]
    fn timer_interrupt() {
        let mut c = Scsp::new();
        c.write_reg(0x424, 0x40, 0xffff); // timer A → level 1 bit
        c.write_reg(0x426, 0x40, 0xffff); // ... and level 2 bit: level 3
        c.write_reg(0x41e, int::TIMER_A, 0xffff);
        c.write_reg(0x418, 0x01f0, 0xffff); // ÷2, start 0xF0
        let mut n = 0;
        while c.irq() == 0 {
            c.sample();
            n += 1;
            assert!(n < 100);
        }
        assert_eq!(n, 30, "(0xFF - 0xF0) ticks of 2 samples");
        assert_eq!(c.irq(), 3);
        assert_ne!(c.read_reg(0x420, true) & int::TIMER_A, 0);
        c.write_reg(0x422, int::TIMER_A, 0xffff);
        assert_eq!(c.irq(), 0);
    }

    #[test]
    fn midi_fifo() {
        let mut c = Scsp::new();
        c.write_reg(0x41e, int::MIDI_IN, 0xffff);
        c.write_reg(0x424, 0x08, 0xffff);
        c.write_reg(0x426, 0x08, 0xffff);
        assert_ne!(c.read_reg(0x404, false) & 0x100, 0, "MIEMP");
        for b in [0xa0, 0x1a, 0x02, 0x55, 0x66] {
            c.midi_in(b);
        }
        assert_eq!(c.irq(), 3);
        assert_eq!(c.stats.midi_overflows, 1);
        let st = c.read_reg(0x404, false);
        assert_eq!(st & 0x700, 0x600, "full and overflowed, nothing popped");
        assert_eq!(c.read_reg(0x404, true) & 0xff, 0xa0);
        assert_eq!(c.read_reg(0x404, true) & 0xff, 0x1a);
        assert_eq!(c.read_reg(0x404, true) & 0xff, 0x02);
        assert_eq!(c.irq(), 3);
        assert_eq!(c.read_reg(0x404, true) & 0x1ff, 0x55);
        assert_eq!(c.irq(), 0, "cleared once read empty");
        assert_ne!(c.read_reg(0x404, false) & 0x100, 0);
    }

    #[test]
    fn slot_monitor_and_byte_writes() {
        let mut c = Scsp::new();
        setup(&mut c, 5, 1, 0, 0x2000);
        c.write_reg(5 * 0x20 + 0x10, 7 << 11, 0xffff); // OCT +7: 128 samples per sample
        key(&mut c, 5, true);
        for _ in 0..40 {
            c.sample();
        }
        c.write_reg(0x408, 5 << 11, 0xff00);
        let m = c.read_reg(0x408, true);
        assert_eq!(m >> 11, 5);
        assert_eq!(m >> 7 & 0xf, ((40 * 128) >> 12) as u16);
        // A low-byte write leaves the high byte alone and doesn't key on.
        c.write_reg(0x20 * 6, 0x00ff, 0x00ff);
        assert_eq!(c.read_reg(0x20 * 6, false), 0x00ff);
    }
}
