//! `texpack-upscale`: a texture dump (`run --dump-textures`) through an
//! external upscaler, or a built-in resampler, into a texture pack. No ROM
//! needed. docs/texture-packs.md ("Batch upscaling") describes the steps:
//! transparent pixels filled from their neighbours so the upscaler only sees
//! opaque RGB, borders padded the way the renderer wraps the texture and
//! cropped after, the original's low-frequency colour put back (`--colour
//! match`), alpha scaled here and cut at 0.5 like the renderer.

use model2::enhance::{address, Address, Rgba};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

/// Source pixels of context on each side of a texture (ESRGAN's receptive
/// field is wide) and of a tile.
const PAD_TEXTURE: usize = 16;
const PAD_TILE: usize = 8;
/// Smallest side handed to an external upscaler: realesrgan-ncnn-vulkan
/// makes garbage of 2-6 pixel inputs and smears 8-12 (tried with all its
/// models); smaller images get more padding.
const MIN_INPUT: usize = 16;
/// Bumped when the output for the same settings changes, so packs made by
/// an older version are not taken as up to date.
const VERSION: u32 = 1;
/// Width (Gaussian sigma, in source texels) of the low-pass that colour
/// matching compares: wide enough to leave the upscaler's detail alone,
/// narrow enough to follow the texture's own colour changes.
const MATCH_SIGMA: f32 = 2.0;
const RECORD: &str = "upscale.txt";

/// How tile patterns (8x8, HUD and text) are scaled: ESRGAN-style models
/// do badly on them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileMode {
    /// Through the upscaler, like textures.
    Model,
    /// The built-in resampler.
    Builtin,
    /// Pixel copies: as sharp as the original.
    Nearest,
    /// Not in the pack: drawn as the original.
    Skip,
}

impl TileMode {
    fn parse(s: &str) -> Option<TileMode> {
        Some(match s {
            "model" => TileMode::Model,
            "builtin" => TileMode::Builtin,
            "nearest" => TileMode::Nearest,
            "skip" => TileMode::Skip,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            TileMode::Model => "model",
            TileMode::Builtin => "builtin",
            TileMode::Nearest => "nearest",
            TileMode::Skip => "skip",
        }
    }
}

/// What is done about the upscaler's colour drift.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colour {
    /// The upscaler's colours as they come.
    Keep,
    /// The original's low-frequency colour put back (`match_colour`).
    Match,
}

impl Colour {
    fn parse(s: &str) -> Option<Colour> {
        Some(match s {
            "keep" => Colour::Keep,
            "match" => Colour::Match,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Colour::Keep => "keep",
            Colour::Match => "match",
        }
    }
}

pub struct Options {
    pub scale: usize,
    /// Command template with `{in}`, `{out}`, `{scale}`; None: built-in.
    pub upscaler: Option<String>,
    pub tiles: TileMode,
    pub colour: Colour,
    pub min_uses: u64,
    /// Longest side of a pack image; larger results are scaled down to the
    /// largest whole factor that fits.
    pub max_side: usize,
    /// Mean shift per channel (0-255) above which an image is reported.
    pub drift: f32,
    pub work: PathBuf,
    pub force: bool,
    pub keep_work: bool,
    /// Seconds the upscaler may go without writing a file before it is
    /// stopped.
    pub stall: u64,
}

impl Options {
    /// What the output depends on, recorded in the pack.
    fn settings(&self) -> String {
        format!(
            "scale={}\tupscaler={}\ttiles={}\tcolour={}\tmax_side={}\tversion={VERSION}",
            self.scale,
            self.upscaler.as_deref().unwrap_or("builtin"),
            self.tiles.name(),
            self.colour.name(),
            self.max_side
        )
    }
}

/// One dump from the index.
#[derive(Debug, PartialEq)]
pub(crate) struct Dump {
    pub(crate) name: String,
    pub(crate) uses: u64,
    /// Addressing per axis (u, v) for padding and alpha: textures wrap or
    /// mirror, tiles clamp.
    pub(crate) modes: [Address; 2],
    /// A texture drawn both repeated and mirrored on an axis (padded
    /// mirrored there).
    pub(crate) mixed: bool,
    pub(crate) tile: bool,
    /// The texture's surface key (`surf=`, 16 hex digits), in indexes that
    /// have it.
    pub(crate) surf: Option<String>,
}

/// The dumps listed in a dump's index.txt. A texture's padding comes from
/// its `wrap=` field (every way it was drawn) or, in indexes written before
/// that field, from the first use's header (h0 bits 8 and 9). An axis drawn
/// only repeated pads repeated; mirrored, unknown or both pads mirrored:
/// that is exact at a mirrored edge, and at a repeated one still continues
/// the texture's own edge rather than bringing in the opposite side.
pub(crate) fn parse_index(text: &str) -> Vec<Dump> {
    let mut out = vec![];
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let mut fields = line.split('\t');
        let name = fields.next().unwrap_or("").trim().to_string();
        let tile = name.starts_with("tile_");
        if !tile && !name.starts_with("tex_") {
            continue;
        }
        let kv: BTreeMap<&str, &str> = fields.filter_map(|f| f.split_once('=')).collect();
        let uses = kv.get("uses").and_then(|u| u.parse().ok()).unwrap_or(1);
        // Ways seen per axis: [repeat, mirror].
        let mut seen = [[false; 2]; 2];
        if let Some(w) = kv.get("wrap") {
            for way in w.split(',') {
                for (axis, c) in way.chars().take(2).enumerate() {
                    seen[axis][(c == 'm') as usize] = true;
                }
            }
        } else if let Some(h0) =
            kv.get("header").and_then(|h| h.split(',').next()).and_then(|h| u16::from_str_radix(h, 16).ok())
        {
            for (axis, s) in seen.iter_mut().enumerate() {
                s[(h0 >> (8 + axis) & 1) as usize] = true;
            }
        }
        let mode = |s: [bool; 2]| if s == [true, false] { Address::Repeat } else { Address::Mirror };
        let modes = if tile { [Address::Clamp; 2] } else { [mode(seen[0]), mode(seen[1])] };
        let mixed = !tile && seen.iter().any(|s| s[0] && s[1]);
        let surf = kv.get("surf").map(|s| s.to_string());
        out.push(Dump { name, uses, modes, mixed, tile, surf });
    }
    out
}

pub(crate) fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

fn opaque(img: &Rgba, i: usize) -> bool {
    img.data[i * 4 + 3] >= 128
}

/// The 8 neighbours of pixel `i`, found with the image's addressing.
fn neighbours(img: &Rgba, i: usize, modes: [Address; 2]) -> impl Iterator<Item = usize> + '_ {
    let (x, y) = ((i % img.width) as i32, (i / img.width) as i32);
    (-1..=1).flat_map(move |dy| {
        (-1..=1)
            .filter(move |&dx| dx != 0 || dy != 0)
            .map(move |dx| address(y + dy, img.height, modes[1]) * img.width + address(x + dx, img.width, modes[0]))
    })
}

/// Give transparent pixels (alpha < 128) the colour of their opaque
/// surroundings, layer by layer outwards: each takes the mean of its
/// already known neighbours. The upscaler then never sees the meaningless
/// colour under them (texel 15, pen 0), so it cannot bleed into the edge
/// pixels that stay opaque. False if nothing is opaque.
pub(crate) fn fill_transparent(img: &mut Rgba, modes: [Address; 2]) -> bool {
    let n = img.width * img.height;
    let mut known: Vec<bool> = (0..n).map(|i| opaque(img, i)).collect();
    if !known.contains(&true) {
        return false;
    }
    let mut queued = known.clone();
    let mut frontier: Vec<usize> =
        (0..n).filter(|&i| !known[i] && neighbours(img, i, modes).any(|j| known[j])).collect();
    for &i in &frontier {
        queued[i] = true;
    }
    while !frontier.is_empty() {
        let colours: Vec<[u8; 3]> = frontier
            .iter()
            .map(|&i| {
                let (mut sum, mut count) = ([0u32; 3], 0);
                for j in neighbours(img, i, modes).filter(|&j| known[j]) {
                    for (s, &v) in sum.iter_mut().zip(&img.data[j * 4..j * 4 + 3]) {
                        *s += v as u32;
                    }
                    count += 1;
                }
                sum.map(|s| ((s + count / 2) / count) as u8)
            })
            .collect();
        for (&i, c) in frontier.iter().zip(&colours) {
            img.data[i * 4..i * 4 + 3].copy_from_slice(c);
            known[i] = true;
        }
        let mut next = vec![];
        for &i in &frontier {
            for j in neighbours(img, i, modes) {
                if !queued[j] {
                    queued[j] = true;
                    next.push(j);
                }
            }
        }
        frontier = next;
    }
    true
}

/// `img` with `pad[axis]` pixels added on both sides of each axis, taken
/// with the renderer's own addressing (`enhance::address`).
pub(crate) fn pad(img: &Rgba, pad: [usize; 2], modes: [Address; 2]) -> Rgba {
    let (w, h) = (img.width + 2 * pad[0], img.height + 2 * pad[1]);
    let mut data = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let sy = address(y as i32 - pad[1] as i32, img.height, modes[1]);
        for x in 0..w {
            let sx = address(x as i32 - pad[0] as i32, img.width, modes[0]);
            let o = (sy * img.width + sx) * 4;
            data.extend_from_slice(&img.data[o..o + 4]);
        }
    }
    Rgba { width: w, height: h, data }
}

pub(crate) fn crop(img: &Rgba, x0: usize, y0: usize, w: usize, h: usize) -> Rgba {
    let mut data = Vec::with_capacity(w * h * 4);
    for y in y0..y0 + h {
        let o = (y * img.width + x0) * 4;
        data.extend_from_slice(&img.data[o..o + w * 4]);
    }
    Rgba { width: w, height: h, data }
}

/// Catmull-Rom weights for the taps at -1, 0, 1, 2 from a sample `t` past tap 0.
fn cubic(t: f32) -> [f32; 4] {
    let (t2, t3) = (t * t, t * t * t);
    [(-t3 + 2.0 * t2 - t) / 2.0, (3.0 * t3 - 5.0 * t2 + 2.0) / 2.0, (-3.0 * t3 + 4.0 * t2 + t) / 2.0, (t3 - t2) / 2.0]
}

/// The built-in resampler: separable Catmull-Rom (bicubic) to `s` times
/// the size, edges clamped (the image is padded already). Pixel centres
/// stay aligned, as the renderer's lighting needs. Alpha is left 255.
fn upscale_builtin(img: &Rgba, s: usize) -> Rgba {
    let (w, h, nw, nh) = (img.width, img.height, img.width * s, img.height * s);
    let taps = |o: usize, size: usize| {
        let x = (o as f32 + 0.5) / s as f32 - 0.5;
        let x0 = x.floor();
        let wt = cubic(x - x0);
        let idx = [-1, 0, 1, 2].map(|d| (x0 as i32 + d).clamp(0, size as i32 - 1) as usize);
        (idx, wt)
    };
    let mut rows = vec![0f32; nw * h * 3];
    for ox in 0..nw {
        let (idx, wt) = taps(ox, w);
        for y in 0..h {
            for c in 0..3 {
                rows[(y * nw + ox) * 3 + c] =
                    (0..4).map(|k| wt[k] * img.data[(y * w + idx[k]) * 4 + c] as f32).sum::<f32>();
            }
        }
    }
    let mut data = vec![255u8; nw * nh * 4];
    for oy in 0..nh {
        let (idx, wt) = taps(oy, h);
        for x in 0..nw {
            for c in 0..3 {
                let v: f32 = (0..4).map(|k| wt[k] * rows[(idx[k] * nw + x) * 3 + c]).sum();
                data[(oy * nw + x) * 4 + c] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Rgba { width: nw, height: nh, data }
}

/// Pixel copies at `s` times the size (alpha too).
fn upscale_nearest(img: &Rgba, s: usize) -> Rgba {
    let (nw, nh) = (img.width * s, img.height * s);
    let mut data = Vec::with_capacity(nw * nh * 4);
    for y in 0..nh {
        for x in 0..nw {
            let o = ((y / s) * img.width + x / s) * 4;
            data.extend_from_slice(&img.data[o..o + 4]);
        }
    }
    Rgba { width: nw, height: nh, data }
}

/// Area-average RGB down to `nw` x `nh` (for the size cap).
fn shrink(img: &Rgba, nw: usize, nh: usize) -> Rgba {
    // Source pixels and weights covering each output pixel on an axis.
    let spans = |from: usize, to: usize| -> Vec<Vec<(usize, f32)>> {
        let r = from as f32 / to as f32;
        (0..to)
            .map(|o| {
                let (a, b) = (o as f32 * r, (o + 1) as f32 * r);
                (a.floor() as usize..(b.ceil() as usize).min(from))
                    .map(|i| (i, (b.min(i as f32 + 1.0) - a.max(i as f32)) / r))
                    .collect()
            })
            .collect()
    };
    let (sx, sy) = (spans(img.width, nw), spans(img.height, nh));
    let mut data = vec![255u8; nw * nh * 4];
    for (oy, ys) in sy.iter().enumerate() {
        for (ox, xs) in sx.iter().enumerate() {
            let mut c = [0f32; 3];
            for &(y, wy) in ys {
                for &(x, wx) in xs {
                    let o = (y * img.width + x) * 4;
                    for (k, c) in c.iter_mut().enumerate() {
                        *c += wy * wx * img.data[o + k] as f32;
                    }
                }
            }
            for (k, c) in c.iter().enumerate() {
                data[(oy * nw + ox) * 4 + k] = c.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Rgba { width: nw, height: nh, data }
}

/// The source's alpha at `s` times the size: sampled bilinearly with its
/// addressing and cut at 0.5, the renderer's rule for original translucent
/// texels and for pack images, so the cut-out keeps its shape.
fn alpha_up(src: &Rgba, s: usize, modes: [Address; 2], out: &mut Rgba) {
    for y in 0..out.height {
        for x in 0..out.width {
            let (_, a) = src.sample((x as f32 + 0.5) / s as f32, (y as f32 + 0.5) / s as f32, modes[0], modes[1]);
            out.data[(y * out.width + x) * 4 + 3] = if a >= 0.5 { 255 } else { 0 };
        }
    }
}

/// Mean RGB of the opaque pixels.
fn mean_opaque(img: &Rgba) -> Option<[f32; 3]> {
    let (mut sum, mut n) = ([0f64; 3], 0usize);
    for px in img.data.as_chunks::<4>().0.iter().filter(|p| p[3] >= 128) {
        for (s, &v) in sum.iter_mut().zip(px) {
            *s += v as f64;
        }
        n += 1;
    }
    (n > 0).then(|| sum.map(|s| (s / n as f64) as f32))
}

/// Largest per-channel difference of the opaque pixels' mean colours.
fn drift(src: &Rgba, out: &Rgba) -> f32 {
    match (mean_opaque(src), mean_opaque(out)) {
        (Some(a), Some(b)) => (0..3).map(|c| (a[c] - b[c]).abs()).fold(0.0, f32::max),
        _ => 0.0,
    }
}

/// Put the source's low-frequency colour back into `out` (`s` times its
/// size, alpha already put back) and keep the upscaler's detail: out +=
/// lowpass(src) - lowpass(out). Both low-passes are Gaussian (`MATCH_SIGMA`
/// source texels) at the source's resolution, `out` averaged per `s` x `s`
/// block first, and share one weight per texel: its opaque share of the
/// block, zero where the source is transparent, so transparent pixels
/// (whose colour means nothing) pull nothing on either side. They follow
/// the texture's addressing, so a repeated or mirrored texture's
/// correction tiles too; a clamped image (a tile) has nothing past its
/// edge, rather than its edge texel repeated. The blurred sums are
/// interpolated bilinearly to `out`'s pixels before dividing, so the
/// correction is smooth and adds no edges of its own. A flat shift comes
/// back exactly; zero-mean detail inside a block is left as it is.
fn match_colour(src: &Rgba, out: &mut Rgba, s: usize, modes: [Address; 2]) {
    let (w, h) = (src.width, src.height);
    debug_assert_eq!((out.width, out.height), (w * s, h * s));
    let n = (s * s) as f32;
    // Per texel, times its weight: the source's RGB, `out`'s block mean
    // (of its opaque pixels) RGB, and the weight.
    let mut acc = vec![[0f32; 7]; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            if src.data[i + 3] < 128 {
                continue;
            }
            let a = &mut acc[y * w + x];
            for by in 0..s {
                for bx in 0..s {
                    let o = ((y * s + by) * out.width + x * s + bx) * 4;
                    if out.data[o + 3] >= 128 {
                        for (a, &v) in a[3..6].iter_mut().zip(&out.data[o..o + 3]) {
                            *a += v as f32 / n;
                        }
                        a[6] += 1.0 / n;
                    }
                }
            }
            for c in 0..3 {
                a[c] = src.data[i + c] as f32 * a[6];
            }
        }
    }
    // Separable Gaussian with the addressing.
    let r = (3.0 * MATCH_SIGMA).ceil() as i32;
    let k: Vec<f32> = (-r..=r).map(|d| (-(d * d) as f32 / (2.0 * MATCH_SIGMA * MATCH_SIGMA)).exp()).collect();
    let blur = |e: &[[f32; 7]], horizontal: bool| -> Vec<[f32; 7]> {
        (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as i32, (i / w) as i32);
                let (c, size, mode) = if horizontal { (x, w, modes[0]) } else { (y, h, modes[1]) };
                let mut sum = [0f32; 7];
                for (d, &kw) in (-r..=r).zip(&k) {
                    if mode == Address::Clamp && !(0..size as i32).contains(&(c + d)) {
                        continue;
                    }
                    let t = address(c + d, size, mode);
                    let j = if horizontal { y as usize * w + t } else { t * w + x as usize };
                    for (s, v) in sum.iter_mut().zip(&e[j]) {
                        *s += kw * v;
                    }
                }
                sum
            })
            .collect()
    };
    let acc = blur(&blur(&acc, true), false);
    // Bilinear to `out`'s pixels, centres aligned.
    let taps = |o: usize, size: usize, mode: Address| {
        let t = (o as f32 + 0.5) / s as f32 - 0.5;
        let t0 = t.floor();
        ([address(t0 as i32, size, mode), address(t0 as i32 + 1, size, mode)], t - t0)
    };
    for oy in 0..out.height {
        let ([y0, y1], fy) = taps(oy, h, modes[1]);
        for ox in 0..out.width {
            let ([x0, x1], fx) = taps(ox, w, modes[0]);
            let mut v = [0f32; 7];
            for (j, wt) in [
                (y0 * w + x0, (1.0 - fx) * (1.0 - fy)),
                (y0 * w + x1, fx * (1.0 - fy)),
                (y1 * w + x0, (1.0 - fx) * fy),
                (y1 * w + x1, fx * fy),
            ] {
                for (v, a) in v.iter_mut().zip(&acc[j]) {
                    *v += wt * a;
                }
            }
            if v[6] <= 1e-6 {
                continue;
            }
            let o = (oy * out.width + ox) * 4;
            for c in 0..3 {
                let fix = (v[c] - v[3 + c]) / v[6];
                out.data[o + c] = (out.data[o + c] as f32 + fix).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// How one dump is scaled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum How {
    External,
    Builtin,
    Nearest,
}

struct Job {
    dump: Dump,
    how: How,
    src_hash: u64,
}

/// Padding for an image of `w` x `h`: the context, and enough to reach
/// `MIN_INPUT` for an external upscaler.
fn padding(w: usize, h: usize, tile: bool, how: How) -> [usize; 2] {
    let p = if tile { PAD_TILE } else { PAD_TEXTURE };
    let min = if how == How::External { MIN_INPUT } else { 0 };
    [p.max(min.saturating_sub(w).div_ceil(2)), p.max(min.saturating_sub(h).div_ceil(2))]
}

/// The pack image from the source and the upscaled padded RGB: checks the
/// size, crops, caps the size (`fit`: the scale kept, at most `scale`),
/// matches colour (`Colour::Match`), then puts back alpha. Returns the
/// image and its mean colour shift before and after matching.
fn finish(
    src: &Rgba,
    up: Rgba,
    dump: &Dump,
    pad: [usize; 2],
    scale: usize,
    fit: usize,
    colour: Colour,
) -> Result<(Rgba, [f32; 2]), String> {
    let (pw, ph) = ((src.width + 2 * pad[0]) * scale, (src.height + 2 * pad[1]) * scale);
    if (up.width, up.height) != (pw, ph) {
        return Err(format!(
            "upscaler wrote {}x{}, expected {pw}x{ph} ({scale}x of {}x{}): does SCALE match the model?",
            up.width,
            up.height,
            pw / scale,
            ph / scale
        ));
    }
    let mut out = crop(&up, pad[0] * scale, pad[1] * scale, src.width * scale, src.height * scale);
    if fit < scale {
        out = shrink(&out, src.width * fit, src.height * fit);
    }
    put_alpha(src, &mut out, dump, fit);
    let before = drift(src, &out);
    if colour == Colour::Match {
        match_colour(src, &mut out, fit, dump.modes);
        // A tile's transparent pixels: pen 0's colour again.
        put_alpha(src, &mut out, dump, fit);
    }
    let after = drift(src, &out);
    Ok((out, [before, after]))
}

/// Alpha from the source (opaque sources stay opaque); a tile's
/// transparent pixels get back pen 0's colour, as in the dump.
fn put_alpha(src: &Rgba, out: &mut Rgba, dump: &Dump, s: usize) {
    if src.data.as_chunks::<4>().0.iter().all(|p| p[3] == 255) {
        out.data.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p[3] = 255);
        return;
    }
    alpha_up(src, s, dump.modes, out);
    if dump.tile {
        if let Some(pen0) = src.data.as_chunks::<4>().0.iter().find(|p| p[3] < 128) {
            let pen0 = [pen0[0], pen0[1], pen0[2]];
            for p in out.data.as_chunks_mut::<4>().0.iter_mut().filter(|p| p[3] == 0) {
                p[..3].copy_from_slice(&pen0);
            }
        }
    }
}

pub(crate) fn decode(path: &Path) -> Result<Rgba, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let img = png::decode(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Rgba { width: img.width, height: img.height, data: img.rgba })
}

/// Write through a temporary name and rename, so an interrupted run never
/// leaves a half-written file that looks finished.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Size and modification time of a pack file, to see it is still ours.
pub(crate) fn stamp(path: &Path) -> Option<String> {
    let m = std::fs::metadata(path).ok()?;
    let t = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(format!("{},{}", m.len(), t.as_nanos()))
}

/// Run `f` over `items` on every core.
pub(crate) fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<Option<R>>> = Mutex::new((0..items.len()).map(|_| None).collect());
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(items.len().max(1));
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(item) = items.get(i) else { break };
                let r = f(item);
                out.lock().unwrap()[i] = Some(r);
            });
        }
    });
    out.into_inner().unwrap().into_iter().map(|r| r.unwrap()).collect()
}

/// Run `cmd` with `sh -c`, its output to `log`. Stopped (with what it
/// started) when no new file has appeared in `out` for `stall` seconds:
/// realesrgan-ncnn-vulkan was seen to spin forever on its last image.
pub(crate) fn run_watched(tool: &str, cmd: &str, log: &Path, out: &Path, stall: u64) -> Result<(), String> {
    let file = std::fs::File::create(log).map_err(|e| format!("{}: {e}", log.display()))?;
    let err = file.try_clone().map_err(|e| e.to_string())?;
    let mut c = Command::new("sh");
    c.arg("-c").arg(cmd).stdout(file).stderr(err);
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut c, 0);
    let mut child = c.spawn().map_err(|e| format!("{tool}: {e}"))?;
    let count = || std::fs::read_dir(out).map_or(0, |d| d.count());
    let (mut seen, mut since) = (count(), Instant::now());
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return if status.success() { Ok(()) } else { Err(format!("{tool} exited with {status}")) };
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        let n = count();
        if n != seen {
            (seen, since) = (n, Instant::now());
        } else if since.elapsed().as_secs() >= stall {
            // The tool's whole process group (it was made the leader of its
            // own), signalled directly. Not `kill -KILL -PID`: on the CI
            // runner (Ubuntu's kill) this test took the runner's own agent
            // down with it, every time (2026-11-25).
            #[cfg(unix)]
            if let Ok(pgid) = libc::pid_t::try_from(child.id()) {
                if pgid > 1 {
                    // SAFETY: killpg only sends a signal; pgid is our child's own group.
                    unsafe { libc::killpg(pgid, libc::SIGKILL) };
                }
            }
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{tool} wrote nothing for {stall} s: stopped (rerun to retry what is missing)"));
        }
    }
}

/// `template` with `{in}`, `{out}` (shell-quoted) and `{scale}` filled in.
pub(crate) fn expand(template: &str, input: &Path, output: &Path, scale: usize) -> String {
    let q = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', r"'\''"));
    template.replace("{in}", &q(input)).replace("{out}", &q(output)).replace("{scale}", &scale.to_string())
}

/// The pack's record: settings, then a line per image made, with its
/// source's hash and its own stamp. Later lines win.
struct Record {
    settings: Option<String>,
    images: BTreeMap<String, (u64, String)>,
}

fn read_record(dest: &Path) -> Record {
    let mut r = Record { settings: None, images: BTreeMap::new() };
    let Ok(text) = std::fs::read_to_string(dest.join(RECORD)) else { return r };
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        if let Some(s) = line.strip_prefix("settings\t") {
            r.settings = Some(s.to_string());
        } else if let [name, src, out] = line.split('\t').collect::<Vec<_>>()[..] {
            if let (Some(h), Some(o)) = (src.strip_prefix("src="), out.strip_prefix("out=")) {
                if let Ok(h) = u64::from_str_radix(h, 16) {
                    r.images.insert(name.to_string(), (h, o.to_string()));
                }
            }
        }
    }
    r
}

const RECORD_HEAD: &str =
    "# reModel texpack-upscale record: the settings, then per image its source's hash and the pack\n\
     # file's size and time. Keeps reruns to new or changed dumps; delete it to redo everything.\n";

fn write_record(dest: &Path, settings: &str, images: &BTreeMap<String, (u64, String)>) -> Result<(), String> {
    let mut text = format!("{RECORD_HEAD}settings\t{settings}\n");
    for (name, (h, o)) in images {
        let _ = writeln!(text, "{name}\tsrc={h:016x}\tout={o}");
    }
    write_atomic(&dest.join(RECORD), text.as_bytes())
}

/// The pack's index.txt: every image's file name, one a line (`#` lines
/// are comments). The browser build reads it to know which files exist.
pub(crate) fn write_pack_index(dest: &Path) -> Result<usize, String> {
    let mut names: Vec<String> = std::fs::read_dir(dest)
        .map_err(|e| format!("{}: {e}", dest.display()))?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".png") && !n.starts_with('.'))
        .collect();
    names.sort();
    let mut text = String::from("# reModel texture pack: the image files in this directory, one a line.\n");
    for n in &names {
        let _ = writeln!(text, "{n}");
    }
    write_atomic(&dest.join("index.txt"), text.as_bytes())?;
    Ok(names.len())
}

/// What happened to one job.
enum Done {
    Made {
        name: String,
        src_hash: u64,
        /// Mean colour shift before and after matching.
        drift: [f32; 2],
    },
    Empty,
    Failed(String),
    /// Prepared for the external upscaler: padding.
    Queued([usize; 2]),
}

/// Upscale the dumps in `src` (with its index.txt) into the pack `dest`.
/// `game` only goes into the commands printed at the end.
pub fn run(src: &Path, dest: &Path, o: &Options, game: &str) -> Result<(), String> {
    let start = Instant::now();
    let index_path = src.join("index.txt");
    let index = std::fs::read_to_string(&index_path)
        .map_err(|e| format!("{}: {e} (make one with `make dump-textures`)", index_path.display()))?;
    let dumps = parse_index(&index);
    if o.scale < 1 || o.max_side < 1 {
        return Err("--scale and --max-side must be at least 1".into());
    }
    std::fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let settings = o.settings();
    let mut record = read_record(dest);
    if record.settings.as_ref().is_some_and(|s| *s != settings) && !o.force {
        return Err(format!(
            "{} was made with other settings ({}); use another TEXPACK= or FORCE=1 to redo it all",
            dest.display(),
            record.settings.unwrap_or_default().replace('\t', " ")
        ));
    }
    if o.force {
        record.images.clear();
    }
    println!(
        "texpack-upscale: {} -> {}, {}x (sides up to {}), upscaler: {}, tiles: {}, colour: {}",
        src.display(),
        dest.display(),
        o.scale,
        o.max_side,
        o.upscaler.as_deref().unwrap_or("built-in bicubic"),
        o.tiles.name(),
        o.colour.name()
    );

    // Sort the dumps into jobs.
    let (total, mut below, mut skipped, mut current, mut kept, mut missing, mut mixed) =
        (dumps.len(), 0, 0, 0, 0, 0, 0);
    let mut jobs = vec![];
    for d in dumps {
        if d.uses < o.min_uses {
            below += 1;
            continue;
        }
        let how = match (d.tile, o.tiles, &o.upscaler) {
            (true, TileMode::Skip, _) => {
                skipped += 1;
                continue;
            }
            (true, TileMode::Nearest, _) => How::Nearest,
            (true, TileMode::Builtin, _) | (_, _, None) => How::Builtin,
            _ => How::External,
        };
        let Ok(bytes) = std::fs::read(src.join(format!("{}.png", d.name))) else {
            missing += 1;
            continue;
        };
        let src_hash = fnv(&bytes);
        let out = dest.join(format!("{}.png", d.name));
        if out.exists() {
            match record.images.get(&d.name) {
                Some((h, st)) if Some(st) == stamp(&out).as_ref() && *h == src_hash => {
                    current += 1;
                    continue;
                }
                Some((_, st)) if Some(st) == stamp(&out).as_ref() => {} // dump changed: redo
                _ if !o.force => {
                    kept += 1; // made or edited by hand
                    continue;
                }
                _ => {}
            }
        }
        mixed += d.mixed as usize;
        jobs.push(Job { dump: d, how, src_hash });
    }
    let (tex, tiles) = (jobs.iter().filter(|j| !j.dump.tile).count(), jobs.iter().filter(|j| j.dump.tile).count());
    println!(
        "{total} dumps: {current} up to date, {kept} kept (not made by this tool), {below} below MIN_USES, \
         {skipped} tiles skipped, {missing} missing; {} to do ({tex} textures, {tiles} tiles; \
         {mixed} textures drawn both repeated and mirrored, padded mirrored)",
        jobs.len()
    );

    let (work_in, work_out) = (o.work.join("in"), o.work.join("out"));
    let external = jobs.iter().any(|j| j.how == How::External);
    if external {
        let _ = std::fs::remove_dir_all(&o.work);
        for d in [&work_in, &work_out] {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
    }
    let log = Mutex::new(
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dest.join(RECORD))
            .map_err(|e| format!("{}: {e}", dest.join(RECORD).display()))?,
    );
    {
        let mut f = log.lock().unwrap();
        let _ = writeln!(f, "{RECORD_HEAD}settings\t{settings}");
    }
    let fit = |w: usize, h: usize| o.scale.min((o.max_side / w.max(h)).max(1));
    // Write a made image and log it at once, so an interrupted run keeps it.
    let save = |job: &Job, img: Rgba, drift: [f32; 2]| -> Done {
        let path = dest.join(format!("{}.png", job.dump.name));
        if let Err(e) = write_atomic(&path, &png::encode_rgba(img.width, img.height, &img.data)) {
            return Done::Failed(e);
        }
        let st = stamp(&path).unwrap_or_default();
        let _ = writeln!(log.lock().unwrap(), "{}\tsrc={:016x}\tout={st}", job.dump.name, job.src_hash);
        Done::Made { name: job.dump.name.clone(), src_hash: job.src_hash, drift }
    };

    // Prepare: built-in and nearest jobs are done here; external ones are
    // written, filled and padded, as opaque RGB for the upscaler.
    let t0 = Instant::now();
    let mut done = par_map(&jobs, |job| {
        let d = &job.dump;
        let mut img = match decode(&src.join(format!("{}.png", d.name))) {
            Ok(i) => i,
            Err(e) => return Done::Failed(e),
        };
        let s = fit(img.width, img.height);
        if job.how == How::Nearest {
            if !img.data.as_chunks::<4>().0.iter().any(|p| p[3] >= 128) {
                return Done::Empty;
            }
            return save(job, upscale_nearest(&img, s), [0.0; 2]);
        }
        let source = img.clone();
        if !fill_transparent(&mut img, d.modes) {
            return Done::Empty;
        }
        let p = padding(img.width, img.height, d.tile, job.how);
        let padded = pad(&img, p, d.modes);
        if job.how == How::Builtin {
            return match finish(&source, upscale_builtin(&padded, s), d, p, s, s, o.colour) {
                Ok((out, drift)) => save(job, out, drift),
                Err(e) => Done::Failed(e),
            };
        }
        let rgb: Vec<u8> = padded.data.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
        match std::fs::write(
            work_in.join(format!("{}.png", d.name)),
            png::encode_rgb(padded.width, padded.height, &rgb),
        ) {
            Ok(()) => Done::Queued(p),
            Err(e) => Done::Failed(format!("{}: {e}", work_in.display())),
        }
    });
    let t_prepare = t0.elapsed();

    // The external upscaler, once over the directory.
    let t1 = Instant::now();
    let queued = done.iter().filter(|d| matches!(d, Done::Queued(_))).count();
    let mut upscaler_failed = None;
    if let (Some(template), true) = (&o.upscaler, queued > 0) {
        let cmd = expand(template, &work_in, &work_out, o.scale);
        println!("upscaling {queued} images: {cmd}");
        let log_path = o.work.join("upscaler.log");
        if let Err(e) = run_watched("upscaler", &cmd, &log_path, &work_out, o.stall) {
            let text = std::fs::read_to_string(&log_path).unwrap_or_default();
            let tail: Vec<&str> = text.lines().rev().take(15).collect();
            for l in tail.iter().rev() {
                eprintln!("  {l}");
            }
            upscaler_failed = Some(e);
        }
    }
    let t_upscale = t1.elapsed();

    // Finish the external jobs (after a failure too: what it did write is
    // kept, and a rerun retries the rest).
    let t2 = Instant::now();
    {
        let pending: Vec<usize> = (0..jobs.len()).filter(|&i| matches!(done[i], Done::Queued(_))).collect();
        let finished = par_map(&pending, |&i| {
            let (job, Done::Queued(p)) = (&jobs[i], &done[i]) else { unreachable!() };
            let r = (|| {
                let source = decode(&src.join(format!("{}.png", job.dump.name)))?;
                let up = decode(&work_out.join(format!("{}.png", job.dump.name)))
                    .map_err(|e| format!("no output from the upscaler ({e})"))?;
                let s = fit(source.width, source.height);
                finish(&source, up, &job.dump, *p, o.scale, s, o.colour)
            })();
            match r {
                Ok((out, drift)) => save(job, out, drift),
                Err(e) => Done::Failed(e),
            }
        });
        for (i, d) in pending.into_iter().zip(finished) {
            done[i] = d;
        }
    }
    let t_finish = t2.elapsed();
    drop(log);

    // The record, compacted, and the pack's index.
    let (mut made, mut empty, mut failed, mut drifted) = (0, 0, vec![], vec![]);
    for (job, d) in jobs.iter().zip(&done) {
        match d {
            Done::Made { name, src_hash, drift } => {
                made += 1;
                let st = stamp(&dest.join(format!("{name}.png"))).unwrap_or_default();
                record.images.insert(name.clone(), (*src_hash, st));
                if drift[0] > o.drift || drift[1] > o.drift {
                    drifted.push((name.clone(), *drift));
                }
            }
            Done::Empty => empty += 1,
            Done::Failed(e) => failed.push(format!("{}: {e}", job.dump.name)),
            Done::Queued(_) => failed.push(format!("{}: not upscaled", job.dump.name)),
        }
    }
    write_record(dest, &settings, &record.images)?;
    let images = write_pack_index(dest)?;
    let bytes: u64 =
        std::fs::read_dir(dest).map(|r| r.filter_map(|e| e.ok()?.metadata().ok()).map(|m| m.len()).sum()).unwrap_or(0);

    println!(
        "made {made} in {:.1} s (prepare and built-in {:.1} s, upscaler {:.1} s, finish {:.1} s); \
         {empty} fully transparent (left out); {} failed",
        start.elapsed().as_secs_f32(),
        t_prepare.as_secs_f32(),
        t_upscale.as_secs_f32(),
        t_finish.as_secs_f32(),
        failed.len()
    );
    for f in failed.iter().take(10) {
        println!("  failed: {f}");
    }
    if made > 0 {
        let _ = std::fs::remove_file(dest.join("drift.txt"));
    }
    if !drifted.is_empty() {
        drifted.sort_by(|a, b| b.1[0].total_cmp(&a.1[0]));
        let mut list = String::from(
            "# Mean colour shift (largest channel, 0-255) of pack images against their dumps, of this\n\
             # run's images over the threshold: before and after colour matching (equal with --colour keep).\n\
             # name\tbefore\tafter\n",
        );
        for (n, d) in &drifted {
            let _ = writeln!(list, "{n}\t{:.1}\t{:.1}", d[0], d[1]);
        }
        let _ = write_atomic(&dest.join("drift.txt"), list.as_bytes());
        // Over the threshold (and the worst) at k = 0 before, 1 after.
        let summary = |k: usize| {
            let part = |kind: &str| {
                let over = drifted.iter().filter(|(n, d)| n.starts_with(kind) && d[k] > o.drift);
                let (n, worst) = over.fold((0, 0f32), |(n, w), (_, d)| (n + 1, w.max(d[k])));
                let worst = if n > 0 { format!(" (worst {worst:.1})") } else { String::new() };
                format!("{n} {}{worst}", if kind == "tex_" { "textures" } else { "tiles" })
            };
            format!("{}, {}", part("tex_"), part("tile_"))
        };
        let (b, a) = (summary(0), summary(1));
        if o.colour == Colour::Match {
            println!(
                "colour drift over {}/255 (mean shift of a channel): before matching {b}; after {a} (drift.txt)",
                o.drift
            );
        } else {
            println!(
                "colour drift over {}/255 (mean shift of a channel; lighting assumes the image stands for the \
                 full-luma dump; --colour match corrects it): {b} (drift.txt)",
                o.drift
            );
        }
        for (n, d) in drifted.iter().filter(|(n, _)| n.starts_with("tex_")).take(8) {
            println!("  {n}: {:.1} -> {:.1}", d[0], d[1]);
        }
    }
    println!("pack: {}: {images} images, {} MiB (index.txt lists them)", dest.display(), bytes >> 20);
    let ok = failed.is_empty() && upscaler_failed.is_none();
    if ok && external && !o.keep_work {
        let _ = std::fs::remove_dir_all(&o.work);
    } else if external {
        println!("work files kept in {}", o.work.display());
    }
    let p = dest.display();
    println!(
        "look: make run GAME={game} STEPS=600000000 SNAP=60 INPUT=coin-start PACK={p} \
         VIEW=\"--height 1080 --aspect 16:9 --hud edges\"; make pack-check GAME={game} PACK={p}"
    );
    match (upscaler_failed, failed.len()) {
        (Some(e), _) => Err(e),
        (None, 0) => Ok(()),
        (None, n) => Err(format!("{n} images failed")),
    }
}

/// `remodel texpack-upscale SRC DEST [options]`.
pub fn main(args: &[String], game: &str) -> Result<(), String> {
    let mut o = Options {
        scale: 4,
        upscaler: None,
        tiles: TileMode::Model,
        colour: Colour::Match,
        min_uses: 1,
        max_side: 4096,
        drift: 6.0,
        work: PathBuf::from("out/scratch/texpack-upscale"),
        force: false,
        keep_work: false,
        stall: 300,
    };
    let mut pos = vec![];
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().cloned().ok_or(format!("{a} needs a value"));
        let num = |v: String| v.parse::<usize>().map_err(|_| format!("{a}: not a number: {v}"));
        match a.as_str() {
            "--scale" => o.scale = num(val()?)?,
            "--upscaler" => o.upscaler = Some(val()?).filter(|u| !u.trim().is_empty()),
            "--tiles" => {
                let v = val()?;
                o.tiles =
                    TileMode::parse(&v).ok_or(format!("--tiles wants model, builtin, nearest or skip, not {v}"))?
            }
            "--colour" => {
                let v = val()?;
                o.colour = Colour::parse(&v).ok_or(format!("--colour wants keep or match, not {v}"))?
            }
            "--min-uses" => o.min_uses = num(val()?)? as u64,
            "--max-side" => o.max_side = num(val()?)?,
            "--drift" => o.drift = val()?.parse().map_err(|_| "--drift wants a number")?,
            "--work" => o.work = PathBuf::from(val()?),
            "--force" => o.force = true,
            "--keep-work" => o.keep_work = true,
            "--stall" => o.stall = num(val()?)? as u64,
            _ if a.starts_with("--") => return Err(format!("unknown option {a}")),
            _ => pos.push(a.clone()),
        }
    }
    let [src, dest] = &pos[..] else { return Err("texpack-upscale wants SRC DEST".into()) };
    run(Path::new(src), Path::new(dest), &o, game)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Address::*;

    fn img(w: usize, h: usize, px: impl Fn(usize, usize) -> [u8; 4]) -> Rgba {
        Rgba { width: w, height: h, data: (0..w * h).flat_map(|i| px(i % w, i / w)).collect() }
    }

    fn red(i: &Rgba, x: usize, y: usize) -> u8 {
        i.data[(y * i.width + x) * 4]
    }

    #[test]
    fn pads_like_the_renderer_and_crops_back() {
        // 3x2, red = x + 10 * y.
        let src = img(3, 2, |x, y| [(x + 10 * y) as u8, 0, 0, 255]);
        let row = |p: &Rgba, y: usize| (0..p.width).map(|x| red(p, x, y)).collect::<Vec<_>>();
        let r = pad(&src, [2, 1], [Repeat, Repeat]);
        assert_eq!((r.width, r.height), (7, 4));
        assert_eq!(row(&r, 0), [11, 12, 10, 11, 12, 10, 11]);
        assert_eq!(row(&r, 1), [1, 2, 0, 1, 2, 0, 1]);
        let m = pad(&src, [2, 1], [Mirror, Mirror]);
        assert_eq!(row(&m, 0), [1, 0, 0, 1, 2, 2, 1]);
        assert_eq!(row(&m, 3), [11, 10, 10, 11, 12, 12, 11]);
        let c = pad(&src, [2, 1], [Clamp, Clamp]);
        assert_eq!(row(&c, 0), [0, 0, 0, 1, 2, 2, 2]);
        assert_eq!(row(&c, 3), [10, 10, 10, 11, 12, 12, 12]);
        // Padding wider than the image keeps wrapping.
        let w = pad(&src, [4, 0], [Repeat, Clamp]);
        assert_eq!(row(&w, 0), [2, 0, 1, 2, 0, 1, 2, 0, 1, 2, 0]);
        for p in [r, m, c, w] {
            let (px, py) = ((p.width - 3) / 2, (p.height - 2) / 2);
            assert_eq!(crop(&p, px, py, 3, 2), src);
        }
        // Scaled up, padded and cropped, a flat image stays flat.
        let flat = img(4, 4, |_, _| [90, 40, 20, 255]);
        let up = upscale_builtin(&pad(&flat, [2, 2], [Mirror, Mirror]), 3);
        assert!(crop(&up, 6, 6, 12, 12).data.chunks(4).all(|p| p == [90, 40, 20, 255]));
    }

    #[test]
    fn fills_transparent_pixels_from_opaque_ones() {
        // Opaque blue column 0, the rest transparent garbage (white).
        let mut i = img(4, 1, |x, _| if x == 0 { [0, 0, 200, 255] } else { [255, 255, 255, 0] });
        assert!(fill_transparent(&mut i, [Clamp, Clamp]));
        assert!(i.data.chunks(4).all(|p| p[..3] == [0, 0, 200]));
        assert_eq!(i.data[7], 0, "alpha is untouched");
        // Repeat reaches across the edge: pixel 3 is next to pixel 0.
        let mut i = img(5, 1, |x, _| match x {
            0 => [100, 0, 0, 255],
            2 => [0, 100, 0, 255],
            _ => [255, 255, 255, 0],
        });
        assert!(fill_transparent(&mut i, [Repeat, Clamp]));
        assert_eq!(&i.data[4..7], [50, 50, 0]);
        assert_eq!(&i.data[16..19], [100, 0, 0], "pixel 4 wraps to pixel 0");
        let mut none = img(2, 2, |_, _| [9, 9, 9, 0]);
        assert!(!fill_transparent(&mut none, [Clamp, Clamp]));
    }

    #[test]
    fn alpha_is_cut_at_half_like_the_renderer() {
        // Opaque, transparent: at 4x the edge falls half way, at 1.0.
        let src = img(2, 1, |x, _| [50, 50, 50, if x == 0 { 255 } else { 0 }]);
        let mut out = img(8, 4, |_, _| [0, 0, 0, 7]);
        alpha_up(&src, 4, [Clamp, Clamp], &mut out);
        let row: Vec<u8> = (0..8).map(|x| out.data[x * 4 + 3]).collect();
        assert_eq!(row, [255, 255, 255, 255, 0, 0, 0, 0]);
        // Repeat: the transparent pixel's far side meets the opaque one.
        alpha_up(&src, 4, [Repeat, Repeat], &mut out);
        let row: Vec<u8> = (0..8).map(|x| out.data[x * 4 + 3]).collect();
        assert_eq!(row, [255, 255, 255, 255, 0, 0, 0, 0]);
        let src = img(3, 1, |x, _| [50, 50, 50, if x == 1 { 0 } else { 255 }]);
        let mut out = img(12, 1, |_, _| [0; 4]);
        alpha_up(&src, 4, [Clamp, Clamp], &mut out);
        let row: Vec<u8> = (0..12).map(|x| out.data[x * 4 + 3]).collect();
        // Opaque, transparent, opaque: transparent from 1.0 to 2.0.
        assert_eq!(row, [255, 255, 255, 255, 0, 0, 0, 0, 255, 255, 255, 255]);
    }

    #[test]
    fn reads_wrap_modes_from_new_and_old_index_lines() {
        let text = "# comment\n\
            tex_00000000000000aa_32x32\tframe=60\tuses=9\theader=0100,0,0,0\tcolour=0x7fff\twrap=mr,rr\n\
            tex_00000000000000bb_64x32\tframe=60\tuses=3\theader=0200,0,0,0\tcolour=0x7fff\n\
            tex_00000000000000cc_32x32\tframe=60\tuses=1\theader=0000,0,0,0\twrap=rr\n\
            tex_00000000000000dd_32x32\tframe=60\tuses=1\n\
            tile_00000000000000ee\tframe=0\tuses=40\ttile=0x0005\tpalette_bank=0x02\n\
            \n\
            something_else\tuses=4\n";
        let d = parse_index(text);
        assert_eq!(d.len(), 5);
        let modes: Vec<_> = d.iter().map(|d| (d.modes, d.mixed, d.uses)).collect();
        // Drawn mirrored and repeated in u: mirror there, flagged; v repeat.
        assert_eq!(modes[0], ([Mirror, Repeat], true, 9));
        // Old line: the header's bits 8 (u) and 9 (v).
        assert_eq!(modes[1], ([Repeat, Mirror], false, 3));
        assert_eq!(modes[2], ([Repeat, Repeat], false, 1));
        // Nothing known: mirror.
        assert_eq!(modes[3], ([Mirror, Mirror], false, 1));
        assert_eq!(modes[4], ([Clamp, Clamp], false, 40));
        assert!(d[4].tile && !d[0].tile);
    }

    #[test]
    fn checks_the_upscaler_output_size_and_keeps_tile_pen_0() {
        let src = img(8, 8, |x, _| if x < 4 { [10, 20, 30, 255] } else { [200, 0, 0, 0] });
        let dump = Dump { name: "tile_1".into(), uses: 1, modes: [Clamp; 2], mixed: false, tile: true, surf: None };
        let p = padding(8, 8, true, How::External);
        assert_eq!(p, [8, 8]);
        let padded = pad(&src, p, dump.modes);
        let e = finish(&src, upscale_builtin(&padded, 2), &dump, p, 4, 4, Colour::Match).unwrap_err();
        assert!(e.contains("wrote 48x48, expected 96x96"), "{e}");
        let mut filled = src.clone();
        fill_transparent(&mut filled, dump.modes);
        let (out, drift) =
            finish(&src, upscale_builtin(&pad(&filled, p, dump.modes), 4), &dump, p, 4, 4, Colour::Match).unwrap();
        assert_eq!((out.width, out.height), (32, 32));
        assert!(drift[0] < 1.0 && drift[1] < 1.0);
        assert_eq!(&out.data[..4], [10, 20, 30, 255]);
        assert_eq!(&out.data[31 * 4..32 * 4], [200, 0, 0, 0], "transparent: pen 0's colour back");
        // The size cap: 4x asked, 2x kept.
        let (out, _) =
            finish(&src, upscale_builtin(&pad(&filled, p, dump.modes), 4), &dump, p, 4, 2, Colour::Keep).unwrap();
        assert_eq!((out.width, out.height), (16, 16));
        assert_eq!(padding(4, 40, false, How::External), [16, 16]);
        assert_eq!(padding(4, 40, false, How::Builtin), [16, 16]);
    }

    /// `src` blown up by pixel copies with `f(x, y, rgb)` applied (opaque).
    fn upscaled(src: &Rgba, s: usize, f: impl Fn(usize, usize, [u8; 3]) -> [u8; 3]) -> Rgba {
        let n = upscale_nearest(src, s);
        img(n.width, n.height, |x, y| {
            let o = (y * n.width + x) * 4;
            let [r, g, b] = f(x, y, [n.data[o], n.data[o + 1], n.data[o + 2]]);
            [r, g, b, 255]
        })
    }

    #[test]
    fn colour_matching_undoes_a_shift_and_keeps_detail() {
        // Texel-level pattern, shifted by the "upscaler": back exactly.
        let src = img(8, 8, |x, y| [(x * 20 + 40) as u8, ((x + y) % 2 * 100 + 50) as u8, 90, 255]);
        let mut up = upscaled(&src, 4, |_, _, c| [c[0] + 20, c[1] - 15, c[2] + 9]);
        match_colour(&src, &mut up, 4, [Repeat, Mirror]);
        assert_eq!(up, upscale_nearest(&src, 4));
        // Zero-mean detail inside each texel's block survives untouched.
        let flat = img(6, 6, |_, _| [100, 100, 100, 255]);
        let detail =
            |x: usize, y: usize, c: [u8; 3]| c.map(|v| if (x + y).is_multiple_of(2) { v + 30 } else { v - 30 });
        let want = upscaled(&flat, 4, detail);
        let mut up = upscaled(&flat, 4, |x, y, c| detail(x, y, c).map(|v| v + 12));
        match_colour(&flat, &mut up, 4, [Clamp, Clamp]);
        assert_eq!(up, want);
    }

    #[test]
    fn colour_matching_tiles_and_ignores_transparent_pixels() {
        // Drift only in the left half: a repeating texture's correction
        // runs on across its edge as smoothly as between any two pixels
        // inside (the result tiles as the upscaler's own output did); a
        // clamped one's jumps there.
        let src = img(16, 4, |_, _| [100, 100, 100, 255]);
        let jumps = |modes: [Address; 2]| {
            let up = upscaled(&src, 2, |x, _, c| if x < 16 { c.map(|v| v + 30) } else { c });
            let mut out = up.clone();
            match_colour(&src, &mut out, 2, modes);
            let fix = |x| red(&out, x, 0) as i32 - red(&up, x, 0) as i32;
            let inside = (0..31).map(|x| (fix(x + 1) - fix(x)).abs()).max().unwrap();
            ((fix(0) - fix(31)).abs(), inside)
        };
        let (seam, inside) = jumps([Repeat, Repeat]);
        assert!(seam <= inside, "{seam} {inside}");
        let (seam, inside) = jumps([Clamp, Clamp]);
        assert!(seam >= 10 && seam > 2 * inside, "{seam} {inside}");
        // Transparent pixels pull nothing, on either side: the dump's
        // (white garbage) nor the result's (red, alpha 0); opaque pixels
        // already right stay as they are.
        let src = img(8, 8, |x, _| if x < 4 { [60, 80, 100, 255] } else { [255, 255, 255, 0] });
        let up = img(32, 32, |x, _| if x < 16 { [60, 80, 100, 255] } else { [200, 0, 0, 0] });
        let mut out = up.clone();
        match_colour(&src, &mut out, 4, [Clamp, Clamp]);
        assert_eq!(out, up);
    }

    #[test]
    fn runs_an_external_upscaler_and_stops_a_stalled_one() {
        let dir = std::env::temp_dir().join(format!("remodel-texpack-upscale-ext-{}", std::process::id()));
        let (src, dest) = (dir.join("dump"), dir.join("pack"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&src).unwrap();
        let tex = img(32, 32, |x, y| [(x * 8) as u8, (y * 8) as u8, 99, if x > 28 { 0 } else { 255 }]);
        std::fs::write(src.join("tex_00000000000000aa_32x32.png"), png::encode_rgba(32, 32, &tex.data)).unwrap();
        std::fs::write(src.join("index.txt"), "tex_00000000000000aa_32x32\tuses=5\twrap=rr\n").unwrap();
        // A 1x "upscaler": copies; the result is the dump again.
        let mut o = Options {
            scale: 1,
            upscaler: Some("cp {in}/*.png {out}/".into()),
            tiles: TileMode::Model,
            colour: Colour::Match,
            min_uses: 1,
            max_side: 4096,
            drift: 6.0,
            work: dir.join("work"),
            force: false,
            keep_work: false,
            stall: 300,
        };
        run(&src, &dest, &o, "g").unwrap();
        let out = decode(&dest.join("tex_00000000000000aa_32x32.png")).unwrap();
        // Same alpha and opaque colour (transparent pixels carry the fill).
        for (a, b) in out.data.chunks(4).zip(tex.data.chunks(4)) {
            assert_eq!(a[3], b[3]);
            assert!(a[3] == 0 || a == b);
        }
        assert!(!o.work.exists(), "work files removed");
        // Wrong size, then a tool that hangs.
        o.force = true;
        o.scale = 2;
        assert!(run(&src, &dest, &o, "g").unwrap_err().contains("1 images failed"));
        o.upscaler = Some("sleep 30".into());
        o.stall = 1;
        let t = Instant::now();
        assert!(run(&src, &dest, &o, "g").unwrap_err().contains("wrote nothing for 1 s"));
        assert!(t.elapsed().as_secs() < 10);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn expands_the_template() {
        let c = expand("up -i {in} -o {out} -s {scale}", Path::new("a b"), Path::new("it's"), 4);
        assert_eq!(c, r"up -i 'a b' -o 'it'\''s' -s 4");
    }

    #[test]
    fn redoes_only_new_or_changed_dumps() {
        let dir = std::env::temp_dir().join(format!("remodel-texpack-upscale-test-{}", std::process::id()));
        let (src, dest) = (dir.join("dump"), dir.join("pack"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&src).unwrap();
        let tex = img(32, 32, |x, y| [(x * 8) as u8, (y * 8) as u8, 99, if x > 28 { 0 } else { 255 }]);
        let tile = img(8, 8, |x, _| [50, 60, 70, if x > 3 { 255 } else { 0 }]);
        let put = |name: &str, i: &Rgba| {
            std::fs::write(src.join(format!("{name}.png")), png::encode_rgba(i.width, i.height, &i.data)).unwrap()
        };
        put("tex_00000000000000aa_32x32", &tex);
        put("tile_00000000000000bb", &tile);
        put("tex_00000000000000cc_32x32", &img(32, 32, |_, _| [0; 4]));
        put("tex_00000000000000dd_32x32", &tex);
        std::fs::write(
            src.join("index.txt"),
            "# x\ntex_00000000000000aa_32x32\tframe=0\tuses=5\twrap=rr\n\
             tile_00000000000000bb\tframe=0\tuses=5\n\
             tex_00000000000000cc_32x32\tframe=0\tuses=5\n\
             tex_00000000000000dd_32x32\tframe=0\tuses=1\n",
        )
        .unwrap();
        let mut o = Options {
            scale: 2,
            upscaler: None,
            tiles: TileMode::Builtin,
            colour: Colour::Match,
            min_uses: 2,
            max_side: 4096,
            drift: 6.0,
            work: dir.join("work"),
            force: false,
            keep_work: false,
            stall: 300,
        };
        run(&src, &dest, &o, "g").unwrap();
        let a = dest.join("tex_00000000000000aa_32x32.png");
        let out = decode(&a).unwrap();
        assert_eq!((out.width, out.height), (64, 64));
        assert!(out.data.chunks(4).all(|p| p[3] == 0 || p[3] == 255));
        assert!(!dest.join("tex_00000000000000cc_32x32.png").exists(), "fully transparent");
        assert!(!dest.join("tex_00000000000000dd_32x32.png").exists(), "below MIN_USES");
        let index = std::fs::read_to_string(dest.join("index.txt")).unwrap();
        let names: Vec<&str> = index.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(names, ["tex_00000000000000aa_32x32.png", "tile_00000000000000bb.png"]);
        // Nothing new: nothing redone.
        let before = stamp(&a);
        std::thread::sleep(std::time::Duration::from_millis(20));
        run(&src, &dest, &o, "g").unwrap();
        assert_eq!(stamp(&a), before);
        // A changed dump is redone; a hand-edited pack image is kept.
        put("tex_00000000000000aa_32x32", &img(32, 32, |_, _| [1, 2, 3, 255]));
        let t = dest.join("tile_00000000000000bb.png");
        std::fs::write(&t, png::encode_rgba(1, 1, &[1, 1, 1, 255])).unwrap();
        run(&src, &dest, &o, "g").unwrap();
        assert_eq!(&decode(&a).unwrap().data[..4], [1, 2, 3, 255]);
        assert_eq!(decode(&t).unwrap().width, 1);
        // Other settings: refused, unless forced.
        o.scale = 3;
        assert!(run(&src, &dest, &o, "g").unwrap_err().contains("other settings"));
        o.force = true;
        run(&src, &dest, &o, "g").unwrap();
        assert_eq!(decode(&t).unwrap().width, 24);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
