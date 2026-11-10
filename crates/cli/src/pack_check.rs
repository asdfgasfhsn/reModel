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

/// Per-frame limits of `pack-check --identity`, as shares of the frame's
/// pixels (percent) off by more than 16 levels in a channel, drawn
/// unfiltered. An identity pack is not pixel-identical to the original: the
/// pack is sampled bilinearly in colour where the original filters texel
/// values and looks the result up (the luma and translation tables are not
/// linear), so the two differ between texels of different values (*filter
/// order*, `raster::IdentitySplit`, docs/texture-packs.md). Measured
/// 2026-11-11 at native size with each frame's own textures: 0.30%
/// (srallyc), 0.50% (topskatr), 0.71% (sgt24h) on average and up to 2.6% on
/// frames with crowds drawn small, all of it filter order and cut-out
/// edges; the classes that mean a fault, one texel value under the pixel
/// (*same*) and another polygon on top (*other*), were 0.000% and at most
/// 0.001% on every frame. Mipmapped drawing is a different filter (distant
/// textures average): reported, not held to these.
///
/// The limit on *same* + *other* in the software's split: what a wrong
/// look, a wrong key or lighting the pack can't express produces. The one
/// such gap found (a skater's collar, topskatr, since fixed) was 0.004% of
/// frames 2100 and 2400 and 0.007% of 2700 (7-13 pixels at 496x384), and
/// 0.003% of 5100 (with the fix reverted, 2026-11-11); 0.005% would catch it
/// on 2700 alone. This is three times the most measured without a fault
/// (0.001%, 2 pixels), and at 496x384 allows 5 pixels.
pub const IDENTITY_FAULT_LIMIT: f64 = 0.003;
/// A sanity ceiling on each unfiltered renderer's whole share and on the
/// split's filter order (texels differ + cut-out), which are otherwise only
/// reported: about twice the worst frame measured (2.6%, topskatr 3600), so
/// a pack that looks wrong across much of a frame fails even where the
/// split would put it down to filter order.
pub const IDENTITY_CEILING: f64 = 5.0;
/// How far the GPU's unfiltered share may be from the software's in a
/// frame (percentage points): the split is the software's, so the GPU is
/// held to agreeing with it. Measured at most 0.025 (sgt24h 2400).
pub const IDENTITY_GPU_SW: f64 = 0.1;
/// With the depth buffer the split isn't computed (`raster::identity_split`
/// draws without it), so no pixel can be put down to filter order: each
/// unfiltered renderer's whole share is held to this instead, which frames
/// with crowds drawn small fail by filter order alone (strict, not silent).
pub const IDENTITY_TOLERANCE: f64 = 1.0;

/// One frame's unfiltered shares (percent of the frame off by more than 16
/// levels): GPU, software, and the software's split by cause (texels
/// differ, cut-out, same, other) when it was computed.
#[derive(Clone, Copy, Debug)]
struct IdentityShares {
    gpu_off: f64,
    sw_exact: f64,
    split: Option<[f64; 4]>,
}

/// Why a frame fails the identity limits (empty: it passes).
fn identity_faults(s: &IdentityShares) -> Vec<String> {
    let mut why = Vec::new();
    let worst = s.gpu_off.max(s.sw_exact);
    let Some([texels, cut_out, same, other]) = s.split else {
        if worst > IDENTITY_TOLERANCE {
            why.push(format!("{worst:.3}% off, no split (depth buffer): over {IDENTITY_TOLERANCE}%"));
        }
        return why;
    };
    if same + other > IDENTITY_FAULT_LIMIT {
        why.push(format!("same + other {:.4}%: over {IDENTITY_FAULT_LIMIT}%", same + other));
    }
    if texels + cut_out > IDENTITY_CEILING {
        why.push(format!("filter order {:.3}%: over the {IDENTITY_CEILING}% ceiling", texels + cut_out));
    }
    if worst > IDENTITY_CEILING {
        why.push(format!("{worst:.3}% off: over the {IDENTITY_CEILING}% ceiling"));
    }
    if (s.gpu_off - s.sw_exact).abs() > IDENTITY_GPU_SW {
        why.push(format!("GPU {:.3}%, software {:.3}%: more than {IDENTITY_GPU_SW} apart", s.gpu_off, s.sw_exact));
    }
    why
}

/// `pack-check --identity`: draw the captured frames with a pack that
/// stands for the original textures (`pack`, normally a dump made with
/// `--dump-textures`, or with None each frame's own textures) and compare
/// them with the frames drawn without one, on each renderer: GPU unfiltered
/// and mipmapped, software exact and trilinear. A pack made of the dump's
/// own images must draw the original frame. Each frame's line splits the
/// software's differing pixels by cause (`raster::identity_split`, without
/// the depth buffer); Err if any frame fails `identity_faults` (frame by
/// frame, not the mean): the fault classes over `IDENTITY_FAULT_LIMIT`,
/// filter order or a renderer's whole share over `IDENTITY_CEILING`, the
/// GPU more than `IDENTITY_GPU_SW` from the software, or, with the depth
/// buffer (no split), a renderer's whole share over `IDENTITY_TOLERANCE`.
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
    println!("channel) over the frame / share of its pixels off by more than 16 levels; sw exact's 3D pixels off by");
    println!("more than 16 by cause, share of the frame: texels differ under the filter (filter order), a cut-out");
    println!("texel among them, one texel value (a gap), another polygon on top (no depth buffer only)");
    println!(
        "{:<8} {:>16} {:>16} {:>16} {:>16}  {:>28}",
        "frame", "gpu off", "gpu mip", "sw exact", "sw mip", "texels/cut-out/same/other"
    );
    let mut totals = [0.0f64; 8];
    let mut split_totals = [0.0f64; 4];
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
        // Why the software's pixels differ (without the depth buffer only).
        let mut split = None;
        let causes = if depth {
            "-".to_string()
        } else {
            let hooks: &mut dyn Enhance = match &mut dump_pack {
                Some(p) => p,
                None => &mut frame_pack,
            };
            let s = raster::identity_split(&mut sw_polys.clone(), &src, &view, hooks);
            let px = (view.width * view.height) as f64 / 100.0;
            let share = [s.texels_differ, s.cut_out, s.same_texel, s.other_polygon].map(|c| c as f64 / px);
            for (t, v) in split_totals.iter_mut().zip(share) {
                *t += v;
            }
            split = Some(share);
            format!("{:.3} {:.3} {:.3} {:.3}%", share[0], share[1], share[2], share[3])
        };
        let cell = |(m, l): (f64, f64)| format!("{m:.2} / {l:.3}%");
        println!(
            "{:<8} {:>16} {:>16} {:>16} {:>16}  {causes:>28}",
            f.frame,
            cell(results[0]),
            cell(results[1]),
            cell(results[2]),
            cell(results[3])
        );
        let why = identity_faults(&IdentityShares { gpu_off: results[0].1, sw_exact: results[2].1, split });
        if !why.is_empty() {
            over.push(format!("{} ({})", f.frame, why.join("; ")));
        }
        for (k, (m, l)) in results.into_iter().enumerate() {
            totals[2 * k] += m;
            totals[2 * k + 1] += l;
        }
        n += 1;
    }
    let m: Vec<f64> = totals.iter().map(|t| t / n.max(1) as f64).collect();
    let cell = |k: usize| format!("{:.2} / {:.3}%", m[2 * k], m[2 * k + 1]);
    let s = split_totals.map(|t| t / n.max(1) as f64);
    let causes = if depth { "-".to_string() } else { format!("{:.3} {:.3} {:.3} {:.3}%", s[0], s[1], s[2], s[3]) };
    println!("{:<8} {:>16} {:>16} {:>16} {:>16}  {causes:>28}", "mean", cell(0), cell(1), cell(2), cell(3));
    println!(
        "pictures      {}/FRAME-orig.png (GPU, no pack), -off.png, -mip.png (GPU, pack), -sw.png (software, pack)",
        out_dir.display()
    );
    // Unfiltered (GPU off and software exact), frame by frame.
    if !over.is_empty() {
        return Err(format!(
            "identity pack: {} of {n} frames fail unfiltered, the pack does not stand for the textures as these \
             frames draw them: {}",
            over.len(),
            over.join(", ")
        ));
    }
    if depth {
        println!(
            "identity      ok (unfiltered, depth buffer, no split: at most {IDENTITY_TOLERANCE}% of pixels off by more \
             than 16 levels)"
        );
    } else {
        println!(
            "identity      ok (unfiltered: same + other at most {IDENTITY_FAULT_LIMIT}%, filter order and whole frame \
             at most {IDENTITY_CEILING}%, GPU within {IDENTITY_GPU_SW} of software)"
        );
    }
    Ok(())
}

/// `compare` over the whole frame.
fn whole(a: &[u8], b: &[u8]) -> (f64, f64) {
    compare(a, b, &vec![true; a.len() / 3])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shares(gpu_off: f64, sw_exact: f64, split: Option<[f64; 4]>) -> IdentityShares {
        IdentityShares { gpu_off, sw_exact, split }
    }

    #[test]
    fn identity_passes_filter_order() {
        // topskatr 3600 and sgt24h 2400, each frame's own textures: over
        // 1% by filter order alone.
        assert!(identity_faults(&shares(2.560, 2.556, Some([2.574, 0.001, 0.0, 0.0]))).is_empty());
        assert!(identity_faults(&shares(2.040, 2.065, Some([2.301, 0.007, 0.0, 0.0]))).is_empty());
        // 0.001% another polygon (topskatr 6212).
        assert!(identity_faults(&shares(0.327, 0.326, Some([0.371, 0.002, 0.0, 0.001]))).is_empty());
    }

    #[test]
    fn identity_fails_gaps() {
        // The collar before its fix: 0.007% and 0.004% one texel value
        // (topskatr 2700, 2400).
        assert_eq!(identity_faults(&shares(0.108, 0.104, Some([0.089, 0.008, 0.007, 0.0]))).len(), 1);
        assert_eq!(identity_faults(&shares(0.076, 0.076, Some([0.070, 0.002, 0.004, 0.0]))).len(), 1);
        // Cut-out edges decided differently.
        assert_eq!(identity_faults(&shares(0.2, 0.2, Some([0.1, 0.0, 0.0, 0.01]))).len(), 1);
    }

    #[test]
    fn identity_fails_a_wrong_look() {
        // A texture drawn with another look: its flat areas differ too.
        assert!(!identity_faults(&shares(12.0, 12.0, Some([1.0, 0.0, 11.0, 0.0]))).is_empty());
        // Even one the split puts down to filter order (a palette-like
        // texture with the wrong table) fails at the ceiling.
        assert!(!identity_faults(&shares(5.5, 5.5, Some([5.5, 0.0, 0.0, 0.0]))).is_empty());
        // As do differences outside the split.
        assert_eq!(identity_faults(&shares(6.0, 6.0, Some([0.5, 0.0, 0.0, 0.0]))).len(), 1);
    }

    #[test]
    fn identity_holds_the_gpu_to_the_software() {
        assert_eq!(identity_faults(&shares(0.9, 0.3, Some([0.3, 0.0, 0.0, 0.0]))).len(), 1);
    }

    #[test]
    fn identity_without_split_is_strict() {
        // With the depth buffer there's no split, so filter order can't be
        // told apart: the whole share is held to 1%.
        assert_eq!(identity_faults(&shares(2.560, 2.556, None)).len(), 1);
        assert!(identity_faults(&shares(0.5, 0.5, None)).is_empty());
    }
}
