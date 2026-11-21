//! `remodel scene ...`: named scenes, and drawing them with no emulator
//! (docs/uplift-studio.md, "Scenes"; the design is in
//! docs/uplift/scenes-and-server.md).
//!
//! A scene is a recipe (a game's input script and a frame; the manifest's
//! `scenes` and a project's `scenes.txt`) and a local copy made from it
//! (`<data dir>/<set>/scenes/<name>/`: `scene.txt`, `capture/`, `state.save`,
//! `thumb.png`), which is derived from the ROM and never shared.
//!
//! `list` and `render` need no ROM and no emulator; `make` and `verify` run
//! the game from power-on with a fresh EEPROM. Nothing here changes
//! emulation: a scene's machine is its own.

use crate::answer::{Answer, Code};
use model2::capture::{self, Frame};
use model2::video::{self, View};
use model2::{input::InputScript, raster};
use romset::{Game, RomSet};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

const USAGE: &str = "\
remodel [--game G] scene SUB [NAME] [options]   (docs/uplift-studio.md, \"Scenes\")
  list [--tag T]                      the game's scenes and whether each has a local copy
  make NAME | --all [--replace]       run a recipe from power-on; keep its capture, state, hash (needs the ROM)
  verify [NAME]                       run recipes again; a different state hash marks the scene stale (needs the ROM)
  render NAME [--pack DIR] [--crop X,Y,W,H] [--against original] [--view native|H[,A:B]] [--out DIR]
                                      draw a scene from its capture: no boot, no ROM, no emulator
common: --json (one object: ok, code, error, data, warnings, next), --project DIR (reads DIR/scenes.txt)
exit codes: 0 done, 1 a check failed, 2 bad request, 3 something missing, 4 internal error
";

/// Tags a scene may carry (docs/uplift/interface.md).
pub const TAGS: &[&str] = &["attract", "menu", "select", "play", "result", "hud"];

// ------------------------------------------------------------------ recipes

#[derive(Clone, Debug, PartialEq)]
pub struct Recipe {
    pub name: String,
    pub title: String,
    pub input: String,
    pub frame: u64,
    pub tags: Vec<String>,
    /// Where it came from: the manifest, or a project's `scenes.txt`.
    pub source: &'static str,
    /// A project's wishes, kept for pack checks (not used here).
    pub required: bool,
    pub coverage: Option<f64>,
}

fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn check_recipe(game: &Game, r: &Recipe) -> Result<(), String> {
    if !valid_name(&r.name) {
        return Err(format!("scene name `{}`: lower-case letters, digits and `-` only", r.name));
    }
    for t in &r.tags {
        if !TAGS.contains(&t.as_str()) {
            let known: Vec<String> = TAGS.iter().map(|s| s.to_string()).collect();
            return Err(Answer::unknown("tag", t, &known).message());
        }
    }
    InputScript::parse(&r.input, &game.controls).map_err(|e| format!("scene `{}`: input: {e}", r.name))?;
    Ok(())
}

/// The game's manifest scenes.
pub fn manifest_recipes(game: &Game) -> Vec<Recipe> {
    game.scenes
        .iter()
        .map(|s| Recipe {
            name: s.name.into(),
            title: s.title.into(),
            input: s.input.into(),
            frame: s.frame,
            tags: s.tags.iter().map(|t| t.to_string()).collect(),
            source: "manifest",
            required: false,
            coverage: None,
        })
        .collect()
}

const SCENE_KEYS: &[&str] = &["title", "input", "frame", "tags", "clip", "required", "coverage"];

/// Parse a project's `scenes.txt`:
///
/// ```text
/// [scene course-start]
/// title = Novice course, first stretch
/// input = coin-start
/// frame = 3600
/// tags  = play hud
/// required = yes
/// coverage = 0.95
/// ```
///
/// Strict: an unknown key or tag is an error that lists near matches.
pub fn parse_scenes_txt(text: &str, game: &Game) -> Result<Vec<Recipe>, String> {
    let mut out: Vec<Recipe> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let at = |m: String| format!("scenes.txt line {}: {m}", i + 1);
        if let Some(h) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let name = h.strip_prefix("scene").map(str::trim).filter(|n| !n.is_empty());
            let Some(name) = name else { return Err(at(format!("expected `[scene NAME]`, got `{line}`"))) };
            out.push(Recipe {
                name: name.into(),
                title: name.into(),
                input: String::new(),
                frame: 0,
                tags: vec![],
                source: "project",
                required: false,
                coverage: None,
            });
            continue;
        }
        let Some(cur) = out.last_mut() else { return Err(at("a setting before any `[scene NAME]`".into())) };
        let Some((k, v)) = line.split_once('=') else {
            return Err(at(format!("expected `key = value`, got `{line}`")));
        };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "title" => cur.title = v.into(),
            "input" => cur.input = v.into(),
            "frame" => cur.frame = v.parse().map_err(|_| at(format!("frame wants a whole number, got `{v}`")))?,
            "tags" => cur.tags = v.split_whitespace().map(str::to_string).collect(),
            "required" => cur.required = matches!(v, "yes" | "true" | "1"),
            "coverage" => {
                cur.coverage = Some(v.parse().map_err(|_| at(format!("coverage wants a number, got `{v}`")))?)
            }
            "clip" => {}
            _ => {
                let known: Vec<String> = SCENE_KEYS.iter().map(|s| s.to_string()).collect();
                return Err(at(Answer::unknown("setting", k, &known).message()));
            }
        }
    }
    let mut seen = HashSet::new();
    for r in &out {
        if !seen.insert(r.name.clone()) {
            return Err(format!("scenes.txt: scene `{}` is defined twice", r.name));
        }
        if r.frame == 0 {
            return Err(format!("scenes.txt: scene `{}` needs `frame = N`", r.name));
        }
        check_recipe(game, r)?;
    }
    Ok(out)
}

/// All the game's recipes: the manifest's, then the project's, which win on
/// a name. `project` is a directory with a `scenes.txt` (`None`: the
/// working directory's, if it has one).
pub fn recipes(game: &Game, project: Option<&Path>) -> Result<Vec<Recipe>, String> {
    let mut all = manifest_recipes(game);
    for r in &all {
        check_recipe(game, r)?;
    }
    let file = match project {
        Some(d) => Some(d.join("scenes.txt")),
        None => Some(PathBuf::from("scenes.txt")).filter(|p| p.is_file()),
    };
    if let Some(f) = file {
        if f.is_file() {
            let text = std::fs::read_to_string(&f).map_err(|e| format!("{}: {e}", f.display()))?;
            for r in parse_scenes_txt(&text, game).map_err(|e| format!("{}: {e}", f.display()))? {
                all.retain(|x| x.name != r.name);
                all.push(r);
            }
        }
    }
    Ok(all)
}

// ------------------------------------------------------------ local copies

/// Where scene copies live: `REMODEL_DATA`, else `out/` in a source tree,
/// else the user's data folder (as the player's saves).
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("REMODEL_DATA") {
        return PathBuf::from(d);
    }
    if Path::new("out").is_dir() {
        return PathBuf::from("out");
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("remodel")
}

pub fn scenes_dir(game: &Game) -> PathBuf {
    data_dir().join(game.id).join("scenes")
}

/// `scene.txt`: `key = value` lines.
fn read_meta(dir: &Path) -> Option<BTreeMap<String, String>> {
    let text = std::fs::read_to_string(dir.join("scene.txt")).ok()?;
    Some(
        text.lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect(),
    )
}

fn write_meta(dir: &Path, m: &BTreeMap<String, String>) -> std::io::Result<()> {
    let mut t = String::from(
        "# A scene's local copy (derived from the ROM: never share it). `remodel scene verify` checks it.\n",
    );
    for (k, v) in m {
        t += &format!("{k} = {v}\n");
    }
    std::fs::write(dir.join("scene.txt"), t)
}

/// A scene's state in the library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// No local copy.
    Missing,
    Ready,
    /// A local copy that no longer matches (why).
    Stale(String),
    /// A local copy whose recipe is gone: it works until a state-format
    /// change, and cannot be made again.
    Unreproducible,
}

impl State {
    fn name(&self) -> &'static str {
        match self {
            State::Missing => "missing",
            State::Ready => "ready",
            State::Stale(_) => "stale",
            State::Unreproducible => "unreproducible",
        }
    }
}

fn has_capture(dir: &Path) -> bool {
    capture_frame_dir(&dir.join("capture")).is_some() && dir.join("capture/texture-rom.bin").is_file()
}

/// The one frame directory in a scene's capture.
fn capture_frame_dir(root: &Path) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> =
        std::fs::read_dir(root).ok()?.flatten().map(|e| e.path()).filter(|p| p.join("meta.txt").is_file()).collect();
    dirs.sort();
    dirs.into_iter().next()
}

fn state_of(dir: &Path, recipe: Option<&Recipe>) -> State {
    let Some(meta) = read_meta(dir).filter(|_| has_capture(dir)) else { return State::Missing };
    let Some(r) = recipe else { return State::Unreproducible };
    if let Some(why) = meta.get("stale") {
        return State::Stale(why.clone());
    }
    if meta.get("input") != Some(&r.input) || meta.get("frame") != Some(&r.frame.to_string()) {
        return State::Stale("the recipe changed since the copy was made".into());
    }
    State::Ready
}

// ------------------------------------------------------------------ args

#[derive(Default)]
struct Args {
    pos: Vec<String>,
    vals: HashMap<String, Vec<String>>,
    sw: HashSet<String>,
}

impl Args {
    fn parse(a: &[String], switches: &[&str], valued: &[&str]) -> Result<Args, Answer> {
        let mut r = Args::default();
        let mut it = a.iter();
        while let Some(x) = it.next() {
            if switches.contains(&x.as_str()) {
                r.sw.insert(x.clone());
            } else if valued.contains(&x.as_str()) {
                let Some(v) = it.next() else {
                    return Err(Answer::fail(Code::BadRequest, format!("{x} wants a value")));
                };
                r.vals.entry(x.clone()).or_default().push(v.clone());
            } else if x.starts_with("--") {
                let known: Vec<String> = switches.iter().chain(valued).map(|s| s.to_string()).collect();
                return Err(Answer::unknown("option", x, &known));
            } else {
                r.pos.push(x.clone());
            }
        }
        Ok(r)
    }

    fn has(&self, f: &str) -> bool {
        self.sw.contains(f)
    }

    fn one(&self, f: &str) -> Option<&str> {
        self.vals.get(f).and_then(|v| v.last()).map(String::as_str)
    }
}

// ------------------------------------------------------------------ main

pub fn main(o: &crate::Opts) -> ExitCode {
    let rest = &o.args[1..];
    let json = rest.iter().any(|a| a == "--json");
    let Some(sub) = rest.iter().find(|a| !a.starts_with("--")).cloned() else {
        // No subcommand: the usage is the answer.
        let a = Answer::fail(Code::BadRequest, "scene wants a subcommand: list, make, verify or render");
        return a.emit(json, || USAGE.to_string());
    };
    let argv: Vec<String> = {
        let mut skipped = false;
        rest.iter()
            .filter(|a| {
                if !skipped && **a == sub {
                    skipped = true;
                    return false;
                }
                true
            })
            .cloned()
            .collect()
    };
    let a = match run_sub(o, &sub, &argv) {
        Ok((a, text)) => {
            return a.emit(json, || text);
        }
        Err(a) => a,
    };
    a.emit(json, String::new)
}

fn run_sub(o: &crate::Opts, sub: &str, argv: &[String]) -> Result<(Answer, String), Answer> {
    let game = romset::games::find(&o.game_id).map_err(|e| Answer::fail(Code::BadRequest, e.to_string()))?;
    let project_flag = ["--project"];
    match sub {
        "list" => {
            let a = Args::parse(argv, &["--json"], &["--tag", "--project"])?;
            list(game, &a)
        }
        "make" => {
            let a = Args::parse(argv, &["--json", "--all", "--replace"], &project_flag)?;
            make(o, game, &a)
        }
        "verify" => {
            let a = Args::parse(argv, &["--json"], &project_flag)?;
            verify(o, game, &a)
        }
        "render" => {
            let a = Args::parse(argv, &["--json"], &["--pack", "--crop", "--against", "--view", "--project"])?;
            render_cmd(o, game, &a)
        }
        _ => {
            let known: Vec<String> = ["list", "make", "verify", "render"].iter().map(|s| s.to_string()).collect();
            Err(Answer::unknown("scene subcommand", sub, &known).next("remodel scene --help"))
        }
    }
}

fn load_recipes(game: &Game, a: &Args) -> Result<Vec<Recipe>, Answer> {
    recipes(game, a.one("--project").map(Path::new)).map_err(|e| Answer::fail(Code::BadRequest, e))
}

fn find_recipe<'a>(rs: &'a [Recipe], name: &str) -> Result<&'a Recipe, Answer> {
    rs.iter().find(|r| r.name == name).ok_or_else(|| {
        let known: Vec<String> = rs.iter().map(|r| r.name.clone()).collect();
        Answer::unknown("scene", name, &known).next("remodel scene list")
    })
}

/// The names asked for: one positional NAME, or `--all`.
fn wanted<'a>(rs: &'a [Recipe], a: &Args, what: &str) -> Result<Vec<&'a Recipe>, Answer> {
    match (a.pos.as_slice(), a.has("--all")) {
        ([], false) => Err(Answer::fail(Code::BadRequest, format!("scene {what}: which scene? NAME or --all"))
            .next("remodel scene list")),
        ([n], false) => Ok(vec![find_recipe(rs, n)?]),
        ([], true) => Ok(rs.iter().collect()),
        _ => Err(Answer::fail(Code::BadRequest, format!("scene {what}: give one NAME, or --all, not both"))),
    }
}

// ------------------------------------------------------------------ list

fn list(game: &Game, a: &Args) -> Result<(Answer, String), Answer> {
    let rs = load_recipes(game, a)?;
    let tag = a.one("--tag");
    if let Some(t) = tag {
        if !TAGS.contains(&t) {
            let known: Vec<String> = TAGS.iter().map(|s| s.to_string()).collect();
            return Err(Answer::unknown("tag", t, &known));
        }
    }
    let root = scenes_dir(game);
    let mut rows = Vec::new();
    let mut text = String::from("name\tframe\tstate\ttags\ttitle\n");
    for r in rs.iter().filter(|r| tag.is_none_or(|t| r.tags.iter().any(|x| x == t))) {
        let st = state_of(&root.join(&r.name), Some(r));
        rows.push(json!({
            "name": r.name, "title": r.title, "tags": r.tags, "frame": r.frame, "input": r.input,
            "source": r.source, "state": st.name(),
            "local": st != State::Missing, "stale": matches!(st, State::Stale(_)),
            "why": if let State::Stale(w) = &st { json!(w) } else { Value::Null },
            "tour": Value::Null,
        }));
        text += &format!("{}\t{}\t{}\t{}\t{}\n", r.name, r.frame, st.name(), r.tags.join(","), r.title);
    }
    // Local copies without a recipe.
    let known: HashSet<&str> = rs.iter().map(|r| r.name.as_str()).collect();
    for e in std::fs::read_dir(&root).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if !known.contains(n.as_str()) && state_of(&e.path(), None) == State::Unreproducible && tag.is_none() {
            rows.push(json!({"name": n, "title": n, "tags": [], "frame": Value::Null, "source": "local",
                "state": "unreproducible", "local": true, "stale": false, "tour": Value::Null}));
            text += &format!("{n}\t\tunreproducible\t\t\n");
        }
    }
    let missing = rows.iter().filter(|r| r["state"] == "missing").count();
    let mut ans = Answer::done(json!({"scenes": rows, "tags": TAGS, "dir": root.display().to_string()}));
    if missing > 0 && !rs.is_empty() {
        ans = ans.next("remodel scene make --all   (needs the ROM)");
    }
    Ok((ans, text))
}

// ------------------------------------------------------------------ make

struct Reached {
    machine: model2::Machine,
    hash: u64,
    script: InputScript,
}

/// Run the recipe from power-on, with a fresh EEPROM and the machine of the
/// command line's geometrizer mode, to the start of its frame; the hash is
/// taken as `--save-state` saves it, before that frame's inputs.
fn run_to(roms: &RomSet, r: &Recipe) -> Result<Reached, String> {
    let script = InputScript::parse(&r.input, &roms.game.controls)?;
    let mut m = crate::machine(roms);
    let mut last = 0;
    while m.frames < r.frame {
        if let Err(s) = m.run_slice(u64::MAX / 2) {
            return Err(format!("the machine stopped at frame {} ({s:?}) before frame {}", m.frames, r.frame));
        }
        if m.frames != last {
            if m.frames == r.frame {
                break;
            }
            script.apply(m.frames, &mut m.bus);
            last = m.frames;
        }
    }
    let hash = m.bus.state_hash();
    Ok(Reached { machine: m, hash, script })
}

fn load_roms(o: &crate::Opts, game: &'static Game) -> Result<RomSet, Answer> {
    RomSet::load(game, &o.rom_dir, false).map_err(|e| {
        Answer::fail(Code::Missing, format!("no ROMs for {}: {e}", game.id)).next(format!(
            "put the {} ROMs in {} (remodel --game {} verify)",
            game.id,
            o.rom_dir.display(),
            game.id
        ))
    })
}

fn geo_name() -> &'static str {
    match crate::GEO_MODE.get().copied().unwrap_or_default() {
        model2::geo::GeoMode::Chip => "chip",
        _ => "native",
    }
}

fn make_one(roms: &RomSet, r: &Recipe, dir: &Path) -> Result<Value, String> {
    let started = Instant::now();
    let mut reached = run_to(roms, r)?;
    let m = &mut reached.machine;
    let tmp = dir.with_extension("tmp");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let state = model2::save_session(m, None);
    std::fs::write(tmp.join("state.save"), &state).map_err(|e| e.to_string())?;
    // The frame as a snapshot at this frame takes it: after its inputs.
    reached.script.apply(m.frames, &mut m.bus);
    let frame = Frame::grab(&mut m.bus);
    frame.save(&tmp.join("capture"), &m.bus.texture_rom).map_err(|e| format!("capture: {e}"))?;
    crate::draw_png(m, None, &tmp.join("thumb.png"));
    let mut meta = BTreeMap::new();
    meta.insert("name".to_string(), r.name.clone());
    meta.insert("game".into(), roms.game.id.into());
    meta.insert("input".into(), r.input.clone());
    meta.insert("frame".into(), r.frame.to_string());
    meta.insert("eeprom".into(), "fresh".into());
    meta.insert("geometrizer".into(), geo_name().into());
    meta.insert("remodel".into(), remodel_version::VERSION.into());
    meta.insert("state_hash".into(), format!("{:016x}", reached.hash));
    write_meta(&tmp, &meta).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(dir);
    std::fs::rename(&tmp, dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(json!({"name": r.name, "frame": r.frame, "state_hash": format!("{:016x}", reached.hash),
        "dir": dir.display().to_string(), "state_bytes": state.len(), "seconds": round2(started.elapsed().as_secs_f64())}))
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn make(o: &crate::Opts, game: &'static Game, a: &Args) -> Result<(Answer, String), Answer> {
    let rs = load_recipes(game, a)?;
    let want = wanted(&rs, a, "make")?;
    let root = scenes_dir(game);
    let replace = a.has("--replace");
    // Nothing is overwritten without --replace.
    let mut todo = Vec::new();
    let mut skipped = Vec::new();
    for r in want {
        let d = root.join(&r.name);
        if d.exists() && !replace {
            if a.has("--all") {
                skipped.push(r.name.clone());
                continue;
            }
            return Err(Answer::fail(Code::BadRequest, format!("scene `{}` already has a local copy", r.name))
                .next(format!("remodel scene make {} --replace", r.name)));
        }
        todo.push(r);
    }
    if todo.is_empty() {
        let ans = Answer::done(json!({"made": [], "failed": [], "skipped": skipped}));
        return Ok((ans, "nothing to make (every scene has a local copy; --replace makes them again)\n".into()));
    }
    let roms = load_roms(o, game)?;
    std::fs::create_dir_all(&root).map_err(|e| Answer::fail(Code::Internal, format!("{}: {e}", root.display())))?;
    let (mut made, mut failed) = (Vec::new(), Vec::new());
    let mut text = String::new();
    for r in todo {
        match make_one(&roms, r, &root.join(&r.name)) {
            Ok(v) => {
                text += &format!(
                    "made          {} (frame {}, state hash {}, {} s)\n",
                    r.name,
                    r.frame,
                    v["state_hash"].as_str().unwrap_or(""),
                    v["seconds"]
                );
                made.push(v);
            }
            Err(e) => {
                text += &format!("failed        {}: {e}\n", r.name);
                failed.push(json!({"name": r.name, "error": e}));
            }
        }
    }
    let data = json!({"made": made, "failed": failed, "skipped": skipped});
    let ans = if failed.is_empty() {
        Answer::done(data).next(format!("remodel scene render {} --json", made[0]["name"].as_str().unwrap_or("NAME")))
    } else {
        Answer::fail(Code::Internal, format!("{} scene(s) could not be made", failed.len())).with_data(data)
    };
    Ok((ans, text))
}

// ---------------------------------------------------------------- verify

fn verify(o: &crate::Opts, game: &'static Game, a: &Args) -> Result<(Answer, String), Answer> {
    let rs = load_recipes(game, a)?;
    let explicit = !a.pos.is_empty();
    let want: Vec<&Recipe> = match a.pos.as_slice() {
        [] => rs.iter().collect(),
        [n] => vec![find_recipe(&rs, n)?],
        _ => return Err(Answer::fail(Code::BadRequest, "scene verify: one NAME, or none for every scene")),
    };
    let root = scenes_dir(game);
    let mut todo = Vec::new();
    let mut rows = Vec::new();
    let mut text = String::new();
    for r in want {
        let d = root.join(&r.name);
        if state_of(&d, Some(r)) == State::Missing {
            if explicit {
                return Err(Answer::fail(Code::Missing, format!("scene `{}` has no local copy", r.name))
                    .next(format!("remodel scene make {}   (needs the ROM)", r.name)));
            }
            rows.push(json!({"name": r.name, "ok": Value::Null, "why": "no local copy"}));
            continue;
        }
        todo.push(r);
    }
    if !todo.is_empty() {
        let roms = load_roms(o, game)?;
        for r in todo {
            let d = root.join(&r.name);
            let mut meta = read_meta(&d).unwrap_or_default();
            let recorded = meta.get("state_hash").cloned().unwrap_or_default();
            let why = if meta.get("input") != Some(&r.input) || meta.get("frame") != Some(&r.frame.to_string()) {
                Some("the recipe changed since the copy was made".to_string())
            } else {
                match run_to(&roms, r) {
                    Err(e) => Some(e),
                    Ok(got) => {
                        let h = format!("{:016x}", got.hash);
                        (h != recorded).then(|| format!("state hash {h}, recorded {recorded}: the emulation changed"))
                    }
                }
            };
            match &why {
                None => {
                    meta.remove("stale");
                    meta.insert("verified".into(), remodel_version::VERSION.into());
                    text += &format!("ok            {} (frame {}, state hash {recorded})\n", r.name, r.frame);
                }
                Some(w) => {
                    meta.insert("stale".into(), w.clone());
                    text += &format!("STALE         {}: {w}\n", r.name);
                }
            }
            let _ = write_meta(&d, &meta);
            rows.push(json!({"name": r.name, "ok": why.is_none(), "why": why}));
        }
    }
    let stale: Vec<&Value> = rows.iter().filter(|r| r["ok"] == false).collect();
    let data = json!({"scenes": rows});
    let ans = if stale.is_empty() {
        Answer::done(data)
    } else {
        let names: Vec<&str> = stale.iter().filter_map(|r| r["name"].as_str()).collect();
        let next = match names[..] {
            [one] => format!("remodel scene make {one} --replace   (needs the ROM)"),
            _ => "remodel scene make --all --replace   (needs the ROM)".into(),
        };
        Answer::gate(format!("{} scene(s) are stale", stale.len()), data).next(next)
    };
    Ok((ans, text))
}

// ---------------------------------------------------------------- render

/// What to draw.
#[derive(Clone)]
pub struct DrawOpts {
    pub pack: Option<PathBuf>,
    /// Game pixels (496x384), whatever the view.
    pub crop: Option<[i64; 4]>,
    /// Also draw the original (no pack, no look).
    pub against: bool,
    pub view: View,
}

pub struct Picture {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}

pub struct Drawn {
    pub picture: Picture,
    pub original: Option<Picture>,
    /// Share of pixels that differ from the original (0 to 1).
    pub changed_share: Option<f64>,
    pub polygons: usize,
    pub warnings: Vec<String>,
    pub millis: f64,
}

fn crop_picture(p: Picture, view: &View, c: [i64; 4]) -> Result<Picture, String> {
    let (x0, y0) = view.to_output(c[0] as f32, c[1] as f32);
    let (x1, y1) = view.to_output((c[0] + c[2]) as f32, (c[1] + c[3]) as f32);
    let (x0, y0, x1, y1) = (x0.round() as i64, y0.round() as i64, x1.round() as i64, y1.round() as i64);
    if x0 < 0 || y0 < 0 || x1 > p.width as i64 || y1 > p.height as i64 || x1 <= x0 || y1 <= y0 {
        return Err(format!(
            "--crop {},{},{},{} falls outside the {}x{} view (game pixels: x {:.0} to {:.0}, y 0 to 384)",
            c[0],
            c[1],
            c[2],
            c[3],
            p.width,
            p.height,
            view.x_min(),
            view.x_min() + p.width as f32 / view.sx
        ));
    }
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let mut rgb = Vec::with_capacity(w * h * 3);
    for y in y0 as usize..y1 as usize {
        rgb.extend_from_slice(&p.rgb[(y * p.width + x0 as usize) * 3..(y * p.width + x1 as usize) * 3]);
    }
    Ok(Picture { width: w, height: h, rgb })
}

/// Draw the capture in `scene_dir` on `gpu`. Nothing here touches a machine.
pub fn draw(game: &Game, scene_dir: &Path, o: &DrawOpts, gpu: &mut gpu::Renderer) -> Result<Drawn, Answer> {
    let started = Instant::now();
    let cap = scene_dir.join("capture");
    let missing = |m: String| {
        Answer::fail(Code::Missing, m).next(format!("remodel scene make {}   (needs the ROM)", scene_name(scene_dir)))
    };
    let frame_dir =
        capture_frame_dir(&cap).ok_or_else(|| missing(format!("scene `{}` has no capture", scene_name(scene_dir))))?;
    let rom = capture::load_texture_rom(&cap).map_err(|e| missing(format!("{}: {e}", cap.display())))?;
    let f =
        Frame::load(&frame_dir).map_err(|e| Answer::fail(Code::Internal, format!("{}: {e}", frame_dir.display())))?;
    let view = o.view.with_hud_windows(game.hud_windows, game.hud_element_windows);
    let src = f.sources(&rom);
    let side = f.light_frame();
    let light = crate::light_options(game);
    let mut polys = raster::build_shaded_with(&f.stream, &src, side.as_ref(), light.as_ref());
    video::HudLayout::new(&f.tiles, &f.palette, &view).anchor(&mut polys, &view);
    let placed = raster::place(&mut polys, &view);
    let layers = video::layers_2d(&f.tiles, &f.palette, &view, !placed.is_empty());
    gpu.set_quality(gpu::Quality {
        msaa: crate::AA.get().copied().unwrap_or(1),
        depth: crate::DEPTH.get().copied().unwrap_or(false),
        tex_filter: crate::TEX_FILTER.get().copied().unwrap_or_default(),
        ..crate::gpu_light(light.as_ref())
    });
    let mut warnings = Vec::new();
    let (w, h) = (view.width, view.height);
    let mut original = None;
    if o.against {
        gpu.set_pack(None);
        gpu.set_look(None);
        original = Some(Picture { width: w, height: h, rgb: gpu.frame(&polys, &placed, &src, &view, &layers) });
    }
    let mut packed = false;
    if let Some(dir) = &o.pack {
        match crate::texpack::accept(dir) {
            Ok(_) => match crate::texpack::Packs::new(None, Some(dir.clone())) {
                Ok(p) => {
                    gpu.set_pack(Some(Box::new(p)));
                    packed = true;
                }
                Err(e) => return Err(Answer::fail(Code::Missing, e)),
            },
            Err(e) => warnings.push(format!("pack {} not drawn: {e}", dir.display())),
        }
    } else {
        gpu.set_pack(None);
    }
    gpu.set_look(crate::LOOK.get().map(|l| std::sync::Arc::new(l.clone())));
    let rgb = gpu.frame(&polys, &placed, &src, &view, &layers);
    let changed_share = original.as_ref().map(|orig| {
        let n = rgb.chunks(3).zip(orig.rgb.chunks(3)).filter(|(a, b)| a != b).count();
        n as f64 / (w * h) as f64
    });
    if o.against && !packed && crate::LOOK.get().is_none() {
        warnings.push(
            "nothing to compare: no --pack (or it was refused) and no --look, so both pictures are the original".into(),
        );
    }
    let mut picture = Picture { width: w, height: h, rgb };
    if let Some(c) = o.crop {
        let bad = |e| Answer::fail(Code::BadRequest, e);
        picture = crop_picture(picture, &view, c).map_err(bad)?;
        original = original.map(|p| crop_picture(p, &view, c)).transpose().map_err(bad)?;
    }
    Ok(Drawn {
        picture,
        original,
        changed_share,
        polygons: polys.len(),
        warnings,
        millis: started.elapsed().as_secs_f64() * 1e3,
    })
}

fn scene_name(dir: &Path) -> String {
    dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Two pictures side by side (the shorter padded with black).
pub fn side_by_side(a: &Picture, b: &Picture) -> Picture {
    let (w, h) = (a.width + b.width, a.height.max(b.height));
    let mut rgb = vec![0u8; w * h * 3];
    for (p, x0) in [(a, 0), (b, a.width)] {
        for y in 0..p.height {
            rgb[(y * w + x0) * 3..(y * w + x0 + p.width) * 3]
                .copy_from_slice(&p.rgb[y * p.width * 3..(y + 1) * p.width * 3]);
        }
    }
    Picture { width: w, height: h, rgb }
}

pub fn save_png(path: &Path, p: &Picture) -> Result<(), String> {
    std::fs::write(path, png::encode_rgb(p.width, p.height, &p.rgb)).map_err(|e| format!("{}: {e}", path.display()))
}

fn parse_crop(s: &str) -> Result<[i64; 4], Answer> {
    let bad = || Answer::fail(Code::BadRequest, format!("--crop wants X,Y,W,H in game pixels (496x384), got `{s}`"));
    let v: Vec<i64> = s.split(',').map(|p| p.trim().parse::<i64>()).collect::<Result<_, _>>().map_err(|_| bad())?;
    match v[..] {
        [x, y, w, h] if w > 0 && h > 0 => Ok([x, y, w, h]),
        _ => Err(bad()),
    }
}

/// `--view native`, `--view HEIGHT` or `--view HEIGHT,A:B` (default: the
/// command line's `--height/--aspect/--hud`, else 1080p 16:9 with the HUD
/// at the edges).
pub fn parse_view(s: Option<&str>) -> Result<View, Answer> {
    let bad = || Answer::fail(Code::BadRequest, "--view wants native, HEIGHT or HEIGHT,A:B (for example 1080,16:9)");
    let Some(s) = s else {
        return Ok(crate::VIEW.get().copied().unwrap_or_else(|| View::new(1080, 16.0 / 9.0, video::Hud::Edges)));
    };
    if s == "native" {
        return Ok(View::native());
    }
    let (h, asp) = s.split_once(',').map_or((s, "16:9"), |(h, a)| (h, a));
    let height: usize = h.parse().ok().filter(|&h| (96..=4320).contains(&h)).ok_or_else(bad)?;
    let (aw, ah) = asp.split_once(':').unwrap_or((asp, "1"));
    let aspect = aw.parse::<f32>().map_err(|_| bad())? / ah.parse::<f32>().map_err(|_| bad())?;
    if !(0.5..=4.0).contains(&aspect) {
        return Err(bad());
    }
    Ok(View::new(height, aspect, video::Hud::Edges))
}

fn render_cmd(o: &crate::Opts, game: &'static Game, a: &Args) -> Result<(Answer, String), Answer> {
    let started = Instant::now();
    let rs = load_recipes(game, a)?;
    let [name] = a.pos.as_slice() else {
        return Err(Answer::fail(Code::BadRequest, "scene render: which scene? NAME").next("remodel scene list"));
    };
    let recipe = find_recipe(&rs, name)?;
    let against = match a.one("--against") {
        None => false,
        Some("original") => true,
        Some(other) => {
            return Err(Answer::unknown("--against target", other, &["original".to_string()]));
        }
    };
    let crop = a.one("--crop").map(parse_crop).transpose()?;
    let view = parse_view(a.one("--view"))?;
    let mut packs: Vec<PathBuf> =
        a.vals.get("--pack").map(|v| v.iter().map(PathBuf::from).collect()).unwrap_or_default();
    if packs.is_empty() {
        packs.extend(o.pack_dir.clone());
    }
    if packs.len() > 1 {
        return Err(Answer::fail(Code::BadRequest, "layered packs (several --pack) are not built yet: give one"));
    }
    let dir = scenes_dir(game).join(&recipe.name);
    if state_of(&dir, Some(recipe)) == State::Missing {
        return Err(Answer::fail(Code::Missing, format!("scene `{}` has no local copy", recipe.name))
            .next(format!("remodel scene make {}   (needs the ROM)", recipe.name)));
    }
    // `--out DIR` is the CLI global option that says where `run` writes.
    let out = crate::OUT_DIR.get().cloned().unwrap_or_else(|| data_dir().join(game.id).join("scene-render"));
    std::fs::create_dir_all(&out).map_err(|e| Answer::fail(Code::Internal, format!("{}: {e}", out.display())))?;
    let t_gpu = Instant::now();
    let mut gpu = gpu::Renderer::new().map_err(|e| Answer::fail(Code::Internal, format!("no GPU: {e}")))?;
    let gpu_ms = t_gpu.elapsed().as_secs_f64() * 1e3;
    let opts = DrawOpts { pack: packs.first().cloned(), crop, against, view };
    let d = draw(game, &dir, &opts, &mut gpu)?;
    // Render output is regenerated every time (the same files for the same
    // inputs), so it is replaced in place; drafts and approved images are
    // never written here.
    let stem = &recipe.name;
    let t_write = Instant::now();
    let mut files = BTreeMap::new();
    let mut jobs: Vec<(PathBuf, Picture)> = Vec::new();
    let mut put = |k: &str, suffix: &str, p: Picture| {
        let path = out.join(format!("{stem}{suffix}.png"));
        files.insert(k.to_string(), path.display().to_string());
        jobs.push((path, p));
    };
    let sheet = d.original.as_ref().map(|orig| side_by_side(orig, &d.picture));
    if let (Some(orig), Some(sheet)) = (d.original.as_ref(), sheet) {
        put("original", "-original", Picture { width: orig.width, height: orig.height, rgb: orig.rgb.clone() });
        put(
            "picture",
            "-pack",
            Picture { width: d.picture.width, height: d.picture.height, rgb: d.picture.rgb.clone() },
        );
        put("sheet", "-sheet", sheet);
    } else {
        put("picture", "", Picture { width: d.picture.width, height: d.picture.height, rgb: d.picture.rgb.clone() });
    }
    // The PNGs are encoded side by side: at 1080p each costs about 0.2 s.
    let failed: Vec<String> = std::thread::scope(|sc| {
        let hs: Vec<_> = jobs.iter().map(|(path, p)| sc.spawn(move || save_png(path, p))).collect();
        hs.into_iter().filter_map(|h| h.join().unwrap_or_else(|_| Err("png thread panicked".into())).err()).collect()
    });
    if let Some(e) = failed.into_iter().next() {
        return Err(Answer::fail(Code::Internal, e));
    }
    let write_ms = t_write.elapsed().as_secs_f64() * 1e3;
    let ms = started.elapsed().as_secs_f64() * 1e3;
    let data = json!({
        "scene": recipe.name, "picture": files["picture"], "original": files.get("original"), "sheet": files.get("sheet"),
        "width": d.picture.width, "height": d.picture.height, "polygons": d.polygons,
        "changed_share": d.changed_share.map(|c| (c * 1e6).round() / 1e6),
        "pack": packs.first().map(|p| p.display().to_string()),
        "crop": crop, "ms": ms.round(), "ms_gpu_init": gpu_ms.round(), "ms_draw": d.millis.round(), "ms_write": write_ms.round(),
    });
    let mut text = format!(
        "scene         {} (frame {}), {}x{}{}\n",
        recipe.name,
        recipe.frame,
        d.picture.width,
        d.picture.height,
        if crop.is_some() { " crop" } else { "" }
    );
    for (k, v) in &files {
        text += &format!("{k:<13} {v}\n");
    }
    if let Some(c) = d.changed_share {
        text += &format!("changed       {:.2}% of pixels against the original\n", c * 100.0);
    }
    text += &format!("time          {:.0} ms (GPU start {gpu_ms:.0}, draw {:.0}, write {write_ms:.0})\n", ms, d.millis);
    let mut ans = Answer::done(data);
    if let State::Stale(why) = state_of(&dir, Some(recipe)) {
        ans = ans.warn(format!("scene `{}` is stale ({why}); the picture is the copy as made", recipe.name));
    }
    for w in d.warnings {
        ans = ans.warn(w);
    }
    Ok((ans, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game() -> &'static Game {
        romset::games::find("topskatr").unwrap()
    }

    #[test]
    fn scenes_txt_parses_and_is_strict() {
        let t = "[scene a-b]\ntitle = A B\ninput = coin-start\nframe = 100\ntags = play hud\nrequired = yes\ncoverage = 0.9\n";
        let r = parse_scenes_txt(t, game()).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].name.as_str(), r[0].frame, r[0].required), ("a-b", 100, true));
        // An unknown key lists near matches; so does an unknown tag.
        let e = parse_scenes_txt("[scene a]\ninput = coin-start\nframes = 3\n", game()).unwrap_err();
        assert!(e.contains("did you mean: frame"), "{e}");
        let e = parse_scenes_txt("[scene a]\ninput = coin-start\nframe = 3\ntags = plya\n", game()).unwrap_err();
        assert!(e.contains("play"), "{e}");
        // A bad input script and a missing frame are refused.
        assert!(parse_scenes_txt("[scene a]\ninput = nonsense@1-2\nframe = 3\n", game()).is_err());
        assert!(parse_scenes_txt("[scene a]\ninput = coin-start\n", game()).is_err());
        assert!(parse_scenes_txt("[scene A]\ninput = coin-start\nframe = 3\n", game()).is_err());
        assert!(parse_scenes_txt("[scene a]\nframe = 3\n[scene a]\nframe = 4\n", game()).is_err());
    }

    #[test]
    fn project_scenes_win_over_the_manifest() {
        let dir = std::env::temp_dir().join(format!("remodel-scene-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = manifest_recipes(game()).first().cloned();
        if let Some(f) = first {
            std::fs::write(dir.join("scenes.txt"), format!("[scene {}]\ninput = coin-start\nframe = 77\n", f.name))
                .unwrap();
            let all = recipes(game(), Some(&dir)).unwrap();
            let got = all.iter().find(|r| r.name == f.name).unwrap();
            assert_eq!((got.frame, got.source), (77, "project"));
            assert_eq!(all.iter().filter(|r| r.name == f.name).count(), 1);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_scenes_are_valid_for_every_game() {
        for g in romset::games::ALL.iter().copied() {
            let rs = manifest_recipes(g);
            let mut names = HashSet::new();
            for r in &rs {
                check_recipe(g, r).unwrap_or_else(|e| panic!("{}: {e}", g.id));
                assert!(names.insert(r.name.clone()), "{}: duplicate scene {}", g.id, r.name);
                assert!(r.frame > 0);
            }
        }
    }

    #[test]
    fn crop_maps_game_pixels_through_the_view() {
        let v = View::new(768, 16.0 / 9.0, video::Hud::Edges);
        let p = Picture { width: v.width, height: v.height, rgb: vec![7; v.width * v.height * 3] };
        let c = crop_picture(p, &v, [0, 0, 248, 192]).unwrap();
        assert!((c.width as f32 - 248.0 * v.sx).abs() <= 1.0 && (c.height as f32 - 192.0 * v.sy).abs() <= 1.0);
        let p = Picture { width: v.width, height: v.height, rgb: vec![7; v.width * v.height * 3] };
        assert!(crop_picture(p, &v, [400, 300, 200, 200]).is_err());
    }

    #[test]
    fn unknown_scene_lists_near_matches() {
        let rs = vec![Recipe {
            name: "course-start".into(),
            title: String::new(),
            input: String::new(),
            frame: 1,
            tags: vec![],
            source: "manifest",
            required: false,
            coverage: None,
        }];
        let e = find_recipe(&rs, "course-star").unwrap_err();
        assert_eq!(e.code, Code::BadRequest);
        assert!(e.message().contains("course-start"));
    }
}
