//! Temporal anti-aliasing and upscaling ([`crate::Quality::temporal`]).
//!
//! With it on, the 3D layer is drawn with a sub-pixel jitter into a colour
//! target of the internal size (the render scale) and an "aux" target
//! holding, per pixel, the motion since the previous displayed frame and
//! the depth (`AUX_FORMAT`, encoding in [`TemporalDebug::aux`]). A resolve
//! pass then makes an output-size picture (the history) that the compose
//! pass reads in place of the 3D layer. Motion comes from smooth motion's
//! plan (`model2::smooth::Frame::motion`) through
//! `model2::raster::build_motion`.
//!
//! The resolve (`temporal.wgsl`, which describes the algorithm) blends
//! each frame into the history: the jittered samples reconstructed at
//! each output pixel, the history reprojected by the motion and clipped
//! into this frame's neighbourhood, rejected where the surface wasn't
//! there. The blend factors are [`Tuning`].

use crate::{le, Placed, Poly, Renderer, Sources, Stipple, View, FORMAT};
use model2::raster::Centres;

/// The aux target: motion (r, g), 1/z (b), expected previous 1/z (a).
pub(crate) const AUX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The accumulated picture: premultiplied RGBA, display values.
const HISTORY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Whether the adapter can do temporal anti-aliasing: 16-bit float
/// targets that can be drawn to, read and filtered (always on WebGPU; may
/// be missing on the OpenGL fallback).
pub fn temporal_supported(adapter: &wgpu::Adapter) -> bool {
    let f = adapter.get_texture_format_features(AUX_FORMAT);
    f.allowed_usages.contains(wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING)
        && f.flags.contains(wgpu::TextureFormatFeatureFlags::FILTERABLE)
}

/// Which displayed image a frame is: game frame `frame`
/// (`smooth::Frame::frame`) at `t` on its in-between axis (1 for the
/// game's own frame, less for an in-between).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameTag {
    pub frame: u64,
    pub t: f32,
}

impl FrameTag {
    /// The game's own frame `frame` (t = 1).
    pub fn game(frame: u64) -> Self {
        FrameTag { frame, t: 1.0 }
    }
}

/// How a displayed frame follows the last one ([`step`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// Start accumulating again (a cut, or time out of range).
    Reset,
    /// The same image again (a redraw): no motion, another jittered sample.
    Repeat,
    /// The previous image was at `tp` on this frame's axis from its
    /// previous pose (0) to its own (1): `raster::MotionIn::tp`.
    Blend { tp: f32 },
    /// No motion data: drawn without (no previous tag, or none given).
    NoMotion,
}

/// The step from the frame last displayed (`last`; None: none, or one
/// without a tag) to `now`; `cut` from `now`'s motion data.
pub fn step(last: Option<FrameTag>, now: FrameTag, cut: bool) -> Step {
    let Some(last) = last else { return Step::NoMotion };
    if cut && now.frame != last.frame {
        return Step::Reset;
    }
    if last == now {
        return Step::Repeat;
    }
    // Game frames apart, plus where the last image was on its own axis.
    let tp = (last.frame as f64 - now.frame as f64 + last.t as f64) as f32;
    if !(-2.0..now.t).contains(&tp) {
        return Step::Reset;
    }
    Step::Blend { tp }
}

/// Motion data for [`Renderer::present_bus_motion`]: one game frame's
/// (`smooth::Frame::motion`) and which image of it is drawn.
#[derive(Clone, Copy, Debug)]
pub struct BusMotion<'a> {
    pub tag: FrameTag,
    /// The frame's list posed with the frame before's transforms.
    pub s0: &'a [u32],
    /// The frame's list posed with its own; None: the bus engine's stream.
    pub s1: Option<&'a [u32]>,
    pub valid: &'a [bool],
    pub cut: bool,
}

impl<'a> BusMotion<'a> {
    /// `f`'s motion data, drawn as `tag`.
    pub fn of(f: &'a model2::smooth::Frame, tag: FrameTag, s1: Option<&'a [u32]>) -> Self {
        let m = f.motion();
        BusMotion { tag, s0: &m.s0, s1, valid: &m.valid, cut: m.cut }
    }
}

/// The last temporal frame's internal targets, read back
/// ([`Renderer::temporal_debug`]).
#[derive(Clone, Debug)]
pub struct TemporalDebug {
    /// Internal size.
    pub width: u32,
    pub height: u32,
    /// RGBA, premultiplied (alpha 0.5 on blended stipple over nothing).
    pub colour: Vec<u8>,
    /// Per pixel:
    /// - `[0]`, `[1]`: motion in output pixels, from where this pixel's
    ///   (unjittered) sample is to where that point was in the previous
    ///   displayed image (0 without motion data), within +-16384;
    /// - `[2]`: 1/z of the surface drawn, 6.2e-5 to 6.0e4, negative for
    ///   a blended stipple polygon; 0 where nothing is drawn;
    /// - `[3]`: the 1/z that point had in the previous image (same range),
    ///   or 0: no motion data. (The target holds it negative where the
    ///   polygon is thin: that is `thin`.)
    pub aux: Vec<[f32; 4]>,
    /// Per pixel: the surface drawn is a thin polygon ([`Tuning::thin`]).
    pub thin: Vec<bool>,
    /// The jitter in internal pixels: pixel (i, j) holds the sample at
    /// `(i + 0.5 - jitter.0, j + 0.5 - jitter.1)` internal pixels.
    pub jitter: (f32, f32),
    /// Internal over output size, per axis.
    pub rs: (f32, f32),
    pub step: Step,
}

/// Point `i` of the Halton sequence in `base`, in [0, 1).
fn halton(mut i: u32, base: u32) -> f32 {
    let (mut f, mut r) = (1.0f32, 0.0f32);
    while i > 0 {
        f /= base as f32;
        r += f * (i % base) as f32;
        i /= base;
    }
    r
}

/// The jitter in internal pixels (each within +-0.5) for the frame
/// `count` frames after a reset at render scale `scale`: none for the
/// first, so a single frame is the plain picture; then Halton(2, 3) over
/// a cycle of 8 to 32 frames (more when upscaling, so each output pixel
/// gets as many samples).
pub fn jitter(count: u32, scale: f32) -> (f32, f32) {
    if count == 0 {
        return (0.0, 0.0);
    }
    let len = (8.0 / (scale * scale)).round().clamp(8.0, 32.0) as u32;
    let i = 1 + count % len;
    (halton(i, 2) - 0.5, halton(i, 3) - 0.5)
}

/// IEEE half to single.
pub(crate) fn f16_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let e = (h >> 10) & 0x1f;
    let m = (h & 0x3ff) as f32;
    sign * match e {
        0 => m * 2f32.powi(-24),
        31 if m == 0.0 => f32::INFINITY,
        31 => f32::NAN,
        e => (1.0 + m / 1024.0) * 2f32.powi(e as i32 - 15),
    }
}

/// What the polygons' previous positions mean in a temporal frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Motion3d {
    /// No motion data.
    None,
    /// The same image as the last: zero motion everywhere.
    Repeat,
    /// `Placed::prev`, where a polygon has it.
    Prev,
}

impl Motion3d {
    /// A polygon's previous positions (empty: where it is), or None
    /// without motion data. Data with a point not finite or not in front
    /// counts as none.
    pub(crate) fn of(self, pl: &Placed) -> Option<&[[f32; 3]]> {
        match self {
            Motion3d::None => None,
            Motion3d::Repeat => Some(&[]),
            Motion3d::Prev => (!pl.prev.is_empty()
                && pl.prev.len() == pl.points.len()
                && pl.prev.iter().all(|p| p.iter().all(|c| c.is_finite()) && p[2] > 0.0))
            .then_some(&pl.prev[..]),
        }
    }
}

/// The temporal side of a raster pass (`Renderer::encode_3d`).
pub(crate) struct TemporalPass<'a> {
    pub aux: &'a wgpu::TextureView,
    /// Internal size, and internal over output size.
    pub size: (u32, u32),
    pub rs: (f32, f32),
    /// The jitter in output pixels.
    pub jitter_d: (f32, f32),
    /// Texture derivative scale for filtered pack images (a LOD bias).
    pub lod: f32,
    pub keep: bool,
    pub motion: Motion3d,
    /// Which polygons are thin ([`Tuning::thin`], `thin_len`, internal
    /// px); None: none.
    pub thin: Option<(f32, f32)>,
    /// Thin polygons are widened to this many internal px, drawn at
    /// their coverage ([`Tuning::cover`]; 0: not widened).
    pub cover: f32,
    /// Only polygons whose long edges are their own ([`Tuning::isolated`]).
    pub isolated: bool,
}

/// A thin polygon ([`thin`]): how narrow it is, and how to widen it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Thin {
    /// Its width across, internal px.
    pub width: f32,
    /// The direction across (unit, internal px) and its middle along it.
    pub normal: (f32, f32),
    pub mid: f32,
}

impl Thin {
    /// The polygon's coverage once widened to `to` internal px (`widen`).
    pub(crate) fn coverage(&self, to: f32) -> f32 {
        (self.width / to).clamp(0.0, 1.0)
    }

    /// Point (x, y) (output px) moved so the polygon is `to` internal px
    /// wide across, about its middle line: the offset in output px.
    pub(crate) fn widen(&self, (x, y): (f32, f32), rs: (f32, f32), to: f32) -> (f32, f32) {
        let (qx, qy) = (x * rs.0, y * rs.1);
        let d = qx * self.normal.0 + qy * self.normal.1 - self.mid;
        let k = d * (to / self.width.max(0.01) - 1.0);
        (k * self.normal.0 / rs.0, k * self.normal.1 / rs.1)
    }
}

/// The frame's polygon edges, to tell a thin feature (a wire, a rail: its
/// long edges its own) from a sliver of a mesh (a distant terrain or road
/// strip seen edge-on, whose long edges its neighbours share and which the
/// mesh around it already covers).
pub(crate) struct Edges(std::collections::HashMap<[i32; 4], u32>);

impl Edges {
    fn key(a: (f32, f32), b: (f32, f32)) -> [i32; 4] {
        let q = |v: f32| (v * 8.0).round() as i32;
        let (a, b) = ((q(a.0), q(a.1)), (q(b.0), q(b.1)));
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        [a.0, a.1, b.0, b.1]
    }

    /// Every edge of every polygon (points in output px).
    pub(crate) fn new<'a>(polys: impl Iterator<Item = &'a [model2::raster::SVtx]>) -> Self {
        let mut m = std::collections::HashMap::new();
        for p in polys {
            for k in 0..p.len() {
                let (a, b) = (&p[k], &p[(k + 1) % p.len()]);
                *m.entry(Self::key((a.x, a.y), (b.x, b.y))).or_insert(0) += 1;
            }
        }
        Edges(m)
    }

    /// Whether a long edge of a polygon (points in output px; at least
    /// half as long as its longest) is another polygon's too.
    pub(crate) fn long_edge_shared(&self, points: &[(f32, f32)]) -> bool {
        let n = points.len();
        let len = |k: usize| {
            let (a, b) = (points[k], points[(k + 1) % n]);
            (a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)
        };
        let longest = (0..n).map(len).fold(0.0f32, f32::max);
        (0..n).any(|k| {
            len(k) >= 0.25 * longest && self.0.get(&Self::key(points[k], points[(k + 1) % n])).is_some_and(|&c| c > 1)
        })
    }
}

/// Whether a convex polygon (points in output px) is thin at internal
/// over output size `rs`: narrower than `width` internal pixels across
/// (the least, over its edges, of the farthest point from the edge's
/// line: a convex polygon's width) and at least `len` long (so a mesh of
/// small polygons far away is not a thin feature). Such a polygon is hit
/// by some jitter phases' samples and missed by others.
pub(crate) fn thin(points: &[(f32, f32)], rs: (f32, f32), (width, len): (f32, f32)) -> Option<Thin> {
    let p: Vec<(f32, f32)> = points.iter().map(|&(x, y)| (x * rs.0, y * rs.1)).collect();
    let n = p.len();
    if n < 3 {
        return None;
    }
    let mut best: Option<Thin> = None;
    let mut longest = 0.0f32;
    for k in 0..n {
        let (a, b) = (p[k], p[(k + 1) % n]);
        let (ex, ey) = (b.0 - a.0, b.1 - a.1);
        let l = (ex * ex + ey * ey).sqrt();
        for q in &p {
            longest = longest.max(((q.0 - a.0).powi(2) + (q.1 - a.1).powi(2)).sqrt());
        }
        if l.is_nan() || l <= 1e-6 {
            continue;
        }
        let normal = (-ey / l, ex / l);
        let along = |q: &(f32, f32)| q.0 * normal.0 + q.1 * normal.1;
        let (lo, hi) =
            p.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), q| (lo.min(along(q)), hi.max(along(q))));
        if best.is_none_or(|t| hi - lo < t.width) {
            best = Some(Thin { width: hi - lo, normal, mid: (lo + hi) / 2.0 });
        }
    }
    best.filter(|t| t.width < width && longest >= len && t.width.is_finite())
}

impl TemporalPass<'_> {
    /// The `ScreenT` uniform (raster.wgsl), with the clip rectangles at
    /// `clip_base` in the outlines buffer.
    pub(crate) fn screen_t(&self, clip_base: u32) -> Vec<u8> {
        let mut b = le([self.rs.0, self.rs.1, self.jitter_d.0, self.jitter_d.1, self.lod].map(f32::to_le_bytes));
        b.extend(le([self.keep as u32, clip_base, 0].map(u32::to_le_bytes)));
        b
    }
}

/// The last temporal frame's internal targets.
pub(crate) struct LastFrame {
    colour: wgpu::Texture,
    aux: wgpu::Texture,
    history: wgpu::Texture,
    size: (u32, u32),
    jitter: (f32, f32),
    rs: (f32, f32),
    step: Step,
    /// What the resolve decided (`fs_inspect`), when asked for.
    inspect: Option<wgpu::Texture>,
}

/// Temporal state kept by the renderer.
pub(crate) struct Temporal {
    pub supported: bool,
    /// Frames resolved since the last reset.
    pub count: u32,
    /// The last frame's tag (None: it had none).
    pub last: Option<FrameTag>,
    /// The last frame's window centres.
    pub centres: Vec<Centres>,
    /// Set for the next frame (`set_temporal_frame`).
    pub pending: Option<(Option<FrameTag>, Step, Vec<Centres>)>,
    /// Test hook: jitter on.
    pub jitter: bool,
    /// Which of the ping-pong targets ("a" / "b") this frame writes.
    flip: bool,
    pub last_frame: Option<LastFrame>,
    /// Test hook: the resolve copies the nearest sample, no history.
    pub passthrough: bool,
    /// Test hook: also write what the resolve decided (`fs_inspect`).
    pub inspect: bool,
    pub tuning: Tuning,
    shader: wgpu::ShaderModule,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// The resolve's pipelines: `fs_resolve`, `fs_inspect`.
    resolve: [Option<wgpu::RenderPipeline>; 2],
    /// The sharpening pass (`fs_sharpen`): its layout (the resolve's
    /// uniform, the resolved picture) and pipeline.
    sharpen_layout: wgpu::BindGroupLayout,
    sharpen: Option<wgpu::RenderPipeline>,
}

/// The format of the resolve's decisions (`fs_inspect`).
const INSPECT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;

/// The resolve's constants (`Resolve::k0..k2` in temporal.wgsl).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    /// Blend factor of a sample at the pixel's centre with trusted history.
    pub alpha: f32,
    /// Reconstruction kernel: weight exp(-kernel d^2), d in output px; at
    /// 100% and at 50% (and below), interpolated between.
    pub kernel: (f32, f32),
    /// Clip box of a moving pixel: mean +- gamma deviations, at 100% and
    /// at 50% (and below), interpolated between.
    pub gamma: (f32, f32),
    /// Blend factor at fast motion, reached at `fast_len` output px.
    pub alpha_fast: f32,
    pub fast_len: f32,
    /// Blend factor without motion data (clip only).
    pub alpha_nodata: f32,
    /// Blend factor on blended stipple.
    pub alpha_sheer: f32,
    /// Blend factor when motion changed since the last frame: from
    /// `vel_lo` px of change, full at `vel_lo + vel_span`.
    pub alpha_vel: f32,
    pub vel_lo: f32,
    pub vel_span: f32,
    /// Depth test against the last frame: 1/z within this share.
    pub depth_tol: f32,
    /// Sharpening of the resolved picture before compose (0 off, 1 the
    /// most; `fs_sharpen`). The history itself is not sharpened.
    pub sharpen: f32,
    /// The history's resampling: Keys' cubic with this C (0.5:
    /// Catmull-Rom; more is sharper).
    pub cubic: f32,
    /// Thin features: a polygon narrower than this many internal pixels
    /// (and at least `thin_len` long) is thin (0: none are; the handling
    /// is off).
    pub thin: f32,
    pub thin_len: f32,
    /// Thin polygons are drawn widened to this many internal pixels at
    /// their coverage (over what is behind), so every jitter phase's
    /// samples hit them; 0: drawn as they are.
    pub cover: f32,
    /// Only a polygon none of whose long edges (half its longest or more)
    /// another polygon shares is thin (1), or any (0): a sliver of a mesh
    /// seen edge-on is not a thin feature, the mesh around it covers it.
    pub isolated: f32,
    /// A thin polygon's motion carries the pixels around it (1) or not
    /// (0).
    pub thin_motion: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            alpha: 0.12,
            kernel: (3.0, 3.0),
            gamma: (1.25, 2.0),
            alpha_fast: 0.6,
            fast_len: 8.0,
            alpha_nodata: 0.8,
            alpha_sheer: 0.25,
            alpha_vel: 0.8,
            vel_lo: 0.5,
            vel_span: 4.0,
            depth_tol: 0.1,
            sharpen: 0.0,
            cubic: 0.75,
            thin: 1.0,
            thin_len: 2.0,
            cover: 1.0,
            isolated: 1.0,
            thin_motion: 1.0,
        }
    }
}

impl Tuning {
    /// `name=value,...` over the defaults (for tuning runs; unknown names
    /// are an error).
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut t = Tuning::default();
        for kv in s.split(',').filter(|kv| !kv.is_empty()) {
            let (k, v) = kv.split_once('=').ok_or_else(|| format!("{kv}: want name=value"))?;
            let v: f32 = v.parse().map_err(|_| format!("{kv}: not a number"))?;
            match k {
                "alpha" => t.alpha = v,
                "kernel" => t.kernel.0 = v,
                "kernel50" => t.kernel.1 = v,
                "gamma" => t.gamma.0 = v,
                "gamma50" => t.gamma.1 = v,
                "fast" => t.alpha_fast = v,
                "fast_len" => t.fast_len = v,
                "nodata" => t.alpha_nodata = v,
                "sheer" => t.alpha_sheer = v,
                "vel" => t.alpha_vel = v,
                "vel_lo" => t.vel_lo = v,
                "vel_span" => t.vel_span = v,
                "depth_tol" => t.depth_tol = v,
                "sharpen" => t.sharpen = v,
                "cubic" => t.cubic = v,
                "thin" => t.thin = v,
                "thin_len" => t.thin_len = v,
                "cover" => t.cover = v,
                "isolated" => t.isolated = v,
                "thin_motion" => t.thin_motion = v,
                _ => return Err(format!("{k}: no such tuning constant")),
            }
        }
        Ok(t)
    }

    fn words(&self) -> [f32; 16] {
        [
            self.alpha,
            self.kernel.0,
            self.gamma.0,
            self.gamma.1,
            self.alpha_fast,
            self.fast_len,
            self.alpha_nodata,
            self.alpha_sheer,
            self.alpha_vel,
            self.vel_lo,
            self.vel_span,
            self.depth_tol,
            self.kernel.1,
            self.sharpen,
            self.cubic,
            (self.thin > 0.0 && self.thin_motion > 0.0) as u8 as f32,
        ]
    }
}

impl Temporal {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("temporal"),
            source: wgpu::ShaderSource::Wgsl(include_str!("temporal.wgsl").into()),
        });
        let filterable = wgpu::BindGroupLayoutEntry {
            binding: 4,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("resolve"),
            entries: &[
                crate::uniform(0, wgpu::ShaderStages::FRAGMENT),
                crate::texture(1),
                crate::texture(2),
                crate::texture(3),
                filterable,
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sharpen_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sharpen"),
            entries: &[crate::uniform(0, wgpu::ShaderStages::FRAGMENT), crate::texture(6)],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("history"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Temporal {
            supported: true,
            count: 0,
            last: None,
            centres: Vec::new(),
            pending: None,
            jitter: true,
            flip: false,
            last_frame: None,
            passthrough: false,
            inspect: false,
            tuning: Tuning::default(),
            shader,
            layout,
            sampler,
            resolve: [None, None],
            sharpen_layout,
            sharpen: None,
        }
    }

    /// Start accumulating again.
    pub(crate) fn reset(&mut self) {
        self.count = 0;
        self.last = None;
        self.centres.clear();
    }
}

/// What the resolve reads and writes: the pass boundary (an FSR-style
/// upscaler would take the same).
pub(crate) struct ResolveIo<'a> {
    /// This frame's internal colour (premultiplied) and aux targets.
    pub colour: &'a wgpu::TextureView,
    pub aux: &'a wgpu::TextureView,
    /// The last frame's aux target (internal size) and history (output
    /// size); not read when `count` is 0.
    pub aux_prev: &'a wgpu::TextureView,
    pub history_prev: &'a wgpu::TextureView,
    /// The history to write (output size).
    pub history_out: &'a wgpu::TextureView,
    pub render: (u32, u32),
    pub display: (u32, u32),
    /// This frame's and the last's, in internal pixels.
    pub jitter: (f32, f32),
    pub jitter_prev: (f32, f32),
    /// Frames resolved since the last reset (0: this one starts afresh).
    pub count: u32,
    /// Copy the nearest sample (test hook).
    pub passthrough: bool,
    pub tuning: Tuning,
}

impl Renderer {
    /// The last frame drawn with temporal on: the internal colour and aux
    /// targets, read back. Blocks: native only.
    pub fn temporal_debug(&self) -> Option<TemporalDebug> {
        let l = self.temporal.last_frame.as_ref()?;
        let (w, h) = l.size;
        let colour = self.read_back_bpp(self.device.create_command_encoder(&Default::default()), &l.colour, w, h, 4);
        let raw = self.read_back_bpp(self.device.create_command_encoder(&Default::default()), &l.aux, w, h, 8);
        let (aux, thin) = raw
            .as_chunks::<8>()
            .0
            .iter()
            .map(|p| {
                let a = [0, 2, 4, 6].map(|k| f16_to_f32(u16::from_le_bytes([p[k], p[k + 1]])));
                ([a[0], a[1], a[2], a[3].abs()], a[3] < 0.0)
            })
            .unzip();
        Some(TemporalDebug { width: w, height: h, colour, aux, thin, jitter: l.jitter, rs: l.rs, step: l.step })
    }

    /// The history the last temporal frame wrote (output size,
    /// premultiplied RGBA), read back. Blocks: native only.
    pub fn temporal_history(&self) -> Option<(u32, u32, Vec<[f32; 4]>)> {
        let l = self.temporal.last_frame.as_ref()?;
        let (w, h) = (l.history.width(), l.history.height());
        let raw = self.read_back_bpp(self.device.create_command_encoder(&Default::default()), &l.history, w, h, 8);
        let px = raw
            .as_chunks::<8>()
            .0
            .iter()
            .map(|p| [0, 2, 4, 6].map(|k| f16_to_f32(u16::from_le_bytes([p[k], p[k + 1]]))))
            .collect();
        Some((w, h, px))
    }

    /// Sharpening of the temporal picture after the resolve, before
    /// compose (0 off, 1 the most; [`crate::TemporalTuning::sharpen`]): the 3D
    /// layer only, so tiles and the HUD are untouched. The history is not
    /// sharpened.
    pub fn set_temporal_sharpen(&mut self, strength: f32) {
        self.temporal.tuning.sharpen = strength.clamp(0.0, 1.0);
    }

    /// Test hook: from the next temporal frame on, also record what the
    /// resolve decides per pixel ([`Renderer::temporal_inspect`]); a
    /// second pass, the picture is unchanged.
    pub fn set_temporal_inspect(&mut self, on: bool) {
        self.temporal.inspect = on;
    }

    /// What the last temporal frame's resolve decided per output pixel,
    /// read back, with [`Renderer::set_temporal_inspect`] on (else None):
    /// `[rules, this frame's share, clip distance, trust term]` as
    /// temporal.wgsl's `Res::dbg` describes. Blocks: native only.
    pub fn temporal_inspect(&self) -> Option<(u32, u32, Vec<[f32; 4]>)> {
        let t = self.temporal.last_frame.as_ref()?.inspect.as_ref()?;
        let (w, h) = (t.width(), t.height());
        let raw = self.read_back_bpp(self.device.create_command_encoder(&Default::default()), t, w, h, 16);
        let px = raw
            .as_chunks::<16>()
            .0
            .iter()
            .map(|p| std::array::from_fn(|k| f32::from_le_bytes(p[k * 4..k * 4 + 4].try_into().unwrap())))
            .collect();
        Some((w, h, px))
    }

    /// A texture kept between frames by label, re-created if its size or
    /// format changed; true when (re-)created.
    fn kept_target(
        &mut self,
        label: &'static str,
        (w, h): (u32, u32),
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
    ) -> (wgpu::Texture, bool) {
        let same = |t: &wgpu::Texture| t.width() == w && t.height() == h && t.format() == format && t.usage() == usage;
        if let Some((_, t)) = self.textures.iter().find(|(l, t)| *l == label && same(t)) {
            return (t.clone(), false);
        }
        let t = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        self.textures.retain(|(l, _)| *l != label);
        self.textures.push((label, t.clone()));
        (t, true)
    }

    /// The 3D layer with temporal on: drawn jittered at the internal size
    /// with the aux target, then resolved into the history, whose view
    /// (output size, `HISTORY_FORMAT`) is returned for compose.
    pub(crate) fn encode_temporal(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        polys: &[Poly],
        placed: &[Placed],
        src: &Sources,
        view: &View,
    ) -> wgpu::TextureView {
        let (tag, step, centres) = self.temporal.pending.take().unwrap_or((None, Step::NoMotion, Vec::new()));
        if step == Step::Reset {
            self.temporal.count = 0;
        }
        self.temporal.last = tag;
        self.temporal.centres = centres;
        let scale = self.quality.temporal.min(100) as f32 / 100.0;
        let (dw, dh) = (view.width as u32, view.height as u32);
        let size = if self.quality.temporal >= 100 {
            (dw, dh)
        } else {
            (((dw as f32 * scale).round() as u32).max(1), ((dh as f32 * scale).round() as u32).max(1))
        };
        let rs = (size.0 as f32 / dw as f32, size.1 as f32 / dh as f32);
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST;
        // Both of each ping-pong pair (this frame's, the last's), so a new
        // one resets the history once, not on two frames running.
        let flip = self.temporal.flip as usize;
        let (colour, _) = self.kept_target("3d layer t", size, FORMAT, usage);
        let aux = ["3d aux a", "3d aux b"].map(|l| self.kept_target(l, size, AUX_FORMAT, usage));
        let history = ["history a", "history b"].map(|l| self.kept_target(l, (dw, dh), HISTORY_FORMAT, usage));
        if aux.iter().chain(&history).any(|(_, fresh)| *fresh) {
            self.temporal.count = 0;
        }
        let (aux_prev, history_prev) = (aux[1 - flip].0.clone(), history[1 - flip].0.clone());
        let aux = aux[flip].0.clone();
        let history = history[flip].0.clone();
        let count = self.temporal.count;
        let jitter_prev = self.temporal.last_frame.as_ref().map_or((0.0, 0.0), |l| l.jitter);
        let j = if self.temporal.jitter { jitter(count, scale) } else { (0.0, 0.0) };
        // Keep the pattern only where it can be reconstructed.
        let keep = self.quality.stipple == Stipple::Keep && view.sy * rs.1 >= 1.5;
        let motion = match step {
            Step::Repeat => Motion3d::Repeat,
            Step::Blend { .. } => Motion3d::Prev,
            Step::Reset | Step::NoMotion => Motion3d::None,
        };
        let colour_view = colour.create_view(&Default::default());
        let aux_view = aux.create_view(&Default::default());
        let pass = TemporalPass {
            aux: &aux_view,
            size,
            rs,
            jitter_d: (j.0 / rs.0, j.1 / rs.1),
            lod: rs.0.min(rs.1),
            keep,
            motion,
            thin: (self.temporal.tuning.thin > 0.0)
                .then_some((self.temporal.tuning.thin, self.temporal.tuning.thin_len)),
            cover: self.temporal.tuning.cover,
            isolated: self.temporal.tuning.isolated > 0.0,
        };
        self.encode_3d(enc, polys, placed, src, view, &colour_view, Some(&pass));
        let history_view = history.create_view(&Default::default());
        let io = ResolveIo {
            colour: &colour_view,
            aux: &aux_view,
            aux_prev: &aux_prev.create_view(&Default::default()),
            history_prev: &history_prev.create_view(&Default::default()),
            history_out: &history_view,
            render: size,
            display: (dw, dh),
            jitter: j,
            jitter_prev,
            count,
            passthrough: self.temporal.passthrough,
            tuning: self.temporal.tuning,
        };
        self.encode_resolve(enc, &io, None);
        let inspect = self.temporal.inspect.then(|| {
            let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
            let (t, _) = self.kept_target("resolve inspect", (dw, dh), INSPECT_FORMAT, usage);
            self.encode_resolve(enc, &io, Some(&t.create_view(&Default::default())));
            t
        });
        self.temporal.last_frame = Some(LastFrame { colour, aux, history, size, jitter: j, rs, step, inspect });
        self.temporal.count = count.saturating_add(1);
        self.temporal.flip = !self.temporal.flip;
        if self.temporal.tuning.sharpen > 0.0 {
            let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
            let (t, _) = self.kept_target("3d sharpened", (dw, dh), HISTORY_FORMAT, usage);
            let view = t.create_view(&Default::default());
            self.encode_sharpen(enc, &io, &view);
            return view;
        }
        history_view
    }

    /// The sharpening pass (`fs_sharpen`): `io.history_out` sharpened into
    /// `out` (output size, `HISTORY_FORMAT`).
    fn encode_sharpen(&mut self, enc: &mut wgpu::CommandEncoder, io: &ResolveIo, out: &wgpu::TextureView) {
        let pipeline = match &self.temporal.sharpen {
            Some(p) => p.clone(),
            None => {
                let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("sharpen"),
                    bind_group_layouts: &[Some(&self.temporal.sharpen_layout)],
                    immediate_size: 0,
                });
                let p = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("sharpen"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &self.temporal.shader,
                        entry_point: Some("vs"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &self.temporal.shader,
                        entry_point: Some("fs_sharpen"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: HISTORY_FORMAT,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                });
                self.temporal.sharpen = Some(p.clone());
                p
            }
        };
        let params = resolve_params(io);
        let params = wgpu::util::DeviceExt::create_buffer_init(
            &self.device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("sharpen"),
                contents: &params,
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sharpen"),
            layout: &self.temporal.sharpen_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(io.history_out) },
            ],
        });
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sharpen"),
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

    /// The resolve (`fs_resolve`) into `io.history_out`, or with `inspect`
    /// what it decides (`fs_inspect`) into that target instead.
    fn encode_resolve(&mut self, enc: &mut wgpu::CommandEncoder, io: &ResolveIo, inspect: Option<&wgpu::TextureView>) {
        let (k, entry, format) = match inspect {
            None => (0, "fs_resolve", HISTORY_FORMAT),
            Some(_) => (1, "fs_inspect", INSPECT_FORMAT),
        };
        let pipeline = match &self.temporal.resolve[k] {
            Some(p) => p.clone(),
            None => {
                let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("resolve"),
                    bind_group_layouts: &[Some(&self.temporal.layout)],
                    immediate_size: 0,
                });
                let p = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("resolve"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &self.temporal.shader,
                        entry_point: Some("vs"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &self.temporal.shader,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                });
                self.temporal.resolve[k] = Some(p.clone());
                p
            }
        };
        let params = resolve_params(io);
        let params = wgpu::util::DeviceExt::create_buffer_init(
            &self.device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("resolve"),
                contents: &params,
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("resolve"),
            layout: &self.temporal.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(io.colour) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(io.aux) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(io.aux_prev) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(io.history_prev) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&self.temporal.sampler) },
            ],
        });
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("resolve"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: inspect.unwrap_or(io.history_out),
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

/// The resolve's uniform (`Resolve` in temporal.wgsl).
fn resolve_params(io: &ResolveIo) -> Vec<u8> {
    let mut params = le([io.display.0, io.display.1, io.render.0, io.render.1].map(|v| (v as f32).to_le_bytes()));
    params.extend(le(
        [io.jitter.0, io.jitter.1, io.jitter_prev.0, io.jitter_prev.1, io.count as f32].map(f32::to_le_bytes)
    ));
    params.extend(le([io.passthrough as u32, 0, 0].map(u32::to_le_bytes)));
    params.extend(le(io.tuning.words().map(f32::to_le_bytes)));
    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps() {
        let tag = |frame, t| FrameTag { frame, t };
        assert_eq!(step(None, tag(5, 1.0), false), Step::NoMotion);
        assert_eq!(step(Some(tag(4, 1.0)), tag(5, 1.0), false), Step::Blend { tp: 0.0 });
        assert_eq!(step(Some(tag(5, 1.0)), tag(5, 1.0), false), Step::Repeat);
        assert_eq!(step(Some(tag(5, 0.5)), tag(5, 1.0), false), Step::Blend { tp: 0.5 });
        assert_eq!(step(Some(tag(5, 0.5)), tag(6, 0.5), false), Step::Blend { tp: -0.5 });
        assert_eq!(step(Some(tag(3, 1.0)), tag(5, 1.0), false), Step::Blend { tp: -1.0 });
        // Too far, backwards, a cut.
        assert_eq!(step(Some(tag(1, 1.0)), tag(5, 1.0), false), Step::Reset);
        assert_eq!(step(Some(tag(6, 1.0)), tag(5, 1.0), false), Step::Reset);
        assert_eq!(step(Some(tag(5, 1.0)), tag(5, 0.5), false), Step::Reset);
        assert_eq!(step(Some(tag(4, 1.0)), tag(5, 1.0), true), Step::Reset);
        assert_eq!(step(Some(tag(5, 1.0)), tag(5, 1.0), true), Step::Repeat);
    }

    #[test]
    fn jitter_sequence() {
        assert_eq!(jitter(0, 1.0), (0.0, 0.0));
        let (x, y) = jitter(1, 1.0);
        assert!((x + 0.25).abs() < 1e-6 && (y - 1.0 / 6.0).abs() < 1e-6, "{x} {y}");
        for count in 1..100 {
            let (x, y) = jitter(count, 0.5);
            assert!((-0.5..0.5).contains(&x) && (-0.5..0.5).contains(&y));
        }
        // Cycles of 8 at full scale, 32 at half.
        assert_eq!(jitter(3, 1.0), jitter(11, 1.0));
        assert_ne!(jitter(3, 0.5), jitter(11, 0.5));
        assert_eq!(jitter(3, 0.5), jitter(35, 0.5));
    }

    use crate::Quality;
    use model2::raster::{place, SVtx, Texture};
    use model2::video::{Hud, Layers2d, HEIGHT, WIDTH};

    /// A quad from (x0, y0) to (x1, y1) in game pixels at 1/z `iz`.
    fn quad(x0: f32, y0: f32, x1: f32, y1: f32, iz: f32, key: f32, window: usize) -> Poly {
        let v = |x, y, u: f32, w: f32| SVtx { x, y, iz, uz: u * iz, vz: w * iz };
        Poly {
            points: vec![v(x0, y0, 0.0, 0.0), v(x1, y0, 31.0, 0.0), v(x1, y1, 31.0, 31.0), v(x0, y1, 0.0, 31.0)],
            key,
            window,
            clip: if window == 0 { (0.0, 0.0, WIDTH as f32, HEIGHT as f32) } else { (100.5, 90.0, 300.25, 250.0) },
            texture: None,
            colour: [7, 19, 28],
            luma: 200,
            checker: false,
            header: [0; 4],
            block: 0,
            at: 0,
            prev: Vec::new(),
            shadow: false,
            lit: Vec::new(),
            pixel: None,
            lit_range: [0, 255],
            fine: false,
        }
    }

    /// Flat, textured, checkerboard and moving polygons over two windows.
    fn scene() -> Vec<Poly> {
        let tex = Texture {
            x0: 64,
            y0: 32,
            w: 32,
            h: 32,
            mirror_x: false,
            mirror_y: true,
            bank: 0,
            translucent: true,
            luma_table: 3,
        };
        let mut p = vec![quad(-20.0, -10.0, 520.0, 400.0, 0.001, 900.0, 0)];
        let mut t = quad(40.3, 30.7, 260.9, 220.2, 0.004, 500.0, 0);
        t.texture = Some(tex);
        // Not flat in depth, and turned.
        t.points[2].iz = 0.006;
        t.points[1].x += 30.0;
        p.push(t);
        let mut c = quad(150.2, 120.6, 400.1, 300.4, 0.005, 400.0, 0);
        c.checker = true;
        c.colour = [31, 2, 9];
        p.push(c);
        // Moving: where it was 10 game pixels right and 5 up, at the same
        // depth (W = 1).
        let mut m = quad(300.0, 40.0, 420.0, 150.0, 0.01, 100.0, 0);
        m.prev = m.points.iter().map(|v| [v.x + 10.0, v.y - 5.0, 1.0]).collect();
        m.colour = [3, 30, 3];
        p.push(m);
        // Previous position nearly at the eye: huge but finite motion.
        let mut n = quad(20.0, 300.0, 120.0, 370.0, 0.01, 90.0, 0);
        n.prev = n.points.iter().map(|v| [v.x + 1.0, v.y, 1e-3]).collect();
        p.push(n);
        let mut w = quad(80.0, 80.0, 330.0, 270.0, 0.02, 50.0, 1);
        w.points.pop();
        w.colour = [20, 20, 31];
        p.push(w);
        p
    }

    /// Temporal on at 100% with no jitter (the first frame after a reset,
    /// or the test hook) and the checkerboard kept draws the frame the
    /// off path does, byte for byte, with and without the depth buffer;
    /// and the off path after temporal frames is a fresh renderer's.
    #[test]
    fn identity_at_100_without_jitter() {
        let Ok(mut gpu) = Renderer::new() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let mut seed = 0x1234_5678u32;
        let mut bytes = |n: usize| {
            (0..n)
                .map(|_| {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (seed >> 24) as u8
                })
                .collect::<Vec<u8>>()
        };
        let (render, palette) = (bytes(0x480000), bytes(0x20000));
        let src = Sources {
            texture_rom: &[],
            texture_point_ram: &[],
            render_ram: &render,
            palette: &palette,
            xoff: 0.0,
            yoff: 0.0,
        };
        // Two output pixels a game pixel: Keep stays on.
        let view = View::new(768, 4.0 / 3.0, Hud::Center);
        let base = (0..WIDTH * HEIGHT).flat_map(|i| [(i % 251) as u8, (i % 13 * 19) as u8, 90, 255]).collect();
        let layers = Layers2d {
            base,
            side: 0,
            hud: vec![0; WIDTH * HEIGHT * 4],
            ext_w: WIDTH,
            backdrop: [0, 0, 0],
            tiles: None,
        };
        let mut polys = scene();
        let placed = place(&mut polys, &view);
        let fresh = |depth| {
            let mut g = Renderer::new().unwrap();
            g.set_quality(Quality { depth, ..Default::default() });
            g.frame(&polys, &placed, &src, &view, &layers)
        };
        for depth in [false, true] {
            let off = fresh(depth);
            let q = Quality { depth, temporal: 100, stipple: Stipple::Keep, ..Default::default() };
            assert_eq!(gpu.set_quality(q), q);
            assert_eq!(gpu.frame(&polys, &placed, &src, &view, &layers), off, "first temporal frame, depth {depth}");
            // More frames, with motion, the jitter off and the resolve
            // passing the samples through by the hooks: the raster side
            // alone draws the same.
            gpu.set_temporal_jitter(false);
            gpu.set_temporal_passthrough(true);
            for (k, step) in [Step::Repeat, Step::Blend { tp: 0.0 }].into_iter().enumerate() {
                gpu.set_temporal_frame(FrameTag::game(k as u64), step, Vec::new());
                assert_eq!(gpu.frame(&polys, &placed, &src, &view, &layers), off, "{step:?}, depth {depth}");
            }
            gpu.set_temporal_passthrough(false);
            // The aux target of that last frame (Blend: motion data).
            let d = gpu.temporal_debug().unwrap();
            assert_eq!((d.width, d.height, d.jitter), (view.width as u32, view.height as u32, (0.0, 0.0)));
            assert!(d.aux.iter().flatten().all(|c| c.is_finite()));
            let at = |x: f32, y: f32| {
                let (x, y) = view.to_output(x, y);
                d.aux[y as usize * d.width as usize + x as usize]
            };
            let m = at(360.0, 100.0);
            assert!((m[0] - 10.0 * view.sx).abs() < 0.05 && (m[1] + 5.0 * view.sy).abs() < 0.05, "{m:?}");
            assert!((m[2] - 0.01).abs() < 1e-5 && (m[3] - 0.01).abs() < 1e-5, "{m:?}");
            // Nearly at the eye: clamped; no motion data: zero, a = 0.
            let n = at(70.0, 335.0);
            assert!(n[0] == 16384.0 && n[1] == 16384.0 && (n[3] - 10.0).abs() < 0.01, "{n:?}");
            let s = at(5.0, 5.0);
            assert!(s == [0.0, 0.0, s[2], 0.0] && (s[2] - 0.001).abs() < 1e-6, "{s:?}");
            // Jittered frames differ, and the off path after is as fresh.
            gpu.set_temporal_jitter(true);
            assert_ne!(gpu.frame(&polys, &placed, &src, &view, &layers), off);
            gpu.set_quality(Quality { depth, ..Default::default() });
            assert_eq!(gpu.frame(&polys, &placed, &src, &view, &layers), off, "off after temporal, depth {depth}");
        }
        // Blended stipple: half of the checkerboard polygon over what is
        // behind it, sheer (negative 1/z) in the aux target.
        gpu.set_quality(Quality { temporal: 100, ..Default::default() });
        gpu.frame(&polys, &placed, &src, &view, &layers);
        let d = gpu.temporal_debug().unwrap();
        let (x, y) = view.to_output(390.0, 290.0);
        let i = y as usize * d.width as usize + x as usize;
        assert!(d.aux[i][2] < 0.0 && (d.aux[i][2] + 0.005).abs() < 1e-5, "{:?}", d.aux[i]);
        assert_eq!(d.colour[i * 4 + 3], 255, "blended over the background polygon");
    }

    /// Smooth shading (`Quality::shading`, raster.wgsl's `_l` entry points): with
    /// no lit polygon the frame is the off path's byte for byte, with and
    /// without depth and temporal; corner values ignored with shading
    /// flat; the lit pipelines draw a lit polygon whose corners all carry
    /// its own luma (+0.5, as `light` gives them) as the flat one, byte for
    /// byte; and varying corners agree with the software renderer.
    #[test]
    fn shading_off_path_and_lit_pipelines() {
        let Ok(gpu) = Renderer::new() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        drop(gpu);
        let mut seed = 0x0bad_cafeu32;
        let mut bytes = |n: usize| {
            (0..n)
                .map(|_| {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (seed >> 24) as u8
                })
                .collect::<Vec<u8>>()
        };
        let (render, palette) = (bytes(0x480000), bytes(0x20000));
        let src = Sources {
            texture_rom: &[],
            texture_point_ram: &[],
            render_ram: &render,
            palette: &palette,
            xoff: 0.0,
            yoff: 0.0,
        };
        let view = View::new(768, 4.0 / 3.0, Hud::Center);
        let base = (0..WIDTH * HEIGHT).flat_map(|i| [(i % 251) as u8, (i % 13 * 19) as u8, 90, 255]).collect();
        let layers = Layers2d {
            base,
            side: 0,
            hud: vec![0; WIDTH * HEIGHT * 4],
            ext_w: WIDTH,
            backdrop: [0, 0, 0],
            tiles: None,
        };
        let mut polys = scene();
        let placed = place(&mut polys, &view);
        let frame = |q: Quality, polys: &[Poly]| {
            let mut g = Renderer::new().unwrap();
            g.set_quality(q);
            g.frame(polys, &placed, &src, &view, &layers)
        };
        let vertex = |q: Quality| Quality { shading: crate::Shading::Vertex, crease: 40, ..q };
        // Corners at the polygon's own luma, and varying ones.
        let with = |f: &dyn Fn(&Poly, usize) -> f32| {
            polys
                .iter()
                .map(|p| Poly { lit: (0..p.points.len()).map(|k| [f(p, k), 0.0, 0.0, 0.0]).collect(), ..p.clone() })
                .collect::<Vec<_>>()
        };
        let own = with(&|p, _| p.luma as f32 + 0.5);
        let varied = with(&|_, k| 20.0 + 60.0 * k as f32);
        for (depth, temporal) in [(false, 0), (true, 0), (false, 100), (true, 100), (false, 67)] {
            let q = Quality { depth, temporal, stipple: Stipple::Keep, ..Default::default() };
            let off = frame(q, &polys);
            assert_eq!(frame(vertex(q), &polys), off, "no lit polygon, depth {depth}, temporal {temporal}");
            assert_eq!(
                frame(q, &varied),
                off,
                "shading flat ignores corner values, depth {depth}, temporal {temporal}"
            );
            assert_eq!(frame(vertex(q), &own), off, "lit at their own luma, depth {depth}, temporal {temporal}");
            if temporal == 0 {
                assert_ne!(frame(vertex(q), &varied), off, "varying corners show, depth {depth}");
            }
        }
        assert_eq!(
            model2::raster::draw_view(&mut own.clone(), &src, &view),
            model2::raster::draw_view(&mut polys.clone(), &src, &view),
            "software: own lumas"
        );
        // Against the software renderer, on the flat-coloured polygons
        // (every luma level a different colour here): where they differ
        // drawing flat (edge pixels the two cover differently), and where a
        // pixel's luma truncates the other way.
        let plain: Vec<usize> = (0..polys.len()).filter(|&k| polys[k].texture.is_none() && !polys[k].checker).collect();
        let placed: Vec<Placed> = plain
            .iter()
            .enumerate()
            .map(|(k, &i)| Placed { poly: k, ..placed.iter().find(|pl| pl.poly == i).unwrap().clone() })
            .collect();
        let pick = |v: &[Poly]| plain.iter().map(|&k| v[k].clone()).collect::<Vec<_>>();
        let differ = |polys: &[Poly], q: Quality| {
            let mut g = Renderer::new().unwrap();
            g.set_quality(q);
            let (rgb, covered) = g.draw(polys, &placed, &src, &view);
            let (srgb, scovered) = model2::raster::draw_view_depth(&mut polys.to_vec(), &src, &view, q.depth);
            let n = (0..covered.len())
                .filter(|&i| {
                    covered[i] != scovered[i] || (covered[i] && rgb[i * 3..i * 3 + 3] != srgb[i * 3..i * 3 + 3])
                })
                .count();
            (n, covered.iter().filter(|&&c| c).count())
        };
        for depth in [false, true] {
            let q = Quality { depth, ..Default::default() };
            let (flat, n) = differ(&pick(&polys), q);
            let (lit, _) = differ(&pick(&varied), vertex(q));
            assert!(n > 10_000 && lit <= flat + n / 1000, "depth {depth}: {lit} lit against {flat} flat of {n}");
        }
        // Per pixel (`Poly::pixel`): a lamp whose value at the face's
        // normal is the polygon's own luma (+0.5), every corner with that
        // normal: the off frame byte for byte. Normals turning across the
        // polygons (a highlight) agree with the software renderer as well
        // as flat does; so does the clamp to `lit_range`.
        let pixel = |q: Quality| Quality { shading: crate::Shading::Pixel, crease: 40, ..q };
        let lamp = |p: &Poly| model2::light::PixelLight {
            lamp: model2::light::Lamp {
                l: [0.0, 0.0, 1.0],
                ambient: p.luma as f32 + 0.5 - 100.0,
                diffuse: 60.0,
                spec_scale: 40.0,
                squarings: 2,
                front: true,
                base: model2::light::Base::D,
            },
            offset: 0.0,
        };
        let with_normals = |f: &dyn Fn(usize) -> [f32; 3], range: [u8; 2]| {
            polys
                .iter()
                .map(|p| {
                    let pl = lamp(p);
                    let lit = (0..p.points.len())
                        .map(|k| {
                            let n = f(k);
                            [model2::light::clamp_corner(pl.luma(n)), n[0], n[1], n[2]]
                        })
                        .collect();
                    Poly { lit, pixel: Some(pl), lit_range: range, ..p.clone() }
                })
                .collect::<Vec<_>>()
        };
        let own_px = with_normals(&|_| [0.0, 0.0, 1.0], [0, 255]);
        let tilt = |k: usize| {
            let a = k as f32 * 1.7;
            let n = [0.45 * a.cos(), 0.45 * a.sin(), 0.8];
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            n.map(|c| c / l)
        };
        let turning = with_normals(&tilt, [0, 255]);
        for (depth, temporal) in [(false, 0), (true, 0), (false, 100), (true, 100), (false, 67)] {
            let q = Quality { depth, temporal, stipple: Stipple::Keep, ..Default::default() };
            let off = frame(q, &polys);
            assert_eq!(
                frame(pixel(q), &own_px),
                off,
                "per pixel at their own normal, depth {depth}, temporal {temporal}"
            );
            if temporal == 0 {
                assert_ne!(frame(pixel(q), &turning), off, "turning normals show, depth {depth}");
            }
        }
        assert_eq!(
            model2::raster::draw_view(&mut own_px.clone(), &src, &view),
            model2::raster::draw_view(&mut polys.clone(), &src, &view),
            "software: own normals"
        );
        for depth in [false, true] {
            let q = Quality { depth, ..Default::default() };
            let (flat, n) = differ(&pick(&polys), q);
            let (lit, _) = differ(&pick(&turning), pixel(q));
            assert!(lit <= flat + n / 1000, "per pixel, depth {depth}: {lit} lit against {flat} flat of {n}");
            let clamped = with_normals(&tilt, [40, 120]);
            let (lit, _) = differ(&pick(&clamped), pixel(q));
            assert!(lit <= flat + n / 1000, "clamped, depth {depth}: {lit} lit against {flat} flat of {n}");
            // With `Vertex` the GPU draws the corner lumas only.
            let corners_only: Vec<Poly> = turning.iter().map(|p| Poly { pixel: None, ..p.clone() }).collect();
            let mut g = Renderer::new().unwrap();
            g.set_quality(vertex(q));
            let as_vertex = g.draw(&pick(&turning), &placed, &src, &view);
            assert_eq!(as_vertex, g.draw(&pick(&corners_only), &placed, &src, &view), "`Vertex` ignores `pixel`");
        }
        // Finer levels (`Quality::shading_fine`, `Poly::fine`): no lit
        // polygon, the off frame; lit, the software's blend of two columns
        // (every level a random colour here, so the blend is far from
        // either column), within a step of rounding where the
        // interpolated luma differs in the last bits.
        let fine = |q: Quality| Quality { shading_fine: true, ..q };
        let finer = |v: &[Poly]| v.iter().map(|p| Poly { fine: true, ..p.clone() }).collect::<Vec<_>>();
        for (depth, temporal) in [(false, 0), (true, 0), (false, 100), (true, 67)] {
            let q = Quality { depth, temporal, stipple: Stipple::Keep, ..Default::default() };
            let off = frame(q, &polys);
            assert_eq!(frame(fine(vertex(q)), &polys), off, "fine, no lit polygon, depth {depth}, temporal {temporal}");
            if temporal == 0 {
                assert_ne!(frame(fine(vertex(q)), &finer(&varied)), frame(vertex(q), &varied), "fine shows");
            }
        }
        let near = |polys: &[Poly], q: Quality| {
            let mut g = Renderer::new().unwrap();
            g.set_quality(q);
            let (rgb, covered) = g.draw(polys, &placed, &src, &view);
            let (srgb, scovered) = model2::raster::draw_view_depth(&mut polys.to_vec(), &src, &view, q.depth);
            (0..covered.len())
                .filter(|&i| {
                    covered[i] != scovered[i]
                        || (covered[i] && (0..3).any(|c| rgb[i * 3 + c].abs_diff(srgb[i * 3 + c]) > 1))
                })
                .count()
        };
        for depth in [false, true] {
            let q = Quality { depth, ..Default::default() };
            let (flat, n) = differ(&pick(&polys), q);
            let lit = near(&pick(&finer(&varied)), fine(vertex(q)));
            assert!(lit <= flat + n / 1000, "fine vertex, depth {depth}: {lit} against {flat} flat of {n}");
            let lit = near(&pick(&finer(&turning)), fine(pixel(q)));
            assert!(lit <= flat + n / 1000, "fine pixel, depth {depth}: {lit} against {flat} flat of {n}");
            let clamped = finer(&with_normals(&tilt, [40, 120]));
            let lit = near(&pick(&clamped), fine(pixel(q)));
            assert!(lit <= flat + n / 1000, "fine clamped, depth {depth}: {lit} against {flat} flat of {n}");
        }
    }

    /// The resolve keeps the history finite whatever it reads: the last
    /// history and aux target filled with NaN and infinities, then frames
    /// with motion (also the huge vectors of the near-eye polygon), at
    /// 100% and when upscaling.
    #[test]
    fn history_stays_finite() {
        let Ok(mut gpu) = Renderer::new() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (render, palette) = (vec![0x5au8; 0x480000], vec![0x3cu8; 0x20000]);
        let src = Sources {
            texture_rom: &[],
            texture_point_ram: &[],
            render_ram: &render,
            palette: &palette,
            xoff: 0.0,
            yoff: 0.0,
        };
        let view = View::new(480, 4.0 / 3.0, Hud::Center);
        let layers = Layers2d {
            base: vec![40; WIDTH * HEIGHT * 4],
            side: 0,
            hud: vec![0; WIDTH * HEIGHT * 4],
            ext_w: WIDTH,
            backdrop: [0, 0, 0],
            tiles: None,
        };
        let mut polys = scene();
        let placed = place(&mut polys, &view);
        // Half floats: NaN, +inf, -inf, 65504.
        let bad: Vec<u8> =
            [0x7e00u16, 0x7c00, 0xfc00, 0x7bff].iter().cycle().take(4096).flat_map(|h| h.to_le_bytes()).collect();
        for scale in [100, 67, 50] {
            gpu.set_quality(Quality { temporal: scale, ..Default::default() });
            for k in 0..6u64 {
                // Poison everything the next frame reads from the last.
                for (label, t) in
                    gpu.textures.iter().filter(|(l, _)| l.starts_with("history") || l.starts_with("3d aux"))
                {
                    let (w, h) = (t.width(), t.height());
                    let row = w as usize * 8;
                    let data: Vec<u8> = (0..h as usize).flat_map(|_| bad.iter().cycle().take(row).copied()).collect();
                    gpu.queue.write_texture(
                        t.as_image_copy(),
                        &data,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(row as u32),
                            rows_per_image: None,
                        },
                        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    );
                    let _ = label;
                }
                let step = if k == 0 { Step::NoMotion } else { Step::Blend { tp: 0.0 } };
                gpu.set_temporal_frame(FrameTag::game(k), step, Vec::new());
                gpu.frame(&polys, &placed, &src, &view, &layers);
                let (_, _, hist) = gpu.temporal_history().unwrap();
                let bad_px =
                    hist.iter().filter(|p| !p.iter().all(|c| c.is_finite() && (0.0..=1.0).contains(c))).count();
                assert_eq!(bad_px, 0, "scale {scale} frame {k}");
            }
        }
    }

    /// A still scene converges: frames repeated with jitter change less
    /// and less, and stay near the plain frame.
    #[test]
    fn still_scene_converges() {
        let Ok(mut gpu) = Renderer::new() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (render, palette) = (vec![0x5au8; 0x480000], vec![0x3cu8; 0x20000]);
        let src = Sources {
            texture_rom: &[],
            texture_point_ram: &[],
            render_ram: &render,
            palette: &palette,
            xoff: 0.0,
            yoff: 0.0,
        };
        let view = View::new(480, 4.0 / 3.0, Hud::Center);
        let layers = Layers2d {
            base: vec![40; WIDTH * HEIGHT * 4],
            side: 0,
            hud: vec![0; WIDTH * HEIGHT * 4],
            ext_w: WIDTH,
            backdrop: [0, 0, 0],
            tiles: None,
        };
        let mut polys = scene();
        let placed = place(&mut polys, &view);
        let off = gpu.frame(&polys, &placed, &src, &view, &layers);
        for scale in [100, 50] {
            gpu.set_quality(Quality { temporal: scale, ..Default::default() });
            let mut frames = Vec::new();
            for k in 0..40u64 {
                let step = if k == 0 { Step::NoMotion } else { Step::Repeat };
                gpu.set_temporal_frame(FrameTag::game(1), step, Vec::new());
                frames.push(gpu.frame(&polys, &placed, &src, &view, &layers));
            }
            let mean =
                |a: &[u8], b: &[u8]| a.iter().zip(b).map(|(x, y)| x.abs_diff(*y) as f64).sum::<f64>() / a.len() as f64;
            let late = mean(&frames[38], &frames[39]);
            let to_off = mean(&frames[39], &off);
            assert!(late < 0.25, "scale {scale}: frame-to-frame change {late} after 38 frames");
            assert!(to_off < 3.0, "scale {scale}: {to_off} from the plain frame");
        }
    }

    /// A polygon whose colour alternates every frame (topskatr's FINISH
    /// sign) shows this frame's colour inside, not a running average: the
    /// neighbourhood there holds one colour, which the history is clipped
    /// to. At 100% and upscaled, with motion data and without.
    #[test]
    fn alternating_colour_is_not_averaged() {
        let Ok(mut gpu) = Renderer::new() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (render, palette) = (vec![0x5au8; 0x480000], vec![0x3cu8; 0x20000]);
        let src = Sources {
            texture_rom: &[],
            texture_point_ram: &[],
            render_ram: &render,
            palette: &palette,
            xoff: 0.0,
            yoff: 0.0,
        };
        let view = View::new(480, 4.0 / 3.0, Hud::Center);
        let layers = Layers2d {
            base: vec![40; WIDTH * HEIGHT * 4],
            side: 0,
            hud: vec![0; WIDTH * HEIGHT * 4],
            ext_w: WIDTH,
            backdrop: [0, 0, 0],
            tiles: None,
        };
        let mut plain_gpu = Renderer::new().unwrap();
        for (scale, step) in
            [(100, Step::Repeat), (50, Step::Repeat), (100, Step::Blend { tp: 0.0 }), (67, Step::NoMotion)]
        {
            gpu.set_quality(Quality { temporal: scale, ..Default::default() });
            for k in 0..24u64 {
                let mut q = quad(100.0, 100.0, 300.0, 250.0, 0.01, 100.0, 0);
                q.colour = if k % 2 == 0 { [31, 31, 20] } else { [0, 0, 0] };
                if let Step::Blend { .. } = step {
                    q.prev = q.points.iter().map(|v| [v.x, v.y, 1.0]).collect();
                }
                let mut polys = vec![q];
                let placed = place(&mut polys, &view);
                gpu.set_temporal_frame(FrameTag::game(k), if k == 0 { Step::NoMotion } else { step }, Vec::new());
                let out = gpu.frame(&polys, &placed, &src, &view, &layers);
                if k < 8 {
                    continue;
                }
                let plain = plain_gpu.frame(&polys, &placed, &src, &view, &layers);
                let (x, y) = view.to_output(200.0, 175.0);
                let i = (y as usize * view.width + x as usize) * 3;
                let d = (0..3).map(|c| out[i + c].abs_diff(plain[i + c])).max().unwrap();
                assert!(
                    d <= 2,
                    "scale {scale} {step:?} frame {k}: {:?} against {:?}",
                    &out[i..i + 3],
                    &plain[i..i + 3]
                );
            }
        }
    }

    /// A history a rounding step off in a channel whose neighbourhood has
    /// no spread (alpha 0.99951 on opaque samples: the half float below 1)
    /// is kept, not replaced by the neighbourhood's mean (sgt24h's decal
    /// edge dots): two renderers drawing the same frames, one with every
    /// opaque history texel's alpha set one step below 1 before the last
    /// frame, give the same picture.
    #[test]
    fn rounding_in_the_history_is_not_clipped() {
        let Ok(mut gpu) = Renderer::new() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let mut other = Renderer::new().unwrap();
        // Colours from the palette: varied, so the two quads differ.
        let render = vec![0x5au8; 0x480000];
        let palette: Vec<u8> = (0..0x20000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
        let src = Sources {
            texture_rom: &[],
            texture_point_ram: &[],
            render_ram: &render,
            palette: &palette,
            xoff: 0.0,
            yoff: 0.0,
        };
        let view = View::new(480, 4.0 / 3.0, Hud::Center);
        let layers = Layers2d {
            base: vec![40; WIDTH * HEIGHT * 4],
            side: 0,
            hud: vec![0; WIDTH * HEIGHT * 4],
            ext_w: WIDTH,
            backdrop: [0, 0, 0],
            tiles: None,
        };
        // A light quad on a dark one, its edges between pixel centres.
        let mut back = quad(-20.0, -10.0, 520.0, 400.0, 0.001, 900.0, 0);
        back.colour = [20, 2, 2];
        let mut front = quad(100.3, 100.4, 300.6, 250.7, 0.01, 100.0, 0);
        front.colour = [31, 31, 0];
        let mut polys = vec![back, front];
        let placed = place(&mut polys, &view);
        let mut outs = Vec::new();
        for g in [&mut gpu, &mut other] {
            g.set_quality(Quality { temporal: 100, ..Default::default() });
            for k in 0..12u64 {
                if k == 11 && outs.is_empty() {
                    let l = g.temporal.last_frame.as_ref().unwrap();
                    let t = l.history.clone();
                    let (w, h) = (t.width(), t.height());
                    let mut raw = g.read_back_bpp(g.device.create_command_encoder(&Default::default()), &t, w, h, 8);
                    for p in raw.chunks_mut(8) {
                        if p[6..8] == 0x3c00u16.to_le_bytes() {
                            p[6..8].copy_from_slice(&0x3bffu16.to_le_bytes());
                        }
                    }
                    g.queue.write_texture(
                        t.as_image_copy(),
                        &raw,
                        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 8), rows_per_image: None },
                        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    );
                }
                g.set_temporal_frame(FrameTag::game(1), if k == 0 { Step::NoMotion } else { Step::Repeat }, Vec::new());
                let out = g.frame(&polys, &placed, &src, &view, &layers);
                if k == 11 {
                    outs.push(out);
                }
            }
        }
        let worst = outs[0].iter().zip(&outs[1]).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        assert!(worst <= 1, "a history alpha one step below 1 changed a pixel by {worst} levels");
    }

    /// Sharpening (`fs_sharpen`) changes the picture but never draws a
    /// colour outside its cross neighbourhood in the unsharpened one (no
    /// halos, no ringing), and leaves the history alone.
    #[test]
    fn sharpening_stays_within_the_neighbourhood() {
        let Ok(mut plain) = Renderer::new() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let mut sharp = Renderer::new().unwrap();
        let render = vec![0x5au8; 0x480000];
        let palette: Vec<u8> = (0..0x20000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
        let src = Sources {
            texture_rom: &[],
            texture_point_ram: &[],
            render_ram: &render,
            palette: &palette,
            xoff: 0.0,
            yoff: 0.0,
        };
        let view = View::new(480, 4.0 / 3.0, Hud::Center);
        let layers = Layers2d {
            base: vec![40; WIDTH * HEIGHT * 4],
            side: 0,
            hud: vec![0; WIDTH * HEIGHT * 4],
            ext_w: WIDTH,
            backdrop: [0, 0, 0],
            tiles: None,
        };
        let mut polys = scene();
        let placed = place(&mut polys, &view);
        let q = Quality { temporal: 100, ..Default::default() };
        plain.set_quality(q);
        sharp.set_quality(q);
        sharp.set_temporal_sharpen(1.0);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for k in 0..6u64 {
            let step = if k == 0 { Step::NoMotion } else { Step::Repeat };
            for (g, out) in [(&mut plain, &mut a), (&mut sharp, &mut b)] {
                g.set_temporal_frame(FrameTag::game(1), step, Vec::new());
                *out = g.frame(&polys, &placed, &src, &view, &layers);
            }
        }
        assert_eq!(plain.temporal_history().unwrap().2, sharp.temporal_history().unwrap().2, "history sharpened");
        assert_ne!(a, b, "sharpening did nothing");
        let (w, h) = (view.width, view.height);
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                for c in 0..3 {
                    let at =
                        |dx: isize, dy: isize| a[((y as isize + dy) as usize * w + (x as isize + dx) as usize) * 3 + c];
                    let n = [at(0, 0), at(-1, 0), at(1, 0), at(0, -1), at(0, 1)];
                    let (lo, hi) = (*n.iter().min().unwrap(), *n.iter().max().unwrap());
                    let v = b[(y * w + x) * 3 + c];
                    assert!(v + 1 >= lo && v <= hi.saturating_add(1), "({x}, {y}) {c}: {v} outside {lo}..{hi}");
                }
            }
        }
    }

    /// The thin-line scenes: a dark background, a light line `thick` game
    /// px thick from (60, y) to (440, y + 25) that moved `dy` down since
    /// the last frame (none: `y` None), and an occluder in front of it,
    /// all with motion data.
    fn line_scene(y: Option<f32>, thick: f32, dy: f32, occluder: bool) -> Vec<Poly> {
        let still = |p: &mut Poly| p.prev = p.points.iter().map(|v| [v.x, v.y, 1.0]).collect();
        let mut back = quad(-20.0, -10.0, 520.0, 400.0, 0.001, 900.0, 0);
        back.colour = [20, 2, 2];
        still(&mut back);
        let mut p = vec![back];
        if let Some(y) = y {
            let mut l = quad(0.0, 0.0, 1.0, 1.0, 0.01, 100.0, 0);
            let v = |x, y| SVtx { x, y, iz: 0.01, uz: 0.0, vz: 0.0 };
            l.points = vec![v(60.0, y), v(440.0, y + 25.0), v(440.0, y + 25.0 + thick), v(60.0, y + thick)];
            l.colour = [31, 31, 0];
            l.prev = l.points.iter().map(|v| [v.x, v.y - dy, 1.0]).collect();
            p.push(l);
        }
        if occluder {
            let mut o = quad(200.0, 100.0, 300.0, 250.0, 0.05, 50.0, 0);
            o.colour = [2, 20, 31];
            still(&mut o);
            p.push(o);
        }
        p
    }

    /// Renderers, sources and layers for the thin-line tests.
    struct LineRig {
        render: Vec<u8>,
        palette: Vec<u8>,
        view: View,
        layers: Layers2d,
    }

    impl LineRig {
        fn new() -> Self {
            LineRig {
                render: vec![0x5au8; 0x480000],
                palette: (0..0x20000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect(),
                view: View::new(480, 4.0 / 3.0, Hud::Center),
                layers: Layers2d {
                    base: vec![40; WIDTH * HEIGHT * 4],
                    side: 0,
                    hud: vec![0; WIDTH * HEIGHT * 4],
                    ext_w: WIDTH,
                    backdrop: [0, 0, 0],
                    tiles: None,
                },
            }
        }

        /// Frame `k` of `polys` (the first after a reset).
        fn frame(&self, g: &mut Renderer, k: u64, mut polys: Vec<Poly>) -> Vec<u8> {
            let src = Sources {
                texture_rom: &[],
                texture_point_ram: &[],
                render_ram: &self.render,
                palette: &self.palette,
                xoff: 0.0,
                yoff: 0.0,
            };
            let placed = place(&mut polys, &self.view);
            let step = if k == 0 { Step::NoMotion } else { Step::Blend { tp: 0.0 } };
            g.set_temporal_frame(FrameTag::game(k), step, Vec::new());
            g.frame(&polys, &placed, &src, &self.view, &self.layers)
        }

        /// RGB at game point (x, y).
        fn at(&self, img: &[u8], x: f32, y: f32) -> [u8; 3] {
            let (x, y) = self.view.to_output(x, y);
            let i = (y as usize * self.view.width + x as usize) * 3;
            [img[i], img[i + 1], img[i + 2]]
        }

        /// Along the line at `y` (`line_scene`), per output column, the
        /// most any pixel within 4 rows of it differs from the background
        /// `bg` (summed |RGB|).
        fn profile(&self, img: &[u8], y: f32, thick: f32, bg: [u8; 3]) -> Vec<u32> {
            let mut out = Vec::new();
            let (x0, _) = self.view.to_output(80.0, 0.0);
            let (x1, _) = self.view.to_output(420.0, 0.0);
            for ox in x0 as usize..x1 as usize {
                let gx = (ox as f32 + 0.5) / self.view.sx;
                let gy = y + 25.0 * (gx - 60.0) / 380.0 + thick / 2.0;
                let (_, oy) = self.view.to_output(gx, gy);
                let peak = (oy as usize - 4..=oy as usize + 4)
                    .map(|r| {
                        let i = (r * self.view.width + ox) * 3;
                        (0..3).map(|c| img[i + c].abs_diff(bg[c]) as u32).sum::<u32>()
                    })
                    .max()
                    .unwrap();
                out.push(peak);
            }
            out
        }
    }

    /// A line half an output pixel thick (a quarter of an internal pixel
    /// at 50%) under jitter stays whole when upscaling: every column along
    /// it shows it, every frame, where without the thin-feature handling it
    /// breaks into dashes (some jitter phases miss it and the clip throws
    /// its history away).
    #[test]
    fn thin_line_stays_whole_when_upscaled() {
        let Ok(mut on) = Renderer::new() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let rig = LineRig::new();
        let mut off = Renderer::new().unwrap();
        off.set_temporal_tuning(Tuning { thin: 0.0, ..Tuning::default() });
        // The line's and the background's colours, from a thick line.
        let mut plain = Renderer::new().unwrap();
        let thick = rig.frame(&mut plain, 0, line_scene(Some(150.0), 12.0, 0.0, false));
        let (bg, fg) = (rig.at(&thick, 30.0, 30.0), rig.at(&thick, 250.0, 168.0));
        let contrast: u32 = (0..3).map(|c| fg[c].abs_diff(bg[c]) as u32).sum();
        assert!(contrast > 200, "{fg:?} {bg:?}");
        let thin = 0.4;
        for scale in [50, 67] {
            let mut least = [u32::MAX; 2];
            for (n, g) in [&mut on, &mut off].into_iter().enumerate() {
                g.set_quality(Quality { temporal: scale, ..Default::default() });
                for k in 0..40u64 {
                    let out = rig.frame(g, k, line_scene(Some(150.0), thin, 0.0, false));
                    if k >= 24 {
                        let p = rig.profile(&out, 150.0, thin, bg);
                        least[n] = least[n].min(*p.iter().min().unwrap());
                    }
                }
            }
            // The line covers about half of its pixels: at least a sixth
            // of the contrast everywhere along it.
            assert!(
                least[0] * 6 >= contrast,
                "scale {scale}: the line fades to {} of {contrast} somewhere (without: {})",
                least[0],
                least[1]
            );
            assert!(least[0] > least[1], "scale {scale}: {} not above {} without", least[0], least[1]);
        }
    }

    /// A smooth-shaded thin line (the `_l_t` entry points) is widened and
    /// drawn at its coverage as a flat one is: lit at its own luma, it
    /// gives the flat line's pictures, frame by frame, at 67% and 50%.
    #[test]
    fn thin_lit_line_as_flat() {
        let Ok(mut flat) = Renderer::new() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let rig = LineRig::new();
        let mut lit = Renderer::new().unwrap();
        let own = |polys: Vec<Poly>| {
            polys
                .into_iter()
                .map(|p| Poly { lit: (0..p.points.len()).map(|_| [p.luma as f32 + 0.5, 0.0, 0.0, 0.0]).collect(), ..p })
                .collect::<Vec<_>>()
        };
        for scale in [50, 67] {
            for shading in [crate::Shading::Vertex, crate::Shading::Pixel] {
                flat.set_quality(Quality { temporal: 0, ..Default::default() });
                flat.set_quality(Quality { temporal: scale, ..Default::default() });
                lit.set_quality(Quality { temporal: 0, ..Default::default() });
                lit.set_quality(Quality { temporal: scale, shading, crease: 40, ..Default::default() });
                for k in 0..12u64 {
                    let a = rig.frame(&mut flat, k, line_scene(Some(150.0), 0.4, 0.0, false));
                    let b = rig.frame(&mut lit, k, own(line_scene(Some(150.0), 0.4, 0.0, false)));
                    assert!(a == b, "scale {scale}, {shading:?}, frame {k}: lit thin line differs from flat");
                }
            }
        }
    }

    /// The thin-line handling leaves nothing behind: once a thin line is
    /// gone, has jumped away, or an opaque polygon passes in front of it,
    /// the picture there is the background (or the occluder) within a few
    /// frames, at 50% and 100%.
    #[test]
    fn thin_line_leaves_no_trail() {
        let Ok(mut g) = Renderer::new() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let rig = LineRig::new();
        let thin = 0.4;
        for scale in [50, 100] {
            for case in 0..3 {
                g.set_quality(Quality { temporal: 0, ..Default::default() });
                g.set_quality(Quality { temporal: scale, ..Default::default() });
                for k in 0..30u64 {
                    rig.frame(&mut g, k, line_scene(Some(150.0), thin, 0.0, false));
                }
                let mut last = Vec::new();
                for k in 30..36u64 {
                    let polys = match case {
                        0 => line_scene(None, thin, 0.0, false),
                        1 => line_scene(Some(220.0), thin, if k == 30 { 70.0 } else { 0.0 }, false),
                        _ => line_scene(Some(150.0), thin, 0.0, true),
                    };
                    last = rig.frame(&mut g, k, polys);
                }
                let bg = rig.at(&last, 30.0, 30.0);
                let occ = rig.at(&last, 250.0, 120.0);
                // Where the line was, away from its new place and the
                // occluder's edges.
                for gx in (90..410).step_by(3) {
                    let gx = gx as f32;
                    let gy = 150.0 + 25.0 * (gx - 60.0) / 380.0 + thin / 2.0;
                    let inside = case == 2 && (204.0..296.0).contains(&gx);
                    if case == 2 && !inside {
                        continue;
                    }
                    let want = if inside { occ } else { bg };
                    for dy in [-1.5f32, -0.75, 0.0, 0.75, 1.5] {
                        let px = rig.at(&last, gx, gy + dy);
                        let d = (0..3).map(|c| px[c].abs_diff(want[c])).max().unwrap();
                        assert!(d <= 3, "scale {scale} case {case} at ({gx}, {}): {px:?} not {want:?}", gy + dy);
                    }
                }
            }
        }
    }

    #[test]
    fn thin_polygons() {
        // A sliver half an internal pixel across at 50%, long: thin; its
        // widening makes it 1 internal px across.
        let pts = [(10.0, 10.0), (200.0, 30.0), (200.0, 31.0), (10.0, 11.0)];
        let t = thin(&pts, (0.5, 0.5), (1.0, 2.0)).expect("thin");
        assert!((t.width - 0.497).abs() < 0.01, "{t:?}");
        let w: Vec<(f32, f32)> = pts
            .iter()
            .map(|&p| {
                let d = t.widen(p, (0.5, 0.5), 1.0);
                (p.0 + d.0, p.1 + d.1)
            })
            .collect();
        let t2 = thin(&w, (0.5, 0.5), (2.0, 2.0)).expect("still thin at 2");
        assert!((t2.width - 1.0).abs() < 1e-3, "{t2:?}");
        assert!((t.coverage(1.0) - t.width).abs() < 1e-6);
        // Wide, or thin but short (a speck): not thin.
        assert_eq!(thin(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)], (1.0, 1.0), (1.0, 2.0)), None);
        assert_eq!(thin(&[(0.0, 0.0), (1.0, 0.0), (1.0, 0.5)], (1.0, 1.0), (1.0, 2.0)), None);
        // Two slivers of a strip share their long edge; a wire has its own.
        let v = |x, y| SVtx { x, y, iz: 1.0, uz: 0.0, vz: 0.0 };
        let a = [v(0.0, 0.0), v(100.0, 0.0), v(100.0, 0.5), v(0.0, 0.5)];
        let b = [v(0.0, 0.5), v(100.0, 0.5), v(100.0, 1.0), v(0.0, 1.0)];
        let wire = [v(0.0, 9.0), v(100.0, 9.0), v(100.0, 9.5), v(0.0, 9.5)];
        let e = Edges::new([&a[..], &b[..], &wire[..]].into_iter());
        let pts = |p: &[SVtx]| p.iter().map(|v| (v.x, v.y)).collect::<Vec<_>>();
        assert!(e.long_edge_shared(&pts(&a)) && e.long_edge_shared(&pts(&b)));
        assert!(!e.long_edge_shared(&pts(&wire)));
    }

    #[test]
    fn halves() {
        for (h, f) in
            [(0x3c00, 1.0), (0xc000, -2.0), (0x7bff, 65504.0), (0x0400, 6.1035156e-5), (0, 0.0), (0x3555, 0.33325195)]
        {
            assert_eq!(f16_to_f32(h), f, "{h:#x}");
        }
    }
}
