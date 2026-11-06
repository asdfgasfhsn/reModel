//! `sharc-disasm` and `sharc-check`: ADSP-21062 program images, as 48-bit
//! words of six little-endian bytes each, loaded at 0x20000 (the Model 2B
//! SHARC uploads; docs/hw/adsp2106x.md says how to capture them).

use adsp2106x::cpu::{self, Bus, Sharc, Step, Width};
use adsp2106x::{decode, Flow, Insn, Op};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

/// Internal memory address the images start at.
const BASE: u32 = 0x2_0000;

pub fn load(path: &Path) -> Result<Vec<u64>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() % 6 != 0 {
        return Err(format!("{}: {} bytes is not a whole number of 48-bit words", path.display(), bytes.len()));
    }
    Ok(bytes.chunks(6).map(|c| c.iter().rev().fold(0u64, |a, &b| a << 8 | b as u64)).collect())
}

/// What recursive descent from the reset and interrupt vectors finds.
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
pub fn discover(words: &[u64]) -> Discovery {
    let end = BASE + words.len() as u32;
    let at = |a: u32| -> Option<Insn> { (BASE..end).contains(&a).then(|| decode(a, words[(a - BASE) as usize])) };
    let mut d = Discovery::default();
    let mut work: Vec<u32> = (0..32).map(|k| BASE + 4 * k).collect();
    work.push(BASE + 5);
    d.labels.insert(BASE + 5, "reset");
    while let Some(a) = work.pop() {
        let Some(insn) = at(a) else { continue };
        if !d.code.insert(a) {
            continue;
        }
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
        // Immediates that point into the image are taken as code pointers
        // (the programs build their command dispatch tables this way).
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

/// `sharc-disasm`: the listing of every word, code first marked by
/// discovery.
pub fn disasm(path: &Path, out: &Path) -> Result<(), String> {
    let words = load(path)?;
    let d = discover(&words);
    let mut s = String::new();
    let last = words.iter().rposition(|&w| w != 0).unwrap_or(0);
    for (k, &w) in words.iter().enumerate().take(last + 1) {
        let a = BASE + k as u32;
        if let Some(kind) = d.labels.get(&a) {
            let _ = writeln!(s, "\n{a:05x}:  {{{kind}}}");
        }
        let mark = if d.code.contains(&a) { ' ' } else { '.' };
        let _ = writeln!(s, "{a:05x}{mark} {w:012x}  {}", decode(a, w));
    }
    std::fs::write(out, s).map_err(|e| format!("{}: {e}", out.display()))?;
    println!(
        "{}: {} words, {} reached as code ('.' marks the rest) -> {}",
        path.display(),
        last + 1,
        d.code.len(),
        out.display()
    );
    Ok(())
}

/// Records the first external access and then stops answering.
#[derive(Default)]
struct StubBus {
    first: Option<(u32, Width, Option<u64>)>,
}

impl Bus for StubBus {
    fn read(&mut self, addr: u32, width: Width) -> u64 {
        self.first.get_or_insert((addr, width, None));
        0
    }
    fn write(&mut self, addr: u32, width: Width, v: u64) {
        self.first.get_or_insert((addr, width, Some(v)));
    }
}

/// Boot `words` the way the Model 2B host does: DMA channel 6 set up for
/// the whole image, then the words written to EPB0 as 16-bit halves, low
/// half first (docs/hw/adsp2106x.md).
pub fn host_boot(words: &[u64]) -> Result<Sharc, String> {
    let mut s = Sharc::new();
    let w32 = |s: &mut Sharc, reg: u32, v: u32| -> Result<(), String> {
        for half in [v & 0xffff, v >> 16] {
            s.host_write(reg, half).map_err(|e| format!("host write {reg:#x}: {e:?}"))?;
        }
        Ok(())
    };
    w32(&mut s, cpu::DMAC6, 0)?;
    w32(&mut s, cpu::II6, BASE)?;
    w32(&mut s, cpu::IM6, 1)?;
    w32(&mut s, cpu::C6, words.len() as u32)?;
    w32(&mut s, cpu::DMAC6, 0xa1)?;
    for &w in words {
        for k in 0..3 {
            s.host_write(cpu::EPB0, (w >> (16 * k)) as u32 & 0xffff).map_err(|e| format!("EPB0: {e:?}"))?;
        }
    }
    Ok(s)
}

/// `sharc-check`: decode coverage, discovery and a run to the first
/// external access, for each image.
pub fn check(paths: &[String], max_steps: u64) -> Result<(), String> {
    let mut bad = false;
    for p in paths {
        let path = Path::new(p);
        let words = load(path)?;
        let last = words.iter().rposition(|&w| w != 0).map_or(0, |k| k + 1);
        let all_unknown: Vec<u32> =
            (0..last).filter(|&k| decode(BASE + k as u32, words[k]).is_unknown()).map(|k| BASE + k as u32).collect();
        let d = discover(&words);
        let code_unknown: Vec<u32> =
            d.code.iter().copied().filter(|&a| decode(a, words[(a - BASE) as usize]).is_unknown()).collect();
        let (mut rts, mut rti, mut loops, mut bad_loops) = (0, 0, 0, Vec::new());
        for &a in &d.code {
            match decode(a, words[(a - BASE) as usize]).op {
                Op::Return { rti: true, .. } => rti += 1,
                Op::Return { .. } => rts += 1,
                Op::Do { end, .. } => {
                    loops += 1;
                    if end <= a || !d.code.contains(&end) {
                        bad_loops.push(a);
                    }
                }
                _ => {}
            }
        }
        let calls = d.labels.values().filter(|k| **k == "call").count();
        println!("{}", path.display());
        println!("  words         {last} nonzero-bounded, {} reached as code", d.code.len());
        println!("  undefined     {} of all words, {} of code", all_unknown.len(), code_unknown.len());
        for a in code_unknown.iter().take(16) {
            println!("    {a:05x}  {:012x}", words[(a - BASE) as usize]);
        }
        println!("  routines      {calls} call targets, {rts} RTS, {rti} RTI, {} indirect branches", d.indirect.len());
        println!("  loops         {loops} DO UNTIL");
        for a in &bad_loops {
            // Reported, not failed: the copro program has one (see
            // docs/hw/adsp2106x.md).
            println!("    note: {a:05x}  {}  (the loop end isn't after it)", decode(*a, words[(a - BASE) as usize]));
        }
        bad |= !code_unknown.is_empty();

        let mut s = host_boot(&words)?;
        let mut bus = StubBus::default();
        let mut stop = String::from("step limit");
        let (mut seen, mut toggled) = (0, false);
        for n in 0..max_steps {
            // Stop when nothing new has run for a while: a polling loop
            // waiting for the host.
            if n % 200_000 == 199_999 {
                let now = s.counts.iter().filter(|&&c| c > 0).count();
                if now == seen {
                    // Waiting for the host: toggle FLAG0 once, as a host
                    // handshake would, then give up.
                    if !toggled {
                        toggled = true;
                        s.flag_in ^= 1;
                        println!("  polling       at pc {:#x}; toggling FLAG0 to {}", s.pc, s.flag_in & 1);
                        continue;
                    }
                    stop = format!("polling (FLAG outputs {:04b})", s.flag_outputs());
                    break;
                }
                seen = now;
            }
            match s.step(&mut bus) {
                Step::Ran => {}
                Step::Idle => {
                    stop = "IDLE".into();
                    break;
                }
                Step::Fault(f) => {
                    stop = format!("{f:x?}");
                    bad = true;
                    break;
                }
            }
            if let Some((a, w, v)) = bus.first {
                stop = match v {
                    Some(v) => format!("external write {a:#x} ({w:?}) = {v:#x}"),
                    None => format!("external read {a:#x} ({w:?})"),
                };
                break;
            }
        }
        let w48 = adsp2106x::mem::WORDS48 as usize;
        let ran: Vec<u32> = (0..s.counts.len())
            .filter(|&k| s.counts[k] > 0)
            .map(|k| if k < w48 { adsp2106x::mem::BLOCK0 } else { adsp2106x::mem::BLOCK1 } + (k % w48) as u32)
            .collect();
        let missed = ran.iter().filter(|a| !d.code.contains(a)).count();
        println!(
            "  run           {} instructions ({} distinct, {missed} not found by discovery), stopped at pc {:#x}: {stop}",
            s.steps,
            ran.len(),
            s.pc
        );
    }
    if bad {
        Err("undefined encodings in code, or a fault (see above)".into())
    } else {
        Ok(())
    }
}
