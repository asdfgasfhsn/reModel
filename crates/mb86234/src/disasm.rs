//! Mnemonics for MB86234 words, for the encodings the interpreter
//! implements (`docs/hw/mb86234.md` says how sure each is). Forms that
//! aren't known print as `??`.

fn sext(v: u32, bits: u32) -> i32 {
    let s = 32 - bits;
    ((v << s) as i32) >> s
}

/// A register of the transfer fields.
pub fn reg_name(r: u32) -> String {
    let r = r & 0x3f;
    let views = ["r10", "r13", "r16", "acc", "p"];
    match r {
        0x00..=0x03 => format!("ar{r}"),
        0x0c => "lc".into(),
        0x23 => "bus".into(),
        0x0f => "null".into(),
        0x10..=0x1e => {
            let (b, part) = ((r - 0x10) / 3, (r - 0x10) % 3);
            format!("{}{}", views[b as usize], ["", ".e", ".m"][part as usize])
        }
        0x1f => "sh".into(),
        0x20 => "st".into(),
        0x21 => "in".into(),
        0x22 => "out".into(),
        _ => format!("r{r:02x}"),
    }
}

/// The ALU operation of code `c`, in words.
pub fn alu_name(c: u32) -> String {
    match c {
        0 => String::new(),
        1 => "acc &= r10".into(),
        2 => "acc |= r10".into(),
        3 => "acc ^= r10".into(),
        4 => "acc = ~acc".into(),
        5 => "fcmp acc, r10".into(),
        6 => "acc += r10".into(),
        7 => "acc -= r10".into(),
        8 => "p = r10*r13".into(),
        9 => "acc += p, p = r10*r13".into(),
        10 => "acc -= p, p = r10*r13".into(),
        11 => "acc = |acc|".into(),
        12 => "acc += p".into(),
        13 => "acc = p, p = r10*r13".into(),
        14 => "acc = float(acc)".into(),
        15 => "acc = int(acc)".into(),
        16 => "acc /= r10".into(),
        17 => "acc = -acc".into(),
        19 => "acc = r13 + r10".into(),
        20 => "acc = r13 - r10".into(),
        22 => "acc >>= sh".into(),
        23 => "acc <<= sh".into(),
        24 => "acc >>= sh (signed)".into(),
        26 => "acc += r10 (int)".into(),
        27 => "acc -= r10 (int)".into(),
        c => format!("?alu{c}"),
    }
}

fn step(m: u32) -> String {
    match sext(m & 0x1f, 5) {
        0 => String::new(),
        d if d > 0 => format!("+{d}"),
        d => format!("{d}"),
    }
}

/// An indirect operand through `ar`: bit 5 set, the register itself;
/// clear, indexed: AR0 + AR2 or AR1 + AR3, and with bit 6 also set
/// circular (the step wraps round a ring, shown `%`).
fn indirect(m: u32, ar: u32) -> String {
    let ring = if m & 0x60 == 0x40 { "%" } else { "" };
    match (m & 0x20 != 0, ar) {
        (true, _) => format!("[ar{ar}]{}", step(m)),
        (false, 2) => format!("[ar0+ar2]{ring}{}", step(m)),
        (false, _) => format!("[ar1+ar3]{ring}{}", step(m)),
    }
}

/// A 9-bit operand: `ar` names the register an indirect one goes through.
fn operand(c9: u32, ar: u32) -> String {
    if c9 & 0x100 != 0 {
        indirect(c9, ar)
    } else if c9 & 0x80 != 0 {
        format!("[ar{}+ar{ar}+{}]", ar & 1, c9 & 0x7f)
    } else {
        format!("[{:#04x}]", c9 & 0x7f)
    }
}

fn transfer(w: u32) -> Option<String> {
    let kind = (w >> 16) & 0x1f;
    let op1 = (w >> 9) & 0x7f;
    let c9 = w & 0x1ff;
    let r = |v: u32| reg_name(v);
    let mult = w >> 26 == 0;
    if !mult && kind >= 0x1c {
        return Some(match kind {
            0x1c if op1 & 0x40 != 0 => format!("ext{} = {}", operand(c9, 3), r(op1)),
            0x1c => format!("{} = {}", operand(c9, 3), r(op1)),
            0x1d => format!("{} = {}", r(op1), operand(c9, 3)),
            0x1e if op1 & 0x40 != 0 => format!("{} = prog{}", r(op1), operand(c9, 2)),
            0x1e => format!("{} = ext{}", r(op1), operand(c9, 3)),
            0x1f if op1 == 0x0f => String::new(),
            _ => format!("{} = {}", r(op1), r(c9)),
        });
    }
    // Group (bits 2-4): the spaces of op1 and c9; bits 0-1: op1's mode.
    let group = kind >> 2;
    let (s1, s9) = match group {
        0 | 1 => ("ext", ""),
        2 => ("", "ext"),
        3 | 4 => ("", ""),
        5 => ("", "prog"),
        _ => return None,
    };
    // Group 3 reads and writes op1 in RAM1 and c9 in RAM0, group 4 the
    // other way round.
    let (s1, s9) = match group {
        3 => ("ram1", s9),
        4 => (s1, "ram1"),
        _ => (s1, s9),
    };
    let a = match kind & 3 {
        0 => format!("{s1}[{op1:#04x}]"),
        1 => format!("{s1}[ar1+ar3+{op1}]"),
        3 => format!("{s1}{}", indirect(op1, 3)),
        _ => return None,
    };
    let b = format!("{s9}{}", operand(c9, 2));
    Some(if mult { format!("r13 = {a}, r10 = {b}") } else { format!("{a} = {b}") })
}

/// One word in words, or `??` and the form when it isn't known.
pub fn line(w: u32) -> String {
    let top = w >> 24;
    let imm24 = w & 0xff_ffff;
    let both = |a: String, b: String| match (a.is_empty(), b.is_empty()) {
        (true, true) => "nop".to_string(),
        (false, true) => a,
        (true, false) => b,
        (false, false) => format!("{a:<24} | {b}"),
    };
    match w >> 26 {
        0x00 | 0x07 => match transfer(w) {
            Some(t) => both(alu_name((w >> 21) & 0x1f), t),
            None => format!("?? transfer type {:02x}", (w >> 16) & 0x1f),
        },
        _ => match top {
            0x36 => format!("mode {:#x} = {:#x}", (w >> 16) & 0xf, w & 0xffff),
            0x38 => format!("p = #{}", sext(w, 24)),
            0x39 => format!("r10 = #{}", sext(w, 24)),
            0x3a => format!("r13 = #{}", sext(w, 24)),
            0x3b => format!("acc = #{}", sext(w, 24)),
            0x3c | 0x3d => {
                let c = (w >> 16) & 0xf;
                let t = if c == 4 { format!("repeat {}", w & 0xffff) } else { format!("ctrl {c} = {:#x}", w & 0xffff) };
                both(alu_name((w >> 20) & 0x1f), t)
            }
            0x63 if w & 0x0040_0000 != 0 => "bus shared RAM".into(),
            0x63 => "bus function unit".into(),
            0x40..=0x7f => format!("{} = #{imm24:#x}", reg_name(top & 0x3f)),
            0xbe | 0xfe => {
                let c = (w >> 16) & 0xff;
                let name = match c & !0x0c {
                    0x00 => "z",
                    0x10 => "nn",
                    0x20 => "le",
                    0xa0 => "steep",
                    _ => return format!("?? condition {c:02x}"),
                };
                let neg = if top == 0xfe { "!" } else { "" };
                if c & 0x0c == 0x04 {
                    format!("call{neg}{name} {:#05x}", w & 0xffff)
                } else if c & 0x0c != 0 {
                    format!("if {neg}{name}: {} = [{:#04x}]", reg_name((w >> 9) & 0x3f), w & 0xff)
                } else {
                    format!("j{neg}{name} {:#05x}", w & 0xffff)
                }
            }
            0xbf => match (w >> 16) & 0xff {
                0x00 => format!("loop {:#05x}", w & 0xffff),
                0x10 => format!("loop r0d {:#05x}", w & 0xffff),
                0x60 => format!("jmp {:#05x}", w & 0xffff),
                0x64 => format!("call {:#05x}", w & 0xffff),
                0x62 | 0x66 => {
                    let op = if (w >> 16) & 0xff == 0x62 { "jmp" } else { "call" };
                    if w & 0x4000 != 0 {
                        format!("{op} {}", reg_name(w & 0x3f))
                    } else {
                        format!("{op} [{:#04x}]", w & 0x7f)
                    }
                }
                0x6a => "ret".into(),
                0x6e => "reti".into(),
                _ => "??".into(),
            },
            _ => "??".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::line;

    #[test]
    fn mnemonics() {
        assert_eq!(line(0x1c1c_3207), "[0x07] = acc");
        assert_eq!(line(0x1c1d_b207), "acc = [0x07]");
        assert_eq!(line(0x1c1f_4419), "out = acc");
        assert_eq!(line(0x1d1f_1e10), "p = r10*r13");
        assert_eq!(line(0x3c04_000c), "repeat 12");
        assert_eq!(line(0x1c1c_4381), "[ar1+ar3]+1 = in");
        assert_eq!(line(0x1c1c_43a1), "[ar3]+1 = in");
        assert_eq!(line(0xbf62_4019), "jmp acc");
        assert_eq!(line(0xfe04_01af), "call!z 0x1af");
        assert_eq!(line(0x4c00_001e), "lc = #0x1e");
        assert_eq!(line(0x1c1e_b380), "acc = prog[ar0+ar2]");
        assert_eq!(line(0x0010_0680), "r13 = [0x03], r10 = ram1[ar0+ar2+0]");
        assert_eq!(line(0xbf6a_0000), "ret");
        assert_eq!(line(0x5a00_007f), "acc.e = #0x7f");
        assert!(line(0x1d3c_4255).starts_with("acc += p, p = r10*r13"));
        assert_eq!(line(0xbe10_026b), "jnn 0x26b");
        assert_eq!(line(0xbf00_05cf), "loop 0x5cf");
        assert_eq!(line(0x1c1e_19a1), "lc = ext[ar3]+1");
    }
}
