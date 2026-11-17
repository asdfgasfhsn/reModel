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
//! light in 255ths. Translucent (checkerboard, shadow) polygons, which the
//! game draws without depth, are not in it.

use crate::{Renderer, FORMAT};
use model2::look::Look;
use model2::raster::{tex_info, Placed, Poly, Sources};
use model2::video::View;
use std::collections::HashMap;
use std::sync::Arc;

/// The texture-ID target's format and the 1/z target's.
pub(crate) const ID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Uint;
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

/// The ink pass's uniform for `look` on a `w` x `h` picture.
fn ink_params(look: &Look, w: u32, h: u32) -> [f32; 16] {
    let p = &look.params;
    let dist = |name: &str| p.num(name).max(1.0);
    // Fade by distance in game units, as 1/z.
    let near = 1.0 / dist("outline_fade_near");
    let far = (1.0f32 / dist("outline_fade_far")).min(near * 0.999);
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
        self.look = look;
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
        self.keys.begin(render_version);
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
                    tag: (weight * 16.0f32).round().clamp(0.0, 255.0) as u32
                        | ((rim * 255.0).round().clamp(0.0, 255.0) as u32) << 8,
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
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let format = if self.quality.temporal > 0 { crate::temporal::HISTORY_FORMAT } else { FORMAT };
        let ids = self.kept_target("look ids", (w, h), ID_FORMAT, usage).0.create_view(&Default::default());
        let iz = self.kept_target("look iz", (w, h), IZ_FORMAT, usage).0.create_view(&Default::default());
        let out = self.kept_target("3d look", (w, h), format, usage).0.create_view(&Default::default());
        self.encode_3d(enc, polys, placed, src, view, &ids, None, Some(&iz));
        let params = ink_params(look, w, h);
        self.ink.encode(&self.device, enc, &params, layer, &ids, &iz, &out, format);
        out
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
