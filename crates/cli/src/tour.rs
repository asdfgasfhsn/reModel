//! `remodel tour list` and `remodel tour run`: named input scripts from
//! power-on that read the game's art, and the ROM read maps they leave
//! (docs/uplift-studio.md, "Tours and coverage"; `coverage.rs` reports).
//!
//! A tour is a line of `LABEL|LENGTH|INPUT`: the length in CPU steps, or
//! frames with an `f` (`6200f`), and the input script of `--input`
//! (`name@from-to`, `name=value@from-to`), with two more terms:
//!
//! - `from=LABEL@FRAME`: start as tour LABEL did up to FRAME (its inputs
//!   to that frame are this tour's first; a branch cannot rewrite them),
//!   then go on with this tour's own.
//! - `mark=NAME@FRAME`: save the state as frame FRAME starts, to
//!   `DIR/states/LABEL-NAME.state` (a scene is a tour and a frame).
//! - `poke=NAME@FROM-TO` (or `poke=ADDR:VALUE[:b|:w|:l]@FROM-TO`): hold a
//!   RAM value, a cheat, to reach content without skill. NAME is a line of
//!   the game's table (`romset::games::pokes`). A tour that pokes, or
//!   branches from one that does, is marked, and so are its states
//!   (`LABEL-NAME.poked.state`); nothing pokes unless a tour says so.
//!
//! A run is always from power-on, in a machine of its own, with the EEPROM
//! as the board is made (never an `eeprom.bin` left in a folder: that
//! changed a state hash on its own, `docs/uplift/content-coverage.md`, E2).
//! So a tour run twice gives the same state hash and the same map.
//! Nothing here changes what the machine does: the maps are written to,
//! never read back, and the state hash is the same as `run`'s.

use crate::answer::{Answer, Code};
use crate::coverage::{self, MapFile};
use model2::readmap::{Ranges, ReadMap};
use romset::RomSet;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

/// The codes under the names this module uses.
pub mod answer {
    use crate::answer::Code;
    pub const BAD_REQUEST: Code = Code::BadRequest;
    pub const MISSING: Code = Code::Missing;
    pub const INTERNAL: Code = Code::Internal;
}

/// A failed answer with the command that would put it right.
pub fn fail(code: Code, msg: impl Into<String>, next: Option<&str>) -> Answer {
    let a = Answer::fail(code, msg);
    match next {
        Some(n) => a.next(n),
        None => a,
    }
}

/// An answer given out as JSON or text; `text` is given its data.
pub trait Finish {
    fn finish(self, json: bool, text: impl FnOnce(&serde_json::Value) -> String) -> ExitCode;
}

impl Finish for Answer {
    fn finish(self, json: bool, text: impl FnOnce(&serde_json::Value) -> String) -> ExitCode {
        let data = self.data.clone();
        self.emit(json, move || text(&data))
    }
}

/// How long a tour runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Length {
    /// CPU instructions, as `run`'s STEPS.
    Steps(u64),
    /// Frames, written `6200f`.
    Frames(u64),
}

impl Length {
    fn parse(s: &str) -> Option<Length> {
        let n = |t: &str| match t.strip_prefix("0x") {
            Some(h) => u64::from_str_radix(h, 16).ok(),
            None => t.parse().ok(),
        };
        match s.strip_suffix('f') {
            Some(f) => n(f).map(Length::Frames),
            None => n(s).map(Length::Steps),
        }
        .filter(|l| !matches!(l, Length::Steps(0) | Length::Frames(0)))
    }

    fn text(self) -> String {
        match self {
            Length::Steps(n) => format!("{n}"),
            Length::Frames(n) => format!("{n}f"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tour {
    pub label: String,
    pub length: Length,
    /// `from=LABEL@FRAME`.
    pub from: Option<(String, u64)>,
    /// `mark=NAME@FRAME`, in the order written.
    pub marks: Vec<(String, u64)>,
    /// The input items of the tour's own, as written.
    pub inputs: Vec<String>,
    /// The line, normalised, and a branch's parents' lines: what a map
    /// or a spine's checkpoints record to know the tour changed.
    pub spec: String,
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

/// `NAME@FRAME`.
fn at(term: &str, what: &str) -> Result<(String, u64), String> {
    let (name, frame) = term.split_once('@').ok_or_else(|| format!("{what}: want NAME@FRAME"))?;
    match number(frame) {
        Some(f) if name_ok(name) => Ok((name.to_string(), f)),
        _ => Err(format!("{what}: want NAME@FRAME with a plain name and a frame number")),
    }
}

/// The tours in `text` (lines of `LABEL|LENGTH|INPUT`, `#` comments),
/// checked against each other: labels are unique, `from=` names a tour
/// that exists and does not loop, a branch's own inputs start after its
/// branch point, marks are named once and fall inside the tour.
pub fn parse(text: &str, source: &str) -> Result<Vec<Tour>, String> {
    let mut tours: Vec<Tour> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let here = format!("{source}:{}", n + 1);
        let mut part = line.splitn(3, '|');
        let (Some(label), Some(length), Some(input)) = (part.next(), part.next(), part.next()) else {
            return Err(format!("{here}: want LABEL|LENGTH|INPUT"));
        };
        let (label, length) = (label.trim(), length.trim());
        if !name_ok(label) {
            return Err(format!("{here}: tour name {label:?}: letters, digits, - and _ only"));
        }
        if tours.iter().any(|t| t.label == label) {
            return Err(format!("{here}: tour {label} is already defined"));
        }
        let length = Length::parse(length)
            .ok_or_else(|| format!("{here}: length {length:?}: want a number of steps, or frames with an f (6200f)"))?;
        let (mut from, mut marks, mut inputs) = (None, Vec::new(), Vec::new());
        for term in input.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            if let Some(f) = term.strip_prefix("from=") {
                if from.is_some() {
                    return Err(format!("{here}: a tour has one from="));
                }
                from = Some(at(f, &format!("{here}: {term}"))?);
            } else if let Some(m) = term.strip_prefix("mark=") {
                let (name, frame) = at(m, &format!("{here}: {term}"))?;
                if marks.iter().any(|(n, _)| *n == name) {
                    return Err(format!("{here}: mark {name} is named twice"));
                }
                if let Length::Frames(len) = length {
                    if frame >= len {
                        return Err(format!("{here}: {term}: the tour ends at frame {len}"));
                    }
                }
                marks.push((name, frame));
            } else {
                inputs.push(term.to_string());
            }
        }
        let spec =
            format!("{label}|{}|{}", length.text(), input.split(',').map(str::trim).collect::<Vec<_>>().join(","));
        tours.push(Tour { label: label.to_string(), length, from, marks, inputs, spec });
    }
    for t in &tours {
        let Some((parent, frame)) = &t.from else { continue };
        if tours.iter().all(|p| p.label != *parent) {
            let all: Vec<&str> = tours.iter().map(|t| t.label.as_str()).collect();
            return Err(format!("{source}: tour {}: from={parent}: no such tour{}", t.label, near(parent, &all)));
        }
        // No loops: follow the chain.
        let (mut at, mut steps) = (parent.as_str(), 0);
        while let Some((next, _)) = tours.iter().find(|p| p.label == at).and_then(|p| p.from.as_ref()) {
            at = next;
            steps += 1;
            if steps > tours.len() || *next == t.label {
                return Err(format!("{source}: tour {} is its own ancestor", t.label));
            }
        }
        for item in &t.inputs {
            if let Some((from_f, _)) = item_range(item) {
                if from_f <= *frame {
                    return Err(format!(
                        "{source}: tour {}: {item} starts at or before the branch point {frame} of from={parent}",
                        t.label
                    ));
                }
            }
        }
    }
    let specs: Vec<String> = tours.iter().map(|t| chain_spec(&tours, &t.label)).collect();
    for (t, spec) in tours.iter_mut().zip(specs) {
        t.spec = spec;
    }
    Ok(tours)
}

/// " (did you mean a, b?)" for the names in `all` close to `name`.
pub fn near(name: &str, all: &[&str]) -> String {
    let close = crate::answer::near_matches(name, &all.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    if close.is_empty() {
        let known = all.join(", ");
        if known.is_empty() {
            String::new()
        } else {
            format!(" (known: {known})")
        }
    } else {
        format!(" (did you mean {}?)", close.join(", "))
    }
}

/// An input item's first and last frame (`name@from-to[/n]`); None for a
/// term that is not one (the checks leave it to `InputScript::parse`).
fn item_range(item: &str) -> Option<(u64, u64)> {
    let (_, frames) = item.rsplit_once('@')?;
    let frames = frames.split_once('/').map_or(frames, |(f, _)| f);
    match frames.split_once('-') {
        Some((a, b)) => Some((number(a)?, number(b)?)),
        None => Some((number(frames)?, number(frames)?)),
    }
}

/// `item` as it applies up to `frame`: unchanged if it ends by then,
/// ended there if it runs on, dropped if it starts later.
pub fn cut(item: &str, frame: u64) -> Option<String> {
    let (what, frames) = item.rsplit_once('@')?;
    let (range, every) = match frames.split_once('/') {
        Some((r, n)) => (r, Some(n)),
        None => (frames, None),
    };
    let (from, to) = item_range(item)?;
    if from > frame {
        return None;
    }
    if to <= frame {
        return Some(item.to_string());
    }
    let _ = range;
    Some(format!("{what}@{from}-{frame}{}", every.map(|n| format!("/{n}")).unwrap_or_default()))
}

/// All of a tour's inputs, in order: its `from=` parent's up to the branch
/// point (recursively), then its own. `coin_start` is the game's own
/// `coin-start` script, which stands for itself in an input.
pub fn inputs(tours: &[Tour], label: &str, coin_start: &str) -> Result<Vec<String>, String> {
    let t = tours.iter().find(|t| t.label == label).ok_or_else(|| format!("no tour {label}"))?;
    let mut out = Vec::new();
    if let Some((parent, frame)) = &t.from {
        for item in inputs(tours, parent, coin_start)? {
            out.extend(cut(&item, *frame));
        }
    }
    for item in &t.inputs {
        if item == "coin-start" {
            out.extend(coin_start.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string));
        } else {
            out.push(item.clone());
        }
    }
    Ok(out)
}

/// A poke in an input item: `poke=NAME@frames` or `poke=ADDR:VALUE...@frames`.
fn is_poke(item: &str) -> bool {
    item.starts_with("poke=")
}

/// Whether tour `label` pokes RAM: its own inputs, or those it inherits.
pub fn uses_pokes(tours: &[Tour], label: &str, coin_start: &str) -> bool {
    inputs(tours, label, coin_start).is_ok_and(|v| v.iter().any(|i| is_poke(i)))
}

/// The game's pokes (`NAME|ADDR:VALUE[:b|:w|:l]|what it does and how it
/// was found`, `#` comments), by name.
pub fn poke_table(text: &str) -> Result<Vec<(String, String, String)>, String> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut p = line.splitn(3, '|');
        let (Some(name), Some(spec), note) = (p.next(), p.next(), p.next().unwrap_or("")) else {
            return Err(format!("pokes:{}: want NAME|ADDR:VALUE[:b|:w|:l]|NOTE", n + 1));
        };
        if !name_ok(name.trim()) || out.iter().any(|(k, _, _)| k == name.trim()) {
            return Err(format!("pokes:{}: poke name {name:?}: letters, digits, - and _, once", n + 1));
        }
        out.push((name.trim().to_string(), spec.trim().to_string(), note.trim().to_string()));
    }
    Ok(out)
}

/// How a named value's bytes read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Unsigned,
    Signed,
    /// An IEEE single (width 4 only).
    Float,
}

/// One named game value: a work-RAM address, its width in bytes, how the
/// bytes read, and the note that says what it is and how it was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueDef {
    pub name: String,
    pub addr: u32,
    pub width: usize,
    pub kind: Kind,
    pub note: String,
}

/// The game's named values (`NAME|ADDR:WIDTH[:signed|:float]|what it is and
/// how it was found`, `#` comments; WIDTH is `b`, `w` or `l`, a byte, a
/// 16-bit word, a 32-bit long, little-endian), in file order.
pub fn value_table(text: &str) -> Result<Vec<ValueDef>, String> {
    let mut out: Vec<ValueDef> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let want = || format!("values:{}: want NAME|ADDR:b|w|l[:signed|:float]|NOTE", n + 1);
        let mut p = line.splitn(3, '|');
        let (Some(name), Some(spec), Some(note)) = (p.next(), p.next(), p.next()) else { return Err(want()) };
        let name = name.trim();
        if !name_ok(name) || out.iter().any(|v| v.name == name) {
            return Err(format!("values:{}: value name {name:?}: letters, digits, - and _, once", n + 1));
        }
        let parts: Vec<&str> = spec.trim().split(':').collect();
        let (addr, width) = match parts.as_slice() {
            [a, w, ..] => (number(a).and_then(|a| u32::try_from(a).ok()), ["b", "w", "l"].iter().position(|x| x == w)),
            _ => return Err(want()),
        };
        let (Some(addr), Some(width)) = (addr, width.map(|i| 1usize << i)) else { return Err(want()) };
        let kind = match parts.get(2).copied() {
            None => Kind::Unsigned,
            Some("signed") => Kind::Signed,
            Some("float") if width == 4 => Kind::Float,
            _ => return Err(want()),
        };
        if parts.len() > 3 || note.trim().is_empty() {
            return Err(want());
        }
        out.push(ValueDef { name: name.into(), addr, width, kind, note: note.trim().into() });
    }
    Ok(out)
}

/// `items` with each named poke (`poke=hold-clock@10-20`) replaced by its
/// table entry (`poke=0x500123:0x63@10-20`); numbers pass through.
pub fn resolve_pokes(items: Vec<String>, table: &[(String, String, String)]) -> Result<Vec<String>, String> {
    items
        .into_iter()
        .map(|item| {
            let Some(rest) = item.strip_prefix("poke=") else { return Ok(item) };
            let (what, frames) = rest.split_once('@').ok_or_else(|| format!("{item}: missing @frames"))?;
            if what.starts_with(|c: char| c.is_ascii_digit()) {
                return Ok(item);
            }
            match table.iter().find(|(k, _, _)| k == what) {
                Some((_, spec, _)) => Ok(format!("poke={spec}@{frames}")),
                None => {
                    let names: Vec<&str> = table.iter().map(|(k, _, _)| k.as_str()).collect();
                    Err(format!("{item}: no poke named {what}{}", near(what, &names)))
                }
            }
        })
        .collect()
}

/// The game's own tours, then each of `extra`'s files in order.
pub fn load_paths(game: &romset::Game, extra: &[PathBuf]) -> Result<Vec<Tour>, String> {
    load_many(game, &extra.iter().map(PathBuf::as_path).collect::<Vec<_>>())
}

/// A tour's line and its `from=` parents' lines (`Tour::spec` of a
/// branch, so a branch changes when its spine does).
fn chain_spec(tours: &[Tour], label: &str) -> String {
    let mut out = String::new();
    let mut cur = tours.iter().find(|t| t.label == label);
    while let Some(t) = cur {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&t.spec);
        cur = t.from.as_ref().and_then(|(p, _)| tours.iter().find(|x| x.label == *p));
    }
    out
}

pub fn load_many(game: &romset::Game, extra: &[&Path]) -> Result<Vec<Tour>, String> {
    let mut text = romset::games::tours(game).to_string();
    let mut source = format!("{} tours", game.id);
    for p in extra {
        let more = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
        text.push('\n');
        text.push_str(&more);
        source = format!("{source} and {}", p.display());
    }
    parse(&text, &source)
}

/// Remove `--name VALUE` from `args`.
pub fn take_opt(args: &mut Vec<String>, name: &str) -> Option<String> {
    let i = args.iter().position(|a| a == name)?;
    if i + 1 >= args.len() {
        return None;
    }
    let v = args.remove(i + 1);
    args.remove(i);
    Some(v)
}

/// Remove the flag `name` from `args`; whether it was there.
pub fn take_flag(args: &mut Vec<String>, name: &str) -> bool {
    match args.iter().position(|a| a == name) {
        Some(i) => {
            args.remove(i);
            true
        }
        None => false,
    }
}

/// Where a game's maps and tour states go: `<out>/coverage`.
pub fn default_dir(game: &str) -> PathBuf {
    crate::OUT_DIR.get().cloned().unwrap_or_else(|| PathBuf::from("out").join(game)).join("coverage")
}

/// The options both verbs share, taken out of the arguments.
pub struct Common {
    pub json: bool,
    /// `--tours FILE`, as many times as given: a branch's spine can be in another file.
    pub tours: Vec<PathBuf>,
    pub dir: Option<PathBuf>,
}

pub fn common(args: &mut Vec<String>) -> Result<Common, String> {
    let json = take_flag(args, "--json");
    let mut tours = Vec::new();
    while let Some(t) = take_opt(args, "--tours") {
        tours.push(PathBuf::from(t));
    }
    let dir = take_opt(args, "--dir");
    Ok(Common { json, tours, dir: dir.map(PathBuf::from) })
}

/// `remodel tour VERB`: `list` here (no ROMs needed); `run` is [`run`].
pub fn list(o: &crate::Opts) -> ExitCode {
    let verb = o.args.get(1).map(String::as_str);
    if verb != Some("list") {
        let e = match verb {
            Some(v) => format!("tour {v}: no such verb{}", near(v, &["list", "run"])),
            None => "tour: say what to do (list, run)".to_string(),
        };
        let json = o.args.iter().any(|a| a == "--json");
        return fail(answer::BAD_REQUEST, e, Some("remodel tour list")).finish(json, |_| String::new());
    }
    let mut args = o.args[2..].to_vec();
    let c = match common(&mut args) {
        Ok(c) => c,
        Err(e) => return fail(answer::BAD_REQUEST, e, None).finish(false, |_| String::new()),
    };
    let a = list_answer(o, &c, &args);
    a.finish(c.json, |d| {
        let mut s = String::new();
        for t in d["tours"].as_array().into_iter().flatten() {
            s.push_str(&format!(
                "{:<14} {:>8}  {}{}{}{}\n",
                t["label"].as_str().unwrap_or(""),
                t["length"].as_str().unwrap_or(""),
                if t["from"].is_null() {
                    String::new()
                } else {
                    format!("from {}@{}  ", t["from"]["tour"].as_str().unwrap_or(""), t["from"]["frame"])
                },
                if t["marks"].as_array().is_some_and(|m| m.is_empty()) {
                    String::new()
                } else {
                    format!("{} marks  ", t["marks"].as_array().map_or(0, |m| m.len()))
                },
                if t["mapped"] == true { "mapped" } else { "no map yet" },
                if t["poked"] == true { "  (pokes RAM)" } else { "" },
            ));
        }
        s
    })
}

fn list_answer(o: &crate::Opts, c: &Common, args: &[String]) -> Answer {
    if let Some(extra) = args.first() {
        return fail(answer::BAD_REQUEST, format!("tour list: unexpected {extra}"), Some("remodel tour list [--json]"));
    }
    let game = match romset::games::find(&o.game_id) {
        Ok(g) => g,
        Err(e) => return fail(answer::BAD_REQUEST, e.to_string(), Some("remodel games")),
    };
    let tours = match load_paths(game, &c.tours) {
        Ok(t) => t,
        Err(e) => return fail(answer::BAD_REQUEST, e, None),
    };
    let dir = c.dir.clone().unwrap_or_else(|| default_dir(game.id));
    let rows: Vec<Value> = tours
        .iter()
        .map(|t| {
            json!({
                "label": t.label,
                "length": t.length.text(),
                "from": t.from.as_ref().map(|(p, f)| json!({"tour": p, "frame": f})),
                "marks": t.marks.iter().map(|(n, f)| json!({"name": n, "frame": f})).collect::<Vec<_>>(),
                "mapped": MapFile::path(&dir, &t.label).exists(),
                "poked": uses_pokes(&tours, &t.label, game.controls.coin_start),
            })
        })
        .collect();
    let mut a = Answer::done(json!({"game": game.id, "tours": rows, "dir": dir.display().to_string()}));
    if tours.is_empty() {
        a = a.warn(format!("{} has no tours: write some in a file and pass --tours FILE", game.id));
    }
    a.next(format!("remodel tour run --all --game {}", game.id))
}

/// `remodel tour run NAME|--all`.
pub fn run(o: &crate::Opts, roms: &RomSet) -> ExitCode {
    let mut args = o.args[2..].to_vec();
    let c = match common(&mut args) {
        Ok(c) => c,
        Err(e) => return fail(answer::BAD_REQUEST, e, None).finish(false, |_| String::new()),
    };
    let a = run_answer(&mut args, &c, roms);
    a.finish(c.json, |d| {
        let mut s = String::new();
        for t in d["tours"].as_array().into_iter().flatten() {
            s.push_str(&format!(
                "tour {}: {} frames, state hash {}, polygon ROM {:.1}% read ({} KB), texture ROM {:.1}% read ({} KB), {:.1} s\n",
                t["label"].as_str().unwrap_or(""),
                t["frames"],
                t["state_hash"].as_str().unwrap_or(""),
                t["polygon"]["share"].as_f64().unwrap_or(0.0) * 100.0,
                t["polygon"]["bytes_read"].as_u64().unwrap_or(0) / 1024,
                t["texture"]["share"].as_f64().unwrap_or(0.0) * 100.0,
                t["texture"]["bytes_read"].as_u64().unwrap_or(0) / 1024,
                t["seconds"].as_f64().unwrap_or(0.0),
            ));
            for m in t["marks"].as_array().into_iter().flatten() {
                s.push_str(&format!("  mark {} at frame {} -> {}\n", m["name"].as_str().unwrap_or(""), m["frame"], m["state"].as_str().unwrap_or("")));
            }
        }
        if d["tours"].as_array().into_iter().flatten().any(|t| t["poked"] == true) {
            s.push_str("(a poked tour is a cheat run: its states are named .poked.state, `coverage` says which tours poke)\n");
        }
        s.push_str("(from power-on, EEPROM as the board is made; `remodel coverage` reports what the tours read)\n");
        s
    })
}

fn run_answer(args: &mut Vec<String>, c: &Common, roms: &RomSet) -> Answer {
    let all = take_flag(args, "--all");
    let usage = "remodel tour run NAME|--all [--tours FILE]... [--dir DIR] [--json]";
    let game = roms.game;
    let tours = match load_paths(game, &c.tours) {
        Ok(t) => t,
        Err(e) => return fail(answer::BAD_REQUEST, e, None),
    };
    let names: Vec<&str> = tours.iter().map(|t| t.label.as_str()).collect();
    let wanted: Vec<&Tour> = match (all, &args[..]) {
        (true, []) => tours.iter().collect(),
        (false, [name]) => match tours.iter().find(|t| t.label == *name) {
            Some(t) => vec![t],
            None => {
                return fail(
                    answer::BAD_REQUEST,
                    format!("no tour {name} for {}{}", game.id, near(name, &names)),
                    Some("remodel tour list"),
                )
            }
        },
        _ => return fail(answer::BAD_REQUEST, "name one tour, or --all", Some(usage)),
    };
    if wanted.is_empty() {
        return fail(answer::MISSING, format!("{} has no tours", game.id), Some("write some and pass --tours FILE"));
    }
    let dir = c.dir.clone().unwrap_or_else(|| default_dir(game.id));
    if let Err(e) = std::fs::create_dir_all(dir.join("states")) {
        return fail(answer::INTERNAL, format!("{}: {e}", dir.display()), None);
    }
    let table = match poke_table(romset::games::pokes(game)) {
        Ok(t) => t,
        Err(e) => return fail(answer::INTERNAL, format!("{} {e}", game.id), None),
    };
    let mut rows = Vec::new();
    let mut warnings = Vec::new();
    for t in wanted {
        let items = match inputs(&tours, &t.label, game.controls.coin_start) {
            Ok(i) => i,
            Err(e) => return fail(answer::BAD_REQUEST, e, None),
        };
        let items = match resolve_pokes(items, &table) {
            Ok(i) => i,
            Err(e) => return fail(answer::BAD_REQUEST, format!("tour {}: {e}", t.label), Some("remodel ram pokes")),
        };
        let script = match model2::input::InputScript::parse(&items.join(","), &game.controls) {
            Ok(s) => s,
            Err(e) => return fail(answer::BAD_REQUEST, format!("tour {}: {e}", t.label), None),
        };
        match run_one(roms, t, &script, &dir, script.pokes()) {
            Ok((row, w)) => {
                rows.push(row);
                warnings.extend(w);
            }
            Err(e) => return fail(answer::INTERNAL, format!("tour {}: {e}", t.label), None),
        }
    }
    let mut a = Answer::done(json!({"game": game.id, "dir": dir.display().to_string(), "tours": rows}));
    for w in warnings {
        a = a.warn(w);
    }
    a.next(format!("remodel coverage --game {}", game.id))
}

/// Tour `label`'s whole input as a script (named pokes resolved) and
/// whether it pokes.
pub fn build_script(
    game: &romset::Game,
    tours: &[Tour],
    label: &str,
) -> Result<(model2::input::InputScript, bool), String> {
    let table = poke_table(romset::games::pokes(game)).map_err(|e| format!("{} {e}", game.id))?;
    let items = resolve_pokes(inputs(tours, label, game.controls.coin_start)?, &table)?;
    let script = model2::input::InputScript::parse(&items.join(","), &game.controls)
        .map_err(|e| format!("tour {label}: {e}"))?;
    let poked = script.pokes();
    Ok((script, poked))
}

/// One tour, from power-on: the maps, the marks' states, a row of numbers.
pub fn run_one(
    roms: &RomSet,
    t: &Tour,
    script: &model2::input::InputScript,
    dir: &Path,
    poked: bool,
) -> Result<(Value, Vec<String>), String> {
    let started = std::time::Instant::now();
    // A machine of its own and the EEPROM as the board is made: nothing
    // is read from an output folder.
    let mut m = crate::machine(roms);
    if crate::GEO_MODE.get().is_none() {
        // The native geometrizer where the game has one: the same frames
        // and state hashes, much faster (`--geo chip` runs the game's own
        // geometrizer program, which reads the same ROM words).
        m.bus.geo.set_mode(model2::geo::GeoMode::Native);
    }
    let poly_map = Arc::new(ReadMap::new(m.bus.geo.poly_rom_words()));
    m.bus.geo.set_read_map(Some(poly_map.clone()));
    let tex_map = ReadMap::new(m.bus.texture_rom.len());
    let (mut last_frame, mut marks) = (0, Vec::new());
    // Frame 0's inputs too, as the player applies them before the first frame.
    script.apply(0, &mut m.bus);
    loop {
        let done = match t.length {
            Length::Steps(n) => m.cpu.icount >= n,
            Length::Frames(n) => m.frames >= n,
        };
        if done {
            break;
        }
        let budget = match t.length {
            Length::Steps(n) => n - m.cpu.icount,
            Length::Frames(_) => u64::MAX / 2,
        };
        if let Err(stop) = m.run_slice(budget) {
            return Err(format!("the machine stopped at frame {}: {stop:?}", m.frames));
        }
        if m.frames != last_frame {
            // Where the rasteriser reads texture headers and UV lists, on
            // a copy of the frame: the machine is not touched.
            let f = model2::capture::Frame::grab(&mut m.bus);
            let src = f.sources(&m.bus.texture_rom);
            let b = model2::raster::BuildIn { tex_map: Some(&tex_map), ..Default::default() };
            let _ = model2::raster::build_with(&f.stream, &src, &b);
            let now = m.frames;
            for (name, frame) in t.marks.iter().filter(|(_, f)| *f == now) {
                // As frame N starts, before its scripted inputs.
                let data = model2::save_session(&mut m, None);
                let path =
                    dir.join("states").join(format!("{}-{name}{}.state", t.label, if poked { ".poked" } else { "" }));
                std::fs::write(&path, data).map_err(|e| format!("{}: {e}", path.display()))?;
                marks.push(json!({"name": name, "frame": frame, "state": path.display().to_string()}));
            }
            script.apply(m.frames, &mut m.bus);
        }
        last_frame = m.frames;
    }
    let hash = m.bus.state_hash();
    let mut warnings = Vec::new();
    for (name, frame) in &t.marks {
        if !marks.iter().any(|v| v["name"] == name.as_str()) {
            warnings.push(format!(
                "tour {}: mark {name} at frame {frame} was not reached (the tour ran {} frames)",
                t.label, m.frames
            ));
        }
    }
    let poly_rom = model2::bus::polygon_rom(roms);
    let map = MapFile {
        game: roms.game.id.to_string(),
        tour: t.label.clone(),
        spec: coverage::fnv(&t.spec),
        frames: m.frames,
        hash,
        poly_words: poly_map.len() as u64,
        tex_words: tex_map.len() as u64,
        poly_read: poly_map.ranges(),
        tex_read: tex_map.ranges(),
        poly_blank: Ranges::blank(&poly_rom, coverage::BLANK_BYTES / 4),
        tex_blank: Ranges::blank(&m.bus.texture_rom, coverage::BLANK_BYTES / 2),
    };
    let path = MapFile::path(dir, &t.label);
    map.save(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (poly, tex) = (map.poly_summary(), map.tex_summary());
    let row = json!({
        "label": t.label,
        "frames": m.frames,
        "state_hash": format!("{hash:016x}"),
        "poked": poked,
        "polygon": poly,
        "texture": tex,
        "marks": marks,
        "map": path.display().to_string(),
        "seconds": started.elapsed().as_secs_f64(),
    });
    Ok((row, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    const COIN: &str = "coin1@10-12,start@20-22";

    /// A tour starts from another's inputs up to its branch point; the
    /// parent's longer holds are cut there and its later ones dropped.
    #[test]
    fn a_branch_inherits_its_parents_inputs_to_the_branch_point() {
        let t = parse("a|100f|coin-start,left@30-60,right@70-80\nb|100f|from=a@40,right@50-60,mark=m@45\n", "test")
            .unwrap();
        assert_eq!(t[1].from, Some(("a".to_string(), 40)));
        assert_eq!(t[1].marks, vec![("m".to_string(), 45)]);
        let got = inputs(&t, "b", COIN).unwrap();
        assert_eq!(got, ["coin1@10-12", "start@20-22", "left@30-40", "right@50-60"]);
        // The whole of a tour is its own, with coin-start spelled out.
        assert_eq!(inputs(&t, "a", COIN).unwrap().len(), 4);
    }

    /// A repeat keeps its period when cut.
    #[test]
    fn a_cut_repeat_keeps_its_period() {
        assert_eq!(cut("punch@10-999/30", 100).as_deref(), Some("punch@10-100/30"));
        assert_eq!(cut("punch@10-90/30", 100).as_deref(), Some("punch@10-90/30"));
        assert_eq!(cut("punch@110-120", 100), None);
        assert_eq!(cut("gun1=10:20@5-50", 20).as_deref(), Some("gun1=10:20@5-20"));
    }

    #[test]
    fn bad_tours_are_refused_with_the_line() {
        let bad = |t: &str| parse(t, "f").unwrap_err();
        assert!(bad("x|10f").contains("f:1: want LABEL|LENGTH|INPUT"));
        assert!(bad("x|0f|").contains("length"));
        assert!(bad("x y|10f|").contains("tour name"));
        assert!(bad("x|10f|\nx|10f|").contains("already defined"));
        assert!(bad("x|10f|from=nope@3").contains("no such tour"));
        assert!(bad("a|10f|\nb|10f|from=aa@3").contains("did you mean a"));
        assert!(bad("a|10f|from=b@3\nb|10f|from=a@3").contains("own ancestor"));
        assert!(bad("a|10f|\nb|20f|from=a@5,left@5-6").contains("branch point"));
        assert!(bad("a|10f|mark=m@2,mark=m@3").contains("named twice"));
        assert!(bad("a|10f|mark=m@10").contains("ends at frame 10"));
        assert!(bad("a|10f|from=a@1,from=a@2").contains("one from="));
    }

    /// A named poke becomes the table's write; a number passes; an unknown
    /// name is refused with the near ones; a branch of a poking tour pokes.
    #[test]
    fn named_pokes_resolve_and_mark_their_branches() {
        let table = poke_table("# c\nhold-clock|0x5830b0:0x0e10:w|a note | with a bar\n").unwrap();
        assert_eq!(table[0].2, "a note | with a bar");
        let got =
            resolve_pokes(vec!["poke=hold-clock@5-9".into(), "poke=0x10:1@2".into(), "left@3".into()], &table).unwrap();
        assert_eq!(got, ["poke=0x5830b0:0x0e10:w@5-9", "poke=0x10:1@2", "left@3"]);
        let e = resolve_pokes(vec!["poke=hold-clok@5".into()], &table).unwrap_err();
        assert!(e.contains("did you mean hold-clock"), "{e}");
        assert!(poke_table("a|1:2\na|3:4").unwrap_err().contains("once"));
        assert!(poke_table("x y|1:2").is_err());
        let t =
            parse("a|100f|poke=hold-clock@10-50\nb|100f|from=a@40,left@50-60\nc|100f|coin-start\n", "test").unwrap();
        assert!(uses_pokes(&t, "a", COIN) && uses_pokes(&t, "b", COIN) && !uses_pokes(&t, "c", COIN));
    }

    #[test]
    fn lengths_in_steps_and_frames() {
        assert_eq!(Length::parse("900000000"), Some(Length::Steps(900_000_000)));
        assert_eq!(Length::parse("6200f"), Some(Length::Frames(6200)));
        assert_eq!(Length::parse("f"), None);
        assert_eq!(Length::parse("0"), None);
    }

    /// The tours shipped with the games parse, their branches and marks
    /// are consistent, and the inputs the shipped tours build are accepted
    /// by the game's own controls.
    #[test]
    fn the_shipped_tours_parse_and_their_inputs_are_valid() {
        for g in romset::games::ALL {
            let tours = load_many(g, &[]).unwrap_or_else(|e| panic!("{}: {e}", g.id));
            for t in &tours {
                let items = inputs(&tours, &t.label, g.controls.coin_start).unwrap();
                let table = poke_table(romset::games::pokes(g)).unwrap_or_else(|e| panic!("{} {e}", g.id));
                let items = resolve_pokes(items, &table).unwrap_or_else(|e| panic!("{} {}: {e}", g.id, t.label));
                model2::input::InputScript::parse(&items.join(","), &g.controls)
                    .unwrap_or_else(|e| panic!("{} {}: {e}", g.id, t.label));
            }
        }
        let top = load_many(romset::games::find("topskatr").unwrap(), &[]).unwrap();
        assert!(top.iter().any(|t| t.label == "bench" && t.length == Length::Steps(600_000_000)));
    }
}
