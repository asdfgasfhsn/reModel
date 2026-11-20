//! SPIKE `remodel lighting-spike OUT FRAME,FRAME,... [--input ..]`: ambient
//! occlusion and shadows from the game's own light, as CPU post-processes
//! over per-pixel position and normal rebuilt from the polygon planes of the
//! software path. Draw-only: the machine is never touched. Env: LS_R (AO
//! radius, view units), LS_S (AO strength), LS_SH (shadow darkness), LS_PLAIN
//! (no hooks, state hash only).
use model2::capture;
use model2::geo::Subst;
use model2::raster::{self, Cast, Sources};
use model2::video::{self, View};
use model2::Machine;
use romset::RomSet;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::Instant;

pub(crate) type V3 = [f32; 3];
pub(crate) fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub(crate) fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(crate) fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub(crate) fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt().max(1e-20);
    [a[0] / l, a[1] / l, a[2] / l]
}
pub(crate) fn env(k: &str, d: f32) -> f32 {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}

/// Per-pixel buffers rebuilt from polygon planes.
pub(crate) struct G {
    pub w: usize,
    pub h: usize,
    pub pos: Vec<V3>,
    pub nrm: Vec<V3>,
    /// Takes the terms: a 3D pixel of a lit, non-HUD, non-sky, non-blob, non-checker polygon.
    pub elig: Vec<bool>,
    /// 0 none, 1 plain, 2 sky, 3 blob shadow, 4 checker, 5 hud, 6 spread (ground in the game's order), 7 no focal
    pub cat: Vec<u8>,
    /// the block of the owner
    pub block: Vec<u32>,
    pub win: Vec<u8>,
}

pub(crate) fn grey(v: f32) -> [u8; 3] {
    let g = (v.clamp(0.0, 1.0) * 255.0) as u8;
    [g, g, g]
}

/// Interleaved 4x4 pattern index.
fn pat(x: usize, y: usize) -> f32 {
    const B: [u8; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
    B[(y & 3) * 4 + (x & 3)] as f32 / 16.0
}

fn ao(g: &G, radius: f32, strength: f32, focal: f32) -> Vec<f32> {
    let (w, h) = (g.w, g.h);
    let mut raw = vec![1.0f32; w * h];
    const N: usize = 16;
    for y in 0..h {
        for x in 0..w {
            let o = y * w + x;
            if !g.elig[o] {
                continue;
            }
            let (p, n) = (g.pos[o], g.nrm[o]);
            let rpx = (radius * focal / p[2]).clamp(3.0, 60.0);
            let rot = pat(x, y) * std::f32::consts::TAU;
            let mut occ = 0.0;
            for k in 0..N {
                let a = k as f32 * 2.399_963 + rot;
                let r = ((k as f32 + 0.5) / N as f32).sqrt() * rpx;
                let (sx, sy) = (x as f32 + a.cos() * r, y as f32 + a.sin() * r);
                if sx < 0.0 || sy < 0.0 || sx >= w as f32 || sy >= h as f32 {
                    continue;
                }
                let so = sy as usize * w + sx as usize;
                if !g.elig[so] {
                    continue;
                }
                let v = sub(g.pos[so], p);
                let d = dot(v, v).sqrt();
                if !(1e-6..radius).contains(&d) {
                    continue;
                }
                let c = dot(n, v) / d;
                occ += (c - 0.15).max(0.0) * (1.0 - d / radius);
            }
            raw[o] = (1.0 - strength * 2.0 * occ / N as f32).clamp(0.0, 1.0);
        }
    }
    // 4x4 surface-aware blur (the pattern repeats every 4 pixels).
    let mut out = raw.clone();
    for y in 0..h {
        for x in 0..w {
            let o = y * w + x;
            if !g.elig[o] {
                continue;
            }
            let (mut s, mut c) = (0.0, 0.0);
            for dy in -2i32..2 {
                for dx in -2i32..2 {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let no = ny as usize * w + nx as usize;
                    if !g.elig[no] || (g.pos[no][2] - g.pos[o][2]).abs() > 0.03 * g.pos[o][2] || dot(g.nrm[no], g.nrm[o]) < 0.9 {
                        continue;
                    }
                    s += raw[no];
                    c += 1.0;
                }
            }
            if c > 0.0 {
                out[o] = s / c;
            }
        }
    }
    out
}

struct Shadow {
    /// 1 lit (or not a receiver), 0 shadowed; fractional at the edge (3x3 PCF).
    term: Vec<f32>,
    extent: (f32, f32),
}

/// A shadow map along `l` (toward the light) from caster triangles in view
/// space, tested at the eligible pixels `ok` selects whose normal faces `l`.
fn shadow(g: &G, tris: &[[V3; 3]], l: V3, ok: &dyn Fn(usize) -> bool, res: usize) -> Shadow {
    let up = if l[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    let e1 = norm(cross(l, up));
    let e2 = cross(l, e1);
    let (w, h) = (g.w, g.h);
    let mut lo = [f32::MAX; 2];
    let mut hi = [f32::MIN; 2];
    for o in 0..w * h {
        if g.elig[o] && ok(o) {
            for (k, e) in [e1, e2].iter().enumerate() {
                let u = dot(g.pos[o], *e);
                lo[k] = lo[k].min(u);
                hi[k] = hi[k].max(u);
            }
        }
    }
    let mut out = Shadow { term: vec![1.0; w * h], extent: (0.0, 0.0) };
    if lo[0] > hi[0] {
        return out;
    }
    let ext = [(hi[0] - lo[0]).max(1e-3), (hi[1] - lo[1]).max(1e-3)];
    out.extent = (ext[0], ext[1]);
    let texel = ext[0].max(ext[1]) / res as f32;
    let (mw, mh) = ((ext[0] / texel) as usize + 2, (ext[1] / texel) as usize + 2);
    let mut map = vec![f32::MIN; mw * mh];
    let to = |p: V3| ((dot(p, e1) - lo[0]) / texel, (dot(p, e2) - lo[1]) / texel, dot(p, l));
    for t in tris {
        let a = t.map(to);
        let (x0, x1) = (a.iter().map(|v| v.0).fold(f32::MAX, f32::min), a.iter().map(|v| v.0).fold(f32::MIN, f32::max));
        let (y0, y1) = (a.iter().map(|v| v.1).fold(f32::MAX, f32::min), a.iter().map(|v| v.1).fold(f32::MIN, f32::max));
        if x1 < 0.0 || y1 < 0.0 || x0 >= mw as f32 || y0 >= mh as f32 || !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
            continue;
        }
        let det = (a[1].1 - a[2].1) * (a[0].0 - a[2].0) + (a[2].0 - a[1].0) * (a[0].1 - a[2].1);
        if det.abs() < 1e-12 {
            // Edge-on to the light: a line (a real map would thicken it).
            continue;
        }
        for my in (y0.max(0.0) as usize)..((y1.ceil() as usize).min(mh - 1) + 1) {
            for mx in (x0.max(0.0) as usize)..((x1.ceil() as usize).min(mw - 1) + 1) {
                let (px, py) = (mx as f32 + 0.5, my as f32 + 0.5);
                let l0 = ((a[1].1 - a[2].1) * (px - a[2].0) + (a[2].0 - a[1].0) * (py - a[2].1)) / det;
                let l1 = ((a[2].1 - a[0].1) * (px - a[2].0) + (a[0].0 - a[2].0) * (py - a[2].1)) / det;
                let l2 = 1.0 - l0 - l1;
                if l0 < -1e-4 || l1 < -1e-4 || l2 < -1e-4 {
                    continue;
                }
                let d = l0 * a[0].2 + l1 * a[1].2 + l2 * a[2].2;
                let m = &mut map[my * mw + mx];
                if d > *m {
                    *m = d;
                }
            }
        }
    }
    for o in 0..w * h {
        if !(g.elig[o] && ok(o)) {
            continue;
        }
        let (p, n) = (g.pos[o], g.nrm[o]);
        let c = dot(n, l);
        if c <= 0.02 {
            continue;
        }
        let tanth = ((1.0 - c * c).max(0.0).sqrt() / c.max(0.15)).min(6.0);
        let bias = texel * (1.5 + 1.5 * tanth);
        let (u, v, d) = to(p);
        let mut lit = 0.0;
        for dy in -1i32..=1 {
            for dx in -1i32..=1 {
                let (mx, my) = ((u as i32 + dx).clamp(0, mw as i32 - 1) as usize, (v as i32 + dy).clamp(0, mh as i32 - 1) as usize);
                if d + bias >= map[my * mw + mx] {
                    lit += 1.0 / 9.0;
                }
            }
        }
        out.term[o] = lit;
    }
    out
}

fn tris_of(casts: &[&Cast], focal: impl Fn(u32) -> (f32, f32)) -> Vec<[V3; 3]> {
    let mut out = Vec::new();
    for c in casts {
        let (fx, fy) = focal(c.block);
        let v: Vec<V3> = c.verts.iter().map(|p| [p[0] / fx, p[1] / fy, p[2]]).collect();
        for k in 1..v.len().saturating_sub(1) {
            out.push([v[0], v[k], v[k + 1]]);
        }
    }
    out
}

pub(crate) struct Scene {
    pub tag: String,
    pub out: std::path::PathBuf,
}

impl Scene {
    pub(crate) fn strip(&self, name: &str, w: usize, h: usize, rows: &[&[u8]]) {
        let mut all = Vec::new();
        for r in rows {
            all.extend_from_slice(r);
        }
        let _ = std::fs::write(self.out.join(format!("{}-{name}.png", self.tag)), png::encode_rgb(w, h * rows.len(), &all));
    }
}

pub(crate) fn pct(a: usize, b: usize) -> f64 {
    100.0 * a as f64 / b.max(1) as f64
}

fn scene(sc: &Scene, cap: &capture::Frame, side: &model2::light::Frame, texrom: &[u16], view: &View, windows: (&[usize], &[usize]), gpu: Option<&mut gpu::Renderer>) {
    let (w, h) = (view.width, view.height);
    let view = view.with_hud_windows(windows.0, windows.1);
    let src = Sources {
        texture_rom: texrom,
        texture_point_ram: &cap.texture_point_ram,
        render_ram: &cap.render_ram,
        palette: &cap.palette,
        xoff: cap.xoff,
        yoff: cap.yoff,
    };
    raster::cast_begin();
    let mut polys = raster::build_shaded(&cap.stream, &src, None);
    let casts = raster::cast_take();
    video::HudLayout::new(&cap.tiles, &cap.palette, &view).anchor(&mut polys, &view);
    let placed = raster::place(&mut polys, &view);
    let vis = raster::visible(&polys, &placed, &src, &view);
    let layer = raster::draw_view_depth(&mut polys, &src, &view, false);
    let orig = video::compose(&cap.tiles, &cap.palette, Some((&layer.0, &layer.1)), &view);
    let by_at: HashMap<u32, usize> = casts.iter().enumerate().filter(|(_, c)| c.survived).map(|(i, c)| (c.at, i)).collect();
    let hud_mask = view.hud_windows | view.hud_element_windows;

    // ---- the buffers
    let t0 = Instant::now();
    let mut g = G { w, h, pos: vec![[0.0; 3]; w * h], nrm: vec![[0.0; 3]; w * h], elig: vec![false; w * h], cat: vec![0; w * h], block: vec![0; w * h], win: vec![0; w * h] };
    let cast_n: Vec<Option<V3>> = casts
        .iter()
        .map(|c| {
            let blk = side.blocks.get(c.block as usize)?;
            let (fx, fy) = blk.focal()?;
            if c.verts.len() < 3 {
                return None;
            }
            let v: Vec<V3> = c.verts.iter().map(|p| [p[0] / fx, p[1] / fy, p[2]]).collect();
            let n = cross(sub(v[1], v[0]), sub(v[2], v[0]));
            (dot(n, n) > 1e-20).then(|| norm(n))
        })
        .collect();
    let mut lum_corr: Vec<(f32, f32)> = Vec::new();
    let mut seen_poly = std::collections::HashSet::new();
    for o in 0..w * h {
        let k = vis.owner[o];
        if k == u32::MAX {
            continue;
        }
        let p = &polys[placed[k as usize].poly];
        g.cat[o] = 1;
        g.block[o] = p.block;
        g.win[o] = p.window as u8;
        let reason = raster::background_reason(p);
        if hud_mask & (1 << p.window) != 0 {
            g.cat[o] = 5;
            continue;
        }
        match reason {
            Some("sky") => g.cat[o] = 2,
            Some("shadow") => g.cat[o] = 3,
            Some("checker") => g.cat[o] = 4,
            Some("spread") => g.cat[o] = 6,
            _ => {}
        }
        let Some(&ci) = by_at.get(&p.at) else { continue };
        let c = &casts[ci];
        let (Some(n), Some((fx, fy))) = (cast_n[ci], side.blocks.get(p.block as usize).and_then(|b| b.focal())) else {
            g.cat[o] = 7;
            continue;
        };
        if matches!(g.cat[o], 2..=4) {
            continue;
        }
        let iz = vis.iz[o];
        if iz <= 0.0 {
            continue;
        }
        let z = 1.0 / iz;
        let (sx, sy) = (o % w, o / w);
        let pos = [((sx as f32 + 0.5) - c.cx) * z / fx, (c.cy - (sy as f32 + 0.5)) * z / fy, z];
        let n = if dot(n, pos) > 0.0 { [-n[0], -n[1], -n[2]] } else { n };
        g.pos[o] = pos;
        g.nrm[o] = n;
        g.elig[o] = true;
        if seen_poly.insert(k) {
            let l = side.blocks[p.block as usize].light;
            if l != [0.0; 3] && p.lit.is_empty() {
                lum_corr.push((dot(n, norm(l)).max(0.0), p.luma as f32));
            }
        }
    }
    let t_gbuf = t0.elapsed().as_secs_f64() * 1000.0;
    let n3d = g.cat.iter().filter(|&&c| c != 0).count();
    let cats = ["none", "plain", "sky", "blob-shadow", "checker", "hud", "spread-ground", "no-focal"];
    let mut line = String::new();
    for (i, nme) in cats.iter().enumerate().skip(1) {
        line += &format!(" {nme} {:.1}%", pct(g.cat.iter().filter(|&&c| c == i as u8).count(), n3d));
    }
    println!("[{}] 3D px {n3d} ({:.1}% of frame); share:{line}; eligible {:.1}% of 3D; gbuffer {t_gbuf:.1} ms", sc.tag, pct(n3d, w * h), pct(g.elig.iter().filter(|&&e| e).count(), n3d));
    let el: Vec<usize> = (0..w * h).filter(|&o| g.elig[o]).collect();
    let mut zs: Vec<f32> = el.iter().map(|&o| g.pos[o][2]).collect();
    zs.sort_by(|a, b| a.total_cmp(b));
    let q = |f: f32| zs.get(((zs.len() as f32 - 1.0) * f) as usize).copied().unwrap_or(0.0);
    let focal = side.blocks.iter().find_map(|b| b.focal()).map_or(1.0, |f| f.0);
    println!("[{}] view depth z of eligible: p5 {:.1} p50 {:.1} p95 {:.1}; focal {:.1} px at z=1; one pixel at p50 = {:.3} units", sc.tag, q(0.05), q(0.5), q(0.95), focal, q(0.5) / focal);

    // Painter vs nearest: a z-buffer over all non-background polygons.
    let mut zb = vec![0.0f32; w * h];
    for pl in &placed {
        let p = &polys[pl.poly];
        if pl.background || hud_mask & (1 << p.window) != 0 {
            continue;
        }
        for t in 1..pl.points.len().saturating_sub(1) {
            let tri = [pl.points[0], pl.points[t], pl.points[t + 1]];
            let fmin = |f: &dyn Fn(&raster::SVtx) -> f32| tri.iter().map(f).fold(f32::MAX, f32::min);
            let fmax = |f: &dyn Fn(&raster::SVtx) -> f32| tri.iter().map(f).fold(f32::MIN, f32::max);
            let (x0, x1) = (fmin(&|v| v.x).max(0.0) as usize, (fmax(&|v| v.x).ceil().max(0.0) as usize).min(w - 1));
            let (y0, y1) = (fmin(&|v| v.y).max(0.0) as usize, (fmax(&|v| v.y).ceil().max(0.0) as usize).min(h - 1));
            let det = (tri[1].y - tri[2].y) * (tri[0].x - tri[2].x) + (tri[2].x - tri[1].x) * (tri[0].y - tri[2].y);
            if det.abs() < 1e-9 {
                continue;
            }
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let l0 = ((tri[1].y - tri[2].y) * (px - tri[2].x) + (tri[2].x - tri[1].x) * (py - tri[2].y)) / det;
                    let l1 = ((tri[2].y - tri[0].y) * (px - tri[2].x) + (tri[0].x - tri[2].x) * (py - tri[2].y)) / det;
                    let l2 = 1.0 - l0 - l1;
                    if l0 < 0.0 || l1 < 0.0 || l2 < 0.0 {
                        continue;
                    }
                    let iz = l0 * tri[0].iz + l1 * tri[1].iz + l2 * tri[2].iz;
                    if iz > zb[y * w + x] {
                        zb[y * w + x] = iz;
                    }
                }
            }
        }
    }
    let (mut over, mut over_spread, mut cmp) = (0usize, 0usize, 0usize);
    for &o in &el {
        if zb[o] <= 0.0 {
            continue;
        }
        cmp += 1;
        if zb[o] > vis.iz[o] * 1.005 {
            over += 1;
            if g.cat[o] == 6 {
                over_spread += 1;
            }
        }
    }
    println!(
        "[{}] eligible px whose shown surface is behind a nearer non-background polygon (any texel, upper bound): {over} of {cmp} ({:.2}%), of which on spread-ground {over_spread}",
        sc.tag,
        pct(over, cmp)
    );

    // ---- light directions
    let mut hist: BTreeMap<[i32; 3], (usize, V3)> = BTreeMap::new();
    for &o in &el {
        let l = side.blocks[g.block[o] as usize].light;
        let e = hist.entry(l.map(|v| (v * 100.0).round() as i32)).or_insert((0, l));
        e.0 += 1;
    }
    let mut hv: Vec<_> = hist.values().copied().collect();
    hv.sort_by(|a, b| b.0.cmp(&a.0));
    println!("[{}] distinct light vectors over eligible px (2 decimals): {}", sc.tag, hv.len());
    for (n, l) in hv.iter().take(6) {
        println!("[{}]   {:>7} px ({:.1}%)  L = ({:.3}, {:.3}, {:.3})  |L|={:.3}", sc.tag, n, pct(*n, el.len()), l[0], l[1], l[2], dot(*l, *l).sqrt());
    }
    let blocks_used: std::collections::BTreeSet<u32> = el.iter().map(|&o| g.block[o]).collect();
    let mut bl: BTreeMap<[i32; 3], usize> = BTreeMap::new();
    for b in &blocks_used {
        *bl.entry(side.blocks[*b as usize].light.map(|v| (v * 100.0).round() as i32)).or_default() += 1;
    }
    println!("[{}] blocks with eligible px: {}; blocks per distinct light: {:?}", sc.tag, blocks_used.len(), bl.values().collect::<Vec<_>>());
    let nn = lum_corr.len() as f32;
    let mut corr = 1.0f32;
    if nn > 3.0 {
        let (mx, my) = (lum_corr.iter().map(|p| p.0).sum::<f32>() / nn, lum_corr.iter().map(|p| p.1).sum::<f32>() / nn);
        let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
        for (x, y) in &lum_corr {
            sxy += (x - mx) * (y - my);
            sxx += (x - mx).powi(2);
            syy += (y - my).powi(2);
        }
        corr = sxy / (sxx * syy).sqrt().max(1e-9);
        println!("[{}] flat untextured polys {}: corr(max(n.L,0), luma) = {:.2} (positive: L points toward the light)", sc.tag, lum_corr.len(), sxy / (sxx * syy).sqrt().max(1e-9));
    }
    let mut wn: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
    for p in &polys {
        wn.entry(p.window).or_default().0 += 1;
    }
    for o in 0..w * h {
        if vis.owner[o] != u32::MAX {
            wn.entry(polys[placed[vis.owner[o] as usize].poly].window).or_default().1 += 1;
        }
    }
    println!("[{}] windows (index: polys, pixels): {:?}; hud mask {:#x}", sc.tag, wn, hud_mask);

    // ---- blob shadows
    let mut blobs: BTreeMap<(u32, usize), (usize, usize, usize)> = BTreeMap::new();
    for c in casts.iter().filter(|c| c.survived && (c.shadow || c.checker && !c.textured)) {
        let e = blobs.entry((c.block, c.window)).or_default();
        e.0 += 1;
        e.1 += c.checker as usize;
    }
    for o in 0..w * h {
        if vis.owner[o] != u32::MAX && g.cat[o] == 3 {
            let p = &polys[placed[vis.owner[o] as usize].poly];
            blobs.entry((p.block, p.window)).or_default().2 += 1;
        }
    }
    for ((b, win), (n, chk, px)) in &blobs {
        let bl = &side.blocks[*b as usize];
        println!(
            "[{}] blob candidate: block {b} window {win} oba {:06x} key {:016x} flags {:#x} polys {n} (checker {chk}) pixels shown {px} light {:?}",
            sc.tag, bl.oba, bl.key, bl.flags, bl.light
        );
    }

    // ---- AO
    let (r, s) = (env("LS_R", 1.0), env("LS_S", 0.8));
    let t0 = Instant::now();
    let a = ao(&g, r, s, focal);
    let t_ao = t0.elapsed().as_secs_f64() * 1000.0;
    let apply = |term: &[f32]| -> Vec<u8> {
        let mut l3 = layer.0.clone();
        for o in 0..w * h {
            if g.elig[o] {
                for c in 0..3 {
                    l3[o * 3 + c] = (l3[o * 3 + c] as f32 * term[o]) as u8;
                }
            }
        }
        video::compose(&cap.tiles, &cap.palette, Some((&l3, &layer.1)), &view)
    };
    let term_img = |term: &[f32]| -> Vec<u8> {
        (0..w * h).flat_map(|o| if g.elig[o] { grey(term[o]) } else if g.cat[o] != 0 { [90, 90, 160] } else { [255, 255, 255] }).collect()
    };
    let ao_img = apply(&a);
    sc.strip("ao", w, h, &[&orig, &ao_img, &term_img(&a)]);
    let mean = |t: &[f32], c: u8| {
        let v: Vec<_> = el.iter().filter(|&&o| c == 0 || g.cat[o] == c).collect();
        v.iter().map(|&&o| t[o] as f64).sum::<f64>() / v.len().max(1) as f64
    };
    let dark = |t: &[f32], th: f32| el.iter().filter(|&&o| t[o] < th).count();
    println!(
        "[{}] AO radius {r} strength {s}: {t_ao:.1} ms CPU (16 samples + 4x4 blur, {} eligible px); mean term {:.3}; px darker than 0.9: {:.1}%, than 0.7: {:.1}%; mean term plain {:.3} spread-ground {:.3}",
        sc.tag,
        el.len(),
        mean(&a, 0),
        pct(dark(&a, 0.9), el.len()),
        pct(dark(&a, 0.7), el.len()),
        mean(&a, 1),
        mean(&a, 6),
    );
    if let Ok(list) = std::env::var("LS_RLIST") {
        for rs in list.split(',') {
            if let Ok(r2) = rs.parse::<f32>() {
                let a2 = ao(&g, r2, s, focal);
                sc.strip(&format!("ao-r{rs}"), w, h, &[&orig, &apply(&a2), &term_img(&a2)]);
                println!("[{}] AO radius {r2}: mean term {:.3}, px < 0.9: {:.1}%, < 0.7: {:.1}%", sc.tag, mean(&a2, 0), pct(dark(&a2, 0.9), el.len()), pct(dark(&a2, 0.7), el.len()));
            }
        }
    }
    if let Some(gr) = gpu {
        let cpu = crate::lighting_gpu::Cpu { elig: &g.elig, term: &a, orig: &orig, w };
        if let Err(e) = crate::lighting_gpu::run(gr, &sc.tag, &sc.out, cap, side, texrom, windows, &casts, &cpu) {
            println!("[{}] GPU error: {e}", sc.tag);
        }
    }
    let same = (0..w * h).filter(|&o| !g.elig[o]).all(|o| orig[o * 3..o * 3 + 3] == ao_img[o * 3..o * 3 + 3]);
    println!("[{}] AO leaves every non-eligible pixel (2D, HUD, sky, blob, checker) byte-identical: {same}", sc.tag);

    // ---- shadows
    let dom = hv.first().map(|h| h.1).unwrap_or([0.0, 1.0, 0.0]);
    let sign = if env("LS_LSIGN", 0.0) != 0.0 { env("LS_LSIGN", 1.0) } else if corr < 0.0 { -1.0 } else { 1.0 };
    println!("[{}] light sign used {sign}", sc.tag);
    let l = norm(dom.map(|v| v * sign));
    let key = |b: u32| side.blocks.get(b as usize).and_then(|b| b.focal()).unwrap_or((1.0, 1.0));
    let zmax = env("LS_ZMAX", 1e9);
    let czmax = env("LS_CZMAX", 1e9);
    let nocut = env("LS_NOCUT", 0.0) != 0.0;
    let casters_ok = |c: &&Cast| !(nocut && c.translucent) && !c.sky && !c.shadow && !c.checker && hud_mask & (1 << c.window) == 0 && c.verts.iter().all(|v| v[2] < czmax);
    let in_dom = |o: usize| norm(side.blocks[g.block[o] as usize].light.map(|v| v * sign)) == l && g.pos[o][2] < zmax;
    // each window's own dominant light
    let wins: Vec<usize> = el.iter().map(|&o| g.win[o] as usize).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    let mut wl: HashMap<usize, V3> = HashMap::new();
    for &wi in &wins {
        let mut hh: BTreeMap<[i32; 3], (usize, V3)> = BTreeMap::new();
        for &o in el.iter().filter(|&&o| g.win[o] as usize == wi) {
            let lv = side.blocks[g.block[o] as usize].light;
            hh.entry(lv.map(|v| (v * 100.0).round() as i32)).or_insert((0, lv)).0 += 1;
        }
        let best = hh.values().max_by_key(|e| e.0).map(|e| e.1).unwrap_or(dom);
        wl.insert(wi, norm(best.map(|v| v * sign)));
        println!("[{}] window {wi}: {} eligible px, {} distinct lights, dominant toward-light ({:.2},{:.2},{:.2})", sc.tag, hh.values().map(|e| e.0).sum::<usize>(), hh.len(), wl[&wi][0], wl[&wi][1], wl[&wi][2]);
    }
    let in_win = |o: usize| norm(side.blocks[g.block[o] as usize].light.map(|v| v * sign)) == wl[&(g.win[o] as usize)] && g.pos[o][2] < zmax;
    let stream_c: Vec<&Cast> = casts.iter().filter(|c| c.survived).filter(casters_ok).collect();
    let full_c: Vec<&Cast> = casts.iter().filter(casters_ok).collect();
    let culled_back = casts.iter().filter(|c| !c.survived && c.backface && !c.double_sided).count();
    let culled_other = casts.iter().filter(|c| !c.survived && !(c.backface && !c.double_sided)).count();
    println!(
        "[{}] caster polygons: stream survivors {}; all seen by the build {}: back-face culled {culled_back}, other drops (view volume / behind eye) {culled_other}",
        sc.tag,
        stream_c.len(),
        full_c.len(),
    );
    let res = env("LS_RES", 2048.0) as usize;
    let per_window = |list: &[&Cast]| -> Shadow {
        let mut out = Shadow { term: vec![1.0; w * h], extent: (0.0, 0.0) };
        for &wi in &wins {
            let cs: Vec<&Cast> = list.iter().copied().filter(|c| c.window == wi).collect();
            let sh = shadow(&g, &tris_of(&cs, key), wl[&wi], &|o| g.win[o] as usize == wi && in_win(o), res);
            for &o in el.iter().filter(|&&o| g.win[o] as usize == wi) {
                out.term[o] = sh.term[o];
            }
            out.extent = sh.extent;
        }
        out
    };
    let t_f = tris_of(&full_c, key);
    let t0 = Instant::now();
    let sh_s = per_window(&stream_c);
    let t_sm = t0.elapsed().as_secs_f64() * 1000.0;
    let t0 = Instant::now();
    let sh_f = per_window(&full_c);
    let t_sf = t0.elapsed().as_secs_f64() * 1000.0;
    // one map for every window (the single-map assumption), global dominant light
    let sh_one = shadow(&g, &t_f, l, &in_dom, res);
    let one_diff = el.iter().filter(|&&o| in_dom(o) && (sh_one.term[o] < 0.5) != (sh_f.term[o] < 0.5)).count();
    println!("[{}] one map over all windows vs a map per window: {one_diff} px of {} in the dominant light differ", sc.tag, el.iter().filter(|&&o| in_dom(o)).count());
    let dk = env("LS_SH", 0.55);
    let to_term = |t: &[f32]| -> Vec<f32> { t.iter().map(|&v| 1.0 - dk * (1.0 - v)).collect() };
    let (ts, tf) = (to_term(&sh_s.term), to_term(&sh_f.term));
    sc.strip("shadow-stream", w, h, &[&orig, &apply(&ts), &term_img(&ts)]);
    sc.strip("shadow-full", w, h, &[&orig, &apply(&tf), &term_img(&tf)]);
    let cnt = |t: &[f32]| el.iter().filter(|&&o| t[o] < 0.5).count();
    let (cs, cf) = (cnt(&sh_s.term), cnt(&sh_f.term));
    let recv = el.iter().filter(|&&o| in_win(o) && dot(g.nrm[o], wl[&(g.win[o] as usize)]) > 0.02).count();
    let diff = el.iter().filter(|&&o| (sh_s.term[o] < 0.5) != (sh_f.term[o] < 0.5)).count();
    println!(
        "[{}] shadow (dominant L=({:.2},{:.2},{:.2}) covers {:.1}% of eligible; receivers facing it {recv}; map {res}px over {:.1}x{:.1} units): in shadow with stream casters {cs} px ({:.1}% of receivers), with unculled {cf} px ({:.1}%); pixels that differ {diff}; extra shadow from unculled {} / lost {}; CPU single-thread {t_sm:.0} ms (stream) / {t_sf:.0} ms (unculled)",
        sc.tag,
        l[0],
        l[1],
        l[2],
        pct(el.iter().filter(|&&o| in_win(o)).count(), el.len()),
        sh_s.extent.0,
        sh_s.extent.1,
        pct(cs, recv),
        pct(cf, recv),
        el.iter().filter(|&&o| sh_s.term[o] >= 0.5 && sh_f.term[o] < 0.5).count(),
        el.iter().filter(|&&o| sh_s.term[o] < 0.5 && sh_f.term[o] >= 0.5).count(),
    );
    // red = shadow only with the unculled casters, blue = only with the stream's, dark = both
    let dimg: Vec<u8> = (0..w * h)
        .flat_map(|o| {
            if !g.elig[o] {
                return orig[o * 3..o * 3 + 3].to_vec();
            }
            match (sh_s.term[o] < 0.5, sh_f.term[o] < 0.5) {
                (false, true) => vec![255, 40, 40],
                (true, false) => vec![40, 40, 255],
                (true, true) => vec![40, 40, 40],
                _ => orig[o * 3..o * 3 + 3].iter().map(|&v| v / 2 + 60).collect(),
            }
        })
        .collect();
    sc.strip("shadow-diff", w, h, &[&orig, &dimg]);
    // blob pixels the real shadow would also cover: test the blob's own surface
    let blob_px: Vec<usize> = (0..w * h).filter(|&o| g.cat[o] == 3).collect();
    if !blob_px.is_empty() {
        let mut g2 = G { w, h, pos: g.pos.clone(), nrm: g.nrm.clone(), elig: vec![false; w * h], cat: g.cat.clone(), block: g.block.clone(), win: g.win.clone() };
        let mut n_ok = 0;
        for &o in &blob_px {
            let p = &polys[placed[vis.owner[o] as usize].poly];
            let Some(&ci) = by_at.get(&p.at) else { continue };
            let (Some(n), Some((fx, fy))) = (cast_n[ci], side.blocks.get(p.block as usize).and_then(|b| b.focal())) else { continue };
            let c = &casts[ci];
            let z = 1.0 / vis.iz[o].max(1e-9);
            let pos = [(((o % w) as f32 + 0.5) - c.cx) * z / fx, (c.cy - ((o / w) as f32 + 0.5)) * z / fy, z];
            g2.pos[o] = pos;
            g2.nrm[o] = if dot(n, pos) > 0.0 { [-n[0], -n[1], -n[2]] } else { n };
            g2.elig[o] = true;
            n_ok += 1;
        }
        let sh_b = shadow(&g2, &t_f, l, &|o| g2.cat[o] == 3, res);
        let inb = blob_px.iter().filter(|&&o| g2.elig[o] && sh_b.term[o] < 0.5).count();
        println!("[{}] blob-shadow pixels {} ({n_ok} with a plane); of those the real shadow (unculled) also covers {inb} ({:.1}%)", sc.tag, blob_px.len(), pct(inb, n_ok));
    }
    let sp: Vec<usize> = el.iter().copied().filter(|&o| g.cat[o] == 6 && in_win(o)).collect();
    println!(
        "[{}] spread-ground receivers (game's order): {} px, shadowed unculled {} ({:.1}%), stream {} ({:.1}%)",
        sc.tag,
        sp.len(),
        sp.iter().filter(|&&o| sh_f.term[o] < 0.5).count(),
        pct(sp.iter().filter(|&&o| sh_f.term[o] < 0.5).count(), sp.len()),
        sp.iter().filter(|&&o| sh_s.term[o] < 0.5).count(),
        pct(sp.iter().filter(|&&o| sh_s.term[o] < 0.5).count(), sp.len())
    );
    if std::env::var_os("LS_B").is_some() {
        let ctx = crate::lighting_shadow::Ctx {
            sc,
            cap,
            side,
            src: &src,
            view: &view,
            polys: &polys,
            placed: &placed,
            casts: &casts,
            hud_mask,
            g: &g,
            sign,
            layer: &layer,
            orig: &orig,
        };
        crate::lighting_shadow::run(&ctx);
    }
    let both: Vec<f32> = (0..w * h).map(|o| a[o] * tf[o]).collect();
    sc.strip("both", w, h, &[&orig, &apply(&both)]);
}

pub fn run(roms: &RomSet, out: &Path, frames: &[u64], spec: &str) -> Result<(), String> {
    let script = model2::input::InputScript::parse(spec, &roms.game.controls).map_err(|e| format!("--input: {e}"))?;
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let plain = std::env::var_os("LS_PLAIN").is_some();
    let mut m: Machine = crate::machine(roms);
    m.bus.geo.set_mode(model2::geo::GeoMode::Native);
    m.bus.geo.set_smooth(true);
    if let Ok(bytes) = std::fs::read(Path::new("out").join(roms.game.id).join("eeprom.bin")) {
        m.bus.eeprom.load(&bytes);
    }
    let mut gr = if std::env::var_os("LG").is_some() { Some(gpu::Renderer::new()?) } else { None };
    let view = View::native();
    let windows = (roms.game.hud_windows, roms.game.hud_element_windows);
    let last = frames.iter().copied().max().unwrap_or(0);
    while m.frames <= last {
        script.apply(m.frames, &mut m.bus);
        m.run_slice(u64::MAX).map_err(|e| format!("stopped: {e:?}"))?;
        m.bus.uart_log.clear();
        if plain || !frames.contains(&m.frames) {
            continue;
        }
        m.bus.geo.sync();
        let Some(sf) = m.bus.geo.engine().smooth_frame() else {
            println!("frame {}: no smooth frame", m.frames);
            continue;
        };
        let cap = capture::Frame::grab(&mut m.bus);
        let texrom: Vec<u16> = m.bus.texture_rom.to_vec();
        let t0 = Instant::now();
        let (rerun, side) = sf.render_subst(&Subst::default(), true);
        let t_rerun = t0.elapsed().as_secs_f64() * 1000.0;
        let Some(side) = side else { continue };
        println!("frame {}: rerun equals machine stream: {} ({t_rerun:.2} ms, {} words)", cap.frame, rerun == cap.stream, rerun.len());
        scene(&Scene { tag: format!("{}-{}", roms.game.id, cap.frame), out: out.to_path_buf() }, &cap, &side, &texrom, &view, windows, gr.as_mut());
    }
    println!("final frame {} state hash {:016x}", m.frames, m.bus.state_hash());
    Ok(())
}
