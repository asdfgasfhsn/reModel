//! Smooth shading, an enhancement (off by default): brightness that varies
//! across a polygon instead of the one luma the geometrizer gave it.
//!
//! This is the generic, stream-level layer: it needs nothing but the
//! raster stream, so it works for every title, either geometrizer path,
//! old captures and the browser. Per object block (one model under one
//! transform) it welds the polygons' vertices by their exact stream words,
//! groups the faces around each vertex into smoothing groups (faces that
//! share an edge, meet at less than the crease angle and look like one
//! material), and gives each polygon corner the angle-weighted mean of its
//! group's face lumas. A polygon is drawn lit only if one of its corners
//! actually varies; every other polygon keeps the stream's integer luma,
//! so planar regions, isolated polygons and everything at crease 0 are the
//! original picture bit for bit. The colour path (luma tables, the 64
//! columns of the game's colour translation) is untouched: a lit pixel's
//! luma is the interpolated corner value, truncated as the hardware
//! truncates, and goes where the polygon's luma went.
//!
//! The pass reads the stream exactly as `raster::build` does and keys its
//! results by the stream index of each polygon's attribute word, so a
//! parsing disagreement can only leave a polygon flat, never shade the
//! wrong one. Nothing here is game- or board-specific beyond the raster
//! stream format (Model 2A, 2B and 2C share it): a board with another
//! stream needs only its own [`Mesh`] adapter. Per-game rules come from
//! `romset::Shading` ([`Options::new`]).
//!
//! Later layers (the design's stages 2 and 4) add a side channel from the
//! native geometrizers (true focal lengths, the game's own normals and
//! lighting entries) and the game's lighting formula per vertex and per
//! pixel; this layer is what every title gets without them.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// A fast hasher for the weld map's keys (three 24-bit words): one
/// multiply per word.
#[derive(Default)]
struct WeldHasher(u64);

impl Hasher for WeldHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u32(b as u32);
        }
    }

    fn write_u32(&mut self, v: u32) {
        self.0 = (self.0.rotate_left(23) ^ v as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }

    fn write_usize(&mut self, v: usize) {
        self.write_u32(v as u32);
    }
}
use std::time::Instant;

/// How polygons are shaded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Shading {
    /// One luma per polygon, as the game drew it (the default).
    #[default]
    Flat,
    /// Per-vertex brightness, interpolated across the polygon.
    Vertex,
    /// Per-pixel lighting with interpolated normals. Needs the exact layer
    /// (not built yet): drawn as `Vertex`.
    Pixel,
}

impl Shading {
    /// `flat` (or `off`), `vertex` or `pixel`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "flat" | "off" => Some(Shading::Flat),
            "vertex" => Some(Shading::Vertex),
            "pixel" => Some(Shading::Pixel),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Shading::Flat => "flat",
            Shading::Vertex => "vertex",
            Shading::Pixel => "pixel",
        }
    }
}

/// A crease angle meaning "the game's own" (`romset::Shading::crease`).
pub const CREASE_AUTO: u8 = 255;

/// Without the true focal lengths (the side channel, not built yet), the
/// guess: `fx = fy = FOCAL_GUESS * window width`. Only the crease test and
/// the angle weights depend on it.
pub const FOCAL_GUESS: f32 = 1.0;

/// The material stand-in (the lighting entry isn't in the stream): two
/// faces sharing an edge are one material only if their lumas differ by
/// at most `MATERIAL_SLACK + MATERIAL_SLOPE * 255 * |na - nb|` (the
/// difference a diffuse light could make between their normals, plus
/// rounding): a self-lit sign on a lit wall stays a separate surface.
/// The slope was 1.5 in the design; 0.6 (a diffuse coefficient up to
/// about 150) splits the saturated (255) and differently lit parts the
/// captures showed blended into their neighbours (lastbrnx, schamp) and
/// changes nothing visible on curved surfaces of one material.
pub const MATERIAL_SLACK: f32 = 6.0;
pub const MATERIAL_SLOPE: f32 = 0.6;

/// `crease` parsed: `auto` or degrees 0 to 180.
pub fn parse_crease(s: &str) -> Option<u8> {
    match s {
        "auto" => Some(CREASE_AUTO),
        n => n.parse::<u8>().ok().filter(|n| *n <= 180),
    }
}

/// What [`shade`] does for a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    pub shading: Shading,
    /// Degrees, 0 to 180 (never [`CREASE_AUTO`]: [`Options::new`] resolves
    /// it). 0: nothing is smoothed.
    pub crease: u8,
    /// Windows never shaded, a bit each (HUD windows and the game's
    /// `skip_windows`).
    pub skip_windows: u64,
    /// Checkerboard polygons stay flat (applied by `raster::build_with`,
    /// which knows the polygon headers).
    pub skip_checker: bool,
    /// Fill [`Shaded::info`] (for tools).
    pub diagnose: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { shading: Shading::Flat, crease: 0, skip_windows: 0, skip_checker: false, diagnose: false }
    }
}

fn window_mask(windows: &[usize]) -> u64 {
    windows.iter().filter(|&&w| w < 64).fold(0, |m, &w| m | 1 << w)
}

impl Options {
    /// The options for a game: `crease` in degrees or [`CREASE_AUTO`] (the
    /// game's `rules.crease`); its HUD windows are never shaded.
    pub fn new(shading: Shading, crease: u8, rules: &romset::Shading, hud_windows: &[usize]) -> Self {
        Self::with_mask(shading, crease, rules, window_mask(hud_windows))
    }

    /// [`Options::new`] with the HUD windows as a mask (bit per window).
    pub fn with_mask(shading: Shading, crease: u8, rules: &romset::Shading, hud_windows: u64) -> Self {
        Options {
            shading,
            crease: if crease == CREASE_AUTO { rules.crease } else { crease.min(180) },
            skip_windows: hud_windows | window_mask(rules.skip_windows),
            skip_checker: rules.skip_checker,
            diagnose: false,
        }
    }

    /// Whether [`shade`] can light anything.
    pub fn on(&self) -> bool {
        self.shading != Shading::Flat && self.crease > 0
    }
}

/// One object block of a [`Mesh`].
#[derive(Clone, Debug)]
pub struct MeshBlock {
    /// The window (viewport) it is drawn in, counted from 0 in the frame.
    pub window: usize,
    /// Focal lengths (x, y): positions are view space times these.
    pub focal: (f32, f32),
    /// Its polygons in [`Mesh::polys`] and vertices in [`Mesh::verts`].
    pub polys: std::ops::Range<usize>,
    pub verts: std::ops::Range<usize>,
}

/// One polygon record of a [`Mesh`].
#[derive(Clone, Copy, Debug)]
pub struct MeshPoly {
    /// Stream index of its attribute word: the key results are given by.
    pub at: u32,
    /// Ring (P1', P0', Pa, Pb) as vertex ids (welded within the block);
    /// `corners` of them are used (3 for a triangle).
    pub ring: [u32; 4],
    pub corners: u8,
    /// The stream's luma, 0-255.
    pub luma: u8,
    /// The stream's back-face bit.
    pub back: bool,
    /// Attribute bit 17.
    pub double_sided: bool,
    /// Attribute bits 8-9 (0: advances the strip, never drawn).
    pub link: u8,
}

/// A frame's polygons as `shade` needs them: welded vertex positions and
/// per polygon its ring, luma and facing. Board-neutral;
/// [`mesh_from_stream`] makes one from a Model 2A/2B/2C raster stream.
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub blocks: Vec<MeshBlock>,
    pub polys: Vec<MeshPoly>,
    /// Positions as the stream has them: view space times the focal
    /// lengths (x, y), z.
    pub verts: Vec<[f32; 3]>,
}

fn f24(w: u32) -> f32 {
    f32::from_bits(w << 8)
}

fn s12(v: u32) -> f32 {
    (((v & 0xfff) << 20) as i32 >> 20) as f32
}

/// Parse a raster stream into a [`Mesh`], reading it as `raster::build`
/// does (windows, `0x1` object blocks, linked polygon records). Vertices
/// are welded per block by their three 24-bit words exactly (-0 taken as
/// 0); the focal lengths are guessed from the window width
/// ([`FOCAL_GUESS`]).
pub fn mesh_from_stream(stream: &[u32]) -> Mesh {
    let mut mesh = Mesh::default();
    let rd = |i: usize| stream.get(i).copied().unwrap_or(0xff00_000f);
    let mut widths: Vec<f32> = Vec::new();
    let mut ids: HashMap<[u32; 3], u32, BuildHasherDefault<WeldHasher>> = HashMap::default();
    let mut i = 0;
    while i < stream.len() {
        let w = stream[i];
        i += 1;
        if w >> 24 != 0xff {
            continue;
        }
        match w & 0xf {
            0x3 => {
                let x = |k: usize| s12(rd(i + k) >> 12);
                widths.push((x(1) - x(0)).abs());
                i += 6;
            }
            0x8 => i += 1,
            0x4 => {
                let count = rd(i + 1) as usize;
                i += 2 + count;
            }
            0x1 => {
                let Some(win) = widths.len().checked_sub(1) else { continue };
                ids.clear();
                let vstart = mesh.verts.len();
                let pstart = mesh.polys.len();
                let mut vertex = |k: usize, mesh: &mut Mesh| -> u32 {
                    let key = [0, 1, 2].map(|c| {
                        let w = rd(k + c) & 0xff_ffff;
                        if w == 0x80_0000 {
                            0
                        } else {
                            w
                        }
                    });
                    *ids.entry(key).or_insert_with(|| {
                        mesh.verts.push(key.map(f24));
                        (mesh.verts.len() - 1) as u32
                    })
                };
                let mut p0 = vertex(i + 2, &mut mesh);
                let mut p1 = vertex(i + 5, &mut mesh);
                i += 8;
                loop {
                    let attr = rd(i);
                    if attr >> 24 == 0xff || attr & 3 == 0 {
                        break;
                    }
                    let quad = attr & 1 != 0;
                    let luma_word = rd(i + 1);
                    let at = i as u32;
                    let pa = vertex(i + 3, &mut mesh);
                    let pb = if quad { vertex(i + 6, &mut mesh) } else { pa };
                    i += if quad { 9 } else { 6 };
                    let link = ((attr >> 8) & 3) as u8;
                    mesh.polys.push(MeshPoly {
                        at,
                        ring: [p1, p0, pa, pb],
                        corners: if quad { 4 } else { 3 },
                        luma: ((luma_word >> 15) & 0xff) as u8,
                        back: luma_word & (1 << 23) != 0,
                        double_sided: attr & (1 << 17) != 0,
                        link,
                    });
                    (p0, p1) = match link {
                        1 => (p0, pa),
                        3 => (pb, p1),
                        _ => (pa, pb),
                    };
                }
                let f = FOCAL_GUESS * widths[win].max(1.0);
                mesh.blocks.push(MeshBlock {
                    window: win,
                    focal: (f, f),
                    polys: pstart..mesh.polys.len(),
                    verts: vstart..mesh.verts.len(),
                });
            }
            _ => {}
        }
    }
    mesh
}

/// What became of a polygon record ([`Shaded::info`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Class {
    /// Drawn with corner values.
    Lit,
    /// Took part, but no corner of it varies: the stream's luma.
    Flat,
    /// Link 0: advances the strip, never drawn.
    Hidden,
    /// A back face that isn't double-sided: culled by the rasteriser.
    Culled,
    /// No usable face normal (zero area).
    Degenerate,
    /// In a window that is never shaded (HUD).
    Excluded,
}

/// Per polygon record, with [`Options::diagnose`].
#[derive(Clone, Copy, Debug)]
pub struct Info {
    /// Stream index of its attribute word.
    pub at: u32,
    pub class: Class,
    /// Its object block, counted from 0 in the frame.
    pub block: u32,
    /// Ring edges (bit k: corner k to corner k + 1) shared with another
    /// face but kept hard: by the crease angle (`crease`) or as a change
    /// of material (`material`).
    pub crease: u8,
    pub material: u8,
}

/// Counts for one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub polys: u32,
    pub lit: u32,
    pub flat: u32,
    pub hidden: u32,
    pub culled: u32,
    pub degenerate: u32,
    pub excluded: u32,
    /// Shared edges (between two faces taking part), and of them kept hard
    /// by the crease angle and by the material rule.
    pub edges: u32,
    pub crease_edges: u32,
    pub material_splits: u32,
    /// Time [`shade`] took, ms.
    pub ms: f32,
}

impl Stats {
    pub fn add(&mut self, o: &Stats) {
        self.polys += o.polys;
        self.lit += o.lit;
        self.flat += o.flat;
        self.hidden += o.hidden;
        self.culled += o.culled;
        self.degenerate += o.degenerate;
        self.excluded += o.excluded;
        self.edges += o.edges;
        self.crease_edges += o.crease_edges;
        self.material_splits += o.material_splits;
        self.ms += o.ms;
    }
}

/// [`shade`]'s result: the corner values of the polygons drawn lit.
#[derive(Clone, Debug, Default)]
pub struct Shaded {
    /// Stream index of the attribute word of each lit polygon, ascending.
    pub at: Vec<u32>,
    /// Per lit polygon, in ring order (P1', P0', Pa, Pb): `[luma, nx, ny,
    /// nz]`, luma as a float 0-255 before truncation; the normal is zero
    /// (none) at this level.
    pub corners: Vec<[[f32; 4]; 4]>,
    pub stats: Stats,
    /// Every polygon record, with [`Options::diagnose`]; else empty.
    pub info: Vec<Info>,
}

impl Shaded {
    /// The corner values of the polygon whose attribute word is at stream
    /// index `at`, if it is lit.
    pub fn get(&self, at: u32) -> Option<&[[f32; 4]; 4]> {
        self.at.binary_search(&at).ok().map(|k| &self.corners[k])
    }
}

/// Smooth-shade a frame's raster stream: per polygon drawn lit, its corner
/// values. Empty when `opts` shades nothing ([`Options::on`]).
pub fn shade(stream: &[u32], opts: &Options) -> Shaded {
    if !opts.on() {
        return Shaded::default();
    }
    let t = Instant::now();
    let mut s = shade_mesh(&mesh_from_stream(stream), opts);
    s.stats.ms = t.elapsed().as_secs_f32() * 1e3;
    s
}

type V3 = [f32; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn find(parent: &mut [u32], mut x: u32) -> u32 {
    while parent[x as usize] != x {
        let up = parent[parent[x as usize] as usize];
        parent[x as usize] = up;
        x = up;
    }
    x
}

fn union(parent: &mut [u32], a: u32, b: u32) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb {
        parent[ra.max(rb) as usize] = ra.min(rb);
    }
}

/// A face taking part, within its block.
struct Face {
    poly: usize,
    n: V3,
    /// Its first corner in the block's corner arrays.
    corner: u32,
}

/// [`shade`] on a mesh (no timing).
pub fn shade_mesh(mesh: &Mesh, opts: &Options) -> Shaded {
    let mut out = Shaded::default();
    if !opts.on() {
        return out;
    }
    let c = (opts.crease as f32).to_radians().cos();
    // Reused per block.
    let mut faces: Vec<Face> = Vec::new();
    let mut face_of: Vec<u32> = Vec::new();
    let mut edges: Vec<(u32, u32, u32, u8)> = Vec::new();
    let mut parent: Vec<u32> = Vec::new();
    let mut weight: Vec<f32> = Vec::new();
    let mut members: Vec<(u32, u32)> = Vec::new();
    let mut hard: Vec<(u8, u8)> = Vec::new();
    let mut group_value: Vec<Option<f32>> = Vec::new();
    for (bi, b) in mesh.blocks.iter().enumerate() {
        let (fx, fy) = (
            if b.focal.0.is_finite() && b.focal.0 != 0.0 { b.focal.0 } else { 1.0 },
            if b.focal.1.is_finite() && b.focal.1 != 0.0 { b.focal.1 } else { 1.0 },
        );
        let pos = |v: u32| {
            let p = mesh.verts[v as usize];
            [p[0] / fx, p[1] / fy, p[2]]
        };
        let skipped = b.window < 64 && opts.skip_windows & (1 << b.window) != 0;
        faces.clear();
        face_of.clear();
        face_of.resize(b.polys.len(), u32::MAX);
        let mut classes = vec![Class::Flat; b.polys.len()];
        for (k, pi) in b.polys.clone().enumerate() {
            let p = &mesh.polys[pi];
            out.stats.polys += 1;
            classes[k] = if p.link == 0 {
                Class::Hidden
            } else if p.back && !p.double_sided {
                Class::Culled
            } else if skipped {
                Class::Excluded
            } else {
                match face_normal(p, &pos) {
                    Some(n) => {
                        face_of[k] = faces.len() as u32;
                        faces.push(Face { poly: pi, n, corner: 0 });
                        Class::Flat
                    }
                    None => Class::Degenerate,
                }
            };
        }
        // Corners, their weights (the ring's interior angle there).
        let mut nc = 0u32;
        weight.clear();
        for f in faces.iter_mut() {
            f.corner = nc;
            let p = &mesh.polys[f.poly];
            let n = p.corners as usize;
            for k in 0..n {
                let v = pos(p.ring[k]);
                let a = sub(pos(p.ring[(k + n - 1) % n]), v);
                let e = sub(pos(p.ring[(k + 1) % n]), v);
                let s = cross(a, e);
                let w = dot(s, s).sqrt().atan2(dot(a, e));
                weight.push(if w.is_finite() { w } else { 0.0 });
            }
            nc += n as u32;
        }
        parent.clear();
        parent.extend(0..nc);
        // A repeated vertex in one ring is one corner.
        for f in &faces {
            let p = &mesh.polys[f.poly];
            let n = p.corners as usize;
            for k in 0..n {
                for j in k + 1..n {
                    if p.ring[k] == p.ring[j] {
                        union(&mut parent, f.corner + k as u32, f.corner + j as u32);
                    }
                }
            }
        }
        // Shared edges: (lower id, higher id, face, corner k of the edge
        // k -> k + 1).
        edges.clear();
        for (fi, f) in faces.iter().enumerate() {
            let p = &mesh.polys[f.poly];
            let n = p.corners as usize;
            for k in 0..n {
                let (a, e) = (p.ring[k], p.ring[(k + 1) % n]);
                if a != e {
                    edges.push((a.min(e), a.max(e), fi as u32, k as u8));
                }
            }
        }
        edges.sort_unstable();
        hard.clear();
        hard.resize(faces.len(), (0, 0));
        let mut r = 0;
        while r < edges.len() {
            let mut e = r + 1;
            while e < edges.len() && (edges[e].0, edges[e].1) == (edges[r].0, edges[r].1) {
                e += 1;
            }
            for x in r..e {
                for y in x + 1..e {
                    let (fa, ka) = (edges[x].2 as usize, edges[x].3 as usize);
                    let (fb, kb) = (edges[y].2 as usize, edges[y].3 as usize);
                    if fa == fb {
                        continue;
                    }
                    let (pa, pb) = (&mesh.polys[faces[fa].poly], &mesh.polys[faces[fb].poly]);
                    out.stats.edges += 1;
                    let (na, nb) = (faces[fa].n, faces[fb].n);
                    if dot(na, nb) < c {
                        out.stats.crease_edges += 1;
                        hard[fa].0 |= 1 << ka;
                        hard[fb].0 |= 1 << kb;
                        continue;
                    }
                    let d = sub(na, nb);
                    let slack = MATERIAL_SLACK + MATERIAL_SLOPE * 255.0 * dot(d, d).sqrt();
                    if pa.back != pb.back || (pa.luma as f32 - pb.luma as f32).abs() > slack {
                        out.stats.material_splits += 1;
                        hard[fa].1 |= 1 << ka;
                        hard[fb].1 |= 1 << kb;
                        continue;
                    }
                    // The same two vertices in both: unite the corners at each.
                    let corner = |f: usize, v: u32| {
                        let p = &mesh.polys[faces[f].poly];
                        let k = p.ring[..p.corners as usize].iter().position(|&x| x == v).unwrap_or(0);
                        faces[f].corner + k as u32
                    };
                    for v in [edges[r].0, edges[r].1] {
                        union(&mut parent, corner(fa, v), corner(fb, v));
                    }
                }
            }
            r = e;
        }
        // Groups: the corners sharing a root, with their faces.
        members.clear();
        for (fi, f) in faces.iter().enumerate() {
            let n = mesh.polys[f.poly].corners as u32;
            for k in 0..n {
                members.push((find(&mut parent, f.corner + k), fi as u32));
            }
        }
        // (root, face) pairs: corners of one face in one group (a repeated
        // vertex) count once.
        members.sort_unstable();
        members.dedup();
        // Per root: the mean (or None where nothing varies); per (root,
        // face) of a duplicate face, its offset from that mean.
        group_value.clear();
        group_value.resize(nc as usize, None);
        let mut offsets: HashMap<(u32, u32), f32> = HashMap::new();
        let mut g = 0;
        while g < members.len() {
            let root = members[g].0;
            let mut e = g + 1;
            while e < members.len() && members[e].0 == root {
                e += 1;
            }
            let group = &members[g..e];
            let (lo, hi) = group.iter().fold((u8::MAX, 0u8), |(lo, hi), &(_, f)| {
                let l = mesh.polys[faces[f as usize].poly].luma;
                (lo.min(l), hi.max(l))
            });
            let value = (lo != hi).then(|| {
                let (mut sum, mut wsum) = (0.0f64, 0.0f64);
                for (k, &(_, f)) in group.iter().enumerate() {
                    let face = &faces[f as usize];
                    let p = &mesh.polys[face.poly];
                    // A face sent again over the same vertices (a decal,
                    // glass) counts once.
                    let mut ids = p.ring;
                    ids[..p.corners as usize].sort_unstable();
                    let dup = group[..k].iter().find(|&&(_, o)| {
                        let q = &mesh.polys[faces[o as usize].poly];
                        let mut qi = q.ring;
                        qi[..q.corners as usize].sort_unstable();
                        q.corners == p.corners && qi == ids
                    });
                    if let Some(&(_, o)) = dup {
                        // It takes the surface's values, offset by the
                        // difference of their lumas: a darker decal stays
                        // darker by as much.
                        let q = &mesh.polys[faces[o as usize].poly];
                        if q.luma != p.luma {
                            offsets.insert((root, f), p.luma as f32 - q.luma as f32);
                        }
                        continue;
                    }
                    // This face's weight at the group's vertex: the sum of
                    // its corners in the group (one, unless repeated).
                    let w: f32 = (0..p.corners as u32)
                        .filter(|&j| find(&mut parent, face.corner + j) == root)
                        .map(|j| weight[(face.corner + j) as usize])
                        .sum();
                    sum += w as f64 * (p.luma as f64 + 0.5);
                    wsum += w as f64;
                }
                if wsum > 0.0 {
                    (sum / wsum) as f32
                } else {
                    // No angle anywhere (all corners degenerate): the plain mean.
                    group.iter().map(|&(_, f)| mesh.polys[faces[f as usize].poly].luma as f32 + 0.5).sum::<f32>()
                        / group.len() as f32
                }
            });
            group_value[root as usize] = value;
            g = e;
        }
        for (fi, f) in faces.iter().enumerate() {
            let p = &mesh.polys[f.poly];
            let n = p.corners as usize;
            let mut vals = [None; 4];
            for (k, v) in vals.iter_mut().enumerate().take(n) {
                let root = find(&mut parent, f.corner + k as u32);
                let off = offsets.get(&(root, fi as u32)).copied().unwrap_or(0.0);
                *v = group_value[root as usize].map(|v| v + off);
            }
            let k = f.poly - b.polys.start;
            if vals.iter().any(|v| v.is_some()) {
                let own = p.luma as f32 + 0.5;
                let mut corners = [[0.0f32; 4]; 4];
                for (j, v) in vals.iter().enumerate() {
                    corners[j][0] = v.unwrap_or(own);
                }
                if n == 3 {
                    corners[3] = corners[2];
                }
                out.at.push(p.at);
                out.corners.push(corners);
                classes[k] = Class::Lit;
            }
        }
        for (k, cl) in classes.iter().enumerate() {
            match cl {
                Class::Lit => out.stats.lit += 1,
                Class::Flat => out.stats.flat += 1,
                Class::Hidden => out.stats.hidden += 1,
                Class::Culled => out.stats.culled += 1,
                Class::Degenerate => out.stats.degenerate += 1,
                Class::Excluded => out.stats.excluded += 1,
            }
            if opts.diagnose {
                let fi = face_of[k];
                let (crease, material) = if fi == u32::MAX { (0, 0) } else { hard[fi as usize] };
                out.info.push(Info {
                    at: mesh.polys[b.polys.start + k].at,
                    class: *cl,
                    block: bi as u32,
                    crease,
                    material,
                });
            }
        }
    }
    // Blocks come in stream order, so `at` is ascending already; keep the
    // promise whatever the mesh's source.
    if out.at.windows(2).any(|w| w[0] >= w[1]) {
        let mut order: Vec<usize> = (0..out.at.len()).collect();
        order.sort_by_key(|&k| out.at[k]);
        out.at = order.iter().map(|&k| out.at[k]).collect();
        out.corners = order.iter().map(|&k| out.corners[k]).collect();
    }
    out
}

/// A polygon's unit face normal in unscaled view space, oriented by the
/// game's facing (`n·Pa >= 0` is a front face, the programs' convention),
/// so mirrored models and either winding give the same answer. None for a
/// polygon with no area.
fn face_normal(p: &MeshPoly, pos: &impl Fn(u32) -> V3) -> Option<V3> {
    let n = p.corners as usize;
    let r: Vec<V3> = p.ring[..n].iter().map(|&v| pos(v)).collect();
    // (P1' - P0') x (Pa - P0'), as the programs compute it.
    let (e1, e2) = (sub(r[0], r[1]), sub(r[2], r[1]));
    let mut g = cross(e1, e2);
    let scale = dot(e1, e1) * dot(e2, e2);
    let gg = dot(g, g);
    if (gg.is_nan() || gg <= 1e-12 * scale) && n == 4 {
        // Newell's sum over the ring.
        g = [0.0; 3];
        for k in 0..n {
            let (a, b) = (r[k], r[(k + 1) % n]);
            g[0] += (a[1] - b[1]) * (a[2] + b[2]);
            g[1] += (a[2] - b[2]) * (a[0] + b[0]);
            g[2] += (a[0] - b[0]) * (a[1] + b[1]);
        }
    }
    let len2 = dot(g, g);
    let size = r.iter().map(|v| dot(sub(*v, r[1]), sub(*v, r[1]))).fold(0.0f32, f32::max);
    if !len2.is_finite() || len2 <= 1e-12 * size * size {
        return None;
    }
    let len = len2.sqrt();
    let mut g = g.map(|x| x / len);
    if (dot(g, r[2]) >= 0.0) == p.back {
        g = g.map(|x| -x);
    }
    Some(g)
}

/// The luma a pixel of a lit polygon is drawn with: its interpolated
/// corner value truncated and clamped to 0-255, as the hardware turns the
/// lighting result into the polygon's luma (CFIB); NaN gives 0.
pub fn quantise(lum: f32) -> u32 {
    if lum.is_nan() {
        0
    } else {
        lum.clamp(0.0, 255.0) as u32
    }
}

/// Raster streams for tests (here and in `raster`): one window, then
/// object blocks of polygons given by their corners.
#[cfg(test)]
pub(crate) mod test_stream {
    /// One polygon to draw: corners (3 or 4, in ring order P1', P0', Pa,
    /// Pb, view space times the focal length), luma, back-face bit,
    /// double-sided.
    #[derive(Clone, Debug)]
    pub struct TPoly {
        pub corners: Vec<[f32; 3]>,
        pub luma: u8,
        pub back: bool,
        pub double_sided: bool,
    }

    pub fn tpoly(corners: &[[f32; 3]], luma: u8) -> TPoly {
        TPoly { corners: corners.to_vec(), luma, back: false, double_sided: false }
    }

    /// A stream with one 496x384 window (centre at the middle) and one
    /// object block per entry of `blocks`. Each polygon is drawn with link
    /// 2, after a link-0 polygon that sets the strip's edge to its first
    /// two corners. Returns the stream and, per block, the stream index of
    /// each drawn polygon's attribute word.
    pub fn stream(blocks: &[Vec<TPoly>]) -> (Vec<u32>, Vec<Vec<u32>>) {
        let xy = |x: i32, y: i32| ((x as u32 & 0xfff) << 12) | (y as u32 & 0xfff);
        let mut s = vec![0xff00_0003, xy(0, 0), xy(495, 383), xy(248, 192), xy(248, 192), xy(248, 192), xy(248, 192)];
        let put = |s: &mut Vec<u32>, p: [f32; 3]| s.extend(p.map(|v| v.to_bits() >> 8));
        let mut ats = Vec::new();
        for b in blocks {
            let mut at = Vec::new();
            s.extend([0xff00_0001, 0, 0]);
            let first = b.first().map_or([[0.0; 3]; 2], |p| [p.corners[1], p.corners[0]]);
            put(&mut s, first[0]);
            put(&mut s, first[1]);
            for p in b {
                // Link 0: the next edge is (Pa, Pb) = (P0', P1') of the polygon.
                s.extend([1, 0, 0]);
                put(&mut s, p.corners[1]);
                put(&mut s, p.corners[0]);
                let quad = p.corners.len() == 4;
                at.push(s.len() as u32);
                s.extend([
                    quad as u32 | 2 | 2 << 8 | 1 << 10 | (p.double_sided as u32) << 17,
                    (p.luma as u32) << 15 | (p.back as u32) << 23,
                    0,
                ]);
                put(&mut s, p.corners[2]);
                if quad {
                    put(&mut s, p.corners[3]);
                }
            }
            ats.push(at);
        }
        s.push(0xff00_000f);
        (s, ats)
    }
}

#[cfg(test)]
mod tests {
    use super::test_stream::{stream, tpoly, TPoly};
    use super::*;

    fn opts(crease: u8) -> Options {
        Options { shading: Shading::Vertex, crease, diagnose: true, ..Default::default() }
    }

    /// Focal length the test streams are made with (the guess for a
    /// 495-pixel window).
    const F: f32 = 495.0;

    /// A view-space point to stream space.
    fn sp(p: [f32; 3]) -> [f32; 3] {
        [p[0] * F, p[1] * F, p[2]]
    }

    /// Lighting of a face of normal n: what a game might send.
    fn lambert(n: [f32; 3]) -> u8 {
        let l = [0.3f32, -0.5, 0.81];
        let len = dot(l, l).sqrt();
        // Linear (no clamp at the terminator): the mean is kept exactly.
        (128.0 + 100.0 * dot(n, l) / len) as u8
    }

    /// A box (6 quads) around `c` facing the eye, lit by `lambert`.
    fn cube(c: [f32; 3], s: f32) -> Vec<TPoly> {
        let v = |x: f32, y: f32, z: f32| [c[0] + x * s, c[1] + y * s, c[2] + z * s];
        let faces = [
            ([0.0, 0.0, -1.0], [v(-1., -1., -1.), v(1., -1., -1.), v(1., 1., -1.), v(-1., 1., -1.)]),
            ([0.0, 0.0, 1.0], [v(-1., -1., 1.), v(-1., 1., 1.), v(1., 1., 1.), v(1., -1., 1.)]),
            ([-1.0, 0.0, 0.0], [v(-1., -1., -1.), v(-1., 1., -1.), v(-1., 1., 1.), v(-1., -1., 1.)]),
            ([1.0, 0.0, 0.0], [v(1., -1., -1.), v(1., -1., 1.), v(1., 1., 1.), v(1., 1., -1.)]),
            ([0.0, -1.0, 0.0], [v(-1., -1., -1.), v(-1., -1., 1.), v(1., -1., 1.), v(1., -1., -1.)]),
            ([0.0, 1.0, 0.0], [v(-1., 1., -1.), v(1., 1., -1.), v(1., 1., 1.), v(-1., 1., 1.)]),
        ];
        faces
            .iter()
            .map(|(n, q)| {
                // Outward normal n: a front face (n·P >= 0) faces away
                // from the eye, as the programs have it; all double-sided.
                let back = dot(*n, q[0]) < 0.0;
                TPoly { corners: q.map(sp).to_vec(), luma: lambert(*n), back, double_sided: true }
            })
            .collect()
    }

    /// An open cylinder of `sides` quads around the vertical axis through `c`.
    fn cylinder(c: [f32; 3], r: f32, h: f32, sides: usize) -> Vec<TPoly> {
        let at = |k: usize, y: f32| {
            let a = k as f32 / sides as f32 * std::f32::consts::TAU;
            [c[0] + r * a.cos(), c[1] + y, c[2] + r * a.sin()]
        };
        (0..sides)
            .map(|k| {
                let q = [at(k, 0.0), at(k + 1, 0.0), at(k + 1, h), at(k, h)];
                let mid = (k as f32 + 0.5) / sides as f32 * std::f32::consts::TAU;
                let n = [mid.cos(), 0.0, mid.sin()];
                let back = dot(n, q[0]) < 0.0;
                TPoly { corners: q.map(sp).to_vec(), luma: lambert(n), back, double_sided: true }
            })
            .collect()
    }

    fn classes(s: &Shaded) -> Vec<Class> {
        s.info.iter().filter(|i| i.class != Class::Hidden).map(|i| i.class).collect()
    }

    #[test]
    fn a_cube_stays_flat_and_a_cylinder_smooths() {
        let (s, _) = stream(&[cube([0.0, 0.0, 10.0], 1.0)]);
        let sh = shade(&s, &opts(40));
        assert!(sh.at.is_empty(), "{:?}", classes(&sh));
        assert_eq!(sh.stats.flat, 6);
        assert_eq!(sh.stats.crease_edges, 12);
        let (s, ats) = stream(&[cylinder([0.0, -0.5, 12.0], 1.0, 1.0, 16)]);
        let sh = shade(&s, &opts(40));
        // Every side varies but the ones whose neighbours are both lit alike.
        assert!(sh.at.len() >= 12, "{:?}", classes(&sh));
        assert_eq!(sh.stats.lit + sh.stats.flat, 16);
        assert_eq!(sh.stats.crease_edges, 0);
        // Mean preservation: each lit side's corners average within a few
        // levels of its own luma.
        // (Sides at the silhouette, whose neighbour faces the other way,
        // are a material change on that side: not averaged there.)
        let split = |at: u32| sh.info.iter().any(|i| i.at == at && i.material != 0);
        for (k, at) in ats[0].iter().enumerate() {
            if let (Some(c), false) = (sh.get(*at), split(*at)) {
                let own = cylinder([0.0, -0.5, 12.0], 1.0, 1.0, 16)[k].luma as f32 + 0.5;
                let mean = c.iter().map(|v| v[0]).sum::<f32>() / 4.0;
                assert!((mean - own).abs() < 8.0, "side {k}: {mean} vs {own}");
                // Two corners shared with each neighbour: values between.
                assert!(c.iter().all(|v| v[0] >= 0.0 && v[0] <= 256.0));
            }
        }
        // At 20 degrees (16 sides meet at 22.5) it is faceted again.
        assert!(shade(&s, &opts(20)).at.is_empty());
    }

    #[test]
    fn crease_zero_and_flat_shade_nothing() {
        let (s, _) = stream(&[cylinder([0.0, -0.5, 12.0], 1.0, 1.0, 16)]);
        assert!(shade(&s, &opts(0)).at.is_empty());
        assert_eq!(shade(&s, &opts(0)).stats, Stats::default());
        let flat = Options { shading: Shading::Flat, crease: 40, ..Default::default() };
        assert!(shade(&s, &flat).at.is_empty());
    }

    /// A grid of coplanar quads of one luma: nothing varies, nothing lit.
    /// The same grid with one quad of another luma (a sign on a wall):
    /// not merged (material), so still nothing lit.
    #[test]
    fn planar_regions_stay_flat() {
        let grid = |odd: Option<(usize, u8)>| {
            let mut v = Vec::new();
            for j in 0..4 {
                for i in 0..4 {
                    let (x, y) = (i as f32 * 0.5 - 1.0, j as f32 * 0.5 - 1.0);
                    let q = [[x, y, 10.0], [x + 0.5, y, 10.0], [x + 0.5, y + 0.5, 10.0], [x, y + 0.5, 10.0]];
                    let luma = match odd {
                        Some((k, l)) if k == j * 4 + i => l,
                        _ => 120,
                    };
                    v.push(tpoly(&q.map(sp), luma));
                }
            }
            v
        };
        let (s, _) = stream(&[grid(None)]);
        let sh = shade(&s, &opts(40));
        assert!(sh.at.is_empty());
        assert_eq!(sh.stats.flat, 16);
        assert!(sh.stats.edges > 0);
        let (s, _) = stream(&[grid(Some((5, 200)))]);
        let sh = shade(&s, &opts(40));
        assert!(sh.at.is_empty());
        assert_eq!(sh.stats.material_splits, 4);
        // Within the material slack: averaged (a little shading).
        let (s, _) = stream(&[grid(Some((5, 124)))]);
        let sh = shade(&s, &opts(40));
        assert!(!sh.at.is_empty());
        for c in &sh.corners {
            assert!(c.iter().all(|v| (120.0..=125.0).contains(&v[0])), "{c:?}");
        }
    }

    /// Link-0 polygons, culled back faces (2C sends luma 0 for them),
    /// degenerate ones and HUD windows take no part and get nothing.
    #[test]
    fn exclusions() {
        let mut cyl = cylinder([0.0, -0.5, 12.0], 1.0, 1.0, 16);
        // Sides facing the eye are back faces (n·P < 0): single-sided and
        // luma 0, as the 2C programs send them.
        for p in cyl.iter_mut() {
            p.double_sided = false;
            if p.back {
                p.luma = 0;
            }
        }
        // A degenerate one: all corners on a line.
        cyl.push(tpoly(&[[0.0, 0.0, 9.0], [1.0, 0.0, 9.0], [2.0, 0.0, 9.0], [3.0, 0.0, 9.0]].map(sp), 50));
        let (s, ats) = stream(&[cyl.clone()]);
        let sh = shade(&s, &opts(40));
        assert_eq!(sh.stats.degenerate, 1);
        assert_eq!(sh.stats.culled as usize, cyl.iter().filter(|p| p.back).count());
        // Hidden: one link-0 polygon before each drawn one.
        assert_eq!(sh.stats.hidden as usize, cyl.len());
        for (p, at) in cyl.iter().zip(&ats[0]) {
            if let Some(c) = sh.get(*at) {
                assert!(!p.back);
                // Luma 0 of the culled faces is never averaged in.
                assert!(c.iter().all(|v| v[0] >= lambert_min()), "{c:?}");
            }
        }
        assert!(!sh.at.is_empty());
        // The same in a HUD window: nothing.
        let o = Options { skip_windows: 1, ..opts(40) };
        let sh = shade(&s, &o);
        assert!(sh.at.is_empty());
        assert_eq!(sh.stats.excluded as usize, cyl.len() - sh.stats.culled as usize);
    }

    fn lambert_min() -> f32 {
        28.0
    }

    /// A mirrored model (x negated: the winding flips) shades the same.
    #[test]
    fn mirrored_blocks_give_the_same_lumas() {
        let cyl = cylinder([0.3, -0.5, 12.0], 1.0, 1.0, 12);
        let mirror: Vec<TPoly> = cyl
            .iter()
            .map(|p| TPoly { corners: p.corners.iter().map(|c| [-c[0], c[1], c[2]]).collect(), ..p.clone() })
            .collect();
        let (a, _) = stream(&[cyl]);
        let (b, _) = stream(&[mirror]);
        let (sa, sb) = (shade(&a, &opts(40)), shade(&b, &opts(40)));
        assert!(!sa.at.is_empty());
        assert_eq!(sa.at, sb.at);
        for (x, y) in sa.corners.iter().zip(&sb.corners) {
            for k in 0..4 {
                assert!((x[k][0] - y[k][0]).abs() < 1e-3, "{x:?} {y:?}");
            }
        }
    }

    /// Two blocks with the same geometry are never welded to each other:
    /// each is shaded on its own (here each alone is flat).
    #[test]
    fn blocks_are_not_welded_together() {
        let a = vec![tpoly(&[[0.0, 0.0, 10.0], [1.0, 0.0, 10.0], [1.0, 1.0, 10.5], [0.0, 1.0, 10.5]].map(sp), 100)];
        let b = vec![tpoly(&[[0.0, 1.0, 10.5], [1.0, 1.0, 10.5], [1.0, 2.0, 10.6], [0.0, 2.0, 10.6]].map(sp), 140)];
        let (s, _) = stream(&[a.clone(), b.clone()]);
        assert!(shade(&s, &opts(40)).at.is_empty());
        // In one block they share an edge and smooth.
        let (s, _) = stream(&[[a, b].concat()]);
        assert_eq!(shade(&s, &opts(40)).at.len(), 2);
    }

    /// A duplicate of a surface's quad (glass, a decal) gets the surface's
    /// corner values and doesn't double its weight.
    #[test]
    fn duplicates_count_once() {
        let mut cyl = cylinder([0.0, -0.5, 12.0], 1.0, 1.0, 16);
        let k = cyl.iter().position(|p| !p.back).unwrap() + 1;
        let mut glass = cyl[k].clone();
        // Sent again from the next corner, as sgt24h's glass.
        glass.corners.rotate_left(1);
        cyl.push(glass);
        let (s, ats) = stream(&[cyl.clone()]);
        let sh = shade(&s, &opts(40));
        let (body, glass) = (sh.get(ats[0][k]).unwrap(), sh.get(*ats[0].last().unwrap()).unwrap());
        let mut a: Vec<u32> = body.iter().map(|v| v[0].to_bits()).collect();
        let mut b: Vec<u32> = glass.iter().map(|v| v[0].to_bits()).collect();
        a.sort();
        b.sort();
        assert_eq!(a, b);
        // A darker copy (a decal of its own luma) keeps its difference.
        let mut dark = cyl.clone();
        dark.last_mut().unwrap().luma -= 30;
        let (s3, ats3) = stream(&[dark]);
        let sh3 = shade(&s3, &opts(40));
        let mut d: Vec<f32> = sh3.get(*ats3[0].last().unwrap()).unwrap().iter().map(|v| v[0] + 30.0).collect();
        let mut b: Vec<f32> = sh3.get(ats3[0][k]).unwrap().iter().map(|v| v[0]).collect();
        d.sort_by(f32::total_cmp);
        b.sort_by(f32::total_cmp);
        assert_eq!(d, b);
        // And without the duplicate, the body's values are the same.
        cyl.pop();
        let (s2, ats2) = stream(&[cyl]);
        assert_eq!(shade(&s2, &opts(40)).get(ats2[0][k]).unwrap(), body);
    }

    /// Welding: two ring vertices of a block get one id exactly when their
    /// stream words are equal (-0 as 0), over strips of every link type,
    /// triangles and repeated positions (a brute-force weld).
    #[test]
    fn welding_matches_a_brute_force_weld() {
        let mut seed = 0x9e37_79b9u32;
        let mut rnd = |n: u32| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed % n
        };
        for _ in 0..50 {
            // A pool of a few positions (so many repeat), one with -0.
            let pool: Vec<[f32; 3]> =
                (0..6).map(|k| [k as f32 * 3.0, if k == 2 { -0.0 } else { (k % 3) as f32 }, 5.0 + k as f32]).collect();
            let mut s = vec![0xff00_0003, 0, (495 << 12) | 383, 0, 0, 0, 0];
            let put = |s: &mut Vec<u32>, p: [f32; 3]| s.extend(p.map(|v| v.to_bits() >> 8));
            let mut words = Vec::new();
            for _ in 0..2 {
                s.extend([0xff00_0001, 0, 0]);
                let block: Vec<[f32; 3]> = (0..30).map(|_| pool[rnd(6) as usize]).collect();
                put(&mut s, block[0]);
                put(&mut s, block[1]);
                let mut k = 2;
                while k + 1 < block.len() {
                    let quad = rnd(2) == 1;
                    s.extend([quad as u32 | 2 | rnd(4) << 8, 100 << 15, 0]);
                    put(&mut s, block[k]);
                    if quad {
                        put(&mut s, block[k + 1]);
                    }
                    k += 2;
                }
                words.push(block);
            }
            s.push(0xff00_000f);
            let mesh = mesh_from_stream(&s);
            assert_eq!(mesh.blocks.len(), 2);
            for b in &mesh.blocks {
                let ids: Vec<u32> =
                    mesh.polys[b.polys.clone()].iter().flat_map(|p| p.ring[..p.corners as usize].to_vec()).collect();
                for &x in &ids {
                    for &y in &ids {
                        let key = |v: u32| mesh.verts[v as usize].map(|f| if f == 0.0 { 0 } else { f.to_bits() >> 8 });
                        assert_eq!(x == y, key(x) == key(y));
                        assert!(b.verts.contains(&(x as usize)));
                    }
                }
            }
        }
    }

    #[test]
    fn options_from_rules() {
        let rules = romset::Shading { crease: 33, skip_windows: &[2], ..romset::Shading::DEFAULT };
        let o = Options::new(Shading::Vertex, CREASE_AUTO, &rules, &[1]);
        assert_eq!((o.crease, o.skip_windows), (33, 0b110));
        assert!(!Options::new(Shading::Vertex, 0, &rules, &[]).on());
        assert_eq!(Options::new(Shading::Pixel, 200, &rules, &[]).crease, 180);
        assert_eq!(parse_crease("auto"), Some(CREASE_AUTO));
        assert_eq!(parse_crease("181"), None);
        assert_eq!(Shading::parse("off"), Some(Shading::Flat));
        assert_eq!(quantise(f32::NAN), 0);
        assert_eq!(quantise(300.0), 255);
        assert_eq!(quantise(17.99), 17);
    }
}
