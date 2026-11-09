//! `remodel depth-check`: draw captured frames on the GPU with the depth
//! buffer off and on, report where they differ and write the differences
//! as pictures to look at (docs/enhancements.md: every difference should be
//! a fix, such as intersecting polygons or a mis-sorted one, and not a new
//! glitch).

use model2::capture::{self, Frame};
use model2::raster;
use model2::video::{self, View};
use std::path::Path;

/// The frame as the GPU draws it with `depth` on or off.
fn draw(gpu: &mut gpu::Renderer, f: &Frame, rom: &[u16], view: &View, depth: bool, msaa: u32) -> (Vec<u8>, usize) {
    gpu.set_quality(gpu::Quality { msaa, depth });
    let src = f.sources(rom);
    let mut polys = raster::build(&f.stream, &src);
    video::HudLayout::new(&f.tiles, &f.palette, view).anchor(&mut polys, view);
    let placed = raster::place(&mut polys, view);
    // DEPTH_PROBE=X,Y (output pixel) with DEPTH_PROBE_FRAME=N: the polygons
    // covering it in draw order, with their depth there.
    if let Some((x, y)) = std::env::var("DEPTH_PROBE").ok().and_then(|v| {
        let (x, y) = v.split_once(',')?;
        Some((x.trim().parse::<f32>().ok()?, y.trim().parse::<f32>().ok()?))
    }) {
        if depth && std::env::var("DEPTH_PROBE_FRAME").is_ok_and(|n| n == f.frame.to_string()) {
            println!("polygons at pixel {x},{y} in draw order:");
            for pl in &placed {
                let q = &pl.points;
                for k in 1..q.len().saturating_sub(1) {
                    let (a, b, c) = (q[0], q[k], q[k + 1]);
                    let d = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
                    let l1 = ((b.y - c.y) * (x - c.x) + (c.x - b.x) * (y - c.y)) / d;
                    let l2 = ((c.y - a.y) * (x - c.x) + (a.x - c.x) * (y - c.y)) / d;
                    let l3 = 1.0 - l1 - l2;
                    if l1 >= 0.0 && l2 >= 0.0 && l3 >= 0.0 {
                        let p = &polys[pl.poly];
                        println!(
                            "  window {} rank {} group {} key {:.1} 1/z {:.6}{}{}  colour {:?} luma {} header {:04x?}{}",
                            p.window,
                            pl.rank,
                            pl.group,
                            p.key,
                            l1 * a.iz + l2 * b.iz + l3 * c.iz,
                            if pl.background { " background" } else { "" },
                            if p.checker { " checker" } else { "" },
                            p.colour,
                            p.luma,
                            p.header,
                            if p.texture.is_some() { " textured" } else { "" }
                        );
                    }
                }
            }
        }
    }
    let layers = video::layers_2d(&f.tiles, &f.palette, view, !placed.is_empty());
    (gpu.frame(&polys, &placed, &src, view, &layers), polys.len())
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

pub fn run(
    root: &Path,
    hud_windows: (&[usize], &[usize]),
    view: Option<View>,
    msaa: u32,
    out_dir: &Path,
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
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    println!("{:<8} {:>6} {:>9} {:>9}  bounding box", "frame", "polys", "differ", ">16");
    let mut changed = 0;
    for d in &dirs {
        let f = Frame::load(d).map_err(|e| format!("{}: {e}", d.display()))?;
        let (off, n) = draw(&mut gpu, &f, &texture_rom, &view, false, msaa);
        let (on, _) = draw(&mut gpu, &f, &texture_rom, &view, true, msaa);
        let (mut differ, mut large) = (0, 0);
        let (mut x0, mut y0, mut x1, mut y1) = (view.width, view.height, 0, 0);
        for i in 0..view.width * view.height {
            let m = (0..3).map(|c| off[i * 3 + c].abs_diff(on[i * 3 + c])).max().unwrap_or(0);
            if m > 0 {
                differ += 1;
                large += (m > 16) as usize;
                let (x, y) = (i % view.width, i / view.width);
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            }
        }
        // DEPTH_DARK_FRAME=N: where the picture goes from dark to light
        // with the depth buffer on (a polygon that went missing).
        if std::env::var("DEPTH_DARK_FRAME").is_ok_and(|n| n == f.frame.to_string()) {
            let sum = |v: &[u8], i: usize| v[i * 3] as u32 + v[i * 3 + 1] as u32 + v[i * 3 + 2] as u32;
            let hits: Vec<usize> =
                (0..view.width * view.height).filter(|&i| sum(&off, i) < 40 && sum(&on, i) > 200).collect();
            if let Some(&i) = hits.get(hits.len() / 2) {
                println!("dark to light: {} pixels, e.g. {},{}", hits.len(), i % view.width, i / view.width);
            }
            let hits: Vec<usize> =
                (0..view.width * view.height).filter(|&i| sum(&on, i) < 150 && sum(&off, i) > 350).collect();
            if let Some(&i) = hits.get(hits.len() / 2) {
                println!("light to dark: {} pixels, e.g. {},{}", hits.len(), i % view.width, i / view.width);
            }
        }
        let pct = |n: usize| 100.0 * n as f64 / (view.width * view.height) as f64;
        if differ == 0 {
            println!("{:<8} {n:>6} {:>8.3}% {:>8.3}%", f.frame, 0.0, 0.0);
            continue;
        }
        changed += 1;
        println!("{:<8} {n:>6} {:>8.3}% {:>8.3}%  ({x0},{y0})-({x1},{y1})", f.frame, pct(differ), pct(large));
        // A margin round the differences, at least a window of context.
        let (cx0, cy0) = (x0.saturating_sub(48), y0.saturating_sub(48));
        let (cx1, cy1) = ((x1 + 48).min(view.width), (y1 + 48).min(view.height));
        let (w, h, rgb) = picture(&view, &off, &on, (cx0, cy0, cx1, cy1));
        let _ = std::fs::write(out_dir.join(format!("depth-{:05}.png", f.frame)), png::encode_rgb(w, h, &rgb));
        let _ = std::fs::write(
            out_dir.join(format!("depth-{:05}-on.png", f.frame)),
            png::encode_rgb(view.width, view.height, &on),
        );
    }
    println!(
        "changed       {changed} of {} frames; pictures (off | on | differences) in {}",
        dirs.len(),
        out_dir.display()
    );
    Ok(())
}
