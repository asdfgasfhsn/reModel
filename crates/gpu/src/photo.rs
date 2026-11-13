//! Photo mode's picture (docs/photo-mode.md): a board frame drawn from
//! geometry the caller posed (`model2::photo`), as one image or the sum of
//! several (motion blur), its depth as 1/z, then depth of field and a
//! colour look over it, the HUD laid back on top if wanted, and the
//! result brought down to the output's size (supersampling) with the
//! film's touches: chromatic aberration, grain, letterbox.
//!
//! The frame itself is drawn by the renderer's own passes
//! ([`Renderer::present_board`]) into images of the photo's size, with the
//! HUD and, if asked, the 2D background left out ([`Renderer::photo`]
//! sets `photo_hide` for them). Nothing here changes how other frames are
//! drawn.

use crate::temporal::AUX_FORMAT;
use crate::{BoardFrame, Renderer, FORMAT};
use model2::raster::{Placed, Poly};
use model2::video::View;
use std::collections::HashMap;
use std::sync::Arc;
use wgpu::util::DeviceExt;

const IZ_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
const Z_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Depth of field's tiles (`photo.wgsl`'s `TILE`).
const TILE: u32 = 16;

/// The look laid over the picture. The default changes nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// Depth of field: the distance in focus (the geometrizer's view
    /// units, along the view axis) and the blur of something infinitely
    /// far, as a fraction of the picture's height (0: none); bokeh, how
    /// much bright parts stand out in the blur (0 to 1).
    pub focus: f32,
    pub aperture: f32,
    pub bokeh: f32,
    /// Exposure in stops, contrast and saturation (1: as is), warmth and
    /// tint (-1 to 1, 0: as is), vignette (0 to 1).
    pub exposure: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub warmth: f32,
    pub tint: f32,
    pub vignette: f32,
    /// Film grain (0 to 1), chromatic aberration (0 to 1), and a letterbox
    /// opening's width over height (0: none).
    pub grain: f32,
    pub aberration: f32,
    pub letterbox: f32,
    /// Seeds the grain and the blur's sampling.
    pub seed: u32,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            focus: 1.0,
            aperture: 0.0,
            bokeh: 0.5,
            exposure: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            warmth: 0.0,
            tint: 0.0,
            vignette: 0.0,
            grain: 0.0,
            aberration: 0.0,
            letterbox: 0.0,
            seed: 1,
        }
    }
}

/// One image of the photo: geometry for the frame (a raster stream and
/// its texture point RAM) and its lighting side channel, if it has one.
pub struct Shot<'a> {
    pub stream: &'a [u32],
    pub points: &'a [u16],
    pub light: Option<Arc<model2::light::Frame>>,
}

/// What [`Renderer::photo`] draws.
pub struct Request<'a> {
    /// The picture at the size it is drawn (supersampled: larger than the
    /// output).
    pub view: View,
    /// The images, averaged (motion blur); the last gives the depth.
    pub shots: &'a [Shot<'a>],
    /// Lay the frame's own HUD (tile HUD and HUD windows, as the game drew
    /// them, whatever the camera) over the picture.
    pub hud: bool,
    /// Leave the 2D background out (the backdrop colour instead).
    pub hide_base: bool,
    pub look: Look,
    /// Bloom's chain this many levels longer than in play (the picture is
    /// that many times twice the size), so its glow is as wide.
    pub bloom_levels: u32,
    /// The near plane for the posed geometry (view space z; 0: the game's).
    pub near: f32,
}

/// The passes' pipelines and the last picture's depth (for picking a
/// focus distance).
pub(crate) struct Photo {
    depth: wgpu::RenderPipeline,
    depth_layout: wgpu::BindGroupLayout,
    accum: [wgpu::RenderPipeline; 2],
    accum_layout: wgpu::BindGroupLayout,
    tile: wgpu::RenderPipeline,
    tile_layout: wgpu::BindGroupLayout,
    post: [wgpu::RenderPipeline; 2],
    post_layout: wgpu::BindGroupLayout,
    finish: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    finish_layout: wgpu::BindGroupLayout,
    finish_module: wgpu::ShaderModule,
    /// The last picture's 1/z.
    pub iz: Option<wgpu::Texture>,
}

fn module(device: &wgpu::Device, part: usize) -> wgpu::ShaderModule {
    let src = include_str!("photo.wgsl");
    let parts: Vec<&str> = src.split("//// split\n").collect();
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("photo"),
        source: wgpu::ShaderSource::Wgsl(format!("{}{}", parts[0], parts[part]).into()),
    })
}

fn tex_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    crate::texture(binding)
}

#[allow(clippy::too_many_arguments)]
fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    module: &wgpu::ShaderModule,
    vs: &str,
    fs: &str,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
    depth: bool,
) -> wgpu::RenderPipeline {
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("photo"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    let attributes = wgpu::vertex_attr_array![0 => Float32x3];
    let buffers = [Some(wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &attributes,
    })];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("photo"),
        layout: Some(&pl),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(vs),
            compilation_options: Default::default(),
            buffers: if depth { &buffers } else { &[] },
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: depth.then(|| wgpu::DepthStencilState {
            format: Z_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Greater),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fs),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

impl Photo {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = |label: &str, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries })
        };
        let all = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let depth_layout = layout("photo depth", &[crate::uniform(0, all)]);
        let accum_layout = layout("photo accum", &[crate::uniform(0, all), tex_entry(1)]);
        let tile_layout = layout("photo tile", &[crate::uniform(0, all), tex_entry(2)]);
        let post_layout = layout(
            "photo post",
            &[crate::uniform(0, all), tex_entry(1), tex_entry(2), tex_entry(3), tex_entry(4), tex_entry(5)],
        );
        let finish_layout = layout("photo finish", &[crate::uniform(0, all), tex_entry(1)]);
        let (m_depth, m_accum, m_post) = (module(device, 1), module(device, 2), module(device, 3));
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let add = Some(wgpu::BlendState { color: add, alpha: add });
        let both = |f: &dyn Fn(wgpu::TextureFormat) -> wgpu::RenderPipeline| [f(AUX_FORMAT), f(FORMAT)];
        Photo {
            depth: pipeline(device, &depth_layout, &m_depth, "vs_depth", "fs_depth", IZ_FORMAT, None, true),
            accum: both(&|f| pipeline(device, &accum_layout, &m_accum, "vs_full", "fs_accum", f, add, false)),
            tile: pipeline(device, &tile_layout, &m_post, "vs_full", "fs_tile", IZ_FORMAT, None, false),
            post: both(&|f| pipeline(device, &post_layout, &m_post, "vs_full", "fs_post", f, None, false)),
            finish: HashMap::new(),
            finish_module: module(device, 4),
            depth_layout,
            accum_layout,
            tile_layout,
            post_layout,
            finish_layout,
            iz: None,
        }
    }
}

/// `placed`'s polygons (of `polys`) as triangles of (x, y, 1/z) in output
/// pixels, each clipped to its window: what the depth pass draws. See-
/// through (checkerboard) polygons and shadows have no depth of their own.
pub(crate) fn depth_triangles(polys: &[Poly], placed: &[Placed]) -> Vec<f32> {
    let mut out = Vec::new();
    for p in placed {
        let poly = &polys[p.poly];
        if poly.checker || poly.shadow || p.points.len() < 3 {
            continue;
        }
        let mut pts: Vec<[f32; 3]> = p.points.iter().map(|v| [v.x, v.y, v.iz]).collect();
        let (x0, y0, x1, y1) = p.clip;
        for (axis, edge, keep_above) in [(0, x0, true), (0, x1, false), (1, y0, true), (1, y1, false)] {
            pts = clip(&pts, axis, edge, keep_above);
            if pts.len() < 3 {
                break;
            }
        }
        for i in 1..pts.len().saturating_sub(1) {
            for v in [pts[0], pts[i], pts[i + 1]] {
                out.extend(v);
            }
        }
    }
    out
}

/// One Sutherland-Hodgman step: the part of the polygon with coordinate
/// `axis` above (or below) `edge`, 1/z interpolated linearly (it is linear
/// in screen space).
fn clip(pts: &[[f32; 3]], axis: usize, edge: f32, keep_above: bool) -> Vec<[f32; 3]> {
    let inside = |p: &[f32; 3]| if keep_above { p[axis] >= edge } else { p[axis] <= edge };
    let mut out = Vec::with_capacity(pts.len() + 2);
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        if inside(&a) {
            out.push(a);
        }
        if inside(&a) != inside(&b) {
            let t = (edge - a[axis]) / (b[axis] - a[axis]);
            out.push(std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t));
        }
    }
    out
}

fn params(values: &[[u8; 4]]) -> Vec<u8> {
    values.iter().flatten().copied().collect()
}

impl Renderer {
    fn photo_texture(&self, label: &str, w: u32, h: u32, format: wgpu::TextureFormat) -> wgpu::Texture {
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC;
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    }

    /// The photo pass's depth: called from `encode_frame` when a photo's
    /// depth is wanted (`photo_depth`), with the frame's polygons.
    pub(crate) fn encode_photo_depth(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        polys: &[Poly],
        placed: &[Placed],
        targets: &(wgpu::TextureView, wgpu::TextureView),
        size: (u32, u32),
    ) {
        let Some(photo) = &self.photo else { return };
        let verts = depth_triangles(polys, placed);
        let uniform = params(&[size.0 as f32, size.1 as f32, 0.0, 0.0].map(f32::to_le_bytes));
        let uniform = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("photo depth params"),
            contents: &uniform,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("photo depth"),
            layout: &photo.depth_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
        });
        let vbuf = (!verts.is_empty()).then(|| {
            let bytes: Vec<u8> = verts.iter().flat_map(|f| f.to_le_bytes()).collect();
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("photo depth vertices"),
                contents: &bytes,
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("photo depth"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &targets.0,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &targets.1,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Discard }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        if let Some(vbuf) = &vbuf {
            pass.set_pipeline(&photo.depth);
            pass.set_bind_group(0, &bind, &[]);
            pass.set_vertex_buffer(0, vbuf.slice(..));
            pass.draw(0..(verts.len() / 3) as u32, 0..1);
        }
    }

    /// Draw a photo of `f` (a board frame, with `req`'s geometry in place
    /// of its own) into `target` (`format`) at `origin`, `out` pixels:
    /// the images at `req.view`'s size, their depth, depth of field, the
    /// look and the HUD there, then brought down to `out`. Submits.
    pub fn photo(
        &mut self,
        f: &BoardFrame,
        req: &Request,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        origin: (u32, u32),
        out: (u32, u32),
    ) {
        if req.shots.is_empty() {
            return;
        }
        let (w, h) = (req.view.width as u32, req.view.height as u32);
        if self.photo.is_none() {
            self.photo = Some(Box::new(Photo::new(&self.device)));
        }
        // Without 16-bit float targets, 8 bits.
        let (hdr, wide) = if self.temporal.supported { (0, AUX_FORMAT) } else { (1, FORMAT) };
        let view_of = |t: &wgpu::Texture| t.create_view(&Default::default());
        let image = self.photo_texture("photo image", w, h, FORMAT);
        let iz = self.photo_texture("photo depth", w, h, IZ_FORMAT);
        let zbuf = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("photo depth test"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Z_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let sum = (req.shots.len() > 1).then(|| self.photo_texture("photo sum", w, h, wide));
        let saved = (self.photo_hide, self.bloom_extra);
        self.photo_hide = (true, req.hide_base);
        self.bloom_extra = req.bloom_levels;
        self.photo_near = req.near;
        let image_view = view_of(&image);
        let n = req.shots.len();
        for (i, shot) in req.shots.iter().enumerate() {
            self.set_light_frame(shot.light.clone());
            if i + 1 == n {
                self.photo_depth = Some((view_of(&iz), view_of(&zbuf)));
            }
            self.present_board(f, Some((shot.stream, shot.points)), None, &req.view, &image_view, FORMAT, (0, 0));
            if let Some(sum) = &sum {
                let photo = self.photo.as_ref().unwrap();
                let u = params(&[1.0 / n as f32, 0.0, 0.0, 0.0].map(f32::to_le_bytes));
                let u = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("photo accum params"),
                    contents: &u,
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("photo accum"),
                    layout: &photo.accum_layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: u.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&image_view) },
                    ],
                });
                let mut enc = self.device.create_command_encoder(&Default::default());
                {
                    let sum_view = view_of(sum);
                    let load = if i == 0 { wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT) } else { wgpu::LoadOp::Load };
                    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("photo accum"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &sum_view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
                        })],
                        ..Default::default()
                    });
                    pass.set_pipeline(&photo.accum[hdr]);
                    pass.set_bind_group(0, &bind, &[]);
                    pass.draw(0..3, 0..1);
                }
                self.queue.submit([enc.finish()]);
            }
        }
        self.photo_near = 0.0;
        // The HUD as the game drew it: where its picture with the HUD
        // differs from the one without.
        let hud = req.hud.then(|| {
            let with = self.photo_texture("photo hud", w, h, FORMAT);
            let without = self.photo_texture("photo hudless", w, h, FORMAT);
            for (t, hide) in [(&with, false), (&without, true)] {
                self.photo_hide = (hide, req.hide_base);
                self.present_board(f, None, None, &req.view, &view_of(t), FORMAT, (0, 0));
            }
            (with, without)
        });
        (self.photo_hide, self.bloom_extra) = saved;
        let photo = self.photo.as_ref().unwrap();
        let look = &req.look;
        let tiles = self.photo_texture("photo tiles", w.div_ceil(TILE), h.div_ceil(TILE), IZ_FORMAT);
        let post = self.photo_texture("photo post", w, h, wide);
        let scene = sum.as_ref().map_or_else(|| image_view.clone(), view_of);
        let (hud_view, hudless_view) = match &hud {
            Some((a, b)) => (view_of(a), view_of(b)),
            None => (image_view.clone(), image_view.clone()),
        };
        let fl = f32::to_le_bytes;
        let ul = u32::to_le_bytes;
        let post_params = params(&[
            fl(w as f32),
            fl(h as f32),
            fl(1.0 / look.focus.max(1e-6)),
            fl(look.aperture * h as f32),
            fl((look.aperture * 2.5).min(0.06) * h as f32),
            fl(look.bokeh),
            fl(2f32.powf(look.exposure)),
            fl(look.contrast),
            fl(look.saturation),
            fl(look.warmth),
            fl(look.tint),
            fl(look.vignette),
            ul(req.hud as u32),
            ul(look.seed),
            fl(0.0),
            fl(0.0),
        ]);
        let post_params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("photo post params"),
            contents: &post_params,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let iz_view = view_of(&iz);
        let tiles_view = view_of(&tiles);
        let tile_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("photo tile"),
            layout: &photo.tile_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: post_params.as_entire_binding() },
                tex(2, &iz_view),
            ],
        });
        let post_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("photo post"),
            layout: &photo.post_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: post_params.as_entire_binding() },
                tex(1, &scene),
                tex(2, &iz_view),
                tex(3, &tiles_view),
                tex(4, &hud_view),
                tex(5, &hudless_view),
            ],
        });
        let n_side = (w as f32 / out.0.max(1) as f32).ceil().clamp(1.0, 8.0) as u32;
        let finish_params = params(&[
            fl(w as f32),
            fl(h as f32),
            fl(out.0 as f32),
            fl(out.1 as f32),
            fl(origin.0 as f32),
            fl(origin.1 as f32),
            fl(look.letterbox),
            fl(look.grain),
            fl(look.aberration),
            ul(n_side),
            ul(look.seed),
            fl(0.0),
        ]);
        let finish_params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("photo finish params"),
            contents: &finish_params,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let post_view = view_of(&post);
        let finish_bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("photo finish"),
            layout: &photo.finish_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: finish_params.as_entire_binding() },
                tex(1, &post_view),
            ],
        });
        let photo = self.photo.as_mut().unwrap();
        let finish = photo
            .finish
            .entry(format)
            .or_insert_with(|| {
                pipeline(
                    &self.device,
                    &photo.finish_layout,
                    &photo.finish_module,
                    "vs_full",
                    "fs_finish",
                    format,
                    None,
                    false,
                )
            })
            .clone();
        let photo = self.photo.as_ref().unwrap();
        let mut enc = self.device.create_command_encoder(&Default::default());
        let full = |enc: &mut wgpu::CommandEncoder,
                    view: &wgpu::TextureView,
                    pipe: &wgpu::RenderPipeline,
                    bind: &wgpu::BindGroup,
                    clear: bool,
                    viewport: Option<(u32, u32, u32, u32)>| {
            let load = if clear { wgpu::LoadOp::Clear(wgpu::Color::BLACK) } else { wgpu::LoadOp::Load };
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("photo"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            if let Some((x, y, w, h)) = viewport {
                pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
            }
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
        };
        full(&mut enc, &tiles_view, &photo.tile, &tile_bind, true, None);
        full(&mut enc, &post_view, &photo.post[hdr], &post_bind, true, None);
        full(&mut enc, target, &finish, &finish_bind, true, Some((origin.0, origin.1, out.0, out.1)));
        self.queue.submit([enc.finish()]);
        self.photo.as_mut().unwrap().iz = Some(iz);
    }

    /// The last photo's 1/z at pixel (`x`, `y`) of its supersampled
    /// picture, if one was drawn there (0: nothing, or infinitely far).
    /// Blocks: native only.
    pub fn photo_iz_at(&self, x: u32, y: u32) -> Option<f32> {
        let iz = self.photo.as_ref()?.iz.as_ref()?;
        if x >= iz.width() || y >= iz.height() {
            return None;
        }
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("photo pick"),
            size: 256,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: iz,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256), rows_per_image: None },
            },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        let data = slice.get_mapped_range().ok()?;
        Some(f32::from_le_bytes([data[0], data[1], data[2], data[3]]))
    }
}

fn tex(binding: u32, v: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(v) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipping_keeps_the_inside_with_its_depth() {
        let tri = [[0.0, 0.0, 1.0], [10.0, 0.0, 2.0], [0.0, 10.0, 3.0]];
        let c = clip(&tri, 0, 5.0, false);
        assert_eq!(c.len(), 4);
        assert!(c.iter().all(|p| p[0] <= 5.0));
        // On the cut edge from (0,0) to (10,0), 1/z halfway between.
        assert!(c.iter().any(|p| p[0] == 5.0 && p[1] == 0.0 && (p[2] - 1.5).abs() < 1e-6));
        assert!(clip(&tri, 1, 20.0, true).is_empty());
    }
}
