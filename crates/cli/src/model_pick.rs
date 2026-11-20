//! SPIKE `remodel model-pick DIR OUT [--at X,Y]...`: which model drew each
//! 3D pixel of captured frames (captures made with `--geo native --light`,
//! whose light.bin carries each object block's model address and key).
//! Writes OUT/FRAME-models.png (3D pixels coloured by model key, grey where
//! there is none, black off the 3D layer), OUT/FRAME-models.json, and
//! prints a summary line per frame.

use model2::capture;
use model2::raster;
use model2::video::{self, View};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

fn colour(key: u64) -> [u8; 3] {
    let mut h = key.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    h ^= h >> 29;
    let c = |s: u32| 64 + ((h >> s) & 0x7f) as u8 * 1;
    [c(0), c(8), c(16)].map(|v| v.saturating_add(40))
}

#[derive(Default)]
struct M {
    oba: u32,
    obas: std::collections::BTreeSet<u32>,
    pixels: u64,
    polys: u64,
    blocks: u64,
    /// SPIKE: per object block, its polygons' texture headers and base colours.
    tex: BTreeMap<u32, std::collections::BTreeSet<String>>,
}

pub fn run(root: &Path, windows: (&[usize], &[usize]), out_dir: &Path, at: &[(usize, usize)]) -> Result<(), String> {
    let rom = capture::load_texture_rom(root).map_err(|e| format!("{}: {e}", root.display()))?;
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let view = View::native().with_hud_windows(windows.0, windows.1);
    let (w, h) = (view.width, view.height);
    let frames = capture::frames(root).map_err(|e| e.to_string())?;
    for item in frames {
        let (_, f) = item.map_err(|e| e.to_string())?;
        let Some(side) = f.light_frame() else {
            println!("frame {}: no light.bin", f.frame);
            continue;
        };
        let src = f.sources(&rom);
        let mut polys = raster::build_shaded(&f.stream, &src, None);
        video::HudLayout::new(&f.tiles, &f.palette, &view).anchor(&mut polys, &view);
        let placed = raster::place(&mut polys, &view);
        let vis = raster::visible(&polys, &placed, &src, &view);
        let max_block = polys.iter().map(|p| p.block as usize + 1).max().unwrap_or(0);
        let key_of = |p: &raster::Poly| side.blocks.get(p.block as usize).map_or(0, |b| b.key);
        let mut rgb = vec![0u8; w * h * 3];
        let (mut px3d, mut with, mut without) = (0u64, 0u64, 0u64);
        let mut models: BTreeMap<u64, M> = BTreeMap::new();
        let mut seen_poly = std::collections::HashSet::new();
        let mut seen_block = std::collections::HashSet::new();
        for o in 0..w * h {
            let k = vis.owner[o];
            if k == u32::MAX {
                continue;
            }
            px3d += 1;
            let p = &polys[placed[k as usize].poly];
            let key = key_of(p);
            if key == 0 {
                without += 1;
                rgb[o * 3..o * 3 + 3].copy_from_slice(&[90, 90, 90]);
            } else {
                with += 1;
                rgb[o * 3..o * 3 + 3].copy_from_slice(&colour(key));
                let m = models.entry(key).or_default();
                m.oba = side.blocks[p.block as usize].oba;
                m.obas.insert(m.oba);
                m.pixels += 1;
                if seen_poly.insert(k) {
                    m.polys += 1;
                    m.tex.entry(p.block).or_default().insert(format!("{:04x}{:04x}{:04x}{:04x}/{},{},{}/l{}", p.header[0], p.header[1], p.header[2], p.header[3], p.colour[0], p.colour[1], p.colour[2], p.luma));
                }
                if seen_block.insert(p.block) {
                    m.blocks += 1;
                }
            }
        }
        let direct = side.blocks.iter().filter(|b| b.flags & 1 != 0).count();
        let polyram = side.blocks.iter().filter(|b| b.poly_ram()).count();
        let mut js = String::new();
        let _ = write!(
            js,
            "{{\"frame\":{},\"blocks\":{},\"max_poly_block\":{},\"pixels_3d\":{px3d},\"with_model\":{with},\"no_model\":{without},\"models\":{{",
            f.frame,
            side.blocks.len(),
            max_block
        );
        for (i, (k, m)) in models.iter().enumerate() {
            let _ = write!(
                js,
                "{}\"{k:016x}\":{{\"oba\":\"0x{:06x}\",\"pixels\":{},\"polys\":{},\"instances\":{},\"obas\":[{}],\"tex\":[{}]}}",
                if i > 0 { "," } else { "" },
                m.oba,
                m.pixels,
                m.polys,
                m.blocks,
                m.obas.iter().map(|a| format!("\"0x{a:06x}\"")).collect::<Vec<_>>().join(","),
                m.tex.values().map(|t| format!("\"{}\"", t.iter().cloned().collect::<Vec<_>>().join("+"))).collect::<Vec<_>>().join(",")
            );
        }
        js.push_str("}}\n");
        std::fs::write(out_dir.join(format!("{:06}-models.json", f.frame)), js).map_err(|e| e.to_string())?;
        std::fs::write(out_dir.join(format!("{:06}-models.png", f.frame)), png::encode_rgb(w, h, &rgb))
            .map_err(|e| e.to_string())?;
        let distinct_oba: std::collections::BTreeSet<u32> = models.values().map(|m| m.oba).collect();
        println!(
            "frame {:>6}: direct {} polyram {} blocks {:>3} (polys see {:>3}) 3d px {:>6} model {:>6} ({:.1}%) none {:>6} distinct keys {:>3} distinct oba {:>3}",
            f.frame,
            direct,
            polyram,
            side.blocks.len(),
            max_block,
            px3d,
            with,
            100.0 * with as f64 / px3d.max(1) as f64,
            without,
            models.len(),
            distinct_oba.len()
        );
        for &(x, y) in at {
            let o = y * w + x;
            let k = vis.owner[o];
            if k == u32::MAX {
                println!("  at {x},{y}: not 3D");
                continue;
            }
            let p = &polys[placed[k as usize].poly];
            let b = side.blocks.get(p.block as usize);
            println!(
                "  at {x},{y}: model {:016x} oba {:06x} block {}",
                b.map_or(0, |b| b.key),
                b.map_or(0, |b| b.oba),
                p.block
            );
        }
    }
    Ok(())
}
