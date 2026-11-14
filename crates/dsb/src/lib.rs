//! Sega's MPEG music boards: the DSB2 ("Digital Sound Board 2") of
//! `topskatr`, below, and the DSB1 of `stcc` ([`dsb1`]).
//!
//! The DSB2:
//!
//! A 68000 runs Sega's firmware from its own ROM. It takes the host's
//! MIDI-style byte stream on a UART (IRQ1), keeps channel-14 messages (`AE`)
//! and forwards everything else out of its own UART, and drives an MPEG
//! decoder that reads MPEG-1 Layer II frames straight from the MPEG ROMs.
//! A periodic tick (IRQ2) times fades and loop-point updates.
//!
//! Memory map (`docs/hw/sound.md` §3.2):
//!
//! | address | device |
//! |---|---|
//! | `000000-01FFFF` | program ROM |
//! | `C00001`/`C00003` | UART data / control and status |
//! | `D00001` | latch: bit 5 decoder reset (low), bit 4 tick acknowledge (low pulse), bits 3-0 LEDs |
//! | `E00001`/`E00003` | decoder ports: boot upload / command bytes |
//! | `E80001` | decoder status: bit 1 busy (never set here) |
//! | `F00000-F1FFFF` | 128 KB RAM |
//!
//! The decoder runs at 32 kHz; [`Dsb2::run`] resamples to 44.1 kHz.

pub mod audio;
mod decoder;
pub mod dsb1;
pub mod soundtrack;

pub use decoder::Event;
use decoder::MpegChip;
pub use dsb1::Dsb1;
pub use soundtrack::{Manifest, Replacement, Soundtrack};
use std::collections::{HashMap, VecDeque};

/// 68000 clock: 33 MHz / 3.
pub const CPU_HZ: u64 = 11_000_000;
/// Output sample rate of [`Dsb2::run`].
pub const OUT_HZ: u64 = 44_100;
/// MPEG decoder output rate.
pub const MPEG_HZ: u64 = 32_000;
/// Default IRQ2 rate. The firmware does not pin it down; 1 kHz is the
/// MAME/Supermodel figure.
pub const TICK_HZ: u64 = 1_000;
/// One byte at 31.25 kbaud, 10 bits per byte.
const BYTE_CYCLES: u64 = CPU_HZ * 10 / 31_250;

const ROM_SIZE: usize = 0x20000;
const RAM_BASE: u32 = 0xf00000;
const RAM_SIZE: usize = 0x20000;

struct Bus {
    rom: Vec<u8>,
    ram: Vec<u8>,
    latch: u8,
    rx: Option<u8>,
    rx_queue: VecDeque<u8>,
    tx: Vec<u8>,
    tick_pending: bool,
    chip: MpegChip,
    upload: u64,
    /// Unmapped accesses by address; bit 31 set for writes.
    unmapped: HashMap<u32, u64>,
}

impl Bus {
    fn read(&mut self, a: u32) -> u8 {
        let a = a & 0xff_ffff;
        match a {
            0..=0x1ffff => self.rom[a as usize],
            0xf00000..=0xf1ffff => self.ram[(a - RAM_BASE) as usize],
            0xc00001 => self.rx.take().unwrap_or(0),
            // TxRDY and TxEMPTY always; RxRDY when a byte is waiting.
            0xc00003 => 0x05 | (self.rx.is_some() as u8) << 1,
            0xe80001 => 0,
            _ => {
                *self.unmapped.entry(a).or_default() += 1;
                0xff
            }
        }
    }

    fn write(&mut self, a: u32, v: u8) {
        let a = a & 0xff_ffff;
        match a {
            0xf00000..=0xf1ffff => self.ram[(a - RAM_BASE) as usize] = v,
            0xc00001 => self.tx.push(v),
            0xc00003 => {}
            0xd00001 => {
                if self.latch & 0x20 != 0 && v & 0x20 == 0 {
                    self.chip.reset();
                }
                if v & 0x10 == 0 {
                    self.tick_pending = false;
                }
                self.latch = v;
            }
            0xe00001 => self.upload += 1,
            0xe00003 => self.chip.write(v),
            _ => *self.unmapped.entry(a | 0x8000_0000).or_default() += 1,
        }
    }
}

impl m68k::Bus for Bus {
    fn read8(&mut self, a: u32) -> u8 {
        self.read(a)
    }
    fn read16(&mut self, a: u32) -> u16 {
        (self.read(a) as u16) << 8 | self.read(a | 1) as u16
    }
    fn write8(&mut self, a: u32, v: u8) {
        self.write(a, v)
    }
    fn write16(&mut self, a: u32, v: u16) {
        self.write(a, (v >> 8) as u8);
        self.write(a | 1, v as u8);
    }
}

/// Counters for diagnostics.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub instructions: u64,
    pub cycles: u64,
    pub ticks: u64,
    /// Bytes delivered to the UART receiver.
    pub rx_bytes: u64,
    /// Bytes that arrived before the firmware read the previous one.
    pub overruns: u64,
    /// Bytes written to the decoder's upload port `0xE00001`.
    pub upload_bytes: u64,
    /// MPEG frames decoded.
    pub frames: u64,
}

/// The DSB2 board.
pub struct Dsb2 {
    cpu: m68k::Cpu,
    bus: Bus,
    stats: Stats,
    tick_cycles: u64,
    next_tick: u64,
    next_rx: u64,
    out_samples: u64,
    /// Resampler: position between `hist[1]` and `hist[2]`, and the last
    /// four 32 kHz samples.
    phase: f64,
    hist: [(f32, f32); 4],
    /// PC sampled once per output sample.
    pcs: HashMap<u32, u64>,
    /// Decoder events with their time in output samples.
    events: Vec<(u64, Event)>,
    /// Keep `pcs` and `events` (off for live playback, where they would
    /// cost time every sample and grow without bound).
    diagnostics: bool,
}

impl Dsb2 {
    /// `program` is the DSB2 program chip as dumped (16-bit words with
    /// their bytes swapped); `mpeg` is the MPEG ROM space, plain bytes.
    pub fn new(program: &[u8], mpeg: Vec<u8>) -> Self {
        let mut rom = vec![0xff; ROM_SIZE];
        for (d, s) in rom.as_chunks_mut::<2>().0.iter_mut().zip(program.as_chunks::<2>().0) {
            *d = [s[1], s[0]];
        }
        let bus = Bus {
            rom,
            ram: vec![0; RAM_SIZE],
            latch: 0xff,
            rx: None,
            rx_queue: VecDeque::new(),
            tx: Vec::new(),
            tick_pending: false,
            chip: MpegChip::new(mpeg),
            upload: 0,
            unmapped: HashMap::new(),
        };
        let mut d = Dsb2 {
            cpu: m68k::Cpu::new(),
            bus,
            stats: Stats::default(),
            tick_cycles: CPU_HZ / TICK_HZ,
            next_tick: CPU_HZ / TICK_HZ,
            next_rx: 0,
            out_samples: 0,
            phase: 0.0,
            hist: [(0.0, 0.0); 4],
            pcs: HashMap::new(),
            events: Vec::new(),
            diagnostics: true,
        };
        d.cpu.reset(&mut d.bus);
        d
    }

    /// Record the PC histogram and decoder events (on by default; turn off
    /// for long live playback).
    pub fn set_diagnostics(&mut self, on: bool) {
        self.diagnostics = on;
    }

    /// Queue a byte from the host link; it reaches the UART receiver at
    /// 31.25 kbaud after any bytes already queued.
    pub fn midi_in(&mut self, byte: u8) {
        self.bus.rx_queue.push_back(byte);
    }

    /// Run for `samples` output samples, appending interleaved stereo 16-bit
    /// PCM at 44.1 kHz to `out`.
    pub fn run(&mut self, samples: usize, out: &mut Vec<i16>) {
        out.reserve(samples * 2);
        let step = MPEG_HZ as f64 / OUT_HZ as f64;
        for _ in 0..samples {
            self.out_samples += 1;
            self.run_cpu(self.out_samples * CPU_HZ / OUT_HZ);
            self.phase += step;
            while self.phase >= 1.0 {
                self.phase -= 1.0;
                self.hist.rotate_left(1);
                self.hist[3] = self.bus.chip.next();
            }
            let (l, r) = cubic(&self.hist, self.phase as f32);
            let (rl, rr) = self.bus.chip.replacement_next();
            let (l, r) = (l + rl, r + rr);
            out.push((l * 32767.0).clamp(-32768.0, 32767.0) as i16);
            out.push((r * 32767.0).clamp(-32768.0, 32767.0) as i16);
            if self.diagnostics {
                *self.pcs.entry(self.cpu.pc).or_default() += 1;
                let t = self.out_samples;
                self.events.extend(self.bus.chip.events.drain(..).map(|e| (t, e)));
            } else {
                self.bus.chip.events.clear();
            }
        }
    }

    fn run_cpu(&mut self, until: u64) {
        while self.stats.cycles < until {
            let now = self.stats.cycles;
            if now >= self.next_tick {
                self.next_tick += self.tick_cycles;
                self.bus.tick_pending = true;
                self.stats.ticks += 1;
            }
            if now >= self.next_rx {
                if let Some(v) = self.bus.rx_queue.pop_front() {
                    self.stats.overruns += self.bus.rx.is_some() as u64;
                    self.bus.rx = Some(v);
                    self.stats.rx_bytes += 1;
                    self.next_rx = now + BYTE_CYCLES;
                }
            }
            let level = if self.bus.tick_pending { 2 } else { self.bus.rx.is_some() as u8 };
            self.cpu.set_irq(level);
            self.stats.cycles += self.cpu.step(&mut self.bus) as u64;
            self.stats.instructions += 1;
        }
    }

    pub fn stats(&self) -> Stats {
        Stats { upload_bytes: self.bus.upload, frames: self.bus.chip.frames, ..self.stats.clone() }
    }

    /// Decoder events so far, with their time in 44.1 kHz output samples.
    pub fn events(&self) -> &[(u64, Event)] {
        &self.events
    }

    /// Program counter samples (one per output sample), most frequent first.
    pub fn pc_histogram(&self) -> Vec<(u32, u64)> {
        let mut v: Vec<_> = self.pcs.iter().map(|(&a, &n)| (a, n)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }

    /// Bytes the firmware forwarded out of its own UART.
    pub fn tx(&self) -> &[u8] {
        &self.bus.tx
    }

    /// Unmapped accesses (address, count); bit 31 marks writes.
    pub fn unmapped(&self) -> Vec<(u32, u64)> {
        let mut v: Vec<_> = self.bus.unmapped.iter().map(|(&a, &n)| (a, n)).collect();
        v.sort();
        v
    }

    pub fn halted(&self) -> bool {
        self.cpu.halted()
    }

    /// The decoder's gain registers `A0`-`A7`, `B0`-`B7`.
    pub fn gains(&self) -> [u8; 16] {
        self.bus.chip.gain
    }

    pub fn playing(&self) -> bool {
        self.bus.chip.playing()
    }

    /// The firmware's track table: entry n - 1 is the MPEG ROM range
    /// `[start, end)` that `AE 10 n` plays. Read from the layout of
    /// topskatr's firmware (`docs/hw/sound.md` §3.7): a pointer at `0x8000`
    /// to the entry pointers, the track count + 1 at `0x800A`, and in each
    /// entry a flags word then the 32-bit start and end. Empty when the
    /// program does not look like that.
    pub fn track_table(&self) -> Vec<(u32, u32)> {
        let rom = &self.bus.rom;
        let be32 = |a: usize| rom.get(a..a + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap()));
        let (Some(table), Some(count)) = (be32(0x8000), rom.get(0x800a..0x800c)) else { return vec![] };
        let count = u16::from_be_bytes([count[0], count[1]]) as usize;
        if !(2..=128).contains(&count) {
            return vec![];
        }
        let mut v = vec![];
        for i in 0..count - 1 {
            let entry = be32(table as usize + 4 * i).map(|e| e as usize);
            let range = entry.and_then(|e| Some((be32(e + 2)? & 0xff_ffff, be32(e + 6)? & 0xff_ffff)));
            match range {
                Some((s, e)) if s < e => v.push((s, e)),
                _ => return vec![],
            }
        }
        v
    }

    /// Play replacement tracks from `st` in place of the ROM ranges it
    /// replaces.
    pub fn set_soundtrack(&mut self, st: Soundtrack) {
        self.bus.chip.soundtrack = Some(st);
    }

    pub fn soundtrack(&self) -> Option<&Soundtrack> {
        self.bus.chip.soundtrack.as_ref()
    }

    /// The soundtrack track playing in place of the ROM, if any.
    pub fn replacing(&self) -> Option<u8> {
        self.bus.chip.replacing()
    }
}

/// Catmull-Rom interpolation between `h[1]` and `h[2]`.
pub(crate) fn cubic(h: &[(f32, f32); 4], t: f32) -> (f32, f32) {
    let f = |p0: f32, p1: f32, p2: f32, p3: f32| {
        p1 + 0.5 * t * (p2 - p0 + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + t * (3.0 * (p1 - p2) + p3 - p0)))
    };
    (f(h[0].0, h[1].0, h[2].0, h[3].0), f(h[0].1, h[1].1, h[2].1, h[3].1))
}

// Save states. ROMs and diagnostics (PC histogram, events, forwarded bytes,
// unmapped accesses) stay as they are.
state::fields!(Dsb2 { cpu, bus, stats, tick_cycles, next_tick, next_rx, out_samples, phase, hist });
state::fields!(Bus { ram, latch, rx, rx_queue, tick_pending, chip, upload });
state::fields!(Stats { instructions, cycles, ticks, rx_bytes, overruns, upload_bytes, frames });

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny program (stored byte-swapped, as dumped) that writes a
    /// decoder command sequence and then spins.
    fn board(code: &[u16], mpeg: Vec<u8>) -> Dsb2 {
        let mut words: Vec<u16> = vec![0x00f0, 0xfffe, 0x0000, 0x0400];
        words.resize(0x200, 0xffff);
        words.extend(code);
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        Dsb2::new(&bytes, mpeg)
    }

    /// `move.b #v,$e00003.l`
    fn cmd(v: u8) -> [u16; 4] {
        [0x13fc, v as u16, 0x00e0, 0x0003]
    }

    #[test]
    fn plays_frames_between_start_and_end() {
        // Two silent 32 kHz 128 kbit/s stereo frames at 0x100.
        let mut mpeg = vec![0; 0x1000];
        for f in 0..2 {
            mpeg[0x100 + 576 * f..][..4].copy_from_slice(&[0xff, 0xfd, 0x88, 0x00]);
        }
        let end = 0x100 + 2 * 576;
        let mut code = vec![];
        for b in [0x14, 0, 1, 0, 0x24, 0, (end >> 8) as u8, end as u8, 0xa0, 0xff, 0xa5, 0xff, 0x74] {
            code.extend(cmd(b));
        }
        code.push(0x60fe); // bra.s *
        let mut d = board(&code, mpeg);
        let mut out = vec![];
        d.run(4410, &mut out);
        assert_eq!(out.len(), 8820);
        assert!(out.iter().all(|&s| s == 0));
        let ev: Vec<_> = d.events().iter().map(|e| e.1).collect();
        assert!(ev.contains(&Event::Play { start: 0x100, end, looped: false }), "{ev:?}");
        assert!(ev.contains(&Event::SegmentEnd { restart: None }));
        assert_eq!(d.stats().frames, 2);
        assert!(!d.playing());
    }

    #[test]
    fn uart_bytes_raise_irq1_at_line_rate() {
        // IRQ1 handler at 0x500: read the UART into RAM, count, rte.
        // The tick handler at 0x520 just acknowledges (latch bit 4 low).
        let mut d = {
            let mut words: Vec<u16> = vec![0x00f0, 0xfffe, 0x0000, 0x0400];
            words.resize(0x32, 0xffff);
            words.extend([0x0000, 0x0500, 0x0000, 0x0520]); // autovectors 1 and 2
            words.resize(0x200, 0xffff);
            words.extend([0x4e72, 0x2000, 0x60fa]); // 0x400: stop #$2000; bra.s 0x400
            words.resize(0x280, 0xffff);
            // 0x500: move.b $c00001,$f00010; addq.b #1,$f00011; rte
            words.extend([0x13f9, 0x00c0, 0x0001, 0x00f0, 0x0010, 0x5239, 0x00f0, 0x0011, 0x4e73]);
            words.resize(0x290, 0xffff);
            words.extend([0x13fc, 0x0000, 0x00d0, 0x0001, 0x4e73]); // 0x520: move.b #0,$d00001; rte
            let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
            Dsb2::new(&bytes, vec![])
        };
        for b in [0xae, 0x10, 0x05] {
            d.midi_in(b);
        }
        let mut out = vec![];
        d.run(441, &mut out); // 10 ms: three bytes take under 1 ms
        assert_eq!(d.bus.ram[0x10], 0x05);
        assert_eq!(d.bus.ram[0x11], 3);
        assert_eq!(d.stats().overruns, 0);
    }

    #[test]
    fn reads_the_firmware_track_table() {
        // Table pointer 0x8010, count 3 (two tracks); entries as in topskatr:
        // flags, start, end, fade-in, volumes, [loop start].
        let mut words: Vec<u16> = vec![0x00f0, 0xfffe, 0x0000, 0x0400];
        words.resize(0x4000, 0xffff);
        words.extend([0x0000, 0x8010, 0, 0, 0xff, 0x0003, 0, 0]);
        words.extend([0x0000, 0x8020, 0x0000, 0x8040, 0, 0, 0, 0]);
        words.extend([0x8002, 0x0001, 0x1000, 0x0041, 0x3fc0, 0, 0x8080, 0x0001, 0x1000, 0, 0, 0, 0, 0, 0, 0]);
        words.extend([0x0002, 0x0096, 0xfbc2, 0x00ab, 0x68c2, 0, 0x8080, 0]);
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let d = Dsb2::new(&bytes, vec![]);
        assert_eq!(d.track_table(), vec![(0x011000, 0x413fc0), (0x96fbc2, 0xab68c2)]);
        // No table: empty.
        assert!(board(&[0x60fe], vec![]).track_table().is_empty());
    }
}
