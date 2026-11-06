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

fn src(op: Option<&Operand>) -> Option<String> {
    match op? {
        Operand::Reg(r) => Some(format!("c.r[{r}]")),
        Operand::Lit(v) => Some(format!("{v:#x}u32")),
        _ => None,
    }
}

fn reg(op: Option<&Operand>) -> Option<usize> {
    match op? {
        Operand::Reg(r) => Some(*r as usize),
        _ => None,
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
    let ops = &i.operands;
    let op = i.word >> 24;
    let next = i.next();
    let fault = format!("map_err(|kind| Stop::Fault {{ kind, ip: {:#x} }})?", i.addr);
    Some(match op {
        0x08 | 0x09 | 0x0b | 0x10..=0x17 => {
            let Some(Operand::Target(t)) = ops.first() else { return None };
            match op {
                0x08 => format!("c.ip = {t:#x};"),
                0x09 => format!("c.call(h, {t:#x}, {next:#x});"),
                0x0b => format!("c.r[30] = {next:#x}; c.ip = {t:#x};"),
                _ => format!("c.ip = if c.cond({}) {{ {t:#x} }} else {{ {next:#x} }};", op & 7),
            }
        }
        0x0a => format!("c.ret(h).{fault};"),
        0x20..=0x27 => format!("c.r[{}] = c.cond({}) as u32;", reg(ops.first())?, op & 7),
        0x30..=0x3f => {
            let (a, b) = (src(ops.first())?, src(ops.get(1))?);
            let Some(Operand::Target(t)) = ops.get(2) else { return None };
            let take = match op {
                0x30 | 0x37 => {
                    return Some(format!(
                        "let set = {b} & (1u32 << ({a} & 31)) != 0; c.set_cc(if set {{ 2 }} else {{ 0 }}); \
                         c.ip = if set == {} {{ {t:#x} }} else {{ {next:#x} }};",
                        op == 0x37
                    ))
                }
                0x31..=0x36 => format!("c.compare({a}, {b});"),
                _ => format!("c.compare({a} as i32, {b} as i32);"),
            };
            format!("{take} c.ip = if c.cond({}) {{ {t:#x} }} else {{ {next:#x} }};", op & 7)
        }
        0x58..=0x7f => return translate_reg(i),
        0x80..=0xcf => {
            let m = match ops.iter().find(|o| matches!(o, Operand::Mem(_))) {
                Some(Operand::Mem(m)) => ea(m),
                _ => return None,
            };
            let load = || reg(ops.get(1));
            let store = || reg(ops.first());
            match op {
                0x84 => format!("c.ip = {m};"),
                0x85 => format!("let a = {m}; c.r[{}] = {next:#x}; c.ip = a;", load()?),
                0x86 => format!("let a = {m}; c.call(h, a, {next:#x});"),
                0x8c => format!("c.r[{}] = {m};", load()?),
                0x80 => format!("c.r[{}] = h.read8({m}) as u32;", load()?),
                0xc0 => format!("c.r[{}] = h.read8({m}) as i8 as i32 as u32;", load()?),
                0x88 => format!("c.r[{}] = h.read16({m}) as u32;", load()?),
                0xc8 => format!("c.r[{}] = h.read16({m}) as i16 as i32 as u32;", load()?),
                0x82 | 0xc2 => format!("h.write8({m}, c.r[{}] as u8);", store()?),
                0x8a | 0xca => format!("h.write16({m}, c.r[{}] as u16);", store()?),
                0x90 | 0x98 | 0xa0 | 0xb0 | 0x92 | 0x9a | 0xa2 | 0xb2 => {
                    let n = match op & !2 {
                        0x90 => 1,
                        0x98 => 2,
                        0xa0 => 3,
                        _ => 4,
                    };
                    let mut s = format!("let a = {m};");
                    for k in 0..n {
                        let addr = if k == 0 { "a".to_string() } else { format!("a.wrapping_add({})", 4 * k) };
                        if op & 2 == 0 {
                            let _ = write!(s, " c.r[{}] = h.read32({addr});", (load()? + k) & 31);
                        } else {
                            let _ = write!(s, " h.write32({addr}, c.r[{}]);", (store()? + k) & 31);
                        }
                    }
                    s
                }
                _ => return None,
            }
        }
        _ => return None,
    })
}

fn translate_reg(i: &Insn) -> Option<String> {
    let ops = &i.operands;
    let full = ((i.word >> 24) << 4) | ((i.word >> 7) & 0xf);
    let (s1, s2) = (|| src(ops.first()), || src(ops.get(1)));
    let dst3 = |e: String| Some(format!("c.r[{}] = {e};", reg(ops.get(2))?));
    let bit = |e: String| format!("(1u32 << ({e} & 31))");
    match full {
        0x580 => dst3(format!("{} ^ {}", s2()?, bit(s1()?))),
        0x581 => dst3(format!("{} & {}", s2()?, s1()?)),
        0x582 => dst3(format!("{} & !{}", s2()?, s1()?)),
        0x583 => dst3(format!("{} | {}", s2()?, bit(s1()?))),
        0x584 => dst3(format!("!{} & {}", s2()?, s1()?)),
        0x586 => dst3(format!("{} ^ {}", s2()?, s1()?)),
        0x587 => dst3(format!("{} | {}", s2()?, s1()?)),
        0x588 => dst3(format!("!({} | {})", s2()?, s1()?)),
        0x589 => dst3(format!("!({} ^ {})", s2()?, s1()?)),
        0x58a => Some(format!("c.r[{}] = !{};", reg(ops.get(1))?, s1()?)),
        0x58b => dst3(format!("{} | !{}", s2()?, s1()?)),
        0x58c => dst3(format!("{} & !{}", s2()?, bit(s1()?))),
        0x58d => dst3(format!("!{} | {}", s2()?, s1()?)),
        0x58e => dst3(format!("!({} & {})", s2()?, s1()?)),
        0x590 | 0x591 => dst3(format!("{}.wrapping_add({})", s2()?, s1()?)),
        0x592 | 0x593 => dst3(format!("{}.wrapping_sub({})", s2()?, s1()?)),
        0x598 => dst3(format!("{{ let n = {}; if n >= 32 {{ 0 }} else {{ {} >> n }} }}", s1()?, s2()?)),
        0x59b => dst3(format!("(({} as i32) >> {}.min(31)) as u32", s2()?, s1()?)),
        0x59c | 0x59e => dst3(format!("{{ let n = {}; if n >= 32 {{ 0 }} else {{ {} << n }} }}", s1()?, s2()?)),
        0x59d => dst3(format!("{}.rotate_left({} & 31)", s2()?, s1()?)),
        0x5a0 => Some(format!("c.compare({}, {});", s1()?, s2()?)),
        0x5a1 => Some(format!("c.compare({} as i32, {} as i32);", s1()?, s2()?)),
        0x5ae => Some(format!("c.set_cc(if {} & {} != 0 {{ 2 }} else {{ 0 }});", s2()?, bit(s1()?))),
        0x5cc | 0x5dc | 0x5ec | 0x5fc => {
            let n = ((full - 0x5cc) >> 4) as usize + 1;
            let d = reg(ops.get(1))?;
            Some(match ops.first()? {
                Operand::Reg(s) => {
                    let s = *s as usize;
                    let t: Vec<String> = (0..n).map(|k| format!("c.r[{}]", (s + k) & 31)).collect();
                    let dsts: Vec<String> = (0..n).map(|k| format!("c.r[{}]", (d + k) & 31)).collect();
                    if n == 1 {
                        format!("{} = {};", dsts[0], t[0])
                    } else {
                        format!("let t = [{}]; [{}] = t;", t.join(", "), dsts.join(", "))
                    }
                }
                Operand::Lit(v) => {
                    let mut s = format!("c.r[{d}] = {v:#x};");
                    for k in 1..n {
                        let _ = write!(s, " c.r[{}] = 0;", (d + k) & 31);
                    }
                    s
                }
                _ => return None,
            })
        }
        0x701 | 0x741 => dst3(format!("{}.wrapping_mul({})", s2()?, s1()?)),
        _ => None,
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
    let _ = writeln!(s, "/// {translated} of {total} instructions translated, the rest run through `Cpu::exec`.");
    let _ = writeln!(s, "#[allow(dead_code)]\npub const TRANSLATED: (usize, usize) = ({translated}, {total});\n");
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
