//! Model 2A's TGP (Fujitsu MB86234) as the i960 sees it, with a **stand-in
//! for the chip itself**: the program upload and the FIFO ports are
//! modelled, the program is *not run*. The stand-in answers the one
//! request the game's load check makes and records every other call the
//! game makes, so the TGP's function interface can be read off a run
//! (`docs/hw/model2a.md`). For a program it recognises, the functions are
//! answered by `tgp2a_hle` (what each was read to do, not the chip);
//! otherwise games that wait for the TGP's answers stall (an empty FIFO
//! reads as 0).

use std::collections::{BTreeMap, VecDeque};

use crate::tgp2a_hle::{self, Hle};

/// Raw events kept for a run's log (the rest only count).
const TRACE_LIMIT: usize = 200_000;

/// What the i960 did at a TGP port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A write to the command ports (`0x880000-0x883FFF`): `a` is the
    /// address offset, `v` the value written.
    Command,
    /// A write to the data port (`0x884000-0x887FFF`, `a` the offset) that has the form of a
    /// command header ([`header`]).
    Header,
    /// Any other write to the data port (`0x884000`): an argument.
    Data,
    /// A read of the data port; `v` is what it returned.
    Reply,
    /// A read of the data port with nothing to return.
    EmptyReply,
}

/// One logged access: frame, i960 instruction address, kind, address
/// offset, value.
pub type Event = (u64, u32, Kind, u32, u32);

/// What the game did with one command port, over a run.
#[derive(Clone, Debug, Default)]
pub struct Function {
    /// Commands written to the port.
    pub calls: u64,
    /// Data words written between a call and the next command: how many
    /// calls had that many.
    pub args: BTreeMap<u32, u64>,
    /// Data-port reads between a call and the next command (the words
    /// the game took as the answer; `empty_reads` counts those that
    /// found the FIFO empty).
    pub replies: BTreeMap<u32, u64>,
    pub empty_reads: u64,
    pub first_frame: u64,
    pub last_frame: u64,
    /// The values the command words carried, first few distinct.
    pub values: Vec<u32>,
}

/// The call being received: the command port and what followed it so far.
#[derive(Clone, Copy, Debug, Default)]
struct Open {
    port: Option<u32>,
    args: u32,
    replies: u32,
    empties: u32,
}

#[derive(Default)]
pub struct Tgp2a {
    /// The program as uploaded: 32-bit words, one per store (not the
    /// 64-bit pairs of 2C's MB86235).
    pub program: Vec<u32>,
    pub uploading: bool,
    /// The upload has finished (the control register's bit 31 fell).
    pub booted: bool,
    /// What the stand-in answers with.
    pub fifo_out: VecDeque<u32>,
    /// The model of the program's functions, if the program uploaded is
    /// one that has one (`tgp2a_hle`); else the stand-in answers only the
    /// load check.
    pub hle: Option<Hle>,

    // Diagnostics, not machine state.
    pub trace: Vec<Event>,
    pub functions: BTreeMap<u32, Function>,
    open: Open,
    pub empty_reads: u64,
    /// The frame and instruction address of the first empty read.
    pub first_empty: Option<(u64, u32)>,
}

/// Command port of a write at `offset` from `0x880000`: the address bits
/// the game varies, `(offset >> 4) & 0xff` as on the 2C ports.
pub fn port(offset: u32) -> u32 {
    (offset >> 4) & 0xff
}

/// The function number of a command header the game writes to the data
/// port, if `w` has that form: the number in bits 23-30 (as the hardware
/// tags a write to command port `0x880000 + 0x10 * number` on 2C) and the
/// low 16 bits both bytes equal to it. Every command word in a run
/// (`docs/hw/model2a.md`) is of this form and no argument is known to
/// be, but a float argument could look like one by chance: *a heuristic*,
/// good for reading a log.
pub fn header(w: u32) -> Option<u32> {
    let f = (w >> 23) & 0xff;
    (w >> 31 == 0 && f != 0 && w & 0xffff == f * 0x101 && w & 0x007f_0000 == 0).then_some(f)
}

impl Tgp2a {
    /// The control register (`0x980000`) was written. Bit 31 rising
    /// starts an upload, falling ends it and "boots" the (absent) chip.
    pub fn control(&mut self, old: u32, new: u32) {
        let (was, is) = (old & 0x8000_0000 != 0, new & 0x8000_0000 != 0);
        if is && !was {
            self.uploading = true;
            self.booted = false;
            self.hle = None;
            self.program.clear();
        } else if was && !is {
            self.uploading = false;
            self.booted = true;
            self.hle = tgp2a_hle::recognise(&self.program);
        }
    }

    /// A store to the data port during an upload is a program word; else
    /// an argument.
    pub fn data_write(&mut self, offset: u32, v: u32, frame: u64, ip: u32) {
        if self.uploading {
            self.program.push(v);
            return;
        }
        // A model that knows the program knows how many words each
        // function takes, so it needs no guess at what is a header.
        let model_header = match &self.hle {
            Some(h) => h.expects_header().then(|| header(v)).flatten(),
            None => header(v),
        };
        match model_header {
            Some(f) => {
                self.begin(f, v, frame);
                if let Some(h) = &mut self.hle {
                    h.begin(f, &mut self.fifo_out);
                }
                self.log((frame, ip, Kind::Header, offset, v));
            }
            None => {
                self.open.args += 1;
                if let Some(h) = &mut self.hle {
                    h.data(v, &mut self.fifo_out);
                }
                self.log((frame, ip, Kind::Data, offset, v));
            }
        }
    }

    /// A call to function `p` starts, its command word `v`.
    fn begin(&mut self, p: u32, v: u32, frame: u64) {
        self.close();
        self.open = Open { port: Some(p), ..Open::default() };
        let f = self.functions.entry(p).or_default();
        if f.calls == 0 {
            f.first_frame = frame;
        }
        f.calls += 1;
        f.last_frame = frame;
        if f.values.len() < 8 && !f.values.contains(&v) {
            f.values.push(v);
        }
    }

    /// A store to a command port.
    pub fn command_write(&mut self, offset: u32, v: u32, frame: u64, ip: u32) {
        let p = port(offset);
        self.begin(p, v, frame);
        self.log((frame, ip, Kind::Command, offset, v));
        // The only request the stand-in answers: port 8 (a write to
        // 0x880080 after the upload) is answered with 8, which the game's
        // load check wants to read back.
        if p == 8 && self.booted {
            self.fifo_out.push_back(8);
        } else if let Some(h) = &mut self.hle {
            h.begin(p, &mut self.fifo_out);
        }
    }

    /// A read of the data port.
    pub fn data_read(&mut self, frame: u64, ip: u32) -> u32 {
        match self.fifo_out.pop_front() {
            Some(v) => {
                self.open.replies += 1;
                self.log((frame, ip, Kind::Reply, 0, v));
                v
            }
            None => {
                self.empty_reads += 1;
                self.open.replies += 1;
                self.open.empties += 1;
                self.first_empty.get_or_insert((frame, ip));
                self.log((frame, ip, Kind::EmptyReply, 0, 0));
                0
            }
        }
    }

    fn log(&mut self, e: Event) {
        if self.trace.len() < TRACE_LIMIT {
            self.trace.push(e);
        }
    }

    /// Settle the call being received into its port's statistics.
    fn close(&mut self) {
        if let Some(p) = self.open.port.take() {
            let o = self.open;
            let f = self.functions.entry(p).or_default();
            *f.args.entry(o.args).or_default() += 1;
            *f.replies.entry(o.replies).or_default() += 1;
            f.empty_reads += o.empties as u64;
        }
    }

    /// Settle the last call too (before reading the statistics).
    pub fn finish(&mut self) {
        self.close();
    }
}

impl state::State for Tgp2a {
    fn save(&self, w: &mut state::Writer) {
        self.program.save(w);
        self.uploading.save(w);
        self.booted.save(w);
        self.fifo_out.save(w);
        self.hle.save(w);
    }
    fn load(&mut self, r: &mut state::Reader) -> Result<(), state::Error> {
        self.program.load(r)?;
        self.uploading.load(r)?;
        self.booted.load(r)?;
        self.fifo_out.load(r)?;
        self.hle.load(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_headers() {
        // Function 3 with no argument, 2, 0x5c and 0x2a as the game writes them.
        assert_eq!(header(0x0180_0303), Some(3));
        assert_eq!(header(0x0100_0202), Some(2));
        assert_eq!(header(0x2e00_5c5c), Some(0x5c));
        assert_eq!(header(0x1500_2a2a), Some(0x2a));
        // Floats and zero are arguments.
        assert_eq!(header(0x4360_84f1), None);
        assert_eq!(header(0), None);
        assert_eq!(header(0xbf60_0085), None);
    }

    #[test]
    fn calls_are_counted_with_their_arguments_and_reads() {
        let mut t = Tgp2a::default();
        t.control(0, 0x8000_0000);
        t.data_write(0, 0x1234_5678, 0, 0);
        t.control(0x8000_0000, 0);
        assert_eq!(t.program, [0x1234_5678]);
        for frame in 0..3 {
            t.data_write(0, 0x1500_2a2a, frame, 0);
            t.data_write(0, 0x3f80_0000, frame, 0);
            t.data_write(0, 0x0100_0202, frame, 0);
            assert_eq!(t.data_read(frame, 0), 0);
        }
        t.finish();
        let (f42, f2) = (&t.functions[&0x2a], &t.functions[&2]);
        assert_eq!((f42.calls, f42.args[&1], f42.replies[&0]), (3, 3, 3));
        assert_eq!((f2.calls, f2.args[&0], f2.replies[&1], f2.empty_reads), (3, 3, 3, 3));
        assert_eq!((f42.first_frame, f42.last_frame), (0, 2));
        assert_eq!(t.first_empty, Some((0, 0)));
    }
}
