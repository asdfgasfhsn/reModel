//! The MB86235 JIT's part of the board (docs/jit.md, "Copro JITs"): the
//! JIT for the program a TGP holds, and for `make jit-copro-lockstep` the
//! interpreter beside it, run first on what the JIT will find (its writes
//! kept to itself) and compared with it after every run. Native builds
//! only.

use jit::mb86235::{differs, Mb86235Jit};
use mb86235::cpu::{External, Step, Tgp};

/// Which external addresses are RAM, as a word index (two addresses with
/// the same index are the same word): what a write to them changes.
pub(crate) type Key<'k> = &'k dyn Fn(u32) -> Option<u32>;

/// A bus that keeps the writes made through it to itself, over `inner`.
struct Overlay<'k, E> {
    inner: E,
    key: Key<'k>,
    writes: Vec<(u32, u32)>,
}

impl<E: External> External for Overlay<'_, E> {
    fn read(&mut self, a: u32) -> u32 {
        if let Some(k) = (self.key)(a) {
            if let Some(&(_, v)) = self.writes.iter().rev().find(|&&(w, _)| (self.key)(w) == Some(k)) {
                return v;
            }
        }
        self.inner.read(a)
    }
    fn write(&mut self, a: u32, v: u32) {
        self.writes.push((a, v));
    }
}

/// `inner`, its writes listed.
struct Listed<E> {
    inner: E,
    writes: Vec<(u32, u32)>,
}

impl<E: External> External for Listed<E> {
    fn read(&mut self, a: u32) -> u32 {
        self.inner.read(a)
    }
    fn write(&mut self, a: u32, v: u32) {
        self.writes.push((a, v));
        self.inner.write(a, v)
    }
}

/// A TGP's JIT: on (`code`), and in a lockstep (`check`), the interpreter
/// kept beside it.
#[derive(Default)]
pub(crate) struct TgpJit {
    pub code: Option<Box<Mb86235Jit>>,
    pub check: bool,
    shadow: Option<Box<Tgp>>,
    pub checked: u64,
    /// The TGP's name in a difference's report.
    pub name: &'static str,
    /// Words per block at most.
    max_block: usize,
}

impl TgpJit {
    pub fn new(name: &'static str, max_block: usize) -> Self {
        TgpJit { name, max_block, ..TgpJit::default() }
    }

    /// Turn it on or off, as `c` configures the JIT, with the hot-block
    /// list `list` (on fails where Cranelift has no back end).
    pub fn set(
        &mut self,
        on: bool,
        c: &jit::Config,
        list: Option<&std::sync::Arc<jit::HotList>>,
    ) -> Result<(), String> {
        self.code = if on {
            let mut j =
                self.code.take().map_or_else(|| Mb86235Jit::with_max_block(self.max_block).map(Box::new), Ok)?;
            j.set_background(c.background);
            if let Some(l) = list {
                j.set_hot_list(l.clone());
            }
            Some(j)
        } else {
            None
        };
        self.check = c.copro_check;
        self.shadow = None;
        Ok(())
    }

    /// The TGP was changed other than by running (a boot, a loaded or
    /// copied state): the lockstep's interpreter starts again from it.
    pub fn reset(&mut self) {
        self.shadow = None;
    }

    /// `Tgp::run` on the JIT (`None` with it off). `key` says which
    /// external addresses are RAM (for the lockstep).
    #[inline]
    pub fn run<E: External>(&mut self, t: &mut Tgp, ext: &mut E, max: u64, key: Key) -> Option<Step> {
        let j = self.code.as_deref_mut()?;
        if !self.check {
            return Some(j.run(t, ext, max));
        }
        let shadow = match &mut self.shadow {
            Some(s) => s,
            None => {
                let mut s = Box::new(Tgp::new());
                s.copy_from(t);
                s.counts.clone_from(&t.counts);
                self.shadow.insert(s)
            }
        };
        shadow.fifo_in.clone_from(&t.fifo_in);
        shadow.fifo_out.clone_from(&t.fifo_out);
        shadow.ext_flags = t.ext_flags;
        let (pc, pending) = (t.pc, t.pending());
        let mut over = Overlay { inner: &mut *ext, key, writes: Vec::new() };
        let want = shadow.run(&mut over, max);
        let want_writes = over.writes;
        let mut listed = Listed { inner: &mut *ext, writes: Vec::new() };
        let got = j.run(t, &mut listed, max);
        self.checked += 1;
        // The RAMs and counts are compared too: 16 KB, so every 64th run
        // (and at every stall) to keep the lockstep's pace.
        let full = self.checked.is_multiple_of(64) || got != Step::Ran;
        let what = differs_regs(shadow, t, full)
            .or((want != got).then_some("where the run stopped"))
            .or((want_writes != listed.writes).then_some("external writes"));
        if let Some(what) = what {
            panic!(
                "copro JIT lockstep: the {} MB86235 on the JIT differs from the interpreter in {what}, in a run of up to \
                 {max} words from pc {pc:#x} (delayed branch {pending:?}; interpreter {want:?} at {:#x}, JIT {got:?} at \
                 {:#x})",
                self.name, shadow.pc, t.pc
            );
        }
        Some(got)
    }
}

/// [`differs`], or with `full` false, everything but the RAMs and counts.
fn differs_regs(a: &Tgp, b: &Tgp, full: bool) -> Option<&'static str> {
    if full {
        return differs(a, b);
    }
    let same = (a.aa, a.ab, a.ma, a.mb, a.ar, a.pr, a.prp, a.pwp) == (b.aa, b.ab, b.ma, b.mb, b.ar, b.pr, b.prp, b.pwp)
        && (a.eb, a.eo, a.sp, a.st, a.md, a.lpc, a.rpc, a.pdr, a.ddr, a.pc)
            == (b.eb, b.eo, b.sp, b.st, b.md, b.lpc, b.rpc, b.pdr, b.ddr, b.pc)
        && (a.pcs, a.pcsp, a.pending(), a.steps) == (b.pcs, b.pcsp, b.pending(), b.steps)
        && a.fifo_in == b.fifo_in
        && a.fifo_out == b.fifo_out;
    (!same).then(|| differs(a, b).unwrap_or("registers"))
}

/// A SHARC's JIT (Model 2B's copro or geometrizer: docs/jit.md, "The
/// SHARC"): on (`code`), and in a lockstep (`check`), the interpreter kept
/// beside it.
///
/// The lockstep runs the JIT first, on the board's bus, recording what
/// the bus gave it (each read, and the FLAG inputs it set after each
/// instruction it was asked about), then the interpreter on a replay of
/// that (the same reads in the same order, its writes compared with the
/// JIT's, the FLAG inputs as the bus set them, stopping at the step the
/// JIT stopped at), and compares the two cores after every run: so
/// neither sees the other's effects, and the board's bus needs no copy.
#[derive(Default)]
pub(crate) struct SharcJitSlot {
    pub code: Option<Box<jit::adsp2106x::SharcJit>>,
    pub check: bool,
    shadow: Option<Box<Sharc>>,
    pub checked: u64,
    /// The SHARC's name in a difference's report.
    pub name: &'static str,
    log: Log,
}

impl SharcJitSlot {
    pub fn new(name: &'static str) -> Self {
        SharcJitSlot { name, ..SharcJitSlot::default() }
    }

    pub fn on(&self) -> bool {
        self.code.is_some()
    }

    /// Turn it on or off, as `c` configures the JIT, with the hot-block
    /// list `list` (on fails where Cranelift has no back end).
    pub fn set(
        &mut self,
        on: bool,
        c: &jit::Config,
        list: Option<&std::sync::Arc<jit::HotList>>,
    ) -> Result<(), String> {
        self.code = if on {
            let mut j = self.code.take().map_or_else(|| jit::adsp2106x::SharcJit::new().map(Box::new), Ok)?;
            j.set_background(c.background);
            if let Some(l) = list {
                j.set_hot_list(l.clone());
            }
            Some(j)
        } else {
            None
        };
        self.check = c.copro_check;
        self.shadow = None;
        Ok(())
    }

    /// The core was changed other than by running (an upload, a host
    /// interface access, a loaded state): the lockstep's interpreter
    /// starts again from it.
    pub fn reset(&mut self) {
        self.shadow = None;
    }

    /// The core's state was copied from another core's (run-ahead): the
    /// JIT watches its words again, and the lockstep starts again.
    pub fn copied(&mut self) {
        if let Some(j) = &mut self.code {
            j.rewatch();
        }
        self.shadow = None;
    }

    /// `Sharc::run_bus` on the JIT (`None` with it off).
    #[inline]
    pub fn run<B: SharcBus>(&mut self, s: &mut Sharc, ext: &mut B) -> Option<SharcStep> {
        let j = self.code.as_deref_mut()?;
        if !self.check {
            return Some(j.run(s, ext));
        }
        Some(self.run_checked(s, ext))
    }

    #[inline(never)]
    fn run_checked<B: SharcBus>(&mut self, s: &mut Sharc, ext: &mut B) -> SharcStep {
        let j = self.code.as_deref_mut().expect("on");
        let shadow = match &mut self.shadow {
            Some(sh) => sh,
            None => {
                let mut sh = Box::new(Sharc::new());
                sh.copy_from(s, &mut Vec::new());
                sh.counts.clone_from(&s.counts);
                self.shadow.insert(sh)
            }
        };
        // The board sets the FLAG inputs between runs.
        shadow.flag_in = s.flag_in;
        if shadow.digest() != s.digest() {
            panic!(
                "copro JIT lockstep: the {} ADSP-21062 was changed outside a run, other than its FLAG inputs \
                 (a board change the lockstep doesn't know of)",
                self.name
            );
        }
        let (pc, steps) = (s.pc, s.steps);
        self.log.clear();
        let mut rec = Recorder { inner: ext, log: &mut self.log };
        let got = j.run(s, &mut rec);
        let mut rep = Replay { log: &self.log, r: 0, w: 0, f: 0, end: s.steps, ran: got == SharcStep::Ran, bad: None };
        let want = shadow.run_bus(&mut rep);
        let mut bad = rep.bad;
        if bad.is_none() && (rep.r != self.log.reads.len() || rep.w != self.log.writes.len()) {
            bad = Some("its external accesses (the interpreter made fewer)");
        }
        self.checked += 1;
        // Memory and counts every 64th run (and when it stops).
        let full = self.checked.is_multiple_of(64) || got != SharcStep::Ran;
        let what = bad
            .or((want != got).then_some("how the run ended"))
            .or((shadow.digest() != s.digest()).then_some("registers, stacks, pipeline or counters"))
            .or((full && shadow.mem.cols != s.mem.cols).then_some("internal memory"))
            .or((full && shadow.counts != s.counts).then_some("execution counts"))
            .or((full && shadow.iop != s.iop).then_some("IOP registers"));
        if let Some(what) = what {
            panic!(
                "copro JIT lockstep: the {} ADSP-21062 on the JIT differs from the interpreter in {what}, in a run from \
                 pc {pc:#x} at step {steps} (interpreter {want:?} at pc {:#x} step {}, JIT {got:?} at pc {:#x} step {})",
                self.name, shadow.pc, shadow.steps, s.pc, s.steps
            );
        }
        got
    }
}

use adsp2106x::cpu::{Bus as SharcBus, Sharc, Step as SharcStep, Width};

/// What a bus gave a run: its reads, the writes made to it, and the FLAG
/// inputs after each call that may set them, by step.
#[derive(Default)]
struct Log {
    reads: Vec<(u32, Width, u64)>,
    writes: Vec<(u32, Width, u64)>,
    flags: Vec<(u64, u8)>,
}

impl Log {
    fn clear(&mut self) {
        self.reads.clear();
        self.writes.clear();
        self.flags.clear();
    }
}

/// `inner`, logged.
struct Recorder<'a, B> {
    inner: &'a mut B,
    log: &'a mut Log,
}

impl<B: SharcBus> SharcBus for Recorder<'_, B> {
    fn read(&mut self, addr: u32, width: Width) -> u64 {
        let v = self.inner.read(addr, width);
        self.log.reads.push((addr, width, v));
        v
    }
    fn write(&mut self, addr: u32, width: Width, v: u64) {
        self.log.writes.push((addr, width, v));
        self.inner.write(addr, width, v)
    }
    fn after_step(&mut self, cpu: &mut Sharc) -> bool {
        let go = self.inner.after_step(cpu);
        self.log.flags.push((cpu.steps, cpu.flag_in));
        go
    }
    fn block_budget(&mut self, cpu: &Sharc) -> u64 {
        self.inner.block_budget(cpu)
    }
    fn after_access(&mut self, cpu: &mut Sharc) {
        self.inner.after_access(cpu);
        self.log.flags.push((cpu.steps, cpu.flag_in));
    }
}

/// A logged run played back to the interpreter.
struct Replay<'a> {
    log: &'a Log,
    r: usize,
    w: usize,
    f: usize,
    /// The step the JIT's run stopped at, and whether it stopped there by
    /// the bus's say (rather than IDLE or a fault, which the interpreter
    /// must reach itself).
    end: u64,
    ran: bool,
    bad: Option<&'static str>,
}

impl SharcBus for Replay<'_> {
    fn read(&mut self, addr: u32, width: Width) -> u64 {
        match self.log.reads.get(self.r) {
            Some(&(a, wd, v)) if (a, wd) == (addr, width) => {
                self.r += 1;
                v
            }
            _ => {
                self.bad.get_or_insert("its external reads");
                0
            }
        }
    }
    fn write(&mut self, addr: u32, width: Width, v: u64) {
        if self.log.writes.get(self.w) == Some(&(addr, width, v)) {
            self.w += 1;
        } else {
            self.bad.get_or_insert("its external writes");
        }
    }
    fn after_step(&mut self, cpu: &mut Sharc) -> bool {
        while let Some(&(at, flags)) = self.log.flags.get(self.f) {
            if at > cpu.steps {
                break;
            }
            cpu.flag_in = flags;
            self.f += 1;
        }
        if cpu.steps > self.end {
            self.bad.get_or_insert("where the run stopped (the interpreter ran on)");
            return false;
        }
        self.bad.is_none() && (cpu.steps < self.end || !self.ran)
    }
}
