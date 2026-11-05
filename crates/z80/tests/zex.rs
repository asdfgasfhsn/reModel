//! Runs the core against Frank Cringle's Z80 instruction exercisers, zexdoc
//! (documented flags only) and zexall (all flags), as CP/M programs.
//!
//! `make z80-test` runs this with `Z80_TESTS` (default `out/z80-tests`)
//! holding `zexdoc.com` and `zexall.com`; a missing file is skipped. The
//! programs aren't part of the repository: see `docs/hw/z80.md` for where to
//! get them. Each takes a few billion T-states, so run it in release.
//!
//! The CP/M environment is the least the exercisers need: the program at
//! 0x100, a RET at the BDOS entry (5) with the top of the stack in the word
//! at 6, BDOS functions 2 (print E) and 9 (print the `$`-terminated string
//! at DE) handled when PC reaches 5, and a jump to 0 ending the run.

use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;
use z80::{Bus, Cpu};

struct Mem(Box<[u8; 0x1_0000]>);

impl Bus for Mem {
    fn read(&mut self, a: u16) -> u8 {
        self.0[a as usize]
    }
    fn write(&mut self, a: u16, v: u8) {
        self.0[a as usize] = v;
    }
    fn input(&mut self, _port: u16) -> u8 {
        0xff
    }
    fn output(&mut self, _port: u16, _v: u8) {}
}

/// Run one CP/M program; returns its console output.
fn run(name: &str, prog: &[u8]) -> String {
    let mut mem = Mem(Box::new([0; 0x1_0000]));
    mem.0[0x100..0x100 + prog.len()].copy_from_slice(prog);
    mem.0[5] = 0xc9; // RET
    mem.0[6] = 0x00;
    mem.0[7] = 0xf0; // stack top 0xf000
    let mut cpu = Cpu::new();
    cpu.pc = 0x100;
    cpu.sp = 0xf000;
    let mut out = String::new();
    let mut line = String::new();
    let (mut insns, mut tstates) = (0u64, 0u64);
    let start = Instant::now();
    loop {
        match cpu.pc {
            0 => break,
            5 => {
                let mut put = |ch: u8| {
                    let ch = ch as char;
                    out.push(ch);
                    line.push(ch);
                    if ch == '\n' {
                        print!("{name}: {line}");
                        std::io::stdout().flush().ok();
                        line.clear();
                    }
                };
                match cpu.bc as u8 {
                    2 => put(cpu.de as u8),
                    9 => {
                        let mut a = cpu.de;
                        while mem.0[a as usize] != b'$' {
                            put(mem.0[a as usize]);
                            a = a.wrapping_add(1);
                        }
                    }
                    f => panic!("{name}: BDOS function {f} not supported"),
                }
            }
            _ => {}
        }
        tstates += cpu.step(&mut mem) as u64;
        insns += 1;
    }
    let secs = start.elapsed().as_secs_f64();
    println!(
        "{name}: {insns} instructions, {tstates} T-states in {secs:.1} s: {:.0} M instructions/s, {:.0} MHz equivalent",
        insns as f64 / secs / 1e6,
        tstates as f64 / secs / 1e6
    );
    out
}

fn exerciser(file: &str) {
    let dir = PathBuf::from(std::env::var("Z80_TESTS").unwrap_or_else(|_| "out/z80-tests".into()));
    let path = dir.join(file);
    let Ok(prog) = std::fs::read(&path) else {
        println!("{}: not found, skipped (see docs/hw/z80.md)", path.display());
        return;
    };
    let out = run(file, &prog);
    let errors = out.matches("ERROR").count();
    assert!(out.contains("Tests complete"), "{file}: did not finish");
    assert_eq!(errors, 0, "{file}: {errors} tests failed");
}

#[test]
#[ignore = "needs zexdoc.com in Z80_TESTS; run with make z80-test"]
fn zexdoc() {
    exerciser("zexdoc.com");
}

#[test]
#[ignore = "needs zexall.com in Z80_TESTS; run with make z80-test"]
fn zexall() {
    exerciser("zexall.com");
}
