//! SPIKE (scatter) `remodel uplift-scatter MODE FROM COUNT OUT OBA`: many small
//! instances (crossed-quad grass tufts with a generated RGBA cut-out, or tiny
//! solid pebbles) stood on the polygons of ONE ground model, placed from the
//! model's own polygon data with a fixed seed, drawn from the side channel's
//! block transform in software (and, with SC_GPU, on the wgpu renderer).
//! MODE: list (which models are on screen, and a model's polygons) | run.
//! Env: SC_N, SC_KIND=tuft|pebble, SC_H, SC_W, SC_SEED, SC_CLASS=all|green|HEX,..,
//! SC_SLOPE (deg), SC_MARGIN, SC_FADE=z0,z1, SC_GPU, SC_GPU_N, SC_RMIN (px).
use crate::uplift_mesh::*;
use model2::capture;
use model2::enhance::Rgba;
use model2::geo::Subst;
use model2::light::{Base, Block, Lamp};
use model2::raster::{self, Poly, Sources};
use model2::uplift::{self, Extra, Instance, KeyMode, Mesh, Skin};
use model2::video::{self, View};
use model2::Machine;
use romset::RomSet;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::Instant;

fn env_f(k: &str, d: f32) -> f32 {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}

fn splitmix(x: &mut u64) -> u64 {
    *x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}
fn rnd(x: &mut u64) -> f32 {
    (splitmix(x) >> 40) as f32 / (1u64 << 24) as f32
}

type V3 = [f32; 3];
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn mul(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn unit(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    if l > 1e-12 {
        mul(a, 1.0 / l)
    } else {
        a
    }
}

/// One polygon of a model: vertices in the game's order [p1, p0, pa, (pb)].
struct MPoly {
    v: Vec<V3>,
    /// The model's own normal word (non-compact models): points away from the eye.
    stored: Option<V3>,
    attr: u32,
}

fn link_next(attr: u32, p0: V3, p1: V3, pa: V3, pb: V3) -> (V3, V3) {
    match (attr >> 8) & 3 {
        1 => (p0, pa),
        3 => (pb, p1),
        _ => (pa, pb),
    }
}

fn parse_model(w: &[u32], compact: bool) -> Vec<MPoly> {
    let v = |i: usize| -> V3 {
        let x = if compact { f32::from_bits(w[i] & !1) } else { f32::from_bits(w[i]) };
        [x, f32::from_bits(w[i + 1]), f32::from_bits(w[i + 2])]
    };
    let mut out = Vec::new();
    if w.len() < 6 {
        return out;
    }
    let (mut p0, mut p1) = (v(0), v(3));
    let mut i = 6;
    while i + 4 <= w.len() {
        let attr = w[i];
        if attr & 3 == 0 {
            break;
        }
        let quad = attr & 1 != 0;
        let first = if compact { i + 1 } else { i + 4 };
        let len = if compact { 1 + if quad { 6 } else { 3 } } else { 10 };
        if first + if quad { 6 } else { 3 } > w.len() {
            break;
        }
        let pa = v(first);
        let pb = if quad { v(first + 3) } else { pa };
        let stored = (!compact).then(|| [f32::from_bits(w[i + 1]), f32::from_bits(w[i + 2]), f32::from_bits(w[i + 3])]);
        out.push(MPoly { v: if quad { vec![p1, p0, pa, pb] } else { vec![p1, p0, pa] }, stored, attr });
        (p0, p1) = link_next(attr, p0, p1, pa, pb);
        i += len;
    }
    out
}

/// The same polygons in the game's stream (view space x*fx, y*fy, z), and
/// each record's index in the stream.
fn stream_polys(s: &[u32], at: usize) -> (Vec<usize>, Vec<Vec<V3>>) {
    let v = |i: usize| [f24(s[i]), f24(s[i + 1]), f24(s[i + 2])];
    let (mut recs, mut polys) = (Vec::new(), Vec::new());
    if at + 9 > s.len() {
        return (recs, polys);
    }
    let (mut p0, mut p1) = (v(at + 3), v(at + 6));
    let mut i = at + 9;
    while i + 6 <= s.len() && s[i] >> 24 != 0xff && s[i] & 3 != 0 {
        let attr = s[i];
        let quad = attr & 1 != 0;
        if quad && i + 9 > s.len() {
            break;
        }
        let pa = v(i + 3);
        let pb = if quad { v(i + 6) } else { pa };
        recs.push(i);
        polys.push(if quad { vec![p1, p0, pa, pb] } else { vec![p1, p0, pa] });
        (p0, p1) = link_next(attr, p0, p1, pa, pb);
        i += if quad { 9 } else { 6 };
    }
    (recs, polys)
}

struct Inst {
    poly: usize,
    corner: [usize; 3],
    w: [f32; 3],
    p: V3,
    n: V3,
    h: f32,
}

struct Scatter {
    mesh: Mesh,
    per: usize,
    vper: usize,
    inst: Vec<Inst>,
    up: V3,
    classes: Vec<(u64, [u32; 3], usize, f32)>,
    eligible: usize,
}

fn header_hash(h: [u16; 4]) -> u64 {
    h.iter().fold(0xcbf2_9ce4_8422_2325u64, |a, &x| (a ^ x as u64).wrapping_mul(0x100_0000_01b3))
}

fn area(v: &[V3]) -> f32 {
    let mut a = 0.0;
    for k in 1..v.len() - 1 {
        a += 0.5 * dot(cross(sub(v[k], v[0]), sub(v[k + 1], v[0])), cross(sub(v[k], v[0]), sub(v[k + 1], v[0]))).sqrt();
    }
    a
}

/// Visible-side unit normal of a polygon (model space): the stored normal's
/// negation where the model has one, else the geometric one as wound.
fn poly_normal(p: &MPoly) -> V3 {
    let g = cross(sub(p.v[1], p.v[0]), sub(p.v[2], p.v[0]));
    let g = if p.v.len() == 4 {
        add(g, cross(sub(p.v[2], p.v[0]), sub(p.v[3], p.v[0])))
    } else {
        g
    };
    let g = unit(g);
    match p.stored {
        Some(s) => {
            if dot(g, s) > 0.0 {
                mul(g, -1.0)
            } else {
                g
            }
        }
        None => g,
    }
}

fn tuft_texture(w: usize, h: usize) -> Rgba {
    let mut data = vec![0u8; w * h * 4];
    let blades = 9usize;
    let mut s = 12345u64;
    let bl: Vec<(f32, f32, f32, f32, [f32; 3])> = (0..blades)
        .map(|b| {
            let base = 0.12 + 0.76 * (b as f32 + 0.5 * rnd(&mut s)) / blades as f32;
            let lean = (rnd(&mut s) - 0.5) * 0.45;
            let top = 0.55 + 0.45 * rnd(&mut s);
            let hw = 0.035 + 0.02 * rnd(&mut s);
            let g = 0.55 + 0.45 * rnd(&mut s);
            (base, lean, top, hw, [90.0 * g, 170.0 * g, 50.0 * g])
        })
        .collect();
    for y in 0..h {
        let vv = 1.0 - (y as f32 + 0.5) / h as f32; // 0 at the root, 1 at the top
        for x in 0..w {
            let u = (x as f32 + 0.5) / w as f32;
            let o = (y * w + x) * 4;
            for (base, lean, top, hw, col) in bl.iter().rev() {
                if vv > *top {
                    continue;
                }
                let t = vv / top;
                let cx = base + lean * t * t;
                let half = hw * (1.0 - t).max(0.0).powf(0.8) + 0.004;
                if (u - cx).abs() < half {
                    let shade = 0.55 + 0.6 * t;
                    data[o] = (col[0] * shade).min(255.0) as u8;
                    data[o + 1] = (col[1] * shade).min(255.0) as u8;
                    data[o + 2] = (col[2] * shade).min(255.0) as u8;
                    data[o + 3] = 255;
                }
            }
        }
    }
    Rgba { width: w, height: h, data }
}

fn pebble_texture(n: usize) -> Rgba {
    let mut data = vec![255u8; n * n * 4];
    let mut s = 77u64;
    for i in 0..n * n {
        let g = 105 + (rnd(&mut s) * 55.0) as u8;
        data[i * 4] = g;
        data[i * 4 + 1] = g - 6;
        data[i * 4 + 2] = g - 16;
    }
    Rgba { width: n, height: n, data }
}

/// Class selection: all, green (g > r and g > b in the palette colour), or hash list.
fn class_ok(sel: &str, hash: u64, c: [u32; 3]) -> bool {
    match sel {
        "all" => true,
        "green" => c[1] > c[0] && c[1] > c[2],
        l => l.split(',').any(|h| u64::from_str_radix(h.trim_start_matches("0x"), 16).is_ok_and(|x| x == hash & 0xffff_ffff)),
    }
}

#[allow(clippy::too_many_arguments)]
fn build(
    mp: &[MPoly],
    hdrs: &[([u16; 4], [u32; 3])],
    kind: &str,
    n: usize,
    seed: u64,
    h0: f32,
    w0: f32,
    sel: &str,
    slope_deg: f32,
    margin: f32,
    ref_up: Option<V3>,
    accept: &dyn Fn(V3) -> bool,
) -> Scatter {
    let normals: Vec<V3> = mp
        .iter()
        .map(|p| {
            let n = poly_normal(p);
            match ref_up {
                // Compact models have no stored normal: the winding says nothing of the visible side, so
                // every polygon's normal is turned to the side of the reference polygon's.
                Some(r) if p.stored.is_none() && dot(n, r) < 0.0 => mul(n, -1.0),
                _ => n,
            }
        })
        .collect();
    let areas: Vec<f32> = mp.iter().map(|p| area(&p.v)).collect();
    // The model's "up": the area-weighted mean visible-side normal of the selected class.
    let mut up = [0.0; 3];
    for (j, p) in mp.iter().enumerate() {
        if class_ok(sel, header_hash(hdrs[j].0), hdrs[j].1) {
            up = add(up, mul(normals[j], areas[j]));
        }
        let _ = p;
    }
    let up = unit(up);
    let cos_lim = slope_deg.to_radians().cos();
    let mut cum = Vec::new();
    let mut tot = 0.0;
    let mut classes: HashMap<u64, ([u32; 3], usize, f32)> = HashMap::new();
    let mut eligible = 0;
    for (j, _) in mp.iter().enumerate() {
        let ok = class_ok(sel, header_hash(hdrs[j].0), hdrs[j].1) && dot(normals[j], up) >= cos_lim && areas[j] > 0.0;
        if ok {
            tot += areas[j];
            eligible += 1;
        }
        cum.push(tot);
        let e = classes.entry(header_hash(hdrs[j].0)).or_insert((hdrs[j].1, 0, 0.0));
        e.1 += 1;
        e.2 += areas[j];
    }
    let mut cl: Vec<(u64, [u32; 3], usize, f32)> = classes.into_iter().map(|(k, v)| (k, v.0, v.1, v.2)).collect();
    cl.sort_by(|a, b| b.3.total_cmp(&a.3));
    let mut sc = Scatter {
        mesh: Mesh { v: vec![], n: vec![], uv: vec![], tris: vec![] },
        per: if kind == "pebble" { 8 } else { 4 },
        vper: if kind == "pebble" { 6 } else { 8 },
        inst: vec![],
        up,
        classes: cl,
        eligible,
    };
    if tot <= 0.0 {
        return sc;
    }
    let mut rs = seed;
    for _ in 0..n {
        // Polygon by area, then a point on it away from its edges (bounded retries).
        let mut tries = 0;
        let found = loop {
            tries += 1;
            let t = rnd(&mut rs) * tot;
            let j = cum.partition_point(|&c| c <= t).min(mp.len() - 1);
            let nv = mp[j].v.len();
            // A quad is two triangles (0,1,2) (0,2,3), by area.
            let tri = if nv == 4 {
                let a0 = area(&[mp[j].v[0], mp[j].v[1], mp[j].v[2]]);
                if rnd(&mut rs) * areas[j] < a0 {
                    [0, 1, 2]
                } else {
                    [0, 2, 3]
                }
            } else {
                [0, 1, 2]
            };
            let (mut a, mut b) = (rnd(&mut rs), rnd(&mut rs));
            if a + b > 1.0 {
                a = 1.0 - a;
                b = 1.0 - b;
            }
            let w = [1.0 - a - b, a, b];
            let p = {
                let vs = |c: usize| mp[j].v[tri[c]];
                add(add(mul(vs(0), w[0]), mul(vs(1), w[1])), mul(vs(2), w[2]))
            };
            if w.iter().cloned().fold(1.0, f32::min) >= margin && accept(p) {
                break Some((j, tri, w));
            }
            if tries >= 4000 {
                break None;
            }
        };
        let Some((j, corner, w)) = found else { continue };
        let vs = |c: usize| mp[j].v[corner[c]];
        let p = add(add(mul(vs(0), w[0]), mul(vs(1), w[1])), mul(vs(2), w[2]));
        let nn = normals[j];
        let t1 = unit(cross(nn, if nn[0].abs() < 0.8 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] }));
        let t2 = cross(nn, t1);
        let ang = rnd(&mut rs) * std::f32::consts::TAU;
        let (ca, sa) = (ang.cos(), ang.sin());
        let (a1, a2) = (add(mul(t1, ca), mul(t2, sa)), add(mul(t1, -sa), mul(t2, ca)));
        let scale = 0.7 + 0.6 * rnd(&mut rs);
        let (h, wd) = (h0 * scale, w0 * scale);
        let base = sc.mesh.v.len() as u32;
        if kind == "pebble" {
            let c = add(p, mul(nn, h * 0.4));
            let ax = [mul(a1, h), mul(nn, h * 0.6), mul(a2, h * 0.8)];
            let dirs = [mul(a1, 1.0), mul(a1, -1.0), nn, mul(nn, -0.2), mul(a2, 1.0), mul(a2, -1.0)];
            let _ = ax;
            for d in dirs {
                let q = [dot(d, a1), dot(d, nn), dot(d, a2)];
                let off = add(add(mul(a1, q[0] * h), mul(nn, q[1] * h * 0.6)), mul(a2, q[2] * h * 0.8));
                sc.mesh.v.push(add(c, off));
                sc.mesh.n.push(unit(d));
                sc.mesh.uv.push([0.5 + 0.5 * q[0], 0.5 + 0.5 * q[2]]);
            }
            // Octahedron: +a1 -a1 up down +a2 -a2 = 0..5.
            for (a, b, cc) in [(0, 2, 4), (4, 2, 1), (1, 2, 5), (5, 2, 0), (4, 3, 0), (1, 3, 4), (5, 3, 1), (0, 3, 5)] {
                let (va, vb, vc) = (sc.mesh.v[(base + a) as usize], sc.mesh.v[(base + b) as usize], sc.mesh.v[(base + cc) as usize]);
                let g = cross(sub(vb, va), sub(vc, va));
                let mid = mul(add(add(va, vb), vc), 1.0 / 3.0);
                if dot(g, sub(mid, c)) >= 0.0 {
                    sc.mesh.tris.push([base + a, base + b, base + cc]);
                } else {
                    sc.mesh.tris.push([base + a, base + cc, base + b]);
                }
            }
        } else {
            for dir in [a1, a2] {
                let q = sc.mesh.v.len() as u32;
                for (u, vv, side, hh) in [(0.0, 1.0, -1.0, 0.0), (1.0, 1.0, 1.0, 0.0), (1.0, 0.0, 1.0, 1.0), (0.0, 0.0, -1.0, 1.0)] {
                    sc.mesh.v.push(add(add(p, mul(dir, side * wd * 0.5)), mul(nn, hh * h)));
                    // Lit as the ground: the visible-side normal (the driver negates it as the game's convention needs).
                    sc.mesh.n.push(nn);
                    sc.mesh.uv.push([u, vv]);
                }
                sc.mesh.tris.push([q, q + 1, q + 2]);
                sc.mesh.tris.push([q, q + 2, q + 3]);
            }
        }
        sc.inst.push(Inst { poly: j, corner, w, p, n: nn, h });
    }
    sc
}

/// The visible side of the largest polygon of the class, seen from the eye in this frame.
fn eye_ref(mp: &[MPoly], hd: &[([u16; 4], [u32; 3])], sel: &str, b: &Block) -> Option<V3> {
    // Only a polygon wholly in front of the eye says which side faces it.
    let front = |p: &MPoly| p.v.iter().all(|v| uplift::to_view(&b.xf, 0, *v)[2] > 0.5);
    let (j, _) = mp
        .iter()
        .enumerate()
        .filter(|(j, p)| class_ok(sel, header_hash(hd[*j].0), hd[*j].1) && front(p))
        .max_by(|a, b| area(&a.1.v).total_cmp(&area(&b.1.v)))?;
    let p = &mp[j];
    let n = poly_normal(p);
    let c = mul(p.v.iter().fold([0.0; 3], |a, v| add(a, *v)), 1.0 / p.v.len() as f32);
    let (vc, vn) = (uplift::to_view(&b.xf, 0, c), uplift::to_view(&b.xf, 0, add(c, n)));
    let nv = sub(vn, vc);
    Some(if dot(nv, vc) > 0.0 { mul(n, -1.0) } else { n })
}

fn colour_of(src: &Sources, h3: u32) -> [u32; 3] {
    let o = (0x1000 + ((h3 >> 6) & 0x3ff) as usize) * 2;
    let c = src.palette.get(o..o + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]])) as u32;
    [c & 31, (c >> 5) & 31, (c >> 10) & 31]
}

pub fn run(roms: &RomSet, mode: &str, from: u64, count: u64, out: &Path, oba: u32, spec: &str) -> Result<(), String> {
    let script = model2::input::InputScript::parse(spec, &roms.game.controls).map_err(|e| format!("--input: {e}"))?;
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let rom = model2::bus::polygon_rom(roms);
    let mask = (rom.len().next_power_of_two() - 1) as u32;
    let mut m: Machine = crate::machine(roms);
    m.bus.geo.set_mode(model2::geo::GeoMode::Native);
    m.bus.geo.set_smooth(true);
    if let Ok(bytes) = std::fs::read(Path::new("out").join(roms.game.id).join("eeprom.bin")) {
        m.bus.eeprom.load(&bytes);
    }
    let view = View::native();
    let (vw, vh) = (view.width, view.height);
    let windows = (roms.game.hud_windows, roms.game.hud_element_windows);
    let view = view.with_hud_windows(windows.0, windows.1);

    let kind = std::env::var("SC_KIND").unwrap_or_else(|_| "tuft".into());
    let n_inst: usize = env_f("SC_N", 800.0) as usize;
    let seed = env_f("SC_SEED", 1.0) as u64;
    let sel = std::env::var("SC_CLASS").unwrap_or_else(|_| "all".into());
    let slope = env_f("SC_SLOPE", 35.0);
    let margin = env_f("SC_MARGIN", 0.06);
    let fade: Option<(f32, f32)> = std::env::var("SC_FADE").ok().and_then(|s| {
        let v: Vec<f32> = s.split(',').filter_map(|x| x.parse().ok()).collect();
        (v.len() == 2).then(|| (v[0], v[1]))
    });
    let tex = Arc::new(Skin { img: if kind == "pebble" { pebble_texture(32) } else { tuft_texture(64, 128) } });
    let mut gpu = if std::env::var_os("SC_GPU").is_some() {
        let mut g = gpu::Renderer::new()?;
        println!("gpu {}", g.adapter());
        g.set_pack(Some(Box::new(SkinPack(Arc::new(tex.img.clone())))));
        Some(g)
    } else {
        None
    };
    let gpu_max = env_f("SC_GPU_N", 8.0) as u32;
    let mut gpu_n = 0u32;

    let obas: Vec<u32> = std::env::var("SC_OBAS").ok().map(|s| s.split(',').filter_map(|x| u32::from_str_radix(x, 16).ok()).collect()).unwrap_or_default();
    let mut obas = obas;
    if !obas.contains(&oba) {
        obas.push(oba);
    }
    let mut printed: Vec<u32> = Vec::new();
    let mut models: HashMap<u32, (Scatter, Vec<MPoly>)> = HashMap::new();
    let (mut offs, mut total): (HashMap<u32, usize>, usize) = (HashMap::new(), 0);
    let names = ["block", "tri-min", "host", "d-tri-min", "d-host", "d-plane"];
    // Per variant: (tuft px, wrong over on other models, over on ground, hidden where ground, hidden where other).
    let mut st: Vec<[Stat; 6]> = (0..names.len()).map(|_| Default::default()).collect();
    let mut series: Vec<Vec<f64>> = vec![vec![]; names.len()];
    let mut ref_px = Stat::default();
    let mut tuft_only: Vec<(Stat, Stat)> = (0..names.len()).map(|_| Default::default()).collect();
    let (mut drift, mut drift_frames) = (Stat::default(), 0);
    let (mut t_gen, mut t_with, mut t_without) = (0.0f64, 0.0f64, 0.0f64);
    let (mut tris_max, mut inst_max, mut active_stat) = (0usize, 0usize, Stat::default());
    let mut notes: Vec<String> = Vec::new();
    let mut prev: Option<(Vec<bool>, Vec<[f32; 3]>, usize)> = None; // active, (x, y, hpx*shrink), ground px
    let (mut ev_lost, mut ev_gain) = ([0u32; 4], [0u32; 4]); // by size class: >=1, >=3, >=6, >=12 px
    let mut ev_lost_edge = 0u32;
    let mut ev_block_lost = 0u32;
    let mut ev_block_px: Vec<usize> = Vec::new();
    let mut shown_total = Stat::default();
    let mut gone_z: Vec<f32> = Vec::new();
    let mut frames = 0u32;
    let mut rows: HashMap<&str, Vec<Vec<Vec<u8>>>> = HashMap::new();
    let strip_names = ["host", "d-plane", "tri-min", "ref", "d-host"];
    let pick: Vec<u64> = (0..8).map(|i| from + (i * (count.max(2) - 1)) / 7).collect();
    let mut gpu_lines: Vec<String> = Vec::new();

    while m.frames < from + count {
        script.apply(m.frames, &mut m.bus);
        m.run_slice(u64::MAX).map_err(|e| format!("stopped: {e:?}"))?;
        m.bus.uart_log.clear();
        if m.frames < from || mode == "off" {
            continue;
        }
        m.bus.geo.sync();
        let Some(sf) = m.bus.geo.engine().smooth_frame() else { continue };
        let cap = capture::Frame::grab(&mut m.bus);
        let texrom: Vec<u16> = m.bus.texture_rom.to_vec();
        let ctx = Ctx { cap: &cap, texrom: &texrom, view: &view };
        let src = Sources {
            texture_rom: &texrom,
            texture_point_ram: &cap.texture_point_ram,
            render_ram: &cap.render_ram,
            palette: &cap.palette,
            xoff: cap.xoff,
            yoff: cap.yoff,
        };
        let rec = Subst::default();
        let (s_plain, side) = sf.render_subst(&rec, true);
        let side = side.ok_or("no side channel")?;
        let base = raster::build_shaded(&s_plain, &src, None);
        let places = uplift::walk(&s_plain, cap.xoff, cap.yoff);
        if places.len() != side.blocks.len() {
            notes.push(format!("frame {}: walk {} blocks, side {}", cap.frame, places.len(), side.blocks.len()));
            continue;
        }
        // Visible owners in the game's own order (no scatter).
        let mut bp = base.clone();
        let placed0 = raster::place(&mut bp, &view);
        let vis = raster::visible(&bp, &placed0, &src, &view);

        if mode == "list" {
            let mut by: HashMap<u32, (usize, [usize; 4], usize, [u64; 3])> = HashMap::new();
            for (o, &k) in vis.owner.iter().enumerate() {
                if k == u32::MAX {
                    continue;
                }
                let p = &bp[placed0[k as usize].poly];
                let oba = side.blocks[p.block as usize].oba & 0xff_ffff;
                let e = by.entry(oba).or_insert((0, [9999, 9999, 0, 0], 0, [0; 3]));
                e.0 += 1;
                let (x, y) = (o % vw, o / vw);
                e.1 = [e.1[0].min(x), e.1[1].min(y), e.1[2].max(x), e.1[3].max(y)];
                e.3[0] += p.colour[0] as u64;
                e.3[1] += p.colour[1] as u64;
                e.3[2] += p.colour[2] as u64;
            }
            let mut v: Vec<_> = by.into_iter().collect();
            v.sort_by_key(|e| std::cmp::Reverse(e.1 .0));
            for (oba, (px, bb, _, c)) in v.iter().take(14) {
                let npoly = bp.iter().filter(|p| side.blocks[p.block as usize].oba & 0xff_ffff == *oba).count();
                println!(
                    "frame {} model {oba:06x}: {px} px, bbox {bb:?}, polys {npoly}, blocks {}, mean colour {:?}",
                    cap.frame,
                    side.blocks.iter().filter(|b| b.oba & 0xff_ffff == *oba).count(),
                    [c[0] / (*px as u64).max(1), c[1] / (*px as u64).max(1), c[2] / (*px as u64).max(1)]
                );
            }
            // Colour-coded owners: the top 10 models in distinct colours over the picture dimmed.
            let pal: [[u8; 3]; 10] = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0], [255, 0, 255], [0, 255, 255], [255, 128, 0], [128, 0, 255], [255, 255, 255], [0, 128, 128]];
            let top: Vec<u32> = v.iter().take(10).map(|e| e.0).collect();
            let img = render(&mut bp.clone(), &ctx, false);
            let mut o = img.clone();
            for (i, &k) in vis.owner.iter().enumerate() {
                if k == u32::MAX {
                    continue;
                }
                let p = &bp[placed0[k as usize].poly];
                let ob = side.blocks[p.block as usize].oba & 0xff_ffff;
                if let Some(t) = top.iter().position(|&x| x == ob) {
                    for c in 0..3 {
                        o[i * 3 + c] = ((img[i * 3 + c] as u32 + pal[t][c] as u32 * 2) / 3) as u8;
                    }
                }
            }
            let mut both = Vec::new();
            for y in 0..vh {
                both.extend_from_slice(&img[y * vw * 3..(y + 1) * vw * 3]);
                both.extend_from_slice(&o[y * vw * 3..(y + 1) * vw * 3]);
            }
            let _ = std::fs::write(out.join(format!("owners-{}.png", cap.frame)), png::encode_rgb(vw * 2, vh, &both));
            if oba != 0 {
                if let Some((w, compact)) = words_of(&rec, &rom, mask, oba) {
                    let mp = parse_model(&w, compact);
                    let ks: Vec<usize> = (0..side.blocks.len()).filter(|&k| side.blocks[k].oba & 0xff_ffff == oba).collect();
                    let k = ks[0];
                    let (recs0, sp) = stream_polys(&s_plain, places[k].at);
                    let mut pxs = vec![0usize; recs0.len()];
                    let mut rgbs = vec![[0u64; 3]; recs0.len()];
                    for (o, &ow) in vis.owner.iter().enumerate().filter(|(_, &q)| q != u32::MAX) {
                        let pp = &bp[placed0[ow as usize].poly];
                        if pp.block as usize == k {
                            if let Some(j) = recs0.iter().position(|&r| r as u32 == pp.at) {
                                pxs[j] += 1;
                                for c in 0..3 {
                                    rgbs[j][c] += img[o * 3 + c] as u64;
                                }
                            }
                        }
                    }
                    let tha0 = s_plain[places[k].at + 2] & 0xff_ffff;
                    let _ = tha0;
                    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                    let mut edges = Vec::new();
                    for p in &mp {
                        for q in &p.v {
                            for a in 0..3 {
                                lo[a] = lo[a].min(q[a]);
                                hi[a] = hi[a].max(q[a]);
                            }
                        }
                        for e in 0..p.v.len() {
                            let d = sub(p.v[(e + 1) % p.v.len()], p.v[e]);
                            edges.push(dot(d, d).sqrt());
                        }
                    }
                    edges.sort_by(|a, b| a.total_cmp(b));
                    println!(
                        "model {oba:06x}: compact {compact}, {} polygons (stream {}), bbox {lo:?} .. {hi:?}, edge median {:.3} p10 {:.3} p90 {:.3}",
                        mp.len(),
                        sp.len(),
                        edges[edges.len() / 2],
                        edges[edges.len() / 10],
                        edges[edges.len() * 9 / 10]
                    );
                    let b = side.blocks[k];
                    let z = sp.iter().flat_map(|p| p.iter().map(|v| v[2])).fold((f32::MAX, f32::MIN), |a, z| (a.0.min(z), a.1.max(z)));
                    let ms = (b.xf[0] * b.xf[0] + b.xf[1] * b.xf[1] + b.xf[2] * b.xf[2]).sqrt();
                    println!("block {k}: focal {} {} z range {:.1}..{:.1} matrix scale {ms}", b.fx, b.fy, z.0, z.1);
                    let hd: Vec<([u16; 4], [u32; 3])> = {
                        let mut tha = s_plain[places[k].at + 2] & 0xff_ffff;
                        mp.iter()
                            .map(|p| {
                                let h = uplift::header_at(&src, tha);
                                let step = ((((p.attr >> 12) & 31) << 27) as i32) >> 27;
                                tha = (tha as i32 + step * 4) as u32 & 0xff_ffff;
                                (h, colour_of(&src, h[3] as u32))
                            })
                            .collect()
                    };
                    let rf = eye_ref(&mp, &hd, "all", &b);
                    let sc = build(&mp, &hd, "tuft", 10, 1, 1.0, 1.0, "all", 180.0, 0.0, rf, &|_| true);
                    for (j, p) in mp.iter().enumerate() {
                        let n = poly_normal(p);
                        let n = if p.stored.is_none() && rf.is_some_and(|r| dot(n, r) < 0.0) { mul(n, -1.0) } else { n };
                        let zc = p.v.iter().map(|v| uplift::to_view(&b.xf, 0, *v)[2]).fold((f32::MAX, f32::MIN), |a, z| (a.0.min(z), a.1.max(z)));
                        println!(
                            "poly {j}: visible px {} mean rgb {:?} class {:08x} colour {:?} verts {} area {:.0} n.up {:.2} stored {} z {:.0}..{:.0} attr {:#x}",
                            pxs[j],
                            rgbs[j].map(|c| c / (pxs[j] as u64).max(1)),
                            header_hash(hd[j].0) & 0xffff_ffff, hd[j].1, p.v.len(), area(&p.v), dot(n, sc.up), p.stored.is_some(), zc.0, zc.1, p.attr
                        );
                    }
                    println!("up (model) {:?}", sc.up);
                    for j in [4usize, 5, 6, 7] {
                        let pj = &sp[j];
                        let sx: Vec<String> = pj.iter().map(|v| format!("({:.0},{:.0},z{:.1})", places[k].centre.0 + v[0] / v[2], places[k].centre.1 - v[1] / v[2], v[2])).collect();
                        println!("poly {j} screen {}", sx.join(" "));
                    }
                    for (h, c, n, a) in sc.classes.iter().take(12) {
                        println!("class {:08x}: colour {c:?}, polygons {n}, area {a:.1}", h & 0xffff_ffff);
                    }
                    let mut slopes = [0usize; 6];
                    for p in &mp {
                        let n = poly_normal(p);
                        let n = if p.stored.is_none() && rf.is_some_and(|r| dot(n, r) < 0.0) { mul(n, -1.0) } else { n };
                        let c = dot(n, sc.up).clamp(-1.0, 1.0).acos().to_degrees();
                        slopes[((c / 10.0) as usize).min(5)] += 1;
                    }
                    println!("slope histogram (10 deg bins vs mean normal): {slopes:?}");
                }
            }
        }
        if mode == "list" {
            continue;
        }

        // ---------- run: the ground models' blocks (SC_OBAS: more chunks of the same ground).
        let ks: Vec<usize> = (0..side.blocks.len()).filter(|&k| obas.contains(&(side.blocks[k].oba & 0xff_ffff))).collect();
        frames += 1;
        let ground_px: usize = {
            let mut n = 0;
            for &k in vis.owner.iter().filter(|&&k| k != u32::MAX) {
                if ks.contains(&(bp[placed0[k as usize].poly].block as usize)) {
                    n += 1;
                }
            }
            n
        };
        if ks.is_empty() {
            if let Some((_, _, gp)) = &prev {
                if *gp > 0 {
                    ev_block_lost += 1;
                    ev_block_px.push(*gp);
                }
            }
            prev = Some((vec![], vec![], 0));
            notes.push(format!("frame {}: no ground model drawn", cap.frame));
            continue;
        }
        for &k0 in &ks {
            let oba_k = side.blocks[k0].oba & 0xff_ffff;
            if models.contains_key(&oba_k) {
                continue;
            }
            let Some((w, compact)) = words_of(&rec, &rom, mask, oba_k) else {
                notes.push(format!("frame {}: model {oba_k:06x} not read", cap.frame));
                continue;
            };
            let mp = parse_model(&w, compact);
            let mut tha = s_plain[places[k0].at + 2] & 0xff_ffff;
            let hd: Vec<([u16; 4], [u32; 3])> = mp
                .iter()
                .map(|p| {
                    let h = uplift::header_at(&src, tha);
                    let step = ((((p.attr >> 12) & 31) << 27) as i32) >> 27;
                    tha = (tha as i32 + step * 4) as u32 & 0xff_ffff;
                    (h, colour_of(&src, h[3] as u32))
                })
                .collect();
            let ms = (side.blocks[k0].xf[0].powi(2) + side.blocks[k0].xf[1].powi(2) + side.blocks[k0].xf[2].powi(2)).sqrt().max(1e-6);
            let h = if std::env::var_os("SC_HV").is_some() { env_f("SC_HV", 0.5) / ms } else { env_f("SC_H", 1.0) };
            let wd = env_f("SC_W", 0.0);
            let rf = eye_ref(&mp, &hd, &sel, &side.blocks[k0]);
            if rf.is_none() && compact {
                notes.push(format!("frame {}: model {oba_k:06x}: no polygon of the class in front of the eye yet", cap.frame));
                continue;
            }
            // Verge: within SC_VERGE of a polygon of the track class (SC_TRACK=hash), in model units.
            let verge = env_f("SC_VERGE", 0.0);
            let track = std::env::var("SC_TRACK").ok().and_then(|t| u64::from_str_radix(&t, 16).ok());
            let tpolys: Vec<&MPoly> = match track {
                Some(t) => mp.iter().enumerate().filter(|(j, _)| header_hash(hd[*j].0) & 0xffff_ffff == t).map(|(_, p)| p).collect(),
                None => vec![],
            };
            let accept = |p: V3| {
                verge <= 0.0
                    || tpolys.iter().any(|tp| {
                        (0..tp.v.len()).any(|e| {
                            let (a, b) = (tp.v[e], tp.v[(e + 1) % tp.v.len()]);
                            let ab = sub(b, a);
                            let t = (dot(sub(p, a), ab) / dot(ab, ab).max(1e-9)).clamp(0.0, 1.0);
                            let q = add(a, mul(ab, t));
                            let d = sub(p, q);
                            dot(d, d) < verge * verge
                        })
                    })
            };
            let sc = build(&mp, &hd, &kind, n_inst, seed ^ oba_k as u64, h, if wd > 0.0 { wd } else { h * 0.8 }, &sel, slope, margin, rf, &accept);
            println!(
                "scatter {oba_k:06x}: {} instances on {} eligible of {} polygons, {} tris, kind {kind}, h {h}, up {:?}",
                sc.inst.len(),
                sc.eligible,
                mp.len(),
                sc.mesh.tris.len(),
                sc.up
            );
            println!("  verge {verge} track polygons {}", tpolys.len());
            if sc.inst.is_empty() {
                models.insert(oba_k, (sc, mp));
                offs.insert(oba_k, total);
                continue;
            }
            offs.insert(oba_k, total);
            total += sc.inst.len();
            models.insert(oba_k, (sc, mp));
        }
        let n_drawn: usize = ks.iter().map(|&k| models.get(&(side.blocks[k].oba & 0xff_ffff)).map_or(0, |m| m.0.inst.len())).sum();
        inst_max = inst_max.max(n_drawn);

        // Per block, per frame data, then every variant's polygons.
        let mut var_polys: Vec<Vec<Poly>> = vec![Vec::new(); names.len() + 1]; // + reference
        let mut keep_base: Vec<Poly> = Vec::new();
        let mut act_now: Vec<bool> = vec![false; total];
        let mut feet: Vec<[f32; 3]> = vec![[0.0; 3]; total];
        let tg = Instant::now();
        for (ord, &k) in ks.iter().enumerate() {
            let b = side.blocks[k];
            let pl = places[k];
            let oba_k = b.oba & 0xff_ffff;
            let Some((sc, mpk)) = models.get(&oba_k) else { continue };
            let n_i = sc.inst.len();
            let off = offs[&oba_k];
            let (recs, sp) = stream_polys(&s_plain, pl.at);
            if sp.len() != mpk.len() {
                notes.push(format!("frame {}: polygons model {} stream {}", cap.frame, mpk.len(), sp.len()));
                continue;
            }
            let mut key_of: HashMap<u32, (f32, [u32; 3])> = HashMap::new();
            for p in base.iter().filter(|p| p.block as usize == k) {
                key_of.insert(p.at, (p.key, p.colour));
            }
            let med = {
                let mut v: Vec<f32> = base.iter().filter(|p| p.block as usize == k).map(|p| p.key).collect();
                v.sort_by(|a, b| a.total_cmp(b));
                v.get(v.len() / 2).copied().unwrap_or(0.0)
            };
            let mut active = vec![false; n_i];
            let (mut hkey, mut foot_z, mut cols) = (vec![0f32; n_i], vec![0f32; n_i], vec![[0u32; 3]; n_i]);
            let mut shrink = vec![1f32; n_i];
            let mut fpx = vec![[0f32; 3]; n_i];
            let mut dr = 0.0;
            let mut dn = 0;
            for (i, ins) in sc.inst.iter().enumerate() {
                let v = uplift::to_view(&b.xf, 0, ins.p);
                foot_z[i] = v[2];
                let (sx, sy) = uplift::project(&b, &pl, v);
                if let Some(&(kk, c)) = key_of.get(&(recs[ins.poly] as u32)) {
                    hkey[i] = kk;
                    cols[i] = c;
                    active[i] = v[2] > 0.5;
                }
                if let Some((z0, z1)) = fade {
                    shrink[i] = ((z1 - v[2]) / (z1 - z0)).clamp(0.0, 1.0);
                    if shrink[i] < 0.03 {
                        active[i] = false;
                    }
                }
                fpx[i] = [sx, sy, ins.h * b.fy * shrink[i] / v[2].max(0.1)];
                // Steadiness: the same point from the game's own vertices of its polygon.
                if active[i] && v[2] > 1.0 {
                    let g = {
                        let c = ins.corner;
                        let q = |t: usize| sp[ins.poly][c[t]];
                        add(add(mul(q(0), ins.w[0]), mul(q(1), ins.w[1])), mul(q(2), ins.w[2]))
                    };
                    if g[2] > 1.0 {
                        let (gx, gy) = (pl.centre.0 + g[0] / g[2], pl.centre.1 - g[1] / g[2]);
                        if gx.abs() < 4000.0 && gy.abs() < 4000.0 {
                            let e = (((sx - gx) as f64).powi(2) + ((sy - gy) as f64).powi(2)).sqrt();
                            drift.add(e);
                            dr += e;
                            dn += 1;
                        }
                    }
                }
            }
            let _ = (dr, dn);
            drift_frames += 1;
            if !printed.contains(&oba_k) {
                printed.push(oba_k);
                let mut per_poly = vec![0usize; sp.len()];
                for ins in &sc.inst {
                    per_poly[ins.poly] += 1;
                }
                println!("instances per polygon {per_poly:?}");
                for i in (0..n_i).step_by(n_i / 8 + 1) {
                    println!("  inst {i}: poly {} foot ({:.1},{:.1}) z {:.1} height px {:.1} active {}", sc.inst[i].poly, fpx[i][0], fpx[i][1], foot_z[i], fpx[i][2], active[i]);
                }
            }
            act_now[off..off + n_i].copy_from_slice(&active);
            feet[off..off + n_i].copy_from_slice(&fpx);
            // Lamp of the block, from its dominant face material.
            let faces = side.faces_of(&b);
            let mut counts: HashMap<u16, usize> = HashMap::new();
            for f in faces {
                *counts.entry(f.mat).or_insert(0) += 1;
            }
            let Some(mat) = counts.iter().max_by_key(|e| *e.1).map(|e| *e.0) else { continue };
            let face = faces.iter().find(|f| f.mat == mat).unwrap();
            let lamp = Lamp::of(&b, face, &side.materials[mat as usize]);
            let _ = Base::D;
            let colour = base.iter().find(|p| p.block as usize == k).map(|p| p.colour).unwrap_or([16, 16, 16]);
            let fixed = vec![med; n_i];
            let t0 = Instant::now();
            let roots: Vec<V3> = sc.inst.iter().map(|x| x.p).collect();
            let build_v = |key: KeyMode<'_>, plane: bool| -> Vec<Poly> {
                let mut i = Instance {
                    mesh: &sc.mesh,
                    skin: tex.clone(),
                    scale: [1.0; 3],
                    offset: [0.0; 3],
                    lamp,
                    colour,
                    key,
                    extra: Extra { per: sc.per, active: Some(&active[..]), plane: plane.then_some(&foot_z[..]), colours: Some(&cols[..]), ..Default::default() },
                    double_sided: kind != "pebble",
                    smooth: true,
                    flip: false,
                };
                if fade.is_some() {
                    i.extra.shrink = Some(&shrink[..]);
                    i.extra.roots = Some(&roots[..]);
                    i.extra.vper = sc.vper;
                }
                uplift::polys(&i, &b, &pl)
            };
            let (v_block, v_min, v_host, v_plane) =
                (build_v(KeyMode::Table(&fixed), false), build_v(KeyMode::TriMin, false), build_v(KeyMode::Table(&hkey), false), build_v(KeyMode::Table(&hkey), true));
            var_polys[3].extend(v_min.iter().cloned());
            var_polys[6].extend(v_min.iter().cloned());
            var_polys[4].extend(v_host.iter().cloned());
            var_polys[0].extend(v_block);
            var_polys[1].extend(v_min);
            var_polys[2].extend(v_host);
            var_polys[5].extend(v_plane);
            if ord == 0 {
                t_gen += t0.elapsed().as_secs_f64();
            }
            let _ = &mut keep_base;
        }
        let _ = tg;
        let tris = var_polys[2].len();
        tris_max = tris_max.max(tris);
        let n_act = act_now.iter().filter(|&&a| a).count();
        active_stat.add(n_act as f64);

        // ---- Pop events against the previous frame.
        if let Some((pa, pf, gp)) = &prev {
            let inside = |f: &[f32; 3]| f[0] >= 0.0 && f[0] < vw as f32 && f[1] >= 0.0 && f[1] < vh as f32;
            let n = pa.len().min(act_now.len());
            for i in 0..n {
                let cls = |h: f32| (h >= 1.0) as usize + (h >= 3.0) as usize + (h >= 6.0) as usize + (h >= 12.0) as usize;
                if pa[i] && !act_now[i] && inside(&pf[i]) && pf[i][2] >= 1.0 {
                    for c in 0..cls(pf[i][2]) {
                        ev_lost[c] += 1;
                    }
                    if !(pf[i][1] > vh as f32 - 4.0 || pf[i][0] < 4.0 || pf[i][0] > vw as f32 - 4.0) {
                        ev_lost_edge += 1;
                    }
                    gone_z.push(pf[i][2]);
                }
                if !pa[i] && act_now[i] && inside(&feet[i]) && feet[i][2] >= 1.0 {
                    for c in 0..cls(feet[i][2]) {
                        ev_gain[c] += 1;
                    }
                }
            }
            let _ = gp;
        }
        shown_total.add(feet.iter().zip(&act_now).filter(|(f, a)| **a && f[0] >= 0.0 && f[0] < vw as f32 && f[1] >= 0.0 && f[1] < vh as f32 && f[2] >= 1.0).count() as f64);
        prev = Some((act_now.clone(), feet.clone(), ground_px));

        // ---- Draw every variant, with the scatter's pixels marked.
        let t0 = Instant::now();
        let img0 = {
            let mut p = base.clone();
            render(&mut p, &ctx, false)
        };
        t_without += t0.elapsed().as_secs_f64();
        let t0 = Instant::now();
        let img_host = {
            let mut p = base.clone();
            p.extend(var_polys[2].iter().cloned());
            render(&mut p, &ctx, false)
        };
        t_with += t0.elapsed().as_secs_f64();
        let ground_blocks: Vec<usize> = ks.clone();
        let owner_other: Vec<u8> = vis
            .owner
            .iter()
            .map(|&k| {
                if k == u32::MAX {
                    0
                } else if ground_blocks.contains(&(bp[placed0[k as usize].poly].block as usize)) {
                    1
                } else {
                    2
                }
            })
            .collect();
        uplift::MARK.store(true, Relaxed);
        let depth_of = |i: usize| matches!(i, 3 | 4 | 5 | 6);
        let mut marks: Vec<Vec<bool>> = Vec::new();
        for (vi, vp) in var_polys.iter().enumerate() {
            let mut p = base.clone();
            p.extend(vp.iter().cloned());
            if vi == 6 {
                uplift::ALL_TEST.store(true, Relaxed);
            }
            let im = render(&mut p, &ctx, depth_of(vi));
            uplift::ALL_TEST.store(false, Relaxed);
            marks.push(im.chunks(3).map(|p| p[0] == 255 && p[1] == 0 && p[2] == 255).collect());
        }
        uplift::MARK.store(false, Relaxed);
        let refm = &marks[6];
        let rc = refm.iter().filter(|x| **x).count();
        ref_px.add(rc as f64);
        let mut wrongs = Vec::new();
        for vi in 0..names.len() {
            let mk = &marks[vi];
            let (mut px, mut over_o, mut over_g, mut hid_g, mut hid_o) = (0usize, 0usize, 0usize, 0usize, 0usize);
            for o in 0..mk.len() {
                px += mk[o] as usize;
                if mk[o] && !refm[o] {
                    if owner_other[o] == 2 {
                        over_o += 1
                    } else {
                        over_g += 1
                    }
                }
                if !mk[o] && refm[o] {
                    if owner_other[o] == 1 {
                        hid_g += 1
                    } else {
                        hid_o += 1
                    }
                }
            }
            st[vi][0].add(px as f64);
            st[vi][1].add(over_o as f64);
            st[vi][2].add(over_g as f64);
            st[vi][3].add(hid_g as f64);
            st[vi][4].add(hid_o as f64);
            st[vi][5].add((over_o + over_g + hid_g + hid_o) as f64);
            series[vi].push((over_o + over_g + hid_g + hid_o) as f64);
            wrongs.push((px, over_o, over_g, hid_g, hid_o));
        }
        // Cut-outs among themselves: the scatter alone, sorted vs the depth-exact picture.
        {
            let mut r = var_polys[6].clone();
            uplift::ALL_TEST.store(true, Relaxed);
            let (rr, rcov) = raster::draw_view_depth(&mut r, &src, &view, true);
            uplift::ALL_TEST.store(false, Relaxed);
            for vi in 0..names.len() {
                let mut p = var_polys[vi].clone();
                let (a, acov) = raster::draw_view_depth(&mut p, &src, &view, depth_of(vi));
                let (mut cov_diff, mut col_diff, mut both) = (0usize, 0usize, 0usize);
                for o in 0..acov.len() {
                    if acov[o] != rcov[o] {
                        cov_diff += 1;
                    } else if acov[o] {
                        both += 1;
                        if a[o * 3..o * 3 + 3] != rr[o * 3..o * 3 + 3] {
                            col_diff += 1;
                        }
                    }
                }
                let _ = both;
                tuft_only[vi].0.add(cov_diff as f64);
                tuft_only[vi].1.add(col_diff as f64);
            }
        }
        println!(
            "frame {}: blocks {}, instances active {}/{}, tris {}, ground px {}, ref tuft px {} | host px {} wrong {:?}| d-plane px {} wrong {:?}",
            cap.frame,
            ks.len(),
            n_act,
            n_drawn,
            tris,
            ground_px,
            rc,
            wrongs[2].0,
            [wrongs[2].1, wrongs[2].2, wrongs[2].3, wrongs[2].4],
            wrongs[5].0,
            [wrongs[5].1, wrongs[5].2, wrongs[5].3, wrongs[5].4]
        );

        // ---- Strips: original, the option, and the wrong-pixel map.
        if pick.contains(&cap.frame) {
            // Centre the crop on the tufts of this frame (host variant).
            let (mut sx, mut sy, mut sn) = (0u64, 0u64, 0u64);
            for (o, &mk) in marks[2].iter().enumerate() {
                if mk {
                    sx += (o % vw) as u64;
                    sy += (o / vw) as u64;
                    sn += 1;
                }
            }
            let c = if let Some(v) = std::env::var("SC_CROP").ok().map(|s| s.split(',').filter_map(|x| x.parse::<f32>().ok()).collect::<Vec<_>>()).filter(|v| v.len() == 2) {
                (v[0], v[1])
            } else if sn > 0 {
                ((sx / sn) as f32, (sy / sn) as f32)
            } else {
                (248.0, 192.0)
            };
            for name in strip_names {
                let vi = if name == "ref" { 6 } else { names.iter().position(|n| *n == name).unwrap() };
                let mut p = base.clone();
                p.extend(var_polys[vi].iter().cloned());
                if vi == 6 {
                    uplift::ALL_TEST.store(true, Relaxed);
                }
                let im = render(&mut p, &ctx, depth_of(vi));
                uplift::ALL_TEST.store(false, Relaxed);
                // Wrong-pixel map: red = tuft shown where the reference has none; blue = hidden where the reference shows it.
                let mut wm = img0.clone();
                for o in 0..marks[vi].len() {
                    if marks[vi][o] && !refm[o] {
                        wm[o * 3..o * 3 + 3].copy_from_slice(&[255, 0, 0]);
                    } else if !marks[vi][o] && refm[o] {
                        wm[o * 3..o * 3 + 3].copy_from_slice(&[0, 80, 255]);
                    }
                }
                let r = rows.entry(name).or_insert_with(|| vec![vec![], vec![], vec![]]);
                r[0].push(crop(&img0, vw, vh, c.0, c.1, 248, 192, 1));
                r[1].push(crop(&im, vw, vh, c.0, c.1, 248, 192, 1));
                r[2].push(crop(&wm, vw, vh, c.0, c.1, 248, 192, 1));
            }
            let _ = &img_host;
        }

        // ---- GPU, offscreen, one frame at a time.
        if let Some(g) = gpu.as_mut() {
            if gpu_n < gpu_max && frames % 8 == 1 {
                gpu_n += 1;
                let (g0, t_g0) = gpu_draw(g, &base, &ctx, &view, false, false);
                let mut line = format!("GPU frame {} base {:.1} ms", cap.frame, t_g0);
                let mut gimgs: Vec<(String, Vec<u8>, Vec<u8>)> = Vec::new();
                for (vi, depth) in [(2usize, false), (2, true), (5, true), (1, true)] {
                    let mut p = base.clone();
                    p.extend(var_polys[vi].iter().cloned());
                    let (gi, tg) = gpu_draw(g, &p, &ctx, &view, depth, false);
                    // The software picture of the same variant.
                    let mut ps = p.clone();
                    let si = render(&mut ps, &ctx, depth);
                    let region = or(&differs(&si, &img0), &differs(&gi, &g0));
                    let c = cmp(&si, &gi, &region);
                    line += &format!(
                        "\n  variant {} depth {}: gpu {:.1} ms, scatter region {} px, sw vs gpu differing {} ({:.1}%) >16 {} mean {:.2}",
                        names[vi],
                        depth,
                        tg,
                        c.mask,
                        c.any,
                        100.0 * c.any as f64 / c.mask.max(1) as f64,
                        c.big,
                        c.mean
                    );
                    gimgs.push((format!("{}-{}", names[vi], depth), gi, si));
                }
                if gpu_n == 1 {
                    // Timing: native and 1080p, with and without, mean of 3 (host variant, depth off).
                    let mut p = base.clone();
                    p.extend(var_polys[2].iter().cloned());
                    let big = View::new(1080, 16.0 / 9.0, video::Hud::Edges).with_hud_windows(windows.0, windows.1);
                    let (mut a, mut b, mut c2, mut d) = (0.0, 0.0, 0.0, 0.0);
                    for _ in 0..3 {
                        a += gpu_draw(g, &p, &ctx, &view, false, false).1;
                        b += gpu_draw(g, &base, &ctx, &view, false, false).1;
                        c2 += gpu_draw(g, &p, &ctx, &big, false, false).1;
                        d += gpu_draw(g, &base, &ctx, &big, false, false).1;
                    }
                    line += &format!("\n  cost per frame (incl. read-back): native with {:.2} ms without {:.2} ms; 1080p with {:.2} ms without {:.2} ms; tris {}", a / 3.0, b / 3.0, c2 / 3.0, d / 3.0, tris);
                    let (gi, _) = gpu_draw(g, &p, &ctx, &view, true, false);
                    let _ = gi;
                }
                gpu_lines.push(line);
                // Strip of this GPU frame: base | sw host | gpu host | gpu host depth | gpu plane depth.
                let (ca, cb) = (248.0, 192.0);
                let r = rows.entry("gpu").or_insert_with(|| vec![vec![], vec![], vec![], vec![], vec![]]);
                r[0].push(crop(&g0, vw, vh, ca, cb, 248, 192, 1));
                r[1].push(crop(&gimgs[0].2, vw, vh, ca, cb, 248, 192, 1));
                r[2].push(crop(&gimgs[0].1, vw, vh, ca, cb, 248, 192, 1));
                r[3].push(crop(&gimgs[1].1, vw, vh, ca, cb, 248, 192, 1));
                r[4].push(crop(&gimgs[2].1, vw, vh, ca, cb, 248, 192, 1));
            }
        }
    }
    if mode == "off" || mode == "list" {
        println!("state hash {:016x}", m.bus.state_hash());
        return Ok(());
    }
    for (name, r) in &rows {
        if !r[0].is_empty() {
            strip(r, 248, 192, &out.join(format!("strip-{name}.png")));
        }
    }
    println!("frames {frames}, instances {n_inst} per ground model, max drawn per frame {inst_max}, triangles per frame (host variant) {tris_max}, active per frame {}", active_stat.show());
    println!("shown (foot on screen, >=1 px) per frame: {}", shown_total.show());
    println!("1. steadiness: reprojection of every active instance's foot vs the game's own vertices of its polygon (px): {}", drift.show());
    println!("reference (geometric depth, every polygon tested) tuft px per frame: {}", ref_px.show());
    for (vi, n) in names.iter().enumerate() {
        let jump: Vec<f64> = series[vi].windows(2).map(|w| (w[1] - w[0]).abs()).collect();
        let jm = jump.iter().sum::<f64>() / jump.len().max(1) as f64;
        println!(
            "2. {n:>9}: tuft px {} | over other models {} | over ground (ref hidden) {} | hidden where ground {} | hidden where other {} | total wrong {} | frame-to-frame change of wrong mean {:.0} worst {:.0}",
            st[vi][0].show(),
            st[vi][1].show(),
            st[vi][2].show(),
            st[vi][3].show(),
            st[vi][4].show(),
            st[vi][5].show(),
            jm,
            jump.iter().cloned().fold(0.0, f64::max)
        );
    }
    for (vi, n) in names.iter().enumerate() {
        println!("4. {n:>9}: scatter alone vs depth-exact: coverage differs {} | both covered, colour differs {}", tuft_only[vi].0.show(), tuft_only[vi].1.show());
    }
    println!(
        "3. pop: block not drawn after being drawn {ev_block_lost} (ground px the frame before {:?}); instances lost in view by size >=1/3/6/12 px: {ev_lost:?} (not near the screen edge: {ev_lost_edge}); gained {ev_gain:?}; lost size px (mean {:.1})",
        ev_block_px,
        gone_z.iter().sum::<f32>() / gone_z.len().max(1) as f32
    );
    let nf = frames.max(1) as f64;
    println!(
        "5. cost per frame (software): polygons {:.3} ms, draw with {:.1} ms without {:.1} ms; tris {tris_max}",
        1000.0 * t_gen / nf,
        1000.0 * t_with / nf,
        1000.0 * t_without / nf
    );
    for l in &gpu_lines {
        println!("{l}");
    }
    let _ = (drift_frames,);
    for n in notes.iter().take(30) {
        println!("note: {n}");
    }
    println!("state hash {:016x}", m.bus.state_hash());
    let _: Option<Block> = None;
    Ok(())
}
