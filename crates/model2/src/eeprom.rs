//! 93C46 serial EEPROM in x16 organisation (64 words), bit-banged by the
//! CPU through I/O port pins. Protocol per the Microchip 93xx46 datasheet
//! (summarised in docs/hw/peripherals.md): a start bit, a 2-bit opcode and
//! 6 address bits, sampled on rising CLK edges while CS is high. Writes
//! complete instantly, which the datasheet allows (tWC is a maximum).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Waiting for a start bit (CS high, DI high on a rising edge).
    Idle,
    /// Shifting in opcode + address (8 bits after the start bit).
    Command { bits: u32, n: u32 },
    /// Shifting in 16 data bits for WRITE or WRAL.
    Data { op: Op, addr: u8, bits: u32, n: u32 },
    /// Shifting out read data; `n` bits of `word` remain.
    Read { addr: u8, word: u32, n: u32 },
    /// Instruction complete; DO shows ready until CS drops.
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Write,
    WriteAll,
}

pub struct Eeprom {
    pub data: [u16; 64],
    write_enabled: bool,
    state: State,
    cs: bool,
    clk: bool,
    dout: bool,
    /// Set whenever stored contents change, so callers can persist them.
    pub dirty: bool,
}

impl Default for Eeprom {
    fn default() -> Self {
        Self::new()
    }
}

impl Eeprom {
    /// A blank part: all ones, powered up write-disabled.
    pub fn new() -> Self {
        Eeprom {
            data: [0xffff; 64],
            write_enabled: false,
            state: State::Idle,
            cs: false,
            clk: false,
            dout: true,
            dirty: false,
        }
    }

    pub fn load(&mut self, bytes: &[u8]) {
        for (w, b) in self.data.iter_mut().zip(bytes.as_chunks::<2>().0) {
            *w = u16::from_le_bytes(*b);
        }
    }

    pub fn save(&self) -> Vec<u8> {
        self.data.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    /// Serial data out.
    pub fn dout(&self) -> bool {
        self.dout
    }

    /// Drive the input pins.
    pub fn set_lines(&mut self, cs: bool, clk: bool, di: bool) {
        if !cs {
            if self.cs {
                self.state = State::Idle;
            }
            // Ready/busy polling: with writes instant, the part reads ready.
            self.dout = true;
        } else if clk && !self.clk {
            self.rising_edge(di);
        }
        self.cs = cs;
        self.clk = clk;
    }

    fn rising_edge(&mut self, di: bool) {
        let bit = di as u32;
        self.state = match self.state {
            State::Idle if di => State::Command { bits: 0, n: 0 },
            State::Idle => State::Idle,
            State::Command { bits, n } => {
                let bits = (bits << 1) | bit;
                if n + 1 < 8 {
                    State::Command { bits, n: n + 1 }
                } else {
                    self.command(bits)
                }
            }
            State::Data { op, addr, bits, n } => {
                let bits = (bits << 1) | bit;
                if n + 1 < 16 {
                    State::Data { op, addr, bits, n: n + 1 }
                } else {
                    let v = bits as u16;
                    if self.write_enabled {
                        match op {
                            Op::Write => self.data[addr as usize] = v,
                            Op::WriteAll => self.data = [v; 64],
                        }
                        self.dirty = true;
                    }
                    State::Done
                }
            }
            State::Read { addr, word, n } => {
                if n == 0 {
                    // Sequential read: continue with the next word.
                    let addr = (addr + 1) & 63;
                    let word = self.data[addr as usize] as u32;
                    self.dout = word & 0x8000 != 0;
                    State::Read { addr, word, n: 15 }
                } else {
                    self.dout = (word >> (n - 1)) & 1 != 0;
                    State::Read { addr, word, n: n - 1 }
                }
            }
            State::Done => State::Done,
        };
    }

    /// Opcode (2 bits) and address (6 bits) are in; returns the next state.
    fn command(&mut self, bits: u32) -> State {
        let addr = (bits & 63) as u8;
        match bits >> 6 {
            0b10 => {
                // Dummy zero, then D15..D0 on the following edges.
                self.dout = false;
                State::Read { addr, word: self.data[addr as usize] as u32, n: 16 }
            }
            0b01 => State::Data { op: Op::Write, addr, bits: 0, n: 0 },
            0b11 => {
                if self.write_enabled {
                    self.data[addr as usize] = 0xffff;
                    self.dirty = true;
                }
                State::Done
            }
            _ => match addr >> 4 {
                0b11 => {
                    self.write_enabled = true;
                    State::Done
                }
                0b00 => {
                    self.write_enabled = false;
                    State::Done
                }
                0b10 => {
                    if self.write_enabled {
                        self.data = [0xffff; 64];
                        self.dirty = true;
                    }
                    State::Done
                }
                _ => State::Data { op: Op::WriteAll, addr: 0, bits: 0, n: 0 },
            },
        }
    }
}

// Save states.
state::fields!(Eeprom { data, write_enabled, state, cs, clk, dout, dirty });

impl state::State for State {
    fn save(&self, w: &mut state::Writer) {
        match *self {
            State::Idle => 0u8.save(w),
            State::Command { bits, n } => (1u8, bits, n).save(w),
            State::Data { op, addr, bits, n } => {
                (2u8, matches!(op, Op::WriteAll), addr).save(w);
                (bits, n).save(w);
            }
            State::Read { addr, word, n } => (3u8, addr, (word, n)).save(w),
            State::Done => 4u8.save(w),
        }
    }
    fn load(&mut self, r: &mut state::Reader) -> Result<(), state::Error> {
        let mut tag = 0u8;
        tag.load(r)?;
        *self = match tag {
            0 => State::Idle,
            1 => {
                let mut v = (0u32, 0u32);
                v.load(r)?;
                State::Command { bits: v.0, n: v.1 }
            }
            2 => {
                let (mut all, mut addr, mut v) = (false, 0u8, (0u32, 0u32));
                all.load(r)?;
                addr.load(r)?;
                v.load(r)?;
                State::Data { op: if all { Op::WriteAll } else { Op::Write }, addr, bits: v.0, n: v.1 }
            }
            3 => {
                let (mut addr, mut v) = (0u8, (0u32, 0u32));
                addr.load(r)?;
                v.load(r)?;
                State::Read { addr, word: v.0, n: v.1 }
            }
            4 => State::Done,
            t => return Err(state::Error(format!("bad EEPROM state {t}"))),
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Clock `n` bits of `value` (MSB first) into the part.
    fn send(e: &mut Eeprom, value: u32, n: u32) {
        for i in (0..n).rev() {
            let di = (value >> i) & 1 != 0;
            e.set_lines(true, false, di);
            e.set_lines(true, true, di);
        }
    }

    fn read_word(e: &mut Eeprom, addr: u32) -> u16 {
        e.set_lines(false, false, false);
        send(e, 0b1_10 << 6 | addr, 9);
        assert!(!e.dout(), "dummy zero");
        let mut v = 0;
        for _ in 0..16 {
            e.set_lines(true, false, false);
            e.set_lines(true, true, false);
            v = (v << 1) | e.dout() as u16;
        }
        e.set_lines(false, false, false);
        v
    }

    #[test]
    fn write_needs_enable_then_reads_back() {
        let mut e = Eeprom::new();
        e.set_lines(false, false, false);
        send(&mut e, (0b1_01 << 6 | 5) << 16 | 0x1234, 25); // WRITE ignored: disabled
        e.set_lines(false, false, false);
        assert_eq!(read_word(&mut e, 5), 0xffff);

        send(&mut e, 0b1_00_11 << 4, 9); // EWEN
        e.set_lines(false, false, false);
        send(&mut e, (0b1_01 << 6 | 5) << 16 | 0x1234, 25);
        e.set_lines(false, false, false);
        assert_eq!(read_word(&mut e, 5), 0x1234);
        assert!(e.dirty);
    }

    #[test]
    fn leading_zeros_before_start_bit_are_ignored() {
        let mut e = Eeprom::new();
        e.data[9] = 0xbeef;
        e.set_lines(false, false, false);
        send(&mut e, 0, 3);
        send(&mut e, 0b1_10 << 6 | 9, 9);
        let mut v = 0u16;
        for _ in 0..16 {
            e.set_lines(true, false, false);
            e.set_lines(true, true, false);
            v = (v << 1) | e.dout() as u16;
        }
        assert_eq!(v, 0xbeef);
    }
}
