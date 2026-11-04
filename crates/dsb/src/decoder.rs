//! The DSB2's MPEG decoder chip (Sega 315-6028A) as the firmware drives it.
//!
//! A command byte stream on `0xE00003`, as worked out from topskatr's DSB2
//! firmware (see `docs/hw/sound.md` §3.4):
//!
//! | bytes | meaning |
//! |---|---|
//! | `14`/`15` + 3 | start address (24-bit byte address in the MPEG ROM, MSB first) |
//! | `24`/`25` + 3 | end address |
//! | `64`/`65` | loop: at the end, restart from the start address current then |
//! | `74`/`75` | play from start to end |
//! | `80`-`87` | stop (also clears loop) |
//! | `A0`-`A7`, `B0`-`B7` + 1 | output gain registers; `A0` L←L, `A1` L←R, `A4` R←L, `A5` R←R |
//!
//! The low opcode bit (MAME: player 0/1) is ignored: one player is modelled.
//! Other bytes, such as the boot upload, are ignored.

use mpeg::{Decoder, FRAME_SAMPLES};

/// Something the decoder did, for diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Play {
        start: u32,
        end: u32,
        looped: bool,
    },
    Stop,
    /// A start address written while playing (the next loop point).
    LoopStart(u32),
    /// End of a segment: restarted at `start`, or stopped.
    SegmentEnd {
        restart: Option<u32>,
    },
    Gain {
        reg: u8,
        value: u8,
    },
    /// No frame header at this address: playback stops.
    BadFrame(u32),
    /// A command byte with no known meaning.
    Unknown(u8),
}

pub(crate) struct MpegChip {
    rom: Vec<u8>,
    cmd: u8,
    args: Vec<u8>,
    need: usize,
    start: u32,
    end: u32,
    looped: bool,
    playing: bool,
    cur: u32,
    seg_end: u32,
    /// Gain registers `A0`-`A7` then `B0`-`B7`.
    pub gain: [u8; 16],
    dec: Decoder,
    pcm: Box<[[f32; FRAME_SAMPLES]; 2]>,
    at: usize,
    pub events: Vec<Event>,
    pub frames: u64,
}

impl MpegChip {
    pub fn new(rom: Vec<u8>) -> Self {
        MpegChip {
            rom,
            cmd: 0,
            args: Vec::new(),
            need: 0,
            start: 0,
            end: 0,
            looped: false,
            playing: false,
            cur: 0,
            seg_end: 0,
            gain: [0; 16],
            dec: Decoder::new(),
            pcm: Box::new([[0.0; FRAME_SAMPLES]; 2]),
            at: FRAME_SAMPLES,
            events: Vec::new(),
            frames: 0,
        }
    }

    /// Chip reset (latch bit 5 low): stop and forget everything.
    pub fn reset(&mut self) {
        let rom = std::mem::take(&mut self.rom);
        let events = std::mem::take(&mut self.events);
        *self = MpegChip { events, frames: self.frames, ..MpegChip::new(rom) };
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    /// A byte written to the command port.
    pub fn write(&mut self, v: u8) {
        if self.need > 0 {
            self.args.push(v);
            self.need -= 1;
            if self.need == 0 {
                self.execute();
            }
            return;
        }
        self.cmd = v;
        self.args.clear();
        self.need = match v {
            0x14 | 0x15 | 0x24 | 0x25 => 3,
            0xa0..=0xa7 | 0xb0..=0xb7 => 1,
            _ => 0,
        };
        if self.need == 0 {
            self.execute();
        }
    }

    fn execute(&mut self) {
        let addr = || self.args.iter().fold(0u32, |a, &b| a << 8 | b as u32);
        match self.cmd {
            0x14 | 0x15 => {
                self.start = addr();
                if self.playing {
                    self.events.push(Event::LoopStart(self.start));
                }
            }
            0x24 | 0x25 => self.end = addr(),
            0x64 | 0x65 => self.looped = true,
            0x74 | 0x75 => {
                self.playing = true;
                self.cur = self.start;
                self.seg_end = self.end;
                self.at = FRAME_SAMPLES;
                self.dec.reset();
                self.events.push(Event::Play { start: self.start, end: self.end, looped: self.looped });
            }
            0x80..=0x87 => {
                if self.playing {
                    self.events.push(Event::Stop);
                }
                self.playing = false;
                self.looped = false;
            }
            c @ (0xa0..=0xa7 | 0xb0..=0xb7) => {
                let reg = (c & 7) | (c & 0x10) >> 1;
                if self.gain[reg as usize] != self.args[0] {
                    self.events.push(Event::Gain { reg: c, value: self.args[0] });
                }
                self.gain[reg as usize] = self.args[0];
            }
            c => self.events.push(Event::Unknown(c)),
        }
    }

    /// The next 32 kHz stereo sample after the gain matrix.
    pub fn next(&mut self) -> (f32, f32) {
        if self.at >= FRAME_SAMPLES && !self.refill() {
            return (0.0, 0.0);
        }
        let (l, r) = (self.pcm[0][self.at], self.pcm[1][self.at]);
        self.at += 1;
        let g = |i: usize| self.gain[i] as f32 / 255.0;
        (g(0) * l + g(1) * r, g(4) * l + g(5) * r)
    }

    /// Decode the next frame; false when not playing.
    fn refill(&mut self) -> bool {
        if !self.playing {
            return false;
        }
        let rom_end = self.rom.len() as u32;
        let frame_fits = |cur: u32, end: u32, rom: &[u8]| {
            mpeg::Header::parse(rom.get(cur as usize..).unwrap_or(&[]))
                .is_some_and(|h| cur + h.frame_len() as u32 <= end.min(rom_end))
        };
        if !frame_fits(self.cur, self.seg_end, &self.rom) {
            if self.looped {
                self.cur = self.start;
                self.seg_end = self.end;
                self.events.push(Event::SegmentEnd { restart: Some(self.start) });
            } else {
                self.events.push(Event::SegmentEnd { restart: None });
                self.playing = false;
                return false;
            }
        }
        match self.dec.decode_frame(&self.rom[self.cur as usize..], &mut self.pcm) {
            Ok(h) => {
                self.cur += h.frame_len() as u32;
                self.at = 0;
                self.frames += 1;
                true
            }
            Err(_) => {
                self.events.push(Event::BadFrame(self.cur));
                self.playing = false;
                false
            }
        }
    }
}
