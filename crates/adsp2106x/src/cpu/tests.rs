//! Interpreter tests. Expected values come from the manual's definitions
//! (sections cited per test), not from running other software.

use super::*;
use crate::float;
use state::State;

/// External memory: a map, recording accesses.
#[derive(Default)]
struct Ext {
    mem: std::collections::HashMap<u32, u64>,
    log: Vec<(u32, Width, Option<u64>)>,
}

impl Bus for Ext {
    fn read(&mut self, a: u32, w: Width) -> u64 {
        self.log.push((a, w, None));
        self.mem.get(&a).copied().unwrap_or(0)
    }
    fn write(&mut self, a: u32, w: Width, v: u64) {
        self.log.push((a, w, Some(v)));
        self.mem.insert(a, v);
    }
}

fn put(fields: &[(u32, u32, u64)]) -> u64 {
    fields.iter().fold(0, |w, &(hi, lo, v)| w | ((v << lo) & (((1u64 << (hi - lo + 1)) - 1) << lo)))
}

// --- a small assembler ---------------------------------------------------

const T: u64 = 31; // TRUE
const NOP: u64 = 0;

fn alu(op: u64, n: u64, x: u64, y: u64) -> u64 {
    op << 12 | n << 8 | x << 4 | y
}
fn mulc(op: u64, n: u64, x: u64, y: u64) -> u64 {
    1 << 20 | alu(op, n, x, y)
}
fn shc(op: u64, n: u64, x: u64, y: u64) -> u64 {
    2 << 20 | alu(op, n, x, y)
}
/// Type 2: `IF cond compute`.
fn t2(cond: u64, cf: u64) -> u64 {
    put(&[(47, 40, 0x01), (37, 33, cond), (22, 0, cf)])
}
fn c(cf: u64) -> u64 {
    t2(T, cf)
}
/// Type 17: `ureg = data32`.
fn ld(ureg: u64, data: u32) -> u64 {
    put(&[(47, 40, 0x0f), (39, 32, ureg), (31, 0, data as u64)])
}
/// Type 8 with an absolute target.
fn jump(target: u32, cond: u64, call: bool, db: bool, la: bool) -> u64 {
    put(&[
        (47, 40, 0x06),
        (39, 39, call as u64),
        (38, 38, la as u64),
        (37, 33, cond),
        (26, 26, db as u64),
        (23, 0, target as u64),
    ])
}
/// Type 9 PC-relative with a compute and optional ELSE.
fn jrel(cond: u64, rel: i32, call: bool, db: bool, else_: bool, cf: u64) -> u64 {
    put(&[
        (47, 40, 0x09),
        (39, 39, call as u64),
        (37, 33, cond),
        (32, 27, rel as u64 & 0x3f),
        (26, 26, db as u64),
        (25, 25, else_ as u64),
        (22, 0, cf),
    ])
}
fn rts(cond: u64, db: bool, lr: bool) -> u64 {
    put(&[(47, 40, 0x0a), (37, 33, cond), (26, 26, db as u64), (24, 24, lr as u64)])
}
const RTI: u64 = 0x0b3e_0000_0000;
const IDLE: u64 = 0x0080_0000_0000;
/// Type 12: `LCNTR = n, DO (PC,rel) UNTIL LCE`.
fn do_lce(n: u16, rel: i32) -> u64 {
    put(&[(47, 40, 0x0c), (39, 24, n as u64), (23, 0, rel as u64 & 0xff_ffff)])
}
/// Type 13: `DO (PC,rel) UNTIL term`.
fn do_until(term: u64, rel: i32) -> u64 {
    put(&[(47, 40, 0x0e), (37, 33, term), (23, 0, rel as u64 & 0xff_ffff)])
}
/// Type 3: universal register <-> memory, M-register modifier.
fn t3(post: bool, i: u64, m: u64, pm: bool, write: bool, ureg: u64, cf: u64) -> u64 {
    put(&[
        (47, 45, 2),
        (44, 44, post as u64),
        (43, 41, i & 7),
        (40, 38, m & 7),
        (37, 33, T),
        (32, 32, pm as u64),
        (31, 31, write as u64),
        (30, 23, ureg),
        (22, 0, cf),
    ])
}
/// Type 1: dual move.
#[allow(clippy::too_many_arguments)]
fn t1(dw: bool, di: u64, dm: u64, dr: u64, pw: bool, pi: u64, pm: u64, pr: u64, cf: u64) -> u64 {
    put(&[
        (47, 45, 1),
        (44, 44, dw as u64),
        (43, 41, di),
        (40, 38, dm),
        (37, 37, pw as u64),
        (36, 33, dr),
        (32, 30, pi & 7),
        (29, 27, pm & 7),
        (26, 23, pr),
        (22, 0, cf),
    ])
}
/// Type 4: register file <-> memory, 6-bit immediate modifier.
fn t4(i: u64, pm: bool, write: bool, post: bool, data: i32, dreg: u64) -> u64 {
    put(&[
        (47, 44, 0b0110),
        (43, 41, i & 7),
        (40, 40, pm as u64),
        (39, 39, write as u64),
        (38, 38, post as u64),
        (37, 33, T),
        (32, 27, data as u64 & 0x3f),
        (26, 23, dreg),
    ])
}
/// Type 16: `DM(I,M) = data`.
fn st_imm(i: u64, m: u64, pm: bool, data: u32) -> u64 {
    put(&[(47, 44, 0b1001), (43, 41, i & 7), (40, 38, m & 7), (37, 37, pm as u64), (31, 0, data as u64)])
}
/// Type 14.
fn direct(pm: bool, write: bool, ureg: u64, addr: u32) -> u64 {
    put(&[(47, 42, 0b000100), (41, 41, pm as u64), (40, 40, write as u64), (39, 32, ureg), (31, 0, addr as u64)])
}
/// Type 18 on system register 0x70 | `sreg`.
fn bit(bop: u64, sreg: u64, data: u32) -> u64 {
    put(&[(47, 40, 0x14), (39, 37, bop), (35, 32, sreg), (31, 0, data as u64)])
}
const SET: u64 = 0;
const CLR: u64 = 1;
const TGL: u64 = 2;
const TST: u64 = 4;
const XOR: u64 = 5;
/// Type 6 without a transfer: `shiftop` is the 8-bit shifter opcode.
fn shimm(cond: u64, op: u64, rn: u64, rx: u64, imm: u64) -> u64 {
    put(&[
        (47, 40, 0x02),
        (37, 33, cond),
        (30, 27, imm >> 8),
        (21, 16, op >> 2),
        (15, 8, imm & 0xff),
        (7, 4, rn),
        (3, 0, rx),
    ])
}
/// Type 5: `dst = src`.
fn mv(src: u64, dst: u64, cf: u64) -> u64 {
    put(&[(47, 44, 0b0111), (43, 36, src), (35, 31, T), (30, 23, dst), (22, 0, cf)])
}
/// Type 19.
fn modimm(i: u64, data: i32, bitrev: bool) -> u64 {
    put(&[
        (47, 40, 0x16),
        (39, 39, bitrev as u64),
        (38, 38, (i >= 8) as u64),
        (34, 32, i & 7),
        (31, 0, data as u32 as u64),
    ])
}
/// Type 20, bits 39-33: LPU LPO SPU SPO PPU PPO FC.
fn stacks(bits: u64) -> u64 {
    put(&[(47, 40, 0x17), (39, 33, bits)])
}

// Universal register codes.
const R0: u64 = 0;
const R1: u64 = 1;
const R2: u64 = 2;
const I0: u64 = 0x10;
const M0: u64 = 0x20;
const L0: u64 = 0x30;
const B0: u64 = 0x40;
const LCNTR: u64 = 0x68;
const CURLCNTR: u64 = 0x67;
const IRPTL: u64 = 0x79;
const MODE2R: u64 = 0x7a;
const MODE1R: u64 = 0x7b;
const ASTATR: u64 = 0x7c;
const IMASKR: u64 = 0x7d;
const PXR: u64 = 0xdb;
const TPERIOD: u64 = 0xde;
const TCOUNT: u64 = 0xdf;

// ALU opcodes used below.
const ADD: u64 = 0x01;
const SUB: u64 = 0x02;
const INC: u64 = 0x29;
const DEC: u64 = 0x2a;
const FADD: u64 = 0x81;

fn boot(prog: &[u64]) -> (Sharc, Ext) {
    let mut s = Sharc::new();
    s.load48(0x2_0000, prog);
    s.start_at(0x2_0000);
    (s, Ext::default())
}

/// Run `n` instructions and return their addresses relative to 0x20000.
fn trace(s: &mut Sharc, e: &mut Ext, n: usize) -> Vec<u32> {
    (0..n)
        .map(|_| {
            assert_eq!(s.step(e), Step::Ran, "at {:#x}", s.pc);
            s.pc - 0x2_0000
        })
        .collect()
}

fn run(prog: &[u64], n: usize) -> Sharc {
    let (mut s, mut e) = boot(prog);
    trace(&mut s, &mut e, n);
    s
}

/// A 40-bit register value from an f32.
fn f(v: f32) -> u64 {
    (v.to_bits() as u64) << 8
}

fn fv(s: &Sharc, n: usize) -> f32 {
    f32::from_bits((s.r[n] >> 8) as u32)
}

// --- ALU fixed point (UM B-4 to B-23, §2.5) ------------------------------

#[test]
fn fixed_add_flags_and_saturation() {
    let mut p = vec![ld(R0, 0x7fff_ffff), ld(R1, 1), c(alu(ADD, 2, 0, 1))];
    let s = run(&p, 3);
    assert_eq!(s.rf(2), 0x8000_0000);
    assert_eq!(s.astat() & (AZ | AV | AN | AC), AV | AN, "overflow, no carry");
    assert_eq!(s.stky() & AOS, AOS, "sticky fixed-point overflow");
    // ALUSAT: positive overflow returns 0x7FFFFFFF; AV still set.
    p.insert(0, ld(MODE1R, ALUSAT));
    let s = run(&p, 4);
    assert_eq!(s.rf(2), 0x7fff_ffff);
    assert_eq!(s.astat() & (AV | AN), AV);
    // Negative overflow saturates to 0x80000000.
    let s = run(&[ld(MODE1R, ALUSAT), ld(R0, 0x8000_0000), ld(R1, 1), c(alu(SUB, 2, 0, 1))], 4);
    assert_eq!(s.rf(2), 0x8000_0000);
    // Carry out without overflow; the 8 LSBs of the result are zero.
    let s = run(&[ld(R0, 0xffff_ffff), ld(R1, 2), c(alu(ADD, 2, 0, 1))], 3);
    assert_eq!(s.r[2], 1 << 8);
    assert_eq!(s.astat() & (AZ | AV | AN | AC), AC);
}

#[test]
fn subtract_carry_and_borrow_forms() {
    // Rx - Ry: AC is the adder's carry, set when no borrow occurs.
    let s = run(&[ld(R0, 5), ld(R1, 3), c(alu(SUB, 2, 0, 1))], 3);
    assert_eq!((s.rf(2), s.astat() & AC), (2, AC));
    let s = run(&[ld(R0, 3), ld(R1, 5), c(alu(SUB, 2, 0, 1))], 3);
    assert_eq!((s.rf(2) as i32, s.astat() & (AC | AN)), (-2, AN));
    // Rx - Ry + CI - 1 with AC clear borrows one more.
    let s = run(&[ld(R0, 3), ld(R1, 5), c(alu(SUB, 2, 0, 1)), c(alu(0x06, 3, 0, 1))], 4);
    assert_eq!(s.rf(3) as i32, -3);
    // Rx + Ry + CI with AC set.
    let s = run(&[ld(R0, 0xffff_ffff), ld(R1, 1), c(alu(ADD, 2, 0, 1)), c(alu(0x05, 3, 1, 1))], 4);
    assert_eq!(s.rf(3), 3);
    // Rx - 1 of 0 wraps to -1 with no carry; Rx + 1 of MAX overflows.
    let s = run(&[ld(R0, 0), c(alu(DEC, 1, 0, 0))], 2);
    assert_eq!((s.rf(1), s.astat() & (AC | AN)), (u32::MAX, AN));
    let s = run(&[ld(R0, 0x7fff_ffff), c(alu(INC, 1, 0, 0))], 2);
    assert_eq!(s.astat() & AV, AV);
}

#[test]
fn average_rounds_to_nearest_even_or_truncates() {
    // (5 + 2)/2 = 3.5: nearest even 4, truncation 3 (UM B-8).
    let s = run(&[ld(R0, 5), ld(R1, 2), c(alu(0x09, 2, 0, 1))], 3);
    assert_eq!(s.rf(2), 4);
    let s = run(&[ld(R0, 7), ld(R1, 2), c(alu(0x09, 2, 0, 1))], 3);
    assert_eq!(s.rf(2), 4, "4.5 rounds to even");
    let s = run(&[ld(MODE1R, TRUNC), ld(R0, 5), ld(R1, 2), c(alu(0x09, 2, 0, 1))], 4);
    assert_eq!(s.rf(2), 3);
    let s = run(&[ld(R0, (-5i32) as u32), ld(R1, 0), c(alu(0x09, 2, 0, 1))], 3);
    assert_eq!(s.rf(2) as i32, -2, "-2.5 rounds to even");
}

#[test]
fn compare_and_compare_accumulation() {
    // COMP sets AZ/AN and shifts x>y into ASTAT bit 31 (UM §2.5.3.9).
    let s = run(&[ld(R0, 5), ld(R1, 3), c(alu(0x0a, 0, 0, 1)), c(alu(0x0a, 0, 1, 0)), c(alu(0x0a, 0, 0, 0))], 5);
    assert_eq!(s.astat() & (AZ | AN), AZ);
    assert_eq!(s.astat() >> 24, 0b0010_0000, "true, false, false from the oldest");
    assert_eq!(s.astat() & AC, 0);
}

#[test]
fn abs_clip_min_max_logic() {
    let s = run(&[ld(R0, 0x8000_0000), c(alu(0x30, 1, 0, 0))], 2);
    assert_eq!(s.rf(1), 0x8000_0000);
    assert_eq!(s.astat() & (AV | AS | AN), AV | AS | AN);
    let s = run(&[ld(MODE1R, ALUSAT), ld(R0, 0x8000_0000), c(alu(0x30, 1, 0, 0))], 3);
    assert_eq!(s.rf(1), 0x7fff_ffff);
    // CLIP Rx BY Ry: |Ry| with the sign of Rx when |Rx| >= |Ry|.
    let s = run(&[ld(R0, (-9i32) as u32), ld(R1, (-4i32) as u32), c(alu(0x63, 2, 0, 1)), c(alu(0x63, 3, 1, 0))], 4);
    assert_eq!((s.rf(2) as i32, s.rf(3) as i32), (-4, -4));
    let s = run(&[ld(R0, (-9i32) as u32), ld(R1, 4), c(alu(0x61, 2, 0, 1)), c(alu(0x62, 3, 0, 1))], 4);
    assert_eq!((s.rf(2) as i32, s.rf(3)), (-9, 4));
    let s = run(&[ld(R0, 0xf0f0), ld(R1, 0xff00), c(alu(0x42, 2, 0, 1)), c(alu(0x43, 3, 0, 0))], 4);
    assert_eq!((s.rf(2), s.rf(3)), (0x0ff0, !0xf0f0));
    assert_eq!(s.astat() & (AZ | AN | AC | AV), AN);
}

#[test]
fn conditions_follow_table_3_2() {
    // Fixed-point overflow without saturation: LT uses AN xor AV.
    let p = |mode: u32| {
        vec![
            ld(MODE1R, mode),
            ld(R0, 0x8000_0000),
            ld(R1, 1),
            c(alu(SUB, 2, 0, 1)),       // overflows to 0x7FFFFFFF: AN clear, AV set
            t2(1, shc(0xc0, 5, 5, 7)),  // IF LT R5 = BSET R5 BY R7 (shifter: ALU flags kept)
            t2(18, shc(0xc0, 6, 6, 7)), // IF GT R6 = BSET R6 BY R7
        ]
    };
    let s = run(&p(0), 6);
    assert_eq!((s.rf(5), s.rf(6)), (1, 0), "true result is negative");
    let s = run(&p(ALUSAT), 6);
    assert_eq!((s.rf(5), s.rf(6)), (1, 0), "saturated to 0x80000000: negative");
    // Floating point: LT is AN and not AZ; LE includes zero.
    let s = run(&[ld(R0, 0x8000_0000), c(alu(0xa1, 1, 0, 0)), t2(1, shc(0xc0, 5, 5, 7)), t2(2, shc(0xc0, 6, 6, 7))], 4);
    assert_eq!((s.rf(5), s.rf(6)), (0, 1), "-0.0 is not less than zero, but is <= 0");
    // TRUE, NOT LCE outside a loop (CURLCNTR reads 0xFFFFFFFF).
    let s = run(&[t2(31, alu(INC, 5, 5, 5)), t2(15, alu(INC, 6, 6, 6))], 2);
    assert_eq!((s.rf(5), s.rf(6)), (1, 1));
}

// --- floating point (UM B-24 to B-44, §2.2) ------------------------------

#[test]
fn float_add_32_and_40_bit() {
    let p = |mode: u32| {
        vec![ld(MODE1R, mode), ld(R0, 0x3f80_0000), ld(R1, 0x3080_0000), c(alu(FADD, 2, 0, 1)), c(alu(0x82, 3, 2, 0))]
    };
    // 1 + 2^-30 is exact in 40 bits; (1 + 2^-30) - 1 recovers 2^-30.
    let s = run(&p(0), 5);
    assert_eq!(s.r[2], f(1.0) | 2);
    assert_eq!(fv(&s, 3), 2f32.powi(-30));
    let s = run(&p(RND32), 5);
    assert_eq!((s.r[2], s.r[3]), (f(1.0), 0));
    assert_eq!(s.astat() & (AZ | AF), AZ | AF);
    // With RND32 the inputs' 8 LSBs are ignored.
    let s = run(&[ld(MODE1R, RND32), ld(R0, 0x3f80_0000), ld(R1, 0), c(alu(FADD, 2, 0, 1))], 4);
    assert_eq!(s.r[2], f(1.0));
}

#[test]
fn float_specials_and_flags() {
    // Opposite infinities: invalid, all-ones result, sticky AIS.
    let s = run(&[ld(R0, 0x7f80_0000), ld(R1, 0xff80_0000), c(alu(FADD, 2, 0, 1))], 3);
    assert_eq!(s.r[2], float::NAN);
    assert_eq!((s.astat() & AI, s.stky() & AIS), (AI, AIS));
    // Overflow: +Inf to nearest, NORM.MAX when truncating.
    let s = run(&[ld(MODE1R, RND32), ld(R0, 0x7f7f_ffff), c(alu(FADD, 2, 0, 0))], 3);
    assert_eq!((s.r[2], s.astat() & AV, s.stky() & AVS), (f(f32::INFINITY), AV, AVS));
    let s = run(&[ld(MODE1R, RND32 | TRUNC), ld(R0, 0x7f7f_ffff), c(alu(FADD, 2, 0, 0))], 3);
    assert_eq!(s.r[2], f(f32::MAX));
    // Underflow: 1.5*2^-126 - 2^-126 is denormal, so zero with AZ and
    // sticky AUS.
    let s = run(&[ld(R0, 0x00c0_0000), ld(R1, 0x0080_0000), c(alu(0x82, 2, 0, 1))], 3);
    assert_eq!(s.r[2], 0);
    assert_eq!((s.astat() & AZ, s.stky() & AUS), (AZ, AUS));
    // COMP of floats; MIN/MAX of +0 and -0 (UM B-42).
    let s =
        run(&[ld(R0, 0), ld(R1, 0x8000_0000), c(alu(0xe1, 2, 0, 1)), c(alu(0xe2, 3, 0, 1)), c(alu(0x8a, 0, 0, 1))], 5);
    assert_eq!((s.r[2], s.r[3]), (f(-0.0), f(0.0)));
    assert_eq!(s.astat() & (AZ | AN), AZ, "+0 equals -0");
}

#[test]
fn fix_trunc_float_conversions() {
    let p = |mode: u32, op: u64, v: f32| vec![ld(MODE1R, mode), ld(R0, v.to_bits()), c(alu(op, 1, 0, 2))];
    let fix = |mode, op, v| run(&p(mode, op, v), 3).rf(1) as i32;
    assert_eq!(fix(0, 0xc9, 2.5), 2, "FIX: nearest even");
    assert_eq!(fix(0, 0xc9, -2.5), -2);
    assert_eq!(fix(TRUNC, 0xc9, -2.5), -3, "FIX with TRUNC: toward -Infinity");
    assert_eq!(fix(TRUNC, 0xc9, -0.25), -1);
    assert_eq!(fix(0, 0xcd, -2.7), -2, "TRUNC: toward zero whatever MODE1 says");
    // FIX by a scale factor: 1.5 * 2^3.
    let s = run(&[ld(R0, 1.5f32.to_bits()), ld(R2, 3), c(alu(0xd9, 1, 0, 2))], 3);
    assert_eq!(s.rf(1), 12);
    // Overflow: all ones without saturation (AI), clamp with it.
    let s = run(&p(0, 0xc9, 1e10), 3);
    assert_eq!((s.r[1], s.astat() & (AV | AI)), (float::NAN, AV | AI));
    let s = run(&p(ALUSAT, 0xc9, -1e10), 3);
    assert_eq!(s.rf(1), 0x8000_0000);
    // FLOAT Rx BY Ry.
    let s = run(&[ld(R0, (-3i32) as u32), ld(R2, (-1i32) as u32), c(alu(0xda, 1, 0, 2))], 3);
    assert_eq!(fv(&s, 1), -1.5);
    // LOGB, MANT, SCALB, COPYSIGN.
    let s = run(&[ld(R0, 12.0f32.to_bits()), c(alu(0xc1, 1, 0, 0)), c(alu(0xad, 2, 0, 0))], 3);
    assert_eq!((s.rf(1), s.rf(2)), (3, 0xc000_0000));
    let s = run(&[ld(R0, 0), c(alu(0xc1, 1, 0, 0))], 2);
    assert_eq!((s.r[1], s.astat() & AV), (f(f32::NEG_INFINITY), AV), "LOGB 0 is -Infinity unsaturated");
    let s = run(&[ld(MODE1R, ALUSAT), ld(R0, 0), c(alu(0xc1, 1, 0, 0))], 3);
    assert_eq!(s.rf(1), 0x8000_0000);
    let s = run(
        &[
            ld(R0, 3.0f32.to_bits()),
            ld(R2, 2),
            c(alu(0xbd, 1, 0, 2)),
            ld(R3 as u64, 0xbf80_0000),
            c(alu(0xe0, 4, 0, 3)),
        ],
        5,
    );
    assert_eq!((fv(&s, 1), fv(&s, 4)), (12.0, -3.0));
}

const R3: u32 = 3;

// --- multiplier (UM B-45 to B-53, §2.6) --------------------------------------

#[test]
fn fixed_multiply_formats() {
    // 0.5 * 0.5 signed fractional: the product is shifted left one bit,
    // so Rn (bits 63-32) holds 0.25 = 0x20000000.
    let s = run(&[ld(R0, 0x4000_0000), ld(R1, 0x4000_0000), c(mulc(0x78, 2, 0, 1))], 3);
    assert_eq!(s.rf(2), 0x2000_0000);
    // Signed integer: -3 * 7 in bits 31-0.
    let s = run(&[ld(R0, (-3i32) as u32), ld(R1, 7), c(mulc(0x70, 2, 0, 1))], 3);
    assert_eq!(s.rf(2) as i32, -21);
    assert_eq!(s.astat() & (MN | MV), MN);
    // Unsigned integer overflow: MV (upper 48 bits nonzero), sticky MOS.
    let s = run(&[ld(R0, 0x10000), ld(R1, 0x10000), c(mulc(0x40, 2, 0, 1))], 3);
    assert_eq!((s.rf(2), s.astat() & MV, s.stky() & MOS), (0, MV, MOS));
    // Signed fractional -1 * -1 overflows (the one case bit 63 != 62).
    let s = run(&[ld(R0, 0x8000_0000), ld(R1, 0x8000_0000), c(mulc(0x78, 2, 0, 1))], 3);
    assert_eq!(s.astat() & MV, MV);
}

#[test]
fn multiply_accumulate_round_and_saturate() {
    // MRF = Rx*Ry (SSF) twice accumulated, then Rn = RND MRF.
    let p = vec![
        ld(R0, 0x4000_0001),
        ld(R1, 0x4000_0000),
        c(mulc(0x7c, 0, 0, 1)),     // MRF = R0 * R1 (SSF)
        c(mulc(0xbc, 0, 0, 1)),     // MRF = MRF + R0 * R1 (SSF)
        c(mulc(0x18 | 1, 2, 0, 0)), // R2 = RND MRF (SF)
    ];
    let s = run(&p, 5);
    // Each product is 0x20000000_80000000; the sum 0x40000001_00000000.
    assert_eq!(s.mr[s.mrf()], 0x4000_0001_0000_0000);
    assert_eq!(s.rf(2), 0x4000_0001);
    // Rounding at bit 32 is to nearest even.
    let s = run(&[ld(R0, 0x4000_0001), ld(R1, 0x4000_0000), c(mulc(0x79, 2, 0, 1))], 3);
    assert_eq!(s.rf(2), 0x2000_0000, "0x20000000.8 rounds to even");
    // SAT MRF (SI) after MR1F = R0 (sign-extending into MR2).
    let mr1f_r0 = 1u64 << 22 | 1 << 16 | 1 << 12;
    let s = run(&[ld(R0, 0x0000_0005), c(mr1f_r0), c(mulc(0x01, 2, 0, 0)), c(mulc(0x09, 3, 0, 0))], 4);
    assert_eq!(s.rf(2), 0x7fff_ffff, "integer saturation");
    assert_eq!(s.rf(3), 0x0000_0005, "fractional: in range");
    let s = run(&[ld(R0, 0x8000_0000), c(mr1f_r0), c(1 << 22 | 2 << 12 | 4 << 8)], 3);
    assert_eq!(s.rf(4), 0xffff_ffff, "MR2 reads sign-extended");
    // MRB = 0 clears all 80 bits and the flags.
    let s = run(&[ld(R0, 0x8000_0000), c(mr1f_r0), c(mulc(0x16, 0, 0, 0)), c(mulc(0x14, 0, 0, 0))], 4);
    assert_eq!(s.mr, [0, 0]);
}

#[test]
fn srcu_selects_the_foreground_mr() {
    let s = run(&[ld(R0, 7), ld(R1, 1), c(mulc(0x70 | 4, 0, 0, 1)), ld(MODE1R, SRCU), c(mulc(0x70 | 6, 0, 0, 1))], 5);
    // The first MRF and the second MRB are the same physical register.
    assert_eq!(s.mr, [7, 0]);
}

#[test]
fn float_multiply() {
    let s = run(&[ld(MODE1R, RND32), ld(R0, 1.5f32.to_bits()), ld(R1, (-2.5f32).to_bits()), c(mulc(0x30, 2, 0, 1))], 4);
    assert_eq!(fv(&s, 2), -3.75);
    assert_eq!(s.astat() & (MN | MV | MU | MI), MN);
    let s = run(&[ld(R0, 0x7f80_0000), ld(R1, 0), c(mulc(0x30, 2, 0, 1))], 3);
    assert_eq!((s.r[2], s.astat() & MI, s.stky() & MIS), (float::NAN, MI, MIS));
}

// --- shifter (UM B-54 to B-75, §2.7) -------------------------------------------

#[test]
fn shifts_and_rotates() {
    let sh = |op: u64, x: u32, y: u32| {
        let s = run(&[ld(R0, x), ld(R1, y), c(shc(op, 2, 0, 1))], 3);
        (s.rf(2), s.astat() & (SZ | SV | SS))
    };
    assert_eq!(sh(0x00, 0x8000_0001, 4), (0x10, SV), "bits shifted out to the left");
    assert_eq!(sh(0x00, 0x8000_0000, (-31i32) as u32), (1, 0));
    assert_eq!(sh(0x00, 0x1234, 40), (0, SZ | SV), "off-scale left");
    assert_eq!(sh(0x04, 0x8000_0000, (-4i32) as u32), (0xf800_0000, 0));
    assert_eq!(sh(0x04, 0x8000_0000, (-100i32) as u32), (u32::MAX, 0));
    assert_eq!(sh(0x08, 0x8000_0001, 1), (3, 0));
    assert_eq!(sh(0x08, 0x8000_0001, (-1i32) as u32), (0xc000_0000, 0));
    // Immediate data8 is signed for shifts: -8 is a right shift.
    let s = run(&[ld(R0, 0x1200), shimm(T, 0x00, 1, 0, 0xf8)], 2);
    assert_eq!(s.rf(1), 0x12);
    // Rn = Rn OR LSHIFT Rx BY Ry.
    let s = run(&[ld(R0, 0xf), ld(R1, 8), ld(R2, 1), c(shc(0x20, 2, 0, 1))], 4);
    assert_eq!(s.rf(2), 0xf01);
}

#[test]
fn bit_operations() {
    let s = run(&[ld(R0, 0), shimm(T, 0xc0, 1, 0, 5), shimm(T, 0xc8, 2, 1, 5), shimm(T, 0xc4, 3, 1, 40)], 4);
    assert_eq!((s.rf(1), s.rf(2), s.rf(3)), (0x20, 0, 0x20));
    assert_eq!(s.astat() & (SV | SZ), SV, "BCLR out of range: unchanged, SV");
    // BTST sets SZ when the bit is zero, or out of range.
    let s = run(&[ld(R0, 0x20), shimm(T, 0xcc, 0, 0, 5)], 2);
    assert_eq!(s.astat() & SZ, 0);
    let s = run(&[ld(R0, 0x20), shimm(T, 0xcc, 0, 0, 4)], 2);
    assert_eq!(s.astat() & SZ, SZ);
    let s = run(&[ld(R0, 0x20), ld(R1, (-1i32) as u32), c(shc(0xcc, 0, 0, 1))], 3);
    assert_eq!(s.astat() & (SZ | SV), SZ | SV);
}

#[test]
fn field_deposit_and_extract_manual_examples() {
    // Figure 2.7: R0=FDEP R1 BY R2 with R1=0xFF, R2: len6=8, bit6=16.
    let s = run(&[ld(R1, 0xff), ld(R2, 8 << 6 | 16), c(shc(0x44, 0, 1, 2))], 3);
    assert_eq!(s.rf(0), 0x00ff_0000);
    // Figure 2.8: R3=FEXT R4 BY R5 with R4=0x8788000000 (40-bit), bit6=23,
    // len6=8: 0x0F.
    let s = run(&[ld(4, 0x8788_0000), ld(5, 8 << 6 | 23), c(shc(0x40, 3, 4, 5))], 3);
    assert_eq!(s.rf(3), 0x0f);
    // FEXT (SE) of a negative field, and FDEP (SE).
    let s = run(&[ld(4, 0x0000_0f00), shimm(T, 0x48, 3, 4, 8 | 4 << 6)], 2);
    assert_eq!(s.rf(3) as i32, -1);
    let s = run(&[ld(4, 0b1010), shimm(T, 0x4c, 3, 4, 4 | 4 << 6)], 2);
    assert_eq!(s.rf(3), 0xffff_ffa0);
    // A field reaching past bit 31: SV, no sign extension.
    let s = run(&[ld(4, 0xffff), shimm(T, 0x4c, 3, 4, 28 | 8 << 6)], 2);
    assert_eq!((s.rf(3), s.astat() & SV), (0xf000_0000, SV));
    // Rn = Rn OR FDEP.
    let s = run(&[ld(4, 0x3), ld(3, 0x1), shimm(T, 0x64, 3, 4, 4 | 2 << 6)], 3);
    assert_eq!(s.rf(3), 0x31);
}

#[test]
fn exponent_and_leading_bits() {
    let one = |op: u64, x: u32| {
        let s = run(&[ld(R0, x), c(shc(op, 1, 0, 0))], 2);
        (s.rf(1) as i32, s.astat() & (SZ | SV | SS))
    };
    assert_eq!(one(0x80, 0x4000_0000), (0, SZ), "EXP: one leading sign bit");
    assert_eq!(one(0x80, 1), (-30, 0));
    assert_eq!(one(0x80, 0xffff_ff00), (-23, SS));
    assert_eq!(one(0x88, 0x0000_ffff), (16, 0));
    assert_eq!(one(0x88, 0), (32, SV));
    assert_eq!(one(0x8c, 0xf000_0000), (4, 0));
    assert_eq!(one(0x88, 0x8000_0000), (0, SZ));
    // EXP (EX) after an ALU overflow reports +1.
    let s = run(&[ld(R0, 0x7fff_ffff), c(alu(INC, 1, 0, 0)), c(shc(0x84, 2, 1, 1))], 3);
    assert_eq!((s.rf(2), s.astat() & SS), (1, 0), "AV xor sign(0x80000000) = 0");
}

#[test]
fn short_float_pack_unpack() {
    let s = run(&[ld(R0, 1.5f32.to_bits()), c(shc(0x90, 1, 0, 0)), c(shc(0x94, 2, 1, 1))], 3);
    assert_eq!(s.rf(1), 0x3c00, "e4=7, fraction 10000000000");
    assert_eq!(fv(&s, 2), 1.5);
}

// --- multifunction (UM §2.8, B.3) ---------------------------------------------

#[test]
fn dual_add_subtract_and_parallel_multiply() {
    // R4 = R2 + R3, R5 = R2 - R3 (fixed dual add/subtract).
    let s = run(&[ld(R2, 10), ld(3, 3), c(alu(0x75, 4, 2, 3))], 3);
    assert_eq!((s.rf(4), s.rf(5)), (13, 7));
    // F8 = F0 * F4, F12 = F8 + F12 in one instruction: both read the old
    // F8 and F12.
    let cf = 1 << 22 | 0b011000 << 16 | 8 << 12 | 12 << 8;
    let s = run(
        &[ld(0, 2.0f32.to_bits()), ld(4, 3.0f32.to_bits()), ld(8, 1.0f32.to_bits()), ld(12, 10.0f32.to_bits()), c(cf)],
        5,
    );
    assert_eq!((fv(&s, 8), fv(&s, 12)), (6.0, 11.0));
    // The multiplier and dual add/subtract: F1 = F0*F4, F2 = F8+F12, F5 = F8-F12.
    let cf = 1 << 22 | 0b11 << 20 | 5 << 16 | 1 << 12 | 2 << 8;
    let s = run(
        &[ld(0, 2.0f32.to_bits()), ld(4, 3.0f32.to_bits()), ld(8, 1.0f32.to_bits()), ld(12, 10.0f32.to_bits()), c(cf)],
        5,
    );
    assert_eq!((fv(&s, 1), fv(&s, 2), fv(&s, 5)), (6.0, 11.0, -9.0));
}

// --- data moves and DAGs (UM Chapter 4) -------------------------------------------

#[test]
fn dual_move_reads_registers_before_writes() {
    // DM(I0,M0) = R1, PM(I8,M8) = R2, R1 = R1 + R2: memory gets the old R1.
    let p = vec![
        ld(I0, 0x2_8000),
        ld(M0, 1),
        ld(0x18, 0x2_9000),
        ld(0x28, 2),
        ld(R1, 5),
        ld(R2, 7),
        t1(true, 0, 0, 1, true, 8, 8, 2, alu(ADD, 1, 1, 2)),
    ];
    let s = run(&p, 7);
    assert_eq!(s.mem.read32(0x2_8000), 5);
    assert_eq!(s.mem.read32(0x2_9000), 7);
    assert_eq!(s.rf(1), 12);
    assert_eq!((s.i[0], s.i[8]), (0x2_8001, 0x2_9002), "post-modify updates");
}

#[test]
fn load_beats_compute_to_the_same_register() {
    // R1 = R1 + R1, R1 = DM(I0,M0): the DM transfer has precedence.
    let p = vec![ld(I0, 0x2_8000), ld(R1, 2), t3(true, 0, 0, false, false, R1, alu(ADD, 1, 1, 1))];
    let (mut s, mut e) = boot(&p);
    s.mem.write32(0x2_8000, 99);
    trace(&mut s, &mut e, 3);
    assert_eq!(s.rf(1), 99);
}

#[test]
fn pre_modify_and_immediate_modifiers() {
    let p = vec![
        ld(I0, 0x2_8000),
        ld(M0, 4),
        ld(R1, 11),
        t3(false, 0, 0, false, true, R1, 0), // DM(M0,I0) = R1: no update
        t4(0, false, true, true, 3, 1),      // DM(I0,3) = R1: post-modify by 3
        t4(0, false, false, false, -3, 2),   // R2 = DM(-3,I0)
    ];
    let s = run(&p, 6);
    assert_eq!(s.mem.read32(0x2_8004), 11);
    assert_eq!(s.mem.read32(0x2_8000), 11);
    assert_eq!(s.i[0], 0x2_8003);
    assert_eq!(s.rf(2), 11);
}

#[test]
fn circular_buffer_figure_4_4() {
    // Length 11, base 0, modifier 4: 0 4 8 1 5 9 2 6 10 3 7 (UM Figure 4.4).
    let mut p = vec![ld(B0 + 1, 0x2_8000), ld(L0 + 1, 11), ld(M0 + 1, 4)];
    for _ in 0..11 {
        p.push(t3(true, 1, 1, false, true, R1, alu(INC, 1, 1, 1)));
    }
    let s = run(&p, p.len());
    let order: Vec<u32> = (0..11).map(|k| s.mem.read32(0x2_8000 + k)).collect();
    assert_eq!(order, [0, 3, 6, 9, 1, 4, 7, 10, 2, 5, 8], "the n-th access stored n");
    assert_eq!(s.i[1], 0x2_8000, "back at the start");
}

#[test]
fn circular_wrap_on_i7_sets_sticky_and_latch() {
    let p = vec![
        ld(B0 + 7, 0x2_8000),
        ld(L0 + 7, 2),
        ld(M0 + 7, 1),
        t3(true, 7, 7, false, true, R1, 0),
        t3(true, 7, 7, false, true, R1, 0),
    ];
    let s = run(&p, 5);
    assert_eq!(s.i[7], 0x2_8000);
    assert_eq!(s.stky() & CB7S, CB7S);
    assert_ne!(s.irptl & (1 << 21), 0);
}

#[test]
fn bit_reverse_mode_and_instruction() {
    // I0=0x80400000; R1=DM(I0,3): address 0x201, I0=0x80400003 (UM §4.3.3.1).
    let mut s = Sharc::new();
    s.set_mode1(BR0);
    s.i[0] = 0x8040_0000;
    let a = s.dag_output(&Addr { i: 0, modifier: Modifier::Imm(3), post: true });
    assert_eq!((a, s.i[0]), (0x201, 0x8040_0003));
    // Only I0 is reversed.
    s.i[1] = 0x8040_0000;
    assert_eq!(s.dag_output(&Addr { i: 1, modifier: Modifier::Imm(0), post: true }), 0x8040_0000);
    // BITREV (I1,4): I1 = bit-reverse of (I1 + 4).
    let s = run(&[ld(I0 + 1, 0), modimm(1, 4, true)], 2);
    assert_eq!(s.i[1], 0x2000_0000);
}

#[test]
fn dag_registers_and_b_loads_i() {
    let s = run(&[ld(B0 + 9, 0x12_3456), ld(M0 + 9, 0xff_fffe), mv(M0 + 9, R1, 0), mv(B0 + 9, R2, 0)], 4);
    assert_eq!(s.i[9], 0x12_3456);
    assert_eq!(s.rf(1) as i32, -2, "DAG2 M registers read sign-extended");
    assert_eq!(s.rf(2), 0x12_3456);
}

#[test]
fn px_transfers() {
    // PX via the PM bus is 48 bits; via the DM bus, 40 (UM §5.1.4).
    let p = vec![
        ld(0x18, 0x2_0400),
        ld(I0, 0x2_8000),
        t3(true, 0, 0, true, false, PXR, 0), // PX = PM(I8,M8)
        t3(true, 0, 0, false, true, PXR, 0), // DM(I0,M0) = PX
        mv(0xdc, R1, 0),
        mv(0xdd, R2, 0),
    ];
    let (mut s, mut e) = boot(&p);
    s.mem.write48(0x2_0400, 0x1234_5678_9abc);
    trace(&mut s, &mut e, 6);
    assert_eq!(s.px, 0x1234_5678_9abc);
    assert_eq!((s.rf(1), s.rf(2)), (0x9abc, 0x1234_5678));
    // The DM write: 40 bits, low 8 zero, as a 48-bit word.
    assert_eq!(s.mem.read48(0x2_8000), 0x1234_5678_9a00);
}

#[test]
fn short_words_and_sign_extension() {
    let p = |mode: u32| vec![ld(MODE1R, mode), direct(false, false, R1, 0x5_0001)];
    let (mut s, mut e) = boot(&p(0));
    s.mem.write32(0x2_8000, 0x8001_7fff);
    trace(&mut s, &mut e, 2);
    assert_eq!(s.rf(1), 0x8001, "MSW, zero-filled");
    let (mut s, mut e) = boot(&p(SSE));
    s.mem.write32(0x2_8000, 0x8001_7fff);
    trace(&mut s, &mut e, 2);
    assert_eq!(s.rf(1), 0xffff_8001, "sign-extended with SSE");
}

#[test]
fn data_width_follows_imdw() {
    // SYSCON IMDW1: block 1 holds 40-bit data in 48-bit words.
    let p = vec![
        ld(I0, 0),
        ld(R0, 0x10 | 1 << 9),
        t3(true, 0, 0, false, true, R0, 0), // SYSCON = 0x210
        ld(R1, 0x1234_5678),
        direct(false, true, R1, 0x2_8010),
    ];
    let s = run(&p, 5);
    assert_eq!(s.mem.read48(0x2_8010), 0x1234_5678_0000);
}

#[test]
fn immediate_stores_and_lcntr() {
    // DM(I0,M0) = data and PM(I8,M8) = data (type 16); LCNTR as a register.
    let p = vec![
        ld(I0, 0x2_8000),
        ld(M0, 1),
        ld(0x18, 0x2_9000),
        st_imm(0, 0, false, 0x1234),
        st_imm(0, 0, true, 0x5678),
        st_imm(0, 0, false, 0x9abc),
        ld(LCNTR, 7),
        mv(LCNTR, R1, 0),
    ];
    let s = run(&p, p.len());
    assert_eq!((s.mem.read32(0x2_8000), s.mem.read32(0x2_8001)), (0x1234, 0x9abc));
    assert_eq!(s.mem.read32(0x2_9000), 0x5678);
    assert_eq!((s.lcntr, s.rf(1)), (7, 7));
}

#[test]
fn external_accesses_use_the_bus() {
    let p = vec![
        ld(R1, 0xabcd),
        direct(false, true, R1, 0x40_0010),
        direct(true, false, R2, 0x40_0020),
        direct(false, false, 3, 0x40_0030),
    ];
    let (mut s, mut e) = boot(&p);
    e.mem.insert(0x40_0020, 0x1122_3344_5566);
    e.mem.insert(0x40_0030, 0x7777);
    trace(&mut s, &mut e, 4);
    assert_eq!(e.log[0], (0x40_0010, Width::Data32, Some(0xabcd)));
    assert_eq!(e.log[1], (0x40_0020, Width::Word48, None));
    assert_eq!(s.r[2], 0x0011_2233_4455, "PM bus: the upper 40 bits");
    assert_eq!(s.rf(3), 0x7777);
}

// --- sequencer (UM Chapter 3) ------------------------------------------------------

#[test]
fn delayed_and_non_delayed_jumps() {
    let inc = |r: u64| c(alu(INC, r, r, r));
    let p = vec![jump(0x2_0005, T, false, true, false), inc(1), inc(2), inc(3), inc(4), NOP];
    let (mut s, mut e) = boot(&p);
    assert_eq!(trace(&mut s, &mut e, 4), [0, 1, 2, 5]);
    assert_eq!((s.rf(1), s.rf(2), s.rf(3)), (1, 1, 0));
    let p = vec![jump(0x2_0005, T, false, false, false), inc(1), inc(2), inc(3), inc(4), NOP];
    let (mut s, mut e) = boot(&p);
    assert_eq!(trace(&mut s, &mut e, 2), [0, 5]);
    assert_eq!(s.cycles, 6, "two fill cycles at start, two NOPs after the jump");
    assert_eq!(s.rf(1), 0);
}

#[test]
fn calls_and_returns() {
    // Non-delayed CALL returns to the next word; CALL (DB) to the third.
    let p = vec![
        jump(0x2_0008, T, true, false, false),
        jump(0x2_0008, T, true, true, false),
        c(alu(INC, 1, 1, 1)),
        c(alu(INC, 2, 2, 2)),
        c(alu(INC, 3, 3, 3)),
        IDLE,
        NOP,
        NOP,
        rts(T, true, false),
        c(alu(INC, 5, 5, 5)),
        c(alu(INC, 6, 6, 6)),
    ];
    let (mut s, mut e) = boot(&p);
    let t = trace(&mut s, &mut e, 12);
    assert_eq!(t, [0, 8, 9, 10, 1, 2, 3, 8, 9, 10, 4, 5]);
    assert!(s.pc_stack().is_empty());
}

#[test]
fn conditional_branch_with_else_compute() {
    // IF EQ JUMP (PC,3), ELSE R1 = R1 + 1: AZ clear, so the compute runs.
    let p = vec![ld(R0, 1), c(alu(0x21, 0, 0, 0)), jrel(0, 3, false, false, true, alu(INC, 1, 1, 1)), NOP, NOP, NOP];
    let (mut s, mut e) = boot(&p);
    assert_eq!(trace(&mut s, &mut e, 4), [0, 1, 2, 3]);
    assert_eq!(s.rf(1), 1);
    // Without ELSE, a false condition skips the compute too.
    let p = vec![ld(R0, 1), c(alu(0x21, 0, 0, 0)), jrel(0, 3, false, false, false, alu(INC, 1, 1, 1)), NOP];
    let s = run(&p, 4);
    assert_eq!(s.rf(1), 0);
    // True: jumps, and the compute (no ELSE) runs with it.
    let p = vec![ld(R0, 0), c(alu(0x21, 0, 0, 0)), jrel(0, 3, false, false, false, alu(INC, 1, 1, 1)), NOP, NOP, NOP];
    let (mut s, mut e) = boot(&p);
    assert_eq!(trace(&mut s, &mut e, 4), [0, 1, 2, 5]);
    assert_eq!(s.rf(1), 1);
}

#[test]
fn counter_loops_of_every_length() {
    let inc = |r: u64| c(alu(INC, r, r, r));
    // One-instruction loops of 3, 2 and 1 iterations (UM Figure 3.7).
    for n in [3u16, 2, 1] {
        let (mut s, mut e) = boot(&[do_lce(n, 1), inc(1), NOP]);
        let t = trace(&mut s, &mut e, n as usize + 2);
        assert_eq!(t.iter().filter(|&&a| a == 1).count(), n as usize, "{t:?}");
        assert_eq!(*t.last().unwrap(), 2);
        assert_eq!((s.rf(1), s.loop_depth(), s.pc_stack().len()), (n as u32, 0, 0));
    }
    // Two-instruction loops (Figure 3.8).
    for n in [2u16, 1, 5] {
        let (mut s, mut e) = boot(&[do_lce(n, 2), inc(1), inc(2), NOP]);
        let t = trace(&mut s, &mut e, 2 * n as usize + 2);
        assert_eq!(*t.last().unwrap(), 3, "{t:?}");
        assert_eq!((s.rf(1), s.rf(2)), (n as u32, n as u32));
    }
    // A longer loop, and CURLCNTR during it.
    let (mut s, mut e) = boot(&[do_lce(4, 3), inc(1), mv(CURLCNTR, R2, 0), inc(3), NOP]);
    trace(&mut s, &mut e, 1 + 4 * 3 + 1);
    assert_eq!((s.rf(1), s.rf(3)), (4, 4));
    // On the last iteration the stacks pop two instructions before the
    // end, so CURLCNTR then reads 0xFFFFFFFF (UM §3.5.3.1).
    assert_eq!(s.rf(2), u32::MAX);
    assert_eq!(s.loop_depth(), 0);
}

#[test]
fn loop_counter_from_a_register_and_nesting() {
    let p = vec![
        ld(R0, 3),
        put(&[(47, 40, 0x0d), (39, 32, R0), (23, 0, 4)]), // LCNTR = R0, DO (PC,4)
        NOP,
        do_lce(2, 1), // LCNTR = 2, DO (PC,1)
        c(alu(INC, 1, 1, 1)),
        c(alu(INC, 2, 2, 2)),
        NOP,
    ];
    let (mut s, mut e) = boot(&p);
    for _ in 0..200 {
        if s.pc == 0x2_0006 {
            break;
        }
        assert_eq!(s.step(&mut e), Step::Ran);
    }
    assert_eq!(s.pc, 0x2_0006);
    assert_eq!((s.rf(1), s.rf(2)), (6, 3));
    assert_eq!((s.loop_depth(), s.pc_stack().len()), (0, 0));
}

#[test]
fn condition_based_loops_and_the_pipeline() {
    let dec0 = c(alu(DEC, 0, 0, 0));
    // One-instruction loop: after the condition becomes true it runs three
    // more times (UM §3.5.1.3).
    let (mut s, mut e) = boot(&[ld(R0, 5), do_until(0, 1), dec0, NOP]);
    for _ in 0..30 {
        if s.pc == 0x2_0003 {
            break;
        }
        s.step(&mut e);
    }
    assert_eq!(s.rf(0) as i32, -3);
    // Two-instruction loop, condition set by the first: one more pass.
    let (mut s, mut e) = boot(&[ld(R0, 2), do_until(0, 2), dec0, NOP, NOP]);
    for _ in 0..30 {
        if s.pc == 0x2_0004 {
            break;
        }
        s.step(&mut e);
    }
    assert_eq!(s.rf(0) as i32, -1);
    // Three-instruction loop: tested at the top; one full pass more.
    let (mut s, mut e) = boot(&[ld(R0, 3), do_until(0, 3), dec0, NOP, NOP, NOP]);
    for _ in 0..40 {
        if s.pc == 0x2_0005 {
            break;
        }
        s.step(&mut e);
    }
    assert_eq!(s.rf(0) as i32, -1);
    assert_eq!(s.loop_depth(), 0);
}

#[test]
fn jump_la_leaves_a_loop() {
    // A FOREVER loop left with IF EQ JUMP (LA).
    // (Not in the last three instructions of the loop, UM §3.4.)
    let p = vec![
        ld(R0, 3),
        do_until(31, 5),
        c(alu(DEC, 0, 0, 0)),
        jump(0x2_0008, 0, false, false, true),
        NOP,
        NOP,
        NOP,
        NOP,
        NOP,
    ];
    let (mut s, mut e) = boot(&p);
    for _ in 0..60 {
        if s.pc == 0x2_0008 {
            break;
        }
        s.step(&mut e);
    }
    assert_eq!(s.pc, 0x2_0008);
    assert_eq!(s.rf(0), 0);
    assert_eq!((s.loop_depth(), s.pc_stack().len()), (0, 0));
}

#[test]
fn rts_lr_reenters_a_loop_without_counting_twice() {
    // A non-delayed CALL as the last instruction but one of a loop,
    // returning with RTS (LR) (UM §3.5.1.1).
    let p = vec![
        do_lce(3, 3),
        c(alu(INC, 1, 1, 1)),
        jump(0x2_0006, T, true, false, false),
        c(alu(INC, 2, 2, 2)),
        jump(0x2_0008, T, false, false, false),
        NOP,
        c(alu(INC, 3, 3, 3)),
        rts(T, false, true),
        NOP,
    ];
    let (mut s, mut e) = boot(&p);
    for _ in 0..60 {
        if s.pc == 0x2_0008 {
            break;
        }
        s.step(&mut e);
    }
    assert_eq!(s.pc, 0x2_0008);
    assert_eq!((s.rf(1), s.rf(2), s.rf(3)), (3, 3, 3));
}

#[test]
fn system_register_bit_operations() {
    let p = vec![
        ld(0x70, 0xf0),
        bit(SET, 0, 0x01),
        bit(CLR, 0, 0x10),
        bit(TGL, 0, 0x03),
        bit(TST, 0, 0xe2),
        mv(ASTATR, R1, 0),
        bit(XOR, 0, 0xe3),
        mv(ASTATR, R2, 0),
    ];
    let s = run(&p, 8);
    assert_eq!(s.ustat1, 0xe2);
    assert_eq!(s.rf(1) & BTF, BTF);
    assert_eq!(s.rf(2) & BTF, 0);
    // BIT TST in a condition: IF TF.
    let s = run(&[ld(0x71, 4), bit(TST, 1, 4), t2(13, alu(INC, 3, 3, 3))], 3);
    assert_eq!(s.rf(3), 1);
}

#[test]
fn alternate_registers() {
    let p = vec![ld(R0, 1), ld(R2 + 8, 5), bit(SET, 0xb, SRRFL), ld(R0, 2), bit(CLR, 0xb, SRRFL), mv(0x0a, 3, 0)];
    let s = run(&p, 6);
    assert_eq!((s.rf(0), s.r_alt[0] >> 8), (1, 2));
    assert_eq!(s.rf(3), 5, "the upper half did not switch");
    let s = run(&[ld(I0 + 5, 9), bit(SET, 0xb, SRD1H), ld(I0 + 5, 3), mv(I0 + 5, R1, 0)], 4);
    assert_eq!((s.rf(1), s.dag_alt[0][5]), (3, 9));
}

#[test]
fn sticky_and_stack_status() {
    // STKY bits 21-26 are read-only; others stay set until cleared.
    let s = run(&[ld(0x7e, 0)], 1);
    assert_eq!(s.stky() & (PCEM | SSEM | LSEM), PCEM | SSEM | LSEM);
    // PUSH STS, PUSH PCSTK.
    let s = run(&[stacks(0b0010100), mv(0x7e, R1, 0)], 2);
    assert_eq!(s.rf(1) & (PCEM | SSEM), 0, "a push clears the empty flags");
    assert_eq!(s.pc_stack(), &[0]);
}

#[test]
fn push_and_pop_status() {
    let p = vec![
        ld(MODE1R, ALUSAT),
        ld(ASTATR, AV | AN),
        stacks(0b0010000),
        ld(MODE1R, 0),
        ld(ASTATR, 0),
        stacks(0b0001000),
    ];
    let s = run(&p, 6);
    assert_eq!((s.mode1, s.astat() & (AV | AN)), (ALUSAT, AV | AN));
}

#[test]
fn undefined_instruction_faults() {
    let (mut s, mut e) = boot(&[NOP, 0x0500_0000_0000]);
    assert_eq!(s.step(&mut e), Step::Ran);
    assert_eq!(s.step(&mut e), Step::Fault(Fault::Undefined { pc: 0x2_0001, word: 0x0500_0000_0000 }));
    let (mut s, mut e) = boot(&[c(3 << 20 | 1)]);
    assert!(matches!(s.step(&mut e), Step::Fault(Fault::Undefined { .. })));
    // An unimplemented IOP register (SPORT0 transmit control).
    let (mut s, mut e) = boot(&[ld(R0, 1), direct(false, true, R0, 0xe0)]);
    s.step(&mut e);
    assert_eq!(s.step(&mut e), Step::Fault(Fault::Iop { pc: 0x2_0001, addr: 0xe0, write: true }));
}

// --- interrupts (UM §3.6) ---------------------------------------------------------

#[test]
fn software_interrupt_vectors_and_returns() {
    let mut p = vec![
        ld(IMASKR, 1 << 28),
        ld(MODE1R, IRPTEN),
        ld(IRPTL, 1 << 28),
        c(alu(INC, 1, 1, 1)),
        c(alu(INC, 2, 2, 2)),
        IDLE,
    ];
    p.resize(0x70, NOP);
    p.extend([ld(5, 7), RTI]);
    let (mut s, mut e) = boot(&p);
    let t = trace(&mut s, &mut e, 6);
    assert_eq!(t, [0, 1, 2, 0x70, 0x71, 3]);
    assert_eq!((s.rf(5), s.rf(1), s.imaskp, s.irptl), (7, 1, 0, 0));
}

#[test]
fn timer_interrupt_pushes_and_pops_status() {
    let mut p = vec![
        ld(TPERIOD, 3),
        ld(TCOUNT, 3),
        ld(IMASKR, 1 << IRQ_TMZH),
        ld(MODE1R, IRPTEN | ALUSAT),
        ld(MODE2R, 1 << 5),
        NOP,
        NOP,
        NOP,
        IDLE,
    ];
    p.resize(0x10, NOP);
    // The handler changes MODE1 and ASTAT; RTI restores them.
    p.extend([ld(MODE1R, 0), ld(ASTATR, AZ), c(alu(INC, 6, 6, 6)), ld(MODE2R, 0), RTI]);
    let (mut s, mut e) = boot(&p);
    for _ in 0..30 {
        if s.step(&mut e) == Step::Idle {
            break;
        }
    }
    assert_eq!(s.rf(6), 1);
    assert_eq!(s.mode1, IRPTEN | ALUSAT);
    assert_eq!(s.astat() & AZ, 0);
}

#[test]
fn delayed_branch_holds_off_interrupts() {
    let mut p = vec![
        ld(IMASKR, 1 << 28),
        ld(MODE1R, IRPTEN),
        jump(0x2_0006, T, false, true, false),
        ld(IRPTL, 1 << 28),
        NOP,
        NOP,
        NOP,
    ];
    p.resize(0x70, NOP);
    p.extend([RTI]);
    let (mut s, mut e) = boot(&p);
    // The latch is set in the first delay slot; the interrupt is taken
    // only after the jump lands, and returns to the jump's target.
    assert_eq!(trace(&mut s, &mut e, 7), [0, 1, 2, 3, 4, 0x70, 6]);
}

// --- host interface and boot (UM §8, §11.6.3) ------------------------------------------

/// Write a 32-bit IOP register as two 16-bit host transfers, LSW first.
fn host32(s: &mut Sharc, reg: u32, v: u32) {
    s.host_write(reg, v & 0xffff).unwrap();
    s.host_write(reg, v >> 16).unwrap();
}

#[test]
fn host_boot_through_dma_channel_6() {
    let mut prog = vec![0u64; 0x100];
    prog[5] = 0x063e_0402_0080; // JUMP 0x20080 (DB)
    prog[0x40] = RTI;
    prog[0x80] = ld(R0, 0x1234);
    prog[0x81] = IDLE;
    let mut s = Sharc::new();
    let mut e = Ext::default();
    // Reset state: DMAC6 = 0xA1, II6 = 0x20000, C6 = 256; nothing runs.
    assert_eq!(s.step(&mut e), Step::Idle);
    for (k, w) in prog.iter().enumerate() {
        for h in 0..3 {
            s.host_write(EPB0, (w >> (16 * h)) as u32 & 0xffff).unwrap();
        }
        assert_eq!(s.irptl & (1 << IRQ_EP0) != 0, k == 0xff, "EP0I when the count runs out");
    }
    assert_eq!(s.mem.read48(0x2_0005), 0x063e_0402_0080);
    let t: Vec<u32> = (0..5)
        .map(|_| {
            assert_eq!(s.step(&mut e), Step::Ran);
            s.pc
        })
        .collect();
    assert_eq!(t, [0x2_0040, 0x2_0005, 0x2_0006, 0x2_0007, 0x2_0080]);
    s.step(&mut e);
    assert_eq!(s.rf(0), 0x1234);
    assert_eq!(s.step(&mut e), Step::Idle);
}

#[test]
fn host_sets_up_a_longer_boot_like_model_2b() {
    // The 2B host: DMAC6 = 0, II6, IM6, C6, DMAC6 = 0xA1, then the data.
    let mut s = Sharc::new();
    host32(&mut s, DMAC6, 0);
    host32(&mut s, II6, 0x2_0000);
    host32(&mut s, IM6, 1);
    host32(&mut s, C6, 0x1000);
    host32(&mut s, DMAC6, 0xa1);
    for k in 0..0x1000u64 {
        let w = if k == 0x40 { RTI } else { k << 8 };
        for h in 0..3 {
            s.host_write(EPB0, (w >> (16 * h)) as u32 & 0xffff).unwrap();
        }
    }
    assert_eq!(s.mem.read48(0x2_0fff), 0xfff << 8);
    assert_ne!(s.irptl & (1 << IRQ_EP0), 0);
}

#[test]
fn message_registers_both_ways() {
    let mut s = Sharc::new();
    host32(&mut s, MSGR0, 0x1234_5678);
    let p = vec![direct(false, false, R1, MSGR0), ld(R2, 0xcafe_f00d), direct(false, true, R2, MSGR0 + 1)];
    s.load48(0x2_0000, &p);
    s.start_at(0x2_0000);
    let mut e = Ext::default();
    trace(&mut s, &mut e, 3);
    assert_eq!(s.rf(1), 0x1234_5678);
    assert_eq!(s.host_read(MSGR0 + 1), Ok(0xf00d));
    assert_eq!(s.host_read(MSGR0 + 1), Ok(0xcafe));
    assert!(matches!(s.host_write(0xe0, 0), Ok(())), "the first half only latches");
    assert_eq!(s.host_write(0xe0, 0), Err(HostError::Unimplemented { addr: 0xe0 }));
}

#[test]
fn flags_as_outputs() {
    // MODE2 FLG2O/FLG3O (bits 17-18) as the 2B programs set them; FLAG0
    // is an input.
    let p = vec![ld(MODE2R, 0x6_0000), bit(SET, 0xc, 1 << 21), t2(9, alu(INC, 1, 1, 1))];
    let (mut s, mut e) = boot(&p);
    s.flag_in = 1;
    trace(&mut s, &mut e, 3);
    assert_eq!(s.flag_outputs(), 0b0100);
    assert_eq!(s.rf(1), 1, "IF FLAG0_IN sees the pin");
}

#[test]
fn spinning_on_flag_inputs() {
    // The two waits of the 2B programs: `IF FLAG0_IN JUMP (PC,0)`, and a
    // one-NOP `DO ... UNTIL NOT FLAG0_IN`.
    let p = vec![
        jrel(9, 0, false, false, false, 0),
        do_until(25, 1),
        NOP,
        c(alu(INC, 1, 1, 1)),
        jump(0x2_0003, T, false, false, false),
    ];
    let (mut s, mut e) = boot(&p);
    s.flag_in = 1;
    trace(&mut s, &mut e, 1);
    assert!(s.spinning(), "taken jump to itself on FLAG0");
    s.flag_in = 0;
    trace(&mut s, &mut e, 1);
    assert!(!s.spinning(), "fell through");
    s.flag_in = 1;
    trace(&mut s, &mut e, 2);
    assert_eq!(s.pc, 0x2_0002);
    assert!(s.spinning(), "inside the NOP loop");
    s.flag_in = 0;
    let t = trace(&mut s, &mut e, 4);
    assert_eq!(t[t.len() - 1], 3, "left the loop: {t:?}");
    assert!(!s.spinning());
    // An unmasked, enabled interrupt that is latched ends a spin.
    let (mut s, mut e) = boot(&[jrel(9, 0, false, false, false, 0)]);
    s.flag_in = 1;
    trace(&mut s, &mut e, 1);
    s.set_mode1(IRPTEN);
    s.imask |= 1 << IRQ_VIRPT;
    s.latch(IRQ_VIRPT);
    assert!(!s.spinning());
}

#[test]
fn save_state_round_trip() {
    let p = vec![
        ld(R0, 3),
        do_lce(5, 2),
        c(alu(INC, 1, 1, 1)),
        c(mulc(0xbc, 0, 1, 0)),
        jump(0x2_0001, T, false, true, false),
        NOP,
        NOP,
    ];
    let (mut a, mut e) = boot(&p);
    trace(&mut a, &mut e, 7);
    let bytes = state::to_bytes(&a);
    let mut b = Sharc::new();
    State::load(&mut b, &mut state::Reader::new(&bytes)).unwrap();
    let ta = trace(&mut a, &mut e, 40);
    let tb = trace(&mut b, &mut e, 40);
    assert_eq!(ta, tb);
    assert_eq!((a.r, a.mr, a.stky()), (b.r, b.mr, b.stky()));
}
