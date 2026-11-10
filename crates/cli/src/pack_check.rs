//! `remodel pack-check`: draw captured frames with a texture pack on the
//! GPU with each filtering of its images (off, mipmaps, 16x anisotropic),
//! and in software with trilinear mipmaps and as a supersampled reference
//! (what an ideal filter shows); write them all as pictures and measure how
//! far each is from the reference where the pack changes the picture. With
//! `--identity`, check instead that a pack standing for the original
//! textures (a dump, or each frame's own) draws the original frames.

use crate::texpack::Packs;
use gpu::TexFilter;
use model2::capture::{self, Frame};
use model2::enhance::{Enhance, Found, Rgba, TexInfo, TexturePack, TileInfo};
use model2::raster;
use model2::video::{self, View};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

/// The GPU filterings compared, with their file names.
const FILTERS: [(TexFilter, &str); 3] =
    [(TexFilter::Off, "off"), (TexFilter::Mip, "mip"), (TexFilter::Aniso(16), "aniso16")];

/// Mean absolute difference per channel (0-255) over the pixels in `mask`,
/// and the share of them off by more than 16 in a channel.
fn compare(a: &[u8], b: &[u8], mask: &[bool]) -> (f64, f64) {
    let (mut sum, mut large, mut n) = (0u64, 0usize, 0usize);
    for (i, _) in mask.iter().enumerate().filter(|(_, m)| **m) {
        let d = [0, 1, 2].map(|c| a[i * 3 + c].abs_diff(b[i * 3 + c]));
        sum += d.iter().map(|&x| x as u64).sum::<u64>();
        large += d.iter().any(|&x| x > 16) as usize;
        n += 1;
    }
    (sum as f64 / (3 * n).max(1) as f64, 100.0 * large as f64 / n.max(1) as f64)
}

/// Mean time of `n` runs of `f`, in ms.
fn time(n: usize, mut f: impl FnMut()) -> f64 {
    let t = Instant::now();
    for _ in 0..n {
        f();
    }
    t.elapsed().as_secs_f64() * 1e3 / n as f64
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    root: &Path,
    hud_windows: (&[usize], &[usize]),
    view: Option<View>,
    aa: u32,
    depth: bool,
    pack: &Path,
    out_dir: &Path,
    light: Option<&model2::light::Options>,
) -> Result<(), String> {
    let texture_rom = capture::load_texture_rom(root)
        .map_err(|e| format!("{}: {e} (capture with `make capture`)", root.display()))?;
    let mut gpu = gpu::Renderer::new()?;
    let mut plain = gpu::Renderer::new()?;
    let mut view = view.unwrap_or_else(|| View::new(1080, 16.0 / 9.0, video::Hud::Edges));
    view = view.with_hud_windows(hud_windows.0, hud_windows.1);
    // The reference supersamples at least 4x4.
    let ss = super::supersample_factor(aa.max(16));
    let q = gpu::Quality { msaa: aa, depth, tex_filter: TexFilter::Off, ..super::gpu_light(light) };
    let msaa = plain.set_quality(q).msaa;
    gpu.set_pack(Some(Box::new(Packs::new(None, Some(pack.to_path_buf()))?)));
    let mut sw_pack = Packs::new(None, Some(pack.to_path_buf()))?;
    println!("gpu           {}", gpu.adapter());
    println!(
        "view          {}x{}, GPU {msaa} samples a pixel, depth buffer {}; reference: software, pack bilinear, {ss}x{ss} supersampled",
        view.width,
        view.height,
        if depth { "on" } else { "off" }
    );
    println!("pack          {}", pack.display());
    let mut dirs: Vec<_> = std::fs::read_dir(root)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("meta.txt").exists())
        .collect();
    dirs.sort();
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    println!("mean absolute difference from the reference (0-255 a channel) where the pack changes the frame; GPU ms a whole frame");
    println!(
        "{:<8} {:>7}  {:>7} {:>7} {:>7}  {:>14}  {:>7} {:>7} {:>7}  {:>8} {:>8}",
        "frame", "changed", "off", "mip", "aniso16", "gpu-sw mip >16", "off ms", "mip ms", "16 ms", "ref ms", "MB mips"
    );
    let mut totals = [0.0f64; 7];
    let mut n = 0;
    for d in &dirs {
        let f = Frame::load(d).map_err(|e| format!("{}: {e}", d.display()))?;
        let src = f.sources(&texture_rom);
        let mut polys = raster::build_shaded(&f.stream, &src, light);
        if polys.is_empty() {
            continue;
        }
        video::HudLayout::new(&f.tiles, &f.palette, &view).anchor(&mut polys, &view);
        let sw_polys = polys.clone();
        let placed = raster::place(&mut polys, &view);
        let layers = video::layers_2d(&f.tiles, &f.palette, &view, !placed.is_empty());
        let name = |s: &str| out_dir.join(format!("{:05}-{s}.png", f.frame));
        let save = |s: &str, rgb: &[u8]| {
            let _ = std::fs::write(name(s), png::encode_rgb(view.width, view.height, rgb));
        };
        // The pixels the pack changes: GPU with the pack (unfiltered)
        // against without.
        let original = plain.frame(&polys, &placed, &src, &view, &layers);
        let mut frames = Vec::new();
        let mut ms = Vec::new();
        let mut mb = 0.0;
        for (filter, label) in FILTERS {
            gpu.set_quality(gpu::Quality { tex_filter: filter, ..q });
            let rgb = gpu.frame(&polys, &placed, &src, &view, &layers);
            ms.push(time(3, || {
                gpu.frame(&polys, &placed, &src, &view, &layers);
            }));
            if filter == TexFilter::Mip {
                mb = gpu.pack_stats().bytes as f64 / (1 << 20) as f64;
            }
            save(label, &rgb);
            frames.push(rgb);
        }
        let mask: Vec<bool> =
            original.as_chunks::<3>().0.iter().zip(frames[0].as_chunks::<3>().0).map(|(a, b)| a != b).collect();
        let changed = 100.0 * mask.iter().filter(|m| **m).count() as f64 / mask.len() as f64;
        // Software: trilinear at the output size, and the reference.
        let mut p = sw_polys.clone();
        let layer = raster::draw_enhanced_filtered(&mut p, &src, &view, &mut sw_pack, depth, TexFilter::Mip);
        let sw_mip = video::compose(&f.tiles, &f.palette, Some((&layer.0, &layer.1)), &view);
        save("swmip", &sw_mip);
        let t = Instant::now();
        let big = view.supersampled(ss);
        let mut p = sw_polys.clone();
        let layer = raster::draw_enhanced_depth(&mut p, &src, &big, &mut sw_pack, depth);
        let big_frame = video::compose(&f.tiles, &f.palette, Some((&layer.0, &layer.1)), &big);
        drop(layer);
        let reference = video::downsample(&big_frame, big.width, big.height, ss);
        drop(big_frame);
        let ref_ms = t.elapsed().as_secs_f64() * 1e3;
        save("ref", &reference);
        let diffs: Vec<f64> = frames.iter().map(|g| compare(g, &reference, &mask).0).collect();
        let (_, gpu_sw) = compare(&frames[1], &sw_mip, &mask);
        println!(
            "{:<8} {:>6.1}%  {:>7.2} {:>7.2} {:>7.2}  {:>13.3}%  {:>7.1} {:>7.1} {:>7.1}  {:>8.0} {:>8.1}",
            f.frame, changed, diffs[0], diffs[1], diffs[2], gpu_sw, ms[0], ms[1], ms[2], ref_ms, mb
        );
        for (t, v) in totals.iter_mut().zip([diffs[0], diffs[1], diffs[2], gpu_sw, ms[0], ms[1], ms[2]]) {
            *t += v;
        }
        n += 1;
    }
    let m = |v: f64| v / n.max(1) as f64;
    println!(
        "mean     {:>7}  {:>7.2} {:>7.2} {:>7.2}  {:>13.3}%  {:>7.1} {:>7.1} {:>7.1}",
        "",
        m(totals[0]),
        m(totals[1]),
        m(totals[2]),
        m(totals[3]),
        m(totals[4]),
        m(totals[5]),
        m(totals[6])
    );
    println!(
        "pictures      {}/FRAME-{{off,mip,aniso16}}.png (GPU), -swmip.png (software trilinear), -ref.png (reference)",
        out_dir.display()
    );
    Ok(())
}

/// `remodel tile-check`: draw captured frames with a texture pack's tile
/// images live on the GPU (`gpu::Renderer::layers_2d`) and compare them
/// with the software snapshots' tile replacement (`compose_enhanced`) over
/// the same 3D layer; write both, and the frame without the pack, as
/// pictures.
pub fn run_tiles(
    root: &Path,
    hud_windows: (&[usize], &[usize]),
    view: Option<View>,
    pack: &Path,
    out_dir: &Path,
    light: Option<&model2::light::Options>,
) -> Result<(), String> {
    let texture_rom = capture::load_texture_rom(root)
        .map_err(|e| format!("{}: {e} (capture with `make capture`)", root.display()))?;
    let mut gpu = gpu::Renderer::new()?;
    let mut plain = gpu::Renderer::new()?;
    let mut view = view.unwrap_or_else(|| View::new(1080, 16.0 / 9.0, video::Hud::Edges));
    view = view.with_hud_windows(hud_windows.0, hud_windows.1);
    let q = super::gpu_light(light);
    gpu.set_quality(q);
    plain.set_quality(q);
    gpu.set_pack(Some(Box::new(Packs::new(None, Some(pack.to_path_buf()))?)));
    let mut sw_pack = Packs::new(None, Some(pack.to_path_buf()))?;
    println!("gpu           {}", gpu.adapter());
    println!("view          {}x{}", view.width, view.height);
    println!("pack          {}", pack.display());
    let mut dirs: Vec<_> = std::fs::read_dir(root)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("meta.txt").exists())
        .collect();
    dirs.sort();
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    println!(
        "tiles: patterns replaced; changed: pixels the pack changes; gpu-sw: pixels where the GPU's tiles differ \
         from the software's (any, > 16); 2d ms: the layers on the CPU, plain and with tiles; frame ms: a whole \
         GPU frame with readback, the same 3D, plain and with tiles"
    );
    println!(
        "{:<8} {:>6} {:>8}  {:>9} {:>9}  {:>8} {:>8}  {:>8} {:>8}",
        "frame", "tiles", "changed", "gpu-sw", ">16", "2d ms", "tiles ms", "frame ms", "tiles ms"
    );
    let (mut worst, mut n) = (0usize, 0);
    for d in &dirs {
        let f = Frame::load(d).map_err(|e| format!("{}: {e}", d.display()))?;
        let src = f.sources(&texture_rom);
        let mut polys = raster::build_shaded(&f.stream, &src, light);
        video::HudLayout::new(&f.tiles, &f.palette, &view).anchor(&mut polys, &view);
        let placed = raster::place(&mut polys, &view);
        let has_3d = !placed.is_empty();
        let name = |s: &str| out_dir.join(format!("{:05}-{s}.png", f.frame));
        let save = |s: &str, rgb: &[u8]| {
            let _ = std::fs::write(name(s), png::encode_rgb(view.width, view.height, rgb));
        };
        let t = Instant::now();
        let layers = video::layers_2d(&f.tiles, &f.palette, &view, has_3d);
        let plain_ms = t.elapsed().as_secs_f64() * 1e3;
        let original = plain.frame(&polys, &placed, &src, &view, &layers);
        // Twice: the first looks the patterns up and fills the atlas.
        gpu.layers_2d(&f.tiles, &f.palette, &view, has_3d);
        let t = Instant::now();
        let tiled = gpu.layers_2d(&f.tiles, &f.palette, &view, has_3d);
        let tiles_ms = t.elapsed().as_secs_f64() * 1e3;
        let frame = gpu.frame(&polys, &placed, &src, &view, &tiled);
        let replaced = tiled.tiles.as_ref().map_or(0, |m| m.replaced);
        let frame_ms = [&layers, &tiled].map(|l| {
            time(5, || {
                gpu.frame(&polys, &placed, &src, &view, l);
            })
        });
        // The software's tiles over the GPU's own 3D layer.
        let layer = gpu.draw(&polys, &placed, &src, &view);
        let sw = video::compose_enhanced(&f.tiles, &f.palette, Some((&layer.0, &layer.1)), &view, &mut sw_pack);
        save("orig", &original);
        save("gpu", &frame);
        save("sw", &sw);
        let px = |a: &[u8], b: &[u8], min: u8| {
            let (a, b) = (a.as_chunks::<3>().0, b.as_chunks::<3>().0);
            a.iter().zip(b).filter(|(p, q)| (0..3).any(|c| p[c].abs_diff(q[c]) > min)).count()
        };
        let total = (view.width * view.height) as f64;
        let changed = px(&original, &frame, 0);
        let (any, large) = (px(&frame, &sw, 0), px(&frame, &sw, 16));
        worst = worst.max(large);
        println!(
            "{:<8} {:>6} {:>7.2}%  {:>8.3}% {:>8.3}%  {:>8.2} {:>8.2}  {:>8.2} {:>8.2}",
            f.frame,
            replaced,
            100.0 * changed as f64 / total,
            100.0 * any as f64 / total,
            100.0 * large as f64 / total,
            plain_ms,
            tiles_ms,
            frame_ms[0],
            frame_ms[1]
        );
        n += 1;
    }
    println!("pictures      {}/FRAME-{{orig,gpu,sw}}.png ({n} frames)", out_dir.display());
    if worst > 0 {
        println!("note          some pixels differ by more than 16 between the GPU's tiles and the software's");
    }
    Ok(())
}

/// A pack of each captured frame's own textures, as `--dump-textures` would
/// dump them from that frame (`raster::texture_image`): the identity check
/// without a dump.
#[derive(Default)]
struct FramePack {
    images: HashMap<u64, Arc<Rgba>>,
    mips: HashMap<u64, Arc<Vec<Rgba>>>,
}

impl FramePack {
    fn new(polys: &[raster::Poly], src: &raster::Sources) -> Self {
        let mut keys = raster::TexKeys::default();
        let mut images = HashMap::new();
        for p in polys {
            let Some(t) = &p.texture else { continue };
            let Some(info) = raster::tex_info(p, t, src, &mut keys) else { continue };
            images.entry(info.key).or_insert_with(|| Arc::new(raster::texture_image(src, &info)));
        }
        FramePack { images, mips: HashMap::new() }
    }
}

impl Enhance for FramePack {
    fn texture(&mut self, info: &TexInfo) -> Option<Arc<Rgba>> {
        self.images.get(&info.key).cloned()
    }
    fn texture_mips(&mut self, info: &TexInfo) -> Option<Arc<Vec<Rgba>>> {
        let img = self.texture(info)?;
        Some(self.mips.entry(info.key).or_insert_with(|| Arc::new(img.mip_chain())).clone())
    }
    fn texture_drawn(&mut self, _: &TexInfo, _: &raster::Sources, _: usize) {}
    fn tile(&mut self, _: &TileInfo) -> Option<Arc<Rgba>> {
        None
    }
}

impl TexturePack for FramePack {
    fn find(&mut self, info: &TexInfo) -> Found {
        match self.images.get(&info.key) {
            Some(img) => Found::Image(img.clone()),
            None => Found::Missing,
        }
    }
}

/// Share of a frame's pixels (percent) off by more than 16 levels in a
/// channel that an identity pack may differ from the original by, drawn
/// unfiltered: the pack is sampled bilinearly in colour where the original
/// filters texel indices and looks the result up (the luma and translation
/// tables are not linear), and rounds. Measured 2026-11-11 with each
/// frame's own textures: 0.30% (srallyc), 0.50% (topskatr), 0.71% (sgt24h)
/// on average. Mipmapped drawing is a different filter (distant textures
/// average), so it is reported but not held to this.
pub const IDENTITY_TOLERANCE: f64 = 1.0;

/// `pack-check --identity`: draw the captured frames with a pack that
/// stands for the original textures (`pack`, normally a dump made with
/// `--dump-textures`, or with None each frame's own textures) and compare
/// them with the frames drawn without one, on each renderer: GPU unfiltered
/// and mipmapped, software exact and trilinear. A pack made of the dump's
/// own images must draw the original frame; Err if, in any frame, either
/// unfiltered renderer's share of pixels off by more than 16 levels exceeds
/// `IDENTITY_TOLERANCE` (frame by frame, not the mean: topskatr and sgt24h
/// fail this on single frames while their means are within it, a known
/// open item, docs/texture-packs.md).
pub fn identity(
    root: &Path,
    hud_windows: (&[usize], &[usize]),
    view: Option<View>,
    aa: u32,
    depth: bool,
    pack: Option<&Path>,
    out_dir: &Path,
) -> Result<(), String> {
    let texture_rom = capture::load_texture_rom(root)
        .map_err(|e| format!("{}: {e} (capture with `make capture`)", root.display()))?;
    let mut gpu = gpu::Renderer::new()?;
    let mut plain = gpu::Renderer::new()?;
    let mut view = view.unwrap_or_else(View::native);
    view = view.with_hud_windows(hud_windows.0, hud_windows.1);
    let q = gpu::Quality { msaa: aa, depth, tex_filter: TexFilter::Off, ..Default::default() };
    plain.set_quality(q);
    let mut dump_pack = match pack {
        Some(p) => {
            gpu.set_pack(Some(Box::new(Packs::new(None, Some(p.to_path_buf()))?)));
            Some(Packs::new(None, Some(p.to_path_buf()))?)
        }
        None => None,
    };
    println!("gpu           {}", gpu.adapter());
    println!("view          {}x{}, depth buffer {}", view.width, view.height, if depth { "on" } else { "off" });
    println!("pack          {}", pack.map_or("each frame's own textures".into(), |p| p.display().to_string()));
    let mut dirs: Vec<_> = std::fs::read_dir(root)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("meta.txt").exists())
        .collect();
    dirs.sort();
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    println!("identity: the frame with the pack against without, per renderer: mean absolute difference (0-255 a");
    println!("channel) over the frame / share of its pixels off by more than 16 levels");
    println!("{:<8} {:>16} {:>16} {:>16} {:>16}", "frame", "gpu off", "gpu mip", "sw exact", "sw mip");
    let mut totals = [0.0f64; 8];
    let mut n = 0;
    let mut over = Vec::new();
    for d in &dirs {
        let f = Frame::load(d).map_err(|e| format!("{}: {e}", d.display()))?;
        let src = f.sources(&texture_rom);
        let mut polys = raster::build(&f.stream, &src);
        if polys.is_empty() {
            continue;
        }
        video::HudLayout::new(&f.tiles, &f.palette, &view).anchor(&mut polys, &view);
        let sw_polys = polys.clone();
        let mut frame_pack = FramePack::new(&sw_polys, &src);
        if dump_pack.is_none() {
            gpu.set_pack(Some(Box::new(FramePack::new(&sw_polys, &src))));
        }
        let placed = raster::place(&mut polys, &view);
        let layers = video::layers_2d(&f.tiles, &f.palette, &view, !placed.is_empty());
        let save = |s: &str, rgb: &[u8]| {
            let path = out_dir.join(format!("{:05}-{s}.png", f.frame));
            let _ = std::fs::write(path, png::encode_rgb(view.width, view.height, rgb));
        };
        let gpu_orig = plain.frame(&polys, &placed, &src, &view, &layers);
        save("orig", &gpu_orig);
        let mut results = Vec::new();
        for (filter, label) in [(TexFilter::Off, "off"), (TexFilter::Mip, "mip")] {
            gpu.set_quality(gpu::Quality { tex_filter: filter, ..q });
            let rgb = gpu.frame(&polys, &placed, &src, &view, &layers);
            save(label, &rgb);
            results.push(whole(&rgb, &gpu_orig));
        }
        let sw = |hooks: Option<&mut dyn Enhance>, filter| {
            let mut p = sw_polys.clone();
            let layer = match hooks {
                Some(h) => raster::draw_enhanced_filtered(&mut p, &src, &view, h, depth, filter),
                None => raster::draw_view_depth(&mut p, &src, &view, depth),
            };
            video::compose(&f.tiles, &f.palette, Some((&layer.0, &layer.1)), &view)
        };
        let sw_orig = sw(None, TexFilter::Off);
        for filter in [TexFilter::Off, TexFilter::Mip] {
            let hooks: &mut dyn Enhance = match &mut dump_pack {
                Some(p) => p,
                None => &mut frame_pack,
            };
            let rgb = sw(Some(hooks), filter);
            if filter == TexFilter::Off {
                save("sw", &rgb);
            }
            results.push(whole(&rgb, &sw_orig));
        }
        let cell = |(m, l): (f64, f64)| format!("{m:.2} / {l:.3}%");
        println!(
            "{:<8} {:>16} {:>16} {:>16} {:>16}",
            f.frame,
            cell(results[0]),
            cell(results[1]),
            cell(results[2]),
            cell(results[3])
        );
        if results[0].1.max(results[2].1) > IDENTITY_TOLERANCE {
            over.push(f.frame.to_string());
        }
        for (k, (m, l)) in results.into_iter().enumerate() {
            totals[2 * k] += m;
            totals[2 * k + 1] += l;
        }
        n += 1;
    }
    let m: Vec<f64> = totals.iter().map(|t| t / n.max(1) as f64).collect();
    let cell = |k: usize| format!("{:.2} / {:.3}%", m[2 * k], m[2 * k + 1]);
    println!("{:<8} {:>16} {:>16} {:>16} {:>16}", "mean", cell(0), cell(1), cell(2), cell(3));
    println!(
        "pictures      {}/FRAME-orig.png (GPU, no pack), -off.png, -mip.png (GPU, pack), -sw.png (software, pack)",
        out_dir.display()
    );
    // Unfiltered (GPU off and software exact), frame by frame.
    if !over.is_empty() {
        return Err(format!(
            "identity pack: {} of {n} frames ({}) have more than {IDENTITY_TOLERANCE}% of pixels off by more than \
             16 levels unfiltered: the pack does not stand for the textures as these frames draw them",
            over.len(),
            over.join(", ")
        ));
    }
    println!("identity      ok (unfiltered: tolerance {IDENTITY_TOLERANCE}% of pixels off by more than 16 levels)");
    Ok(())
}

/// `compare` over the whole frame.
fn whole(a: &[u8], b: &[u8]) -> (f64, f64) {
    compare(a, b, &vec![true; a.len() / 3])
}
