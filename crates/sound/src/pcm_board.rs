//! The original Model 2's sound board (`daytona`, `vcop`, `desert`): a
//! 68000 with two MultiPCM sample chips and a YM3438 (OPN2) FM chip, fed
//! the main board's MIDI-style bytes on a UART (`docs/hw/model2.md` §5).
//!
//! 68000 map, from the sound programs:
//!
//! | address | device |
//! |---|---|
//! | `000000-0FFFFF` | program ROM (the 128 KB chip, mirrored) |
//! | `C00001` | an input: bit 0 picks a velocity curve (unknown source; reads `FF`) |
//! | `C10001` | latch the program steps through `1, 2, 4 ... 80` (LEDs; ignored) |
//! | `C20001` / `C20003` | UART data / control and status (8251-like) |
//! | `C40001` / `C40003` / `C40005` | MultiPCM 1: data (read: status), slot, register |
//! | `C50001` | MultiPCM 1's sample bank |
//! | `C60001` / `C60003` / `C60005` | MultiPCM 2, likewise |
//! | `C70001` | MultiPCM 2's sample bank |
//! | `D00001` / `D00003` | FM part 1: address (read: status) / data |
//! | `D00005` / `D00007` | FM part 2: address / data |
//! | `F00000-F0FFFF` | 64 KB work RAM |
//!
//! The UART's receive line raises interrupt level 2 (autovector) while a
//! byte is unread; reading the data port clears it. Nothing else
//! interrupts: the program polls the FM chip's timer B for its tempo.
//!
//! Clocks are *unconfirmed*: the CPU at 10 MHz (only how much the program
//! gets done between timer ticks depends on it), the FM chip at 8 MHz
//! (timer B's period, so the music's tempo), the MultiPCMs at the 44.1 kHz
//! output rate (inferred from the samples' tuning, `docs/hw/multipcm.md`).

use crate::multipcm::MultiPcm;
use crate::opn2::Opn2;
use m68k::Cpu;
use std::collections::VecDeque;

pub const CPU_HZ: u64 = 10_000_000;
pub const FM_CLOCK: u64 = 8_000_000;
/// Output rate (the MultiPCMs' own).
pub const SAMPLE_RATE: u32 = 44_100;
/// FM chip samples a second (its clock / 144), as a fraction of output
/// samples: FM samples per output sample = FM_CLOCK / (144 * 44100).
const FM_DEN: u64 = 144 * SAMPLE_RATE as u64;
/// CPU cycles per output sample, as a fraction: CPU_HZ / 44100.
const CPU_DEN: u64 = SAMPLE_RATE as u64;
/// One 10-bit frame at 31.25 kbaud, in thousandths of an output sample.
const BYTE_TIME: u32 = SAMPLE_RATE * 10 * 1000 / 31_250;
/// The FM chip is busy for 32 of its internal cycles (6 clocks each)
/// after a data write (*unconfirmed*), in CPU cycles.
const FM_BUSY: u64 = 32 * 6 * CPU_HZ / FM_CLOCK;

const RAM_BASE: u32 = 0xf0_0000;
const RAM_SIZE: usize = 0x1_0000;

/// Bus counters.
#[derive(Clone, Default, Debug)]
pub struct Stats {
    pub unmapped_reads: u64,
    pub unmapped_writes: u64,
    /// First few unmapped accesses: (address, is write, PC).
    pub unmapped: Vec<(u32, bool, u32)>,
    pub rx_bytes: u64,
    /// Bytes that arrived while the previous one was still unread.
    pub overruns: u64,
    pub fm_writes: u64,
    /// FM status reads that found the chip busy.
    pub fm_busy_waits: u64,
    pub leds: u8,
}

/// What the 68000 sees.
pub struct Bus {
    rom: Vec<u8>,
    ram: Vec<u8>,
    pub pcm: [MultiPcm; 2],
    pub fm: Opn2,
    /// The received byte not yet read, if any.
    rx: Option<u8>,
    /// CPU cycles since reset, and when the FM chip stops being busy.
    now: u64,
    fm_busy_until: u64,
    pc: u32,
    pub stats: Stats,
}

impl Bus {
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
}

impl m68k::Bus for Bus {
    fn read8(&mut self, addr: u32) -> u8 {
        match addr {
            0..0x10_0000 => {
                let n = self.rom.len().max(1);
                self.rom.get(addr as usize % n).copied().unwrap_or(0xff)
            }
            // An input: bit 0 picks one of two velocity curves (0x9CA).
            // Unknown what drives it (a jumper?); reads as all ones.
            0xc0_0000..0xc1_0000 => 0xff,
            // UART status: TxRDY, TxEMPTY, RxRDY in bit 1, no errors.
            0xc2_0003 => 0x05 | (self.rx.is_some() as u8) << 1,
            0xc2_0001 => self.rx.take().unwrap_or(0),
            0xc4_0000..0xc5_0000 => self.pcm[0].status(),
            0xc6_0000..0xc7_0000 => self.pcm[1].status(),
            0xd0_0000..0xd1_0000 => {
                let busy = self.now < self.fm_busy_until;
                if busy {
                    self.stats.fm_busy_waits += 1;
                }
                self.fm.status() | (busy as u8) << 7
            }
            RAM_BASE..0xf1_0000 => self.ram[(addr - RAM_BASE) as usize],
            _ => {
                self.unmapped(addr, false);
                0xff
            }
        }
    }

    fn read16(&mut self, addr: u32) -> u16 {
        match addr {
            RAM_BASE..0xf1_0000 => {
                let a = (addr - RAM_BASE) as usize;
                u16::from_be_bytes([self.ram[a], self.ram[a + 1]])
            }
            _ => (self.read8(addr) as u16) << 8 | self.read8(addr + 1) as u16,
        }
    }

    fn write8(&mut self, addr: u32, v: u8) {
        match addr {
            0..0x10_0000 => {}
            0xc1_0000..0xc2_0000 => self.stats.leds = v,
            // UART control (mode and command bytes): nothing to model.
            0xc2_0000..0xc3_0000 => {}
            0xc4_0000..0xc5_0000 => self.pcm[0].write((addr & 0xffff) >> 1, v),
            0xc5_0000..0xc6_0000 => self.pcm[0].set_bank(v),
            0xc6_0000..0xc7_0000 => self.pcm[1].write((addr & 0xffff) >> 1, v),
            0xc7_0000..0xc8_0000 => self.pcm[1].set_bank(v),
            0xd0_0000..0xd1_0000 => {
                let port = (addr >> 1) & 3;
                self.fm.write(port, v);
                if port & 1 != 0 {
                    self.fm_busy_until = self.now + FM_BUSY;
                    self.stats.fm_writes += 1;
                }
            }
            RAM_BASE..0xf1_0000 => self.ram[(addr - RAM_BASE) as usize] = v,
            _ => self.unmapped(addr, true),
        }
    }

    fn write16(&mut self, addr: u32, v: u16) {
        match addr {
            RAM_BASE..0xf1_0000 => {
                let a = (addr - RAM_BASE) as usize;
                self.ram[a..a + 2].copy_from_slice(&v.to_be_bytes());
            }
            _ => {
                self.write8(addr, (v >> 8) as u8);
                self.write8(addr + 1, v as u8);
            }
        }
    }
}

pub struct PcmBoard {
    pub cpu: Cpu,
    pub bus: Bus,
    /// Host bytes not yet delivered over the link.
    link: VecDeque<u8>,
    link_time: u32,
    /// CPU cycles run ahead of (positive) or behind the sample clock, in
    /// 1/44100ths of a cycle; the FM chip's sample clock likewise.
    cycles: i64,
    fm_phase: u64,
    /// FM output accumulated since the last output sample.
    pub samples: u64,
    pub instructions: u64,
    /// Program words that started an instruction (one bit each).
    pub coverage: Vec<u64>,
}

impl PcmBoard {
    /// The board from the 68000's big-endian view of its program and the
    /// two MultiPCMs' sample ROMs (each its chips concatenated).
    pub fn new(rom: Vec<u8>, samples_a: Vec<u8>, samples_b: Vec<u8>) -> Self {
        let words = rom.len() / 2;
        let mut bus = Bus {
            rom,
            ram: vec![0; RAM_SIZE],
            pcm: [MultiPcm::new(samples_a), MultiPcm::new(samples_b)],
            fm: Opn2::new(),
            rx: None,
            now: 0,
            fm_busy_until: 0,
            pc: 0,
            stats: Stats::default(),
        };
        let mut cpu = Cpu::new();
        cpu.reset(&mut bus);
        PcmBoard {
            cpu,
            bus,
            link: VecDeque::new(),
            link_time: BYTE_TIME,
            cycles: 0,
            fm_phase: 0,
            samples: 0,
            instructions: 0,
            coverage: vec![0; words.div_ceil(64)],
        }
    }

    /// Queue a byte from the host; it reaches the UART at the link's rate.
    pub fn midi_in(&mut self, byte: u8) {
        self.link.push_back(byte);
    }

    pub fn link_pending(&self) -> usize {
        self.link.len()
    }

    /// Produce `samples` stereo samples at 44.1 kHz, appended to `out` as
    /// interleaved left/right.
    pub fn run(&mut self, samples: usize, out: &mut Vec<i16>) {
        out.reserve(samples * 2);
        for _ in 0..samples {
            self.link_time = (self.link_time + 1000).min(BYTE_TIME);
            if self.link_time >= BYTE_TIME {
                if let Some(b) = self.link.pop_front() {
                    if self.bus.rx.replace(b).is_some() {
                        self.bus.stats.overruns += 1;
                    }
                    self.bus.stats.rx_bytes += 1;
                    self.link_time -= BYTE_TIME;
                }
            }
            // FM chip samples due in this output sample: its timers, and
            // its output averaged.
            self.fm_phase += FM_CLOCK;
            let (mut fl, mut fr, mut fn_) = (0i32, 0i32, 0i32);
            while self.fm_phase >= FM_DEN {
                self.fm_phase -= FM_DEN;
                let (l, r) = self.bus.fm.step();
                fl += l;
                fr += r;
                fn_ += 1;
            }
            self.cycles += CPU_HZ as i64;
            while self.cycles > 0 {
                self.cpu.set_irq(if self.bus.rx.is_some() { 2 } else { 0 });
                let pc = self.cpu.pc;
                let w = (pc >> 1) as usize;
                if let Some(c) = self.coverage.get_mut(w >> 6) {
                    *c |= 1 << (w & 63);
                }
                self.bus.pc = pc;
                let c = self.cpu.step(&mut self.bus) as u64;
                self.bus.now += c;
                self.cycles -= (c * CPU_DEN) as i64;
                self.instructions += 1;
            }
            let (al, ar) = self.bus.pcm[0].sample();
            let (bl, br) = self.bus.pcm[1].sample();
            let n = fn_.max(1);
            // Mix: the MultiPCMs' voices at 16-bit scale, a twelfth (up
            // to 56 voices), the FM's 14-bit channels as they are. The
            // board's analog mix is not known: a listening choice.
            let l = (al + bl) / 12 + fl / n;
            let r = (ar + br) / 12 + fr / n;
            out.push(l.clamp(-32768, 32767) as i16);
            out.push(r.clamp(-32768, 32767) as i16);
            self.samples += 1;
        }
    }

    pub fn covered_words(&self) -> u64 {
        self.coverage.iter().map(|w| w.count_ones() as u64).sum()
    }
}

// Save states: ROMs, coverage and statistics stay as they are.
state::fields!(PcmBoard { cpu, bus, link, link_time, cycles, fm_phase, samples, instructions });
state::fields!(Bus { ram, pcm, fm, rx, now, fm_busy_until, pc });

#[cfg(test)]
mod tests {
    use super::*;

    /// A program that waits for timer B in a loop and counts overflows in
    /// RAM, set up as the sound programs do.
    fn polling_rom() -> Vec<u8> {
        // Reset vectors: SSP 0xF0FFFE, PC 0x100.
        let mut rom = vec![0u8; 0x400];
        rom[..8].copy_from_slice(&[0x00, 0xf0, 0xff, 0xfe, 0x00, 0x00, 0x01, 0x00]);
        let code: &[u16] = &[
            0x13fc, 0x0026, 0x00d0, 0x0001, // move.b #$26,$d00001
            0x13fc, 0x00fc, 0x00d0, 0x0003, // move.b #$fc,$d00003
            0x13fc, 0x0027, 0x00d0, 0x0001, // move.b #$27,$d00001
            0x13fc, 0x002a, 0x00d0, 0x0003, // move.b #$2a,$d00003
            0x0839, 0x0001, 0x00d0, 0x0001, // loop: btst #1,$d00001
            0x67f6, //                         beq.s loop
            0x13fc, 0x002a, 0x00d0, 0x0003, // move.b #$2a,$d00003 (reset flag)
            0x5279, 0x00f0, 0x0000, //         addq.w #1,$f00000
            0x60e6, //                         bra.s loop
        ];
        for (i, w) in code.iter().enumerate() {
            rom[0x100 + 2 * i..0x102 + 2 * i].copy_from_slice(&w.to_be_bytes());
        }
        rom
    }

    #[test]
    fn timer_b_paces_a_polling_loop() {
        let mut b = PcmBoard::new(polling_rom(), vec![0; 0x2000], vec![0; 0x2000]);
        let mut out = Vec::new();
        b.run(44100, &mut out);
        let ticks = u16::from_be_bytes([b.bus.ram[0], b.bus.ram[1]]) as f64;
        // 8 MHz / 144 / 64 samples a tick.
        let expect = FM_CLOCK as f64 / 144.0 / 64.0;
        assert!((ticks - expect).abs() < 3.0, "{ticks} vs {expect}");
    }

    /// A save state taken mid-run resumes to the same output: CPU, RAM,
    /// timers, a sounding MultiPCM voice and queued link bytes.
    #[test]
    fn save_state_round_trip() {
        let mut samples = vec![0u8; 0x4000];
        samples[..12].copy_from_slice(&[0, 0x20, 0, 0, 0, 0xf0, 0x00, 0, 0xf0, 0, 0x08, 0]);
        for (i, b) in samples[0x2000..0x3000].iter_mut().enumerate() {
            *b = ((i * 7) % 200) as u8;
        }
        let board = || PcmBoard::new(polling_rom(), samples.clone(), vec![0; 0x2000]);
        let mut a = board();
        let mut out = Vec::new();
        a.run(1000, &mut out);
        for (port, v) in [(1, 3), (2, 1), (0, 0), (2, 3), (0, 0x0a), (2, 5), (0, 0x01), (2, 4), (0, 0x80)] {
            a.bus.pcm[0].write(port, v);
        }
        a.midi_in(0x90);
        a.midi_in(0x40);
        a.run(500, &mut out);
        let saved = state::to_bytes(&a);
        let mut rest_a = Vec::new();
        a.run(3000, &mut rest_a);
        let mut b = board();
        state::State::load(&mut b, &mut state::Reader::new(&saved)).unwrap();
        let mut rest_b = Vec::new();
        b.run(3000, &mut rest_b);
        assert!(rest_a.iter().any(|&v| v != 0));
        assert_eq!(rest_a, rest_b);
        assert_eq!(a.cpu.pc, b.cpu.pc);
    }
}
