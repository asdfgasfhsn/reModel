//! `tgp-disasm`: a Model 2A TGP (MB86234) program image, listed.
//!
//! It splits the image into the parts its shape shows (the header, the
//! dispatch table, the functions the table points at), counts opcodes by
//! the top byte, and lists every word with the mnemonic of
//! `mb86234::disasm` (the encodings the interpreter implements;
//! `docs/hw/mb86234.md` says how sure each is). Words of a form not known
//! show as `??`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

/// A word of the leading run is a jump-vector candidate when its top byte
/// is this (every 2A program starts with such a run; `docs/hw/mb86234.md`).
const JUMP_TOP: u8 = 0xbf;
/// The top 16 bits of the return-like word (`BF6A0000`).
const RETURN: u32 = 0xbf6a;
/// The top 16 bits of the jump to a register's address (`BF62`).
const JUMP_REG: u32 = 0xbf62;
/// The shortest run of such words taken as the dispatch table.
const TABLE_MIN: usize = 32;

/// What the image's shape shows.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Analysis {
    pub words: usize,
    /// The words before the table: the leading run of jump-like words
    /// is the header, the rest (if any) the setup code.
    pub header: std::ops::Range<usize>,
    /// The dispatch table: the longest run of jump-like words.
    pub table: std::ops::Range<usize>,
    /// Table entries whose target (the low 16 bits) lies outside the image.
    pub bad_targets: usize,
    /// Table slots whose target is 0 (as slot 0 of every 2A program's:
    /// no function there).
    pub zero_slots: usize,
    /// The word after the table, if it is a return: where every unused
    /// slot points.
    pub common_return: Option<usize>,
    /// Distinct targets of the table, ascending: where the functions start.
    pub entries: Vec<usize>,
    /// How many table slots point at each entry.
    pub slots: BTreeMap<usize, usize>,
    /// Words by top byte.
    pub tops: BTreeMap<u8, usize>,
}

/// Is `w` one of the jump-like words (top byte `0xbf`)?
fn jumpish(w: u32) -> bool {
    (w >> 24) as u8 == JUMP_TOP
}

pub fn analyse(program: &[u32]) -> Analysis {
    let mut a = Analysis { words: program.len(), ..Analysis::default() };
    for &w in program {
        *a.tops.entry((w >> 24) as u8).or_default() += 1;
    }
    // The header: the run of jump-like words at the start.
    a.header = 0..program.iter().take_while(|&&w| jumpish(w)).count();
    // The table: the longest such run after the header, if it is long
    // enough to be one.
    let mut best = 0..0;
    let mut start = None;
    for i in a.header.end..=program.len() {
        match (start, program.get(i).is_some_and(|&w| jumpish(w))) {
            (None, true) => start = Some(i),
            (Some(s), false) => {
                if i - s > best.len() {
                    best = s..i;
                }
                start = None;
            }
            _ => {}
        }
    }
    if best.len() >= TABLE_MIN {
        // The run includes the word before the first slot (`srallyc`'s
        // `BF6A....` return; `vf2`'s main loop's `BF62` jump through the
        // table) and the common return after the last, which the unused
        // slots point at: the table is what lies between, slot F being
        // function F (`docs/hw/mb86234.md`).
        let ret = |i: usize| program[i] >> 16 == RETURN;
        if ret(best.start) || program[best.start] >> 16 == JUMP_REG {
            best.start += 1;
        }
        if ret(best.end - 1) {
            best.end -= 1;
        }
        a.table = best;
        a.common_return = program[a.table.end..].first().is_some_and(|&w| w >> 16 == RETURN).then_some(a.table.end);
    }
    for &w in &program[a.table.clone()] {
        let t = (w & 0xffff) as usize;
        if t == 0 {
            a.zero_slots += 1;
        } else if t >= program.len() {
            a.bad_targets += 1;
        } else {
            *a.slots.entry(t).or_default() += 1;
        }
    }
    a.entries = a.slots.keys().copied().collect();
    a
}

pub fn disasm(file: &Path, out: &Path) -> Result<(), String> {
    let bytes = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    if bytes.len() % 4 != 0 {
        return Err(format!("{}: {} bytes is not a whole number of 32-bit words", file.display(), bytes.len()));
    }
    let program: Vec<u32> = bytes.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
    let a = analyse(&program);
    let range = |r: &std::ops::Range<usize>| {
        if r.is_empty() {
            "none".to_string()
        } else {
            format!("words {:#x}-{:#x} ({} words)", r.start, r.end - 1, r.len())
        }
    };
    println!("words         {} (32 bits each)", a.words);
    println!("header        {}", range(&a.header));
    println!(
        "dispatch      {}, slot = function number; {} slots with target 0, {} targets out of range",
        range(&a.table),
        a.zero_slots,
        a.bad_targets
    );
    let unused = a.common_return.map_or(0, |r| a.slots.get(&r).copied().unwrap_or(0));
    println!(
        "functions     {} distinct entry points for {} slots ({} unused slots point at the common return {})",
        a.entries.len(),
        a.table.len() - a.zero_slots - a.bad_targets,
        unused,
        a.common_return.map_or("none".to_string(), |r| format!("{r:#x}"))
    );
    println!("opcodes       {} distinct top bytes; the commonest:", a.tops.len());
    let mut by_count: Vec<_> = a.tops.iter().collect();
    by_count.sort_by_key(|&(op, n)| (std::cmp::Reverse(*n), *op));
    for (op, n) in by_count.iter().take(8) {
        println!("  {op:02x} {n:>5} {:>5.1}%", **n as f64 * 100.0 / a.words.max(1) as f64);
    }

    // The listing: words with where they are in the structure.
    let mut text = String::new();
    let _ = writeln!(text, "; {} words; mnemonics: docs/hw/mb86234.md", a.words);
    let mut unknown = 0;
    for (i, &w) in program.iter().enumerate() {
        if a.entries.contains(&i) {
            let slots: Vec<String> = a
                .table
                .clone()
                .filter(|&s| (program[s] & 0xffff) as usize == i)
                .map(|s| format!("function {:#x}", s - a.table.start))
                .collect();
            let _ = writeln!(text, "\nfunc_{i:03x}:  ; dispatch slot(s): {}", slots.join(", "));
        }
        let line = ::mb86234::disasm::line(w);
        unknown += line.starts_with("??") as usize;
        let note = if a.header.contains(&i) {
            format!("  ; vector {i}")
        } else if a.table.contains(&i) {
            format!("  ; function {:#x}", i - a.table.start)
        } else {
            String::new()
        };
        let _ = writeln!(text, "  {i:03x}  {w:08x}  {line}{note}");
    }
    std::fs::write(out, text).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("listing written to {} ({unknown} words of unknown form)", out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_header_table_and_entries() {
        // 2 header words, 4 filler, a return, a 40-slot table pointing at 3
        // places, the common return.
        let mut p = vec![0xbf60_0006, 0xbf60_0010, 0x1c00_0000, 0x3c00_0000, 0x1c00_0001, 0x7c00_0002, 0xbf6a_0000];
        let base = p.len();
        for i in 0..40u32 {
            p.push(0xbf60_0000 | (base as u32 + 41 + (i % 3)));
        }
        p.push(0xbf6a_0000);
        p.extend([0x1c00_0000; 8]);
        let a = analyse(&p);
        assert_eq!(a.header, 0..2);
        assert_eq!(a.table, base..base + 40, "the returns around the table aren't slots");
        assert_eq!(a.common_return, Some(base + 40));
        assert_eq!(a.entries, vec![base + 41, base + 42, base + 43]);
        assert_eq!(a.bad_targets, 0);
        assert_eq!(a.slots[&(base + 41)], 14);
        assert_eq!(a.tops[&0xbf], 44);
    }

    #[test]
    fn short_runs_are_no_table() {
        let a = analyse(&[0xbf60_0001, 0x1c00_0000, 0xbf60_0000, 0xbf60_0000, 0x1c00_0000]);
        assert!(a.table.is_empty());
        assert_eq!(a.header, 0..1);
    }

    #[test]
    fn targets_outside_the_image_are_counted() {
        let mut p = vec![0x1c00_0000];
        p.extend((0..40).map(|_| 0xbf60_ffffu32));
        let a = analyse(&p);
        assert_eq!(a.bad_targets, 40);
        assert!(a.entries.is_empty());
    }
}
