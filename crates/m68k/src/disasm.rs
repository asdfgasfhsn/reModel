//! 68000 disassembler, Motorola syntax (lower case, `$` hex).

use crate::{ea_ok, Sz, ALL, ALTERABLE, CONTROL, CTRL_ALT, DATA, DATA_ALT, IMM, MEM_ALT, POSTINC, PREDEC};

const CC: [&str; 16] = ["t", "f", "hi", "ls", "cc", "cs", "ne", "eq", "vc", "vs", "pl", "mi", "ge", "lt", "gt", "le"];

struct D<F: Fn(u32) -> u16> {
    addr: u32,
    len: u32,
    fetch: F,
}

/// Illegal or unassigned encoding.
struct Bad;

type R<T> = Result<T, Bad>;

fn hex(v: u32) -> String {
    if v < 10 {
        format!("{v}")
    } else {
        format!("${v:x}")
    }
}

fn signed(v: i32) -> String {
    if v < 0 {
        format!("-{}", hex(v.unsigned_abs()))
    } else {
        hex(v as u32)
    }
}

impl<F: Fn(u32) -> u16> D<F> {
    fn next(&mut self) -> u16 {
        let w = (self.fetch)(self.addr.wrapping_add(self.len));
        self.len += 2;
        w
    }
    fn next32(&mut self) -> u32 {
        let hi = self.next() as u32;
        hi << 16 | self.next() as u32
    }
    /// Address of the next extension word (the base for PC-relative modes).
    fn here(&self) -> u32 {
        self.addr.wrapping_add(self.len)
    }

    fn imm(&mut self, sz: Sz) -> String {
        let v = match sz {
            Sz::B => self.next() as u32 & 0xff,
            Sz::W => self.next() as u32,
            Sz::L => self.next32(),
        };
        format!("#{}", hex(v))
    }

    fn index(&mut self, base: &str) -> String {
        let ext = self.next();
        let r = if ext & 0x8000 != 0 { 'a' } else { 'd' };
        let s = if ext & 0x800 != 0 { 'l' } else { 'w' };
        format!("{}({base},{r}{}.{s})", signed(ext as i8 as i32), (ext >> 12) & 7)
    }

    fn ea(&mut self, mode: u16, reg: u16, sz: Sz, class: u16) -> R<String> {
        if !ea_ok(mode, reg, class) {
            return Err(Bad);
        }
        let r = reg & 7;
        Ok(match mode & 7 {
            0 => format!("d{r}"),
            1 => format!("a{r}"),
            2 => format!("(a{r})"),
            3 => format!("(a{r})+"),
            4 => format!("-(a{r})"),
            5 => format!("{}(a{r})", signed(self.next() as i16 as i32)),
            6 => self.index(&format!("a{r}")),
            _ => match r {
                0 => format!("${:x}.w", self.next() as i16 as u32 & 0xff_ffff),
                1 => format!("${:x}.l", self.next32()),
                2 => {
                    let base = self.here();
                    let t = base.wrapping_add(self.next() as i16 as u32);
                    format!("${t:x}(pc)")
                }
                3 => {
                    let base = self.here();
                    let ext = self.next();
                    let t = base.wrapping_add(ext as i8 as u32);
                    let x = if ext & 0x8000 != 0 { 'a' } else { 'd' };
                    let s = if ext & 0x800 != 0 { 'l' } else { 'w' };
                    format!("${t:x}(pc,{x}{}.{s})", (ext >> 12) & 7)
                }
                _ => self.imm(sz),
            },
        })
    }

    /// Effective address from the low six opcode bits.
    fn src(&mut self, op: u16, sz: Sz, class: u16) -> R<String> {
        self.ea((op >> 3) & 7, op & 7, sz, class)
    }

    fn insn(&mut self, op: u16) -> R<String> {
        let (mode, reg) = ((op >> 3) & 7, op & 7);
        let rx = (op >> 9) & 7;
        let size = Sz::from_bits(op >> 6);
        let ins = |m: &str, sz: Option<Sz>, ops: String| {
            let m = format!("{m}{}", sz.map_or("", Sz::suffix));
            if ops.is_empty() {
                m
            } else {
                format!("{m:<8}{ops}")
            }
        };
        Ok(match op >> 12 {
            0x0 => {
                if op & 0x100 != 0 {
                    if mode == 1 {
                        let sz = if op & 0x40 != 0 { Sz::L } else { Sz::W };
                        let mem = format!("{}(a{reg})", signed(self.next() as i16 as i32));
                        return Ok(if op & 0x80 != 0 {
                            ins("movep", Some(sz), format!("d{rx},{mem}"))
                        } else {
                            ins("movep", Some(sz), format!("{mem},d{rx}"))
                        });
                    }
                    let k = (op >> 6) & 3;
                    let m = ["btst", "bchg", "bclr", "bset"][k as usize];
                    let dst = self.src(op, Sz::B, if k == 0 { DATA } else { DATA_ALT })?;
                    return Ok(ins(m, None, format!("d{rx},{dst}")));
                }
                let k = (op >> 9) & 7;
                if k == 4 {
                    let kind = (op >> 6) & 3;
                    let m = ["btst", "bchg", "bclr", "bset"][kind as usize];
                    if !ea_ok(mode, reg, if kind == 0 { DATA & !IMM } else { DATA_ALT }) {
                        return Err(Bad);
                    }
                    let bit = self.next() & 0xff;
                    let dst = self.src(op, Sz::B, ALL)?;
                    return Ok(ins(m, None, format!("#{bit},{dst}")));
                }
                let sz = size.ok_or(Bad)?;
                let m = match k {
                    0 => "ori",
                    1 => "andi",
                    2 => "subi",
                    3 => "addi",
                    5 => "eori",
                    6 => "cmpi",
                    _ => return Err(Bad),
                };
                if mode == 7 && reg == 4 {
                    if !matches!(k, 0 | 1 | 5) || sz == Sz::L {
                        return Err(Bad);
                    }
                    let i = self.imm(sz);
                    return Ok(ins(m, None, format!("{i},{}", if sz == Sz::B { "ccr" } else { "sr" })));
                }
                if !ea_ok(mode, reg, DATA_ALT) {
                    return Err(Bad);
                }
                let i = self.imm(sz);
                let dst = self.src(op, sz, DATA_ALT)?;
                ins(m, Some(sz), format!("{i},{dst}"))
            }
            0x1..=0x3 => {
                let sz = match op >> 12 {
                    1 => Sz::B,
                    2 => Sz::L,
                    _ => Sz::W,
                };
                let dmode = (op >> 6) & 7;
                if dmode == 1 {
                    if sz == Sz::B {
                        return Err(Bad);
                    }
                    let s = self.src(op, sz, ALL)?;
                    return Ok(ins("movea", Some(sz), format!("{s},a{rx}")));
                }
                if !ea_ok(dmode, rx, DATA_ALT) {
                    return Err(Bad);
                }
                let s = self.src(op, sz, if sz == Sz::B { DATA } else { ALL })?;
                let d = self.ea(dmode, rx, sz, DATA_ALT)?;
                ins("move", Some(sz), format!("{s},{d}"))
            }
            0x4 => self.group4(op)?,
            0x5 => match size {
                None => {
                    let cc = CC[(op >> 8) as usize & 15];
                    if mode == 1 {
                        let base = self.here();
                        let t = base.wrapping_add(self.next() as i16 as u32);
                        let m = if cc == "f" { "dbra".to_string() } else { format!("db{cc}") };
                        ins(&m, None, format!("d{reg},${t:x}"))
                    } else {
                        let d = self.src(op, Sz::B, DATA_ALT)?;
                        ins(&format!("s{cc}"), None, d)
                    }
                }
                Some(sz) => {
                    if mode == 1 && sz == Sz::B {
                        return Err(Bad);
                    }
                    let q = if rx == 0 { 8 } else { rx };
                    let m = if op & 0x100 != 0 { "subq" } else { "addq" };
                    let d = self.src(op, sz, ALTERABLE)?;
                    ins(m, Some(sz), format!("#{q},{d}"))
                }
            },
            0x6 => {
                let cc = (op >> 8) as usize & 15;
                let base = self.here();
                let (disp, s) = match op & 0xff {
                    0 => (self.next() as i16 as u32, ".w"),
                    d => (d as i8 as u32, ".s"),
                };
                let m = match cc {
                    0 => "bra".to_string(),
                    1 => "bsr".to_string(),
                    _ => format!("b{}", CC[cc]),
                };
                ins(&format!("{m}{s}"), None, format!("${:x}", base.wrapping_add(disp)))
            }
            0x7 => {
                if op & 0x100 != 0 {
                    return Err(Bad);
                }
                ins("moveq", None, format!("#{},d{rx}", signed(op as i8 as i32)))
            }
            0x8 | 0xc => {
                let opmode = (op >> 6) & 7;
                let and = op >> 12 == 0xc;
                match opmode {
                    3 | 7 => {
                        let m = match (and, opmode) {
                            (false, 3) => "divu",
                            (false, _) => "divs",
                            (true, 3) => "mulu",
                            (true, _) => "muls",
                        };
                        let s = self.src(op, Sz::W, DATA)?;
                        ins(m, Some(Sz::W), format!("{s},d{rx}"))
                    }
                    4 if mode <= 1 => {
                        let m = if and { "abcd" } else { "sbcd" };
                        ins(m, None, if mode == 0 { format!("d{reg},d{rx}") } else { format!("-(a{reg}),-(a{rx})") })
                    }
                    5 if and && mode == 0 => ins("exg", None, format!("d{rx},d{reg}")),
                    5 if and && mode == 1 => ins("exg", None, format!("a{rx},a{reg}")),
                    6 if and && mode == 1 => ins("exg", None, format!("d{rx},a{reg}")),
                    _ => {
                        let sz = Sz::from_bits(opmode).ok_or(Bad)?;
                        let m = if and { "and" } else { "or" };
                        if opmode < 4 {
                            let s = self.src(op, sz, DATA)?;
                            ins(m, Some(sz), format!("{s},d{rx}"))
                        } else {
                            let d = self.src(op, sz, MEM_ALT)?;
                            ins(m, Some(sz), format!("d{rx},{d}"))
                        }
                    }
                }
            }
            0x9 | 0xd => {
                let opmode = (op >> 6) & 7;
                let sub = op >> 12 == 0x9;
                let m = if sub { "sub" } else { "add" };
                match opmode {
                    3 | 7 => {
                        let sz = if opmode == 3 { Sz::W } else { Sz::L };
                        let s = self.src(op, sz, ALL)?;
                        ins(&format!("{m}a"), Some(sz), format!("{s},a{rx}"))
                    }
                    _ => {
                        let sz = Sz::from_bits(opmode).ok_or(Bad)?;
                        if opmode >= 4 && mode <= 1 {
                            let ops = if mode == 0 { format!("d{reg},d{rx}") } else { format!("-(a{reg}),-(a{rx})") };
                            ins(&format!("{m}x"), Some(sz), ops)
                        } else if opmode < 4 {
                            let s = self.src(op, sz, if sz == Sz::B { DATA } else { ALL })?;
                            ins(m, Some(sz), format!("{s},d{rx}"))
                        } else {
                            let d = self.src(op, sz, MEM_ALT)?;
                            ins(m, Some(sz), format!("d{rx},{d}"))
                        }
                    }
                }
            }
            0xb => {
                let opmode = (op >> 6) & 7;
                match opmode {
                    3 | 7 => {
                        let sz = if opmode == 3 { Sz::W } else { Sz::L };
                        let s = self.src(op, sz, ALL)?;
                        ins("cmpa", Some(sz), format!("{s},a{rx}"))
                    }
                    0..=2 => {
                        let sz = Sz::from_bits(opmode).ok_or(Bad)?;
                        let s = self.src(op, sz, if sz == Sz::B { DATA } else { ALL })?;
                        ins("cmp", Some(sz), format!("{s},d{rx}"))
                    }
                    _ => {
                        let sz = Sz::from_bits(opmode).ok_or(Bad)?;
                        if mode == 1 {
                            ins("cmpm", Some(sz), format!("(a{reg})+,(a{rx})+"))
                        } else {
                            let d = self.src(op, sz, DATA_ALT)?;
                            ins("eor", Some(sz), format!("d{rx},{d}"))
                        }
                    }
                }
            }
            0xe => {
                let dir = if op & 0x100 != 0 { "l" } else { "r" };
                match size {
                    None => {
                        if op & 0x800 != 0 {
                            return Err(Bad);
                        }
                        let k = ["as", "ls", "rox", "ro"][(op >> 9) as usize & 3];
                        let d = self.src(op, Sz::W, MEM_ALT)?;
                        ins(&format!("{k}{dir}"), Some(Sz::W), d)
                    }
                    Some(sz) => {
                        let k = ["as", "ls", "rox", "ro"][(op >> 3) as usize & 3];
                        let count = if op & 0x20 != 0 {
                            format!("d{rx}")
                        } else {
                            format!("#{}", if rx == 0 { 8 } else { rx })
                        };
                        ins(&format!("{k}{dir}"), Some(sz), format!("{count},d{reg}"))
                    }
                }
            }
            _ => return Err(Bad),
        })
    }

    fn group4(&mut self, op: u16) -> R<String> {
        let (mode, reg) = ((op >> 3) & 7, op & 7);
        let rx = (op >> 9) & 7;
        let size = Sz::from_bits(op >> 6);
        let ins = |m: &str, sz: Option<Sz>, ops: String| {
            let m = format!("{m}{}", sz.map_or("", Sz::suffix));
            if ops.is_empty() {
                m
            } else {
                format!("{m:<8}{ops}")
            }
        };
        if op & 0x100 != 0 {
            return Ok(match (op >> 6) & 7 {
                6 => {
                    let s = self.src(op, Sz::W, DATA)?;
                    ins("chk", Some(Sz::W), format!("{s},d{rx}"))
                }
                7 => {
                    let s = self.src(op, Sz::L, CONTROL)?;
                    ins("lea", None, format!("{s},a{rx}"))
                }
                _ => return Err(Bad),
            });
        }
        Ok(match (op >> 8) & 0xf {
            k @ (0x0 | 0x2 | 0x4 | 0x6) => match size {
                Some(sz) => {
                    let m = ["negx", "clr", "neg", "not"][k as usize / 2];
                    let d = self.src(op, sz, DATA_ALT)?;
                    ins(m, Some(sz), d)
                }
                None => match k {
                    0x0 => {
                        let d = self.src(op, Sz::W, DATA_ALT)?;
                        ins("move", None, format!("sr,{d}"))
                    }
                    0x4 => {
                        let s = self.src(op, Sz::W, DATA)?;
                        ins("move", None, format!("{s},ccr"))
                    }
                    0x6 => {
                        let s = self.src(op, Sz::W, DATA)?;
                        ins("move", None, format!("{s},sr"))
                    }
                    _ => return Err(Bad),
                },
            },
            0x8 => match (op >> 6) & 3 {
                0 => {
                    let d = self.src(op, Sz::B, DATA_ALT)?;
                    ins("nbcd", None, d)
                }
                1 if mode == 0 => ins("swap", None, format!("d{reg}")),
                1 => {
                    let s = self.src(op, Sz::L, CONTROL)?;
                    ins("pea", None, s)
                }
                k if mode == 0 => ins("ext", Some(if k == 2 { Sz::W } else { Sz::L }), format!("d{reg}")),
                k => {
                    let sz = if k == 2 { Sz::W } else { Sz::L };
                    if !ea_ok(mode, reg, CTRL_ALT | PREDEC) {
                        return Err(Bad);
                    }
                    let mask = self.next();
                    let d = self.src(op, sz, CTRL_ALT | PREDEC)?;
                    let list = reglist(if mode == 4 { mask.reverse_bits() } else { mask });
                    ins("movem", Some(sz), format!("{list},{d}"))
                }
            },
            0xa => match size {
                Some(sz) => {
                    let d = self.src(op, sz, DATA_ALT)?;
                    ins("tst", Some(sz), d)
                }
                None if op == 0x4afc => "illegal".to_string(),
                None => {
                    let d = self.src(op, Sz::B, DATA_ALT)?;
                    ins("tas", None, d)
                }
            },
            0xc if op & 0x80 != 0 => {
                let sz = if op & 0x40 != 0 { Sz::L } else { Sz::W };
                if !ea_ok(mode, reg, CONTROL | POSTINC) {
                    return Err(Bad);
                }
                let mask = self.next();
                let s = self.src(op, sz, CONTROL | POSTINC)?;
                ins("movem", Some(sz), format!("{s},{}", reglist(mask)))
            }
            0xe => match op {
                0x4e40..=0x4e4f => ins("trap", None, format!("#{}", op & 15)),
                0x4e50..=0x4e57 => ins("link", None, format!("a{reg},#{}", signed(self.next() as i16 as i32))),
                0x4e58..=0x4e5f => ins("unlk", None, format!("a{reg}")),
                0x4e60..=0x4e67 => ins("move", None, format!("a{reg},usp")),
                0x4e68..=0x4e6f => ins("move", None, format!("usp,a{reg}")),
                0x4e70 => "reset".into(),
                0x4e71 => "nop".into(),
                0x4e72 => ins("stop", None, format!("#{}", hex(self.next() as u32))),
                0x4e73 => "rte".into(),
                0x4e75 => "rts".into(),
                0x4e76 => "trapv".into(),
                0x4e77 => "rtr".into(),
                0x4e80..=0x4eff => {
                    let m = if op & 0x40 != 0 { "jmp" } else { "jsr" };
                    let t = self.src(op, Sz::L, CONTROL)?;
                    ins(m, None, t)
                }
                _ => return Err(Bad),
            },
            _ => return Err(Bad),
        })
    }
}

/// MOVEM register list, bit 0 = D0 ... bit 15 = A7, e.g. `d0-d2/a6`.
fn reglist(mask: u16) -> String {
    let mut parts = Vec::new();
    for (bank, base) in [('d', 0), ('a', 8)] {
        let mut i = 0;
        while i < 8 {
            if mask & (1 << (base + i)) == 0 {
                i += 1;
                continue;
            }
            let start = i;
            while i < 8 && mask & (1 << (base + i)) != 0 {
                i += 1;
            }
            parts.push(if i - 1 == start {
                format!("{bank}{start}")
            } else {
                format!("{bank}{start}-{bank}{}", i - 1)
            });
        }
    }
    if parts.is_empty() {
        "#0".into()
    } else {
        parts.join("/")
    }
}

/// Disassemble the instruction at `addr`; returns its text and length in
/// bytes. Unassigned encodings come out as `dc.w`.
pub fn disasm(addr: u32, fetch16: impl Fn(u32) -> u16) -> (String, u32) {
    let mut d = D { addr, len: 0, fetch: fetch16 };
    let op = d.next();
    match d.insn(op) {
        Ok(s) => (s, d.len),
        Err(Bad) => (format!("dc.w    ${op:04x}"), 2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dis(words: &[u16]) -> (String, u32) {
        disasm(0x1000, |a| words.get(((a - 0x1000) / 2) as usize).copied().unwrap_or(0))
    }

    #[test]
    fn formats() {
        assert_eq!(dis(&[0x3219]), ("move.w  (a1)+,d1".into(), 2));
        assert_eq!(dis(&[0x203c, 0x1234, 0x5678]), ("move.l  #$12345678,d0".into(), 6));
        assert_eq!(dis(&[0x48e7, 0xfffe]), ("movem.l d0-d7/a0-a6,-(a7)".into(), 4));
        assert_eq!(dis(&[0x4cdf, 0x7fff]), ("movem.l (a7)+,d0-d7/a0-a6".into(), 4));
        assert_eq!(dis(&[0x6100, 0x0010]), ("bsr.w   $1012".into(), 4));
        assert_eq!(dis(&[0x66fe]), ("bne.s   $1000".into(), 2));
        assert_eq!(dis(&[0x51c8, 0xfffe]), ("dbra    d0,$1000".into(), 4));
        assert_eq!(dis(&[0x41fa, 0x0006]), ("lea     $1008(pc),a0".into(), 4));
        assert_eq!(dis(&[0x1030, 0x1804]), ("move.b  4(a0,d1.l),d0".into(), 4));
        assert_eq!(dis(&[0x4e75]), ("rts".into(), 2));
        assert_eq!(dis(&[0x70ff]), ("moveq   #-1,d0".into(), 2));
        assert_eq!(dis(&[0x0240, 0x00ff]), ("andi.w  #$ff,d0".into(), 4));
        assert_eq!(dis(&[0x007c, 0x0700]), ("ori     #$700,sr".into(), 4));
        assert_eq!(dis(&[0xe349]), ("lsl.w   #1,d1".into(), 2));
        assert_eq!(dis(&[0x13fc, 0x0001, 0x00ff, 0x0000]), ("move.b  #1,$ff0000.l".into(), 8));
        assert_eq!(dis(&[0x4afc]), ("illegal".into(), 2));
        assert_eq!(dis(&[0xffff]), ("dc.w    $ffff".into(), 2));
    }
}
