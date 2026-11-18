//! Text to tiles: find the 8x8 tile patterns a region of a frame is drawn
//! with (from texmap's ID and position images), and cut a typeset image
//! laid over that region into the tiles' replacements, one `tile_<key>.png`
//! each (docs/uplift-studio.md).

#![allow(clippy::needless_range_loop, clippy::chunks_exact_to_as_chunks, clippy::manual_is_multiple_of)]

use crate::studio_img::{blank, nearest, Image};
use std::collections::HashMap;

/// One drawn tile: where its 8x8 cell is in the frame, in output pixels.
#[derive(Clone, Debug)]
pub struct Cell {
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub size: f32,
    /// Pixels of the cell visible in the region.
    pub pixels: usize,
}

pub fn decode_ids(ids: &Image) -> Vec<u32> {
    ids.rgba.chunks_exact(4).map(|p| p[0] as u32 | (p[1] as u32) << 8 | (p[2] as u32) << 16).collect()
}

/// The IDs in a region of the ID image, with their pixel counts.
pub fn ids_in_region(ids: &[u32], width: usize, region: (usize, usize, usize, usize)) -> HashMap<u32, usize> {
    let mut m = HashMap::new();
    let height = ids.len() / width;
    for y in region.1..(region.1 + region.3).min(height) {
        for x in region.0..(region.0 + region.2).min(width) {
            *m.entry(ids[y * width + x]).or_insert(0) += 1;
        }
    }
    m
}

/// The tile cells drawn in `region` (x, y, w, h in output pixels): every
/// pixel whose ID is a tile gives its cell's corner from its position
/// inside the tile (the position image: 0-255 across the 8 texels) and the
/// frame's scale (`scale` output pixels a game pixel); pixels agreeing on a
/// corner within a pixel are one cell. Cells with under a quarter of their
/// area in the region are left out.
pub fn tile_cells(
    ids: &[u32],
    uv: &Image,
    names: &HashMap<u32, String>,
    region: (usize, usize, usize, usize),
    scale: f32,
) -> Vec<Cell> {
    let width = uv.width;
    let height = ids.len() / width;
    // id -> clusters (sum x, sum y, count)
    let mut clusters: HashMap<u32, Vec<(f64, f64, usize)>> = HashMap::new();
    for y in region.1..(region.1 + region.3).min(height) {
        for x in region.0..(region.0 + region.2).min(width) {
            let id = ids[y * width + x];
            if !names.get(&id).is_some_and(|n| n.starts_with("tile_")) {
                continue;
            }
            let o = (y * width + x) * 4;
            if uv.rgba[o + 2] == 0 {
                continue;
            }
            // texel position of the pixel's centre, 0..8
            let u = (uv.rgba[o] as f32 + 0.5) / 256.0 * 8.0;
            let v = (uv.rgba[o + 1] as f32 + 0.5) / 256.0 * 8.0;
            let (cx, cy) = (x as f32 + 0.5 - u * scale, y as f32 + 0.5 - v * scale);
            let list = clusters.entry(id).or_default();
            match list
                .iter_mut()
                .find(|c| (c.0 / c.2 as f64 - cx as f64).abs() <= 1.0 && (c.1 / c.2 as f64 - cy as f64).abs() <= 1.0)
            {
                Some(c) => {
                    c.0 += cx as f64;
                    c.1 += cy as f64;
                    c.2 += 1;
                }
                None => list.push((cx as f64, cy as f64, 1)),
            }
        }
    }
    let size = 8.0 * scale;
    let mut cells: Vec<Cell> = clusters
        .into_iter()
        .flat_map(|(id, list)| {
            let name = names[&id].clone();
            list.into_iter().filter(move |c| c.2 as f32 >= size * size / 4.0).map(move |c| Cell {
                name: name.clone(),
                x: (c.0 / c.2 as f64) as f32,
                y: (c.1 / c.2 as f64) as f32,
                size,
                pixels: c.2,
            })
        })
        .collect();
    cells.sort_by(|a, b| {
        (a.y.round() as i64, a.x.round() as i64, &a.name).cmp(&(b.y.round() as i64, b.x.round() as i64, &b.name))
    });
    cells
}

/// Where a typeset image sits in the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fit {
    /// Filling the box, the image's proportions given up.
    Stretch,
    /// As large as fits the box, proportions kept, aligned.
    Contain,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Left,
    Center,
    Right,
}

pub fn place(img: (usize, usize), bx: (f32, f32, f32, f32), fit: Fit, align: Align) -> Placement {
    let (iw, ih) = (img.0 as f32, img.1 as f32);
    match fit {
        Fit::Stretch => Placement { x: bx.0, y: bx.1, w: bx.2, h: bx.3 },
        Fit::Contain => {
            let k = (bx.2 / iw).min(bx.3 / ih);
            let (w, h) = (iw * k, ih * k);
            let x = match align {
                Align::Left => bx.0,
                Align::Center => bx.0 + (bx.2 - w) / 2.0,
                Align::Right => bx.0 + bx.2 - w,
            };
            Placement { x, y: bx.1 + (bx.3 - h) / 2.0, w, h }
        }
    }
}

/// The box of the pixels with alpha, grown by `margin`; the image if none.
pub fn trim(img: &Image, margin: usize) -> Image {
    let (mut x0, mut y0, mut x1, mut y1) = (img.width, img.height, 0, 0);
    for y in 0..img.height {
        for x in 0..img.width {
            if img.rgba[(y * img.width + x) * 4 + 3] > 8 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 <= x0 {
        return img.clone();
    }
    let (x0, y0) = (x0.saturating_sub(margin), y0.saturating_sub(margin));
    let (x1, y1) = ((x1 + margin).min(img.width), (y1 + margin).min(img.height));
    crate::studio_img::crop(img, x0, y0, x1 - x0, y1 - y0)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AlphaMode {
    /// The typeset image's alpha; nothing of the original shows.
    Typeset,
    /// The original tile's alpha and, where the typeset image is clear,
    /// its colour: the text is set into the tile's own shape.
    Original,
}

/// Premultiplied-average sample of the typeset image over the box
/// (x0, y0)-(x1, y1) in its pixels (outside it is clear).
fn sample(img: &Image, x0: f32, y0: f32, x1: f32, y1: f32) -> [f32; 4] {
    let n = (((x1 - x0).max(y1 - y0)).ceil() as usize).clamp(1, 6);
    let (mut acc, mut cnt) = ([0f32; 4], 0f32);
    for j in 0..n {
        for i in 0..n {
            let px = x0 + (x1 - x0) * (i as f32 + 0.5) / n as f32;
            let py = y0 + (y1 - y0) * (j as f32 + 0.5) / n as f32;
            cnt += 1.0;
            if px < 0.0 || py < 0.0 || px >= img.width as f32 || py >= img.height as f32 {
                continue;
            }
            let o = (py as usize * img.width + px as usize) * 4;
            let a = img.rgba[o + 3] as f32 / 255.0;
            for c in 0..3 {
                acc[c] += img.rgba[o + c] as f32 * a;
            }
            acc[3] += a;
        }
    }
    let a = acc[3];
    if a <= 0.0 {
        return [0.0; 4];
    }
    [acc[0] / a, acc[1] / a, acc[2] / a, a / cnt]
}

/// Cut `typeset`, placed at `at`, into the replacement of one cell:
/// `tile_px` pixels a side (8 x scale). `orig` is the original 8x8 tile.
pub fn slice_cell(
    typeset: &Image,
    at: &Placement,
    cell: &Cell,
    orig: Option<&Image>,
    tile_px: usize,
    mode: AlphaMode,
) -> Image {
    let mut out = blank(tile_px, tile_px, [0; 4]);
    let big = orig.map(|o| nearest(o, tile_px.div_ceil(o.width).max(1)));
    let (sx, sy) = (typeset.width as f32 / at.w, typeset.height as f32 / at.h);
    for y in 0..tile_px {
        for x in 0..tile_px {
            // The pixel's box in the frame, then in the typeset image.
            let fx0 = cell.x + x as f32 / tile_px as f32 * cell.size;
            let fx1 = cell.x + (x + 1) as f32 / tile_px as f32 * cell.size;
            let fy0 = cell.y + y as f32 / tile_px as f32 * cell.size;
            let fy1 = cell.y + (y + 1) as f32 / tile_px as f32 * cell.size;
            let t = sample(typeset, (fx0 - at.x) * sx, (fy0 - at.y) * sy, (fx1 - at.x) * sx, (fy1 - at.y) * sy);
            let o = (y * tile_px + x) * 4;
            // The original's pixel (nearest of its enlargement).
            let og = big.as_ref().map(|b| {
                let bo = ((y * b.height / tile_px) * b.width + x * b.width / tile_px) * 4;
                [b.rgba[bo], b.rgba[bo + 1], b.rgba[bo + 2], b.rgba[bo + 3]]
            });
            let px = match (mode, og) {
                (AlphaMode::Original, Some(g)) => {
                    // text over the original's own colour, the original's shape
                    let a = t[3];
                    let c = |i: usize| (t[i] * a + g[i] as f32 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
                    [c(0), c(1), c(2), g[3]]
                }
                _ => [t[0].round() as u8, t[1].round() as u8, t[2].round() as u8, (t[3] * 255.0).round() as u8],
            };
            out.rgba[o..o + 4].copy_from_slice(&px);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame of 64x32 output pixels at scale 2 (so 16 px tile cells):
    /// tile 5 at x 8..24 y 8..24 and tile 6 at x 24..40, with a position image.
    fn frame() -> (Vec<u32>, Image, HashMap<u32, String>) {
        let (w, h) = (64usize, 32usize);
        let mut ids = vec![0u32; w * h];
        let mut uv = blank(w, h, [0, 0, 0, 255]);
        for (id, x0) in [(5u32, 8usize), (6, 24)] {
            for y in 8..24 {
                for x in x0..x0 + 16 {
                    ids[y * w + x] = id;
                    let o = (y * w + x) * 4;
                    uv.rgba[o] = ((x - x0) * 256 / 16) as u8;
                    uv.rgba[o + 1] = ((y - 8) * 256 / 16) as u8;
                    uv.rgba[o + 2] = 255;
                }
            }
        }
        let names =
            HashMap::from([(5, "tile_aa".to_string()), (6, "tile_bb".to_string()), (7, "tex_c_8x8".to_string())]);
        (ids, uv, names)
    }

    #[test]
    fn finds_cells_and_their_corners() {
        let (ids, uv, names) = frame();
        let cells = tile_cells(&ids, &uv, &names, (0, 0, 64, 32), 2.0);
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].name, "tile_aa");
        assert!((cells[0].x - 8.0).abs() < 1.0 && (cells[0].y - 8.0).abs() < 1.0, "{:?}", cells[0]);
        assert!((cells[1].x - 24.0).abs() < 1.0);
        assert_eq!(cells[0].size, 16.0);
        // a region holding a sliver of a tile finds none of it
        assert!(tile_cells(&ids, &uv, &names, (0, 0, 10, 32), 2.0).is_empty());
        assert_eq!(tile_cells(&ids, &uv, &names, (22, 0, 30, 32), 2.0).len(), 1);
        let counts = ids_in_region(&ids, 64, (0, 0, 20, 32));
        assert_eq!(counts[&5], 12 * 16);
    }

    #[test]
    fn slices_a_typeset_image_across_two_tiles() {
        let (ids, uv, names) = frame();
        let cells = tile_cells(&ids, &uv, &names, (0, 0, 64, 32), 2.0);
        // 32x16 typeset image: left half red, right half blue, over the two cells (x 8..40, y 8..24)
        let mut t = blank(32, 16, [0; 4]);
        for y in 0..16 {
            for x in 0..32 {
                let c = if x < 16 { [255, 0, 0, 255] } else { [0, 0, 255, 255] };
                t.rgba[(y * 32 + x) * 4..][..4].copy_from_slice(&c);
            }
        }
        let at = place((32, 16), (8.0, 8.0, 32.0, 16.0), Fit::Stretch, Align::Left);
        let a = slice_cell(&t, &at, &cells[0], None, 16, AlphaMode::Typeset);
        let b = slice_cell(&t, &at, &cells[1], None, 16, AlphaMode::Typeset);
        assert_eq!(a.rgba[..4], [255, 0, 0, 255]);
        assert_eq!(a.rgba[(15 * 16 + 15) * 4..][..4], [255, 0, 0, 255]);
        assert_eq!(b.rgba[..4], [0, 0, 255, 255]);
        // Original alpha: shape from the original (here a left half only),
        // colour from the text.
        let mut orig = blank(8, 8, [10, 10, 10, 0]);
        for y in 0..8 {
            for x in 0..4 {
                orig.rgba[(y * 8 + x) * 4..][..4].copy_from_slice(&[10, 10, 10, 255]);
            }
        }
        let c = slice_cell(&t, &at, &cells[0], Some(&orig), 16, AlphaMode::Original);
        assert_eq!(c.rgba[..4], [255, 0, 0, 255]);
        assert_eq!(c.rgba[12 * 4..][..4][3], 0);
        // outside the typeset image: clear in typeset mode, the original in original mode
        let far = place((32, 16), (200.0, 200.0, 32.0, 16.0), Fit::Stretch, Align::Left);
        let d = slice_cell(&t, &far, &cells[0], Some(&orig), 16, AlphaMode::Original);
        assert_eq!(d.rgba[..4], [10, 10, 10, 255]);
        assert_eq!(slice_cell(&t, &far, &cells[0], None, 16, AlphaMode::Typeset).rgba[3], 0);
    }

    #[test]
    fn placement_and_trim() {
        let p = place((100, 50), (0.0, 0.0, 40.0, 40.0), Fit::Contain, Align::Left);
        assert_eq!((p.w, p.h, p.x, p.y), (40.0, 20.0, 0.0, 10.0));
        let p = place((100, 50), (0.0, 0.0, 40.0, 40.0), Fit::Contain, Align::Right);
        assert_eq!(p.x, 0.0);
        let p = place((10, 50), (0.0, 0.0, 40.0, 50.0), Fit::Contain, Align::Right);
        assert_eq!(p.x, 30.0);
        let mut im = blank(10, 10, [0; 4]);
        im.rgba[(5 * 10 + 4) * 4 + 3] = 255;
        let t = trim(&im, 1);
        assert_eq!((t.width, t.height), (3, 3));
    }
}
