use super::*;
use i960::cpu::Bus;

/// 64 KB of memory: the first 16 KB ROM, the rest RAM with write counters.
struct Flat {
    mem: Vec<u8>,
    gens: Vec<u32>,
    retired: u64,
}

const ROM_END: u32 = 0x4000;

impl Flat {
    fn new(rom: &[u32], ram: &[(u32, u32)]) -> Self {
        let mut f = Flat { mem: vec![0; 0x1_0000], gens: vec![0; 0x1_0000 >> GEN_SHIFT], retired: 0 };
        for (i, &w) in rom.iter().enumerate() {
            f.mem[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes());
        }
        for &(a, w) in ram {
            f.write32(a, w);
        }
        f
    }
}

impl Bus for Flat {
    fn read8(&mut self, a: u32) -> u8 {
        self.mem[a as usize & 0xffff]
    }
    fn read16(&mut self, a: u32) -> u16 {
        self.read8(a) as u16 | (self.read8(a + 1) as u16) << 8
    }
    fn read32(&mut self, a: u32) -> u32 {
        self.read16(a) as u32 | (self.read16(a + 2) as u32) << 16
    }
    fn write8(&mut self, a: u32, v: u8) {
        if a & 0xffff >= ROM_END {
            self.gens[(a as usize & 0xffff) >> GEN_SHIFT] += 1;
            self.mem[a as usize & 0xffff] = v;
        }
    }
    fn write16(&mut self, a: u32, v: u16) {
        self.write8(a, v as u8);
        self.write8(a + 1, (v >> 8) as u8);
    }
    fn write32(&mut self, a: u32, v: u32) {
        self.write16(a, v as u16);
        self.write16(a + 2, (v >> 16) as u16);
    }
    fn fetch(&self, a: u32) -> Option<u32> {
        let a = a as usize;
        (a + 4 <= self.mem.len()).then(|| u32::from_le_bytes(self.mem[a..a + 4].try_into().unwrap()))
    }
    fn is_rom(&self, a: u32) -> bool {
        a < ROM_END
    }
}

impl Host for Flat {
    fn retire(&mut self, _cpu: &mut Cpu) -> bool {
        self.retired += 1;
        true
    }
}

impl JitHost for Flat {
    fn code_generations(&self) -> &[u32] {
        &self.gens
    }
    fn code_generation_index(&self, a: u32) -> Option<usize> {
        ((ROM_END..0x1_0000).contains(&a)).then_some((a >> GEN_SHIFT) as usize)
    }
}

fn cpu_at(ip: u32) -> Cpu {
    let mut c = Cpu::new();
    c.ip = ip;
    c.r[1] = 0x8000; // sp
    c.r[31] = 0x7fc0; // fp
    c
}

/// Run at least `steps` instructions as JIT blocks (translated on first
/// visit) from `ip`, then as many on the interpreter, and compare.
fn same_as_interpreter(rom: &[u32], ram: &[(u32, u32)], ip: u32, steps: u64) -> (Cpu, Flat, Stats) {
    let (mut b, mut hb) = (cpu_at(ip), Flat::new(rom, ram));
    let mut jit = Jit::<Flat>::new(Config { hot: 0, ..Config::default() }).unwrap();
    while b.icount < steps {
        if jit.run(&mut b, &mut hb).is_none() {
            b.step_insn(&mut hb).unwrap();
            hb.retired += 1;
        }
    }
    let (mut a, mut ha) = (cpu_at(ip), Flat::new(rom, ram));
    while a.icount < b.icount {
        a.step(&mut ha).unwrap();
        ha.retired += 1;
    }
    assert_eq!(a.arch_state(), b.arch_state(), "registers differ");
    assert_eq!(ha.retired, hb.retired, "a retire per instruction");
    assert!(ha.mem == hb.mem, "memory differs");
    (b, hb, jit.stats())
}

// Encoders for the formats (80960KB programmer's reference).
fn reg(full: u32, src1: u32, lit1: bool, src2: u32, dst: u32) -> u32 {
    (full >> 4) << 24 | dst << 19 | src2 << 14 | (lit1 as u32) << 11 | (full & 0xf) << 7 | src1
}
fn cobr(op: u32, src1: u32, lit1: bool, src2: u32, from: u32, to: u32) -> u32 {
    op << 24 | src1 << 19 | src2 << 14 | (lit1 as u32) << 13 | (to.wrapping_sub(from) & 0x1ffc)
}
fn ctrl(op: u32, from: u32, to: u32) -> u32 {
    op << 24 | (to.wrapping_sub(from) & 0xff_fffc)
}
/// MEMA with a base register and a 12-bit offset.
fn mem(op: u32, reg: u32, abase: u32, off: u32) -> u32 {
    op << 24 | reg << 19 | abase << 14 | 0x2000 | off
}
const G: u32 = 16;

fn assemble(code: &[(u32, u32)]) -> Vec<u32> {
    let mut w = vec![0u32; 0x40];
    for &(a, v) in code {
        w[a as usize / 4] = v;
    }
    w
}

#[test]
fn loop_with_loads_stores_and_calls() {
    let w = assemble(&[
        (0x00, reg(0x5cc, 0x10, true, 0, G)),           // mov 0x10, g0
        (0x04, reg(0x5cc, 5, true, 0, G + 1)),          // mov 5, g1
        (0x08, 0x8c00_3000 | (G + 2) << 19),            // lda 0x5000, g2
        (0x0c, 0x5000),                                 //
        (0x10, reg(0x590, G + 1, false, G, G)),         // addo g1, g0, g0
        (0x14, mem(0x92, G, G + 2, 0)),                 // st g0, (g2)
        (0x18, mem(0x90, G + 3, G + 2, 0)),             // ld (g2), g3
        (0x1c, reg(0x592, 1, true, G + 1, G + 1)),      // subo 1, g1, g1
        (0x20, cobr(0x35, 0, true, G + 1, 0x20, 0x10)), // cmpobne 0, g1, 0x10
        (0x24, ctrl(0x09, 0x24, 0x40)),                 // call 0x40
        (0x28, ctrl(0x08, 0x28, 0x28)),                 // b 0x28
        (0x40, reg(0x59c, 2, true, G, G + 4)),          // shlo 2, g0, g4
        (0x44, mem(0x82, G + 4, G + 2, 1)),             // stob g4, 1(g2)
        (0x48, mem(0xc0, G + 5, G + 2, 1)),             // ldib 1(g2), g5
        (0x4c, reg(0x5a1, G + 5, false, G + 4, 0)),     // cmpi g5, g4
        (0x50, reg(0x5ae, 3, true, G + 4, 0)),          // chkbit 3, g4
        (0x54, 0x0a00_0000),                            // ret
    ]);
    let fetch = |a: u32| w.get(a as usize / 4).copied();
    let listing: Vec<String> = (0..0x58).step_by(4).map(|a| format!("{a:x}: {}", decode(a, fetch))).collect();
    let listing = listing.join("\n");
    for want in
        ["mov", "lda", "addo", "st ", "ld ", "subo", "cmpobne", "call", "shlo", "stob", "ldib", "cmpi", "chkbit"]
    {
        assert!(listing.contains(want), "{want} missing from\n{listing}");
    }
    let (c, _, stats) = same_as_interpreter(&w, &[], 0, 200);
    assert_eq!(c.r[G as usize], 0x10 + 5 + 4 + 3 + 2 + 1, "{listing}");
    assert!(stats.blocks >= 3 && stats.fallbacks == 0, "{stats:?}");
}

#[test]
fn code_in_ram_is_retranslated_when_written() {
    // ROM: call 0x4000; st g6, (g7) (a new first instruction for it);
    // call 0x4000; spin. RAM: mov 1, g0; ret.
    let w = assemble(&[
        (0x0, ctrl(0x09, 0x0, 0x4000)),
        (0x4, mem(0x92, G + 6, G + 7, 0)),
        (0x8, ctrl(0x09, 0x8, 0x4000)),
        (0xc, ctrl(0x08, 0xc, 0xc)),
    ]);
    let ram = [(0x4000, reg(0x5cc, 1, true, 0, G)), (0x4004, 0x0a00_0000)];
    let start = || {
        let (mut c, h) = (cpu_at(0), Flat::new(&w, &ram));
        c.r[G as usize + 6] = reg(0x5cc, 2, true, 0, G);
        c.r[G as usize + 7] = 0x4000;
        (c, h)
    };
    let mut jit = Jit::<Flat>::new(Config { hot: 0, ..Config::default() }).unwrap();
    let (mut b, mut hb) = start();
    while b.icount < 20 {
        if jit.run(&mut b, &mut hb).is_none() {
            b.step_insn(&mut hb).unwrap();
        }
    }
    let (mut a, mut ha) = start();
    while a.icount < b.icount {
        a.step(&mut ha).unwrap();
    }
    assert_eq!(a.arch_state(), b.arch_state());
    assert_eq!(b.r[G as usize], 2, "the second call ran the new code");
    let stats = jit.stats();
    assert!(stats.ram_blocks >= 2 && stats.dropped >= 1, "{stats:?}");
}

#[test]
fn a_block_that_overwrites_its_own_code_leaves_after_the_store() {
    // ROM: call 0x4000; spin. RAM: st g6, (g7) (g7 = 0x4008, two
    // instructions on, in the same block); mov 0, g1; mov 1, g0 (becomes
    // mov 2, g0); ret.
    let w = assemble(&[(0x0, ctrl(0x09, 0x0, 0x4000)), (0x4, ctrl(0x08, 0x4, 0x4))]);
    let ram = [
        (0x4000, mem(0x92, G + 6, G + 7, 0)),
        (0x4004, reg(0x5cc, 0, true, 0, G + 1)),
        (0x4008, reg(0x5cc, 1, true, 0, G)),
        (0x400c, 0x0a00_0000),
    ];
    let start = || {
        let (mut c, h) = (cpu_at(0), Flat::new(&w, &ram));
        c.r[G as usize + 6] = reg(0x5cc, 2, true, 0, G);
        c.r[G as usize + 7] = 0x4008;
        (c, h)
    };
    let mut jit = Jit::<Flat>::new(Config { hot: 0, ..Config::default() }).unwrap();
    let (mut b, mut hb) = start();
    while b.icount < 12 {
        if jit.run(&mut b, &mut hb).is_none() {
            b.step_insn(&mut hb).unwrap();
        }
    }
    let (mut a, mut ha) = start();
    while a.icount < b.icount {
        a.step(&mut ha).unwrap();
    }
    assert_eq!(a.arch_state(), b.arch_state());
    assert_eq!(b.r[G as usize], 2, "the store's own block ran the new instruction");
    assert!(jit.stats().ram_blocks >= 2, "{:?}", jit.stats());
}
