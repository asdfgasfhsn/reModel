//! `remodel catalogue`: reading the catalogue the observer writes
//! (`model2::catalogue`, docs/uplift-studio.md "Catalogue"): the merged
//! index of all sessions, the coverage report, merging stores, and the
//! viewer. Observing itself is `run --catalogue` and the player's
//! `--catalogue`; this side needs no ROM.

use model2::catalogue::{jaccard, Ident, Kind, SCREEN_LIKE};
use model2::texwatch;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// One TSV file as rows of columns by header name; a missing file is no rows.
pub fn read_tsv(path: &Path) -> Vec<HashMap<String, String>> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    let mut lines = text.lines();
    let Some(head) = lines.next() else { return Vec::new() };
    let cols: Vec<&str> = head.split('\t').collect();
    lines
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| cols.iter().zip(l.split('\t')).map(|(c, v)| (c.to_string(), v.to_string())).collect())
        .collect()
}

fn num<T: std::str::FromStr + Default>(r: &HashMap<String, String>, k: &str) -> T {
    r.get(k).and_then(|v| v.parse().ok()).unwrap_or_default()
}

fn hex(r: &HashMap<String, String>, k: &str) -> u64 {
    r.get(k).and_then(|v| u64::from_str_radix(v, 16).ok()).unwrap_or(0)
}

fn text(r: &HashMap<String, String>, k: &str) -> String {
    r.get(k).cloned().unwrap_or_default()
}

/// One entry's rows from one session.
#[derive(Clone, Debug, Default)]
pub struct SeenRow {
    pub samples: u32,
    pub pixels: u64,
    pub best_px: u32,
    pub best_frame: u64,
    pub bbox: [u16; 4],
    pub first: u64,
    pub last: u64,
    /// Local screen -> samples.
    pub screens: BTreeMap<u32, u32>,
    pub wrap: String,
}

#[derive(Clone, Debug, Default)]
pub struct LoadRow {
    pub frame: u64,
    pub offset: u32,
    pub len: u32,
    pub hash: u64,
    pub dest: String,
    pub source: Option<u32>,
    pub screen: Option<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct ScreenRow {
    pub samples: u32,
    pub image: Option<String>,
    pub top: Vec<Ident>,
}

/// A session as stored.
#[derive(Clone, Debug, Default)]
pub struct SessionData {
    pub id: String,
    pub info: BTreeMap<String, String>,
    pub entries: BTreeMap<Ident, EntryRow>,
    pub seen: HashMap<Ident, SeenRow>,
    pub edges: Vec<(Ident, Ident, u64)>,
    pub screens: Vec<ScreenRow>,
    pub loads: Vec<LoadRow>,
    /// Samples: (frame, local screen, saved image or none).
    pub samples: Vec<(u64, u32, Option<String>)>,
}

#[derive(Clone, Debug, Default)]
pub struct EntryRow {
    pub name: String,
    pub w: u32,
    pub h: u32,
    pub surface: u64,
    pub count: u32,
    pub loaded: Option<u64>,
    pub loads: u32,
    pub detail: String,
    pub with: Vec<u64>,
}

impl SessionData {
    pub fn started(&self) -> u64 {
        self.info.get("started").and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    pub fn load(dir: &Path) -> Option<SessionData> {
        let id = dir.file_name()?.to_string_lossy().into_owned();
        let info: BTreeMap<String, String> = std::fs::read_to_string(dir.join("info.txt"))
            .ok()?
            .lines()
            .filter_map(|l| l.split_once(' ').map(|(k, v)| (k.to_string(), v.to_string())))
            .collect();
        let mut s = SessionData { id, info, ..Default::default() };
        for r in read_tsv(&dir.join("entries.tsv")) {
            let Ok(id) = text(&r, "ident").parse::<Ident>() else { continue };
            s.entries.insert(
                id,
                EntryRow {
                    name: text(&r, "name"),
                    w: num(&r, "w"),
                    h: num(&r, "h"),
                    surface: hex(&r, "surface"),
                    count: num(&r, "count"),
                    loaded: r.get("loaded").and_then(|v| v.parse().ok()),
                    loads: num(&r, "loads"),
                    detail: text(&r, "detail"),
                    with: text(&r, "with").split(',').filter_map(|k| u64::from_str_radix(k, 16).ok()).collect(),
                },
            );
        }
        for r in read_tsv(&dir.join("seen.tsv")) {
            let Ok(id) = text(&r, "ident").parse::<Ident>() else { continue };
            let screens = text(&r, "screens")
                .split(',')
                .filter_map(|p| p.split_once(':').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))))
                .collect();
            s.seen.insert(
                id,
                SeenRow {
                    samples: num(&r, "samples"),
                    pixels: num(&r, "pixels"),
                    best_px: num(&r, "best_px"),
                    best_frame: num(&r, "best_frame"),
                    bbox: [num(&r, "x0"), num(&r, "y0"), num(&r, "x1"), num(&r, "y1")],
                    first: num(&r, "first"),
                    last: num(&r, "last"),
                    screens,
                    wrap: text(&r, "wrap"),
                },
            );
        }
        for r in read_tsv(&dir.join("edges.tsv")) {
            if let (Ok(a), Ok(b)) = (text(&r, "a").parse(), text(&r, "b").parse()) {
                s.edges.push((a, b, num(&r, "weight")));
            }
        }
        for r in read_tsv(&dir.join("screens.tsv")) {
            s.screens.push(ScreenRow {
                samples: num(&r, "samples"),
                image: Some(text(&r, "image")).filter(|i| i != "-"),
                top: text(&r, "top").split(',').filter_map(|t| t.parse().ok()).collect(),
            });
        }
        for r in read_tsv(&dir.join("loads.tsv")) {
            s.loads.push(LoadRow {
                frame: num(&r, "frame"),
                offset: hex(&r, "offset") as u32,
                len: num(&r, "len"),
                hash: hex(&r, "hash"),
                dest: text(&r, "dest"),
                source: r.get("source").and_then(|v| u32::from_str_radix(v, 16).ok()),
                screen: r.get("screen").and_then(|v| v.parse().ok()),
            });
        }
        for r in read_tsv(&dir.join("samples.tsv")) {
            s.samples.push((num(&r, "frame"), num(&r, "screen"), Some(text(&r, "image")).filter(|i| i != "-")));
        }
        Some(s)
    }
}

/// An entry over every session.
#[derive(Clone, Debug, Default)]
pub struct Merged {
    pub id: Option<Ident>,
    pub row: EntryRow,
    pub samples: u32,
    pub pixels: u64,
    pub best_px: u32,
    /// Session index and frame of the best sighting, its box.
    pub best: (usize, u64),
    pub bbox: [u16; 4],
    /// (session index, frame) of first and last sight.
    pub first: (usize, u64),
    pub last: (usize, u64),
    pub sessions: BTreeSet<usize>,
    /// Global screens it was seen on, with sample counts.
    pub screens: BTreeMap<usize, u32>,
    /// The first frame in which a load fed it (session index, frame).
    pub loaded: Option<(usize, u64)>,
    pub loads: u32,
    /// The wrapping modes seen (`rr`, `mr`, `rm`, `mm`), comma separated.
    pub wrap: BTreeSet<String>,
    /// Largest polygon count (objects) or tile count (layouts).
    pub count: u32,
    pub with: BTreeSet<u64>,
}

/// A screen over every session: the sessions' screens whose most covering
/// textures and tiles are alike.
#[derive(Clone, Debug, Default)]
pub struct Screen {
    pub members: Vec<(usize, u32)>,
    pub samples: u32,
    pub top: Vec<Ident>,
    /// A saved frame of it: (session index, file name).
    pub image: Option<(usize, String)>,
}

pub struct Index {
    pub dir: PathBuf,
    pub game: String,
    pub sessions: Vec<SessionData>,
    pub entries: BTreeMap<Ident, Merged>,
    pub screens: Vec<Screen>,
    pub edges: HashMap<(Ident, Ident), u64>,
}

impl Index {
    pub fn load(dir: &Path) -> Result<Index, String> {
        let sdir = dir.join("sessions");
        let mut sessions: Vec<SessionData> = std::fs::read_dir(&sdir)
            .map_err(|e| format!("{}: {e} (observe first: `make catalogue`)", sdir.display()))?
            .filter_map(|e| e.ok())
            .filter_map(|e| SessionData::load(&e.path()))
            .collect();
        sessions.sort_by(|a, b| (a.started(), &a.id).cmp(&(b.started(), &b.id)));
        let game = std::fs::read_to_string(dir.join("format.txt"))
            .ok()
            .and_then(|t| t.lines().find_map(|l| l.strip_prefix("game ").map(str::to_string)))
            .unwrap_or_default();
        Ok(Self::merge(dir, game, sessions))
    }

    /// Merge sessions (in order) into the catalogue's view.
    pub fn merge(dir: &Path, game: String, sessions: Vec<SessionData>) -> Index {
        // Global screens: each session's screens join the first earlier
        // screen they are alike, else start one.
        let mut screens: Vec<Screen> = Vec::new();
        let mut global: HashMap<(usize, u32), usize> = HashMap::new();
        for (si, s) in sessions.iter().enumerate() {
            for (li, sc) in s.screens.iter().enumerate() {
                let best = screens
                    .iter()
                    .enumerate()
                    .map(|(g, gs)| (g, jaccard(&gs.top, &sc.top)))
                    .max_by(|a, b| a.1.total_cmp(&b.1))
                    .filter(|b| b.1 >= SCREEN_LIKE);
                let g = match best {
                    Some((g, _)) => g,
                    None => {
                        screens.push(Screen { top: sc.top.clone(), ..Default::default() });
                        screens.len() - 1
                    }
                };
                let gs = &mut screens[g];
                gs.members.push((si, li as u32));
                gs.samples += sc.samples;
                if gs.image.is_none() {
                    gs.image = sc.image.clone().map(|i| (si, i));
                }
                global.insert((si, li as u32), g);
            }
        }
        let mut entries: BTreeMap<Ident, Merged> = BTreeMap::new();
        let mut edges: HashMap<(Ident, Ident), u64> = HashMap::new();
        for (si, s) in sessions.iter().enumerate() {
            for (id, row) in &s.entries {
                let m = entries.entry(*id).or_insert_with(|| Merged {
                    id: Some(*id),
                    row: row.clone(),
                    first: (si, u64::MAX),
                    ..Default::default()
                });
                m.sessions.insert(si);
                m.count = m.count.max(row.count);
                m.row.loads = m.row.loads.max(row.loads);
                m.loads = m.row.loads;
                m.with.extend(row.with.iter().copied());
                if m.loaded.is_none() {
                    m.loaded = row.loaded.map(|f| (si, f));
                }
                if let Some(a) = s.seen.get(id) {
                    m.wrap.extend(a.wrap.split(',').filter(|w| *w != "-" && !w.is_empty()).map(str::to_string));
                    m.samples += a.samples;
                    m.pixels += a.pixels;
                    if a.best_px > m.best_px {
                        m.best_px = a.best_px;
                        m.best = (si, a.best_frame);
                        m.bbox = a.bbox;
                    }
                    if m.first.1 == u64::MAX {
                        m.first = (si, a.first);
                    }
                    m.last = (si, a.last);
                    for (l, n) in &a.screens {
                        if let Some(&g) = global.get(&(si, *l)) {
                            *m.screens.entry(g).or_insert(0) += n;
                        }
                    }
                }
            }
            for (a, b, w) in &s.edges {
                *edges.entry((*a, *b)).or_insert(0) += w;
            }
        }
        Index { dir: dir.to_path_buf(), game, sessions, entries, screens, edges }
    }

    pub fn of_kind(&self, k: Kind) -> impl Iterator<Item = (&Ident, &Merged)> {
        self.entries.iter().filter(move |(i, _)| i.0 == k)
    }

    /// Entries first seen in session `si`.
    pub fn new_in(&self, si: usize) -> Vec<Ident> {
        self.entries.iter().filter(|(_, m)| m.first.0 == si).map(|(i, _)| *i).collect()
    }

    /// The saved picture of the best sighting of `m`, else of the screen
    /// it appears on most, as `frames/NAME`.
    pub fn picture_of(&self, m: &Merged) -> Option<String> {
        let name = format!("{}-{:06}.png", self.sessions.get(m.best.0)?.id, m.best.1);
        if self.dir.join("frames").join(&name).exists() {
            return Some(name);
        }
        let g = m.screens.iter().max_by_key(|(_, n)| **n)?.0;
        self.screens[*g].image.as_ref().map(|(_, n)| n.clone())
    }

    /// Texture entries as sheet rectangles: `(bank, x, y, w, h)`.
    pub fn tex_rects(&self) -> Vec<(Ident, [u32; 5])> {
        self.of_kind(Kind::Tex)
            .filter_map(|(id, m)| {
                let get = |k: &str| m.row.detail.split(' ').find_map(|t| t.strip_prefix(k)?.parse::<u32>().ok());
                Some((*id, [get("bank=")?, get("x=")?, get("y=")?, m.row.w, m.row.h]))
            })
            .collect()
    }
}

/// How much of the loaded texture RAM was drawn from.
#[derive(Clone, Debug, Default)]
pub struct LoadCoverage {
    pub loads: usize,
    pub bytes: u64,
    pub sourced: usize,
    /// Loads (distinct by place and data) at least partly under a drawn
    /// texture's sheet rectangle, and the bytes of those that are.
    pub drawn: usize,
    pub drawn_bytes: u64,
    /// Bytes of the loads that lie under a drawn texture's sheet rectangle.
    pub covered_bytes: u64,
    /// The loads no drawn texture touches, biggest first: (session, row).
    pub never: Vec<(usize, LoadRow)>,
    /// Distinct texture entries a load fed, of all textures drawn.
    pub tex_loaded: usize,
    pub tex_total: usize,
}

pub fn load_coverage(ix: &Index) -> LoadCoverage {
    let mut ranges: Vec<(u32, u32)> =
        ix.tex_rects().iter().flat_map(|(_, r)| texwatch::rect_ranges(r[0], r[1], r[2], r[3], r[4])).collect();
    ranges.sort_unstable();
    let mut merged: Vec<(u32, u32)> = Vec::new();
    for r in ranges {
        match merged.last_mut() {
            Some(l) if r.0 <= l.1 => l.1 = l.1.max(r.1),
            _ => merged.push(r),
        }
    }
    let touches = |a: u32, b: u32| {
        let i = merged.partition_point(|r| r.1 <= a);
        merged.get(i).is_some_and(|r| r.0 < b)
    };
    let mut c = LoadCoverage::default();
    let mut seen: BTreeSet<(u32, u32, u64)> = BTreeSet::new();
    for (si, s) in ix.sessions.iter().enumerate() {
        for l in &s.loads {
            if !seen.insert((l.offset, l.len, l.hash)) {
                continue;
            }
            c.loads += 1;
            c.bytes += l.len as u64;
            c.sourced += l.source.is_some() as usize;
            let (a, b) = (l.offset, l.offset + l.len);
            let i = merged.partition_point(|r| r.1 <= a);
            c.covered_bytes +=
                merged[i..].iter().take_while(|r| r.0 < b).map(|r| r.1.min(b) - r.0.max(a)).sum::<u32>() as u64;
            if touches(a, b) {
                c.drawn += 1;
                c.drawn_bytes += l.len as u64;
            } else {
                c.never.push((si, l.clone()));
            }
        }
    }
    c.never.sort_by(|a, b| b.1.len.cmp(&a.1.len).then(a.1.frame.cmp(&b.1.frame)));
    c.tex_total = ix.of_kind(Kind::Tex).count();
    c.tex_loaded = ix.of_kind(Kind::Tex).filter(|(_, m)| m.loaded.is_some()).count();
    c
}

/// Good-Turing coverage of the entries of a kind: the share of what a new
/// sample would show that the catalogue already holds, 1 - (entries seen
/// in one sample only) / (sightings). A guide, not a measurement.
pub fn coverage_estimate<'a>(entries: impl Iterator<Item = &'a Merged>) -> (usize, usize, f64) {
    let (mut n, mut once, mut total) = (0usize, 0usize, 0u64);
    for m in entries {
        n += 1;
        once += (m.samples == 1) as usize;
        total += m.samples as u64;
    }
    (n, once, if total == 0 { 0.0 } else { 1.0 - once as f64 / total as f64 })
}

fn kb(b: u64) -> String {
    if b >= 1 << 20 {
        format!("{:.1} MB", b as f64 / (1 << 20) as f64)
    } else {
        format!("{} KB", b / 1024)
    }
}

/// The coverage report, as text.
pub fn report(ix: &Index) -> String {
    let mut o = String::new();
    let samples: u64 = ix.sessions.iter().map(|s| s.samples.len() as u64).sum();
    let _ = writeln!(
        o,
        "catalogue {}: {} sessions, {} samples, {} entries, {} screens",
        ix.game,
        ix.sessions.len(),
        samples,
        ix.entries.len(),
        ix.screens.len()
    );
    let _ = writeln!(o, "\nkind     entries  new in the last session  seen once  coverage (Good-Turing estimate)");
    let last = ix.sessions.len().saturating_sub(1);
    for k in Kind::ALL {
        let (n, once, cov) = coverage_estimate(ix.of_kind(k).map(|(_, m)| m));
        let new = ix.of_kind(k).filter(|(_, m)| m.first.0 == last).count();
        let _ = writeln!(o, "{:<8} {n:>7}  {new:>23}  {once:>9}  {:>5.1}%", k.as_str(), cov * 100.0);
    }
    let _ = writeln!(o, "\nsessions (oldest first): id, label, samples, frames, new entries");
    for (si, s) in ix.sessions.iter().enumerate() {
        let g = |k: &str| s.info.get(k).map_or("-", |v| v.as_str());
        let _ = writeln!(
            o,
            "  {} {:<28} {:>5} samples, frames {}-{}, {} new{}",
            s.id,
            g("label"),
            s.samples.len(),
            g("first_frame"),
            g("last_frame"),
            ix.new_in(si).len(),
            if g("dropped") != "0" && g("dropped") != "-" {
                format!(", {} samples dropped", g("dropped"))
            } else {
                String::new()
            }
        );
    }
    let _ = writeln!(o, "\nscreens (most seen first): samples, textures, tiles, layouts, objects, seen once, discovery in the last quarter");
    let mut order: Vec<usize> = (0..ix.screens.len()).collect();
    order.sort_by_key(|&g| std::cmp::Reverse(ix.screens[g].samples));
    for g in order.iter().take(40) {
        let sc = &ix.screens[*g];
        let on: Vec<&Merged> = ix.entries.values().filter(|m| m.screens.contains_key(g)).collect();
        let n = |k: Kind| on.iter().filter(|m| m.id.is_some_and(|i| i.0 == k)).count();
        let once = on.iter().filter(|m| m.screens[g] == 1).count();
        let _ = writeln!(
            o,
            "  screen {g:<3} {:>5} samples  {:>4} tex {:>4} tile {:>3} layout {:>3} object   once {once:>4}   {}",
            sc.samples,
            n(Kind::Tex),
            n(Kind::Tile),
            n(Kind::Layout),
            n(Kind::Object),
            completeness(*g, ix)
        );
    }
    if let Some(s) = ix.sessions.last() {
        let new = ix.new_in(ix.sessions.len() - 1);
        let _ = writeln!(o, "\nnew in the last session ({}): {} entries", s.id, new.len());
        for k in Kind::ALL {
            let n = new.iter().filter(|i| i.0 == k).count();
            if n > 0 {
                let _ = writeln!(o, "  {} {}", n, k.as_str());
            }
        }
    }
    let c = load_coverage(ix);
    let _ = writeln!(o, "\nloaded vs drawn (texture RAM uploads against the textures seen drawn)");
    if c.loads == 0 {
        let _ = writeln!(o, "  no loads recorded (sessions made without the texture watch)");
    } else {
        let _ = writeln!(
            o,
            "  {} loads, {}; {} of them from a ROM address found ({:.0}%); {} loads ({}) are under a drawn texture, {} ({}) are not",
            c.loads,
            kb(c.bytes),
            c.sourced,
            100.0 * c.sourced as f64 / c.loads as f64,
            c.drawn,
            kb(c.drawn_bytes),
            c.loads - c.drawn,
            kb(c.bytes - c.drawn_bytes)
        );
        let _ = writeln!(
            o,
            "  by bytes: {:.0}% of the loaded texture RAM lies under textures seen drawn (the rest was loaded and never drawn, or not yet observed)",
            100.0 * c.covered_bytes as f64 / c.bytes.max(1) as f64
        );
        let _ = writeln!(o, "  textures drawn that a load fed: {} of {}", c.tex_loaded, c.tex_total);
        for (si, l) in c.never.iter().take(10) {
            let _ = writeln!(
                o,
                "  never drawn: frame {} {} +{:#x} {}  ({})",
                l.frame,
                l.dest,
                l.len,
                l.source.map_or(String::new(), |s| format!("from ROM {:#x}", 0x0200_0000 + s)),
                ix.sessions[*si].id
            );
        }
    }
    let mut surf: BTreeMap<u64, usize> = BTreeMap::new();
    for (_, m) in ix.of_kind(Kind::Tex) {
        *surf.entry(m.row.surface).or_insert(0) += 1;
    }
    let shared: usize = surf.values().filter(|&&n| n > 1).count();
    let _ = writeln!(
        o,
        "\nartwork: {} texture keys are {} distinct artworks (surface keys); {} artworks appear under more than one key (other colours, luma tables)",
        ix.of_kind(Kind::Tex).count(),
        surf.len(),
        shared
    );
    o
}

/// How complete a screen looks: the entries first seen in the last
/// quarter of its samples against all of them.
fn completeness(g: usize, ix: &Index) -> String {
    let mut late = 0;
    let mut all = 0;
    for m in ix.entries.values().filter(|m| m.screens.contains_key(&g)) {
        all += 1;
        // Seen on this screen in a single sample: a late discovery is one
        // whose only sightings here are few.
        late += (m.screens[&g] == 1) as usize;
    }
    if all == 0 {
        return "empty".into();
    }
    let frac = late as f64 / all as f64;
    format!(
        "{:.0}% of its entries seen once ({})",
        frac * 100.0,
        if frac < 0.1 {
            "looks complete"
        } else if frac < 0.3 {
            "mostly seen"
        } else {
            "thin: observe it longer"
        }
    )
}

/// `thumbs/tiles.dat`: the tile patterns the sessions saw, by key, as 8x8 RGBA.
pub fn read_atlas(dir: &Path) -> HashMap<u64, Vec<u8>> {
    let b = std::fs::read(dir.join("thumbs").join("tiles.dat")).unwrap_or_default();
    b.as_chunks::<{ model2::catalogue::TILE_RECORD }>()
        .0
        .iter()
        .map(|c| (u64::from_le_bytes(c[..8].try_into().unwrap()), c[8..].to_vec()))
        .collect()
}

/// Whether `dir` is a catalogue (rather than a texmap output).
pub fn is_catalogue(dir: &Path) -> bool {
    dir.join("sessions").is_dir() && dir.join("format.txt").is_file()
}

/// The catalogue as a texmap-shaped map directory the studio reads
/// (`inventory.tsv`, `groups.tsv`, `ids.tsv`, `textures/`): every texture
/// and tile pattern seen, ranked by pixels, grouped by what touches.
/// There are no per-frame ID images (the catalogue keeps none), so the
/// studio's `--frame` selections need a texmap map beside it; `--key`,
/// `--group`, `--surface`, `--all` and the rest work. Returns the entry count.
pub fn export_map(ix: &Index, out: &Path) -> Result<usize, String> {
    let tdir = out.join("textures");
    std::fs::create_dir_all(&tdir).map_err(|e| format!("{}: {e}", tdir.display()))?;
    let mut list: Vec<(&Ident, &Merged)> =
        ix.entries.iter().filter(|(i, _)| matches!(i.0, Kind::Tex | Kind::Tile)).collect();
    list.sort_by(|a, b| b.1.pixels.cmp(&a.1.pixels).then(a.0.cmp(b.0)));
    let id_of: HashMap<Ident, u32> = list.iter().enumerate().map(|(i, (id, _))| (**id, i as u32 + 2)).collect();
    let mut edges = crate::texmap::Edges::default();
    for ((a, b), w) in &ix.edges {
        if let (Some(&ia), Some(&ib)) = (id_of.get(a), id_of.get(b)) {
            edges.0.insert((ia.min(ib), ia.max(ib)), *w);
        }
    }
    let pixels: HashMap<u32, usize> = list.iter().map(|(id, m)| (id_of[*id], m.pixels as usize)).collect();
    let groups = crate::texmap::group(&edges, &pixels, 8, 24);
    let group_of: HashMap<u32, usize> =
        groups.iter().enumerate().flat_map(|(i, g)| g.iter().map(move |&id| (id, i + 1))).collect();
    let mut inv = String::from(
        "id\tname\tkind\tw\th\tsurface\tframes\tpixels\tbest_frame\tbest_pixels\tpolys\tuv_used_pct\twrap\tspan_u\tspan_v\tgroup\tgroup_size\tdetail\n",
    );
    let mut ids = String::from("id\tname\n0\tnone\n1\tflat\n");
    let atlas = read_atlas(&ix.dir);
    for (id, m) in &list {
        let n = id_of[*id];
        let g = group_of.get(&n).copied();
        let _ = writeln!(
            inv,
            "{n}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t-\t{}\t2.00\t2.00\t{}\t{}\t{}",
            m.row.name,
            id.0.as_str(),
            m.row.w,
            m.row.h,
            if id.0 == Kind::Tex { format!("{:016x}", m.row.surface) } else { "-".into() },
            m.samples,
            m.pixels,
            m.best.1,
            m.best_px,
            m.count,
            if m.wrap.is_empty() { "-".to_string() } else { m.wrap.iter().cloned().collect::<Vec<_>>().join(",") },
            g.map_or("-".to_string(), |g| g.to_string()),
            g.map_or(1, |g| groups[g - 1].len()),
            m.row.detail
        );
        let _ = writeln!(ids, "{n}\t{}", m.row.name);
        let dst = tdir.join(format!("{}.png", m.row.name));
        if id.0 == Kind::Tex {
            let _ = std::fs::copy(ix.dir.join("thumbs").join(format!("{}.png", m.row.name)), &dst);
        } else if let Some(px) = atlas.get(&id.1) {
            let _ = std::fs::write(&dst, png::encode_rgba(8, 8, px));
        }
    }
    let mut gt = String::from("group\tsize\tpixels\tmembers\n");
    for (i, g) in groups.iter().enumerate().filter(|(_, g)| g.len() > 1) {
        let px: usize = g.iter().map(|id| pixels[id]).sum();
        let _ = writeln!(
            gt,
            "{}\t{}\t{}\t{}",
            i + 1,
            g.len(),
            px,
            g.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")
        );
    }
    for (name, text) in [("inventory.tsv", inv), ("ids.tsv", ids), ("groups.tsv", gt)] {
        std::fs::write(out.join(name), text).map_err(|e| format!("{}: {e}", out.join(name).display()))?;
    }
    Ok(list.len())
}

/// Capture-shaped directories (`NNNNNN/{meta.txt, tiles.bin, palette.bin}`
/// and empty stream files) of the frames whose tile data the sessions kept,
/// for `studio region-capture --capture-dir`. A frame is numbered
/// `1000000 * (session index + 1) + frame`; `index.tsv` says which is which.
pub fn export_captures(ix: &Index, out: &Path) -> Result<usize, String> {
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut index = String::from("number\tsession\tframe\tscreen\n");
    let mut n = 0;
    for (si, s) in ix.sessions.iter().enumerate() {
        for (frame, screen, image) in &s.samples {
            let Some(image) = image else { continue };
            let z = ix.dir.join("frames").join(image.replace(".png", ".tiles.z"));
            let Ok(raw) = std::fs::read(&z) else { continue };
            let Ok(raw) = png::zlib::decompress(&raw) else { continue };
            let (tl, pl) = (
                u32::from_le_bytes(raw[0..4].try_into().unwrap()) as usize,
                u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize,
            );
            if raw.len() < 8 + tl + pl {
                continue;
            }
            let number = 1_000_000 * (si as u64 + 1) + frame;
            let d = out.join(format!("{number:06}"));
            std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
            let put =
                |f: &str, b: &[u8]| std::fs::write(d.join(f), b).map_err(|e| format!("{}: {e}", d.join(f).display()));
            put("tiles.bin", &raw[8..8 + tl])?;
            put("palette.bin", &raw[8 + tl..8 + tl + pl])?;
            for f in ["stream.bin", "points.bin", "render.bin"] {
                put(f, &[])?;
            }
            put("meta.txt", format!("frame {number}\nxoff 84\nyoff 130\n").as_bytes())?;
            let _ = writeln!(index, "{number}\t{}\t{frame}\t{screen}", s.id);
            n += 1;
        }
    }
    std::fs::write(out.join("index.tsv"), index).map_err(|e| e.to_string())?;
    Ok(n)
}

/// Copy the sessions and pictures of the catalogue at `from` into `to`
/// (merging two stores: sessions never overlap, pictures are named by
/// content or session). Returns the sessions copied.
pub fn merge_stores(from: &Path, to: &Path) -> Result<usize, String> {
    let mut n = 0;
    let copy_dir = |a: &Path, b: &Path| -> Result<usize, String> {
        let mut c = 0;
        std::fs::create_dir_all(b).map_err(|e| format!("{}: {e}", b.display()))?;
        for e in std::fs::read_dir(a).map_err(|e| format!("{}: {e}", a.display()))?.flatten() {
            let dest = b.join(e.file_name());
            if e.path().is_dir() || dest.exists() {
                continue;
            }
            std::fs::copy(e.path(), &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
            c += 1;
        }
        Ok(c)
    };
    for sub in ["thumbs", "frames"] {
        if from.join(sub).is_dir() {
            copy_dir(&from.join(sub), &to.join(sub))?;
        }
    }
    let sdir = from.join("sessions");
    for e in std::fs::read_dir(&sdir).map_err(|e| format!("{}: {e}", sdir.display()))?.flatten() {
        let dest = to.join("sessions").join(e.file_name());
        if dest.exists() {
            continue;
        }
        copy_dir(&e.path(), &dest)?;
        n += 1;
    }
    if !to.join("format.txt").exists() {
        let _ = std::fs::copy(from.join("format.txt"), to.join("format.txt"));
    }
    Ok(n)
}

/// `remodel catalogue SUB`.
pub fn main(args: &[String], game: &str) -> Result<bool, String> {
    let sub = args.first().map(String::as_str).unwrap_or("help");
    let mut rest: Vec<String> = args.iter().skip(1).cloned().collect();
    let mut take = |flag: &str| -> Option<String> {
        let i = rest.iter().position(|a| a == flag)?;
        (i + 1 < rest.len()).then(|| {
            let v = rest.remove(i + 1);
            rest.remove(i);
            v
        })
    };
    let dir = take("--dir").map(PathBuf::from);
    let dir = dir.unwrap_or_else(|| PathBuf::from("out").join(game).join("catalogue"));
    match sub {
        "report" => {
            let ix = Index::load(&dir)?;
            let r = report(&ix);
            print!("{r}");
            std::fs::write(dir.join("report.txt"), &r).map_err(|e| e.to_string())?;
            Ok(true)
        }
        "viewer" => {
            let ix = Index::load(&dir)?;
            let p = crate::catalogue_viewer::write(&ix)?;
            println!("viewer        {}", p.display());
            Ok(true)
        }
        "export-map" => {
            let ix = Index::load(&dir)?;
            let out = rest.first().map(PathBuf::from).unwrap_or_else(|| dir.join("map"));
            let n = export_map(&ix, &out)?;
            println!("map           {n} textures and tile patterns -> {}", out.display());
            Ok(true)
        }
        "captures" => {
            let ix = Index::load(&dir)?;
            let out = rest.first().map(PathBuf::from).unwrap_or_else(|| dir.join("captures"));
            let n = export_captures(&ix, &out)?;
            println!("captures      {n} frames with tile data -> {} (index.tsv: which is which)", out.display());
            Ok(true)
        }
        "glyphs" => {
            let counts = if take("--counts").as_deref() == Some("down") {
                crate::glyphs::Counts::Down
            } else {
                crate::glyphs::Counts::Up
            };
            crate::glyphs::reinfer(&dir.join("glyphs"), counts)?;
            Ok(true)
        }
        "merge" => {
            let from = rest.first().ok_or("usage: remodel catalogue merge SRC_DIR [--dir DEST]")?;
            let n = merge_stores(Path::new(from), &dir)?;
            println!("merged        {n} sessions from {from} into {}", dir.display());
            Ok(true)
        }
        "tsv" => {
            let ix = Index::load(&dir)?;
            let mut o = String::from("ident\tname\tkind\tw\th\tsurface\tsamples\tsessions\tscreens\tbest_px\tfirst\tlast\tloaded\tloads\tdetail\n");
            for (id, m) in &ix.entries {
                let _ = writeln!(
                    o,
                    "{id}\t{}\t{}\t{}\t{}\t{:016x}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    m.row.name,
                    id.0.as_str(),
                    m.row.w,
                    m.row.h,
                    m.row.surface,
                    m.samples,
                    m.sessions.len(),
                    m.screens.len(),
                    m.best_px,
                    ix.sessions[m.first.0].id,
                    ix.sessions[m.last.0].id,
                    m.loaded.map_or("-".into(), |(_, f)| f.to_string()),
                    m.loads,
                    m.row.detail
                );
            }
            std::fs::write(dir.join("index.tsv"), &o).map_err(|e| e.to_string())?;
            println!("index         {}", dir.join("index.tsv").display());
            Ok(true)
        }
        _ => {
            println!(
                "remodel catalogue SUB [--dir DIR]   (DIR default out/GAME/catalogue)\n\
                 \x20 report          coverage, new since last, screens, loaded vs drawn (also report.txt)\n\
                 \x20 viewer          write DIR/index.html, a static browser of the catalogue\n\
                 \x20 export-map [OUT]  the catalogue as a texmap-shaped map (inventory.tsv, groups.tsv, textures/) for `studio init --map`\n\
                 \x20 captures [OUT]    capture-shaped tile data of the saved frames, for `studio region-capture --capture-dir`\n\
                 \x20 tsv             write DIR/index.tsv, one row per entry\n\
                 \x20 merge SRC       copy the sessions and pictures of another store into DIR\n\
                 \x20 glyphs          infer the tile-font mapping again from DIR/glyphs (a `make catalogue-glyphs` trace), --counts up|down\n\
                 observing is `run --catalogue DIR` (make catalogue) and remodel-play --catalogue"
            );
            Ok(sub == "help")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model2::catalogue::{Info, Observation, Seen, Session, Settings};
    use model2::enhance::Rgba;
    use model2::idmap::{Entry, Key, Registry};
    use model2::texwatch::Load;

    /// A scratch directory under out/scratch, removed first.
    fn scratch(name: &str) -> PathBuf {
        let d = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("out")
            .join("scratch")
            .join("catalogue-tests")
            .join(name);
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn registry() -> Registry {
        let mut reg = Registry::default();
        for (key, w) in [(1u64, 32u32), (2, 64)] {
            reg.intern(Key::Tex(key), || Entry {
                key: Key::Tex(key),
                name: format!("tex_{key:016x}_{w}x{w}"),
                w,
                h: w,
                surface: key + 100,
                detail: format!("bank=0 x={} y=0 luma=0 colour=0001", key * 64),
                image: Rgba { width: w as usize, height: w as usize, data: vec![255; (w * w * 4) as usize] },
            });
        }
        reg
    }

    fn tex(key: u64, w: u32, px: u32) -> (Seen, Info) {
        let id = Ident(Kind::Tex, key);
        (
            Seen { id, pixels: px, bbox: [10, 10, 40, 40], count: 1, wraps: 1 },
            Info {
                id,
                name: format!("tex_{key:016x}_{w}x{w}"),
                w,
                h: w,
                surface: key + 100,
                detail: format!("bank=0 x={} y=0 luma=0 colour=0001", key * 64),
                reg_id: Some(key as u32 + 1),
                place: Some([0, key as u32 * 64, 0, w, w]),
                with: Vec::new(),
            },
        )
    }

    fn obs(frame: u64, keys: &[(u64, u32, u32)]) -> Observation {
        let (seen, infos) = keys.iter().map(|&(k, w, px)| tex(k, w, px)).unzip();
        Observation { frame, seen, infos, edges: Vec::new() }
    }

    fn session(dir: &Path, label: &str, runs: &[Observation], loads: Vec<Load>) -> String {
        let mut st = Settings::new(dir.to_path_buf(), "test");
        st.label = label.into();
        let mut s = Session::new(st, None).unwrap();
        s.loads(loads, vec![]);
        let reg = registry();
        for o in runs {
            s.ingest(o, &reg, || model2::catalogue::Shot { rgb: vec![0u8; 496 * 384 * 3], ..Default::default() });
        }
        s.finish().unwrap()
    }

    fn load(frame: u64, offset: u32, len: u32) -> Load {
        Load { frame, offset, len, hash: frame * 7 + offset as u64, head: vec![], mid: vec![] }
    }

    #[test]
    fn sessions_add_up_and_the_index_merges_them() {
        let dir = scratch("merge");
        // Session one sees texture 1 twice; session two sees 1 and 2.
        let a = session(&dir, "first", &[obs(10, &[(1, 32, 500)]), obs(40, &[(1, 32, 900)])], vec![]);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let b = session(&dir, "second", &[obs(5, &[(1, 32, 100), (2, 64, 3000)])], vec![load(3, 64, 4096)]);
        assert_ne!(a, b);
        let ix = Index::load(&dir).unwrap();
        assert_eq!(ix.sessions.len(), 2);
        assert_eq!(ix.sessions[0].id, a, "oldest first");
        let t1 = &ix.entries[&Ident(Kind::Tex, 1)];
        assert_eq!((t1.samples, t1.best_px, t1.sessions.len()), (3, 900, 2));
        assert_eq!(t1.first, (0, 10));
        assert_eq!(t1.last, (1, 5));
        assert_eq!(t1.best, (0, 40), "the best sighting is session one's second sample");
        // What is new in the last session is texture 2 only.
        assert_eq!(ix.new_in(1), vec![Ident(Kind::Tex, 2)]);
        assert!(!ix.screens.is_empty());
        // The report names the kinds, the new entry and loaded vs drawn.
        let r = report(&ix);
        assert!(r.contains("2 sessions") && r.contains("tex") && r.contains("new in the last session"), "{r}");
        assert!(r.contains("loaded vs drawn") && r.contains("1 loads"), "{r}");
    }

    #[test]
    fn merging_stores_is_copying_sessions() {
        let (one, two) = (scratch("store-a"), scratch("store-b"));
        session(&one, "a", &[obs(1, &[(1, 32, 400)])], vec![]);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        session(&two, "b", &[obs(2, &[(2, 64, 800)])], vec![]);
        assert_eq!(merge_stores(&two, &one).unwrap(), 1);
        // A second merge changes nothing.
        assert_eq!(merge_stores(&two, &one).unwrap(), 0);
        let ix = Index::load(&one).unwrap();
        assert_eq!((ix.sessions.len(), ix.entries.len()), (2, 2));
        assert!(one.join("thumbs").read_dir().unwrap().count() >= 2);
    }

    #[test]
    fn loads_join_the_textures_they_fed_and_count_what_was_never_drawn() {
        let dir = scratch("loads");
        // Texture 1 sits at x=64, y=0 (32x32): the first 4 KB page. One
        // load covers it (frame 3); another, far away, no texture.
        let loads = vec![load(3, 0, 4096), load(4, 0x10_0000 - 4096, 4096)];
        session(&dir, "l", &[obs(9, &[(1, 32, 400)])], loads);
        let ix = Index::load(&dir).unwrap();
        let t = &ix.entries[&Ident(Kind::Tex, 1)];
        assert_eq!(t.loaded, Some((0, 3)), "the load that fed it");
        assert!(t.loads >= 1);
        let c = load_coverage(&ix);
        assert_eq!((c.loads, c.drawn), (2, 1));
        assert_eq!(c.never.len(), 1);
        assert_eq!(c.never[0].1.frame, 4);
        assert_eq!((c.tex_loaded, c.tex_total), (1, 1));
    }

    #[test]
    fn coverage_estimate_counts_entries_seen_once() {
        let mk = |s| Merged { samples: s, ..Default::default() };
        let m = [mk(1), mk(1), mk(4), mk(4)];
        let (n, once, cov) = coverage_estimate(m.iter());
        assert_eq!((n, once), (4, 2));
        assert!((cov - 0.8).abs() < 1e-9);
    }

    #[test]
    fn idents_round_trip() {
        let id = Ident(Kind::Layout, 0xdead_beef_0123_4567);
        assert_eq!(id.to_string().parse::<Ident>().unwrap(), id);
        assert!("nope".parse::<Ident>().is_err() && "tex:zz".parse::<Ident>().is_err());
    }
}
