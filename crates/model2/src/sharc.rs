//! Model 2B's copro: an ADSP-21062 SHARC running a program the i960
//! uploads through its host interface (docs/hw/model2b.md).
//!
//! The i960 sees the same copro ports as on 2C: commands go into an input
//! FIFO (tagged through `0x880000+`, raw through `0x884000`), results come
//! back through `0x884000`, and `0x980004` bit 0 is set while there are
//! none. On the SHARC's side (from its program): it reads commands from
//! external `0x400000` once FLAG0, the input FIFO's empty flag, is low,
//! and writes results to external `0xC00000` once FLAG1, the output FIFO's
//! full flag, is low.
//!
//! The rest of its external bus: buffer RAM, the display list the i960
//! builds, in bank 2 (`fvipers` and `schamp` have the copro write parts of
//! the list there), and on boards with copro data ROMs (`indy500`,
//! `fvipers`, `schamp`) those ROMs in bank 3.

use adsp2106x::cpu::{Bus, Fault, Sharc, Step, Width};
use std::collections::{BTreeMap, VecDeque};

/// SHARC instructions per i960 instruction: a 40 MHz SHARC (general
/// knowledge; one instruction a cycle) against the i960's 25 MHz at
/// `CYCLES_PER_INSN` cycles an instruction, rounded.
pub const STEPS_PER_INSN: u64 = 5;
/// How far the copro may run while the i960 waits on an empty output FIFO.
const STALL_LIMIT: u64 = 4_000_000;
/// Results the output FIFO holds before FLAG1 reports it full.
/// *Unconfirmed*: the depth isn't known; the program only waits on it.
const OUT_DEPTH: usize = 64;

/// External bus addresses the copro program uses.
const IN_FIFO: u32 = 0x40_0000;
const OUT_FIFO: u32 = 0xc0_0000;
/// External memory banks 2 and 3 with the copro's SYSCON MSIZE 10 (banks
/// of 8M words from `0x400000`, [UM §5.4.1]): buffer RAM, then the data
/// ROMs (docs/hw/model2b.md, "The copro's external memory"), each
/// repeating through its bank.
const BUFFER_BANK: u32 = 0x140_0000;
const ROM_BANK: u32 = 0x1c0_0000;
const BANK_END: u32 = 0x240_0000;

/// Buffer RAM word `addr` of bank 2, as a byte offset into `buffer`.
#[inline]
fn buffer_offset(buffer: &[u8], addr: u32) -> usize {
    ((addr - BUFFER_BANK) as usize * 4) & (buffer.len() - 1)
}

/// The copro SHARC and its FIFOs.
pub struct Copro {
    pub cpu: Box<Sharc>,
    pub fifo_in: VecDeque<u32>,
    pub fifo_out: VecDeque<u32>,
    /// The host holds the SHARC's bus (`0x980000` bit 31): it is being
    /// programmed through its IOP registers and doesn't run.
    pub held: bool,
    /// A program has been uploaded and released.
    pub booted: bool,
    /// The host writes of the last upload, each `IOP register << 32 |
    /// data` (diagnostics: [`image`] turns them into the program).
    pub program: Vec<u64>,
    pub fault: Option<Fault>,
    /// The copro data ROMs (32-bit words, as on 2C) in bank 3 of its
    /// external bus, on boards that have them.
    rom: Option<std::sync::Arc<[u32]>>,
    /// The core is spinning on a flag input with these inputs: stepping
    /// it again can't change anything until they do.
    spin: Option<u8>,
    /// Recompiled code for the uploaded program, if the build has it
    /// (docs/recompiler.md); `interpret` keeps it on the interpreter.
    native: Option<crate::recomp::SharcCoproRun>,
    pub interpret: bool,
    /// Diagnostics: reads of an empty input FIFO, host interface errors,
    /// and accesses to anything else on the external bus.
    pub empty_reads: u64,
    pub host_errors: u64,
    pub other: BTreeMap<u32, u64>,
}

/// The copro SHARC's external bus: its FIFOs, buffer RAM and the data ROMs.
pub struct CoproBus<'a> {
    fifo_in: &'a mut VecDeque<u32>,
    fifo_out: &'a mut VecDeque<u32>,
    empty_reads: &'a mut u64,
    other: &'a mut BTreeMap<u32, u64>,
    buffer: &'a mut [u8],
    /// The geometrizer's command words in buffer RAM, which a store ends
    /// (`Geometrizer::commands`).
    commands: &'a mut BTreeMap<u32, u32>,
    rom: Option<&'a [u32]>,
    /// The core's `steps` to stop at.
    end: u64,
    /// The FLAG inputs the current instruction ran with.
    flags: u8,
    spin: &'a mut Option<u8>,
}

impl CoproBus<'_> {
    /// FLAG0: input FIFO empty; FLAG1: output FIFO full.
    fn flags(&self) -> u8 {
        self.fifo_in.is_empty() as u8 | ((self.fifo_out.len() >= OUT_DEPTH) as u8) << 1
    }

    fn set_flags(&mut self, cpu: &mut Sharc) {
        self.flags = self.flags();
        cpu.flag_in = (cpu.flag_in & !3) | self.flags;
    }
}

impl Bus for CoproBus<'_> {
    fn read(&mut self, addr: u32, _: Width) -> u64 {
        if addr == IN_FIFO {
            return match self.fifo_in.pop_front() {
                Some(v) => v as u64,
                None => {
                    *self.empty_reads += 1;
                    0
                }
            };
        }
        match (addr, self.rom) {
            (BUFFER_BANK..ROM_BANK, _) => {
                let o = buffer_offset(self.buffer, addr);
                return u32::from_le_bytes(self.buffer[o..o + 4].try_into().unwrap()) as u64;
            }
            (ROM_BANK..BANK_END, Some(rom)) => return rom[(addr - ROM_BANK) as usize % rom.len()] as u64,
            _ => {}
        }
        *self.other.entry(addr).or_default() += 1;
        0
    }
    fn write(&mut self, addr: u32, _: Width, v: u64) {
        match addr {
            OUT_FIFO => self.fifo_out.push_back(v as u32),
            BUFFER_BANK..ROM_BANK => {
                let o = buffer_offset(self.buffer, addr);
                self.buffer[o..o + 4].copy_from_slice(&(v as u32).to_le_bytes());
                if !self.commands.is_empty() {
                    self.commands.remove(&(o as u32));
                }
            }
            _ => *self.other.entry(addr).or_default() += 1,
        }
    }
    /// Stop when the program only waits on a FLAG input (it can't do
    /// anything else until the FIFOs change), else set the inputs for the
    /// next instruction.
    #[inline(always)]
    fn after_step(&mut self, cpu: &mut Sharc) -> bool {
        if cpu.spinning() {
            *self.spin = Some(self.flags);
            return false;
        }
        if cpu.steps >= self.end {
            return false;
        }
        self.set_flags(cpu);
        true
    }
    /// Up to the step limit: until then `after_step` only sets the FLAG
    /// inputs, which change only with the FIFOs (`after_access`), and the
    /// core can't be spinning inside a block.
    #[inline(always)]
    fn block_budget(&mut self, cpu: &Sharc) -> u64 {
        self.end.saturating_sub(cpu.steps)
    }
    #[inline(always)]
    fn after_access(&mut self, cpu: &mut Sharc) {
        self.set_flags(cpu);
    }
}

impl Default for Copro {
    fn default() -> Self {
        Self::new()
    }
}

impl Copro {
    pub fn new() -> Self {
        Copro {
            cpu: Box::new(Sharc::new()),
            fifo_in: VecDeque::new(),
            fifo_out: VecDeque::new(),
            held: false,
            booted: false,
            program: Vec::new(),
            fault: None,
            rom: None,
            spin: None,
            native: None,
            interpret: false,
            empty_reads: 0,
            host_errors: 0,
            other: BTreeMap::new(),
        }
    }

    /// A copro with `data` (the copro data ROMs as 32-bit words) on its
    /// external bus, or none if `data` is empty.
    pub fn with_data(data: &[u32]) -> Self {
        Copro { rom: (!data.is_empty()).then(|| data.into()), ..Copro::new() }
    }

    /// FLAG0: input FIFO empty; FLAG1: output FIFO full (as
    /// `CoproBus::flags`).
    fn flags(&self) -> u8 {
        self.fifo_in.is_empty() as u8 | ((self.fifo_out.len() >= OUT_DEPTH) as u8) << 1
    }

    /// `0x980000` bit 31. Taking the bus starts a new upload with the
    /// SHARC reset into host boot mode; releasing it lets the program run.
    pub fn control(&mut self, hold: bool) {
        if hold && !self.held {
            *self = Copro {
                empty_reads: self.empty_reads,
                host_errors: self.host_errors,
                interpret: self.interpret,
                rom: self.rom.take(),
                ..Copro::new()
            };
        }
        if !hold && self.held {
            self.booted = true;
            self.attach();
        }
        self.held = hold;
    }

    /// Use the build's recompiled code for the program the core holds, if
    /// it has some and `interpret` isn't set.
    pub fn attach(&mut self) {
        self.native = None;
        if self.interpret {
            self.cpu.watch_code(&[], &[]);
        } else {
            self.native =
                crate::recomp::attach_sharc(&mut self.cpu, crate::recomp::SharcRole::Copro).and_then(|p| p.copro);
            if self.native.is_none() {
                self.cpu.watch_code(&[], &[]);
            }
        }
    }

    /// True if the copro runs recompiled code.
    pub fn is_native(&self) -> bool {
        self.native.is_some()
    }

    /// A host write to IOP register `reg` (the window at `0x8C0000`, or
    /// EPB0 through `0x884000` during an upload).
    pub fn host_write(&mut self, reg: u32, v: u32) {
        if self.held {
            self.program.push((reg as u64) << 32 | v as u64);
        }
        if self.cpu.host_write(reg, v).is_err() {
            self.host_errors += 1;
        }
    }

    pub fn host_read(&mut self, reg: u32) -> u32 {
        self.cpu.host_read(reg).unwrap_or_else(|_| {
            self.host_errors += 1;
            0
        })
    }

    /// Run up to `n` instructions, stopping early when the program waits
    /// on its FIFOs. `buffer` is buffer RAM, `commands` the geometrizer's
    /// command words in it (`Geometrizer::commands`).
    #[inline]
    pub fn run(&mut self, n: u64, buffer: &mut [u8], commands: &mut BTreeMap<u32, u32>) {
        if !self.booted || self.held || self.fault.is_some() {
            return;
        }
        if self.spin == Some(self.flags()) {
            return;
        }
        self.step(n, buffer, commands);
    }

    /// True if [`Copro::run`] does nothing until the host changes its
    /// FIFOs or control: no program, held, faulted, or spinning on a flag.
    pub fn idle(&self) -> bool {
        !self.booted || self.held || self.fault.is_some() || self.spin == Some(self.flags())
    }

    /// What `n` calls of `run(per, ..)` do, with nothing else touching the
    /// copro in between (the i960 in an idle loop), in as few runs as
    /// that allows: one, unless the core stops early. Then the calls left
    /// are no-ops if it spins on a flag or faulted, and if it went IDLE
    /// each of them ticks it once (and may wake it), so they run one by
    /// one until it is running again.
    pub fn run_calls(&mut self, n: u64, per: u64, buffer: &mut [u8], commands: &mut BTreeMap<u32, u32>) {
        let mut left = n;
        while left > 0 && !self.idle() {
            if self.cpu.idle {
                self.step(per, buffer, commands);
                left -= 1;
                continue;
            }
            let before = self.cpu.steps;
            self.step(per * left, buffer, commands);
            let ran = self.cpu.steps - before;
            if ran >= per * left {
                return;
            }
            // It stopped trying step `ran + 1`, which the call after
            // `ran / per` full ones made.
            left -= (ran / per + 1).min(left);
        }
    }

    #[inline(never)]
    fn step(&mut self, n: u64, buffer: &mut [u8], commands: &mut BTreeMap<u32, u32>) {
        self.spin = None;
        if n == 0 {
            return;
        }
        let mut bus = CoproBus {
            fifo_in: &mut self.fifo_in,
            fifo_out: &mut self.fifo_out,
            empty_reads: &mut self.empty_reads,
            other: &mut self.other,
            buffer,
            commands,
            rom: self.rom.as_deref(),
            end: self.cpu.steps + n,
            flags: 0,
            spin: &mut self.spin,
        };
        bus.set_flags(&mut self.cpu);
        let r = match self.native {
            Some(run) => run(&mut self.cpu, &mut bus),
            None => self.cpu.run_bus(&mut bus),
        };
        if let Step::Fault(f) = r {
            self.fault = Some(f);
        }
    }

    /// Let the copro run until it has a result for the i960 (which stalls
    /// on an empty output FIFO), or for a bounded time.
    pub fn run_for_result(&mut self, buffer: &mut [u8], commands: &mut BTreeMap<u32, u32>) {
        let mut left = STALL_LIMIT;
        while self.fifo_out.is_empty() && left > 0 && self.booted && !self.held && self.fault.is_none() {
            let before = self.cpu.steps;
            self.spin = None;
            self.step(left.min(4096), buffer, commands);
            let ran = self.cpu.steps - before;
            if ran == 0 || self.spin.is_some() {
                break;
            }
            left = left.saturating_sub(ran);
        }
    }
}

/// The program a SHARC holds after an upload's host writes (`IOP register
/// << 32 | data` each): its 48-bit words from 0x20000 up to the last
/// nonzero one, as `remodel sharc-disasm` reads them.
pub fn image(host_writes: &[u64]) -> Vec<u64> {
    let mut s = Sharc::new();
    for &w in host_writes {
        let _ = s.host_write((w >> 32) as u32, w as u32);
    }
    adsp2106x::cpu::program_image(&s)
}

/// The most executed words of a SHARC's internal memory: (address, count).
pub fn hot_spots(cpu: &Sharc, n: usize) -> Vec<(u32, u64)> {
    let mut v: Vec<(u32, u64)> =
        (0x2_0000..0x2_8000u32).filter_map(|a| cpu.count_at(a).filter(|&c| c > 0).map(|c| (a, c))).collect();
    v.sort_by_key(|&(a, c)| (std::cmp::Reverse(c), a));
    v.truncate(n);
    v
}

// Save states: the core, the FIFOs and the bus state (buffer RAM is the
// board's). Diagnostics stay.
impl Copro {
    /// Make this copro's state `src`'s, as a save and load would, keeping
    /// what its core has decoded ([`Sharc::copy_from`]) and taking `src`'s
    /// recompiled code (the same program) instead of looking it up again.
    pub(crate) fn copy_from(&mut self, src: &Copro, buf: &mut Vec<u8>) {
        self.cpu.copy_from(&src.cpu, buf);
        self.fifo_in.clone_from(&src.fifo_in);
        self.fifo_out.clone_from(&src.fifo_out);
        (self.held, self.booted) = (src.held, src.booted);
        self.fault = src.fault;
        self.spin = src.spin;
        self.native = src.native;
    }
}

impl state::State for Copro {
    fn save(&self, w: &mut state::Writer) {
        self.cpu.save(w);
        self.fifo_in.save(w);
        self.fifo_out.save(w);
        self.held.save(w);
        self.booted.save(w);
    }
    fn load(&mut self, r: &mut state::Reader) -> Result<(), state::Error> {
        self.cpu.load(r)?;
        self.fifo_in.load(r)?;
        self.fifo_out.load(r)?;
        self.held.load(r)?;
        self.booted.load(r)?;
        self.fault = None;
        self.spin = None;
        self.attach();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adsp2106x::cpu::{C6, DMAC6, EPB0, II6, IM6};

    /// Boot `words` (from 0x20000) the way the 2B i960 does.
    fn upload(c: &mut Copro, words: &[u64]) {
        c.control(true);
        let w32 = |c: &mut Copro, reg, v: u32| {
            c.host_write(reg, v & 0xffff);
            c.host_write(reg, v >> 16);
        };
        w32(c, DMAC6, 0);
        w32(c, II6, 0x2_0000);
        w32(c, IM6, 1);
        w32(c, C6, words.len() as u32);
        w32(c, DMAC6, 0xa1);
        for &w in words {
            for k in 0..3 {
                c.host_write(EPB0, (w >> (16 * k)) as u32 & 0xffff);
            }
        }
        c.control(false);
    }

    /// A copro that answers each command word with the word plus one,
    /// waiting on FLAG0 (input empty) and FLAG1 (output full) as the 2B
    /// program does.
    #[test]
    fn fifos_flags_and_spinning() {
        let mut p = vec![0u64; 0x89];
        p[0x05] = 0x063e_0002_0080; // JUMP 0x20080
        p[0x40] = 0x0b3e_0000_0000; // RTI (the boot DMA's interrupt)
        p[0x80..0x89].copy_from_slice(&[
            0x0f10_0040_0000, // I0 = 0x400000
            0x0f11_00c0_0000, // I1 = 0xc00000
            0x0f20_0000_0000, // M0 = 0
            0x0712_0000_0000, // IF FLAG0_IN JUMP (PC,0)
            0x503e_0000_0000, // R0 = DM(I0,M0)
            0x013e_0002_9000, // R0 = R0 + 1
            0x0714_0000_0000, // IF FLAG1_IN JUMP (PC,0)
            0x523e_8000_0000, // DM(I1,M0) = R0
            0x063e_0002_0083, // JUMP 0x20083
        ]);
        let mut c = Copro::new();
        let mut buffer = vec![0u8; 0x2_0000];
        let mut commands = BTreeMap::new();
        upload(&mut c, &p);
        assert_eq!(c.host_errors, 0);
        c.run(100, &mut buffer, &mut commands);
        assert!(c.spin.is_some(), "waits for a command");
        let steps = c.cpu.steps;
        c.run(100, &mut buffer, &mut commands);
        assert_eq!(c.cpu.steps, steps, "not stepped while nothing changed");
        c.fifo_in.extend([41, 99]);
        c.run(100, &mut buffer, &mut commands);
        assert_eq!(c.fifo_out, [42, 100]);
        assert!(c.fifo_in.is_empty() && c.spin.is_some());
        // A full output FIFO holds it at FLAG1 until the i960 reads.
        c.fifo_in.extend(0..OUT_DEPTH as u32 + 3);
        c.run(10_000, &mut buffer, &mut commands);
        assert_eq!(c.fifo_out.len(), OUT_DEPTH);
        c.fifo_out.clear();
        c.run_for_result(&mut buffer, &mut commands);
        assert!(!c.fifo_out.is_empty());
        let bytes = state::to_bytes(&c);
        let mut d = Copro::new();
        state::State::load(&mut d, &mut state::Reader::new(&bytes)).unwrap();
        c.run(10_000, &mut buffer, &mut commands);
        d.run(10_000, &mut buffer, &mut commands);
        assert_eq!(c.fifo_out, d.fifo_out);
    }

    /// `run_calls(n, 5)` leaves the copro as `n` calls of `run(5)` do:
    /// running, stopping to spin on a flag part way through a call, and
    /// going IDLE (which ticks it once per call from then on).
    #[test]
    fn batched_runs_are_single_runs() {
        let mut spin = vec![0u64; 0x89];
        spin[0x05] = 0x063e_0002_0080; // JUMP 0x20080
        spin[0x40] = 0x0b3e_0000_0000; // RTI
        spin[0x80..0x89].copy_from_slice(&[
            0x0f10_0040_0000, // I0 = 0x400000
            0x0f11_00c0_0000, // I1 = 0xc00000
            0x0f20_0000_0000, // M0 = 0
            0x0712_0000_0000, // IF FLAG0_IN JUMP (PC,0)
            0x503e_0000_0000, // R0 = DM(I0,M0)
            0x013e_0002_9000, // R0 = R0 + 1
            0x0714_0000_0000, // IF FLAG1_IN JUMP (PC,0)
            0x523e_8000_0000, // DM(I1,M0) = R0
            0x063e_0002_0083, // JUMP 0x20083
        ]);
        let mut idle = spin.clone();
        idle[0x84] = 0x0080_0000_0000; // IDLE in place of the read
        for (p, words) in [(&spin, 0..40u32), (&idle, 0..1)] {
            for n in [1, 2, 3, 7, 50, 400] {
                let (mut a, mut b) = (Copro::new(), Copro::new());
                let (mut buffer, mut commands) = (vec![0u8; 0x2_0000], BTreeMap::new());
                for c in [&mut a, &mut b] {
                    upload(c, p);
                    c.fifo_in.extend(words.clone());
                }
                for _ in 0..n {
                    a.run(STEPS_PER_INSN, &mut buffer, &mut commands);
                }
                b.run_calls(n, STEPS_PER_INSN, &mut buffer, &mut commands);
                assert_eq!(state::to_bytes(&a), state::to_bytes(&b), "{n} calls");
                assert_eq!((a.cpu.steps, a.cpu.cycles), (b.cpu.steps, b.cpu.cycles), "{n} calls");
            }
        }
    }

    /// Buffer RAM in bank 2 (word addressed, repeating through the bank)
    /// and the data ROMs in bank 3, repeating through theirs; no ROMs
    /// without data ROMs.
    #[test]
    fn external_memory() {
        let mut buffer = vec![0u8; 0x2_0000];
        // A command word at buffer word 7, which the store there ends.
        let mut commands = BTreeMap::from([(28, 0)]);
        let (mut fifo_in, mut fifo_out, mut empty, mut other, mut spin) =
            (VecDeque::new(), VecDeque::new(), 0, BTreeMap::new(), None);
        let rom = [10, 11, 12];
        let mut bus = CoproBus {
            fifo_in: &mut fifo_in,
            fifo_out: &mut fifo_out,
            empty_reads: &mut empty,
            other: &mut other,
            buffer: &mut buffer,
            commands: &mut commands,
            rom: Some(&rom),
            end: 0,
            flags: 0,
            spin: &mut spin,
        };
        assert_eq!(bus.read(ROM_BANK + 1, Width::Data32), 11);
        assert_eq!(bus.read(ROM_BANK + 4, Width::Data32), 11);
        bus.write(BUFFER_BANK + 7, Width::Data32, 0x1234_5678);
        assert_eq!(bus.read(BUFFER_BANK + 7, Width::Data32), 0x1234_5678);
        assert_eq!(bus.read(BUFFER_BANK + 0x8000 + 7, Width::Data32), 0x1234_5678);
        assert_eq!(bus.buffer[28..32], [0x78, 0x56, 0x34, 0x12]);
        assert!(bus.commands.is_empty());
        bus.rom = None;
        assert_eq!(bus.read(ROM_BANK + 1, Width::Data32), 0);
        assert_eq!(other.len(), 1);
        assert!(Copro::with_data(&[]).rom.is_none());
    }
}
