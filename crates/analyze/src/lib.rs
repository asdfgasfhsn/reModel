//! Recursive-descent code discovery over the i960 program ROM.

use i960::{decode, Flow, Insn, Operand};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub struct Image<'a> {
    pub base: u32,
    pub bytes: &'a [u8],
}

impl Image<'_> {
    pub fn word(&self, addr: u32) -> Option<u32> {
        let off = addr.checked_sub(self.base)? as usize;
        let b = self.bytes.get(off..off + 4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn contains(&self, addr: u32) -> bool {
        addr >= self.base && ((addr - self.base) as usize) < self.bytes.len()
    }

    pub fn decode(&self, addr: u32) -> Insn {
        decode(addr, |a| self.word(a))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SeedKind {
    Reset,
    Interrupt(u8),
    SysProc(u16),
    Fault(u8),
}

pub struct Boot {
    pub prcb: u32,
    pub reset_ip: u32,
    pub int_table: u32,
    pub sysproc_table: u32,
    pub fault_table: u32,
    pub seeds: Vec<(SeedKind, u32)>,
}

/// Read the boot record and system tables. Layout matches the i960 KB
/// initialisation scheme as observed in this ROM: SAT at 0, PRCB pointer at
/// SAT+4, first IP at SAT+12, interrupt table at PRCB+20, and the
/// sysproc/fault table bases in the SAT segment descriptors at +152/+168.
pub fn read_boot(img: &Image) -> Option<Boot> {
    let prcb = img.word(4)?;
    let reset_ip = img.word(12)?;
    let int_table = img.word(prcb + 20)?;
    let sysproc_table = img.word(152)?;
    let fault_table = img.word(168)?;
    let code = |a: u32| a & 3 == 0 && a >= 0x100 && img.contains(a);

    let mut seeds = vec![(SeedKind::Reset, reset_ip)];
    for v in 8..=255u32 {
        if let Some(a) = img.word(int_table + 36 + (v - 8) * 4) {
            if code(a) {
                seeds.push((SeedKind::Interrupt(v as u8), a));
            }
        }
    }
    // The tables sit just below the handlers they point at, so a table ends
    // once its read pointer reaches the lowest handler found so far.
    let mut bound = reset_ip;
    for n in 0..260u32 {
        let at = sysproc_table + 48 + n * 4;
        if at >= bound {
            break;
        }
        let Some(e) = img.word(at) else { break };
        let a = e & !3;
        if code(a) {
            bound = bound.min(a);
            seeds.push((SeedKind::SysProc(n as u16), a));
        }
    }
    for n in 0..32u32 {
        let at = fault_table + n * 8;
        if at >= bound.min(sysproc_table) {
            break;
        }
        let Some(e) = img.word(at) else { break };
        let a = e & !3;
        if code(a) {
            seeds.push((SeedKind::Fault(n as u8), a));
        }
    }
    Some(Boot { prcb, reset_ip, int_table, sysproc_table, fault_table, seeds })
}

#[derive(Default)]
pub struct Analysis {
    pub insns: BTreeMap<u32, Insn>,
    /// Function entry points (call/bal/seed targets).
    pub functions: BTreeSet<u32>,
    /// Basic block leaders.
    pub labels: BTreeSet<u32>,
    pub indirect: Vec<u32>,
    pub invalid: Vec<u32>,
    /// Statically known data references (loads/stores/lda), by target.
    pub data_refs: BTreeMap<u32, u32>,
    /// Entries reached only through `lda` code-pointer heuristics.
    pub speculative: BTreeSet<u32>,
}

/// i960 inter-agent communication port; a `synmovq` here sends a message to
/// the processor itself.
const IAC_PORT: u32 = 0xff00_0010;
const IAC_REINIT: u32 = 0x93;

/// Registers an instruction overwrites (conservative, for constant tracking).
fn clobbers(insn: &Insn) -> Vec<u8> {
    match insn.flow {
        Flow::Call(_) | Flow::IndirectCall | Flow::SysCall | Flow::Link(_) | Flow::IndirectLink => {
            return (0..32).collect();
        }
        _ => {}
    }
    let width = match insn.mnemonic {
        "ldl" | "movl" | "emul" | "ediv" => 2,
        "ldt" | "movt" => 3,
        "ldq" | "movq" => 4,
        _ => 1,
    };
    if insn.mnemonic.starts_with("st") || insn.mnemonic.starts_with("cmp") {
        return vec![];
    }
    match insn.operands.last() {
        Some(Operand::Reg(r)) if insn.operands.len() > 1 => (*r..r + width).filter(|&x| x < 32).collect(),
        _ => vec![],
    }
}

fn reg_of(m: &i960::MemRef) -> Option<u8> {
    match *m {
        i960::MemRef::Base { abase, disp: 0 } => Some(abase),
        _ => None,
    }
}

/// Decode a short run at `addr` and accept it as plausible code.
fn looks_like_code(img: &Image, mut addr: u32) -> bool {
    for _ in 0..12 {
        let i = img.decode(addr);
        match i.flow {
            Flow::Invalid => return false,
            Flow::Return | Flow::Jump(_) | Flow::IndirectJump => return true,
            _ => addr = i.next(),
        }
    }
    true
}

pub fn explore(img: &Image, seeds: &[u32]) -> Analysis {
    let mut a = Analysis::default();
    let mut work: VecDeque<u32> = VecDeque::new();
    for &s in seeds {
        a.functions.insert(s);
        work.push_back(s);
    }
    // Code addresses materialised with `lda`, tried once the direct graph is done.
    let mut pointer_candidates: BTreeSet<u32> = BTreeSet::new();

    loop {
        while let Some(start) = work.pop_front() {
            a.labels.insert(start);
            let mut pc = start;
            let mut consts: [Option<u32>; 32] = [None; 32];
            let mut prev: Option<Insn> = None;
            loop {
                if a.insns.contains_key(&pc) || !img.contains(pc) {
                    break;
                }
                let insn = img.decode(pc);
                let next = insn.next();
                for op in &insn.operands {
                    if let Operand::Mem(m) = op {
                        if let Some(t) = m.static_addr() {
                            // `lda` materialises constants (often float bit patterns),
                            // so only real accesses count towards the memory map.
                            if insn.mnemonic != "lda"
                                && !matches!(insn.flow, Flow::Jump(_) | Flow::Call(_) | Flow::Link(_))
                            {
                                *a.data_refs.entry(t).or_default() += 1;
                            }
                        }
                    }
                }

                let mut found: Vec<(u32, bool)> = Vec::new();
                let mut resolved = false;
                let reg_const = |op: Option<&Operand>, consts: &[Option<u32>; 32]| match op {
                    Some(Operand::Reg(r)) => consts[*r as usize],
                    Some(Operand::Mem(m)) => reg_of(m).and_then(|r| consts[r as usize]),
                    _ => None,
                };

                // Processor reinitialisation: follow the new first IP.
                if insn.mnemonic == "synmovq" {
                    let dst = reg_const(insn.operands.first(), &consts);
                    let src = reg_const(insn.operands.get(1), &consts);
                    if let (Some(IAC_PORT), Some(msg)) = (dst, src) {
                        if img.word(msg).map(|w| w >> 24) == Some(IAC_REINIT) {
                            if let Some(ip) = img.word(msg + 12) {
                                found.push((ip, true));
                            }
                        }
                    }
                }

                // Indirect transfers through a register holding a known constant.
                if matches!(insn.flow, Flow::IndirectCall | Flow::IndirectLink | Flow::IndirectJump) {
                    if let Some(t) = reg_const(insn.operands.first(), &consts) {
                        if img.contains(t) {
                            found.push((t, insn.flow != Flow::IndirectJump));
                            resolved = true;
                        }
                    }
                }

                // Jump table: `ld disp[gX*4], gY` immediately followed by `bx (gY)`.
                if insn.flow == Flow::IndirectJump && !resolved {
                    if let (Some(p), Some(Operand::Mem(m))) = (&prev, insn.operands.first()) {
                        if let (
                            Some(Operand::Mem(i960::MemRef::Index { disp, scale: 4, .. })),
                            Some(Operand::Reg(ld_dst)),
                            Some(r),
                        ) = (p.operands.first(), p.operands.get(1), reg_of(m))
                        {
                            if p.mnemonic == "ld" && *ld_dst == r {
                                let mut n = 0;
                                while let Some(t) = img.word(disp + n * 4) {
                                    // Entries must be aligned code addresses; the table
                                    // also can't run into its own targets.
                                    if t & 3 != 0
                                        || !img.contains(t)
                                        || t < 0x100
                                        || (t > *disp && t < disp + n * 4 + 4)
                                        || n >= 1024
                                    {
                                        break;
                                    }
                                    found.push((t, false));
                                    n += 1;
                                }
                                resolved = n > 0;
                            }
                        }
                    }
                }

                for r in clobbers(&insn) {
                    consts[r as usize] = None;
                }
                if insn.mnemonic == "lda" {
                    if let (Some(Operand::Mem(m)), Some(Operand::Reg(r))) =
                        (insn.operands.first(), insn.operands.get(1))
                    {
                        if let Some(v) = m.static_addr() {
                            consts[*r as usize] = Some(v);
                            if v & 3 == 0 && v >= 0x100 && img.contains(v) {
                                pointer_candidates.insert(v);
                            }
                        }
                    }
                }

                let flow = insn.flow;
                prev = Some(insn.clone());
                a.insns.insert(pc, insn);
                for (t, func) in found {
                    if func {
                        a.functions.insert(t);
                    }
                    a.labels.insert(t);
                    work.push_back(t);
                }
                let mut push = |t: u32, func: bool, a: &mut Analysis| {
                    if func {
                        a.functions.insert(t);
                    }
                    a.labels.insert(t);
                    work.push_back(t);
                };
                match flow {
                    Flow::Next | Flow::CondFault | Flow::SysCall => pc = next,
                    Flow::Branch(t) => {
                        push(t, false, &mut a);
                        a.labels.insert(next);
                        pc = next;
                    }
                    Flow::Call(t) | Flow::Link(t) => {
                        push(t, true, &mut a);
                        pc = next;
                    }
                    Flow::IndirectCall | Flow::IndirectLink => {
                        if !resolved {
                            a.indirect.push(pc);
                        }
                        pc = next;
                    }
                    Flow::Jump(t) => {
                        push(t, false, &mut a);
                        break;
                    }
                    Flow::IndirectJump => {
                        if !resolved {
                            a.indirect.push(pc);
                        }
                        break;
                    }
                    Flow::Return => break,
                    Flow::Invalid => {
                        a.invalid.push(pc);
                        break;
                    }
                }
                if a.labels.contains(&pc) {
                    // New block: constants from this path may not hold on others.
                    consts = [None; 32];
                }
            }
        }

        // Data words that point just past a terminator into plausible code:
        // dispatch tables and callback pointers stored in ROM.
        for off in (0..img.bytes.len() as u32).step_by(4) {
            let Some(p) = img.word(img.base + off) else { break };
            if p & 3 != 0 || p < 0x700 || !img.contains(p) || a.insns.contains_key(&p) || a.speculative.contains(&p) {
                continue;
            }
            let before = img.decode(p - 4);
            if matches!(before.flow, Flow::Return | Flow::Jump(_) | Flow::IndirectJump) || before.word == 0 {
                pointer_candidates.insert(p);
            }
        }

        // Second pass: code pointers taken with `lda` (callbacks, task tables).
        let fresh: Vec<u32> = pointer_candidates
            .iter()
            .copied()
            .filter(|p| !a.insns.contains_key(p) && !a.speculative.contains(p) && looks_like_code(img, *p))
            .collect();
        pointer_candidates.clear();
        if fresh.is_empty() {
            break;
        }
        for p in fresh {
            a.speculative.insert(p);
            a.functions.insert(p);
            work.push_back(p);
        }
    }
    a
}
