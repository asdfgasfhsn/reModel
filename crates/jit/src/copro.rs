//! What the copro JITs share (docs/jit.md, "Copro JITs"): the Cranelift
//! module, its settings, the counters, and the IR helpers both lowerings
//! use. The chips' own lowerings are `mb86234` and `mb86235`.

use crate::pool::{Compiled, Compiler, Job, Urgency};
use cranelift_codegen::ir::{types, AbiParam, InstBuilder, MemFlagsData, SigRef, Signature, Type, Value};
use cranelift_frontend::FunctionBuilder;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{default_libcall_names, Module};
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

pub(crate) const I8: Type = types::I8;
pub(crate) const I32: Type = types::I32;
pub(crate) const I64: Type = types::I64;
pub(crate) const F32: Type = types::F32;
pub(crate) const F64: Type = types::F64;

pub(crate) fn trusted() -> MemFlagsData {
    MemFlagsData::trusted()
}

/// A Cranelift JIT module for this machine, optimiser off unless
/// `optimise` (as the i960's: docs/jit.md, "Back end"). The blocks it
/// holds are compiled by the translation workers (`pool`) and copied in.
pub(crate) fn module(optimise: bool) -> Result<JITModule, String> {
    Ok(JITModule::new(JITBuilder::with_isa(crate::pool::isa(optimise)?, default_libcall_names())))
}

/// A module whose code is freed with it. A copro JIT is made per uploaded
/// program and dropped with its machine (run-ahead's and the launcher's
/// machines come and go), so unlike the i960's it gives its memory back.
pub(crate) struct OwnedModule(pub ManuallyDrop<JITModule>);

impl Drop for OwnedModule {
    fn drop(&mut self) {
        // SAFETY: the module's code is only reachable through the JIT that
        // owns this, which is being dropped.
        unsafe { ManuallyDrop::take(&mut self.0).free_memory() };
    }
}

// SAFETY: a JIT moves between threads with the geometrizer that owns it
// (its worker runs one frame at a time); it is never used from two.
unsafe impl Send for OwnedModule {}

/// What a copro JIT has done.
#[derive(Clone, Copy, Debug, Default)]
pub struct CoproStats {
    /// Blocks translated, and their words.
    pub blocks: u64,
    pub words: u64,
    /// Of those, words left to the interpreter's step (in a block).
    pub fallbacks: u64,
    /// Host time translating (on the workers, and installing the code
    /// here), and the slowest block.
    pub time: Duration,
    pub slowest: Duration,
    /// Bytes of machine code.
    pub code_bytes: u64,
    /// Block runs, and words run on the interpreter outside any block (a
    /// start the JIT couldn't take: mid-repeat, a delay slot, the budget).
    pub runs: u64,
    pub interpreted: u64,
    /// Finished translations thrown away (the chip had another program
    /// by then).
    pub discarded: u64,
}

impl CoproStats {
    pub fn add(&mut self, o: &CoproStats) {
        self.blocks += o.blocks;
        self.words += o.words;
        self.fallbacks += o.fallbacks;
        self.time += o.time;
        self.slowest = self.slowest.max(o.slowest);
        self.code_bytes += o.code_bytes;
        self.runs += o.runs;
        self.interpreted += o.interpreted;
        self.discarded += o.discarded;
    }
}

/// A signature for a helper: `params` in, an I32 out if `ret`.
pub(crate) fn sig(b: &mut FunctionBuilder, params: &[Type], ret: Option<Type>) -> SigRef {
    let mut s = Signature::new(b.func.signature.call_conv);
    s.params.extend(params.iter().map(|&t| AbiParam::new(t)));
    if let Some(r) = ret {
        s.returns.push(AbiParam::new(r));
    }
    b.import_signature(s)
}

/// Call helper `f` (an `extern "C"` function's address) through `sig`.
pub(crate) fn call(b: &mut FunctionBuilder, ptr: Type, sig: SigRef, f: usize, args: &[Value]) -> Option<Value> {
    let callee = b.ins().iconst(ptr, f as i64);
    let call = b.ins().call_indirect(sig, callee, args);
    b.inst_results(call).first().copied()
}

/// A copro JIT's side of the translation workers (`pool`): the blocks
/// queued, their results coming back, and the run of words the
/// interpreter is going through while one isn't ready.
pub(crate) struct Background {
    /// On the workers; off: translated at once, here (tests).
    pub on: bool,
    tx: Sender<Done>,
    rx: Receiver<Done>,
    alive: Arc<AtomicBool>,
    /// Blocks queued: start, and the word after its last.
    pub pending: HashMap<u32, u32>,
    /// A queued block being interpreted (after its start, before its
    /// end): those words aren't block starts and aren't queued.
    pub through: (u32, u32),
}

/// A finished translation: for the program with `key`, the block from
/// `start` to `end`.
pub(crate) struct Done {
    pub key: u64,
    pub start: u32,
    pub end: u32,
    pub code: Result<Compiled, String>,
}

impl Background {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Background {
            on: true,
            tx,
            rx,
            alive: Arc::new(AtomicBool::new(true)),
            pending: HashMap::new(),
            through: (0, 0),
        }
    }

    /// True if `pc` is inside the queued block being interpreted.
    #[inline]
    pub fn inside(&self, pc: u32) -> bool {
        self.through.0 < pc && pc < self.through.1
    }

    /// Queue the block at `start` (to `end`) of program `key`, its IR
    /// emitted by `build` (a function of three pointers, returning `ret`).
    #[allow(clippy::too_many_arguments)]
    pub fn queue(
        &mut self,
        key: u64,
        start: u32,
        end: u32,
        urgency: Urgency,
        ret: Option<Type>,
        build: impl FnOnce(&mut FunctionBuilder, Type) + Send + 'static,
    ) {
        let (tx, alive) = (self.tx.clone(), self.alive.clone());
        let job: Job = Box::new(move |c: &mut Compiler| {
            if !alive.load(Ordering::Relaxed) {
                return;
            }
            let code = catch_unwind(AssertUnwindSafe(|| c.compile(false, 3, ret, build)))
                .unwrap_or_else(|_| Err("the translation panicked".into()));
            let _ = tx.send(Done { key, start, end, code });
        });
        self.pending.insert(start, end);
        crate::pool::submit(self.on, urgency, job);
    }

    /// The translations finished so far (`wait`: every one queued).
    pub fn finished(&mut self, wait: bool) -> Vec<Done> {
        let mut v = Vec::new();
        if wait {
            while !self.pending.is_empty() {
                let Ok(d) = self.rx.recv() else { break };
                self.pending.remove(&d.start);
                v.push(d);
            }
        }
        while let Ok(d) = self.rx.try_recv() {
            self.pending.remove(&d.start);
            v.push(d);
        }
        v
    }

    /// Forget what is queued (the chip has another program): its results
    /// are told apart by their key.
    pub fn clear(&mut self) {
        self.pending.clear();
        self.through = (0, 0);
    }
}

impl Drop for Background {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
    }
}

/// Put `done`'s code in `module` and make it executable: the entry points
/// and code sizes, in order, `None` for any that failed.
pub(crate) fn install(module: &mut JITModule, sig: &Signature, done: &[&Compiled]) -> Vec<Option<(*const u8, usize)>> {
    let ids: Vec<_> = done.iter().map(|c| c.define(module, sig)).collect();
    if module.finalize_definitions().is_err() {
        return vec![None; done.len()];
    }
    ids.iter().zip(done).map(|(id, c)| id.map(|id| (module.get_finalized_function(id), c.bytes.len()))).collect()
}

/// A block's signature in `module`: three pointers in, `ret` out.
pub(crate) fn block_signature(module: &JITModule, ret: Option<Type>) -> Signature {
    let ptr = module.target_config().pointer_type();
    let mut s = module.make_signature();
    s.params.extend([AbiParam::new(ptr), AbiParam::new(ptr), AbiParam::new(ptr)]);
    s.returns.extend(ret.map(AbiParam::new));
    s
}
