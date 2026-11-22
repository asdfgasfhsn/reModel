//! `remodel pack check`: every gate level on a studio project's pack, one
//! report (docs/uplift-studio.md, "Gates").
//!
//! Levels: `asset` (the asset gates of gates.rs on every approved image),
//! `pack` (`lint` and `budget` from pack-lint, `known`, `selected`), `scene`
//! (`coverage`, from the scenes' captures: a GPU, no ROM) and `clip` (needs
//! a run: `pack-flicker` is its gate, so it is listed as skipped).
//!
//! Output, in the project's `reports/DATE-VERSION/`: `report.json` (all
//! results), `gates.tsv` (one row per asset and gate: value, limit, verdict,
//! the pass criterion) and `report.md` (totals, failures first).

use crate::answer::{Answer, Code};
use crate::gates::{self, GateResult, Limits, Verdict};
use crate::scene::Args;
use crate::studio::{self, today};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
remodel pack check [--project DIR] [--level asset,pack,scene,clip] [--asset ID].. [--release] [--json]
  Every gate on the project's pack/ and a report in reports/DATE-VERSION/ (report.json, gates.tsv, report.md).
  asset: file, alpha, seam, changed, noise, stale.  pack: lint, budget, known, selected.  scene: coverage (needs
  the scenes made, a GPU, no ROM).  clip: pack-flicker on a --pack-log run (listed as skipped here).
  --release makes `selected` and `coverage` errors; without it they warn. Exit 1 when a gate is at error.
  Limits are gates.txt in the project (GATE, VALUE, REASON by tabs); the report lists each override.
";

pub fn main(o: &crate::Opts) -> ExitCode {
    let rest = &o.args[1..];
    let json = rest.iter().any(|a| a == "--json");
    match run(&o.game_id, rest, crate::OUT_DIR.get().cloned()) {
        Ok((a, text)) => a.emit(json, || text),
        Err(a) => a.emit(json, || USAGE.to_string()),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Asset,
    Pack,
    Scene,
    Clip,
}

impl Level {
    const ALL: [Level; 4] = [Level::Asset, Level::Pack, Level::Scene, Level::Clip];

    fn name(self) -> &'static str {
        match self {
            Level::Asset => "asset",
            Level::Pack => "pack",
            Level::Scene => "scene",
            Level::Clip => "clip",
        }
    }
}

/// The pack and scene gates' own table row.
fn row(
    asset: &str,
    gate: &'static str,
    value: f64,
    limit: f64,
    verdict: Verdict,
    rule: String,
    fix: &str,
) -> GateResult {
    let mut r = GateResult::info(asset, gate, rule);
    r.value = value;
    r.limit = limit;
    r.verdict = verdict;
    if verdict >= Verdict::Warn {
        r.fix = fix.into();
    }
    r
}

fn run(default_game: &str, rest: &[String], out: Option<PathBuf>) -> Result<(Answer, String), Answer> {
    let Some(sub) = rest.iter().find(|a| !a.starts_with("--")) else {
        return Err(Answer::fail(Code::BadRequest, "pack wants a verb: check").next("remodel pack check --help"));
    };
    if sub != "check" {
        return Err(Answer::unknown("pack verb", sub, &["check".to_string()]));
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
    let a = Args::parse(&argv, &["--json", "--release"], &["--project", "--level", "--asset"])?;
    let levels: Vec<Level> = match a.one("--level") {
        None => Level::ALL.to_vec(),
        Some(list) => {
            let mut v = Vec::new();
            for l in list.split(',') {
                match Level::ALL.iter().find(|x| x.name() == l) {
                    Some(x) => v.push(*x),
                    None => {
                        let known: Vec<String> = Level::ALL.iter().map(|l| l.name().to_string()).collect();
                        return Err(Answer::unknown("level", l, &known));
                    }
                }
            }
            v
        }
    };
    let release = a.has("--release");
    let project = PathBuf::from(a.one("--project").unwrap_or("."));
    let t = std::time::Instant::now();
    if !project.join("project.txt").is_file() {
        return Err(Answer::fail(
            Code::Missing,
            format!("{}: not a studio project (no project.txt)", project.display()),
        )
        .next("remodel pack check --project DIR"));
    }
    let limits = Limits::load(&project).map_err(|e| Answer::fail(Code::BadRequest, e))?;
    let pack_dir = project.join("pack");
    let mut results: Vec<GateResult> = Vec::new();
    let mut lint_items: Vec<Value> = Vec::new();
    let mut skipped_levels: Vec<Value> = Vec::new();
    let ids = a.vals.get("--asset").cloned().unwrap_or_default();

    for level in &levels {
        match level {
            Level::Asset => {
                let checked = studio::check_assets(&project, &ids, Some(studio::Src::Pack))?;
                results.extend(checked.into_iter().flat_map(|c| c.gates));
            }
            Level::Pack => {
                if !ids.is_empty() {
                    skipped_levels.push(json!({"level": "pack", "why": "--asset narrows to the asset level"}));
                    continue;
                }
                pack_level(&project, &pack_dir, &limits, release, &mut results, &mut lint_items)?;
            }
            Level::Scene => match scene_level(default_game, &project, &pack_dir, &limits, release, &mut results) {
                Ok(()) => {}
                Err(why) => skipped_levels.push(json!({"level": "scene", "why": why})),
            },
            Level::Clip => skipped_levels.push(json!({
                "level": "clip",
                "why": "needs a run: remodel run --pack-log FILE, then remodel pack-flicker FILE (exit 1 over 5000 px per 100 frames)",
            })),
        }
    }

    // Per gate: how many of each verdict, and the limit and criterion of the first.
    let mut by_gate: BTreeMap<&'static str, Vec<&GateResult>> = BTreeMap::new();
    for r in &results {
        by_gate.entry(r.gate).or_default().push(r);
    }
    let count = |v: Verdict| results.iter().filter(|r| r.verdict == v).count();
    let (errors, warns) = (count(Verdict::Error), count(Verdict::Warn));
    let summary: Vec<Value> = by_gate
        .iter()
        .map(|(g, rs)| {
            let n = |v: Verdict| rs.iter().filter(|r| r.verdict == v).count();
            let applies = rs.iter().find(|r| r.verdict != Verdict::Info);
            json!({
                "gate": g, "checked": rs.len() - n(Verdict::Info), "pass": n(Verdict::Pass), "warn": n(Verdict::Warn),
                "error": n(Verdict::Error), "not_applicable": n(Verdict::Info),
                "limit": applies.map(|r| r.limit),
                "criterion": applies.copied().or(rs.first().copied()).map(|r| r.rule.split(':').next().unwrap_or("").to_string()),
            })
        })
        .collect();
    let failing: Vec<&GateResult> = {
        let mut f: Vec<&GateResult> = results.iter().filter(|r| r.verdict >= Verdict::Warn).collect();
        f.sort_by(|x, y| y.verdict.cmp(&x.verdict).then(x.gate.cmp(y.gate)).then(x.asset.cmp(&y.asset)));
        f
    };

    let version = remodel_version::VERSION.replace(['/', ' '], "-");
    // `--out DIR` is the CLI's global option: where this run writes.
    let out_dir = out.unwrap_or_else(|| project.join("reports").join(format!("{}-{version}", today())));
    let overrides = limits.overrides();
    let data = json!({
        "done": errors == 0,
        "levels": levels.iter().map(|l| l.name()).collect::<Vec<_>>(),
        "counts": {"error": errors, "warn": warns, "pass": count(Verdict::Pass), "not_applicable": count(Verdict::Info)},
        "gates": summary,
        "failures": failing.iter().take(50).map(|r| r.to_json()).collect::<Vec<_>>(),
        "failures_total": failing.len(),
        "lint": lint_items,
        "overrides": overrides.iter().map(|(g, v, r)| json!({"gate": g, "limit": v, "reason": r})).collect::<Vec<_>>(),
        "skipped": skipped_levels,
        "report": out_dir.display().to_string(),
        "ms": t.elapsed().as_millis() as u64,
    });
    write_report(&out_dir, &results, &data, &failing).map_err(|e| Answer::fail(Code::Internal, e))?;

    let mut text = String::new();
    let _ = writeln!(
        text,
        "pack check {} ({})",
        project.display(),
        levels.iter().map(|l| l.name()).collect::<Vec<_>>().join(", ")
    );
    let _ = writeln!(text, "{:<11}{:>8}{:>7}{:>7}{:>7}  pass criterion", "gate", "checked", "pass", "warn", "error");
    for s in data["gates"].as_array().into_iter().flatten() {
        let _ = writeln!(
            text,
            "{:<11}{:>8}{:>7}{:>7}{:>7}  {}",
            s["gate"].as_str().unwrap_or(""),
            s["checked"].as_u64().unwrap_or(0),
            s["pass"].as_u64().unwrap_or(0),
            s["warn"].as_u64().unwrap_or(0),
            s["error"].as_u64().unwrap_or(0),
            s["criterion"].as_str().unwrap_or("")
        );
    }
    // Every error (up to 25), then the first two warnings of each gate.
    let mut shown = 0;
    let mut per_gate: BTreeMap<(&str, Verdict), usize> = BTreeMap::new();
    for r in &failing {
        let n = per_gate.entry((r.gate, r.verdict)).or_default();
        *n += 1;
        if (r.verdict == Verdict::Error && *n <= 25) || (r.verdict == Verdict::Warn && *n <= 2) {
            let _ = writeln!(text, "{:<6} {:<10} {:<28} {}", r.verdict.name(), r.gate, r.asset, r.rule);
            shown += 1;
        }
    }
    if failing.len() > shown {
        let _ = writeln!(text, "... and {} more in {}/gates.tsv", failing.len() - shown, out_dir.display());
    }
    for (g, v, r) in &overrides {
        let _ = writeln!(text, "override      {g} limit {v}: {r}");
    }
    for s in &skipped_levels {
        let _ =
            writeln!(text, "skipped       {}: {}", s["level"].as_str().unwrap_or(""), s["why"].as_str().unwrap_or(""));
    }
    let _ = writeln!(text, "report        {}", out_dir.display());
    let _ = writeln!(text, "# pack check: {errors} error, {warns} warn, done: {}", data["done"]);
    let ans = if errors == 0 {
        Answer::done(data)
    } else {
        Answer::gate(format!("{errors} gate result(s) at error (see report.md)"), data)
            .next(format!("remodel asset check --project {} ID", project.display()))
    };
    Ok((ans, text))
}

/// `lint`, `budget`, `known` and `selected`.
fn pack_level(
    project: &Path,
    pack_dir: &Path,
    limits: &Limits,
    release: bool,
    out: &mut Vec<GateResult>,
    lint_items: &mut Vec<Value>,
) -> Result<(), Answer> {
    if !pack_dir.is_dir() {
        return Err(Answer::fail(Code::Missing, format!("{}: no pack/ (nothing approved yet)", pack_dir.display()))
            .next(format!("remodel studio pack {}", project.display())));
    }
    let f = crate::pack_lint::findings(pack_dir, 8192).map_err(|e| Answer::fail(Code::Missing, e))?;
    let (e, w) = (f.count("error"), f.count("warning"));
    for (level, check, what, items) in &f.found {
        lint_items.push(json!({"level": level, "check": check.to_string(), "what": what, "files": items}));
    }
    out.push(row(
        "pack",
        "lint",
        e as f64,
        0.0,
        if e > 0 { Verdict::Error } else { Verdict::Pass },
        format!("pack-lint errors at most 0: {e} errors, {w} warnings"),
        "run `remodel pack-lint pack/` for each finding",
    ));
    // budget: GPU memory of everything drawn at once, mipmapped, in MB.
    let mb = |b: u64| b as f64 / (1 << 20) as f64;
    let native = limits.get("budget", mb(model2::pack::BUDGET_NATIVE));
    let browser = mb(model2::pack::BUDGET_WASM).min(native);
    let used = mb(f.gpu[1]);
    out.push(row(
        "pack",
        "budget",
        used,
        native,
        if used > native {
            Verdict::Error
        } else if used > browser {
            Verdict::Warn
        } else {
            Verdict::Pass
        },
        format!("GPU memory (mipmapped) at most {native:.0} MB native and {browser:.0} MB in the browser: {used:.1} MB ({:.1} MB without mipmaps)", mb(f.gpu[0])),
        "shrink the biggest images (`asset normalise` at a smaller scale) or leave the least visible out",
    ));
    // known and selected: against the project's inventory and selection.
    let p = studio::Project { dir: project.to_path_buf() };
    let sel: BTreeSet<String> = p.selection().into_iter().map(|r| r.name).collect();
    let inv: BTreeSet<String> = p.inventory().map(|v| v.into_iter().map(|i| i.name).collect()).unwrap_or_default();
    let in_pack: Vec<String> = studio::png_names(pack_dir);
    let is_asset = |n: &str| n.starts_with("tex_") || n.starts_with("tile_") || n.starts_with("art_");
    // An art_ file is known by identity: its surface key and size are an
    // artwork of the project's maps, whatever colourings they were seen in.
    let art: BTreeSet<String> = studio::artworks(&p).unwrap_or_default().iter().map(|a| a.stem()).collect();
    let is_known = |n: &str| if n.starts_with("art_") { art.contains(n) } else { inv.contains(n) };
    if inv.is_empty() {
        out.push(GateResult::info("pack", "known", "no texture map in the project to resolve files against"));
    } else {
        let unknown: Vec<&String> = in_pack.iter().filter(|n| is_asset(n) && !is_known(n)).collect();
        let unselected = in_pack.iter().filter(|n| !sel.contains(*n) && !n.starts_with("region_")).count();
        out.push(row(
            "pack",
            "known",
            unknown.len() as f64,
            0.0,
            if unknown.is_empty() { Verdict::Pass } else { Verdict::Warn },
            format!(
                "files the catalogue has not seen at most 0: {} (first: {}); {unselected} not in the selection",
                unknown.len(),
                unknown.iter().take(3).map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
            ),
            "a file whose name is no texture of the game's map draws nowhere: rename it or remove it",
        ));
    }
    // A selected colouring is covered by its own image or by its artwork's.
    let covered = |n: &String| {
        in_pack.contains(n)
            || p.selection().iter().find(|r| &r.name == n).is_some_and(|r| {
                let Ok(model2::pack::FileKind::Image { width, height, .. }) = model2::pack::parse_name(n) else {
                    return false;
                };
                r.surface.len() == 16 && in_pack.contains(&format!("art_{}_{width}x{height}", r.surface))
            })
    };
    let short: Vec<&String> = sel.iter().filter(|n| !covered(n)).collect();
    out.push(row(
        "pack",
        "selected",
        short.len() as f64,
        0.0,
        match (short.is_empty(), release) {
            (true, _) => Verdict::Pass,
            (false, true) => Verdict::Error,
            (false, false) => Verdict::Warn,
        },
        format!(
            "selected assets not approved at most 0: {} of {} (first: {}){}",
            short.len(),
            sel.len(),
            short.iter().take(3).map(|s| s.as_str()).collect::<Vec<_>>().join(", "),
            if release { "" } else { "; an error with --release" }
        ),
        "draft, check and approve them (`remodel studio status PROJECT --missing`)",
    ));
    Ok(())
}

/// `coverage` for each scene with a local copy. `Err(why)` when the level
/// cannot run here.
fn scene_level(
    default_game: &str,
    project: &Path,
    pack_dir: &Path,
    limits: &Limits,
    release: bool,
    out: &mut Vec<GateResult>,
) -> Result<(), String> {
    let meta = std::fs::read_to_string(project.join("project.txt")).unwrap_or_default();
    let game_id = meta
        .lines()
        .find_map(|l| l.strip_prefix("game\t"))
        .map(str::to_string)
        .unwrap_or_else(|| default_game.to_string());
    let game = romset::games::find(&game_id).map_err(|e| e.to_string())?;
    let (shares, missing) = crate::scene::pack_coverage(game, Some(project), pack_dir).map_err(|a| a.message())?;
    if shares.is_empty() {
        return Err(format!(
            "no scene of {game_id} has a local copy ({} listed): `remodel scene make --all` (needs the ROM)",
            missing.len()
        ));
    }
    let lim = limits.get("coverage", gates::COVERAGE_MIN_PCT);
    for (name, share, drawn) in shares {
        let pct = share * 100.0;
        out.push(row(
            &name,
            "coverage",
            pct,
            lim,
            match (pct >= lim, release) {
                (true, _) => Verdict::Pass,
                (false, true) => Verdict::Error,
                (false, false) => Verdict::Warn,
            },
            format!(
                "pixels differing from the original at least {lim}%: {pct:.1}% (native view; scene `{name}`){}",
                drawn.map_or(String::new(), |d| format!(
                    "; of its textured polygon area {:.1}% is drawn from pack images (by identity, overlaps counted), a figure that does not need the image to differ",
                    d * 100.0
                ))
            ),
            "restyle what the scene draws most: `remodel scene render NAME --pack pack/ --against original`",
        ));
    }
    Ok(())
}

fn write_report(dir: &Path, results: &[GateResult], data: &Value, failing: &[&GateResult]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let w = |name: &str, s: String| {
        std::fs::write(dir.join(name), s).map_err(|e| format!("{}: {e}", dir.join(name).display()))
    };
    let mut tsv = format!("{}\n", gates::TSV_HEADER);
    for r in results {
        tsv += &r.tsv();
        tsv.push('\n');
    }
    w("gates.tsv", tsv)?;
    let mut all = data.clone();
    all["results"] = Value::Array(results.iter().map(GateResult::to_json).collect());
    w("report.json", serde_json::to_string_pretty(&all).map_err(|e| e.to_string())?)?;
    let mut md = String::from("# Pack check\n\n");
    let c = &data["counts"];
    let _ = writeln!(
        md,
        "{} error, {} warn, {} pass, {} not applicable. Done: {}.\n",
        c["error"], c["warn"], c["pass"], c["not_applicable"], data["done"]
    );
    md += "| Gate | Checked | Pass | Warn | Error | Pass criterion |\n|---|---|---|---|---|---|\n";
    for s in data["gates"].as_array().into_iter().flatten() {
        let _ = writeln!(
            md,
            "| {} | {} | {} | {} | {} | {} |",
            s["gate"].as_str().unwrap_or(""),
            s["checked"],
            s["pass"],
            s["warn"],
            s["error"],
            s["criterion"].as_str().unwrap_or("")
        );
    }
    md += "\n## Failures, errors first\n\n";
    if failing.is_empty() {
        md += "None.\n";
    }
    for r in failing.iter().take(200) {
        let _ = writeln!(md, "- **{}** `{}` {}: {}", r.verdict.name(), r.gate, r.asset, r.rule);
        if !r.fix.is_empty() {
            let _ = writeln!(md, "  - fix: {}", r.fix);
        }
    }
    if failing.len() > 200 {
        let _ = writeln!(md, "\n{} more in gates.tsv.", failing.len() - 200);
    }
    let ov = data["overrides"].as_array().cloned().unwrap_or_default();
    if !ov.is_empty() {
        md += "\n## Limit overrides (gates.txt)\n\n";
        for o in ov {
            let _ = writeln!(
                md,
                "- `{}` {}: {}",
                o["gate"].as_str().unwrap_or(""),
                o["limit"],
                o["reason"].as_str().unwrap_or("")
            );
        }
    }
    let sk = data["skipped"].as_array().cloned().unwrap_or_default();
    if !sk.is_empty() {
        md += "\n## Not run\n\n";
        for s in sk {
            let _ = writeln!(md, "- {}: {}", s["level"].as_str().unwrap_or(""), s["why"].as_str().unwrap_or(""));
        }
    }
    w("report.md", md)
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
    const B: &str = "tex_fedcba9876543210_16x16";

    /// The slice's acceptance test (docs/uplift-studio.md, "Gates"): on a
    /// project with a good asset and a bad one, `pack check` says in numbers
    /// what is wrong, writes its three files and exits as the verdicts say;
    /// fixed, the same command says it is done.
    #[test]
    fn pack_check_signs_a_revision_off_by_numbers() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/scratch/pack-gates-test");
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["original", "draft", "pack", "status"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        std::fs::write(dir.join("project.txt"), "name\tt\ngame\ttopskatr\n").unwrap();
        let mut sel = String::from("name\tkind\tgroup\tsurface\tpixels\tcategory\treason\n");
        for n in [A, B] {
            sel += &format!("{n}\ttex\t0\t-\t100\tc\tr\n");
        }
        std::fs::write(dir.join("selection.tsv"), sel).unwrap();
        for n in [A, B] {
            let o = picture(16, 16, |x, y| [(x * 16) as u8, (y * 16) as u8, 90, if x < 4 && y < 4 { 0 } else { 255 }]);
            std::fs::write(dir.join(format!("original/{n}.png")), o).unwrap();
        }
        let write = |n: &str, cut: bool| {
            let p = picture(32, 32, |x, y| {
                [255 - (x * 8) as u8, (y * 8) as u8, 200, if cut && x < 8 && y < 8 { 0 } else { 255 }]
            });
            std::fs::write(dir.join(format!("draft/{n}.png")), p).unwrap();
        };
        write(A, true);
        write(B, false); // an opaque draft for a cut-out
        let args = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let ctx = studio::Ctx { game: "topskatr".into(), rom: "rom".into(), jit: false, input: None };
        let approve = |extra: &[&str]| {
            let mut a = args(&["approve", dir.to_str().unwrap(), "--drafted"]);
            a.extend(extra.iter().map(|s| s.to_string()));
            studio::main(&a, &ctx).unwrap();
        };
        approve(&[]);
        let out = dir.join("report");
        let go = |extra: &[&str]| {
            let mut a = args(&["check", "--project", dir.to_str().unwrap(), "--level", "asset,pack"]);
            a.extend(extra.iter().map(|s| s.to_string()));
            run("topskatr", &a, Some(out.clone())).unwrap().0
        };
        let a = go(&[]);
        assert_eq!(a.code, Code::GateFailed, "{:?}", a.error);
        let j = a.to_json();
        let gates = j["data"]["gates"].as_array().unwrap();
        for g in
            ["file", "alpha", "alpha.soft", "seam", "changed", "noise", "stale", "lint", "budget", "known", "selected"]
        {
            let row = gates.iter().find(|r| r["gate"] == g).unwrap_or_else(|| panic!("no gate {g}"));
            assert!(row["criterion"].is_string(), "{g} states its pass criterion");
        }
        let bad = j["data"]["failures"].as_array().unwrap().iter().find(|f| f["gate"] == "alpha").unwrap();
        assert_eq!(bad["asset"], B);
        assert_eq!(bad["verdict"], "error");
        assert_eq!(bad["limit"], gates::ALPHA_CUTOUT_PCT);
        assert!(bad["value"].as_f64().unwrap() > gates::ALPHA_CUTOUT_PCT);
        assert!(!bad["fix"].as_str().unwrap().is_empty());
        assert_eq!(j["data"]["done"], false);
        for f in ["gates.tsv", "report.md", "report.json"] {
            assert!(out.join(f).is_file(), "{f}");
        }
        let tsv = std::fs::read_to_string(out.join("gates.tsv")).unwrap();
        assert!(tsv.starts_with("asset\tgate\tvalue\tlimit\tverdict\trule"), "{tsv}");
        assert!(tsv.contains(&format!("{B}\talpha\t")), "{tsv}");
        assert!(std::fs::read_to_string(out.join("report.md")).unwrap().contains("Failures, errors first"));
        // Narrowed to the good asset: nothing at error.
        let ok = go(&["--level", "asset", "--asset", A]);
        assert_eq!(ok.code, Code::Done, "{:?}", ok.error);
        // Fixed (the draft keeps its cut-out, approved again): done.
        write(B, true);
        approve(&["--replace"]);
        assert_eq!(go(&[]).code, Code::Done);
        // `selected` warns, and is an error under --release.
        std::fs::remove_file(dir.join(format!("pack/{A}.png"))).unwrap();
        assert_eq!(go(&[]).code, Code::Done, "an unapproved selected asset warns");
        assert_eq!(go(&["--release"]).code, Code::GateFailed, "and blocks a release");
        // A limit override needs a gate name and a reason; the report lists it.
        std::fs::write(dir.join("gates.txt"), "nois\t30\tgrain on purpose\n").unwrap();
        let e = run("topskatr", &args(&["check", "--project", dir.to_str().unwrap()]), None).err().unwrap();
        assert_eq!(e.code, Code::BadRequest);
        std::fs::write(dir.join("gates.txt"), "noise\t30\tgrain on purpose\n").unwrap();
        let a = go(&["--release"]);
        assert_eq!(a.to_json()["data"]["overrides"][0]["reason"], "grain on purpose");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Item 5's acceptance test (docs/uplift-studio.md, "Artworks"), without a
    /// ROM: one authored image stands for an artwork's three colourings.
    /// `bake` makes each colouring's image through the catalogue's ramps
    /// (and leaves a hand-made one alone); `known` and `selected` resolve
    /// the `art_` file by its surface key; in `fixed` mode no colour copies
    /// exist and the manifest `needs artworks`, in `baked` mode it `uses`
    /// them.
    #[test]
    fn one_art_image_stands_for_an_artworks_colourings() {
        use model2::idmap::ramp_text;
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/scratch/pack-gates-art-test");
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["original", "draft", "pack", "status", "map/textures"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        std::fs::write(dir.join("project.txt"), "name\tt\ngame\ttopskatr\n").unwrap();
        let ramp = |f: &dyn Fn(usize) -> [u8; 3]| -> [[u8; 3]; 16] { std::array::from_fn(f) };
        let red = ramp(&|i| [(i * 16) as u8, 20, 0]);
        let blue = ramp(&|i| [0, 20, (255 - i * 12) as u8]);
        let green = ramp(&|i| [10, (i * 15) as u8, 30]);
        let names = ["tex_00000000000000a1_16x16", "tex_00000000000000a2_16x16", "tex_00000000000000a3_16x16"];
        let surface = "00000000000000aa";
        let mut inv = String::from("id\tname\tkind\tw\th\tsurface\tframes\tpixels\tbest_frame\tbest_pixels\tpolys\tuv_used_pct\twrap\tspan_u\tspan_v\tgroup\tgroup_size\tdetail\n");
        let mut sel = String::from("name\tkind\tgroup\tsurface\tpixels\tcategory\treason\n");
        for (i, (n, r)) in names.iter().zip([&red, &blue, &green]).enumerate() {
            inv += &format!(
                "{}\t{n}\ttex\t16\t16\t{surface}\t1\t{}\t1\t1\t1\t100\trr\t2\t2\t0\t1\tbank=0 x=0 y=0 luma=1 colour=000{} ramp={}\n",
                i + 2,
                1000 - i * 100,
                i + 1,
                ramp_text(r)
            );
            sel += &format!("{n}\ttex\t0\t{surface}\t100\twall\tr\n");
            let im = picture(16, 16, |x, y| {
                let c = r[(x + y) % 16];
                [c[0], c[1], c[2], 255]
            });
            std::fs::write(dir.join(format!("map/textures/{n}.png")), im).unwrap();
        }
        std::fs::write(dir.join("map/inventory.tsv"), inv).unwrap();
        std::fs::write(dir.join("selection.tsv"), sel).unwrap();
        // The author paints over the most drawn colouring (red), twice as big.
        let art_name = format!("art_{surface}_16x16");
        std::fs::write(
            dir.join(format!("pack/{art_name}.png")),
            picture(32, 32, |x, y| {
                let c = red[(x / 2 + y / 2 + (x % 2)) % 16];
                [c[0], c[1].saturating_add(1), c[2], 255]
            }),
        )
        .unwrap();
        let args = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let ctx = studio::Ctx { game: "topskatr".into(), rom: "rom".into(), jit: false, input: None };
        let d = dir.to_str().unwrap();
        // The artwork index sees one artwork under three keys.
        let arts = studio::artworks(&studio::Project { dir: dir.clone() }).unwrap();
        assert_eq!((arts.len(), arts[0].variants.len(), arts[0].stem()), (1, 3, art_name.clone()));
        // Baked: a tex_ image per colouring, each through its ramp.
        studio::main(&args(&["bake", d, "--all"]), &ctx).unwrap();
        let load = |n: &str| png::decode(&std::fs::read(dir.join(format!("pack/{n}.png"))).unwrap()).unwrap();
        let art = load(&art_name);
        assert_eq!(load(names[0]).rgba, art.rgba, "the reference is the authored image");
        let b = load(names[1]);
        assert_eq!((b.width, b.height), (32, 32));
        let near = |p: &[u8], c: [u8; 3]| (0..3).all(|k| (p[k] as i32 - c[k] as i32).abs() <= 1);
        // Pixel (0,0) sat on red ramp entry 0: it is blue's entry 0 now.
        assert!(near(&b.rgba[..4], blue[0]), "{:?} vs {:?}", &b.rgba[..4], blue[0]);
        assert!(std::fs::read_to_string(dir.join("status/artworks.tsv")).unwrap().contains("baked"));
        studio::main(&args(&["pack", d, "--no-lint"]), &ctx).unwrap();
        let manifest = std::fs::read_to_string(dir.join("pack/pack.txt")).unwrap();
        assert!(manifest.contains("uses\tartworks") && !manifest.contains("needs"), "{manifest}");
        // A hand-made colouring survives a rebuild; the copies are reproduced.
        std::fs::write(dir.join(format!("pack/{}.png", names[2])), picture(16, 16, |_, _| [1, 2, 3, 255])).unwrap();
        let before = std::fs::read(dir.join(format!("pack/{}.png", names[1]))).unwrap();
        studio::main(&args(&["bake", d, &art_name]), &ctx).unwrap();
        assert_eq!(std::fs::read(dir.join(format!("pack/{}.png", names[1]))).unwrap(), before);
        assert_eq!(load(names[2]).width, 16, "the hand-made image is kept");
        // The gates resolve the art_ file by identity.
        let out = dir.join("report");
        let go = |extra: &[&str]| {
            let mut a = args(&["check", "--project", d, "--level", "pack,asset"]);
            a.extend(extra.iter().map(|s| s.to_string()));
            run("topskatr", &a, Some(out.clone())).unwrap().0.to_json()
        };
        let rows = |j: &Value, g: &str| {
            j["data"]["gates"].as_array().unwrap().iter().find(|r| r["gate"] == g).cloned().unwrap()
        };
        let j = go(&[]);
        assert_eq!(rows(&j, "known")["warn"], 0, "{j}");
        assert_eq!(rows(&j, "selected")["warn"], 0);
        // Fixed: the copies go, the hand-made one stays, the artwork is drawn as it is.
        studio::main(&args(&["bake", d, "--all", "--mode", "fixed"]), &ctx).unwrap();
        assert!(!dir.join(format!("pack/{}.png", names[1])).exists());
        assert!(dir.join(format!("pack/{}.png", names[2])).exists());
        studio::main(&args(&["pack", d, "--no-lint"]), &ctx).unwrap();
        assert!(std::fs::read_to_string(dir.join("pack/pack.txt")).unwrap().contains("needs\tartworks"));
        std::fs::remove_file(dir.join(format!("pack/{}.png", names[2]))).unwrap();
        let j = go(&["--release"]);
        assert_eq!(rows(&j, "selected")["error"], 0, "an art_ image covers every colouring selected: {j}");
        // A file for an artwork the maps do not have is unknown.
        std::fs::write(dir.join("pack/art_00000000000000bb_16x16.png"), picture(16, 16, |_, _| [0, 0, 0, 255]))
            .unwrap();
        assert_eq!(rows(&go(&[]), "known")["warn"], 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pack_level_row_carries_its_number_and_a_fix_only_on_failure() {
        let ok = row("pack", "budget", 10.0, 512.0, Verdict::Pass, "r".into(), "do it");
        assert!(ok.fix.is_empty());
        let bad = row("pack", "budget", 600.0, 512.0, Verdict::Error, "r".into(), "do it");
        assert_eq!(bad.fix, "do it");
        assert_eq!(bad.to_json()["limit"], 512.0);
    }
}
