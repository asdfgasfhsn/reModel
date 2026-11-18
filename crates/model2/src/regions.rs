//! Region overrides for the 2D layer (docs/texture-packs.md, "Regions"): a
//! texture pack can say "where this arrangement of tiles is in a tile map,
//! draw this image over that rectangle". Tiles are 8x8 and one pattern is
//! one pack key wherever it is used, so art that spans several tiles (a
//! logo built from shared fill tiles, a word whose letters are also other
//! words) can't be reshaped by tile replacement alone.
//!
//! A region is keyed by the tile pattern keys ([`crate::enhance::TileInfo::key`])
//! at the cells of a rectangle of a tile map, with wildcards, and is found
//! anywhere in the maps (in map space, so scrolling never matters and a
//! region that scrolls half off the screen is still matched). Everything
//! here is render-only; nothing reads back into the game.
//!
//! This module holds the format (`regions.txt` and `region_<name>.png`), the
//! parsed [`RegionSet`] and its per-cell slices of the images; matching
//! against a frame's tile RAM is `video::match_regions`, and drawing is in
//! `video` (software) and the GPU renderer's `tiles.rs`.

use crate::enhance::{Fnv, Rgba};
use std::path::Path;
use std::sync::Arc;

/// The regions file in a pack.
pub const FILE: &str = "regions.txt";
/// File-name prefix of region images (`region_<name>.png`).
pub const IMAGE_PREFIX: &str = "region_";
/// The feature name in a manifest's `uses` / `needs`.
pub const FEATURE: &str = "regions";

/// Key stored for a blank tile (all pen 0), which has no tile key.
pub const BLANK: u64 = 1;

/// What a cell of a region's layout must be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellKey {
    /// `*`: anything (animated or irrelevant parts).
    Any,
    /// `_`: a blank tile (all pen 0), whatever its colours.
    Blank,
    /// 16 hex digits: this tile pattern (`TileInfo::key`).
    Tile(u64),
}

/// Plane constraint: A is maps 0 and 1, B maps 2 and 3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plane {
    Any,
    A,
    B,
}

/// Priority constraint: every non-wildcard cell's map entry has (high) or
/// hasn't (low) the priority bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Any,
    High,
    Low,
}

/// One `region` line of `regions.txt`.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionDef {
    pub name: String,
    /// Size in tiles.
    pub w: usize,
    pub h: usize,
    /// Row-major, `w * h`.
    pub cells: Vec<CellKey>,
    pub plane: Plane,
    pub layer: Layer,
    /// `at=MAP:X,Y`: only at this tile cell of this map (0-3, cells 0-63);
    /// the default is anywhere.
    pub at: Option<(usize, usize, usize)>,
    /// `k=v` fields this build doesn't know, as written (ignored).
    pub unknown: Vec<String>,
}

/// A region name: lowercase letters, digits, `-` and `_`; it is part of the
/// image's file name.
pub fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 48
        && n.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn parse_key(s: &str) -> Result<CellKey, String> {
    match s {
        "*" => Ok(CellKey::Any),
        "_" => Ok(CellKey::Blank),
        k if k.len() == 16 && k.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) => {
            Ok(CellKey::Tile(u64::from_str_radix(k, 16).map_err(|e| e.to_string())?))
        }
        k => Err(format!("`{k}` isn't a cell (`*`, `_` or 16 lowercase hex digits)")),
    }
}

impl RegionDef {
    /// The `region` line of this definition (`parse` reads it back).
    pub fn to_line(&self) -> String {
        let rows: Vec<String> = self
            .cells
            .chunks(self.w)
            .map(|r| {
                r.iter()
                    .map(|c| match c {
                        CellKey::Any => "*".to_string(),
                        CellKey::Blank => "_".to_string(),
                        CellKey::Tile(k) => format!("{k:016x}"),
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .collect();
        let mut s = format!("region\t{}\t{}x{}\tcells={}", self.name, self.w, self.h, rows.join("/"));
        match self.plane {
            Plane::Any => {}
            Plane::A => s += "\tplane=a",
            Plane::B => s += "\tplane=b",
        }
        match self.layer {
            Layer::Any => {}
            Layer::High => s += "\tlayer=high",
            Layer::Low => s += "\tlayer=low",
        }
        if let Some((m, x, y)) = self.at {
            s += &format!("\tat={m}:{x},{y}");
        }
        for u in &self.unknown {
            s += "\t";
            s += u;
        }
        s
    }
}

/// Parse a regions file: one `region` line per region (a keyword, the name,
/// `WxH`, then `k=v` fields separated by tabs or spaces); `#` comments and
/// blank lines are skipped; unknown keywords are ignored; a duplicate name
/// or a malformed line is an error saying which line.
pub fn parse(text: &str) -> Result<Vec<RegionDef>, String> {
    let mut out: Vec<RegionDef> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let at = |e: String| format!("{FILE} line {}: {e}", i + 1);
        let line = raw.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut f = line.split_whitespace();
        if f.next() != Some("region") {
            continue;
        }
        let name = f.next().ok_or_else(|| at("a region needs a name".into()))?;
        if !valid_name(name) {
            return Err(at(format!("`{name}` isn't a region name (lowercase letters, digits, - and _)")));
        }
        if out.iter().any(|r| r.name == name) {
            return Err(at(format!("region `{name}` is given twice")));
        }
        let size = f.next().ok_or_else(|| at("a region needs a size WxH (in tiles)".into()))?;
        let (w, h) = size
            .split_once('x')
            .and_then(|(w, h)| Some((w.parse::<usize>().ok()?, h.parse::<usize>().ok()?)))
            .filter(|&(w, h)| (1..=64).contains(&w) && (1..=64).contains(&h))
            .ok_or_else(|| at(format!("`{size}` isn't a size WxH of 1 to 64 tiles")))?;
        let mut d = RegionDef {
            name: name.into(),
            w,
            h,
            cells: Vec::new(),
            plane: Plane::Any,
            layer: Layer::Any,
            at: None,
            unknown: Vec::new(),
        };
        let mut have_cells = false;
        for kv in f {
            let Some((k, v)) = kv.split_once('=') else {
                d.unknown.push(kv.into());
                continue;
            };
            match k {
                "cells" => {
                    let rows: Vec<&str> = v.split('/').collect();
                    if rows.len() != h {
                        return Err(at(format!("cells has {} rows, the region is {h} tall", rows.len())));
                    }
                    for r in rows {
                        let row = r.split(',').map(parse_key).collect::<Result<Vec<_>, _>>().map_err(at)?;
                        if row.len() != w {
                            return Err(at(format!("a row of cells has {} cells, the region is {w} wide", row.len())));
                        }
                        d.cells.extend(row);
                    }
                    have_cells = true;
                }
                "plane" => {
                    d.plane = match v {
                        "a" => Plane::A,
                        "b" => Plane::B,
                        "any" => Plane::Any,
                        _ => return Err(at(format!("plane is a, b or any, not `{v}`"))),
                    }
                }
                "layer" => {
                    d.layer = match v {
                        "high" => Layer::High,
                        "low" => Layer::Low,
                        "any" => Layer::Any,
                        _ => return Err(at(format!("layer is high, low or any, not `{v}`"))),
                    }
                }
                "at" => {
                    let p = v.split_once(':').and_then(|(m, xy)| {
                        let (x, y) = xy.split_once(',')?;
                        Some((m.parse::<usize>().ok()?, x.parse::<usize>().ok()?, y.parse::<usize>().ok()?))
                    });
                    match p {
                        Some((m, x, y)) if m < 4 && x < 64 && y < 64 => d.at = Some((m, x, y)),
                        _ => return Err(at(format!("at is MAP:X,Y (map 0-3, cells 0-63), not `{v}`"))),
                    }
                }
                _ => d.unknown.push(kv.into()),
            }
        }
        if !have_cells {
            return Err(at(format!("region `{name}` has no cells=")));
        }
        if d.cells.iter().all(|c| matches!(c, CellKey::Any)) {
            return Err(at(format!("region `{name}` is all wildcards: it would match everywhere")));
        }
        out.push(d);
    }
    Ok(out)
}

/// The text of a regions file for `defs`.
pub fn to_text(defs: &[RegionDef]) -> String {
    let mut s = String::from(
        "# reModel regions: region NAME WxH cells=KEY,KEY/KEY,KEY [plane=a|b] [layer=high|low] [at=MAP:X,Y]\n\
         # (rows of tile keys, `*` any, `_` blank); image region_NAME.png (docs/texture-packs.md, \"Regions\")\n",
    );
    for d in defs {
        s += &d.to_line();
        s += "\n";
    }
    s
}

/// A region with its image.
#[derive(Clone)]
pub struct Region {
    pub def: RegionDef,
    /// The replacement over the whole rectangle (`w * 8` x `h * 8` game
    /// pixels at 1x; any larger size).
    pub image: Arc<Rgba>,
    /// Index of this region's first slice in [`RegionSet::slices`].
    pub first_slice: u32,
}

/// One tile cell of a region's image: what a renderer draws for a pattern
/// the way it draws a tile image (any multiple of 8x8 in size; sampled
/// bilinear and clamped to the cell).
#[derive(Clone)]
pub struct Slice {
    /// A key for caches (atlases), from the region's name, the cell and the
    /// image's size; it can't be confused with a tile key in practice.
    pub key: u64,
    pub image: Arc<Rgba>,
}

/// The regions of a pack that have images.
#[derive(Clone, Default)]
pub struct RegionSet {
    pub regions: Vec<Region>,
    /// Every region's cells' slices, row-major per region.
    pub slices: Vec<Slice>,
    /// Region names without a usable image (no file, unreadable, or smaller
    /// than 8 pixels per tile): they draw as the original.
    pub skipped: Vec<(String, String)>,
}

/// `img` cut to cell (`ix`, `iy`) of a `w` x `h` grid, edges at the integer
/// fractions of its size (a multiple of `w` x `h` cuts evenly).
pub fn cell_image(img: &Rgba, w: usize, h: usize, ix: usize, iy: usize) -> Rgba {
    let (x0, x1) = (ix * img.width / w, (ix + 1) * img.width / w);
    let (y0, y1) = (iy * img.height / h, (iy + 1) * img.height / h);
    let mut data = Vec::with_capacity((x1 - x0) * (y1 - y0) * 4);
    for y in y0..y1 {
        data.extend_from_slice(&img.data[(y * img.width + x0) * 4..(y * img.width + x1) * 4]);
    }
    Rgba { width: x1 - x0, height: y1 - y0, data }
}

impl RegionSet {
    /// Build the set from parsed definitions and an image source (by
    /// region name).
    pub fn new(defs: Vec<RegionDef>, mut image: impl FnMut(&str) -> Option<Arc<Rgba>>) -> RegionSet {
        let mut set = RegionSet::default();
        for d in defs {
            let Some(img) = image(&d.name) else {
                let why = format!("no readable {IMAGE_PREFIX}{}.png", d.name);
                set.skipped.push((d.name, why));
                continue;
            };
            if img.width < 8 * d.w || img.height < 8 * d.h {
                let why = format!(
                    "its image is {}x{}, under 8 pixels per tile ({}x{})",
                    img.width,
                    img.height,
                    8 * d.w,
                    8 * d.h
                );
                set.skipped.push((d.name, why));
                continue;
            }
            let first_slice = set.slices.len() as u32;
            for iy in 0..d.h {
                for ix in 0..d.w {
                    let mut f = Fnv::default();
                    f.bytes(b"region");
                    f.bytes(d.name.as_bytes());
                    f.bytes(&[0, ix as u8, iy as u8]);
                    f.bytes(&(img.width as u32).to_le_bytes());
                    f.bytes(&(img.height as u32).to_le_bytes());
                    f.bytes(&img.data[..img.data.len().min(64)]);
                    set.slices.push(Slice { key: f.0, image: Arc::new(cell_image(&img, d.w, d.h, ix, iy)) });
                }
            }
            set.regions.push(Region { def: d, image: img, first_slice });
        }
        set
    }

    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    /// The side in texels of the largest slice (for sizing an atlas).
    pub fn max_cell(&self) -> usize {
        self.slices.iter().map(|s| s.image.width.max(s.image.height)).max().unwrap_or(8)
    }

    /// Load a pack directory's regions: `regions.txt` and the images, read
    /// by `decode`. A missing file is no regions; an unreadable or
    /// malformed one is an error (the caller prints it and draws without).
    pub fn load_dir(dir: &Path, decode: &mut dyn FnMut(&Path) -> Option<Arc<Rgba>>) -> Result<RegionSet, String> {
        let text = match std::fs::read_to_string(dir.join(FILE)) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(RegionSet::default()),
            Err(e) => return Err(format!("{}: {e}", dir.join(FILE).display())),
        };
        let defs = parse(&text)?;
        Ok(RegionSet::new(defs, |n| decode(&dir.join(format!("{IMAGE_PREFIX}{n}.png")))))
    }
}

/// Where regions matched in a frame: per tile map, per cell, the slice to
/// draw (`u32::MAX`: none).
pub struct RegionGrid {
    pub cells: [Vec<u32>; 4],
    /// Matches per region (by index in the set).
    pub matches: Vec<u32>,
    pub any: bool,
}

pub const NO_SLICE: u32 = u32::MAX;

impl RegionGrid {
    pub fn empty(regions: usize) -> RegionGrid {
        RegionGrid { cells: std::array::from_fn(|_| vec![NO_SLICE; 64 * 64]), matches: vec![0; regions], any: false }
    }

    /// The slice for cell (`cx`, `cy`) of `map`, if a region covers it.
    #[inline]
    pub fn at(&self, map: usize, cx: usize, cy: usize) -> Option<u32> {
        let s = self.cells[map & 3][(cy & 63) * 64 + (cx & 63)];
        (s != NO_SLICE).then_some(s)
    }
}

/// Match `set`'s regions against per-map cell layouts: `keys[map][cy * 64 +
/// cx]` is the cell's tile key ([`BLANK`] for a blank tile) and `prio` its
/// priority bit; `enabled[map]` says whether the map is on. Regions are
/// searched anywhere in a map, wrapping at its edges; where regions
/// overlap, the first in the file wins the cell.
pub fn match_layout(set: &RegionSet, keys: &[Vec<u64>; 4], prio: &[Vec<bool>; 4], enabled: [bool; 4]) -> RegionGrid {
    let mut grid = RegionGrid::empty(set.regions.len());
    for (ri, r) in set.regions.iter().enumerate() {
        let d = &r.def;
        for map in 0..4 {
            if !enabled[map] {
                continue;
            }
            match d.plane {
                Plane::A if map >= 2 => continue,
                Plane::B if map < 2 => continue,
                _ => {}
            }
            let anchors: Vec<(usize, usize)> = match d.at {
                Some((m, x, y)) => {
                    if m == map {
                        vec![(x, y)]
                    } else {
                        continue;
                    }
                }
                None => (0..64).flat_map(|y| (0..64).map(move |x| (x, y))).collect(),
            };
            'anchor: for (ax, ay) in anchors {
                for iy in 0..d.h {
                    for ix in 0..d.w {
                        let i = ((ay + iy) & 63) * 64 + ((ax + ix) & 63);
                        let ok = match d.cells[iy * d.w + ix] {
                            CellKey::Any => true,
                            CellKey::Blank => keys[map][i] == BLANK,
                            CellKey::Tile(k) => keys[map][i] == k,
                        } && (d.cells[iy * d.w + ix] == CellKey::Any
                            || match d.layer {
                                Layer::Any => true,
                                Layer::High => prio[map][i],
                                Layer::Low => !prio[map][i],
                            });
                        if !ok {
                            continue 'anchor;
                        }
                    }
                }
                grid.matches[ri] += 1;
                for iy in 0..d.h {
                    for ix in 0..d.w {
                        let i = ((ay + iy) & 63) * 64 + ((ax + ix) & 63);
                        if grid.cells[map][i] == NO_SLICE {
                            grid.cells[map][i] = r.first_slice + (iy * d.w + ix) as u32;
                            grid.any = true;
                        }
                    }
                }
            }
        }
    }
    grid
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(name: &str, w: usize, h: usize, cells: Vec<CellKey>) -> RegionDef {
        RegionDef { name: name.into(), w, h, cells, plane: Plane::Any, layer: Layer::Any, at: None, unknown: vec![] }
    }

    fn solid(w: usize, h: usize) -> Arc<Rgba> {
        Arc::new(Rgba { width: w, height: h, data: [9, 8, 7, 255].repeat(w * h) })
    }

    #[test]
    fn parses_and_round_trips() {
        let text = "# c\nregion\tlogo-a\t2x2\tcells=0123456789abcdef,*/_,0000000000000042\tplane=b\tlayer=high\tat=2:3,4\tnote=x\n\
                    sparkle\tignored\nregion\tb\t1x1\tcells=0000000000000001\n";
        let defs = parse(text).unwrap();
        assert_eq!(defs.len(), 2);
        let a = &defs[0];
        assert_eq!((a.w, a.h, a.plane, a.layer, a.at), (2, 2, Plane::B, Layer::High, Some((2, 3, 4))));
        assert_eq!(a.cells[0], CellKey::Tile(0x0123456789abcdef));
        assert_eq!(a.cells[1], CellKey::Any);
        assert_eq!(a.cells[2], CellKey::Blank);
        assert_eq!(a.unknown, ["note=x"]);
        assert_eq!(parse(&to_text(&defs)).unwrap(), defs);
    }

    #[test]
    fn refuses_malformed_lines_with_the_line() {
        for (text, what) in [
            ("region\tA\t1x1\tcells=0000000000000001\n", "region name"),
            ("region\ta\t1x1\tcells=0000000000000001\nregion\ta\t1x1\tcells=0000000000000002\n", "twice"),
            ("\nregion\ta\t2x1\tcells=0000000000000001\n", "1 cells"),
            ("region\ta\t1x1\tcells=*\n", "wildcards"),
            ("region\ta\t1x1\tcells=zz\n", "isn't a cell"),
            ("region\ta\t1x1\n", "no cells"),
            ("region\ta\t1x1\tcells=0000000000000001\tplane=c\n", "plane"),
            ("region\ta\t0x1\tcells=0000000000000001\n", "size"),
        ] {
            let e = parse(text).unwrap_err();
            assert!(e.contains("line"), "{e}");
            assert!(e.contains(what) || what == "1 cells", "{what}: {e}");
        }
    }

    fn layout(set: &[(usize, usize, usize, u64)]) -> ([Vec<u64>; 4], [Vec<bool>; 4]) {
        let mut keys: [Vec<u64>; 4] = std::array::from_fn(|_| vec![BLANK; 4096]);
        let prio: [Vec<bool>; 4] = std::array::from_fn(|_| vec![false; 4096]);
        for &(m, x, y, k) in set {
            keys[m][y * 64 + x] = k;
        }
        (keys, prio)
    }

    fn two_by_one() -> RegionSet {
        RegionSet::new(vec![def("r", 2, 1, vec![CellKey::Tile(0xaa), CellKey::Tile(0xbb)])], |_| Some(solid(16, 8)))
    }

    #[test]
    fn matches_exact_anywhere_and_wraps() {
        let set = two_by_one();
        let (keys, prio) = layout(&[(1, 10, 20, 0xaa), (1, 11, 20, 0xbb), (1, 63, 5, 0xaa), (1, 0, 5, 0xbb)]);
        let g = match_layout(&set, &keys, &prio, [true; 4]);
        assert_eq!(g.matches, [2]);
        assert_eq!(g.at(1, 10, 20), Some(0));
        assert_eq!(g.at(1, 11, 20), Some(1));
        // Across the map's edge.
        assert_eq!(g.at(1, 63, 5), Some(0));
        assert_eq!(g.at(1, 0, 5), Some(1));
        assert_eq!(g.at(1, 12, 20), None);
        assert_eq!(g.at(0, 10, 20), None);
    }

    #[test]
    fn partial_arrangements_do_not_match() {
        let set = two_by_one();
        let (keys, prio) = layout(&[(0, 5, 5, 0xaa), (0, 6, 5, 0xcc), (0, 9, 9, 0xbb)]);
        let g = match_layout(&set, &keys, &prio, [true; 4]);
        assert!(!g.any && g.matches == [0]);
    }

    #[test]
    fn wildcards_and_blank_and_constraints() {
        let d = RegionDef {
            plane: Plane::B,
            layer: Layer::High,
            ..def("w", 3, 1, vec![CellKey::Tile(0xaa), CellKey::Any, CellKey::Blank])
        };
        let set = RegionSet::new(vec![d], |_| Some(solid(24, 8)));
        let (keys, mut prio) = layout(&[(2, 1, 1, 0xaa), (2, 2, 1, 0x1234)]);
        // Priority missing: no match; set on the non-wild cells: match.
        assert!(!match_layout(&set, &keys, &prio, [true; 4]).any);
        prio[2][64 + 1] = true;
        prio[2][64 + 3] = true;
        let g = match_layout(&set, &keys, &prio, [true; 4]);
        assert_eq!((g.matches[0], g.at(2, 3, 1)), (1, Some(2)));
        // Plane A has the same layout: not matched there.
        let (keys, prio) = layout(&[(0, 1, 1, 0xaa), (0, 2, 1, 0x1234)]);
        assert!(!match_layout(&set, &keys, &prio, [true; 4]).any);
        // Disabled maps are not searched.
        let (keys, mut prio2) = layout(&[(2, 1, 1, 0xaa)]);
        prio2[2][64 + 1] = true;
        prio2[2][64 + 3] = true;
        assert!(!match_layout(&set, &keys, &prio2, [true, true, false, true]).any);
        let _ = prio;
    }

    #[test]
    fn at_pins_a_position_and_first_region_wins_overlap() {
        let mut a = def("a", 1, 1, vec![CellKey::Tile(0xaa)]);
        a.at = Some((0, 3, 3));
        let b = def("b", 1, 1, vec![CellKey::Tile(0xaa)]);
        let set = RegionSet::new(vec![a, b], |_| Some(solid(8, 8)));
        let (keys, prio) = layout(&[(0, 3, 3, 0xaa), (0, 4, 3, 0xaa)]);
        let g = match_layout(&set, &keys, &prio, [true; 4]);
        assert_eq!((g.at(0, 3, 3), g.at(0, 4, 3)), (Some(0), Some(1)));
        assert_eq!(g.matches, [1, 2]);
    }

    #[test]
    fn slices_cut_the_image_by_cell_and_regions_need_enough_pixels() {
        let data = (0..16 * 8).flat_map(|i| if i % 16 < 8 { [255, 0, 0, 255] } else { [0, 0, 255, 255] }).collect();
        let img = Arc::new(Rgba { width: 16, height: 8, data });
        let set = RegionSet::new(vec![def("r", 2, 1, vec![CellKey::Tile(1), CellKey::Tile(2)])], |_| Some(img.clone()));
        assert_eq!(set.slices.len(), 2);
        assert_eq!(set.slices[0].image.data[..4], [255, 0, 0, 255]);
        assert_eq!(set.slices[1].image.data[..4], [0, 0, 255, 255]);
        assert_ne!(set.slices[0].key, set.slices[1].key);
        let small =
            RegionSet::new(vec![def("r", 2, 1, vec![CellKey::Tile(1), CellKey::Tile(2)])], |_| Some(solid(8, 8)));
        assert!(small.is_empty() && small.skipped.len() == 1);
        let none = RegionSet::new(vec![def("r", 1, 1, vec![CellKey::Tile(1)])], |_| None);
        assert!(none.is_empty() && none.skipped.len() == 1);
    }
}
