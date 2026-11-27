//! `remodel crawl`: explore a game's menus from a point in a tour, keeping
//! the branches that read ROM nobody had read (docs/uplift/content-coverage.md,
//! "The crawler"; docs/uplift-studio.md, "Tours and coverage").
//!
//! The crawler holds a set of saved states, each reached by a tour. A try
//! loads one, presses a button or two (or holds an analog stick over) for a
//! few frames, waits, and asks what the run read: words of the polygon ROM
//! the geometrizer fetched and texture headers the rasteriser parsed
//! (`model2::readmap`) that no map had marked. If there are any, the state
//! it ended in joins the set and the inputs that reached it are written as a
//! branch tour (`from=PARENT@FRAME,...`) to `DIR/crawl.txt`, a tours file
//! `remodel tour run --tours` takes. Waiting is an input: menus advance on
//! timers.
//!
//! The policy spreads tries: the next try is from the state with the fewest
//! tries for what it has yielded, so a lucky branch does not take them all.
//! Texture headers are sampled every few frames (`--tex-every`, default 6),
//! so a try's texture count is a floor; the polygon ROM is counted exactly,
//! as a bus read. `crawl.map` holds everything the tries read; `coverage`
//! counts it beside the tours' maps. Nothing here changes what the machine
//! does: states are loaded and run, never altered, and a pokes option is
//! asked for by name. A branch's state hash is the same as `tour run` of its
//! line gives (`crawl verify`).

use crate::answer::Answer;
use crate::coverage::{self, MapFile};
use crate::tour::{self, answer, fail, Finish};
use model2::readmap::{Ranges, ReadMap};
use romset::RomSet;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

/// How far a standing poke's range reaches: well past any run.
const STANDING: u64 = 10_000_000;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[self.below(v.len())]
    }
}

/// A node: a state and the tour that reaches it.
struct Node {
    label: String,
    frame: u64,
    /// The whole input from power-on, cut at `frame` (what a child inherits).
    items: Vec<String>,
    state: PathBuf,
    tries: u32,
    kids: u32,
}

/// What the tries may press: the game's buttons (not coins, test or
/// service) and its analog channels.
struct Menu {
    buttons: Vec<&'static str>,
    analogs: Vec<&'static str>,
}

impl Menu {
    fn of(c: &romset::Controls) -> Menu {
        let skip = |n: &str| n.starts_with("coin") || n.starts_with("test") || n.starts_with("service");
        Menu {
            buttons: c.buttons.iter().map(|b| b.name).filter(|n| !skip(n)).collect(),
            analogs: c.analogs.iter().map(|a| a.name).collect(),
        }
    }
}

/// A try's inputs from frame `start` on: one to three presses (sometimes two
/// buttons at once, sometimes a stick held over) and a wait. Returns the
/// terms and the frame the try ends at.
fn plan(rng: &mut Rng, menu: &Menu, start: u64) -> (Vec<String>, u64) {
    let mut terms = Vec::new();
    let mut t = start;
    let presses = if rng.below(10) == 0 { 0 } else { 1 + rng.below(3) };
    for _ in 0..presses {
        let hold = rng.pick(&[2u64, 6, 20, 60]);
        if !menu.analogs.is_empty() && rng.below(3) == 0 {
            let a = rng.pick(&menu.analogs);
            let v = rng.pick(&["0x00", "0xff"]);
            terms.push(format!("{a}={v}@{t}-{}", t + hold - 1));
        } else if !menu.buttons.is_empty() {
            let n = if rng.below(4) == 0 { 2 } else { 1 };
            let mut named: Vec<&str> = Vec::new();
            for _ in 0..n {
                let b = rng.pick(&menu.buttons);
                if !named.contains(&b) {
                    named.push(b);
                    terms.push(format!("{b}@{t}-{}", t + hold - 1));
                }
            }
        }
        t += hold + rng.pick(&[10u64, 60, 180]);
    }
    // Menus advance on timers: now and then wait long enough for a countdown.
    let wait = if rng.below(5) == 0 { 900 } else { rng.pick(&[30u64, 120, 300]) };
    (terms, t + wait)
}

/// The share of `rom`'s non-blank bytes that `read` covers.
fn share(read: &Ranges, blank: &Ranges, words: u64, width: u64) -> Value {
    let used = blank.clip(words).complement(words);
    let all = used.count() * width;
    let got = read.clip(words).intersect(&used).count() * width;
    json!({"bytes_read": got, "non_blank_bytes": all, "share": got as f64 / all.max(1) as f64})
}

/// Options taken out of the arguments.
#[derive(Debug)]
struct Opts {
    from: (String, u64),
    minutes: Option<f64>,
    tries: Option<u64>,
    seed: u64,
    tex_every: u64,
    pokes: Vec<String>,
    maps: Option<PathBuf>,
    keep_max: usize,
}

fn options(args: &mut Vec<String>) -> Result<Opts, String> {
    let from = tour::take_opt(args, "--from").ok_or("--from TOUR@FRAME: where to start")?;
    let (label, frame) = from.split_once('@').ok_or("--from TOUR@FRAME")?;
    let frame = frame.parse().map_err(|_| "--from TOUR@FRAME: FRAME is a frame number")?;
    let mut num = |name: &str| -> Result<Option<f64>, String> {
        tour::take_opt(args, name)
            .map(|v| v.parse::<f64>().map_err(|_| format!("{name} {v:?}: want a number")))
            .transpose()
    };
    let (minutes, tries) = (num("--minutes")?, num("--tries")?.map(|t| t as u64));
    if minutes.is_none() && tries.is_none() {
        return Err("--minutes N or --tries N: how long to explore".into());
    }
    let seed = num("--seed")?.map_or(1, |s| s as u64).max(1);
    let tex_every = num("--tex-every")?.map_or(6, |s| s as u64).max(1);
    let keep_max = num("--keep-max")?.map_or(300, |s| s as usize);
    let pokes = tour::take_opt(args, "--poke").map(|p| p.split(',').map(str::to_string).collect()).unwrap_or_default();
    Ok(Opts {
        from: (label.to_string(), frame),
        minutes,
        tries,
        seed,
        tex_every,
        pokes,
        maps: tour::take_opt(args, "--maps").map(PathBuf::from),
        keep_max,
    })
}

/// `remodel crawl --from TOUR@FRAME (--minutes N | --tries N)` or
/// `remodel crawl verify [--n N]`.
pub fn run(o: &crate::Opts, roms: &RomSet) -> ExitCode {
    let mut args = o.args[1..].to_vec();
    let verify = args.first().is_some_and(|a| a == "verify");
    if verify {
        args.remove(0);
    }
    let c = match tour::common(&mut args) {
        Ok(c) => c,
        Err(e) => return fail(answer::BAD_REQUEST, e, None).finish(false, |_| String::new()),
    };
    let json = c.json;
    let a = if verify { verify_answer(&mut args, &c, roms) } else { crawl_answer(&mut args, &c, roms, json) };
    a.finish(json, |d| {
        let mut s = String::new();
        if let Some(rows) = d["verified"].as_array() {
            for r in rows {
                s.push_str(&format!(
                    "{:<12} frame {:>6}  crawler {}  tour run {}  {}\n",
                    r["label"].as_str().unwrap_or(""),
                    r["frame"],
                    r["crawl_hash"].as_str().unwrap_or(""),
                    r["tour_hash"].as_str().unwrap_or(""),
                    if r["match"] == true { "same" } else { "DIFFERENT" }
                ));
            }
            return s;
        }
        s.push_str(&format!(
            "crawled {} from {}@{}: {} tries in {:.0} s, {} kept\n",
            d["game"].as_str().unwrap_or(""),
            d["from"]["tour"].as_str().unwrap_or(""),
            d["from"]["frame"],
            d["tries"],
            d["seconds"].as_f64().unwrap_or(0.0),
            d["kept"]
        ));
        for k in ["polygon", "texture"] {
            let (b, a) = (&d["coverage"]["before"][k], &d["coverage"]["after"][k]);
            s.push_str(&format!(
                "  {k:<8} read {:.2}% -> {:.2}% of non-blank bytes ({} -> {} KB)\n",
                b["share"].as_f64().unwrap_or(0.0) * 100.0,
                a["share"].as_f64().unwrap_or(0.0) * 100.0,
                b["bytes_read"].as_u64().unwrap_or(0) / 1024,
                a["bytes_read"].as_u64().unwrap_or(0) / 1024
            ));
        }
        for n in d["nodes"].as_array().into_iter().flatten() {
            s.push_str(&format!(
                "  {:<12} frame {:>6}  from {:<12} +{} B polygon, +{} B texture\n",
                n["label"].as_str().unwrap_or(""),
                n["frame"],
                n["parent"].as_str().unwrap_or(""),
                n["new_polygon_bytes"],
                n["new_texture_bytes"]
            ));
        }
        s.push_str(&format!(
            "branch tours: {} (remodel tour run NAME --tours that file); reads: {} (`coverage` counts it)\n",
            d["tours_file"].as_str().unwrap_or(""),
            d["map"].as_str().unwrap_or("")
        ));
        s
    })
}

fn crawl_dir(c: &tour::Common, game: &str) -> PathBuf {
    c.dir.clone().unwrap_or_else(|| tour::default_dir(game).with_file_name("crawl"))
}

fn crawl_answer(args: &mut Vec<String>, c: &tour::Common, roms: &RomSet, json: bool) -> Answer {
    let usage = "remodel crawl --from TOUR@FRAME (--minutes N | --tries N) [--seed S] [--poke NAME,..] [--tex-every N] [--keep-max N] [--dir DIR] [--maps DIR] [--tours FILE] [--json]";
    let opts = match options(args) {
        Ok(o) => o,
        Err(e) => return fail(answer::BAD_REQUEST, e, Some(usage)),
    };
    if let Some(extra) = args.first() {
        return fail(answer::BAD_REQUEST, format!("crawl: unexpected {extra}"), Some(usage));
    }
    let game = roms.game;
    let tours = match tour::load_paths(game, &c.tours) {
        Ok(t) => t,
        Err(e) => return fail(answer::BAD_REQUEST, e, None),
    };
    let names: Vec<&str> = tours.iter().map(|t| t.label.as_str()).collect();
    if !names.contains(&opts.from.0.as_str()) {
        return fail(
            answer::BAD_REQUEST,
            format!("no tour {} for {}{}", opts.from.0, game.id, tour::near(&opts.from.0, &names)),
            Some("remodel tour list"),
        );
    }
    let table = match tour::poke_table(romset::games::pokes(game)) {
        Ok(t) => t,
        Err(e) => return fail(answer::INTERNAL, format!("{} {e}", game.id), None),
    };
    for p in &opts.pokes {
        if !table.iter().any(|t| t.name == *p) {
            let known: Vec<&str> = table.iter().map(|t| t.name.as_str()).collect();
            return fail(
                answer::BAD_REQUEST,
                format!("--poke {p}: no such poke{}", tour::near(p, &known)),
                Some("remodel ram pokes"),
            );
        }
    }
    let dir = crawl_dir(c, game.id);
    let maps_dir = opts.maps.clone().unwrap_or_else(|| tour::default_dir(game.id));
    match crawl(roms, &tours, &table, &opts, &dir, &maps_dir, json) {
        Ok(a) => a,
        Err(e) => fail(answer::INTERNAL, e, None),
    }
}

fn crawl(
    roms: &RomSet,
    tours: &[tour::Tour],
    table: &[romset::pokes::Poke],
    opts: &Opts,
    dir: &Path,
    maps_dir: &Path,
    quiet: bool,
) -> Result<Answer, String> {
    let game = roms.game;
    let started = Instant::now();
    let (root_label, root_frame) = (&opts.from.0, opts.from.1);
    let states = dir.join("states");
    // A fresh crawl: an old one's states and tours are not this one's.
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(&states).map_err(|e| format!("{}: {e}", states.display()))?;

    let mut m = crate::machine(roms);
    if crate::GEO_MODE.get().is_none() {
        m.bus.geo.set_mode(model2::geo::GeoMode::Native);
    }
    let poly_words = m.bus.geo.poly_rom_words();
    let tex_words = m.bus.texture_rom.len();
    let poly_rom = model2::bus::polygon_rom(roms);
    let poly_blank = Ranges::blank(&poly_rom, coverage::BLANK_BYTES / 4);
    let tex_blank = Ranges::blank(&m.bus.texture_rom, coverage::BLANK_BYTES / 2);

    // What the tours have already read: the crawl looks for more.
    let seen_poly = ReadMap::new(poly_words);
    let seen_tex = ReadMap::new(tex_words);
    let mut seeded = Vec::new();
    for t in tours {
        let p = MapFile::path(maps_dir, &t.label);
        if let Ok(mf) = MapFile::load(&p) {
            if mf.game == game.id {
                seen_poly.add(&mf.poly_read);
                seen_tex.add(&mf.tex_read);
                seeded.push(t.label.clone());
            }
        }
    }
    let before = (seen_poly.ranges(), seen_tex.ranges());
    let found_poly = ReadMap::new(poly_words);
    let found_tex = ReadMap::new(tex_words);

    // The root: the tour up to FRAME from power-on, and its state.
    let root_items: Vec<String> =
        tour::resolve_pokes(tour::inputs(tours, root_label, game.controls.coin_start)?, table)?
            .iter()
            .filter_map(|i| tour::cut(i, root_frame))
            .collect();
    let root_script = model2::input::InputScript::parse(&root_items.join(","), &game.controls)?;
    let root_poked = root_script.pokes() || !opts.pokes.is_empty();
    let poked_tag = if root_poked { ".poked" } else { "" };
    let mut seen_hashes: HashSet<u64> = HashSet::new();
    let root_state = states.join(format!("root{poked_tag}.state"));
    {
        let poly = Arc::new(ReadMap::new(poly_words));
        m.bus.geo.set_read_map(Some(poly.clone()));
        let tex = ReadMap::new(tex_words);
        root_script.apply(0, &mut m.bus);
        let mut last = 0;
        while m.frames < root_frame {
            m.run_slice(u64::MAX / 2).map_err(|s| format!("the machine stopped at frame {}: {s:?}", m.frames))?;
            if m.frames != last {
                last = m.frames;
                sample_textures(&mut m, &tex, opts.tex_every);
                if m.frames < root_frame {
                    root_script.apply(m.frames, &mut m.bus);
                }
            }
        }
        std::fs::write(&root_state, model2::save_session(&mut m, None))
            .map_err(|e| format!("{}: {e}", root_state.display()))?;
        found_poly.or_in(&poly);
        found_tex.or_in(&tex);
        seen_poly.or_in(&poly);
        seen_tex.or_in(&tex);
        root_script.apply(root_frame, &mut m.bus);
        seen_hashes.insert(m.bus.state_hash());
    }

    let menu = Menu::of(&game.controls);
    let mut rng = Rng(opts.seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut nodes = vec![Node {
        label: root_label.clone(),
        frame: root_frame,
        items: root_items,
        state: root_state,
        tries: 0,
        kids: 0,
    }];
    let mut kept_rows: Vec<Value> = Vec::new();
    let mut tours_text = format!(
        "# Branch tours found by `remodel crawl --from {root_label}@{root_frame}` ({}), seed {}.\n# Run one with `remodel tour run NAME --tours THIS FILE`.\n",
        game.id, opts.seed
    );
    let mut tries = 0u64;
    let mut kept = 0u32;
    loop {
        if opts.tries.is_some_and(|n| tries >= n)
            || opts.minutes.is_some_and(|mi| started.elapsed().as_secs_f64() >= mi * 60.0)
        {
            break;
        }
        // The state with the fewest tries for what it has yielded.
        let pick = (0..nodes.len())
            .min_by(|&a, &b| {
                let score = |n: &Node| n.tries as f64 / (1.0 + 0.5 * n.kids as f64);
                score(&nodes[a]).total_cmp(&score(&nodes[b])).then(a.cmp(&b))
            })
            .unwrap();
        nodes[pick].tries += 1;
        tries += 1;
        let (frame, parent_label) = (nodes[pick].frame, nodes[pick].label.clone());
        let (mut terms, end) = plan(&mut rng, &menu, frame + 1);
        // Standing pokes: held from the first frame after the branch point.
        let standing: Vec<String> =
            opts.pokes.iter().map(|p| format!("poke={p}@{}-{}", frame + 1, frame + 1 + STANDING)).collect();
        let mut own = standing.clone();
        own.append(&mut terms);
        let mut items = nodes[pick].items.clone();
        items.extend(tour::resolve_pokes(own.clone(), table)?);
        let script = model2::input::InputScript::parse(&items.join(","), &game.controls)?;

        let data = std::fs::read(&nodes[pick].state).map_err(|e| format!("{}: {e}", nodes[pick].state.display()))?;
        model2::load_session(&mut m, None, &data)?;
        if crate::GEO_MODE.get().is_none() {
            m.bus.geo.set_mode(model2::geo::GeoMode::Native);
        }
        let poly = Arc::new(ReadMap::new(poly_words));
        m.bus.geo.set_read_map(Some(poly.clone()));
        let tex = ReadMap::new(tex_words);
        script.apply(m.frames, &mut m.bus);
        let mut last = m.frames;
        while m.frames < end {
            m.run_slice(u64::MAX / 2).map_err(|s| format!("the machine stopped at frame {}: {s:?}", m.frames))?;
            if m.frames != last {
                last = m.frames;
                sample_textures(&mut m, &tex, opts.tex_every);
                if m.frames < end {
                    script.apply(m.frames, &mut m.bus);
                }
            }
        }
        let (new_poly, new_tex) = (poly.count_new(&seen_poly), tex.count_new(&seen_tex));
        found_poly.or_in(&poly);
        found_tex.or_in(&tex);
        seen_poly.or_in(&poly);
        seen_tex.or_in(&tex);
        if new_poly + new_tex == 0 || nodes.len() >= opts.keep_max {
            continue;
        }
        let state_data = model2::save_session(&mut m, None);
        script.apply(end, &mut m.bus);
        let hash = m.bus.state_hash();
        if !seen_hashes.insert(hash) {
            continue;
        }
        kept += 1;
        let label = format!("crawl-{kept:04}");
        let path = states.join(format!("{label}{poked_tag}.state"));
        std::fs::write(&path, state_data).map_err(|e| format!("{}: {e}", path.display()))?;
        let line = format!("{label}|{end}f|from={parent_label}@{frame},{}", own.join(","));
        let line = line.trim_end_matches(',').to_string();
        tours_text.push_str(&format!("# state hash {hash:016x}\n{line}\n"));
        std::fs::write(dir.join("crawl.txt"), &tours_text).map_err(|e| format!("crawl.txt: {e}"))?;
        // A node's own inputs for its children: the parent's, cut at the
        // branch point, then its own (as `tour run` builds them).
        let mut child_items: Vec<String> = nodes[pick].items.iter().filter_map(|i| tour::cut(i, frame)).collect();
        child_items.extend(tour::resolve_pokes(own.clone(), table)?.iter().filter_map(|i| tour::cut(i, end)));
        nodes[pick].kids += 1;
        nodes.push(Node { label: label.clone(), frame: end, items: child_items, state: path, tries: 0, kids: 0 });
        kept_rows.push(json!({
            "label": label, "frame": end, "parent": parent_label, "parent_frame": frame,
            "new_polygon_bytes": new_poly * 4, "new_texture_bytes": new_tex * 2,
            "inputs": own, "hash": format!("{hash:016x}"),
        }));
        if !quiet {
            eprintln!("kept {label} at frame {end} from {parent_label}@{frame}: +{} B polygon, +{} B texture ({tries} tries, {:.0} s)", new_poly * 4, new_tex * 2, started.elapsed().as_secs_f64());
        }
    }

    // The tries' reads, as a map `coverage` counts beside the tours'.
    let map = MapFile {
        game: game.id.to_string(),
        tour: "crawl".into(),
        spec: coverage::fnv("crawl"),
        frames: 0,
        hash: 0,
        poly_words: poly_words as u64,
        tex_words: tex_words as u64,
        poly_read: found_poly.ranges(),
        tex_read: found_tex.ranges(),
        poly_blank: poly_blank.clone(),
        tex_blank: tex_blank.clone(),
    };
    let map_path = dir.join("crawl.map");
    map.save(&map_path).map_err(|e| format!("{}: {e}", map_path.display()))?;
    let (after_poly, after_tex) = (seen_poly.ranges(), seen_tex.ranges());
    let coverage_json = json!({
        "seeded_from": seeded,
        "before": {
            "polygon": share(&before.0, &poly_blank, poly_words as u64, 4),
            "texture": share(&before.1, &tex_blank, tex_words as u64, 2),
        },
        "after": {
            "polygon": share(&after_poly, &poly_blank, poly_words as u64, 4),
            "texture": share(&after_tex, &tex_blank, tex_words as u64, 2),
        },
    });
    let data = json!({
        "game": game.id,
        "from": {"tour": root_label, "frame": root_frame},
        "seed": opts.seed,
        "poked": root_poked,
        "tries": tries,
        "kept": kept,
        "seconds": started.elapsed().as_secs_f64(),
        "coverage": coverage_json,
        "nodes": kept_rows,
        "tours_file": dir.join("crawl.txt").display().to_string(),
        "map": map_path.display().to_string(),
        "dir": dir.display().to_string(),
    });
    std::fs::write(dir.join("crawl.json"), serde_json::to_string_pretty(&data).unwrap_or_default())
        .map_err(|e| format!("crawl.json: {e}"))?;
    let mut a = Answer::done(data);
    if root_poked {
        a = a.warn("pokes were held: these branch tours, their states (.poked.state) and the crawl map are cheat runs");
    }
    if seeded.is_empty() {
        a = a.warn(format!(
            "no tour maps in {} to count against, so everything the crawl read is new (run `remodel tour run --all` first)",
            maps_dir.display()
        ));
    }
    Ok(a.next(format!(
        "remodel crawl verify --game {} (replays the branches with tour run), then remodel coverage --game {}",
        game.id, game.id
    )))
}

/// Parse the texture headers the frame draws, on a copy (the rasteriser's
/// reads, as `tour run` notes them), every `every`th frame.
fn sample_textures(m: &mut model2::Machine, tex: &ReadMap, every: u64) {
    if !m.frames.is_multiple_of(every) {
        return;
    }
    let f = model2::capture::Frame::grab(&mut m.bus);
    let src = f.sources(&m.bus.texture_rom);
    let b = model2::raster::BuildIn { tex_map: Some(tex), ..Default::default() };
    let _ = model2::raster::build_with(&f.stream, &src, &b);
}

/// `remodel crawl verify [--n N]`: replay branches with `tour run`'s own
/// path and compare the state hash the crawler saw.
fn verify_answer(args: &mut Vec<String>, c: &tour::Common, roms: &RomSet) -> Answer {
    let game = roms.game;
    let dir = crawl_dir(c, game.id);
    let n: usize = tour::take_opt(args, "--n").and_then(|v| v.parse().ok()).unwrap_or(3);
    let text = match std::fs::read_to_string(dir.join("crawl.json")) {
        Ok(t) => t,
        Err(_) => {
            return fail(
                answer::MISSING,
                format!("no crawl in {}", dir.display()),
                Some("remodel crawl --from TOUR@FRAME --tries N"),
            )
        }
    };
    let data: Value = match serde_json::from_str(&text) {
        Ok(d) => d,
        Err(e) => return fail(answer::INTERNAL, format!("crawl.json: {e}"), None),
    };
    let crawl_txt = dir.join("crawl.txt");
    let mut files: Vec<&Path> = c.tours.iter().map(PathBuf::as_path).collect();
    files.push(&crawl_txt);
    let tours = match tour::load_many(game, &files) {
        Ok(t) => t,
        Err(e) => return fail(answer::BAD_REQUEST, e, None),
    };
    let mut nodes: Vec<&Value> = data["nodes"].as_array().into_iter().flatten().collect();
    // The deepest first: a long chain of loads is the hard case.
    nodes.sort_by_key(|v| std::cmp::Reverse(v["frame"].as_u64().unwrap_or(0)));
    let tmp = dir.join("verify");
    if let Err(e) = std::fs::create_dir_all(tmp.join("states")) {
        return fail(answer::INTERNAL, e.to_string(), None);
    }
    let mut rows = Vec::new();
    for v in nodes.into_iter().take(n) {
        let label = v["label"].as_str().unwrap_or("");
        let Some(t) = tours.iter().find(|t| t.label == label) else {
            return fail(answer::INTERNAL, format!("crawl.txt has no tour {label}"), None);
        };
        let (script, poked) = match tour::build_script(game, &tours, label) {
            Ok(s) => s,
            Err(e) => return fail(answer::BAD_REQUEST, e, None),
        };
        match tour::run_one(roms, t, &script, &tmp, poked) {
            Ok((row, _)) => {
                let h = row["state_hash"].as_str().unwrap_or("").to_string();
                rows.push(json!({
                    "label": label, "frame": v["frame"], "crawl_hash": v["hash"], "tour_hash": h,
                    "match": v["hash"].as_str() == Some(h.as_str()),
                }));
            }
            Err(e) => return fail(answer::INTERNAL, e, None),
        }
    }
    let bad = rows.iter().filter(|r| r["match"] != true).count();
    let _ = std::fs::remove_dir_all(&tmp);
    let a = Answer::done(json!({"game": game.id, "verified": rows, "different": bad}));
    if bad > 0 {
        return Answer::gate(format!("{bad} branch(es) did not replay to the crawler's state hash"), a.data.clone());
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A try's inputs start after the branch point (a branch cannot rewrite
    /// the past), name only buttons the game has (never a coin, test or
    /// service switch), and parse as an input script; the same seed makes
    /// the same tries.
    #[test]
    fn tries_are_valid_inputs_after_the_branch_point() {
        for g in romset::games::ALL {
            let menu = Menu::of(&g.controls);
            assert!(menu
                .buttons
                .iter()
                .all(|b| !b.starts_with("coin") && !b.starts_with("test") && !b.starts_with("service")));
            let (mut a, mut b) = (Rng(7), Rng(7));
            for _ in 0..200 {
                let (terms, end) = plan(&mut a, &menu, 1301);
                assert_eq!(plan(&mut b, &menu, 1301), (terms.clone(), end));
                assert!(end > 1301);
                for t in &terms {
                    let (_, frames) = t.rsplit_once('@').unwrap();
                    let from: u64 = frames.split('-').next().unwrap().parse().unwrap();
                    assert!(from > 1300 && from < end, "{t} in {}", g.id);
                }
                model2::input::InputScript::parse(&terms.join(","), &g.controls)
                    .unwrap_or_else(|e| panic!("{}: {e}", g.id));
            }
        }
    }

    #[test]
    fn options_need_a_start_and_a_length() {
        let args = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        assert!(options(&mut args("--tries 5")).unwrap_err().contains("--from"));
        assert!(options(&mut args("--from coin-start@10")).unwrap_err().contains("--minutes"));
        let o = options(&mut args("--from coin-start@1300 --tries 5 --seed 3 --poke a,b")).unwrap();
        assert_eq!(o.from, ("coin-start".to_string(), 1300));
        assert_eq!((o.tries, o.seed, o.pokes), (Some(5), 3, vec!["a".to_string(), "b".to_string()]));
        assert!(options(&mut args("--from coin-start@x --tries 5")).is_err());
    }
}
