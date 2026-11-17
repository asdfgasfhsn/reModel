//! `remodel texmap CAPTURE OUT`: which texture is where on screen.
//!
//! For each captured frame, draw it with the software renderer and write
//! an ID image (every pixel -> the texture or tile pattern that drew it),
//! a per-texture inventory (coverage, bounding box, polygons, the part of
//! the texture that shows), a contact sheet and groups of textures that
//! belong together; then one static HTML viewer over the lot
//! (`texmap_viewer`). A render-side tool: it reads captures and changes
//! nothing of the emulation (docs/uplift-studio.md, docs/texture-packs.md).
//!
//! IDs are numbers, one table for the whole run (`ids.tsv`), so a texture
//! has the same ID in every frame: 0 nothing (backdrop), 1 an untextured
//! polygon, 2 and up the textures and tile patterns in order of first
//! sight. The image stores an ID in a pixel's RGB (`r | g << 8 | b << 16`).
//! Textures are named as texture dumps and packs name them
//! (`tex_<key>_<w>x<h>`, `tile_<key>`), so a map entry is a pack file.

use model2::capture::{self, Frame};
use model2::enhance::{Rgba, TexInfo};
use model2::raster::{self, Placed, Poly, Sources, TexKeys};
use model2::video::{self, TilePixels, View};
use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::Path;

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

/// Name an entry's 3D texture.
fn tex_entry(info: &TexInfo, src: &Sources) -> Entry {
    Entry {
        key: Key::Tex(info.key),
        name: info.name(),
        w: info.width,
        h: info.height,
        surface: info.surface,
        detail: format!(
            "bank={} x={} y={} luma={} colour={:04x}{}",
            info.bank,
            info.x0,
            info.y0,
            info.luma_table,
            info.colour,
            if info.translucent { " translucent" } else { "" }
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

/// Groups of IDs that belong together: greedy agglomeration over the
/// adjacency edges, strongest first. Two groups merge when the edge
/// between them has at least `min_weight` touching pixel pairs and the
/// merged group has at most `cap` members, so a whole course (one object
/// block of dozens of textures that all touch) falls apart at its weakest
/// links instead of becoming one group. Every ID in `pixels` is in exactly
/// one group (most are alone). Groups are in order of total coverage,
/// members by coverage.
pub fn group(edges: &Edges, pixels: &HashMap<u32, usize>, min_weight: u64, cap: usize) -> Vec<Vec<u32>> {
    let mut ids: Vec<u32> = pixels.keys().copied().collect();
    ids.sort_unstable();
    let index: HashMap<u32, usize> = ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    let mut parent: Vec<usize> = (0..ids.len()).collect();
    let mut size = vec![1usize; ids.len()];
    fn find(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    let mut sorted: Vec<_> = edges.0.iter().filter(|(_, &w)| w >= min_weight).collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (&(a, b), _) in sorted {
        let (Some(&ia), Some(&ib)) = (index.get(&a), index.get(&b)) else { continue };
        let (ra, rb) = (find(&mut parent, ia), find(&mut parent, ib));
        if ra != rb && size[ra] + size[rb] <= cap {
            parent[rb] = ra;
            size[ra] += size[rb];
        }
    }
    let mut groups: HashMap<usize, Vec<u32>> = HashMap::new();
    for (i, &id) in ids.iter().enumerate() {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(id);
    }
    let mut out: Vec<Vec<u32>> = groups.into_values().collect();
    for g in &mut out {
        g.sort_by(|a, b| pixels[b].cmp(&pixels[a]).then(a.cmp(b)));
    }
    let total = |g: &Vec<u32>| g.iter().map(|id| pixels[id]).sum::<usize>();
    out.sort_by(|a, b| total(b).cmp(&total(a)).then(a[0].cmp(&b[0])));
    out
}

// 3x5 digits, one row per byte (bit 2 the left pixel).
const DIGITS: [[u8; 5]; 10] = [
    [7, 5, 5, 5, 7],
    [2, 6, 2, 2, 7],
    [7, 1, 7, 4, 7],
    [7, 1, 7, 1, 7],
    [5, 5, 7, 1, 1],
    [7, 4, 7, 1, 7],
    [7, 4, 7, 5, 7],
    [7, 1, 1, 1, 1],
    [7, 5, 7, 5, 7],
    [7, 5, 7, 1, 7],
];

/// Draw `n` in digits `scale` times their 3x5 size at (x, y), white on
/// `bg`.
fn draw_number(img: &mut [u8], width: usize, x: usize, y: usize, n: u32, scale: usize, bg: [u8; 3]) {
    let text = n.to_string();
    let (tw, th) = (text.len() * 4 * scale + scale, 7 * scale);
    let height = img.len() / (width * 3);
    for py in y..(y + th).min(height) {
        for px in x..(x + tw).min(width) {
            img[(py * width + px) * 3..][..3].copy_from_slice(&bg);
        }
    }
    for (i, c) in text.bytes().enumerate() {
        for (row, bits) in DIGITS[(c - b'0') as usize].iter().enumerate() {
            for col in 0..3 {
                if bits & (4 >> col) != 0 {
                    for (dy, dx) in (0..scale).flat_map(|a| (0..scale).map(move |b| (a, b))) {
                        let (px, py) = (x + (i * 4 + col + 1) * scale + dx, y + (row + 1) * scale + dy);
                        if px < width && py < height {
                            img[(py * width + px) * 3..][..3].copy_from_slice(&[255, 255, 255]);
                        }
                    }
                }
            }
        }
    }
}

/// Cell size of the contact sheet's thumbnails.
const CELL: usize = 96;
const CELL_PAD: usize = 8;

/// A contact sheet: the frame on the left, the textures it shows to its
/// right, biggest coverage first, each with its ID.
pub fn contact_sheet(picture: &[u8], w: usize, h: usize, shown: &[&Stat], reg: &Registry) -> (usize, usize, Vec<u8>) {
    let cell = CELL + CELL_PAD;
    let rows = (h / cell).max(1);
    let textured: Vec<&&Stat> = shown.iter().filter(|s| reg.entry(s.id).is_some()).collect();
    let cols = textured.len().div_ceil(rows).clamp(1, 8);
    let sw = w + 8 + cols * cell;
    let sh = h.max(cell);
    let mut img = vec![24u8; sw * sh * 3];
    for y in 0..h {
        img[y * sw * 3..][..w * 3].copy_from_slice(&picture[y * w * 3..][..w * 3]);
    }
    for (n, s) in textured.iter().take(rows * cols).enumerate() {
        let e = reg.entry(s.id).unwrap();
        let (cx, cy) = (w + 8 + (n / rows) * cell + CELL_PAD / 2, (n % rows) * cell + CELL_PAD / 2);
        // Integer magnification up to the cell, nearest neighbour; larger
        // images shrink by sampling.
        let (iw, ih) = (e.w as usize, e.h as usize);
        let (tw, th, _) = fit(iw, ih, CELL);
        for ty in 0..th {
            for tx in 0..tw {
                let (sx, sy) = (tx * iw / tw, ty * ih / th);
                let p = &e.image.data[(sy * iw + sx) * 4..][..4];
                let checker = if (tx / 4 + ty / 4) % 2 == 0 { 70u32 } else { 100 };
                let a = p[3] as u32;
                let px = ((cy + ty) * sw + cx + tx) * 3;
                for c in 0..3 {
                    img[px + c] = ((p[c] as u32 * a + checker * (255 - a)) / 255) as u8;
                }
            }
        }
        let bg = if e.kind() == "tile" { [20, 90, 140] } else { [20, 20, 20] };
        draw_number(&mut img, sw, cx, cy, s.id, 2, bg);
    }
    (sw, sh, img)
}

/// The size an `iw` x `ih` image is drawn at inside a `cell` square: whole
/// multiples when it is smaller, scaled down when larger. The third value
/// is the scale (0 when shrunk).
fn fit(iw: usize, ih: usize, cell: usize) -> (usize, usize, usize) {
    let m = iw.max(ih).max(1);
    if m <= cell {
        let k = cell / m;
        (iw * k, ih * k, k)
    } else {
        ((iw * cell / m).max(1), (ih * cell / m).max(1), 0)
    }
}

/// One frame's output: the PNGs and its stats.
pub struct FrameOut {
    pub frame: u64,
    pub width: usize,
    pub height: usize,
    pub stats: Vec<Stat>,
}

/// Command options.
pub struct Options<'a> {
    pub view: View,
    pub windows: (&'a [usize], &'a [usize]),
    pub light: Option<&'a model2::light::Options>,
    /// Frames shown in the viewer (spread evenly over the run).
    pub viewer_frames: usize,
    /// Largest group.
    pub group_cap: usize,
    /// Keep every n-th captured frame.
    pub every: usize,
}

/// A frame's pictures: the 3D layer and 2D layers composed, and the IDs.
fn render_frame(f: &Frame, rom: &[u16], o: &Options, reg: &mut Registry) -> (Vec<u8>, IdFrame, TilePixels) {
    let view = o.view.with_hud_windows(o.windows.0, o.windows.1);
    let src = f.sources(rom);
    let mut polys = raster::build_shaded(&f.stream, &src, o.light);
    video::HudLayout::new(&f.tiles, &f.palette, &view).anchor(&mut polys, &view);
    let placed = raster::place(&mut polys, &view);
    let (vis, uv) = raster::visible_uv(&polys, &placed, &src, &view);
    let tiles = video::tile_pixels(&f.tiles, &f.palette);
    let ids = identify(&polys, &placed, &vis, &uv, Some(&tiles), &view, &src, reg);
    let picture = if polys.is_empty() {
        video::compose(&f.tiles, &f.palette, None, &view)
    } else {
        let layer = raster::draw_view_depth(&mut polys, &src, &view, false);
        video::compose(&f.tiles, &f.palette, Some((&layer.0, &layer.1)), &view)
    };
    (picture, ids, tiles)
}

fn id_png(f: &IdFrame) -> Vec<u8> {
    let rgb: Vec<u8> = f.ids.iter().flat_map(|&id| encode_id(id)).collect();
    png::encode_rgb(f.width, f.height, &rgb)
}

/// The texel position image: red and green the position in the texture
/// (0-255 across it), blue 255 where there is one.
fn uv_png(f: &IdFrame, reg: &Registry) -> Vec<u8> {
    let mut rgb = vec![0u8; f.width * f.height * 3];
    for (o, &id) in f.ids.iter().enumerate() {
        if let Some(e) = reg.entry(id) {
            let [u, v] = f.uv[o];
            rgb[o * 3] = (u * 256.0 / e.w as f32).min(255.0) as u8;
            rgb[o * 3 + 1] = (v * 256.0 / e.h as f32).min(255.0) as u8;
            rgb[o * 3 + 2] = 255;
        }
    }
    png::encode_rgb(f.width, f.height, &rgb)
}

fn pct(n: usize, of: usize) -> f64 {
    100.0 * n as f64 / of.max(1) as f64
}

fn uv_text(s: &Stat, reg: &Registry) -> String {
    match reg.entry(s.id) {
        Some(_) if s.uv_box[0] != u32::MAX => {
            format!("{}\t{}\t{}\t{}", s.uv_box[0], s.uv_box[1], s.uv_box[2], s.uv_box[3])
        }
        _ => "-\t-\t-\t-".into(),
    }
}

/// `frame.tsv`: one row per ID the frame shows, biggest first.
fn frame_tsv(fo: &FrameOut, reg: &Registry, group_of: &HashMap<u32, usize>) -> String {
    let total = fo.width * fo.height;
    let mut s = String::from(
        "id\tname\tkind\tpixels\tpct\tx0\ty0\tx1\ty1\tpolys\tuv_used_pct\tuv_u0\tuv_v0\tuv_u1\tuv_v1\twrap\tspan_u\tspan_v\tgroup\n",
    );
    let mut rows: Vec<&Stat> = fo.stats.iter().collect();
    rows.sort_by(|a, b| b.pixels.cmp(&a.pixels).then(a.id.cmp(&b.id)));
    for st in rows {
        let (name, kind, texels) = match st.id {
            FLAT_ID => ("flat".to_string(), "flat", 0),
            _ => reg.entry(st.id).map_or(("?".into(), "?", 0), |e| (e.name.clone(), e.kind(), (e.w * e.h) as usize)),
        };
        let used = if texels > 0 && !st.seen.is_empty() { format!("{:.1}", pct(st.used, texels)) } else { "-".into() };
        let _ = writeln!(
            s,
            "{}\t{}\t{}\t{}\t{:.3}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.2}\t{:.2}\t{}",
            st.id,
            name,
            kind,
            st.pixels,
            pct(st.pixels, total),
            st.bbox[0],
            st.bbox[1],
            st.bbox[2],
            st.bbox[3],
            st.polys,
            used,
            uv_text(st, reg),
            if st.wraps != 0 { wrap_names(st.wraps) } else { "-".into() },
            st.span[0],
            st.span[1],
            group_of.get(&st.id).map_or("-".to_string(), |g| g.to_string()),
        );
    }
    s
}

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 32 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// The run's results, from which the run-level files are written.
pub struct Run {
    pub reg: Registry,
    pub frames: Vec<FrameOut>,
    pub totals: HashMap<u32, Total>,
    pub groups: Vec<Vec<u32>>,
    pub group_of: HashMap<u32, usize>,
}

impl Run {
    fn group_size(&self, id: u32) -> usize {
        self.group_of.get(&id).map_or(1, |&g| self.groups[g - 1].len())
    }

    /// IDs by total coverage, biggest first.
    pub fn ranked(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.totals.keys().copied().filter(|&i| i >= FIRST_ID).collect();
        ids.sort_by(|a, b| self.totals[b].pixels.cmp(&self.totals[a].pixels).then(a.cmp(b)));
        ids
    }

    /// `inventory.tsv`: every texture and tile of the run.
    pub fn inventory_tsv(&self) -> String {
        let mut s = String::from("id\tname\tkind\tw\th\tsurface\tframes\tpixels\tbest_frame\tbest_pixels\tpolys\tuv_used_pct\twrap\tspan_u\tspan_v\tgroup\tgroup_size\tdetail\n");
        for id in self.ranked() {
            let (e, t) = (self.reg.entry(id).unwrap(), &self.totals[&id]);
            let used =
                if t.seen.is_empty() { "-".to_string() } else { format!("{:.1}", pct(t.used(), (e.w * e.h) as usize)) };
            let _ = writeln!(
                s,
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.2}\t{:.2}\t{}\t{}\t{}",
                id,
                e.name,
                e.kind(),
                e.w,
                e.h,
                if e.surface != 0 { format!("{:016x}", e.surface) } else { "-".into() },
                t.frames,
                t.pixels,
                t.best.0,
                t.best.1,
                t.polys,
                used,
                if t.wraps != 0 { wrap_names(t.wraps) } else { "-".into() },
                t.span[0],
                t.span[1],
                self.group_of.get(&id).map_or("-".to_string(), |g| g.to_string()),
                self.group_size(id),
                e.detail
            );
        }
        s
    }

    /// `groups.tsv`: one row per group of two or more.
    pub fn groups_tsv(&self) -> String {
        let mut s = String::from("group\tsize\tpixels\tmembers\n");
        for (i, g) in self.groups.iter().enumerate().filter(|(_, g)| g.len() > 1) {
            let px: usize = g.iter().map(|id| self.totals[id].pixels).sum();
            let m: Vec<String> = g.iter().map(|id| id.to_string()).collect();
            let _ = writeln!(s, "{}\t{}\t{}\t{}", i + 1, g.len(), px, m.join(","));
        }
        s
    }

    /// `ids.tsv`: the ID table.
    pub fn ids_tsv(&self) -> String {
        let mut s = String::from("id\tname\n0\tnone\n1\tflat\n");
        for (i, e) in self.reg.entries.iter().enumerate() {
            let _ = writeln!(s, "{}\t{}", FIRST_ID as usize + i, e.name);
        }
        s
    }

    /// `texmap.json`: all of the above, for tools.
    pub fn json(&self, game: &str) -> String {
        let mut s = format!("{{\"game\":{},\"frames\":[", json_str(game));
        for (i, f) in self.frames.iter().enumerate() {
            let _ = write!(
                s,
                "{}{{\"frame\":{},\"width\":{},\"height\":{},\"textures\":[",
                if i > 0 { "," } else { "" },
                f.frame,
                f.width,
                f.height
            );
            for (j, st) in f.stats.iter().enumerate() {
                let _ = write!(
                    s,
                    "{}{{\"id\":{},\"pixels\":{},\"bbox\":[{},{},{},{}],\"polys\":{},\"uv_box\":[{},{},{},{}]}}",
                    if j > 0 { "," } else { "" },
                    st.id,
                    st.pixels,
                    st.bbox[0],
                    st.bbox[1],
                    st.bbox[2],
                    st.bbox[3],
                    st.polys,
                    st.uv_box[0] as i64,
                    st.uv_box[1] as i64,
                    st.uv_box[2],
                    st.uv_box[3]
                );
            }
            s.push_str("]}");
        }
        s.push_str("],\"textures\":[");
        for (i, id) in self.ranked().into_iter().enumerate() {
            let (e, t) = (self.reg.entry(id).unwrap(), &self.totals[&id]);
            let _ = write!(
                s,
                "{}{{\"id\":{},\"name\":{},\"kind\":{},\"w\":{},\"h\":{},\"surface\":{},\"frames\":{},\"pixels\":{},\"polys\":{},\"wrap\":{},\"group\":{},\"detail\":{}}}",
                if i > 0 { "," } else { "" },
                id,
                json_str(&e.name),
                json_str(e.kind()),
                e.w,
                e.h,
                if e.surface != 0 { json_str(&format!("{:016x}", e.surface)) } else { "null".into() },
                t.frames,
                t.pixels,
                t.polys,
                json_str(&wrap_names(t.wraps)),
                self.group_of.get(&id).copied().unwrap_or(0),
                json_str(&e.detail)
            );
        }
        s.push_str("],\"groups\":[");
        for (i, g) in self.groups.iter().filter(|g| g.len() > 1).enumerate() {
            let m: Vec<String> = g.iter().map(|id| id.to_string()).collect();
            let _ = write!(s, "{}[{}]", if i > 0 { "," } else { "" }, m.join(","));
        }
        s.push_str("]}\n");
        s
    }
}

/// Touching pixel pairs two textures need before they are linked: about
/// one pair per 96 rows of the view (8 at 768 high), so a corner touching
/// is not a link and a shared edge is.
pub fn min_edge(view: &View) -> u64 {
    (view.height as u64 / 96).max(2)
}

/// Run `texmap`: every `o.every`-th frame of the capture at `root`,
/// results in `out_dir`.
pub fn run(root: &Path, out_dir: &Path, game: &str, o: &Options) -> Result<(), String> {
    let rom = capture::load_texture_rom(root)
        .map_err(|e| format!("{}: {e} (capture with `make capture`)", root.display()))?;
    let frames = capture::frames(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let fdir = out_dir.join("frames");
    let tdir = out_dir.join("textures");
    for d in [&fdir, &tdir] {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let mut reg = Registry::default();
    let mut edges = Edges::default();
    let mut outs: Vec<FrameOut> = Vec::new();
    let mut totals: HashMap<u32, Total> = HashMap::new();
    for (n, item) in frames.enumerate() {
        let (dir, f) = item.map_err(|e| e.to_string())?;
        if n % o.every.max(1) != 0 {
            continue;
        }
        let (picture, ids, _) = render_frame(&f, &rom, o, &mut reg);
        let st = stats(&ids, &reg);
        edges.add_frame(&ids);
        for s in &st {
            totals.entry(s.id).or_default().add(f.frame, s);
        }
        let name = |s: &str| fdir.join(format!("{:06}{s}", f.frame));
        let w = |p: std::path::PathBuf, b: Vec<u8>| std::fs::write(&p, b).map_err(|e| format!("{}: {e}", p.display()));
        w(name(".png"), png::encode_rgb(ids.width, ids.height, &picture))?;
        w(name("-ids.png"), id_png(&ids))?;
        w(name("-uv.png"), uv_png(&ids, &reg))?;
        let mut by_cover: Vec<&Stat> = st.iter().filter(|s| s.id >= FIRST_ID).collect();
        by_cover.sort_by(|a, b| b.pixels.cmp(&a.pixels).then(a.id.cmp(&b.id)));
        let (cw, ch, sheet) = contact_sheet(&picture, ids.width, ids.height, &by_cover, &reg);
        w(name("-contact.png"), png::encode_rgb(cw, ch, &sheet))?;
        let count = |k: &str| by_cover.iter().filter(|s| reg.entry(s.id).is_some_and(|e| e.kind() == k)).count();
        println!(
            "frame {:<6} {:>4} textures and tiles ({} 3D, {} tiles) from {}",
            f.frame,
            by_cover.len(),
            count("tex"),
            count("tile"),
            dir.display()
        );
        outs.push(FrameOut { frame: f.frame, width: ids.width, height: ids.height, stats: st });
    }
    if outs.is_empty() {
        return Err(format!("{}: no captured frames", root.display()));
    }
    let pixels: HashMap<u32, usize> =
        totals.iter().filter(|(&id, _)| id >= FIRST_ID).map(|(&id, t)| (id, t.pixels)).collect();
    let groups = group(&edges, &pixels, min_edge(&o.view), o.group_cap);
    let group_of = groups.iter().enumerate().flat_map(|(i, g)| g.iter().map(move |&id| (id, i + 1))).collect();
    let run = Run { reg, frames: outs, totals, groups, group_of };
    for e in &run.reg.entries {
        let p = tdir.join(format!("{}.png", e.name));
        std::fs::write(&p, png::encode_rgba(e.image.width, e.image.height, &e.image.data))
            .map_err(|e| format!("{}: {e}", p.display()))?;
    }
    let put = |name: &str, text: String| {
        let p = out_dir.join(name);
        std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))
    };
    put("ids.tsv", run.ids_tsv())?;
    put("inventory.tsv", run.inventory_tsv())?;
    put("groups.tsv", run.groups_tsv())?;
    put("texmap.json", run.json(game))?;
    for fo in &run.frames {
        put(&format!("frames/{:06}.tsv", fo.frame), frame_tsv(fo, &run.reg, &run.group_of))?;
    }
    put("index.html", crate::texmap_viewer::html(&run, out_dir, game, o.viewer_frames)?)?;
    let multi = run.groups.iter().filter(|g| g.len() > 1).count();
    println!(
        "{} frames, {} textures and tiles in {} groups of two or more (cap {}, min edge {} pixel pairs)",
        run.frames.len(),
        run.reg.entries.len(),
        multi,
        o.group_cap,
        min_edge(&o.view)
    );
    println!("viewer        {}/index.html", out_dir.display());
    println!("inventory     {}/inventory.tsv, groups.tsv, texmap.json, ids.tsv", out_dir.display());
    println!("per frame     {}/frames/FRAME{{.png,-ids.png,-uv.png,-contact.png,.tsv}}", out_dir.display());
    println!("textures      {}/textures/ (the pack's file names)", out_dir.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use model2::raster::SVtx;

    #[test]
    fn id_round_trip() {
        for id in [0, 1, 2, 255, 256, 65535, 65536, 0xff_ffff] {
            assert_eq!(decode_id(encode_id(id)), id);
        }
        assert_eq!(encode_id(0x01_02_03), [3, 2, 1]);
    }

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32, tex: (u32, u32, u32, u32), key: f32, block: u32) -> Poly {
        // A screen-aligned rectangle covering texels (0,0)-(tex.2,tex.3)
        // of the texture at sheet position (tex.0, tex.1), 1/z = 1.
        let (w, h) = (tex.2 as f32, tex.3 as f32);
        let v = |x, y, u, v| SVtx { x, y, iz: 1.0, uz: u, vz: v };
        Poly {
            points: vec![v(x0, y0, 0.0, 0.0), v(x1, y0, w, 0.0), v(x1, y1, w, h), v(x0, y1, 0.0, h)],
            key,
            window: 0,
            clip: (0.0, 0.0, 64.0, 64.0),
            texture: Some(raster::Texture {
                x0: tex.0,
                y0: tex.1,
                w: tex.2,
                h: tex.3,
                mirror_x: false,
                mirror_y: false,
                bank: 0,
                translucent: false,
                luma_table: 0,
            }),
            colour: [31, 31, 31],
            luma: 255,
            checker: false,
            header: [0; 4],
            block,
            at: 0,
            prev: Vec::new(),
            shadow: false,
            lit: Vec::new(),
            pixel: None,
            lit_range: [0, 255],
            fine: false,
            lamp: None,
        }
    }

    /// Three rectangles on a 64x64 view: two textures side by side in one
    /// object block, a third in another, and one texture repeated twice.
    fn synthetic(reg: &mut Registry) -> IdFrame {
        let mut ram = vec![0u8; 0x40_0000];
        // Give the textures different texels so their keys differ.
        ram[0] = 0x12;
        ram[0x8000] = 0x34;
        ram[0x10000] = 0x56;
        let palette = vec![0u8; 0x2_0000];
        let rom = [0u16; 4];
        let src = Sources {
            texture_rom: &rom,
            texture_point_ram: &[],
            render_ram: &ram,
            palette: &palette,
            xoff: 0.0,
            yoff: 0.0,
        };
        let view = View { width: 64, height: 64, sx: 1.0, sy: 1.0, ..View::native() };
        let mut polys = vec![
            // Texture A (16x16 at sheet x 0..16, y 0), left half, block 7.
            rect(0.0, 0.0, 16.0, 32.0, (0, 0, 16, 16), 9.0, 7),
            // Texture B next to it, same block.
            rect(16.0, 0.0, 32.0, 32.0, (0, 64, 16, 16), 8.0, 7),
            // Texture C elsewhere, another block, touching B's right edge.
            rect(32.0, 0.0, 48.0, 32.0, (0, 128, 16, 16), 7.0, 8),
            // A again, bottom, with its coordinates doubled: repeats twice.
            rect(0.0, 40.0, 16.0, 56.0, (0, 0, 16, 16), 6.0, 9),
        ];
        polys[3].points.iter_mut().for_each(|p| {
            p.uz *= 2.0;
            p.vz *= 2.0;
        });
        let placed = raster::place(&mut polys, &view);
        let (vis, uv) = raster::visible_uv(&polys, &placed, &src, &view);
        identify(&polys, &placed, &vis, &uv, None, &view, &src, reg)
    }

    #[test]
    fn identifies_and_measures_textures() {
        let mut reg = Registry::default();
        let f = synthetic(&mut reg);
        assert_eq!(reg.entries.len(), 3, "A, B and C: A's second use is the same key");
        let id_at = |x: usize, y: usize| f.ids[y * 64 + x];
        let (a, b, c) = (id_at(4, 4), id_at(20, 4), id_at(40, 4));
        assert!(a >= FIRST_ID && b >= FIRST_ID && c >= FIRST_ID);
        assert!(a != b && b != c && a != c);
        assert_eq!(id_at(4, 44), a, "the same texture is the same ID");
        assert_eq!(id_at(60, 60), NONE_ID);
        let st = stats(&f, &reg);
        let sa = st.iter().find(|s| s.id == a).unwrap();
        assert_eq!(sa.pixels, 16 * 32 + 16 * 16);
        assert_eq!(sa.polys, 2);
        assert_eq!(sa.bbox, [0, 0, 15, 55]);
        // The second polygon covers the texture twice over: all of it
        // shows, and the extent is 2 texture widths.
        assert_eq!(sa.used, 16 * 16);
        assert!((sa.span[0] - 2.0).abs() < 0.2, "{:?}", sa.span);
        let sb = st.iter().find(|s| s.id == b).unwrap();
        assert_eq!((sb.pixels, sb.polys), (16 * 32, 1));
        // 16 rows of the 32 drawn... the rectangle samples v in 0..16 over
        // 32 pixels: the texture is all visible, stretched.
        assert_eq!(sb.uv_box, [0, 0, 15, 15]);
        assert_eq!(wrap_names(sb.wraps), "rr");
        // Uv image: the pixel at the middle of B's left edge is near u 0.
        assert!(f.uv[4 * 64 + 16][0] < 1.0);
    }

    #[test]
    fn partial_uv_use() {
        // A rectangle showing only the left half of a 16x16 texture.
        let mut reg = Registry::default();
        let (ram, palette, rom) = (vec![0u8; 0x40_0000], vec![0u8; 0x2_0000], [0u16; 4]);
        let src = Sources {
            texture_rom: &rom,
            texture_point_ram: &[],
            render_ram: &ram,
            palette: &palette,
            xoff: 0.0,
            yoff: 0.0,
        };
        let view = View { width: 32, height: 32, sx: 1.0, sy: 1.0, ..View::native() };
        let mut p = rect(0.0, 0.0, 16.0, 16.0, (0, 0, 16, 16), 1.0, 1);
        p.points[1].uz = 8.0;
        p.points[2].uz = 8.0;
        let mut polys = vec![p];
        let placed = raster::place(&mut polys, &view);
        let (vis, uv) = raster::visible_uv(&polys, &placed, &src, &view);
        let f = identify(&polys, &placed, &vis, &uv, None, &view, &src, &mut reg);
        let s = &stats(&f, &reg)[0];
        assert_eq!(s.uv_box, [0, 0, 7, 15]);
        assert_eq!(s.used, 8 * 16);
        assert!(s.span[0] < 0.6 && s.span[1] > 0.9);
    }

    #[test]
    fn edges_link_touching_textures_of_one_block() {
        let mut reg = Registry::default();
        let f = synthetic(&mut reg);
        let mut e = Edges::default();
        e.add_frame(&f);
        let (a, b, c) = (f.ids[4 * 64 + 4], f.ids[4 * 64 + 20], f.ids[4 * 64 + 40]);
        // A and B share a 32-pixel edge in one block; B and C touch but
        // are different blocks: no edge.
        assert_eq!(e.0.get(&(a.min(b), a.max(b))), Some(&32));
        assert_eq!(e.0.get(&(b.min(c), b.max(c))), None);
    }

    #[test]
    fn grouping_caps_and_thresholds() {
        let mut e = Edges::default();
        // A chain 2-3-4-5-6 with a weak link in the middle, and a lone 7.
        for (a, b, w) in [(2, 3, 50), (3, 4, 40), (4, 5, 3), (5, 6, 30), (6, 7, 1)] {
            e.0.insert((a, b), w);
        }
        let px: HashMap<u32, usize> = (2..=7).map(|i| (i, 100 - i as usize)).collect();
        // The weak link (3) is under the threshold: two groups, and 7 alone.
        let g = group(&e, &px, 8, 10);
        assert_eq!(g, vec![vec![2, 3, 4], vec![5, 6], vec![7]]);
        // With a low threshold and a cap of 3 the chain splits at the
        // cap, strongest edges first.
        let g = group(&e, &px, 1, 3);
        assert_eq!(g, vec![vec![2, 3, 4], vec![5, 6, 7]]);
        // Every ID is in exactly one group.
        assert_eq!(g.iter().map(Vec::len).sum::<usize>(), px.len());
    }

    #[test]
    fn wrap_names_match_the_pack_index() {
        assert_eq!(wrap_names(1), "rr");
        assert_eq!(wrap_names(0b0011), "rr,mr");
        assert_eq!(wrap_names(0b1000), "mm");
    }

    #[test]
    fn contact_sheet_lays_out_the_frame_and_labels() {
        let mut reg = Registry::default();
        let f = synthetic(&mut reg);
        let st = stats(&f, &reg);
        let mut shown: Vec<&Stat> = st.iter().collect();
        shown.sort_by_key(|a| std::cmp::Reverse(a.pixels));
        let pic = vec![7u8; 64 * 64 * 3];
        let (w, h, img) = contact_sheet(&pic, 64, 64, &shown, &reg);
        assert!(w > 64 && h == 104 && img.len() == w * h * 3);
        assert_eq!(&img[..3], &[7, 7, 7], "the frame is on the left");
    }
}
