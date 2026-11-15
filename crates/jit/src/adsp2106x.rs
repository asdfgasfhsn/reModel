//! The ADSP-21062 (Model 2B's copro and geometrizer SHARCs) on Cranelift
//! (docs/jit.md, "The SHARC"): runs of straight-line words of the
//! uploaded program translated to machine code as regions, each entered
//! at any of its words and run as one block of the interpreter's
//! (`Sharc::block_start`/`block_exec`/`block_end`), exactly as the static
//! recompiler's region functions run them (`codegen::program_module`).
//!
//! [`SharcJit::run`] has `Sharc::run_bus`'s contract: instructions until
//! the bus's `after_step` says to stop or one doesn't run (IDLE, a
//! fault). A word that can't be in a block (`codegen::in_block`: a DO, a
//! stack operation, an unconditional branch, a system register operand),
//! a block the core can't run now (`block_start`: interrupts or the timer
//! on, a fetch reaching a loop end, the bus's budget) and a word a block
//! leaves before (a taken branch, an IOP register access) run as the
//! interpreter's single step, so a run is step for step an interpreted
//! one.
//!
//! The words of each translated region are watched (`Sharc::watch_words`):
//! a write to one drops its region, which is translated again from the new
//! words when it next runs.

use crate::copro::{block_signature, install, module, Background, CoproStats, OwnedModule};
use crate::hotlist::{Chip, HotList};
use crate::pool::{Compiled, Urgency};
use ::adsp2106x::codegen::{self, in_block};
use ::adsp2106x::cpu::{code_hash, program_image, word_index, BlockEnd, Bus, Fault, Sharc, Step};
use ::adsp2106x::{decode, Insn};
use std::collections::BTreeMap;
use std::mem::ManuallyDrop;
use std::sync::Arc;
use std::time::Instant;

/// Cranelift's optimiser for the regions: on, as each word is many IR
/// instructions and a region runs many times (docs/jit.md, "The SHARC").
const OPTIMISE: bool = true;

/// Words per region at most (the recompiler's `REGION_WORDS`).
pub const MAX_REGION: u32 = 128;

/// How a region's run ended (`Ctx::kind`): before a word (0, or 2 for a
/// word that declined), after one, or on a fault.
const BEFORE: u32 = 0;
const LAST: u32 = 1;
const BAIL: u32 = 2;
const FAULT: u32 = 3;

/// Passed to every region: where to enter it and the block's limits; on
/// return how it ended.
#[repr(C)]
struct Ctx {
    /// The word to start at, and the one the block stops before at the
    /// latest (`Block::stop`).
    at: u32,
    stop: u32,
    /// The block's `steps` before it, less `at` (a word's step count once
    /// it has run is this plus its address plus one).
    base: u64,
    /// STKY before the last word ran.
    st: u32,
    /// How it ended, and at which word.
    kind: u32,
    end: u32,
    fault: Option<Fault>,
    /// Words run on the interpreter's step (`h_exec`).
    helper: u64,
    /// The IOP registers and internal memory's two blocks (their `Vec`s
    /// don't move while a block runs): what an inline read looks at.
    iop: *const u32,
    mem: [*const u16; 2],
}

/// A translated region: `(sharc, bus, ctx)`, `bus` pointing at a `&mut
/// dyn Bus`.
type RegionFn = unsafe extern "C" fn(*mut Sharc, *mut u8, *mut Ctx);

struct Region {
    start: u32,
    /// The words as translated (the IR points at them).
    words: Arc<[Insn]>,
    code: Option<RegionFn>,
    /// Dropped (overwritten, or its translation failed): its table
    /// entries are cold again.
    dead: bool,
}

impl Region {
    fn end(&self) -> u32 {
        self.start + self.words.len() as u32
    }
}

/// Table entries, per internal 48-bit word (`word_index`): not planned,
/// no region (a word that can't be in a block), else region + `FIRST`
/// (queued or translated).
const COLD: u32 = 0;
const NEVER: u32 = 1;
const FIRST: u32 = 2;

/// The JIT for the program a SHARC holds: found again by the hash of its
/// code (`code_hash` over what `codegen::discover` reaches), so a state
/// load or another upload of the same code keeps the translations.
pub struct SharcJit {
    module: OwnedModule,
    table: Box<[u32]>,
    regions: Vec<Region>,
    key: Option<u64>,
    /// The recompiler's regions for the program (`codegen::regions` over
    /// the discovered code): start and length. A word reached outside
    /// them starts a region of its own.
    planned: BTreeMap<u32, u32>,
    /// The core's watch on the translated words is to be set up again
    /// (its state was copied from another core's).
    rewatch: bool,
    stats: CoproStats,
    jobs: Background,
    list: Option<Arc<HotList>>,
}

impl SharcJit {
    pub fn new() -> Result<Self, String> {
        Ok(SharcJit {
            module: OwnedModule(ManuallyDrop::new(module(OPTIMISE)?)),
            table: vec![COLD; 2 * ::adsp2106x::mem::WORDS48 as usize].into_boxed_slice(),
            regions: Vec::new(),
            key: None,
            planned: BTreeMap::new(),
            rewatch: false,
            stats: CoproStats::default(),
            jobs: {
                let mut b = Background::new();
                b.optimise = OPTIMISE;
                b
            },
            list: None,
        })
    }

    /// Translate on the background workers (the default: a region runs on
    /// the interpreter until its code is ready), or at once.
    pub fn set_background(&mut self, on: bool) {
        self.jobs.on = on;
    }

    /// Note translated regions in `list`, and translate the ones it lists
    /// for each program as the core gets it.
    pub fn set_hot_list(&mut self, list: Arc<HotList>) {
        self.list = Some(list);
        self.rewatch = true;
    }

    pub fn stats(&self) -> CoproStats {
        self.stats
    }

    /// The core's state was replaced from another core's (run-ahead's
    /// copy): watch the translated words again before the next run.
    pub fn rewatch(&mut self) {
        self.rewatch = true;
    }

    /// Wait for the regions being translated, and install them (tests).
    pub fn wait(&mut self) {
        self.install(true, 0);
    }

    /// [`Sharc::run_bus`] on translated code.
    pub fn run<B: Bus>(&mut self, s: &mut Sharc, ext: &mut B) -> Step {
        self.follow(s);
        loop {
            let r = self.one(s, ext);
            if r != Step::Ran || !ext.after_step(s) {
                return r;
            }
        }
    }

    /// One block or single step, as the recompiled `run`'s loop body.
    #[inline]
    fn one<B: Bus>(&mut self, s: &mut Sharc, ext: &mut B) -> Step {
        if s.code_dirty() {
            self.overwritten(s);
        }
        let a = s.compiled_pc();
        if let Some(r) = self.region_at(s, a) {
            let region = &self.regions[r as usize];
            let budget = ext.block_budget(s);
            if let Some(b) = s.block_start(a, region.end(), budget) {
                let code = region.code.expect("region_at found code");
                let mut ctx = Ctx {
                    at: a,
                    stop: b.stop,
                    base: b.steps.wrapping_sub(a as u64),
                    st: 0,
                    kind: BEFORE,
                    end: a,
                    fault: None,
                    helper: 0,
                    iop: s.iop.as_ptr(),
                    mem: [s.mem.cols[0].as_ptr(), s.mem.cols[1].as_ptr()],
                };
                let mut d: &mut dyn Bus = ext;
                // SAFETY: made by `translate` for the words at `region`,
                // which hold what it was translated from (they are watched;
                // `overwritten` drops a region before it can run stale).
                unsafe { code(s, &mut d as *mut &mut dyn Bus as *mut u8, &mut ctx) };
                self.stats.runs += 1;
                let end = match (ctx.kind, ctx.fault) {
                    (LAST, _) => BlockEnd::Last(ctx.end),
                    (FAULT, Some(f)) => BlockEnd::Fault(ctx.end, f),
                    _ => BlockEnd::Before(ctx.end),
                };
                let r = s.block_end(&b, end, ctx.st);
                self.stats.block_words += s.steps - b.steps;
                self.stats.helper_words += ctx.helper;
                if let Some(r) = r {
                    return r;
                }
            }
        }
        self.stats.interpreted += 1;
        s.step(ext)
    }

    /// The region with code to run at `a`, if there is one; queues one
    /// for a word not planned yet, and installs a finished translation.
    #[inline]
    fn region_at(&mut self, s: &mut Sharc, a: u32) -> Option<u32> {
        let k = word_index(a)?;
        let r = match self.table[k] {
            COLD => return self.cold(s, a, k),
            NEVER => return None,
            e => e - FIRST,
        };
        if self.regions[r as usize].code.is_some() {
            return Some(r);
        }
        self.waiting(r)
    }

    #[cold]
    fn cold(&mut self, s: &mut Sharc, a: u32, k: usize) -> Option<u32> {
        self.queue(s, a, Urgency::Now);
        match self.table[k] {
            e if e >= FIRST => self.waiting(e - FIRST),
            _ => None,
        }
    }

    /// Region `r` is queued: install it if its translation is done.
    #[cold]
    fn waiting(&mut self, r: u32) -> Option<u32> {
        let start = self.regions[r as usize].start;
        self.jobs.reached(start);
        self.install(false, start);
        self.regions[r as usize].code.is_some().then_some(r)
    }

    /// Take up the program the core holds: on the first run, after a new
    /// upload or a state load (the core watches nothing then), and after
    /// a copy of another core's state.
    fn follow(&mut self, s: &mut Sharc) {
        if self.key.is_some() && s.code_watched() && !self.rewatch {
            return;
        }
        self.rewatch = false;
        let image = program_image(s);
        let d = codegen::discover(&image);
        let words: Vec<u32> = d.code.iter().copied().collect();
        let key = code_hash(&image, &words);
        s.watch_code(&[], &[]);
        s.watch_words([]);
        if self.key != Some(key) {
            self.key = Some(key);
            self.table.fill(COLD);
            self.regions.clear();
            self.jobs.clear();
            self.planned = codegen::regions(&image, &words).into_iter().collect();
            if let Some(list) = &self.list {
                for a in list.get(Chip::Adsp2106x, key) {
                    if word_index(a).is_some_and(|k| self.table[k] == COLD) {
                        self.queue(s, a, Urgency::Later);
                    }
                }
            }
            return;
        }
        // The same code: keep the regions whose words are still there,
        // and watch them again.
        for r in 0..self.regions.len() {
            if self.regions[r].dead {
                continue;
            }
            let same = self.regions[r].words.iter().all(|i| s.mem.read48(i.addr) == i.word);
            if same {
                s.watch_words(self.regions[r].words.iter().map(|i| i.addr));
            } else {
                self.drop_region(r);
            }
        }
    }

    /// Drop the regions holding words the program wrote.
    #[cold]
    fn overwritten(&mut self, s: &mut Sharc) {
        for a in s.take_overwritten() {
            let Some(k) = word_index(a) else { continue };
            if self.table[k] >= FIRST {
                self.drop_region((self.table[k] - FIRST) as usize);
            }
            self.table[k] = COLD;
        }
    }

    fn drop_region(&mut self, r: usize) {
        let region = &mut self.regions[r];
        region.dead = true;
        region.code = None;
        for i in region.words.iter() {
            if let Some(k) = word_index(i.addr) {
                self.table[k] = COLD;
            }
        }
    }

    /// The words of the region to run from `a`: the planned region holding
    /// it (from as far back as its words are still free and can be in a
    /// block), else from `a` on; up to a word that can't be in a block,
    /// one already in a region, the end of a 4,096-word group or
    /// [`MAX_REGION`] words.
    fn plan(&self, s: &Sharc, a: u32) -> Vec<Insn> {
        let free = |w: u32| word_index(w).is_some_and(|k| self.table[k] == COLD);
        let insn = |w: u32| decode(w, s.mem.read48(w));
        if !free(a) || !in_block(&insn(a)) {
            return Vec::new();
        }
        let (lo, hi) = match self.planned.range(..=a).next_back() {
            Some((&p, &n)) if a < p + n => (p, p + n),
            _ => (a, a + MAX_REGION),
        };
        let mut start = a;
        while start > lo && start & 0xfff != 0 && free(start - 1) && in_block(&insn(start - 1)) {
            start -= 1;
        }
        let mut v = Vec::new();
        let mut w = start;
        loop {
            v.push(insn(w));
            w += 1;
            if w >= hi || w & 0xfff == 0 || v.len() as u32 >= MAX_REGION || !free(w) || !in_block(&insn(w)) {
                break;
            }
        }
        v
    }

    /// Queue the region to run from `a`, or mark `a` as having none.
    fn queue(&mut self, s: &mut Sharc, a: u32, urgency: Urgency) {
        let words: Arc<[Insn]> = self.plan(s, a).into();
        if words.is_empty() {
            if urgency == Urgency::Now {
                if let Some(k) = word_index(a) {
                    self.table[k] = NEVER;
                }
            }
            return;
        }
        let r = self.regions.len() as u32;
        let start = words[0].addr;
        for i in words.iter() {
            self.table[word_index(i.addr).expect("planned words are internal")] = r + FIRST;
        }
        s.watch_words(words.iter().map(|i| i.addr));
        self.regions.push(Region { start, words: words.clone(), code: None, dead: false });
        let end = start + words.len() as u32;
        self.jobs.queue(self.key.unwrap_or(0), start, end, urgency, None, move |b, ptr| {
            emit::Emit::new(b, ptr).region(&words);
        });
    }

    /// Install the finished translation of the region at `want` (`wait`:
    /// every one queued), throwing away any for another program or a
    /// region since dropped.
    fn install(&mut self, wait: bool, want: u32) {
        let done = self.jobs.finished(wait, want);
        if done.is_empty() {
            return;
        }
        let t0 = Instant::now();
        let mut keep = Vec::new();
        for d in done {
            let r = word_index(d.start).map(|k| self.table[k]).filter(|&e| e >= FIRST).map(|e| (e - FIRST) as usize);
            let ours = Some(d.key) == self.key
                && r.is_some_and(|r| {
                    let g = &self.regions[r];
                    g.start == d.start && g.end() == d.end && !g.dead && g.code.is_none()
                });
            match d.code {
                Ok(c) if ours => keep.push((r.expect("ours"), c)),
                Err(e) if ours => {
                    // Its words run on the interpreter from now on.
                    eprintln!("jit: ADSP-21062 region at {:#x}: {e}", d.start);
                    let r = r.expect("ours");
                    self.drop_region(r);
                    for i in self.regions[r].words.clone().iter() {
                        if let Some(k) = word_index(i.addr) {
                            self.table[k] = NEVER;
                        }
                    }
                }
                _ => self.stats.discarded += 1,
            }
        }
        if let (Some(list), Some(key)) = (&self.list, self.key) {
            list.add(Chip::Adsp2106x, key, keep.iter().map(|&(r, _)| self.regions[r].start));
        }
        let sig = block_signature(&self.module.0, None);
        let compiled: Vec<&Compiled> = keep.iter().map(|(_, c)| c).collect();
        let entries = install(&mut self.module.0, &sig, &compiled);
        for ((r, c), entry) in keep.iter().zip(entries) {
            self.stats.time += c.time;
            self.stats.slowest = self.stats.slowest.max(c.time);
            let Some((ptr, size)) = entry else {
                self.drop_region(*r);
                continue;
            };
            // SAFETY: made with the signature of `RegionFn` (the C convention).
            let code = unsafe { std::mem::transmute::<*const u8, RegionFn>(ptr) };
            let g = &mut self.regions[*r];
            g.code = Some(code);
            self.stats.blocks += 1;
            self.stats.words += g.words.len() as u64;
            self.stats.fallbacks += g.words.iter().filter(|i| !emit::lowered(i)).count() as u64;
            self.stats.code_bytes += size as u64;
        }
        self.stats.time += t0.elapsed();
    }
}

/// What kind of word `insn` is, for profiles of what the programs run:
/// its instruction type and compute operation.
pub fn kind(insn: &Insn) -> String {
    use ::adsp2106x::{Compute, MulOp, Op};
    let mul = |m: &MulOp| match m {
        MulOp::FMul => "fmul",
        MulOp::Mul { .. } => "mul",
        MulOp::Sat { .. } => "sat",
        MulOp::Rnd { .. } => "rnd",
        MulOp::Clear(_) => "clrmr",
    };
    let c = |c: &Compute| match c {
        Compute::None => String::new(),
        Compute::Alu { op, .. } => format!(" {op:?}"),
        Compute::AddSub { float, .. } => format!(" {}addsub", if *float { "f" } else { "" }),
        Compute::Mul { op, .. } => format!(" {}", mul(op)),
        Compute::Shift { op, .. } => format!(" {op:?}"),
        Compute::MrMove { .. } => " mrmove".into(),
        Compute::Multi { mul: m, alu, .. } => format!(" {}+{alu:?}", mul(m)),
        Compute::MultiAddSub { float, .. } => format!(" {}mul+addsub", if *float { "f" } else { "" }),
        Compute::Unknown(_) => " unknown".into(),
    };
    let ureg = |r: u8| match r >> 4 {
        0 => "r",
        1..=4 => "dag",
        0xd => "px",
        _ => "sys",
    };
    match &insn.op {
        Op::Dual { compute, .. } => format!("dual{}", c(compute)),
        Op::Compute { compute, .. } => format!("compute{}", c(compute)),
        Op::Transfer { compute, mv, .. } => {
            format!("transfer {} {}{}", if mv.write { "store" } else { "load" }, ureg(mv.reg), c(compute))
        }
        Op::RegMove { compute, src, dst, .. } => format!("regmove {}->{}{}", ureg(*src), ureg(*dst), c(compute)),
        Op::ShiftImm { op, mv, .. } => format!("shiftimm {op:?}{}", if mv.is_some() { " +move" } else { "" }),
        Op::Modify { compute, .. } => format!("modify{}", c(compute)),
        Op::Branch { call, cond, delayed, compute, .. } => format!(
            "{}{}{}{}",
            if *call { "call" } else { "jump" },
            if *cond == ::adsp2106x::TRUE { "" } else { " if" },
            if *delayed { " (db)" } else { "" },
            c(compute)
        ),
        Op::JumpElseMove { compute, .. } => format!("jump-else-move{}", c(compute)),
        Op::Return { cond, .. } => format!("return{}", if *cond == ::adsp2106x::TRUE { "" } else { " if" }),
        Op::Do { count, .. } => format!("do {}", if count.is_some() { "lce" } else { "cond" }),
        Op::Direct { write, reg, .. } => format!("direct {} {}", if *write { "store" } else { "load" }, ureg(*reg)),
        Op::IndirectImm { write, reg, .. } => {
            format!("indirect-imm {} {}", if *write { "store" } else { "load" }, ureg(*reg))
        }
        Op::StoreImm { .. } => "store-imm".into(),
        Op::LoadImm { reg, .. } => format!("load-imm {}", ureg(*reg)),
        Op::Bit { .. } => "bit".into(),
        Op::ModifyImm { .. } => "modify-imm".into(),
        Op::Stacks(_) => "stacks".into(),
        Op::Nop => "nop".into(),
        other => format!("{other:?}").chars().take_while(|c| c.is_alphanumeric()).collect(),
    }
}

/// The words a core has executed, by [`kind`] and by whether the JIT
/// could hold them in a block: (kind, can be in a block, executions),
/// most executed first.
pub fn profile(s: &Sharc) -> Vec<(String, bool, u64)> {
    let mut m: BTreeMap<(String, bool), u64> = BTreeMap::new();
    for a in (0x2_0000..0x2_5000u32).chain(0x2_8000..0x2_d000) {
        let n = s.count_at(a).unwrap_or(0);
        if n > 0 {
            let i = decode(a, s.mem.read48(a));
            *m.entry((kind(&i), in_block(&i))).or_default() += n;
        }
    }
    let mut v: Vec<_> = m.into_iter().map(|((k, b), n)| (k, b, n)).collect();
    v.sort_by_key(|e| std::cmp::Reverse(e.2));
    v
}

mod emit;

#[cfg(test)]
mod tests;
