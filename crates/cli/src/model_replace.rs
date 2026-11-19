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
fn rom_words(rom: &[u32], mask: u32, base: u32, end: u32) -> Vec<u32> {
    let n = end.wrapping_sub(base) & 0xff_ffff;
    (0..n).map(|k| rom[((base + k) & 0xff_ffff & mask) as usize]).collect()
}

/// A compact model's records: (word index of the attribute, vertex word
/// indices). The two start vertices come first (attribute index None).
fn records(w: &[u32]) -> (Vec<usize>, Vec<(usize, Vec<usize>)>) {
    let start: Vec<usize> = (0..6).collect();
    let mut recs = Vec::new();
    let mut i = 6;
    while i < w.len() {
        let attr = w[i];
        if attr & 3 == 0 {
            break;
        }
        let k = if attr & 1 != 0 { 6 } else { 3 };
        recs.push((i, (i + 1..(i + 1 + k).min(w.len())).collect()));
        i += 1 + k;
    }
    (start, recs)
}

fn draw(stream: &[u32], cap: &capture::Frame, texrom: &[u16], points: &[u16], view: &View, windows: (&[usize], &[usize])) -> Vec<u8> {
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
        let words_of = |oba: u32| -> Option<(Vec<u32>, bool, u32)> {
            seen.iter().find(|s| s.0 == oba).map(|s| (rom_words(&rom, mask, s.0, s.1), s.2, s.3))
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
                        sub.models.push((s.0, rom_words(&rom, mask, s.0, s.1)));
                    }
                }
                let t0 = Instant::now();
                let (s_sub, _) = sf.render_subst(&sub, false);
                t_sub += t0.elapsed().as_secs_f64();
                let a = draw(&cap.stream, &cap, &texrom, &cap.texture_point_ram, &view, windows);
                let b = draw(&s_sub, &cap, &texrom, &cap.texture_point_ram, &view, windows);
                let diff = a.chunks(3).zip(b.chunks(3)).filter(|(x, y)| x != y).count();
                println!(
                    "frame {}: models {} | machine stream {} words, rerun-with-own-words {} words, equal {} | rerun without override equal {} | differing pixels {}",
                    cap.frame,
                    sub.models.len(),
                    cap.stream.len(),
                    s_sub.len(),
                    s_sub == cap.stream,
                    s_plain == cap.stream,
                    diff
                );
            }
            "scale" | "box" => {
                let oba = oba_arg.ok_or("OBA wanted")?;
                let Some((orig, compact, n)) = words_of(oba) else {
                    println!("frame {}: model {oba:06x} not drawn", cap.frame);
                    continue;
                };
                if !compact {
                    return Err(format!("{oba:06x} is not a compact model"));
                }
                let mut sub = Subst::default();
                let mut points = cap.texture_point_ram.clone();
                let mut trom = texrom.clone();
                if mode == "scale" {
                    let mut w = orig.clone();
                    let (start, recs) = records(&w);
                    let idx: Vec<usize> =
                        start.iter().copied().chain(recs.iter().flat_map(|r| r.1.iter().copied())).collect();
                    for (k, i) in idx.iter().enumerate() {
                        let keep = w[*i] & 1;
                        let v = f(w[*i]) * 1.5;
                        w[*i] = if k % 3 == 0 { (v.to_bits() & !1) | keep } else { v.to_bits() };
                    }
                    sub.models.push((oba, w));
                } else {
                    let first = records(&orig).1.first().map_or(0, |r| orig[r.0]);
                    let mesh = box_mesh(&orig, first);
                    sub.counts.push((oba, 8));
                    sub.models.push((oba, mesh));
                    // UVs: the box's faces take the original's first polygon's.
                    let probe = Subst { models: sub.models.clone(), counts: sub.counts.clone(), ..Default::default() };
                    let (s1, _) = sf.render_subst(&probe, false);
                    let obj = seen.iter().position(|s| s.0 == oba).unwrap();
                    let blocks: Vec<usize> =
                        s1.iter().enumerate().filter(|(_, &w)| w >> 24 == 0xff && w & 15 == 1).map(|(i, _)| i).collect();
                    let at = blocks[obj];
                    let tpa = s1[at + 1] & 0xff_ffff;
                    let src = Sources {
                        texture_rom: &texrom,
                        texture_point_ram: &points,
                        render_ram: &cap.render_ram,
                        palette: &cap.palette,
                        xoff: 0.0,
                        yoff: 0.0,
                    };
                    let first_quad = records(&orig).1.first().is_some_and(|r| orig[r.0] & 1 != 0);
                    let pw = |a: u32| -> u16 {
                        if a & 0x80_0000 != 0 {
                            src.texture_point_ram.get((a & 0xffff) as usize).copied().unwrap_or(0)
                        } else {
                            src.texture_rom.get((a & 0x7f_ffff) as usize).copied().unwrap_or(0)
                        }
                    };
                    let uv: Vec<u16> = if first_quad {
                        (0..8).map(|k| pw(tpa + k)).collect()
                    } else {
                        vec![0, 0, 0, 256, 256, 256, 256, 0]
                    };
                    for poly in 0..8u32 {
                        for (k, &v) in uv.iter().enumerate() {
                            let a = tpa + poly * 8 + k as u32;
                            if a & 0x80_0000 != 0 {
                                points[(a & 0xffff) as usize] = v;
                            } else {
                                trom[(a & 0x7f_ffff) as usize] = v;
                            }
                        }
                    }
                    if cap.frame % 1 == 0 && nframes == 1 {
                        println!("box: original {} words, list count {}, tpa {:06x}", orig.len(), n, tpa);
                    }
                }
                let t0 = Instant::now();
                let (s_sub, _) = sf.render_subst(&sub, false);
                t_sub += t0.elapsed().as_secs_f64();
                let before = draw(&cap.stream, &cap, &texrom, &cap.texture_point_ram, &view, windows);
                let after = draw(&s_sub, &cap, &trom, &points, &view, windows);
                let diff = before.chunks(3).zip(after.chunks(3)).filter(|(x, y)| x != y).count();
                println!("frame {}: stream {} -> {} words, pixels changed {}", cap.frame, cap.stream.len(), s_sub.len(), diff);
                strip.push((before, after));
            }
            _ => return Err("MODE: list none identity scale box".into()),
        }
    }
    if !strip.is_empty() {
        let (w, h) = (view.width, view.height);
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
        std::fs::write(out.join(format!("{mode}-strip.png")), png::encode_rgb(w * n, h * 2, &rgb)).map_err(|e| e.to_string())?;
    }
    println!(
        "frames {nframes}; rerun time per frame: unchanged {:.2} ms, with substitution {:.2} ms; state hash {:016x}",
        1000.0 * t_plain / nframes.max(1) as f64,
        1000.0 * t_sub / nframes.max(1) as f64,
        m.bus.state_hash()
    );
    Ok(())
}
