//! What draws each pixel of a frame: texture and tile identification shared
//! by `remodel texmap` and the catalogue. Render-side only; it changes
//! nothing of the emulation.
//!
//! IDs are numbers, one table per [`Registry`]: 0 nothing (backdrop), 1 an
//! untextured polygon, 2 and up the textures and tile patterns in order of
//! first sight. An ID image stores an ID in a pixel's RGB.

use crate::enhance::{Rgba, TexInfo};
use crate::raster::{self, Placed, Poly, Sources, TexKeys};
use crate::video::{self, TilePixels, View};
use std::collections::{BTreeSet, HashMap};

/// "Nothing": the backdrop, or a pixel no layer draws.
pub const NONE_ID: u32 = 0;
/// An untextured polygon (flat colour).
pub const FLAT_ID: u32 = 1;
/// The first ID of a texture or tile.
pub const FIRST_ID: u32 = 2;

/// An ID as the RGB of an ID image pixel.
pub fn encode_id(id: u32) -> [u8; 3] {
    [id as u8, (id >> 8) as u8, (id >> 16) as u8]
}

/// [`encode_id`]'s inverse (for tools and tests that read ID images).
#[allow(dead_code)]
pub fn decode_id(rgb: [u8; 3]) -> u32 {
    rgb[0] as u32 | (rgb[1] as u32) << 8 | (rgb[2] as u32) << 16
}

/// What an entry is: the key texture packs name it by.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Key {
    /// A 3D texture (`TexInfo::key`).
    Tex(u64),
    /// A 2D-layer 8x8 tile pattern (`TileInfo::key`).
    Tile(u64),
}

/// One texture or tile pattern of the run.
#[derive(Clone, Debug)]
pub struct Entry {
    pub key: Key,
    /// The dump / pack file stem.
    pub name: String,
    pub w: u32,
    pub h: u32,
    /// The surface key (textures; 0 for tiles).
    pub surface: u64,
    /// Where it came from, for the inventory: `bank=.. x=.. y=.. luma=..
    /// colour=..` (textures), `tile=.. bank=..` (tiles).
    pub detail: String,
    /// The image as the game draws it at full luma.
    pub image: Rgba,
}

impl Entry {
    pub fn kind(&self) -> &'static str {
        match self.key {
            Key::Tex(_) => "tex",
            Key::Tile(_) => "tile",
        }
    }
}

/// The run's IDs.
#[derive(Default)]
pub struct Registry {
    by_key: HashMap<Key, u32>,
    pub entries: Vec<Entry>,
}

impl Registry {
    /// The ID of `key`, made (from `make`) at first sight.
    pub fn intern(&mut self, key: Key, make: impl FnOnce() -> Entry) -> u32 {
        if let Some(&id) = self.by_key.get(&key) {
            return id;
        }
        self.entries.push(make());
        let id = FIRST_ID + self.entries.len() as u32 - 1;
        self.by_key.insert(key, id);
        id
    }

    pub fn entry(&self, id: u32) -> Option<&Entry> {
        id.checked_sub(FIRST_ID).and_then(|i| self.entries.get(i as usize))
    }
}

/// What a frame drew of one ID, over the pixels that show it.
#[derive(Clone, Debug, Default)]
pub struct Draw {
    /// Polygons with at least one visible pixel.
    pub polys: usize,
    /// Wrapping used, a bit per way: 1 mirror in u, 2 mirror in v.
    pub wraps: u8,
    /// Extent of the unwrapped texel coordinates over visible pixels.
    pub raw_min: [f32; 2],
    pub raw_max: [f32; 2],
}

/// A frame's identification of what drew each output pixel.
pub struct IdFrame {
    pub width: usize,
    pub height: usize,
    pub ids: Vec<u32>,
    /// What may be adjacent in a group: 3D pixels of one object block (and
    /// window), or tiles of one layer. Pixels of different classes never
    /// link two textures.
    pub class: Vec<u32>,
    /// Texel position (u, v) of the pixel in its texture, wrapped into it
    /// (a tile's 8x8 pattern likewise); `[0, 0]` where there is none.
    pub uv: Vec<[f32; 2]>,
    pub draws: HashMap<u32, Draw>,
}

/// Wrap a texel coordinate into `0..size` as the renderer does.
fn wrap_uv(c: f32, size: u32, mirror: bool) -> f32 {
    let s = size.max(1) as f32;
    if mirror {
        let m = c.rem_euclid(2.0 * s);
        if m >= s {
            2.0 * s - m
        } else {
            m
        }
    } else {
        c.rem_euclid(s)
    }
}

/// A colour ramp (what each texel value 0 to 15 of a texture is drawn in at
/// full luma, `raster::texture_look`) as 96 hex digits: the catalogue's
/// record of one colouring (`ramp=` in an entry's detail).
pub fn ramp_text(ramp: &[[u8; 3]; 16]) -> String {
    ramp.iter().map(|c| format!("{:02x}{:02x}{:02x}", c[0], c[1], c[2])).collect()
}

/// [`ramp_text`]'s inverse; None unless exactly 96 hex digits.
pub fn parse_ramp(s: &str) -> Option<[[u8; 3]; 16]> {
    if s.len() != 96 || !s.is_ascii() {
        return None;
    }
    let mut out = [[0u8; 3]; 16];
    for (i, c) in out.iter_mut().enumerate() {
        for (ch, v) in c.iter_mut().enumerate() {
            let at = i * 6 + ch * 2;
            *v = u8::from_str_radix(&s[at..at + 2], 16).ok()?;
        }
    }
    Some(out)
}

/// The ramp in an entry's detail text (None for tiles and older entries).
pub fn ramp_of_detail(detail: &str) -> Option<[[u8; 3]; 16]> {
    detail.split(' ').find_map(|t| t.strip_prefix("ramp=")).and_then(parse_ramp)
}

/// Name an entry's 3D texture.
fn tex_entry(info: &TexInfo, src: &Sources) -> Entry {
    Entry {
        key: Key::Tex(info.key),
        name: info.name(),
        w: info.width,
        h: info.height,
        surface: info.surface,
        detail: format!(
            "bank={} x={} y={} luma={} colour={:04x}{} ramp={}",
            info.bank,
            info.x0,
            info.y0,
            info.luma_table,
            info.colour,
            if info.translucent { " translucent" } else { "" },
            ramp_text(&raster::texture_look(src, info))
        ),
        image: raster::texture_image(src, info),
    }
}

const NO_TILE: u32 = u32::MAX;

/// A placed polygon's ID, wrapping bits and texture (w, h, mirror u, mirror v).
type PolyId = (u32, u8, Option<(u32, u32, bool, bool)>);

/// Identify what draws each output pixel of a frame: the front-most layer
/// as `video::compose` stacks them (high-priority tiles over the 3D layer
/// over the low-priority tiles). `polys` and `placed` are the frame's
/// (`raster::place`), `vis` and `uv` `raster::visible_uv`'s.
#[allow(clippy::too_many_arguments)]
pub fn identify(
    polys: &[Poly],
    placed: &[Placed],
    vis: &raster::Visible,
    uv: &[[f32; 2]],
    tiles: Option<&TilePixels>,
    view: &View,
    src: &Sources,
    reg: &mut Registry,
) -> IdFrame {
    let (w, h) = (view.width, view.height);
    let mut keys = TexKeys::default();
    // Per placed polygon: its ID, how it wraps.
    let mut poly_id: Vec<Option<PolyId>> = vec![None; placed.len()];
    let mut tile_id: Vec<Option<u32>> = vec![None; tiles.map_or(0, |t| t.infos.len())];
    let mut out = IdFrame {
        width: w,
        height: h,
        ids: vec![NONE_ID; w * h],
        class: vec![0; w * h],
        uv: vec![[0.0; 2]; w * h],
        draws: HashMap::new(),
    };
    let mut seen_poly: BTreeSet<(u32, u32)> = BTreeSet::new();
    for y in 0..h {
        for x in 0..w {
            let o = y * w + x;
            let (gx, gy) = (((x as f32 + 0.5) / view.sx - view.pad).floor(), ((y as f32 + 0.5) / view.sy).floor());
            let game = (gx >= 0.0 && gy >= 0.0 && (gx as usize) < video::WIDTH && (gy as usize) < video::HEIGHT)
                .then(|| gy as usize * video::WIDTH + gx as usize);
            let tile = |hit: video::TileHit,
                        high: bool,
                        out: &mut IdFrame,
                        tile_id: &mut Vec<Option<u32>>,
                        reg: &mut Registry| {
                let t = tiles?;
                if hit.info == NO_TILE {
                    return None;
                }
                let id = *tile_id[hit.info as usize].get_or_insert_with(|| {
                    let info = &t.infos[hit.info as usize];
                    reg.intern(Key::Tile(info.key), || Entry {
                        key: Key::Tile(info.key),
                        name: info.name(),
                        w: 8,
                        h: 8,
                        surface: 0,
                        detail: format!("tile={} bank={}", info.tile, info.bank),
                        image: info.image(),
                    })
                });
                out.ids[o] = id;
                out.class[o] = 1 << 31 | high as u32;
                out.uv[o] = [hit.col as f32 + 0.5, hit.row as f32 + 0.5];
                out.draws.entry(id).or_default();
                Some(())
            };
            if let Some(g) = game {
                if let Some(t) = tiles {
                    if tile(t.high[g], true, &mut out, &mut tile_id, reg).is_some() {
                        continue;
                    }
                }
            }
            let k = vis.owner[o];
            if k != u32::MAX {
                let k = k as usize;
                let p = &polys[placed[k].poly];
                let (id, mirror, tex) = *poly_id[k].get_or_insert_with(|| {
                    match p.texture.as_ref().and_then(|t| raster::tex_info(p, t, src, &mut keys).map(|i| (t, i))) {
                        Some((t, info)) => {
                            let id = reg.intern(Key::Tex(info.key), || tex_entry(&info, src));
                            (id, t.mirror_x as u8 | (t.mirror_y as u8) << 1, Some((t.w, t.h, t.mirror_x, t.mirror_y)))
                        }
                        None => (FLAT_ID, 0, None),
                    }
                });
                out.ids[o] = id;
                out.class[o] = (p.window as u32) << 20 | (p.block & 0xf_ffff);
                let d = out.draws.entry(id).or_insert_with(|| Draw {
                    raw_min: [f32::MAX; 2],
                    raw_max: [f32::MIN; 2],
                    ..Draw::default()
                });
                d.wraps |= 1 << mirror;
                if seen_poly.insert((id, k as u32)) {
                    d.polys += 1;
                }
                if let Some((tw, th, mx, my)) = tex {
                    let [u, v] = uv[o];
                    d.raw_min = [d.raw_min[0].min(u), d.raw_min[1].min(v)];
                    d.raw_max = [d.raw_max[0].max(u), d.raw_max[1].max(v)];
                    out.uv[o] = [wrap_uv(u, tw, mx), wrap_uv(v, th, my)];
                }
                continue;
            }
            if let Some(g) = game {
                if let Some(t) = tiles {
                    tile(t.low[g], false, &mut out, &mut tile_id, reg);
                }
            }
        }
    }
    out
}

/// What a frame shows of one texture or tile.
#[derive(Clone, Debug, Default)]
pub struct Stat {
    pub id: u32,
    pub pixels: usize,
    /// Output pixels: x0, y0, x1, y1 (inclusive).
    pub bbox: [usize; 4],
    pub polys: usize,
    pub wraps: u8,
    /// Texels (of w * h; 0 where the texture is too big to track) that show.
    pub used: usize,
    /// Texel bounding box of the visible part: u0, v0, u1, v1 (inclusive).
    pub uv_box: [u32; 4],
    /// Unwrapped extent over the texture's size: above 1, it repeats.
    pub span: [f32; 2],
    /// Which texels show (w * h; empty if untracked).
    pub seen: Vec<bool>,
}

/// Textures bigger than this many texels are not tracked texel by texel.
const TRACK_MAX: usize = 1 << 18;

/// Per-ID statistics of a frame, in ID order (the IDs that show).
pub fn stats(f: &IdFrame, reg: &Registry) -> Vec<Stat> {
    let mut by: HashMap<u32, Stat> = HashMap::new();
    for y in 0..f.height {
        for x in 0..f.width {
            let o = y * f.width + x;
            let id = f.ids[o];
            if id == NONE_ID {
                continue;
            }
            let s = by.entry(id).or_insert_with(|| {
                let tracked = reg.entry(id).map_or(0, |e| e.w as usize * e.h as usize);
                Stat {
                    id,
                    bbox: [usize::MAX, usize::MAX, 0, 0],
                    uv_box: [u32::MAX, u32::MAX, 0, 0],
                    seen: if tracked > 0 && tracked <= TRACK_MAX { vec![false; tracked] } else { Vec::new() },
                    ..Stat::default()
                }
            });
            s.pixels += 1;
            s.bbox = [s.bbox[0].min(x), s.bbox[1].min(y), s.bbox[2].max(x), s.bbox[3].max(y)];
            if let Some(e) = reg.entry(id) {
                let [u, v] = f.uv[o];
                let (tu, tv) = ((u as u32).min(e.w - 1), (v as u32).min(e.h - 1));
                s.uv_box = [s.uv_box[0].min(tu), s.uv_box[1].min(tv), s.uv_box[2].max(tu), s.uv_box[3].max(tv)];
                if !s.seen.is_empty() {
                    let i = (tv * e.w + tu) as usize;
                    if !s.seen[i] {
                        s.seen[i] = true;
                        s.used += 1;
                    }
                }
            }
        }
    }
    let mut out: Vec<Stat> = by.into_values().collect();
    for s in &mut out {
        if let Some(d) = f.draws.get(&s.id) {
            s.polys = d.polys;
            s.wraps = d.wraps;
            if let Some(e) = reg.entry(s.id).filter(|e| e.kind() == "tex" && d.raw_min[0] <= d.raw_max[0]) {
                s.span = [(d.raw_max[0] - d.raw_min[0]) / e.w as f32, (d.raw_max[1] - d.raw_min[1]) / e.h as f32];
            }
        }
    }
    out.sort_by_key(|s| s.id);
    out
}

/// What a texture shows over the run: frames and pixels summed, the
/// frame it is biggest in, the union of its visible texels and the widest
/// repeat.
#[derive(Clone, Debug, Default)]
pub struct Total {
    pub frames: usize,
    pub pixels: usize,
    pub polys: usize,
    pub best: (u64, usize),
    pub wraps: u8,
    pub span: [f32; 2],
    pub seen: Vec<bool>,
    pub uv_box: [u32; 4],
}

impl Total {
    pub fn add(&mut self, frame: u64, s: &Stat) {
        if self.frames == 0 {
            self.uv_box = [u32::MAX, u32::MAX, 0, 0];
        }
        self.frames += 1;
        self.pixels += s.pixels;
        self.polys += s.polys;
        if s.pixels > self.best.1 {
            self.best = (frame, s.pixels);
        }
        self.wraps |= s.wraps;
        self.span = [self.span[0].max(s.span[0]), self.span[1].max(s.span[1])];
        if self.seen.len() < s.seen.len() {
            self.seen.resize(s.seen.len(), false);
        }
        for (a, b) in self.seen.iter_mut().zip(&s.seen) {
            *a |= *b;
        }
        self.uv_box = [
            self.uv_box[0].min(s.uv_box[0]),
            self.uv_box[1].min(s.uv_box[1]),
            self.uv_box[2].max(s.uv_box[2]),
            self.uv_box[3].max(s.uv_box[3]),
        ];
    }

    pub fn used(&self) -> usize {
        self.seen.iter().filter(|&&b| b).count()
    }
}

/// The wrapping modes of a `wraps` mask, as the pack index writes them
/// (`rr`, `mr`, `rm`, `mm`: u then v, repeat or mirror), comma separated.
pub fn wrap_names(mask: u8) -> String {
    let names = ["rr", "mr", "rm", "mm"];
    (0..4).filter(|m| mask & (1 << m) != 0).map(|m| names[m]).collect::<Vec<_>>().join(",")
}

/// Pixel pairs of different IDs that touch (4-neighbours) in one object
/// class: the evidence that two textures are one object's parts.
#[derive(Default)]
pub struct Edges(pub HashMap<(u32, u32), u64>);

impl Edges {
    pub fn add_frame(&mut self, f: &IdFrame) {
        let (w, h) = (f.width, f.height);
        let mut pair = |a: usize, b: usize| {
            let (ia, ib) = (f.ids[a], f.ids[b]);
            if ia >= FIRST_ID && ib >= FIRST_ID && ia != ib && f.class[a] == f.class[b] {
                *self.0.entry((ia.min(ib), ia.max(ib))).or_insert(0) += 1;
            }
        };
        for y in 0..h {
            for x in 0..w {
                let o = y * w + x;
                if x + 1 < w {
                    pair(o, o + 1);
                }
                if y + 1 < h {
                    pair(o, o + w);
                }
            }
        }
    }
}
