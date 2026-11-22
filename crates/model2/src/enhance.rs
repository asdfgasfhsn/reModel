//! Texture dumping and replacement (texture packs): a modern enhancement,
//! not hardware. The renderers call an `Enhance` implementation for every
//! textured polygon and every on-screen tile pattern; it can record them
//! (dumping) and hand back a replacement image to sample instead.
//!
//! Keys are content hashes, so they do not depend on where the game put a
//! texture in render RAM this time: see docs/texture-packs.md.

use crate::raster::Sources;
use std::sync::Arc;

/// An 8-bit RGBA image, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

/// Filtering of texture pack images (originals are never filtered this
/// way). `Off` samples the image bilinearly as is, the reference; `Mip`
/// blends the two nearest levels of its mip chain (trilinear); `Aniso(n)`
/// adds up to n-times anisotropic filtering (GPU only: the software
/// renderer draws it as `Mip`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TexFilter {
    #[default]
    Off,
    Mip,
    Aniso(u8),
}

impl TexFilter {
    /// The setting's spelling: `off`, `mip`, `2`, `4`, `8` or `16`.
    pub fn parse(s: &str) -> Option<TexFilter> {
        match s {
            "off" | "0" => Some(TexFilter::Off),
            "mip" | "1" => Some(TexFilter::Mip),
            "2" | "4" | "8" | "16" => s.parse().ok().map(TexFilter::Aniso),
            _ => None,
        }
    }

    /// Mip levels are used (`Mip` or `Aniso`).
    pub fn mipmapped(self) -> bool {
        self != TexFilter::Off
    }
}

impl std::fmt::Display for TexFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TexFilter::Off => f.write_str("off"),
            TexFilter::Mip => f.write_str("mip"),
            TexFilter::Aniso(n) => write!(f, "{n}"),
        }
    }
}

/// How a sampling coordinate outside the image is brought back inside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Address {
    Repeat,
    Mirror,
    Clamp,
}

/// The pixel `c` (any integer) samples on an axis of `size` pixels.
pub fn address(c: i32, size: usize, mode: Address) -> usize {
    let s = size as i32;
    (match mode {
        Address::Repeat => c.rem_euclid(s),
        Address::Mirror => {
            let m = c.rem_euclid(2 * s);
            if m >= s {
                2 * s - 1 - m
            } else {
                m
            }
        }
        Address::Clamp => c.clamp(0, s - 1),
    }) as usize
}

impl Rgba {
    /// Bilinear sample at (x, y) in pixels (pixel centres at +0.5). Returns
    /// RGB (0-255, weighted by alpha so transparent texels do not darken
    /// edges) and alpha (0-1).
    pub fn sample(&self, x: f32, y: f32, ax: Address, ay: Address) -> ([f32; 3], f32) {
        let (x, y) = (x - 0.5, y - 0.5);
        let (fx, fy) = (x - x.floor(), y - y.floor());
        let (x0, y0) = (x.floor() as i32, y.floor() as i32);
        let (mut rgb, mut alpha) = ([0.0f32; 3], 0.0f32);
        for (dx, dy, w) in
            [(0, 0, (1.0 - fx) * (1.0 - fy)), (1, 0, fx * (1.0 - fy)), (0, 1, (1.0 - fx) * fy), (1, 1, fx * fy)]
        {
            let px = address(x0 + dx, self.width, ax);
            let py = address(y0 + dy, self.height, ay);
            let o = (py * self.width + px) * 4;
            let wa = w * self.data[o + 3] as f32 / 255.0;
            for (c, &v) in rgb.iter_mut().zip(&self.data[o..o + 3]) {
                *c += wa * v as f32;
            }
            alpha += wa;
        }
        if alpha > 0.0 {
            rgb = rgb.map(|c| c / alpha);
        }
        (rgb, alpha)
    }

    /// Plain bilinear sample of a premultiplied image at (x, y) in pixels
    /// (pixel centres at +0.5): RGBA, 0-255 each, as the GPU's linear
    /// filter of a premultiplied texture.
    pub fn sample_premultiplied(&self, x: f32, y: f32, ax: Address, ay: Address) -> [f32; 4] {
        let (x, y) = (x - 0.5, y - 0.5);
        let (fx, fy) = (x - x.floor(), y - y.floor());
        let (x0, y0) = (x.floor() as i32, y.floor() as i32);
        let mut c = [0.0f32; 4];
        for (dx, dy, w) in
            [(0, 0, (1.0 - fx) * (1.0 - fy)), (1, 0, fx * (1.0 - fy)), (0, 1, (1.0 - fx) * fy), (1, 1, fx * fy)]
        {
            let px = address(x0 + dx, self.width, ax);
            let py = address(y0 + dy, self.height, ay);
            let o = (py * self.width + px) * 4;
            for (c, &v) in c.iter_mut().zip(&self.data[o..o + 4]) {
                *c += w * v as f32;
            }
        }
        c
    }

    /// This image with colour multiplied by alpha (rounded to nearest), the
    /// form filtered sampling and `mip_chain` work in.
    pub fn premultiplied(&self) -> Rgba {
        let mut data = self.data.clone();
        for px in data.as_chunks_mut::<4>().0 {
            let a = px[3] as u32;
            for c in &mut px[..3] {
                *c = ((*c as u32 * a + 127) / 255) as u8;
            }
        }
        Rgba { width: self.width, height: self.height, data }
    }

    /// Premultiplied mip levels, level 0 (this image premultiplied) first,
    /// each half the size of the last (rounded down, at least 1) down to
    /// 1x1. Each texel is a box filter over the area it covers in the level
    /// above: 2x2 texels for even sizes, while an odd size weights the three
    /// source rows or columns by their overlap. Values are averaged as
    /// stored (display values, not linear light) and rounded to nearest.
    pub fn mip_chain(&self) -> Vec<Rgba> {
        let mut levels = vec![self.premultiplied()];
        loop {
            let last = levels.last().unwrap();
            if last.width <= 1 && last.height <= 1 {
                break;
            }
            let next = last.half();
            levels.push(next);
        }
        levels
    }

    /// The next mip level down: see `mip_chain`.
    fn half(&self) -> Rgba {
        let (w, h) = ((self.width / 2).max(1), (self.height / 2).max(1));
        let (xs, ys) = (box_weights(self.width, w), box_weights(self.height, h));
        // Weights are in 1/w (1/h) of a source texel; each list of them sums
        // to the source width (height).
        let total = (self.width * self.height) as u64;
        let mut data = Vec::with_capacity(w * h * 4);
        for ry in &ys {
            for rx in &xs {
                let mut sum = [0u64; 4];
                for &(sy, wy) in ry {
                    for &(sx, wx) in rx {
                        let o = (sy * self.width + sx) * 4;
                        for (s, &v) in sum.iter_mut().zip(&self.data[o..o + 4]) {
                            *s += v as u64 * wx * wy;
                        }
                    }
                }
                data.extend(sum.map(|s| ((s + total / 2) / total) as u8));
            }
        }
        Rgba { width: w, height: h, data }
    }
}

/// Trilinear sample of a mip chain (`Rgba::mip_chain`) at texture
/// coordinates (s, t), 0-1 over the image, at level of detail `lod` (log2
/// of texels of level 0 per pixel; clamped to the chain, magnified below 0):
/// a bilinear sample of the two levels around `lod`, blended. Returns RGB
/// (0-255, un-premultiplied) and alpha (0-1), as `Rgba::sample`.
pub fn sample_trilinear(chain: &[Rgba], s: f32, t: f32, lod: f32, ax: Address, ay: Address) -> ([f32; 3], f32) {
    let top = chain.len().saturating_sub(1) as f32;
    let lod = if lod.is_nan() { 0.0 } else { lod.clamp(0.0, top) };
    let l0 = lod.floor() as usize;
    let f = lod - l0 as f32;
    let at = |l: &Rgba| l.sample_premultiplied(s * l.width as f32, t * l.height as f32, ax, ay);
    let mut c = at(&chain[l0]);
    if f > 0.0 && l0 + 1 < chain.len() {
        let c1 = at(&chain[l0 + 1]);
        for (a, b) in c.iter_mut().zip(c1) {
            *a += (b - *a) * f;
        }
    }
    let alpha = c[3] / 255.0;
    let rgb = if c[3] > 0.0 { [0, 1, 2].map(|k| c[k] / c[3] * 255.0) } else { [0.0; 3] };
    (rgb, alpha)
}

/// A texture's material maps (normal, specular scale, gloss) assembled into
/// one image for the renderers (docs/texture-packs.md, "Material maps"):
/// RGBA8 data, not colour, per texel normal x and y (encoded as in a `.n`
/// file, `(v - 128) / 127`, with y pointing along increasing v whatever the
/// pack's `normal_y`), specular scale, gloss. A map the pack doesn't have
/// is neutral (128). z is not stored: `sqrt(max(1 - x² - y², 0))`.
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
    /// `MapKind::bit` of each map present.
    pub has: u8,
    /// Strengths from the manifest, pack-wide times the map's `material`
    /// line: of the normal map and of the specular map.
    pub normal_scale: f32,
    pub spec_scale: f32,
    /// Identifies the material: a hash of the file stems it was assembled
    /// from (`material_id`). Textures sharing all their maps share it.
    pub id: u64,
}

/// A channel's value where the pack has no map of its kind.
pub const NEUTRAL: u8 = 128;

/// A material's identity from the stems of its normal, specular and gloss
/// maps (None: no such map).
pub fn material_id(stems: [Option<&str>; 3]) -> u64 {
    let mut f = Fnv::default();
    for s in stems {
        if let Some(s) = s {
            f.bytes(s.as_bytes());
        }
        f.byte(0);
    }
    f.0
}

/// A normal component from its byte: `(v - 128) / 127`, within -1..1.
fn unit(v: u8) -> f32 {
    ((v as f32 - 128.0) / 127.0).clamp(-1.0, 1.0)
}

/// A normal component's byte (`unit`'s inverse; 0 is written as 1).
fn encode_unit(x: f32) -> u8 {
    (128.0 + 127.0 * x).round().clamp(1.0, 255.0) as u8
}

impl Material {
    /// Assemble a material from its decoded files (normal, specular, gloss;
    /// None: none, or unreadable), named `stems` (the same order; None: no
    /// such file), under the pack's manifest `m`. The material is the size of
    /// the normal map, else the specular's, else the gloss's; the others are
    /// resampled to it bilinearly with repeat addressing. Normal x and y come
    /// from red and green (alpha and blue ignored), flipped to point along
    /// increasing v with `normal_y up`, and scaled back to length 1 where
    /// longer; specular and gloss from red. Without any map: 1x1, neutral.
    pub fn assemble(maps: [Option<&Rgba>; 3], m: &crate::pack::Manifest, stems: [Option<&str>; 3]) -> Material {
        let [n, s, g] = maps;
        let (width, height) = [n, s, g].into_iter().flatten().next().map_or((1, 1), |b| (b.width, b.height));
        let mut data = vec![NEUTRAL; width * height * 4];
        let mut has = 0;
        if let Some(n) = n {
            has |= crate::pack::MapKind::Normal.bit();
            for (o, i) in data.as_chunks_mut::<4>().0.iter_mut().zip(n.data.as_chunks::<4>().0) {
                let (x, mut y) = (unit(i[0]), unit(i[1]));
                if m.normal_y_up {
                    y = -y;
                }
                let len = (x * x + y * y).sqrt();
                let k = if len > 1.0 { 1.0 / len } else { 1.0 };
                o[0] = encode_unit(x * k);
                o[1] = encode_unit(y * k);
            }
        }
        for (ch, map, kind) in [(2, s, crate::pack::MapKind::Spec), (3, g, crate::pack::MapKind::Gloss)] {
            let Some(map) = map else { continue };
            has |= kind.bit();
            let same = map.width == width && map.height == height;
            for y in 0..height {
                for x in 0..width {
                    data[(y * width + x) * 4 + ch] = if same {
                        map.data[(y * width + x) * 4]
                    } else {
                        let sx = (x as f32 + 0.5) * map.width as f32 / width as f32;
                        let sy = (y as f32 + 0.5) * map.height as f32 / height as f32;
                        map.sample_red(sx, sy)
                    };
                }
            }
        }
        // Per-map strengths: the `material` line of the stem a map was found
        // under (without its suffix), times the pack-wide value.
        let line = |stem: Option<&str>| {
            let base = stem?.rsplit_once('.').map_or(stem?, |(b, _)| b);
            m.materials.get(base)
        };
        let normal_scale = m.normal_scale * line(stems[0]).and_then(|p| p.normal).unwrap_or(1.0);
        let spec_scale = m.spec_scale * line(stems[1]).and_then(|p| p.spec).unwrap_or(1.0);
        Material { width, height, data, has, normal_scale, spec_scale, id: material_id(stems) }
    }

    /// Mip levels, level 0 (this material) first, down to 1x1, with the
    /// box weights of `Rgba::mip_chain` (odd sizes by overlap). Normals are
    /// averaged as unit vectors and renormalised (averaging their bytes
    /// would shorten them, which with z reconstructed flattens nothing and
    /// tilts everything); a sum shorter than 1e-6 is flat. Specular and gloss
    /// are plain averages. Never premultiplied: maps have no alpha.
    pub fn mip_chain(&self) -> Vec<Material> {
        let mut levels = vec![self.clone()];
        loop {
            let last = levels.last().unwrap();
            if last.width <= 1 && last.height <= 1 {
                break;
            }
            let next = last.half();
            levels.push(next);
        }
        levels
    }

    /// The next mip level down: see `mip_chain`.
    fn half(&self) -> Material {
        let (w, h) = ((self.width / 2).max(1), (self.height / 2).max(1));
        let (xs, ys) = (box_weights(self.width, w), box_weights(self.height, h));
        let total = (self.width * self.height) as u64;
        let mut data = Vec::with_capacity(w * h * 4);
        for ry in &ys {
            for rx in &xs {
                let mut n = [0f64; 3];
                let mut sum = [0u64; 2];
                for &(sy, wy) in ry {
                    for &(sx, wx) in rx {
                        let o = (sy * self.width + sx) * 4;
                        let px = &self.data[o..o + 4];
                        let (x, y) = (unit(px[0]), unit(px[1]));
                        let z = (1.0 - x * x - y * y).max(0.0).sqrt();
                        let wt = (wx * wy) as f64;
                        for (a, v) in n.iter_mut().zip([x, y, z]) {
                            *a += wt * v as f64;
                        }
                        sum[0] += px[2] as u64 * wx * wy;
                        sum[1] += px[3] as u64 * wx * wy;
                    }
                }
                // The weights sum to `total`: the mean vector's length is
                // norm / total.
                let norm = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                let (x, y) = if norm / (total as f64) < 1e-6 {
                    (NEUTRAL, NEUTRAL)
                } else {
                    (encode_unit((n[0] / norm) as f32), encode_unit((n[1] / norm) as f32))
                };
                data.extend([x, y].into_iter().chain(sum.map(|s| ((s + total / 2) / total) as u8)));
            }
        }
        Material { width: w, height: h, data, ..*self }
    }
}

impl Material {
    /// The normal at (x, y) in texels of this level (texel centres at +0.5),
    /// sampled bilinearly with addressing `ax`, `ay`: (x, y) components, each
    /// `(v - 128) / 127` of the filtered bytes (z is not stored). A flat
    /// texel (128) gives exactly 0.
    pub fn sample_normal(&self, x: f32, y: f32, ax: Address, ay: Address) -> (f32, f32) {
        let (x, y) = (x - 0.5, y - 0.5);
        let (fx, fy) = (x - x.floor(), y - y.floor());
        let (x0, y0) = (x.floor() as i32, y.floor() as i32);
        let (mut nx, mut ny) = (0.0f32, 0.0f32);
        for (dx, dy, w) in
            [(0, 0, (1.0 - fx) * (1.0 - fy)), (1, 0, fx * (1.0 - fy)), (0, 1, (1.0 - fx) * fy), (1, 1, fx * fy)]
        {
            let px = address(x0 + dx, self.width, ax);
            let py = address(y0 + dy, self.height, ay);
            let o = (py * self.width + px) * 4;
            nx += w * (self.data[o] as f32 - 128.0);
            ny += w * (self.data[o + 1] as f32 - 128.0);
        }
        ((nx / 127.0).clamp(-1.0, 1.0), (ny / 127.0).clamp(-1.0, 1.0))
    }
}

impl Rgba {
    /// Bilinear sample of the red channel alone at (x, y) in pixels (pixel
    /// centres at +0.5), repeat addressing, rounded: how a map of another
    /// size is brought to its material's.
    fn sample_red(&self, x: f32, y: f32) -> u8 {
        let (x, y) = (x - 0.5, y - 0.5);
        let (fx, fy) = (x - x.floor(), y - y.floor());
        let (x0, y0) = (x.floor() as i32, y.floor() as i32);
        let mut v = 0.0f32;
        for (dx, dy, w) in
            [(0, 0, (1.0 - fx) * (1.0 - fy)), (1, 0, fx * (1.0 - fy)), (0, 1, (1.0 - fx) * fy), (1, 1, fx * fy)]
        {
            let px = address(x0 + dx, self.width, Address::Repeat);
            let py = address(y0 + dy, self.height, Address::Repeat);
            v += w * self.data[(py * self.width + px) * 4] as f32;
        }
        v.round().clamp(0.0, 255.0) as u8
    }
}

/// A texture pack's answer for a texture's material maps.
#[derive(Clone, Debug)]
pub enum FoundMaterial {
    /// The maps, assembled.
    Maps(Arc<Material>),
    /// In the pack but not all of its files decoded yet: ask again later
    /// (the texture is drawn without them meanwhile).
    Pending,
    /// The pack has no maps for this texture (callers may cache this).
    Missing,
}

/// A pack whose files arrive one at a time when asked for (the browser's
/// fetches), for `gather_material`.
pub trait MapSource {
    /// The pack has the file `stem` (and it hasn't failed to load).
    fn has(&self, stem: &str) -> bool;
    /// It has arrived, decoded.
    fn arrived(&self, stem: &str) -> bool;
    /// Ask for it (asking again before it arrives does nothing).
    fn ask(&mut self, stem: &str);
    /// Take it once arrived.
    fn take(&mut self, stem: &str) -> Option<Arc<Rgba>>;
}

/// `TexturePack::material` for a pack of files that arrive when asked for:
/// the material's files by `pack::resolve_maps`, asked for until all have
/// arrived (a failed one is left out once the source no longer `has` it),
/// then taken and assembled.
pub fn gather_material(src: &mut dyn MapSource, info: &TexInfo, m: &crate::pack::Manifest) -> FoundMaterial {
    let stems = crate::pack::resolve_maps(info, |s| src.has(s));
    if stems.iter().all(Option::is_none) {
        return FoundMaterial::Missing;
    }
    let mut pending = false;
    for s in stems.iter().flatten() {
        if !src.arrived(s) {
            pending = true;
            src.ask(s);
        }
    }
    if pending {
        return FoundMaterial::Pending;
    }
    let files = stems.each_ref().map(|s| src.take(s.as_deref()?));
    let mat = Material::assemble(files.each_ref().map(Option::as_deref), m, stems.each_ref().map(Option::as_deref));
    FoundMaterial::Maps(Arc::new(mat))
}

/// For each of `to` texels standing for `from` source texels, the source
/// texels it overlaps and by how much, in units of 1/`to` of a source texel.
fn box_weights(from: usize, to: usize) -> Vec<Vec<(usize, u64)>> {
    (0..to)
        .map(|i| {
            // Destination texel i is [i * from, (i + 1) * from) in these
            // units; source texel j is [j * to, (j + 1) * to).
            let (lo, hi) = (i * from, (i + 1) * from);
            (lo / to..hi.div_ceil(to)).map(|j| (j, (hi.min((j + 1) * to) - lo.max(j * to)) as u64)).collect()
        })
        .collect()
}

/// 64-bit FNV-1a.
#[derive(Clone, Copy)]
pub struct Fnv(pub u64);

impl Default for Fnv {
    fn default() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    pub fn byte(&mut self, b: u8) {
        self.0 = (self.0 ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }

    pub fn bytes(&mut self, bs: &[u8]) {
        for &b in bs {
            self.byte(b);
        }
    }
}

/// A textured polygon's texture, as dumps and packs see it.
#[derive(Clone, Debug)]
pub struct TexInfo {
    /// Hash of the texels plus the header fields that change how they look.
    pub key: u64,
    /// Hash of the texels and the size only (the surface key): the same
    /// texels drawn in any colour, luma table or translucency share it.
    /// Material maps key by it (`pack`); images never do.
    pub surface: u64,
    pub width: u32,
    pub height: u32,
    /// The polygon's 4-word texture header (docs/hw/geometrizer.md 5.3).
    pub header: [u16; 4],
    /// Polygon colour (xBGR555, palette entry 0x1000 + header[3] >> 6).
    pub colour: u16,
    pub bank: u32,
    pub x0: u32,
    pub y0: u32,
    pub luma_table: u32,
    pub translucent: bool,
}

impl TexInfo {
    /// File stem in dumps and packs.
    pub fn name(&self) -> String {
        format!("tex_{:016x}_{}x{}", self.key, self.width, self.height)
    }

    /// File stem of the image a pack may give the whole artwork
    /// (`art_<surface>_<w>x<h>`): drawn for this colouring when the pack has
    /// no `tex_` image of its own for it.
    pub fn art_name(&self) -> String {
        format!("art_{:016x}_{}x{}", self.surface, self.width, self.height)
    }

    /// File stem of material maps shared by every colour of these texels
    /// (`surf_<surface>_<w>x<h>`, with a map suffix in a pack).
    pub fn surface_name(&self) -> String {
        format!("surf_{:016x}_{}x{}", self.surface, self.width, self.height)
    }
}

/// An 8x8 tile pattern with the colours it is drawn in.
#[derive(Clone, Debug)]
pub struct TileInfo {
    /// Hash of the pens plus the palette colours of the pens it uses.
    pub key: u64,
    /// Tile number (0-0x3fff).
    pub tile: u16,
    /// Palette bank: entries bank * 16 + pen.
    pub bank: u16,
    /// Pens, row by row.
    pub pens: [u8; 64],
    /// xBGR555 colours of the bank's 16 pens.
    pub colours: [u16; 16],
}

impl TileInfo {
    pub fn name(&self) -> String {
        format!("tile_{:016x}", self.key)
    }

    /// The tile as the game draws it: pen 0 is transparent (alpha 0) but
    /// keeps its colour, which the opaque back plane shows.
    pub fn image(&self) -> Rgba {
        let mut data = Vec::with_capacity(64 * 4);
        for &p in &self.pens {
            let c = crate::video::rgb(self.colours[p as usize]);
            data.extend_from_slice(&[c[0], c[1], c[2], if p == 0 { 0 } else { 255 }]);
        }
        Rgba { width: 8, height: 8, data }
    }
}

/// Hooks the renderers call when given one (`raster::draw_enhanced`,
/// `video::compose_enhanced`).
pub trait Enhance {
    /// A textured polygon is about to be drawn: its replacement, if any.
    fn texture(&mut self, info: &TexInfo) -> Option<Arc<Rgba>>;
    /// `texture` as a premultiplied mip chain (`Rgba::mip_chain`), for
    /// filtered drawing (`TexFilter::Mip`). The default builds it every
    /// time; implementations keep it.
    fn texture_mips(&mut self, info: &TexInfo) -> Option<Arc<Vec<Rgba>>> {
        self.texture(info).map(|img| Arc::new(img.mip_chain()))
    }
    /// The polygon covered `pixels` output pixels (`raster::texture_image`
    /// renders the original texture for a dump).
    fn texture_drawn(&mut self, info: &TexInfo, src: &Sources, pixels: usize);
    /// A tile pattern is on screen this frame: its replacement, if any
    /// (any multiple of 8x8, sampled at output resolution).
    fn tile(&mut self, info: &TileInfo) -> Option<Arc<Rgba>>;
    /// The pack's region overrides for the 2D layers (`regions`), if it
    /// has any: matched against each frame's tile maps and drawn over the
    /// tile patterns.
    fn regions(&mut self) -> Option<Arc<crate::regions::RegionSet>> {
        None
    }
    /// A textured polygon with a lamp (`light::Options::lamps`, materials
    /// on) is about to be drawn: its material maps, if the pack has them.
    /// Never asked for otherwise.
    fn texture_material(&mut self, _info: &TexInfo) -> Option<Arc<Material>> {
        None
    }
    /// `texture_material` as a mip chain (`Material::mip_chain`), for filtered
    /// drawing. The default builds it every time; implementations keep it.
    fn texture_material_mips(&mut self, info: &TexInfo) -> Option<Arc<Vec<Material>>> {
        self.texture_material(info).map(|m| Arc::new(m.mip_chain()))
    }
    /// A polygon with a material was drawn; the material changed the luma
    /// of `changed` of its pixels.
    fn material_drawn(&mut self, _info: &TexInfo, _changed: usize) {}
    /// The replacement `texture` gave for `info` is `selflit`: drawn as
    /// painted, not scaled by the game's lighting
    /// (`TexturePack::image_selflit`).
    fn texture_selflit(&mut self, _info: &TexInfo) -> bool {
        false
    }
    /// The `miss` overlay is on: a textured polygon `texture` gave no
    /// replacement for is drawn flat in `raster::MISS_COLOUR`, as painted.
    fn overlay_miss(&mut self) -> bool {
        false
    }
}

/// What [`TexturePack::preload`] hands out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreloadKind {
    Texture,
    Tile,
}

/// A texture pack's answer for one texture.
#[derive(Clone, Debug)]
pub enum Found {
    /// The replacement image (straight alpha).
    Image(Arc<Rgba>),
    /// In the pack but not loaded yet: draw the original this time and ask
    /// again later.
    Pending,
    /// Not in the pack (or unusable): draw the original. Callers may cache
    /// this answer.
    Missing,
}

/// A source of replacement textures for renderers that only look textures
/// up (the GPU renderer), unlike `Enhance`, which also dumps and needs
/// pixel counts.
pub trait TexturePack {
    fn find(&mut self, info: &TexInfo) -> Found;
    /// Up to `max` images nobody has asked for yet, decoded, by key, for
    /// the renderer to upload before their first draw (`Fixes::preload`);
    /// asked again every frame, it also keeps the next ones decoding. A
    /// pack that can't (or has none left) returns none.
    fn preload(&mut self, _kind: PreloadKind, _max: usize) -> Vec<(u64, Arc<Rgba>)> {
        Vec::new()
    }
    /// Start `preload` of `kind` over (the renderer dropped what it held).
    fn preload_restart(&mut self, _kind: PreloadKind) {}
    /// Whether the pack has any tile images (`tile_<key>.png`): without,
    /// the renderer leaves the 2D layers alone.
    fn has_tiles(&self) -> bool {
        false
    }
    /// A tile pattern's replacement, as `find` for textures.
    fn find_tile(&mut self, _info: &TileInfo) -> Found {
        Found::Missing
    }
    /// The pack's region overrides (`regions::RegionSet`), if it has any
    /// that this reader loaded. The renderer takes the 2D path for a pack
    /// with regions even if it has no tile images.
    fn regions(&mut self) -> Option<Arc<crate::regions::RegionSet>> {
        None
    }
    /// Whether the pack has any material maps it reads (a manifest of
    /// format 2 or later and `.n`, `.s` or `.g` files): without, the
    /// renderer never asks for `material`.
    fn has_materials(&self) -> bool {
        false
    }
    /// A texture's material maps, looked up per kind by image name then
    /// surface name (`pack::resolve_maps`), assembled
    /// (`Material::assemble`). Independent of `find`: a material needs no
    /// replacement image. Like images, a material handed over is the
    /// caller's to keep: asked again (after the renderer dropped it), the
    /// pack may load it again.
    fn material(&mut self, _info: &TexInfo) -> FoundMaterial {
        FoundMaterial::Missing
    }
    /// Whether the image this pack draws for `info` is `selflit` (the
    /// manifest's `material STEM selflit`): None if the pack has no image
    /// for it, so a stack asks the layer that supplies it.
    fn image_selflit(&mut self, _info: &TexInfo) -> Option<bool> {
        None
    }
    /// The pack's manifest (`pack::Manifest::NONE`: it has none).
    fn manifest(&self) -> &crate::pack::Manifest {
        &crate::pack::NO_MANIFEST
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addressing() {
        assert_eq!(address(-1, 4, Address::Repeat), 3);
        assert_eq!(address(4, 4, Address::Mirror), 3);
        assert_eq!(address(-1, 4, Address::Mirror), 0);
        assert_eq!(address(9, 4, Address::Clamp), 3);
    }

    #[test]
    fn bilinear_ignores_transparent_colour() {
        // Opaque red next to transparent black: the colour stays red, the
        // alpha falls off.
        let img = Rgba { width: 2, height: 1, data: vec![255, 0, 0, 255, 0, 0, 0, 0] };
        let (rgb, a) = img.sample(1.0, 0.5, Address::Clamp, Address::Clamp);
        assert_eq!(rgb, [255.0, 0.0, 0.0]);
        assert!((a - 0.5).abs() < 1e-6);
        let (rgb, a) = img.sample(0.5, 0.5, Address::Clamp, Address::Clamp);
        assert_eq!((rgb, a), ([255.0, 0.0, 0.0], 1.0));
    }

    fn img(width: usize, height: usize, px: &[[u8; 4]]) -> Rgba {
        Rgba { width, height, data: px.concat() }
    }

    #[test]
    fn premultiplies() {
        let i = img(2, 1, &[[255, 128, 10, 128], [200, 100, 50, 0]]);
        assert_eq!(i.premultiplied(), img(2, 1, &[[128, 64, 5, 128], [0, 0, 0, 0]]));
    }

    #[test]
    fn mip_chain_two_by_two() {
        let i = img(2, 2, &[[0, 0, 0, 255], [255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]]);
        let chain = i.mip_chain();
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0], i);
        // 255 / 4 = 63.75 rounds to 64.
        assert_eq!(chain[1], img(1, 1, &[[64, 64, 64, 255]]));
    }

    #[test]
    fn mip_chain_sizes() {
        let sizes = |w, h| {
            let i = Rgba { width: w, height: h, data: vec![255; w * h * 4] };
            let c = i.mip_chain();
            assert!(c.iter().all(|l| l.data.iter().all(|&v| v == 255)));
            c.iter().map(|l| (l.width, l.height)).collect::<Vec<_>>()
        };
        assert_eq!(sizes(1, 1), [(1, 1)]);
        assert_eq!(sizes(8, 2), [(8, 2), (4, 1), (2, 1), (1, 1)]);
        assert_eq!(sizes(5, 3), [(5, 3), (2, 1), (1, 1)]);
    }

    #[test]
    fn mip_chain_odd_sizes_weigh_by_overlap() {
        // 3 -> 1: all three equally.
        let i = img(3, 1, &[[0, 0, 0, 255], [90, 0, 0, 255], [180, 0, 0, 255]]);
        assert_eq!(i.mip_chain()[1], img(1, 1, &[[90, 0, 0, 255]]));
        // 5 -> 2: texel 2 is split between both halves (weights 2, 2, 1 of 5).
        let i = img(5, 1, &[[100, 0, 0, 255], [100, 0, 0, 255], [200, 0, 0, 255], [0, 0, 0, 255], [0, 0, 0, 255]]);
        assert_eq!(i.mip_chain()[1], img(2, 1, &[[120, 0, 0, 255], [40, 0, 0, 255]]));
        // Odd both ways: 3x3 -> 1x1 is the plain mean.
        let px: Vec<[u8; 4]> = (0..9).map(|k| [k * 10, 0, 0, 255]).collect();
        assert_eq!(img(3, 3, &px).mip_chain()[1], img(1, 1, &[[40, 0, 0, 255]]));
    }

    #[test]
    fn tex_filter_spellings() {
        for (s, f) in
            [("off", TexFilter::Off), ("mip", TexFilter::Mip), ("2", TexFilter::Aniso(2)), ("16", TexFilter::Aniso(16))]
        {
            assert_eq!(TexFilter::parse(s), Some(f));
            assert_eq!(f.to_string(), s);
        }
        assert_eq!(TexFilter::parse("3"), None);
        assert_eq!(TexFilter::default(), TexFilter::Off);
    }

    #[test]
    fn trilinear_matches_bilinear_at_level_0_and_blends_levels() {
        let i = img(2, 2, &[[255, 0, 0, 255], [0, 0, 255, 255], [0, 255, 0, 255], [255, 255, 255, 0]]);
        let chain = i.mip_chain();
        for (x, y) in [(0.5, 0.5), (1.0, 0.75), (1.3, 1.9)] {
            let (rgb, a) = i.sample(x, y, Address::Repeat, Address::Mirror);
            let (rgb2, a2) = sample_trilinear(&chain, x / 2.0, y / 2.0, -1.0, Address::Repeat, Address::Mirror);
            assert!((a - a2).abs() < 1e-5, "{a} {a2}");
            // Premultiplying rounds to 8 bits.
            assert!(rgb.iter().zip(rgb2).all(|(p, q)| (p - q).abs() < 1.5), "{rgb:?} {rgb2:?}");
        }
        // Half way to the 1x1 level, anywhere: the mean of level 0's
        // bilinear sample and the 1x1 level.
        let (_, a) = sample_trilinear(&chain, 0.25, 0.25, 0.5, Address::Repeat, Address::Repeat);
        let top = chain[1].data[3] as f32 / 255.0;
        assert!((a - (1.0 + top) / 2.0).abs() < 1e-5);
        // Beyond the chain: the last level.
        let (rgb, _) = sample_trilinear(&chain, 0.9, 0.1, 9.0, Address::Repeat, Address::Repeat);
        let last = &chain[1].data;
        assert!((rgb[0] - last[0] as f32 / last[3] as f32 * 255.0).abs() < 1e-3);
    }

    /// A map file from per-texel red and green (blue 255, alpha 0: both
    /// ignored).
    fn map(width: usize, height: usize, rg: &[[u8; 2]]) -> Rgba {
        Rgba { width, height, data: rg.iter().flat_map(|&[r, g]| [r, g, 255, 0]).collect() }
    }

    fn manifest(text: &str) -> crate::pack::Manifest {
        crate::pack::Manifest::parse(text).unwrap()
    }

    #[test]
    fn material_fills_missing_maps_with_neutral() {
        let m = manifest("format 2\nnormal_y down\n");
        let n = map(2, 1, &[[200, 60], [128, 128]]);
        let stems = [Some("tex_0123456789abcdef_2x1.n"), None, None];
        let mat = Material::assemble([Some(&n), None, None], &m, stems);
        assert_eq!((mat.width, mat.height, mat.has), (2, 1, 1));
        assert_eq!(mat.data, [200, 60, 128, 128, 128, 128, 128, 128]);
        assert_eq!((mat.normal_scale, mat.spec_scale), (1.0, 1.0));
        assert_eq!(mat.id, material_id(stems));
        // A specular map alone sets the size; normal and gloss are neutral.
        let s = map(1, 2, &[[0, 9], [255, 9]]);
        let mat = Material::assemble([None, Some(&s), None], &m, [None, Some("surf_0123456789abcdef_1x2.s"), None]);
        assert_eq!((mat.width, mat.height, mat.has), (1, 2, 2));
        assert_eq!(mat.data, [128, 128, 0, 128, 128, 128, 255, 128]);
        assert_ne!(mat.id, material_id(stems));
        // Nothing at all: 1x1 neutral.
        let mat = Material::assemble([None, None, None], &m, [None; 3]);
        assert_eq!((mat.width, mat.height, mat.has, &mat.data[..]), (1, 1, 0, &[128u8; 4][..]));
    }

    #[test]
    fn material_resamples_maps_to_the_base_size() {
        let m = manifest("format 2\n");
        let n = Rgba { width: 4, height: 2, data: [128, 128, 255, 255].repeat(8) };
        // A 2x1 specular map: bilinear with repeat, so at the base's texel
        // centres a quarter and three quarters between the two values,
        // wrapping around at the edges.
        let s = map(2, 1, &[[0, 0], [200, 0]]);
        // A 1x1 gloss map: everywhere the same.
        let g = map(1, 1, &[[77, 0]]);
        let mat = Material::assemble([Some(&n), Some(&s), Some(&g)], &m, [Some("a.n"), Some("a.s"), Some("a.g")]);
        assert_eq!((mat.width, mat.height, mat.has), (4, 2, 7));
        let spec: Vec<u8> = mat.data.chunks(4).map(|p| p[2]).collect();
        assert_eq!(spec, [50, 50, 150, 150, 50, 50, 150, 150]);
        assert!(mat.data.chunks(4).all(|p| p[3] == 77 && p[0] == 128 && p[1] == 128));
    }

    #[test]
    fn material_normal_y_and_length() {
        // up (the default): green brighter leans to the image's top, which
        // is decreasing v, so the stored y (along increasing v) flips.
        let n = map(4, 1, &[[128, 228], [128, 28], [255, 0], [0, 128]]);
        let up = Material::assemble([Some(&n), None, None], &manifest("format 2\n"), [Some("a.n"), None, None]);
        let down = Material::assemble(
            [Some(&n), None, None],
            &manifest("format 2\nnormal_y down\n"),
            [Some("a.n"), None, None],
        );
        let rg = |m: &Material| m.data.chunks(4).map(|p| [p[0], p[1]]).collect::<Vec<_>>();
        // (1, -1) is longer than 1: scaled back to (0.707, -0.707), or its
        // mirror; 0 encodes as 1 (both mean -1).
        assert_eq!(rg(&down), [[128, 228], [128, 28], [218, 38], [1, 128]]);
        assert_eq!(rg(&up), [[128, 28], [128, 228], [218, 218], [1, 128]]);
    }

    #[test]
    fn material_strengths_come_from_the_manifest() {
        let m = manifest(
            "format 2\nnormal_scale 2\nspec_scale 0.5\n\
             material\tsurf_0123456789abcdef_2x2\tnormal=0.25\n\
             material\ttex_0123456789abcdef_2x2\tspec=3\tnormal=4\n",
        );
        let n = map(1, 1, &[[128, 128]]);
        let stems = [Some("surf_0123456789abcdef_2x2.n"), Some("tex_0123456789abcdef_2x2.s"), None];
        let mat = Material::assemble([Some(&n), Some(&n), None], &m, stems);
        // Each map takes the line of the stem it was found under.
        assert_eq!((mat.normal_scale, mat.spec_scale), (0.5, 1.5));
    }

    #[test]
    fn material_mips_renormalise_normals() {
        let m = manifest("format 2\nnormal_y down\n");
        let mat = |rg: &[[u8; 2]], w, h| Material::assemble([Some(&map(w, h, rg)), None, None], &m, [None; 3]);
        // Straight right (1, 0, 0) and straight out (0, 0, 1): the mean
        // direction is 45 degrees, x = 0.707 (218); averaging the bytes would
        // give 191 or 192, a normal tilted less.
        let chain = mat(&[[255, 128], [128, 128]], 2, 1).mip_chain();
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[1].data[..2], [218, 128]);
        // Opposite slopes cancel to flat; equal ones are kept exactly.
        assert_eq!(mat(&[[204, 128], [52, 128]], 2, 1).mip_chain()[1].data[..2], [128, 128]);
        assert_eq!(mat(&[[204, 90], [204, 90]], 2, 1).mip_chain()[1].data[..2], [204, 90]);
        // Exactly opposite unit normals in the plane: sum zero, flat.
        assert_eq!(mat(&[[255, 128], [1, 128]], 2, 1).mip_chain()[1].data[..2], [128, 128]);
        // Specular and gloss: plain box averages, with Rgba::mip_chain's
        // weights for odd sizes (5 -> 2: 2, 2, 1 of 5).
        let s = map(5, 1, &[[100, 0], [100, 0], [200, 0], [0, 0], [0, 0]]);
        let c = Material::assemble([None, Some(&s), Some(&s)], &m, [None; 3]).mip_chain();
        assert_eq!(c.iter().map(|l| (l.width, l.height)).collect::<Vec<_>>(), [(5, 1), (2, 1), (1, 1)]);
        assert_eq!(c[1].data, [128, 128, 120, 120, 128, 128, 40, 40]);
        assert!(c.iter().all(|l| l.id == c[0].id && l.has == 6));
        // A flat map stays flat at every level, any size.
        let flat = Material::assemble([Some(&map(7, 3, &[[128, 128]; 21])), None, None], &m, [None; 3]);
        assert!(flat.mip_chain().iter().all(|l| l.data.iter().all(|&v| v == 128)));
    }

    /// Files that arrive when the test says, as the browser's fetches do.
    #[derive(Default)]
    struct Fetched {
        files: std::collections::HashSet<String>,
        failed: std::collections::HashSet<String>,
        ready: std::collections::HashMap<String, Arc<Rgba>>,
        wanted: Vec<String>,
    }

    impl MapSource for Fetched {
        fn has(&self, stem: &str) -> bool {
            self.files.contains(stem) && !self.failed.contains(stem)
        }
        fn arrived(&self, stem: &str) -> bool {
            self.ready.contains_key(stem)
        }
        fn ask(&mut self, stem: &str) {
            if !self.wanted.iter().any(|w| w == stem) {
                self.wanted.push(stem.to_string());
            }
        }
        fn take(&mut self, stem: &str) -> Option<Arc<Rgba>> {
            self.ready.remove(stem)
        }
    }

    #[test]
    fn gathers_a_material_as_its_files_arrive() {
        let info = TexInfo {
            key: 0xab,
            surface: 0xcd,
            width: 8,
            height: 8,
            header: [0; 4],
            colour: 0,
            bank: 0,
            x0: 0,
            y0: 0,
            luma_table: 0,
            translucent: false,
        };
        let (n, s) = (format!("{}.n", info.name()), format!("{}.s", info.surface_name()));
        let m = manifest("format 2\nnormal_y down\n");
        let mut src = Fetched { files: [n.clone(), s.clone()].into_iter().collect(), ..Default::default() };
        assert!(matches!(gather_material(&mut src, &info, &m), FoundMaterial::Pending));
        assert!(matches!(gather_material(&mut src, &info, &m), FoundMaterial::Pending));
        assert_eq!(src.wanted, [n.clone(), s.clone()], "each file asked for");
        // The normal map arrives; the specular map fails.
        src.ready.insert(n.clone(), Arc::new(map(2, 2, &[[200, 60]; 4])));
        assert!(matches!(gather_material(&mut src, &info, &m), FoundMaterial::Pending));
        src.failed.insert(s.clone());
        let FoundMaterial::Maps(mat) = gather_material(&mut src, &info, &m) else { panic!("not assembled") };
        assert_eq!((mat.width, mat.height, mat.has), (2, 2, 1));
        assert_eq!(mat.data[..4], [200, 60, 128, 128]);
        assert_eq!(mat.id, material_id([Some(&n), None, None]));
        assert!(src.ready.is_empty(), "taken");
        // A texture with no maps.
        let other = TexInfo { key: 1, surface: 2, ..info };
        assert!(matches!(gather_material(&mut src, &other, &m), FoundMaterial::Missing));
    }

    #[test]
    fn transparent_texels_do_not_darken_mips() {
        // Opaque red beside transparent green: the averaged texel,
        // un-premultiplied, is still pure red.
        let i = img(2, 1, &[[255, 0, 0, 255], [0, 255, 0, 0]]);
        let m = &i.mip_chain()[1].data;
        assert_eq!(m[..], [128, 0, 0, 128]);
        assert_eq!((m[0] as f32 / m[3] as f32 * 255.0).round(), 255.0);
    }
}
