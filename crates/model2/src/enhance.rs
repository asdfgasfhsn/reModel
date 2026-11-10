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
    /// Whether the pack has any tile images (`tile_<key>.png`): without,
    /// the renderer leaves the 2D layers alone.
    fn has_tiles(&self) -> bool {
        false
    }
    /// A tile pattern's replacement, as `find` for textures.
    fn find_tile(&mut self, _info: &TileInfo) -> Found {
        Found::Missing
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
