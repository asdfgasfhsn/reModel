//! `remodel serve`: a long-running server with live game sessions, speaking
//! MCP (JSON-RPC 2.0, one message a line) over standard input and output
//! and, with `--socket`, over a Unix socket; and `remodel call`, the same
//! verbs from the command line, over that socket
//! (docs/uplift/scenes-and-server.md, "The live session").
//!
//! **The MCP messages are written here, not taken from a library.** A
//! server needs `initialize`, `notifications/initialized`, `ping`,
//! `tools/list` and `tools/call`, about 100 lines over `serde_json`, which
//! the CLI already has. The Rust MCP libraries pull in an async runtime
//! and a web stack (tokio, hyper or axum for the HTTP transports) that
//! reModel has nowhere else, and they move with a protocol that is still
//! changing (three dated revisions in 2024 and 2025); the part of the
//! protocol used here has not changed between them. The tool listing comes
//! from the verb table (`session::VERBS`), not from anything the protocol
//! code knows.
//!
//! Requests are handled one at a time, in order, whichever connection they
//! arrive on: a lockstep server has nothing to run in parallel, and one
//! machine's frames do not interleave with another's drawing. The server
//! never opens a network port: the socket is a file, mode 0600.

use crate::answer::{Answer, Code};
use crate::session::{self, Config, Server};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

/// The MCP revisions this server speaks, newest first.
const PROTOCOLS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "reModel live game sessions. The game advances only when you call session.step. Start with session.open, set inputs with session.input, step, read named values with session.read, keep points with session.mark and session.restore, draw with session.render (the PNG is a file; the answer gives its path), and write what you did as a tour with session.export. Every tool answers with one envelope: ok, code, error, data, warnings, next.";

// JSON-RPC error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

/// One line in, the reply line out (None for a notification).
pub fn handle_line(server: &mut Server, line: &str) -> Option<String> {
    let reply = match serde_json::from_str::<Value>(line) {
        Err(e) => Some(rpc_error(Value::Null, PARSE_ERROR, format!("parse error: {e}"))),
        Ok(msg) => handle(server, &msg),
    };
    reply.map(|r| r.to_string())
}

fn handle(server: &mut Server, msg: &Value) -> Option<Value> {
    let Some(obj) = msg.as_object() else {
        return Some(rpc_error(
            Value::Null,
            INVALID_REQUEST,
            "a JSON-RPC message is an object (batches are not used by MCP)",
        ));
    };
    let id = obj.get("id").cloned();
    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        // A response to something the server never asked.
        return id.map(|id| rpc_error(id, INVALID_REQUEST, "no method"));
    };
    if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(rpc_error(id.unwrap_or(Value::Null), INVALID_REQUEST, "jsonrpc must be \"2.0\""));
    }
    // A notification (no id) is never answered.
    let id = id?;
    let params = obj.get("params").cloned().unwrap_or(Value::Null);
    let ok = |result: Value| json!({"jsonrpc": "2.0", "id": id, "result": result});
    Some(match method {
        "initialize" => {
            let want = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
            let version = PROTOCOLS.iter().find(|v| **v == want).unwrap_or(&PROTOCOLS[0]);
            ok(json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "remodel", "title": "reModel live sessions", "version": remodel_version::VERSION},
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => ok(json!({})),
        "tools/list" => ok(json!({"tools": session::tools()})),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(rpc_error(id, INVALID_PARAMS, "tools/call wants params.name"));
            };
            if session::find_verb(name).is_err() {
                let known: Vec<String> = session::VERBS.iter().map(|v| v.name.to_string()).collect();
                let a = Answer::unknown("tool", name, &known);
                return Some(rpc_error(id, INVALID_PARAMS, a.message()));
            }
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            if !(args.is_null() || args.is_object()) {
                return Some(rpc_error(id, INVALID_PARAMS, "params.arguments is an object"));
            }
            let a = server.call(name, &args);
            let envelope = a.to_json();
            ok(json!({
                "content": [{"type": "text", "text": envelope.to_string()}],
                "structuredContent": envelope,
                "isError": !a.ok(),
            }))
        }
        other => rpc_error(id, METHOD_NOT_FOUND, format!("method not found: {other}")),
    })
}

// ------------------------------------------------------------------ transport

type Out = Arc<Mutex<Box<dyn Write + Send>>>;

enum Msg {
    Line(String, Out),
    /// Standard input ended.
    StdinClosed,
}

static STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn pump(reader: impl BufRead, out: Out, tx: mpsc::Sender<Msg>, stdin: bool) {
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        if tx.send(Msg::Line(line, out.clone())).is_err() {
            return;
        }
    }
    if stdin {
        let _ = tx.send(Msg::StdinClosed);
    }
}

/// Removes the socket file when the server ends, however it does.
struct SocketFile(Option<PathBuf>);

impl Drop for SocketFile {
    fn drop(&mut self) {
        if let Some(p) = &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}

pub const SERVE_USAGE: &str = "\
remodel --game G serve [--socket [PATH]] [--marks N] [--max-step N] [--sessions N] [--tours FILE]... [--no-stdio]
                       [--checkpoint-every N] [--index-every K]
  MCP (JSON-RPC, a message a line) on standard input and output, and with --socket on a Unix
  socket (default out/serve/GAME.sock, mode 0600). No network port is ever opened.
  --marks N       marks a session keeps (default 16)         --max-step N   frames one step runs (default 3600)
  --sessions N    sessions open at once (default 4)           --tours FILE   a project's tours, for `from: tour:..`
  --no-stdio      serve the socket only (stop with SIGTERM)
  --checkpoint-every N / --index-every K   when an open makes a recording's spine: a checkpoint every N
                  frames (default 600), a row of values every K (default 30); `session.open` and
                  `spine.index` can also say it per call (`every`, `index_every`)
  Pictures and exported tours go to out/serve/GAME/ (--out DIR, before `serve`, moves them). Run it through tools/capped.
";

fn take(args: &mut Vec<String>, name: &str) -> Result<Option<String>, String> {
    match args.iter().position(|a| a == name) {
        None => Ok(None),
        Some(i) if i + 1 < args.len() => {
            let v = args.remove(i + 1);
            args.remove(i);
            Ok(Some(v))
        }
        Some(_) => Err(format!("{name} wants a value")),
    }
}

fn number(args: &mut Vec<String>, name: &str, default: u64, min: u64) -> Result<u64, String> {
    match take(args, name)? {
        None => Ok(default),
        Some(v) => v
            .parse::<u64>()
            .ok()
            .filter(|n| *n >= min)
            .ok_or_else(|| format!("{name} wants a number, {min} or more, got `{v}`")),
    }
}

/// The socket's default place.
pub fn default_socket(game: &str) -> PathBuf {
    PathBuf::from("out/serve").join(format!("{game}.sock"))
}

/// What `remodel serve`'s options say: the config, the socket (if any) and
/// whether standard input is left alone.
pub type ServeOpts = (Config, Option<PathBuf>, bool);

/// `remodel serve`'s options.
pub fn parse_serve(mut args: Vec<String>, game_id: &str) -> Result<ServeOpts, String> {
    {
        let mut cfg = Config::default();
        // `--socket` alone means the default place.
        let socket = match args.iter().position(|a| a == "--socket") {
            None => None,
            Some(i) => {
                let path = match args.get(i + 1).filter(|n| !n.starts_with("--")) {
                    Some(p) => {
                        let p = PathBuf::from(p);
                        args.remove(i + 1);
                        p
                    }
                    None => default_socket(game_id),
                };
                args.remove(i);
                Some(path)
            }
        };
        cfg.marks = number(&mut args, "--marks", cfg.marks as u64, 1)? as usize;
        cfg.max_step = number(&mut args, "--max-step", cfg.max_step, 1)?;
        cfg.sessions = number(&mut args, "--sessions", cfg.sessions as u64, 1)? as usize;
        cfg.checkpoint_every = number(&mut args, "--checkpoint-every", cfg.checkpoint_every, 1)?;
        cfg.index_every = number(&mut args, "--index-every", cfg.index_every, 1)?;
        while let Some(t) = take(&mut args, "--tours")?.map(PathBuf::from) {
            if !t.is_file() {
                return Err(format!("--tours {}: no such file", t.display()));
            }
            cfg.tours.push(t);
        }
        let no_stdio = match args.iter().position(|a| a == "--no-stdio") {
            Some(i) => {
                args.remove(i);
                true
            }
            None => false,
        };
        if let Some(extra) = args.first() {
            return Err(format!("serve: unexpected `{extra}`"));
        }
        if no_stdio && socket.is_none() {
            return Err("--no-stdio needs --socket".into());
        }
        cfg.out = crate::OUT_DIR.get().cloned().unwrap_or_else(|| PathBuf::from("out/serve"));
        Ok((cfg, socket, no_stdio))
    }
}

/// `remodel serve`.
pub fn run(o: &crate::Opts, roms: &romset::RomSet) -> ExitCode {
    let parsed = parse_serve(o.args[1..].to_vec(), roms.game.id);
    let (cfg, socket, no_stdio) = match parsed {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            eprint!("{SERVE_USAGE}");
            return ExitCode::from(2);
        }
    };
    let mut server = Server::new(roms.game, Some(roms), cfg);
    serve(&mut server, socket, no_stdio)
}

fn serve(server: &mut Server, socket: Option<PathBuf>, no_stdio: bool) -> ExitCode {
    // SAFETY: the handler only stores to an atomic.
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    let (tx, rx) = mpsc::channel::<Msg>();
    let mut _guard = SocketFile(None);
    if let Some(path) = &socket {
        match listen(path, tx.clone()) {
            Ok(()) => {
                _guard = SocketFile(Some(path.clone()));
                eprintln!("remodel serve: {} on {}", server.game.id, path.display());
            }
            Err(e) => {
                eprintln!("error: {}: {e}", path.display());
                return ExitCode::from(4);
            }
        }
    }
    if !no_stdio {
        let out: Out = Arc::new(Mutex::new(Box::new(std::io::stdout())));
        let tx = tx.clone();
        std::thread::spawn(move || pump(std::io::BufReader::new(std::io::stdin()), out, tx, true));
    }
    drop(tx);
    while !STOP.load(std::sync::atomic::Ordering::Relaxed) {
        match rx.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(Msg::Line(line, out)) => {
                if let Some(reply) = handle_line(server, &line) {
                    let mut w = out.lock().unwrap_or_else(|e| e.into_inner());
                    let _ = writeln!(w, "{reply}");
                    let _ = w.flush();
                }
            }
            Ok(Msg::StdinClosed) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    ExitCode::SUCCESS
}

/// Listen on `path` (a file, mode 0600): a stale socket from a server that
/// died is replaced, a live one is not.
fn listen(path: &Path, tx: mpsc::Sender<Msg>) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    if path.exists() {
        if UnixStream::connect(path).is_ok() {
            return Err("another server is listening there".into());
        }
        let _ = std::fs::remove_file(path);
    }
    // Bind where only this user can reach it, then open the mode.
    let listener = UnixListener::bind(path).map_err(|e| e.to_string())?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(conn) = conn else { continue };
            let Ok(w) = conn.try_clone() else { continue };
            let out: Out = Arc::new(Mutex::new(Box::new(w)));
            let tx = tx.clone();
            std::thread::spawn(move || pump(std::io::BufReader::new(conn), out, tx, false));
        }
    });
    Ok(())
}

// ------------------------------------------------------------------ the command line

pub const CALL_USAGE: &str = "\
remodel [--game G] call VERB [name=value ...] [--socket PATH] [--json]
remodel [--game G] session VERB [name=value ...]        (session VERB is call session.VERB)
  Run one verb on a running server (remodel serve --socket) and print its answer, the same envelope the
  MCP tool gives. A list takes its name more than once: inputs=start inputs=steer=0x80.
  --socket PATH   the server's socket (default out/serve/GAME.sock)
remodel call VERB --help   the verb's text and arguments, from the verb table (no server needed)
";

/// `remodel call VERB ...` and `remodel session VERB ...`.
pub fn call(o: &crate::Opts) -> ExitCode {
    let mut args = o.args[1..].to_vec();
    let json = args.iter().position(|a| a == "--json").map(|i| args.remove(i)).is_some();
    let a = (|| -> Result<Answer, Answer> {
        let socket = take(&mut args, "--socket").map_err(|e| Answer::fail(Code::BadRequest, e))?.map(PathBuf::from);
        let mut it = args.into_iter();
        let Some(first) = it.next().filter(|f| f != "--help" && f != "-h") else {
            print!("{CALL_USAGE}");
            let names: Vec<&str> = session::VERBS.iter().map(|v| v.name).collect();
            println!("verbs: {}", names.join(", "));
            return Err(Answer::fail(Code::BadRequest, "call: which verb?").next("remodel call session.open"));
        };
        let verb = if o.args[0] == "session" && !first.contains('.') { format!("session.{first}") } else { first };
        let v = session::find_verb(&verb)?;
        let pairs: Vec<String> = it.collect();
        if pairs.iter().any(|p| p == "--help" || p == "-h") {
            print!("{}", session::help(v));
            return Ok(Answer::done(Value::Null));
        }
        let tool_args = session::args_from_pairs(v, &pairs)?;
        session::validate(v, &tool_args)?;
        let socket = socket.unwrap_or_else(|| default_socket(&o.game_id));
        ask(&socket, &verb, &tool_args)
    })();
    match a {
        Ok(a) | Err(a) => {
            let text = if a.data.is_null() {
                String::new()
            } else if let Some(t) = a.data["text"].as_str() {
                // `session.read format=tsv`: the lines themselves.
                t.to_string()
            } else if let Some(l) = a.data["listing"].as_array() {
                // `session.disasm`: the listing.
                l.iter().filter_map(Value::as_str).map(|s| format!("{s}\n")).collect()
            } else {
                serde_json::to_string_pretty(&a.data).unwrap_or_default() + "\n"
            };
            a.emit(json, || text)
        }
    }
}

pub const SPINE_USAGE: &str = "\
remodel --game G spine index LABEL [--tours FILE]... [--every N] [--index-every K] [--dir DIR] [--json]
remodel --game G spine where LABEL [--tours FILE]... [--near NAME=V]... [--where 'NAME OP V']... [--limit N] [--gap F] [--dir DIR] [--json]
  A spine is a recording of a whole run (a tour from power-on, `--record-tour`). `index` replays it once and
  keeps, in out/GAME/spines/LABEL/ (--dir moves the root), a checkpoint every N frames (default 600; a session
  opened at tour:LABEL@F loads the nearest and steps the rest) and the game's named values every K frames
  (default 30). `where` asks the index: with --near the frames nearest the point (at least --gap frames apart,
  default 90), with only --where the stretches that match (--where screen=7, speed>3). Both take the tour
  files with --tours as many times as a branch's spine needs; `where` needs no ROM once the index is made.
";

/// `remodel spine index|where LABEL ...`: the `spine.index` and `spine.where` verbs without a server.
pub fn spine_main(o: &crate::Opts) -> ExitCode {
    let mut args = o.args[1..].to_vec();
    let json = args.iter().position(|a| a == "--json").map(|i| args.remove(i)).is_some();
    let a = (|| -> Result<Answer, Answer> {
        let bad = |e: String| Answer::fail(Code::BadRequest, e).next("remodel spine --help");
        let game = romset::games::find(&o.game_id).map_err(|e| bad(e.to_string()))?;
        let mut cfg = Config::default();
        let mut call = serde_json::Map::new();
        let put_list = |m: &mut serde_json::Map<String, Value>, k: &str, v: String| {
            if let Some(a) = m.entry(k).or_insert_with(|| json!([])).as_array_mut() {
                a.push(json!(v));
            }
        };
        let mut positional = Vec::new();
        let mut it = args.into_iter();
        while let Some(a) = it.next() {
            let mut val = |name: &str| it.next().ok_or_else(|| bad(format!("{name} wants a value")));
            match a.as_str() {
                "--help" | "-h" => {
                    print!("{SPINE_USAGE}");
                    return Err(bad("spine: which verb (index, where)?".into()));
                }
                "--tours" => {
                    let t = PathBuf::from(val("--tours")?);
                    if !t.is_file() {
                        return Err(bad(format!("--tours {}: no such file", t.display())));
                    }
                    cfg.tours.push(t);
                }
                "--dir" => cfg.spines = Some(PathBuf::from(val("--dir")?)),
                "--every" | "--limit" | "--gap" | "--index-every" => {
                    let v = val(&a)?;
                    let n: u64 = v.parse().map_err(|_| bad(format!("{a} wants a number, got `{v}`")))?;
                    call.insert(a[2..].replace('-', "_"), json!(n));
                }
                "--near" => put_list(&mut call, "near", val("--near")?),
                "--where" => put_list(&mut call, "where", val("--where")?),
                f if f.starts_with("--") => return Err(bad(format!("spine: unknown option {f}"))),
                _ => positional.push(a),
            }
        }
        let (verb, label) = match positional.as_slice() {
            [v, l] if v == "index" || v == "where" => (format!("spine.{v}"), l.clone()),
            _ => return Err(bad("want: spine index LABEL, or spine where LABEL".into())),
        };
        call.insert("spine".into(), json!(label));
        cfg.out = crate::OUT_DIR.get().cloned().unwrap_or_else(|| PathBuf::from("out/serve"));
        let roms = romset::RomSet::load(game, &o.rom_dir, false).ok();
        if verb == "spine.index" && roms.is_none() {
            return Err(Answer::fail(
                Code::Missing,
                format!("no ROMs for {}: a spine is made by running the game", game.id),
            )
            .next(format!("put the {} ROMs in {}", game.id, o.rom_dir.display())));
        }
        let mut server = Server::new(game, roms.as_ref(), cfg);
        let args = Value::Object(call);
        let v = session::find_verb(&verb)?;
        session::validate(v, &args)?;
        Ok(server.call(&verb, &args))
    })();
    let a = a.unwrap_or_else(|a| a);
    a.emit(json, || {
        let d = &a.data;
        let mut s = String::new();
        if let Some(rs) = d["results"].as_array() {
            for r in rs {
                if d["kind"] == "nearest" {
                    s.push_str(&format!("frame {:>6}  distance {:<10.4}  {}", r["frame"], r["distance"].as_f64().unwrap_or(0.0), r["open"].as_str().unwrap_or("")));
                    let vals: Vec<String> = r["values"].as_object().into_iter().flatten().map(|(k, v)| format!("{k}={v}")).collect();
                    s.push_str(&format!("\n    {}\n", vals.join(" ")));
                } else {
                    s.push_str(&format!("frames {:>6} to {:<6} ({} rows)  {}\n", r["from"], r["to"], r["rows"], r["open"].as_str().unwrap_or("")));
                }
            }
        } else if !d.is_null() {
            s.push_str(&format!(
                "spine {}: {} frames, {} checkpoints every {} ({} MB), {} index rows every {} ({} values), {:.1} s\n  {}\n",
                d["spine"].as_str().unwrap_or(""), d["frames"], d["checkpoints"], d["checkpoint_every"], d["checkpoint_mb"],
                d["index_rows"], d["index_every"], d["values"], d["seconds"].as_f64().unwrap_or(0.0), d["dir"].as_str().unwrap_or("")
            ));
        }
        s
    })
}

/// One `tools/call` on a server's socket.
fn ask(socket: &Path, verb: &str, args: &Value) -> Result<Answer, Answer> {
    use std::os::unix::net::UnixStream;
    let mut conn = UnixStream::connect(socket).map_err(|e| {
        Answer::fail(Code::Missing, format!("no server at {}: {e}", socket.display()))
            .next("remodel serve --game GAME --socket   (through tools/capped)")
    })?;
    let req = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": verb, "arguments": args}});
    writeln!(conn, "{req}").map_err(|e| Answer::fail(Code::Internal, e.to_string()))?;
    let mut line = String::new();
    std::io::BufReader::new(&conn).read_line(&mut line).map_err(|e| Answer::fail(Code::Internal, e.to_string()))?;
    let reply: Value =
        serde_json::from_str(&line).map_err(|e| Answer::fail(Code::Internal, format!("the server's reply: {e}")))?;
    if let Some(e) = reply.get("error") {
        return Err(Answer::fail(
            Code::BadRequest,
            e["message"].as_str().unwrap_or("the server refused the call").to_string(),
        ));
    }
    let env = &reply["result"]["structuredContent"];
    let code = match env["code"].as_u64() {
        Some(1) => Code::GateFailed,
        Some(2) => Code::BadRequest,
        Some(3) => Code::Missing,
        Some(4) => Code::Internal,
        _ => Code::Done,
    };
    let mut a = if code == Code::Done {
        Answer::done(env["data"].clone())
    } else {
        Answer::fail(code, env["error"].as_str().unwrap_or("").to_string()).with_data(env["data"].clone())
    };
    for w in env["warnings"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        a = a.warn(w);
    }
    if let Some(n) = env["next"].as_str() {
        a = a.next(n);
    }
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::VERBS;

    fn server() -> Server<'static> {
        Server::new(romset::games::find("topskatr").unwrap(), None, Config::default())
    }

    fn opts(args: &[&str]) -> Result<ServeOpts, String> {
        parse_serve(args.iter().map(|s| s.to_string()).collect(), "topskatr")
    }

    #[test]
    fn serve_options_include_the_spine_granularity() {
        let (c, socket, no_stdio) = opts(&[]).unwrap();
        assert_eq!((c.checkpoint_every, c.index_every), (crate::spine::CHECKPOINT_EVERY, crate::spine::INDEX_EVERY));
        assert!(socket.is_none() && !no_stdio);
        let (c, socket, _) =
            opts(&["--socket", "--index-every", "10", "--checkpoint-every", "300", "--marks", "3"]).unwrap();
        assert_eq!((c.checkpoint_every, c.index_every, c.marks), (300, 10, 3));
        assert_eq!(socket.unwrap(), default_socket("topskatr"));
        for bad in [
            &["--index-every", "0"][..],
            &["--index-every", "x"],
            &["--index-every"],
            &["--checkpoint-every", "0"],
            &["--checkpoint-every", "-3"],
            &["--no-stdio"],
            &["--bogus"],
        ] {
            assert!(opts(bad).is_err(), "{bad:?}");
        }
        assert!(opts(&["--index-every", "0"]).unwrap_err().contains("--index-every wants a number, 1 or more"));
        assert!(SERVE_USAGE.contains("--index-every K") && SERVE_USAGE.contains("--checkpoint-every N"));
    }

    #[test]
    fn a_verbs_help_is_printable_for_every_verb() {
        // `remodel call VERB --help` prints this; it must name every argument.
        for v in VERBS {
            let h = session::help(v);
            assert!(h.contains(&session::usage(v)), "{}", v.name);
        }
        assert!(CALL_USAGE.contains("--help") || CALL_USAGE.contains("VERB"));
    }

    fn rpc(s: &mut Server, msg: Value) -> Value {
        serde_json::from_str(&handle_line(s, &msg.to_string()).expect("a reply")).unwrap()
    }

    #[test]
    fn initialize_lists_tools_and_answers_ping() {
        let mut s = server();
        let r = rpc(
            &mut s,
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}}),
        );
        assert_eq!(r["id"], 1);
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(r["result"]["serverInfo"]["name"], "remodel");
        assert!(r["result"]["capabilities"]["tools"].is_object());
        // An unknown revision gets the newest the server speaks.
        let r = rpc(
            &mut s,
            json!({"jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {"protocolVersion": "1999-01-01"}}),
        );
        assert_eq!(r["result"]["protocolVersion"], PROTOCOLS[0]);
        // The initialized notification is not answered.
        assert!(handle_line(&mut s, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
        assert_eq!(rpc(&mut s, json!({"jsonrpc": "2.0", "id": "p", "method": "ping"}))["result"], json!({}));
        let r = rpc(&mut s, json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}));
        let tools = r["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        let table: Vec<&str> = VERBS.iter().map(|v| v.name).collect();
        assert_eq!(names, table, "tools/list is the verb table");
        for t in tools {
            assert_eq!(t["inputSchema"]["type"], "object");
            assert!(!t["description"].as_str().unwrap().is_empty());
        }
        let step = tools.iter().find(|t| t["name"] == "session.step").unwrap();
        assert_eq!(step["inputSchema"]["required"], json!(["frames"]));
        assert_eq!(step["inputSchema"]["properties"]["frames"]["type"], "integer");
        assert_eq!(step["inputSchema"]["properties"]["inputs"]["type"], "array");
    }

    #[test]
    fn bad_calls_get_proper_json_rpc_errors() {
        let mut s = server();
        let parse = serde_json::from_str::<Value>(&handle_line(&mut s, "{not json").unwrap()).unwrap();
        assert_eq!(parse["error"]["code"], PARSE_ERROR);
        assert_eq!(parse["id"], Value::Null);
        let r = rpc(&mut s, json!({"jsonrpc": "2.0", "id": 7, "method": "frobnicate"}));
        assert_eq!(r["error"]["code"], METHOD_NOT_FOUND);
        assert_eq!(r["id"], 7);
        let r =
            rpc(&mut s, json!({"jsonrpc": "2.0", "id": 8, "method": "tools/call", "params": {"name": "session.stp"}}));
        assert_eq!(r["error"]["code"], INVALID_PARAMS);
        assert!(r["error"]["message"].as_str().unwrap().contains("session.step"), "{r}");
        let r = rpc(&mut s, json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {}}));
        assert_eq!(r["error"]["code"], INVALID_PARAMS);
        let r = rpc(
            &mut s,
            json!({"jsonrpc": "2.0", "id": 10, "method": "tools/call", "params": {"name": "session.step", "arguments": [1]}}),
        );
        assert_eq!(r["error"]["code"], INVALID_PARAMS);
        let r = rpc(&mut s, json!([1, 2]));
        assert_eq!(r["error"]["code"], INVALID_REQUEST);
        let r = rpc(&mut s, json!({"id": 11, "method": "ping"}));
        assert_eq!(r["error"]["code"], INVALID_REQUEST);
    }

    #[test]
    fn a_tool_that_fails_answers_with_the_envelope_and_is_an_error() {
        let mut s = server();
        let r = rpc(
            &mut s,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "session.step", "arguments": {}}}),
        );
        assert_eq!(r["result"]["isError"], true);
        let env = &r["result"]["structuredContent"];
        assert_eq!(env["ok"], false);
        assert_eq!(env["code"], 2);
        assert!(env["error"].as_str().unwrap().contains("`frames` is needed"));
        assert!(env["next"].as_str().unwrap().contains("remodel call session.step"));
        // The text content is the same envelope.
        let text: Value = serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(&text, env);
        // No session open: a session verb says so.
        let a = s.call("session.mark", &json!({"name": "a"}));
        assert_eq!(a.code, Code::BadRequest);
        assert!(a.message().contains("no session is open"));
        // Without ROMs the server cannot open one, and says what is missing.
        let a = s.call("session.open", &json!({}));
        assert_eq!(a.code, Code::Missing);
        // A wrong game is refused before anything else.
        let a = s.call("session.open", &json!({"game": "vf2"}));
        assert_eq!(a.code, Code::BadRequest);
        assert!(a.message().contains("runs topskatr"));
        let a = s.call("session.open", &json!({"from": "moon"}));
        assert_eq!(a.code, Code::Missing, "the missing ROM comes first");
    }

    #[test]
    fn the_command_line_and_the_tools_share_the_table() {
        // Every verb's arguments build from key=value pairs, and what they
        // build validates: the CLI cannot take what the tool does not.
        for v in VERBS {
            let pairs: Vec<String> = v
                .args
                .iter()
                .filter(|a| a.required)
                .map(|a| match a.ty {
                    session::Ty::Int => format!("{}=3", a.name),
                    _ => format!("{}=x", a.name),
                })
                .collect();
            let args = session::args_from_pairs(v, &pairs).unwrap();
            session::validate(v, &args).unwrap_or_else(|e| panic!("{}: {:?}", v.name, e.error));
        }
    }

    #[test]
    fn a_socket_serves_one_call_and_is_removed() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("remodel-serve-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.sock");
        let (tx, rx) = mpsc::channel::<Msg>();
        listen(&path, tx).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        // A second server on the same socket is refused.
        assert!(listen(&path, mpsc::channel::<Msg>().0).unwrap_err().contains("another server"));
        let client = std::thread::spawn({
            let path = path.clone();
            move || ask(&path, "session.mark", &json!({"name": "a"}))
        });
        let mut s = server();
        let Msg::Line(line, out) = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap() else { panic!() };
        let reply = handle_line(&mut s, &line).unwrap();
        writeln!(out.lock().unwrap(), "{reply}").unwrap();
        let a = client.join().unwrap().unwrap();
        assert_eq!(a.code, Code::BadRequest);
        assert!(a.message().contains("no session is open"));
        // No server there: exit code 3 and the command that starts one.
        drop(_guard_for(&path));
        let e = ask(&path, "session.close", &json!({})).unwrap_err();
        assert_eq!(e.code, Code::Missing);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn _guard_for(p: &Path) -> SocketFile {
        SocketFile(Some(p.to_path_buf()))
    }
}
