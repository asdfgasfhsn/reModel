//! Tests of the look's passes (`super`), on the GPU: outlines appear on a
//! depth step and on a texture boundary and nowhere inside a flat area,
//! toon lighting quantises into its steps and leaves unlit polygons alone,
//! a tag opts a texture out, and a look with nothing on draws the frame
//! it would without one, byte for byte.

use crate::{Quality, Renderer, Shading};
use model2::look::{Look, Tag};
use model2::raster::{place, tex_info, Placed, Poly, SVtx, Sources, TexKeys, Texture};
use model2::video::{Hud, Layers2d, View, HEIGHT, WIDTH};
use std::sync::Arc;

/// A quad from (x0, y0) to (x1, y1) in game pixels at 1/z `iz`.
fn quad(x0: f32, y0: f32, x1: f32, y1: f32, iz: f32, colour: [u32; 3], key: f32) -> Poly {
    let v = |x, y| SVtx { x, y, iz, uz: 0.0, vz: 0.0 };
    Poly {
        points: vec![v(x0, y0), v(x1, y0), v(x1, y1), v(x0, y1)],
        key,
        window: 0,
        clip: (0.0, 0.0, WIDTH as f32, HEIGHT as f32),
        texture: None,
        colour,
        luma: 240,
        checker: false,
        header: [0; 4],
        block: 0,
        at: 0,
        prev: Vec::new(),
        shadow: false,
        lit: Vec::new(),
        pixel: None,
        lit_range: [0, 255],
        fine: false,
        lamp: None,
    }
}

/// Video RAM where a pixel's colour follows its brightness (colour table
/// column k is about 4k), texel values random.
fn ram() -> (Vec<u8>, Vec<u8>) {
    let mut seed = 0x5eed_1234u32;
    let mut bytes = |n: usize| {
        (0..n)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 24) as u8
            })
            .collect::<Vec<u8>>()
    };
    let (mut render, mut palette) = (bytes(0x48_0000), bytes(0x2_0000));
    for i in 0..128 {
        render[0x40_0000 + (3 * 128 + i) * 2] = 64 + i as u8;
    }
    for ch in 0..3 {
        for c in 0..32 {
            for lum in 0..64 {
                palette[0x1_0000 + ch * 0x4000 + (c * 256 + lum) * 2] = (4 * lum + c / 4).min(255) as u8;
            }
        }
    }
    (render, palette)
}

const W: usize = 1440;

struct Scene {
    render: Vec<u8>,
    palette: Vec<u8>,
    view: View,
    layers: Layers2d,
}

impl Scene {
    fn new() -> Scene {
        let (render, palette) = ram();
        let view = View::new(1080, 4.0 / 3.0, Hud::Center);
        assert_eq!(view.width, W);
        let layers = Layers2d {
            base: vec![0; WIDTH * HEIGHT * 4],
            side: 0,
            hud: vec![0; WIDTH * HEIGHT * 4],
            ext_w: WIDTH,
            backdrop: [0, 0, 0],
            tiles: None,
        };
        Scene { render, palette, view, layers }
    }

    /// Game pixel to output pixel, x and y.
    fn px(&self) -> (impl Fn(usize) -> usize, impl Fn(usize) -> usize) {
        let (sx, sy) = (self.view.sx, self.view.sy);
        (move |x| (x as f32 * sx).round() as usize, move |y| (y as f32 * sy).round() as usize)
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

    /// `polys` drawn with `look` (and `quality`), as RGB.
    fn draw(&self, gpu: &mut Renderer, polys: &[Poly], look: Option<Look>, quality: Quality) -> Vec<u8> {
        let mut polys = polys.to_vec();
        let placed: Vec<Placed> = place(&mut polys, &self.view);
        gpu.set_quality(quality);
        gpu.set_look(look.map(Arc::new));
        gpu.frame(&polys, &placed, &self.src(), &self.view, &self.layers)
    }
}

/// Output pixel (x, y) of a game pixel position.
fn at(rgb: &[u8], x: usize, y: usize) -> [u8; 3] {
    let i = (y * W + x) * 3;
    [rgb[i], rgb[i + 1], rgb[i + 2]]
}

fn lum(c: [u8; 3]) -> i32 {
    c.iter().map(|&v| v as i32).sum::<i32>() / 3
}

/// A look with the outline pass only, no fade, at weight 2.
fn ink_look() -> Look {
    let mut l = Look::default();
    for (k, v) in [
        ("toon_steps", "0"),
        ("outline_weight", "2"),
        ("outline_opacity", "1"),
        ("outline_fade_near", "100000"),
        ("outline_fade_far", "100000"),
    ] {
        l.params.set(k, v).unwrap();
    }
    l
}

/// Far backdrop, a near box on the left and, at the same depth beside it,
/// another of another colour (a texture boundary); a tilted floor under
/// them (1/z varying across it).
fn flat_scene() -> Vec<Poly> {
    let mut floor = quad(0.0, 270.0, 496.0, 384.0, 0.002, [4, 20, 6], 800.0);
    floor.points[0].iz = 0.0012;
    floor.points[1].iz = 0.0012;
    floor.points[2].iz = 0.0044;
    floor.points[3].iz = 0.0044;
    vec![
        quad(-10.0, -10.0, 510.0, 400.0, 0.001, [6, 12, 28], 900.0),
        floor,
        quad(60.0, 60.0, 180.0, 200.0, 0.004, [28, 8, 4], 500.0),
        quad(180.0, 60.0, 300.0, 200.0, 0.004, [4, 28, 8], 400.0),
    ]
}

/// A depth step draws ink on the nearer side, a boundary between two
/// textures at one depth draws a line, and flat areas (including a tilted
/// floor, steeply changing in 1/z) stay untouched.
#[test]
fn outlines_on_depth_steps_and_texture_boundaries_only() {
    let Some(mut gpu) = Renderer::for_test() else { return };
    let scene = Scene::new();
    let polys = flat_scene();
    let off = scene.draw(&mut gpu, &polys, None, Quality::default());
    let on = scene.draw(&mut gpu, &polys, Some(ink_look()), Quality::default());
    let (gx, gy) = scene.px();
    // Output pixels of the red box's left edge (x = 60 game pixels), and
    // in its middle: ink on the box's side of the edge.
    let (ex, ey) = (gx(60), gy(130));
    assert_ne!(at(&off, ex + 1, ey), at(&on, ex + 1, ey), "no ink at the box's edge");
    assert!(lum(at(&on, ex + 1, ey)) < 40, "ink is dark: {:?}", at(&on, ex + 1, ey));
    // Not on the far side of the step.
    assert_eq!(at(&off, ex - 4, ey), at(&on, ex - 4, ey), "ink on the far side");
    // Inside a flat area.
    assert_eq!(at(&off, gx(120), ey), at(&on, gx(120), ey), "ink inside the box");
    assert_eq!(at(&off, gx(10), gy(30)), at(&on, gx(10), gy(30)), "ink on the backdrop");
    // The floor: a tilted plane, its middle and near the tilt's steep end.
    for (x, y) in [(250, 300), (400, 350), (30, 280)] {
        assert_eq!(at(&off, gx(x), gy(y)), at(&on, gx(x), gy(y)), "ink inside the tilted floor at {x},{y}");
    }
    // The boundary of the two boxes (x = 180) is between coplanar pieces
    // (the same depth): painted, not a shape, so no line.
    let (bx, by) = (gx(180), gy(130));
    let changed = (bx - 4..bx + 4).filter(|&x| at(&off, x, by) != at(&on, x, by)).count();
    assert_eq!(changed, 0, "{changed} pixels of ink between coplanar pieces");
    // Weight scales with the picture: a quarter of the height has thinner
    // lines (counted in pixels, 2 at 1080: about 1 at 540).
    let thick = (ex..ex + 12).filter(|&x| at(&off, x, ey) != at(&on, x, ey)).count();
    assert!((2..=3).contains(&thick), "line thickness {thick}");
}

/// Coplanar polygons of different textures draw no line between them,
/// level or tilted (a floor of many pieces is not a drawing of them); the
/// same pieces meeting at a crease do; and a tag naming an outline weight
/// lines the boundary anyway.
#[test]
fn texture_lines_only_where_the_surface_bends() {
    let Some(mut gpu) = Renderer::for_test() else { return };
    let scene = Scene::new();
    let (gx, gy) = scene.px();
    let (bx, by) = (gx(250), gy(300));
    // Two floor pieces side by side with different textures, one plane
    // tilted away down the picture and rising to the right.
    let plane = |x: f32, y: f32| 0.002 + 0.00001 * x + 0.00002 * (y - 270.0);
    let tex = |y0| Texture {
        x0: 64,
        y0,
        w: 32,
        h: 32,
        mirror_x: false,
        mirror_y: false,
        bank: 0,
        translucent: false,
        luma_table: 3,
    };
    let floor = |x0: f32, x1: f32, y0: u32| {
        let mut q = quad(x0, 270.0, x1, 384.0, 0.0, [4, 20, 6], 800.0 + y0 as f32);
        q.texture = Some(tex(y0));
        for (i, v) in q.points.iter_mut().enumerate() {
            v.iz = plane(v.x, v.y);
            v.uz = [0.0, 31.0, 31.0, 0.0][i] * v.iz;
            v.vz = [0.0, 0.0, 31.0, 31.0][i] * v.iz;
        }
        q
    };
    let backdrop = quad(-10.0, -10.0, 510.0, 400.0, 0.001, [6, 12, 28], 900.0);
    let level = vec![backdrop, floor(0.0, 250.0, 32), floor(250.0, 496.0, 96)];
    let off = scene.draw(&mut gpu, &level, None, Quality::default());
    let on = scene.draw(&mut gpu, &level, Some(ink_look()), Quality::default());
    let ink = |off: &[u8], on: &[u8]| (bx - 6..bx + 6).filter(|&x| at(off, x, by) != at(on, x, by)).count();
    assert_eq!(ink(&off, &on), 0, "ink between coplanar floor pieces");
    // A crease: the right piece rises steeper.
    let mut creased = level.clone();
    for v in creased[2].points.iter_mut() {
        v.iz = plane(v.x, v.y) + 0.00025 * (v.x - 250.0);
    }
    let off_c = scene.draw(&mut gpu, &creased, None, Quality::default());
    let on_c = scene.draw(&mut gpu, &creased, Some(ink_look()), Quality::default());
    let n = ink(&off_c, &on_c);
    assert!((1..=4).contains(&n), "{n} pixels of ink at a crease between textures");
    // A tag naming an outline weight lines the coplanar boundary anyway.
    let mut look = ink_look();
    for (i, y0) in [(1, 32), (2, 96)] {
        let info = tex_info(&level[i], &tex(y0), &scene.src(), &mut TexKeys::default()).unwrap();
        look.tags.insert(info.name(), Tag { outline: Some(1.0), ..Tag::default() });
    }
    let tagged = scene.draw(&mut gpu, &level, Some(look), Quality::default());
    let n = ink(&off, &tagged);
    assert!((1..=4).contains(&n), "{n} pixels of ink at a tagged boundary");
}

/// A ground quad running from the camera to the horizon is drawn in the
/// game's order, without depth (`background_reason`: "spread"), beside
/// depth-tested pieces of the same plane: it is in the ID pass like any
/// surface, so its seam draws no line (the stray ink across Top Skater's
/// ground came from its being missing there).
#[test]
fn a_ground_drawn_in_the_games_order_has_no_seam_line() {
    let Some(mut gpu) = Renderer::for_test() else { return };
    let scene = Scene::new();
    let (gx, gy) = scene.px();
    // One plane, 1/z rising 10 times down the picture.
    let plane = |y: f32| 0.0005 + 0.00004 * (y - 270.0);
    let ground = |x0: f32, x1: f32, y0: f32, colour, key| {
        let mut q = quad(x0, y0, x1, 384.0, 0.0, colour, key);
        for v in q.points.iter_mut() {
            v.iz = plane(v.y);
        }
        q
    };
    let mut wide = ground(0.0, 250.0, 270.0, [4, 20, 6], 800.0);
    wide.points[0].y = 270.0;
    let polys = vec![
        quad(-10.0, -10.0, 510.0, 400.0, 0.0001, [6, 12, 28], 900.0),
        wide,
        ground(250.0, 496.0, 340.0, [20, 4, 6], 700.0),
    ];
    assert!(model2::raster::background_reason(&polys[1]).is_some(), "the wide ground is a background");
    let off = scene.draw(&mut gpu, &polys, None, Quality::default());
    let on = scene.draw(&mut gpu, &polys, Some(ink_look()), Quality::default());
    let (bx, by) = (gx(250), gy(370));
    let ink = (bx - 8..bx + 8).filter(|&x| at(&off, x, by) != at(&on, x, by)).count();
    assert_eq!(ink, 0, "{ink} pixels of ink at the seam of the ground");
}

/// With the lines off, or on nothing at all, the frame is the one drawn
/// without a look, byte for byte; and the effects off cost no target.
#[test]
fn a_look_with_nothing_on_draws_the_frame_without_one() {
    let Some(mut gpu) = Renderer::for_test() else { return };
    let scene = Scene::new();
    let polys = flat_scene();
    for depth in [false, true] {
        let q = Quality { depth, ..Default::default() };
        let plain = scene.draw(&mut gpu, &polys, None, q);
        let mut quiet = Look::default();
        quiet.params.set("outline", "off").unwrap();
        quiet.params.set("toon_steps", "0").unwrap();
        quiet.params.set("halftone", "0.5").unwrap();
        quiet.params.set("rim", "0.5").unwrap();
        assert!(!quiet.active());
        assert_eq!(scene.draw(&mut gpu, &polys, Some(quiet), q), plain, "depth {depth}");
        // Back to none after a look: the same again.
        assert_eq!(scene.draw(&mut gpu, &polys, None, q), plain);
    }
    // No ID or depth target was made for any of that.
    assert!(!gpu.textures.iter().any(|(l, _)| l.starts_with("look") || *l == "3d look"));
    // With the lines on they are.
    scene.draw(&mut gpu, &polys, Some(ink_look()), Quality::default());
    assert!(gpu.textures.iter().any(|(l, _)| *l == "look ids"));
}

/// A tagged texture draws no ink at its edges; the same texture untagged
/// does.
#[test]
fn a_tag_opts_a_texture_out() {
    let Some(mut gpu) = Renderer::for_test() else { return };
    let scene = Scene::new();
    let tex = Texture {
        x0: 64,
        y0: 32,
        w: 32,
        h: 32,
        mirror_x: false,
        mirror_y: false,
        bank: 0,
        translucent: false,
        luma_table: 3,
    };
    let mut near = quad(60.0, 60.0, 180.0, 200.0, 0.004, [28, 8, 4], 500.0);
    near.texture = Some(tex);
    for (i, v) in near.points.iter_mut().enumerate() {
        v.uz = [0.0, 31.0, 31.0, 0.0][i] * 0.004;
        v.vz = [0.0, 0.0, 31.0, 31.0][i] * 0.004;
    }
    let polys = vec![quad(-10.0, -10.0, 510.0, 400.0, 0.001, [6, 12, 28], 900.0), near.clone()];
    let info = tex_info(&near, &tex, &scene.src(), &mut TexKeys::default()).unwrap();
    let (gx, gy) = scene.px();
    let (ex, ey) = (gx(60), gy(130));
    let off = scene.draw(&mut gpu, &polys, None, Quality::default());
    let untagged = scene.draw(&mut gpu, &polys, Some(ink_look()), Quality::default());
    assert_ne!(at(&off, ex + 1, ey), at(&untagged, ex + 1, ey), "no ink without the tag");
    let mut look = ink_look();
    look.tags.insert(info.name(), Tag { outline: Some(0.0), ..Tag::default() });
    let tagged = scene.draw(&mut gpu, &polys, Some(look), Quality::default());
    assert_eq!(off, tagged, "a tagged texture is outlined");
    // The surface key works as well.
    let mut look = ink_look();
    look.tags.insert(info.surface_name(), Tag { outline: Some(0.0), ..Tag::default() });
    assert_eq!(off, scene.draw(&mut gpu, &polys, Some(look), Quality::default()));
}

/// A smooth-lit polygon's brightness across it, in `steps` hard bands: the
/// colours along it are the steps' and no others; unlit polygons keep
/// theirs; and the darkest band's halftone puts the dots' colour in it.
#[test]
fn toon_lighting_quantises_into_its_steps() {
    let Some(mut gpu) = Renderer::for_test() else { return };
    let scene = Scene::new();
    let mut lit = quad(40.0, 60.0, 440.0, 160.0, 0.004, [28, 8, 4], 500.0);
    // Brightness 30 at the left to 250 at the right.
    lit.lit = [30.0, 250.0, 250.0, 30.0].map(|l| [l, 0.0, 0.0, 1.0]).to_vec();
    let flat = quad(40.0, 200.0, 440.0, 300.0, 0.004, [28, 8, 4], 400.0);
    let polys = vec![lit, flat];
    let q = Quality { shading: Shading::Vertex, ..Default::default() };
    let (px, py) = scene.px();
    // Away from the polygon's own edges (a pixel or two each end).
    let row = |rgb: &[u8], g: usize| -> Vec<[u8; 3]> { (px(40) + 3..px(440) - 3).map(|x| at(rgb, x, py(g))).collect() };
    let distinct = |v: &[[u8; 3]]| {
        let mut d = v.to_vec();
        d.sort();
        d.dedup();
        d
    };
    let smooth = scene.draw(&mut gpu, &polys, None, q);
    assert!(distinct(&row(&smooth, 110)).len() > 20, "the smooth row has many levels");
    for steps in [2u32, 3, 4] {
        let mut look = Look::default();
        look.params.set("outline", "off").unwrap();
        look.params.set("toon_steps", &steps.to_string()).unwrap();
        let toon = scene.draw(&mut gpu, &polys, Some(look), q);
        let d = distinct(&row(&toon, 110));
        assert_eq!(d.len(), steps as usize, "{steps} steps give {d:?}");
        // Dark to light along the row, in order.
        let r = row(&toon, 110);
        assert!(r.windows(2).all(|w| lum(w[0]) <= lum(w[1])), "bands are monotonic");
        // The unlit polygon is as before.
        assert_eq!(row(&toon, 250), row(&smooth, 250));
    }
    // Halftone: dots of the ink colour appear in the darkest band only.
    let mut look = Look::default();
    look.params.set("outline", "off").unwrap();
    look.params.set("halftone", "0.5").unwrap();
    look.params.set("halftone_colour", "#ff00ff").unwrap();
    let dots = scene.draw(&mut gpu, &polys, Some(look), q);
    let r = row(&dots, 110);
    let magenta = |c: &[u8; 3]| c[0] > 200 && c[1] < 60 && c[2] > 200;
    let first = r.iter().position(magenta).expect("dots in the dark band");
    let last = r.iter().rposition(magenta).unwrap();
    // The dark band is the left part of the row only.
    assert!(last < r.len() * 6 / 10, "dots reach {last} of {}", r.len());
    assert!(first < last);
    // Coverage is about what was asked for within the band (a loose check).
    let band = &r[..last + 1];
    let share = band.iter().filter(|c| magenta(c)).count() as f32 / band.len() as f32;
    assert!((0.2..0.7).contains(&share), "dot share {share}");
}

/// An emissive tag draws a texture at full brightness whatever the
/// polygon's luma.
#[test]
fn an_emissive_tag_draws_full_brightness() {
    let Some(mut gpu) = Renderer::for_test() else { return };
    let scene = Scene::new();
    let tex = Texture {
        x0: 64,
        y0: 32,
        w: 32,
        h: 32,
        mirror_x: false,
        mirror_y: false,
        bank: 0,
        translucent: false,
        luma_table: 3,
    };
    let mut p = quad(60.0, 60.0, 180.0, 200.0, 0.004, [28, 8, 4], 500.0);
    p.texture = Some(tex);
    p.luma = 90;
    let info = tex_info(&p, &tex, &scene.src(), &mut TexKeys::default()).unwrap();
    let polys = vec![p];
    let (gx, gy) = scene.px();
    let dim = scene.draw(&mut gpu, &polys, None, Quality::default());
    let mut look = Look::default();
    look.params.set("outline", "off").unwrap();
    look.params.set("toon_steps", "0").unwrap();
    look.tags.insert(info.name(), Tag { emissive: true, ..Tag::default() });
    assert!(look.active());
    let lit = scene.draw(&mut gpu, &polys, Some(look), Quality::default());
    let (a, b) = (at(&dim, gx(120), gy(130)), at(&lit, gx(120), gy(130)));
    assert!(lum(b) > lum(a) + 20, "emissive {b:?} against {a:?}");
}
