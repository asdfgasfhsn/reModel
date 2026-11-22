//! `remodel asset check`: the asset gates (gates.rs) on a studio project's
//! images, and the approval record `stale` compares with.
//!
//! The record is `status/approved.tsv` (`name`, hash, date), written by
//! `studio approve`; `status/checked.tsv` (the same columns) is written by
//! `asset check` for a draft with no gate at `error`, which is what makes
//! an asset `checked`. Both are plain text, like the rest of the project.
//!
//! `check_assets` and `refusals` are the library entry points: item 6's
//! `project approve` calls `refusals` and stops on any asset that has a gate
//! at `error`, unless given an override with a reason.

use super::*;
use crate::answer::{Answer, Code};
use crate::gates::{self, Asset, GateResult, Limits, Verdict};
use serde_json::{json, Value};

/// Where the checked image is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Src {
    Draft,
    Pack,
}

impl Src {
    fn dir(self) -> &'static str {
        match self {
            Src::Draft => "draft",
            Src::Pack => "pack",
        }
    }
}

/// One asset's results.
pub struct Checked {
    pub name: String,
    pub from: Src,
    pub status: Status,
    pub gates: Vec<GateResult>,
}

impl Checked {
    /// The worst verdict, where gates that do not apply (`info`) count as a pass.
    pub fn verdict(&self) -> Verdict {
        match gates::worst(&self.gates) {
            Verdict::Info => Verdict::Pass,
            v => v,
        }
    }

    fn json(&self, checked: bool) -> Value {
        json!({
            "asset": self.name, "status": self.status.name(), "from": self.from.dir(),
            "file": format!("{}/{}.png", self.from.dir(), self.name),
            "checked": checked, "verdict": self.verdict().name(),
            "gates": self.gates.iter().map(GateResult::to_json).collect::<Vec<_>>(),
        })
    }
}

impl Project {
    fn ledger_of(&self, file: &str) -> BTreeMap<String, String> {
        std::fs::read_to_string(self.p("status").join(file))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                l.split_once('\t').map(|(n, r)| (n.to_string(), r.split('\t').next().unwrap_or("").to_string()))
            })
            .collect()
    }

    /// Hashes recorded in `file` (`approved.tsv` or `checked.tsv`) by name.
    pub(super) fn hashes(&self, file: &str) -> BTreeMap<String, u64> {
        self.ledger_of(file).into_iter().filter_map(|(n, h)| Some((n, u64::from_str_radix(&h, 16).ok()?))).collect()
    }

    /// Record (or with `None` remove) the hash of each name's file in `file`.
    pub(super) fn record_hashes(&self, file: &str, items: &[(String, Option<u64>)]) -> Result<(), String> {
        if items.is_empty() {
            return Ok(());
        }
        let mut m = self.ledger_of(file);
        for (n, h) in items {
            match h {
                Some(h) => {
                    m.insert(n.clone(), gates::hash_hex(*h));
                }
                None => {
                    m.remove(n);
                }
            }
        }
        let mut s = String::new();
        for (n, h) in &m {
            let _ = writeln!(s, "{n}\t{h}\t{}", today());
        }
        std::fs::create_dir_all(self.p("status")).map_err(|e| e.to_string())?;
        std::fs::write(self.p("status").join(file), s).map_err(|e| e.to_string())
    }
}

/// `(wrap u, wrap v)` the way `studio seam` decides: from the inventory row's
/// wrap column, with an axis the game never draws past its size clamped.
pub(super) fn wraps(inv: Option<&Inv>, name: &str) -> (Wrap, Wrap) {
    let (mut wu, mut wv) = tile_wrap(inv, name);
    if let Some(i) = inv.filter(|i| i.kind == "tex") {
        if i.span_u <= 1.0 {
            wu = Wrap::Clamp;
        }
        if i.span_v <= 1.0 {
            wv = Wrap::Clamp;
        }
    }
    (wu, wv)
}

/// The asset gates on each of `names` (selection rows or file stems; empty:
/// every image in `draft/` and `pack/`). `from` is where the image is
/// (default: the draft when there is one, else the approved image).
pub fn check_assets(dir: &Path, names: &[String], from: Option<Src>) -> Result<Vec<Checked>, Answer> {
    let p = Project { dir: dir.to_path_buf() };
    p.require().map_err(|e| Answer::fail(Code::Missing, e))?;
    let limits = Limits::load(dir).map_err(|e| Answer::fail(Code::BadRequest, e))?;
    let rows = p.selection();
    let inv: HashMap<String, Inv> =
        p.inventory().unwrap_or_default().into_iter().map(|i| (i.name.clone(), i)).collect();
    let mut stems: Vec<String> = Vec::new();
    if names.is_empty() {
        // Every image where it is: in `from`'s folder, else in draft/ or pack/ (the draft wins).
        let mut all: BTreeSet<String> = BTreeSet::new();
        for (src, d) in [(Src::Draft, "draft"), (Src::Pack, "pack")] {
            if from.is_none_or(|f| f == src) {
                all.extend(png_names(&p.p(d)));
            }
        }
        stems = all.into_iter().collect();
    }
    let mut known: Vec<String> = rows.iter().map(|r| r.name.clone()).collect();
    known.extend(png_names(&p.p("draft")));
    known.extend(png_names(&p.p("pack")));
    known.sort();
    known.dedup();
    for n in names {
        match known.iter().find(|k| name_matches(k, n)) {
            Some(k) => stems.push(k.clone()),
            None => {
                // Near matches only: the whole project's names would fill the screen.
                let near = crate::answer::near_matches(n, &known);
                let shown: Vec<String> = near.into_iter().take(6).collect();
                return Err(Answer::unknown("asset", n, &shown).next("remodel studio status PROJECT --tsv"));
            }
        }
    }
    let approved = p.hashes("approved.tsv");
    // The asset gates are arithmetic on one image each: spread them over the cores.
    let one = |name: &String| -> Result<Checked, Answer> {
        let src = match from {
            Some(f) => f,
            None if p.p("draft").join(format!("{name}.png")).is_file() => Src::Draft,
            None => Src::Pack,
        };
        let path = p.p(src.dir()).join(format!("{name}.png"));
        if !path.is_file() {
            return Err(Answer::fail(Code::Missing, format!("{}: no such image", path.display()))
                .next("write the draft to draft/NAME.png, then check it"));
        }
        let bytes =
            std::fs::read(&path).map_err(|e| Answer::fail(Code::Internal, format!("{}: {e}", path.display())))?;
        let image = png::decode(&bytes);
        let original = p.original(name).and_then(|f| img::load(&f).ok());
        let asset = Asset {
            name,
            image: image.as_ref().map_err(|e| e.clone()),
            bytes: bytes.len() as u64,
            original: original.as_ref(),
            wrap: wraps(inv.get(name), name),
            hash: gates::hash_bytes(&bytes),
            approved: if src == Src::Pack { approved.get(name).copied() } else { None },
        };
        Ok(Checked { name: name.clone(), from: src, status: p.status_of(name), gates: gates::check(&asset, &limits) })
    };
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(stems.len().max(1));
    let chunk = stems.len().div_ceil(threads).max(1);
    let parts: Vec<Result<Vec<Checked>, Answer>> = std::thread::scope(|sc| {
        let hs: Vec<_> = stems.chunks(chunk).map(|c| sc.spawn(|| c.iter().map(&one).collect())).collect();
        hs.into_iter()
            .map(|h| h.join().unwrap_or_else(|_| Err(Answer::fail(Code::Internal, "an asset gate panicked"))))
            .collect()
    });
    let mut out = Vec::new();
    for part in parts {
        out.extend(part?);
    }
    Ok(out)
}

/// What `project approve` asks before approving: the assets among `names`
/// (draft images) with a gate at `error`, with those gates. Empty: all may be
/// approved.
#[allow(dead_code)] // called by `project approve` (item 6); tested below
pub fn refusals(dir: &Path, names: &[String]) -> Result<Vec<Checked>, Answer> {
    Ok(check_assets(dir, names, Some(Src::Draft))?.into_iter().filter(|c| c.verdict() == Verdict::Error).collect())
}

const USAGE: &str = "\
remodel asset check [ID ...|--all] [--project DIR] [--from draft|pack] [--json]
  The asset gates on one image: file, alpha, seam, changed, noise (errors: file, alpha, stale; the rest warn).
  ID is a name, a 16-digit key or a file stem; --all checks every image in draft/ and pack/.
  Exit 0 when no gate is at error, 1 when one is. A draft with none is recorded as checked.
";

pub fn asset_main(o: &crate::Opts) -> std::process::ExitCode {
    let rest = &o.args[1..];
    let json = rest.iter().any(|a| a == "--json");
    match asset_run(rest) {
        Ok((a, text)) => a.emit(json, || text),
        Err(a) => a.emit(json, || USAGE.to_string()),
    }
}

fn asset_run(rest: &[String]) -> Result<(Answer, String), Answer> {
    use crate::scene::Args;
    let Some(sub) = rest.iter().find(|a| !a.starts_with("--")) else {
        return Err(Answer::fail(Code::BadRequest, "asset wants a verb: check").next("remodel asset check --help"));
    };
    if sub != "check" {
        return Err(Answer::unknown("asset verb", sub, &["check".to_string()]));
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
    let a = Args::parse(&argv, &["--json", "--all"], &["--project", "--from"])?;
    let from = match a.one("--from") {
        None => None,
        Some("draft") => Some(Src::Draft),
        Some("pack") => Some(Src::Pack),
        Some(o) => return Err(Answer::unknown("--from", o, &["draft".to_string(), "pack".to_string()])),
    };
    if a.pos.is_empty() == !a.has("--all") {
        return Err(Answer::fail(Code::BadRequest, "asset check: which asset? ID ... or --all")
            .next("remodel studio status PROJECT --tsv"));
    }
    let dir = PathBuf::from(a.one("--project").unwrap_or("."));
    let t = std::time::Instant::now();
    let checked = check_assets(&dir, &a.pos, from)?;
    let p = Project { dir: dir.clone() };
    // A draft with no gate at error is `checked`.
    let mut passed = Vec::new();
    for c in &checked {
        if c.from == Src::Draft && c.verdict() < Verdict::Error {
            if let Ok(b) = std::fs::read(p.p("draft").join(format!("{}.png", c.name))) {
                passed.push((c.name.clone(), Some(gates::hash_bytes(&b))));
            }
        }
    }
    p.record_hashes("checked.tsv", &passed).map_err(|e| Answer::fail(Code::Internal, e))?;
    let was_checked: HashSet<&str> = passed.iter().map(|x| x.0.as_str()).collect();
    let rows: Vec<Value> = checked.iter().map(|c| c.json(was_checked.contains(c.name.as_str()))).collect();
    let count = |v: Verdict| checked.iter().flat_map(|c| &c.gates).filter(|g| g.verdict == v).count();
    let mut data = json!({
        "assets": rows,
        "counts": {"error": count(Verdict::Error), "warn": count(Verdict::Warn), "info": count(Verdict::Info), "pass": count(Verdict::Pass)},
        "ms": t.elapsed().as_millis() as u64,
    });
    if let [only] = checked.as_slice() {
        data["status"] = json!(only.status.name());
        data["gates"] = data["assets"][0]["gates"].clone();
    }
    let mut text = String::new();
    for c in &checked {
        let _ = writeln!(text, "{} ({}/): {}", c.name, c.from.dir(), c.verdict().name());
        text += &gates::text_lines(&c.gates);
    }
    let _ = writeln!(text, "# asset gates: {} error, {} warn", count(Verdict::Error), count(Verdict::Warn));
    let failed: Vec<&str> = checked.iter().filter(|c| c.verdict() == Verdict::Error).map(|c| c.name.as_str()).collect();
    let mut ans = if failed.is_empty() {
        Answer::done(data)
    } else {
        Answer::gate(format!("{} asset(s) with a gate at error: {}", failed.len(), failed.join(", ")), data)
    };
    for c in &checked {
        if c.verdict() == Verdict::Warn {
            let w: Vec<&str> = c.gates.iter().filter(|g| g.verdict == Verdict::Warn).map(|g| g.gate).collect();
            ans = ans.warn(format!("{}: {}", c.name, w.join(", ")));
        }
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

    /// A project with one cut-out texture, its original and a restyled draft.
    fn project(tag: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/scratch").join(format!("gates-test-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["original", "draft", "pack", "status"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        std::fs::write(dir.join("project.txt"), "name\tt\ngame\ttopskatr\n").unwrap();
        std::fs::write(
            dir.join("selection.tsv"),
            format!("{SEL_HEADER}\ntex_0123456789abcdef_16x16\ttex\t0\t-\t100\tc\tr\n"),
        )
        .unwrap();
        let cut = |x: usize, y: usize| x < 4 && y < 4;
        std::fs::write(
            dir.join("original/tex_0123456789abcdef_16x16.png"),
            picture(16, 16, |x, y| [(x * 16) as u8, (y * 16) as u8, 90, if cut(x, y) { 0 } else { 255 }]),
        )
        .unwrap();
        dir
    }

    fn draft(dir: &Path, keep_cutout: bool) {
        let cut = move |x: usize, y: usize| keep_cutout && x < 8 && y < 8;
        std::fs::write(
            dir.join("draft/tex_0123456789abcdef_16x16.png"),
            picture(32, 32, |x, y| [255 - (x * 8) as u8, (y * 8) as u8, 200, if cut(x, y) { 0 } else { 255 }]),
        )
        .unwrap();
    }

    #[test]
    fn a_bad_draft_is_refused_and_a_good_one_is_checked_approved_then_stale() {
        let dir = project("flow");
        let name = vec!["tex_0123456789abcdef_16x16".to_string()];
        // An opaque draft for a cut-out: `alpha` fails, so approval is refused.
        draft(&dir, false);
        let r = refusals(&dir, &name).unwrap();
        assert_eq!(r.len(), 1);
        assert!(r[0].gates.iter().any(|g| g.gate == "alpha" && g.verdict == Verdict::Error));
        // A draft that keeps the cut-out passes the errors.
        draft(&dir, true);
        assert!(refusals(&dir, &name).unwrap().is_empty());
        // Approving records the hash; the image then checks as not stale ...
        let p = Project { dir: dir.clone() };
        approve(&p, &Args::parse(&["--drafted".to_string()]).unwrap()).unwrap();
        let c = check_assets(&dir, &name, Some(Src::Pack)).unwrap();
        let stale = c[0].gates.iter().find(|g| g.gate == "stale").unwrap();
        assert_eq!(stale.verdict, Verdict::Pass, "{stale:?}");
        // ... and an edit after approval makes it stale, an error.
        draft(&dir, false);
        std::fs::copy(
            dir.join("draft/tex_0123456789abcdef_16x16.png"),
            dir.join("pack/tex_0123456789abcdef_16x16.png"),
        )
        .unwrap();
        let c = check_assets(&dir, &name, Some(Src::Pack)).unwrap();
        let stale = c[0].gates.iter().find(|g| g.gate == "stale").unwrap();
        assert_eq!(stale.verdict, Verdict::Error, "{stale:?}");
        // An unknown name lists near ones; the exit code is 2.
        let e = check_assets(&dir, &["tex_0123456789abcdef_16x1".to_string()], None).err().unwrap();
        assert_eq!(e.code, Code::BadRequest);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
