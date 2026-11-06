//! Static recompilation of an ADSP-21062 program into Rust
//! (docs/recompiler.md).
//!
//! Each compiled word becomes a function that runs one interpreter step of
//! that instruction with its decoding as a constant
//! ([`Sharc::run_compiled`](crate::cpu::Sharc::run_compiled)); everything
//! it calls is `#[inline(always)]`, so the compiler specialises each one.
//! The program's `step` dispatches on the address of the instruction about
//! to execute and leaves any other to the interpreter, so correctness never
//! depends on which words were compiled. Words go 256 to a submodule so
//! that rustc compiles them in parallel.
//!
//! The output is derived from game code: write it under `out/`, never commit
//! it.

use crate::compute::{Mr, MulDst};
use crate::{decode, Addr, BitOp, Compute, Count, Flow, Insn, Modifier, Move, MulOp, Op, StackOps, Target};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// Internal memory address program images start at.
pub const BASE: u32 = 0x2_0000;
/// Words per submodule.
const MODULE_WORDS: usize = 256;

fn mr(m: Mr) -> String {
    format!("Mr::{m:?}")
}

fn mul_dst(d: MulDst) -> String {
    match d {
        MulDst::Reg => "MulDst::Reg".into(),
        MulDst::Mr(m) => format!("MulDst::Mr({})", mr(m)),
    }
}

fn mul_op(op: MulOp) -> String {
    match op {
        MulOp::Mul { acc, sub, dst, xs, ys, frac, round } => format!(
            "MulOp::Mul {{ acc: {}, sub: {sub}, dst: {}, xs: {xs}, ys: {ys}, frac: {frac}, round: {round} }}",
            acc.map_or("None".into(), |m| format!("Some({})", mr(m))),
            mul_dst(dst)
        ),
        MulOp::Sat { src, dst, signed, frac } => {
            format!("MulOp::Sat {{ src: {}, dst: {}, signed: {signed}, frac: {frac} }}", mr(src), mul_dst(dst))
        }
        MulOp::Rnd { src, dst, signed } => {
            format!("MulOp::Rnd {{ src: {}, dst: {}, signed: {signed} }}", mr(src), mul_dst(dst))
        }
        MulOp::Clear(m) => format!("MulOp::Clear({})", mr(m)),
        MulOp::FMul => "MulOp::FMul".into(),
    }
}

/// `c` as a Rust constant expression.
pub fn compute_expr(c: &Compute) -> String {
    match *c {
        Compute::None => "Compute::None".into(),
        Compute::Alu { op, rn, rx, ry } => {
            format!("Compute::Alu {{ op: AluOp::{op:?}, rn: {rn}, rx: {rx}, ry: {ry} }}")
        }
        Compute::AddSub { float, ra, rs, rx, ry } => {
            format!("Compute::AddSub {{ float: {float}, ra: {ra}, rs: {rs}, rx: {rx}, ry: {ry} }}")
        }
        Compute::Mul { op, rn, rx, ry } => {
            format!("Compute::Mul {{ op: {}, rn: {rn}, rx: {rx}, ry: {ry} }}", mul_op(op))
        }
        Compute::Shift { op, rn, rx, ry } => {
            format!("Compute::Shift {{ op: ShiftOp::{op:?}, rn: {rn}, rx: {rx}, ry: {ry} }}")
        }
        Compute::MrMove { to_mr, mr, rk } => format!("Compute::MrMove {{ to_mr: {to_mr}, mr: {mr}, rk: {rk} }}"),
        Compute::Multi { mul, alu, rm, ra, regs } => format!(
            "Compute::Multi {{ mul: {}, alu: AluOp::{alu:?}, rm: {rm}, ra: {ra}, regs: {:?} }}",
            mul_op(mul),
            regs
        ),
        Compute::MultiAddSub { float, rm, ra, rs, regs } => {
            format!("Compute::MultiAddSub {{ float: {float}, rm: {rm}, ra: {ra}, rs: {rs}, regs: {regs:?} }}")
        }
        Compute::Unknown(v) => format!("Compute::Unknown({v:#x})"),
    }
}

fn addr(a: &Addr) -> String {
    let m = match a.modifier {
        Modifier::M(m) => format!("Modifier::M({m})"),
        Modifier::Imm(v) => format!("Modifier::Imm({v})"),
    };
    format!("Addr {{ i: {}, modifier: {m}, post: {} }}", a.i, a.post)
}

fn mv(m: &Move) -> String {
    format!("Move {{ addr: {}, write: {}, reg: {} }}", addr(&m.addr), m.write, m.reg)
}

fn target(t: &Target) -> String {
    match *t {
        Target::Abs(a) => format!("Target::Abs({a:#x})"),
        Target::Rel(d) => format!("Target::Rel({d})"),
        Target::Indirect { i, m } => format!("Target::Indirect {{ i: {i}, m: {m} }}"),
    }
}

fn op_expr(op: &Op) -> String {
    let c = compute_expr;
    match op {
        Op::Dual { compute, dm, pm } => {
            format!("Op::Dual {{ compute: {}, dm: {}, pm: {} }}", c(compute), mv(dm), mv(pm))
        }
        Op::Compute { cond, compute } => format!("Op::Compute {{ cond: {cond}, compute: {} }}", c(compute)),
        Op::Transfer { cond, compute, mv: m } => {
            format!("Op::Transfer {{ cond: {cond}, compute: {}, mv: {} }}", c(compute), mv(m))
        }
        Op::RegMove { cond, compute, src, dst } => {
            format!("Op::RegMove {{ cond: {cond}, compute: {}, src: {src}, dst: {dst} }}", c(compute))
        }
        Op::ShiftImm { cond, op, rn, rx, imm, mv: m } => format!(
            "Op::ShiftImm {{ cond: {cond}, op: ShiftOp::{op:?}, rn: {rn}, rx: {rx}, imm: {imm}, mv: {} }}",
            m.as_ref().map_or("None".into(), |m| format!("Some({})", mv(m)))
        ),
        Op::Modify { cond, compute, i, m } => {
            format!("Op::Modify {{ cond: {cond}, compute: {}, i: {i}, m: {m} }}", c(compute))
        }
        Op::Branch { cond, call, target: t, delayed, loop_abort, clear_int, compute, else_ } => format!(
            "Op::Branch {{ cond: {cond}, call: {call}, target: {}, delayed: {delayed}, loop_abort: {loop_abort}, \
             clear_int: {clear_int}, compute: {}, else_: {else_} }}",
            target(t),
            c(compute)
        ),
        Op::JumpElseMove { cond, target: t, compute, mv: m } => format!(
            "Op::JumpElseMove {{ cond: {cond}, target: {}, compute: {}, mv: {} }}",
            target(t),
            c(compute),
            mv(m)
        ),
        Op::Return { cond, rti, delayed, loop_reentry, compute, else_ } => format!(
            "Op::Return {{ cond: {cond}, rti: {rti}, delayed: {delayed}, loop_reentry: {loop_reentry}, compute: {}, \
             else_: {else_} }}",
            c(compute)
        ),
        Op::Do { count, term, end } => {
            let count = match count {
                None => "None".into(),
                Some(Count::Imm(n)) => format!("Some(Count::Imm({n}))"),
                Some(Count::Ureg(r)) => format!("Some(Count::Ureg({r}))"),
            };
            format!("Op::Do {{ count: {count}, term: {term}, end: {end:#x} }}")
        }
        Op::Direct { pm, write, reg, addr } => {
            format!("Op::Direct {{ pm: {pm}, write: {write}, reg: {reg}, addr: {addr:#x} }}")
        }
        Op::IndirectImm { write, reg, i, offset } => {
            format!("Op::IndirectImm {{ write: {write}, reg: {reg}, i: {i}, offset: {offset} }}")
        }
        Op::StoreImm { addr: a, data } => format!("Op::StoreImm {{ addr: {}, data: {data:#x} }}", addr(a)),
        Op::LoadImm { reg, data } => format!("Op::LoadImm {{ reg: {reg}, data: {data:#x} }}"),
        Op::Bit { op, sreg, data } => {
            let op = match op {
                BitOp::Set => "Set",
                BitOp::Clr => "Clr",
                BitOp::Tgl => "Tgl",
                BitOp::Tst => "Tst",
                BitOp::Xor => "Xor",
            };
            format!("Op::Bit {{ op: BitOp::{op}, sreg: {sreg}, data: {data:#x} }}")
        }
        Op::ModifyImm { i, data, bitrev } => format!("Op::ModifyImm {{ i: {i}, data: {data}, bitrev: {bitrev} }}"),
        Op::Stacks(s) => {
            let StackOps { push_loop, pop_loop, push_sts, pop_sts, push_pcstk, pop_pcstk, flush_cache } = s;
            format!(
                "Op::Stacks(StackOps {{ push_loop: {push_loop}, pop_loop: {pop_loop}, push_sts: {push_sts}, \
                 pop_sts: {pop_sts}, push_pcstk: {push_pcstk}, pop_pcstk: {pop_pcstk}, flush_cache: {flush_cache} }})"
            )
        }
        Op::Nop => "Op::Nop".into(),
        Op::Idle => "Op::Idle".into(),
        Op::Idle16 => "Op::Idle16".into(),
        Op::CJump { target: t } => format!("Op::CJump {{ target: {} }}", target(t)),
        Op::RFrame => "Op::RFrame".into(),
        Op::Unknown => "Op::Unknown".into(),
    }
}

/// `insn` as a Rust constant expression.
pub fn insn_expr(insn: &Insn) -> String {
    format!("Insn {{ addr: {:#x}, word: {:#x}, op: {} }}", insn.addr, insn.word, op_expr(&insn.op))
}

/// What recursive descent from the reset and interrupt vectors finds in a
/// program image (48-bit words loaded at [`BASE`]).
#[derive(Default)]
pub struct Discovery {
    /// Words reached as instructions.
    pub code: BTreeSet<u32>,
    /// Branch, call and loop targets, with how they were reached.
    pub labels: BTreeMap<u32, &'static str>,
    /// Indirect branches and calls (targets unknown statically).
    pub indirect: BTreeSet<u32>,
}

/// Follow control flow from the reset vector (0x20004: execution starts
/// at 0x20005 after the boot interrupt) and every interrupt vector.
/// Immediates loaded or stored that point into the image past the vectors
/// are taken as code pointers: the Model 2B programs build their command
/// dispatch tables that way, and their indirect jumps go through them.
pub fn discover(words: &[u64]) -> Discovery {
    let end = BASE + words.len() as u32;
    let at = |a: u32| -> Option<Insn> { (BASE..end).contains(&a).then(|| decode(a, words[(a - BASE) as usize])) };
    let mut d = Discovery::default();
    let mut work: Vec<u32> = (0..32).map(|k| BASE + 4 * k).collect();
    work.push(BASE + 5);
    d.labels.insert(BASE + 5, "reset");
    // Words explored (followed on from); delay slots are code without
    // being followed, until something else reaches them.
    let mut explored = BTreeSet::new();
    while let Some(a) = work.pop() {
        let Some(insn) = at(a) else { continue };
        if !explored.insert(a) {
            continue;
        }
        d.code.insert(a);
        // The two words after a delayed transfer run before it lands.
        let slots = |d: &mut Discovery| {
            for s in [a + 1, a + 2] {
                if at(s).is_some() {
                    d.code.insert(s);
                }
            }
        };
        let target = |t: Option<u32>, kind: &'static str, d: &mut Discovery, work: &mut Vec<u32>| match t {
            Some(t) => {
                d.labels.entry(t).or_insert(kind);
                work.push(t);
            }
            None => {
                d.indirect.insert(a);
            }
        };
        if let Op::StoreImm { data, .. } | Op::LoadImm { data, .. } = insn.op {
            if (BASE + 0x80..end).contains(&data) {
                d.labels.entry(data).or_insert("table");
                work.push(data);
            }
        }
        match insn.flow() {
            Flow::Next | Flow::Idle => work.push(a + 1),
            Flow::Jump { target: t, delayed } => {
                target(t, "jump", &mut d, &mut work);
                if delayed {
                    slots(&mut d);
                }
            }
            Flow::Branch { target: t, delayed } | Flow::Call { target: t, delayed } => {
                let kind = if matches!(insn.flow(), Flow::Call { .. }) { "call" } else { "branch" };
                target(t, kind, &mut d, &mut work);
                if delayed {
                    slots(&mut d);
                    work.push(a + 3);
                } else {
                    work.push(a + 1);
                }
            }
            Flow::Return { cond, delayed } => {
                if delayed {
                    slots(&mut d);
                }
                if cond {
                    work.push(if delayed { a + 3 } else { a + 1 });
                }
            }
            Flow::Do { end } => {
                d.labels.entry(end + 1).or_insert("loop exit");
                work.push(a + 1);
                work.push(end + 1);
            }
            Flow::Invalid => {}
        }
    }
    d
}

/// Static checks for self-modifying code, as messages (empty if none
/// apply): compiled words that store through the PM bus, which is how a
/// program would write instructions. Their targets are DAG2 addresses,
/// known only at run time, so the core watches every compiled word
/// (`Sharc::watch_code`) and an overwritten one goes back to the
/// interpreter; this only says where to look.
pub fn check(words: &[u64], code: &BTreeSet<u32>) -> Vec<String> {
    let pm_writes: Vec<u32> = code
        .iter()
        .copied()
        .filter(|&a| {
            let Some(&w) = words.get((a - BASE) as usize) else { return false };
            match decode(a, w).op {
                Op::Dual { pm, .. } => pm.write,
                Op::Transfer { mv, .. } | Op::JumpElseMove { mv, .. } => mv.write && mv.addr.pm(),
                Op::ShiftImm { mv: Some(mv), .. } => mv.write && mv.addr.pm(),
                Op::Direct { pm, write, .. } => pm && write,
                Op::IndirectImm { write, i, .. } => write && i >= 8,
                Op::StoreImm { addr, .. } => addr.pm(),
                _ => false,
            }
        })
        .collect();
    let mut notes = Vec::new();
    if !pm_writes.is_empty() {
        notes.push(format!(
            "{} compiled words store through the PM bus (first {:#07x}); writes into compiled code are caught at run time",
            pm_writes.len(),
            pm_writes[0]
        ));
    }
    notes
}

/// A Rust module for `program` (48-bit words from [`BASE`]) with `pub const
/// HASH` (its [`program_hash`](crate::cpu::program_hash)), `IMAGE` (the
/// program), `WORDS` (the compiled addresses), a `step` that runs one
/// instruction like [`Sharc::step`](crate::cpu::Sharc::step) and a `run`
/// that runs them like [`Sharc::run_bus`](crate::cpu::Sharc::run_bus),
/// compiled for the addresses in `words`.
pub fn program_module(program: &[u64], words: &BTreeSet<u32>) -> String {
    let hash = crate::cpu::program_hash(program);
    let words: Vec<u32> =
        words.iter().copied().filter(|&a| a >= BASE && ((a - BASE) as usize) < program.len()).collect();
    let mut s = String::new();
    let _ = writeln!(s, "// ADSP-21062 program {hash:#018x}, recompiled by `remodel recomp`. Derived from");
    let _ = writeln!(s, "// game code: generated under out/, never commit it.");
    s.push_str("#[allow(unused_imports)]\n");
    s.push_str(
        "use adsp2106x::{compute::{MfRegs, Mr, MulDst}, cpu::{Bus, Sharc, Step}, AluOp, Addr, BitOp, Compute, \
         Count, Insn, Modifier, Move, MulOp, Op, ShiftOp, StackOps, Target};\n\n",
    );
    let _ = writeln!(s, "pub const HASH: u64 = {hash:#018x};\n");
    s.push_str("/// The image compiled: 48-bit words from 0x20000.\npub const IMAGE: &[u64] = &[\n");
    for chunk in program.chunks(8) {
        let line: Vec<String> = chunk.iter().map(|w| format!("{w:#x}")).collect();
        let _ = writeln!(s, "    {},", line.join(", "));
    }
    s.push_str("];\n\n");
    s.push_str("/// The compiled words.\npub const WORDS: &[u32] = &[\n");
    for chunk in words.chunks(12) {
        let line: Vec<String> = chunk.iter().map(|a| format!("{a:#x}")).collect();
        let _ = writeln!(s, "    {},", line.join(", "));
    }
    s.push_str("];\n\n");
    s.push_str(
        "/// Instructions until the bus's `after_step` says to stop, as `Sharc::run_bus` runs them.\n\
         pub fn run<B: Bus>(s: &mut Sharc, ext: &mut B) -> Step {\n    loop {\n        let r = step(s, ext);\n        \
         if r != Step::Ran || !ext.after_step(s) {\n            return r;\n        }\n    }\n}\n\n",
    );
    s.push_str("/// One instruction, as `Sharc::step` runs it.\n");
    s.push_str(
        "#[inline(always)]\npub fn step<B: Bus>(s: &mut Sharc, ext: &mut B) -> Step {\n    match s.compiled_pc() {\n",
    );
    for (k, chunk) in words.chunks(MODULE_WORDS).enumerate() {
        for &a in chunk {
            let _ = writeln!(s, "        {a:#x} => m{k}::w{a:x}(s, ext),");
        }
    }
    s.push_str("        _ => s.step_uncompiled(ext),\n    }\n}\n");
    for (k, chunk) in words.chunks(MODULE_WORDS).enumerate() {
        let _ = writeln!(s, "\nmod m{k} {{\n    use super::*;");
        for &a in chunk {
            let insn = decode(a, program[(a - BASE) as usize]);
            let index = crate::cpu::word_index(a).expect("compiled words are internal");
            let _ = writeln!(
                s,
                "\n    // {insn}\n    #[inline(never)]\n    pub fn w{a:x}<B: Bus>(s: &mut Sharc, ext: &mut B) -> Step {{\n        \
                 const I: Insn = {};\n        s.run_compiled(&I, {index}, ext)\n    }}",
                insn_expr(&insn)
            );
        }
        s.push_str("}\n");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_name_their_types() {
        // A dual move with a multifunction compute, a branch, a DO and a
        // type 20: every type in the expression is named by a path the
        // generated module imports.
        for w in [0x063e_0002_0080u64, 0x0b3e_0000_0000, 0x503e_0000_0000, 0x013e_0002_9000, 0x0712_0000_0000] {
            let e = insn_expr(&decode(0x2_0010, w));
            assert!(e.starts_with("Insn { addr: 0x20010"), "{e}");
            assert!(e.contains("op: Op::"), "{e}");
        }
    }

    #[test]
    fn module_dispatches_compiled_words() {
        let mut p = vec![0u64; 0x81];
        p[0x05] = 0x063e_0002_0080; // JUMP 0x20080
        p[0x80] = 0x013e_0002_9000; // R0 = R0 + 1
        let d = discover(&p);
        assert!(d.code.contains(&0x2_0080));
        let m = program_module(&p, &d.code);
        assert!(m.contains("0x20080 => m0::w20080(s, ext),"));
        assert!(m.contains("_ => s.step_uncompiled(ext),"));
    }

    #[test]
    fn delay_slots_reached_later_are_followed() {
        let mut p = vec![0u64; 0x90];
        p[0x05] = 0x063e_0002_0080; // JUMP 0x20080
        p[0x80] = 0x0b3e_0000_0000; // RTI: an interrupt vector-like stop
                                    // 0x20081: JUMP 0x20088 (DB): 0x20082-3 are its slots.
        p[0x81] = 0x063e_0402_0088;
        p[0x88] = 0x063e_0002_0083; // JUMP 0x20083: a slot, then on
        p[0x83] = 0x013e_0002_9000; // R0 = R0 + 1
        p[0x84] = 0x0a3e_0000_0000; // RTS
        let mut p2 = p.clone();
        p2[0x80] = 0x013e_0002_9000;
        let d = discover(&p2);
        assert!(d.code.contains(&0x2_0083), "the slot");
        assert!(d.code.contains(&0x2_0084), "followed from the slot once jumped to");
    }
}
