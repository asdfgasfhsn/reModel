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

/// A 9-bit operand: `ar` names the register an indirect one goes through.
fn operand(c9: u32, ar: u32) -> String {
    if c9 & 0x100 != 0 {
        format!("[ar{ar}]{}", step(c9))
    } else if c9 & 0x80 != 0 {
        format!("[ar2+{}]", c9 & 0x7f)
    } else {
        format!("[{:#04x}]", c9 & 0x7f)
    }
}

fn transfer(w: u32) -> Option<String> {
    let kind = (w >> 16) & 0x1f;
    let op1 = (w >> 9) & 0x7f;
    let c9 = w & 0x1ff;
    let r = |v: u32| reg_name(v);
    Some(if w >> 26 == 0 {
        let b = operand(c9, if kind == 0x0f { 2 } else { 3 });
        let a = match kind {
            0x0c => format!("ram1[{op1:#04x}]"),
            0x0f => format!("[ar3]{}", step(op1)),
            0x10 if op1 & 0x40 != 0 => format!("[{op1:#04x}]"),
            0x10 => format!("ram1[{op1:#04x}]"),
            _ => return None,
        };
        format!("r13 = {a}, r10 = {b}")
    } else {
        match kind {
            0x1c if op1 & 0x40 != 0 => format!("ext{} = {}", operand(c9, 3), r(op1)),
            0x1c => format!("{} = {}", operand(c9, 3), r(op1)),
            0x1d => format!("{} = {}", r(op1), operand(c9, 3)),
            0x1e => format!("{} = ext{}", r(op1), operand(c9, if op1 & 0x40 != 0 { 2 } else { 3 })),
            0x1f if op1 == 0x0f => String::new(),
            0x1f => format!("{} = {}", r(op1), r(c9)),
            0x0c => format!("ram1[{op1:#04x}] = {}", operand(c9, 3)),
            0x10 => format!("[{op1:#04x}] = {}", operand(c9, 3)),
            0x08 => format!("[{op1:#04x}] = ext{}", operand(c9, 3)),
            0x03 => format!("ext[ar3]{} = {}", step(op1), operand(c9, 2)),
            0x07 => format!("ext[ar3]{} = alt{}", step(op1), operand(c9, 2)),
            0x0b => format!("[ar3]{} = ext{}", step(op1), operand(c9, 2)),
            0x0f | 0x13 => format!("[ar3]{} = {}", step(op1), operand(c9, 2)),
            _ => return None,
        }
    })
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
            0x39 => format!("r10 = #{}", sext(w, 24)),
            0x3b => format!("acc = #{}", sext(w, 24)),
            0x3c | 0x3d => {
                let c = (w >> 16) & 0xf;
                let t = if c == 4 { format!("repeat {}", w & 0xffff) } else { format!("ctrl {c} = {:#x}", w & 0xffff) };
                both(alu_name((w >> 20) & 0x1f), t)
            }
            0x40..=0x43 => format!("ar{} = #{imm24:#x}", top & 3),
            0x50..=0x5f => format!("{} = #{imm24:#x}", reg_name(0x10 + (top & 0xf))),
            0x63 if w & 0x0040_0000 != 0 => "bus shared RAM".into(),
            0x63 => "bus function unit".into(),
            0x7c => "?? 7c (no-op)".into(),
            0xbe | 0xfe => {
                let c = (w >> 16) & 0xff;
                let name = match c & !0x0c {
                    0x00 => "z",
                    0x10 => "nn",
                    0x20 => "le",
                    _ => return format!("?? condition {c:02x}"),
                };
                let neg = if top == 0xfe { "!" } else { "" };
                if c & 0x0c != 0 {
                    format!("if {neg}{name}: {} = [{:#04x}]", reg_name((w >> 9) & 0x3f), w & 0xff)
                } else {
                    format!("j{neg}{name} {:#05x}", w & 0xffff)
                }
            }
            0xbf => match (w >> 16) & 0xff {
                0x60 => format!("jmp {:#05x}", w & 0xffff),
                0x64 => format!("call {:#05x}", w & 0xffff),
                0x66 => format!("call {}", reg_name(w & 0x3f)),
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
        assert_eq!(line(0x1c1c_4381), "[ar3]+1 = in");
        assert_eq!(line(0xbf6a_0000), "ret");
        assert_eq!(line(0x5a00_007f), "acc.e = #0x7f");
        assert!(line(0x1d3c_4255).starts_with("acc += p, p = r10*r13"));
        assert_eq!(line(0xbe10_026b), "jnn 0x26b");
    }
}
