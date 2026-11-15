//! The translation workers (docs/jit.md, "Translation off the game's
//! thread"): Cranelift compiling a block runs on a small pool of
//! background threads shared by every JIT in the process, so the thread
//! that runs the game never waits for it.
//!
//! A JIT plans a block on its own thread (reading the code it will run),
//! hands the pool a job that builds the block's IR and compiles it to
//! bytes, and goes on interpreting. The bytes come back on the JIT's
//! channel, and the JIT copies them into its own executable memory (its
//! Cranelift module) at a block boundary, when it next reaches the block.
//! What is translated when varies with the host; what the machine does
//! doesn't, as a block and the interpreter retire the same instructions
//! with the same effects (docs/jit.md, "Timing stays exact").
//!
//! The machine code only calls helpers by absolute address and has no
//! relocations in practice; any it has (a libcall) go to the owning
//! module with the bytes, which resolves them as its own definitions.

use cranelift_codegen::control::ControlPlane;
use cranelift_codegen::ir::{AbiParam, Signature, Type};
use cranelift_codegen::isa::OwnedTargetIsa;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{FuncId, Module, ModuleReloc};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The Cranelift target for this machine, optimiser off unless
/// `optimise` (docs/jit.md, "Back end"). The workers' and the modules'
/// must agree, so both come from here.
pub(crate) fn isa(optimise: bool) -> Result<OwnedTargetIsa, String> {
    let mut flags = settings::builder();
    let set = |f: &mut settings::Builder, k: &str, v: &str| f.set(k, v).map_err(|e| format!("cranelift {k}: {e}"));
    set(&mut flags, "use_colocated_libcalls", "false")?;
    set(&mut flags, "is_pic", "false")?;
    set(&mut flags, "opt_level", if optimise { "speed" } else { "none" })?;
    if !optimise {
        set(&mut flags, "regalloc_algorithm", "single_pass")?;
    }
    set(&mut flags, "enable_verifier", if cfg!(debug_assertions) { "true" } else { "false" })?;
    cranelift_native::builder()
        .map_err(|e| format!("this machine has no Cranelift back end: {e}"))?
        .finish(settings::Flags::new(flags))
        .map_err(|e| format!("cranelift: {e}"))
}

/// A block compiled to bytes, not yet in executable memory.
pub(crate) struct Compiled {
    pub bytes: Vec<u8>,
    pub align: u64,
    pub relocs: Vec<ModuleReloc>,
    /// Host time building the IR and compiling it (on the worker).
    pub time: Duration,
}

impl Compiled {
    /// Put it in `module`'s memory; [`Module::finalize_definitions`] then
    /// makes it executable (once for a batch). Returns its id there.
    pub fn define(&self, module: &mut cranelift_jit::JITModule, sig: &Signature) -> Option<FuncId> {
        let id = module.declare_anonymous_function(sig).ok()?;
        module.define_function_bytes(id, self.align, &self.bytes, &self.relocs).ok()?;
        Some(id)
    }
}

/// What a worker compiles with: Cranelift's contexts, reused job to job,
/// and the targets.
pub(crate) struct Compiler {
    ctx: Context,
    fctx: FunctionBuilderContext,
    isas: [Option<OwnedTargetIsa>; 2],
}

impl Compiler {
    fn new() -> Self {
        Compiler { ctx: Context::new(), fctx: FunctionBuilderContext::new(), isas: [None, None] }
    }

    /// Compile a function of `params` pointer arguments (returning `ret`
    /// if any) whose body `build` emits, given the builder and the pointer
    /// type.
    pub fn compile(
        &mut self,
        optimise: bool,
        params: usize,
        ret: Option<Type>,
        build: impl FnOnce(&mut FunctionBuilder, Type),
    ) -> Result<Compiled, String> {
        let t0 = Instant::now();
        let slot = &mut self.isas[optimise as usize];
        if slot.is_none() {
            *slot = Some(isa(optimise)?);
        }
        let isa = slot.as_ref().expect("just made");
        let ptr = isa.pointer_type();
        self.ctx.clear();
        let mut sig = Signature::new(isa.default_call_conv());
        sig.params.extend((0..params).map(|_| AbiParam::new(ptr)));
        sig.returns.extend(ret.map(AbiParam::new));
        self.ctx.func.signature = sig;
        {
            let mut b = FunctionBuilder::new(&mut self.ctx.func, &mut self.fctx);
            build(&mut b, ptr);
            b.seal_all_blocks();
            b.finalize(isa.frontend_config());
        }
        let disasm = std::env::var_os("REMODEL_JIT_DISASM");
        self.ctx.set_disasm(disasm.is_some());
        self.ctx.compile(&**isa, &mut ControlPlane::default()).map_err(|e| format!("{:?}", e.inner))?;
        let code = self.ctx.compiled_code().expect("just compiled");
        if let (Some(dir), Some(text)) = (disasm, &code.vcode) {
            // A developer's look at the machine code (docs/jit.md).
            static N: AtomicUsize = AtomicUsize::new(0);
            let path = std::path::Path::new(&dir).join(format!("{:05}.s", N.fetch_add(1, Ordering::Relaxed)));
            let _ = std::fs::write(path, text);
        }
        let relocs =
            code.buffer.relocs().iter().map(|r| ModuleReloc::from_mach_reloc(r, &self.ctx.func, FuncId::from_u32(0)));
        let out = Compiled {
            bytes: code.code_buffer().to_vec(),
            align: code.buffer.min_alignment as u64,
            relocs: relocs.collect(),
            time: Duration::ZERO,
        };
        self.ctx.clear();
        Ok(Compiled { time: t0.elapsed(), ..out })
    }
}

/// A job: compile something and send it to whoever asked.
pub(crate) type Job = Box<dyn FnOnce(&mut Compiler) + Send>;

/// How soon a job is wanted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Urgency {
    /// The game is running this code now (on the interpreter until it is
    /// ready).
    Now,
    /// Ahead of time (the hot-block list, docs/jit.md "Hot-block list"):
    /// after every `Now` job.
    Later,
}

/// A queued job, taken by whoever runs it first: it can be in both
/// queues once promoted ([`promote`]).
type Item = Arc<Mutex<Option<Job>>>;

/// A job handed to the pool, to [`promote`] it if the game reaches its
/// code before a worker gets to it.
pub(crate) struct Ticket(Item);

struct Queue {
    now: VecDeque<Item>,
    later: VecDeque<Item>,
}

struct Pool {
    queue: Mutex<Queue>,
    ready: Condvar,
}

static POOL: OnceLock<Pool> = OnceLock::new();
/// Workers started; and how many to have (0: from the core count).
static STARTED: AtomicUsize = AtomicUsize::new(0);
static WANTED: AtomicUsize = AtomicUsize::new(0);

/// Jobs queued and not yet finished, for [`busy`].
static OUTSTANDING: AtomicUsize = AtomicUsize::new(0);

/// How many translation workers to run (before the first job; later
/// calls change nothing). The default leaves cores to the game's own
/// threads: the core count less three, from one to eight.
pub fn set_workers(n: usize) {
    WANTED.store(n.max(1), Ordering::Relaxed);
}

/// The number of translation workers (started or to be).
pub fn workers() -> usize {
    match WANTED.load(Ordering::Relaxed) {
        0 => {
            let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
            std::env::var("REMODEL_JIT_WORKERS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(cores.saturating_sub(3))
                .clamp(1, 8)
        }
        n => n,
    }
}

/// Jobs waiting or being compiled, across every JIT.
pub fn busy() -> usize {
    OUTSTANDING.load(Ordering::Relaxed)
}

fn pool() -> &'static Pool {
    let p = POOL.get_or_init(|| Pool {
        queue: Mutex::new(Queue { now: VecDeque::new(), later: VecDeque::new() }),
        ready: Condvar::new(),
    });
    if STARTED.load(Ordering::Acquire) == 0 {
        let n = workers();
        if STARTED.compare_exchange(0, n, Ordering::AcqRel, Ordering::Acquire).is_ok() {
            for k in 0..n {
                let spawned = std::thread::Builder::new().name(format!("jit-{k}")).spawn(move || work(p));
                if spawned.is_err() && k == 0 {
                    // No thread at all: jobs will run where they are
                    // submitted.
                    STARTED.store(usize::MAX, Ordering::Release);
                }
            }
        }
    }
    p
}

fn work(p: &'static Pool) {
    lower_priority();
    let mut c = Compiler::new();
    loop {
        let item = {
            let mut q = p.queue.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(j) = q.now.pop_front().or_else(|| q.later.pop_front()) {
                    break j;
                }
                q = p.ready.wait(q).unwrap_or_else(|e| e.into_inner());
            }
        };
        let job = item.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(job) = job {
            job(&mut c);
            OUTSTANDING.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

/// Make the calling thread yield to the game's threads when cores are
/// short: the translation workers, and any other background work done
/// beside a game (the player's `--jit-warm` run). Linux: a nice of 10;
/// macOS: the "utility" quality-of-service class, which also keeps the
/// thread off the performance cores the game's threads want; Windows:
/// below-normal priority.
pub fn lower_priority() {
    #[cfg(target_os = "linux")]
    // SAFETY: plain system calls on this thread's own id.
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, libc::gettid() as libc::id_t, 10);
    }
    #[cfg(target_os = "macos")]
    {
        // <pthread/qos.h>: qos_class_t's QOS_CLASS_UTILITY.
        const QOS_CLASS_UTILITY: u32 = 0x11;
        extern "C" {
            fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
        }
        // SAFETY: changes only the calling thread's own scheduling class.
        unsafe {
            pthread_set_qos_class_self_np(QOS_CLASS_UTILITY, 0);
        }
    }
    #[cfg(windows)]
    {
        const THREAD_PRIORITY_BELOW_NORMAL: i32 = -1;
        extern "system" {
            fn GetCurrentThread() -> *mut core::ffi::c_void;
            fn SetThreadPriority(thread: *mut core::ffi::c_void, priority: i32) -> i32;
        }
        // SAFETY: the pseudo-handle of the calling thread, its own priority.
        unsafe {
            SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
        }
    }
}

thread_local! {
    /// For jobs run where they are submitted (`background` off).
    static HERE: RefCell<Option<Compiler>> = const { RefCell::new(None) };
}

/// Run `job` on a worker, or here and now if `background` is off (or no
/// worker thread could be started). The ticket, if it was queued.
pub(crate) fn submit(background: bool, urgency: Urgency, job: Job) -> Option<Ticket> {
    if background {
        let p = pool();
        if STARTED.load(Ordering::Acquire) != usize::MAX {
            OUTSTANDING.fetch_add(1, Ordering::Relaxed);
            let item: Item = Arc::new(Mutex::new(Some(job)));
            let mut q = p.queue.lock().unwrap_or_else(|e| e.into_inner());
            match urgency {
                Urgency::Now => q.now.push_back(item.clone()),
                Urgency::Later => q.later.push_back(item.clone()),
            }
            drop(q);
            p.ready.notify_one();
            return Some(Ticket(item));
        }
    }
    HERE.with(|h| job(h.borrow_mut().get_or_insert_with(Compiler::new)));
    None
}

/// The game reached the code of a job queued for `Later`: it goes to the
/// front of the line too, if no worker has taken it yet.
pub(crate) fn promote(t: &Ticket) {
    if t.0.lock().unwrap_or_else(|e| e.into_inner()).is_none() {
        return;
    }
    let p = pool();
    p.queue.lock().unwrap_or_else(|e| e.into_inner()).now.push_front(t.0.clone());
    p.ready.notify_one();
}
