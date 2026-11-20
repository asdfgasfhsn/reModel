//! SPIKE (tier 2): a free mesh (own vertices, per-vertex normals, own UVs,
//! own RGBA texture) drawn through the software renderer in a game model's
//! place or anchored to it, from what the side channel records for the
//! object block: its transform ([`Block::xf`]), focal lengths, light, and
//! the window / centre / place in the stream that [`walk`] reads from the
//! raster stream. Nothing here touches the machine.

use crate::enhance::Rgba;
use crate::light::{self, Block, Lamp, PixelLight};
use crate::raster::{clip_view, Poly, SVtx, Vtx, NEAR};
use std::f32::consts::PI;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;

/// When set, skin pixels are drawn flat magenta (to count what shows).
pub static MARK: AtomicBool = AtomicBool::new(false);
/// Skin pixels written by the last draws (reset by the caller).
pub static SKIN_PIXELS: AtomicU64 = AtomicU64::new(0);
/// The sum of the game-level lumas of those pixels.
pub static SKIN_LUMA: AtomicU64 = AtomicU64::new(0);

/// An indexed triangle mesh in its own space.
pub struct Mesh {
    pub v: Vec<[f32; 3]>,
    pub n: Vec<[f32; 3]>,
    pub uv: Vec<[f32; 2]>,
    pub tris: Vec<[u32; 3]>,
}

/// Unit UV sphere, outward normals, counter-clockwise from outside.
pub fn uv_sphere(stacks: usize, slices: usize) -> Mesh {
    let (mut v, mut uv, mut tris) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..=stacks {
        let th = PI * i as f32 / stacks as f32;
        for j in 0..=slices {
            let ph = 2.0 * PI * j as f32 / slices as f32;
            v.push([th.sin() * ph.cos(), th.cos(), th.sin() * ph.sin()]);
            uv.push([j as f32 / slices as f32, i as f32 / stacks as f32]);
        }
    }
    let at = |i: usize, j: usize| (i * (slices + 1) + j) as u32;
    for i in 0..stacks {
        for j in 0..slices {
            let (a, b, c, d) = (at(i, j), at(i, j + 1), at(i + 1, j), at(i + 1, j + 1));
            if i > 0 {
                tris.push([a, b, c]);
            }
            if i + 1 < stacks {
                tris.push([b, d, c]);
            }
        }
    }
    Mesh { n: v.clone(), v, uv, tris }
}

/// Torus around the y axis: major radius 1, minor radius `r`.
pub fn torus(major: usize, minor: usize, r: f32) -> Mesh {
    let (mut v, mut n, mut uv, mut tris) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for i in 0..=major {
        let a = 2.0 * PI * i as f32 / major as f32;
        for j in 0..=minor {
            let b = 2.0 * PI * j as f32 / minor as f32;
            let nn = [b.cos() * a.cos(), b.sin(), b.cos() * a.sin()];
            n.push(nn);
            v.push([a.cos() + r * nn[0], r * nn[1], a.sin() + r * nn[2]]);
            uv.push([i as f32 / major as f32 * 2.0, j as f32 / minor as f32]);
        }
    }
    let at = |i: usize, j: usize| (i * (minor + 1) + j) as u32;
    for i in 0..major {
        for j in 0..minor {
            let (a, b, c, d) = (at(i, j), at(i, j + 1), at(i + 1, j), at(i + 1, j + 1));
            tris.push([a, b, c]);
            tris.push([b, d, c]);
        }
    }
    Mesh { v, n, uv, tris }
}

/// A checker with a numbered 8x8 grid (cells labelled by a 3x5 digit font),
/// RGBA, any size.
pub fn grid_texture(size: usize) -> Rgba {
    const FONT: [u16; 10] = [0x7B6F, 0x2C97, 0x73E7, 0x73CF, 0x5BC9, 0x79CF, 0x79EF, 0x7249, 0x7BEF, 0x7BCF];
    let cells = 8;
    let cs = size / cells;
    let mut data = vec![255u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let (cx, cy) = ((x / cs).min(7), (y / cs).min(7));
            let dark = (cx + cy) % 2 == 0;
            let hue = [(cx * 32) as u8, (cy * 32) as u8, 160 - (cx * 8 + cy * 8) as u8];
            let base = if dark {
                [hue[0] / 2 + 40, hue[1] / 2 + 40, hue[2] / 2 + 40]
            } else {
                [hue[0] / 2 + 150, hue[1] / 2 + 150, hue[2] / 2 + 100]
            };
            let o = (y * size + x) * 4;
            data[o..o + 3].copy_from_slice(&base);
            let (lx, ly) = (x % cs, y % cs);
            let sc = (cs / 8).max(1);
            let (gx, gy) = (lx / sc, ly / sc);
            let num = cy * cells + cx;
            for (d, digit) in [num / 10, num % 10].into_iter().enumerate() {
                let gx0 = 1 + d * 4;
                if (gx0..gx0 + 3).contains(&gx) && (1..6).contains(&gy) {
                    let bit = (gy - 1) * 3 + (gx - gx0);
                    if FONT[digit] >> (14 - bit) & 1 != 0 {
                        data[o..o + 3].copy_from_slice(&[255, 255, 255]);
                    }
                }
            }
            // Alpha 0 in a quarter of the last cell: a hole.
            if cx == 7 && cy == 7 && lx < cs / 2 && ly < cs / 2 {
                data[o + 3] = 0;
            }
        }
    }
    Rgba { width: size, height: size, data }
}

/// The texture bank number a free mesh's polygons carry on the GPU path
/// (the sheet has banks 0 and 1; the shader skips the sheet texel for it).
pub const SKIN_BANK: u32 = 2;
/// Side of the (unused) sheet rectangle a mesh polygon names: texture
/// coordinates are in its texels, scaled by the image's size over it.
pub const SKIN_REGION: u32 = 16;

/// Turn software skin triangles into polygons the GPU renderer draws from a
/// pack image: a texture on a bank the shader does not read, coordinates
/// in region texels. The image comes from a pack that answers
/// `bank == SKIN_BANK`.
pub fn gpu_skin(polys: &mut [Poly]) {
    for p in polys.iter_mut().filter(|p| p.skin.is_some()) {
        p.texture = Some(crate::raster::Texture {
            x0: 0,
            y0: 0,
            w: SKIN_REGION,
            h: SKIN_REGION,
            mirror_x: false,
            mirror_y: false,
            bank: SKIN_BANK,
            translucent: false,
            luma_table: 0,
        });
        for v in &mut p.points {
            v.uz *= SKIN_REGION as f32;
            v.vz *= SKIN_REGION as f32;
        }
    }
}

/// The colour (5 bits a channel) the block's first texture header gives:
/// the stream's `0x1` command at `at` carries the header address, which
/// the program sent even when the model's own words were removed.
pub fn block_colour(stream: &[u32], at: usize, src: &crate::raster::Sources) -> [u32; 3] {
    let tha = stream.get(at + 2).copied().unwrap_or(0) & 0xff_ffff;
    let h3 = src.point_word(tha + 3) as usize;
    let o = (0x1000 + ((h3 >> 6) & 0x3ff)) * 2;
    let c = src.palette.get(o..o + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]])) as u32;
    [c & 31, (c >> 5) & 31, (c >> 10) & 31]
}

/// What a skin triangle needs at draw time ([`Poly::skin`]).
#[derive(Debug)]
pub struct Skin {
    pub img: Rgba,
}

/// What the stream says of one object block (read by [`walk`]).
#[derive(Clone, Copy, Debug)]
pub struct Place {
    /// Counted from 0 among the blocks after the first window
    /// ([`Poly::block`]); the side channel's block index.
    pub block: u32,
    /// Index in the stream of its `0x1` command word.
    pub at: usize,
    pub window: usize,
    pub centre: (f32, f32),
    pub clip: (f32, f32, f32, f32),
}

fn s12(v: u32) -> f32 {
    (((v & 0xfff) << 20) as i32 >> 20) as f32
}

/// Each object block's window, centre and clip in the raster stream.
pub fn walk(stream: &[u32], xoff: f32, yoff: f32) -> Vec<Place> {
    let rd = |i: usize| stream.get(i).copied().unwrap_or(0xff00_000f);
    let mut windows: Vec<(f32, f32, f32, f32, [(f32, f32); 4])> = Vec::new();
    let (mut out, mut i, mut block) = (Vec::new(), 0, 0u32);
    while i < stream.len() {
        let w = stream[i];
        i += 1;
        if w >> 24 != 0xff {
            continue;
        }
        match w & 0xf {
            0x3 => {
                let f = |k: usize| (s12(rd(i + k) >> 12), s12(rd(i + k)));
                let to = |(x, y): (f32, f32)| (x + xoff, (384.0 - y) + yoff);
                let (a, b) = (to(f(0)), to(f(1)));
                let mut c = [(0.0, 0.0); 4];
                for (k, c) in c.iter_mut().enumerate() {
                    *c = to(f(2 + k));
                }
                windows.push((a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1), c));
                i += 6;
            }
            0x8 => i += 1,
            0x4 => i += 2 + rd(i + 1) as usize,
            0x1 => {
                let Some(win) = windows.len().checked_sub(1) else { continue };
                let wd = &windows[win];
                out.push(Place {
                    block,
                    at: i - 1,
                    window: win,
                    centre: wd.4[((w >> 6) & 3) as usize],
                    clip: (wd.0, wd.1, wd.2, wd.3),
                });
                block += 1;
                i += 8;
            }
            _ => {}
        }
    }
    out
}

/// How the free mesh's triangles are sorted among the game's polygons.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum KeyMode {
    /// One key for the whole mesh: the given z (the original block's sort
    /// position, or the mesh centre's).
    Fixed(f32),
    /// Every triangle at the mesh's nearest z.
    Near,
    /// Per triangle, the game's modes 1 and 2: nearest / farthest vertex z.
    TriMin,
    TriMax,
}

/// How a mesh sits in the object block's model space, and how it is drawn.
pub struct Instance<'a> {
    pub mesh: &'a Mesh,
    pub skin: Arc<Skin>,
    /// Mesh space to block model space: scale per axis, then translate.
    pub scale: [f32; 3],
    pub offset: [f32; 3],
    /// The lamp of a polygon of the original block (its material and
    /// light); `front` is set per triangle here.
    pub lamp: Lamp,
    /// Palette colour of a neighbouring polygon (for the colour tables).
    pub colour: [u32; 3],
    pub key: KeyMode,
    pub double_sided: bool,
    /// Smooth (per-vertex normals, per pixel) or flat (face normal).
    pub smooth: bool,
    /// Flip the normals' sign (calibration of the game's convention).
    pub flip: bool,
}

pub fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn unit(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt();
    if l > 1e-12 {
        a.map(|c| c / l)
    } else {
        a
    }
}

/// Block model space to view space with the block's matrix `s` (0 or 1).
pub fn to_view(xf: &[f32; 24], s: usize, p: [f32; 3]) -> [f32; 3] {
    let (m, t) = (&xf[12 * s..12 * s + 9], &xf[12 * s + 9..12 * s + 12]);
    [
        t[0] + p[0] * m[0] + p[1] * m[3] + p[2] * m[6],
        t[1] + p[0] * m[1] + p[1] * m[4] + p[2] * m[7],
        t[2] + p[0] * m[2] + p[1] * m[5] + p[2] * m[8],
    ]
}

fn rot(xf: &[f32; 24], n: [f32; 3]) -> [f32; 3] {
    let m = &xf[0..9];
    [
        n[0] * m[0] + n[1] * m[3] + n[2] * m[6],
        n[0] * m[1] + n[1] * m[4] + n[2] * m[7],
        n[0] * m[2] + n[1] * m[5] + n[2] * m[8],
    ]
}

/// Screen position of a view-space point (focal lengths, window centre).
pub fn project(b: &Block, pl: &Place, v: [f32; 3]) -> (f32, f32) {
    (pl.centre.0 + v[0] * b.fx / v[2], pl.centre.1 - v[1] * b.fy / v[2])
}

/// The mesh's triangles as polygons for the renderer.
pub fn polys(inst: &Instance, b: &Block, pl: &Place) -> Vec<Poly> {
    let m = inst.mesh;
    let det = {
        let r = &b.xf[0..9];
        dot([r[0], r[1], r[2]], cross([r[3], r[4], r[5]], [r[6], r[7], r[8]]))
    };
    let s = inst.scale;
    let view: Vec<[f32; 3]> = m
        .v
        .iter()
        .map(|p| to_view(&b.xf, 0, [p[0] * s[0] + inst.offset[0], p[1] * s[1] + inst.offset[1], p[2] * s[2] + inst.offset[2]]))
        .collect();
    // The game's convention has normals pointing away from the eye on the
    // faces it draws; a mesh's outward normals are negated (calibrated by
    // `flip`).
    let sign = if inst.flip { 1.0 } else { -1.0 };
    let nrm: Vec<[f32; 3]> =
        m.n.iter().map(|n| unit(rot(&b.xf, [n[0] / s[0], n[1] / s[1], n[2] / s[2]]).map(|c| c * sign))).collect();
    let near_z = view.iter().map(|v| v[2]).fold(f32::MAX, f32::min);
    let mut out = Vec::new();
    for t in &m.tris {
        let vs = t.map(|i| view[i as usize]);
        if vs.iter().all(|v| v[2] <= 0.0) {
            continue;
        }
        let g = cross(sub(vs[1], vs[0]), sub(vs[2], vs[0]));
        let outward = if det < 0.0 { g.map(|c| -c) } else { g };
        // Visible when its outward side faces the eye at the origin.
        if !inst.double_sided && dot(outward, vs[0]) >= 0.0 {
            continue;
        }
        let face_n = unit(outward.map(|c| c * sign));
        let lamp = Lamp { front: true, ..inst.lamp };
        let pix = PixelLight { lamp, offset: 0.0 };
        let verts: Vec<Vtx> = (0..3)
            .map(|k| {
                let n = if inst.smooth { nrm[t[k] as usize] } else { face_n };
                let l = light::clamp_corner(lamp.eval(n));
                let uv = m.uv[t[k] as usize];
                Vtx {
                    x: vs[k][0] * b.fx,
                    y: vs[k][1] * b.fy,
                    z: vs[k][2],
                    u: uv[0],
                    v: uv[1],
                    lit: [l, n[0], n[1], n[2]],
                    ..Default::default()
                }
            })
            .collect();
        let (minz, maxz) = verts.iter().fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(v.z), b.max(v.z)));
        let key = match inst.key {
            KeyMode::Fixed(z) => z,
            KeyMode::Near => near_z,
            KeyMode::TriMin => minz,
            KeyMode::TriMax => maxz,
        };
        let (cx, cy) = pl.centre;
        let clipped = clip_view(&verts, cx, cy, NEAR);
        if clipped.len() < 3 {
            continue;
        }
        let points = clipped
            .iter()
            .map(|p| {
                let iz = 1.0 / p.z;
                SVtx { x: cx + p.x * iz, y: cy - p.y * iz, iz, uz: p.u * iz, vz: p.v * iz }
            })
            .collect();
        out.push(Poly {
            points,
            key,
            window: pl.window,
            clip: pl.clip,
            texture: None,
            colour: inst.colour,
            luma: light::quantise(verts[0].lit[0]),
            checker: false,
            header: [0; 4],
            block: pl.block,
            at: 0,
            prev: Vec::new(),
            shadow: false,
            lit: clipped.iter().map(|v| v.lit).collect(),
            pixel: inst.smooth.then_some(pix),
            lit_range: [0, 255],
            fine: false,
            lamp: None,
            skin: Some(inst.skin.clone()),
        });
    }
    out
}
