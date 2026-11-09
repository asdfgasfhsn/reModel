//! `remodel gpu-check`: draw captured frames with the software rasteriser
//! and the GPU one, time both and compare them.

use model2::capture::{self, Frame};
use model2::raster;
use model2::video::{self, Hud, View};
use std::path::Path;
use std::time::Instant;

/// How two 3D layers differ.
#[derive(Clone, Default)]
struct Diff {
    pixels: usize,
    /// Drawn by one renderer only.
    coverage: usize,
    /// Drawn by both, in different colours / by more than 16 in a channel.
    colour: usize,
    large: usize,
}

fn diff(a: &(Vec<u8>, Vec<bool>), b: &(Vec<u8>, Vec<bool>)) -> Diff {
    let mut d = Diff { pixels: a.1.len(), ..Default::default() };
    for i in 0..a.1.len() {
        if a.1[i] != b.1[i] {
            d.coverage += 1;
        } else if a.1[i] {
            let m = (0..3).map(|c| a.0[i * 3 + c].abs_diff(b.0[i * 3 + c])).max().unwrap_or(0);
            d.colour += (m > 0) as usize;
            d.large += (m > 16) as usize;
        }
    }
    d
}

fn pct(n: usize, of: usize) -> f64 {
    100.0 * n as f64 / of.max(1) as f64
}

/// Software | GPU | differences (white: coverage, red: colour, scaled x8).
fn comparison(view: &View, sw: &(Vec<u8>, Vec<bool>), gpu: &(Vec<u8>, Vec<bool>)) -> Vec<u8> {
    let (w, h) = (view.width, view.height);
    let mut out = vec![0u8; w * 3 * h * 3];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let row = y * w * 3;
            out[(row + x) * 3..(row + x) * 3 + 3].copy_from_slice(&sw.0[i * 3..i * 3 + 3]);
            out[(row + w + x) * 3..(row + w + x) * 3 + 3].copy_from_slice(&gpu.0[i * 3..i * 3 + 3]);
            let d = if sw.1[i] != gpu.1[i] {
                [255, 255, 255]
            } else {
                let m = (0..3).map(|c| sw.0[i * 3 + c].abs_diff(gpu.0[i * 3 + c])).max().unwrap_or(0);
                [m.saturating_mul(8), 0, 0]
            };
            out[(row + 2 * w + x) * 3..(row + 2 * w + x) * 3 + 3].copy_from_slice(&d);
        }
    }
    out
}

/// Software supersampled | GPU multisampled | differences (x8) of whole
/// frames.
fn frame_comparison(view: &View, sw: &[u8], gpu: &[u8]) -> Vec<u8> {
    let (w, h) = (view.width, view.height);
    let mut out = vec![0u8; w * 3 * h * 3];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 3;
            let row = y * w * 3;
            out[(row + x) * 3..(row + x) * 3 + 3].copy_from_slice(&sw[i..i + 3]);
            out[(row + w + x) * 3..(row + w + x) * 3 + 3].copy_from_slice(&gpu[i..i + 3]);
            let d = [0, 1, 2].map(|c| sw[i + c].abs_diff(gpu[i + c]).saturating_mul(8));
            out[(row + 2 * w + x) * 3..(row + 2 * w + x) * 3 + 3].copy_from_slice(&d);
        }
    }
    out
}

/// Mean time of `n` runs of `f`, in ms.
fn time(n: usize, mut f: impl FnMut()) -> f64 {
    let t = Instant::now();
    for _ in 0..n {
        f();
    }
    t.elapsed().as_secs_f64() * 1e3 / n as f64
}

/// What multisampling and the depth buffer cost the GPU: whole frames
/// (3D layer, compose, readback) per configuration, mean over the frames.
fn cost(gpu: &mut gpu::Renderer, root_dirs: &[std::path::PathBuf], texture_rom: &[u16], views: &[(&str, View)]) {
    let configs = [(1, false), (4, false), (8, false), (1, true), (4, true), (8, true)];
    let frames: Vec<_> = root_dirs.iter().filter_map(|d| Frame::load(d).ok()).collect();
    println!("cost          whole frame on the GPU incl. readback, mean over {} frames, ms", frames.len());
    print!("{:<12}", "");
    for (m, d) in configs {
        print!(
            " {:>10}",
            format!("{}{}", if m == 1 { "off".to_string() } else { format!("{m}x") }, if d { "+depth" } else { "" })
        );
    }
    println!();
    for (name, view) in views {
        print!("{name:<12}");
        for (msaa, depth) in configs {
            let q = gpu.set_quality(gpu::Quality { msaa, depth });
            if q.msaa != msaa {
                print!(" {:>10}", "n/a");
                continue;
            }
            let mut total = 0.0;
            for f in &frames {
                let src = f.sources(texture_rom);
                let mut polys = raster::build(&f.stream, &src);
                video::HudLayout::new(&f.tiles, &f.palette, view).anchor(&mut polys, view);
                let placed = raster::place(&mut polys, view);
                let layers = video::layers_2d(&f.tiles, &f.palette, view, !placed.is_empty());
                gpu.frame(&polys, &placed, &src, view, &layers);
                total += time(3, || {
                    gpu.frame(&polys, &placed, &src, view, &layers);
                });
            }
            print!(" {:>10.2}", total / frames.len().max(1) as f64);
        }
        println!();
    }
}

pub fn run(root: &Path, hud_windows: (&[usize], &[usize]), aa: u32, depth: bool) -> Result<(), String> {
    let texture_rom = capture::load_texture_rom(root)
        .map_err(|e| format!("{}: {e} (capture with `make capture`)", root.display()))?;
    let mut gpu = gpu::Renderer::new()?;
    println!("gpu           {}", gpu.adapter());
    let q = gpu.set_quality(gpu::Quality { msaa: aa, depth });
    println!(
        "quality       {} samples a pixel on the GPU ({} supersampled in software), depth buffer {}",
        q.msaa,
        if aa > 1 {
            format!("{}x{}", super::supersample_factor(aa), super::supersample_factor(aa))
        } else {
            "no".into()
        },
        if depth { "on" } else { "off" }
    );
    let ss = if aa > 1 { super::supersample_factor(aa) } else { 1 };
    let mut dirs: Vec<_> = std::fs::read_dir(root)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("meta.txt").exists())
        .collect();
    dirs.sort();
    let mut wide = View::new(1080, 16.0 / 9.0, Hud::Edges);
    wide = wide.with_hud_windows(hud_windows.0, hud_windows.1);
    let mut native = View::native();
    native = native.with_hud_windows(hud_windows.0, hud_windows.1);
    let views = [("native", native), ("1080p 16:9", wide)];
    // Warm up (shader compilation, first allocations) before timing.
    if let Some(d) = dirs.first() {
        let f = Frame::load(d).map_err(|e| e.to_string())?;
        let src = f.sources(&texture_rom);
        let mut polys = raster::build(&f.stream, &src);
        let placed = raster::place(&mut polys, &native);
        gpu.draw(&polys, &placed, &src, &native);
    }
    println!(
        "{:<8} {:>6}  {:<11} {:>9} {:>9}  {:>9} {:>9} {:>9}  {:>9}  {:>8} {:>8} {:>8}",
        "frame", "polys", "view", "software", "gpu", "coverage", "colour", ">16", "compose", "build", "place", "2d"
    );
    let mut totals = vec![(0.0, 0.0, Diff::default()); views.len()];
    // Frames where the fast 2D layer path differs from the reference one.
    let mut tiles_differ = Vec::new();
    for d in &dirs {
        let f = Frame::load(d).map_err(|e| format!("{}: {e}", d.display()))?;
        let src = f.sources(&texture_rom);
        let polys = raster::build(&f.stream, &src);
        for (k, (name, view)) in views.iter().enumerate() {
            let hud = video::HudLayout::new(&f.tiles, &f.palette, view);
            let mut sw_polys = polys.clone();
            hud.anchor(&mut sw_polys, view);
            let t = Instant::now();
            let sw = raster::draw_view_depth(&mut sw_polys, &src, view, depth);
            let sw_ms = t.elapsed().as_secs_f64() * 1e3;
            let t = Instant::now();
            let mut gpu_polys = polys.clone();
            hud.anchor(&mut gpu_polys, view);
            let placed = raster::place(&mut gpu_polys, view);
            let g = gpu.draw(&gpu_polys, &placed, &src, view);
            let gpu_ms = t.elapsed().as_secs_f64() * 1e3;
            let mut df = diff(&sw, &g);
            // What a live frame costs on the CPU before the GPU: parse the
            // stream, place the polygons, build the 2D layers.
            let ms = |t: Instant| t.elapsed().as_secs_f64() * 1e3;
            let t = Instant::now();
            let mut prep_polys = raster::build(&f.stream, &src);
            let build_ms = ms(t);
            let t = Instant::now();
            hud.anchor(&mut prep_polys, view);
            let _ = raster::place(&mut prep_polys, view);
            let place_ms = ms(t);
            // The whole frame: the GPU's compositor against the CPU's, both
            // given the GPU's 3D layer, so only the 2D blend is compared.
            if k == 0 && video::tile_planes_check(&f.tiles, &f.palette) != 0 {
                tiles_differ.push(f.frame);
            }
            let t = Instant::now();
            let layers = video::layers_2d(&f.tiles, &f.palette, view, !placed.is_empty());
            let layers_ms = ms(t);
            let gpu_frame = gpu.frame(&gpu_polys, &placed, &src, view, &layers);
            let cpu_frame = video::compose(&f.tiles, &f.palette, Some((&g.0, &g.1)), view);
            let mut compose_diff =
                gpu_frame.as_chunks::<3>().0.iter().zip(cpu_frame.as_chunks::<3>().0).filter(|(a, b)| a != b).count();
            // Multisampled: the 3D layers no longer match pixel for pixel.
            // Compare whole frames instead, the GPU's against the software
            // renderer's supersampled one.
            let mut frames_png = None;
            if ss > 1 {
                let big = view.supersampled(ss);
                let mut p = polys.clone();
                let big_layer = raster::draw_view_depth(&mut p, &src, &big, depth);
                let big_frame = video::compose(&f.tiles, &f.palette, Some((&big_layer.0, &big_layer.1)), &big);
                let sw_frame = video::downsample(&big_frame, big.width, big.height, ss);
                let pairs = sw_frame.as_chunks::<3>().0.iter().zip(gpu_frame.as_chunks::<3>().0);
                df.coverage = 0;
                df.colour = pairs.clone().filter(|(a, b)| a != b).count();
                df.large = pairs.filter(|(a, b)| (0..3).any(|c| a[c].abs_diff(b[c]) > 16)).count();
                compose_diff = 0;
                frames_png = Some(frame_comparison(view, &sw_frame, &gpu_frame));
            }
            println!(
                "{:<8} {:>6}  {name:<11} {sw_ms:>7.1}ms {gpu_ms:>7.1}ms  {:>8.3}% {:>8.3}% {:>8.3}%  {:>8.3}%  {build_ms:>6.2}ms {place_ms:>6.2}ms {layers_ms:>6.2}ms",
                f.frame,
                polys.len(),
                pct(df.coverage, df.pixels),
                pct(df.colour, df.pixels),
                pct(df.large, df.pixels),
                pct(compose_diff, df.pixels)
            );
            if k == 1 {
                let _ = std::fs::write(d.join("gpu-frame.png"), png::encode_rgb(view.width, view.height, &gpu_frame));
            }
            if k == 0 {
                let rgb = frames_png.unwrap_or_else(|| comparison(view, &sw, &g));
                let png = png::encode_rgb(view.width * 3, view.height, &rgb);
                let _ = std::fs::write(d.join("gpu-check.png"), png);
            }
            let t = &mut totals[k];
            t.0 += sw_ms;
            t.1 += gpu_ms;
            t.2.pixels += df.pixels;
            t.2.coverage += df.coverage;
            t.2.colour += df.colour;
            t.2.large += df.large;
        }
    }
    let n = dirs.len().max(1) as f64;
    for ((name, _), (sw, g, df)) in views.iter().zip(&totals) {
        println!(
            "mean     {:>6}  {name:<11} {:>7.1}ms {:>7.1}ms  {:>8.3}% {:>8.3}% {:>8.3}%",
            "",
            sw / n,
            g / n,
            pct(df.coverage, df.pixels),
            pct(df.colour, df.pixels),
            pct(df.large, df.pixels)
        );
    }
    if aa > 1 {
        println!("(multisampled: coverage is 0 and colour/>16 count pixels of the whole frame, the GPU's against the software renderer's supersampled one)");
    }
    cost(&mut gpu, &dirs, &texture_rom, &views);
    match tiles_differ.len() {
        0 => println!("2d layers     fast path identical to the reference on all {} frames", dirs.len()),
        n => println!("2d layers     fast path DIFFERS from the reference on {n} frames: {tiles_differ:?}"),
    }
    println!("comparisons   {}/*/gpu-check.png (software | GPU | differences)", root.display());
    println!("gpu frames    {}/*/gpu-frame.png (1080p 16:9, composed on the GPU)", root.display());
    Ok(())
}
