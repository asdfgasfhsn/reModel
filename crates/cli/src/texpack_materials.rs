//! `texpack-materials`: normal maps (`.n`) for a texture pack, made from its
//! images (or a dump's where the pack has none), by a built-in generator or
//! an external tool. No ROM needed. docs/texture-packs.md ("Material maps")
//! describes it. The built-in derives relief from brightness, which is not
//! height: painted shading, lettering and shadows come out as bumps, so the
//! result is a starting point for editing by hand, kept on reruns.
//!
//! Per texture in the dump's index (tiles never): the source (the pack's
//! image if it has one, else the dump's) has its transparent pixels filled
//! from their opaque neighbours, so a cut-out's outline makes no slope;
//! relief is computed wrap-aware (repeated, or mirrored about the edge
//! texels' centres, per axis as the dump says the texture is drawn), so
//! a repeated map has no seam and a mirrored one's component across the
//! mirror line is zero; texels under transparent base texels then take
//! the mean of their opaque neighbours (they show only in mip levels).
//! One map per surface (`surf_`, every colour of the texels) when the dump
//! has surface keys, from its most-used image, else per image (`tex_`).

use crate::texpack_upscale::{self as up, Dump};
use model2::enhance::{address, Address, Rgba};
use model2::pack::{self, FileKind, Manifest, MapKind};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

/// Bumped when the output for the same settings changes.
const VERSION: u32 = 1;
const RECORD: &str = "materials.txt";
/// Defaults: the slope gain, and the width (Gaussian sigma, in the
/// texture's own texels) of the brightness changes taken as shading or
/// colour areas rather than relief.
pub const STRENGTH: f32 = 1.0;
pub const DETAIL: f32 = 4.0;
/// Source texels of context on each side of an image handed to an
/// external generator, and its smallest side.
const PAD_TEXELS: usize = 16;
const MIN_INPUT: usize = 16;

pub struct Options {
    /// The dump the pack was made from (`index.txt`: which textures, wrap=,
    /// surf=, uses=). None: the pack's own `tex_` images, padded mirrored,
    /// one map per image.
    pub dump: Option<PathBuf>,
    /// Command template with `{in}`, `{out}`; None: built-in.
    pub normalgen: Option<String>,
    /// The external generator's green points up (else down).
    pub gen_y_up: bool,
    pub strength: f32,
    pub detail: f32,
    pub min_uses: u64,
    pub work: PathBuf,
    /// Redo every map this tool made (hand-made and edited ones are kept).
    pub force: bool,
    pub keep_work: bool,
    pub stall: u64,
}

impl Options {
    /// What the output depends on, recorded in the pack.
    fn settings(&self, pack_y_up: bool) -> String {
        let generator = match &self.normalgen {
            None => format!("builtin\tstrength={}\tdetail={}", self.strength, self.detail),
            Some(c) => format!("{c}\tgen_normal_y={}", if self.gen_y_up { "up" } else { "down" }),
        };
        format!("generator={generator}\tnormal_y={}\tversion={VERSION}", if pack_y_up { "up" } else { "down" })
    }
}

// ---------------------------------------------------------------- maths

/// The texel coordinate `c` (any integer) stands for on an axis of `size`:
/// repeated, or reflected about the edge texels' centres (mirrored and
/// anything else). Reflecting about the centre rather than the edge line
/// (as the renderer's mirror does, `enhance::address`) makes the derived
/// relief symmetric about the edge texel, so its slope across a mirror line
/// is exactly zero and the mirrored drawing has no crease; the half-texel
/// shift of the line is not visible.
fn at(c: i64, size: usize, mode: Address) -> usize {
    let s = size as i64;
    if mode == Address::Repeat {
        return c.rem_euclid(s) as usize;
    }
    if s == 1 {
        return 0;
    }
    let p = 2 * (s - 1);
    let m = c.rem_euclid(p);
    (if m < s { m } else { p - m }) as usize
}

/// Luminance (Rec. 601 weights) of each pixel, 0 to 1.
fn luminance(img: &Rgba) -> Vec<f32> {
    img.data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) / 255.0)
        .collect()
}

/// Separable Gaussian blur, `sigma` pixels per axis (0: none), with `at`'s
/// addressing.
fn blur(v: &[f32], w: usize, h: usize, sigma: [f32; 2], modes: [Address; 2]) -> Vec<f32> {
    let pass = |v: &[f32], axis: usize| -> Vec<f32> {
        let s = sigma[axis];
        if s < 0.05 {
            return v.to_vec();
        }
        let r = (3.0 * s).ceil() as i64;
        let k: Vec<f32> = (-r..=r).map(|d| (-(d * d) as f32 / (2.0 * s * s)).exp()).collect();
        let total: f32 = k.iter().sum();
        let k: Vec<f32> = k.iter().map(|x| x / total).collect();
        (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as i64, (i / w) as i64);
                let mut sum = 0f32;
                for (d, &kw) in (-r..=r).zip(&k) {
                    let j = if axis == 0 {
                        y as usize * w + at(x + d, w, modes[0])
                    } else {
                        at(y + d, h, modes[1]) * w + x as usize
                    };
                    sum += kw * v[j];
                }
                sum
            })
            .collect()
    };
    pass(&pass(v, 0), 1)
}

/// Slopes per pixel by a 3x3 Sobel operator (x to the right, y down), with
/// `at`'s addressing: (right - left) / 8, the same for rows.
fn sobel(v: &[f32], w: usize, h: usize, modes: [Address; 2]) -> Vec<[f32; 2]> {
    let px = |x: i64, y: i64| v[at(y, h, modes[1]) * w + at(x, w, modes[0])];
    (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as i64, (i / w) as i64);
            let right = px(x + 1, y - 1) + 2.0 * px(x + 1, y) + px(x + 1, y + 1);
            let left = px(x - 1, y - 1) + 2.0 * px(x - 1, y) + px(x - 1, y + 1);
            let down = px(x - 1, y + 1) + 2.0 * px(x, y + 1) + px(x + 1, y + 1);
            let upper = px(x - 1, y - 1) + 2.0 * px(x, y - 1) + px(x + 1, y - 1);
            [(right - left) / 8.0, (down - upper) / 8.0]
        })
        .collect()
}

fn normalise(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l < 1e-6 || !l.is_finite() {
        [0.0, 0.0, 1.0]
    } else {
        v.map(|c| c / l)
    }
}

/// Whether the image continues across its edges on `axis` (0: u, 1: v)
/// as a repeated texture should: the mean brightness step between its last
/// and first rows of texels is not much larger than between neighbours
/// inside (pairs with a transparent texel left out). Many textures are
/// marked repeated but never drawn past their edge (signs, faces, panels):
/// their edges don't meet.
pub(crate) fn tiles(img: &Rgba, axis: usize) -> bool {
    match image_seam(img, axis) {
        Some((seam, inside)) => seam <= 2.0 * inside + 0.02,
        None => true,
    }
}

/// Whether the image has a line along its edges on `axis`: its brightness
/// changes much more between the first two (and last two) rows of texels
/// than between neighbours inside (a frame drawn round a sign). Relief
/// centred on the edge is then expected where it repeats.
pub(crate) fn edge_line(img: &Rgba, axis: usize) -> bool {
    let (w, h) = (img.width, img.height);
    let (along, across) = if axis == 0 { (h, w) } else { (w, h) };
    if across < 4 {
        return false;
    }
    let lum = luminance(img);
    let px = |i: usize, j: usize| if axis == 0 { i * w + j } else { j * w + i };
    let step = |a: usize, b: usize| {
        (img.data[a * 4 + 3] >= 128 && img.data[b * 4 + 3] >= 128).then(|| (lum[a] - lum[b]).abs())
    };
    let last = across - 1;
    let border: Vec<f32> =
        (0..along).flat_map(|i| [step(px(i, 0), px(i, 1)), step(px(i, last - 1), px(i, last))]).flatten().collect();
    match image_seam(img, axis) {
        Some((_, inside)) if !border.is_empty() => {
            border.iter().sum::<f32>() / border.len() as f32 > 2.0 * inside + 0.02
        }
        _ => false,
    }
}

/// The mean brightness step across the image's edge on `axis` (between
/// its last and first rows of texels) and between neighbours inside, of
/// pairs of opaque texels. None: too small, or nothing to compare.
fn image_seam(img: &Rgba, axis: usize) -> Option<(f32, f32)> {
    let lum = luminance(img);
    edge_steps(img.width, img.height, axis, &|a, b| {
        (img.data[a * 4 + 3] >= 128 && img.data[b * 4 + 3] >= 128).then(|| (lum[a] - lum[b]).abs())
    })
}

/// Mean of `step(a, b)` (None: pair left out) over the pairs of pixels
/// across the edge on `axis`, and over neighbouring pairs inside.
fn edge_steps(w: usize, h: usize, axis: usize, step: &dyn Fn(usize, usize) -> Option<f32>) -> Option<(f32, f32)> {
    let (along, across) = if axis == 0 { (h, w) } else { (w, h) };
    if across < 3 {
        return None;
    }
    let px = |i: usize, j: usize| if axis == 0 { i * w + j } else { j * w + i };
    let mean = |it: &mut dyn Iterator<Item = Option<f32>>| {
        let (s, n) = it.flatten().fold((0f32, 0usize), |(s, n), v| (s + v, n + 1));
        (n > 0).then(|| s / n as f32)
    };
    let seam = mean(&mut (0..along).map(|i| step(px(i, across - 1), px(i, 0))))?;
    let inside = mean(
        &mut (0..along).flat_map(|i| (0..across - 1).map(move |j| (i, j))).map(|(i, j)| step(px(i, j + 1), px(i, j))),
    )?;
    Some((seam, inside))
}

/// `modes` with a repeated axis whose image doesn't tile (`tiles`) taken
/// as mirrored: relief from the step between its far edges would draw a
/// ridge along the border of a sign or panel that is never repeated.
pub(crate) fn edge_modes(src: &Rgba, modes: [Address; 2]) -> [Address; 2] {
    [0, 1].map(|a| if modes[a] == Address::Repeat && !tiles(src, a) { Address::Mirror } else { modes[a] })
}

fn opaque_mask(img: &Rgba) -> Vec<bool> {
    img.data.as_chunks::<4>().0.iter().map(|p| p[3] >= 128).collect()
}

/// Unit normals (x to the image's right, y to its top, z out) derived from
/// the brightness of `src`, an image covering a texture of `tex` texels.
/// Height = luminance minus its Gaussian blur of `detail` texels (so broad
/// shading and colour areas painted into the texture don't become slopes,
/// only finer structure does); slopes by Sobel, per texel of the texture
/// (so a 4x image gets the relief of the 1x one, sharper); normal =
/// normalise(-strength dh/dx, -strength dh/dy_up, 1). Transparent pixels
/// are filled first and given their opaque neighbours' normals after. None:
/// nothing in `src` is opaque.
pub(crate) fn derive(
    src: &Rgba,
    tex: [u32; 2],
    modes: [Address; 2],
    strength: f32,
    detail: f32,
) -> Option<Vec<[f32; 3]>> {
    let (w, h) = (src.width, src.height);
    let mut filled = src.clone();
    if !up::fill_transparent(&mut filled, modes) {
        return None;
    }
    // Pixels per texel on each axis.
    let s = [w as f32 / tex[0].max(1) as f32, h as f32 / tex[1].max(1) as f32];
    let lum = luminance(&filled);
    let height: Vec<f32> = if detail > 0.0 {
        let low = blur(&lum, w, h, [detail * s[0], detail * s[1]], modes);
        lum.iter().zip(&low).map(|(l, b)| l - b).collect()
    } else {
        lum
    };
    let mut n: Vec<[f32; 3]> = sobel(&height, w, h, modes)
        .into_iter()
        .map(|[gx, gy]| {
            // Rows run down; the normal's y points up: dh/dy_up = -gy.
            normalise([-strength * gx * s[0], strength * gy * s[1], 1.0])
        })
        .collect();
    fill_cutouts(&mut n, &opaque_mask(src), w, h, modes);
    Some(n)
}

/// Give every texel not in `known` the mean of its known neighbours'
/// normals, layer by layer outwards (the colour fill's rule), renormalised;
/// all flat if nothing is known.
fn fill_cutouts(n: &mut [[f32; 3]], known: &[bool], w: usize, h: usize, modes: [Address; 2]) {
    let mut known = known.to_vec();
    if !known.contains(&true) {
        n.iter_mut().for_each(|v| *v = [0.0, 0.0, 1.0]);
        return;
    }
    let neighbours = |i: usize| {
        let (x, y) = ((i % w) as i32, (i / w) as i32);
        (-1..=1).flat_map(move |dy| {
            (-1..=1)
                .filter(move |&dx| dx != 0 || dy != 0)
                .map(move |dx| address(y + dy, h, modes[1]) * w + address(x + dx, w, modes[0]))
        })
    };
    let mut queued = known.clone();
    let mut frontier: Vec<usize> = (0..w * h).filter(|&i| !known[i] && neighbours(i).any(|j| known[j])).collect();
    for &i in &frontier {
        queued[i] = true;
    }
    while !frontier.is_empty() {
        let means: Vec<[f32; 3]> = frontier
            .iter()
            .map(|&i| {
                let mut sum = [0f32; 3];
                for j in neighbours(i).filter(|&j| known[j]) {
                    for c in 0..3 {
                        sum[c] += n[j][c];
                    }
                }
                normalise(sum)
            })
            .collect();
        for (&i, m) in frontier.iter().zip(means) {
            n[i] = m;
            known[i] = true;
        }
        let mut next = vec![];
        for &i in &frontier {
            for j in neighbours(i) {
                if !queued[j] {
                    queued[j] = true;
                    next.push(j);
                }
            }
        }
        frontier = next;
    }
}

/// A component -1..1 as stored: 128 + 127 v, so 128 is exactly 0.
fn to_byte(v: f32) -> u8 {
    (128.0 + 127.0 * v).round().clamp(1.0, 255.0) as u8
}

/// Normals (y up) as a normal map's RGB in the pack's green direction.
fn encode(n: &[[f32; 3]], y_up: bool) -> Vec<u8> {
    n.iter().flat_map(|v| [to_byte(v[0]), to_byte(if y_up { v[1] } else { -v[1] }), to_byte(v[2].max(0.0))]).collect()
}

/// A normal map's texel as the renderer reads it: x, y from red and green
/// (`(v - 128) / 127`, y turned to point up), scaled back to length 1 if
/// longer, z reconstructed (blue ignored).
pub(crate) fn read_normal(p: &[u8], y_up: bool) -> [f32; 3] {
    let c = |v: u8| ((v as f32 - 128.0) / 127.0).clamp(-1.0, 1.0);
    let (x, y) = (c(p[0]), if y_up { c(p[1]) } else { -c(p[1]) });
    let l = (x * x + y * y).sqrt();
    let (x, y) = if l > 1.0 { (x / l, y / l) } else { (x, y) };
    [x, y, (1.0 - x * x - y * y).max(0.0).sqrt()]
}

/// `img` with `pad` pixels added on each side of each axis, by `at`'s
/// addressing (for an external generator).
fn pad_reflect(img: &Rgba, pad: [usize; 2], modes: [Address; 2]) -> Rgba {
    let (w, h) = (img.width + 2 * pad[0], img.height + 2 * pad[1]);
    let mut data = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let sy = at(y as i64 - pad[1] as i64, img.height, modes[1]);
        for x in 0..w {
            let sx = at(x as i64 - pad[0] as i64, img.width, modes[0]);
            let o = (sy * img.width + sx) * 4;
            data.extend_from_slice(&img.data[o..o + 4]);
        }
    }
    Rgba { width: w, height: h, data }
}

// ---------------------------------------------------------------- the run

/// One map to make: its file stem (with `.n`), the image it is made from,
/// the texture's size in texels, how it is drawn and how often.
#[derive(Debug)]
struct Target {
    stem: String,
    source: PathBuf,
    tex: [u32; 2],
    modes: [Address; 2],
    uses: u64,
}

fn tex_size(name: &str) -> Option<[u32; 2]> {
    match pack::parse_name(name) {
        Ok(FileKind::Image { width, height, .. }) => Some([width, height]),
        _ => None,
    }
}

/// The maps to make: per surface (or per image, without `surf=`), from
/// the member with a pack image and the most uses. Also counts textures
/// with neither a pack nor a dump image.
fn targets(pack: &Path, dump: Option<&Path>) -> Result<(Vec<Target>, usize, usize), String> {
    let Some(dump) = dump else {
        // The pack's own images: nothing known about wrapping.
        let mut out = vec![];
        for e in std::fs::read_dir(pack).map_err(|e| format!("{}: {e}", pack.display()))? {
            let name = e.map_err(|e| e.to_string())?.file_name().to_string_lossy().into_owned();
            let Some(stem) = name.strip_suffix(".png") else { continue };
            if let Some(tex) = tex_size(stem) {
                out.push(Target {
                    stem: pack::map_stem(stem, MapKind::Normal),
                    source: pack.join(&name),
                    tex,
                    modes: [Address::Mirror; 2],
                    uses: u64::MAX,
                });
            }
        }
        out.sort_by(|a, b| a.stem.cmp(&b.stem));
        let n = out.len();
        return Ok((out, n, 0));
    };
    let index_path = dump.join(pack::INDEX);
    let index = std::fs::read_to_string(&index_path)
        .map_err(|e| format!("{}: {e} (make one with `make dump-textures`)", index_path.display()))?;
    let dumps: Vec<Dump> = up::parse_index(&index).into_iter().filter(|d| !d.tile).collect();
    let textures = dumps.len();
    // Members by map base, in index order.
    let mut groups: BTreeMap<String, Vec<(&Dump, [u32; 2])>> = BTreeMap::new();
    for d in &dumps {
        let Some(tex) = tex_size(&d.name) else { continue };
        let base = match &d.surf {
            Some(s) => format!("surf_{s}_{}x{}", tex[0], tex[1]),
            None => d.name.clone(),
        };
        groups.entry(base).or_default().push((d, tex));
    }
    let (mut out, mut missing) = (vec![], 0);
    for (base, members) in groups {
        // An axis is repeated only if every use of every member repeats it.
        let modes = [0, 1].map(|a| {
            if members.iter().all(|(d, _)| d.modes[a] == Address::Repeat) {
                Address::Repeat
            } else {
                Address::Mirror
            }
        });
        let uses = members.iter().map(|(d, _)| d.uses).sum();
        let in_pack = |d: &Dump| pack.join(format!("{}.png", d.name)).exists();
        let best = members.iter().enumerate().max_by_key(|(i, (d, _))| (in_pack(d), d.uses, usize::MAX - i));
        let (_, (d, tex)) = best.expect("a group has members");
        let source = [pack, dump].map(|dir| dir.join(format!("{}.png", d.name))).into_iter().find(|p| p.exists());
        let Some(source) = source else {
            missing += 1;
            continue;
        };
        out.push(Target { stem: pack::map_stem(&base, MapKind::Normal), source, tex: *tex, modes, uses });
    }
    Ok((out, textures, missing))
}

/// The pack's record: settings, then a line per map made, with its
/// source's hash and its own stamp. Later lines win.
struct Record {
    settings: Option<String>,
    maps: BTreeMap<String, (u64, String)>,
}

fn read_record(dest: &Path) -> Record {
    let mut r = Record { settings: None, maps: BTreeMap::new() };
    let Ok(text) = std::fs::read_to_string(dest.join(RECORD)) else { return r };
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        if let Some(s) = line.strip_prefix("settings\t") {
            r.settings = Some(s.to_string());
        } else if let [name, src, out] = line.split('\t').collect::<Vec<_>>()[..] {
            if let (Some(h), Some(o)) = (src.strip_prefix("src="), out.strip_prefix("out=")) {
                if let Ok(h) = u64::from_str_radix(h, 16) {
                    r.maps.insert(name.to_string(), (h, o.to_string()));
                }
            }
        }
    }
    r
}

const RECORD_HEAD: &str =
    "# reModel texpack-materials record: the settings, then per map made its source's hash and the\n\
     # map file's size and time. Reruns redo only new or changed sources and keep maps that don't\n\
     # match (made or edited by hand); delete a map to have it made again.\n";

fn write_record(dest: &Path, settings: &str, maps: &BTreeMap<String, (u64, String)>) -> Result<(), String> {
    let mut text = format!("{RECORD_HEAD}settings\t{settings}\n");
    for (name, (h, o)) in maps {
        let _ = writeln!(text, "{name}\tsrc={h:016x}\tout={o}");
    }
    up::write_atomic(&dest.join(RECORD), text.as_bytes())
}

/// Add `materials` to `pack.txt`'s `uses` (creating the manifest with
/// `format 2`, `keys 1` and `game` if there is none), changing no other
/// line. Returns what was done.
fn declare(dest: &Path, game: &str) -> Result<&'static str, String> {
    let path = dest.join(pack::MANIFEST);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let m = Manifest {
                format: 2,
                keys: 1,
                game: Some(game.to_string()),
                uses: vec!["materials".into()],
                ..Manifest::NONE
            };
            up::write_atomic(&path, m.to_text().as_bytes())?;
            return Ok("written");
        }
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let m = Manifest::parse(&text).map_err(|e| format!("{e} (fix it, or remove it to have one written)"))?;
    if m.uses.iter().any(|u| u == "materials") {
        return Ok("unchanged");
    }
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let keyword = |l: &str| l.trim_start_matches('\u{feff}').trim().split(['\t', ' ']).next().unwrap_or("").to_string();
    if let Some(l) = lines.iter_mut().find(|l| keyword(l) == "uses") {
        let sep = if l.trim_end().ends_with("uses") { "\t" } else { "," };
        l.truncate(l.trim_end().len());
        *l += sep;
        *l += "materials";
    } else {
        // After the header keywords, before any `material` line.
        let at = lines
            .iter()
            .rposition(|l| matches!(keyword(l).as_str(), "format" | "keys" | "name" | "game"))
            .map_or(lines.len(), |i| i + 1);
        lines.insert(at, "uses\tmaterials".into());
    }
    let mut out = lines.join("\n");
    out.push('\n');
    Manifest::parse(&out).map_err(|e| format!("{}: rewriting `uses` broke it ({e})", path.display()))?;
    up::write_atomic(&path, out.as_bytes())?;
    Ok("`uses materials` added")
}

/// How one target ended.
enum Done {
    Made {
        stem: String,
        src_hash: u64,
        untiled: usize,
    },
    Empty,
    /// No relief at all: no map (a stale one of ours removed).
    Flat,
    Failed(String),
    /// Written for the external generator, with its padding.
    Queued([usize; 2]),
}

struct Job {
    target: Target,
    src_hash: u64,
}

/// Make the normal maps for the pack in `dest`. `game` goes into a new
/// `pack.txt` and the commands printed at the end.
pub fn run(dest: &Path, o: &Options, game: &str) -> Result<(), String> {
    let start = Instant::now();
    if !dest.is_dir() {
        return Err(format!("{}: texture pack directory not found", dest.display()));
    }
    if !(0.0..=64.0).contains(&o.strength) || !(0.0..=1024.0).contains(&o.detail) {
        return Err("--strength wants 0 to 64, --detail 0 to 1024".into());
    }
    let manifest = match std::fs::read_to_string(dest.join(pack::MANIFEST)) {
        Ok(t) => Manifest::parse(&t).map_err(|e| format!("{}: {e}", dest.display()))?,
        Err(_) => Manifest::NONE,
    };
    let y_up = manifest.normal_y_up;
    let settings = o.settings(y_up);
    let mut record = read_record(dest);
    if record.settings.as_ref().is_some_and(|s| *s != settings) && !o.force {
        return Err(format!(
            "{} has maps made with other settings ({}); FORCE=1 remakes them all with these (hand-made and \
             edited maps are kept either way)",
            dest.display(),
            record.settings.unwrap_or_default().replace('\t', " ")
        ));
    }
    let (targets, textures, missing) = targets(dest, o.dump.as_deref())?;
    let surfaces = targets.iter().filter(|t| t.stem.starts_with("surf_")).count();
    println!(
        "texpack-materials: {} ({}), generator: {}, maps in normal_y {}",
        dest.display(),
        match &o.dump {
            Some(d) => format!("textures from {}", d.display()),
            None => "its own images, no dump: padded mirrored".into(),
        },
        match &o.normalgen {
            None => format!("built-in from luminance, strength {} detail {}", o.strength, o.detail),
            Some(c) => c.clone(),
        },
        if y_up { "up" } else { "down" }
    );

    let (mut below, mut current, mut kept) = (0, 0, 0);
    let mut jobs = vec![];
    for t in targets {
        if t.uses < o.min_uses {
            below += 1;
            continue;
        }
        let Ok(mut bytes) = std::fs::read(&t.source) else { continue };
        bytes.extend([t.modes[0] as u8, t.modes[1] as u8]);
        let src_hash = up::fnv(&bytes);
        let out = dest.join(format!("{}.png", t.stem));
        if out.exists() {
            let ours = record.maps.get(&t.stem).filter(|(_, st)| Some(st) == up::stamp(&out).as_ref());
            match ours {
                None => {
                    kept += 1; // made or edited by hand
                    continue;
                }
                Some((h, _)) if *h == src_hash && !o.force => {
                    current += 1;
                    continue;
                }
                Some(_) => {} // the source changed, or forced: redo
            }
        }
        jobs.push(Job { target: t, src_hash });
    }
    println!(
        "{textures} textures -> {} maps ({surfaces} per surface, the rest per image): {current} up to date, \
         {kept} kept (made or edited by hand), {below} below MIN_USES, {missing} without an image; {} to do",
        jobs.len() + current + kept + below,
        jobs.len()
    );

    let (work_in, work_out) = (o.work.join("in"), o.work.join("out"));
    let external = o.normalgen.is_some() && !jobs.is_empty();
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
    // Write a made map and log it at once, so an interrupted run keeps it.
    let save = |job: &Job, src: &Rgba, n: &[[f32; 3]], untiled: usize| -> Done {
        let path = dest.join(format!("{}.png", job.target.stem));
        let rgb = encode(n, y_up);
        if rgb.chunks(3).all(|p| p[0].abs_diff(128) <= 1 && p[1].abs_diff(128) <= 1) {
            let _ = std::fs::remove_file(&path);
            return Done::Flat;
        }
        if let Err(e) = up::write_atomic(&path, &png::encode_rgb(src.width, src.height, &rgb)) {
            return Done::Failed(e);
        }
        let st = up::stamp(&path).unwrap_or_default();
        let _ = writeln!(log.lock().unwrap(), "{}\tsrc={:016x}\tout={st}", job.target.stem, job.src_hash);
        Done::Made { stem: job.target.stem.clone(), src_hash: job.src_hash, untiled }
    };
    let input_name = |t: &Target| format!("{}.png", t.stem.trim_end_matches(".n"));

    let t0 = Instant::now();
    let mut done = up::par_map(&jobs, |job| {
        let t = &job.target;
        let src = match up::decode(&t.source) {
            Ok(i) => i,
            Err(e) => return Done::Failed(e),
        };
        let modes = edge_modes(&src, t.modes);
        let untiled = (0..2).filter(|&a| modes[a] != t.modes[a]).count();
        if o.normalgen.is_none() {
            return match derive(&src, t.tex, modes, o.strength, o.detail) {
                Some(n) => save(job, &src, &n, untiled),
                None => Done::Empty,
            };
        }
        let mut filled = src.clone();
        if !up::fill_transparent(&mut filled, modes) {
            return Done::Empty;
        }
        let per = |size: usize, texels: u32| (size / texels.max(1) as usize).max(1);
        let p = [
            (PAD_TEXELS * per(src.width, t.tex[0])).max(MIN_INPUT.saturating_sub(src.width).div_ceil(2)),
            (PAD_TEXELS * per(src.height, t.tex[1])).max(MIN_INPUT.saturating_sub(src.height).div_ceil(2)),
        ];
        let padded = pad_reflect(&filled, p, modes);
        let rgb: Vec<u8> = padded.data.as_chunks::<4>().0.iter().flat_map(|q| [q[0], q[1], q[2]]).collect();
        match std::fs::write(work_in.join(input_name(t)), png::encode_rgb(padded.width, padded.height, &rgb)) {
            Ok(()) => Done::Queued(p),
            Err(e) => Done::Failed(format!("{}: {e}", work_in.display())),
        }
    });
    let t_prepare = t0.elapsed();

    let t1 = Instant::now();
    let queued = done.iter().filter(|d| matches!(d, Done::Queued(_))).count();
    let mut tool_failed = None;
    if let (Some(template), true) = (&o.normalgen, queued > 0) {
        let cmd = up::expand(template, &work_in, &work_out, 1);
        println!("generating {queued} normal maps: {cmd}");
        let log_path = o.work.join("normalgen.log");
        if let Err(e) = up::run_watched("normal map generator", &cmd, &log_path, &work_out, o.stall) {
            let text = std::fs::read_to_string(&log_path).unwrap_or_default();
            let tail: Vec<&str> = text.lines().rev().take(15).collect();
            for l in tail.iter().rev() {
                eprintln!("  {l}");
            }
            tool_failed = Some(e);
        }
    }
    let t_tool = t1.elapsed();

    // Finish the external jobs (after a failure too: what it wrote is kept).
    {
        let pending: Vec<usize> = (0..jobs.len()).filter(|&i| matches!(done[i], Done::Queued(_))).collect();
        let finished = up::par_map(&pending, |&i| {
            let (job, Done::Queued(p)) = (&jobs[i], &done[i]) else { unreachable!() };
            let t = &job.target;
            let r = (|| {
                let src = up::decode(&t.source)?;
                let out = up::decode(&work_out.join(input_name(t)))
                    .map_err(|e| format!("no output from the generator ({e})"))?;
                let (pw, ph) = (src.width + 2 * p[0], src.height + 2 * p[1]);
                if (out.width, out.height) != (pw, ph) {
                    return Err(format!(
                        "generator wrote {}x{}, expected {pw}x{ph} (its input's size)",
                        out.width, out.height
                    ));
                }
                let out = up::crop(&out, p[0], p[1], src.width, src.height);
                let mut n: Vec<[f32; 3]> =
                    out.data.as_chunks::<4>().0.iter().map(|q| read_normal(q, o.gen_y_up)).collect();
                let modes = edge_modes(&src, t.modes);
                let untiled = (0..2).filter(|&a| modes[a] != t.modes[a]).count();
                fill_cutouts(&mut n, &opaque_mask(&src), src.width, src.height, modes);
                Ok((src, n, untiled))
            })();
            match r {
                Ok((src, n, untiled)) => save(job, &src, &n, untiled),
                Err(e) => Done::Failed(e),
            }
        });
        for (i, d) in pending.into_iter().zip(finished) {
            done[i] = d;
        }
    }
    drop(log);

    let (mut made, mut empty, mut flat, mut untiled_axes, mut failed) = (0, 0, 0, 0, vec![]);
    for (job, d) in jobs.iter().zip(&done) {
        match d {
            Done::Made { stem, src_hash, untiled } => {
                made += 1;
                untiled_axes += untiled;
                let st = up::stamp(&dest.join(format!("{stem}.png"))).unwrap_or_default();
                record.maps.insert(stem.clone(), (*src_hash, st));
            }
            Done::Empty => empty += 1,
            Done::Flat => flat += 1,
            Done::Failed(e) => failed.push(format!("{}: {e}", job.target.stem)),
            Done::Queued(_) => failed.push(format!("{}: not generated", job.target.stem)),
        }
    }
    // Forget maps that are gone (deleted: made again next time anyway).
    record.maps.retain(|stem, _| dest.join(format!("{stem}.png")).exists());
    write_record(dest, &settings, &record.maps)?;
    let manifest_done = declare(dest, game)?;
    let files = up::write_pack_index(dest)?;
    let (mut n_maps, mut map_bytes) = (0, 0u64);
    for e in std::fs::read_dir(dest).map_err(|e| e.to_string())?.filter_map(|e| e.ok()) {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.ends_with(".n.png") {
            n_maps += 1;
            map_bytes += e.metadata().map_or(0, |m| m.len());
        }
    }
    println!(
        "made {made} in {:.1} s (built-in or preparing {:.1} s, generator {:.1} s); {empty} fully transparent \
         and {flat} without relief (left out); {} failed; {untiled_axes} axes marked repeated whose image \
         doesn't tile, treated as mirrored at the edge",
        start.elapsed().as_secs_f32(),
        t_prepare.as_secs_f32(),
        t_tool.as_secs_f32(),
        failed.len()
    );
    for f in failed.iter().take(10) {
        println!("  failed: {f}");
    }
    println!(
        "pack: {}: {n_maps} normal maps, {:.1} MiB; {files} files in index.txt; pack.txt {manifest_done}",
        dest.display(),
        map_bytes as f64 / (1 << 20) as f64
    );
    if o.normalgen.is_none() && made > 0 {
        println!(
            "note: brightness is not height: painted shading, lettering and shadows come out as relief. \
             The maps are a starting point to edit by hand; edited maps are kept on reruns."
        );
    }
    let ok = failed.is_empty() && tool_failed.is_none();
    if ok && external && !o.keep_work {
        let _ = std::fs::remove_dir_all(&o.work);
    } else if external {
        println!("work files kept in {}", o.work.display());
    }
    let p = dest.display();
    let dump = o.dump.as_ref().map(|d| format!(" TEXDUMP={}", d.display())).unwrap_or_default();
    println!("check: make pack-lint PACK={p}{dump} (game {game})");
    match (tool_failed, failed.len()) {
        (Some(e), _) => Err(e),
        (None, 0) => Ok(()),
        (None, n) => Err(format!("{n} maps failed")),
    }
}

/// `remodel texpack-materials PACK [options]`.
pub fn main(args: &[String], game: &str) -> Result<(), String> {
    let mut o = Options {
        dump: None,
        normalgen: None,
        gen_y_up: true,
        strength: STRENGTH,
        detail: DETAIL,
        min_uses: 1,
        work: PathBuf::from("out/scratch/texpack-materials"),
        force: false,
        keep_work: false,
        stall: 300,
    };
    let mut pos = vec![];
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().cloned().ok_or(format!("{a} needs a value"));
        let num = |v: String| v.parse::<f32>().ok().filter(|x| x.is_finite()).ok_or(format!("{a}: not a number: {v}"));
        match a.as_str() {
            "--dump" => o.dump = Some(PathBuf::from(val()?)).filter(|d| !d.as_os_str().is_empty()),
            "--normalgen" => o.normalgen = Some(val()?).filter(|c| !c.trim().is_empty()),
            "--normal-y" => {
                o.gen_y_up = match val()?.as_str() {
                    "up" => true,
                    "down" => false,
                    v => return Err(format!("--normal-y wants up or down, not {v}")),
                }
            }
            "--strength" => o.strength = num(val()?)?,
            "--detail" => o.detail = num(val()?)?,
            "--min-uses" => o.min_uses = val()?.parse().map_err(|_| "--min-uses wants a number")?,
            "--work" => o.work = PathBuf::from(val()?),
            "--force" => o.force = true,
            "--keep-work" => o.keep_work = true,
            "--stall" => o.stall = val()?.parse().map_err(|_| "--stall wants seconds")?,
            _ if a.starts_with("--") => return Err(format!("unknown option {a}")),
            _ => pos.push(a.clone()),
        }
    }
    let [dest] = &pos[..] else { return Err("texpack-materials wants PACK (a texture pack directory)".into()) };
    run(Path::new(dest), &o, game)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Address::*;

    fn img(w: usize, h: usize, px: impl Fn(usize, usize) -> [u8; 4]) -> Rgba {
        Rgba { width: w, height: h, data: (0..w * h).flat_map(|i| px(i % w, i / w)).collect() }
    }

    /// A deterministic pseudo-random texture.
    fn noise(w: usize, h: usize, seed: u32) -> Rgba {
        img(w, h, |x, y| {
            let mut v = (x as u32).wrapping_mul(0x9e37_79b9) ^ (y as u32).wrapping_mul(0x85eb_ca6b) ^ seed;
            v ^= v >> 15;
            v = v.wrapping_mul(0x2c1b_3c6d);
            v ^= v >> 12;
            [v as u8, (v >> 8) as u8, (v >> 16) as u8, 255]
        })
    }

    fn rgb(n: &[[f32; 3]]) -> Vec<u8> {
        encode(n, true)
    }

    #[test]
    fn reflects_about_the_edge_texels() {
        let row: Vec<usize> = (-3..7).map(|c| at(c, 4, Mirror)).collect();
        assert_eq!(row, [3, 2, 1, 0, 1, 2, 3, 2, 1, 0]);
        let row: Vec<usize> = (-3..7).map(|c| at(c, 4, Repeat)).collect();
        assert_eq!(row, [1, 2, 3, 0, 1, 2, 3, 0, 1, 2]);
        assert_eq!(at(-5, 1, Mirror), 0);
    }

    #[test]
    fn flat_input_gives_exactly_the_flat_normal() {
        for (w, h, s) in [(16, 8, 1), (64, 32, 4), (1, 1, 1), (3, 5, 1)] {
            for modes in [[Repeat, Repeat], [Mirror, Repeat], [Mirror, Mirror]] {
                let n =
                    derive(&img(w, h, |_, _| [93, 140, 27, 255]), [(w / s) as u32, (h / s) as u32], modes, 3.0, 8.0)
                        .unwrap();
                assert!(rgb(&n).chunks(3).all(|p| p == [128, 128, 255]), "{w}x{h} {modes:?}");
            }
        }
        assert!(derive(&img(4, 4, |_, _| [9, 9, 9, 0]), [4, 4], [Repeat; 2], 1.0, 4.0).is_none());
    }

    #[test]
    fn a_bright_bump_leans_outwards() {
        // A bright spot in the middle: right of it the normal leans right,
        // above it up (green > 128 with normal_y up).
        let src =
            img(16, 16, |x, y| if (6..10).contains(&x) && (6..10).contains(&y) { [255; 4] } else { [0, 0, 0, 255] });
        let n = rgb(&derive(&src, [16, 16], [Repeat; 2], 1.0, 0.0).unwrap());
        let at = |x: usize, y: usize| &n[(y * 16 + x) * 3..(y * 16 + x) * 3 + 3];
        assert!(at(10, 8)[0] > 160 && at(5, 8)[0] < 96, "{:?} {:?}", at(10, 8), at(5, 8));
        assert!(at(8, 5)[1] > 160 && at(8, 10)[1] < 96, "{:?} {:?}", at(8, 5), at(8, 10));
        // normal_y down flips green only.
        let d = encode(&derive(&src, [16, 16], [Repeat; 2], 1.0, 0.0).unwrap(), false);
        let o = (5 * 16 + 8) * 3;
        assert_eq!(d[o + 1] as u16, 256 - at(8, 5)[1] as u16);
        assert_eq!((d[o], d[o + 2]), (at(8, 5)[0], at(8, 5)[2]));
    }

    #[test]
    fn repeated_maps_tile_without_a_seam() {
        // On a repeated texture the map is the same whichever texel is
        // taken as the origin: shifting the source shifts the map, bit for
        // bit, so its opposite edges meet like any two neighbours inside.
        let (w, h, dx, dy) = (24, 16, 7, 5);
        let src = noise(w, h, 1);
        let shifted = img(w, h, |x, y| {
            let o = (((y + h - dy) % h) * w + (x + w - dx) % w) * 4;
            src.data[o..o + 4].try_into().unwrap()
        });
        for (detail, s) in [(4.0, 1), (2.0, 2)] {
            let tex = [(w / s) as u32, (h / s) as u32];
            let a = rgb(&derive(&src, tex, [Repeat; 2], 1.0, detail).unwrap());
            let b = rgb(&derive(&shifted, tex, [Repeat; 2], 1.0, detail).unwrap());
            for y in 0..h {
                for x in 0..w {
                    let (i, j) = ((y * w + x) * 3, (((y + dy) % h) * w + (x + dx) % w) * 3);
                    assert_eq!(a[i..i + 3], b[j..j + 3], "({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn mirrored_maps_meet_their_mirror_image() {
        // A texture mirrored in u is drawn as I, then I reversed, ... The
        // map of I (mirrored) is the map of that unfolded strip (repeated)
        // over I's texels, and in the reversed copy x changes sign, which
        // the renderer applies: so the drawn normals continue across the
        // mirror line, where the x component is exactly 0.
        let (w, h) = (12, 10);
        let src = noise(w, h, 2);
        let p = 2 * (w - 1);
        let unfolded = img(p, h, |x, y| {
            let sx = if x < w { x } else { p - x };
            src.data[(y * w + sx) * 4..(y * w + sx) * 4 + 4].try_into().unwrap()
        });
        let a = rgb(&derive(&src, [w as u32, h as u32], [Mirror, Repeat], 1.5, 3.0).unwrap());
        let u = rgb(&derive(&unfolded, [p as u32, h as u32], [Repeat, Repeat], 1.5, 3.0).unwrap());
        for y in 0..h {
            for x in 0..w {
                let (i, j) = ((y * w + x) * 3, (y * p + x) * 3);
                assert_eq!(a[i..i + 3], u[j..j + 3], "({x}, {y})");
                if (1..w - 1).contains(&x) {
                    let m = (y * p + p - x) * 3;
                    assert_eq!(u[m] as u16, 256 - a[i] as u16, "x flips in the mirrored copy");
                    assert_eq!(u[m + 1..m + 3], a[i + 1..i + 3]);
                }
            }
            assert_eq!(a[y * w * 3], 128, "x is 0 on the mirror line");
            assert_eq!(a[(y * w + w - 1) * 3], 128);
        }
    }

    #[test]
    fn cut_outs_make_no_relief_and_take_their_neighbours_normals() {
        // A flat colour with a transparent hole (garbage colour under it):
        // the outline is no slope, and the hole is flat.
        let holed = |x: usize, y: usize| {
            if (4..9).contains(&x) && (3..7).contains(&y) {
                [255, 0, 255, 0]
            } else {
                [90, 90, 90, 255]
            }
        };
        let n = rgb(&derive(&img(16, 12, holed), [16, 12], [Repeat; 2], 2.0, 4.0).unwrap());
        assert!(n.chunks(3).all(|p| p == [128, 128, 255]));
        // Relief everywhere, and a hole: a texel under it next to the
        // opaque part is the mean of its opaque neighbours' normals, not
        // flat and not the garbage colour's.
        let ramp = |x: usize, y: usize| {
            if (5..11).contains(&x) && (5..11).contains(&y) {
                [255, 255, 255, 0]
            } else {
                let v = ((x * 37 + y * 11) % 256) as u8;
                [v, v, v, 255]
            }
        };
        let src = img(16, 16, ramp);
        let n = derive(&src, [16, 16], [Repeat; 2], 1.0, 0.0).unwrap();
        let known = opaque_mask(&src);
        let i = 5 * 16 + 5; // the hole's corner: five opaque neighbours
        let mut sum = [0f32; 3];
        for j in [4 * 16 + 4, 4 * 16 + 5, 4 * 16 + 6, 5 * 16 + 4, 6 * 16 + 4] {
            assert!(known[j]);
            (0..3).for_each(|c| sum[c] += n[j][c]);
        }
        let want = normalise(sum);
        assert!((0..3).all(|c| (n[i][c] - want[c]).abs() < 1e-6), "{:?} {want:?}", n[i]);
        assert!(n[i][0].abs() > 0.05, "not flat");
        assert!(n.iter().all(|v| ((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt() - 1.0).abs() < 1e-5));
    }

    #[test]
    fn repeated_images_that_dont_tile_are_taken_as_mirrored_at_the_edge() {
        // A gradient across u (a panel never drawn past its edge): its far
        // edges don't meet, so no ridge along its border; v tiles.
        let panel = img(16, 8, |x, _| [(x * 16) as u8, 0, 0, 255]);
        assert_eq!(edge_modes(&panel, [Repeat, Repeat]), [Mirror, Repeat]);
        assert_eq!(edge_modes(&noise(16, 16, 3), [Repeat, Repeat]), [Repeat, Repeat]);
        assert_eq!(edge_modes(&panel, [Mirror, Mirror]), [Mirror, Mirror]);
        // A frame line on both edges: tiles, and is a line along the edge.
        let framed = img(16, 16, |x, _| if x == 0 || x == 15 { [0, 0, 0, 255] } else { [200, 200, 200, 255] });
        assert!(tiles(&framed, 0) && edge_line(&framed, 0) && !edge_line(&framed, 1));
    }

    #[test]
    fn reads_normals_as_the_renderer_does() {
        assert_eq!(read_normal(&[128, 128, 255], true), [0.0, 0.0, 1.0]);
        let n = read_normal(&[255, 1, 0], true);
        assert!((n[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6 && n[2] < 1e-3, "scaled back to 1");
        assert_eq!(read_normal(&[128, 200, 0], false)[1], -read_normal(&[128, 200, 0], true)[1]);
    }

    fn scratch(name: &str) -> PathBuf {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/scratch").join(name);
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn options(dump: &Path, work: PathBuf) -> Options {
        Options {
            dump: Some(dump.to_path_buf()),
            normalgen: None,
            gen_y_up: true,
            strength: STRENGTH,
            detail: DETAIL,
            min_uses: 1,
            work,
            force: false,
            keep_work: false,
            stall: 300,
        }
    }

    fn put(d: &Path, name: &str, i: &Rgba) {
        std::fs::write(d.join(format!("{name}.png")), png::encode_rgba(i.width, i.height, &i.data)).unwrap()
    }

    #[test]
    fn reruns_keep_hand_edits_and_remake_what_is_gone() {
        let dir = scratch("texpack-materials-test");
        let (dump, pack) = (dir.join("dump"), dir.join("pack"));
        std::fs::create_dir_all(&dump).unwrap();
        std::fs::create_dir_all(&pack).unwrap();
        // Two colours of one surface (a: 2x in the pack, b drawn more but
        // without a pack image), an image without surf=, one drawn once, a
        // tile.
        let (a, b, c, d) = (
            "tex_00000000000000aa_16x16",
            "tex_00000000000000bb_16x16",
            "tex_00000000000000cc_8x8",
            "tex_00000000000000dd_8x8",
        );
        put(&dump, a, &noise(16, 16, 3));
        put(&dump, b, &noise(16, 16, 4));
        put(&dump, c, &noise(8, 8, 5));
        put(&dump, d, &noise(8, 8, 6));
        put(&pack, a, &noise(32, 32, 7));
        put(&pack, "tile_00000000000000ee", &noise(32, 32, 8));
        std::fs::write(
            dump.join("index.txt"),
            format!(
                "# x\n{a}\tuses=2\tsurf=0000000000000011\twrap=rr\n{b}\tuses=9\tsurf=0000000000000011\twrap=mr\n\
                 {c}\tuses=3\twrap=rm\n{d}\tuses=1\tsurf=0000000000000022\n\
                 tile_00000000000000ee\tuses=4\n"
            ),
        )
        .unwrap();
        let mut o = options(&dump, dir.join("work"));
        o.min_uses = 2;
        run(&pack, &o, "topskatr").unwrap();
        let surf = pack.join("surf_0000000000000011_16x16.n.png");
        let own = pack.join(format!("{c}.n.png"));
        let m = up::decode(&surf).unwrap();
        assert_eq!((m.width, m.height), (32, 32), "from the member with a pack image, at its size");
        assert_eq!(up::decode(&own).unwrap().width, 8, "no surf=: per image, from the dump");
        assert!(!pack.join("surf_0000000000000022_8x8.n.png").exists(), "below MIN_USES");
        assert!(!pack.join("tile_00000000000000ee.n.png").exists(), "tiles never");
        let mf = Manifest::parse(&std::fs::read_to_string(pack.join("pack.txt")).unwrap()).unwrap();
        assert_eq!(
            (mf.format, mf.game.as_deref(), &mf.uses[..]),
            (2, Some("topskatr"), &["materials".to_string()][..])
        );
        let index = std::fs::read_to_string(pack.join("index.txt")).unwrap();
        assert!(index.contains("surf_0000000000000011_16x16.n.png") && index.contains(&format!("{c}.n.png")));
        assert!(crate::pack_lint::lint(&pack, Some(&dump), 8192).unwrap());
        let made = std::fs::read(&own).unwrap();
        // Nothing new: nothing redone.
        let before = up::stamp(&surf);
        std::thread::sleep(std::time::Duration::from_millis(20));
        run(&pack, &o, "topskatr").unwrap();
        assert_eq!(up::stamp(&surf), before);
        // Edit one by hand, delete the other: the edit is kept, the
        // deleted one made again, the same.
        let edited = png::encode_rgb(32, 32, &[128, 140, 250].repeat(32 * 32));
        std::fs::write(&surf, &edited).unwrap();
        std::fs::remove_file(&own).unwrap();
        run(&pack, &o, "topskatr").unwrap();
        assert_eq!(std::fs::read(&surf).unwrap(), edited);
        assert_eq!(std::fs::read(&own).unwrap(), made);
        // A changed source is redone (the map is still ours).
        put(&dump, c, &noise(8, 8, 9));
        run(&pack, &o, "topskatr").unwrap();
        assert_ne!(std::fs::read(&own).unwrap(), made);
        // Other settings: refused; forced, ours remade, the edit still kept.
        o.strength = 2.0;
        assert!(run(&pack, &o, "topskatr").unwrap_err().contains("other settings"));
        o.force = true;
        let before = std::fs::read(&own).unwrap();
        run(&pack, &o, "topskatr").unwrap();
        assert_ne!(std::fs::read(&own).unwrap(), before);
        assert_eq!(std::fs::read(&surf).unwrap(), edited);
        // A hand-written manifest: only `uses` changes, and the maps follow
        // its green direction.
        std::fs::write(pack.join("pack.txt"), "# mine\nformat 2\nname my pack\nnormal_y down\n").unwrap();
        run(&pack, &o, "topskatr").unwrap();
        let text = std::fs::read_to_string(pack.join("pack.txt")).unwrap();
        assert_eq!(text, "# mine\nformat 2\nname my pack\nuses\tmaterials\nnormal_y down\n");
        assert!(std::fs::read_to_string(pack.join(RECORD)).unwrap().contains("normal_y=down"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn drives_an_external_generator() {
        let dir = scratch("texpack-materials-ext-test");
        let (dump, pack) = (dir.join("dump"), dir.join("pack"));
        std::fs::create_dir_all(&dump).unwrap();
        std::fs::create_dir_all(&pack).unwrap();
        let name = "tex_00000000000000aa_8x8";
        put(&dump, name, &img(8, 8, |x, _| [50, 50, 50, if x == 7 { 0 } else { 255 }]));
        std::fs::write(dump.join("index.txt"), format!("{name}\tuses=1\twrap=rr\n")).unwrap();
        // A "generator" that copies its input: grey 50 reads as a normal
        // leaning left and (green down) up, which comes back as stored.
        let mut o = options(&dump, dir.join("work"));
        o.normalgen = Some("cp {in}/*.png {out}/".into());
        o.gen_y_up = false;
        run(&pack, &o, "topskatr").unwrap();
        let m = up::decode(&pack.join(format!("{name}.n.png"))).unwrap();
        let want = encode(&[read_normal(&[50, 50, 50], false)], true);
        assert_eq!(m.data[..3], want[..]);
        assert!(!o.work.exists(), "work files removed");
        // Output of the wrong size: that map fails.
        o.normalgen = Some(format!("cp {}/{name}.png {{out}}/{name}.png", dump.display()));
        o.force = true;
        assert!(run(&pack, &o, "topskatr").unwrap_err().contains("1 maps failed"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
