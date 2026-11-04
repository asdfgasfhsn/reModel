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
fn rgb(entry: u16) -> [u8; 3] {
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

/// Line-scroll table entry for `map` on line `y` if enabled, else HSCROLL.
fn hscroll(tiles: &[u8], map: usize, y: usize) -> u16 {
    let hs = rd16(tiles, HSCROLL + 2 * map);
    if hs & 0x8000 != 0 {
        rd16(tiles, LINE_SCROLL + 0x400 * map + 2 * y)
    } else {
        hs
    }
}

/// X scroll used for drawing `map` on line `y`. In left/right split mode
/// the line-scroll entry is only the split point, and the map scrolls by its
/// HSCROLL register. Confirmed from topskatr's attract text, which stays
/// still while the split wipes it in and out; MAME instead applies the entry
/// as scroll as well, which makes the text jump between frames.
fn split_scroll(tiles: &[u8], map: usize, y: usize) -> u16 {
    let first = map & 2;
    if (rd16(tiles, VSCROLL + 2 * first) >> 13) & 3 >= 2 {
        rd16(tiles, HSCROLL + 2 * map)
    } else {
        hscroll(tiles, map, y)
    }
}

/// Palette index for `map` at screen pixel (x, y), scrolled by the
/// registers of `scroll_map`, or None if transparent or disabled.
fn sample(tiles: &[u8], map: usize, scroll_map: usize, x: usize, y: usize, opaque: bool) -> Option<usize> {
    if rd16(tiles, VSCROLL + 2 * map) & 0x8000 != 0 {
        return None;
    }
    let h = split_scroll(tiles, scroll_map, y) as usize & 0x1ff;
    let v = rd16(tiles, VSCROLL + 2 * scroll_map) as usize & 0x1ff;
    let mx = (x + 512 - h) & 511;
    let my = (y + v) & 511;
    let entry = rd16(tiles, map * 0x2000 + ((my / 8) * 64 + mx / 8) * 2) as usize;
    let p = pen(tiles, entry, my & 7, mx & 7);
    if p == 0 && !opaque {
        return None;
    }
    Some(((entry & 0x7f80) >> 3) | p as usize)
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
    let mut out = vec![0u8; WIDTH * HEIGHT * 3];
    let backdrop = rgb(rd16(palette, 0));
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let mut c = backdrop;
            // Plane B is opaque (its pen 0 is the backdrop); plane A on top.
            for (plane, opaque) in [(1, true), (0, false)] {
                let (map, scroll) = map_for(tiles, plane, x, y);
                if let Some(i) = sample(tiles, map, scroll, x, y, opaque) {
                    c = rgb(rd16(palette, (i & 0xfff) * 2));
                }
            }
            out[(y * WIDTH + x) * 3..][..3].copy_from_slice(&c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn colour_bank_shares_tile_bits() {
        let mut tiles = vec![0u8; 0x10_0000];
        // Map 0 entry (0,0) = tile 0x0080 (bank bits 7..14 = 1), pen 3 everywhere.
        tiles[0..2].copy_from_slice(&0x0080u16.to_le_bytes());
        let off = CHAR_BASE + 0x80 * 32;
        tiles[off..off + 32].fill(0x33);
        assert_eq!(sample(&tiles, 0, 0, 0, 0, false), Some(0x10 | 3));
    }
}
