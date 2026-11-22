//! `remodel studio SUB DIR ...`: the uplift studio's project commands
//! (docs/uplift-studio.md). A project is a directory of plain files; every
//! command is non-interactive, idempotent and prints TSV or short text, so
//! an agent and a person use the same ones. Nothing here is specific to a
//! game: it works on what `remodel texmap` writes and on texture packs.

#![allow(clippy::needless_range_loop, clippy::chunks_exact_to_as_chunks, clippy::manual_is_multiple_of)]

#[path = "studio_art.rs"]
mod art;
#[path = "studio_gates.rs"]
mod asset_gates;
#[path = "studio_project.rs"]
mod project_cmd;
#[path = "studio_regions.rs"]
mod regions;

pub use art::artworks;
pub use asset_gates::{asset_main, check_assets, refusals, Src};
pub use project_cmd::{build_main, project_main};

use crate::studio_answer::unknown;
use crate::studio_img::{self as img, Image, Wrap};
use crate::studio_text as text;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// What the global options hold that `studio compare` needs.
pub struct Ctx {
    pub game: String,
    pub rom: PathBuf,
    pub jit: bool,
    pub input: Option<String>,
}

const USAGE: &str = "\
remodel studio SUB PROJECT [options]   (docs/uplift-studio.md)
  init PROJECT [--name N] [--game G] [--map [NAME=]TEXMAP_DIR].. [--copy-map] [--style FILE] [--capture-dir DIR]
  region PROJECT --frame F --region X,Y,W,H [--scale S] [--map NAME]   what is drawn there (TSV)
  select PROJECT [--key K,..] [--group N] [--surface S] [--frame F --region X,Y,W,H]
         [--kind tex|tile] [--min-pixels N] [--all] [--category C] [--reason TEXT]
  status PROJECT [--tsv] [--missing] [--strict]   (--strict: exit 1 unless every selected image is approved)
  approve PROJECT [NAME..] [--drafted] [--category C] [--group N] [--undo] [--replace]
  import PROJECT --from DIR --tool TEXT [NAME..|--group N|--category C|--all] [--replace]   drafts made elsewhere, with provenance
  pack PROJECT [--approve-all] [--no-lint] [--replace]      pack.txt, index.txt, pack-lint
  draft PROJECT --brief FILE [NAME..|--group N|--category C|--all] [--cmd TEMPLATE]
         [--ref IMG].. [--jobs N] [--retries N] [--timeout SECS] [--scale N] [--keep-size] [--replace] [--dry-run]
  style PROJECT --recipe FILE [NAME..|--group N|--category C|--all] [--from original|draft] [--replace]
  variants PROJECT --of KEY [--also KEY,..] [--base IMG] [--mode luma|ratio|mean] [--strength S] [--dry-run] [--replace]
  artworks PROJECT [--tsv] [--min-variants N]   the artworks (surface keys) with their colourings, ramps and art_ images
  bake PROJECT [ART..|--all] [--from DIR] [--mode baked|fixed] [--dry-run] [--replace]
         the colourings of each art_ image: baked tex_ copies through the colour ramps, or fixed (drawn as it is)
  text PROJECT --frame F --region X,Y,W,H (--image FILE | --text STR --font FILE ..)
         [--box X,Y,W,H] [--pad N] [--bg #rrggbb|auto] [--fit contain|stretch] [--align left|center|right]
         [--alpha original|typeset] [--tile-scale N] [--dry-run] [--replace]
  region-capture PROJECT NAME --capture-dir DIR --frame F --region X,Y,W,H [--plane a|b] [--layer high|low]
         [--also F,..] [--wild X,Y;..] [--image FILE] [--ref-scale N]   a 2D region override (tile layout + original)
  regions PROJECT [--tsv]   the project's regions and whether each has its replacement
  seam PROJECT [--from pack|draft] [--tsv] [--strict]
  compare PROJECT [--pack DIR] [--steps N] [--snap N] [--frames F,..] [--jobs N]
         [--run-args \"--height 1080 --aspect 16:9\"] [--force]   (needs the ROMs)
  compare PROJECT --scene NAME[,NAME..]|all [--pack DIR] [--into DIR] [--view native|H[,A:B]]
         before and after from the scenes' captures: no run, no ROM (remodel scene make first)
  A run's output is never reused: compare refuses a folder that holds one unless --force reruns it.
Every command takes --json: one object (ok, code, error, data, warnings, next) instead of text.
Exit codes: 0 done, 1 a check failed, 2 the request was wrong (unknown names and options are errors),
3 something needed is missing, 4 an internal error. Nothing overwrites a draft or an approved image
without --replace.
";

pub fn main(args: &[String], ctx: &Ctx) -> Result<bool, String> {
    let Some(sub) = args.first() else {
        print!("{USAGE}");
        return Ok(true);
    };
    let Some((_, allowed)) = OPTIONS.iter().find(|(n, _)| n == sub) else {
        let names: Vec<&str> = OPTIONS.iter().map(|(n, _)| *n).collect();
        return Err(format!("studio: {}\n{USAGE}", unknown("subcommand", sub, &names)));
    };
    prescan(sub, allowed, &args[1..])?;
    let a = Args::parse(&args[1..])?;
    a.check(sub, allowed)?;
    let dir = a.pos.first().ok_or_else(|| format!("studio {sub}: which PROJECT directory?\n{USAGE}"))?;
    let p = Project { dir: PathBuf::from(dir) };
    match sub.as_str() {
        "init" => init(&p, &a),
        "region" => region(&p, &a),
        "select" => select(&p, &a),
        "status" => status(&p, &a),
        "approve" => approve(&p, &a),
        "import" => import(&p, &a),
        "pack" => pack(&p, &a),
        "draft" => draft(&p, &a),
        "style" => style(&p, &a),
        "variants" => variants(&p, &a),
        "artworks" => art::list(&p, &a),
        "bake" => art::bake(&p, &a),
        "text" => text_cmd(&p, &a),
        "region-capture" => regions::capture(&p, &a),
        "regions" => regions::list(&p, &a),
        "seam" => seam(&p, &a),
        "compare" => compare(&p, &a, ctx),
        _ => unreachable!("every name in OPTIONS has an arm"),
    }
}

/// Every subcommand and the options it reads (values and switches). An
/// option not listed for the subcommand is an error that names the near
/// ones, never a value that is silently ignored. `--json` is global.
const OPTIONS: &[(&str, &[&str])] = &[
    ("init", &["--name", "--game", "--map", "--style", "--capture-dir", "--copy-map"]),
    ("region", &["--frame", "--region", "--scale", "--map"]),
    (
        "select",
        &[
            "--key",
            "--group",
            "--surface",
            "--frame",
            "--region",
            "--kind",
            "--min-pixels",
            "--all",
            "--category",
            "--reason",
            "--map",
            "--scale",
        ],
    ),
    ("status", &["--tsv", "--missing", "--strict"]),
    ("approve", &["--drafted", "--category", "--group", "--undo", "--all", "--replace"]),
    ("import", &["--from", "--tool", "--prompts", "--group", "--category", "--all", "--replace"]),
    ("pack", &["--approve-all", "--no-lint", "--replace"]),
    (
        "draft",
        &[
            "--brief",
            "--ref",
            "--jobs",
            "--retries",
            "--timeout",
            "--scale",
            "--cmd",
            "--group",
            "--category",
            "--all",
            "--keep-size",
            "--replace",
            "--force",
            "--dry-run",
        ],
    ),
    ("style", &["--recipe", "--from", "--group", "--category", "--all", "--replace"]),
    ("variants", &["--of", "--also", "--base", "--mode", "--strength", "--ink", "--map", "--dry-run", "--replace"]),
    ("artworks", &["--tsv", "--min-variants"]),
    ("bake", &["--all", "--from", "--mode", "--dry-run", "--replace"]),
    (
        "text",
        &[
            "--frame",
            "--region",
            "--image",
            "--trim",
            "--margin",
            "--box",
            "--fit",
            "--align",
            "--alpha",
            "--tile-scale",
            "--pad",
            "--bg",
            "--category",
            "--reason",
            "--text",
            "--size",
            "--colour",
            "--stroke",
            "--stroke-width",
            "--typesetter",
            "--font",
            "--scale",
            "--map",
            "--dry-run",
            "--replace",
        ],
    ),
    (
        "region-capture",
        &[
            "--capture-dir",
            "--frame",
            "--region",
            "--plane",
            "--layer",
            "--also",
            "--wild",
            "--image",
            "--ref-scale",
            "--pin",
            "--scale",
            "--map",
            "--game-px",
        ],
    ),
    ("regions", &["--tsv"]),
    ("seam", &["--from", "--tsv", "--strict"]),
    (
        "compare",
        &["--pack", "--into", "--steps", "--snap", "--run-args", "--frames", "--jobs", "--force", "--scene", "--view"],
    ),
];

/// The options typed, before anything is parsed: one the subcommand does not
/// read is named with its near matches (parsing would first complain that it
/// "wants a value"). A value is skipped, so one that starts with `--` (a
/// `--run-args "--height 1080"`) is not taken for an option.
fn prescan(sub: &str, allowed: &[&str], args: &[String]) -> Result<(), String> {
    let mut it = args.iter();
    while let Some(x) = it.next() {
        if !x.starts_with("--") || x == "--json" {
            continue;
        }
        if !allowed.contains(&x.as_str()) {
            return Err(format!(
                "studio {sub}: {}\n(its options: {})",
                unknown("option", x, allowed),
                allowed.join(" ")
            ));
        }
        if !SWITCHES.contains(&x.as_str()) {
            it.next();
        }
    }
    Ok(())
}

/// A warning for the answer's `warnings` (stderr, read back by `--json`).
fn warn(msg: &str) {
    eprintln!("warning: {msg}");
}

// ------------------------------------------------------------ arguments

const SWITCHES: &[&str] = &[
    "--tsv",
    "--missing",
    "--force",
    "--dry-run",
    "--drafted",
    "--all",
    "--undo",
    "--copy-map",
    "--approve-all",
    "--no-lint",
    "--strict",
    "--game-px",
    "--pin",
    "--keep-size",
    "--replace",
    "--json",
];

pub struct Args {
    pub pos: Vec<String>,
    flags: Vec<(String, String)>,
    sw: HashSet<String>,
}

impl Args {
    pub fn parse(a: &[String]) -> Result<Args, String> {
        let mut r = Args { pos: vec![], flags: vec![], sw: HashSet::new() };
        let mut it = a.iter();
        while let Some(x) = it.next() {
            if SWITCHES.contains(&x.as_str()) {
                r.sw.insert(x.clone());
            } else if x.starts_with("--") {
                let v = it.next().ok_or_else(|| format!("{x} wants a value"))?;
                r.flags.push((x.clone(), v.clone()));
            } else {
                r.pos.push(x.clone());
            }
        }
        Ok(r)
    }

    /// Every option given must be one `sub` reads (or the global `--json`).
    fn check(&self, sub: &str, allowed: &[&str]) -> Result<(), String> {
        let mut used: Vec<&str> = self.flags.iter().map(|(f, _)| f.as_str()).collect();
        used.extend(self.sw.iter().map(String::as_str));
        used.sort();
        used.dedup();
        for u in used {
            if u != "--json" && !allowed.contains(&u) {
                return Err(format!(
                    "studio {sub}: {}\n(its options: {})",
                    unknown("option", u, allowed),
                    allowed.join(" ")
                ));
            }
        }
        Ok(())
    }

    fn get(&self, k: &str) -> Option<&str> {
        self.flags.iter().rev().find(|(f, _)| f == k).map(|(_, v)| v.as_str())
    }

    fn all(&self, k: &str) -> Vec<&str> {
        self.flags.iter().filter(|(f, _)| f == k).map(|(_, v)| v.as_str()).collect()
    }

    fn has(&self, k: &str) -> bool {
        self.sw.contains(k)
    }

    fn num<T: std::str::FromStr>(&self, k: &str) -> Result<Option<T>, String> {
        self.get(k).map(|v| v.parse::<T>().map_err(|_| format!("{k}: `{v}` is not a number"))).transpose()
    }

    /// `X,Y,W,H`.
    fn rect(&self, k: &str) -> Result<Option<(usize, usize, usize, usize)>, String> {
        let Some(v) = self.get(k) else { return Ok(None) };
        let n: Vec<usize> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        match n[..] {
            [x, y, w, h] if w > 0 && h > 0 => Ok(Some((x, y, w, h))),
            _ => Err(format!("{k}: `{v}` is not X,Y,W,H")),
        }
    }
}

// -------------------------------------------------------------- project

#[derive(Clone, Debug)]
pub struct Inv {
    pub id: u32,
    pub name: String,
    pub kind: String,
    pub w: usize,
    pub h: usize,
    pub surface: String,
    pub pixels: u64,
    pub best_frame: String,
    /// The map (a name from `Project::maps`) whose frames and ids this row is from.
    pub map: String,
    pub wrap: String,
    pub group: u32,
    /// How many texture sizes the coordinates run over (above 1: it repeats).
    pub span_u: f32,
    pub span_v: f32,
    /// The catalogue's detail text (`colour=`, `luma=`, `ramp=`, ...).
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sel {
    pub name: String,
    pub kind: String,
    pub group: u32,
    pub surface: String,
    pub pixels: u64,
    pub category: String,
    pub reason: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Original,
    Drafted,
    Approved,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Status::Original => "original",
            Status::Drafted => "drafted",
            Status::Approved => "approved",
        }
    }
}

pub struct Project {
    pub dir: PathBuf,
}

const SEL_HEADER: &str = "name\tkind\tgroup\tsurface\tpixels\tcategory\treason";

fn clean(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

impl Project {
    fn p(&self, sub: &str) -> PathBuf {
        self.dir.join(sub)
    }

    fn map_names(&self) -> String {
        self.maps().into_iter().map(|m| m.0).collect::<Vec<_>>().join(", ")
    }

    fn require(&self) -> Result<(), String> {
        if self.p("project.txt").is_file() {
            Ok(())
        } else {
            Err(format!("{}: not a studio project (no project.txt): `studio init` it first", self.dir.display()))
        }
    }

    fn meta(&self) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        for l in std::fs::read_to_string(self.p("project.txt")).unwrap_or_default().lines() {
            if let Some((k, v)) = l.split_once('\t') {
                m.insert(k.to_string(), v.to_string());
            }
        }
        m
    }

    /// The texture maps of the project: `map/` itself when it holds an
    /// inventory (one map, named `map`), else each folder (or link) in it
    /// that does, by name (a course run and a character-select run, say).
    fn maps(&self) -> Vec<(String, PathBuf)> {
        let m = self.p("map");
        if m.join("inventory.tsv").is_file() {
            return vec![("map".to_string(), m)];
        }
        let mut v: Vec<(String, PathBuf)> = std::fs::read_dir(&m)
            .map(|d| {
                d.filter_map(|e| e.ok())
                    .map(|e| (e.file_name().to_string_lossy().to_string(), e.path()))
                    .filter(|(_, p)| p.join("inventory.tsv").is_file())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

    /// The directory of map `name`.
    fn map_dir(&self, name: &str) -> PathBuf {
        self.maps().into_iter().find(|(n, _)| n == name).map(|(_, d)| d).unwrap_or_else(|| self.p("map"))
    }

    /// Every map's inventory, merged: a name in several maps is listed once,
    /// from the first map that has it (maps in name order); `id`s are only
    /// meaningful with `Inv::map`.
    pub(crate) fn inventory(&self) -> Result<Vec<Inv>, String> {
        let maps = self.maps();
        if maps.is_empty() {
            return Err(format!("{}: no texture map (run `make texmap`, then init --map)", self.p("map").display()));
        }
        let mut out: Vec<Inv> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for (name, dir) in &maps {
            for i in self.inventory_of(name, dir)? {
                if seen.insert(i.name.clone()) {
                    out.push(i);
                }
            }
        }
        Ok(out)
    }

    /// One map's inventory.
    fn inventory_of(&self, map: &str, dir: &Path) -> Result<Vec<Inv>, String> {
        let path = dir.join("inventory.tsv");
        let t = std::fs::read_to_string(&path)
            .map_err(|e| format!("{}: {e} (run `make texmap`, then init --map)", path.display()))?;
        let mut lines = t.lines();
        let head: Vec<&str> = lines.next().unwrap_or("").split('\t').collect();
        let col = |n: &str| head.iter().position(|h| *h == n);
        let (Some(id), Some(name)) = (col("id"), col("name")) else {
            return Err(format!("{}: no id and name columns", path.display()));
        };
        let get = |f: &[&str], n: &str| col(n).and_then(|i| f.get(i)).copied().unwrap_or("").to_string();
        Ok(lines
            .filter_map(|l| {
                let f: Vec<&str> = l.split('\t').collect();
                Some(Inv {
                    id: f.get(id)?.parse().ok()?,
                    name: f.get(name)?.to_string(),
                    kind: get(&f, "kind"),
                    w: get(&f, "w").parse().unwrap_or(0),
                    h: get(&f, "h").parse().unwrap_or(0),
                    surface: get(&f, "surface"),
                    pixels: get(&f, "pixels").parse().unwrap_or(0),
                    best_frame: get(&f, "best_frame"),
                    map: map.to_string(),
                    wrap: get(&f, "wrap"),
                    group: get(&f, "group").parse().unwrap_or(0),
                    span_u: get(&f, "span_u").parse().unwrap_or(2.0),
                    span_v: get(&f, "span_v").parse().unwrap_or(2.0),
                    detail: get(&f, "detail"),
                })
            })
            .collect())
    }

    pub(crate) fn selection(&self) -> Vec<Sel> {
        std::fs::read_to_string(self.p("selection.tsv"))
            .unwrap_or_default()
            .lines()
            .skip(1)
            .filter_map(|l| {
                let f: Vec<&str> = l.split('\t').collect();
                Some(Sel {
                    name: f.first()?.to_string(),
                    kind: f.get(1)?.to_string(),
                    group: f.get(2)?.parse().unwrap_or(0),
                    surface: f.get(3)?.to_string(),
                    pixels: f.get(4)?.parse().unwrap_or(0),
                    category: f.get(5).unwrap_or(&"").to_string(),
                    reason: f.get(6).unwrap_or(&"").to_string(),
                })
            })
            .collect()
    }

    fn write_selection(&self, rows: &[Sel]) -> Result<(), String> {
        let mut s = format!("{SEL_HEADER}\n");
        for r in rows {
            let _ = writeln!(
                s,
                "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                r.name,
                r.kind,
                r.group,
                r.surface,
                r.pixels,
                clean(&r.category),
                clean(&r.reason)
            );
        }
        std::fs::write(self.p("selection.tsv"), s).map_err(|e| e.to_string())
    }

    fn status_of(&self, name: &str) -> Status {
        if self.p("pack").join(format!("{name}.png")).is_file() {
            Status::Approved
        } else if self.p("draft").join(format!("{name}.png")).is_file() {
            Status::Drafted
        } else {
            Status::Original
        }
    }

    /// What stands in the way of writing `name`'s draft: an image already
    /// there (a draft, or an approved one) and no `--replace`.
    fn blocked(&self, name: &str, a: &Args) -> Option<&'static str> {
        if a.has("--replace") {
            None
        } else if self.p("pack").join(format!("{name}.png")).is_file() {
            Some("approved")
        } else if self.p("draft").join(format!("{name}.png")).is_file() {
            Some("draft")
        } else {
            None
        }
    }

    /// The original as dumped: the project's copy, else the map's.
    fn original(&self, name: &str) -> Option<PathBuf> {
        if name.starts_with("art_") {
            // An artwork's original is its elected look: the colouring
            // drawn most.
            let aw = art::artworks(self).ok()?.into_iter().find(|a| a.stem() == name)?;
            return self.map_texture(&aw.variants.first()?.name, true);
        }
        self.map_texture(name, true)
    }

    /// The map's own copy of a texture's original (any map has it).
    fn map_texture(&self, name: &str, project_first: bool) -> Option<PathBuf> {
        let mut dirs = vec![];
        if project_first {
            dirs.push(self.p("original"));
        }
        dirs.extend(self.maps().into_iter().map(|(_, d)| d.join("textures")));
        dirs.iter().map(|d| d.join(format!("{name}.png"))).find(|p| p.is_file())
    }

    fn ledger(&self) -> BTreeMap<String, (String, String, String)> {
        let mut m = BTreeMap::new();
        for l in std::fs::read_to_string(self.p("status/status.txt")).unwrap_or_default().lines() {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() >= 4 {
                m.insert(f[0].to_string(), (f[1].to_string(), f[2].to_string(), f[3].to_string()));
            }
        }
        m
    }

    /// Record how `name`'s draft was made: tool, the file under prompts/
    /// holding the prompt or parameters (or `-`), the date. One line per
    /// image, replaced if there is one.
    fn record(&self, name: &str, tool: &str, prompt: &str) -> Result<(), String> {
        // Parallel draft jobs record concurrently; read-modify-write under a lock.
        static LEDGER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LEDGER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut m = self.ledger();
        m.insert(name.to_string(), (clean(tool), prompt.to_string(), today()));
        let mut s = String::new();
        for (n, (t, p, d)) in m {
            let _ = writeln!(s, "{n}\t{t}\t{p}\t{d}");
        }
        std::fs::create_dir_all(self.p("status")).map_err(|e| e.to_string())?;
        std::fs::write(self.p("status/status.txt"), s).map_err(|e| e.to_string())
    }

    fn write_prompt(&self, name: &str, body: &str) -> Result<String, String> {
        std::fs::create_dir_all(self.p("prompts")).map_err(|e| e.to_string())?;
        let f = format!("{name}.txt");
        std::fs::write(self.p("prompts").join(&f), body).map_err(|e| e.to_string())?;
        Ok(f)
    }

    /// The selected rows a command works on: names given, a group, a
    /// category, or all.
    ///
    /// Strict: a name (a row's name, or its 16-digit key) that is not in
    /// the selection, a group with no rows or a category nobody has is an
    /// error, never "nothing to narrow by, so every row"; filters given
    /// together all apply.
    fn pick(&self, a: &Args, skip_first_pos: bool) -> Result<Vec<Sel>, String> {
        let rows = self.selection();
        let names: Vec<&String> = a.pos.iter().skip(skip_first_pos as usize).collect();
        let group: Option<u32> = a.num("--group")?;
        let cat = a.get("--category");
        let unknown: Vec<&str> =
            names.iter().filter(|n| !rows.iter().any(|r| name_matches(&r.name, n))).map(|n| n.as_str()).collect();
        if !unknown.is_empty() {
            let shown: Vec<&str> = unknown.iter().copied().take(8).collect();
            return Err(format!(
                "not in the selection ({} rows): {}{} (`studio status PROJECT --tsv` lists the names; a name is the row's name or its 16-digit key)",
                rows.len(),
                shown.join(", "),
                if unknown.len() > shown.len() { format!(", and {} more", unknown.len() - shown.len()) } else { String::new() }
            ));
        }
        if let Some(g) = group {
            if !rows.iter().any(|r| r.group == g) {
                return Err(format!("--group {g}: no selected row is in group {g}"));
            }
        }
        if let Some(c) = cat {
            if !rows.iter().any(|r| r.category == c) {
                let cats: BTreeSet<&str> = rows.iter().map(|r| r.category.as_str()).collect();
                return Err(format!(
                    "--category {c}: no selected row has it (categories: {})",
                    cats.into_iter().collect::<Vec<_>>().join(", ")
                ));
            }
        }
        if names.is_empty() && group.is_none() && cat.is_none() && !a.has("--all") {
            return Err("which rows? name them, or --group N, --category C or --all".into());
        }
        let picked: Vec<Sel> = rows
            .iter()
            .filter(|r| {
                (names.is_empty() || names.iter().any(|n| name_matches(&r.name, n)))
                    && group.is_none_or(|g| r.group == g)
                    && cat.is_none_or(|c| r.category == c)
            })
            .cloned()
            .collect();
        if picked.is_empty() {
            return Err("no selected rows match all of those filters".into());
        }
        Ok(picked)
    }
}

/// What a command that skipped images that were there says: a warning
/// (the first few names) and, when it wrote nothing at all, an error
/// naming the fix.
fn report_kept(kept: &[(String, &str)], wrote: usize, fail_if_none: bool) -> Result<(), String> {
    if kept.is_empty() {
        return Ok(());
    }
    let shown: Vec<String> = kept.iter().take(6).map(|(n, w)| format!("{n} ({w})")).collect();
    let msg = format!(
        "{} kept, not overwritten: {}{}",
        kept.len(),
        shown.join(", "),
        if kept.len() > 6 { format!(", and {} more", kept.len() - 6) } else { String::new() }
    );
    if wrote == 0 && fail_if_none {
        return Err(format!("{msg}\nnext: run it again with --replace to overwrite them"));
    }
    warn(&format!("{msg}; --replace overwrites them"));
    Ok(())
}

/// Whether the user's token names a row: its name, or its 16-digit key.
fn name_matches(row: &str, tok: &str) -> bool {
    row == tok
        || (tok.len() == 16
            && tok.bytes().all(|b| b.is_ascii_hexdigit())
            && (row == format!("tile_{tok}") || row.starts_with(&format!("tex_{tok}_"))))
}

pub(crate) fn today() -> String {
    let secs =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let z = secs / 86400 + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", y + (m <= 2) as i64, m, d)
}

fn frame_stem(f: &str) -> Result<String, String> {
    f.trim_end_matches(".png")
        .parse::<u64>()
        .map(|n| format!("{n:06}"))
        .map_err(|_| format!("--frame: `{f}` is not a frame number"))
}

fn copy_file(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(p) = to.parent() {
        std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    std::fs::copy(from, to).map(|_| ()).map_err(|e| format!("{} -> {}: {e}", from.display(), to.display()))
}

// ----------------------------------------------------------------- init

fn init(p: &Project, a: &Args) -> Result<bool, String> {
    for d in ["original", "draft", "pack", "prompts", "status", "work"] {
        std::fs::create_dir_all(p.p(d)).map_err(|e| e.to_string())?;
    }
    let mut meta = p.meta();
    meta.entry("created".into()).or_insert_with(today);
    let base = p.dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if let Some(n) = a.get("--name") {
        meta.insert("name".into(), clean(n));
    }
    meta.entry("name".into()).or_insert(base);
    if let Some(g) = a.get("--game") {
        meta.insert("game".into(), clean(g));
    }
    let mut s = String::new();
    for (k, v) in &meta {
        let _ = writeln!(s, "{k}\t{v}");
    }
    std::fs::write(p.p("project.txt"), s).map_err(|e| e.to_string())?;
    if !p.p("selection.tsv").exists() {
        std::fs::write(p.p("selection.tsv"), format!("{SEL_HEADER}\n")).map_err(|e| e.to_string())?;
    }
    if let Some(f) = a.get("--style") {
        copy_file(Path::new(f), &p.p("style.md"))?;
    } else if !p.p("style.md").exists() {
        std::fs::write(
            p.p("style.md"),
            "# Style sheet\n\nPalette, materials, mood, what stays recognisable, what must not change\n(readability of signs and the HUD). Every later step reads this.\n",
        )
        .map_err(|e| e.to_string())?;
    }
    if let Some(c) = a.get("--capture-dir") {
        let abs = std::fs::canonicalize(c).map_err(|e| format!("--capture-dir {c}: {e}"))?;
        meta.insert("capture".into(), abs.display().to_string());
        let mut s = String::new();
        for (k, v) in &meta {
            let _ = writeln!(s, "{k}\t{v}");
        }
        std::fs::write(p.p("project.txt"), s).map_err(|e| e.to_string())?;
    }
    // A catalogue given as a map is turned into one (docs/uplift-studio.md,
    // "Catalogue"): written into the project, from the sessions as they are.
    let mut made: Vec<String> = Vec::new();
    for m in a.all("--map") {
        let (name, path) = match m.split_once('=') {
            Some((n, d)) => (n.to_string(), d.to_string()),
            None => {
                (Path::new(m).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), m.to_string())
            }
        };
        if crate::catalogue::is_catalogue(Path::new(&path)) {
            let ix = crate::catalogue::Index::load(Path::new(&path))?;
            let out = p.p("catalogue-maps").join(&name);
            let n = crate::catalogue::export_map(&ix, &out)?;
            println!("map           {name}: the catalogue {path}, {n} textures and tile patterns");
            made.push(format!("{name}={}", out.display()));
        } else {
            made.push(m.to_string());
        }
    }
    let maps: Vec<&str> = made.iter().map(String::as_str).collect();
    if maps.len() > 1 || maps.iter().any(|m| m.contains('=')) {
        // Several maps: map/ is a folder of links (or copies) by name.
        let dst = p.p("map");
        if dst.is_symlink() {
            std::fs::remove_file(&dst).map_err(|e| e.to_string())?;
        }
        std::fs::create_dir_all(&dst).map_err(|e| e.to_string())?;
        for m in &maps {
            let (name, path) = match m.split_once('=') {
                Some((n, d)) => (n.to_string(), d),
                None => (Path::new(m).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), *m),
            };
            let src = std::fs::canonicalize(path).map_err(|e| format!("--map {m}: {e}"))?;
            let to = dst.join(&name);
            if to.is_symlink() {
                std::fs::remove_file(&to).map_err(|e| e.to_string())?;
            }
            if a.has("--copy-map") {
                copy_dir(&src, &to)?;
            } else if !to.exists() {
                #[cfg(unix)]
                std::os::unix::fs::symlink(&src, &to).map_err(|e| e.to_string())?;
                #[cfg(not(unix))]
                copy_dir(&src, &to)?;
            }
            println!("map           {name}: {}", src.display());
        }
    } else if let Some(m) = maps.first() {
        let src = std::fs::canonicalize(m).map_err(|e| format!("--map {m}: {e}"))?;
        let dst = p.p("map");
        if dst.is_symlink() {
            std::fs::remove_file(&dst).map_err(|e| e.to_string())?;
        }
        if a.has("--copy-map") {
            copy_dir(&src, &dst)?;
            println!("map           copied {}", src.display());
        } else if dst.exists() {
            return Err(format!("{} exists and is not a link: remove it or use --copy-map", dst.display()));
        } else {
            #[cfg(unix)]
            std::os::unix::fs::symlink(&src, &dst).map_err(|e| e.to_string())?;
            #[cfg(not(unix))]
            copy_dir(&src, &dst)?;
            println!("map           {} -> {}", dst.display(), src.display());
        }
    }
    println!("project       {} ({} selected)", p.dir.display(), p.selection().len());
    Ok(true)
}

fn copy_dir(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for e in std::fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))? {
        let e = e.map_err(|e| e.to_string())?;
        let to = dst.join(e.file_name());
        if e.path().is_dir() {
            copy_dir(&e.path(), &to)?;
        } else {
            std::fs::copy(e.path(), to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

// --------------------------------------------------------------- region

/// The frame's stem, its ids decoded, the ids image, its scale, and the
/// map it is from (`--map NAME`, else the one map that has the frame: with
/// several maps holding it, say which).
type FrameFiles = (String, Vec<u32>, Image, f32, (String, PathBuf));

fn frame_files(p: &Project, a: &Args) -> Result<FrameFiles, String> {
    let stem = frame_stem(a.get("--frame").ok_or("--frame F wants a frame of the map")?)?;
    let map = pick_map(p, a, &stem)?;
    let idp = map.1.join("frames").join(format!("{stem}-ids.png"));
    let ids = img::load(&idp).map_err(|e| format!("{e} (frames the map has: {}/frames/)", map.1.display()))?;
    let scale = match a.num::<f32>("--scale")? {
        Some(s) => s,
        None => ids.height as f32 / 384.0,
    };
    Ok((stem, text::decode_ids(&ids), ids, scale, map))
}

/// The map for a command about frame `stem`.
fn pick_map(p: &Project, a: &Args, stem: &str) -> Result<(String, PathBuf), String> {
    let maps = p.maps();
    if maps.is_empty() {
        return Err(format!("{}: no texture map (init --map)", p.p("map").display()));
    }
    if let Some(n) = a.get("--map") {
        return maps
            .into_iter()
            .find(|(m, _)| m == n)
            .ok_or_else(|| format!("--map {n}: the project's maps are {}", p.map_names()));
    }
    let has: Vec<&(String, PathBuf)> =
        maps.iter().filter(|(_, d)| d.join("frames").join(format!("{stem}-ids.png")).is_file()).collect();
    match has.len() {
        1 => Ok(has[0].clone()),
        0 => Ok(maps[0].clone()),
        _ => Err(format!(
            "frame {stem} is in several maps ({}): say --map NAME",
            has.iter().map(|m| m.0.as_str()).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// The ids of one map's frames: id -> texture or tile name.
fn id_names(p: &Project, map: &(String, PathBuf)) -> Result<HashMap<u32, String>, String> {
    Ok(p.inventory_of(&map.0, &map.1)?.into_iter().map(|i| (i.id, i.name)).collect())
}

fn region(p: &Project, a: &Args) -> Result<bool, String> {
    p.require()?;
    let rect = a.rect("--region")?.ok_or("--region X,Y,W,H (output pixels of the frame)")?;
    let (stem, ids, idimg, scale, map) = frame_files(p, a)?;
    let inv = p.inventory_of(&map.0, &map.1)?;
    let by_id: HashMap<u32, &Inv> = inv.iter().map(|i| (i.id, i)).collect();
    let counts = text::ids_in_region(&ids, idimg.width, rect);
    let mut rows: Vec<(u32, usize)> = counts.into_iter().filter(|(id, _)| *id >= 2).collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.1));
    println!("# frame {stem} of map {}, region {},{},{},{}, scale {scale:.3}", map.0, rect.0, rect.1, rect.2, rect.3);
    println!("name\tkind\tid\tsize\tpixels\tgroup\tsurface\twrap");
    for (id, n) in &rows {
        if let Some(i) = by_id.get(id) {
            println!("{}	{}	{id}	{}x{}	{n}	{}	{}	{}", i.name, i.kind, i.w, i.h, i.group, i.surface, i.wrap);
        }
    }
    let uvp = map.1.join("frames").join(format!("{stem}-uv.png"));
    if let Ok(uv) = img::load(&uvp) {
        let names = id_names(p, &map)?;
        let cells = text::tile_cells(&ids, &uv, &names, rect, scale);
        if !cells.is_empty() {
            println!("# tile cells (corner x, y, size in output pixels), reading order");
            println!("tile\tx\ty\tsize\tpixels");
            for c in cells {
                println!("{}\t{:.1}\t{:.1}\t{:.1}\t{}", c.name, c.x, c.y, c.size, c.pixels);
            }
        }
    }
    Ok(true)
}

// --------------------------------------------------------------- select

/// `id_map`: the map numeric IDs are looked up in (they differ per map); None
/// with several maps makes a numeric token an error.
fn resolve_keys(
    inv: &[Inv],
    tokens: &[String],
    id_map: Option<&str>,
    several_maps: bool,
) -> Result<Vec<usize>, String> {
    let mut out = Vec::new();
    for t in tokens {
        let t = t.trim();
        if t.is_empty() {
            continue;
        }
        let hit: Vec<usize> =
            if let (true, Ok(id)) = (t.len() <= 7 && t.bytes().all(|b| b.is_ascii_digit()), t.parse::<u32>()) {
                if several_maps && id_map.is_none() {
                    return Err(format!("--key {t}: IDs differ per map; say --map NAME, or use the name or hash"));
                }
                inv.iter()
                    .enumerate()
                    .filter(|(_, i)| i.id == id && id_map.is_none_or(|m| i.map == m))
                    .map(|(n, _)| n)
                    .collect()
            } else {
                let exact: Vec<usize> = inv.iter().enumerate().filter(|(_, i)| i.name == t).map(|(n, _)| n).collect();
                if exact.is_empty() {
                    let (a, b) = (format!("tex_{t}_"), format!("tile_{t}"));
                    inv.iter()
                        .enumerate()
                        .filter(|(_, i)| i.name.starts_with(&a) || i.name == b || i.name.starts_with(&format!("{t}_")))
                        .map(|(n, _)| n)
                        .collect()
                } else {
                    exact
                }
            };
        if hit.is_empty() {
            return Err(format!("--key {t}: not in the map's inventory"));
        }
        out.extend(hit);
    }
    Ok(out)
}

fn select(p: &Project, a: &Args) -> Result<bool, String> {
    p.require()?;
    let inv = p.inventory()?;
    let several = p.maps().len() > 1;
    let mut idx: Vec<usize> = Vec::new();
    let keys: Vec<String> = a.all("--key").iter().flat_map(|k| k.split(',')).map(str::to_string).collect();
    idx.extend(resolve_keys(&inv, &keys, a.get("--map"), several)?);
    if let Some(g) = a.num::<u32>("--group")? {
        idx.extend(inv.iter().enumerate().filter(|(_, i)| i.group == g).map(|(n, _)| n));
    }
    if let Some(s) = a.get("--surface") {
        let hit: Vec<usize> =
            inv.iter().enumerate().filter(|(_, i)| i.surface == s && i.kind == "tex").map(|(n, _)| n).collect();
        if hit.is_empty() {
            return Err(format!("--surface {s}: no texture has it"));
        }
        idx.extend(hit);
    }
    if a.get("--region").is_some() {
        let rect = a.rect("--region")?.unwrap();
        let (_, ids, idimg, _, map) = frame_files(p, a)?;
        let counts = text::ids_in_region(&ids, idimg.width, rect);
        // This map's ids -> names -> rows of the merged inventory.
        let names = id_names(p, &map)?;
        let by_name: HashMap<&str, usize> = inv.iter().enumerate().map(|(n, i)| (i.name.as_str(), n)).collect();
        idx.extend(counts.keys().filter_map(|id| names.get(id)).filter_map(|n| by_name.get(n.as_str())).copied());
    }
    if a.has("--all") {
        idx.extend(0..inv.len());
    }
    if idx.is_empty() {
        return Err("select what? --key, --group, --surface, --frame with --region, or --all".into());
    }
    let kind = a.get("--kind");
    let min: u64 = a.num("--min-pixels")?.unwrap_or(0);
    let mut seen = BTreeSet::new();
    let idx: Vec<usize> = idx
        .into_iter()
        .filter(|&n| kind.is_none_or(|k| inv[n].kind == k) && inv[n].pixels >= min && seen.insert(n))
        .collect();

    let mut rows = p.selection();
    let (mut added, mut updated) = (0, 0);
    for n in idx {
        let i = &inv[n];
        let fresh = Sel {
            name: i.name.clone(),
            kind: i.kind.clone(),
            group: i.group,
            surface: i.surface.clone(),
            pixels: i.pixels,
            category: a.get("--category").unwrap_or("").to_string(),
            reason: a.get("--reason").unwrap_or("").to_string(),
        };
        match rows.iter_mut().find(|r| r.name == i.name) {
            Some(r) => {
                let before = r.clone();
                r.group = i.group;
                r.surface = i.surface.clone();
                r.pixels = i.pixels;
                if a.get("--category").is_some() {
                    r.category = fresh.category;
                }
                if a.get("--reason").is_some() {
                    r.reason = fresh.reason;
                }
                if *r != before {
                    updated += 1;
                }
            }
            None => {
                rows.push(fresh);
                added += 1;
            }
        }
        if let Some(src) = p.map_texture(&i.name, false) {
            let to = p.p("original").join(format!("{}.png", i.name));
            if !to.exists() {
                copy_file(&src, &to)?;
            }
        }
    }
    rows.sort_by(|x, y| x.name.cmp(&y.name));
    p.write_selection(&rows)?;
    println!(
        "selection     {added} added, {updated} changed, {} in all ({})",
        rows.len(),
        p.p("selection.tsv").display()
    );
    Ok(true)
}

// --------------------------------------------------------------- status

fn status(p: &Project, a: &Args) -> Result<bool, String> {
    p.require()?;
    let rows = p.selection();
    let ledger = p.ledger();
    let st: Vec<(&Sel, Status)> = rows.iter().map(|r| (r, p.status_of(&r.name))).collect();
    if a.has("--tsv") {
        println!("name\tkind\tgroup\tcategory\tstatus\ttool\tprompt\tdate");
        for (r, s) in &st {
            if a.has("--missing") && *s != Status::Original {
                continue;
            }
            let l = ledger.get(&r.name).cloned().unwrap_or(("-".into(), "-".into(), "-".into()));
            println!("{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}", r.name, r.kind, r.group, r.category, s.name(), l.0, l.1, l.2);
        }
        return Ok(!(a.has("--strict") && st.iter().any(|x| x.1 != Status::Approved)));
    }
    let count = |s: Status| st.iter().filter(|x| x.1 == s).count();
    let meta = p.meta();
    println!(
        "project       {} (game {})",
        meta.get("name").map(String::as_str).unwrap_or("?"),
        meta.get("game").map(String::as_str).unwrap_or("?")
    );
    println!(
        "selected      {}: {} original, {} drafted, {} approved",
        st.len(),
        count(Status::Original),
        count(Status::Drafted),
        count(Status::Approved)
    );
    let mut cats: BTreeMap<&str, [usize; 3]> = BTreeMap::new();
    for (r, s) in &st {
        let c = cats.entry(if r.category.is_empty() { "-" } else { &r.category }).or_default();
        c[*s as usize] += 1;
    }
    println!("{:<14}{:>9}{:>9}{:>9}", "category", "original", "drafted", "approved");
    for (c, n) in &cats {
        println!("{c:<14}{:>9}{:>9}{:>9}", n[0], n[1], n[2]);
    }
    let missing: Vec<&str> = st.iter().filter(|x| x.1 == Status::Original).map(|x| x.0.name.as_str()).collect();
    if !missing.is_empty() {
        let shown: Vec<&str> = missing.iter().take(if a.has("--missing") { usize::MAX } else { 8 }).copied().collect();
        println!(
            "missing       {} without a draft: {}{}",
            missing.len(),
            shown.join(" "),
            if shown.len() < missing.len() { " ..." } else { "" }
        );
    }
    let no_orig: Vec<&str> = rows.iter().filter(|r| p.original(&r.name).is_none()).map(|r| r.name.as_str()).collect();
    if !no_orig.is_empty() {
        println!("no original   {}: {}", no_orig.len(), no_orig.join(" "));
    }
    let sel: HashSet<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    for d in ["draft", "pack"] {
        let stray: Vec<String> = png_names(&p.p(d)).into_iter().filter(|n| !sel.contains(n.as_str())).collect();
        if !stray.is_empty() {
            println!("not selected  {} in {d}/: {}", stray.len(), stray.join(" "));
        }
    }
    let unrecorded = st.iter().filter(|x| x.1 != Status::Original && !ledger.contains_key(&x.0.name)).count();
    if unrecorded > 0 {
        println!("unrecorded    {unrecorded} images have no line in status/status.txt (made by hand)");
    }
    // With --strict the gate of `pack check`'s `selected`: every selected image approved.
    let short = st.iter().filter(|x| x.1 != Status::Approved).count();
    println!(
        "# selected gate: {short} of {} not approved (pass: 0{})",
        st.len(),
        if a.has("--strict") { "; exit 1 otherwise" } else { "; --strict makes it exit 1" }
    );
    Ok(!(a.has("--strict") && short > 0))
}

pub(crate) fn png_names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".png").map(str::to_string))
                .filter(|n| !n.contains('.'))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

// -------------------------------------------------------------- approve

fn approve(p: &Project, a: &Args) -> Result<bool, String> {
    p.require()?;
    let rows = if a.has("--drafted") {
        p.selection().into_iter().filter(|r| p.p("draft").join(format!("{}.png", r.name)).is_file()).collect::<Vec<_>>()
    } else {
        p.pick(a, true)?
    };
    let (mut n, mut skipped) = (0, 0);
    let mut kept: Vec<(String, &str)> = Vec::new();
    // The hash of each approved file, for the `stale` gate (status/approved.tsv).
    let mut records: Vec<(String, Option<u64>)> = Vec::new();
    let recorded = p.hashes("approved.tsv");
    for r in &rows {
        let (d, k) = (p.p("draft").join(format!("{}.png", r.name)), p.p("pack").join(format!("{}.png", r.name)));
        if a.has("--undo") {
            if k.is_file() {
                std::fs::remove_file(&k).map_err(|e| e.to_string())?;
                records.push((r.name.clone(), None));
                n += 1;
            }
        } else if d.is_file() {
            // Copy only what changed, so a second run does nothing; an
            // approved image that differs from the draft is replaced only
            // on request.
            let (draft, approved) = (std::fs::read(&d).ok(), std::fs::read(&k).ok());
            if draft != approved {
                if k.is_file() && !a.has("--replace") {
                    kept.push((r.name.clone(), "approved, and the draft differs"));
                } else {
                    copy_file(&d, &k)?;
                    records.extend(draft.map(|b| (r.name.clone(), Some(crate::gates::hash_bytes(&b)))));
                    n += 1;
                }
            } else if let Some(b) = draft {
                if !recorded.contains_key(&r.name) {
                    records.push((r.name.clone(), Some(crate::gates::hash_bytes(&b))));
                }
            }
        } else {
            skipped += 1;
        }
    }
    p.record_hashes("approved.tsv", &records)?;
    println!(
        "{}      {n} images, {skipped} without a draft",
        if a.has("--undo") { "unapproved" } else { "approved  " }
    );
    report_kept(&kept, n, false)?;
    Ok(true)
}

// --------------------------------------------------------------- import

/// Bring images made elsewhere (by hand, another project) into draft/,
/// recording `--tool` as their provenance; `--prompts DIR` also copies
/// their NAME.txt prompt files.
fn import(p: &Project, a: &Args) -> Result<bool, String> {
    p.require()?;
    let from = PathBuf::from(a.get("--from").ok_or("--from DIR: where the images are")?);
    let tool = a.get("--tool").ok_or("--tool TEXT: how they were made, for the record")?;
    let rows = p.pick(a, true)?;
    let (mut n, mut missing) = (0, 0);
    let mut kept: Vec<(String, &str)> = Vec::new();
    for r in &rows {
        let src = from.join(format!("{}.png", r.name));
        if !src.is_file() {
            missing += 1;
            continue;
        }
        if let Some(w) = p.blocked(&r.name, a) {
            kept.push((r.name.clone(), w));
            continue;
        }
        copy_file(&src, &p.p("draft").join(format!("{}.png", r.name)))?;
        let mut pf = "-".to_string();
        if let Some(pd) = a.get("--prompts") {
            let f = Path::new(pd).join(format!("{}.txt", r.name));
            if f.is_file() {
                copy_file(&f, &p.p("prompts").join(format!("{}.txt", r.name)))?;
                pf = format!("{}.txt", r.name);
            }
        }
        p.record(&r.name, tool, &pf)?;
        n += 1;
    }
    println!("imported      {n} images into draft/, {missing} not in {}", from.display());
    report_kept(&kept, n, true)?;
    Ok(true)
}

// ----------------------------------------------------------------- pack

fn pack(p: &Project, a: &Args) -> Result<bool, String> {
    p.require()?;
    if a.has("--approve-all") {
        let mut ap = vec!["--drafted".to_string()];
        if a.has("--replace") {
            ap.push("--replace".to_string());
        }
        approve(p, &Args::parse(&ap)?)?;
    }
    let dir = p.p("pack");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let meta = p.meta();
    let n_regions = regions::write_pack(p, &dir)?;
    let names = png_names(&dir);
    let mut manifest = String::from("# reModel pack, written by `remodel studio pack`\nformat\t2\nkeys\t1\n");
    let _ = writeln!(manifest, "name\t{}", meta.get("name").map(String::as_str).unwrap_or("studio pack"));
    if let Some(g) = meta.get("game") {
        let _ = writeln!(manifest, "game\t{g}");
    }
    if n_regions > 0 {
        manifest += "uses\tregions\n";
    }
    // Artwork images: `needs` where a colouring is drawn from one (fixed, or
    // not baked), else `uses` (the baked copies do the work; the art_ image
    // only covers colours nobody catalogued).
    if names.iter().any(|n| n.starts_with("art_")) {
        manifest += if p.needs_artworks() { "needs\tartworks\n" } else { "uses\tartworks\n" };
    }
    std::fs::write(dir.join("pack.txt"), manifest).map_err(|e| e.to_string())?;
    let mut index = String::from("# one file name a line, for the browser\n");
    for n in &names {
        let _ = writeln!(index, "{n}.png");
    }
    std::fs::write(dir.join("index.txt"), index).map_err(|e| e.to_string())?;
    println!(
        "pack          {} images in {} (pack.txt, index.txt{})",
        names.len(),
        dir.display(),
        if n_regions > 0 { format!(", {n_regions} regions") } else { String::new() }
    );
    if a.has("--no-lint") {
        return Ok(true);
    }
    crate::pack_lint::lint(&dir, None, 8192)
}

// ---------------------------------------------------------------- style

fn tile_wrap(i: Option<&Inv>, name: &str) -> (Wrap, Wrap) {
    img::parse_wrap(i.map(|i| i.wrap.as_str()).unwrap_or("-"), name.starts_with("tile_"))
}

fn style(p: &Project, a: &Args) -> Result<bool, String> {
    p.require()?;
    let rf_path = a.get("--recipe").ok_or("--recipe FILE")?;
    let rf = img::RecipeFile::parse(&std::fs::read_to_string(rf_path).map_err(|e| format!("{rf_path}: {e}"))?)?;
    let rows = p.pick(a, true)?;
    let inv: HashMap<String, Inv> =
        p.inventory().unwrap_or_default().into_iter().map(|i| (i.name.clone(), i)).collect();
    let from = a.get("--from").unwrap_or("original");
    let recipe_name = Path::new(rf_path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut done = 0;
    let mut kept: Vec<(String, &str)> = Vec::new();
    for r in &rows {
        if let Some(w) = p.blocked(&r.name, a) {
            kept.push((r.name.clone(), w));
            continue;
        }
        let src_path = if from == "draft" {
            p.p("draft").join(format!("{}.png", r.name))
        } else {
            p.original(&r.name).unwrap_or_default()
        };
        if !src_path.is_file() {
            println!("skipped       {}: no {from} image", r.name);
            continue;
        }
        let src = img::load(&src_path)?;
        let rec = rf.resolve(&r.category, &r.name).map_err(|e| format!("{}: {e}", r.name))?;
        let (wu, wv) = tile_wrap(inv.get(&r.name), &r.name);
        let out = img::style(&src, &rec, wu, wv);
        img::save(&p.p("draft").join(format!("{}.png", r.name)), &out)?;
        let body = format!(
            "local style processor\nrecipe\t{recipe_name} [material {}]\nfrom\t{from}\nwrap\t{} {}\n{rec:#?}\n",
            r.category,
            wu.name(),
            wv.name()
        );
        let pf = p.write_prompt(&r.name, &body)?;
        p.record(&r.name, &format!("style: {recipe_name}"), &pf)?;
        done += 1;
    }
    println!("styled        {done} of {} into draft/", rows.len());
    report_kept(&kept, done, true)?;
    Ok(true)
}

// ------------------------------------------------------------- variants

fn variants(p: &Project, a: &Args) -> Result<bool, String> {
    p.require()?;
    let inv = p.inventory()?;
    let of = a.get("--of").ok_or("--of KEY: the texture whose restyle feeds the surface")?;
    let several = p.maps().len() > 1;
    let k = *resolve_keys(&inv, &[of.to_string()], a.get("--map"), several)?.first().unwrap();
    let key = &inv[k];
    if key.kind != "tex" {
        return Err(format!("{}: tiles have no surfaces", key.name));
    }
    let base_path =
        a.get("--base").map(PathBuf::from).unwrap_or_else(|| p.p("draft").join(format!("{}.png", key.name)));
    let base = img::load(&base_path).map_err(|e| format!("{e} (restyle {} first, or --base IMG)", key.name))?;
    let reference = img::load(&p.original(&key.name).ok_or("the key's original is missing")?)?;
    let mode = match a.get("--mode") {
        Some(m) => img::TintMode::parse(m).ok_or("--mode: luma, ratio or mean")?,
        None => img::TintMode::Luma,
    };
    let strength: f32 = a.num("--strength")?.unwrap_or(1.0);
    let ink = img::parse_colour(a.get("--ink").unwrap_or("#111018"))?;
    let mut members: Vec<&Inv> =
        inv.iter().filter(|i| i.kind == "tex" && i.surface == key.surface && i.name != key.name).collect();
    // --also: keys of other surfaces that take the same look (a texture
    // drawn in colours the game made from other texels).
    let also: Vec<String> = a.all("--also").iter().flat_map(|k| k.split(',')).map(str::to_string).collect();
    for n in resolve_keys(&inv, &also, a.get("--map"), several)? {
        if inv[n].name != key.name && !members.iter().any(|m| m.name == inv[n].name) {
            members.push(&inv[n]);
        }
    }
    let mut rows = p.selection();
    let cat = rows.iter().find(|r| r.name == key.name).map(|r| r.category.clone()).unwrap_or_default();
    println!("surface       {} feeds {} other keys besides {}", key.surface, members.len(), key.name);
    let mut kept: Vec<(String, &str)> = Vec::new();
    let mut wrote = 0;
    for m in &members {
        if let Some(w) = p.blocked(&m.name, a) {
            if !a.has("--dry-run") {
                kept.push((m.name.clone(), w));
                continue;
            }
        }
        let Some(tp) = p.original(&m.name) else {
            println!("skipped       {}: no original", m.name);
            continue;
        };
        let target = img::load(&tp)?;
        let out = img::tint(&base, &reference, &target, mode, strength, ink);
        let mr = img::mean_rgb(&reference);
        let mt = img::mean_rgb(&target);
        println!(
            "variant       {}  mean {:.0},{:.0},{:.0} -> {:.0},{:.0},{:.0}",
            m.name, mr[0], mr[1], mr[2], mt[0], mt[1], mt[2]
        );
        if a.has("--dry-run") {
            continue;
        }
        img::save(&p.p("draft").join(format!("{}.png", m.name)), &out)?;
        wrote += 1;
        let pf = p.write_prompt(
            &m.name,
            &format!(
                "surface variant of {}\nmode\t{mode:?}\nstrength\t{strength}\nbase\t{}\n",
                key.name,
                base_path.display()
            ),
        )?;
        p.record(&m.name, &format!("variant of {} ({mode:?})", key.name), &pf)?;
        if !rows.iter().any(|r| r.name == m.name) {
            rows.push(Sel {
                name: m.name.clone(),
                kind: m.kind.clone(),
                group: m.group,
                surface: m.surface.clone(),
                pixels: m.pixels,
                category: cat.clone(),
                reason: format!("same surface as {}", key.name),
            });
            let to = p.p("original").join(format!("{}.png", m.name));
            if !to.exists() {
                copy_file(&tp, &to)?;
            }
        }
    }
    rows.sort_by(|x, y| x.name.cmp(&y.name));
    if !a.has("--dry-run") {
        p.write_selection(&rows)?;
    }
    report_kept(&kept, wrote, true)?;
    Ok(true)
}

// ----------------------------------------------------------------- text

fn typeset_with_magick(a: &Args, out: &Path) -> Result<(), String> {
    let text = a.get("--text").ok_or("--text STR or --image FILE")?;
    let size = a.get("--size").unwrap_or("96");
    let fill = a.get("--colour").unwrap_or("#ffffff");
    let stroke = a.get("--stroke").unwrap_or("none");
    let sw = a.get("--stroke-width").unwrap_or("0");
    let status = if let Some(t) = a.get("--typesetter") {
        Command::new("sh")
            .arg("-c")
            .arg(t)
            .env("STUDIO_TEXT", text)
            .env("STUDIO_FONT", a.get("--font").unwrap_or(""))
            .env("STUDIO_SIZE", size)
            .env("STUDIO_FILL", fill)
            .env("STUDIO_STROKE", stroke)
            .env("STUDIO_STROKEW", sw)
            .env("STUDIO_OUT", out)
            .status()
    } else {
        let mut c = Command::new("magick");
        c.args(["-background", "none"]);
        if let Some(f) = a.get("--font") {
            c.args(["-font", f]);
        }
        c.args(["-pointsize", size, "-fill", fill, "-stroke", stroke, "-strokewidth", sw]);
        c.arg(format!("label:{text}")).arg(out).status()
    };
    match status {
        Ok(s) if s.success() && out.is_file() => Ok(()),
        Ok(s) => Err(format!("the typesetter failed ({s})")),
        Err(e) => Err(format!("cannot run the typesetter: {e} (give --image a typeset PNG, or --typesetter a command using $STUDIO_TEXT, $STUDIO_FONT, $STUDIO_SIZE, $STUDIO_FILL, $STUDIO_STROKE, $STUDIO_STROKEW, $STUDIO_OUT; the default is ImageMagick's `magick`)")),
    }
}

/// The commonest colour (4 bits a channel) of a box of an image.
fn modal_colour(im: &Image, bx: (usize, usize, usize, usize)) -> [u8; 3] {
    let mut h: HashMap<[u8; 3], usize> = HashMap::new();
    for y in bx.1..(bx.1 + bx.3).min(im.height) {
        for x in bx.0..(bx.0 + bx.2).min(im.width) {
            let o = (y * im.width + x) * 4;
            *h.entry([im.rgba[o] & 0xf0, im.rgba[o + 1] & 0xf0, im.rgba[o + 2] & 0xf0]).or_default() += 1;
        }
    }
    let Some((bucket, _)) = h.into_iter().max_by_key(|(c, n)| (*n, *c)) else { return [0; 3] };
    let (mut sum, mut n) = ([0u64; 3], 0u64);
    for y in bx.1..(bx.1 + bx.3).min(im.height) {
        for x in bx.0..(bx.0 + bx.2).min(im.width) {
            let o = (y * im.width + x) * 4;
            if [im.rgba[o] & 0xf0, im.rgba[o + 1] & 0xf0, im.rgba[o + 2] & 0xf0] == bucket {
                for c in 0..3 {
                    sum[c] += im.rgba[o + c] as u64;
                }
                n += 1;
            }
        }
    }
    [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8]
}

/// `--pad`: N (all sides), X,Y (left and right, top and bottom) or
/// L,T,R,B, in output pixels; (left, top, right, bottom).
fn parse_pad(v: Option<&str>) -> Result<(f32, f32, f32, f32), String> {
    let Some(v) = v else { return Ok((0.0, 0.0, 0.0, 0.0)) };
    let n: Vec<f32> = v
        .split(',')
        .map(|x| x.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .map_err(|_| format!("--pad: `{v}` is not N, X,Y or LEFT,TOP,RIGHT,BOTTOM"))?;
    match n[..] {
        [a] => Ok((a, a, a, a)),
        [x, y] => Ok((x, y, x, y)),
        [l, t, r, b] => Ok((l, t, r, b)),
        _ => Err(format!("--pad: `{v}` is not N, X,Y or LEFT,TOP,RIGHT,BOTTOM")),
    }
}

fn text_cmd(p: &Project, a: &Args) -> Result<bool, String> {
    p.require()?;
    let region = a.rect("--region")?.ok_or("--region X,Y,W,H: the frame's pixels the text covers")?;
    let (stem, ids, idimg, scale, map) = frame_files(p, a)?;
    let uv = img::load(&map.1.join("frames").join(format!("{stem}-uv.png")))?;
    let names = id_names(p, &map)?;
    let cells = text::tile_cells(&ids, &uv, &names, region, scale);
    if cells.is_empty() {
        return Err("no tiles are drawn in that region".into());
    }
    let _ = idimg;
    // The typeset image.
    let ts_path = match a.get("--image") {
        Some(f) => PathBuf::from(f),
        None => {
            let f = p.p("work/text-typeset.png");
            std::fs::create_dir_all(p.p("work")).map_err(|e| e.to_string())?;
            typeset_with_magick(a, &f)?;
            f
        }
    };
    let mut ts = img::load(&ts_path)?;
    if a.get("--image").is_none() || a.get("--trim").is_some() {
        ts = text::trim(&ts, a.num("--margin")?.unwrap_or(0));
    }
    let bx = a.rect("--box")?.unwrap_or(region);
    let fit = match a.get("--fit").unwrap_or("contain") {
        "contain" => text::Fit::Contain,
        "stretch" => text::Fit::Stretch,
        o => return Err(format!("--fit {o}: contain or stretch")),
    };
    let align = match a.get("--align").unwrap_or("left") {
        "left" => text::Align::Left,
        "center" => text::Align::Center,
        "right" => text::Align::Right,
        o => return Err(format!("--align {o}: left, center or right")),
    };
    let mode = match a.get("--alpha").unwrap_or("original") {
        "original" => text::AlphaMode::Original,
        "typeset" => text::AlphaMode::Typeset,
        o => return Err(format!("--alpha {o}: original or typeset")),
    };
    let tile_px = 8 * a.num::<usize>("--tile-scale")?.unwrap_or(4).clamp(1, 16);
    let (pl, pt, pr, pb) = parse_pad(a.get("--pad"))?;
    let mut at = text::place(
        (ts.width, ts.height),
        (bx.0 as f32 + pl, bx.1 as f32 + pt, (bx.2 as f32 - pl - pr).max(1.0), (bx.3 as f32 - pt - pb).max(1.0)),
        fit,
        align,
    );
    if let Some(bg) = a.get("--bg") {
        // The text on a solid ground over the whole box, so what the old
        // text drew there is covered: `auto` takes the box's commonest
        // colour in the frame.
        let colour = if bg == "auto" {
            modal_colour(&img::load(&map.1.join("frames").join(format!("{stem}.png")))?, bx)
        } else {
            img::parse_colour(bg)?
        };
        let k = at.w / ts.width as f32;
        let (cw, ch) = (((bx.2 as f32 / k).round() as usize).max(1), ((bx.3 as f32 / k).round() as usize).max(1));
        let mut canvas = img::blank(cw, ch, [colour[0], colour[1], colour[2], 255]);
        let (ox, oy) = (((at.x - bx.0 as f32) / k).round() as isize, ((at.y - bx.1 as f32) / k).round() as isize);
        for y in 0..ts.height {
            for x in 0..ts.width {
                let (dx, dy) = (x as isize + ox, y as isize + oy);
                if dx < 0 || dy < 0 || dx >= cw as isize || dy >= ch as isize {
                    continue;
                }
                let (so, d) = ((y * ts.width + x) * 4, (dy as usize * cw + dx as usize) * 4);
                let al = ts.rgba[so + 3] as f32 / 255.0;
                for c in 0..3 {
                    canvas.rgba[d + c] =
                        (ts.rgba[so + c] as f32 * al + canvas.rgba[d + c] as f32 * (1.0 - al)).round() as u8;
                }
            }
        }
        ts = canvas;
        at = text::Placement { x: bx.0 as f32, y: bx.1 as f32, w: bx.2 as f32, h: bx.3 as f32 };
    }
    // One image a key: the cell with the most pixels stands for it.
    let mut best: BTreeMap<&str, &text::Cell> = BTreeMap::new();
    let mut dup: BTreeMap<&str, usize> = BTreeMap::new();
    for c in &cells {
        *dup.entry(&c.name).or_default() += 1;
        if best.get(c.name.as_str()).is_none_or(|b| c.pixels > b.pixels) {
            best.insert(&c.name, c);
        }
    }
    println!("cells         {} drawn, {} distinct tiles", cells.len(), best.len());
    for (n, k) in &dup {
        if *k > 1 {
            println!("shared        {n} is drawn {k} times in the region; its image follows the biggest one");
        }
    }
    let mut rows = p.selection();
    let cat = a.get("--category").unwrap_or("text").to_string();
    let reason = a.get("--reason").map(str::to_string).unwrap_or_else(|| {
        format!("text over frame {stem} region {},{},{},{}", region.0, region.1, region.2, region.3)
    });
    let mut kept: Vec<(String, &str)> = Vec::new();
    for (name, c) in &best {
        if !a.has("--dry-run") {
            if let Some(w) = p.blocked(name, a) {
                kept.push((name.to_string(), w));
                continue;
            }
        }
        let orig = p.original(name).and_then(|f| img::load(&f).ok());
        let out = text::slice_cell(&ts, &at, c, orig.as_ref(), tile_px, mode);
        if a.has("--dry-run") {
            continue;
        }
        img::save(&p.p("draft").join(format!("{name}.png")), &out)?;
        let pf = p.write_prompt(name, &format!("text to tiles\ntypeset\t{}\nframe\t{stem}\nregion\t{region:?}\nbox\t{bx:?}\nfit\t{fit:?} {align:?}\nalpha\t{mode:?}\n", ts_path.display()))?;
        p.record(name, "text to tiles", &pf)?;
        if !rows.iter().any(|r| r.name == *name) {
            let i = p.inventory()?.into_iter().find(|i| i.name == *name);
            rows.push(Sel {
                name: name.to_string(),
                kind: "tile".into(),
                group: i.as_ref().map(|i| i.group).unwrap_or(0),
                surface: "0".into(),
                pixels: i.as_ref().map(|i| i.pixels).unwrap_or(0),
                category: cat.clone(),
                reason: reason.clone(),
            });
            if let Some(f) = p.map_texture(name, false) {
                let to = p.p("original").join(format!("{name}.png"));
                if !to.exists() {
                    copy_file(&f, &to)?;
                }
            }
        }
    }
    if !a.has("--dry-run") {
        rows.sort_by(|x, y| x.name.cmp(&y.name));
        p.write_selection(&rows)?;
        println!("wrote         {} tile images into draft/", best.len() - kept.len());
    }
    report_kept(&kept, best.len() - kept.len(), true)?;
    Ok(true)
}

// ----------------------------------------------------------------- seam

fn seam(p: &Project, a: &Args) -> Result<bool, String> {
    p.require()?;
    let from = a.get("--from").unwrap_or("pack");
    let inv: HashMap<String, Inv> =
        p.inventory().unwrap_or_default().into_iter().map(|i| (i.name.clone(), i)).collect();
    let tsv = a.has("--tsv");
    println!("name\twrap_u\twrap_v\tu_jump\tu_ratio\tv_jump\tv_ratio\tverdict\tnote");
    let (mut checked, mut suspects) = (0, 0);
    for name in png_names(&p.p(from)) {
        let im = img::load(&p.p(from).join(format!("{name}.png")))?;
        let (wu, wv) = asset_gates::wraps(inv.get(&name), &name);
        let (once_u, once_v) = match inv.get(&name).filter(|i| i.kind == "tex") {
            Some(i) => (i.span_u <= 1.0, i.span_v <= 1.0),
            None => (false, false),
        };
        let u = img::seam_axis(&im, true, wu);
        let v = img::seam_axis(&im, false, wv);
        // Does the original have the same seam? Then it is the game's own.
        let orig = p.original(&name).and_then(|f| img::load(&f).ok());
        let (ou, ov) = orig.as_ref().map(|o| (img::seam_axis(o, true, wu), img::seam_axis(o, false, wv))).unzip();
        checked += 1;
        let bad_u = u.suspect && !ou.as_ref().is_some_and(|o| o.suspect);
        let bad_v = v.suspect && !ov.as_ref().is_some_and(|o| o.suspect);
        let (verdict, note) = match (bad_u, bad_v) {
            (false, false) => ("ok", ""),
            (true, false) => ("suspect", "left/right edges"),
            (false, true) => ("suspect", "top/bottom edges"),
            (true, true) => ("suspect", "both axes"),
        };
        suspects += (verdict == "suspect") as usize;
        if tsv && verdict == "ok" && !a.has("--strict") {
            // quiet rows are still printed: a TSV is the whole truth
        }
        println!(
            "{name}\t{}\t{}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{verdict}\t{note}",
            if once_u { "once" } else { wu.name() },
            if once_v { "once" } else { wv.name() },
            u.jump,
            u.ratio,
            v.jump,
            v.ratio
        );
    }
    println!(
        "# seam check: {checked} images in {from}/, {suspects} suspect (pass: edge jump at most {} levels or {}x the typical step; a warning, an error with --strict)",
        crate::gates::SEAM_LEVELS,
        crate::gates::SEAM_RATIO
    );
    Ok(!(a.has("--strict") && suspects > 0))
}

// ---------------------------------------------------------------- draft

struct Brief {
    prompt: String,
    refs: Vec<PathBuf>,
    scale: usize,
    context: String,
}

fn parse_brief(path: &Path) -> Result<Brief, String> {
    let t = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let base = path.parent().unwrap_or(Path::new("."));
    let mut b = Brief { prompt: String::new(), refs: vec![], scale: 4, context: "frame".into() };
    for l in t.lines() {
        if let Some(d) = l.strip_prefix('@') {
            let (k, v) = d.split_once(char::is_whitespace).unwrap_or((d, ""));
            let v = v.trim();
            match k {
                "ref" => b.refs.push(base.join(v)),
                "scale" => b.scale = v.parse().map_err(|_| format!("brief: @scale {v}"))?,
                "context" if ["none", "frame", "group", "both"].contains(&v) => b.context = v.to_string(),
                _ => {
                    return Err(format!(
                        "brief: unknown directive `@{k} {v}` (@ref FILE, @scale N, @context none|frame|group|both)"
                    ))
                }
            }
        } else {
            b.prompt.push_str(l);
            b.prompt.push('\n');
        }
    }
    if b.prompt.trim().is_empty() {
        return Err("the brief has no prompt text".into());
    }
    Ok(b)
}

fn aspect_text(w: usize, h: usize) -> String {
    fn g(a: usize, b: usize) -> usize {
        if b == 0 {
            a
        } else {
            g(b, a % b)
        }
    }
    let d = g(w, h).max(1);
    format!("{}:{}", w / d, h / d)
}

/// The frame crop where `i` is drawn most (its best frame), or None.
fn frame_crop(p: &Project, i: &Inv) -> Option<Image> {
    let stem = frame_stem(&i.best_frame).ok()?;
    let frames = p.map_dir(&i.map).join("frames");
    let tsv = std::fs::read_to_string(frames.join(format!("{stem}.tsv"))).ok()?;
    let mut lines = tsv.lines();
    let head: Vec<&str> = lines.next()?.split('\t').collect();
    let col = |n: &str| head.iter().position(|h| *h == n);
    let (ci, c0, c1, c2, c3) = (col("id")?, col("x0")?, col("y0")?, col("x1")?, col("y1")?);
    let row = lines
        .map(|l| l.split('\t').collect::<Vec<_>>())
        .find(|f| f.get(ci).and_then(|v| v.parse::<u32>().ok()) == Some(i.id))?;
    let n = |c: usize| row.get(c).and_then(|v| v.parse::<usize>().ok());
    let (x0, y0, x1, y1) = (n(c0)?, n(c1)?, n(c2)?, n(c3)?);
    let frame = img::load(&frames.join(format!("{stem}.png"))).ok()?;
    let m = ((x1 - x0).max(y1 - y0) / 4).max(24);
    let (cx0, cy0) = (x0.saturating_sub(m), y0.saturating_sub(m));
    let (cx1, cy1) = ((x1 + m).min(frame.width - 1), (y1 + m).min(frame.height - 1));
    Some(img::crop(&frame, cx0, cy0, cx1 - cx0 + 1, cy1 - cy0 + 1))
}

/// A sheet of the group's other members (their drafts where there are any).
fn group_sheet(p: &Project, inv: &[Inv], i: &Inv) -> Option<Image> {
    let others: Vec<&Inv> =
        inv.iter().filter(|o| o.group == i.group && o.name != i.name && o.kind == i.kind).take(8).collect();
    if others.is_empty() || i.group == 0 {
        return None;
    }
    let cell = 160usize;
    let cols = 4usize.min(others.len());
    let rows = others.len().div_ceil(cols);
    let mut sheet = img::blank(cols * cell, rows * cell, [60, 60, 60, 255]);
    for (n, o) in others.iter().enumerate() {
        let path = [p.p("draft").join(format!("{}.png", o.name))]
            .into_iter()
            .find(|f| f.is_file())
            .or_else(|| p.original(&o.name));
        let Some(im) = path.and_then(|f| img::load(&f).ok()) else { continue };
        let k = (cell - 8) as f32 / im.width.max(im.height) as f32;
        let (w, h) = (((im.width as f32 * k) as usize).max(1), ((im.height as f32 * k) as usize).max(1));
        let t = img::resize(&im, w, h);
        let (ox, oy) = ((n % cols) * cell + (cell - w) / 2, (n / cols) * cell + (cell - h) / 2);
        for y in 0..h {
            for x in 0..w {
                let s = (y * w + x) * 4;
                let d = ((oy + y) * sheet.width + ox + x) * 4;
                sheet.rgba[d..d + 3].copy_from_slice(&t.rgba[s..s + 3]);
            }
        }
    }
    Some(sheet)
}

fn run_job(
    cmd: &str,
    dir: &Path,
    prompt_file: &Path,
    retries: usize,
    timeout: u64,
    env: &[(&str, String)],
) -> Result<(), String> {
    let log_path = dir.join("run.log");
    let mut last = String::new();
    for attempt in 1..=retries + 1 {
        let _ = std::fs::remove_file(dir.join("out.png"));
        let log = std::fs::OpenOptions::new().create(true).append(true).open(&log_path).map_err(|e| e.to_string())?;
        let mut log2 = log.try_clone().map_err(|e| e.to_string())?;
        {
            use std::io::Write as _;
            let _ = writeln!(log2, "== attempt {attempt}: {cmd}");
        }
        let mut c = Command::new("sh");
        c.arg("-c")
            .arg(cmd)
            .current_dir(dir)
            .stdin(Stdio::from(std::fs::File::open(prompt_file).map_err(|e| e.to_string())?));
        c.stdout(Stdio::from(log)).stderr(Stdio::from(log2));
        for (k, v) in env {
            c.env(k, v);
        }
        let mut child = c.spawn().map_err(|e| format!("cannot start `{cmd}`: {e}"))?;
        let start = std::time::Instant::now();
        let status = loop {
            match child.try_wait().map_err(|e| e.to_string())? {
                Some(s) => break Some(s),
                None if start.elapsed().as_secs() > timeout => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                None => std::thread::sleep(std::time::Duration::from_millis(300)),
            }
        };
        match status {
            Some(s) if s.success() && img::load(&dir.join("out.png")).is_ok() => return Ok(()),
            Some(s) => {
                last = format!(
                    "attempt {attempt}: exit {s}, {}",
                    if dir.join("out.png").is_file() { "out.png does not decode" } else { "no out.png" }
                )
            }
            None => last = format!("attempt {attempt}: timed out after {timeout}s"),
        }
        if attempt <= retries {
            std::thread::sleep(std::time::Duration::from_secs(2 * attempt as u64));
        }
    }
    Err(last)
}

fn draft(p: &Project, a: &Args) -> Result<bool, String> {
    p.require()?;
    let brief_path =
        PathBuf::from(a.get("--brief").ok_or("--brief FILE: the prompt, with @ref/@scale/@context lines")?);
    let mut brief = parse_brief(&brief_path)?;
    for r in a.all("--ref") {
        brief.refs.push(PathBuf::from(r));
    }
    if let Some(s) = a.num::<usize>("--scale")? {
        brief.scale = s;
    }
    for r in &brief.refs {
        if !r.is_file() {
            return Err(format!("reference image {} not found", r.display()));
        }
    }
    let template = a.get("--cmd").unwrap_or("codex exec {images} -").to_string();
    let rows = p.pick(a, true)?;
    let inv = p.inventory().unwrap_or_default();
    let jobs: usize = a.num("--jobs")?.unwrap_or(3);
    let retries: usize = a.num("--retries")?.unwrap_or(2);
    let timeout: u64 = a.num("--timeout")?.unwrap_or(900);
    if a.has("--force") {
        warn("--force is --replace here; use --replace");
    }
    let (force, dry) = (a.has("--replace") || a.has("--force"), a.has("--dry-run"));

    // Prepare every job (cheap), then run those without a draft.
    struct Job {
        name: String,
        dir: PathBuf,
        w: usize,
        h: usize,
        cmd: String,
        prompt: String,
        inputs: Vec<String>,
    }
    let mut todo = Vec::new();
    let mut skipped = 0;
    for r in &rows {
        if !force && p.p("draft").join(format!("{}.png", r.name)).is_file() {
            skipped += 1;
            continue;
        }
        let Some(op) = p.original(&r.name) else {
            println!("skipped       {}: no original", r.name);
            continue;
        };
        let orig = img::load(&op)?;
        let jd = p.p("work/draft").join(&r.name);
        std::fs::create_dir_all(&jd).map_err(|e| e.to_string())?;
        let k = (512 / orig.width.max(orig.height)).clamp(2, 16);
        let mut files = vec!["in1.png".to_string()];
        let mut desc = vec!["Image 1: the original texture, enlarged".to_string()];
        img::save(&jd.join("in1.png"), &img::nearest(&orig, k))?;
        for (n, rf) in brief.refs.iter().enumerate() {
            let f = format!("ref{}.png", n + 1);
            // Reference images may be any format the png codec reads.
            copy_file(rf, &jd.join(&f))?;
            files.push(f);
            desc.push(format!(
                "Image {}: reference {}",
                files.len(),
                rf.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
            ));
        }
        let me = inv.iter().find(|i| i.name == r.name);
        if let Some(me) = me {
            if matches!(brief.context.as_str(), "frame" | "both") {
                if let Some(c) = frame_crop(p, me) {
                    let k = (768.0f32 / c.width.max(c.height) as f32).min(1.0);
                    let c = img::resize(
                        &c,
                        ((c.width as f32 * k) as usize).max(1),
                        ((c.height as f32 * k) as usize).max(1),
                    );
                    img::save(&jd.join("ctx-frame.png"), &c)?;
                    files.push("ctx-frame.png".into());
                    desc.push(format!(
                        "Image {}: where the texture is used in the game (context only, do not copy its look)",
                        files.len()
                    ));
                }
            }
            if matches!(brief.context.as_str(), "group" | "both") {
                if let Some(s) = group_sheet(p, &inv, me) {
                    img::save(&jd.join("ctx-group.png"), &s)?;
                    files.push("ctx-group.png".into());
                    desc.push(format!("Image {}: the other textures of the same object (restyled ones show the target look); match them", files.len()));
                }
            }
        }
        let mut prompt = brief.prompt.clone();
        for (k, v) in [
            ("{name}", r.name.clone()),
            ("{w}", orig.width.to_string()),
            ("{h}", orig.height.to_string()),
            ("{aspect}", aspect_text(orig.width, orig.height)),
            ("{scale}", brief.scale.to_string()),
            ("{category}", r.category.clone()),
            ("{group}", r.group.to_string()),
            ("{reason}", r.reason.clone()),
            ("{images}", desc.join(". ") + "."),
        ] {
            prompt = prompt.replace(k, &v);
        }
        if !prompt.contains("out.png") {
            prompt.push_str("\nSave the result as out.png in the current directory. Reply with the file path only.\n");
        }
        std::fs::write(jd.join("prompt.txt"), &prompt).map_err(|e| e.to_string())?;
        let imgs: Vec<String> = files.iter().map(|f| format!("-i {f}")).collect();
        let cmd = template
            .replace("{images}", &imgs.join(" "))
            .replace("{files}", &files.join(" "))
            .replace("{prompt_file}", "prompt.txt")
            .replace("{out}", "out.png");
        todo.push(Job { name: r.name.clone(), dir: jd, w: orig.width, h: orig.height, cmd, prompt, inputs: files });
    }
    println!("jobs          {} to run, {} already drafted (--replace redoes)", todo.len(), skipped);
    if skipped > 0 {
        warn(&format!("{skipped} already drafted or approved and kept; --replace redoes them"));
    }
    if dry {
        for j in &todo {
            println!("{}\t{}\t{}", j.name, j.dir.display(), j.cmd);
        }
        return Ok(true);
    }
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(String, Result<(), String>)>> = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..jobs.clamp(1, 16) {
            s.spawn(|| loop {
                let n = next.fetch_add(1, Ordering::SeqCst);
                let Some(j) = todo.get(n) else { break };
                let env =
                    [("STUDIO_NAME", j.name.clone()), ("STUDIO_W", j.w.to_string()), ("STUDIO_H", j.h.to_string())];
                let r = run_job(&j.cmd, &j.dir, &j.dir.join("prompt.txt"), retries, timeout, &env).and_then(|_| {
                    finish_draft(p, j.name.as_str(), &j.dir, j.w, j.h, brief.scale, a.has("--keep-size"))
                });
                match &r {
                    Ok(()) => {
                        let tool = template.split_whitespace().next().unwrap_or("generator");
                        let note = format!(
                            "{tool}: {}",
                            brief_path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
                        );
                        let body = format!(
                            "{}\n---\ncommand\t{}\ninputs\t{}\nbrief\t{}\n",
                            j.prompt,
                            j.cmd,
                            j.inputs.join(" "),
                            brief_path.display()
                        );
                        let _ = p.write_prompt(&j.name, &body).and_then(|pf| p.record(&j.name, &note, &pf));
                        println!("drafted       {}", j.name);
                    }
                    Err(e) => println!("failed        {}: {e} (log: {})", j.name, j.dir.join("run.log").display()),
                }
                results.lock().unwrap().push((j.name.clone(), r));
            });
        }
    });
    let res = results.into_inner().unwrap();
    let failed = res.iter().filter(|r| r.1.is_err()).count();
    println!("draft         {} drafted, {failed} failed, {skipped} skipped", res.len() - failed);
    Ok(failed == 0)
}

fn finish_draft(
    p: &Project,
    name: &str,
    jd: &Path,
    w: usize,
    h: usize,
    scale: usize,
    keep: bool,
) -> Result<(), String> {
    let out = img::load(&jd.join("out.png"))?;
    let mut im = if keep {
        out
    } else {
        // Crop to the original's proportions (a generator's frame is not
        // always the aspect asked for), then size to scale x original.
        let want = w as f32 / h as f32;
        let have = out.width as f32 / out.height as f32;
        let cropped = if (have / want - 1.0).abs() > 0.02 {
            if have > want {
                let nw = (out.height as f32 * want) as usize;
                img::crop(&out, (out.width - nw) / 2, 0, nw, out.height)
            } else {
                let nh = (out.width as f32 / want) as usize;
                img::crop(&out, 0, (out.height - nh) / 2, out.width, nh)
            }
        } else {
            out
        };
        img::resize(&cropped, w * scale, h * scale)
    };
    inherit_cutout(&mut im, p.original(name).and_then(|f| img::load(&f).ok()).as_ref());
    img::save(&p.p("draft").join(format!("{name}.png")), &im)
}

/// A cut-out texture's transparent texels stay transparent in its restyle:
/// an image model returns an opaque picture, and drawn as it is the
/// cut-out becomes a solid rectangle. The original's alpha, resampled to
/// the draft's size, is applied (kept hard where the original's was hard;
/// the lower of the two where the draft has its own transparency). A draft
/// of an opaque original is left alone.
fn inherit_cutout(im: &mut Image, original: Option<&Image>) {
    let Some(o) = original else { return };
    if o.rgba.chunks_exact(4).all(|p| p[3] == 255) {
        return;
    }
    let hard = o.rgba.chunks_exact(4).all(|p| p[3] == 0 || p[3] == 255);
    let mask = img::resize(o, im.width, im.height);
    for (d, m) in im.rgba.chunks_exact_mut(4).zip(mask.rgba.chunks_exact(4)) {
        let a = if hard {
            if m[3] >= 128 {
                255
            } else {
                0
            }
        } else {
            m[3]
        };
        d[3] = d[3].min(a);
    }
}

// -------------------------------------------------------------- compare

fn compare(p: &Project, a: &Args, ctx: &Ctx) -> Result<bool, String> {
    p.require()?;
    if let Some(names) = a.get("--scene") {
        return compare_scenes(p, a, ctx, names);
    }
    let pack_dir = a.get("--pack").map(PathBuf::from).unwrap_or_else(|| p.p("pack"));
    let out = a.get("--into").map(PathBuf::from).unwrap_or_else(|| p.p("compare"));
    let steps = a.get("--steps").unwrap_or("600000000").to_string();
    let snap = a.get("--snap").unwrap_or("300").to_string();
    let extra: Vec<String> = a.get("--run-args").unwrap_or("").split_whitespace().map(str::to_string).collect();
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // From any directory: the repo root (where `rom/` and `out/` are) is
    // found by walking up from the working directory, the project and this
    // program; the runs start there, with absolute paths.
    let root = find_root(p).ok_or(
        "cannot find the repo root (a folder with Makefile and crates/) above the working directory or the project",
    )?;
    let rom = if ctx.rom.is_absolute() || ctx.rom.exists() {
        std::fs::canonicalize(&ctx.rom).unwrap_or(ctx.rom.clone())
    } else {
        find_up(&[root.clone(), cwd()], &ctx.rom)
            .ok_or_else(|| format!("ROM directory {} not found from {}", ctx.rom.display(), root.display()))?
    };
    let eeprom = find_up(&[root.clone(), cwd()], &PathBuf::from("out").join(&ctx.game).join("eeprom.bin"))
        .unwrap_or_else(|| root.join("out").join(&ctx.game).join("eeprom.bin"));
    let out = absolute(&out);
    let pack_dir = absolute(&pack_dir);
    // The pack as it is now: a copy taken at the start, so a pack being
    // rebuilt while this runs is never seen half written (a copy, not
    // links: writers truncate files in place).
    let snapshot = out.join(".pack-snapshot");
    if !pack_dir.is_dir() {
        return Err(format!("{}: pack directory not found", pack_dir.display()));
    }
    let _ = std::fs::remove_dir_all(&snapshot);
    copy_dir(&pack_dir, &snapshot)?;
    let force = a.has("--force");
    let jobs: usize = a.num("--jobs")?.unwrap_or(2);
    let mut kids = Vec::new();
    for (label, pack) in [("before", None), ("after", Some(&snapshot))] {
        let od = out.join(label);
        // Old frames are never compared silently: a pack changed since is
        // not what they show (an agent once lost a lot of time to this).
        if !force && od.join("frames/final.png").is_file() {
            return Err(format!(
                "{} already holds a run's frames; compare never reuses old output: --force runs again, or --into a new folder",
                od.display()
            ));
        }
        std::fs::create_dir_all(&od).map_err(|e| e.to_string())?;
        if eeprom.is_file() {
            let _ = std::fs::copy(&eeprom, od.join("eeprom.bin"));
        }
        let mut c = Command::new(&exe);
        c.current_dir(&root);
        c.args(["--game", &ctx.game, "--rom"]).arg(&rom);
        if ctx.jit {
            c.arg("--jit");
        }
        if let Some(i) = &ctx.input {
            c.args(["--input", i]);
        }
        c.args(&extra).arg("--out").arg(&od);
        if let Some(pk) = pack {
            c.arg("--texture-pack").arg(pk);
        }
        c.args(["run", &steps, &snap]);
        let log = std::fs::File::create(od.join("run.log")).map_err(|e| e.to_string())?;
        c.stdout(Stdio::from(log.try_clone().map_err(|e| e.to_string())?)).stderr(Stdio::from(log));
        kids.push((label, c));
    }
    // At most `jobs` at once.
    let mut running: Vec<(&str, std::process::Child)> = Vec::new();
    let mut failed = false;
    let mut queue = kids.into_iter();
    loop {
        while running.len() < jobs.max(1) {
            match queue.next() {
                Some((l, mut c)) => {
                    println!("running       {l}");
                    running.push((l, c.spawn().map_err(|e| format!("{e}"))?));
                }
                None => break,
            }
        }
        if running.is_empty() {
            break;
        }
        let mut i = 0;
        while i < running.len() {
            match running[i].1.try_wait().map_err(|e| e.to_string())? {
                Some(s) => {
                    let (l, _) = running.remove(i);
                    println!("finished      {l} ({s})");
                    failed |= !s.success();
                }
                None => i += 1,
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    let _ = std::fs::remove_dir_all(&snapshot);
    if failed {
        return Err(format!("a run failed: see {}/before/run.log and after/run.log", out.display()));
    }
    let stems = |d: &str| -> BTreeSet<String> {
        png_names(&out.join(d).join("frames")).into_iter().filter(|n| n.chars().all(|c| c.is_ascii_digit())).collect()
    };
    let mut common: Vec<String> = stems("before").intersection(&stems("after")).cloned().collect();
    if let Some(f) = a.get("--frames") {
        let want: Vec<String> = f.split(',').map(frame_stem).collect::<Result<_, _>>()?;
        common.retain(|c| want.contains(c));
    }
    if common.is_empty() {
        return Err("no frames in both runs".into());
    }
    let mut tsv = String::from("frame\tchanged_pct\tmean_abs_diff\n");
    let mut sheets = Vec::new();
    for f in &common {
        let b = img::load(&out.join("before/frames").join(format!("{f}.png")))?;
        let af = img::load(&out.join("after/frames").join(format!("{f}.png")))?;
        let (changed, mad) = frame_diff(&b, &af);
        let _ = writeln!(tsv, "{f}\t{changed:.1}\t{mad:.2}");
        let sheet = side_by_side(&b, &af);
        img::save(&out.join(format!("sheet-{f}.png")), &sheet)?;
        sheets.push(sheet);
    }
    std::fs::write(out.join("compare.tsv"), &tsv).map_err(|e| e.to_string())?;
    overview(&out, &sheets)?;
    println!(
        "compare       {} frames: {}/sheet-FRAME.png (before | after), sheet-all.png, compare.tsv",
        common.len(),
        out.display()
    );
    Ok(true)
}

/// One page of a few evenly spaced sheets: `sheet-all.png`.
fn overview(out: &Path, sheets: &[Image]) -> Result<(), String> {
    // One page of a few evenly spaced frames.
    let pick: Vec<&Image> = if sheets.len() <= 4 {
        sheets.iter().collect()
    } else {
        (0..4).map(|i| &sheets[i * (sheets.len() - 1) / 3]).collect()
    };
    let width = pick.iter().map(|s| s.width).max().unwrap_or(0);
    let k = (2400.0f32 / width as f32).min(1.0);
    let scaled: Vec<Image> =
        pick.iter().map(|s| img::resize(s, (s.width as f32 * k) as usize, (s.height as f32 * k) as usize)).collect();
    let h: usize = scaled.iter().map(|s| s.height).sum();
    let mut all = img::blank(scaled.iter().map(|s| s.width).max().unwrap_or(1), h.max(1), [30, 30, 30, 255]);
    let mut y = 0;
    for s in &scaled {
        all.rgba[y * all.width * 4..(y * all.width * 4) + s.rgba.len()].copy_from_slice(&s.rgba);
        y += s.height;
    }
    img::save(&out.join("sheet-all.png"), &all)?;
    Ok(())
}

/// `compare --scene`: the same before and after from scenes' captures, with
/// no run and no ROM (docs/uplift-studio.md, "Scenes"). Every output is
/// drawn fresh, so nothing old is read back.
fn compare_scenes(p: &Project, a: &Args, ctx: &Ctx, names: &str) -> Result<bool, String> {
    use crate::scene;
    let game = romset::games::find(&ctx.game).map_err(|e| e.to_string())?;
    let pack_dir = a.get("--pack").map(PathBuf::from).unwrap_or_else(|| p.p("pack"));
    if !pack_dir.is_dir() {
        return Err(format!("{}: pack directory not found", pack_dir.display()));
    }
    let out = absolute(&a.get("--into").map(PathBuf::from).unwrap_or_else(|| p.p("compare")));
    let rs = scene::recipes(game, Some(&p.dir))?;
    let all: Vec<String> = rs.iter().map(|r| r.name.clone()).collect();
    let want: Vec<String> =
        if names == "all" { all.clone() } else { names.split(',').map(|n| n.trim().to_string()).collect() };
    for n in &want {
        if !all.contains(n) {
            return Err(crate::answer::Answer::unknown("scene", n, &all).message());
        }
    }
    let view = scene::parse_view(a.get("--view")).map_err(|e| e.message())?;
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut gpu = gpu::Renderer::new().map_err(|e| format!("no GPU: {e}"))?;
    let root = scene::scenes_dir(game);
    let mut tsv = String::from("scene\tchanged_pct\tmean_abs_diff\n");
    let mut sheets = Vec::new();
    for n in &want {
        let dir = root.join(n);
        let opts = scene::DrawOpts { pack: vec![pack_dir.clone()], crop: None, against: true, view };
        let d = scene::draw(game, &dir, &opts, &mut gpu).map_err(|e| {
            format!("{}{}", e.message(), e.next.as_ref().map(|n| format!(" (next: {n})")).unwrap_or_default())
        })?;
        for w in &d.warnings {
            eprintln!("warning: {w}");
        }
        let to_img = |p: &scene::Picture| Image {
            width: p.width,
            height: p.height,
            rgba: p.rgb.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        };
        let (b, af) = (to_img(d.original.as_ref().ok_or("no original drawn")?), to_img(&d.picture));
        let (changed, mad) = frame_diff(&b, &af);
        let _ = writeln!(tsv, "{n}\t{changed:.1}\t{mad:.2}");
        let sheet = side_by_side(&b, &af);
        img::save(&out.join(format!("sheet-{n}.png")), &sheet)?;
        sheets.push(sheet);
    }
    std::fs::write(out.join("compare.tsv"), &tsv).map_err(|e| e.to_string())?;
    overview(&out, &sheets)?;
    println!(
        "compare       {} scenes: {}/sheet-SCENE.png (before | after), sheet-all.png, compare.tsv",
        want.len(),
        out.display()
    );
    Ok(true)
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_default()
}

fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd().join(p)
    }
}

/// The first of `rel` under `starts` or any of their ancestors that exists.
fn find_up(starts: &[PathBuf], rel: &Path) -> Option<PathBuf> {
    for s in starts {
        let s = std::fs::canonicalize(s).unwrap_or_else(|_| s.clone());
        for d in s.ancestors() {
            let c = d.join(rel);
            if c.exists() {
                return Some(c);
            }
        }
    }
    None
}

/// The repo root: the nearest folder above the working directory, the
/// project or this program with a `Makefile` and a `crates/` folder.
fn find_root(p: &Project) -> Option<PathBuf> {
    let exe = std::env::current_exe().unwrap_or_default();
    for s in [cwd(), absolute(&p.dir), exe] {
        let s = std::fs::canonicalize(&s).unwrap_or(s);
        if let Some(d) = s.ancestors().find(|d| d.join("Makefile").is_file() && d.join("crates").is_dir()) {
            return Some(d.to_path_buf());
        }
    }
    None
}

fn side_by_side(a: &Image, b: &Image) -> Image {
    let gap = 8;
    let mut s = img::blank(a.width + b.width + gap, a.height.max(b.height), [30, 30, 30, 255]);
    for (im, x0) in [(a, 0), (b, a.width + gap)] {
        for y in 0..im.height {
            let d = (y * s.width + x0) * 4;
            s.rgba[d..d + im.width * 4].copy_from_slice(&im.rgba[y * im.width * 4..(y + 1) * im.width * 4]);
        }
    }
    s
}

fn frame_diff(a: &Image, b: &Image) -> (f32, f32) {
    if a.width != b.width || a.height != b.height {
        return (100.0, 255.0);
    }
    let (mut n, mut sum) = (0usize, 0u64);
    for (x, y) in a.rgba.chunks_exact(4).zip(b.rgba.chunks_exact(4)) {
        let d: u32 = (0..3).map(|c| (x[c] as i32 - y[c] as i32).unsigned_abs()).sum();
        if d > 6 {
            n += 1;
        }
        sum += d as u64;
    }
    let px = (a.width * a.height).max(1);
    (100.0 * n as f32 / px as f32, sum as f32 / (3 * px) as f32)
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../out/scratch/studio-tests"))
            .join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn args(s: &str) -> Args {
        Args::parse(&s.split_whitespace().map(str::to_string).collect::<Vec<_>>()).unwrap()
    }

    /// A two-texture, one-tile "map".
    fn map(dir: &Path) {
        let m = dir.join("map");
        std::fs::create_dir_all(m.join("textures")).unwrap();
        std::fs::create_dir_all(m.join("frames")).unwrap();
        std::fs::write(
            m.join("inventory.tsv"),
            "id\tname\tkind\tw\th\tsurface\tframes\tpixels\tbest_frame\tbest_pixels\tpolys\tuv_used_pct\twrap\tspan_u\tspan_v\tgroup\tgroup_size\tdetail\n\
             2\ttex_aaaaaaaaaaaaaaaa_8x8\ttex\t8\t8\tS1\t1\t900\t300\t900\t1\t100\trr\t2\t2\t1\t2\t-\n\
             3\ttex_bbbbbbbbbbbbbbbb_8x8\ttex\t8\t8\tS1\t1\t500\t300\t500\t1\t100\trr\t2\t2\t1\t2\t-\n\
             4\ttile_cccccccccccccccc\ttile\t8\t8\t0\t1\t100\t300\t100\t1\t100\t-\t1\t1\t2\t1\t-\n",
        )
        .unwrap();
        for (n, c) in [
            ("tex_aaaaaaaaaaaaaaaa_8x8", [200u8, 100, 50]),
            ("tex_bbbbbbbbbbbbbbbb_8x8", [50, 100, 200]),
            ("tile_cccccccccccccccc", [255, 255, 255]),
        ] {
            img::save(&m.join("textures").join(format!("{n}.png")), &img::blank(8, 8, [c[0], c[1], c[2], 255]))
                .unwrap();
        }
        // a 16x16 frame: ids image with tex 2 left half, tex 3 right half
        let mut ids = img::blank(16, 16, [0, 0, 0, 255]);
        for y in 0..16 {
            for x in 0..16 {
                ids.rgba[(y * 16 + x) * 4] = if x < 8 { 2 } else { 3 };
            }
        }
        img::save(&m.join("frames/000300-ids.png"), &ids).unwrap();
    }

    #[test]
    fn project_select_status_approve_pack() {
        let d = tmp("flow");
        let p = Project { dir: d.clone() };
        init(&p, &args(&format!("{} --name demo --game g", d.display()))).unwrap();
        map(&d);
        // selection by key hash, id, surface and frame region
        select(&p, &args(&format!("{} --key aaaaaaaaaaaaaaaa --category wall --reason first", d.display()))).unwrap();
        select(&p, &args(&format!("{} --key 4 --reason tile", d.display()))).unwrap();
        select(&p, &args(&format!("{} --frame 300 --region 8,0,8,16 --category wall", d.display()))).unwrap();
        let rows = p.selection();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].category, "wall");
        assert_eq!(rows[0].reason, "first");
        assert_eq!(rows[1].name, "tex_bbbbbbbbbbbbbbbb_8x8");
        // idempotent
        let before = std::fs::read_to_string(d.join("selection.tsv")).unwrap();
        select(&p, &args(&format!("{} --key aaaaaaaaaaaaaaaa,4", d.display()))).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("selection.tsv")).unwrap(), before);
        assert!(d.join("original/tile_cccccccccccccccc.png").is_file());
        assert!(select(&p, &args(&format!("{} --key nope", d.display()))).is_err());
        // by surface adds nothing new; by group of tile
        select(&p, &args(&format!("{} --surface S1 --kind tex", d.display()))).unwrap();
        assert_eq!(p.selection().len(), 3);
        // status
        assert_eq!(p.status_of("tex_aaaaaaaaaaaaaaaa_8x8"), Status::Original);
        img::save(&d.join("draft/tex_aaaaaaaaaaaaaaaa_8x8.png"), &img::blank(32, 32, [1, 2, 3, 255])).unwrap();
        assert_eq!(p.status_of("tex_aaaaaaaaaaaaaaaa_8x8"), Status::Drafted);
        approve(&p, &args(&format!("{} --drafted", d.display()))).unwrap();
        assert_eq!(p.status_of("tex_aaaaaaaaaaaaaaaa_8x8"), Status::Approved);
        assert_eq!(p.status_of("tex_bbbbbbbbbbbbbbbb_8x8"), Status::Original);
        p.record("tex_aaaaaaaaaaaaaaaa_8x8", "test", "-").unwrap();
        p.record("tex_aaaaaaaaaaaaaaaa_8x8", "test2", "-").unwrap();
        assert_eq!(p.ledger().len(), 1);
        assert_eq!(p.ledger()["tex_aaaaaaaaaaaaaaaa_8x8"].0, "test2");
        status(&p, &args(&format!("{}", d.display()))).unwrap();
        // pack: manifest and index
        assert!(pack(&p, &args(&format!("{} --no-lint", d.display()))).unwrap());
        let idx = std::fs::read_to_string(d.join("pack/index.txt")).unwrap();
        assert!(idx.contains("tex_aaaaaaaaaaaaaaaa_8x8.png") && !idx.contains("bbbb"));
        assert!(std::fs::read_to_string(d.join("pack/pack.txt")).unwrap().contains("game\tg"));
        approve(&p, &args(&format!("{} --undo tex_aaaaaaaaaaaaaaaa_8x8", d.display()))).unwrap();
        assert_eq!(p.status_of("tex_aaaaaaaaaaaaaaaa_8x8"), Status::Drafted);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn surface_variants_tint_each_key_from_its_original() {
        let d = tmp("variants");
        let p = Project { dir: d.clone() };
        init(&p, &args(&format!("{}", d.display()))).unwrap();
        map(&d);
        select(&p, &args(&format!("{} --key aaaaaaaaaaaaaaaa --category ground", d.display()))).unwrap();
        // the restyle of key a: flat grey, 4x
        img::save(&d.join("draft/tex_aaaaaaaaaaaaaaaa_8x8.png"), &img::blank(32, 32, [150, 150, 150, 255])).unwrap();
        variants(&p, &args(&format!("{} --of aaaaaaaaaaaaaaaa", d.display()))).unwrap();
        let v = img::load(&d.join("draft/tex_bbbbbbbbbbbbbbbb_8x8.png")).unwrap();
        // b's original is blue where a's is orange: the variant is bluer
        assert!(v.rgba[2] > v.rgba[0], "{:?}", &v.rgba[..4]);
        assert_eq!(v.width, 32);
        let sel = p.selection();
        let b = sel.iter().find(|r| r.name.contains("bbbb")).unwrap();
        assert_eq!(b.category, "ground");
        assert!(b.reason.contains("same surface"));
        assert!(p.ledger()["tex_bbbbbbbbbbbbbbbb_8x8"].0.starts_with("variant of"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn region_picker_lists_ids() {
        let d = tmp("region");
        let p = Project { dir: d.clone() };
        init(&p, &args(&format!("{}", d.display()))).unwrap();
        map(&d);
        assert!(region(&p, &args(&format!("{} --frame 300 --region 0,0,8,8", d.display()))).unwrap());
        assert!(region(&p, &args(&format!("{} --frame 301 --region 0,0,8,8", d.display()))).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn seam_reports_only_what_the_original_lacked() {
        let d = tmp("seam");
        let p = Project { dir: d.clone() };
        init(&p, &args(&format!("{}", d.display()))).unwrap();
        map(&d);
        // a smooth ramp original and a pack image with a dark top row
        let mut ramp = img::blank(16, 16, [0, 0, 0, 255]);
        for y in 0..16 {
            for x in 0..16 {
                let v = (128.0 + 100.0 * (x as f32 / 16.0 * std::f32::consts::TAU).sin()) as u8;
                ramp.rgba[(y * 16 + x) * 4..][..3].copy_from_slice(&[v, v, v]);
            }
        }
        let name = "tex_aaaaaaaaaaaaaaaa_8x8";
        img::save(&d.join(format!("original/{name}.png")), &ramp).unwrap();
        let mut bad = ramp.clone();
        for x in 0..16 {
            bad.rgba[x * 4..x * 4 + 3].copy_from_slice(&[0, 0, 0]);
        }
        img::save(&d.join(format!("pack/{name}.png")), &bad).unwrap();
        assert!(seam(&p, &args(&format!("{} --strict", d.display()))).is_ok_and(|ok| !ok));
        img::save(&d.join(format!("pack/{name}.png")), &ramp).unwrap();
        assert!(seam(&p, &args(&format!("{} --strict", d.display()))).unwrap());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn brief_directives_and_prompt() {
        let d = tmp("brief");
        let f = d.join("b.md");
        std::fs::write(&f, "@ref logo.png\n@scale 2\n@context both\nRedraw {name} as {aspect}.\n").unwrap();
        let b = parse_brief(&f).unwrap();
        assert_eq!((b.scale, b.context.as_str()), (2, "both"));
        assert!(b.refs[0].ends_with("logo.png") && b.prompt.trim() == "Redraw {name} as {aspect}.");
        std::fs::write(&f, "@bogus 1\nx\n").unwrap();
        assert!(parse_brief(&f).is_err());
        assert_eq!(aspect_text(128, 64), "2:1");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn draft_runner_runs_a_template_resumes_and_sizes() {
        let d = tmp("draft");
        let p = Project { dir: d.clone() };
        init(&p, &args(&format!("{}", d.display()))).unwrap();
        map(&d);
        select(&p, &args(&format!("{} --key aaaaaaaaaaaaaaaa,bbbbbbbbbbbbbbbb --category x", d.display()))).unwrap();
        std::fs::write(d.join("brief.md"), "Make {name}. Save out.png.\n").unwrap();
        // The "generator": copies the first input to out.png (a 2:1 image would be cropped to 1:1).
        let cmd = "cp in1.png out.png; cat > /dev/null; echo {images} > args.txt";
        let line = format!("{} --brief {}/brief.md --all --jobs 2 --scale 2 --retries 0", d.display(), d.display());
        let mut a = args(&line);
        a.flags.push(("--cmd".into(), cmd.into()));
        assert!(draft(&p, &a).unwrap());
        let out = img::load(&d.join("draft/tex_aaaaaaaaaaaaaaaa_8x8.png")).unwrap();
        assert_eq!((out.width, out.height), (16, 16));
        assert!(std::fs::read_to_string(d.join("prompts/tex_aaaaaaaaaaaaaaaa_8x8.txt"))
            .unwrap()
            .contains("Make tex_aaaaaaaaaaaaaaaa_8x8"));
        assert!(
            p.ledger()["tex_bbbbbbbbbbbbbbbb_8x8"].0.starts_with("sh:")
                || p.ledger()["tex_bbbbbbbbbbbbbbbb_8x8"].0.contains("brief.md")
        );
        // resumable: a second run finds both done and runs nothing (the command would fail)
        let mut a2 = args(&line);
        a2.flags.push(("--cmd".into(), "false".into()));
        assert!(draft(&p, &a2).unwrap());
        // a failing generator fails the job and leaves no draft
        std::fs::remove_file(d.join("draft/tex_bbbbbbbbbbbbbbbb_8x8.png")).unwrap();
        let mut a3 = args(&line);
        a3.flags.push(("--cmd".into(), "exit 1".into()));
        assert!(!draft(&p, &a3).unwrap());
        assert!(!d.join("draft/tex_bbbbbbbbbbbbbbbb_8x8.png").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn names_and_filters_are_strict() {
        let d = tmp("strict");
        let p = Project { dir: d.clone() };
        init(&p, &args(&format!("{}", d.display()))).unwrap();
        map(&d);
        select(&p, &args(&format!("{} --key aaaaaaaaaaaaaaaa,bbbbbbbbbbbbbbbb --category wall", d.display()))).unwrap();
        let recipe = d.join("r.txt");
        std::fs::write(&recipe, "[default]\ntones = 3\n").unwrap();
        let run = |extra: &str| style(&p, &args(&format!("{} --recipe {} {extra}", d.display(), recipe.display())));
        // An unknown name refuses, naming it; nothing is restyled.
        let e = run("tex_nope").unwrap_err();
        assert!(e.contains("not in the selection") && e.contains("tex_nope"), "{e}");
        assert!(run("aaaaaaaaaaaaaaaa tex_nope").is_err());
        assert!(!d.join("draft/tex_aaaaaaaaaaaaaaaa_8x8.png").exists());
        // A substring of a name is not a name; the key is.
        assert!(run("aaaaaaaa").is_err());
        assert!(run("aaaaaaaaaaaaaaaa").unwrap());
        assert!(d.join("draft/tex_aaaaaaaaaaaaaaaa_8x8.png").is_file());
        assert!(!d.join("draft/tex_bbbbbbbbbbbbbbbb_8x8.png").exists());
        // Unknown group or category: an error, never every row.
        assert!(run("--category nope").unwrap_err().contains("categories: wall"));
        assert!(run("--group 99").unwrap_err().contains("group 99"));
        // With nothing to narrow by, `--all` is needed.
        assert!(run("").unwrap_err().contains("which rows"));
        // Filters together all apply.
        let e = run("aaaaaaaaaaaaaaaa --category nope").unwrap_err();
        assert!(e.contains("--category"), "{e}");
        assert!(approve(&p, &args(&format!("{} nothing", d.display()))).is_err());
        assert!(import(&p, &args(&format!("{} --from {} --tool x tex_none", d.display(), d.display()))).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn several_maps_are_searched_together() {
        let d = tmp("maps");
        let p = Project { dir: d.clone() };
        let (m1, m2) = (d.join("m1"), d.join("m2"));
        for (m, name, id) in [(&m1, "tex_aaaaaaaaaaaaaaaa_8x8", 2), (&m2, "tex_dddddddddddddddd_8x8", 2)] {
            std::fs::create_dir_all(m.join("textures")).unwrap();
            std::fs::create_dir_all(m.join("frames")).unwrap();
            std::fs::write(
                m.join("inventory.tsv"),
                format!(
                    "id\tname\tkind\tw\th\tsurface\tframes\tpixels\tbest_frame\tbest_pixels\tpolys\tuv_used_pct\twrap\tspan_u\tspan_v\tgroup\tgroup_size\tdetail\n\
                     {id}\t{name}\ttex\t8\t8\tS\t1\t900\t300\t900\t1\t100\trr\t2\t2\t1\t1\t-\n"
                ),
            )
            .unwrap();
            img::save(&m.join("textures").join(format!("{name}.png")), &img::blank(8, 8, [9, 9, 9, 255])).unwrap();
            let mut ids = img::blank(16, 16, [0, 0, 0, 255]);
            ids.rgba.chunks_exact_mut(4).for_each(|c| c[0] = id as u8);
            img::save(&m.join("frames/000300-ids.png"), &ids).unwrap();
        }
        init(&p, &args(&format!("{0} --map one={1} --map two={2}", d.display(), m1.display(), m2.display()))).unwrap();
        assert_eq!(p.maps().iter().map(|m| m.0.as_str()).collect::<Vec<_>>(), ["one", "two"]);
        let inv = p.inventory().unwrap();
        assert_eq!(inv.len(), 2);
        assert_eq!((inv[0].map.as_str(), inv[1].map.as_str()), ("one", "two"));
        // Select by name from either map; the originals come from the map that has them.
        select(&p, &args(&format!("{} --key dddddddddddddddd,aaaaaaaaaaaaaaaa", d.display()))).unwrap();
        assert!(d.join("original/tex_dddddddddddddddd_8x8.png").is_file());
        assert!(d.join("original/tex_aaaaaaaaaaaaaaaa_8x8.png").is_file());
        // IDs differ per map: a number needs --map; a frame in both maps needs --map.
        assert!(select(&p, &args(&format!("{} --key 2", d.display()))).unwrap_err().contains("IDs differ per map"));
        assert!(select(&p, &args(&format!("{} --key 2 --map two", d.display()))).unwrap());
        let e = region(&p, &args(&format!("{} --frame 300 --region 0,0,8,8", d.display()))).unwrap_err();
        assert!(e.contains("several maps"), "{e}");
        assert!(region(&p, &args(&format!("{} --frame 300 --region 0,0,8,8 --map one", d.display()))).unwrap());
        assert!(region(&p, &args(&format!("{} --frame 300 --region 0,0,8,8 --map zzz", d.display()))).is_err());
        // Selecting what a frame region draws goes through that map's ids.
        let before = p.selection().len();
        select(&p, &args(&format!("{} --frame 300 --region 0,0,8,8 --map two", d.display()))).unwrap();
        assert_eq!(p.selection().len(), before);
        // A single linked folder is still one map named `map`.
        let one = tmp("onemap");
        let q = Project { dir: one.clone() };
        init(&q, &args(&format!("{} --map {}", one.display(), m1.display()))).unwrap();
        assert_eq!(q.maps().iter().map(|m| m.0.as_str()).collect::<Vec<_>>(), ["map"]);
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&one);
    }

    #[test]
    fn pad_takes_one_two_or_four_values() {
        assert_eq!(parse_pad(None).unwrap(), (0.0, 0.0, 0.0, 0.0));
        assert_eq!(parse_pad(Some("3")).unwrap(), (3.0, 3.0, 3.0, 3.0));
        assert_eq!(parse_pad(Some("4,2")).unwrap(), (4.0, 2.0, 4.0, 2.0));
        assert_eq!(parse_pad(Some("1,2,3,4")).unwrap(), (1.0, 2.0, 3.0, 4.0));
        assert!(parse_pad(Some("1,2,3")).is_err() && parse_pad(Some("x")).is_err());
    }

    #[test]
    fn the_repo_root_is_found_from_the_project() {
        let d = tmp("root");
        std::fs::create_dir_all(d.join("crates")).unwrap();
        std::fs::write(d.join("Makefile"), "").unwrap();
        std::fs::create_dir_all(d.join("out/g/studio/p")).unwrap();
        std::fs::create_dir_all(d.join("rom")).unwrap();
        let p = Project { dir: d.join("out/g/studio/p") };
        assert!(find_root(&p).is_some());
        let found = find_up(&[d.join("out/g/studio/p")], Path::new("rom")).unwrap();
        assert_eq!(found, std::fs::canonicalize(d.join("rom")).unwrap());
        assert!(find_up(&[d.join("out")], Path::new("no/such/thing")).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn region_capture_cuts_a_layout_and_the_pack_carries_it() {
        use model2::capture::Frame;
        use model2::regions::RegionSet;
        let d = tmp("regioncap");
        let p = Project { dir: d.clone() };
        init(&p, &args(&format!("{} --game g", d.display()))).unwrap();
        // Plane A (map 0): tiles 1 and 2 at cells (10, 10) and (11, 10), scrolled 5 pixels
        // right; the other maps off.
        let mut tiles = vec![0u8; 0x10_0000];
        let put = |t: &mut [u8], off: usize, v: u16| t[off..off + 2].copy_from_slice(&v.to_le_bytes());
        put(&mut tiles, (10 * 64 + 10) * 2, 1);
        put(&mut tiles, (10 * 64 + 11) * 2, 2);
        for m in 1..4 {
            put(&mut tiles, 0xa008 + 2 * m, 0x8000);
        }
        put(&mut tiles, 0xa000, 5);
        tiles[0x8_0000 + 32..0x8_0000 + 64].fill(0x11);
        tiles[0x8_0000 + 64..0x8_0000 + 96].fill(0x22);
        let mut palette = vec![0u8; 0x2_0000];
        palette[2..4].copy_from_slice(&0x03e0u16.to_le_bytes());
        palette[4..6].copy_from_slice(&0x03ffu16.to_le_bytes());
        let frame = Frame {
            frame: 300,
            stream: vec![],
            texture_point_ram: vec![0; 4],
            render_ram: vec![0; 8],
            palette,
            tiles: tiles.clone(),
            xoff: 0.0,
            yoff: 0.0,
            light: None,
            scan: None,
        };
        let cap = d.join("cap");
        frame.save(&cap, &[0]).unwrap();
        // The cells are at screen x = 85..101, y = 80..88 (5 pixels of scroll).
        let line = format!(
            "{} logo --capture-dir {} --frame 300 --region 86,81,14,6 --game-px --ref-scale 2",
            d.display(),
            cap.display()
        );
        assert!(regions::capture(&p, &args(&line)).unwrap());
        let text = std::fs::read_to_string(d.join("regions/logo/region.txt")).unwrap();
        let defs = model2::regions::parse(&text).unwrap();
        assert_eq!((defs[0].w, defs[0].h), (2, 1));
        assert_eq!(defs[0].plane, model2::regions::Plane::A);
        let o = img::load(&d.join("regions/logo/original.png")).unwrap();
        assert_eq!((o.width, o.height), (32, 16));
        // The original is the game's picture of those cells: green then yellow.
        assert_eq!(&o.rgba[..3], &[0, 255, 0]);
        assert_eq!(&o.rgba[(16 * 4)..(16 * 4) + 3], &[255, 255, 0]);
        // No replacement yet: nothing goes into the pack.
        assert!(pack(&p, &args(&format!("{} --no-lint", d.display()))).unwrap());
        assert!(!d.join("pack/regions.txt").exists());
        assert!(!std::fs::read_to_string(d.join("pack/pack.txt")).unwrap().contains("regions"));
        let mut listing = args(&format!("{}", d.display()));
        listing.sw.insert("--tsv".into());
        assert!(regions::list(&p, &listing).unwrap());
        // Give it one (4x): the pack then has the regions file, the image and the feature.
        img::save(&d.join("regions/logo/replacement.png"), &img::blank(64, 32, [200, 10, 10, 255])).unwrap();
        assert!(pack(&p, &args(&format!("{}", d.display()))).unwrap());
        assert!(d.join("pack/region_logo.png").is_file());
        assert!(std::fs::read_to_string(d.join("pack/pack.txt")).unwrap().contains("uses\tregions"));
        assert!(std::fs::read_to_string(d.join("pack/index.txt")).unwrap().contains("region_logo.png"));
        // A reader loads it and finds it in the frame, where the scroll put it.
        let set = RegionSet::load_dir(&d.join("pack"), &mut |f| {
            let bytes = std::fs::read(f).ok()?;
            let i = png::decode(&bytes).ok()?;
            Some(std::sync::Arc::new(model2::enhance::Rgba { width: i.width, height: i.height, data: i.rgba }))
        })
        .unwrap();
        assert_eq!(set.regions.len(), 1);
        let grid = model2::video::match_regions(&tiles, &frame.palette, &set);
        assert!(grid.at(0, 10, 10).is_some() && grid.at(0, 11, 10).is_some());
        // Unapproving (removing the replacement) takes it back out of the pack.
        std::fs::remove_file(d.join("regions/logo/replacement.png")).unwrap();
        assert!(pack(&p, &args(&format!("{} --no-lint", d.display()))).unwrap());
        assert!(!d.join("pack/region_logo.png").exists() && !d.join("pack/regions.txt").exists());
        // Refusals: a bad name, an off-screen rectangle.
        let bad =
            format!("{} Bad --capture-dir {} --frame 300 --region 86,81,14,6 --game-px", d.display(), cap.display());
        assert!(regions::capture(&p, &args(&bad)).is_err());
        let off =
            format!("{} off --capture-dir {} --frame 300 --region 600,300,14,6 --game-px", d.display(), cap.display());
        assert!(regions::capture(&p, &args(&off)).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn today_is_a_date() {
        let t = today();
        assert_eq!(t.len(), 10);
        assert!(t.starts_with("20"));
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn ctx() -> Ctx {
        Ctx { game: "g".into(), rom: PathBuf::from("rom"), jit: false, input: None }
    }

    /// Defect (vision, "The defects"): `--keep-size` was read but missing
    /// from the switches, so it took the next argument as its value.
    #[test]
    fn keep_size_is_a_switch_and_options_are_strict() {
        let a = Args::parse(&strings(&["P", "--keep-size", "tex_a", "--brief", "b.txt"])).unwrap();
        assert!(a.has("--keep-size"));
        assert_eq!(a.pos, vec!["P", "tex_a"]);
        assert_eq!(a.get("--brief"), Some("b.txt"));
        // an option the subcommand does not read is an error naming the near ones
        let allowed = OPTIONS.iter().find(|(n, _)| *n == "draft").unwrap().1;
        let bad = Args::parse(&strings(&["P", "--replce", "x"])).unwrap();
        let e = bad.check("draft", allowed).unwrap_err();
        assert!(e.contains("did you mean --replace"), "{e}");
        let bad = Args::parse(&strings(&["P", "--tsv"])).unwrap();
        assert!(bad.check("draft", allowed).unwrap_err().contains("unknown option `--tsv`"));
        // --json is global
        Args::parse(&strings(&["P", "--json"])).unwrap().check("draft", allowed).unwrap();
        // an unknown subcommand lists the near ones, before anything is read
        let e = main(&strings(&["drat", "P"]), &ctx()).unwrap_err();
        assert!(e.contains("did you mean draft"), "{e}");
        // the same before parsing: an unknown option is named, not "wants a value"
        let e = main(&strings(&["draft", "P", "--replce"]), &ctx()).unwrap_err();
        assert!(e.contains("did you mean --replace"), "{e}");
        // a value starting with -- is a value
        let e = main(&strings(&["compare", "P", "--run-args", "--height 1080"]), &ctx()).unwrap_err();
        assert!(!e.contains("unknown option"), "{e}");
    }

    /// Defect: a generated draft is an opaque picture; a cut-out texture's
    /// transparent texels must stay transparent in it.
    #[test]
    fn a_draft_inherits_the_cutouts_alpha() {
        let d = tmp("cutout");
        let p = Project { dir: d.clone() };
        init(&p, &args(&format!("{} --name demo --game g", d.display()))).unwrap();
        map(&d);
        select(&p, &args(&format!("{} --key aaaaaaaaaaaaaaaa --category wall", d.display()))).unwrap();
        let name = "tex_aaaaaaaaaaaaaaaa_8x8";
        // the original is a cut-out: its left half is transparent
        let mut cut = img::blank(8, 8, [200, 100, 50, 255]);
        for y in 0..8 {
            for x in 0..4 {
                cut.rgba[(y * 8 + x) * 4 + 3] = 0;
            }
        }
        img::save(&d.join(format!("original/{name}.png")), &cut).unwrap();
        // the "generator" returns an opaque picture of the wrong look
        img::save(&d.join("generated.png"), &img::blank(32, 32, [0, 200, 0, 255])).unwrap();
        img::save(&d.join("small.png"), &img::blank(16, 16, [0, 0, 200, 255])).unwrap();
        std::fs::write(d.join("brief.txt"), "Redraw {name}.\n@scale 4\n@context none\n").unwrap();
        let run = |rf: &str, extra: &[&str]| {
            let mut v = vec![
                d.to_string_lossy().to_string(),
                "--brief".into(),
                d.join("brief.txt").to_string_lossy().to_string(),
                "--ref".into(),
                d.join(rf).to_string_lossy().to_string(),
                "--cmd".into(),
                "cp ref1.png out.png".into(),
                "--all".into(),
                "--jobs".into(),
                "1".into(),
                "--retries".into(),
                "0".into(),
            ];
            v.extend(extra.iter().map(|x| x.to_string()));
            draft(&p, &Args::parse(&v).unwrap())
        };
        assert!(run("generated.png", &[]).unwrap());
        let got = img::load(&d.join(format!("draft/{name}.png"))).unwrap();
        assert_eq!((got.width, got.height), (32, 32));
        let at = |x: usize, y: usize| got.rgba[(y * 32 + x) * 4..][..4].to_vec();
        assert_eq!(at(3, 10)[3], 0, "the left half stays cut out");
        assert_eq!(at(20, 10), vec![0, 200, 0, 255], "the right half is the new picture");
        // a second run keeps the draft; --replace with --keep-size redoes it at the
        // generator's own size (16x16), the cut-out resampled to it
        let before = std::fs::read(d.join(format!("draft/{name}.png"))).unwrap();
        assert!(run("small.png", &[]).unwrap());
        assert_eq!(std::fs::read(d.join(format!("draft/{name}.png"))).unwrap(), before);
        assert!(run("small.png", &["--keep-size", "--replace"]).unwrap());
        let small = img::load(&d.join(format!("draft/{name}.png"))).unwrap();
        assert_eq!((small.width, small.height), (16, 16));
        assert_eq!(small.rgba[(5 * 16 + 2) * 4 + 3], 0);
        assert_eq!(small.rgba[(5 * 16 + 12) * 4 + 3], 255);
        // an opaque original leaves a draft opaque
        let mut im = img::blank(4, 4, [1, 2, 3, 255]);
        inherit_cutout(&mut im, Some(&img::blank(8, 8, [9, 9, 9, 255])));
        assert!(im.rgba.chunks_exact(4).all(|q| q[3] == 255));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Nothing overwrites a draft or an approved image without --replace.
    #[test]
    fn nothing_is_overwritten_without_replace() {
        let d = tmp("replace");
        let p = Project { dir: d.clone() };
        init(&p, &args(&format!("{} --name demo --game g", d.display()))).unwrap();
        map(&d);
        select(&p, &args(&format!("{} --key aaaaaaaaaaaaaaaa --category wall", d.display()))).unwrap();
        let name = "tex_aaaaaaaaaaaaaaaa_8x8";
        let from = d.join("from");
        let put = |v: u8| img::save(&from.join(format!("{name}.png")), &img::blank(32, 32, [v, v, v, 255])).unwrap();
        let red = |dir: &str| img::load(&d.join(format!("{dir}/{name}.png"))).unwrap().rgba[0];
        let imp = |extra: &str| {
            import(&p, &args(&format!("{} --from {} --tool test --all{extra}", d.display(), from.display())))
        };
        put(9);
        assert!(imp("").unwrap());
        assert_eq!(red("draft"), 9);
        // a second import refuses, naming the fix, and changes nothing
        put(7);
        let e = imp("").unwrap_err();
        assert!(e.contains("kept") && e.contains("--replace"), "{e}");
        assert_eq!(red("draft"), 9);
        assert!(imp(" --replace").unwrap());
        assert_eq!(red("draft"), 7);
        // approve copies; a changed draft does not replace the approved image unless asked
        assert!(approve(&p, &args(&format!("{} --drafted", d.display()))).unwrap());
        assert_eq!(red("pack"), 7);
        put(5);
        assert!(imp(" --replace").unwrap());
        assert!(approve(&p, &args(&format!("{} --drafted", d.display()))).unwrap());
        assert_eq!(red("pack"), 7, "the approved image stays");
        assert!(approve(&p, &args(&format!("{} --drafted --replace", d.display()))).unwrap());
        assert_eq!(red("pack"), 5);
        // style: a draft stands in the way too
        let rf = d.join("recipe.txt");
        std::fs::write(&rf, "[default]\n").unwrap();
        let e = style(&p, &args(&format!("{} --recipe {} --all", d.display(), rf.display()))).unwrap_err();
        assert!(e.contains("kept"), "{e}");
        assert_eq!(red("draft"), 5);
        let _ = std::fs::remove_dir_all(&d);
    }
}
