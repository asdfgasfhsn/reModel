//! SPIKE `remodel gpu-pick DIR OUT [FRAME,..]`: which object block drew each
//! 3D pixel, read back from the GPU's ID target (the look's ID pass, whose tag
//! word carries the block number +1 in bits 18+ with `Renderer::pick_blocks`),
//! against the software renderer's `visible` owner (what `model-pick` uses), at
//! native size and 1080p; and what the ID pass and the read-back cost.
use model2::capture;
use model2::raster;
use model2::video::{self, View};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

pub fn run(root: &Path, windows: (&[usize], &[usize]), out: &Path, only: &[u64]) -> Result<(), String> {
    let rom = capture::load_texture_rom(root).map_err(|e| format!("{}: {e}", root.display()))?;
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let mut gpu = gpu::Renderer::new()?;
    println!("gpu {}", gpu.adapter());
    let look = Arc::new(model2::look::Look::comic());
    let views = [("native", View::native()), ("1080p", View::new(1080, 16.0 / 9.0, video::Hud::Edges))];
    for item in capture::frames(root).map_err(|e| e.to_string())? {
        let (_, f) = item.map_err(|e| e.to_string())?;
        if !only.is_empty() && !only.contains(&f.frame) {
            continue;
        }
        let Some(side) = f.light_frame() else { continue };
        let src = f.sources(&rom);
        for (name, view) in &views {
            let view = view.with_hud_windows(windows.0, windows.1);
            let (w, h) = (view.width, view.height);
            let mut polys = raster::build_shaded(&f.stream, &src, None);
            video::HudLayout::new(&f.tiles, &f.palette, &view).anchor(&mut polys, &view);
            let placed = raster::place(&mut polys, &view);
            if placed.is_empty() {
                continue;
            }
            let vis = raster::visible(&polys, &placed, &src, &view);
            let layers = video::layers_2d(&f.tiles, &f.palette, &view, true);
            gpu.set_quality(crate::gpu_light(None));
            let time = |gpu: &mut gpu::Renderer, look: Option<Arc<model2::look::Look>>, pick: bool, n: u32| {
                gpu.set_look(look);
                gpu.pick_blocks = pick;
                gpu.forget_ram();
                let _ = gpu.frame(&polys, &placed, &src, &view, &layers);
                let t = Instant::now();
                for _ in 0..n {
                    gpu.forget_ram();
                    let _ = gpu.frame(&polys, &placed, &src, &view, &layers);
                }
                t.elapsed().as_secs_f64() * 1e3 / n as f64
            };
            let t_plain = time(&mut gpu, None, false, 5);
            let t_look = time(&mut gpu, Some(look.clone()), false, 5);
            let t_pick = time(&mut gpu, Some(look.clone()), true, 5);
            let t = Instant::now();
            let ids = gpu.read_ids(w as u32, h as u32);
            let t_read = t.elapsed().as_secs_f64() * 1e3;
            let (mut n3d, mut both, mut same_block, mut same_key, mut soft_only, mut gpu_only, mut gpu_none) =
                (0u64, 0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
            let mut shown = 0;
            let mut grid = 0u64;
            let blocks = side.blocks.len();
            for y in (0..h).step_by(4) {
                for x in (0..w).step_by(4) {
                    let o = y * w + x;
                    let sk = vis.owner[o];
                    let sb = (sk != u32::MAX).then(|| polys[placed[sk as usize].poly].block as usize);
                    let tag = ids[o][1] >> 18;
                    let gb = (tag != 0).then(|| tag as usize - 1);
                    grid += 1;
                    if sb.is_none() && gb.is_none() {
                        continue;
                    }
                    n3d += 1;
                    let key = |b: usize| side.blocks.get(b).map_or(0, |bl| bl.key);
                    match (sb, gb) {
                        (Some(a), Some(b)) => {
                            both += 1;
                            same_block += (a == b) as u64;
                            same_key += (key(a) == key(b)) as u64;
                            if key(a) != key(b) && shown < 3 {
                                shown += 1;
                                println!("    differ at {x},{y}: software block {a} (oba {:06x}), gpu block {b} (oba {:06x}) of {blocks}", side.blocks[a].oba & 0xff_ffff, side.blocks.get(b).map_or(0, |bl| bl.oba & 0xff_ffff));
                            }
                        }
                        (Some(_), None) => gpu_none += 1,
                        (None, Some(_)) => gpu_only += 1,
                        _ => {}
                    }
                    soft_only += sb.is_some() as u64;
                }
            }
            let _ = soft_only;
            println!(
                "frame {:>5} {name:>6} {w}x{h}: grid {grid}, 3D pixels (either) {n3d}, both {both}: same block {:.2}% same key {:.2}%; software only {gpu_none}, gpu only {gpu_only}; agreement over either {:.2}% ; ms/frame (incl. RGB read-back): plain {t_plain:.1} look {t_look:.1} look+block tag {t_pick:.1}; ID read-back {t_read:.1} ms",
                f.frame,
                100.0 * same_block as f64 / both.max(1) as f64,
                100.0 * same_key as f64 / both.max(1) as f64,
                100.0 * same_key as f64 / n3d.max(1) as f64,
            );
        }
    }
    let _ = out;
    Ok(())
}
