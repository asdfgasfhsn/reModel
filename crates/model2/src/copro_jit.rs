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

    /// Turn it on or off (on fails where Cranelift has no back end).
    pub fn set(&mut self, on: bool, check: bool) -> Result<(), String> {
        self.code = if on {
            Some(self.code.take().map_or_else(|| Mb86235Jit::with_max_block(self.max_block).map(Box::new), Ok)?)
        } else {
            None
        };
        self.check = check;
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
