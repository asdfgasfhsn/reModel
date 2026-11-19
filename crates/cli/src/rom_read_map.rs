//! SPIKE `remodel rom-read-map LABEL=FILE...`: how much of the polygon ROM
//! the runs read (files written by `run` with MP_READMAP=FILE).
use std::collections::BTreeSet;

struct Run {
    label: String,
    read: Vec<bool>,
    starts: BTreeSet<u32>,
    ends: BTreeSet<u32>,
}

fn load(label: &str, path: &str, n: usize, mask: u32) -> Result<Run, String> {
    let t = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let mut r = Run { label: label.into(), read: vec![false; n], starts: BTreeSet::new(), ends: BTreeSet::new() };
    for l in t.lines() {
        let f: Vec<&str> = l.split(' ').collect();
        let v = |i: usize| f[i].parse::<usize>().unwrap_or(0);
        match f[0] {
            "R" => (v(1)..v(2).min(n)).for_each(|i| r.read[i] = true),
            "S" => {
                r.starts.insert(v(1) as u32 & mask);
            }
            "E" => {
                r.ends.insert(v(1) as u32 & mask);
            }
            _ => {}
        }
    }
    Ok(r)
}

/// Parse a compact model from word `s` (two vertices, then polygons of
/// attr + 1 or 2 vertices, ended by attr&3==0) within `[s, lim)`: the
/// polygon count if it ends cleanly on a terminator.
fn parse(rom: &[u32], s: usize, lim: usize) -> Option<usize> {
    parse_x(rom, s, lim, false)
}

/// `strict`: also every attribute word has bits 2-7 clear and its link bits
/// (8-9) equal to their copy (23-24), and every vertex word is a finite
/// float under 1e6 in magnitude.
fn parse_x(rom: &[u32], s: usize, lim: usize, strict: bool) -> Option<usize> {
    let good = |w: u32| !strict || (f32::from_bits(w).is_finite() && f32::from_bits(w).abs() < 1e6);
    if s + 6 > lim || !rom[s..s + 6].iter().all(|&w| good(w)) {
        return None;
    }
    let mut i = s + 6;
    let mut n = 0;
    while i < lim {
        let attr = rom[i];
        i += 1;
        if attr & 3 == 0 {
            return (n > 0).then_some(n);
        }
        if strict && (attr & 0xfc != 0 || (attr >> 8) & 3 != (attr >> 23) & 3) {
            return None;
        }
        let k = if attr & 1 != 0 { 6 } else { 3 };
        if i + k > lim || !rom[i..i + k].iter().all(|&w| good(w)) {
            return None;
        }
        i += k;
        n += 1;
    }
    None
}

pub fn run(rom: &[u32], args: &[String]) -> Result<(), String> {
    let n = rom.len();
    let mask = (n.next_power_of_two() - 1) as u32;
    // Blank: inside a run of 16+ identical words (64+ bytes).
    let mut blank = vec![false; n];
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j < n && rom[j] == rom[i] {
            j += 1;
        }
        if j - i >= 16 {
            blank[i..j].iter_mut().for_each(|b| *b = true);
        }
        i = j;
    }
    let nb = blank.iter().filter(|&&b| !b).count();
    println!("polygon ROM {} bytes, non-blank {} bytes ({:.1}%)", n * 4, nb * 4, 100.0 * nb as f64 / n as f64);
    let mut runs = Vec::new();
    for a in args {
        let (l, p) = a.split_once('=').ok_or("LABEL=FILE")?;
        runs.push(load(l, p, n, mask)?);
    }
    let mut uni = vec![false; n];
    let (mut us, mut ue) = (BTreeSet::new(), BTreeSet::new());
    println!("{:<12} {:>10} {:>8} {:>8} | {:>10} {:>8} {:>8} | added bytes (nonblank)", "run", "read B", "/total", "/nonblk", "union B", "/total", "/nonblk");
    for r in &runs {
        let cnt = |v: &[bool]| (v.iter().filter(|&&b| b).count(), v.iter().zip(&blank).filter(|(&a, &b)| a && !b).count());
        let (rt, rn) = cnt(&r.read);
        let before = cnt(&uni);
        for k in 0..n {
            uni[k] |= r.read[k];
        }
        us.extend(r.starts.iter().copied());
        ue.extend(r.ends.iter().copied());
        let (ut, un) = cnt(&uni);
        println!(
            "{:<12} {:>10} {:>7.2}% {:>7.2}% | {:>10} {:>7.2}% {:>7.2}% | +{} ({} non-blank)",
            r.label,
            rt * 4,
            100.0 * rt as f64 / n as f64,
            100.0 * rn as f64 / nb as f64,
            ut * 4,
            100.0 * ut as f64 / n as f64,
            100.0 * un as f64 / nb as f64,
            (ut - before.0) * 4,
            (un - before.1) * 4
        );
    }
    // Unread non-blank stretches.
    let mut st: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < n {
        if !uni[i] && !blank[i] {
            let s = i;
            while i < n && !uni[i] && !blank[i] {
                i += 1;
            }
            st.push((s, i));
        } else {
            i += 1;
        }
    }
    let total: usize = st.iter().map(|s| s.1 - s.0).sum();
    let mut sizes: Vec<usize> = st.iter().map(|s| (s.1 - s.0) * 4).collect();
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    let small: usize = st.iter().filter(|s| (s.1 - s.0) * 4 < 16384).map(|s| (s.1 - s.0) * 4).sum();
    println!("unread non-blank stretches: {} totalling {} bytes", st.len(), total * 4);
    println!("ten largest (bytes): {:?}", &sizes[..sizes.len().min(10)]);
    println!("in stretches under 16 KB (between read data): {} bytes; in 16 KB and over: {} bytes", small, total * 4 - small);
    // Back to back.
    let near = |s: u32, d: u32| (0..=d).any(|k| ue.contains(&s.wrapping_sub(k)));
    let ns = us.len();
    println!(
        "object starts {} ends {}: start == an end {} ; end+1..2 {} ; end+1..8 {}",
        ns,
        ue.len(),
        us.iter().filter(|&&s| ue.contains(&s)).count(),
        us.iter().filter(|&&s| near(s, 2)).count(),
        us.iter().filter(|&&s| near(s, 8)).count()
    );
    // Parse test: the parser on read models (validation), then on unread stretches.
    let ok_strict = us.iter().filter(|&&s| parse_x(rom, s as usize, n, true).is_some()).count();
    println!("strict parser on read model starts: {ok_strict} of {}", us.len());
    let ok_read = us.iter().filter(|&&s| parse(rom, s as usize, n).is_some()).count();
    println!("parser on {} read model starts: {} parse to a terminator", ns, ok_read);
    let ok_ends: usize = us
        .iter()
        .filter(|&&s| match parse(rom, s as usize, n) {
            Some(_) => true,
            None => false,
        })
        .count();
    let _ = ok_ends;
    let big: Vec<&(usize, usize)> = st.iter().filter(|s| s.1 - s.0 >= 8).collect();
    let after_read = big.iter().filter(|s| s.0 > 0 && uni[s.0 - 1]).count();
    let clean = big.iter().filter(|s| parse(rom, s.0, s.1).is_some()).count();
    let strict = big.iter().filter(|s| parse_x(rom, s.0, s.1, true).is_some()).count();
    println!("strict parse from the start of those stretches: {strict} of {}", big.len());
    let clean_after = big.iter().filter(|s| s.0 > 0 && uni[s.0 - 1] && parse(rom, s.0, s.1).is_some()).count();
    println!(
        "unread stretches of 8+ words: {}; begin right after read data: {}; parse cleanly from their start: {} (of those right after read data: {})",
        big.len(),
        after_read,
        clean,
        clean_after
    );
    // Stretch-level: how much of the unread bytes is covered by greedy model parsing from stretch starts.
    let mut covered = 0usize;
    for s in &big {
        let mut at = s.0;
        while let Some(_) = parse_x(rom, at, s.1, true) {
            // advance past the model: re-walk to find its end
            let mut i = at + 6;
            loop {
                let a = rom[i];
                i += 1;
                if a & 3 == 0 {
                    break;
                }
                i += if a & 1 != 0 { 6 } else { 3 };
            }
            covered += i - at;
            at = i;
        }
    }
    println!("greedy back-to-back strict parse of the unread stretches covers {} of {} bytes", covered * 4, total * 4);
    Ok(())
}
