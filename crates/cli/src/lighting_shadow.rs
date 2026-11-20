//! SPIKE (round 2): shadows from the game's light on the CPU, with the four
//! blockers of round 1 worked on: alpha-tested casters, windows pooled by
//! rule, cascades or a distance limit, blob shadows hidden, the game's own
//! lighting for the shadowed brightness, and stability over frames.
//! Env: LB_RES (map size, 2048), LB_CASC (cascade z boundaries, "0,1e9"),
//! LB_POOL (1: windows of equal light and focal share a map), LB_DK
//! (constant darkening, 0.55), LB_BLOBT (share of a blob's pixels the real
//! shadow must cover to hide it, 0.5), LB_CROP ("x,y,w,h" zoom, 4x),
//! LB_CRAWL (sub-texel crawl table).
use crate::lighting_spike::{cross, dot, env, grey, norm, pct, sub, Scene, G, V3};
use model2::capture;
use model2::raster::{self, Cast, Placed, Poly, Sources, Texture};
use model2::video::{self, View};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Instant;

pub struct Ctx<'a> {
    pub sc: &'a Scene,
    pub cap: &'a capture::Frame,
    pub side: &'a model2::light::Frame,
    pub src: &'a Sources<'a>,
    pub view: &'a View,
    pub polys: &'a [Poly],
    pub placed: &'a [Placed],
    pub casts: &'a [Cast],
    pub hud_mask: u32,
    pub g: &'a G,
    pub sign: f32,
    pub layer: &'a (Vec<u8>, Vec<bool>),
    pub orig: &'a [u8],
}

#[derive(Clone)]
struct Tri {
    v: [V3; 3],
    uv: Option<([[f32; 2]; 3], Texture)>,
    win: usize,
    key: u64,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    /// Every polygon the build saw, cut-outs as solid (round 1).
    Solid,
    /// Without the cut-outs.
    NoCut,
    /// Cut-outs sampled for their texels; culled back faces of solid
    /// polygons included, culled cut-outs dropped.
    Alpha,
}

fn casters(c: &Ctx, mode: Mode, blob_at: &BTreeSet<u32>) -> (Vec<Tri>, [usize; 4]) {
    let by_at: HashMap<u32, &Poly> = c.polys.iter().map(|p| (p.at, p)).collect();
    let mut out = Vec::new();
    let mut n = [0usize; 4]; // from polygons, solid culled faces, dropped cut-outs, textured samples
    for ca in c.casts {
        if ca.sky || ca.shadow || ca.checker || blob_at.contains(&ca.at) || c.hud_mask & (1 << ca.window) != 0 || ca.verts.len() < 3 {
            continue;
        }
        if mode == Mode::NoCut && ca.translucent {
            continue;
        }
        let Some((fx, fy)) = c.side.blocks.get(ca.block as usize).and_then(|b| b.focal()) else { continue };
        let key = c.side.blocks[ca.block as usize].key;
        let poly = if ca.survived && mode == Mode::Alpha { by_at.get(&ca.at).copied() } else { None };
        let (verts, uvs): (Vec<V3>, Option<Vec<[f32; 2]>>) = match poly {
            Some(p) if p.points.iter().all(|v| v.iz > 0.0) => {
                n[0] += 1;
                let v = p.points.iter().map(|s| {
                    let z = 1.0 / s.iz;
                    [(s.x - ca.cx) * z / fx, (ca.cy - s.y) * z / fy, z]
                });
                let uv = p.texture.as_ref().filter(|t| t.translucent).map(|_| p.points.iter().map(|s| [s.uz / s.iz, s.vz / s.iz]).collect());
                (v.collect(), uv)
            }
            _ => {
                if mode == Mode::Alpha && ca.translucent {
                    n[2] += 1;
                    continue;
                }
                n[1] += 1;
                (ca.verts.iter().map(|p| [p[0] / fx, p[1] / fy, p[2]]).collect(), None)
            }
        };
        let tex = poly.and_then(|p| p.texture.clone());
        for k in 1..verts.len() - 1 {
            let uv = match (&uvs, &tex) {
                (Some(u), Some(t)) => {
                    n[3] += 1;
                    Some(([u[0], u[k], u[k + 1]], t.clone()))
                }
                _ => None,
            };
            out.push(Tri { v: [verts[0], verts[k], verts[k + 1]], uv, win: ca.window, key });
        }
    }
    (out, n)
}

/// A shadow map along `l` over the pixels `pix` (their light-space bounds),
/// tested at them: the lit fraction per pixel (3x3 PCF), and the texel size
/// in view units. `shift`: the map's origin moved by that many texels.
fn smap(g: &G, tris: &[&Tri], l: V3, pix: &[usize], res: usize, shift: f32, src: &Sources) -> (Vec<f32>, f32) {
    let up = if l[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    let e1 = norm(cross(l, up));
    let e2 = cross(l, e1);
    let mut lo = [f32::MAX; 2];
    let mut hi = [f32::MIN; 2];
    for &o in pix {
        for (k, e) in [e1, e2].iter().enumerate() {
            let u = dot(g.pos[o], *e);
            lo[k] = lo[k].min(u);
            hi[k] = hi[k].max(u);
        }
    }
    let mut term = vec![1.0f32; pix.len()];
    if pix.is_empty() {
        return (term, 0.0);
    }
    let ext = [(hi[0] - lo[0]).max(1e-3), (hi[1] - lo[1]).max(1e-3)];
    let texel = ext[0].max(ext[1]) / res as f32;
    lo[0] -= shift * texel;
    lo[1] -= shift * texel;
    let (mw, mh) = ((ext[0] / texel) as usize + 3, (ext[1] / texel) as usize + 3);
    let mut map = vec![f32::MIN; mw * mh];
    let to = |p: V3| ((dot(p, e1) - lo[0]) / texel, (dot(p, e2) - lo[1]) / texel, dot(p, l));
    for t in tris {
        let a = t.v.map(to);
        let (x0, x1) = (a.iter().map(|v| v.0).fold(f32::MAX, f32::min), a.iter().map(|v| v.0).fold(f32::MIN, f32::max));
        let (y0, y1) = (a.iter().map(|v| v.1).fold(f32::MAX, f32::min), a.iter().map(|v| v.1).fold(f32::MIN, f32::max));
        if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) || x1 < 0.0 || y1 < 0.0 || x0 >= mw as f32 || y0 >= mh as f32 {
            continue;
        }
        let det = (a[1].1 - a[2].1) * (a[0].0 - a[2].0) + (a[2].0 - a[1].0) * (a[0].1 - a[2].1);
        if det.abs() < 1e-12 {
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
                if d <= *m {
                    continue;
                }
                if let Some((uv, tex)) = &t.uv {
                    let u = l0 * uv[0][0] + l1 * uv[1][0] + l2 * uv[2][0];
                    let v = l0 * uv[0][1] + l1 * uv[1][1] + l2 * uv[2][1];
                    if raster::filtered_texel(tex, src, u, v).is_none() {
                        continue;
                    }
                }
                *m = d;
            }
        }
    }
    for (i, &o) in pix.iter().enumerate() {
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
        term[i] = lit;
    }
    (term, texel)
}

struct Plan {
    /// Windows sharing a map, and their toward-light vector.
    groups: Vec<(Vec<usize>, V3)>,
    /// Receiver depth ranges (view z) with a map each; beyond the last: no shadow.
    casc: Vec<(f32, f32)>,
    res: usize,
}

struct Out {
    /// 1 lit or not a receiver, 0 shadowed.
    f: Vec<f32>,
    /// Receivers facing the light.
    recv: Vec<bool>,
    info: Vec<String>,
    /// (texel / pixel footprint) weighted by receivers.
    ratio: f64,
}

fn pp_plan_groups(c: &Ctx, g: &G, tris: &[Tri]) -> Plan {
    plan(c, g, tris, true, &[0.0, 1e9], 2048)
}

fn pp_groups(p: &Plan, win: usize) -> Option<(usize, V3)> {
    p.groups.iter().enumerate().find(|(_, g)| g.0.contains(&win)).map(|(i, g)| (i, g.1))
}

fn lkey(l: V3) -> [i32; 3] {
    l.map(|v| (v * 100.0).round() as i32)
}

fn plan(c: &Ctx, g: &G, tris: &[Tri], pool: bool, casc: &[f32], res: usize) -> Plan {
    let mut wl: BTreeMap<usize, HashMap<[i32; 3], (usize, V3)>> = BTreeMap::new();
    let mut wf: BTreeMap<usize, (i32, i32)> = BTreeMap::new();
    for o in 0..g.w * g.h {
        if g.elig[o] {
            let b = &c.side.blocks[g.block[o] as usize];
            wl.entry(g.win[o] as usize).or_default().entry(lkey(norm(b.light.map(|v| v * c.sign)))).or_insert((0, norm(b.light.map(|v| v * c.sign)))).0 += 1;
            wf.entry(g.win[o] as usize).or_insert((b.fx.round() as i32, b.fy.round() as i32));
        }
    }
    let _ = tris;
    let mut groups: Vec<(Vec<usize>, V3)> = Vec::new();
    let mut keys: Vec<((i32, i32), [i32; 3])> = Vec::new();
    for (w, h) in &wl {
        let best = h.values().max_by_key(|e| e.0).unwrap().1;
        let key = (wf[w], lkey(best));
        match keys.iter().position(|k| *k == key) {
            Some(i) if pool => groups[i].0.push(*w),
            _ => {
                keys.push(key);
                groups.push((vec![*w], best));
            }
        }
    }
    let mut cs = Vec::new();
    for k in 0..casc.len().saturating_sub(1) {
        cs.push((casc[k], casc[k + 1]));
    }
    Plan { groups, casc: cs, res }
}

fn shadow_all(c: &Ctx, g: &G, tris: &[Tri], pl: &Plan, shift: f32) -> Out {
    let n = g.w * g.h;
    let mut out = Out { f: vec![1.0; n], recv: vec![false; n], info: Vec::new(), ratio: 0.0 };
    let (mut rsum, mut rn) = (0.0f64, 0usize);
    for (gi, (wins, l)) in pl.groups.iter().enumerate() {
        let mine: Vec<&Tri> = tris.iter().filter(|t| wins.contains(&t.win)).collect();
        for (ci, &(z0, z1)) in pl.casc.iter().enumerate() {
            let pix: Vec<usize> = (0..n)
                .filter(|&o| {
                    g.elig[o]
                        && wins.contains(&(g.win[o] as usize))
                        && norm(c.side.blocks[g.block[o] as usize].light.map(|v| v * c.sign)) == *l
                        && g.pos[o][2] >= z0
                        && g.pos[o][2] < z1
                })
                .collect();
            if pix.is_empty() {
                continue;
            }
            let (f, texel) = smap(g, &mine, *l, &pix, pl.res, shift, c.src);
            let mut zs: Vec<f32> = pix.iter().map(|&o| g.pos[o][2]).collect();
            zs.sort_by(|a, b| a.total_cmp(b));
            let zmed = zs[zs.len() / 2];
            let fx = c.side.blocks[g.block[pix[0]] as usize].fx;
            let ratio = texel / (zmed / fx);
            let mut sh = 0;
            let mut rc = 0;
            for (i, &o) in pix.iter().enumerate() {
                out.f[o] = f[i];
                if dot(g.nrm[o], *l) > 0.02 {
                    out.recv[o] = true;
                    rc += 1;
                    sh += (f[i] < 0.5) as usize;
                }
            }
            rsum += ratio as f64 * pix.len() as f64;
            rn += pix.len();
            out.info.push(format!(
                "group {gi} windows {wins:?} L=({:.2},{:.2},{:.2}) cascade {ci} z {z0:.0}..{z1:.0}: {} px, receivers {rc}, shadowed {sh} ({:.1}%), {} casters, texel {texel:.3} units = {ratio:.2} x one pixel at median z {zmed:.1}",
                l[0], l[1], l[2], pix.len(), pct(sh, rc), mine.len()
            ));
        }
    }
    out.ratio = rsum / rn.max(1) as f64;
    out
}

/// The shadow's effect on a polygon, from the game's own formula without
/// its diffuse and specular parts: ambient / luma (the luma truncates, so
/// +0.5 both); None if the face is not found.
fn game_ratio(c: &Ctx, p: &Poly, n: V3) -> Option<f32> {
    let b = c.side.blocks.get(p.block as usize)?;
    if b.flags & model2::light::Block::HAS_FACES == 0 {
        return None;
    }
    let faces = c.side.faces.get(b.first as usize..(b.first + b.count) as usize)?;
    let (mut best, mut bd) = (None, 0.98f32);
    for f in faces {
        let d = dot(n, norm(f.n)).abs();
        if d > bd {
            let m = c.side.materials.get(f.mat as usize)?;
            let lamp = model2::light::Lamp::of(b, f, m);
            if (lamp.eval(f.n) - p.luma as f32).abs() < 2.0 {
                bd = d;
                best = Some(lamp.ambient);
            }
        }
    }
    let amb = best?;
    Some(if p.luma as f32 <= amb + 0.5 { 1.0 } else { ((amb + 0.5) / (p.luma as f32 + 0.5)).clamp(0.0, 1.0) })
}

fn cast_normals(c: &Ctx) -> HashMap<u32, V3> {
    let mut m = HashMap::new();
    for ca in c.casts.iter().filter(|ca| ca.survived) {
        let Some((fx, fy)) = c.side.blocks.get(ca.block as usize).and_then(|b| b.focal()) else { continue };
        if ca.verts.len() < 3 {
            continue;
        }
        let v: Vec<V3> = ca.verts.iter().map(|p| [p[0] / fx, p[1] / fy, p[2]]).collect();
        let n = cross(sub(v[1], v[0]), sub(v[2], v[0]));
        if dot(n, n) > 1e-20 {
            m.insert(ca.at, norm(n));
        }
    }
    m
}

/// Position and normal per pixel for a polygon set (as `lighting_spike`).
fn build_g(c: &Ctx, polys: &[Poly], placed: &[Placed], vis: &raster::Visible, cn: &HashMap<u32, V3>) -> G {
    let (w, h) = (c.g.w, c.g.h);
    let cx_of: HashMap<u32, (f32, f32)> = c.casts.iter().filter(|ca| ca.survived).map(|ca| (ca.at, (ca.cx, ca.cy))).collect();
    let mut g = G { w, h, pos: vec![[0.0; 3]; w * h], nrm: vec![[0.0; 3]; w * h], elig: vec![false; w * h], cat: vec![0; w * h], block: vec![0; w * h], win: vec![0; w * h] };
    for o in 0..w * h {
        let k = vis.owner[o];
        if k == u32::MAX {
            continue;
        }
        let p = &polys[placed[k as usize].poly];
        g.cat[o] = 1;
        g.block[o] = p.block;
        g.win[o] = p.window as u8;
        if c.hud_mask & (1 << p.window) != 0 {
            g.cat[o] = 5;
            continue;
        }
        match raster::background_reason(p) {
            Some("sky") => g.cat[o] = 2,
            Some("shadow") => g.cat[o] = 3,
            Some("checker") => g.cat[o] = 4,
            Some("spread") => g.cat[o] = 6,
            _ => {}
        }
        if matches!(g.cat[o], 2..=4) {
            continue;
        }
        let (Some(&n), Some(&(cx, cy)), Some((fx, fy))) = (cn.get(&p.at), cx_of.get(&p.at), c.side.blocks.get(p.block as usize).and_then(|b| b.focal())) else { continue };
        let iz = vis.iz[o];
        if iz <= 0.0 {
            continue;
        }
        let z = 1.0 / iz;
        let (sx, sy) = (o % w, o / w);
        let pos = [((sx as f32 + 0.5) - cx) * z / fx, (cy - (sy as f32 + 0.5)) * z / fy, z];
        g.nrm[o] = if dot(n, pos) > 0.0 { [-n[0], -n[1], -n[2]] } else { n };
        g.pos[o] = pos;
        g.elig[o] = true;
    }
    g
}

fn n_px(c: &Ctx) -> usize {
    c.g.w * c.g.h
}

fn is_blob(p: &Poly) -> bool {
    matches!(raster::background_reason(p), Some("shadow" | "checker"))
}

struct Snap {
    mask: Vec<u8>,
    keys: BTreeSet<u64>,
    ntris: usize,
    shadowed: usize,
}

thread_local! {
    static HIST: RefCell<Vec<Snap>> = const { RefCell::new(Vec::new()) };
}

fn zoom(img: &[u8], w: usize, crop: (usize, usize, usize, usize), k: usize) -> Vec<u8> {
    let (x0, y0, cw, ch) = crop;
    let mut out = Vec::with_capacity(cw * ch * k * k * 3);
    for y in 0..ch * k {
        for x in 0..cw * k {
            let o = ((y0 + y / k) * w + x0 + x / k) * 3;
            out.extend_from_slice(&img[o..o + 3]);
        }
    }
    out
}

pub fn run(c: &Ctx) {
    let (w, h) = (c.g.w, c.g.h);
    let tag = &c.sc.tag;
    let res = env("LB_RES", 2048.0) as usize;
    let casc: Vec<f32> = std::env::var("LB_CASC").unwrap_or_else(|_| "0,1e9".into()).split(',').filter_map(|s| s.parse().ok()).collect();
    let pool = env("LB_POOL", 1.0) != 0.0;
    let dk = env("LB_DK", 0.55);
    let g = c.g;
    let n = w * h;
    let hud_note = c.hud_mask;
    println!("[{tag}] B: hud mask {hud_note:#x}, res {res}, cascades {casc:?}, pool {pool}");

    // ---- 0. blob shadows by rule: flat black or checker polygons (the build's
    // own tests), and cut-out textured polygons whose shown pixels are all
    // near-black (a shaped blob texture)
    let vis0 = raster::visible(c.polys, c.placed, c.src, c.view);
    let mut shown = vec![(0usize, 0usize); c.placed.len()];
    for o in 0..n_px(c) {
        let k = vis0.owner[o];
        if k != u32::MAX && c.layer.1[o] {
            shown[k as usize].0 += 1;
            shown[k as usize].1 += c.layer.0[o * 3..o * 3 + 3].iter().all(|&v| v <= 12) as usize;
        }
    }
    let mut why = [0usize; 3];
    let blob_flag: Vec<bool> = (0..c.placed.len())
        .map(|k| {
            let p = &c.polys[c.placed[k].poly];
            let dark = p.texture.as_ref().is_some_and(|t| t.translucent) && shown[k].0 >= 8 && shown[k].1 * 100 >= shown[k].0 * 95;
            let rule = is_blob(p);
            why[0] += rule as usize;
            why[1] += (dark && !rule) as usize;
            rule || dark
        })
        .collect();
    let blob_at: BTreeSet<u32> = (0..c.placed.len()).filter(|&k| blob_flag[k]).map(|k| c.polys[c.placed[k].poly].at).collect();
    println!("[{tag}] B4 blob rule: flat black or checker polygons {}, cut-out textured near-black polygons {}", why[0], why[1]);
    // ---- 1. casters
    let t0 = Instant::now();
    let (t_solid, ns) = casters(c, Mode::Solid, &blob_at);
    let (t_nocut, _) = casters(c, Mode::NoCut, &blob_at);
    let (t_alpha, na) = casters(c, Mode::Alpha, &blob_at);
    println!(
        "[{tag}] B1 casters: solid {} tris, no cut-outs {}, alpha-tested {} (polygons {} sampled {}, culled solid faces {}, cut-outs dropped {}) ({:.0} ms)",
        t_solid.len(),
        t_nocut.len(),
        t_alpha.len(),
        na[0],
        na[3],
        na[1],
        na[2],
        t0.elapsed().as_secs_f64() * 1e3
    );
    let _ = ns;
    let one = |tris: &[Tri], pool: bool, casc: &[f32], res: usize, shift: f32| {
        let pl = plan(c, g, tris, pool, casc, res);
        shadow_all(c, g, tris, &pl, shift)
    };
    let cnt = |o: &Out| -> (usize, usize) {
        let r = (0..n).filter(|&i| o.recv[i]).count();
        (r, (0..n).filter(|&i| o.recv[i] && o.f[i] < 0.5).count())
    };
    let nolimit = [0.0, 1e9];
    for (name, tris) in [("solid casters", &t_solid), ("no cut-outs", &t_nocut), ("alpha-tested", &t_alpha)] {
        let t0 = Instant::now();
        let o = one(tris, false, &nolimit, res, 0.0);
        let (r, s) = cnt(&o);
        println!("[{tag}] B1 {name} (per-window maps, one cascade): receivers {r}, shadowed {s} = {:.1}% ({:.0} ms)", pct(s, r), t0.elapsed().as_secs_f64() * 1e3);
    }

    // ---- 2. windows
    let pw = one(&t_alpha, false, &nolimit, res, 0.0);
    let pp = one(&t_alpha, true, &nolimit, res, 0.0);
    for i in &pp.info {
        println!("[{tag}] B2 pooled: {i}");
    }
    for i in &pw.info {
        println!("[{tag}] B2 per-window: {i}");
    }
    let mut per_win: BTreeMap<usize, (usize, usize, usize)> = BTreeMap::new();
    for o in 0..n {
        if pp.recv[o] || pw.recv[o] {
            let e = per_win.entry(g.win[o] as usize).or_default();
            e.0 += 1;
            e.1 += (pw.f[o] < 0.5) as usize;
            e.2 += (pp.f[o] < 0.5) as usize;
        }
    }
    println!("[{tag}] B2 receivers per window (receivers, shadowed per-window maps, shadowed pooled): {per_win:?}");

    {
        let bb = |it: &mut dyn Iterator<Item = V3>| {
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for p in it {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            format!("x {:.1}..{:.1} y {:.1}..{:.1} z {:.1}..{:.1}", lo[0], hi[0], lo[1], hi[1], lo[2], hi[2])
        };
        for wi in t_alpha.iter().map(|t| t.win).collect::<BTreeSet<_>>() {
            let nt = t_alpha.iter().filter(|t| t.win == wi).count();
            println!(
                "[{tag}] B2 window {wi}: {nt} caster tris, bounds {}; receivers bounds {}",
                bb(&mut t_alpha.iter().filter(|t| t.win == wi).flat_map(|t| t.v)),
                bb(&mut (0..n).filter(|&o| g.elig[o] && g.win[o] as usize == wi).map(|o| g.pos[o]))
            );
        }
    }
    if let Some(big) = per_win.iter().max_by_key(|e| e.1 .0).map(|e| *e.0) {
        // Floor window `big` as receivers, the other windows' casters only.
        let others: Vec<Tri> = t_alpha.iter().filter(|t| t.win != big).cloned().collect();
        let o = one(&others, true, &nolimit, res, 0.0);
        let sh = (0..n).filter(|&i| g.win[i] as usize == big && o.recv[i] && o.f[i] < 0.5).count();
        println!("[{tag}] B2 receivers of window {big} shadowed by the other windows' casters alone: {sh}");
    }
    {
        // Ground truth by brute force: a ray from each sampled floor receiver
        // (window with most receivers among those with few casters) toward the
        // light against every caster triangle, against the map's answer.
        let (mut hit_ray, mut hit_map, mut tot, mut hit_both) = (0usize, 0usize, 0usize, 0usize);
        let floor_win = per_win.iter().filter(|e| e.1 .0 > 1000).min_by_key(|e| t_alpha.iter().filter(|t| t.win == *e.0).count()).map(|e| *e.0);
        if let (Some(fw), Some((_, l))) = (floor_win, pp_groups(&pp_plan_groups(c, g, &t_alpha), floor_win.unwrap_or(0))) {
            for o in (0..n).filter(|&o| g.win[o] as usize == fw && pp.recv[o]).step_by(7) {
                tot += 1;
                let p = g.pos[o];
                let mut hit = false;
                for t in t_alpha.iter().filter(|t| t.win != fw || true) {
                    let (e1, e2) = (sub(t.v[1], t.v[0]), sub(t.v[2], t.v[0]));
                    let pv = cross(l, e2);
                    let det = dot(e1, pv);
                    if det.abs() < 1e-9 {
                        continue;
                    }
                    let tv = sub(p, t.v[0]);
                    let u = dot(tv, pv) / det;
                    let qv = cross(tv, e1);
                    let v = dot(l, qv) / det;
                    let tt = dot(e2, qv) / det;
                    if u >= 0.0 && v >= 0.0 && u + v <= 1.0 && tt > 0.03 {
                        hit = true;
                        break;
                    }
                }
                let m = pp.f[o] < 0.5;
                hit_ray += hit as usize;
                hit_map += m as usize;
                hit_both += (hit && m) as usize;
            }
            println!("[{tag}] B2 brute-force check on window {fw} floor samples: {tot} rays, hit a caster {hit_ray}, the map says shadowed {hit_map}, both {hit_both}");
        }
    }
    // Do two windows share a view space? Window A's floor plane (the common
    // normal of its lowest pixels) against the lowest pixels (the feet) of B.
    {
        let wins: BTreeSet<usize> = (0..n).filter(|&o| g.elig[o]).map(|o| g.win[o] as usize).collect();
        for &a in &wins {
            let pa: Vec<usize> = (0..n).filter(|&o| g.elig[o] && g.win[o] as usize == a && o / w > h * 2 / 3).collect();
            if pa.len() < 500 {
                continue;
            }
            let mut hist: HashMap<[i32; 3], usize> = HashMap::new();
            for &o in &pa {
                *hist.entry(g.nrm[o].map(|v| (v * 20.0).round() as i32)).or_default() += 1;
            }
            let (&k, &cnt) = hist.iter().max_by_key(|e| e.1).unwrap();
            let nn = norm(k.map(|v| v as f32 / 20.0));
            let mut ds: Vec<f32> = pa.iter().filter(|&&o| dot(g.nrm[o], nn) > 0.98).map(|&o| dot(g.pos[o], nn)).collect();
            ds.sort_by(|x, y| x.total_cmp(y));
            let d0 = ds[ds.len() / 2];
            let spread = ds[ds.len() * 9 / 10] - ds[ds.len() / 10];
            for &b in &wins {
                if b == a {
                    continue;
                }
                let mut pb: Vec<usize> = (0..n).filter(|&o| g.elig[o] && g.win[o] as usize == b).collect();
                if pb.len() < 50 {
                    continue;
                }
                pb.sort_by_key(|&o| std::cmp::Reverse(o / w));
                let mut dd: Vec<f32> = pb.iter().take(40).map(|&o| dot(g.pos[o], nn) - d0).collect();
                dd.sort_by(|x, y| x.total_cmp(y));
                println!(
                    "[{tag}] B2 view-space test: floor of window {a} (normal ({:.2},{:.2},{:.2}), {cnt} px, plane spread {spread:.3}) vs the lowest 40 px of window {b} (screen rows {}..{}): signed distance median {:.3}, range {:.3}..{:.3}",
                    nn[0], nn[1], nn[2], pb[39.min(pb.len() - 1)] / w, pb[0] / w, dd[dd.len() / 2], dd[0], dd[dd.len() - 1]
                );
            }
        }
    }
    // ---- 3. cascades / limit
    let main = one(&t_alpha, pool, &casc, res, 0.0);
    for i in &main.info {
        println!("[{tag}] B3 {i}");
    }
    let (r, s) = cnt(&main);
    println!("[{tag}] B3 chosen plan: receivers {r}, shadowed {s} = {:.1}%; texel/pixel footprint (receiver mean) {:.2}", pct(s, r), main.ratio);
    if std::env::var_os("LB_CRAWL").is_some() {
        let zb: Vec<f32> = std::env::var("LB_CRAWLC").unwrap_or_else(|_| "0,1e9".into()).split(',').filter_map(|s| s.parse().ok()).collect();
        for (r2, cz) in [(1024usize, &nolimit[..]), (2048, &nolimit[..]), (4096, &nolimit[..]), (2048, &zb[..]), (4096, &zb[..])] {
            let a = one(&t_alpha, pool, cz, r2, 0.0);
            let b = one(&t_alpha, pool, cz, r2, 0.5);
            let sh = (0..n).filter(|&i| a.recv[i] && a.f[i] < 0.5).count();
            let fl = (0..n).filter(|&i| a.recv[i] && (a.f[i] < 0.5) != (b.f[i] < 0.5)).count();
            println!("[{tag}] B3 crawl: map {r2}, cascades {cz:?}: shadowed {sh}, flipped by a half-texel shift {fl} ({:.2}% of shadowed), texel/pixel {:.2}", pct(fl, sh), a.ratio);
        }
    }

    // ---- 4/5. blobs and the term
    let cn = cast_normals(c);
    let blob_polys: Vec<usize> = (0..c.placed.len()).filter(|&k| blob_flag[k]).collect();
    let mut polys2: Vec<Poly> = c.polys.iter().enumerate().filter(|(i, _)| !blob_flag[c.placed.iter().position(|pl| pl.poly == *i).unwrap_or(0)]).map(|(_, p)| p.clone()).collect();
    let placed2 = raster::place(&mut polys2, c.view);
    let vis2 = raster::visible(&polys2, &placed2, c.src, c.view);
    let g2 = build_g(c, &polys2, &placed2, &vis2, &cn);
    let c2 = Ctx { g: &g2, ..*c };
    let layer2 = raster::draw_view_depth(&mut polys2.clone(), c.src, c.view, false);
    let sh2 = {
        let pl = plan(&c2, &g2, &t_alpha, pool, &casc, res);
        shadow_all(&c2, &g2, &t_alpha, &pl, 0.0)
    };
    // The ratio per polygon (placed index of polys2).
    let mut matched = (0usize, 0usize);
    let ratio2: Vec<f32> = placed2
        .iter()
        .map(|pl| {
            let p = &polys2[pl.poly];
            match cn.get(&p.at).and_then(|&nn| game_ratio(c, p, nn)) {
                Some(r) => {
                    matched.0 += 1;
                    r
                }
                None => {
                    matched.1 += 1;
                    1.0 - dk
                }
            }
        })
        .collect();
    println!("[{tag}] B5 game-formula ratio found for {} polygons, constant {} used for {}", matched.0, 1.0 - dk, matched.1);
    // Blobs: the share of each one's pixels the real shadow would cover.
    let blob_t = env("LB_BLOBT", 0.5);
    let mut keep_px = vec![false; n];
    let by_block_win: Vec<(u32, usize)> = blob_polys.iter().map(|&k| (c.polys[c.placed[k].poly].block, c.polys[c.placed[k].poly].window)).collect::<BTreeSet<_>>().into_iter().collect();
        for (b, win) in &by_block_win {
        let px: Vec<usize> = (0..n)
            .filter(|&o| {
                let k = vis0.owner[o];
                k != u32::MAX && blob_flag[k as usize] && c.polys[c.placed[k as usize].poly].block == *b && c.polys[c.placed[k as usize].poly].window == *win
            })
            .collect();
        let covered = px.iter().filter(|&&o| g2.elig[o] && sh2.f[o] < 0.5).count();
        let ground = px.iter().filter(|&&o| g2.elig[o]).count();
        let share = covered as f64 / px.len().max(1) as f64;
        let blk = &c.side.blocks[*b as usize];
        let hide = share >= blob_t as f64;
        println!(
            "[{tag}] B4 blob block {b} window {win} oba {:06x} key {:016x}: {} px shown, ground behind it known for {ground}, real shadow covers {covered} ({:.0}%): {}",
            blk.oba, blk.key, px.len(), 100.0 * share, if hide { "hidden" } else { "kept" }
        );
        if !hide {
            for &o in &px {
                keep_px[o] = true;
            }
        }
    }
    // Compose: the blob-less layer with kept blobs put back; the term.
    let mut l_const = c.layer.0.clone();
    let mut l_game = layer2.0.clone();
    let mut mask_game = layer2.1.clone();
    for o in 0..n {
        if keep_px[o] {
            l_game[o * 3..o * 3 + 3].copy_from_slice(&c.layer.0[o * 3..o * 3 + 3]);
            mask_game[o] = c.layer.1[o];
        }
    }
    let mut tgrey = vec![[255u8, 255, 255]; n];
    let mut dark_const = 0.0f64;
    let mut dark_game = (0.0f64, 0usize);
    for o in 0..n {
        // constant, original polygons (blobs kept)
        if g.elig[o] && main.f[o] < 1.0 {
            let t = 1.0 - dk * (1.0 - main.f[o]);
            for ch in 0..3 {
                l_const[o * 3 + ch] = (l_const[o * 3 + ch] as f32 * t) as u8;
            }
        }
        if g2.elig[o] && !keep_px[o] {
            let k = vis2.owner[o] as usize;
            let r = ratio2[k];
            let t = 1.0 - (1.0 - r) * (1.0 - sh2.f[o]);
            tgrey[o] = grey(t);
            if sh2.f[o] < 0.5 {
                dark_game.0 += r as f64;
                dark_game.1 += 1;
            }
            for ch in 0..3 {
                l_game[o * 3 + ch] = (l_game[o * 3 + ch] as f32 * t) as u8;
            }
        } else if g2.cat[o] == 0 || !g2.elig[o] {
            tgrey[o] = [200, 200, 255];
        }
        if g.elig[o] && main.f[o] < 0.5 {
            dark_const += (1.0 - dk) as f64;
        }
    }
    let _ = dark_const;
    println!(
        "[{tag}] B5 mean brightness factor in shadow: constant {:.2}; the game's formula {:.2} over {} shadowed px",
        1.0 - dk,
        dark_game.0 / dark_game.1.max(1) as f64,
        dark_game.1
    );
    let img_const = video::compose(&c.cap.tiles, &c.cap.palette, Some((&l_const, &c.layer.1)), c.view);
    let img_game = video::compose(&c.cap.tiles, &c.cap.palette, Some((&l_game, &mask_game)), c.view);
    let tg: Vec<u8> = tgrey.iter().flatten().copied().collect();
    c.sc.strip("b-shadow", w, h, &[c.orig, &img_const, &img_game, &tg]);
    if let Ok(cr) = std::env::var("LB_CROP") {
        let v: Vec<usize> = cr.split(',').filter_map(|s| s.parse().ok()).collect();
        if v.len() == 4 && v[0] + v[2] <= w && v[1] + v[3] <= h {
            let rows: Vec<Vec<u8>> = [c.orig, &img_const, &img_game].iter().map(|im| zoom(im, w, (v[0], v[1], v[2], v[3]), 4)).collect();
            let all: Vec<u8> = rows.iter().flatten().copied().collect();
            let _ = std::fs::write(c.sc.out.join(format!("{tag}-b-crop.png")), png::encode_rgb(v[2] * 4, v[3] * 4 * 3, &all));
        }
    }

    // ---- 6. stability against the frames before
    let mask: Vec<u8> = (0..n).map(|o| if !g.elig[o] { 0 } else if main.f[o] < 0.5 { 2 } else { 1 }).collect();
    let keys: BTreeSet<u64> = t_alpha.iter().map(|t| t.key).collect();
    let shadowed = mask.iter().filter(|&&m| m == 2).count();
    HIST.with(|h| {
        let mut h = h.borrow_mut();
        let mut line = format!("shadowed px {shadowed}, caster tris {}, caster models {}", t_alpha.len(), keys.len());
        if let Some(p) = h.last() {
            let both = (0..n).filter(|&o| mask[o] != 0 && p.mask[o] != 0).count();
            let fl = (0..n).filter(|&o| mask[o] != 0 && p.mask[o] != 0 && mask[o] != p.mask[o]).count();
            line += &format!("; vs previous: models in {} out {}, tris {:+}, shadowed {:+}, receivers in both {both} changed {fl} ({:.1}%)", keys.difference(&p.keys).count(), p.keys.difference(&keys).count(), t_alpha.len() as i64 - p.ntris as i64, shadowed as i64 - p.shadowed as i64, pct(fl, both));
        }
        if h.len() >= 2 {
            let (p1, p2) = (&h[h.len() - 1], &h[h.len() - 2]);
            let flick = (0..n).filter(|&o| mask[o] != 0 && p1.mask[o] != 0 && p2.mask[o] != 0 && p2.mask[o] == mask[o] && p1.mask[o] != mask[o]).count();
            line += &format!("; flicker (shadow state of the previous frame differs from both its neighbours) {flick} px ({:.2}% of shadowed)", pct(flick, shadowed));
        }
        println!("[{tag}] B6 {line}");
        h.push(Snap { mask, keys, ntris: t_alpha.len(), shadowed });
    });
}
