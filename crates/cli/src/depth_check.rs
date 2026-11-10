//! `remodel depth-check`: draw captured frames on the GPU with the depth
//! buffer off and on, report where they differ and write the differences
//! as pictures to look at (docs/enhancements.md: every difference should be
//! a fix, such as intersecting polygons or a mis-sorted one, and not a new
//! glitch). `--probe X,Y@FRAME` follows one pixel through the depth
//! buffer (`raster::probe_depth`); `--against DIR` compares the depth-on
//! frames with an earlier run's, to see what a change of the rules did.

use model2::capture::{self, Frame};
use model2::raster;
use model2::video::{self, View};
use std::path::Path;

/// A pixel to follow through the depth buffer: `--probe X,Y@FRAME`.
#[derive(Clone, Copy, Debug)]
pub struct Probe {
    pub x: usize,
    pub y: usize,
    pub frame: u64,
}

impl Probe {
    /// `X,Y@FRAME`, X and Y in output pixels (as the -on pictures).
    pub fn parse(s: &str) -> Option<Probe> {
        let (xy, frame) = s.split_once('@')?;
        let (x, y) = xy.split_once(',')?;
        Some(Probe { x: x.trim().parse().ok()?, y: y.trim().parse().ok()?, frame: frame.trim().parse().ok()? })
    }
}

/// The frame's polygons (smooth-shaded with `light`), sorted and placed
/// in `view`.
fn prepare(
    f: &Frame,
    src: &raster::Sources,
    view: &View,
    light: Option<&model2::light::Options>,
) -> (Vec<raster::Poly>, Vec<raster::Placed>) {
    let mut polys = raster::build_shaded(&f.stream, src, light);
    video::HudLayout::new(&f.tiles, &f.palette, view).anchor(&mut polys, view);
    let placed = raster::place(&mut polys, view);
    (polys, placed)
}

/// The frame as the GPU draws it with `depth` on or off.
#[allow(clippy::too_many_arguments)]
fn draw(
    gpu: &mut gpu::Renderer,
    f: &Frame,
    src: &raster::Sources,
    polys: &[raster::Poly],
    placed: &[raster::Placed],
    view: &View,
    depth: bool,
    msaa: u32,
    light: Option<&model2::light::Options>,
) -> Vec<u8> {
    gpu.set_quality(gpu::Quality { msaa, depth, ..super::gpu_light(light) });
    let layers = video::layers_2d(&f.tiles, &f.palette, view, !placed.is_empty());
    gpu.frame(polys, placed, src, view, &layers)
}

/// Print what `raster::probe_depth` finds at a probed pixel, with the GPU's
/// colours there.
fn report_probe(
    pr: &Probe,
    polys: &[raster::Poly],
    placed: &[raster::Placed],
    src: &raster::Sources,
    view: &View,
    (off, on): (&[u8], &[u8]),
) {
    if pr.x >= view.width || pr.y >= view.height {
        println!("probe {},{}@{}: outside the {}x{} view", pr.x, pr.y, pr.frame, view.width, view.height);
        return;
    }
    let r = raster::probe_depth(polys, placed, src, view, pr.x, pr.y);
    let i = (pr.y * view.width + pr.x) * 3;
    // The software rasteriser's 3D layer there, depth off and on.
    let soft = |depth| {
        let (rgb, covered) = raster::draw_view_depth(&mut polys.to_vec(), src, view, depth);
        covered[i / 3].then(|| [rgb[i], rgb[i + 1], rgb[i + 2]])
    };
    println!(
        "probe {},{}@{}: GPU off {:?}, on {:?}; software off {:?}, on {:?}; {} polygons cover it, in draw order",
        pr.x,
        pr.y,
        pr.frame,
        &off[i..i + 3],
        &on[i..i + 3],
        soft(false),
        soft(true),
        r.hits.len()
    );
    println!(
        "  {:>3} {:>5} {:>3} {:>5} {:>6} {:>5} {:>11} {:<8} {:>10} {:>10} {:>10} {:>4} result",
        "hit", "draw", "win", "rank", "group", "block", "key", "kept", "1/z", "depth", "buffer", "from"
    );
    for (k, hit) in r.hits.iter().enumerate() {
        let (pl, p) = (&placed[hit.index], &polys[placed[hit.index].poly]);
        let result = match (hit.background, hit.passed, hit.colour) {
            (_, _, None) => "draws nothing (checker or cut-out)".to_string(),
            (Some(_), _, Some(c)) => format!("drawn untested {c:?}"),
            (None, true, Some(c)) => format!("pass {c:?}"),
            (None, false, Some(c)) => format!("FAIL (would be {c:?})"),
        };
        let lumas = if p.lit.is_empty() {
            String::new()
        } else {
            format!(" lumas {:?}", p.lit.iter().map(|l| (l[0] * 10.0).round() / 10.0).collect::<Vec<_>>())
        };
        println!(
            "  {k:>3} {:>5} {:>3} {:>5} {:>6} {:>5} {:>11.4} {:<8} {:>10.6} {:>10.8} {:>10.8} {:>4} {result}{lumas}",
            hit.index,
            p.window,
            pl.rank,
            pl.group,
            p.block,
            p.key,
            hit.background.unwrap_or("-"),
            hit.iz,
            hit.depth,
            hit.buffer,
            hit.buffer_from.map_or("-".to_string(), |b| b.to_string()),
        );
    }
    let name = |w: Option<usize>| {
        w.map_or("none".to_string(), |k| format!("hit {k} (block {})", polys[placed[r.hits[k].index].poly].block))
    };
    println!("  shows: depth off {}, depth on {}", name(r.winner_off), name(r.winner_on));
    // Where the painter's polygon lost: by how much nearer the polygon that
    // beat it was, against the rank tolerance.
    for (k, hit) in r.hits.iter().enumerate() {
        if let (false, Some(_), Some(e)) = (hit.passed, hit.colour, hit.buffer_from) {
            let he = &r.hits[e];
            let (rl, re) = (placed[hit.index].rank, placed[he.index].rank);
            println!(
                "  hit {k} lost to hit {e}: 1/z {:.3e} nearer (iz_E/iz_L - 1), rank tolerance {:.3e} ({} polygons between)",
                he.iz / hit.iz - 1.0,
                (rl as f32 - re as f32) * raster::DEPTH_BIAS,
                rl as i64 - re as i64
            );
        }
    }
}

/// Off | on | differences (the on picture dimmed, differing pixels bright
/// red where one channel is off by more than 16, orange otherwise),
/// cropped to the differences' bounding box plus a margin.
fn picture(view: &View, off: &[u8], on: &[u8], bbox: (usize, usize, usize, usize)) -> (usize, usize, Vec<u8>) {
    let (x0, y0, x1, y1) = bbox;
    let (w, h) = (x1 - x0, y1 - y0);
    let mut out = vec![0u8; w * 3 * h * 3];
    for y in 0..h {
        for x in 0..w {
            let i = ((y0 + y) * view.width + x0 + x) * 3;
            let m = (0..3).map(|c| off[i + c].abs_diff(on[i + c])).max().unwrap_or(0);
            let d = match m {
                0 => [on[i] / 3, on[i + 1] / 3, on[i + 2] / 3],
                1..=16 => [255, 160, 0],
                _ => [255, 0, 0],
            };
            let row = y * w * 3;
            out[(row + x) * 3..(row + x) * 3 + 3].copy_from_slice(&off[i..i + 3]);
            out[(row + w + x) * 3..(row + w + x) * 3 + 3].copy_from_slice(&on[i..i + 3]);
            out[(row + 2 * w + x) * 3..(row + 2 * w + x) * 3 + 3].copy_from_slice(&d);
        }
    }
    (w * 3, h, out)
}

/// Pixels where `a` and `b` differ, where by more than 16 in a channel,
/// and their bounding box (x0, y0, x1, y1).
fn compare(view: &View, a: &[u8], b: &[u8]) -> (usize, usize, (usize, usize, usize, usize)) {
    let (mut differ, mut large) = (0, 0);
    let (mut x0, mut y0, mut x1, mut y1) = (view.width, view.height, 0, 0);
    for i in 0..view.width * view.height {
        let m = (0..3).map(|c| a[i * 3 + c].abs_diff(b[i * 3 + c])).max().unwrap_or(0);
        if m > 0 {
            differ += 1;
            large += (m > 16) as usize;
            let (x, y) = (i % view.width, i / view.width);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
        }
    }
    (differ, large, (x0, y0, x1, y1))
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    root: &Path,
    hud_windows: (&[usize], &[usize]),
    view: Option<View>,
    msaa: u32,
    out_dir: &Path,
    probes: &[Probe],
    against: Option<&Path>,
    light: Option<&model2::light::Options>,
) -> Result<(), String> {
    let texture_rom = capture::load_texture_rom(root)
        .map_err(|e| format!("{}: {e} (capture with `make capture`)", root.display()))?;
    let mut gpu = gpu::Renderer::new()?;
    let mut view = view.unwrap_or_else(|| View::new(1080, 4.0 / 3.0, video::Hud::Edges));
    view = view.with_hud_windows(hud_windows.0, hud_windows.1);
    println!("gpu           {}", gpu.adapter());
    println!("view          {}x{}, {} samples a pixel", view.width, view.height, msaa);
    let mut dirs: Vec<_> = std::fs::read_dir(root)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("meta.txt").exists())
        .collect();
    dirs.sort();
    // With probes, only the probed frames.
    if !probes.is_empty() {
        let frame = |d: &std::path::PathBuf| d.file_name()?.to_str()?.parse::<u64>().ok();
        dirs.retain(|d| frame(d).is_none_or(|n| probes.iter().any(|p| p.frame == n)));
    }
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    println!("{:<8} {:>6} {:>9} {:>9}  bounding box", "frame", "polys", "differ", ">16");
    let (mut changed, mut moved_total) = (0, 0);
    for d in &dirs {
        let f = Frame::load(d).map_err(|e| format!("{}: {e}", d.display()))?;
        let src = f.sources(&texture_rom);
        let (polys, placed) = prepare(&f, &src, &view, light);
        let n = polys.len();
        let off = draw(&mut gpu, &f, &src, &polys, &placed, &view, false, msaa, light);
        let on = draw(&mut gpu, &f, &src, &polys, &placed, &view, true, msaa, light);
        let (differ, large, bbox) = compare(&view, &off, &on);
        let (x0, y0, x1, y1) = bbox;
        let pct = |n: usize| 100.0 * n as f64 / (view.width * view.height) as f64;
        if differ == 0 {
            println!("{:<8} {n:>6} {:>8.3}% {:>8.3}%", f.frame, 0.0, 0.0);
        } else {
            println!("{:<8} {n:>6} {:>8.3}% {:>8.3}%  ({x0},{y0})-({x1},{y1})", f.frame, pct(differ), pct(large));
        }
        for pr in probes.iter().filter(|p| p.frame == f.frame) {
            report_probe(pr, &polys, &placed, &src, &view, (&off, &on));
        }
        if let Some(dir) = against {
            // The earlier run's depth-on frame; where it has none, its
            // depth-on frame was its depth-off one (unchanged here).
            let before = match std::fs::read(dir.join(format!("depth-{:05}-on.png", f.frame))) {
                Ok(file) => {
                    let img = png::decode(&file)?;
                    if (img.width, img.height) != (view.width, view.height) {
                        return Err(format!(
                            "{}: frame {} is not {}x{}",
                            dir.display(),
                            f.frame,
                            view.width,
                            view.height
                        ));
                    }
                    img.rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect()
                }
                Err(_) => off.clone(),
            };
            let (moved, moved_large, b) = compare(&view, &before, &on);
            let _ = std::fs::remove_file(out_dir.join(format!("against-{:05}.png", f.frame)));
            if moved > 0 {
                println!(
                    "  against  {:>8} pixels changed, {:>8} by >16  ({},{})-({},{})",
                    moved, moved_large, b.0, b.1, b.2, b.3
                );
                let crop = (
                    b.0.saturating_sub(48),
                    b.1.saturating_sub(48),
                    (b.2 + 48).min(view.width),
                    (b.3 + 48).min(view.height),
                );
                let (w, h, rgb) = picture(&view, &before, &on, crop);
                let _ =
                    std::fs::write(out_dir.join(format!("against-{:05}.png", f.frame)), png::encode_rgb(w, h, &rgb));
            }
            moved_total += moved;
        }
        let name = |s: &str| out_dir.join(format!("depth-{:05}{s}.png", f.frame));
        if differ == 0 {
            // No pictures, and none left from an earlier run.
            let _ = std::fs::remove_file(name(""));
            let _ = std::fs::remove_file(name("-on"));
            continue;
        }
        changed += 1;
        // A margin round the differences, at least a window of context.
        let (cx0, cy0) = (x0.saturating_sub(48), y0.saturating_sub(48));
        let (cx1, cy1) = ((x1 + 48).min(view.width), (y1 + 48).min(view.height));
        let (w, h, rgb) = picture(&view, &off, &on, (cx0, cy0, cx1, cy1));
        let _ = std::fs::write(name(""), png::encode_rgb(w, h, &rgb));
        let _ = std::fs::write(name("-on"), png::encode_rgb(view.width, view.height, &on));
    }
    println!(
        "changed       {changed} of {} frames; pictures (off | on | differences) in {}",
        dirs.len(),
        out_dir.display()
    );
    if let Some(dir) = against {
        println!(
            "against       {moved_total} pixels of the depth-on frames differ from {} (pictures: before | now | differences, against-FRAME.png)",
            dir.display()
        );
    }
    Ok(())
}
