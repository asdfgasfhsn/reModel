//! Moves: a per-game vocabulary of short actions learned from a person's
//! recordings (docs/uplift/scenes-and-server.md, "Exploring from a recording",
//! "Slice 5 as built"; docs/uplift-studio.md, "Moves").
//!
//! `remodel moves learn` does four things, all game-generic (what is game
//! specific is the game's value table, and its input table):
//!
//! 1. **Segment.** The recording's input terms become runs (one input held
//!    over some frames). Runs that overlap in time are one *cluster*, the
//!    unit that recurs: a steer, a jump, a jump while steering. A cluster is
//!    named from its parts: a digital button by how long it was held (tap,
//!    short, hold, long), an analog channel by direction, strength (lo, mid,
//!    hi: its share of the full swing) and a duration bucket (5, 10, 20, 40,
//!    80, 160 frames).
//! 2. **Measure.** One pass over the recording from a spine checkpoint before
//!    its first race frame. At each cluster's first frame the machine is
//!    copied and the copy runs a window with the cluster's inputs removed and
//!    everything else as recorded; the recording's own run is the other half.
//!    The difference in the named values over the window is the cluster's
//!    effect: speed, heading (from `heading-x` and `heading-z`), frames in the
//!    air, height, score.
//! 3. **Tag.** Each sample is tagged with the conditions it started under:
//!    airborne or not, a speed band (terciles of the samples' own speeds) and
//!    the screen.
//! 4. **Aggregate.** Per move and condition: the median and range of each
//!    effect and the sample count; moves with too few samples are dropped.
//!
//! The result is the move table (`crates/romset/src/games/moves/GAME.txt`),
//! which `session.moves` lists and `session.step move=NAME` plays.

use crate::answer::{Answer, Code};
use crate::tour::{self, Length, Tour, ValueDef};
use model2::bus::CopyMarks;
use model2::input::InputScript;
use model2::Machine;
use romset::{Game, RomSet};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

/// Names the value table must use for what moves measure (a table without
/// one just has no effect on it).
pub const V_SCREEN: &str = "screen";
pub const V_SPEED: &str = "speed";
pub const V_HEADING_X: &str = "heading-x";
pub const V_HEADING_Z: &str = "heading-z";
pub const V_AIRBORNE: &str = "airborne";
pub const V_HEIGHT: &str = "pos-y";
pub const V_SCORE: &str = "score";
pub const V_POS_X: &str = "pos-x";
pub const V_POS_Z: &str = "pos-z";
pub const V_VEL_X: &str = "vel-x";
pub const V_VEL_Z: &str = "vel-z";

/// The effects of a move, in the order the table writes them: speed at the
/// window's end, the heading's turn and the direction of travel's turn (degrees),
/// frames in the air, the highest point, score, and where the skater ended
/// along the direction of travel as the move began (`dfwd`) and across it
/// (`dlat`, positive towards +x when travelling along -z: the skater's left), each the recording's own run less the
/// run without the move.
pub const EFFECTS: [&str; 8] = ["dspeed", "dyaw", "dpath", "dair", "dheight", "dscore", "dfwd", "dlat"];

/// Frames measured after a cluster ends.
pub const TAIL: u64 = 60;
/// The longest window measured.
pub const MAX_WINDOW: u64 = 240;
/// The longest cluster that is a move (frames); a longer chain of
/// overlapping inputs is a stretch of driving.
pub const MAX_CLUSTER: u64 = 160;
/// Fewest samples a move or a condition row needs.
pub const MIN_SAMPLES: usize = 3;

const DURATIONS: [u64; 6] = [5, 10, 20, 40, 80, 160];

// ------------------------------------------------------------------ runs and clusters

/// One input held over frames, from a recording's terms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    /// The index of the term it came from.
    pub item: usize,
    pub name: String,
    pub value: Option<u8>,
    pub from: u64,
    pub to: u64,
}

fn num(s: &str) -> Option<u64> {
    match s.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

/// The runs in a recording's terms (`name@a-b`, `name=0xNN@a-b`). Pokes,
/// repeating terms and guns are not runs.
pub fn runs(items: &[String]) -> Vec<Run> {
    let mut out = Vec::new();
    for (item, it) in items.iter().enumerate() {
        if it.starts_with("poke=") || it.contains('/') {
            continue;
        }
        let Some((what, frames)) = it.split_once('@') else { continue };
        let range = match frames.split_once('-') {
            Some((a, b)) => num(a).zip(num(b)),
            None => num(frames).map(|a| (a, a)),
        };
        let Some((from, to)) = range.filter(|(a, b)| a <= b) else { continue };
        let (name, value) = match what.split_once('=') {
            Some((n, v)) => match num(v).and_then(|v| u8::try_from(v).ok()) {
                Some(v) => (n, Some(v)),
                None => continue,
            },
            None => (what, None),
        };
        out.push(Run { item, name: name.to_string(), value, from, to });
    }
    out
}

/// How a digital hold of `n` frames is named.
pub fn hold_class(n: u64) -> &'static str {
    match n {
        0..=6 => "tap",
        7..=14 => "short",
        15..=40 => "hold",
        _ => "long",
    }
}

/// The duration bucket nearest `n` frames (by ratio).
pub fn duration_bucket(n: u64) -> u64 {
    let n = n.max(1) as f64;
    *DURATIONS
        .iter()
        .min_by(|a, b| (n / **a as f64).ln().abs().total_cmp(&(n / **b as f64).ln().abs()))
        .expect("the buckets are not empty")
}

/// An analog value's strength class and direction: the share of the full
/// swing (`span`) it is from the centre, and the sign (negative is the
/// channel's negative direction).
pub fn analog_class(a: &romset::AnalogInput, v: u8) -> (&'static str, bool) {
    let span = if a.span == 0 { 1.0 } else { a.span as f64 };
    let frac = (v as f64 - a.centre as f64) / span;
    let s = frac.abs();
    (
        if s < 0.4 {
            "lo"
        } else if s < 0.8 {
            "mid"
        } else {
            "hi"
        },
        frac < 0.0,
    )
}

/// One input of a cluster, named and placed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub label: String,
    pub name: String,
    pub value: Option<u8>,
    /// Frames after the cluster's first.
    pub off: u64,
    pub len: u64,
    /// An analog part's duration is its bucket's.
    pub fixed_len: Option<u64>,
}

/// Runs that overlap in time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cluster {
    pub from: u64,
    pub to: u64,
    /// Indexes into the runs it was made from.
    pub runs: Vec<usize>,
    pub parts: Vec<Part>,
}

impl Cluster {
    pub fn len(&self) -> u64 {
        self.to - self.from + 1
    }

    /// The move's name: the parts' labels, in order, joined by `+`.
    pub fn key(&self) -> String {
        self.parts.iter().map(|p| p.label.as_str()).collect::<Vec<_>>().join("+")
    }
}

fn part_of(game: &Game, r: &Run, from: u64) -> Part {
    let len = r.to - r.from + 1;
    let analog = game.controls.analogs.iter().find(|a| a.name == r.name);
    match (analog, r.value) {
        (Some(a), Some(v)) => {
            let (strength, neg) = analog_class(a, v);
            let base = if a.steering {
                (if neg { "steer-left" } else { "steer-right" }).to_string()
            } else {
                format!("{}{}", a.name, if neg { "-neg" } else { "" })
            };
            let b = duration_bucket(len);
            Part {
                label: format!("{base}-{strength}-{b}f"),
                name: r.name.clone(),
                value: Some(v),
                off: r.from - from,
                len: b,
                fixed_len: Some(b),
            }
        }
        _ => Part {
            label: format!("{}-{}", r.name, hold_class(len)),
            name: r.name.clone(),
            value: r.value,
            off: r.from - from,
            len,
            fixed_len: None,
        },
    }
}

/// Clusters of overlapping runs (a run that starts on or before the
/// cluster's last frame joins it), in time order. `skipped_long` counts the
/// chains over [`MAX_CLUSTER`] frames, which are not returned.
pub fn clusters(game: &Game, runs: &[Run]) -> (Vec<Cluster>, usize) {
    let mut order: Vec<usize> = (0..runs.len()).collect();
    order.sort_by_key(|i| (runs[*i].from, runs[*i].to, runs[*i].name.clone()));
    let mut groups: Vec<(u64, u64, Vec<usize>)> = Vec::new();
    for i in order {
        let r = &runs[i];
        match groups.last_mut() {
            Some(g) if r.from <= g.1 => {
                g.1 = g.1.max(r.to);
                g.2.push(i);
            }
            _ => groups.push((r.from, r.to, vec![i])),
        }
    }
    let mut out = Vec::new();
    let mut long = 0;
    for (from, to, idx) in groups {
        if to - from + 1 > MAX_CLUSTER {
            long += 1;
            continue;
        }
        let parts = idx.iter().map(|i| part_of(game, &runs[*i], from)).collect();
        out.push(Cluster { from, to, runs: idx, parts });
    }
    (out, long)
}

// ------------------------------------------------------------------ samples

/// What the game was doing as a move started.
#[derive(Clone, Debug, PartialEq)]
pub struct Cond {
    pub airborne: Option<bool>,
    pub speed: Option<f64>,
    pub screen: String,
}

/// One measured occurrence of a move.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    /// The recording.
    pub label: String,
    pub key: String,
    pub at: u64,
    pub window: u64,
    pub parts: Vec<Part>,
    pub cond: Cond,
    pub fx: [Option<f64>; 8],
}

/// The cutoffs that split the samples' speeds into thirds (low below the
/// first, high at or above the second); empty with too few speeds.
pub fn speed_bands(samples: &[Sample]) -> Vec<f64> {
    let mut v: Vec<f64> = samples.iter().filter_map(|s| s.cond.speed).collect();
    if v.len() < 6 {
        return vec![];
    }
    v.sort_by(f64::total_cmp);
    let r = |x: f64| (x * 100.0).round() / 100.0;
    vec![r(v[v.len() / 3]), r(v[2 * v.len() / 3])]
}

/// `low`, `mid` or `high` for a speed (None without bands or a speed).
pub fn band_of(bands: &[f64], speed: Option<f64>) -> Option<&'static str> {
    let s = speed?;
    let [a, b] = <[f64; 2]>::try_from(bands).ok()?;
    Some(if s < a {
        "low"
    } else if s < b {
        "mid"
    } else {
        "high"
    })
}

// ------------------------------------------------------------------ the table

/// A move's measured effect: median, lowest and highest.
#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub name: String,
    pub median: f64,
    pub lo: f64,
    pub hi: f64,
}

/// One line of the table: a move under one set of conditions.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveRow {
    pub name: String,
    /// Input terms, frames counted from the move's first frame.
    pub terms: Vec<String>,
    pub frames: u64,
    /// `airborne=0`, `speed=mid`, `screen=racing`; empty is any.
    pub cond: Vec<(String, String)>,
    /// Frames measured from the move's start.
    pub window: u64,
    pub effects: Vec<Effect>,
    pub samples: usize,
}

impl MoveRow {
    pub fn effect(&self, name: &str) -> Option<&Effect> {
        self.effects.iter().find(|e| e.name == name)
    }

    pub fn cond_text(&self) -> String {
        if self.cond.is_empty() {
            "any".to_string()
        } else {
            self.cond.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(",")
        }
    }

    /// Whether the row's conditions are all among `have`.
    pub fn matches(&self, have: &[(String, String)]) -> bool {
        self.cond.iter().all(|c| have.contains(c))
    }
}

/// The parsed table: its settings and its rows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    /// `@speed-bands A B`.
    pub speed_bands: Vec<f64>,
    pub rows: Vec<MoveRow>,
}

impl Table {
    /// Distinct move names, in file order.
    pub fn names(&self) -> Vec<&str> {
        let mut n: Vec<&str> = Vec::new();
        for r in &self.rows {
            if !n.contains(&r.name.as_str()) {
                n.push(&r.name);
            }
        }
        n
    }

    /// A move's first row (its inputs are the same on every row).
    pub fn find(&self, name: &str) -> Option<&MoveRow> {
        self.rows.iter().find(|r| r.name == name)
    }

    /// The row to predict a move's effect under `have`: the row with the most
    /// conditions that all hold (an exact one when there is one), else the
    /// unconditional row.
    pub fn best(&self, name: &str, have: &[(String, String)]) -> Option<&MoveRow> {
        self.rows.iter().filter(|r| r.name == name && r.matches(have)).max_by_key(|r| r.cond.len())
    }
}

fn r3(x: f64) -> f64 {
    let y = (x * 1000.0).round() / 1000.0;
    if y == 0.0 {
        0.0
    } else {
        y
    }
}

impl MoveRow {
    fn line(&self) -> String {
        let fx: Vec<String> = std::iter::once(format!("w={}", self.window))
            .chain(self.effects.iter().map(|e| format!("{}={}[{}..{}]", e.name, r3(e.median), r3(e.lo), r3(e.hi))))
            .collect();
        format!(
            "{}|{} over {} frames|{}|{}|{}",
            self.name,
            self.terms.join(","),
            self.frames,
            self.cond_text(),
            fx.join(";"),
            self.samples
        )
    }
}

impl Table {
    pub fn to_text(&self, header: &str) -> String {
        let mut s = String::new();
        for l in header.lines() {
            s.push_str(&format!("# {l}\n"));
        }
        if self.speed_bands.len() == 2 {
            s.push_str(&format!("@speed-bands {} {}\n", self.speed_bands[0], self.speed_bands[1]));
        }
        for r in &self.rows {
            s.push_str(&r.line());
            s.push('\n');
        }
        s
    }
}

fn name_ok(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '+'))
}

/// The table in `text`: `#` comments, `@speed-bands A B`, and lines of
/// `NAME|TERMS over N frames|CONDITIONS|EFFECTS|SAMPLES`.
pub fn parse_table(text: &str) -> Result<Table, String> {
    let mut t = Table::default();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let at = |m: &str| format!("moves:{}: {m}", n + 1);
        if let Some(rest) = line.strip_prefix('@') {
            let w: Vec<&str> = rest.split_whitespace().collect();
            match w[..] {
                ["speed-bands", a, b] => {
                    let (a, b) = (a.parse::<f64>(), b.parse::<f64>());
                    let (Ok(a), Ok(b)) = (a, b) else { return Err(at("@speed-bands wants two numbers")) };
                    t.speed_bands = vec![a, b];
                }
                _ => return Err(at(&format!("unknown setting `@{rest}`"))),
            }
            continue;
        }
        let want = "want NAME|TERMS over N frames|CONDITIONS|EFFECTS|SAMPLES";
        let p: Vec<&str> = line.split('|').collect();
        let [name, inputs, cond, fx, samples] = p[..] else { return Err(at(want)) };
        if !name_ok(name) {
            return Err(at(&format!("move name {name:?}: letters, digits, - _ and +")));
        }
        let (terms, frames) = inputs.split_once(" over ").ok_or_else(|| at(want))?;
        let frames: u64 = frames
            .strip_suffix(" frames")
            .and_then(|f| f.trim().parse().ok())
            .filter(|f| *f > 0)
            .ok_or_else(|| at("INPUTS end with `over N frames`"))?;
        let terms: Vec<String> = terms.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        if terms.iter().any(|t| !t.contains('@')) || terms.is_empty() {
            return Err(at("INPUTS are terms with frames, like curving=0xf0@0-19"));
        }
        let cond: Vec<(String, String)> = if cond.trim() == "any" {
            vec![]
        } else {
            cond.split(',')
                .map(|c| c.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
                .collect::<Option<_>>()
                .ok_or_else(|| at("CONDITIONS are `any` or NAME=VALUE,..."))?
        };
        let (mut window, mut effects) = (0u64, Vec::new());
        for f in fx.split(';').filter(|f| !f.is_empty()) {
            let (k, v) = f.split_once('=').ok_or_else(|| at("EFFECTS are NAME=MEDIAN[LO..HI];..."))?;
            if k == "w" {
                window = v.parse().map_err(|_| at("w= is a frame count"))?;
                continue;
            }
            let bad = || at(&format!("effect `{f}`: want NAME=MEDIAN[LO..HI]"));
            let (m, rest) = v.split_once('[').ok_or_else(bad)?;
            let (lo, hi) = rest.strip_suffix(']').and_then(|r| r.split_once("..")).ok_or_else(bad)?;
            let num = |s: &str| s.parse::<f64>().ok().filter(|x| x.is_finite());
            effects.push(Effect {
                name: k.to_string(),
                median: num(m).ok_or_else(bad)?,
                lo: num(lo).ok_or_else(bad)?,
                hi: num(hi).ok_or_else(bad)?,
            });
        }
        let samples: usize = samples.trim().parse().map_err(|_| at("SAMPLES is a count"))?;
        if let Some(first) = t.rows.iter().find(|r| r.name == name) {
            if first.terms != terms || first.frames != frames {
                return Err(at(&format!("move {name} has two different INPUTS")));
            }
        }
        t.rows.push(MoveRow { name: name.into(), terms, frames, cond, window, effects, samples });
    }
    Ok(t)
}

/// The game's shipped table (empty where there is none).
pub fn shipped(game: &Game) -> Result<Table, String> {
    parse_table(romset::games::moves(game)).map_err(|e| format!("{} {e}", game.id))
}

// ------------------------------------------------------------------ aggregation

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// What aggregating dropped.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dropped {
    /// Moves seen, and those kept.
    pub moves: usize,
    pub kept: usize,
    /// Condition rows left out for having too few samples.
    pub rows: usize,
    pub samples_of_dropped_moves: usize,
}

fn effect_rows(samples: &[&Sample]) -> Vec<Effect> {
    let mut out = Vec::new();
    for (i, name) in EFFECTS.iter().enumerate() {
        let mut v: Vec<f64> = samples.iter().filter_map(|s| s.fx[i]).collect();
        if v.is_empty() {
            continue;
        }
        let (lo, hi) =
            (v.iter().copied().fold(f64::INFINITY, f64::min), v.iter().copied().fold(f64::NEG_INFINITY, f64::max));
        out.push(Effect { name: name.to_string(), median: median(&mut v), lo, hi });
    }
    out
}

fn part_term(p: &Part, off: u64, len: u64, value: Option<u8>) -> String {
    let what = match value {
        Some(v) => format!("{}={:#04x}", p.name, v),
        None => p.name.clone(),
    };
    if len <= 1 {
        format!("{what}@{off}")
    } else {
        format!("{what}@{off}-{}", off + len - 1)
    }
}

/// The table: each move that has at least `min` samples, as one row for
/// all its samples and one per (airborne, speed band, screen) with at least
/// `min` of its own.
pub fn aggregate(samples: &[Sample], bands: &[f64], min: usize) -> (Table, Dropped) {
    let mut by_key: BTreeMap<&str, Vec<&Sample>> = BTreeMap::new();
    for s in samples {
        by_key.entry(&s.key).or_default().push(s);
    }
    let mut keys: Vec<(&str, Vec<&Sample>)> = by_key.into_iter().collect();
    keys.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));
    let mut table = Table { speed_bands: bands.to_vec(), rows: Vec::new() };
    let mut drop = Dropped { moves: keys.len(), ..Dropped::default() };
    for (key, ss) in keys {
        if ss.len() < min {
            drop.samples_of_dropped_moves += ss.len();
            continue;
        }
        drop.kept += 1;
        // The canonical inputs: each part's median value, offset and length.
        let first = &ss[0].parts;
        let (mut terms, mut frames) = (Vec::new(), 1);
        for (i, p0) in first.iter().enumerate() {
            let col = |f: &dyn Fn(&Part) -> u64| {
                let mut v: Vec<f64> = ss.iter().map(|s| f(&s.parts[i]) as f64).collect();
                median(&mut v).round() as u64
            };
            let off = col(&|p| p.off);
            let len = p0.fixed_len.unwrap_or_else(|| col(&|p| p.len));
            let value = p0.value.map(|_| col(&|p| p.value.unwrap_or(0) as u64) as u8);
            frames = frames.max(off + len);
            terms.push(part_term(p0, off, len, value));
        }
        let window = {
            let mut w: Vec<f64> = ss.iter().map(|s| s.window as f64).collect();
            median(&mut w).round() as u64
        };
        let mk = |cond: Vec<(String, String)>, set: &[&Sample]| MoveRow {
            name: key.to_string(),
            terms: terms.clone(),
            frames,
            cond,
            window,
            effects: effect_rows(set),
            samples: set.len(),
        };
        table.rows.push(mk(vec![], &ss));
        // Rows for every combination of the conditions (airborne, speed band, and
        // the screen if the samples span more than one), each with at least `min`
        // samples of its own and a set of samples no less specific row already has.
        let screens: std::collections::BTreeSet<&str> = ss.iter().map(|s| s.cond.screen.as_str()).collect();
        let keys: Vec<&str> =
            ["airborne", "speed", "screen"].into_iter().filter(|k| *k != "screen" || screens.len() > 1).collect();
        let full: Vec<Vec<(String, String)>> = ss.iter().map(|s| cond_of(&s.cond, bands)).collect();
        let mut masks: Vec<usize> = (1..(1usize << keys.len())).collect();
        masks.sort_by_key(|m| (m.count_ones(), *m));
        let mut emitted: Vec<Vec<usize>> = vec![(0..ss.len()).collect()];
        for mask in masks {
            let mut groups: BTreeMap<Vec<(String, String)>, Vec<usize>> = BTreeMap::new();
            for (i, f) in full.iter().enumerate() {
                let pairs: Option<Vec<(String, String)>> = keys
                    .iter()
                    .enumerate()
                    .filter(|(b, _)| mask >> b & 1 == 1)
                    .map(|(_, k)| f.iter().find(|(n, _)| n == k).cloned())
                    .collect();
                if let Some(p) = pairs {
                    groups.entry(p).or_default().push(i);
                }
            }
            for (cond, idx) in groups {
                if idx.len() < min {
                    drop.rows += 1;
                } else if !emitted.contains(&idx) {
                    let set: Vec<&Sample> = idx.iter().map(|i| ss[*i]).collect();
                    table.rows.push(mk(cond, &set));
                    emitted.push(idx);
                }
            }
        }
    }
    (table, drop)
}

/// The condition pairs a sample's start has.
pub fn cond_of(c: &Cond, bands: &[f64]) -> Vec<(String, String)> {
    let mut v = Vec::new();
    if let Some(a) = c.airborne {
        v.push(("airborne".to_string(), (a as u8).to_string()));
    }
    if let Some(b) = band_of(bands, c.speed) {
        v.push(("speed".to_string(), b.to_string()));
    }
    if !c.screen.is_empty() {
        v.push(("screen".to_string(), c.screen.clone()));
    }
    v
}

// ------------------------------------------------------------------ measuring

/// What one frame looked like, read from the value table.
#[derive(Clone, Copy, Debug, Default)]
struct Obs {
    screen: Option<f64>,
    speed: Option<f64>,
    hx: Option<f64>,
    hz: Option<f64>,
    air: Option<f64>,
    y: Option<f64>,
    score: Option<f64>,
    px: Option<f64>,
    pz: Option<f64>,
    vx: Option<f64>,
    vz: Option<f64>,
}

struct Reader {
    defs: [Option<ValueDef>; 11],
}

impl Reader {
    fn new(table: &[ValueDef]) -> Reader {
        let f = |n: &str| table.iter().find(|d| d.name == n).cloned();
        Reader {
            defs: [
                f(V_SCREEN),
                f(V_SPEED),
                f(V_HEADING_X),
                f(V_HEADING_Z),
                f(V_AIRBORNE),
                f(V_HEIGHT),
                f(V_SCORE),
                f(V_POS_X),
                f(V_POS_Z),
                f(V_VEL_X),
                f(V_VEL_Z),
            ],
        }
    }

    fn read(&self, m: &Machine) -> Obs {
        let g = |i: usize| {
            let d = self.defs[i].as_ref()?;
            m.bus.peek(d.addr, d.width).and_then(|raw| crate::ram::decode(d, raw).as_f64())
        };
        Obs {
            screen: g(0),
            speed: g(1),
            hx: g(2),
            hz: g(3),
            air: g(4),
            y: g(5),
            score: g(6),
            px: g(7),
            pz: g(8),
            vx: g(9),
            vz: g(10),
        }
    }
}

/// Whether a window is on the race: the screen value reads the race's, with
/// a few frames of 0 (the game's mode word drops to 0 now and then, and
/// while a section loads) but never another screen.
fn on_race(w: &[Obs], race: f64) -> bool {
    let zeros = w.iter().filter(|o| o.screen == Some(0.0)).count();
    w.iter().all(|o| o.screen == Some(race) || o.screen == Some(0.0)) && zeros <= (w.len() / 10).max(3)
}

fn yaw(o: &Obs) -> Option<f64> {
    Some(o.hx?.atan2(o.hz?).to_degrees())
}

fn path(o: &Obs) -> Option<f64> {
    // Standing still has no direction of travel.
    let (x, z) = (o.vx?, o.vz?);
    (x.abs() + z.abs() > 1e-3).then(|| x.atan2(z).to_degrees())
}

fn turn(p: Option<f64>, q: Option<f64>) -> Option<f64> {
    let mut d = p? - q?;
    while d > 180.0 {
        d -= 360.0;
    }
    while d < -180.0 {
        d += 360.0;
    }
    Some(d)
}

/// The effects of the move: `a` (the recording, with it) against `b`
/// (without), both from the move's first frame, `window + 1` observations.
/// Where run `a` ended against run `b`, along the direction of travel at the
/// start (forward) and across it (`dlat`: towards +x when the travel is along -z): None if the skater was still.
fn along(start: Obs, a: &Obs, b: &Obs) -> Option<(f64, f64)> {
    let (vx, vz) = (start.vx?, start.vz?);
    let n = vx.hypot(vz);
    if n < 1e-3 {
        return None;
    }
    let (fx, fz) = (vx / n, vz / n);
    let (dx, dz) = (a.px? - b.px?, a.pz? - b.pz?);
    Some((dx * fx + dz * fz, dx * -fz + dz * fx))
}

fn effects(a: &[Obs], b: &[Obs]) -> [Option<f64>; 8] {
    let (la, lb) = (a.last().copied().unwrap_or_default(), b.last().copied().unwrap_or_default());
    let sub = |x: Option<f64>, y: Option<f64>| Some(x? - y?);
    let air = |o: &[Obs]| -> Option<f64> {
        o.iter()
            .skip(1)
            .map(|x| x.air)
            .collect::<Option<Vec<f64>>>()
            .map(|v| v.iter().filter(|a| **a > 0.0).count() as f64)
    };
    let top = |o: &[Obs]| -> Option<f64> {
        o.iter().map(|x| x.y).collect::<Option<Vec<f64>>>().map(|v| v.iter().copied().fold(f64::NEG_INFINITY, f64::max))
    };
    [
        sub(la.speed, lb.speed),
        turn(yaw(&la), yaw(&lb)),
        turn(path(&la), path(&lb)),
        sub(air(a), air(b)),
        sub(top(a), top(b)),
        sub(la.score, lb.score),
        along(a[0], &la, &lb).map(|d| d.0),
        along(a[0], &la, &lb).map(|d| d.1),
    ]
}

/// What a measuring pass did.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub recordings: usize,
    pub frames_replayed: u64,
    pub clusters: usize,
    pub measured: usize,
    pub skipped_long: usize,
    pub skipped_off_race: usize,
    pub skipped_window: usize,
    pub seconds: f64,
}

impl Stats {
    fn add(&mut self, o: &Stats) {
        self.recordings += o.recordings;
        self.frames_replayed += o.frames_replayed;
        self.clusters += o.clusters;
        self.measured += o.measured;
        self.skipped_long += o.skipped_long;
        self.skipped_off_race += o.skipped_off_race;
        self.skipped_window += o.skipped_window;
        self.seconds += o.seconds;
    }
}

/// What a measuring pass is told.
#[derive(Clone, Debug)]
pub struct Plan {
    /// The named screen the race is on (a state of the `screen` value).
    pub screen: String,
    pub tail: u64,
    pub spines: PathBuf,
}

fn run_frame(m: &mut Machine) -> Result<(), String> {
    let before = m.frames;
    while m.frames == before {
        m.run_slice(u64::MAX / 2).map_err(|e| format!("the machine stopped at frame {}: {e:?}", m.frames))?;
    }
    m.bus.uart_log.clear();
    Ok(())
}

/// Measure every move cluster of recording `label` that starts on the race
/// screen. One pass from the spine checkpoint before the race; at each
/// cluster's first frame a copy of the machine runs the window without the
/// cluster's inputs.
pub fn measure(
    roms: &RomSet,
    tours: &[Tour],
    label: &str,
    table: &[ValueDef],
    states: &[tour::StateDef],
    plan: &Plan,
) -> Result<(Vec<Sample>, Stats), String> {
    let game = roms.game;
    let t0 = Instant::now();
    let t = tours.iter().find(|t| t.label == label).ok_or_else(|| format!("no tour {label}"))?;
    let Length::Frames(len) = t.length else {
        return Err(format!("tour {label}: a recording has a length in frames (like 19368f)"));
    };
    let race_state = states
        .iter()
        .find(|s| s.value == V_SCREEN && s.name == plan.screen)
        .ok_or_else(|| format!("the value table names no screen `{}`", plan.screen))?;
    let race = race_state.number as f64;
    let items = crate::session::tour_items(game, tours, label)?;
    let script = InputScript::parse(&items.join(","), &game.controls).map_err(|e| format!("tour {label}: {e}"))?;
    // The spine: checkpoints and the index (made once).
    let dir = crate::spine::dir_of(&plan.spines, label);
    let key = crate::spine::key(game, t, crate::spine::native());
    if crate::spine::meta(&dir, key).is_none() {
        crate::spine::build(roms, t, &script, &dir, crate::spine::CHECKPOINT_EVERY, crate::spine::INDEX_EVERY, table)?;
    }
    let index = crate::spine::load_index(&dir, key, table).ok_or("the spine has no index")?;
    let col = index.names.iter().position(|n| n == V_SCREEN).ok_or("the value table has no `screen`")?;
    let rows: Vec<u64> = index.rows.iter().filter(|(_, v)| v[col] == Some(race)).map(|(f, _)| *f).collect();
    let mut stats = Stats { recordings: 1, ..Stats::default() };
    let (Some(first), Some(last)) = (rows.first().copied(), rows.last().copied()) else {
        stats.seconds = t0.elapsed().as_secs_f64();
        return Ok((vec![], stats));
    };
    let end = len.min(last + 2 * index.every);
    let reader = Reader::new(table);
    let mut m = crate::spine::fresh_machine(roms);
    script.apply(0, &mut m.bus);
    let mut base = 0;
    if let Some((f, path)) = crate::spine::nearest(&dir, key, first) {
        let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        model2::load_session(&mut m, None, &data)?;
        base = f;
        script.apply(m.frames, &mut m.bus);
    }
    let rs = runs(&items);
    let (cl, long) = clusters(game, &rs);
    stats.skipped_long = long;
    stats.clusters = cl.len() + long;
    let mut scratch = Box::new(Machine::new(roms));
    scratch.configure_like(&mut m);
    let mut marks = CopyMarks::default();
    // (cluster, the window, observations without the cluster)
    let mut forks: Vec<(usize, u64, Vec<Obs>)> = Vec::new();
    let mut obs: Vec<Obs> = vec![reader.read(&m)];
    let mut next = cl.iter().position(|c| c.from > base).unwrap_or(cl.len());
    while m.frames < end {
        run_frame(&mut m)?;
        let f = m.frames;
        let o = reader.read(&m);
        obs.push(o);
        while next < cl.len() && cl[next].from < f {
            next += 1;
        }
        if next < cl.len() && cl[next].from == f {
            let c = &cl[next];
            let window = (c.len() + plan.tail).min(MAX_WINDOW);
            if o.screen != Some(race) {
                stats.skipped_off_race += 1;
            } else if f + window > end {
                stats.skipped_window += 1;
            } else {
                // The recording's terms that reach the window, less the cluster's.
                let drop: Vec<usize> = c.runs.iter().map(|i| rs[*i].item).collect();
                let keep: Vec<String> = items
                    .iter()
                    .enumerate()
                    .filter(|(i, it)| {
                        !drop.contains(i) && tour::item_range(it).is_none_or(|(a, b)| b >= f && a <= f + window)
                    })
                    .map(|(_, it)| it.clone())
                    .collect();
                let sb = InputScript::parse(&keep.join(","), &game.controls)?;
                m.bus.geo.sync();
                scratch.bus.geo.sync();
                scratch.copy_from(&mut m, &mut marks);
                let mut b = vec![reader.read(&scratch)];
                for k in 0..window {
                    sb.apply(f + k, &mut scratch.bus);
                    run_frame(&mut scratch)?;
                    b.push(reader.read(&scratch));
                }
                forks.push((next, window, b));
            }
            next += 1;
        }
        script.apply(f, &mut m.bus);
    }
    stats.frames_replayed = end - base;
    let mut out = Vec::new();
    for (ci, window, b) in forks {
        let c = &cl[ci];
        let i0 = (c.from - base) as usize;
        let a = &obs[i0..=i0 + window as usize];
        if !on_race(a, race) || !on_race(&b, race) {
            stats.skipped_off_race += 1;
            continue;
        }
        stats.measured += 1;
        out.push(Sample {
            label: label.to_string(),
            key: c.key(),
            at: c.from,
            window,
            parts: c.parts.clone(),
            cond: Cond { airborne: a[0].air.map(|x| x > 0.0), speed: a[0].speed, screen: plan.screen.clone() },
            fx: effects(a, &b),
        });
    }
    stats.seconds = t0.elapsed().as_secs_f64();
    Ok((out, stats))
}

fn samples_tsv(samples: &[Sample]) -> String {
    let mut s = String::from(
        "recording\tmove\tframe\twindow\tairborne\tspeed\tdspeed\tdyaw\tdpath\tdair\tdheight\tdscore\tdfwd\tdlat\n",
    );
    let f = |x: Option<f64>| x.map_or("-".to_string(), |x| format!("{}", r3(x)));
    for x in samples {
        s.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            x.label,
            x.key,
            x.at,
            x.window,
            x.cond.airborne.map_or("-".to_string(), |a| (a as u8).to_string()),
            f(x.cond.speed),
            x.fx.iter().map(|e| f(*e)).collect::<Vec<_>>().join("\t")
        ));
    }
    s
}

// ------------------------------------------------------------------ the commands

pub const USAGE: &str = "\
remodel moves learn [GAME] --tours FILE... [--label L]... [--skip L]... [--out FILE] [--samples FILE]
                    [--min-samples N] [--screen NAME] [--tail N] [--dir DIR] [--json]
remodel moves check [GAME] --tours FILE... [--table FILE] [--label L]... [--skip L]... [--samples FILE] [--json]
  `learn` cuts the recordings' race stretches into the input spans that recur (a button tap or hold, an
  analog channel held at a strength for a duration, and such inputs that overlap), measures what each did
  by replaying a window from a spine checkpoint with and without it, tags each sample (airborne, speed
  band, screen) and writes the move table (--out, default out/GAME/moves.txt; the game's shipped one is
  crates/romset/src/games/moves/GAME.txt). `check` measures the moves found in the recordings and compares
  them with the table's predictions. FILEs are tour files (--tours, repeatable; names after it up to the
  next option are files too); every tour of a file with a length in frames is a recording unless --label
  names some or --skip drops some. --screen is the value table's named screen the race is on (default
  racing); --tail frames are measured after a move ends (default 60); --min-samples N (default 3) is the
  fewest samples a move or a condition row needs. Needs the ROM; run it through tools/capped.
";

struct Args {
    game: Option<String>,
    files: Vec<PathBuf>,
    labels: Vec<String>,
    skip: Vec<String>,
    out: Option<PathBuf>,
    samples: Option<PathBuf>,
    table: Option<PathBuf>,
    min: usize,
    screen: String,
    tail: u64,
    dir: Option<PathBuf>,
}

fn parse_args(args: &[String]) -> Result<(String, Args), String> {
    let mut a = Args {
        game: None,
        files: vec![],
        labels: vec![],
        skip: vec![],
        out: None,
        samples: None,
        table: None,
        min: MIN_SAMPLES,
        screen: "racing".into(),
        tail: TAIL,
        dir: None,
    };
    let mut verb = None;
    let mut it = args.iter().peekable();
    while let Some(x) = it.next() {
        let mut val = |n: &str| it.next().cloned().ok_or_else(|| format!("{n} wants a value"));
        match x.as_str() {
            "--tours" => {
                a.files.push(PathBuf::from(val("--tours")?));
                while let Some(f) = it.next_if(|f| !f.starts_with("--")) {
                    a.files.push(PathBuf::from(f));
                }
            }
            "--label" => a.labels.push(val("--label")?),
            "--skip" => a.skip.push(val("--skip")?),
            "--out" => a.out = Some(PathBuf::from(val("--out")?)),
            "--samples" => a.samples = Some(PathBuf::from(val("--samples")?)),
            "--table" => a.table = Some(PathBuf::from(val("--table")?)),
            "--dir" => a.dir = Some(PathBuf::from(val("--dir")?)),
            "--screen" => a.screen = val("--screen")?,
            "--min-samples" => {
                a.min = val("--min-samples")?
                    .parse()
                    .ok()
                    .filter(|n| *n >= 1)
                    .ok_or("--min-samples wants a number from 1")?
            }
            "--tail" => a.tail = val("--tail")?.parse().map_err(|_| "--tail wants a number of frames")?,
            f if f.starts_with("--") => return Err(format!("moves: unknown option {f}")),
            w if verb.is_none() => verb = Some(w.to_string()),
            w if a.game.is_none() => a.game = Some(w.to_string()),
            w => return Err(format!("moves: unexpected `{w}`")),
        }
    }
    Ok((verb.ok_or("want: moves learn|check")?, a))
}

/// The recordings to measure: the labels asked for, else every tour of the
/// files that has a length in frames, less `skip`.
fn recordings(tours: &[Tour], files: &[Tour], a: &Args) -> Result<Vec<String>, String> {
    let mut labels: Vec<String> = if a.labels.is_empty() {
        files.iter().filter(|t| matches!(t.length, Length::Frames(_))).map(|t| t.label.clone()).collect()
    } else {
        a.labels.clone()
    };
    labels.retain(|l| !a.skip.contains(l));
    for l in &labels {
        if !tours.iter().any(|t| &t.label == l) {
            let known: Vec<String> = tours.iter().map(|t| t.label.clone()).collect();
            return Err(Answer::unknown("tour", l, &known).error.clone().unwrap_or_default());
        }
    }
    if labels.is_empty() {
        return Err("no recordings: give --tours FILE (tours with a length in frames) or --label".into());
    }
    Ok(labels)
}

/// `remodel moves learn|check`.
pub fn main(o: &crate::Opts) -> ExitCode {
    let mut args = o.args[1..].to_vec();
    let json = args.iter().position(|a| a == "--json").map(|i| args.remove(i)).is_some();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let a = run(o, &args);
    a.emit(json, || a.data["text"].as_str().unwrap_or("").to_string())
}

fn run(o: &crate::Opts, args: &[String]) -> Answer {
    let bad = |e: String| Answer::fail(Code::BadRequest, e).next("remodel moves --help");
    let (verb, a) = match parse_args(args) {
        Ok(p) => p,
        Err(e) => return bad(e),
    };
    if verb != "learn" && verb != "check" {
        return bad(format!("moves {verb}: want learn or check"));
    }
    let game = match romset::games::find(a.game.as_deref().unwrap_or(&o.game_id)) {
        Ok(g) => g,
        Err(e) => return bad(e.to_string()),
    };
    if a.files.is_empty() && a.labels.is_empty() {
        return bad("want --tours FILE".into());
    }
    for f in &a.files {
        if !f.is_file() {
            return bad(format!("--tours {}: no such file", f.display()));
        }
    }
    let all = match tour::load_paths(game, &a.files) {
        Ok(t) => t,
        Err(e) => return bad(e),
    };
    let own: Vec<Tour> = a
        .files
        .iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .filter_map(|t| tour::parse(&t, "tours").ok())
        .flatten()
        .collect();
    let labels = match recordings(&all, &own, &a) {
        Ok(l) => l,
        Err(e) => return bad(e),
    };
    let roms = match RomSet::load(game, &o.rom_dir, false) {
        Ok(r) => r,
        Err(_) => {
            return Answer::fail(
                Code::Missing,
                format!("no ROMs for {}: moves are measured by running the game", game.id),
            )
            .next(format!("put the {} ROMs in {}", game.id, o.rom_dir.display()))
        }
    };
    let values = romset::games::values(game);
    let (table, states) = match (tour::value_table(values), tour::state_table(values)) {
        (Ok(t), Ok(s)) if !t.is_empty() => (t, s),
        (Err(e), _) | (_, Err(e)) => return bad(format!("{} {e}", game.id)),
        _ => return bad(format!("{} has no value table: moves are measured on named values", game.id)),
    };
    let plan = Plan {
        screen: a.screen.clone(),
        tail: a.tail,
        spines: a.dir.clone().unwrap_or_else(|| crate::spine::default_dir(game.id)),
    };
    let t0 = Instant::now();
    let (mut samples, mut stats) = (Vec::new(), Stats::default());
    for l in &labels {
        match measure(&roms, &all, l, &table, &states, &plan) {
            Ok((s, st)) => {
                eprintln!(
                    "moves: {l}: {} clusters, {} measured, {} frames replayed, {:.1} s",
                    st.clusters, st.measured, st.frames_replayed, st.seconds
                );
                samples.extend(s);
                stats.add(&st);
            }
            Err(e) => return Answer::fail(Code::Internal, format!("recording {l}: {e}")),
        }
    }
    if let Some(p) = &a.samples {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Err(e) = std::fs::write(p, samples_tsv(&samples)) {
            return Answer::fail(Code::Internal, format!("{}: {e}", p.display()));
        }
    }
    let total = t0.elapsed().as_secs_f64();
    if verb == "check" {
        return check(game, &a, &samples, &stats, total);
    }
    learn_result(game, &a, &labels, &samples, &stats, total)
}

fn stats_json(s: &Stats) -> Value {
    json!({
        "recordings": s.recordings, "frames_replayed": s.frames_replayed, "clusters": s.clusters,
        "measured": s.measured, "skipped_too_long": s.skipped_long, "skipped_off_race": s.skipped_off_race,
        "skipped_window": s.skipped_window,
    })
}

fn learn_result(game: &Game, a: &Args, labels: &[String], samples: &[Sample], stats: &Stats, secs: f64) -> Answer {
    let bands = speed_bands(samples);
    let (table, dropped) = aggregate(samples, &bands, a.min);
    let header = format!(
        "{}'s moves, learned by `remodel moves learn` from a person's own recordings ({}), not from the ROM.\n\
         NAME|INPUTS over N frames|conditions|effects (median[min..max] over the window w, from the move's first frame)|samples\n\
         Inputs are tour terms with frames counted from the move's first frame. Effects are the recording's own\n\
         run less a run with the move's inputs removed: dspeed (speed at the window's end), dyaw (heading, degrees),\n\
         dpath (direction of travel), dair (frames in the air), dheight (highest point), dscore (score), dfwd and dlat\n\
         (where the skater ended along and across its direction of travel at the start, units; dlat is towards +x when travelling along -z, the skater's left). Conditions: airborne=0|1,\n\
         speed=low|mid|high (cutoffs at @speed-bands), screen. Rows with fewer than {} samples are left out.",
        game.id,
        labels.join(", "),
        a.min
    );
    let text = table.to_text(&header);
    let out = a.out.clone().unwrap_or_else(|| PathBuf::from("out").join(game.id).join("moves.txt"));
    if let Some(d) = out.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Err(e) = std::fs::write(&out, &text) {
        return Answer::fail(Code::Internal, format!("{}: {e}", out.display()));
    }
    let mut top: Vec<(&str, usize)> =
        table.rows.iter().filter(|r| r.cond.is_empty()).map(|r| (r.name.as_str(), r.samples)).collect();
    top.sort_by_key(|t| std::cmp::Reverse(t.1));
    let report = format!(
        "{} recordings, {} frames replayed, {:.1} s\n{} move clusters, {} measured ({} too long, {} off the race, {} past the window)\n\
         {} moves seen, {} kept, {} dropped for under {} samples ({} samples), {} condition rows dropped\n{} rows in {}\n\
         top: {}\n",
        stats.recordings,
        stats.frames_replayed,
        secs,
        stats.clusters,
        stats.measured,
        stats.skipped_long,
        stats.skipped_off_race,
        stats.skipped_window,
        dropped.moves,
        dropped.kept,
        dropped.moves - dropped.kept,
        a.min,
        dropped.samples_of_dropped_moves,
        dropped.rows,
        table.rows.len(),
        out.display(),
        top.iter().take(8).map(|(n, c)| format!("{n} ({c})")).collect::<Vec<_>>().join(", ")
    );
    Answer::done(json!({
        "out": out.display().to_string(), "recordings": labels, "stats": stats_json(stats), "seconds": secs,
        "moves_seen": dropped.moves, "moves_kept": dropped.kept, "moves_dropped": dropped.moves - dropped.kept,
        "rows": table.rows.len(), "rows_dropped": dropped.rows, "speed_bands": bands,
        "top": top.iter().take(8).map(|(n, c)| json!({"move": n, "samples": c})).collect::<Vec<_>>(),
        "text": report,
    }))
}

/// Predicted against measured, per effect.
fn check(game: &Game, a: &Args, samples: &[Sample], stats: &Stats, secs: f64) -> Answer {
    let table = match &a.table {
        Some(p) => {
            std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display())).and_then(|t| parse_table(&t))
        }
        None => shipped(game),
    };
    let table = match table {
        Ok(t) => t,
        Err(e) => return Answer::fail(Code::BadRequest, e),
    };
    let mut per: Vec<Vec<(f64, f64, f64, bool)>> = vec![Vec::new(); EFFECTS.len()];
    let (mut predicted, mut unknown) = (0usize, 0usize);
    let mut unknown_names: BTreeMap<&str, usize> = BTreeMap::new();
    for s in samples {
        let have = cond_of(&s.cond, &table.speed_bands);
        let Some(row) = table.best(&s.key, &have) else {
            unknown += 1;
            *unknown_names.entry(&s.key).or_default() += 1;
            continue;
        };
        predicted += 1;
        for (i, name) in EFFECTS.iter().enumerate() {
            if let (Some(actual), Some(e)) = (s.fx[i], row.effect(name)) {
                per[i].push((e.median, actual, 0.0, actual >= e.lo && actual <= e.hi));
            }
        }
    }
    let mut text = format!(
        "{} samples measured in {} recordings ({:.1} s): {} predicted from the table, {} of moves the table does not have\n",
        samples.len(),
        stats.recordings,
        secs,
        predicted,
        unknown
    );
    let mut fx = Vec::new();
    text.push_str("effect     n   mean|actual|   MAE table   MAE zero   skill   in range\n");
    for (i, name) in EFFECTS.iter().enumerate() {
        let v = &per[i];
        if v.is_empty() {
            continue;
        }
        let n = v.len() as f64;
        let mean_abs = v.iter().map(|x| x.1.abs()).sum::<f64>() / n;
        let mae = v.iter().map(|x| (x.0 - x.1).abs()).sum::<f64>() / n;
        let mae0 = mean_abs;
        let inr = v.iter().filter(|x| x.3).count() as f64 / n;
        let skill = if mae0 > 0.0 { 1.0 - mae / mae0 } else { 0.0 };
        text.push_str(&format!(
            "{:<8} {:>5}   {:>10.3}   {:>9.3}   {:>8.3}   {:>5.2}   {:>6.0}%\n",
            name,
            v.len(),
            mean_abs,
            mae,
            mae0,
            skill,
            inr * 100.0
        ));
        fx.push(json!({"effect": name, "n": v.len(), "mean_abs_actual": r3(mean_abs), "mae_table": r3(mae), "mae_zero": r3(mae0), "skill": r3(skill), "in_range": r3(inr)}));
    }
    text.push_str("skill = 1 - MAE(table) / MAE(predicting no effect); in range = the measured effect lay inside the row's min..max\n");
    Answer::done(json!({
        "samples": samples.len(), "predicted": predicted, "unknown_moves": unknown,
        "unknown": unknown_names, "effects": fx, "stats": stats_json(stats), "seconds": secs, "text": text,
    }))
}

/// Where a loaded table goes for the server: the file given, else the game's.
pub fn load(game: &Game, file: Option<&Path>) -> Result<Table, String> {
    match file {
        Some(p) => parse_table(&std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?),
        None => shipped(game),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game() -> &'static Game {
        romset::games::find("topskatr").unwrap()
    }

    fn items(s: &str) -> Vec<String> {
        s.split(',').map(str::to_string).collect()
    }

    #[test]
    fn runs_come_from_terms_and_skip_pokes_and_repeats() {
        let r = runs(&items(
            "curving=0xf0@100-119,jumpfront@130-133,poke=hold-clock@1-9,punch@5-90/30,start@12,slide=0x80@3-4",
        ));
        assert_eq!(
            r,
            vec![
                Run { item: 0, name: "curving".into(), value: Some(0xf0), from: 100, to: 119 },
                Run { item: 1, name: "jumpfront".into(), value: None, from: 130, to: 133 },
                Run { item: 4, name: "start".into(), value: None, from: 12, to: 12 },
                Run { item: 5, name: "slide".into(), value: Some(0x80), from: 3, to: 4 },
            ]
        );
    }

    #[test]
    fn digital_holds_and_analog_strengths_are_bucketed() {
        assert_eq!([1, 6, 7, 14, 15, 40, 41].map(hold_class), ["tap", "tap", "short", "short", "hold", "hold", "long"]);
        assert_eq!([1, 7, 8, 14, 15, 28, 29, 93, 500].map(duration_bucket), [5, 5, 10, 10, 20, 20, 40, 80, 160]);
        let g = game();
        let a = &g.controls.analogs[0];
        assert_eq!(analog_class(a, 0xf0), ("hi", true), "0xf0 is full left");
        assert_eq!(analog_class(a, 0x10), ("hi", false), "0x10 is full right");
        assert_eq!(analog_class(a, 0xc0), ("mid", true));
        assert_eq!(analog_class(a, 0x90), ("lo", true));
    }

    #[test]
    fn overlapping_runs_are_one_cluster_with_a_readable_name() {
        let it = items("curving=0xf0@100-119,curving=0xf0@200-219,jumptail@205-208,jumpfront@300-303,curving=0x10@310-312,curving=0xf0@400-700");
        let (c, long) = clusters(game(), &runs(&it));
        assert_eq!(long, 1, "a 301-frame chain is driving, not a move");
        let keys: Vec<String> = c.iter().map(Cluster::key).collect();
        assert_eq!(keys, ["steer-left-hi-20f", "steer-left-hi-20f+jumptail-tap", "jumpfront-tap", "steer-right-hi-5f"]);
        assert_eq!((c[1].from, c[1].to, c[1].len()), (200, 219, 20));
        assert_eq!(c[1].parts[1].off, 5);
        // The same stream shifted in time is the same moves.
        let shifted: Vec<String> = items("curving=0xf0@1100-1119");
        assert_eq!(clusters(game(), &runs(&shifted)).0[0].key(), keys[0]);
    }

    fn sample(key: &str, speed: f64, air: bool, dspeed: f64, dyaw: f64) -> Sample {
        Sample {
            label: "t".into(),
            key: key.into(),
            at: 1,
            window: 80,
            parts: vec![Part {
                label: key.into(),
                name: "curving".into(),
                value: Some(0xf0),
                off: 0,
                len: 20,
                fixed_len: Some(20),
            }],
            cond: Cond { airborne: Some(air), speed: Some(speed), screen: "racing".into() },
            fx: [Some(dspeed), Some(dyaw), None, Some(0.0), None, Some(0.0), None, None],
        }
    }

    #[test]
    fn conditions_bucket_speeds_into_terciles() {
        let ss: Vec<Sample> = (0..9).map(|i| sample("m", 1.0 + i as f64, false, 0.0, 0.0)).collect();
        let b = speed_bands(&ss);
        assert_eq!(b, vec![4.0, 7.0]);
        assert_eq!([3.9, 4.0, 6.9, 7.0].map(|s| band_of(&b, Some(s)).unwrap()), ["low", "mid", "mid", "high"]);
        assert_eq!(band_of(&[], Some(3.0)), None);
        assert!(speed_bands(&ss[..3]).is_empty(), "too few speeds for bands");
        let c = cond_of(&ss[0].cond, &b);
        assert_eq!(
            c,
            vec![("airborne".into(), "0".into()), ("speed".into(), "low".into()), ("screen".into(), "racing".into())]
        );
    }

    #[test]
    fn aggregating_takes_medians_ranges_and_drops_thin_moves() {
        let mut ss = Vec::new();
        for i in 0..6 {
            ss.push(sample("steer-left-hi-20f", 1.0 + i as f64, false, i as f64, -8.0 - i as f64));
        }
        ss.push(sample("steer-left-hi-20f", 3.0, true, 100.0, 0.0));
        ss.push(sample("rare", 3.0, false, 1.0, 1.0));
        ss.push(sample("rare", 3.0, false, 1.0, 1.0));
        let bands = speed_bands(&ss);
        let (t, d) = aggregate(&ss, &bands, 3);
        assert_eq!((d.moves, d.kept), (2, 1), "{d:?}");
        assert_eq!(d.samples_of_dropped_moves, 2);
        let any = t.rows.iter().find(|r| r.cond.is_empty()).unwrap();
        assert_eq!(any.samples, 7);
        let e = any.effect("dspeed").unwrap();
        assert_eq!((e.median, e.lo, e.hi), (3.0, 0.0, 100.0));
        assert_eq!(any.terms, vec!["curving=0xf0@0-19"]);
        assert_eq!(any.frames, 20);
        assert!(any.effect("dheight").is_none(), "an effect never measured is not made up");
        // A condition row needs `min` samples of its own: one airborne sample is not a row.
        assert!(t.rows.iter().all(|r| r.cond.iter().all(|c| c != &("airborne".to_string(), "1".to_string()))));
        assert!(t.rows.iter().any(|r| r.cond.contains(&("airborne".to_string(), "0".to_string()))));
        assert!(d.rows >= 1);
        // The best row for a condition is the most specific that holds.
        let have = cond_of(&ss[0].cond, &bands);
        assert!(!t.best("steer-left-hi-20f", &have).unwrap().cond.is_empty());
        assert_eq!(t.best("steer-left-hi-20f", &[]).unwrap().cond.len(), 0);
        assert!(t.best("nope", &have).is_none());
    }

    #[test]
    fn the_table_round_trips_and_refuses_bad_lines() {
        let ss: Vec<Sample> =
            (0..8).map(|i| sample("steer-left-hi-20f", 1.0 + i as f64, i % 2 == 0, i as f64 * 0.25, -8.5)).collect();
        let bands = speed_bands(&ss);
        let (t, _) = aggregate(&ss, &bands, 3);
        let text = t.to_text("a header\nsecond line");
        assert!(text.starts_with("# a header\n# second line\n@speed-bands "));
        let back = parse_table(&text).unwrap();
        assert_eq!(back, t);
        assert_eq!(back.names(), vec!["steer-left-hi-20f"]);
        let one =
            "m-1|curving=0xf0@0-19,jumpfront@2-9 over 20 frames|airborne=0,speed=mid|w=80;dspeed=-0.12[-0.3..0.1]|11";
        let r = &parse_table(one).unwrap().rows[0];
        assert_eq!((r.terms.len(), r.frames, r.window, r.samples), (2, 20, 80, 11));
        assert_eq!(r.effect("dspeed").unwrap().lo, -0.3);
        assert_eq!(r.cond_text(), "airborne=0,speed=mid");
        for bad in [
            "x y|a@0 over 3 frames|any|w=1|1|z",
            "x y|a@0 over 3 frames|any|w=1;a=1[0..2]|2",
            "x|a@0 over 3 frames|any|w=1;dspeed=1|2",
            "x|a over 3 frames|any|w=1|2",
            "x|a@0 over 0 frames|any|w=1|2",
            "x|a@0 over 3 frames|a|w=1|2",
            "x|a@0 over 3 frames|any|w=1|many",
            "@nope 1",
            "x|a@0 over 3 frames|any|w=1|2\nx|b@0 over 3 frames|any|w=1|2",
        ] {
            assert!(parse_table(bad).is_err(), "{bad}");
        }
        assert!(parse_table("# only a comment\n\n").unwrap().rows.is_empty());
    }

    #[test]
    fn the_shipped_tables_parse_and_their_inputs_are_valid() {
        for g in romset::games::ALL {
            let t = shipped(g).unwrap_or_else(|e| panic!("{e}"));
            for r in &t.rows {
                let terms = r.terms.join(",");
                InputScript::parse(&terms, &g.controls).unwrap_or_else(|e| panic!("{} {}: {e}", g.id, r.name));
                for t in &r.terms {
                    let (_, hi) = tour::item_range(t).unwrap();
                    assert!(hi < r.frames, "{} {}: {t} runs past {} frames", g.id, r.name, r.frames);
                }
            }
        }
    }

    #[test]
    fn effects_are_the_difference_of_two_runs() {
        let o = |speed: f64, yaw_deg: f64, air: bool, y: f64, score: f64| Obs {
            screen: Some(7.0),
            speed: Some(speed),
            hx: Some(yaw_deg.to_radians().sin()),
            hz: Some(yaw_deg.to_radians().cos()),
            air: Some(air as u8 as f64),
            y: Some(y),
            score: Some(score),
            ..Obs::default()
        };
        let a = [
            o(3.0, 0.0, false, 0.0, 0.0),
            o(3.1, 5.0, true, 4.0, 0.0),
            o(3.2, 175.0, true, 9.0, 100.0),
            o(3.5, 170.0, false, 0.0, 100.0),
        ];
        let b = [
            o(3.0, 0.0, false, 0.0, 0.0),
            o(3.0, 0.0, false, 0.0, 0.0),
            o(3.0, -175.0, false, 0.0, 0.0),
            o(3.0, -170.0, false, 0.0, 0.0),
        ];
        let e = effects(&a, &b);
        assert!((e[0].unwrap() - 0.5).abs() < 1e-9);
        assert!((e[1].unwrap() - (-20.0)).abs() < 1e-6, "170 against -170 wraps to -20: {:?}", e[1]);
        assert_eq!(e[3], Some(2.0));
        assert_eq!(e[4], Some(9.0));
        assert_eq!(e[5], Some(100.0));
        assert_eq!(e[2], None, "no velocity, no direction of travel");
        // Along and across the direction of travel (forward is -z, across is +x).
        let at = |vx: f64, vz: f64, px: f64, pz: f64| Obs {
            vx: Some(vx),
            vz: Some(vz),
            px: Some(px),
            pz: Some(pz),
            ..Obs::default()
        };
        let (start, ea, eb) = (at(0.0, -2.0, 0.0, 0.0), at(0.0, -2.0, 2.0, -10.0), at(0.0, -2.0, 0.0, -8.0));
        let (fwd, lat) = along(start, &ea, &eb).unwrap();
        assert!((fwd - 2.0).abs() < 1e-9 && (lat - 2.0).abs() < 1e-9, "{fwd} {lat}");
        assert_eq!(along(at(0.0, 0.0, 0.0, 0.0), &ea, &eb), None);
    }

    #[test]
    fn arguments_take_files_after_tours_and_options() {
        let v = |s: &str| s.split_whitespace().map(str::to_string).collect::<Vec<_>>();
        let (verb, a) =
            parse_args(&v("learn topskatr --tours a.txt b.txt --out o.txt --min-samples 5 --label x --label y"))
                .unwrap();
        assert_eq!(verb, "learn");
        assert_eq!(a.game.as_deref(), Some("topskatr"));
        assert_eq!((a.files.len(), a.min, a.labels.len()), (2, 5, 2));
        assert!(parse_args(&v("learn --nope")).is_err());
        assert!(parse_args(&v("learn --min-samples 0")).is_err());
    }

    /// Measuring is deterministic, the effect of a jump is a jump, and a
    /// steer moves the skater sideways (skipped without the ROM).
    #[test]
    fn measuring_a_move_is_stable_and_a_jump_goes_up() {
        let g = game();
        let rom = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rom");
        let Ok(roms) = RomSet::load(g, &rom, false) else { return };
        let out =
            Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../out/scratch/moves-test-{}", std::process::id()));
        std::fs::create_dir_all(&out).unwrap();
        std::env::set_var("REMODEL_LOCK_DIR", out.join("locks"));
        let file = out.join("t.txt");
        // The game's own coin-start reaches the race; a hop and a steer in it.
        std::fs::write(&file, "mv|3800f|coin-start,jumpfront@3500-3504,curving=0xf0@3650-3669\n").unwrap();
        let tours = tour::load_paths(g, std::slice::from_ref(&file)).unwrap();
        let values = romset::games::values(g);
        let (table, states) = (tour::value_table(values).unwrap(), tour::state_table(values).unwrap());
        let plan = Plan { screen: "racing".into(), tail: TAIL, spines: out.join("spines") };
        let (a, stats) = measure(&roms, &tours, "mv", &table, &states, &plan).unwrap();
        let (b, _) = measure(&roms, &tours, "mv", &table, &states, &plan).unwrap();
        assert_eq!(a, b, "the same recording measures the same");
        assert_eq!(stats.measured, 2, "{stats:?}");
        let jump = a.iter().find(|s| s.key == "jumpfront-tap").expect("the hop is a move");
        assert!(jump.fx[3].unwrap() >= 10.0, "a hop is some frames in the air: {:?}", jump.fx);
        assert!(jump.fx[4].unwrap() > 5.0, "and some height: {:?}", jump.fx);
        assert_eq!(jump.cond.airborne, Some(false));
        let steer = a.iter().find(|s| s.key == "steer-left-hi-20f").expect("the steer is a move");
        assert!(steer.fx[7].unwrap().abs() > 1.0, "steering moves the skater sideways: {:?}", steer.fx);
        assert_eq!(steer.fx[3], Some(0.0), "and does not jump");
        assert!(steer.window >= 20 + TAIL - 1 && steer.at == 3650, "{steer:?}");
        // Leaving the move in changes nothing about a recording with no moves.
        std::fs::write(&file, "mv|3800f|coin-start\n").unwrap();
        let tours = tour::load_paths(g, std::slice::from_ref(&file)).unwrap();
        let none = Plan { spines: out.join("spines2"), ..plan };
        assert!(measure(&roms, &tours, "mv", &table, &states, &none).unwrap().0.is_empty());
        let _ = std::fs::remove_dir_all(&out);
    }
}
