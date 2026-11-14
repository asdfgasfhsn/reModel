//! Material maps of a texture pack on the GPU (docs/texture-packs.md,
//! "Material maps"): looked up per texture, uploaded as one `Rgba8Unorm`
//! texture each (`model2::enhance::Material`: normal x, y, specular,
//! gloss; never sRGB), with mip levels when `tex_filter` has them, and kept
//! under the pack's memory budget with its images. The material pipelines
//! (`Variant::Mat`, `Variant::MatFiltered`; raster.wgsl's `fs_mat*`) bind
//! one in group 1 binding 2 beside the polygon's image.

use model2::enhance::{FoundMaterial, Material, TexInfo, TexturePack};
use std::collections::{HashMap, HashSet};

/// A material's texture.
pub(crate) struct MatTex {
    /// The whole texture (all its levels), for the material pipelines to
    /// bind beside the image.
    pub view: wgpu::TextureView,
    /// Mip levels (`Material::mip_chain`) or level 0 alone.
    pub mipmapped: bool,
    /// Its normal maps' strength (`Material::normal_scale`).
    pub normal_scale: f32,
    pub bytes: u64,
    /// Frame it was last drawn in (for eviction, with the images).
    pub used: u64,
}

/// The renderer's materials: which texture has which, and the textures.
#[derive(Default)]
pub(crate) struct Materials {
    /// Material id by texture key (None: the texture has none). An id whose
    /// texture was dropped is asked for again.
    of_key: HashMap<u64, Option<u64>>,
    /// Uploaded materials by id.
    pub tex: HashMap<u64, MatTex>,
    /// Materials drawn this frame, and uploaded this frame.
    drawn: HashSet<u64>,
    pub uploaded: u32,
    /// A material was too large for the GPU (warned once).
    warned: bool,
    /// Group 1 of the material pipelines by (image key, material id,
    /// address modes; None: the 1x1 stand-in), made when first needed and
    /// dropped with either texture (`forget_image`, `forget_material`).
    pub groups: HashMap<crate::SlotKey, wgpu::BindGroup>,
    /// 1x1 stand-ins for group 1 where a polygon has no image or no
    /// material (never sampled then).
    pub dummy: Option<[wgpu::TextureView; 2]>,
}

impl Materials {
    /// Forget everything (another pack, or materials off).
    pub fn clear(&mut self) {
        *self = Materials { warned: self.warned, ..Default::default() };
    }

    /// The image of texture `key` was dropped: its bind groups too.
    pub fn forget_image(&mut self, key: u64) {
        self.groups.retain(|k, _| k.0 != Some(key));
    }

    /// Material `id` was dropped: its texture and bind groups.
    pub fn forget_material(&mut self, id: u64) {
        self.tex.remove(&id);
        self.groups.retain(|k, _| k.1 != Some(id));
    }

    /// Before a frame's lookups.
    pub fn begin(&mut self) {
        self.drawn.clear();
        self.uploaded = 0;
    }

    /// Keep only the textures of form `mipmapped` (`tex_filter` changed;
    /// the bind groups, whose samplers changed, all go).
    pub fn retain_form(&mut self, mipmapped: bool) {
        self.tex.retain(|_, t| t.mipmapped == mipmapped);
        self.groups.clear();
    }

    /// The material of `info`'s texture, drawn in `frame`: looked up in
    /// `pack` the first time (and after its texture was dropped), uploaded
    /// when new, stamped. Its id, None when it has none (or not yet).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        pack: &mut dyn TexturePack,
        info: &TexInfo,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mipmapped: bool,
        frame: u64,
        max: usize,
    ) -> Option<u64> {
        let id = match self.of_key.get(&info.key) {
            Some(None) => return None,
            Some(Some(id)) if self.tex.contains_key(id) => *id,
            _ => match pack.material(info) {
                FoundMaterial::Pending => return None,
                FoundMaterial::Missing => {
                    self.of_key.insert(info.key, None);
                    return None;
                }
                FoundMaterial::Maps(m) => {
                    if m.width > max || m.height > max {
                        if !self.warned {
                            self.warned = true;
                            eprintln!(
                                "texture pack: the material maps of {} are {}x{}, more than this GPU's {max}x{max}; \
                                 drawing without them",
                                info.name(),
                                m.width,
                                m.height
                            );
                        }
                        self.of_key.insert(info.key, None);
                        return None;
                    }
                    self.of_key.insert(info.key, Some(m.id));
                    if let std::collections::hash_map::Entry::Vacant(e) = self.tex.entry(m.id) {
                        e.insert(upload(device, queue, &m, mipmapped));
                        self.uploaded += 1;
                    }
                    m.id
                }
            },
        };
        if let Some(t) = self.tex.get_mut(&id) {
            t.used = frame;
        }
        self.drawn.insert(id);
        Some(id)
    }

    /// The 1x1 stand-ins (an image, a material), uploaded once.
    pub fn dummies(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> [wgpu::TextureView; 2] {
        self.dummy
            .get_or_insert_with(|| {
                [[0u8; 4], [128u8; 4]].map(|texel| {
                    crate::upload_levels(device, queue, "material stand-in", std::iter::once((1, 1, &texel[..]))).0
                })
            })
            .clone()
    }

    /// Distinct materials drawn this frame.
    pub fn drawn(&self) -> u32 {
        self.drawn.len() as u32
    }

    /// Bytes held on the GPU.
    pub fn bytes(&self) -> u64 {
        self.tex.values().map(|t| t.bytes).sum()
    }
}

/// Upload a material: one level, or its mip chain.
fn upload(device: &wgpu::Device, queue: &wgpu::Queue, m: &Material, mipmapped: bool) -> MatTex {
    let chain;
    let levels: &[Material] = if mipmapped {
        chain = m.mip_chain();
        &chain
    } else {
        std::slice::from_ref(m)
    };
    let (view, bytes) = crate::upload_levels(
        device,
        queue,
        "pack material",
        levels.iter().map(|l| (l.width as u32, l.height as u32, &l.data[..])),
    );
    MatTex { view, mipmapped, normal_scale: m.normal_scale, bytes, used: 0 }
}

#[cfg(test)]
mod tests {
    use crate::{Quality, Renderer, Stipple};
    use model2::enhance::{Enhance, Found, FoundMaterial, Material, Rgba, TexFilter, TexInfo, TexturePack, TileInfo};
    use model2::light::{Base, Lamp, PixelLight};
    use model2::raster::{place, Poly, PolyLamp, Relief, SVtx, Sources, Tangent, Texture};
    use model2::video::{Layers2d, View, HEIGHT, WIDTH};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// Every texture has a 16x16 image; textures of the widths in `maps`
    /// have a flat 32x32 material. Counts material lookups by width.
    struct Pack {
        maps: Vec<u32>,
        asked: Arc<Mutex<HashMap<u32, u32>>>,
    }

    impl TexturePack for Pack {
        fn find(&mut self, _: &TexInfo) -> Found {
            Found::Image(Arc::new(Rgba { width: 16, height: 16, data: [200, 100, 50, 255].repeat(256) }))
        }
        fn has_materials(&self) -> bool {
            true
        }
        fn material(&mut self, info: &TexInfo) -> FoundMaterial {
            *self.asked.lock().unwrap().entry(info.width).or_default() += 1;
            if !self.maps.contains(&info.width) {
                return FoundMaterial::Missing;
            }
            let n = Rgba { width: 32, height: 32, data: [128, 128, 255, 255].repeat(1024) };
            let name = format!("{}.n", info.name());
            let m =
                Material::assemble([Some(&n), None, None], &model2::pack::Manifest::NONE, [Some(&name), None, None]);
            FoundMaterial::Maps(Arc::new(m))
        }
    }

    /// A lamp lighting a polygon facing the eye from the upper left, with
    /// a tangent frame along the screen's x.
    fn lamp(ambient: f32) -> PolyLamp {
        let l = [-0.5f32, 0.4, 0.75];
        let len = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
        PolyLamp {
            light: PixelLight {
                lamp: Lamp {
                    l: l.map(|c| c / len),
                    ambient,
                    diffuse: 180.0,
                    spec_scale: 40.0,
                    squarings: 2,
                    front: true,
                    base: Base::D,
                },
                offset: 0.0,
            },
            n: [0.0, 0.0, 1.0],
            out: -1.0,
            tangent: Some(Tangent { t: [1.0, 0.0, 0.0], h: 1.0 }),
            range: [0, 255],
            relief: Relief::default(),
        }
    }

    /// A textured quad, `w` texels wide (the width makes its key), with a
    /// lamp.
    fn quad(x: f32, w: u32) -> Poly {
        let v = |x, y| SVtx { x, y, iz: 1.0, uz: 0.0, vz: 0.0 };
        Poly {
            points: vec![v(x, 50.0), v(x + 100.0, 50.0), v(x + 100.0, 150.0), v(x, 150.0)],
            key: 1.0,
            window: 0,
            clip: (0.0, 0.0, WIDTH as f32, HEIGHT as f32),
            texture: Some(Texture {
                x0: 0,
                y0: 0,
                w,
                h: 32,
                mirror_x: false,
                mirror_y: false,
                bank: 0,
                translucent: false,
                luma_table: 0,
            }),
            colour: [1, 1, 1],
            luma: 128,
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
            lamp: Some(Box::new(lamp(40.0))),
        }
    }

    /// Materials (flat here) are uploaded and counted and change no pixel;
    /// over the budget they are dropped with the images, least recently
    /// drawn first, and asked for again when next drawn.
    #[test]
    fn materials_load_draw_nothing_and_share_the_budget() {
        let Some(mut gpu) = Renderer::for_test() else { return };
        let render = vec![0u8; 0x48_0000];
        let palette = vec![0u8; 0x2_0000];
        let src = Sources {
            texture_rom: &[],
            texture_point_ram: &[],
            render_ram: &render,
            palette: &palette,
            xoff: 0.0,
            yoff: 0.0,
        };
        let view = View::native();
        let asked = Arc::new(Mutex::new(HashMap::new()));
        let draw = |gpu: &mut Renderer, widths: &[u32]| {
            let mut polys: Vec<Poly> =
                widths.iter().enumerate().map(|(i, &w)| quad(20.0 + 120.0 * i as f32, w)).collect();
            let placed = place(&mut polys, &view);
            let out = gpu.draw(&polys, &placed, &src, &view);
            (out, gpu.pack_stats())
        };
        let pack = |maps: &[u32]| Box::new(Pack { maps: maps.to_vec(), asked: asked.clone() });
        let (image, mat) = (16 * 16 * 4u64, 32 * 32 * 4u64);
        for filter in [TexFilter::Off, TexFilter::Mip] {
            // Off: never asked; on: the same pictures.
            gpu.set_quality(Quality { tex_filter: filter, ..Default::default() });
            gpu.set_pack(Some(pack(&[32, 64])));
            let (off, s) = draw(&mut gpu, &[32, 64, 128]);
            assert!(asked.lock().unwrap().is_empty());
            assert_eq!((s.replaced, s.materials, s.material_held, s.material_bytes), (3, 0, 0, 0));
            gpu.set_quality(Quality { tex_filter: filter, materials: true, ..Default::default() });
            gpu.set_pack(Some(pack(&[32, 64])));
            let (on, s) = draw(&mut gpu, &[32, 64, 128]);
            assert!(on == off, "materials change no pixel");
            // A 32x32 mip chain: 1365 texels.
            let one = if filter.mipmapped() { 4 * 1365 } else { mat };
            assert_eq!((s.materials, s.material_uploaded, s.material_held, s.material_bytes), (2, 2, 2, 2 * one));
            // Kept: not asked for again.
            let (_, s) = draw(&mut gpu, &[32, 64, 128]);
            assert_eq!((s.materials, s.material_uploaded, s.material_held), (2, 0, 2));
            assert_eq!(asked.lock().unwrap().values().sum::<u32>(), 3);
            asked.lock().unwrap().clear();
        }
        // One budget of two images and one material. Frame 1 draws 32 and 64
        // (an image and a material each), frame 2 draws 128 (an image
        // alone): over budget, frame 1's are dropped in one order (smaller
        // first within a frame: the images, then a material) until it fits.
        gpu.set_quality(Quality { materials: true, ..Default::default() });
        gpu.set_pack(Some(pack(&[32, 64])));
        gpu.pack_budget = 2 * image + mat;
        let (_, s) = draw(&mut gpu, &[32, 64]);
        // This frame's are never dropped, even over the budget.
        assert_eq!((s.held, s.material_held, s.bytes + s.material_bytes), (2, 2, 2 * image + 2 * mat));
        assert_eq!(gpu.materials.groups.len(), 2, "a bind group per image and material");
        let (_, s) = draw(&mut gpu, &[128]);
        assert_eq!((s.held, s.material_held), (1, 1), "{s:?}");
        assert_eq!(s.bytes + s.material_bytes, image + mat);
        // No bind group outlives a texture it binds.
        let kept = |gpu: &Renderer| {
            gpu.materials.groups.keys().all(|(image, material, _)| {
                image.is_none_or(|k| gpu.pack_tex.get(&k).is_some_and(Option::is_some))
                    && material.is_none_or(|id| gpu.materials.tex.contains_key(&id))
            })
        };
        assert!(kept(&gpu));
        // Drawn again: the dropped material is asked for and uploaded again.
        asked.lock().unwrap().clear();
        let (_, s) = draw(&mut gpu, &[32, 64]);
        assert_eq!((s.materials, s.material_uploaded), (2, 1), "{s:?}");
        assert_eq!(asked.lock().unwrap().values().sum::<u32>(), 1);
        // Materials off: dropped at once.
        gpu.set_quality(Quality::default());
        let (_, s) = draw(&mut gpu, &[32, 64]);
        assert_eq!((s.materials, s.material_held, s.material_bytes), (0, 0, 0));
    }

    /// A 64x64 material: a dome of radius 20 texels around (32, 32), its
    /// normals leaning out from the centre (x along u, y along v), and
    /// ridges along v elsewhere; or flat everywhere.
    fn dome(flat: bool) -> Material {
        let mut data = Vec::with_capacity(64 * 64 * 4);
        for y in 0..64 {
            for x in 0..64 {
                let (dx, dy) = (x as f32 + 0.5 - 32.0, y as f32 + 0.5 - 32.0);
                let r = (dx * dx + dy * dy).sqrt();
                let (nx, ny) = if flat {
                    (0.0, 0.0)
                } else if r < 20.0 {
                    (0.6 * dx / 20.0, 0.6 * dy / 20.0)
                } else {
                    (0.4 * (x as f32 * 0.7).sin(), 0.0)
                };
                let enc = |v: f32| (128.0 + 127.0 * v).round() as u8;
                data.extend([enc(nx), enc(ny), 128, 128]);
            }
        }
        Material { width: 64, height: 64, data, has: 1, normal_scale: 1.0, spec_scale: 1.0, id: 7 }
    }

    /// The same pack for both renderers: maybe an image for every texture,
    /// a material for every texture (whichever polygons ask).
    #[derive(Clone)]
    struct Maps {
        image: Option<Arc<Rgba>>,
        material: Option<Arc<Material>>,
    }

    impl TexturePack for Maps {
        fn find(&mut self, _: &TexInfo) -> Found {
            self.image.clone().map_or(Found::Missing, Found::Image)
        }
        fn has_materials(&self) -> bool {
            true
        }
        fn material(&mut self, _: &TexInfo) -> FoundMaterial {
            self.material.clone().map_or(FoundMaterial::Missing, FoundMaterial::Maps)
        }
    }

    impl Enhance for Maps {
        fn texture(&mut self, _: &TexInfo) -> Option<Arc<Rgba>> {
            self.image.clone()
        }
        fn texture_drawn(&mut self, _: &TexInfo, _: &Sources, _: usize) {}
        fn tile(&mut self, _: &TileInfo) -> Option<Arc<Rgba>> {
            None
        }
        fn texture_material(&mut self, _: &TexInfo) -> Option<Arc<Material>> {
            self.material.clone()
        }
    }

    fn maps(image: bool, material: Option<Material>) -> Maps {
        let image = image.then(|| {
            let data =
                (0..128 * 128).flat_map(|i| [60 + (i % 128) as u8, 90 + (i / 128 % 64) as u8, 140, 255]).collect();
            Arc::new(Rgba { width: 128, height: 128, data })
        });
        Maps { image, material: material.map(Arc::new) }
    }

    /// A scene of textured polygons with lamps, in perspective (1/z varying
    /// across them), repeated and mirrored, one lit to full luma whatever
    /// its normal, one smooth-shaded with turning corner normals (per pixel
    /// with `pixel`), and polygons without a lamp or texture.
    fn scene(pixel: bool) -> Vec<Poly> {
        let tex = |x0: u32, w: u32, h: u32, mirror: bool| Texture {
            x0,
            y0: 64,
            w,
            h,
            mirror_x: mirror,
            mirror_y: mirror,
            bank: 0,
            translucent: false,
            luma_table: 3,
        };
        // A planar quad in view space around `c` with half-edges `e1`, `e2`,
        // projected (focal length 495, centre (248, 192)), its texture
        // coordinates 0..u along e1 and 0..v along e2: perspective-correct
        // as the game's polygons are.
        let mk = |c: [f32; 3], e1: [f32; 3], e2: [f32; 3], (u, v): (f32, f32), t: Texture| {
            let pt = |a: f32, b: f32| {
                let p = [0, 1, 2].map(|k| c[k] + a * e1[k] + b * e2[k]);
                let iz = 1.0 / p[2];
                let (s, tt) = ((a + 1.0) / 2.0 * u, (b + 1.0) / 2.0 * v);
                SVtx { x: 248.0 + 495.0 * p[0] * iz, y: 192.0 - 495.0 * p[1] * iz, iz, uz: s * iz, vz: tt * iz }
            };
            let mut p = quad(0.0, t.w);
            p.points = vec![pt(-1.0, -1.0), pt(1.0, -1.0), pt(1.0, 1.0), pt(-1.0, 1.0)];
            p.texture = Some(t);
            p
        };
        let mut v = Vec::new();
        let mut a = mk([-1.4, 0.85, 6.0], [1.1, 0.0, 0.5], [0.0, 0.7, 0.2], (128.0, 128.0), tex(0, 64, 64, false));
        a.luma = 150;
        v.push(a);
        let mut b = mk([1.4, 0.85, 6.0], [1.1, 0.0, -0.4], [0.0, 0.7, 0.0], (96.0, 160.0), tex(64, 32, 64, true));
        b.luma = 90;
        b.colour = [20, 9, 3];
        v.push(b);
        // Lit to full luma at every normal.
        let mut c = mk([-2.0, -0.9, 6.0], [0.6, 0.0, 0.0], [0.0, 0.7, 0.0], (64.0, 64.0), tex(128, 64, 64, false));
        c.luma = 255;
        c.lamp = Some(Box::new(lamp(255.0)));
        v.push(c);
        // Smooth-shaded: turning corner normals, the lighting per pixel.
        let mut d = mk([-0.35, -0.9, 6.0], [0.7, 0.0, 0.3], [0.0, 0.7, 0.0], (64.0, 64.0), tex(192, 64, 64, false));
        let l = lamp(40.0).light;
        d.lit = (0..4)
            .map(|k| {
                let a = k as f32 * 1.7;
                let n = [0.4 * a.cos(), 0.4 * a.sin(), 0.82];
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                let n = n.map(|c| c / len);
                [model2::light::clamp_corner(l.luma(n)), n[0], n[1], n[2]]
            })
            .collect();
        d.pixel = pixel.then_some(l);
        d.luma = l.luma([0.0, 0.0, 1.0]) as u32;
        d.colour = [5, 25, 12];
        v.push(d);
        // No lamp; untextured.
        let mut e = mk([1.6, -0.6, 6.0], [0.7, 0.0, 0.0], [0.0, 0.4, 0.0], (64.0, 64.0), tex(256, 64, 64, false));
        e.lamp = None;
        v.push(e);
        let mut f = mk([1.6, -1.5, 6.0], [0.7, 0.0, 0.0], [0.0, 0.3, 0.0], (1.0, 1.0), tex(0, 32, 32, false));
        f.texture = None;
        f.lamp = None;
        v.push(f);
        for (k, p) in v.iter_mut().enumerate() {
            p.key = 10.0 - k as f32;
        }
        v
    }

    /// Random video RAM (every luma table value and colour a random one,
    /// so any difference of level shows), luma table 3 brighter than most;
    /// with `smooth`, colour tables that are ramps (column k about 4k) and
    /// luma table 3 rising with the index, so that a pixel's colour follows
    /// its brightness and a difference measures one.
    fn ram(smooth: bool) -> (Vec<u8>, Vec<u8>) {
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
            render[0x40_0000 + (3 * 128 + i) * 2] = if smooth { 64 + i as u8 } else { 160 + (i as u8 % 96) };
        }
        if smooth {
            for ch in 0..3 {
                for c in 0..32 {
                    for lum in 0..64 {
                        palette[0x1_0000 + ch * 0x4000 + (c * 256 + lum) * 2] = (4 * lum + c / 4).min(255) as u8;
                    }
                }
            }
        }
        (render, palette)
    }

    fn with_relief(polys: &[Poly], relief: Relief) -> Vec<Poly> {
        polys
            .iter()
            .map(|p| {
                let mut p = p.clone();
                if let Some(l) = p.lamp.as_mut() {
                    l.relief = relief;
                }
                p
            })
            .collect()
    }

    /// Material maps on the GPU (`fs_mat*`): a pack of flat maps draws
    /// byte for byte the frame without materials, with the original texels
    /// or an image, unfiltered, mipmapped and anisotropic, with the depth
    /// buffer, multisampling and temporal anti-aliasing, smooth-shaded or
    /// not, under both rules; a real map changes it; the temporal pipelines
    /// draw the plain ones' frame on an unjittered first frame; and the GPU
    /// agrees with the software renderer as closely as without materials.
    #[test]
    fn material_pipelines_identities_and_software() {
        let Some(gpu) = Renderer::for_test() else { return };
        // One renderer, its pack set again (forgetting every texture) and
        // its history reset before each frame: a fresh renderer's frame.
        let gpu = std::cell::RefCell::new(gpu);
        let (render, palette) = ram(false);
        let src = Sources {
            texture_rom: &[],
            texture_point_ram: &[],
            render_ram: &render,
            palette: &palette,
            xoff: 0.0,
            yoff: 0.0,
        };
        let view = View::native();
        let layers = Layers2d {
            base: vec![0; WIDTH * HEIGHT * 4],
            side: 0,
            hud: vec![0; WIDTH * HEIGHT * 4],
            ext_w: WIDTH,
            backdrop: [0, 0, 0],
            tiles: None,
        };
        let frame = |q: Quality, polys: &[Poly], pack: &Maps| {
            let mut polys = polys.to_vec();
            let placed = place(&mut polys, &view);
            let mut g = gpu.borrow_mut();
            g.set_pack(Some(Box::new(pack.clone())));
            let q = g.set_quality(q);
            g.reset_history();
            (g.frame(&polys, &placed, &src, &view, &layers), g.pack_stats(), q)
        };
        let pixel = |q: Quality| Quality { shading: crate::Shading::Pixel, crease: 40, ..q };
        for (image, filter) in [false, true]
            .into_iter()
            .flat_map(|i| [TexFilter::Off, TexFilter::Mip, TexFilter::Aniso(16)].map(|f| (i, f)))
        {
            for relief in [Relief::default(), Relief::Faithful] {
                let polys = with_relief(&scene(true), relief);
                for (msaa, depth, temporal) in
                    [(1, false, 0), (1, true, 0), (4, true, 0), (1, false, 100), (1, true, 67)]
                {
                    let q = pixel(Quality {
                        msaa,
                        depth,
                        temporal,
                        tex_filter: filter,
                        stipple: Stipple::Keep,
                        ..Default::default()
                    });
                    let what =
                        format!("image {image} {filter} {relief:?} msaa {msaa} depth {depth} temporal {temporal}");
                    let (off, _, _) = frame(q, &polys, &maps(image, Some(dome(false))));
                    let on = Quality { materials: true, ..q };
                    let (flat, s, _) = frame(on, &polys, &maps(image, Some(dome(true))));
                    assert!(s.materials == 1 && s.material_held == 1, "{what}: {s:?}");
                    assert!(flat == off, "flat maps: {what}");
                    let (dome_on, s, _) = frame(on, &polys, &maps(image, Some(dome(false))));
                    assert!(dome_on != off, "a dome shows: {what}");
                    assert!(s.materials == 1, "{what}: {s:?}");
                    if temporal == 100 {
                        // The `_t` entry points on an unjittered first frame
                        // draw what the plain ones draw.
                        let (plain, _, _) =
                            frame(Quality { temporal: 0, ..on }, &polys, &maps(image, Some(dome(false))));
                        assert!(dome_on == plain, "temporal first frame: {what}");
                    }
                }
            }
        }
        let (render, palette) = ram(true);
        let src = Sources { render_ram: &render, palette: &palette, ..src };
        // Against the software renderer (the 3D layer alone): no more
        // pixels apart by more than one step per channel than without
        // materials, beyond a few in a thousand (bilinear weights, f32
        // order of operations, a column flipped at a boundary).
        let near = |q: Quality, polys: &[Poly], pack: &Maps| {
            let mut gp = polys.to_vec();
            let placed = place(&mut gp, &view);
            let mut g = gpu.borrow_mut();
            g.reset_history();
            g.set_pack(Some(Box::new(pack.clone())));
            g.set_quality(q);
            let (rgb, covered) = g.draw(&gp, &placed, &src, &view);
            let mut hooks = pack.clone();
            let (srgb, scovered) = model2::raster::draw_enhanced_filtered(
                &mut polys.to_vec(),
                &src,
                &view,
                &mut hooks,
                q.depth,
                q.tex_filter,
            );
            let far = (0..covered.len())
                .filter(|&i| {
                    covered[i] != scovered[i]
                        || (covered[i] && (0..3).any(|c| rgb[i * 3 + c].abs_diff(srgb[i * 3 + c]) > 1))
                })
                .count();
            (far, covered.iter().filter(|&&c| c).count())
        };
        for (pixel_lit, image, filter, relief, depth) in [false, true].into_iter().flat_map(|px| {
            [false, true].into_iter().flat_map(move |i| {
                [TexFilter::Off, TexFilter::Mip].into_iter().flat_map(move |f| {
                    [Relief::default(), Relief::Faithful, Relief::Diffuse { tenths: 80 }]
                        .into_iter()
                        .flat_map(move |r| [false, true].map(move |d| (px, i, f, r, d)))
                })
            })
        }) {
            let shading = if pixel_lit { crate::Shading::Pixel } else { crate::Shading::Vertex };
            let q = Quality { depth, tex_filter: filter, shading, crease: 40, ..Default::default() };
            let polys = with_relief(&scene(pixel_lit), relief);
            let (floor, n) = near(q, &polys, &maps(image, None));
            let (got, _) = near(Quality { materials: true, ..q }, &polys, &maps(image, Some(dome(false))));
            let what = format!("{shading:?} image {image} {filter} {relief:?} depth {depth}");
            assert!(n > 50_000 && got <= floor + n / 200, "{what}: {got} against {floor} of {n}");
        }
    }
}
