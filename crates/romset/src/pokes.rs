//! A game's poke table (`games/pokes/GAME.txt`): RAM writes found by running
//! the game, one per line,
//!
//! ```text
//! NAME|ADDR:VALUE[:b|:w|:l]|what it does, how it was found[|cheat: LABEL]
//! ```
//!
//! `#` starts a comment line. A tour holds a poke by name
//! (`poke=NAME@from-to`, `remodel tour`); a line with the last field,
//! `cheat: LABEL`, is also one of the player's cheats (the Esc menu's Cheats
//! page, docs/play.md), and LABEL is what the page calls it. The note may
//! itself hold `|`; only a last field that starts with `cheat:` is the
//! label, and it is not part of the note.

use crate::Game;

/// One line of a poke table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Poke {
    /// Letters, digits, `-` and `_`; once in a table.
    pub name: String,
    /// The write as the line spells it (`0x5830b0:0x0e10:w`): what a tour's
    /// `poke=` takes.
    pub spec: String,
    /// Where it writes (work RAM).
    pub addr: u32,
    pub value: u32,
    /// Bytes: 1, 2 or 4.
    pub width: usize,
    /// What it does and how it was found (without the cheat label).
    pub note: String,
    /// The player's cheat this is, by its label (the 4th field).
    pub cheat: Option<String>,
}

impl Poke {
    /// The note's first sentence (the whole note if it has no full stop
    /// followed by a space): the small line under a cheat's label.
    pub fn short_note(&self) -> &str {
        match self.note.find(". ") {
            Some(i) => &self.note[..=i],
            None => self.note.trim_end(),
        }
    }
}

fn name_ok(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn number(s: &str) -> Option<u64> {
    match s.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

/// `ADDR:VALUE[:b|:w|:l]`: address, value and width in bytes (a byte when
/// none is given).
fn spec(s: &str) -> Result<(u32, u32, usize), String> {
    let want = "want ADDR:VALUE[:b|:w|:l]";
    let mut it = s.split(':');
    let (Some(a), Some(v)) = (it.next(), it.next()) else { return Err(want.into()) };
    let width = match it.next() {
        None | Some("b") => 1,
        Some("w") => 2,
        Some("l") => 4,
        Some(w) => return Err(format!("width {w:?}: want b, w or l")),
    };
    if it.next().is_some() {
        return Err(want.into());
    }
    let (Some(addr), Some(value)) = (number(a), number(v)) else { return Err("ADDR and VALUE are numbers".into()) };
    if addr > u32::MAX as u64 || value >= 1u64 << (8 * width) {
        return Err(format!("the value does not fit {width} byte(s)"));
    }
    Ok((addr as u32, value as u32, width))
}

/// The table in `text`, in order. An error names the line (`pokes:N: ...`).
pub fn parse(text: &str) -> Result<Vec<Poke>, String> {
    let mut out: Vec<Poke> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let at = n + 1;
        // The last field is the cheat label if it says so.
        let (body, cheat) = match line.rsplit_once('|') {
            Some((body, last)) if last.trim_start().starts_with("cheat:") => {
                let label = last.trim_start().trim_start_matches("cheat:").trim();
                if label.is_empty() {
                    return Err(format!("pokes:{at}: `cheat:` wants a label"));
                }
                (body, Some(label.to_string()))
            }
            _ => (line, None),
        };
        let mut p = body.splitn(3, '|');
        let (Some(name), Some(sp), note) = (p.next(), p.next(), p.next().unwrap_or("")) else {
            return Err(format!("pokes:{at}: want NAME|ADDR:VALUE[:b|:w|:l]|NOTE[|cheat: LABEL]"));
        };
        let name = name.trim();
        if !name_ok(name) || out.iter().any(|k| k.name == name) {
            return Err(format!("pokes:{at}: poke name {name:?}: letters, digits, - and _, once"));
        }
        let sp = sp.trim();
        let (addr, value, width) = spec(sp).map_err(|e| format!("pokes:{at}: {name}: {e}"))?;
        out.push(Poke {
            name: name.to_string(),
            spec: sp.to_string(),
            addr,
            value,
            width,
            note: note.trim().to_string(),
            cheat,
        });
    }
    Ok(out)
}

/// `game`'s poke table ([`crate::games::pokes`]); a table that does not
/// parse is empty (a test over every shipped table keeps them parsing).
pub fn table(game: &Game) -> Vec<Poke> {
    parse(crate::games::pokes(game)).unwrap_or_default()
}

/// `game`'s cheats: the table's lines that carry a label, in order.
pub fn cheats(game: &Game) -> Vec<Poke> {
    table(game).into_iter().filter(|p| p.cheat.is_some()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_with_and_without_the_cheat_field() {
        let t = parse(
            "# c\n\nhold-clock|0x5830b0:0x0e10:w|Held at 60 s. Found by search.|cheat: Time never runs out\n\
             id-6|0x582f00:6:l|forced | to 6\nbyte|0x500000:1|note\nbare|0x500001:0x7f\n",
        )
        .unwrap();
        assert_eq!(t.len(), 4);
        let c = &t[0];
        assert_eq!((c.name.as_str(), c.spec.as_str()), ("hold-clock", "0x5830b0:0x0e10:w"));
        assert_eq!((c.addr, c.value, c.width), (0x5830b0, 0x0e10, 2));
        assert_eq!(c.note, "Held at 60 s. Found by search.", "the label is not in the note");
        assert_eq!(c.cheat.as_deref(), Some("Time never runs out"));
        assert_eq!(c.short_note(), "Held at 60 s.");
        // A note may hold a bar; no 4th field means no cheat.
        assert_eq!((t[1].note.as_str(), t[1].cheat.as_deref(), t[1].width), ("forced | to 6", None, 4));
        assert_eq!((t[2].width, t[2].short_note()), (1, "note"));
        assert_eq!((t[3].note.as_str(), t[3].value), ("", 0x7f));
    }

    #[test]
    fn bad_lines_are_refused_with_their_number() {
        for (text, want) in [
            ("a|1:2\na|3:4", "once"),
            ("x y|1:2", "poke name"),
            ("only-a-name", "want NAME"),
            ("a|zz:1", "numbers"),
            ("a|0x10:0x100", "does not fit"),
            ("a|0x10:1:q", "width"),
            ("a|0x10:1:w:l", "want ADDR"),
            ("a|0x10:1|n|cheat:", "wants a label"),
            ("# ok\na|0x10:1|n|cheat:   ", "pokes:2"),
        ] {
            let e = parse(text).unwrap_err();
            assert!(e.contains(want), "{text:?}: {e}");
        }
    }

    /// Every shipped table parses, and a cheat has a label.
    #[test]
    fn every_shipped_table_parses() {
        for g in crate::games::ALL {
            let t = parse(crate::games::pokes(g)).unwrap_or_else(|e| panic!("{} {e}", g.id));
            for p in &t {
                assert!(p.cheat.as_deref().is_none_or(|l| !l.is_empty()), "{} {}", g.id, p.name);
            }
        }
        let top = cheats(crate::games::find("topskatr").unwrap());
        assert_eq!(top.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["hold-clock"]);
    }
}
