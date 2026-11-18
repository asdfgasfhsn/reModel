//! `remodel look-render`: draw captured frames on the GPU without and with a
//! look (docs/texture-packs.md, "Looks"; docs/enhancements.md, "Looks"):
//! the look's parameters can be given on the command line, so an agent
//! tuning a look renders the same frames again after each change without a
//! window. Writes `FRAME-plain.png`, `FRAME-look.png` and a `FRAME-sheet.png`
//! of the two side by side; `--bench N` also times the frame's draw
//! (GPU time, no read-back) with the look off and on.

use gpu::Quality;
use model2::capture::{self, Frame};
use model2::look::Look;
use model2::raster;
use model2::video::{self, View};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

pub struct Options<'a> {
    pub view: View,
    pub windows: (&'a [usize], &'a [usize]),
    /// Smooth shading's options (and with them the quality's shading).
    pub light: Option<&'a model2::light::Options>,
    pub look: Look,
    pub pack: Option<&'a Path>,
    pub quality: Quality,
    /// Only these captured frames (empty: all).
    pub frames: Vec<u64>,
    /// Draws to time for each of off and on (0: none).
    pub bench: usize,
}

/// Mean ms of `n` draws of the frame into a kept texture, waiting for the
/// GPU at the end (no read-back).
#[allow(clippy::too_many_arguments)]
fn time_draws(
    gpu: &mut gpu::Renderer,
    polys: &[raster::Poly],
    placed: &[raster::Placed],
    src: &raster::Sources,
    view: &View,
    layers: &video::Layers2d,
    n: usize,
    version: u64,
) -> f64 {
    let target = gpu.device().create_texture(&gpu::wgpu::TextureDescriptor {
        label: Some("look bench"),
        size: gpu::wgpu::Extent3d { width: view.width as u32, height: view.height as u32, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: gpu::wgpu::TextureDimension::D2,
        format: gpu::wgpu::TextureFormat::Rgba8Unorm,
        usage: gpu::wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let tv = target.create_view(&Default::default());
    let wait = |g: &gpu::Renderer| {
        let _ = g.device().poll(gpu::wgpu::PollType::Wait { submission_index: None, timeout: None });
    };
    // Warm up (pipelines, kept targets), then time.
    // The game's RAM as the player knows it: unchanged between draws, so
    // nothing is uploaded or hashed again.
    for _ in 0..3 {
        gpu.set_ram_versions(version, version);
        gpu.present(polys, placed, src, view, layers, &tv, gpu::wgpu::TextureFormat::Rgba8Unorm);
    }
    wait(gpu);
    gpu.set_profile(true);
    gpu.take_phases();
    let t = Instant::now();
    for _ in 0..n {
        gpu.set_ram_versions(version, version);
        gpu.present(polys, placed, src, view, layers, &tv, gpu::wgpu::TextureFormat::Rgba8Unorm);
    }
    wait(gpu);
    let ms = t.elapsed().as_secs_f64() * 1e3 / n as f64;
    let p = gpu.take_phases();
    gpu.set_profile(false);
    let each = |d: std::time::Duration| d.as_secs_f64() * 1e3 / n as f64;
    println!(
        "              host: prep {:.2} resources {:.2} pass {:.2} rest {:.2} submit {:.2} ms (look: polygon ids {:.2}, id pass {:.2})",
        each(p.prep3d),
        each(p.resources3d),
        each(p.pass3d),
        each(p.rest),
        each(p.submit),
        each(p.look),
        each(p.look_id)
    );
    ms
}

pub fn run(root: &Path, out_dir: &Path, o: &Options) -> Result<(), String> {
    let texture_rom = capture::load_texture_rom(root)
        .map_err(|e| format!("{}: {e} (capture with `make capture`)", root.display()))?;
    let mut gpu = gpu::Renderer::new()?;
    let view = o.view.with_hud_windows(o.windows.0, o.windows.1);
    if let Some(dir) = o.pack {
        gpu.set_pack(Some(Box::new(crate::texpack::Packs::new(None, Some(dir.to_path_buf()))?)));
    }
    let look = Arc::new(o.look.clone());
    println!("gpu           {}", gpu.adapter());
    println!("view          {}x{}", view.width, view.height);
    println!("look          {}", o.look.name.as_deref().unwrap_or("(unnamed)"));
    for l in o.look.to_text().lines().filter(|l| !l.starts_with('#') && !l.starts_with("format")) {
        println!("  {l}");
    }
    let mut dirs: Vec<_> = std::fs::read_dir(root)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("meta.txt").exists())
        .collect();
    dirs.sort();
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    for d in &dirs {
        let f = Frame::load(d).map_err(|e| format!("{}: {e}", d.display()))?;
        if !o.frames.is_empty() && !o.frames.contains(&f.frame) {
            continue;
        }
        let src = f.sources(&texture_rom);
        let side = f.light_frame();
        let mut polys = raster::build_shaded_with(&f.stream, &src, side.as_ref(), o.light);
        if polys.is_empty() {
            continue;
        }
        video::HudLayout::new(&f.tiles, &f.palette, &view).anchor(&mut polys, &view);
        let placed = raster::place(&mut polys, &view);
        let layers = video::layers_2d(&f.tiles, &f.palette, &view, !placed.is_empty());
        gpu.set_quality(o.quality);
        gpu.set_look(None);
        let plain = gpu.frame(&polys, &placed, &src, &view, &layers);
        gpu.set_look(Some(look.clone()));
        let styled = gpu.frame(&polys, &placed, &src, &view, &layers);
        let (w, h) = (view.width, view.height);
        let name = |s: &str| out_dir.join(format!("{:05}-{s}.png", f.frame));
        let save = |s: &str, rgb: &[u8], w: usize| {
            let _ = std::fs::write(name(s), png::encode_rgb(w, h, rgb));
        };
        save("plain", &plain, w);
        save("look", &styled, w);
        let mut sheet = Vec::with_capacity(plain.len() * 2);
        for y in 0..h {
            sheet.extend_from_slice(&plain[y * w * 3..(y + 1) * w * 3]);
            sheet.extend_from_slice(&styled[y * w * 3..(y + 1) * w * 3]);
        }
        save("sheet", &sheet, w * 2);
        let changed = plain.chunks(3).zip(styled.chunks(3)).filter(|(a, b)| a != b).count();
        let mut line = format!(
            "frame {:>5}: {} polygons (typical distance {:.0}), {:.1}% of pixels changed",
            f.frame,
            placed.len(),
            gpu::depth_reference(&polys, &placed).unwrap_or(0.0),
            100.0 * changed as f64 / (w * h) as f64
        );
        if o.bench > 0 {
            gpu.set_look(None);
            let off = time_draws(&mut gpu, &polys, &placed, &src, &view, &layers, o.bench, f.frame + 1);
            gpu.set_look(Some(look.clone()));
            let on = time_draws(&mut gpu, &polys, &placed, &src, &view, &layers, o.bench, f.frame + 1);
            line += &format!("; draw {off:.2} ms without, {on:.2} ms with ({:+.2})", on - off);
        }
        println!("{line}");
    }
    Ok(())
}
