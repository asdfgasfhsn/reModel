//! The Model 2 sound board: a 68EC000 at 11.2896 MHz with one SCSP, fed by
//! the main board over a 31.25 kbaud MIDI link (`docs/hw/sound.md` §1).
//!
//! 68000 map: sound RAM (shared with the SCSP) at 0, SCSP registers at
//! 0x100000, a control latch at 0x400001, program ROM at 0x600000 and sample
//! ROM at 0x800000. No bus wait states are modelled (MAME adds one per RAM
//! and SCSP access, a guess).

use crate::scsp::{Scsp, RAM_SIZE};
use m68k::Cpu;
use std::collections::VecDeque;

pub const CPU_HZ: u32 = 11_289_600;
pub const SAMPLE_RATE: u32 = 44_100;
const CYCLES_PER_SAMPLE: i32 = (CPU_HZ / SAMPLE_RATE) as i32;
/// One 10-bit MIDI frame at 31.25 kbaud, in thousandths of a sample.
const BYTE_TIME: u32 = SAMPLE_RATE * 10 * 1000 / 31_250;

const ROM_BASE: u32 = 0x60_0000;
const SAMPLE_BASE: u32 = 0x80_0000;

/// Bus activity counters.
#[derive(Clone, Default, Debug)]
pub struct BusStats {
    pub unmapped_reads: u64,
    pub unmapped_writes: u64,
    /// First few unmapped accesses: (address, is write, PC).
    pub unmapped: Vec<(u32, bool, u32)>,
    pub sample_rom_reads: u64,
    pub latch_writes: u64,
    pub rom_writes: u64,
}

/// What the 68000 sees.
pub struct BoardBus {
    pub scsp: Scsp,
    pub rom: Vec<u8>,
    pub samples: Vec<u8>,
    /// Last value written to the 0x400001 latch (bits 3-0 look like LEDs).
    pub latch: u8,
    /// Reset in progress: the vector fetch reads ROM at address 0.
    boot: bool,
    pc: u32,
    pub stats: BusStats,
}

impl BoardBus {
    fn unmapped(&mut self, addr: u32, write: bool) {
        if write {
            self.stats.unmapped_writes += 1;
        } else {
            self.stats.unmapped_reads += 1;
        }
        if self.stats.unmapped.len() < 16 {
            self.stats.unmapped.push((addr, write, self.pc));
        }
    }

    fn rom8(data: &[u8], off: u32) -> Option<u8> {
        data.get(off as usize).copied()
    }
}

impl m68k::Bus for BoardBus {
    fn read8(&mut self, addr: u32) -> u8 {
        match addr {
            0..0x8_0000 if self.boot => Self::rom8(&self.rom, addr).unwrap_or(0),
            0..0x8_0000 => self.scsp.ram[addr as usize],
            0x10_0000..0x11_0000 => {
                let w = self.scsp.read_reg(addr & 0xfff, addr & 1 != 0);
                if addr & 1 != 0 {
                    w as u8
                } else {
                    (w >> 8) as u8
                }
            }
            ROM_BASE..SAMPLE_BASE => match Self::rom8(&self.rom, addr - ROM_BASE) {
                Some(b) => b,
                None => {
                    self.unmapped(addr, false);
                    0xff
                }
            },
            SAMPLE_BASE.. => {
                self.stats.sample_rom_reads += 1;
                Self::rom8(&self.samples, addr - SAMPLE_BASE).unwrap_or(0xff)
            }
            _ => {
                self.unmapped(addr, false);
                0xff
            }
        }
    }

    fn read16(&mut self, addr: u32) -> u16 {
        match addr {
            0..0x8_0000 if !self.boot => {
                let a = addr as usize;
                u16::from_be_bytes([self.scsp.ram[a], self.scsp.ram[a + 1]])
            }
            0x10_0000..0x11_0000 => self.scsp.read_reg(addr & 0xfff, true),
            _ => (self.read8(addr) as u16) << 8 | self.read8(addr + 1) as u16,
        }
    }

    fn write8(&mut self, addr: u32, v: u8) {
        match addr {
            0..0x8_0000 => self.scsp.ram[addr as usize] = v,
            0x10_0000..0x11_0000 => {
                let (w, mask) = if addr & 1 != 0 { (v as u16, 0x00ff) } else { ((v as u16) << 8, 0xff00) };
                self.scsp.write_reg(addr & 0xfff, w, mask);
            }
            0x40_0000..=0x40_0001 => {
                self.latch = v;
                self.stats.latch_writes += 1;
            }
            ROM_BASE.. => self.stats.rom_writes += 1,
            _ => self.unmapped(addr, true),
        }
    }

    fn write16(&mut self, addr: u32, v: u16) {
        match addr {
            0..0x8_0000 => {
                let a = addr as usize;
                self.scsp.ram[a..a + 2].copy_from_slice(&v.to_be_bytes());
            }
            0x10_0000..0x11_0000 => self.scsp.write_reg(addr & 0xfff, v, 0xffff),
            0x40_0000 => self.write8(0x40_0001, v as u8),
            _ => {
                self.write8(addr, (v >> 8) as u8);
                self.write8(addr + 1, v as u8);
            }
        }
    }
}

pub struct SoundBoard {
    pub cpu: Cpu,
    pub bus: BoardBus,
    /// Host bytes not yet delivered over the serial link.
    link: VecDeque<u8>,
    link_time: u32,
    /// CPU cycles run ahead of (positive) or behind the sample clock.
    cycles: i32,
    /// Output samples produced so far.
    pub samples: u64,
    pub instructions: u64,
    /// One bit per program ROM word that has started an instruction.
    pub coverage: Vec<u64>,
    /// Executed instructions per 256-byte block of program ROM.
    pub hot: Vec<u64>,
}

impl SoundBoard {
    /// Build the board from the 68000's big-endian views of the program and
    /// sample ROMs, and reset the CPU.
    ///
    /// Reset: the 68000 fetches SSP/PC from address 0, which is sound RAM.
    /// How the real board provides them is unknown (MAME copies the ROM's
    /// first 16 bytes into RAM; see MH-4). Here the vector fetch reads the
    /// program ROM, and the driver installs its own table in RAM.
    pub fn new(rom: Vec<u8>, samples: Vec<u8>) -> Self {
        let words = rom.len() / 2;
        let mut bus =
            BoardBus { scsp: Scsp::new(), rom, samples, latch: 0, boot: true, pc: 0, stats: BusStats::default() };
        let mut cpu = Cpu::new();
        cpu.reset(&mut bus);
        bus.boot = false;
        debug_assert_eq!(bus.scsp.ram.len(), RAM_SIZE);
        SoundBoard {
            cpu,
            bus,
            link: VecDeque::new(),
            link_time: BYTE_TIME,
            cycles: 0,
            samples: 0,
            instructions: 0,
            coverage: vec![0; words.div_ceil(64)],
            hot: vec![0; words.div_ceil(128)],
        }
    }

    /// Queue a byte from the host. It reaches the SCSP's MIDI-in at the
    /// link's byte rate.
    pub fn midi_in(&mut self, byte: u8) {
        self.link.push_back(byte);
    }

    /// Bytes still queued on the link.
    pub fn link_pending(&self) -> usize {
        self.link.len()
    }

    /// Produce `samples` stereo samples at 44.1 kHz, appended to `out` as
    /// interleaved left/right. The CPU runs in one-sample slices.
    pub fn run(&mut self, samples: usize, out: &mut Vec<i16>) {
        out.reserve(samples * 2);
        for _ in 0..samples {
            self.link_time = (self.link_time + 1000).min(BYTE_TIME);
            if self.link_time >= BYTE_TIME {
                if let Some(b) = self.link.pop_front() {
                    self.bus.scsp.midi_in(b);
                    self.link_time -= BYTE_TIME;
                }
            }
            self.cycles += CYCLES_PER_SAMPLE;
            while self.cycles > 0 {
                self.cpu.set_irq(self.bus.scsp.irq());
                let pc = self.cpu.pc;
                if let Some(off) = pc.checked_sub(ROM_BASE).filter(|&o| (o as usize) < self.bus.rom.len()) {
                    let w = (off >> 1) as usize;
                    self.coverage[w >> 6] |= 1 << (w & 63);
                    self.hot[w >> 7] += 1;
                }
                self.bus.pc = pc;
                self.cycles -= self.cpu.step(&mut self.bus) as i32;
                self.instructions += 1;
            }
            let (l, r) = self.bus.scsp.sample();
            out.push(l);
            out.push(r);
            self.samples += 1;
        }
    }

    /// Program ROM words that have started an instruction.
    pub fn covered_words(&self) -> u64 {
        self.coverage.iter().map(|w| w.count_ones() as u64).sum()
    }
}
