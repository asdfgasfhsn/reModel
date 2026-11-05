//! The light gun interface of Model 2C gun games (The House of the Dead),
//! as the game's code drives it. It sits behind I/O chip registers 9-13
//! (`0x1C00012`-`0x1C0001A`, the upper byte lanes of the I/O words):
//!
//! - the game writes a parameter byte to register 10 (`0x1C00014`), then a
//!   command byte to register 9 (`0x1C00012`);
//! - it waits until register 13 (`0x1C0001A`) has bits 2 and 3 set;
//! - it reads a reply from register 11 (`0x1C00016`) and register 12
//!   (`0x1C00018`); only register 12 is used.
//!
//! Commands below `0x80` write the parameter to a register of the device
//! (its low seven bits), those from `0x80` read one back; `0x00` selects
//! one of the gun counters and `0x87` reads the selected one. At boot the
//! game streams a 5 KB configuration image through command `0x07` (the
//! device is a programmable logic part loaded by the game, presumably the
//! gun timing logic; unconfirmed).
//!
//! Counters `0`/`1` are player 1's X (low byte, then two high bits), `2`/`3`
//! its Y, `4`-`7` player 2's X and Y, and counter 8 holds one bit a player
//! (bit 0 player 1, bit 1 player 2), read active low; which is which is in
//! the game's manifest ([`romset::Gun`]).

/// The gun interface's state, plus what the guns currently report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GunBoard {
    /// Device registers written by commands `0x00`-`0x7F`.
    regs: [u8; 16],
    /// The last reply (register 12).
    reply: u8,
    /// Configuration bytes received (command `0x07`).
    pub config_bytes: u32,
    /// The counters command `0x87` reads, set from the players' guns.
    pub counters: [u8; 16],
}

impl GunBoard {
    /// Register 13: ready (bits 2 and 3) at once; nothing here takes time.
    pub const STATUS: u8 = 0x0c;

    /// The game wrote command `cmd` with parameter `param`.
    pub fn command(&mut self, cmd: u8, param: u8) {
        let reg = (cmd & 0x0f) as usize;
        if cmd & 0x80 == 0 {
            self.regs[reg] = param;
            if reg == 7 {
                self.config_bytes += 1;
            }
        } else if reg == 7 {
            self.reply = self.counters[self.regs[0] as usize & 15];
        } else {
            self.reply = self.regs[reg];
        }
    }

    /// Register 12.
    pub fn reply(&self) -> u8 {
        self.reply
    }

    /// Set a 10-bit position: its low byte in counter `n`, its two high
    /// bits in counter `n + 1`.
    pub fn set_counter_pair(&mut self, n: u8, v: u16) {
        let n = n as usize & 15;
        self.counters[n] = v as u8;
        self.counters[(n + 1) & 15] = (v >> 8) as u8 & 3;
    }

    /// Set or clear a gun's "sees nothing" bit in counter 8.
    pub fn set_offscreen(&mut self, bit: u8, offscreen: bool) {
        if offscreen {
            self.counters[8] |= 1 << bit;
        } else {
            self.counters[8] &= !(1 << bit);
        }
    }
}

state::fields!(GunBoard { regs, reply, config_bytes, counters });

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_selected_counter() {
        let mut g = GunBoard::default();
        g.set_counter_pair(2, 0x2c5);
        g.set_offscreen(1, true);
        // Select counter 2, read it; then counter 3 and 8.
        for (n, want) in [(2, 0xc5), (3, 0x02), (8, 0x02)] {
            g.command(0x00, n);
            g.command(0x87, 0);
            assert_eq!(g.reply(), want);
        }
        // Other registers read back what was written.
        g.command(0x01, 0x5a);
        g.command(0x81, 0);
        assert_eq!(g.reply(), 0x5a);
        g.command(0x07, 0x12);
        assert_eq!(g.config_bytes, 1);
    }
}
