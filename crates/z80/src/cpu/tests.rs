use super::*;

/// 64 KB of RAM; reads from port P return `ins` in turn, writes are logged.
struct Ram {
    mem: Vec<u8>,
    ins: Vec<u8>,
    outs: Vec<(u16, u8)>,
    ack: u8,
    retis: u32,
}

impl Ram {
    fn new() -> Self {
        Ram { mem: vec![0; 0x1_0000], ins: Vec::new(), outs: Vec::new(), ack: 0xff, retis: 0 }
    }
    fn w16(&mut self, a: u16, v: u16) {
        self.mem[a as usize] = v as u8;
        self.mem[a as usize + 1] = (v >> 8) as u8;
    }
    fn r16(&self, a: u16) -> u16 {
        self.mem[a as usize] as u16 | (self.mem[a as usize + 1] as u16) << 8
    }
}

impl Bus for Ram {
    fn read(&mut self, a: u16) -> u8 {
        self.mem[a as usize]
    }
    fn write(&mut self, a: u16, v: u8) {
        self.mem[a as usize] = v;
    }
    fn input(&mut self, _port: u16) -> u8 {
        if self.ins.is_empty() {
            0xff
        } else {
            self.ins.remove(0)
        }
    }
    fn output(&mut self, port: u16, v: u8) {
        self.outs.push((port, v));
    }
    fn int_ack(&mut self) -> u8 {
        self.ack
    }
    fn reti(&mut self) {
        self.retis += 1;
    }
}

const CODE: u16 = 0x1000;

/// A CPU at `code` (loaded at 0x1000) with the stack at 0x8000.
fn boot(code: &[u8]) -> (Cpu, Ram) {
    let mut ram = Ram::new();
    ram.mem[CODE as usize..CODE as usize + code.len()].copy_from_slice(code);
    let mut cpu = Cpu::new();
    cpu.pc = CODE;
    cpu.sp = 0x8000;
    (cpu, ram)
}

/// Run until HALT; returns the T-states used, the HALT's included.
fn run(cpu: &mut Cpu, ram: &mut Ram) -> u32 {
    let mut t = 0;
    for _ in 0..100_000 {
        t += cpu.step(ram);
        if cpu.halted() {
            return t;
        }
    }
    panic!("no HALT");
}

/// Run `code` (which must end in HALT).
fn exec(code: &[u8]) -> (Cpu, Ram) {
    let (mut cpu, mut ram) = boot(code);
    run(&mut cpu, &mut ram);
    (cpu, ram)
}

const HALT: u8 = 0x76;

#[test]
fn daa_after_add_and_sub() {
    // LD A,0x15; ADD A,0x27; DAA
    let (c, _) = exec(&[0x3e, 0x15, 0xc6, 0x27, 0x27, HALT]);
    assert_eq!(c.a, 0x42);
    assert_eq!(c.f & (C | N | Z), 0);
    // LD A,0x99; ADD A,0x01; DAA: 100 -> 00 with carry.
    let (c, _) = exec(&[0x3e, 0x99, 0xc6, 0x01, 0x27, HALT]);
    assert_eq!(c.a, 0x00);
    assert_eq!(c.f & (C | Z | P), C | Z | P);
    // LD A,0x42; SUB 0x15; DAA: 27, N kept.
    let (c, _) = exec(&[0x3e, 0x42, 0xd6, 0x15, 0x27, HALT]);
    assert_eq!(c.a, 0x27);
    assert_eq!(c.f & (C | N), N);
    // LD A,0; SUB 1; DAA: 0 - 1 = 99 with borrow.
    let (c, _) = exec(&[0x3e, 0x00, 0xd6, 0x01, 0x27, HALT]);
    // H after a subtraction: H in and a low digit under 6 (here 0xf).
    assert_eq!(c.a, 0x99);
    assert_eq!(c.f & (C | N | S | H | P), C | N | S | P);
    // Half-carry from the low digit: 0x08 + 0x08 = 0x10 -> 16.
    let (c, _) = exec(&[0x3e, 0x08, 0xc6, 0x08, 0x27, HALT]);
    assert_eq!(c.a, 0x16);
    assert_eq!(c.f & H, 0);
}

#[test]
fn ldir_copies_and_times() {
    // LD HL,0x2000; LD DE,0x3000; LD BC,5; LD A,0; LDIR; HALT
    let (mut c, mut r) = boot(&[0x21, 0x00, 0x20, 0x11, 0x00, 0x30, 0x01, 0x05, 0x00, 0x3e, 0, 0xed, 0xb0, HALT]);
    r.mem[0x2000..0x2005].copy_from_slice(&[1, 2, 3, 4, 0x0a]);
    for _ in 0..4 {
        c.step(&mut r);
    }
    let t: u32 = (0..5).map(|_| c.step(&mut r)).sum();
    assert_eq!(t, 4 * 21 + 16);
    assert_eq!(&r.mem[0x3000..0x3005], &[1, 2, 3, 4, 0x0a]);
    assert_eq!((c.bc, c.hl(), c.de), (0, 0x2005, 0x3005));
    // P/V clear at the end; X from bit 3, Y from bit 1 of A + last byte (0x0a).
    assert_eq!(c.f & (P | H | N | X | Y), X | Y);
    assert_eq!(c.pc, CODE + 13);
}

#[test]
fn lddr_backwards() {
    // LD HL,0x2002; LD DE,0x3002; LD BC,3; LDDR; HALT
    let (c, r) = {
        let (mut c, mut r) = boot(&[0x21, 0x02, 0x20, 0x11, 0x02, 0x30, 0x01, 0x03, 0x00, 0xed, 0xb8, HALT]);
        r.mem[0x2000..0x2003].copy_from_slice(&[7, 8, 9]);
        run(&mut c, &mut r);
        (c, r)
    };
    assert_eq!(&r.mem[0x3000..0x3003], &[7, 8, 9]);
    assert_eq!((c.bc, c.hl(), c.de), (0, 0x1fff, 0x2fff));
}

#[test]
fn cpir_finds_and_stops() {
    // LD HL,0x2000; LD BC,8; LD A,0x33; CPIR; HALT
    let (mut c, mut r) = boot(&[0x21, 0x00, 0x20, 0x01, 0x08, 0x00, 0x3e, 0x33, 0xed, 0xb1, HALT]);
    r.mem[0x2000..0x2008].copy_from_slice(&[0x11, 0x22, 0x33, 0x44, 0, 0, 0, 0]);
    run(&mut c, &mut r);
    assert_eq!(c.hl(), 0x2003);
    assert_eq!(c.bc, 5);
    assert_eq!(c.f & (Z | P | N), Z | P | N);
    // Not found: runs out, Z and P/V clear.
    let (mut c, mut r) = boot(&[0x21, 0x00, 0x20, 0x01, 0x03, 0x00, 0x3e, 0x55, 0xed, 0xb1, HALT]);
    r.mem[0x2000..0x2003].copy_from_slice(&[1, 2, 3]);
    run(&mut c, &mut r);
    assert_eq!((c.hl(), c.bc), (0x2003, 0));
    assert_eq!(c.f & (Z | P), 0);
}

#[test]
fn inir_and_otir() {
    // LD HL,0x2000; LD BC,0x0310; INIR; HALT
    let (mut c, mut r) = boot(&[0x21, 0x00, 0x20, 0x01, 0x10, 0x03, 0xed, 0xb2, HALT]);
    r.ins = vec![0xaa, 0xbb, 0xcc];
    run(&mut c, &mut r);
    assert_eq!(&r.mem[0x2000..0x2003], &[0xaa, 0xbb, 0xcc]);
    assert_eq!((c.bc, c.hl()), (0x0010, 0x2003));
    assert_ne!(c.f & Z, 0);
    // N from bit 7 of the last byte; k = 0xcc + 0x11 = 0xdd, no carry.
    assert_eq!(c.f & (N | C | H), N);

    // LD HL,0x2000; LD BC,0x0340; OTIR; HALT
    let (mut c, mut r) = boot(&[0x21, 0x00, 0x20, 0x01, 0x40, 0x03, 0xed, 0xb3, HALT]);
    r.mem[0x2000..0x2003].copy_from_slice(&[1, 2, 0xf0]);
    run(&mut c, &mut r);
    // The port's high byte is B after the decrement.
    assert_eq!(r.outs, vec![(0x0240, 1), (0x0140, 2), (0x0040, 0xf0)]);
    assert_eq!((c.bc, c.hl()), (0x0040, 0x2003));
    // k = 0xf0 + L (0x03) = 0xf3: no carry; P = parity(3 ^ 0) = even.
    assert_eq!(c.f & (Z | N | C | P), Z | N | P);
}

#[test]
fn add_adc_sbc_16_flags() {
    // SCF; LD HL,0x0fff; LD BC,1; ADD HL,BC: H set, C cleared, S/Z/PV kept.
    let (c, _) = exec(&[0x37, 0x21, 0xff, 0x0f, 0x01, 0x01, 0x00, 0x09, HALT]);
    assert_eq!(c.hl(), 0x1000);
    assert_eq!(c.f & (H | C | N), H);
    // ADD: X/Y from the high byte of the result (0x28 -> both).
    let (c, _) = exec(&[0x21, 0x00, 0x28, 0x01, 0x00, 0x00, 0x09, HALT]);
    assert_eq!(c.f & XY, XY);
    // LD HL,0xffff; LD DE,1; ADD HL,DE: carry out.
    let (c, _) = exec(&[0x21, 0xff, 0xff, 0x11, 0x01, 0x00, 0x19, HALT]);
    assert_eq!(c.hl(), 0);
    assert_eq!(c.f & (C | H), C | H);
    // SCF; LD HL,0x7fff; LD BC,0; ADC HL,BC: overflow to 0x8000.
    let (c, _) = exec(&[0x37, 0x21, 0xff, 0x7f, 0x01, 0x00, 0x00, 0xed, 0x4a, HALT]);
    assert_eq!(c.hl(), 0x8000);
    assert_eq!(c.f, S | P | H);
    // OR A; LD HL,0x8000; LD BC,1; SBC HL,BC: overflow to 0x7fff.
    let (c, _) = exec(&[0xb7, 0x21, 0x00, 0x80, 0x01, 0x01, 0x00, 0xed, 0x42, HALT]);
    assert_eq!(c.hl(), 0x7fff);
    assert_eq!(c.f, P | H | N | XY);
    // OR A; LD HL,0; LD BC,1; SBC HL,BC: borrow.
    let (c, _) = exec(&[0xb7, 0x21, 0x00, 0x00, 0x01, 0x01, 0x00, 0xed, 0x42, HALT]);
    assert_eq!(c.hl(), 0xffff);
    assert_eq!(c.f, S | H | N | C | XY);
    // OR A; SBC HL,HL: zero (Z is set on the 16-bit result).
    let (c, _) = exec(&[0xb7, 0x21, 0x34, 0x12, 0xed, 0x62, HALT]);
    assert_eq!(c.hl(), 0);
    assert_eq!(c.f, Z | N);
    // ADC HL with a zero result and carry: 0xffff + 0 + 1.
    let (c, _) = exec(&[0x37, 0x21, 0xff, 0xff, 0x01, 0x00, 0x00, 0xed, 0x4a, HALT]);
    assert_eq!(c.hl(), 0);
    assert_eq!(c.f, Z | H | C);
}

#[test]
fn bit_undocumented_flags() {
    // LD A,(0x2827) sets MEMPTR to 0x2828; BIT 0,(HL) shows its high byte.
    let (c, _) = exec(&[0x3a, 0x27, 0x28, 0x21, 0x00, 0x30, 0xcb, 0x46, HALT]);
    assert_eq!(c.wz, 0x2828);
    assert_eq!(c.f & (XY | Z | P | H | N), XY | Z | P | H);
    // BIT 7,B with bit 7 set: S, and X/Y from B (0xa8).
    let (c, _) = exec(&[0x06, 0xa8, 0xcb, 0x78, HALT]);
    assert_eq!(c.f & (S | Z | XY | H), S | X | Y | H);
    // LD IX,0x2800; BIT 1,(IX-1): X/Y from 0x27, the high byte of IX+d.
    let (mut c, mut r) = boot(&[0xdd, 0x21, 0x00, 0x28, 0xdd, 0xcb, 0xff, 0x4e, HALT]);
    r.mem[0x27ff] = 0x02;
    run(&mut c, &mut r);
    assert_eq!(c.f & (XY | Z | H | S), Y | H);
    // The same through FDCB with a clear bit.
    let (c, _) = exec(&[0xfd, 0x21, 0x10, 0x08, 0xfd, 0xcb, 0x00, 0x46, HALT]);
    assert_eq!(c.f & (XY | Z | P), X | Z | P);
}

#[test]
fn index_halves_and_ddcb_copy() {
    // LD IX,0x1234; LD A,IXH (DD 7C); ADD A,IXL (DD 85); LD IYL,0x55 (FD 2E 55)
    let (c, _) = exec(&[0xdd, 0x21, 0x34, 0x12, 0xdd, 0x7c, 0xdd, 0x85, 0xfd, 0x2e, 0x55, HALT]);
    assert_eq!(c.a, 0x46);
    assert_eq!(c.iy() & 0xff, 0x55);
    // LD H,(IX+1) uses the real H.
    let (mut c, mut r) = boot(&[0xdd, 0x21, 0x00, 0x20, 0xdd, 0x66, 0x01, HALT]);
    r.mem[0x2001] = 0x9a;
    run(&mut c, &mut r);
    assert_eq!((c.hl() >> 8, c.ix()), (0x9a, 0x2000));
    // RLC (IX+0),B (DD CB 00 00): result in memory and in B.
    let (mut c, mut r) = boot(&[0xdd, 0x21, 0x00, 0x20, 0xdd, 0xcb, 0x00, 0x00, HALT]);
    r.mem[0x2000] = 0x81;
    run(&mut c, &mut r);
    assert_eq!((r.mem[0x2000], (c.bc >> 8) as u8), (0x03, 0x03));
    assert_eq!(c.f & C, C);
    // SLL B (CB 30): shift left, bit 0 set.
    let (c, _) = exec(&[0x06, 0x80, 0xcb, 0x30, HALT]);
    assert_eq!(c.bc >> 8, 0x01);
    assert_eq!(c.f & C, C);
}

#[test]
fn im2_vector_fetch() {
    // IM 2; LD A,0x40; LD I,A; EI; NOP; NOP...
    let (mut c, mut r) = boot(&[0xed, 0x5e, 0x3e, 0x40, 0xed, 0x47, 0xfb, 0x00, 0x00, 0x00]);
    r.ack = 0x10;
    r.w16(0x4010, 0x1234);
    for _ in 0..5 {
        c.step(&mut r);
    }
    assert_eq!(c.im, 2);
    c.set_int(true);
    // EI's delay has passed (a NOP ran after it).
    let t = c.step(&mut r);
    assert_eq!(t, 19);
    assert_eq!(c.pc, 0x1234);
    assert_eq!(r.r16(c.sp), CODE + 8);
    assert!(!c.iff1 && !c.iff2);
}

#[test]
fn im0_runs_the_rst_from_the_bus() {
    // IM 0 (the reset mode); EI; NOP; the device supplies RST 0x28.
    let (mut c, mut r) = boot(&[0xfb, 0x00, 0x00]);
    r.ack = 0xef;
    c.step(&mut r);
    c.step(&mut r);
    c.set_int(true);
    assert_eq!(c.step(&mut r), 13);
    assert_eq!(c.pc, 0x28);
    assert_eq!(r.r16(c.sp), CODE + 2);
}

#[test]
fn ei_delays_one_instruction() {
    // IM 1; EI; INC A; INC A; with INT held.
    let (mut c, mut r) = boot(&[0xed, 0x56, 0xfb, 0x3c, 0x3c]);
    c.set_int(true);
    c.step(&mut r); // IM 1
    c.step(&mut r); // EI
    c.step(&mut r); // INC A: no interrupt yet
    assert_eq!(c.a, 0);
    assert_eq!(c.step(&mut r), 13);
    assert_eq!(c.pc, 0x38);
    assert_eq!(r.r16(c.sp), CODE + 4);
}

#[test]
fn halt_then_interrupt_returns_after_halt() {
    // IM 1; EI; HALT
    let (mut c, mut r) = boot(&[0xed, 0x56, 0xfb, HALT, 0x00]);
    for _ in 0..3 {
        c.step(&mut r);
    }
    assert!(c.halted());
    assert_eq!(c.step(&mut r), 4);
    c.set_int(true);
    c.step(&mut r);
    assert!(!c.halted());
    assert_eq!(c.pc, 0x38);
    assert_eq!(r.r16(c.sp), CODE + 4);
}

#[test]
fn nmi_keeps_iff2_for_retn() {
    // EI; NOP; NOP; at 0x66: RETN
    let (mut c, mut r) = boot(&[0xfb, 0x00, 0x00]);
    r.mem[0x66..0x68].copy_from_slice(&[0xed, 0x45]);
    c.step(&mut r);
    c.step(&mut r);
    c.set_nmi(true);
    assert_eq!(c.step(&mut r), 11);
    assert_eq!(c.pc, 0x66);
    assert!(!c.iff1 && c.iff2);
    // Held high: no second NMI.
    assert_eq!(c.step(&mut r), 14);
    assert_eq!(c.pc, CODE + 2);
    assert!(c.iff1);
}

#[test]
fn reti_tells_the_bus() {
    let (mut c, mut r) = boot(&[0xed, 0x4d]);
    r.w16(0x8000, 0x2000);
    c.step(&mut r);
    assert_eq!((c.pc, r.retis), (0x2000, 1));
}

#[test]
fn refresh_register() {
    // LD A,0x80; LD R,A; NOP; LD IX,0; CB 00 (RLC B); LD A,R
    let (c, _) = exec(&[0x3e, 0x80, 0xed, 0x4f, 0x00, 0xdd, 0x21, 0, 0, 0xcb, 0x00, 0xed, 0x5f, HALT]);
    // After LD R,A: NOP 1, LD IX 2, RLC B 2, LD A,R 2 -> 7; bit 7 kept.
    assert_eq!(c.a, 0x87);
    // The low seven bits wrap without touching bit 7.
    let mut c = Cpu::new();
    c.r = 0xff;
    c.inc_r();
    assert_eq!(c.r, 0x80);
}

#[test]
fn ports_carry_the_high_byte() {
    // LD A,0x12; OUT (0x34),A; LD BC,0x5678; LD A,0x9a; OUT (C),A; IN E,(C)
    let (mut c, mut r) = boot(&[0x3e, 0x12, 0xd3, 0x34, 0x01, 0x78, 0x56, 0x3e, 0x9a, 0xed, 0x79, 0xed, 0x58, HALT]);
    r.ins = vec![0x00];
    run(&mut c, &mut r);
    assert_eq!(r.outs, vec![(0x1234, 0x12), (0x5678, 0x9a)]);
    assert_eq!(c.de & 0xff, 0);
    assert_eq!(c.f & (Z | P), Z | P);
}

#[test]
fn instruction_timing() {
    let cases: &[(&[u8], u32)] = &[
        (&[0x00], 4),
        (&[0x01, 0, 0], 10),
        (&[0xdd, 0x21, 0, 0], 14),
        (&[0xdd, 0x7e, 0], 19),
        (&[0xdd, 0x36, 0, 0], 19),
        (&[0xdd, 0x34, 0], 23),
        (&[0xdd, 0xcb, 0, 0x06], 23),
        (&[0xdd, 0xcb, 0, 0x46], 20),
        (&[0xcb, 0x00], 8),
        (&[0xcb, 0x06], 15),
        (&[0xcb, 0x46], 12),
        (&[0xed, 0x42], 15),
        (&[0xed, 0x43, 0, 0], 20),
        (&[0xed, 0x67], 18),
        (&[0xed, 0x00], 8),
        (&[0xcd, 0, 0], 17),
        (&[0xc4, 0, 0], 17), // CALL NZ taken (F = 0)
        (&[0xe3], 19),
        (&[0xdd, 0xe9], 8),
        (&[0x10, 0], 13),
    ];
    for (code, t) in cases {
        let (mut c, mut r) = boot(code);
        c.f = 0;
        c.bc = 0x0200;
        assert_eq!(c.step(&mut r), *t, "{code:02x?}");
    }
    let (mut c, mut r) = boot(&[0xc4, 0, 0]);
    c.f = Z;
    assert_eq!(c.step(&mut r), 10);
}

#[test]
fn save_state_round_trip() {
    let (mut c, mut r) = boot(&[0x3e, 0x42, 0x08, 0xd9, 0xed, 0x5e, 0xfb, 0x00]);
    c.set_ix(0x1111);
    for _ in 0..5 {
        c.step(&mut r);
    }
    c.set_int(true);
    c.cyc = 0;
    let bytes = state::to_bytes(&c);
    let mut d = Cpu::new();
    state::State::load(&mut d, &mut state::Reader::new(&bytes)).unwrap();
    assert_eq!(format!("{c:?}"), format!("{d:?}"));
}
