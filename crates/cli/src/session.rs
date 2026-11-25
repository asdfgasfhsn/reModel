//! Live game sessions: the verbs of `remodel serve` (docs/uplift/scenes-and-server.md,
//! "The live session"), as one table that the MCP tools list, the argument
//! checks and the command line (`remodel call`) all read, so they cannot
//! drift.
//!
//! A session is a machine of its own that advances only when told to step
//! (lockstep), the inputs it was given (its log, which `session.export`
//! writes as a branch tour that replays to the same state hash), named
//! copies of the machine (marks), and render state (camera) that never
//! reaches the log.
//!
//! **Inputs.** What is held applies to the frame the session is on (the one
//! about to run) and every frame after, as a tour's script does; the frame
//! the session started on keeps the inputs of what it started from (a
//! tour's, a scene recipe's, none at power-on). So a session that sets
//! `start` and steps 30 frames runs 30 frames with `start`, and the log is
//! exact: the same frames and state hash as a tour of those terms.

use crate::answer::{Answer, Code};
use crate::tour::{self, ValueDef};
use model2::bus::CopyMarks;
use model2::input::{InputScript, Recorder};
use model2::Machine;
use romset::{Game, RomSet};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

// ------------------------------------------------------------------ the verb table

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    Str,
    Int,
    /// A list of strings.
    List,
}

#[derive(Debug)]
pub struct Arg {
    pub name: &'static str,
    pub ty: Ty,
    pub required: bool,
    pub about: &'static str,
}

#[derive(Debug)]
pub struct Verb {
    pub name: &'static str,
    pub about: &'static str,
    pub args: &'static [Arg],
}

const fn arg(name: &'static str, ty: Ty, required: bool, about: &'static str) -> Arg {
    Arg { name, ty, required, about }
}

const SESSION: Arg =
    arg("session", Ty::Str, false, "the session's id (from session.open); optional while exactly one is open");
const VIEW: Arg =
    arg("view", Ty::Str, false, "native, HEIGHT or HEIGHT,A:B (for example 1080,16:9); default 1080p 16:9");
const PACK: Arg = arg("pack", Ty::List, false, "texture pack folders, highest first (layered)");
const LOOK: Arg = arg("look", Ty::Str, false, "comic, off, or the path of a look file (default: none)");
const CROP: Arg = arg("crop", Ty::Str, false, "X,Y,W,H in the game's own 496x384 picture");
const AGAINST: Arg =
    arg("against", Ty::Str, false, "original: also draw the picture with no pack or look, and a side by side");
const CAMERA: Arg = arg(
    "camera",
    Ty::Str,
    false,
    "photo mode's camera words for this picture: orbit=30,tilt=-8,z=1 (back, orbit, tilt, yaw, pitch, roll, x, y, z, fov, near, hud)",
);
const NAME: Arg = arg("name", Ty::Str, false, "file name stem for the picture (letters, digits, - and _)");

/// Every verb the server has: the MCP tools, and what `remodel call` takes.
pub static VERBS: &[Verb] = &[
    Verb {
        name: "session.open",
        about: "Open a session: a machine of its own that runs only when stepped. from: power-on, scene:NAME (a made scene's state) or tour:LABEL@FRAME (as a branch tour does).",
        args: &[
            arg("from", Ty::Str, false, "power-on (default), scene:NAME or tour:LABEL@FRAME"),
            arg("game", Ty::Str, false, "the game; must be the one this server runs"),
        ],
    },
    Verb {
        name: "session.input",
        about: "Set the inputs held from the current frame on (replacing what was held): buttons by the game's input names (start, punch2), analog channels (steer=0x80), guns (gun1=10:20). An empty list releases everything.",
        args: &[SESSION, arg("inputs", Ty::List, false, "the held inputs, e.g. [\"start\", \"steer=0x80\"]")],
    },
    Verb {
        name: "session.step",
        about: "Run frames, and only these. Optionally with inputs for just those frames, and stopping early at a condition. Never more than the server's --max-step frames.",
        args: &[
            SESSION,
            arg("frames", Ty::Int, true, "how many frames to run (at most --max-step)"),
            arg("inputs", Ty::List, false, "inputs held for just this step (as session.input)"),
            arg("until", Ty::Str, false, "stop early: `value NAME OP V` (OP gt ge lt le eq ne; NAME from the game's value table) or `frames N`"),
        ],
    },
    Verb {
        name: "session.mark",
        about: "Keep a named copy of the machine, to return to. A mark costs one machine of memory; the server holds at most --marks of them per session.",
        args: &[SESSION, arg("name", Ty::Str, true, "the mark's name (letters, digits, - and _); an existing one is replaced")],
    },
    Verb {
        name: "session.restore",
        about: "Return to a mark: the machine, the log and the held inputs go back to how they were.",
        args: &[SESSION, arg("name", Ty::Str, true, "the mark")],
    },
    Verb {
        name: "session.forget",
        about: "Drop a mark and free its memory.",
        args: &[SESSION, arg("name", Ty::Str, true, "the mark")],
    },
    Verb {
        name: "session.read",
        about: "Read named values from the game's value table, or raw memory as ADDR[:b|w|l] (little-endian; default l). No names: every named value.",
        args: &[SESSION, arg("names", Ty::List, false, "value names, or ADDR[:b|w|l]")],
    },
    Verb {
        name: "session.poke",
        about: "Hold a value in RAM for some frames, as a tour's poke= does (a cheat: logged, and the exported tour is marked). WHAT is a name from the game's poke table or ADDR:VALUE[:b|w|l].",
        args: &[
            SESSION,
            arg("what", Ty::Str, true, "a poke-table name, or ADDR:VALUE[:b|w|l]"),
            arg("frames", Ty::Int, false, "how many frames from the current one (default 1)"),
        ],
    },
    Verb {
        name: "session.camera",
        about: "Set (or with no argument, read) photo mode's camera for this session's pictures. It is render state only and never enters the log. reset or an empty string returns to the game's own.",
        args: &[SESSION, CAMERA],
    },
    Verb {
        name: "session.render",
        about: "Draw the current frame (the PNG goes to a file; the answer returns its path), with a view, pack, look, crop and the session's camera (or one for this picture). A camera needs the machine to have stepped at least once since opening or restoring.",
        args: &[SESSION, VIEW, PACK, LOOK, CROP, AGAINST, CAMERA, NAME],
    },
    Verb {
        name: "session.export",
        about: "The session's log as a branch tour line (LABEL|LENGTH|from=SPINE@F INPUTS) that replays to the same state hash with `remodel tour run`.",
        args: &[
            SESSION,
            arg("label", Ty::Str, false, "the tour's name (default session-ID)"),
            arg("file", Ty::Str, false, "also write the line to this file name, in the server's output folder"),
        ],
    },
    Verb { name: "session.close", about: "Free the session's machine and marks.", args: &[SESSION] },
    Verb {
        name: "scene.render",
        about: "A still of a made scene, drawn from its capture with no emulator (the server keeps the GPU warm). The scene must have been made with `remodel scene make`.",
        args: &[
            arg("scene", Ty::Str, true, "the scene's name"),
            VIEW,
            PACK,
            LOOK,
            CROP,
            AGAINST,
            CAMERA,
        ],
    },
];

fn ty_name(t: Ty) -> &'static str {
    match t {
        Ty::Str => "a string",
        Ty::Int => "a whole number",
        Ty::List => "a list of strings",
    }
}

/// The MCP tool listing for the table.
pub fn tools() -> Value {
    let tools: Vec<Value> = VERBS
        .iter()
        .map(|v| {
            let props: serde_json::Map<String, Value> = v
                .args
                .iter()
                .map(|a| {
                    let mut p = match a.ty {
                        Ty::Str => json!({"type": "string"}),
                        Ty::Int => json!({"type": "integer", "minimum": 0}),
                        Ty::List => json!({"type": "array", "items": {"type": "string"}}),
                    };
                    p["description"] = json!(a.about);
                    (a.name.to_string(), p)
                })
                .collect();
            let required: Vec<&str> = v.args.iter().filter(|a| a.required).map(|a| a.name).collect();
            json!({
                "name": v.name,
                "description": v.about,
                "inputSchema": {"type": "object", "properties": props, "required": required, "additionalProperties": false},
            })
        })
        .collect();
    json!(tools)
}

pub fn find_verb(name: &str) -> Result<&'static Verb, Answer> {
    VERBS.iter().find(|v| v.name == name).ok_or_else(|| {
        let known: Vec<String> = VERBS.iter().map(|v| v.name.to_string()).collect();
        Answer::unknown("verb", name, &known).next("remodel call --help")
    })
}

/// Check `args` against the verb's table entry: an object, no unknown
/// names, the right types, the required ones present.
pub fn validate(v: &Verb, args: &Value) -> Result<(), Answer> {
    let empty = serde_json::Map::new();
    let obj = match args {
        Value::Null => &empty,
        Value::Object(o) => o,
        _ => return Err(Answer::fail(Code::BadRequest, format!("{}: the arguments are an object", v.name))),
    };
    let names: Vec<String> = v.args.iter().map(|a| a.name.to_string()).collect();
    for (k, val) in obj {
        let Some(a) = v.args.iter().find(|a| a.name == k) else {
            let mut e = Answer::unknown("argument", k, &names);
            e = e.next(usage(v));
            return Err(e);
        };
        let ok = match a.ty {
            Ty::Str => val.is_string(),
            Ty::Int => val.is_u64(),
            Ty::List => val.is_array() && val.as_array().is_some_and(|l| l.iter().all(Value::is_string)),
        };
        if !ok {
            return Err(Answer::fail(Code::BadRequest, format!("{}: `{k}` is {}, got {val}", v.name, ty_name(a.ty)))
                .next(usage(v)));
        }
    }
    for a in v.args.iter().filter(|a| a.required) {
        if !obj.contains_key(a.name) {
            return Err(Answer::fail(Code::BadRequest, format!("{}: `{}` is needed ({})", v.name, a.name, a.about))
                .next(usage(v)));
        }
    }
    Ok(())
}

/// `remodel call VERB name=value ...` for a verb.
pub fn usage(v: &Verb) -> String {
    let args: Vec<String> =
        v.args.iter().map(|a| if a.required { format!("{}=..", a.name) } else { format!("[{}=..]", a.name) }).collect();
    format!("remodel call {} {}", v.name, args.join(" "))
}

/// The arguments of `remodel call VERB key=value...` as the JSON the tool
/// takes, by the verb's table: numbers and booleans by type, a list by
/// repeating the key.
pub fn args_from_pairs(v: &Verb, pairs: &[String]) -> Result<Value, Answer> {
    let mut o = serde_json::Map::new();
    let names: Vec<String> = v.args.iter().map(|a| a.name.to_string()).collect();
    for p in pairs {
        let (k, val) = p.split_once('=').ok_or_else(|| {
            Answer::fail(Code::BadRequest, format!("{}: `{p}` is not name=value", v.name)).next(usage(v))
        })?;
        let a =
            v.args.iter().find(|a| a.name == k).ok_or_else(|| Answer::unknown("argument", k, &names).next(usage(v)))?;
        let bad = |what: &str| Answer::fail(Code::BadRequest, format!("{}: `{k}` wants {what}, got `{val}`", v.name));
        match a.ty {
            Ty::Str => {
                o.insert(k.into(), json!(val));
            }
            Ty::Int => {
                let n = num(val).ok_or_else(|| bad("a whole number"))?;
                o.insert(k.into(), json!(n));
            }
            Ty::List => {
                let l = o.entry(k.to_string()).or_insert_with(|| json!([]));
                if !val.is_empty() {
                    l.as_array_mut().unwrap().push(json!(val));
                }
            }
        }
    }
    Ok(Value::Object(o))
}

fn num(s: &str) -> Option<u64> {
    match s.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

fn name_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn s_arg<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k).and_then(Value::as_str)
}

fn n_arg(args: &Value, k: &str) -> Option<u64> {
    args.get(k).and_then(Value::as_u64)
}

fn l_arg(args: &Value, k: &str) -> Vec<String> {
    args.get(k)
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn bad(msg: impl Into<String>) -> Answer {
    Answer::fail(Code::BadRequest, msg)
}

fn internal(msg: impl Into<String>) -> Answer {
    Answer::fail(Code::Internal, msg)
}

// ------------------------------------------------------------------ the render lock

/// A slot of the `render` lock class (`tools/lock`): held only while the
/// GPU draws, so a render does not wait behind a build.
struct RenderLock {
    _file: Option<std::fs::File>,
}

impl RenderLock {
    fn dir() -> PathBuf {
        if let Some(d) = std::env::var_os("REMODEL_LOCK_DIR") {
            return PathBuf::from(d);
        }
        // The main checkout's out/locks, as tools/lock finds it: from a
        // worktree, `.git` is a file naming MAIN/.git/worktrees/NAME.
        let mut d = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        loop {
            let g = d.join(".git");
            if g.is_dir() {
                return d.join("out/locks");
            }
            if g.is_file() {
                if let Some(line) = std::fs::read_to_string(&g).ok().and_then(|t| t.lines().next().map(str::to_string))
                {
                    if let Some((_, gd)) = line.split_once(':') {
                        if let Some((main, _)) = gd.trim().split_once("/.git/") {
                            return PathBuf::from(main).join("out/locks");
                        }
                    }
                }
                return d.join("out/locks");
            }
            if !d.pop() {
                return PathBuf::from("out/locks");
            }
        }
    }

    /// Take a slot, waiting up to `REMODEL_LOCK_WAIT` seconds (default 600).
    fn take() -> Result<RenderLock, Answer> {
        use std::os::unix::io::AsRawFd;
        // An ancestor (a `tools/lock render -- ...`) already holds it.
        if std::env::var("REMODEL_LOCKS_HELD").is_ok_and(|h| h.split(',').any(|c| c == "render")) {
            return Ok(RenderLock { _file: None });
        }
        let dir = Self::dir();
        let slots: u32 =
            std::env::var("REMODEL_LOCK_RENDER_SLOTS").ok().and_then(|s| s.parse().ok()).unwrap_or(3).max(1);
        let wait: u64 = std::env::var("REMODEL_LOCK_WAIT").ok().and_then(|s| s.parse().ok()).unwrap_or(600);
        std::fs::create_dir_all(&dir).map_err(|e| internal(format!("{}: {e}", dir.display())))?;
        let start = Instant::now();
        loop {
            for i in 0..slots {
                let path = dir.join(format!("render.{i}"));
                let Ok(f) = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&path)
                else {
                    continue;
                };
                // SAFETY: flock on a file descriptor this process owns.
                if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                    use std::io::Write;
                    let secs = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0.0, |d| d.as_secs_f64());
                    let info = json!({"pid": std::process::id(), "start": secs, "cmd": "remodel serve (drawing)"});
                    let _ = f.set_len(0);
                    let _ = (&f).write_all(format!("{info}\n").as_bytes());
                    return Ok(RenderLock { _file: Some(f) });
                }
            }
            if start.elapsed().as_secs() >= wait {
                return Err(Answer::fail(Code::Missing, format!("the render lock stayed taken for {wait} s"))
                    .next("tools/lock status"));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

// ------------------------------------------------------------------ sessions

/// How many frames before a picture the display list's recording starts
/// (as `scene make`'s).
const DISPLAY_LEAD: u64 = 3;

/// The most frames `session.open from tour:..` runs to get to its start.
const OPEN_MAX_FRAMES: u64 = 200_000;

type Sample = (u64, [u8; 4], [u8; 8]);

#[derive(Clone)]
struct PokeLog {
    /// `poke=ADDR:VALUE[:b|w|l]`.
    term: String,
    from: u64,
    to: u64,
}

struct Mark {
    name: String,
    frame: u64,
    hash: u64,
    m: Box<Machine>,
    log: Vec<Sample>,
    pokes: Vec<PokeLog>,
    held: Vec<String>,
}

/// What a session started from.
enum Start {
    PowerOn,
    Scene,
    Tour(String, u64),
}

pub struct Session {
    id: String,
    kind: Start,
    m: Box<Machine>,
    /// The frame the session started on: its inputs are the start's.
    start: u64,
    /// For a scene start: the recipe's inputs to its frame, the tour terms
    /// a replay starts with.
    base: Vec<String>,
    held: Vec<String>,
    log: Vec<Sample>,
    pokes: Vec<PokeLog>,
    marks: Vec<Mark>,
    camera: String,
    smooth_on: bool,
    /// The start (a tour it branches from) or a poke is a cheat.
    poked: bool,
    frames_run: u64,
}

impl Session {
    fn frame(&self) -> u64 {
        self.m.frames
    }

    /// Set the inputs of `frame` on the bus: `held`, and the pokes that
    /// cover it; note them in the log.
    fn apply(&mut self, game: &Game, frame: u64, held: &[String]) -> Result<(), String> {
        let mut items: Vec<String> = held.iter().map(|h| format!("{h}@{frame}")).collect();
        items.extend(
            self.pokes.iter().filter(|p| p.from <= frame && frame <= p.to).map(|p| format!("{}@{frame}", p.term)),
        );
        let script = InputScript::parse(&items.join(","), &game.controls)?;
        script.apply(frame, &mut self.m.bus);
        if frame > self.start {
            let s = (frame, self.m.bus.inputs, self.m.bus.analog);
            match self.log.last_mut() {
                Some(l) if l.0 == frame => *l = s,
                _ => self.log.push(s),
            }
        }
        Ok(())
    }

    /// Run one frame.
    fn run_frame(&mut self) -> Result<(), String> {
        let before = self.m.frames;
        while self.m.frames == before {
            self.m
                .run_slice(u64::MAX / 2)
                .map_err(|e| format!("the machine stopped at frame {}: {e:?}", self.m.frames))?;
        }
        // The sound link's commands go nowhere; they would pile up.
        self.m.bus.uart_log.clear();
        Ok(())
    }

    fn hash(&mut self) -> u64 {
        self.m.bus.state_hash()
    }

    fn marks_json(&self) -> Value {
        json!(self
            .marks
            .iter()
            .map(|k| json!({"name": k.name, "frame": k.frame, "state_hash": format!("{:016x}", k.hash)}))
            .collect::<Vec<_>>())
    }

    /// The log as a branch tour line's terms (after `from=`), and whether
    /// guns were in play (their aim is not logged).
    fn terms(&self, game: &Game) -> (Vec<String>, bool) {
        let mut rec = Recorder::default();
        for (f, i, a) in &self.log {
            rec.sample(*f, &game.controls, i, a);
        }
        let mut t = rec.terms();
        for p in &self.pokes {
            let to = p.to.min(self.frame());
            if p.from > to {
                continue;
            }
            t.push(if p.from == to {
                format!("{}@{}", p.term, p.from)
            } else {
                format!("{}@{}-{}", p.term, p.from, to)
            });
        }
        (t, !game.controls.guns.is_empty())
    }
}

/// `until`: when a step stops early.
#[derive(Debug)]
enum Until {
    Frames(u64),
    Value { def: ValueDef, op: &'static str, v: f64 },
}

fn parse_until(text: &str, table: &[ValueDef]) -> Result<Until, Answer> {
    let want = || bad(format!("until `{text}`: want `value NAME OP V` (OP: gt ge lt le eq ne) or `frames N`"));
    let w: Vec<&str> = text.split_whitespace().collect();
    match w[..] {
        ["frames", n] => Ok(Until::Frames(num(n).ok_or_else(want)?)),
        ["value", name, op, v] => {
            let op = match op {
                "gt" | ">" => "gt",
                "ge" | ">=" => "ge",
                "lt" | "<" => "lt",
                "le" | "<=" => "le",
                "eq" | "==" | "=" => "eq",
                "ne" | "!=" => "ne",
                _ => return Err(want()),
            };
            let v: f64 = match num(v) {
                Some(n) => n as f64,
                None => v.parse().map_err(|_| want())?,
            };
            let def = table.iter().find(|d| d.name == name).cloned().ok_or_else(|| {
                let all: Vec<String> = table.iter().map(|d| d.name.clone()).collect();
                Answer::unknown("value", name, &all).next("remodel ram values")
            })?;
            Ok(Until::Value { def, op, v })
        }
        _ => Err(want()),
    }
}

/// A named value now, as a number (None: unreadable or not finite).
fn read_def(m: &Machine, def: &ValueDef) -> Option<(u32, Value)> {
    let raw = m.bus.peek(def.addr, def.width)?;
    Some((raw, crate::ram::decode(def, raw)))
}

impl Until {
    fn met(&self, m: &Machine, ran: u64) -> bool {
        match self {
            Until::Frames(n) => ran >= *n,
            Until::Value { def, op, v } => {
                let Some(x) = read_def(m, def).and_then(|(_, val)| val.as_f64()) else { return false };
                match *op {
                    "gt" => x > *v,
                    "ge" => x >= *v,
                    "lt" => x < *v,
                    "le" => x <= *v,
                    "eq" => x == *v,
                    _ => x != *v,
                }
            }
        }
    }
}

/// `item` is a held input: a name, `name=VALUE` or `gun=X:Y`, with no frames.
fn check_held(items: &[String], game: &Game) -> Result<(), Answer> {
    for it in items {
        if it.contains('@') || it.contains(',') || it.starts_with("poke=") || it == "coin-start" {
            return Err(bad(format!(
                "input `{it}`: held inputs are a name (start), name=VALUE (steer=0x80) or gun1=X:Y, with no frames; use session.poke for pokes, and step for time"
            ))
            .next("remodel inputs"));
        }
        InputScript::parse(&format!("{it}@0"), &game.controls)
            .map_err(|e| bad(format!("input `{it}`: {e}")).next("remodel inputs"))?;
    }
    Ok(())
}

// ------------------------------------------------------------------ the server's state

/// The server's limits and where it writes.
#[derive(Clone)]
pub struct Config {
    pub marks: usize,
    pub max_step: u64,
    pub sessions: usize,
    pub tours: Option<PathBuf>,
    /// Pictures and exported tours go here.
    pub out: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Config { marks: 16, max_step: 3600, sessions: 4, tours: None, out: PathBuf::from("out/serve") }
    }
}

pub struct Server<'a> {
    pub game: &'static Game,
    roms: Option<&'a RomSet>,
    pub cfg: Config,
    sessions: BTreeMap<String, Session>,
    next_id: u32,
    gpu: Option<gpu::Renderer>,
    looks: HashMap<String, Arc<model2::look::Look>>,
    /// Machines freed by `forget` and `close`, for the next mark.
    #[allow(clippy::vec_box)] // a Machine is large: moving the box is cheap
    spare: Vec<Box<Machine>>,
    table: Vec<ValueDef>,
}

fn rss_bytes() -> u64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1).and_then(|p| p.parse::<u64>().ok()))
        .map_or(0, |pages| pages * 4096)
}

impl<'a> Server<'a> {
    pub fn new(game: &'static Game, roms: Option<&'a RomSet>, cfg: Config) -> Server<'a> {
        let table = tour::value_table(romset::games::values(game)).unwrap_or_default();
        Server {
            game,
            roms,
            cfg,
            sessions: BTreeMap::new(),
            next_id: 1,
            gpu: None,
            looks: HashMap::new(),
            spare: Vec::new(),
            table,
        }
    }

    /// Run a verb: the answer in the shared envelope, whatever happens.
    pub fn call(&mut self, verb: &str, args: &Value) -> Answer {
        let v = match find_verb(verb) {
            Ok(v) => v,
            Err(a) => return a,
        };
        if let Err(a) = validate(v, args) {
            return a;
        }
        let r = match verb {
            "session.open" => self.open(args),
            "session.input" => self.with(args, Self::input),
            "session.step" => self.with(args, Self::step),
            "session.mark" => self.with(args, Self::mark),
            "session.restore" => self.with(args, Self::restore),
            "session.forget" => self.with(args, Self::forget),
            "session.read" => self.with(args, Self::read),
            "session.poke" => self.with(args, Self::poke),
            "session.camera" => self.with(args, Self::camera),
            "session.render" => self.with(args, Self::render),
            "session.export" => self.with(args, Self::export),
            "session.close" => self.close(args),
            "scene.render" => self.scene_render(args),
            _ => Err(internal(format!("{verb} is in the table but not wired"))),
        };
        r.unwrap_or_else(|a| a)
    }

    fn roms(&self) -> Result<&'a RomSet, Answer> {
        self.roms.ok_or_else(|| {
            Answer::fail(Code::Missing, format!("no ROMs for {}: the server has no machine to run", self.game.id))
                .next(format!("put the {} ROMs in the ROM folder and run remodel serve again", self.game.id))
        })
    }

    /// The session `args` names (the only one open if it names none),
    /// taken out for the verb and put back after.
    fn with(
        &mut self,
        args: &Value,
        f: fn(&mut Self, &mut Session, &Value) -> Result<Answer, Answer>,
    ) -> Result<Answer, Answer> {
        let id = match s_arg(args, "session") {
            Some(id) => id.to_string(),
            None => {
                let mut ids = self.sessions.keys();
                match (ids.next(), ids.next()) {
                    (Some(only), None) => only.clone(),
                    (None, _) => {
                        return Err(bad("no session is open").next("session.open"));
                    }
                    _ => {
                        let all: Vec<&str> = self.sessions.keys().map(String::as_str).collect();
                        return Err(bad(format!("which session? several are open: {}", all.join(", "))));
                    }
                }
            }
        };
        let Some(mut s) = self.sessions.remove(&id) else {
            let all: Vec<String> = self.sessions.keys().cloned().collect();
            return Err(Answer::unknown("session", &id, &all).next("session.open"));
        };
        let r = f(self, &mut s, args);
        self.sessions.insert(id, s);
        r
    }

    fn fresh_machine(&self, roms: &RomSet) -> Box<Machine> {
        let mut m = Box::new(crate::machine(roms));
        if crate::GEO_MODE.get().is_none() {
            // The native geometrizer where the game has one: the same
            // frames and state hashes, much faster (as tours run).
            m.bus.geo.set_mode(model2::geo::GeoMode::Native);
        }
        m
    }

    // ---------------------------------------------------------------- open

    fn open(&mut self, args: &Value) -> Result<Answer, Answer> {
        let game = self.game;
        if let Some(g) = s_arg(args, "game") {
            if g != game.id {
                return Err(
                    bad(format!("this server runs {}, not {g}", game.id)).next(format!("remodel serve --game {g}"))
                );
            }
        }
        let roms = self.roms()?;
        if self.sessions.len() >= self.cfg.sessions {
            let ids: Vec<&str> = self.sessions.keys().map(String::as_str).collect();
            return Err(bad(format!(
                "{} sessions are open, the server's limit (--sessions): {}",
                self.sessions.len(),
                ids.join(", ")
            ))
            .with_data(json!({"limit": self.cfg.sessions, "open": ids}))
            .next("session.close"));
        }
        let from = s_arg(args, "from").unwrap_or("power-on").to_string();
        let t0 = Instant::now();
        let rss0 = rss_bytes();
        let id = format!("s{}", self.next_id);
        let mut m = self.fresh_machine(roms);
        let (kind, start, base, poked, mut warnings) = match from.as_str() {
            "power-on" => {
                InputScript::default().apply(0, &mut m.bus);
                (Start::PowerOn, 0, vec![], false, vec![])
            }
            f if f.starts_with("scene:") => {
                let name = &f[6..];
                let rs = crate::scene::recipes(game, None).map_err(bad)?;
                let r = rs.iter().find(|r| r.name == name).ok_or_else(|| {
                    let known: Vec<String> = rs.iter().map(|r| r.name.clone()).collect();
                    Answer::unknown("scene", name, &known).next("remodel scene list")
                })?;
                let dir = crate::scene::scenes_dir(game).join(name);
                let data = std::fs::read(dir.join("state.save")).map_err(|_| {
                    Answer::fail(Code::Missing, format!("scene `{name}` has no local copy (no state.save)"))
                        .next(format!("remodel scene make {name}   (needs the ROM)"))
                })?;
                model2::load_session(&mut m, None, &data).map_err(|e| {
                    Answer::fail(Code::Missing, format!("scene `{name}`: its state does not load: {e}"))
                        .next(format!("remodel scene make {name} --replace"))
                })?;
                let mut warnings = Vec::new();
                let hash = format!("{:016x}", m.bus.state_hash());
                if let Some(want) = crate::scene::read_meta(&dir).and_then(|mt| mt.get("state_hash").cloned()) {
                    if want != hash {
                        warnings.push(format!("scene `{name}` is stale: its state hashes to {hash}, recorded {want}"));
                    }
                }
                let mut base = Vec::new();
                for item in r.input.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    if item == "coin-start" {
                        base.extend(
                            game.controls
                                .coin_start
                                .split(',')
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .filter_map(|i| tour::cut(i, r.frame)),
                        );
                    } else {
                        base.extend(tour::cut(item, r.frame));
                    }
                }
                let script = InputScript::parse(&r.input, &game.controls).map_err(bad)?;
                script.apply(m.frames, &mut m.bus);
                (Start::Scene, m.frames, base, false, warnings)
            }
            f if f.starts_with("tour:") => {
                let (label, frame) = f[5..]
                    .split_once('@')
                    .and_then(|(l, n)| Some((l, num(n)?)))
                    .ok_or_else(|| bad(format!("from `{f}`: want tour:LABEL@FRAME")))?;
                if frame == 0 || frame > OPEN_MAX_FRAMES {
                    return Err(bad(format!("from `{f}`: the frame is 1 to {OPEN_MAX_FRAMES}"))
                        .with_data(json!({"limit": OPEN_MAX_FRAMES})));
                }
                let tours = tour::load(game, self.cfg.tours.as_deref()).map_err(bad)?;
                if tours.iter().all(|t| t.label != label) {
                    let known: Vec<String> = tours.iter().map(|t| t.label.clone()).collect();
                    return Err(Answer::unknown("tour", label, &known).next("remodel tour list"));
                }
                let (script, poked) = tour::build_script(game, &tours, label).map_err(bad)?;
                script.apply(0, &mut m.bus);
                let mut last = 0;
                while m.frames < frame {
                    m.run_slice(u64::MAX / 2)
                        .map_err(|e| internal(format!("the machine stopped at frame {}: {e:?}", m.frames)))?;
                    if m.frames != last {
                        last = m.frames;
                        script.apply(m.frames, &mut m.bus);
                    }
                }
                m.bus.uart_log.clear();
                (Start::Tour(label.to_string(), frame), frame, vec![], poked, vec![])
            }
            other => {
                return Err(bad(format!("from `{other}`: want power-on, scene:NAME or tour:LABEL@FRAME"))
                    .next("remodel scene list"));
            }
        };
        self.next_id += 1;
        let mut s = Session {
            id: id.clone(),
            kind,
            m,
            start,
            base,
            held: vec![],
            log: vec![],
            pokes: vec![],
            marks: vec![],
            camera: String::new(),
            smooth_on: false,
            poked,
            frames_run: 0,
        };
        let hash = s.hash();
        let (frame, ms) = (s.frame(), t0.elapsed().as_secs_f64() * 1e3);
        let rss = rss_bytes().saturating_sub(rss0);
        self.sessions.insert(id.clone(), s);
        let mut a = Answer::done(json!({
            "session": id, "game": game.id, "from": from, "frame": frame, "state_hash": format!("{hash:016x}"),
            "limits": {"marks": self.cfg.marks, "max_step": self.cfg.max_step, "sessions": self.cfg.sessions},
            "ms": ms.round(), "rss_added_mb": (rss / (1 << 20)),
        }));
        for w in warnings.drain(..) {
            a = a.warn(w);
        }
        Ok(a.next(format!("session.step session={id} frames=60")))
    }

    // ---------------------------------------------------------------- input, step

    fn input(&mut self, s: &mut Session, args: &Value) -> Result<Answer, Answer> {
        let items = l_arg(args, "inputs");
        check_held(&items, self.game)?;
        s.held = items;
        if s.frame() > s.start {
            let held = s.held.clone();
            s.apply(self.game, s.frame(), &held).map_err(internal)?;
        }
        let mut a = Answer::done(json!({"session": s.id, "frame": s.frame(), "held": s.held}));
        if s.frame() == s.start {
            a = a.warn(format!(
                "frame {} keeps the inputs it started with; these apply from frame {}",
                s.start,
                s.start + 1
            ));
        }
        Ok(a)
    }

    fn step(&mut self, s: &mut Session, args: &Value) -> Result<Answer, Answer> {
        let game = self.game;
        let frames = n_arg(args, "frames").unwrap_or(0);
        if frames == 0 {
            return Err(bad("step: `frames` is at least 1"));
        }
        if frames > self.cfg.max_step {
            return Err(bad(format!(
                "step: {frames} frames is over the server's limit of {} (--max-step)",
                self.cfg.max_step
            ))
            .with_data(json!({"limit": self.cfg.max_step, "asked": frames}))
            .next(format!("session.step frames={}", self.cfg.max_step)));
        }
        let until = s_arg(args, "until").map(|u| parse_until(u, &self.table)).transpose()?;
        let held = if args.get("inputs").is_some() {
            let items = l_arg(args, "inputs");
            check_held(&items, game)?;
            items
        } else {
            s.held.clone()
        };
        let t0 = Instant::now();
        // The frame about to run takes the step's inputs (not the start's).
        if s.frame() > s.start {
            s.apply(game, s.frame(), &held).map_err(internal)?;
        }
        // The display list of the last frames, for a camera.
        if s.smooth_on && frames > DISPLAY_LEAD {
            s.m.bus.geo.set_smooth(false);
            s.smooth_on = false;
        }
        let (mut ran, mut why) = (0u64, "frames");
        while ran < frames {
            if !s.smooth_on && frames - ran <= DISPLAY_LEAD {
                if crate::GEO_MODE.get().copied().unwrap_or_default() == model2::geo::GeoMode::Chip {
                    s.m.bus.geo.set_mode(model2::geo::GeoMode::Native);
                }
                s.m.bus.geo.set_smooth(true);
                s.smooth_on = true;
            }
            s.run_frame().map_err(internal)?;
            ran += 1;
            let f = s.frame();
            s.apply(game, f, &held).map_err(internal)?;
            if let Some(u) = &until {
                if u.met(&s.m, ran) {
                    why = "until";
                    break;
                }
            }
        }
        s.frames_run += ran;
        let secs = t0.elapsed().as_secs_f64();
        let hash = s.hash();
        Ok(Answer::done(json!({
            "session": s.id, "frame": s.frame(), "state_hash": format!("{hash:016x}"),
            "frames_run": ran, "stopped": why, "held": held,
            "ms": (secs * 1e3).round(), "frames_per_second": (ran as f64 / secs.max(1e-9)).round(),
        })))
    }

    // ---------------------------------------------------------------- marks

    fn mark(&mut self, s: &mut Session, args: &Value) -> Result<Answer, Answer> {
        let name = s_arg(args, "name").unwrap_or("");
        if !name_ok(name) {
            return Err(bad(format!("mark name `{name}`: letters, digits, - and _ only")));
        }
        let replacing = s.marks.iter().position(|k| k.name == name);
        if replacing.is_none() && s.marks.len() >= self.cfg.marks {
            let oldest = s.marks.first().map(|k| k.name.clone()).unwrap_or_default();
            return Err(bad(format!(
                "{} marks are kept, the server's limit (--marks); the oldest is `{oldest}`",
                s.marks.len()
            ))
            .with_data(json!({"limit": self.cfg.marks, "oldest": oldest}))
            .next(format!("session.forget name={oldest}")));
        }
        let t0 = Instant::now();
        let mut k = match replacing {
            Some(i) => s.marks.remove(i),
            None => {
                let roms = self.roms()?;
                let m = match self.spare.pop() {
                    Some(m) => m,
                    None => {
                        let mut m = Box::new(Machine::new(roms));
                        m.configure_like(&mut s.m);
                        m
                    }
                };
                Mark { name: name.to_string(), frame: 0, hash: 0, m, log: vec![], pokes: vec![], held: vec![] }
            }
        };
        s.m.bus.geo.sync();
        k.m.copy_from(&mut s.m, &mut CopyMarks::default());
        k.frame = s.frame();
        k.hash = s.hash();
        k.log = s.log.clone();
        k.pokes = s.pokes.clone();
        k.held = s.held.clone();
        s.marks.push(k);
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        Ok(Answer::done(
            json!({"session": s.id, "mark": name, "frame": s.frame(), "state_hash": format!("{:016x}", s.marks.last().unwrap().hash), "marks": s.marks_json(), "ms": ms.round()}),
        ))
    }

    fn find_mark(s: &Session, args: &Value) -> Result<usize, Answer> {
        let name = s_arg(args, "name").unwrap_or("");
        s.marks.iter().position(|k| k.name == name).ok_or_else(|| {
            let known: Vec<String> = s.marks.iter().map(|k| k.name.clone()).collect();
            Answer::unknown("mark", name, &known)
        })
    }

    fn restore(&mut self, s: &mut Session, args: &Value) -> Result<Answer, Answer> {
        let i = Self::find_mark(s, args)?;
        let t0 = Instant::now();
        let k = &mut s.marks[i];
        k.m.bus.geo.sync();
        s.m.bus.geo.sync();
        s.m.copy_from(&mut k.m, &mut CopyMarks::default());
        s.log = k.log.clone();
        s.pokes = k.pokes.clone();
        s.held = k.held.clone();
        // The display list recorded is not this frame's now.
        s.m.bus.geo.set_smooth(false);
        s.smooth_on = false;
        let hash = s.hash();
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        Ok(Answer::done(
            json!({"session": s.id, "mark": s.marks[i].name, "frame": s.frame(), "state_hash": format!("{hash:016x}"), "held": s.held, "ms": ms.round()}),
        ))
    }

    fn forget(&mut self, s: &mut Session, args: &Value) -> Result<Answer, Answer> {
        let i = Self::find_mark(s, args)?;
        let k = s.marks.remove(i);
        if self.spare.len() < self.cfg.marks {
            self.spare.push(k.m);
        }
        Ok(Answer::done(json!({"session": s.id, "forgot": k.name, "marks": s.marks_json()})))
    }

    // ---------------------------------------------------------------- read, poke

    fn read(&mut self, s: &mut Session, args: &Value) -> Result<Answer, Answer> {
        let names = l_arg(args, "names");
        let mut rows = Vec::new();
        let all: Vec<String>;
        let names: &[String] = if names.is_empty() {
            all = self.table.iter().map(|d| d.name.clone()).collect();
            &all
        } else {
            &names
        };
        for n in names {
            if let Some(d) = self.table.iter().find(|d| d.name == *n) {
                let Some((raw, v)) = read_def(&s.m, d) else {
                    return Err(bad(format!("value {n}: {:#x} is not mapped memory", d.addr)));
                };
                rows.push(
                    json!({"name": d.name, "addr": format!("{:#x}", d.addr), "width": d.width, "raw": raw, "value": v}),
                );
                continue;
            }
            // ADDR[:b|w|l]
            let (a, w) = n.split_once(':').unwrap_or((n, "l"));
            let (Some(addr), Some(width)) =
                (num(a).and_then(|a| u32::try_from(a).ok()), ["b", "w", "l"].iter().position(|x| *x == w))
            else {
                let table: Vec<String> = self.table.iter().map(|d| d.name.clone()).collect();
                return Err(Answer::unknown("value", n, &table).next("remodel ram values"));
            };
            let width = 1usize << width;
            let Some(raw) = s.m.bus.peek(addr, width) else {
                return Err(bad(format!("{n}: {addr:#x} is not mapped memory")));
            };
            rows.push(json!({"name": null, "addr": format!("{addr:#x}"), "width": width, "raw": raw, "value": raw}));
        }
        let mut a = Answer::done(json!({"session": s.id, "frame": s.frame(), "values": rows}));
        if self.table.is_empty() {
            a = a.warn(format!("{} has no named values yet: raw ADDR[:b|w|l] reads work", self.game.id));
        }
        Ok(a)
    }

    fn poke(&mut self, s: &mut Session, args: &Value) -> Result<Answer, Answer> {
        let game = self.game;
        let what = s_arg(args, "what").unwrap_or("");
        let frames = n_arg(args, "frames").unwrap_or(1);
        if frames == 0 || frames > self.cfg.max_step {
            return Err(bad(format!("poke: frames is 1 to {} (--max-step)", self.cfg.max_step))
                .with_data(json!({"limit": self.cfg.max_step})));
        }
        let table = tour::poke_table(romset::games::pokes(game)).map_err(|e| internal(format!("{} {e}", game.id)))?;
        let term = tour::resolve_pokes(vec![format!("poke={what}@0")], &table)
            .map_err(|e| bad(e).next("remodel ram pokes"))?;
        let term = term[0].strip_suffix("@0").unwrap_or(&term[0]).to_string();
        InputScript::parse(&format!("{term}@0"), &game.controls).map_err(|e| bad(format!("poke `{what}`: {e}")))?;
        let from = s.frame().max(s.start + 1);
        let to = from + frames - 1;
        s.pokes.push(PokeLog { term: term.clone(), from, to });
        s.poked = true;
        if s.frame() > s.start {
            let held = s.held.clone();
            s.apply(game, s.frame(), &held).map_err(internal)?;
        }
        Ok(Answer::done(json!({"session": s.id, "poke": term, "from": from, "to": to, "frame": s.frame()}))
            .warn("a poke is a cheat: the session's exported tour is marked as one"))
    }

    // ---------------------------------------------------------------- camera, render

    fn camera(&mut self, s: &mut Session, args: &Value) -> Result<Answer, Answer> {
        if let Some(c) = s_arg(args, "camera") {
            let c = if c == "reset" { "" } else { c };
            if !c.is_empty() {
                model2::photo::Spec::parse(c, 1.0).map_err(|e| bad(format!("camera: {e}")))?;
            }
            s.camera = c.to_string();
        }
        s.m.bus.geo.sync();
        let display = s.m.bus.geo.engine().smooth_frame().is_some();
        Ok(Answer::done(
            json!({"session": s.id, "camera": (!s.camera.is_empty()).then_some(&s.camera), "display_list": display}),
        ))
    }

    fn look(&mut self, spec: &str) -> Result<Option<Arc<model2::look::Look>>, Answer> {
        if spec == "off" || spec == "none" {
            return Ok(None);
        }
        if let Some(l) = self.looks.get(spec) {
            return Ok(Some(l.clone()));
        }
        let l = if spec == "comic" {
            model2::look::Look::comic()
        } else {
            match model2::look::Look::load(Path::new(spec)) {
                Ok(Some(l)) => l,
                Ok(None) => return Err(bad(format!("look: no look file at {spec}"))),
                Err(e) => return Err(bad(format!("look: {e}"))),
            }
        };
        let l = Arc::new(l);
        self.looks.insert(spec.to_string(), l.clone());
        Ok(Some(l))
    }

    /// The draw options the arguments give (the same words as `scene render`).
    fn draw_opts(&mut self, args: &Value, camera: Option<String>) -> Result<crate::scene::DrawOpts, Answer> {
        let against = match s_arg(args, "against") {
            None => false,
            Some("original") => true,
            Some(o) => return Err(Answer::unknown("against target", o, &["original".to_string()])),
        };
        let crop = match s_arg(args, "crop") {
            None => None,
            Some(c) => {
                let v: Vec<i64> = c
                    .split(',')
                    .map(|p| p.trim().parse::<i64>())
                    .collect::<Result<_, _>>()
                    .map_err(|_| bad(format!("crop `{c}`: want X,Y,W,H in game pixels (496x384)")))?;
                match v[..] {
                    [x, y, w, h] if w > 0 && h > 0 => Some([x, y, w, h]),
                    _ => return Err(bad(format!("crop `{c}`: want X,Y,W,H in game pixels (496x384)"))),
                }
            }
        };
        let look = match s_arg(args, "look") {
            Some(l) => self.look(l)?,
            None => None,
        };
        Ok(crate::scene::DrawOpts {
            pack: l_arg(args, "pack").into_iter().map(PathBuf::from).collect(),
            crop,
            against,
            view: crate::scene::parse_view(s_arg(args, "view"))?,
            camera,
            look,
        })
    }

    /// Draw with the GPU kept warm, holding the render lock only while it
    /// draws. Returns the result, the GPU start in ms and the time waiting
    /// for the lock.
    fn drawing<T>(&mut self, f: impl FnOnce(&mut gpu::Renderer) -> Result<T, Answer>) -> Result<(T, f64, f64), Answer> {
        let t = Instant::now();
        let _lock = RenderLock::take()?;
        let lock_ms = t.elapsed().as_secs_f64() * 1e3;
        let t = Instant::now();
        if self.gpu.is_none() {
            self.gpu = Some(gpu::Renderer::new().map_err(|e| internal(format!("no GPU: {e}")))?);
        }
        let gpu_ms = t.elapsed().as_secs_f64() * 1e3;
        let r = f(self.gpu.as_mut().unwrap())?;
        Ok((r, gpu_ms, lock_ms))
    }

    fn render(&mut self, s: &mut Session, args: &Value) -> Result<Answer, Answer> {
        let game = self.game;
        self.roms()?;
        let t0 = Instant::now();
        let camera =
            s_arg(args, "camera").map(str::to_string).or_else(|| (!s.camera.is_empty()).then(|| s.camera.clone()));
        let camera = camera.filter(|c| !c.is_empty() && c != "reset");
        let name = match s_arg(args, "name") {
            Some(n) if name_ok(n) => n.to_string(),
            Some(n) => return Err(bad(format!("name `{n}`: letters, digits, - and _ only"))),
            None => format!("{}-f{}", s.id, s.frame()),
        };
        let opts = self.draw_opts(args, camera.clone())?;
        // The frame as the machine holds it (and the display list, for a camera).
        s.m.bus.geo.sync();
        let display = if camera.is_some() { s.m.bus.geo.engine().smooth_frame() } else { None };
        if camera.is_some() && display.is_none() {
            return Err(Answer::fail(
                Code::Missing,
                "the machine holds no display list for this frame: step at least one frame (after open or restore), and the game's geometrizer needs a native implementation",
            )
            .next(format!("session.step session={} frames=1", s.id)));
        }
        let frame = model2::capture::Frame::grab(&mut s.m.bus);
        let rom = s.m.bus.texture_rom.clone();
        let (d, gpu_ms, lock_ms) =
            self.drawing(|g| crate::scene::draw_frame(game, frame, &rom, display.as_deref(), &opts, g))?;
        let t_write = Instant::now();
        let files = crate::scene::write_drawn(&self.cfg.out.join(game.id), &name, &d).map_err(internal)?;
        let write_ms = t_write.elapsed().as_secs_f64() * 1e3;
        let mut a = Answer::done(json!({
            "session": s.id, "frame": s.frame(), "picture": files["picture"], "original": files.get("original"), "sheet": files.get("sheet"),
            "width": d.picture.width, "height": d.picture.height, "polygons": d.polygons,
            "changed_share": d.changed_share.map(|c| (c * 1e6).round() / 1e6),
            "drawn_share": d.drawn_share.map(|c| (c * 1e6).round() / 1e6),
            "camera": camera, "crop": opts.crop,
            "ms": (t0.elapsed().as_secs_f64() * 1e3).round(), "ms_draw": d.millis.round(), "ms_gpu_init": gpu_ms.round(),
            "ms_lock": lock_ms.round(), "ms_write": write_ms.round(),
        }));
        for w in d.warnings {
            a = a.warn(w);
        }
        Ok(a)
    }

    fn scene_render(&mut self, args: &Value) -> Result<Answer, Answer> {
        let game = self.game;
        let rs = crate::scene::recipes(game, None).map_err(bad)?;
        let opts = self.draw_opts(args, s_arg(args, "camera").map(str::to_string))?;
        let req = crate::scene::SceneRender {
            name: s_arg(args, "scene").unwrap_or("").to_string(),
            against: opts.against,
            crop: opts.crop,
            view: opts.view,
            camera: opts.camera.clone(),
            packs: opts.pack.clone(),
            look: opts.look.clone(),
            overlay: false,
        };
        let out = self.cfg.out.join(game.id);
        let mut drawer = |dir: &Path, o: &crate::scene::DrawOpts| -> Result<(crate::scene::Drawn, f64), Answer> {
            let (d, gpu_ms, _) = self.drawing(|g| crate::scene::draw(game, dir, o, g))?;
            Ok((d, gpu_ms))
        };
        crate::scene::render_scene(game, &rs, &req, &out, &mut drawer).map(|(a, _)| a)
    }

    // ---------------------------------------------------------------- export, close

    fn export(&mut self, s: &mut Session, args: &Value) -> Result<Answer, Answer> {
        let game = self.game;
        let label = s_arg(args, "label").map(str::to_string).unwrap_or_else(|| format!("session-{}", s.id));
        if !name_ok(&label) {
            return Err(bad(format!("label `{label}`: letters, digits, - and _ only")));
        }
        if s.frame() == 0 {
            return Err(bad("nothing to export: the session is at frame 0").next("session.step"));
        }
        let (terms, guns) = s.terms(game);
        let mut all: Vec<String> = Vec::new();
        match &s.kind {
            Start::PowerOn => {}
            Start::Scene => all.extend(s.base.iter().cloned()),
            Start::Tour(label, frame) => all.push(format!("from={label}@{frame}")),
        }
        all.extend(terms);
        let line = format!("{label}|{}f|{}", s.frame(), all.join(","));
        // The line must be one a tour file takes.
        let check = tour::parse(&line, "export");
        if let (Start::Tour(..), Err(_)) = (&s.kind, &check) {
            // A branch needs its parent in the file: parsed with it below.
        } else if let Err(e) = check {
            return Err(internal(format!("the exported line does not parse: {e}")));
        }
        let hash = s.hash();
        let mut file = None;
        if let Some(name) = s_arg(args, "file") {
            if !name_ok(name) {
                return Err(bad(format!("file `{name}`: a plain name (letters, digits, - and _)")));
            }
            let dir = self.cfg.out.join(game.id);
            std::fs::create_dir_all(&dir).map_err(|e| internal(format!("{}: {e}", dir.display())))?;
            let path = dir.join(format!("{name}.tours"));
            // A branch from a tour of the project's file needs that file's lines too.
            let mut text = match &self.cfg.tours {
                Some(p) => std::fs::read_to_string(p).unwrap_or_default(),
                None => String::new(),
            };
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&line);
            text.push('\n');
            std::fs::write(&path, text).map_err(|e| internal(format!("{}: {e}", path.display())))?;
            file = Some(path.display().to_string());
        }
        let mut a = Answer::done(json!({
            "session": s.id, "label": label, "line": line, "file": file, "frame": s.frame(),
            "state_hash": format!("{hash:016x}"), "poked": s.poked,
        }));
        if guns {
            a = a.warn("a light-gun game: where the gun aimed is not logged, so a replay is not exact if it aimed anywhere but the middle");
        }
        if s.poked {
            a = a.warn("the log has pokes (or branches from a tour that pokes): the tour is a cheat run, as the tour format marks them");
        }
        Ok(a.next(match &file {
            Some(f) => format!("remodel tour run {label} --tours {f}"),
            None => format!("remodel tour run {label} --tours FILE   (FILE holds the line)"),
        }))
    }

    fn close(&mut self, args: &Value) -> Result<Answer, Answer> {
        let r = self.with(args, |srv, s, _| {
            let n = s.marks.len();
            for k in s.marks.drain(..) {
                if srv.spare.len() < srv.cfg.marks {
                    srv.spare.push(k.m);
                }
            }
            Ok(Answer::done(json!({"session": s.id, "frame": s.frame(), "frames_run": s.frames_run, "marks_freed": n})))
        })?;
        if let Some(id) = r.data["session"].as_str() {
            self.sessions.remove(id);
        }
        if self.sessions.is_empty() {
            // Nothing is kept but the GPU and the pack caches.
            self.spare.clear();
        }
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_complete_and_unique() {
        let mut names: Vec<&str> = VERBS.iter().map(|v| v.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), VERBS.len());
        assert_eq!(VERBS.len(), 13);
        for v in VERBS {
            let mut a: Vec<&str> = v.args.iter().map(|a| a.name).collect();
            a.sort();
            a.dedup();
            assert_eq!(a.len(), v.args.len(), "{} has an argument twice", v.name);
        }
        assert_eq!(tools().as_array().unwrap().len(), VERBS.len());
    }

    #[test]
    fn pairs_become_typed_arguments() {
        let v = find_verb("session.step").unwrap();
        let a = args_from_pairs(
            v,
            &["frames=30".into(), "inputs=start".into(), "inputs=steer=0x80".into(), "until=value speed gt 2".into()],
        )
        .unwrap();
        assert_eq!(a, json!({"frames": 30, "inputs": ["start", "steer=0x80"], "until": "value speed gt 2"}));
        assert!(validate(v, &a).is_ok());
        let e = args_from_pairs(v, &["frame=3".into()]).unwrap_err();
        assert!(e.error.as_ref().unwrap().contains("did you mean: frames"));
        assert!(args_from_pairs(v, &["frames=x".into()]).is_err());
        assert!(args_from_pairs(v, &["frames".into()]).is_err());
    }

    #[test]
    fn arguments_are_checked_by_the_table() {
        let v = find_verb("session.step").unwrap();
        assert!(validate(v, &json!({})).unwrap_err().error.as_ref().unwrap().contains("`frames` is needed"));
        assert!(validate(v, &json!({"frames": "ten"})).unwrap_err().error.as_ref().unwrap().contains("a whole number"));
        assert!(validate(v, &json!({"frames": 1, "inputs": [1]})).is_err());
        assert!(validate(v, &json!({"frames": 1, "bogus": 1}))
            .unwrap_err()
            .error
            .as_ref()
            .unwrap()
            .contains("unknown argument"));
        assert!(validate(v, &json!([1])).is_err());
        assert!(validate(v, &json!({"frames": 1})).is_ok());
        assert_eq!(find_verb("session.stp").unwrap_err().code, Code::BadRequest);
    }

    #[test]
    fn until_words_are_strict() {
        let t = tour::value_table("speed|0x582f90:l:float|a note\nclock|0x5830b0:w|a note\n").unwrap();
        assert!(matches!(parse_until("value speed gt 2.5", &t), Ok(Until::Value { op: "gt", .. })));
        assert!(matches!(parse_until("frames 0x10", &t), Ok(Until::Frames(16))));
        assert!(parse_until("value speed bigger 1", &t).is_err());
        assert!(parse_until("value spede gt 1", &t).unwrap_err().error.as_ref().unwrap().contains("speed"));
        assert!(parse_until("when speed", &t).is_err());
    }

    #[test]
    fn held_inputs_are_names_not_scripts() {
        let g = romset::games::find("topskatr").unwrap();
        assert!(check_held(&["start".into()], g).is_ok());
        assert!(check_held(&[], g).is_ok());
        assert!(check_held(&["start@10".into()], g).is_err());
        assert!(check_held(&["coin-start".into()], g).is_err());
        assert!(check_held(&["poke=0x10:1".into()], g).is_err());
        assert!(check_held(&["nonesuch".into()], g).is_err());
    }

    // ------------------------------------------------------------ with the ROM

    /// The script's held inputs at `frame` (names, as `session.input`
    /// takes them); only plain `name@a-b` terms.
    fn held_at(items: &[String], frame: u64) -> Vec<String> {
        items
            .iter()
            .filter_map(|i| {
                let (what, r) = i.rsplit_once('@')?;
                let (a, b) = r.split_once('-').unwrap_or((r, r));
                (a.parse::<u64>().ok()? <= frame && frame <= b.parse::<u64>().ok()?).then(|| what.to_string())
            })
            .collect()
    }

    /// Drive a session along a script to `to`: before each stretch of
    /// frames that hold the same inputs, say what is held, then step it.
    fn drive(srv: &mut Server, items: &[String], from: u64, to: u64) {
        let mut c = from;
        while c < to {
            let h = held_at(items, c);
            let mut n = 1;
            while c + n < to && held_at(items, c + n) == h {
                n += 1;
            }
            let a = srv.call("session.input", &json!({"inputs": h}));
            assert!(a.ok(), "{:?}", a.error);
            let a = srv.call("session.step", &json!({"frames": n}));
            assert!(a.ok(), "{:?}", a.error);
            c += n;
        }
        // The frame reached takes its own inputs.
        srv.call("session.input", &json!({"inputs": held_at(items, to)}));
    }

    fn hash_of(a: &Answer) -> String {
        a.data["state_hash"].as_str().unwrap().to_string()
    }

    /// The whole of a session against the game (skipped without the ROM):
    /// power-on and a scene recipe's inputs end on the scene's hash;
    /// mark, step, restore gives the mark's hash; the exported line, run as
    /// a tour, gives the session's; the clock reads as the game's at a
    /// known frame; the limits refuse; render writes a PNG.
    #[test]
    fn a_session_replays_a_scene_and_a_tour_and_keeps_its_marks() {
        let game = romset::games::find("topskatr").unwrap();
        let rom = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rom");
        let Ok(roms) = RomSet::load(game, &rom, false) else { return };
        let out =
            Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../out/scratch/serve-test-{}", std::process::id()));
        std::env::set_var("REMODEL_LOCK_DIR", out.join("locks"));
        let cfg = Config { marks: 2, max_step: 3600, sessions: 2, tours: None, out: out.clone() };
        let mut srv = Server::new(game, Some(&roms), cfg);
        let recipes = crate::scene::recipes(game, None).unwrap();
        let recipe = |n: &str| recipes.iter().find(|r| r.name == n).unwrap().clone();
        let (up, go) = (recipe("coin-up"), recipe("race-course"));
        let script: Vec<String> = game.controls.coin_start.split(',').map(str::to_string).collect();

        // Open at power-on, step the recipe's inputs to its frame: the scene's hash.
        let a = srv.call("session.open", &json!({}));
        assert!(a.ok(), "{:?}", a.error);
        let first = a.data["session"].as_str().unwrap().to_string();
        assert_eq!(a.data["frame"], 0);
        drive(&mut srv, &script, 0, up.frame);
        let want = format!("{:016x}", crate::scene::run_to(&roms, &up, false).unwrap().hash);
        let at_up = srv.call("session.read", &json!({"names": ["0x5830b0:w"]}));
        assert!(at_up.ok(), "{:?}", at_up.error);
        assert_eq!(at_up.data["frame"], up.frame);
        let h = srv.sessions.get_mut(&first).unwrap().hash();
        assert_eq!(format!("{h:016x}"), want, "the session's hash at the scene's frame is the scene's");

        // Mark, step, restore: the mark's hash, and the log cut back.
        let marked = srv.call("session.mark", &json!({"name": "a"}));
        assert!(marked.ok(), "{:?}", marked.error);
        assert_eq!(hash_of(&marked), want);
        let stepped = srv.call("session.step", &json!({"frames": 40, "inputs": ["start"]}));
        assert!(stepped.ok());
        assert_ne!(hash_of(&stepped), want);
        let back = srv.call("session.restore", &json!({"name": "a"}));
        assert!(back.ok(), "{:?}", back.error);
        assert_eq!(hash_of(&back), want, "restore gives the mark's hash");
        assert_eq!(back.data["frame"], up.frame);
        // And stepping on from the restore repeats what it did before.
        let again = srv.call("session.step", &json!({"frames": 40, "inputs": ["start"]}));
        assert_eq!(hash_of(&again), hash_of(&stepped), "the same inputs from the same mark give the same state");

        // The marks limit names the oldest; forget frees a place.
        assert!(srv.call("session.mark", &json!({"name": "b"})).ok());
        let full = srv.call("session.mark", &json!({"name": "c"}));
        assert_eq!(full.code, Code::BadRequest);
        assert_eq!(full.data["oldest"], "a");
        assert!(srv.call("session.forget", &json!({"name": "a"})).ok());
        assert!(srv.call("session.mark", &json!({"name": "c"})).ok());
        assert!(srv.call("session.restore", &json!({"name": "zz"})).message().contains("unknown mark"));
        // The step limit.
        let a = srv.call("session.step", &json!({"frames": 3601}));
        assert_eq!(a.code, Code::BadRequest);
        assert_eq!(a.data["limit"], 3600);
        // The sessions limit.
        assert!(srv.call("session.open", &json!({})).ok());
        let a = srv.call("session.open", &json!({}));
        assert_eq!(a.code, Code::BadRequest);
        assert!(a.message().contains("limit"));
        let a = srv.call("session.step", &json!({"frames": 1}));
        assert!(a.message().contains("several are open"), "{}", a.message());
        assert!(srv.call("session.close", &json!({"session": "s2"})).ok());

        // Export and replay: the line, run as a tour, ends on the session's hash.
        let exp = srv.call("session.export", &json!({"label": "walk", "file": "walk"}));
        assert!(exp.ok(), "{:?}", exp.error);
        let line = exp.data["line"].as_str().unwrap().to_string();
        assert!(line.starts_with(&format!("walk|{}f|", srv.sessions[&first].frame())), "{line}");
        let tours = tour::parse(&line, "export").unwrap();
        let (tour_script, poked) = tour::build_script(game, &tours, "walk").unwrap();
        assert!(!poked);
        let dir = out.join("tour");
        std::fs::create_dir_all(dir.join("states")).unwrap();
        let (row, _) = tour::run_one(&roms, &tours[0], &tour_script, &dir, false).unwrap();
        assert_eq!(row["state_hash"].as_str().unwrap(), exp.data["state_hash"].as_str().unwrap(), "{line}");
        assert!(std::fs::read_to_string(exp.data["file"].as_str().unwrap()).unwrap().contains(&line));

        // A poke is logged, and the replay still agrees.
        let p = srv.call("session.poke", &json!({"what": "0x5830b0:0x0e10:w", "frames": 3}));
        assert!(p.ok(), "{:?}", p.error);
        let st = srv.call("session.step", &json!({"frames": 5}));
        let exp = srv.call("session.export", &json!({"label": "cheat"}));
        assert_eq!(exp.data["poked"], true);
        let line = exp.data["line"].as_str().unwrap();
        assert!(line.contains("poke=0x5830b0:0x0e10:w@"), "{line}");
        let tours = tour::parse(line, "export").unwrap();
        let (tour_script, poked) = tour::build_script(game, &tours, "cheat").unwrap();
        assert!(poked);
        let (row, _) = tour::run_one(&roms, &tours[0], &tour_script, &dir, true).unwrap();
        assert_eq!(row["state_hash"].as_str().unwrap(), hash_of(&st), "{line}");
        let _ = std::fs::remove_dir_all(&dir);

        // A start that is not a tour of the game's is refused.
        assert!(srv.call("session.close", &json!({})).ok());
        let a = srv.call("session.open", &json!({"from": "tour:nonesuch@100"}));
        assert_eq!(a.code, Code::BadRequest);
        assert!(a.message().contains("unknown tour"));

        // The clock at a known frame: the Novice start's LIMIT of 58 s.
        assert!(srv.call("session.open", &json!({})).ok());
        drive(&mut srv, &script, 0, go.frame);
        let a = srv.call("session.read", &json!({"names": ["clock", "course"]}));
        assert!(a.ok(), "{:?}", a.error);
        let clock = a.data["values"][0]["value"].as_u64().unwrap();
        assert_eq!(clock / 60, 58, "the LIMIT shows 58 at frame {}", go.frame);
        // until: stop when the clock falls under a line.
        let a = srv.call("session.step", &json!({"frames": 100, "until": format!("value clock lt {}", clock - 10)}));
        assert_eq!(a.data["stopped"], "until", "{a:?}");
        assert_eq!(a.data["frames_run"], 11);

        // Render writes a PNG (where there is a GPU).
        if gpu::Renderer::new().is_ok() {
            let a = srv.call("session.render", &json!({"view": "540,16:9", "name": "t"}));
            assert!(a.ok(), "{:?}", a.error);
            let png = std::fs::read(a.data["picture"].as_str().unwrap()).unwrap();
            assert_eq!(&png[1..4], b"PNG");
            assert_eq!(a.data["frame"], go.frame + 11);
            // A camera, after a step (the display list is recorded in a step's last frames).
            assert!(srv.call("session.step", &json!({"frames": 4})).ok());
            let a = srv.call("session.camera", &json!({"camera": "orbit=20,tilt=-5"}));
            assert!(a.ok() && a.data["display_list"] == true, "{a:?}");
            let a = srv.call("session.render", &json!({"view": "540,16:9", "name": "t-cam", "look": "comic"}));
            assert!(a.ok(), "{:?}", a.error);
            assert_eq!(a.data["camera"], "orbit=20,tilt=-5");
            // A frame restored has no display list until a step.
            assert!(srv.call("session.mark", &json!({"name": "m"})).ok());
            assert!(srv.call("session.restore", &json!({"name": "m"})).ok());
            let a = srv.call("session.render", &json!({"name": "t-cam2"}));
            assert_eq!(a.code, Code::Missing, "{a:?}");
            assert!(srv.call("session.camera", &json!({"camera": "reset"})).ok());
            assert!(srv.call("session.render", &json!({"view": "native", "name": "t-plain"})).ok());
            // The camera never entered the log: the same line either way.
            let l1 = srv.call("session.export", &json!({})).data["line"].clone();
            srv.call("session.camera", &json!({"camera": "orbit=20"}));
            assert_eq!(srv.call("session.export", &json!({})).data["line"], l1);
        }
        let _ = std::fs::remove_dir_all(&out);
    }
}
