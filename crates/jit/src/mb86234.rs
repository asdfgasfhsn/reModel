//! The MB86234 (the Model 2A TGP) on Cranelift (docs/jit.md, "Copro
//! JITs"): blocks of the uploaded program translated to machine code,
//! each word lowered to IR exactly as `mb86234::Cpu::step` defines it.
//!
//! The chip runs synchronously: the board runs it after each host access
//! until it waits for input ([`Mb86234Jit::run`] has `Cpu::run`'s
//! contract). So timing is "where does it stop": a block never holds a
//! word that reads the input FIFO except as its first, and the dispatcher
//! makes the interpreter's check before each block, so a run stops on
//! exactly the word, with exactly the state, the interpreter's would.
//! The step limit is kept the same way (a block runs only if it fits in
//! what is left).
//!
//! Everything is translated (words of a form the interpreter doesn't know
//! count as unknown and do nothing, as there); the parts with state
//! outside the registers and RAM (the FIFOs, the return stack, the
//! external bus and function unit, the diagnostics) are calls into the
//! interpreter's own methods.

use crate::copro::{call, module, sig, trusted, CoproStats, OwnedModule, F32, F64, I32, I8};
use ::mb86234::{reads_fifo, reg, Bus, Cpu, Stop};
use cranelift_codegen::ir::{condcodes::IntCC, AbiParam, InstBuilder, MemFlagsData, SigRef, Type, Value};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::Module;
use std::mem::{offset_of, ManuallyDrop};
use std::time::Instant;

/// Words per block at most.
const MAX_BLOCK: usize = 64;
/// Program addresses (the PC is 16 bits).
const SPAN: usize = 0x1_0000;

/// A translated block: `(cpu, bus, ctx)`, where `bus` points at a
/// `&mut dyn Bus`.
type BlockFn = unsafe extern "C" fn(*mut Cpu, *mut u8, *const Ctx);

/// Passed to every block: the RAM and the program (their `Vec`s don't
/// move while a block runs).
#[repr(C)]
struct Ctx {
    ram: *mut u32,
    prog: *const u32,
    prog_len: u32,
}

struct Block {
    code: BlockFn,
    /// Words it runs (a repeated word counts each time).
    cost: u64,
}

/// Slot values: not translated yet, nothing to translate, else block + 2.
const COLD: u32 = 0;
const NEVER: u32 = 1;

/// The JIT for one uploaded program.
pub struct Mb86234Jit {
    module: OwnedModule,
    ctx: Context,
    fctx: FunctionBuilderContext,
    blocks: Vec<Block>,
    table: Box<[u32]>,
    program: Vec<u32>,
    stats: CoproStats,
}

impl Mb86234Jit {
    /// A JIT for `program` (the words the chip runs; translated as it
    /// reaches them).
    pub fn new(program: &[u32]) -> Result<Self, String> {
        let module = module(false)?;
        let ctx = module.make_context();
        Ok(Mb86234Jit {
            module: OwnedModule(ManuallyDrop::new(module)),
            ctx,
            fctx: FunctionBuilderContext::new(),
            blocks: Vec::new(),
            table: vec![COLD; SPAN].into_boxed_slice(),
            program: program.to_vec(),
            stats: CoproStats::default(),
        })
    }

    /// Make this the JIT for `program` instead, keeping its module: the
    /// old blocks' code stays allocated until the JIT is dropped (cheaper
    /// than a new module where programs come and go in numbers, as in
    /// the tests).
    pub fn reload(&mut self, program: &[u32]) {
        self.blocks.clear();
        self.table.fill(COLD);
        self.program = program.to_vec();
    }

    /// Is this the JIT for `program`?
    pub fn is_for(&self, program: &[u32]) -> bool {
        self.program == program
    }

    pub fn stats(&self) -> CoproStats {
        self.stats
    }

    /// [`Cpu::run`] on translated code: run until the program needs input
    /// it hasn't got, or for `limit` words, stopping where and as the
    /// interpreter does. `cpu.program` must be the program this JIT is
    /// for ([`Mb86234Jit::is_for`]).
    pub fn run(&mut self, cpu: &mut Cpu, bus: &mut dyn Bus, limit: u64) -> Stop {
        debug_assert!(cpu.program.len() == self.program.len());
        let mut left = limit;
        while left > 0 {
            let pc = cpu.pc;
            let w = cpu.program.get(pc as usize).copied().unwrap_or(0);
            if reads_fifo(w) && cpu.fifo_in.is_empty() {
                return Stop::Input;
            }
            // A block starts with no repeat under way (a stop for input
            // can leave one: the interpreter finishes it).
            if cpu.repeat == 0 {
                let slot = self.table[pc as usize];
                let id = match slot {
                    COLD => match self.translate(pc) {
                        Some(id) => Some(id),
                        None => {
                            self.table[pc as usize] = NEVER;
                            None
                        }
                    },
                    NEVER => None,
                    s => Some(s - 2),
                };
                if let Some(id) = id {
                    let b = &self.blocks[id as usize];
                    if b.cost <= left {
                        let mut bus: &mut dyn Bus = bus;
                        let ctx = Ctx {
                            ram: cpu.ram.as_mut_ptr(),
                            prog: cpu.program.as_ptr(),
                            prog_len: cpu.program.len() as u32,
                        };
                        // SAFETY: made by `translate` for this program; it
                        // touches the CPU by field offsets and through the
                        // helpers below, the RAM within its 0x400 words and
                        // the program within `prog_len`.
                        unsafe { (b.code)(cpu, &mut bus as *mut &mut dyn Bus as *mut u8, &ctx) };
                        cpu.steps += b.cost;
                        left -= b.cost;
                        self.stats.runs += 1;
                        continue;
                    }
                }
            }
            self.stats.interpreted += 1;
            cpu.run(bus, 1);
            left -= 1;
        }
        Stop::Limit
    }

    fn fetch(&self, pc: u32) -> u32 {
        self.program.get(pc as usize).copied().unwrap_or(0)
    }

    /// The words of the block at `pc`: up to and including a control
    /// word, stopping before a word that reads the input FIFO (it must be
    /// checked first) unless it is the first. A repeat (`3C04 n`) and the
    /// word it repeats go together, or the block ends before them.
    fn plan(&self, start: u32) -> Vec<Planned> {
        let mut v = Vec::new();
        let mut pc = start;
        while v.len() < MAX_BLOCK {
            let w = self.fetch(pc);
            if !v.is_empty() && reads_fifo(w) {
                break;
            }
            if control(w) {
                v.push(Planned { pc, w, times: 1, repeated: false });
                break;
            }
            if let Some(n) = repeat(w) {
                let next = (pc + 1) & 0xffff;
                let r = self.fetch(next);
                if v.len() + 2 > MAX_BLOCK || control(r) || repeat(r).is_some() || reads_fifo(r) {
                    // Leave the repeat to the interpreter.
                    if v.is_empty() {
                        v.push(Planned { pc, w, times: 1, repeated: false });
                    }
                    break;
                }
                v.push(Planned { pc, w, times: 1, repeated: false });
                v.push(Planned { pc: next, w: r, times: n.max(1), repeated: true });
                pc = (next + 1) & 0xffff;
                continue;
            }
            v.push(Planned { pc, w, times: 1, repeated: false });
            pc = (pc + 1) & 0xffff;
        }
        v
    }

    fn translate(&mut self, start: u32) -> Option<u32> {
        let t0 = Instant::now();
        let words = self.plan(start);
        if words.is_empty() {
            return None;
        }
        let ptr = self.module.0.target_config().pointer_type();
        let mut s = self.module.0.make_signature();
        s.params.extend([AbiParam::new(ptr), AbiParam::new(ptr), AbiParam::new(ptr)]);
        self.ctx.func.signature = s;
        let prog_len = self.program.len();
        {
            let mut b = FunctionBuilder::new(&mut self.ctx.func, &mut self.fctx);
            let mut e = Emit::new(&mut b, ptr, prog_len);
            e.block(&words);
            b.seal_all_blocks();
            b.finalize(self.module.0.target_config());
        }
        let m = &mut self.module.0;
        let id = m.declare_anonymous_function(&self.ctx.func.signature).ok()?;
        let defined = m.define_function(id, &mut self.ctx);
        let size = self.ctx.compiled_code().map_or(0, |c| c.code_buffer().len());
        m.clear_context(&mut self.ctx);
        if let Err(e) = defined {
            eprintln!("jit: MB86234 block at {start:#x}: {e:?}");
            return None;
        }
        m.finalize_definitions().ok()?;
        // SAFETY: made with the signature of `BlockFn` (the C convention).
        let code = unsafe { std::mem::transmute::<*const u8, BlockFn>(m.get_finalized_function(id)) };
        let cost = words.iter().map(|p| p.times as u64).sum();
        self.blocks.push(Block { code, cost });
        let id = self.blocks.len() as u32 - 1;
        self.table[start as usize] = id + 2;
        let dt = t0.elapsed();
        self.stats.blocks += 1;
        self.stats.words += words.len() as u64;
        self.stats.code_bytes += size as u64;
        self.stats.time += dt;
        self.stats.slowest = self.stats.slowest.max(dt);
        Some(id)
    }
}

/// The first part of the chips' state in which `a` and `b` differ, if
/// any: everything the interpreter keeps, its diagnostics included (the
/// lockstep's and the tests' comparison).
pub fn differs(a: &Cpu, b: &Cpu) -> Option<&'static str> {
    let fa = &a.funcs;
    let fb = &b.funcs;
    let checks: [(&'static str, bool); 17] = [
        ("registers", a.regs == b.regs),
        ("the ALU's accumulator", a.acc_in == b.acc_in),
        ("pc", a.pc == b.pc),
        ("the return stack", a.stack == b.stack),
        ("the repeat", (a.repeat, a.repeat_armed) == (b.repeat, b.repeat_armed)),
        ("flags", (a.negative, a.zero) == (b.negative, b.zero)),
        ("control registers", a.ctrl == b.ctrl),
        ("mode registers", a.mode == b.mode),
        ("the bus mode", a.shared_bus == b.shared_bus),
        ("the input FIFO", a.fifo_in == b.fifo_in),
        ("the output FIFO", a.fifo_out == b.fifo_out),
        ("RAM", a.ram == b.ram),
        ("the step count", a.steps == b.steps),
        ("unknown words", a.unknown == b.unknown),
        ("unknown external addresses", a.unknown_ext == b.unknown_ext),
        (
            "the function unit",
            (fa.angle, fa.recip, fa.rsqrt, fa.atan_x, fa.atan_y, fa.atan_t)
                == (fb.angle, fb.recip, fb.rsqrt, fb.atan_x, fb.atan_y, fb.atan_t),
        ),
        ("the program", a.program == b.program),
    ];
    checks.iter().find(|c| !c.1).map(|c| c.0)
}

/// A word of a block, and how many times it runs (a repeated word's
/// count, else 1).
struct Planned {
    pc: u32,
    w: u32,
    times: u32,
    repeated: bool,
}

/// Does `w` (possibly) change the PC other than to the next word?
fn control(w: u32) -> bool {
    matches!(w >> 24, 0xbe | 0xfe | 0xbf)
}

/// The count of a repeat word (`3C`/`3D` writing control register 4).
fn repeat(w: u32) -> Option<u32> {
    (matches!(w >> 24, 0x3c | 0x3d) && (w >> 16) & 0xf == 4).then_some(w & 0xffff)
}

fn sext(v: u32, bits: u32) -> u32 {
    let s = 32 - bits;
    (((v << s) as i32) >> s) as u32
}

/// A float register's views (`mb86234::view_base`): the base register
/// and which view (0 whole, 1 exponent, 2 mantissa).
fn view_base(r: u8) -> Option<(u8, u8)> {
    match r {
        0x10..=0x1e => Some((0x10 + (r - 0x10) / 3 * 3, (r - 0x10) % 3)),
        _ => None,
    }
}

const OFF_REGS: i32 = offset_of!(Cpu, regs) as i32;
const OFF_ACC_IN: i32 = offset_of!(Cpu, acc_in) as i32;
const OFF_PC: i32 = offset_of!(Cpu, pc) as i32;
const OFF_REPEAT: i32 = offset_of!(Cpu, repeat) as i32;
const OFF_NEG: i32 = offset_of!(Cpu, negative) as i32;
const OFF_ZERO: i32 = offset_of!(Cpu, zero) as i32;
const OFF_CTRL: i32 = offset_of!(Cpu, ctrl) as i32;
const OFF_MODE: i32 = offset_of!(Cpu, mode) as i32;
const OFF_SHARED: i32 = offset_of!(Cpu, shared_bus) as i32;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Space {
    Int,
    Ext,
    Prog,
}

/// An operand's address before masking: index plus base.
#[derive(Clone, Copy)]
struct Ea {
    index: Value,
    base: Option<Value>,
}

/// Where a word leaves the PC.
enum Next {
    Seq,
    At(Value),
}

struct Sigs {
    pop: SigRef,
    push_out: SigRef,
    ext_read: SigRef,
    ext_write: SigRef,
    unknown: SigRef,
    push: SigRef,
    ret: SigRef,
    steep: SigRef,
    word: SigRef,
}

struct Emit<'a, 'b> {
    b: &'a mut FunctionBuilder<'b>,
    ptr: Type,
    cpu: Value,
    bus: Value,
    ram: Value,
    prog: Value,
    prog_len: usize,
    sigs: Sigs,
}

impl<'a, 'b> Emit<'a, 'b> {
    fn new(b: &'a mut FunctionBuilder<'b>, ptr: Type, prog_len: usize) -> Self {
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let p = b.block_params(entry).to_vec();
        let ram = b.ins().load(ptr, trusted(), p[2], offset_of!(Ctx, ram) as i32);
        let prog = b.ins().load(ptr, trusted(), p[2], offset_of!(Ctx, prog) as i32);
        let sigs = Sigs {
            pop: sig(b, &[ptr], Some(I32)),
            push_out: sig(b, &[ptr, I32], None),
            ext_read: sig(b, &[ptr, ptr, I32], Some(I32)),
            ext_write: sig(b, &[ptr, ptr, I32, I32], None),
            unknown: sig(b, &[ptr, I32], None),
            push: sig(b, &[ptr, I32], None),
            ret: sig(b, &[ptr, I32, I32], Some(I32)),
            steep: sig(b, &[ptr], Some(I32)),
            word: sig(b, &[F64], Some(I32)),
        };
        Emit { b, ptr, cpu: p[0], bus: p[1], ram, prog, prog_len, sigs }
    }

    fn k(&mut self, v: u32) -> Value {
        self.b.ins().iconst(I32, v as i64)
    }

    fn ld(&mut self, off: i32) -> Value {
        self.b.ins().load(I32, trusted(), self.cpu, off)
    }

    fn st(&mut self, off: i32, v: Value) {
        self.b.ins().store(trusted(), v, self.cpu, off);
    }

    fn reg(&mut self, r: u8) -> Value {
        self.ld(OFF_REGS + 4 * (r as i32 & 0x3f))
    }

    fn set_reg(&mut self, r: u8, v: Value) {
        self.st(OFF_REGS + 4 * (r as i32 & 0x3f), v);
    }

    fn helper(&mut self, s: SigRef, f: usize, args: &[Value]) -> Option<Value> {
        call(self.b, self.ptr, s, f, args)
    }

    fn unknown(&mut self, w: u32) {
        let (cpu, s, w) = (self.cpu, self.sigs.unknown, self.k(w));
        self.helper(s, h_unknown as HUnknown as usize, &[cpu, w]);
    }

    // ---- the block

    fn block(&mut self, words: &[Planned]) {
        let mut next = None;
        for p in words {
            if p.repeated {
                self.repeated(p);
            } else {
                next = Some(self.word(p.pc, p.w));
            }
        }
        let last = words.last().expect("a block has words");
        let pc = match next {
            Some(Next::At(v)) if !last.repeated => self.b.ins().band_imm_u(v, 0xffff),
            _ => self.k((last.pc + 1) & 0xffff),
        };
        self.st(OFF_PC, pc);
        self.b.ins().return_(&[]);
    }

    /// The word after a repeat, run `times` times: as the interpreter,
    /// which holds the PC on it while the count runs down to 0.
    fn repeated(&mut self, p: &Planned) {
        let count = self.b.declare_var(I32);
        let init = self.k(p.times);
        self.b.def_var(count, init);
        let body = self.b.create_block();
        let after = self.b.create_block();
        self.b.ins().jump(body, &[]);
        self.b.switch_to_block(body);
        self.word(p.pc, p.w);
        let c = self.b.use_var(count);
        let c = self.b.ins().iadd_imm_s(c, -1);
        self.b.def_var(count, c);
        self.b.ins().brif(c, body, &[], after, &[]);
        self.b.switch_to_block(after);
        let zero = self.k(0);
        self.st(OFF_REPEAT, zero);
    }

    /// One word, as `Cpu::step` runs it (the PC and the step count are
    /// the block's).
    fn word(&mut self, pc: u32, w: u32) -> Next {
        let top = w >> 24;
        match w >> 26 {
            0x00 | 0x07 => self.op(w),
            _ => match top {
                0x36 => {
                    let v = self.k(w & 0xffff);
                    self.st(OFF_MODE + 4 * ((w >> 16) & 0xf) as i32, v);
                }
                0x38..=0x3b => {
                    let r = [reg::P, reg::R10, reg::R13, reg::ACC][(top - 0x38) as usize];
                    let v = self.k(sext(w, 24));
                    self.write_regs(&[(r, v)], None);
                }
                0x3c | 0x3d => {
                    let alu = ((w >> 20) & 0x1f) as u8;
                    if alu != 0 {
                        let out = self.alu(w, alu);
                        self.commit(&[], out);
                    }
                    let c = (w >> 16) & 0xf;
                    let v = self.k(w & 0xffff);
                    self.st(OFF_CTRL + 4 * c as i32, v);
                    if c == 4 {
                        self.st(OFF_REPEAT, v);
                    }
                }
                0x40..=0x7f => {
                    let r = (top & 0x3f) as u8;
                    let v = match view_base(r) {
                        Some((_, 0)) => {
                            let old = self.reg(r);
                            let hi = self.b.ins().band_imm_u(old, 0xffff_0000);
                            self.b.ins().bor_imm_u(hi, (w & 0xffff) as i64)
                        }
                        _ => self.k(w & 0xff_ffff),
                    };
                    self.write_regs(&[(r, v)], None);
                }
                0xbe | 0xfe => return self.cond_word(pc, w),
                0xbf => return self.bf(pc, w),
                _ => self.unknown(w),
            },
        }
        Next::Seq
    }

    fn flag(&mut self, off: i32) -> Value {
        let v = self.b.ins().load(I8, trusted(), self.cpu, off);
        self.b.ins().icmp_imm_u(IntCC::NotEqual, v, 0)
    }

    /// `BE`/`FE`: a conditional jump, call or RAM0 load.
    fn cond_word(&mut self, pc: u32, w: u32) -> Next {
        let next = pc + 1;
        let cond = (w >> 16) & 0xff;
        let taken = match cond & !0x0c {
            0x00 => self.flag(OFF_ZERO),
            0x10 => {
                let n = self.flag(OFF_NEG);
                self.b.ins().bxor_imm_u(n, 1)
            }
            0x20 => {
                let (n, z) = (self.flag(OFF_NEG), self.flag(OFF_ZERO));
                self.b.ins().bor(n, z)
            }
            0xa0 => {
                let (cpu, s) = (self.cpu, self.sigs.steep);
                let v = self.helper(s, h_steep as HSteep as usize, &[cpu]).expect("returns");
                self.b.ins().icmp_imm_u(IntCC::NotEqual, v, 0)
            }
            _ => {
                self.unknown(w);
                return Next::Seq;
            }
        };
        let taken = if w >> 24 == 0xfe { self.b.ins().bxor_imm_u(taken, 1) } else { taken };
        let (t, n) = (self.k(w & 0xffff), self.k(next));
        if cond & 0x0c == 0x04 {
            // A conditional call.
            let (yes, done) = (self.b.create_block(), self.b.create_block());
            self.b.ins().brif(taken, yes, &[], done, &[]);
            self.b.switch_to_block(yes);
            let (cpu, s) = (self.cpu, self.sigs.push);
            self.helper(s, h_push as HPush as usize, &[cpu, n]);
            self.b.ins().jump(done, &[]);
            self.b.switch_to_block(done);
            Next::At(self.b.ins().select(taken, t, n))
        } else if cond & 0x0c != 0 {
            // A conditional load of RAM0 into a register.
            let (yes, done) = (self.b.create_block(), self.b.create_block());
            self.b.ins().brif(taken, yes, &[], done, &[]);
            self.b.switch_to_block(yes);
            let r = ((w >> 9) & 0x3f) as u8;
            let a = self.k(w & 0xff);
            let v = self.ram_read(a);
            self.write_regs(&[(r, v)], None);
            self.b.ins().jump(done, &[]);
            self.b.switch_to_block(done);
            Next::At(n)
        } else {
            Next::At(self.b.ins().select(taken, t, n))
        }
    }

    /// `BF`: loops, jumps, calls, returns.
    fn bf(&mut self, pc: u32, w: u32) -> Next {
        let next = pc + 1;
        match (w >> 16) & 0xff {
            0x00 | 0x10 => {
                let r = if w & 0x0010_0000 != 0 { reg::LC2 } else { reg::LC };
                let c = self.reg(r);
                let c = self.b.ins().iadd_imm_s(c, -1);
                let c = if r == reg::LC { self.b.ins().band_imm_u(c, 0xff) } else { c };
                self.set_reg(r, c);
                let (t, n) = (self.k(w & 0xffff), self.k(next));
                Next::At(self.b.ins().select(c, t, n))
            }
            0x60 => Next::At(self.k(w & 0xffff)),
            0x64 => {
                let (cpu, s, n) = (self.cpu, self.sigs.push, self.k(next));
                self.helper(s, h_push as HPush as usize, &[cpu, n]);
                Next::At(self.k(w & 0xffff))
            }
            0x62 => Next::At(self.jump_target(w)),
            0x66 => {
                let (cpu, s, n) = (self.cpu, self.sigs.push, self.k(next));
                // Push, then read the target, as the interpreter does.
                self.helper(s, h_push as HPush as usize, &[cpu, n]);
                Next::At(self.jump_target(w))
            }
            0x6a => {
                let (cpu, s, wk, n) = (self.cpu, self.sigs.ret, self.k(w), self.k(next));
                Next::At(self.helper(s, h_ret as HRet as usize, &[cpu, wk, n]).expect("returns"))
            }
            0x6e => Next::Seq,
            _ => {
                self.unknown(w);
                Next::Seq
            }
        }
    }

    fn jump_target(&mut self, w: u32) -> Value {
        let v = if w & 0x4000 != 0 {
            self.read_reg((w & 0x3f) as u8)
        } else {
            let a = self.k(w & 0x7f);
            self.ram_read(a)
        };
        self.b.ins().band_imm_u(v, 0xffff)
    }

    // ---- registers

    fn read_reg(&mut self, r: u8) -> Value {
        let r = r & 0x3f;
        if r == reg::FIFO_IN {
            let (cpu, s) = (self.cpu, self.sigs.pop);
            return self.helper(s, h_pop as HPop as usize, &[cpu]).expect("returns");
        }
        match view_base(r) {
            Some((b, 1)) => {
                let v = self.reg(b);
                let e = self.b.ins().ushr_imm_u(v, 23);
                self.b.ins().band_imm_u(e, 0xff)
            }
            Some((b, 2)) => {
                let v = self.reg(b);
                let m = self.b.ins().band_imm_u(v, 0x7f_ffff);
                let s = self.b.ins().ushr_imm_u(v, 31);
                let s = self.b.ins().ishl_imm_u(s, 23);
                self.b.ins().bor(m, s)
            }
            _ => self.reg(r),
        }
    }

    /// `Cpu::write_regs`, its decisions made here (the registers are
    /// constants of the word).
    fn write_regs(&mut self, writes: &[(u8, Value)], alu_acc: Option<Value>) {
        let mut moved_acc = false;
        for &(r, v) in writes {
            let r = r & 0x3f;
            if r == reg::FIFO_OUT {
                let (cpu, s) = (self.cpu, self.sigs.push_out);
                self.helper(s, h_push_out as HPushOut as usize, &[cpu, v]);
                continue;
            }
            let (base, part) = view_base(r).unwrap_or((r, 0));
            let new = match part {
                0 => v,
                _ => {
                    let old = if base == reg::ACC && alu_acc.is_some() { self.ld(OFF_ACC_IN) } else { self.reg(base) };
                    if part == 1 {
                        let kept = self.b.ins().band_imm_u(old, 0x807f_ffff);
                        let e = self.b.ins().band_imm_u(v, 0xff);
                        let e = self.b.ins().ishl_imm_u(e, 23);
                        self.b.ins().bor(kept, e)
                    } else {
                        let kept = self.b.ins().band_imm_u(old, 0x7f80_0000);
                        let m = self.b.ins().band_imm_u(v, 0x7f_ffff);
                        let s = self.b.ins().ushr_imm_u(v, 23);
                        let s = self.b.ins().band_imm_u(s, 1);
                        let s = self.b.ins().ishl_imm_u(s, 31);
                        let x = self.b.ins().bor(kept, m);
                        self.b.ins().bor(x, s)
                    }
                }
            };
            let new = match base {
                reg::LC => self.b.ins().band_imm_u(new, 0xff),
                reg::AR0..=reg::AR3 => self.b.ins().band_imm_u(new, 0xff_ffff),
                reg::BUS => {
                    let bit = self.b.ins().band_imm_u(new, 0x0040_0000);
                    let on = self.b.ins().icmp_imm_u(IntCC::NotEqual, bit, 0);
                    self.b.ins().store(trusted(), on, self.cpu, OFF_SHARED);
                    new
                }
                _ => new,
            };
            if base == reg::ACC {
                moved_acc = true;
                self.st(OFF_ACC_IN, new);
                if alu_acc.is_some() {
                    continue;
                }
            }
            self.set_reg(base, new);
        }
        if let Some(a) = alu_acc {
            self.set_reg(reg::ACC, a);
            if !moved_acc {
                self.st(OFF_ACC_IN, a);
            }
        }
    }

    fn commit(&mut self, writes: &[(u8, Value)], out: AluOut) {
        self.write_regs(writes, out.acc);
        if let Some(p) = out.product {
            self.set_reg(reg::P, p);
        }
    }

    // ---- memory

    fn ram_addr(&mut self, a: Value) -> Value {
        let i = self.b.ins().band_imm_u(a, 0x3ff);
        let i = self.b.ins().uextend(self.ptr, i);
        let off = self.b.ins().ishl_imm_u(i, 2);
        self.b.ins().iadd(self.ram, off)
    }

    fn ram_read(&mut self, a: Value) -> Value {
        let p = self.ram_addr(a);
        self.b.ins().load(I32, trusted(), p, 0)
    }

    fn ram_write(&mut self, a: Value, v: Value) {
        let p = self.ram_addr(a);
        self.b.ins().store(trusted(), v, p, 0);
    }

    fn prog_read(&mut self, a: Value) -> Value {
        if self.prog_len == 0 {
            return self.k(0);
        }
        let i = self.b.ins().band_imm_u(a, 0xffff);
        let inside = self.b.ins().icmp_imm_u(IntCC::UnsignedLessThan, i, self.prog_len as i64);
        let zero = self.k(0);
        let i = self.b.ins().select(inside, i, zero);
        let i = self.b.ins().uextend(self.ptr, i);
        let off = self.b.ins().ishl_imm_u(i, 2);
        let p = self.b.ins().iadd(self.prog, off);
        let v = self.b.ins().load(I32, trusted(), p, 0);
        self.b.ins().select(inside, v, zero)
    }

    fn ext_read(&mut self, a: Value) -> Value {
        let (cpu, bus, s) = (self.cpu, self.bus, self.sigs.ext_read);
        self.helper(s, h_ext_read as HExtRead as usize, &[cpu, bus, a]).expect("returns")
    }

    fn ext_write(&mut self, a: Value, v: Value) {
        let (cpu, bus, s) = (self.cpu, self.bus, self.sigs.ext_write);
        self.helper(s, h_ext_write as HExtWrite as usize, &[cpu, bus, a, v]);
    }

    fn load(&mut self, s: Space, a: Value) -> Value {
        match s {
            Space::Int => self.ram_read(a),
            Space::Ext => self.ext_read(a),
            Space::Prog => self.prog_read(a),
        }
    }

    fn store(&mut self, s: Space, a: Value, v: Value) {
        match s {
            Space::Int => self.ram_write(a, v),
            Space::Ext => self.ext_write(a, v),
            Space::Prog => {}
        }
    }

    fn at(&mut self, ea: Ea, s: Space) -> Value {
        let sum = match ea.base {
            Some(b) => self.b.ins().iadd(ea.index, b),
            None => ea.index,
        };
        self.b.ins().band_imm_u(sum, if s == Space::Int { 0x3ff } else { 0xff_ffff })
    }

    fn base(&mut self, ar: u8) -> Value {
        self.reg(ar & 1)
    }

    /// `Cpu::post`: an indirect operand through `ar`, stepped after.
    fn post(&mut self, ar: u8, m: u32) -> Ea {
        let a = self.reg(ar);
        let next = self.b.ins().iadd_imm_s(a, sext(m & 0x1f, 5) as i32 as i64);
        let new = if m & 0x60 == 0x40 {
            let hi = self.b.ins().band_imm_u(a, !(::mb86234::RING - 1) as i64);
            let lo = self.b.ins().band_imm_u(next, (::mb86234::RING - 1) as i64);
            self.b.ins().bor(hi, lo)
        } else {
            next
        };
        let new = self.b.ins().band_imm_u(new, 0xff_ffff);
        self.set_reg(ar, new);
        let base = (m & 0x20 == 0).then(|| self.base(ar));
        Ea { index: a, base }
    }

    fn relative(&mut self, ar: u8, n: u32) -> Ea {
        let a = self.reg(ar);
        let index = self.b.ins().iadd_imm_s(a, n as i64);
        let base = Some(self.base(ar));
        Ea { index, base }
    }

    fn c9(&mut self, c9: u32, ar: u8) -> Ea {
        if c9 & 0x100 != 0 {
            self.post(ar, c9)
        } else if c9 & 0x80 != 0 {
            self.relative(ar, c9 & 0x7f)
        } else {
            Ea { index: self.k(c9), base: None }
        }
    }

    // ---- the ALU + transfer words

    fn op(&mut self, w: u32) {
        let alu = ((w >> 21) & 0x1f) as u8;
        let kind = (w >> 16) & 0x1f;
        let op1 = (w >> 9) & 0x7f;
        let c9 = w & 0x1ff;
        let out = self.alu(w, alu);
        if w >> 26 == 0x07 && kind >= 0x1c {
            match kind {
                0x1c => {
                    let v = self.read_reg(op1 as u8);
                    let s = if op1 & 0x40 != 0 { Space::Ext } else { Space::Int };
                    let ea = self.c9(c9, reg::AR3);
                    let a = self.at(ea, s);
                    self.commit(&[], out);
                    self.store(s, a, v);
                }
                0x1d => {
                    let ea = self.c9(c9, reg::AR3);
                    let a = self.at(ea, Space::Int);
                    let v = self.ram_read(a);
                    self.commit(&[(op1 as u8, v)], out);
                }
                0x1e => {
                    let v = if op1 & 0x40 != 0 {
                        let ea = self.c9(c9, reg::AR2);
                        let a = self.at(ea, Space::Prog);
                        self.prog_read(a)
                    } else {
                        let ea = self.c9(c9, reg::AR3);
                        let a = self.at(ea, Space::Ext);
                        self.ext_read(a)
                    };
                    self.commit(&[(op1 as u8, v)], out);
                }
                _ => {
                    let v = self.read_reg(c9 as u8);
                    self.commit(&[(op1 as u8, v)], out);
                }
            }
            return;
        }
        let group = kind >> 2;
        let (s1, s9) = match group {
            0 | 1 => (Space::Ext, Space::Int),
            2 => (Space::Int, Space::Ext),
            3 | 4 => (Space::Int, Space::Int),
            5 => (Space::Int, Space::Prog),
            _ => {
                self.unknown(w);
                self.commit(&[], out);
                return;
            }
        };
        let is_mult = w >> 26 == 0;
        let a1 = match kind & 3 {
            0 => self.k(op1),
            1 => {
                let ea = self.relative(reg::AR3, op1);
                self.at(ea, s1)
            }
            3 => {
                let ea = self.post(reg::AR3, op1);
                self.at(ea, s1)
            }
            _ => {
                self.unknown(w);
                self.commit(&[], out);
                return;
            }
        };
        let ea9 = self.c9(c9, reg::AR2);
        let mut a9 = self.at(ea9, s9);
        let mut a1 = a1;
        match group {
            3 => {
                let x = self.b.ins().band_imm_u(a1, 0x1ff);
                a1 = self.b.ins().bor_imm_u(x, 0x200);
                a9 = self.b.ins().band_imm_u(a9, 0x1ff);
            }
            4 => {
                a1 = self.b.ins().band_imm_u(a1, 0x1ff);
                let x = self.b.ins().band_imm_u(a9, 0x1ff);
                a9 = self.b.ins().bor_imm_u(x, 0x200);
            }
            _ => {}
        }
        let v9 = self.load(s9, a9);
        if is_mult {
            let v1 = self.load(s1, a1);
            self.commit(&[(reg::R13, v1), (reg::R10, v9)], out);
            return;
        }
        self.commit(&[], out);
        self.store(s1, a1, v9);
    }

    // ---- flags and floats

    fn set_flags_f(&mut self, v: Value) {
        let m = self.b.ins().band_imm_u(v, 0x7fff_ffff);
        let zero = self.b.ins().icmp_imm_u(IntCC::Equal, m, 0);
        let s = self.b.ins().icmp_imm_s(IntCC::SignedLessThan, v, 0);
        let nz = self.b.ins().bxor_imm_u(zero, 1);
        let neg = self.b.ins().band(s, nz);
        self.b.ins().store(trusted(), zero, self.cpu, OFF_ZERO);
        self.b.ins().store(trusted(), neg, self.cpu, OFF_NEG);
    }

    fn set_flags_i(&mut self, v: Value) {
        let zero = self.b.ins().icmp_imm_u(IntCC::Equal, v, 0);
        let neg = self.b.ins().icmp_imm_s(IntCC::SignedLessThan, v, 0);
        self.b.ins().store(trusted(), zero, self.cpu, OFF_ZERO);
        self.b.ins().store(trusted(), neg, self.cpu, OFF_NEG);
    }

    /// `mb86234::f`: a float word's value, exponent 255 an ordinary binade.
    fn f(&mut self, v: Value) -> Value {
        let e = self.b.ins().band_imm_u(v, 0x7f80_0000);
        let big = self.b.ins().icmp_imm_u(IntCC::Equal, e, 0x7f80_0000);
        let down = self.b.ins().iadd_imm_s(v, -(1 << 23));
        let v = self.b.ins().select(big, down, v);
        let x = self.b.ins().bitcast(F32, MemFlagsData::new(), v);
        let d = self.b.ins().fpromote(F64, x);
        let two = self.b.ins().f64const(2.0);
        let d2 = self.b.ins().fmul(d, two);
        self.b.ins().select(big, d2, d)
    }

    /// `mb86234::word`: `x` rounded to a float word; past single
    /// precision's range (rare) the interpreter's own function.
    fn word_of(&mut self, x: Value) -> Value {
        let r = self.b.ins().fdemote(F32, x);
        let bits = self.b.ins().bitcast(I32, MemFlagsData::new(), r);
        let e = self.b.ins().band_imm_u(bits, 0x7f80_0000);
        let wide = self.b.ins().icmp_imm_u(IntCC::Equal, e, 0x7f80_0000);
        let slow = self.b.create_block();
        let done = self.b.create_block();
        let v = self.b.append_block_param(done, I32);
        self.b.ins().brif(wide, slow, &[], done, &[bits.into()]);
        self.b.switch_to_block(slow);
        self.b.set_cold_block(slow);
        let s = self.sigs.word;
        let w = self.helper(s, h_word as HWord as usize, &[x]).expect("returns");
        self.b.ins().jump(done, &[w.into()]);
        self.b.switch_to_block(done);
        v
    }

    /// A float result: rounded, flags set.
    fn fl(&mut self, x: Value) -> Option<Value> {
        let w = self.word_of(x);
        self.set_flags_f(w);
        Some(w)
    }

    fn int(&mut self, v: Value) -> Option<Value> {
        self.set_flags_i(v);
        Some(v)
    }

    /// `Cpu::alu`: ALU code `code` of word `w`. Reads happen here, before
    /// the word's transfer; writes are the caller's (`commit`).
    fn alu(&mut self, w: u32, code: u8) -> AluOut {
        if code == 0 {
            return AluOut::default();
        }
        let a = self.ld(OFF_ACC_IN);
        let r10 = self.reg(reg::R10);
        let r13 = self.reg(reg::R13);
        let p = self.reg(reg::P);
        let product = |e: &mut Self| {
            let (x, y) = (e.f(r10), e.f(r13));
            let m = e.b.ins().fmul(x, y);
            e.word_of(m)
        };
        let sh = |e: &mut Self| {
            let s = e.reg(reg::SH);
            e.b.ins().band_imm_u(s, 31)
        };
        let acc = |acc: Option<Value>| AluOut { acc, product: None };
        match code {
            1 => {
                let v = self.b.ins().band(a, r10);
                acc(self.int(v))
            }
            2 => {
                let v = self.b.ins().bor(a, r10);
                acc(self.int(v))
            }
            3 => {
                let v = self.b.ins().bxor(a, r10);
                acc(self.int(v))
            }
            4 => {
                let v = self.b.ins().bnot(a);
                acc(self.int(v))
            }
            5 => {
                let (x, y) = (self.f(a), self.f(r10));
                let d = self.b.ins().fsub(x, y);
                let d = self.word_of(d);
                self.set_flags_f(d);
                AluOut::default()
            }
            6 | 7 => {
                let (x, y) = (self.f(a), self.f(r10));
                let v = if code == 6 { self.b.ins().fadd(x, y) } else { self.b.ins().fsub(x, y) };
                acc(self.fl(v))
            }
            8 => AluOut { acc: None, product: Some(product(self)) },
            9 | 10 => {
                let (x, y) = (self.f(a), self.f(p));
                let v = if code == 9 { self.b.ins().fadd(x, y) } else { self.b.ins().fsub(x, y) };
                let acc = self.fl(v);
                AluOut { acc, product: Some(product(self)) }
            }
            11 => {
                let x = self.f(a);
                let v = self.b.ins().fabs(x);
                acc(self.fl(v))
            }
            12 => {
                let (x, y) = (self.f(a), self.f(p));
                let v = self.b.ins().fadd(x, y);
                acc(self.fl(v))
            }
            13 => {
                self.set_flags_f(p);
                AluOut { acc: Some(p), product: Some(product(self)) }
            }
            14 => {
                let x = self.b.ins().fcvt_from_sint(F32, a);
                let x = self.b.ins().fpromote(F64, x);
                acc(self.fl(x))
            }
            15 => {
                let x = self.f(a);
                let m = self.ld(OFF_MODE + 4 * 0xa);
                let m = self.b.ins().band_imm_u(m, 3);
                let t = self.b.ins().trunc(x);
                let r = self.b.ins().nearest(x);
                let v = self.b.ins().select(m, r, t);
                let v = self.b.ins().fcvt_to_sint_sat(I32, v);
                acc(self.int(v))
            }
            16 => {
                let (x, y) = (self.f(a), self.f(r10));
                let v = self.b.ins().fdiv(x, y);
                acc(self.fl(v))
            }
            17 => {
                let x = self.f(a);
                let v = self.b.ins().fneg(x);
                acc(self.fl(v))
            }
            19 | 20 => {
                let (x, y) = (self.f(r13), self.f(r10));
                let v = if code == 19 { self.b.ins().fadd(x, y) } else { self.b.ins().fsub(x, y) };
                acc(self.fl(v))
            }
            22 => {
                let s = sh(self);
                let v = self.b.ins().ushr(a, s);
                acc(self.int(v))
            }
            23 => {
                let s = sh(self);
                let v = self.b.ins().ishl(a, s);
                acc(self.int(v))
            }
            24 => {
                let s = sh(self);
                let v = self.b.ins().sshr(a, s);
                acc(self.int(v))
            }
            26 => {
                let v = self.b.ins().iadd(a, r10);
                acc(self.int(v))
            }
            27 => {
                let v = self.b.ins().isub(a, r10);
                acc(self.int(v))
            }
            _ => {
                self.unknown(w);
                AluOut::default()
            }
        }
    }
}

#[derive(Default, Clone, Copy)]
struct AluOut {
    acc: Option<Value>,
    product: Option<Value>,
}

// The helpers: the interpreter's own methods.
//
// SAFETY (all of them): called only by blocks `Mb86234Jit::run` started,
// with the CPU and the `&mut dyn Bus` it passed, live and distinct.

type HPop = unsafe extern "C" fn(*mut Cpu) -> u32;
type HPushOut = unsafe extern "C" fn(*mut Cpu, u32);
type HExtRead = unsafe extern "C" fn(*mut Cpu, *mut u8, u32) -> u32;
type HExtWrite = unsafe extern "C" fn(*mut Cpu, *mut u8, u32, u32);
type HUnknown = unsafe extern "C" fn(*mut Cpu, u32);
type HPush = unsafe extern "C" fn(*mut Cpu, u32);
type HRet = unsafe extern "C" fn(*mut Cpu, u32, u32) -> u32;
type HSteep = unsafe extern "C" fn(*mut Cpu) -> u32;
type HWord = unsafe extern "C" fn(f64) -> u32;

unsafe extern "C" fn h_pop(c: *mut Cpu) -> u32 {
    let c = unsafe { &mut *c };
    c.fifo_in.pop_front().unwrap_or(0)
}

unsafe extern "C" fn h_push_out(c: *mut Cpu, v: u32) {
    unsafe { &mut *c }.fifo_out.push_back(v);
}

unsafe extern "C" fn h_ext_read(c: *mut Cpu, bus: *mut u8, a: u32) -> u32 {
    let (c, bus) = unsafe { (&mut *c, &mut *(bus as *mut &mut dyn Bus)) };
    c.ext_read(*bus, a)
}

unsafe extern "C" fn h_ext_write(c: *mut Cpu, bus: *mut u8, a: u32, v: u32) {
    let (c, bus) = unsafe { (&mut *c, &mut *(bus as *mut &mut dyn Bus)) };
    c.ext_write(*bus, a, v)
}

unsafe extern "C" fn h_unknown(c: *mut Cpu, w: u32) {
    *unsafe { &mut *c }.unknown.entry(w).or_default() += 1;
}

unsafe extern "C" fn h_push(c: *mut Cpu, a: u32) {
    unsafe { &mut *c }.push(a);
}

/// `BF6A`: the return address, or (an empty stack: an unknown word)
/// `next`.
unsafe extern "C" fn h_ret(c: *mut Cpu, w: u32, next: u32) -> u32 {
    let c = unsafe { &mut *c };
    match c.stack.pop() {
        Some(a) => a,
        None => {
            *c.unknown.entry(w).or_default() += 1;
            next
        }
    }
}

unsafe extern "C" fn h_steep(c: *mut Cpu) -> u32 {
    unsafe { &*c }.funcs.atan_steep() as u32
}

extern "C" fn h_word(x: f64) -> u32 {
    ::mb86234::word(x)
}

#[cfg(test)]
mod tests;
