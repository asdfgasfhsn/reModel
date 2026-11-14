//! A protection device on the expansion bus at `0x1D80000`, as the games
//! that check for it use it (docs/hw/model2b.md, "Protection device";
//! docs/hw/model2a.md for the keyed interface). Everything here comes from
//! the game's own code: the registers it writes and reads, in what order,
//! and what it compares the answer with. Two interfaces are known
//! ([`romset::ProtectionInterface`]):
//!
//! `Tables` (`doa`): all 16-bit accesses, with the status register polled
//! for `0xFFF0` ("ready") after each step: a command (`0x4000`), a data
//! block into the device's RAM (`0x0000` on, at a halfword offset), a table
//! address (`0x7FF4`) and table words (`0x7FF6`, two tables: 24 halfwords
//! at address 0 and 128 at `0x80`), `0x7FF0` cleared, the block's offset
//! written to `0x7FF2` to start, then the answer read from `0x7FF8`, the
//! first read discarded.
//!
//! `Keyed` (`dynamcop`): the block into the device's RAM (`0x0000` on), its
//! halfword offset to `0x10008`, a key to `0x1000C`, which starts it, then
//! the answer read from `0x1000E`, the first read discarded, with bit 0 of
//! `0x10000` (busy) polled clear after each read.
//!
//! On both, each halfword of the answer carries the earlier of its two
//! bytes in its high half.
//!
//! The answer itself isn't computed: the device turns the block into it in
//! a way that isn't known, so each request is answered with the bytes the
//! manifest names ([`romset::Protection`]), which are the ones the game
//! compares the answer with.

use romset::ProtectionInterface;

/// Where the device sits in the i960's address space: from here to
/// [`Device::end`].
pub const START: u32 = 0x01d8_0000;

/// Status: the value the game waits for after every step.
const READY: u16 = 0xfff0;

pub struct Device {
    interface: ProtectionInterface,
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
        Device { interface: p.interface, replies, command: 0, reading: None, latch: 0 }
    }

    /// The last address of the device's registers.
    pub fn end(&self) -> u32 {
        match self.interface {
            ProtectionInterface::Tables => 0x01d8_7fff,
            ProtectionInterface::Keyed => 0x01d9_ffff,
        }
    }

    /// A read of `n` bytes at `off` from [`START`].
    pub fn read(&mut self, off: u32, n: usize) -> u32 {
        match (self.interface, off & !1) {
            (ProtectionInterface::Tables, 0x400c) => READY as u32,
            (ProtectionInterface::Tables, 0x7ff8) | (ProtectionInterface::Keyed, 0x1000e) if n <= 2 => {
                self.next() as u32
            }
            // Keyed's status (0x10000) reads 0: never busy.
            _ => 0,
        }
    }

    /// The output register: one halfword behind the answer.
    fn next(&mut self) -> u16 {
        let v = self.latch;
        self.latch = match &mut self.reading {
            Some((r, i)) => {
                let w = self.replies[*r].1.get(*i).copied().unwrap_or(0);
                *i += 1;
                w
            }
            None => 0,
        };
        v
    }

    /// A write of `v` (`n` bytes) at `off` from [`START`].
    pub fn write(&mut self, off: u32, v: u32, _n: usize) {
        match (self.interface, off & !1) {
            (ProtectionInterface::Tables, 0x4000) => self.command = v as u16,
            // Start: answer the request the command asked for, if it's one
            // the manifest knows (otherwise the answer reads as zeros).
            (ProtectionInterface::Tables, 0x7ff2) => self.start(),
            // The key both names the request and starts it.
            (ProtectionInterface::Keyed, 0x1000c) => {
                self.command = v as u16;
                self.start();
            }
            // The data block, the tables, offsets and resets: taken and not
            // kept, as nothing is computed from them.
            _ => {}
        }
    }

    fn start(&mut self) {
        self.reading = self.replies.iter().position(|r| r.0 == self.command).map(|r| (r, 0));
        self.latch = 0;
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
        let p = romset::Protection { interface: ProtectionInterface::Tables, replies: R };
        let mut d = Device::new(&p, &[0, 0, b'A', b'B', b'C', 0]);
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

    #[test]
    fn keyed_answers_the_key_one_halfword_behind() {
        static R: &[romset::ProtectionReply] = &[romset::ProtectionReply { command: 0x7700, rom: 2, len: 4 }];
        let p = romset::Protection { interface: ProtectionInterface::Keyed, replies: R };
        let mut d = Device::new(&p, &[0, 0, b'U', b'C', b'H', b'I']);
        assert_eq!(d.end(), 0x01d9_ffff);
        d.write(0x0000, 0xcb13, 2);
        d.write(0x10008, 0, 2);
        d.write(0x1000c, 0x7700, 2);
        assert_eq!(d.read(0x1000e, 2), 0);
        assert_eq!(d.read(0x10000, 2) & 1, 0);
        assert_eq!(d.read(0x1000e, 2), u32::from_be_bytes([0, 0, b'U', b'C']));
        assert_eq!(d.read(0x1000e, 2), u32::from_be_bytes([0, 0, b'H', b'I']));
        // Another key reads zeros.
        d.write(0x1000c, 0x1326, 2);
        assert_eq!(d.read(0x1000e, 2) | d.read(0x1000e, 2), 0);
    }
}
