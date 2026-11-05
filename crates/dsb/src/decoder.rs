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
//!
//! With a [`Soundtrack`] attached, a play whose ROM range is replaced streams
//! the replacement file instead (at the output rate, see
//! [`MpegChip::replacement_next`]), under the same loop, stop and gain
//! commands.

use crate::audio::{Step, Voice};
use crate::soundtrack::Soundtrack;
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
    /// The play just issued streams this soundtrack track instead of the ROM.
    Replaced {
        track: u8,
    },
    /// The play's replacement file failed to load; the ROM plays instead.
    ReplaceFailed {
        track: u8,
    },
    /// A replacement reached its loop end (and looped) or its end (and stopped).
    ReplacementEnd {
        track: u8,
        looped: bool,
    },
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
    pub soundtrack: Option<Soundtrack>,
    voice: Option<Voice>,
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
            soundtrack: None,
            voice: None,
        }
    }

    /// Chip reset (latch bit 5 low): stop and forget everything.
    pub fn reset(&mut self) {
        let rom = std::mem::take(&mut self.rom);
        let events = std::mem::take(&mut self.events);
        let soundtrack = self.soundtrack.take();
        *self = MpegChip { events, soundtrack, frames: self.frames, ..MpegChip::new(rom) };
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
                self.voice = None;
                if let Some(st) = &mut self.soundtrack {
                    match st.voice(self.start, self.end, crate::OUT_HZ as u32) {
                        Some(Ok(v)) => {
                            self.events.push(Event::Replaced { track: v.track });
                            self.voice = Some(v);
                        }
                        Some(Err(_)) => {
                            let track =
                                st.replacements.iter().find(|r| r.start == self.start).map_or(0, |r| r.spec.track);
                            self.events.push(Event::ReplaceFailed { track });
                        }
                        None => {}
                    }
                }
            }
            0x80..=0x87 => {
                if self.playing {
                    self.events.push(Event::Stop);
                }
                self.playing = false;
                self.looped = false;
                self.voice = None;
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
        self.matrix(l, r)
    }

    /// The gain matrix of `docs/hw/sound.md` §3.4.
    fn matrix(&self, l: f32, r: f32) -> (f32, f32) {
        let g = |i: usize| self.gain[i] as f32 / 255.0;
        (g(0) * l + g(1) * r, g(4) * l + g(5) * r)
    }

    /// Whether a replacement track is playing.
    pub fn replacing(&self) -> Option<u8> {
        self.voice.as_ref().map(|v| v.track)
    }

    /// The next output-rate (44.1 kHz) sample of the replacement track, after
    /// its gain and the gain matrix; silence when none is playing.
    pub fn replacement_next(&mut self) -> (f32, f32) {
        let Some(v) = &mut self.voice else { return (0.0, 0.0) };
        let gain = v.gain;
        let s = match v.next(self.looped) {
            Step::Sample(s) => s,
            Step::Looped(s) => {
                self.events.push(Event::ReplacementEnd { track: v.track, looped: true });
                s
            }
            Step::Ended => {
                self.events.push(Event::ReplacementEnd { track: v.track, looped: false });
                self.voice = None;
                self.playing = false;
                return (0.0, 0.0);
            }
        };
        self.matrix(s[0] * gain, s[1] * gain)
    }

    /// Decode the next frame; false when not playing.
    fn refill(&mut self) -> bool {
        if !self.playing || self.voice.is_some() {
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

// Save states. A replacement soundtrack track that is playing (`voice`)
// isn't kept: it stops on load. The ROM and events stay as they are.
state::fields!(MpegChip { cmd, args, need, start, end, looped, playing, cur, seg_end, gain, dec, pcm, at, frames });

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soundtrack::Manifest;

    fn send(c: &mut MpegChip, bytes: &[u8]) {
        for &b in bytes {
            c.write(b);
        }
    }

    /// A chip whose range `0x100-0x200` is replaced by 100 frames of DC 0.5
    /// at 44.1 kHz (so no resampling is involved).
    fn chip(manifest: &str) -> (MpegChip, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("remodel-chip-{}-{}", std::process::id(), manifest.len()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("dc.wav"), crate::audio::encode_wav(44100, 1, 3, 32, &[0.5; 100])).unwrap();
        let st = Soundtrack::resolve(Manifest::parse(manifest).unwrap(), &dir, &[(0x100, 0x200)]).unwrap();
        let mut c = MpegChip::new(vec![0; 0x1000]);
        c.soundtrack = Some(st);
        (c, dir)
    }

    #[test]
    fn replacement_follows_gain_loop_and_stop() {
        let (mut c, dir) = chip("track 1 = dc.wav\n");
        // Mode 2 gains as the firmware sets them, loop armed, play.
        send(&mut c, &[0x14, 0, 1, 0, 0x24, 0, 2, 0, 0xa0, 0xff, 0xa1, 0, 0xa4, 0, 0xa5, 0x80, 0x64, 0x74]);
        assert_eq!(c.replacing(), Some(1));
        assert!(c.events.contains(&Event::Replaced { track: 1 }));
        assert_eq!(c.next(), (0.0, 0.0), "the ROM path is silent");
        let (l, r) = c.replacement_next();
        assert_eq!((l, r), (0.5, 0.5 * 128.0 / 255.0));
        // A fade step: the firmware rewrites the gains.
        send(&mut c, &[0xa0, 0x40, 0xa5, 0x20]);
        let (l, r) = c.replacement_next();
        assert_eq!((l, r), (0.5 * 64.0 / 255.0, 0.5 * 32.0 / 255.0));
        // Looping: well past the 100-frame file it is still playing.
        for _ in 0..248 {
            assert_ne!(c.replacement_next(), (0.0, 0.0));
        }
        let loops = c.events.iter().filter(|e| matches!(e, Event::ReplacementEnd { looped: true, .. })).count();
        assert_eq!(loops, 2);
        // Stop.
        send(&mut c, &[0x84]);
        assert_eq!((c.replacement_next(), c.playing(), c.replacing()), ((0.0, 0.0), false, None));
        // One-shot: plays the file once, then the decoder is idle.
        send(&mut c, &[0x74]);
        let n = std::iter::from_fn(|| (c.replacement_next() != (0.0, 0.0)).then_some(())).count();
        assert_eq!(n, 100);
        assert!(!c.playing());
        assert_eq!(c.events.last(), Some(&Event::ReplacementEnd { track: 1, looped: false }));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn manifest_loop_mode_and_gain_override_the_game() {
        let (mut c, dir) = chip("[track 1]\nfile = dc.wav\nloop = on\ngain = 0.5\n");
        send(&mut c, &[0x14, 0, 1, 0, 0x24, 0, 2, 0, 0xa0, 0xff, 0xa5, 0xff, 0x74]); // not armed
        for _ in 0..300 {
            assert_eq!(c.replacement_next(), (0.25, 0.25));
        }
        // Another range is not replaced: the ROM path takes it.
        send(&mut c, &[0x84, 0x14, 0, 3, 0, 0x24, 0, 4, 0, 0x74]);
        assert_eq!(c.replacing(), None);
        assert!(c.playing());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
