//! Sega DSB1 ("Digital Sound Board"), the MPEG music board of `stcc`.
//!
//! A Z80 runs Sega's "MPEG SOUND DRIVER Z80" from its own ROM. It takes the
//! host's MIDI-style byte stream on an 8251-style UART (interrupt mode 1 on
//! receive), keeps channel-15 messages (`AF`), and drives an MPEG decoder
//! through I/O ports: start and end addresses, a play mode, an attenuation,
//! and the decoder's current address read back. Worked out from the program
//! alone (`docs/hw/dsb1.md`).
//!
//! | address / port | device |
//! |---|---|
//! | `0000-7FFF` | program ROM (the first 32 KB of the chip; the program uses 8 KB) |
//! | `8000-FFFF` | 32 KB RAM |
//! | port `E0` | play mode: 0 stop, 1 play to the end, 2 play and continue |
//! | ports `E2`-`E4` | write: start address; read: the decoder's current address |
//! | ports `E5`-`E7` | end address (the segment's last byte) |
//! | port `E8` | attenuation, `00` loudest to `7F` silent (bit 7 set: setup values, ignored) |
//! | ports `E9`, `EB` | written per track and at boot; meaning unknown, ignored |
//! | port `EA` | LEDs |
//! | ports `F0`/`F1` | UART data / control and status |
//!
//! The decoder is the DSB2's [`MpegChip`] driven through these ports: a
//! "continue" play, at the end of its segment, carries on at the start and
//! end addresses written meanwhile, which is how the firmware chains a
//! track's segments and loops.

use crate::decoder::{Event, MpegChip};
use crate::soundtrack::Soundtrack;
use crate::{cubic, MPEG_HZ, OUT_HZ};
use std::collections::{HashMap, VecDeque};

/// Z80 clock. *Unconfirmed*: the program doesn't pin it down; 4 MHz fits
/// the UART's x16 mode at 31.25 kbaud from the CPU clock divided by 8.
pub const CPU_HZ: u64 = 4_000_000;
/// One byte at 31.25 kbaud, 10 bits per byte.
const BYTE_CYCLES: u64 = CPU_HZ * 10 / 31_250;

const ROM_SIZE: usize = 0x8000;
const RAM_SIZE: usize = 0x8000;

struct Bus {
    rom: Vec<u8>,
    ram: Vec<u8>,
    rx: Option<u8>,
    rx_queue: VecDeque<u8>,
    tx: u64,
    chip: MpegChip,
    /// Ports `E2`-`E7` as last written: start, then end, MSB first.
    addr: [u8; 6],
    mode: u8,
    atten: u8,
    leds: u8,
    /// Accesses to ports with no known meaning: (port, value written or
    /// `None` for a read).
    other: HashMap<(u8, Option<u8>), u64>,
}

impl Bus {
    fn be24(b: &[u8]) -> u32 {
        (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32
    }
}

impl z80::Bus for Bus {
    fn read(&mut self, a: u16) -> u8 {
        match a {
            0..=0x7fff => self.rom[a as usize],
            _ => self.ram[a as usize - 0x8000],
        }
    }

    fn write(&mut self, a: u16, v: u8) {
        if a >= 0x8000 {
            self.ram[a as usize - 0x8000] = v;
        }
    }

    fn input(&mut self, port: u16) -> u8 {
        match port as u8 {
            0xf0 => self.rx.take().unwrap_or(0),
            // TxRDY and TxEMPTY always; RxRDY when a byte is waiting.
            0xf1 => 0x05 | (self.rx.is_some() as u8) << 1,
            p @ 0xe2..=0xe4 => (self.chip.position() >> (8 * (0xe4 - p))) as u8,
            p => {
                *self.other.entry((p, None)).or_default() += 1;
                0xff
            }
        }
    }

    fn output(&mut self, port: u16, v: u8) {
        match port as u8 {
            0xf0 => self.tx += 1,
            0xf1 => {}
            p @ 0xe2..=0xe7 => {
                self.addr[(p - 0xe2) as usize] = v;
                // The address takes effect with its low byte; the firmware
                // always writes all three, MSB first.
                match p {
                    0xe4 => self.chip.set_start(Bus::be24(&self.addr[..3])),
                    0xe7 => self.chip.set_end(Bus::be24(&self.addr[3..]) + 1),
                    _ => {}
                }
            }
            0xe0 => {
                self.mode = v;
                match v {
                    0 => self.chip.stop(),
                    _ => {
                        self.chip.set_looped(v & 2 != 0);
                        if !self.chip.playing() {
                            self.chip.play();
                        }
                    }
                }
            }
            0xe8 if v < 0x80 => {
                if self.atten != v {
                    self.chip.events.push(Event::Attenuation(v));
                }
                self.atten = v;
            }
            0xea => self.leds = v,
            p => *self.other.entry((p, Some(v))).or_default() += 1,
        }
    }
}

/// Output gain for an attenuation value: linear from 1 at `00` to silence at
/// `7F`. *Unconfirmed*: the firmware only shows that `7F` is silent (fades
/// out end there and stop the decoder) and that smaller is louder.
fn atten_gain(v: u8) -> f32 {
    (0x7f - v.min(0x7f)) as f32 / 127.0
}

/// Counters for diagnostics.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub instructions: u64,
    pub cycles: u64,
    /// Bytes delivered to the UART receiver.
    pub rx_bytes: u64,
    /// Bytes that arrived before the program read the previous one.
    pub overruns: u64,
    /// Bytes the program echoed out of its UART.
    pub tx_bytes: u64,
    /// MPEG frames decoded.
    pub frames: u64,
}

/// The DSB1 board.
pub struct Dsb1 {
    cpu: z80::Cpu,
    bus: Bus,
    stats: Stats,
    next_rx: u64,
    out_samples: u64,
    /// Resampler state, as in the DSB2.
    phase: f64,
    hist: [(f32, f32); 4],
    pcs: HashMap<u16, u64>,
    events: Vec<(u64, Event)>,
    diagnostics: bool,
}

impl Dsb1 {
    /// `program` is the Z80 program chip; `mpeg` is the MPEG ROM space.
    pub fn new(program: &[u8], mpeg: Vec<u8>) -> Self {
        let mut rom = vec![0xff; ROM_SIZE];
        let n = program.len().min(ROM_SIZE);
        rom[..n].copy_from_slice(&program[..n]);
        let mut chip = MpegChip::new(mpeg);
        // Straight through: the DSB1 has no gain matrix of its own.
        chip.set_gain(0xa0, 0xff);
        chip.set_gain(0xa5, 0xff);
        chip.events.clear();
        let bus = Bus {
            rom,
            ram: vec![0; RAM_SIZE],
            rx: None,
            rx_queue: VecDeque::new(),
            tx: 0,
            chip,
            addr: [0; 6],
            mode: 0,
            atten: 0x7f,
            leds: 0,
            other: HashMap::new(),
        };
        Dsb1 {
            cpu: z80::Cpu::new(),
            bus,
            stats: Stats::default(),
            next_rx: 0,
            out_samples: 0,
            phase: 0.0,
            hist: [(0.0, 0.0); 4],
            pcs: HashMap::new(),
            events: Vec::new(),
            diagnostics: true,
        }
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
            let g = atten_gain(self.bus.atten);
            let (l, r) = ((l + rl) * g, (r + rr) * g);
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
            if now >= self.next_rx {
                if let Some(v) = self.bus.rx_queue.pop_front() {
                    self.stats.overruns += self.bus.rx.is_some() as u64;
                    self.bus.rx = Some(v);
                    self.stats.rx_bytes += 1;
                    self.next_rx = now + BYTE_CYCLES;
                }
            }
            // RxRDY drives INT.
            self.cpu.set_int(self.bus.rx.is_some());
            self.stats.cycles += self.cpu.step(&mut self.bus) as u64;
            self.stats.instructions += 1;
        }
    }

    pub fn stats(&self) -> Stats {
        Stats { tx_bytes: self.bus.tx, frames: self.bus.chip.frames, ..self.stats.clone() }
    }

    /// Decoder events so far, with their time in 44.1 kHz output samples.
    pub fn events(&self) -> &[(u64, Event)] {
        &self.events
    }

    /// Program counter samples (one per output sample), most frequent first.
    pub fn pc_histogram(&self) -> Vec<(u16, u64)> {
        let mut v: Vec<_> = self.pcs.iter().map(|(&a, &n)| (a, n)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }

    /// Writes to ports with no known meaning, as (port, value, count), and
    /// reads of unknown ports as (port, `None`, count).
    pub fn other_ports(&self) -> Vec<(u8, Option<u8>, u64)> {
        let mut v: Vec<_> = self.bus.other.iter().map(|(&(p, v), &n)| (p, v, n)).collect();
        v.sort();
        v
    }

    pub fn halted(&self) -> bool {
        self.cpu.halted()
    }

    pub fn playing(&self) -> bool {
        self.bus.chip.playing()
    }

    /// Port `E8` as last written.
    pub fn attenuation(&self) -> u8 {
        self.bus.atten
    }

    /// The firmware's track list: entry n - 1 is the MPEG ROM range
    /// `[start, end)` of the first segment `AF 6F n` plays. Read from the
    /// layout of the "MPEG SOUND DRIVER Z80" program (`docs/hw/dsb1.md`):
    /// a header at ROM `0x1000` (copied to RAM `0xA000`) with pointers to
    /// the track entries and the segment table and the track count + 1;
    /// each entry has a fade rate, a volume, a port `E9` value, then segment
    /// numbers. Empty when the program does not look like that.
    pub fn track_table(&self) -> Vec<(u32, u32)> {
        let rom = &self.bus.rom;
        // RAM 0xA000-0xFFFF is ROM 0x1000-0x6FFF.
        let at = |ram: usize| ram.checked_sub(0x9000).and_then(|a| rom.get(a).copied());
        let le16 = |ram: usize| Some(at(ram)? as usize | (at(ram + 1)? as usize) << 8);
        let (Some(tracks), Some(segs), Some(count)) = (le16(0xa000), le16(0xa002), at(0xa004)) else {
            return vec![];
        };
        if !(2..=128).contains(&count) {
            return vec![];
        }
        let mut v = vec![];
        for i in 0..count as usize - 1 {
            let seg = le16(tracks + 2 * i).and_then(|e| {
                // Fade-in rate (2 bytes; zero: none), volume, port E9
                // value, then the first segment.
                let n = at(e + 4)?;
                (n < 0xf0).then_some(segs + 6 * n as usize)
            });
            let range = seg.and_then(|s| {
                let b: Option<Vec<u8>> = (0..6).map(|k| at(s + k)).collect();
                let b = b?;
                Some((Bus::be24(&b[..3]), Bus::be24(&b[3..]) + 1))
            });
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

// Save states. ROMs and diagnostics stay as they are.
state::fields!(Dsb1 { cpu, bus, stats, next_rx, out_samples, phase, hist });
state::fields!(Bus { ram, rx, rx_queue, tx, chip, addr, mode, atten, leds });
state::fields!(Stats { instructions, cycles, rx_bytes, overruns, tx_bytes, frames });

#[cfg(test)]
mod tests {
    use super::*;

    /// A program at 0 (padded to the ROM size) with an interrupt handler.
    fn board(code: &[u8], handler: &[u8], mpeg: Vec<u8>) -> Dsb1 {
        let mut rom = code.to_vec();
        rom.resize(0x38, 0xff);
        rom.extend(handler);
        Dsb1::new(&rom, mpeg)
    }

    fn silent_frames(n: usize, at: usize) -> Vec<u8> {
        let mut mpeg = vec![0; 0x2000];
        for f in 0..n {
            mpeg[at + 576 * f..][..4].copy_from_slice(&[0xff, 0xfd, 0x88, 0x00]);
        }
        mpeg
    }

    #[test]
    fn uart_bytes_interrupt_at_line_rate() {
        // im 1; ei; loop: jr loop. Handler: in a,(f0); ld (8000),a;
        // ld hl,8001; inc (hl); ei; ret.
        let code = [0xed, 0x56, 0xfb, 0x18, 0xfe];
        let handler = [0xdb, 0xf0, 0x32, 0x00, 0x80, 0x21, 0x01, 0x80, 0x34, 0xfb, 0xc9];
        let mut d = board(&code, &handler, vec![]);
        for b in [0xaf, 0x6f, 0x03] {
            d.midi_in(b);
        }
        let mut out = vec![];
        d.run(441, &mut out);
        assert_eq!(d.bus.ram[0], 0x03);
        assert_eq!(d.bus.ram[1], 3);
        assert_eq!(d.stats().overruns, 0);
    }

    #[test]
    fn continue_mode_chains_segments_and_reads_back_the_position() {
        // Segment A: two frames at 0x100; segment B: one frame at 0x600.
        let mut mpeg = silent_frames(2, 0x100);
        mpeg[0x600..0x604].copy_from_slice(&[0xff, 0xfd, 0x88, 0x00]);
        let a_end = 0x100 + 2 * 576 - 1;
        let b_end = 0x600 + 576 - 1;
        let mut code = vec![];
        let mut out_ = |p: u8, v: u8| code.extend([0x3e, v, 0xd3, p]); // ld a,v; out (p),a
        for (p, v) in [(0xe2, 0), (0xe3, 1), (0xe4, 0), (0xe5, 0), (0xe6, (a_end >> 8) as u8), (0xe7, a_end as u8)] {
            out_(p, v);
        }
        out_(0xe8, 0);
        out_(0xe0, 2);
        // Queue B while A plays.
        for (p, v) in [(0xe2, 0), (0xe3, 6), (0xe4, 0), (0xe5, 0), (0xe6, (b_end >> 8) as u8), (0xe7, b_end as u8)] {
            out_(p, v);
        }
        code.extend([0x18, 0xfe]);
        let mut d = board(&code, &[], mpeg);
        let mut out = vec![];
        d.run(10, &mut out);
        assert!(d.playing());
        // 90 ms: A's two frames and B's have been read.
        d.run(3990, &mut out);
        assert!(d.playing());
        assert_eq!(d.bus.chip.position(), b_end + 1);
        // Play to the end: B finishes and the decoder stops.
        z80::Bus::output(&mut d.bus, 0xe0, 1);
        d.run(4000, &mut out);
        let ev: Vec<_> = d.events().iter().map(|e| e.1).collect();
        assert!(ev.contains(&Event::Play { start: 0x100, end: a_end + 1, looped: true }), "{ev:?}");
        assert!(ev.contains(&Event::LoopStart(0x600)));
        assert!(ev.contains(&Event::SegmentEnd { restart: Some(0x600) }));
        assert!(ev.contains(&Event::SegmentEnd { restart: None }));
        assert_eq!(d.stats().frames, 3);
        assert!(!d.playing());
        // The position reads back through E2-E4.
        assert_eq!(z80::Bus::input(&mut d.bus, 0xe3), ((b_end + 1) >> 8) as u8);
    }

    #[test]
    fn attenuation_scales_the_output() {
        assert_eq!(atten_gain(0), 1.0);
        assert_eq!(atten_gain(0x7f), 0.0);
        assert!(atten_gain(0x40) < atten_gain(0x20));
    }

    #[test]
    fn save_state_round_trip() {
        let code = [0xed, 0x56, 0xfb, 0x18, 0xfe];
        let handler = [0xdb, 0xf0, 0x32, 0x00, 0x80, 0xfb, 0xc9];
        let mut a = board(&code, &handler, vec![]);
        a.midi_in(0x42);
        a.run(100, &mut vec![]);
        let bytes = state::to_bytes(&a);
        let mut b = board(&code, &handler, vec![]);
        let mut r = state::Reader::new(&bytes);
        state::State::load(&mut b, &mut r).unwrap();
        assert_eq!(b.bus.ram[0], 0x42);
        assert_eq!(state::to_bytes(&b), bytes);
    }
}
