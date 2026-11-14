//! Model 2A's TGP (Fujitsu MB86234) as the i960 sees it: the program
//! upload, the FIFO ports, and the chip itself, the uploaded program run
//! by the `mb86234` interpreter (`docs/hw/model2a.md`, `mb86234.md`). Its
//! external bus reaches the geometrizer's buffer RAM, where the program
//! writes the matrices of the display list.
//!
//! [`Tgp2a::use_hle`] swaps the chip for `tgp2a_hle` (what milestone 2
//! read each function of `srallyc`'s program to do), kept for comparison;
//! for a program neither knows, only the load check is answered. Every
//! call is logged either way, so the function interface can be read off a
//! run.

use std::collections::{BTreeMap, VecDeque};

use crate::tgp2a_hle::{self, Hle};

/// Words the chip may run for one host access before it is taken to be
/// stuck (no function of `srallyc`'s program takes a thousand).
const RUN_LIMIT: u64 = 1_000_000;

/// The chip's external bus: the buffer RAM (32K words, mirrored), and the
/// geometrizer's record of the command words in it, which a store ends.
pub struct Shared<'a> {
    pub buffer: &'a mut [u8],
    pub commands: &'a mut BTreeMap<u32, u32>,
    /// The data ROM from `0x800000` (`Role::TgpData`, 32-bit words).
    pub rom: &'a [u32],
}

impl mb86234::Bus for Shared<'_> {
    fn read(&mut self, addr: u32) -> u32 {
        if addr >= mb86234::ROM_BASE {
            return self.rom.get((addr - mb86234::ROM_BASE) as usize).copied().unwrap_or(0);
        }
        let o = (addr as usize & 0x7fff) * 4;
        u32::from_le_bytes(self.buffer[o..o + 4].try_into().unwrap())
    }
    fn write(&mut self, addr: u32, v: u32) {
        if addr >= mb86234::ROM_BASE {
            return;
        }
        let o = (addr as usize & 0x7fff) * 4;
        self.buffer[o..o + 4].copy_from_slice(&v.to_le_bytes());
        if !self.commands.is_empty() {
            self.commands.remove(&(o as u32));
        }
    }
}

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
    /// What the stand-in or the function model answers with (the chip's
    /// own FIFO is [`mb86234::Cpu::fifo_out`]).
    pub fifo_out: VecDeque<u32>,
    /// The chip, running the uploaded program (unless [`Tgp2a::use_hle`]).
    pub cpu: Option<Box<mb86234::Cpu>>,
    /// The model of the program's functions, if [`Tgp2a::use_hle`] and the
    /// program uploaded is one that has one (`tgp2a_hle`).
    pub hle: Option<Hle>,
    /// Configuration: answer with `tgp2a_hle` instead of running the chip.
    pub use_hle: bool,
    /// Configuration: the function unit's lookup ROM (the board's
    /// `CoproData`, 32-bit words).
    pub tables: Vec<u32>,

    // Diagnostics, not machine state.
    pub trace: Vec<Event>,
    pub functions: BTreeMap<u32, Function>,
    open: Open,
    pub empty_reads: u64,
    /// The frame and instruction address of the first empty read.
    pub first_empty: Option<(u64, u32)>,
    /// Host accesses after which the chip was still running at
    /// [`RUN_LIMIT`] words.
    pub runaways: u64,
    /// The function open and the chip's pc at the first of them.
    pub first_runaway: Option<(Option<u32>, u32)>,

    /// The chip on the JIT (docs/jit.md, "Copro JITs"), for the program
    /// it holds, when [`Tgp2a::set_jit`] turned it on.
    #[cfg(not(target_arch = "wasm32"))]
    jit: Jit,
}

/// The MB86234 JIT's part of the board: whether it is on, its translation
/// of the program the chip holds, and (for `jit-copro-lockstep`) the
/// interpreter beside it.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct Jit {
    on: bool,
    check: bool,
    /// Translating on the workers (`jit::Config::background`).
    background: bool,
    code: Option<Box<jit::mb86234::Mb86234Jit>>,
    shadow: Option<Box<mb86234::Cpu>>,
    /// Runs compared (lockstep), and the stats of programs dropped.
    checked: u64,
    retired: jit::CoproStats,
}

/// A bus that keeps an interpreter's writes to itself and lists them, over
/// the real one, for the lockstep: it runs first, on the memory as the JIT
/// will find it.
#[cfg(not(target_arch = "wasm32"))]
struct Overlay<'a, 'b> {
    inner: &'a mut Shared<'b>,
    writes: Vec<(u32, u32)>,
}

#[cfg(not(target_arch = "wasm32"))]
impl mb86234::Bus for Overlay<'_, '_> {
    fn read(&mut self, addr: u32) -> u32 {
        // The last write to the same word (a ROM address is never kept).
        let word = |a: u32| if a >= mb86234::ROM_BASE { a } else { a & 0x7fff };
        match self.writes.iter().rev().find(|&&(a, _)| addr < mb86234::ROM_BASE && word(a) == word(addr)) {
            Some(&(_, v)) => v,
            None => mb86234::Bus::read(self.inner, addr),
        }
    }
    fn write(&mut self, addr: u32, v: u32) {
        if addr < mb86234::ROM_BASE {
            self.writes.push((addr, v));
        }
    }
}

/// The real bus, its writes listed.
#[cfg(not(target_arch = "wasm32"))]
struct Listed<'a, 'b> {
    inner: &'a mut Shared<'b>,
    writes: Vec<(u32, u32)>,
}

#[cfg(not(target_arch = "wasm32"))]
impl mb86234::Bus for Listed<'_, '_> {
    fn read(&mut self, addr: u32) -> u32 {
        mb86234::Bus::read(self.inner, addr)
    }
    fn write(&mut self, addr: u32, v: u32) {
        if addr < mb86234::ROM_BASE {
            self.writes.push((addr, v));
        }
        mb86234::Bus::write(self.inner, addr, v)
    }
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
    /// The interface of a board whose function unit has `tables`.
    pub fn new(tables: Vec<u32>) -> Self {
        Tgp2a { tables, ..Tgp2a::default() }
    }

    /// The control register (`0x980000`) was written. Bit 31 rising
    /// starts an upload, falling ends it and "boots" the (absent) chip.
    pub fn control(&mut self, old: u32, new: u32) {
        let (was, is) = (old & 0x8000_0000 != 0, new & 0x8000_0000 != 0);
        if is && !was {
            self.uploading = true;
            self.booted = false;
            self.hle = None;
            self.cpu = None;
            self.program.clear();
        } else if was && !is {
            self.uploading = false;
            self.booted = true;
            if self.use_hle {
                self.hle = tgp2a_hle::recognise(&self.program);
            } else {
                self.cpu = Some(Box::new(mb86234::Cpu::new(self.program.clone(), self.rom())));
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.jit_follow();
    }

    /// Run the chip on the JIT (docs/jit.md), or not; with `check`, beside
    /// the interpreter, comparing them after every run (panics at the first
    /// difference: `make jit-copro-lockstep`). Fails where Cranelift has
    /// no back end for this machine.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_jit(&mut self, c: &jit::Config) -> Result<(), String> {
        if c.copro {
            // Fail now rather than at the upload.
            jit::mb86234::Mb86234Jit::new(&[])?;
        }
        self.jit.on = c.copro;
        self.jit.check = c.copro_check;
        self.jit.background = c.background;
        self.jit_follow();
        Ok(())
    }

    /// The JIT's counters and, in lockstep, the runs compared.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn jit_stats(&self) -> Option<(jit::CoproStats, u64)> {
        let mut s = self.jit.retired;
        if let Some(c) = &self.jit.code {
            s.add(&c.stats());
        }
        self.jit.on.then_some((s, self.jit.checked))
    }

    /// Bring the JIT's translation in line with the program the chip
    /// holds: drop it for another (an upload, a loaded state), make one
    /// for a new one.
    #[cfg(not(target_arch = "wasm32"))]
    fn jit_follow(&mut self) {
        let j = &mut self.jit;
        let program = self.cpu.as_ref().filter(|_| j.on).map(|c| &c.program);
        if j.code.as_ref().is_some_and(|c| program.is_none_or(|p| !c.is_for(p))) {
            if let Some(c) = j.code.take() {
                j.retired.add(&c.stats());
            }
        }
        if let (None, Some(p)) = (&j.code, program) {
            j.code = jit::mb86234::Mb86234Jit::new(p).ok().map(|mut c| {
                c.set_background(j.background);
                Box::new(c)
            });
        }
        // The lockstep's interpreter starts again from the chip as it is.
        j.shadow = None;
    }

    /// The function unit's ROM: the first 64K words of `tables`.
    fn rom(&self) -> Vec<u32> {
        self.tables.get(..0x1_0000).unwrap_or(&self.tables).to_vec()
    }

    /// Run the chip until it waits for input.
    fn run(&mut self, bus: &mut Shared) {
        if let Some(c) = &mut self.cpu {
            #[cfg(not(target_arch = "wasm32"))]
            let stop = match &mut self.jit.code {
                Some(j) if self.jit.check => {
                    let shadow = self.jit.shadow.get_or_insert_with(|| c.clone());
                    self.jit.checked += 1;
                    check_run(c, shadow, j, bus)
                }
                Some(j) => j.run(c, bus, RUN_LIMIT),
                None => c.run(bus, RUN_LIMIT),
            };
            #[cfg(target_arch = "wasm32")]
            let stop = c.run(bus, RUN_LIMIT);
            if stop == mb86234::Stop::Limit {
                self.runaways += 1;
                self.first_runaway.get_or_insert((self.open.port, c.pc));
            }
        }
    }

    /// Nothing to read (the host's status register).
    pub fn output_empty(&self) -> bool {
        match &self.cpu {
            Some(c) => c.fifo_out.is_empty(),
            None => self.fifo_out.is_empty(),
        }
    }

    /// A store to the data port during an upload is a program word; else
    /// an argument.
    pub fn data_write(&mut self, offset: u32, v: u32, frame: u64, ip: u32, bus: &mut Shared) {
        if self.uploading {
            self.program.push(v);
            return;
        }
        if let Some(c) = &mut self.cpu {
            c.fifo_in.push_back(v);
            self.run(bus);
            match header(v) {
                Some(f) => {
                    self.begin(f, v, frame);
                    self.log((frame, ip, Kind::Header, offset, v));
                }
                None => {
                    self.open.args += 1;
                    self.log((frame, ip, Kind::Data, offset, v));
                }
            }
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
    pub fn command_write(&mut self, offset: u32, v: u32, frame: u64, ip: u32, bus: &mut Shared) {
        let p = port(offset);
        self.begin(p, v, frame);
        self.log((frame, ip, Kind::Command, offset, v));
        // The chip reads a command port's word with the port number in
        // bits 23-30 (its load check looks for 8 there), as 2C's hardware
        // tags them.
        if let Some(c) = &mut self.cpu {
            c.fifo_in.push_back((p << 23) | (v & 0x7f_ffff));
            self.run(bus);
            return;
        }
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
    pub fn data_read(&mut self, frame: u64, ip: u32, bus: &mut Shared) -> u32 {
        if self.cpu.as_ref().is_some_and(|c| c.fifo_out.is_empty()) {
            self.run(bus);
        }
        let fifo = match &mut self.cpu {
            Some(c) => &mut c.fifo_out,
            None => &mut self.fifo_out,
        };
        match fifo.pop_front() {
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
        self.cpu.save(w);
    }
    fn load(&mut self, r: &mut state::Reader) -> Result<(), state::Error> {
        self.program.load(r)?;
        self.uploading.load(r)?;
        self.booted.load(r)?;
        self.fifo_out.load(r)?;
        self.hle.load(r)?;
        // The program and ROM aren't in the state: they come from the
        // upload and the board.
        let mut saved: Option<Box<mb86234::Cpu>> = None;
        saved.load(r)?;
        let rom = self.rom();
        self.cpu = saved.map(|mut c| {
            c.program = self.program.clone();
            c.funcs.rom = rom;
            c
        });
        #[cfg(not(target_arch = "wasm32"))]
        self.jit_follow();
        Ok(())
    }
}

/// One run of the chip on the JIT with the interpreter beside it
/// (`shadow`, kept in step: it gets the same input), which runs first on
/// the memory the JIT will find; then everything is compared.
#[cfg(not(target_arch = "wasm32"))]
fn check_run(
    c: &mut mb86234::Cpu,
    shadow: &mut mb86234::Cpu,
    j: &mut jit::mb86234::Mb86234Jit,
    bus: &mut Shared,
) -> mb86234::Stop {
    shadow.fifo_in.clone_from(&c.fifo_in);
    shadow.fifo_out.clone_from(&c.fifo_out);
    let pc = c.pc;
    let mut over = Overlay { inner: bus, writes: Vec::new() };
    let want = shadow.run(&mut over, RUN_LIMIT);
    let want_writes = over.writes;
    let mut listed = Listed { inner: bus, writes: Vec::new() };
    let got = j.run(c, &mut listed, RUN_LIMIT);
    let what = jit::mb86234::differs(shadow, c)
        .or((want != got).then_some("where it stopped"))
        .or((want_writes != listed.writes).then_some("external writes"));
    if let Some(what) = what {
        panic!(
            "copro JIT lockstep: the MB86234 on the JIT differs from the interpreter in {what}, \
             in a run from pc {pc:#x} (interpreter stopped at {:#x}, JIT at {:#x})",
            shadow.pc, c.pc
        );
    }
    got
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

    fn with_bus<R>(f: impl FnOnce(&mut Shared) -> R) -> R {
        let (mut ram, mut commands) = (vec![0u8; 0x2_0000], BTreeMap::new());
        f(&mut Shared { buffer: &mut ram, commands: &mut commands, rom: &[] })
    }

    #[test]
    fn calls_are_counted_with_their_arguments_and_reads() {
        // The stand-in (an unknown program, no chip), which only logs.
        let mut t = Tgp2a { use_hle: true, ..Tgp2a::default() };
        with_bus(|bus| {
            t.control(0, 0x8000_0000);
            t.data_write(0, 0x1234_5678, 0, 0, bus);
            t.control(0x8000_0000, 0);
            assert_eq!(t.program, [0x1234_5678]);
            for frame in 0..3 {
                t.data_write(0, 0x1500_2a2a, frame, 0, bus);
                t.data_write(0, 0x3f80_0000, frame, 0, bus);
                t.data_write(0, 0x0100_0202, frame, 0, bus);
                assert_eq!(t.data_read(frame, 0, bus), 0);
            }
        });
        t.finish();
        let (f42, f2) = (&t.functions[&0x2a], &t.functions[&2]);
        assert_eq!((f42.calls, f42.args[&1], f42.replies[&0]), (3, 3, 3));
        assert_eq!((f2.calls, f2.args[&0], f2.replies[&1], f2.empty_reads), (3, 3, 3, 3));
        assert_eq!((f42.first_frame, f42.last_frame), (0, 2));
        assert_eq!(t.first_empty, Some((0, 0)));
    }

    #[test]
    fn the_chip_runs_the_upload() {
        // acc = 8; then for each input word: out = acc, and acc += the word
        // (integers); and a word stored to buffer RAM word 2 on the way.
        let program = [
            0x3b00_0008, // acc = 8
            0x1c1f_2621, // r13 = in
            0x1c1f_2013, // r10 = r13
            0x1c1f_4419, // out = acc
            0x6340_0000, // bus: shared RAM
            0x4300_0002, // ar3 = 2
            0x1f5c_a7a0, // acc += r10 | ext[ar3] = r13
            0xbf60_0001, // jmp 1
        ];
        let mut t = Tgp2a::default();
        let (mut ram, mut commands) = (vec![0u8; 0x2_0000], BTreeMap::new());
        let bus = &mut Shared { buffer: &mut ram, commands: &mut commands, rom: &[] };
        t.control(0, 0x8000_0000);
        for w in program {
            t.data_write(0, w, 0, 0, bus);
        }
        t.control(0x8000_0000, 0);
        t.command_write(0x80, 0, 0, 0, bus);
        assert_eq!(t.data_read(0, 0, bus), 8);
        t.data_write(0, 5, 0, 0, bus);
        assert_eq!(t.data_read(0, 0, bus), 8 + (8 << 23));
        assert!(t.output_empty());
        assert_eq!(ram[8..12], 5u32.to_le_bytes());
        let c = t.cpu.as_ref().unwrap();
        assert!(c.unknown.is_empty() && t.runaways == 0);
    }
}
