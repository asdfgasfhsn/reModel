//! `remodel lighting-check --spec-stats`: how much specular highlight a
//! game casts, from captures with the lighting side channel (`light.bin`,
//! `make lighting-capture`). Software only, no GPU. Per frame, over the 3D
//! layer's textured polygons (shadows and HUD windows left out):
//!
//! - polygons: textured, with a lamp (the exact layer, level L2, whose
//!   lighting depends on the normal: what material maps act on), and with
//!   a lamp whose lighting entry has a specular term (`squarings !=
//!   NO_SPEC`, a non-zero scale);
//! - pixels (as shown: the game's draw order, cut-out texels, not under
//!   high-priority tiles): those of
//!   textured polygons, of polygons with a specular term, and where that
//!   term changes the drawn brightness by at least 1 and 4 levels (the
//!   lamp evaluated with and without it, each truncated and clamped to the
//!   polygon's hue range as drawn), with mean and max; where the term is
//!   at least a level before clamping but is clamped away (the surface
//!   already at its top level). Two normals: the face's (`flat`: what the
//!   game draws) and the smooth-shaded normals interpolated (`smooth`: what
//!   `--shading pixel` and a material map would light with).
//!
//! Pictures (`FRAME-spec.png`, every `--every` Nth frame with a 3D layer):
//! the frame as the game draws it | the flat term | the smooth term;
//! black where nothing is textured, the frame dimmed where a textured
//! polygon has no specular term, dark blue where it has one that adds
//! nothing, red to yellow to white for 1, 8, 32 levels and more.

use model2::capture;
use model2::light::{self, Class, Material, Options};
use model2::raster::{self, Poly};
use model2::video::{self, View};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

#[derive(Default, Clone, Copy)]
struct Px {
    /// Pixels where the term adds >= 1 and >= 4 levels.
    ge1: u64,
    ge4: u64,
    /// Sum and max of the levels added.
    sum: u64,
    max: u32,
    /// The term is a level or more before clamping but adds nothing; of
    /// those, clamped below 255 by the hue range.
    clamped: u64,
    hue_cap: u64,
    /// The term under a level before clamping (geometry: the highlight
    /// points elsewhere).
    tiny: u64,
    /// With the term doubled (a specular map's most): pixels it would add
    /// >= 1 and >= 4 levels to.
    x2_ge1: u64,
    x2_ge4: u64,
}

impl Px {
    fn add(&mut self, (c, raw, cap, c2): (u32, f32, bool, u32)) {
        self.hue_cap += (c == 0 && raw >= 1.0 && cap) as u64;
        self.tiny += (raw < 1.0) as u64;
        self.x2_ge1 += (c2 >= 1) as u64;
        self.x2_ge4 += (c2 >= 4) as u64;
        self.ge1 += (c >= 1) as u64;
        self.ge4 += (c >= 4) as u64;
        self.sum += c as u64;
        self.max = self.max.max(c);
        self.clamped += (c == 0 && raw >= 1.0) as u64;
    }
    fn merge(&mut self, o: &Px) {
        self.ge1 += o.ge1;
        self.ge4 += o.ge4;
        self.sum += o.sum;
        self.max = self.max.max(o.max);
        self.clamped += o.clamped;
        self.hue_cap += o.hue_cap;
        self.tiny += o.tiny;
        self.x2_ge1 += o.x2_ge1;
        self.x2_ge4 += o.x2_ge4;
    }
}

#[derive(Default, Clone, Copy)]
struct Count {
    textured: u64,
    l2: u64,
    lamp: u64,
    spec: u64,
    /// With a specular control but a zero scale (no term in effect).
    zero_scale: u64,
    tex_px: u64,
    spec_px: u64,
    flat: Px,
    smooth: Px,
}

impl Count {
    fn merge(&mut self, o: &Count) {
        self.textured += o.textured;
        self.l2 += o.l2;
        self.lamp += o.lamp;
        self.spec += o.spec;
        self.zero_scale += o.zero_scale;
        self.tex_px += o.tex_px;
        self.spec_px += o.spec_px;
        self.flat.merge(&o.flat);
        self.smooth.merge(&o.smooth);
    }
}

fn pct(a: u64, b: u64) -> f64 {
    if b == 0 {
        0.0
    } else {
        100.0 * a as f64 / b as f64
    }
}

/// Whether a lamp has a specular term at all.
fn has_spec(l: &light::Lamp) -> bool {
    l.squarings != Material::NO_SPEC && l.spec_scale != 0.0
}

/// The levels the specular term adds to a pixel lit with `pl` for unit
/// normal `n`, clamped to `range`; the term itself before clamping;
/// whether the hue range caps it below 255; the levels the term doubled
/// would add.
fn added(pl: &light::PixelLight, n: [f32; 3], range: [u8; 2]) -> (u32, f32, bool, u32) {
    let [lo, hi] = range.map(u32::from);
    let with = light::quantise(pl.luma(n)).clamp(lo, hi);
    let base = light::Lamp { squarings: Material::NO_SPEC, ..pl.lamp };
    let without = light::quantise(base.eval(n) + pl.offset).clamp(lo, hi);
    let raw = pl.lamp.eval(n) - base.eval(n);
    let raw = if raw.is_finite() { raw } else { 0.0 };
    let x2 = light::quantise(base.eval(n) + pl.offset + 2.0 * raw).clamp(lo, hi);
    (with.saturating_sub(without), raw, hi < 255 && light::quantise(pl.luma(n)) > hi, x2.saturating_sub(without))
}

fn heat(c: u32) -> [u8; 3] {
    let c = c as f32;
    let r = (96.0 + c * 20.0).min(255.0);
    let g = ((c - 1.0) * 255.0 / 31.0).clamp(0.0, 255.0);
    let b = ((c - 32.0) * 8.0).clamp(0.0, 255.0);
    [r as u8, g as u8, b as u8]
}

#[derive(Default)]
struct Line {
    lamps: HashMap<(u32, u32), (u64, u64)>,
}

pub fn run(
    root: &Path,
    hud_windows: (&[usize], &[usize]),
    view: Option<View>,
    opts: Options,
    out_dir: &Path,
    every: usize,
) -> Result<bool, String> {
    let texture_rom = capture::load_texture_rom(root)
        .map_err(|e| format!("{}: {e} (capture with `make lighting-capture`)", root.display()))?;
    let view = view.unwrap_or_else(|| View::new(720, 4.0 / 3.0, video::Hud::Edges));
    let view = view.with_hud_windows(hud_windows.0, hud_windows.1);
    let (w, h) = (view.width, view.height);
    // Per pixel with the lamps material maps get, so both normals are there.
    let opts = Options { diagnose: true, shading: light::Shading::Pixel, ..opts }.with_lamps(true);
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let mut rep = String::new();
    let _ =
        writeln!(rep, "spec-stats    view {w}x{h}; textured 3D polygons (no shadows, no HUD windows), pixels as shown");
    let _ = writeln!(
        rep,
        "{:<7} {:>6} {:>6} {:>6} {:>6} {:>8} {:>8}  {:>7} {:>7} {:>5} {:>4}  {:>7} {:>7} {:>5} {:>4}",
        "frame",
        "tex",
        "L2",
        "lamp",
        "spec",
        "tex px",
        "spec px",
        "flat>=1",
        ">=4",
        "mean",
        "max",
        "smth>=1",
        ">=4",
        "mean",
        "max"
    );
    print!("{rep}");
    let frames = capture::frames(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let mut total = Count::default();
    let mut lines = Line::default();
    let (mut n_frames, mut no_side, mut with_3d) = (0usize, 0usize, 0usize);
    for item in frames {
        let (_, f) = item.map_err(|e| format!("{}: {e}", root.display()))?;
        n_frames += 1;
        let Some(side) = f.light_frame() else {
            no_side += 1;
            continue;
        };
        let src = f.sources(&texture_rom);
        let sh = light::shade_with(&f.stream, Some(&side), &opts);
        let info: HashMap<u32, light::Info> = sh.info.iter().map(|i| (i.at, *i)).collect();
        let b = raster::BuildIn {
            lit: Some(&sh),
            skip_checker: opts.skip_checker,
            fine: opts.fine,
            relief: opts.relief,
            ..Default::default()
        };
        let mut polys: Vec<Poly> = raster::build_with(&f.stream, &src, &b).0;
        video::HudLayout::new(&f.tiles, &f.palette, &view).anchor(&mut polys, &view);
        let placed = raster::place(&mut polys, &view);
        let vis = raster::visible(&polys, &placed, &src, &view);
        // What the 2D layers leave of the 3D layer: compose it in a marker
        // colour and see where that survives (high-priority tiles cover it).
        let cover: Vec<bool> = vis.owner.iter().map(|&k| k != u32::MAX).collect();
        let marker = [1u8, 254, 3];
        let marked = video::compose(&f.tiles, &f.palette, Some((&marker.repeat(w * h), &cover)), &view);
        let shown: Vec<bool> = (0..w * h).map(|o| cover[o] && marked[o * 3..o * 3 + 3] == marker).collect();
        let mut c = Count::default();
        let counted =
            |p: &Poly| p.texture.is_some() && !p.shadow && info.get(&p.at).is_none_or(|i| i.class != Class::Excluded);
        // Per pixel: 0 none, 1 textured, 2 specular lamp; and the two terms.
        let mut kind = vec![0u8; w * h];
        let mut terms = vec![(0u32, 0u32); w * h];
        for (k, pl) in placed.iter().enumerate() {
            let p = &polys[pl.poly];
            if !counted(p) {
                continue;
            }
            c.textured += 1;
            c.l2 += info.get(&p.at).is_some_and(|i| i.level == light::Level::L2) as u64;
            let Some(l) = &p.lamp else { continue };
            c.lamp += 1;
            c.zero_scale += (l.light.lamp.squarings != Material::NO_SPEC && l.light.lamp.spec_scale == 0.0) as u64;
            if !has_spec(&l.light.lamp) {
                continue;
            }
            c.spec += 1;
            let key = (l.light.lamp.spec_scale.to_bits(), l.light.lamp.squarings);
            for (o, n) in raster::lamp_pixel_normals(p, pl, &view) {
                if vis.owner[o] != k as u32 || !shown[o] {
                    continue;
                }
                let af = added(&l.light, l.n, l.range);
                let a = added(&l.light, n, l.range);
                let (cf, cs) = (af.0, a.0);
                c.flat.add(af);
                c.smooth.add(a);
                kind[o] = 2;
                terms[o] = (cf, cs);
                let e = lines.lamps.entry(key).or_default();
                e.0 += 1;
                e.1 += (cs >= 1) as u64;
            }
        }
        for o in 0..w * h {
            let k = vis.owner[o];
            if k == u32::MAX || !shown[o] {
                continue;
            }
            if counted(&polys[placed[k as usize].poly]) {
                c.tex_px += 1;
                c.spec_px += (kind[o] == 2) as u64;
                if kind[o] == 0 {
                    kind[o] = 1;
                }
            }
        }
        let mean = |p: &Px| if p.ge1 == 0 { 0.0 } else { p.sum as f64 / p.ge1 as f64 };
        let _ = writeln!(
            rep,
            "{:<7} {:>6} {:>6} {:>6} {:>6} {:>8} {:>8}  {:>7} {:>7} {:>5.1} {:>4}  {:>7} {:>7} {:>5.1} {:>4}",
            f.frame,
            c.textured,
            c.l2,
            c.lamp,
            c.spec,
            c.tex_px,
            c.spec_px,
            c.flat.ge1,
            c.flat.ge4,
            mean(&c.flat),
            c.flat.max,
            c.smooth.ge1,
            c.smooth.ge4,
            mean(&c.smooth),
            c.smooth.max
        );
        print!("{}", rep.lines().last().map(|l| format!("{l}\n")).unwrap_or_default());
        if c.textured > 0 {
            if with_3d % every.max(1) == 0 {
                let flat = raster::build_shaded_with(&f.stream, &src, Some(&side), None);
                let mut flat = flat;
                video::HudLayout::new(&f.tiles, &f.palette, &view).anchor(&mut flat, &view);
                let layer = raster::draw_view(&mut flat, &src, &view);
                let rgb = video::compose(&f.tiles, &f.palette, Some((&layer.0, &layer.1)), &view);
                let mut pic = vec![0u8; 3 * w * h * 3];
                for y in 0..h {
                    for x in 0..w {
                        let o = y * w + x;
                        let px = &rgb[o * 3..o * 3 + 3];
                        let row = y * 3 * w;
                        pic[(row + x) * 3..(row + x) * 3 + 3].copy_from_slice(px);
                        for (panel, term) in [(1, terms[o].0), (2, terms[o].1)] {
                            let colour = match kind[o] {
                                0 => [0, 0, 0],
                                1 => [px[0] / 4, px[1] / 4, px[2] / 4],
                                _ if term == 0 => [0, 0, 110],
                                _ => heat(term),
                            };
                            let i = (row + panel * w + x) * 3;
                            pic[i..i + 3].copy_from_slice(&colour);
                        }
                    }
                }
                let _ =
                    std::fs::write(out_dir.join(format!("{:06}-spec.png", f.frame)), png::encode_rgb(3 * w, h, &pic));
            }
            with_3d += 1;
        }
        total.merge(&c);
    }
    let _ = writeln!(
        rep,
        "frames        {n_frames} ({no_side} without light.bin, skipped; {with_3d} with textured 3D polygons)"
    );
    let _ = writeln!(
        rep,
        "polygons      {} textured: {} at level L2 ({:.1}%), {} with a lamp ({:.1}%), {} with a specular term ({:.2}% of textured, {:.2}% of those with a lamp); {} more with a specular control but a zero scale",
        total.textured,
        total.l2,
        pct(total.l2, total.textured),
        total.lamp,
        pct(total.lamp, total.textured),
        total.spec,
        pct(total.spec, total.textured),
        pct(total.spec, total.lamp),
        total.zero_scale
    );
    let _ = writeln!(
        rep,
        "pixels        {} textured shown, {} of polygons with a specular term ({:.2}%)",
        total.tex_px,
        total.spec_px,
        pct(total.spec_px, total.tex_px)
    );
    for (name, p) in [("flat", &total.flat), ("smooth", &total.smooth)] {
        let _ = writeln!(
            rep,
            "  {name:<6}      adds >=1 level: {} ({:.2}% of those, {:.3}% of textured), >=4: {} ({:.2}%, {:.3}%); mean {:.2} where >=1, {:.3} over all; max {}; clamped away (term >=1 before clamping, adds nothing): {} ({:.2}%; {} by a hue range under 255); term under a level: {} ({:.2}%); doubled it would add >=1: {} ({:.2}%), >=4: {} ({:.2}%)",
            p.ge1,
            pct(p.ge1, total.spec_px),
            pct(p.ge1, total.tex_px),
            p.ge4,
            pct(p.ge4, total.spec_px),
            pct(p.ge4, total.tex_px),
            if p.ge1 == 0 { 0.0 } else { p.sum as f64 / p.ge1 as f64 },
            if total.spec_px == 0 { 0.0 } else { p.sum as f64 / total.spec_px as f64 },
            p.max,
            p.clamped,
            pct(p.clamped, total.spec_px),
            p.hue_cap,
            p.tiny,
            pct(p.tiny, total.spec_px),
            p.x2_ge1,
            pct(p.x2_ge1, total.spec_px),
            p.x2_ge4,
            pct(p.x2_ge4, total.spec_px)
        );
    }
    let mut ls: Vec<_> = lines.lamps.iter().collect();
    ls.sort_by_key(|e| std::cmp::Reverse(e.1 .0));
    let _ = writeln!(rep, "lamps         specular entries by pixels shown (scale, squarings: pixels, smooth >=1)");
    for ((s, q), (px, ge1)) in ls.iter().take(8) {
        let _ = writeln!(rep, "  {:>9.3} {:>3}: {:>9} {:>9}", f32::from_bits(*s), q, px, ge1);
    }
    let _ = writeln!(rep, "pictures      FRAME-spec.png (frame | flat term | smooth term) in {}", out_dir.display());
    let tail: String = rep.lines().skip_while(|l| !l.starts_with("frames ")).map(|l| format!("{l}\n")).collect();
    print!("{tail}");
    std::fs::write(out_dir.join("spec-stats.txt"), &rep).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    Ok(true)
}
