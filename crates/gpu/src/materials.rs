//! Material maps of a texture pack on the GPU (docs/texture-packs.md,
//! "Material maps"): looked up per texture, uploaded as one `Rgba8Unorm`
//! texture each (`model2::enhance::Material`: normal x, y, specular,
//! gloss; never sRGB), with mip levels when `tex_filter` has them, and kept
//! under the pack's memory budget with its images. Nothing samples them yet.

use model2::enhance::{FoundMaterial, Material, TexInfo, TexturePack};
use std::collections::{HashMap, HashSet};

/// A material's texture.
pub(crate) struct MatTex {
    /// The whole texture (all its levels), for the material pipelines to
    /// bind beside the image.
    #[expect(dead_code, reason = "bound once materials are drawn")]
    pub view: wgpu::TextureView,
    /// Mip levels (`Material::mip_chain`) or level 0 alone.
    pub mipmapped: bool,
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
}

impl Materials {
    /// Forget everything (another pack, or materials off).
    pub fn clear(&mut self) {
        *self = Materials { warned: self.warned, ..Default::default() };
    }

    /// Before a frame's lookups.
    pub fn begin(&mut self) {
        self.drawn.clear();
        self.uploaded = 0;
    }

    /// Keep only the textures of form `mipmapped` (`tex_filter` changed).
    pub fn retain_form(&mut self, mipmapped: bool) {
        self.tex.retain(|_, t| t.mipmapped == mipmapped);
    }

    /// The material of `info`'s texture, drawn in `frame`: looked up in
    /// `pack` the first time (and after its texture was dropped), uploaded
    /// when new, stamped.
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
    ) {
        let id = match self.of_key.get(&info.key) {
            Some(None) => return,
            Some(Some(id)) if self.tex.contains_key(id) => *id,
            _ => match pack.material(info) {
                FoundMaterial::Pending => return,
                FoundMaterial::Missing => {
                    self.of_key.insert(info.key, None);
                    return;
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
                        return;
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
    MatTex { view, mipmapped, bytes, used: 0 }
}

#[cfg(test)]
mod tests {
    use crate::{Quality, Renderer};
    use model2::enhance::{Found, FoundMaterial, Material, Rgba, TexFilter, TexInfo, TexturePack};
    use model2::raster::{place, Poly, SVtx, Sources, Texture};
    use model2::video::{View, HEIGHT, WIDTH};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// Every texture has a 16x16 image; textures of the widths in `maps`
    /// have a 32x32 material. Counts material lookups by width.
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
            let n = Rgba { width: 32, height: 32, data: [200, 90, 0, 0].repeat(1024) };
            let name = format!("{}.n", info.name());
            let m =
                Material::assemble([Some(&n), None, None], &model2::pack::Manifest::NONE, [Some(&name), None, None]);
            FoundMaterial::Maps(Arc::new(m))
        }
    }

    /// A textured quad, `w` texels wide (the width makes its key).
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
            lamp: None,
        }
    }

    /// Materials are uploaded and counted but change no pixel; over the
    /// budget they are dropped with the images, least recently drawn first,
    /// and asked for again when next drawn.
    #[test]
    fn materials_load_draw_nothing_and_share_the_budget() {
        let Ok(mut gpu) = Renderer::new() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
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
        let (_, s) = draw(&mut gpu, &[128]);
        assert_eq!((s.held, s.material_held), (1, 1), "{s:?}");
        assert_eq!(s.bytes + s.material_bytes, image + mat);
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
}
