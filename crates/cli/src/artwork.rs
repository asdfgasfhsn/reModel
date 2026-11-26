//! Artwork identity for packs (docs/uplift-studio.md, "Artworks"): which
//! texture keys are one artwork, what each colouring's ramp is, and the
//! build step that makes the colour copies of one authored image.
//!
//! An **artwork** is a set of texels (the surface key) at a size; a
//! **variant** is one way the game colours it (the image key a pack's
//! `tex_` file is named by). The catalogue records each variant's colour
//! ramp (`ramp=` in its detail: what texel values 0 to 15 are drawn in at
//! full luma). A pack names the artwork with one `art_` image; `bake` here
//! makes the variants' images from it.
//!
//! Nothing here needs a ROM, a GPU or the emulator: it works on the
//! catalogue's or a texture map's rows and on images.

#![allow(clippy::chunks_exact_to_as_chunks)]

use model2::idmap::ramp_of_detail;
use model2::pack::{parse_name, FileKind};
use png::Image;
use std::collections::BTreeMap;

/// What texel values 0 to 15 are drawn in at full luma.
pub type Ramp = [[u8; 3]; 16];

/// Mean distance (levels per channel) from the nearest ramp colour below
/// which an authored image is taken to be a repaint or an upscale of the
/// game's own art: its colourings are baked without a word.
pub const BAKE_SILENT: f32 = 4.0;
/// Up to this distance the colourings are baked and listed for review;
/// from it, the image is a restyle the ramp cannot predict, and is copied.
pub const BAKE_REVIEW: f32 = 12.0;
/// Two ramps closer than this (mean level difference per entry and
/// channel) are one colouring: the image is copied, not baked.
pub const RAMPS_CLOSE: f32 = 4.0;

/// One row of a catalogue or texture map: what the artwork index reads.
pub struct Row<'a> {
    pub name: &'a str,
    /// The surface key, 16 hex digits (`-` for a tile: skipped).
    pub surface: &'a str,
    pub pixels: u64,
    pub detail: &'a str,
}

/// One colouring of an artwork.
#[derive(Clone, Debug)]
pub struct Variant {
    /// The pack file stem, `tex_<key>_<w>x<h>`.
    pub name: String,
    pub pixels: u64,
    /// None for a catalogue made before ramps were recorded.
    pub ramp: Option<Ramp>,
}

/// A surface key at a size, and every colouring seen of it.
#[derive(Clone, Debug)]
pub struct Artwork {
    pub surface: u64,
    pub w: u32,
    pub h: u32,
    /// Most pixels first.
    pub variants: Vec<Variant>,
}

impl Artwork {
    /// The pack file stem of the artwork's own image.
    pub fn stem(&self) -> String {
        format!("art_{:016x}_{}x{}", self.surface, self.w, self.h)
    }

    pub fn pixels(&self) -> u64 {
        self.variants.iter().map(|v| v.pixels).sum()
    }

    /// Distinct colours of the richest ramp.
    pub fn distinct(&self) -> usize {
        self.variants.iter().filter_map(|v| v.ramp.as_ref()).map(distinct).max().unwrap_or(0)
    }
}

impl Artwork {
    /// The colouring to paint over: the one with the most distinct colours
    /// in its ramp (a flatter ramp cannot give back detail it never had),
    /// then the one drawn most. Baking from it is exact on the game's own
    /// art; from a flatter colouring it loses what that ramp merged.
    pub fn richest(&self) -> Option<&Variant> {
        self.variants
            .iter()
            .filter(|v| v.ramp.is_some())
            .max_by_key(|v| (v.ramp.as_ref().map_or(0, distinct), v.pixels))
    }
}

fn distinct(r: &Ramp) -> usize {
    let mut v: Vec<[u8; 3]> = r.to_vec();
    v.sort();
    v.dedup();
    v.len()
}

/// Group texture rows into artworks, biggest first (by pixels drawn). Rows
/// that are not textures (tiles, names no pack reads) are left out.
pub fn group(rows: &[Row]) -> Vec<Artwork> {
    let mut by: BTreeMap<(u64, u32, u32), Artwork> = BTreeMap::new();
    for r in rows {
        // A per-palette variant is a colouring of its artwork too (its ramp is its own).
        let Ok(FileKind::Image { width, height, .. } | FileKind::Variant { width, height, .. }) = parse_name(r.name)
        else {
            continue;
        };
        let Some(surface) = (r.surface.len() == 16).then(|| u64::from_str_radix(r.surface, 16).ok()).flatten() else {
            continue;
        };
        let a = by.entry((surface, width, height)).or_insert_with(|| Artwork {
            surface,
            w: width,
            h: height,
            variants: Vec::new(),
        });
        if a.variants.iter().any(|v| v.name == r.name) {
            continue;
        }
        a.variants.push(Variant { name: r.name.to_string(), pixels: r.pixels, ramp: ramp_of_detail(r.detail) });
    }
    let mut v: Vec<Artwork> = by.into_values().collect();
    for a in &mut v {
        a.variants.sort_by(|x, y| y.pixels.cmp(&x.pixels).then(x.name.cmp(&y.name)));
    }
    v.sort_by(|a, b| b.pixels().cmp(&a.pixels()).then(a.surface.cmp(&b.surface)));
    v
}

fn dist(a: [u8; 3], b: [u8; 3]) -> f32 {
    (0..3).map(|c| (a[c] as f32 - b[c] as f32).abs()).sum::<f32>() / 3.0
}

fn nearest(c: [u8; 3], ramp: &Ramp) -> (f32, Vec<usize>) {
    let ds: Vec<f32> = ramp.iter().map(|&r| dist(c, r)).collect();
    let m = ds.iter().copied().fold(f32::MAX, f32::min);
    (m, ds.iter().enumerate().filter(|(_, &d)| d == m).map(|(i, _)| i).collect())
}

/// Mean distance of the image's opaque pixels from the nearest colour of
/// `ramp`, in levels per channel: how far the authored image lies from a
/// colouring of the original.
pub fn ramp_distance(img: &Image, ramp: &Ramp) -> f32 {
    let (mut sum, mut n) = (0.0f64, 0u64);
    for p in img.rgba.chunks_exact(4).filter(|p| p[3] >= 128) {
        sum += nearest([p[0], p[1], p[2]], ramp).0 as f64;
        n += 1;
    }
    if n == 0 {
        0.0
    } else {
        (sum / n as f64) as f32
    }
}

/// Mean level difference per entry and channel between two ramps.
pub fn ramps_apart(a: &Ramp, b: &Ramp) -> f32 {
    a.iter().zip(b).map(|(&x, &y)| dist(x, y)).sum::<f32>() / 16.0
}

/// The image as another colouring of the artwork: each pixel is placed on
/// the `from` ramp (its nearest colour), takes the same place on the `to`
/// ramp, and keeps whatever part of it lay off the ramp (its difference
/// from that colour). A pixel as near two ramp entries takes the mean of
/// their places. Alpha is kept.
pub fn bake(img: &Image, from: &Ramp, to: &Ramp) -> Image {
    let mut out = img.clone();
    for (o, p) in out.rgba.chunks_exact_mut(4).zip(img.rgba.chunks_exact(4)) {
        let c = [p[0], p[1], p[2]];
        let (_, at) = nearest(c, from);
        for ch in 0..3 {
            let place: f32 = at.iter().map(|&i| to[i][ch] as f32 - from[i][ch] as f32).sum::<f32>() / at.len() as f32;
            o[ch] = (c[ch] as f32 + place).round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// What the build does with an authored image, by how far it lies from the
/// ramp it was painted over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    /// Under `BAKE_SILENT`: bake every colouring.
    Faithful,
    /// Up to `BAKE_REVIEW`: bake, and list it for review.
    Review,
    /// Beyond: a restyle. Copy the image to every colouring and list it as
    /// "may want its own art".
    Restyle,
    /// No variant of the artwork has a ramp (a catalogue from before ramps
    /// were recorded): copy.
    NoRamp,
}

impl Band {
    pub fn name(self) -> &'static str {
        match self {
            Band::Faithful => "faithful",
            Band::Review => "review",
            Band::Restyle => "restyle",
            Band::NoRamp => "no-ramp",
        }
    }
}

/// How one variant's image was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    /// The authored image itself: the colouring it was painted over.
    Reference,
    Baked,
    /// Copied: a restyle, a colouring too like the reference, or no ramp.
    Copied,
}

/// The plan for one artwork: which variant the image was painted over, how
/// far from its ramp it lies, and what that means.
#[derive(Clone, Debug)]
pub struct Plan {
    /// Index into `Artwork::variants`.
    pub reference: usize,
    pub distance: f32,
    pub band: Band,
}

/// The reference is the colouring whose ramp the image lies nearest (the
/// one the author painted over); ties go to the richer ramp, then the one
/// drawn more.
pub fn plan(art: &Image, aw: &Artwork) -> Plan {
    let mut best: Option<(usize, f32)> = None;
    for (i, v) in aw.variants.iter().enumerate() {
        let Some(r) = &v.ramp else { continue };
        let d = ramp_distance(art, r);
        let better = match best {
            None => true,
            Some((j, bd)) => {
                d < bd - 0.01
                    || ((d - bd).abs() <= 0.01
                        && distinct(r) > distinct(aw.variants[j].ramp.as_ref().expect("a ramp, as it was chosen")))
            }
        };
        if better {
            best = Some((i, d));
        }
    }
    match best {
        None => Plan { reference: 0, distance: 0.0, band: Band::NoRamp },
        Some((reference, distance)) => Plan {
            reference,
            distance,
            band: if distance < BAKE_SILENT {
                Band::Faithful
            } else if distance < BAKE_REVIEW {
                Band::Review
            } else {
                Band::Restyle
            },
        },
    }
}

/// Variant `i`'s image, made from the authored one under `plan`.
pub fn make(art: &Image, aw: &Artwork, plan: &Plan, i: usize) -> (Image, How) {
    if i == plan.reference {
        return (art.clone(), How::Reference);
    }
    let (Some(from), Some(to)) = (&aw.variants[plan.reference].ramp, &aw.variants[i].ramp) else {
        return (art.clone(), How::Copied);
    };
    match plan.band {
        Band::Faithful | Band::Review if ramps_apart(from, to) >= RAMPS_CLOSE => (bake(art, from, to), How::Baked),
        _ => (art.clone(), How::Copied),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(f: impl Fn(usize) -> [u8; 3]) -> Ramp {
        std::array::from_fn(f)
    }

    fn image(px: &[[u8; 3]]) -> Image {
        Image { width: px.len(), height: 1, rgba: px.iter().flat_map(|c| [c[0], c[1], c[2], 255]).collect() }
    }

    fn red() -> Ramp {
        ramp(|i| [(i * 16) as u8, 0, 0])
    }
    fn blue() -> Ramp {
        ramp(|i| [0, 0, (255 - i * 8) as u8])
    }

    fn detail(r: &Ramp, colour: &str) -> String {
        format!("bank=0 x=0 y=0 luma=1 colour={colour} ramp={}", model2::idmap::ramp_text(r))
    }

    #[test]
    fn rows_group_by_surface_and_size_with_their_ramps() {
        let (d1, d2) = (detail(&red(), "0001"), detail(&blue(), "0002"));
        let rows = [
            Row { name: "tex_0000000000000001_8x8", surface: "00000000000000aa", pixels: 10, detail: &d1 },
            Row { name: "tex_0000000000000002_8x8", surface: "00000000000000aa", pixels: 30, detail: &d2 },
            Row { name: "tex_0000000000000003_8x8", surface: "00000000000000bb", pixels: 5, detail: "" },
            Row { name: "tile_0000000000000004", surface: "-", pixels: 99, detail: "" },
        ];
        let g = group(&rows);
        assert_eq!(g.len(), 2, "the tile is no artwork");
        assert_eq!(g[0].stem(), "art_00000000000000aa_8x8");
        assert_eq!(g[0].pixels(), 40);
        assert_eq!(g[0].variants[0].name, "tex_0000000000000002_8x8", "most drawn first");
        assert_eq!(g[0].variants[0].ramp, Some(blue()));
        assert!(g[1].variants[0].ramp.is_none());
    }

    #[test]
    fn the_richest_colouring_is_the_one_to_paint_over() {
        let flat = ramp(|i| [(i / 4 * 60) as u8, 0, 0]);
        let (d1, d2) = (detail(&flat, "0001"), detail(&red(), "0002"));
        let rows = [
            Row { name: "tex_0000000000000001_4x1", surface: "00000000000000aa", pixels: 90, detail: &d1 },
            Row { name: "tex_0000000000000002_4x1", surface: "00000000000000aa", pixels: 10, detail: &d2 },
        ];
        let aw = group(&rows).remove(0);
        assert_eq!(aw.variants[0].name, "tex_0000000000000001_4x1", "drawn most");
        assert_eq!(aw.richest().unwrap().name, "tex_0000000000000002_4x1", "most colours");
    }

    #[test]
    fn baking_onto_the_ramp_it_came_from_changes_nothing_and_onto_another_follows_it() {
        let img = image(&[[0, 0, 0], [16, 0, 0], [48, 0, 0], [240, 0, 0]]);
        assert_eq!(bake(&img, &red(), &red()), img);
        let b = bake(&img, &red(), &blue());
        let want: Vec<[u8; 3]> = [0usize, 1, 3, 15].iter().map(|&i| blue()[i]).collect();
        for (o, w) in b.rgba.chunks(4).zip(&want) {
            assert_eq!([o[0], o[1], o[2]], *w);
        }
    }

    #[test]
    fn what_lies_off_the_ramp_is_kept() {
        // One level above ramp entry 2: the offset survives the move.
        let img = image(&[[33, 2, 0]]);
        let b = bake(&img, &red(), &blue());
        let to = blue()[2];
        assert_eq!(&b.rgba[..3], &[to[0] + 1, 2, to[2]]);
        assert_eq!(b.rgba[3], 255, "alpha kept");
    }

    #[test]
    fn the_distance_picks_the_band_and_the_reference() {
        let rows_d = [detail(&red(), "0001"), detail(&blue(), "0002")];
        let rows = [
            Row { name: "tex_0000000000000001_4x1", surface: "00000000000000aa", pixels: 50, detail: &rows_d[0] },
            Row { name: "tex_0000000000000002_4x1", surface: "00000000000000aa", pixels: 10, detail: &rows_d[1] },
        ];
        let aw = group(&rows).remove(0);
        // Painted over the blue ramp, though red is drawn more.
        let on_blue = image(&[blue()[0], blue()[3], blue()[9], blue()[15]]);
        let p = plan(&on_blue, &aw);
        assert_eq!((aw.variants[p.reference].name.as_str(), p.band), ("tex_0000000000000002_4x1", Band::Faithful));
        let (red_img, how) = make(&on_blue, &aw, &p, 0);
        assert_eq!(how, How::Baked);
        assert_eq!(&red_img.rgba[..3], &red()[0]);
        let (own, how) = make(&on_blue, &aw, &p, 1);
        assert_eq!((how, own), (How::Reference, on_blue.clone()));
        // A restyle far from both ramps is copied.
        let neon = image(&[[0, 255, 0], [10, 250, 10], [0, 240, 30], [5, 255, 5]]);
        let p = plan(&neon, &aw);
        assert_eq!(p.band, Band::Restyle);
        assert!(p.distance >= BAKE_REVIEW);
        for i in 0..2 {
            assert_eq!(make(&neon, &aw, &p, i).0, neon);
        }
        // In between: baked, and listed.
        let mid = image(&[[8, 6, 6], [24, 6, 0], [40, 6, 6], [8, 0, 6]]);
        let p = plan(&mid, &aw);
        assert!(p.distance >= BAKE_SILENT && p.distance < BAKE_REVIEW, "{}", p.distance);
        assert_eq!(p.band, Band::Review);
    }

    #[test]
    fn two_alike_ramps_copy_and_a_catalogue_without_ramps_copies() {
        let mut near = red();
        near[3][0] += 2;
        let (d1, d2) = (detail(&red(), "0001"), detail(&near, "0002"));
        let rows = [
            Row { name: "tex_0000000000000001_4x1", surface: "00000000000000aa", pixels: 50, detail: &d1 },
            Row { name: "tex_0000000000000002_4x1", surface: "00000000000000aa", pixels: 10, detail: &d2 },
            Row { name: "tex_0000000000000003_4x1", surface: "00000000000000aa", pixels: 5, detail: "" },
        ];
        let aw = group(&rows).remove(0);
        let img = image(&[red()[1], red()[2], red()[3], red()[4]]);
        let p = plan(&img, &aw);
        assert_eq!(p.band, Band::Faithful);
        for i in [1, 2] {
            assert_eq!(make(&img, &aw, &p, i), (img.clone(), How::Copied));
        }
        let none =
            group(&[Row { name: "tex_0000000000000003_4x1", surface: "00000000000000cc", pixels: 1, detail: "" }]);
        assert_eq!(plan(&img, &none[0]).band, Band::NoRamp);
    }
}
