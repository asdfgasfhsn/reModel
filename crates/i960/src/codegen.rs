//! Static recompilation of i960 code into Rust.
//!
//! Code is cut into basic blocks, and each block becomes a Rust function
//! that runs its instructions in order. Common instructions (loads and
//! stores, moves, integer arithmetic, compares, branches, calls) are
//! translated into straight Rust on the register file; the rest call the
//! interpreter's [`Cpu::exec`](crate::cpu::Cpu::exec) with the instruction
//! as a constant, so nothing is decoded at run time.
//!
//! Timing is kept at instruction granularity: after every instruction the
//! block calls [`Host::retire`](crate::cpu::Host::retire), exactly where the
//! interpreter's caller would run its between-instruction work, and returns
//! early if that asks it to or an interrupt is due. A recompiled run is
//! therefore indistinguishable from an interpreted one. Addresses that
//! aren't a block start (such as an interrupt's return point) run on the
//! interpreter until they reach one.
//!
//! The output is derived from game code: write it under `out/`, never commit
//! it.

use crate::lower::{lower, Alu, Op, Src, Width};
use crate::{Flow, Insn, MemRef, Operand};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// FNV-1a hash of the words in `[0, end)`, identifying a program image.
pub fn image_hash(fetch: impl Fn(u32) -> Option<u32>, end: u32) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for a in (0..end).step_by(4) {
        h = (h ^ fetch(a).unwrap_or(0) as u64).wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// Block start addresses: `extra` (entry points the caller knows about,
/// e.g. indirect targets), branch and call targets, the instruction after
/// any control transfer, and anything not reached by falling through.
pub fn leaders(insns: &BTreeMap<u32, Insn>, extra: &BTreeSet<u32>) -> BTreeSet<u32> {
    let mut l: BTreeSet<u32> = extra.iter().copied().filter(|a| insns.contains_key(a)).collect();
    let mut fall_into = BTreeSet::new();
    for i in insns.values() {
        match i.flow {
            Flow::Next | Flow::CondFault => {
                fall_into.insert(i.next());
            }
            Flow::Jump(t) | Flow::Call(t) | Flow::Link(t) => {
                l.insert(t);
                l.insert(i.next());
            }
            Flow::Branch(t) => {
                l.insert(t);
                l.insert(i.next());
                fall_into.insert(i.next());
            }
            _ => {
                l.insert(i.next());
            }
        }
    }
    l.extend(insns.keys().filter(|a| !fall_into.contains(a)));
    l.retain(|a| insns.contains_key(a));
    l
}

fn src(s: Src) -> String {
    match s {
        Src::Reg(r) => format!("c.r[{r}]"),
        Src::Lit(v) => format!("{v:#x}u32"),
    }
}

fn ea(m: &MemRef) -> String {
    let base = |r: u8, disp: u32| {
        if disp == 0 {
            format!("c.r[{r}]")
        } else {
            format!("c.r[{r}].wrapping_add({disp:#x})")
        }
    };
    let index = |i: u8, scale: u8| {
        if scale == 1 {
            format!("c.r[{i}]")
        } else {
            format!("c.r[{i}].wrapping_mul({scale})")
        }
    };
    match *m {
        MemRef::Abs(a) | MemRef::IpRel { target: a } => format!("{a:#x}u32"),
        MemRef::Base { abase, disp } => base(abase, disp),
        MemRef::Index { disp, index: i, scale } => format!("{disp:#x}u32.wrapping_add({})", index(i, scale)),
        MemRef::BaseIndex { abase, disp, index: i, scale } => {
            format!("{}.wrapping_add({})", base(abase, disp), index(i, scale))
        }
    }
}

fn operand_expr(o: &Operand) -> String {
    match o {
        Operand::Mem(m) => format!("Operand::Mem(MemRef::{m:?})"),
        o => format!("Operand::{o:?}"),
    }
}

/// `insn` as a constant [`InsnRef`](crate::InsnRef) expression.
pub fn insn_expr(i: &Insn) -> String {
    let ops: Vec<String> = i.operands.iter().map(operand_expr).collect();
    format!(
        "InsnRef {{ addr: {:#x}, size: {}, word: {:#010x}, mnemonic: {:?}, operands: &[{}], flow: Flow::{:?} }}",
        i.addr,
        i.size,
        i.word,
        i.mnemonic,
        ops.join(", "),
        i.flow
    )
}

/// Rust statements for `i`, or `None` to leave it to the interpreter.
/// Statements that transfer control set `c.ip` themselves.
fn translate(i: &Insn) -> Option<String> {
    Some(rust(&lower(i)?, i.addr))
}

/// `op` (the instruction at `addr`) as Rust statements.
fn rust(op: &Op, addr: u32) -> String {
    let fault = format!("map_err(|kind| Stop::Fault {{ kind, ip: {addr:#x} }})?");
    let bit = |e: String| format!("(1u32 << ({e} & 31))");
    let dst3 = |d: u8, e: String| format!("c.r[{d}] = {e};");
    match *op {
        Op::Jump(t) => format!("c.ip = {t:#x};"),
        Op::Call { target, next } => format!("c.call(h, {target:#x}, {next:#x});"),
        Op::Link { target, next } => format!("c.r[30] = {next:#x}; c.ip = {target:#x};"),
        Op::Branch { mask, target, next } => {
            format!("c.ip = if c.cond({mask}) {{ {target:#x} }} else {{ {next:#x} }};")
        }
        Op::Ret => format!("c.ret(h).{fault};"),
        Op::Test { mask, dst } => format!("c.r[{dst}] = c.cond({mask}) as u32;"),
        Op::BitBranch { bit: a, src: b, set, target, next } => format!(
            "let set = {} & (1u32 << ({} & 31)) != 0; c.set_cc(if set {{ 2 }} else {{ 0 }}); \
             c.ip = if set == {set} {{ {target:#x} }} else {{ {next:#x} }};",
            src(b),
            src(a)
        ),
        Op::CmpBranch { signed, a, b, mask, target, next } => {
            let take = if signed {
                format!("c.compare({} as i32, {} as i32);", src(a), src(b))
            } else {
                format!("c.compare({}, {});", src(a), src(b))
            };
            format!("{take} c.ip = if c.cond({mask}) {{ {target:#x} }} else {{ {next:#x} }};")
        }
        Op::JumpInd(m) => format!("c.ip = {};", ea(&m)),
        Op::LinkInd { ea: m, dst, next } => format!("let a = {}; c.r[{dst}] = {next:#x}; c.ip = a;", ea(&m)),
        Op::CallInd { ea: m, next } => format!("let a = {}; c.call(h, a, {next:#x});", ea(&m)),
        Op::Lda { ea: m, dst } => format!("c.r[{dst}] = {};", ea(&m)),
        Op::LoadNarrow { width, signed, ea: m, dst } => {
            let m = ea(&m);
            match (width, signed) {
                (Width::Byte, false) => format!("c.r[{dst}] = h.read8({m}) as u32;"),
                (Width::Byte, true) => format!("c.r[{dst}] = h.read8({m}) as i8 as i32 as u32;"),
                (Width::Short, false) => format!("c.r[{dst}] = h.read16({m}) as u32;"),
                (Width::Short, true) => format!("c.r[{dst}] = h.read16({m}) as i16 as i32 as u32;"),
            }
        }
        Op::StoreNarrow { width: Width::Byte, ea: m, src: r } => format!("h.write8({}, c.r[{r}] as u8);", ea(&m)),
        Op::StoreNarrow { width: Width::Short, ea: m, src: r } => format!("h.write16({}, c.r[{r}] as u16);", ea(&m)),
        Op::Load { n, ea: m, dst: r } | Op::Store { n, ea: m, src: r } => {
            let mut s = format!("let a = {};", ea(&m));
            for k in 0..n as usize {
                let addr = if k == 0 { "a".to_string() } else { format!("a.wrapping_add({})", 4 * k) };
                if matches!(op, Op::Load { .. }) {
                    let _ = write!(s, " c.r[{}] = h.read32({addr});", (r as usize + k) & 31);
                } else {
                    let _ = write!(s, " h.write32({addr}, c.r[{}]);", (r as usize + k) & 31);
                }
            }
            s
        }
        Op::Alu { op, a, b, dst } => {
            let (s1, s2) = (src(a), src(b));
            dst3(
                dst,
                match op {
                    Alu::NotBit => format!("{s2} ^ {}", bit(s1)),
                    Alu::And => format!("{s2} & {s1}"),
                    Alu::AndNot => format!("{s2} & !{s1}"),
                    Alu::SetBit => format!("{s2} | {}", bit(s1)),
                    Alu::NotAnd => format!("!{s2} & {s1}"),
                    Alu::Xor => format!("{s2} ^ {s1}"),
                    Alu::Or => format!("{s2} | {s1}"),
                    Alu::Nor => format!("!({s2} | {s1})"),
                    Alu::Xnor => format!("!({s2} ^ {s1})"),
                    Alu::OrNot => format!("{s2} | !{s1}"),
                    Alu::ClrBit => format!("{s2} & !{}", bit(s1)),
                    Alu::NotOr => format!("!{s2} | {s1}"),
                    Alu::Nand => format!("!({s2} & {s1})"),
                    Alu::Add => format!("{s2}.wrapping_add({s1})"),
                    Alu::Sub => format!("{s2}.wrapping_sub({s1})"),
                    Alu::Shro => format!("{{ let n = {s1}; if n >= 32 {{ 0 }} else {{ {s2} >> n }} }}"),
                    Alu::Shri => format!("(({s2} as i32) >> {s1}.min(31)) as u32"),
                    Alu::Shl => format!("{{ let n = {s1}; if n >= 32 {{ 0 }} else {{ {s2} << n }} }}"),
                    Alu::Rotate => format!("{s2}.rotate_left({s1} & 31)"),
                    Alu::Mul => format!("{s2}.wrapping_mul({s1})"),
                },
            )
        }
        Op::Not { a, dst } => format!("c.r[{dst}] = !{};", src(a)),
        Op::Compare { signed: false, a, b } => format!("c.compare({}, {});", src(a), src(b)),
        Op::Compare { signed: true, a, b } => format!("c.compare({} as i32, {} as i32);", src(a), src(b)),
        Op::ChkBit { bit: a, src: b } => {
            format!("c.set_cc(if {} & {} != 0 {{ 2 }} else {{ 0 }});", src(b), bit(src(a)))
        }
        Op::Mov { n, src: s, dst: d } => {
            let (n, d) = (n as usize, d as usize);
            match s {
                Src::Reg(s) => {
                    let s = s as usize;
                    let t: Vec<String> = (0..n).map(|k| format!("c.r[{}]", (s + k) & 31)).collect();
                    let dsts: Vec<String> = (0..n).map(|k| format!("c.r[{}]", (d + k) & 31)).collect();
                    if n == 1 {
                        format!("{} = {};", dsts[0], t[0])
                    } else {
                        format!("let t = [{}]; [{}] = t;", t.join(", "), dsts.join(", "))
                    }
                }
                Src::Lit(v) => {
                    let mut s = format!("c.r[{d}] = {v:#x};");
                    for k in 1..n {
                        let _ = write!(s, " c.r[{}] = 0;", (d + k) & 31);
                    }
                    s
                }
            }
        }
    }
}

/// Blocks per submodule of a program module. rustc never splits a module
/// across codegen units, so a whole program in one module compiles on one
/// thread; in submodules it compiles on as many as the crate's codegen
/// units allow.
const BLOCKS_PER_MODULE: usize = 512;

/// A Rust module for the code in `insns` (ROM image `hash`), with
/// `pub const HASH` and `pub fn blocks<H: Host>()` listing each block's
/// start address and function. `extra` are entry points to start blocks at
/// besides the ones found from control flow.
pub fn program_module(insns: &BTreeMap<u32, Insn>, extra: &BTreeSet<u32>, hash: u64) -> String {
    let leaders = leaders(insns, extra);
    let mut s = String::new();
    let _ = writeln!(s, "// i960 program {hash:#018x}, recompiled by `remodel recomp`. Derived from game");
    let _ = writeln!(s, "// code: generated under out/, never commit it.");
    s.push_str(
        "#[allow(unused_imports)]\nuse i960::{cpu::{Bus, Cpu, Host, Stop}, Flow, InsnRef, MemRef, Operand};\n\n",
    );
    let _ = writeln!(s, "pub const HASH: u64 = {hash:#018x};\n");
    let _ = writeln!(s, "pub type Block<H> = fn(&mut Cpu, &mut H) -> Result<(), Stop>;\n");
    let (mut translated, mut total) = (0, 0);
    let order: Vec<u32> = leaders.iter().copied().collect();
    let chunks: Vec<&[u32]> = order.chunks(BLOCKS_PER_MODULE).collect();
    for (m, chunk) in chunks.iter().enumerate() {
        let _ = writeln!(s, "mod m{m:03} {{\n#[allow(unused_imports)]\nuse super::*;\n");
        for &start in chunk.iter() {
            let _ = writeln!(
                s,
                "#[inline(never)]\nfn b{start:08x}<H: Host>(c: &mut Cpu, h: &mut H) -> Result<(), Stop> {{"
            );
            let mut pc = start;
            loop {
                let i = &insns[&pc];
                let next = i.next();
                let _ = writeln!(s, "    // {i}");
                let _ = writeln!(s, "    h.at({pc:#x});");
                total += 1;
                let code = translate(i);
                let fallback = code.is_none();
                match code {
                    Some(code) => {
                        translated += 1;
                        let _ = writeln!(s, "    {{ {code} }}");
                    }
                    None => {
                        let _ = writeln!(s, "    {{ const I: InsnRef<'static> = {};", insn_expr(i));
                        let _ =
                            writeln!(s, "      c.exec(h, &I).map_err(|kind| Stop::Fault {{ kind, ip: {pc:#x} }})?; }}");
                    }
                }
                s.push_str("    c.icount += 1;\n");
                let ends = !matches!(i.flow, Flow::Next | Flow::CondFault);
                if fallback && !ends {
                    // Some "sequential" instructions can still move the IP: an
                    // IAC reinit, or an interrupt taken by `modpc`.
                    let _ = writeln!(s, "    if c.ip != {next:#x} {{ h.retire(c); return Ok(()); }}");
                }
                if ends {
                    s.push_str("    h.retire(c);\n    Ok(())\n}\n\n");
                    break;
                }
                if !insns.contains_key(&next) || leaders.contains(&next) {
                    let _ = writeln!(s, "    h.retire(c);\n    c.ip = {next:#x};\n    Ok(())\n}}\n");
                    break;
                }
                let _ = writeln!(s, "    if !h.retire(c) || c.interrupt_due() {{ c.ip = {next:#x}; return Ok(()); }}");
                pc = next;
            }
        }
        s.push_str("pub(super) fn blocks<H: Host>(v: &mut Vec<(u32, Block<H>)>) {\n");
        for &start in chunk.iter() {
            let _ = writeln!(s, "    v.push(({start:#x}, b{start:08x}::<H>));");
        }
        s.push_str("}\n}\n\n");
    }
    let _ = writeln!(s, "// {translated} of {total} instructions translated, the rest run through `Cpu::exec`.\n");
    let _ = writeln!(
        s,
        "pub fn blocks<H: Host>() -> Vec<(u32, Block<H>)> {{\n    let mut v = Vec::with_capacity({});",
        leaders.len()
    );
    for m in 0..chunks.len() {
        let _ = writeln!(s, "    m{m:03}::blocks(&mut v);");
    }
    s.push_str("    v\n}\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode;

    fn prog(words: &[u32]) -> BTreeMap<u32, Insn> {
        let fetch = |a: u32| words.get(a as usize / 4).copied();
        (0..words.len() as u32 * 4).step_by(4).map(|a| (a, decode(a, fetch))).collect()
    }

    #[test]
    fn blocks_split_at_branches_and_targets() {
        // 0: mov 1, g0   4: be 0xc   8: mov 2, g0   c: ret
        let p = prog(&[0x5c80_0e01, 0x1200_0008, 0x5c80_0e02, 0x0a00_0000]);
        assert_eq!(leaders(&p, &BTreeSet::new()).into_iter().collect::<Vec<_>>(), [0, 8, 0xc]);
    }

    #[test]
    fn common_instructions_translate() {
        let p = prog(&[0x5c80_0e01, 0x1200_0008, 0x0a00_0000]);
        assert_eq!(translate(&p[&0]).unwrap(), "c.r[16] = 0x1;");
        assert!(translate(&p[&4]).unwrap().contains("c.cond(2)"));
        let m = program_module(&p, &BTreeSet::new(), 1);
        assert!(m.contains("fn b00000000<H: Host>") && m.contains("(0x8, b00000008::<H>)"), "{m}");
    }
}
