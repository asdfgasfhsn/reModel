//! Static recompilation of an MB86235 program into Rust.
//!
//! Each executed program word becomes a match arm that calls
//! [`Tgp::exec`](crate::cpu::Tgp::exec) with that word's decoding as a
//! constant. `exec` and its helpers are `#[inline(always)]`, so the compiler
//! specialises every arm: the decode and operand dispatch fold away while the
//! semantics stay those of the interpreter. Words not in `words` fall back to
//! the interpreter.
//!
//! The output is derived from game code: write it under `out/`, never commit
//! it.

use crate::{BranchTarget, Control, Insn, Loc, Transfer};
use std::fmt::Write as _;

fn target(t: &BranchTarget) -> String {
    format!("BranchTarget::{t:?}")
}

fn loc(l: &Loc) -> String {
    match l {
        Loc::Reg(r) => format!("Loc::Reg({r})"),
        Loc::Mem(m) => format!("Loc::Mem({m:?})"),
        Loc::Imm(v) => format!("Loc::Imm({v})"),
        Loc::Ext { disp } => format!("Loc::Ext {{ disp: {disp} }}"),
    }
}

fn control(c: &Control) -> String {
    match c {
        Control::Branch { cond, negate, target: t } => {
            format!("Control::Branch {{ cond: {cond}, negate: {negate}, target: {} }}", target(t))
        }
        Control::Jump(t) => format!("Control::Jump({})", target(t)),
        Control::Call { cond, target: t } => format!("Control::Call {{ cond: {cond:?}, target: {} }}", target(t)),
        other => format!("Control::{other:?}"),
    }
}

fn transfer(t: &Transfer) -> String {
    match t {
        Transfer::None => "Transfer::None".into(),
        Transfer::Move { src, dst } => format!("Transfer::Move {{ src: {}, dst: {} }}", loc(src), loc(dst)),
        Transfer::Double { a, b } => {
            format!("Transfer::Double {{ a: ({}, {}), b: ({}, {}) }}", loc(&a.0), loc(&a.1), loc(&b.0), loc(&b.1))
        }
        Transfer::Unsupported(s) => format!("Transfer::Unsupported({s:?})"),
        Transfer::Control(c) => format!("Transfer::Control({})", control(c)),
    }
}

/// `insn` as a Rust constant expression.
pub fn insn_expr(insn: &Insn) -> String {
    format!(
        "Insn {{ addr: {}, word: {:#x}, format: {}, alu: {:?}, mul: {:?}, transfer: {} }}",
        insn.addr,
        insn.word,
        insn.format,
        insn.alu,
        insn.mul,
        transfer(&insn.transfer)
    )
}

/// A Rust module for `program` with `pub const HASH` (its
/// [`program_hash`](crate::cpu::program_hash)) and a `step` function that
/// runs one word, compiled for the addresses in `words`.
pub fn program_module(program: &[u64], words: &[u32]) -> String {
    let hash = crate::cpu::program_hash(program);
    let mut s = String::new();
    let _ = writeln!(s, "// MB86235 program {hash:#018x}, recompiled by `remodel recomp`. Derived from game");
    let _ = writeln!(s, "// code: generated under out/, never commit it.");
    s.push_str("#[allow(unused_imports)]\n");
    s.push_str(
        "use mb86235::{cpu::{External, Step, Tgp}, AluOp, BranchTarget, Control, Insn, Loc, Mem, MulOp, Transfer};\n\n",
    );
    let _ = writeln!(s, "pub const HASH: u64 = {hash:#018x};\n");
    s.push_str("pub fn step<E: External>(t: &mut Tgp, ext: &mut E) -> Step {\n    match t.pc {\n");
    for &a in words {
        let word = program.get(a as usize).copied().unwrap_or(0);
        let insn = crate::decode(a, word);
        let _ = writeln!(
            s,
            "        {a} => {{\n            const I: Insn = {};\n            t.exec(&I, ext)\n        }}",
            insn_expr(&insn)
        );
    }
    s.push_str("        _ => t.step(ext),\n    }\n}\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_round_trip_through_debug() {
        // A few words of different formats; the generated expression must
        // name every type with a path the generated module imports.
        for w in [0xc700_0200_0000_0000u64, 0x3c04_c20e_c23b_258a, 0xa101_e020_8900_00c7, 0x4084_e2ad_cebf_00be] {
            let e = insn_expr(&crate::decode(0x10, w));
            assert!(e.starts_with("Insn { addr: 16"), "{e}");
            assert!(!e.contains("Reg(") || e.contains("Loc::Reg(") || e.contains("BranchTarget::Reg("), "{e}");
        }
    }
}
