//! The catalogue: every asset a game shows, built by observing it
//! (docs/uplift-studio.md, "Catalogue"). Render and observation side only:
//! nothing here changes what the emulation does or its state.
//!
//! What is catalogued, as [`Kind`]s: 3D textures (`tex`: the texture dump's
//! key, with its surface key, size, luma table, colour and wrap), 2D tile
//! patterns (`tile`), 2D layouts (`layout`: a connected arrangement of
//! tiles such as a logo, a label or a number field, identified by its
//! tiles and by its shape alone) and objects (`object`: one model's
//! polygon list, by its model address when the geometrizer's side channel
//! knows it, else by the set of polygon headers). Beside what is drawn the
//! catalogue records what is *loaded* (`texwatch`: texture RAM uploads with
//! their destination, a hash and, where it is a verbatim copy, the ROM
//! address) and joins the two: a texture knows when it was loaded, a
//! loaded region knows what was drawn from it.
//!
//! # Store
//!
//! `out/<game>/catalogue/` holds one directory per observing session
//! (`sessions/<id>/`) and the pictures all sessions share (`thumbs/`,
//! `frames/`). Everything a session writes is plain TSV and either
//! append-only (`samples.tsv`, `loads.tsv`) or rewritten whole and
//! atomically every half minute and at the end (`entries.tsv`, `seen.tsv`,
//! `edges.tsv`, `screens.tsv`, `info.txt`), so a killed session loses at
//! most its last half minute. Sessions never touch each other's files, so
//! merging two catalogues is copying directories (`remodel catalogue
//! merge`), and the index every report reads is computed from the
//! sessions.
//!
//! # Observing while playing
//!
//! [`Catalogue`] is the game thread's side: `tick` once per emulated frame
//! drains the texture watch (a version-counter compare unless the game
//! wrote texels) and, every N frames or when the tile layer was rewritten
//! or textures were loaded, copies the frame's renderer inputs
//! (`capture::Frame`, about 4 MB) to a worker thread, which identifies,
//! renders and writes. A busy worker drops the sample, never the game's
//! time.

use crate::capture::Frame;
use crate::enhance::{Fnv, Rgba};
use crate::idmap::{self, Edges, IdFrame, Key, Registry, FIRST_ID};
use crate::light;
use crate::raster::{self, Poly};
use crate::texwatch::{self, Load, RomIndex};
use crate::video::{self, View};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;

/// The store's format number (`format.txt`).
pub const FORMAT: u32 = 1;

/// What an entry is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum Kind {
    Tex,
    Tile,
    Layout,
    Object,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Tex, Kind::Tile, Kind::Layout, Kind::Object];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Tex => "tex",
            Kind::Tile => "tile",
            Kind::Layout => "layout",
            Kind::Object => "object",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// An entry's identity: kind and 64-bit key, written `kind:0123456789abcdef`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Ident(pub Kind, pub u64);

impl std::fmt::Display for Ident {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}:{:016x}", self.0.as_str(), self.1)
    }
}

impl std::str::FromStr for Ident {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let (k, v) = s.split_once(':').ok_or_else(|| format!("bad entry id {s:?}"))?;
        let kind = Kind::parse(k).ok_or_else(|| format!("bad entry kind in {s:?}"))?;
        Ok(Ident(kind, u64::from_str_radix(v, 16).map_err(|_| format!("bad entry key in {s:?}"))?))
    }
}

/// One entry's sighting in one sample.
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    pub id: Ident,
    /// Visible pixels (layouts: 64 per tile).
    pub pixels: u32,
    /// Where: x0, y0, x1, y1 in game pixels (inclusive).
    pub bbox: [u16; 4],
    /// Polygons (objects), tiles (layouts), else 0.
    pub count: u32,
    /// Textures: the wrapping used, a bit per way (1 mirror in u, 2 mirror
    /// in v: `idmap::wrap_names`); 0 otherwise.
    pub wraps: u8,
}

/// What an entry is, as far as one sighting tells.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub id: Ident,
    /// File stem (`tex_<key>_<w>x<h>`, `tile_<key>`, `layout_<key>`,
    /// `object_<key>`).
    pub name: String,
    pub w: u32,
    pub h: u32,
    /// Surface key (textures), shape key (layouts), model address (objects).
    pub surface: u64,
    pub detail: String,
    /// The [`Registry`] ID of a texture or tile (its image is there).
    pub reg_id: Option<u32>,
    /// Texture sheet placement `[bank, x, y, w, h]`.
    pub place: Option<[u32; 5]>,
    /// Objects: the keys of the textures it shows.
    pub with: Vec<u64>,
}

/// A frame as the catalogue sees it.
#[derive(Clone, Debug, Default)]
pub struct Observation {
    pub frame: u64,
    pub seen: Vec<Seen>,
    pub infos: Vec<Info>,
    /// Touching pairs of textures and tiles (one object block, one layer),
    /// by touching pixel pairs.
    pub edges: Vec<(Ident, Ident, u64)>,
}

fn hash_parts(parts: &[u64]) -> u64 {
    let mut f = Fnv::default();
    for p in parts {
        f.bytes(&p.to_le_bytes());
    }
    f.0
}

/// A frame identified: its [`Observation`], and what drawing its picture
/// needs.
pub struct Observed<'a> {
    pub obs: Observation,
    frame: &'a Frame,
    polys: Vec<Poly>,
    src: raster::Sources<'a>,
    view: View,
}

/// What a saved frame keeps: its picture and, for `catalogue captures`
/// (studio region capture), the tile RAM and palette it was drawn from.
#[derive(Default)]
pub struct Shot {
    /// Native 496x384 RGB.
    pub rgb: Vec<u8>,
    pub tiles: Vec<u8>,
    pub palette: Vec<u8>,
}

impl Observed<'_> {
    /// The frame as the game shows it, with its tile data.
    pub fn picture(mut self) -> Shot {
        let f = self.frame;
        let rgb = if self.polys.is_empty() {
            video::compose(&f.tiles, &f.palette, None, &self.view)
        } else {
            let layer = raster::draw_view_depth(&mut self.polys, &self.src, &self.view, false);
            video::compose(&f.tiles, &f.palette, Some((&layer.0, &layer.1)), &self.view)
        };
        Shot { rgb, tiles: f.tiles.clone(), palette: f.palette.clone() }
    }
}

/// Most frames whose tile data is kept (about 30 KB each, compressed).
const SNAPSHOT_CAP: usize = 300;

/// Tile instances' anchor: the pixel at pattern position (3, 3).
const ANCHOR: [f32; 2] = [3.5, 3.5];

/// The smallest tile arrangement that is a layout, and the largest (a
/// whole scrolling background is not).
const LAYOUT_TILES: std::ops::RangeInclusive<usize> = 4..=600;

/// Identify what a captured frame draws.
pub fn observe<'a>(
    f: &'a Frame,
    rom: &'a [u16],
    reg: &mut Registry,
    windows: (&[usize], &[usize]),
    side: Option<&light::Frame>,
) -> Observed<'a> {
    let view = View::native().with_hud_windows(windows.0, windows.1);
    let src = f.sources(rom);
    let mut polys = raster::build_shaded(&f.stream, &src, None);
    video::HudLayout::new(&f.tiles, &f.palette, &view).anchor(&mut polys, &view);
    let placed = raster::place(&mut polys, &view);
    let (vis, uv) = raster::visible_uv(&polys, &placed, &src, &view);
    let tiles = video::tile_pixels(&f.tiles, &f.palette);
    let ids = idmap::identify(&polys, &placed, &vis, &uv, Some(&tiles), &view, &src, reg);
    let mut obs = Observation { frame: f.frame, ..Default::default() };
    let stats = idmap::stats(&ids, reg);
    let mut by_id: HashMap<u32, Ident> = HashMap::new();
    // A texture key drawn through several palettes has an ID for each
    // (`idmap::Registry::intern_pal`); the catalogue's entry is the key, so
    // its sightings are added up and the palettes listed (`pals=`).
    let mut at: HashMap<Ident, usize> = HashMap::new();
    for s in stats.iter().filter(|s| s.id >= FIRST_ID) {
        let Some(e) = reg.entry(s.id) else { continue };
        let id = match e.key {
            Key::Tex(k) => Ident(Kind::Tex, k),
            Key::Tile(k) => Ident(Kind::Tile, k),
        };
        by_id.insert(s.id, id);
        let bbox = [s.bbox[0] as u16, s.bbox[1] as u16, s.bbox[2] as u16, s.bbox[3] as u16];
        if let Some(&i) = at.get(&id) {
            let seen = &mut obs.seen[i];
            seen.pixels += s.pixels as u32;
            seen.count += s.polys as u32;
            seen.wraps |= s.wraps;
            seen.bbox = [
                seen.bbox[0].min(bbox[0]),
                seen.bbox[1].min(bbox[1]),
                seen.bbox[2].max(bbox[2]),
                seen.bbox[3].max(bbox[3]),
            ];
            let info = &mut obs.infos[i];
            if let Some(pal) = e.detail.split(' ').find_map(|t| t.strip_prefix("pal=")) {
                if info.detail.split(' ').any(|t| t.starts_with("pals=")) {
                    info.detail.push_str(&format!(",{pal}"));
                } else {
                    let first = info.detail.split(' ').find_map(|t| t.strip_prefix("pal=")).unwrap_or("").to_string();
                    info.detail.push_str(&format!(" pals={first},{pal}"));
                }
            }
            continue;
        }
        at.insert(id, obs.seen.len());
        obs.seen.push(Seen { id, pixels: s.pixels as u32, bbox, count: s.polys as u32, wraps: s.wraps });
        obs.infos.push(Info {
            id,
            name: e.name.clone(),
            w: e.w,
            h: e.h,
            surface: e.surface,
            detail: e.detail.clone(),
            reg_id: Some(s.id),
            place: place_of(&e.detail, e.w, e.h),
            with: Vec::new(),
        });
    }
    let mut edges = Edges::default();
    edges.add_frame(&ids);
    for (&(a, b), &w) in &edges.0 {
        if let (Some(&ia), Some(&ib)) = (by_id.get(&a), by_id.get(&b)) {
            obs.edges.push((ia.min(ib), ia.max(ib), w));
        }
    }
    layouts(&ids, reg, &mut obs);
    objects(&polys, &ids, &by_id, side, &mut obs);
    Observed { obs, frame: f, polys, src, view }
}

/// `bank=B x=X y=Y ...` of a texture's detail, with its size.
fn place_of(detail: &str, w: u32, h: u32) -> Option<[u32; 5]> {
    let get = |k: &str| detail.split(' ').find_map(|t| t.strip_prefix(k)?.parse::<u32>().ok());
    Some([get("bank=")?, get("x=")?, get("y=")?, w, h])
}

/// Layouts: 8-connected arrangements of tile instances on one layer.
fn layouts(ids: &IdFrame, reg: &Registry, obs: &mut Observation) {
    // One anchor pixel per tile instance: (layer, column, row) -> pattern.
    let mut cells: HashMap<(u8, i32, i32), (u64, [u16; 2])> = HashMap::new();
    for y in 3..ids.height {
        for x in 3..ids.width {
            let o = y * ids.width + x;
            if ids.class[o] >> 31 == 0 || ids.uv[o] != ANCHOR {
                continue;
            }
            if let Some(Key::Tile(k)) = reg.entry(ids.ids[o]).map(|e| e.key) {
                let layer = (ids.class[o] & 1) as u8;
                cells.insert((layer, ((x - 3) / 8) as i32, ((y - 3) / 8) as i32), (k, [x as u16, y as u16]));
            }
        }
    }
    let mut left: BTreeSet<(u8, i32, i32)> = cells.keys().copied().collect();
    while let Some(&start) = left.iter().next() {
        left.remove(&start);
        let mut comp = vec![start];
        let mut i = 0;
        while i < comp.len() {
            let (l, cx, cy) = comp[i];
            i += 1;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let n = (l, cx + dx, cy + dy);
                    if left.remove(&n) {
                        comp.push(n);
                    }
                }
            }
        }
        let keys: BTreeSet<u64> = comp.iter().map(|c| cells[c].0).collect();
        if !LAYOUT_TILES.contains(&comp.len()) || keys.len() < 2 {
            continue;
        }
        let (x0, y0) = (comp.iter().map(|c| c.1).min().unwrap(), comp.iter().map(|c| c.2).min().unwrap());
        let (x1, y1) = (comp.iter().map(|c| c.1).max().unwrap(), comp.iter().map(|c| c.2).max().unwrap());
        let mut parts: Vec<(i32, i32, u64)> = comp.iter().map(|c| (c.1 - x0, c.2 - y0, cells[c].0)).collect();
        parts.sort();
        let layer = start.0 as u64;
        let shape = hash_parts(&parts.iter().flat_map(|p| [p.0 as u64, p.1 as u64]).chain([layer]).collect::<Vec<_>>());
        let key =
            hash_parts(&parts.iter().flat_map(|p| [p.0 as u64, p.1 as u64, p.2]).chain([layer]).collect::<Vec<_>>());
        let px: Vec<[u16; 2]> = comp.iter().map(|c| cells[c].1).collect();
        let bbox = [
            px.iter().map(|p| p[0]).min().unwrap().saturating_sub(3),
            px.iter().map(|p| p[1]).min().unwrap().saturating_sub(3),
            px.iter().map(|p| p[0]).max().unwrap() + 4,
            px.iter().map(|p| p[1]).max().unwrap() + 4,
        ];
        let id = Ident(Kind::Layout, key);
        let (w, h) = ((x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32);
        obs.seen.push(Seen { id, pixels: comp.len() as u32 * 64, bbox, count: comp.len() as u32, wraps: 0 });
        obs.infos.push(Info {
            id,
            name: format!("layout_{key:016x}"),
            w,
            h,
            surface: shape,
            detail: format!(
                "shape={shape:016x} cells={w}x{h} tiles={} layer={}",
                comp.len(),
                ["low", "high"][layer as usize]
            ),
            reg_id: None,
            place: None,
            with: Vec::new(),
        });
    }
}

/// Objects: one per object block of the frame, by model address when the
/// side channel knows it, else by the set of its polygon headers.
fn objects(
    polys: &[Poly],
    ids: &IdFrame,
    by_id: &HashMap<u32, Ident>,
    side: Option<&light::Frame>,
    obs: &mut Observation,
) {
    #[derive(Default)]
    struct Block {
        headers: BTreeSet<[u16; 4]>,
        polys: u32,
        pixels: u32,
        bbox: [u16; 4],
        tex: BTreeSet<u64>,
    }
    let mut blocks: BTreeMap<u32, Block> = BTreeMap::new();
    for p in polys {
        let b = blocks.entry(p.block).or_default();
        b.headers.insert(p.header);
        b.polys += 1;
        b.bbox = [u16::MAX, u16::MAX, 0, 0];
    }
    for y in 0..ids.height {
        for x in 0..ids.width {
            let o = y * ids.width + x;
            let c = ids.class[o];
            if ids.ids[o] == idmap::NONE_ID || c >> 31 != 0 {
                continue;
            }
            let Some(b) = blocks.get_mut(&(c & 0xf_ffff)) else { continue };
            b.pixels += 1;
            b.bbox =
                [b.bbox[0].min(x as u16), b.bbox[1].min(y as u16), b.bbox[2].max(x as u16), b.bbox[3].max(y as u16)];
            if let Some(Ident(Kind::Tex, k)) = by_id.get(&ids.ids[o]) {
                b.tex.insert(*k);
            }
        }
    }
    // One entry per object: the same model drawn twice adds up.
    let mut found: BTreeMap<u64, (Seen, Info)> = BTreeMap::new();
    for (&n, b) in blocks.iter().filter(|(_, b)| b.pixels > 0) {
        let oba = side.and_then(|s| s.blocks.get(n as usize)).map_or(0, |bl| bl.oba);
        let headers = hash_parts(&b.headers.iter().flat_map(|h| h.map(|v| v as u64)).collect::<Vec<_>>());
        let key = if oba != 0 { hash_parts(&[0x006f_6261, oba as u64]) } else { headers };
        let e = found.entry(key).or_insert_with(|| {
            let id = Ident(Kind::Object, key);
            (
                Seen { id, pixels: 0, bbox: [u16::MAX, u16::MAX, 0, 0], count: 0, wraps: 0 },
                Info {
                    id,
                    name: format!("object_{key:016x}"),
                    w: 0,
                    h: 0,
                    surface: oba as u64,
                    detail: format!(
                        "polys={} headers={} {}",
                        b.polys,
                        b.headers.len(),
                        if oba != 0 {
                            format!("oba=0x{oba:06x}")
                        } else {
                            "oba=- (keyed by its polygon headers)".into()
                        }
                    ),
                    reg_id: None,
                    place: None,
                    with: Vec::new(),
                },
            )
        });
        e.0.pixels += b.pixels;
        e.0.count += b.polys;
        e.0.bbox = [
            e.0.bbox[0].min(b.bbox[0]),
            e.0.bbox[1].min(b.bbox[1]),
            e.0.bbox[2].max(b.bbox[2]),
            e.0.bbox[3].max(b.bbox[3]),
        ];
        e.1.with.extend(b.tex.iter().copied());
    }
    for (_, (s, mut i)) in found {
        i.with.sort_unstable();
        i.with.dedup();
        i.w = s.count;
        obs.seen.push(s);
        obs.infos.push(i);
    }
}

// ---------------------------------------------------------------------
// The session's store.

/// What the session knows of one entry.
#[derive(Clone, Debug)]
struct Row {
    info: Info,
    /// The frame whose load fed it at first sight, and the most loads
    /// seen overlapping its texels.
    loaded: Option<u64>,
    loads: u32,
    /// Largest `Seen::count` (polygons of an object, tiles of a layout).
    count: u32,
    with: BTreeSet<u64>,
    /// Texture palette ids (`pal=`, `pals=` of the detail) seen over the
    /// samples: the key drawn through each (`enhance::palette_id`).
    pals: BTreeSet<String>,
}

/// The palette ids a texture detail names (`pal=` and `pals=`).
fn detail_pals(detail: &str) -> impl Iterator<Item = &str> {
    detail
        .split(' ')
        .filter_map(|t| t.strip_prefix("pals=").or_else(|| t.strip_prefix("pal=")))
        .flat_map(|v| v.split(','))
        .filter(|p| !p.is_empty())
}

/// A texture detail as `entries.tsv` writes it: the first sighting's `pal=`
/// and `ramp=` stay, and `pals=` lists every palette the key was drawn
/// through when it was more than one.
fn detail_with_pals(detail: &str, pals: &BTreeSet<String>) -> String {
    let mut out: Vec<&str> = detail.split(' ').filter(|t| !t.starts_with("pals=")).collect();
    let list;
    if pals.len() > 1 {
        list = format!("pals={}", pals.iter().cloned().collect::<Vec<_>>().join(","));
        out.push(&list);
    }
    out.join(" ")
}

#[derive(Clone, Debug, Default)]
struct Agg {
    samples: u32,
    pixels: u64,
    best_px: u32,
    best_frame: u64,
    best_bbox: [u16; 4],
    wraps: u8,
    first: u64,
    last: u64,
    screens: BTreeMap<u32, u32>,
}

#[derive(Clone, Debug)]
struct Screen {
    top: Vec<Ident>,
    samples: u32,
    first: u64,
    last: u64,
    image: Option<String>,
}

struct LoadRow {
    load: Load,
    src: Option<u32>,
}

/// Settings of an observing session.
#[derive(Clone, Debug)]
pub struct Settings {
    /// The catalogue directory (`out/<game>/catalogue`).
    pub dir: PathBuf,
    pub game: String,
    /// What this run is (an input script, "played").
    pub label: String,
    /// Frames between samples.
    pub every: u64,
    /// The most frame pictures the session saves.
    pub frame_cap: usize,
    /// The game's HUD windows (`View::with_hud_windows`).
    pub windows: (Vec<usize>, Vec<usize>),
    /// Wait for the worker instead of dropping a sample it has no time
    /// for (scripted runs that go faster than real time).
    pub block: bool,
}

impl Settings {
    pub fn new(dir: PathBuf, game: &str) -> Self {
        Settings {
            dir,
            game: game.into(),
            label: String::new(),
            every: 30,
            frame_cap: 600,
            windows: (vec![], vec![]),
            block: false,
        }
    }
}

/// A record of `thumbs/tiles.dat`: key (8 bytes) and the pattern's pixels.
pub const TILE_RECORD: usize = 8 + 8 * 8 * 4;

/// What a screen is recognised by: a texture by its artwork (surface key,
/// whatever colour it is drawn in), a tile by its number (its pattern
/// whatever palette or fade it is shown in).
fn screen_sig(i: &Info) -> Ident {
    match i.id.0 {
        Kind::Tex => Ident(Kind::Tex, i.surface),
        Kind::Tile => {
            let n = i.detail.split(' ').find_map(|t| t.strip_prefix("tile=")?.parse::<u64>().ok());
            Ident(Kind::Tile, n.unwrap_or(i.id.1))
        }
        _ => i.id,
    }
}

/// How many top entries identify a screen, and how alike two screens' top
/// lists must be (Jaccard) to be one.
pub const SCREEN_TOP: usize = 24;
pub const SCREEN_LIKE: f64 = 0.5;

pub fn jaccard<T: Ord>(a: &[T], b: &[T]) -> f64 {
    let (sa, sb): (BTreeSet<&T>, BTreeSet<&T>) = (a.iter().collect(), b.iter().collect());
    let both = sa.intersection(&sb).count();
    let all = sa.len() + sb.len() - both;
    if all == 0 {
        1.0
    } else {
        both as f64 / all as f64
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Write `text` to `path` through a temporary file, so a reader (or a
/// crash) never sees half of it.
pub fn write_atomic(path: &Path, text: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// One observing session's writer.
pub struct Session {
    dir: PathBuf,
    root: PathBuf,
    id: String,
    settings: Settings,
    started: u64,
    entries: HashMap<Ident, Row>,
    aggs: HashMap<Ident, Agg>,
    edges: HashMap<(Ident, Ident), u64>,
    screens: Vec<Screen>,
    page_loaded: HashMap<u32, (u64, u32)>,
    pending: Vec<LoadRow>,
    rom_index: Option<RomIndex>,
    samples_log: std::fs::File,
    loads_log: std::fs::File,
    thumbs: HashSet<String>,
    /// The tile patterns already in `thumbs/tiles.dat` (an append-only
    /// atlas: 8 bytes of key and 8x8 RGBA each) and its file.
    tiles_known: HashSet<u64>,
    tiles_dat: std::fs::File,
    saved: usize,
    snapshots: usize,
    pub n_samples: u64,
    n_loads: u64,
    n_loads_sourced: u64,
    resyncs: u64,
    first_frame: Option<u64>,
    last_frame: u64,
    dirty: bool,
    last_flush: std::time::Instant,
    busy: std::time::Duration,
    dropped: u64,
    texwatch_cost: (u64, u64),
}

impl Session {
    /// Start a session in `settings.dir`; `data_rom` is the ROM image
    /// uploads are looked up in (None: no sources).
    pub fn new(settings: Settings, data_rom: Option<Arc<[u8]>>) -> std::io::Result<Session> {
        let root = settings.dir.clone();
        let started = now_secs();
        let id = format!("{started:010x}-{:04x}", std::process::id() & 0xffff);
        let dir = root.join("sessions").join(&id);
        for d in [dir.clone(), root.join("thumbs"), root.join("frames")] {
            std::fs::create_dir_all(d)?;
        }
        write_atomic(
            &root.join("format.txt"),
            format!("reModel catalogue {FORMAT}\ngame {}\n", settings.game).as_bytes(),
        )?;
        let open = |n: &str| std::fs::OpenOptions::new().create(true).append(true).open(dir.join(n));
        let atlas = root.join("thumbs").join("tiles.dat");
        let tiles_known = std::fs::read(&atlas)
            .map(|b| {
                b.as_chunks::<TILE_RECORD>().0.iter().map(|c| u64::from_le_bytes(c[..8].try_into().unwrap())).collect()
            })
            .unwrap_or_default();
        let tiles_dat = std::fs::OpenOptions::new().create(true).append(true).open(&atlas)?;
        let mut s = Session {
            samples_log: open("samples.tsv")?,
            loads_log: open("loads.tsv")?,
            dir,
            root,
            id,
            settings,
            started,
            entries: HashMap::new(),
            aggs: HashMap::new(),
            edges: HashMap::new(),
            screens: Vec::new(),
            page_loaded: HashMap::new(),
            pending: Vec::new(),
            rom_index: data_rom.map(RomIndex::new),
            thumbs: HashSet::new(),
            tiles_known,
            tiles_dat,
            saved: 0,
            snapshots: 0,
            n_samples: 0,
            n_loads: 0,
            n_loads_sourced: 0,
            resyncs: 0,
            first_frame: None,
            last_frame: 0,
            dirty: true,
            last_flush: std::time::Instant::now(),
            busy: Default::default(),
            dropped: 0,
            texwatch_cost: (0, 0),
        };
        writeln!(s.samples_log, "frame\tscreen\ttex\ttile\tlayout\tobject\tnew\timage")?;
        writeln!(s.loads_log, "frame\toffset\tlen\thash\tdest\tsource\tscreen")?;
        s.flush()?;
        Ok(s)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Texture uploads the watch found (and frames it lost track at).
    pub fn loads(&mut self, loads: Vec<Load>, resyncs: Vec<u64>) {
        for r in resyncs {
            self.resyncs += 1;
            let _ = writeln!(self.samples_log, "#resync\t{r}");
        }
        for load in loads {
            self.n_loads += 1;
            let src = self.rom_index.as_ref().and_then(|ix| {
                let from_head = ix.find(&load.head);
                let from_mid = ix.find(&load.mid).map(|o| o.wrapping_sub(((load.len as usize / 2) & !63) as u32));
                from_head.or(from_mid)
            });
            self.n_loads_sourced += src.is_some() as u64;
            for p in load.offset / texwatch::PAGE as u32..(load.offset + load.len).div_ceil(texwatch::PAGE as u32) {
                let e = self.page_loaded.entry(p).or_insert((0, 0));
                *e = (load.frame, e.1 + 1);
            }
            self.pending.push(LoadRow { load, src });
        }
        self.dirty = true;
    }

    /// Texture ranges `[start, end)` and what drew from them, for a
    /// `loaded vs drawn` count: not here, in the index.
    fn thumb(&mut self, name: &str, png: impl FnOnce() -> Vec<u8>, overwrite: bool) {
        let path = self.root.join("thumbs").join(format!("{name}.png"));
        if overwrite || (!self.thumbs.contains(name) && !path.exists()) {
            let _ = std::fs::write(&path, png());
        }
        self.thumbs.insert(name.to_string());
    }

    /// Record one identified frame. `picture` draws it, called only when
    /// the frame is saved or crops of it are.
    pub fn ingest(&mut self, obs: &Observation, reg: &Registry, picture: impl FnOnce() -> Shot) {
        let started = std::time::Instant::now();
        let frame = obs.frame;
        self.first_frame.get_or_insert(frame);
        self.last_frame = frame;
        self.n_samples += 1;
        // The screen: the nearest earlier one by its most covering textures
        // and tiles, else a new one.
        let sig: HashMap<Ident, Ident> = obs.infos.iter().map(|i| (i.id, screen_sig(i))).collect();
        let mut top: Vec<(u32, Ident)> = obs
            .seen
            .iter()
            .filter(|s| matches!(s.id.0, Kind::Tex | Kind::Tile))
            .map(|s| (s.pixels, sig.get(&s.id).copied().unwrap_or(s.id)))
            .collect();
        top.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut top: Vec<Ident> = top.into_iter().map(|t| t.1).collect::<BTreeSet<_>>().into_iter().collect();
        top.truncate(SCREEN_TOP);
        let (best, like) = self
            .screens
            .iter()
            .enumerate()
            .map(|(i, s)| (i, jaccard(&s.top, &top)))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap_or((0, 0.0));
        let (screen, new_screen) = if like >= SCREEN_LIKE && !self.screens.is_empty() {
            (best, false)
        } else {
            self.screens.push(Screen { top, samples: 0, first: frame, last: frame, image: None });
            (self.screens.len() - 1, true)
        };
        {
            let s = &mut self.screens[screen];
            s.samples += 1;
            s.last = frame;
        }
        // Which entries are new, and which are seen bigger than ever.
        let info: HashMap<Ident, &Info> = obs.infos.iter().map(|i| (i.id, i)).collect();
        let mut new = 0;
        let mut new_ids: Vec<Ident> = Vec::new();
        let mut improved: Vec<Ident> = Vec::new();
        for s in &obs.seen {
            let Some(i) = info.get(&s.id) else { continue };
            let row = self.entries.entry(s.id).or_insert_with(|| {
                new += 1;
                new_ids.push(s.id);
                Row {
                    info: (*i).clone(),
                    loaded: None,
                    loads: 0,
                    count: 0,
                    with: BTreeSet::new(),
                    pals: BTreeSet::new(),
                }
            });
            row.count = row.count.max(s.count);
            row.with.extend(i.with.iter().copied());
            if s.id.0 == Kind::Tex {
                row.pals.extend(detail_pals(&i.detail).map(str::to_string));
            }
            if let Some(p) = i.place.filter(|_| s.id.0 == Kind::Tex) {
                let (mut at, mut n) = (0, 0);
                for page in texwatch::rect_pages(p[0], p[1], p[2], p[3], p[4]) {
                    if let Some(&(f, c)) = self.page_loaded.get(&page) {
                        at = at.max(f);
                        n = n.max(c);
                    }
                }
                if row.loaded.is_none() && at > 0 {
                    row.loaded = Some(at);
                }
                row.loads = row.loads.max(n);
            }
            let a = self.aggs.entry(s.id).or_insert_with(|| Agg { first: frame, ..Default::default() });
            a.samples += 1;
            a.pixels += s.pixels as u64;
            a.last = frame;
            a.wraps |= s.wraps;
            *a.screens.entry(screen as u32).or_insert(0) += 1;
            let bigger = s.pixels > a.best_px;
            if bigger && (a.best_px == 0 || s.pixels >= 2 * a.best_px && s.pixels >= 200) && s.id.0 != Kind::Tile {
                improved.push(s.id);
            }
            if bigger {
                (a.best_px, a.best_frame, a.best_bbox) = (s.pixels, frame, s.bbox);
            }
        }
        for &(a, b, w) in &obs.edges {
            *self.edges.entry((a, b)).or_insert(0) += w;
        }
        if self.edges.len() > 400_000 {
            self.edges.retain(|_, w| *w >= 4);
        }
        // Pictures: the texture and tile images once, the frame when it
        // shows something new, and crops of layouts and objects.
        for s in obs.seen.iter().filter(|s| matches!(s.id.0, Kind::Tex | Kind::Tile)) {
            let Some(e) = self.entries.get(&s.id) else { continue };
            if self.thumbs.contains(&e.info.name) || self.tiles_known.contains(&s.id.1) && s.id.0 == Kind::Tile {
                continue;
            }
            if let Some(entry) = e.info.reg_id.and_then(|r| reg.entry(r)) {
                let name = e.info.name.clone();
                let img: &Rgba = &entry.image;
                if s.id.0 == Kind::Tile {
                    let mut rec = s.id.1.to_le_bytes().to_vec();
                    rec.extend_from_slice(&img.data);
                    if rec.len() == TILE_RECORD {
                        let _ = self.tiles_dat.write_all(&rec);
                    }
                    self.tiles_known.insert(s.id.1);
                } else {
                    self.thumb(&name, || png::encode_rgba(img.width, img.height, &img.data), false);
                }
            }
        }
        let save = (new > 0 || new_screen || !improved.is_empty()) && self.saved < self.settings.frame_cap;
        let crops: Vec<Ident> = obs
            .seen
            .iter()
            .filter(|s| matches!(s.id.0, Kind::Layout | Kind::Object))
            .filter(|s| {
                let name = &info[&s.id].name;
                !self.thumbs.contains(name) && !self.root.join("thumbs").join(format!("{name}.png")).exists()
                    || improved.contains(&s.id)
            })
            .map(|s| s.id)
            .collect();
        let mut image_name = None;
        if save || !crops.is_empty() {
            let shot = picture();
            let rgb = shot.rgb;
            // A frame that shows a new layout keeps its tile data, so a
            // region can be cut from it later.
            let new_layout = obs.seen.iter().any(|s| s.id.0 == Kind::Layout && new_ids.contains(&s.id));
            if save && new_layout && self.snapshots < SNAPSHOT_CAP && !shot.tiles.is_empty() {
                let mut raw = (shot.tiles.len() as u32).to_le_bytes().to_vec();
                raw.extend_from_slice(&(shot.palette.len() as u32).to_le_bytes());
                raw.extend_from_slice(&shot.tiles);
                raw.extend_from_slice(&shot.palette);
                let p = self.root.join("frames").join(format!("{}-{frame:06}.tiles.z", self.id));
                if std::fs::write(p, png::zlib::compress(&raw)).is_ok() {
                    self.snapshots += 1;
                }
            }
            if save {
                let name = format!("{}-{frame:06}.png", self.id);
                let _ = std::fs::write(
                    self.root.join("frames").join(&name),
                    png::encode_rgb(video::WIDTH, video::HEIGHT, &rgb),
                );
                self.saved += 1;
                image_name = Some(name.clone());
                if new_screen || self.screens[screen].image.is_none() {
                    self.screens[screen].image = Some(name);
                }
            }
            for id in crops {
                let s = obs.seen.iter().find(|s| s.id == id).unwrap();
                let pad = 6u16;
                let (x0, y0) = (s.bbox[0].saturating_sub(pad) as usize, s.bbox[1].saturating_sub(pad) as usize);
                let (x1, y1) = ((s.bbox[2] + pad) as usize, (s.bbox[3] + pad) as usize);
                let (x1, y1) = (x1.min(video::WIDTH - 1), y1.min(video::HEIGHT - 1));
                if x1 < x0 || y1 < y0 {
                    continue;
                }
                let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
                let mut crop = Vec::with_capacity(w * h * 3);
                for y in y0..=y1 {
                    crop.extend_from_slice(&rgb[(y * video::WIDTH + x0) * 3..(y * video::WIDTH + x1 + 1) * 3]);
                }
                let name = info[&id].name.clone();
                self.thumb(&name, || png::encode_rgb(w, h, &crop), true);
            }
        }
        // The loads since the last sample belong to this screen.
        let loads = std::mem::take(&mut self.pending);
        self.write_loads(loads, Some(screen));
        let n = |k: Kind| obs.seen.iter().filter(|s| s.id.0 == k).count();
        let _ = writeln!(
            self.samples_log,
            "{frame}\t{screen}\t{}\t{}\t{}\t{}\t{new}\t{}",
            n(Kind::Tex),
            n(Kind::Tile),
            n(Kind::Layout),
            n(Kind::Object),
            image_name.unwrap_or_else(|| "-".into())
        );
        self.dirty = true;
        self.busy += started.elapsed();
        if self.last_flush.elapsed() > std::time::Duration::from_secs(30) {
            let _ = self.flush();
        }
    }

    fn write_loads(&mut self, loads: Vec<LoadRow>, screen: Option<usize>) {
        for r in loads {
            let l = &r.load;
            let _ = writeln!(
                self.loads_log,
                "{}\t{:x}\t{}\t{:016x}\t{}\t{}\t{}",
                l.frame,
                l.offset,
                l.len,
                l.hash,
                texwatch::dest_text(l.offset, l.len),
                r.src.map_or("-".into(), |o| format!("{o:x}")),
                screen.map_or("-".into(), |s| s.to_string())
            );
        }
    }

    /// A sample the worker had no time for.
    pub fn dropped(&mut self, n: u64) {
        self.dropped = n;
    }

    /// The watch's cost: frames it scanned and host nanoseconds.
    pub fn watch_cost(&mut self, scans: u64, ns: u64) {
        self.texwatch_cost = (scans, ns);
    }

    /// Write the aggregate files (atomically).
    pub fn flush(&mut self) -> std::io::Result<()> {
        self.last_flush = std::time::Instant::now();
        let mut idents: Vec<&Ident> = self.entries.keys().collect();
        idents.sort();
        let mut e = String::from("ident\tkind\tname\tw\th\tsurface\tcount\tloaded\tloads\tdetail\twith\n");
        for id in &idents {
            let r = &self.entries[id];
            let i = &r.info;
            let _ = writeln!(
                e,
                "{id}\t{}\t{}\t{}\t{}\t{:016x}\t{}\t{}\t{}\t{}\t{}",
                id.0.as_str(),
                i.name,
                i.w,
                i.h,
                i.surface,
                r.count,
                r.loaded.map_or("-".into(), |f| f.to_string()),
                r.loads,
                if id.0 == Kind::Tex { detail_with_pals(&i.detail, &r.pals) } else { i.detail.clone() },
                r.with.iter().map(|k| format!("{k:016x}")).collect::<Vec<_>>().join(",")
            );
        }
        write_atomic(&self.dir.join("entries.tsv"), e.as_bytes())?;
        let mut s =
            String::from("ident\tsamples\tpixels\tbest_px\tbest_frame\tx0\ty0\tx1\ty1\tfirst\tlast\tscreens\twrap\n");
        for id in &idents {
            let Some(a) = self.aggs.get(id) else { continue };
            let b = a.best_bbox;
            let sc: Vec<String> = a.screens.iter().map(|(k, n)| format!("{k}:{n}")).collect();
            let _ = writeln!(
                s,
                "{id}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                a.samples,
                a.pixels,
                a.best_px,
                a.best_frame,
                b[0],
                b[1],
                b[2],
                b[3],
                a.first,
                a.last,
                sc.join(","),
                if a.wraps != 0 { crate::idmap::wrap_names(a.wraps) } else { "-".into() }
            );
        }
        write_atomic(&self.dir.join("seen.tsv"), s.as_bytes())?;
        let mut pairs: Vec<(&(Ident, Ident), &u64)> = self.edges.iter().filter(|(_, &w)| w >= 4).collect();
        pairs.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let mut t = String::from("a\tb\tweight\n");
        for ((a, b), w) in pairs {
            let _ = writeln!(t, "{a}\t{b}\t{w}");
        }
        write_atomic(&self.dir.join("edges.tsv"), t.as_bytes())?;
        let mut sc = String::from("screen\tsamples\tfirst\tlast\timage\ttop\n");
        for (i, s) in self.screens.iter().enumerate() {
            let _ = writeln!(
                sc,
                "{i}\t{}\t{}\t{}\t{}\t{}",
                s.samples,
                s.first,
                s.last,
                s.image.as_deref().unwrap_or("-"),
                s.top.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(",")
            );
        }
        write_atomic(&self.dir.join("screens.tsv"), sc.as_bytes())?;
        let info = format!(
            "session {}\ngame {}\nlabel {}\nstarted {}\nupdated {}\nformat {FORMAT}\nevery {}\nsamples {}\nfirst_frame {}\nlast_frame {}\n\
             loads {}\nloads_sourced {}\nresyncs {}\ndropped {}\nworker_ms {}\nwatch_scans {}\nwatch_ms {}\nsaved_frames {}\n",
            self.id,
            self.settings.game,
            self.settings.label.replace('\n', " "),
            self.started,
            now_secs(),
            self.settings.every,
            self.n_samples,
            self.first_frame.unwrap_or(0),
            self.last_frame,
            self.n_loads,
            self.n_loads_sourced,
            self.resyncs,
            self.dropped,
            self.busy.as_millis(),
            self.texwatch_cost.0,
            self.texwatch_cost.1 / 1_000_000,
            self.saved
        );
        write_atomic(&self.dir.join("info.txt"), info.as_bytes())?;
        self.samples_log.flush()?;
        self.loads_log.flush()?;
        self.dirty = false;
        Ok(())
    }

    /// End the session: loads with no sample after them, the aggregates.
    pub fn finish(mut self) -> std::io::Result<String> {
        let loads = std::mem::take(&mut self.pending);
        self.write_loads(loads, None);
        self.flush()?;
        Ok(self.id.clone())
    }
}

// ---------------------------------------------------------------------
// The game thread's side.

enum Job {
    Sample(Box<Frame>),
}

/// What the game thread sends beside samples: texture loads and resync
/// frames, and at the end the watch's cost (frames scanned, nanoseconds).
enum Msg {
    Loads(Vec<Load>, Vec<u64>),
    Cost(u64, u64),
}

/// What an observing run did, for its closing line.
#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub session: String,
    pub dir: PathBuf,
    pub samples: u64,
    pub dropped: u64,
    pub entries: usize,
    pub loads: u64,
    /// Host milliseconds the game thread spent in `tick`.
    pub tick_ms: f64,
    pub watch_ms: f64,
    pub worker_ms: f64,
}

/// Observing a running machine.
pub struct Catalogue {
    tx: Option<mpsc::SyncSender<Job>>,
    loads_tx: mpsc::Sender<Msg>,
    handle: Option<std::thread::JoinHandle<Summary>>,
    dropped: Arc<std::sync::atomic::AtomicU64>,
    every: u64,
    block: bool,
    next_due: u64,
    last_sample: u64,
    last_tile_writes: u64,
    loaded_since: bool,
    tick_time: std::time::Duration,
    pub session: String,
}

impl Catalogue {
    /// Start observing `bus`: turns its texture watch on and starts the
    /// worker.
    pub fn start(settings: Settings, bus: &mut crate::Model2Bus) -> std::io::Result<Catalogue> {
        let rom: Arc<[u16]> = bus.texture_rom.clone();
        let data: Arc<[u8]> = bus.data_rom().into();
        let every = settings.every.max(1);
        let block = settings.block;
        let session = Session::new(settings.clone(), Some(data))?;
        let id = session.id().to_string();
        bus.tex_watch = Some(Box::default());
        let (tx, rx) = mpsc::sync_channel::<Job>(2);
        let (loads_tx, loads_rx) = mpsc::channel::<Msg>();
        let dropped = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let d2 = dropped.clone();
        let handle = std::thread::Builder::new()
            .name("catalogue".into())
            .spawn(move || worker(session, settings, rom, rx, loads_rx, d2))?;
        Ok(Catalogue {
            tx: Some(tx),
            loads_tx,
            handle: Some(handle),
            dropped,
            every,
            block,
            next_due: 0,
            last_sample: 0,
            last_tile_writes: bus.tile_ram_writes(),
            loaded_since: false,
            tick_time: Default::default(),
            session: id,
        })
    }

    /// Once per emulated frame (when `bus.frames` changed): pass on what
    /// the texture watch found and, when one is due, a sample.
    pub fn tick(&mut self, bus: &mut crate::Model2Bus) {
        let t = std::time::Instant::now();
        let mut waited = std::time::Duration::ZERO;
        let frame = bus.frames;
        if let Some(w) = bus.tex_watch.as_deref_mut() {
            let (loads, resyncs) = w.take();
            if !resyncs.is_empty() || frame < self.last_sample {
                // A state was loaded (or the game rewound): sample at once.
                self.next_due = 0;
            }
            if !loads.is_empty() {
                self.loaded_since = true;
            }
            if !loads.is_empty() || !resyncs.is_empty() {
                let _ = self.loads_tx.send(Msg::Loads(loads, resyncs));
            }
        }
        // On change: the tile layer rewritten in bulk (a screen change; a
        // counter's digits are a few writes), or textures loaded, at most
        // every 8 frames; else every `every` frames.
        let writes = bus.tile_ram_writes();
        let bulk = writes.wrapping_sub(self.last_tile_writes) > 1500;
        let since = frame.saturating_sub(self.last_sample);
        let due = frame >= self.next_due || ((bulk || self.loaded_since) && since >= 8);
        // Loads come in bursts: wait for the burst to end before looking.
        let settled = !self.loaded_since || since >= 8 || frame >= self.next_due;
        if due && settled && frame > 0 {
            self.last_tile_writes = writes;
            self.loaded_since = false;
            let f = Frame::grab(bus);
            if let Some(tx) = &self.tx {
                let job = Job::Sample(Box::new(f));
                if self.block {
                    // Waiting for the worker is the scripted run's choice,
                    // not the observer's cost.
                    let w = std::time::Instant::now();
                    let _ = tx.send(job);
                    waited = w.elapsed();
                } else if tx.try_send(job).is_err() {
                    self.dropped.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
            self.last_sample = frame;
            self.next_due = frame + self.every;
        }
        self.tick_time += t.elapsed().saturating_sub(waited);
    }

    /// Stop observing: finish the worker, write the session, turn the
    /// watch off.
    pub fn finish(mut self, bus: &mut crate::Model2Bus) -> Summary {
        let (scans, ns) = bus.tex_watch.as_ref().map_or((0, 0), |w| (w.scans, w.scan_ns));
        if let Some(w) = bus.tex_watch.as_deref_mut() {
            let (loads, resyncs) = w.take();
            let _ = self.loads_tx.send(Msg::Loads(loads, resyncs));
        }
        let _ = self.loads_tx.send(Msg::Cost(scans, ns));
        bus.tex_watch = None;
        self.tx = None;
        let mut s = self.handle.take().and_then(|h| h.join().ok()).unwrap_or_default();
        s.tick_ms = self.tick_time.as_secs_f64() * 1e3;
        s.watch_ms = ns as f64 / 1e6;
        s
    }
}

fn worker(
    mut session: Session,
    settings: Settings,
    rom: Arc<[u16]>,
    rx: mpsc::Receiver<Job>,
    loads_rx: mpsc::Receiver<Msg>,
    dropped: Arc<std::sync::atomic::AtomicU64>,
) -> Summary {
    let mut reg = Registry::default();
    let (hud_a, hud_b) = (settings.windows.0.clone(), settings.windows.1.clone());
    let mut watch = (0u64, 0u64);
    let mut pump = |session: &mut Session| {
        while let Ok(m) = loads_rx.try_recv() {
            match m {
                Msg::Loads(loads, resyncs) => session.loads(loads, resyncs),
                Msg::Cost(scans, ns) => watch = (scans, ns),
            }
        }
    };
    while let Ok(Job::Sample(frame)) = rx.recv() {
        pump(&mut session);
        let side = frame.light_frame().or_else(|| frame.scan_frame());
        let observed = observe(&frame, &rom, &mut reg, (&hud_a, &hud_b), side.as_ref());
        let obs = observed.obs.clone();
        let mut observed = Some(observed);
        session.ingest(&obs, &reg, || observed.take().map(Observed::picture).unwrap_or_default());
    }
    pump(&mut session);
    session.dropped(dropped.load(std::sync::atomic::Ordering::Relaxed));
    session.watch_cost(watch.0, watch.1);
    let summary = Summary {
        session: session.id().to_string(),
        dir: settings.dir.clone(),
        samples: session.n_samples,
        dropped: dropped.load(std::sync::atomic::Ordering::Relaxed),
        entries: session.entries.len(),
        loads: session.n_loads,
        worker_ms: session.busy.as_secs_f64() * 1e3,
        ..Default::default()
    };
    let _ = session.finish();
    summary
}

impl Drop for Catalogue {
    /// A session dropped without `finish` (the player leaving a game):
    /// let the worker write what it has.
    fn drop(&mut self) {
        self.tx = None;
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
