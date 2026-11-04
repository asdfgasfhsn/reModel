//! Model 2 rasteriser (software), consuming the geometrizer's output
//! stream. Follows docs/hw/geometrizer.md sections 3.4 and 4-6; most of it
//! is MAME-only description, so treat it as a hypothesis to check against
//! the game's frames.
//!
//! First stage: flat shading. Polygons are filled with their texture
//! header's base colour, scaled by the polygon's luma, and drawn back to
//! front (painter's algorithm on the z-sort key).

use crate::video::{HEIGHT, WIDTH};

/// One 24-bit float from the stream (IEEE single with the low 8 bits cut).
fn f24(w: u32) -> f32 {
    f32::from_bits(w << 8)
}

#[derive(Clone, Copy, Debug, Default)]
struct V3 {
    x: f32,
    y: f32,
    z: f32,
}

#[derive(Clone, Copy, Debug)]
struct Window {
    /// Viewport in screen pixels: x0..x1, y0..y1.
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    /// Projection centres, in screen pixels.
    centres: [(f32, f32); 4],
}

/// A polygon ready to draw.
#[derive(Clone, Debug)]
pub struct Poly {
    pub points: Vec<(f32, f32)>,
    pub key: f32,
    pub window: usize,
    /// Viewport clip rectangle (x0, y0, x1, y1) in screen pixels.
    pub clip: (f32, f32, f32, f32),
    pub rgb: [u8; 3],
}

/// Inputs the rasteriser reads besides the stream.
pub struct Sources<'a> {
    /// Texture header / point ROM ("textures" region) as u16 words.
    pub texture_rom: &'a [u16],
    /// Palette RAM (xBGR555, polygon colours at entries 0x1000+).
    pub palette: &'a [u8],
    /// Screen offsets from the H/V sync registers: 84 + H, 130 + V.
    pub xoff: f32,
    pub yoff: f32,
}

/// Signed 12-bit field.
fn s12(v: u32) -> f32 {
    (((v & 0xfff) << 20) as i32 >> 20) as f32
}

fn rgb555(e: u16) -> [u8; 3] {
    let c = |v: u16| ((v & 31) << 3 | (v & 31) >> 2) as u8;
    [c(e), c(e >> 5), c(e >> 10)]
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
                // Window: start, end, centres 0-3, each 0x00XXXYYY in window
                // space (Y up from the bottom of a 512-high space).
                let f = |k: usize| (s12(rd(i + k) >> 12), s12(rd(i + k)));
                let (sx, sy) = f(0);
                let (ex, ey) = f(1);
                let to_screen = |(x, y): (f32, f32)| (x + src.xoff, (384.0 - y) + src.yoff);
                let (ax, ay) = to_screen((sx, sy));
                let (bx, by) = to_screen((ex, ey));
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
                let tha = rd(i + 1) & 0x7f_ffff;
                let v = |k: usize| V3 { x: f24(rd(k)), y: f24(rd(k + 1)), z: f24(rd(k + 2)) };
                let mut p0 = v(i + 2);
                let mut p1 = v(i + 5);
                i += 8;
                let mut header = tha as usize;
                let mut prev_z = 1e10f32;
                loop {
                    let attr = rd(i);
                    if attr >> 24 == 0xff || attr & 3 == 0 {
                        break;
                    }
                    let quad = attr & 1 != 0;
                    let luma_word = rd(i + 1);
                    let pa = v(i + 3);
                    let pb = if quad { v(i + 6) } else { pa };
                    i += if quad { 9 } else { 6 };

                    let verts: Vec<V3> = if quad { vec![p1, p0, pa, pb] } else { vec![p1, p0, pa] };
                    let link = (attr >> 8) & 3;
                    let double_sided = attr & (1 << 17) != 0;
                    let backface = luma_word & (1 << 23) != 0;
                    let zs = verts.iter().map(|p| p.z);
                    let (minz, maxz) = zs.fold((f32::MAX, f32::MIN), |(a, b), z| (a.min(z), b.max(z)));
                    let key = match (attr >> 10) & 3 {
                        0 => prev_z,
                        1 => minz,
                        2 => maxz,
                        _ => 1e10,
                    };
                    prev_z = key;

                    let colour_entry = src.texture_rom.get(header + 3).copied().unwrap_or(0) >> 6;
                    let step = ((((attr >> 12) & 31) << 27) as i32 >> 27) as isize;
                    header = (header as isize + step * 4).max(0) as usize;

                    let drawn = link != 0 && (double_sided || !backface) && maxz > 0.0;
                    if drawn {
                        let w = &windows[win];
                        let (cx, cy) = w.centres[centre];
                        // No near plane in hardware; clip at a small z so
                        // projection stays finite.
                        let clipped = clip_near(&verts, 1.0);
                        if clipped.len() >= 3 {
                            let points = clipped.iter().map(|p| (cx + p.x / p.z, cy - p.y / p.z)).collect();
                            let luma = (luma_word >> 15) & 0xff;
                            let base = rd16(src.palette, 0x1000 + (colour_entry as usize & 0x3ff));
                            let rgb = rgb555(base).map(|c| ((c as u32 * luma.max(32)) / 255).min(255) as u8);
                            let clip = (w.x0, w.y0, w.x1, w.y1);
                            polys.push(Poly { points, key, window: win, clip, rgb });
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

fn rd16(ram: &[u8], entry: usize) -> u16 {
    let o = entry * 2;
    u16::from_le_bytes([ram.get(o).copied().unwrap_or(0), ram.get(o + 1).copied().unwrap_or(0)])
}

/// Sutherland-Hodgman against z >= near.
fn clip_near(v: &[V3], near: f32) -> Vec<V3> {
    let mut out = Vec::with_capacity(v.len() + 2);
    for k in 0..v.len() {
        let (a, b) = (v[k], v[(k + 1) % v.len()]);
        let (ain, bin) = (a.z >= near, b.z >= near);
        if ain {
            out.push(a);
        }
        if ain != bin {
            let t = (near - a.z) / (b.z - a.z);
            out.push(V3 { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t, z: near });
        }
    }
    out
}

/// Draw polygons into an RGB layer with a coverage mask: windows in order,
/// and within a window from far (large key) to near.
pub fn draw(polys: &mut [Poly]) -> (Vec<u8>, Vec<bool>) {
    let mut rgb = vec![0u8; WIDTH * HEIGHT * 3];
    let mut covered = vec![false; WIDTH * HEIGHT];
    polys.sort_by(|a, b| a.window.cmp(&b.window).then(b.key.total_cmp(&a.key)));
    for p in polys.iter() {
        fill(&p.points, p.clip, |x, y| {
            let o = y * WIDTH + x;
            covered[o] = true;
            rgb[o * 3..o * 3 + 3].copy_from_slice(&p.rgb);
        });
    }
    (rgb, covered)
}

/// Scanline fill of a convex polygon with pixel-centre sampling, limited to
/// `clip` (x0, y0, x1, y1) and the screen.
fn fill(pts: &[(f32, f32)], clip: (f32, f32, f32, f32), mut plot: impl FnMut(usize, usize)) {
    let (ymin, ymax) = pts.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
    let y0 = ymin.max(clip.1).ceil().max(0.0) as i64;
    let y1 = (ymax.min(clip.3).ceil() as i64).min(HEIGHT as i64);
    for y in y0..y1 {
        let yc = y as f32 + 0.5;
        let (mut xl, mut xr) = (f32::MAX, f32::MIN);
        for k in 0..pts.len() {
            let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
            if (a.1 <= yc && b.1 > yc) || (b.1 <= yc && a.1 > yc) {
                let x = a.0 + (yc - a.1) * (b.0 - a.0) / (b.1 - a.1);
                xl = xl.min(x);
                xr = xr.max(x);
            }
        }
        if xl > xr {
            continue;
        }
        let x0 = (xl.max(clip.0) - 0.5).ceil().max(0.0) as i64;
        let x1 = ((xr.min(clip.2) - 0.5).ceil() as i64).min(WIDTH as i64);
        for x in x0..x1 {
            plot(x as usize, y as usize);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_a_square() {
        let mut n = 0;
        let screen = (0.0, 0.0, WIDTH as f32, HEIGHT as f32);
        fill(&[(10.0, 10.0), (20.0, 10.0), (20.0, 20.0), (10.0, 20.0)], screen, |_, _| n += 1);
        assert_eq!(n, 100);
    }

    #[test]
    fn near_clip_keeps_front_part() {
        let tri = [V3 { x: 0.0, y: 0.0, z: -1.0 }, V3 { x: 0.0, y: 0.0, z: 3.0 }, V3 { x: 1.0, y: 0.0, z: 3.0 }];
        let c = clip_near(&tri, 1.0);
        assert_eq!(c.len(), 4);
        assert!(c.iter().all(|p| p.z >= 1.0));
    }
}
