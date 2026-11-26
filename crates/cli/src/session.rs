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
use crate::spine;
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
    "photo mode's camera words for this picture: orbit=30,tilt=-8,z=1 (back, orbit, tilt, yaw, pitch, roll, x, y, z, fov, near, hud), or preset=chase|side|top first (a third-person view relative to the game's camera)",
);
const NAME: Arg = arg("name", Ty::Str, false, "file name stem for the picture (letters, digits, - and _)");

/// Every verb the server has: the MCP tools, and what `remodel call` takes.
pub static VERBS: &[Verb] = &[
    Verb {
        name: "session.open",
        about: "Open a session: a machine of its own that runs only when stepped. from: power-on, scene:NAME (a made scene's state) or tour:LABEL@FRAME (as a branch tour does).",
        args: &[
            arg("from", Ty::Str, false, "power-on (default), scene:NAME or tour:LABEL@FRAME"),
            arg("overlay", Ty::List, false, "with tour:LABEL@FRAME: inputs added on top of the recording on the way there, as tour terms with frames (jumpfront@2560-3000); the session then exports as a branch that carries them"),
            arg("index_every", Ty::Int, false, "when this open makes the recording's spine: a row of values every this many frames (default: the server's --index-every, 30)"),
            arg("checkpoint_every", Ty::Int, false, "when this open makes the recording's spine: a checkpoint every this many frames (default: the server's --checkpoint-every, 600)"),
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
        about: "Run frames, and only these. Optionally with moves (`move`), with inputs for just those frames, following a recording's inputs (`follow`, with `inputs` as an overlay held on top), and stopping early at a condition. Writes watched by session.watch during the step are reported in `watch`. Never more than the server's --max-step frames.",
        args: &[
            SESSION,
            arg("frames", Ty::Int, false, "how many frames to run (at most --max-step); needed unless `move` is given (with `move` and `until`, the most frames the repeats run: default --max-step)"),
            arg("inputs", Ty::List, false, "inputs held for just this step (as session.input); with follow, an overlay added on top of the recording's"),
            arg("move", Ty::List, false, "moves from the game's move table (session.moves), run in sequence in place of held inputs: each move's inputs for its frames. Without `until` the sequence runs once; with `until` it repeats until the condition holds or `frames` (default --max-step) are run. `wait-N` is N frames of no input (`wait-20`), to space moves out. Not with `follow` or `inputs`"),
            arg("follow", Ty::Str, false, "tour:LABEL (or LABEL): apply that tour's inputs, frame by frame, for the frames stepped (the held inputs are not used; `inputs` is added on top). A tour with pokes is followed with its pokes (a cheat run)"),
            arg("until", Ty::Str, false, "stop early: `value NAME OP V` (OP gt ge lt le eq ne), `distance NX,NY[,NZ] to X,Y[,Z] OP D` (names from the game's value table), `changes NAME` (differs from the step's start), `screen NAME|N` (the table's named screens), or `frames N`"),
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
        about: "Read named values from the game's value table, or raw memory: ADDR[:b|w|l][:signed|:float] (little-endian; default l), or ADDR:LEN (or ADDR..END) for a range, as a hex dump (with :b|w|l and :signed|:float a list of values). No names: every named value. A poke is applied at the start of the frame it covers, so right after a step a poked value reads as the poke wrote it, before the game has run that frame (and the game may rewrite it as it runs): `when=before_poke` reads the poked bytes as the previous frame left them.",
        args: &[
            SESSION,
            arg("names", Ty::List, false, "value names, or ADDR[:b|w|l][:signed|:float], or ADDR:LEN / ADDR..END"),
            arg("format", Ty::Str, false, "json (default: a row a value), compact (one object, name: value) or tsv (a line a value, in `text`)"),
            arg("when", Ty::Str, false, "after (default: as the machine is, pokes of this frame applied) or before_poke (bytes a poke of this frame covers read as the previous frame left them)"),
        ],
    },
    Verb {
        name: "session.watch",
        about: "Watch writes to memory during steps: WHAT is value names or ADDR[:LEN] (LEN bytes, or b|w|l; default 4). The next steps report every write that touches one (up to `limit` a step) with the frame being run, the i960 instruction that wrote (`ip`), the size and the value. A watch never changes the emulation (state hashes are the same with and without) but runs the CPU slower while it is set, and a session's own pokes are not shown. No `what`: the current watch; an empty list clears it.",
        args: &[
            SESSION,
            arg("what", Ty::List, false, "value names, or ADDR[:LEN]; an empty list clears the watch"),
            arg("limit", Ty::Int, false, "most writes kept in a step (default 200, at most 5000)"),
        ],
    },
    Verb {
        name: "session.disasm",
        about: "Disassemble the main CPU's (i960) code at an address, as the machine holds it now (ROM or RAM). The listing is an answer, not a file: do not commit it.",
        args: &[
            SESSION,
            arg("addr", Ty::Str, true, "the address (hex with 0x, or decimal)"),
            arg("count", Ty::Int, false, "instructions (default 16, at most 256)"),
            arg("back", Ty::Int, false, "start this many bytes before the address (rounded to a word; the listing marks the address with =>; going back can start inside an instruction)"),
        ],
    },
    Verb {
        name: "session.moves",
        about: "The game's moves (learned from a person's recordings, `remodel moves learn`), each with its inputs and frames and the measured effect (median and range of speed, heading, airtime, height, score over a window) under the conditions asked for: `current` (default; the session's airborne, speed band and screen now), `all` (each move's overall row) or explicit ones like `airborne=0,speed=mid`. Play one with session.step move=NAME.",
        args: &[
            SESSION,
            arg("where", Ty::Str, false, "current (default), all, or conditions NAME=VALUE,... (airborne=0|1, speed=low|mid|high, screen=NAME)"),
            arg("sort", Ty::Str, false, "an effect to order by, lowest first (dspeed, dyaw, dair, dheight, dscore), `-` before it for highest first; default most samples first"),
            arg("limit", Ty::Int, false, "most moves listed (default 40)"),
        ],
    },
    Verb { name: "session.list", about: "The open sessions: id, where each started, its frame, marks, held inputs and watch.", args: &[] },
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
        about: "Set (or with no argument, read) photo mode's camera for this session's pictures. It is render state only and never enters the log. reset or an empty string returns to the game's own. `preset=chase|side|top` is a third-person view relative to the game's camera (chase: back and a little up; side: a quarter turn; top: up and looking down), with other words after it (`preset=side,fov=1.3`); the eye can land inside walls or a ceiling of an enclosed course, and the game draws little of what is behind its own camera, so a view from further back shows a bare floor there.",
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
        name: "spine.index",
        about: "Replay a recording (a tour from power-on) once and keep, under out/GAME/spines/LABEL/, a checkpoint every N frames (so session.open at tour:LABEL@F loads the nearest and steps the rest) and the game's named values every K frames (for spine.where). Replaces what was there.",
        args: &[
            arg("spine", Ty::Str, true, "the tour's label (a tour with a length in frames)"),
            arg("every", Ty::Int, false, "a checkpoint every this many frames (default 600)"),
            arg("index_every", Ty::Int, false, "a row of values every this many frames (default 30)"),
        ],
    },
    Verb {
        name: "spine.where",
        about: "Frames along a recording by what the game was doing, from the spine's index (made by spine.index, or on the first call): with near (NAME=VALUE points, e.g. pos-x=12,pos-z=-4000) the frames nearest it; with only where (NAME OP VALUE, e.g. screen=7) the stretches that match. Each result has an `open` for session.open.",
        args: &[
            arg("spine", Ty::Str, true, "the tour's label"),
            arg("near", Ty::List, false, "points to be near: NAME=VALUE, with names from the game's value table"),
            arg("where", Ty::List, false, "conditions that must hold: NAME OP VALUE (OP = != > >= < <=), e.g. screen=7, speed>3"),
            arg("limit", Ty::Int, false, "most results (default 5)"),
            arg("gap", Ty::Int, false, "with near: results at least this many frames apart (default 90)"),
        ],
    },
    Verb {
        name: "session.probe",
        about: "What draws each game pixel of a rectangle in the session's current frame, top to bottom: 2D tile layers (plane, map cell, tile_KEY, the pack's region), the 3D polygon (texture key, palette variant, polygon, object) and which file of the packs given supplies each. No GPU, no picture.",
        args: &[
            SESSION,
            arg("at", Ty::Str, true, "X,Y or X,Y,W,H in game pixels (496x384)"),
            PACK,
        ],
    },
    Verb {
        name: "scene.probe",
        about: "session.probe for a made scene's capture, with no emulator: what draws each game pixel of a rectangle and which pack file supplies it.",
        args: &[arg("scene", Ty::Str, true, "the scene's name"), arg("at", Ty::Str, true, "X,Y or X,Y,W,H in game pixels (496x384)"), PACK],
    },
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

/// `remodel call VERB --help`: the verb's text and each argument, from the
/// table the tool listing and the checks read.
pub fn help(v: &Verb) -> String {
    let mut s = format!("{}\n  {}\n\n  {}\n", v.name, v.about, usage(v));
    if v.args.is_empty() {
        s.push_str("\n  no arguments\n");
    } else {
        s.push_str("\narguments (name=value; a list repeats the name):\n");
        for a in v.args {
            let ty = match a.ty {
                Ty::Str => "text",
                Ty::Int => "whole number",
                Ty::List => "list",
            };
            s.push_str(&format!(
                "  {:<16} {:<12} {}  {}\n",
                a.name,
                ty,
                if a.required { "required" } else { "optional" },
                a.about
            ));
        }
    }
    s
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

/// Writes `session.watch` keeps in a step unless asked for more.
const DEFAULT_WATCH_LIMIT: usize = 200;

/// A tour's whole input as terms, named pokes resolved.
pub(crate) fn tour_items(game: &Game, tours: &[tour::Tour], label: &str) -> Result<Vec<String>, String> {
    let table = tour::poke_table(romset::games::pokes(game)).map_err(|e| format!("{} {e}", game.id))?;
    tour::resolve_pokes(tour::inputs(tours, label, game.controls.coin_start)?, &table)
}

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
    pre: Vec<(u32, u8)>,
    pre_frame: u64,
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
    /// `session.watch`: what to report writes to during steps.
    watch: Vec<WatchItem>,
    watch_limit: usize,
    /// The bytes a poke covering `pre_frame` overwrote, as that frame's
    /// start found them (`session.read when=before_poke`).
    pre: Vec<(u32, u8)>,
    pre_frame: u64,
    /// What the session was opened from, as asked (`session.list`).
    from: String,
}

/// One watched range: `[start, end)`, and the named value it is, if it is.
#[derive(Clone)]
struct WatchItem {
    what: String,
    start: u32,
    end: u32,
    def: Option<ValueDef>,
}

/// `n` bytes at `addr` as the machine holds them, or (`before`) as the
/// previous frame left them where a poke of this frame has overwritten them.
fn peek_at(s: &Session, addr: u32, width: usize, before: bool) -> Option<u32> {
    let mut raw = s.m.bus.peek(addr, width)?;
    if before && s.pre_frame == s.frame() {
        for i in 0..width {
            if let Some(&(_, b)) = s.pre.iter().find(|x| x.0 == addr.wrapping_add(i as u32)) {
                raw = (raw & !(0xff << (8 * i))) | ((b as u32) << (8 * i));
            }
        }
    }
    Some(raw)
}

/// Whether a poke of the frame the session is on covers any of these bytes.
fn poke_covers(s: &Session, addr: u32, len: usize) -> bool {
    s.pre_frame == s.frame() && s.pre.iter().any(|&(a, _)| a >= addr && (a as u64) < addr as u64 + len as u64)
}

/// A raw read: one value, or a range of bytes (as values, if a width or
/// kind was asked for).
enum RawRead {
    One { addr: u32, width: usize, kind: tour::Kind },
    Range { addr: u32, len: usize, elem: Option<(usize, tour::Kind)> },
}

/// The most bytes one range read gives.
const RANGE_MAX: usize = 4096;

/// `ADDR[:b|w|l][:signed|float]`, `ADDR:float`, `ADDR:LEN[:b|w|l][:signed|float]`
/// or `ADDR..END[...]`. None where the text does not start with an
/// address (it is not a raw read at all).
fn parse_raw(spec: &str) -> Option<Result<RawRead, String>> {
    let mut parts = spec.split(':');
    let head = parts.next()?;
    let (addr, end) = match head.split_once("..") {
        Some((a, e)) => (num(a)?, Some(num(e)?)),
        None => (num(head)?, None),
    };
    let addr = u32::try_from(addr).ok()?;
    let mut len = end.map(|e| e.saturating_sub(addr as u64));
    let (mut width, mut kind) = (None, None);
    for (i, p) in parts.enumerate() {
        match p {
            "b" | "w" | "l" if width.is_none() => {
                width = Some(match p {
                    "b" => 1,
                    "w" => 2,
                    _ => 4,
                })
            }
            "signed" | "float" if kind.is_none() => {
                kind = Some(if p == "signed" { tour::Kind::Signed } else { tour::Kind::Float })
            }
            n if i == 0 && len.is_none() && num(n).is_some() => len = num(n),
            _ => {
                return Some(Err(format!(
                    "`{p}` is not b, w, l, signed, float or a length (ADDR[:LEN][:b|w|l][:signed|float])"
                )))
            }
        }
    }
    if kind == Some(tour::Kind::Float) && width.is_some_and(|w| w != 4) {
        return Some(Err("float is a long: ADDR:l:float, or ADDR:float".into()));
    }
    let width_given = width.is_some();
    let width = width.unwrap_or(4);
    Some(match len {
        None => Ok(RawRead::One { addr, width, kind: kind.unwrap_or(tour::Kind::Unsigned) }),
        Some(l) if l == 0 || l as usize > RANGE_MAX => Err(format!("a range is 1 to {RANGE_MAX} bytes")),
        Some(l) => {
            let l = l as usize;
            let elem = (kind.is_some() || width_given).then_some((width, kind.unwrap_or(tour::Kind::Unsigned)));
            if elem.is_some_and(|(w, _)| !l.is_multiple_of(w)) {
                Err(format!("{l} bytes is not a whole number of {width}-byte values"))
            } else {
                Ok(RawRead::Range { addr, len: l, elem })
            }
        }
    })
}

/// The writes a step's watch saw, as JSON.
fn watch_json(items: &[WatchItem], w: &model2::bus::Watch) -> Value {
    let hits: Vec<Value> = w
        .hits
        .iter()
        .map(|&(_, ip, addr, v, n, frame)| {
            let end = addr.wrapping_add(n as u32);
            let item = items.iter().find(|i| addr < i.end && end > i.start);
            let def = item.and_then(|i| i.def.as_ref());
            let exact = def.filter(|d| d.addr == addr && d.width == n);
            let mut h = json!({
                "frame": frame, "ip": format!("{ip:#x}"), "addr": format!("{addr:#x}"), "size": n,
                "value": exact.map_or_else(|| json!(v), |d| crate::ram::decode(d, v)), "raw": format!("{v:#x}"),
            });
            if let Some(d) = def {
                h["name"] = json!(d.name);
            }
            h
        })
        .collect();
    json!({"writes": hits.len() as u64 + w.dropped, "kept": hits.len(), "dropped": w.dropped, "hits": hits})
}
/// The address and width of a poke term (`poke=ADDR:VALUE[:b|w|l]`).
fn poke_region(term: &str) -> Option<(u32, usize)> {
    let mut it = term.strip_prefix("poke=")?.split(':');
    let addr = u32::try_from(num(it.next()?)?).ok()?;
    it.next()?;
    let width = match it.next() {
        None | Some("b") => 1,
        Some("w") => 2,
        _ => 4,
    };
    Some((addr, width))
}

impl Session {
    fn frame(&self) -> u64 {
        self.m.frames
    }

    /// Set the inputs of `frame` on the bus: `held`, and the pokes that
    /// cover it; note them in the log.
    fn apply(&mut self, game: &Game, frame: u64, held: &[String]) -> Result<(), String> {
        self.apply_with(game, frame, held, None)
    }

    /// Remember what the pokes of `frame` are about to overwrite (the
    /// first time they apply to that frame).
    fn note_pre(&mut self, frame: u64) {
        if self.pre_frame != frame {
            self.pre.clear();
            self.pre_frame = frame;
        }
        let mut add: Vec<(u32, u8)> = Vec::new();
        for p in self.pokes.iter().filter(|p| p.from <= frame && frame <= p.to) {
            if let Some((addr, w)) = poke_region(&p.term) {
                for i in 0..w as u32 {
                    let a = addr.wrapping_add(i);
                    if !self.pre.iter().any(|x| x.0 == a) && !add.iter().any(|x| x.0 == a) {
                        if let Some(b) = self.m.bus.peek(a, 1) {
                            add.push((a, b as u8));
                        }
                    }
                }
            }
        }
        self.pre.extend(add);
    }

    /// [`Session::apply`], with the frame's inputs taken from `script` (a
    /// recording followed, built once for the step, with the held inputs
    /// and the session's pokes in it) when there is one.
    fn apply_with(
        &mut self,
        game: &Game,
        frame: u64,
        held: &[String],
        script: Option<&InputScript>,
    ) -> Result<(), String> {
        self.note_pre(frame);
        match script {
            Some(script) => script.apply(frame, &mut self.m.bus),
            None => {
                let mut items: Vec<String> = held.iter().map(|h| format!("{h}@{frame}")).collect();
                items.extend(
                    self.pokes
                        .iter()
                        .filter(|p| p.from <= frame && frame <= p.to)
                        .map(|p| format!("{}@{frame}", p.term)),
                );
                let script = InputScript::parse(&items.join(","), &game.controls)?;
                script.apply(frame, &mut self.m.bus);
            }
        }
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
    Value {
        def: ValueDef,
        op: &'static str,
        v: f64,
    },
    /// The distance from the named values (2 or 3 of them) to a point.
    Distance {
        defs: Vec<ValueDef>,
        to: Vec<f64>,
        op: &'static str,
        d: f64,
    },
    /// The value's raw bytes differ from what they were at the step's start.
    Changes {
        def: ValueDef,
        start: Option<u32>,
    },
}

const UNTIL_WORDS: &str = "want `value NAME OP V` (OP: gt ge lt le eq ne), `distance NX,NY[,NZ] to X,Y[,Z] OP D`, `changes NAME`, `screen NAME|N` or `frames N`";

fn parse_op(op: &str) -> Option<&'static str> {
    Some(match op {
        "gt" | ">" => "gt",
        "ge" | ">=" => "ge",
        "lt" | "<" => "lt",
        "le" | "<=" => "le",
        "eq" | "==" | "=" => "eq",
        "ne" | "!=" => "ne",
        _ => return None,
    })
}

fn find_def(table: &[ValueDef], name: &str) -> Result<ValueDef, Answer> {
    table.iter().find(|d| d.name == name).cloned().ok_or_else(|| {
        let all: Vec<String> = table.iter().map(|d| d.name.clone()).collect();
        Answer::unknown("value", name, &all).next("remodel ram values")
    })
}

fn float_arg(v: &str) -> Option<f64> {
    match num(v) {
        Some(n) => Some(n as f64),
        None => v.parse::<f64>().ok().filter(|f| f.is_finite()),
    }
}

fn parse_until(text: &str, table: &[ValueDef], states: &[tour::StateDef]) -> Result<Until, Answer> {
    let want = || bad(format!("until `{text}`: {UNTIL_WORDS}"));
    let w: Vec<&str> = text.split_whitespace().collect();
    match w[..] {
        ["frames", n] => Ok(Until::Frames(num(n).ok_or_else(want)?)),
        ["value", name, op, v] => {
            let op = parse_op(op).ok_or_else(want)?;
            let v = float_arg(v).ok_or_else(want)?;
            Ok(Until::Value { def: find_def(table, name)?, op, v })
        }
        ["distance", names, "to", point, op, d] => {
            let names: Vec<&str> = names.split(',').collect();
            let to: Vec<f64> = point.split(',').map(float_arg).collect::<Option<_>>().ok_or_else(want)?;
            if !(2..=3).contains(&names.len()) || names.len() != to.len() {
                return Err(bad(format!(
                    "until `{text}`: a distance wants as many value names as coordinates, two or three"
                )));
            }
            let defs = names.iter().map(|n| find_def(table, n)).collect::<Result<Vec<_>, _>>()?;
            Ok(Until::Distance { defs, to, op: parse_op(op).ok_or_else(want)?, d: float_arg(d).ok_or_else(want)? })
        }
        ["changes", name] => Ok(Until::Changes { def: find_def(table, name)?, start: None }),
        ["screen", name] => {
            let def = find_def(table, "screen").map_err(|_| {
                bad("until screen: the game's value table has no value named `screen`").next("remodel ram values")
            })?;
            let screens: Vec<&tour::StateDef> = states.iter().filter(|s| s.value == "screen").collect();
            let v = match screens.iter().find(|s| s.name == name) {
                Some(s) => s.number as f64,
                None => match float_arg(name) {
                    Some(n) => n,
                    None => {
                        let known: Vec<String> = screens.iter().map(|s| s.name.clone()).collect();
                        let e = if known.is_empty() {
                            bad(format!(
                                "until screen {name}: the value table names no screens; give the number (screen 7)"
                            ))
                        } else {
                            Answer::unknown("screen", name, &known)
                        };
                        return Err(e.next("remodel ram values"));
                    }
                },
            };
            Ok(Until::Value { def, op: "eq", v })
        }
        _ => Err(want()),
    }
}

/// A named value now, as a number (None: unreadable or not finite).
fn read_def(m: &Machine, def: &ValueDef) -> Option<(u32, Value)> {
    let raw = m.bus.peek(def.addr, def.width)?;
    Some((raw, crate::ram::decode(def, raw)))
}

fn compare(op: &str, x: f64, v: f64) -> bool {
    match op {
        "gt" => x > v,
        "ge" => x >= v,
        "lt" => x < v,
        "le" => x <= v,
        "eq" => x == v,
        _ => x != v,
    }
}

impl Until {
    /// Note the machine as the step starts (what `changes` compares with).
    fn arm(&mut self, m: &Machine) {
        if let Until::Changes { def, start } = self {
            *start = m.bus.peek(def.addr, def.width);
        }
    }

    fn distance(defs: &[ValueDef], to: &[f64], m: &Machine) -> Option<f64> {
        let mut sum = 0.0;
        for (d, t) in defs.iter().zip(to) {
            let x = read_def(m, d)?.1.as_f64()?;
            sum += (x - t) * (x - t);
        }
        Some(sum.sqrt())
    }

    fn met(&self, m: &Machine, ran: u64) -> bool {
        match self {
            Until::Frames(n) => ran >= *n,
            Until::Value { def, op, v } => {
                let Some(x) = read_def(m, def).and_then(|(_, val)| val.as_f64()) else { return false };
                compare(op, x, *v)
            }
            Until::Distance { defs, to, op, d } => Self::distance(defs, to, m).is_some_and(|x| compare(op, x, *d)),
            Until::Changes { def, start } => m.bus.peek(def.addr, def.width) != *start,
        }
    }

    /// What the condition reads now, for the step's answer.
    fn reading(&self, m: &Machine) -> Value {
        match self {
            Until::Frames(_) => Value::Null,
            Until::Value { def, .. } | Until::Changes { def, .. } => read_def(m, def).map_or(Value::Null, |(_, v)| v),
            Until::Distance { defs, to, .. } => {
                Self::distance(defs, to, m).map_or(Value::Null, |d| json!((d * 1e4).round() / 1e4))
            }
        }
    }
}

/// A move's term (`curving=0xf0@0-19`, frames counted from the move's first
/// frame) with its frames moved to start at `at`.
fn shift_term(term: &str, at: u64) -> Result<String, String> {
    let (what, _) = term.rsplit_once('@').ok_or_else(|| format!("move term `{term}` has no frames"))?;
    let (a, b) = tour::item_range(term).ok_or_else(|| format!("move term `{term}`: bad frames"))?;
    Ok(if a == b { format!("{what}@{}", at + a) } else { format!("{what}@{}-{}", at + a, at + b) })
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
#[derive(Clone, Debug)]
pub struct Config {
    pub marks: usize,
    pub max_step: u64,
    pub sessions: usize,
    pub tours: Vec<PathBuf>,
    /// Where spines' checkpoints and indexes go (default out/GAME/spines).
    pub spines: Option<PathBuf>,
    /// A checkpoint every this many frames of a spine.
    pub checkpoint_every: u64,
    /// A row of values every this many frames of a spine.
    pub index_every: u64,
    /// Pictures and exported tours go here.
    pub out: PathBuf,
    /// A move table to use in place of the game's own.
    pub moves: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            marks: 16,
            max_step: 3600,
            sessions: 4,
            tours: Vec::new(),
            spines: None,
            checkpoint_every: crate::spine::CHECKPOINT_EVERY,
            index_every: crate::spine::INDEX_EVERY,
            out: PathBuf::from("out/serve"),
            moves: None,
        }
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
    states: Vec<tour::StateDef>,
    /// The game's moves (or why they did not load).
    moves: Result<crate::moves::Table, String>,
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
        let states = tour::state_table(romset::games::values(game)).unwrap_or_default();
        let moves = crate::moves::load(game, cfg.moves.as_deref());
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
            states,
            moves,
        }
    }

    /// The move table, or the answer that says there is none.
    fn moves_table(&self) -> Result<&crate::moves::Table, Answer> {
        match &self.moves {
            Err(e) => Err(internal(format!("the move table does not load: {e}"))),
            Ok(t) if t.rows.is_empty() => {
                Err(Answer::fail(Code::Missing, format!("{} has no move table", self.game.id))
                    .next("remodel moves learn --tours FILE   (learns moves from a recording; needs the ROM)"))
            }
            Ok(t) => Ok(t),
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
            "session.watch" => self.with(args, Self::watch),
            "session.disasm" => self.with(args, Self::disasm),
            "session.list" => self.list(),
            "session.moves" => self.with(args, Self::moves),
            "session.camera" => self.with(args, Self::camera),
            "session.render" => self.with(args, Self::render),
            "session.export" => self.with(args, Self::export),
            "session.close" => self.close(args),
            "spine.index" => self.spine_index(args),
            "spine.where" => self.spine_where(args),
            "scene.render" => self.scene_render(args),
            "scene.probe" => self.scene_probe(args),
            "session.probe" => self.with(args, Self::probe),
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
        // The native geometrizer where the game has one: the same frames
        // and state hashes, much faster (as tours run).
        crate::spine::fresh_machine(roms)
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
        let from = s_arg(args, "from").unwrap_or("power-on").to_string();
        let overlay = l_arg(args, "overlay");
        if !overlay.is_empty() {
            if !from.starts_with("tour:") {
                return Err(bad(
                    "open: `overlay` goes with from=tour:LABEL@FRAME (it is added on top of that recording)",
                ));
            }
            for it in &overlay {
                if it.starts_with("poke=") || it == "coin-start" || !it.contains('@') || tour::item_range(it).is_none()
                {
                    return Err(bad(format!(
                        "overlay `{it}`: want NAME@FROM-TO (jumpfront@2560-3000, steer=0x80@100-200); no pokes"
                    ))
                    .next("remodel inputs"));
                }
                InputScript::parse(it, &game.controls)
                    .map_err(|e| bad(format!("overlay `{it}`: {e}")).next("remodel inputs"))?;
            }
        }
        if n_arg(args, "index_every") == Some(0) || n_arg(args, "checkpoint_every") == Some(0) {
            return Err(bad("open: `index_every` and `checkpoint_every` are at least 1"));
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
        let t0 = Instant::now();
        let rss0 = rss_bytes();
        let id = format!("s{}", self.next_id);
        let mut m = self.fresh_machine(roms);
        let mut ckpt = 0;
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
                let tours = tour::load_paths(game, &self.cfg.tours).map_err(bad)?;
                let Some(t) = tours.iter().find(|t| t.label == label) else {
                    let known: Vec<String> = tours.iter().map(|t| t.label.clone()).collect();
                    return Err(Answer::unknown("tour", label, &known).next("remodel tour list"));
                };
                let (rec_script, rec_poked) = tour::build_script(game, &tours, label).map_err(bad)?;
                let mut tw: Vec<String> = Vec::new();
                // The overlay: terms added on top of the recording up to the open frame.
                let mut overlay = overlay;
                let first = overlay.iter().filter_map(|i| tour::item_range(i).map(|r| r.0)).min();
                if first.is_some_and(|a| a > frame) {
                    tw.push(format!("the overlay starts after frame {frame}: nothing of it is applied"));
                    overlay.clear();
                }
                if overlay.iter().filter_map(|i| tour::item_range(i).map(|r| r.1)).any(|b| b > frame) {
                    tw.push(format!(
                        "the overlay runs past frame {frame}: the session holds what you set with session.input from there, not the rest of it"
                    ));
                }
                let first = overlay.iter().filter_map(|i| tour::item_range(i).map(|r| r.0)).min();
                let (script, poked) = if overlay.is_empty() {
                    (rec_script.clone(), rec_poked)
                } else {
                    let mut items = tour_items(game, &tours, label).map_err(bad)?;
                    items.extend(overlay.iter().cloned());
                    let s = InputScript::parse(&items.join(","), &game.controls).map_err(bad)?;
                    let poked = s.pokes();
                    (s, poked)
                };
                script.apply(0, &mut m.bus);
                // The nearest checkpoint at or before the frame (made on the
                // first open of a spine: one replay), or power-on. With an
                // overlay, one before the overlay's first frame.
                let reach = first.map_or(frame, |a| frame.min(a.saturating_sub(1)));
                let mut from_ckpt = 0;
                if matches!(t.length, tour::Length::Frames(_)) {
                    let dir = spine::dir_of(&self.spines_root(), label);
                    let key = spine::key(game, t, spine::native());
                    if spine::meta(&dir, key).is_none() {
                        let table = self.table.clone();
                        let ce = n_arg(args, "checkpoint_every").unwrap_or(self.cfg.checkpoint_every);
                        let ie = n_arg(args, "index_every").unwrap_or(self.cfg.index_every);
                        if ce == 0 || ie == 0 {
                            return Err(bad("open: `checkpoint_every` and `index_every` are at least 1"));
                        }
                        let b = spine::build(roms, t, &rec_script, &dir, ce, ie, &table).map_err(internal)?;
                        tw.push(format!(
                            "made the spine's checkpoints and index ({} checkpoints, {} rows, {:.1} s, once: {})",
                            b.meta.checkpoints.len(),
                            b.rows,
                            b.seconds,
                            dir.display()
                        ));
                    }
                    if let Some((f, path)) = spine::nearest(&dir, key, reach) {
                        let loaded = std::fs::read(&path)
                            .map_err(|e| e.to_string())
                            .and_then(|d| model2::load_session(&mut m, None, &d));
                        match loaded {
                            Ok(()) => from_ckpt = f,
                            Err(e) => {
                                // A checkpoint that will not load (an older state format): the
                                // whole run from power-on, and the cache is dropped for the next open.
                                tw.push(format!("checkpoint {f} does not load ({e}): replayed from power-on"));
                                let _ = std::fs::remove_dir_all(&dir);
                                m = self.fresh_machine(roms);
                                script.apply(0, &mut m.bus);
                            }
                        }
                    }
                }
                let mut last = m.frames;
                if from_ckpt > 0 {
                    script.apply(m.frames, &mut m.bus);
                }
                ckpt = from_ckpt;
                while m.frames < frame {
                    m.run_slice(u64::MAX / 2)
                        .map_err(|e| internal(format!("the machine stopped at frame {}: {e:?}", m.frames)))?;
                    if m.frames != last {
                        last = m.frames;
                        script.apply(m.frames, &mut m.bus);
                    }
                }
                m.bus.uart_log.clear();
                // An overlaid open exports as a branch from just before the
                // overlay, with the recording's own terms from there to the
                // open frame restated (and the overlay's) as the branch's own.
                let (from_at, base) = match first {
                    None => (frame, vec![]),
                    Some(a) => {
                        let f0 = a.saturating_sub(1);
                        let mut base = Vec::new();
                        for item in tour_items(game, &tours, label).map_err(bad)?.iter().chain(overlay.iter()) {
                            let Some(cut) = tour::cut(item, frame) else { continue };
                            if f0 > 0 {
                                if let Some(t) = tour::tail(&cut, f0 + 1).map_err(bad)? {
                                    base.push(t);
                                }
                            } else {
                                base.push(cut);
                            }
                        }
                        (f0, base)
                    }
                };
                (Start::Tour(label.to_string(), from_at), frame, base, poked, tw)
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
            watch: Vec::new(),
            watch_limit: DEFAULT_WATCH_LIMIT,
            pre: Vec::new(),
            pre_frame: u64::MAX,
            from: from.clone(),
        };
        let hash = s.hash();
        let (frame, ms) = (s.frame(), t0.elapsed().as_secs_f64() * 1e3);
        let rss = rss_bytes().saturating_sub(rss0);
        self.sessions.insert(id.clone(), s);
        let mut a = Answer::done(json!({
            "session": id, "game": game.id, "from": from, "frame": frame, "state_hash": format!("{hash:016x}"),
            "limits": {"marks": self.cfg.marks, "max_step": self.cfg.max_step, "sessions": self.cfg.sessions},
            "ms": ms.round(), "rss_added_mb": (rss / (1 << 20)),
            "checkpoint": ckpt, "stepped": frame.saturating_sub(ckpt),
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

    /// The recording `follow` names: its label and its terms, pokes resolved.
    fn follow_items(&self, follow: &str) -> Result<(String, Vec<String>), Answer> {
        let label = follow.strip_prefix("tour:").unwrap_or(follow);
        let tours = tour::load_paths(self.game, &self.cfg.tours).map_err(bad)?;
        if !tours.iter().any(|t| t.label == label) {
            let known: Vec<String> = tours.iter().map(|t| t.label.clone()).collect();
            return Err(Answer::unknown("tour", label, &known).next("remodel tour list"));
        }
        let items = tour_items(self.game, &tours, label).map_err(bad)?;
        Ok((label.to_string(), items))
    }

    /// The moves a step plays: their names, inputs and frames, from the table.
    fn step_moves(&self, names: &[String]) -> Result<Vec<(String, Vec<String>, u64)>, Answer> {
        // `wait-N` is N frames of no input, so moves can be spaced out.
        let wait = |n: &str| n.strip_prefix("wait-").and_then(|w| w.trim_end_matches('f').parse::<u64>().ok());
        if names.iter().all(|n| wait(n).is_some_and(|w| w > 0)) {
            return Err(bad("step: `move` names only waits; give a move from session.moves too"));
        }
        let table = self.moves_table()?;
        names
            .iter()
            .map(|n| {
                if let Some(w) = wait(n).filter(|w| *w > 0) {
                    return Ok((n.clone(), Vec::new(), w));
                }
                table.find(n).map(|r| (r.name.clone(), r.terms.clone(), r.frames)).ok_or_else(|| {
                    let known: Vec<String> = table.names().iter().map(|s| s.to_string()).collect();
                    Answer::unknown("move", n, &known).next("session.moves")
                })
            })
            .collect()
    }

    fn step(&mut self, s: &mut Session, args: &Value) -> Result<Answer, Answer> {
        let game = self.game;
        let move_names = l_arg(args, "move");
        let moves = if move_names.is_empty() { Vec::new() } else { self.step_moves(&move_names)? };
        let seq: u64 = moves.iter().map(|m| m.2).sum();
        let until_given = s_arg(args, "until").is_some();
        if !moves.is_empty() {
            if args.get("follow").is_some() || args.get("inputs").is_some() {
                return Err(bad(
                    "step: `move` is not combined with `follow` or `inputs` (a move is the step's inputs)",
                ));
            }
            if !until_given {
                if let Some(f) = n_arg(args, "frames").filter(|f| *f != seq) {
                    return Err(bad(format!(
                        "step: the moves are {seq} frames; `frames` ({f}) would cut them or leave frames over (leave it out, or add an `until` to repeat them)"
                    )));
                }
            }
        }
        // Moves with no `until` run once. A step that starts on the frame the
        // session opened on runs one more, as that frame keeps its start's inputs.
        let lead = u64::from(s.frame() <= s.start);
        let frames = if moves.is_empty() {
            n_arg(args, "frames").unwrap_or(0)
        } else if until_given {
            n_arg(args, "frames").unwrap_or(self.cfg.max_step)
        } else {
            seq + lead
        };
        if frames == 0 {
            return Err(bad("step: `frames` is at least 1 (or give `move`)"));
        }
        if frames > self.cfg.max_step {
            return Err(bad(format!(
                "step: {frames} frames is over the server's limit of {} (--max-step)",
                self.cfg.max_step
            ))
            .with_data(json!({"limit": self.cfg.max_step, "asked": frames}))
            .next(format!("session.step frames={}", self.cfg.max_step)));
        }
        let mut until = s_arg(args, "until").map(|u| parse_until(u, &self.table, &self.states)).transpose()?;
        let following = s_arg(args, "follow").map(|f| self.follow_items(f)).transpose()?;
        // Held: the step's own inputs, else the session's; following a
        // recording, only what the step is given (an overlay on top of it).
        let held = if !moves.is_empty() {
            Vec::new()
        } else if args.get("inputs").is_some() {
            let items = l_arg(args, "inputs");
            check_held(&items, game)?;
            items
        } else if following.is_some() {
            Vec::new()
        } else {
            s.held.clone()
        };
        // The first frame this step gives inputs to, and the last.
        let (first, last) = (s.frame().max(s.start + 1), s.frame() + frames);
        // A followed recording, the overlay and the session's pokes as one script.
        let mut script = None;
        if let Some((label, items)) = &following {
            let mut terms: Vec<String> = Vec::new();
            for it in items {
                if it.starts_with("poke=") {
                    // The recording's pokes become the session's, so the log (and
                    // an export) carries them and `before_poke` knows them.
                    let (what, _) = it.rsplit_once('@').unwrap_or((it, ""));
                    let range = tour::item_range(it).filter(|_| !it.contains('/')).ok_or_else(|| {
                        bad(format!("follow {label}: `{it}` repeats, and a repeating poke cannot be followed"))
                    })?;
                    let (from, to) = (range.0.max(first), range.1.min(last));
                    let log = PokeLog { term: what.to_string(), from, to };
                    if from <= to && !s.pokes.iter().any(|p| p.term == log.term && p.from == from && p.to == to) {
                        s.pokes.push(log);
                        s.poked = true;
                    }
                } else {
                    terms.push(it.clone());
                }
            }
            terms.extend(held.iter().map(|h| format!("{h}@{first}-{last}")));
            terms.extend(s.pokes.iter().map(|p| format!("{}@{}-{}", p.term, p.from, p.to)));
            script = Some(
                InputScript::parse(&terms.join(","), &game.controls)
                    .map_err(|e| bad(format!("follow {label}: {e}")))?,
            );
        }
        // Moves: each one's terms from where the last ended, repeated while
        // `until` has frames left; the session's held inputs return after.
        if !moves.is_empty() {
            let mut terms: Vec<String> = Vec::new();
            let mut pos = first;
            let limit = s.frame() + frames;
            'seq: while pos < limit {
                for (_, mt, mf) in &moves {
                    for t in mt {
                        terms.push(shift_term(t, pos).map_err(internal)?);
                    }
                    pos += mf;
                    if pos >= limit {
                        break 'seq;
                    }
                }
                if !until_given {
                    break;
                }
            }
            terms.extend(s.held.iter().map(|h| format!("{h}@{limit}")));
            terms.extend(s.pokes.iter().map(|p| format!("{}@{}-{}", p.term, p.from, p.to)));
            script = Some(
                InputScript::parse(&terms.join(","), &game.controls)
                    .map_err(|e| internal(format!("a move's inputs do not parse: {e}")))?,
            );
        }
        if let Some(u) = &mut until {
            u.arm(&s.m);
        }
        if !s.watch.is_empty() {
            let ranges = s.watch.iter().map(|w| (w.start, w.end)).collect();
            s.m.bus.watch =
                Some(Box::new(model2::bus::Watch { ranges, hits: Vec::new(), cap: s.watch_limit, dropped: 0 }));
        }
        let t0 = Instant::now();
        let run = (|| -> Result<(u64, &'static str), String> {
            // The frame about to run takes the step's inputs (not the start's).
            if s.frame() > s.start {
                s.apply_with(game, s.frame(), &held, script.as_ref())?;
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
                s.run_frame()?;
                ran += 1;
                let f = s.frame();
                s.apply_with(game, f, &held, script.as_ref())?;
                if let Some(u) = &until {
                    if u.met(&s.m, ran) {
                        why = "until";
                        break;
                    }
                }
            }
            Ok((ran, why))
        })();
        let watch = s.m.bus.watch.take();
        let (ran, why) = run.map_err(internal)?;
        s.frames_run += ran;
        let secs = t0.elapsed().as_secs_f64();
        let hash = s.hash();
        let mut data = json!({
            "session": s.id, "frame": s.frame(), "state_hash": format!("{hash:016x}"),
            "frames_run": ran, "stopped": why, "held": held,
            "ms": (secs * 1e3).round(), "frames_per_second": (ran as f64 / secs.max(1e-9)).round(),
        });
        if let Some((label, _)) = &following {
            data["followed"] = json!(label);
        }
        if !moves.is_empty() {
            data["moves"] = json!(moves.iter().map(|m| m.0.as_str()).collect::<Vec<_>>());
            data["sequence_frames"] = json!(seq);
            data["repeats"] = json!(ran.div_ceil(seq.max(1)).max(1));
        }
        if let Some(u) = until.as_ref().filter(|u| !matches!(u, Until::Frames(_))) {
            data["until_reading"] = u.reading(&s.m);
        }
        let mut a = Answer::done(json!(null));
        if let Some(w) = watch {
            data["watch"] = watch_json(&s.watch, &w);
            if w.dropped > 0 {
                a = a.warn(format!(
                    "{} writes over the watch's limit of {} were not kept (session.watch limit=...)",
                    w.dropped, s.watch_limit
                ));
            }
        }
        Ok(a.with_data(data))
    }

    /// The conditions the session is in now, as the move table words them:
    /// `airborne`, `speed` (a band of the table's cutoffs) and `screen`.
    fn conditions(&self, s: &Session, bands: &[f64]) -> Vec<(String, String)> {
        let val = |name: &str| {
            let def = self.table.iter().find(|d| d.name == name)?;
            read_def(&s.m, def).and_then(|(_, v)| v.as_f64())
        };
        let screen = val(crate::moves::V_SCREEN);
        let name = screen.and_then(|n| {
            self.states
                .iter()
                .find(|st| st.value == crate::moves::V_SCREEN && st.number as f64 == n)
                .map(|st| st.name.clone())
        });
        crate::moves::cond_of(
            &crate::moves::Cond {
                airborne: val(crate::moves::V_AIRBORNE).map(|a| a > 0.0),
                speed: val(crate::moves::V_SPEED),
                screen: name.unwrap_or_default(),
            },
            bands,
        )
    }

    fn moves(&mut self, s: &mut Session, args: &Value) -> Result<Answer, Answer> {
        let table = self.moves_table()?;
        let have = self.conditions(s, &table.speed_bands);
        let mode = s_arg(args, "where").unwrap_or("current");
        let want: Option<Vec<(String, String)>> = match mode {
            "all" => None,
            "current" => Some(have.clone()),
            other => Some(
                other
                    .split(',')
                    .map(|c| c.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
                    .collect::<Option<_>>()
                    .ok_or_else(|| {
                        bad(format!("where `{other}`: want current, all, or NAME=VALUE,... (airborne=0, speed=mid)"))
                    })?,
            ),
        };
        let mut rows: Vec<&crate::moves::MoveRow> = Vec::new();
        for name in table.names() {
            let row = match &want {
                None => table.best(name, &[]),
                Some(w) => table.best(name, w),
            };
            rows.extend(row);
        }
        match s_arg(args, "sort") {
            None => rows.sort_by_key(|r| std::cmp::Reverse(r.samples)),
            Some(k) => {
                let (desc, name) = match k.strip_prefix('-') {
                    Some(n) => (true, n),
                    None => (false, k),
                };
                if !crate::moves::EFFECTS.contains(&name) {
                    let all: Vec<String> = crate::moves::EFFECTS.iter().map(|e| e.to_string()).collect();
                    return Err(Answer::unknown("effect", name, &all));
                }
                let key = |r: &crate::moves::MoveRow| {
                    r.effect(name).map_or(f64::INFINITY, |e| if desc { -e.median } else { e.median })
                };
                rows.sort_by(|a, b| key(a).total_cmp(&key(b)));
            }
        }
        let total = rows.len();
        rows.truncate(n_arg(args, "limit").unwrap_or(40) as usize);
        let list: Vec<Value> = rows
            .iter()
            .map(|r| {
                let fx: serde_json::Map<String, Value> = r
                    .effects
                    .iter()
                    .map(|e| (e.name.clone(), json!({"median": e.median, "min": e.lo, "max": e.hi})))
                    .collect();
                json!({
                    "move": r.name, "inputs": r.terms, "frames": r.frames, "conditions": r.cond_text(),
                    "conditional": !r.cond.is_empty(), "window": r.window, "effects": fx, "samples": r.samples,
                })
            })
            .collect();
        let cond: Vec<String> = have.iter().map(|(k, v)| format!("{k}={v}")).collect();
        Ok(Answer::done(json!({
            "session": s.id, "frame": s.frame(), "current": cond, "where": mode, "count": total, "moves": list,
            "speed_bands": table.speed_bands,
        }))
        .next("session.step move=NAME"))
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
                Mark {
                    name: name.to_string(),
                    frame: 0,
                    hash: 0,
                    m,
                    log: vec![],
                    pokes: vec![],
                    held: vec![],
                    pre: vec![],
                    pre_frame: u64::MAX,
                }
            }
        };
        s.m.bus.geo.sync();
        k.m.copy_from(&mut s.m, &mut CopyMarks::default());
        k.frame = s.frame();
        k.hash = s.hash();
        k.log = s.log.clone();
        k.pokes = s.pokes.clone();
        k.held = s.held.clone();
        k.pre = s.pre.clone();
        k.pre_frame = s.pre_frame;
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
        s.pre = k.pre.clone();
        s.pre_frame = k.pre_frame;
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
        let format = match s_arg(args, "format") {
            None | Some("json") => "json",
            Some("compact") => "compact",
            Some("tsv") => "tsv",
            Some(o) => return Err(Answer::unknown("format", o, &["json".into(), "compact".into(), "tsv".into()])),
        };
        let before = match s_arg(args, "when") {
            None | Some("after") => false,
            Some("before_poke") => true,
            Some(o) => return Err(Answer::unknown("when", o, &["after".into(), "before_poke".into()])),
        };
        let all: Vec<String>;
        let names: &[String] = if names.is_empty() {
            all = self.table.iter().map(|d| d.name.clone()).collect();
            &all
        } else {
            &names
        };
        // (key, the row as JSON, the value alone, whether a poke of this frame covers it)
        let mut rows: Vec<(String, Value, Value, bool)> = Vec::new();
        for n in names {
            if let Some(d) = self.table.iter().find(|d| d.name == *n) {
                let Some(raw) = peek_at(s, d.addr, d.width, before) else {
                    return Err(bad(format!("value {n}: {:#x} is not mapped memory", d.addr)));
                };
                let v = crate::ram::decode(d, raw);
                let covered = poke_covers(s, d.addr, d.width);
                rows.push((
                    n.clone(),
                    json!({"name": d.name, "addr": format!("{:#x}", d.addr), "width": d.width, "raw": raw, "value": v}),
                    v,
                    covered,
                ));
                continue;
            }
            let table: Vec<String> = self.table.iter().map(|d| d.name.clone()).collect();
            let raw_read = match parse_raw(n) {
                Some(r) => r.map_err(|e| bad(format!("{n}: {e}")))?,
                None => return Err(Answer::unknown("value", n, &table).next("remodel ram values")),
            };
            match raw_read {
                RawRead::One { addr, width, kind } => {
                    let Some(raw) = peek_at(s, addr, width, before) else {
                        return Err(bad(format!("{n}: {addr:#x} is not mapped memory")));
                    };
                    let def = ValueDef { name: String::new(), addr, width, kind, note: String::new() };
                    let v = crate::ram::decode(&def, raw);
                    rows.push((
                        n.clone(),
                        json!({"name": null, "addr": format!("{addr:#x}"), "width": width, "raw": raw, "value": v}),
                        v,
                        poke_covers(s, addr, width),
                    ));
                }
                RawRead::Range { addr, len, elem } => {
                    let mut bytes = Vec::with_capacity(len);
                    for i in 0..len as u32 {
                        let a = addr.wrapping_add(i);
                        let Some(b) = peek_at(s, a, 1, before) else {
                            return Err(bad(format!(
                                "{n}: {a:#x} is not mapped memory (the range stops at {addr:#x}+{i})"
                            )));
                        };
                        bytes.push(b as u8);
                    }
                    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                    let mut row = json!({"name": null, "addr": format!("{addr:#x}"), "len": len, "hex": hex});
                    let mut alone = json!(hex);
                    if let Some((width, kind)) = elem {
                        let def = ValueDef { name: String::new(), addr, width, kind, note: String::new() };
                        let vals: Vec<Value> = bytes
                            .chunks_exact(width)
                            .map(|c| {
                                let raw = c.iter().enumerate().fold(0u32, |a, (i, b)| a | (*b as u32) << (8 * i));
                                crate::ram::decode(&def, raw)
                            })
                            .collect();
                        row["width"] = json!(width);
                        row["values"] = json!(vals);
                        alone = json!(vals);
                    }
                    rows.push((n.clone(), row, alone, poke_covers(s, addr, len)));
                }
            }
        }
        let frame = s.frame();
        let mut data = match format {
            "compact" => {
                let m: serde_json::Map<String, Value> = rows.iter().map(|r| (r.0.clone(), r.2.clone())).collect();
                json!({"session": s.id, "frame": frame, "values": m})
            }
            "tsv" => {
                let text: String = rows
                    .iter()
                    .map(|r| {
                        let v = match &r.2 {
                            Value::String(t) => t.clone(),
                            Value::Array(a) => a.iter().map(Value::to_string).collect::<Vec<_>>().join(" "),
                            o => o.to_string(),
                        };
                        format!("{}\t{v}\n", r.0)
                    })
                    .collect();
                json!({"session": s.id, "frame": frame, "text": text})
            }
            _ => {
                json!({"session": s.id, "frame": frame, "values": rows.iter().map(|r| r.1.clone()).collect::<Vec<_>>()})
            }
        };
        data["when"] = json!(if before { "before_poke" } else { "after" });
        let mut a = Answer::done(json!(null));
        let covered: Vec<&str> = rows.iter().filter(|r| r.3).map(|r| r.0.as_str()).collect();
        if !covered.is_empty() && !before {
            data["poked"] = json!(covered);
            a = a.warn(format!(
                "{} {} covered by a poke at frame {frame}: pokes apply at the start of a frame, so it reads as the poke wrote it, before the game has run that frame (which may rewrite it); `when=before_poke` reads it as the previous frame left it",
                covered.join(", "),
                if covered.len() == 1 { "is" } else { "are" }
            ));
        }
        if before && !(s.pre_frame == frame && !s.pre.is_empty()) {
            a = a.warn("when=before_poke: no poke has applied to this frame, so there is nothing to read before (the values are as the machine is)");
        }
        if self.table.is_empty() {
            a = a.warn(format!("{} has no named values yet: raw ADDR[:b|w|l] reads work", self.game.id));
        }
        Ok(a.with_data(data))
    }

    fn watch(&mut self, s: &mut Session, args: &Value) -> Result<Answer, Answer> {
        if let Some(l) = n_arg(args, "limit") {
            if l == 0 || l > 5000 {
                return Err(bad("watch: `limit` is 1 to 5000"));
            }
            s.watch_limit = l as usize;
        }
        if args.get("what").is_some() {
            let what = l_arg(args, "what");
            if what.len() > 32 {
                return Err(bad("watch: at most 32 things at once"));
            }
            let mut items = Vec::new();
            for w in &what {
                if let Some(d) = self.table.iter().find(|d| d.name == *w) {
                    items.push(WatchItem {
                        what: w.clone(),
                        start: d.addr,
                        end: d.addr + d.width as u32,
                        def: Some(d.clone()),
                    });
                    continue;
                }
                let (a, l) = w.split_once(':').unwrap_or((w, "4"));
                let len = match l {
                    "b" => Some(1),
                    "w" => Some(2),
                    "l" => Some(4),
                    n => num(n),
                };
                let (Some(addr), Some(len)) = (num(a).and_then(|a| u32::try_from(a).ok()), len) else {
                    let table: Vec<String> = self.table.iter().map(|d| d.name.clone()).collect();
                    return Err(Answer::unknown("value", w, &table).next("session.watch what=NAME or ADDR[:LEN]"));
                };
                if len == 0 || len > 65536 || addr.checked_add(len as u32).is_none() {
                    return Err(bad(format!("watch `{w}`: a length of 1 to 65536 bytes")));
                }
                items.push(WatchItem { what: w.clone(), start: addr, end: addr + len as u32, def: None });
            }
            s.watch = items;
        }
        let watching: Vec<Value> = s
            .watch
            .iter()
            .map(|w| json!({"what": w.what, "addr": format!("{:#x}", w.start), "len": w.end - w.start}))
            .collect();
        Ok(Answer::done(json!({"session": s.id, "watching": watching, "limit": s.watch_limit}))
            .next(format!("session.step session={} frames=60   (the writes come back in `watch`)", s.id)))
    }

    fn disasm(&mut self, s: &mut Session, args: &Value) -> Result<Answer, Answer> {
        let at = s_arg(args, "addr").unwrap_or("");
        let addr = num(at)
            .and_then(|a| u32::try_from(a).ok())
            .ok_or_else(|| bad(format!("disasm: addr `{at}` is a number (0x1234)")))?;
        let count = n_arg(args, "count").unwrap_or(16);
        if count == 0 || count > 256 {
            return Err(bad("disasm: `count` is 1 to 256"));
        }
        let back = n_arg(args, "back").unwrap_or(0);
        if back > 1024 {
            return Err(bad("disasm: `back` is at most 1024 bytes"));
        }
        let mut a = addr.saturating_sub(back as u32) & !3;
        let mut lines = Vec::new();
        let mut unmapped = false;
        for _ in 0..count {
            if s.m.bus.peek(a, 4).is_none() {
                unmapped = true;
                break;
            }
            let insn = i960::decode(a, |x| s.m.bus.peek(x, 4));
            let mark = if a == addr { "=>" } else { "  " };
            lines.push(format!("{mark} {a:#08x}  {:08x}  {insn}", insn.word));
            a = insn.next();
        }
        let mut ans = Answer::done(json!({"session": s.id, "addr": format!("{addr:#x}"), "listing": lines}));
        if unmapped {
            ans = ans.warn(format!("{a:#x} is not mapped memory: the listing stops there"));
        }
        if back > 0 {
            ans = ans.warn(
                "`back` starts before the address: if it lands inside a two-word instruction the first lines are wrong",
            );
        }
        Ok(ans)
    }

    fn list(&mut self) -> Result<Answer, Answer> {
        let rows: Vec<Value> = self
            .sessions
            .values()
            .map(|s| {
                json!({
                    "session": s.id, "from": s.from, "frame": s.frame(), "started": s.start,
                    "marks": s.marks.iter().map(|k| k.name.as_str()).collect::<Vec<_>>(),
                    "held": s.held, "pokes": s.pokes.len(), "poked": s.poked,
                    "watching": s.watch.iter().map(|w| w.what.as_str()).collect::<Vec<_>>(),
                    "camera": (!s.camera.is_empty()).then_some(&s.camera),
                })
            })
            .collect();
        Ok(Answer::done(json!({"sessions": rows, "limit": self.cfg.sessions})).next("session.open"))
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
                let words = crate::scene::expand_camera(c).map_err(|e| bad(format!("camera: {e}")))?;
                model2::photo::Spec::parse(&words, 1.0).map_err(|e| bad(format!("camera: {e}")))?;
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
        if camera.as_deref().is_some_and(|c| ["preset=side", "preset=top"].iter().any(|p| c.contains(p))) {
            a = a.warn("a camera preset moves the eye by shares of the scene's scale and the game's world is not tested for walls: in an enclosed course (a hall, a tunnel) the eye can land inside a wall or the ceiling, and the picture is then empty or shows back faces; if so, try preset=chase or a smaller move (tilt=, orbit=, z=)");
        }
        Ok(a)
    }

    fn probe_stack(args: &Value) -> Result<crate::probe::Stack, Answer> {
        let packs: Vec<PathBuf> = l_arg(args, "pack").into_iter().map(PathBuf::from).collect();
        crate::probe::Stack::open(&packs).map_err(|e| Answer::fail(Code::Missing, e))
    }

    fn probe(&mut self, s: &mut Session, args: &Value) -> Result<Answer, Answer> {
        self.roms()?;
        let rect = crate::probe::parse_rect(s_arg(args, "at").unwrap_or(""))?;
        let stack = Self::probe_stack(args)?;
        s.m.bus.geo.sync();
        let frame = model2::capture::Frame::grab(&mut s.m.bus);
        let rom = s.m.bus.texture_rom.clone();
        let mut data = crate::probe::probe_frame(&frame, &rom, rect, &stack);
        data["session"] = json!(s.id);
        data["frame"] = json!(s.frame());
        Ok(Answer::done(data))
    }

    fn scene_probe(&mut self, args: &Value) -> Result<Answer, Answer> {
        let game = self.game;
        let rs = crate::scene::recipes(game, None).map_err(bad)?;
        let rect = crate::probe::parse_rect(s_arg(args, "at").unwrap_or(""))?;
        let name = s_arg(args, "scene").unwrap_or("");
        let Some(r) = rs.iter().find(|r| r.name == name) else {
            let known: Vec<String> = rs.iter().map(|r| r.name.clone()).collect();
            return Err(Answer::unknown("scene", name, &known));
        };
        let dir = crate::scene::scenes_dir(game).join(&r.name);
        let stack = Self::probe_stack(args)?;
        let mut data = crate::probe::probe_scene(&dir, rect, &stack)?;
        data["scene"] = json!(r.name);
        Ok(Answer::done(data))
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
            // An overlaid open branches from just before the overlay (or from
            // power-on) and restates the recording from there as its own terms.
            Start::Tour(label, frame) => {
                if *frame > 0 {
                    all.push(format!("from={label}@{frame}"));
                }
                all.extend(s.base.iter().cloned());
            }
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
            let mut text = String::new();
            for p in &self.cfg.tours {
                text.push_str(&std::fs::read_to_string(p).unwrap_or_default());
                text.push('\n');
            }
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

    // ---------------------------------------------------------------- spines

    fn spines_root(&self) -> PathBuf {
        self.cfg.spines.clone().unwrap_or_else(|| spine::default_dir(self.game.id))
    }

    /// The tour `label` of the game's and the project's files, and its
    /// whole input as a script.
    fn spine_tour(&self, label: &str) -> Result<(tour::Tour, InputScript), Answer> {
        let tours = tour::load_paths(self.game, &self.cfg.tours).map_err(bad)?;
        let Some(t) = tours.iter().find(|t| t.label == label) else {
            let known: Vec<String> = tours.iter().map(|t| t.label.clone()).collect();
            return Err(Answer::unknown("tour", label, &known).next("remodel tour list"));
        };
        let (script, _) = tour::build_script(self.game, &tours, label).map_err(bad)?;
        Ok((t.clone(), script))
    }

    fn spine_build(
        &self,
        t: &tour::Tour,
        script: &InputScript,
        every: u64,
        index_every: u64,
    ) -> Result<spine::Built, Answer> {
        let roms = self.roms()?;
        spine::build(roms, t, script, &spine::dir_of(&self.spines_root(), &t.label), every, index_every, &self.table)
            .map_err(|e| bad(e).next("remodel tour list"))
    }

    fn spine_index(&mut self, args: &Value) -> Result<Answer, Answer> {
        let label = s_arg(args, "spine").unwrap_or("");
        let every = n_arg(args, "every").unwrap_or(self.cfg.checkpoint_every);
        let index_every = n_arg(args, "index_every").unwrap_or(self.cfg.index_every);
        if every == 0 || index_every == 0 {
            return Err(bad("spine.index: `every` and `index_every` are at least 1"));
        }
        let (t, script) = self.spine_tour(label)?;
        let b = self.spine_build(&t, &script, every, index_every)?;
        let dir = spine::dir_of(&self.spines_root(), label);
        Ok(Answer::done(json!({
            "spine": label, "dir": dir.display().to_string(), "key": format!("{:016x}", b.meta.key),
            "frames": b.meta.frames, "checkpoint_every": every, "checkpoints": b.meta.checkpoints.len(),
            "index_every": index_every, "index_rows": b.rows, "values": self.table.len(),
            "checkpoint_mb": (b.bytes / (1 << 20)), "seconds": b.seconds,
        }))
        .next(format!("spine.where spine={label} near=NAME=VALUE")))
    }

    fn spine_where(&mut self, args: &Value) -> Result<Answer, Answer> {
        let label = s_arg(args, "spine").unwrap_or("");
        let limit = n_arg(args, "limit").unwrap_or(5).clamp(1, 1000) as usize;
        let gap = n_arg(args, "gap").unwrap_or(90);
        let near =
            l_arg(args, "near").iter().map(|s| spine::parse_near(s)).collect::<Result<Vec<_>, _>>().map_err(bad)?;
        let filters =
            l_arg(args, "where").iter().map(|s| spine::parse_filter(s)).collect::<Result<Vec<_>, _>>().map_err(bad)?;
        let (t, script) = self.spine_tour(label)?;
        let dir = spine::dir_of(&self.spines_root(), label);
        let key = spine::key(self.game, &t, spine::native());
        let mut made = None;
        let index = match spine::load_index(&dir, key, &self.table) {
            Some(i) => i,
            None => {
                if self.roms.is_none() {
                    return Err(Answer::fail(
                        Code::Missing,
                        format!("spine {label} has no index (or it is of another recording)"),
                    )
                    .next(format!("remodel spine index {label}   (needs the ROM)")));
                }
                let b = self.spine_build(&t, &script, self.cfg.checkpoint_every, self.cfg.index_every)?;
                made =
                    Some(format!("made the spine's index and checkpoints ({} rows, {:.1} s, once)", b.rows, b.seconds));
                spine::load_index(&dir, key, &self.table)
                    .ok_or_else(|| internal("the index just made does not load"))?
            }
        };
        let found = index.find(&near, &filters, limit, gap).map_err(|e| bad(e).next("remodel ram values"))?;
        let (kind, results, empty) = match &found {
            spine::Found::Nearest(h) => {
                ("nearest", h.iter().map(|h| spine::hit_json(label, h)).collect::<Vec<_>>(), h.is_empty())
            }
            spine::Found::Spans(s) => {
                ("spans", s.iter().map(|s| spine::span_json(label, s)).collect::<Vec<_>>(), s.is_empty())
            }
        };
        let mut a = Answer::done(json!({
            "spine": label, "kind": kind, "frames": index.frames, "index_every": index.every,
            "results": results,
        }));
        if empty {
            a = a.warn("nothing in the index matches");
        }
        if let Some(w) = made {
            a = a.warn(w);
        }
        Ok(a.next("session.open from=tour:LABEL@FRAME (a result's `open`)"))
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
        assert_eq!(VERBS.len(), 21);
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
        // `frames` is needed unless a move gives the length: the step itself says so.
        assert!(validate(v, &json!({"move": ["a"]})).is_ok());
        assert!(validate(v, &json!({"move": "a"})).is_err(), "a move is a list");
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
        assert!(matches!(parse_until("value speed gt 2.5", &t, &[]), Ok(Until::Value { op: "gt", .. })));
        assert!(matches!(parse_until("frames 0x10", &t, &[]), Ok(Until::Frames(16))));
        assert!(parse_until("value speed bigger 1", &t, &[]).is_err());
        assert!(parse_until("value spede gt 1", &t, &[]).unwrap_err().error.as_ref().unwrap().contains("speed"));
        assert!(parse_until("when speed", &t, &[]).is_err());
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
        let cfg = Config { marks: 2, max_step: 3600, sessions: 2, out: out.clone(), ..Config::default() };
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
            // A preset is a camera too, and the ones that can end inside walls say so.
            let a = srv.call("session.render", &json!({"view": "540,16:9", "name": "t-top", "camera": "preset=top"}));
            assert!(a.ok(), "{:?}", a.error);
            assert!(a.warnings.iter().any(|w| w.contains("inside a wall")), "{:?}", a.warnings);
            let a = srv.call("session.camera", &json!({"camera": "preset=chase,fov=1.2"}));
            assert!(a.ok() && a.data["camera"] == "preset=chase,fov=1.2", "{a:?}");
            let a = srv.call("session.render", &json!({"view": "540,16:9", "name": "t-chase"}));
            assert!(a.ok() && a.warnings.is_empty(), "{:?} {:?}", a.error, a.warnings);
            let e = srv.call("session.camera", &json!({"camera": "preset=roof"}));
            assert_eq!(e.code, Code::BadRequest);
            assert!(e.message().contains("chase, side, top"), "{}", e.message());
            assert!(srv.call("session.camera", &json!({"camera": "orbit=20,tilt=-5"})).ok());
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
    /// A short recording as a spine (skipped without the ROM): the checkpoints
    /// give the same state as stepping from power-on, `spine.where` finds
    /// frames that open, a changed recording drops the cache, and a branch
    /// exported from the session replays to its hash, its spine in another file.
    #[test]
    fn a_spine_opens_from_checkpoints_and_exports_a_branch() {
        let game = romset::games::find("topskatr").unwrap();
        let rom = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rom");
        let Ok(roms) = RomSet::load(game, &rom, false) else { return };
        let out =
            Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../out/scratch/spine-test-{}", std::process::id()));
        std::fs::create_dir_all(&out).unwrap();
        std::env::set_var("REMODEL_LOCK_DIR", out.join("locks"));
        let items: Vec<String> = game.controls.coin_start.split(',').map(str::to_string).collect();
        let spine_file = out.join("spine.txt");
        let write_spine = |len: u64| {
            std::fs::write(&spine_file, format!("spn|{len}f|coin-start\n")).unwrap();
        };
        write_spine(1300);
        let cfg = Config {
            tours: vec![spine_file.clone()],
            spines: Some(out.join("spines")),
            checkpoint_every: 400,
            index_every: 50,
            out: out.clone(),
            ..Config::default()
        };
        let mut srv = Server::new(game, Some(&roms), cfg.clone());

        // The state at frame 1000 stepped from power-on: the reference.
        let a = srv.call("session.open", &json!({}));
        assert!(a.ok(), "{:?}", a.error);
        drive(&mut srv, &items, 0, 1000);
        let want = format!("{:016x}", srv.sessions.values_mut().next().unwrap().hash());
        assert!(srv.call("session.close", &json!({})).ok());

        // The first open makes the cache (once); it loads the checkpoint at 800 and steps 200.
        let a = srv.call("session.open", &json!({"from": "tour:spn@1000"}));
        assert!(a.ok(), "{:?}", a.error);
        assert_eq!(a.data["checkpoint"], 800, "{a:?}");
        assert_eq!(a.data["stepped"], 200);
        assert!(a.warnings.iter().any(|w| w.contains("made the spine's")), "{:?}", a.warnings);
        assert_eq!(hash_of(&a), want, "opening through a checkpoint gives power-on's state");
        assert!(srv.call("session.close", &json!({})).ok());
        // Every frame, at, before and after a checkpoint, agrees; the second open makes nothing.
        for f in [1u64, 399, 400, 401, 799] {
            let a = srv.call("session.open", &json!({"from": format!("tour:spn@{f}")}));
            assert!(a.ok() && a.warnings.is_empty(), "{:?} {:?}", a.error, a.warnings);
            assert_eq!(a.data["checkpoint"], f / 400 * 400);
            let h = hash_of(&a);
            let b = srv.call("session.close", &json!({}));
            assert!(b.ok());
            let a = srv.call("session.open", &json!({}));
            assert!(a.ok());
            drive(&mut srv, &items, 0, f);
            let w = format!("{:016x}", srv.sessions.values_mut().next().unwrap().hash());
            assert_eq!(h, w, "frame {f}");
            assert!(srv.call("session.close", &json!({})).ok());
        }

        // spine.where: the stretches on a screen, and the frames nearest one.
        let a = srv.call("spine.where", &json!({"spine": "spn", "where": ["screen>=0"]}));
        assert!(a.ok(), "{:?}", a.error);
        assert_eq!(a.data["kind"], "spans");
        assert_eq!(a.data["results"][0]["from"], 50);
        assert_eq!(a.data["results"][0]["to"], 1300 / 50 * 50);
        let a = srv.call("spine.where", &json!({"spine": "spn", "near": ["clock=0", "screen=32"], "limit": 3}));
        assert!(a.ok(), "{:?}", a.error);
        let hits = a.data["results"].as_array().unwrap();
        assert_eq!(a.data["kind"], "nearest");
        assert!(!hits.is_empty());
        let at = hits[0]["frame"].as_u64().unwrap();
        assert_eq!(hits[0]["distance"], 0.0, "the boot text is on screen 32: {hits:?}");
        assert_eq!(hits[0]["open"], format!("tour:spn@{at}"));
        assert_eq!(hits[0]["values"]["screen"], 32.0);
        let e = srv.call("spine.where", &json!({"spine": "spn", "near": ["scren=1"]}));
        assert_eq!(e.code, Code::BadRequest);
        assert!(e.message().contains("did you mean screen"), "{}", e.message());
        let a = srv.call("session.open", &json!({"from": hits[0]["open"]}));
        assert!(a.ok(), "{:?}", a.error);
        assert_eq!(a.data["frame"], at);
        // And on a screen that is never there: nothing, with a word about it.
        let a = srv.call("spine.where", &json!({"spine": "spn", "where": ["screen=999"]}));
        assert!(a.ok() && a.data["results"].as_array().unwrap().is_empty());
        assert!(srv.call("session.close", &json!({})).ok());

        // A branch from the spine at 1000: step with its own inputs, export, replay.
        let a = srv.call("session.open", &json!({"from": "tour:spn@1000"}));
        assert!(a.ok(), "{:?}", a.error);
        assert!(srv.call("session.input", &json!({"inputs": ["start"]})).ok());
        let a = srv.call("session.step", &json!({"frames": 40}));
        assert!(a.ok(), "{:?}", a.error);
        let exp = srv.call("session.export", &json!({"label": "br", "file": "br"}));
        assert!(exp.ok(), "{:?}", exp.error);
        let line = exp.data["line"].as_str().unwrap().to_string();
        assert!(line.starts_with("br|1040f|from=spn@1000,"), "{line}");
        // The spine in one file, the branch alone in another.
        let branch_file = out.join("branch.txt");
        std::fs::write(&branch_file, format!("{line}\n")).unwrap();
        let tours = tour::load_paths(game, &[spine_file.clone(), branch_file.clone()]).unwrap();
        assert!(tour::load_paths(game, std::slice::from_ref(&branch_file)).is_err(), "a branch alone has no spine");
        let (script, poked) = tour::build_script(game, &tours, "br").unwrap();
        assert!(!poked);
        let dir = out.join("tour");
        std::fs::create_dir_all(dir.join("states")).unwrap();
        let t = tours.iter().find(|t| t.label == "br").unwrap();
        let (row, _) = tour::run_one(&roms, t, &script, &dir, false).unwrap();
        assert_eq!(row["state_hash"].as_str().unwrap(), exp.data["state_hash"].as_str().unwrap(), "{line}");
        // A branch opened from the branch: its spine's spine is in the other file.
        assert!(srv.call("session.close", &json!({})).ok());
        let mut cfg2 = cfg.clone();
        cfg2.tours.push(branch_file);
        let mut srv2 = Server::new(game, Some(&roms), cfg2);
        let a = srv2.call("session.open", &json!({"from": "tour:br@1020"}));
        assert!(a.ok(), "{:?}", a.error);
        assert_eq!(a.data["frame"], 1020);

        // A changed recording drops the cache: the next open makes it again.
        write_spine(1250);
        let mut srv3 = Server::new(game, Some(&roms), cfg);
        let a = srv3.call("session.open", &json!({"from": "tour:spn@1000"}));
        assert!(a.ok(), "{:?}", a.error);
        assert!(a.warnings.iter().any(|w| w.contains("made the spine's")), "{:?}", a.warnings);
        assert_eq!(hash_of(&a), want, "the same inputs, the same state");
        let _ = std::fs::remove_dir_all(&out);
    }

    // ------------------------------------------------------------ new verbs and options, no ROM

    #[test]
    fn the_richer_untils_parse_and_refuse() {
        let t = tour::value_table(
            "px|0x10:l:float|n\npy|0x14:l:float|n\npz|0x18:l:float|n\nscreen|0x1c:l|n\nracing|screen=7|a screen\n",
        )
        .unwrap();
        assert_eq!(t.len(), 4, "a named state is not a value");
        let s = tour::state_table("racing|screen=7|r\ntitle|screen=0x14|t\n").unwrap();
        let u = parse_until("distance px,py,pz to 1,2.5,-3 lt 20", &t, &s).unwrap();
        assert!(matches!(&u, Until::Distance { defs, to, op: "lt", d }
            if defs.len() == 3 && *to == vec![1.0, 2.5, -3.0] && *d == 20.0));
        assert!(matches!(parse_until("distance px,pz to 1,2 le 5", &t, &s), Ok(Until::Distance { .. })));
        for bad in [
            "distance px,py,pz to 1,2 lt 5",
            "distance px to 1 lt 5",
            "distance px,py to 1,2 near 5",
            "distance px,py to 1,x lt 5",
            "distance px,py,pz,px to 1,2,3,4 lt 5",
            "changes",
            "screen",
        ] {
            assert!(parse_until(bad, &t, &s).is_err(), "{bad}");
        }
        let e = parse_until("distance pxx,py to 1,2 lt 5", &t, &s).unwrap_err();
        assert!(e.message().contains("did you mean: px"), "{}", e.message());
        assert!(matches!(parse_until("changes px", &t, &s), Ok(Until::Changes { .. })));
        assert!(parse_until("changes nope", &t, &s).is_err());
        // screen: a name, a hex name, a number.
        let at = |w: &str| match parse_until(w, &t, &s) {
            Ok(Until::Value { op: "eq", v, .. }) => v,
            o => panic!("{w}: {o:?}"),
        };
        assert_eq!((at("screen racing"), at("screen title"), at("screen 9")), (7.0, 20.0, 9.0));
        let e = parse_until("screen raceing", &t, &s).unwrap_err();
        assert!(e.message().contains("did you mean: racing"), "{}", e.message());
        let e = parse_until("screen racing", &t, &[]).unwrap_err();
        assert!(e.message().contains("names no screens"), "{}", e.message());
        let no_screen = tour::value_table("px|0x10:l:float|n\n").unwrap();
        assert!(parse_until("screen 1", &no_screen, &s).unwrap_err().message().contains("no value named `screen`"));
        // The game's own table names its screens and still has all its values.
        let g = romset::games::find("topskatr").unwrap();
        let (vals, states) = (
            tour::value_table(romset::games::values(g)).unwrap(),
            tour::state_table(romset::games::values(g)).unwrap(),
        );
        assert!(vals.iter().any(|v| v.name == "screen") && vals.iter().all(|v| !v.name.contains('=')));
        assert!(states.iter().any(|s| s.name == "racing" && s.value == "screen" && s.number == 7));
        assert!(matches!(parse_until("screen racing", &vals, &states), Ok(Until::Value { v, .. }) if v == 7.0));
    }

    #[test]
    fn raw_reads_take_kinds_and_ranges() {
        let one = |s: &str| match parse_raw(s) {
            Some(Ok(RawRead::One { addr, width, kind })) => (addr, width, kind),
            o => panic!("{s}: {:?}", o.map(|r| r.is_ok())),
        };
        use tour::Kind::*;
        assert_eq!(one("0x500000"), (0x500000, 4, Unsigned));
        assert_eq!(one("0x500000:w"), (0x500000, 2, Unsigned));
        assert_eq!(one("0x500000:l:float"), (0x500000, 4, Float));
        assert_eq!(one("0x500000:float"), (0x500000, 4, Float));
        assert_eq!(one("0x500000:b:signed"), (0x500000, 1, Signed));
        assert_eq!(one("0x500000:signed:w"), (0x500000, 2, Signed));
        let range = |s: &str| match parse_raw(s) {
            Some(Ok(RawRead::Range { addr, len, elem })) => (addr, len, elem),
            o => panic!("{s}: {:?}", o.map(|r| r.is_ok())),
        };
        assert_eq!(range("0x500000:16"), (0x500000, 16, None));
        assert_eq!(range("0x500000..0x500020"), (0x500000, 32, None));
        assert_eq!(range("0x500000:16:l:float"), (0x500000, 16, Some((4, Float))));
        assert_eq!(range("0x500000:8:w"), (0x500000, 8, Some((2, Unsigned))));
        for bad in ["0x500000:w:float", "0x500000:6:l", "0x500000:0", "0x500000:99999", "0x500000:q", "0x500000:b:w"] {
            assert!(matches!(parse_raw(bad), Some(Err(_))), "{bad}");
        }
        // Not an address: a name for the table to find.
        assert!(parse_raw("speed").is_none());
        assert!(parse_raw("").is_none());
    }

    #[test]
    fn new_verbs_check_their_arguments() {
        let ok = |v: &str, a: Value| validate(find_verb(v).unwrap(), &a).map_err(|e| e.message());
        assert!(ok("session.watch", json!({})).is_ok());
        assert!(ok("session.watch", json!({"what": ["clock", "0x500000:16"], "limit": 50})).is_ok());
        assert!(ok("session.watch", json!({"what": "clock"})).unwrap_err().contains("a list of strings"));
        assert!(ok("session.watch", json!({"limit": "x"})).unwrap_err().contains("a whole number"));
        assert!(ok("session.watch", json!({"mark": 1})).unwrap_err().contains("unknown argument"));
        assert!(ok("session.disasm", json!({})).unwrap_err().contains("`addr` is needed"));
        assert!(ok("session.disasm", json!({"addr": "0x1000", "count": 4, "back": 8})).is_ok());
        assert!(ok("session.disasm", json!({"addr": 4096})).unwrap_err().contains("a string"));
        assert!(ok("session.list", json!({})).is_ok());
        assert!(ok("session.list", json!({"session": "s1"})).unwrap_err().contains("unknown argument"));
        assert!(ok("session.step", json!({"frames": 3, "follow": "tour:x", "inputs": ["start"]})).is_ok());
        assert!(ok("session.step", json!({"frames": 3, "follow": 4})).unwrap_err().contains("a string"));
        assert!(ok("session.open", json!({"from": "tour:x@5", "overlay": ["start@1-2"], "index_every": 10})).is_ok());
        assert!(ok("session.open", json!({"overlay": "start@1-2"})).unwrap_err().contains("a list of strings"));
        assert!(ok("session.open", json!({"index_every": "k"})).is_err());
        assert!(ok("session.read", json!({"names": ["0x1:float"], "format": "compact", "when": "before_poke"})).is_ok());
        assert!(ok("session.read", json!({"format": 1})).is_err());
        // A command line builds the same arguments.
        let v = find_verb("session.watch").unwrap();
        let a = args_from_pairs(v, &["what=clock".into(), "what=0x500000:16".into(), "limit=9".into()]).unwrap();
        assert_eq!(a, json!({"what": ["clock", "0x500000:16"], "limit": 9}));
        // `what=` (an empty value) is an empty list: it clears the watch.
        assert_eq!(args_from_pairs(v, &["what=".into()]).unwrap(), json!({"what": []}));
        let v = find_verb("session.open").unwrap();
        let a =
            args_from_pairs(v, &["from=tour:a@9".into(), "overlay=start@1-2".into(), "overlay=selright@3-4".into()]);
        assert_eq!(a.unwrap(), json!({"from": "tour:a@9", "overlay": ["start@1-2", "selright@3-4"]}));
    }

    #[test]
    fn a_verb_has_help_from_the_table() {
        for v in VERBS {
            let h = help(v);
            assert!(h.starts_with(v.name) && h.contains(v.about), "{}", v.name);
            for a in v.args {
                assert!(h.contains(a.name) && h.contains(a.about), "{} {}", v.name, a.name);
            }
            assert_eq!(h.matches("required").count(), v.args.iter().filter(|a| a.required).count(), "{}", v.name);
        }
        let h = help(find_verb("session.step").unwrap());
        assert!(h.contains("follow") && h.contains("until") && h.contains("whole number"), "{h}");
        assert!(help(find_verb("session.list").unwrap()).contains("no arguments"));
    }

    #[test]
    fn the_server_without_roms_refuses_new_verbs_plainly() {
        let mut srv = Server::new(romset::games::find("topskatr").unwrap(), None, Config::default());
        // No session: watch, disasm and read say so; list needs none.
        for v in ["session.watch", "session.list"] {
            let a = srv.call(v, &json!({}));
            if v == "session.list" {
                assert!(a.ok());
                assert_eq!(a.data["sessions"], json!([]));
            } else {
                assert!(a.message().contains("no session is open"), "{}", a.message());
            }
        }
        assert!(srv.call("session.disasm", &json!({"addr": "0x1000"})).message().contains("no session is open"));
        // Overlay and index options are checked before the ROM is wanted.
        let e = srv.call("session.open", &json!({"overlay": ["start@1-2"]}));
        assert_eq!(e.code, Code::BadRequest);
        assert!(e.message().contains("from=tour:LABEL@FRAME"), "{}", e.message());
        for bad in ["start", "poke=0x500000:1@5-6", "nonesuch@1-2", "start@x-y", "coin-start"] {
            let e = srv.call("session.open", &json!({"from": "tour:x@9", "overlay": [bad]}));
            assert_eq!(e.code, Code::BadRequest, "{bad}");
            assert!(e.message().contains("overlay"), "{bad}: {}", e.message());
        }
        let e = srv.call("session.open", &json!({"from": "tour:x@9", "index_every": 0}));
        assert_eq!(e.code, Code::BadRequest, "{}", e.message());
        // A good overlay gets as far as the missing ROM.
        let e = srv.call("session.open", &json!({"from": "tour:x@9", "overlay": ["start@1-2"]}));
        assert_eq!(e.code, Code::Missing);
    }

    // ------------------------------------------------------------ with the ROM

    /// The game, its ROMs and a scratch folder (None without the ROM).
    fn rom_env(tag: &str) -> Option<(&'static Game, RomSet, PathBuf)> {
        let game = romset::games::find("topskatr").unwrap();
        let rom = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rom");
        let roms = RomSet::load(game, &rom, false).ok()?;
        let out =
            Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../out/scratch/{tag}-test-{}", std::process::id()));
        std::fs::create_dir_all(&out).unwrap();
        std::env::set_var("REMODEL_LOCK_DIR", out.join("locks"));
        Some((game, roms, out))
    }

    /// The state hash `tour run` ends label `label` on, from `files`.
    fn replay_hash(game: &Game, roms: &RomSet, files: &[PathBuf], label: &str, dir: &Path) -> String {
        let tours = tour::load_paths(game, files).unwrap();
        let (script, poked) = tour::build_script(game, &tours, label).unwrap();
        std::fs::create_dir_all(dir.join("states")).unwrap();
        let t = tours.iter().find(|t| t.label == label).unwrap();
        let (row, _) = tour::run_one(roms, t, &script, dir, poked).unwrap();
        row["state_hash"].as_str().unwrap().to_string()
    }

    fn ok(a: Answer) -> Answer {
        assert!(a.ok(), "{:?} {:?}", a.error, a.data);
        a
    }

    /// A watch changes nothing and sees what the game writes (skipped
    /// without the ROM): the same state hash with and without, the clock's
    /// writes one a frame with the frame, the writer's address and the
    /// value, and a poke of the session's own is not among them.
    #[test]
    fn a_watch_changes_nothing_and_reports_the_clocks_writes() {
        let Some((game, roms, out)) = rom_env("watch") else { return };
        let cs = out.join("cs.tours");
        std::fs::write(&cs, "cs|4000f|coin-start\n").unwrap();
        let cfg = Config { tours: vec![cs.clone()], out: out.clone(), ..Config::default() };
        let mut srv = Server::new(game, Some(&roms), cfg);
        let go = crate::scene::recipes(game, None).unwrap().iter().find(|r| r.name == "race-course").unwrap().frame;
        ok(srv.call("session.open", &json!({})));
        let a = ok(srv.call("session.step", &json!({"frames": go, "follow": "tour:cs"})));
        assert_eq!(a.data["followed"], "cs");
        ok(srv.call("session.mark", &json!({"name": "go"})));
        let plain = ok(srv.call("session.step", &json!({"frames": 120, "follow": "cs"})));
        assert!(plain.data.get("watch").is_none());
        ok(srv.call("session.restore", &json!({"name": "go"})));
        let w = ok(srv.call("session.watch", &json!({"what": ["clock", "0x5830b0:4"]})));
        assert_eq!(w.data["watching"][0]["len"], 2);
        assert_eq!(w.data["watching"][1]["len"], 4);
        let watched = ok(srv.call("session.step", &json!({"frames": 120, "follow": "cs"})));
        assert_eq!(hash_of(&watched), hash_of(&plain), "a watch must not change the emulation");
        let hits = watched.data["watch"]["hits"].as_array().unwrap();
        assert!(hits.len() >= 100, "the clock is written every frame: {}", hits.len());
        assert_eq!(watched.data["watch"]["dropped"], 0);
        let named: Vec<&Value> = hits.iter().filter(|h| h["name"] == "clock").collect();
        assert!(named.len() >= 100, "{} writes to the clock by name", named.len());
        assert!(hits.iter().all(|h| h["ip"].as_str().unwrap() != "0x0" && h["frame"].as_u64().unwrap() >= go));
        let frames: Vec<u64> = hits.iter().map(|h| h["frame"].as_u64().unwrap()).collect();
        assert!(frames.windows(2).all(|p| p[0] <= p[1]) && frames[0] >= go && *frames.last().unwrap() <= go + 120);
        // The clock counts down by one a frame: the values written fall, by their raw word.
        let raw: Vec<u64> = named
            .iter()
            .map(|h| u64::from_str_radix(h["raw"].as_str().unwrap().trim_start_matches("0x"), 16).unwrap())
            .collect();
        assert!(raw.first() > raw.last(), "{raw:?}");
        // The limit keeps the first writes and counts the rest.
        ok(srv.call("session.restore", &json!({"name": "go"})));
        ok(srv.call("session.watch", &json!({"limit": 5})));
        let few = ok(srv.call("session.step", &json!({"frames": 120, "follow": "cs"})));
        assert_eq!(hash_of(&few), hash_of(&plain));
        assert_eq!(few.data["watch"]["kept"], 5);
        assert!(
            few.data["watch"]["dropped"].as_u64().unwrap() >= 95 && few.warnings.iter().any(|w| w.contains("not kept"))
        );
        // A poke of the session's own is not the program's write: the same
        // writes with a poke (of the value the word holds) as without.
        ok(srv.call("session.restore", &json!({"name": "go"})));
        ok(srv.call("session.watch", &json!({"what": ["0x5dc090:4"], "limit": 1000})));
        let free = ok(srv.call("session.step", &json!({"frames": 3, "follow": "cs"})));
        ok(srv.call("session.restore", &json!({"name": "go"})));
        ok(srv.call("session.poke", &json!({"what": "0x5dc090:7:l", "frames": 1})));
        let poked = ok(srv.call("session.step", &json!({"frames": 3, "follow": "cs"})));
        assert_eq!(poked.data["watch"]["writes"], free.data["watch"]["writes"], "{:?}", poked.data["watch"]);
        // Clearing it: the step carries no `watch`.
        ok(srv.call("session.watch", &json!({"what": []})));
        let none = ok(srv.call("session.step", &json!({"frames": 3, "follow": "cs"})));
        assert!(none.data.get("watch").is_none());
        // Disassembly: the code at the writer's address reads as i960.
        let ip = hits[0]["ip"].as_str().unwrap().to_string();
        let d = ok(srv.call("session.disasm", &json!({"addr": ip, "count": 6, "back": 8})));
        let listing = d.data["listing"].as_array().unwrap();
        assert_eq!(listing.len(), 6);
        assert!(listing.iter().any(|l| l.as_str().unwrap().starts_with("=>")), "{listing:?}");
        assert!(listing.iter().any(|l| l.as_str().unwrap().contains(" st")), "the writer is a store: {listing:?}");
        // The list shows the session and its marks.
        let l = ok(srv.call("session.list", &json!({})));
        assert_eq!(l.data["sessions"][0]["marks"], json!(["go"]));
        let _ = std::fs::remove_dir_all(&out);
    }

    /// Following a recording gives the tour's own state, and with an overlay
    /// the state of a hand-written branch (skipped without the ROM); an open
    /// with an overlay is the tour with the overlay's terms, and its export
    /// replays to its hash.
    #[test]
    fn following_a_recording_and_its_overlay_replay_as_tours() {
        let Some((game, roms, out)) = rom_env("follow") else { return };
        let spine = out.join("rec.tours");
        std::fs::write(&spine, "rec|2000f|coin-start,selright@1300-1310,jumpfront@1500-1700\n").unwrap();
        let hand = out.join("hand.tours");
        std::fs::write(
            &hand,
            "plain|1650f|coin-start,selright@1300-1310,jumpfront@1500-1700\n\
             ovl|1650f|coin-start,selright@1300-1310,jumpfront@1500-1700,selright@1200-1400\n\
             hand|1650f|from=rec@1400,jumpfront@1500-1700,selright@1401-1650\n",
        )
        .unwrap();
        let files = vec![spine.clone(), hand.clone()];
        let cfg = Config {
            tours: files.clone(),
            spines: Some(out.join("spines")),
            checkpoint_every: 400,
            index_every: 100,
            out: out.clone(),
            ..Config::default()
        };
        let mut srv = Server::new(game, Some(&roms), cfg);
        let dir = out.join("run");
        let want = |label: &str| replay_hash(game, &roms, &files, label, &dir);

        // From power-on, following the recording equals the plain tour.
        ok(srv.call("session.open", &json!({})));
        let a = ok(srv.call("session.step", &json!({"frames": 1650, "follow": "tour:rec"})));
        assert_eq!(hash_of(&a), want("plain"), "follow equals the tour's replay");
        assert_eq!(a.data["followed"], "rec");
        // In pieces, too: the log is the same and the export replays.
        ok(srv.call("session.close", &json!({})));
        ok(srv.call("session.open", &json!({})));
        for n in [500, 700, 450] {
            ok(srv.call("session.step", &json!({"frames": n, "follow": "rec"})));
        }
        let a = ok(srv.call("session.step", &json!({"frames": 1, "follow": "rec"})));
        let exp = ok(srv.call("session.export", &json!({"label": "again", "file": "again"})));
        let again = PathBuf::from(exp.data["file"].as_str().unwrap());
        assert_eq!(hash_of(&a), exp.data["state_hash"].as_str().unwrap());
        assert_eq!(
            replay_hash(game, &roms, std::slice::from_ref(&again), "again", &dir),
            hash_of(&a),
            "the followed log replays"
        );
        // An unknown recording names the near ones.
        let e = srv.call("session.step", &json!({"frames": 1, "follow": "tour:rek"}));
        assert!(e.message().contains("did you mean: rec"), "{}", e.message());
        ok(srv.call("session.close", &json!({})));

        // From a spine: follow with an overlay held on top equals the hand-written branch.
        let o = ok(srv.call("session.open", &json!({"from": "tour:rec@1400"})));
        assert_eq!(o.data["checkpoint"], 1200);
        let a = ok(srv.call("session.step", &json!({"frames": 250, "follow": "rec", "inputs": ["selright"]})));
        assert_eq!(a.data["frame"], 1650);
        assert_eq!(a.data["held"], json!(["selright"]));
        assert_eq!(hash_of(&a), want("hand"), "follow with an overlay equals a hand-written branch");
        let exp = ok(srv.call("session.export", &json!({"label": "viaf", "file": "viaf"})));
        let line = exp.data["line"].as_str().unwrap();
        assert!(line.starts_with("viaf|1650f|from=rec@1400,"), "{line}");
        let viaf = PathBuf::from(exp.data["file"].as_str().unwrap());
        assert_eq!(replay_hash(game, &roms, &[viaf], "viaf", &dir), hash_of(&a));
        ok(srv.call("session.close", &json!({})));

        // Open with an overlay: the tour with its terms added; the checkpoint is one before it.
        let o = ok(srv.call("session.open", &json!({"from": "tour:rec@1650", "overlay": ["selright@1200-1400"]})));
        assert_eq!(o.data["checkpoint"], 800, "{o:?}");
        assert_eq!(hash_of(&o), want("ovl"), "an overlaid open is the tour with the overlay");
        // Stepping on, then exporting: a branch from before the overlay, replaying to the session's hash.
        ok(srv.call("session.input", &json!({"inputs": ["start"]})));
        let a = ok(srv.call("session.step", &json!({"frames": 30})));
        let exp = ok(srv.call("session.export", &json!({"label": "viao", "file": "viao"})));
        let line = exp.data["line"].as_str().unwrap();
        assert!(line.starts_with("viao|1680f|from=rec@1199,"), "{line}");
        assert!(line.contains("selright@1200-1400"), "{line}");
        let viao = PathBuf::from(exp.data["file"].as_str().unwrap());
        assert_eq!(replay_hash(game, &roms, &[viao], "viao", &dir), hash_of(&a), "{line}");
        // An overlay that starts after the open frame changes nothing, and says so.
        ok(srv.call("session.close", &json!({})));
        let o = ok(srv.call("session.open", &json!({"from": "tour:rec@1650", "overlay": ["start@1700-1800"]})));
        assert!(o.warnings.iter().any(|w| w.contains("starts after")), "{:?}", o.warnings);
        assert_eq!(hash_of(&o), want("plain"));
        // One that runs past it says the session holds its own inputs from there.
        ok(srv.call("session.close", &json!({})));
        let o = ok(srv.call("session.open", &json!({"from": "tour:rec@1650", "overlay": ["selright@1600-1800"]})));
        assert!(o.warnings.iter().any(|w| w.contains("runs past")), "{:?}", o.warnings);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// Each `until` form stops where it should (skipped without the ROM).
    #[test]
    fn each_until_form_stops_where_expected() {
        let Some((game, roms, out)) = rom_env("until") else { return };
        let cs = out.join("cs.tours");
        std::fs::write(&cs, "cs|4000f|coin-start\n").unwrap();
        let cfg = Config { tours: vec![cs.clone()], out: out.clone(), ..Config::default() };
        let mut srv = Server::new(game, Some(&roms), cfg);
        // screen NAME: from power-on to the first frame on the racing screen.
        ok(srv.call("session.open", &json!({})));
        let a = ok(srv.call("session.step", &json!({"frames": 3600, "follow": "cs", "until": "screen racing"})));
        assert_eq!(a.data["stopped"], "until", "{a:?}");
        let at = a.data["frame"].as_u64().unwrap();
        assert!((3000..3300).contains(&at), "the race loads about frame 3150: {at}");
        assert_eq!(a.data["until_reading"], 7);
        let r = ok(srv.call("session.read", &json!({"names": ["screen"]})));
        assert_eq!(r.data["values"][0]["value"], 7);
        // One frame short of it, the screen is something else.
        ok(srv.call("session.close", &json!({})));
        ok(srv.call("session.open", &json!({})));
        let b = ok(srv.call("session.step", &json!({"frames": at - 1, "follow": "cs"})));
        assert_ne!(ok(srv.call("session.read", &json!({"names": ["screen"]}))).data["values"][0]["value"], 7, "{b:?}");
        // changes NAME: the clock runs once the race has started.
        ok(srv.call("session.step", &json!({"frames": 40, "follow": "cs"})));
        let before = ok(srv.call("session.read", &json!({"names": ["clock"]}))).data["values"][0]["value"].clone();
        let c = ok(srv.call("session.step", &json!({"frames": 200, "follow": "cs", "until": "changes clock"})));
        assert_eq!(c.data["stopped"], "until", "{c:?}");
        assert_ne!(c.data["until_reading"], before);
        assert!(c.data["frames_run"].as_u64().unwrap() <= 2, "{c:?}");
        // distance: back to a mark, and out to where the skater is 60 frames on.
        ok(srv.call("session.step", &json!({"frames": 30, "follow": "cs"})));
        ok(srv.call("session.mark", &json!({"name": "m"})));
        ok(srv.call("session.step", &json!({"frames": 60, "follow": "cs"})));
        let p = ok(srv.call("session.read", &json!({"names": ["pos-x", "pos-y", "pos-z"], "format": "compact"})));
        let (x, y, z) =
            (p.data["values"]["pos-x"].clone(), p.data["values"]["pos-y"].clone(), p.data["values"]["pos-z"].clone());
        ok(srv.call("session.restore", &json!({"name": "m"})));
        let until = format!("distance pos-x,pos-y,pos-z to {x},{y},{z} lt 0.001");
        let d = ok(srv.call("session.step", &json!({"frames": 200, "follow": "cs", "until": until})));
        assert_eq!(d.data["stopped"], "until", "{d:?}");
        assert_eq!(d.data["frames_run"], 60, "{d:?}");
        assert_eq!(d.data["until_reading"], 0.0);
        // Two coordinates: the distance over x and z alone.
        ok(srv.call("session.restore", &json!({"name": "m"})));
        let until = format!("distance pos-x,pos-z to {x},{z} lt 0.001");
        let d = ok(srv.call("session.step", &json!({"frames": 200, "follow": "cs", "until": until})));
        assert_eq!(d.data["frames_run"], 60, "{d:?}");
        // A point never reached: the step runs its frames.
        ok(srv.call("session.restore", &json!({"name": "m"})));
        let d = ok(srv.call(
            "session.step",
            &json!({"frames": 10, "follow": "cs", "until": "distance pos-x,pos-z to 99999,99999 lt 1"}),
        ));
        assert_eq!(d.data["stopped"], "frames");
        let _ = std::fs::remove_dir_all(&out);
    }

    /// Reads: floats, signed words, ranges, the compact forms, and a value a
    /// poke holds, as the frame left it and after the poke (skipped without
    /// the ROM).
    #[test]
    fn reads_take_floats_ranges_and_the_poke_timing() {
        let Some((game, roms, out)) = rom_env("read") else { return };
        let cs = out.join("cs.tours");
        std::fs::write(&cs, "cs|4000f|coin-start\n").unwrap();
        let cfg = Config { tours: vec![cs], out: out.clone(), ..Config::default() };
        let mut srv = Server::new(game, Some(&roms), cfg);
        ok(srv.call("session.open", &json!({})));
        ok(srv.call("session.step", &json!({"frames": 3400, "follow": "cs"})));
        // A named float and the same word read raw as a float.
        let r =
            ok(srv
                .call("session.read", &json!({"names": ["speed", "0x582f90:float", "0x582f90:l:float", "0x582f90"]})));
        let v = &r.data["values"];
        assert!(v[0]["value"].as_f64().unwrap() > 0.5, "{r:?}");
        assert_eq!(v[0]["value"], v[1]["value"]);
        assert_eq!(v[1]["value"], v[2]["value"]);
        assert_eq!(v[3]["raw"], v[0]["raw"]);
        // Signed: -1 is the skater object's +0xcc word.
        let r = ok(srv.call("session.read", &json!({"names": ["0x582fdc:signed", "0x582fdc:l", "skater-cc"]})));
        assert_eq!(r.data["values"][0]["value"], r.data["values"][2]["value"]);
        // A range: a hex dump, the same bytes as the words, and as floats.
        let r = ok(srv.call(
            "session.read",
            &json!({"names": ["0x582f90:8", "0x582f90..0x582f98", "0x582f90:8:l:float", "0x582f90:8:w"]}),
        ));
        let v = &r.data["values"];
        assert_eq!(v[0]["hex"].as_str().unwrap().len(), 16);
        assert_eq!(v[0]["hex"], v[1]["hex"]);
        assert_eq!(v[2]["values"].as_array().unwrap().len(), 2);
        assert_eq!(
            v[2]["values"][0],
            ok(srv.call("session.read", &json!({"names": ["speed"]}))).data["values"][0]["value"]
        );
        assert_eq!(v[3]["values"].as_array().unwrap().len(), 4);
        let e = srv.call("session.read", &json!({"names": ["0x582f90:5000"]}));
        assert_eq!(e.code, Code::BadRequest);
        let e = srv.call("session.read", &json!({"names": ["0x10000000:4"]}));
        assert!(e.message().contains("not mapped"), "{}", e.message());
        // Compact and tsv.
        let c = ok(srv.call("session.read", &json!({"names": ["clock", "speed", "0x582f90:4"], "format": "compact"})));
        assert!(c.data["values"].is_object() && c.data["values"]["clock"].is_u64(), "{c:?}");
        assert!(c.data["values"]["0x582f90:4"].is_string());
        let t = ok(srv.call("session.read", &json!({"names": ["clock", "speed"], "format": "tsv"})));
        let text = t.data["text"].as_str().unwrap();
        assert!(text.starts_with("clock\t") && text.lines().count() == 2, "{text:?}");
        assert_eq!(srv.call("session.read", &json!({"format": "xml"})).code, Code::BadRequest);
        assert_eq!(srv.call("session.read", &json!({"when": "never"})).code, Code::BadRequest);
        // The poke timing: a poke is applied at once for the frame about to run.
        let screen_now =
            ok(srv.call("session.read", &json!({"names": ["screen"]}))).data["values"][0]["value"].as_u64().unwrap();
        assert_ne!(screen_now, 13);
        let nothing = ok(srv.call("session.read", &json!({"names": ["screen"], "when": "before_poke"})));
        assert!(nothing.warnings.iter().any(|w| w.contains("no poke has applied")), "{:?}", nothing.warnings);
        ok(srv.call("session.poke", &json!({"what": "0x5dc090:13:l", "frames": 2})));
        let after = ok(srv.call("session.read", &json!({"names": ["screen", "clock"]})));
        assert_eq!(after.data["values"][0]["value"], 13, "the poke is there as soon as it is made");
        assert!(after.warnings.iter().any(|w| w.contains("before_poke")), "{:?}", after.warnings);
        assert_eq!(after.data["poked"], json!(["screen"]));
        assert_eq!(after.data["when"], "after");
        let before = ok(srv.call("session.read", &json!({"names": ["screen", "clock"], "when": "before_poke"})));
        assert_eq!(before.data["values"][0]["value"], screen_now, "as the frame left it");
        assert!(before.warnings.is_empty(), "{:?}", before.warnings);
        // A raw read of the poked bytes, and a range over them.
        let raw = ok(srv.call("session.read", &json!({"names": ["0x5dc090:b", "0x5dc08f:4"], "when": "before_poke"})));
        assert_eq!(raw.data["values"][0]["value"], screen_now & 0xff);
        // After a step the poke has applied again to the next frame: the value
        // the previous frame left is whatever the game made of it, the poked one is 13.
        ok(srv.call("session.step", &json!({"frames": 1, "follow": "cs"})));
        let after = ok(srv.call("session.read", &json!({"names": ["screen"]})));
        assert_eq!(after.data["values"][0]["value"], 13);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// Where two saved states differ: the first runs of differing bytes.
    fn diff_runs(a: &[u8], b: &[u8]) -> String {
        if a.len() != b.len() {
            return format!("{} against {} bytes", a.len(), b.len());
        }
        let (mut runs, mut total, mut i) = (Vec::new(), 0usize, 0usize);
        while i < a.len() {
            if a[i] != b[i] {
                let s = i;
                while i < a.len() && a[i] != b[i] {
                    i += 1;
                }
                total += i - s;
                if runs.len() < 8 {
                    runs.push(format!("{s:#x}+{}", i - s));
                }
            } else {
                i += 1;
            }
        }
        format!("{total} bytes differ of {}, in runs at {}", a.len(), runs.join(" "))
    }

    /// A restore after pokes is the machine a fresh replay makes, all of it:
    /// the saved state byte for byte, and the same state on after more frames
    /// (skipped without the ROM). The notes' one unexplained observation was
    /// a task missing after a restore in a session that had used pokes.
    #[test]
    fn a_restore_after_pokes_equals_a_fresh_replay() {
        let Some((game, roms, out)) = rom_env("restore") else { return };
        let cs = out.join("cs.tours");
        std::fs::write(&cs, "cs|4000f|coin-start\n").unwrap();
        let cfg = Config { tours: vec![cs], out: out.clone(), sessions: 3, ..Config::default() };
        let mut srv = Server::new(game, Some(&roms), cfg);
        // The two machines are the same, all of it: the hash, and every byte of the
        // saved state, but the render and palette RAM write counts. Those are versions
        // for a renderer and only ever grow in a copy (`Model2Bus::copy_from`), so a
        // machine restored after more frames than a fresh one ran carries bigger ones
        // (at most two small counters, 3 bytes here); they are no part of the state hash.
        let same = |srv: &mut Server, a: &str, b: &str, what: &str| {
            let (sa, ha, va) = {
                let s = srv.sessions.get_mut(a).unwrap();
                (s.m.save_state(), s.hash(), s.m.bus.video_ram_versions())
            };
            let (sb, hb, vb) = {
                let s = srv.sessions.get_mut(b).unwrap();
                (s.m.save_state(), s.hash(), s.m.bus.video_ram_versions())
            };
            assert_eq!(ha, hb, "{what}: the hash");
            assert_eq!(sa.len(), sb.len(), "{what}: the size of the saved state");
            let differing = sa.iter().zip(&sb).filter(|(x, y)| x != y).count();
            if va == vb {
                assert_eq!(differing, 0, "{what}: the machine differs from a fresh replay's: {}", diff_runs(&sa, &sb));
            } else {
                assert!(differing <= 16, "{what}: more than the two version counters differ: {}", diff_runs(&sa, &sb));
            }
        };
        let step = |srv: &mut Server, id: &str, n: u64| {
            ok(srv.call("session.step", &json!({"session": id, "frames": n, "follow": "cs"})))
        };
        // Session A runs to 3400 (in the race), marks, pokes things, runs on; session B is the fresh replay.
        let a = ok(srv.call("session.open", &json!({}))).data["session"].as_str().unwrap().to_string();
        let b = ok(srv.call("session.open", &json!({}))).data["session"].as_str().unwrap().to_string();
        step(&mut srv, &a, 3400);
        step(&mut srv, &b, 3400);
        ok(srv.call("session.mark", &json!({"session": a, "name": "m"})));
        // Pokes of a flag, a position and the clock; a step with them; a second poke; more steps.
        for what in ["0x5830b0:0x0e10:w", "0x583034:0x42c80000:l", "0x5ce960:1:l", "0x5830ac:19:l"] {
            ok(srv.call("session.poke", &json!({"session": a, "what": what, "frames": 20})));
        }
        step(&mut srv, &a, 50);
        ok(srv.call("session.poke", &json!({"session": a, "what": "0x5dc090:0:l", "frames": 5})));
        step(&mut srv, &a, 120);
        // Restore, and A must be B: every byte of the saved state, and the hash.
        ok(srv.call("session.restore", &json!({"session": a, "name": "m"})));
        same(&mut srv, &a, &b, "after the restore");
        // On, with the same inputs: the same state.
        step(&mut srv, &a, 200);
        step(&mut srv, &b, 200);
        same(&mut srv, &a, &b, "200 frames after the restore");
        // And with a poke held across the restore: a mark taken while pokes are pending.
        ok(srv.call("session.poke", &json!({"session": a, "what": "0x5830b0:0x0e10:w", "frames": 40})));
        ok(srv.call("session.poke", &json!({"session": b, "what": "0x5830b0:0x0e10:w", "frames": 40})));
        step(&mut srv, &a, 10);
        ok(srv.call("session.mark", &json!({"session": a, "name": "p"})));
        step(&mut srv, &a, 60);
        ok(srv.call("session.restore", &json!({"session": a, "name": "p"})));
        step(&mut srv, &a, 80);
        step(&mut srv, &b, 90);
        same(&mut srv, &a, &b, "a mark taken under a pending poke, restored");
        // The task list the check walks: the same words after the restore as in the replay.
        let tasks = |srv: &mut Server, id: &str| {
            ok(srv.call("session.read", &json!({"session": id, "names": ["0x582660..0x582760"], "format": "tsv"}))).data
                ["text"]
                .clone()
        };
        assert_eq!(tasks(&mut srv, &a), tasks(&mut srv, &b));
        let _ = std::fs::remove_dir_all(&out);
    }

    const TEST_MOVES: &str = "# a test table\n@speed-bands 2 4\n\
        steer-l|curving=0xf0@0-9 over 10 frames|any|w=70;dspeed=0.5[0..1]|5\n\
        steer-l|curving=0xf0@0-9 over 10 frames|airborne=0|w=70;dspeed=0.7[0.2..1]|3\n\
        hop|jumpfront@2-5 over 8 frames|any|w=60;dair=20[10..30];dheight=9[5..12]|4\n";

    /// Moves in a session (skipped without the ROM): `step move=` is the
    /// move's raw inputs frame for frame (same hash, same exported line), a
    /// move repeats under `until` and stops where it should, the cap and the
    /// refusals, and `session.moves` lists and filters.
    #[test]
    fn moves_are_steps_that_equal_their_raw_inputs() {
        let Some((game, roms, out)) = rom_env("moves") else { return };
        let cs = out.join("cs.tours");
        std::fs::write(&cs, "cs|4000f|coin-start\n").unwrap();
        let table = out.join("moves.txt");
        std::fs::write(&table, TEST_MOVES).unwrap();
        let cfg = Config { tours: vec![cs.clone()], out: out.clone(), moves: Some(table), ..Config::default() };
        let mut srv = Server::new(game, Some(&roms), cfg);
        ok(srv.call("session.open", &json!({})));
        ok(srv.call("session.step", &json!({"frames": 3600, "follow": "cs", "until": "screen racing"})));
        ok(srv.call("session.step", &json!({"frames": 200, "follow": "cs"})));
        ok(srv.call("session.mark", &json!({"name": "m"})));
        let export = |srv: &mut Server| ok(srv.call("session.export", &json!({"label": "x"}))).data.clone();
        let line = |srv: &mut Server| export(srv)["line"].clone();
        let hash = |srv: &mut Server| export(srv)["state_hash"].clone();

        // A one-input move is that input held for its frames: the same state and the same log.
        let a = ok(srv.call("session.step", &json!({"move": ["steer-l"]})));
        assert_eq!((a.data["frames_run"].clone(), a.data["repeats"].clone()), (json!(10), json!(1)), "{a:?}");
        let (hash_move, line_move) = (hash(&mut srv), line(&mut srv));
        ok(srv.call("session.restore", &json!({"name": "m"})));
        ok(srv.call("session.input", &json!({"inputs": ["curving=0xf0"]})));
        let r = ok(srv.call("session.step", &json!({"frames": 10})));
        assert_eq!(r.data["frame"], a.data["frame"]);
        // The frame after the move takes the held inputs again (none).
        ok(srv.call("session.input", &json!({"inputs": []})));
        assert_eq!(hash(&mut srv), hash_move, "the move leaves the machine as its raw inputs do");
        assert_eq!(line(&mut srv), line_move, "the same log, so the same exported tour");

        // A move with a gap: nothing for two frames, a button for four, two more of nothing.
        ok(srv.call("session.restore", &json!({"name": "m"})));
        let hop = ok(srv.call("session.step", &json!({"move": ["hop"]})));
        assert_eq!(hop.data["frames_run"], 8);
        let hop_hash = hash(&mut srv);
        ok(srv.call("session.restore", &json!({"name": "m"})));
        ok(srv.call("session.step", &json!({"frames": 2, "inputs": []})));
        ok(srv.call("session.step", &json!({"frames": 4, "inputs": ["jumpfront"]})));
        ok(srv.call("session.step", &json!({"frames": 2, "inputs": []})));
        ok(srv.call("session.input", &json!({"inputs": []})));
        assert_eq!(hash(&mut srv), hop_hash);

        // A sequence is the moves one after the other.
        ok(srv.call("session.restore", &json!({"name": "m"})));
        let seq = ok(srv.call("session.step", &json!({"move": ["steer-l", "hop"]})));
        assert_eq!(seq.data["frames_run"], 18);
        assert_eq!(seq.data["moves"], json!(["steer-l", "hop"]));

        // wait-N is frames of nothing: the same as stepping them with no inputs.
        ok(srv.call("session.restore", &json!({"name": "m"})));
        let w = ok(srv.call("session.step", &json!({"move": ["wait-3", "hop"]})));
        assert_eq!(w.data["frames_run"], 11);
        let w_hash = hash(&mut srv);
        ok(srv.call("session.restore", &json!({"name": "m"})));
        ok(srv.call("session.step", &json!({"frames": 5, "inputs": []})));
        ok(srv.call("session.step", &json!({"frames": 4, "inputs": ["jumpfront"]})));
        ok(srv.call("session.step", &json!({"frames": 2, "inputs": []})));
        ok(srv.call("session.input", &json!({"inputs": []})));
        assert_eq!(hash(&mut srv), w_hash);
        assert_eq!(srv.call("session.step", &json!({"move": ["wait-3"]})).code, Code::BadRequest);

        // until: a frame count stops mid-move; the cap stops a condition never met; the default cap is --max-step.
        let got = |a: &Answer| (a.data["stopped"].clone(), a.data["frames_run"].clone(), a.data["repeats"].clone());
        ok(srv.call("session.restore", &json!({"name": "m"})));
        let u = ok(srv.call("session.step", &json!({"move": ["steer-l"], "until": "frames 35"})));
        assert_eq!(got(&u), (json!("until"), json!(35), json!(4)), "{u:?}");
        ok(srv.call("session.restore", &json!({"name": "m"})));
        let never = "value speed lt 0.0001";
        let u = ok(srv.call("session.step", &json!({"move": ["steer-l"], "until": never, "frames": 25})));
        assert_eq!(got(&u), (json!("frames"), json!(25), json!(3)), "{u:?}");
        srv.cfg.max_step = 50;
        ok(srv.call("session.restore", &json!({"name": "m"})));
        let u = ok(srv.call("session.step", &json!({"move": ["steer-l"], "until": never})));
        assert_eq!(got(&u), (json!("frames"), json!(50), json!(5)), "the default cap is --max-step: {u:?}");
        srv.cfg.max_step = 3600;
        // A point reached after two repeats stops there, not before.
        ok(srv.call("session.restore", &json!({"name": "m"})));
        ok(srv.call("session.step", &json!({"move": ["steer-l", "steer-l"]})));
        let p = ok(srv.call("session.read", &json!({"names": ["pos-x", "pos-z"], "format": "compact"})));
        let (x, z) = (p.data["values"]["pos-x"].clone(), p.data["values"]["pos-z"].clone());
        ok(srv.call("session.restore", &json!({"name": "m"})));
        let until = format!("distance pos-x,pos-z to {x},{z} lt 0.001");
        let d = ok(srv.call("session.step", &json!({"move": ["steer-l"], "until": until})));
        assert_eq!(got(&d), (json!("until"), json!(20), json!(2)), "{d:?}");

        // Refusals: an unknown move names the near ones; moves are not mixed with inputs; frames that disagree.
        let e = srv.call("session.step", &json!({"move": ["steer-r"]}));
        assert_eq!(e.code, Code::BadRequest);
        assert!(e.message().contains("steer-l"), "{}", e.message());
        for bad in [
            json!({"move": ["hop"], "inputs": ["start"]}),
            json!({"move": ["hop"], "follow": "cs"}),
            json!({"move": ["hop"], "frames": 3}),
            json!({}),
        ] {
            assert_eq!(srv.call("session.step", &bad).code, Code::BadRequest, "{bad}");
        }

        // session.moves: all of them, the conditions given, the current ones, an effect to sort by.
        let l = ok(srv.call("session.moves", &json!({"where": "all"})));
        assert_eq!(l.data["count"], 2);
        let names: Vec<&str> =
            l.data["moves"].as_array().unwrap().iter().map(|m| m["move"].as_str().unwrap()).collect();
        assert_eq!(names, ["steer-l", "hop"], "most samples first");
        assert_eq!(l.data["moves"][0]["inputs"], json!(["curving=0xf0@0-9"]));
        assert_eq!(l.data["moves"][1]["effects"]["dair"]["median"], 20.0);
        let l = ok(srv.call("session.moves", &json!({"where": "airborne=0", "sort": "-dspeed"})));
        assert_eq!(l.data["moves"][0]["move"], "steer-l");
        assert_eq!(l.data["moves"][0]["conditions"], "airborne=0", "the row for these conditions, not the overall");
        assert_eq!(l.data["moves"][0]["effects"]["dspeed"]["median"], 0.7);
        let l = ok(srv.call("session.moves", &json!({})));
        let cur: Vec<&str> = l.data["current"].as_array().unwrap().iter().filter_map(|c| c.as_str()).collect();
        assert!(cur.contains(&"screen=racing"), "{cur:?}");
        assert_eq!(srv.call("session.moves", &json!({"sort": "dspeeed"})).code, Code::BadRequest);
        assert_eq!(srv.call("session.moves", &json!({"where": "airborne"})).code, Code::BadRequest);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// A move table that does not load, or a game with none, is a plain answer.
    #[test]
    fn no_moves_is_a_plain_answer() {
        let cfg = Config { moves: Some(PathBuf::from("/nonexistent/moves.txt")), ..Config::default() };
        let srv = Server::new(romset::games::find("topskatr").unwrap(), None, cfg);
        let e = srv.moves_table().err().unwrap();
        assert!(e.message().contains("move table does not load"), "{}", e.message());
        let srv = Server::new(romset::games::find("vf2").unwrap(), None, Config::default());
        let e = srv.moves_table().err().unwrap();
        assert_eq!(e.code, Code::Missing);
        assert!(e.message().contains("no move table"), "{}", e.message());
    }
}
