//! SPIKE `remodel shader-pass [DIR]`: draw captured frames offscreen, then
//! run a WGSL file's pass over the finished picture (gpu::userpass). Env:
//! SP_SHADER (file; none = only the baseline), SP_VIEW (native|1080),
//! SP_FRAMES (A,B), SP_OUT (dir), SP_PARAMS (8 floats, comma separated),
//! SP_REPEAT (timed passes), SP_SAVE (write PNGs).
use gpu::userpass::{parse_params, UserPass};
use gpu::wgpu;
use model2::capture::{self, Frame};
use model2::raster;
use model2::video::{self, View};
use std::path::Path;
use std::time::Instant;

pub fn run(root: &Path, windows: (&[usize], &[usize])) -> Result<(), String> {
    let env = |k: &str| std::env::var(k).ok();
    let texture_rom = capture::load_texture_rom(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let mut g = gpu::Renderer::new()?;
    println!("gpu           {}", g.adapter());
    let view = if env("SP_VIEW").as_deref() == Some("1080") {
        View::new(1080, 16.0 / 9.0, video::Hud::Edges)
    } else {
        View::native()
    }
    .with_hud_windows(windows.0, windows.1);
    let (w, h) = (view.width as u32, view.height as u32);
    println!("view          {w}x{h}");
    let frames: Vec<u64> = env("SP_FRAMES").unwrap_or_default().split(',').filter_map(|s| s.parse().ok()).collect();
    let out = std::path::PathBuf::from(env("SP_OUT").unwrap_or_else(|| ".".into()));
    let repeat: usize = env("SP_REPEAT").and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut params = [0f32; 8];
    if let Some(p) = env("SP_PARAMS") {
        for (i, v) in p.split(',').enumerate().take(8) {
            params[i] = v.parse().unwrap_or(0.0);
        }
    }
    let source = env("SP_SHADER").map(|p| std::fs::read_to_string(&p).map_err(|e| format!("{p}: {e}"))).transpose()?;
    let fmt = wgpu::TextureFormat::Rgba8Unorm;
    let t = Instant::now();
    let pass = match &source {
        None => None,
        Some(s) => {
            let ps = parse_params(s);
            println!("params        {} declared: {:?}", ps.len(), ps);
            let r = UserPass::new(g.device(), s, fmt);
            println!("compile       {:.2} ms (create module + pipeline, error scopes popped)", t.elapsed().as_secs_f64() * 1e3);
            match r {
                Ok(p) => Some(p),
                Err(e) => {
                    println!("shader error  falling back to no shader:\n{e}");
                    None
                }
            }
        }
    };
    let mk = |g: &gpu::Renderer, label| {
        g.device().create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: fmt,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    };
    let wait = |g: &gpu::Renderer| {
        let _ = g.device().poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(std::time::Duration::from_secs(20)) });
    };
    let mut dirs: Vec<_> = std::fs::read_dir(root)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("meta.txt").exists())
        .collect();
    dirs.sort();
    for d in &dirs {
        let f = Frame::load(d).map_err(|e| format!("{}: {e}", d.display()))?;
        if !frames.contains(&f.frame) {
            continue;
        }
        let src = f.sources(&texture_rom);
        let mut polys = raster::build_shaded_with(&f.stream, &src, None, None);
        video::HudLayout::new(&f.tiles, &f.palette, &view).anchor(&mut polys, &view);
        let placed = raster::place(&mut polys, &view);
        let layers = video::layers_2d(&f.tiles, &f.palette, &view, !placed.is_empty());
        let pic = mk(&g, "picture");
        let pv = pic.create_view(&Default::default());
        g.present(&polys, &placed, &src, &view, &layers, &pv, fmt);
        let base = g.read_texture(&pic, w, h);
        let plain = g.frame(&polys, &placed, &src, &view, &layers);
        // The same picture as the CLI's RGB path gives?
        let same_rgb = base.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).eq(plain.iter().copied());
        let alpha = base.as_chunks::<4>().0.iter().filter(|p| p[3] != 255).count();
        print!("frame {:>5}: baseline == frame() rgb: {same_rgb}, non-opaque alpha px {alpha}", f.frame);
        let Some(pass) = &pass else {
            println!(" (no shader)");
            continue;
        };
        let target = mk(&g, "shaded");
        let tv = target.create_view(&Default::default());
        let mut enc = g.device().create_command_encoder(&Default::default());
        let t = Instant::now();
        pass.encode(g.device(), g.queue(), &mut enc, &pv, &tv, (w, h), (496.0, 384.0), f.frame as u32, params);
        g.queue().submit([enc.finish()]);
        wait(&g);
        let first = t.elapsed().as_secs_f64() * 1e3;
        let shaded = g.read_texture(&target, w, h);
        let diff = base.iter().zip(&shaded).filter(|(a, b)| a != b).count();
        let maxd = base.iter().zip(&shaded).map(|(&a, &b)| (a as i32 - b as i32).abs()).max().unwrap_or(0);
        print!("; first pass {first:.2} ms; differing bytes vs baseline {diff} of {} (max {maxd})", base.len());
        if repeat > 0 {
            let mut enc = g.device().create_command_encoder(&Default::default());
            let t = Instant::now();
            for _ in 0..repeat {
                pass.encode(g.device(), g.queue(), &mut enc, &pv, &tv, (w, h), (496.0, 384.0), f.frame as u32, params);
            }
            g.queue().submit([enc.finish()]);
            wait(&g);
            print!("; {:.3} ms/pass over {repeat} (submit+wait)", t.elapsed().as_secs_f64() * 1e3 / repeat as f64);
        }
        println!();
        if env("SP_SAVE").is_some() {
            let _ = std::fs::create_dir_all(&out);
            let rgb = |v: &[u8]| v.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect::<Vec<u8>>();
            let _ = std::fs::write(out.join(format!("{:05}-shaded.png", f.frame)), png::encode_rgb(w as usize, h as usize, &rgb(&shaded)));
            let _ = std::fs::write(out.join(format!("{:05}-plain.png", f.frame)), png::encode_rgb(w as usize, h as usize, &rgb(&base)));
        }
    }
    Ok(())
}
