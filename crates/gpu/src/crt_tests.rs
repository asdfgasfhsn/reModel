//! Tests of the CRT pass and the wipe (`super`), on the GPU.

use crate::{Renderer, FORMAT};

/// A test picture of `w` x `h`: a grey ramp across the top half, white and
/// red bars below, a black box at the left.
fn pattern(w: u32, h: u32) -> Vec<u8> {
    let mut px = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let v = if y < h / 2 {
                let g = (x * 255 / (w - 1)) as u8;
                [g, g, g]
            } else if x < w / 4 {
                [0, 0, 0]
            } else if (x / 16) % 2 == 0 {
                [255, 255, 255]
            } else {
                [200, 30, 30]
            };
            px.extend([v[0], v[1], v[2], 255]);
        }
    }
    px
}

/// `pattern` through the CRT pass at `strength`, read back.
fn run(gpu: &mut Renderer, w: u32, h: u32, strength: f32) -> Vec<u8> {
    gpu.crt.input(&gpu.device, w, h, FORMAT);
    let texture = gpu.crt.input.as_ref().unwrap().texture.clone();
    gpu.queue.write_texture(
        texture.as_image_copy(),
        &pattern(w, h),
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: None },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    let out = gpu.texture("crt out", w, h, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    let target = out.create_view(&Default::default());
    gpu.crt.encode(&gpu.device, &mut enc, &target, FORMAT, (0, 0), (w, h), strength, 384.0);
    gpu.queue.submit([enc.finish()]);
    gpu.read_texture(&out, w, h)
}

/// With `REMODEL_CRT_DUMP=dir`, the picture as a PPM there (for a look).
fn dump(name: &str, w: u32, h: u32, rgba: &[u8]) {
    let Ok(dir) = std::env::var("REMODEL_CRT_DUMP") else { return };
    let mut f = format!("P6\n{w} {h}\n255\n").into_bytes();
    f.extend(rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2]]));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(format!("{dir}/{name}.ppm"), f).unwrap();
}

fn luma(rgba: &[u8], w: u32, x: u32, y: u32) -> f32 {
    let i = ((y * w + x) * 4) as usize;
    0.299 * rgba[i] as f32 + 0.587 * rgba[i + 1] as f32 + 0.114 * rgba[i + 2] as f32
}

/// The scanlines follow the game's 384 lines: at strength 1 a line's
/// centre is brighter than its edge, the pattern repeats every line, black
/// stays black, and strength 0 leaves a flat part alone.
#[test]
fn scanlines_follow_the_games_lines() {
    let Some(mut gpu) = Renderer::for_test() else { return };
    let (w, h) = (640, 1152);
    let on = run(&mut gpu, w, h, 1.0);
    dump("crt-on", w, h, &on);
    // Three output pixels a game line; on the red bars (a dark-ish colour,
    // so the beam is thin), away from the edges.
    let ppl = h / 384;
    let x = w * 5 / 8 + 8;
    let at = |l: u32, k: u32| luma(&on, w, x, l * ppl + k);
    let (centre, border) = (at(300, 1), at(300, 0).min(at(300, 2)));
    assert!(centre > border * 1.15, "centre {centre} border {border}");
    // The same on another line: aligned to the lines, not to the output.
    assert!((at(301, 1) - centre).abs() < 14.0, "{} {centre}", at(301, 1));
    // Black stays black (through the mask, vignette and the rest).
    for y in [h * 5 / 8, h * 3 / 4] {
        assert!(luma(&on, w, w / 16, y) < 3.0, "{}", luma(&on, w, w / 16, y));
    }
    // Off: a flat colour stays (blur and the rest do nothing to it).
    let off = run(&mut gpu, w, h, 0.0);
    dump("crt-off", w, h, &off);
    let flat = |y| luma(&off, w, w / 8, y);
    assert!((flat(h * 5 / 8) - flat(h * 5 / 8 + 1)).abs() < 3.0);
}

/// The pass is a pure function of the picture and strength, and at under
/// about 1.5 pixels a line it has no scanlines or mask to beat against the
/// pixel grid.
#[test]
fn crt_is_deterministic_and_fades_out_at_low_density() {
    let Some(mut gpu) = Renderer::for_test() else { return };
    let a = run(&mut gpu, 320, 576, 0.6);
    assert!(a == run(&mut gpu, 320, 576, 0.6));
    let low = run(&mut gpu, 320, 384, 1.0);
    dump("crt-low", 320, 384, &low);
    let d = (luma(&low, 320, 200, 300) - luma(&low, 320, 200, 301)).abs();
    assert!(d < 12.0, "{d}");
}

/// The wipe copies a rectangle of the kept picture and nothing else.
#[test]
fn wipe_copies_the_rectangle_only() {
    let Some(mut gpu) = Renderer::for_test() else { return };
    let (w, h) = (64, 32);
    let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
    let paint = |gpu: &mut Renderer, t: &wgpu::Texture, v: u8| {
        let view = t.create_view(&Default::default());
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        let c = v as f64 / 255.0;
        drop(enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: c, g: c, b: c, a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        }));
        gpu.queue.submit([enc.finish()]);
    };
    let target = gpu.texture("wipe target", w, h, usage);
    paint(&mut gpu, &target, 200);
    gpu.wipe_source(w, h, FORMAT);
    let before = gpu.crt.before.as_ref().unwrap().texture.clone();
    paint(&mut gpu, &before, 50);
    gpu.wipe(&target.create_view(&Default::default()), FORMAT, (8, 4, 20, 10));
    let got = gpu.read_texture(&target, w, h);
    let at = |x: u32, y: u32| got[((y * w + x) * 4) as usize];
    assert_eq!((at(8, 4), at(27, 13)), (50, 50), "inside");
    assert_eq!((at(7, 4), at(28, 13), at(8, 3), at(27, 14)), (200, 200, 200, 200), "outside");
}
