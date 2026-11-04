//! Runs the core against the public SingleStepTests 68000 suite
//! (<https://github.com/SingleStepTests/680x0>, `68000/v1`).
//!
//! `make m68k-tests` downloads and decompresses the suite into
//! `out/m68k-tests/json` and runs this. Environment:
//! - `M68K_TESTS`: directory holding `json/` (default `out/m68k-tests`);
//! - `M68K_FILTER`: only run files whose name contains this;
//! - `M68K_VERBOSE=n`: print the first `n` failures of each file.
//!
//! The suite models the two-word prefetch queue; this core does not, so the
//! queued words are placed in memory at PC and PC+2 for the run (and the
//! original bytes restored afterwards unless the instruction wrote them).
//! Registers, SR, PC and RAM are compared; cycle counts and the final
//! prefetch are not. Cases whose expected bus activity fetches the
//! address-error vector are counted separately.

use m68k::{Bus, Cpu};
use std::collections::HashMap;
use std::path::PathBuf;

// ---- a minimal JSON reader ----

enum J {
    Num(i64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
    Lit,
}

impl J {
    fn get(&self, k: &str) -> &J {
        match self {
            J::Obj(v) => v.iter().find(|(n, _)| n == k).map(|(_, j)| j).unwrap_or_else(|| panic!("no key {k}")),
            _ => panic!("not an object"),
        }
    }
    fn num(&self) -> i64 {
        match self {
            J::Num(n) => *n,
            _ => panic!("not a number"),
        }
    }
    fn u32(&self) -> u32 {
        self.num() as u32
    }
    fn arr(&self) -> &[J] {
        match self {
            J::Arr(v) => v,
            _ => panic!("not an array"),
        }
    }
    fn str(&self) -> &str {
        match self {
            J::Str(s) => s,
            _ => panic!("not a string"),
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) {
        self.ws();
        assert_eq!(self.s[self.i], c, "expected '{}' at {}", c as char, self.i);
        self.i += 1;
    }
    fn value(&mut self) -> J {
        self.ws();
        match self.s[self.i] {
            b'{' => {
                self.i += 1;
                let mut v = Vec::new();
                loop {
                    self.ws();
                    if self.s[self.i] == b'}' {
                        self.i += 1;
                        break;
                    }
                    let k = match self.value() {
                        J::Str(s) => s,
                        _ => panic!("object key"),
                    };
                    self.eat(b':');
                    v.push((k, self.value()));
                    self.ws();
                    if self.s[self.i] == b',' {
                        self.i += 1;
                    }
                }
                J::Obj(v)
            }
            b'[' => {
                self.i += 1;
                let mut v = Vec::new();
                loop {
                    self.ws();
                    if self.s[self.i] == b']' {
                        self.i += 1;
                        break;
                    }
                    v.push(self.value());
                    self.ws();
                    if self.s[self.i] == b',' {
                        self.i += 1;
                    }
                }
                J::Arr(v)
            }
            b'"' => {
                self.i += 1;
                let start = self.i;
                while self.s[self.i] != b'"' {
                    self.i += if self.s[self.i] == b'\\' { 2 } else { 1 };
                }
                let s = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
                self.i += 1;
                J::Str(s)
            }
            b'-' | b'0'..=b'9' => {
                let start = self.i;
                self.i += 1;
                while self.i < self.s.len() && matches!(self.s[self.i], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
                {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.s[start..self.i]).unwrap();
                J::Num(t.parse::<i64>().unwrap_or_else(|_| t.parse::<f64>().unwrap() as i64))
            }
            _ => {
                while self.i < self.s.len() && self.s[self.i].is_ascii_alphabetic() {
                    self.i += 1;
                }
                J::Lit
            }
        }
    }
}

// ---- test memory ----

struct Mem {
    ram: Vec<u8>,
    touched: Vec<u32>,
    written: HashMap<u32, ()>,
}

impl Mem {
    fn set(&mut self, a: u32, v: u8) {
        let a = a & 0xff_ffff;
        self.ram[a as usize] = v;
        self.touched.push(a);
    }
    fn clear(&mut self) {
        for &a in &self.touched {
            self.ram[a as usize] = 0;
        }
        self.touched.clear();
        self.written.clear();
    }
}

impl Bus for Mem {
    fn read8(&mut self, a: u32) -> u8 {
        self.ram[(a & 0xff_ffff) as usize]
    }
    fn read16(&mut self, a: u32) -> u16 {
        (self.read8(a) as u16) << 8 | self.read8(a.wrapping_add(1)) as u16
    }
    fn write8(&mut self, a: u32, v: u8) {
        let a = a & 0xff_ffff;
        self.written.insert(a, ());
        self.set(a, v);
    }
    fn write16(&mut self, a: u32, v: u16) {
        self.write8(a, (v >> 8) as u8);
        self.write8(a.wrapping_add(1), v as u8);
    }
}

const REGS: [&str; 15] = ["d0", "d1", "d2", "d3", "d4", "d5", "d6", "d7", "a0", "a1", "a2", "a3", "a4", "a5", "a6"];

fn load(cpu: &mut Cpu, mem: &mut Mem, st: &J) {
    for (i, r) in REGS.iter().enumerate() {
        let v = st.get(r).u32();
        if i < 8 {
            cpu.d[i] = v;
        } else {
            cpu.a[i - 8] = v;
        }
    }
    cpu.set_sr(0x2700);
    cpu.set_ssp(st.get("ssp").u32());
    cpu.set_usp(st.get("usp").u32());
    cpu.set_sr(st.get("sr").u32() as u16);
    cpu.pc = st.get("pc").u32();
    for e in st.get("ram").arr() {
        let e = e.arr();
        mem.set(e[0].u32(), e[1].u32() as u8);
    }
}

/// Run one case; `Err` describes the first mismatches.
fn run_case(cpu: &mut Cpu, mem: &mut Mem, case: &J) -> Result<(), String> {
    mem.clear();
    let init = case.get("initial");
    load(cpu, mem, init);
    let pc = init.get("pc").u32();
    let pf = init.get("prefetch").arr();
    let saved: Vec<(u32, u8)> =
        (0..4).map(|i| pc.wrapping_add(i) & 0xff_ffff).map(|a| (a, mem.ram[a as usize])).collect();
    for (i, w) in pf.iter().enumerate() {
        let a = pc.wrapping_add(2 * i as u32);
        let w = w.u32();
        mem.set(a, (w >> 8) as u8);
        mem.set(a.wrapping_add(1), w as u8);
    }
    cpu.step(mem);
    for (a, v) in saved {
        if !mem.written.contains_key(&a) {
            mem.ram[a as usize] = v;
        }
    }

    let fin = case.get("final");
    let mut errs = Vec::new();
    for (i, r) in REGS.iter().enumerate() {
        let want = fin.get(r).u32();
        let got = if i < 8 { cpu.d[i] } else { cpu.a[i - 8] };
        if got != want {
            errs.push(format!("{r} {got:08x} want {want:08x}"));
        }
    }
    let checks = [("usp", cpu.usp(), fin.get("usp").u32()), ("ssp", cpu.ssp(), fin.get("ssp").u32())];
    for (n, got, want) in checks {
        if got != want {
            errs.push(format!("{n} {got:08x} want {want:08x}"));
        }
    }
    let (sr, want_sr) = (cpu.sr() as u32, fin.get("sr").u32());
    if sr != want_sr {
        errs.push(format!("sr {sr:04x} want {want_sr:04x}"));
    }
    let want_pc = fin.get("pc").u32();
    if cpu.pc != want_pc {
        errs.push(format!("pc {:06x} want {want_pc:06x}", cpu.pc));
    }
    for e in fin.get("ram").arr() {
        let e = e.arr();
        let (a, want) = (e[0].u32() & 0xff_ffff, e[1].u32() as u8);
        let got = mem.ram[a as usize];
        if got != want {
            errs.push(format!("[{a:06x}] {got:02x} want {want:02x}"));
        }
    }
    if errs.is_empty() {
        Ok(())
    } else {
        errs.truncate(8);
        Err(errs.join(", "))
    }
}

/// True if the expected bus activity reads the address-error vector.
fn is_address_error(case: &J) -> bool {
    case.get("transactions").arr().iter().any(|t| {
        let t = t.arr();
        t.len() >= 4 && t[0].str() == "r" && t[3].u32() == 12
    })
}

#[test]
#[ignore = "needs the SingleStepTests suite: make m68k-tests"]
fn singlestep() {
    let root = std::env::var("M68K_TESTS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../out/m68k-tests"));
    let dir = root.join("json");
    let filter = std::env::var("M68K_FILTER").unwrap_or_default();
    let verbose: usize = std::env::var("M68K_VERBOSE").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e} (run `make m68k-tests`)", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter(|p| p.file_name().unwrap().to_string_lossy().contains(&filter))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no test files in {}", dir.display());

    let mut cpu = Cpu::new();
    let mut mem = Mem { ram: vec![0; 1 << 24], touched: Vec::new(), written: HashMap::new() };
    let (mut total, mut pass, mut ae_total, mut ae_pass) = (0, 0, 0, 0);
    let mut below = Vec::new();
    println!("{:<16} {:>6} {:>6} {:>8} {:>10}", "file", "cases", "fail", "pass%", "addr-err");
    for path in &files {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read(path).unwrap();
        let cases = Parser { s: &text, i: 0 }.value();
        let (mut n, mut ok, mut an, mut aok, mut shown, mut ae_shown) = (0, 0, 0, 0, 0, 0);
        for case in cases.arr() {
            let ae = is_address_error(case);
            let r = run_case(&mut cpu, &mut mem, case);
            if ae {
                an += 1;
                match r {
                    Ok(()) => aok += 1,
                    Err(e) if ae_shown < verbose => {
                        ae_shown += 1;
                        println!("  FAIL (address error) {}: {e}", case.get("name").str());
                    }
                    Err(_) => {}
                }
                continue;
            }
            n += 1;
            match r {
                Ok(()) => ok += 1,
                Err(e) if shown < verbose => {
                    shown += 1;
                    println!("  FAIL {}: {e}", case.get("name").str());
                }
                Err(_) => {}
            }
        }
        let pct = if n == 0 { 100.0 } else { 100.0 * ok as f64 / n as f64 };
        println!("{name:<16} {n:>6} {:>6} {pct:>7.2}% {aok:>5}/{an:<5}", n - ok);
        if pct < 99.0 {
            below.push(name);
        }
        total += n;
        pass += ok;
        ae_total += an;
        ae_pass += aok;
    }
    println!(
        "total: {pass}/{total} passed ({:.3}%); address-error cases {ae_pass}/{ae_total}",
        100.0 * pass as f64 / total.max(1) as f64
    );
    assert!(below.is_empty(), "files below 99%: {below:?}");
}
