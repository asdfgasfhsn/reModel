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
//! A side channel ([`Frame`], `side.rs`) makes it better where it exists:
//! the native geometrizers record per block the true focal lengths, the
//! model address and whether it is direct data (level L1), and per polygon
//! the normal and lighting entry the game lit it with (L2); on the chip
//! path a scan of the display list gives the block facts (`scan.rs`).
//! [`shade_with`] checks it against the stream first ([`check_mesh`]) and
//! uses what passes: L1 the focal lengths (no guess), direct data and the
//! game's model rules; L2 also the game's normals for the crease test and
//! its lighting entries as the material boundaries. The game's formula per
//! vertex and per pixel (the design's stage 4) builds on the same data.

mod scan;
mod side;

pub use scan::{scan_list, ScanBoard, ScanProgram, ScanState, ScanVariant};
pub use side::{
    check_face, check_mesh, self_check, Base, Block, Cause, CheckStats, Checked, Face, Facts, Frame, Lamp, Level,
    Material, Tap, Verdict,
};

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

/// How polygons are shaded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Shading {
    /// One luma per polygon, as the game drew it (the default).
    #[default]
    Flat,
    /// Per-vertex brightness, interpolated across the polygon.
    Vertex,
    /// Per-pixel lighting: the game's formula for the normal interpolated
    /// across the polygon. Needs the exact layer (level L2, the native
    /// geometrizer's side channel); elsewhere drawn as `Vertex`
    /// ([`Stats::effective`]).
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

/// Without the true focal lengths (no side channel: level L0), the guess:
/// `fx = fy = FOCAL_GUESS * window width`. Only the crease test and the
/// angle weights depend on it.
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
    /// Models never shaded, and models with a crease angle of their own
    /// (`romset::Shading::skip_models`, `model_crease`): model address
    /// ranges, known from level L1 up.
    pub skip_models: &'static [(u32, u32)],
    pub model_crease: &'static [(u32, u32, u8)],
}

impl Default for Options {
    fn default() -> Self {
        Options {
            shading: Shading::Flat,
            crease: 0,
            skip_windows: 0,
            skip_checker: false,
            diagnose: false,
            skip_models: &[],
            model_crease: &[],
        }
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
            skip_models: rules.skip_models,
            model_crease: rules.model_crease,
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
    /// Focal lengths (x, y): positions are view space times these (a guess,
    /// [`FOCAL_GUESS`], unless a side channel gives them).
    pub focal: (f32, f32),
    /// Which `0x1` command of the stream it is, counted from 0 (blocks
    /// before the first window are counted but not kept).
    pub cmd: u32,
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
    /// The stream's `0x1` commands (object blocks), kept or not.
    pub commands: u32,
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
                mesh.commands += 1;
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
                    cmd: mesh.commands - 1,
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
    /// In a window that is never shaded (HUD), or a model the game's rules
    /// exclude.
    Excluded,
    /// Direct data (level L1 and up): polygons given with their luma.
    Direct,
    /// No usable normal by the side channel (level L2): a zero normal, the
    /// models' unlit marker.
    Unlit,
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
    /// Its block's level.
    pub level: Level,
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
    pub direct: u32,
    pub unlit: u32,
    /// Shared edges (between two faces taking part), and of them kept hard
    /// by the crease angle and by the material rule.
    pub edges: u32,
    pub crease_edges: u32,
    pub material_splits: u32,
    /// Of the lit polygons: lit with the game's formula per corner (the
    /// exact layer, level L2), and of those per pixel ([`Shading::Pixel`]).
    pub exact: u32,
    pub pixel: u32,
    /// Time [`shade`] took, ms.
    pub ms: f32,
    /// The side channel's self-check, and the blocks by level
    /// ([`check_mesh`]; without a side channel every block is L0).
    pub check: CheckStats,
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
        self.direct += o.direct;
        self.unlit += o.unlit;
        self.edges += o.edges;
        self.crease_edges += o.crease_edges;
        self.material_splits += o.material_splits;
        self.exact += o.exact;
        self.pixel += o.pixel;
        self.ms += o.ms;
        self.check.add(&o.check);
    }

    /// What a frame shaded at `requested` was drawn as, for a status line
    /// or menu: `Pixel` only where some polygon was lit per pixel (the
    /// exact layer exists: the native geometrizer's side channel); without
    /// it (the chip, old captures, every block demoted) `Pixel` is drawn as
    /// `Vertex`. A frame with nothing lit says what was asked.
    pub fn effective(&self, requested: Shading) -> Shading {
        match requested {
            Shading::Pixel if self.pixel == 0 && self.lit > 0 => Shading::Vertex,
            s => s,
        }
    }

    /// The highest level a block of the frame was shaded at (L2: the
    /// game's formula; L1: block facts; L0: the stream alone).
    pub fn level(&self) -> Level {
        if self.check.l2 > 0 {
            Level::L2
        } else if self.check.l1 > 0 {
            Level::L1
        } else {
            Level::L0
        }
    }
}

/// [`shade`]'s result: the corner values of the polygons drawn lit.
#[derive(Clone, Debug, Default)]
pub struct Shaded {
    /// Stream index of the attribute word of each lit polygon, ascending.
    pub at: Vec<u32>,
    /// Per lit polygon, in ring order (P1', P0', Pa, Pb): `[luma, nx, ny,
    /// nz]`, luma as a float 0-255 before truncation; the normal (the
    /// exact layer's vertex normal, in the face's own convention) is zero
    /// where there is none (levels L0 and L1).
    pub corners: Vec<[[f32; 4]; 4]>,
    /// Per lit polygon: its lighting for per-pixel evaluation
    /// ([`Shading::Pixel`] at level L2); None: drawn from the corner lumas.
    pub pixel: Vec<Option<PixelLight>>,
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

    /// The per-pixel lighting of the polygon at stream index `at`, if it
    /// is lit per pixel.
    pub fn pixel_light(&self, at: u32) -> Option<&PixelLight> {
        self.at.binary_search(&at).ok().and_then(|k| self.pixel.get(k)?.as_ref())
    }
}

/// Smooth-shade a frame's raster stream: per polygon drawn lit, its corner
/// values. Empty when `opts` shades nothing ([`Options::on`]). From the
/// stream alone (level L0): [`shade_with`] without a side channel.
pub fn shade(stream: &[u32], opts: &Options) -> Shaded {
    shade_with(stream, None, opts)
}

/// [`shade`] with the frame's side channel, if there is one: it is checked
/// against the stream ([`check_mesh`]) and each block shaded at the level
/// it passes (counted in [`Stats::check`]).
pub fn shade_with(stream: &[u32], side: Option<&Frame>, opts: &Options) -> Shaded {
    if !opts.on() {
        return Shaded::default();
    }
    let t = crate::geo::Stopwatch::start();
    let mesh = mesh_from_stream(stream);
    let mut s = match side {
        Some(fr) => {
            let checked = check_mesh(&mesh, fr);
            let sides: Vec<Option<BlockSide>> = checked
                .blocks
                .iter()
                .map(|&(level, si)| {
                    si.map(|si| {
                        let block = fr.blocks[si];
                        let faces = if level == Level::L2 { fr.faces_of(&block) } else { &[] };
                        BlockSide { level, block, faces, materials: &fr.materials }
                    })
                })
                .collect();
            let mut s = shade_mesh_with(&mesh, Some(&sides), opts);
            s.stats.check = checked.stats;
            s
        }
        None => {
            let mut s = shade_mesh(&mesh, opts);
            s.stats.check.l0 = mesh.blocks.len() as u32;
            s
        }
    };
    s.stats.ms = t.elapsed().as_secs_f32() * 1e3;
    s
}

/// One mesh block's side channel as [`shade_mesh_with`] takes it.
#[derive(Clone, Copy, Debug)]
pub struct BlockSide<'a> {
    pub level: Level,
    pub block: Block,
    /// Its faces, one per polygon record of the block (level L2 only;
    /// empty otherwise).
    pub faces: &'a [Face],
    /// The frame's materials ([`Frame::materials`]; level L2).
    pub materials: &'a [Material],
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
struct Part {
    poly: usize,
    n: V3,
    /// Its first corner in the block's corner arrays.
    corner: u32,
    /// Level L2: its material (lighting entry's values) and the program's
    /// facing; else `u32::MAX`, and the stream's.
    mat: u32,
    front: bool,
    /// Level L2: the normal as the program lit with it (`n` is it, or it
    /// negated to the geometric side: `flip`), and its lighting.
    raw: V3,
    flip: bool,
    light: Option<PixelLight>,
}

/// One polygon's lighting as the game computes it, for a normal other than
/// its own (the exact layer, level L2): [`Lamp::eval`] plus an offset that
/// makes the polygon's own normal give its own luma (the stream's) exactly.
/// The offset is 0 where the formula reproduces the stream's luma (every
/// 2C and 2A polygon); a 2B polygon whose recomputation is a level off
/// (the SHARC rounds toward zero) gets `F + 0.5 - eval(n)`, so that no
/// polygon brightens or darkens by a level just for being smoothed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PixelLight {
    pub lamp: Lamp,
    pub offset: f32,
}

impl PixelLight {
    /// The luma (before truncation) for unit normal `n`, in the face's own
    /// convention (as [`Face::n`]).
    pub fn luma(&self, n: [f32; 3]) -> f32 {
        self.lamp.eval(n) + self.offset
    }

    /// A polygon's lighting from its face, material and the stream's luma
    /// `luma` (0-255).
    pub fn of(block: &Block, face: &Face, m: &Material, luma: u8) -> PixelLight {
        let lamp = Lamp::of(block, face, m);
        let v = lamp.eval(face.n);
        let offset = if quantise(v) == luma as u32 || !v.is_finite() { 0.0 } else { luma as f32 + 0.5 - v };
        PixelLight { lamp, offset }
    }

    /// Whether the luma depends on the normal at all (a material with no
    /// diffuse and no specular term is its ambient everywhere).
    pub fn varies(&self) -> bool {
        self.lamp.diffuse != 0.0 || (self.lamp.squarings != Material::NO_SPEC && self.lamp.spec_scale != 0.0)
    }
}

/// A corner value of the exact layer: the hardware clamps a lighting
/// result into 0-255 before it truncates, so a corner is clamped (to just
/// under 256, which still truncates to 255) before it is interpolated.
pub fn clamp_corner(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(0.0, 255.996)
    }
}

/// Vertex normals within this of the face's own (dot product above 1 -
/// this, about 0.08 degrees) are taken as the face's: float noise between
/// coplanar faces' normals never lights a polygon.
pub const NORMAL_EPS: f32 = 1e-6;

/// [`shade`] on a mesh (no timing), at level L0.
pub fn shade_mesh(mesh: &Mesh, opts: &Options) -> Shaded {
    shade_mesh_with(mesh, None, opts)
}

/// [`shade_mesh`] with a side channel per block (`sides[k]` for
/// `mesh.blocks[k]`; None: L0).
pub fn shade_mesh_with(mesh: &Mesh, sides: Option<&[Option<BlockSide>]>, opts: &Options) -> Shaded {
    let mut out = Shaded::default();
    if !opts.on() {
        return out;
    }
    let crease_cos = (opts.crease as f32).to_radians().cos();
    // Reused per block.
    let mut faces: Vec<Part> = Vec::new();
    let mut face_of: Vec<u32> = Vec::new();
    let mut edges: Vec<(u32, u32, u32, u8)> = Vec::new();
    let mut parent: Vec<u32> = Vec::new();
    let mut weight: Vec<f32> = Vec::new();
    let mut members: Vec<(u32, u32)> = Vec::new();
    let mut hard: Vec<(u8, u8)> = Vec::new();
    let mut group_value: Vec<Option<f32>> = Vec::new();
    let mut group_normal: Vec<Option<V3>> = Vec::new();
    for (bi, b) in mesh.blocks.iter().enumerate() {
        let side = sides.and_then(|s| s.get(bi).copied().flatten()).filter(|s| s.level != Level::L0);
        let level = side.map_or(Level::L0, |s| s.level);
        let focal = side.and_then(|s| s.block.focal()).unwrap_or(b.focal);
        let (fx, fy) = (
            if focal.0.is_finite() && focal.0 != 0.0 { focal.0 } else { 1.0 },
            if focal.1.is_finite() && focal.1 != 0.0 { focal.1 } else { 1.0 },
        );
        let direct = side.is_some_and(|s| s.block.flags & Block::DIRECT != 0);
        let oba = side.map(|s| s.block.oba);
        let in_range = |lo: u32, hi: u32| oba.is_some_and(|a| (lo..=hi).contains(&a));
        let skip_model = opts.skip_models.iter().any(|&(lo, hi)| in_range(lo, hi));
        let c = match opts.model_crease.iter().find(|&&(lo, hi, _)| in_range(lo, hi)) {
            Some(&(_, _, deg)) => (deg.min(180) as f32).to_radians().cos(),
            None => crease_cos,
        };
        let game_faces = side.filter(|s| s.level == Level::L2 && s.faces.len() == b.polys.len()).map(|s| s.faces);
        let pos = |v: u32| {
            let p = mesh.verts[v as usize];
            [p[0] / fx, p[1] / fy, p[2]]
        };
        let skipped = (b.window < 64 && opts.skip_windows & (1 << b.window) != 0) || skip_model;
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
            } else if direct {
                Class::Direct
            } else if game_faces.is_some_and(|gf| gf[k].flags & Face::NONE != 0) {
                Class::Unlit
            } else {
                match face_normal(p, &pos) {
                    Some(g) => {
                        face_of[k] = faces.len() as u32;
                        // Level L2: the normal the game lit with (on the
                        // same side as the geometric one), its material
                        // and facing.
                        let (n, mat, front, raw, flip, light) = match (game_faces, side) {
                            (Some(gf), Some(sd)) => {
                                let f = &gf[k];
                                let len = dot(f.n, f.n).sqrt();
                                let mut n = f.n.map(|x| x / len);
                                let flip = dot(n, g) < 0.0;
                                if flip {
                                    n = n.map(|x| -x);
                                }
                                let light =
                                    sd.materials.get(f.mat as usize).map(|m| PixelLight::of(&sd.block, f, m, p.luma));
                                (n, f.mat as u32, f.front(), f.n, flip, light)
                            }
                            _ => (g, u32::MAX, !p.back, g, false, None),
                        };
                        faces.push(Part { poly: pi, n, corner: 0, mat, front, raw, flip, light });
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
                    // One material: at L2 the same lighting entry values
                    // and facing; else the same facing and lumas no
                    // further apart than a light could make them.
                    let other = if game_faces.is_some() {
                        faces[fa].mat != faces[fb].mat || faces[fa].front != faces[fb].front
                    } else {
                        let d = sub(na, nb);
                        let slack = MATERIAL_SLACK + MATERIAL_SLOPE * 255.0 * dot(d, d).sqrt();
                        pa.back != pb.back || (pa.luma as f32 - pb.luma as f32).abs() > slack
                    };
                    if other {
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
        // Level L2, the exact layer: each corner lit with the game's
        // formula for its group's normal.
        let exact = game_faces.is_some();
        if exact {
            exact_corners(
                mesh,
                &faces,
                &members,
                &mut parent,
                &weight,
                &mut group_normal,
                opts,
                &mut out,
                &mut classes,
                b,
                &pos,
            );
        }
        // Per root: the mean (or None where nothing varies); per (root,
        // face) of a duplicate face, its offset from that mean.
        group_value.clear();
        group_value.resize(nc as usize, None);
        let mut offsets: HashMap<(u32, u32), f32> = HashMap::new();
        let mut g = 0;
        while !exact && g < members.len() {
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
        for (fi, f) in faces.iter().enumerate().filter(|_| !exact) {
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
                out.pixel.push(None);
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
                Class::Direct => out.stats.direct += 1,
                Class::Unlit => out.stats.unlit += 1,
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
                    level,
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
        out.pixel = order.iter().map(|&k| out.pixel[k]).collect();
    }
    out
}

/// Faces of one surface sent again over the same vertices (a decal, glass)
/// count once in a group: whether face `f` of `group` (members (root,
/// face)) repeats an earlier member's vertex set.
fn repeats(mesh: &Mesh, faces: &[Part], group: &[(u32, u32)], k: usize) -> bool {
    let ids = |f: u32| {
        let p = &mesh.polys[faces[f as usize].poly];
        let mut ids = p.ring;
        ids[..p.corners as usize].sort_unstable();
        (p.corners, ids)
    };
    let me = ids(group[k].1);
    group[..k].iter().any(|&(_, o)| ids(o) == me)
}

/// The exact layer (level L2) for one block: per group of corners (as
/// [`shade_mesh_with`] built them), the angle-weighted mean of the faces'
/// game normals, normalised; each face's corner lit with its own lighting
/// entry ([`PixelLight`]) for that normal. A polygon is lit only where a
/// corner's truncated luma differs from its own (`Vertex`), or a corner's
/// normal from its own (`Pixel`, which then lights it per pixel): a
/// polygon whose corners all keep the face's normal is drawn by the
/// integer path, whatever the float arithmetic would give.
#[allow(clippy::too_many_arguments)]
fn exact_corners(
    mesh: &Mesh,
    faces: &[Part],
    members: &[(u32, u32)],
    parent: &mut [u32],
    weight: &[f32],
    group_normal: &mut Vec<Option<V3>>,
    opts: &Options,
    out: &mut Shaded,
    classes: &mut [Class],
    b: &MeshBlock,
    pos: &dyn Fn(u32) -> V3,
) {
    group_normal.clear();
    group_normal.resize(parent.len(), None);
    let bits = |n: V3| n.map(f32::to_bits);
    let mut g = 0;
    while g < members.len() {
        let root = members[g].0;
        let mut e = g + 1;
        while e < members.len() && members[e].0 == root {
            e += 1;
        }
        let group = &members[g..e];
        let first = bits(faces[group[0].1 as usize].n);
        if group.iter().any(|&(_, f)| bits(faces[f as usize].n) != first) {
            let mut s = [0.0f64; 3];
            for (k, &(_, f)) in group.iter().enumerate() {
                if repeats(mesh, faces, group, k) {
                    continue;
                }
                let face = &faces[f as usize];
                let p = &mesh.polys[face.poly];
                let w: f32 = (0..p.corners as u32)
                    .filter(|&j| find(parent, face.corner + j) == root)
                    .map(|j| weight[(face.corner + j) as usize])
                    .sum();
                // No angle anywhere (degenerate corners): equal weights.
                let w = if w > 0.0 { w as f64 } else { 1e-6 };
                for (c, n) in s.iter_mut().zip(face.n) {
                    *c += w * n as f64;
                }
            }
            let len = (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt();
            if len.is_finite() && len > 1e-12 {
                group_normal[root as usize] = Some(s.map(|x| (x / len) as f32));
            }
        }
        g = e;
    }
    let pixel = opts.shading == Shading::Pixel;
    // Each face's corners (value, normal), whether a corner's normal
    // moved, its own value and its mean as drawn (estimated).
    struct Lit {
        corners: [[f32; 4]; 4],
        moved: bool,
        own: f32,
        mean: f32,
    }
    let mut lits: Vec<Option<Lit>> = Vec::with_capacity(faces.len());
    for f in faces {
        let Some(light) = f.light else {
            lits.push(None);
            continue;
        };
        let p = &mesh.polys[f.poly];
        let n = p.corners as usize;
        let mut corners = [[0.0f32; 4]; 4];
        let mut moved = false;
        let mut sum = [0.0f32; 3];
        for (k, c) in corners.iter_mut().enumerate().take(n) {
            let root = find(parent, f.corner + k as u32);
            let nv = match group_normal[root as usize] {
                Some(v) if dot(v, f.n) < 1.0 - NORMAL_EPS => {
                    moved = true;
                    if f.flip {
                        v.map(|x| -x)
                    } else {
                        v
                    }
                }
                // The face's own normal, bit for bit as the game lit it.
                _ => f.raw,
            };
            *c = [clamp_corner(light.luma(nv)), nv[0], nv[1], nv[2]];
            for i in 0..3 {
                sum[i] += nv[i];
            }
        }
        if n == 3 {
            corners[3] = corners[2];
        }
        let own = clamp_corner(light.luma(f.raw));
        let corner_mean = corners[..n].iter().map(|c| c[0]).sum::<f32>() / n as f32;
        // Per pixel the middle of the polygon has the corners' mean
        // normal, not their mean value: half of each.
        let len = dot(sum, sum).sqrt();
        let mean = if pixel && moved && len > 1e-6 {
            0.5 * (corner_mean + clamp_corner(light.luma(sum.map(|x| x / len))))
        } else {
            corner_mean
        };
        lits.push(Some(Lit { corners, moved, own, mean }));
    }
    // Each smooth surface (faces joined through corner groups: one
    // material, no crease between) keeps its overall brightness: the
    // normalised mean normals of faces whose normals scatter around the
    // light are brighter than the faces themselves (the corners of a bumpy
    // or noisy surface face the light more squarely than its facets), so
    // the surface's area-weighted mean as drawn is moved back to its mean
    // as the game lit it, by one offset for the whole surface (no seams,
    // and the shape within it, highlights included, kept).
    let mut region: Vec<u32> = (0..faces.len() as u32).collect();
    let mut g = 0;
    while g < members.len() {
        let mut e = g + 1;
        while e < members.len() && members[e].0 == members[g].0 {
            e += 1;
        }
        for m in &members[g + 1..e] {
            union(&mut region, members[g].1, m.1);
        }
        g = e;
    }
    let mut acc: Vec<(f64, f64, bool)> = vec![(0.0, 0.0, false); faces.len()];
    for (fi, (f, l)) in faces.iter().zip(&lits).enumerate() {
        let Some(l) = l else { continue };
        let p = &mesh.polys[f.poly];
        let ring: Vec<V3> = p.ring[..p.corners as usize].iter().map(|&v| pos(v)).collect();
        let mut area = [0.0f32; 3];
        for k in 1..ring.len().saturating_sub(1) {
            let c = cross(sub(ring[k], ring[0]), sub(ring[k + 1], ring[0]));
            for i in 0..3 {
                area[i] += c[i];
            }
        }
        let a = dot(area, area).sqrt() as f64;
        let a = if a.is_finite() { a } else { 0.0 };
        let r = &mut acc[find(&mut region, fi as u32) as usize];
        r.0 += a * (l.own - l.mean) as f64;
        r.1 += a;
        r.2 |= l.moved;
    }
    for (fi, (f, l)) in faces.iter().zip(&lits).enumerate() {
        let (Some(l), Some(light)) = (l, f.light) else { continue };
        let p = &mesh.polys[f.poly];
        let r = acc[find(&mut region, fi as u32) as usize];
        let shift = if r.2 && r.1 > 0.0 { (r.0 / r.1) as f32 } else { 0.0 };
        let mut corners = l.corners;
        let mut differs = false;
        for c in corners.iter_mut() {
            c[0] = clamp_corner(c[0] + shift);
            differs |= quantise(c[0]) != p.luma as u32;
        }
        let light = PixelLight { offset: light.offset + shift, ..light };
        let lit = if pixel { (l.moved && light.varies()) || differs } else { differs };
        if lit {
            out.at.push(p.at);
            out.corners.push(corners);
            out.pixel.push(pixel.then_some(light));
            out.stats.exact += 1;
            out.stats.pixel += pixel as u32;
            classes[f.poly - b.polys.start] = Class::Lit;
        }
    }
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

/// A display list and polygon ROM for the native geometrizers' light hooks
/// and the list scan (tests in `geo_native`, `geo_native_2c` and here):
/// a window, the transform, focal lengths, a light, four lighting entries
/// (one with specular), then an object with given normals (random, some
/// zero, some double-sided), a new light vector and entry, the same model
/// with normals computed (`mode`'s bit 1 set) and specular on, and a block
/// of direct data.
#[cfg(test)]
pub(crate) mod test_scene {
    pub fn f(v: f32) -> u32 {
        v.to_bits()
    }

    pub fn cmd(fnc: u32) -> u32 {
        fnc << 23 | fnc << 8 | fnc
    }

    /// The model at polygon ROM `0x800000`: P0, P1 and `n` polygons.
    pub fn rom(n: usize) -> Vec<u32> {
        let mut seed = 0x1234_5678u32;
        let mut rnd = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0
        };
        let mut rom = vec![f(0.0), f(0.0), f(0.0), f(1.0), f(0.0), f(0.0)];
        for k in 0..n {
            let quad = k % 3 != 2;
            let link = [2u32, 1, 3, 2][k % 4];
            let attr = (if quad { 1 } else { 2 })
                | link << 8
                | ((k % 5 == 0) as u32) << 17
                | ((k % 4) as u32) << 18
                | ((k % 2) as u32) << 27;
            let mut nv = [rnd(), rnd(), rnd()];
            let len = (nv[0] * nv[0] + nv[1] * nv[1] + nv[2] * nv[2]).sqrt();
            nv = nv.map(|x| x / len);
            if k % 7 == 3 {
                nv = [0.0; 3];
            }
            rom.push(attr);
            rom.extend(nv.map(f));
            for _ in 0..2 {
                rom.extend([f(rnd()), f(rnd()), f(rnd() * 0.5)]);
            }
        }
        rom.push(0);
        rom
    }

    /// The list (with `mode` for the second object: bit 1 computed normals,
    /// bit 0 specular).
    pub fn list(mode: u32) -> Vec<u32> {
        let mut l = vec![cmd(0x03)];
        l.extend([0, 495 << 16 | 383, 248 << 16 | 192, 248 << 16 | 192, 248 << 16 | 192, 248 << 16 | 192]);
        l.push(cmd(0x0b));
        l.extend([0.9, 0.1, 0.0, -0.1, 0.9, 0.2, 0.0, -0.2, 0.95, 0.1, -0.2, 10.0].map(f));
        l.extend([cmd(0x09), f(500.0), f(480.0), cmd(0x0a), f(0.3), f(-0.5), f(0.81), cmd(0x16), f(1.0)]);
        // Entries 0-3: diffuse, ambient, specular scale, control.
        l.extend([cmd(0x06), 0, 4]);
        for (d, a, s, c) in [(100u32, 20u32, 0u32, 0u32), (180, 40, 60, 2), (0, 90, 0, 0), (200, 10, 120, 5)] {
            l.extend([d | a << 8 | s << 16 | c << 24, f(0.5)]);
        }
        l.extend([cmd(0x07), 0, cmd(0x01), 0x12_3456, 0x0065_4321, 0x80_0000, 0]);
        // A new light and entry 2's values, then the model again.
        l.extend([cmd(0x0a), f(-0.6), f(0.2), f(0.77), cmd(0x06), 2, 1, 150 | 30 << 8 | 80 << 16 | 1 << 24, f(0.25)]);
        l.extend([cmd(0x07), mode, cmd(0x01), 0x12_3456, 0x0065_4321, 0x80_0000, 12]);
        // Direct data: P0, P1, a triangle and a quad.
        l.extend([cmd(0x02), 0x11, 0x22]);
        l.extend([1.0, 2.0, 12.0, 3.0, 2.0, 12.0].map(f));
        l.extend([2 | 2 << 8, 99 << 23, f(0.0), f(1.0), f(3.0), f(12.0)]);
        l.extend([1 | 2 << 8, 50 << 23, f(0.0), f(2.0), f(4.0), f(12.0), f(5.0), f(4.0), f(12.0)]);
        l.extend([0, cmd(0x0f)]);
        l
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

    /// Side-channel words: written and read back; anything malformed is
    /// refused.
    #[test]
    fn side_channel_words() {
        let mut tap = Tap::new();
        tap.block(0x80_1000, Block::BACK_UNLIT, 500f32.to_bits(), 480f32.to_bits(), [1, 2, 3]);
        let m = [20f32.to_bits(), 100f32.to_bits(), 0];
        tap.face([0, 0, 1f32.to_bits()], false, true, Base::D, 3 << 18, m, Material::NO_SPEC);
        // Raw: normalised here. The same material again: shared.
        tap.face([3f32.to_bits(), 0, 4f32.to_bits()], true, false, Base::S0, 3 << 18, m, Material::NO_SPEC);
        tap.face([0, 0x8000_0000, 0], false, true, Base::Dotl, 0, m, 2);
        tap.face([f32::NAN.to_bits(), 0, 0], false, true, Base::D, 0, m, 99);
        tap.block(0, Block::DIRECT, 0, 0, [0; 3]);
        let fr = tap.finish(1 << 40 | 5);
        assert_eq!(fr.blocks[0].flags, Block::BACK_UNLIT | Block::HAS_FACES);
        assert_eq!((fr.blocks[0].first, fr.blocks[0].count, fr.blocks[1].first, fr.blocks[1].count), (0, 4, 4, 0));
        let f = &fr.faces;
        assert_eq!((f[0].mat, f[1].mat, f[2].mat, f[3].mat), (0, 0, 1, 2), "deduplicated with the squarings");
        assert_eq!(f[1].n, [0.6, 0.0, 0.8]);
        assert_eq!(f[1].flags, Face::RAW | Face::BASE_S0 | 3 << Face::ENTRY_SHIFT);
        assert_eq!((f[1].entry(), f[1].front(), f[0].front()), (3, false, true));
        assert_eq!(f[2].flags & (Face::ZERO | Face::NONE), Face::ZERO | Face::NONE, "-0 is zero");
        assert_eq!(f[3].flags & (Face::ZERO | Face::NONE), Face::NONE);
        assert_eq!(fr.materials[1].squarings, 2);
        let w = fr.to_words();
        assert_eq!(w.len(), 6 + 2 * 9 + 4 * 4 + 3 * 4);
        let back = Frame::from_words(&w).unwrap();
        assert_eq!((back.frame, &back.blocks, back.materials.len()), (1 << 40 | 5, &fr.blocks, 3));
        assert!(back.faces.iter().zip(&fr.faces).all(|(a, b)| a.flags == b.flags && a.mat == b.mat));
        assert!(Frame::from_words(&w[..w.len() - 1]).is_none());
        assert!(Frame::from_words(&[]).is_none());
        let mut bad = w.clone();
        bad[6 + 7] = 3; // the first block's faces from 3: past the end
        assert!(Frame::from_words(&bad).is_none());
        let mut bad = w;
        bad[0] ^= 1;
        assert!(Frame::from_words(&bad).is_none());
    }

    /// The formula, against hand-worked values.
    #[test]
    fn lamp() {
        let m = Material { ambient: 20.0, diffuse: 100.0, spec_scale: 50.0, squarings: Material::NO_SPEC };
        let b = Block { light: [0.0, 0.6, 0.8], ..Default::default() };
        let face = |front: bool, base: u16| Face { n: [0.0, 0.0, 1.0], mat: 0, flags: front as u16 | base };
        let n = [0.0, 0.0, 1.0];
        assert_eq!(Lamp::of(&b, &face(true, 0), &m).eval(n), 100.0);
        assert_eq!(Lamp::of(&b, &face(false, 0), &m).eval(n), 20.0, "a back face: -n·L, clamped");
        // Specular from d = 0.8: 2·0.8·1 − 0.8 = 0.8, squared twice.
        let s = Material { squarings: 2, ..m };
        assert!((Lamp::of(&b, &face(true, 0), &s).eval(n) - (100.0 + 0.4096 * 50.0)).abs() < 1e-4);
        // Back face: base D = −0.8 gives none; base n·L gives the front's.
        assert_eq!(Lamp::of(&b, &face(false, 0), &s).eval(n), 20.0);
        assert!((Lamp::of(&b, &face(false, Face::BASE_DOTL), &s).eval(n) - (20.0 + 0.4096 * 50.0)).abs() < 1e-4);
        let none = Material { squarings: 0, ..m };
        assert!((Lamp::of(&b, &face(true, Face::BASE_S0), &none).eval(n) - 140.0).abs() < 1e-4);
    }

    /// A side channel for one of `test_stream`'s blocks: per drawn polygon
    /// its normal, facing (by the stream's back-face bit) and material
    /// (index into `mats`), as a 2B program sends them (a lit back face
    /// carries the back-face bit); the link-0 polygons before each get a zero
    /// normal and material `mats.len()` (all zero: luma 0).
    fn side_block(tap: &mut Tap, focal: f32, polys: &[(TPoly, [f32; 3], usize)], mats: &[Material]) {
        tap.block(0x80_0000, Block::TRUNCATE, focal.to_bits(), focal.to_bits(), L.map(f32::to_bits));
        let bits = |m: &Material| [m.ambient.to_bits(), m.diffuse.to_bits(), m.spec_scale.to_bits()];
        for (p, n, k) in polys {
            tap.face([0; 3], false, true, Base::D, 0, [0; 3], Material::NO_SPEC);
            let m = &mats[*k];
            tap.face(n.map(f32::to_bits), false, !p.back, Base::D, (*k as u32) << 18, bits(m), m.squarings);
        }
    }

    const L: [f32; 3] = [0.3, -0.5, 0.81];

    /// The cylinder with lumas from the formula (material `mat(k)` for side
    /// k), and its normals.
    fn lit_cylinder(focal: f32, mats: &[Material], mat: impl Fn(usize) -> usize) -> Vec<(TPoly, [f32; 3], usize)> {
        cylinder([0.0, -0.5, 12.0], 1.0, 1.0, 16)
            .into_iter()
            .enumerate()
            .map(|(k, mut p)| {
                let mid = (k as f32 + 0.5) / 16.0 * std::f32::consts::TAU;
                let n = [mid.cos(), 0.0, mid.sin()];
                let b = Block { light: L, ..Default::default() };
                let face = Face { n, mat: 0, flags: !p.back as u16 };
                p.luma = quantise(Lamp::of(&b, &face, &mats[mat(k)]).eval(n)) as u8;
                for c in p.corners.iter_mut() {
                    c[0] *= focal / F;
                    c[1] *= focal / F;
                }
                (p, n, mat(k))
            })
            .collect()
    }

    /// Level L1 uses the true focal lengths: a cylinder drawn with a focal
    /// length far from the guess shades as it does at the guess's (L0);
    /// direct data stays flat; a frame whose blocks don't line up is L0.
    #[test]
    fn l1_focal_lengths_and_direct_data() {
        let mats = [Material { ambient: 30.0, diffuse: 180.0, spec_scale: 0.0, squarings: Material::NO_SPEC }];
        let at_guess = lit_cylinder(F, &mats, |_| 0);
        let (s0, _) = stream(&[at_guess.iter().map(|p| p.0.clone()).collect()]);
        let want = shade(&s0, &opts(40));
        assert!(want.at.len() >= 8);
        let far = lit_cylinder(6.0 * F, &mats, |_| 0);
        let (s, _) = stream(&[far.iter().map(|p| p.0.clone()).collect()]);
        // L0 guesses the focal length from the window: the sides look
        // further apart than they are and stay faceted.
        assert!(shade(&s, &opts(40)).at.len() < want.at.len());
        let mut tap = Tap::new();
        tap.block(0x80_0000, Block::BACK_UNLIT, (6.0 * F).to_bits(), (6.0 * F).to_bits(), [0; 3]);
        let side = tap.finish(1);
        let sh = shade_with(&s, Some(&side), &opts(40));
        assert_eq!((sh.stats.check.l1, sh.stats.check.l0), (1, 0));
        assert_eq!(sh.at.len(), want.at.len());
        for (a, b) in sh.corners.iter().zip(&want.corners) {
            for k in 0..4 {
                assert!((a[k][0] - b[k][0]).abs() < 0.05, "{a:?} {b:?}");
            }
        }
        // As direct data: nothing lit.
        let mut tap = Tap::new();
        tap.block(0, Block::DIRECT | Block::BACK_UNLIT, 0, 0, [0; 3]);
        let sh = shade_with(&s, Some(&tap.finish(1)), &opts(40));
        assert!(sh.at.is_empty());
        assert_eq!(sh.stats.direct, 16);
        // Another number of blocks than the stream's: L0, as without.
        let two = Frame { blocks: vec![side.blocks[0]; 2], ..side.clone() };
        let sh = shade_with(&s, Some(&two), &opts(40));
        assert_eq!((sh.stats.check.misaligned_frames, sh.stats.check.l0), (1, 1));
        assert_eq!(sh.at, shade(&s, &opts(40)).at);
        // Model rules by address (known from L1).
        let skip = Options { skip_models: &[(0x80_0000, 0x80_00ff)], ..opts(40) };
        let sh = shade_with(&s, Some(&side), &skip);
        assert!(sh.at.is_empty());
        assert_eq!(sh.stats.excluded, 16);
        let sharp = Options { model_crease: &[(0x80_0000, 0x80_0000, 10)], ..opts(40) };
        assert!(shade_with(&s, Some(&side), &sharp).at.is_empty());
    }

    /// Level L2: faces whose recomputed luma is the stream's are used with
    /// the game's normals and materials (lighting entries) in place of the
    /// luma rule; a zero normal stays flat; a block that fails the check is
    /// L1 and shades as without faces.
    #[test]
    fn l2_materials_and_demotion() {
        let a = Material { ambient: 30.0, diffuse: 180.0, spec_scale: 0.0, squarings: Material::NO_SPEC };
        let b = Material { ambient: 31.0, ..a };
        let mats = [a, b, Material::default()];
        // Alternating materials of nearly equal lumas: one surface at
        // L0/L1 (the luma rule), every edge a material change at L2.
        let polys = lit_cylinder(F, &mats, |k| k % 2);
        let (s, _) = stream(&[polys.iter().map(|p| p.0.clone()).collect()]);
        let l0 = shade(&s, &opts(40));
        assert!(l0.at.len() >= 8);
        let mut tap = Tap::new();
        side_block(&mut tap, F, &polys, &mats);
        let side = tap.finish(1);
        let chk = self_check(&s, &side);
        assert_eq!((chk.stats.l2, chk.stats.mismatch, chk.stats.exact), (1, 0, 32), "{:?}", chk.stats);
        let sh = shade_with(&s, Some(&side), &opts(40));
        assert!(sh.at.is_empty(), "no two neighbours share a material");
        assert!(sh.info.iter().all(|i| i.level == Level::L2));
        // One material: smooth again, with the game's normals.
        let one = lit_cylinder(F, &mats, |_| 0);
        let (s1, _) = stream(&[one.iter().map(|p| p.0.clone()).collect()]);
        let mut tap = Tap::new();
        side_block(&mut tap, F, &one, &mats);
        let side1 = tap.finish(1);
        let sh = shade_with(&s1, Some(&side1), &opts(40));
        assert_eq!(sh.at.len(), shade(&s1, &opts(40)).at.len());
        // A zero normal (the unlit marker) takes no part.
        let mut zero = side1.clone();
        zero.faces[1].n = [0.0; 3];
        zero.faces[1].flags |= Face::ZERO | Face::NONE;
        let s_zero = {
            let mut p = one.clone();
            p[0].0.luma = 30; // ambient alone
            stream(&[p.iter().map(|p| p.0.clone()).collect()]).0
        };
        let sh = shade_with(&s_zero, Some(&zero), &opts(40));
        assert_eq!((sh.stats.check.l2, sh.stats.unlit), (1, 1), "{:?}", sh.stats.check);
        // The wrong light: demoted to L1, shaded as by the stream (with
        // the true focal length, here the guess).
        let mut wrong = side.clone();
        wrong.blocks[0].light = [-0.3, 0.5, -0.81];
        let sh = shade_with(&s, Some(&wrong), &opts(40));
        assert_eq!((sh.stats.check.l1, sh.stats.check.demoted, sh.stats.check.l2), (1, 1, 0));
        assert_eq!(sh.at, l0.at);
        // Faces not lining up with the polygons: L0.
        let mut short = side.clone();
        short.blocks[0].count -= 1;
        short.faces.pop();
        let s = self_check(&s, &short).stats;
        assert_eq!((s.misaligned_blocks, s.l0), (1, 1));
    }

    fn pixel(crease: u8) -> Options {
        Options { shading: Shading::Pixel, ..opts(crease) }
    }

    /// The exact layer (L2): each corner is the game's formula for the
    /// normalised, angle-weighted mean of its group's normals; `Pixel`
    /// keeps the lighting for per-pixel evaluation, `Vertex` doesn't.
    #[test]
    fn exact_corners_are_the_formula_of_the_mean_normal() {
        let m = Material { ambient: 30.0, diffuse: 180.0, spec_scale: 0.0, squarings: Material::NO_SPEC };
        let one = lit_cylinder(F, &[m], |_| 0);
        let (s, ats) = stream(&[one.iter().map(|p| p.0.clone()).collect()]);
        let mut tap = Tap::new();
        side_block(&mut tap, F, &one, &[m]);
        let side = tap.finish(1);
        let sh = shade_with(&s, Some(&side), &opts(40));
        assert!(sh.at.len() >= 8, "{:?}", sh.stats);
        assert_eq!((sh.stats.exact, sh.stats.pixel), (sh.at.len() as u32, 0));
        assert!(sh.pixel.iter().all(Option::is_none));
        assert_eq!(sh.stats.effective(Shading::Vertex), Shading::Vertex);
        // Side k's corners at angle k/16 and (k+1)/16 of the turn: the
        // mean of two neighbours' normals (equal weights), normalised.
        let k = (0..16).find(|&k| !one[k].0.back && !one[(k + 1) % 16].0.back && !one[(k + 15) % 16].0.back).unwrap();
        let c = sh.get(ats[0][k]).expect("lit");
        let lamp =
            Lamp::of(&Block { light: L, ..Default::default() }, &Face { flags: Face::FRONT, ..Default::default() }, &m);
        // Up to one offset for the whole surface (its brightness kept).
        let n0 = |a: f32| {
            let a = a / 16.0 * std::f32::consts::TAU;
            [a.cos(), 0.0, a.sin()]
        };
        let shift = c[0][0] - clamp_corner(lamp.eval(n0(k as f32)));
        assert!(shift.abs() < 1.0, "{shift}");
        for (corner, a) in [(0usize, k as f32), (1, k as f32 + 1.0), (2, k as f32 + 1.0), (3, k as f32)] {
            let n = n0(a);
            let want = clamp_corner(lamp.eval(n)) + shift;
            assert!((c[corner][0] - want).abs() < 0.05, "corner {corner}: {} vs {want}", c[corner][0]);
            assert!(dot([c[corner][1], c[corner][2], c[corner][3]], n) > 0.9999, "{c:?}");
        }
        // Per pixel: the same corners, the lighting kept.
        let px = shade_with(&s, Some(&side), &pixel(40));
        assert_eq!(px.stats.pixel, px.at.len() as u32);
        assert!(px.at.len() >= sh.at.len());
        let pl = px.pixel_light(ats[0][k]).expect("per pixel");
        assert_eq!(pl.lamp, lamp);
        // The same corner normals; values within the surfaces' offsets.
        let pc = px.get(ats[0][k]).unwrap();
        for j in 0..4 {
            assert_eq!(pc[j][1..], c[j][1..]);
            assert!((pc[j][0] - c[j][0]).abs() < 1.0);
        }
        assert_eq!(px.stats.effective(Shading::Pixel), Shading::Pixel);
        // Without the side channel `Pixel` has nothing to light per pixel.
        let l0 = shade(&s, &pixel(40));
        assert!(!l0.at.is_empty() && l0.stats.pixel == 0);
        assert_eq!(l0.stats.effective(Shading::Pixel), Shading::Vertex);
        // Crease 0: nothing, at either level.
        assert!(shade_with(&s, Some(&side), &pixel(0)).at.is_empty());
    }

    /// Where every vertex normal is its face's normal (a planar region, a
    /// box at 40 degrees) the exact layer lights nothing, `Vertex` or
    /// `Pixel`: those polygons take the flat integer path.
    #[test]
    fn exact_layer_is_flat_where_normals_are_the_faces() {
        let m = Material { ambient: 40.0, diffuse: 150.0, spec_scale: 80.0, squarings: 2 };
        let face_luma = |n: [f32; 3], front: bool| {
            let f = Face { n, flags: front as u16, ..Default::default() };
            quantise(Lamp::of(&Block { light: L, ..Default::default() }, &f, &m).eval(n)) as u8
        };
        // A grid of coplanar quads, every normal the same.
        let n = [0.0f32, 0.0, -1.0];
        let mut polys = Vec::new();
        for j in 0..3 {
            for i in 0..3 {
                let (x, y) = (i as f32 * 0.5 - 1.0, j as f32 * 0.5 - 1.0);
                let q = [[x, y, 10.0], [x + 0.5, y, 10.0], [x + 0.5, y + 0.5, 10.0], [x, y + 0.5, 10.0]];
                let mut p = tpoly(&q.map(sp), 0);
                p.back = dot(n, q[0]) < 0.0;
                p.double_sided = true;
                p.luma = face_luma(n, !p.back);
                polys.push((p, n, 0));
            }
        }
        // And a box, all of whose edges are creases at 40 degrees.
        let cube: Vec<_> = cube([0.0, 0.0, 14.0], 1.0)
            .into_iter()
            .zip([
                [0.0, 0.0, -1.0],
                [0.0, 0.0, 1.0],
                [-1.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, -1.0, 0.0],
                [0.0, 1.0, 0.0],
            ])
            .map(|(mut p, n)| {
                p.luma = face_luma(n, !p.back);
                (p, n, 0)
            })
            .collect();
        for block in [polys, cube] {
            let (s, _) = stream(&[block.iter().map(|p| p.0.clone()).collect()]);
            let mut tap = Tap::new();
            side_block(&mut tap, F, &block, &[m]);
            let side = tap.finish(1);
            assert_eq!(self_check(&s, &side).stats.l2, 1);
            for o in [opts(40), pixel(40), opts(180), pixel(180)] {
                let sh = shade_with(&s, Some(&side), &o);
                if o.crease == 40 || block.len() == 9 {
                    assert!(sh.at.is_empty(), "{:?} {:?}", o.shading, sh.stats);
                }
            }
        }
    }

    /// A bumpy surface (faces whose normals scatter around the light):
    /// the normalised mean normals at its corners face the light more
    /// squarely than the faces do, but the surface as drawn keeps the
    /// brightness the game gave it (one offset for the surface).
    #[test]
    fn exact_layer_keeps_a_surface_s_brightness() {
        let m = Material { ambient: 20.0, diffuse: 200.0, spec_scale: 0.0, squarings: Material::NO_SPEC };
        let l = [0.0f32, 0.0, 1.0];
        let mut seed = 7u32;
        let mut rnd = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1 << 24) as f32 - 0.5
        };
        // A 6x6 grid facing the eye, heights scattered.
        let h: Vec<f32> = (0..49).map(|_| rnd() * 0.25).collect();
        let at = |i: usize, j: usize| [i as f32 * 0.3 - 0.9, j as f32 * 0.3 - 0.9, 8.0 + h[j * 7 + i]];
        let mut block = Vec::new();
        for j in 0..6 {
            for i in 0..6 {
                let q = [at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)];
                let e1 = sub(q[0], q[1]);
                let e2 = sub(q[2], q[1]);
                let mut n = cross(e1, e2);
                let len = dot(n, n).sqrt();
                n = n.map(|x| x / len);
                if dot(n, q[2]) > 0.0 {
                    n = n.map(|x| -x);
                }
                // Faces the eye: a back face by the programs' test (n·P < 0), lit from the front side.
                let mut p = tpoly(&q.map(sp), 0);
                p.back = true;
                p.double_sided = true;
                let f = Face { n, flags: 0, ..Default::default() };
                p.luma = quantise(Lamp::of(&Block { light: l, ..Default::default() }, &f, &m).eval(n)) as u8;
                block.push((p, n, 0));
            }
        }
        let (s, ats) = stream(&[block.iter().map(|p| p.0.clone()).collect()]);
        let mut tap = Tap::new();
        tap.block(0x80_0000, Block::TRUNCATE, F.to_bits(), F.to_bits(), l.map(f32::to_bits));
        let bits = |m: &Material| [m.ambient.to_bits(), m.diffuse.to_bits(), m.spec_scale.to_bits()];
        for (p, n, _) in &block {
            tap.face([0; 3], false, true, Base::D, 0, [0; 3], Material::NO_SPEC);
            tap.face(n.map(f32::to_bits), false, !p.back, Base::D, 0, bits(&m), m.squarings);
        }
        let side = tap.finish(1);
        assert_eq!(self_check(&s, &side).stats.l2, 1, "{:?}", self_check(&s, &side).stats);
        let sh = shade_with(&s, Some(&side), &opts(60));
        assert!(sh.at.len() > 20, "{:?}", sh.stats);
        // Mean of the corner values against the faces' own (+0.5).
        let (mut drawn, mut own) = (0.0f32, 0.0f32);
        for ((p, _, _), at) in block.iter().zip(&ats[0]) {
            own += p.luma as f32 + 0.5;
            drawn += sh.get(*at).map_or(p.luma as f32 + 0.5, |c| c.iter().map(|v| v[0]).sum::<f32>() / 4.0);
        }
        let n = block.len() as f32;
        assert!((drawn / n - own / n).abs() < 1.0, "drawn {} against {}", drawn / n, own / n);
    }

    /// A 2B polygon whose recomputed luma is a level off the stream's
    /// (the SHARC truncates) is offset so its own normal gives its own
    /// luma: smoothing never moves it by a level by itself.
    #[test]
    fn exact_layer_offsets_a_level_off_polygon_to_its_own_luma() {
        let m = Material { ambient: 30.0, diffuse: 180.0, spec_scale: 0.0, squarings: Material::NO_SPEC };
        let face = Face { n: [0.6, 0.0, -0.8], flags: Face::FRONT, ..Default::default() };
        let b = Block { light: L, ..Default::default() };
        let v = Lamp::of(&b, &face, &m).eval(face.n);
        let exact = PixelLight::of(&b, &face, &m, quantise(v) as u8);
        assert_eq!(exact.offset, 0.0);
        let off = PixelLight::of(&b, &face, &m, quantise(v) as u8 + 1);
        assert_eq!(quantise(off.luma(face.n)), quantise(v) + 1);
        assert!((off.luma(face.n) - (quantise(v) as f32 + 1.5)).abs() < 1e-3);
        let ambient = Material { diffuse: 0.0, ..m };
        assert!(!PixelLight::of(&b, &face, &ambient, 30).varies());
        assert!(PixelLight::of(&b, &face, &Material { diffuse: 0.0, squarings: 1, spec_scale: 9.0, ..m }, 30).varies());
        assert_eq!((clamp_corner(-3.0), clamp_corner(300.0) as u32, clamp_corner(f32::NAN)), (0.0, 255, 0.0));
    }
}
