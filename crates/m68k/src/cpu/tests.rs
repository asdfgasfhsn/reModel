use super::*;

/// 64 KB of RAM, mirrored through the address space.
struct Ram(Vec<u8>);

impl Ram {
    fn new() -> Self {
        Ram(vec![0; 0x1_0000])
    }
    fn at(&self, a: u32) -> usize {
        a as usize & (self.0.len() - 1)
    }
    fn w16(&mut self, a: u32, v: u16) {
        self.write16(a, v);
    }
    fn w32(&mut self, a: u32, v: u32) {
        self.write16(a, (v >> 16) as u16);
        self.write16(a + 2, v as u16);
    }
    fn r32(&mut self, a: u32) -> u32 {
        (self.read16(a) as u32) << 16 | self.read16(a + 2) as u32
    }
    fn load(&mut self, a: u32, words: &[u16]) {
        for (i, w) in words.iter().enumerate() {
            self.w16(a + 2 * i as u32, *w);
        }
    }
}

impl Bus for Ram {
    fn read8(&mut self, a: u32) -> u8 {
        self.0[self.at(a)]
    }
    fn read16(&mut self, a: u32) -> u16 {
        (self.read8(a) as u16) << 8 | self.read8(a + 1) as u16
    }
    fn write8(&mut self, a: u32, v: u8) {
        let i = self.at(a);
        self.0[i] = v;
    }
    fn write16(&mut self, a: u32, v: u16) {
        self.write8(a, (v >> 8) as u8);
        self.write8(a + 1, v as u8);
    }
}

const CODE: u32 = 0x1000;
const STACK: u32 = 0x8000;

/// A CPU reset into `code` at 0x1000 with the stack at 0x8000.
fn boot(code: &[u16]) -> (Cpu, Ram) {
    let mut ram = Ram::new();
    ram.w32(0, STACK);
    ram.w32(4, CODE);
    ram.load(CODE, code);
    let mut cpu = Cpu::new();
    cpu.reset(&mut ram);
    (cpu, ram)
}

fn ccr(cpu: &Cpu) -> u16 {
    cpu.sr() & 0x1f
}

const X: u16 = 0x10;
const N: u16 = 8;
const Z: u16 = 4;
const V: u16 = 2;
const C: u16 = 1;

#[test]
fn reset_loads_ssp_and_pc() {
    let (cpu, _) = boot(&[0x4e71]);
    assert_eq!(cpu.a[7], STACK);
    assert_eq!(cpu.ssp(), STACK);
    assert_eq!(cpu.pc, CODE);
    assert_eq!(cpu.sr(), 0x2700);
}

#[test]
fn move_sets_flags() {
    // move.w #$8000,d0 ; move.l #0,d1
    let (mut cpu, mut ram) = boot(&[0x303c, 0x8000, 0x223c, 0, 0]);
    cpu.set_sr(0x2700 | X | V | C);
    cpu.step(&mut ram);
    assert_eq!(cpu.d[0] & 0xffff, 0x8000);
    assert_eq!(ccr(&cpu), X | N);
    cpu.step(&mut ram);
    assert_eq!(ccr(&cpu), X | Z);
    assert_eq!(cpu.pc, CODE + 10);
}

#[test]
fn addx_z_is_sticky() {
    // addx.b d1,d0 twice: 0+0 keeps Z, then 1+0 clears it, then 0+0 keeps it clear.
    let (mut cpu, mut ram) = boot(&[0xd101, 0xd101, 0xd101]);
    cpu.set_sr(0x2700 | Z);
    cpu.step(&mut ram);
    assert_eq!(ccr(&cpu), Z);
    cpu.d[1] = 1;
    cpu.step(&mut ram);
    assert_eq!(ccr(&cpu), 0);
    cpu.d[0] = 0;
    cpu.d[1] = 0;
    cpu.step(&mut ram);
    assert_eq!(ccr(&cpu), 0);
}

#[test]
fn divu_overflow_leaves_register() {
    // divu.w d1,d0
    let (mut cpu, mut ram) = boot(&[0x80c1, 0x80c1]);
    cpu.d[0] = 0x0010_0000;
    cpu.d[1] = 2;
    cpu.step(&mut ram);
    assert_eq!(cpu.d[0], 0x0010_0000);
    assert_ne!(ccr(&cpu) & V, 0);
    cpu.d[0] = 100_003;
    cpu.d[1] = 10;
    cpu.step(&mut ram);
    assert_eq!(cpu.d[0], 3 << 16 | 10_000);
    assert_eq!(ccr(&cpu) & (V | C | Z | N), 0);
}

#[test]
fn divide_by_zero_traps() {
    let (mut cpu, mut ram) = boot(&[0x80c1]);
    ram.w32(5 * 4, 0x2000);
    cpu.step(&mut ram);
    assert_eq!(cpu.pc, 0x2000);
    assert_eq!(ram.r32(STACK - 4), CODE + 2);
}

#[test]
fn abcd_decimal_add() {
    // abcd d1,d0: 0x45 + 0x38 + X = 0x84; then 0x99 + 0x01 = 0x00 carry.
    let (mut cpu, mut ram) = boot(&[0xc101, 0xc101]);
    cpu.d[0] = 0x45;
    cpu.d[1] = 0x38;
    cpu.set_sr(0x2700 | X | Z);
    cpu.step(&mut ram);
    assert_eq!(cpu.d[0] & 0xff, 0x84);
    assert_eq!(ccr(&cpu) & (X | C | Z), 0);
    cpu.d[0] = 0x99;
    cpu.d[1] = 0x01;
    cpu.step(&mut ram);
    assert_eq!(cpu.d[0] & 0xff, 0x00);
    assert_eq!(ccr(&cpu) & (X | C), X | C);
}

#[test]
fn autovectored_interrupt() {
    // nop loop; level 3 autovector (27) -> 0x3000.
    let (mut cpu, mut ram) = boot(&[0x4e71, 0x4e71, 0x4e71]);
    ram.w32(27 * 4, 0x3000);
    cpu.set_sr(0x2200);
    cpu.step(&mut ram);
    cpu.set_irq(2);
    cpu.step(&mut ram);
    assert_eq!(cpu.pc, CODE + 4, "level 2 is masked at mask 2");
    cpu.set_irq(3);
    cpu.step(&mut ram);
    assert_eq!(cpu.pc, 0x3000);
    assert_eq!(cpu.sr() & 0x2700, 0x2300);
    assert_eq!(ram.read16(STACK - 6), 0x2200);
    assert_eq!(ram.r32(STACK - 4), CODE + 4);
}

#[test]
fn stop_waits_for_interrupt() {
    // stop #$2000, then nop.
    let (mut cpu, mut ram) = boot(&[0x4e72, 0x2000, 0x4e71]);
    ram.w32(30 * 4, 0x3000);
    cpu.step(&mut ram);
    assert!(cpu.stopped());
    for _ in 0..10 {
        assert_eq!(cpu.step(&mut ram), 4);
    }
    assert_eq!(cpu.pc, CODE + 4);
    cpu.set_irq(6);
    cpu.step(&mut ram);
    assert!(!cpu.stopped());
    assert_eq!(cpu.pc, 0x3000);
    assert_eq!(ram.r32(STACK - 4), CODE + 4);
}

#[test]
fn nmi_is_edge_triggered() {
    let (mut cpu, mut ram) = boot(&[0x4e71; 8]);
    ram.w32(31 * 4, 0x3000);
    ram.load(0x3000, &[0x4e71; 4]);
    cpu.set_irq(7);
    cpu.step(&mut ram);
    assert_eq!(cpu.pc, 0x3000);
    // Still held at 7 under mask 7: no second interrupt.
    cpu.step(&mut ram);
    assert_eq!(cpu.pc, 0x3002);
}

#[test]
fn user_mode_swaps_stacks_and_traps_privileged() {
    // move #0,sr (enter user mode) ; move #$2700,sr (privileged in user mode)
    let (mut cpu, mut ram) = boot(&[0x46fc, 0x0000, 0x46fc, 0x2700]);
    ram.w32(8 * 4, 0x2000);
    cpu.set_usp(0x6000);
    cpu.step(&mut ram);
    assert_eq!(cpu.a[7], 0x6000);
    assert_eq!(cpu.ssp(), STACK);
    cpu.step(&mut ram);
    assert_eq!(cpu.pc, 0x2000);
    assert_eq!(cpu.a[7], STACK - 6);
    assert_eq!(ram.r32(STACK - 4), CODE + 4, "privilege violation stacks the faulting PC");
    assert_eq!(cpu.usp(), 0x6000);
}

#[test]
fn odd_word_access_is_address_error() {
    // move.w (a0),d0 with a0 odd.
    let (mut cpu, mut ram) = boot(&[0x3010]);
    ram.w32(3 * 4, 0x2000);
    cpu.a[0] = 0x4001;
    cpu.step(&mut ram);
    assert_eq!(cpu.pc, 0x2000);
    assert_eq!(cpu.a[7], STACK - 14);
    assert_eq!(ram.r32(STACK - 12), 0x4001);
    assert_eq!(ram.read16(STACK - 8), 0x3010);
}

#[test]
fn movem_round_trip() {
    // movem.l d0-d2/a0,-(a7) ; movem.l (a7)+,d3-d5/a1
    let (mut cpu, mut ram) = boot(&[0x48e7, 0xe080, 0x4cdf, 0x0238]);
    cpu.d[0] = 1;
    cpu.d[1] = 2;
    cpu.d[2] = 3;
    cpu.a[0] = 4;
    cpu.step(&mut ram);
    assert_eq!(cpu.a[7], STACK - 16);
    assert_eq!(ram.r32(STACK - 16), 1);
    cpu.step(&mut ram);
    assert_eq!(cpu.a[7], STACK);
    assert_eq!((cpu.d[3], cpu.d[4], cpu.d[5], cpu.a[1]), (1, 2, 3, 4));
}

#[test]
fn shifts_and_rotates() {
    // asl.b #1,d0 (V set: sign changes) ; roxr.w #1,d1 through X ; lsr.l d2,d3 (count 0)
    let (mut cpu, mut ram) = boot(&[0xe300, 0xe251, 0xe4ab]);
    cpu.d[0] = 0x40;
    cpu.step(&mut ram);
    assert_eq!(cpu.d[0], 0x80);
    assert_eq!(ccr(&cpu), N | V);
    cpu.d[1] = 1;
    cpu.set_sr(0x2700 | X);
    cpu.step(&mut ram);
    assert_eq!(cpu.d[1], 0x8000);
    assert_eq!(ccr(&cpu), X | N | C);
    cpu.d[2] = 64;
    cpu.d[3] = 5;
    cpu.step(&mut ram);
    assert_eq!(cpu.d[3], 5);
    assert_eq!(ccr(&cpu), X);
}

#[test]
fn bsr_rts_and_dbra() {
    // 1000: moveq #2,d0 ; 1002: bsr.s 100a ; 1004: dbra d0,1002 ; 1008: nop ; 100a: rts
    let (mut cpu, mut ram) = boot(&[0x7002, 0x6106, 0x51c8, 0xfffc, 0x4e71, 0x4e75]);
    let mut calls = 0;
    while cpu.pc != CODE + 8 {
        if cpu.pc == CODE + 10 {
            calls += 1;
        }
        cpu.step(&mut ram);
    }
    assert_eq!(calls, 3);
    assert_eq!(cpu.d[0] & 0xffff, 0xffff);
    assert_eq!(cpu.a[7], STACK);
}

/// Every opcode the disassembler rejects must raise illegal/line A/line F
/// in the interpreter, and vice versa.
#[test]
fn decoder_agrees_with_disassembler() {
    for op in 0..=0xffffu16 {
        let (text, _) = crate::disasm(CODE, |a| if a == CODE { op } else { 0 });
        let mut cpu = Cpu::new();
        let mut ram = Ram::new();
        for v in [VEC_ILLEGAL, VEC_LINE_A, VEC_LINE_F] {
            ram.w32(v as u32 * 4, 0x7f00);
        }
        ram.w16(CODE, op);
        cpu.a = [0x2000; 8];
        cpu.pc = CODE;
        cpu.step(&mut ram);
        let illegal = cpu.pc == 0x7f00 && ram.r32(cpu.a[7] + 2) == CODE;
        assert_eq!(illegal, text.starts_with("dc.w") || op == 0x4afc, "{op:04x}: {text}");
    }
}
