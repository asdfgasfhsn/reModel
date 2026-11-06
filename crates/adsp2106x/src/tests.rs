//! Decoder and disassembler tests: encodings built from the opcode
//! diagrams of UM Appendices A and B, and the manual's own examples.

use super::*;

fn put(fields: &[(u32, u32, u64)]) -> u64 {
    fields.iter().fold(0, |w, &(hi, lo, v)| w | ((v << lo) & (((1u64 << (hi - lo + 1)) - 1) << lo)))
}

fn dis(w: u64) -> String {
    decode(0x2_0100, w).to_string()
}

#[test]
fn model2b_reset_vector_and_interrupt_vectors() {
    // The words both Model 2B programs have at 0x20005 and 0x20040.
    let i = decode(0x2_0005, 0x063e_0402_0080);
    assert_eq!(i.to_string(), "JUMP 0x20080 (DB)");
    assert_eq!(i.flow(), Flow::Jump { target: Some(0x2_0080), delayed: true });
    assert_eq!(dis(0x0b3e_0000_0000), "RTI");
    assert_eq!(dis(0x0f7b_0001_8000), "MODE1 = 0x18000");
    assert_eq!(dis(0x0f7a_0006_0000), "MODE2 = 0x60000");
    assert_eq!(dis(0), "NOP");
}

#[test]
fn type1_dual_move_manual_example() {
    // R7=BSET R6 BY R0, DM(I0,M3)=R5, PM(I11,M15)=R4; (UM A-16)
    let cf = 2 << 20 | 0xc0 << 12 | 7 << 8 | 6 << 4;
    let w = put(&[
        (47, 45, 1),
        (44, 44, 1),
        (43, 41, 0),
        (40, 38, 3),
        (37, 37, 1),
        (36, 33, 5),
        (32, 30, 3),
        (29, 27, 7),
        (26, 23, 4),
        (22, 0, cf),
    ]);
    assert_eq!(dis(w), "R7 = BSET R6 BY R0, DM(I0,M3) = R5, PM(I11,M15) = R4");
}

#[test]
fn type3_and_type4_transfers() {
    // R6=R3-R11, DM(I0,M1)=ASTAT; (UM A-18): post-modify, write, DAG1.
    let cf = 0x02 << 12 | 6 << 8 | 3 << 4 | 11;
    let w = put(&[
        (47, 45, 2),
        (44, 44, 1),
        (43, 41, 0),
        (40, 38, 1),
        (37, 33, 31),
        (32, 32, 0),
        (31, 31, 1),
        (30, 23, 0x7c),
        (22, 0, cf),
    ]);
    assert_eq!(dis(w), "R6 = R3 - R11, DM(I0,M1) = ASTAT");
    // IF NOT SV ..., PX=PM(I12,M12): DAG2, read, pre-modify shown as (M,I).
    let w = put(&[
        (47, 45, 2),
        (44, 44, 0),
        (43, 41, 4),
        (40, 38, 4),
        (37, 33, 23),
        (32, 32, 1),
        (31, 31, 0),
        (30, 23, 0xdb),
    ]);
    assert_eq!(dis(w), "IF NOT SV PX = PM(M12,I12)");
    // Type 4: R12=R3 AND R1, DM(6,I1)=R6; (UM A-20).
    let cf = 0x40 << 12 | 12 << 8 | 3 << 4 | 1;
    let w = put(&[
        (47, 44, 0b0110),
        (43, 41, 1),
        (40, 40, 0),
        (39, 39, 1),
        (38, 38, 0),
        (37, 33, 31),
        (32, 27, 6),
        (26, 23, 6),
        (22, 0, cf),
    ]);
    assert_eq!(dis(w), "R12 = R3 AND R1, DM(6,I1) = R6");
    // A negative 6-bit modifier.
    let w = put(&[(47, 44, 0b0110), (43, 41, 2), (40, 40, 1), (38, 38, 1), (37, 33, 31), (32, 27, 0x3f), (26, 23, 11)]);
    assert_eq!(dis(w), "R11 = PM(I10,-1)");
}

#[test]
fn type5_type6_type7() {
    // LCNTR=L7;
    let w = put(&[(47, 44, 0b0111), (43, 36, 0x37), (35, 31, 31), (30, 23, 0x68)]);
    assert_eq!(dis(w), "LCNTR = L7");
    // IF NOT SZ R3=FEXT R1 BY 8:4; (UM A-24): bit6=8, len6=4.
    let imm: u64 = 8 | 4 << 6;
    let w = put(&[
        (47, 40, 0x02),
        (37, 33, 24),
        (30, 27, imm >> 8),
        (21, 16, 0x40 >> 2),
        (15, 8, imm & 0xff),
        (7, 4, 3),
        (3, 0, 1),
    ]);
    assert_eq!(dis(w), "IF NOT SZ R3 = FEXT R1 BY 8:4");
    // The Model 2B programs' LSHIFT by -23.
    assert_eq!(dis(0x023e_7800_e900), "R0 = LSHIFT R0 BY -23");
    // IF NOT LCE MODIFY(I3,M1);
    let w = put(&[(47, 40, 0x04), (38, 38, 0), (37, 33, 15), (32, 30, 3), (29, 27, 1)]);
    assert_eq!(dis(w), "IF NOT LCE MODIFY (I3,M1)");
}

#[test]
fn branches_returns_and_loops() {
    // IF AV JUMP(PC,0xA4)(LA); at 0x20100.
    let w = put(&[(47, 40, 0x07), (38, 38, 1), (37, 33, 4), (23, 0, 0xa4)]);
    assert_eq!(dis(w), "IF AV JUMP (PC,164) {0x201a4} (LA)");
    assert_eq!(decode(0x2_0100, w).flow(), Flow::Branch { target: Some(0x2_01a4), delayed: false });
    // IF EQ CALL(PC,17)(DB), ELSE R6=R6-1; (UM A-31)
    let cf = 0x2a << 12 | 6 << 8 | 6 << 4;
    let w = put(&[(47, 40, 0x09), (39, 39, 1), (37, 33, 0), (32, 27, 17), (26, 26, 1), (25, 25, 1), (22, 0, cf)]);
    assert_eq!(dis(w), "IF EQ CALL (PC,17) {0x20111} (DB), ELSE R6 = R6 - 1");
    // JUMP(M8,I12), R6=R6-1;
    let w = put(&[(47, 40, 0x08), (37, 33, 31), (32, 30, 4), (29, 27, 0), (22, 0, cf)]);
    assert_eq!(dis(w), "JUMP (M8,I12), R6 = R6 - 1");
    assert_eq!(decode(0, w).flow(), Flow::Jump { target: None, delayed: false });
    // IF TF JUMP(M8,I8), ELSE R6=DM(I6,M1); (UM A-32)
    let w =
        put(&[(47, 45, 6), (44, 44, 0), (43, 41, 6), (40, 38, 1), (37, 33, 13), (32, 30, 0), (29, 27, 0), (26, 23, 6)]);
    assert_eq!(dis(w), "IF TF JUMP (M8,I8), ELSE R6 = DM(I6,M1)");
    // IF NOT GT RTS(DB); (code 2, LE, is NOT GT) and RTS (LR).
    assert_eq!(dis(put(&[(47, 40, 0x0a), (37, 33, 2), (26, 26, 1)])), "IF LE RTS (DB)");
    assert_eq!(dis(put(&[(47, 40, 0x0a), (37, 33, 31), (24, 24, 1)])), "RTS (LR)");
    // LCNTR=100, DO (PC,16) UNTIL LCE; and DO (PC,7) UNTIL AC.
    let w = put(&[(47, 40, 0x0c), (39, 24, 100), (23, 0, 16)]);
    assert_eq!(dis(w), "LCNTR = 100, DO 0x20110 UNTIL LCE");
    assert_eq!(dis(put(&[(47, 40, 0x0d), (39, 32, 0x0c), (23, 0, 16)])), "LCNTR = R12, DO 0x20110 UNTIL LCE");
    assert_eq!(dis(put(&[(47, 40, 0x0e), (37, 33, 3), (23, 0, 7)])), "DO 0x20107 UNTIL AC");
    assert_eq!(dis(put(&[(47, 40, 0x0e), (37, 33, 31), (23, 0, 7)])), "DO 0x20107 UNTIL FOREVER");
    // A backward PC-relative jump.
    assert_eq!(dis(put(&[(47, 40, 0x07), (37, 33, 31), (23, 0, 0xff_fffe)])), "JUMP (PC,-2) {0x200fe}");
}

#[test]
fn immediate_moves_and_miscellaneous() {
    // DM(0x15F0)=ASTAT; and DMWAIT-style PM read.
    assert_eq!(
        dis(put(&[(47, 42, 0b000100), (41, 41, 0), (40, 40, 1), (39, 32, 0x7c), (31, 0, 0x15f0)])),
        "DM(0x15f0) = ASTAT"
    );
    assert_eq!(
        dis(put(&[(47, 42, 0b000100), (41, 41, 1), (40, 40, 0), (39, 32, 0x05), (31, 0, 0x48_9060)])),
        "R5 = PM(0x489060)"
    );
    // DM(24,I5)=TCOUNT; USTAT1=PM(offs,I13);
    assert_eq!(
        dis(put(&[(47, 45, 5), (44, 44, 0), (43, 41, 5), (40, 40, 1), (39, 32, 0xdf), (31, 0, 24)])),
        "DM(0x18,I5) = TCOUNT"
    );
    assert_eq!(
        dis(put(&[(47, 45, 5), (44, 44, 1), (43, 41, 5), (40, 40, 0), (39, 32, 0x70), (31, 0, 8)])),
        "USTAT1 = PM(0x8,I13)"
    );
    // DM(I4,M0)=19304; IMASK=0xFFFC0060;
    assert_eq!(
        dis(put(&[(47, 44, 0b1001), (43, 41, 4), (40, 38, 0), (37, 37, 0), (31, 0, 19304)])),
        "DM(I4,M0) = 0x4b68"
    );
    assert_eq!(dis(put(&[(47, 40, 0x0f), (39, 32, 0x7d), (31, 0, 0xfffc_0060)])), "IMASK = 0xfffc0060");
    // BIT SET MODE2 0x70; BIT TST ASTAT 0x2000;
    assert_eq!(dis(put(&[(47, 40, 0x14), (39, 37, 0), (35, 32, 0xa), (31, 0, 0x70)])), "BIT SET MODE2 0x70");
    assert_eq!(dis(put(&[(47, 40, 0x14), (39, 37, 4), (35, 32, 0xc), (31, 0, 0x2000)])), "BIT TST ASTAT 0x2000");
    // MODIFY (I4,304); BITREV (I7,...);
    assert_eq!(dis(put(&[(47, 40, 0x16), (38, 38, 0), (34, 32, 4), (31, 0, 304)])), "MODIFY (I4,0x130)");
    assert_eq!(dis(put(&[(47, 40, 0x16), (39, 39, 1), (38, 38, 1), (34, 32, 7), (31, 0, 8)])), "BITREV (I15,0x8)");
    // PUSH LOOP, PUSH STS; POP PCSTK, FLUSH CACHE;
    assert_eq!(dis(put(&[(47, 40, 0x17), (39, 39, 1), (37, 37, 1)])), "PUSH LOOP, PUSH STS");
    assert_eq!(dis(put(&[(47, 40, 0x17), (34, 34, 1), (33, 33, 1)])), "POP PCSTK, FLUSH CACHE");
    assert_eq!(dis(put(&[(47, 40, 0x00), (39, 39, 1)])), "IDLE");
    // CJUMP and RFRAME (UM A-54, A-55).
    assert_eq!(dis(0x1804_0002_1234), "CJUMP 0x21234 (DB)");
    assert_eq!(dis(0x1900_0000_0000), "RFRAME");
}

#[test]
fn compute_fields() {
    let alu = |op: u32, n: u32, x: u32, y: u32| Compute::decode(op << 12 | n << 8 | x << 4 | y).to_string();
    assert_eq!(alu(0x05, 1, 2, 3), "R1 = R2 + R3 + CI");
    assert_eq!(alu(0x89, 1, 2, 3), "F1 = (F2 + F3)/2");
    assert_eq!(alu(0xd9, 1, 2, 3), "R1 = FIX F2 BY R3");
    assert_eq!(alu(0xda, 1, 2, 3), "F1 = FLOAT R2 BY R3");
    assert_eq!(alu(0xbd, 1, 2, 3), "F1 = SCALB F2 BY R3");
    assert_eq!(alu(0x8a, 0, 2, 3), "COMP(F2, F3)");
    // Dual add/subtract: the opcode's low nibble is Rs.
    assert_eq!(alu(0x74, 1, 2, 3), "R1 = R2 + R3, R4 = R2 - R3");
    assert_eq!(alu(0xf4, 1, 2, 3), "F1 = F2 + F3, F4 = F2 - F3");
    let mul = |op: u32| Compute::decode(1 << 20 | op << 12 | 1 << 8 | 2 << 4 | 3).to_string();
    assert_eq!(mul(0x79), "R1 = R2 * R3 (SSFR)");
    assert_eq!(mul(0x40), "R1 = R2 * R3 (UUI)");
    assert_eq!(mul(0x54), "MRF = R2 * R3 (SUI)");
    assert_eq!(mul(0xbc), "MRF = MRF + R2 * R3 (SSF)");
    assert_eq!(mul(0xfe), "MRB = MRB - R2 * R3 (SSF)");
    assert_eq!(mul(0x82), "R1 = MRB + R2 * R3 (UUI)");
    assert_eq!(mul(0x01), "R1 = SAT MRF (SI)");
    assert_eq!(mul(0x0e), "MRB = SAT MRB (UF)");
    assert_eq!(mul(0x19), "R1 = RND MRF (SF)");
    assert_eq!(mul(0x14), "MRF = 0");
    assert_eq!(mul(0x30), "F1 = F2 * F3");
    let sh = |op: u32| Compute::decode(2 << 20 | op << 12 | 1 << 8 | 2 << 4 | 3).to_string();
    assert_eq!(sh(0x20), "R1 = R1 OR LSHIFT R2 BY R3");
    assert_eq!(sh(0x6c), "R1 = R1 OR FDEP R2 BY R3 (SE)");
    assert_eq!(sh(0x84), "R1 = EXP R2 (EX)");
    assert_eq!(sh(0xcc), "BTST R2 BY R3");
    assert_eq!(sh(0x94), "F1 = FUNPACK R2");
    // MR transfers (UM B-52): 1 00000 T AI RK.
    assert_eq!(Compute::decode(1 << 22 | 1 << 16 | 1 << 12 | 5 << 8).to_string(), "MR1F = R5");
    assert_eq!(Compute::decode(1 << 22 | 6 << 12 | 3 << 8).to_string(), "R3 = MR2B");
    // Parallel multiplier/ALU (UM Table B.7): opcode 011000, F8 = F0*F4,
    // F12 = F8 + F12 with RM=8, RA=12 and the restricted operands.
    let cf = 1 << 22 | 0b011000 << 16 | 8 << 12 | 12 << 8;
    assert_eq!(Compute::decode(cf).to_string(), "F8 = F0 * F4, F12 = F8 + F12");
    let cf = 1 << 22 | 0b001001 << 16 | 1 << 12 | 9 << 8 | 0b11_10_01_00;
    assert_eq!(Compute::decode(cf).to_string(), "MRF = MRF + R3 * R6 (SSF), R9 = R9 - R12");
    // Multiplier and dual add/subtract: 1 1x RS RM RA ...
    let cf = 1 << 22 | 0b11 << 20 | 5 << 16 | 1 << 12 | 2 << 8;
    assert_eq!(Compute::decode(cf).to_string(), "F1 = F0 * F4, F2 = F8 + F12, F5 = F8 - F12");
}

#[test]
fn undefined_encodings_are_reported() {
    let unknown = |w: u64| decode(0x2_0000, w).is_unknown();
    // Group 000 sub-opcodes the manual doesn't define.
    for sub in [0b00011u64, 0b00101, 0b10101, 0b11010, 0b11111] {
        assert!(unknown(put(&[(47, 45, 0), (44, 40, sub)])), "000 {sub:05b}");
    }
    // Undefined compute fields: CU=11, a hole in the ALU table, the
    // multiplier's d=01 form, rounding an integer product.
    assert!(unknown(put(&[(47, 40, 0x01), (37, 33, 31), (22, 0, 3 << 20 | 1)])));
    assert!(unknown(put(&[(47, 40, 0x01), (37, 33, 31), (22, 0, 0x03 << 12)])));
    assert!(unknown(put(&[(47, 40, 0x01), (37, 33, 31), (22, 0, 1 << 20 | 0x42 << 12)])));
    assert!(unknown(put(&[(47, 40, 0x01), (37, 33, 31), (22, 0, 1 << 20 | 0x71 << 12)])));
    // Universal register codes outside Figures A.1/A.2, a write to PC,
    // an undefined BIT operation and system register.
    assert!(unknown(put(&[(47, 40, 0x0f), (39, 32, 0x55)])));
    assert!(unknown(put(&[(47, 40, 0x0f), (39, 32, 0x63)])));
    assert!(unknown(put(&[(47, 40, 0x14), (39, 37, 0b011), (35, 32, 0xb)])));
    assert!(unknown(put(&[(47, 40, 0x14), (39, 37, 0), (35, 32, 0x3)])));
    // A compute-only shifter op in the immediate-shift form.
    assert!(unknown(put(&[(47, 40, 0x02), (37, 33, 31), (21, 16, 0x80 >> 2)])));
    assert_eq!(decode(0, put(&[(47, 40, 0x03)])).flow(), Flow::Invalid);
    assert!(decode(0, put(&[(47, 40, 0x03)])).to_string().contains("undefined"));
}
