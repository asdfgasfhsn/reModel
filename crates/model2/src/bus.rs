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

use crate::eeprom::Eeprom;
use crate::geo::Geometrizer;
use i960::cpu::Bus;
use mb86235::cpu::{External, Step, Tgp};
use romset::{Role, RomSet};
use std::collections::BTreeMap;

/// Main CPU clock: 25 MHz.
pub const CPU_HZ: u64 = 25_000_000;
/// One video frame in CPU cycles: 656x424 total pixels at a 16 MHz pixel
/// clock gives 57.524 Hz, which matches measured captures of Model 2 boards
/// (see docs/hw/peripherals.md).
pub const FRAME_CYCLES: u64 = 434_600;

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

/// Index of buffer RAM in `mems`; shared with the TGP's external bus.
const BUFFER_RAM: usize = 3;
/// How far the TGP may run while the i960 waits on an empty output FIFO.
const TGP_STALL_LIMIT: u64 = 2_000_000;

/// The TGP's view of its external bus.
struct TgpExt<'a> {
    buffer: &'a mut [u8],
    data: &'a [u32],
}

impl External for TgpExt<'_> {
    fn read(&mut self, addr: u32) -> u32 {
        match addr {
            0x40_0000..=0x7f_ffff => {
                let o = (addr as usize & 0x7fff) * 4;
                u32::from_le_bytes(self.buffer[o..o + 4].try_into().unwrap())
            }
            0x80_0000..=0x9f_ffff => self.data.get((addr - 0x80_0000) as usize).copied().unwrap_or(0),
            _ => 0,
        }
    }
    fn write(&mut self, addr: u32, v: u32) {
        if let 0x40_0000..=0x7f_ffff = addr {
            let o = (addr as usize & 0x7fff) * 4;
            self.buffer[o..o + 4].copy_from_slice(&v.to_le_bytes());
        }
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
    /// For each 64 KB page, 1 + the index of the map entry in it (0: none).
    pages: Vec<u8>,
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
    /// The second MB86235: transforms display lists for the rasteriser.
    pub geo: Geometrizer,
    /// "textures" ROM (texture headers and point lists) as u16 words.
    pub texture_rom: Vec<u16>,
    /// The geometry processor, running the uploaded program.
    pub tgp: Tgp,
    /// "copro_data" ROM as 32-bit words, at TGP external address 0x800000.
    copro_data: Vec<u32>,
    /// First instruction the TGP model couldn't execute, if any.
    pub tgp_fault: Option<u32>,
    /// The last run stopped waiting for input.
    tgp_stalled: bool,
    pub copro_empty_reads: u64,
    /// Running FNV-1a hash of every value the i960 read from the copro, to
    /// compare runs.
    pub copro_result_hash: u64,
    /// Where the CPU read the copro output FIFO while it was empty (real
    /// hardware stalls there until a result arrives), by IP.
    pub copro_empty_by_ip: BTreeMap<u32, u64>,
    /// IP of the instruction being executed, set by the machine.
    pub cur_ip: u32,
    pub geo_words: u64,
    /// When set, geo port writes (address, value) are recorded here.
    pub geo_trace: Option<Vec<(u32, u32)>>,
    /// Write watch (debugging): an address, and the (cycle, ip, address,
    /// value, size) of every memory write touching its 32-bit word.
    pub watch: Option<(u32, Vec<(u64, u32, u32, u32, usize)>)>,
    /// Video status bit 2 at 0x98000C. Hypothesis (from the boot code, which
    /// times how long it takes to change): toggles once per frame.
    pub frame_toggle: bool,
    board_ctrl: u8,

    // 315-5649 I/O: inputs are active low.
    pub inputs: [u8; 4],
    io_analog_channel: usize,
    pub analog: [u8; 8],
    /// 315-5649 output latches, by register.
    io_latch: [u8; 16],
    pub eeprom: Eeprom,

    /// Non-memory accesses, by word address.
    pub io_log: AccessLog,
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
        let mut pages = vec![0u8; 0x1_0000];
        for (i, m) in map.iter().enumerate() {
            for p in m.start >> 16..=m.end >> 16 {
                assert_eq!(pages[p as usize], 0, "two map entries share page {p:#x}");
                pages[p as usize] = i as u8 + 1;
            }
        }
        let timer = || Timer { reload: 0xfffff, started_at: 0, running: false };
        Model2Bus {
            mems,
            map,
            pages,
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
            tgp: Tgp::new(),
            copro_data: roms
                .region32(Role::CoproData)
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| u32::from_le_bytes(*c))
                .collect(),
            tgp_fault: None,
            tgp_stalled: false,
            texture_rom: roms
                .region32(Role::Textures)
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .collect(),
            geo: Geometrizer::new(
                roms.region32(Role::Polygons).as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect(),
            ),
            copro_empty_reads: 0,
            copro_result_hash: 0xcbf2_9ce4_8422_2325,
            copro_empty_by_ip: BTreeMap::new(),
            cur_ip: 0,
            geo_words: 0,
            geo_trace: None,
            watch: None,
            frame_toggle: false,
            board_ctrl: 0,
            inputs: [0xff; 4],
            io_analog_channel: 0,
            analog: [0x80; 8],
            io_latch: [0; 16],
            eeprom: Eeprom::new(),
            io_log: AccessLog::default(),
            unmapped: BTreeMap::new(),
        }
    }

    fn find(&self, addr: u32) -> Option<(usize, usize)> {
        let m = &self.map[self.pages[(addr >> 16) as usize].checked_sub(1)? as usize];
        if addr < m.start || addr > m.end {
            return None;
        }
        Some((m.mem, (addr - m.start) as usize % self.mems[m.mem].data.len()))
    }

    /// Run the TGP for up to `n` words, stopping when it waits for input.
    pub fn run_tgp(&mut self, n: u64) {
        if !self.copro_booted || self.tgp_fault.is_some() {
            return;
        }
        // A stalled TGP stays stalled until its input FIFO has a word; this
        // is called on every i960 instruction, so don't step it until then.
        if self.tgp_stalled && self.tgp.fifo_in.is_empty() {
            return;
        }
        let mut ext = TgpExt { buffer: &mut self.mems[BUFFER_RAM].data, data: &self.copro_data };
        match self.tgp.run(&mut ext, n) {
            Step::Unsupported(pc) => self.tgp_fault = Some(pc),
            r => self.tgp_stalled = r == Step::Stalled,
        }
    }

    /// Start processing this frame's display list (called at vblank). It
    /// runs alongside the i960; `geo.sync()` waits for it.
    pub fn run_geometrizer(&mut self) {
        self.geo.start_frame(&self.mems[BUFFER_RAM].data);
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
            0x0080_0000..=0x0080_7fff => self.geo.port_read(addr - 0x0080_0000),
            0x0088_4000..=0x0088_7fff => {
                // The i960 stalls until a result arrives: let the TGP run.
                if self.tgp.fifo_out.is_empty() {
                    self.run_tgp(TGP_STALL_LIMIT);
                }
                match self.tgp.fifo_out.pop_front() {
                    Some(v) => {
                        self.copro_result_hash = (self.copro_result_hash ^ v as u64).wrapping_mul(0x100_0000_01b3);
                        v
                    }
                    None => {
                        self.copro_empty_reads += 1;
                        *self.copro_empty_by_ip.entry(self.cur_ip).or_default() += 1;
                        0
                    }
                }
            }
            0x0098_0000 => self.copro_ctrl,
            0x0098_0004 => self.tgp.fifo_out.is_empty() as u32,
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

    /// 315-5649 register read. Inputs are active low. Unverified (MAME):
    /// port A bit 0 ("control mode") routes EEPROM DO onto port B bit 5, with
    /// bits 7, 6 and 4 reading high.
    fn io_reg_read(&mut self, reg: usize) -> u8 {
        match reg {
            0 | 5 => self.io_latch[reg],
            1 if self.io_latch[0] & 1 != 0 => (self.inputs[0] & 0x0f) | 0xd0 | ((self.eeprom.dout() as u8) << 5),
            1 => self.inputs[0],
            2 => self.inputs[1],
            3 => self.inputs[2],
            6 => self.inputs[3],
            8 => 0xff,
            0xf => {
                let v = self.analog[self.io_analog_channel & 7];
                self.io_analog_channel = (self.io_analog_channel + 1) & 7;
                v
            }
            _ => 0,
        }
    }

    fn io_reg_write(&mut self, reg: usize, v: u8) {
        self.io_latch[reg & 15] = v;
        match reg {
            // Port A: bit 5 DI, bit 6 CS, bit 7 CLK.
            0 => self.eeprom.set_lines(v & 0x40 != 0, v & 0x80 != 0, v & 0x20 != 0),
            0xf => self.io_analog_channel = (v & 7) as usize,
            _ => {}
        }
    }

    /// `lanes` is the byte-enable mask of the access within its word.
    fn io_write32(&mut self, addr: u32, v: u32, lanes: u32) -> bool {
        if let (Some(t), 0x0080_0000..=0x0080_7fff) = (self.geo_trace.as_mut(), addr) {
            t.push((addr, v));
        }
        match addr & !3 {
            0x0080_0000..=0x0080_7fff => {
                if addr >= 0x0080_4000 {
                    self.geo_words += 1;
                }
                let buffer = &mut self.mems[BUFFER_RAM].data;
                if !self.geo.port_write(addr - 0x0080_0000, v, buffer) {
                    return false;
                }
            }
            0x0088_4000..=0x0088_7fff => {
                if self.copro_uploading {
                    // Program RAM takes 64-bit words, low half first.
                    match self.copro_upload_half.take() {
                        None => self.copro_upload_half = Some(v),
                        Some(lo) => self.copro_program.push(((v as u64) << 32) | lo as u64),
                    }
                } else {
                    self.tgp.fifo_in.push_back(v);
                }
            }
            0x0088_0000..=0x0088_3fff => {
                let tagged = (v & 0x800f_ffff) | (((addr >> 4) & 0xff) << 23);
                self.tgp.fifo_in.push_back(tagged);
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
                    self.tgp.boot(&self.copro_program);
                    self.tgp_stalled = false;
                }
                self.copro_ctrl = v;
            }
            0x0098_0008 => self.geo.control_write(v),
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
                // Two 8-bit registers per word, in byte lanes 0 and 2.
                let reg = (((addr - 0x01c0_0000) >> 2) * 2) as usize;
                for k in 0..2 {
                    if lanes & (1 << (2 * k)) != 0 {
                        self.io_reg_write(reg + k, (v >> (16 * k)) as u8);
                    }
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
        if let Some((w, log)) = &mut self.watch {
            if addr & !3 == *w & !3 {
                log.push((self.now, self.cur_ip, addr, v, n));
            }
        }
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
        let shift = (addr & 3) * 8;
        let shifted = v << shift;
        let lanes = ((1u32 << n) - 1) << (addr & 3);
        if self.io_write32(addr, shifted, lanes) {
            return;
        }
        self.unmapped(addr, Some(v));
    }

    /// Backup SRAM at 0x01D00000.
    pub fn backup_ram(&self) -> &[u8] {
        &self.mems[8].data
    }

    /// Tile RAM, character RAM and tilemap registers: 0x01000000-0x011FFFFF.
    pub fn tile_ram(&self) -> &[u8] {
        &self.mems[5].data
    }

    /// Buffer RAM shared with the geometrizer and TGP: 0x00900000-0x0091FFFF.
    pub fn buffer_ram(&self) -> &[u8] {
        &self.mems[BUFFER_RAM].data
    }

    /// 3D screen offsets from the H/V sync registers (0x01040000 and
    /// 0x01060000): (84 + H, 130 + V), per docs/hw/geometrizer.md 4.4.
    pub fn sync_offsets(&self) -> (f32, f32) {
        let t = &self.mems[5].data;
        let h = i16::from_le_bytes([t[0x4_0000], t[0x4_0001]]) as f32;
        let v = i16::from_le_bytes([t[0x6_0000], t[0x6_0001]]) as f32;
        (84.0 + h, 130.0 + v)
    }

    /// Render RAM from 0x11000000: texture banks, luma tables, framebuffers.
    pub fn render_ram(&self) -> &[u8] {
        &self.mems[10].data
    }

    /// Palette and colour tables: 0x01800000-0x0181FFFF.
    pub fn palette_ram(&self) -> &[u8] {
        &self.mems[6].data
    }

    /// Hash of all RAM, the last raster stream and the copro results. Two
    /// runs with the same inputs must agree; used to check that speed-ups
    /// and the recompiler don't change behaviour.
    pub fn state_hash(&mut self) -> u64 {
        self.geo.sync();
        let mut h = Fnv(0xcbf2_9ce4_8422_2325);
        for m in &self.mems {
            if m.writable {
                m.data.chunks(8).for_each(|c| h.word(c.iter().fold(0, |a, &b| a << 8 | b as u64)));
            }
        }
        self.geo.engine().stream.iter().for_each(|&w| h.word(w as u64));
        h.word(self.copro_result_hash);
        h.0
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
    fn is_rom(&self, a: u32) -> bool {
        a < 0x20_0000
    }
}

/// Access counts by address, with a cheap hash (logged on every I/O access).
pub type AccessLog = std::collections::HashMap<u32, Access, std::hash::BuildHasherDefault<AddrHasher>>;

#[derive(Default)]
pub struct AddrHasher(u64);

impl std::hash::Hasher for AddrHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        bytes.iter().for_each(|&b| self.0 = (self.0 << 8 | b as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    }
    fn write_u32(&mut self, v: u32) {
        self.0 = (v as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
}

struct Fnv(u64);

impl Fnv {
    fn word(&mut self, v: u64) {
        self.0 = (self.0 ^ v).wrapping_mul(0x100_0000_01b3);
    }
}
