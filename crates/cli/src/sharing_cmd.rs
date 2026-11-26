//! `remodel studio sharing`: the command (sharing.rs is the analysis,
//! sharing_collect.rs the reading of the scenes).

#![allow(clippy::needless_range_loop)]

use crate::answer::{Answer, Code};
use crate::scene::Args;
use crate::sharing::*;

fn rgb(t: crate::sharing_collect::Tone) -> String {
    format!("{:.0}/{:.0}/{:.0}", t[0], t[1], t[2])
}
use crate::sharing_collect::{Collected, PartInfo};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const USAGE: &str = "\
remodel studio sharing PROJECT [--owners FILE] [--pack DIR] [--scene NAME,..] [--sheets DIR] [--tsv FILE]
         [--fix DIR] [--skin-limit X] [--other-limit X] [--variant-limit X] [--refresh] [--write-map] [--all-rows] [--json]
  Who shares which texture, and is the pack consistent for each? Reads every scene of the project that has a
  local copy (no ROM, no GPU; the first run draws each capture, later runs read PROJECT/sharing/), groups the
  palettes drawn together on one object into owners, and compares what the pack draws for each (key, palette)
  with the original through that palette.
  owners.txt (PROJECT/owners.txt, or --owners): `NAME|pal,pal,..` names an owner and its palettes (hex ids as
  texmap prints `pal=`); the objects it does not take are clustered as auto-N.
  Flags: outlier (a skin part's tone off its owner's by more than --skin-limit, a CIE76 colour difference, 8; any
  other part's brightness (L*) by more than --other-limit, 15: a look may recolour clothes, not make one part
  of a skater lighter or darker than the rest), original (a character's part the pack does not draw), needs-variant:PAL
  (draws the plain image that the key's palette PAL draws too, and the two look different by more than
  --variant-limit; 10).
  --tsv FILE the rows (default PROJECT/sharing/sharing.tsv); --sheets DIR a contact sheet per owner (original
  through the palette | what the pack draws); --write-map writes PROJECT/map/sharing, a texture map with a row
  per (key, palette) of every key the scenes draw through more than one palette, so `studio select --key`
  finds each variant; --all-rows lists the rows with no flag too.
  --fix DIR writes, for every flagged part of a character, the variant image `tex_KEY~PAL_WxH.png` (the plain
  `tex_KEY_WxH.png` for a key the scenes draw through one palette) at the tone its owner's other parts put it at (the pack's image for that palette, or the original through it when
  the pack has none, each colour scaled so the mean is the target): a starting point to look at, import and
  approve.
  Exit 1 when a character has a flag. Also an optional gate, `sharing`, in `pack check`.
";

fn project_game(project: &Path, default: &str) -> String {
    std::fs::read_to_string(project.join("project.txt"))
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix("game\t"))
        .map(str::to_string)
        .unwrap_or_else(|| default.to_string())
}

/// The project's scenes that have a local copy: (name, directory).
pub fn project_scenes(game: &'static romset::Game, project: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let rs = crate::scene::recipes(game, Some(project))?;
    let root = crate::scene::scenes_dir(game);
    Ok(rs
        .iter()
        .filter(|r| crate::scene::state_of(&root.join(&r.name), Some(r)) != crate::scene::State::Missing)
        .map(|r| (r.name.clone(), root.join(&r.name)))
        .collect())
}

/// What a run of the analysis on a project leaves.
pub struct Run {
    pub collected: Collected,
    pub report: Report,
    pub pack: PackIndex,
    pub scenes: usize,
    pub owners_file: bool,
}

pub struct Request<'a> {
    pub game: &'a str,
    pub project: &'a Path,
    pub owners_file: Option<&'a Path>,
    pub pack_dir: Option<&'a Path>,
    pub only: &'a [String],
    pub limits: &'a Limits,
    pub refresh: bool,
    pub progress: bool,
}

pub fn run_project(q: &Request) -> Result<Run, String> {
    let game = romset::games::find(&project_game(q.project, q.game)).map_err(|e| e.to_string())?;
    let mut scenes = project_scenes(game, q.project)?;
    if !q.only.is_empty() {
        scenes.retain(|(n, _)| q.only.contains(n));
    }
    if scenes.is_empty() {
        return Err("no scene of the project has a local copy (`remodel scene make --all`; needs the ROM)".into());
    }
    let owners_path = q.owners_file.map(Path::to_path_buf).unwrap_or_else(|| q.project.join("owners.txt"));
    let (named, owners_file) = match std::fs::read_to_string(&owners_path) {
        Ok(t) => (parse_owners(&t)?, true),
        Err(_) if q.owners_file.is_none() => (Vec::new(), false),
        Err(e) => return Err(format!("{}: {e}", owners_path.display())),
    };
    let collected = crate::sharing_collect::collect(game, q.project, &scenes, q.refresh, q.progress)?;
    let pack = PackIndex::load(&q.pack_dir.map(Path::to_path_buf).unwrap_or_else(|| q.project.join("pack")));
    let report = analyse(&collected.sights, &collected.parts, &named, &|i| pack.side(i), q.limits);
    Ok(Run { collected, report, pack, scenes: scenes.len(), owners_file })
}

fn write_map(project: &Path, run: &Run) -> Result<usize, String> {
    let store = crate::sharing_collect::store_dir(project);
    let dir = project.join("map").join("sharing");
    let tex = dir.join("textures");
    std::fs::create_dir_all(&tex).map_err(|e| e.to_string())?;
    let existing = crate::studio::Project { dir: project.to_path_buf() };
    // The rows other maps have, for the wrapping and group of a key's variants.
    let known: HashMap<String, crate::studio::Inv> = existing
        .inventory()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| i.map != "sharing")
        .map(|i| (i.name.clone(), i))
        .collect();
    let mut px: BTreeMap<(u64, u32), u64> = BTreeMap::new();
    for s in &run.collected.sights {
        *px.entry((s.key, s.pal)).or_insert(0) += s.pixels;
    }
    let mut by_key: BTreeMap<u64, Vec<u32>> = BTreeMap::new();
    for &(k, p) in px.keys() {
        by_key.entry(k).or_default().push(p);
    }
    let mut rows: Vec<(u64, String, &PartInfo, bool)> = Vec::new();
    for (k, pals) in &by_key {
        let infos: Vec<&PartInfo> = pals.iter().filter_map(|p| run.collected.parts.get(&(*k, *p))).collect();
        if pals.len() > 1 {
            for i in &infos {
                rows.push((px[&(*k, i.pal)], i.name(), i, false));
            }
            if let Some(top) = infos.iter().max_by_key(|i| (px[&(*k, i.pal)], std::cmp::Reverse(i.pal))) {
                let plain = format!("tex_{k:016x}_{}x{}", top.w, top.h);
                rows.push((px[&(*k, top.pal)], plain, top, true));
            }
        } else if let Some(i) = infos.first() {
            rows.push((px[&(*k, i.pal)], format!("tex_{k:016x}_{}x{}", i.w, i.h), i, false));
        }
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut inv = String::from(
        "id\tname\tkind\tw\th\tsurface\tframes\tpixels\tbest_frame\tbest_pixels\tpolys\tuv_used_pct\twrap\tspan_u\tspan_v\tgroup\tgroup_size\tdetail\n",
    );
    for (n, (pixels, name, info, fallback)) in rows.iter().enumerate() {
        let src = store.join("orig").join(format!("{}.png", info.name()));
        if let Ok(b) = std::fs::read(&src) {
            let _ = std::fs::write(tex.join(format!("{name}.png")), b);
        }
        let like = known
            .get(name)
            .or_else(|| known.get(&format!("tex_{:016x}_{}x{}", info.key, info.w, info.h)))
            .or_else(|| known.values().find(|i| i.name.starts_with(&format!("tex_{:016x}", info.key))));
        let (wrap, su, sv, group) =
            like.map_or(("rr".to_string(), 2.0, 2.0, 0), |i| (i.wrap.clone(), i.span_u, i.span_v, i.group));
        let _ = writeln!(
            inv,
            "{}\t{}\ttex\t{}\t{}\t{:016x}\t0\t{}\t-\t0\t0\t0\t{}\t{:.2}\t{:.2}\t{}\t1\tpal={:08x} ramp={}{}",
            n + 1,
            name,
            info.w,
            info.h,
            info.surface,
            pixels,
            if wrap.is_empty() || wrap == "-" { "rr" } else { &wrap },
            su,
            sv,
            group,
            info.pal,
            info.ramp,
            if *fallback { " fallback" } else { "" }
        );
    }
    std::fs::write(dir.join("inventory.tsv"), inv).map_err(|e| e.to_string())?;
    std::fs::write(
        dir.join("README.txt"),
        "Written by `remodel studio sharing --write-map`: a row per (key, palette) of every key the project's scenes draw through more than one palette (the original through that palette), and the plain row. Rewritten on every run; the ids mean nothing.\n",
    )
    .map_err(|e| e.to_string())?;
    Ok(rows.len())
}

/// Variant images for every flagged part of a character, into `dir`: the pack's image for the
/// (key, palette) (the original through the palette where the pack has none) repainted to the
/// tone the owner's other parts put it at (`Row::target`). Returns the file names.
fn write_fixes(run: &Run, store: &Path, dir: &Path, lim: &Limits) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // A key seen through one palette is repainted as its plain image, a key through several as a variant.
    let mut pals: HashMap<u64, usize> = HashMap::new();
    for &(k, _) in run.collected.parts.keys() {
        *pals.entry(k).or_insert(0) += 1;
    }
    // A (key, palette) two owners share (Ash's and Alex's neck) is repainted to their pixel-weighted mean.
    let mut by_name: BTreeMap<String, (String, [f64; 3], f64, &Row)> = BTreeMap::new();
    for r in run.report.rows.iter().filter(|r| r.character && !r.flags.is_empty()) {
        let Some(t) = r.target else { continue };
        let name = if pals.get(&r.key) == Some(&1) { format!("tex_{:016x}_{}x{}", r.key, r.w, r.h) } else { r.name() };
        let e = by_name.entry(name.clone()).or_insert((name, [0.0; 3], 0.0, r));
        let w = r.pixels as f64;
        for c in 0..3 {
            e.1[c] += t[c] as f64 * w;
        }
        e.2 += w;
    }
    let mut done = Vec::new();
    let mut tsv = String::from("name\tfrom\tdraw\tbefore\ttarget\n");
    for (name, sum, w, r) in by_name.values() {
        let target = [(sum[0] / w) as f32, (sum[1] / w) as f32, (sum[2] / w) as f32];
        // Only as far as it takes to read as consistent: halfway into the limit, not all the way to the
        // expected tone (a part drawn on purpose in another hue or a shade lighter keeps that).
        let limit = if r.skin { lim.skin } else { lim.other };
        let target = if r.draw != crate::sharing::Draw::Original && r.dev > 0.5 * limit {
            let k = 1.0 - 0.5 * limit / r.dev;
            [
                r.pack[0] + (target[0] - r.pack[0]) * k,
                r.pack[1] + (target[1] - r.pack[1]) * k,
                r.pack[2] + (target[2] - r.pack[2]) * k,
            ]
        } else if r.draw != crate::sharing::Draw::Original {
            r.pack
        } else {
            target
        };
        let src = run.pack.file(r.key, r.pal).map(|(_, p)| p.clone());
        let src = src.unwrap_or_else(|| store.join("orig").join(format!("{}.png", r.name())));
        let Ok(im) = crate::studio_img::load(&src) else { continue };
        let fixed = repaint(&im, target);
        crate::studio_img::save(&dir.join(format!("{name}.png")), &fixed)?;
        let _ = writeln!(
            tsv,
            "{name}\t{}\t{}\t{}\t{}",
            src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            r.draw.name(),
            rgb(r.pack),
            rgb(target)
        );
        done.push(format!("{name}.png"));
    }
    std::fs::write(dir.join("fixes.tsv"), tsv).map_err(|e| e.to_string())?;
    Ok(done)
}

pub fn main(o: &crate::Opts) -> std::process::ExitCode {
    let rest = &o.args[2..];
    let json = rest.iter().any(|a| a == "--json");
    let go = || -> Result<(Answer, String), Answer> {
        let a = Args::parse(
            rest,
            &["--json", "--refresh", "--write-map", "--all-rows", "--help"],
            &[
                "--owners",
                "--pack",
                "--scene",
                "--sheets",
                "--tsv",
                "--fix",
                "--skin-limit",
                "--other-limit",
                "--variant-limit",
            ],
        )?;
        if a.has("--help") {
            return Ok((Answer::done(json!({"usage": USAGE})), USAGE.to_string()));
        }
        let Some(project) = a.pos.first().map(PathBuf::from) else {
            return Err(Answer::fail(Code::BadRequest, format!("studio sharing: which PROJECT?\n{USAGE}")));
        };
        if !project.join("project.txt").is_file() {
            return Err(Answer::fail(
                Code::Missing,
                format!("{}: not a studio project (no project.txt)", project.display()),
            ));
        }
        let num = |k: &str, d: f32| -> Result<f32, Answer> {
            a.one(k).map_or(Ok(d), |v| {
                v.parse::<f32>()
                    .ok()
                    .filter(|x| *x > 0.0)
                    .ok_or_else(|| Answer::fail(Code::BadRequest, format!("{k}: a positive number")))
            })
        };
        let dl = Limits::default();
        let lim = Limits {
            skin: num("--skin-limit", dl.skin)?,
            other: num("--other-limit", dl.other)?,
            variant: num("--variant-limit", dl.variant)?,
            ..dl
        };
        let only: Vec<String> =
            a.one("--scene").map(|s| s.split(',').map(str::to_string).collect()).unwrap_or_default();
        let started = std::time::Instant::now();
        let run = run_project(&Request {
            game: &o.game_id,
            project: &project,
            owners_file: a.one("--owners").map(Path::new),
            pack_dir: a.one("--pack").map(Path::new),
            only: &only,
            limits: &lim,
            refresh: a.has("--refresh"),
            progress: true,
        })
        .map_err(|e| {
            let code =
                if e.starts_with("no scene") || e.contains("No such file") { Code::Missing } else { Code::BadRequest };
            Answer::fail(code, e)
        })?;
        let store = crate::sharing_collect::store_dir(&project);
        let tsv = a.one("--tsv").map(PathBuf::from).unwrap_or_else(|| store.join("sharing.tsv"));
        std::fs::write(&tsv, rows_tsv(&run.report))
            .map_err(|e| Answer::fail(Code::Internal, format!("{}: {e}", tsv.display())))?;
        let mut sheets = Vec::new();
        if let Some(dir) = a.one("--sheets") {
            std::fs::create_dir_all(dir).map_err(|e| Answer::fail(Code::Internal, format!("{dir}: {e}")))?;
            for ow in &run.report.owners {
                let rows: Vec<&Row> = run.report.rows.iter().filter(|r| r.owner == ow.name).take(48).collect();
                if let Some(im) = contact_sheet(&rows, &store, &run.pack, 72) {
                    let p = Path::new(dir).join(format!("owner-{}.png", ow.name));
                    crate::studio_img::save(&p, &im).map_err(|e| Answer::fail(Code::Internal, e))?;
                    sheets.push(p.display().to_string());
                }
            }
        }
        let mut fixed: Vec<String> = Vec::new();
        if let Some(dir) = a.one("--fix") {
            fixed = write_fixes(&run, &store, Path::new(dir), &lim).map_err(|e| Answer::fail(Code::Internal, e))?;
        }
        let mapped = if a.has("--write-map") {
            Some(write_map(&project, &run).map_err(|e| Answer::fail(Code::Internal, e))?)
        } else {
            None
        };
        let r = &run.report;
        let (cf, of) = (r.character_flags(), r.other_flags());
        let flagged: Vec<&Row> = r.rows.iter().filter(|x| a.has("--all-rows") || !x.flags.is_empty()).collect();
        let mut text = String::new();
        let _ = writeln!(
            text,
            "sharing {}: {} scenes ({} read, {} drawn{}), {} (key, palette, owner) rows, {} palettes{}",
            project.display(),
            run.scenes,
            run.collected.cached,
            run.collected.fresh,
            if run.collected.failed.is_empty() {
                String::new()
            } else {
                format!(", {} failed", run.collected.failed.len())
            },
            r.rows.len(),
            run.collected.parts.keys().map(|k| k.1).collect::<BTreeSet<_>>().len(),
            if run.owners_file { "" } else { " (no owners.txt: every owner is clustered)" }
        );
        text.push_str(&owners_text(r));
        let _ = writeln!(text, "flags         {cf} on characters, {of} elsewhere");
        for x in flagged.iter().filter(|x| x.character).take(80) {
            let _ = writeln!(
                text,
                "{:<9} {:<8} {} {:<5} dev {:.2}  {}",
                x.owner,
                x.draw.name(),
                x.name(),
                if x.skin { "skin" } else { "other" },
                x.dev,
                x.flags.iter().map(Flag::text).collect::<Vec<_>>().join(" ")
            );
        }
        let _ = writeln!(text, "rows          {}", tsv.display());
        if let Some(dir) = a.one("--fix") {
            let _ = writeln!(
                text,
                "fixes         {} variant images in {dir} (`studio import`, then `studio approve`)",
                fixed.len()
            );
        }
        for s in &sheets {
            let _ = writeln!(text, "sheet         {s}");
        }
        if let Some(n) = mapped {
            let _ = writeln!(text, "map           {}/map/sharing ({n} rows)", project.display());
        }
        let data = json!({
            "scenes": run.scenes, "read": run.collected.cached, "drawn": run.collected.fresh,
            "failed": run.collected.failed.iter().map(|(n, e)| json!({"scene": n, "error": e})).collect::<Vec<_>>(),
            "owners": r.owners.iter().map(|o| json!({
                "name": o.name, "character": o.named,
                "palettes": o.pals.iter().map(|p| format!("{p:08x}")).collect::<Vec<_>>(),
                "objects": o.objects, "scenes": o.scenes, "stems": o.stems.iter().map(|(n, c)| json!({"stem": n, "scenes": c})).collect::<Vec<_>>(), "pixels": o.pixels, "parts": o.parts, "flags": o.flags,
                "skin_ratio": o.skin_ratio,
            })).collect::<Vec<_>>(),
            "flags": {"characters": cf, "elsewhere": of},
            "limits": {"skin": lim.skin, "other": lim.other, "variant": lim.variant},
            "rows": flagged.iter().take(400).map(|x| json!({
                "owner": x.owner, "character": x.character, "name": x.name(),
                "key": format!("{:016x}", x.key), "pal": format!("{:08x}", x.pal),
                "scenes": x.scenes, "groups": x.groups, "pixels": x.pixels, "share": x.share,
                "draw": x.draw.name(), "kind": if x.skin { "skin" } else { "other" },
                "orig": x.orig, "pack": x.pack, "ratio": x.ratio, "dev": x.dev,
                "flags": x.flags.iter().map(Flag::text).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "rows_total": r.rows.len(), "tsv": tsv.display().to_string(), "sheets": sheets, "fixes": fixed,
            "ms": started.elapsed().as_millis() as u64,
        });
        let mut ans = Answer::done(data);
        for (n, e) in run.collected.failed.iter().take(5) {
            ans = ans.warn(format!("scene {n}: {e}"));
        }
        if cf > 0 {
            ans = Answer::gate(format!("{cf} flag(s) on characters"), ans.data.clone())
                .next(format!("remodel studio sharing {} --sheets DIR", project.display()));
            for (n, e) in run.collected.failed.iter().take(5) {
                ans = ans.warn(format!("scene {n}: {e}"));
            }
        }
        Ok((ans, text))
    };
    match go() {
        Ok((a, text)) => a.emit(json, || text),
        Err(a) => a.emit(json, || USAGE.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sharing::{analyse, Draw, Limits, PackSide};

    /// Needs scenes made from the ROM (`remodel scene make`): skipped, silently, without them.
    #[test]
    fn finds_the_hand_j_alex_and_pnut_share() {
        let game = romset::games::find("topskatr").unwrap();
        let roots = [
            std::env::var_os("REMODEL_DATA").map(|d| PathBuf::from(d).join("topskatr/scenes")),
            Some(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../out/topskatr/scenes"))),
        ];
        let Some(root) = roots.into_iter().flatten().find(|r| r.join("p13-race-j-3600").is_dir()) else { return };
        let names = ["p13-race-j-3600", "alex-03300", "pnut-03300"];
        let scenes: Vec<(String, PathBuf)> = names.iter().map(|n| (n.to_string(), root.join(n))).collect();
        if scenes.iter().any(|(_, d)| !d.join("capture").is_dir()) {
            return;
        }
        let proj = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../out/scratch/sharing-tests"))
            .join(format!("rom-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&proj);
        let c = crate::sharing_collect::collect(game, &proj, &scenes, false, false).unwrap();
        assert!(c.failed.is_empty(), "{:?}", c.failed);
        let r =
            analyse(&c.sights, &c.parts, &[], &|i| PackSide { draw: Draw::Plain, tone: i.orig }, &Limits::default());
        // The hand 3d5cfef2: J's tan, Alex's pale peach, P-Nut's pink, one key.
        let hand: Vec<_> = r.rows.iter().filter(|x| x.key >> 32 == 0x3d5cfef2).collect();
        let pals: std::collections::BTreeSet<u32> = hand.iter().map(|x| x.pal).collect();
        assert!(pals.contains(&0x6615927f) && pals.contains(&0x0a882d08) && pals.contains(&0xbce93cac), "{pals:08x?}");
        let owners: std::collections::BTreeSet<&str> = hand.iter().map(|x| x.owner.as_str()).collect();
        assert!(owners.len() >= 3, "J, Alex and P-Nut are three owners: {owners:?}");
        let tone = |pal: u32| hand.iter().find(|x| x.pal == pal).unwrap().orig;
        assert!(tone(0x6615927f)[0] < tone(0x0a882d08)[0] - 30.0, "J's hand is darker than Alex's");
        // The plain image the pack would draw is flagged where the palettes differ so much.
        assert!(hand.iter().all(|x| x.flags.iter().any(|f| matches!(f, crate::sharing::Flag::NeedsVariant(_)))));
        // A second read comes from the store.
        let again = crate::sharing_collect::collect(game, &proj, &scenes, false, false).unwrap();
        assert_eq!((again.fresh, again.cached), (0, 3));
        let _ = std::fs::remove_dir_all(&proj);
    }
}
