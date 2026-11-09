//! Model 2 3D rasteriser on the GPU, through wgpu (Vulkan on Linux and
//! Android, Metal on macOS and iOS, DirectX 12 on Windows, WebGPU in
//! browsers, OpenGL as a fallback).
//!
//! It draws the same placed polygons as the software rasteriser
//! (`model2::raster::place`) and shades them with a port of its shading,
//! reading texels, luma tables and colour translation from the game's
//! video RAM. The software rasteriser stays the reference: `remodel
//! gpu-check` draws captured frames both ways and compares them.
//!
//! Polygons are drawn in order in a single pass, by default with no depth
//! buffer: GPUs keep primitive order for overlapping pixels, which gives the
//! painter's algorithm the software renderer uses. [`Quality`] adds
//! multisampling and a depth buffer. [`Renderer::present`] then blends the
//! 2D tile layers around the 3D layer into the output.

use model2::raster::{Placed, Poly, Sources};
use model2::video::{Layers2d, View, HEIGHT, WIDTH};
use std::collections::HashMap;
use wgpu::util::DeviceExt;

/// Image quality options for the 3D layer (docs/enhancements.md). The
/// default is the original look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quality {
    /// Samples per pixel: 1 (off), 4, or 8 where the GPU has it
    /// ([`max_msaa`]). Edges blend by the fraction of the pixel covered.
    pub msaa: u32,
    /// Depth buffer from 1/z, per window (`model2::raster::depth_value`).
    pub depth: bool,
}

impl Default for Quality {
    fn default() -> Self {
        Quality { msaa: 1, depth: false }
    }
}

/// The depth format; 32-bit float keeps the precision of
/// `model2::raster::depth_value`.
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// How a draw of polygons is set up in the raster pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Mode {
    /// No depth buffer.
    Plain,
    /// Depth tested (a polygon wins where its depth is at least what is
    /// there) and written.
    Depth,
    /// Depth tested but not written: translucent polygons, and the colour
    /// of a group of polygons (see `Placed::group`).
    Test,
    /// Depth written where tested, no colour: a group's depth, after its
    /// colour.
    DepthWrite,
    /// With a depth buffer in the pass, but neither tested nor written:
    /// polygons the game sorted behind everything.
    Background,
    /// Empties the depth buffer within the scissor rectangle.
    Clear,
}

/// Samples per pixel the GPU can multisample the 3D layer with, at most 8:
/// 4 everywhere (WebGPU's only count), 8 where the adapter reports it for
/// both the colour and depth formats. Needs the device created with
/// [`features`].
pub fn max_msaa(adapter: &wgpu::Adapter) -> u32 {
    let ok = |f: wgpu::TextureFormat, n| adapter.get_texture_format_features(f).flags.sample_count_supported(n);
    if ok(FORMAT, 8) && ok(DEPTH_FORMAT, 8) {
        8
    } else {
        4
    }
}

/// Device features to request: the adapter's per-format sample counts
/// (8x multisampling), where it has them.
pub fn features(adapter: &wgpu::Adapter) -> wgpu::Features {
    adapter.features() & wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    raster_shader: wgpu::ShaderModule,
    raster_pipeline_layout: wgpu::PipelineLayout,
    /// Raster pipelines by sample count and mode, made when first needed.
    raster: HashMap<(u32, Mode), wgpu::RenderPipeline>,
    quality: Quality,
    max_msaa: u32,
    raster_layout: wgpu::BindGroupLayout,
    compose_shader: wgpu::ShaderModule,
    compose_layout: wgpu::BindGroupLayout,
    /// The compose pipeline for the last output format used.
    compose: Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    adapter: String,
    /// Render RAM and palette RAM on the GPU, with the versions uploaded.
    ram: [Option<(Option<u64>, wgpu::Buffer)>; 2],
    /// Versions for the next frame's RAM (see `set_ram_versions`).
    versions: Option<(u64, u64)>,
    /// Textures kept between frames, by label.
    textures: Vec<(&'static str, wgpu::Texture)>,
}

/// Bytes per vertex: x, y, 1/z, u/z, v/z, polygon index.
const VERTEX_BYTES: u64 = 24;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn storage(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// `bytes` padded to whole u32 words (storage buffers are read as u32).
fn words(bytes: &[u8]) -> Vec<u8> {
    let mut v = bytes.to_vec();
    v.resize(bytes.len().div_ceil(4).max(1) * 4, 0);
    v
}

fn le(values: impl IntoIterator<Item = [u8; 4]>) -> Vec<u8> {
    values.into_iter().flatten().collect()
}

/// The raster shader with the depth buffer's constants filled in.
fn raster_source() -> String {
    include_str!("raster.wgsl")
        .replace("@DEPTH_BIAS@", &format!("{:?}", model2::raster::DEPTH_BIAS))
        .replace("@DEPTH_SCALE@", &format!("{:?}", model2::raster::DEPTH_SCALE))
}

/// Limits to ask for: the portable defaults, with storage buffers (render
/// RAM is 4+ MB) and textures (window and output size) as large as the
/// adapter allows.
pub fn limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
    let a = adapter.limits();
    wgpu::Limits {
        max_storage_buffer_binding_size: a.max_storage_buffer_binding_size,
        max_texture_dimension_1d: a.max_texture_dimension_1d,
        max_texture_dimension_2d: a.max_texture_dimension_2d,
        ..wgpu::Limits::downlevel_defaults()
    }
}

impl Renderer {
    /// A headless renderer on the best GPU available (blocks; not for the
    /// web, where adapters arrive asynchronously: use `from_device`).
    pub fn new() -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| format!("no GPU adapter: {e}"))?;
        let name = adapter_name(&adapter);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("remodel"),
            required_limits: limits(&adapter),
            required_features: features(&adapter),
            ..Default::default()
        }))
        .map_err(|e| format!("{name}: {e}"))?;
        let mut r = Self::from_device(device, queue, name);
        r.max_msaa = max_msaa(&adapter);
        Ok(r)
    }

    /// A renderer on a device the caller created (with at least
    /// [`limits`]). Multisampling is limited to 4x unless the caller says
    /// otherwise with [`Renderer::set_max_msaa`] (from [`max_msaa`], for a
    /// device created with [`features`]).
    pub fn from_device(device: wgpu::Device, queue: wgpu::Queue, adapter: String) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("raster"),
            source: wgpu::ShaderSource::Wgsl(raster_source().into()),
        });
        let raster_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("raster"),
            entries: &[uniform(0, wgpu::ShaderStages::VERTEX_FRAGMENT), storage(1), storage(2), storage(3)],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("raster"),
            bind_group_layouts: &[Some(&raster_layout)],
            immediate_size: 0,
        });
        let compose_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("compose"),
            source: wgpu::ShaderSource::Wgsl(include_str!("compose.wgsl").into()),
        });
        let compose_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("compose"),
            entries: &[uniform(0, wgpu::ShaderStages::FRAGMENT), texture(1), texture(2), texture(3)],
        });
        Renderer {
            device,
            queue,
            raster_shader: shader,
            raster_pipeline_layout: pipeline_layout,
            raster: HashMap::new(),
            quality: Quality::default(),
            max_msaa: 4,
            raster_layout,
            compose_shader,
            compose_layout,
            compose: None,
            adapter,
            ram: [None, None],
            versions: None,
            textures: Vec::new(),
        }
    }

    /// The GPU in use: name, backend and driver.
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// The most samples per pixel [`Quality::msaa`] can use here.
    pub fn max_msaa(&self) -> u32 {
        self.max_msaa
    }

    /// Allow up to `n` samples (see [`max_msaa`]).
    pub fn set_max_msaa(&mut self, n: u32) {
        self.max_msaa = n.clamp(4, 8);
    }

    /// Draw the 3D layer with `quality` from the next frame on. The sample
    /// count is rounded to what exists (1, 4, 8) and limited to the GPU's;
    /// the result is the quality in effect.
    pub fn set_quality(&mut self, quality: Quality) -> Quality {
        let msaa = match quality.msaa {
            0 | 1 => 1,
            2..=7 => 4,
            _ => self.max_msaa.min(8),
        };
        self.quality = Quality { msaa, ..quality };
        self.quality
    }

    pub fn quality(&self) -> Quality {
        self.quality
    }

    /// The raster pipeline for `samples` and `mode`.
    fn raster_pipeline(&mut self, samples: u32, mode: Mode) -> wgpu::RenderPipeline {
        if let Some(p) = self.raster.get(&(samples, mode)) {
            return p.clone();
        }
        let (vs, fs) = match mode {
            Mode::Clear => ("vs_clear", "fs_clear"),
            Mode::Depth | Mode::Test | Mode::DepthWrite => ("vs", "fs_depth"),
            _ => ("vs", "fs"),
        };
        let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x3, 2 => Uint32];
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: VERTEX_BYTES,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        })];
        let depth_stencil = (mode != Mode::Plain).then(|| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(matches!(mode, Mode::Depth | Mode::DepthWrite | Mode::Clear)),
            depth_compare: Some(if matches!(mode, Mode::Depth | Mode::Test | Mode::DepthWrite) {
                wgpu::CompareFunction::GreaterEqual
            } else {
                wgpu::CompareFunction::Always
            }),
            stencil: Default::default(),
            bias: Default::default(),
        });
        let p = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("raster"),
            layout: Some(&self.raster_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &self.raster_shader,
                entry_point: Some(vs),
                compilation_options: Default::default(),
                buffers: if mode == Mode::Clear { &[] } else { &buffers },
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil,
            multisample: wgpu::MultisampleState { count: samples, mask: !0, alpha_to_coverage_enabled: false },
            fragment: Some(wgpu::FragmentState {
                module: &self.raster_shader,
                entry_point: Some(fs),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FORMAT,
                    blend: None,
                    write_mask: if matches!(mode, Mode::Clear | Mode::DepthWrite) {
                        wgpu::ColorWrites::empty()
                    } else {
                        wgpu::ColorWrites::ALL
                    },
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.raster.insert((samples, mode), p.clone());
        p
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// The queue, e.g. to present a surface texture after [`Self::present`].
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Versions of the next frame's render RAM and palette RAM (e.g.
    /// `Model2Bus::video_ram_versions`): unchanged ones aren't uploaded
    /// again. Without this, every frame uploads both.
    pub fn set_ram_versions(&mut self, render: u64, palette: u64) {
        self.versions = Some((render, palette));
    }

    /// Forget the game memory uploaded, so the next frame uploads it whole:
    /// for another machine, whose RAM versions say nothing about what the
    /// last one's buffers hold.
    pub fn forget_ram(&mut self) {
        self.ram = [None, None];
    }

    /// A storage buffer holding `data` (game memory slot 0: render RAM,
    /// 1: palette RAM), uploaded only if `version` changed.
    fn ram_buffer(&mut self, slot: usize, data: &[u8], version: Option<u64>) -> wgpu::Buffer {
        let size = data.len().div_ceil(4).max(1) as u64 * 4;
        if let Some((v, b)) = &self.ram[slot] {
            if b.size() == size && version.is_some() && *v == version {
                return b.clone();
            }
        }
        let b = match &self.ram[slot] {
            Some((_, b)) if b.size() == size => b.clone(),
            _ => self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(["render ram", "palette"][slot]),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        };
        if data.len().is_multiple_of(4) {
            self.queue.write_buffer(&b, 0, data);
        } else {
            self.queue.write_buffer(&b, 0, &words(data));
        }
        self.ram[slot] = Some((version, b.clone()));
        b
    }

    /// A texture kept between frames (re-created if the size changed).
    fn kept_texture(&mut self, label: &'static str, w: u32, h: u32, usage: wgpu::TextureUsages) -> wgpu::Texture {
        if let Some((_, t)) = self.textures.iter().find(|(l, t)| *l == label && t.width() == w && t.height() == h) {
            return t.clone();
        }
        let t = self.texture(label, w, h, usage);
        self.textures.retain(|(l, _)| *l != label);
        self.textures.push((label, t.clone()));
        t
    }

    /// A render attachment kept between frames: the multisampled colour
    /// image or the depth buffer.
    fn kept_attachment(
        &mut self,
        label: &'static str,
        w: u32,
        h: u32,
        samples: u32,
        format: wgpu::TextureFormat,
    ) -> wgpu::Texture {
        let same = |t: &wgpu::Texture| {
            t.width() == w && t.height() == h && t.sample_count() == samples && t.format() == format
        };
        if let Some((_, t)) = self.textures.iter().find(|(l, t)| *l == label && same(t)) {
            return t.clone();
        }
        let t = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.textures.retain(|(l, _)| *l != label);
        self.textures.push((label, t.clone()));
        t
    }

    /// Encode the 3D layer of `placed` into `target` (FORMAT, the size of
    /// `view`; alpha 0 where nothing is drawn, a fraction at multisampled
    /// edges, with colour premultiplied by it).
    fn encode_3d(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        polys: &[Poly],
        placed: &[Placed],
        src: &Sources,
        view: &View,
        target: &wgpu::TextureView,
    ) {
        let (w, h) = (view.width as u32, view.height as u32);
        let mut verts: Vec<u8> = Vec::new();
        let mut table: Vec<u8> = Vec::new();
        let q = self.quality;
        // Draws: runs of polygons in the same window and depth mode.
        struct Run {
            vertices: std::ops::Range<u32>,
            window: usize,
            mode: Mode,
            clip: (f32, f32, f32, f32),
        }
        let mut runs: Vec<Run> = Vec::new();
        let push = |runs: &mut Vec<Run>, vertices: std::ops::Range<u32>, window: usize, mode: Mode, clip| match runs
            .last_mut()
        {
            Some(r) if r.window == window && r.mode == mode && r.vertices.end == vertices.start => {
                r.vertices.end = vertices.end
            }
            _ => runs.push(Run { vertices, window, mode, clip }),
        };
        // The members of a group (`Placed::group`) of more than one polygon
        // are drawn tested against the depth before the group, and write
        // their depth once all are drawn: the depth-only draws to come.
        let mut group_depth: Vec<Run> = Vec::new();
        let group_len = |i: usize| {
            let g = placed[i].group;
            let before = placed[..i].iter().rev().take_while(|o| o.group == g).count();
            let after = placed[i..].iter().take_while(|o| o.group == g).count();
            before + after
        };
        for (i, pl) in placed.iter().enumerate() {
            let p = &polys[pl.poly];
            let t = p.texture;
            let flags = t
                .map_or(0, |t| 1 | (t.mirror_x as u32) << 1 | (t.mirror_y as u32) << 2 | (t.translucent as u32) << 3)
                | (p.checker as u32) << 4;
            let (x0, y0, x1, y1) = pl.clip;
            let tex = t.map_or([0; 4], |t| [t.x0, t.y0, t.w, t.h]);
            let (bank, luma_table) = t.map_or((0, 0), |t| (t.bank, t.luma_table));
            table.extend(le([x0, y0, x1, y1].map(f32::to_le_bytes)));
            let ints = tex.into_iter().chain([flags, bank, luma_table, p.luma]).chain(p.colour).chain([pl.rank]);
            table.extend(le(ints.map(u32::to_le_bytes)));
            let first = (verts.len() as u64 / VERTEX_BYTES) as u32;
            // Convex polygon as a triangle fan.
            for k in 1..pl.points.len().saturating_sub(1) {
                for v in [pl.points[0], pl.points[k], pl.points[k + 1]] {
                    verts.extend(le([v.x, v.y, v.iz, v.uz, v.vz].map(f32::to_le_bytes)));
                    verts.extend((i as u32).to_le_bytes());
                }
            }
            let end = (verts.len() as u64 / VERTEX_BYTES) as u32;
            let grouped = q.depth && !pl.background && !p.checker && group_len(i) > 1;
            let mode = match (q.depth, pl.background, p.checker) {
                (false, ..) => Mode::Plain,
                (_, true, _) => Mode::Background,
                (_, _, true) => Mode::Test,
                _ if grouped => Mode::Test,
                _ => Mode::Depth,
            };
            push(&mut runs, first..end, p.window, mode, pl.clip);
            if grouped {
                group_depth.push(Run { vertices: first..end, window: p.window, mode: Mode::DepthWrite, clip: pl.clip });
            }
            if placed.get(i + 1).is_none_or(|n| n.group != pl.group) {
                for r in group_depth.drain(..) {
                    push(&mut runs, r.vertices, r.window, r.mode, r.clip);
                }
            }
        }
        if table.is_empty() {
            table.resize(64, 0);
        }
        let versions = self.versions.take();
        let render = self.ram_buffer(0, src.render_ram, versions.map(|v| v.0));
        let palette = self.ram_buffer(1, src.palette, versions.map(|v| v.1));
        let d = &self.device;
        let buffer = |label, contents: &[u8], usage| {
            d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage })
        };
        let screen = le([w as f32, h as f32, view.sx, view.sy].map(f32::to_le_bytes));
        let screen = buffer("screen", &screen, wgpu::BufferUsages::UNIFORM);
        let table = buffer("polys", &table, wgpu::BufferUsages::STORAGE);
        let vertices = buffer("vertices", &words(&verts), wgpu::BufferUsages::VERTEX);
        let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("raster"),
            layout: &self.raster_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: screen.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: table.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: render.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: palette.as_entire_binding() },
            ],
        });
        let samples = q.msaa.max(1);
        let msaa = (samples > 1).then(|| self.kept_attachment("3d msaa", w, h, samples, FORMAT));
        let msaa_view = msaa.as_ref().map(|t| t.create_view(&Default::default()));
        let depth = q.depth.then(|| self.kept_attachment("3d depth", w, h, samples, DEPTH_FORMAT));
        let depth_view = depth.as_ref().map(|t| t.create_view(&Default::default()));
        let pipelines: Vec<(Mode, wgpu::RenderPipeline)> = if q.depth {
            [Mode::Depth, Mode::Test, Mode::DepthWrite, Mode::Background]
                .map(|m| (m, self.raster_pipeline(samples, m)))
                .into()
        } else {
            vec![(Mode::Plain, self.raster_pipeline(samples, Mode::Plain))]
        };
        let clear_pipeline = q.depth.then(|| self.raster_pipeline(samples, Mode::Clear));
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("3d layer"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: msaa_view.as_ref().unwrap_or(target),
                depth_slice: None,
                resolve_target: msaa_view.as_ref().map(|_| target),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: if msaa_view.is_some() { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store },
                },
            })],
            depth_stencil_attachment: depth_view.as_ref().map(|view| wgpu::RenderPassDepthStencilAttachment {
                view,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Discard }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_bind_group(0, &bind, &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        for (k, run) in runs.iter().enumerate() {
            // Each window starts with an empty depth buffer where it draws
            // (the first has the pass's clear).
            if let (Some(clear), true) = (&clear_pipeline, k > 0 && runs[k - 1].window != run.window) {
                let (x0, y0) = (run.clip.0.floor().max(0.0) as u32, run.clip.1.floor().max(0.0) as u32);
                let (x1, y1) = ((run.clip.2.ceil().max(0.0) as u32).min(w), (run.clip.3.ceil().max(0.0) as u32).min(h));
                if x1 > x0 && y1 > y0 {
                    pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
                    pass.set_pipeline(clear);
                    pass.draw(0..3, 0..1);
                    pass.set_scissor_rect(0, 0, w, h);
                }
            }
            if let Some((_, pipeline)) = pipelines.iter().find(|(m, _)| *m == run.mode) {
                pass.set_pipeline(pipeline);
            }
            pass.draw(run.vertices.clone(), 0..1);
        }
    }

    fn texture(&self, label: &str, w: u32, h: u32, usage: wgpu::TextureUsages) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage,
            view_formats: &[],
        })
    }

    /// Submit `enc` after copying `texture` (FORMAT, w x h) out, and return
    /// its RGBA pixels. Blocks: native only.
    fn read_back(&self, mut enc: wgpu::CommandEncoder, texture: &wgpu::Texture, w: u32, h: u32) -> Vec<u8> {
        let row = (w * 4).div_ceil(256) * 256;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        let data = slice.get_mapped_range().expect("readback buffer is mapped");
        (0..h as usize).flat_map(|y| data[y * row as usize..y * row as usize + w as usize * 4].to_vec()).collect()
    }

    /// Read back a `w`x`h` texture with `COPY_SRC` usage (4 bytes a pixel,
    /// in its own channel order). Blocks: native only.
    pub fn read_texture(&self, texture: &wgpu::Texture, w: u32, h: u32) -> Vec<u8> {
        self.read_back(self.device.create_command_encoder(&Default::default()), texture, w, h)
    }

    /// Draw `placed` (from `model2::raster::place` over `polys`) at the size
    /// of `view` and read it back: RGB and a coverage mask, like the
    /// software rasteriser's `draw_view`. Blocks: native only.
    pub fn draw(&mut self, polys: &[Poly], placed: &[Placed], src: &Sources, view: &View) -> (Vec<u8>, Vec<bool>) {
        let (w, h) = (view.width as u32, view.height as u32);
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        let target = self.texture("3d layer", w, h, usage);
        let mut enc = self.device.create_command_encoder(&Default::default());
        self.encode_3d(&mut enc, polys, placed, src, view, &target.create_view(&Default::default()));
        let rgba = self.read_back(enc, &target, w, h);
        // Colour is premultiplied by coverage at multisampled edges.
        let un = |c: u8, a: u8| ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
        let rgb = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| {
                if p[3] == 0 || p[3] == 255 {
                    [p[0], p[1], p[2]]
                } else {
                    [un(p[0], p[3]), un(p[1], p[3]), un(p[2], p[3])]
                }
            })
            .collect();
        let covered = rgba.as_chunks::<4>().0.iter().map(|p| p[3] != 0).collect();
        (rgb, covered)
    }

    fn upload(&mut self, label: &'static str, rgba: &[u8], w: u32, h: u32) -> wgpu::Texture {
        let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        let t = self.kept_texture(label, w, h, usage);
        self.queue.write_texture(
            t.as_image_copy(),
            rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: None },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        t
    }

    fn compose_pipeline(&mut self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        if let Some((f, p)) = &self.compose {
            if *f == format {
                return p.clone();
            }
        }
        let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("compose"),
            bind_group_layouts: &[Some(&self.compose_layout)],
            immediate_size: 0,
        });
        let p = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("compose"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &self.compose_shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.compose_shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.compose = Some((format, p.clone()));
        p
    }

    /// Encode a whole frame into `target` (an output of `format`, the size
    /// of `view`; use a non-sRGB format: colours are already display
    /// values): the 3D layer of `placed`, with the 2D `layers` around it.
    #[allow(clippy::too_many_arguments)]
    fn encode_frame(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        polys: &[Poly],
        placed: &[Placed],
        src: &Sources,
        view: &View,
        layers: &Layers2d,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        origin: (u32, u32),
    ) {
        let (w, h) = (view.width as u32, view.height as u32);
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let layer3d = self.kept_texture("3d layer", w, h, usage);
        let layer3d_view = layer3d.create_view(&Default::default());
        self.encode_3d(enc, polys, placed, src, view, &layer3d_view);
        let base = self.upload("base", &layers.base, (WIDTH + 2 * layers.side) as u32, HEIGHT as u32);
        let hud = self.upload("hud", &layers.hud, layers.ext_w as u32, HEIGHT as u32);
        let b = layers.backdrop.map(|c| c as f32 / 255.0);
        let mut params = le([view.sx, view.sy, view.pad].map(f32::to_le_bytes));
        params.extend(le([!placed.is_empty() as u32, layers.ext_w as u32, layers.side as u32].map(u32::to_le_bytes)));
        params.extend(le([origin.0 as f32, origin.1 as f32].map(f32::to_le_bytes)));
        params.extend(le([b[0], b[1], b[2], 1.0].map(f32::to_le_bytes)));
        let params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("compose params"),
            contents: &params,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let views = [base.create_view(&Default::default()), hud.create_view(&Default::default())];
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("compose"),
            layout: &self.compose_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&views[0]) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&layer3d_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&views[1]) },
            ],
        });
        let pipeline = self.compose_pipeline(format);
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("compose"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        pass.set_viewport(origin.0 as f32, origin.1 as f32, w as f32, h as f32, 0.0, 1.0);
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Draw a whole frame into `target` (a surface texture, say) and submit.
    #[allow(clippy::too_many_arguments)]
    pub fn present(
        &mut self,
        polys: &[Poly],
        placed: &[Placed],
        src: &Sources,
        view: &View,
        layers: &Layers2d,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
    ) {
        let mut enc = self.device.create_command_encoder(&Default::default());
        self.encode_frame(&mut enc, polys, placed, src, view, layers, target, format, (0, 0));
        self.queue.submit([enc.finish()]);
    }

    /// Draw the Model 2 board's current frame into `target` at `origin`
    /// (the rest of `target` is cleared to black) and submit. `geo` is the
    /// geometrizer output to draw (raster stream, texture point RAM), or
    /// None for the bus's own engine.
    #[allow(clippy::too_many_arguments)]
    pub fn present_bus(
        &mut self,
        bus: &mut model2::Model2Bus,
        geo: Option<(&[u32], &[u16])>,
        view: &View,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        origin: (u32, u32),
    ) {
        if geo.is_none() {
            bus.geo.sync();
        }
        let b = &*bus;
        let geo = geo.unwrap_or_else(|| (&b.geo.engine().stream, &b.geo.engine().texture_point_ram));
        self.draw_bus(b, geo, view, target, format, origin);
    }

    /// [`Renderer::present_bus`] with the polygons of the geometrizer's
    /// previous frame (`Geometrizer::last_frame`), not waiting for the one
    /// in progress.
    pub fn present_bus_last(
        &mut self,
        bus: &model2::Model2Bus,
        view: &View,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        origin: (u32, u32),
    ) {
        self.draw_bus(bus, bus.geo.last_frame(), view, target, format, origin);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_bus(
        &mut self,
        b: &model2::Model2Bus,
        (stream, points): (&[u32], &[u16]),
        view: &View,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        origin: (u32, u32),
    ) {
        let (xoff, yoff) = b.sync_offsets();
        let src = Sources {
            texture_rom: &b.texture_rom,
            texture_point_ram: points,
            render_ram: b.render_ram(),
            palette: b.palette_ram(),
            xoff,
            yoff,
        };
        let mut polys = model2::raster::build(stream, &src);
        model2::video::HudLayout::new(b.tile_ram(), b.palette_ram(), view).anchor(&mut polys, view);
        let placed = model2::raster::place(&mut polys, view);
        let layers = model2::video::layers_2d(b.tile_ram(), b.palette_ram(), view, !placed.is_empty());
        let (render, palette) = b.video_ram_versions();
        self.set_ram_versions(render, palette);
        let mut enc = self.device.create_command_encoder(&Default::default());
        self.encode_frame(&mut enc, &polys, &placed, &src, view, &layers, target, format, origin);
        self.queue.submit([enc.finish()]);
    }

    /// `present` into an offscreen texture and read it back as RGB, for
    /// checking against the CPU compositor. Blocks: native only.
    pub fn frame(
        &mut self,
        polys: &[Poly],
        placed: &[Placed],
        src: &Sources,
        view: &View,
        layers: &Layers2d,
    ) -> Vec<u8> {
        let (w, h) = (view.width as u32, view.height as u32);
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        let out = self.texture("frame", w, h, usage);
        let mut enc = self.device.create_command_encoder(&Default::default());
        self.encode_frame(
            &mut enc,
            polys,
            placed,
            src,
            view,
            layers,
            &out.create_view(&Default::default()),
            FORMAT,
            (0, 0),
        );
        let rgba = self.read_back(enc, &out, w, h);
        rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect()
    }
}

/// "Name (backend, driver)" for an adapter.
pub fn adapter_name(adapter: &wgpu::Adapter) -> String {
    let info = adapter.get_info();
    format!("{} ({:?}, {})", info.name, info.backend, info.driver)
}

#[cfg(test)]
mod tests {
    fn validate(src: &str) {
        let module = naga::front::wgsl::parse_str(src).expect("WGSL parses");
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::default())
            .validate(&module)
            .expect("WGSL validates");
    }

    #[test]
    fn shaders_parse_and_validate() {
        validate(&super::raster_source());
        validate(include_str!("compose.wgsl"));
    }
}
