//! The JIT against the interpreter on random programs (no game code):
//! every format, random state and input, runs of every length from one
//! word (the copro's two-word slices cut blocks anywhere).

use super::*;

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
    /// A register value: floats of ordinary size, small integers, the
    /// special values, random bits.
    fn value(&mut self) -> u32 {
        match self.below(8) {
            0 => self.next(),
            1 => self.below(64),
            2 => [0, 0x8000_0000, 0x7f80_0000, 0xff80_0000, 0x7fc0_0000, 0x7f80_0001, 0x0000_0001, 0x437f_0000]
                [self.below(8) as usize],
            3 => (self.below(2) << 31) | (self.below(255) << 23) | (self.next() & 0x7f_ffff),
            _ => (self.below(2) << 31) | ((0x70 + self.below(24)) << 23) | (self.next() & 0x7f_ffff),
        }
    }
}

fn put(w: u64, hi: u32, lo: u32, v: u64) -> u64 {
    let mask = ((1u64 << (hi - lo + 1)) - 1) << lo;
    (w & !mask) | ((v << lo) & mask)
}

/// A random word for a program of `len` words: any format, with branch
/// targets and addressing kept within reach.
fn random_word(r: &mut Rng, len: u32) -> u64 {
    let mut w = r.next64();
    // Words the model can't execute (format 3, MOV4, MOVI) end a run:
    // rare, so that runs go on.
    let format =
        if r.below(64) == 0 { 3 } else { [0, 1, 1, 2, 2, 4, 5, 5, 6, 6, 6, 7, 7][r.below(13) as usize] as u64 };
    w = put(w, 63, 61, format);
    if format == 5 && (w >> 39) & 3 == 2 && r.below(16) != 0 {
        w = put(w, 40, 39, 0);
    }
    if format == 4 && (w >> 38) & 3 == 3 && r.below(16) != 0 {
        w = put(w, 39, 38, 1);
    }
    // Memory operands mostly through AR0-AR3 (kept small), modes all.
    match format {
        2 | 6 => {
            let cop = [0, 1, 2, 3, 4, 5, 8, 9, 10, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x18, 0x19, 0x1a, 0x1b, 0x1f]
                [r.below(20) as usize] as u64;
            w = put(w, 26, 22, cop);
            // REP and SETL counts small and immediate, mostly.
            if cop == 1 || cop == 2 {
                w = put(w, 21, 16, if r.below(4) == 0 { 1 } else { 0 });
                w = put(w, 15, 0, r.below(4) as u64);
            } else if cop >= 0x10 {
                // Conditions: any (some undefined).
                w = put(w, 21, 16, r.below(0x1a) as u64);
                let ty = [0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7][r.below(11) as usize] as u64;
                w = put(w, 15, 13, ty);
                w = put(w, 12, 12, (ty == 0 && r.below(4) == 0) as u64);
                w = put(w, 11, 0, r.below(len) as u64);
                if cop == 0x13 || cop == 0x14 || cop == 0x15 {
                    w = put(w, 11, 0, (r.below(16) as i32 - 8) as u64 & 0xfff);
                }
            }
        }
        _ => {}
    }
    // An ALU op: sometimes none.
    if r.below(4) == 0 {
        w = put(w, 60, 56, 7);
    }
    w
}

/// A random state, the same on both TGPs (the same generator state).
fn randomise(t: &mut Tgp, r: &mut Rng, len: u32) {
    for v in t.aram.iter_mut().chain(t.bram.iter_mut()) {
        *v = if r.below(4) == 0 { r.below(len) } else { r.value() };
    }
    for bank in [&mut t.aa, &mut t.ab, &mut t.ma, &mut t.mb] {
        for v in bank.iter_mut() {
            *v = if r.below(6) == 0 { r.below(len) } else { r.value() };
        }
    }
    for v in t.ar.iter_mut() {
        *v = if r.below(8) == 0 { 0x400 + r.below(0x40) } else { r.below(0x40) };
    }
    for v in t.pr.iter_mut() {
        *v = r.value();
    }
    t.prp = r.below(24);
    t.pwp = r.below(24);
    t.eb = r.next() & 0xff_ffff;
    t.eo = r.below(0x100);
    t.sp = r.below(0x400);
    t.st = r.next() & !((1 << 14) | (1 << 15));
    t.md = r.next() & 0xffff;
    t.lpc = r.below(6);
    t.pdr = r.next();
    t.ddr = r.next();
    t.pcs = [r.below(len), r.below(len), r.below(len), r.below(len)];
    t.pcsp = r.below(4) as usize;
    t.ext_flags = r.below(8);
}

/// External memory of 4K words (mirrored).
struct Ext(Vec<u32>);

impl External for Ext {
    fn read(&mut self, a: u32) -> u32 {
        self.0[(a & 0xfff) as usize]
    }
    fn write(&mut self, a: u32, v: u32) {
        self.0[(a & 0xfff) as usize] = v;
    }
}

fn fuzz(jit: &mut Mb86235Jit, seed: u64, len: u32, runs: usize) {
    let mut r = Rng(seed);
    let mut program: Vec<u64> = (0..len).map(|_| random_word(&mut r, len)).collect();
    // A jump back to the start (and its delay slot) closes the program.
    program.push(0xc000_0000_0000_0000 | 0x7u64 << 56 | 1 << 41 | 0x12 << 22);
    program.push(0xc700_0200_0000_0000);
    let (mut a, mut b) = (Tgp::new(), Tgp::new());
    a.boot(&program);
    b.boot(&program);
    let state = r.next64();
    randomise(&mut a, &mut Rng(state), len);
    randomise(&mut b, &mut Rng(state), len);
    let mem: Vec<u32> = (0..0x1000).map(|_| r.value()).collect();
    let (mut ea, mut eb) = (Ext(mem.clone()), Ext(mem));
    for run in 0..runs {
        for _ in 0..r.below(4) {
            let v = r.value();
            a.fifo_in.push_back(v);
            b.fifo_in.push_back(v);
        }
        if r.below(4) == 0 {
            a.fifo_out.clear();
            b.fifo_out.clear();
        }
        let max = [1, 2, 2, 3, 7, 50, 256][r.below(7) as usize];
        let start = (a.pc, a.pending());
        let sa = a.run(&mut ea, max);
        let sb = jit.run(&mut b, &mut eb, max);
        let what =
            differs(&a, &b).or((sa != sb).then_some("the step result")).or((ea.0 != eb.0).then_some("external memory"));
        if let Some(what) = what {
            panic!(
                "seed {seed} run {run} (max {max}): {what} differs (from pc {:#x} pending {:?}; interpreter {sa:?} at \
                 {:#x}, JIT {sb:?} at {:#x})",
                start.0, start.1, a.pc, b.pc
            );
        }
        if matches!(sa, Step::Unsupported(_)) {
            break;
        }
    }
}

/// Fewer programs in a debug build (`make check`); `make jit-test` runs
/// them all.
const SEEDS: u64 = if cfg!(debug_assertions) { 10 } else { 300 };

#[test]
fn random_programs_run_as_interpreted() {
    let mut jit = Mb86235Jit::new().expect("this machine has a Cranelift back end");
    for seed in 1..SEEDS {
        fuzz(&mut jit, seed.wrapping_mul(0x9e37_79b9_7f4a_7c15), 48, 200);
    }
    assert!(jit.stats().runs > 0);
}

#[test]
fn long_programs() {
    let mut jit = Mb86235Jit::new().expect("this machine has a Cranelift back end");
    for seed in 1..(SEEDS / 10).max(3) {
        fuzz(&mut jit, seed.wrapping_mul(0x2545_f491_4f6c_dd1d), 400, 400);
    }
}

#[test]
fn a_repeat_cut_by_the_budget() {
    // REP 5 ; ADD AA0, #1, AA0 (integer) ; then a jump back with a NOP in
    // its delay slot, run two words at a time.
    let rep = 0xc000_0000_0000_0000u64 | 0x7 << 56 | 1 << 41 | 0x01 << 22 | 5;
    let add = 0xc000_0000_0000_0000u64 | 0x10 << 56 | 0x19 << 47 | 0x10 << 42 | 1 << 41;
    let jmp = 0xc000_0000_0000_0000u64 | 0x7 << 56 | 1 << 41 | 0x12 << 22;
    let nop = 0xc700_0200_0000_0000u64;
    let program = [rep, add, jmp, nop];
    let (mut a, mut b) = (Tgp::new(), Tgp::new());
    a.boot(&program);
    b.boot(&program);
    let mut jit = Mb86235Jit::new().unwrap();
    let mut ext = Ext(vec![0; 0x1000]);
    for _ in 0..100 {
        assert_eq!(a.run(&mut ext, 2), jit.run(&mut b, &mut ext, 2));
        assert_eq!(differs(&a, &b), None);
    }
    assert_eq!(a.aa[0], b.aa[0]);
    assert!(a.aa[0] > 50);
}
