//! SPIKE: the occlusion pass on the GPU (gpu::aospike), compared with the
//! CPU term of `lighting_spike`, drawn offscreen at native size, 1080p and
//! (LG_4K) 4K. Env: LG_R (radius, view units), LG_S (strength), LG_SIZES
//! (comma list of native,1080,4k).
use gpu::aospike::AoSpike;
use gpu::wgpu;
use model2::capture;
use model2::raster::{self, Cast, Sources};
use model2::video::{self, Hud, View};
use std::collections::HashMap;
use std::time::Instant;

fn env(k: &str, d: f32) -> f32 {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}

pub struct Cpu<'a> {
    pub elig: &'a [bool],
    pub term: &'a [f32],
    pub orig: &'a [u8],
    pub w: usize,
}

fn wait(g: &gpu::Renderer) {
    let _ = g.device().poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(std::time::Duration::from_secs(20)) });
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    g: &mut gpu::Renderer,
    tag: &str,
    out: &std::path::Path,
    cap: &capture::Frame,
    side: &model2::light::Frame,
    texrom: &[u16],
    windows: (&[usize], &[usize]),
    casts: &[Cast],
    cpu: &Cpu,
) -> Result<(), String> {
    let (r, s) = (env("LG_R", 1.0), env("LG_S", 1.0));
    let sizes = std::env::var("LG_SIZES").unwrap_or_else(|_| "native,1080".into());
    let src = Sources {
        texture_rom: texrom,
        texture_point_ram: &cap.texture_point_ram,
        render_ram: &cap.render_ram,
        palette: &cap.palette,
        xoff: cap.xoff,
        yoff: cap.yoff,
    };
    let by_at: HashMap<u32, usize> = casts.iter().enumerate().filter(|(_, c)| c.survived).map(|(i, c)| (c.at, i)).collect();
    let fmt = wgpu::TextureFormat::Rgba8Unorm;
    println!("[{tag}] gpu {}", g.adapter());
    for name in sizes.split(',') {
        let base_view = match name {
            "native" => View::native(),
            "1080" => View::new(1080, 16.0 / 9.0, Hud::Edges),
            "4k" => View::new(2160, 16.0 / 9.0, Hud::Edges),
            _ => continue,
        };
        let view = base_view.with_hud_windows(windows.0, windows.1);
        let (w, h) = (view.width as u32, view.height as u32);
        let mut polys = raster::build_shaded(&cap.stream, &src, None);
        video::HudLayout::new(&cap.tiles, &cap.palette, &view).anchor(&mut polys, &view);
        let placed = raster::place(&mut polys, &view);
        let layers = video::layers_2d(&cap.tiles, &cap.palette, &view, !placed.is_empty());
        let hud_mask = view.hud_windows | view.hud_element_windows;
        // The per-polygon table.
        let mut takes = 0usize;
        let table: Vec<[f32; 8]> = placed
            .iter()
            .map(|pl| {
                let p = &polys[pl.poly];
                let none = [0.0; 8];
                if hud_mask & (1 << p.window) != 0 || matches!(raster::background_reason(p), Some("sky" | "shadow" | "checker")) {
                    return none;
                }
                let Some(&ci) = by_at.get(&p.at) else { return none };
                let c = &casts[ci];
                let Some((fx, fy)) = side.blocks.get(p.block as usize).and_then(|b| b.focal()) else { return none };
                if c.verts.len() < 3 {
                    return none;
                }
                let v: Vec<[f32; 3]> = c.verts.iter().map(|p| [p[0] / fx, p[1] / fy, p[2]]).collect();
                let (a, b) = ([v[1][0] - v[0][0], v[1][1] - v[0][1], v[1][2] - v[0][2]], [v[2][0] - v[0][0], v[2][1] - v[0][1], v[2][2] - v[0][2]]);
                let n = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
                let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                if l < 1e-10 {
                    return none;
                }
                takes += 1;
                [n[0] / l, n[1] / l, n[2] / l, 1.0, c.cx, c.cy, fx, fy]
            })
            .collect();
        println!("[{tag}] {name} {w}x{h}: {} placed polygons, {takes} take the term", placed.len());
        let mk = |g: &gpu::Renderer| {
            g.device().create_texture(&wgpu::TextureDescriptor {
                label: Some("lg"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: fmt,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let tex = mk(g);
        let tv = tex.create_view(&Default::default());
        // One timed frame: draw, wait; (ms, picture).
        let frame = |g: &mut gpu::Renderer| -> f64 {
            let t = Instant::now();
            g.present(&polys, &placed, &src, &view, &layers, &tv, fmt);
            wait(g);
            t.elapsed().as_secs_f64() * 1e3
        };
        let spike = |mode: f32, reps: u32, gbuf_only: bool| AoSpike { table: table.clone(), radius: r, strength: s, reps, gbuf_only, mode };
        g.set_ao_spike(None)?;
        frame(g); // warm-up: pipelines, uploads
        let t_base = frame(g).min(frame(g));
        let base = g.read_texture(&tex, w, h);
        g.set_ao_spike(Some(spike(0.0, 1, true)))?;
        frame(g);
        let t_gb = frame(g);
        g.set_ao_spike(Some(spike(0.0, 1, false)))?;
        frame(g);
        let t_ao1 = frame(g);
        let applied = g.read_texture(&tex, w, h);
        g.set_ao_spike(Some(spike(0.0, 11, false)))?;
        let t_ao11 = frame(g);
        g.set_ao_spike(Some(spike(1.0, 1, false)))?;
        frame(g);
        let termimg = g.read_texture(&tex, w, h);
        g.set_ao_spike(Some(spike(2.0, 1, false)))?;
        frame(g);
        let maskimg = g.read_texture(&tex, w, h);
        g.set_ao_spike(None)?;
        println!(
            "[{tag}] {name}: frame wall ms (draw+wait, min of 2): baseline {t_base:.2}; + ID pass (G-buffer) {t_gb:.2}; + occlusion 16 taps {t_ao1:.2}; 11 repeats {t_ao11:.2} => one occlusion pair of passes {:.3} ms, G-buffer pass {:.3} ms",
            (t_ao11 - t_ao1) / 10.0,
            t_gb - t_base
        );
        let rgb = |v: &[u8]| v.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect::<Vec<u8>>();
        let save = |n: &str, v: &[u8]| {
            let _ = std::fs::write(out.join(format!("{tag}-{name}-{n}.png")), png::encode_rgb(w as usize, h as usize, v));
        };
        let (b3, a3) = (rgb(&base), rgb(&applied));
        // x4 difference.
        let diff: Vec<u8> = b3.iter().zip(&a3).map(|(&b, &a)| 255 - ((b as i32 - a as i32).unsigned_abs() * 4).min(255) as u8).collect();
        save("base", &b3);
        save("ao", &a3);
        save("diff4x", &diff);
        save("term", &rgb(&termimg));
        if let Ok(cr) = std::env::var("LG_CROP") {
            let v: Vec<usize> = cr.split(',').filter_map(|s| s.parse().ok()).collect();
            let k = (1920 / w as usize).max(1) * 2;
            if v.len() == 4 && name == "1080" && v[0] + v[2] <= w as usize && v[1] + v[3] <= h as usize {
                let mut all = Vec::new();
                for im in [&b3, &a3] {
                    for y in 0..v[3] * k {
                        for x in 0..v[2] * k {
                            let o = ((v[1] + y / k) * w as usize + v[0] + x / k) * 3;
                            all.extend_from_slice(&im[o..o + 3]);
                        }
                    }
                }
                let _ = std::fs::write(out.join(format!("{tag}-{name}-crop.png")), png::encode_rgb(v[2] * k, v[3] * k * 2, &all));
            }
        }
        let n = (w * h) as usize;
        let changed = (0..n).filter(|&o| b3[o * 3..o * 3 + 3] != a3[o * 3..o * 3 + 3]).count();
        let mag: Vec<bool> = (0..n).map(|o| maskimg[o * 4..o * 4 + 3] == [255, 0, 255] && base[o * 4..o * 4 + 3] != [255, 0, 255]).collect();
        println!(
            "[{tag}] {name}: pixels changed {changed} ({:.2}%); eligible and visible (mask) {} ({:.1}%)",
            100.0 * changed as f64 / n as f64,
            mag.iter().filter(|&&m| m).count(),
            100.0 * mag.iter().filter(|&&m| m).count() as f64 / n as f64
        );
        // Mean darkening over the eligible pixels, from the images.
        let (mut sb, mut sa, mut cnt) = (0.0f64, 0.0f64, 0usize);
        let (mut d9, mut d7) = (0usize, 0usize);
        for o in (0..n).filter(|&o| mag[o]) {
            let (bl, al) = (b3[o * 3] as f64 + b3[o * 3 + 1] as f64 + b3[o * 3 + 2] as f64, a3[o * 3] as f64 + a3[o * 3 + 1] as f64 + a3[o * 3 + 2] as f64);
            if bl > 30.0 {
                sb += bl;
                sa += al;
                cnt += 1;
                let t = al / bl;
                d9 += (t < 0.9) as usize;
                d7 += (t < 0.7) as usize;
            }
        }
        println!(
            "[{tag}] {name}: over {cnt} visible eligible px of brightness>10: mean luminance ratio after/before {:.4}; px darker than x0.9 {:.1}%, than x0.7 {:.2}%",
            sa / sb.max(1.0),
            100.0 * d9 as f64 / cnt.max(1) as f64,
            100.0 * d7 as f64 / cnt.max(1) as f64
        );
        // Outside the eligible mask: what changed at 2D / HUD / sky pixels
        // (by the CPU's masks at the nearest game pixel).
        if name == "native" {
            let (mut tot, mut dif, mut agree_e, mut e_cpu_only, mut e_gpu_only) = (0.0f64, 0.0f64, 0usize, 0usize, 0usize);
            let (mut sg, mut sc, mut n_t, mut big) = (0.0f64, 0.0f64, 0usize, 0usize);
            let mut hist = [0usize; 6];
            for o in 0..n {
                let ce = cpu.elig[o];
                let ge = mag[o];
                if ce && ge {
                    agree_e += 1;
                    let gt = termimg[o * 4] as f64 / 255.0;
                    let ct = (cpu.term[o].clamp(0.0, 1.0) * 255.0).floor() as f64 / 255.0;
                    let d = (gt - ct).abs();
                    tot += d;
                    sg += gt;
                    sc += ct;
                    n_t += 1;
                    big += (d > 0.05) as usize;
                    hist[((d * 100.0) as usize).min(5)] += 1;
                } else if ce {
                    e_cpu_only += 1;
                } else if ge {
                    e_gpu_only += 1;
                }
                if !ce && !ge {
                    // 2D, HUD, sky, blob, checker, background: untouched?
                    dif += (b3[o * 3..o * 3 + 3] != a3[o * 3..o * 3 + 3]) as u8 as f64;
                }
            }
            println!(
                "[{tag}] native vs CPU: eligible in both {agree_e}, CPU only {e_cpu_only}, GPU only {e_gpu_only}; term mean GPU {:.4} CPU {:.4}; mean |diff| {:.4}; |diff|>0.05 {:.2}%; diff histogram (per 0.01, last 0.05+) {:?}",
                sg / n_t.max(1) as f64,
                sc / n_t.max(1) as f64,
                tot / n_t.max(1) as f64,
                100.0 * big as f64 / n_t.max(1) as f64,
                hist
            );
            println!("[{tag}] native: pixels in neither eligible set that changed (baseline vs AO): {dif} (must be 0)");
            let cpu_img: Vec<u8> = (0..n)
                .flat_map(|o| if cpu.elig[o] { let v = (cpu.term[o].clamp(0.0, 1.0) * 255.0) as u8; [v, v, v] } else { [255, 255, 255] })
                .collect();
            let _ = std::fs::write(out.join(format!("{tag}-native-term-cpu.png")), png::encode_rgb(w as usize, h as usize, &cpu_img));
            let _ = (&cpu.orig, cpu.w);
        }
    }
    Ok(())
}
