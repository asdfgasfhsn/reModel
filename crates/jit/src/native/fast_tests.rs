//! The inline plain-memory paths and the quiet step (docs/jit.md, "Plain
//! memory inline", "A cheaper step") against the interpreter on random
//! programs: hits and misses of every page kind, the edges of each page
//! and of the span, unaligned accesses, writes into code pages and into
//! the block's own code, I/O with side effects and events between quiet
//! stretches.

use super::*;
use i960::cpu::Bus;

/// A deterministic generator (xorshift).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
    fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
}

// The map: page 0 ROM (reads inline), page 1 code RAM (writes counted per
// KB), page 2 data RAM, page 3 I/O (reads count up, writes are logged),
// the last page of the span a mirror of page 2 (inline), and page 2 again
// just above the span (through the bus only).
const RAM: u32 = 0x1_0000;
const DATA: u32 = 0x2_0000;
const IO: u32 = 0x3_0000;
const TOP: u32 = FAST_SPAN - FAST_PAGE;

struct Mapped {
    mem: Vec<u8>,
    /// Write counts of the two RAM pages, and the code RAM's counters.
    writes: [u64; 2],
    gens: Vec<u32>,
    fast: Vec<FastPage>,
    /// Retires so far, and every one at an event (each `EVERY`th), with
    /// the CPU's instruction count then.
    now: u64,
    events: Vec<(u64, u64)>,
    io_reads: u32,
    io: Vec<(u64, u32, u32, u32)>,
    /// A deliberate error, to see that the tests catch it.
    bug: Option<Bug>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Bug {
    /// A quiet stretch one retire too long.
    QuietOverrun,
    /// The inline write counts only once in the code RAM's counters.
    GenAdd,
    /// The inline paths reach the data page for the I/O page.
    IoInline,
    /// Code RAM's writes not counted inline.
    NoGen,
}

const EVERY: u64 = 37;

impl Mapped {
    fn new(code: &[(u32, u32)], bug: Option<Bug>) -> Box<Self> {
        let mut m = Box::new(Mapped {
            mem: vec![0; 0x3_0000],
            writes: [0; 2],
            gens: vec![0; (FAST_PAGE >> GEN_SHIFT) as usize],
            fast: vec![FastPage::NONE; FAST_PAGES],
            now: 0,
            events: Vec::new(),
            io_reads: 0,
            io: Vec::new(),
            bug,
        });
        for &(a, w) in code {
            m.mem[a as usize..a as usize + 4].copy_from_slice(&w.to_le_bytes());
        }
        let base = m.mem.as_mut_ptr();
        let page = |k: u32| unsafe { base.add((k * FAST_PAGE) as usize) };
        let (w0, w1): (*mut u64, *mut u64) = (&mut m.writes[0], &mut m.writes[1]);
        let gens = m.gens.as_mut_ptr();
        m.fast[0] = FastPage { data: page(0), ..FastPage::NONE };
        let gen_add = if bug == Some(Bug::GenAdd) { 1 } else { 2 };
        let gen = if bug == Some(Bug::NoGen) { std::ptr::null_mut() } else { gens };
        m.fast[1] = FastPage { data: page(1), writes: w0, gen, gen_add };
        m.fast[2] = FastPage { data: page(2), writes: w1, ..FastPage::NONE };
        m.fast[(TOP >> FAST_SHIFT) as usize] = m.fast[2];
        if bug == Some(Bug::IoInline) {
            m.fast[3] = m.fast[2];
        }
        m
    }

    /// Where `a` is in `mem`, and which RAM page it is (0, 1) if any.
    fn loc(a: u32) -> Option<(usize, Option<usize>)> {
        let a = match a {
            0..IO => a,
            TOP.. if a < FAST_SPAN => a - TOP + DATA,
            FAST_SPAN.. if a < FAST_SPAN + FAST_PAGE => a - FAST_SPAN + DATA,
            _ => return None,
        };
        Some((a as usize, (a >= RAM).then_some((a >= DATA) as usize)))
    }

    fn read(&mut self, a: u32, n: u32) -> u32 {
        if a & !(FAST_PAGE - 1) == IO {
            self.io_reads += 1;
            self.io.push((self.now, a, n, u32::MAX));
            return self.io_reads.wrapping_mul(0x9e37_79b9);
        }
        (0..n)
            .map(|i| Self::loc(a.wrapping_add(i)).map_or(0, |(o, _)| self.mem[o] as u32) << (8 * i))
            .fold(0, |x, y| x | y)
    }

    /// As `Model2Bus` counts: one write; its first and last bytes' code
    /// counters bumped.
    fn write(&mut self, a: u32, v: u32, n: u32) {
        if a & !(FAST_PAGE - 1) == IO {
            self.io.push((self.now, a, n, v));
            return;
        }
        let Some((_, Some(page))) = Self::loc(a) else { return };
        self.writes[page] += 1;
        for o in [a, a.wrapping_add(n - 1)] {
            if let Some((o, Some(0))) = Self::loc(o) {
                self.gens[(o - RAM as usize) >> GEN_SHIFT] += 1;
            }
        }
        for i in 0..n {
            if let Some((o, Some(_))) = Self::loc(a.wrapping_add(i)) {
                self.mem[o] = (v >> (8 * i)) as u8;
            }
        }
    }
}

impl Bus for Mapped {
    fn read8(&mut self, a: u32) -> u8 {
        self.read(a, 1) as u8
    }
    fn read16(&mut self, a: u32) -> u16 {
        self.read(a, 2) as u16
    }
    fn read32(&mut self, a: u32) -> u32 {
        self.read(a, 4)
    }
    fn write8(&mut self, a: u32, v: u8) {
        self.write(a, v as u32, 1)
    }
    fn write16(&mut self, a: u32, v: u16) {
        self.write(a, v as u32, 2)
    }
    fn write32(&mut self, a: u32, v: u32) {
        self.write(a, v, 4)
    }
    fn fetch(&self, a: u32) -> Option<u32> {
        let (o, _) = Self::loc(a).filter(|&(o, _)| o + 4 <= self.mem.len())?;
        Some(u32::from_le_bytes(self.mem[o..o + 4].try_into().unwrap()))
    }
    fn is_rom(&self, a: u32) -> bool {
        a < RAM
    }
}

impl Host for Mapped {
    fn retire(&mut self, cpu: &mut Cpu) -> bool {
        self.now += 1;
        if self.now.is_multiple_of(EVERY) {
            self.events.push((self.now, cpu.icount));
        }
        true
    }
}

impl JitHost for Mapped {
    fn code_generations(&self) -> &[u32] {
        &self.gens
    }
    fn code_generation_index(&self, a: u32) -> Option<usize> {
        ((RAM..DATA).contains(&a)).then(|| ((a - RAM) >> GEN_SHIFT) as usize)
    }
    fn fast_pages(&mut self) -> *const FastPage {
        self.fast.as_ptr()
    }
    fn quiet(&mut self) -> u64 {
        // The retires before the next event's.
        let q = EVERY - 1 - self.now % EVERY;
        q + (self.bug == Some(Bug::QuietOverrun)) as u64
    }
    fn retire_quiet(&mut self, _cpu: &mut Cpu, n: u64) {
        self.now += n;
    }
}

// Encoders (80960KB programmer's reference).
fn reg(full: u32, src1: u32, lit1: bool, src2: u32, dst: u32) -> u32 {
    (full >> 4) << 24 | dst << 19 | src2 << 14 | (lit1 as u32) << 11 | (full & 0xf) << 7 | src1
}
fn mema(op: u32, reg: u32, abase: u32, off: u32) -> u32 {
    op << 24 | reg << 19 | abase << 14 | 0x2000 | off
}
fn ctrl(op: u32, from: u32, to: u32) -> u32 {
    op << 24 | (to.wrapping_sub(from) & 0xff_fffc)
}
const G: u32 = 16;

/// Addresses near the edges of every page kind, and of the counters'
/// KBs, the span and the code.
fn address(r: &mut Rng, code: (u32, u32)) -> u32 {
    let edges = [
        0,
        0x100,
        FAST_PAGE - 0x10,
        RAM,
        RAM + 0x3f0,
        RAM + 0x7f8,
        code.0,
        code.1 - 0x10,
        DATA - 0x10,
        DATA,
        DATA + 0x5000,
        IO - 0x10,
        IO,
        IO + 0x40,
        TOP - 0x10,
        TOP,
        FAST_SPAN - 0x10,
        FAST_SPAN,
        0x8000_0000,
    ];
    let e = edges[r.below(edges.len() as u32) as usize];
    e.wrapping_add(r.below(0x18)).wrapping_sub(4)
}

/// A random program at `RAM + 0x800`: addresses loaded and used by
/// loads and stores of every width, some ALU work on the values, a store
/// now and then over its own patch slot (a `mov` it runs, rewritten
/// between two forms), a branch back to the start.
fn program(r: &mut Rng) -> (Vec<(u32, u32)>, u32) {
    let start = RAM + 0x800;
    let len = 8 + r.below(48);
    let patch = start + 4 * (1 + 2 * r.below(len / 2 - 1));
    let end = start + 8 * len + 8;
    let mut code: Vec<(u32, u32)> = Vec::new();
    let emit = |w: u32, code: &mut Vec<(u32, u32)>| code.push((start + 4 * code.len() as u32, w));
    let ops = [0x80, 0x82, 0x88, 0x8a, 0x90, 0x92, 0x98, 0x9a, 0xb0, 0xb2, 0xc0, 0xc8, 0xc2, 0xca];
    let mut k = 0;
    loop {
        let at = start + 4 * code.len() as u32;
        if at >= end - 8 {
            break;
        }
        if at == patch {
            emit(reg(0x5cc, 1, true, 0, G + 14), &mut code); // mov 1, g14 (patched)
            continue;
        }
        if k % 2 == 0 {
            // lda ADDR, gA (two words)
            if at + 8 > end - 8 || at + 4 == patch {
                emit(reg(0x590, G + 9, false, G + 8, G + 9), &mut code);
                continue;
            }
            let a = if r.below(6) == 0 { patch } else { address(r, (start, end)) };
            emit(0x8c00_3000 | (G + 1 + r.below(3)) << 19, &mut code);
            emit(a, &mut code);
        } else {
            let base = G + 1 + r.below(3);
            let w = match r.below(8) {
                0 => reg(0x590, G + 8, false, G + 9, G + 9),   // addo g8, g9, g9
                1 => reg(0x589, G + 10, false, G + 9, G + 10), // xor
                2 => mema(0x92, G + 14 + 1, base, 0),          // st g15, (gA): the patch's other form
                3 => mema(0x92, G + 12, base, 0),              // st g12, (gA): the patch's first form
                _ => {
                    let op = ops[r.below(ops.len() as u32) as usize];
                    let d = match op {
                        0x98 | 0x9a => G + 8,
                        0xb0 | 0xb2 => G + 8,
                        _ => G + 8 + r.below(4),
                    };
                    mema(op, d, base, r.below(8))
                }
            };
            emit(w, &mut code);
        }
        k += 1;
    }
    let at = start + 4 * code.len() as u32;
    emit(ctrl(0x08, at, start), &mut code); // b start
    (code, start)
}

fn cpu_for(r: &mut Rng, ip: u32) -> Cpu {
    let mut c = Cpu::new();
    c.ip = ip;
    c.r[1] = DATA + 0x8000;
    c.r[31] = DATA + 0x7fc0;
    for g in 8..12 {
        c.r[16 + g] = r.next();
    }
    c.r[16 + 12] = reg(0x5cc, 1, true, 0, G + 14);
    c.r[16 + 15] = reg(0x5cc, 2, true, 0, G + 14);
    c
}

/// Run `steps` instructions of a random program on the JIT and the
/// interpreter, comparing the CPUs after every block, then memory, the
/// counters, the I/O and the events. `Err` on the first difference.
fn compare(seed: u64, steps: u64, bug: Option<Bug>, background: bool) -> Result<Stats, String> {
    let mut r = Rng(seed);
    let (code, start) = program(&mut r);
    let mut r2 = Rng(seed ^ 0x5555);
    let (mut b, mut hb) = (cpu_for(&mut r2, start), Mapped::new(&code, bug));
    let mut r2 = Rng(seed ^ 0x5555);
    let (mut a, mut ha) = (cpu_for(&mut r2, start), Mapped::new(&code, None));
    let hot = r.below(3);
    let mut jit = Jit::<Mapped>::new(Config { hot, background, ..Config::default() }).unwrap();
    while b.icount < steps {
        match jit.run(&mut b, &mut hb) {
            Some(Err(_)) => break,
            Some(Ok(())) => {}
            None => {
                if b.step_insn(&mut *hb).is_err() {
                    break;
                }
                hb.retire(&mut b);
            }
        }
        while a.icount < b.icount {
            if a.step(&mut *ha).is_err() {
                return Err(format!("the interpreter faulted at {:#x}, the JIT didn't", a.ip));
            }
            ha.retire(&mut a);
        }
        if a.arch_state() != b.arch_state() {
            return Err(format!("registers differ after {} instructions", b.icount));
        }
        if ha.now != hb.now {
            return Err(format!("retires differ after {} instructions: {} {}", b.icount, ha.now, hb.now));
        }
    }
    if ha.mem != hb.mem {
        return Err("memory differs".into());
    }
    if (ha.writes, &ha.gens) != (hb.writes, &hb.gens) {
        return Err(format!("write counts differ: {:?} {:?}", ha.writes, hb.writes));
    }
    if ha.io != hb.io {
        return Err("I/O differs".into());
    }
    if ha.events != hb.events {
        return Err("events differ".into());
    }
    Ok(jit.stats())
}

#[test]
fn random_memory_programs_match_the_interpreter() {
    let mut total = Stats::default();
    for seed in 1..=300u64 {
        let s = compare(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15), 3000, None, false)
            .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        total.blocks += s.blocks;
        total.runs += s.runs;
        total.dropped += s.dropped;
    }
    assert!(total.runs > 10_000 && total.dropped > 0, "{total:?}");
}

#[test]
fn random_memory_programs_translated_in_the_background() {
    for seed in 1..=60u64 {
        compare(seed.wrapping_mul(0x2545_f491_4f6c_dd1d), 3000, None, true)
            .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
    }
}

#[test]
fn deliberate_errors_are_caught() {
    for bug in [Bug::QuietOverrun, Bug::GenAdd, Bug::IoInline, Bug::NoGen] {
        let caught =
            (1..=100u64).any(|seed| compare(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15), 3000, Some(bug), false).is_err());
        assert!(caught, "{bug:?} not caught");
    }
}

#[test]
fn the_helpers_plain_bus_matches_the_machine() {
    // `Plain` (calls, returns and fallbacks) against the bus itself, at
    // the same edges.
    let mut r = Rng(77);
    let (mut x, mut y) = (Mapped::new(&[], None), Mapped::new(&[], None));
    for _ in 0..20_000 {
        let a = address(&mut r, (RAM + 0x800, RAM + 0x900));
        let v = r.next();
        let fast = y.fast.as_ptr();
        let mut p = Plain { h: &mut *y, fast };
        match r.below(6) {
            0 => assert_eq!(x.read8(a), p.read8(a), "{a:#x}"),
            1 => assert_eq!(x.read16(a), p.read16(a), "{a:#x}"),
            2 => assert_eq!(x.read32(a), p.read32(a), "{a:#x}"),
            3 => (x.write8(a, v as u8), p.write8(a, v as u8)).1,
            4 => (x.write16(a, v as u16), p.write16(a, v as u16)).1,
            _ => (x.write32(a, v), p.write32(a, v)).1,
        }
    }
    assert!(x.mem == y.mem && x.writes == y.writes && x.gens == y.gens && x.io == y.io);
}
