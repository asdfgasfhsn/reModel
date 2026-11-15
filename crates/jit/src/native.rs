//! The Cranelift back end (docs/jit.md).

use crate::hotlist::{Chip, HotList};
use crate::pool::{Compiled, Compiler, Job, Ticket, Urgency};
use cranelift_codegen::ir::{
    condcodes::IntCC, types, AbiParam, Block as ClifBlock, BlockArg, Endianness, InstBuilder, MemFlagsData, SigRef,
    Signature, Type, Value,
};
use cranelift_frontend::FunctionBuilder;
use cranelift_jit::JITModule;
use cranelift_module::Module;
use i960::cpu::{Cpu, Host, Stop};
use i960::lower::{lower, Alu, Op, Src, Width};
use i960::{decode, Flow, Insn, MemRef};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::marker::PhantomData;
use std::mem::offset_of;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Code is translated below this address (the program ROM and the RAM the
/// games copy code to); anything above runs on the interpreter.
pub const CODE_SPAN: u32 = 0x100_0000;
/// Bytes per write counter of [`JitHost::code_generations`].
pub const GEN_PAGE: u32 = 1 << GEN_SHIFT;
pub const GEN_SHIFT: u32 = 10;

/// What the JIT needs from the machine beyond what recompiled code does:
/// a way to see writes to code in writable memory, and (optionally) its
/// plain memory and quiet stretches, which blocks use without a call.
pub trait JitHost: Host {
    /// Write counters: one per [`GEN_PAGE`] bytes of the writable memories
    /// code may run from, bumped by every write into them. The JIT keeps
    /// a block from writable memory only while the counters of the pages
    /// it was read from (or its words, compared, when they moved) say it
    /// still holds what was translated.
    fn code_generations(&self) -> &[u32];
    /// The counter of the byte at `addr`, if it is in writable memory the
    /// host counts writes to. `None` there (and for unmapped addresses)
    /// leaves the code to the interpreter.
    fn code_generation_index(&self, addr: u32) -> Option<usize>;
    /// The plain memory below [`FAST_SPAN`], [`FAST_PAGES`] entries a
    /// [`FAST_PAGE`] each, for this run of a block (docs/jit.md, "Plain
    /// memory inline"): an aligned access to a page with an entry is made
    /// by the block itself, exactly as the bus would make it; anything
    /// else goes through the bus. [`no_fast_pages`] (the default) sends
    /// everything through the bus.
    fn fast_pages(&mut self) -> *const FastPage {
        no_fast_pages()
    }
    /// How many retires from now are quiet: each would only pass time
    /// (no event, no interrupt line, no other processor running) and
    /// return true, so the next that many may be made at once, later, by
    /// [`JitHost::retire_quiet`], as long as nothing reaches the machine in
    /// between (docs/jit.md, "A cheaper step"). 0 (the default): none.
    fn quiet(&mut self) -> u64 {
        0
    }
    /// Make `n` retires that [`JitHost::quiet`] said are quiet, exactly as
    /// `n` calls of [`Host::retire`] would.
    fn retire_quiet(&mut self, cpu: &mut Cpu, n: u64) {
        for _ in 0..n {
            self.retire(cpu);
        }
    }
}

/// Plain memory is used inline below this address.
pub const FAST_SPAN: u32 = 0x400_0000;
/// Bytes per [`FastPage`].
pub const FAST_PAGE: u32 = 1 << FAST_SHIFT;
pub const FAST_SHIFT: u32 = 16;
pub const FAST_PAGES: usize = (FAST_SPAN >> FAST_SHIFT) as usize;

/// One [`FAST_PAGE`] of the address space as blocks see it: where its
/// bytes are, if it is all plain memory (a read has no effect but its
/// value), and what a write there changes besides them, if it is plain
/// writable memory too.
#[repr(C, align(32))]
#[derive(Clone, Copy, Debug)]
pub struct FastPage {
    /// The page's first byte in host memory, its bytes in order
    /// (little-endian words), or null: accesses here go through the bus.
    pub data: *mut u8,
    /// For writes: the memory's write count, which a write adds 1 to, or
    /// null: writes here go through the bus.
    pub writes: *mut u64,
    /// For writes into code RAM: the write counter
    /// ([`JitHost::code_generations`]) of the page's first [`GEN_PAGE`],
    /// those of the rest following it, or null. A write (aligned, so
    /// within one counter's page) adds `gen_add` to its counter.
    pub gen: *mut u32,
    pub gen_add: u32,
}

impl FastPage {
    pub const NONE: FastPage =
        FastPage { data: std::ptr::null_mut(), writes: std::ptr::null_mut(), gen: std::ptr::null_mut(), gen_add: 0 };
}

struct NoFast([FastPage; FAST_PAGES]);
// SAFETY: all null; nothing is ever reached through it.
unsafe impl Sync for NoFast {}
static NO_FAST: NoFast = NoFast([FastPage::NONE; FAST_PAGES]);

/// A table of [`FAST_PAGES`] empty entries: every access through the bus.
pub fn no_fast_pages() -> *const FastPage {
    NO_FAST.0.as_ptr()
}

/// How the JIT runs.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Times a block start is dispatched on the interpreter before it is
    /// translated (0: at once).
    pub hot: u32,
    /// Cranelift's optimiser (`speed`). Off by default: it adds half
    /// again to the translation time for a few percent (docs/jit.md); the
    /// full register allocator is on either way.
    pub optimise: bool,
    /// Instructions per block at most.
    pub max_block: usize,
    /// The copro programs on their JITs too (the MB86234 of Model 2A, the
    /// MB86235s of 2C: `docs/jit.md`, "Copro JITs"); with `copro_check`,
    /// each beside its interpreter, compared after every run (a lockstep).
    pub copro: bool,
    pub copro_check: bool,
    /// Translate on the background workers (`pool`), interpreting a block
    /// until its code is ready, so the game never waits for Cranelift
    /// (docs/jit.md, "Translation off the game's thread"). Off: at once,
    /// on the thread that needs it (tests, measurements).
    pub background: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config { hot: 2, optimise: false, max_block: 128, copro: true, copro_check: false, background: true }
    }
}

/// What the JIT has done.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    /// Blocks translated, and their instructions.
    pub blocks: u64,
    pub insns: u64,
    /// Of those instructions, how many call the interpreter.
    pub fallbacks: u64,
    /// Blocks from writable memory among them.
    pub ram_blocks: u64,
    /// Host time translating (IR, Cranelift, linking), and the slowest
    /// block (on the workers).
    pub time: Duration,
    pub slowest: Duration,
    /// Of `time`: building the IR and compiling it (on the workers), and
    /// copying the code in and making it executable (on the JIT's
    /// thread).
    pub compile: Duration,
    pub link: Duration,
    /// Bytes of machine code.
    pub code_bytes: u64,
    /// Blocks run, and instructions run on the interpreter because no
    /// block started there (yet).
    pub runs: u64,
    pub interpreted: u64,
    /// Writable-memory blocks whose pages were written since: checked word
    /// by word and kept, or dropped because the code changed.
    pub rechecked: u64,
    pub dropped: u64,
    /// Finished translations thrown away: their code was written while
    /// they were being made (or a newer one was asked for).
    pub discarded: u64,
}

/// A translated block: `(cpu, host, ctx) -> 0` (ran) or 1 (faulted, the
/// fault in `ctx`).
type BlockFn = unsafe extern "C" fn(*mut Cpu, *mut u8, *mut Ctx) -> u32;

/// Passed to every block: where the host's write counters and plain
/// memory are this time, the quiet retires it may still make inline, and
/// where a helper leaves a fault.
#[repr(C)]
struct Ctx {
    gens: *const u32,
    fast: *const FastPage,
    /// Quiet retires left ([`JitHost::quiet`]): while nonzero, an
    /// instruction's step is counting it, inline. `granted - quiet` are
    /// owed to the machine, made ([`settle`]) before anything reaches it.
    quiet: u64,
    granted: u64,
    fault: Option<Stop>,
}

/// A block from writable memory: the words it was translated from, the
/// counters of the pages they are on, and those counters' values when the
/// words were last known to be there (read by the block after each of its
/// own stores).
struct RamCode {
    words: Box<[u32]>,
    pages: [usize; 2],
    seen: Box<[u32; 2]>,
}

struct Block {
    code: BlockFn,
    start: u32,
    /// The instructions it holds, for the ones it hands the interpreter
    /// (by address, so they must not move).
    #[allow(dead_code)]
    insns: Box<[Insn]>,
    ram: Option<RamCode>,
}

#[derive(Clone, Copy)]
enum Slot {
    /// Not translated: dispatched this many times.
    Cold(u32),
    /// Being translated (on a worker): interpreted until it is ready.
    Queued,
    Block(u32),
    /// Nothing to translate here (undecodable, or not code the JIT can
    /// track).
    Never,
}

const SLOTS_PER_PAGE: usize = 1024;

/// A block being translated: its job's number (a later one for the same
/// start would supersede it) and, from writable memory, what it is being
/// translated from.
struct Pending {
    seq: u64,
    end: u32,
    ram: Option<RamCode>,
    /// Queued ahead of time (`Later`), not yet promoted: the job, to move
    /// it up when the game gets there first.
    later: Option<Ticket>,
}

/// A finished translation, back from a worker.
struct Done {
    start: u32,
    seq: u64,
    insns: Box<[Insn]>,
    code: Result<Compiled, String>,
}

/// The JIT for a machine `H`.
pub struct Jit<H> {
    module: JITModule,
    blocks: Vec<Block>,
    /// Block (or state) by instruction address / 4, in pages.
    table: Vec<Option<Box<[Slot; SLOTS_PER_PAGE]>>>,
    /// Addresses a block must stop before (the machine's idle loops: it
    /// looks at them between blocks).
    stops: BTreeSet<u32>,
    config: Config,
    stats: Stats,
    /// Instructions left to the interpreter in translated blocks, by
    /// mnemonic.
    fallback_kinds: BTreeMap<&'static str, u64>,
    pending: HashMap<u32, Pending>,
    seq: u64,
    /// The rest of a queued block the interpreter is running through
    /// (after its start, before its end): its addresses aren't block
    /// starts, so they aren't counted towards translation.
    through: (u32, u32),
    done_tx: Sender<Done>,
    done_rx: Receiver<Done>,
    /// Finished translations received, by start, not yet installed: each
    /// goes in when the game reaches it (copying a block in costs about
    /// 10 µs, so a boot's whole hot-block list going in at once would be
    /// a hitch of its own).
    received: HashMap<u32, Done>,
    /// Cleared when the JIT is dropped: its jobs not yet started are
    /// skipped.
    alive: Arc<AtomicBool>,
    /// The hot-block list this JIT notes its blocks in, and the program's
    /// hash (docs/jit.md, "Hot-block list").
    list: Option<(Arc<HotList>, u64)>,
    /// Listed starts in writable memory, translated once the game first
    /// runs code there (it has copied its code by then).
    listed_ram: Vec<u32>,
    host: PhantomData<fn(&mut H)>,
}

impl<H> Drop for Jit<H> {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
    }
}

impl<H: JitHost + 'static> Jit<H> {
    pub fn new(config: Config) -> Result<Self, String> {
        let module = crate::copro::module(config.optimise)?;
        let (done_tx, done_rx) = channel();
        Ok(Jit {
            module,
            blocks: Vec::new(),
            table: Vec::new(),
            stops: BTreeSet::new(),
            config,
            stats: Stats::default(),
            fallback_kinds: BTreeMap::new(),
            pending: HashMap::new(),
            seq: 0,
            through: (0, 0),
            done_tx,
            done_rx,
            received: HashMap::new(),
            alive: Arc::new(AtomicBool::new(true)),
            list: None,
            listed_ram: Vec::new(),
            host: PhantomData,
        })
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// The instructions translated blocks leave to the interpreter, by
    /// mnemonic, most first (where translating more would pay).
    pub fn fallback_kinds(&self) -> Vec<(&'static str, u64)> {
        let mut v: Vec<_> = self.fallback_kinds.iter().map(|(&k, &n)| (k, n)).collect();
        v.sort_by_key(|&(k, n)| (std::cmp::Reverse(n), k));
        v
    }

    pub fn config(&self) -> Config {
        self.config
    }

    /// Note translated blocks in `list`, under the program `hash`, and
    /// queue the blocks it already lists for translation now, behind any
    /// the game is waiting for (those in ROM; those in writable memory
    /// when the game first runs code there). Set the stops first. Entries
    /// are hints: one that doesn't decode is left alone, one for other
    /// code costs a translation, and code in RAM is checked as always.
    pub fn set_hot_list(&mut self, list: Arc<HotList>, hash: u64, host: &H) {
        for a in list.get(Chip::I960, hash) {
            if a >= CODE_SPAN || a & 3 != 0 || !matches!(*self.slot(a), Slot::Cold(_)) {
                continue;
            }
            if host.is_rom(a) {
                if let Some(insns) = self.plan(a, host) {
                    self.queue(insns, host, Urgency::Later);
                }
            } else if host.code_generation_index(a).is_some() {
                self.listed_ram.push(a);
            }
        }
        self.list = Some((list, hash));
    }

    /// Blocks stop before these addresses (they still start at them).
    pub fn set_stops(&mut self, stops: impl IntoIterator<Item = u32>) {
        self.stops = stops.into_iter().collect();
    }

    fn slot(&mut self, ip: u32) -> &mut Slot {
        let i = (ip / 4) as usize;
        let (page, at) = (i / SLOTS_PER_PAGE, i % SLOTS_PER_PAGE);
        if self.table.is_empty() {
            self.table.resize_with((CODE_SPAN / 4) as usize / SLOTS_PER_PAGE, || None);
        }
        &mut self.table[page].get_or_insert_with(|| Box::new([Slot::Cold(0); SLOTS_PER_PAGE]))[at]
    }

    /// Run the block at the CPU's IP, if there is one. `None` if there is
    /// none (yet): the caller runs one instruction on the interpreter. A
    /// block start that is due is queued for translation (docs/jit.md,
    /// "Translation off the game's thread"), and interpreted until its
    /// code is ready. Interrupts must have been taken first, as for a
    /// recompiled block.
    #[inline]
    pub fn run(&mut self, cpu: &mut Cpu, host: &mut H) -> Option<Result<(), Stop>> {
        let ip = cpu.ip;
        if ip >= CODE_SPAN || ip & 3 != 0 {
            self.stats.interpreted += 1;
            return None;
        }
        let (slot, hot) = (*self.slot(ip), self.config.hot);
        let id = match slot {
            Slot::Block(id) => {
                self.through = (0, 0);
                id
            }
            Slot::Never => {
                self.stats.interpreted += 1;
                return None;
            }
            _ if self.through.0 < ip && ip < self.through.1 => {
                self.stats.interpreted += 1;
                return None;
            }
            Slot::Cold(n) if n < hot => {
                *self.slot(ip) = Slot::Cold(n + 1);
                self.stats.interpreted += 1;
                return None;
            }
            Slot::Cold(_) | Slot::Queued => match self.cold(ip, host) {
                Some(id) => id,
                None => {
                    self.stats.interpreted += 1;
                    return None;
                }
            },
        };
        let mut id = id;
        if self.blocks[id as usize].ram.is_some() && !self.still_there(id, host) {
            // The code changed: translate what is there now.
            self.stats.dropped += 1;
            *self.slot(ip) = Slot::Cold(hot);
            match self.cold(ip, host) {
                Some(new) => id = new,
                None => {
                    self.stats.interpreted += 1;
                    return None;
                }
            }
        }
        self.stats.runs += 1;
        let gens = host.code_generations().as_ptr();
        let fast = host.fast_pages();
        // An interrupt due already (one more of priority 31 after the
        // dispatcher took one) is taken after the first instruction.
        let quiet = if cpu.interrupt_due() { 0 } else { host.quiet() };
        let mut ctx = Ctx { gens, fast, quiet, granted: quiet, fault: None };
        let code = self.blocks[id as usize].code;
        // SAFETY: the block was made for `H` by `emit`, from instructions
        // it owns; it only touches the CPU and host through the helpers
        // below, the CPU's fields by offset and the host's plain memory
        // through `fast`.
        let r = unsafe { code(cpu, host as *mut H as *mut u8, &mut ctx) };
        settle(cpu, host, &mut ctx);
        match r {
            0 => Some(Ok(())),
            _ => Some(Err(ctx.fault.expect("a faulting helper leaves its fault"))),
        }
    }

    /// A start with no block yet: queue it if it isn't, install what the
    /// workers have finished, and say whether its block is among them.
    #[cold]
    fn cold(&mut self, ip: u32, host: &H) -> Option<u32> {
        if let Some(t) = self.pending.get_mut(&ip).and_then(|p| p.later.take()) {
            crate::pool::promote(&t);
        }
        if let Slot::Cold(_) = *self.slot(ip) {
            self.queue_at(ip, host, Urgency::Now);
            if !self.listed_ram.is_empty() && !host.is_rom(ip) {
                self.queue_listed_ram(host);
            }
        }
        self.install(host, ip);
        match *self.slot(ip) {
            Slot::Block(id) => Some(id),
            _ => {
                // Interpreted until it is ready: what follows its start is
                // the block's, not starts of its own.
                self.through = self.pending.get(&ip).map_or((0, 0), |p| (ip, p.end));
                None
            }
        }
    }

    /// The listed starts in writable memory, planned from what is there
    /// now (a start that doesn't decode is left to the game).
    fn queue_listed_ram(&mut self, host: &H) {
        for a in std::mem::take(&mut self.listed_ram) {
            if matches!(*self.slot(a), Slot::Cold(_)) {
                if let Some(insns) = self.plan(a, host) {
                    self.queue(insns, host, Urgency::Later);
                }
            }
        }
    }

    /// True if block `id` (from writable memory) still holds what is in
    /// memory: its pages weren't written, or its words are the same.
    fn still_there(&mut self, id: u32, host: &H) -> bool {
        let b = &mut self.blocks[id as usize];
        let Some(ram) = &mut b.ram else { return true };
        ram.current(b.start, host, &mut self.stats)
    }

    /// Queue the block at `ip` for translation (planned now, from memory
    /// as it is), or mark it as nothing to translate. True if queued.
    fn queue_at(&mut self, ip: u32, host: &H, urgency: Urgency) -> bool {
        let queued = self.plan(ip, host).is_some_and(|insns| self.queue(insns, host, urgency));
        if !queued {
            *self.slot(ip) = Slot::Never;
        }
        queued
    }

    /// The instructions of the block at `start`: up to and including the
    /// first that transfers control, stopping short of a stop address and
    /// of anything that doesn't decode.
    fn plan(&self, start: u32, host: &H) -> Option<Vec<Insn>> {
        let rom = host.is_rom(start);
        let mut v = Vec::new();
        let mut pc = start;
        loop {
            let i = decode(pc, |a| host.fetch(a));
            // One kind of memory per block: ROM, or writable memory checked
            // for writes.
            if i.flow == Flow::Invalid || host.is_rom(pc) != rom {
                break;
            }
            let ends = !matches!(i.flow, Flow::Next | Flow::CondFault);
            let next = i.next();
            v.push(i);
            if ends || v.len() >= self.config.max_block || self.stops.contains(&next) || next >= CODE_SPAN {
                break;
            }
            pc = next;
        }
        (!v.is_empty()).then_some(v)
    }

    /// Translate the whole program as the static recompiler cuts it: a
    /// block at each leader of `insns`, running to the next. Returns how
    /// many blocks; times them in [`Stats`]. (A measurement: these blocks
    /// are used from then on, as if each had been translated when it
    /// first ran.) The workers translate them, all at once, and this
    /// waits for them.
    pub fn translate_program(&mut self, insns: &BTreeMap<u32, Insn>, extra: &BTreeSet<u32>, host: &H) -> usize {
        let leaders = i960::codegen::leaders(insns, extra);
        let before = self.stats.blocks;
        for &start in &leaders {
            if start >= CODE_SPAN || start & 3 != 0 || !host.is_rom(start) {
                continue;
            }
            let mut block = Vec::new();
            let mut pc = start;
            loop {
                let i = insns[&pc].clone();
                let next = i.next();
                let ends = !matches!(i.flow, Flow::Next | Flow::CondFault);
                block.push(i);
                if ends || !insns.contains_key(&next) || leaders.contains(&next) || self.stops.contains(&next) {
                    break;
                }
                pc = next;
            }
            self.queue(block, host, Urgency::Now);
        }
        self.wait(host);
        (self.stats.blocks - before) as usize
    }

    /// Hand `insns` (a block) to the translation workers. False if it
    /// can't be translated (code in writable memory the host doesn't
    /// track, or over more than two of its counters' pages).
    fn queue(&mut self, insns: Vec<Insn>, host: &H, urgency: Urgency) -> bool {
        let start = insns[0].addr;
        let end = insns.last().map_or(start, Insn::next);
        let ram = if host.is_rom(start) {
            None
        } else {
            let (Some(first), Some(last)) = (host.code_generation_index(start), host.code_generation_index(end - 1))
            else {
                return false;
            };
            // A block over more than two counters' pages isn't worth the
            // checks (they are 1 KB; blocks are at most 128 instructions).
            if last.abs_diff(first) > 1 {
                return false;
            }
            let gens = host.code_generations();
            let words: Option<Vec<u32>> = (start..end).step_by(4).map(|a| host.fetch(a)).collect();
            let Some(words) = words else { return false };
            Some(RamCode {
                words: words.into_boxed_slice(),
                pages: [first, last],
                seen: Box::new([gens[first], gens[last]]),
            })
        };
        self.seq += 1;
        let seq = self.seq;
        // The block reads the counters as last checked from here: the box
        // goes with the block when it is installed, so it doesn't move.
        let ram_at = ram.as_ref().map(|r| (r.pages, &*r.seen as *const [u32; 2] as usize));
        let insns = insns.into_boxed_slice();
        let (tx, alive, optimise) = (self.done_tx.clone(), self.alive.clone(), self.config.optimise);
        let job: Job = Box::new(move |c: &mut Compiler| {
            if !alive.load(Ordering::Relaxed) {
                return;
            }
            let code = catch_unwind(AssertUnwindSafe(|| {
                c.compile(crate::pool::Opt::of(optimise, crate::pool::Opt::Alloc), 3, Some(I32), |b, ptr| {
                    // The instructions are boxed, so the addresses the
                    // block hands the interpreter stay valid once it is
                    // back.
                    let mut e = Emit::<H>::new(b, ptr, ram_at.map(|(p, s)| (p, s as *const [u32; 2])));
                    e.block(&insns);
                })
            }))
            .unwrap_or_else(|_| Err("the translation panicked".into()));
            let _ = tx.send(Done { start, seq, insns, code });
        });
        self.pending.insert(start, Pending { seq, end, ram, later: None });
        *self.slot(start) = Slot::Queued;
        let ticket = crate::pool::submit(self.config.background, urgency, job);
        if let (Some(p), Urgency::Later) = (self.pending.get_mut(&start), urgency) {
            p.later = ticket;
        }
        true
    }

    /// Wait for every queued block and install them (a measurement, and
    /// for tests; the game never waits).
    pub fn wait(&mut self, host: &H) {
        let mut ready = Vec::new();
        for (_, d) in std::mem::take(&mut self.received) {
            self.take(d, host, &mut ready);
        }
        while !self.pending.is_empty() {
            match self.done_rx.recv() {
                Ok(d) => self.take(d, host, &mut ready),
                Err(_) => break,
            }
        }
        self.finish(ready);
    }

    /// Install the block at `want` if the workers have finished it (at a
    /// block boundary: the dispatcher is between blocks).
    fn install(&mut self, host: &H, want: u32) {
        while let Ok(d) = self.done_rx.try_recv() {
            // A newer job for the same start supersedes an older one.
            if self.received.get(&d.start).is_none_or(|o| o.seq < d.seq) {
                self.received.insert(d.start, d);
            } else {
                self.stats.discarded += 1;
            }
        }
        if let Some(d) = self.received.remove(&want) {
            let mut ready = Vec::new();
            self.take(d, host, &mut ready);
            self.finish(ready);
        }
    }

    /// One finished translation: into this JIT's memory (`finish` makes it
    /// executable), or thrown away if it is stale.
    fn take(&mut self, d: Done, host: &H, ready: &mut Vec<Ready>) {
        if self.pending.get(&d.start).is_none_or(|p| p.seq != d.seq) {
            self.stats.discarded += 1;
            return;
        }
        let Some(mut p) = self.pending.remove(&d.start) else { return };
        let c = match d.code {
            Ok(c) => c,
            Err(e) => {
                eprintln!("jit: block at {:#x}: {e}", d.start);
                *self.slot(d.start) = Slot::Never;
                return;
            }
        };
        self.stats.compile += c.time;
        self.stats.time += c.time;
        // Code in writable memory that was written while it was being
        // translated: throw it away; the next visit translates what is
        // there now.
        if let Some(ram) = &mut p.ram {
            if !ram.current(d.start, host, &mut self.stats) {
                self.stats.discarded += 1;
                *self.slot(d.start) = Slot::Cold(self.config.hot);
                return;
            }
        }
        let t = Instant::now();
        let sig = self.block_signature();
        match c.define(&mut self.module, &sig) {
            Some(id) => {
                ready.push(Ready { id, start: d.start, insns: d.insns, ram: p.ram, size: c.bytes.len(), time: c.time })
            }
            None => *self.slot(d.start) = Slot::Never,
        }
        self.stats.link += t.elapsed();
        self.stats.time += t.elapsed();
    }

    /// Make the defined blocks executable and put them in the table.
    fn finish(&mut self, ready: Vec<Ready>) {
        if ready.is_empty() {
            return;
        }
        if let Some((list, hash)) = &self.list {
            list.add(Chip::I960, *hash, ready.iter().map(|r| r.start));
        }
        let t = Instant::now();
        if let Err(e) = self.module.finalize_definitions() {
            eprintln!("jit: {e:?}");
            for r in ready {
                *self.slot(r.start) = Slot::Never;
            }
            return;
        }
        for r in ready {
            let ptr = self.module.get_finalized_function(r.id);
            // SAFETY: made with `block_signature`, the C convention of `BlockFn`.
            let code = unsafe { std::mem::transmute::<*const u8, BlockFn>(ptr) };
            let fallbacks = r.insns.iter().filter(|i| lower(i).is_none()).count();
            for i in r.insns.iter().filter(|i| lower(i).is_none()) {
                *self.fallback_kinds.entry(i.mnemonic).or_default() += 1;
            }
            self.stats.blocks += 1;
            self.stats.insns += r.insns.len() as u64;
            self.stats.fallbacks += fallbacks as u64;
            self.stats.ram_blocks += r.ram.is_some() as u64;
            self.stats.code_bytes += r.size as u64;
            self.stats.slowest = self.stats.slowest.max(r.time);
            self.blocks.push(Block { code, start: r.start, insns: r.insns, ram: r.ram });
            *self.slot(r.start) = Slot::Block(self.blocks.len() as u32 - 1);
        }
        self.stats.link += t.elapsed();
        self.stats.time += t.elapsed();
    }

    fn block_signature(&self) -> Signature {
        let ptr = self.module.target_config().pointer_type();
        let mut s = self.module.make_signature();
        s.params.extend([AbiParam::new(ptr), AbiParam::new(ptr), AbiParam::new(ptr)]);
        s.returns.push(AbiParam::new(types::I32));
        s
    }
}

/// A block defined in the module, to be made executable.
struct Ready {
    id: cranelift_module::FuncId,
    start: u32,
    insns: Box<[Insn]>,
    ram: Option<RamCode>,
    size: usize,
    time: Duration,
}

impl RamCode {
    /// True if memory still holds the words this was translated from at
    /// `start`: its pages weren't written, or its words are the same
    /// (and the counters are noted as seen).
    fn current<H: JitHost>(&mut self, start: u32, host: &H, stats: &mut Stats) -> bool {
        let gens = host.code_generations();
        let now = [gens[self.pages[0]], gens[self.pages[1]]];
        if now == *self.seen {
            return true;
        }
        stats.rechecked += 1;
        let same = self.words.iter().enumerate().all(|(k, &w)| host.fetch(start + 4 * k as u32) == Some(w));
        if same {
            *self.seen = now;
        }
        same
    }
}

const I32: Type = types::I32;

fn trusted() -> MemFlagsData {
    MemFlagsData::trusted()
}

// Offsets of the CPU's fields the blocks use directly.
const OFF_R: usize = offset_of!(Cpu, r);
const OFF_IP: usize = offset_of!(Cpu, ip);
const OFF_AC: usize = offset_of!(Cpu, ac);
const OFF_ICOUNT: usize = offset_of!(Cpu, icount);

/// Guest memory as blocks reach it inline: little-endian, maybe unaligned
/// in host memory.
fn guest() -> MemFlagsData {
    MemFlagsData::new().with_notrap().with_endianness(Endianness::Little)
}

/// Helper calls' signatures, imported into the function being built.
struct Sigs {
    read: SigRef,
    write: SigRef,
    call: SigRef,
    ret: SigRef,
    exec: SigRef,
    step: SigRef,
}

/// Emits one block's IR.
struct Emit<'a, 'b, H> {
    b: &'a mut FunctionBuilder<'b>,
    ptr: Type,
    cpu: Value,
    host: Value,
    ctx: Value,
    sigs: Sigs,
    /// Shared exit for a helper's fault: returns 1.
    fault: ClifBlock,
    /// For a block from writable memory: its counters' indices and where
    /// it keeps their values as last checked.
    ram: Option<([usize; 2], *const [u32; 2])>,
    host_type: PhantomData<fn(&mut H)>,
}

impl<'a, 'b, H: JitHost> Emit<'a, 'b, H> {
    fn new(b: &'a mut FunctionBuilder<'b>, ptr: Type, ram: Option<([usize; 2], *const [u32; 2])>) -> Self {
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let p = b.block_params(entry).to_vec();
        let mut sig = |params: &[Type], ret: bool| {
            let mut s = Signature::new(b.func.signature.call_conv);
            s.params.extend(params.iter().map(|&t| AbiParam::new(t)));
            if ret {
                s.returns.push(AbiParam::new(I32));
            }
            b.import_signature(s)
        };
        let sigs = Sigs {
            read: sig(&[ptr, ptr, ptr, I32, I32], true),
            write: sig(&[ptr, ptr, ptr, I32, I32, I32], false),
            call: sig(&[ptr, ptr, ptr, I32, I32, I32], false),
            ret: sig(&[ptr, ptr, ptr, I32], true),
            exec: sig(&[ptr, ptr, ptr, ptr], true),
            step: sig(&[ptr, ptr, ptr, I32, I32], true),
        };
        let fault = b.create_block();
        Emit { b, ptr, cpu: p[0], host: p[1], ctx: p[2], sigs, fault, ram, host_type: PhantomData }
    }

    fn konst(&mut self, v: u32) -> Value {
        self.b.ins().iconst(I32, v as i64)
    }

    fn get(&mut self, r: u8) -> Value {
        self.b.ins().load(I32, trusted(), self.cpu, (OFF_R + 4 * (r as usize & 31)) as i32)
    }

    fn set(&mut self, r: u8, v: Value) {
        self.b.ins().store(trusted(), v, self.cpu, (OFF_R + 4 * (r as usize & 31)) as i32);
    }

    fn src(&mut self, s: Src) -> Value {
        match s {
            Src::Reg(r) => self.get(r),
            Src::Lit(v) => self.konst(v),
        }
    }

    fn set_ip(&mut self, v: Value) {
        self.b.ins().store(trusted(), v, self.cpu, OFF_IP as i32);
    }

    fn set_ip_const(&mut self, v: u32) {
        let v = self.konst(v);
        self.set_ip(v);
    }

    /// AC with the condition code replaced by `cc`.
    fn set_cc(&mut self, cc: Value) {
        let ac = self.b.ins().load(I32, trusted(), self.cpu, OFF_AC as i32);
        let kept = self.b.ins().band_imm_u(ac, !7i64 & 0xffff_ffff);
        let ac = self.b.ins().bor(kept, cc);
        self.b.ins().store(trusted(), ac, self.cpu, OFF_AC as i32);
    }

    /// The branch condition for `mask` (`Cpu::cond`), as a truth value.
    fn cond(&mut self, mask: u32) -> Value {
        let ac = self.b.ins().load(I32, trusted(), self.cpu, OFF_AC as i32);
        let cc = self.b.ins().band_imm_u(ac, 7);
        if mask == 0 {
            self.b.ins().icmp_imm_u(IntCC::Equal, cc, 0)
        } else {
            let m = self.b.ins().band_imm_u(cc, mask as i64);
            self.b.ins().icmp_imm_u(IntCC::NotEqual, m, 0)
        }
    }

    /// `Cpu::compare`: less 4, equal 2, greater 1.
    fn compare(&mut self, signed: bool, a: Value, b: Value) {
        let lt = self.b.ins().icmp(if signed { IntCC::SignedLessThan } else { IntCC::UnsignedLessThan }, a, b);
        let eq = self.b.ins().icmp(IntCC::Equal, a, b);
        let (four, two, one) = (self.konst(4), self.konst(2), self.konst(1));
        let ge = self.b.ins().select(eq, two, one);
        let cc = self.b.ins().select(lt, four, ge);
        self.set_cc(cc);
    }

    fn bit(&mut self, a: Value) -> Value {
        let one = self.konst(1);
        let n = self.b.ins().band_imm_u(a, 31);
        self.b.ins().ishl(one, n)
    }

    fn ea(&mut self, m: &MemRef) -> Value {
        match *m {
            MemRef::Abs(a) | MemRef::IpRel { target: a } => self.konst(a),
            MemRef::Base { abase, disp } => {
                let base = self.get(abase);
                self.b.ins().iadd_imm_s(base, disp as i32 as i64)
            }
            MemRef::Index { disp, index, scale } => {
                let i = self.get(index);
                let s = self.b.ins().imul_imm_u(i, scale as i64);
                self.b.ins().iadd_imm_s(s, disp as i32 as i64)
            }
            MemRef::BaseIndex { abase, disp, index, scale } => {
                let base = self.get(abase);
                let base = self.b.ins().iadd_imm_s(base, disp as i32 as i64);
                let i = self.get(index);
                let s = self.b.ins().imul_imm_u(i, scale as i64);
                self.b.ins().iadd(base, s)
            }
        }
    }

    fn helper(&mut self, f: usize) -> Value {
        self.b.ins().iconst(self.ptr, f as i64)
    }

    fn call(&mut self, sig: SigRef, f: usize, args: &[Value]) -> Option<Value> {
        let callee = self.helper(f);
        let call = self.b.ins().call_indirect(sig, callee, args);
        self.b.inst_results(call).first().copied()
    }

    /// The [`FastPage`] of `addr` (below [`FAST_SPAN`]).
    fn fast_page(&mut self, addr: Value) -> Value {
        let table = self.b.ins().load(self.ptr, trusted(), self.ctx, offset_of!(Ctx, fast) as i32);
        let page = self.b.ins().ushr_imm_u(addr, FAST_SHIFT as i64);
        let page = self.b.ins().uextend(self.ptr, page);
        let at = self.b.ins().ishl_imm_u(page, size_of::<FastPage>().trailing_zeros() as i64);
        self.b.ins().iadd(table, at)
    }

    /// Where `addr` is in the page whose bytes start at `data`.
    fn in_page(&mut self, data: Value, addr: Value) -> Value {
        let off = self.b.ins().band_imm_u(addr, (FAST_PAGE - 1) as i64);
        let off = self.b.ins().uextend(self.ptr, off);
        self.b.ins().iadd(data, off)
    }

    /// Nonzero unless `addr` is aligned for `width` bytes and below
    /// [`FAST_SPAN`]: the accesses that may be made inline.
    fn not_inline(&mut self, width: u32, addr: Value) -> Value {
        let mask = !(FAST_SPAN - 1) | (width - 1);
        self.b.ins().band_imm_u(addr, mask as i64)
    }

    /// A read of `width` bytes at `addr`: inline from plain memory
    /// ([`JitHost::fast_pages`]), else through the bus.
    fn read(&mut self, width: u32, addr: Value, ip: u32) -> Value {
        let (check, hit, slow, join) =
            (self.b.create_block(), self.b.create_block(), self.b.create_block(), self.b.create_block());
        let out = self.b.append_block_param(join, I32);
        self.b.set_cold_block(slow);
        let far = self.not_inline(width, addr);
        self.b.ins().brif(far, slow, &[], check, &[]);
        self.b.switch_to_block(check);
        let page = self.fast_page(addr);
        let data = self.b.ins().load(self.ptr, trusted(), page, offset_of!(FastPage, data) as i32);
        self.b.ins().brif(data, hit, &[], slow, &[]);
        self.b.switch_to_block(hit);
        let p = self.in_page(data, addr);
        let v = match width {
            1 => self.b.ins().uload8(I32, guest(), p, 0),
            2 => self.b.ins().uload16(I32, guest(), p, 0),
            _ => self.b.ins().load(I32, guest(), p, 0),
        };
        self.b.ins().jump(join, &[BlockArg::Value(v)]);
        self.b.switch_to_block(slow);
        let f = match width {
            1 => (read8::<H> as ReadFn) as usize,
            2 => (read16::<H> as ReadFn) as usize,
            _ => (read32::<H> as ReadFn) as usize,
        };
        let ip = self.konst(ip);
        let (cpu, host, ctx, sig) = (self.cpu, self.host, self.ctx, self.sigs.read);
        let v = self.call(sig, f, &[cpu, host, ctx, addr, ip]).expect("reads return a value");
        self.b.ins().jump(join, &[BlockArg::Value(v)]);
        self.b.switch_to_block(join);
        out
    }

    /// A write of `width` bytes of `v` at `addr`: inline into plain
    /// writable memory, counted as the bus counts it (the memory's write
    /// count, the code RAM counter of its [`GEN_PAGE`]), else through the
    /// bus.
    fn write(&mut self, width: u32, addr: Value, v: Value, ip: u32) {
        let (check, hit, gen, store, slow, join) = (
            self.b.create_block(),
            self.b.create_block(),
            self.b.create_block(),
            self.b.create_block(),
            self.b.create_block(),
            self.b.create_block(),
        );
        self.b.set_cold_block(slow);
        let far = self.not_inline(width, addr);
        self.b.ins().brif(far, slow, &[], check, &[]);
        self.b.switch_to_block(check);
        let page = self.fast_page(addr);
        let writes = self.b.ins().load(self.ptr, trusted(), page, offset_of!(FastPage, writes) as i32);
        self.b.ins().brif(writes, hit, &[], slow, &[]);
        self.b.switch_to_block(hit);
        let n = self.b.ins().load(types::I64, trusted(), writes, 0);
        let n = self.b.ins().iadd_imm_s(n, 1);
        self.b.ins().store(trusted(), n, writes, 0);
        let gens = self.b.ins().load(self.ptr, trusted(), page, offset_of!(FastPage, gen) as i32);
        self.b.ins().brif(gens, gen, &[], store, &[]);
        self.b.switch_to_block(gen);
        let k = self.b.ins().band_imm_u(addr, (FAST_PAGE - 1) as i64);
        let k = self.b.ins().ushr_imm_u(k, GEN_SHIFT as i64);
        let k = self.b.ins().uextend(self.ptr, k);
        let k = self.b.ins().ishl_imm_u(k, 2);
        let at = self.b.ins().iadd(gens, k);
        let g = self.b.ins().load(I32, trusted(), at, 0);
        let add = self.b.ins().load(I32, trusted(), page, offset_of!(FastPage, gen_add) as i32);
        let g = self.b.ins().iadd(g, add);
        self.b.ins().store(trusted(), g, at, 0);
        self.b.ins().jump(store, &[]);
        self.b.switch_to_block(store);
        let data = self.b.ins().load(self.ptr, trusted(), page, offset_of!(FastPage, data) as i32);
        let p = self.in_page(data, addr);
        match width {
            1 => self.b.ins().istore8(guest(), v, p, 0),
            2 => self.b.ins().istore16(guest(), v, p, 0),
            _ => self.b.ins().store(guest(), v, p, 0),
        };
        self.b.ins().jump(join, &[]);
        self.b.switch_to_block(slow);
        let f = match width {
            1 => (write8::<H> as WriteFn) as usize,
            2 => (write16::<H> as WriteFn) as usize,
            _ => (write32::<H> as WriteFn) as usize,
        };
        let ip = self.konst(ip);
        let (cpu, host, ctx, sig) = (self.cpu, self.host, self.ctx, self.sigs.write);
        self.call(sig, f, &[cpu, host, ctx, addr, v, ip]);
        self.b.ins().jump(join, &[]);
        self.b.switch_to_block(join);
    }

    /// Continue in a new block if `fail` is zero; else go to `to`.
    fn exit_if(&mut self, fail: Value, to: ClifBlock) {
        let go_on = self.b.create_block();
        self.b.ins().brif(fail, to, &[], go_on, &[]);
        self.b.switch_to_block(go_on);
    }

    /// The block: each instruction as the static recompiler's
    /// `program_module` runs it. What follows each instruction (count it,
    /// retire it, decide whether to go on) is inline while the machine is
    /// quiet (`Ctx::quiet`: count it, owe the retire), else one helper
    /// call, to keep the IR, and so the translation time, small.
    fn block(&mut self, insns: &[Insn]) {
        let done = self.b.create_block();
        for (k, i) in insns.iter().enumerate() {
            let pc = i.addr;
            let next = i.next();
            let op = lower(i);
            match op {
                Some(op) => self.op(&op, pc),
                None => {
                    let (cpu, host, ctx, sig) = (self.cpu, self.host, self.ctx, self.sigs.exec);
                    let insn = self.helper(i as *const Insn as usize);
                    let r =
                        self.call(sig, (exec::<H> as ExecFn) as usize, &[cpu, host, ctx, insn]).expect("exec returns");
                    let fault = self.fault;
                    self.exit_if(r, fault);
                }
            }
            let ends = !matches!(i.flow, Flow::Next | Flow::CondFault);
            let last = ends || k + 1 == insns.len();
            // After a store, a block from writable memory checks its own
            // code wasn't written.
            let check_code = self.ram.is_some() && !last && op.is_none_or(|o| o.stores());
            let full = self.b.create_block();
            let go_on = self.b.create_block();
            // A fallback (or a call, a return: anything that always calls
            // the machine) leaves `quiet` 0, so its step is the full one.
            if op.is_some_and(|o| !always_calls(&o)) {
                let quick = self.b.create_block();
                let q = self.b.ins().load(types::I64, trusted(), self.ctx, offset_of!(Ctx, quiet) as i32);
                self.b.ins().brif(q, quick, &[], full, &[]);
                self.b.switch_to_block(quick);
                let q = self.b.ins().iadd_imm_s(q, -1);
                self.b.ins().store(trusted(), q, self.ctx, offset_of!(Ctx, quiet) as i32);
                let n = self.b.ins().load(types::I64, trusted(), self.cpu, OFF_ICOUNT as i32);
                let n = self.b.ins().iadd_imm_s(n, 1);
                self.b.ins().store(trusted(), n, self.cpu, OFF_ICOUNT as i32);
                if last {
                    if !ends {
                        self.set_ip_const(next);
                    }
                    self.b.ins().jump(done, &[]);
                } else if let (true, Some((pages, seen))) = (check_code, self.ram) {
                    let changed = self.code_written(pages, seen);
                    let leave = self.b.create_block();
                    self.b.ins().brif(changed, leave, &[], go_on, &[]);
                    self.b.switch_to_block(leave);
                    self.set_ip_const(next);
                    self.b.ins().jump(done, &[]);
                } else {
                    self.b.ins().jump(go_on, &[]);
                }
            } else {
                self.b.ins().jump(full, &[]);
            }
            self.b.switch_to_block(full);
            // Some "sequential" instructions can still move the IP: an IAC
            // reinit, or an interrupt taken by `modpc`.
            let mut flags = if op.is_none() && !ends { STEP_MOVED } else { 0 };
            if ends {
                flags |= STEP_ENDS;
            }
            let mut flags = self.konst(flags);
            if let (true, Some((pages, seen))) = (check_code, self.ram) {
                // This block's own code may have been written: leave it,
                // and the dispatcher checks it again.
                let changed = self.code_written(pages, seen);
                let zero = self.konst(0);
                let changed = self.b.ins().icmp(IntCC::NotEqual, changed, zero);
                let leave = self.konst(STEP_LEAVE);
                let leave = self.b.ins().select(changed, leave, zero);
                flags = self.b.ins().bor(flags, leave);
            }
            let (cpu, host, ctx, sig) = (self.cpu, self.host, self.ctx, self.sigs.step);
            let n = self.konst(next);
            let f = if last { (step_last::<H> as StepFn) as usize } else { (step::<H> as StepFn) as usize };
            let stop = self.call(sig, f, &[cpu, host, ctx, n, flags]).expect("step returns");
            if last {
                self.b.ins().jump(done, &[]);
                break;
            }
            self.b.ins().brif(stop, done, &[], go_on, &[]);
            self.b.switch_to_block(go_on);
        }
        self.b.switch_to_block(done);
        let zero = self.konst(0);
        self.b.ins().return_(&[zero]);
        self.b.switch_to_block(self.fault);
        let one = self.konst(1);
        self.b.ins().return_(&[one]);
    }

    /// Nonzero if a counter of this block's pages moved since it was last
    /// checked.
    fn code_written(&mut self, pages: [usize; 2], seen: *const [u32; 2]) -> Value {
        let gens = self.b.ins().load(self.ptr, trusted(), self.ctx, offset_of!(Ctx, gens) as i32);
        let seen = self.helper(seen as usize);
        let mut any = None;
        for (k, &p) in pages.iter().enumerate() {
            let now = self.b.ins().load(I32, trusted(), gens, (4 * p) as i32);
            let then = self.b.ins().load(I32, trusted(), seen, 4 * k as i32);
            let d = self.b.ins().bxor(now, then);
            any = Some(match any {
                Some(a) => self.b.ins().bor(a, d),
                None => d,
            });
        }
        any.expect("two pages")
    }

    fn op(&mut self, op: &Op, pc: u32) {
        match *op {
            Op::Jump(t) => self.set_ip_const(t),
            Op::Call { target, next } => {
                let (cpu, host, ctx, sig) = (self.cpu, self.host, self.ctx, self.sigs.call);
                let (t, n, ip) = (self.konst(target), self.konst(next), self.konst(pc));
                self.call(sig, (call::<H> as CallFn) as usize, &[cpu, host, ctx, t, n, ip]);
            }
            Op::Link { target, next } => {
                let n = self.konst(next);
                self.set(30, n);
                self.set_ip_const(target);
            }
            Op::Branch { mask, target, next } => {
                let c = self.cond(mask);
                let (t, n) = (self.konst(target), self.konst(next));
                let ip = self.b.ins().select(c, t, n);
                self.set_ip(ip);
            }
            Op::Ret => {
                let (cpu, host, ctx, sig) = (self.cpu, self.host, self.ctx, self.sigs.ret);
                let ip = self.konst(pc);
                let r = self.call(sig, (ret::<H> as RetFn) as usize, &[cpu, host, ctx, ip]).expect("ret returns");
                let fault = self.fault;
                self.exit_if(r, fault);
            }
            Op::Test { mask, dst } => {
                let c = self.cond(mask);
                let v = self.b.ins().uextend(I32, c);
                self.set(dst, v);
            }
            Op::BitBranch { bit, src, set, target, next } => {
                let (b, a) = (self.src(src), self.src(bit));
                let m = self.bit(a);
                let x = self.b.ins().band(b, m);
                let is_set = self.b.ins().icmp_imm_u(IntCC::NotEqual, x, 0);
                let (two, zero) = (self.konst(2), self.konst(0));
                let cc = self.b.ins().select(is_set, two, zero);
                self.set_cc(cc);
                let (t, n) = (self.konst(target), self.konst(next));
                let ip = if set { self.b.ins().select(is_set, t, n) } else { self.b.ins().select(is_set, n, t) };
                self.set_ip(ip);
            }
            Op::CmpBranch { signed, a, b, mask, target, next } => {
                let (a, b) = (self.src(a), self.src(b));
                self.compare(signed, a, b);
                let c = self.cond(mask);
                let (t, n) = (self.konst(target), self.konst(next));
                let ip = self.b.ins().select(c, t, n);
                self.set_ip(ip);
            }
            Op::JumpInd(m) => {
                let a = self.ea(&m);
                self.set_ip(a);
            }
            Op::LinkInd { ea, dst, next } => {
                let a = self.ea(&ea);
                let n = self.konst(next);
                self.set(dst, n);
                self.set_ip(a);
            }
            Op::CallInd { ea, next } => {
                let a = self.ea(&ea);
                let (cpu, host, ctx, sig) = (self.cpu, self.host, self.ctx, self.sigs.call);
                let (n, ip) = (self.konst(next), self.konst(pc));
                self.call(sig, (call::<H> as CallFn) as usize, &[cpu, host, ctx, a, n, ip]);
            }
            Op::Lda { ea, dst } => {
                let a = self.ea(&ea);
                self.set(dst, a);
            }
            Op::LoadNarrow { width, signed, ea, dst } => {
                let a = self.ea(&ea);
                let w = if width == Width::Byte { 1 } else { 2 };
                let v = self.read(w, a, pc);
                let v = if signed {
                    let s = 32 - 8 * w as i64;
                    let up = self.b.ins().ishl_imm_u(v, s);
                    self.b.ins().sshr_imm_u(up, s)
                } else {
                    v
                };
                self.set(dst, v);
            }
            Op::StoreNarrow { width, ea, src } => {
                let a = self.ea(&ea);
                let v = self.get(src);
                let w = if width == Width::Byte { 1 } else { 2 };
                self.write(w, a, v, pc);
            }
            Op::Load { n, ea, dst } => {
                let a = self.ea(&ea);
                for k in 0..n {
                    let at = self.b.ins().iadd_imm_s(a, 4 * k as i64);
                    let v = self.read(4, at, pc);
                    self.set(dst.wrapping_add(k), v);
                }
            }
            Op::Store { n, ea, src } => {
                let a = self.ea(&ea);
                for k in 0..n {
                    let at = self.b.ins().iadd_imm_s(a, 4 * k as i64);
                    let v = self.get(src.wrapping_add(k));
                    self.write(4, at, v, pc);
                }
            }
            Op::Alu { op, a, b, dst } => {
                let (s1, s2) = (self.src(a), self.src(b));
                let v = match op {
                    Alu::NotBit => {
                        let m = self.bit(s1);
                        self.b.ins().bxor(s2, m)
                    }
                    Alu::And => self.b.ins().band(s2, s1),
                    Alu::AndNot => self.b.ins().band_not(s2, s1),
                    Alu::SetBit => {
                        let m = self.bit(s1);
                        self.b.ins().bor(s2, m)
                    }
                    Alu::NotAnd => self.b.ins().band_not(s1, s2),
                    Alu::Xor => self.b.ins().bxor(s2, s1),
                    Alu::Or => self.b.ins().bor(s2, s1),
                    Alu::Nor => {
                        let v = self.b.ins().bor(s2, s1);
                        self.b.ins().bnot(v)
                    }
                    Alu::Xnor => {
                        let v = self.b.ins().bxor(s2, s1);
                        self.b.ins().bnot(v)
                    }
                    Alu::OrNot => self.b.ins().bor_not(s2, s1),
                    Alu::ClrBit => {
                        let m = self.bit(s1);
                        self.b.ins().band_not(s2, m)
                    }
                    Alu::NotOr => self.b.ins().bor_not(s1, s2),
                    Alu::Nand => {
                        let v = self.b.ins().band(s2, s1);
                        self.b.ins().bnot(v)
                    }
                    Alu::Add => self.b.ins().iadd(s2, s1),
                    Alu::Sub => self.b.ins().isub(s2, s1),
                    Alu::Shro | Alu::Shl => {
                        let shifted =
                            if op == Alu::Shro { self.b.ins().ushr(s2, s1) } else { self.b.ins().ishl(s2, s1) };
                        let big = self.b.ins().icmp_imm_u(IntCC::UnsignedGreaterThanOrEqual, s1, 32);
                        let zero = self.konst(0);
                        self.b.ins().select(big, zero, shifted)
                    }
                    Alu::Shri => {
                        let max = self.konst(31);
                        let n = self.b.ins().umin(s1, max);
                        self.b.ins().sshr(s2, n)
                    }
                    Alu::Rotate => self.b.ins().rotl(s2, s1),
                    Alu::Mul => self.b.ins().imul(s2, s1),
                };
                self.set(dst, v);
            }
            Op::Not { a, dst } => {
                let v = self.src(a);
                let v = self.b.ins().bnot(v);
                self.set(dst, v);
            }
            Op::Compare { signed, a, b } => {
                let (a, b) = (self.src(a), self.src(b));
                self.compare(signed, a, b);
            }
            Op::ChkBit { bit, src } => {
                let (b, a) = (self.src(src), self.src(bit));
                let m = self.bit(a);
                let x = self.b.ins().band(b, m);
                let is_set = self.b.ins().icmp_imm_u(IntCC::NotEqual, x, 0);
                let (two, zero) = (self.konst(2), self.konst(0));
                let cc = self.b.ins().select(is_set, two, zero);
                self.set_cc(cc);
            }
            Op::Mov { n, src, dst } => match src {
                Src::Reg(s) => {
                    let vals: Vec<Value> = (0..n).map(|k| self.get(s.wrapping_add(k))).collect();
                    for (k, v) in vals.into_iter().enumerate() {
                        self.set(dst.wrapping_add(k as u8), v);
                    }
                }
                Src::Lit(v) => {
                    let v = self.konst(v);
                    self.set(dst, v);
                    for k in 1..n {
                        let z = self.konst(0);
                        self.set(dst.wrapping_add(k), z);
                    }
                }
            },
        }
    }
}

// The helpers blocks call: the machine's own methods, for `H`.
//
// SAFETY (all of them): called only by blocks `Jit::<H>::run` started, with
// the CPU, host and context it passed in, which are live and distinct.

type ReadFn = unsafe extern "C" fn(*mut Cpu, *mut u8, *mut Ctx, u32, u32) -> u32;
type WriteFn = unsafe extern "C" fn(*mut Cpu, *mut u8, *mut Ctx, u32, u32, u32);
type CallFn = unsafe extern "C" fn(*mut Cpu, *mut u8, *mut Ctx, u32, u32, u32);
type RetFn = unsafe extern "C" fn(*mut Cpu, *mut u8, *mut Ctx, u32) -> u32;
type ExecFn = unsafe extern "C" fn(*mut Cpu, *mut u8, *mut Ctx, *const Insn) -> u32;
type StepFn = unsafe extern "C" fn(*mut Cpu, *mut u8, *mut Ctx, u32, u32) -> u32;

/// The machine as the helpers that run the interpreter's code (`call`,
/// `ret`, the fallbacks) give it to the CPU: plain memory through the
/// block's [`FastPage`]s, as the inline paths reach it (so a call's frame
/// is written without the bus's lookups), the rest through the machine.
struct Plain<'a, H> {
    h: &'a mut H,
    fast: *const FastPage,
}

impl<H: JitHost> Plain<'_, H> {
    /// The page of `a` if an access of `n` bytes there is plain (aligned,
    /// below [`FAST_SPAN`], an entry in the table).
    #[inline(always)]
    fn page(&self, a: u32, n: u32) -> Option<&FastPage> {
        if a & (!(FAST_SPAN - 1) | (n - 1)) != 0 {
            return None;
        }
        // SAFETY: the table has FAST_PAGES entries (`JitHost::fast_pages`).
        Some(unsafe { &*self.fast.add((a >> FAST_SHIFT) as usize) })
    }

    #[inline(always)]
    fn read(&self, a: u32, n: u32) -> Option<u32> {
        let e = self.page(a, n)?;
        if e.data.is_null() {
            return None;
        }
        // SAFETY: a page with `data` has FAST_PAGE bytes there, and the
        // access is within it.
        let p = unsafe { e.data.add((a & (FAST_PAGE - 1)) as usize) };
        let mut v = 0u32;
        for i in 0..n as usize {
            v |= (unsafe { *p.add(i) } as u32) << (8 * i);
        }
        Some(v)
    }

    #[inline(always)]
    fn write(&mut self, a: u32, v: u32, n: u32) -> bool {
        let Some(e) = self.page(a, n) else { return false };
        if e.writes.is_null() {
            return false;
        }
        // SAFETY: a page with `writes` has `data`, and `gen` (if set)
        // has a counter for each GEN_PAGE of it.
        unsafe {
            *e.writes += 1;
            let off = (a & (FAST_PAGE - 1)) as usize;
            if !e.gen.is_null() {
                let g = e.gen.add(off >> GEN_SHIFT);
                *g = (*g).wrapping_add(e.gen_add);
            }
            for i in 0..n as usize {
                *e.data.add(off + i) = (v >> (8 * i)) as u8;
            }
        }
        true
    }
}

impl<H: JitHost> i960::cpu::Bus for Plain<'_, H> {
    fn read8(&mut self, a: u32) -> u8 {
        self.read(a, 1).map_or_else(|| self.h.read8(a), |v| v as u8)
    }
    fn read16(&mut self, a: u32) -> u16 {
        self.read(a, 2).map_or_else(|| self.h.read16(a), |v| v as u16)
    }
    fn read32(&mut self, a: u32) -> u32 {
        self.read(a, 4).unwrap_or_else(|| self.h.read32(a))
    }
    fn write8(&mut self, a: u32, v: u8) {
        if !self.write(a, v as u32, 1) {
            self.h.write8(a, v)
        }
    }
    fn write16(&mut self, a: u32, v: u16) {
        if !self.write(a, v as u32, 2) {
            self.h.write16(a, v)
        }
    }
    fn write32(&mut self, a: u32, v: u32) {
        if !self.write(a, v, 4) {
            self.h.write32(a, v)
        }
    }
    fn fetch(&self, a: u32) -> Option<u32> {
        self.h.fetch(a)
    }
    fn is_rom(&self, a: u32) -> bool {
        self.h.is_rom(a)
    }
}

/// An op whose code always calls a helper (which leaves `quiet` 0): its
/// step needs no inline path.
fn always_calls(op: &Op) -> bool {
    matches!(op, Op::Call { .. } | Op::CallInd { .. } | Op::Ret)
}

/// Before anything reaches the machine: make the retires the block owes
/// it (the quiet ones it counted inline), and grant no more (what is about
/// to happen may end the quiet).
#[inline(always)]
fn settle<H: JitHost>(c: &mut Cpu, h: &mut H, ctx: &mut Ctx) {
    let owed = ctx.granted - ctx.quiet;
    if owed != 0 {
        h.retire_quiet(c, owed);
    }
    ctx.granted = 0;
    ctx.quiet = 0;
}

/// Unpack a helper's arguments, settling first.
///
/// SAFETY: as for the helpers.
#[inline(always)]
unsafe fn enter<'a, H: JitHost>(c: *mut Cpu, h: *mut u8, ctx: *mut Ctx) -> (&'a mut Cpu, &'a mut H, &'a mut Ctx) {
    let (c, h, ctx) = unsafe { (&mut *c, &mut *(h as *mut H), &mut *ctx) };
    settle(c, h, ctx);
    (c, h, ctx)
}

unsafe extern "C" fn read8<H: JitHost>(c: *mut Cpu, h: *mut u8, ctx: *mut Ctx, a: u32, ip: u32) -> u32 {
    let (_, h, _) = unsafe { enter::<H>(c, h, ctx) };
    h.at(ip);
    h.read8(a) as u32
}

unsafe extern "C" fn read16<H: JitHost>(c: *mut Cpu, h: *mut u8, ctx: *mut Ctx, a: u32, ip: u32) -> u32 {
    let (_, h, _) = unsafe { enter::<H>(c, h, ctx) };
    h.at(ip);
    h.read16(a) as u32
}

unsafe extern "C" fn read32<H: JitHost>(c: *mut Cpu, h: *mut u8, ctx: *mut Ctx, a: u32, ip: u32) -> u32 {
    let (_, h, _) = unsafe { enter::<H>(c, h, ctx) };
    h.at(ip);
    h.read32(a)
}

unsafe extern "C" fn write8<H: JitHost>(c: *mut Cpu, h: *mut u8, ctx: *mut Ctx, a: u32, v: u32, ip: u32) {
    let (_, h, _) = unsafe { enter::<H>(c, h, ctx) };
    h.at(ip);
    h.write8(a, v as u8)
}

unsafe extern "C" fn write16<H: JitHost>(c: *mut Cpu, h: *mut u8, ctx: *mut Ctx, a: u32, v: u32, ip: u32) {
    let (_, h, _) = unsafe { enter::<H>(c, h, ctx) };
    h.at(ip);
    h.write16(a, v as u16)
}

unsafe extern "C" fn write32<H: JitHost>(c: *mut Cpu, h: *mut u8, ctx: *mut Ctx, a: u32, v: u32, ip: u32) {
    let (_, h, _) = unsafe { enter::<H>(c, h, ctx) };
    h.at(ip);
    h.write32(a, v)
}

unsafe extern "C" fn call<H: JitHost>(c: *mut Cpu, h: *mut u8, ctx: *mut Ctx, target: u32, next: u32, ip: u32) {
    let (c, h, ctx) = unsafe { enter::<H>(c, h, ctx) };
    h.at(ip);
    c.call(&mut Plain { h, fast: ctx.fast }, target, next)
}

unsafe extern "C" fn ret<H: JitHost>(c: *mut Cpu, h: *mut u8, ctx: *mut Ctx, ip: u32) -> u32 {
    let (c, h, ctx) = unsafe { enter::<H>(c, h, ctx) };
    h.at(ip);
    match c.ret(&mut Plain { h, fast: ctx.fast }) {
        Ok(()) => 0,
        Err(kind) => {
            ctx.fault = Some(Stop::Fault { kind, ip });
            1
        }
    }
}

unsafe extern "C" fn exec<H: JitHost>(c: *mut Cpu, h: *mut u8, ctx: *mut Ctx, insn: *const Insn) -> u32 {
    let (c, h, ctx) = unsafe { enter::<H>(c, h, ctx) };
    let insn = unsafe { &*insn };
    h.at(insn.addr);
    match c.exec(&mut Plain { h, fast: ctx.fast }, &insn.view()) {
        Ok(()) => 0,
        Err(kind) => {
            ctx.fault = Some(Stop::Fault { kind, ip: insn.addr });
            1
        }
    }
}

/// `step` flags: the instruction was left to the interpreter and doesn't
/// end the block (it may still have moved the IP); it ends the block (it
/// set the IP); leave the block after it whatever else happens.
const STEP_MOVED: u32 = 1;
const STEP_ENDS: u32 = 2;
const STEP_LEAVE: u32 = 4;

/// After an instruction in the middle of a block, when it isn't quiet:
/// count and retire it. Nonzero to leave the block, with the IP where it
/// goes on (`next`, unless the instruction moved it). Else how many of the
/// retires after it are quiet, for the block to count inline.
unsafe extern "C" fn step<H: JitHost>(c: *mut Cpu, h: *mut u8, ctx: *mut Ctx, next: u32, flags: u32) -> u32 {
    let (c, h, ctx) = unsafe { enter::<H>(c, h, ctx) };
    c.icount += 1;
    if flags & STEP_MOVED != 0 && c.ip != next {
        h.retire(c);
        return 1;
    }
    if !h.retire(c) || c.interrupt_due() || flags & STEP_LEAVE != 0 {
        c.ip = next;
        return 1;
    }
    let quiet = h.quiet();
    ctx.quiet = quiet;
    ctx.granted = quiet;
    0
}

/// After a block's last instruction, when it isn't quiet: count and retire
/// it, and put the IP at `next` unless it set it (a control transfer) or
/// moved it.
unsafe extern "C" fn step_last<H: JitHost>(c: *mut Cpu, h: *mut u8, ctx: *mut Ctx, next: u32, flags: u32) -> u32 {
    let (c, h, _) = unsafe { enter::<H>(c, h, ctx) };
    c.icount += 1;
    h.retire(c);
    if flags & STEP_ENDS == 0 && (flags & STEP_MOVED == 0 || c.ip == next) {
        c.ip = next;
    }
    1
}

#[cfg(test)]
mod fast_tests;
#[cfg(test)]
mod tests;
