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
  render NAME [--pack DIR] [--crop X,Y,W,H] [--against original] [--overlay miss] [--view native|H[,A:B]] [--out DIR]
         [--camera SPEC]              draw a scene from its capture: no boot, no ROM, no emulator;
                                      --camera \"orbit=30,tilt=-8,z=1\" sees it from another camera (photo mode's words)
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
pub(crate) fn read_meta(dir: &Path) -> Option<BTreeMap<String, String>> {
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
pub struct Args {
    pub pos: Vec<String>,
    pub vals: HashMap<String, Vec<String>>,
    pub sw: HashSet<String>,
}

impl Args {
    pub fn parse(a: &[String], switches: &[&str], valued: &[&str]) -> Result<Args, Answer> {
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

    pub fn has(&self, f: &str) -> bool {
        self.sw.contains(f)
    }

    pub fn one(&self, f: &str) -> Option<&str> {
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
            let a =
                Args::parse(argv, &["--json"], &["--pack", "--crop", "--against", "--view", "--project", "--camera"])?;
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

/// The frame's display list in a scene's folder (`Frame::to_bytes`).
pub const DISPLAY_FILE: &str = "display.bin";

/// How many frames before a scene's the display list is recorded from.
const DISPLAY_LEAD: u64 = 3;

pub(crate) struct Reached {
    pub(crate) machine: model2::Machine,
    pub(crate) hash: u64,
    script: InputScript,
}

/// Run the recipe from power-on, with a fresh EEPROM and the machine of the
/// command line's geometrizer mode, to the start of its frame; the hash is
/// taken as `--save-state` saves it, before that frame's inputs.
pub(crate) fn run_to(roms: &RomSet, r: &Recipe, display: bool) -> Result<Reached, String> {
    let script = InputScript::parse(&r.input, &roms.game.controls)?;
    let mut m = crate::machine(roms);
    let mut last = 0;
    while m.frames < r.frame {
        if let Err(s) = m.run_slice(u64::MAX / 2) {
            return Err(format!("the machine stopped at frame {} ({s:?}) before frame {}", m.frames, r.frame));
        }
        // The last frames on the native geometrizer with smooth motion's
        // recording, which keeps the display list of the frame the capture
        // takes (`display.bin`). Both compute or record what the chip's
        // frames give: the state hash is the same.
        if display && m.frames + DISPLAY_LEAD == r.frame {
            if crate::GEO_MODE.get().copied().unwrap_or_default() == model2::geo::GeoMode::Chip {
                m.bus.geo.set_mode(model2::geo::GeoMode::Native);
            }
            m.bus.geo.set_smooth(true);
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
    let mut reached = run_to(roms, r, true)?;
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
    // The frame's display list, to see it from another camera
    // (`scene render --camera`): only with the native geometrizer's
    // recording, which a game whose program it does not implement lacks.
    let display = m.bus.geo.engine().smooth_frame().map(|f| f.to_bytes());
    if let Some(bytes) = &display {
        std::fs::write(tmp.join(DISPLAY_FILE), bytes).map_err(|e| format!("{DISPLAY_FILE}: {e}"))?;
    }
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
    meta.insert("display".into(), if display.is_some() { "yes" } else { "none" }.into());
    write_meta(&tmp, &meta).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(dir);
    std::fs::rename(&tmp, dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(json!({"name": r.name, "frame": r.frame, "state_hash": format!("{:016x}", reached.hash),
        "dir": dir.display().to_string(), "state_bytes": state.len(), "display_bytes": display.as_ref().map(Vec::len),
        "seconds": round2(started.elapsed().as_secs_f64())}))
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
                match run_to(&roms, r, false) {
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
    /// Packs stacked, highest first (`model2::layered`); none: the original.
    pub pack: Vec<PathBuf>,
    /// Game pixels (496x384), whatever the view.
    pub crop: Option<[i64; 4]>,
    /// Also draw the original (no pack, no look).
    pub against: bool,
    pub view: View,
    /// The scene seen from another camera (photo mode's words,
    /// `model2::photo::Spec`), from its `display.bin`.
    pub camera: Option<String>,
    /// The look drawn with (a session picks its own; the scene commands
    /// pass the command line's, [`global_look`]).
    pub look: Option<std::sync::Arc<model2::look::Look>>,
}

/// The command line's `--look`, as a draw option.
pub fn global_look() -> Option<std::sync::Arc<model2::look::Look>> {
    crate::LOOK.get().map(|l| std::sync::Arc::new(l.clone()))
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
    /// Of the textured polygons' area (overlaps counted), the share drawn
    /// from a pack image: by identity, so a replacement that looks like the
    /// original counts. None without a pack.
    pub drawn_share: Option<f64>,
    /// With the `miss` overlay: the share of the picture's pixels in the
    /// overlay's colour (a pixel of the game's own that is exactly that
    /// magenta counts too). None without the overlay.
    pub miss_pixels: Option<f64>,
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

/// What a scene's display list gives for a camera.
pub struct Posed {
    pub stream: Vec<u32>,
    pub side: Option<model2::light::Frame>,
    pub near: f32,
    pub hud: bool,
    pub warnings: Vec<String>,
}

/// A frame's display list seen from the camera `spec` (`model2::photo::Spec`)
/// by the native geometrizer: no ROM, no machine.
pub fn pose_frame(frame: &model2::smooth::Frame, spec: &str, lit: bool) -> Result<Posed, Answer> {
    let scale = model2::photo::scene_scale(frame.plan.transforms());
    let spec = model2::photo::Spec::parse(spec, scale)
        .map_err(|e| Answer::fail(Code::BadRequest, format!("--camera: {e}")))?;
    let (stream, side) = model2::photo::render(frame, &spec.camera, 1.0, lit);
    Ok(Posed { stream, side, near: spec.near_plane(scale), hud: spec.hud, warnings: spec.warnings })
}

/// `display.bin`'s bytes, or the answer that says to make the scene again.
fn read_display(scene_dir: &Path) -> Result<Vec<u8>, Answer> {
    let name = scene_name(scene_dir);
    let again = |m: String| {
        Answer::fail(Code::Missing, m).next(format!("remodel scene make {name} --replace   (needs the ROM)"))
    };
    match std::fs::read(scene_dir.join(DISPLAY_FILE)) {
        Ok(b) => Ok(b),
        Err(_) if read_meta(scene_dir).and_then(|m| m.get("display").cloned()).as_deref() == Some("none") => {
            Err(again(format!(
                "scene `{name}` has no display list: its game's geometrizer program has no native implementation here, so there is no free camera"
            )))
        }
        Err(_) => Err(again(format!(
            "scene `{name}` has no {DISPLAY_FILE} (it was made before the free camera): run `remodel scene make {name} --replace` again"
        ))),
    }
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
    let display = match &o.camera {
        Some(_) => Some(display_frame(scene_dir)?),
        None => None,
    };
    let mut d = draw_frame(game, f, &rom, display.as_ref(), o, gpu)?;
    d.millis = started.elapsed().as_secs_f64() * 1e3;
    Ok(d)
}

/// The scene's display list, parsed.
fn display_frame(scene_dir: &Path) -> Result<model2::smooth::Frame, Answer> {
    let name = scene_name(scene_dir);
    let bytes = read_display(scene_dir)?;
    model2::smooth::Frame::from_bytes(&bytes).map_err(|e| {
        Answer::fail(Code::Missing, format!("scene `{name}`: {DISPLAY_FILE}: {e}"))
            .next(format!("remodel scene make {name} --replace   (needs the ROM)"))
    })
}

/// Draw a frame in hand: a scene's capture ([`draw`]) or a live machine's
/// ([`Frame::grab`], a session). `display` is the frame's display list,
/// which `o.camera` needs. Nothing here touches a machine.
pub fn draw_frame(
    game: &Game,
    mut f: Frame,
    rom: &[u16],
    display: Option<&model2::smooth::Frame>,
    o: &DrawOpts,
    gpu: &mut gpu::Renderer,
) -> Result<Drawn, Answer> {
    let started = Instant::now();
    let view = o.view.with_hud_windows(game.hud_windows, game.hud_element_windows);
    let light = crate::light_options(game);
    let mut side = f.light_frame();
    // The free camera: the display list through the native geometrizer
    // again, as photo mode does (`model2::photo::render`), and photo mode's
    // near plane and HUD.
    let (mut near, mut hide_hud, mut warnings) = (0.0, false, Vec::new());
    if let Some(spec) = &o.camera {
        let Some(display) = display else {
            return Err(Answer::fail(Code::Missing, "this frame has no display list, so no free camera"));
        };
        let posed = pose_frame(display, spec, light.as_ref().is_some_and(|l| l.runs()))?;
        (f.stream, side, near, hide_hud) = (posed.stream, posed.side, posed.near, !posed.hud);
        warnings = posed.warnings;
    }
    let src = f.sources(rom);
    let mut polys = raster::build_shaded_near(&f.stream, &src, side.as_ref(), light.as_ref(), near);
    video::HudLayout::new(&f.tiles, &f.palette, &view).anchor(&mut polys, &view);
    let mut placed = raster::place(&mut polys, &view);
    if hide_hud {
        placed.retain(|p| {
            let w = polys[p.poly].window;
            w >= 32 || view.hud_windows >> w & 1 == 0
        });
    }
    // Photo mode's picture without the HUD: its layer transparent (the
    // original patterns, not a pack's tile images).
    let hidden = |mut layers: video::Layers2d, has_3d: bool| {
        if hide_hud {
            if layers.tiles.is_some() {
                layers = video::layers_2d(&f.tiles, &f.palette, &view, has_3d);
            }
            layers.hud.fill(0);
        }
        layers
    };
    let layers = hidden(video::layers_2d(&f.tiles, &f.palette, &view, !placed.is_empty()), !placed.is_empty());
    gpu.set_quality(gpu::Quality {
        msaa: crate::AA.get().copied().unwrap_or(1),
        depth: crate::DEPTH.get().copied().unwrap_or(false),
        tex_filter: crate::TEX_FILTER.get().copied().unwrap_or_default(),
        ..crate::gpu_light(light.as_ref())
    });
    let (w, h) = (view.width, view.height);
    let mut original = None;
    if o.against {
        gpu.set_pack(None);
        gpu.set_look(None);
        original = Some(Picture { width: w, height: h, rgb: gpu.frame(&polys, &placed, &src, &view, &layers) });
    }
    let mut packed = false;
    if !o.pack.is_empty() {
        if let Err(e) = crate::pack_layers::check_layers(&o.pack) {
            return Err(Answer::fail(Code::BadRequest, e).next("remodel pack resolve DIR..."));
        }
        let mut layers: Vec<Box<dyn model2::enhance::TexturePack + Send>> = Vec::new();
        for dir in &o.pack {
            match crate::texpack::accept(dir) {
                Ok(_) => match crate::texpack::Packs::new(None, Some(dir.clone())) {
                    Ok(p) => layers.push(Box::new(p)),
                    Err(e) => return Err(Answer::fail(Code::Missing, e)),
                },
                Err(e) => warnings.push(format!("pack {} not drawn: {e}", dir.display())),
            }
        }
        packed = !layers.is_empty();
        match layers.len() {
            0 => gpu.set_pack(None),
            1 => gpu.set_pack(layers.pop()),
            _ => gpu.set_pack(Some(Box::new(model2::layered::LayeredPack::new(layers)))),
        }
    } else {
        gpu.set_pack(None);
    }
    gpu.set_look(o.look.clone());
    gpu.set_overlay_miss(crate::OVERLAY_MISS.load(std::sync::atomic::Ordering::Relaxed));
    // What the pack's lookups gave, by texture: the pack log, in scratch.
    let log_path = packed.then(|| {
        let root = std::env::var("REMODEL_SCRATCH").unwrap_or_else(|_| "out/scratch".into());
        Path::new(&root).join(format!("scene-draw-{}.log", std::process::id()))
    });
    if let Some(path) = &log_path {
        if let Ok(log) = model2::packlog::PackLog::create(path) {
            gpu.set_pack_log(Some(log));
            gpu.set_log_frame(0);
        }
    }
    // The pack's tile images (HUD, menus, title) in place of the patterns:
    // the GPU's own 2D layers (twice: the first looks the patterns up and
    // fills the tile atlas).
    let has_3d = !placed.is_empty();
    let layers = if packed {
        gpu.layers_2d(&f.tiles, &f.palette, &view, has_3d);
        hidden(gpu.layers_2d(&f.tiles, &f.palette, &view, has_3d), has_3d)
    } else {
        layers
    };
    let rgb = gpu.frame(&polys, &placed, &src, &view, &layers);
    let drawn_share = log_path.map(|path| {
        gpu.set_pack_log(None);
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let _ = std::fs::remove_file(&path);
        drawn_share_of(&text)
    });
    let changed_share = original.as_ref().map(|orig| {
        let n = rgb.chunks(3).zip(orig.rgb.chunks(3)).filter(|(a, b)| a != b).count();
        n as f64 / (w * h) as f64
    });
    if o.against && !packed && crate::LOOK.get().is_none() {
        warnings.push(
            "nothing to compare: no --pack (or it was refused) and no --look, so both pictures are the original".into(),
        );
    }
    let miss_pixels = crate::OVERLAY_MISS.load(std::sync::atomic::Ordering::Relaxed).then(|| {
        let n = rgb.chunks(3).filter(|p| *p == raster::MISS_COLOUR).count();
        n as f64 / (w * h) as f64
    });
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
        drawn_share: drawn_share.flatten(),
        miss_pixels,
        polygons: polys.len(),
        warnings,
        millis: started.elapsed().as_secs_f64() * 1e3,
    })
}

/// The share of textured polygon area drawn from a pack image in a pack log
/// (`t` lines: outcome `hit`, `up` or `held` is a replacement). None when
/// no texture was drawn.
pub fn drawn_share_of(log: &str) -> Option<f64> {
    let (mut drawn, mut all) = (0.0f64, 0.0f64);
    for l in log.lines().filter(|l| l.starts_with("t\t")) {
        let f: Vec<&str> = l.split('\t').collect();
        let (Some(outcome), Some(px)) = (f.get(4), f.get(6).and_then(|v| v.parse::<f64>().ok())) else { continue };
        all += px;
        if matches!(*outcome, "hit" | "up" | "held") {
            drawn += px;
        }
    }
    (all > 0.0).then_some(drawn / all)
}

/// `pack_coverage`'s answer: the share of changed pixels by scene, and the scenes skipped.
pub type CoverageShares = (Vec<(String, f64, Option<f64>)>, Vec<String>);

/// `pack check`'s `coverage` gate: for each scene of `game` that has a local
/// copy, draw it with `pack` and without at the native view and count the
/// share of pixels that differ (0 to 1). Returns the shares by scene and
/// the names of scenes with no local copy (skipped). One GPU for all.
pub fn pack_coverage(game: &'static Game, project: Option<&Path>, pack: &Path) -> Result<CoverageShares, Answer> {
    let rs = recipes(game, project).map_err(|e| Answer::fail(Code::BadRequest, e))?;
    let root = scenes_dir(game);
    let (mut ready, mut missing) = (Vec::new(), Vec::new());
    for r in &rs {
        if state_of(&root.join(&r.name), Some(r)) == State::Missing {
            missing.push(r.name.clone());
        } else {
            ready.push(r);
        }
    }
    if ready.is_empty() {
        return Ok((vec![], missing));
    }
    let mut gpu = gpu::Renderer::new().map_err(|e| Answer::fail(Code::Internal, format!("no GPU: {e}")))?;
    let mut out = Vec::new();
    for r in ready {
        let o = DrawOpts {
            pack: vec![pack.to_path_buf()],
            crop: None,
            against: true,
            view: View::native(),
            camera: None,
            look: global_look(),
        };
        let d = draw(game, &root.join(&r.name), &o, &mut gpu)?;
        out.push((r.name.clone(), d.changed_share.unwrap_or(0.0), d.drawn_share));
    }
    Ok((out, missing))
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
    let rs = load_recipes(game, a)?;
    let [name] = a.pos.as_slice() else {
        return Err(Answer::fail(Code::BadRequest, "scene render: which scene? NAME").next("remodel scene list"));
    };
    let against = match a.one("--against") {
        None => false,
        Some("original") => true,
        Some(other) => {
            return Err(Answer::unknown("--against target", other, &["original".to_string()]));
        }
    };
    let mut packs: Vec<PathBuf> =
        a.vals.get("--pack").map(|v| v.iter().map(PathBuf::from).collect()).unwrap_or_default();
    if packs.is_empty() {
        packs.extend(o.pack_dir.clone());
    }
    let req = SceneRender {
        name: name.clone(),
        against,
        crop: a.one("--crop").map(parse_crop).transpose()?,
        view: parse_view(a.one("--view"))?,
        camera: a.one("--camera").map(str::to_string),
        packs,
        look: global_look(),
        // The overlay (the global `--overlay`, which `main` has parsed and
        // removed from the arguments): only `miss` is built.
        overlay: crate::OVERLAY_MISS.load(std::sync::atomic::Ordering::Relaxed),
    };
    // `--out DIR` is the CLI global option that says where `run` writes.
    let out = crate::OUT_DIR.get().cloned().unwrap_or_else(|| data_dir().join(game.id).join("scene-render"));
    render_scene(game, &rs, &req, &out, &mut |dir, opts| {
        let t_gpu = Instant::now();
        let mut gpu = gpu::Renderer::new().map_err(|e| Answer::fail(Code::Internal, format!("no GPU: {e}")))?;
        let gpu_ms = t_gpu.elapsed().as_secs_f64() * 1e3;
        Ok((draw(game, dir, opts, &mut gpu)?, gpu_ms))
    })
}

/// Draws a scene's capture: the GPU's start in ms with the picture.
pub type Drawer<'a> = dyn FnMut(&Path, &DrawOpts) -> Result<(Drawn, f64), Answer> + 'a;

/// What `scene render` is asked for.
pub struct SceneRender {
    pub name: String,
    pub against: bool,
    pub crop: Option<[i64; 4]>,
    pub view: View,
    pub camera: Option<String>,
    pub packs: Vec<PathBuf>,
    pub look: Option<std::sync::Arc<model2::look::Look>>,
    /// `--overlay miss`.
    pub overlay: bool,
}

/// The pictures of a drawn frame, written to `out` as `STEM[-original|-pack|-sheet].png`;
/// the paths by kind (`picture`, and `original` and `sheet` when drawn against the original).
pub fn write_drawn(out: &Path, stem: &str, d: &Drawn) -> Result<BTreeMap<String, String>, String> {
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut files = BTreeMap::new();
    let mut jobs: Vec<(PathBuf, Picture)> = Vec::new();
    let mut put = |k: &str, suffix: &str, p: Picture| {
        let path = out.join(format!("{stem}{suffix}.png"));
        files.insert(k.to_string(), path.display().to_string());
        jobs.push((path, p));
    };
    let copy = |p: &Picture| Picture { width: p.width, height: p.height, rgb: p.rgb.clone() };
    let sheet = d.original.as_ref().map(|orig| side_by_side(orig, &d.picture));
    if let (Some(orig), Some(sheet)) = (d.original.as_ref(), sheet) {
        put("original", "-original", copy(orig));
        put("picture", "-pack", copy(&d.picture));
        put("sheet", "-sheet", sheet);
    } else {
        put("picture", "", copy(&d.picture));
    }
    // The PNGs are encoded side by side: at 1080p each costs about 0.2 s.
    let failed: Vec<String> = std::thread::scope(|sc| {
        let hs: Vec<_> = jobs.iter().map(|(path, p)| sc.spawn(move || save_png(path, p))).collect();
        hs.into_iter().filter_map(|h| h.join().unwrap_or_else(|_| Err("png thread panicked".into())).err()).collect()
    });
    match failed.into_iter().next() {
        Some(e) => Err(e),
        None => Ok(files),
    }
}

/// Draw a scene's capture (`scene render`), through `drawer`, which makes
/// the GPU (or has one kept warm, the server's) and says how long that took.
pub fn render_scene(
    game: &'static Game,
    rs: &[Recipe],
    req: &SceneRender,
    out: &Path,
    drawer: &mut Drawer,
) -> Result<(Answer, String), Answer> {
    let started = Instant::now();
    let recipe = find_recipe(rs, &req.name)?;
    let (camera, crop, packs) = (&req.camera, req.crop, &req.packs);
    if let Some(spec) = camera {
        // A wrong word is a wrong request whatever the scene holds.
        model2::photo::Spec::parse(spec, 1.0).map_err(|e| Answer::fail(Code::BadRequest, format!("--camera: {e}")))?;
    }
    let overlay = req.overlay.then_some("miss");
    if overlay.is_some() && packs.is_empty() {
        return Err(Answer::fail(
            Code::BadRequest,
            "--overlay miss needs a pack: with none, every textured polygon would be a miss",
        )
        .next("remodel scene render NAME --pack DIR --overlay miss"));
    }
    let dir = scenes_dir(game).join(&recipe.name);
    if state_of(&dir, Some(recipe)) == State::Missing {
        return Err(Answer::fail(Code::Missing, format!("scene `{}` has no local copy", recipe.name))
            .next(format!("remodel scene make {}   (needs the ROM)", recipe.name)));
    }
    if camera.is_some() {
        read_display(&dir)?;
    }
    let opts = DrawOpts {
        pack: packs.clone(),
        crop,
        against: req.against,
        view: req.view,
        camera: camera.clone(),
        look: req.look.clone(),
    };
    let (d, gpu_ms) = drawer(&dir, &opts)?;
    // Render output is regenerated every time (the same files for the same
    // inputs), so it is replaced in place; drafts and approved images are
    // never written here.
    let t_write = Instant::now();
    let files = write_drawn(out, &recipe.name, &d).map_err(|e| Answer::fail(Code::Internal, e))?;
    let write_ms = t_write.elapsed().as_secs_f64() * 1e3;
    let ms = started.elapsed().as_secs_f64() * 1e3;
    let data = json!({
        "scene": recipe.name, "picture": files["picture"], "original": files.get("original"), "sheet": files.get("sheet"),
        "width": d.picture.width, "height": d.picture.height, "polygons": d.polygons,
        "changed_share": d.changed_share.map(|c| (c * 1e6).round() / 1e6),
        "drawn_share": d.drawn_share.map(|c| (c * 1e6).round() / 1e6),
        "overlay": overlay,
        "miss_share": overlay.and(d.drawn_share).map(|c| ((1.0 - c) * 1e6).round() / 1e6),
        "miss_pixels": d.miss_pixels.map(|c| (c * 1e6).round() / 1e6),
        "pack": packs.first().map(|p| p.display().to_string()),
        "layers": packs.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        "crop": crop, "camera": camera, "ms": ms.round(), "ms_gpu_init": gpu_ms.round(), "ms_draw": d.millis.round(), "ms_write": write_ms.round(),
    });
    let mut text = format!(
        "scene         {} (frame {}), {}x{}{}\n",
        recipe.name,
        recipe.frame,
        d.picture.width,
        d.picture.height,
        if crop.is_some() { " crop" } else { "" }
    );
    if let Some(c) = &camera {
        text += &format!("camera        {c} (the display list through the native geometrizer; the game's HUD left out unless hud=on)\n");
    }
    for (k, v) in &files {
        text += &format!("{k:<13} {v}\n");
    }
    if let Some(c) = d.changed_share {
        text += &format!("changed       {:.2}% of pixels against the original\n", c * 100.0);
    }
    if let Some(c) = d.drawn_share {
        text += &format!(
            "drawn         {:.2}% of the textured polygon area from pack images (by identity, overlaps counted)\n",
            c * 100.0
        );
    }
    if let (Some(share), Some(px)) = (d.drawn_share.filter(|_| overlay.is_some()), d.miss_pixels) {
        text += &format!(
            "overlay miss  {:.2}% of the textured polygon area drew from no pack image ({:.2}% of the pixels are magenta)\n",
            (1.0 - share) * 100.0,
            px * 100.0
        );
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

    fn pose(dir: &Path, spec: &str, lit: bool) -> Result<Posed, Answer> {
        pose_frame(&display_frame(dir)?, spec, lit)
    }

    #[test]
    fn the_drawn_share_counts_polygon_area_of_replaced_textures() {
        let log = "# header\nF\t0\n\
            t\t0000000000000001\t00000000000000aa\t8x8\thit\t2\t300\t1\t1\t0:0:0\n\
            t\t0000000000000002\t00000000000000aa\t8x8\tup\t1\t100\t1\t1\t0:0:0\n\
            t\t0000000000000003\t00000000000000bb\t8x8\tmiss:not-in-pack\t1\t600\t1\t1\t0:0:0\n\
            k\t00000001\t0000000000000004\thit\t99\t1\t1\n";
        assert_eq!(drawn_share_of(log), Some(0.4), "tiles do not count");
        assert_eq!(drawn_share_of("F\t0\n"), None);
    }

    /// A pack's `tile_<key>.png` replaces the pattern in a scene's picture:
    /// `scene render --pack` draws the 2D layers through the pack as the
    /// player does. Needs the `race-go` scene copy and a GPU (skipped
    /// without).
    #[test]
    fn a_pack_tile_image_reaches_the_scene_picture() {
        let dir = scenes_dir(game()).join("race-go");
        let Some(frame_dir) = capture_frame_dir(&dir.join("capture")) else { return };
        let Ok(mut gpu) = gpu::Renderer::new() else { return };
        let f = Frame::load(&frame_dir).unwrap();
        // The most used tile pattern of the frame's enabled maps.
        let mut uses: HashMap<u64, usize> = HashMap::new();
        for map in (0..4).filter(|&m| video::map_enabled(&f.tiles, m)) {
            for i in 0..4096 {
                let (key, _) = video::cell_key(&f.tiles, &f.palette, map, i & 63, i >> 6);
                if key != model2::regions::BLANK {
                    *uses.entry(key).or_default() += 1;
                }
            }
        }
        let Some((&key, _)) = uses.iter().max_by_key(|(k, n)| (**n, **k)) else { return };
        let root = std::env::var("REMODEL_SCRATCH").unwrap_or_else(|_| "out/scratch".into());
        let pack = Path::new(&root).join(format!("scene-tile-test-{}", std::process::id()));
        std::fs::create_dir_all(&pack).unwrap();
        let magenta: Vec<u8> = (0..32 * 32).flat_map(|_| [255u8, 0, 255, 255]).collect();
        std::fs::write(pack.join(format!("tile_{key:016x}.png")), png::encode_rgba(32, 32, &magenta)).unwrap();
        let opts = |pack: Vec<PathBuf>| DrawOpts {
            pack,
            crop: None,
            against: false,
            view: View::new(540, 16.0 / 9.0, video::Hud::Edges),
            camera: None,
            look: None,
        };
        let count = |d: &Drawn| d.picture.rgb.chunks(3).filter(|p| **p == [255, 0, 255]).count();
        let plain = draw(game(), &dir, &opts(vec![]), &mut gpu).unwrap_or_else(|e| panic!("{e:?}"));
        let tiled = draw(game(), &dir, &opts(vec![pack.clone()]), &mut gpu).unwrap_or_else(|e| panic!("{e:?}"));
        let _ = std::fs::remove_dir_all(&pack);
        assert!(count(&tiled) > count(&plain) + 50, "tile image missing: {} against {}", count(&tiled), count(&plain));
    }

    fn game() -> &'static Game {
        romset::games::find("topskatr").unwrap()
    }

    /// A Top Skater scene copy with a display list (`scene make` made it
    /// from the ROM; the test is skipped without one), a 3D-rich one first.
    fn scene_with_display() -> Option<PathBuf> {
        let root = scenes_dir(game());
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.join(DISPLAY_FILE).is_file() && has_capture(p))
            .collect();
        let rank = |p: &PathBuf| ["course-4800", "race-course", "race-go"].iter().position(|n| scene_name(p) == *n);
        dirs.sort_by_key(|p| (rank(p).unwrap_or(9), p.clone()));
        dirs.into_iter().next()
    }

    /// Photo mode's picture of a scene's frame and camera: the player's own
    /// draw (`gpu::Renderer::photo`, as `remodel-play`'s photo mode calls
    /// it) of a board frame made from the scene's capture and the display
    /// list's replay, at the view, as RGB.
    fn photo_picture(dir: &Path, spec: &str, view: &View, gpu: &mut gpu::Renderer) -> Vec<u8> {
        let cap = dir.join("capture");
        let rom: std::sync::Arc<[u16]> = capture::load_texture_rom(&cap).unwrap().into();
        let f = Frame::load(&capture_frame_dir(&cap).unwrap()).unwrap();
        let sf = model2::smooth::Frame::from_bytes(&std::fs::read(dir.join(DISPLAY_FILE)).unwrap()).unwrap();
        let spec = model2::photo::Spec::parse(spec, model2::photo::scene_scale(sf.plan.transforms())).unwrap();
        let (stream, side) = model2::photo::render(&sf, &spec.camera, 1.0, false);
        let board = gpu::BoardFrame::from_capture(&f, rom, None);
        let shot =
            gpu::PhotoShot { stream: &stream, points: &f.texture_point_ram, light: side.map(std::sync::Arc::new) };
        let scale = model2::photo::scene_scale(sf.plan.transforms());
        let req = gpu::PhotoRequest {
            view: *view,
            shots: &[shot],
            hud: spec.hud,
            hide_base: false,
            look: gpu::PhotoLook::default(),
            bloom_levels: 0,
            near: spec.near_plane(scale),
        };
        let (w, h) = (view.width as u32, view.height as u32);
        let format = gpu::wgpu::TextureFormat::Rgba8Unorm;
        let texture = gpu.device().create_texture(&gpu::wgpu::TextureDescriptor {
            label: Some("photo test"),
            size: gpu::wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: gpu::wgpu::TextureDimension::D2,
            format,
            usage: gpu::wgpu::TextureUsages::RENDER_ATTACHMENT | gpu::wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        gpu.photo(&board, &req, &texture.create_view(&Default::default()), format, (0, 0), (w, h));
        let rgba = gpu.read_texture(&texture, w, h);
        rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect()
    }

    /// Pixels that differ by more than 16 in a channel (the GPU check's
    /// "large" difference), as a share.
    fn large_share(a: &[u8], b: &[u8]) -> f64 {
        assert_eq!(a.len(), b.len());
        let n =
            a.chunks(3).zip(b.chunks(3)).filter(|(p, q)| p.iter().zip(*q).any(|(x, y)| x.abs_diff(*y) > 16)).count();
        n as f64 / (a.len() / 3) as f64
    }

    /// The display list, run again through the native geometrizer with no
    /// machine, is the frame the capture holds word for word; a moved
    /// camera is not. Needs a Top Skater scene copy with a display list
    /// (skipped without).
    #[test]
    fn a_scenes_display_list_replays_its_captured_frame() {
        let Some(dir) = scene_with_display() else { return };
        let f = Frame::load(&capture_frame_dir(&dir.join("capture")).unwrap()).unwrap();
        let again = pose(&dir, "hud=on", false).unwrap_or_else(|e| panic!("{e:?}"));
        assert!(again.stream == f.stream, "{}: the replay is not the captured stream", dir.display());
        assert_eq!(again.near, 0.0);
        let moved = pose(&dir, "orbit=30,tilt=-8", false).unwrap_or_else(|e| panic!("{e:?}"));
        assert!(moved.stream != f.stream && !moved.stream.is_empty());
        assert!(moved.near > 0.0 && !moved.hud);
    }

    /// `scene render --camera` draws what photo mode draws for the frame
    /// and camera, within the GPU check's tolerance (no more than 1% of
    /// pixels off by over 16 in a channel); the game's own view, with its
    /// HUD, is the plain scene picture. Needs a Top Skater scene copy with
    /// a display list and a GPU (skipped without).
    #[test]
    fn a_scene_camera_matches_photo_modes_picture() {
        let Some(dir) = scene_with_display() else { return };
        let Ok(mut gpu) = gpu::Renderer::new() else { return };
        let view = View::new(540, 16.0 / 9.0, video::Hud::Edges);
        let opts = |camera: Option<&str>| DrawOpts {
            pack: vec![],
            crop: None,
            against: false,
            view,
            camera: camera.map(str::to_string),
            look: None,
        };
        let plain = draw(game(), &dir, &opts(None), &mut gpu).unwrap_or_else(|e| panic!("{e:?}"));
        let same = draw(game(), &dir, &opts(Some("hud=on")), &mut gpu).unwrap_or_else(|e| panic!("{e:?}"));
        assert!(
            large_share(&plain.picture.rgb, &same.picture.rgb) < 1e-4,
            "the game's view differs from the plain picture"
        );
        let shown = view.with_hud_windows(game().hud_windows, game().hud_element_windows);
        for spec in ["orbit=30,tilt=-8", "orbit=-100,tilt=25,x=0.4,fov=1.3", "yaw=15,roll=5,z=0.5", "hud=off"] {
            let ours = draw(game(), &dir, &opts(Some(spec)), &mut gpu).unwrap_or_else(|e| panic!("{e:?}"));
            let theirs = photo_picture(&dir, spec, &shown, &mut gpu);
            let share = large_share(&ours.picture.rgb, &theirs);
            eprintln!("{spec}: {:.4}% of pixels differ from photo mode by over 16", share * 100.0);
            assert!(share < 0.01, "{spec}: {:.3}% of pixels differ from photo mode's", share * 100.0);
        }
    }

    #[test]
    fn a_scene_without_a_display_list_says_to_make_it_again() {
        let root = std::env::var("REMODEL_SCRATCH").unwrap_or_else(|_| "out/scratch".into());
        let dir = Path::new(&root).join(format!("scene-no-display-{}", std::process::id())).join("old-scene");
        std::fs::create_dir_all(&dir).unwrap();
        let e = pose(&dir, "orbit=10", false).err().expect("no display.bin");
        assert_eq!(e.code, Code::Missing);
        assert!(e.message().contains("scene make old-scene"), "{}", e.message());
        assert_eq!(e.next.as_deref(), Some("remodel scene make old-scene --replace   (needs the ROM)"));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn camera_words_are_photo_modes_and_strict() {
        let spec = model2::photo::Spec::parse("back=60,orbit=30,tilt=-8,fov=1.5,near=2", 10.0).unwrap();
        assert_eq!(spec.near, 0.02);
        assert_eq!(spec.camera.zoom, 1.5);
        assert!(!spec.camera.is_identity() && !spec.hud);
        assert_eq!(spec.warnings.len(), 1, "back has no history to scrub");
        let e = model2::photo::Spec::parse("orbit=thirty", 1.0).unwrap_err();
        assert!(e.contains("wants a number"), "{e}");
        let e = model2::photo::Spec::parse("dolly=3", 1.0).unwrap_err();
        assert!(e.contains("unknown camera word") && e.contains("orbit"), "{e}");
        assert!(model2::photo::Spec::parse("hud=maybe", 1.0).is_err());
        assert!(model2::photo::Spec::parse("", 1.0).unwrap().camera.is_identity());
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
