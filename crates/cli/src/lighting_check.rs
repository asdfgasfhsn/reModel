//! `remodel lighting-check`: smooth shading (`model2::light`) on captured
//! frames. Each frame is drawn flat and shaded (`--shading vertex`, the
//! default, or `pixel`) on the software and GPU renderers, and the
//! invariants that need no ground truth are checked:
//!
//! - A: at crease 0 nothing is lit, and both renderers draw the flat frame
//!   byte for byte (with the GPU's shading option on);
//! - C: brightness: a lit polygon's corners, and its pixels as drawn,
//!   average close to its own luma, and the frame's mean colour barely
//!   moves;
//! - D: every pixel whose visible polygon isn't lit is the flat frame's,
//!   on both renderers;
//! - E: software and GPU agree shaded about as well as flat;
//! - B (frames captured with a side channel, `light.bin`): it lines up
//!   with the stream on every frame, the flat luma recomputed from it is
//!   the stream's (`light::check_mesh`), and the list scan's block facts
//!   (`scan.bin`) are the native geometrizer's;
//! - F (likewise): the stream-only pass (L0) against the one with the side
//!   channel: hard-edge decisions that agree, pixels that differ;
//! - J: shading changes a pixel's level within its colour ramp, never its
//!   hue (`raster::same_hue`; with finer levels too);
//! - K (`--shading-fine on`): finer levels against the game's levels on
//!   the same corners: how many lit pixels change, the mean colour shift
//!   (a bias), and the steps between neighbouring pixels of untextured lit
//!   polygons (banding: few large steps; finer levels: many small ones);
//! - with `--shading-balance off` (no brightness offset per smooth
//!   surface) the same checks, C report-only (it measures what the
//!   balance corrects: bumpy surfaces come out brighter);
//! - G: the pass is deterministic (run again, the same corners); that a
//!   polygon whose vertex normals are its face's is drawn flat is A here
//!   and unit tests (`light`, `raster`, `gpu`);
//! - H, I: what the pass costs and what it did (lit, flat, excluded,
//!   crease edges, material splits, blocks by level).
//!
//! Pictures per frame: flat and vertex or pixel (software, whole frames),
//! their differences, and the polygons' classes in false colour. `--probe`
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
fn class_colour(p: &Poly, info: Option<&light::Info>) -> [f32; 3] {
    let class = info.map(|i| i.class);
    match class {
        // Lit: L0 (stream only) orange, L1 (block facts) yellow, L2 (the
        // game's normals and materials) green.
        _ if !p.lit.is_empty() => match info.map(|i| i.level) {
            Some(light::Level::L2) => [0.2, 0.85, 0.25],
            Some(light::Level::L1) => [0.95, 0.85, 0.1],
            _ => [1.0, 0.55, 0.1],
        },
        Some(Class::Lit) => [0.2, 0.3, 0.9], // lit but dropped: a shadow or checker
        Some(Class::Excluded) => [0.3, 0.5, 1.0], // excluded (HUD windows): blue
        Some(Class::Direct) => [0.55, 0.35, 0.15], // direct data: brown
        Some(Class::Unlit) => [0.95, 0.95, 0.95], // zero normal (L2): white
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
        let c = class_colour(p, info.get(&p.at));
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
            "  draw {:>5} window {} block {:>4} at {:>7} class {:<10} {:?} luma {:>3} colour {:?} textured {} corners {:?} hard edges crease {:04b} material {:04b}",
            hit.index,
            p.window,
            p.block,
            p.at,
            inf.map_or("-".to_string(), |i| format!("{:?}", i.class)),
            inf.map_or(light::Level::L0, |i| i.level),
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
    let _ = writeln!(
        out,
        "bench         GPU whole frame incl. compose and readback, mean ms over {} frames (with the side channel where captured)",
        frames.len()
    );
    let levels = [None, Some(light::Shading::Vertex), Some(light::Shading::Pixel)];
    // CPU: `light::shade` and building the polygons, without and with the
    // lamps material maps need (`Options::lamps`), best of 3 per frame.
    let best = |f: &mut dyn FnMut()| {
        (0..3)
            .map(|_| {
                let s = Instant::now();
                f();
                s.elapsed().as_secs_f64() * 1e3
            })
            .fold(f64::MAX, f64::min)
    };
    let mut cpu = [[0.0f64; 5]; 3];
    for f in frames {
        let src = f.sources(texture_rom);
        let side = f.light_frame();
        for (k, s) in [light::Shading::Flat, light::Shading::Vertex, light::Shading::Pixel].into_iter().enumerate() {
            for (j, lamps) in [false, true].into_iter().enumerate() {
                let o = Options { shading: s, ..*opts }.with_lamps(lamps);
                cpu[k][j] += best(&mut || {
                    std::hint::black_box(light::shade_with(&f.stream, side.as_ref(), &o));
                });
                cpu[k][2 + j] += best(&mut || {
                    std::hint::black_box(raster::build_shaded_with(&f.stream, &src, side.as_ref(), Some(&o)));
                });
            }
            // With lamps, the textures' texel scan kept from the frame
            // before (`raster::TexelCache`, render RAM unchanged: as the
            // renderer's `present_bus*` keep it).
            let o = Options { shading: s, ..*opts }.with_lamps(true);
            let cache = std::cell::RefCell::new(raster::TexelCache::default());
            cache.borrow_mut().begin(Some(1));
            let mut build = || {
                let sh = light::shade_with(&f.stream, side.as_ref(), &o);
                let b = raster::BuildIn {
                    lit: Some(&sh),
                    skip_checker: o.skip_checker,
                    fine: o.fine,
                    relief: o.relief,
                    texels: Some(&cache),
                    ..Default::default()
                };
                std::hint::black_box(raster::build_with(&f.stream, &src, &b));
            };
            build();
            cpu[k][4] += best(&mut build);
        }
    }
    let n = frames.len().max(1) as f64;
    for (k, s) in ["flat", "vertex", "pixel"].iter().enumerate() {
        let c = cpu[k].map(|v| v / n);
        let _ = writeln!(
            out,
            "  CPU {s:<6} shade {:.2} ms, with lamps {:.2} ms (+{:.2}); build {:.2} ms, with lamps {:.2} ms (+{:.2}), \
             and the texel scan kept across frames {:.2} ms (+{:.2})",
            c[0],
            c[1],
            c[1] - c[0],
            c[2],
            c[3],
            c[3] - c[2],
            c[4],
            c[4] - c[2]
        );
    }
    for (name, height) in [("1080p", 1080), ("4K", 2160)] {
        let view = View::new(height, 16.0 / 9.0, video::Hud::Edges).with_hud_windows(hud.0, hud.1);
        let mut t = [0.0f64; 3];
        for f in frames {
            let src = f.sources(texture_rom);
            let layers = video::layers_2d(&f.tiles, &f.palette, &view, true);
            let side = f.light_frame();
            for (k, level) in levels.into_iter().enumerate() {
                let o = level.map(|s| Options { shading: s, ..*opts });
                let built = raster::build_shaded_with(&f.stream, &src, side.as_ref(), o.as_ref());
                let (polys, placed) = placed(built, f, &view);
                gpu.set_quality(super::gpu_light(o.as_ref()));
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
            "  {name:<6} flat {:>7.2} ms   vertex {:>7.2} ms (+{:.2})   pixel {:>7.2} ms (+{:.2})",
            t[0] / n,
            t[1] / n,
            (t[1] - t[0]) / n,
            t[2] / n,
            (t[2] - t[0]) / n
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
    use_side: bool,
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
    // Pictures and the report by the level: vertex keeps the plain names;
    // finer levels add `-fine`, the balance off `-nobal`.
    let nobal = if opts.balance { "" } else { "-nobal" };
    let fine = if opts.fine { "-fine" } else { "" };
    let level = format!("{}{fine}{nobal}", opts.shading.name());
    let level = level.as_str();
    let prefix = match (opts.shading == light::Shading::Pixel, opts.fine) {
        (true, true) => "pixel-fine-",
        (true, false) => "pixel-",
        (false, true) => "fine-",
        (false, false) => "",
    };
    let prefix = if opts.balance { prefix.to_string() } else { format!("{prefix}nobal-") };
    let prefix = prefix.as_str();
    let tag = |s: &str| format!("{prefix}{s}");
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let mut rep = String::new();
    let _ = writeln!(rep, "gpu           {}", gpu.adapter());
    let _ = writeln!(rep, "view          {}x{}", view.width, view.height);
    let _ = writeln!(
        rep,
        "shading       {level}, crease {} degrees, brightness balance {}, focal guess {} x window width (L0); side channel {}",
        opts.crease,
        if opts.balance { "on" } else { "off" },
        light::FOCAL_GUESS,
        if use_side { "used where captured (light.bin)" } else { "not used (--no-side)" }
    );
    // A `--capture` (a directory a frame) or a `--capture-seq` sequence
    // (consecutive frames: shimmer in motion), in order.
    let frames = capture::frames(root).map_err(|e| format!("{}: {e}", root.display()))?;
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
    let mut build_ms: Vec<f32> = Vec::new();
    let (mut c_ok, mut c_all) = (0usize, 0usize);
    // C over the pixels as drawn: per lit polygon the mean of its pixels'
    // lumas against its own; and over all lit pixels the mean difference
    // (a bias: smoothing must not brighten or darken surfaces overall).
    let (mut cp_ok, mut cp_all, mut cp_hist) = (0usize, 0usize, [0usize; 4]);
    let (mut bias_sum, mut bias_px) = (0f64, 0usize);
    // The polygons moving the most light: (sum of luma moved, frame, at,
    // own luma, pixels, block).
    let mut heavy: Vec<(f64, u64, u32, u32, usize, u32)> = Vec::new();
    let (mut g_ok, mut g_all) = (0usize, 0usize);
    let mut dev_hist = [0usize; 4];
    #[allow(clippy::type_complexity)]
    let mut worst: Vec<(f32, u64, u32, u32, Vec<f32>, (f32, f32))> = Vec::new();
    let (mut e_flat, mut e_lit, mut e_px) = (0usize, 0usize, 0usize);
    let mut loaded = Vec::new();
    // B and F, over the frames with a side channel.
    let mut side_frames = 0usize;
    let mut scan = model2::geo::LightCheck::default();
    let (mut f_agree, mut f_edges, mut f_px, mut f_all) = (0usize, 0usize, 0usize, 0usize);
    let (mut f_lit0, mut f_lit) = (0usize, 0usize);
    let mut hue_px = 0usize;
    let mut hue_flat = 0usize;
    let mut hue_clamped = 0usize;
    // K (finer levels): against the same frame at the game's levels.
    let mut k = FineCheck::default();
    let wanted = |f: &Frame| probes.is_empty() || probes.iter().any(|p| p.frame == f.frame);
    for (n, item) in frames.filter(|r| r.as_ref().map_or(true, |(_, f)| wanted(f))).enumerate() {
        let (_, f) = item.map_err(|e| format!("{}: {e}", root.display()))?;
        let src = f.sources(&texture_rom);
        let mut line = String::new();
        let side = if use_side { f.light_frame() } else { None };
        // The pass, timed alone (best of 3: the machine is shared).
        let mut sh = light::shade_with(&f.stream, side.as_ref(), &opts);
        for _ in 0..2 {
            let again = light::shade_with(&f.stream, side.as_ref(), &opts);
            if again.stats.ms < sh.stats.ms {
                sh = again;
            }
        }
        shade_ms.push(sh.stats.ms);
        // G: the same again.
        let again = light::shade_with(&f.stream, side.as_ref(), &opts);
        let bits = |s: &light::Shaded| s.corners.iter().flatten().flatten().map(|v| v.to_bits()).collect::<Vec<_>>();
        g_all += 1;
        g_ok += (again.at == sh.at && bits(&again) == bits(&sh) && again.pixel == sh.pixel) as usize;
        let st = sh.stats;
        total.add(&st);
        let info: std::collections::HashMap<u32, light::Info> = sh.info.iter().map(|i| (i.at, *i)).collect();
        let t = Instant::now();
        let flat_polys = raster::build(&f.stream, &src);
        let t_flat = t.elapsed().as_secs_f32() * 1e3;
        let t = Instant::now();
        let lit_polys = raster::build_with(
            &f.stream,
            &src,
            &raster::BuildIn { lit: Some(&sh), skip_checker: opts.skip_checker, fine: opts.fine, ..Default::default() },
        )
        .0;
        // What shading adds to building the polygons (the hue ranges).
        build_ms.push(t.elapsed().as_secs_f32() * 1e3 - t_flat);
        // B: the side channel lines up and agrees with the stream (counted
        // in `sh.stats.check`); the list scan's facts are the hooks'.
        if let Some(sd) = &side {
            side_frames += 1;
            let c = sh.stats.check;
            if c.misaligned_frames > 0 {
                failures.push(format!("B: frame {}: the side channel's blocks don't line up with the stream", f.frame));
            }
            if let Some(sc) = f.scan_frame() {
                compare_scan(&mut scan, f.frame, sd, &sc);
            }
        }
        // F: the stream alone (L0) against the side channel.
        if side.is_some() {
            let l0 = light::shade(&f.stream, &opts);
            let hard = |s: &light::Shaded| -> std::collections::HashMap<u32, (u8, bool)> {
                s.info.iter().map(|i| (i.at, (i.crease | i.material, i.class == Class::Lit))).collect()
            };
            let (h0, h1) = (hard(&l0), hard(&sh));
            for (at, (e1, _)) in &h1 {
                if let Some((e0, _)) = h0.get(at) {
                    f_edges += 4;
                    f_agree += 4 - (e0 ^ e1).count_ones() as usize;
                }
            }
            f_lit0 += l0.at.len();
            f_lit += sh.at.len();
            let polys0 = raster::build_with(
                &f.stream,
                &src,
                &raster::BuildIn {
                    lit: Some(&l0),
                    skip_checker: opts.skip_checker,
                    fine: opts.fine,
                    ..Default::default()
                },
            )
            .0;
            let (p0, _) = placed(polys0, &f, &view);
            let (pl, _) = placed(
                raster::build_with(
                    &f.stream,
                    &src,
                    &raster::BuildIn {
                        lit: Some(&sh),
                        skip_checker: opts.skip_checker,
                        fine: opts.fine,
                        ..Default::default()
                    },
                )
                .0,
                &f,
                &view,
            );
            f_px += compare(&soft(&p0, &src, &view).0, &soft(&pl, &src, &view).0, None).0;
            f_all += view.width * view.height;
            if n % every.max(1) == 0 && probes.is_empty() {
                // The L0 picture, to compare with -vertex.
                let sw0 = soft(&p0, &src, &view);
                let rgb = video::compose(&f.tiles, &f.palette, Some((&sw0.0, &sw0.1)), &view);
                let _ = std::fs::write(
                    out_dir.join(format!("{:06}-vertex-l0.png", f.frame)),
                    png::encode_rgb(view.width, view.height, &rgb),
                );
            }
        }
        // A: crease 0.
        let sh0 = light::shade_with(&f.stream, side.as_ref(), &zero);
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
        // Lit by the pass but drawn flat because their colour would change
        // hue (`raster::build_with`; shadows and skipped checkerboards
        // aside).
        hue_flat += lp
            .iter()
            .filter(|p| p.lit.is_empty() && !p.shadow && !(p.checker && opts.skip_checker) && sh.get(p.at).is_some())
            .count();
        hue_clamped += lp.iter().filter(|p| !p.lit.is_empty() && p.lit_range != [0, 255]).count();
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
        for pl in &lpl {
            let p = &lp[pl.poly];
            let px = raster::lit_pixel_lumas(p, pl, &view);
            if px.is_empty() {
                continue;
            }
            let d: f64 = px.iter().map(|&l| l as f64 - p.luma as f64).sum();
            bias_sum += d;
            heavy.push((d, f.frame, p.at, p.luma, px.len(), p.block));
            bias_px += px.len();
            let dev = (d / px.len() as f64).abs();
            cp_all += 1;
            cp_ok += (dev <= 8.0) as usize;
            cp_hist[match dev {
                d if d <= 8.0 => 0,
                d if d <= 16.0 => 1,
                d if d <= 32.0 => 2,
                _ => 3,
            }] += 1;
        }
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
        // J: shading changes a pixel's level, never its hue (the colour
        // tables may map levels to other hues: such polygons stay flat).
        let hue = (0..px)
            .filter(|&i| {
                let (a, b) = (&sw_flat.0[i * 3..i * 3 + 3], &sw_lit.0[i * 3..i * 3 + 3]);
                a != b && !raster::same_hue([a[0], a[1], a[2]], [b[0], b[1], b[2]])
            })
            .count();
        hue_px += hue;
        if hue > px / 5000 {
            failures.push(format!("J: frame {}: {hue} pixels change hue, not only level", f.frame));
        }
        // K: finer levels against the game's levels, the same corners.
        if opts.fine {
            let levels = raster::build_with(
                &f.stream,
                &src,
                &raster::BuildIn { lit: Some(&sh), skip_checker: opts.skip_checker, ..Default::default() },
            )
            .0;
            let (kp, _) = placed(levels, &f, &view);
            let sw_levels = soft(&kp, &src, &view);
            let lit_px: Vec<bool> =
                vis.owner.iter().map(|&o| o != u32::MAX && !lp[lpl[o as usize].poly].lit.is_empty()).collect();
            // Steps only over untextured polygons: where banding shows (a
            // texture's own detail would swamp it).
            let plain: Vec<bool> =
                (0..px).map(|i| lit_px[i] && lp[lpl[vis.owner[i] as usize].poly].texture.is_none()).collect();
            // Reported, not failed: a pixel is at most a column's step from
            // the game's level, and some tables step far (fvipers' skin
            // from black straight to (70, 65, 60)); that each lies between
            // two adjacent columns of its hue is `raster`'s unit tests, the
            // hue J above.
            k.add(&sw_levels.0, &sw_lit.0, (&lit_px, &plain), &vis.owner, view.width);
            if n % every.max(1) == 0 && probes.is_empty() {
                let rgb = video::compose(&f.tiles, &f.palette, Some((&sw_levels.0, &sw_levels.1)), &view);
                let _ = std::fs::write(
                    out_dir.join(format!("{:06}-{}{nobal}.png", f.frame, opts.shading.name())),
                    png::encode_rgb(view.width, view.height, &rgb),
                );
            }
        }
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
        if dmean.abs() > 1.0 && opts.balance {
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
            save(level, &li);
            save(&tag("diff"), &diff_picture(&fl, &li));
            save(&tag("layers"), &layers_picture(&lp, &lpl, &src, &view, &sw_flat.0, &info));
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
    if side_frames > 0 {
        let c = &total.check;
        let n = c.compared().max(1) as f64;
        let blocks = (c.l0 + c.l1 + c.l2).max(1) as f64;
        let _ = writeln!(
            tail,
            "B             {side_frames} frames with a side channel: {} misaligned; blocks L2 {:.2}%, L1 {:.2}%, L0 {:.2}%, {} demoted; polygons {}: exact {:.3}%, within 1 {:.3}%, mismatching {:.3}%, {} without a normal",
            c.misaligned_frames,
            100.0 * c.l2 as f64 / blocks,
            100.0 * c.l1 as f64 / blocks,
            100.0 * c.l0 as f64 / blocks,
            c.demoted,
            c.compared(),
            100.0 * c.exact as f64 / n,
            100.0 * c.near as f64 / n,
            100.0 * c.mismatch as f64 / n,
            c.skipped
        );
        let causes = |v: &[u32; 6]| {
            light::Cause::ALL.iter().map(|k| format!("{} {}", k.name(), v[*k as usize])).collect::<Vec<_>>().join(", ")
        };
        let _ =
            writeln!(tail, "B causes      mismatching: {}; within 1: {}", causes(&c.causes), causes(&c.near_causes));
        let _ = writeln!(
            tail,
            "B scan        {} frames: blocks equal {}, equal where known {}, focal not known {}, differ {}; {} the scan gave up on, {} with another block count",
            scan.frames, scan.equal, scan.partial, scan.unknown, scan.differing, scan.scan_failed, scan.count_differing
        );
        if c.mismatch as f64 > 0.001 * n {
            failures.push(format!(
                "B: {} of {} polygons' flat lumas mismatch the side channel (over 0.1%)",
                c.mismatch,
                c.compared()
            ));
        }
        if scan.differing > 0 || scan.count_differing > 0 {
            failures.push(format!(
                "B: the list scan's block facts differ from the native geometrizer's ({} blocks, {} frames' counts; first {:?})",
                scan.differing, scan.count_differing, scan.first
            ));
        }
        let _ = writeln!(
            tail,
            "F             L0 (stream only) against the side channel: {:.2}% of shared ring edges decided alike (initial target 95%); lit polygons {} -> {}; {:.3}% of pixels differ",
            100.0 * f_agree as f64 / f_edges.max(1) as f64,
            f_lit0,
            f_lit,
            100.0 * f_px as f64 / f_all.max(1) as f64
        );
    }
    let slowest = shade_ms.iter().copied().fold(0.0f32, f32::max);
    let _ = writeln!(
        tail,
        "cost          light::shade median {:.3} ms, worst {:.3} ms a frame (best of 3 runs each); building the polygons lit costs {:.3} ms more (median)",
        median(&mut shade_ms),
        slowest,
        median(&mut build_ms)
    );
    let c_share = if c_all == 0 { 100.0 } else { 100.0 * c_ok as f64 / c_all as f64 };
    let _ = writeln!(
        tail,
        "C             {c_share:.2}% of {c_all} lit polygons have corners averaging within 8 of their own luma (initial target 99%); within 16: {}, 32: {}, more: {}",
        dev_hist[1], dev_hist[2], dev_hist[3]
    );
    let _ = writeln!(
        tail,
        "C pixels      {:.2}% of {cp_all} lit polygons drawn average within 8 of their own luma; within 16: {}, 32: {}, more: {}; over all {bias_px} lit pixels the luma moved {:+.3} levels on average",
        if cp_all == 0 { 100.0 } else { 100.0 * cp_ok as f64 / cp_all as f64 },
        cp_hist[1],
        cp_hist[2],
        cp_hist[3],
        bias_sum / bias_px.max(1) as f64
    );
    heavy.sort_by(|a, b| b.0.abs().total_cmp(&a.0.abs()));
    for (d, frame, at, luma, n, block) in heavy.iter().take(6) {
        let _ = writeln!(
            tail,
            "  moved {d:>+10.0}: frame {frame} at {at} (block {block}) luma {luma}, {n} pixels averaging {:+.1}",
            d / *n as f64
        );
    }
    let _ = writeln!(
        tail,
        "I level       {} lit polygons: {} with the game's formula per corner (L2), {} per pixel; drawn as {}",
        total.lit,
        total.exact,
        total.pixel,
        total.effective(opts.shading).name()
    );
    let _ = writeln!(tail, "G             {g_ok} of {g_all} frames shaded the same twice");
    if g_ok != g_all {
        failures.push(format!("G: {} frames shaded differently when run again", g_all - g_ok));
    }
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
        "E             software against GPU, pixels over 16 levels: flat {:.3}%, {level} {:.3}%",
        100.0 * e_flat as f64 / e_px.max(1) as f64,
        100.0 * e_lit as f64 / e_px.max(1) as f64
    );
    let _ = writeln!(
        tail,
        "J             pixels whose hue (not only level) shading changed: {} = {:.4}% (fails over 0.02% in a frame); {} polygons kept flat because their colour tables keep their hue at no other luma; {} lit polygons clamped to the lumas of their hue",
        hue_px,
        100.0 * hue_px as f64 / e_px.max(1) as f64,
        hue_flat,
        hue_clamped
    );
    if !opts.balance {
        let _ = writeln!(tail, "C             report-only: the brightness balance is off");
    } else if c_share < 99.0 {
        warnings.push(format!("C: {c_share:.2}% of lit polygons within 8 levels (initial target 99%)"));
    }
    if opts.fine {
        let lit = k.lit.max(1) as f64;
        let mean = k.shift.map(|s| s / lit);
        let _ = writeln!(
            tail,
            "K             finer levels against the game's levels over {} lit pixels: {:.2}% differ, {} by more than 16; mean RGB shift {:+.3} {:+.3} {:+.3}; steps between neighbouring pixels of one lit untextured polygon: levels {} averaging {:.2}, fine {} averaging {:.2} (largest channel)",
            k.lit,
            100.0 * k.differ as f64 / lit,
            k.far,
            mean[0],
            mean[1],
            mean[2],
            k.steps[0],
            k.step_sum[0] / k.steps[0].max(1) as f64,
            k.steps[1],
            k.step_sum[1] / k.steps[1].max(1) as f64,
        );
        if mean.iter().any(|m| m.abs() > 1.0) {
            warnings.push(format!("K: finer levels moved the mean colour of lit pixels by {mean:+.3?}"));
        }
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
        "{}  pictures (FRAME-flat, -{level}, -vertex-l0 (stream only, with a side channel), -{}, -{}: lit L0 orange, L1 yellow, L2 green, grey flat, blue excluded, brown direct data, white zero normal, dark blue lit but a shadow, red crease, magenta material edges) in {}",
        if failures.is_empty() { "PASS        " } else { "FAILED      " },
        tag("diff"),
        tag("layers"),
        out_dir.display()
    );
    print!("{tail}");
    rep.push_str(&tail);
    let report = if prefix.is_empty() {
        "report.txt".to_string()
    } else {
        format!("report-{}.txt", &prefix[..prefix.len() - 1])
    };
    let _ = std::fs::write(out_dir.join(report), rep);
    Ok(failures.is_empty())
}

/// K: finer levels against the game's levels on the same lit pixels.
#[derive(Default)]
struct FineCheck {
    lit: usize,
    differ: usize,
    far: usize,
    /// Per channel, the sum of (fine - levels).
    shift: [f64; 3],
    /// Steps between horizontally neighbouring pixels of one lit untextured
    /// polygon (levels, fine): how many, and the sum of their sizes (the largest
    /// channel difference). Banding is few large steps; finer levels make
    /// them many and small.
    steps: [usize; 2],
    step_sum: [f64; 2],
}

impl FineCheck {
    /// Adds a frame: `levels` and `fine` 3D layers, `lit` the pixels whose
    /// visible polygon is lit (`plain`: and untextured), `owner` that
    /// polygon.
    fn add(&mut self, levels: &[u8], fine: &[u8], (lit, plain): (&[bool], &[bool]), owner: &[u32], width: usize) {
        for i in 0..lit.len() {
            if !lit[i] {
                continue;
            }
            self.lit += 1;
            let m = diff_at(levels, fine, i);
            self.differ += (m > 0) as usize;
            self.far += (m > 16) as usize;
            for c in 0..3 {
                self.shift[c] += fine[i * 3 + c] as f64 - levels[i * 3 + c] as f64;
            }
            if plain[i] && i % width + 1 < width && plain[i + 1] && owner[i + 1] == owner[i] {
                for (k, img) in [levels, fine].into_iter().enumerate() {
                    let d = diff_at(img, &img[3..], i);
                    if d > 0 {
                        self.steps[k] += 1;
                        self.step_sum[k] += d as f64;
                    }
                }
            }
        }
    }
}

/// The largest channel difference of pixel `i` between two RGB images.
fn diff_at(a: &[u8], b: &[u8], i: usize) -> u8 {
    (0..3).map(|c| a[i * 3 + c].abs_diff(b[i * 3 + c])).max().unwrap_or(0)
}

/// B: one frame's list scan (`scan.bin`) against the native geometrizer's
/// block facts (`light.bin`), as the engine counts them.
fn compare_scan(c: &mut model2::geo::LightCheck, frame: u64, native: &light::Frame, scan: &light::Frame) {
    use light::Block;
    c.frames += 1;
    if scan.blocks.len() != native.blocks.len() {
        c.count_differing += 1;
    }
    for (k, (n, s)) in native.blocks.iter().zip(&scan.blocks).enumerate() {
        let (nf, sf) = (n.facts(), s.facts());
        if s.flags & Block::UNKNOWN != 0 {
            c.unknown += 1;
        } else if nf == sf {
            c.equal += 1;
        } else if s.flags & Block::PARTIAL != 0 && (nf.0, nf.2) == (sf.0, sf.2) {
            c.partial += 1;
        } else {
            c.differing += 1;
            c.first.get_or_insert((frame, k, *n, *s));
        }
    }
}
