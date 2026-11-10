//! `remodel temporal-probe`: the GPU's temporal anti-aliasing path
//! (`gpu::Quality::temporal`) on captured frames and on a live run, to
//! look at. On captures: the identity (temporal at 100% unjittered with
//! the checkerboard kept draws what the off path draws, byte for byte),
//! jittered frames, and the checkerboard blended against kept. Live: the
//! motion data from smooth motion's plan through the aux target, as
//! statistics and a picture of the vectors.

use model2::capture::{self, Frame};
use model2::raster;
use model2::video::{self, View};
use std::path::Path;

fn write_png(path: &Path, w: usize, h: usize, rgb: &[u8]) -> Result<(), String> {
    std::fs::write(path, png::encode_rgb(w, h, rgb)).map_err(|e| format!("{}: {e}", path.display()))
}

fn default_view(view: Option<View>, windows: (&[usize], &[usize])) -> View {
    view.unwrap_or_else(|| View::new(1080, 4.0 / 3.0, video::Hud::Edges)).with_hud_windows(windows.0, windows.1)
}

/// Pixels that differ between two RGB pictures.
fn differ(a: &[u8], b: &[u8]) -> usize {
    a.chunks(3).zip(b.chunks(3)).filter(|(x, y)| x != y).count()
}

/// The captured frames in `root`: identity, jitter, stipple.
pub fn captures(
    root: &Path,
    windows: (&[usize], &[usize]),
    view: Option<View>,
    quality: gpu::Quality,
    out: &Path,
) -> Result<(), String> {
    let texture_rom = capture::load_texture_rom(root)
        .map_err(|e| format!("{}: {e} (capture with `make capture`)", root.display()))?;
    let mut gpu = gpu::Renderer::new()?;
    let view = default_view(view, windows);
    let (w, h) = (view.width, view.height);
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let keep_ok = view.sy >= 1.5;
    println!("gpu           {}", gpu.adapter());
    println!("view          {w}x{h} ({:.3} pixels a game pixel), depth {}", view.sy, quality.depth);
    println!("quality       temporal {}%, stipple {:?}", quality.temporal, quality.stipple);
    if !keep_ok {
        println!("note          under 1.5 pixels a game pixel Keep is Blend: identity fails on checkerboard pixels");
    }
    let mut dirs: Vec<_> = std::fs::read_dir(root)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("meta.txt").exists())
        .collect();
    dirs.sort();
    println!(
        "{:<8} {:>6} {:>10} {:>10} {:>10} {:>10} {:>9}",
        "frame", "polys", "identity", "off-after", "jittered", "blend-keep", "aux ok"
    );
    let (mut failed, mut pictures) = (0, 0);
    for d in &dirs {
        let f = Frame::load(d).map_err(|e| format!("{}: {e}", d.display()))?;
        let src = f.sources(&texture_rom);
        let mut polys = raster::build(&f.stream, &src);
        video::HudLayout::new(&f.tiles, &f.palette, &view).anchor(&mut polys, &view);
        let placed = raster::place(&mut polys, &view);
        let layers = video::layers_2d(&f.tiles, &f.palette, &view, !placed.is_empty());
        let off_q = gpu::Quality { depth: quality.depth, ..Default::default() };
        let keep_q = gpu::Quality { temporal: 100, stipple: gpu::Stipple::Keep, ..off_q };
        let blend_q = gpu::Quality { stipple: gpu::Stipple::Blend, ..keep_q };
        gpu.set_quality(off_q);
        let off = gpu.frame(&polys, &placed, &src, &view, &layers);
        // Each set_quality to another temporal setting starts afresh: the
        // first frame is unjittered.
        gpu.set_quality(keep_q);
        let keep = gpu.frame(&polys, &placed, &src, &view, &layers);
        gpu.set_quality(blend_q);
        let blend = gpu.frame(&polys, &placed, &src, &view, &layers);
        // The asked-for quality: its first frame, then repeats (jittered).
        gpu.set_quality(off_q);
        gpu.set_quality(quality);
        let first = gpu.frame(&polys, &placed, &src, &view, &layers);
        let mut jittered = Vec::new();
        for _ in 0..3 {
            gpu.set_temporal_frame(gpu::FrameTag::game(f.frame), gpu::Step::Repeat, Vec::new());
            jittered = gpu.frame(&polys, &placed, &src, &view, &layers);
        }
        let dbg = gpu.temporal_debug().ok_or("no temporal frame drawn")?;
        let aux_ok = dbg.aux.iter().flatten().all(|c| c.is_finite())
            // Repeats: zero motion (to float rounding), the same 1/z.
            && dbg.aux.iter().all(|a| a[0].abs() < 0.01 && a[1].abs() < 0.01 && (a[3] == 0.0 || a[3] == a[2].abs()));
        // And the off path again, after all that.
        gpu.set_quality(off_q);
        let after = gpu.frame(&polys, &placed, &src, &view, &layers);
        let (id, af, jd, bk) =
            (differ(&off, &keep), differ(&off, &after), differ(&first, &jittered), differ(&keep, &blend));
        println!("{:<8} {:>6} {id:>10} {af:>10} {jd:>10} {bk:>10} {aux_ok:>9}", f.frame, polys.len());
        let bad = (keep_ok && id != 0) || af != 0 || !aux_ok;
        failed += bad as usize;
        // Pictures: the first frames, those with a checkerboard, failures.
        if bad || pictures < 3 || (bk > 0 && pictures < 6) {
            pictures += 1;
            let name = |s: &str| out.join(format!("{}-{s}.png", f.frame));
            write_png(&name("off"), w, h, &off)?;
            write_png(&name("first"), w, h, &first)?;
            write_png(&name("jittered"), w, h, &jittered)?;
            write_png(&name("blend"), w, h, &blend)?;
            write_png(&name("keep"), w, h, &keep)?;
            // Where blend and keep differ, over a dimmed frame.
            let mark: Vec<u8> = keep
                .chunks(3)
                .zip(blend.chunks(3))
                .flat_map(|(k, b)| if k != b { [255, 0, 255] } else { [k[0] / 3, k[1] / 3, k[2] / 3] })
                .collect();
            write_png(&name("stipple"), w, h, &mark)?;
            if id != 0 {
                let mark: Vec<u8> = off
                    .chunks(3)
                    .zip(keep.chunks(3))
                    .flat_map(|(a, b)| if a != b { [255, 0, 0] } else { [a[0] / 3, a[1] / 3, a[2] / 3] })
                    .collect();
                write_png(&name("identity"), w, h, &mark)?;
            }
        }
    }
    println!("pictures      {}", out.display());
    if failed > 0 {
        return Err(format!("{failed} frames failed (identity, off path after temporal, or aux)"));
    }
    println!("ok            identity holds on {} frames", dirs.len());
    Ok(())
}

/// A picture of motion vectors: hue the direction, brightness the length
/// (full at 16 output pixels); grey where there is no motion data, black
/// where nothing is drawn; blended checkerboard pixels whitened.
fn motion_picture(aux: &[[f32; 4]]) -> Vec<u8> {
    aux.iter()
        .flat_map(|a| {
            if a[2] == 0.0 {
                return [0, 0, 0];
            }
            if a[3] == 0.0 {
                return [96, 96, 96];
            }
            let len = (a[0] * a[0] + a[1] * a[1]).sqrt();
            let v = 0.2 + 0.8 * (len / 16.0).min(1.0);
            let hue = (a[1].atan2(a[0]) / std::f32::consts::TAU + 1.0).fract() * 6.0;
            let x = 1.0 - (hue % 2.0 - 1.0).abs();
            let (r, g, b) = match hue as u32 {
                0 => (1.0, x, 0.0),
                1 => (x, 1.0, 0.0),
                2 => (0.0, 1.0, x),
                3 => (0.0, x, 1.0),
                4 => (x, 0.0, 1.0),
                _ => (1.0, 0.0, x),
            };
            let sheer = if a[2] < 0.0 { 0.5 } else { 0.0 };
            [r, g, b].map(|c: f32| ((c * v * (1.0 - sheer) + sheer) * 255.0).round() as u8)
        })
        .collect()
}

/// Run to `from` with the native geometrizer and smooth motion recording,
/// then draw `count` frames with temporal on, through `present_bus`.
#[allow(clippy::too_many_arguments)]
pub fn live(
    m: &mut model2::Machine,
    script: &model2::input::InputScript,
    from: u64,
    count: u64,
    windows: (&[usize], &[usize]),
    view: Option<View>,
    quality: gpu::Quality,
    out: &Path,
) -> Result<(), String> {
    m.bus.geo.set_mode(model2::geo::GeoMode::Native);
    m.bus.geo.set_smooth(true);
    let view = default_view(view, windows);
    let mut gpu = gpu::Renderer::new()?;
    let q = gpu.set_quality(quality);
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("gpu           {}", gpu.adapter());
    println!("view          {}x{}, temporal {}%, stipple {:?}", view.width, view.height, q.temporal, q.stipple);
    let advance = |m: &mut model2::Machine, to: u64| -> Result<(), String> {
        while m.frames < to {
            let last = m.frames;
            m.run_slice(u64::MAX).map_err(|s| format!("stopped at frame {}: {s:?}", m.frames))?;
            if m.frames != last {
                script.apply(m.frames, &mut m.bus);
            }
        }
        Ok(())
    };
    advance(m, from)?;
    println!(
        "{:<8} {:<16} {:>7} {:>7} {:>7} {:>8} {:>8} {:>9} {:>6}",
        "frame", "step", "drawn", "motion", "sheer", "median", "p99", "max", "finite"
    );
    for k in 0..count {
        advance(m, from + k + 1)?;
        let rgb = gpu.frame_bus(&mut m.bus, &view);
        let d = gpu.temporal_debug().ok_or("no temporal frame drawn")?;
        let px = d.aux.len().max(1) as f64;
        let drawn = d.aux.iter().filter(|a| a[2] != 0.0).count();
        let sheer = d.aux.iter().filter(|a| a[2] < 0.0).count();
        let mut lens: Vec<f32> =
            d.aux.iter().filter(|a| a[3] != 0.0).map(|a| (a[0] * a[0] + a[1] * a[1]).sqrt()).collect();
        lens.sort_by(f32::total_cmp);
        let at = |q: f64| lens.get(((lens.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(0.0);
        let finite = d.aux.iter().flatten().all(|c| c.is_finite());
        println!(
            "{:<8} {:<16} {:>6.1}% {:>6.1}% {:>6.1}% {:>8.2} {:>8.2} {:>9.1} {:>6}",
            m.frames,
            format!("{:?}", d.step),
            100.0 * drawn as f64 / px,
            100.0 * lens.len() as f64 / px,
            100.0 * sheer as f64 / px,
            at(0.5),
            at(0.99),
            at(1.0),
            finite
        );
        if !finite {
            return Err(format!("frame {}: the aux target has values that aren't finite", m.frames));
        }
        let name = |s: &str| out.join(format!("{}-{s}.png", m.frames));
        write_png(&name("colour"), view.width, view.height, &rgb)?;
        write_png(&name("mv"), d.width as usize, d.height as usize, &motion_picture(&d.aux))?;
    }
    println!("pictures      {}", out.display());
    Ok(())
}
