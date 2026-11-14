//! The JIT against the interpreter on random programs (no game code):
//! every word form, random state, random input, compared after every run.

use super::*;
use ::mb86234::Funcs;

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
    /// A register value: often a float of ordinary size, sometimes an
    /// integer, a small count, or a float of extreme exponent.
    fn value(&mut self) -> u32 {
        match self.below(8) {
            0 => self.next(),
            1 => self.below(64),
            2 => (self.next() & 0x807f_ffff) | (0xff << 23),
            3 => self.next() & 0x807f_ffff,
            4 => (self.next() & 0x807f_ffff) | ((0xf8 + self.below(8)) << 23),
            _ => ((self.below(2)) << 31) | ((0x70 + self.below(32)) << 23) | (self.next() & 0x7f_ffff),
        }
    }
}

/// A bus of 4K words (mirrored) that remembers what is written; unwritten words
/// read as a function of their address.
struct Mem {
    words: Vec<u32>,
}

impl Bus for Mem {
    fn read(&mut self, a: u32) -> u32 {
        self.words[(a & 0xfff) as usize]
    }
    fn write(&mut self, a: u32, v: u32) {
        self.words[(a & 0xfff) as usize] = v;
    }
}

/// A random word of program `len` words, of every form the interpreter
/// has (and some it hasn't).
fn random_word(r: &mut Rng, len: u32) -> u32 {
    let t = r.below(len);
    match r.below(20) {
        0..=5 => (r.below(2) << 26 | r.next() & 0x03ff_ffff) | if r.below(2) == 0 { 0 } else { 0x1c00_0000 },
        6 => 0x1c00_0000 | (r.below(4) + 0x1c) << 16 | r.next() & 0x00e0_ffff,
        7 => 0x3600_0000 | r.below(16) << 16 | r.next() & 0xffff,
        8 => (0x38 + r.below(4)) << 24 | r.next() & 0xff_ffff,
        9 => {
            // A control register, often the repeat count (kept small).
            let c = if r.below(2) == 0 { 4 } else { r.below(16) };
            let imm = if c == 4 { r.below(5) } else { r.next() & 0xffff };
            (0x3c + r.below(2)) << 24 | (r.below(32)) << 20 | c << 16 | imm
        }
        10..=12 => (0x40 + r.below(64)) << 24 | r.next() & 0xff_ffff,
        13 | 14 => {
            let cond = [0x00, 0x10, 0x20, 0xa0, 0x0a, 0x30][r.below(6) as usize] | [0, 4, 8, 0xc][r.below(4) as usize];
            let top = if r.below(2) == 0 { 0xbe } else { 0xfe };
            top << 24 | cond << 16 | r.next() & 0xfe00 | t & 0x1ff
        }
        15..=17 => {
            let op = [0x00, 0x10, 0x60, 0x64, 0x62, 0x66, 0x6a, 0x6e, 0x33][r.below(9) as usize];
            let low = match op {
                0x62 | 0x66 => {
                    if r.below(2) == 0 {
                        // Mostly through AR3 (kept small): a random PC
                        // would run off into the empty words after the
                        // program, which only costs time.
                        0x4000 | [0x03, 0x03, 0x03, 0x19][r.below(4) as usize]
                    } else {
                        r.below(0x80)
                    }
                }
                _ => t,
            };
            0xbf00_0000 | op << 16 | low
        }
        _ => r.next(),
    }
}

/// Both machines from the same random state.
fn random_cpu(r: &mut Rng, program: Vec<u32>) -> Cpu {
    let tables: Vec<u32> = (0..0x1_0000).map(|_| r.value()).collect();
    let mut c = Cpu::new(program, tables);
    for v in c.regs.iter_mut() {
        *v = r.value();
    }
    // Pointers that stay near the start of RAM and the program.
    for a in 0..4 {
        c.regs[a] = r.below(0x40);
    }
    c.regs[reg::BUS as usize] = r.below(2) << 22;
    c.shared_bus = c.regs[reg::BUS as usize] != 0;
    c.regs[reg::LC as usize] = r.below(8);
    c.regs[reg::LC2 as usize] = r.below(8);
    c.acc_in = c.regs[reg::ACC as usize];
    for v in c.ram.iter_mut() {
        *v = r.value();
    }
    // Jump targets in RAM0's direct words, often.
    for v in c.ram[..0x80].iter_mut() {
        if r.below(2) == 0 {
            *v = r.below(0x40);
        }
    }
    c.mode[0xa] = [4, 7][r.below(2) as usize];
    c.funcs = Funcs { angle: r.next(), recip: r.value(), rsqrt: r.value(), ..c.funcs.clone() };
    c
}

fn same(a: &Cpu, b: &Cpu) -> Result<(), String> {
    differs(a, b).map_or(Ok(()), |w| Err(w.to_string()))
}

/// `runs` runs of up to `limit` words of a random program on both, with
/// input between them.
fn fuzz(jit: &mut Mb86234Jit, seed: u64, len: u32, runs: usize, limit: u64) {
    let mut r = Rng(seed);
    let mut program: Vec<u32> = (0..len).map(|_| random_word(&mut r, len)).collect();
    program.push(0xbf60_0000);
    let mut interp = random_cpu(&mut r, program.clone());
    let mut jit_cpu = interp.clone();
    jit.reload(&program);
    let words: Vec<u32> = (0..0x1000).map(|_| r.value()).collect();
    let (mut ma, mut mb) = (Mem { words: words.clone() }, Mem { words });
    for run in 0..runs {
        for _ in 0..r.below(4) {
            let v = r.value();
            interp.fifo_in.push_back(v);
            jit_cpu.fifo_in.push_back(v);
        }
        let start = interp.pc;
        let sa = interp.run(&mut ma, limit);
        let sb = jit.run(&mut jit_cpu, &mut mb, limit);
        let what = same(&interp, &jit_cpu)
            .err()
            .or((sa != sb).then(|| "stop".into()))
            .or((ma.words != mb.words).then(|| "bus memory".into()));
        if let Some(what) = what {
            panic!(
                "seed {seed} run {run}: {what} differs (started at pc {:#x}, interpreter stopped at {:#x}, JIT at {:#x})",
                start, interp.pc, jit_cpu.pc
            );
        }
    }
}

/// Fewer programs in a debug build (`make check`); `make jit-test` runs
/// them all.
const SEEDS: u64 = if cfg!(debug_assertions) { 12 } else { 400 };

#[test]
fn random_programs_run_as_interpreted() {
    let mut jit = Mb86234Jit::new(&[]).expect("this machine has a Cranelift back end");
    for seed in 1..SEEDS {
        fuzz(&mut jit, seed * 0x9e37_79b9, 48, 40, 200);
    }
}

#[test]
fn long_runs_and_the_limit() {
    let mut jit = Mb86234Jit::new(&[]).expect("this machine has a Cranelift back end");
    for seed in 1..(SEEDS / 10).max(3) {
        fuzz(&mut jit, seed * 0x51_7cc1_b727, 200, 20, 5_000);
        // Limits that cut blocks short.
        fuzz(&mut jit, seed * 0x2545_f491, 64, 200, 3);
    }
}

#[test]
fn a_repeat_and_its_word() {
    // acc = 0; repeat 5: acc += r10 (integer); out = acc; wait for input.
    let program = vec![
        0x3b00_0000,            // acc = 0
        0x3900_0003,            // r10 = 3
        0x3c04_0005,            // repeat 5
        0x1c1f_1e10 | 26 << 21, // acc += r10 (ALU 26), a transfer to nowhere
        0x1c1f_4419,            // out = acc
        0x1c1f_2621,            // r13 = in
        0xbf60_0000,            // jmp 0
    ];
    let mut a = Cpu::new(program.clone(), Vec::new());
    let mut b = a.clone();
    let mut jit = Mb86234Jit::new(&program).unwrap();
    let sa = a.run(&mut ::mb86234::NoBus, 1000);
    let sb = jit.run(&mut b, &mut ::mb86234::NoBus, 1000);
    assert_eq!(sa, sb);
    same(&a, &b).unwrap();
    assert!(jit.stats().runs > 0);
}
