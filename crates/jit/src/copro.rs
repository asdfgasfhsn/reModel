//! What the copro JITs share (docs/jit.md, "Copro JITs"): the Cranelift
//! module, its settings, the counters, and the IR helpers both lowerings
//! use. The chips' own lowerings are `mb86234` and `mb86235`.

use cranelift_codegen::ir::{types, AbiParam, InstBuilder, MemFlagsData, SigRef, Signature, Type, Value};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::FunctionBuilder;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::default_libcall_names;
use std::mem::ManuallyDrop;
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
/// `optimise` (as the i960's: docs/jit.md, "Back end").
pub(crate) fn module(optimise: bool) -> Result<JITModule, String> {
    let mut flags = settings::builder();
    let set = |f: &mut settings::Builder, k: &str, v: &str| f.set(k, v).map_err(|e| format!("cranelift {k}: {e}"));
    set(&mut flags, "use_colocated_libcalls", "false")?;
    set(&mut flags, "is_pic", "false")?;
    set(&mut flags, "opt_level", if optimise { "speed" } else { "none" })?;
    if !optimise {
        set(&mut flags, "regalloc_algorithm", "single_pass")?;
    }
    set(&mut flags, "enable_verifier", if cfg!(debug_assertions) { "true" } else { "false" })?;
    let isa = cranelift_native::builder()
        .map_err(|e| format!("this machine has no Cranelift back end: {e}"))?
        .finish(settings::Flags::new(flags))
        .map_err(|e| format!("cranelift: {e}"))?;
    Ok(JITModule::new(JITBuilder::with_isa(isa, default_libcall_names())))
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
    /// Host time translating, and the slowest block.
    pub time: Duration,
    pub slowest: Duration,
    /// Bytes of machine code.
    pub code_bytes: u64,
    /// Block runs, and words run on the interpreter outside any block (a
    /// start the JIT couldn't take: mid-repeat, a delay slot, the budget).
    pub runs: u64,
    pub interpreted: u64,
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
