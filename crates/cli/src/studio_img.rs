//! The studio's image work, std only: loading and saving PNGs, resampling,
//! the local style processor (recipes: palette ramp, despeckle, edge ink,
//! halftone, all wrap-aware), surface tinting and the seam check
//! (docs/uplift-studio.md).
//!
//! Textures repeat or mirror on the model, so every filter here runs on a
//! copy padded the way the texture wraps and crops the padding after:
//! what leaves one edge continues at the opposite one, and an edge line is
//! drawn only where the texture really ends (clamped), never where it
//! wraps.

#![allow(clippy::needless_range_loop, clippy::chunks_exact_to_as_chunks, clippy::manual_is_multiple_of)]

pub use png::Image;
use std::collections::HashMap;
use std::path::Path;

// ---------------------------------------------------------------- files

pub fn load(path: &Path) -> Result<Image, String> {
    let d = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    png::decode(&d).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn save(path: &Path, img: &Image) -> Result<(), String> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    let opaque = img.rgba.chunks_exact(4).all(|p| p[3] == 255);
    let data = if opaque {
        let rgb: Vec<u8> = img.rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        png::encode_rgb(img.width, img.height, &rgb)
    } else {
        png::encode_rgba(img.width, img.height, &img.rgba)
    };
    std::fs::write(path, data).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn blank(w: usize, h: usize, c: [u8; 4]) -> Image {
    Image { width: w, height: h, rgba: c.iter().copied().cycle().take(w * h * 4).collect() }
}

pub fn parse_colour(s: &str) -> Result<[u8; 3], String> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("`{s}` is not a #rrggbb colour"));
    }
    let v = u32::from_str_radix(h, 16).unwrap();
    Ok([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

// ------------------------------------------------------------- wrapping

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wrap {
    Repeat,
    Mirror,
    Clamp,
}

impl Wrap {
    pub fn name(self) -> &'static str {
        match self {
            Wrap::Repeat => "repeat",
            Wrap::Mirror => "mirror",
            Wrap::Clamp => "clamp",
        }
    }
}

pub fn wrap_idx(i: isize, n: usize, m: Wrap) -> usize {
    let n = n as isize;
    (match m {
        Wrap::Repeat => i.rem_euclid(n),
        Wrap::Clamp => i.clamp(0, n - 1),
        Wrap::Mirror => {
            let p = i.rem_euclid(2 * n);
            if p < n {
                p
            } else {
                2 * n - 1 - p
            }
        }
    }) as usize
}

/// The wrap of an inventory's `wrap` column (`rr`, `mr`, `rm`, `mm`, a
/// comma list of them, or `-`): u then v. A list repeats on an axis if any
/// use does (the stricter test); `-` (never seen wrapping) is repeat for
/// textures and clamp for tiles.
pub fn parse_wrap(s: &str, tile: bool) -> (Wrap, Wrap) {
    let uses: Vec<&str> = s.split(',').filter(|x| x.len() == 2).collect();
    if uses.is_empty() {
        let d = if tile { Wrap::Clamp } else { Wrap::Repeat };
        return (d, d);
    }
    let axis = |i: usize| {
        if uses.iter().any(|u| u.as_bytes()[i] == b'r') {
            Wrap::Repeat
        } else {
            Wrap::Mirror
        }
    };
    (axis(0), axis(1))
}

pub fn parse_wrap_word(s: &str) -> Option<Wrap> {
    Some(match s {
        "repeat" | "r" => Wrap::Repeat,
        "mirror" | "m" => Wrap::Mirror,
        "clamp" | "c" => Wrap::Clamp,
        _ => return None,
    })
}

/// `img` with `px` pixels added on every side, as it wraps.
pub fn pad(img: &Image, px: usize, wu: Wrap, wv: Wrap) -> Image {
    let (w, h) = (img.width, img.height);
    let (nw, nh) = (w + 2 * px, h + 2 * px);
    let mut out = blank(nw, nh, [0; 4]);
    for y in 0..nh {
        let sy = wrap_idx(y as isize - px as isize, h, wv);
        for x in 0..nw {
            let sx = wrap_idx(x as isize - px as isize, w, wu);
            let (s, d) = ((sy * w + sx) * 4, (y * nw + x) * 4);
            out.rgba[d..d + 4].copy_from_slice(&img.rgba[s..s + 4]);
        }
    }
    out
}

pub fn crop(img: &Image, x0: usize, y0: usize, w: usize, h: usize) -> Image {
    let mut out = blank(w, h, [0; 4]);
    for y in 0..h {
        let s = ((y0 + y) * img.width + x0) * 4;
        out.rgba[y * w * 4..(y + 1) * w * 4].copy_from_slice(&img.rgba[s..s + w * 4]);
    }
    out
}

// ----------------------------------------------------------- resampling

fn axis_weights(src: usize, dst: usize) -> Vec<Vec<(usize, f32)>> {
    (0..dst)
        .map(|o| {
            if dst >= src {
                // bilinear, pixel centres
                let s = (o as f32 + 0.5) * src as f32 / dst as f32 - 0.5;
                let i0 = s.floor();
                let f = s - i0;
                let a = (i0 as isize).clamp(0, src as isize - 1) as usize;
                let b = (i0 as isize + 1).clamp(0, src as isize - 1) as usize;
                vec![(a, 1.0 - f), (b, f)]
            } else {
                // area average
                let (a, b) = (o as f32 * src as f32 / dst as f32, (o + 1) as f32 * src as f32 / dst as f32);
                let mut v = Vec::new();
                let mut i = a.floor() as usize;
                while (i as f32) < b && i < src {
                    let w = (b.min(i as f32 + 1.0) - a.max(i as f32)).max(0.0);
                    if w > 0.0 {
                        v.push((i, w / (b - a)));
                    }
                    i += 1;
                }
                v
            }
        })
        .collect()
}

/// Bilinear when growing, area average when shrinking, each axis alone.
pub fn resize(img: &Image, nw: usize, nh: usize) -> Image {
    if nw == img.width && nh == img.height {
        return img.clone();
    }
    let (wx, wy) = (axis_weights(img.width, nw), axis_weights(img.height, nh));
    let mut tmp = vec![0f32; nw * img.height * 4];
    for y in 0..img.height {
        for (x, ws) in wx.iter().enumerate() {
            for c in 0..4 {
                tmp[(y * nw + x) * 4 + c] =
                    ws.iter().map(|&(i, w)| img.rgba[(y * img.width + i) * 4 + c] as f32 * w).sum();
            }
        }
    }
    let mut out = blank(nw, nh, [0; 4]);
    for (y, ws) in wy.iter().enumerate() {
        for x in 0..nw {
            for c in 0..4 {
                let v: f32 = ws.iter().map(|&(i, w)| tmp[(i * nw + x) * 4 + c] * w).sum();
                out.rgba[(y * nw + x) * 4 + c] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// Whole-number nearest-neighbour enlargement.
pub fn nearest(img: &Image, k: usize) -> Image {
    let (nw, nh) = (img.width * k, img.height * k);
    let mut out = blank(nw, nh, [0; 4]);
    for y in 0..nh {
        for x in 0..nw {
            let s = ((y / k) * img.width + x / k) * 4;
            let d = (y * nw + x) * 4;
            out.rgba[d..d + 4].copy_from_slice(&img.rgba[s..s + 4]);
        }
    }
    out
}

pub fn mean_rgb(img: &Image) -> [f32; 3] {
    let (mut s, mut n) = ([0f64; 3], 0f64);
    for p in img.rgba.chunks_exact(4) {
        if p[3] == 0 {
            continue;
        }
        for c in 0..3 {
            s[c] += p[c] as f64;
        }
        n += 1.0;
    }
    let n = n.max(1.0);
    [(s[0] / n) as f32, (s[1] / n) as f32, (s[2] / n) as f32]
}

fn luma(r: f32, g: f32, b: f32) -> f32 {
    0.299 * r + 0.587 * g + 0.114 * b
}

// -------------------------------------------------------------- recipes

/// A resolved recipe: how one texture is restyled locally.
#[derive(Clone, Debug)]
pub struct Recipe {
    /// Control colours of the ramp, dark to light.
    pub palette: Vec<[u8; 3]>,
    /// Tones the ramp is sampled into (a palette of exactly this many is used as it is).
    pub tones: usize,
    /// Output size over the source's.
    pub scale: usize,
    /// Gaussian sigma in source texels, before quantising.
    pub blur: f32,
    /// Median radius in output pixels.
    pub median: usize,
    /// Percent of the darkest and lightest pixels clipped when stretching the contrast.
    pub stretch: f32,
    /// Added to the stretched brightness (0..1 is the whole range): shifts the mix of tones.
    pub bias: f32,
    /// Regions of one tone smaller than this (source texels squared) are absorbed by the neighbour.
    pub despeckle: f32,
    /// Edge ink width in output pixels (0: none).
    pub ink: f32,
    pub ink_colour: [u8; 3],
    /// Which side of a tone step the ink is drawn on.
    pub ink_side: InkSide,
    /// Tone steps smaller than this draw no ink.
    pub ink_step: usize,
    /// Hairline in output pixels on the edges where the texture is clamped
    /// (never on wrapped edges).
    pub border_ink: f32,
    /// Halftone between tones: 0 off (hard steps), else the share of a
    /// tone step that is dotted around the step (1: the whole ramp).
    pub halftone: f32,
    /// Distance between dots, output pixels.
    pub pitch: f32,
    /// Screen angle in degrees; 0 and 45 repeat exactly over the texture.
    pub angle: f32,
    /// Overrides the texture's own wrap (u, v).
    pub wrap: Option<Wrap>,
    /// Width in source texels over which a repeating texture's jump between
    /// its opposite edges is levelled out before restyling (0: not).
    pub seamless: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InkSide {
    Light,
    Dark,
    Both,
}

impl Default for Recipe {
    fn default() -> Self {
        Recipe {
            palette: vec![[0x11, 0x10, 0x18], [0xff, 0xf6, 0xe0]],
            tones: 3,
            scale: 4,
            blur: 0.6,
            median: 2,
            stretch: 2.0,
            bias: 0.0,
            despeckle: 0.0,
            ink: 0.0,
            ink_colour: [0x11, 0x10, 0x18],
            ink_side: InkSide::Light,
            ink_step: 1,
            border_ink: 0.0,
            halftone: 0.0,
            pitch: 4.0,
            angle: 45.0,
            wrap: None,
            seamless: 0,
        }
    }
}

/// The recipe file: `[default]`, `[material NAME]` and `[key NAME]`
/// sections of `name = value` lines (`#` comments). A texture's recipe is
/// the default, then its material's (the selection's category), then its
/// own `[key]` section, each overriding the last.
#[derive(Default, Debug)]
pub struct RecipeFile {
    sections: Vec<(String, HashMap<String, String>)>,
}

impl RecipeFile {
    pub fn parse(text: &str) -> Result<RecipeFile, String> {
        let mut f = RecipeFile::default();
        for (n, line) in text.lines().enumerate() {
            // A comment starts with `# ` (or a bare `#`) at the line's start or after a space; `#rrggbb` is a colour.
            let mut cut = line.len();
            for (i, _) in line.match_indices('#') {
                let before_ok = i == 0 || line[..i].ends_with(char::is_whitespace);
                let after = line[i + 1..].chars().next();
                if before_ok && after.is_none_or(|c| c.is_whitespace() || i == 0) {
                    cut = i;
                    break;
                }
            }
            let line = line[..cut].trim();
            if line.is_empty() {
                continue;
            }
            if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                f.sections.push((s.trim().to_string(), HashMap::new()));
            } else if let Some((k, v)) = line.split_once('=') {
                let Some(sec) = f.sections.last_mut() else {
                    return Err(format!("recipe line {}: `{line}` before any [section]", n + 1));
                };
                sec.1.insert(k.trim().to_string(), v.trim().to_string());
            } else {
                return Err(format!("recipe line {}: `{line}` is not `name = value`", n + 1));
            }
        }
        // Check every section parses now, not when first used.
        for (s, _) in f.sections.clone() {
            let (kind, name) = s.split_once(' ').unwrap_or((&s, ""));
            f.resolve(if kind == "material" { name } else { "" }, if kind == "key" { name } else { "" })
                .map_err(|e| format!("recipe [{s}]: {e}"))?;
        }
        Ok(f)
    }

    pub fn resolve(&self, material: &str, key: &str) -> Result<Recipe, String> {
        let mut map: HashMap<&str, &str> = HashMap::new();
        for want in ["default".to_string(), format!("material {material}"), format!("key {key}")] {
            for (s, kv) in &self.sections {
                if *s == want {
                    for (k, v) in kv {
                        map.insert(k, v);
                    }
                }
            }
        }
        recipe_from(&map)
    }
}

fn recipe_from(m: &HashMap<&str, &str>) -> Result<Recipe, String> {
    let mut r = Recipe::default();
    let num = |k: &str, v: &str| v.parse::<f32>().map_err(|_| format!("{k}: `{v}` is not a number"));
    for (&k, &v) in m {
        match k {
            "palette" => {
                r.palette = v
                    .split(|c: char| c.is_whitespace() || c == ',')
                    .filter(|s| !s.is_empty())
                    .map(parse_colour)
                    .collect::<Result<_, _>>()?;
                if r.palette.is_empty() {
                    return Err("palette is empty".into());
                }
            }
            "tones" => r.tones = (num(k, v)? as usize).clamp(2, 16),
            "scale" => r.scale = (num(k, v)? as usize).clamp(1, 16),
            "blur" => r.blur = num(k, v)?.max(0.0),
            "median" => r.median = num(k, v)?.max(0.0) as usize,
            "stretch" => r.stretch = num(k, v)?.clamp(0.0, 45.0),
            "bias" => r.bias = num(k, v)?,
            "despeckle" => r.despeckle = num(k, v)?.max(0.0),
            "ink" => r.ink = num(k, v)?.max(0.0),
            "ink_colour" => r.ink_colour = parse_colour(v)?,
            "ink_side" => {
                r.ink_side = match v {
                    "light" => InkSide::Light,
                    "dark" => InkSide::Dark,
                    "both" => InkSide::Both,
                    _ => return Err(format!("ink_side: `{v}` is not light, dark or both")),
                }
            }
            "ink_step" => r.ink_step = (num(k, v)? as usize).max(1),
            "border_ink" => r.border_ink = num(k, v)?.max(0.0),
            "halftone" => r.halftone = num(k, v)?.clamp(0.0, 1.0),
            "pitch" => r.pitch = num(k, v)?.max(2.0),
            "angle" => r.angle = num(k, v)?,
            "seamless" => r.seamless = num(k, v)?.max(0.0) as usize,
            "wrap" => {
                r.wrap = match v {
                    "auto" => None,
                    _ => Some(
                        parse_wrap_word(v)
                            .ok_or_else(|| format!("wrap: `{v}` is not auto, repeat, mirror or clamp"))?,
                    ),
                }
            }
            _ => return Err(format!("unknown setting `{k}`")),
        }
    }
    Ok(r)
}

/// `n` tones sampled evenly along the control colours (linear in sRGB).
pub fn ramp(palette: &[[u8; 3]], n: usize) -> Vec<[u8; 3]> {
    if palette.len() == n {
        return palette.to_vec();
    }
    (0..n)
        .map(|i| {
            if palette.len() == 1 {
                return palette[0];
            }
            let t = i as f32 / (n - 1) as f32 * (palette.len() - 1) as f32;
            let (a, f) = (t.floor() as usize, t - t.floor());
            let b = (a + 1).min(palette.len() - 1);
            let mut c = [0u8; 3];
            for k in 0..3 {
                c[k] = (palette[a][k] as f32 * (1.0 - f) + palette[b][k] as f32 * f).round() as u8;
            }
            c
        })
        .collect()
}

// ------------------------------------------------------ the processor

struct Plane {
    w: usize,
    h: usize,
    d: Vec<f32>,
}

fn gauss(p: &Plane, sigma: f32) -> Plane {
    if sigma < 0.3 {
        return Plane { w: p.w, h: p.h, d: p.d.clone() };
    }
    let r = (sigma * 3.0).ceil() as isize;
    let k: Vec<f32> = (-r..=r).map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let sum: f32 = k.iter().sum();
    let k: Vec<f32> = k.iter().map(|v| v / sum).collect();
    let mut t = vec![0f32; p.w * p.h];
    for y in 0..p.h {
        for x in 0..p.w {
            t[y * p.w + x] = k
                .iter()
                .enumerate()
                .map(|(i, w)| p.d[y * p.w + (x as isize + i as isize - r).clamp(0, p.w as isize - 1) as usize] * w)
                .sum();
        }
    }
    let mut o = vec![0f32; p.w * p.h];
    for y in 0..p.h {
        for x in 0..p.w {
            o[y * p.w + x] = k
                .iter()
                .enumerate()
                .map(|(i, w)| t[(y as isize + i as isize - r).clamp(0, p.h as isize - 1) as usize * p.w + x] * w)
                .sum();
        }
    }
    Plane { w: p.w, h: p.h, d: o }
}

fn median(p: &Plane, r: usize) -> Plane {
    if r == 0 {
        return Plane { w: p.w, h: p.h, d: p.d.clone() };
    }
    let mut o = vec![0f32; p.w * p.h];
    let mut win = Vec::with_capacity((2 * r + 1) * (2 * r + 1));
    for y in 0..p.h {
        for x in 0..p.w {
            win.clear();
            for dy in -(r as isize)..=r as isize {
                let yy = (y as isize + dy).clamp(0, p.h as isize - 1) as usize;
                for dx in -(r as isize)..=r as isize {
                    let xx = (x as isize + dx).clamp(0, p.w as isize - 1) as usize;
                    win.push(p.d[yy * p.w + xx]);
                }
            }
            let mid = win.len() / 2;
            win.select_nth_unstable_by(mid, |a, b| a.total_cmp(b));
            o[y * p.w + x] = win[mid];
        }
    }
    Plane { w: p.w, h: p.h, d: o }
}

/// Absorb 4-connected regions of one level smaller than `min_area` into
/// the level they touch most. Returns which pixels changed.
fn despeckle(levels: &mut [u8], w: usize, h: usize, min_area: usize) -> Vec<bool> {
    let mut changed = vec![false; w * h];
    if min_area <= 1 {
        return changed;
    }
    let mut seen = vec![false; w * h];
    let mut stack = Vec::new();
    for start in 0..w * h {
        if seen[start] {
            continue;
        }
        let lv = levels[start];
        let mut comp = Vec::new();
        stack.push(start);
        seen[start] = true;
        while let Some(i) = stack.pop() {
            comp.push(i);
            let (x, y) = (i % w, i / w);
            let mut push = |j: usize| {
                if !seen[j] && levels[j] == lv {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if x > 0 {
                push(i - 1);
            }
            if x + 1 < w {
                push(i + 1);
            }
            if y > 0 {
                push(i - w);
            }
            if y + 1 < h {
                push(i + w);
            }
        }
        if comp.len() >= min_area {
            continue;
        }
        let mut votes = [0usize; 32];
        for &i in &comp {
            let (x, y) = (i % w, i / w);
            let mut v = |j: usize| {
                if levels[j] != lv {
                    votes[levels[j] as usize & 31] += 1;
                }
            };
            if x > 0 {
                v(i - 1);
            }
            if x + 1 < w {
                v(i + 1);
            }
            if y > 0 {
                v(i - w);
            }
            if y + 1 < h {
                v(i + w);
            }
        }
        if let Some((best, n)) = votes.iter().enumerate().max_by_key(|&(_, n)| *n) {
            if *n > 0 {
                for &i in &comp {
                    levels[i] = best as u8;
                    changed[i] = true;
                }
            }
        }
    }
    changed
}

fn divisor_near(target: f32, n: usize) -> f32 {
    (2..=n.max(2))
        .filter(|d| n % d == 0)
        .min_by(|a, b| (*a as f32 - target).abs().total_cmp(&(*b as f32 - target).abs()))
        .unwrap_or(n.max(2)) as f32
}

/// The dot screen's value (0..1) at output position (x, y) of the
/// unpadded image: periodic over the image for angles 0 and 45.
fn screen(x: f32, y: f32, r: &Recipe, period: f32) -> f32 {
    let (u, v) = if r.angle.rem_euclid(90.0) == 45.0 {
        (x + y, x - y)
    } else if r.angle.rem_euclid(90.0) == 0.0 {
        (x, y)
    } else {
        let a = r.angle.to_radians();
        (x * a.cos() + y * a.sin(), -x * a.sin() + y * a.cos())
    };
    let k = std::f32::consts::TAU / period;
    0.5 + 0.25 * ((k * u).cos() + (k * v).cos())
}

/// Restyle `src` with `r` as a texture wrapping (`wu`, `wv`).
pub fn style(src: &Image, r: &Recipe, wu: Wrap, wv: Wrap) -> Image {
    let (wu, wv) = match r.wrap {
        Some(w) => (w, w),
        None => (wu, wv),
    };
    let s = r.scale;
    let padsrc = 8usize;
    let leveled;
    let src = if r.seamless > 0 && (wu == Wrap::Repeat || wv == Wrap::Repeat) {
        leveled = level_seams(src, r.seamless, wu, wv);
        &leveled
    } else {
        src
    };
    let padded = pad(src, padsrc, wu, wv);
    let up = resize(&padded, padded.width * s, padded.height * s);
    let (pw, ph) = (up.width, up.height);
    let padpx = padsrc * s;
    let (ow, oh) = (src.width * s, src.height * s);

    let lum = Plane {
        w: pw,
        h: ph,
        d: up.rgba.chunks_exact(4).map(|p| luma(p[0] as f32, p[1] as f32, p[2] as f32) / 255.0).collect(),
    };
    let lum = gauss(&lum, r.blur * s as f32);
    let lum = median(&lum, r.median);

    // Contrast stretch over the unpadded part.
    let mut inner: Vec<f32> = Vec::with_capacity(ow * oh);
    for y in 0..oh {
        inner.extend_from_slice(&lum.d[(y + padpx) * pw + padpx..(y + padpx) * pw + padpx + ow]);
    }
    inner.sort_by(|a, b| a.total_cmp(b));
    let cut = (inner.len() as f32 * r.stretch / 100.0) as usize;
    let (lo, hi) = (inner[cut.min(inner.len() - 1)], inner[inner.len() - 1 - cut.min(inner.len() - 1)]);
    let span = (hi - lo).max(1.0 / 64.0);

    let n = r.tones;
    let tone_t: Vec<f32> =
        lum.d.iter().map(|&v| (((v - lo) / span + r.bias).clamp(0.0, 1.0)) * (n - 1) as f32).collect();
    let mut level: Vec<u8> = tone_t.iter().map(|t| t.round() as u8).collect();
    let min_area = (r.despeckle * (s * s) as f32).round() as usize;
    let changed = despeckle(&mut level, pw, ph, min_area);

    // Tones: hard, or dotted around the steps.
    let period = if r.angle.rem_euclid(90.0) == 45.0 {
        divisor_near(r.pitch * std::f32::consts::SQRT_2, gcd(ow, oh))
    } else if r.angle.rem_euclid(90.0) == 0.0 {
        divisor_near(r.pitch, gcd(ow, oh))
    } else {
        r.pitch
    };
    let colours = ramp(&r.palette, n);
    let mut tone = level.clone();
    if r.halftone > 0.0 {
        for y in 0..ph {
            for x in 0..pw {
                let i = y * pw + x;
                if changed[i] {
                    continue;
                }
                let t = tone_t[i];
                let (lo_l, f) = (t.floor(), t - t.floor());
                // Dotted only in a band around the step's middle.
                let wd = r.halftone;
                let f2 = ((f - (0.5 - wd / 2.0)) / wd).clamp(0.0, 1.0);
                let sc = screen(x as f32 - padpx as f32, y as f32 - padpx as f32, r, period);
                let up_tone = (lo_l as usize + 1).min(n - 1);
                tone[i] = if f2 > sc { up_tone as u8 } else { lo_l as u8 };
            }
        }
    }

    // Colour, ink on the hard levels.
    let mut out = blank(pw, ph, [0, 0, 0, 255]);
    for i in 0..pw * ph {
        let c = colours[tone[i] as usize];
        out.rgba[i * 4..i * 4 + 3].copy_from_slice(&c);
    }
    if r.ink > 0.0 {
        let rad = r.ink.ceil() as isize;
        for y in 0..ph {
            for x in 0..pw {
                let l0 = level[y * pw + x] as i32;
                let mut hit = false;
                'k: for dy in -rad..=rad {
                    let yy = y as isize + dy;
                    if yy < 0 || yy >= ph as isize {
                        continue;
                    }
                    for dx in -rad..=rad {
                        let xx = x as isize + dx;
                        if xx < 0 || xx >= pw as isize || (dx * dx + dy * dy) as f32 > r.ink * r.ink {
                            continue;
                        }
                        let l1 = level[yy as usize * pw + xx as usize] as i32;
                        let ok = match r.ink_side {
                            InkSide::Light => l0 - l1,
                            InkSide::Dark => l1 - l0,
                            InkSide::Both => (l0 - l1).abs(),
                        };
                        if ok >= r.ink_step as i32 {
                            hit = true;
                            break 'k;
                        }
                    }
                }
                if hit {
                    out.rgba[(y * pw + x) * 4..(y * pw + x) * 4 + 3].copy_from_slice(&r.ink_colour);
                }
            }
        }
    }
    let mut res = crop(&out, padpx, padpx, ow, oh);
    if r.border_ink > 0.0 {
        let b = r.border_ink.ceil() as usize;
        let mut line = |x: usize, y: usize| {
            let o = (y * ow + x) * 4;
            res.rgba[o..o + 3].copy_from_slice(&r.ink_colour);
        };
        for i in 0..b.min(oh) {
            if wv == Wrap::Clamp {
                for x in 0..ow {
                    line(x, i);
                    line(x, oh - 1 - i);
                }
            }
        }
        for i in 0..b.min(ow) {
            if wu == Wrap::Clamp {
                for y in 0..oh {
                    line(i, y);
                    line(ow - 1 - i, y);
                }
            }
        }
    }
    // Cut-outs stay cut out.
    if src.rgba.chunks_exact(4).any(|p| p[3] != 255) {
        for y in 0..oh {
            for x in 0..ow {
                res.rgba[(y * ow + x) * 4 + 3] = src.rgba[((y / s) * src.width + x / s) * 4 + 3];
            }
        }
    }
    res
}

/// Level the jump between a repeating texture's opposite edges: along every
/// row (and column) the two edge values are brought together, the
/// correction fading out linearly over `n` texels, so a texture that was
/// nearly but not quite seamless has no step to turn into a tone change.
pub fn level_seams(src: &Image, n: usize, wu: Wrap, wv: Wrap) -> Image {
    let mut out = src.clone();
    let (w, h) = (src.width, src.height);
    let mut fix = |len: usize, lines: usize, at: &dyn Fn(usize, usize) -> usize| {
        let n = n.min(len / 2);
        for l in 0..lines {
            for c in 0..3 {
                let (a, b) = (out.rgba[at(l, 0) * 4 + c] as f32, out.rgba[at(l, len - 1) * 4 + c] as f32);
                let r = (a - b) / 2.0;
                for x in 0..n {
                    let wgt = 1.0 - x as f32 / n as f32;
                    let (i, j) = (at(l, x) * 4 + c, at(l, len - 1 - x) * 4 + c);
                    out.rgba[i] = (out.rgba[i] as f32 - r * wgt).round().clamp(0.0, 255.0) as u8;
                    out.rgba[j] = (out.rgba[j] as f32 + r * wgt).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    };
    if wu == Wrap::Repeat && w >= 4 {
        fix(w, h, &|row, x| row * w + x);
    }
    if wv == Wrap::Repeat && h >= 4 {
        fix(h, w, &|col, y| y * w + col);
    }
    out
}

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

// ----------------------------------------------------------- the tinter

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TintMode {
    /// Gain of the mean colours, per channel.
    Ratio,
    /// The same gain, then the restyle's own brightness put back, so the
    /// tones and the ink keep their values and only the hue moves.
    Luma,
    /// One brightness gain for all channels (what a mean-colour match does).
    Mean,
}

impl TintMode {
    pub fn parse(s: &str) -> Option<TintMode> {
        Some(match s {
            "ratio" => TintMode::Ratio,
            "luma" => TintMode::Luma,
            "mean" => TintMode::Mean,
            _ => return None,
        })
    }
}

/// `base` (a restyled image) drawn in another key's colour: the gain from
/// the mean colour of `reference` (the original `base` restyles) to that
/// of `target` (the other key's original), `strength` 0..1 of the way.
/// Pixels that are the ink stay ink. Sizes of `target` and `reference`
/// need not match each other's or the base's.
pub fn tint(base: &Image, reference: &Image, target: &Image, mode: TintMode, strength: f32, ink: [u8; 3]) -> Image {
    let (mr, mt) = (mean_rgb(reference), mean_rgb(target));
    let gain: [f32; 3] = if mode == TintMode::Mean {
        let g = luma(mt[0], mt[1], mt[2]) / luma(mr[0], mr[1], mr[2]).max(1.0);
        [g; 3]
    } else {
        [mt[0] / mr[0].max(1.0), mt[1] / mr[1].max(1.0), mt[2] / mr[2].max(1.0)]
    };
    let gain = gain.map(|g| 1.0 + (g.clamp(0.2, 5.0) - 1.0) * strength);
    let mut out = base.clone();
    for p in out.rgba.chunks_exact_mut(4) {
        let d: i32 = (0..3).map(|c| (p[c] as i32 - ink[c] as i32).abs()).sum();
        if d < 12 {
            continue;
        }
        let v = [p[0] as f32 * gain[0], p[1] as f32 * gain[1], p[2] as f32 * gain[2]];
        let v = if mode == TintMode::Luma {
            let (y0, y1) = (luma(p[0] as f32, p[1] as f32, p[2] as f32), luma(v[0], v[1], v[2]).max(1.0));
            let k = y0 / y1;
            let v = [v[0] * k, v[1] * k, v[2] * k];
            // Out of gamut: desaturate towards the grey of the same
            // brightness rather than clip, so the brightness holds.
            let (mx, mn) = (v[0].max(v[1]).max(v[2]), v[0].min(v[1]).min(v[2]));
            let mut t = 1.0f32;
            if mx > 255.0 && mx > y0 {
                t = t.min((255.0 - y0) / (mx - y0));
            }
            if mn < 0.0 && mn < y0 {
                t = t.min(y0 / (y0 - mn));
            }
            [y0 + (v[0] - y0) * t, y0 + (v[1] - y0) * t, y0 + (v[2] - y0) * t]
        } else {
            v
        };
        for c in 0..3 {
            p[c] = v[c].round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

// ------------------------------------------------------------ seam check

/// One axis of the seam check.
#[derive(Clone, Debug, PartialEq)]
pub struct SeamAxis {
    pub wrap: Wrap,
    /// Mean difference (0-255) between the opposite edge lines (repeat), or between the edge line and the next (mirror).
    pub jump: f32,
    /// `jump` over the mean difference between neighbouring lines inside.
    pub ratio: f32,
    pub suspect: bool,
}

fn line_diff(img: &Image, a: (usize, usize), b: (usize, usize), vertical: bool) -> f32 {
    // vertical: columns a.0 and b.0 over all rows; else rows a.1 and b.1 over all columns.
    let n = if vertical { img.height } else { img.width };
    let mut s = 0f32;
    let mut cnt = 0f32;
    for i in 0..n {
        let (pa, pb) = if vertical { ((a.0, i), (b.0, i)) } else { ((i, a.1), (i, b.1)) };
        let (oa, ob) = ((pa.1 * img.width + pa.0) * 4, (pb.1 * img.width + pb.0) * 4);
        if img.rgba[oa + 3] == 0 && img.rgba[ob + 3] == 0 {
            continue;
        }
        s += (0..3).map(|c| (img.rgba[oa + c] as f32 - img.rgba[ob + c] as f32).abs()).sum::<f32>() / 3.0;
        cnt += 1.0;
    }
    if cnt == 0.0 {
        0.0
    } else {
        s / cnt
    }
}

pub fn seam_axis(img: &Image, vertical: bool, wrap: Wrap) -> SeamAxis {
    let n = if vertical { img.width } else { img.height };
    if n < 4 || wrap == Wrap::Clamp {
        return SeamAxis { wrap, jump: 0.0, ratio: 0.0, suspect: false };
    }
    let at = |i: usize| if vertical { (i, 0) } else { (0, i) };
    let typical = (0..n - 1).map(|i| line_diff(img, at(i), at(i + 1), vertical)).sum::<f32>() / (n - 1) as f32;
    let typical = typical.max(2.0);
    let (jump, edge_line) = match wrap {
        Wrap::Repeat => {
            let j = line_diff(img, at(0), at(n - 1), vertical);
            // An edge line unlike its inner neighbour, at either edge, is an
            // ink line or border drawn on a wrapped edge.
            let e = line_diff(img, at(0), at(1), vertical).max(line_diff(img, at(n - 1), at(n - 2), vertical));
            (j, e)
        }
        _ => {
            let e = line_diff(img, at(0), at(1), vertical).max(line_diff(img, at(n - 1), at(n - 2), vertical));
            (0.0, e)
        }
    };
    let worst = jump.max(edge_line);
    let ratio = worst / typical;
    SeamAxis { wrap, jump: worst, ratio, suspect: ratio > 2.5 && worst > 12.0 }
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    /// A tileable test picture: smooth bumps that wrap, 32x32.
    fn bumps() -> Image {
        let mut im = blank(32, 32, [0, 0, 0, 255]);
        for y in 0..32 {
            for x in 0..32 {
                let u = (x as f32 / 32.0 * std::f32::consts::TAU).sin();
                let v = (y as f32 / 32.0 * std::f32::consts::TAU * 2.0).cos();
                let l = (128.0 + 90.0 * (u + v) / 2.0) as u8;
                im.rgba[(y * 32 + x) * 4..(y * 32 + x) * 4 + 3].copy_from_slice(&[l, l, l]);
            }
        }
        im
    }

    fn recipe() -> Recipe {
        Recipe { ink: 1.0, tones: 3, ..Recipe::default() }
    }

    #[test]
    fn wrap_indices() {
        assert_eq!(wrap_idx(-1, 8, Wrap::Repeat), 7);
        assert_eq!(wrap_idx(-1, 8, Wrap::Mirror), 0);
        assert_eq!(wrap_idx(8, 8, Wrap::Mirror), 7);
        assert_eq!(wrap_idx(-3, 8, Wrap::Clamp), 0);
        assert_eq!(parse_wrap("rr,mr", false), (Wrap::Repeat, Wrap::Repeat));
        assert_eq!(parse_wrap("mm", false), (Wrap::Mirror, Wrap::Mirror));
        assert_eq!(parse_wrap("-", true), (Wrap::Clamp, Wrap::Clamp));
    }

    #[test]
    fn processed_repeat_texture_stays_seamless() {
        let out = style(&bumps(), &recipe(), Wrap::Repeat, Wrap::Repeat);
        assert_eq!((out.width, out.height), (128, 128));
        for vertical in [true, false] {
            let a = seam_axis(&out, vertical, Wrap::Repeat);
            assert!(!a.suspect, "seam {a:?}");
        }
    }

    #[test]
    fn halftone_repeats_exactly_over_the_texture() {
        let r = Recipe { halftone: 1.0, pitch: 4.0, ..recipe() };
        let out = style(&bumps(), &r, Wrap::Repeat, Wrap::Repeat);
        for vertical in [true, false] {
            let a = seam_axis(&out, vertical, Wrap::Repeat);
            assert!(!a.suspect, "halftone seam {a:?}");
        }
    }

    #[test]
    fn no_ink_on_wrapped_edges() {
        // One flat tone: no steps inside, so any ink is edge ink.
        let flat = blank(16, 16, [120, 120, 120, 255]);
        let ink = [0x11, 0x10, 0x18];
        let count = |im: &Image| im.rgba.chunks_exact(4).filter(|p| p[..3] == ink).count();
        let r =
            Recipe { ink: 2.0, border_ink: 2.0, palette: vec![[100, 100, 100], [200, 200, 200]], ..Recipe::default() };
        assert_eq!(count(&style(&flat, &r, Wrap::Repeat, Wrap::Repeat)), 0);
        assert_eq!(count(&style(&flat, &r, Wrap::Mirror, Wrap::Mirror)), 0);
        // Clamped: the hairline is there, on every edge.
        let c = style(&flat, &r, Wrap::Clamp, Wrap::Clamp);
        assert_eq!(c.rgba[..3], ink);
        let w = c.width;
        assert_eq!(c.rgba[((w / 2) * w + w - 1) * 4..][..3], ink);
        // Mixed: only the clamped axis's edges.
        let m = style(&flat, &r, Wrap::Repeat, Wrap::Clamp);
        assert_eq!(m.rgba[(w / 2) * 4..][..3], ink);
        assert_ne!(m.rgba[((w / 2) * w) * 4..][..3], ink);
    }

    #[test]
    fn ink_follows_a_tone_step_across_the_wrapped_edge() {
        // A dark band straddling the left/right edge: ink around it must be
        // continuous, found on both sides of the seam.
        let mut im = blank(16, 16, [230, 230, 230, 255]);
        for y in 0..16 {
            for x in [0usize, 1, 14, 15] {
                im.rgba[(y * 16 + x) * 4..(y * 16 + x) * 4 + 3].copy_from_slice(&[10, 10, 10]);
            }
        }
        let r = Recipe {
            ink: 1.0,
            blur: 0.0,
            median: 0,
            palette: vec![[100, 100, 100], [200, 200, 200]],
            ..Recipe::default()
        };
        let out = style(&im, &r, Wrap::Repeat, Wrap::Repeat);
        let ink = [0x11, 0x10, 0x18];
        let at = |x: usize, y: usize| out.rgba[(y * out.width + x) * 4..][..3].to_vec();
        // The band wraps across the seam (output x 56..64 and 0..8); ink
        // runs along both its light-side edges, none in the band's middle.
        let any = |r: std::ops::Range<usize>| r.into_iter().any(|x| at(x, 20) == ink.to_vec());
        assert!(any(5..12));
        assert!(any(52..59));
        assert!(!any(60..64));
        assert!(!any(0..3));
        assert!(!any(25..40));
    }

    #[test]
    fn leveling_removes_a_seam_step() {
        // a left-to-right ramp: the wrap from right to left is a big step
        let mut ramp = blank(32, 8, [0, 0, 0, 255]);
        for y in 0..8 {
            for x in 0..32 {
                let v = (x * 7) as u8;
                ramp.rgba[(y * 32 + x) * 4..][..3].copy_from_slice(&[v, v, v]);
            }
        }
        assert!(seam_axis(&ramp, true, Wrap::Repeat).suspect);
        let l = level_seams(&ramp, 8, Wrap::Repeat, Wrap::Repeat);
        assert!(!seam_axis(&l, true, Wrap::Repeat).suspect, "{:?}", seam_axis(&l, true, Wrap::Repeat));
        // the middle is untouched, mirror axes are left alone
        assert_eq!(l.rgba[16 * 4], ramp.rgba[16 * 4]);
        assert_eq!(level_seams(&ramp, 8, Wrap::Mirror, Wrap::Mirror), ramp);
    }

    #[test]
    fn despeckle_absorbs_small_regions() {
        let mut lv = vec![2u8; 100];
        lv[55] = 0;
        let ch = despeckle(&mut lv, 10, 10, 4);
        assert_eq!(lv[55], 2);
        assert!(ch[55]);
        // a big region stays
        let mut lv = vec![2u8; 100];
        for i in 40..60 {
            lv[i] = 0;
        }
        despeckle(&mut lv, 10, 10, 4);
        assert_eq!(lv[45], 0);
    }

    #[test]
    fn palette_ramps() {
        let p = ramp(&[[0, 0, 0], [200, 100, 50]], 3);
        assert_eq!(p[1], [100, 50, 25]);
        assert_eq!(ramp(&[[1, 2, 3], [4, 5, 6], [7, 8, 9]], 3), vec![[1, 2, 3], [4, 5, 6], [7, 8, 9]]);
    }

    #[test]
    fn recipe_file_layers_and_rejects_typos() {
        let f = RecipeFile::parse(
            "[default]\nscale = 2\nink = 1\n[material ash]\nink = 3\ntones = 4\n[key tex_a_8x8]\ntones = 5\n",
        )
        .unwrap();
        let r = f.resolve("ash", "tex_a_8x8").unwrap();
        assert_eq!((r.scale, r.ink, r.tones), (2, 3.0, 5));
        let r = f.resolve("wall", "x").unwrap();
        assert_eq!((r.ink, r.tones), (1.0, 3));
        assert!(RecipeFile::parse("[default]\ninc = 3\n").is_err());
        assert!(RecipeFile::parse("[default]\npalette = red\n").is_err());
        let r = RecipeFile::parse("# top\n[default]\nink_colour = #112233 # why\npalette = #000000 #ffffff\n")
            .unwrap()
            .resolve("", "")
            .unwrap();
        assert_eq!((r.ink_colour, r.palette.len()), ([0x11, 0x22, 0x33], 2));
    }

    #[test]
    fn tint_moves_hue_and_keeps_ink_and_luma() {
        let ink = [0x11, 0x10, 0x18];
        let mut base = blank(2, 1, [180, 180, 180, 255]);
        base.rgba[4..7].copy_from_slice(&ink);
        let refm = blank(1, 1, [100, 100, 100, 255]);
        let tgt = blank(1, 1, [150, 100, 70, 255]);
        let t = tint(&base, &refm, &tgt, TintMode::Luma, 1.0, ink);
        assert_eq!(t.rgba[4..7], ink);
        assert!(t.rgba[0] > t.rgba[1] && t.rgba[1] > t.rgba[2], "{:?}", &t.rgba[..4]);
        let y = |p: &[u8]| luma(p[0] as f32, p[1] as f32, p[2] as f32);
        assert!((y(&t.rgba[..4]) - 180.0).abs() < 3.0);
        // Mean mode keeps grey grey.
        let m = tint(&base, &refm, &blank(1, 1, [200, 200, 200, 255]), TintMode::Mean, 1.0, ink);
        assert_eq!(m.rgba[0], m.rgba[2]);
        assert!(m.rgba[0] > 180);
        // strength 0 is the base
        assert_eq!(tint(&base, &refm, &tgt, TintMode::Ratio, 0.0, ink), base);
    }

    #[test]
    fn seam_check_flags_an_edge_line_and_a_jump() {
        let ok = style(&bumps(), &recipe(), Wrap::Repeat, Wrap::Repeat);
        assert!(!seam_axis(&ok, true, Wrap::Repeat).suspect);
        // ink line on the top rows, as the first slice's ground had
        let mut bad = ok.clone();
        for x in 0..bad.width {
            for y in 0..3 {
                bad.rgba[(y * bad.width + x) * 4..][..3].copy_from_slice(&[0, 0, 0]);
            }
        }
        let h = seam_axis(&bad, false, Wrap::Repeat);
        assert!(h.suspect, "{h:?}");
        assert!(!seam_axis(&bad, true, Wrap::Repeat).suspect);
        // a left-right jump
        let mut jump = blank(16, 16, [0, 0, 0, 255]);
        for y in 0..16 {
            for x in 0..16 {
                let v = (x * 16) as u8;
                jump.rgba[(y * 16 + x) * 4..][..3].copy_from_slice(&[v, v, v]);
            }
        }
        assert!(seam_axis(&jump, true, Wrap::Repeat).suspect);
        // a clamped axis is never checked
        assert!(!seam_axis(&jump, true, Wrap::Clamp).suspect);
    }

    #[test]
    fn resize_round_trips_a_flat_colour_and_nearest_enlarges() {
        let im = blank(3, 2, [10, 20, 30, 255]);
        let big = resize(&im, 12, 8);
        assert_eq!(big.rgba[..4], [10, 20, 30, 255]);
        assert_eq!(resize(&big, 3, 2), im);
        let n = nearest(&im, 2);
        assert_eq!((n.width, n.height), (6, 4));
    }
}
