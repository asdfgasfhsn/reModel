//! Model 2 rasteriser (software), consuming the geometrizer's output
//! stream. Follows docs/hw/geometrizer.md sections 3.4 and 4-6; most of it
//! is MAME-only description, so treat it as a hypothesis to check against
//! the game's frames.
//!
//! Polygons are drawn back to front (painter's algorithm on the z-sort key)
//! with perspective-correct, point-sampled textures (level 0, no mipmaps).

use crate::enhance::{sample_trilinear, Address, Enhance, Fnv, Material, Rgba, TexFilter, TexInfo};
use crate::video::{View, HEIGHT, WIDTH};
use std::collections::HashMap;
use std::sync::Arc;

/// One 24-bit float from the stream (IEEE single with the low 8 bits cut).
fn f24(w: u32) -> f32 {
    f32::from_bits(w << 8)
}

/// View-space vertex with texture coordinates (texels), with motion data
/// ([`build_motion`]) its previous view-space position (zero without), and
/// with smooth shading ([`build_with`]) its corner values (`[luma, nx, ny,
/// nz]`, `crate::light::Shaded::corners`; zero without).
#[derive(Clone, Copy, Debug, Default)]
struct Vtx {
    x: f32,
    y: f32,
    z: f32,
    u: f32,
    v: f32,
    px: f32,
    py: f32,
    pz: f32,
    lit: [f32; 4],
}

/// Screen-space vertex with perspective-divided attributes.
#[derive(Clone, Copy, Debug)]
pub struct SVtx {
    pub x: f32,
    pub y: f32,
    /// 1/z, u/z and v/z: linear in screen space.
    pub iz: f32,
    pub uz: f32,
    pub vz: f32,
}

#[derive(Clone, Copy, Debug)]
struct Window {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    centres: Centres,
}

/// A window's four centres in screen pixels (sync offsets applied).
pub type Centres = [(f32, f32); 4];

/// What [`build_motion`] needs to give each polygon its previous screen
/// position: the frame's stream posed two ways, word for word in the
/// layout of the stream being drawn (`crate::smooth::Motion`).
pub struct MotionIn<'a> {
    /// Frame N's list posed with frame N-1's transforms (`Motion::s0`).
    pub s0: &'a [u32],
    /// Frame N's list posed with its own (the engine's stream).
    pub s1: &'a [u32],
    /// When the previous image was, on the axis from `s0` (0) to `s1` (1):
    /// a vertex was at `s0 + tp * (s1 - s0)` in view space. May lie
    /// outside 0..1 (extrapolated).
    pub tp: f32,
    /// Per object block (`Motion::valid`); ignored (all valid) when its
    /// length isn't the stream's number of blocks.
    pub valid: &'a [bool],
    /// The previous displayed frame's centres by window index (a window
    /// missing here: this frame's).
    pub prev_centres: &'a [Centres],
}

/// Texture parameters from a polygon's 4-word header (section 5.3).
#[derive(Clone, Copy, Debug)]
pub struct Texture {
    /// Position on the texture sheet and size, in texels.
    pub x0: u32,
    pub y0: u32,
    pub w: u32,
    pub h: u32,
    pub mirror_x: bool,
    pub mirror_y: bool,
    pub bank: u32,
    /// Texel 15 is transparent.
    pub translucent: bool,
    pub luma_table: u32,
}

/// A polygon ready to draw.
#[derive(Clone, Debug)]
pub struct Poly {
    pub points: Vec<SVtx>,
    pub key: f32,
    pub window: usize,
    /// Viewport clip rectangle (x0, y0, x1, y1) in screen pixels.
    pub clip: (f32, f32, f32, f32),
    pub texture: Option<Texture>,
    /// Base colour components (5 bits each) from palette entry 0x1000 + n.
    pub colour: [u32; 3],
    /// Polygon luma, 0-255.
    pub luma: u32,
    /// Header bit 15: every other pixel is dropped (a checkerboard, by
    /// screen pixel), textured or flat. waverunr's arch shadow is a flat
    /// one (docs/hw/geometrizer.md 5.3).
    pub checker: bool,
    /// The 4-word texture header, kept for probing.
    pub header: [u16; 4],
    /// The object block (the stream's `0x1` command, counted from 0 in the
    /// frame) the polygon came in: one model's polygon list. For probing.
    pub block: u32,
    /// The stream index of its attribute word: what smooth shading's
    /// results are keyed by (`crate::light::Shaded::at`). For probing.
    pub at: u32,
    /// Motion data ([`build_motion`]): per point, where it was in the
    /// previous displayed image, in homogeneous screen pixels `[X, Y, W]`
    /// (the position is `(X / W, Y / W)`; its 1/z was `iz / W`), linear in
    /// screen space like [`SVtx::uz`]. Empty when there is none: no
    /// motion data, the object's motion unknown, or a previous position
    /// behind the eye.
    pub prev: Vec<[f32; 3]>,
    /// Flat and black: a shadow. The games' shadows are flat black (or
    /// dithered black) polygons laid on the ground, at depths that don't
    /// match it (hotd's are screen-parallel slices), so with the depth
    /// buffer they keep the game's order instead.
    pub shadow: bool,
    /// Smooth shading ([`build_with`], `crate::light`): per point, its
    /// corner values `[luma, nx, ny, nz]` (luma a float 0-255 before
    /// truncation; the normal zero where there is none), linear in view
    /// space, so `luma * iz` is linear in screen space like
    /// [`SVtx::uz`]. Empty: the polygon is drawn with [`Poly::luma`] (no
    /// smooth shading, not lit, or a shadow).
    pub lit: Vec<[f32; 4]>,
    /// Smooth shading per pixel (`crate::light::Shading::Pixel` at level
    /// L2): the polygon's lighting, evaluated for the normal (`lit`'s
    /// `nx, ny, nz`) interpolated to each pixel. None: the interpolated
    /// luma.
    pub pixel: Option<crate::light::PixelLight>,
    /// Smooth shading: the lumas (inclusive) a lit pixel is clamped to,
    /// those over which the game's colour tables keep the polygon's hue
    /// ([`hue_range`]); `[0, 255]` when not lit.
    pub lit_range: [u8; 2],
    /// Smooth shading's finer levels (`light::Options::fine`): a lit
    /// pixel blends the two colour columns around its brightness
    /// ([`fine_level`]) instead of taking the game's level. False when not
    /// lit.
    pub fine: bool,
    /// Material maps (`light::Options::lamps`): the lighting a material's
    /// normal map perturbs, for a textured polygon of the exact layer
    /// (level L2) whose lamp depends on its normal. None otherwise, and
    /// always without `lamps`.
    pub lamp: Option<Box<PolyLamp>>,
}

/// What a polygon's material maps need of its lighting ([`Poly::lamp`]),
/// worked out per polygon when it is built (docs/texture-packs.md, "How
/// material maps light").
#[derive(Clone, Debug, PartialEq)]
pub struct PolyLamp {
    /// The lamp the game lit it with (`light::Lamped::light`).
    pub light: crate::light::PixelLight,
    /// The normal the game lit it with, unit, in the face's convention.
    pub n: [f32; 3],
    /// +1 or -1: `out * n` points out of the side the eye sees (toward
    /// the eye), the side a material's normals stand out of.
    pub out: f32,
    /// Its tangent frame; None where its texture coordinates are
    /// degenerate (then a normal map does nothing).
    pub tangent: Option<Tangent>,
    /// The lumas (inclusive) a pixel changed by a material is clamped to,
    /// those over which the polygon keeps its hue (as [`Poly::lit_range`]).
    pub range: [u8; 2],
    /// How the material's normal changes the luma.
    pub relief: Relief,
}

/// The largest factor the relief rule ([`Relief::Diffuse`]) brightens by.
const RELIEF_MAX: f32 = 4.0;

/// The luma a pixel of the relief rule ([`Relief::Diffuse`]) is drawn at for
/// a texel whose luma table value is `table` (None: untextured), its luma
/// clamped to at most `hi`: its flat luma `l` lowered to where that
/// texel's brightness stops rising (the top column [`fine_level`] reaches,
/// the game's level at `hi`, capped at 63; scenery lit to luma 255 is
/// usually there or past it), then times `f`. So `f` scales the brightness
/// the flat surface is drawn at, whatever saturated it: the lamp's sum,
/// the texel or the cap. Pure black (a luma under 1, a zero table value)
/// stays black.
pub fn relieved(l: f32, table: Option<u32>, hi: u8, f: f32) -> f32 {
    let top = match table {
        Some(0) => return l,
        Some(v) => 256.0 * ((v * hi as u32 / 256).min(63) as f32 + 0.5) / v as f32,
        None => 4.0 * ((hi as u32 >> 2).min(63) as f32 + 0.5),
    };
    if l.is_nan() || l < 1.0 {
        return l;
    }
    (l.min(top).min(255.0) * f).clamp(0.0, 255.0)
}

/// How a material's normal map changes a pixel's luma ([`material_luma`]):
/// the `materials_relief` setting (`gpu::Quality::materials_relief`, the
/// CLI's `--materials-relief off|S`). With `d` the game's facing-signed
/// `N . L` at the drawn normal and `d'` at the mapped one, and `B` the
/// brightness the flat surface is drawn at. Pure black stays black under
/// both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Relief {
    /// Faithful: `B + clamp(eval(N')) - clamp(eval(N))`, the map's normal
    /// substituted in the game's own sum (specular included), `B` the
    /// pixel's luma. A surface lit to 255 at every normal (much of the
    /// scenery) shows no relief.
    Faithful,
    /// Relief: `B (1 + s (max(d', 0) - max(d, 0)))`, the diffuse change
    /// relative to the brightness drawn (per texel, after the game's caps:
    /// [`relieved`]), ambient ignored, so it shows on fully lit scenery
    /// too. `tenths`: the strength `s` in tenths (20: 2). The factor is at
    /// most [`RELIEF_MAX`]; no relief where no light reaches (Lambert's
    /// clamp) or the lamp has no diffuse term.
    Diffuse { tenths: u8 },
}

impl Default for Relief {
    /// Relief at strength 2 ([`Relief::DEFAULT_TENTHS`]).
    fn default() -> Self {
        Relief::Diffuse { tenths: Relief::DEFAULT_TENTHS }
    }
}

impl Relief {
    /// The relief rule's strength by default, in tenths: 2.
    pub const DEFAULT_TENTHS: u8 = 20;
    /// The strongest relief, in tenths: 8.
    pub const MAX_TENTHS: u8 = 80;

    /// `off` (the faithful rule) or a relief strength from 0.1 to 8 (to a
    /// tenth).
    pub fn parse(s: &str) -> Option<Relief> {
        if s == "off" {
            return Some(Relief::Faithful);
        }
        let tenths = (s.parse::<f32>().ok()? * 10.0).round();
        (1.0..=Relief::MAX_TENTHS as f32).contains(&tenths).then_some(Relief::Diffuse { tenths: tenths as u8 })
    }

    /// The name [`Relief::parse`] reads: `off`, or the strength (`2`,
    /// `1.5`).
    pub fn name(self) -> String {
        match self {
            Relief::Faithful => "off".into(),
            Relief::Diffuse { tenths } if tenths % 10 == 0 => format!("{}", tenths / 10),
            Relief::Diffuse { tenths } => format!("{}.{}", tenths / 10, tenths % 10),
        }
    }

    /// The relief rule's strength; None for the faithful rule.
    pub fn strength(self) -> Option<f32> {
        match self {
            Relief::Faithful => None,
            Relief::Diffuse { tenths } => Some(tenths as f32 / 10.0),
        }
    }
}

/// A polygon's tangent frame for normal maps: the unit direction of
/// increasing u on the surface in unscaled view space, and the handedness
/// `h` (+1 or -1) that puts `h * (o x T)` on the side of increasing v, for
/// `o` the polygon's normal toward the eye. Constant over the polygon (its
/// texture mapping is affine over its plane).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tangent {
    pub t: [f32; 3],
    pub h: f32,
}

/// The tangent frame of a polygon from its corners (unscaled view space)
/// and texture coordinates (texels): the least-squares solution of `p_i -
/// p_0 = T (u_i - u_0) + B (v_i - v_0)` (exact for a triangle), `T`
/// normalised and the handedness from `o`, the normal toward the eye. None
/// where the coordinates are degenerate (all on one line in (u, v)) or the
/// result isn't finite.
pub fn tangent_frame(ring: &[([f32; 3], [f32; 2])], o: [f32; 3]) -> Option<Tangent> {
    let (p0, uv0) = *ring.first()?;
    let (mut a, mut b, mut d) = (0.0f64, 0.0f64, 0.0f64);
    let (mut su, mut sv) = ([0.0f64; 3], [0.0f64; 3]);
    for &(p, uv) in &ring[1..] {
        let (du, dv) = ((uv[0] - uv0[0]) as f64, (uv[1] - uv0[1]) as f64);
        a += du * du;
        b += du * dv;
        d += dv * dv;
        for c in 0..3 {
            let dp = (p[c] - p0[c]) as f64;
            su[c] += du * dp;
            sv[c] += dv * dp;
        }
    }
    let det = a * d - b * b;
    if det.is_nan() || det <= 1e-6 * a * d {
        return None;
    }
    let t = [0, 1, 2].map(|c| (su[c] * d - sv[c] * b) / det);
    let bv = [0, 1, 2].map(|c| (sv[c] * a - su[c] * b) / det);
    let len = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt();
    let blen = (bv[0] * bv[0] + bv[1] * bv[1] + bv[2] * bv[2]).sqrt();
    if !(len.is_finite() && blen.is_finite() && len > 1e-12 && blen > 1e-12) {
        return None;
    }
    let t = t.map(|x| x / len);
    let o = o.map(|x| x as f64);
    let c = [o[1] * t[2] - o[2] * t[1], o[2] * t[0] - o[0] * t[2], o[0] * t[1] - o[1] * t[0]];
    let side = c[0] * bv[0] + c[1] * bv[1] + c[2] * bv[2];
    if side.is_nan() || side.abs() <= 1e-9 * blen {
        return None;
    }
    Some(Tangent { t: t.map(|x| x as f32), h: side.signum() as f32 })
}

/// A material's normal-map luma for one pixel (docs/texture-packs.md, "How
/// material maps light"): `base` is the pixel's luma before truncation
/// without the material, `n` its normal as drawn (the interpolated corner
/// normals, or the face's; any length, face convention), `(x, y)` the
/// map's normal there (unit components along increasing u and v, mirror
/// signs applied) and `scale` the pack's strength. With [`Relief::Faithful`]
/// `(base + E1 - E0, None)`, with `E0`, `E1` the lamp at `n` and at `n`
/// tilted by the map (each clamped to 0-255); with [`Relief::Diffuse`]
/// `(base, Some(f))`: the pixel is drawn at `f` times the brightness the
/// flat surface is drawn at, texel by texel ([`relieved`]). None where the
/// map changes nothing (a flat texel, no frame, no normal, the same
/// value), so such a pixel takes the path without materials bit for bit.
pub fn material_luma(
    l: &PolyLamp,
    n: [f32; 3],
    base: f32,
    (x, y): (f32, f32),
    scale: f32,
) -> Option<(f32, Option<f32>)> {
    let tg = l.tangent?;
    let (kx, ky) = (scale * x, scale * y);
    if kx == 0.0 && ky == 0.0 {
        return None;
    }
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let unit = |v: [f32; 3]| {
        let len = dot(v, v).sqrt();
        (len >= 1e-6).then(|| v.map(|c| c / len))
    };
    let n = unit(n)?;
    let z = (1.0 - x * x - y * y).max(0.0).sqrt();
    let t = unit([kx, ky, z])?;
    let o = n.map(|c| c * l.out);
    let ot = dot(o, tg.t);
    let tp = unit([0, 1, 2].map(|c| tg.t[c] - o[c] * ot))?;
    let bp = [o[1] * tp[2] - o[2] * tp[1], o[2] * tp[0] - o[0] * tp[2], o[0] * tp[1] - o[1] * tp[0]].map(|c| c * tg.h);
    let o2 = unit([0, 1, 2].map(|c| t[2] * o[c] + t[0] * tp[c] + t[1] * bp[c]))?;
    let n2 = o2.map(|c| c * l.out);
    let lamp = &l.light.lamp;
    let facing = |n: [f32; 3]| {
        let d = (n[0] * lamp.l[0] + n[1] * lamp.l[1]) + n[2] * lamp.l[2];
        if lamp.front {
            d
        } else {
            -d
        }
    };
    match l.relief {
        Relief::Faithful => {
            let (e0, e1) = (lamp.eval(n).clamp(0.0, 255.0), lamp.eval(n2).clamp(0.0, 255.0));
            (e0.is_finite() && e1.is_finite() && e1 != e0).then_some((base + (e1 - e0), None))
        }
        Relief::Diffuse { tenths } => {
            if lamp.diffuse == 0.0 {
                return None;
            }
            // Lambert: no relief where no light reaches (facing away).
            let dd = facing(n2).max(0.0) - facing(n).max(0.0);
            if dd == 0.0 {
                return None;
            }
            // None where the factor is 1.
            let f = (1.0 + tenths as f32 / 10.0 * dd).max(0.0);
            (f.is_finite() && f != 1.0).then_some((base, Some(f.min(RELIEF_MAX))))
        }
    }
}

/// A polygon placed in a view's output pixels, ready for a rasteriser
/// (this one or a GPU backend).
#[derive(Clone, Debug)]
pub struct Placed {
    /// Index of the polygon in the (sorted) slice passed to [`place`].
    pub poly: usize,
    /// Vertices in output pixels.
    pub points: Vec<SVtx>,
    /// Clip rectangle (x0, y0, x1, y1) in output pixels.
    pub clip: (f32, f32, f32, f32),
    /// Position in draw order within the polygon's window (for the depth
    /// buffer's tie-break, [`depth_value`]).
    pub rank: u32,
    /// Polygons the game sorted to within [`GROUP_SPAN`] of one depth
    /// (including a header's "same depth as the previous polygon") share a
    /// group number: layers of one object, such as a marker and its outline
    /// or a jacket over a body, whose order is the game's. With the depth
    /// buffer they draw in order among themselves and are depth tested as a
    /// group against what was drawn before.
    pub group: u32,
    /// Drawn in the game's order, without the depth test or write, with the
    /// depth buffer on: sorted behind everything (sort key mode 3, and the
    /// polygons linked to it: skies), flat black shadows ([`Poly::shadow`]),
    /// translucent (checkerboard) polygons, whose layering over the scene
    /// is the game's, and polygons covering a wide range of depth
    /// ([`PAINTER_SPREAD`]).
    pub background: bool,
    /// [`Poly::prev`] in output pixels (homogeneous, `[X, Y, W]`); empty
    /// without.
    pub prev: Vec<[f32; 3]>,
}

/// Depth buffer. Off by default; the original look is the painter's
/// algorithm. Polygons carry 1/z per vertex (`SVtx::iz`), linear in screen
/// space, so each pixel's depth is exact. Per window (HUD windows draw over
/// the scene) the buffer starts empty. A polygon wins a pixel where its
/// depth is at least what is there, so equal depths keep the game's order,
/// and [`depth_value`] lets a later polygon win by up to [`DEPTH_BIAS`] of
/// its depth for each polygon between, which covers the rounding between a
/// decal and the surface under it. Polygons sorted together
/// ([`Placed::group`]) keep the game's order among themselves. What can't be
/// trusted to depth is drawn in the game's order without testing or writing
/// ([`Placed::background`]). Alpha-tested texels that aren't drawn don't
/// write depth.
pub const DEPTH_BIAS: f32 = 1e-5;
/// Scale of the depth buffer's mapping of 1/z into [0, 1), [`depth_value`].
pub const DEPTH_SCALE: f32 = 0.1;
/// How far (as a fraction of 1/z) behind what is drawn a later polygon may
/// be and still win the test ([`depth_test_value`]), whatever the number of
/// polygons between: the game's order is kept for a decal laid on a
/// faceted surface of its model (segawski's jersey text and logo lie up to
/// 0.3% behind the jersey's facets). What is written is the polygon's own
/// depth.
pub const DEPTH_TOLERANCE: f32 = 0.005;

/// The depth buffer's value for a fragment of 1/z `iz` and draw order
/// `rank` in its window: larger is nearer. Monotonic, and finest where 1/z
/// is small (far away), so a float buffer keeps about 1e-6 relative
/// precision over the whole range.
pub fn depth_value(iz: f32, rank: u32) -> f32 {
    depth_scaled(iz, rank, 1.0)
}

/// The value a fragment is tested with: [`depth_value`] with 1/z raised by
/// [`DEPTH_TOLERANCE`].
pub fn depth_test_value(iz: f32, rank: u32) -> f32 {
    depth_scaled(iz, rank, 1.0 + DEPTH_TOLERANCE)
}

fn depth_scaled(iz: f32, rank: u32, factor: f32) -> f32 {
    let z = iz * (1.0 + rank as f32 * DEPTH_BIAS) * factor;
    z / (z + DEPTH_SCALE)
}

/// Inputs the rasteriser reads besides the stream.
pub struct Sources<'a> {
    /// "textures" ROM (texture headers and point lists) as u16 words.
    pub texture_rom: &'a [u16],
    /// Texture point RAM (u16), written by raster command 4.
    pub texture_point_ram: &'a [u16],
    /// Render RAM from 0x11000000: texture banks at +0 / +0x200000, luma
    /// tables at +0x400000.
    pub render_ram: &'a [u8],
    /// Palette RAM from 0x01800000, including the colour translation
    /// tables at +0x10000.
    pub palette: &'a [u8],
    /// Screen offsets from the H/V sync registers: 84 + H, 130 + V.
    pub xoff: f32,
    pub yoff: f32,
}

impl Sources<'_> {
    /// A u16 from texture point space: bit 23 selects point RAM.
    fn point_word(&self, addr: u32) -> u16 {
        if addr & 0x80_0000 != 0 {
            self.texture_point_ram.get((addr & 0xffff) as usize).copied().unwrap_or(0)
        } else {
            self.texture_rom.get((addr & 0x7f_ffff) as usize).copied().unwrap_or(0)
        }
    }

    /// 4bpp texel at sheet coordinates (x, y) in `bank` (section 5.1).
    fn texel(&self, bank: u32, x: u32, y: u32) -> u32 {
        let (x, y) = if x >= 1024 { (x - 1024, y ^ 1024) } else { (x, y) };
        let word = ((y & 2047) / 2) * 512 + (x & 1023) / 2;
        let o = (bank as usize & 1) * 0x20_0000 + word as usize * 2;
        let w = u16::from_le_bytes([self.render_ram[o], self.render_ram[o + 1]]) as u32;
        let shift = 12 - 4 * ((x & 1) + 2 * (y & 1));
        (w >> shift) & 15
    }

    fn luma(&self, table: u32, index: u32) -> u32 {
        let o = 0x40_0000 + ((table * 128 + index) as usize & 0x7fff) * 2;
        self.render_ram.get(o).copied().unwrap_or(0) as u32
    }

    /// Colour translation: component `c` (5 bits) of channel `ch` at luma.
    fn translate(&self, ch: usize, c: u32, lum: u32) -> u8 {
        let o = 0x1_0000 + ch * 0x4000 + ((c * 256 + lum) as usize) * 2;
        self.palette.get(o).copied().unwrap_or(0)
    }
}

/// Whether two colours have the same hue, i.e. differ only in level: each
/// channel as a share of the brightest is within a quarter. Colours near
/// black (every channel below 32) have no reliable hue and match anything.
pub fn same_hue(a: [u8; 3], b: [u8; 3]) -> bool {
    let (ma, mb) = (*a.iter().max().unwrap(), *b.iter().max().unwrap());
    if ma < 32 || mb < 32 {
        return true;
    }
    (0..3).all(|k| (a[k] as f32 / ma as f32 - b[k] as f32 / mb as f32).abs() <= 0.25)
}

/// Smooth shading (`Poly::lit`): the lumas `[lo, hi]` (inclusive, around
/// the polygon's own `luma`) over which its colour keeps its hue. The
/// colour translation tables are the game's and need not be brightness
/// ramps: a game may map the top levels of a colour to another hue
/// (topskatr's ramp boards: white at luma 255, red a few levels below), so
/// that the polygon's luma in effect picks its colour. A lit pixel's luma
/// is clamped into this range, so shading changes a pixel's level, never
/// its hue (`lighting-check` J). Untextured: the columns (`luma / 4`)
/// around its own whose colour has the hue of its own. Textured: the same
/// for each level a texel can take (a value of its luma table, at indices
/// `index` (inclusive): those its texels can reach), intersected.
pub fn hue_range(src: &Sources, colour: [u32; 3], texture: Option<&Texture>, luma: u32, index: (u32, u32)) -> [u8; 2] {
    hue_range_with(&mut HueCache::default(), src, colour, texture, luma, index)
}

/// What [`hue_range`] works out that many polygons of a frame share: per
/// colour, the span of columns of its own hue around each column; per luma
/// table and indices, the distinct levels.
#[derive(Default)]
pub struct HueCache {
    spans: HashMap<[u32; 3], [Option<(u32, u32)>; 64]>,
    levels: HashMap<(u32, u32, u32), Vec<u32>>,
}

/// [`hue_range`] with what it works out kept in `cache` (one frame's
/// sources).
pub fn hue_range_with(
    cache: &mut HueCache,
    src: &Sources,
    colour: [u32; 3],
    texture: Option<&Texture>,
    luma: u32,
    index: (u32, u32),
) -> [u8; 2] {
    let spans = cache.spans.entry(colour).or_insert([None; 64]);
    let col = |k: u32| [0, 1, 2].map(|ch| src.translate(ch, colour[ch], k));
    // The columns around `own` (up to `top`) whose colour has its hue.
    let mut span = |own: u32, top: u32| {
        let (lo, hi) = *spans[own as usize].get_or_insert_with(|| {
            let c = col(own);
            let mut lo = own;
            while lo > 0 && same_hue(c, col(lo - 1)) {
                lo -= 1;
            }
            let mut hi = own;
            while hi < 63 && same_hue(c, col(hi + 1)) {
                hi += 1;
            }
            (lo, hi)
        });
        (lo, hi.min(top))
    };
    match texture {
        None => {
            let (lo, hi) = span(luma >> 2, 63);
            [(lo * 4) as u8, (hi * 4 + 3).min(255) as u8]
        }
        Some(t) => {
            let levels = cache.levels.entry((t.luma_table, index.0, index.1)).or_insert_with(|| {
                let mut seen = [false; 256];
                // Level 0 is column 0 at every luma: no limit.
                (index.0..=index.1.min(127))
                    .map(|i| src.luma(t.luma_table, i))
                    .filter(|&v| v != 0 && !std::mem::replace(&mut seen[v as usize], true))
                    .collect()
            });
            let (mut lo, mut hi) = (0u32, 255u32);
            for &v in levels.iter() {
                let c = |l: u32| (v * l / 256).min(63);
                let (top, own) = (c(255), c(luma));
                let (clo, chi) = span(own, top);
                // The lumas whose column is clo or more, chi or less.
                if clo > 0 {
                    lo = lo.max((clo * 256).div_ceil(v));
                }
                if chi < top {
                    hi = hi.min(((chi + 1) * 256 - 1) / v);
                }
            }
            [lo.min(luma) as u8, hi.max(luma) as u8]
        }
    }
}

/// The luma table indices a textured polygon's pixels can read: from its
/// darkest opaque texel to its brightest (bilinear filtering blends
/// neighbours, so anything between), `texel * 17 / 2`. All 128 when it has
/// no opaque texel.
pub fn texel_indices(src: &Sources, t: &Texture) -> (u32, u32) {
    let (mut lo, mut hi) = (15u32, 0u32);
    // A 16-bit word holds a 2x2 block of texels, and textures sit on
    // 32-texel boundaries: a word at a time (as `Sources::texel` reads).
    'rows: for y in (t.y0..t.y0 + t.h).step_by(2) {
        for x in (t.x0..t.x0 + t.w).step_by(2) {
            let (x, y) = if x >= 1024 { (x - 1024, y ^ 1024) } else { (x, y) };
            let word = ((y & 2047) / 2) * 512 + (x & 1023) / 2;
            let o = (t.bank as usize & 1) * 0x20_0000 + word as usize * 2;
            let (Some(&a), Some(&b)) = (src.render_ram.get(o), src.render_ram.get(o + 1)) else { continue };
            let w = u16::from_le_bytes([a, b]) as u32;
            for k in 0..4 {
                let v = (w >> (4 * k)) & 15;
                if t.translucent && v == 15 {
                    continue;
                }
                lo = lo.min(v);
                hi = hi.max(v);
            }
            if lo == 0 && hi == 15 {
                break 'rows;
            }
        }
    }
    if lo > hi {
        (0, 127)
    } else {
        (lo * 17 / 2, hi * 17 / 2)
    }
}

fn s12(v: u32) -> f32 {
    (((v & 0xfff) << 20) as i32 >> 20) as f32
}

fn rd16(ram: &[u8], entry: usize) -> u16 {
    let o = entry * 2;
    u16::from_le_bytes([ram.get(o).copied().unwrap_or(0), ram.get(o + 1).copied().unwrap_or(0)])
}

fn texture_from_header(h: [u16; 4]) -> Option<Texture> {
    if h[0] & (1 << 14) == 0 {
        return None;
    }
    Some(Texture {
        w: 32 << (h[0] & 7),
        h: 32 << ((h[0] >> 3) & 7),
        mirror_x: h[0] & (1 << 8) != 0,
        mirror_y: h[0] & (1 << 9) != 0,
        translucent: h[0] & (1 << 13) != 0,
        luma_table: (h[1] & 0xff) as u32,
        x0: ((h[2] & 0x3f) as u32) * 32,
        y0: (((h[2] >> 6) & 0x1f) as u32) * 32,
        bank: ((h[2] >> 12) & 1) as u32,
    })
}

/// Parse a frame's raster stream into screen-space polygons.
pub fn build(stream: &[u32], src: &Sources) -> Vec<Poly> {
    build_impl(stream, src, &BuildIn::default()).0
}

/// What [`build_with`] adds to the polygons: motion data (as
/// [`build_motion`]) and smooth shading's corner values.
#[derive(Default)]
pub struct BuildIn<'a> {
    pub motion: Option<&'a MotionIn<'a>>,
    /// `crate::light::shade`'s result for the stream: each lit polygon
    /// gets [`Poly::lit`] (shadows excepted, and checkerboard polygons
    /// when `skip_checker`).
    pub lit: Option<&'a crate::light::Shaded>,
    /// Checkerboard polygons stay flat (`light::Options::skip_checker`).
    pub skip_checker: bool,
    /// Lit polygons get [`Poly::fine`] (`light::Options::fine`).
    pub fine: bool,
    /// Material relief rule for [`PolyLamp::relief`]
    /// (`light::Options::relief`).
    pub relief: Relief,
    /// The luma table indices textures' texels reach ([`texel_indices`]),
    /// kept across frames while render RAM is unchanged (None: worked out
    /// for this frame alone).
    pub texels: Option<&'a std::cell::RefCell<TexelCache>>,
}

/// [`texel_indices`] by texture rectangle (bank, x0, y0, w, h,
/// translucent), kept across frames: valid while render RAM is unchanged,
/// which [`TexelCache::begin`] checks (as [`TexKeys`]).
#[derive(Clone, Debug, Default)]
pub struct TexelCache {
    version: Option<u64>,
    #[allow(clippy::type_complexity)]
    indices: HashMap<(u32, u32, u32, u32, u32, bool), (u32, u32)>,
}

impl TexelCache {
    /// Before a frame: keep what is known if render RAM is still at
    /// `render_version`, else forget it. `None` (version unknown) always
    /// forgets.
    pub fn begin(&mut self, render_version: Option<u64>) {
        if render_version.is_none() || render_version != self.version {
            self.indices.clear();
        }
        self.version = render_version;
    }

    /// Forget everything.
    pub fn clear(&mut self) {
        self.indices.clear();
        self.version = None;
    }

    /// Textures known.
    pub fn len(&self) -> usize {
        self.indices.len()
    }

    /// Whether no texture is known.
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

/// [`build`], smooth-shaded with `opts` (`crate::light::shade`) when given
/// and on.
pub fn build_shaded(stream: &[u32], src: &Sources, opts: Option<&crate::light::Options>) -> Vec<Poly> {
    build_shaded_with(stream, src, None, opts)
}

/// [`build_shaded`] with the stream's lighting side channel, if any
/// (`crate::light::shade_with`).
pub fn build_shaded_with(
    stream: &[u32],
    src: &Sources,
    side: Option<&crate::light::Frame>,
    opts: Option<&crate::light::Options>,
) -> Vec<Poly> {
    match opts.filter(|o| o.runs()) {
        Some(o) => {
            let sh = crate::light::shade_with(stream, side, o);
            let b = BuildIn {
                lit: Some(&sh),
                skip_checker: o.skip_checker,
                fine: o.fine,
                relief: o.relief,
                ..Default::default()
            };
            build_with(stream, src, &b).0
        }
        None => build(stream, src),
    }
}

/// [`build_motion`] with smooth shading too. The polygons are [`build`]'s
/// exactly, apart from [`Poly::prev`] and [`Poly::lit`].
pub fn build_with(stream: &[u32], src: &Sources, b: &BuildIn) -> (Vec<Poly>, Vec<Centres>) {
    let (polys, windows) = build_impl(stream, src, b);
    (polys, windows.iter().map(|w| w.centres).collect())
}

/// [`build`], and with motion data `m` each polygon's previous position
/// ([`Poly::prev`]); also the windows' centres, by window index (to pass
/// as the next frame's [`MotionIn::prev_centres`]). The polygons are
/// [`build`]'s exactly, apart from `prev`.
///
/// The streams are read in lockstep: a vertex's previous view-space
/// position is the words at its place in `s0` and `s1`, blended at `tp`.
/// It is carried through clipping (decided by the current position only)
/// like the texture coordinates, so it is exact for motion that is an
/// affine map of view space (rigid motion, scale). Without motion for the
/// frame (`m` None, or the streams not all the same length or not agreeing
/// on the command words), every `prev` is empty.
pub fn build_motion(stream: &[u32], src: &Sources, m: Option<&MotionIn>) -> (Vec<Poly>, Vec<Centres>) {
    build_with(stream, src, &BuildIn { motion: m, ..Default::default() })
}

fn build_impl(stream: &[u32], src: &Sources, input: &BuildIn) -> (Vec<Poly>, Vec<Window>) {
    let m = input.motion;
    // Smooth shading: the lit polygons by the stream index of their
    // attribute word, ascending, walked with the stream.
    let lit = input.lit.filter(|l| !l.at.is_empty());
    let mut lit_next = 0;
    // The luma table indices each texture's texels reach (`texel_indices`),
    // worked out once a frame, or kept across frames by the caller.
    #[allow(clippy::type_complexity)]
    let mut own_texels: HashMap<(u32, u32, u32, u32, u32, bool), (u32, u32)> = HashMap::new();
    let mut kept = input.texels.map(|c| c.borrow_mut());
    let texels = match kept.as_deref_mut() {
        Some(c) => &mut c.indices,
        None => &mut own_texels,
    };
    // `hue_range` by colour, luma table, luma and indices, likewise.
    #[allow(clippy::type_complexity)]
    let mut hues: HashMap<([u32; 3], Option<u32>, u32, (u32, u32)), [u8; 2]> = HashMap::new();
    let mut hue_cache = HueCache::default();
    let mut polys = Vec::new();
    let mut windows: Vec<Window> = Vec::new();
    let mut block = 0;
    let mut i = 0;
    let rd = |i: usize| stream.get(i).copied().unwrap_or(0xff00_000f);
    let given = m.is_some();
    let mut m = m.filter(|m| m.s0.len() == stream.len() && m.s1.len() == stream.len());
    // With motion: the stream's `0x1` commands so far (what `valid` is
    // indexed by; `block` only counts those after a window), and the one
    // each polygon came in.
    let (mut blocks, mut from) = (0usize, Vec::new());
    let prev_at = |m: &MotionIn, k: usize| {
        let (a, b) = (f24(m.s0[k]), f24(m.s1[k]));
        a + (b - a) * m.tp
    };
    while i < stream.len() {
        let w = stream[i];
        i += 1;
        if w >> 24 != 0xff {
            continue; // stray data word: resynchronise on the next command
        }
        if let Some(mm) = m {
            if mm.s0[i - 1] != w || mm.s1[i - 1] != w {
                m = None; // not the same layout: no motion for the frame
            } else if w & 0xf == 0x1 {
                blocks += 1;
            }
        }
        let centre = ((w >> 6) & 3) as usize;
        match w & 0xf {
            0x3 => {
                // Window: start, end, centres 0-3 as 0x00XXXYYY, window Y up.
                let f = |k: usize| (s12(rd(i + k) >> 12), s12(rd(i + k)));
                let to_screen = |(x, y): (f32, f32)| (x + src.xoff, (384.0 - y) + src.yoff);
                let (ax, ay) = to_screen(f(0));
                let (bx, by) = to_screen(f(1));
                let mut centres = [(0.0, 0.0); 4];
                for (k, c) in centres.iter_mut().enumerate() {
                    *c = to_screen(f(2 + k));
                }
                windows.push(Window { x0: ax.min(bx), x1: ax.max(bx), y0: ay.min(by), y1: ay.max(by), centres });
                i += 6;
            }
            0x8 => i += 1,
            0x4 => {
                let count = rd(i + 1) as usize;
                i += 2 + count;
            }
            0x1 => {
                let Some(win) = windows.len().checked_sub(1) else { continue };
                let mut tpa = rd(i) & 0xff_ffff;
                let mut tha = rd(i + 1) & 0xff_ffff;
                let pos = |k: usize| {
                    let (x, y, z) = (f24(rd(k)), f24(rd(k + 1)), f24(rd(k + 2)));
                    match m {
                        Some(m) if k + 2 < stream.len() => {
                            let (px, py, pz) = (prev_at(m, k), prev_at(m, k + 1), prev_at(m, k + 2));
                            Vtx { x, y, z, u: 0.0, v: 0.0, px, py, pz, lit: [0.0; 4] }
                        }
                        _ => Vtx { x, y, z, ..Default::default() },
                    }
                };
                let mut p0 = pos(i + 2);
                let mut p1 = pos(i + 5);
                i += 8;
                let mut prev_z = 1e10f32;
                block += 1;
                loop {
                    let attr = rd(i);
                    if attr >> 24 == 0xff || attr & 3 == 0 {
                        break;
                    }
                    let quad = attr & 1 != 0;
                    let luma_word = rd(i + 1);
                    let at = i as u32;
                    let mut lamped = None;
                    let corners = lit
                        .and_then(|l| {
                            while l.at.get(lit_next).is_some_and(|&a| (a as usize) < i) {
                                lit_next += 1;
                            }
                            (l.at.get(lit_next) == Some(&(i as u32))).then(|| {
                                lamped = l.lamps.get(lit_next).copied().flatten();
                                let flat = l.flat.get(lit_next).copied().unwrap_or(false);
                                (&l.corners[lit_next], l.pixel.get(lit_next).copied().flatten(), flat)
                            })
                        })
                        // Entries only for materials aren't lit.
                        .filter(|c| !c.2)
                        .map(|(c, px, _)| (c, px));
                    let pa = pos(i + 3);
                    let pb = if quad { pos(i + 6) } else { pa };
                    i += if quad { 9 } else { 6 };

                    let mut verts: Vec<Vtx> = if quad { vec![p1, p0, pa, pb] } else { vec![p1, p0, pa] };
                    // Texture points: (V, U) pairs, 13.3 fixed, per vertex.
                    for (k, vtx) in verts.iter_mut().enumerate() {
                        vtx.v = src.point_word(tpa + 2 * k as u32) as f32 / 8.0;
                        vtx.u = src.point_word(tpa + 2 * k as u32 + 1) as f32 / 8.0;
                        if let Some((c, _)) = corners {
                            vtx.lit = c[k];
                        }
                    }
                    tpa += 2 * verts.len() as u32;

                    let header = [0, 1, 2, 3].map(|k| src.point_word(tha + k));
                    let step = ((((attr >> 12) & 31) << 27) as i32) >> 27;
                    tha = (tha as i32 + step * 4) as u32 & 0xff_ffff;

                    let link = (attr >> 8) & 3;
                    let double_sided = attr & (1 << 17) != 0;
                    let backface = luma_word & (1 << 23) != 0;
                    let (minz, maxz) = verts.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.z), b.max(p.z)));
                    let key = match (attr >> 10) & 3 {
                        0 => prev_z,
                        1 => minz,
                        2 => maxz,
                        _ => 1e10,
                    };
                    prev_z = key;

                    if link != 0 && (double_sided || !backface) && maxz > 0.0 {
                        let w = &windows[win];
                        let (cx, cy) = w.centres[centre];
                        let clipped = clip_view(&verts, cx, cy);
                        if clipped.len() >= 3 {
                            let prev = match m {
                                Some(m) => {
                                    let (pcx, pcy) = m.prev_centres.get(win).map_or((cx, cy), |c| c[centre]);
                                    from.push(blocks - 1);
                                    previous(&clipped, pcx, pcy)
                                }
                                None => Vec::new(),
                            };
                            let points = clipped
                                .iter()
                                .map(|p| {
                                    let iz = 1.0 / p.z;
                                    SVtx { x: cx + p.x * iz, y: cy - p.y * iz, iz, uz: p.u * iz, vz: p.v * iz }
                                })
                                .collect();
                            let c = rd16(src.palette, 0x1000 + ((header[3] >> 6) as usize & 0x3ff)) as u32;
                            let colour = [c & 31, (c >> 5) & 31, (c >> 10) & 31];
                            let luma = (luma_word >> 15) & 0xff;
                            let texture = texture_from_header(header);
                            let shadow =
                                texture.is_none() && (0..3).all(|ch| src.translate(ch, colour[ch], luma >> 2) == 0);
                            let checker = header[0] & (1 << 15) != 0;
                            // Lit: clamped to the lumas that keep its hue;
                            // none besides its own: flat.
                            let lit_at =
                                corners.filter(|_| !shadow && !(checker && input.skip_checker)).and_then(|(cs, px)| {
                                    let table = texture.map(|t| t.luma_table);
                                    let mut range = *hues.entry((colour, table, luma, (0, 127))).or_insert_with(|| {
                                        hue_range_with(&mut hue_cache, src, colour, texture.as_ref(), luma, (0, 127))
                                    });
                                    // Corner lumas inside the range need no more (an
                                    // interpolated luma lies between them); per pixel
                                    // the range must be the widest that is safe.
                                    let inside = px.is_none() && {
                                        let q = |k: usize| crate::light::quantise(cs[k][0]);
                                        (0..verts.len()).all(|k| (range[0] as u32..=range[1] as u32).contains(&q(k)))
                                    };
                                    if let (Some(t), true) = (&texture, range != [0, 255] && !inside) {
                                        // Only the levels its texels can take.
                                        let key = (t.bank, t.x0, t.y0, t.w, t.h, t.translucent);
                                        let index = *texels.entry(key).or_insert_with(|| texel_indices(src, t));
                                        if index != (0, 127) {
                                            range = *hues.entry((colour, table, luma, index)).or_insert_with(|| {
                                                hue_range_with(
                                                    &mut hue_cache,
                                                    src,
                                                    colour,
                                                    texture.as_ref(),
                                                    luma,
                                                    index,
                                                )
                                            });
                                        }
                                    }
                                    (range[0] != range[1]).then_some((px, range))
                                });
                            let (lit, pixel, lit_range, fine) = match lit_at {
                                Some((px, range)) => (clipped.iter().map(|v| v.lit).collect(), px, range, input.fine),
                                None => (Vec::new(), None, [0, 255], false),
                            };
                            // Material maps: textured polygons with a lamp.
                            let lamp = match (lamped, &texture) {
                                (Some(lm), Some(t)) if !shadow => {
                                    let range = if lit.is_empty() {
                                        // The widest range that keeps the hue (as per
                                        // pixel lighting).
                                        let table = Some(t.luma_table);
                                        let key = (t.bank, t.x0, t.y0, t.w, t.h, t.translucent);
                                        let index = *texels.entry(key).or_insert_with(|| texel_indices(src, t));
                                        *hues.entry((colour, table, luma, index)).or_insert_with(|| {
                                            hue_range_with(&mut hue_cache, src, colour, Some(t), luma, index)
                                        })
                                    } else {
                                        lit_range
                                    };
                                    let [fx, fy] = lm.focal;
                                    let ring: Vec<([f32; 3], [f32; 2])> =
                                        verts.iter().map(|v| ([v.x / fx, v.y / fy, v.z], [v.u, v.v])).collect();
                                    // The side the eye sees: the face's plane
                                    // offset `n . p` is negative where `n` points
                                    // toward the eye (at the origin).
                                    let mid = ring
                                        .iter()
                                        .fold([0.0f32; 3], |a, (p, _)| [a[0] + p[0], a[1] + p[1], a[2] + p[2]]);
                                    let n = lm.n;
                                    let out =
                                        if n[0] * mid[0] + n[1] * mid[1] + n[2] * mid[2] >= 0.0 { -1.0 } else { 1.0 };
                                    let tangent = tangent_frame(&ring, n.map(|c| c * out));
                                    Some(Box::new(PolyLamp {
                                        light: lm.light,
                                        n,
                                        out,
                                        tangent,
                                        range,
                                        relief: input.relief,
                                    }))
                                }
                                _ => None,
                            };
                            polys.push(Poly {
                                points,
                                key,
                                window: win,
                                clip: (w.x0, w.y0, w.x1, w.y1),
                                texture,
                                colour,
                                luma,
                                shadow,
                                checker,
                                header,
                                block: block - 1,
                                at,
                                prev,
                                lit,
                                pixel,
                                lit_range,
                                fine,
                                lamp,
                            });
                        }
                    }

                    // Next polygon's leading edge (section 3.4).
                    (p0, p1) = match link {
                        1 => (p0, pa),
                        3 => (pb, p1),
                        _ => (pa, pb),
                    };
                }
            }
            _ => {}
        }
    }
    match m {
        // Objects whose motion is unknown: no previous position.
        Some(m) if m.valid.len() == blocks => {
            for (p, &b) in polys.iter_mut().zip(&from) {
                if !m.valid[b] {
                    p.prev = Vec::new();
                }
            }
        }
        Some(_) => {}
        // Motion lost on the way (the streams' layouts differ): none.
        None if given => polys.iter_mut().for_each(|p| p.prev = Vec::new()),
        None => {}
    }
    (polys, windows)
}

/// A clipped polygon's previous positions ([`Poly::prev`]) for previous
/// window centre (cx', cy'): per point `[(cx' z' + x') / z, (cy' z' - y') /
/// z, z' / z]` for its depth `z` and previous view-space position (x', y',
/// z'), so the first two over the third are the previous screen position
/// `(cx' + x'/z', cy' - y'/z')`. Empty if any previous position is behind
/// the eye.
fn previous(v: &[Vtx], cx: f32, cy: f32) -> Vec<[f32; 3]> {
    if !v.iter().all(|p| p.pz > NEAR) {
        return Vec::new();
    }
    v.iter()
        .map(|p| {
            let iz = 1.0 / p.z;
            [(cx * p.pz + p.px) * iz, (cy * p.pz - p.py) * iz, p.pz * iz]
        })
        .collect()
}

/// Nearest view-space z kept by [`clip_view`]: only there to keep 1/z
/// finite. The side planes bound x/z and y/z, so a vertex this close
/// to the eye still projects inside the guard band.
const NEAR: f32 = 1e-6;

/// Screen-space guard band [`clip_view`] clips to, in game pixels beyond
/// each edge of the screen: wide enough for any widened view to draw what
/// lies past the original frame, and small enough to keep projected
/// coordinates (and 1/z across a span) well inside f32 precision.
const GUARD: f32 = 2.0 * WIDTH as f32;

/// Clip a view-space polygon to the volume that projects into the guard
/// band around the screen, given the window centre (cx, cy) in screen
/// pixels (sx = cx + x/z, sy = cy - y/z): four side planes through the
/// eye and a near plane at [`NEAR`]. Polygons are clipped, not dropped,
/// so geometry crossing in front of the camera keeps its visible part.
fn clip_view(v: &[Vtx], cx: f32, cy: f32) -> Vec<Vtx> {
    let (x0, x1) = (-GUARD - cx, WIDTH as f32 + GUARD - cx);
    let (y0, y1) = (cy - (HEIGHT as f32 + GUARD), cy + GUARD);
    // Each plane as (a, b, c, d): inside where a·x + b·y + c·z + d >= 0.
    let planes = [
        [0.0, 0.0, 1.0, -NEAR],
        [1.0, 0.0, -x0, 0.0],
        [-1.0, 0.0, x1, 0.0],
        [0.0, 1.0, -y0, 0.0],
        [0.0, -1.0, y1, 0.0],
    ];
    let mut poly = v.to_vec();
    for [a, b, c, d] in planes {
        if poly.len() < 3 {
            break;
        }
        poly = clip_plane(&poly, |p| a * p.x + b * p.y + c * p.z + d);
    }
    poly
}

/// Sutherland-Hodgman against one plane (inside where `dist` >= 0),
/// interpolating texture coordinates linearly in view space.
fn clip_plane(v: &[Vtx], dist: impl Fn(&Vtx) -> f32) -> Vec<Vtx> {
    let mut out = Vec::with_capacity(v.len() + 1);
    for k in 0..v.len() {
        let (a, b) = (v[k], v[(k + 1) % v.len()]);
        let (da, db) = (dist(&a), dist(&b));
        if da >= 0.0 {
            out.push(a);
        }
        if (da >= 0.0) != (db >= 0.0) {
            let t = da / (da - db);
            let l = |p: f32, q: f32| p + (q - p) * t;
            out.push(Vtx {
                x: l(a.x, b.x),
                y: l(a.y, b.y),
                z: l(a.z, b.z),
                u: l(a.u, b.u),
                v: l(a.v, b.v),
                px: l(a.px, b.px),
                py: l(a.py, b.py),
                pz: l(a.pz, b.pz),
                lit: [0, 1, 2, 3].map(|c| l(a.lit[c], b.lit[c])),
            });
        }
    }
    out
}

/// Wrap a texel coordinate into 0..size, mirrored or repeating.
fn wrap(c: i32, size: u32, mirror: bool) -> u32 {
    let s = size as i32;
    if mirror {
        let m = c.rem_euclid(2 * s);
        (if m >= s { 2 * s - 1 - m } else { m }) as u32
    } else {
        c.rem_euclid(s) as u32
    }
}

/// Bilinearly filtered texel at (u, v), scaled to 8 bits (0-255). For a
/// translucent texture, texel 15 is transparent: the opaque neighbours'
/// average stands in for it, and the pixel is dropped when less than half
/// its weight is opaque (docs/hw/geometrizer.md 5.4; the hardware filter is
/// unconfirmed).
fn filtered_texel(t: &Texture, src: &Sources, u: f32, v: f32) -> Option<u32> {
    let (u, v) = (u - 0.5, v - 0.5);
    let (fu, fv) = (u - u.floor(), v - v.floor());
    let (u0, v0) = (u.floor() as i32, v.floor() as i32);
    let (mut sum, mut weight) = (0.0, 0.0);
    for (du, dv, w) in
        [(0, 0, (1.0 - fu) * (1.0 - fv)), (1, 0, fu * (1.0 - fv)), (0, 1, (1.0 - fu) * fv), (1, 1, fu * fv)]
    {
        let tx = t.x0 + wrap(u0 + du, t.w, t.mirror_x);
        let ty = t.y0 + wrap(v0 + dv, t.h, t.mirror_y);
        let texel = src.texel(t.bank, tx, ty);
        if !(t.translucent && texel == 15) {
            sum += texel as f32 * w;
            weight += w;
        }
    }
    if weight < 0.5 {
        return None;
    }
    Some((sum / weight * 17.0).round() as u32)
}

/// Shade one pixel; None if it is transparent.
fn shade(p: &Poly, src: &Sources, x: usize, y: usize, u: f32, v: f32) -> Option<[u8; 3]> {
    shade_luma(p, PxLuma::own(p), src, x, y, u, v)
}

/// A pixel's brightness as its polygon is drawn there: the luma the game's
/// levels take (the polygon's own, or a smooth-shaded pixel's), and with
/// finer levels ([`Poly::fine`], and every pixel a material changes) that
/// pixel's luma before truncation with the lumas it is clamped to
/// ([`Poly::lit_range`], [`PolyLamp::range`]).
#[derive(Clone, Copy, PartialEq)]
struct PxLuma {
    luma: u32,
    fine: Option<(f32, [u8; 2])>,
    /// The relief rule's factor ([`relieved`]); `fine` is then the
    /// flat luma.
    rel: Option<f32>,
}

impl PxLuma {
    fn own(p: &Poly) -> Self {
        PxLuma { luma: p.luma, fine: None, rel: None }
    }

    /// The luma and finer levels for a texel of luma table value `table`.
    fn at(self, table: Option<u32>) -> (u32, Option<(f32, [u8; 2])>) {
        match (self.fine, self.rel) {
            (Some((l, range)), Some(f)) => {
                let l = relieved(l, table, range[1], f);
                (crate::light::quantise(l).clamp(range[0] as u32, range[1] as u32), Some((l, range)))
            }
            _ => (self.luma, self.fine),
        }
    }
}

/// Smooth shading's finer levels (`shading_fine`): for a lit pixel whose
/// luma before truncation is `luma`, of a polygon whose pixels are clamped
/// to the lumas `range` ([`Poly::lit_range`]), the two adjacent colour
/// columns to blend `(k, k1)` and the weight of `k1`. `table`: the luma
/// table's value at the pixel's texel (textured), or None (untextured).
///
/// The game's column at luma `l` is `level(l)` truncated, with `level(l) =
/// table * l / 256` (at most 63) or `l / 4`. A column stands for the levels
/// from k to k + 1, so a level `y` is drawn as the blend of the columns
/// around `y - 0.5` (column k alone at k + 0.5): across a gradient the
/// mean column is the game's, nothing gets brighter or darker. Both
/// columns are among those the polygon shows with the game's levels at the
/// lumas in `range`, which keep its hue: the blend is between two of the
/// game's own colours of that hue, and never leaves it.
pub fn fine_level(table: Option<u32>, luma: f32, range: [u8; 2]) -> (u32, u32, f32) {
    let level = |l: u32| match table {
        Some(v) => (v * l / 256).min(63),
        None => (l >> 2).min(63),
    };
    let (a, b) = (level(range[0] as u32), level(range[1] as u32));
    let l = if luma.is_nan() { 0.0 } else { luma };
    let y = match table {
        Some(v) => v as f32 * l / 256.0,
        None => l / 4.0,
    } - 0.5;
    let x = y.clamp(a as f32, b as f32);
    let k = (x.floor() as u32).min(b);
    (k, (k + 1).min(b), x - k as f32)
}

/// The columns finer levels blend for a pixel of colour `colour`
/// ([`fine_level`]'s `(k, k1, weight)`), kept off black: where the game's
/// own column there, `game`, is pure black (all three channels 0) the pixel
/// is that column alone, and a pair with one pure black column takes the
/// other alone. Black ink (outlines, eyes, seams) stays black and its
/// neighbours keep the game's colour next to it, with no dark fringe
/// blending toward it; elsewhere the blend is unchanged.
fn fine_columns(src: &Sources, colour: [u32; 3], game: u32, k: (u32, u32, f32)) -> (u32, u32, f32) {
    let black = |k: u32| (0..3).all(|ch| src.translate(ch, colour[ch], k) == 0);
    if black(game) {
        (game, game, 0.0)
    } else if black(k.0) {
        (k.1, k.1, 0.0)
    } else if black(k.1) {
        (k.0, k.0, 0.0)
    } else {
        k
    }
}

/// Channel `ch` of colour component `c` blended between the columns
/// [`fine_level`] gives, before rounding.
fn fine_translate(src: &Sources, ch: usize, c: u32, (k, k1, f): (u32, u32, f32)) -> f32 {
    let (a, b) = (src.translate(ch, c, k) as f32, src.translate(ch, c, k1) as f32);
    a + (b - a) * f
}

/// [`shade`] at a given brightness (a smooth-shaded pixel's).
fn shade_luma(p: &Poly, px: PxLuma, src: &Sources, x: usize, y: usize, u: f32, v: f32) -> Option<[u8; 3]> {
    if p.checker && (x ^ y) & 1 != 0 {
        return None;
    }
    let table = match p.texture {
        Some(t) => Some(src.luma(t.luma_table, filtered_texel(&t, src, u, v)? / 2)),
        None => None,
    };
    let (luma, fine) = px.at(table);
    let lum = match table {
        Some(value) => (value * luma / 256).min(63),
        None => luma >> 2,
    };
    if let Some((l, range)) = fine {
        let k = fine_columns(src, p.colour, lum, fine_level(table, l, range));
        // Rust's round(): half away from zero (never negative here).
        return Some([0, 1, 2].map(|ch| fine_translate(src, ch, p.colour[ch], k).round() as u8));
    }
    Some([0, 1, 2].map(|ch| src.translate(ch, p.colour[ch], lum)))
}

/// Describe the polygons covering screen pixel (x, y), in draw order
/// (debugging aid: `remodel run` prints these with `REMODEL_PROBE=x,y`).
pub fn probe(polys: &[Poly], src: &Sources, x: f32, y: f32) -> Vec<String> {
    let inside = |p: &Poly| {
        let n = p.points.len();
        let side = |i: usize| {
            let (a, b) = (p.points[i], p.points[(i + 1) % n]);
            (b.x - a.x) * (y - a.y) - (b.y - a.y) * (x - a.x)
        };
        n >= 3 && ((0..n).all(|i| side(i) >= 0.0) || (0..n).all(|i| side(i) <= 0.0))
    };
    polys
        .iter()
        .filter(|p| inside(p))
        .map(|p| {
            let mut hist = [0u32; 16];
            if let Some(t) = p.texture {
                for ty in 0..t.h {
                    for tx in 0..t.w {
                        hist[src.texel(t.bank, t.x0 + tx, t.y0 + ty) as usize] += 1;
                    }
                }
            }
            format!(
                "window {} key {:.1} 1/z {:.5}..{:.5} header {:04x?} colour {:?} luma {} texel histogram {hist:?}\n    texture {:?} points {:?}",
                p.window,
                p.key,
                p.points.iter().map(|v| v.iz).fold(f32::MAX, f32::min),
                p.points.iter().map(|v| v.iz).fold(f32::MIN, f32::max),
                p.header,
                p.colour,
                p.luma,
                p.texture,
                p.points.iter().map(|v| (v.x as i32, v.y as i32)).collect::<Vec<_>>()
            ) + &p.lamp.as_ref().map_or(String::new(), |l| {
                format!(
                    "\n    material lamp {:?} offset {} n {:?} out {} tangent {:?} range {:?}; at n {:.2}",
                    l.light.lamp,
                    l.light.offset,
                    l.n,
                    l.out,
                    l.tangent,
                    l.range,
                    l.light.lamp.eval(l.n)
                )
            })
        })
        .collect()
}

/// Largest texture (in texels) that dumps and packs handle; bigger ones
/// come from garbage headers.
const MAX_HOOKED_TEXELS: u32 = 1 << 21;

/// Texel hashes of sheet rectangles (`[bank, x0, y0, w, h]`) for
/// `tex_info`. A fresh one serves one frame; one kept across frames stays
/// valid while render RAM is unchanged, which `begin` checks.
#[derive(Clone, Debug, Default)]
pub struct TexKeys {
    version: Option<u64>,
    rects: HashMap<[u32; 5], u64>,
}

impl TexKeys {
    /// Before a frame: keep the hashes if render RAM is still at
    /// `render_version` (`Model2Bus::video_ram_versions`), else forget them.
    /// `None` (version unknown) always forgets.
    pub fn begin(&mut self, render_version: Option<u64>) {
        if render_version.is_none() || render_version != self.version {
            self.rects.clear();
        }
        self.version = render_version;
    }

    /// Forget every hash (render RAM changed in a way the version doesn't
    /// show, such as switching to another machine's RAM).
    pub fn clear(&mut self) {
        self.rects.clear();
        self.version = None;
    }
}

/// Identify a polygon's texture for dumps and packs: a hash of its texels
/// (cached per sheet rectangle in `keys`) and of the header fields and
/// colour that change its appearance. None for an oversized (garbage)
/// texture.
pub fn tex_info(p: &Poly, t: &Texture, src: &Sources, keys: &mut TexKeys) -> Option<TexInfo> {
    if t.w * t.h > MAX_HOOKED_TEXELS {
        return None;
    }
    let texels = *keys.rects.entry([t.bank, t.x0, t.y0, t.w, t.h]).or_insert_with(|| {
        let mut f = Fnv::default();
        for y in 0..t.h {
            for x in 0..t.w {
                f.byte(src.texel(t.bank, t.x0 + x, t.y0 + y) as u8);
            }
        }
        f.0
    });
    let colour = (p.colour[0] | p.colour[1] << 5 | p.colour[2] << 10) as u16;
    // One FNV-1a stream: the surface key is its state after the texels and
    // the size, the image key goes on through the fields that change the
    // texture's look (docs/texture-packs.md).
    let mut f = Fnv::default();
    f.bytes(&texels.to_le_bytes());
    for v in [t.w, t.h] {
        f.bytes(&v.to_le_bytes());
    }
    let surface = f.0;
    for v in [t.luma_table, t.translucent as u32, colour as u32] {
        f.bytes(&v.to_le_bytes());
    }
    Some(TexInfo {
        key: f.0,
        surface,
        width: t.w,
        height: t.h,
        header: p.header,
        colour,
        bank: t.bank,
        x0: t.x0,
        y0: t.y0,
        luma_table: t.luma_table,
        translucent: t.translucent,
    })
}

/// Full polygon luma: the brightest a texture can be drawn.
const FULL_LUMA: u32 = 255;

/// A texture as the game's pipeline draws it at full polygon luma, one
/// pixel per texel (unfiltered); texel 15 of a translucent texture has
/// alpha 0. This is what dumps contain and packs replace.
pub fn texture_image(src: &Sources, info: &TexInfo) -> Rgba {
    let look = texture_look(src, info);
    let mut data = Vec::with_capacity((info.width * info.height * 4) as usize);
    for y in 0..info.height {
        for x in 0..info.width {
            let t = src.texel(info.bank, info.x0 + x, info.y0 + y) as usize;
            let rgb = look[t];
            data.extend_from_slice(&[rgb[0], rgb[1], rgb[2], if info.translucent && t == 15 { 0 } else { 255 }]);
        }
    }
    Rgba { width: info.width as usize, height: info.height as usize, data }
}

/// The colour each texel value (0-15) of a texture is drawn in at full
/// polygon luma: what `texture_image` maps texels through. It depends on
/// the luma table and colour translation tables as they are now, which
/// the texture's key does not include: a game rewriting the translation
/// tables (srallyc's fade from white into the race, sgt24h's sky tint)
/// draws the same key in another look.
pub fn texture_look(src: &Sources, info: &TexInfo) -> [[u8; 3]; 16] {
    let c = [info.colour as u32 & 31, (info.colour as u32 >> 5) & 31, (info.colour as u32 >> 10) & 31];
    std::array::from_fn(|t| {
        let lum = (src.luma(info.luma_table, t as u32 * 17 / 2) * FULL_LUMA / 256).min(63);
        [0, 1, 2].map(|ch| src.translate(ch, c[ch], lum))
    })
}

/// Lighting for a replaced texture: the replacement stands for the texture
/// at full luma, so each pixel is scaled per channel by what the original
/// pipeline gives at this polygon's luma over what it gives at full luma,
/// for the original (filtered) texel under the pixel. That keeps the
/// game's luma table, cap at 63 and colour translation curves. Where that
/// texel is black at full luma in a channel, the channel adds the
/// original's value instead ([`Light::apply`]).
struct Light {
    /// Polygon luma is full: nothing to scale.
    full: bool,
    colour: [u32; 3],
    /// Fallback ratio where the original texel is transparent or black at
    /// full luma: the ratio for the brightest luma.
    poly: [f32; 3],
    /// The pixel's brightness: its luma, with finer levels
    /// ([`PxLuma::fine`]) the luma before truncation and the range it is
    /// clamped to, and the relief rule's factor (applied per texel).
    px: PxLuma,
}

impl Light {
    /// For polygon `p` drawn at `px` (its own luma, or a smooth-shaded
    /// pixel's).
    fn new(p: &Poly, px: PxLuma, src: &Sources) -> Self {
        let luma = px.luma;
        let fine = px.fine;
        let poly = [0, 1, 2].map(|ch| {
            let full = src.translate(ch, p.colour[ch], 63) as f32;
            if full > 0.0 {
                Self::at(src, p.colour, px, ch, 64) / full
            } else {
                luma as f32 / FULL_LUMA as f32
            }
        });
        Light { full: fine.is_none() && luma >= FULL_LUMA, colour: p.colour, poly, px }
    }

    /// Channel `ch` of `colour` at luma table value `value`: the game's
    /// level, or with finer levels the blend of two.
    fn at(src: &Sources, colour: [u32; 3], px: PxLuma, ch: usize, value: u32) -> f32 {
        let (luma, fine) = px.at(Some(value));
        match fine {
            Some((l, range)) => {
                let game = (value * luma / 256).min(63);
                fine_translate(src, ch, colour[ch], fine_columns(src, colour, game, fine_level(Some(value), l, range)))
            }
            None => src.translate(ch, colour[ch], (value * luma / 256).min(63)) as f32,
        }
    }

    /// Per channel, the scale and the offset that take the replacement's
    /// colour to the lit one (`replacement * scale + offset`). The offset
    /// is zero except where the original texel is black at full luma in
    /// that channel but not at this luma (a translation table that is not
    /// monotonic, such as topskatr's red at level 63 over a brown at lower
    /// levels): nothing scales up from black, so that channel adds the
    /// original's own value at this luma.
    fn apply(&self, t: &Texture, src: &Sources, u: f32, v: f32, rgb: [f32; 3]) -> [u8; 3] {
        let px = |c: f32| c.round().clamp(0.0, 255.0) as u8;
        if self.full {
            return rgb.map(px);
        }
        let Some(t8) = filtered_texel(t, src, u, v) else { return [0, 1, 2].map(|ch| px(rgb[ch] * self.poly[ch])) };
        let l = src.luma(t.luma_table, t8 / 2);
        let lf = (l * FULL_LUMA / 256).min(63);
        [0, 1, 2].map(|ch| {
            let full = src.translate(ch, self.colour[ch], lf);
            let here = Self::at(src, self.colour, self.px, ch, l);
            px(if full == 0 { rgb[ch] * self.poly[ch] + here } else { rgb[ch] * (here / full as f32) })
        })
    }
}

/// Shade one pixel from a replacement image; None if transparent.
#[allow(clippy::too_many_arguments)]
fn shade_replaced(
    t: &Texture,
    checker: bool,
    img: &Rgba,
    light: &Light,
    src: &Sources,
    x: usize,
    y: usize,
    u: f32,
    v: f32,
) -> Option<[u8; 3]> {
    if checker && (x ^ y) & 1 != 0 {
        return None;
    }
    let mode = |mirror| if mirror { Address::Mirror } else { Address::Repeat };
    let sx = u * img.width as f32 / t.w as f32;
    let sy = v * img.height as f32 / t.h as f32;
    let (rgb, alpha) = img.sample(sx, sy, mode(t.mirror_x), mode(t.mirror_y));
    if alpha < 0.5 {
        return None;
    }
    Some(light.apply(t, src, u, v, rgb))
}

/// A replacement image as the filter wants it.
enum Replacement {
    Image(Arc<Rgba>),
    Mips(Arc<Vec<Rgba>>),
}

/// Screen-space gradients (d/dx, d/dy) of 1/z, u/z and v/z over a placed
/// polygon: a plane through the fan triangle of largest area (the three
/// are affine on screen for a planar polygon).
struct Gradients {
    dx: [f32; 3],
    dy: [f32; 3],
}

impl Gradients {
    fn new(pts: &[SVtx]) -> Self {
        let mut best = (0.0f32, Gradients { dx: [0.0; 3], dy: [0.0; 3] });
        for k in 1..pts.len().saturating_sub(1) {
            let (a, b, c) = (pts[0], pts[k], pts[k + 1]);
            let (x1, y1, x2, y2) = (b.x - a.x, b.y - a.y, c.x - a.x, c.y - a.y);
            let det = x1 * y2 - x2 * y1;
            if det.abs() <= best.0 {
                continue;
            }
            let attr = |v: SVtx| [v.iz, v.uz, v.vz];
            let (fa, fb, fc) = (attr(a), attr(b), attr(c));
            let dx = [0, 1, 2].map(|i| ((fb[i] - fa[i]) * y2 - (fc[i] - fa[i]) * y1) / det);
            let dy = [0, 1, 2].map(|i| (x1 * (fc[i] - fa[i]) - x2 * (fb[i] - fa[i])) / det);
            best = (det.abs(), Gradients { dx, dy });
        }
        best.1
    }
}

/// Shade one pixel from a replacement's mip chain, trilinearly at the level
/// of detail of the pixel's footprint (as the GPU's `fs_packf`): None if
/// transparent.
#[allow(clippy::too_many_arguments)]
fn shade_filtered(
    t: &Texture,
    checker: bool,
    chain: &[Rgba],
    grad: &Gradients,
    light: &Light,
    src: &Sources,
    x: usize,
    y: usize,
    (iz, uz, vz): (f32, f32, f32),
) -> Option<[u8; 3]> {
    if checker && (x ^ y) & 1 != 0 {
        return None;
    }
    let (u, v) = (uz / iz, vz / iz);
    // Texels of level 0 per output pixel along x and y.
    let (sx, sy) = (chain[0].width as f32 / t.w as f32, chain[0].height as f32 / t.h as f32);
    let d = |g: &[f32; 3]| ((g[1] - u * g[0]) / iz * sx, (g[2] - v * g[0]) / iz * sy);
    let ((ax, ay), (bx, by)) = (d(&grad.dx), d(&grad.dy));
    let rho = (ax * ax + ay * ay).max(bx * bx + by * by).sqrt();
    let mode = |mirror| if mirror { Address::Mirror } else { Address::Repeat };
    let (rgb, alpha) =
        sample_trilinear(chain, u / t.w as f32, v / t.h as f32, rho.log2(), mode(t.mirror_x), mode(t.mirror_y));
    if alpha < 0.5 {
        return None;
    }
    Some(light.apply(t, src, u, v, rgb))
}

/// Draw polygons into an RGB layer with a coverage mask: windows in order,
/// and within a window from far (large key) to near.
pub fn draw(polys: &mut [Poly], src: &Sources) -> (Vec<u8>, Vec<bool>) {
    draw_view(polys, src, &View::native())
}

/// `draw` at the size of `view`. Viewports spanning the full original
/// width are widened to the view's edges, so whatever 3D the game sends
/// beyond the original frame is drawn.
pub fn draw_view(polys: &mut [Poly], src: &Sources, view: &View) -> (Vec<u8>, Vec<bool>) {
    draw_impl(polys, src, view, None, false, TexFilter::Off)
}

/// `draw_view` with the depth buffer on or off (see [`DEPTH_BIAS`]).
pub fn draw_view_depth(polys: &mut [Poly], src: &Sources, view: &View, depth: bool) -> (Vec<u8>, Vec<bool>) {
    draw_impl(polys, src, view, None, depth, TexFilter::Off)
}

/// `draw_view`, reporting textures to `hooks` and drawing the replacements
/// it returns (texture dumps and packs).
pub fn draw_enhanced(polys: &mut [Poly], src: &Sources, view: &View, hooks: &mut dyn Enhance) -> (Vec<u8>, Vec<bool>) {
    draw_impl(polys, src, view, Some(hooks), false, TexFilter::Off)
}

/// `draw_enhanced` with the depth buffer on or off.
pub fn draw_enhanced_depth(
    polys: &mut [Poly],
    src: &Sources,
    view: &View,
    hooks: &mut dyn Enhance,
    depth: bool,
) -> (Vec<u8>, Vec<bool>) {
    draw_impl(polys, src, view, Some(hooks), depth, TexFilter::Off)
}

/// `draw_enhanced_depth` with replacement images filtered as `filter` asks:
/// `Off` is `draw_enhanced_depth`; `Mip` and `Aniso` sample the images'
/// mip chains trilinearly at each pixel's level of detail (the software
/// renderer has no anisotropic filtering: `Aniso` draws as `Mip`).
pub fn draw_enhanced_filtered(
    polys: &mut [Poly],
    src: &Sources,
    view: &View,
    hooks: &mut dyn Enhance,
    depth: bool,
    filter: TexFilter,
) -> (Vec<u8>, Vec<bool>) {
    draw_impl(polys, src, view, Some(hooks), depth, filter)
}

/// A polygon whose 1/z ranges over more than this factor (a ground or
/// floor quad running from the camera to the horizon) keeps the game's
/// order with the depth buffer, like a background: the game sorts it by
/// its far edge so that it is drawn first, and where objects stand in or
/// slightly below it (topskatr's ramp in the sand) they are meant to
/// show over it.
pub const PAINTER_SPREAD: f32 = 8.0;

/// How much nearer than a group's first polygon (as a fraction of its
/// sort key) another can sort and still be of the group: 5%.
pub const GROUP_SPAN: f32 = 0.05;

/// Sort key from `build` for "behind everything" (the polygon header's
/// mode 3); keys at or above this are background.
const BACKGROUND_KEY: f32 = 1e9;

/// Sort `polys` into draw order (windows in order, and within a window from
/// far to near) and place each in `view`'s output pixels. Viewports
/// spanning the full original width are widened to the view's edges. HUD
/// polygons in `view.hud_windows` move as a unit with the HUD placement
/// (those in `hud_element_windows` have already been moved by
/// `HudLayout::anchor`).
pub fn place(polys: &mut [Poly], view: &View) -> Vec<Placed> {
    polys.sort_by(|a, b| a.window.cmp(&b.window).then(b.key.total_cmp(&a.key)));
    let mut window_start = 0;
    let mut group = 0;
    let mut group_key = 0.0;
    polys
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if i > 0 && polys[i - 1].window != p.window {
                window_start = i;
            }
            // Keys run far to near: a new group when the key is more than
            // GROUP_SPAN of the group's first below it.
            if i == 0 || polys[i - 1].window != p.window || group_key - p.key > GROUP_SPAN * group_key.abs() {
                group += 1;
                group_key = p.key;
            }
            let dx = if view.hud_windows & (1 << p.window) != 0 {
                let (lo, hi) = p.points.iter().fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(v.x), b.max(v.x)));
                view.hud_shift((lo + hi) / 2.0)
            } else {
                0.0
            };
            let points = p
                .points
                .iter()
                .map(|v| {
                    let (x, y) = view.to_output(v.x + dx, v.y);
                    SVtx { x, y, ..*v }
                })
                .collect();
            // view.to_output, homogeneous.
            let prev = p.prev.iter().map(|&[x, y, w]| [(x + (dx + view.pad) * w) * view.sx, y * view.sy, w]).collect();
            let (mut x0, y0, mut x1, y1) = p.clip;
            if view.pad > 0.0 && x0 <= 1.0 && x1 >= WIDTH as f32 - 1.0 {
                (x0, x1) = (view.x_min(), WIDTH as f32 + view.pad);
            }
            let (cx0, cy0) = view.to_output(x0, y0);
            let (cx1, cy1) = view.to_output(x1, y1);
            Placed {
                poly: i,
                points,
                clip: (cx0, cy0, cx1, cy1),
                rank: (i - window_start) as u32,
                group,
                background: background_reason(p).is_some(),
                prev,
            }
        })
        .collect()
}

/// Why a polygon is drawn in the game's order with the depth buffer on
/// ([`Placed::background`]), or None if it is depth tested.
pub fn background_reason(p: &Poly) -> Option<&'static str> {
    let (lo, hi) = p.points.iter().fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(v.iz), b.max(v.iz)));
    if p.key >= BACKGROUND_KEY {
        Some("sky")
    } else if p.shadow {
        Some("shadow")
    } else if p.checker {
        Some("checker")
    } else if hi > lo * PAINTER_SPREAD {
        Some("spread")
    } else {
        None
    }
}

/// Output pixels (x0, x1, y0, y1) where a window whose first polygon has
/// clip rectangle `clip` empties the depth buffer.
fn clear_rect(clip: (f32, f32, f32, f32), w: usize, h: usize) -> (usize, usize, usize, usize) {
    let (y0, y1) = (clip.1.floor().max(0.0) as usize, (clip.3.ceil().max(0.0) as usize).min(h));
    let (x0, x1) = (clip.0.floor().max(0.0) as usize, (clip.2.ceil().max(0.0) as usize).min(w));
    (x0.min(x1), x1, y0, y1)
}

/// One polygon covering a probed pixel ([`probe_depth`]).
#[derive(Clone, Debug)]
pub struct DepthHit {
    /// Index in draw order (the `placed` slice).
    pub index: usize,
    /// Why it is drawn untested ([`background_reason`]), if it is.
    pub background: Option<&'static str>,
    /// 1/z at the pixel centre, and its depth value ([`depth_value`]; it
    /// is tested with [`depth_test_value`]).
    pub iz: f32,
    pub depth: f32,
    /// The buffer it is tested against (for a group's members, the depth
    /// from before the group), and the hit that wrote it (None: empty).
    pub buffer: f32,
    pub buffer_from: Option<usize>,
    /// Passed the test (always, for a background polygon).
    pub passed: bool,
    /// Its colour there, None where it draws nothing (checkerboard parity
    /// or a cut-out texel).
    pub colour: Option<[u8; 3]>,
}

/// What the software rasteriser does at one output pixel with the depth
/// buffer on, and what it would do with it off.
#[derive(Clone, Debug, Default)]
pub struct DepthProbe {
    /// The polygons covering the pixel centre, in draw order, each once.
    pub hits: Vec<DepthHit>,
    /// Hits (indices into `hits`) whose colour shows with the depth buffer
    /// off and on.
    pub winner_off: Option<usize>,
    pub winner_on: Option<usize>,
}

/// Follow output pixel (x, y) of `view` through the depth buffer as
/// `draw_view_depth` draws `placed` (from [`place`]) with it on: the
/// coverage of the pixel centre, clip rectangles, window clears, groups,
/// the checkerboard and cut-out texels as the rasteriser has them (original
/// textures, not packs).
pub fn probe_depth(polys: &[Poly], placed: &[Placed], src: &Sources, view: &View, x: usize, y: usize) -> DepthProbe {
    let (w, h) = (view.width, view.height);
    let mut out = DepthProbe::default();
    let (mut zbuf, mut owner) = (0f32, None);
    let mut pending: Vec<(f32, usize)> = Vec::new();
    let (mut window, mut group) = (usize::MAX, u32::MAX);
    for (index, pl) in placed.iter().enumerate() {
        let p = &polys[pl.poly];
        if pl.group != group {
            group = pl.group;
            for (d, k) in pending.drain(..) {
                if d > zbuf {
                    (zbuf, owner) = (d, Some(k));
                }
            }
        }
        if p.window != window {
            window = p.window;
            let (x0, x1, y0, y1) = clear_rect(pl.clip, w, h);
            if (x0..x1).contains(&x) && (y0..y1).contains(&y) {
                (zbuf, owner) = (0.0, None);
            }
        }
        let (px, py) = (x as f32, y as f32);
        let clip = (pl.clip.0.max(px), pl.clip.1.max(py), pl.clip.2.min(px + 1.0), pl.clip.3.min(py + 1.0));
        let mut at = None;
        fill(&pl.points, clip, w, h, |_, _, iz, uz, vz| at = Some((iz, uz, vz)));
        let Some((iz, uz, vz)) = at else { continue };
        let background = background_reason(p);
        let depth = depth_value(iz, pl.rank);
        let passed = background.is_some() || depth_test_value(iz, pl.rank) >= zbuf;
        let (gx, gy) = ((px / view.sx) as usize, (py / view.sy) as usize);
        let colour = shade(p, src, gx, gy, uz / iz, vz / iz);
        let k = out.hits.len();
        if colour.is_some() {
            out.winner_off = Some(k);
            if passed {
                out.winner_on = Some(k);
                if background.is_none() {
                    pending.push((depth, k));
                }
            }
        }
        out.hits.push(DepthHit { index, background, iz, depth, buffer: zbuf, buffer_from: owner, passed, colour });
    }
    out
}

/// What [`draw_view`] shows at each output pixel ([`visible`]).
#[derive(Clone, Debug, Default)]
pub struct Visible {
    /// The polygon whose colour shows (index into `placed`), or `u32::MAX`.
    pub owner: Vec<u32>,
    /// Its 1/z at the pixel centre (0 where none).
    pub iz: Vec<f32>,
    /// Its previous position there ([`Placed::prev`] interpolated across
    /// the polygon, homogeneous output pixels `[X, Y, W]`), `W` 0 where it
    /// has none.
    pub prev: Vec<[f32; 3]>,
}

/// Per output pixel of `view`, the polygon [`draw_view`] shows (the depth
/// buffer off, original textures: the game's order, the checkerboard and
/// cut-out texels as drawn), its 1/z and its motion data: the motion
/// vector a temporal renderer would write there. `placed` is
/// [`place`]'s for `polys`.
pub fn visible(polys: &[Poly], placed: &[Placed], src: &Sources, view: &View) -> Visible {
    let (w, h) = (view.width, view.height);
    let mut out = Visible { owner: vec![u32::MAX; w * h], iz: vec![0.0; w * h], prev: vec![[0.0; 3]; w * h] };
    let (gsx, gsy) = (view.sx, view.sy);
    for (k, pl) in placed.iter().enumerate() {
        let p = &polys[pl.poly];
        let mut hits = Vec::new();
        fill(&pl.points, pl.clip, w, h, |x, y, iz, uz, vz| {
            let (gx, gy) = ((x as f32 / gsx) as usize, (y as f32 / gsy) as usize);
            hits.push((y * w + x, iz, shade(p, src, gx, gy, uz / iz, vz / iz).is_some()));
        });
        // The same fill visits the same pixels in the same order whatever
        // the attributes: carry [X, Y, W] as (u/z, v/z, 1/z).
        let mut prev = Vec::with_capacity(if pl.prev.is_empty() { 0 } else { hits.len() });
        if !pl.prev.is_empty() {
            let pts: Vec<SVtx> =
                pl.points.iter().zip(&pl.prev).map(|(v, q)| SVtx { iz: q[2], uz: q[0], vz: q[1], ..*v }).collect();
            fill(&pts, pl.clip, w, h, |_, _, ww, xx, yy| prev.push([xx, yy, ww]));
        }
        for (i, &(o, iz, shown)) in hits.iter().enumerate() {
            if shown {
                out.owner[o] = k as u32;
                out.iz[o] = iz;
                out.prev[o] = prev.get(i).copied().unwrap_or([0.0; 3]);
            }
        }
    }
    out
}

fn draw_impl(
    polys: &mut [Poly],
    src: &Sources,
    view: &View,
    mut hooks: Option<&mut dyn Enhance>,
    depth: bool,
    filter: TexFilter,
) -> (Vec<u8>, Vec<bool>) {
    let mut keys = TexKeys::default();
    let (w, h) = (view.width, view.height);
    let mut rgb = vec![0u8; w * h * 3];
    let mut covered = vec![false; w * h];
    let placed = place(polys, view);
    // Texel-space filtering works per output pixel; the checker pattern
    // stays in game pixels.
    let (gsx, gsy) = (view.sx, view.sy);
    let mut zbuf = vec![0f32; if depth { w * h } else { 0 }];
    let mut window = usize::MAX;
    // Depths a group of polygons writes, applied when it ends.
    let mut pending: Vec<(usize, f32)> = Vec::new();
    let mut group = u32::MAX;
    for pl in &placed {
        let p = &polys[pl.poly];
        let (points, clip) = (&pl.points[..], pl.clip);
        if pl.group != group {
            group = pl.group;
            for (o, d) in pending.drain(..) {
                zbuf[o] = zbuf[o].max(d);
            }
        }
        // Each window starts with an empty depth buffer where it draws.
        if depth && p.window != window {
            window = p.window;
            let (x0, x1, y0, y1) = clear_rect(clip, w, h);
            for y in y0..y1 {
                zbuf[y * w + x0..y * w + x1].fill(0.0);
            }
        }
        let test = depth && !pl.background;
        let rank = pl.rank;
        let hooked = match (hooks.as_deref_mut(), &p.texture) {
            (Some(hk), Some(t)) => tex_info(p, t, src, &mut keys).map(|info| {
                let img = if filter.mipmapped() {
                    hk.texture_mips(&info).map(Replacement::Mips)
                } else {
                    hk.texture(&info).map(Replacement::Image)
                };
                (info, img)
            }),
            _ => None,
        };
        // Material maps: asked for only for a polygon with a lamp and a
        // tangent frame (`light::Options::lamps`).
        let material = match (hooks.as_deref_mut(), &hooked, p.lamp.as_deref(), &p.texture) {
            (Some(hk), Some((info, _)), Some(l), Some(t)) if l.tangent.is_some() => {
                let chain = if filter.mipmapped() {
                    hk.texture_material_mips(info).map(MatLevels::Mips)
                } else {
                    hk.texture_material(info).map(MatLevels::One)
                };
                chain.map(|c| (l, t, c))
            }
            _ => None,
        };
        let mut pixels = 0;
        // Smooth shading: each pixel's luma, in the order `fill` visits
        // them (None: the polygon's own).
        let lumas = (!p.lit.is_empty()).then(|| lit_values(points, p, clip, w, h));
        // Material maps: each pixel's luma where the material changes it.
        let mat_lumas = material.as_ref().map(|(l, t, c)| {
            material_values(points, p, l, t, c.levels(), filter.mipmapped(), lumas.as_deref(), clip, w, h)
        });
        let mat_range = p.lamp.as_ref().map_or([0, 255], |l| l.range);
        let [lo, hi] = p.lit_range.map(u32::from);
        let own = PxLuma::own(p);
        let mut n = 0;
        let mut changed = 0;
        let mut luma_at = || {
            n += 1;
            if let Some(Some((v, rel))) = mat_lumas.as_ref().map(|m| m[n - 1]) {
                changed += 1;
                let [lo, hi] = mat_range.map(u32::from);
                return PxLuma { luma: crate::light::quantise(v).clamp(lo, hi), fine: Some((v, mat_range)), rel };
            }
            lumas.as_ref().map_or(own, |l| {
                let v = l[n - 1];
                PxLuma {
                    luma: crate::light::quantise(v).clamp(lo, hi),
                    fine: p.fine.then_some((v, p.lit_range)),
                    rel: None,
                }
            })
        };
        if let (Some((_, Some(Replacement::Mips(chain)))), Some(t)) = (&hooked, &p.texture) {
            let light0 = Light::new(p, own, src);
            let grad = Gradients::new(points);
            fill(points, clip, w, h, |x, y, iz, uz, vz| {
                let luma = luma_at();
                let o = y * w + x;
                if test && depth_test_value(iz, rank) < zbuf[o] {
                    return;
                }
                let lit;
                let light = if luma == own {
                    &light0
                } else {
                    lit = Light::new(p, luma, src);
                    &lit
                };
                let (gx, gy) = ((x as f32 / gsx) as usize, (y as f32 / gsy) as usize);
                if let Some(c) = shade_filtered(t, p.checker, chain, &grad, light, src, gx, gy, (iz, uz, vz)) {
                    if test {
                        pending.push((o, depth_value(iz, rank)));
                    }
                    covered[o] = true;
                    rgb[o * 3..o * 3 + 3].copy_from_slice(&c);
                    pixels += 1;
                }
            });
        } else if let (Some((_, Some(Replacement::Image(img)))), Some(t)) = (&hooked, &p.texture) {
            let light0 = Light::new(p, own, src);
            fill(points, clip, w, h, |x, y, iz, uz, vz| {
                let luma = luma_at();
                let o = y * w + x;
                if test && depth_test_value(iz, rank) < zbuf[o] {
                    return;
                }
                let lit;
                let light = if luma == own {
                    &light0
                } else {
                    lit = Light::new(p, luma, src);
                    &lit
                };
                let (gx, gy) = ((x as f32 / gsx) as usize, (y as f32 / gsy) as usize);
                if let Some(c) = shade_replaced(t, p.checker, img, light, src, gx, gy, uz / iz, vz / iz) {
                    if test {
                        pending.push((o, depth_value(iz, rank)));
                    }
                    covered[o] = true;
                    rgb[o * 3..o * 3 + 3].copy_from_slice(&c);
                    pixels += 1;
                }
            });
        } else {
            fill(points, clip, w, h, |x, y, iz, uz, vz| {
                let luma = luma_at();
                let o = y * w + x;
                if test && depth_test_value(iz, rank) < zbuf[o] {
                    return;
                }
                let (gx, gy) = ((x as f32 / gsx) as usize, (y as f32 / gsy) as usize);
                if let Some(c) = shade_luma(p, luma, src, gx, gy, uz / iz, vz / iz) {
                    if test {
                        pending.push((o, depth_value(iz, rank)));
                    }
                    covered[o] = true;
                    rgb[o * 3..o * 3 + 3].copy_from_slice(&c);
                    pixels += 1;
                }
            });
        }
        if let (Some(hk), Some((info, _))) = (hooks.as_deref_mut(), &hooked) {
            hk.texture_drawn(info, src, pixels);
            if material.is_some() {
                hk.material_drawn(info, changed);
            }
        }
    }
    (rgb, covered)
}

/// A smooth-shaded polygon's luma at each pixel `fill` visits, in that
/// order, before truncation: its corner lumas times 1/z, interpolated as
/// `fill` interpolates (perspective-correct), divided by the pixel's 1/z.
/// Per pixel ([`Poly::pixel`]): the corner normals likewise, normalised,
/// and the polygon's lighting for that normal (the interpolated luma where
/// the normals cancel out). Drawn truncated (`light::quantise`) and clamped
/// to [`Poly::lit_range`], or with finer levels as [`fine_level`] says.
fn lit_values(points: &[SVtx], p: &Poly, clip: (f32, f32, f32, f32), w: usize, h: usize) -> Vec<f32> {
    let lit = &p.lit;
    let pts: Vec<SVtx> = points.iter().zip(lit).map(|(v, l)| SVtx { uz: l[0] * v.iz, vz: l[1] * v.iz, ..*v }).collect();
    let mut out = Vec::new();
    match &p.pixel {
        None => fill(&pts, clip, w, h, |_, _, iz, lz, _| out.push(lz / iz)),
        Some(pl) => {
            // Two passes over the same pixels: (luma, nx), then (ny, nz).
            let mut first = Vec::new();
            fill(&pts, clip, w, h, |_, _, iz, lz, nxz| first.push((iz, lz, nxz)));
            let pts2: Vec<SVtx> =
                points.iter().zip(lit).map(|(v, l)| SVtx { uz: l[2] * v.iz, vz: l[3] * v.iz, ..*v }).collect();
            let mut k = 0;
            fill(&pts2, clip, w, h, |_, _, _, nyz, nzz| {
                let (iz, lz, nxz) = first[k];
                k += 1;
                out.push(pixel_luma(pl, lz / iz, [nxz / iz, nyz / iz, nzz / iz]));
            });
        }
    }
    out
}

/// A polygon's material as the filter wants it: level 0 alone (`tex_filter
/// off`) or its mip chain.
enum MatLevels {
    One(Arc<Material>),
    Mips(Arc<Vec<Material>>),
}

impl MatLevels {
    fn levels(&self) -> &[Material] {
        match self {
            MatLevels::One(m) => std::slice::from_ref(&**m),
            MatLevels::Mips(c) => c,
        }
    }
}

/// A material's normal at texture coordinates (u, v) (texels) of texture
/// `t`: (x, y) along increasing u and v, sampled bilinearly from `chain`
/// at level of detail `lod` (log2 of level 0's texels per pixel; trilinear
/// between the two levels around it when `lod` is above 0 and the chain has
/// them), with the polygon's addressing per axis; on a mirrored axis the
/// component changes sign in every second repeat, where the image runs
/// backwards.
pub fn material_normal(chain: &[Material], t: &Texture, u: f32, v: f32, lod: f32) -> (f32, f32) {
    let mode = |mirror| if mirror { Address::Mirror } else { Address::Repeat };
    let (s, tt) = (u / t.w as f32, v / t.h as f32);
    let top = chain.len().saturating_sub(1) as f32;
    let lod = if lod.is_nan() { 0.0 } else { lod.clamp(0.0, top) };
    let l0 = lod.floor() as usize;
    let f = lod - l0 as f32;
    let at =
        |m: &Material| m.sample_normal(s * m.width as f32, tt * m.height as f32, mode(t.mirror_x), mode(t.mirror_y));
    let (mut x, mut y) = at(&chain[l0]);
    if f > 0.0 && l0 + 1 < chain.len() {
        let (x1, y1) = at(&chain[l0 + 1]);
        x += (x1 - x) * f;
        y += (y1 - y) * f;
    }
    let odd = |c: f32| c.floor().rem_euclid(2.0) == 1.0;
    if t.mirror_x && odd(s) {
        x = -x;
    }
    if t.mirror_y && odd(tt) {
        y = -y;
    }
    (x, y)
}

/// A polygon's luma with its material at each pixel `fill` visits, in that
/// order: Some where the material changes it ([`material_luma`]; the base is
/// the smooth-shaded luma `lumas` gives, or the polygon's own + 0.5), None
/// where the pixel is drawn as without materials.
#[allow(clippy::too_many_arguments)]
fn material_values(
    points: &[SVtx],
    p: &Poly,
    l: &PolyLamp,
    t: &Texture,
    chain: &[Material],
    mipmapped: bool,
    lumas: Option<&[f32]>,
    clip: (f32, f32, f32, f32),
    w: usize,
    h: usize,
) -> Vec<Option<(f32, Option<f32>)>> {
    // Per pixel: 1/z, u/z, v/z.
    let mut px = Vec::new();
    fill(points, clip, w, h, |_, _, iz, uz, vz| px.push((iz, uz, vz)));
    // Smooth-shaded with normals (the exact layer): the normal interpolated
    // like the luma; else the face's.
    let lit_normals = p.lit.len() == points.len() && p.lit.iter().any(|c| c[1] != 0.0 || c[2] != 0.0 || c[3] != 0.0);
    let mut normals = Vec::new();
    if lit_normals {
        let pts: Vec<SVtx> =
            points.iter().zip(&p.lit).map(|(v, c)| SVtx { uz: c[1] * v.iz, vz: c[2] * v.iz, ..*v }).collect();
        fill(&pts, clip, w, h, |_, _, iz, a, b| normals.push([a / iz, b / iz, 0.0]));
        let pts: Vec<SVtx> = points.iter().zip(&p.lit).map(|(v, c)| SVtx { uz: c[3] * v.iz, ..*v }).collect();
        let mut k = 0;
        fill(&pts, clip, w, h, |_, _, iz, c, _| {
            normals[k][2] = c / iz;
            k += 1;
        });
    }
    let grad = mipmapped.then(|| Gradients::new(points));
    let (sx, sy) = (chain[0].width as f32 / t.w as f32, chain[0].height as f32 / t.h as f32);
    let own = p.luma as f32 + 0.5;
    let scale = chain[0].normal_scale;
    px.iter()
        .enumerate()
        .map(|(k, &(iz, uz, vz))| {
            let (u, v) = (uz / iz, vz / iz);
            let lod = grad.as_ref().map_or(0.0, |g| {
                let d = |g: &[f32; 3]| ((g[1] - u * g[0]) / iz * sx, (g[2] - v * g[0]) / iz * sy);
                let ((ax, ay), (bx, by)) = (d(&g.dx), d(&g.dy));
                (ax * ax + ay * ay).max(bx * bx + by * by).sqrt().log2()
            });
            let xy = material_normal(chain, t, u, v, lod);
            let n = if lit_normals { normals[k] } else { l.n };
            let base = lumas.and_then(|b| b.get(k).copied()).unwrap_or(own);
            material_luma(l, n, base, xy, scale)
        })
        .collect()
}

/// A placed smooth-shaded polygon's luma at each pixel it covers (as drawn,
/// before anything hides it, truncated and clamped as the game's levels
/// draw it; for tools). Empty when it isn't lit.
pub fn lit_pixel_lumas(p: &Poly, pl: &Placed, view: &View) -> Vec<u32> {
    if p.lit.is_empty() {
        return Vec::new();
    }
    let [lo, hi] = p.lit_range.map(u32::from);
    let v = lit_values(&pl.points, p, pl.clip, view.width, view.height);
    v.into_iter().map(|l| crate::light::quantise(l).clamp(lo, hi)).collect()
}

/// A per-pixel lit polygon's luma (before truncation) at a pixel whose
/// interpolated corner luma is `luma` and normal `n` (not unit): the
/// lighting for `n` normalised, or `luma` where `n` is (nearly) zero. The
/// GPU's `pixel_luma` (raster.wgsl) is the same.
pub fn pixel_luma(pl: &crate::light::PixelLight, luma: f32, n: [f32; 3]) -> f32 {
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len.is_nan() || len < 1e-6 {
        return luma;
    }
    pl.luma(n.map(|x| x / len))
}

/// Scanline fill of a convex polygon with pixel-centre sampling, limited to
/// `clip` (x0, y0, x1, y1) and the screen. `plot` receives (x, y, 1/z,
/// u/z, v/z) interpolated linearly in screen space.
fn fill(
    pts: &[SVtx],
    clip: (f32, f32, f32, f32),
    width: usize,
    height: usize,
    mut plot: impl FnMut(usize, usize, f32, f32, f32),
) {
    let (ymin, ymax) = pts.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.y), b.max(p.y)));
    let y0 = ymin.max(clip.1).ceil().max(0.0) as i64;
    let y1 = (ymax.min(clip.3).ceil() as i64).min(height as i64);
    for y in y0..y1 {
        let yc = y as f32 + 0.5;
        // Left and right span ends: (x, iz, uz, vz).
        let mut l = (f32::MAX, 0.0, 0.0, 0.0);
        let mut r = (f32::MIN, 0.0, 0.0, 0.0);
        for k in 0..pts.len() {
            let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
            if (a.y <= yc && b.y > yc) || (b.y <= yc && a.y > yc) {
                let t = (yc - a.y) / (b.y - a.y);
                let e = |p: f32, q: f32| p + (q - p) * t;
                let s = (e(a.x, b.x), e(a.iz, b.iz), e(a.uz, b.uz), e(a.vz, b.vz));
                if s.0 < l.0 {
                    l = s;
                }
                if s.0 > r.0 {
                    r = s;
                }
            }
        }
        if l.0 > r.0 {
            continue;
        }
        let x0 = (l.0.max(clip.0) - 0.5).ceil().max(0.0) as i64;
        let x1 = ((r.0.min(clip.2) - 0.5).ceil() as i64).min(width as i64);
        let span = (r.0 - l.0).max(1e-6);
        for x in x0..x1 {
            let t = ((x as f32 + 0.5) - l.0) / span;
            let e = |p: f32, q: f32| p + (q - p) * t;
            plot(x as usize, y as usize, e(l.1, r.1), e(l.2, r.2), e(l.3, r.3));
        }
    }
}

/// Where a pack standing for the original textures (`pack-check
/// --identity`) draws a frame differently from the original: pixels off by
/// more than 16 levels in a channel, by cause, for the software renderer
/// drawing unfiltered and without the depth buffer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct IdentitySplit {
    /// The bilinear filter's texels under the pixel hold different values:
    /// the original filters texel values and looks the result up, the
    /// pack filters colours, and the two differ wherever the look is not
    /// linear in the texel value (a palette-like look, such as crowds).
    pub texels_differ: usize,
    /// One of those texels is a cut-out (texel 15 of a translucent
    /// texture).
    pub cut_out: usize,
    /// One texel value under the pixel (or no texture): a pack standing
    /// for the texture must draw it as the original does, so these are
    /// gaps (a wrong look, lighting the replacement can't express).
    pub same_texel: usize,
    /// Another polygon drawn on top in one of the two (a cut-out decided
    /// differently at the edge).
    pub other_polygon: usize,
}

/// [`IdentitySplit`] of a frame's (unlit) polygons drawn with the
/// replacements `hooks` gives against without.
pub fn identity_split(polys: &mut [Poly], src: &Sources, view: &View, hooks: &mut dyn Enhance) -> IdentitySplit {
    let (w, h) = (view.width, view.height);
    let placed = place(polys, view);
    let mut keys = TexKeys::default();
    // Per pixel, the placed polygon on top, its (u, v) and colour.
    let none = (usize::MAX, 0.0f32, 0.0f32, [0u8; 3]);
    let (mut orig, mut pack) = (vec![none; w * h], vec![none; w * h]);
    let (gsx, gsy) = (view.sx, view.sy);
    for (k, pl) in placed.iter().enumerate() {
        let p = &polys[pl.poly];
        let own = PxLuma::own(p);
        let img = p.texture.as_ref().and_then(|t| tex_info(p, t, src, &mut keys)).and_then(|i| hooks.texture(&i));
        let light = Light::new(p, own, src);
        fill(&pl.points, pl.clip, w, h, |x, y, iz, uz, vz| {
            let (gx, gy) = ((x as f32 / gsx) as usize, (y as f32 / gsy) as usize);
            let (u, v) = (uz / iz, vz / iz);
            let o = y * w + x;
            let plain = shade_luma(p, own, src, gx, gy, u, v);
            if let Some(c) = plain {
                orig[o] = (k, u, v, c);
            }
            let c = match (&img, &p.texture) {
                (Some(img), Some(t)) => shade_replaced(t, p.checker, img, &light, src, gx, gy, u, v),
                _ => plain,
            };
            if let Some(c) = c {
                pack[o] = (k, u, v, c);
            }
        });
    }
    let mut split = IdentitySplit::default();
    for (a, b) in orig.iter().zip(&pack) {
        if (0..3).all(|c| a.3[c].abs_diff(b.3[c]) <= 16) {
            continue;
        }
        if a.0 != b.0 {
            split.other_polygon += 1;
            continue;
        }
        let Some(t) = polys[placed[a.0].poly].texture else {
            split.same_texel += 1;
            continue;
        };
        // The texels `filtered_texel` weighs.
        let (u, v) = (a.1 - 0.5, a.2 - 0.5);
        let (fu, fv) = (u - u.floor(), v - v.floor());
        let (u0, v0) = (u.floor() as i32, v.floor() as i32);
        let texels: Vec<u32> =
            [(0, 0, (1.0 - fu) * (1.0 - fv)), (1, 0, fu * (1.0 - fv)), (0, 1, (1.0 - fu) * fv), (1, 1, fu * fv)]
                .into_iter()
                .filter(|&(_, _, wt)| wt > 0.0)
                .map(|(du, dv, _)| {
                    src.texel(t.bank, t.x0 + wrap(u0 + du, t.w, t.mirror_x), t.y0 + wrap(v0 + dv, t.h, t.mirror_y))
                })
                .collect();
        if t.translucent && texels.contains(&15) {
            split.cut_out += 1;
        } else if texels.iter().all(|&x| x == texels[0]) {
            split.same_texel += 1;
        } else {
            split.texels_differ += 1;
        }
    }
    split
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sv(x: f32, y: f32) -> SVtx {
        SVtx { x, y, iz: 1.0, uz: 0.0, vz: 0.0 }
    }

    #[test]
    fn tex_keys_last_while_render_ram_is_unchanged() {
        let mut k = TexKeys::default();
        let held = |k: &TexKeys| !k.rects.is_empty();
        k.begin(Some(7));
        k.rects.insert([0; 5], 1);
        k.begin(Some(7));
        assert!(held(&k));
        k.begin(Some(8));
        assert!(!held(&k));
        k.rects.insert([0; 5], 1);
        k.clear();
        k.begin(Some(8));
        assert!(!held(&k), "clear forgets the version too");
        k.rects.insert([0; 5], 1);
        k.begin(None);
        assert!(!held(&k));
        k.rects.insert([0; 5], 1);
        k.begin(None);
        assert!(!held(&k), "an unknown version never keeps hashes");
    }

    #[test]
    fn fills_a_square() {
        let mut n = 0;
        let screen = (0.0, 0.0, WIDTH as f32, HEIGHT as f32);
        fill(
            &[sv(10.0, 10.0), sv(20.0, 10.0), sv(20.0, 20.0), sv(10.0, 20.0)],
            screen,
            WIDTH,
            HEIGHT,
            |_, _, _, _, _| n += 1,
        );
        assert_eq!(n, 100);
    }

    #[test]
    fn checker_applies_to_flat_polygons() {
        let (ram, palette) = (vec![0u8; 0x40_0000 + 0x8000], vec![0u8; 0x1_c000]);
        let src = Sources {
            texture_rom: &[],
            texture_point_ram: &[],
            render_ram: &ram,
            palette: &palette,
            xoff: 0.0,
            yoff: 0.0,
        };
        let p = Poly {
            points: Vec::new(),
            key: 0.0,
            window: 0,
            clip: (0.0, 0.0, 0.0, 0.0),
            texture: texture_from_header([0x80c0, 0, 0, 0]),
            colour: [0, 0, 6],
            luma: 255,
            checker: true,
            header: [0x80c0, 0, 0, 0],
            shadow: false,
            block: 0,
            at: 0,
            prev: Vec::new(),
            lit: Vec::new(),
            pixel: None,
            lit_range: [0, 255],
            fine: false,
            lamp: None,
        };
        assert!(p.texture.is_none());
        assert!(shade(&p, &src, 2, 4, 0.0, 0.0).is_some());
        assert!(shade(&p, &src, 3, 4, 0.0, 0.0).is_none());
    }

    /// A flat polygon covering the middle of the screen: colour index `c`
    /// (translated to c * 20), at 1/z `iz`, sort key `key`.
    fn flat(c: u32, iz: f32, key: f32, window: usize) -> Poly {
        let v = |x, y| SVtx { x, y, iz, uz: 0.0, vz: 0.0 };
        Poly {
            points: vec![v(100.0, 100.0), v(300.0, 100.0), v(300.0, 300.0), v(100.0, 300.0)],
            key,
            window,
            clip: (0.0, 0.0, WIDTH as f32, HEIGHT as f32),
            texture: None,
            colour: [c, c, c],
            luma: 255,
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

    fn depth_sources(palette: &mut [u8]) {
        for ch in 0..3 {
            for c in 1..4u32 {
                palette[0x1_0000 + ch * 0x4000 + (c as usize * 256 + 63) * 2] = (c * 20) as u8;
            }
        }
    }

    #[test]
    fn depth_buffer_resolves_what_the_game_sorted_wrong() {
        let ram = vec![0u8; 0x40_0000 + 0x8000];
        let mut palette = vec![0u8; 0x1_c000];
        depth_sources(&mut palette);
        let src = Sources {
            texture_rom: &[],
            texture_point_ram: &[],
            render_ram: &ram,
            palette: &palette,
            xoff: 0.0,
            yoff: 0.0,
        };
        let view = View::native();
        let at = |rgb: &[u8]| rgb[(200 * WIDTH + 200) * 3];
        // Sorted far to near by key: the nearer polygon (1/z 0.5, colour
        // 20) has the larger key, so the game draws it first and the one
        // behind it (1/z 0.1, colour 40) over it.
        let polys = || vec![flat(1, 0.5, 5.0, 0), flat(2, 0.1, 1.0, 0)];
        let off = draw_view_depth(&mut polys(), &src, &view, false);
        assert_eq!(at(&off.0), 40);
        let on = draw_view_depth(&mut polys(), &src, &view, true);
        assert_eq!(at(&on.0), 20);
        // The same depth, or within the rank tie-break: the game's order.
        let tie = || vec![flat(1, 0.5, 5.0, 0), flat(2, 0.5, 1.0, 0)];
        assert_eq!(at(&draw_view_depth(&mut tie(), &src, &view, true).0), 40);
        // Another window starts with an empty buffer: it draws over.
        let windows = || vec![flat(1, 0.5, 1.0, 0), flat(2, 0.1, 1.0, 1)];
        assert_eq!(at(&draw_view_depth(&mut windows(), &src, &view, true).0), 40);
        // A shadow or a background keeps the game's order too.
        let background = || vec![flat(1, 0.5, 5.0, 0), flat(2, 0.1, 1e10, 0)];
        assert_eq!(at(&draw_view_depth(&mut background(), &src, &view, true).0), 20);
        // Off is the painter's algorithm, whatever the depths.
        assert_eq!(at(&draw_view(&mut polys(), &src, &view).0), 40);
        // A decal a little behind its surface (within DEPTH_TOLERANCE) is
        // drawn in the game's order; one well behind it is hidden.
        let decal = |iz| vec![flat(1, 0.5, 5.0, 0), flat(2, iz, 1.0, 0)];
        assert_eq!(at(&draw_view_depth(&mut decal(0.5 * (1.0 - 0.6 * DEPTH_TOLERANCE)), &src, &view, true).0), 40);
        assert_eq!(at(&draw_view_depth(&mut decal(0.5 * (1.0 - 2.0 * DEPTH_TOLERANCE)), &src, &view, true).0), 20);
    }

    #[test]
    fn probe_follows_the_depth_buffer() {
        let ram = vec![0u8; 0x40_0000 + 0x8000];
        let mut palette = vec![0u8; 0x1_c000];
        depth_sources(&mut palette);
        let src = Sources {
            texture_rom: &[],
            texture_point_ram: &[],
            render_ram: &ram,
            palette: &palette,
            xoff: 0.0,
            yoff: 0.0,
        };
        let view = View::native();
        let mut polys = vec![flat(1, 0.5, 5.0, 0), flat(2, 0.1, 1.0, 0), flat(3, 0.1, 1e10, 0)];
        let placed = place(&mut polys, &view);
        let r = probe_depth(&polys, &placed, &src, &view, 200, 200);
        assert_eq!(r.hits.len(), 3);
        // Sorted: the sky (colour 3) first, then 1, then 2, which fails.
        assert_eq!(r.hits[0].background, Some("sky"));
        assert!(r.hits[1].passed && !r.hits[2].passed);
        assert_eq!(r.hits[2].buffer_from, Some(1));
        assert_eq!((r.winner_off, r.winner_on), (Some(2), Some(1)));
        // Outside every polygon: nothing.
        assert!(probe_depth(&polys, &placed, &src, &view, 10, 10).hits.is_empty());
    }

    #[test]
    fn depth_value_grows_toward_the_viewer() {
        let mut last = 0.0;
        for k in 0..60 {
            let d = depth_value(1e-6 * 1.5f32.powi(k), 0);
            assert!(d > last && d < 1.0, "{k}: {d}");
            last = d;
        }
        // A later polygon of the same depth wins.
        assert!(depth_value(0.1, 5) > depth_value(0.1, 4));
    }

    #[test]
    fn groups_gather_polygons_sorted_together() {
        let mut polys = vec![flat(1, 0.5, 10.0, 0), flat(2, 0.5, 9.8, 0), flat(3, 0.5, 5.0, 0), flat(1, 0.5, 5.0, 1)];
        let placed = place(&mut polys, &View::native());
        assert_eq!(placed.iter().map(|p| p.group - placed[0].group).collect::<Vec<_>>(), [0, 0, 1, 2]);
        assert_eq!(placed.iter().map(|p| p.rank).collect::<Vec<_>>(), [0, 1, 2, 0]);
    }

    #[test]
    fn close_geometry_is_clipped_not_dropped() {
        let v = |x, y, z| Vtx { x, y, z, u: x, ..Default::default() };
        // A triangle in front of the eye, nearer than one unit: kept whole.
        let c = clip_view(&[v(-0.1, 0.0, 0.5), v(0.1, 0.0, 0.5), v(0.0, 0.1, 0.4)], 248.0, 192.0);
        assert_eq!(c.len(), 3);
        // One crossing the eye plane keeps its front part, inside the guard band.
        let c = clip_view(&[v(0.0, 0.0, -1.0), v(-0.1, 0.0, 0.5), v(0.1, 0.0, 0.5)], 248.0, 192.0);
        assert!(c.len() >= 3);
        for p in &c {
            // Near NEAR, give or take rounding at the plane: 1/z stays finite.
            assert!(p.z > NEAR * 0.5);
            let sx = 248.0 + p.x / p.z;
            assert!((-GUARD - 1.0..=WIDTH as f32 + GUARD + 1.0).contains(&sx));
        }
        // Wholly behind the eye: nothing.
        assert!(clip_view(&[v(0.0, 0.0, -1.0), v(1.0, 0.0, -1.0), v(0.0, 1.0, -2.0)], 248.0, 192.0).len() < 3);
    }

    #[test]
    fn bilinear_filter_and_translucent_coverage() {
        // Texels (0,0) = 0 and (1,0) = 15 share the sheet's first word.
        let mut render_ram = vec![0u8; 0x40_0000];
        render_ram[1] = 0x0f;
        let src = Sources {
            texture_rom: &[],
            texture_point_ram: &[],
            render_ram: &render_ram,
            palette: &[],
            xoff: 0.0,
            yoff: 0.0,
        };
        let mut t = texture_from_header([0x4000, 0, 0, 0]).unwrap();
        // Halfway between the two texel centres: the average, 7.5 * 17.
        assert_eq!(filtered_texel(&t, &src, 1.0, 0.5), Some(128));
        assert_eq!(filtered_texel(&t, &src, 0.5, 0.5), Some(0));
        t.translucent = true;
        // 15 is transparent: mostly-opaque pixels take the opaque texel,
        // mostly-transparent ones are dropped.
        assert_eq!(filtered_texel(&t, &src, 0.9, 0.5), Some(0));
        assert_eq!(filtered_texel(&t, &src, 1.1, 0.5), None);
    }

    /// A pack of one fixed image for every texture.
    struct OneImage(Arc<Rgba>);

    impl Enhance for OneImage {
        fn texture(&mut self, _: &TexInfo) -> Option<Arc<Rgba>> {
            Some(self.0.clone())
        }
        fn texture_drawn(&mut self, _: &TexInfo, _: &Sources, _: usize) {}
        fn tile(&mut self, _: &crate::enhance::TileInfo) -> Option<Arc<Rgba>> {
            None
        }
    }

    #[test]
    fn identity_pack_where_full_luma_is_black_in_a_channel() {
        // Texel 0 (all of render RAM) has luma table value 64: level 63 at
        // full polygon luma, 32 at luma 128. Colour 1's green is black at
        // level 63 but not at 32 (a translation table that is not
        // monotonic, like topskatr's red over brown); red scales.
        let mut ram = vec![0u8; 0x40_0000 + 0x8000];
        ram[0x40_0000] = 64;
        let mut palette = vec![0u8; 0x1_c000];
        let tr = |ch: usize, lum: usize| 0x1_0000 + ch * 0x4000 + (256 + lum) * 2;
        palette[tr(0, 63)] = 200;
        palette[tr(0, 32)] = 100;
        palette[tr(1, 32)] = 50;
        let src = Sources {
            texture_rom: &[],
            texture_point_ram: &[],
            render_ram: &ram,
            palette: &palette,
            xoff: 0.0,
            yoff: 0.0,
        };
        let t = Texture {
            x0: 0,
            y0: 0,
            w: 32,
            h: 32,
            mirror_x: false,
            mirror_y: false,
            bank: 0,
            translucent: false,
            luma_table: 0,
        };
        let mut p = flat(1, 1.0, 1.0, 0);
        p.texture = Some(t);
        p.luma = 128;
        let info = tex_info(&p, &t, &src, &mut TexKeys::default()).unwrap();
        let image = Arc::new(texture_image(&src, &info));
        assert_eq!(&image.data[..4], &[200, 0, 0, 255]);
        let view = View::native();
        let at = |rgb: &[u8]| rgb[(200 * WIDTH + 200) * 3..][..3].to_vec();
        let plain = draw_view(&mut [p.clone()], &src, &view).0;
        assert_eq!(at(&plain), [100, 50, 0]);
        // The replacement scales red by 100 / 200 and adds green's own
        // value: the original's colour.
        let packed = draw_enhanced(&mut [p.clone()], &src, &view, &mut OneImage(image.clone())).0;
        assert_eq!(at(&packed), [100, 50, 0]);
        assert_eq!(identity_split(&mut [p.clone()], &src, &view, &mut OneImage(image)), IdentitySplit::default());
        // An image in another colour is a gap at every pixel of one texel
        // value.
        let wrong = Rgba { width: 1, height: 1, data: vec![0, 0, 200, 255] };
        let s = identity_split(&mut [p], &src, &view, &mut OneImage(Arc::new(wrong)));
        assert_eq!((s.texels_differ, s.cut_out, s.other_polygon), (0, 0, 0));
        assert_eq!(s.same_texel, 200 * 200);
    }

    /// The surface key: texels and size only, the same in every colour,
    /// luma table and translucency; the image key unchanged by it (both are
    /// pack file names, so neither may ever change for keys 1).
    #[test]
    fn surface_key_is_colour_independent_and_stable() {
        let mut ram = vec![0u8; 0x40_0000 + 0x8000];
        for (i, b) in ram[..0x2000].iter_mut().enumerate() {
            *b = (i * 7 % 251) as u8;
        }
        let palette = vec![0u8; 0x1_c000];
        let src = Sources {
            texture_rom: &[],
            texture_point_ram: &[],
            render_ram: &ram,
            palette: &palette,
            xoff: 0.0,
            yoff: 0.0,
        };
        let t = Texture {
            x0: 0,
            y0: 0,
            w: 32,
            h: 16,
            mirror_x: false,
            mirror_y: false,
            bank: 0,
            translucent: false,
            luma_table: 0,
        };
        let info = |p: &Poly, t: &Texture| tex_info(p, t, &src, &mut TexKeys::default()).unwrap();
        let base = info(&flat(1, 1.0, 1.0, 0), &t);
        // The documented definitions, spelled out.
        let mut f = Fnv::default();
        for y in 0..16 {
            for x in 0..32 {
                f.byte(src.texel(0, x, y) as u8);
            }
        }
        let texels = f.0;
        let mut s = Fnv::default();
        s.bytes(&texels.to_le_bytes());
        s.bytes(&32u32.to_le_bytes());
        s.bytes(&16u32.to_le_bytes());
        assert_eq!(base.surface, s.0);
        let colour = (1 | 1 << 5 | 1 << 10) as u32;
        for v in [0, 0, colour] {
            s.bytes(&v.to_le_bytes());
        }
        assert_eq!(base.key, s.0);
        // Pinned: a change here renames every pack file.
        assert_eq!((base.key, base.surface), (0x60c2_e035_3d1e_03b8, 0x4c59_2423_467f_3b65));
        assert_eq!(base.surface_name(), "surf_4c592423467f3b65_32x16");
        // Another colour, luma table, translucency or mirroring: another
        // image, the same surface.
        let mut lit = t;
        lit.luma_table = 3;
        lit.translucent = true;
        lit.mirror_x = true;
        for (p, t) in [(flat(9, 1.0, 1.0, 0), t), (flat(1, 1.0, 1.0, 0), lit)] {
            let i = info(&p, &t);
            assert_eq!(i.surface, base.surface);
            assert_ne!(i.key, base.key);
        }
        // Other texels or another size: another surface.
        for t in [Texture { x0: 32, ..t }, Texture { h: 32, ..t }] {
            assert_ne!(info(&flat(1, 1.0, 1.0, 0), &t).surface, base.surface);
        }
    }

    #[test]
    fn wrap_repeat_and_mirror() {
        assert_eq!(wrap(-1, 32, false), 31);
        assert_eq!(wrap(32, 32, true), 31);
        assert_eq!(wrap(-1, 32, true), 0);
    }

    /// A raster stream: one window (centre 0 at (cx, cy) in window
    /// coordinates, Y up) and an object block of double-sided quads given
    /// as view-space points (the first two are the block's P0 and P1, then
    /// two per quad), the positions through `f`.
    fn motion_stream(cx: i32, cy: i32, quads: &[[[f32; 3]; 4]], f: impl Fn([f32; 3]) -> [f32; 3]) -> Vec<u32> {
        let xy = |x: i32, y: i32| ((x as u32 & 0xfff) << 12) | (y as u32 & 0xfff);
        let mut s = vec![0xff00_0003, xy(0, 0), xy(495, 383), xy(cx, cy), xy(cx, cy), xy(cx, cy), xy(cx, cy)];
        let put = |s: &mut Vec<u32>, p: [f32; 3]| s.extend(f(p).map(|v| v.to_bits() >> 8));
        for q in quads {
            s.extend([0xff00_0001, 0, 0]);
            // P0 and P1 are the quad's second and first points.
            put(&mut s, q[1]);
            put(&mut s, q[0]);
            s.extend([1 | 1 << 8 | 1 << 10 | 1 << 17, 0, 0]);
            put(&mut s, q[2]);
            put(&mut s, q[3]);
        }
        s.push(0xff00_000f);
        s
    }

    fn motion_sources(palette: &[u8]) -> Sources<'_> {
        Sources { texture_rom: &[], texture_point_ram: &[], render_ram: &[], palette, xoff: 0.0, yoff: 0.0 }
    }

    /// An affine map of view space (positions are times the focal
    /// lengths, so x and y are large beside z): a turn about x, a stretch
    /// across and a move.
    fn moved(p: [f32; 3]) -> [f32; 3] {
        let (c, s) = (0.01f32.cos(), 0.01f32.sin());
        [1.02 * p[0] + 0.7, c * p[1] - s * p[2] - 0.4, s * p[1] + c * p[2] + 6.0]
    }

    /// Quads: one in front of the eye, one crossing the near plane, one
    /// running far past the side of the screen (clipped by the guard
    /// band).
    const QUADS: [[[f32; 3]; 4]; 3] = [
        [[-300.0, -200.0, 10.0], [300.0, -200.0, 10.0], [300.0, 200.0, 12.0], [-300.0, 200.0, 12.0]],
        [[-50.0, -30.0, -4.0], [50.0, -30.0, -4.0], [50.0, 30.0, 9.0], [-50.0, 30.0, 9.0]],
        [[-100.0, -50.0, 5.0], [90000.0, -50.0, 5.0], [90000.0, 50.0, 5.0], [-100.0, 50.0, 5.0]],
    ];

    fn plain(p: &Poly) -> String {
        format!("{:?}", Poly { prev: Vec::new(), ..p.clone() })
    }

    /// The previous position of a polygon's points is the projection of
    /// the moved view-space point, through clipping, with the previous
    /// frame's window centre, and stays exact between points when
    /// interpolated linearly in screen space and divided.
    #[test]
    fn motion_gives_the_exact_previous_projection() {
        let palette = vec![0u8; 0x1_c000];
        let src = motion_sources(&palette);
        let s1 = motion_stream(248, 192, &QUADS, |p| p);
        let s0 = motion_stream(248, 192, &QUADS, moved);
        let prev_centres = [[(251.0, 190.0); 4]];
        let m = MotionIn { s0: &s0, s1: &s1, tp: 0.0, valid: &[true; 3], prev_centres: &prev_centres };
        let (polys, centres) = build_motion(&s1, &src, Some(&m));
        assert_eq!(centres, vec![[(248.0, 192.0); 4]]);
        // Exactly build's polygons, apart from prev.
        let plain_polys: Vec<String> = build(&s1, &src).iter().map(plain).collect();
        assert_eq!(polys.iter().map(plain).collect::<Vec<_>>(), plain_polys);
        assert_eq!(polys.len(), 3);
        assert!(polys[1].points.len() > 4 || polys[1].points.iter().any(|p| p.iz > 1.0), "clipped at the eye");
        assert!(polys[2].points.iter().all(|p| p.x < 3000.0), "clipped by the guard band");
        // The view-space point a screen point shows (current centre).
        let (cx, cy) = (248.0, 192.0);
        let view = |x: f32, y: f32, iz: f32| [(x - cx) / iz, (cy - y) / iz, 1.0 / iz];
        let project = |p: [f32; 3]| (251.0 + p[0] / p[2], 190.0 - p[1] / p[2]);
        let close = |a: (f32, f32), b: (f32, f32), what: &str| {
            let tol = 2e-3 * (1.0 + b.0.abs().max(b.1.abs()) / 100.0);
            assert!((a.0 - b.0).abs() < tol && (a.1 - b.1).abs() < tol, "{what}: {a:?} vs {b:?}");
        };
        for (n, p) in polys.iter().enumerate() {
            assert_eq!(p.prev.len(), p.points.len());
            for (k, (v, h)) in p.points.iter().zip(&p.prev).enumerate() {
                close((h[0] / h[2], h[1] / h[2]), project(moved(view(v.x, v.y, v.iz))), &format!("{n}.{k}"));
                // Previous 1/z.
                let want = 1.0 / moved(view(v.x, v.y, v.iz))[2];
                assert!((v.iz / h[2] - want).abs() < 1e-4 * want, "{n}.{k}");
            }
            // Between two points, a third of the way across the screen.
            let (a, b) = (0, p.points.len() / 2);
            let s = 1.0 / 3.0;
            let l = |x: f32, y: f32| x + (y - x) * s;
            let (va, vb) = (p.points[a], p.points[b]);
            let (sx, sy, iz) = (l(va.x, vb.x), l(va.y, vb.y), l(va.iz, vb.iz));
            let h: [f32; 3] = std::array::from_fn(|c| l(p.prev[a][c], p.prev[b][c]));
            close((h[0] / h[2], h[1] / h[2]), project(moved(view(sx, sy, iz))), &format!("{n} between"));
        }
    }

    #[test]
    fn visible_matches_the_drawing_and_carries_motion() {
        let palette = vec![0u8; 0x1_c000];
        let src = motion_sources(&palette);
        let s1 = motion_stream(248, 192, &QUADS, |p| p);
        let s0 = motion_stream(248, 192, &QUADS, moved);
        let m = MotionIn { s0: &s0, s1: &s1, tp: 0.0, valid: &[], prev_centres: &[] };
        let view = View::new(240, 16.0 / 9.0, crate::video::Hud::Center);
        let (mut polys, _) = build_motion(&s1, &src, Some(&m));
        let placed = place(&mut polys, &view);
        let vis = visible(&polys, &placed, &src, &view);
        let (_, covered) = draw_view(&mut polys.clone(), &src, &view);
        let shown: Vec<bool> = vis.owner.iter().map(|&o| o != u32::MAX).collect();
        assert_eq!(shown, covered);
        assert!(covered.iter().any(|&c| c));
        // Pixels of a polygon with motion data have a previous position
        // inside the hull of its points' previous positions; others none.
        for (o, _) in shown.iter().enumerate().filter(|(_, s)| **s) {
            let [x, y, w] = vis.prev[o];
            let pl = &placed[vis.owner[o] as usize];
            if pl.prev.is_empty() {
                assert_eq!(w, 0.0);
                continue;
            }
            let (px, py) = (x / w, y / w);
            let lo = |k: usize| pl.prev.iter().map(|q| q[k] / q[2]).fold(f32::MAX, f32::min) - 1e-2;
            let hi = |k: usize| pl.prev.iter().map(|q| q[k] / q[2]).fold(f32::MIN, f32::max) + 1e-2;
            assert!(w > 0.0 && (lo(0)..=hi(0)).contains(&px) && (lo(1)..=hi(1)).contains(&py), "{px} {py}");
        }
        // Without motion: no previous positions.
        let mut plain = build(&s1, &src);
        let placed = place(&mut plain, &view);
        assert!(visible(&plain, &placed, &src, &view).prev.iter().all(|p| p[2] == 0.0));
    }

    #[test]
    fn motion_identity_and_validity() {
        let palette = vec![0u8; 0x1_c000];
        let src = motion_sources(&palette);
        let s1 = motion_stream(248, 192, &QUADS, |p| p);
        // The same pose twice: the previous position is the current one.
        let m = MotionIn { s0: &s1, s1: &s1, tp: 0.0, valid: &[], prev_centres: &[] };
        let (polys, _) = build_motion(&s1, &src, Some(&m));
        for p in &polys {
            for (v, h) in p.points.iter().zip(&p.prev) {
                assert!((h[0] / h[2] - v.x).abs() < 1e-3 && (h[1] / h[2] - v.y).abs() < 1e-3, "{v:?} {h:?}");
                assert!((h[2] - 1.0).abs() < 1e-6);
            }
            assert!(!p.prev.is_empty());
        }
        // tp blends: halfway between the poses.
        let s0 = motion_stream(248, 192, &QUADS, moved);
        let half = motion_stream(248, 192, &QUADS, |p| {
            let q = moved(p);
            std::array::from_fn(|c| (p[c] + q[c]) / 2.0)
        });
        let at = |s0: &[u32], tp: f32| {
            let m = MotionIn { s0, s1: &s1, tp, valid: &[], prev_centres: &[] };
            build_motion(&s1, &src, Some(&m)).0
        };
        let (a, b) = (at(&s0, 0.5), at(&half, 0.0));
        for (p, q) in a.iter().zip(&b) {
            for (h, g) in p.prev.iter().zip(&q.prev) {
                assert!((h[0] / h[2] - g[0] / g[2]).abs() < 2e-2, "{h:?} {g:?}");
            }
        }
        let with = |valid: &[bool], s0: &[u32]| {
            let m = MotionIn { s0, s1: &s1, tp: 0.0, valid, prev_centres: &[] };
            build_motion(&s1, &src, Some(&m)).0.iter().map(|p| !p.prev.is_empty()).collect::<Vec<_>>()
        };
        // Blocks whose motion isn't known have none.
        assert_eq!(with(&[true, false, true], &s0), [true, false, true]);
        // Validity of the wrong length is ignored.
        assert_eq!(with(&[false], &s0), [true; 3]);
        // A previous position behind the eye: none for that polygon.
        let behind = motion_stream(248, 192, &QUADS, |p| if p[1] > 0.0 { [p[0], p[1], -p[2]] } else { p });
        assert_eq!(with(&[], &behind), [false; 3]);
        // Streams that don't line up: no motion at all.
        let mut other = s0.clone();
        other[0] = 0xff00_0008;
        assert_eq!(with(&[], &other), [false; 3]);
        assert_eq!(with(&[], &s0[1..]), [false; 3]);
        // Without motion data, nothing.
        assert!(build_motion(&s1, &src, None).0.iter().all(|p| p.prev.is_empty()));
    }

    /// Smooth shading's corner values through `build_with`: crease 0 gives
    /// `build`'s polygons exactly; lit polygons carry their corners per
    /// point, and a point made by clipping (here at the near plane) has
    /// the value interpolated along its edge (linear in view space, as the
    /// position); shadows get none; drawn with each corner at the
    /// polygon's own luma (+0.5) the picture is the flat one.
    #[test]
    fn corner_values_through_build_and_clipping() {
        use crate::light::{self, test_stream};
        let f = 495.0;
        let sp = |p: [f32; 3]| [p[0] * f, p[1] * f, p[2]];
        // A strip bent by 20 degrees about x = 0.5; the second quad runs
        // from z 4 to behind the eye.
        let bend = 20f32.to_radians();
        let a = [[-0.5, -0.3, 4.0], [0.5, -0.3, 4.0], [0.5, 0.3, 4.0], [-0.5, 0.3, 4.0]];
        let (c, s) = (bend.cos(), bend.sin());
        let b = [
            [0.5, -0.3, 4.0],
            [0.5 + 15.0 * c, -0.3, 4.0 - 15.0 * s],
            [0.5 + 15.0 * c, 0.3, 4.0 - 15.0 * s],
            [0.5, 0.3, 4.0],
        ];
        let (s1, _) =
            test_stream::stream(&[vec![test_stream::tpoly(&a.map(sp), 100), test_stream::tpoly(&b.map(sp), 140)]]);
        let mut palette = vec![0u8; 0x1_c000];
        palette[0x1_0000..].fill(1);
        for ch in 0..3 {
            for lum in 0..64 {
                palette[0x1_0000 + ch * 0x4000 + lum * 2] = (lum * 4 + 1) as u8;
            }
        }
        let src = motion_sources(&palette);
        let opts = |crease| light::Options { shading: light::Shading::Vertex, crease, ..Default::default() };
        let plain = build(&s1, &src);
        let at = |crease| {
            let sh = light::shade(&s1, &opts(crease));
            let (p, _) = build_with(&s1, &src, &BuildIn { lit: Some(&sh), ..Default::default() });
            (p, sh)
        };
        let (zero, sh0) = at(0);
        assert!(sh0.at.is_empty());
        assert_eq!(format!("{zero:?}"), format!("{plain:?}"));
        let (lit, sh) = at(40);
        assert_eq!(sh.at.len(), 2);
        assert_eq!(lit.len(), 2);
        // The view-space points of the polygons' corners and values.
        let view = |v: &SVtx| [(v.x - 248.0) / v.iz / f, (192.0 - v.y) / v.iz / f, 1.0 / v.iz];
        let corner = |x: [f32; 3]| b.iter().any(|c| (0..3).all(|k| (c[k] - x[k]).abs() < 1e-3));
        assert!(lit[1].points.iter().any(|v| !corner(view(v))), "clipped");
        for (p, (q, corners)) in lit.iter().zip([(a, sh.corners[0]), (b, sh.corners[1])]) {
            assert_eq!(p.lit.len(), p.points.len());
            assert_eq!(
                format!("{:?}", Poly { lit: Vec::new(), ..p.clone() }),
                format!("{:?}", plain[lit.iter().position(|x| std::ptr::eq(x, p)).unwrap()])
            );
            for (v, l) in p.points.iter().zip(&p.lit) {
                let x = view(v);
                // On which edge of the original quad, and where.
                let (k, t) = (0..4)
                    .map(|k| {
                        let (e0, e1) = (q[k], q[(k + 1) % 4]);
                        let d = [0, 1, 2].map(|c| e1[c] - e0[c]);
                        let t = (0..3).map(|c| (x[c] - e0[c]) * d[c]).sum::<f32>()
                            / (0..3).map(|c| d[c] * d[c]).sum::<f32>();
                        let off = (0..3).map(|c| (e0[c] + d[c] * t - x[c]).powi(2)).sum::<f32>();
                        (off, k, t)
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .map(|(_, k, t)| (k, t.clamp(0.0, 1.0)))
                    .unwrap();
                let want = corners[k][0] + (corners[(k + 1) % 4][0] - corners[k][0]) * t;
                assert!((l[0] - want).abs() < 1e-2, "{l:?} vs {want} on edge {k} at {t}");
            }
        }
        // The corners at the shared edge are the mean, the others their own.
        assert!((sh.corners[0][1][0] - 120.5).abs() < 1e-3 && sh.corners[0][0][0] == 100.5);
        // Colour tables that turn the colour red from level 30 up (a
        // game's palette by level, topskatr's boards): interpolating the
        // luma would change the hue, so each polygon's lumas are clamped
        // to the levels of its own hue. A ramp of one hue (any curve)
        // leaves them all.
        let mut red = palette.clone();
        for ch in 1..3 {
            for lum in 30..64 {
                red[0x1_0000 + ch * 0x4000 + lum * 2] = 0;
            }
        }
        let sh = light::shade(&s1, &opts(40));
        let (p, _) = build_with(&s1, &motion_sources(&red), &BuildIn { lit: Some(&sh), ..Default::default() });
        // The first quad (lumas 100 to 120, levels 25 to 30) would cross
        // into red: kept below level 30; the second (120 to 140) is red
        // throughout.
        assert!(!p[0].lit.is_empty() && !p[1].lit.is_empty());
        assert_eq!((p[0].lit_range, p[1].lit_range), ([0, 119], [120, 255]));
        let v = View::native();
        let (flat_red, _) = draw_view(&mut plain.clone(), &motion_sources(&red), &v);
        let (lit_red, _) = draw_view(&mut p.clone(), &motion_sources(&red), &v);
        assert!(flat_red != lit_red, "still shaded");
        assert!(
            flat_red.chunks(3).zip(lit_red.chunks(3)).all(|(a, b)| same_hue([a[0], a[1], a[2]], [b[0], b[1], b[2]])),
            "no pixel changes hue"
        );
        let mut curve = palette.clone();
        for ch in 0..3 {
            for lum in 0..64 {
                curve[0x1_0000 + ch * 0x4000 + lum * 2] = ((lum * lum) / 16 + 1) as u8;
            }
        }
        let (p, _) = build_with(&s1, &motion_sources(&curve), &BuildIn { lit: Some(&sh), ..Default::default() });
        assert!(p.iter().all(|p| !p.lit.is_empty() && p.lit_range == [0, 255]), "a curve of one hue: lit");
        assert!(same_hue([255, 255, 255], [120, 121, 119]) && !same_hue([255, 255, 255], [255, 0, 0]));
        assert!(same_hue([10, 0, 0], [0, 20, 0]), "near black has no hue");
        // Shadows (flat black at their luma) get none.
        let black = vec![0u8; 0x1_c000];
        let sh = light::shade(&s1, &opts(40));
        let (dark, _) = build_with(&s1, &motion_sources(&black), &BuildIn { lit: Some(&sh), ..Default::default() });
        assert!(dark.iter().all(|p| p.shadow && p.lit.is_empty()));
        // Every corner at the polygon's own luma: the flat picture.
        let own: Vec<Poly> = plain
            .iter()
            .map(|p| Poly { lit: vec![[p.luma as f32 + 0.5, 0.0, 0.0, 0.0]; p.points.len()], ..p.clone() })
            .collect();
        assert_eq!(draw_view(&mut own.clone(), &src, &v), draw_view(&mut plain.clone(), &src, &v));
        // And with the bend lit, the picture changes.
        assert!(draw_view(&mut lit.clone(), &src, &v) != draw_view(&mut plain.clone(), &src, &v));
    }

    /// Per-pixel lighting (`Poly::pixel`): with every corner's normal the
    /// face's own, the flat picture exactly; with normals turning across
    /// the quad, a highlight between the corners that no interpolation of
    /// their lumas gives; truncation as `light::quantise`; the hue clamp.
    #[test]
    fn per_pixel_lighting_and_rounding() {
        use crate::light::{test_stream, Lamp, PixelLight};
        let q = [[-0.4f32, -0.3, 2.0], [0.4, -0.3, 2.0], [0.4, 0.3, 2.0], [-0.4, 0.3, 2.0]];
        let f = 495.0;
        let (s, _) = test_stream::stream(&[vec![test_stream::tpoly(&q.map(|p| [p[0] * f, p[1] * f, p[2]]), 100)]]);
        let mut palette = vec![0u8; 0x1_c000];
        for ch in 0..3 {
            for lum in 0..64 {
                palette[0x1_0000 + ch * 0x4000 + lum * 2] = (lum * 4) as u8;
            }
        }
        let src = motion_sources(&palette);
        let plain = build(&s, &src);
        assert_eq!(plain.len(), 1);
        // Light along the view axis, a material with a sharp highlight.
        let lamp = Lamp {
            l: [0.0, 0.0, 1.0],
            ambient: 20.0,
            diffuse: 60.0,
            spec_scale: 150.0,
            squarings: 4,
            front: true,
            base: crate::light::Base::D,
        };
        let n0 = [0.0f32, 0.0, 1.0];
        let own = lamp.eval(n0);
        let pl = PixelLight { lamp, offset: 0.0 };
        let with = |normals: [[f32; 3]; 4], pl: PixelLight, range: [u8; 2]| {
            let mut p = plain[0].clone();
            p.luma = crate::light::quantise(own);
            p.lit = normals.iter().map(|n| [crate::light::clamp_corner(pl.luma(*n)), n[0], n[1], n[2]]).collect();
            p.pixel = Some(pl);
            p.lit_range = range;
            p
        };
        let v = View::native();
        let mut flat = plain[0].clone();
        flat.luma = crate::light::quantise(own);
        let (want, _) = draw_view(&mut [flat.clone()], &src, &v);
        let (got, _) = draw_view(&mut [with([n0; 4], pl, [0, 255])], &src, &v);
        assert_eq!(got, want, "the face's own normal everywhere: flat");
        // Corner normals tilted 30 degrees outward: the centre faces the
        // light, the corners don't.
        let tilt = |x: f32, y: f32| {
            let n = [x * 0.5, y * 0.5, 0.75f32.sqrt()];
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            n.map(|c| c / l)
        };
        let normals = [tilt(-1.0, -1.0), tilt(1.0, -1.0), tilt(1.0, 1.0), tilt(-1.0, 1.0)];
        let (got, _) = draw_view(&mut [with(normals, pl, [0, 255])], &src, &v);
        let at = |img: &[u8], x: usize, y: usize| img[(y * v.width + x) * 3];
        let corner = crate::light::quantise(pl.luma(normals[0]));
        // The centre: the normals' mean is the face's (the highlight's peak).
        assert!(at(&got, 248, 192) + 8 >= (crate::light::quantise(own) >> 2) as u8 * 4);
        assert!(corner + 40 < crate::light::quantise(own), "{corner} vs {own}");
        // Brightest at the centre, darker toward every corner.
        assert!(
            at(&got, 248, 192) > at(&got, 248 - 60, 192 - 45)
                && at(&got, 248 - 60, 192 - 45) > at(&got, 248 - 95, 192 - 70)
        );
        // Clamped into the lumas that keep the hue.
        let (low, _) = draw_view(&mut [with(normals, pl, [0, 60])], &src, &v);
        assert!(low.chunks(3).all(|c| c[0] <= 60));
        assert!(pixel_luma(&pl, 77.0, [0.0; 3]) == 77.0, "no normal: the interpolated luma");
    }

    /// Finer levels (`fine_level`): the columns around the level less half
    /// a column, so column k alone at k + 0.5 and the mean over a gradient
    /// the game's; never outside the levels of the polygon's luma range;
    /// textured by the luma table's value.
    #[test]
    fn fine_levels_blend_adjacent_columns() {
        let all = [0u8, 255];
        assert_eq!(fine_level(None, 2.0, all), (0, 1, 0.0));
        assert_eq!(fine_level(None, 6.0, all), (1, 2, 0.0));
        assert_eq!(fine_level(None, 4.0, all), (0, 1, 0.5));
        assert_eq!(fine_level(None, 0.0, all), (0, 1, 0.0), "clamped at column 0");
        assert_eq!(fine_level(None, 255.9, all), (63, 63, 0.0), "clamped at column 63");
        assert_eq!(fine_level(None, f32::NAN, all), (0, 1, 0.0));
        // The range of a polygon whose colour changes hue above level 29.
        assert_eq!(fine_level(None, 200.0, [0, 119]), (29, 29, 0.0));
        assert_eq!(fine_level(None, 20.0, [120, 255]), (30, 31, 0.0));
        // Textured: level = value * luma / 256.
        assert_eq!(fine_level(Some(128), 100.0, all), (49, 50, 0.5));
        assert_eq!(fine_level(Some(0), 100.0, all), (0, 0, 0.0));
        // Over a gradient the mean column is the game's (ends aside).
        let (mut game, mut fine, mut n) = (0f64, 0f64, 0);
        for i in 0..24000 {
            let l = 8.0 + i as f32 * 0.01;
            let (k, _, f) = fine_level(None, l, all);
            game += (crate::light::quantise(l) >> 2) as f64;
            fine += k as f64 + f as f64;
            n += 1;
        }
        assert!(((fine - game) / n as f64).abs() < 0.01, "{} vs {}", fine / n as f64, game / n as f64);
        // Both columns within the range's levels, adjacent, weight in 0..1.
        let mut seed = 0x2468_ace1u32;
        let mut next = |m: u32| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) % m
        };
        for _ in 0..20000 {
            let (a, b) = (next(256), next(256));
            let range = [a.min(b) as u8, a.max(b) as u8];
            let table = if next(2) == 0 { None } else { Some(next(256)) };
            let l = next(300_000) as f32 / 1000.0 - 20.0;
            let level = |l: u8| match table {
                Some(v) => (v * l as u32 / 256).min(63),
                None => l as u32 >> 2,
            };
            let (k, k1, f) = fine_level(table, l, range);
            assert!(level(range[0]) <= k && k1 <= level(range[1]), "{table:?} {l} {range:?}: {k} {k1}");
            assert!(k1 == k || k1 == k + 1);
            assert!((0.0..1.0).contains(&f) && (k1 > k || f == 0.0), "{f}");
        }
    }

    /// A lit polygon with finer levels: each pixel between the two columns
    /// around the game's level (a column's step at most from it), more
    /// distinct colours than the game's levels give, nothing for a polygon
    /// that isn't lit, and no hue change where the tables turn red.
    #[test]
    fn fine_levels_draw_between_columns() {
        use crate::light::test_stream;
        let q = [[-0.4f32, -0.3, 2.0], [0.4, -0.3, 2.0], [0.4, 0.3, 2.0], [-0.4, 0.3, 2.0]];
        let f = 495.0;
        let (s, _) = test_stream::stream(&[vec![test_stream::tpoly(&q.map(|p| [p[0] * f, p[1] * f, p[2]]), 100)]]);
        let mut palette = vec![0u8; 0x1_c000];
        palette[0x1_0000..].fill(1);
        for ch in 0..3 {
            for lum in 0..64 {
                palette[0x1_0000 + ch * 0x4000 + lum * 2] = (lum * 4) as u8;
            }
        }
        let src = motion_sources(&palette);
        let plain = build(&s, &src);
        let lit = |fine: bool, range: [u8; 2]| {
            let mut p = plain[0].clone();
            p.lit = [40.0f32, 200.0, 200.0, 40.0].iter().map(|&l| [l, 0.0, 0.0, 0.0]).collect();
            p.lit_range = range;
            p.fine = fine;
            p
        };
        let v = View::native();
        let (levels, cov) = draw_view(&mut [lit(false, [0, 255])], &src, &v);
        let (fine, _) = draw_view(&mut [lit(true, [0, 255])], &src, &v);
        let mut shown = (std::collections::HashSet::new(), std::collections::HashSet::new());
        for i in 0..cov.len() {
            if !cov[i] {
                continue;
            }
            let (a, b) = (levels[i * 3], fine[i * 3]);
            assert!(a % 4 == 0 && a.abs_diff(b) <= 4, "pixel {i}: levels {a}, fine {b}");
            shown.0.insert(a);
            shown.1.insert(b);
        }
        assert!(shown.1.len() > 3 * shown.0.len(), "{} against {}", shown.1.len(), shown.0.len());
        // Not lit: `fine` changes nothing.
        let unlit = Poly { fine: true, ..plain[0].clone() };
        assert_eq!(draw_view(&mut [unlit], &src, &v), draw_view(&mut plain.clone(), &src, &v));
        // A table that turns red from level 30 up, the polygon clamped below.
        let mut red = palette.clone();
        for ch in 1..3 {
            for lum in 30..64 {
                red[0x1_0000 + ch * 0x4000 + lum * 2] = 0;
            }
        }
        let rsrc = motion_sources(&red);
        let (flat, _) = draw_view(&mut plain.clone(), &rsrc, &v);
        let (fine, _) = draw_view(&mut [lit(true, [0, 119])], &rsrc, &v);
        assert!(flat.chunks(3).zip(fine.chunks(3)).all(|(a, b)| same_hue([a[0], a[1], a[2]], [b[0], b[1], b[2]])));
        assert!(fine.chunks(3).all(|c| c[0] <= 29 * 4), "never past the range's top column");
    }

    /// Finer levels next to pure black (`fine_columns`): where the game's
    /// column is black the pixel is black, a pair with a black column takes
    /// the other alone, and other pairs blend as before. Drawn, black ink
    /// stays where the game's levels put it and nothing lies between black
    /// and the first lit column (no dark fringe).
    #[test]
    fn fine_levels_keep_pure_black() {
        // Column 0 black, then (70, 65, 60) rising by 3 a column (fvipers'
        // ink tables step like this); colour component 0 only.
        let mut palette = vec![0u8; 0x1_c000];
        for ch in 0..3 {
            for lum in 1..64 {
                palette[0x1_0000 + ch * 0x4000 + lum * 2] = (70 - 5 * ch + 3 * (lum - 1)) as u8;
            }
        }
        let src = motion_sources(&palette);
        let c = [0u32; 3];
        assert_eq!(fine_columns(&src, c, 0, (0, 1, 0.4)), (0, 0, 0.0), "the game's column black");
        assert_eq!(fine_columns(&src, c, 1, (0, 1, 0.4)), (1, 1, 0.0), "pair with black: the other");
        assert_eq!(fine_columns(&src, c, 1, (1, 2, 0.4)), (1, 2, 0.4), "no black: unchanged");
        // A table black at the top as well (not seen in games).
        let mut top = palette.clone();
        for ch in 0..3 {
            top[0x1_0000 + ch * 0x4000 + 9 * 2] = 0;
        }
        assert_eq!(fine_columns(&motion_sources(&top), c, 8, (8, 9, 0.7)), (8, 8, 0.0));
        // Textured, through the luma table's value: the game's column is
        // the texel's, black where it is (dark texel), whatever the blend.
        let (game, k) = (40 * 20 / 256u32, fine_level(Some(40), 20.7, [0, 255]));
        assert_eq!((game, k.0), (3, 2));
        assert_eq!(fine_columns(&src, c, game, k), k);
        let (game, k) = (10 * 20 / 256u32, fine_level(Some(10), 20.7, [0, 255]));
        assert_eq!((game, fine_columns(&src, c, game, k)), (0, (0, 0, 0.0)));

        // Drawn: a polygon lit from luma 0 to 30 (columns 0 to 7).
        use crate::light::test_stream;
        let q = [[-0.4f32, -0.3, 2.0], [0.4, -0.3, 2.0], [0.4, 0.3, 2.0], [-0.4, 0.3, 2.0]];
        let f = 495.0;
        let (s, _) = test_stream::stream(&[vec![test_stream::tpoly(&q.map(|p| [p[0] * f, p[1] * f, p[2]]), 100)]]);
        let plain = build(&s, &src);
        let lit = |fine: bool| {
            let mut p = plain[0].clone();
            p.colour = c;
            p.lit = [0.2f32, 30.0, 30.0, 0.2].iter().map(|&l| [l, 0.0, 0.0, 0.0]).collect();
            p.lit_range = [0, 255];
            p.fine = fine;
            p
        };
        let v = View::native();
        let (levels, cov) = draw_view(&mut [lit(false)], &src, &v);
        let (fine, _) = draw_view(&mut [lit(true)], &src, &v);
        let (mut black, mut blended) = (0, 0);
        for i in (0..cov.len()).filter(|&i| cov[i]) {
            let (a, b) = (&levels[i * 3..i * 3 + 3], &fine[i * 3..i * 3 + 3]);
            assert_eq!(a == [0, 0, 0], b == [0, 0, 0], "pixel {i}: levels {a:?}, fine {b:?}");
            if b == [0, 0, 0] {
                black += 1;
            } else {
                assert!(b[0] >= 70 && b[1] >= 65 && b[2] >= 60, "pixel {i}: fine {b:?} below the first lit column");
            }
            blended += (b[0] % 3 != 70 % 3) as usize;
        }
        assert!(black > 100 && blended > 100, "{black} black, {blended} blended");
    }

    /// `place` maps the previous positions as it maps the points.
    #[test]
    fn placed_motion_is_in_output_pixels() {
        let palette = vec![0u8; 0x1_c000];
        let src = motion_sources(&palette);
        let s1 = motion_stream(248, 192, &QUADS[..1], |p| p);
        let s0 = motion_stream(248, 192, &QUADS[..1], moved);
        let m = MotionIn { s0: &s0, s1: &s1, tp: 0.0, valid: &[], prev_centres: &[] };
        let mut polys = build_motion(&s1, &src, Some(&m)).0;
        let screen: Vec<(f32, f32)> = polys[0].prev.iter().map(|h| (h[0] / h[2], h[1] / h[2])).collect();
        let mut view = View::new(768, 16.0 / 9.0, crate::video::Hud::Center);
        view.hud_windows = 0;
        assert!(view.pad > 0.0);
        let placed = place(&mut polys, &view);
        for (h, &(x, y)) in placed[0].prev.iter().zip(&screen) {
            let (ox, oy) = view.to_output(x, y);
            assert!((h[0] / h[2] - ox).abs() < 1e-3 && (h[1] / h[2] - oy).abs() < 1e-3);
        }
        assert_eq!(placed[0].prev.len(), placed[0].points.len());
        // Without motion, nothing.
        assert!(place(&mut build(&s1, &src), &view)[0].prev.is_empty());
    }
    /// Material maps (`Poly::lamp`): scenes of textured quads with a side
    /// channel (level L2), drawn with a material.
    mod materials {
        use super::*;
        use crate::light::{self, Base, Lamp, Shading, Tap};

        /// Focal length the scenes are made with (the 495-pixel window's).
        const F: f32 = 495.0;
        /// Toward the light (the light vector is its opposite): up, right
        /// and from the eye's side.
        const S: [f32; 3] = [0.6, 0.4, -0.7];
        const LIT: light::Material = light::Material { ambient: 40.0, diffuse: 150.0, spec_scale: 0.0, squarings: 255 };
        /// Scenery lit to 255 at every normal (ambient 255).
        const SATURATED: light::Material =
            light::Material { ambient: 255.0, diffuse: 255.0, spec_scale: 0.0, squarings: 255 };
        /// Both rules (`--materials-relief off`, and relief at strength 2).
        const RELIEFS: [Relief; 2] = [Relief::Faithful, Relief::Diffuse { tenths: 20 }];

        fn unit(v: [f32; 3]) -> [f32; 3] {
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            v.map(|c| c / l)
        }

        fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
            a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
        }

        /// One quad: view-space corners (ring order P1', P0', Pa, Pb),
        /// texture coordinates (texels) per corner, the normal the game
        /// lit it with, the back-face bit, double-sided.
        #[derive(Clone)]
        struct Quad {
            p: [[f32; 3]; 4],
            uv: [[f32; 2]; 4],
            n: [f32; 3],
            back: bool,
        }

        /// A quad of view-space size 2 around `c` with edges `e1`, `e2`
        /// (unit), u along e1 and v along e2 from 0 to `w` and `h`, lit by
        /// the game with normal `n` (its own sign).
        fn quad(c: [f32; 3], e1: [f32; 3], e2: [f32; 3], wh: [f32; 2], n: [f32; 3]) -> Quad {
            let at = |a: f32, b: f32| [0, 1, 2].map(|k| c[k] + a * e1[k] + b * e2[k]);
            let back = dot(n, c) < 0.0;
            Quad {
                p: [at(-1.0, -1.0), at(1.0, -1.0), at(1.0, 1.0), at(-1.0, 1.0)],
                uv: [[0.0, 0.0], [wh[0], 0.0], [wh[0], wh[1]], [0.0, wh[1]]],
                n,
                back,
            }
        }

        /// The texture's header word 0: 64x64, textured, mirror bits.
        fn header0(mirror_u: bool, mirror_v: bool) -> u16 {
            0x4000 | 1 | 1 << 3 | (mirror_u as u16) << 8 | (mirror_v as u16) << 9
        }

        /// A stream of `blocks` (each one object block of quads, in the
        /// layout of `light::test_stream`: a link-0 record before each),
        /// the texture point RAM (header, then each record's points) and the
        /// side channel for lighting entry `mat` and light vector `-S`. Each
        /// quad's luma is the game's (`Lamp::eval` of its normal).
        fn scene(blocks: &[Vec<Quad>], h0: u16, mat: light::Material) -> (Vec<u32>, Vec<u16>, light::Frame) {
            let l = unit(S).map(|c| -c);
            let xy = |x: i32, y: i32| ((x as u32 & 0xfff) << 12) | (y as u32 & 0xfff);
            let mut s =
                vec![0xff00_0003, xy(0, 0), xy(495, 383), xy(248, 192), xy(248, 192), xy(248, 192), xy(248, 192)];
            let put = |s: &mut Vec<u32>, p: [f32; 3]| s.extend([p[0] * F, p[1] * F, p[2]].map(|v| v.to_bits() >> 8));
            let mut ram = vec![h0, 0, 0, 0];
            let mut tap = Tap::new();
            let bits = [mat.ambient.to_bits(), mat.diffuse.to_bits(), mat.spec_scale.to_bits()];
            for b in blocks {
                s.extend([0xff00_0001, 0x80_0000 | ram.len() as u32, 0x80_0000]);
                tap.block(0x80_0000, light::Block::TRUNCATE, F.to_bits(), F.to_bits(), l.map(f32::to_bits));
                put(&mut s, b[0].p[1]);
                put(&mut s, b[0].p[0]);
                for q in b {
                    // Link 0: the next edge is (P0', P1') of the quad.
                    s.extend([1, 0, 0]);
                    put(&mut s, q.p[1]);
                    put(&mut s, q.p[0]);
                    ram.extend([0u16; 8]);
                    tap.face([0; 3], false, true, Base::D, 0, [0; 3], light::Material::NO_SPEC);
                    let face = light::Face { n: q.n, mat: 0, flags: !q.back as u16 };
                    let block = light::Block { light: l, ..Default::default() };
                    let luma = light::quantise(Lamp::of(&block, &face, &mat).eval(q.n));
                    s.extend([1 | 2 | 2 << 8 | 1 << 10 | 1 << 17, luma << 15 | (q.back as u32) << 23, 0]);
                    put(&mut s, q.p[2]);
                    put(&mut s, q.p[3]);
                    for uv in q.uv {
                        ram.extend([(uv[1] * 8.0) as u16, (uv[0] * 8.0) as u16]);
                    }
                    tap.face(q.n.map(f32::to_bits), false, !q.back, Base::D, 0, bits, mat.squarings);
                }
            }
            s.push(0xff00_000f);
            (s, ram, tap.finish(1))
        }

        /// Render RAM: every texel 8 (luma table 0 index 68: value 64).
        fn render_ram() -> Vec<u8> {
            let mut ram = vec![0x88u8; 0x40_0000 + 0x8000];
            ram[0x40_0000..].fill(0);
            for i in 0..128 {
                ram[0x40_0000 + i * 2] = 64;
            }
            ram
        }

        /// Palette: colour 0's translation a grey ramp (column k: 4k).
        fn palette() -> Vec<u8> {
            let mut p = vec![0u8; 0x1_c000];
            for ch in 0..3 {
                for lum in 0..64 {
                    p[0x1_0000 + ch * 0x4000 + lum * 2] = (lum * 4) as u8;
                }
            }
            p
        }

        /// A 64x64 material: a dome of radius 20 texels around (32, 32)
        /// (normals leaning out from its centre, x along u, y along v), flat
        /// elsewhere; or flat everywhere.
        fn dome(flat: bool) -> Material {
            let mut data = Vec::with_capacity(64 * 64 * 4);
            for y in 0..64 {
                for x in 0..64 {
                    let (dx, dy) = (x as f32 + 0.5 - 32.0, y as f32 + 0.5 - 32.0);
                    let r = (dx * dx + dy * dy).sqrt();
                    let (nx, ny) = if flat || r >= 20.0 { (0.0, 0.0) } else { (0.6 * dx / 20.0, 0.6 * dy / 20.0) };
                    let enc = |v: f32| (128.0 + 127.0 * v).round() as u8;
                    data.extend([enc(nx), enc(ny), 128, 128]);
                }
            }
            Material { width: 64, height: 64, data, has: 1, normal_scale: 1.0, spec_scale: 1.0, id: 1 }
        }

        /// A pack: maybe one image for every texture, maybe one material.
        struct Maps {
            image: Option<Arc<Rgba>>,
            material: Option<Arc<Material>>,
            asked: usize,
        }

        impl Enhance for Maps {
            fn texture(&mut self, _: &TexInfo) -> Option<Arc<Rgba>> {
                self.image.clone()
            }
            fn texture_drawn(&mut self, _: &TexInfo, _: &Sources, _: usize) {}
            fn tile(&mut self, _: &crate::enhance::TileInfo) -> Option<Arc<Rgba>> {
                None
            }
            fn texture_material(&mut self, _: &TexInfo) -> Option<Arc<Material>> {
                self.asked += 1;
                self.material.clone()
            }
        }

        fn maps(image: bool, material: Option<Material>) -> Maps {
            let image = image.then(|| {
                // A replacement image: a soft grey gradient.
                let data = (0..64 * 64).flat_map(|i| [100 + (i % 64) as u8, 120, 140, 255]).collect();
                Arc::new(Rgba { width: 64, height: 64, data })
            });
            Maps { image, material: material.map(Arc::new), asked: 0 }
        }

        /// Draw `stream` built with `opts` (lamps or not, side channel or
        /// not) through `pack` with `filter`.
        fn draw(
            stream: &[u32],
            ram: &[u16],
            side: Option<&light::Frame>,
            opts: &light::Options,
            pack: &mut Maps,
            filter: TexFilter,
        ) -> (Vec<u8>, Vec<Poly>) {
            let (rr, pal) = (render_ram(), palette());
            let src = Sources {
                texture_rom: &[],
                texture_point_ram: ram,
                render_ram: &rr,
                palette: &pal,
                xoff: 0.0,
                yoff: 0.0,
            };
            let polys = build_shaded_with(stream, &src, side, Some(opts));
            let mut drawn = polys.clone();
            (draw_enhanced_filtered(&mut drawn, &src, &View::native(), pack, false, filter).0, polys)
        }

        fn opts(shading: Shading, lamps: bool) -> light::Options {
            light::Options { shading, crease: 40, ..Default::default() }.with_lamps(lamps)
        }

        /// The red channel's mean over 3x3 pixels around the screen point
        /// of view-space `p`.
        fn bright(img: &[u8], p: [f32; 3]) -> f32 {
            let (x, y) = ((248.0 + p[0] * F / p[2]) as usize, (192.0 - p[1] * F / p[2]) as usize);
            let mut s = 0.0;
            for dy in 0..3 {
                for dx in 0..3 {
                    s += img[((y + dy - 1) * WIDTH + x + dx - 1) * 3] as f32;
                }
            }
            s / 9.0
        }

        /// The view-space point of quad `q` at texture coordinates (u, v)
        /// (its mapping is affine: bilinear over a parallelogram).
        fn point(q: &Quad, u: f32, v: f32) -> [f32; 3] {
            let (a, b) = ((u - q.uv[0][0]) / (q.uv[1][0] - q.uv[0][0]), (v - q.uv[0][1]) / (q.uv[3][1] - q.uv[0][1]));
            [0, 1, 2].map(|k| q.p[0][k] + a * (q.p[1][k] - q.p[0][k]) + b * (q.p[3][k] - q.p[0][k]))
        }

        /// The dome under the game's light is brighter on the slopes that
        /// face the light, whatever the orientation: checked along each
        /// edge direction of the quad through the dome's centre at (cu, cv)
        /// (`e1`, `e2` in view space), against the direction to the light,
        /// independent of the renderer's frame.
        fn assert_dome(img: &[u8], q: &Quad, (cu, cv): (f32, f32), what: &str) {
            let c = point(q, cu, cv);
            for (du, dv) in [(12.0, 0.0), (0.0, 12.0)] {
                let (a, b) = (point(q, cu + du, cv + dv), point(q, cu - du, cv - dv));
                let e = [0, 1, 2].map(|k| a[k] - c[k]);
                let toward = dot(e, unit(S));
                let (ba, bb) = (bright(img, a), bright(img, b));
                assert!(toward.abs() > 0.05, "{what}: a test direction across the light");
                assert!(
                    (ba - bb) * toward.signum() > 8.0,
                    "{what}: along ({du}, {dv}) toward the light {toward:.2}: {ba} vs {bb}"
                );
            }
        }

        /// A quad facing the eye (front face), seen from behind (a
        /// double-sided back face), with its texture drawn reversed, and
        /// under a mirroring model matrix; in each, a dome lit from a known
        /// side is bright on that side, in every shading mode.
        #[test]
        fn dome_is_lit_from_the_light_s_side() {
            let tilt = -25f32.to_radians();
            let (e1, e2) = ([tilt.cos(), 0.0, tilt.sin()], [0.0, 1.0, 0.0]);
            // The normal by the game's convention: away from the eye.
            let n = unit([-tilt.sin(), 0.0, tilt.cos()]);
            let front = quad([0.0, 0.0, 4.0], e1, e2, [64.0, 64.0], n);
            assert!(!front.back);
            // Seen from behind: the game's normal points at the eye.
            let back = Quad { back: true, n: n.map(|c| -c), ..front.clone() };
            // The texture drawn right to left.
            let flipped = Quad { uv: [[64.0, 0.0], [0.0, 0.0], [0.0, 64.0], [64.0, 64.0]], ..front.clone() };
            // A model mirrored in x: positions and normal mirrored.
            let mx = |p: [f32; 3]| [-p[0], p[1], p[2]];
            let mirrored = Quad { p: front.p.map(mx), n: mx(n), ..front.clone() };
            for (q, what) in
                [(&front, "front"), (&back, "double-sided back"), (&flipped, "flipped"), (&mirrored, "mirrored")]
            {
                let (s, ram, side) = scene(&[vec![q.clone()]], header0(false, false), LIT);
                assert_eq!(light::self_check(&s, &side).stats.l2, 1, "{what}");
                for (shading, relief) in
                    [Shading::Flat, Shading::Vertex, Shading::Pixel].into_iter().flat_map(|s| RELIEFS.map(|r| (s, r)))
                {
                    for filter in [TexFilter::Off, TexFilter::Mip] {
                        let o = opts(shading, true).with_relief(relief);
                        let (img, polys) = draw(&s, &ram, Some(&side), &o, &mut maps(false, Some(dome(false))), filter);
                        let l = polys[0].lamp.as_ref().expect("a lamp");
                        assert!(l.tangent.is_some(), "{what}");
                        assert_dome(&img, q, (32.0, 32.0), &format!("{what} {shading:?} {filter} {relief:?}"));
                        // Off: none of it.
                        let (plain, _) = draw(
                            &s,
                            &ram,
                            Some(&side),
                            &opts(shading, false),
                            &mut maps(false, Some(dome(false))),
                            filter,
                        );
                        assert!(plain != img, "{what}");
                    }
                }
            }
        }

        /// A texture repeated mirrored across the quad: in the second
        /// repeat the image runs backwards and the normals' component along
        /// that axis changes sign, so each repeat's dome is lit the same.
        #[test]
        fn mirrored_repeat_keeps_the_relief() {
            let q = quad([0.0, 0.0, 4.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [128.0, 128.0], [0.0, 0.0, 1.0]);
            for (mu, mv) in [(true, false), (false, true), (true, true)] {
                let (s, ram, side) = scene(&[vec![q.clone()]], header0(mu, mv), LIT);
                let (img, _) = draw(
                    &s,
                    &ram,
                    Some(&side),
                    &opts(Shading::Flat, true),
                    &mut maps(false, Some(dome(false))),
                    TexFilter::Off,
                );
                for c in [(32.0, 32.0), (96.0, 32.0), (32.0, 96.0), (96.0, 96.0)] {
                    assert_dome(&img, &q, c, &format!("mirror {mu} {mv} at {c:?}"));
                }
            }
            // Repeated (not mirrored): every copy the same, no sign change.
            let (s, ram, side) = scene(&[vec![q.clone()]], header0(false, false), LIT);
            let (img, _) = draw(
                &s,
                &ram,
                Some(&side),
                &opts(Shading::Flat, true),
                &mut maps(false, Some(dome(false))),
                TexFilter::Off,
            );
            assert_dome(&img, &q, (96.0, 96.0), "repeat");
        }

        /// A pack whose maps are all flat draws byte for byte what the
        /// same pack draws without materials (original texels or a
        /// replacement image, filtered or not, in every shading mode, and
        /// with finer levels); so does a polygon with an ambient-only lamp
        /// or no lamp (no side channel), whatever its map; and with lamps
        /// but no material nothing changes either.
        #[test]
        fn flat_maps_ambient_only_and_no_lamp_change_nothing() {
            // A strip bent by 30 degrees (smoothed at crease 40) and a
            // double-sided back face, each mapped once.
            let bend = 30f32.to_radians();
            let a = quad([-1.0, 0.0, 5.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [64.0, 64.0], [0.0, 0.0, 1.0]);
            let e1 = [bend.cos(), 0.0, -bend.sin()];
            let mut b = quad([0.0, 0.0, 5.0], e1, [0.0, 1.0, 0.0], [64.0, 64.0], unit([bend.sin(), 0.0, bend.cos()]));
            // Share the edge: b's left edge is a's right edge.
            let shift = [0, 1, 2].map(|k| a.p[1][k] - b.p[0][k]);
            b.p = b.p.map(|p| [p[0] + shift[0], p[1] + shift[1], p[2] + shift[2]]);
            (b.p[0], b.p[3]) = (a.p[1], a.p[2]);
            let back = quad([0.5, -1.5, 6.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [64.0, 64.0], [0.0, 0.0, -1.0]);
            assert!(back.back);
            let (s, ram, side) = scene(&[vec![a, b], vec![back]], header0(false, false), LIT);
            assert_eq!(light::self_check(&s, &side).stats.l2, 2);
            for shading in [Shading::Flat, Shading::Vertex, Shading::Pixel] {
                for fine in [false, true] {
                    for image in [false, true] {
                        for (filter, relief) in
                            [TexFilter::Off, TexFilter::Mip].into_iter().flat_map(|f| RELIEFS.map(|r| (f, r)))
                        {
                            let o = opts(shading, true).with_fine(fine).with_relief(relief);
                            let off = opts(shading, false).with_fine(fine);
                            let what = format!("{shading:?} fine {fine} image {image} {filter} {relief:?}");
                            let (want, _) = draw(&s, &ram, Some(&side), &off, &mut maps(image, None), filter);
                            let mut flat = maps(image, Some(dome(true)));
                            let (got, polys) = draw(&s, &ram, Some(&side), &o, &mut flat, filter);
                            assert!(flat.asked > 0 && polys.iter().all(|p| p.lamp.is_some()), "{what}");
                            assert!(got == want, "flat maps: {what}");
                            // Lamps but no material: the same.
                            let (none, _) = draw(&s, &ram, Some(&side), &o, &mut maps(image, None), filter);
                            assert!(none == want, "no material: {what}");
                            // The dome does change it.
                            let (dome, _) =
                                draw(&s, &ram, Some(&side), &o, &mut maps(image, Some(dome(false))), filter);
                            assert!(dome != want, "{what}");
                            // Smooth shading lights the strip (the test's point).
                            if shading != Shading::Flat {
                                assert!(polys.iter().any(|p| !p.lit.is_empty()), "{what}");
                            }
                        }
                    }
                }
            }
            // An ambient-only lamp: no map can show (no entry for it).
            let ambient = light::Material { diffuse: 0.0, ..LIT };
            let (s2, ram2, side2) = scene(&[vec![back_like()]], header0(false, false), ambient);
            for shading in [Shading::Flat, Shading::Vertex, Shading::Pixel] {
                let o = opts(shading, true);
                let (want, _) =
                    draw(&s2, &ram2, Some(&side2), &opts(shading, false), &mut maps(false, None), TexFilter::Off);
                let mut m = maps(false, Some(dome(false)));
                let (got, polys) = draw(&s2, &ram2, Some(&side2), &o, &mut m, TexFilter::Off);
                assert!(got == want && polys.iter().all(|p| p.lamp.is_none()) && m.asked == 0, "ambient {shading:?}");
            }
            // No side channel (level L0): no lamp, nothing asked, the same.
            for shading in [Shading::Flat, Shading::Vertex, Shading::Pixel] {
                let (want, _) = draw(&s, &ram, None, &opts(shading, false), &mut maps(false, None), TexFilter::Off);
                let mut m = maps(false, Some(dome(false)));
                let (got, polys) = draw(&s, &ram, None, &opts(shading, true), &mut m, TexFilter::Off);
                assert!(got == want && polys.iter().all(|p| p.lamp.is_none()) && m.asked == 0, "L0 {shading:?}");
            }
        }

        /// A surface lit to 255 at every normal: the game's sum (the
        /// faithful rule) can't show a map; the relief rule darkens the
        /// slopes facing away from the light (the dome still lit from the
        /// light's side).
        #[test]
        fn saturated_surface_shows_relative_relief() {
            let q = back_like();
            let (s, ram, side) = scene(&[vec![q.clone()]], header0(false, false), SATURATED);
            for shading in [Shading::Flat, Shading::Pixel] {
                let (want, _) =
                    draw(&s, &ram, Some(&side), &opts(shading, false), &mut maps(false, None), TexFilter::Off);
                for relief in RELIEFS {
                    let o = opts(shading, true).with_relief(relief);
                    let (img, _) = draw(&s, &ram, Some(&side), &o, &mut maps(false, Some(dome(false))), TexFilter::Off);
                    let what = format!("{shading:?} {relief:?}");
                    if relief == Relief::Faithful {
                        assert!(img == want, "{what}: saturated, no relief");
                        continue;
                    }
                    assert_dome(&img, &q, (32.0, 32.0), &what);
                }
            }
        }

        /// The relief rule per pixel: a factor (finite, 0 to [`RELIEF_MAX`])
        /// with the base passed through, none where nothing lights the flat
        /// normal; and
        /// [`relieved`]: pure black stays black, a saturated texel is
        /// scaled from where it saturates, results in 0-255.
        #[test]
        fn relative_relief_edges() {
            let t = Tangent { t: [1.0, 0.0, 0.0], h: 1.0 };
            let lamp = |ambient: f32, l: [f32; 3]| PolyLamp {
                light: light::PixelLight {
                    lamp: Lamp {
                        l,
                        ambient,
                        diffuse: 200.0,
                        spec_scale: 0.0,
                        squarings: 255,
                        front: true,
                        base: Base::D,
                    },
                    offset: 0.0,
                },
                n: [0.0, 0.0, 1.0],
                out: -1.0,
                tangent: Some(t),
                range: [0, 255],
                relief: Relief::Faithful,
            };
            let n = [0.0, 0.0, 1.0];
            for relief in RELIEFS.into_iter().skip(1) {
                let with = |a: f32, l: [f32; 3]| PolyLamp { relief, ..lamp(a, l) };
                let ok = |v: Option<(f32, Option<f32>)>, base: f32| {
                    v.is_none_or(|(b, f)| {
                        b == base && f.is_some_and(|f| f.is_finite() && (0.0..=RELIEF_MAX).contains(&f))
                    })
                };
                // Lit edge-on at the flat normal: no light on the side of
                // the bump that faces away.
                let edge = with(0.0, [1.0, 0.0, 0.0]);
                for xy in [(0.3, 0.0), (-0.3, 0.0)] {
                    let v = material_luma(&edge, n, 80.0, xy, 1.0);
                    assert!(ok(v, 80.0), "{relief:?} {v:?}");
                }
                // Facing away from the light, the bump too: nothing.
                let away = with(0.0, [0.0, 0.0, 1.0]);
                assert_eq!(material_luma(&away, [0.0, 0.0, -1.0], 80.0, (0.3, 0.0), 1.0), None, "{relief:?}");
                let mut some = 0;
                for (xy, base) in [((0.5, 0.4), 300.0), ((-0.5, 0.1), 120.0), ((0.9, 0.0), 255.0)] {
                    for l in [[0.0, 0.0, 1.0], unit([0.6, 0.0, 0.8]), unit([-0.6, 0.3, 0.7])] {
                        let v = material_luma(&with(60.0, l), n, base, xy, 4.0);
                        some += v.is_some() as u32;
                        assert!(ok(v, base), "{relief:?} {v:?}");
                    }
                }
                assert!(some > 0, "{relief:?}");
            }
            // Pure black, a zero table value: unchanged.
            assert_eq!(relieved(0.5, Some(200), 255, 0.5), 0.5);
            assert_eq!(relieved(100.0, Some(0), 255, 0.5), 100.0);
            assert!(relieved(f32::NAN, Some(10), 255, 0.5).is_nan());
            // A texel saturating at luma 81.28 (table value 200): scaled from there.
            assert_eq!(relieved(255.0, Some(200), 255, 0.5), 63.5 * 256.0 / 200.0 * 0.5);
            assert_eq!(relieved(40.0, Some(200), 255, 0.5), 20.0);
            assert_eq!(relieved(200.0, None, 255, 2.0), 255.0);
            // A dim texel (table value 9): its top column is 8 (9 * 255 /
            // 256), which the finer levels reach at luma 256 * 8.5 / 9.
            assert_eq!(relieved(500.0, Some(9), 255, 0.5), 256.0 * 8.5 / 9.0 * 0.5);
            // Its range ends at luma 120: column 4, reached at 256 * 4.5 / 9 = 128.
            assert_eq!(relieved(500.0, Some(9), 120, 0.5), 64.0);
            assert_eq!(Relief::parse("off"), Some(Relief::Faithful));
            assert_eq!(Relief::parse("2"), Some(Relief::Diffuse { tenths: 20 }));
            assert_eq!(Relief::parse("1.5"), Some(Relief::Diffuse { tenths: 15 }));
            assert_eq!(Relief::parse("8"), Some(Relief::Diffuse { tenths: 80 }));
            for bad in ["0", "0.04", "8.1", "-1", "sum", "diff:2", "", "nan"] {
                assert_eq!(Relief::parse(bad), None, "{bad}");
            }
            for r in [Relief::Faithful, Relief::Diffuse { tenths: 20 }, Relief::Diffuse { tenths: 15 }] {
                assert_eq!(Relief::parse(&r.name()), Some(r));
            }
            assert_eq!(Relief::default(), Relief::Diffuse { tenths: Relief::DEFAULT_TENTHS });
            assert_eq!(light::Options::default().relief, Relief::default());
        }

        fn back_like() -> Quad {
            quad([0.0, 0.0, 4.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [64.0, 64.0], [0.0, 0.0, 1.0])
        }

        /// Texture coordinates squeezed to a line (every corner at one u):
        /// no tangent frame, no change, nothing not finite.
        #[test]
        fn degenerate_coordinates_change_nothing() {
            let mut q = back_like();
            q.uv = [[5.0, 0.0], [5.0, 0.0], [5.0, 64.0], [5.0, 64.0]];
            let (s, ram, side) = scene(&[vec![q.clone()]], header0(false, false), LIT);
            for shading in [Shading::Flat, Shading::Pixel] {
                let (want, _) =
                    draw(&s, &ram, Some(&side), &opts(shading, false), &mut maps(false, None), TexFilter::Off);
                let mut m = maps(false, Some(dome(false)));
                let (got, polys) = draw(&s, &ram, Some(&side), &opts(shading, true), &mut m, TexFilter::Off);
                let l = polys[0].lamp.as_ref().expect("a lamp");
                assert!(l.tangent.is_none() && m.asked == 0 && got == want, "{shading:?}");
            }
            // The frame and the luma never give anything but a finite
            // value or None, whatever goes in.
            let ring =
                |uv: [[f32; 2]; 3]| [([0.0, 0.0, 1.0], uv[0]), ([1.0, 0.0, 1.0], uv[1]), ([0.0, 1.0, 1.0], uv[2])];
            assert!(
                tangent_frame(&ring([[0.0, 0.0], [1.0, 1.0], [2.0, 2.0]]), [0.0, 0.0, -1.0]).is_none(),
                "on a line"
            );
            assert!(tangent_frame(&ring([[0.0, 0.0], [f32::NAN, 1.0], [0.0, 2.0]]), [0.0, 0.0, -1.0]).is_none());
            let t = tangent_frame(&ring([[0.0, 0.0], [4.0, 0.0], [0.0, 4.0]]), [0.0, 0.0, -1.0]).unwrap();
            assert_eq!(t.t, [1.0, 0.0, 0.0]);
            let lamp = Lamp {
                l: [0.0, 0.0, 1.0],
                ambient: 10.0,
                diffuse: 100.0,
                spec_scale: 0.0,
                squarings: 255,
                front: true,
                base: Base::D,
            };
            let pl = PolyLamp {
                light: light::PixelLight { lamp, offset: 0.0 },
                n: [0.0, 0.0, 1.0],
                out: -1.0,
                tangent: Some(t),
                range: [0, 255],
                relief: Relief::Faithful,
            };
            for n in [[0.0; 3], [f32::NAN, 0.0, 1.0], [f32::INFINITY, 0.0, 0.0], [1e-30, 0.0, 0.0]] {
                assert_eq!(material_luma(&pl, n, 50.5, (0.3, 0.2), 1.0), None, "{n:?}");
            }
            for xy in [(f32::NAN, 0.0), (0.0, 0.0)] {
                assert_eq!(material_luma(&pl, [0.0, 0.0, 1.0], 50.5, xy, 1.0), None, "{xy:?}");
            }
            assert_eq!(material_luma(&pl, [0.0, 0.0, 1.0], 50.5, (0.3, 0.2), 0.0), None, "scale 0");
            let (v, _) = material_luma(&pl, [0.0, 0.0, 1.0], 50.5, (0.3, 0.2), 4.0).unwrap();
            assert!(v.is_finite() && v < 50.5, "tilted away from a light on the axis: darker");
        }
    }
}
