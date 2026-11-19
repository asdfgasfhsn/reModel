//! SPIKE (tier 2) `remodel uplift-mesh MODE FROM COUNT OUT OBA [OBA2]`: a free
//! mesh (generated sphere / torus, generated RGBA texture) drawn in place of
//! model OBA (A: the original removed by the tier-1 substitution) and anchored
//! to model OBA2 (B: scenery, original kept), in software, from the side
//! channel's block records only. MODE: none | ab | clip. Env: MP_KEY, MP_SMOOTH,
//! MP_FLIP, MP_OFF=x,y,z (B's offset in OBA2's units, as multiples of its extent).
use model2::capture;
use model2::geo::Subst;
use model2::light::{Block, Lamp};
use model2::raster::{self, Poly, Sources};
use model2::uplift::{self, Instance, KeyMode, Mesh, Place, Skin};
use model2::video::{self, View};
use model2::Machine;
use romset::RomSet;
use std::path::Path;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::Instant;

fn f24(w: u32) -> f32 {
    f32::from_bits(w << 8)
}

/// A model's vertices in stream order (P0', P1', then each record's Pa and,
/// for quads, Pb), with the matrix select (compact models: bit 0 of x).
fn model_vertices(w: &[u32], compact: bool) -> Vec<([f32; 3], usize)> {
    let v = |i: usize| -> ([f32; 3], usize) {
        let sel = if compact { (w[i] & 1) as usize } else { 0 };
        let x = if compact { f32::from_bits(w[i] & !1) } else { f32::from_bits(w[i]) };
        ([x, f32::from_bits(w[i + 1]), f32::from_bits(w[i + 2])], sel)
    };
    let mut out = vec![v(0), v(3)];
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
        out.push(v(first));
        if quad {
            out.push(v(first + 3));
        }
        i += len;
    }
    out
}

/// A block's vertices in the stream (view space x*fx, y*fy, z) in order.
fn stream_vertices(s: &[u32], at: usize) -> Vec<[f32; 3]> {
    let v = |i: usize| [f24(s[i]), f24(s[i + 1]), f24(s[i + 2])];
    let mut out = vec![v(at + 3), v(at + 6)];
    let mut i = at + 9;
    while i + 6 <= s.len() && s[i] >> 24 != 0xff && s[i] & 3 != 0 {
        let quad = s[i] & 1 != 0;
        out.push(v(i + 3));
        if quad {
            out.push(v(i + 6));
        }
        i += if quad { 9 } else { 6 };
    }
    out
}

fn bounds(vs: &[([f32; 3], usize)], only0: bool) -> ([f32; 3], [f32; 3]) {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for (p, s) in vs {
        if only0 && *s != 0 || p.iter().any(|c| !c.is_finite()) {
            continue;
        }
        for a in 0..3 {
            lo[a] = lo[a].min(p[a]);
            hi[a] = hi[a].max(p[a]);
        }
    }
    (std::array::from_fn(|a| (lo[a] + hi[a]) / 2.0), std::array::from_fn(|a| (hi[a] - lo[a]) / 2.0))
}

fn words_of(rec: &Subst, rom: &[u32], mask: u32, oba: u32) -> Option<(Vec<u32>, bool)> {
    let seen = rec.seen.lock().unwrap().clone();
    let ram = rec.ram.lock().unwrap().clone();
    seen.iter().find(|s| s.0 == oba).map(|s| {
        let w = if s.0 & 0x80_0000 != 0 {
            crate::model_replace::rom_words(rom, mask, s.0, s.1)
        } else {
            (0..s.1.wrapping_sub(s.0) & 0xff_ffff).map(|k| ram.get(&(s.0 + k)).map_or(0, |e| e.0)).collect()
        };
        (w, s.2)
    })
}

struct Ctx<'a> {
    cap: &'a capture::Frame,
    texrom: &'a [u16],
    view: &'a View,
}

fn render(polys: &mut Vec<Poly>, c: &Ctx, depth: bool) -> Vec<u8> {
    let src = Sources {
        texture_rom: c.texrom,
        texture_point_ram: &c.cap.texture_point_ram,
        render_ram: &c.cap.render_ram,
        palette: &c.cap.palette,
        xoff: c.cap.xoff,
        yoff: c.cap.yoff,
    };
    let layer = raster::draw_view_depth(polys, &src, c.view, depth);
    video::compose(&c.cap.tiles, &c.cap.palette, Some((&layer.0, &layer.1)), c.view)
}

fn magenta(img: &[u8]) -> usize {
    img.chunks(3).filter(|p| p[0] == 255 && p[1] == 0 && p[2] == 255).count()
}

fn crop(img: &[u8], w: usize, h: usize, cx: f32, cy: f32, cw: usize, ch: usize, z: usize) -> Vec<u8> {
    let x0 = (cx as i64 - cw as i64 / 2).clamp(0, (w - cw) as i64) as usize;
    let y0 = (cy as i64 - ch as i64 / 2).clamp(0, (h - ch) as i64) as usize;
    let mut o = Vec::with_capacity(cw * ch * z * z * 3);
    for y in 0..ch {
        let mut row = Vec::new();
        for x in 0..cw {
            for _ in 0..z {
                row.extend_from_slice(&img[((y0 + y) * w + x0 + x) * 3..((y0 + y) * w + x0 + x) * 3 + 3]);
            }
        }
        for _ in 0..z {
            o.extend_from_slice(&row);
        }
    }
    o
}

fn strip(rows: &[Vec<Vec<u8>>], w: usize, h: usize, path: &Path) {
    let n = rows[0].len();
    let mut rgb = vec![0u8; w * n * h * rows.len() * 3];
    for (r, row) in rows.iter().enumerate() {
        for (c, img) in row.iter().enumerate() {
            for y in 0..h {
                let d = ((r * h + y) * w * n + c * w) * 3;
                rgb[d..d + w * 3].copy_from_slice(&img[y * w * 3..(y + 1) * w * 3]);
            }
        }
    }
    let _ = std::fs::write(path, png::encode_rgb(w * n, h * rows.len(), &rgb));
}

#[derive(Default)]
struct Stat {
    v: Vec<f64>,
}
impl Stat {
    fn add(&mut self, x: f64) {
        self.v.push(x);
    }
    fn show(&self) -> String {
        if self.v.is_empty() {
            return "-".into();
        }
        let mean = self.v.iter().sum::<f64>() / self.v.len() as f64;
        format!("mean {:.3} worst {:.3} (n={})", mean, self.v.iter().cloned().fold(f64::MIN, f64::max), self.v.len())
    }
}

pub fn run(roms: &RomSet, mode: &str, from: u64, count: u64, out: &Path, oba: u32, oba2: Option<u32>, spec: &str) -> Result<(), String> {
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
    let sphere = uplift::uv_sphere(12, 18);
    let torus = uplift::torus(16, 10, 0.32);
    let skin = Arc::new(Skin { img: uplift::grid_texture(256) });
    let keyname = std::env::var("MP_KEY").unwrap_or_else(|_| "tri-min".into());
    let smooth = std::env::var_os("MP_FLAT").is_none();
    let flip = std::env::var_os("MP_FLIP").is_some();
    let off: [f32; 3] = std::env::var("MP_OFF")
        .ok()
        .and_then(|s| {
            let v: Vec<f32> = s.split(',').filter_map(|x| x.parse().ok()).collect();
            (v.len() == 3).then(|| [v[0], v[1], v[2]])
        })
        .unwrap_or([1.3, 0.0, 0.0]);
    let pick = (0..7).map(|i| from + (i * (count.max(2) - 1)) / 6).collect::<Vec<_>>();

    // Statistics.
    let (mut s_vert, mut s_cent, mut s_drift) = (Stat::default(), Stat::default(), Stat::default());
    let (mut l_orig, mut l_mesh, mut l_b_orig, mut l_b_mesh) = (Stat::default(), Stat::default(), Stat::default(), Stat::default());
    let (mut t_polys, mut t_draw_with, mut t_draw_without) = (0.0f64, 0.0f64, 0.0f64);
    let names = ["block-median", "centre", "near", "tri-min", "tri-max", "depth"];
    let mut order: Vec<(Stat, Stat)> = (0..names.len() * 2).map(|_| Default::default()).collect();
    let mut order_b: Vec<(Stat, Stat)> = (0..names.len()).map(|_| Default::default()).collect();
    let (mut rows_a, mut rows_b, mut rows_c): (Vec<Vec<Vec<u8>>>, Vec<Vec<Vec<u8>>>, Vec<Vec<Vec<u8>>>) =
        (vec![vec![], vec![]], vec![vec![], vec![]], vec![vec![], vec![]]);
    let (mut frames, mut missing, mut inst_max, mut tris_total) = (0u32, 0u32, 0usize, 0usize);
    let mut notes: Vec<String> = Vec::new();
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
        let mut base = raster::build_shaded(&s_plain, &src, None);
        let places_plain = uplift::walk(&s_plain, cap.xoff, cap.yoff);
        if places_plain.len() != side.blocks.len() {
            notes.push(format!("frame {}: walk found {} blocks, side channel {}", cap.frame, places_plain.len(), side.blocks.len()));
            continue;
        }
        frames += 1;
        // The original's visible pixels' mean game luma, by block.
        let mut bp = base.clone();
        let placed = raster::place(&mut bp, &view);
        let vis = raster::visible(&bp, &placed, &src, &view);
        let mut lum_of = |blocks: &[usize]| -> (f64, usize) {
            let (mut s, mut n) = (0f64, 0usize);
            for &k in vis.owner.iter().filter(|&&k| k != u32::MAX) {
                let p = &bp[placed[k as usize].poly];
                if blocks.contains(&(p.block as usize)) {
                    s += p.luma as f64;
                    n += 1;
                }
            }
            (s, n)
        };
        let t0 = Instant::now();
        let mut img0 = {
            let mut p = base.clone();
            render(&mut p, &ctx, false)
        };
        t_draw_without += t0.elapsed().as_secs_f64();

        let lamp_of = |k: usize| -> Option<(Lamp, [u32; 3])> {
            let b = side.blocks[k];
            let faces = side.faces_of(&b);
            let mut counts = std::collections::HashMap::new();
            for f in faces {
                *counts.entry(f.mat).or_insert(0) += 1;
            }
            let mat = counts.iter().max_by_key(|e| *e.1).map(|e| *e.0)?;
            let face = faces.iter().find(|f| f.mat == mat)?;
            let colour = base.iter().find(|p| p.block as usize == k)?.colour;
            Some((Lamp::of(&b, face, &side.materials[mat as usize]), colour))
        };

        // ---- Test A: replace OBA.
        let ks: Vec<usize> = (0..side.blocks.len()).filter(|&k| side.blocks[k].oba & 0xff_ffff == oba).collect();
        let Some((orig, compact)) = words_of(&rec, &rom, mask, oba) else {
            missing += 1;
            notes.push(format!("frame {}: model {oba:06x} not drawn", cap.frame));
            continue;
        };
        let mut sub = Subst::default();
        sub.models.push((oba, vec![0; orig.len()]));
        let (s_sub, side_sub) = sf.render_subst(&sub, true);
        let side_sub = side_sub.ok_or("no side channel (sub)")?;
        let places_sub = uplift::walk(&s_sub, cap.xoff, cap.yoff);
        let mv = model_vertices(&orig, compact);
        let (c0, e0) = bounds(&mv, true);
        inst_max = inst_max.max(ks.len());
        let mut a_polys_all: Vec<Poly> = Vec::new();
        let mut centre_px = (248.0f32, 192.0f32);
        let mut tp = 0.0;
        for &k in &ks {
            let Some((lamp, colour)) = lamp_of(k) else { continue };
            let b = side_sub.blocks[k];
            let pl = places_sub[k];
            // M1: reprojection of every original vertex, recorded transform vs the game's.
            let sv = stream_vertices(&s_plain, places_plain[k].at);
            let pb = side.blocks[k];
            if sv.len() == mv.len() {
                let mut cx = (0f64, 0f64, 0usize);
                for ((p, sel), s) in mv.iter().zip(&sv) {
                    let v = uplift::to_view(&pb.xf, *sel, *p);
                    let (mx, my) = uplift::project(&pb, &places_plain[k], v);
                    let (gx, gy) = (places_plain[k].centre.0 + s[0] / s[2], places_plain[k].centre.1 - s[1] / s[2]);
                    if gx.abs() < 4000.0 && gy.abs() < 4000.0 && s[2] > 0.0 {
                        s_vert.add((((mx - gx) as f64).powi(2) + ((my - gy) as f64).powi(2)).sqrt());
                        cx.0 += gx as f64;
                        cx.1 += gy as f64;
                        cx.2 += 1;
                    }
                }
                if cx.2 > 0 {
                    centre_px = ((cx.0 / cx.2 as f64) as f32, (cx.1 / cx.2 as f64) as f32);
                }
            } else {
                notes.push(format!("frame {}: vertex count model {} stream {}", cap.frame, mv.len(), sv.len()));
            }
            let zc = uplift::to_view(&b.xf, 0, c0)[2];
            let med = {
                let mut ks: Vec<f32> = base.iter().filter(|p| p.block as usize == k).map(|p| p.key).collect();
                ks.sort_by(|a, b| a.total_cmp(b));
                ks.get(ks.len() / 2).copied().unwrap_or(zc)
            };
            let keymode = |n: &str| match n {
                "block-median" => KeyMode::Fixed(med),
                "centre" => KeyMode::Fixed(zc),
                "near" => KeyMode::Near,
                "tri-max" => KeyMode::TriMax,
                _ => KeyMode::TriMin,
            };
            let inst = |kn: &str, sc: f32, dbl: bool| Instance {
                mesh: &sphere,
                skin: skin.clone(),
                scale: e0.map(|e| e * sc),
                offset: c0,
                lamp,
                colour,
                key: keymode(kn),
                double_sided: dbl,
                smooth,
                flip,
            };
            if frames == 1 {
                println!("lamp A: {:?} colour {:?} bbox centre {:?} half {:?} compact {}", lamp, colour, c0, e0, compact);
            }
            let t0 = Instant::now();
            let main = uplift::polys(&inst(&keyname, 1.0, false), &b, &pl);
            tp += t0.elapsed().as_secs_f64();
            tris_total = main.len();
            // M2: projected centre of the mesh's vertices vs the original's vertex centroid.
            let pts = main.iter().flat_map(|p| p.points.iter()).collect::<Vec<_>>();
            if !pts.is_empty() {
                let mc = (pts.iter().map(|p| p.x as f64).sum::<f64>() / pts.len() as f64, pts.iter().map(|p| p.y as f64).sum::<f64>() / pts.len() as f64);
                // Compare with the original's vertex cloud: both centroids of screen vertices.
                s_cent.add(((mc.0 - centre_px.0 as f64).powi(2) + (mc.1 - centre_px.1 as f64).powi(2)).sqrt());
            }
            a_polys_all.extend(main);
            // Sort / depth options (counted by magenta pixels vs the depth reference).
            if frames % 3 == 1 {
                uplift::MARK.store(true, Relaxed);
                let mut imgs = Vec::new();
                for (oi, n) in names.iter().enumerate() {
                    let mut p: Vec<Poly> = base.iter().filter(|p| !ks.contains(&(p.block as usize))).cloned().collect();
                    let kn = if *n == "depth" { "tri-min" } else { n };
                    p.extend(uplift::polys(&inst(kn, 1.0, false), &b, &pl));
                    imgs.push(render(&mut p, &ctx, *n == "depth"));
                    let _ = oi;
                }
                uplift::MARK.store(false, Relaxed);
                let refm: Vec<bool> = imgs[5].chunks(3).map(|p| p[0] == 255 && p[1] == 0 && p[2] == 255).collect();
                for (oi, im) in imgs.iter().enumerate() {
                    let n = magenta(im);
                    let d = im.chunks(3).zip(&refm).filter(|(p, r)| (p[0] == 255 && p[1] == 0 && p[2] == 255) != **r).count();
                    order[oi].0.add(n as f64);
                    order[oi].1.add(d as f64);
                }
            }
            if mode == "clip" || (frames % 6 == 1 && mode == "ab") {
                // Clipping: a mesh larger than its distance to the eye, double-sided.
                let sc = 1.3 * zc / e0.iter().cloned().fold(0.0, f32::max);
                let mut p: Vec<Poly> = base.iter().filter(|p| !ks.contains(&(p.block as usize))).cloned().collect();
                p.extend(uplift::polys(&inst("tri-min", sc, true), &b, &pl));
                let img = render(&mut p, &ctx, false);
                if pick.contains(&cap.frame) || frames % 6 == 1 {
                    rows_c[0].push(crop(&img0, vw, vh, 248.0, 192.0, 248, 192, 1));
                    rows_c[1].push(crop(&img, vw, vh, 248.0, 192.0, 248, 192, 1));
                }
            }
        }
        // Lighting: mesh pixels' mean luma (draw with the mesh alone, fresh counters).
        let mut with: Vec<Poly> = base.iter().filter(|p| !ks.contains(&(p.block as usize))).cloned().collect();
        let n_orig_removed = with.len();
        let _ = n_orig_removed;
        with.extend(a_polys_all.iter().cloned());
        let (lo_s, lo_n) = lum_of(&ks);
        if lo_n > 0 {
            l_orig.add(lo_s / lo_n as f64);
        }
        uplift::SKIN_PIXELS.store(0, Relaxed);
        uplift::SKIN_LUMA.store(0, Relaxed);
        let t0 = Instant::now();
        let img_a = render(&mut with, &ctx, false);
        t_draw_with += t0.elapsed().as_secs_f64();
        let (sp, sl) = (uplift::SKIN_PIXELS.load(Relaxed), uplift::SKIN_LUMA.load(Relaxed));
        if sp > 0 {
            l_mesh.add(sl as f64 / sp as f64);
        }
        t_polys += tp;
        println!(
            "A frame {}: blocks {:?}, mesh tris drawn {}, mesh px {}, mean luma mesh {:.1} orig {:.1}, orig px {}, centre-dist {:.2} px",
            cap.frame,
            ks,
            tris_total,
            sp,
            if sp > 0 { sl as f64 / sp as f64 } else { 0.0 },
            if lo_n > 0 { lo_s / lo_n as f64 } else { 0.0 },
            lo_n,
            s_cent.v.last().copied().unwrap_or(-1.0)
        );
        if pick.contains(&cap.frame) {
            rows_a[0].push(crop(&img0, vw, vh, centre_px.0, centre_px.1, 160, 120, 2));
            rows_a[1].push(crop(&img_a, vw, vh, centre_px.0, centre_px.1, 160, 120, 2));
        }
        let _ = &mut img0;

        // ---- Test B: anchor to OBA2, original kept.
        if let Some(oba2) = oba2 {
            let kb: Vec<usize> = (0..side.blocks.len()).filter(|&k| side.blocks[k].oba & 0xff_ffff == oba2).collect();
            if let Some((w2, c2)) = words_of(&rec, &rom, mask, oba2) {
                let mv2 = model_vertices(&w2, c2);
                let (cc, ee) = bounds(&mv2, true);
                let r = std::env::var("MP_R").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.35) * ee.iter().cloned().fold(0.0, f32::max);
                let mut bpolys: Vec<Poly> = Vec::new();
                let mut anchor_px = (248.0f32, 192.0f32);
                let mut lines = Vec::new();
                for &k in &kb {
                    let Some((lamp, colour)) = lamp_of(k) else { continue };
                    let b = side.blocks[k];
                    let pl = places_plain[k];
                    // Drift: the anchor point (model origin of the scenery's first vertex) vs the game's.
                    let sv = stream_vertices(&s_plain, pl.at);
                    if sv.len() == mv2.len() {
                        for ((p, sel), s) in mv2.iter().zip(&sv).take(40) {
                            let (mx, my) = uplift::project(&b, &pl, uplift::to_view(&b.xf, *sel, *p));
                            let (gx, gy) = (pl.centre.0 + s[0] / s[2], pl.centre.1 - s[1] / s[2]);
                            if s[2] > 0.0 && gx.abs() < 4000.0 && gy.abs() < 4000.0 {
                                s_drift.add((((mx - gx) as f64).powi(2) + ((my - gy) as f64).powi(2)).sqrt());
                            }
                        }
                    }
                    let offset = [cc[0] + off[0] * ee[0] + r * off[0].signum(), cc[1] + off[1] * ee[1], cc[2] + off[2] * ee[2]];
                    let zc = uplift::to_view(&b.xf, 0, offset)[2];
                    let med = {
                        let mut ks: Vec<f32> = base.iter().filter(|p| p.block as usize == k).map(|p| p.key).collect();
                        ks.sort_by(|a, b| a.total_cmp(b));
                        ks.get(ks.len() / 2).copied().unwrap_or(zc)
                    };
                    let mk = |kn: &str| Instance {
                        mesh: &torus,
                        skin: skin.clone(),
                        scale: [r; 3],
                        offset,
                        lamp,
                        colour,
                        key: match kn {
                            "block-median" => KeyMode::Fixed(med),
                            "centre" => KeyMode::Fixed(zc),
                            "near" => KeyMode::Near,
                            "tri-max" => KeyMode::TriMax,
                            _ => KeyMode::TriMin,
                        },
                        double_sided: false,
                        smooth,
                        flip,
                    };
                    if frames == 1 {
                        println!("lamp B: {:?} colour {:?}", lamp, colour);
                    }
                    let main = uplift::polys(&mk(&keyname), &b, &pl);
                    let ap = uplift::project(&b, &pl, uplift::to_view(&b.xf, 0, offset));
                    anchor_px = (ap.0, ap.1);
                    lines.push(format!("block {k} anchor ({:.1},{:.1}) z {:.1} tris {}", ap.0, ap.1, zc, main.len()));
                    bpolys.extend(main);
                    if frames % 3 == 1 {
                        uplift::MARK.store(true, Relaxed);
                        let mut imgs = Vec::new();
                        for n in names.iter() {
                            let mut p = base.clone();
                            let kn = if *n == "depth" { "tri-min" } else { n };
                            p.extend(uplift::polys(&mk(kn), &b, &pl));
                            imgs.push(render(&mut p, &ctx, *n == "depth"));
                        }
                        uplift::MARK.store(false, Relaxed);
                        let refm: Vec<bool> = imgs[5].chunks(3).map(|p| p[0] == 255 && p[1] == 0 && p[2] == 255).collect();
                        for (oi, im) in imgs.iter().enumerate() {
                            let d = im.chunks(3).zip(&refm).filter(|(p, r)| (p[0] == 255 && p[1] == 0 && p[2] == 255) != **r).count();
                            order_b[oi].0.add(magenta(im) as f64);
                            order_b[oi].1.add(d as f64);
                        }
                    }
                }
                let (bs, bn) = lum_of(&kb);
                if bn > 0 {
                    l_b_orig.add(bs / bn as f64);
                }
                let mut with = base.clone();
                with.extend(bpolys);
                uplift::SKIN_PIXELS.store(0, Relaxed);
                uplift::SKIN_LUMA.store(0, Relaxed);
                let img_b = render(&mut with, &ctx, false);
                let (sp, sl) = (uplift::SKIN_PIXELS.load(Relaxed), uplift::SKIN_LUMA.load(Relaxed));
                if sp > 0 {
                    l_b_mesh.add(sl as f64 / sp as f64);
                }
                println!("B frame {}: {} | mesh px {} mean luma {:.1} scenery luma {:.1}", cap.frame, lines.join("; "), sp, if sp > 0 { sl as f64 / sp as f64 } else { 0.0 }, if bn > 0 { bs / bn as f64 } else { 0.0 });
                if pick.contains(&cap.frame) {
                    rows_b[0].push(crop(&img0, vw, vh, anchor_px.0, anchor_px.1, 160, 120, 2));
                    rows_b[1].push(crop(&img_b, vw, vh, anchor_px.0, anchor_px.1, 160, 120, 2));
                }
            } else {
                notes.push(format!("frame {}: scenery {oba2:06x} not drawn", cap.frame));
            }
        }
        base.clear();
    }
    if !rows_a[0].is_empty() {
        strip(&rows_a, 320, 240, &out.join("strip-A.png"));
    }
    if !rows_b[0].is_empty() {
        strip(&rows_b, 320, 240, &out.join("strip-B.png"));
    }
    if !rows_c[0].is_empty() {
        strip(&rows_c, 248, 192, &out.join("strip-clip.png"));
    }
    println!("frames {frames} (model missing {missing}), max instances {inst_max}, mesh triangles {tris_total}");
    println!("M1 vertex reprojection error (px, every vertex of the original, recorded transform vs game): {}", s_vert.show());
    println!("M2 mesh centre vs original centre (px): {}", s_cent.show());
    println!("B anchor drift (px, first 40 vertices of the scenery model per frame): {}", s_drift.show());
    println!("lighting A: mean game luma mesh {} | original {}", l_mesh.show(), l_orig.show());
    println!("lighting B: mean game luma mesh {} | scenery {}", l_b_mesh.show(), l_b_orig.show());
    for (i, n) in names.iter().enumerate() {
        println!("order A {n:>13}: mesh px {} | differing from depth-path {}", order[i].0.show(), order[i].1.show());
    }
    for (i, n) in names.iter().enumerate() {
        println!("order B {n:>13}: mesh px {} | differing from depth-path {}", order_b[i].0.show(), order_b[i].1.show());
    }
    let nf = frames.max(1) as f64;
    println!(
        "cost per frame: mesh polygons {:.3} ms, draw with {:.1} ms without {:.1} ms (software, one render each)",
        1000.0 * t_polys / nf,
        1000.0 * t_draw_with / nf,
        1000.0 * t_draw_without / nf
    );
    for n in notes.iter().take(10) {
        println!("note: {n}");
    }
    println!("state hash {:016x}", m.bus.state_hash());
    let _ = Block::default();
    let _: Option<Mesh> = None;
    let _: Option<Place> = None;
    Ok(())
}
