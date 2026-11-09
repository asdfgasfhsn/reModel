//! Model 2 rasteriser (software), consuming the geometrizer's output
//! stream. Follows docs/hw/geometrizer.md sections 3.4 and 4-6; most of it
//! is MAME-only description, so treat it as a hypothesis to check against
//! the game's frames.
//!
//! Polygons are drawn back to front (painter's algorithm on the z-sort key)
//! with perspective-correct, point-sampled textures (level 0, no mipmaps).

use crate::enhance::{Address, Enhance, Fnv, Rgba, TexInfo};
use crate::video::{View, HEIGHT, WIDTH};
use std::collections::HashMap;

/// One 24-bit float from the stream (IEEE single with the low 8 bits cut).
fn f24(w: u32) -> f32 {
    f32::from_bits(w << 8)
}

/// View-space vertex with texture coordinates (texels).
#[derive(Clone, Copy, Debug, Default)]
struct Vtx {
    x: f32,
    y: f32,
    z: f32,
    u: f32,
    v: f32,
}

/// Screen-space vertex with perspective-divided attributes.
#[derive(Clone, Copy, Debug)]
pub struct SVtx {
    pub x: f32,
    pub y: f32,
    /// 1/z, u/z and v/z: linear in screen space.
    pub iz: f32,
    pub uz: f32,
    pub vz: f32,
}

#[derive(Clone, Copy, Debug)]
struct Window {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    centres: [(f32, f32); 4],
}

/// Texture parameters from a polygon's 4-word header (section 5.3).
#[derive(Clone, Copy, Debug)]
pub struct Texture {
    /// Position on the texture sheet and size, in texels.
    pub x0: u32,
    pub y0: u32,
    pub w: u32,
    pub h: u32,
    pub mirror_x: bool,
    pub mirror_y: bool,
    pub bank: u32,
    /// Texel 15 is transparent.
    pub translucent: bool,
    pub luma_table: u32,
}

/// A polygon ready to draw.
#[derive(Clone, Debug)]
pub struct Poly {
    pub points: Vec<SVtx>,
    pub key: f32,
    pub window: usize,
    /// Viewport clip rectangle (x0, y0, x1, y1) in screen pixels.
    pub clip: (f32, f32, f32, f32),
    pub texture: Option<Texture>,
    /// Base colour components (5 bits each) from palette entry 0x1000 + n.
    pub colour: [u32; 3],
    /// Polygon luma, 0-255.
    pub luma: u32,
    /// Header bit 15: every other pixel is dropped (a checkerboard, by
    /// screen pixel), textured or flat. waverunr's arch shadow is a flat
    /// one (docs/hw/geometrizer.md 5.3).
    pub checker: bool,
    /// The 4-word texture header, kept for probing.
    pub header: [u16; 4],
    /// Flat and black: a shadow. The games' shadows are flat black (or
    /// dithered black) polygons laid on the ground, at depths that don't
    /// match it (hotd's are screen-parallel slices), so with the depth
    /// buffer they keep the game's order instead.
    pub shadow: bool,
}

/// A polygon placed in a view's output pixels, ready for a rasteriser
/// (this one or a GPU backend).
#[derive(Clone, Debug)]
pub struct Placed {
    /// Index of the polygon in the (sorted) slice passed to [`place`].
    pub poly: usize,
    /// Vertices in output pixels.
    pub points: Vec<SVtx>,
    /// Clip rectangle (x0, y0, x1, y1) in output pixels.
    pub clip: (f32, f32, f32, f32),
    /// Position in draw order within the polygon's window (for the depth
    /// buffer's tie-break, [`depth_value`]).
    pub rank: u32,
    /// Polygons the game sorted to within [`GROUP_SPAN`] of one depth
    /// (including a header's "same depth as the previous polygon") share a
    /// group number: layers of one object, such as a marker and its outline
    /// or a jacket over a body, whose order is the game's. With the depth
    /// buffer they draw in order among themselves and are depth tested as a
    /// group against what was drawn before.
    pub group: u32,
    /// Drawn in the game's order, without the depth test or write, with the
    /// depth buffer on: sorted behind everything (sort key mode 3, and the
    /// polygons linked to it: skies), flat black shadows ([`Poly::shadow`]),
    /// translucent (checkerboard) polygons, whose layering over the scene
    /// is the game's, and polygons covering a wide range of depth
    /// ([`PAINTER_SPREAD`]).
    pub background: bool,
}

/// Depth buffer. Off by default; the original look is the painter's
/// algorithm. Polygons carry 1/z per vertex (`SVtx::iz`), linear in screen
/// space, so each pixel's depth is exact. Per window (HUD windows draw over
/// the scene) the buffer starts empty. A polygon wins a pixel where its
/// depth is at least what is there, so equal depths keep the game's order,
/// and [`depth_value`] lets a later polygon win by up to [`DEPTH_BIAS`] of
/// its depth for each polygon between, which covers the rounding between a
/// decal and the surface under it. Polygons sorted together
/// ([`Placed::group`]) keep the game's order among themselves. What can't be
/// trusted to depth is drawn in the game's order without testing or writing
/// ([`Placed::background`]). Alpha-tested texels that aren't drawn don't
/// write depth.
pub const DEPTH_BIAS: f32 = 1e-5;
/// Scale of the depth buffer's mapping of 1/z into [0, 1), [`depth_value`].
pub const DEPTH_SCALE: f32 = 0.1;

/// The depth buffer's value for a fragment of 1/z `iz` and draw order
/// `rank` in its window: larger is nearer. Monotonic, and finest where 1/z
/// is small (far away), so a float buffer keeps about 1e-6 relative
/// precision over the whole range.
pub fn depth_value(iz: f32, rank: u32) -> f32 {
    let z = iz * (1.0 + rank as f32 * DEPTH_BIAS);
    z / (z + DEPTH_SCALE)
}

/// Inputs the rasteriser reads besides the stream.
pub struct Sources<'a> {
    /// "textures" ROM (texture headers and point lists) as u16 words.
    pub texture_rom: &'a [u16],
    /// Texture point RAM (u16), written by raster command 4.
    pub texture_point_ram: &'a [u16],
    /// Render RAM from 0x11000000: texture banks at +0 / +0x200000, luma
    /// tables at +0x400000.
    pub render_ram: &'a [u8],
    /// Palette RAM from 0x01800000, including the colour translation
    /// tables at +0x10000.
    pub palette: &'a [u8],
    /// Screen offsets from the H/V sync registers: 84 + H, 130 + V.
    pub xoff: f32,
    pub yoff: f32,
}

impl Sources<'_> {
    /// A u16 from texture point space: bit 23 selects point RAM.
    fn point_word(&self, addr: u32) -> u16 {
        if addr & 0x80_0000 != 0 {
            self.texture_point_ram.get((addr & 0xffff) as usize).copied().unwrap_or(0)
        } else {
            self.texture_rom.get((addr & 0x7f_ffff) as usize).copied().unwrap_or(0)
        }
    }

    /// 4bpp texel at sheet coordinates (x, y) in `bank` (section 5.1).
    fn texel(&self, bank: u32, x: u32, y: u32) -> u32 {
        let (x, y) = if x >= 1024 { (x - 1024, y ^ 1024) } else { (x, y) };
        let word = ((y & 2047) / 2) * 512 + (x & 1023) / 2;
        let o = (bank as usize & 1) * 0x20_0000 + word as usize * 2;
        let w = u16::from_le_bytes([self.render_ram[o], self.render_ram[o + 1]]) as u32;
        let shift = 12 - 4 * ((x & 1) + 2 * (y & 1));
        (w >> shift) & 15
    }

    fn luma(&self, table: u32, index: u32) -> u32 {
        let o = 0x40_0000 + ((table * 128 + index) as usize & 0x7fff) * 2;
        self.render_ram.get(o).copied().unwrap_or(0) as u32
    }

    /// Colour translation: component `c` (5 bits) of channel `ch` at luma.
    fn translate(&self, ch: usize, c: u32, lum: u32) -> u8 {
        let o = 0x1_0000 + ch * 0x4000 + ((c * 256 + lum) as usize) * 2;
        self.palette.get(o).copied().unwrap_or(0)
    }
}

fn s12(v: u32) -> f32 {
    (((v & 0xfff) << 20) as i32 >> 20) as f32
}

fn rd16(ram: &[u8], entry: usize) -> u16 {
    let o = entry * 2;
    u16::from_le_bytes([ram.get(o).copied().unwrap_or(0), ram.get(o + 1).copied().unwrap_or(0)])
}

fn texture_from_header(h: [u16; 4]) -> Option<Texture> {
    if h[0] & (1 << 14) == 0 {
        return None;
    }
    Some(Texture {
        w: 32 << (h[0] & 7),
        h: 32 << ((h[0] >> 3) & 7),
        mirror_x: h[0] & (1 << 8) != 0,
        mirror_y: h[0] & (1 << 9) != 0,
        translucent: h[0] & (1 << 13) != 0,
        luma_table: (h[1] & 0xff) as u32,
        x0: ((h[2] & 0x3f) as u32) * 32,
        y0: (((h[2] >> 6) & 0x1f) as u32) * 32,
        bank: ((h[2] >> 12) & 1) as u32,
    })
}

/// Parse a frame's raster stream into screen-space polygons.
pub fn build(stream: &[u32], src: &Sources) -> Vec<Poly> {
    let mut polys = Vec::new();
    let mut windows: Vec<Window> = Vec::new();
    let mut i = 0;
    let rd = |i: usize| stream.get(i).copied().unwrap_or(0xff00_000f);
    while i < stream.len() {
        let w = stream[i];
        i += 1;
        if w >> 24 != 0xff {
            continue; // stray data word: resynchronise on the next command
        }
        let centre = ((w >> 6) & 3) as usize;
        match w & 0xf {
            0x3 => {
                // Window: start, end, centres 0-3 as 0x00XXXYYY, window Y up.
                let f = |k: usize| (s12(rd(i + k) >> 12), s12(rd(i + k)));
                let to_screen = |(x, y): (f32, f32)| (x + src.xoff, (384.0 - y) + src.yoff);
                let (ax, ay) = to_screen(f(0));
                let (bx, by) = to_screen(f(1));
                let mut centres = [(0.0, 0.0); 4];
                for (k, c) in centres.iter_mut().enumerate() {
                    *c = to_screen(f(2 + k));
                }
                windows.push(Window { x0: ax.min(bx), x1: ax.max(bx), y0: ay.min(by), y1: ay.max(by), centres });
                i += 6;
            }
            0x8 => i += 1,
            0x4 => {
                let count = rd(i + 1) as usize;
                i += 2 + count;
            }
            0x1 => {
                let Some(win) = windows.len().checked_sub(1) else { continue };
                let mut tpa = rd(i) & 0xff_ffff;
                let mut tha = rd(i + 1) & 0xff_ffff;
                let pos = |k: usize| Vtx { x: f24(rd(k)), y: f24(rd(k + 1)), z: f24(rd(k + 2)), u: 0.0, v: 0.0 };
                let mut p0 = pos(i + 2);
                let mut p1 = pos(i + 5);
                i += 8;
                let mut prev_z = 1e10f32;
                loop {
                    let attr = rd(i);
                    if attr >> 24 == 0xff || attr & 3 == 0 {
                        break;
                    }
                    let quad = attr & 1 != 0;
                    let luma_word = rd(i + 1);
                    let pa = pos(i + 3);
                    let pb = if quad { pos(i + 6) } else { pa };
                    i += if quad { 9 } else { 6 };

                    let mut verts: Vec<Vtx> = if quad { vec![p1, p0, pa, pb] } else { vec![p1, p0, pa] };
                    // Texture points: (V, U) pairs, 13.3 fixed, per vertex.
                    for (k, vtx) in verts.iter_mut().enumerate() {
                        vtx.v = src.point_word(tpa + 2 * k as u32) as f32 / 8.0;
                        vtx.u = src.point_word(tpa + 2 * k as u32 + 1) as f32 / 8.0;
                    }
                    tpa += 2 * verts.len() as u32;

                    let header = [0, 1, 2, 3].map(|k| src.point_word(tha + k));
                    let step = ((((attr >> 12) & 31) << 27) as i32) >> 27;
                    tha = (tha as i32 + step * 4) as u32 & 0xff_ffff;

                    let link = (attr >> 8) & 3;
                    let double_sided = attr & (1 << 17) != 0;
                    let backface = luma_word & (1 << 23) != 0;
                    let (minz, maxz) = verts.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.z), b.max(p.z)));
                    let key = match (attr >> 10) & 3 {
                        0 => prev_z,
                        1 => minz,
                        2 => maxz,
                        _ => 1e10,
                    };
                    prev_z = key;

                    if link != 0 && (double_sided || !backface) && maxz > 0.0 {
                        let w = &windows[win];
                        let (cx, cy) = w.centres[centre];
                        let clipped = clip_view(&verts, cx, cy);
                        if clipped.len() >= 3 {
                            let points = clipped
                                .iter()
                                .map(|p| {
                                    let iz = 1.0 / p.z;
                                    SVtx { x: cx + p.x * iz, y: cy - p.y * iz, iz, uz: p.u * iz, vz: p.v * iz }
                                })
                                .collect();
                            let c = rd16(src.palette, 0x1000 + ((header[3] >> 6) as usize & 0x3ff)) as u32;
                            let colour = [c & 31, (c >> 5) & 31, (c >> 10) & 31];
                            let luma = (luma_word >> 15) & 0xff;
                            let texture = texture_from_header(header);
                            let shadow =
                                texture.is_none() && (0..3).all(|ch| src.translate(ch, colour[ch], luma >> 2) == 0);
                            polys.push(Poly {
                                points,
                                key,
                                window: win,
                                clip: (w.x0, w.y0, w.x1, w.y1),
                                texture,
                                colour,
                                luma,
                                shadow,
                                checker: header[0] & (1 << 15) != 0,
                                header,
                            });
                        }
                    }

                    // Next polygon's leading edge (section 3.4).
                    (p0, p1) = match link {
                        1 => (p0, pa),
                        3 => (pb, p1),
                        _ => (pa, pb),
                    };
                }
            }
            _ => {}
        }
    }
    polys
}

/// Nearest view-space z kept by [`clip_view`]: only there to keep 1/z
/// finite. The side planes bound x/z and y/z, so a vertex this close
/// to the eye still projects inside the guard band.
const NEAR: f32 = 1e-6;

/// Screen-space guard band [`clip_view`] clips to, in game pixels beyond
/// each edge of the screen: wide enough for any widened view to draw what
/// lies past the original frame, and small enough to keep projected
/// coordinates (and 1/z across a span) well inside f32 precision.
const GUARD: f32 = 2.0 * WIDTH as f32;

/// Clip a view-space polygon to the volume that projects into the guard
/// band around the screen, given the window centre (cx, cy) in screen
/// pixels (sx = cx + x/z, sy = cy - y/z): four side planes through the
/// eye and a near plane at [`NEAR`]. Polygons are clipped, not dropped,
/// so geometry crossing in front of the camera keeps its visible part.
fn clip_view(v: &[Vtx], cx: f32, cy: f32) -> Vec<Vtx> {
    let (x0, x1) = (-GUARD - cx, WIDTH as f32 + GUARD - cx);
    let (y0, y1) = (cy - (HEIGHT as f32 + GUARD), cy + GUARD);
    // Each plane as (a, b, c, d): inside where a·x + b·y + c·z + d >= 0.
    let planes = [
        [0.0, 0.0, 1.0, -NEAR],
        [1.0, 0.0, -x0, 0.0],
        [-1.0, 0.0, x1, 0.0],
        [0.0, 1.0, -y0, 0.0],
        [0.0, -1.0, y1, 0.0],
    ];
    let mut poly = v.to_vec();
    for [a, b, c, d] in planes {
        if poly.len() < 3 {
            break;
        }
        poly = clip_plane(&poly, |p| a * p.x + b * p.y + c * p.z + d);
    }
    poly
}

/// Sutherland-Hodgman against one plane (inside where `dist` >= 0),
/// interpolating texture coordinates linearly in view space.
fn clip_plane(v: &[Vtx], dist: impl Fn(&Vtx) -> f32) -> Vec<Vtx> {
    let mut out = Vec::with_capacity(v.len() + 1);
    for k in 0..v.len() {
        let (a, b) = (v[k], v[(k + 1) % v.len()]);
        let (da, db) = (dist(&a), dist(&b));
        if da >= 0.0 {
            out.push(a);
        }
        if (da >= 0.0) != (db >= 0.0) {
            let t = da / (da - db);
            let l = |p: f32, q: f32| p + (q - p) * t;
            out.push(Vtx { x: l(a.x, b.x), y: l(a.y, b.y), z: l(a.z, b.z), u: l(a.u, b.u), v: l(a.v, b.v) });
        }
    }
    out
}

/// Wrap a texel coordinate into 0..size, mirrored or repeating.
fn wrap(c: i32, size: u32, mirror: bool) -> u32 {
    let s = size as i32;
    if mirror {
        let m = c.rem_euclid(2 * s);
        (if m >= s { 2 * s - 1 - m } else { m }) as u32
    } else {
        c.rem_euclid(s) as u32
    }
}

/// Bilinearly filtered texel at (u, v), scaled to 8 bits (0-255). For a
/// translucent texture, texel 15 is transparent: the opaque neighbours'
/// average stands in for it, and the pixel is dropped when less than half
/// its weight is opaque (docs/hw/geometrizer.md 5.4; the hardware filter is
/// unconfirmed).
fn filtered_texel(t: &Texture, src: &Sources, u: f32, v: f32) -> Option<u32> {
    let (u, v) = (u - 0.5, v - 0.5);
    let (fu, fv) = (u - u.floor(), v - v.floor());
    let (u0, v0) = (u.floor() as i32, v.floor() as i32);
    let (mut sum, mut weight) = (0.0, 0.0);
    for (du, dv, w) in
        [(0, 0, (1.0 - fu) * (1.0 - fv)), (1, 0, fu * (1.0 - fv)), (0, 1, (1.0 - fu) * fv), (1, 1, fu * fv)]
    {
        let tx = t.x0 + wrap(u0 + du, t.w, t.mirror_x);
        let ty = t.y0 + wrap(v0 + dv, t.h, t.mirror_y);
        let texel = src.texel(t.bank, tx, ty);
        if !(t.translucent && texel == 15) {
            sum += texel as f32 * w;
            weight += w;
        }
    }
    if weight < 0.5 {
        return None;
    }
    Some((sum / weight * 17.0).round() as u32)
}

/// Shade one pixel; None if it is transparent.
fn shade(p: &Poly, src: &Sources, x: usize, y: usize, u: f32, v: f32) -> Option<[u8; 3]> {
    if p.checker && (x ^ y) & 1 != 0 {
        return None;
    }
    let lum = match p.texture {
        Some(t) => {
            let t8 = filtered_texel(&t, src, u, v)?;
            (src.luma(t.luma_table, t8 / 2) * p.luma / 256).min(63)
        }
        None => p.luma >> 2,
    };
    Some([0, 1, 2].map(|ch| src.translate(ch, p.colour[ch], lum)))
}

/// Describe the polygons covering screen pixel (x, y), in draw order
/// (debugging aid: `remodel run` prints these with `REMODEL_PROBE=x,y`).
pub fn probe(polys: &[Poly], src: &Sources, x: f32, y: f32) -> Vec<String> {
    let inside = |p: &Poly| {
        let n = p.points.len();
        let side = |i: usize| {
            let (a, b) = (p.points[i], p.points[(i + 1) % n]);
            (b.x - a.x) * (y - a.y) - (b.y - a.y) * (x - a.x)
        };
        n >= 3 && ((0..n).all(|i| side(i) >= 0.0) || (0..n).all(|i| side(i) <= 0.0))
    };
    polys
        .iter()
        .filter(|p| inside(p))
        .map(|p| {
            let mut hist = [0u32; 16];
            if let Some(t) = p.texture {
                for ty in 0..t.h {
                    for tx in 0..t.w {
                        hist[src.texel(t.bank, t.x0 + tx, t.y0 + ty) as usize] += 1;
                    }
                }
            }
            format!(
                "window {} key {:.1} 1/z {:.5}..{:.5} header {:04x?} colour {:?} luma {} texel histogram {hist:?}\n    texture {:?} points {:?}",
                p.window,
                p.key,
                p.points.iter().map(|v| v.iz).fold(f32::MAX, f32::min),
                p.points.iter().map(|v| v.iz).fold(f32::MIN, f32::max),
                p.header,
                p.colour,
                p.luma,
                p.texture,
                p.points.iter().map(|v| (v.x as i32, v.y as i32)).collect::<Vec<_>>()
            )
        })
        .collect()
}

/// Largest texture (in texels) that dumps and packs handle; bigger ones
/// come from garbage headers.
const MAX_HOOKED_TEXELS: u32 = 1 << 21;

/// Identify a polygon's texture for dumps and packs: a hash of its texels
/// (cached per sheet rectangle in `rects`) and of the header fields and
/// colour that change its appearance.
fn tex_info(p: &Poly, t: &Texture, src: &Sources, rects: &mut HashMap<[u32; 5], u64>) -> Option<TexInfo> {
    if t.w * t.h > MAX_HOOKED_TEXELS {
        return None;
    }
    let texels = *rects.entry([t.bank, t.x0, t.y0, t.w, t.h]).or_insert_with(|| {
        let mut f = Fnv::default();
        for y in 0..t.h {
            for x in 0..t.w {
                f.byte(src.texel(t.bank, t.x0 + x, t.y0 + y) as u8);
            }
        }
        f.0
    });
    let colour = (p.colour[0] | p.colour[1] << 5 | p.colour[2] << 10) as u16;
    let mut f = Fnv::default();
    f.bytes(&texels.to_le_bytes());
    for v in [t.w, t.h, t.luma_table, t.translucent as u32, colour as u32] {
        f.bytes(&v.to_le_bytes());
    }
    Some(TexInfo {
        key: f.0,
        width: t.w,
        height: t.h,
        header: p.header,
        colour,
        bank: t.bank,
        x0: t.x0,
        y0: t.y0,
        luma_table: t.luma_table,
        translucent: t.translucent,
    })
}

/// Full polygon luma: the brightest a texture can be drawn.
const FULL_LUMA: u32 = 255;

/// A texture as the game's pipeline draws it at full polygon luma, one
/// pixel per texel (unfiltered); texel 15 of a translucent texture has
/// alpha 0. This is what dumps contain and packs replace.
pub fn texture_image(src: &Sources, info: &TexInfo) -> Rgba {
    let c = [info.colour as u32 & 31, (info.colour as u32 >> 5) & 31, (info.colour as u32 >> 10) & 31];
    let mut data = Vec::with_capacity((info.width * info.height * 4) as usize);
    for y in 0..info.height {
        for x in 0..info.width {
            let t = src.texel(info.bank, info.x0 + x, info.y0 + y);
            let lum = (src.luma(info.luma_table, t * 17 / 2) * FULL_LUMA / 256).min(63);
            let rgb = [0, 1, 2].map(|ch| src.translate(ch, c[ch], lum));
            data.extend_from_slice(&[rgb[0], rgb[1], rgb[2], if info.translucent && t == 15 { 0 } else { 255 }]);
        }
    }
    Rgba { width: info.width as usize, height: info.height as usize, data }
}

/// Lighting for a replaced texture: the replacement stands for the texture
/// at full luma, so each pixel is scaled per channel by what the original
/// pipeline gives at this polygon's luma over what it gives at full luma,
/// for the original (filtered) texel under the pixel. That keeps the
/// game's luma table, cap at 63 and colour translation curves.
struct Light {
    /// Polygon luma is full: nothing to scale.
    full: bool,
    luma: u32,
    colour: [u32; 3],
    /// Fallback ratio where the original texel is transparent or black at
    /// full luma: the ratio for the brightest luma.
    poly: [f32; 3],
}

impl Light {
    fn new(p: &Poly, src: &Sources) -> Self {
        let poly = [0, 1, 2].map(|ch| {
            let full = src.translate(ch, p.colour[ch], 63) as f32;
            if full > 0.0 {
                src.translate(ch, p.colour[ch], (64 * p.luma / 256).min(63)) as f32 / full
            } else {
                p.luma as f32 / FULL_LUMA as f32
            }
        });
        Light { full: p.luma >= FULL_LUMA, luma: p.luma, colour: p.colour, poly }
    }

    fn ratio(&self, t: &Texture, src: &Sources, u: f32, v: f32) -> [f32; 3] {
        if self.full {
            return [1.0; 3];
        }
        let Some(t8) = filtered_texel(t, src, u, v) else { return self.poly };
        let l = src.luma(t.luma_table, t8 / 2);
        let (lp, lf) = ((l * self.luma / 256).min(63), (l * FULL_LUMA / 256).min(63));
        [0, 1, 2].map(|ch| {
            let full = src.translate(ch, self.colour[ch], lf);
            if full == 0 {
                self.poly[ch]
            } else {
                src.translate(ch, self.colour[ch], lp) as f32 / full as f32
            }
        })
    }
}

/// Shade one pixel from a replacement image; None if transparent.
#[allow(clippy::too_many_arguments)]
fn shade_replaced(
    t: &Texture,
    checker: bool,
    img: &Rgba,
    light: &Light,
    src: &Sources,
    x: usize,
    y: usize,
    u: f32,
    v: f32,
) -> Option<[u8; 3]> {
    if checker && (x ^ y) & 1 != 0 {
        return None;
    }
    let mode = |mirror| if mirror { Address::Mirror } else { Address::Repeat };
    let sx = u * img.width as f32 / t.w as f32;
    let sy = v * img.height as f32 / t.h as f32;
    let (rgb, alpha) = img.sample(sx, sy, mode(t.mirror_x), mode(t.mirror_y));
    if alpha < 0.5 {
        return None;
    }
    let r = light.ratio(t, src, u, v);
    Some([0, 1, 2].map(|ch| (rgb[ch] * r[ch]).round().clamp(0.0, 255.0) as u8))
}

/// Draw polygons into an RGB layer with a coverage mask: windows in order,
/// and within a window from far (large key) to near.
pub fn draw(polys: &mut [Poly], src: &Sources) -> (Vec<u8>, Vec<bool>) {
    draw_view(polys, src, &View::native())
}

/// `draw` at the size of `view`. Viewports spanning the full original
/// width are widened to the view's edges, so whatever 3D the game sends
/// beyond the original frame is drawn.
pub fn draw_view(polys: &mut [Poly], src: &Sources, view: &View) -> (Vec<u8>, Vec<bool>) {
    draw_impl(polys, src, view, None, false)
}

/// `draw_view` with the depth buffer on or off (see [`DEPTH_BIAS`]).
pub fn draw_view_depth(polys: &mut [Poly], src: &Sources, view: &View, depth: bool) -> (Vec<u8>, Vec<bool>) {
    draw_impl(polys, src, view, None, depth)
}

/// `draw_view`, reporting textures to `hooks` and drawing the replacements
/// it returns (texture dumps and packs).
pub fn draw_enhanced(polys: &mut [Poly], src: &Sources, view: &View, hooks: &mut dyn Enhance) -> (Vec<u8>, Vec<bool>) {
    draw_impl(polys, src, view, Some(hooks), false)
}

/// `draw_enhanced` with the depth buffer on or off.
pub fn draw_enhanced_depth(
    polys: &mut [Poly],
    src: &Sources,
    view: &View,
    hooks: &mut dyn Enhance,
    depth: bool,
) -> (Vec<u8>, Vec<bool>) {
    draw_impl(polys, src, view, Some(hooks), depth)
}

/// A polygon whose 1/z ranges over more than this factor (a ground or
/// floor quad running from the camera to the horizon) keeps the game's
/// order with the depth buffer, like a background: the game sorts it by
/// its far edge so that it is drawn first, and where objects stand in or
/// slightly below it (topskatr's ramp in the sand) they are meant to
/// show over it.
pub const PAINTER_SPREAD: f32 = 8.0;

/// How much nearer than a group's first polygon (as a fraction of its
/// sort key) another can sort and still be of the group: 5%.
pub const GROUP_SPAN: f32 = 0.05;

/// Sort key from `build` for "behind everything" (the polygon header's
/// mode 3); keys at or above this are background.
const BACKGROUND_KEY: f32 = 1e9;

/// Sort `polys` into draw order (windows in order, and within a window from
/// far to near) and place each in `view`'s output pixels. Viewports
/// spanning the full original width are widened to the view's edges. HUD
/// polygons in `view.hud_windows` move as a unit with the HUD placement
/// (those in `hud_element_windows` have already been moved by
/// `HudLayout::anchor`).
pub fn place(polys: &mut [Poly], view: &View) -> Vec<Placed> {
    polys.sort_by(|a, b| a.window.cmp(&b.window).then(b.key.total_cmp(&a.key)));
    let mut window_start = 0;
    let mut group = 0;
    let mut group_key = 0.0;
    polys
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if i > 0 && polys[i - 1].window != p.window {
                window_start = i;
            }
            // Keys run far to near: a new group when the key is more than
            // GROUP_SPAN of the group's first below it.
            if i == 0 || polys[i - 1].window != p.window || group_key - p.key > GROUP_SPAN * group_key.abs() {
                group += 1;
                group_key = p.key;
            }
            let dx = if view.hud_windows & (1 << p.window) != 0 {
                let (lo, hi) = p.points.iter().fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(v.x), b.max(v.x)));
                view.hud_shift((lo + hi) / 2.0)
            } else {
                0.0
            };
            let points = p
                .points
                .iter()
                .map(|v| {
                    let (x, y) = view.to_output(v.x + dx, v.y);
                    SVtx { x, y, ..*v }
                })
                .collect();
            let (mut x0, y0, mut x1, y1) = p.clip;
            if view.pad > 0.0 && x0 <= 1.0 && x1 >= WIDTH as f32 - 1.0 {
                (x0, x1) = (view.x_min(), WIDTH as f32 + view.pad);
            }
            let (cx0, cy0) = view.to_output(x0, y0);
            let (cx1, cy1) = view.to_output(x1, y1);
            Placed {
                poly: i,
                points,
                clip: (cx0, cy0, cx1, cy1),
                rank: (i - window_start) as u32,
                group,
                background: p.key >= BACKGROUND_KEY || p.shadow || p.checker || {
                    let (lo, hi) = p.points.iter().fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(v.iz), b.max(v.iz)));
                    hi > lo * PAINTER_SPREAD
                },
            }
        })
        .collect()
}

fn draw_impl(
    polys: &mut [Poly],
    src: &Sources,
    view: &View,
    mut hooks: Option<&mut dyn Enhance>,
    depth: bool,
) -> (Vec<u8>, Vec<bool>) {
    let mut rects = HashMap::new();
    let (w, h) = (view.width, view.height);
    let mut rgb = vec![0u8; w * h * 3];
    let mut covered = vec![false; w * h];
    let placed = place(polys, view);
    // Texel-space filtering works per output pixel; the checker pattern
    // stays in game pixels.
    let (gsx, gsy) = (view.sx, view.sy);
    let mut zbuf = vec![0f32; if depth { w * h } else { 0 }];
    let mut window = usize::MAX;
    // Depths a group of polygons writes, applied when it ends.
    let mut pending: Vec<(usize, f32)> = Vec::new();
    let mut group = u32::MAX;
    for pl in &placed {
        let p = &polys[pl.poly];
        let (points, clip) = (&pl.points[..], pl.clip);
        if pl.group != group {
            group = pl.group;
            for (o, d) in pending.drain(..) {
                zbuf[o] = zbuf[o].max(d);
            }
        }
        // Each window starts with an empty depth buffer where it draws.
        if depth && p.window != window {
            window = p.window;
            let (y0, y1) = (clip.1.floor().max(0.0) as usize, (clip.3.ceil().max(0.0) as usize).min(h));
            let (x0, x1) = (clip.0.floor().max(0.0) as usize, (clip.2.ceil().max(0.0) as usize).min(w));
            for y in y0..y1 {
                zbuf[y * w + x0.min(x1)..y * w + x1].fill(0.0);
            }
        }
        let test = depth && !pl.background;
        // Translucent polygons test but don't write.
        let write = !p.checker;
        let rank = pl.rank;
        let hooked = match (hooks.as_deref_mut(), &p.texture) {
            (Some(hk), Some(t)) => tex_info(p, t, src, &mut rects).map(|info| {
                let img = hk.texture(&info);
                (info, img)
            }),
            _ => None,
        };
        let mut pixels = 0;
        if let (Some((_, Some(img))), Some(t)) = (&hooked, &p.texture) {
            let light = Light::new(p, src);
            fill(points, clip, w, h, |x, y, iz, uz, vz| {
                let o = y * w + x;
                let d = if test { depth_value(iz, rank) } else { 0.0 };
                if test && d < zbuf[o] {
                    return;
                }
                let (gx, gy) = ((x as f32 / gsx) as usize, (y as f32 / gsy) as usize);
                if let Some(c) = shade_replaced(t, p.checker, img, &light, src, gx, gy, uz / iz, vz / iz) {
                    if test && write {
                        pending.push((o, d));
                    }
                    covered[o] = true;
                    rgb[o * 3..o * 3 + 3].copy_from_slice(&c);
                    pixels += 1;
                }
            });
        } else {
            fill(points, clip, w, h, |x, y, iz, uz, vz| {
                let o = y * w + x;
                let d = if test { depth_value(iz, rank) } else { 0.0 };
                if test && d < zbuf[o] {
                    return;
                }
                let (gx, gy) = ((x as f32 / gsx) as usize, (y as f32 / gsy) as usize);
                if let Some(c) = shade(p, src, gx, gy, uz / iz, vz / iz) {
                    if test && write {
                        pending.push((o, d));
                    }
                    covered[o] = true;
                    rgb[o * 3..o * 3 + 3].copy_from_slice(&c);
                    pixels += 1;
                }
            });
        }
        if let (Some(hk), Some((info, _))) = (hooks.as_deref_mut(), &hooked) {
            hk.texture_drawn(info, src, pixels);
        }
    }
    (rgb, covered)
}

/// Scanline fill of a convex polygon with pixel-centre sampling, limited to
/// `clip` (x0, y0, x1, y1) and the screen. `plot` receives (x, y, 1/z,
/// u/z, v/z) interpolated linearly in screen space.
fn fill(
    pts: &[SVtx],
    clip: (f32, f32, f32, f32),
    width: usize,
    height: usize,
    mut plot: impl FnMut(usize, usize, f32, f32, f32),
) {
    let (ymin, ymax) = pts.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.y), b.max(p.y)));
    let y0 = ymin.max(clip.1).ceil().max(0.0) as i64;
    let y1 = (ymax.min(clip.3).ceil() as i64).min(height as i64);
    for y in y0..y1 {
        let yc = y as f32 + 0.5;
        // Left and right span ends: (x, iz, uz, vz).
        let mut l = (f32::MAX, 0.0, 0.0, 0.0);
        let mut r = (f32::MIN, 0.0, 0.0, 0.0);
        for k in 0..pts.len() {
            let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
            if (a.y <= yc && b.y > yc) || (b.y <= yc && a.y > yc) {
                let t = (yc - a.y) / (b.y - a.y);
                let e = |p: f32, q: f32| p + (q - p) * t;
                let s = (e(a.x, b.x), e(a.iz, b.iz), e(a.uz, b.uz), e(a.vz, b.vz));
                if s.0 < l.0 {
                    l = s;
                }
                if s.0 > r.0 {
                    r = s;
                }
            }
        }
        if l.0 > r.0 {
            continue;
        }
        let x0 = (l.0.max(clip.0) - 0.5).ceil().max(0.0) as i64;
        let x1 = ((r.0.min(clip.2) - 0.5).ceil() as i64).min(width as i64);
        let span = (r.0 - l.0).max(1e-6);
        for x in x0..x1 {
            let t = ((x as f32 + 0.5) - l.0) / span;
            let e = |p: f32, q: f32| p + (q - p) * t;
            plot(x as usize, y as usize, e(l.1, r.1), e(l.2, r.2), e(l.3, r.3));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sv(x: f32, y: f32) -> SVtx {
        SVtx { x, y, iz: 1.0, uz: 0.0, vz: 0.0 }
    }

    #[test]
    fn fills_a_square() {
        let mut n = 0;
        let screen = (0.0, 0.0, WIDTH as f32, HEIGHT as f32);
        fill(
            &[sv(10.0, 10.0), sv(20.0, 10.0), sv(20.0, 20.0), sv(10.0, 20.0)],
            screen,
            WIDTH,
            HEIGHT,
            |_, _, _, _, _| n += 1,
        );
        assert_eq!(n, 100);
    }

    #[test]
    fn checker_applies_to_flat_polygons() {
        let (ram, palette) = (vec![0u8; 0x40_0000 + 0x8000], vec![0u8; 0x1_c000]);
        let src = Sources {
            texture_rom: &[],
            texture_point_ram: &[],
            render_ram: &ram,
            palette: &palette,
            xoff: 0.0,
            yoff: 0.0,
        };
        let p = Poly {
            points: Vec::new(),
            key: 0.0,
            window: 0,
            clip: (0.0, 0.0, 0.0, 0.0),
            texture: texture_from_header([0x80c0, 0, 0, 0]),
            colour: [0, 0, 6],
            luma: 255,
            checker: true,
            header: [0x80c0, 0, 0, 0],
            shadow: false,
        };
        assert!(p.texture.is_none());
        assert!(shade(&p, &src, 2, 4, 0.0, 0.0).is_some());
        assert!(shade(&p, &src, 3, 4, 0.0, 0.0).is_none());
    }

    /// A flat polygon covering the middle of the screen: colour index `c`
    /// (translated to c * 20), at 1/z `iz`, sort key `key`.
    fn flat(c: u32, iz: f32, key: f32, window: usize) -> Poly {
        let v = |x, y| SVtx { x, y, iz, uz: 0.0, vz: 0.0 };
        Poly {
            points: vec![v(100.0, 100.0), v(300.0, 100.0), v(300.0, 300.0), v(100.0, 300.0)],
            key,
            window,
            clip: (0.0, 0.0, WIDTH as f32, HEIGHT as f32),
            texture: None,
            colour: [c, c, c],
            luma: 255,
            checker: false,
            header: [0; 4],
            shadow: false,
        }
    }

    fn depth_sources(palette: &mut [u8]) {
        for ch in 0..3 {
            for c in 1..4u32 {
                palette[0x1_0000 + ch * 0x4000 + (c as usize * 256 + 63) * 2] = (c * 20) as u8;
            }
        }
    }

    #[test]
    fn depth_buffer_resolves_what_the_game_sorted_wrong() {
        let ram = vec![0u8; 0x40_0000 + 0x8000];
        let mut palette = vec![0u8; 0x1_c000];
        depth_sources(&mut palette);
        let src = Sources {
            texture_rom: &[],
            texture_point_ram: &[],
            render_ram: &ram,
            palette: &palette,
            xoff: 0.0,
            yoff: 0.0,
        };
        let view = View::native();
        let at = |rgb: &[u8]| rgb[(200 * WIDTH + 200) * 3];
        // Sorted far to near by key: the nearer polygon (1/z 0.5, colour
        // 20) has the larger key, so the game draws it first and the one
        // behind it (1/z 0.1, colour 40) over it.
        let polys = || vec![flat(1, 0.5, 5.0, 0), flat(2, 0.1, 1.0, 0)];
        let off = draw_view_depth(&mut polys(), &src, &view, false);
        assert_eq!(at(&off.0), 40);
        let on = draw_view_depth(&mut polys(), &src, &view, true);
        assert_eq!(at(&on.0), 20);
        // The same depth, or within the rank tie-break: the game's order.
        let tie = || vec![flat(1, 0.5, 5.0, 0), flat(2, 0.5, 1.0, 0)];
        assert_eq!(at(&draw_view_depth(&mut tie(), &src, &view, true).0), 40);
        // Another window starts with an empty buffer: it draws over.
        let windows = || vec![flat(1, 0.5, 1.0, 0), flat(2, 0.1, 1.0, 1)];
        assert_eq!(at(&draw_view_depth(&mut windows(), &src, &view, true).0), 40);
        // A shadow or a background keeps the game's order too.
        let background = || vec![flat(1, 0.5, 5.0, 0), flat(2, 0.1, 1e10, 0)];
        assert_eq!(at(&draw_view_depth(&mut background(), &src, &view, true).0), 20);
        // Off is the painter's algorithm, whatever the depths.
        assert_eq!(at(&draw_view(&mut polys(), &src, &view).0), 40);
    }

    #[test]
    fn depth_value_grows_toward_the_viewer() {
        let mut last = 0.0;
        for k in 0..60 {
            let d = depth_value(1e-6 * 1.5f32.powi(k), 0);
            assert!(d > last && d < 1.0, "{k}: {d}");
            last = d;
        }
        // A later polygon of the same depth wins.
        assert!(depth_value(0.1, 5) > depth_value(0.1, 4));
    }

    #[test]
    fn groups_gather_polygons_sorted_together() {
        let mut polys = vec![flat(1, 0.5, 10.0, 0), flat(2, 0.5, 9.8, 0), flat(3, 0.5, 5.0, 0), flat(1, 0.5, 5.0, 1)];
        let placed = place(&mut polys, &View::native());
        assert_eq!(placed.iter().map(|p| p.group - placed[0].group).collect::<Vec<_>>(), [0, 0, 1, 2]);
        assert_eq!(placed.iter().map(|p| p.rank).collect::<Vec<_>>(), [0, 1, 2, 0]);
    }

    #[test]
    fn close_geometry_is_clipped_not_dropped() {
        let v = |x, y, z| Vtx { x, y, z, u: x, v: 0.0 };
        // A triangle in front of the eye, nearer than one unit: kept whole.
        let c = clip_view(&[v(-0.1, 0.0, 0.5), v(0.1, 0.0, 0.5), v(0.0, 0.1, 0.4)], 248.0, 192.0);
        assert_eq!(c.len(), 3);
        // One crossing the eye plane keeps its front part, inside the guard band.
        let c = clip_view(&[v(0.0, 0.0, -1.0), v(-0.1, 0.0, 0.5), v(0.1, 0.0, 0.5)], 248.0, 192.0);
        assert!(c.len() >= 3);
        for p in &c {
            // Near NEAR, give or take rounding at the plane: 1/z stays finite.
            assert!(p.z > NEAR * 0.5);
            let sx = 248.0 + p.x / p.z;
            assert!((-GUARD - 1.0..=WIDTH as f32 + GUARD + 1.0).contains(&sx));
        }
        // Wholly behind the eye: nothing.
        assert!(clip_view(&[v(0.0, 0.0, -1.0), v(1.0, 0.0, -1.0), v(0.0, 1.0, -2.0)], 248.0, 192.0).len() < 3);
    }

    #[test]
    fn bilinear_filter_and_translucent_coverage() {
        // Texels (0,0) = 0 and (1,0) = 15 share the sheet's first word.
        let mut render_ram = vec![0u8; 0x40_0000];
        render_ram[1] = 0x0f;
        let src = Sources {
            texture_rom: &[],
            texture_point_ram: &[],
            render_ram: &render_ram,
            palette: &[],
            xoff: 0.0,
            yoff: 0.0,
        };
        let mut t = texture_from_header([0x4000, 0, 0, 0]).unwrap();
        // Halfway between the two texel centres: the average, 7.5 * 17.
        assert_eq!(filtered_texel(&t, &src, 1.0, 0.5), Some(128));
        assert_eq!(filtered_texel(&t, &src, 0.5, 0.5), Some(0));
        t.translucent = true;
        // 15 is transparent: mostly-opaque pixels take the opaque texel,
        // mostly-transparent ones are dropped.
        assert_eq!(filtered_texel(&t, &src, 0.9, 0.5), Some(0));
        assert_eq!(filtered_texel(&t, &src, 1.1, 0.5), None);
    }

    #[test]
    fn wrap_repeat_and_mirror() {
        assert_eq!(wrap(-1, 32, false), 31);
        assert_eq!(wrap(32, 32, true), 31);
        assert_eq!(wrap(-1, 32, true), 0);
    }
}
