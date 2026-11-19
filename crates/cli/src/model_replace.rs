//! SPIKE `remodel model-replace MODE FROM COUNT OUT [OBA]`: rerun a frame's
//! display list through the native geometrizer with a substituted polygon
//! source for one model, draw it in software, never touching the machine.
//! MODE: list | none | identity | scale | box.
use model2::capture;
use model2::geo::Subst;
use model2::raster::{self, Sources};
use model2::video::{self, View};
use model2::Machine;
use romset::RomSet;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

fn f(w: u32) -> f32 {
    f32::from_bits(w)
}

/// The words of the model `[base, end)` as the machine's polygon ROM has them.
pub(crate) fn rom_words(rom: &[u32], mask: u32, base: u32, end: u32) -> Vec<u32> {
    let n = end.wrapping_sub(base) & 0xff_ffff;
    (0..n).map(|k| rom[((base + k) & 0xff_ffff & mask) as usize]).collect()
}

/// A compact model's records: (word index of the attribute, vertex word
/// indices). The two start vertices come first (attribute index None).
fn records(w: &[u32]) -> (Vec<usize>, Vec<(usize, Vec<usize>)>) {
    records_x(w, true)
}

/// `compact`: topskatr's; else the standard format, 10 words per polygon
/// (attr, normal, Pa, Pb), whose vertices (not normals) are returned.
fn records_x(w: &[u32], compact: bool) -> (Vec<usize>, Vec<(usize, Vec<usize>)>) {
    let start: Vec<usize> = (0..6).collect();
    let mut recs = Vec::new();
    let mut i = 6;
    while i < w.len() {
        let attr = w[i];
        if attr & 3 == 0 {
            break;
        }
        if compact {
            let k = if attr & 1 != 0 { 6 } else { 3 };
            recs.push((i, (i + 1..(i + 1 + k).min(w.len())).collect()));
            i += 1 + k;
        } else {
            recs.push((i, (i + 4..(i + 10).min(w.len())).collect()));
            i += 10;
        }
    }
    (start, recs)
}

pub(crate) type Gpu = Option<gpu::Renderer>;

fn draw(stream: &[u32], cap: &capture::Frame, texrom: &[u16], points: &[u16], view: &View, windows: (&[usize], &[usize]), gpu: &mut Gpu) -> Vec<u8> {
    let view = view.with_hud_windows(windows.0, windows.1);
    let src = Sources {
        texture_rom: texrom,
        texture_point_ram: points,
        render_ram: &cap.render_ram,
        palette: &cap.palette,
        xoff: cap.xoff,
        yoff: cap.yoff,
    };
    let mut polys = raster::build_shaded(stream, &src, None);
    video::HudLayout::new(&cap.tiles, &cap.palette, &view).anchor(&mut polys, &view);
    let _placed = raster::place(&mut polys, &view);
    if let Some(g) = gpu.as_mut() {
        // SPIKE: the same polygons through the wgpu renderer, offscreen.
        let placed = _placed;
        g.forget_ram();
        g.set_quality(crate::gpu_light(None));
        let layers = video::layers_2d(&cap.tiles, &cap.palette, &view, !placed.is_empty());
        return g.frame(&polys, &placed, &src, &view, &layers);
    }
    if polys.is_empty() {
        return video::compose(&cap.tiles, &cap.palette, None, &view);
    }
    let layer = raster::draw_view_depth(&mut polys, &src, &view, false);
    video::compose(&cap.tiles, &cap.palette, Some((&layer.0, &layer.1)), &view)
}

/// Per model address, the 3D pixels the stream shows (blocks from `side`).
fn pixels(stream: &[u32], side: &model2::light::Frame, cap: &capture::Frame, texrom: &[u16], view: &View) -> BTreeMap<u32, u64> {
    let src = Sources {
        texture_rom: texrom,
        texture_point_ram: &cap.texture_point_ram,
        render_ram: &cap.render_ram,
        palette: &cap.palette,
        xoff: cap.xoff,
        yoff: cap.yoff,
    };
    let mut polys = raster::build_shaded(stream, &src, None);
    let placed = raster::place(&mut polys, view);
    let vis = raster::visible(&polys, &placed, &src, view);
    let mut out = BTreeMap::new();
    for &k in vis.owner.iter().filter(|&&k| k != u32::MAX) {
        let b = polys[placed[k as usize].poly].block as usize;
        if let Some(bl) = side.blocks.get(b) {
            *out.entry(bl.oba & 0xff_ffff).or_insert(0) += 1;
        }
    }
    out
}

fn attr_of(first: u32, link: u32, quad: bool) -> u32 {
    let keep = first & 0xf87c_0000;
    let sort = if (first >> 10) & 3 == 0 { 1 } else { (first >> 10) & 3 };
    keep | (link << 23) | (link << 8) | sort << 10 | 1 << 17 | if quad { 1 } else { 2 }
}

/// A box over the original's bounding box, as a compact model of 8 quad
/// records (4 sides in a strip, a hidden restart, the top, a hidden
/// restart, the bottom) and a terminator.
fn box_mesh(orig: &[u32], first_attr: u32) -> Vec<u32> {
    let (start, recs) = records(orig);
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    let mut see = |i: usize| {
        for a in 0..3 {
            let v = f(orig[i + a]);
            if v.is_finite() {
                lo[a] = lo[a].min(v);
                hi[a] = hi[a].max(v);
            }
        }
    };
    see(start[0]);
    see(3);
    for (_, vs) in &recs {
        for c in vs.chunks(3) {
            see(c[0]);
        }
    }
    let m: [f32; 3] = std::array::from_fn(|a| (lo[a] + hi[a]) / 2.0);
    let h: [f32; 3] = std::array::from_fn(|a| (hi[a] - lo[a]) / 2.0 * 0.7);
    let v = |sx: f32, y: f32, sz: f32| -> [u32; 3] {
        [(m[0] + sx * h[0]).to_bits() & !1, (m[1] + y * h[1]).to_bits(), (m[2] + sz * h[2]).to_bits()]
    };
    let b = [v(-1.0, -1.0, -1.0), v(1.0, -1.0, -1.0), v(1.0, -1.0, 1.0), v(-1.0, -1.0, 1.0)];
    let t = [v(-1.0, 1.0, -1.0), v(1.0, 1.0, -1.0), v(1.0, 1.0, 1.0), v(-1.0, 1.0, 1.0)];
    let mut w: Vec<u32> = Vec::new();
    w.extend(b[0]);
    w.extend(t[0]);
    let mut quad = |link: u32, pa: [u32; 3], pb: [u32; 3]| {
        w.push(attr_of(first_attr, link, true));
        w.extend(pa);
        w.extend(pb);
    };
    quad(2, b[1], t[1]);
    quad(2, b[2], t[2]);
    quad(2, b[3], t[3]);
    quad(2, b[0], t[0]);
    quad(0, t[0], t[1]);
    quad(2, t[3], t[2]);
    quad(0, b[1], b[0]);
    quad(2, b[2], b[3]);
    w.push(0);
    w
}


/// The model's bounding box (centre, half extents), from its vertex words.
fn bbox(orig: &[u32], compact: bool) -> ([f32; 3], [f32; 3]) {
    let (start, recs) = records_x(orig, compact);
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    let mut see = |i: usize| {
        for a in 0..3 {
            let v = f(orig[i + a]);
            if v.is_finite() {
                lo[a] = lo[a].min(v);
                hi[a] = hi[a].max(v);
            }
        }
    };
    see(start[0]);
    see(3);
    for (_, vs) in &recs {
        for c in vs.chunks(3) {
            if c.len() == 3 {
                see(c[0]);
            }
        }
    }
    (std::array::from_fn(|a| (lo[a] + hi[a]) / 2.0), std::array::from_fn(|a| (hi[a] - lo[a]) / 2.0))
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// A box over the original's bounding box (70% extent) in the standard 10-word record: 12
/// triangles (MP_MESH=tri, default: two a face) or 6 quads (MP_MESH=quad). Returns (model
/// words, record count, per record the corner count: 0 = hidden restart quad).
/// Each real polygon is preceded by a hidden restart quad (link type 0) whose (Pa, Pb) =
/// (P0', P1') the polygon needs, except the first, whose edge is the model's two start vertices.
/// The stored normal of every record follows MP_NORM: s (outward x MP_SIGN, default -1: the
/// sign the geometrizer calls front), flip, up, zero, big (x4).
fn std_mesh(orig: &[u32], first: u32) -> (Vec<u32>, usize, Vec<u8>) {
    let (m, e) = bbox(orig, false);
    let h: [f32; 3] = std::array::from_fn(|a| e[a] * 0.7);
    let p = |sx: f32, sy: f32, sz: f32| [m[0] + sx * h[0], m[1] + sy * h[1], m[2] + sz * h[2]];
    // Corners of the box: index bits x, y, z.
    let c = |i: usize| p(if i & 1 != 0 { 1.0 } else { -1.0 }, if i & 2 != 0 { 1.0 } else { -1.0 }, if i & 4 != 0 { 1.0 } else { -1.0 });
    // Faces as 4 corner indices around, with the outward normal axis.
    let faces: [([usize; 4], [f32; 3]); 6] = [
        ([0, 2, 6, 4], [-1.0, 0.0, 0.0]),
        ([1, 5, 7, 3], [1.0, 0.0, 0.0]),
        ([0, 4, 5, 1], [0.0, -1.0, 0.0]),
        ([2, 3, 7, 6], [0.0, 1.0, 0.0]),
        ([0, 1, 3, 2], [0.0, 0.0, -1.0]),
        ([4, 6, 7, 5], [0.0, 0.0, 1.0]),
    ];
    let shape = std::env::var("MP_MESH").unwrap_or_else(|_| "tri".into());
    let mut polys: Vec<(Vec<[f32; 3]>, [f32; 3])> = Vec::new();
    for (idx, n) in faces {
        let v: Vec<[f32; 3]> = idx.iter().map(|&i| c(i)).collect();
        if shape == "quad" {
            polys.push((v, n));
        } else {
            polys.push((vec![v[0], v[1], v[2]], n));
            polys.push((vec![v[0], v[2], v[3]], n));
        }
    }
    let sign: f32 = std::env::var("MP_SIGN").ok().and_then(|s| s.parse().ok()).unwrap_or(-1.0);
    let norm = std::env::var("MP_NORM").unwrap_or_else(|_| "s".into());
    let stored = |n: [f32; 3]| -> [f32; 3] {
        match norm.as_str() {
            "flip" => n.map(|x| -x * sign),
            "up" => [0.0, 1.0, 0.0],
            "zero" => [0.0; 3],
            "big" => n.map(|x| x * sign * 4.0),
            _ => n.map(|x| x * sign),
        }
    };
    // Corner order [P1', P0', Pa, Pb] such that the program's computed normal
    // (P1' - P0') x (Pa - P0') points the way `sign` calls the stored normal.
    let want = |n: [f32; 3]| n.map(|x| x * sign);
    let mut words: Vec<u32> = Vec::new();
    let mut kinds = Vec::new();
    let bits = |v: [f32; 3]| v.map(f32::to_bits);
    let keep = first & 0xf87c_0000;
    let sort = if (first >> 10) & 3 == 0 { 1 } else { (first >> 10) & 3 };
    let real = |tri: bool| keep | (2 << 8) | sort << 10 | if tri { 2 } else { 1 };
    let dummy = keep | sort << 10 | 1 << 17 | 1;
    let mut n_rec = 0;
    for (k, (v, n)) in polys.iter().enumerate() {
        let mut v = v.clone();
        let mut ord: Vec<usize> = (0..v.len()).collect();
        if dot3(cross(sub3(v[ord[0]], v[ord[1]]), sub3(v[ord[2]], v[ord[1]])), want(*n)) < 0.0 {
            ord.reverse();
        }
        v = ord.iter().map(|&i| v[i]).collect();
        // [P1', P0', Pa, (Pb)] = v; the edge before it: P0' = v[1], P1' = v[0].
        if k == 0 {
            words.extend(bits(v[1]));
            words.extend(bits(v[0]));
        } else {
            words.push(dummy);
            words.extend([0; 3]);
            words.extend(bits(v[1]));
            words.extend(bits(v[0]));
            kinds.push(0);
            n_rec += 1;
        }
        words.push(real(v.len() == 3));
        words.extend(bits(stored(*n)));
        words.extend(bits(v[2]));
        words.extend(if v.len() == 4 { bits(v[3]) } else { [0; 3] });
        kinds.push(v.len() as u8);
        n_rec += 1;
    }
    words.push(0);
    (words, n_rec, kinds)
}

pub fn run(roms: &RomSet, mode: &str, from: u64, count: u64, out: &Path, oba_arg: Option<u32>, spec: &str) -> Result<(), String> {
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
    let mut gpu: Gpu = if std::env::var_os("MP_GPU").is_some() { Some(gpu::Renderer::new()?) } else { None };
    if let Some(g) = &gpu {
        println!("gpu {}", g.adapter());
    }
    let windows = (roms.game.hud_windows, roms.game.hud_element_windows);
    let mut strip: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let (mut t_plain, mut t_sub, mut nframes) = (0.0f64, 0.0f64, 0u32);
    while m.frames < from + count {
        script.apply(m.frames, &mut m.bus);
        m.run_slice(u64::MAX).map_err(|e| format!("stopped: {e:?}"))?;
        m.bus.uart_log.clear();
        if m.frames < from || mode == "none" {
            continue;
        }
        m.bus.geo.sync();
        let Some(sf) = m.bus.geo.engine().smooth_frame() else { continue };
        let cap = capture::Frame::grab(&mut m.bus);
        let texrom: Vec<u16> = m.bus.texture_rom.to_vec();
        // The extents of every object this frame, and the unchanged rerun.
        let rec = Subst::default();
        let t0 = Instant::now();
        let (s_plain, side) = sf.render_subst(&rec, true);
        t_plain += t0.elapsed().as_secs_f64();
        let seen = rec.seen.lock().unwrap().clone();
        let side = side.ok_or("no side channel")?;
        let ram = rec.ram.lock().unwrap().clone();
        let words_of = |oba: u32| -> Option<(Vec<u32>, bool, u32)> {
            seen.iter().find(|s| s.0 == oba).map(|s| {
                let w = if s.0 & 0x80_0000 != 0 {
                    rom_words(&rom, mask, s.0, s.1)
                } else {
                    // Polygon RAM: the words as the frame's list left them (unread words 0).
                    (0..s.1.wrapping_sub(s.0) & 0xff_ffff).map(|k| ram.get(&(s.0 + k)).map_or(0, |e| e.0)).collect()
                };
                (w, s.2, s.3)
            })
        };
        nframes += 1;
        match mode {
            "list" => {
                let px = pixels(&s_plain, &side, &cap, &texrom, &view);
                let mut rows: Vec<_> = px.iter().collect();
                rows.sort_by(|a, b| b.1.cmp(a.1));
                println!("frame {}: {} objects, stream equal to machine's: {}", cap.frame, seen.len(), s_plain == cap.stream);
                for (oba, p) in rows.iter().take(25) {
                    let (w, compact, n) = words_of(**oba).unwrap_or_default();
                    println!("  oba {:06x} px {:>6} words {:>5} compact {} list-count {}", oba, p, w.len(), compact, n);
                }
                return Ok(());
            }
            "identity" => {
                let mut sub = Subst::default();
                for s in &seen {
                    if !sub.models.iter().any(|x| x.0 == s.0) {
                        if let Some((w, _, _)) = words_of(s.0) {
                            sub.models.push((s.0, w));
                        }
                    }
                }
                let t0 = Instant::now();
                let (s_sub, _) = sf.render_subst(&sub, false);
                t_sub += t0.elapsed().as_secs_f64();
                let a = draw(&cap.stream, &cap, &texrom, &cap.texture_point_ram, &view, windows, &mut gpu);
                let b = draw(&s_sub, &cap, &texrom, &cap.texture_point_ram, &view, windows, &mut gpu);
                let diff = a.chunks(3).zip(b.chunks(3)).filter(|(x, y)| x != y).count();
                let nram = sub.models.iter().filter(|m| m.0 & 0x80_0000 == 0).count();
                let conflicts = ram.values().filter(|e| e.1 > 0).count();
                println!(
                    "frame {}: ram models {nram} (ram words read with changing values: {conflicts}) | models {} | machine stream {} words, rerun-with-own-words {} words, equal {} | rerun without override equal {} | differing pixels {}",
                    cap.frame,
                    sub.models.len(),
                    cap.stream.len(),
                    s_sub.len(),
                    s_sub == cap.stream,
                    s_plain == cap.stream,
                    diff
                );
            }
            "normals" => {
                // What a standard model's stored normals are: against the winding the link rules
                // give (computed normal c = (P1' - P0') x (Pa - P0')) and against "outward" (away
                // from the model's centre).
                let oba = oba_arg.ok_or("OBA wanted")?;
                let Some((w, compact, _)) = words_of(oba) else { continue };
                let (centre, _) = bbox(&w, compact);
                let v = |i: usize| [f(w[i]), f(w[i + 1]), f(w[i + 2])];
                let (mut p0, mut p1) = (v(0), v(3));
                let (mut n, mut agree_wind, mut agree_out, mut unit, mut zero) = (0, 0, 0, 0, 0);
                let mut i = 6;
                while i + 10 <= w.len() && w[i] & 3 != 0 {
                    let attr = w[i];
                    let quad = attr & 1 != 0;
                    let st = v(i + 1);
                    let pa = v(i + 4);
                    let pb = if quad { v(i + 7) } else { pa };
                    let c = cross(sub3(p1, p0), sub3(pa, p0));
                    let cen = [0, 1, 2].map(|a| (p0[a] + p1[a] + pa[a] + if quad { pb[a] } else { 0.0 }) / if quad { 4.0 } else { 3.0 });
                    if (attr >> 8) & 3 != 0 {
                        n += 1;
                        let len = dot3(st, st).sqrt();
                        zero += (len < 1e-6) as u32;
                        unit += ((len - 1.0).abs() < 0.01) as u32;
                        agree_wind += (dot3(st, c) > 0.0) as u32;
                        agree_out += (dot3(st, sub3(cen, centre)) > 0.0) as u32;
                    }
                    match (attr >> 8) & 3 {
                        1 => p1 = pa,
                        3 => p0 = pb,
                        _ => (p0, p1) = (pa, pb),
                    }
                    i += 10;
                }
                println!("model {oba:06x}: {n} drawn polygons; stored normal: unit {unit}, zero {zero}, agrees with computed (P1'-P0')x(Pa-P0') {agree_wind}, points away from the model centre {agree_out}");
                return Ok(());
            }
            "scale" | "box" | "stdbox" | "remove" => {
                let oba = oba_arg.ok_or("OBA wanted")?;
                let Some((orig, compact, n)) = words_of(oba) else {
                    println!("frame {}: model {oba:06x} not drawn", cap.frame);
                    continue;
                };
                if !compact && mode == "box" {
                    return Err(format!("{oba:06x} is not a compact model (box is compact-only)"));
                }
                if compact && mode == "stdbox" {
                    return Err(format!("{oba:06x} is a compact model (stdbox is standard-format-only)"));
                }
                let mut sub = Subst::default();
                let mut points = cap.texture_point_ram.clone();
                let mut trom = texrom.clone();
                if mode == "remove" {
                    sub.models.push((oba, vec![0; orig.len()]));
                } else if mode == "scale" {
                    let mut w = orig.clone();
                    let (start, recs) = records_x(&w, compact);
                    let idx: Vec<usize> =
                        start.iter().copied().chain(recs.iter().flat_map(|r| r.1.iter().copied())).collect();
                    for (k, i) in idx.iter().enumerate() {
                        let keep = w[*i] & 1;
                        let v = f(w[*i]) * 1.5;
                        w[*i] = if compact && k % 3 == 0 { (v.to_bits() & !1) | keep } else { v.to_bits() };
                    }
                    sub.models.push((oba, w));
                } else {
                    let first = records_x(&orig, compact).1.first().map_or(0, |r| orig[r.0]);
                    let (mesh, nrec, uvs_per_rec) = if mode == "box" {
                        (box_mesh(&orig, first), 8, Vec::new())
                    } else {
                        let (m, nrec, kinds) = std_mesh(&orig, first);
                        (m, nrec, kinds)
                    };
                    sub.counts.push((oba, if mode == "box" { 8 } else { nrec as u32 + 1 }));
                    sub.models.push((oba, mesh));
                    // UVs: the mesh's faces take the original's first polygon's.
                    let probe = Subst { models: sub.models.clone(), counts: sub.counts.clone(), ..Default::default() };
                    let (s1, _) = sf.render_subst(&probe, false);
                    let obj = seen.iter().position(|s| s.0 == oba).unwrap();
                    let blocks: Vec<usize> =
                        s1.iter().enumerate().filter(|(_, &w)| w >> 24 == 0xff && w & 15 == 1).map(|(i, _)| i).collect();
                    let at = blocks[obj];
                    let tpa = s1[at + 1] & 0xff_ffff;
                    let src = Sources {
                        texture_rom: &texrom,
                        texture_point_ram: &cap.texture_point_ram,
                        render_ram: &cap.render_ram,
                        palette: &cap.palette,
                        xoff: 0.0,
                        yoff: 0.0,
                    };
                    let first_quad = records_x(&orig, compact).1.first().is_some_and(|r| orig[r.0] & 1 != 0);
                    let pw = |a: u32| -> u16 {
                        if a & 0x80_0000 != 0 {
                            src.texture_point_ram.get((a & 0xffff) as usize).copied().unwrap_or(0)
                        } else {
                            src.texture_rom.get((a & 0x7f_ffff) as usize).copied().unwrap_or(0)
                        }
                    };
                    let mut put = |a: u32, v: u16| {
                        if a & 0x80_0000 != 0 {
                            points[(a & 0xffff) as usize] = v;
                        } else {
                            trom[(a & 0x7f_ffff) as usize] = v;
                        }
                    };
                    let nouv = std::env::var_os("MP_NOUV").is_some();
                    if mode == "box" {
                        let uv: Vec<u16> = if first_quad {
                            (0..8).map(|k| pw(tpa + k)).collect()
                        } else {
                            vec![0, 0, 0, 256, 256, 256, 256, 0]
                        };
                        for poly in 0..8u32 {
                            for (k, &v) in uv.iter().enumerate() {
                                put(tpa + poly * 8 + k as u32, v);
                            }
                        }
                    } else if !nouv {
                        // The original's first polygon's corner pairs (v, u); a quad of a
                        // triangle-first model gets a fourth corner completing a parallelogram.
                        let mut c: Vec<[u16; 2]> = (0..if first_quad { 4 } else { 3 }).map(|k| [pw(tpa + 2 * k), pw(tpa + 2 * k + 1)]).collect();
                        if c.len() == 3 {
                            c.push([c[0][0].wrapping_add(c[2][0]).wrapping_sub(c[1][0]), c[0][1].wrapping_add(c[2][1]).wrapping_sub(c[1][1])]);
                        }
                        let mut at = tpa;
                        for &k in &uvs_per_rec {
                            // k: 0 = hidden restart quad (zeros), 3 = triangle, 4 = quad
                            for j in 0..if k == 0 { 4 } else { k as usize } {
                                let pair = if k == 0 { [0, 0] } else { c[j] };
                                put(at, pair[0]);
                                put(at + 1, pair[1]);
                                at += 2;
                            }
                        }
                    }
                    if nframes == 1 {
                        println!(
                            "{mode}: original {} words ({} polygons), list count {n:08x}, tpa {tpa:06x}, new model {} words, {nrec} records",
                            orig.len(),
                            records_x(&orig, compact).1.len(),
                            sub.models[0].1.len()
                        );
                    }
                }
                let t0 = Instant::now();
                let (s_sub, _) = sf.render_subst(&sub, false);
                t_sub += t0.elapsed().as_secs_f64();
                let before = draw(&cap.stream, &cap, &texrom, &cap.texture_point_ram, &view, windows, &mut gpu);
                let after = draw(&s_sub, &cap, &trom, &points, &view, windows, &mut gpu);
                let (mut diff, mut sx, mut sy) = (0usize, 0usize, 0usize);
                let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
                for (k, (x, y)) in before.chunks(3).zip(after.chunks(3)).enumerate() {
                    if x != y {
                        let (px, py) = (k % view.width, k / view.width);
                        diff += 1;
                        sx += px;
                        sy += py;
                        (x0, y0, x1, y1) = (x0.min(px), y0.min(py), x1.max(px), y1.max(py));
                    }
                }
                let at = if diff > 0 { format!("centroid {},{} bbox {x0},{y0}-{x1},{y1}", sx / diff, sy / diff) } else { String::new() };
                println!("frame {}: stream {} -> {} words, pixels changed {} {at}", cap.frame, cap.stream.len(), s_sub.len(), diff);
                if mode == "stdbox" {
                    // The object's polygons as the geometrizer sent them (attr, luma word with
                    // the back-face flag at bit 23 and luma at 15-22): real ones (link != 0).
                    let bl: Vec<usize> = s_sub.iter().enumerate().filter(|(_, &w)| w >> 24 == 0xff && w & 15 == 1).map(|(i, _)| i).collect();
                    let at = bl[seen.iter().position(|s| s.0 == oba).unwrap()];
                    let mut i = at + 9;
                    let (mut front, mut back, mut lumas) = (0, 0, Vec::new());
                    while i < s_sub.len() && s_sub[i] >> 24 != 0xff && s_sub[i] & 3 != 0 {
                        let (attr, l) = (s_sub[i], s_sub[i + 1]);
                        if (attr >> 8) & 3 != 0 {
                            if l >> 23 & 1 != 0 {
                                back += 1;
                            } else {
                                front += 1;
                            }
                            lumas.push((l >> 15) & 0xff);
                        }
                        i += if attr & 1 != 0 { 9 } else { 6 };
                    }
                    println!("    box polygons sent: front-facing {front}, back-facing flagged {back}; luma words {lumas:?}");
                }
                if let Some(name) = std::env::var_os("MP_DUMP") {
                    let d = std::path::Path::new(&name);
                    let _ = std::fs::write(d.join(format!("{}-after.png", cap.frame)), png::encode_rgb(view.width, view.height, &after));
                }
                strip.push((before, after));
            }
            _ => return Err("MODE: list none identity scale box stdbox remove".into()),
        }
    }
    if !strip.is_empty() {
        let (mut w, mut h) = (view.width, view.height);
        // MP_CROP=x0,y0,x1,y1,zoom: crop every picture and enlarge it (nearest).
        if let Some(c) = std::env::var("MP_CROP").ok().map(|c| c.split(',').filter_map(|v| v.parse::<usize>().ok()).collect::<Vec<_>>()) {
            if c.len() == 5 {
                let (x0, y0, x1, y1, z) = (c[0], c[1], c[2].min(w), c[3].min(h), c[4].max(1));
                let crop = |img: &Vec<u8>| -> Vec<u8> {
                    let mut o = Vec::new();
                    for y in y0..y1 {
                        let mut row = Vec::new();
                        for x in x0..x1 {
                            for _ in 0..z {
                                row.extend_from_slice(&img[(y * w + x) * 3..(y * w + x) * 3 + 3]);
                            }
                        }
                        for _ in 0..z {
                            o.extend_from_slice(&row);
                        }
                    }
                    o
                };
                for p in strip.iter_mut() {
                    *p = (crop(&p.0), crop(&p.1));
                }
                (w, h) = ((x1 - x0) * z, (y1 - y0) * z);
            }
        }
        let n = strip.len().min(5);
        let mut rgb = vec![0u8; w * n * 2 * h * 3];
        for (c, (a, b)) in strip.iter().take(n).enumerate() {
            for (r, img) in [a, b].iter().enumerate() {
                for y in 0..h {
                    let d = ((r * h + y) * w * n + c * w) * 3;
                    rgb[d..d + w * 3].copy_from_slice(&img[y * w * 3..(y + 1) * w * 3]);
                }
            }
        }
        std::fs::write(out.join(format!("{mode}{}-strip.png", std::env::var("MP_TAG").unwrap_or_default())), png::encode_rgb(w * n, h * 2, &rgb)).map_err(|e| e.to_string())?;
    }
    println!(
        "frames {nframes}; rerun time per frame: unchanged {:.2} ms, with substitution {:.2} ms; state hash {:016x}",
        1000.0 * t_plain / nframes.max(1) as f64,
        1000.0 * t_sub / nframes.max(1) as f64,
        m.bus.state_hash()
    );
    Ok(())
}
