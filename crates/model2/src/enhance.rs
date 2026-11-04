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

/// How a sampling coordinate outside the image is brought back inside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Address {
    Repeat,
    Mirror,
    Clamp,
}

fn address(c: i32, size: usize, mode: Address) -> usize {
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
    /// The polygon covered `pixels` output pixels (`raster::texture_image`
    /// renders the original texture for a dump).
    fn texture_drawn(&mut self, info: &TexInfo, src: &Sources, pixels: usize);
    /// A tile pattern is on screen this frame: its replacement, if any
    /// (any multiple of 8x8, sampled at output resolution).
    fn tile(&mut self, info: &TileInfo) -> Option<Arc<Rgba>>;
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
}
