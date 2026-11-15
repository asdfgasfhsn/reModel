//! The JIT against the interpreter on random programs (no game code):
//! the instruction types a block holds and the ones it doesn't, random
//! registers, memory and modes, runs of every length from one word (the
//! copro's slices cut blocks anywhere), compared in full after every run.

use super::*;
use ::adsp2106x::compute::Compute;
use ::adsp2106x::cpu::{Width, C6, DMAC6, EPB0, II6, IM6};
use ::adsp2106x::Op;

struct Rng(u64);

impl Rng {
    fn next64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn next(&mut self) -> u32 {
        (self.next64() >> 16) as u32
    }
    fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
    fn chance(&mut self, n: u32) -> bool {
        self.below(n) == 0
    }
    /// A 40-bit register value: floats of ordinary size (40- and 32-bit),
    /// small integers, special values, random bits.
    fn value(&mut self) -> u64 {
        let f32bits = |r: &mut Rng, e: u32| ((r.below(2) << 31) | (e << 23) | (r.next() & 0x7f_ffff)) as u64;
        match self.below(8) {
            0 => self.next64() & 0xff_ffff_ffff,
            1 => (self.below(64) as u64) << 8,
            2 => {
                ([0u32, 0x8000_0000, 0x7f80_0000, 0xff80_0000, 0x7fc0_0000, 0x0000_0001, 0x3f80_0000, 0xbf80_0000]
                    [self.below(8) as usize] as u64)
                    << 8
            }
            3 => {
                let e = self.below(256);
                (f32bits(self, e) << 8) | self.below(256) as u64
            }
            4 => ((self.next() as i32 >> self.below(31)) as u32 as u64) << 8,
            _ => {
                let e = 0x70 + self.below(24);
                (f32bits(self, e) << 8) | if self.chance(2) { self.below(256) as u64 } else { 0 }
            }
        }
    }
}

fn put(w: u64, hi: u32, lo: u32, v: u64) -> u64 {
    let mask = ((1u64 << (hi - lo + 1)) - 1) << lo;
    (w & !mask) | ((v << lo) & mask)
}

/// Where programs go, and how long they are.
const PROGRAM: u32 = 0x2_0080;

/// A compute field that decodes.
fn compute(r: &mut Rng) -> u64 {
    loop {
        let c = r.next() & 0x7f_ffff;
        if !matches!(Compute::decode(c), Compute::Unknown(_)) {
            return c as u64;
        }
    }
}

/// A condition: mostly TRUE, else any.
fn cond(r: &mut Rng) -> u64 {
    if r.chance(3) {
        r.below(32) as u64
    } else {
        31
    }
}

/// A data address the program can use: internal block 1 (data), block 0
/// above the program, short words, the external port, now and then the
/// program itself.
fn data_addr(r: &mut Rng, len: u32) -> u32 {
    match r.below(12) {
        0..=4 => 0x2_8000 + r.below(64),
        5..=6 => 0x2_4000 + r.below(64),
        7 => 0x5_0000 + r.below(128),
        8..=9 => 0x40_0000 + r.below(32),
        10 => PROGRAM + len + r.below(16),
        _ => PROGRAM + r.below(len),
    }
}

/// A universal register for a transfer: mostly the register file, the
/// DAG registers and PX; now and then a system register (which a block
/// can't hold).
fn ureg(r: &mut Rng, write_to_reg: bool) -> u64 {
    match r.below(16) {
        0..=10 => r.below(16) as u64,
        11 if !write_to_reg => (0x10 + r.below(64)) as u64,
        12 => 0xdb + r.below(3) as u64,
        13 => [0x70, 0x71, 0x7c, 0x7e][r.below(4) as usize],
        _ => r.below(16) as u64,
    }
}

/// A random word at `addr` of a program of `len` words: every type,
/// branch and loop targets within the program, data addresses in reach.
fn random_word(r: &mut Rng, addr: u32, len: u32) -> u64 {
    loop {
        let mut w = r.next64() & ::adsp2106x::WORD_MASK;
        let rel = |r: &mut Rng| (PROGRAM + r.below(len)).wrapping_sub(addr) as u64 & 0xff_ffff;
        w = match r.below(32) {
            // Type 1: compute and two register file transfers.
            0..=6 => put(put(w, 47, 45, 0b001), 22, 0, compute(r)),
            // Type 2: conditional compute.
            7..=10 => put(put(put(w, 47, 40, 0b000_00001), 37, 33, cond(r)), 22, 0, compute(r)),
            // Type 3: compute and a universal register transfer.
            11..=13 => {
                let write = r.below(2) as u64;
                let w = put(put(w, 47, 45, 0b010), 31, 31, write);
                put(put(put(w, 30, 23, ureg(r, write == 0)), 37, 33, cond(r)), 22, 0, compute(r))
            }
            // Type 4 (register file, immediate modifier).
            14..=15 => {
                put(put(put(put(w, 47, 44, 0b0110), 37, 33, cond(r)), 22, 0, compute(r)), 32, 27, r.below(8) as u64)
            }
            // Type 5: register to register.
            16 => {
                let w = put(put(w, 47, 44, 0b0111), 43, 36, ureg(r, false));
                put(put(put(w, 30, 23, ureg(r, true)), 35, 31, cond(r)), 22, 0, compute(r))
            }
            // Type 6: shift immediate, with or without a transfer.
            17..=18 => {
                let w = if r.chance(2) { put(w, 47, 44, 0b1000) } else { put(w, 47, 40, 0b000_00010) };
                put(put(w, 37, 33, cond(r)), 22, 22, 0)
            }
            // Type 7: MODIFY with a compute.
            19 => put(put(put(w, 47, 40, 0b000_00100), 37, 33, cond(r)), 22, 0, compute(r)),
            // Type 8/9 branches and calls; type 11 returns (rare).
            20 => {
                let w = put(w, 47, 40, if r.chance(2) { 0b000_00110 } else { 0b000_00111 });
                let w = put(put(w, 37, 33, cond(r)), 39, 39, r.chance(6) as u64);
                let t = if w >> 40 & 1 == 0 { (PROGRAM + r.below(len)) as u64 } else { rel(r) };
                put(put(w, 23, 0, t), 38, 38, r.chance(8) as u64)
            }
            21 => {
                let w = put(w, 47, 40, 0b000_01001);
                let d = (r.below(9) as i64 - 4) as u64;
                put(put(put(w, 37, 33, cond(r)), 32, 27, d), 22, 0, compute(r))
            }
            22 if r.chance(4) => put(put(w, 47, 40, 0b000_01010), 37, 33, cond(r)),
            // Type 10: conditional jump, else compute and transfer.
            23 => {
                let w = put(w, 47, 45, 0b111);
                let d = (r.below(9) as i64 - 4) as u64;
                put(put(put(w, 37, 33, r.below(32) as u64), 32, 27, d), 22, 0, compute(r))
            }
            // Type 12/13 loops (short, small counts).
            24 => {
                let end = (r.below(6) + 1) as u64;
                if r.chance(3) {
                    put(put(put(w, 47, 40, 0b000_01110), 37, 33, r.below(32) as u64), 23, 0, end)
                } else {
                    put(put(put(w, 47, 40, 0b000_01100), 39, 24, r.below(5) as u64 + 1), 23, 0, end)
                }
            }
            // Type 14/15/16/17: direct, indirect immediate, immediate store and load.
            25 => {
                let w = put(put(w, 47, 42, 0b000_100), 39, 32, ureg(r, true));
                put(put(w, 31, 0, data_addr(r, len) as u64), 41, 41, 0)
            }
            26 => {
                let w = put(put(w, 47, 45, 0b101), 39, 32, ureg(r, true));
                put(w, 31, 0, (r.below(9) as i32 - 4) as u32 as u64)
            }
            27 => put(put(w, 47, 44, 0b1001), 31, 0, r.next() as u64),
            28..=29 => {
                let reg = if r.chance(2) { r.below(16) } else { 0x10 + r.below(64) };
                let data = if (0x10..0x20).contains(&reg) { data_addr(r, len) } else { r.next() };
                put(put(put(w, 47, 40, 0b000_01111), 39, 32, reg as u64), 31, 0, data as u64)
            }
            // Type 18 BIT on ASTAT/USTAT, type 19 MODIFY/BITREV immediate,
            // type 20 stacks, NOP.
            30 => match r.below(4) {
                0 => put(put(put(w, 47, 40, 0b000_10100), 35, 32, [0, 1, 0xc][r.below(3) as usize]), 36, 36, 0),
                1 => put(put(w, 47, 40, 0b000_10110), 31, 0, (r.below(9) as i32 - 4) as u32 as u64),
                2 => put(w, 47, 40, 0b000_10111) & !(0x3f << 34) | (1u64 << (34 + r.below(6))),
                _ => 0,
            },
            _ => 0,
        };
        let i = decode(addr, w);
        let ok = match i.op {
            Op::Unknown | Op::Idle | Op::Idle16 => false,
            // Mode and interrupt registers stay as set up.
            Op::RegMove { dst, .. } | Op::LoadImm { reg: dst, .. } => !matches!(dst, 0x79 | 0x7a | 0x7b | 0x7d | 0x7f),
            Op::Bit { sreg, .. } => matches!(sreg, 0x70 | 0x71 | 0x7c),
            _ => true,
        };
        if ok {
            return w;
        }
    }
}

/// A test bus: a small external memory and a step limit.
struct TestBus {
    ext: Vec<u64>,
    end: u64,
    writes: Vec<(u32, u64)>,
}

impl Bus for TestBus {
    fn read(&mut self, addr: u32, _: Width) -> u64 {
        self.ext[addr as usize & 0x3f]
    }
    fn write(&mut self, addr: u32, w: Width, v: u64) {
        let v = if w == Width::Data32 { v & 0xffff_ffff } else { v };
        self.ext[addr as usize & 0x3f] = v;
        self.writes.push((addr, v));
    }
    fn after_step(&mut self, cpu: &mut Sharc) -> bool {
        cpu.steps < self.end
    }
    fn block_budget(&mut self, cpu: &Sharc) -> u64 {
        self.end.saturating_sub(cpu.steps)
    }
}

/// A core booted the way the 2B board boots one (host DMA into block 0,
/// the boot interrupt's RTI, a jump to the program), stopped before the
/// program's first word.
fn boot(program: &[u64]) -> Sharc {
    let mut p = vec![0u64; (PROGRAM - 0x2_0000) as usize];
    p[0x05] = 0x063e_0002_0080; // JUMP 0x20080
    p[0x40] = 0x0b3e_0000_0000; // RTI
    p.extend_from_slice(program);
    let mut s = Sharc::new();
    let w32 = |s: &mut Sharc, reg, v: u32| {
        s.host_write(reg, v & 0xffff).unwrap();
        s.host_write(reg, v >> 16).unwrap();
    };
    w32(&mut s, DMAC6, 0);
    w32(&mut s, II6, 0x2_0000);
    w32(&mut s, IM6, 1);
    w32(&mut s, C6, p.len() as u32);
    w32(&mut s, DMAC6, 0xa1);
    for &w in &p {
        for k in 0..3 {
            s.host_write(EPB0, (w >> (16 * k)) as u32 & 0xffff).unwrap();
        }
    }
    let mut bus = TestBus { ext: vec![0; 64], end: u64::MAX, writes: Vec::new() };
    for _ in 0..16 {
        if s.compiled_pc() == PROGRAM {
            return s;
        }
        assert_eq!(s.step(&mut bus), Step::Ran, "boot");
    }
    panic!("the boot didn't reach the program");
}

/// Random registers, DAGs pointing at data, modes.
fn randomise(s: &mut Sharc, r: &mut Rng, len: u32) {
    for k in 0..16 {
        s.r[k] = r.value();
        s.r_alt[k] = r.value();
        s.i[k] = if k < 8 { data_addr(r, len) } else { data_addr(r, len) & 0xff_ffff };
        s.m[k] = (r.below(7) as i32 - 3) as u32 & if k < 8 { u32::MAX } else { 0xff_ffff };
        s.l[k] = if r.chance(4) { r.below(8) } else { 0 };
        s.b[k] = s.i[k].wrapping_sub(r.below(4));
    }
    s.mr = [((r.next64() as i128) << 16) >> 16, r.next64() as i32 as i128];
    s.px = r.next64() & ::adsp2106x::WORD_MASK;
    // RND32 mostly (as the programs run), TRUNC, ALUSAT, SSE, SRCU now
    // and then; bit reversal rarely.
    let mut m1 = if r.chance(4) { 0 } else { ::adsp2106x::cpu::RND32 };
    for (bit, n) in [
        (::adsp2106x::cpu::TRUNC, 4),
        (::adsp2106x::cpu::ALUSAT, 4),
        (::adsp2106x::cpu::SSE, 4),
        (::adsp2106x::cpu::SRCU, 4),
        (::adsp2106x::cpu::BR0, 16),
        (::adsp2106x::cpu::BR8, 16),
    ] {
        if r.chance(n) {
            m1 |= bit;
        }
    }
    s.set_mode1(m1);
    s.flag_in = r.below(16) as u8;
    s.ustat1 = r.next();
}

/// Where the two cores differ, if anywhere.
fn differs(a: &Sharc, b: &Sharc) -> Option<&'static str> {
    if a.digest() != b.digest() {
        return Some("registers, stacks or pipeline");
    }
    if a.mem.cols != b.mem.cols {
        return Some("internal memory");
    }
    if a.counts != b.counts {
        return Some("execution counts");
    }
    if a.iop != b.iop {
        return Some("IOP registers");
    }
    None
}

/// `programs` random programs of `len` words, each run on both in runs
/// of every budget in `runs` (cycled) for `steps` instructions.
fn compare(seed: u64, programs: usize, len: u32, steps: u64) {
    let mut r = Rng(seed);
    let runs = [1u64, 2, 3, 5, 7, 50, 256];
    let mut stats = CoproStats::default();
    for p in 0..programs {
        let words: Vec<u64> = (0..len).map(|k| random_word(&mut r, PROGRAM + k, len)).collect();
        let state = r.next64();
        let make = || {
            let mut s = boot(&words);
            randomise(&mut s, &mut Rng(state), len);
            s
        };
        let (mut want, mut got) = (make(), make());
        let mut jit = SharcJit::new().unwrap();
        jit.set_background(false);
        let ext: Vec<u64> = (0..64).map(|_| r.next64() & ::adsp2106x::WORD_MASK).collect();
        let mut wb = TestBus { ext: ext.clone(), end: 0, writes: Vec::new() };
        let mut gb = TestBus { ext, end: 0, writes: Vec::new() };
        let mut k = 0;
        while want.steps < steps {
            let n = runs[(p + k) % runs.len()];
            k += 1;
            let (pc, at) = (want.pc, want.steps);
            wb.end = want.steps + n;
            gb.end = got.steps + n;
            let w = want.run_bus(&mut wb);
            let g = jit.run(&mut got, &mut gb);
            let what = differs(&want, &got)
                .or((w != g).then_some("how the run ended"))
                .or((wb.ext != gb.ext || wb.writes != gb.writes).then_some("external memory"));
            if let Some(what) = what {
                let listing: Vec<String> =
                    words.iter().enumerate().map(|(j, &w)| format!("{}", decode(PROGRAM + j as u32, w))).collect();
                panic!(
                    "program {p} (seed {seed:#x}): {what} differ after a run of up to {n} from pc {pc:#x}, step {at} \
                     (interpreter {w:?} at {:#x}, JIT {g:?} at {:#x})\n{}",
                    want.pc,
                    got.pc,
                    listing.join("\n")
                );
            }
            if w != Step::Ran {
                break;
            }
        }
        stats.add(&jit.stats());
    }
    // The blocks ran (the programs aren't all faults and single steps).
    assert!(stats.runs > 4 * programs as u64, "{stats:?}");
}

#[test]
fn random_programs() {
    let (programs, steps) = if cfg!(debug_assertions) { (12, 400) } else { (400, 4000) };
    compare(0x5eed_2106, programs, 48, steps);
}

#[test]
fn straight_line_programs() {
    // Short programs, so the runs loop through them many times.
    let (programs, steps) = if cfg!(debug_assertions) { (12, 300) } else { (300, 3000) };
    compare(0x0b10_c4ed, programs, 12, steps);
}
