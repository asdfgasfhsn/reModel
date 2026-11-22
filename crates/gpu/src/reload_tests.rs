//! Live reload of one pack image (docs/uplift/scenes-and-server.md, "The
//! player"): a pack that reports a changed file is drawn again with just
//! that image replaced, the next frame, and the other images' pixels are
//! untouched. The same pack drawn from scratch gives the same picture.

use crate::selflit_tests::Scene;
use crate::{Quality, Renderer};
use model2::enhance::{Found, PackChanges, Rgba, TexInfo, TexturePack};
use model2::raster::place;
use model2::video::{View, WIDTH};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// A pack whose images, by file stem, the test changes while it is drawn
/// with, and which says so as the directory watch does.
#[derive(Clone, Default)]
struct Swap(Arc<Mutex<Inner>>);

#[derive(Default)]
struct Inner {
    images: HashMap<String, Arc<Rgba>>,
    changed: Vec<String>,
}

impl Swap {
    fn set(&self, stem: &str, image: Option<&Arc<Rgba>>) {
        let mut s = self.0.lock().unwrap();
        match image {
            Some(i) => s.images.insert(stem.to_string(), i.clone()),
            None => s.images.remove(stem),
        };
        s.changed.push(stem.to_string());
    }
}

impl TexturePack for Swap {
    fn take_changes(&mut self) -> PackChanges {
        let mut s = self.0.lock().unwrap();
        PackChanges { stems: s.changed.drain(..).map(|c| (c, SystemTime::now())).collect(), reopen: false }
    }
    fn find(&mut self, info: &TexInfo) -> Found {
        let s = self.0.lock().unwrap();
        s.images
            .get(&info.name())
            .or_else(|| s.images.get(&info.art_name()))
            .cloned()
            .map_or(Found::Missing, Found::Image)
    }
}

fn solid(rgb: [u8; 3]) -> Arc<Rgba> {
    Arc::new(Rgba { width: 8, height: 8, data: (0..64).flat_map(|_| [rgb[0], rgb[1], rgb[2], 255]).collect() })
}

/// Pixels (of 3 bytes) that differ between two frames, and the coverage
/// of the first.
fn diff(a: &(Vec<u8>, Vec<bool>), b: &(Vec<u8>, Vec<bool>)) -> Vec<usize> {
    assert_eq!(a.1, b.1, "the same polygons cover the same pixels");
    (0..a.1.len()).filter(|&i| a.0[i * 3..i * 3 + 3] != b.0[i * 3..i * 3 + 3]).collect()
}

#[test]
fn a_changed_image_is_swapped_alone() {
    let Some(mut gpu) = Renderer::for_test() else { return };
    let s = Scene::new();
    // Two textured quads of one artwork, in two colourings (two keys), side
    // by side.
    let (mut a, mut b) = (s.poly(255), s.poly(255));
    b.colour = [2, 2, 2];
    for (p, x) in [(&mut a, 0.0), (&mut b, 200.0)] {
        for v in &mut p.points {
            v.x = 60.0 + x + (v.x - 60.0) * 0.45;
        }
    }
    let src = s.src();
    let tex = |p: &model2::raster::Poly| {
        let t = p.texture.as_ref().unwrap();
        model2::raster::tex_info(p, t, &src, &mut Default::default()).unwrap()
    };
    let (ia, ib) = (tex(&a), tex(&b));
    assert_ne!(ia.key, ib.key);
    assert_eq!(ia.surface, ib.surface);
    let mut polys = vec![a, b];
    let view = View::native();
    let placed = place(&mut polys, &view);
    let (red, blue, green, white) = (solid([220, 20, 20]), solid([20, 20, 220]), solid([20, 200, 20]), solid([250; 3]));

    let pack = Swap::default();
    pack.set(&ia.name(), Some(&red));
    pack.set(&ib.name(), Some(&blue));
    pack.0.lock().unwrap().changed.clear();
    gpu.set_pack(Some(Box::new(pack.clone())));
    gpu.set_quality(Quality::default());
    let draw = |gpu: &mut Renderer| gpu.draw(&polys, &placed, &src, &view);
    let first = draw(&mut gpu);
    assert!(first == draw(&mut gpu), "nothing changed: the same picture");

    // The reference for each state: the pack of that state, opened fresh.
    let mut fresh = Renderer::for_test().unwrap();
    let mut reference = |images: &[(&str, &Arc<Rgba>)]| {
        let p = Swap::default();
        for (stem, i) in images {
            p.set(stem, Some(i));
        }
        p.0.lock().unwrap().changed.clear();
        fresh.set_pack(Some(Box::new(p)));
        fresh.set_quality(Quality::default());
        // Drawn twice: the pack's stabilisations (a colouring without an image
        // borrows its sibling's) settle on the second frame.
        fresh.draw(&polys, &placed, &src, &view);
        fresh.draw(&polys, &placed, &src, &view)
    };
    assert!(first == reference(&[(&ia.name(), &red), (&ib.name(), &blue)]), "first differs from a fresh pack");

    // The first quad's image changes: only its pixels change, to the picture
    // of a pack that always had the new image.
    pack.set(&ia.name(), Some(&green));
    let second = draw(&mut gpu);
    let changed = diff(&first, &second);
    assert!(changed.len() > 10_000, "{} pixels changed", changed.len());
    let bx = polys[1].points.iter().map(|p| p.x).fold(f32::MAX, f32::min) as usize;
    let ax = polys[0].points.iter().map(|p| p.x).fold(f32::MIN, f32::max) as usize;
    assert!(changed.iter().all(|i| i % WIDTH < ax && i % WIDTH < bx), "only the first quad's pixels");
    assert!(second == reference(&[(&ia.name(), &green), (&ib.name(), &blue)]), "second differs from a fresh pack");
    let done = gpu.take_reloaded();
    assert_eq!(done.len(), 1, "{done:?}");
    assert_eq!(done[0].0, ia.name());

    // Its file is removed: the colouring then stands in with its sibling's image, as a pack that never had it does.
    pack.set(&ia.name(), None);
    draw(&mut gpu);
    let third = draw(&mut gpu);
    let want = reference(&[(&ib.name(), &blue)]);
    let d = diff(&third, &want);
    assert!(
        d.is_empty(),
        "third differs from a fresh pack in {} pixels, first {:?} got {:?} want {:?}",
        d.len(),
        d.first().map(|i| (i % WIDTH, i / WIDTH)),
        &third.0[d[0] * 3..d[0] * 3 + 3],
        &want.0[d[0] * 3..d[0] * 3 + 3]
    );
    assert!(!diff(&second, &third).is_empty());

    // An artwork image is added: the first quad draws it (its colouring has
    // no image of its own), the second keeps its own.
    let art = ia.art_name();
    pack.set(&art, Some(&white));
    let fourth = draw(&mut gpu);
    assert!(fourth == reference(&[(&art, &white), (&ib.name(), &blue)]), "fourth differs from a fresh pack");
    assert!(!diff(&third, &fourth).is_empty());
    // The removal, and the artwork image for both colourings of its surface (the
    // second one is read again and keeps its own).
    assert_eq!(gpu.take_reloaded().len(), 3);
    assert!(!gpu.take_pack_reopen());
}
