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
//! multisampling, a depth buffer, filtering of texture pack images,
//! temporal anti-aliasing and upscaling, and smooth shading (all off by
//! default). [`Renderer::present`] then blends the
//! 2D tile layers around the 3D layer into the output.

mod bloom;
mod board;
mod crt;
mod look;
mod materials;
mod photo;
mod temporal;
mod tiles;
pub mod userpass;

pub use board::BoardFrame;
use board::BoardRef;
pub use look::{depth_reference, flat_id, texture_id};
pub use model2::enhance::TexFilter;
use model2::enhance::{Found, PreloadKind, Rgba, TexturePack};
pub use model2::light::{Shading, CREASE_AUTO};
pub use model2::raster::Relief;
use model2::raster::{tex_info, Placed, Poly, Sources, TexKeys};
use model2::video::{Layers2d, View, HEIGHT, WIDTH};
pub use photo::{catch, GpuFault, Look as PhotoLook, Request as PhotoRequest, Shot as PhotoShot};
use std::collections::HashMap;
pub use temporal::{step, temporal_supported, BusMotion, FrameTag, Step, TemporalDebug, Tuning as TemporalTuning};
use temporal::{Temporal, TemporalPass};
/// The wgpu this crate is built with (for callers that make their own targets).
pub use wgpu;
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
    /// Temporal anti-aliasing: the render scale of the 3D layer in percent
    /// (0: off, the default; 100: anti-aliasing at the output size; 50 to
    /// 99: drawn smaller and upscaled). The 3D layer is drawn with a
    /// sub-pixel jitter and accumulated over frames, following motion
    /// ([`Renderer::present_bus_motion`]). Excludes `msaa` (set to 1).
    pub temporal: u8,
    /// Checkerboard translucency with `temporal` on (ignored without).
    pub stipple: Stipple,
    /// Smooth shading (`model2::light`): `Flat` (the default, the
    /// original), or brightness varying across polygons: `Vertex` from
    /// corner values (the game's formula per corner where the side
    /// channel allows, else means of the stream's lumas), `Pixel` the
    /// game's formula per pixel for the interpolated normal where the side
    /// channel allows, `Vertex` elsewhere ([`Renderer::light_effective`]).
    /// Polygons passed in ([`Renderer::draw`], [`Renderer::frame`],
    /// [`Renderer::present`]) are drawn with whatever `Poly::lit` (and,
    /// with `Pixel`, `Poly::pixel`) holds unless this is `Flat`; the
    /// `present_bus*` calls shade the stream themselves
    /// ([`Renderer::set_light_rules`], [`Renderer::set_light_frame`]).
    pub shading: Shading,
    /// The crease angle for `shading`, degrees (0: nothing smoothed), or
    /// [`CREASE_AUTO`] (the default): the game's (`romset::Shading`).
    pub crease: u8,
    /// Finer levels for `shading` (default off): a lit pixel's colour
    /// blends the two colour table columns around its brightness
    /// (`model2::raster::fine_level`) instead of taking one of the game's
    /// 64 levels, so large smooth surfaces don't band. Only lit pixels;
    /// never outside the polygon's hue.
    pub shading_fine: bool,
    /// Brightness balance for `shading` (default on,
    /// `model2::light::Options::balance`): one brightness offset per
    /// smooth surface keeps its mean the game's. Off: the game's formula
    /// at the smoothed normals alone (bumpy surfaces come out brighter).
    pub shading_balance: bool,
    /// Material maps of the texture pack (default off): with a pack that
    /// has them (`TexturePack::has_materials`), the normal maps of drawn
    /// textures change the brightness of polygons the game lit with the
    /// exact lighting layer (`Poly::lamp`: the native geometrizer's side
    /// channel; `model2::raster::material_luma`), as `materials_relief`
    /// says. Their maps are looked up, uploaded and kept under the pack's
    /// memory budget ([`PackStats`]). Polygons passed in are drawn with
    /// whatever `Poly::lamp` holds; the `present_bus*` calls ask the
    /// lighting pass for lamps themselves. Where there is no lamp (the
    /// chip, no side channel) nothing changes. Off, nothing of them is
    /// asked for, decoded or uploaded.
    pub materials: bool,
    /// How material maps change brightness ([`Relief`]): relief at
    /// strength 2 (the default), another strength, or the faithful rule.
    pub materials_relief: Relief,
    /// Bloom strength in percent (0: off, the default; 100: the strongest):
    /// the bright parts of the 3D layer glow, blurred through a chain of
    /// halvings and added over the frame before the HUD. Works with
    /// multisampling and temporal anti-aliasing (it takes the finished 3D
    /// layer at output size). Needs the 16-bit float targets temporal
    /// anti-aliasing does ([`temporal_supported`]); off without. GPU only.
    pub bloom: u8,
    /// The CRT look's strength in percent (0: off, the default; 100: the
    /// strongest): the finished picture (3D, 2D layers and the game's own
    /// HUD) as an arcade monitor shows it, with scanlines for each of the
    /// game's 384 lines, a mask, glow, a vignette and a slight curve
    /// (`crt.wgsl`). A pass over the composed frame: it changes nothing
    /// the machine does, and nothing else about the picture. GPU only.
    pub crt: u8,
}

impl Default for Quality {
    fn default() -> Self {
        Quality {
            msaa: 1,
            depth: false,
            tex_filter: TexFilter::Off,
            temporal: 0,
            stipple: Stipple::Blend,
            shading: Shading::Flat,
            crease: CREASE_AUTO,
            shading_fine: false,
            shading_balance: true,
            materials: false,
            materials_relief: Relief::default(),
            bloom: 0,
            crt: 0,
        }
    }
}

impl Quality {
    /// Whether smooth shading can light anything: not flat, and a crease
    /// angle above 0 (`shading`, `crease` as in [`Quality`]).
    pub fn shading_lit(shading: Shading, crease: u8) -> bool {
        shading != Shading::Flat && crease != 0
    }

    /// Whether the geometrizer's side channel (`Geometrizer::set_light`) is
    /// wanted: smooth shading, or material maps (their lamps come from it,
    /// with shading flat too).
    pub fn side_channel_on(&self) -> bool {
        Self::shading_lit(self.shading, self.crease) || self.materials
    }

    /// A `bloom` setting by name: `off`, or a strength in percent, 1 to 100.
    pub fn parse_bloom(s: &str) -> Option<u8> {
        match s {
            "off" | "0" => Some(0),
            n => n.parse::<u8>().ok().filter(|n| (1..=100).contains(n)),
        }
    }

    /// A `crt` setting by name: `off`, or a strength in percent, 1 to 100.
    pub fn parse_crt(s: &str) -> Option<u8> {
        Self::parse_bloom(s)
    }

    /// A `temporal` setting by name: `off`, `taa` (100) or a render scale
    /// in percent, 50 to 100.
    pub fn parse_temporal(s: &str) -> Option<u8> {
        match s {
            "off" | "0" => Some(0),
            "taa" | "on" => Some(100),
            n => n.parse::<u8>().ok().filter(|n| (50..=100).contains(n)),
        }
    }
}

/// How checkerboard (screen door) translucency is drawn with temporal
/// anti-aliasing on (docs/enhancements.md).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Stipple {
    /// As an even 50% blend: what the pattern averages to, stable at any
    /// render scale.
    #[default]
    Blend,
    /// The pattern, by game pixel at the jittered sample point. Falls back
    /// to `Blend` where the 3D layer has under 1.5 pixels a game pixel,
    /// where the pattern can't be reconstructed.
    Keep,
}

impl Stipple {
    /// `blend` or `keep`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "blend" => Some(Stipple::Blend),
            "keep" => Some(Stipple::Keep),
            _ => None,
        }
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
    /// With material maps (`fs_mat`, `fs_mat_depth`; group 1: the image or
    /// a stand-in, and the material): only for frames where a polygon with
    /// a lamp has a material. Lit polygons too (no `_l` twin).
    Mat,
    /// The same filtered (`vs_pack`, `fs_matf`, `fs_matf_depth`; group 1
    /// adds the sampler).
    MatFiltered,
}

impl Variant {
    /// Index into the temporal pipeline layouts.
    fn index(self) -> usize {
        match self {
            Variant::Original => 0,
            Variant::Pack => 1,
            Variant::PackFiltered => 2,
            Variant::Mat => 3,
            Variant::MatFiltered => 4,
        }
    }

    fn material(self) -> bool {
        matches!(self, Variant::Mat | Variant::MatFiltered)
    }
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
    /// Of the replaced, those drawn with another key's image (a stand-in,
    /// `model2::packlog::Fixes`).
    pub stood_in: u32,
    /// Images uploaded this frame ahead of their first draw
    /// (`Fixes::preload`; not in `uploaded`).
    pub preloaded: u32,
    /// Distinct pack images drawn.
    pub images: u32,
    /// Draw calls of polygons (runs), with or without a pack.
    pub draws: u32,
    /// Images uploaded this frame; images and bytes held on the GPU.
    pub uploaded: u32,
    pub held: u32,
    pub bytes: u64,
    /// Tile patterns (map entries) drawn replaced in the last frame
    /// composed, tile images held, and the tile atlas's bytes (allocated
    /// whole when the first tile image is drawn).
    pub tiles: u32,
    pub tiles_held: u32,
    pub tile_bytes: u64,
    /// With [`Quality::materials`]: distinct materials drawn (on polygons
    /// with a lamp), uploaded this frame, and held on the GPU with their
    /// bytes (inside the same budget as the images, `bytes` not included).
    pub materials: u32,
    pub material_uploaded: u32,
    pub material_held: u32,
    pub material_bytes: u64,
    /// With [`Quality::materials`] and a pack that has maps: textured
    /// polygons the game lit with the exact lighting layer (a lamp and a
    /// tangent frame: the only ones a material can change), and of them
    /// those drawn with a material. 0 lamped while `textured` isn't: no
    /// polygon had the exact layer (the chip, no side channel, every block
    /// demoted by the self-check).
    pub lamped: u32,
    pub material_polys: u32,
}

/// GPU memory pack images may hold before the least recently drawn are
/// dropped.
#[cfg(not(target_arch = "wasm32"))]
const PACK_BUDGET: u64 = model2::pack::BUDGET_NATIVE;
#[cfg(target_arch = "wasm32")]
const PACK_BUDGET: u64 = model2::pack::BUDGET_WASM;

/// What the renderer remembers of the pack's textures between frames, for
/// `Fixes` and the diagnostic.
#[derive(Default)]
struct PackMemory {
    /// By sheet rectangle (bank, x, y, w, h): what it drew last.
    rects: HashMap<[u32; 5], RectLast>,
    /// A key whose image the pack has, by surface (learned as they are
    /// drawn): stands in for the surface's other keys.
    surface_image: HashMap<u64, u64>,
    /// Keys dropped over the budget (a later miss is "evicted").
    evicted: std::collections::HashSet<u64>,
    /// Images uploaded without a draw (`Fixes::preload`).
    preloaded: u32,
}

/// What a sheet rectangle last drew.
#[derive(Clone, Copy)]
struct RectLast {
    key: u64,
    surface: u64,
    /// The key of the last image drawn for its own key there, and the
    /// frame.
    image: Option<u64>,
    hit_frame: u64,
}

/// No pack image: a polygon drawn from its own texels.
const NO_SLOT: u32 = u32::MAX;

/// What a polygon binds in group 1: its image's texture key, its material's
/// id (None: none, a stand-in) and the address modes of a filtered image.
type SlotKey = (Option<u64>, Option<u64>, usize);

/// What a frame's polygons bind in group 1 (`Renderer::pack_slots`).
struct Slots {
    /// Per placed polygon: the slot of its bind group in `groups`
    /// (NO_SLOT: none, drawn from its own texels without a material),
    /// whether it is drawn from a pack image, and its material's normal
    /// scale when one is bound (the material pipelines only).
    slot: Vec<u32>,
    image: Vec<bool>,
    material: Vec<Option<f32>>,
    groups: Vec<wgpu::BindGroup>,
    /// Per slot: a material's (the material pipelines' group 1), else an
    /// image's (the pack pipelines').
    mat_slot: Vec<bool>,
    /// The pipelines of polygons without a material: what the frame would
    /// use with materials off (Original, Pack, PackFiltered).
    variant: Variant,
    /// The pipelines of polygons with a material (Mat, MatFiltered), when
    /// the frame has any.
    mat_variant: Option<Variant>,
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

/// `light::shade`'s result for a stream, side channel and options (a
/// repeated draw of one game frame shades once).
struct LightCache {
    stream: Vec<u32>,
    side: Option<std::sync::Arc<model2::light::Frame>>,
    opts: model2::light::Options,
    shaded: std::sync::Arc<model2::light::Shaded>,
}

/// Where a draw's host time went, summed over the draws since the last
/// [`Renderer::take_phases`] (`Renderer::set_profile`; off by default).
#[derive(Clone, Copy, Debug, Default)]
pub struct Phases {
    /// Draws summed.
    pub draws: u32,
    /// Smooth shading's lighting of the frame (a repeat costs nothing).
    pub shade: std::time::Duration,
    /// Polygons built from the display list.
    pub build: std::time::Duration,
    /// The HUD's polygons moved, the polygons clipped and placed.
    pub place: std::time::Duration,
    /// The 2D layers (tiles, palette) made on the CPU, or found the same
    /// as the draw before's.
    pub layers: std::time::Duration,
    /// Draws that found them the same (`layers` is then the comparing).
    pub layers_reused: u32,
    /// `encode_3d` before its buffers: the vertices, the polygon table,
    /// runs, pack slots.
    pub prep3d: std::time::Duration,
    /// `encode_3d`'s buffers, bind group and pipelines.
    pub resources3d: std::time::Duration,
    /// `encode_3d`'s render pass.
    pub pass3d: std::time::Duration,
    /// The rest of `encode_frame`: temporal's resolve, bloom, the 2D
    /// uploads, the compose pass.
    pub rest: std::time::Duration,
    /// The look's IDs and tags of the polygons (inside `prep3d`).
    pub look: std::time::Duration,
    /// The look's ID pass's own preparation (inside `resources3d`).
    pub look_id: std::time::Duration,
    /// `Queue::submit`.
    pub submit: std::time::Duration,
}

/// What the renderer holds on the GPU, from the driver allocator's own
/// accounting ([`Renderer::memory`]).
#[derive(Clone, Debug, Default)]
pub struct MemoryReport {
    /// Bytes in live allocations.
    pub allocated: u64,
    /// Bytes in the memory blocks they come from (what the process holds).
    pub reserved: u64,
    /// Live allocations by label: bytes and count, largest first.
    pub by_label: Vec<(String, u64, u32)>,
}

impl MemoryReport {
    /// One line: the totals and the largest labels.
    pub fn line(&self) -> String {
        let mb = |b: u64| b as f64 / 1048576.0;
        let mut s = format!("{:.1} MB allocated, {:.1} MB reserved", mb(self.allocated), mb(self.reserved));
        for (label, bytes, n) in self.by_label.iter().take(12) {
            if *bytes >= 262_144 {
                s += &format!("; {label} {:.1} MB", mb(*bytes));
                if *n > 1 {
                    s += &format!(" (x{n})");
                }
            }
        }
        s
    }
}

/// What was made of the last frame's tile RAM and palette: the tile planes
/// (shared by the HUD layout and the 2D layers) and the 2D layers for the
/// view and 3D they were made for.
struct TileCache {
    tiles: Vec<u8>,
    palette: Vec<u8>,
    planes: std::sync::Arc<model2::video::Planes>,
    layers: Option<CachedLayers>,
}

struct CachedLayers {
    /// The view's part in them (`pad`, `sx`, `sy`, `hud`) and whether the
    /// frame has 3D.
    view: (f32, f32, f32, model2::video::Hud),
    has_3d: bool,
    layers: std::sync::Arc<Layers2d>,
    /// Which layers these are (for `Renderer::uploaded`).
    generation: u64,
}

/// Frames a kept texture may go unused before it is dropped.
const KEEP_FRAMES: u64 = 120;

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    raster_shader: wgpu::ShaderModule,
    raster_pipeline_layout: wgpu::PipelineLayout,
    /// Raster pipelines by sample count, mode, variant, temporal (the
    /// `_t` entry points, two targets) and shading (1: the `_l` entry
    /// points, for frames with a smooth-shaded polygon), made when first
    /// needed.
    raster: HashMap<(u32, Mode, Variant, bool, u8), wgpu::RenderPipeline>,
    /// Smooth shading's rules for the `present_bus*` calls: the game's,
    /// and its HUD windows (a bit each).
    light_rules: (&'static romset::Shading, u64),
    /// The last frame's `light::shade` counts (None: not shaded).
    light_stats: Option<model2::light::Stats>,
    /// The lighting side channel for the next frame `build_polys` shades
    /// ([`Renderer::set_light_frame`], or the engine's in `present_bus*`).
    light_side: Option<std::sync::Arc<model2::light::Frame>>,
    /// The last frame's shading, kept for a repeat of the same frame.
    light_cache: Option<LightCache>,
    /// Group 0 of the temporal pipelines (`raster_layout` and binding 5),
    /// and their pipeline layouts by variant.
    raster_layout_t: wgpu::BindGroupLayout,
    pipeline_layouts_t: [wgpu::PipelineLayout; 5],
    /// Temporal anti-aliasing state (`Quality::temporal`).
    temporal: Temporal,
    quality: Quality,
    max_msaa: u32,
    raster_layout: wgpu::BindGroupLayout,
    compose_shader: wgpu::ShaderModule,
    compose_layout: wgpu::BindGroupLayout,
    /// Bloom's passes and the sampler the compose pass reads it with.
    bloom: bloom::Bloom,
    bloom_sampler: wgpu::Sampler,
    /// The CRT pass and the wipe, and when they last drew.
    crt: crt::Crt,
    crt_used: u64,
    /// The compose pipeline for the last output format used.
    compose: Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    adapter: String,
    /// Render RAM and palette RAM on the GPU, with the versions uploaded.
    ram: [Option<(Option<u64>, wgpu::Buffer)>; 2],
    /// Versions for the next frame's RAM (see `set_ram_versions`).
    versions: Option<(u64, u64)>,
    /// The level the next frame shows as black (`Model2Bus::black_level`,
    /// 0-255): the bus's for its own frames, 0 for frames handed in.
    black: f32,
    /// Textures kept between frames, by label.
    textures: Vec<(&'static str, wgpu::Texture)>,
    /// `frames` when each kept texture was last asked for (by label): one
    /// unused for `KEEP_FRAMES` frames is dropped (`evict_unused`), so a
    /// feature turned off gives its memory back.
    texture_used: HashMap<&'static str, u64>,
    /// Frames encoded (`encode_frame`), for `texture_used`.
    frames: u64,
    /// `frames` when bloom's levels were last used.
    bloom_used: u64,
    /// The tile planes and 2D layers of the last frame, made again only
    /// when what they come from changed (a repeat, smooth motion's
    /// in-betweens).
    tile_cache: Option<TileCache>,
    layers_generations: u64,
    /// The generation of the cached layers the next `encode_frame` draws
    /// (one-shot), and the generation and texture each of the `base` and
    /// `hud` textures last got: unchanged layers are not uploaded again.
    layers_generation: Option<u64>,
    uploaded: [Option<(u64, wgpu::Texture)>; 2],
    /// `Phases` are timed (`set_profile`).
    profile: bool,
    phases: Phases,
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
    /// Group 1 of the material pipelines (`Variant::Mat`, `MatFiltered`):
    /// the image, the sampler (filtered only), the material.
    mat_layout: wgpu::BindGroupLayout,
    mat_pipeline_layout: wgpu::PipelineLayout,
    mat_filtered_layout: wgpu::BindGroupLayout,
    mat_filtered_pipeline_layout: wgpu::PipelineLayout,
    /// The luma table indices of textures (`raster::texel_indices`), kept
    /// across frames while render RAM is unchanged.
    texels: std::cell::RefCell<model2::raster::TexelCache>,
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
    /// The pack's material maps on the GPU (`Quality::materials`).
    materials: materials::Materials,
    /// GPU memory pack images and materials may hold together
    /// (`PACK_BUDGET`; smaller in tests).
    pack_budget: u64,
    /// The pack's tile replacements (tiles.rs).
    tiles: tiles::Tiles,
    /// The stabilisations of replacements in effect (`set_pack_fixes`) and
    /// what they remember between frames.
    fixes: model2::packlog::Fixes,
    stable: PackMemory,
    /// The pack diagnostic being written (`set_pack_log`).
    log: Option<model2::packlog::PackLog>,
    /// Photo mode's passes (photo.rs), made when first used.
    photo: Option<Box<photo::Photo>>,
    /// While a photo is drawn: leave out the HUD (tile HUD and HUD
    /// windows) and the 2D background.
    photo_hide: (bool, bool),
    /// While a photo's last image is drawn: its depth (1/z) and depth test
    /// targets, drawn by `encode_frame`.
    photo_depth: Option<(wgpu::TextureView, wgpu::TextureView)>,
    /// Bloom's chain this many levels longer (a photo larger than play).
    bloom_extra: u32,
    /// The look (`set_look`), its tags by key, and its ink pass.
    look: Option<std::sync::Arc<model2::look::Look>>,
    look_tags: look::LookTags,
    ink: look::Ink,
    /// Raster pipelines of the look's ID pass, by mode and variant.
    raster_id: HashMap<(Mode, Variant), wgpu::RenderPipeline>,
    /// While the ID pass is encoded: no material maps (it only needs
    /// cut-outs and depth).
    no_materials: bool,
    /// `frames` when the look's targets were last used.
    look_used: u64,
    /// The look's ID pass will follow this frame's 3D pass (`encode_frame`):
    /// the 3D pass keeps what the ID pass can reuse (`look::IdShare`).
    share_ids: bool,
    /// SPIKE: the ID pass's tag carries each polygon's block number (+1) in bits 18+.
    pub pick_blocks: bool,
    /// The scene's typical distance, steadied (`look::depth_reference`).
    depth_ref: Option<f32>,
    id_share: Option<look::IdShare>,
    /// While a photo is drawn: its near plane (`raster::BuildIn::near`).
    photo_near: f32,
}

/// Bytes per vertex: x, y, 1/z, u/z, v/z, polygon index.
const VERTEX_BYTES: u64 = 24;
/// Bytes per vertex of the temporal pipelines: the same, then the previous
/// position (`Placed::prev`, homogeneous X, Y, W).
const VERTEX_BYTES_T: u64 = 36;
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

/// A texture read with a filtering sampler.
fn filterable(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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

/// A fragment entry point's smooth-shading twin when `lit`: `fs` ->
/// `fs_l`, `fs_t` -> `fs_l_t` (raster.wgsl).
fn lit_entry(fs: &str, lit: bool) -> String {
    match (lit, fs.strip_suffix("_t")) {
        (false, _) => fs.to_string(),
        (true, Some(base)) => format!("{base}_l_t"),
        (true, None) => format!("{fs}_l"),
    }
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

#[cfg(test)]
impl Renderer {
    /// A renderer for a test, or None (said on stderr) where there is no
    /// GPU adapter, so the test skips.
    pub(crate) fn for_test() -> Option<Self> {
        // `make check GPU_TESTS=0`: the GPU is busy (or its memory full),
        // so the tests that need one return early, as without an adapter.
        if std::env::var("REMODEL_GPU_TESTS").is_ok_and(|v| v == "0") {
            eprintln!("GPU tests off (REMODEL_GPU_TESTS=0): skipped");
            return None;
        }
        let r = Self::new().ok();
        if r.is_none() {
            eprintln!("no GPU adapter: skipped");
        }
        r
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
            // Blocks of 4 to 8 MB for the small things; a larger texture
            // gets a block of its own, its size exactly. (The default holds
            // 192 MB in blocks of 128 and 64 MB before the first texture,
            // and `MemoryUsage`'s 8 to 64 MB blocks hold up to one more
            // each of what doesn't fill them.)
            memory_hints: wgpu::MemoryHints::Manual { suballocated_device_memory_block_size: (4 << 20)..(8 << 20) },
            ..Default::default()
        }))
        .map_err(|e| format!("{name}: {e}"))?;
        let mut r = Self::from_device(device, queue, name);
        r.max_msaa = max_msaa(&adapter);
        r.max_anisotropy = max_anisotropy(&adapter);
        r.temporal.supported = temporal_supported(&adapter);
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
        let mat_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material"),
            entries: &[texture(0), texture(2)],
        });
        let mat_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("raster material"),
            bind_group_layouts: &[Some(&raster_layout), Some(&mat_layout)],
            immediate_size: 0,
        });
        let [f0, f1] = filtered_entries();
        let mat_filtered_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material filtered"),
            entries: &[f0, f1, wgpu::BindGroupLayoutEntry { binding: 2, ..f0 }],
        });
        let mat_filtered_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("raster material filtered"),
            bind_group_layouts: &[Some(&raster_layout), Some(&mat_filtered_layout)],
            immediate_size: 0,
        });
        let raster_layout_t = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("raster t"),
            entries: &[
                uniform(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
                storage(1),
                storage(2),
                storage(3),
                storage(4),
                uniform(5, wgpu::ShaderStages::VERTEX_FRAGMENT),
            ],
        });
        let pipeline_layouts_t =
            [None, Some(&pack_layout), Some(&pack_filtered_layout), Some(&mat_layout), Some(&mat_filtered_layout)].map(
                |g1| {
                    let groups = [Some(&raster_layout_t), g1];
                    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("raster t"),
                        bind_group_layouts: if g1.is_some() { &groups } else { &groups[..1] },
                        immediate_size: 0,
                    })
                },
            );
        let temporal = Temporal::new(&device);
        let compose_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("compose"),
            source: wgpu::ShaderSource::Wgsl(include_str!("compose.wgsl").into()),
        });
        let compose_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("compose"),
            entries: &[
                uniform(0, wgpu::ShaderStages::FRAGMENT),
                texture(1),
                texture(2),
                texture(3),
                filterable(4),
                sampler(5),
            ],
        });
        let tiles = tiles::Tiles::new(&device);
        let bloom = bloom::Bloom::new(&device);
        let crt = crt::Crt::new(&device);
        let ink = look::Ink::new(&device);
        let bloom_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bloom"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Renderer {
            device,
            queue,
            raster_shader: shader,
            raster_pipeline_layout: pipeline_layout,
            raster: HashMap::new(),
            light_rules: (&romset::Shading::DEFAULT, 0),
            light_stats: None,
            light_side: None,
            light_cache: None,
            raster_layout_t,
            pipeline_layouts_t,
            temporal,
            quality: Quality::default(),
            max_msaa: 4,
            raster_layout,
            compose_shader,
            compose_layout,
            bloom,
            bloom_sampler,
            crt,
            crt_used: 0,
            compose: None,
            adapter,
            ram: [None, None],
            versions: None,
            black: 0.0,
            textures: Vec::new(),
            texture_used: HashMap::new(),
            frames: 0,
            bloom_used: 0,
            tile_cache: None,
            layers_generations: 0,
            layers_generation: None,
            uploaded: [None, None],
            profile: false,
            phases: Phases::default(),
            pack: None,
            keys: TexKeys::default(),
            pack_tex: HashMap::new(),
            pack_layout,
            pack_pipeline_layout,
            pack_filtered_layout,
            pack_filtered_pipeline_layout,
            mat_layout,
            mat_pipeline_layout,
            mat_filtered_layout,
            mat_filtered_pipeline_layout,
            texels: Default::default(),
            samplers: None,
            max_anisotropy: 16,
            pack_dummy: [None, None],
            pack_stats: PackStats::default(),
            frame: 0,
            pack_warned: false,
            materials: Default::default(),
            pack_budget: PACK_BUDGET,
            fixes: Default::default(),
            stable: PackMemory::default(),
            log: None,
            tiles,
            photo: None,
            photo_hide: (false, false),
            photo_depth: None,
            bloom_extra: 0,
            photo_near: 0.0,
            look: None,
            look_tags: Default::default(),
            ink,
            raster_id: HashMap::new(),
            no_materials: false,
            look_used: 0,
            share_ids: false,
            pick_blocks: false,
            depth_ref: None,
            id_share: None,
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

    /// Time each draw's phases ([`Renderer::take_phases`]); off by default.
    pub fn set_profile(&mut self, on: bool) {
        self.profile = on;
    }

    /// The phase times summed since the last call (zero without
    /// [`Renderer::set_profile`]).
    pub fn take_phases(&mut self) -> Phases {
        std::mem::take(&mut self.phases)
    }

    /// The start of a timed phase: None unless profiling.
    fn tick(&self) -> Option<std::time::Instant> {
        self.profile.then(std::time::Instant::now)
    }

    /// Add the time since `t` to the phase `slot` picks.
    fn lap(&mut self, t: Option<std::time::Instant>, slot: fn(&mut Phases) -> &mut std::time::Duration) {
        if let Some(t) = t {
            *slot(&mut self.phases) += t.elapsed();
        }
    }

    /// What the GPU allocator holds for this renderer's device, by label
    /// (None where the backend can't tell: the browser's).
    pub fn memory(&self) -> Option<MemoryReport> {
        let r = self.device.generate_allocator_report()?;
        let mut by: HashMap<&str, (u64, u32)> = HashMap::new();
        for a in &r.allocations {
            let e = by.entry(a.name.as_str()).or_default();
            e.0 += a.size;
            e.1 += 1;
        }
        let mut by_label: Vec<(String, u64, u32)> = by.into_iter().map(|(l, (b, n))| (l.to_string(), b, n)).collect();
        by_label.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        Some(MemoryReport { allocated: r.total_allocated_bytes, reserved: r.total_reserved_bytes, by_label })
    }

    /// Drop the kept textures and bloom levels nothing has asked for in
    /// `KEEP_FRAMES` frames (a feature turned off, a size left behind).
    fn evict_unused(&mut self) {
        let (frames, used) = (self.frames, &self.texture_used);
        let before = self.textures.len();
        self.textures.retain(|(l, _)| frames.saturating_sub(used.get(l).copied().unwrap_or(0)) <= KEEP_FRAMES);
        if self.textures.len() != before {
            let kept: Vec<&'static str> = self.textures.iter().map(|(l, _)| *l).collect();
            self.texture_used.retain(|l, _| kept.contains(l));
        }
        if frames.saturating_sub(self.bloom_used) > KEEP_FRAMES {
            self.bloom.release();
        }
        if frames.saturating_sub(self.crt_used) > KEEP_FRAMES {
            self.crt.release();
        }
    }

    /// Draw the 3D layer with `quality` from the next frame on. The sample
    /// count is rounded to what exists (1, 4, 8) and limited to the GPU's,
    /// anisotropy likewise (2, 4, 8, 16; mipmaps alone where the GPU has
    /// none); the result is the quality in effect. `temporal` is clamped to
    /// 50..=100 (0 where the GPU can't, [`temporal_supported`]) and turns
    /// multisampling off; any change starts temporal accumulation again.
    pub fn set_quality(&mut self, quality: Quality) -> Quality {
        let temporal = match quality.temporal {
            _ if !self.temporal.supported => 0,
            0 => 0,
            n => n.clamp(50, 100),
        };
        let msaa = match quality.msaa {
            _ if temporal > 0 => 1,
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
            if let Some(p) = self.pack.as_mut() {
                p.preload_restart(PreloadKind::Texture);
            }
            for t in self.pack_tex.values_mut().flatten() {
                t.groups = Default::default();
            }
            self.materials.retain_form(mip);
        }
        if !quality.materials {
            self.materials.clear();
        }
        let bloom = if self.temporal.supported { quality.bloom.min(100) } else { 0 };
        let q = Quality { msaa, tex_filter, temporal, bloom, crt: quality.crt.min(100), ..quality };
        // The CRT pass is after everything the accumulation holds.
        if (Quality { crt: 0, ..q }) != (Quality { crt: 0, ..self.quality }) {
            self.temporal.reset();
        }
        self.quality = q;
        self.quality
    }

    /// Draw with `quality` for the duration of `f` (some `present_*` calls)
    /// and then as before. Unlike [`Renderer::set_quality`] this keeps all
    /// that the quality in effect holds (the temporal accumulation, the
    /// texture pack's samplers and materials), so a picture can be drawn
    /// both ways, each frame, without either one starting again: for the
    /// picture menu's before/after wipe. The texture filter stays the one
    /// in effect, and the multisampling is rounded to what the GPU has;
    /// temporal anti-aliasing is left as `quality` says only if it is off
    /// (the accumulation belongs to the quality in effect).
    pub fn with_quality<R>(&mut self, quality: Quality, f: impl FnOnce(&mut Self) -> R) -> R {
        let msaa = match quality.msaa {
            0 | 1 => 1,
            2..=7 => 4,
            _ => self.max_msaa.min(8),
        };
        let q = Quality { msaa, temporal: 0, tex_filter: self.quality.tex_filter, ..quality };
        let kept = std::mem::replace(&mut self.quality, q);
        // A picture drawn at another quality (the wipe's arcade original)
        // has no look.
        let look = self.look.take();
        let r = f(self);
        self.look = look;
        self.quality = kept;
        r
    }

    /// The picture a wipe ([`Renderer::wipe`]) draws from, to be drawn into
    /// (as a `present_*` target): `w` x `h` in `format`, kept between calls.
    pub fn wipe_source(&mut self, w: u32, h: u32, format: wgpu::TextureFormat) -> wgpu::TextureView {
        self.crt_used = self.frames;
        self.crt.before(&self.device, w, h, format)
    }

    /// Copy the rectangle `rect` (x, y, width, height in pixels) of the
    /// [`Renderer::wipe_source`] picture over the same place in `target`
    /// (of `format`) and submit: the before side of a before/after
    /// comparison.
    pub fn wipe(&mut self, target: &wgpu::TextureView, format: wgpu::TextureFormat, rect: (u32, u32, u32, u32)) {
        let mut enc = self.device.create_command_encoder(&Default::default());
        self.crt.wipe(&self.device, &mut enc, target, format, rect);
        self.queue.submit([enc.finish()]);
    }

    /// Whether [`Quality::temporal`] can be used (from
    /// [`temporal_supported`] for the device's adapter; assumed for a
    /// device from [`Renderer::from_device`], as on WebGPU).
    pub fn set_temporal_supported(&mut self, ok: bool) {
        self.temporal.supported = ok;
        if !ok && self.quality.temporal > 0 {
            self.set_quality(Quality { temporal: 0, ..self.quality });
        }
    }

    /// Start temporal accumulation again from the next frame (a jump in
    /// the picture the motion data doesn't know of: a state load, another
    /// machine shown, a game start).
    pub fn reset_history(&mut self) {
        self.temporal.reset();
    }

    /// How a frame tagged `tag` follows the last one drawn with temporal
    /// on ([`step`]); `cut` from its motion data (`smooth::Motion::cut`).
    pub fn temporal_step(&self, tag: FrameTag, cut: bool) -> Step {
        step(self.temporal.last, tag, cut)
    }

    /// For the next [`Renderer::present`] or [`Renderer::frame`] with
    /// temporal on (one-shot, like `set_ram_versions`): the frame's tag,
    /// its step ([`Renderer::temporal_step`]; with `Step::Blend { tp }`
    /// the caller has built the polygons with `raster::build_motion` at
    /// that `tp` and these centres as the previous ones), and its window
    /// centres (from `build_motion`). Without it a frame has no motion
    /// data (`Step::NoMotion`).
    pub fn set_temporal_frame(&mut self, tag: FrameTag, step: Step, centres: Vec<model2::raster::Centres>) {
        self.temporal.pending = Some((Some(tag), step, centres));
    }

    /// Test hook: jitter on (the default) or off (every frame drawn at the
    /// pixel centres, as the first frame after a reset is).
    pub fn set_temporal_jitter(&mut self, on: bool) {
        self.temporal.jitter = on;
    }

    /// Test hook: the resolve copies each output pixel's nearest internal
    /// sample and ignores the history (stage 3's pass-through), when on.
    pub fn set_temporal_passthrough(&mut self, on: bool) {
        self.temporal.passthrough = on;
    }

    /// The resolve's constants (for tuning; the defaults are the settled
    /// ones).
    pub fn set_temporal_tuning(&mut self, t: TemporalTuning) {
        self.temporal.tuning = t;
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
        self.materials.clear();
        self.keys.clear();
        self.tiles.reset();
        self.stable = PackMemory::default();
        self.pack_stats = PackStats::default();
    }

    /// Which stabilisations of the pack's replacements are on (default all;
    /// `model2::packlog::Fixes`), to compare a run with and without.
    pub fn set_pack_fixes(&mut self, fixes: model2::packlog::Fixes) {
        self.fixes = fixes;
        if !fixes.surface && !fixes.hold {
            self.stable.rects.clear();
        }
        self.tiles.set_fixes(fixes);
    }

    /// Write the pack diagnostic (`model2::packlog`): for each frame drawn,
    /// what the pack's lookups gave.
    pub fn set_pack_log(&mut self, log: Option<model2::packlog::PackLog>) {
        self.log = log;
    }

    /// The game frame the next draws are for (the diagnostic's block).
    pub fn set_log_frame(&mut self, frame: u64) {
        if let Some(l) = self.log.as_mut() {
            l.set_frame(frame);
            l.begin();
        }
    }

    /// Images uploaded ahead of their first draw so far (`Fixes::preload`).
    pub fn preloaded(&self) -> u32 {
        self.stable.preloaded
    }

    /// The tile planes of `tiles` and `palette`, worked out again only if
    /// they differ from the last frame's.
    fn tile_cache(&mut self, tiles: &[u8], palette: &[u8]) -> &mut TileCache {
        let same = self.tile_cache.as_ref().is_some_and(|c| c.tiles == tiles && c.palette == palette);
        if !same {
            self.tile_cache = Some(TileCache {
                tiles: tiles.to_vec(),
                palette: palette.to_vec(),
                planes: std::sync::Arc::new(model2::video::Planes::of(tiles, palette)),
                layers: None,
            });
        }
        self.tile_cache.as_mut().unwrap()
    }

    /// `HudLayout::new` from the cached tile planes (which the 2D layers of
    /// the same frame start from too).
    fn hud_layout(&mut self, tiles: &[u8], palette: &[u8], view: &View) -> model2::video::HudLayout {
        if !model2::video::HudLayout::needed(view) {
            return Default::default();
        }
        let planes = self.tile_cache(tiles, palette).planes.clone();
        model2::video::HudLayout::with_planes(&planes, view)
    }

    /// [`Renderer::layers_2d`] for the draws: the layers of the last frame
    /// again if the tiles, palette, view and 3D are the same (not with a
    /// pack's tile images or a photo's hidden layers, which change what
    /// they make). Sets `layers_generation` for `encode_frame`.
    fn layers_cached(&mut self, tiles: &[u8], palette: &[u8], view: &View, has_3d: bool) -> std::sync::Arc<Layers2d> {
        let pack_tiles = self.pack.as_ref().is_some_and(|p| p.has_tiles());
        if pack_tiles || self.photo_hide != (false, false) {
            self.tile_cache = None;
            self.layers_generation = None;
            return std::sync::Arc::new(self.layers_2d(tiles, palette, view, has_3d));
        }
        let key = (view.pad, view.sx, view.sy, view.hud);
        let cache = self.tile_cache(tiles, palette);
        let hit = cache
            .layers
            .as_ref()
            .filter(|c| c.view == key && c.has_3d == has_3d)
            .map(|c| (c.generation, c.layers.clone()));
        let planes = cache.planes.clone();
        if let Some((generation, layers)) = hit {
            self.layers_generation = Some(generation);
            self.phases.layers_reused += self.profile as u32;
            return layers;
        }
        let layers = std::sync::Arc::new(model2::video::layers_2d_with(&planes, tiles, palette, view, has_3d));
        self.layers_generations += 1;
        let generation = self.layers_generations;
        if let Some(c) = self.tile_cache.as_mut() {
            c.layers = Some(CachedLayers { view: key, has_3d, layers: layers.clone(), generation });
        }
        self.layers_generation = Some(generation);
        layers
    }

    /// The 2D layers of a frame (`model2::video::layers_2d`), with the
    /// pack's tile images (`tile_<key>.png`) in place of the patterns they
    /// replace when it has any: those are then drawn at output resolution
    /// by `present`, `frame` and the `present_bus*` calls (which use this
    /// themselves). Without a pack, or with one that has no tile images,
    /// exactly `layers_2d`.
    pub fn layers_2d(&mut self, tiles: &[u8], palette: &[u8], view: &View, has_3d: bool) -> Layers2d {
        let mut layers = match self.pack.as_mut() {
            Some(pack) if pack.has_tiles() => self.tiles.layers(
                &self.device,
                &self.queue,
                pack.as_mut(),
                tiles,
                palette,
                view,
                has_3d,
                self.log.as_mut(),
            ),
            _ => model2::video::layers_2d(tiles, palette, view, has_3d),
        };
        // A photo without the HUD or the background: transparent, or the
        // backdrop colour (the original patterns' layers, not the pack's).
        let (hud, base) = self.photo_hide;
        if hud || base {
            if layers.tiles.is_some() {
                layers = model2::video::layers_2d(tiles, palette, view, has_3d);
            }
            if hud {
                layers.hud.fill(0);
            }
            if base {
                let b = layers.backdrop;
                layers.base.as_chunks_mut::<4>().0.iter_mut().for_each(|p| *p = [b[0], b[1], b[2], 255]);
            }
        }
        layers
    }

    /// A photo without the HUD: its HUD windows' polygons left out too.
    fn photo_drop_hud(&self, polys: &[Poly], placed: &mut Vec<Placed>, view: &View) {
        if self.photo_hide.0 {
            placed.retain(|p| {
                let w = polys[p.poly].window;
                w >= 32 || view.hud_windows >> w & 1 == 0
            });
        }
    }

    /// Whether a texture pack is set.
    pub fn has_pack(&self) -> bool {
        self.pack.is_some()
    }

    /// Texture pack counters for the last frame drawn.
    pub fn pack_stats(&self) -> PackStats {
        self.pack_stats
    }

    /// Smooth shading's rules for the frames `present_bus*` shades from
    /// the stream (with [`Quality::shading`] on): the game's
    /// (`romset::Game::shading`) and its HUD windows
    /// (`romset::Game::hud_windows`, a bit each), never shaded. Default:
    /// `romset::Shading::DEFAULT`, none.
    pub fn set_light_rules(&mut self, rules: &'static romset::Shading, hud_windows: u64) {
        self.light_rules = (rules, hud_windows);
    }

    /// `light::shade`'s counts for the last frame `present_bus*` drew with
    /// shading on (None otherwise).
    pub fn light_stats(&self) -> Option<model2::light::Stats> {
        self.light_stats
    }

    /// The lighting side channel (`model2::light::Frame`) of the geometry
    /// the next `present_bus*` call draws, when it is passed in (an
    /// in-between); used once. Without it the engine's own frame takes the
    /// engine's (`Engine::light_frame`, with `Geometrizer::set_light` on),
    /// other geometry none. A side channel that doesn't line up with the
    /// stream is dropped by `light::shade_with`'s check.
    pub fn set_light_frame(&mut self, side: Option<std::sync::Arc<model2::light::Frame>>) {
        self.light_side = side;
    }

    /// The options `present_bus*` shades the stream with (None: shading
    /// off).
    pub fn light_options(&self) -> Option<model2::light::Options> {
        let q = self.quality;
        let o = model2::light::Options::with_mask(q.shading, q.crease, self.light_rules.0, self.light_rules.1)
            .with_fine(q.shading_fine)
            .with_balance(q.shading_balance);
        o.on().then_some(o)
    }

    /// The options `build_polys` shades with: [`Renderer::light_options`],
    /// asking for lamps when material maps are on and the pack has some
    /// (then also with shading off).
    fn shade_options(&self) -> Option<model2::light::Options> {
        let q = self.quality;
        let lamps = q.materials && self.pack.as_ref().is_some_and(|p| p.has_materials());
        let o = model2::light::Options::with_mask(q.shading, q.crease, self.light_rules.0, self.light_rules.1)
            .with_fine(q.shading_fine)
            .with_balance(q.shading_balance)
            .with_lamps(lamps)
            .with_relief(q.materials_relief);
        (o.on() || lamps).then_some(o)
    }

    /// The polygons of `stream` as `present_bus*` draws them: with motion
    /// data `m`, smooth-shaded when [`Quality::shading`] is on, with lamps
    /// for material maps when [`Quality::materials`] is; `render_version`
    /// keeps what is worked out from render RAM across frames.
    fn build_polys(
        &mut self,
        stream: &[u32],
        src: &Sources,
        m: Option<&model2::raster::MotionIn>,
        render_version: Option<u64>,
    ) -> (Vec<Poly>, Vec<model2::raster::Centres>) {
        let t_shade = self.tick();
        let opts = self.shade_options();
        let side = self.light_side.take();
        let shaded = opts.map(|o| {
            // The same stream, side channel and options as the last frame
            // (a repeat): its shading again, not worked out again.
            let same = |c: &LightCache| {
                c.opts == o
                    && match (&c.side, &side) {
                        (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
                        (None, None) => true,
                        _ => false,
                    }
                    && c.stream[..] == *stream
            };
            match &self.light_cache {
                Some(c) if same(c) => (c.shaded.clone(), true),
                _ => {
                    let s = std::sync::Arc::new(model2::light::shade_with(stream, side.as_deref(), &o));
                    self.light_cache = Some(LightCache { stream: stream.to_vec(), side, opts: o, shaded: s.clone() });
                    (s, false)
                }
            }
        });
        // A repeat cost nothing. Counted with shading on only.
        self.light_stats = shaded
            .as_ref()
            .filter(|_| self.light_options().is_some())
            .map(|(s, hit)| model2::light::Stats { ms: if *hit { 0.0 } else { s.stats.ms }, ..s.stats });
        let shaded = shaded.map(|s| s.0);
        self.lap(t_shade, |p| &mut p.shade);
        let t_build = self.tick();
        self.texels.get_mut().begin(render_version);
        let input = model2::raster::BuildIn {
            motion: m,
            lit: shaded.as_deref(),
            skip_checker: opts.is_some_and(|o| o.skip_checker),
            fine: opts.is_some_and(|o| o.fine),
            relief: self.quality.materials_relief,
            texels: Some(&self.texels),
            near: self.photo_near,
        };
        let built = model2::raster::build_with(stream, src, &input);
        self.lap(t_build, |p| &mut p.build);
        built
    }

    /// What the last frame drawn with shading on was drawn as
    /// ([`model2::light::Stats::effective`]): `Pixel` asked for but no
    /// polygon lit per pixel (the chip, no side channel) is `Vertex`. None:
    /// shading off.
    pub fn light_effective(&self) -> Option<Shading> {
        Some(self.light_stats?.effective(self.light_options()?.shading))
    }

    /// The raster pipeline for `samples`, `mode` and `variant`, and
    /// `temporal`: the `_t` entry points, with the aux target; `shading`
    /// 1: the smooth-shading (`_l`) fragment entry points.
    fn raster_pipeline(
        &mut self,
        samples: u32,
        mode: Mode,
        variant: Variant,
        temporal: bool,
        shading: u8,
    ) -> wgpu::RenderPipeline {
        // The material entry points are the same lit or not.
        let shading = if variant.material() { 0 } else { shading };
        let key = (samples, mode, variant, temporal, shading);
        if let Some(p) = self.raster.get(&key) {
            return p.clone();
        }
        // The test is in favour of the game's order (raster::depth_test_value).
        let mut constants = Vec::new();
        if mode == Mode::Test {
            constants.push(("DEPTH_FACTOR", f64::from(1.0 + model2::raster::DEPTH_TOLERANCE)));
        }
        if temporal {
            let p = self.raster_pipeline_t(mode, variant, &constants, shading != 0);
            self.raster.insert(key, p.clone());
            return p;
        }
        let (vs, fs) = match (mode, variant) {
            (Mode::Clear, _) => ("vs_clear", "fs_clear"),
            (Mode::Test | Mode::DepthWrite, Variant::Original) => ("vs", "fs_depth"),
            (_, Variant::Original) => ("vs", "fs"),
            (Mode::Test | Mode::DepthWrite, Variant::Pack) => ("vs", "fs_pack_depth"),
            (_, Variant::Pack) => ("vs", "fs_pack"),
            (Mode::Test | Mode::DepthWrite, Variant::PackFiltered) => ("vs_pack", "fs_packf_depth"),
            (_, Variant::PackFiltered) => ("vs_pack", "fs_packf"),
            (Mode::Test | Mode::DepthWrite, Variant::Mat) => ("vs", "fs_mat_depth"),
            (_, Variant::Mat) => ("vs", "fs_mat"),
            (Mode::Test | Mode::DepthWrite, Variant::MatFiltered) => ("vs_pack", "fs_matf_depth"),
            (_, Variant::MatFiltered) => ("vs_pack", "fs_matf"),
        };
        // The material entry points light lit polygons themselves.
        let fs = lit_entry(fs, shading != 0 && mode != Mode::Clear && !variant.material());
        let layout = match variant {
            Variant::Original => &self.raster_pipeline_layout,
            Variant::Pack => &self.pack_pipeline_layout,
            Variant::PackFiltered => &self.pack_filtered_pipeline_layout,
            Variant::Mat => &self.mat_pipeline_layout,
            Variant::MatFiltered => &self.mat_filtered_pipeline_layout,
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
        let fs_options = wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() };
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
                entry_point: Some(&fs),
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
        self.raster.insert(key, p.clone());
        p
    }

    /// The look's ID pass pipeline for `mode` (Test, DepthWrite or Clear)
    /// and `variant` (Original, Pack or PackFiltered): the `fs_id*` entry
    /// points into the ID target and the 1/z target, with the depth rules
    /// of the 3D pass.
    fn id_pipeline(&mut self, mode: Mode, variant: Variant) -> wgpu::RenderPipeline {
        let key = (mode, variant);
        if let Some(p) = self.raster_id.get(&key) {
            return p.clone();
        }
        let mut constants = Vec::new();
        if mode == Mode::Test {
            constants.push(("DEPTH_FACTOR", f64::from(1.0 + model2::raster::DEPTH_TOLERANCE)));
        }
        let (vs, fs) = match (mode, variant) {
            (Mode::Clear, _) => ("vs_clear", "fs_id_clear"),
            (_, Variant::Original) => ("vs", "fs_id"),
            (_, Variant::Pack) => ("vs", "fs_id_pack"),
            (_, Variant::PackFiltered) => ("vs_pack", "fs_id_packf"),
            _ => unreachable!("the ID pass has no materials"),
        };
        let layout = match variant {
            Variant::Original => &self.raster_pipeline_layout,
            Variant::Pack => &self.pack_pipeline_layout,
            _ => &self.pack_filtered_pipeline_layout,
        };
        let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x3, 2 => Uint32];
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: VERTEX_BYTES,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        })];
        let depth_stencil = Some(wgpu::DepthStencilState {
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
        let writes = if matches!(mode, Mode::Clear | Mode::DepthWrite) {
            wgpu::ColorWrites::empty()
        } else {
            wgpu::ColorWrites::ALL
        };
        let targets = [look::ID_FORMAT, look::IZ_FORMAT]
            .map(|format| Some(wgpu::ColorTargetState { format, blend: None, write_mask: writes }));
        let p = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("raster id"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &self.raster_shader,
                entry_point: Some(vs),
                compilation_options: Default::default(),
                buffers: if mode == Mode::Clear { &[] } else { &buffers },
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.raster_shader,
                entry_point: Some(fs),
                compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
                targets: &targets,
            }),
            multiview_mask: None,
            cache: None,
        });
        self.raster_id.insert(key, p.clone());
        p
    }

    /// A temporal raster pipeline (one sample): `vs_t` and the `_t`
    /// fragment entry points, 36-byte vertices, the colour target with
    /// premultiplied blending (for blended stipple) and the aux target.
    fn raster_pipeline_t(
        &mut self,
        mode: Mode,
        variant: Variant,
        constants: &[(&str, f64)],
        lit: bool,
    ) -> wgpu::RenderPipeline {
        let depth = matches!(mode, Mode::Test | Mode::DepthWrite);
        let (vs, fs) = match (mode, variant) {
            (Mode::Clear, _) => ("vs_clear", "fs_clear"),
            (_, Variant::Original) => ("vs_t", if depth { "fs_depth_t" } else { "fs_t" }),
            (_, Variant::Pack) => ("vs_t", if depth { "fs_pack_depth_t" } else { "fs_pack_t" }),
            (_, Variant::PackFiltered) => ("vs_t", if depth { "fs_packf_depth_t" } else { "fs_packf_t" }),
            (_, Variant::Mat) => ("vs_t", if depth { "fs_mat_depth_t" } else { "fs_mat_t" }),
            (_, Variant::MatFiltered) => ("vs_t", if depth { "fs_matf_depth_t" } else { "fs_matf_t" }),
        };
        let fs = lit_entry(fs, lit && mode != Mode::Clear && !variant.material());
        let layout = &self.pipeline_layouts_t[variant.index()];
        let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x3, 2 => Uint32, 3 => Float32x3];
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: VERTEX_BYTES_T,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        })];
        let depth_stencil = (mode != Mode::Plain).then(|| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(matches!(mode, Mode::DepthWrite | Mode::Clear)),
            depth_compare: Some(if depth {
                wgpu::CompareFunction::GreaterEqual
            } else {
                wgpu::CompareFunction::Always
            }),
            stencil: Default::default(),
            bias: Default::default(),
        });
        let fs_options = wgpu::PipelineCompilationOptions { constants, ..Default::default() };
        let writes = if matches!(mode, Mode::Clear | Mode::DepthWrite) {
            wgpu::ColorWrites::empty()
        } else {
            wgpu::ColorWrites::ALL
        };
        let targets = [
            Some(wgpu::ColorTargetState {
                format: FORMAT,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: writes,
            }),
            Some(wgpu::ColorTargetState { format: temporal::AUX_FORMAT, blend: None, write_mask: writes }),
        ];
        self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("raster t"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &self.raster_shader,
                entry_point: Some(vs),
                compilation_options: Default::default(),
                buffers: if mode == Mode::Clear { &[] } else { &buffers },
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.raster_shader,
                entry_point: Some(&fs),
                compilation_options: fs_options,
                targets: &targets,
            }),
            multiview_mask: None,
            cache: None,
        })
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

    /// What each of `placed` binds in group 1 ([`Slots`]): looks textures up
    /// in the pack (once per key) and, with [`Quality::materials`], the
    /// materials of polygons with a lamp; uploads new images and materials,
    /// drops the least recently drawn over the budget and counts
    /// `pack_stats`.
    fn pack_slots(&mut self, polys: &[Poly], placed: &[Placed], src: &Sources, render_version: Option<u64>) -> Slots {
        let n = placed.len();
        let mut out = Slots {
            slot: vec![NO_SLOT; n],
            image: vec![false; n],
            material: vec![None; n],
            groups: Vec::new(),
            mat_slot: Vec::new(),
            variant: Variant::Original,
            mat_variant: None,
        };
        if self.pack.is_none() {
            return out;
        }
        let mip = self.quality.tex_filter.mipmapped();
        if mip && self.samplers.is_none() {
            self.samplers = Some(pack_samplers(&self.device, self.quality.tex_filter));
        }
        let Some(pack) = self.pack.as_mut() else { return out };
        self.keys.begin(render_version);
        self.frame += 1;
        let frame = self.frame;
        let max = self.device.limits().max_texture_dimension_2d as usize;
        let mut stats = PackStats::default();
        // Images the game has not drawn yet, decoded on the pack's threads:
        // a couple a frame (the upload and mip chain are the cost), while
        // there is room.
        if self.fixes.preload {
            let mut used: u64 = self.pack_tex.values().flatten().map(|t| t.bytes).sum::<u64>() + self.materials.bytes();
            let mut texels = 0usize;
            if used < self.pack_budget / 4 * 3 {
                for (key, img) in pack.preload(PreloadKind::Texture, 2) {
                    let usable = img.width > 0 && img.height > 0 && img.width <= max && img.height <= max;
                    if !usable || self.pack_tex.contains_key(&key) {
                        continue;
                    }
                    texels += img.width * img.height;
                    let tex = upload_pack(&self.device, &self.queue, &img, mip);
                    used += tex.bytes;
                    self.pack_tex.insert(key, Some(tex));
                    self.stable.preloaded += 1;
                    stats.preloaded += 1;
                    if texels > 1 << 20 || used >= self.pack_budget / 4 * 3 {
                        break;
                    }
                }
            }
        }
        // Material maps: only asked for when on and the pack has some.
        let materials = self.quality.materials && pack.has_materials() && !self.no_materials;
        self.materials.begin();
        // Per polygon: its image's key, its material's id, its address
        // modes (filtered images only).
        let mut picks: Vec<Option<SlotKey>> = vec![None; n];
        let mut log_recs: HashMap<u64, model2::packlog::TexRec> = HashMap::new();
        for (i, pl) in placed.iter().enumerate() {
            let p = &polys[pl.poly];
            let Some(t) = &p.texture else { continue };
            stats.textured += 1;
            let Some(info) = tex_info(p, t, src, &mut self.keys) else { continue };
            // As the software renderer: only a polygon with a lamp and a
            // tangent frame has a material to draw.
            let material = if materials && p.lamp.as_ref().is_some_and(|l| l.tangent.is_some()) {
                stats.lamped += 1;
                self.materials.draw(pack.as_mut(), &info, &self.device, &self.queue, mip, frame, max)
            } else {
                None
            };
            stats.material_polys += material.is_some() as u32;
            let rect = [t.bank, t.x0, t.y0, t.w, t.h];
            let fixes = self.fixes;
            let remember = fixes.surface || fixes.hold || self.log.is_some();
            // The pack's image for this key, if it has one (and why not).
            let mut miss = None;
            let mut uploaded_now = false;
            match self.pack_tex.entry(info.key) {
                std::collections::hash_map::Entry::Occupied(e) => {
                    if e.get().is_none() {
                        miss = Some("not-in-pack");
                    }
                }
                std::collections::hash_map::Entry::Vacant(e) => match pack.find(&info) {
                    Found::Pending => {
                        miss = Some(if self.stable.evicted.contains(&info.key) { "evicted" } else { "pending" })
                    }
                    Found::Missing => {
                        e.insert(None);
                        miss = Some("not-in-pack");
                    }
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
                            miss = Some("oversize");
                            None
                        } else {
                            stats.uploaded += 1;
                            uploaded_now = true;
                            self.stable.evicted.remove(&info.key);
                            Some(upload_pack(&self.device, &self.queue, &img, mip))
                        };
                        e.insert(tex);
                    }
                },
            }
            let resident = |m: &HashMap<u64, Option<PackTex>>, k: u64| m.get(&k).is_some_and(Option::is_some);
            let prev = if remember { self.stable.rects.get(&rect).copied() } else { None };
            // A stand-in for a key without an image to draw: another key of
            // the same surface that has one, or the image the sheet
            // rectangle drew a moment ago (its texels or colour changed
            // under it).
            let mut draw_key = miss.is_none().then_some(info.key);
            let mut stand_in = false;
            if let Some(why) = miss {
                let allowed = match why {
                    "pending" | "evicted" => fixes.hold,
                    _ => fixes.surface,
                };
                if allowed {
                    let by_surface =
                        self.stable.surface_image.get(&info.surface).copied().filter(|&k| resident(&self.pack_tex, k));
                    let by_rect = prev
                        .filter(|l| frame - l.hit_frame <= model2::packlog::HOLD_FRAMES)
                        .and_then(|l| l.image)
                        .filter(|&k| resident(&self.pack_tex, k));
                    draw_key = by_surface.or(by_rect);
                    stand_in = draw_key.is_some();
                }
            }
            if remember {
                let own = miss.is_none();
                if own {
                    let kept = self.stable.surface_image.get(&info.surface).copied();
                    if !kept.is_some_and(|k| resident(&self.pack_tex, k)) {
                        self.stable.surface_image.insert(info.surface, info.key);
                    }
                }
                self.stable.rects.insert(
                    rect,
                    RectLast {
                        key: info.key,
                        surface: info.surface,
                        image: if own { Some(info.key) } else { prev.and_then(|l| l.image) },
                        hit_frame: if own { frame } else { prev.map_or(0, |l| l.hit_frame) },
                    },
                );
            }
            if let Some(log) = self.log.as_mut() {
                let _ = log;
                let outcome = match (miss, stand_in) {
                    (None, _) if uploaded_now => "up".to_string(),
                    (None, _) => "hit".to_string(),
                    (Some(_), true) => "held".to_string(),
                    (Some(why), false) => {
                        // Why the key has no image: a key that changed under
                        // a rectangle that was replaced a moment ago (a
                        // colour change keeps the texels' surface, a
                        // transient upload does not), or one of a surface
                        // the pack has under another key.
                        let reason = match why {
                            "pending" | "evicted" | "oversize" => why,
                            _ => match prev {
                                Some(l)
                                    if l.key != info.key
                                        && l.image.is_some()
                                        && frame - l.hit_frame <= 4 * model2::packlog::HOLD_FRAMES =>
                                {
                                    if l.surface == info.surface {
                                        "colour"
                                    } else {
                                        "texels"
                                    }
                                }
                                _ if self.stable.surface_image.contains_key(&info.surface) => "sibling",
                                _ => why,
                            },
                        };
                        format!("miss:{reason}")
                    }
                };
                let (mut a, mut cx, mut cy) = (0.0f64, 0.0f64, 0.0f64);
                let pts = &pl.points;
                for k in 0..pts.len() {
                    let (p, q) = (&pts[k], &pts[(k + 1) % pts.len()]);
                    a += (p.x as f64) * (q.y as f64) - (q.x as f64) * (p.y as f64);
                    cx += p.x as f64;
                    cy += p.y as f64;
                }
                let n = pts.len().max(1) as f64;
                let area = (a / 2.0).abs();
                let r = log_recs.entry(info.key).or_insert_with(|| model2::packlog::TexRec {
                    key: info.key,
                    surface: info.surface,
                    width: info.width,
                    height: info.height,
                    outcome: outcome.clone(),
                    rect: [t.bank, t.x0, t.y0],
                    ..Default::default()
                });
                r.polys += 1;
                r.pixels += area;
                r.sx += cx / n * area;
                r.sy += cy / n * area;
            }
            let image = draw_key.and_then(|k| {
                let pt = self.pack_tex.get_mut(&k)?.as_mut()?;
                pt.used = frame;
                stats.replaced += 1;
                stats.stood_in += stand_in as u32;
                Some(k)
            });
            if image.is_some() || material.is_some() {
                let addr = if mip { t.mirror_x as usize | (t.mirror_y as usize) << 1 } else { 0 };
                picks[i] = Some((image, material, addr));
            }
        }
        // Over budget: drop the images and materials drawn longest ago (not
        // this frame's), in one order, and their bind groups.
        let mut bytes: u64 = self.pack_tex.values().flatten().map(|t| t.bytes).sum();
        let mut mat_bytes = self.materials.bytes();
        if bytes + mat_bytes > self.pack_budget {
            let images = self.pack_tex.iter().filter_map(|(k, t)| t.as_ref().map(|t| (t.used, t.bytes, *k, false)));
            let mats = self.materials.tex.iter().map(|(id, t)| (t.used, t.bytes, *id, true));
            let mut old: Vec<(u64, u64, u64, bool)> = images.chain(mats).collect();
            old.sort_unstable();
            for (used, size, key, material) in old {
                if bytes + mat_bytes <= self.pack_budget || used == frame {
                    break;
                }
                if material {
                    self.materials.forget_material(key);
                    mat_bytes -= size;
                } else {
                    self.pack_tex.remove(&key);
                    self.stable.evicted.insert(key);
                    self.materials.forget_image(key);
                    bytes -= size;
                }
            }
        }
        // Polygons with a material are drawn by the material pipelines;
        // all others by the pipelines and bind groups they have with
        // materials off (the image pipelines if the frame replaces any
        // image), so that they draw exactly as there.
        let with_material = picks.iter().flatten().any(|p| p.1.is_some());
        out.variant = match (picks.iter().flatten().any(|p| p.0.is_some()), mip) {
            (false, _) => Variant::Original,
            (true, false) => Variant::Pack,
            (true, true) => Variant::PackFiltered,
        };
        out.mat_variant = with_material.then_some(if mip { Variant::MatFiltered } else { Variant::Mat });
        let dummies = with_material.then(|| self.materials.dummies(&self.device, &self.queue));
        let mut slot_of: HashMap<SlotKey, u32> = HashMap::new();
        let mut images = std::collections::HashSet::new();
        for (i, pick) in picks.iter().enumerate() {
            let Some((image, material, addr)) = *pick else { continue };
            out.image[i] = image.is_some();
            out.material[i] = material.and_then(|id| self.materials.tex.get(&id)).map(|t| t.normal_scale);
            if let Some(key) = image {
                images.insert((key, addr));
            }
            out.slot[i] = *slot_of.entry((image, material, addr)).or_insert_with(|| {
                let group = match (&dummies, material) {
                    (Some(dummy), Some(_)) => self
                        .materials
                        .groups
                        .entry((image, material, addr))
                        .or_insert_with(|| {
                            let view =
                                image.and_then(|k| self.pack_tex.get(&k)?.as_ref()).map_or(&dummy[0], |t| &t.view);
                            let mat =
                                material.and_then(|id| self.materials.tex.get(&id)).map_or(&dummy[1], |t| &t.view);
                            let smp = self.samplers.as_ref().filter(|_| mip).map(|s| &s[addr]);
                            let layout = if mip { &self.mat_filtered_layout } else { &self.mat_layout };
                            bind_mat(&self.device, layout, view, smp, mat)
                        })
                        .clone(),
                    _ => {
                        let pt = image.and_then(|k| self.pack_tex.get_mut(&k)?.as_mut()).expect("an image");
                        pt.groups[addr]
                            .get_or_insert_with(|| match &self.samplers {
                                Some(smp) if mip => {
                                    bind_pack(&self.device, &self.pack_filtered_layout, &pt.view, Some(&smp[addr]))
                                }
                                _ => bind_pack(&self.device, &self.pack_layout, &pt.view, None),
                            })
                            .clone()
                    }
                };
                out.groups.push(group);
                out.mat_slot.push(material.is_some());
                out.groups.len() as u32 - 1
            });
        }
        if let Some(log) = self.log.as_mut() {
            let mut recs: Vec<_> = log_recs.into_values().collect();
            recs.sort_by_key(|r| r.key);
            for r in &recs {
                log.tex(r);
            }
        }
        stats.images = images.len() as u32;
        stats.held = self.pack_tex.values().flatten().count() as u32;
        stats.bytes = bytes;
        stats.materials = self.materials.drawn();
        stats.material_uploaded = self.materials.uploaded;
        stats.material_held = self.materials.tex.len() as u32;
        stats.material_bytes = mat_bytes;
        self.pack_stats = stats;
        out
    }

    /// The 1x1 bind group for group 1 before a replaced polygon is drawn
    /// (for the filtered pipelines once the samplers exist; any sampler
    /// will do, it is never sampled).
    fn pack_dummy(&mut self, variant: Variant) -> wgpu::BindGroup {
        let filtered = variant == Variant::PackFiltered;
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
        self.texture_used.insert(label, self.frames);
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
        self.texture_used.insert(label, self.frames);
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
            // Cleared at the start of its pass and discarded at the end
            // (`encode_3d`): on Apple GPUs it then takes no memory at all.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TRANSIENT_ATTACHMENT,
            view_formats: &[],
        });
        self.textures.retain(|(l, _)| *l != label);
        self.textures.push((label, t.clone()));
        t
    }

    /// Encode the 3D layer of `placed` into `target` (FORMAT, the size of
    /// `view`; alpha 0 where nothing is drawn, a fraction at multisampled
    /// edges, with colour premultiplied by it). With `temporal`, the
    /// temporal pass: `target` and its `aux` are its internal size, drawn jittered with
    /// the `_t` entry points (one sample).
    #[allow(clippy::too_many_arguments)]
    fn encode_3d(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        polys: &[Poly],
        placed: &[Placed],
        src: &Sources,
        view: &View,
        target: &wgpu::TextureView,
        temporal: Option<&TemporalPass>,
        ids: Option<&wgpu::TextureView>,
    ) {
        let t0 = self.tick();
        let (w, h) = temporal.map_or((view.width as u32, view.height as u32), |t| t.size);
        // The look's ID pass (look.rs): `target` is the ID image and `ids`
        // the 1/z image; the same polygons and depth rules, written as
        // what is there instead of colour, never temporal or multisampled.
        let id_pass = ids.is_some();
        debug_assert!(!(id_pass && temporal.is_some()));
        let vertex_bytes = if temporal.is_some() { VERTEX_BYTES_T } else { VERTEX_BYTES };
        let mut verts: Vec<u8> = Vec::new();
        let mut table: Vec<u8> = Vec::new();
        // Temporal: each polygon's real clip rectangle (`clip_t`).
        let mut table_t: Vec<u8> = Vec::new();
        let q = self.quality;
        // Smooth shading: the lit pipelines only for frames with a lit
        // polygon.
        let lit_frame =
            !id_pass && q.shading != Shading::Flat && placed.iter().any(|pl| !polys[pl.poly].lit.is_empty());
        // The ID pass always has a depth buffer, and never a material.
        let depth_on = q.depth || id_pass;
        let versions = self.versions.take();
        let slots = self.pack_slots(polys, placed, src, versions.map(|v| v.0));
        // The look: per polygon, its ID and tag (the ID pass) and what
        // the tags say about its lighting.
        let look = self.look.clone().filter(|l| l.active());
        // The ID pass after this one reuses the polygons' IDs, the pack
        // slots, the vertices and the polygon table (`look::IdShare`).
        let share = self.share_ids && !id_pass && temporal.is_none() && slots.mat_variant.is_none();
        let mut spans: Vec<look::Span> = Vec::new();
        let t_look = self.tick();
        let look_polys = match &look {
            Some(l) => self.look_polys(polys, placed, src, l, id_pass || share, versions.map(|v| v.0)),
            None => Vec::new(),
        };
        self.lap(t_look, |p| &mut p.look);
        let toon = look::toon_params(look.as_deref().filter(|_| !id_pass), view.height as f32);
        // Material maps: their pipelines for the polygons with a material
        // (`Slots::mat_variant`); every polygon of such a frame has an
        // outline entry.
        let variant = slots.variant;
        let mat_frame = slots.mat_variant.is_some();
        let mat_slot = |s: u32| s != NO_SLOT && slots.mat_slot[s as usize];
        // Draws: runs of polygons in the same window and depth mode, and
        // with the same pack image (`slot`; NO_SLOT: any, as a polygon not
        // replaced reads no image), and material (a polygon with one is
        // never in a run without).
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
                        && (r.slot == slot
                            || ((r.slot == NO_SLOT || slot == NO_SLOT) && !mat_slot(r.slot) && !mat_slot(slot))) =>
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
        // Outlines for the shader's 1/z (`outline_iz`) and lit polygons'
        // lumas (`lit_luma`): a directory of (first, count, lit, 0) per
        // polygon, then the points (x, y, 1/z, luma/z).
        let mut outline_dir: Vec<u8> = Vec::new();
        let mut outline_points: Vec<u8> = Vec::new();
        // Temporal: the polygons' edges, for the thin-polygon test.
        let edges = temporal
            .filter(|tp| tp.thin.is_some() && tp.isolated)
            .map(|_| temporal::Edges::new(placed.iter().map(|pl| &pl.points[..])));
        for (i, pl) in placed.iter().enumerate() {
            let p = &polys[pl.poly];
            let t = p.texture;
            let mut flags = t
                .map_or(0, |t| 1 | (t.mirror_x as u32) << 1 | (t.mirror_y as u32) << 2 | (t.translucent as u32) << 3)
                | (p.checker as u32) << 4
                | (slots.image[i] as u32) << 5;
            // A material: bit 9, and bit 10 where its normal is the corners'
            // interpolated (smooth-shaded with normals, raster.rs
            // `material_values`).
            let lit_normals = slots.material[i].is_some()
                && lit_frame
                && p.lit.len() == pl.points.len()
                && p.lit.iter().any(|c| c[1] != 0.0 || c[2] != 0.0 || c[3] != 0.0);
            if slots.material[i].is_some() {
                flags |= 512 | (lit_normals as u32) << 10;
            }
            let (mut x0, mut y0, mut x1, mut y1) = pl.clip;
            // Temporal: the motion of each point, if known.
            let mut motion = None;
            // Temporal: each point's offset widening a thin polygon.
            let mut widen: Vec<(f32, f32)> = Vec::new();
            if let Some(tp) = temporal {
                // The `_t` entry points clip and draw the checkerboard
                // themselves, at the jittered sample point: the shared
                // shading functions get a clip that never discards and no
                // checker bit (raster.wgsl, `pre_t`).
                table_t.extend(le([x0, y0, x1, y1].map(f32::to_le_bytes)));
                (x0, y0, x1, y1) = (-3e38, -3e38, 3e38, 3e38);
                motion = tp.motion.of(pl);
                // Thin polygons (temporal.rs, `thin`): bit 8, marked in
                // the aux target; widened to `tp.cover` internal px so every
                // jitter phase's samples hit them, drawn at their coverage
                // (bits 16-23, 255 = 1) over what is behind.
                // A sliver of a mesh (a long edge shared) is not one.
                let thin = tp.thin.and_then(|t| {
                    let pts: Vec<(f32, f32)> = pl.points.iter().map(|v| (v.x, v.y)).collect();
                    temporal::thin(&pts, tp.rs, t).filter(|_| !edges.as_ref().is_some_and(|e| e.long_edge_shared(&pts)))
                });
                let mut cov = 255;
                if let Some(t) = thin.filter(|_| tp.cover > 0.0) {
                    let to = tp.cover.max(t.width);
                    widen = pl.points.iter().map(|v| t.widen((v.x, v.y), tp.rs, to)).collect();
                    cov = (t.coverage(to) * 255.0).round().clamp(1.0, 255.0) as u32;
                }
                let thin = thin.is_some();
                flags = (flags & !16)
                    | (p.checker as u32) << 6
                    | (motion.is_some() as u32) << 7
                    | (thin as u32) << 8
                    | cov << 16;
            }
            let tex = t.map_or([0; 4], |t| [t.x0, t.y0, t.w, t.h]);
            let (bank, luma_table) = t.map_or((0, 0), |t| (t.bank, t.luma_table));
            // The look's tags: an emissive polygon is drawn at full
            // brightness and unlit; a toon override rides in the outline
            // flags (raster.wgsl `poly_px`).
            let lk = if id_pass { None } else { look_polys.get(i) };
            let emissive = lk.is_some_and(|l| l.emissive);
            table.extend(le([x0, y0, x1, y1].map(f32::to_le_bytes)));
            let luma = if emissive { 255 } else { p.luma };
            let ints = tex.into_iter().chain([flags, bank, luma_table, luma]).chain(p.colour).chain([pl.rank]);
            table.extend(le(ints.map(u32::to_le_bytes)));
            let first = (verts.len() as u64 / vertex_bytes) as u32;
            // Convex polygon as a triangle fan.
            for k in 1..pl.points.len().saturating_sub(1) {
                for j in [0, k, k + 1] {
                    let v = pl.points[j];
                    let (dx, dy) = widen.get(j).copied().unwrap_or((0.0, 0.0));
                    verts.extend(le([v.x + dx, v.y + dy, v.iz, v.uz, v.vz].map(f32::to_le_bytes)));
                    verts.extend((i as u32).to_le_bytes());
                    if temporal.is_some() {
                        // Where it was; without motion data where it is
                        // (homogeneous: a widened point moved as much).
                        let prev = motion
                            .map_or([v.x, v.y, 1.0], |m: &[[f32; 3]]| m.get(j).copied().unwrap_or([v.x, v.y, 1.0]));
                        let prev = [prev[0] + dx * prev[2], prev[1] + dy * prev[2], prev[2]];
                        verts.extend(le(prev.map(f32::to_le_bytes)));
                    }
                }
            }
            let end = (verts.len() as u64 / vertex_bytes) as u32;
            let lit = lit_frame && !p.lit.is_empty() && !emissive;
            let mat = slots.material[i];
            if depth_on || lit || mat.is_some() {
                let first = placed.len() + outline_points.len() / 16;
                let count = pl.points.len();
                // Lit: bit 0, per pixel bit 1, finer levels bit 2, the
                // luma range above.
                let pixel = lit && q.shading == Shading::Pixel && p.pixel.is_some();
                let fine = lit && q.shading_fine;
                let flags = if lit {
                    1 | (pixel as u32) << 1
                        | (fine as u32) << 2
                        | (lk.map_or(0, |l| l.toon as u32) & 15) << 3
                        | (p.lit_range[0] as u32) << 8
                        | (p.lit_range[1] as u32) << 16
                } else {
                    0
                };
                let normals = if pixel || mat.is_some() { (first + count) as u32 } else { 0 };
                outline_dir.extend(le([first as u32, count as u32, flags, normals].map(u32::to_le_bytes)));
                for (k, v) in pl.points.iter().enumerate() {
                    let lz = if lit { p.lit[k][0] * v.iz } else { 0.0 };
                    outline_points.extend(le([v.x, v.y, v.iz, lz].map(f32::to_le_bytes)));
                }
                // The lighting (raster.wgsl `lamp_eval`), offset in .w.
                let lamp = |l: &model2::light::Lamp, offset: f32| {
                    let base = match l.base {
                        model2::light::Base::D => 0,
                        model2::light::Base::S0 => 1,
                        model2::light::Base::Dotl => 2,
                    };
                    let bits = l.front as u32 | base << 1 | l.squarings.min(255) << 8;
                    let mut e = le([l.l[0], l.l[1], l.l[2], f32::from_bits(bits)].map(f32::to_le_bytes));
                    e.extend(le([l.ambient, l.diffuse, l.spec_scale, offset].map(f32::to_le_bytes)));
                    e
                };
                if pixel || lit_normals {
                    // The normals/z per point.
                    for (k, v) in pl.points.iter().enumerate() {
                        let n = p.lit[k];
                        outline_points.extend(le([n[1] * v.iz, n[2] * v.iz, n[3] * v.iz, 0.0].map(f32::to_le_bytes)));
                    }
                }
                if let (true, Some(px)) = (pixel, &p.pixel) {
                    outline_points.extend(lamp(&px.lamp, px.offset));
                }
                if let (Some(scale), Some(l)) = (mat, p.lamp.as_deref()) {
                    // The material's block (raster.wgsl `mat_luma`): the
                    // face's normal and the side the eye sees, the tangent
                    // frame, the lamp alone, the strengths and the range.
                    let t = l.tangent.unwrap_or(model2::raster::Tangent { t: [0.0; 3], h: 0.0 });
                    outline_points.extend(le([l.n[0], l.n[1], l.n[2], l.out].map(f32::to_le_bytes)));
                    outline_points.extend(le([t.t[0], t.t[1], t.t[2], t.h].map(f32::to_le_bytes)));
                    outline_points.extend(lamp(&l.light.lamp, 0.0));
                    let range = f32::from_bits(l.range[0] as u32 | (l.range[1] as u32) << 8);
                    let strength = l.relief.strength().unwrap_or(0.0);
                    outline_points.extend(le([scale, strength, range, 0.0].map(f32::to_le_bytes)));
                }
            } else if lit_frame || mat_frame {
                outline_dir.extend([0; 16]);
            }
            // Translucent (checkerboard) polygons are background; the ID
            // pass leaves them out.
            let grouped = depth_on && !pl.background;
            let mode = match (depth_on, pl.background) {
                (false, _) => Mode::Plain,
                (_, true) => Mode::Background,
                _ => Mode::Test,
            };
            // The ID pass draws what the colour pass does but the translucent
            // polygons (look.rs `in_id_pass`).
            if !(id_pass && !look::in_id_pass(p, pl)) {
                push(&mut runs, first..end, p.window, mode, pl.clip, slots.slot[i]);
            }
            if share {
                spans.push(look::Span {
                    vertices: first..end,
                    window: p.window,
                    clip: pl.clip,
                    slot: slots.slot[i],
                    background: pl.background,
                    in_id: look::in_id_pass(p, pl),
                    group_end: placed.get(i + 1).is_none_or(|n| n.group != pl.group),
                });
            }
            // The depth-only draw repeats a replaced polygon's cut-out.
            if grouped {
                group_depth.push(Run {
                    vertices: first..end,
                    window: p.window,
                    mode: Mode::DepthWrite,
                    clip: pl.clip,
                    slot: slots.slot[i],
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
        self.lap(t0, |p| &mut p.prep3d);
        let t0 = self.tick();
        let render = self.ram_buffer(0, src.render_ram, versions.map(|v| v.0));
        let palette = self.ram_buffer(1, src.palette, versions.map(|v| v.1));
        let d = &self.device;
        let buffer = |label, contents: &[u8], usage| {
            d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage })
        };
        let table = buffer("polys", &table, wgpu::BufferUsages::STORAGE);
        let vertices = buffer("vertices", &words(&verts), wgpu::BufferUsages::VERTEX);
        outline_dir.extend(outline_points);
        // Temporal: the real clip rectangles after the outlines (`clip_t`;
        // a buffer of their own would be a fifth storage buffer, more than
        // the portable limits have).
        let clip_base = (outline_dir.len() / 16) as u32;
        outline_dir.extend(table_t);
        // The ID pass: each polygon's ID and tag words, at `id_base`.
        let id_base = (outline_dir.len() / 16) as u32;
        if id_pass {
            for l in &look_polys {
                outline_dir.extend(le([l.id, l.tag, 0, 0].map(u32::to_le_bytes)));
            }
        }
        if outline_dir.is_empty() {
            outline_dir.resize(16, 0);
        }
        let outlines = buffer("outlines", &outline_dir, wgpu::BufferUsages::STORAGE);

        // The output size (also for the temporal pass, which is placed in
        // output pixels and drawn at its own size through the viewport).
        let mut screen = le([view.width as f32, view.height as f32, view.sx, view.sy].map(f32::to_le_bytes));
        // The look's toon and halftone (raster.wgsl `Screen`); toon.w is
        // where the ID pass's entries are.
        let mut look_words = toon;
        look_words[3] = f32::from_bits(id_base);
        screen.extend(le(look_words.map(f32::to_le_bytes)));
        let screen = buffer("screen", &screen, wgpu::BufferUsages::UNIFORM);
        let mut entries = vec![
            wgpu::BindGroupEntry { binding: 0, resource: screen.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: table.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: render.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: palette.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 4, resource: outlines.as_entire_binding() },
        ];
        let screen_t = temporal.map(|tp| buffer("screen t", &tp.screen_t(clip_base), wgpu::BufferUsages::UNIFORM));
        if let Some(screen_t) = &screen_t {
            entries.push(wgpu::BindGroupEntry { binding: 5, resource: screen_t.as_entire_binding() });
        }
        let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("raster"),
            layout: if temporal.is_some() { &self.raster_layout_t } else { &self.raster_layout },
            entries: &entries,
        });
        let samples = if temporal.is_some() || id_pass { 1 } else { q.msaa.max(1) };
        let msaa = (samples > 1).then(|| self.kept_attachment("3d msaa", w, h, samples, FORMAT));
        let msaa_view = msaa.as_ref().map(|t| t.create_view(&Default::default()));
        let depth_label = if temporal.is_some() { "3d depth t" } else { "3d depth" };
        let depth_label = if id_pass { "look depth" } else { depth_label };
        let depth = depth_on.then(|| self.kept_attachment(depth_label, w, h, samples, DEPTH_FORMAT));
        let depth_view = depth.as_ref().map(|t| t.create_view(&Default::default()));
        let is_t = temporal.is_some();
        // Depth-only draws need no luma: never lit.
        let shading = |m: Mode| (lit_frame && m != Mode::DepthWrite) as u8;
        let pipelines: Vec<(Mode, wgpu::RenderPipeline)> = if id_pass {
            [Mode::Test, Mode::DepthWrite, Mode::Background].map(|m| (m, self.id_pipeline(m, variant))).into()
        } else if q.depth {
            [Mode::Test, Mode::DepthWrite, Mode::Background]
                .map(|m| (m, self.raster_pipeline(samples, m, variant, is_t, shading(m))))
                .into()
        } else {
            vec![(Mode::Plain, self.raster_pipeline(samples, Mode::Plain, variant, is_t, shading(Mode::Plain)))]
        };
        // The material pipelines, for the runs with a material.
        let mat_pipelines: Vec<(Mode, wgpu::RenderPipeline)> = match slots.mat_variant {
            Some(v) => pipelines.iter().map(|(m, _)| (*m, self.raster_pipeline(samples, *m, v, is_t, 0))).collect(),
            None => Vec::new(),
        };
        let clear_pipeline = if id_pass {
            Some(self.id_pipeline(Mode::Clear, variant))
        } else {
            q.depth.then(|| self.raster_pipeline(samples, Mode::Clear, variant, is_t, 0))
        };
        let dummy = (variant != Variant::Original).then(|| self.pack_dummy(variant));
        self.pack_stats.draws = runs.len() as u32;
        let colour = Some(wgpu::RenderPassColorAttachment {
            view: msaa_view.as_ref().unwrap_or(target),
            depth_slice: None,
            resolve_target: msaa_view.as_ref().map(|_| target),
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: if msaa_view.is_some() { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store },
            },
        });
        // Temporal: the aux target too, cleared to 0 (nothing drawn).
        let aux = temporal.map(|tp| wgpu::RenderPassColorAttachment {
            view: tp.aux,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
        });
        let id_colours = ids.map(|iz| {
            let clear =
                wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store };
            [target, iz].map(|view| {
                Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: clear })
            })
        });
        let attachments = [colour, aux];
        self.lap(t0, |p| &mut p.resources3d);
        let t0 = self.tick();
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("3d layer"),
            color_attachments: match &id_colours {
                Some(c) => c,
                None if is_t => &attachments,
                None => &attachments[..1],
            },
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
        // Group 1 holds a material's bind group (the material pipelines'
        // layout): the image pipelines need theirs back first.
        let mut bound_mat = false;
        let unbind_mat = |pass: &mut wgpu::RenderPass, bound: &mut u32, bound_mat: &mut bool| {
            if *bound_mat {
                *bound_mat = false;
                *bound = NO_SLOT;
                if let Some(dummy) = &dummy {
                    pass.set_bind_group(1, dummy, &[]);
                }
            }
        };
        pass.set_vertex_buffer(0, vertices.slice(..));
        for (k, run) in runs.iter().enumerate() {
            // Each window starts with an empty depth buffer where it draws
            // (the first has the pass's clear).
            if let (Some(clear), true) = (&clear_pipeline, k > 0 && runs[k - 1].window != run.window) {
                // Temporal: the rectangle in internal pixels, one more each
                // side for the jitter.
                let clip = temporal.map_or(run.clip, |tp| {
                    let (sx, sy) = tp.rs;
                    (run.clip.0 * sx - 1.0, run.clip.1 * sy - 1.0, run.clip.2 * sx + 1.0, run.clip.3 * sy + 1.0)
                });
                let (x0, y0) = (clip.0.floor().max(0.0) as u32, clip.1.floor().max(0.0) as u32);
                let (x1, y1) = ((clip.2.ceil().max(0.0) as u32).min(w), (clip.3.ceil().max(0.0) as u32).min(h));
                if x1 > x0 && y1 > y0 {
                    unbind_mat(&mut pass, &mut bound, &mut bound_mat);
                    pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
                    pass.set_pipeline(clear);
                    pass.draw(0..3, 0..1);
                    pass.set_scissor_rect(0, 0, w, h);
                }
            }
            let with_mat = mat_slot(run.slot);
            if !with_mat {
                unbind_mat(&mut pass, &mut bound, &mut bound_mat);
            }
            let list = if with_mat { &mat_pipelines } else { &pipelines };
            if let Some((_, pipeline)) = list.iter().find(|(m, _)| *m == run.mode) {
                pass.set_pipeline(pipeline);
            }
            if run.slot != NO_SLOT && run.slot != bound {
                bound = run.slot;
                bound_mat = with_mat;
                pass.set_bind_group(1, &slots.groups[run.slot as usize], &[]);
            }
            pass.draw(run.vertices.clone(), 0..1);
        }
        drop(pass);
        if share {
            self.id_share = Some(look::IdShare {
                slots,
                polys: look_polys,
                spans,
                vertices: vertices.clone(),
                table: table.clone(),
                render: render.clone(),
                palette: palette.clone(),
                variant,
            });
        }
        self.lap(t0, |p| &mut p.pass3d);
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
    fn read_back(&self, enc: wgpu::CommandEncoder, texture: &wgpu::Texture, w: u32, h: u32) -> Vec<u8> {
        self.read_back_bpp(enc, texture, w, h, 4)
    }

    /// `read_back` of a texture of `bpp` bytes a pixel.
    fn read_back_bpp(
        &self,
        mut enc: wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        w: u32,
        h: u32,
        bpp: u32,
    ) -> Vec<u8> {
        let row = (w * bpp).div_ceil(256) * 256;
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
        let n = (w * bpp) as usize;
        (0..h as usize).flat_map(|y| data[y * row as usize..y * row as usize + n].to_vec()).collect()
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
        self.encode_3d(&mut enc, polys, placed, src, view, &target.create_view(&Default::default()), None, None);
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

    /// `rgba` in the kept texture `label` (`slot` of `uploaded`), unless
    /// that texture already holds these cached layers (`generation`).
    fn upload(
        &mut self,
        slot: usize,
        generation: Option<u64>,
        label: &'static str,
        rgba: &[u8],
        w: u32,
        h: u32,
    ) -> wgpu::Texture {
        let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        let t = self.kept_texture(label, w, h, usage);
        if let (Some(g), Some((ug, ut))) = (generation, &self.uploaded[slot]) {
            if *ug == g && *ut == t {
                return t;
            }
        }
        self.uploaded[slot] = generation.map(|g| (g, t.clone()));
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
        self.frames += 1;
        let t_frame = self.tick();
        let in3d = self.phases.prep3d + self.phases.resources3d + self.phases.pass3d;
        let versions = self.versions;
        if let Some(targets) = self.photo_depth.take() {
            self.encode_photo_depth(enc, polys, placed, &targets, (w, h));
        }
        let look = self.look.clone().filter(|l| l.active() && l.params.needs_ids() && !placed.is_empty());
        self.share_ids = look.is_some();
        self.id_share = None;
        let layer3d_view = if self.quality.temporal > 0 {
            // The accumulated picture, output size.
            self.encode_temporal(enc, polys, placed, src, view)
        } else {
            let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
            let layer3d = self.kept_texture("3d layer", w, h, usage);
            let layer3d_view = layer3d.create_view(&Default::default());
            self.encode_3d(enc, polys, placed, src, view, &layer3d_view, None, None);
            layer3d_view
        };
        // The look's ink pass over the finished 3D layer (the ID pass
        // draws the polygons again, so the frame's RAM versions are put
        // back for it).
        self.share_ids = false;
        let layer3d_view = match &look {
            Some(l) => {
                self.look_used = self.frames;
                self.versions = versions;
                self.encode_ink(enc, polys, placed, src, view, l, &layer3d_view)
            }
            None => layer3d_view,
        };
        let tile_group =
            layers.tiles.as_ref().and_then(|m| self.tiles.bind(&self.device, &self.queue, m, layers.ext_w));
        if self.pack.is_some() {
            let (held, bytes) = self.tiles.held();
            let replaced = layers.tiles.as_ref().map_or(0, |m| m.replaced);
            (self.pack_stats.tiles, self.pack_stats.tiles_held, self.pack_stats.tile_bytes) = (replaced, held, bytes);
        }
        // Bloom of the finished 3D layer; a 1x1 stand-in when off (the
        // shader never reads it then).
        let bloom_strength = self.quality.bloom as f32 / 100.0;
        let bloom_view = if self.quality.bloom > 0 && !placed.is_empty() {
            self.bloom_used = self.frames;
            self.bloom.encode(&self.device, enc, &layer3d_view, w, h, self.bloom_extra)
        } else {
            let usage = wgpu::TextureUsages::TEXTURE_BINDING;
            self.kept_texture("bloom none", 1, 1, usage).create_view(&Default::default())
        };
        let generation = self.layers_generation.take();
        let base = self.upload(0, generation, "base", &layers.base, (WIDTH + 2 * layers.side) as u32, HEIGHT as u32);
        let hud = self.upload(1, generation, "hud", &layers.hud, layers.ext_w as u32, HEIGHT as u32);
        let b = layers.backdrop.map(|c| c as f32 / 255.0);
        // With the CRT look the picture is composed into a kept texture at
        // the origin, and the CRT pass writes it to the target.
        let crt = self.quality.crt > 0;
        let crt_view = crt.then(|| self.crt.input(&self.device, w, h, format));
        let compose_origin = if crt { (0, 0) } else { origin };
        let mut params = le([view.sx, view.sy, view.pad].map(f32::to_le_bytes));
        params.extend(le([!placed.is_empty() as u32, layers.ext_w as u32, layers.side as u32].map(u32::to_le_bytes)));
        params.extend(le([compose_origin.0 as f32, compose_origin.1 as f32].map(f32::to_le_bytes)));
        params.extend(le([b[0], b[1], b[2], 1.0].map(f32::to_le_bytes)));
        let bloom_strength = if self.quality.bloom > 0 && !placed.is_empty() { bloom_strength } else { 0.0 };
        let black = self.black / 255.0;
        params.extend(le([bloom_strength, black, w as f32, h as f32].map(f32::to_le_bytes)));
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
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&bloom_view) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&self.bloom_sampler) },
            ],
        });
        let pipeline = match tile_group {
            Some(_) => self.tiles.pipeline(&self.device, &self.compose_shader, &self.compose_layout, format),
            None => self.compose_pipeline(format),
        };
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("compose"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: crt_view.as_ref().unwrap_or(target),
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        pass.set_viewport(compose_origin.0 as f32, compose_origin.1 as f32, w as f32, h as f32, 0.0, 1.0);
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        if let Some(g) = &tile_group {
            pass.set_bind_group(1, g, &[]);
        }
        pass.draw(0..3, 0..1);
        drop(pass);
        if crt {
            self.crt_used = self.frames;
            let strength = self.quality.crt as f32 / 100.0;
            self.crt.encode(&self.device, enc, target, format, origin, (w, h), strength, HEIGHT as f32);
        }
        if let Some(t) = t_frame {
            // Everything but the 3D layer's own encoding, which lapped
            // itself.
            let in3d = self.phases.prep3d + self.phases.resources3d + self.phases.pass3d - in3d;
            self.phases.rest += t.elapsed().saturating_sub(in3d);
        }
        self.evict_unused();
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
        self.black = 0.0;
        let mut enc = self.device.create_command_encoder(&Default::default());
        self.encode_frame(&mut enc, polys, placed, src, view, layers, target, format, (0, 0));
        self.queue.submit([enc.finish()]);
    }

    /// Draw the Model 2 board's current frame into `target` at `origin`
    /// (the rest of `target` is cleared to black) and submit. `geo` is the
    /// geometrizer output to draw (raster stream, texture point RAM), or
    /// None for the bus's own engine. With temporal on, the engine's frame
    /// is drawn with its smooth motion data (`Engine::smooth_frame`) when
    /// it has some, as [`Renderer::present_bus_motion`]; other geometry
    /// without motion data.
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
        bus.geo.sync();
        let e = bus.geo.engine();
        let (smooth, light) = (e.smooth_frame(), e.light_frame());
        let engine = (&e.stream[..], &e.texture_point_ram[..]);
        let own = Own { engine, smooth: smooth.as_deref(), light: light.as_ref(), late: false };
        self.present_ref(BoardRef::of(bus), own, geo, None, view, target, format, origin);
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
        let g = &bus.geo;
        let own =
            Own { engine: g.last_frame(), smooth: g.last_smooth().map(|f| &**f), light: g.last_light(), late: true };
        self.present_ref(BoardRef::of(bus), own, None, None, view, target, format, origin);
    }

    /// [`Renderer::present_bus`] with motion data for temporal
    /// anti-aliasing: `geo` as there (an in-between, say), `motion` the
    /// frame's (`smooth::Frame::motion`, with its tag; `s1` None: the
    /// engine's stream). The step from the last frame is worked out here
    /// ([`step`]), and the previous frame's window centres kept. Without
    /// temporal on, the same as `present_bus`.
    #[allow(clippy::too_many_arguments)]
    pub fn present_bus_motion(
        &mut self,
        bus: &mut model2::Model2Bus,
        geo: Option<(&[u32], &[u16])>,
        motion: Option<&BusMotion>,
        view: &View,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        origin: (u32, u32),
    ) {
        if self.quality.temporal == 0 {
            return self.present_bus(bus, geo, view, target, format, origin);
        }
        if geo.is_none() || motion.is_some_and(|m| m.s1.is_none()) {
            bus.geo.sync();
        }
        let e = bus.geo.engine();
        let (smooth, light) = (e.smooth_frame(), e.light_frame());
        let engine = (&e.stream[..], &e.texture_point_ram[..]);
        let own = Own { engine, smooth: smooth.as_deref(), light: light.as_ref(), late: false };
        self.present_ref(BoardRef::of(bus), own, geo, Some(motion), view, target, format, origin);
    }

    /// Draw a frame copied from a bus ([`BoardFrame::capture`]) as the
    /// `present_bus*` calls draw the bus: `geo` None, the frame's own
    /// polygons (as `present_bus`, or `present_bus_last` for a frame
    /// captured late); else that geometry (an in-between) with `motion`
    /// for temporal anti-aliasing, as `present_bus_motion`.
    #[allow(clippy::too_many_arguments)]
    pub fn present_board(
        &mut self,
        f: &BoardFrame,
        geo: Option<(&[u32], &[u16])>,
        motion: Option<&BusMotion>,
        view: &View,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        origin: (u32, u32),
    ) {
        let own = Own { engine: f.engine(), smooth: f.smooth().map(|s| &**s), light: f.light(), late: f.late() };
        let motion = geo.is_some().then_some(motion);
        self.present_ref(f.board(), own, geo, motion, view, target, format, origin);
    }

    /// What the `present_*` calls share: `b`'s frame with the
    /// geometrizer's `own` output, or `geo` instead; with temporal on,
    /// `motion` Some is the motion data to draw with (as
    /// `present_bus_motion`), None: `own`'s smooth frame's for its own
    /// polygons, none for other geometry.
    #[allow(clippy::too_many_arguments)]
    fn present_ref(
        &mut self,
        b: BoardRef,
        own: Own,
        geo: Option<(&[u32], &[u16])>,
        motion: Option<Option<&BusMotion>>,
        view: &View,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        origin: (u32, u32),
    ) {
        self.set_log_frame(b.frame);
        // The side channel of the geometrizer's own frame, unless one was
        // passed in (`set_light_frame`); other geometry has none.
        if geo.is_none() && self.light_side.is_none() {
            self.light_side = own.light.cloned();
        }
        if self.quality.temporal == 0 {
            return self.draw_bus(b, geo.unwrap_or(own.engine), view, target, format, origin);
        }
        // The frame's own polygons are drawn with its motion data (tagged
        // as the game frame: a repeat when drawn again).
        let own_motion = match geo {
            None => own.smooth.map(|f| BusMotion::of(f, FrameTag::game(f.frame), own.late.then_some(own.engine.0))),
            Some(_) => None,
        };
        let motion = motion.unwrap_or(own_motion.as_ref());
        let (stream, points) = geo.unwrap_or(own.engine);
        let src = b.sources(points);
        self.draw_bus_t(b, stream, &src, own.engine.0, motion, view, target, format, origin);
    }

    /// `draw_bus` with temporal on: `stream` drawn with motion data
    /// against `s1` (list N at its own pose, unless `motion.s1` says).
    #[allow(clippy::too_many_arguments)]
    fn draw_bus_t(
        &mut self,
        b: BoardRef,
        stream: &[u32],
        src: &Sources,
        s1: &[u32],
        motion: Option<&BusMotion>,
        view: &View,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        origin: (u32, u32),
    ) {
        let step = motion.map_or(Step::NoMotion, |m| self.temporal_step(m.tag, m.cut));
        let prev_centres = self.temporal.centres.clone();
        let (mut polys, centres) = {
            let m_in = match (step, motion) {
                (Step::Blend { tp }, Some(m)) if !m.s0.is_empty() => Some(model2::raster::MotionIn {
                    s0: m.s0,
                    s1: m.s1.unwrap_or(s1),
                    tp,
                    valid: m.valid,
                    prev_centres: &prev_centres,
                }),
                _ => None,
            };
            self.build_polys(stream, src, m_in.as_ref(), Some(b.versions.0))
        };
        let t = self.tick();
        self.hud_layout(b.tiles, b.palette, view).anchor(&mut polys, view);
        let placed = model2::raster::place(&mut polys, view);
        self.lap(t, |p| &mut p.place);
        let t = self.tick();
        let layers = self.layers_cached(b.tiles, b.palette, view, !placed.is_empty());
        self.lap(t, |p| &mut p.layers);
        let (render, palette) = b.versions;
        self.set_ram_versions(render, palette);
        self.temporal.pending = Some((motion.map(|m| m.tag), step, centres));
        self.black = b.black;
        let mut enc = self.device.create_command_encoder(&Default::default());
        self.encode_frame(&mut enc, &polys, &placed, src, view, &layers, target, format, origin);
        let t = self.tick();
        self.queue.submit([enc.finish()]);
        self.lap(t, |p| &mut p.submit);
        self.phases.draws += self.profile as u32;
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_bus(
        &mut self,
        b: BoardRef,
        (stream, points): (&[u32], &[u16]),
        view: &View,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        origin: (u32, u32),
    ) {
        let src = b.sources(points);
        let (mut polys, _) = self.build_polys(stream, &src, None, Some(b.versions.0));
        let t = self.tick();
        self.hud_layout(b.tiles, b.palette, view).anchor(&mut polys, view);
        let mut placed = model2::raster::place(&mut polys, view);
        self.photo_drop_hud(&polys, &mut placed, view);
        self.lap(t, |p| &mut p.place);
        let t = self.tick();
        let layers = self.layers_cached(b.tiles, b.palette, view, !placed.is_empty());
        self.lap(t, |p| &mut p.layers);
        let (render, palette) = b.versions;
        self.set_ram_versions(render, palette);
        self.black = b.black;
        let mut enc = self.device.create_command_encoder(&Default::default());
        self.encode_frame(&mut enc, &polys, &placed, &src, view, &layers, target, format, origin);
        let t = self.tick();
        self.queue.submit([enc.finish()]);
        self.lap(t, |p| &mut p.submit);
        self.phases.draws += self.profile as u32;
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
        self.black = 0.0;
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

    /// [`Renderer::present_bus`] with the bus's own engine into an
    /// offscreen texture, read back as RGB (for tools). Blocks: native
    /// only.
    pub fn frame_bus(&mut self, bus: &mut model2::Model2Bus, view: &View) -> Vec<u8> {
        let (w, h) = (view.width as u32, view.height as u32);
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        let out = self.texture("frame", w, h, usage);
        self.present_bus(bus, None, view, &out.create_view(&Default::default()), FORMAT, (0, 0));
        let rgba = self.read_texture(&out, w, h);
        rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect()
    }
}

/// The geometrizer's own output for a frame drawn by `present_ref`: its
/// raster stream and texture point RAM, smooth motion frame and lighting
/// side channel, and whether it is the frame before the newest (late).
struct Own<'a> {
    engine: (&'a [u32], &'a [u16]),
    smooth: Option<&'a model2::smooth::Frame>,
    light: Option<&'a std::sync::Arc<model2::light::Frame>>,
    late: bool,
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
    let (view, bytes) = upload_levels(
        device,
        queue,
        "pack image",
        levels.iter().map(|l| (l.width as u32, l.height as u32, &l.data[..])),
    );
    PackTex { view, mipmapped, groups: Default::default(), bytes, used: 0 }
}

/// A sampled `FORMAT` texture (`Rgba8Unorm`, never sRGB) of `levels`
/// (width, height, RGBA8 rows top to bottom), level 0 first: its view and
/// its bytes.
fn upload_levels<'a>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    levels: impl Iterator<Item = (u32, u32, &'a [u8])>,
) -> (wgpu::TextureView, u64) {
    let levels: Vec<_> = levels.collect();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: levels[0].0, height: levels[0].1, depth_or_array_layers: 1 },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut bytes = 0;
    for (k, &(w, h, data)) in levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: k as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: None },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        bytes += data.len() as u64;
    }
    (texture.create_view(&Default::default()), bytes)
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

/// Group 1 of the material pipelines: the image (or a stand-in), the
/// sampler (filtered only), the material (or a stand-in).
fn bind_mat(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    image: &wgpu::TextureView,
    sampler: Option<&wgpu::Sampler>,
    material: &wgpu::TextureView,
) -> wgpu::BindGroup {
    let mut entries = vec![
        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(image) },
        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(material) },
    ];
    if let Some(s) = sampler {
        entries.push(wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(s) });
    }
    device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("pack material"), layout, entries: &entries })
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
            "vs_t",
            "fs_t",
            "fs_depth_t",
            "fs_pack_t",
            "fs_pack_depth_t",
            "fs_packf_t",
            "fs_packf_depth_t",
            "fs_mat",
            "fs_mat_depth",
            "fs_matf",
            "fs_matf_depth",
            "fs_mat_t",
            "fs_mat_depth_t",
            "fs_matf_t",
            "fs_matf_depth_t",
            "fs_id",
            "fs_id_pack",
            "fs_id_packf",
            "fs_id_clear",
        ] {
            assert!(names.contains(&name), "raster.wgsl has no entry point {name}");
            // Each fragment entry point has its smooth-shading twin.
            // (The material ones light lit polygons themselves.)
            if name.starts_with("fs_")
                && name != "fs_clear"
                && !name.starts_with("fs_mat")
                && !name.starts_with("fs_id")
            {
                let lit = super::lit_entry(name, true);
                assert!(names.contains(&lit.as_str()), "raster.wgsl has no entry point {lit}");
            }
        }
        for name in ["fs", "fs_t"] {
            assert!(names.contains(&name), "raster.wgsl has no entry point {name}");
        }
        validate(include_str!("compose.wgsl"));
        validate(include_str!("look.wgsl"));
        let temporal = validate(include_str!("temporal.wgsl"));
        assert!(temporal.entry_points.iter().any(|e| e.name == "fs_resolve"));
    }

    /// Only the temporal (`_t`) entry points use group 0's binding 5, so
    /// the others keep the five-binding layout; and every temporal
    /// fragment entry point writes the aux target.
    #[test]
    fn only_temporal_entry_points_use_binding_5() {
        let module = naga::front::wgsl::parse_str(&super::raster_source()).unwrap();
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), Default::default())
            .validate(&module)
            .unwrap();
        for (k, ep) in module.entry_points.iter().enumerate() {
            let fi = info.get_entry_point(k);
            let uses = module
                .global_variables
                .iter()
                .any(|(h, v)| v.binding.as_ref().is_some_and(|b| b.group == 0 && b.binding >= 5) && !fi[h].is_empty());
            let temporal = ep.name.ends_with("_t");
            assert_eq!(uses, temporal, "{}", ep.name);
            if temporal && ep.stage == naga::ShaderStage::Fragment {
                let ty = ep.function.result.as_ref().map(|r| &module.types[r.ty].inner);
                let Some(naga::TypeInner::Struct { members, .. }) = ty else { panic!("{}: no struct output", ep.name) };
                assert!(
                    members.iter().any(|m| matches!(m.binding, Some(naga::Binding::Location { location: 1, .. }))),
                    "{}: no aux output",
                    ep.name
                );
            }
        }
    }

    /// The original entry points use group 0 only, so their pipelines keep
    /// the one-group layout; the pack ones add the image in group 1, the
    /// material ones the material too (binding 2), which nothing else
    /// reads.
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
            let material = ep.name.starts_with("fs_mat");
            let pack = ep.name.starts_with("fs_pack") || ep.name.starts_with("fs_id_pack");
            assert_eq!(uses_group_1, pack || material, "{}", ep.name);
            let uses_binding_2 = module
                .global_variables
                .iter()
                .any(|(h, v)| v.binding.as_ref().is_some_and(|b| b.group == 1 && b.binding == 2) && !fi[h].is_empty());
            assert_eq!(uses_binding_2, material, "{}", ep.name);
            // Only the filtered ones sample (derivatives, a sampler).
            let samples =
                module.global_variables.iter().any(|(h, v)| v.name.as_deref() == Some("pack_smp") && !fi[h].is_empty());
            assert_eq!(
                samples,
                ep.name.starts_with("fs_packf") || ep.name.starts_with("fs_matf") || ep.name == "fs_id_packf",
                "{}",
                ep.name
            );
        }
    }
}
