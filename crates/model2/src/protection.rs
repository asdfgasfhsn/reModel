//! A protection device on the expansion bus at `0x1D80000-0x1D87FFF`, as
//! the games that check for it use it (docs/hw/model2b.md, "Protection
//! device"). Everything here comes from the game's own code: the registers
//! it writes and reads, in what order, and what it compares the answer
//! with.
//!
//! A request, all 16-bit accesses, with the status register polled for
//! `0xFFF0` ("ready") after each step: a command (`0x4000`), a data block
//! into the device's RAM (`0x0000` on, at a halfword offset), a table
//! address (`0x7FF4`) and table words (`0x7FF6`, two tables: 24 halfwords
//! at address 0 and 128 at `0x80`), `0x7FF0` cleared, the block's offset
//! written to `0x7FF2` to start, then the answer read from `0x7FF8`, the
//! first read discarded. Each halfword of the answer carries the earlier
//! of its two bytes in its high half.
//!
//! The answer itself isn't computed: the device turns the block into it in
//! a way that isn't known, so each request is answered with the bytes the
//! manifest names ([`romset::Protection`]), which are the ones the game
//! compares the answer with.

/// Where the device sits in the i960's address space.
pub const START: u32 = 0x01d8_0000;
pub const END: u32 = 0x01d8_7fff;

/// Status: the value the game waits for after every step.
const READY: u16 = 0xfff0;

#[derive(Default)]
pub struct Device {
    /// (command, answer as the halfwords the device returns).
    replies: Vec<(u16, Vec<u16>)>,
    command: u16,
    /// The answer being read: index into `replies`, and the next halfword.
    reading: Option<(usize, usize)>,
    /// What the next read of the output register returns: the output is
    /// one halfword behind, so the first read after a start gives this
    /// (zero) and the game discards it.
    latch: u16,
}

impl Device {
    /// The device for `p`, with its answers taken from the program ROM
    /// (`program`, the i960's view from address 0).
    pub fn new(p: &romset::Protection, program: &[u8]) -> Self {
        let byte = |a: u32| program.get(a as usize).copied().unwrap_or(0) as u16;
        let replies = p
            .replies
            .iter()
            .map(|r| {
                (
                    r.command,
                    (0..r.len.div_ceil(2)).map(|i| byte(r.rom + 2 * i) << 8 | byte(r.rom + 2 * i + 1)).collect(),
                )
            })
            .collect();
        Device { replies, ..Device::default() }
    }

    /// A read of `n` bytes at `off` from [`START`].
    pub fn read(&mut self, off: u32, n: usize) -> u32 {
        match off & !1 {
            0x400c => READY as u32,
            0x7ff8 if n <= 2 => {
                let v = self.latch;
                self.latch = match &mut self.reading {
                    Some((r, i)) => {
                        let w = self.replies[*r].1.get(*i).copied().unwrap_or(0);
                        *i += 1;
                        w
                    }
                    None => 0,
                };
                v as u32
            }
            _ => 0,
        }
    }

    /// A write of `v` (`n` bytes) at `off` from [`START`].
    pub fn write(&mut self, off: u32, v: u32, _n: usize) {
        match off & !1 {
            0x4000 => self.command = v as u16,
            // Start: answer the request the command asked for, if it's one
            // the manifest knows (otherwise the answer reads as zeros).
            0x7ff2 => {
                self.reading = self.replies.iter().position(|r| r.0 == self.command).map(|r| (r, 0));
                self.latch = 0;
            }
            // The data block, the table address and words, and `0x7FF0`:
            // taken and not kept, as nothing is computed from them.
            _ => {}
        }
    }
}

impl state::State for Device {
    fn save(&self, w: &mut state::Writer) {
        self.command.save(w);
        self.reading.map(|(r, i)| (r as u64, i as u64)).save(w);
        self.latch.save(w);
    }
    fn load(&mut self, r: &mut state::Reader) -> Result<(), state::Error> {
        self.command.load(r)?;
        let mut reading: Option<(u64, u64)> = None;
        reading.load(r)?;
        self.reading = reading.map(|(r, i)| (r as usize, i as usize)).filter(|&(r, _)| r < self.replies.len());
        self.latch.load(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_a_request_one_halfword_behind() {
        static R: &[romset::ProtectionReply] = &[romset::ProtectionReply { command: 6, rom: 2, len: 3 }];
        let mut d = Device::new(&romset::Protection { replies: R }, &[0, 0, b'A', b'B', b'C', 0]);
        assert_eq!(d.read(0x400c, 2), 0xfff0);
        d.write(0x4000, 6, 2);
        d.write(0x0000, 0x1234, 2);
        d.write(0x7ff2, 0, 2);
        assert_eq!(d.read(0x7ff8, 2), 0);
        assert_eq!(d.read(0x7ff8, 2), u32::from_be_bytes([0, 0, b'A', b'B']));
        assert_eq!(d.read(0x7ff8, 2), u32::from_be_bytes([0, 0, b'C', 0]));
        assert_eq!(d.read(0x7ff8, 2), 0);
        // An unknown command reads zeros.
        d.write(0x4000, 7, 2);
        d.write(0x7ff2, 0, 2);
        assert_eq!(d.read(0x7ff8, 2) | d.read(0x7ff8, 2), 0);
    }
}
