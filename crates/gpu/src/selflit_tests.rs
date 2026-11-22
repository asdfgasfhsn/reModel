//! Draw rules and overlays (docs/uplift/renderer-controls.md), on the GPU
//! against the software renderer: a `selflit` pack image is drawn as
//! painted at any polygon brightness, the `miss` overlay colours a textured
//! polygon with no pack image, and with both off the picture is the one drawn
//! without them, byte for byte.

use crate::{Quality, Renderer};
use model2::enhance::{Enhance, Found, Rgba, TexInfo, TexturePack, TileInfo};
use model2::raster::{place, Poly, SVtx, Sources, Texture, MISS_COLOUR};
use model2::video::{View, HEIGHT, WIDTH};
use std::sync::Arc;

/// A pack of one image for every texture (or none), `selflit` or not, with
/// the overlay on or off: the same object stands in for the CLI's pack on
/// both renderers.
#[derive(Clone)]
struct Rule {
    image: Option<Arc<Rgba>>,
    selflit: bool,
    miss: bool,
}

impl TexturePack for Rule {
    fn find(&mut self, _: &TexInfo) -> Found {
        self.image.clone().map_or(Found::Missing, Found::Image)
    }
    fn image_selflit(&mut self, _: &TexInfo) -> Option<bool> {
        self.image.as_ref().map(|_| self.selflit)
    }
}

impl Enhance for Rule {
    fn texture(&mut self, _: &TexInfo) -> Option<Arc<Rgba>> {
        self.image.clone()
    }
    fn texture_selflit(&mut self, _: &TexInfo) -> bool {
        self.selflit
    }
    fn overlay_miss(&mut self) -> bool {
        self.miss
    }
    fn texture_drawn(&mut self, _: &TexInfo, _: &Sources, _: usize) {}
    fn tile(&mut self, _: &TileInfo) -> Option<Arc<Rgba>> {
        None
    }
}

struct Scene {
    render: Vec<u8>,
    palette: Vec<u8>,
}

impl Scene {
    /// Texel values 0, colour 1 with a translation table that rises with
    /// brightness: the original pipeline darkens at a low luma.
    fn new() -> Scene {
        let mut render = vec![0u8; 0x48_0000];
        render[0x40_0000] = 64;
        let mut palette = vec![0u8; 0x2_0000];
        for ch in 0..3 {
            for lum in 0..64usize {
                palette[0x1_0000 + ch * 0x4000 + (256 + lum) * 2] = (40 + 3 * lum) as u8;
            }
        }
        Scene { render, palette }
    }

    fn src(&self) -> Sources<'_> {
        Sources {
            texture_rom: &[],
            texture_point_ram: &[],
            render_ram: &self.render,
            palette: &self.palette,
            xoff: 0.0,
            yoff: 0.0,
        }
    }

    /// A textured quad at brightness `luma`.
    fn poly(&self, luma: u32) -> Poly {
        let t = Texture {
            x0: 0,
            y0: 0,
            w: 32,
            h: 32,
            mirror_x: false,
            mirror_y: false,
            bank: 0,
            translucent: false,
            luma_table: 0,
        };
        let v = |x, y, uz, vz| SVtx { x, y, iz: 1.0, uz, vz };
        Poly {
            points: vec![
                v(60.0, 100.0, 0.0, 0.0),
                v(440.0, 100.0, 32.0, 0.0),
                v(440.0, 300.0, 32.0, 32.0),
                v(60.0, 300.0, 0.0, 32.0),
            ],
            key: 1.0,
            window: 0,
            clip: (0.0, 0.0, WIDTH as f32, HEIGHT as f32),
            texture: Some(t),
            colour: [1, 1, 1],
            luma,
            checker: false,
            header: [0; 4],
            shadow: false,
            block: 0,
            at: 0,
            prev: Vec::new(),
            lit: Vec::new(),
            pixel: None,
            lit_range: [0, 255],
            fine: false,
            lamp: None,
        }
    }

    fn soft(&self, p: &Poly, rule: &mut Rule) -> (Vec<u8>, Vec<bool>) {
        model2::raster::draw_enhanced(&mut [p.clone()], &self.src(), &View::native(), rule)
    }

    fn gpu(&self, gpu: &mut Renderer, p: &Poly, rule: Option<Rule>) -> (Vec<u8>, Vec<bool>) {
        let mut polys = vec![p.clone()];
        let view = View::native();
        let placed = place(&mut polys, &view);
        gpu.set_pack(rule.clone().map(|r| Box::new(r) as Box<dyn TexturePack + Send>));
        gpu.set_overlay_miss(rule.is_some_and(|r| r.miss));
        gpu.set_quality(Quality::default());
        gpu.draw(&polys, &placed, &self.src(), &view)
    }
}

/// A painted image whose colours are not what the game's table gives.
fn painted() -> Arc<Rgba> {
    let mut data = Vec::new();
    for y in 0..32u32 {
        for x in 0..32u32 {
            data.extend_from_slice(&[(x * 8).min(255) as u8, (y * 8).min(255) as u8, 200, 255]);
        }
    }
    Arc::new(Rgba { width: 32, height: 32, data })
}

/// How many of the pixels both draws cover differ by more than one level in
/// a channel.
fn differ(a: &[u8], b: &[u8], ca: &[bool], cb: &[bool]) -> (usize, usize) {
    let both: Vec<usize> = (0..ca.len()).filter(|&i| ca[i] && cb[i]).collect();
    let off = both.iter().filter(|&&i| (0..3).any(|c| a[i * 3 + c].abs_diff(b[i * 3 + c]) > 1)).count();
    (off, both.len())
}

/// `selflit` draws the image as painted whatever the polygon's brightness:
/// the software renderer's result at luma 128 is the one at full luma, and
/// the GPU draws the same; without the rule the dim polygon is darker.
#[test]
fn selflit_draws_as_painted_on_both_renderers() {
    let Some(mut gpu) = Renderer::for_test() else { return };
    let s = Scene::new();
    let rule = |selflit| Rule { image: Some(painted()), selflit, miss: false };
    let (dim, dim_cov) = s.soft(&s.poly(128), &mut rule(false));
    let (lit, lit_cov) = s.soft(&s.poly(128), &mut rule(true));
    let (full, full_cov) = s.soft(&s.poly(255), &mut rule(false));
    assert_eq!(dim_cov, lit_cov);
    assert_eq!(lit, full, "software: selflit at luma 128 is the image at full brightness");
    assert_eq!(lit_cov, full_cov);
    let n = lit_cov.iter().filter(|&&c| c).count();
    assert!(n > 50_000, "{n} covered");
    let (off, both) = differ(&dim, &lit, &dim_cov, &lit_cov);
    assert!(off > both / 2, "the game's lighting changes {off} of {both} pixels without the rule");
    // The GPU.
    let (g_lit, g_cov) = s.gpu(&mut gpu, &s.poly(128), Some(rule(true)));
    let (g_full, gf_cov) = s.gpu(&mut gpu, &s.poly(255), Some(rule(false)));
    assert_eq!(g_lit, g_full, "GPU: selflit at luma 128 is the image at full brightness");
    assert_eq!(g_cov, gf_cov);
    let (off, both) = differ(&g_lit, &lit, &g_cov, &lit_cov);
    assert!(both > n * 99 / 100 && off <= both / 500, "GPU against software: {off} of {both} differ");
    let (g_dim, g_dcov) = s.gpu(&mut gpu, &s.poly(128), Some(rule(false)));
    let (off, both) = differ(&g_dim, &dim, &g_dcov, &dim_cov);
    assert!(off <= both / 500, "without the rule, GPU against software: {off} of {both} differ");
}

/// The `miss` overlay colours the textured polygon that has no pack image,
/// the same on both renderers, leaves one that has an image alone, and off
/// draws what a pack of no image does.
#[test]
fn the_miss_overlay_colours_what_has_no_image() {
    let Some(mut gpu) = Renderer::for_test() else { return };
    let s = Scene::new();
    let p = s.poly(128);
    let none = |miss| Rule { image: None, selflit: false, miss };
    let (soft, cov) = s.soft(&p, &mut none(true));
    let n = cov.iter().filter(|&&c| c).count();
    assert!(n > 50_000);
    assert!((0..cov.len()).filter(|&i| cov[i]).all(|i| soft[i * 3..i * 3 + 3] == MISS_COLOUR), "software");
    let (g, gcov) = s.gpu(&mut gpu, &p, Some(none(true)));
    let hit = (0..gcov.len()).filter(|&i| gcov[i] && g[i * 3..i * 3 + 3] == MISS_COLOUR).count();
    let all = gcov.iter().filter(|&&c| c).count();
    assert!(all > 50_000 && hit == all, "GPU: {hit} of {all} in the overlay colour");
    // A polygon with an image is not a miss.
    let with = Rule { image: Some(painted()), selflit: false, miss: true };
    let (soft, cov) = s.soft(&p, &mut with.clone());
    assert!((0..cov.len()).filter(|&i| cov[i]).all(|i| soft[i * 3..i * 3 + 3] != MISS_COLOUR));
    let (g, gcov) = s.gpu(&mut gpu, &p, Some(with));
    assert!((0..gcov.len()).filter(|&i| gcov[i]).all(|i| g[i * 3..i * 3 + 3] != MISS_COLOUR));
    // Off: a pack with no image is the original, byte for byte.
    let (off, off_cov) = s.gpu(&mut gpu, &p, Some(none(false)));
    let (orig, orig_cov) = s.gpu(&mut gpu, &p, None);
    assert_eq!((off, off_cov), (orig, orig_cov));
}
