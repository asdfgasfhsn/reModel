use super::*;

const G0: u32 = 16;
const G1: u32 = 17;
const G2: u32 = 18;
const G3: u32 = 19;
const R4: u32 = 4;

struct Ram(Vec<u8>);

impl Ram {
    fn new() -> Self {
        Ram(vec![0; 0x10_0000])
    }
    fn at(&self, a: u32) -> usize {
        a as usize & (self.0.len() - 1)
    }
    fn load(&mut self, addr: u32, words: &[u32]) {
        for (i, w) in words.iter().enumerate() {
            self.write32(addr + 4 * i as u32, *w);
        }
    }
}

impl Bus for Ram {
    fn read8(&mut self, a: u32) -> u8 {
        self.0[self.at(a)]
    }
    fn read16(&mut self, a: u32) -> u16 {
        u16::from_le_bytes([self.read8(a), self.read8(a + 1)])
    }
    fn read32(&mut self, a: u32) -> u32 {
        self.fetch(a).unwrap()
    }
    fn write8(&mut self, a: u32, v: u8) {
        let i = self.at(a);
        self.0[i] = v;
    }
    fn write16(&mut self, a: u32, v: u16) {
        for (i, b) in v.to_le_bytes().into_iter().enumerate() {
            self.write8(a + i as u32, b);
        }
    }
    fn write32(&mut self, a: u32, v: u32) {
        for (i, b) in v.to_le_bytes().into_iter().enumerate() {
            self.write8(a + i as u32, b);
        }
    }
    fn fetch(&self, a: u32) -> Option<u32> {
        let i = self.at(a);
        Some(u32::from_le_bytes(self.0[i..i + 4].try_into().unwrap()))
    }
}

// Minimal encoders for hand-written test programs.
fn reg(op: u32, src1: u32, src2: u32, dst: u32, m1: bool) -> u32 {
    ((op >> 4) << 24) | (dst << 19) | (src2 << 14) | ((m1 as u32) << 11) | ((op & 0xf) << 7) | src1
}
fn lit(op: u32, lit1: u32, src2: u32, dst: u32) -> u32 {
    reg(op, lit1, src2, dst, true)
}
fn ctrl(op: u32, from: u32, to: u32) -> u32 {
    (op << 24) | (to.wrapping_sub(from) & 0x00ff_fffc)
}
fn cobr_lit(op: u32, lit1: u32, src2: u32, from: u32, to: u32) -> u32 {
    (op << 24) | (lit1 << 19) | (src2 << 14) | (1 << 13) | (to.wrapping_sub(from) & 0x1ffc)
}
/// MEMB absolute: returns the two words.
fn mem_abs(op: u32, r: u32, addr: u32) -> [u32; 2] {
    [(op << 24) | (r << 19) | (0xc << 10) | (1 << 12), addr]
}
/// MEMA `offset(abase)`.
fn mem_base(op: u32, r: u32, abase: u32, off: u32) -> u32 {
    (op << 24) | (r << 19) | (abase << 14) | (1 << 13) | off
}
fn mov_lit(v: u32, dst: u32) -> u32 {
    lit(0x5cc, v, 0, dst)
}
const RET: u32 = 0x0a00_0000;

fn setup(prog: &[u32]) -> (Cpu, Ram) {
    let mut ram = Ram::new();
    ram.load(0x1000, prog);
    let mut cpu = Cpu::new();
    cpu.ip = 0x1000;
    cpu.r[FP] = 0x8000;
    cpu.r[SP] = 0x8040;
    (cpu, ram)
}

fn run(cpu: &mut Cpu, ram: &mut Ram, steps: usize) {
    for _ in 0..steps {
        cpu.step(ram).unwrap();
    }
}

#[test]
fn counted_loop() {
    // g0 = 0; g1 = 10; loop: g0 += g1; g1 -= 1; if g1 != 0 goto loop
    let prog = [
        mov_lit(0, G0),
        mov_lit(10, G1),
        reg(0x590, G1, G0, G0, false),
        lit(0x592, 1, G1, G1),
        cobr_lit(0x35, 0, G1, 0x1010, 0x1008),
    ];
    let (mut cpu, mut ram) = setup(&prog);
    run(&mut cpu, &mut ram, 2 + 3 * 10);
    assert_eq!(cpu.r[G0 as usize], 55);
    assert_eq!(cpu.ip, 0x1014);
}

#[test]
fn call_preserves_caller_locals() {
    let prog = [
        mov_lit(7, R4),               // 0x1000
        ctrl(0x09, 0x1004, 0x1100),   // 0x1004 call
        reg(0x5cc, R4, 0, G1, false), // 0x1008 mov r4, g1
    ];
    let (mut cpu, mut ram) = setup(&prog);
    ram.load(0x1100, &[mov_lit(31, R4), reg(0x5cc, R4, 0, G0, false), RET]);
    run(&mut cpu, &mut ram, 6);
    assert_eq!(cpu.r[G0 as usize], 31, "callee result");
    assert_eq!(cpu.r[G1 as usize], 7, "caller r4 restored");
    assert_eq!(cpu.r[FP], 0x8000);
    assert_eq!(cpu.r[SP], 0x8040);
}

// The callee overwrites the caller's register save area (as sgt24h's boot
// does when it clears work RAM); the caller's locals come back from the
// on-chip cache regardless.
#[test]
fn return_restores_cached_frame_not_memory() {
    let prog = [mov_lit(7, R4), ctrl(0x09, 0x1004, 0x1100), reg(0x5cc, R4, 0, G1, false)];
    let (mut cpu, mut ram) = setup(&prog);
    let st = mem_abs(0x92, G0, 0x8010); // st g0 (= 0), caller's r4 slot
    ram.load(0x1100, &[st[0], st[1], RET]);
    run(&mut cpu, &mut ram, 5);
    assert_eq!(cpu.r[G1 as usize], 7);
    assert!(cpu.locals.sets().is_empty());
}

// A frame set up by hand on a 16-byte boundary (as doa's task switch
// does): the call's new frame is 64-byte aligned, and the return gives
// back the caller's FP from PFP's address bits, 4 and 5 included.
#[test]
fn return_keeps_pfp_address_bits_4_and_5() {
    let prog = [ctrl(0x09, 0x1000, 0x1100), mov_lit(1, G0)];
    let (mut cpu, mut ram) = setup(&prog);
    cpu.r[FP] = 0x80d0;
    cpu.r[SP] = 0x8110;
    ram.load(0x1100, &[RET]);
    run(&mut cpu, &mut ram, 1);
    assert_eq!(cpu.r[FP], 0x8140, "new frame on a 64-byte boundary");
    assert_eq!(cpu.r[PFP], 0x80d0);
    run(&mut cpu, &mut ram, 1);
    assert_eq!(cpu.r[FP], 0x80d0);
    assert_eq!(cpu.r[SP], 0x8110);
}

// Four nested calls overflow the three cached sets: the oldest (the
// outermost caller's) goes to memory and returns from there.
#[test]
fn fourth_nested_call_spills_oldest_frame() {
    let prog = [mov_lit(7, R4), ctrl(0x09, 0x1004, 0x1100), reg(0x5cc, R4, 0, G1, false)];
    let (mut cpu, mut ram) = setup(&prog);
    ram.load(0x1100, &[ctrl(0x09, 0x1100, 0x1200), RET]);
    ram.load(0x1200, &[ctrl(0x09, 0x1200, 0x1300), RET]);
    ram.load(0x1300, &[ctrl(0x09, 0x1300, 0x1400), RET]);
    let st = mem_abs(0x92, G0, 0x8010);
    ram.load(0x1400, &[st[0], st[1], RET]);
    run(&mut cpu, &mut ram, 5);
    assert_eq!(cpu.locals.sets().iter().map(|s| s.0).collect::<Vec<_>>(), [0x8040, 0x8080, 0x80c0]);
    run(&mut cpu, &mut ram, 6);
    assert_eq!(cpu.r[G1 as usize], 0, "spilled frame reloaded from memory");
    assert_eq!(cpu.r[FP], 0x8000);
}

// `flushreg` writes the cached sets to memory and empties the cache, so a
// change to a saved frame made after it takes effect.
#[test]
fn flushreg_spills_cached_frames() {
    let prog = [mov_lit(7, R4), ctrl(0x09, 0x1004, 0x1100), reg(0x5cc, R4, 0, G1, false)];
    let (mut cpu, mut ram) = setup(&prog);
    let st = mem_abs(0x92, G0, 0x8010);
    ram.load(0x1100, &[reg(0x66d, 0, 0, 0, false), st[0], st[1], RET]);
    run(&mut cpu, &mut ram, 3);
    assert!(cpu.locals.sets().is_empty());
    assert_eq!(ram.fetch(0x8010), Some(7), "flushed to the frame");
    run(&mut cpu, &mut ram, 3);
    assert_eq!(cpu.r[G1 as usize], 0);
}

#[test]
fn bal_links_through_g14() {
    let prog = [ctrl(0x0b, 0x1000, 0x1100), mov_lit(1, G0)];
    let (mut cpu, mut ram) = setup(&prog);
    // bx (g14)
    ram.load(0x1100, &[(0x84 << 24) | (30 << 14) | (0x4 << 10) | (1 << 12)]);
    run(&mut cpu, &mut ram, 3);
    assert_eq!(cpu.r[G0 as usize], 1);
}

#[test]
fn extended_multiply_divide() {
    let prog = [
        mov_lit(0, G0),
        reg(0x5cc, 0, 0, G0, false), // placeholder replaced below
    ];
    let (mut cpu, mut ram) = setup(&prog);
    cpu.r[G0 as usize] = 0x1234_5678;
    cpu.r[G1 as usize] = 0x9abc_def0;
    ram.load(0x1000, &[reg(0x670, G0, G1, G2, false), reg(0x671, G0, G2, G2, false)]);
    run(&mut cpu, &mut ram, 1);
    let prod = 0x1234_5678u64 * 0x9abc_def0u64;
    assert_eq!(cpu.r[G2 as usize], prod as u32);
    assert_eq!(cpu.r[G3 as usize], (prod >> 32) as u32);
    run(&mut cpu, &mut ram, 1);
    assert_eq!(cpu.r[G2 as usize], 0, "remainder");
    assert_eq!(cpu.r[G3 as usize], 0x9abc_def0, "quotient");
}

#[test]
fn single_precision_add() {
    let (mut cpu, mut ram) = setup(&[reg(0x78f, G0, G1, G2, false)]);
    cpu.r[G0 as usize] = 1.5f32.to_bits();
    cpu.r[G1 as usize] = 2.25f32.to_bits();
    run(&mut cpu, &mut ram, 1);
    assert_eq!(f32::from_bits(cpu.r[G2 as usize]), 3.75);
}

#[test]
fn byte_store_and_signed_load() {
    let st = mem_abs(0x82, G0, 0x4000);
    let ld = mem_abs(0xc0, G1, 0x4000);
    let (mut cpu, mut ram) = setup(&[mov_lit(0, G0), st[0], st[1], ld[0], ld[1]]);
    cpu.r[G0 as usize] = 0xffff_ff80;
    cpu.ip = 0x1004;
    run(&mut cpu, &mut ram, 2);
    assert_eq!(ram.0[0x4000], 0x80);
    assert_eq!(cpu.r[G1 as usize], 0xffff_ff80);
}

#[test]
fn shifts_and_bit_branches() {
    let prog = [
        lit(0x59b, 4, G0, G1),                 // shri 4, g0, g1
        lit(0x59a, 4, G0, G2),                 // shrdi 4, g0, g2
        cobr_lit(0x37, 3, G3, 0x1008, 0x1010), // bbs 3, g3 -> 0x1010
        mov_lit(1, G0),
        mov_lit(2, G0),
    ];
    let (mut cpu, mut ram) = setup(&prog);
    cpu.r[G0 as usize] = (-17i32) as u32;
    cpu.r[G3 as usize] = 0b1000;
    run(&mut cpu, &mut ram, 4);
    assert_eq!(cpu.r[G1 as usize] as i32, -2, "shri floors");
    assert_eq!(cpu.r[G2 as usize] as i32, -1, "shrdi truncates toward zero");
    assert_eq!(cpu.r[G0 as usize], 2, "bbs taken");
}

#[test]
fn iac_reinit_jumps_to_new_ip() {
    // synmovq g3 (= IAC port), g1 (= message)
    let (mut cpu, mut ram) = setup(&[reg(0x602, G3, G1, 0, false)]);
    let prcb = 0x6000;
    ram.load(0x5000, &[0x9300_0000, 0, prcb, 0x2000]);
    ram.write32(prcb + 24, 0x9000);
    cpu.r[G3 as usize] = IAC_PORT;
    cpu.r[G1 as usize] = 0x5000;
    run(&mut cpu, &mut ram, 1);
    assert_eq!(cpu.ip, 0x2000);
    assert_eq!(cpu.prcb, prcb);
    assert_eq!(cpu.r[FP], 0x9000);
}

#[test]
fn interrupt_enters_and_returns() {
    let (mut cpu, mut ram) = setup(&[mov_lit(1, G0), mov_lit(2, G1)]);
    let prcb = 0x6000;
    let table = 0x7000;
    cpu.prcb = prcb;
    ram.write32(prcb + 20, table);
    ram.write32(prcb + 24, 0xa000);
    let vector = 0x52u8; // priority 10
    ram.write32(table + 36 + (vector as u32 - 8) * 4, 0x3000);
    ram.load(0x3000, &[mov_lit(9, G2), RET]);
    // Running at priority 0, not interrupted.
    cpu.pc = PC_SUPERVISOR;
    cpu.ac = 0b010;

    cpu.step(&mut ram).unwrap(); // mov 1, g0
    cpu.post_interrupt(vector);
    cpu.step(&mut ram).unwrap(); // enters handler, runs mov 9, g2
    assert_eq!(cpu.priority(), 10);
    assert!(cpu.r[FP] >= 0xa000, "on interrupt stack");
    cpu.step(&mut ram).unwrap(); // ret
    assert_eq!(cpu.ip, 0x1004);
    assert_eq!(cpu.priority(), 0);
    assert_eq!(cpu.ac, 0b010);
    cpu.step(&mut ram).unwrap();
    assert_eq!((cpu.r[G0 as usize], cpu.r[G1 as usize], cpu.r[G2 as usize]), (1, 2, 9));
}

#[test]
fn masked_interrupt_waits() {
    let (mut cpu, mut ram) = setup(&[mov_lit(1, G0)]);
    cpu.pc = PC_SUPERVISOR | (20 << PC_PRIORITY_SHIFT);
    cpu.post_interrupt(0x52); // priority 10 < 20
    cpu.step(&mut ram).unwrap();
    assert_eq!(cpu.ip, 0x1004);
    assert!(cpu.has_pending());
}

#[test]
fn ext80_roundtrip() {
    for v in [1.0, -2.5, 1e-10, 123456.789] {
        let (a, b, c) = f64_to_ext80(v);
        assert_eq!(ext80_to_f64(a, b, c), v);
    }
}

#[test]
fn mem_base_load_quad() {
    let (mut cpu, mut ram) = setup(&[mem_base(0xb0, G0, G1, 8)]);
    ram.load(0x4008, &[1, 2, 3, 4]);
    cpu.r[G1 as usize] = 0x4000;
    run(&mut cpu, &mut ram, 1);
    assert_eq!(&cpu.r[16..20], &[1, 2, 3, 4]);
}

#[test]
fn icr_routes_external_lines() {
    // synmov g3 (= ICR port), g1 (= value address)
    let (mut cpu, mut ram) = setup(&[reg(0x600, G3, G1, 0, false)]);
    ram.write32(0x5000, 0x0000_5a00); // IRQ1 -> vector 0x5a, others off
    cpu.r[G3 as usize] = ICR_PORT;
    cpu.r[G1 as usize] = 0x5000;
    run(&mut cpu, &mut ram, 1);
    cpu.pc = PC_INTERRUPTED | PC_SUPERVISOR | (31 << PC_PRIORITY_SHIFT);
    cpu.raise_line(0);
    assert!(!cpu.has_pending(), "line 0 unprogrammed");
    cpu.raise_line(1);
    assert_eq!(cpu.highest_pending(), Some(0x5a));
}
