//! Spines: a person's recording (a tour from power-on, `--record-tour`)
//! made quick to open at any frame, and searchable by what the game was
//! doing (docs/uplift/scenes-and-server.md, "Exploring from a recording").
//!
//! One replay from power-on makes two things, cached under
//! `out/GAME/spines/LABEL/` and keyed by the tour's line (and its `from=`
//! parents' lines), so a changed recording drops them:
//!
//! - **checkpoints**: a machine state every N frames (`ckpt-FRAME.state`),
//!   so `session.open from: tour:LABEL@F` loads the nearest one at or
//!   before F and steps the rest, not the whole run from power-on;
//! - **an index** (`index.tsv`): the game's named values (its value table,
//!   `crates/romset/src/games/values/`) every K frames, so an agent can ask
//!   for the frames where the skater was nearest a point, or the stretches
//!   on a screen, and open a session there.
//!
//! Nothing here knows a game: the values come from its table.

use crate::tour::{self, Length, Tour, ValueDef};
use model2::input::InputScript;
use model2::Machine;
use romset::{Game, RomSet};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// A checkpoint every this many frames (about 1.2 MB each; a Novice run of
/// 19,000 frames keeps 32). Opening steps at most this many frames.
pub const CHECKPOINT_EVERY: u64 = 600;
/// A row of the index every this many frames.
pub const INDEX_EVERY: u64 = 30;

/// Bumps when the files' layout changes.
const FORMAT: u32 = 1;

/// Where a game's spines go: `<out>/spines`.
pub fn default_dir(game: &str) -> PathBuf {
    crate::OUT_DIR.get().cloned().unwrap_or_else(|| PathBuf::from("out").join(game)).join("spines")
}

fn fnv(s: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in s.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// What the cache of tour `t` is valid for: the game, the tour's line and
/// its parents', the file layout, the geometrizer the states were made
/// with, and the build (a change to the emulator can change every state
/// after it, so another build makes its own checkpoints).
pub fn key(game: &Game, t: &Tour, native: bool) -> u64 {
    fnv(&format!(
        "{FORMAT}\n{}\n{}\n{}\n{}",
        remodel_version::VERSION,
        game.id,
        if native { "native" } else { "chip" },
        t.spec
    ))
}

fn table_key(table: &[ValueDef]) -> u64 {
    fnv(&table
        .iter()
        .map(|d| format!("{}:{:x}:{}:{:?}", d.name, d.addr, d.width, d.kind))
        .collect::<Vec<_>>()
        .join(","))
}

/// A spine's folder.
pub fn dir_of(root: &Path, label: &str) -> PathBuf {
    root.join(label)
}

fn ckpt_path(dir: &Path, frame: u64) -> PathBuf {
    dir.join(format!("ckpt-{frame:07}.state"))
}

// ------------------------------------------------------------------ the cache

/// What `meta.txt` says about a spine's cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meta {
    pub key: u64,
    pub every: u64,
    pub frames: u64,
    pub checkpoints: Vec<u64>,
}

impl Meta {
    fn text(&self) -> String {
        format!(
            "key={:016x}\nevery={}\nframes={}\ncheckpoints={}\n",
            self.key,
            self.every,
            self.frames,
            self.checkpoints.iter().map(u64::to_string).collect::<Vec<_>>().join(",")
        )
    }

    pub fn parse(text: &str) -> Option<Meta> {
        let get = |k: &str| text.lines().find_map(|l| l.strip_prefix(k).and_then(|r| r.strip_prefix('=')));
        Some(Meta {
            key: u64::from_str_radix(get("key")?, 16).ok()?,
            every: get("every")?.parse().ok()?,
            frames: get("frames")?.parse().ok()?,
            checkpoints: get("checkpoints")?
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s.parse().ok())
                .collect::<Option<Vec<u64>>>()?,
        })
    }
}

/// The checkpoints of spine `dir` if they are those of `key` and every
/// file is there.
pub fn meta(dir: &Path, key: u64) -> Option<Meta> {
    let m = Meta::parse(&std::fs::read_to_string(dir.join("meta.txt")).ok()?)?;
    (m.key == key && m.checkpoints.iter().all(|f| ckpt_path(dir, *f).is_file())).then_some(m)
}

/// The latest checkpoint at or before `frame`: its frame and file.
pub fn nearest(dir: &Path, key: u64, frame: u64) -> Option<(u64, PathBuf)> {
    let m = meta(dir, key)?;
    let f = m.checkpoints.iter().copied().filter(|f| *f <= frame).max()?;
    Some((f, ckpt_path(dir, f)))
}

// ------------------------------------------------------------------ the index

/// The named values every `every` frames along a spine.
#[derive(Clone, Debug, PartialEq)]
pub struct Index {
    pub key: u64,
    pub every: u64,
    pub frames: u64,
    pub table: u64,
    pub names: Vec<String>,
    /// Frame, and each named value then (None: not readable or not finite).
    pub rows: Vec<(u64, Vec<Option<f64>>)>,
}

fn num_text(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("{x}"),
        None => "-".to_string(),
    }
}

impl Index {
    pub fn to_text(&self) -> String {
        let mut s = format!(
            "# reModel spine index, format {FORMAT}\n# key={:016x} every={} frames={} table={:016x}\nframe\t{}\n",
            self.key,
            self.every,
            self.frames,
            self.table,
            self.names.join("\t")
        );
        for (f, vals) in &self.rows {
            s.push_str(&f.to_string());
            for v in vals {
                s.push('\t');
                s.push_str(&num_text(*v));
            }
            s.push('\n');
        }
        s
    }

    pub fn parse(text: &str) -> Result<Index, String> {
        let mut lines = text.lines();
        let _title = lines.next().ok_or("empty index")?;
        let head = lines.next().and_then(|l| l.strip_prefix("# ")).ok_or("no header line")?;
        let field = |k: &str| head.split_whitespace().find_map(|w| w.strip_prefix(k).and_then(|r| r.strip_prefix('=')));
        let bad = || "bad header".to_string();
        let key = u64::from_str_radix(field("key").ok_or_else(bad)?, 16).map_err(|_| bad())?;
        let every = field("every").ok_or_else(bad)?.parse().map_err(|_| bad())?;
        let frames = field("frames").ok_or_else(bad)?.parse().map_err(|_| bad())?;
        let table = u64::from_str_radix(field("table").ok_or_else(bad)?, 16).map_err(|_| bad())?;
        let cols = lines.next().ok_or("no columns")?;
        let names: Vec<String> = cols.split('\t').skip(1).map(str::to_string).collect();
        let mut rows = Vec::new();
        for (n, l) in lines.enumerate() {
            let mut p = l.split('\t');
            let f: u64 = p.next().and_then(|f| f.parse().ok()).ok_or_else(|| format!("row {}: no frame", n + 1))?;
            let vals: Vec<Option<f64>> = p
                .map(|v| if v == "-" { Ok(None) } else { v.parse::<f64>().map(Some) })
                .collect::<Result<_, _>>()
                .map_err(|_| format!("row {}: a value is not a number", n + 1))?;
            if vals.len() != names.len() {
                return Err(format!("row {}: {} values for {} columns", n + 1, vals.len(), names.len()));
            }
            rows.push((f, vals));
        }
        Ok(Index { key, every, frames, table, names, rows })
    }

    fn col(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }
}

/// The index of spine `dir` if it is for `key` and this value table.
pub fn load_index(dir: &Path, key: u64, table: &[ValueDef]) -> Option<Index> {
    let i = Index::parse(&std::fs::read_to_string(dir.join("index.tsv")).ok()?).ok()?;
    (i.key == key && i.table == table_key(table)).then_some(i)
}

// ------------------------------------------------------------------ queries

/// `NAME OP V` (`screen=7`, `speed>3`, `clock<=1000`).
#[derive(Clone, Debug, PartialEq)]
pub struct Filter {
    pub name: String,
    pub op: &'static str,
    pub v: f64,
}

pub fn parse_filter(s: &str) -> Result<Filter, String> {
    let s = s.trim();
    for op in [">=", "<=", "!=", "==", "=", ">", "<"] {
        if let Some((n, v)) = s.split_once(op) {
            let v = v.trim();
            let x = match v.strip_prefix("0x") {
                Some(h) => i64::from_str_radix(h, 16).ok().map(|x| x as f64),
                None => v.parse::<f64>().ok(),
            };
            let (n, x) = (n.trim(), x.ok_or_else(|| format!("where `{s}`: `{v}` is not a number"))?);
            if n.is_empty() {
                break;
            }
            let op = match op {
                "=" | "==" => "==",
                o => o,
            };
            return Ok(Filter { name: n.to_string(), op, v: x });
        }
    }
    Err(format!("where `{s}`: want NAME OP VALUE (OP is =, !=, >, >=, <, <=), e.g. screen=7 or speed>3"))
}

impl Filter {
    fn holds(&self, x: f64) -> bool {
        match self.op {
            "==" => x == self.v,
            "!=" => x != self.v,
            ">" => x > self.v,
            ">=" => x >= self.v,
            "<" => x < self.v,
            _ => x <= self.v,
        }
    }
}

/// `NAME=V`: a point to be near.
pub fn parse_near(s: &str) -> Result<(String, f64), String> {
    let (n, v) = s.split_once('=').ok_or_else(|| format!("near `{s}`: want NAME=VALUE"))?;
    let x = v.trim().parse::<f64>().map_err(|_| format!("near `{s}`: `{v}` is not a number"))?;
    Ok((n.trim().to_string(), x))
}

/// A frame the query found.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub frame: u64,
    /// Distance to the point asked for (0 when none was).
    pub distance: f64,
    pub values: Vec<(String, Option<f64>)>,
}

/// A stretch of consecutive index rows that all match.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub from: u64,
    pub to: u64,
    pub rows: usize,
}

pub enum Found {
    /// With a point: the nearest rows, at least `gap` frames apart.
    Nearest(Vec<Hit>),
    /// Without: the stretches that match, in order.
    Spans(Vec<Span>),
}

impl Index {
    /// Rows where every filter holds, then (with `near`) the closest to
    /// that point, or (without) the stretches. A name the index does not
    /// have is an error naming the ones it has.
    pub fn find(&self, near: &[(String, f64)], filters: &[Filter], limit: usize, gap: u64) -> Result<Found, String> {
        let col = |n: &str| {
            self.col(n).ok_or_else(|| {
                let all: Vec<&str> = self.names.iter().map(String::as_str).collect();
                format!("the index has no value `{n}`{}", tour::near(n, &all))
            })
        };
        let near_cols: Vec<(usize, f64)> =
            near.iter().map(|(n, v)| Ok((col(n)?, *v))).collect::<Result<_, String>>()?;
        let filt: Vec<(usize, &Filter)> =
            filters.iter().map(|f| Ok((col(&f.name)?, f))).collect::<Result<_, String>>()?;
        let ok = |vals: &[Option<f64>]| filt.iter().all(|(c, f)| vals[*c].is_some_and(|x| f.holds(x)));
        if near_cols.is_empty() {
            let mut spans: Vec<Span> = Vec::new();
            let mut open = false;
            for (f, vals) in &self.rows {
                if ok(vals) {
                    match spans.last_mut() {
                        Some(s) if open => {
                            s.to = *f;
                            s.rows += 1;
                        }
                        _ => spans.push(Span { from: *f, to: *f, rows: 1 }),
                    }
                    open = true;
                } else {
                    open = false;
                }
            }
            spans.truncate(limit);
            return Ok(Found::Spans(spans));
        }
        let mut cand: Vec<(f64, usize)> = Vec::new();
        for (i, (_, vals)) in self.rows.iter().enumerate() {
            if !ok(vals) {
                continue;
            }
            let mut d2 = 0.0;
            let mut all = true;
            for (c, v) in &near_cols {
                match vals[*c] {
                    Some(x) => d2 += (x - v) * (x - v),
                    None => all = false,
                }
            }
            if all {
                cand.push((d2.sqrt(), i));
            }
        }
        cand.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut hits: Vec<Hit> = Vec::new();
        for (d, i) in cand {
            let f = self.rows[i].0;
            if hits.iter().any(|h| h.frame.abs_diff(f) < gap) {
                continue;
            }
            hits.push(Hit {
                frame: f,
                distance: d,
                values: self.names.iter().cloned().zip(self.rows[i].1.iter().copied()).collect(),
            });
            if hits.len() >= limit {
                break;
            }
        }
        Ok(Found::Nearest(hits))
    }
}

pub fn hit_json(label: &str, h: &Hit) -> Value {
    let values: serde_json::Map<String, Value> =
        h.values.iter().map(|(n, v)| (n.clone(), v.map_or(Value::Null, |x| json!(x)))).collect();
    json!({"frame": h.frame, "distance": h.distance, "values": values, "open": format!("tour:{label}@{}", h.frame)})
}

pub fn span_json(label: &str, s: &Span) -> Value {
    json!({"from": s.from, "to": s.to, "rows": s.rows, "open": format!("tour:{label}@{}", s.from)})
}

// ------------------------------------------------------------------ building

/// A machine as a session and a tour run make it (the native geometrizer
/// unless `--geo` says otherwise).
pub fn fresh_machine(roms: &RomSet) -> Box<Machine> {
    let mut m = Box::new(crate::machine(roms));
    if crate::GEO_MODE.get().is_none() {
        m.bus.geo.set_mode(model2::geo::GeoMode::Native);
    }
    m
}

pub fn native() -> bool {
    crate::GEO_MODE.get().is_none()
}

/// What building did.
#[derive(Clone, Debug)]
pub struct Built {
    pub meta: Meta,
    pub rows: usize,
    pub seconds: f64,
    pub bytes: u64,
}

/// Replay `t` once from power-on and write its checkpoints (every
/// `every` frames) and its index (every `index_every`; all of `table`'s
/// values) into `dir`, replacing what was there. `script` is the tour's
/// whole input.
#[allow(clippy::too_many_arguments)]
pub fn build(
    roms: &RomSet,
    t: &Tour,
    script: &InputScript,
    dir: &Path,
    every: u64,
    index_every: u64,
    table: &[ValueDef],
) -> Result<Built, String> {
    let Length::Frames(len) = t.length else {
        return Err(format!("tour {}: a spine is a tour with a length in frames (like 19368f), not in steps", t.label));
    };
    let started = Instant::now();
    let key = key(roms.game, t, native());
    // Drop what was there, so no old state mixes with the new.
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut m = fresh_machine(roms);
    script.apply(0, &mut m.bus);
    let (mut last, mut next_ckpt, mut next_row) = (0, every, index_every);
    let (mut checkpoints, mut rows, mut bytes) = (Vec::new(), Vec::new(), 0u64);
    let read = |m: &Machine, rows: &mut Vec<(u64, Vec<Option<f64>>)>| {
        let vals = table
            .iter()
            .map(|d| m.bus.peek(d.addr, d.width).and_then(|raw| crate::ram::decode(d, raw).as_f64()))
            .collect();
        rows.push((m.frames, vals));
    };
    while m.frames < len {
        m.run_slice(u64::MAX / 2).map_err(|e| format!("the machine stopped at frame {}: {e:?}", m.frames))?;
        if m.frames != last {
            last = m.frames;
            script.apply(m.frames, &mut m.bus);
            if m.frames >= next_row {
                read(&m, &mut rows);
                next_row = m.frames - m.frames % index_every + index_every;
            }
            if m.frames >= next_ckpt && m.frames < len {
                let data = model2::save_session(&mut m, None);
                bytes += data.len() as u64;
                let p = ckpt_path(dir, m.frames);
                std::fs::write(&p, data).map_err(|e| format!("{}: {e}", p.display()))?;
                checkpoints.push(m.frames);
                next_ckpt = m.frames - m.frames % every + every;
            }
        }
    }
    let meta = Meta { key, every, frames: m.frames, checkpoints };
    let index = Index {
        key,
        every: index_every,
        frames: m.frames,
        table: table_key(table),
        names: table.iter().map(|d| d.name.clone()).collect(),
        rows,
    };
    let n = index.rows.len();
    std::fs::write(dir.join("index.tsv"), index.to_text()).map_err(|e| format!("index.tsv: {e}"))?;
    // The meta file last: a build that stopped half-way has none, so it counts as no cache.
    std::fs::write(dir.join("meta.txt"), meta.text()).map_err(|e| format!("meta.txt: {e}"))?;
    Ok(Built { meta, rows: n, seconds: started.elapsed().as_secs_f64(), bytes })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Vec<ValueDef> {
        tour::value_table("x|0x10:l:float|a note\nz|0x14:l:float|a note\nscreen|0x18:l|a note\n").unwrap()
    }

    fn index() -> Index {
        let rows = (0..10u64)
            .map(|i| {
                (i * 30, vec![Some(i as f64 * 10.0), Some(-(i as f64) * 100.0), Some(if i < 4 { 7.0 } else { 12.0 })])
            })
            .collect();
        Index {
            key: 0xabc,
            every: 30,
            frames: 300,
            table: table_key(&table()),
            names: vec!["x".into(), "z".into(), "screen".into()],
            rows,
        }
    }

    #[test]
    fn the_index_round_trips_and_checks_its_key_and_table() {
        let mut i = index();
        i.rows[2].1[1] = None;
        let back = Index::parse(&i.to_text()).unwrap();
        assert_eq!(back, i);
        let dir = std::env::temp_dir().join(format!("remodel-spine-test-{}", std::process::id()));
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/scratch").join(dir.file_name().unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.tsv"), i.to_text()).unwrap();
        assert!(load_index(&dir, 0xabc, &table()).is_some());
        assert!(load_index(&dir, 0xabd, &table()).is_none(), "another key (a changed recording) is no index");
        let mut other = table();
        other[0].addr += 4;
        assert!(load_index(&dir, 0xabc, &other).is_none(), "a changed value table is no index");
        assert!(Index::parse("# t\n# key=1 every=1 frames=1 table=1\nframe\tx\n0\t1\t2\n").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The cache is the recording's: another line, a parent's change, the
    /// geometrizer or a missing file all give no checkpoints.
    #[test]
    fn checkpoints_are_dropped_when_the_recording_changes() {
        let g = romset::games::find("topskatr").unwrap();
        let parse = |s: &str| tour::parse(s, "t").unwrap();
        let a = parse("a|100f|coin-start,left@30-60\nb|200f|from=a@50,right@60-70\n");
        let a2 = parse("a|100f|coin-start,left@30-61\nb|200f|from=a@50,right@60-70\n");
        let (ka, kb, kb2) = (key(g, &a[0], true), key(g, &a[1], true), key(g, &a2[1], true));
        assert_ne!(ka, kb);
        assert_ne!(kb, kb2, "a branch's key follows its spine's line");
        assert_ne!(ka, key(g, &a[0], false));
        let dir =
            Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../out/scratch/spine-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let m = Meta { key: ka, every: 600, frames: 1500, checkpoints: vec![600, 1200] };
        assert_eq!(Meta::parse(&m.text()), Some(m.clone()));
        std::fs::write(dir.join("meta.txt"), m.text()).unwrap();
        assert!(meta(&dir, ka).is_none(), "a checkpoint file is missing");
        for f in &m.checkpoints {
            std::fs::write(ckpt_path(&dir, *f), b"x").unwrap();
        }
        assert_eq!(meta(&dir, ka), Some(m));
        assert_eq!(nearest(&dir, ka, 599), None);
        assert_eq!(nearest(&dir, ka, 600).unwrap().0, 600);
        assert_eq!(nearest(&dir, ka, 1199).unwrap().0, 600);
        assert_eq!(nearest(&dir, ka, 9999).unwrap().0, 1200);
        assert!(meta(&dir, kb).is_none(), "another recording's key is no cache");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_index_answers_near_and_where() {
        let i = index();
        let near = vec![("x".to_string(), 52.0), ("z".to_string(), -500.0)];
        let Found::Nearest(h) = i.find(&near, &[], 2, 60).unwrap() else { panic!() };
        assert_eq!(h[0].frame, 150, "row 5 is x 50, z -500");
        assert!(h[1].frame.abs_diff(150) >= 60, "{:?}", h);
        // A filter narrows it; a gap of 0 allows neighbours.
        let on = vec![parse_filter("screen=7").unwrap()];
        let Found::Nearest(h) = i.find(&near, &on, 1, 0).unwrap() else { panic!() };
        assert_eq!(h[0].frame, 90, "the nearest row on screen 7 is row 3");
        // No point: the stretches.
        let Found::Spans(s) = i.find(&[], &[parse_filter("screen=12").unwrap()], 5, 0).unwrap() else { panic!() };
        assert_eq!(s, vec![Span { from: 120, to: 270, rows: 6 }]);
        let Found::Spans(s) =
            i.find(&[], &[parse_filter("x>=0x14").unwrap(), parse_filter("x<50").unwrap()], 5, 0).unwrap()
        else {
            panic!()
        };
        assert_eq!(s, vec![Span { from: 60, to: 120, rows: 3 }]);
        let e = i.find(&[("xx".into(), 1.0)], &[], 1, 0).err().unwrap();
        assert!(e.contains("did you mean x"), "{e}");
        assert!(parse_filter("speed").is_err() && parse_filter("=3").is_err() && parse_filter("a>b").is_err());
        assert_eq!(parse_filter("speed>=2.5").unwrap(), Filter { name: "speed".into(), op: ">=", v: 2.5 });
        assert!(parse_near("x").is_err());
    }
}
