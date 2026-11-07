//! 2D tile layer renderer (System 24-style tile chip), per
//! docs/hw/tilemap.md. Everything here is a working hypothesis from public
//! notes; the game's own screens are the test.
//!
//! Tile RAM offsets (bytes, from 0x01000000):
//!   0x0000/0x2000  plane A maps 0/1   0x4000/0x6000  plane B maps 2/3
//!   0x8000 + 0x400*n  line scroll for map n
//!   0xA000 + 2n  HSCROLL[n]   0xA008 + 2n  VSCROLL[n]
//!   0xC000 / 0xD000  window masks for planes A / B
//! Character RAM at +0x80000: 8x8 tiles, 4bpp, 32 bytes per tile.

use crate::enhance::{Address, Enhance, Fnv, Rgba, TileInfo};
use std::collections::HashMap;
use std::sync::Arc;

pub const WIDTH: usize = 496;
pub const HEIGHT: usize = 384;

const CHAR_BASE: usize = 0x8_0000;
const LINE_SCROLL: usize = 0x8000;
const HSCROLL: usize = 0xa000;
const VSCROLL: usize = 0xa008;
const MASK: [usize; 2] = [0xc000, 0xd000];

fn rd16(ram: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([ram[off], ram[off + 1]])
}

/// xBGR555 to RGB888.
pub fn rgb(entry: u16) -> [u8; 3] {
    let c = |v: u16| ((v & 31) << 3 | (v & 31) >> 2) as u8;
    [c(entry), c(entry >> 5), c(entry >> 10)]
}

/// Pen (0-15) of pixel `px` in row `row` of tile `tile`.
fn pen(tiles: &[u8], tile: usize, row: usize, px: usize) -> u8 {
    let off = CHAR_BASE + (tile & 0x3fff) * 32 + row * 4;
    let w = u32::from_le_bytes([tiles[off], tiles[off + 1], tiles[off + 2], tiles[off + 3]]);
    // Each 16-bit half holds 4 pixels, leftmost in the top nibble.
    let half = if px < 4 { w & 0xffff } else { w >> 16 };
    ((half >> (12 - 4 * (px & 3))) & 0xf) as u8
}

/// X scroll of `map` on line `y`: its line-scroll table entry if enabled,
/// else its HSCROLL register.
///
/// In the left/right split modes both maps scroll by the first map's value
/// (the callers pass it), and the same value places the split: the plane
/// is one 1024-pixel-wide surface, map 0 then map 1, scrolled by the low
/// 10 bits. Confirmed from schamp's story captions, which scroll a line
/// longer than one map across both; topskatr's attract text scrolls out
/// to the left and back in from the right the same way.
fn hscroll(tiles: &[u8], map: usize, y: usize) -> u16 {
    let hs = rd16(tiles, HSCROLL + 2 * map);
    if hs & 0x8000 != 0 {
        rd16(tiles, LINE_SCROLL + 0x400 * map + 2 * y)
    } else {
        hs
    }
}

/// Palette index and priority bit for `map` at screen pixel (x, y),
/// scrolled by the registers of `scroll_map`, or None if transparent or
/// disabled.
fn sample(tiles: &[u8], map: usize, scroll_map: usize, x: usize, y: usize, opaque: bool) -> Option<(usize, bool)> {
    let (entry, row, col) = locate(tiles, map, scroll_map, x, y)?;
    let p = pen(tiles, entry, row, col);
    if p == 0 && !opaque {
        return None;
    }
    Some((((entry & 0x7f80) >> 3) | p as usize, entry & 0x8000 != 0))
}

/// Map entry, and row and column within its tile, for `map` at screen
/// pixel (x, y) scrolled by the registers of `scroll_map`; None if the map
/// is disabled.
fn locate(tiles: &[u8], map: usize, scroll_map: usize, x: usize, y: usize) -> Option<(usize, usize, usize)> {
    if rd16(tiles, VSCROLL + 2 * map) & 0x8000 != 0 {
        return None;
    }
    let h = hscroll(tiles, scroll_map, y) as usize & 0x1ff;
    let v = rd16(tiles, VSCROLL + 2 * scroll_map) as usize & 0x1ff;
    let mx = (x + 512 - h) & 511;
    let my = (y + v) & 511;
    let entry = rd16(tiles, map * 0x2000 + ((my / 8) * 64 + mx / 8) * 2) as usize;
    Some((entry, my & 7, mx & 7))
}

/// Which map of `plane` (0 = A, 1 = B) covers screen pixel (x, y), and
/// whose scroll registers apply. The plane mode is in bits 14-13 of the
/// first map's VSCROLL: 0 = per-cell window mask, 1 = top/bottom split,
/// 2-3 = left/right split, with the split point per line taken from the
/// line-scroll table when enabled (split modes use the first map's scroll).
fn map_for(tiles: &[u8], plane: usize, x: usize, y: usize) -> (usize, usize) {
    let first = plane * 2;
    let vs = rd16(tiles, VSCROLL + 2 * first);
    match (vs >> 13) & 3 {
        0 => {
            let word = rd16(tiles, MASK[plane] + y * 8 + (x / 128) * 2);
            let m = first + ((word >> (15 - (x / 8) % 16)) & 1) as usize;
            (m, m)
        }
        1 => {
            let neg = 0u16.wrapping_sub(vs);
            let first_on_top = neg & 0x200 != 0;
            let m = if (y < (neg & 0x1ff) as usize) == first_on_top { first } else { first + 1 };
            (m, first)
        }
        _ => {
            let hs = hscroll(tiles, first, y);
            let first_on_left = hs & 0x200 != 0;
            let m = if (x < (hs & 0x1ff) as usize) == first_on_left { first } else { first + 1 };
            (m, first)
        }
    }
}

/// Render the tile layers to RGB888 (`WIDTH` x `HEIGHT`).
pub fn render_tiles(tiles: &[u8], palette: &[u8]) -> Vec<u8> {
    render_frame(tiles, palette, None)
}

/// Where HUD (high-priority tile) elements go when the view is wider than
/// the game's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hud {
    /// Keep the HUD in the middle, where the game drew it.
    Center,
    /// Move each HUD element towards the edge it sits nearest.
    Edges,
}

/// How a game frame (`WIDTH` x `HEIGHT` game pixels) maps to the output
/// image: a scale, extra game pixels on each side for wider aspect ratios,
/// and HUD placement. An enhancement: the hardware only has the native view.
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub width: usize,
    pub height: usize,
    /// Output pixels per game pixel, horizontally and vertically.
    pub sx: f32,
    pub sy: f32,
    /// Game pixels added on each side of the original 496.
    pub pad: f32,
    pub hud: Hud,
    /// 3D windows holding HUD elements (bit n = window n), placed like
    /// the tile HUD.
    pub hud_windows: u32,
}

impl View {
    /// The hardware's own 496x384.
    pub fn native() -> Self {
        View { width: WIDTH, height: HEIGHT, sx: 1.0, sy: 1.0, pad: 0.0, hud: Hud::Center, hud_windows: 0 }
    }

    /// An output `height` pixels tall with display aspect `aspect` (e.g.
    /// 16/9). The original frame fills a 4:3 display, so its pixels are
    /// slightly wider than tall; wider aspects add game area at the sides.
    pub fn new(height: usize, aspect: f32, hud: Hud) -> Self {
        let sy = height as f32 / HEIGHT as f32;
        let sx = sy * (4.0 / 3.0) / (WIDTH as f32 / HEIGHT as f32);
        let width = (height as f32 * aspect).round() as usize;
        let pad = ((width as f32 / sx - WIDTH as f32) / 2.0).max(0.0);
        View { width, height, sx, sy, pad, hud, hud_windows: 0 }
    }

    /// Horizontal shift in game pixels for a HUD element centred at game x
    /// `mid`: none when centred, else by the third of the screen it is in.
    pub fn hud_shift(&self, mid: f32) -> f32 {
        if self.hud != Hud::Edges {
            return 0.0;
        }
        if mid < WIDTH as f32 / 3.0 {
            -self.pad
        } else if mid > WIDTH as f32 * 2.0 / 3.0 {
            self.pad
        } else {
            0.0
        }
    }

    /// Game-space x of the left edge of the output (negative when widened).
    pub fn x_min(&self) -> f32 {
        -self.pad
    }

    /// Output coordinates of a game-space point.
    pub fn to_output(&self, x: f32, y: f32) -> (f32, f32) {
        ((x + self.pad) * self.sx, y * self.sy)
    }

    /// Where an output point (pixels from the view's top left) falls in the
    /// game's own 496x384 view, as fractions (0 to 1 across and down); None
    /// outside it, including the area a wide view adds at the sides (a gun
    /// pointed there sees no screen).
    pub fn to_view_fraction(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let (gx, gy) = (x / self.sx - self.pad, y / self.sy);
        let (fx, fy) = (gx / WIDTH as f32, gy / HEIGHT as f32);
        ((0.0..1.0).contains(&fx) && (0.0..1.0).contains(&fy)).then_some((fx, fy))
    }
}

/// Compose a frame at native resolution (see `compose`).
pub fn render_frame(tiles: &[u8], palette: &[u8], layer3d: Option<(&[u8], &[bool])>) -> Vec<u8> {
    compose(tiles, palette, layer3d, &View::native())
}

/// Compose a frame, back to front: backdrop; low-priority tiles (plane B
/// opaque, then A); the 3D layer (already at the view's size) where it has
/// coverage; then high-priority tiles (B, then A). Tiles are scaled with
/// nearest-neighbour sampling. With nothing drawn in 3D the frame is a 2D
/// screen and is pillarboxed in a wide view.
pub fn compose(tiles: &[u8], palette: &[u8], layer3d: Option<(&[u8], &[bool])>, view: &View) -> Vec<u8> {
    compose_impl(tiles, palette, layer3d, view, None)
}

/// `compose`, reporting the tile patterns on screen to `hooks` and drawing
/// the replacements it returns, sampled at output resolution.
pub fn compose_enhanced(
    tiles: &[u8],
    palette: &[u8],
    layer3d: Option<(&[u8], &[bool])>,
    view: &View,
    hooks: &mut dyn Enhance,
) -> Vec<u8> {
    compose_impl(tiles, palette, layer3d, view, Some(hooks))
}

/// A tile pixel for the enhanced compose: map entry (with the priority
/// bit), position in the tile, and the replacement (index into the
/// frame's list, NONE for the original pattern).
#[derive(Clone, Copy)]
struct Hit {
    entry: u16,
    row: u8,
    col: u8,
    rep: u16,
}

const NONE: u16 = u16::MAX;

/// Pattern and colours of map entry `entry` (tile and bank), or None for a
/// blank tile (all pen 0), which is not worth replacing.
fn tile_info(tiles: &[u8], palette: &[u8], entry: usize) -> Option<TileInfo> {
    let mut pens = [0u8; 64];
    for (i, p) in pens.iter_mut().enumerate() {
        *p = pen(tiles, entry, i / 8, i % 8);
    }
    if pens.iter().all(|&p| p == 0) {
        return None;
    }
    let base = (entry & 0x7f80) >> 3;
    let colours = std::array::from_fn(|p| rd16(palette, ((base | p) & 0xfff) * 2) & 0x7fff);
    let mut f = Fnv::default();
    f.bytes(&pens);
    let mut used = [false; 16];
    for &p in &pens {
        used[p as usize] = true;
    }
    for p in (0..16).filter(|&p| used[p]) {
        f.byte(p as u8);
        f.bytes(&colours[p].to_le_bytes());
    }
    Some(TileInfo { key: f.0, tile: (entry & 0x3fff) as u16, bank: (base >> 4) as u16, pens, colours })
}

/// Per game pixel: the base layer (backdrop and low-priority tiles), the
/// HUD (high-priority tiles, None where transparent) and, when `enhanced`,
/// the tile hits of plane B then A.
#[allow(clippy::type_complexity)]
fn tile_planes(
    tiles: &[u8],
    palette: &[u8],
    enhanced: bool,
) -> (Vec<[u8; 3]>, Vec<Option<[u8; 3]>>, Vec<[Option<Hit>; 2]>) {
    let mut hits: Vec<[Option<Hit>; 2]> = if enhanced { vec![[None; 2]; WIDTH * HEIGHT] } else { Vec::new() };
    let backdrop = rgb(rd16(palette, 0));
    let mut base = vec![[0u8; 3]; WIDTH * HEIGHT];
    let mut hud = vec![None; WIDTH * HEIGHT];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let mut samples = [None; 2];
            for (slot, (plane, opaque)) in [(1, true), (0, false)].into_iter().enumerate() {
                let (map, scroll) = map_for(tiles, plane, x, y);
                samples[slot] = sample(tiles, map, scroll, x, y, opaque);
                if enhanced {
                    hits[y * WIDTH + x][slot] = locate(tiles, map, scroll, x, y).map(|(entry, row, col)| Hit {
                        entry: entry as u16,
                        row: row as u8,
                        col: col as u8,
                        rep: NONE,
                    });
                }
            }
            let mut c = backdrop;
            let mut h = None;
            for s in samples.iter().flatten() {
                if !s.1 {
                    c = rgb(rd16(palette, (s.0 & 0xfff) * 2));
                } else if s.0 & 0xf != 0 {
                    // Plane B's pen 0 stays transparent in the high-priority pass.
                    h = Some(rgb(rd16(palette, (s.0 & 0xfff) * 2)));
                }
            }
            base[y * WIDTH + x] = c;
            hud[y * WIDTH + x] = h;
        }
    }
    (base, hud, hits)
}

/// The HUD placed element by element in an extended game-space layer
/// `ext_w` pixels wide (the view's width in game pixels). Returns it and
/// `ext_w`.
fn place_hud(hud: &[Option<[u8; 3]>], shift: &[usize], view: &View) -> (Vec<Option<[u8; 3]>>, usize) {
    let ext_w = WIDTH + (2.0 * view.pad).ceil() as usize;
    let mut hud_ext = vec![None; ext_w * HEIGHT];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            if let Some(c) = hud[y * WIDTH + x] {
                let ex = x + shift[(y / 8) * CELLS_X + x / 8];
                if ex < ext_w {
                    hud_ext[y * ext_w + ex] = Some(c);
                }
            }
        }
    }
    (hud_ext, ext_w)
}

/// The 2D layers of a frame for a compositor that blends them with the 3D
/// layer itself (the GPU): output pixel (ox, oy) is at game row
/// `(oy + 0.5) / sy` and extended column `ex = (ox + 0.5) / sx`; the base
/// layer is at `ex - pad` (outside it: `backdrop` when there is 3D, else
/// black), then the 3D layer where drawn, then the HUD at `ex` where opaque.
pub struct Layers2d {
    /// WIDTH x HEIGHT RGBA (alpha 255).
    pub base: Vec<u8>,
    /// `ext_w` x HEIGHT RGBA, alpha 0 where transparent.
    pub hud: Vec<u8>,
    pub ext_w: usize,
    pub backdrop: [u8; 3],
}

/// `tile_planes` without hits, and several times faster: the plane modes,
/// scroll registers and mask words are read once per line, and pixels are
/// produced in runs that share a map and a tile row. Same results.
fn tile_planes_fast(tiles: &[u8], palette: &[u8]) -> (Vec<[u8; 3]>, Vec<Option<[u8; 3]>>) {
    /// No pixel (transparent or map disabled).
    const NONE: u16 = u16::MAX;
    /// Priority bit in a packed pixel (palette index in the low 12 bits).
    const PRIORITY: u16 = 0x8000;
    let pal: Vec<[u8; 3]> = (0..0x1000).map(|i| rgb(rd16(palette, i * 2))).collect();
    let backdrop = rgb(rd16(palette, 0));
    let vscroll: [u16; 4] = std::array::from_fn(|m| rd16(tiles, VSCROLL + 2 * m));
    let mut base = vec![backdrop; WIDTH * HEIGHT];
    let mut hud = vec![None; WIDTH * HEIGHT];
    // Per line, per plane: packed pixels.
    let mut line = [[NONE; WIDTH]; 2];
    for y in 0..HEIGHT {
        for (slot, (plane, opaque)) in [(1usize, true), (0usize, false)].into_iter().enumerate() {
            let first = plane * 2;
            let mode = (vscroll[first] >> 13) & 3;
            // Scroll position of each map this line.
            let scroll: [(usize, usize); 4] =
                std::array::from_fn(|m| (hscroll(tiles, m, y) as usize & 0x1ff, vscroll[m] as usize & 0x1ff));
            // Split modes: (map before, map after, split x, first map first).
            let split = match mode {
                1 => {
                    let neg = 0u16.wrapping_sub(vscroll[first]);
                    let first_on_top = neg & 0x200 != 0;
                    let m = if (y < (neg & 0x1ff) as usize) == first_on_top { first } else { first + 1 };
                    Some((m, m, 0, true))
                }
                2 | 3 => {
                    let hs = hscroll(tiles, first, y);
                    Some((first, first + 1, (hs & 0x1ff) as usize, hs & 0x200 != 0))
                }
                _ => None,
            };
            let out = &mut line[slot];
            let mut x = 0;
            while x < WIDTH {
                // The map at x, and where the choice may change next.
                let (map, sm, limit) = match split {
                    None => {
                        let word = rd16(tiles, MASK[plane] + y * 8 + (x / 128) * 2);
                        let m = first + ((word >> (15 - (x / 8) % 16)) & 1) as usize;
                        (m, m, (x / 8 + 1) * 8)
                    }
                    Some((a, b, at, first_on_left)) => {
                        let before = x < at;
                        let m = if before == first_on_left { a } else { b };
                        (m, first, if before { at } else { WIDTH })
                    }
                };
                let (h, v) = scroll[sm];
                let mx = (x + 512 - h) & 511;
                // The run: up to the end of this tile column or the limit.
                let end = (x + 8 - (mx & 7)).min(limit).min(WIDTH);
                if vscroll[map] & 0x8000 != 0 {
                    out[x..end].fill(NONE);
                    x = end;
                    continue;
                }
                let my = (y + v) & 511;
                let entry = rd16(tiles, map * 0x2000 + ((my / 8) * 64 + mx / 8) * 2) as usize;
                let off = CHAR_BASE + (entry & 0x3fff) * 32 + (my & 7) * 4;
                let w = u32::from_le_bytes([tiles[off], tiles[off + 1], tiles[off + 2], tiles[off + 3]]);
                let colour = ((entry & 0x7f80) >> 3) as u16 | if entry & 0x8000 != 0 { PRIORITY } else { 0 };
                for (i, o) in out[x..end].iter_mut().enumerate() {
                    // Each 16-bit half holds 4 pixels, leftmost in the top nibble.
                    let px = (mx + i) & 7;
                    let half = if px < 4 { w & 0xffff } else { w >> 16 };
                    let p = ((half >> (12 - 4 * (px & 3))) & 0xf) as u16;
                    *o = if p == 0 && !opaque { NONE } else { colour | p };
                }
                x = end;
            }
        }
        let row = y * WIDTH;
        for x in 0..WIDTH {
            for s in [line[0][x], line[1][x]] {
                if s == NONE {
                    continue;
                }
                if s & PRIORITY == 0 {
                    base[row + x] = pal[(s & 0xfff) as usize];
                } else if s & 0xf != 0 {
                    // Plane B's pen 0 stays transparent in the high-priority pass.
                    hud[row + x] = Some(pal[(s & 0xfff) as usize]);
                }
            }
        }
    }
    (base, hud)
}

/// Pixels where the fast 2D layer path differs from the reference one
/// (`remodel gpu-check` runs this over captured frames).
#[doc(hidden)]
pub fn tile_planes_check(tiles: &[u8], palette: &[u8]) -> usize {
    let (b0, h0, _) = tile_planes(tiles, palette, false);
    let (b1, h1) = tile_planes_fast(tiles, palette);
    b0.iter().zip(&b1).filter(|(a, b)| a != b).count() + h0.iter().zip(&h1).filter(|(a, b)| a != b).count()
}

/// The 2D layers for `view`. `has_3d`: whether the frame draws any 3D
/// (moves the HUD to the edges and fills the sides with the backdrop).
pub fn layers_2d(tiles: &[u8], palette: &[u8], view: &View, has_3d: bool) -> Layers2d {
    let (base, hud) = tile_planes_fast(tiles, palette);
    let shift = hud_shifts(&hud, view, has_3d);
    let (hud_ext, ext_w) = place_hud(&hud, &shift, view);
    Layers2d {
        base: base.iter().flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        hud: hud_ext.iter().flat_map(|c| c.map_or([0; 4], |c| [c[0], c[1], c[2], 255])).collect(),
        ext_w,
        backdrop: rgb(rd16(palette, 0)),
    }
}

fn compose_impl(
    tiles: &[u8],
    palette: &[u8],
    layer3d: Option<(&[u8], &[bool])>,
    view: &View,
    hooks: Option<&mut dyn Enhance>,
) -> Vec<u8> {
    let enhanced = hooks.is_some();
    let backdrop = rgb(rd16(palette, 0));
    let (base, hud, hits) = if enhanced {
        tile_planes(tiles, palette, true)
    } else {
        let (base, hud) = tile_planes_fast(tiles, palette);
        (base, hud, Vec::new())
    };
    let has_3d = layer3d.is_some_and(|(_, cov)| cov.iter().any(|&c| c));
    let shift = hud_shifts(&hud, view, has_3d);
    let (hud_ext, ext_w) = place_hud(&hud, &shift, view);

    if let Some(hooks) = hooks {
        let tl = TileLayers { tiles, palette, backdrop, hits, reps: Vec::new() };
        return tl.output(hooks, layer3d, view, &hud, &shift, has_3d);
    }

    let mut out = vec![0u8; view.width * view.height * 3];
    for oy in 0..view.height {
        let gy = ((oy as f32 + 0.5) / view.sy) as usize;
        if gy >= HEIGHT {
            continue;
        }
        for ox in 0..view.width {
            let o = oy * view.width + ox;
            let ex = (ox as f32 + 0.5) / view.sx;
            let gx = ex - view.pad;
            let mut c = if gx >= 0.0 && (gx as usize) < WIDTH {
                base[gy * WIDTH + gx as usize]
            } else if has_3d {
                backdrop
            } else {
                [0, 0, 0]
            };
            if let Some((rgb3d, covered)) = layer3d {
                if covered[o] {
                    c.copy_from_slice(&rgb3d[o * 3..o * 3 + 3]);
                }
            }
            if let Some(h) = hud_ext.get(gy * ext_w + ex as usize).copied().flatten() {
                c = h;
            }
            out[o * 3..o * 3 + 3].copy_from_slice(&c);
        }
    }
    out
}

/// The enhanced compose's tile state for one frame.
struct TileLayers<'a> {
    tiles: &'a [u8],
    palette: &'a [u8],
    backdrop: [u8; 3],
    hits: Vec<[Option<Hit>; 2]>,
    reps: Vec<Arc<Rgba>>,
}

impl TileLayers<'_> {
    /// Ask `hooks` about every tile pattern on screen, once each.
    fn resolve(&mut self, hooks: &mut dyn Enhance) {
        let mut seen: HashMap<u16, u16> = HashMap::new();
        for px in self.hits.iter_mut() {
            for hit in px.iter_mut().flatten() {
                let e = hit.entry & 0x7fff;
                hit.rep = *seen.entry(e).or_insert_with(|| {
                    match tile_info(self.tiles, self.palette, e as usize).and_then(|info| hooks.tile(&info)) {
                        Some(img) if img.width >= 8 && img.height >= 8 => {
                            self.reps.push(img);
                            (self.reps.len() - 1) as u16
                        }
                        _ => NONE,
                    }
                });
            }
        }
    }

    /// Colour of `hit` at (fx, fy) within its game pixel; None if
    /// transparent. A replacement's alpha decides transparency, except on
    /// the opaque back plane, where the original pen shows through.
    fn colour(&self, hit: Hit, fx: f32, fy: f32, opaque: bool) -> Option<[u8; 3]> {
        if let Some(img) = self.reps.get(hit.rep as usize) {
            let x = (hit.col as f32 + fx) * img.width as f32 / 8.0;
            let y = (hit.row as f32 + fy) * img.height as f32 / 8.0;
            let (c, a) = img.sample(x, y, Address::Clamp, Address::Clamp);
            if a >= 0.5 {
                return Some(c.map(|v| v.round().clamp(0.0, 255.0) as u8));
            }
            if !opaque {
                return None;
            }
        }
        let e = hit.entry as usize;
        let p = pen(self.tiles, e, hit.row as usize, hit.col as usize) as usize;
        if p == 0 && !opaque {
            return None;
        }
        Some(rgb(rd16(self.palette, ((((e & 0x7f80) >> 3) | p) & 0xfff) * 2)))
    }

    /// Low-priority tiles over the backdrop at game pixel (x, y).
    fn base(&self, x: usize, y: usize, fx: f32, fy: f32) -> [u8; 3] {
        let mut c = self.backdrop;
        for (slot, hit) in self.hits[y * WIDTH + x].iter().enumerate() {
            if let Some(h) = hit.filter(|h| h.entry & 0x8000 == 0) {
                if let Some(col) = self.colour(h, fx, fy, slot == 0) {
                    c = col;
                }
            }
        }
        c
    }

    /// High-priority tiles at game pixel (x, y); pen 0 is transparent on
    /// both planes.
    fn hud(&self, x: usize, y: usize, fx: f32, fy: f32) -> Option<[u8; 3]> {
        let mut c = None;
        for h in self.hits[y * WIDTH + x].iter().flatten().filter(|h| h.entry & 0x8000 != 0) {
            if let Some(col) = self.colour(*h, fx, fy, false) {
                c = Some(col);
            }
        }
        c
    }

    /// The output loop of `compose`, sampling every layer at output pixel
    /// centres instead of upscaling game pixels.
    fn output(
        mut self,
        hooks: &mut dyn Enhance,
        layer3d: Option<(&[u8], &[bool])>,
        view: &View,
        hud: &[Option<[u8; 3]>],
        shift: &[usize],
        has_3d: bool,
    ) -> Vec<u8> {
        self.resolve(hooks);
        // Extended HUD layer: the source game x of each placed pixel. Pen-0
        // pixels are kept too (a replacement may cover them) unless a
        // drawn pixel lands on the same spot.
        let ext_w = WIDTH + (2.0 * view.pad).ceil() as usize;
        let mut hud_src = vec![u32::MAX; ext_w * HEIGHT];
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                if !self.hits[y * WIDTH + x].iter().flatten().any(|h| h.entry & 0x8000 != 0) {
                    continue;
                }
                let ex = x + shift[(y / 8) * CELLS_X + x / 8];
                if ex < ext_w && (hud[y * WIDTH + x].is_some() || hud_src[y * ext_w + ex] == u32::MAX) {
                    hud_src[y * ext_w + ex] = x as u32;
                }
            }
        }
        let mut out = vec![0u8; view.width * view.height * 3];
        for oy in 0..view.height {
            let gyf = (oy as f32 + 0.5) / view.sy;
            let gy = gyf as usize;
            if gy >= HEIGHT {
                continue;
            }
            let fy = gyf - gy as f32;
            for ox in 0..view.width {
                let o = oy * view.width + ox;
                let ex = (ox as f32 + 0.5) / view.sx;
                let gx = ex - view.pad;
                let mut c = if gx >= 0.0 && (gx as usize) < WIDTH {
                    self.base(gx as usize, gy, gx - gx.floor(), fy)
                } else if has_3d {
                    self.backdrop
                } else {
                    [0, 0, 0]
                };
                if let Some((rgb3d, covered)) = layer3d {
                    if covered[o] {
                        c.copy_from_slice(&rgb3d[o * 3..o * 3 + 3]);
                    }
                }
                let sx = hud_src.get(gy * ext_w + ex as usize).copied().unwrap_or(u32::MAX);
                if sx != u32::MAX {
                    if let Some(h) = self.hud(sx as usize, gy, ex - ex.floor(), fy) {
                        c = h;
                    }
                }
                out[o * 3..o * 3 + 3].copy_from_slice(&c);
            }
        }
        out
    }
}

const CELLS_X: usize = WIDTH / 8 + 1;
const CELLS_Y: usize = HEIGHT / 8;

/// Horizontal shift (in game pixels) for each 8x8 HUD cell. Centred: every
/// cell moves by `pad`. Edges (only with 3D on screen): cells are grouped
/// into elements (touching cells, allowing a one-cell gap so words stay
/// together), and each element stays left, moves right by `2 * pad`, or
/// moves by `pad`, by which third of the screen its centre is in.
fn hud_shifts(hud: &[Option<[u8; 3]>], view: &View, has_3d: bool) -> Vec<usize> {
    let centre = view.pad.round() as usize;
    let mut shift = vec![centre; CELLS_X * CELLS_Y];
    if view.hud != Hud::Edges || !has_3d || view.pad <= 0.0 {
        return shift;
    }
    let mut used = vec![false; CELLS_X * CELLS_Y];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            if hud[y * WIDTH + x].is_some() {
                used[(y / 8) * CELLS_X + x / 8] = true;
            }
        }
    }
    let mut label = vec![usize::MAX; CELLS_X * CELLS_Y];
    for start in 0..used.len() {
        if !used[start] || label[start] != usize::MAX {
            continue;
        }
        let (mut stack, mut members) = (vec![start], Vec::new());
        label[start] = start;
        while let Some(c) = stack.pop() {
            members.push(c);
            let (cx, cy) = ((c % CELLS_X) as i32, (c / CELLS_X) as i32);
            for dy in -2..=2i32 {
                for dx in -2..=2i32 {
                    let (nx, ny) = (cx + dx, cy + dy);
                    if nx < 0 || ny < 0 || nx >= CELLS_X as i32 || ny >= CELLS_Y as i32 {
                        continue;
                    }
                    let n = ny as usize * CELLS_X + nx as usize;
                    if used[n] && label[n] == usize::MAX {
                        label[n] = start;
                        stack.push(n);
                    }
                }
            }
        }
        let (lo, hi) = members.iter().fold((usize::MAX, 0), |(a, b), &c| (a.min(c % CELLS_X), b.max(c % CELLS_X)));
        let mid = (lo + hi + 1) * 4; // centre in game pixels
        let s = if mid < WIDTH / 3 {
            0
        } else if mid > WIDTH * 2 / 3 {
            (2.0 * view.pad).round() as usize
        } else {
            centre
        };
        for c in members {
            shift[c] = s;
        }
    }
    shift
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_fractions_skip_the_added_sides() {
        let v = View::native();
        assert_eq!(v.to_view_fraction(248.0, 192.0), Some((0.5, 0.5)));
        assert_eq!(v.to_view_fraction(-1.0, 10.0), None);
        // 16:9 at 720 lines: the 4:3 picture is the middle 960 pixels.
        let w = View::new(720, 16.0 / 9.0, Hud::Center);
        assert_eq!(w.to_view_fraction(100.0, 360.0), None);
        let (fx, fy) = w.to_view_fraction(640.0, 360.0).unwrap();
        assert!((fx - 0.5).abs() < 1e-3 && (fy - 0.5).abs() < 1e-3);
        let (fx, _) = w.to_view_fraction(161.0, 0.0).unwrap();
        assert!(fx < 0.01);
    }

    #[test]
    fn pixel_order_within_a_row() {
        let mut tiles = vec![0u8; 0x10_0000];
        // Tile 1, row 0: pixels 0..7 = 1..8.
        let off = CHAR_BASE + 32;
        let lo: u16 = 0x1234;
        let hi: u16 = 0x5678;
        tiles[off..off + 2].copy_from_slice(&lo.to_le_bytes());
        tiles[off + 2..off + 4].copy_from_slice(&hi.to_le_bytes());
        let row: Vec<u8> = (0..8).map(|px| pen(&tiles, 1, 0, px)).collect();
        assert_eq!(row, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    struct Rep(Option<Arc<Rgba>>);

    impl Enhance for Rep {
        fn texture(&mut self, _: &crate::enhance::TexInfo) -> Option<Arc<Rgba>> {
            None
        }
        fn texture_drawn(&mut self, _: &crate::enhance::TexInfo, _: &crate::raster::Sources, _: usize) {}
        fn tile(&mut self, info: &TileInfo) -> Option<Arc<Rgba>> {
            assert_eq!(info.tile, 1);
            self.0.clone()
        }
    }

    #[test]
    fn tile_replacement_at_output_resolution() {
        // Plane A shows tile 1 (all pen 3) at the top left; plane B is blank.
        let mut tiles = vec![0u8; 0x10_0000];
        tiles[0..2].copy_from_slice(&1u16.to_le_bytes());
        tiles[CHAR_BASE + 32..CHAR_BASE + 64].fill(0x33);
        let mut palette = vec![0u8; 0x2_0000];
        palette[0..2].copy_from_slice(&0x7c00u16.to_le_bytes()); // backdrop / pen 0: blue
        palette[6..8].copy_from_slice(&0x03e0u16.to_le_bytes()); // pen 3: green
        let view = View { width: 2 * WIDTH, height: 2 * HEIGHT, sx: 2.0, sy: 2.0, ..View::native() };
        let native = compose(&tiles, &palette, None, &view);
        // No replacement: identical to the original path.
        assert_eq!(compose_enhanced(&tiles, &palette, None, &view, &mut Rep(None)), native);
        // A 4x replacement, red on the left half, transparent on the right.
        let mut data = Vec::new();
        for _ in 0..32 {
            for x in 0..32 {
                data.extend_from_slice(if x < 16 { &[255, 0, 0, 255] } else { &[0, 0, 0, 0] });
            }
        }
        let img = Arc::new(Rgba { width: 32, height: 32, data });
        let out = compose_enhanced(&tiles, &palette, None, &view, &mut Rep(Some(img)));
        let px = |x: usize, y: usize| &out[(y * view.width + x) * 3..(y * view.width + x) * 3 + 3];
        assert_eq!(px(0, 0), [255, 0, 0]);
        assert_eq!(px(6, 3), [255, 0, 0]);
        // Right half of the tile: plane B's pen 0 shows through.
        assert_eq!(px(12, 3), [0, 0, 255]);
        assert_eq!(&native[..3], [0, 255, 0]);
    }

    #[test]
    fn split_plane_scrolls_as_one_wide_surface() {
        let mut tiles = vec![0u8; 0x10_0000];
        let put = |t: &mut [u8], off: usize, v: u16| t[off..off + 2].copy_from_slice(&v.to_le_bytes());
        // Plane A in left/right split mode, map 0 with line scroll; line 0
        // scrolls by 0x210: map 0 left of x = 16, map 1 from there.
        put(&mut tiles, VSCROLL, 0x4000);
        put(&mut tiles, HSCROLL, 0x8000);
        put(&mut tiles, LINE_SCROLL, 0x210);
        // Map 0's last column and map 1's first hold tiles 1 and 2 (pens 1, 2).
        put(&mut tiles, 63 * 2, 1);
        put(&mut tiles, 0x2000, 2);
        tiles[CHAR_BASE + 32..CHAR_BASE + 64].fill(0x11);
        tiles[CHAR_BASE + 64..CHAR_BASE + 96].fill(0x22);
        let at = |x| {
            let (map, scroll) = map_for(&tiles, 0, x, 0);
            sample(&tiles, map, scroll, x, 0, false).map(|s| s.0)
        };
        // The two maps meet at the split point with no gap: both scroll by
        // the line's value, not map 1 by its own HSCROLL.
        assert_eq!(at(8), Some(1));
        assert_eq!(at(15), Some(1));
        assert_eq!(at(16), Some(2));
        assert_eq!(at(23), Some(2));
        assert_eq!(at(24), None);
    }

    #[test]
    fn colour_bank_shares_tile_bits() {
        let mut tiles = vec![0u8; 0x10_0000];
        // Map 0 entry (0,0) = tile 0x0080 (bank bits 7..14 = 1), pen 3 everywhere.
        tiles[0..2].copy_from_slice(&0x0080u16.to_le_bytes());
        let off = CHAR_BASE + 0x80 * 32;
        tiles[off..off + 32].fill(0x33);
        assert_eq!(sample(&tiles, 0, 0, 0, 0, false), Some((0x10 | 3, false)));
    }
}
