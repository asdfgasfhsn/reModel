//! `remodel project ...` and `remodel pack build`: the studio workspace as a
//! project that survives hand-over (docs/uplift-studio.md, "Projects").
//!
//! The workspace's files stay the truth about images (an image in `pack/` is
//! approved, in `draft/` drafted, with its hash in `status/approved.tsv` and
//! `status/checked.tsv`), so every `studio` command keeps working on a
//! project. What this adds, plain text beside them:
//!
//! - `assets.tsv`: one sorted row per selected asset with its status, the
//!   hash it was approved or checked at, who and when, and a note. The
//!   status is *derived* from the files each time (`stale`: the approved
//!   image's hash is no longer the recorded one; `rejected`: a draft turned
//!   down, until a different draft is written), so the file never drifts;
//!   `sync` rewrites it.
//! - `log.tsv`: every change made through these verbs, appended: time,
//!   actor, verb, asset, from, to, note.
//! - `provenance/ID.txt`: how an image was made (tool, prompt, hashes of its
//!   inputs).
//! - `project.txt` gains `format`, `made-with`, `approvers`, `version` and
//!   the pack's `author`, `licence`, `ai`, `program`, `depends`, `conflicts`.
//!
//! The verbs write under a short lock (`.project.lock`), take the actor from
//! `REMODEL_ACTOR` and are idempotent.

use super::*;
use crate::answer::{Answer, Code};
use crate::gates;
use crate::scene::Args as Opt;
use serde_json::{json, Value};

/// The project format this build writes (`project.txt`'s `format`).
const FORMAT: u32 = 1;
const ASSETS_HEADER: &str = "id\tname\tkind\tcategory\tstatus\tcolour\tsource\thash\tby\tdate\tnote";
const LOG_HEADER: &str = "time\tactor\tverb\tasset\tfrom\tto\tnote";
/// What `.gitignore` keeps out: ROM-derived and scratch.
const IGNORE: &str = "# ROM-derived or scratch: never committed\noriginal/\nmap/\ncatalogue-maps/\ncache/\nwork/\ncompare/\nbuild/\nregions/*/original.png\n.project.lock\n";
/// Folders `project init --from-studio` copies. The rest is scratch or
/// derived (a private binary, comparison sheets, the runner's folders).
const COPIED: [&str; 8] = ["briefs", "prompts", "status", "regions", "draft", "pack", "reports", "provenance"];
/// Copied only with `--with-cache`: they are the texture map and the
/// originals, which come from the ROM.
const CACHE: [&str; 3] = ["original", "map", "catalogue-maps"];

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum St {
    Todo,
    Drafted,
    Checked,
    Approved,
    Rejected,
    Stale,
}

impl St {
    const ALL: [St; 6] = [St::Todo, St::Drafted, St::Checked, St::Approved, St::Rejected, St::Stale];

    pub fn name(self) -> &'static str {
        match self {
            St::Todo => "todo",
            St::Drafted => "drafted",
            St::Checked => "checked",
            St::Approved => "approved",
            St::Rejected => "rejected",
            St::Stale => "stale",
        }
    }

    fn parse(s: &str) -> Option<St> {
        St::ALL.into_iter().find(|x| x.name() == s)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub category: String,
    pub status: St,
    pub colour: String,
    pub source: String,
    pub hash: String,
    pub by: String,
    pub date: String,
    pub note: String,
}

impl Row {
    fn json(&self) -> Value {
        json!({"id": self.id, "name": self.name, "kind": self.kind, "category": self.category,
               "status": self.status.name(), "colour": self.colour, "source": self.source,
               "hash": self.hash, "by": self.by, "date": self.date, "note": self.note})
    }

    fn line(&self) -> String {
        let c = |s: &str| if s.is_empty() { "-".to_string() } else { clean(s) };
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            self.id,
            c(&self.name),
            c(&self.kind),
            c(&self.category),
            self.status.name(),
            c(&self.colour),
            c(&self.source),
            c(&self.hash),
            c(&self.by),
            c(&self.date),
            c(&self.note)
        )
    }
}

fn dash(s: &str) -> String {
    if s == "-" {
        String::new()
    } else {
        s.to_string()
    }
}

fn bad(e: impl Into<String>) -> Answer {
    Answer::fail(Code::BadRequest, e)
}

fn io(e: impl std::fmt::Display) -> Answer {
    Answer::fail(Code::Internal, e.to_string())
}

/// `YYYY-MM-DDTHH:MM:SSZ`.
fn now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let s = secs % 86400;
    format!("{}T{:02}:{:02}:{:02}Z", today(), s / 3600, s % 3600 / 60, s % 60)
}

/// The name a writing verb records: `REMODEL_ACTOR`.
fn actor() -> Result<String, Answer> {
    std::env::var("REMODEL_ACTOR").ok().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).ok_or_else(|| {
        bad("REMODEL_ACTOR is not set: a writing verb records who made the change")
            .next("REMODEL_ACTOR=NAME remodel project ...")
    })
}

/// A short lock on the project's files, so two agents' verbs do not
/// interleave their writes. Held until dropped; one left behind by a killed
/// run is taken over after a minute.
struct Lock(PathBuf);

impl Lock {
    fn take(p: &Project) -> Result<Lock, Answer> {
        let path = p.p(".project.lock");
        for _ in 0..300 {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(Lock(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let old = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|d| d.as_secs() > 60);
                    if old {
                        let _ = std::fs::remove_file(&path);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(e) => return Err(io(format!("{}: {e}", path.display()))),
            }
        }
        Err(Answer::fail(Code::Internal, format!("{} is held by another run", path.display())))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl Project {
    fn write_meta(&self, meta: &BTreeMap<String, String>) -> Result<(), String> {
        let mut s = String::new();
        for (k, v) in meta {
            let _ = writeln!(s, "{k}\t{}", clean(v));
        }
        std::fs::write(self.p("project.txt"), s).map_err(|e| e.to_string())
    }

    fn approvers(&self) -> Vec<String> {
        self.meta()
            .get("approvers")
            .map(|a| a.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
            .unwrap_or_default()
    }

    /// The rows of `assets.tsv` as last written.
    fn recorded(&self) -> BTreeMap<String, Row> {
        let mut m = BTreeMap::new();
        for l in std::fs::read_to_string(self.p("assets.tsv")).unwrap_or_default().lines().skip(1) {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() < 11 {
                continue;
            }
            let Some(status) = St::parse(f[4]) else { continue };
            m.insert(
                f[0].to_string(),
                Row {
                    id: f[0].into(),
                    name: dash(f[1]),
                    kind: dash(f[2]),
                    category: dash(f[3]),
                    status,
                    colour: dash(f[5]),
                    source: dash(f[6]),
                    hash: dash(f[7]),
                    by: dash(f[8]),
                    date: dash(f[9]),
                    note: dash(f[10]),
                },
            );
        }
        m
    }

    /// Every selected asset with the status its files give it now.
    /// `recorded` supplies who, when and the note for a row whose status and
    /// hash have not changed since, and a rejection (which no file records).
    pub(super) fn derive(&self) -> Vec<Row> {
        let approved = self.hashes("approved.tsv");
        let checked = self.hashes("checked.tsv");
        let ledger = self.ledger();
        let recorded = self.recorded();
        let arts = self.art_records();
        let file_hash = |dir: &str, id: &str| {
            std::fs::read(self.p(dir).join(format!("{id}.png"))).ok().map(|b| gates::hash_bytes(&b))
        };
        let mut out: Vec<Row> = self
            .selection()
            .into_iter()
            .map(|s| {
                let (pack, draft) = (file_hash("pack", &s.name), file_hash("draft", &s.name));
                let prev = recorded.get(&s.name);
                let (status, hash) = match (pack, draft) {
                    (Some(h), _) => match approved.get(&s.name) {
                        Some(&a) if a != h => (St::Stale, a),
                        Some(&a) => (St::Approved, a),
                        None => (St::Approved, h),
                    },
                    (None, Some(d)) => {
                        let hex = gates::hash_hex(d);
                        if prev.is_some_and(|p| p.status == St::Rejected && p.hash == hex) {
                            (St::Rejected, d)
                        } else if checked.get(&s.name) == Some(&d) {
                            (St::Checked, d)
                        } else {
                            (St::Drafted, d)
                        }
                    }
                    (None, None) => (St::Todo, 0),
                };
                let hash = if status == St::Todo { String::new() } else { gates::hash_hex(hash) };
                let (by, date, note) = match prev {
                    Some(o) if o.status == status && o.hash == hash => (o.by.clone(), o.date.clone(), o.note.clone()),
                    _ => (String::new(), String::new(), String::new()),
                };
                let source = match ledger.get(&s.name) {
                    _ if status == St::Todo => String::new(),
                    Some((tool, _, _)) => source_of(tool).to_string(),
                    None => "hand".to_string(),
                };
                let colour = arts.get(&s.name).map(|r| r.mode.clone()).unwrap_or_default();
                Row {
                    id: s.name.clone(),
                    name: prev.map(|p| p.name.clone()).unwrap_or_default(),
                    kind: s.kind,
                    category: s.category,
                    status,
                    colour,
                    source,
                    hash,
                    by,
                    date,
                    note,
                }
            })
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// Rewrite `assets.tsv` from the files, with `set` giving who, when and
    /// the note for named rows (a verb just did something to them).
    fn sync(&self, set: &HashMap<String, (String, String, String)>) -> Result<Vec<Row>, String> {
        let mut rows = self.derive();
        for r in &mut rows {
            if let Some((by, date, note)) = set.get(&r.id) {
                r.by = by.clone();
                r.date = date.clone();
                r.note = note.clone();
            }
        }
        let mut s = format!("{ASSETS_HEADER}\n");
        for r in &rows {
            s += &r.line();
            s.push('\n');
        }
        // Not rewritten when unchanged, so a clean tree stays clean in git.
        if std::fs::read_to_string(self.p("assets.tsv")).ok().as_deref() != Some(s.as_str()) {
            std::fs::write(self.p("assets.tsv"), s).map_err(|e| e.to_string())?;
        }
        Ok(rows)
    }

    /// Append lines to `log.tsv`: (verb, asset, from, to, note).
    fn log(&self, actor: &str, items: &[(&str, &str, &str, &str, String)]) -> Result<(), String> {
        if items.is_empty() {
            return Ok(());
        }
        let path = self.p("log.tsv");
        let mut s = if path.is_file() { String::new() } else { format!("{LOG_HEADER}\n") };
        let t = now();
        for (verb, asset, from, to, note) in items {
            let _ = writeln!(s, "{t}\t{actor}\t{verb}\t{asset}\t{from}\t{to}\t{}", clean(note));
        }
        use std::io::Write as _;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut f| f.write_all(s.as_bytes()))
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    fn log_tail(&self, n: usize) -> Vec<Value> {
        let t = std::fs::read_to_string(self.p("log.tsv")).unwrap_or_default();
        let lines: Vec<&str> = t.lines().skip(1).collect();
        lines[lines.len().saturating_sub(n)..]
            .iter()
            .map(|l| {
                let f: Vec<&str> = l.split('\t').collect();
                let g = |i: usize| f.get(i).copied().unwrap_or("");
                json!({"time": g(0), "actor": g(1), "verb": g(2), "asset": g(3), "from": g(4), "to": g(5), "note": g(6)})
            })
            .collect()
    }

    pub(super) fn provenance_path(&self, id: &str) -> PathBuf {
        self.p("provenance").join(format!("{id}.txt"))
    }

    /// Set keys in an asset's provenance file (created if there is none),
    /// keeping the order of what is there.
    fn provenance_set(&self, id: &str, kv: &[(&str, String)]) -> Result<(), String> {
        let path = self.provenance_path(id);
        let mut lines: Vec<(String, String)> = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.split_once('\t').map(|(k, v)| (k.to_string(), v.to_string())))
            .collect();
        for (k, v) in kv {
            match lines.iter_mut().find(|l| l.0 == *k) {
                Some(l) => l.1 = clean(v),
                None => lines.push((k.to_string(), clean(v))),
            }
        }
        std::fs::create_dir_all(self.p("provenance")).map_err(|e| e.to_string())?;
        let s: String = lines.iter().map(|(k, v)| format!("{k}\t{v}\n")).collect();
        std::fs::write(&path, s).map_err(|e| e.to_string())
    }
}

/// `hand`, `recipe`, `generated` or `imported`, from the first word of the
/// tool a draft was recorded with.
fn source_of(tool: &str) -> &'static str {
    let t = tool.trim().to_lowercase();
    if t.starts_with("style") || t.starts_with("text") || t.starts_with("variants") || t.starts_with("bake") {
        "recipe"
    } else if t.starts_with("codex") || t.starts_with("draft") || t.starts_with("gen") {
        "generated"
    } else {
        "imported"
    }
}

fn project_of(a: &Opt) -> Project {
    Project { dir: PathBuf::from(a.one("--project").unwrap_or(".")) }
}

fn need(p: &Project) -> Result<(), Answer> {
    p.require().map_err(|e| {
        Answer::fail(Code::Missing, e).next("remodel project init --project DIR   (or --from-studio WORKSPACE)")
    })
}

// ---------------------------------------------------------------- verbs

const USAGE: &str = "\
remodel project VERB [--project DIR] [--json]    (docs/uplift-studio.md, \"Projects\")
  init [--from-studio WORKSPACE] [--with-cache] [--approver NAME,..] [--name N] [--pack-version V] [--author A]
       [--licence L] [--ai none|assisted|generated] [--replace]
       Make a project, or bring an existing studio folder (this one, or WORKSPACE copied to --project DIR) up to the
       manifest: project.txt format, assets.tsv, log.tsv, provenance/, .gitignore. WORKSPACE's private binaries, runs and
       scratch are not copied, nor, without --with-cache, its originals and texture map (they come from the ROM).
  status [--status S,..] [--category C] [--limit N]   counts by status, what waits, the stale and rejected, the log
  approve ID.. | --category C [--override \"REASON\"]   checked drafts to approved, after the asset gates
  reject ID.. --reason \"TEXT\"                         a draft turned down; a new draft returns it to drafted
Writing verbs need REMODEL_ACTOR; approve and reject need it to be in project.txt's approvers when it lists any.
Exit codes: 0 done, 1 a gate failed, 2 the request was wrong, 3 something is missing.
remodel pack build [--project DIR] [--out DIR] [--replace]   the pack to ship, from the approved images
";

pub fn project_main(o: &crate::Opts) -> std::process::ExitCode {
    let rest = &o.args[1..];
    let json = rest.iter().any(|a| a == "--json");
    match project_run(&o.game_id, rest) {
        Ok((a, text)) => a.emit(json, || text),
        Err(a) => a.emit(json, || USAGE.to_string()),
    }
}

pub fn build_main(rest: &[String], json: bool) -> std::process::ExitCode {
    match build_run(rest) {
        Ok((a, text)) => a.emit(json, || text),
        Err(a) => a.emit(json, || USAGE.to_string()),
    }
}

fn project_run(game: &str, rest: &[String]) -> Result<(Answer, String), Answer> {
    const VERBS: [&str; 4] = ["init", "status", "approve", "reject"];
    let Some(sub) = rest.iter().find(|a| !a.starts_with("--")) else {
        return Err(bad("project wants a verb: init, status, approve, reject").next("remodel project --help"));
    };
    if !VERBS.contains(&sub.as_str()) {
        let v: Vec<String> = VERBS.iter().map(|s| s.to_string()).collect();
        return Err(Answer::unknown("project verb", sub, &v));
    }
    let mut skipped = false;
    let argv: Vec<String> = rest
        .iter()
        .filter(|a| {
            if !skipped && *a == sub {
                skipped = true;
                return false;
            }
            true
        })
        .cloned()
        .collect();
    match sub.as_str() {
        "init" => init_cmd(game, &argv),
        "status" => status_cmd(&argv),
        "approve" => approve_cmd(&argv),
        _ => reject_cmd(&argv),
    }
}

// ----------------------------------------------------------------- init

fn init_cmd(game: &str, argv: &[String]) -> Result<(Answer, String), Answer> {
    let a = Opt::parse(
        argv,
        &["--json", "--with-cache", "--replace"],
        &["--project", "--from-studio", "--approver", "--name", "--pack-version", "--author", "--licence", "--ai"],
    )?;
    let p = project_of(&a);
    let actor = actor()?;
    let mut notes: Vec<String> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    let mut copied = 0usize;
    if let Some(from) = a.one("--from-studio") {
        let src = PathBuf::from(from);
        if !src.join("project.txt").is_file() {
            return Err(Answer::fail(Code::Missing, format!("{from}: not a studio workspace (no project.txt)")));
        }
        if p.p("project.txt").is_file() && !a.has("--replace") {
            return Err(bad(format!("{} is already a project: --replace imports over it", p.dir.display())));
        }
        if std::fs::canonicalize(&src).ok() == std::fs::canonicalize(&p.dir).ok() {
            return Err(bad("--from-studio is the project itself: leave it out to bring this folder up to date"));
        }
        copied = import_studio(&p, &src, a.has("--with-cache"), &mut skipped, &mut notes).map_err(io)?;
    } else if !p.p("project.txt").is_file() {
        let mut v: Vec<String> = vec![p.dir.display().to_string(), "--game".into(), game.into()];
        if let Some(n) = a.one("--name") {
            v.extend(["--name".into(), n.into()]);
        }
        let args = Args::parse(&v).map_err(io)?;
        init(&p, &args).map_err(io)?;
    }
    p.require().map_err(io)?;
    let _lock = Lock::take(&p)?;
    // The manifest keys.
    let mut meta = p.meta();
    meta.insert("format".into(), FORMAT.to_string());
    meta.insert("made-with".into(), format!("reModel {}", env!("CARGO_PKG_VERSION")));
    for (opt, key) in [
        ("--name", "name"),
        ("--pack-version", "version"),
        ("--author", "author"),
        ("--licence", "licence"),
        ("--approver", "approvers"),
    ] {
        if let Some(v) = a.one(opt) {
            meta.insert(key.into(), v.to_string());
        }
    }
    if let Some(ai) = a.one("--ai") {
        if !model2::pack::AI_LABELS.contains(&ai) {
            let l: Vec<String> = model2::pack::AI_LABELS.iter().map(|s| s.to_string()).collect();
            return Err(Answer::unknown("--ai", ai, &l));
        }
        meta.insert("ai".into(), ai.to_string());
    }
    meta.entry("version".into()).or_insert_with(|| "0.1.0".into());
    p.write_meta(&meta).map_err(io)?;
    for d in ["draft", "pack", "provenance"] {
        std::fs::create_dir_all(p.p(d)).map_err(io)?;
    }
    if !p.p(".gitignore").is_file() {
        std::fs::write(p.p(".gitignore"), IGNORE).map_err(io)?;
    }
    // Pack images with no recorded hash are taken as approved as they are
    // (so a later edit is `stale`), and each draft or approved image gets a
    // provenance file from the ledger if it has none.
    let approved = p.hashes("approved.tsv");
    let mut trust = Vec::new();
    for n in png_names(&p.p("pack")) {
        if !approved.contains_key(&n) && p.selection().iter().any(|s| s.name == n) {
            if let Ok(b) = std::fs::read(p.p("pack").join(format!("{n}.png"))) {
                trust.push((n, Some(gates::hash_bytes(&b))));
            }
        }
    }
    p.record_hashes("approved.tsv", &trust).map_err(io)?;
    let made = write_provenance(&p, a.one("--from-studio").map(Path::new), None).map_err(io)?;
    let rows = p.sync(&HashMap::new()).map_err(io)?;
    let counts = counts_of(&rows);
    p.log(
        &actor,
        &[(
            if a.one("--from-studio").is_some() { "import" } else { "init" },
            "*",
            "",
            "",
            format!(
                "{} assets: {}; {} hashes recorded as approved; {} provenance files",
                rows.len(),
                counts_text(&counts),
                trust.len(),
                made
            ),
        )],
    )
    .map_err(io)?;
    let data = json!({
        "project": p.dir.display().to_string(), "format": FORMAT, "assets": rows.len(), "counts": counts_json(&counts),
        "copied_files": copied, "hashes_recorded": trust.len(), "provenance_written": made,
        "skipped": skipped, "notes": notes,
    });
    let mut text = format!("project       {} ({} assets: {})\n", p.dir.display(), rows.len(), counts_text(&counts));
    if copied > 0 {
        let _ = writeln!(text, "imported      {copied} files");
    }
    for s in &skipped {
        let _ =
            writeln!(text, "not copied    {} ({})", s["path"].as_str().unwrap_or(""), s["why"].as_str().unwrap_or(""));
    }
    let _ =
        writeln!(text, "recorded      {} approved hashes, {made} provenance files, assets.tsv, log.tsv", trust.len());
    let mut ans = Answer::done(data).next("remodel project status --json");
    if p.approvers().is_empty() {
        ans = ans.warn("project.txt lists no approvers: anyone may approve (`project init --approver NAME`)");
    }
    for n in notes {
        ans = ans.warn(n);
    }
    Ok((ans, text))
}

fn dir_size(p: &Path) -> u64 {
    let Ok(m) = std::fs::symlink_metadata(p) else { return 0 };
    if m.is_dir() {
        std::fs::read_dir(p).map(|d| d.flatten().map(|e| dir_size(&e.path())).sum()).unwrap_or(0)
    } else {
        m.len()
    }
}

/// Copy what a project is out of a studio workspace; returns the files copied.
fn import_studio(
    dest: &Project,
    src: &Path,
    with_cache: bool,
    skipped: &mut Vec<Value>,
    notes: &mut Vec<String>,
) -> Result<usize, String> {
    std::fs::create_dir_all(&dest.dir).map_err(|e| e.to_string())?;
    let mut copied = 0usize;
    let count = |p: &Path| -> usize {
        fn walk(p: &Path) -> usize {
            match std::fs::read_dir(p) {
                Ok(d) => d.flatten().map(|e| if e.path().is_dir() { walk(&e.path()) } else { 1 }).sum(),
                Err(_) => 1,
            }
        }
        walk(p)
    };
    let mut entries: Vec<_> = std::fs::read_dir(src).map_err(|e| e.to_string())?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        let path = e.path();
        let (to, is_dir) = (dest.dir.join(&name), path.is_dir());
        let wanted = if is_dir {
            COPIED.contains(&name.as_str()) || (with_cache && CACHE.contains(&name.as_str()))
        } else {
            // Text files at the top: project.txt, selection.tsv, notes, recipes.
            name != "project.txt"
                && [".txt", ".tsv", ".md"].iter().any(|x| name.ends_with(x))
                && std::fs::metadata(&path).map(|m| m.len() < (4 << 20)).unwrap_or(false)
        };
        if wanted {
            copied += count(&path);
            if is_dir {
                copy_dir(&path, &to)?;
            } else {
                copy_file(&path, &to)?;
            }
        } else if name != "project.txt" {
            let why = if CACHE.contains(&name.as_str()) {
                "from the ROM: --with-cache copies it"
            } else if is_dir {
                "scratch, a private binary or derived output"
            } else {
                "not a project file"
            };
            let mb = (dir_size(&path) as f64 / 1e6 * 10.0).round() / 10.0;
            skipped.push(json!({"path": name, "why": why, "mb": mb}));
        }
    }
    // project.txt: the capture folder is an absolute path into the machine
    // the workspace was made on, so it does not travel.
    let from = std::fs::read_to_string(src.join("project.txt")).map_err(|e| e.to_string())?;
    let mut meta = BTreeMap::new();
    for l in from.lines() {
        if let Some((k, v)) = l.split_once('\t') {
            meta.insert(k.to_string(), v.to_string());
        }
    }
    if let Some(c) = meta.remove("capture") {
        let rel = Path::new(&c).is_relative();
        if rel {
            meta.insert("capture".into(), c);
        } else {
            notes.push("project.txt: the absolute `capture` path was dropped (a project is relative): pass --capture-dir where a command needs the capture".into());
        }
    }
    meta.insert(
        "imported-from".into(),
        format!("{} ({})", src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), today()),
    );
    dest.write_meta(&meta)?;
    // A texture map that was a link to a folder outside the workspace is
    // not followed unless asked: say so.
    if !with_cache && src.join("map").exists() {
        notes.push("the texture map (map/) and originals were not copied: commands that read them (the gates that compare with the original, `known`) say info or warn until `--with-cache`".into());
    }
    Ok(copied)
}

/// Provenance files for what has a ledger line and has none yet: the tool,
/// the prompt, hashes of its inputs (the prompt, the original when the
/// workspace has it), the date. Returns how many were written.
fn write_provenance(p: &Project, workspace: Option<&Path>, only: Option<&[String]>) -> Result<usize, String> {
    let ledger = p.ledger();
    let hash = |f: PathBuf| std::fs::read(f).ok().map(|b| gates::hash_hex(gates::hash_bytes(&b)));
    let mut made = 0;
    for (id, (tool, prompt, date)) in &ledger {
        let has_tool =
            std::fs::read_to_string(p.provenance_path(id)).is_ok_and(|t| t.lines().any(|l| l.starts_with("tool\t")));
        if has_tool || only.is_some_and(|o| !o.contains(id)) {
            continue;
        }
        let mut kv: Vec<(&str, String)> = vec![("tool", tool.clone()), ("source", source_of(tool).into())];
        kv.push(("prompt", prompt.clone()));
        if let Some(h) = hash(p.p("prompts").join(prompt)) {
            kv.push(("prompt-hash", h));
        }
        let original = [workspace.map(|w| w.join("original")), Some(p.p("original"))]
            .into_iter()
            .flatten()
            .map(|d| d.join(format!("{id}.png")))
            .find(|f| f.is_file());
        if let Some(h) = original.and_then(hash) {
            kv.push(("original-hash", h));
        }
        kv.push(("date", date.clone()));
        if let Some(h) = hash(p.p("draft").join(format!("{id}.png"))) {
            kv.push(("draft-hash", h));
        }
        p.provenance_set(id, &kv)?;
        made += 1;
    }
    Ok(made)
}

// --------------------------------------------------------------- status

fn counts_of(rows: &[Row]) -> BTreeMap<St, usize> {
    let mut m: BTreeMap<St, usize> = St::ALL.into_iter().map(|s| (s, 0)).collect();
    for r in rows {
        *m.entry(r.status).or_default() += 1;
    }
    m
}

fn counts_json(c: &BTreeMap<St, usize>) -> Value {
    Value::Object(c.iter().map(|(k, v)| (k.name().to_string(), json!(v))).collect())
}

fn counts_text(c: &BTreeMap<St, usize>) -> String {
    c.iter().filter(|(_, &n)| n > 0).map(|(k, n)| format!("{n} {}", k.name())).collect::<Vec<_>>().join(", ")
}

fn parse_statuses(list: &str) -> Result<Vec<St>, Answer> {
    list.split(',')
        .map(|s| {
            St::parse(s).ok_or_else(|| {
                let known: Vec<String> = St::ALL.iter().map(|s| s.name().to_string()).collect();
                Answer::unknown("status", s, &known)
            })
        })
        .collect()
}

fn status_cmd(argv: &[String]) -> Result<(Answer, String), Answer> {
    let a = Opt::parse(argv, &["--json"], &["--project", "--status", "--category", "--limit"])?;
    let p = project_of(&a);
    need(&p)?;
    let want = a.one("--status").map(parse_statuses).transpose()?;
    let limit: usize =
        a.one("--limit").map(|l| l.parse().map_err(|_| bad("--limit wants a number"))).transpose()?.unwrap_or(100);
    let rows = p.derive();
    let cat = a.one("--category");
    if let Some(c) = cat {
        if !rows.iter().any(|r| r.category == c) {
            let cats: BTreeSet<String> = rows.iter().map(|r| r.category.clone()).collect();
            return Err(Answer::unknown("category", c, &cats.into_iter().collect::<Vec<_>>()));
        }
    }
    let counts = counts_of(&rows);
    let shown: Vec<&Row> = rows
        .iter()
        .filter(|r| want.as_ref().is_none_or(|w| w.contains(&r.status)) && cat.is_none_or(|c| r.category == c))
        .collect();
    let ids = |s: St| -> Vec<&str> { rows.iter().filter(|r| r.status == s).map(|r| r.id.as_str()).take(50).collect() };
    let rejected: Vec<Value> = rows
        .iter()
        .filter(|r| r.status == St::Rejected)
        .take(50)
        .map(|r| json!({"asset": r.id, "reason": r.note, "by": r.by}))
        .collect();
    let mut by_cat: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();
    for r in &rows {
        *by_cat
            .entry(if r.category.is_empty() { "-" } else { &r.category })
            .or_default()
            .entry(r.status.name())
            .or_default() += 1;
    }
    let unrecorded = {
        let rec = p.hashes("approved.tsv");
        rows.iter().filter(|r| r.status == St::Approved && !rec.contains_key(&r.id)).count()
    };
    let meta = p.meta();
    let data = json!({
        "project": meta.get("name"), "game": meta.get("game"), "format": meta.get("format"),
        "version": meta.get("version"),
        "actor": std::env::var("REMODEL_ACTOR").ok(), "approvers": p.approvers(),
        "counts": counts_json(&counts), "categories": by_cat,
        "assets": shown.iter().take(limit).map(|r| r.json()).collect::<Vec<_>>(),
        "listed": shown.len().min(limit), "matching": shown.len(),
        "waiting": {"to_approve": ids(St::Checked), "to_approve_count": counts[&St::Checked],
                    "to_draft": counts[&St::Todo] + counts[&St::Drafted]},
        "stale": ids(St::Stale), "rejected": rejected, "unrecorded_approvals": unrecorded,
        "log": p.log_tail(20),
    });
    let mut text = format!(
        "project       {} (game {}, format {})\nassets        {}: {}\n",
        meta.get("name").map(String::as_str).unwrap_or("?"),
        meta.get("game").map(String::as_str).unwrap_or("?"),
        meta.get("format").map(String::as_str).unwrap_or("none yet: `project init`"),
        rows.len(),
        counts_text(&counts)
    );
    let _ = writeln!(text, "{:<14}{}", "category", St::ALL.map(|s| format!("{:>9}", s.name())).join(""));
    for (c, m) in &by_cat {
        let _ = writeln!(text, "{c:<14}{}", St::ALL.map(|s| format!("{:>9}", m.get(s.name()).unwrap_or(&0))).join(""));
    }
    for (label, s) in [("stale", St::Stale), ("rejected", St::Rejected), ("to approve", St::Checked)] {
        let v = ids(s);
        if !v.is_empty() {
            let _ = writeln!(
                text,
                "{label:<14}{}{}",
                v.iter().take(8).copied().collect::<Vec<_>>().join(" "),
                if v.len() > 8 { " ..." } else { "" }
            );
        }
    }
    if unrecorded > 0 {
        let _ = writeln!(text, "unrecorded    {unrecorded} approved images have no hash (`project init` records them)");
    }
    let mut ans = Answer::done(data);
    if counts[&St::Stale] > 0 {
        ans = ans.warn(format!("{} approved image(s) changed since approval (stale)", counts[&St::Stale]));
    }
    Ok((ans.next("remodel pack check --json"), text))
}

// ------------------------------------------------------ approve, reject

/// The rows named (a name, its 16-digit key) or `--category`, of `rows`.
fn pick<'a>(a: &Opt, rows: &'a [Row]) -> Result<Vec<&'a Row>, Answer> {
    let cat = a.one("--category");
    if a.pos.is_empty() && cat.is_none() {
        return Err(bad("which assets? ID... or --category C"));
    }
    let mut out: Vec<&Row> = Vec::new();
    for n in &a.pos {
        match rows.iter().find(|r| name_matches(&r.id, n)) {
            Some(r) => out.push(r),
            None => {
                let known: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
                let near: Vec<String> = crate::answer::near_matches(n, &known).into_iter().take(6).collect();
                return Err(Answer::unknown("asset", n, &near).next("remodel project status --status checked"));
            }
        }
    }
    if let Some(c) = cat {
        let in_cat: Vec<&Row> = rows.iter().filter(|r| r.category == c).collect();
        if in_cat.is_empty() {
            let cats: BTreeSet<String> = rows.iter().map(|r| r.category.clone()).collect();
            return Err(Answer::unknown("category", c, &cats.into_iter().collect::<Vec<_>>()));
        }
        for r in in_cat {
            if !out.iter().any(|o| o.id == r.id) {
                out.push(r);
            }
        }
    }
    Ok(out)
}

fn may_approve(p: &Project, actor: &str) -> Result<(), Answer> {
    let ap = p.approvers();
    if ap.is_empty() || ap.iter().any(|x| x == actor) {
        Ok(())
    } else {
        Err(bad(format!("{actor} is not an approver of this project ({})", ap.join(", ")))
            .next("an approver runs this, or `project init --approver NAME` adds one"))
    }
}

fn approve_cmd(argv: &[String]) -> Result<(Answer, String), Answer> {
    let a = Opt::parse(argv, &["--json"], &["--project", "--category", "--override"])?;
    let p = project_of(&a);
    need(&p)?;
    let actor = actor()?;
    may_approve(&p, &actor)?;
    let over = a.one("--override").map(str::trim);
    if over == Some("") {
        return Err(bad("--override wants a reason: --override \"why this may be approved\""));
    }
    let _lock = Lock::take(&p)?;
    let rows = p.derive();
    let picked = pick(&a, &rows)?;
    let named = !a.pos.is_empty();
    // What each picked asset is, and what stops it.
    let mut candidates: Vec<&Row> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    let mut wrong: Vec<String> = Vec::new();
    for r in &picked {
        let why = match r.status {
            St::Checked => None,
            St::Drafted if over.is_some() => None,
            St::Approved => Some("already approved"),
            St::Drafted => Some("not checked: `remodel asset check` it first, or --override with a reason"),
            St::Todo => Some("has no draft"),
            St::Rejected => Some("rejected: a new draft returns it to drafted"),
            St::Stale => Some("stale: the approved image changed; replace it from a draft, or restore it"),
        };
        match why {
            None => candidates.push(r),
            Some(w) => {
                // An asset named outright that cannot be approved is the
                // request's fault, except one already approved.
                if named && a.pos.iter().any(|n| name_matches(&r.id, n)) && r.status != St::Approved {
                    wrong.push(format!("{}: {w}", r.id));
                }
                skipped.push(json!({"asset": r.id, "why": w}));
            }
        }
    }
    if !wrong.is_empty() {
        return Err(bad(wrong.join("; ")).next("remodel project status --status checked"));
    }
    // The asset gates again, on the drafts as they are now.
    let names: Vec<String> = candidates.iter().map(|r| r.id.clone()).collect();
    let refused = if names.is_empty() { Vec::new() } else { super::refusals(&p.dir, &names)? };
    let refused_names: HashSet<&str> = refused.iter().map(|c| c.name.as_str()).collect();
    let gate_text = |c: &super::asset_gates::Checked| {
        c.gates
            .iter()
            .filter(|g| g.verdict == gates::Verdict::Error)
            .map(|g| format!("{} {}", g.gate, g.rule))
            .collect::<Vec<_>>()
            .join("; ")
    };
    let mut blocked: Vec<Value> = Vec::new();
    let mut overridden: Vec<(String, String)> = Vec::new();
    for c in &refused {
        let g: Vec<&str> = c.gates.iter().filter(|g| g.verdict == gates::Verdict::Error).map(|g| g.gate).collect();
        match over {
            Some(_) => overridden.push((c.name.clone(), g.join(","))),
            None => blocked.push(json!({"asset": c.name, "why": format!("gate at error: {}", gate_text(c)),
                "gates": c.gates.iter().filter(|g| g.verdict == gates::Verdict::Error).map(gates::GateResult::to_json).collect::<Vec<_>>()})),
        }
    }
    if !blocked.is_empty() && named {
        let list: Vec<String> = blocked.iter().map(|b| b["asset"].as_str().unwrap_or("").to_string()).collect();
        let data = json!({"approved": [], "skipped": skipped, "blocked": blocked});
        return Ok((
            Answer::gate(format!("{} asset(s) have a gate at error: {}", list.len(), list.join(", ")), data)
                .next("fix the draft, or approve with --override \"REASON\""),
            blocked
                .iter()
                .map(|b| format!("{}: {}\n", b["asset"].as_str().unwrap_or(""), b["why"].as_str().unwrap_or("")))
                .collect(),
        ));
    }
    skipped.extend(blocked.iter().map(|b| json!({"asset": b["asset"], "why": b["why"]})));
    let ok: Vec<&Row> =
        candidates.iter().filter(|r| !refused_names.contains(r.id.as_str()) || over.is_some()).copied().collect();
    // How each was made (the ledger line, if it has one), then the approval.
    let ids: Vec<String> = ok.iter().map(|r| r.id.clone()).collect();
    write_provenance(&p, None, Some(&ids)).map_err(io)?;
    // Copy, record the hash, log.
    let mut records: Vec<(String, Option<u64>)> = Vec::new();
    let mut set: HashMap<String, (String, String, String)> = HashMap::new();
    let mut log: Vec<(&str, &str, &str, &str, String)> = Vec::new();
    let date = today();
    for r in &ok {
        let d = p.p("draft").join(format!("{}.png", r.id));
        let bytes = std::fs::read(&d).map_err(|e| io(format!("{}: {e}", d.display())))?;
        copy_file(&d, &p.p("pack").join(format!("{}.png", r.id))).map_err(io)?;
        let h = gates::hash_bytes(&bytes);
        records.push((r.id.clone(), Some(h)));
        let note = overridden
            .iter()
            .find(|o| o.0 == r.id)
            .map(|o| format!("override: {} (gates at error: {})", over.unwrap_or(""), o.1))
            .or_else(|| over.filter(|_| r.status == St::Drafted).map(|o| format!("override: {o} (not checked)")))
            .unwrap_or_default();
        set.insert(r.id.clone(), (actor.clone(), date.clone(), note.clone()));
        log.push(("approve", &r.id, r.status.name(), "approved", note));
        p.provenance_set(
            &r.id,
            &[("approved-hash", gates::hash_hex(h)), ("approved-by", actor.clone()), ("approved-date", date.clone())],
        )
        .map_err(io)?;
    }
    p.record_hashes("approved.tsv", &records).map_err(io)?;
    p.log(&actor, &log).map_err(io)?;
    p.sync(&set).map_err(io)?;
    let approved: Vec<Value> = ok.iter().map(|r| json!({"asset": r.id, "hash": gates::hash_hex(records.iter().find(|x| x.0 == r.id).and_then(|x| x.1).unwrap_or(0))})).collect();
    let mut text = format!("approved      {} assets by {actor}\n", ok.len());
    for s in skipped.iter().take(12) {
        let _ =
            writeln!(text, "skipped       {}: {}", s["asset"].as_str().unwrap_or(""), s["why"].as_str().unwrap_or(""));
    }
    if skipped.len() > 12 {
        let _ = writeln!(text, "skipped       ... and {} more", skipped.len() - 12);
    }
    let data = json!({"approved": approved, "skipped": skipped, "overridden": overridden.iter().map(|o| json!({"asset": o.0, "gates": o.1})).collect::<Vec<_>>()});
    let mut ans = Answer::done(data).next("remodel pack check --json");
    for (n, g) in &overridden {
        ans = ans.warn(format!("{n} approved over a gate at error ({g}) with the reason given"));
    }
    Ok((ans, text))
}

fn reject_cmd(argv: &[String]) -> Result<(Answer, String), Answer> {
    let a = Opt::parse(argv, &["--json"], &["--project", "--reason"])?;
    let p = project_of(&a);
    need(&p)?;
    let actor = actor()?;
    may_approve(&p, &actor)?;
    let reason = a
        .one("--reason")
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .ok_or_else(|| bad("reject wants --reason \"TEXT\""))?;
    if a.pos.is_empty() {
        return Err(bad("which assets? ID..."));
    }
    let _lock = Lock::take(&p)?;
    let rows = p.derive();
    let picked = pick(&a, &rows)?;
    let mut done: Vec<Value> = Vec::new();
    let mut set = HashMap::new();
    let mut log: Vec<(&str, &str, &str, &str, String)> = Vec::new();
    let mut records = Vec::new();
    let date = today();
    for r in &picked {
        let draft = p.p("draft").join(format!("{}.png", r.id));
        if !draft.is_file() {
            return Err(bad(format!("{}: there is no draft to reject (status {})", r.id, r.status.name())));
        }
        // A rejected image is not approved: the approved copy goes, so the
        // draft is what stands rejected.
        let k = p.p("pack").join(format!("{}.png", r.id));
        if k.is_file() {
            std::fs::remove_file(&k).map_err(io)?;
            records.push((r.id.clone(), None));
        }
        let h = std::fs::read(&draft).map(|b| gates::hash_hex(gates::hash_bytes(&b))).map_err(io)?;
        // The rejection is a row that no file records, so it is written in
        // `assets.tsv` first and `derive` finds it there by its hash.
        let mut rec = p.recorded();
        let mut row = (*r).clone();
        row.status = St::Rejected;
        row.hash = h;
        row.by = actor.clone();
        row.date = date.clone();
        row.note = reason.to_string();
        rec.insert(row.id.clone(), row);
        write_recorded(&p, &rec).map_err(io)?;
        set.insert(r.id.clone(), (actor.clone(), date.clone(), reason.to_string()));
        log.push(("reject", &r.id, r.status.name(), "rejected", reason.to_string()));
        done.push(json!({"asset": r.id, "from": r.status.name()}));
    }
    p.record_hashes("approved.tsv", &records).map_err(io)?;
    p.log(&actor, &log).map_err(io)?;
    p.sync(&set).map_err(io)?;
    let text = format!("rejected      {} assets by {actor}: {reason}\n", done.len());
    Ok((Answer::done(json!({"rejected": done, "reason": reason})).next("write a new draft with --replace"), text))
}

fn write_recorded(p: &Project, rows: &BTreeMap<String, Row>) -> Result<(), String> {
    let mut s = format!("{ASSETS_HEADER}\n");
    for r in rows.values() {
        s += &r.line();
        s.push('\n');
    }
    std::fs::write(p.p("assets.tsv"), s).map_err(|e| e.to_string())
}

// ----------------------------------------------------------- pack build

fn build_run(rest: &[String]) -> Result<(Answer, String), Answer> {
    let mut skipped = false;
    let argv: Vec<String> = rest
        .iter()
        .filter(|a| {
            if !skipped && *a == "build" {
                skipped = true;
                return false;
            }
            true
        })
        .cloned()
        .collect();
    let a = Opt::parse(&argv, &["--json", "--replace"], &["--project", "--out"])?;
    let p = project_of(&a);
    need(&p)?;
    let out = a.one("--out").map(PathBuf::from).unwrap_or_else(|| p.p("build"));
    if out.join("pack.txt").is_file() && !a.has("--replace") {
        return Err(bad(format!("{} holds a pack already: --replace builds over it", out.display())));
    }
    let _lock = Lock::take(&p)?;
    std::fs::create_dir_all(&out).map_err(io)?;
    // What is approved, by its hash: a stale image is not shipped.
    let rows = p.derive();
    let mut stale: Vec<&str> = Vec::new();
    let mut names: Vec<&str> = Vec::new();
    for r in &rows {
        match r.status {
            St::Approved => names.push(&r.id),
            St::Stale => stale.push(&r.id),
            _ => {}
        }
    }
    // Rebuilt from nothing: an image no longer approved is not left behind.
    for n in png_names(&out) {
        let _ = std::fs::remove_file(out.join(format!("{n}.png")));
    }
    for n in &names {
        copy_file(&p.p("pack").join(format!("{n}.png")), &out.join(format!("{n}.png"))).map_err(io)?;
    }
    let n_regions = super::regions::write_pack(&p, &out).map_err(io)?;
    let meta = p.meta();
    let mut m = model2::pack::Manifest {
        format: 2,
        keys: 1,
        name: meta.get("name").cloned(),
        game: meta.get("game").cloned(),
        version: meta.get("version").cloned(),
        author: meta.get("author").cloned(),
        licence: meta.get("licence").cloned(),
        ai: meta.get("ai").cloned(),
        program: meta.get("program").cloned(),
        ..model2::pack::Manifest::NONE
    };
    for (k, list) in [("depends", &mut m.depends), ("conflicts", &mut m.conflicts)] {
        if let Some(v) = meta.get(k) {
            list.extend(v.split(';').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()));
        }
    }
    if n_regions > 0 {
        m.uses.push("regions".into());
    }
    if names.iter().any(|n| n.starts_with("art_")) {
        if p.needs_artworks() {
            m.needs.push("artworks".into());
        } else {
            m.uses.push("artworks".into());
        }
    }
    let manifest = m.to_text().replacen(
        "# reModel pack (docs/texture-packs.md, \"Pack format\")",
        "# reModel pack, written by `remodel pack build`",
        1,
    );
    std::fs::write(out.join("pack.txt"), manifest).map_err(io)?;
    let files = png_names(&out);
    let mut index = String::from("# one file name a line, for the browser\n");
    for n in &files {
        let _ = writeln!(index, "{n}.png");
    }
    std::fs::write(out.join("index.txt"), index).map_err(io)?;
    // The look is a pack file.
    let look = p.p(model2::look::FILE);
    if look.is_file() {
        copy_file(&look, &out.join(model2::look::FILE)).map_err(io)?;
    }
    let bytes: u64 =
        files.iter().map(|n| std::fs::metadata(out.join(format!("{n}.png"))).map(|m| m.len()).unwrap_or(0)).sum();
    let data = json!({"out": out.display().to_string(), "files": files.len(), "bytes": bytes, "approved": names.len(),
                      "regions": n_regions, "stale_left_out": stale, "look": look.is_file()});
    let text = format!(
        "pack          {} images ({:.1} MB) and pack.txt in {}{}\n",
        files.len(),
        bytes as f64 / 1e6,
        out.display(),
        if stale.is_empty() { String::new() } else { format!("; {} stale left out", stale.len()) }
    );
    let mut ans = Answer::done(data).next(format!("remodel pack-lint {}", out.display()));
    if !stale.is_empty() {
        ans = ans.warn(format!(
            "{} approved image(s) changed since approval and were left out: {}",
            stale.len(),
            stale.iter().take(5).copied().collect::<Vec<_>>().join(", ")
        ));
    }
    Ok((ans, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 4]) -> Vec<u8> {
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                rgba.extend_from_slice(&f(x, y));
            }
        }
        png::encode_rgba(w, h, &rgba)
    }

    const A: &str = "tex_0123456789abcdef_16x16";
    const B: &str = "tex_00000000000000b2_16x16";

    /// A studio workspace with two selected textures: A drafted and checked
    /// (a draft that keeps the original's cut-out), B approved long ago with
    /// no hash record, plus the clutter an import leaves behind.
    fn workspace(tag: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/scratch").join(format!("project-test-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["original", "draft", "pack", "status", "prompts", "bin", "work"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        std::fs::write(dir.join("project.txt"), "name\tws\ngame\ttopskatr\ncapture\t/gone/worktree/capture\n").unwrap();
        std::fs::write(
            dir.join("selection.tsv"),
            format!("{SEL_HEADER}\n{A}\ttex\t0\t-\t100\twall\tr\n{B}\ttex\t0\t-\t90\tfloor\tr\n"),
        )
        .unwrap();
        let cut = |x: usize, y: usize| x < 4 && y < 4;
        for n in [A, B] {
            std::fs::write(
                dir.join(format!("original/{n}.png")),
                picture(16, 16, |x, y| [(x * 16) as u8, (y * 16) as u8, 90, if cut(x, y) { 0 } else { 255 }]),
            )
            .unwrap();
        }
        let restyled = |keep: bool| {
            picture(32, 32, |x, y| {
                [255 - (x * 8) as u8, (y * 8) as u8, 200, if keep && x < 8 && y < 8 { 0 } else { 255 }]
            })
        };
        std::fs::write(dir.join(format!("draft/{A}.png")), restyled(true)).unwrap();
        std::fs::write(dir.join(format!("pack/{B}.png")), restyled(true)).unwrap();
        std::fs::write(dir.join("status/status.txt"), format!("{A}\tstyle: recipes.txt\t{A}.txt\t2026-11-18\n"))
            .unwrap();
        std::fs::write(dir.join(format!("prompts/{A}.txt")), "a prompt").unwrap();
        std::fs::write(dir.join("bin/remodel"), "binary").unwrap();
        dir
    }

    fn run(args: &[&str]) -> Result<(Answer, String), Answer> {
        project_run("topskatr", &args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    fn a_workspace_is_imported_and_its_work_survives() {
        let ws = workspace("ws");
        let dest = ws.with_file_name("project-test-dest");
        let _ = std::fs::remove_dir_all(&dest);
        let (ws_s, dest_s) = (ws.display().to_string(), dest.display().to_string());
        std::env::set_var("REMODEL_ACTOR", "lead");
        let (ans, _) = run(&["init", "--from-studio", &ws_s, "--project", &dest_s, "--approver", "lead"]).unwrap();
        assert!(ans.ok(), "{:?}", ans.error);
        // Scratch and the private binary are not copied; the originals are
        // not either, without --with-cache; the absolute capture path is gone.
        assert!(!dest.join("bin").exists() && !dest.join("work").exists() && !dest.join("original").exists());
        let meta = std::fs::read_to_string(dest.join("project.txt")).unwrap();
        assert!(!meta.contains("/gone/") && meta.contains("format\t1") && meta.contains("approvers\tlead"), "{meta}");
        assert!(dest.join(".gitignore").is_file() && dest.join("log.tsv").is_file());
        assert_eq!(ans.data["skipped"].as_array().unwrap().len(), 3, "{}", ans.data["skipped"]);
        // A is a draft, B approved by hash now; provenance came from the ledger.
        let rows = Project { dir: dest.clone() }.recorded();
        assert_eq!(rows[A].status, St::Drafted);
        assert_eq!(rows[B].status, St::Approved);
        assert_eq!(rows[A].source, "recipe");
        let prov = std::fs::read_to_string(dest.join(format!("provenance/{A}.txt"))).unwrap();
        assert!(prov.contains("tool\tstyle: recipes.txt") && prov.contains("prompt-hash\t"), "{prov}");
        assert!(prov.contains("original-hash\t"), "the workspace's original is hashed as an input: {prov}");
        // Re-running over it is refused without --replace; --replace is idempotent.
        assert!(run(&["init", "--from-studio", &ws_s, "--project", &dest_s]).is_err());
        let before = std::fs::read_to_string(dest.join("assets.tsv")).unwrap();
        run(&["init", "--project", &dest_s]).unwrap();
        assert_eq!(before, std::fs::read_to_string(dest.join("assets.tsv")).unwrap());
        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(&dest);
    }

    fn approval_goes_through_the_gates_the_approver_and_the_hash() {
        let ws = workspace("flow");
        let dir = ws.display().to_string();
        std::env::set_var("REMODEL_ACTOR", "lead");
        run(&["init", "--project", &dir, "--approver", "lead"]).unwrap();
        // A draft is not checked: naming it is a wrong request, --category skips it.
        let e = run(&["approve", A, "--project", &dir]).err().unwrap();
        assert_eq!(e.code, Code::BadRequest);
        let (ans, _) = run(&["approve", "--category", "wall", "--project", &dir]).unwrap();
        assert!(ans.data["approved"].as_array().unwrap().is_empty());
        // Checking it makes it approvable.
        let a = |s: &str| s.to_string();
        let checked = super::super::check_assets(&ws, &[a(A)], None).unwrap();
        assert!(checked[0].verdict() < gates::Verdict::Error);
        let h = gates::hash_bytes(&std::fs::read(ws.join(format!("draft/{A}.png"))).unwrap());
        Project { dir: ws.clone() }.record_hashes("checked.tsv", &[(a(A), Some(h))]).unwrap();
        // Not an approver: refused with exit code 2.
        std::env::set_var("REMODEL_ACTOR", "intruder");
        assert_eq!(run(&["approve", A, "--project", &dir]).err().unwrap().code, Code::BadRequest);
        std::env::set_var("REMODEL_ACTOR", "lead");
        let (ans, _) = run(&["approve", A, "--project", &dir]).unwrap();
        assert_eq!(ans.data["approved"][0]["asset"], A);
        let rows = Project { dir: ws.clone() }.recorded();
        assert_eq!((rows[A].status, rows[A].by.as_str()), (St::Approved, "lead"));
        assert!(std::fs::read_to_string(ws.join("log.tsv"))
            .unwrap()
            .contains(&format!("approve\t{A}\tchecked\tapproved")));
        let prov = std::fs::read_to_string(ws.join(format!("provenance/{A}.txt"))).unwrap();
        assert!(prov.contains("tool\tstyle: recipes.txt") && prov.contains("approved-by\tlead"), "{prov}");
        // An edit after approval is stale, and `pack build` leaves it out.
        std::fs::write(ws.join(format!("pack/{A}.png")), picture(32, 32, |_, _| [1, 2, 3, 255])).unwrap();
        assert_eq!(Project { dir: ws.clone() }.derive().iter().find(|r| r.id == A).unwrap().status, St::Stale);
        let out = ws.join("build");
        let (ans, _) =
            build_run(&[a("build"), a("--project"), dir.clone(), a("--out"), out.display().to_string()]).unwrap();
        assert_eq!(ans.data["stale_left_out"][0], A);
        assert!(out.join(format!("{B}.png")).is_file() && !out.join(format!("{A}.png")).is_file());
        let manifest = std::fs::read_to_string(out.join("pack.txt")).unwrap();
        assert!(manifest.contains("version\t0.1.0") && manifest.contains("game\ttopskatr"), "{manifest}");
        // Rejecting a draft keeps its reason until a different draft is written.
        std::fs::write(ws.join(format!("draft/{A}.png")), picture(32, 32, |x, _| [x as u8, 9, 9, 255])).unwrap();
        run(&["reject", A, "--reason", "wrong colour", "--project", &dir]).unwrap();
        let rows = Project { dir: ws.clone() }.derive();
        let r = rows.iter().find(|r| r.id == A).unwrap();
        assert_eq!((r.status, r.note.as_str()), (St::Rejected, "wrong colour"));
        std::fs::write(ws.join(format!("draft/{A}.png")), picture(32, 32, |x, _| [x as u8, 10, 9, 255])).unwrap();
        assert_eq!(Project { dir: ws.clone() }.derive().iter().find(|r| r.id == A).unwrap().status, St::Drafted);
        let _ = std::fs::remove_dir_all(&ws);
    }

    fn a_gate_at_error_refuses_approval_unless_overridden_with_a_reason() {
        let ws = workspace("gate");
        let dir = ws.display().to_string();
        std::env::set_var("REMODEL_ACTOR", "lead");
        run(&["init", "--project", &dir]).unwrap();
        // An opaque draft for a cut-out original: the `alpha` gate is at error.
        std::fs::write(ws.join(format!("draft/{A}.png")), picture(32, 32, |x, y| [x as u8, y as u8, 7, 255])).unwrap();
        let h = gates::hash_bytes(&std::fs::read(ws.join(format!("draft/{A}.png"))).unwrap());
        Project { dir: ws.clone() }.record_hashes("checked.tsv", &[(A.to_string(), Some(h))]).unwrap();
        let (ans, _) = run(&["approve", A, "--project", &dir]).unwrap();
        assert_eq!(ans.code, Code::GateFailed);
        assert!(!ws.join(format!("pack/{A}.png")).exists());
        let e = run(&["approve", A, "--override", "", "--project", &dir]).err().unwrap();
        assert_eq!(e.code, Code::BadRequest);
        let (ans, _) =
            run(&["approve", A, "--override", "the cut-out is wrong in the original", "--project", &dir]).unwrap();
        assert!(ans.ok() && ans.warnings.iter().any(|w| w.contains("over a gate at error")), "{:?}", ans.warnings);
        let log = std::fs::read_to_string(ws.join("log.tsv")).unwrap();
        assert!(log.contains("override: the cut-out is wrong"), "{log}");
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// One test: the verbs read `REMODEL_ACTOR`, which is process-wide.
    #[test]
    fn the_project_verbs() {
        a_workspace_is_imported_and_its_work_survives();
        approval_goes_through_the_gates_the_approver_and_the_hash();
        a_gate_at_error_refuses_approval_unless_overridden_with_a_reason();
    }
}
