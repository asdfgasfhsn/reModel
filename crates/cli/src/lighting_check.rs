//! `remodel lighting-check`: smooth shading (`model2::light`, the
//! stream-level layer) on captured frames. Each frame is drawn flat and
//! vertex-shaded on the software and GPU renderers, and the invariants
//! that need no ground truth are checked:
//!
//! - A: at crease 0 nothing is lit, and both renderers draw the flat frame
//!   byte for byte (with the GPU's shading option on);
//! - C: brightness: a lit polygon's corners average close to its own luma,
//!   and the frame's mean colour barely moves;
//! - D: every pixel whose visible polygon isn't lit is the flat frame's,
//!   on both renderers;
//! - E: software and GPU agree vertex-shaded about as well as flat;
//! - H, I: what the pass costs and what it did (lit, flat, excluded,
//!   crease edges, material splits).
//!
//! Pictures per frame: flat and vertex (software, whole frames), their
//! differences, and the polygons' classes in false colour. `--probe`
//! lists the polygons at a pixel with their corner values.

use crate::depth_check::Probe;
use model2::capture::{self, Frame};
use model2::light::{self, Class, Options};
use model2::raster::{self, Placed, Poly};
use model2::video::{self, View};
use std::fmt::Write as _;
use std::path::Path;
use std::time::Instant;

/// Pixels of two RGB images differing at all, and by more than 16 in a
/// channel; over `mask` (all if None).
fn compare(a: &[u8], b: &[u8], mask: Option<&[bool]>) -> (usize, usize) {
    let (mut differ, mut large) = (0, 0);
    for i in 0..a.len() / 3 {
        if mask.is_some_and(|m| !m[i]) {
            continue;
        }
        let m = (0..3).map(|c| a[i * 3 + c].abs_diff(b[i * 3 + c])).max().unwrap_or(0);
        differ += (m > 0) as usize;
        large += (m > 16) as usize;
    }
    (differ, large)
}

/// Software and GPU 3D layers differing (coverage, or by more than 16 in a
/// channel where both draw), as gpu-check counts them.
fn sw_gpu(sw: &(Vec<u8>, Vec<bool>), gpu: &(Vec<u8>, Vec<bool>)) -> (usize, usize) {
    let (mut cov, mut large) = (0, 0);
    for i in 0..sw.1.len() {
        if sw.1[i] != gpu.1[i] {
            cov += 1;
        } else if sw.1[i] && (0..3).any(|c| sw.0[i * 3 + c].abs_diff(gpu.0[i * 3 + c]) > 16) {
            large += 1;
        }
    }
    (cov, large)
}

/// The software renderer's 3D layer of `polys` in `view`.
fn soft(polys: &[Poly], src: &raster::Sources, view: &View) -> (Vec<u8>, Vec<bool>) {
    raster::draw_view(&mut polys.to_vec(), src, view)
}

/// A frame's polygons, anchored and placed (sorted) in `view`.
fn placed(mut polys: Vec<Poly>, f: &Frame, view: &View) -> (Vec<Poly>, Vec<Placed>) {
    video::HudLayout::new(&f.tiles, &f.palette, view).anchor(&mut polys, view);
    let placed = raster::place(&mut polys, view);
    (polys, placed)
}

/// False colour of a polygon's class (scaled by the flat frame's
/// brightness where drawn).
fn class_colour(p: &Poly, class: Option<Class>) -> [f32; 3] {
    match class {
        _ if !p.lit.is_empty() => [1.0, 0.55, 0.1], // lit (stream level, L0): orange
        Some(Class::Lit) => [0.2, 0.3, 0.9],        // lit but dropped: a shadow or checker
        Some(Class::Excluded) => [0.3, 0.5, 1.0],   // excluded (HUD windows): blue
        Some(Class::Degenerate) => [0.7, 0.3, 0.9], // no area: purple
        Some(Class::Flat) | None => [0.62, 0.62, 0.62], // took part, nothing varies: grey
        Some(Class::Hidden | Class::Culled) => [0.0, 0.8, 0.8],
    }
}

/// The layers picture: each visible pixel coloured by its polygon's class,
/// shaded by the flat frame's brightness; hard edges between faces that
/// share them drawn on top (crease: red, material: magenta) where that
/// polygon shows.
#[allow(clippy::too_many_arguments)]
fn layers_picture(
    polys: &[Poly],
    placed: &[Placed],
    src: &raster::Sources,
    view: &View,
    flat3d: &[u8],
    info: &std::collections::HashMap<u32, light::Info>,
) -> Vec<u8> {
    let (w, h) = (view.width, view.height);
    let vis = raster::visible(polys, placed, src, view);
    let mut out = vec![0u8; w * h * 3];
    for o in 0..w * h {
        let k = vis.owner[o];
        if k == u32::MAX {
            continue;
        }
        let p = &polys[placed[k as usize].poly];
        let c = class_colour(p, info.get(&p.at).map(|i| i.class));
        let l =
            (flat3d[o * 3] as f32 * 0.3 + flat3d[o * 3 + 1] as f32 * 0.59 + flat3d[o * 3 + 2] as f32 * 0.11) / 255.0;
        let s = 0.35 + 0.65 * l;
        for ch in 0..3 {
            out[o * 3 + ch] = (c[ch] * s * 255.0).round() as u8;
        }
    }
    // Hard edges, for polygons drawn whole (their points are the ring).
    for (k, pl) in placed.iter().enumerate() {
        let p = &polys[pl.poly];
        let Some(i) = info.get(&p.at) else { continue };
        let n = pl.points.len();
        if (i.crease | i.material) == 0 || !(3..=4).contains(&n) {
            continue;
        }
        for e in 0..n {
            let colour = if i.crease & (1 << e) != 0 {
                [255, 0, 0]
            } else if i.material & (1 << e) != 0 {
                [255, 0, 255]
            } else {
                continue;
            };
            let (a, b) = (pl.points[e], pl.points[(e + 1) % n]);
            let steps = ((b.x - a.x).abs().max((b.y - a.y).abs()).ceil() as usize).clamp(1, 8 * (w + h));
            for s in 0..=steps {
                let t = s as f32 / steps as f32;
                let (x, y) = (a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
                if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 {
                    continue;
                }
                let o = y as usize * w + x as usize;
                if vis.owner[o] == k as u32 {
                    out[o * 3..o * 3 + 3].copy_from_slice(&colour);
                }
            }
        }
    }
    out
}

/// Differences of two frames, scaled by 4 (grey), over the flat frame
/// dimmed to a quarter.
fn diff_picture(flat: &[u8], lit: &[u8]) -> Vec<u8> {
    flat.chunks(3)
        .zip(lit.chunks(3))
        .flat_map(|(a, b)| {
            let m = (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0);
            if m == 0 {
                [a[0] / 4, a[1] / 4, a[2] / 4]
            } else {
                let v = m.saturating_mul(4);
                if b.iter().map(|&x| x as u32).sum::<u32>() >= a.iter().map(|&x| x as u32).sum::<u32>() {
                    [v, v, (v / 2).max(40)] // brighter: white-yellow
                } else {
                    [(v / 2).max(40), v / 2, v] // darker: blue
                }
            }
        })
        .collect()
}

fn median(v: &mut [f32]) -> f32 {
    v.sort_by(f32::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

#[allow(clippy::too_many_arguments)]
fn report_probe(
    pr: &Probe,
    polys: &[Poly],
    placed: &[Placed],
    src: &raster::Sources,
    view: &View,
    info: &std::collections::HashMap<u32, light::Info>,
    (flat, lit): (&[u8], &[u8]),
    out: &mut String,
) {
    if pr.x >= view.width || pr.y >= view.height {
        let _ = writeln!(out, "probe {},{}@{}: outside the {}x{} view", pr.x, pr.y, pr.frame, view.width, view.height);
        return;
    }
    let i = (pr.y * view.width + pr.x) * 3;
    let r = raster::probe_depth(polys, placed, src, view, pr.x, pr.y);
    let _ = writeln!(
        out,
        "probe {},{}@{}: flat {:?}, vertex {:?}; {} polygons cover it, in draw order (the last drawn shows)",
        pr.x,
        pr.y,
        pr.frame,
        &flat[i..i + 3],
        &lit[i..i + 3],
        r.hits.len()
    );
    for hit in &r.hits {
        let p = &polys[placed[hit.index].poly];
        let inf = info.get(&p.at);
        let _ = writeln!(
            out,
            "  draw {:>5} window {} block {:>4} at {:>7} class {:<10} luma {:>3} colour {:?} textured {} corners {:?} hard edges crease {:04b} material {:04b}",
            hit.index,
            p.window,
            p.block,
            p.at,
            inf.map_or("-".to_string(), |i| format!("{:?}", i.class)),
            p.luma,
            p.colour,
            p.texture.is_some(),
            p.lit.iter().map(|l| (l[0] * 10.0).round() / 10.0).collect::<Vec<_>>(),
            inf.map_or(0, |i| i.crease),
            inf.map_or(0, |i| i.material),
        );
    }
}

/// GPU frame times (whole frame, compose and readback; ms, mean of 3 after
/// a warm-up) flat and vertex-shaded, at 1080p and 4K (16:9).
fn bench(
    gpu: &mut gpu::Renderer,
    frames: &[Frame],
    texture_rom: &[u16],
    opts: &Options,
    hud: (&[usize], &[usize]),
    out: &mut String,
) {
    let _ =
        writeln!(out, "bench         GPU whole frame incl. compose and readback, mean ms over {} frames", frames.len());
    for (name, height) in [("1080p", 1080), ("4K", 2160)] {
        let view = View::new(height, 16.0 / 9.0, video::Hud::Edges).with_hud_windows(hud.0, hud.1);
        let mut t = [0.0f64; 2];
        for f in frames {
            let src = f.sources(texture_rom);
            let layers = video::layers_2d(&f.tiles, &f.palette, &view, true);
            for (k, lit) in [false, true].into_iter().enumerate() {
                let built = if lit {
                    raster::build_shaded(&f.stream, &src, Some(opts))
                } else {
                    raster::build(&f.stream, &src)
                };
                let (polys, placed) = placed(built, f, &view);
                let q = if lit { super::gpu_light(Some(opts)) } else { gpu::Quality::default() };
                gpu.set_quality(q);
                gpu.frame(&polys, &placed, &src, &view, &layers);
                let s = Instant::now();
                for _ in 0..3 {
                    gpu.frame(&polys, &placed, &src, &view, &layers);
                }
                t[k] += s.elapsed().as_secs_f64() * 1e3 / 3.0;
            }
        }
        let n = frames.len().max(1) as f64;
        let _ = writeln!(
            out,
            "  {name:<6} flat {:>7.2} ms   vertex {:>7.2} ms   (+{:.2} ms)",
            t[0] / n,
            t[1] / n,
            (t[1] - t[0]) / n
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    root: &Path,
    hud_windows: (&[usize], &[usize]),
    view: Option<View>,
    opts: Options,
    out_dir: &Path,
    probes: &[Probe],
    bench_frames: Option<usize>,
    every: usize,
) -> Result<bool, String> {
    let texture_rom = capture::load_texture_rom(root)
        .map_err(|e| format!("{}: {e} (capture with `make capture`)", root.display()))?;
    let mut gpu = gpu::Renderer::new()?;
    let mut view = view.unwrap_or_else(|| View::new(1080, 4.0 / 3.0, video::Hud::Edges));
    view = view.with_hud_windows(hud_windows.0, hud_windows.1);
    let opts = Options { diagnose: true, ..opts };
    let zero = Options { crease: 0, ..opts };
    let flat_q = gpu::Quality::default();
    let lit_q = super::gpu_light(Some(&opts));
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let mut rep = String::new();
    let _ = writeln!(rep, "gpu           {}", gpu.adapter());
    let _ = writeln!(rep, "view          {}x{}", view.width, view.height);
    let _ = writeln!(
        rep,
        "shading       vertex (stream level), crease {} degrees, focal guess {} x window width",
        opts.crease,
        light::FOCAL_GUESS
    );
    let mut dirs: Vec<_> = std::fs::read_dir(root)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("meta.txt").exists())
        .collect();
    dirs.sort();
    if !probes.is_empty() {
        let frame = |d: &std::path::PathBuf| d.file_name()?.to_str()?.parse::<u64>().ok();
        dirs.retain(|d| frame(d).is_none_or(|n| probes.iter().any(|p| p.frame == n)));
    }
    let _ = writeln!(
        rep,
        "{:<7} {:>6} {:>6} {:>6} {:>5} {:>6} {:>6} {:>5} {:>8}  {:>3} {:>9} {:>14} {:>9} {:>17} {:>17}",
        "frame",
        "polys",
        "lit",
        "flat",
        "excl",
        "crease",
        "mater",
        "A",
        "C <=8",
        "D",
        "mean RGB",
        "changed px",
        "shade ms",
        "E flat >16/cov",
        "E lit >16/cov"
    );
    print!("{rep}");
    let mut failures: Vec<String> = Vec::new();
    // The design's initial targets for brightness (C): reported, not
    // failed (a mean of face lumas moves a polygon at a curvature change by
    // a quarter of its neighbour's difference, legitimately).
    let mut warnings: Vec<String> = Vec::new();
    let mut total = light::Stats::default();
    let mut shade_ms = Vec::new();
    let (mut c_ok, mut c_all) = (0usize, 0usize);
    let mut dev_hist = [0usize; 4];
    #[allow(clippy::type_complexity)]
    let mut worst: Vec<(f32, u64, u32, u32, Vec<f32>, (f32, f32))> = Vec::new();
    let (mut e_flat, mut e_lit, mut e_px) = (0usize, 0usize, 0usize);
    let mut loaded = Vec::new();
    for (n, d) in dirs.iter().enumerate() {
        let f = Frame::load(d).map_err(|e| format!("{}: {e}", d.display()))?;
        let src = f.sources(&texture_rom);
        let mut line = String::new();
        // The pass, timed alone (best of 3: the machine is shared).
        let mut sh = light::shade(&f.stream, &opts);
        for _ in 0..2 {
            let again = light::shade(&f.stream, &opts);
            if again.stats.ms < sh.stats.ms {
                sh = again;
            }
        }
        shade_ms.push(sh.stats.ms);
        let st = sh.stats;
        total.add(&st);
        let info: std::collections::HashMap<u32, light::Info> = sh.info.iter().map(|i| (i.at, *i)).collect();
        let flat_polys = raster::build(&f.stream, &src);
        let lit_polys = raster::build_with(
            &f.stream,
            &src,
            &raster::BuildIn { lit: Some(&sh), skip_checker: opts.skip_checker, ..Default::default() },
        )
        .0;
        // A: crease 0.
        let sh0 = light::shade(&f.stream, &zero);
        let polys0 = raster::build_with(&f.stream, &src, &raster::BuildIn { lit: Some(&sh0), ..Default::default() }).0;
        let (fp, fpl) = placed(flat_polys, &f, &view);
        let (lp, lpl) = placed(lit_polys, &f, &view);
        let (zp, zpl) = placed(polys0, &f, &view);
        let sw_flat = soft(&fp, &src, &view);
        let sw_lit = soft(&lp, &src, &view);
        gpu.set_quality(flat_q);
        let g_flat = gpu.draw(&fp, &fpl, &src, &view);
        gpu.set_quality(lit_q);
        let g_lit = gpu.draw(&lp, &lpl, &src, &view);
        let g_zero = gpu.draw(&zp, &zpl, &src, &view);
        let a_ok = sh0.at.is_empty()
            && format!("{zp:?}") == format!("{fp:?}")
            && soft(&zp, &src, &view) == sw_flat
            && g_zero == g_flat;
        // C: brightness. Per lit polygon, its corners' mean against its
        // own luma (+0.5); the frame's mean colour, 3D layer.
        let lit_list: Vec<&Poly> = lp.iter().filter(|p| !p.lit.is_empty()).collect();
        let mut near = 0;
        for p in &lit_list {
            let c = sh.get(p.at).unwrap();
            let k = if p.points.len() == 3 { 3 } else { 4 };
            let mean = c[..k].iter().map(|v| v[0]).sum::<f32>() / k as f32;
            let dev = mean - (p.luma as f32 + 0.5);
            near += (dev.abs() <= 8.0) as usize;
            let b = match dev.abs() {
                d if d <= 8.0 => 0,
                d if d <= 16.0 => 1,
                d if d <= 32.0 => 2,
                _ => 3,
            };
            dev_hist[b] += 1;
            // The polygon's centre in output pixels (as drawn here).
            let (cx, cy) = p.points.iter().fold((0.0, 0.0), |a, v| (a.0 + v.x, a.1 + v.y));
            let n = p.points.len() as f32;
            worst.push((dev.abs(), f.frame, p.at, p.luma, c[..k].iter().map(|v| v[0]).collect(), (cx / n, cy / n)));
        }
        c_ok += near;
        c_all += lit_list.len();
        let mean = |l: &(Vec<u8>, Vec<bool>)| {
            let mut s = [0f64; 3];
            let n = l.1.iter().filter(|&&c| c).count().max(1) as f64;
            for (i, _) in l.1.iter().enumerate().filter(|(_, c)| **c) {
                for (ch, v) in s.iter_mut().enumerate() {
                    *v += l.0[i * 3 + ch] as f64;
                }
            }
            s.map(|v| v / n)
        };
        let (mf, ml) = (mean(&sw_flat), mean(&sw_lit));
        let dmean = (0..3).map(|c| ml[c] - mf[c]).fold(0.0f64, |a, b| if b.abs() > a.abs() { b } else { a });
        // D: pixels whose visible polygon isn't lit are the flat frame's.
        let vis = raster::visible(&lp, &lpl, &src, &view);
        let unlit: Vec<bool> =
            vis.owner.iter().map(|&o| o == u32::MAX || lp[lpl[o as usize].poly].lit.is_empty()).collect();
        let d_sw = compare(&sw_flat.0, &sw_lit.0, Some(&unlit)).0;
        // The GPU: where it agrees with the software drawing flat (else
        // it may show another polygon there: an edge pixel).
        let agree: Vec<bool> = (0..unlit.len())
            .map(|i| {
                unlit[i] && g_flat.1[i] == sw_flat.1[i] && g_flat.0[i * 3..i * 3 + 3] == sw_flat.0[i * 3..i * 3 + 3]
            })
            .collect();
        let d_gpu = compare(&g_flat.0, &g_lit.0, Some(&agree)).0;
        // The GPU's floor: a pixel where it shows another polygon than the
        // software with the same flat colour (a few a frame).
        let d_ok = d_sw == 0 && d_gpu <= view.width * view.height / 10_000;
        // E: software against GPU, flat and lit.
        let (ef_cov, ef) = sw_gpu(&sw_flat, &g_flat);
        let (el_cov, el) = sw_gpu(&sw_lit, &g_lit);
        let px = view.width * view.height;
        e_flat += ef;
        e_lit += el;
        e_px += px;
        let changed = compare(&sw_flat.0, &sw_lit.0, None).0;
        let pct = |x: usize| 100.0 * x as f64 / px as f64;
        let _ = writeln!(
            line,
            "{:<7} {:>6} {:>6} {:>6} {:>5} {:>6} {:>6} {:>5} {:>7.2}%  {:>3} {:>+9.3} {:>13.2}% {:>9.3} {:>7.3}%/{:>7.3}% {:>7.3}%/{:>7.3}%",
            f.frame,
            st.polys - st.hidden,
            st.lit,
            st.flat,
            st.excluded,
            st.crease_edges,
            st.material_splits,
            if a_ok { "ok" } else { "FAIL" },
            if lit_list.is_empty() { 100.0 } else { 100.0 * near as f64 / lit_list.len() as f64 },
            if d_ok { "ok".to_string() } else { format!("FAIL sw {d_sw} gpu {d_gpu}") },
            dmean,
            pct(changed),
            sh.stats.ms,
            pct(ef),
            pct(ef_cov),
            pct(el),
            pct(el_cov),
        );
        if !a_ok {
            failures.push(format!("A: frame {}: crease 0 isn't the flat frame", f.frame));
        }
        if !d_ok {
            failures
                .push(format!("D: frame {}: {d_sw} software, {d_gpu} GPU pixels of polygons not lit differ", f.frame));
        }
        if dmean.abs() > 1.0 {
            warnings.push(format!("C: frame {}: mean colour moved {dmean:+.3} levels (initial target 1.0)", f.frame));
        }
        if el > ef + px / 1000 + ef / 20 {
            failures.push(format!(
                "E: frame {}: software and GPU differ by >16 on {:.3}% lit, {:.3}% flat",
                f.frame,
                pct(el),
                pct(ef)
            ));
        }
        for pr in probes.iter().filter(|p| p.frame == f.frame) {
            let fl = video::compose(&f.tiles, &f.palette, Some((&sw_flat.0, &sw_flat.1)), &view);
            let li = video::compose(&f.tiles, &f.palette, Some((&sw_lit.0, &sw_lit.1)), &view);
            report_probe(pr, &lp, &lpl, &src, &view, &info, (&fl, &li), &mut line);
        }
        if n % every.max(1) == 0 || !probes.is_empty() {
            let name = |s: &str| out_dir.join(format!("{:06}-{s}.png", f.frame));
            let fl = video::compose(&f.tiles, &f.palette, Some((&sw_flat.0, &sw_flat.1)), &view);
            let li = video::compose(&f.tiles, &f.palette, Some((&sw_lit.0, &sw_lit.1)), &view);
            let save = |s: &str, rgb: &[u8]| {
                let _ = std::fs::write(name(s), png::encode_rgb(view.width, view.height, rgb));
            };
            save("flat", &fl);
            save("vertex", &li);
            save("diff", &diff_picture(&fl, &li));
            save("layers", &layers_picture(&lp, &lpl, &src, &view, &sw_flat.0, &info));
        }
        print!("{line}");
        rep.push_str(&line);
        if bench_frames.is_some_and(|b| loaded.len() < b) && !lp.is_empty() {
            loaded.push(f);
        }
    }
    let mut tail = String::new();
    let _ = writeln!(
        tail,
        "counts        {} polygons drawn or culled: {} lit, {} flat, {} excluded (HUD windows), {} culled back faces, {} degenerate; {} shared edges, {} kept by the crease, {} by the material rule",
        total.polys - total.hidden,
        total.lit,
        total.flat,
        total.excluded,
        total.culled,
        total.degenerate,
        total.edges,
        total.crease_edges,
        total.material_splits
    );
    let slowest = shade_ms.iter().copied().fold(0.0f32, f32::max);
    let _ = writeln!(
        tail,
        "cost          light::shade median {:.3} ms, worst {:.3} ms a frame (best of 3 runs each)",
        median(&mut shade_ms),
        slowest
    );
    let c_share = if c_all == 0 { 100.0 } else { 100.0 * c_ok as f64 / c_all as f64 };
    let _ = writeln!(
        tail,
        "C             {c_share:.2}% of {c_all} lit polygons have corners averaging within 8 of their own luma (initial target 99%); within 16: {}, 32: {}, more: {}",
        dev_hist[1], dev_hist[2], dev_hist[3]
    );
    worst.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (dev, frame, at, luma, corners, (x, y)) in worst.iter().take(8) {
        let _ = writeln!(
            tail,
            "  off by {dev:>5.1}: frame {frame} at {at} luma {luma} corners {:?} centre ({x:.0},{y:.0}) in the view",
            corners.iter().map(|v| v.round()).collect::<Vec<_>>()
        );
    }
    let _ = writeln!(
        tail,
        "E             software against GPU, pixels over 16 levels: flat {:.3}%, vertex {:.3}%",
        100.0 * e_flat as f64 / e_px.max(1) as f64,
        100.0 * e_lit as f64 / e_px.max(1) as f64
    );
    if c_share < 99.0 {
        warnings.push(format!("C: {c_share:.2}% of lit polygons within 8 levels (initial target 99%)"));
    }
    if let Some(n) = bench_frames {
        let frames: Vec<Frame> = loaded.into_iter().take(n).collect();
        bench(&mut gpu, &frames, &texture_rom, &opts, hud_windows, &mut tail);
    }
    for w in &warnings {
        let _ = writeln!(tail, "warn          {w}");
    }
    for f in &failures {
        let _ = writeln!(tail, "FAIL          {f}");
    }
    let _ = writeln!(
        tail,
        "{}  pictures (FRAME-flat, -vertex, -diff, -layers: orange lit, grey flat, blue excluded, dark blue lit but a shadow, red crease, magenta material edges) in {}",
        if failures.is_empty() { "PASS        " } else { "FAILED      " },
        out_dir.display()
    );
    print!("{tail}");
    rep.push_str(&tail);
    let _ = std::fs::write(out_dir.join("report.txt"), rep);
    Ok(failures.is_empty())
}
