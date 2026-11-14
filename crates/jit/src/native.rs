//! The Cranelift back end (docs/jit.md).

use cranelift_codegen::ir::{
    condcodes::IntCC, types, AbiParam, Block as ClifBlock, InstBuilder, MemFlagsData, SigRef, Signature, Type, Value,
};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::JITModule;
use cranelift_module::Module;
use i960::cpu::{Cpu, Host, Stop};
use i960::lower::{lower, Alu, Op, Src, Width};
use i960::{decode, Flow, Insn, MemRef};
use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;
use std::mem::offset_of;
use std::time::{Duration, Instant};

/// Code is translated below this address (the program ROM and the RAM the
/// games copy code to); anything above runs on the interpreter.
pub const CODE_SPAN: u32 = 0x100_0000;
/// Bytes per write counter of [`JitHost::code_generations`].
pub const GEN_PAGE: u32 = 1 << GEN_SHIFT;
pub const GEN_SHIFT: u32 = 10;

/// What the JIT needs from the machine beyond what recompiled code does:
/// a way to see writes to code in writable memory.
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
}

/// How the JIT runs.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Times a block start is dispatched on the interpreter before it is
    /// translated (0: at once).
    pub hot: u32,
    /// Cranelift's optimiser (`speed`) and its full register allocator.
    /// Off by default: it doubles the translation time, and the blocks run
    /// no faster (the i960's own work is a small part of theirs; docs/jit.md).
    pub optimise: bool,
    /// Instructions per block at most.
    pub max_block: usize,
    /// The copro programs on their JITs too (the MB86234 of Model 2A:
    /// `docs/jit.md`, "Copro JITs"); with `copro_check`,
    /// each beside its interpreter, compared after every run (a lockstep).
    pub copro: bool,
    pub copro_check: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config { hot: 2, optimise: false, max_block: 128, copro: true, copro_check: false }
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
    /// block.
    pub time: Duration,
    pub slowest: Duration,
    /// Of `time`: Cranelift compiling (IR to machine code), and making the
    /// code executable.
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
}

/// A translated block: `(cpu, host, ctx) -> 0` (ran) or 1 (faulted, the
/// fault in `ctx`).
type BlockFn = unsafe extern "C" fn(*mut Cpu, *mut u8, *mut Ctx) -> u32;

/// Passed to every block: where the host's write counters are this time,
/// and where a helper leaves a fault.
#[repr(C)]
struct Ctx {
    gens: *const u32,
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
    Block(u32),
    /// Nothing to translate here (undecodable, or not code the JIT can
    /// track).
    Never,
}

const SLOTS_PER_PAGE: usize = 1024;

/// The JIT for a machine `H`.
pub struct Jit<H> {
    module: JITModule,
    ctx: Context,
    fctx: FunctionBuilderContext,
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
    host: PhantomData<fn(&mut H)>,
}

impl<H: JitHost> Jit<H> {
    pub fn new(config: Config) -> Result<Self, String> {
        let module = crate::copro::module(config.optimise)?;
        let ctx = module.make_context();
        Ok(Jit {
            module,
            ctx,
            fctx: FunctionBuilderContext::new(),
            blocks: Vec::new(),
            table: Vec::new(),
            stops: BTreeSet::new(),
            config,
            stats: Stats::default(),
            fallback_kinds: BTreeMap::new(),
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

    /// Run the block at the CPU's IP, translating it if it is due. `None`
    /// if there is none (yet): the caller runs one instruction on the
    /// interpreter. Interrupts must have been taken first, as for a
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
            Slot::Block(id) => id,
            Slot::Never => {
                self.stats.interpreted += 1;
                return None;
            }
            Slot::Cold(n) if n < hot => {
                *self.slot(ip) = Slot::Cold(n + 1);
                self.stats.interpreted += 1;
                return None;
            }
            Slot::Cold(_) => match self.translate_at(ip, host) {
                Some(id) => id,
                None => {
                    *self.slot(ip) = Slot::Never;
                    self.stats.interpreted += 1;
                    return None;
                }
            },
        };
        let mut id = id;
        if self.blocks[id as usize].ram.is_some() && !self.still_there(id, host) {
            // The code changed: translate what is there now.
            self.stats.dropped += 1;
            match self.translate_at(ip, host) {
                Some(new) => id = new,
                None => {
                    *self.slot(ip) = Slot::Never;
                    self.stats.interpreted += 1;
                    return None;
                }
            }
        }
        self.stats.runs += 1;
        let gens = host.code_generations().as_ptr();
        let mut ctx = Ctx { gens, fault: None };
        let code = self.blocks[id as usize].code;
        // SAFETY: the block was made for `H` by `emit`, from instructions
        // it owns; it only touches the CPU and host through the helpers
        // below and the CPU's fields by offset.
        let r = unsafe { code(cpu, host as *mut H as *mut u8, &mut ctx) };
        match r {
            0 => Some(Ok(())),
            _ => Some(Err(ctx.fault.expect("a faulting helper leaves its fault"))),
        }
    }

    /// True if block `id` (from writable memory) still holds what is in
    /// memory: its pages weren't written, or its words are the same.
    fn still_there(&mut self, id: u32, host: &H) -> bool {
        let b = &mut self.blocks[id as usize];
        let Some(ram) = &mut b.ram else { return true };
        let gens = host.code_generations();
        let now = [gens[ram.pages[0]], gens[ram.pages[1]]];
        if now == *ram.seen {
            return true;
        }
        self.stats.rechecked += 1;
        let same = ram.words.iter().enumerate().all(|(k, &w)| host.fetch(b.start + 4 * k as u32) == Some(w));
        if same {
            *ram.seen = now;
        }
        same
    }

    fn translate_at(&mut self, ip: u32, host: &H) -> Option<u32> {
        let insns = self.plan(ip, host)?;
        let id = self.translate(insns, host)?;
        *self.slot(ip) = Slot::Block(id);
        Some(id)
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
    /// first ran.)
    pub fn translate_program(&mut self, insns: &BTreeMap<u32, Insn>, extra: &BTreeSet<u32>, host: &H) -> usize {
        let leaders = i960::codegen::leaders(insns, extra);
        let mut made = 0;
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
            if let Some(id) = self.translate(block, host) {
                *self.slot(start) = Slot::Block(id);
                made += 1;
            }
        }
        made
    }

    fn translate(&mut self, insns: Vec<Insn>, host: &H) -> Option<u32> {
        let t0 = Instant::now();
        let start = insns[0].addr;
        let end = insns.last().map_or(start, Insn::next);
        let ram = if host.is_rom(start) {
            None
        } else {
            let first = host.code_generation_index(start)?;
            let last = host.code_generation_index(end - 1)?;
            // A block over more than two counters' pages isn't worth the
            // checks (they are 1 KB; blocks are at most 128 instructions).
            if last.abs_diff(first) > 1 {
                return None;
            }
            let gens = host.code_generations();
            let words: Option<Vec<u32>> = (start..end).step_by(4).map(|a| host.fetch(a)).collect();
            Some(RamCode {
                words: words?.into_boxed_slice(),
                pages: [first, last],
                seen: Box::new([gens[first], gens[last]]),
            })
        };
        let insns = insns.into_boxed_slice();
        let fallbacks = insns.iter().filter(|i| lower(i).is_none()).count();
        for i in insns.iter().filter(|i| lower(i).is_none()) {
            *self.fallback_kinds.entry(i.mnemonic).or_default() += 1;
        }
        let sig = self.block_signature();
        self.ctx.func.signature = sig;
        let ptr = self.module.target_config().pointer_type();
        {
            let mut b = FunctionBuilder::new(&mut self.ctx.func, &mut self.fctx);
            let mut e = Emit::<H>::new(&mut b, ptr, ram.as_ref().map(|r| (r.pages, &*r.seen as *const [u32; 2])));
            e.block(&insns);
            b.seal_all_blocks();
            b.finalize(self.module.target_config());
        }
        let id = self.module.declare_anonymous_function(&self.ctx.func.signature).ok()?;
        let t1 = Instant::now();
        let defined = self.module.define_function(id, &mut self.ctx);
        let t2 = Instant::now();
        self.stats.compile += t2 - t1;
        let size = self.ctx.compiled_code().map_or(0, |c| c.code_buffer().len());
        self.module.clear_context(&mut self.ctx);
        if let Err(e) = defined {
            eprintln!("jit: block at {start:#x}: {e:?}");
            return None;
        }
        self.module.finalize_definitions().ok()?;
        self.stats.link += t2.elapsed();
        let ptr = self.module.get_finalized_function(id);
        // SAFETY: made with `block_signature`, the C convention of `BlockFn`.
        let code = unsafe { std::mem::transmute::<*const u8, BlockFn>(ptr) };
        let n = insns.len() as u64;
        self.stats.blocks += 1;
        self.stats.insns += n;
        self.stats.fallbacks += fallbacks as u64;
        self.stats.ram_blocks += ram.is_some() as u64;
        self.stats.code_bytes += size as u64;
        self.blocks.push(Block { code, start, insns, ram });
        let dt = t0.elapsed();
        self.stats.time += dt;
        self.stats.slowest = self.stats.slowest.max(dt);
        Some(self.blocks.len() as u32 - 1)
    }

    fn block_signature(&self) -> Signature {
        let ptr = self.module.target_config().pointer_type();
        let mut s = self.module.make_signature();
        s.params.extend([AbiParam::new(ptr), AbiParam::new(ptr), AbiParam::new(ptr)]);
        s.returns.push(AbiParam::new(types::I32));
        s
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
            read: sig(&[ptr, I32, I32], true),
            write: sig(&[ptr, I32, I32, I32], false),
            call: sig(&[ptr, ptr, I32, I32, I32], false),
            ret: sig(&[ptr, ptr, ptr, I32], true),
            exec: sig(&[ptr, ptr, ptr, ptr], true),
            step: sig(&[ptr, ptr, I32, I32], true),
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

    fn read(&mut self, width: u32, addr: Value, ip: u32) -> Value {
        let f = match width {
            1 => (read8::<H> as ReadFn) as usize,
            2 => (read16::<H> as ReadFn) as usize,
            _ => (read32::<H> as ReadFn) as usize,
        };
        let ip = self.konst(ip);
        let (host, sig) = (self.host, self.sigs.read);
        self.call(sig, f, &[host, addr, ip]).expect("reads return a value")
    }

    fn write(&mut self, width: u32, addr: Value, v: Value, ip: u32) {
        let f = match width {
            1 => (write8::<H> as WriteFn) as usize,
            2 => (write16::<H> as WriteFn) as usize,
            _ => (write32::<H> as WriteFn) as usize,
        };
        let ip = self.konst(ip);
        let (host, sig) = (self.host, self.sigs.write);
        self.call(sig, f, &[host, addr, v, ip]);
    }

    /// Continue in a new block if `fail` is zero; else go to `to`.
    fn exit_if(&mut self, fail: Value, to: ClifBlock) {
        let go_on = self.b.create_block();
        self.b.ins().brif(fail, to, &[], go_on, &[]);
        self.b.switch_to_block(go_on);
    }

    /// The block: each instruction as the static recompiler's
    /// `program_module` runs it. What follows each instruction (count it,
    /// retire it, decide whether to go on) is one helper call, to keep the
    /// IR, and so the translation time, small.
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
            // Some "sequential" instructions can still move the IP: an IAC
            // reinit, or an interrupt taken by `modpc`.
            let mut flags = if op.is_none() && !ends { STEP_MOVED } else { 0 };
            if ends {
                flags |= STEP_ENDS;
            }
            let mut flags = self.konst(flags);
            if let (Some((pages, seen)), false, true) = (self.ram, last, op.is_none_or(|o| o.stores())) {
                // This block's own code may have been written: leave it,
                // and the dispatcher checks it again.
                let changed = self.code_written(pages, seen);
                let zero = self.konst(0);
                let changed = self.b.ins().icmp(IntCC::NotEqual, changed, zero);
                let leave = self.konst(STEP_LEAVE);
                let leave = self.b.ins().select(changed, leave, zero);
                flags = self.b.ins().bor(flags, leave);
            }
            let (cpu, host, sig) = (self.cpu, self.host, self.sigs.step);
            let n = self.konst(next);
            let f = if last { (step_last::<H> as StepFn) as usize } else { (step::<H> as StepFn) as usize };
            let stop = self.call(sig, f, &[cpu, host, n, flags]).expect("step returns");
            if last {
                self.b.ins().jump(done, &[]);
                break;
            }
            self.exit_if(stop, done);
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
                let (cpu, host, sig) = (self.cpu, self.host, self.sigs.call);
                let (t, n, ip) = (self.konst(target), self.konst(next), self.konst(pc));
                self.call(sig, (call::<H> as CallFn) as usize, &[cpu, host, t, n, ip]);
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
                let (cpu, host, sig) = (self.cpu, self.host, self.sigs.call);
                let (n, ip) = (self.konst(next), self.konst(pc));
                self.call(sig, (call::<H> as CallFn) as usize, &[cpu, host, a, n, ip]);
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

type ReadFn = unsafe extern "C" fn(*mut u8, u32, u32) -> u32;
type WriteFn = unsafe extern "C" fn(*mut u8, u32, u32, u32);
type CallFn = unsafe extern "C" fn(*mut Cpu, *mut u8, u32, u32, u32);
type RetFn = unsafe extern "C" fn(*mut Cpu, *mut u8, *mut Ctx, u32) -> u32;
type ExecFn = unsafe extern "C" fn(*mut Cpu, *mut u8, *mut Ctx, *const Insn) -> u32;
type StepFn = unsafe extern "C" fn(*mut Cpu, *mut u8, u32, u32) -> u32;

unsafe extern "C" fn read8<H: Host>(h: *mut u8, a: u32, ip: u32) -> u32 {
    let h = unsafe { &mut *(h as *mut H) };
    h.at(ip);
    h.read8(a) as u32
}

unsafe extern "C" fn read16<H: Host>(h: *mut u8, a: u32, ip: u32) -> u32 {
    let h = unsafe { &mut *(h as *mut H) };
    h.at(ip);
    h.read16(a) as u32
}

unsafe extern "C" fn read32<H: Host>(h: *mut u8, a: u32, ip: u32) -> u32 {
    let h = unsafe { &mut *(h as *mut H) };
    h.at(ip);
    h.read32(a)
}

unsafe extern "C" fn write8<H: Host>(h: *mut u8, a: u32, v: u32, ip: u32) {
    let h = unsafe { &mut *(h as *mut H) };
    h.at(ip);
    h.write8(a, v as u8)
}

unsafe extern "C" fn write16<H: Host>(h: *mut u8, a: u32, v: u32, ip: u32) {
    let h = unsafe { &mut *(h as *mut H) };
    h.at(ip);
    h.write16(a, v as u16)
}

unsafe extern "C" fn write32<H: Host>(h: *mut u8, a: u32, v: u32, ip: u32) {
    let h = unsafe { &mut *(h as *mut H) };
    h.at(ip);
    h.write32(a, v)
}

unsafe extern "C" fn call<H: Host>(c: *mut Cpu, h: *mut u8, target: u32, next: u32, ip: u32) {
    let (c, h) = unsafe { (&mut *c, &mut *(h as *mut H)) };
    h.at(ip);
    c.call(h, target, next)
}

unsafe extern "C" fn ret<H: Host>(c: *mut Cpu, h: *mut u8, ctx: *mut Ctx, ip: u32) -> u32 {
    let (c, h, ctx) = unsafe { (&mut *c, &mut *(h as *mut H), &mut *ctx) };
    h.at(ip);
    match c.ret(h) {
        Ok(()) => 0,
        Err(kind) => {
            ctx.fault = Some(Stop::Fault { kind, ip });
            1
        }
    }
}

unsafe extern "C" fn exec<H: Host>(c: *mut Cpu, h: *mut u8, ctx: *mut Ctx, insn: *const Insn) -> u32 {
    let (c, h, ctx, insn) = unsafe { (&mut *c, &mut *(h as *mut H), &mut *ctx, &*insn) };
    h.at(insn.addr);
    match c.exec(h, &insn.view()) {
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

/// After an instruction in the middle of a block: count and retire it.
/// Nonzero to leave the block, with the IP where it goes on (`next`,
/// unless the instruction moved it).
unsafe extern "C" fn step<H: Host>(c: *mut Cpu, h: *mut u8, next: u32, flags: u32) -> u32 {
    let (c, h) = unsafe { (&mut *c, &mut *(h as *mut H)) };
    c.icount += 1;
    if flags & STEP_MOVED != 0 && c.ip != next {
        h.retire(c);
        return 1;
    }
    if !h.retire(c) || c.interrupt_due() || flags & STEP_LEAVE != 0 {
        c.ip = next;
        return 1;
    }
    0
}

/// After a block's last instruction: count and retire it, and put the IP
/// at `next` unless it set it (a control transfer) or moved it.
unsafe extern "C" fn step_last<H: Host>(c: *mut Cpu, h: *mut u8, next: u32, flags: u32) -> u32 {
    let (c, h) = unsafe { (&mut *c, &mut *(h as *mut H)) };
    c.icount += 1;
    h.retire(c);
    if flags & STEP_ENDS == 0 && (flags & STEP_MOVED == 0 || c.ip == next) {
        c.ip = next;
    }
    1
}

#[cfg(test)]
mod tests;
