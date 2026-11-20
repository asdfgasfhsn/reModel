//! SPIKE (lod-wide) `remodel lod-dump CAPDIR OUT` and `remodel lod-render CAPDIR OUT FRAMES [HEIGHT ASPECT]`.
//!
//! lod-dump: per frame and object block (a capture made with `--geo native --light`, so
//! light.bin has each block's model key, transform and display-list address) the block's
//! window, polygons, view-space origin and vertex bounds, and its projected box in units
//! where x = -1, +1 are the left and right edges of the original 496-px picture (u =
//! (screen x - 248) / 248; v = (screen y - 192) / 192). Also one row per distinct model.
//! lod-render: 4:3 and wide renders of chosen frames, and the 3D coverage of the side bands
//! of the wide view for every frame.
use model2::capture;
use model2::raster;
use model2::video::{self, View};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

fn f24(w: u32) -> f32 {
    f32::from_bits(w << 8)
}
fn s12(v: u32) -> f32 {
    (((v & 0xfff) << 20) as i32 >> 20) as f32
}

#[derive(Default, Clone)]
struct Model {
    oba: u32,
    ram: bool,
    polys: u32,
    verts: u32,
    mmin: [f32; 3],
    mmax: [f32; 3],
    has_model_box: bool,
    radius_view: f32,
    tex: Vec<u32>,
    instances: u64,
    frames: u64,
}

fn inv3(m: &[f32; 9]) -> Option<[f32; 9]> {
    // columns of A are m[0..3], m[3..6], m[6..9]: A[r][c] = m[3c+r]
    let a = |r: usize, c: usize| m[3 * c + r] as f64;
    let det = a(0, 0) * (a(1, 1) * a(2, 2) - a(1, 2) * a(2, 1)) - a(0, 1) * (a(1, 0) * a(2, 2) - a(1, 2) * a(2, 0))
        + a(0, 2) * (a(1, 0) * a(2, 1) - a(1, 1) * a(2, 0));
    if det.abs() < 1e-12 {
        return None;
    }
    let c = |r: usize, cc: usize| {
        let (r1, r2) = ((r + 1) % 3, (r + 2) % 3);
        let (c1, c2) = ((cc + 1) % 3, (cc + 2) % 3);
        a(r1, c1) * a(r2, c2) - a(r1, c2) * a(r2, c1)
    };
    // inverse[r][c] = cofactor(c, r) / det
    let mut out = [0f32; 9];
    for r in 0..3 {
        for cc in 0..3 {
            out[3 * cc + r] = (c(cc, r) / det) as f32;
        }
    }
    Some(out)
}

pub fn dump(root: &Path, out_dir: &Path) -> Result<(), String> {
    let rom = capture::load_texture_rom(root).map_err(|e| format!("{}: {e}", root.display()))?;
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let mut blocks_tsv = String::from(
        "frame\tblk\tlst\toba\tkey\twin\tcentre\tnpoly\tnvert\tox\toy\toz\tvminx\tvminy\tvminz\tvmaxx\tvmaxy\tvmaxz\tcx\tcy\tcz\tradius\tu0\tv0\tu1\tv1\tbehind\tram\tdirect\n",
    );
    let mut frames_tsv = String::from("frame\tstream_blocks\tlight_blocks\twindows(x0,y0,x1,y1,cx,cy)\txoff\tyoff\n");
    let mut models: BTreeMap<u64, Model> = BTreeMap::new();
    for item in capture::frames(root).map_err(|e| e.to_string())? {
        let (_, f) = item.map_err(|e| e.to_string())?;
        let Some(side) = f.light_frame() else {
            continue;
        };
        let stream = &f.stream;
        let rd = |i: usize| stream.get(i).copied().unwrap_or(0xff00_000f);
        let pw = |a: u32| -> u16 {
            if a & 0x80_0000 != 0 {
                f.texture_point_ram.get((a & 0xffff) as usize).copied().unwrap_or(0)
            } else {
                rom.get((a & 0x7f_ffff) as usize).copied().unwrap_or(0)
            }
        };
        let mut wins: Vec<(f32, f32, f32, f32, [(f32, f32); 4])> = Vec::new();
        let (mut i, mut blk) = (0usize, 0usize);
        let mut seen_in_frame = std::collections::HashSet::new();
        while i < stream.len() {
            let w = stream[i];
            i += 1;
            if w >> 24 != 0xff {
                continue;
            }
            let centre = ((w >> 6) & 3) as usize;
            match w & 0xf {
                0x3 => {
                    let g = |k: usize| (s12(rd(i + k) >> 12), s12(rd(i + k)));
                    let to_screen = |(x, y): (f32, f32)| (x + f.xoff, (384.0 - y) + f.yoff);
                    let (ax, ay) = to_screen(g(0));
                    let (bx, by) = to_screen(g(1));
                    let mut cs = [(0.0, 0.0); 4];
                    for (k, c) in cs.iter_mut().enumerate() {
                        *c = to_screen(g(2 + k));
                    }
                    wins.push((ax.min(bx), ay.min(by), ax.max(bx), ay.max(by), cs));
                    i += 6;
                }
                0x8 => i += 1,
                0x4 => i += 2 + rd(i + 1) as usize,
                0x1 => {
                    let b = side.blocks.get(blk).copied().unwrap_or_default();
                    let win = wins.len() as i32 - 1;
                    let mut tha = rd(i + 1) & 0xff_ffff;
                    let mut vs: Vec<[f32; 3]> = Vec::new();
                    let pos = |k: usize| [f24(rd(k)), f24(rd(k + 1)), f24(rd(k + 2))];
                    vs.push(pos(i + 2));
                    vs.push(pos(i + 5));
                    i += 8;
                    let mut npoly = 0u32;
                    let mut tex: Vec<u32> = Vec::new();
                    loop {
                        let attr = rd(i);
                        if attr >> 24 == 0xff || attr & 3 == 0 {
                            break;
                        }
                        let quad = attr & 1 != 0;
                        vs.push(pos(i + 3));
                        if quad {
                            vs.push(pos(i + 6));
                        }
                        i += if quad { 9 } else { 6 };
                        npoly += 1;
                        let h = [0, 1, 2, 3].map(|k| pw(tha + k));
                        if h[0] & (1 << 14) != 0 {
                            let t = ((h[2] & 0x103f) as u32) | (((h[2] >> 6) & 0x1f) as u32) << 8 | ((h[0] & 0x3f) as u32) << 16;
                            if !tex.contains(&t) {
                                tex.push(t);
                            }
                        }
                        let step = ((((attr >> 12) & 31) << 27) as i32) >> 27;
                        tha = (tha as i32 + step * 4) as u32 & 0xff_ffff;
                    }
                    let (fx, fy) = (if b.fx != 0.0 { b.fx } else { 1.0 }, if b.fy != 0.0 { b.fy } else { 1.0 });
                    // view space (focus divided out)
                    let vv: Vec<[f32; 3]> = vs.iter().map(|v| [v[0] / fx, v[1] / fy, v[2]]).collect();
                    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                    for v in &vv {
                        for k in 0..3 {
                            lo[k] = lo[k].min(v[k]);
                            hi[k] = hi[k].max(v[k]);
                        }
                    }
                    let c = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
                    let rad = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt() / 2.0;
                    // projected box, 4:3 units
                    let (cx, cy) = if win >= 0 { wins[win as usize].4[centre] } else { (248.0, 192.0) };
                    let (mut u0, mut v0, mut u1, mut v1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                    let mut behind = 0;
                    for v in &vs {
                        if v[2] < 1.0 {
                            behind += 1;
                        }
                        let z = v[2].max(1.0);
                        let (sx, sy) = (cx + v[0] / z, cy - v[1] / z);
                        let (u, w2) = ((sx - 248.0) / 248.0, (sy - 192.0) / 192.0);
                        u0 = u0.min(u);
                        u1 = u1.max(u);
                        v0 = v0.min(w2);
                        v1 = v1.max(w2);
                    }
                    let t = [b.xf[9], b.xf[10], b.xf[11]];
                    let _ = writeln!(
                        blocks_tsv,
                        "{}\t{}\t{:x}\t{:06x}\t{:016x}\t{}\t{}\t{}\t{}\t{:.3}\t{:.3}\t{:.3}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.4}\t{:.4}\t{:.4}\t{:.4}\t{}\t{}\t{}",
                        f.frame, blk, b.lst, b.oba, b.key, win, centre, npoly, vs.len(), t[0], t[1], t[2],
                        lo[0], lo[1], lo[2], hi[0], hi[1], hi[2], c[0], c[1], c[2], rad, u0, v0, u1, v1, behind,
                        b.poly_ram() as u8, (b.flags & 1 != 0) as u8
                    );
                    if b.key != 0 {
                        let m = models.entry(b.key).or_default();
                        if m.instances == 0 {
                            m.oba = b.oba;
                            m.ram = b.poly_ram();
                            m.polys = npoly;
                            m.verts = vs.len() as u32;
                            m.tex = tex;
                            m.radius_view = rad;
                            // model space via the inverse of matrix 0 (not for the compact two-matrix format)
                            if b.flags & Block::COMPACT == 0 {
                                let mm: [f32; 9] = std::array::from_fn(|k| b.xf[k]);
                                if let Some(inv) = inv3(&mm) {
                                    let (mut mlo, mut mhi) = ([f32::MAX; 3], [f32::MIN; 3]);
                                    for v in &vv {
                                        let d = [v[0] - t[0], v[1] - t[1], v[2] - t[2]];
                                        for r in 0..3 {
                                            let p = inv[r] * d[0] + inv[3 + r] * d[1] + inv[6 + r] * d[2];
                                            mlo[r] = mlo[r].min(p);
                                            mhi[r] = mhi[r].max(p);
                                        }
                                    }
                                    m.mmin = mlo;
                                    m.mmax = mhi;
                                    m.has_model_box = true;
                                }
                            }
                        }
                        m.instances += 1;
                        if seen_in_frame.insert(b.key) {
                            m.frames += 1;
                        }
                    }
                    blk += 1;
                }
                _ => {}
            }
        }
        let mut ws = String::new();
        for (k, w) in wins.iter().enumerate() {
            let _ = write!(ws, "{}{:.0},{:.0},{:.0},{:.0},{:.0},{:.0}", if k > 0 { ";" } else { "" }, w.0, w.1, w.2, w.3, w.4[0].0, w.4[0].1);
        }
        let _ = writeln!(frames_tsv, "{}\t{}\t{}\t{}\t{}\t{}", f.frame, blk, side.blocks.len(), ws, f.xoff, f.yoff);
    }
    let mut mt = String::from("key\toba\tram\tnpoly\tnvert\tinstances\tframes\tmodel_box(min x y z max x y z | none)\tradius_view_first\ttextures\n");
    for (k, m) in &models {
        let bx = if m.has_model_box {
            format!("{:.2} {:.2} {:.2} {:.2} {:.2} {:.2}", m.mmin[0], m.mmin[1], m.mmin[2], m.mmax[0], m.mmax[1], m.mmax[2])
        } else {
            "none".into()
        };
        let tx: Vec<String> = m.tex.iter().map(|t| format!("{t:x}")).collect();
        let _ = writeln!(mt, "{:016x}\t{:06x}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.2}\t{}", k, m.oba, m.ram as u8, m.polys, m.verts, m.instances, m.frames, bx, m.radius_view, tx.join(","));
    }
    std::fs::write(out_dir.join("blocks.tsv"), blocks_tsv).map_err(|e| e.to_string())?;
    std::fs::write(out_dir.join("frames.tsv"), frames_tsv).map_err(|e| e.to_string())?;
    std::fs::write(out_dir.join("models.tsv"), mt).map_err(|e| e.to_string())?;
    println!("{} distinct models", models.len());
    Ok(())
}

use model2::light::Block;

pub fn render(root: &Path, windows: (&[usize], &[usize]), out_dir: &Path, only: &[u64], height: usize, aspect: f32) -> Result<(), String> {
    let rom = capture::load_texture_rom(root).map_err(|e| format!("{}: {e}", root.display()))?;
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let nat = View::new(height, 4.0 / 3.0, video::Hud::Center).with_hud_windows(windows.0, windows.1);
    let wide = View::new(height, aspect, video::Hud::Center).with_hud_windows(windows.0, windows.1);
    let edge = |v: &View, e: f32| ((v.pad + e) * v.sx).round();
    println!(
        "view native {}x{} sx {:.4}; wide {}x{} sx {:.4} pad {:.2} game px: x=-1 at px {}, x=+1 at px {}",
        nat.width, nat.height, nat.sx, wide.width, wide.height, wide.sx, wide.pad, edge(&wide, 0.0), edge(&wide, 496.0)
    );
    let mut stats = String::from("frame\tleft_band\tleft_in\tright_band\tright_in\tcover_all\n");
    for item in capture::frames(root).map_err(|e| e.to_string())? {
        let (_, f) = item.map_err(|e| e.to_string())?;
        let src = f.sources(&rom);
        let want = only.contains(&f.frame);
        let one = |view: &View| {
            let mut polys = raster::build_shaded(&f.stream, &src, None);
            video::HudLayout::new(&f.tiles, &f.palette, view).anchor(&mut polys, view);
            let (rgb3d, cov) = raster::draw_view(&mut polys, &src, view);
            (rgb3d, cov)
        };
        let (rgb3d, cov) = one(&wide);
        // band width in output pixels: from 0 to edge(-1) on the left, edge(+1) to width on the right
        let (l1, r0) = (edge(&wide, 0.0) as usize, edge(&wide, 496.0) as usize);
        let bw = l1.min(wide.width - r0).max(1);
        let frac = |x0: usize, x1: usize| {
            let (mut n, mut c) = (0u64, 0u64);
            for y in 0..wide.height {
                for x in x0..x1 {
                    n += 1;
                    if cov[y * wide.width + x] {
                        c += 1;
                    }
                }
            }
            c as f64 / n.max(1) as f64
        };
        let all = cov.iter().filter(|&&c| c).count() as f64 / cov.len() as f64;
        let _ = writeln!(
            stats,
            "{}\t{:.4}\t{:.4}\t{:.4}\t{:.4}\t{:.4}",
            f.frame, frac(l1 - bw, l1), frac(l1, l1 + bw), frac(r0, r0 + bw), frac(r0 - bw, r0), all
        );
        if want {
            let rgb = video::compose(&f.tiles, &f.palette, Some((&rgb3d[..], &cov[..])), &wide);
            std::fs::write(out_dir.join(format!("{:06}-wide.png", f.frame)), png::encode_rgb(wide.width, wide.height, &rgb))
                .map_err(|e| e.to_string())?;
            let (r3, c3) = one(&nat);
            let rgb = video::compose(&f.tiles, &f.palette, Some((&r3[..], &c3[..])), &nat);
            std::fs::write(out_dir.join(format!("{:06}-nat.png", f.frame)), png::encode_rgb(nat.width, nat.height, &rgb))
                .map_err(|e| e.to_string())?;
        }
    }
    std::fs::write(out_dir.join("bands.tsv"), stats).map_err(|e| e.to_string())?;
    Ok(())
}
