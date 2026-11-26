//! Per-palette variants (docs/texture-packs.md, "Per-palette variants"): one
//! texture key drawn through two different colour tables gets two different
//! pack images, the plain image where no variant matches, and a variant
//! added or removed while drawing takes effect the next frame. The software
//! renderer draws the same pictures as the GPU.

use crate::selflit_tests::Scene;
use crate::{Quality, Renderer};
use model2::enhance::{palette_id, Enhance, Found, PackChanges, Rgba, TexInfo, TexturePack, TileInfo};
use model2::raster::{place, tex_info, texture_look, Sources};
use model2::video::{View, WIDTH};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// A pack of images by file stem, variants by `tex_<key>~<pal>_<w>x<h>`,
/// changed by the test while it is drawn with.
#[derive(Clone, Default)]
struct Pack(Arc<Mutex<Inner>>);

#[derive(Default)]
struct Inner {
    images: HashMap<String, Arc<Rgba>>,
    changed: Vec<String>,
    /// Calls of the palette closure: the cost a draw adds.
    palette_calls: usize,
}

impl Pack {
    fn set(&self, stem: &str, image: Option<&Arc<Rgba>>) {
        let mut s = self.0.lock().unwrap();
        match image {
            Some(i) => s.images.insert(stem.to_string(), i.clone()),
            None => s.images.remove(stem),
        };
        s.changed.push(stem.to_string());
    }

    fn calls(&self) -> usize {
        self.0.lock().unwrap().palette_calls
    }
}

impl TexturePack for Pack {
    fn take_changes(&mut self) -> PackChanges {
        let mut s = self.0.lock().unwrap();
        PackChanges { stems: s.changed.drain(..).map(|c| (c, SystemTime::now())).collect(), reopen: false }
    }

    fn find(&mut self, info: &TexInfo) -> Found {
        let s = self.0.lock().unwrap();
        let own = (info.pal != 0).then(|| s.images.get(&info.variant_name())).flatten();
        own.or_else(|| s.images.get(&info.name())).cloned().map_or(Found::Missing, Found::Image)
    }

    fn variant(&mut self, info: &TexInfo, palette: &mut dyn FnMut() -> u32) -> u32 {
        let mut s = self.0.lock().unwrap();
        let prefix = format!("tex_{:016x}~", info.key);
        if !s.images.keys().any(|k| k.starts_with(&prefix)) {
            return 0;
        }
        s.palette_calls += 1;
        let pal = palette();
        let mut probe = info.clone();
        probe.pal = pal;
        if s.images.contains_key(&probe.variant_name()) {
            pal
        } else {
            0
        }
    }
}

impl Enhance for Pack {
    fn texture(&mut self, info: &TexInfo) -> Option<Arc<Rgba>> {
        match TexturePack::find(self, info) {
            Found::Image(i) => Some(i),
            _ => None,
        }
    }
    fn variant(&mut self, info: &TexInfo, palette: &mut dyn FnMut() -> u32) -> u32 {
        TexturePack::variant(self, info, palette)
    }
    fn texture_drawn(&mut self, _: &TexInfo, _: &Sources, _: usize) {}
    fn tile(&mut self, _: &TileInfo) -> Option<Arc<Rgba>> {
        None
    }
}

fn solid(rgb: [u8; 3]) -> Arc<Rgba> {
    Arc::new(Rgba { width: 8, height: 8, data: (0..64).flat_map(|_| [rgb[0], rgb[1], rgb[2], 255]).collect() })
}

/// A scene whose colour table differs from `Scene::new`'s by `k` levels
/// per step: the same texture key, a different ramp.
fn scene(k: u8) -> Scene {
    let mut s = Scene::new();
    for ch in 0..3 {
        for lum in 0..64usize {
            s.palette[0x1_0000 + ch * 0x4000 + (256 + lum) * 2] = (40 + k as usize * lum % 200) as u8;
        }
    }
    s
}

fn centre(rgb: &[u8]) -> [u8; 3] {
    let o = (200 * WIDTH + 250) * 3;
    [rgb[o], rgb[o + 1], rgb[o + 2]]
}

#[test]
fn one_key_two_palettes_two_images() {
    let Some(mut gpu) = Renderer::for_test() else { return };
    let (a, b, c) = (scene(3), scene(2), scene(1));
    let p = a.poly(255);
    let info = |s: &Scene| tex_info(&p, p.texture.as_ref().unwrap(), &s.src(), &mut Default::default()).unwrap();
    let pals: Vec<u32> = [&a, &b, &c].iter().map(|s| palette_id(&texture_look(&s.src(), &info(s)))).collect();
    assert_eq!(info(&a).key, info(&b).key, "the colour table is not in the key");
    assert!(pals[0] != pals[1] && pals[1] != pals[2] && pals[0] != pals[2], "three ramps: {pals:?}");
    let key = info(&a);
    let (red, blue, green) = (solid([220, 20, 20]), solid([20, 20, 220]), solid([20, 200, 20]));
    let with = |pal: u32| TexInfo { pal, ..key.clone() }.variant_name();

    let pack = Pack::default();
    pack.set(&with(pals[0]), Some(&red));
    pack.set(&with(pals[1]), Some(&blue));
    pack.set(&key.name(), Some(&green));
    pack.0.lock().unwrap().changed.clear();
    let mut polys = vec![p.clone()];
    let view = View::native();
    let placed = place(&mut polys, &view);
    gpu.set_pack(Some(Box::new(pack.clone())));
    gpu.set_quality(Quality::default());
    let draw = |gpu: &mut Renderer, s: &Scene| gpu.draw(&polys, &placed, &s.src(), &view);

    let (ga, gb, gc) = (draw(&mut gpu, &a), draw(&mut gpu, &b), draw(&mut gpu, &c));
    assert_eq!(centre(&ga.0), [220, 20, 20], "palette A draws its variant");
    assert_eq!(centre(&gb.0), [20, 20, 220], "palette B draws its own");
    assert_eq!(centre(&gc.0), [20, 200, 20], "palette C has no variant: the plain image");
    // Both renderers agree, for each palette.
    for (s, g) in [(&a, &ga), (&b, &gb), (&c, &gc)] {
        let mut soft = pack.clone();
        let (rgb, _) = model2::raster::draw_enhanced(&mut polys.clone(), &s.src(), &view, &mut soft);
        assert_eq!(centre(&rgb), centre(&g.0));
    }

    // A variant added while drawing: palette C draws it from the next frame.
    let white = solid([250; 3]);
    pack.set(&with(pals[2]), Some(&white));
    assert_eq!(centre(&draw(&mut gpu, &c).0), [250; 3]);
    // Removed: back to the plain image, and palette A is untouched.
    pack.set(&with(pals[2]), None);
    assert_eq!(centre(&draw(&mut gpu, &c).0), [20, 200, 20]);
    assert_eq!(centre(&draw(&mut gpu, &a).0), [220, 20, 20]);
    // The plain image removed: a palette with no variant draws the original.
    pack.set(&key.name(), None);
    let orig = draw(&mut gpu, &c).0;
    assert_ne!(centre(&orig), [20, 200, 20]);
    assert_eq!(centre(&draw(&mut gpu, &b).0), [20, 20, 220], "a variant needs no plain image");
}

#[test]
fn a_pack_without_variants_asks_nothing() {
    let Some(mut gpu) = Renderer::for_test() else { return };
    let s = Scene::new();
    let p = s.poly(255);
    let key = tex_info(&p, p.texture.as_ref().unwrap(), &s.src(), &mut Default::default()).unwrap();
    let pack = Pack::default();
    pack.set(&key.name(), Some(&solid([20, 200, 20])));
    let mut polys = vec![p];
    let view = View::native();
    let placed = place(&mut polys, &view);
    gpu.set_pack(Some(Box::new(pack.clone())));
    gpu.set_quality(Quality::default());
    let out = gpu.draw(&polys, &placed, &s.src(), &view);
    assert_eq!(centre(&out.0), [20, 200, 20]);
    assert_eq!(pack.calls(), 0, "no variant of the key: the palette is never worked out");
}

/// The cost a draw adds (run by hand: `cargo test -p remodel-gpu --release
/// variant_cost -- --ignored --nocapture`): the lookup of a pack without
/// variants, of one with variants for other keys only, of a key that has
/// some (the palette worked out and hashed), and the palette alone.
#[test]
#[ignore = "a measurement, printed"]
fn variant_cost() {
    use std::time::Instant;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/scratch");
    let (dir, none) = (root.join("variant-cost"), root.join("variant-cost-none"));
    for d in [&dir, &none] {
        let _ = std::fs::remove_dir_all(d);
        std::fs::create_dir_all(d).unwrap();
    }
    let s = Scene::new();
    let p = s.poly(255);
    let src = s.src();
    let info = tex_info(&p, p.texture.as_ref().unwrap(), &src, &mut Default::default()).unwrap();
    let png = png::encode_rgba(1, 1, &[1, 2, 3, 255]);
    let pal = palette_id(&texture_look(&src, &info));
    // 300 variants of 100 other keys, then the one of this key.
    for k in 0..100u64 {
        for j in 1..=3u32 {
            let i = TexInfo { key: 0x1000 + k, pal: j, ..info.clone() };
            std::fs::write(dir.join(format!("{}.png", i.variant_name())), &png).unwrap();
        }
    }
    std::fs::write(none.join(format!("{}.png", info.name())), &png).unwrap();
    let mut without = model2::dirpack::DirPack::open(&none).unwrap();
    let mut others = model2::dirpack::DirPack::open(&dir).unwrap();
    let mine = TexInfo { pal, ..info.clone() };
    std::fs::write(dir.join(format!("{}.png", mine.variant_name())), &png).unwrap();
    let mut with = model2::dirpack::DirPack::open(&dir).unwrap();
    const N: u32 = 2_000_000;
    let ns = |f: &mut dyn FnMut()| {
        let t = Instant::now();
        for _ in 0..N {
            f();
        }
        t.elapsed().as_nanos() as f64 / N as f64
    };
    let mut sink = 0u32;
    let a = ns(&mut || sink ^= TexturePack::variant(&mut without, &info, &mut || 1));
    let b = ns(&mut || sink ^= TexturePack::variant(&mut others, &info, &mut || 1));
    let c = ns(&mut || {
        sink ^= TexturePack::variant(&mut with, &info, &mut || model2::raster::texture_palette(&src, &info))
    });
    let d = ns(&mut || sink ^= model2::raster::texture_palette(&src, &info));
    println!(
        "variant lookup per draw: pack with none {a:.1} ns, variants of other keys only {b:.1} ns, \
         a key that has one {c:.1} ns (the palette alone {d:.1} ns); {sink}"
    );
}
