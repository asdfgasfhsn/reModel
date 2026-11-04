//! Model 2 rasteriser (software), consuming the geometrizer's output
//! stream. Follows docs/hw/geometrizer.md sections 3.4 and 4-6; most of it
//! is MAME-only description, so treat it as a hypothesis to check against
//! the game's frames.
//!
//! Polygons are drawn back to front (painter's algorithm on the z-sort key)
//! with perspective-correct, point-sampled textures (level 0, no mipmaps).

use crate::video::{HEIGHT, WIDTH};

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
    iz: f32,
    uz: f32,
    vz: f32,
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
struct Texture {
    x0: u32,
    y0: u32,
    w: u32,
    h: u32,
    mirror_x: bool,
    mirror_y: bool,
    bank: u32,
    translucent: bool,
    checker: bool,
    luma_table: u32,
}

/// A polygon ready to draw.
#[derive(Clone, Debug)]
pub struct Poly {
    pub points: Vec<SVtx>,
    pub key: f32,
    pub window: usize,
    /// Viewport clip rectangle (x0, y0, x1, y1) in screen pixels.
    pub clip: (f32, f32, f32, f32),
    texture: Option<Texture>,
    /// Base colour components (5 bits each) from palette entry 0x1000 + n.
    colour: [u32; 3],
    /// Polygon luma, 0-255.
    luma: u32,
    /// The 4-word texture header, kept for probing.
    header: [u16; 4],
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
        checker: h[0] & (1 << 15) != 0,
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
                        let clipped = clip_near(&verts, 1.0);
                        if clipped.len() >= 3 {
                            let points = clipped
                                .iter()
                                .map(|p| {
                                    let iz = 1.0 / p.z;
                                    SVtx { x: cx + p.x * iz, y: cy - p.y * iz, iz, uz: p.u * iz, vz: p.v * iz }
                                })
                                .collect();
                            let c = rd16(src.palette, 0x1000 + ((header[3] >> 6) as usize & 0x3ff)) as u32;
                            polys.push(Poly {
                                points,
                                key,
                                window: win,
                                clip: (w.x0, w.y0, w.x1, w.y1),
                                texture: texture_from_header(header),
                                colour: [c & 31, (c >> 5) & 31, (c >> 10) & 31],
                                luma: (luma_word >> 15) & 0xff,
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

/// Sutherland-Hodgman against z >= near, interpolating texture coordinates.
fn clip_near(v: &[Vtx], near: f32) -> Vec<Vtx> {
    let mut out = Vec::with_capacity(v.len() + 2);
    for k in 0..v.len() {
        let (a, b) = (v[k], v[(k + 1) % v.len()]);
        let (ain, bin) = (a.z >= near, b.z >= near);
        if ain {
            out.push(a);
        }
        if ain != bin {
            let t = (near - a.z) / (b.z - a.z);
            let l = |p: f32, q: f32| p + (q - p) * t;
            out.push(Vtx { x: l(a.x, b.x), y: l(a.y, b.y), z: near, u: l(a.u, b.u), v: l(a.v, b.v) });
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

/// Shade one pixel; None if it is transparent.
fn shade(p: &Poly, src: &Sources, x: usize, y: usize, u: f32, v: f32) -> Option<[u8; 3]> {
    let lum = match p.texture {
        Some(t) => {
            if t.checker && (x ^ y) & 1 != 0 {
                return None;
            }
            let tx = t.x0 + wrap(u.floor() as i32, t.w, t.mirror_x);
            let ty = t.y0 + wrap(v.floor() as i32, t.h, t.mirror_y);
            let texel = src.texel(t.bank, tx, ty);
            if t.translucent && texel == 15 {
                return None;
            }
            let t8 = texel * 17;
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
                "window {} key {:.1} header {:04x?} colour {:?} luma {} texel histogram {hist:?}\n    texture {:?} points {:?}",
                p.window,
                p.key,
                p.header,
                p.colour,
                p.luma,
                p.texture,
                p.points.iter().map(|v| (v.x as i32, v.y as i32)).collect::<Vec<_>>()
            )
        })
        .collect()
}

/// Draw polygons into an RGB layer with a coverage mask: windows in order,
/// and within a window from far (large key) to near.
pub fn draw(polys: &mut [Poly], src: &Sources) -> (Vec<u8>, Vec<bool>) {
    let mut rgb = vec![0u8; WIDTH * HEIGHT * 3];
    let mut covered = vec![false; WIDTH * HEIGHT];
    polys.sort_by(|a, b| a.window.cmp(&b.window).then(b.key.total_cmp(&a.key)));
    for p in polys.iter() {
        fill(&p.points, p.clip, |x, y, iz, uz, vz| {
            if let Some(c) = shade(p, src, x, y, uz / iz, vz / iz) {
                let o = y * WIDTH + x;
                covered[o] = true;
                rgb[o * 3..o * 3 + 3].copy_from_slice(&c);
            }
        });
    }
    (rgb, covered)
}

/// Scanline fill of a convex polygon with pixel-centre sampling, limited to
/// `clip` (x0, y0, x1, y1) and the screen. `plot` receives (x, y, 1/z,
/// u/z, v/z) interpolated linearly in screen space.
fn fill(pts: &[SVtx], clip: (f32, f32, f32, f32), mut plot: impl FnMut(usize, usize, f32, f32, f32)) {
    let (ymin, ymax) = pts.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.y), b.max(p.y)));
    let y0 = ymin.max(clip.1).ceil().max(0.0) as i64;
    let y1 = (ymax.min(clip.3).ceil() as i64).min(HEIGHT as i64);
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
        let x1 = ((r.0.min(clip.2) - 0.5).ceil() as i64).min(WIDTH as i64);
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
        fill(&[sv(10.0, 10.0), sv(20.0, 10.0), sv(20.0, 20.0), sv(10.0, 20.0)], screen, |_, _, _, _, _| n += 1);
        assert_eq!(n, 100);
    }

    #[test]
    fn near_clip_keeps_front_part() {
        let v = |x, z| Vtx { x, y: 0.0, z, u: x, v: 0.0 };
        let c = clip_near(&[v(0.0, -1.0), v(0.0, 3.0), v(1.0, 3.0)], 1.0);
        assert_eq!(c.len(), 4);
        assert!(c.iter().all(|p| p.z >= 1.0));
    }

    #[test]
    fn wrap_repeat_and_mirror() {
        assert_eq!(wrap(-1, 32, false), 31);
        assert_eq!(wrap(32, 32, true), 31);
        assert_eq!(wrap(-1, 32, true), 0);
    }
}
