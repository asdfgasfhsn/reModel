//! Model 2C-CRX i960 address space.
//!
//! Provenance: the layout below is a working hypothesis taken from MAME's
//! public driver notes, which do not run this game correctly. Treat every
//! register meaning as unverified until the game's own code confirms it.
//!
//! Memory regions are plain byte arrays. Hardware registers are modelled
//! only as far as needed for the game to make progress; every access to
//! anything that isn't plain memory is counted so the runner can report
//! what the game is waiting on.

use i960::cpu::Bus;
use romset::{Role, RomSet};
use std::collections::BTreeMap;

/// Main CPU clock: 25 MHz.
pub const CPU_HZ: u64 = 25_000_000;
pub const VBLANK_HZ: u64 = 60;

// IRQ controller request/enable bits.
pub const IRQ_VBLANK: u32 = 1 << 0;
pub const IRQ_TIMER0: u32 = 1 << 2;
pub const IRQ_UART: u32 = 1 << 10;

/// i960 external line driven by each request bit group.
fn irq_line(bits: u32) -> [bool; 4] {
    [bits & 0x1 != 0, bits & 0x2 != 0, bits & 0x3fc != 0, bits & 0xc00 != 0]
}

#[derive(Default, Clone, Copy)]
pub struct Access {
    pub reads: u64,
    pub writes: u64,
    pub last_write: u32,
}

struct Mem {
    data: Vec<u8>,
    writable: bool,
}

impl Mem {
    fn zeroed(size: usize) -> Self {
        Mem { data: vec![0; size], writable: true }
    }
    fn from(data: Vec<u8>, writable: bool) -> Self {
        Mem { data, writable }
    }
}

/// Region table entry: [start, end] inclusive, index into `mems`. A window
/// larger than its backing store mirrors it.
struct Map {
    start: u32,
    end: u32,
    mem: usize,
}

pub struct Timer {
    reload: u32,
    started_at: u64,
    running: bool,
}

pub struct Model2Bus {
    mems: Vec<Mem>,
    map: Vec<Map>,
    /// Current time in CPU cycles; advanced by the machine.
    pub now: u64,

    pub irq_request: u32,
    pub irq_enable: u32,
    /// Lines newly asserted since the machine last looked.
    pub raised_lines: [bool; 4],
    pub timers: [Timer; 4],

    // Geometry coprocessor (MB86235) interface.
    copro_ctrl: u32,
    copro_uploading: bool,
    pub copro_program: Vec<u64>,
    copro_upload_half: Option<u32>,
    pub copro_booted: bool,
    pub copro_fifo_in: Vec<u32>,
    pub copro_fifo_out: Vec<u32>,
    pub copro_empty_reads: u64,
    pub geo_words: u64,
    /// Video status bit 2 at 0x98000C. Hypothesis (from the boot code, which
    /// times how long it takes to change): toggles once per frame.
    pub frame_toggle: bool,
    board_ctrl: u8,

    // 315-5649 I/O: inputs are active low.
    pub inputs: [u8; 4],
    io_analog_channel: usize,
    pub analog: [u8; 8],

    /// Non-memory accesses, by word address.
    pub io_log: BTreeMap<u32, Access>,
    /// Accesses that hit nothing, by word address.
    pub unmapped: BTreeMap<u32, Access>,
}

impl Model2Bus {
    pub fn new(roms: &RomSet) -> Self {
        let mut program = roms.program.clone();
        program.resize(0x20_0000, 0);
        let mut main_data = roms.region32(Role::MainData);
        main_data.resize(0x200_0000, 0);

        let mems = vec![
            Mem::from(program, false),
            Mem::zeroed(0x4_0000),
            Mem::zeroed(0x10_0000),
            Mem::zeroed(0x2_0000),
            Mem::zeroed(0x40),
            Mem::zeroed(0x20_0000),
            Mem::zeroed(0x2_0000),
            Mem::zeroed(0x4000),
            Mem::from(vec![0xff; 0x4000], true),
            Mem::from(main_data, false),
            Mem::zeroed(0x70_0000),
        ];
        let map = vec![
            Map { start: 0x0000_0000, end: 0x001f_ffff, mem: 0 },
            Map { start: 0x0020_0000, end: 0x0023_ffff, mem: 1 },
            Map { start: 0x0050_0000, end: 0x005f_ffff, mem: 2 },
            Map { start: 0x0090_0000, end: 0x0097_ffff, mem: 3 },
            Map { start: 0x00e0_0000, end: 0x00e0_0037, mem: 4 },
            Map { start: 0x0100_0000, end: 0x011f_ffff, mem: 5 },
            Map { start: 0x0180_0000, end: 0x0181_ffff, mem: 6 },
            Map { start: 0x01a0_0000, end: 0x01a0_3fff, mem: 7 },
            Map { start: 0x01a1_0000, end: 0x01a1_3fff, mem: 7 },
            Map { start: 0x01d0_0000, end: 0x01d0_3fff, mem: 8 },
            Map { start: 0x0200_0000, end: 0x03ff_ffff, mem: 9 },
            Map { start: 0x1100_0000, end: 0x116f_ffff, mem: 10 },
        ];
        let timer = || Timer { reload: 0xfffff, started_at: 0, running: false };
        Model2Bus {
            mems,
            map,
            now: 0,
            irq_request: 0,
            irq_enable: 0,
            raised_lines: [false; 4],
            timers: [timer(), timer(), timer(), timer()],
            copro_ctrl: 0,
            copro_uploading: false,
            copro_program: Vec::new(),
            copro_upload_half: None,
            copro_booted: false,
            copro_fifo_in: Vec::new(),
            copro_fifo_out: Vec::new(),
            copro_empty_reads: 0,
            geo_words: 0,
            frame_toggle: false,
            board_ctrl: 0,
            inputs: [0xff; 4],
            io_analog_channel: 0,
            analog: [0x80; 8],
            io_log: BTreeMap::new(),
            unmapped: BTreeMap::new(),
        }
    }

    fn find(&self, addr: u32) -> Option<(usize, usize)> {
        let m = self.map.iter().find(|m| addr >= m.start && addr <= m.end)?;
        Some((m.mem, (addr - m.start) as usize % self.mems[m.mem].data.len()))
    }

    /// Raise IRQ request bits (only those already enabled latch).
    pub fn request_irq(&mut self, bits: u32) {
        let before = irq_line(self.irq_request);
        self.irq_request |= bits & self.irq_enable;
        let after = irq_line(self.irq_request);
        for i in 0..4 {
            if after[i] && !before[i] {
                self.raised_lines[i] = true;
            }
        }
    }

    /// Advance timers to `now`, raising their IRQs on expiry.
    pub fn tick_timers(&mut self) {
        for i in 0..4 {
            let t = &mut self.timers[i];
            if t.running && self.now - t.started_at >= t.reload as u64 {
                t.running = false;
                t.reload = 0xfffff;
                self.request_irq(IRQ_TIMER0 << i);
            }
        }
    }

    fn log(&mut self, addr: u32, write: Option<u32>) {
        let e = self.io_log.entry(addr & !3).or_default();
        match write {
            Some(v) => {
                e.writes += 1;
                e.last_write = v;
            }
            None => e.reads += 1,
        }
    }

    fn io_read32(&mut self, addr: u32) -> Option<u32> {
        let v = match addr & !3 {
            0x0080_0000..=0x0080_7fff => 0,
            0x0088_4000..=0x0088_7fff => {
                if self.copro_fifo_out.is_empty() {
                    // Hardware would stall the CPU here.
                    self.copro_empty_reads += 1;
                    0
                } else {
                    self.copro_fifo_out.remove(0)
                }
            }
            0x0098_0000 => self.copro_ctrl,
            0x0098_0004 => self.copro_fifo_out.is_empty() as u32,
            0x0098_000c => (self.frame_toggle as u32) << 2,
            0x0098_0014 => {
                if self.copro_uploading && self.copro_program.is_empty() {
                    u32::MAX
                } else {
                    0
                }
            }
            0x0098_0008..=0x0098_003f => 0,
            // Board control byte set during reset; meaning unknown.
            0x00f8_0000 => self.board_ctrl as u32,
            0x00e8_0000 => self.irq_request,
            0x00e8_0004 => self.irq_enable,
            0x00f0_0000..=0x00f0_000f => {
                let t = &self.timers[((addr >> 2) & 3) as usize];
                if t.running {
                    t.reload.saturating_sub((self.now - t.started_at) as u32)
                } else {
                    t.reload
                }
            }
            0x01c0_0000..=0x01c0_001f => {
                let lo = self.io_reg_read((((addr - 0x01c0_0000) >> 2) * 2) as usize) as u32;
                let hi = self.io_reg_read((((addr - 0x01c0_0000) >> 2) * 2 + 1) as usize) as u32;
                lo | (hi << 16)
            }
            // UART: status reports transmitter ready and empty, nothing received.
            0x01c8_0000..=0x01c8_0003 => 0x0005_0000,
            0x1000_0000..=0x105f_ffff => 0,
            _ => return None,
        };
        self.log(addr, None);
        Some(v)
    }

    /// 315-5649 register read. Inputs are active low.
    fn io_reg_read(&mut self, reg: usize) -> u8 {
        match reg {
            1 => self.inputs[0],
            2 => self.inputs[1],
            3 => self.inputs[2],
            6 => self.inputs[3],
            8 => 0xff,
            0xf => {
                let v = self.analog[self.io_analog_channel & 7];
                self.io_analog_channel += 1;
                v
            }
            _ => 0,
        }
    }

    fn io_write32(&mut self, addr: u32, v: u32) -> bool {
        match addr & !3 {
            0x0080_4000..=0x0080_7fff => self.geo_words += 1,
            0x0080_0000..=0x0080_3fff => {}
            0x0088_4000..=0x0088_7fff => {
                if self.copro_uploading {
                    // Program RAM takes 64-bit words, low half first.
                    match self.copro_upload_half.take() {
                        None => self.copro_upload_half = Some(v),
                        Some(lo) => self.copro_program.push(((v as u64) << 32) | lo as u64),
                    }
                } else {
                    self.copro_fifo_in.push(v);
                }
            }
            0x0088_0000..=0x0088_3fff => {
                let tagged = (v & 0x800f_ffff) | (((addr >> 4) & 0xff) << 23);
                self.copro_fifo_in.push(tagged);
            }
            0x0098_0000 => {
                let rising = v & 0x8000_0000 != 0 && self.copro_ctrl & 0x8000_0000 == 0;
                let falling = v & 0x8000_0000 == 0 && self.copro_ctrl & 0x8000_0000 != 0;
                if rising {
                    self.copro_uploading = true;
                    self.copro_program.clear();
                }
                if falling {
                    self.copro_uploading = false;
                    self.copro_booted = true;
                }
                self.copro_ctrl = v;
            }
            0x0098_0004..=0x0098_003f => {}
            0x00f8_0000 => self.board_ctrl = v as u8,
            0x00e8_0000 => self.irq_request &= v,
            0x00e8_0004 => self.irq_enable = v,
            0x00f0_0000..=0x00f0_000f => {
                let t = &mut self.timers[((addr >> 2) & 3) as usize];
                t.reload = v & 0xff_ffff;
                t.started_at = self.now;
                t.running = true;
            }
            0x01c0_0000..=0x01c0_001f => {
                let reg = (((addr - 0x01c0_0000) >> 2) * 2) as usize;
                if reg + 1 == 0xf {
                    self.io_analog_channel = ((v >> 16) & 7) as usize;
                }
            }
            0x01c8_0000..=0x01c8_0003 => {}
            0x1000_0000..=0x105f_ffff => {}
            _ => return false,
        }
        self.log(addr, Some(v));
        true
    }

    fn mem_read(&self, addr: u32, n: usize) -> Option<u32> {
        let (m, off) = self.find(addr)?;
        let d = &self.mems[m].data;
        let mut v = 0u32;
        for i in 0..n {
            v |= (d[(off + i) % d.len()] as u32) << (8 * i);
        }
        Some(v)
    }

    fn mem_write(&mut self, addr: u32, v: u32, n: usize) -> bool {
        let Some((m, off)) = self.find(addr) else { return false };
        let mem = &mut self.mems[m];
        if mem.writable {
            let len = mem.data.len();
            for i in 0..n {
                mem.data[(off + i) % len] = (v >> (8 * i)) as u8;
            }
        }
        true
    }

    fn unmapped(&mut self, addr: u32, write: Option<u32>) {
        let e = self.unmapped.entry(addr & !3).or_default();
        match write {
            Some(v) => {
                e.writes += 1;
                e.last_write = v;
            }
            None => e.reads += 1,
        }
    }

    fn read(&mut self, addr: u32, n: usize) -> u32 {
        if let Some(v) = self.mem_read(addr, n) {
            return v;
        }
        if let Some(w) = self.io_read32(addr) {
            let shift = (addr & 3) * 8;
            let mask = if n == 4 { u32::MAX } else { (1u32 << (8 * n)) - 1 };
            return (w >> shift) & mask;
        }
        self.unmapped(addr, None);
        0
    }

    fn write(&mut self, addr: u32, v: u32, n: usize) {
        if self.mem_write(addr, v, n) {
            return;
        }
        let shifted = v << ((addr & 3) * 8);
        if self.io_write32(addr, shifted) {
            return;
        }
        self.unmapped(addr, Some(v));
    }

    /// Raw view of a RAM/ROM region for snapshots and the recompiler.
    pub fn backup_ram(&self) -> &[u8] {
        &self.mems[8].data
    }
}

impl Bus for Model2Bus {
    fn read8(&mut self, a: u32) -> u8 {
        self.read(a, 1) as u8
    }
    fn read16(&mut self, a: u32) -> u16 {
        self.read(a, 2) as u16
    }
    fn read32(&mut self, a: u32) -> u32 {
        self.read(a, 4)
    }
    fn write8(&mut self, a: u32, v: u8) {
        self.write(a, v as u32, 1)
    }
    fn write16(&mut self, a: u32, v: u16) {
        self.write(a, v as u32, 2)
    }
    fn write32(&mut self, a: u32, v: u32) {
        self.write(a, v, 4)
    }
    fn fetch(&self, a: u32) -> Option<u32> {
        self.mem_read(a, 4)
    }
}
