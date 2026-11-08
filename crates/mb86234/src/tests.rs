//! The interpreter on small hand-assembled programs (no game code).

use super::*;

const ALU: u32 = 21;

fn alu(code: u32, w: u32) -> u32 {
    w | (code << ALU)
}
fn mov(dst: u32, src: u32) -> u32 {
    0x1c1f_0000 | (dst << 9) | src
}
fn store(r: u32, c9: u32) -> u32 {
    0x1c1c_0000 | (r << 9) | c9
}
fn load(r: u32, c9: u32) -> u32 {
    0x1c1d_0000 | ((0x40 | r) << 9) | c9
}
const NOP: u32 = 0x1c1f_1e10;
const RET: u32 = 0xbf6a_0000;
fn jmp(t: u32) -> u32 {
    0xbf60_0000 | t
}
const IN: u32 = reg::FIFO_IN as u32;
const OUT: u32 = reg::FIFO_OUT as u32;
const R10: u32 = reg::R10 as u32;
const R13: u32 = reg::R13 as u32;
const ACC: u32 = reg::ACC as u32;
const P: u32 = reg::P as u32;

fn run(cpu: &mut Cpu, input: &[f32]) -> Vec<f32> {
    cpu.fifo_in.extend(input.iter().map(|x| x.to_bits()));
    assert_eq!(cpu.run(&mut NoBus, 10_000), Stop::Input);
    cpu.fifo_out.drain(..).map(f32::from_bits).collect()
}

#[test]
fn fifo_and_float_alu() {
    // r10 = in; acc = in; acc op= r10; out = acc; loop.
    let prog = |code| vec![mov(R10, IN), mov(ACC, IN), alu(code, NOP), mov(OUT, ACC), jmp(0)];
    for (code, want) in [(6, 3.75), (7, 0.75), (16, 1.5)] {
        let mut c = Cpu::new(prog(code), Vec::new());
        assert_eq!(run(&mut c, &[1.5, 2.25]), [want], "code {code}");
    }
    // r10 = in, r13 = in, p = r10*r13, out = p.
    let mut c = Cpu::new(vec![mov(R10, IN), mov(R13, IN), alu(8, NOP), mov(OUT, P), jmp(0)], Vec::new());
    assert_eq!(run(&mut c, &[1.5, 2.25, 2.0, -3.0]), [3.375, -6.0]);
}

#[test]
fn multiply_accumulate_pipeline() {
    // x*a + y*b + z*c as the program's dot products do it: each word's
    // multiply uses the inputs the word before loaded.
    let mut p = Vec::new();
    for i in 0..6 {
        p.push(store(IN, i)); // RAM0[0..6] = x, y, z, a, b, c
    }
    let ld = |a: u32, b: u32| 0x000c_0000 | (a << 9) | b; // r13 = RAM1[a], r10 = RAM0[b]
    for i in 3..6 {
        p.push(0x1c0c_0000 | (i << 9) | i); // RAM1[i] = RAM0[i]
    }
    p.extend([ld(3, 0), alu(8, ld(4, 1)), alu(13, ld(5, 2)), alu(9, NOP), alu(12, NOP), mov(OUT, ACC), jmp(0)]);
    let mut c = Cpu::new(p, Vec::new());
    assert_eq!(run(&mut c, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]), [32.0]);
}

#[test]
fn accumulator_written_twice() {
    // acc = 1.0 + 2.0 by the ALU while the transfer loads RAM0[0] (5.0):
    // a transfer then sees 3.0, the next ALU operation 5.0.
    let mut c = Cpu::new(
        vec![
            store(IN, 0),
            mov(R10, IN),
            mov(ACC, IN),
            alu(6, load(ACC, 0)),
            mov(OUT, ACC),
            alu(6, NOP),
            mov(OUT, ACC),
            jmp(0),
        ],
        Vec::new(),
    );
    assert_eq!(run(&mut c, &[5.0, 2.0, 1.0]), [3.0, 7.0]);
}

#[test]
fn integers_branches_and_calls() {
    // Count down from the input to 1, a called routine outputting each.
    let p = vec![
        mov(ACC, IN),  // 0
        0x3900_0001,   // 1: r10 = 1
        0xbf64_0007,   // 2: call 7
        alu(27, NOP),  // 3: acc -= r10
        0xbe00_0006,   // 4: jz 6
        jmp(2),        // 5
        jmp(9),        // 6
        mov(OUT, ACC), // 7
        RET,           // 8
        mov(ACC, IN),  // 9: wait
    ];
    let mut c = Cpu::new(p, Vec::new());
    c.fifo_in.push_back(3);
    assert_eq!(c.run(&mut NoBus, 1000), Stop::Input);
    assert_eq!(c.fifo_out.drain(..).collect::<Vec<_>>(), [3, 2, 1]);
    assert_eq!((c.pc, c.stack.len()), (9, 0));
    assert!(c.unknown.is_empty());
}

#[test]
fn repeat_indirect_and_views() {
    // ar3 = 0x205 (RAM1[5]); repeat 3: [ar3]+1 = in; then read them back
    // through ar3 = 0x205 with step -1 after setting ar3 to 0x207.
    let p = vec![
        0x4300_0205,
        0x3c04_0003,
        store(IN, 0x181),
        0x4300_0207,
        0x3c04_0003,
        load(OUT, 0x19f),
        // acc.e = 0x80, acc.m = sign | 0x400000: -3.0
        0x5a00_0080,
        0x5bc0_0000,
        mov(OUT, ACC),
        mov(ACC, IN),
    ];
    let mut c = Cpu::new(p, Vec::new());
    for v in [7, 8, 9] {
        c.fifo_in.push_back(v);
    }
    assert_eq!(c.run(&mut NoBus, 100), Stop::Input);
    let out: Vec<u32> = c.fifo_out.drain(..).collect();
    assert_eq!(out, [9, 8, 7, (-3.0f32).to_bits()]);
    assert_eq!(c.ram[0x205..0x208], [7, 8, 9]);
}

/// A ROM in the layout the function unit reads, computed here.
fn rom() -> Vec<u32> {
    let mut r = vec![0u32; 0x10000];
    for (i, w) in r[..0x4000].iter_mut().enumerate() {
        *w = ((i as f64 * std::f64::consts::TAU / 65536.0).sin() as f32).to_bits();
    }
    for k in 0..8192 {
        let m0 = 1.0 + (k as f64 + 0.5) / 8192.0;
        r[0x8000 + 2 * k] = ((-1.0 / (m0 * m0)) as f32).to_bits();
        r[0x8001 + 2 * k] = ((2.0 / m0) as f32).to_bits();
    }
    for k in 0..8192 {
        let s = if k < 4096 { 1.0 } else { 2f64.sqrt() };
        let m0 = 1.0 + ((k % 4096) as f64 + 0.5) / 4096.0;
        r[0xc000 + 2 * k] = ((-0.5 * s * m0.powf(-1.5)) as f32).to_bits();
        r[0xc001 + 2 * k] = ((1.5 * s / m0.sqrt()) as f32).to_bits();
    }
    r
}

#[test]
fn function_unit() {
    let mut u = Funcs::new(rom());
    let get = |u: &Funcs, a| f32::from_bits(u.read(a).unwrap());
    for (angle, s, c) in [(0, 0.0, 1.0), (0x4000, 1.0, 0.0), (0x8000, 0.0, -1.0), (0xc000, -1.0, 0.0)] {
        u.write(0x20, angle);
        assert_eq!((get(&u, 0x20), get(&u, 0x21)), (s, c), "angle {angle:#x}");
    }
    u.write(0x20, 0x2000);
    assert!((get(&u, 0x20) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-7);
    // 1/x and 1/sqrt(x) as the program computes them.
    for x in [1.0f32, 3.0, -7.5, 0.001, 12345.0] {
        u.write(0x28, x.to_bits());
        let m = f32::from_bits((x.to_bits() & 0x807f_ffff) | (127 << 23));
        let r = get(&u, 0x29) - m * get(&u, 0x28).abs();
        assert!((r * x - 1.0).abs() < 1e-6, "1/{x} = {r}");
        if x > 0.0 {
            u.write(0x2a, x.to_bits());
            let s = get(&u, 0x2b) - m * get(&u, 0x2a);
            assert!((s * x.sqrt() - 1.0).abs() < 1e-6, "1/sqrt {x} = {s}");
        }
    }
    assert_eq!(u.read(0x24), None);
}

#[test]
fn shared_bus_and_ports() {
    struct Ram(Vec<u32>);
    impl Bus for Ram {
        fn read(&mut self, a: u32) -> u32 {
            self.0[a as usize]
        }
        fn write(&mut self, a: u32, v: u32) {
            self.0[a as usize] = v;
        }
    }
    // Store the input to the function unit's angle port, read its sine;
    // then with the shared RAM selected, store it at ext[3].
    let p = vec![
        store(0x40 | IN, 0x20),
        0x1c1e_0000 | (ACC << 9) | 0x20,
        0x6340_0000,
        0x4300_0003,
        store(0x40 | ACC, 0x1a0),
        0x6300_0000,
        mov(ACC, IN),
    ];
    let mut c = Cpu::new(p, rom());
    c.fifo_in.push_back(0x4000);
    let mut ram = Ram(vec![0; 8]);
    assert_eq!(c.run(&mut ram, 100), Stop::Input);
    assert_eq!(ram.0[3], 1.0f32.to_bits());
}

#[test]
fn state_round_trips() {
    let mut c = Cpu::new(vec![store(IN, 0x10), mov(ACC, IN), jmp(0)], Vec::new());
    c.fifo_in.extend([1, 2, 3]);
    c.run(&mut NoBus, 2);
    let bytes = state::to_bytes(&c);
    let mut d = Cpu::new(c.program.clone(), Vec::new());
    state::State::load(&mut d, &mut state::Reader::new(&bytes)).unwrap();
    assert_eq!((d.pc, d.ram[0x10], d.regs[reg::ACC as usize], d.fifo_in.clone()), (2, 1, 2, [3].into()));
    assert_eq!(state::to_bytes(&d), bytes);
}

#[test]
fn loop_counts_down_and_keeps_flags() {
    // lc = in; acc = #0; r10 = #1; a body adding 1 (integer) closed by
    // `loop`; a `jz` that must fall through; out = acc. The body runs lc
    // times, and the loop word leaves the add's flags (not zero) alone.
    // (`srallyc`'s course search loads lc with a cell's polygon count and
    // relies on both.)
    let lc = reg::LC as u32;
    let p = vec![mov(lc, IN), 0x3b00_0000, 0x3900_0001, alu(26, NOP), 0xbf00_0003, 0xbe00_0007, mov(OUT, ACC), jmp(0)];
    let mut c = Cpu::new(p, Vec::new());
    c.fifo_in.extend([5, 1]);
    assert_eq!(c.run(&mut NoBus, 1000), Stop::Input);
    assert_eq!(c.fifo_out.drain(..).collect::<Vec<_>>(), [5, 1]);
    assert_eq!(c.regs[reg::LC as usize], 0);
}
