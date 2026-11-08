use super::*;
use std::collections::HashMap;

#[derive(Default)]
struct Ext(HashMap<u32, u32>);

impl External for Ext {
    fn read(&mut self, a: u32) -> u32 {
        self.0.get(&a).copied().unwrap_or(0)
    }
    fn write(&mut self, a: u32, v: u32) {
        self.0.insert(a, v);
    }
}

fn put(fields: &[(u32, u32, u64)]) -> u64 {
    fields.iter().fold(0, |w, &(hi, lo, v)| w | ((v << lo) & (((1u64 << (hi - lo + 1)) - 1) << lo)))
}

// Register codes and arithmetic output codes used below.
const AA0: u64 = 0x08;
const AA1: u64 = 0x09;
const AA2: u64 = 0x0a;
const OUT_AA0: u64 = 0x10;
const OUT_AA1: u64 = 0x11;

fn mov3(imm: u32, dst: u64) -> u64 {
    put(&[(63, 61, 7), (58, 27, imm as u64), (25, 19, dst)])
}
/// F6 with a single ALU op and a control op.
fn alu_ctrl(op: u64, in1: u64, in2: u64, out: u64, cop: u64, ef1: u64, ef2: u64) -> u64 {
    put(&[
        (63, 61, 6),
        (60, 56, op),
        (55, 52, in1),
        (51, 47, in2),
        (46, 42, out),
        (41, 41, 1),
        (26, 22, cop),
        (21, 16, ef1),
        (15, 0, ef2),
    ])
}
fn ctrl(cop: u64, ef1: u64, ef2: u64) -> u64 {
    alu_ctrl(0x07, 0, 0, 0, cop, ef1, ef2)
}
/// F5: ALU NOP plus a register-to-register MOV2.
fn mov2(src: u64, dst: u64) -> u64 {
    put(&[(63, 61, 5), (60, 56, 7), (41, 41, 1), (37, 31, src), (30, 24, dst)])
}
const NOP: u64 = 0xc700_0200_0000_0000;

fn boot(prog: &[u64]) -> (Tgp, Ext) {
    let mut t = Tgp::new();
    t.boot(prog);
    (t, Ext::default())
}

fn steps(t: &mut Tgp, e: &mut Ext, n: usize) {
    for _ in 0..n {
        assert_eq!(t.step(e), Step::Ran, "at pc {:#x}", t.pc);
    }
}

#[test]
fn float_add_with_constant_to_output_fifo() {
    let prog = [
        mov3(1.0f32.to_bits(), AA0),
        alu_ctrl(0x00, 0, 0x1b, OUT_AA1, 0, 0, 0), // FADD AA0, 1.0, AA1
        mov2(AA1, 0x32),                           // MOV AA1, FO0
    ];
    let (mut t, mut e) = boot(&prog);
    steps(&mut t, &mut e, 3);
    assert_eq!(t.fifo_out.pop_front().map(f32::from_bits), Some(2.0));
}

#[test]
fn input_fifo_read_stalls_without_side_effects() {
    let (mut t, mut e) = boot(&[mov2(0x31, AA0)]);
    assert_eq!(t.step(&mut e), Step::Stalled);
    assert_eq!(t.pc, 0);
    t.fifo_in.push_back(5);
    assert_eq!(t.step(&mut e), Step::Ran);
    assert_eq!(t.aa[0], 5);
}

#[test]
fn call_and_return_run_their_delay_slots() {
    let prog = [
        ctrl(0x1a, 0, 4), // 0: DCALL 4
        mov3(1, AA0),     // 1: delay slot
        mov3(2, AA1),     // 2: return lands here
        NOP,              // 3
        ctrl(0x1b, 0, 0), // 4: DRET
        mov3(3, AA2),     // 5: delay slot
    ];
    let (mut t, mut e) = boot(&prog);
    let mut trace = vec![];
    for _ in 0..5 {
        trace.push(t.pc);
        t.step(&mut e);
    }
    assert_eq!(trace, vec![0, 1, 4, 5, 2]);
    assert_eq!((t.aa[0], t.aa[1], t.aa[2]), (1, 2, 3));
}

#[test]
fn loop_runs_body_lpc_times() {
    let prog = [
        ctrl(0x02, 0, 3),                          // 0: SETL 3
        alu_ctrl(0x10, 0, 0x19, OUT_AA0, 0, 0, 0), // 1: ADD AA0, #1, AA0
        ctrl(0x13, 0, 0xffe),                      // 2: DBLP 1 (2+1-2)
        NOP,                                       // 3: delay slot
        NOP,                                       // 4: after the loop
    ];
    let (mut t, mut e) = boot(&prog);
    while t.pc != 4 {
        assert_eq!(t.step(&mut e), Step::Ran);
        assert!(t.steps < 100, "loop did not terminate");
    }
    assert_eq!(t.aa[0], 3);
}

/// `copy_from` gives the state a save and load would, also from a TGP
/// that ran another program (decoded from the copy's program then).
#[test]
fn copy_is_a_save_and_load() {
    let prog =
        [ctrl(0x02, 0, 3), alu_ctrl(0x10, 0, 0x19, OUT_AA0, 0, 0, 0), ctrl(0x13, 0, 0xffe), NOP, mov2(AA0, 0x32)];
    let (mut a, mut e) = boot(&prog);
    steps(&mut a, &mut e, 4);
    for other in [&[mov3(7, AA0)][..], &prog] {
        let (mut b, mut f) = boot(other);
        steps(&mut b, &mut f, 1);
        b.copy_from(&a);
        assert_eq!(state::to_bytes(&a), state::to_bytes(&b));
        assert_eq!(b.program_hash, a.program_hash);
        // And runs as a loaded copy does (to the end of the loop).
        let mut c = Tgp::new();
        state::State::load(&mut c, &mut state::Reader::new(&state::to_bytes(&a))).unwrap();
        steps(&mut c, &mut Ext::default(), 6);
        steps(&mut b, &mut f, 6);
        assert_eq!((b.pc, b.aa[0]), (4, 3));
        assert_eq!(state::to_bytes(&c), state::to_bytes(&b));
    }
}

#[test]
fn rep_repeats_next_word() {
    let prog = [ctrl(0x01, 0, 3), alu_ctrl(0x10, 0, 0x19, OUT_AA0, 0, 0, 0), NOP];
    let (mut t, mut e) = boot(&prog);
    steps(&mut t, &mut e, 4);
    assert_eq!(t.aa[0], 3);
    assert_eq!(t.pc, 2);
}

#[test]
fn repeated_nop_takes_its_full_count_of_steps() {
    // REP 20; NOP; then a word that marks completion.
    let prog = [ctrl(0x01, 0, 20), NOP, alu_ctrl(0x10, 0, 0x19, OUT_AA0, 0, 0, 0)];
    let (mut t, mut e) = boot(&prog);
    while t.pc != 2 {
        assert_eq!(t.step(&mut e), Step::Ran);
    }
    assert_eq!(t.steps, 21, "REP word plus 20 NOPs");
    assert_eq!(t.counts[1], 20);
    assert_eq!(t.rpc, 0);
    assert_eq!(t.st & RP, 0);
    t.step(&mut e);
    assert_eq!(t.aa[0], 1);
}

#[test]
fn external_transfer_advances_eo() {
    // F5 external read: TRM=1, DIR=1 (ext -> int), disp6 = +1, DR = AA0
    let rd = put(&[(63, 61, 5), (60, 56, 7), (41, 41, 1), (38, 38, 1), (37, 37, 1), (36, 31, 1), (30, 24, AA0)]);
    let (mut t, mut e) = boot(&[mov3(0x80_0000, 0x10), rd, rd]);
    e.0.insert(0x80_0000, 11);
    e.0.insert(0x80_0001, 22);
    steps(&mut t, &mut e, 2);
    assert_eq!(t.aa[0], 11);
    steps(&mut t, &mut e, 1);
    assert_eq!(t.aa[0], 22);
    assert_eq!(t.eo, 2);
}

#[test]
fn subtract_is_in2_minus_in1() {
    let prog = [mov3(10, AA0), mov3(3, AA1), alu_ctrl(0x12, 1, 0x00, 0x12, 0, 0, 0)]; // SUB AA1, AA0, AA2
    let (mut t, mut e) = boot(&prog);
    steps(&mut t, &mut e, 3);
    assert_eq!(t.aa[2], 7);
}
