//! Looks (docs/enhancements.md, "Looks"; model2::look has the file format):
//! what the GPU draws for a `look.txt`.
//!
//! * **Toon lighting and halftone** are in the 3D layer's own shader
//!   (`raster.wgsl`, `poly_px`): a smooth-shaded pixel's brightness is
//!   quantised into hard bands before the game's colour tables turn it into
//!   a colour, and the darkest band can carry dots. Nothing is drawn
//!   differently where the game didn't light a polygon.
//! * **Ink outlines, rim light and speed lines** are one pass over the
//!   finished 3D layer (`look.wgsl`), reading two images of the same size
//!   made by the **ID pass**: the 3D layer's polygons drawn again, with the
//!   same cut-outs and depth rules, into a texture-ID target and a 1/z
//!   target ([`Renderer::encode_3d`] with its ID targets). The ID pass and
//!   the ink pass cost nothing when the look has no screen-space effect:
//!   nothing is allocated or drawn.
//!
//! **The ID target** is `Rg32Uint`. `x` is the polygon's texture ID, 0 where
//! nothing is drawn: for a texture, `2 + (key mod (2^23 - 2))` folded from
//! the 64-bit texture key (`TexInfo::key`, the one pack file names carry;
//! [`texture_id`]); for a flat colour polygon, `2^23 | colour` (15 bits,
//! [`flat_id`]). `y` is the polygon's tag word: bits 0-7 the outline weight
//! in sixteenths of the look's (16: as the look, 0: none), bits 8-15 the rim
//! light in 255ths, bit 16 set where the tag names an outline weight (the texture
//! is lined even against a coplanar neighbour), bit 17 set for a polygon the
//! game draws in its own order without depth (a sky, a ground quad running
//! to the horizon: where two of them meet the depths are not the surface's
//! and draw no line). Translucent (checkerboard, shadow) polygons, which the
//! game draws without depth, are not in it.

use crate::{Renderer, FORMAT};
use model2::look::Look;
use model2::raster::{tex_info, Placed, Poly, Sources};
use model2::video::View;
use std::collections::HashMap;
use std::sync::Arc;

/// The texture-ID target's format and the 1/z target's.
pub(crate) const ID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Uint;
pub(crate) const IZ_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

/// The live ID of the texture with key `key` (`TexInfo::key`): never 0 (no
/// polygon) and never in the flat-colour range (bit 23 set).
pub fn texture_id(key: u64) -> u32 {
    let folded = key ^ (key >> 24) ^ (key >> 48);
    2 + (folded % ((1 << 23) - 2)) as u32
}

/// The live ID of an untextured polygon of base colour `colour` (5 bits
/// each of red, green, blue).
pub fn flat_id(colour: [u32; 3]) -> u32 {
    1 << 23 | (colour[0] & 31 | (colour[1] & 31) << 5 | (colour[2] & 31) << 10)
}

/// A tag stem's key: `tex_<key>_WxH` (the image key) or `surf_<key>_WxH`
/// (the surface key), as `(is_surface, key)`.
fn stem_key(stem: &str) -> Option<(bool, u64)> {
    let (surface, rest) = match (stem.strip_prefix("tex_"), stem.strip_prefix("surf_")) {
        (Some(r), _) => (false, r),
        (_, Some(r)) => (true, r),
        _ => return None,
    };
    u64::from_str_radix(rest.get(..16)?, 16).ok().map(|k| (surface, k))
}

/// A look's tags by texture key and by surface key, for the per-polygon
/// lookup (`TexInfo::key`, `TexInfo::surface`).
#[derive(Default)]
pub(crate) struct LookTags {
    by_key: HashMap<u64, model2::look::Tag>,
    by_surface: HashMap<u64, model2::look::Tag>,
}

impl LookTags {
    pub fn new(look: &Look) -> Self {
        let mut t = LookTags::default();
        for (stem, tag) in &look.tags {
            match stem_key(stem) {
                Some((false, k)) => {
                    t.by_key.insert(k, tag.clone());
                }
                Some((true, k)) => {
                    t.by_surface.insert(k, tag.clone());
                }
                None => {}
            }
        }
        t
    }

    fn is_empty(&self) -> bool {
        self.by_key.is_empty() && self.by_surface.is_empty()
    }
}

/// What the look knows of one placed polygon.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PolyLook {
    /// The ID target's two words (see the module doc).
    pub id: u32,
    pub tag: u32,
    /// Self-lit: drawn at full brightness, unlit.
    pub emissive: bool,
    /// Toon steps for this polygon: 0 follow the look, 1 none, else the
    /// count (`raster.wgsl`, `poly_px`).
    pub toon: u8,
}

/// Whether the ID pass draws a placed polygon: all that are depth tested,
/// and the backgrounds that stand for surfaces (the sky, and a ground or
/// wall quad drawn in the game's order that spans a wide range of depth),
/// not the translucent shadows and checkerboards, which sit on other
/// surfaces and are not shapes of their own. A background is drawn as the
/// colour pass does, over what the earlier ones left and without depth.
pub(crate) fn in_id_pass(p: &Poly, pl: &Placed) -> bool {
    crate::aospike::GBUF_ALL.load(std::sync::atomic::Ordering::Relaxed)
        || !pl.background || matches!(model2::raster::background_reason(p), Some("sky" | "spread"))
}

/// The scene's typical distance (as `1/iz`, game units): where half of the
/// picture's polygon area, by the polygons' mean 1/z, is further away.
/// Skies, shadows and checkerboards don't count. The look's fade distances
/// are in multiples of it when they are left to the scene (`ink_params`).
pub fn depth_reference(polys: &[Poly], placed: &[Placed]) -> Option<f32> {
    let mut items: Vec<(f32, f32)> = placed
        .iter()
        .filter_map(|pl| {
            let p = &polys[pl.poly];
            if matches!(model2::raster::background_reason(p), Some("sky" | "shadow" | "checker")) {
                return None;
            }
            let n = pl.points.len();
            let area: f32 = (0..n)
                .map(|i| {
                    let (a, b) = (&pl.points[i], &pl.points[(i + 1) % n]);
                    a.x * b.y - b.x * a.y
                })
                .sum::<f32>()
                .abs()
                / 2.0;
            let iz = pl.points.iter().map(|v| v.iz).sum::<f32>() / n.max(1) as f32;
            (area > 0.0 && iz > 0.0).then_some((iz, area))
        })
        .collect();
    let total: f32 = items.iter().map(|i| i.1).sum();
    if items.is_empty() || total <= 0.0 {
        return None;
    }
    items.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut acc = 0.0;
    for (iz, area) in &items {
        acc += area;
        if acc >= total / 2.0 {
            return Some(1.0 / iz);
        }
    }
    None
}

/// One placed polygon's draw as the 3D pass made it, for the ID pass.
pub(crate) struct Span {
    pub vertices: std::ops::Range<u32>,
    pub window: usize,
    pub clip: (f32, f32, f32, f32),
    pub slot: u32,
    pub background: bool,
    /// In the ID pass (`in_id_pass`).
    pub in_id: bool,
    /// Last of its group (`Placed::group`).
    pub group_end: bool,
}

/// What the 3D pass keeps for the ID pass that follows it in the same
/// frame (`Renderer::encode_id`).
pub(crate) struct IdShare {
    pub slots: crate::Slots,
    pub polys: Vec<PolyLook>,
    pub spans: Vec<Span>,
    pub vertices: wgpu::Buffer,
    pub table: wgpu::Buffer,
    pub render: wgpu::Buffer,
    pub palette: wgpu::Buffer,
    pub variant: crate::Variant,
}

/// The ink pass: outlines, rim light and speed lines over the 3D layer.
pub(crate) struct Ink {
    layout: wgpu::BindGroupLayout,
    shader: wgpu::ShaderModule,
    pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
}

impl Ink {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("look"),
            source: wgpu::ShaderSource::Wgsl(include_str!("look.wgsl").into()),
        });
        let texture = |binding: u32, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("look"),
            entries: &[
                crate::uniform(0, wgpu::ShaderStages::FRAGMENT),
                texture(1, wgpu::TextureSampleType::Float { filterable: false }),
                texture(2, wgpu::TextureSampleType::Uint),
                texture(3, wgpu::TextureSampleType::Float { filterable: false }),
            ],
        });
        Ink { layout, shader, pipelines: Vec::new() }
    }

    fn pipeline(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        if let Some((_, p)) = self.pipelines.iter().find(|(f, _)| *f == format) {
            return p.clone();
        }
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("look"),
            bind_group_layouts: &[Some(&self.layout)],
            immediate_size: 0,
        });
        let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("look"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &self.shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.pipelines.push((format, p.clone()));
        p
    }

    /// Draw `layer` into `out` (`format`) with the look's lines and speed
    /// lines, from the ID and 1/z images.
    #[allow(clippy::too_many_arguments)]
    fn encode(
        &mut self,
        device: &wgpu::Device,
        enc: &mut wgpu::CommandEncoder,
        params: &[f32; 16],
        layer: &wgpu::TextureView,
        ids: &wgpu::TextureView,
        iz: &wgpu::TextureView,
        out: &wgpu::TextureView,
        format: wgpu::TextureFormat,
    ) {
        let bytes: Vec<u8> = params.iter().flat_map(|f| f.to_le_bytes()).collect();
        let buffer = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("look"),
                contents: &bytes,
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("look"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(layer) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(ids) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(iz) },
            ],
        });
        let pipeline = self.pipeline(device, format);
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("look"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: out,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// With `outline_fade_near` 0, lines are at full strength up to this many
/// times the scene's typical distance and gone at the second (a scene
/// spans three orders of magnitude of depth: a fixed distance in game
/// units fades a whole game's stands and keeps another's).
pub(crate) const FADE_NEAR_AUTO: f32 = 100.0;
pub(crate) const FADE_FAR_AUTO: f32 = 800.0;

/// The ink pass's uniform for `look`, its fade in distances on a `w` x `h` picture.
fn ink_params(look: &Look, w: u32, h: u32, typical: f32) -> [f32; 16] {
    let p = &look.params;
    let dist = |name: &str| p.num(name).max(1.0);
    // Fade by distance in game units, as 1/z; left to the scene (0), in
    // multiples of its typical distance (`depth_reference`).
    let (near_d, far_d) = if p.num("outline_fade_near") > 0.0 {
        (dist("outline_fade_near"), dist("outline_fade_far"))
    } else {
        (FADE_NEAR_AUTO * typical, FADE_FAR_AUTO * typical)
    };
    let near = near_d.max(f32::MIN_POSITIVE);
    let far = far_d.max(near * 1.001);
    let line_on = p.outline_on();
    [
        w as f32,
        h as f32,
        h as f32 / 1080.0,
        p.num("speed_lines"),
        if line_on { p.num("outline_weight") } else { 0.0 },
        if line_on { p.num("outline_opacity") } else { 0.0 },
        p.num("outline_depth"),
        p.num("outline_ids"),
        p.num("outline_r"),
        p.num("outline_g"),
        p.num("outline_b"),
        0.0,
        near,
        far,
        0.0,
        0.0,
    ]
}

/// The toon and halftone values for the 3D layer's shader (`raster.wgsl`
/// `Screen`: toon, half, ink; the toon's w, the ID entries' place, is set
/// by the caller): all zero for no look.
pub(crate) fn toon_params(look: Option<&Look>, out_height: f32) -> [f32; 12] {
    let Some(l) = look else { return [0.0; 12] };
    let p = &l.params;
    let steps = if p.toon_steps() >= 2 { p.toon_steps() as f32 } else { 0.0 };
    let half = if p.halftone_on() { p.num("halftone") } else { 0.0 };
    [
        steps,
        p.num("toon_floor"),
        p.num("toon_cut"),
        0.0,
        half,
        (p.num("halftone_pitch") * out_height / 1080.0).max(1.0),
        p.num("halftone_angle").to_radians(),
        0.0,
        p.num("halftone_r"),
        p.num("halftone_g"),
        p.num("halftone_b"),
        0.0,
    ]
}

impl Renderer {
    /// Draw with `look` (None: no look): ink outlines and the rest of
    /// `model2::look`, as the file says. Render only; changes at once and
    /// costs nothing while the look has no effect (no targets, no passes).
    /// Temporal accumulation starts again when what is drawn into it
    /// changes (toon lighting, emissive and per-texture toon).
    pub fn set_look(&mut self, look: Option<Arc<Look>>) {
        let draws = |l: &Option<Arc<Look>>| {
            l.as_ref().filter(|l| l.active()).map(|l| (l.params.toon_steps(), l.params.halftone_on(), l.tags.clone()))
        };
        // The numbers of toon and halftone are the shader's own; any
        // change of them reaches the history as a change of picture.
        if draws(&look) != draws(&self.look) || toon_changed(&look, &self.look) {
            self.temporal.reset();
        }
        self.look_tags = look.as_ref().map(|l| LookTags::new(l)).unwrap_or_default();
        self.depth_ref = None;
        self.look = look;
    }

    pub(crate) const ID_USAGE: wgpu::TextureUsages = wgpu::TextureUsages::RENDER_ATTACHMENT
        .union(wgpu::TextureUsages::TEXTURE_BINDING)
        .union(wgpu::TextureUsages::COPY_SRC);

    /// SPIKE: the last ID pass's target read back: per pixel (texture ID, tag); with
    /// `pick_blocks` the tag's bits 18+ are the object block number + 1 (0: none).
    pub fn read_ids(&mut self, w: u32, h: u32) -> Vec<[u32; 2]> {
        let t = self.kept_target("look ids", (w, h), ID_FORMAT, Self::ID_USAGE).0;
        let raw = self.read_back_bpp(self.device.create_command_encoder(&Default::default()), &t, w, h, 16);
        raw.chunks_exact(16)
            .map(|c| [u32::from_le_bytes(c[0..4].try_into().unwrap()), u32::from_le_bytes(c[4..8].try_into().unwrap())])
            .collect()
    }

    /// SPIKE (authoring): a `w`x`h` block at (`x`,`y`) of the last ID pass's target (a
    /// frame of `size`), per pixel (texture ID, tag): what pointing at the picture needs.
    pub fn read_ids_rect(&mut self, size: (u32, u32), x: u32, y: u32, w: u32, h: u32) -> Vec<[u32; 2]> {
        let t = self.kept_target("look ids", size, ID_FORMAT, Self::ID_USAGE).0;
        let row = (w * 16).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("id pick"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &t,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        let data = slice.get_mapped_range().expect("mapped");
        let mut out = Vec::new();
        for r in 0..h as usize {
            for c in 0..w as usize {
                let o = r * row as usize + c * 16;
                out.push([
                    u32::from_le_bytes(data[o..o + 4].try_into().unwrap()),
                    u32::from_le_bytes(data[o + 4..o + 8].try_into().unwrap()),
                ]);
            }
        }
        out
    }

    /// The look in use.
    pub fn look(&self) -> Option<&Arc<Look>> {
        self.look.as_ref()
    }

    /// Resolve each placed polygon's texture ID and tag for the look's
    /// passes: only what is needed (`ids`: the ID pass; tags: emissive and
    /// per-texture toon), nothing when neither is.
    pub(crate) fn look_polys(
        &mut self,
        polys: &[Poly],
        placed: &[Placed],
        src: &Sources,
        look: &Look,
        ids: bool,
        render_version: Option<u64>,
    ) -> Vec<PolyLook> {
        if !ids && self.look_tags.is_empty() {
            return Vec::new();
        }
        // The pack's slots began the frame's texture keys already.
        if self.pack.is_none() {
            self.keys.begin(render_version);
        }
        let rim = look.params.num("rim");
        placed
            .iter()
            .map(|pl| {
                let p = &polys[pl.poly];
                let info = p.texture.as_ref().and_then(|t| tex_info(p, t, src, &mut self.keys));
                let tag = info.as_ref().and_then(|i| {
                    self.look_tags.by_key.get(&i.key).or_else(|| self.look_tags.by_surface.get(&i.surface))
                });
                let weight = tag.and_then(|t| t.outline).unwrap_or(1.0);
                let rim = tag.and_then(|t| t.rim).unwrap_or(rim);
                PolyLook {
                    id: match &info {
                        Some(i) => texture_id(i.key),
                        None => flat_id(p.colour),
                    },
                    tag: (if self.pick_blocks { (p.block as u32 + 1) << 18 } else { 0 })
                        | (weight * 16.0f32).round().clamp(0.0, 255.0) as u32
                        | ((rim * 255.0).round().clamp(0.0, 255.0) as u32) << 8
                        | (tag.is_some_and(|t| t.outline.is_some_and(|w| w > 0.0)) as u32) << 16
                        | (pl.background as u32) << 17,
                    emissive: tag.is_some_and(|t| t.emissive),
                    toon: match tag.and_then(|t| t.toon) {
                        None => 0,
                        Some(0) => 1,
                        Some(n) => n.min(15) as u8,
                    },
                }
            })
            .collect()
    }

    /// The ink pass over `layer` (the finished 3D layer): the ID pass into
    /// the ID and 1/z targets, then the lines over the layer into a kept
    /// picture, which is returned for compose to use in its place.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_ink(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        polys: &[Poly],
        placed: &[Placed],
        src: &Sources,
        view: &View,
        look: &Look,
        layer: &wgpu::TextureView,
    ) -> wgpu::TextureView {
        let (w, h) = (view.width as u32, view.height as u32);
        let usage = Self::ID_USAGE;
        let format = if self.quality.temporal > 0 { crate::temporal::HISTORY_FORMAT } else { FORMAT };
        let ids = self.kept_target("look ids", (w, h), ID_FORMAT, usage).0.create_view(&Default::default());
        let iz = self.kept_target("look iz", (w, h), IZ_FORMAT, usage).0.create_view(&Default::default());
        let out = self.kept_target("3d look", (w, h), format, usage).0.create_view(&Default::default());
        match self.id_share.take().filter(|s| s.spans.len() == placed.len()) {
            Some(share) => self.encode_id(enc, placed, view, &ids, &iz, share),
            // The 3D pass was temporal or had materials: the ID pass
            // prepares its polygons itself.
            None => self.encode_3d(enc, polys, placed, src, view, &ids, None, Some(&iz)),
        }
        // The scene's typical distance, steadied over frames (a camera cut
        // starts it again).
        let now = depth_reference(polys, placed).unwrap_or(100.0);
        let typical = match self.depth_ref {
            Some(prev) if (0.5..2.0).contains(&(now / prev)) => (prev.ln() * 0.8 + now.ln() * 0.2).exp(),
            _ => now,
        };
        self.depth_ref = Some(typical);
        let params = ink_params(look, w, h, typical);
        self.ink.encode(&self.device, enc, &params, layer, &ids, &iz, &out, format);
        out
    }

    /// The ID pass from what the 3D pass prepared: its vertices, polygon
    /// table, RAM buffers and pack slots are used as they are, and only the
    /// 1/z points and the polygons' IDs are made (`encode_3d` with its ID
    /// targets prepares all of it again, at 2-3x the CPU time).
    fn encode_id(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        placed: &[Placed],
        view: &View,
        ids: &wgpu::TextureView,
        iz: &wgpu::TextureView,
        sh: IdShare,
    ) {
        let t0 = self.tick();
        let t_look = self.tick();
        let (w, h) = (view.width as u32, view.height as u32);
        // The outline buffer as `encode_3d` lays it out for polygons that
        // are not lit: a directory of (first, count, 0, 0) per polygon, the
        // points (x, y, 1/z, 0), then each polygon's ID words.
        let n = placed.len();
        let points: usize = placed.iter().map(|p| p.points.len()).sum();
        let mut words: Vec<u8> = Vec::with_capacity(16 * (2 * n + points));
        let mut first = n as u32;
        for pl in placed {
            for v in [first, pl.points.len() as u32, 0, 0] {
                words.extend_from_slice(&v.to_le_bytes());
            }
            first += pl.points.len() as u32;
        }
        for pl in placed {
            for v in &pl.points {
                for f in [v.x, v.y, v.iz, 0.0] {
                    words.extend_from_slice(&f.to_le_bytes());
                }
            }
        }
        let id_base = (words.len() / 16) as u32;
        for l in &sh.polys {
            for v in [l.id, l.tag, 0, 0] {
                words.extend_from_slice(&v.to_le_bytes());
            }
        }
        if words.is_empty() {
            words.resize(16, 0);
        }
        let mut screen: Vec<u8> =
            [view.width as f32, view.height as f32, view.sx, view.sy].iter().flat_map(|f| f.to_le_bytes()).collect();
        let mut tail = [0.0f32; 12];
        tail[3] = f32::from_bits(id_base);
        screen.extend(tail.iter().flat_map(|f| f.to_le_bytes()));
        let d = &self.device;
        let buffer = |label, contents: &[u8], usage| {
            wgpu::util::DeviceExt::create_buffer_init(
                d,
                &wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage },
            )
        };
        let outlines = buffer("outlines id", &words, wgpu::BufferUsages::STORAGE);
        let screen = buffer("screen id", &screen, wgpu::BufferUsages::UNIFORM);
        let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("raster id"),
            layout: &self.raster_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: screen.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: sh.table.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: sh.render.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: sh.palette.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: outlines.as_entire_binding() },
            ],
        });
        let depth = self.kept_attachment("look depth", w, h, 1, crate::DEPTH_FORMAT).create_view(&Default::default());
        let test = self.id_pipeline(crate::Mode::Test, sh.variant);
        let write = self.id_pipeline(crate::Mode::DepthWrite, sh.variant);
        let background = self.id_pipeline(crate::Mode::Background, sh.variant);
        let clear = self.id_pipeline(crate::Mode::Clear, sh.variant);
        let dummy = (sh.variant != crate::Variant::Original).then(|| self.pack_dummy(sh.variant));
        // Runs of polygons in one window with one pack image: the 3D
        // pass's grouping, without its translucent polygons, each group's
        // depth written once all its members are drawn.
        struct Run {
            vertices: std::ops::Range<u32>,
            window: usize,
            clip: (f32, f32, f32, f32),
            slot: u32,
            mode: crate::Mode,
        }
        let mut runs: Vec<Run> = Vec::new();
        let push = |runs: &mut Vec<Run>, r: Run| match runs.last_mut() {
            Some(l)
                if l.window == r.window
                    && l.mode == r.mode
                    && l.vertices.end == r.vertices.start
                    && (l.slot == r.slot || l.slot == crate::NO_SLOT || r.slot == crate::NO_SLOT) =>
            {
                l.vertices.end = r.vertices.end;
                if l.slot == crate::NO_SLOT {
                    l.slot = r.slot;
                }
            }
            _ => runs.push(r),
        };
        let mut group: Vec<Run> = Vec::new();
        for s in &sh.spans {
            let run = |mode| Run { vertices: s.vertices.clone(), window: s.window, clip: s.clip, slot: s.slot, mode };
            if !s.background {
                push(&mut runs, run(crate::Mode::Test));
                group.push(run(crate::Mode::DepthWrite));
            } else if s.in_id {
                push(&mut runs, run(crate::Mode::Background));
            }
            if s.group_end {
                for r in group.drain(..) {
                    push(&mut runs, r);
                }
            }
        }
        self.pack_stats.draws += runs.len() as u32;
        self.lap(t_look, |p| &mut p.look_id);
        self.lap(t0, |p| &mut p.resources3d);
        let t0 = self.tick();
        let clear_ops =
            wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store };
        let colours = [ids, iz].map(|view| {
            Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: clear_ops })
        });
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("look id"),
            color_attachments: &colours,
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Discard }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_bind_group(0, &bind, &[]);
        if let Some(dummy) = &dummy {
            pass.set_bind_group(1, dummy, &[]);
        }
        pass.set_vertex_buffer(0, sh.vertices.slice(..));
        let mut bound = crate::NO_SLOT;
        for (k, run) in runs.iter().enumerate() {
            // Each window starts with an empty depth buffer where it draws.
            if k > 0 && runs[k - 1].window != run.window {
                let (x0, y0) = (run.clip.0.floor().max(0.0) as u32, run.clip.1.floor().max(0.0) as u32);
                let (x1, y1) = ((run.clip.2.ceil().max(0.0) as u32).min(w), (run.clip.3.ceil().max(0.0) as u32).min(h));
                if x1 > x0 && y1 > y0 {
                    pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
                    pass.set_pipeline(&clear);
                    pass.draw(0..3, 0..1);
                    pass.set_scissor_rect(0, 0, w, h);
                }
            }
            pass.set_pipeline(match run.mode {
                crate::Mode::DepthWrite => &write,
                crate::Mode::Background => &background,
                _ => &test,
            });
            if run.slot != crate::NO_SLOT && run.slot != bound {
                bound = run.slot;
                pass.set_bind_group(1, &sh.slots.groups[run.slot as usize], &[]);
            }
            pass.draw(run.vertices.clone(), 0..1);
        }
        drop(pass);
        self.lap(t0, |p| &mut p.pass3d);
    }
}

/// Whether the numbers the 3D layer's shader reads changed (the look's
/// toon and halftone parameters).
fn toon_changed(a: &Option<Arc<Look>>, b: &Option<Arc<Look>>) -> bool {
    toon_params(a.as_deref(), 1080.0) != toon_params(b.as_deref(), 1080.0)
}

#[cfg(test)]
#[path = "look_tests.rs"]
mod tests;
