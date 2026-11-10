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

pub use model2::enhance::TexFilter;
use model2::enhance::{Found, Rgba, TexturePack};
use model2::raster::{tex_info, Placed, Poly, Sources, TexKeys};
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
    /// Filtering of texture pack images ([`Renderer::set_pack`]): off (the
    /// image as is, bilinear), mipmaps, or mipmaps with anisotropic
    /// filtering (up to 2, 4, 8 or 16 times; best effort, the GPU may use
    /// less). No effect without a pack: original textures are never
    /// filtered this way.
    pub tex_filter: TexFilter,
}

impl Default for Quality {
    fn default() -> Self {
        Quality { msaa: 1, depth: false, tex_filter: TexFilter::Off }
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
    /// Depth tested but not written (a polygon wins where its depth, raised
    /// by `DEPTH_TOLERANCE`, is at least what is there): the colour of the
    /// polygons of a group (see `Placed::group`).
    Test,
    /// Depth written where tested, no colour: a group's depth, after its
    /// colour.
    DepthWrite,
    /// With a depth buffer in the pass, but neither tested nor written:
    /// `Placed::background` polygons (skies, shadows, translucent ones and
    /// those spanning a wide range of depth).
    Background,
    /// Empties the depth buffer within the scissor rectangle.
    Clear,
}

/// Which shaders a raster pipeline runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Variant {
    /// The original textures (`fs`, `fs_depth`; group 0 only).
    Original,
    /// With texture pack images in group 1, drawn exactly (`fs_pack`,
    /// `fs_pack_depth`): only for frames where a polygon is replaced.
    Pack,
    /// With texture pack mip chains and a sampler in group 1 (`vs_pack`,
    /// `fs_packf`, `fs_packf_depth`): the same, with `tex_filter` on.
    PackFiltered,
}

/// A texture pack image on the GPU.
struct PackTex {
    view: wgpu::TextureView,
    /// A premultiplied mip chain (for filtering), or the image as is, one
    /// level (exact).
    mipmapped: bool,
    /// Bind groups, made when first needed: exact in [0]; filtered by the
    /// sampler's address modes (mirror x | mirror y << 1).
    groups: [Option<wgpu::BindGroup>; 4],
    bytes: u64,
    /// Frame it was last drawn in (for eviction).
    used: u64,
}

/// Texture pack counters for the last frame drawn (see
/// [`Renderer::pack_stats`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PackStats {
    /// Textured polygons looked up in the pack, and of them replaced.
    pub textured: u32,
    pub replaced: u32,
    /// Distinct pack images drawn.
    pub images: u32,
    /// Draw calls of polygons (runs), with or without a pack.
    pub draws: u32,
    /// Images uploaded this frame; images and bytes held on the GPU.
    pub uploaded: u32,
    pub held: u32,
    pub bytes: u64,
}

/// GPU memory pack images may hold before the least recently drawn are
/// dropped.
#[cfg(not(target_arch = "wasm32"))]
const PACK_BUDGET: u64 = 512 << 20;
#[cfg(target_arch = "wasm32")]
const PACK_BUDGET: u64 = 192 << 20;

/// No pack image: a polygon drawn from its own texels.
const NO_SLOT: u32 = u32::MAX;

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

/// The most anisotropic filtering [`Quality::tex_filter`] can ask for on
/// this adapter: 16, or 1 (none) where the backend lacks it.
pub fn max_anisotropy(adapter: &wgpu::Adapter) -> u8 {
    if adapter.get_downlevel_capabilities().flags.contains(wgpu::DownlevelFlags::ANISOTROPIC_FILTERING) {
        16
    } else {
        1
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
    /// Raster pipelines by sample count, mode and variant, made when first
    /// needed.
    raster: HashMap<(u32, Mode, Variant), wgpu::RenderPipeline>,
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
    /// The texture pack, if any (`set_pack`).
    pack: Option<Box<dyn TexturePack + Send>>,
    /// Texture keys, kept while render RAM is unchanged.
    keys: TexKeys,
    /// Pack images by texture key (None: not in the pack).
    pack_tex: HashMap<u64, Option<PackTex>>,
    pack_layout: wgpu::BindGroupLayout,
    pack_pipeline_layout: wgpu::PipelineLayout,
    pack_filtered_layout: wgpu::BindGroupLayout,
    pack_filtered_pipeline_layout: wgpu::PipelineLayout,
    /// Samplers for filtered pack images by address modes (see
    /// `PackTex::groups`), for the current `tex_filter`.
    samplers: Option<[wgpu::Sampler; 4]>,
    max_anisotropy: u8,
    /// A 1x1 image bound before a pack frame's first replaced polygon:
    /// exact, filtered.
    pack_dummy: [Option<wgpu::BindGroup>; 2],
    pack_stats: PackStats,
    /// Frames drawn with a pack, for eviction.
    frame: u64,
    /// An image was too large for the GPU (warned once).
    pack_warned: bool,
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

/// Group 1 of the filtered pack pipelines: the image and its sampler.
fn filtered_entries() -> [wgpu::BindGroupLayoutEntry; 2] {
    [
        wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
    ]
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
        r.max_anisotropy = max_anisotropy(&adapter);
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
            entries: &[uniform(0, wgpu::ShaderStages::VERTEX_FRAGMENT), storage(1), storage(2), storage(3), storage(4)],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("raster"),
            bind_group_layouts: &[Some(&raster_layout)],
            immediate_size: 0,
        });
        let pack_layout = device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("pack"), entries: &[texture(0)] });
        let pack_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("raster pack"),
            bind_group_layouts: &[Some(&raster_layout), Some(&pack_layout)],
            immediate_size: 0,
        });
        let pack_filtered_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pack filtered"),
            entries: &filtered_entries(),
        });
        let pack_filtered_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("raster pack filtered"),
            bind_group_layouts: &[Some(&raster_layout), Some(&pack_filtered_layout)],
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
            pack: None,
            keys: TexKeys::default(),
            pack_tex: HashMap::new(),
            pack_layout,
            pack_pipeline_layout,
            pack_filtered_layout,
            pack_filtered_pipeline_layout,
            samplers: None,
            max_anisotropy: 16,
            pack_dummy: [None, None],
            pack_stats: PackStats::default(),
            frame: 0,
            pack_warned: false,
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

    /// Allow anisotropic filtering up to `n` times (see
    /// [`max_anisotropy`]; 1: none, so `tex_filter` stops at mipmaps).
    pub fn set_max_anisotropy(&mut self, n: u8) {
        self.max_anisotropy = n.clamp(1, 16);
    }

    /// Draw the 3D layer with `quality` from the next frame on. The sample
    /// count is rounded to what exists (1, 4, 8) and limited to the GPU's,
    /// anisotropy likewise (2, 4, 8, 16; mipmaps alone where the GPU has
    /// none); the result is the quality in effect.
    pub fn set_quality(&mut self, quality: Quality) -> Quality {
        let msaa = match quality.msaa {
            0 | 1 => 1,
            2..=7 => 4,
            _ => self.max_msaa.min(8),
        };
        let tex_filter = match quality.tex_filter {
            TexFilter::Aniso(n) if n >= 2 && self.max_anisotropy >= 2 => {
                TexFilter::Aniso(n.min(16).next_power_of_two().min(self.max_anisotropy))
            }
            TexFilter::Aniso(_) => TexFilter::Mip,
            f => f,
        };
        if tex_filter != self.quality.tex_filter {
            // New samplers; images change form between exact and filtered.
            self.samplers = None;
            let mip = tex_filter.mipmapped();
            self.pack_tex.retain(|_, t| t.as_ref().is_none_or(|t| t.mipmapped == mip));
            for t in self.pack_tex.values_mut().flatten() {
                t.groups = Default::default();
            }
        }
        self.quality = Quality { msaa, tex_filter, ..quality };
        self.quality
    }

    pub fn quality(&self) -> Quality {
        self.quality
    }

    /// Draw replaced textures from `pack` from the next frame on (None:
    /// the originals, the default). Images are uploaded when first drawn
    /// and kept, up to a memory budget, until the pack changes.
    pub fn set_pack(&mut self, pack: Option<Box<dyn TexturePack + Send>>) {
        self.pack = pack;
        self.pack_tex.clear();
        self.keys.clear();
        self.pack_stats = PackStats::default();
    }

    /// Whether a texture pack is set.
    pub fn has_pack(&self) -> bool {
        self.pack.is_some()
    }

    /// Texture pack counters for the last frame drawn.
    pub fn pack_stats(&self) -> PackStats {
        self.pack_stats
    }

    /// The raster pipeline for `samples`, `mode` and `variant`.
    fn raster_pipeline(&mut self, samples: u32, mode: Mode, variant: Variant) -> wgpu::RenderPipeline {
        if let Some(p) = self.raster.get(&(samples, mode, variant)) {
            return p.clone();
        }
        let (vs, fs) = match (mode, variant) {
            (Mode::Clear, _) => ("vs_clear", "fs_clear"),
            (Mode::Test | Mode::DepthWrite, Variant::Original) => ("vs", "fs_depth"),
            (_, Variant::Original) => ("vs", "fs"),
            (Mode::Test | Mode::DepthWrite, Variant::Pack) => ("vs", "fs_pack_depth"),
            (_, Variant::Pack) => ("vs", "fs_pack"),
            (Mode::Test | Mode::DepthWrite, Variant::PackFiltered) => ("vs_pack", "fs_packf_depth"),
            (_, Variant::PackFiltered) => ("vs_pack", "fs_packf"),
        };
        let layout = match variant {
            Variant::Original => &self.raster_pipeline_layout,
            Variant::Pack => &self.pack_pipeline_layout,
            Variant::PackFiltered => &self.pack_filtered_pipeline_layout,
        };
        let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x3, 2 => Uint32];
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: VERTEX_BYTES,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        })];
        let depth_stencil = (mode != Mode::Plain).then(|| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(matches!(mode, Mode::DepthWrite | Mode::Clear)),
            depth_compare: Some(if matches!(mode, Mode::Test | Mode::DepthWrite) {
                wgpu::CompareFunction::GreaterEqual
            } else {
                wgpu::CompareFunction::Always
            }),
            stencil: Default::default(),
            bias: Default::default(),
        });
        // The test is in favour of the game's order (raster::depth_test_value).
        let test_factor = [("DEPTH_FACTOR", f64::from(1.0 + model2::raster::DEPTH_TOLERANCE))];
        let fs_options = wgpu::PipelineCompilationOptions {
            constants: if mode == Mode::Test { &test_factor } else { &[] },
            ..Default::default()
        };
        let p = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("raster"),
            layout: Some(layout),
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
                compilation_options: fs_options,
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
        self.raster.insert((samples, mode, variant), p.clone());
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
        self.keys.clear();
    }

    /// For each of `placed`, the slot of its pack image in the returned
    /// bind groups, or NO_SLOT to draw its own texels. Looks textures up in
    /// the pack (once per key), uploads new images and counts `pack_stats`.
    fn pack_slots(
        &mut self,
        polys: &[Poly],
        placed: &[Placed],
        src: &Sources,
        render_version: Option<u64>,
    ) -> (Vec<u32>, Vec<wgpu::BindGroup>) {
        let mut slots = vec![NO_SLOT; placed.len()];
        let mut groups = Vec::new();
        if self.pack.is_none() {
            return (slots, groups);
        }
        let mip = self.quality.tex_filter.mipmapped();
        if mip && self.samplers.is_none() {
            self.samplers = Some(pack_samplers(&self.device, self.quality.tex_filter));
        }
        let Some(pack) = self.pack.as_mut() else { return (slots, groups) };
        self.keys.begin(render_version);
        self.frame += 1;
        let frame = self.frame;
        let max = self.device.limits().max_texture_dimension_2d as usize;
        let mut stats = PackStats::default();
        // Slots by texture key and address modes, this frame.
        let mut slot_of: HashMap<(u64, usize), u32> = HashMap::new();
        for (i, pl) in placed.iter().enumerate() {
            let p = &polys[pl.poly];
            let Some(t) = &p.texture else { continue };
            stats.textured += 1;
            let Some(info) = tex_info(p, t, src, &mut self.keys) else { continue };
            let entry = match self.pack_tex.entry(info.key) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => match pack.find(&info) {
                    Found::Pending => continue,
                    Found::Missing => e.insert(None),
                    Found::Image(img) => {
                        let tex = if img.width == 0 || img.height == 0 || img.width > max || img.height > max {
                            if !self.pack_warned {
                                self.pack_warned = true;
                                eprintln!(
                                    "texture pack: {} is {}x{}, more than this GPU's {max}x{max}; drawing the original",
                                    info.name(),
                                    img.width,
                                    img.height
                                );
                            }
                            None
                        } else {
                            stats.uploaded += 1;
                            Some(upload_pack(&self.device, &self.queue, &img, mip))
                        };
                        e.insert(tex)
                    }
                },
            };
            let Some(pt) = entry else { continue };
            pt.used = frame;
            stats.replaced += 1;
            let addr = if mip { t.mirror_x as usize | (t.mirror_y as usize) << 1 } else { 0 };
            slots[i] = *slot_of.entry((info.key, addr)).or_insert_with(|| {
                let group = pt.groups[addr].get_or_insert_with(|| match &self.samplers {
                    Some(smp) if mip => bind_pack(&self.device, &self.pack_filtered_layout, &pt.view, Some(&smp[addr])),
                    _ => bind_pack(&self.device, &self.pack_layout, &pt.view, None),
                });
                groups.push(group.clone());
                groups.len() as u32 - 1
            });
        }
        // Over budget: drop the images drawn longest ago (not this frame's).
        let mut bytes: u64 = self.pack_tex.values().flatten().map(|t| t.bytes).sum();
        if bytes > PACK_BUDGET {
            let mut old: Vec<(u64, u64, u64)> =
                self.pack_tex.iter().filter_map(|(k, t)| t.as_ref().map(|t| (t.used, t.bytes, *k))).collect();
            old.sort_unstable();
            for (used, size, key) in old {
                if bytes <= PACK_BUDGET || used == frame {
                    break;
                }
                self.pack_tex.remove(&key);
                bytes -= size;
            }
        }
        stats.images = groups.len() as u32;
        stats.held = self.pack_tex.values().flatten().count() as u32;
        stats.bytes = bytes;
        self.pack_stats = stats;
        (slots, groups)
    }

    /// The 1x1 bind group for group 1 before a replaced polygon is drawn
    /// (for the filtered pipelines once the samplers exist; any sampler
    /// will do, it is never sampled).
    fn pack_dummy(&mut self, filtered: bool) -> wgpu::BindGroup {
        let k = filtered as usize;
        if let Some(g) = &self.pack_dummy[k] {
            return g.clone();
        }
        let img = Rgba { width: 1, height: 1, data: vec![0; 4] };
        let t = upload_pack(&self.device, &self.queue, &img, filtered);
        let g = match &self.samplers {
            Some(smp) if filtered => bind_pack(&self.device, &self.pack_filtered_layout, &t.view, Some(&smp[0])),
            _ => bind_pack(&self.device, &self.pack_layout, &t.view, None),
        };
        self.pack_dummy[k] = Some(g.clone());
        g
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
        let versions = self.versions.take();
        let (slots, pack_groups) = self.pack_slots(polys, placed, src, versions.map(|v| v.0));
        // Draws: runs of polygons in the same window and depth mode, and
        // with the same pack image (`slot`; NO_SLOT: any, as a polygon not
        // replaced reads no image).
        struct Run {
            vertices: std::ops::Range<u32>,
            window: usize,
            mode: Mode,
            clip: (f32, f32, f32, f32),
            slot: u32,
        }
        let mut runs: Vec<Run> = Vec::new();
        let push = |runs: &mut Vec<Run>, vertices: std::ops::Range<u32>, window: usize, mode: Mode, clip, slot: u32| {
            match runs.last_mut() {
                Some(r)
                    if r.window == window
                        && r.mode == mode
                        && r.vertices.end == vertices.start
                        && (r.slot == slot || r.slot == NO_SLOT || slot == NO_SLOT) =>
                {
                    r.vertices.end = vertices.end;
                    if r.slot == NO_SLOT {
                        r.slot = slot;
                    }
                }
                _ => runs.push(Run { vertices, window, mode, clip, slot }),
            }
        };
        // The members of a group (`Placed::group`) are drawn tested against
        // the depth before the group, and write their depth once all are
        // drawn: the depth-only draws to come. (A test and a write of
        // different values: raster::depth_test_value.)
        let mut group_depth: Vec<Run> = Vec::new();
        // Outlines for the shader's 1/z (`outline_iz`): a directory of
        // (first, count) per polygon, then the points.
        let mut outline_dir: Vec<u8> = Vec::new();
        let mut outline_points: Vec<u8> = Vec::new();
        for (i, pl) in placed.iter().enumerate() {
            let p = &polys[pl.poly];
            let t = p.texture;
            let flags = t
                .map_or(0, |t| 1 | (t.mirror_x as u32) << 1 | (t.mirror_y as u32) << 2 | (t.translucent as u32) << 3)
                | (p.checker as u32) << 4
                | ((slots[i] != NO_SLOT) as u32) << 5;
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
            if q.depth {
                let first = placed.len() + outline_points.len() / 16;
                outline_dir.extend(le([first as u32, pl.points.len() as u32, 0, 0].map(u32::to_le_bytes)));
                for v in &pl.points {
                    outline_points.extend(le([v.x, v.y, v.iz, 0.0].map(f32::to_le_bytes)));
                }
            }
            // Translucent (checkerboard) polygons are background.
            let grouped = q.depth && !pl.background;
            let mode = match (q.depth, pl.background) {
                (false, _) => Mode::Plain,
                (_, true) => Mode::Background,
                _ => Mode::Test,
            };
            push(&mut runs, first..end, p.window, mode, pl.clip, slots[i]);
            // The depth-only draw repeats a replaced polygon's cut-out.
            if grouped {
                group_depth.push(Run {
                    vertices: first..end,
                    window: p.window,
                    mode: Mode::DepthWrite,
                    clip: pl.clip,
                    slot: slots[i],
                });
            }
            if placed.get(i + 1).is_none_or(|n| n.group != pl.group) {
                for r in group_depth.drain(..) {
                    push(&mut runs, r.vertices, r.window, r.mode, r.clip, r.slot);
                }
            }
        }
        if table.is_empty() {
            table.resize(64, 0);
        }
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
        outline_dir.extend(outline_points);
        if outline_dir.is_empty() {
            outline_dir.resize(16, 0);
        }
        let outlines = buffer("outlines", &outline_dir, wgpu::BufferUsages::STORAGE);
        let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("raster"),
            layout: &self.raster_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: screen.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: table.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: render.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: palette.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: outlines.as_entire_binding() },
            ],
        });
        let samples = q.msaa.max(1);
        let msaa = (samples > 1).then(|| self.kept_attachment("3d msaa", w, h, samples, FORMAT));
        let msaa_view = msaa.as_ref().map(|t| t.create_view(&Default::default()));
        let depth = q.depth.then(|| self.kept_attachment("3d depth", w, h, samples, DEPTH_FORMAT));
        let depth_view = depth.as_ref().map(|t| t.create_view(&Default::default()));
        // The pack shaders only for frames that replace a texture.
        let variant = match (pack_groups.is_empty(), q.tex_filter.mipmapped()) {
            (true, _) => Variant::Original,
            (false, false) => Variant::Pack,
            (false, true) => Variant::PackFiltered,
        };
        let pipelines: Vec<(Mode, wgpu::RenderPipeline)> = if q.depth {
            [Mode::Test, Mode::DepthWrite, Mode::Background]
                .map(|m| (m, self.raster_pipeline(samples, m, variant)))
                .into()
        } else {
            vec![(Mode::Plain, self.raster_pipeline(samples, Mode::Plain, variant))]
        };
        let clear_pipeline = q.depth.then(|| self.raster_pipeline(samples, Mode::Clear, variant));
        let dummy = (variant != Variant::Original).then(|| self.pack_dummy(variant == Variant::PackFiltered));
        self.pack_stats.draws = runs.len() as u32;
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
        if let Some(dummy) = &dummy {
            pass.set_bind_group(1, dummy, &[]);
        }
        let mut bound = NO_SLOT;
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
            if run.slot != NO_SLOT && run.slot != bound {
                bound = run.slot;
                pass.set_bind_group(1, &pack_groups[run.slot as usize], &[]);
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

/// Upload a pack image: as is (straight alpha, one level), or `mipmapped`
/// as its premultiplied mip chain (`Rgba::mip_chain`).
fn upload_pack(device: &wgpu::Device, queue: &wgpu::Queue, img: &Rgba, mipmapped: bool) -> PackTex {
    let chain;
    let levels: &[Rgba] = if mipmapped {
        chain = img.mip_chain();
        &chain
    } else {
        std::slice::from_ref(img)
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pack image"),
        size: wgpu::Extent3d { width: img.width as u32, height: img.height as u32, depth_or_array_layers: 1 },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut bytes = 0;
    for (k, l) in levels.iter().enumerate() {
        let (w, h) = (l.width as u32, l.height as u32);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: k as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &l.data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: None },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        bytes += l.data.len() as u64;
    }
    let view = texture.create_view(&Default::default());
    PackTex { view, mipmapped, groups: Default::default(), bytes, used: 0 }
}

/// Group 1 for a pack image: the image, and for the filtered pipelines its
/// sampler.
fn bind_pack(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    sampler: Option<&wgpu::Sampler>,
) -> wgpu::BindGroup {
    let mut entries = vec![wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) }];
    if let Some(s) = sampler {
        entries.push(wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(s) });
    }
    device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("pack image"), layout, entries: &entries })
}

/// Samplers for filtered pack images, by address modes (mirror x | mirror
/// y << 1; the hardware's mirrored repeat is the software's `Mirror`):
/// trilinear, with `filter`'s anisotropy (wgpu wants all three filters
/// linear for it).
fn pack_samplers(device: &wgpu::Device, filter: TexFilter) -> [wgpu::Sampler; 4] {
    let anisotropy_clamp = match filter {
        TexFilter::Aniso(n) => n.clamp(1, 16) as u16,
        _ => 1,
    };
    let mode = |mirror: bool| if mirror { wgpu::AddressMode::MirrorRepeat } else { wgpu::AddressMode::Repeat };
    [0, 1, 2, 3].map(|a: usize| {
        device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("pack"),
            address_mode_u: mode(a & 1 != 0),
            address_mode_v: mode(a & 2 != 0),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp,
            ..Default::default()
        })
    })
}

/// "Name (backend, driver)" for an adapter.
pub fn adapter_name(adapter: &wgpu::Adapter) -> String {
    let info = adapter.get_info();
    format!("{} ({:?}, {})", info.name, info.backend, info.driver)
}

#[cfg(test)]
mod tests {
    fn validate(src: &str) -> naga::Module {
        let module = naga::front::wgsl::parse_str(src).expect("WGSL parses");
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::default())
            .validate(&module)
            .expect("WGSL validates");
        module
    }

    #[test]
    fn shaders_parse_and_validate() {
        let raster = validate(&super::raster_source());
        let names: Vec<&str> = raster.entry_points.iter().map(|e| e.name.as_str()).collect();
        for name in [
            "vs",
            "fs",
            "fs_depth",
            "fs_pack",
            "fs_pack_depth",
            "vs_pack",
            "fs_packf",
            "fs_packf_depth",
            "vs_clear",
            "fs_clear",
        ] {
            assert!(names.contains(&name), "raster.wgsl has no entry point {name}");
        }
        validate(include_str!("compose.wgsl"));
    }

    /// The original entry points use group 0 only, so their pipelines keep
    /// the one-group layout; the pack ones add the image in group 1.
    #[test]
    fn only_pack_entry_points_use_group_1() {
        let module = naga::front::wgsl::parse_str(&super::raster_source()).unwrap();
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), Default::default())
            .validate(&module)
            .unwrap();
        for (k, ep) in module.entry_points.iter().enumerate() {
            let fi = info.get_entry_point(k);
            let uses_group_1 = module
                .global_variables
                .iter()
                .any(|(h, v)| v.binding.as_ref().is_some_and(|b| b.group == 1) && !fi[h].is_empty());
            assert_eq!(uses_group_1, ep.name.starts_with("fs_pack"), "{}", ep.name);
            // Only the filtered ones sample (derivatives, a sampler).
            let samples =
                module.global_variables.iter().any(|(h, v)| v.name.as_deref() == Some("pack_smp") && !fi[h].is_empty());
            assert_eq!(samples, ep.name.starts_with("fs_packf"), "{}", ep.name);
        }
    }
}
