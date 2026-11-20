// Model 2 3D layer on the GPU: a line-for-line port of the software
// rasteriser's shading (crates/model2/src/raster.rs: `shade`,
// `filtered_texel`, `Sources`). Texels, luma tables and colour translation
// are read from the game's own video RAM, uploaded as-is, so the two
// renderers can only differ in edge coverage and float rounding.

struct Poly {
    // Clip rectangle x0, y0, x1, y1 in output pixels.
    clip: vec4f,
    // Texture position and size on the sheet, in texels.
    tex: vec4u,
    // Bit 0 textured, 1 mirror x, 2 mirror y, 3 translucent, 4 checker,
    // 5 replaced from a texture pack (group 1).
    flags: u32,
    bank: u32,
    luma_table: u32,
    luma: u32,
    // 5-bit colour components; w: draw order within the window (the depth
    // buffer's tie-break).
    colour: vec4u,
}

struct Screen {
    // Output size in pixels, and output pixels per game pixel.
    size: vec2f,
    scale: vec2f,
    // The look (look.rs): toon steps (0: off), the darkest band's level and
    // where the bands start; the ink pass's per-polygon entries' place in
    // `outlines` (a bit pattern).
    toon: vec4f,
    // Halftone in the darkest toon band: dot coverage (0: none), pitch in
    // output pixels, angle in radians; w unused.
    half: vec4f,
    // The halftone dots' colour.
    ink: vec4f,
}

@group(0) @binding(0) var<uniform> screen: Screen;
@group(0) @binding(1) var<storage, read> polys: array<Poly>;
// Render RAM from 0x11000000: texture banks at +0 / +0x200000, luma tables
// at +0x400000.
@group(0) @binding(2) var<storage, read> render_ram: array<u32>;
// Palette RAM, with the colour translation tables at +0x10000.
@group(0) @binding(3) var<storage, read> palette: array<u32>;
// Each polygon's outline for the depth buffer: entry i is (first, count)
// as bit patterns, and entries first..first + count are its points (x, y
// in output pixels, 1/z).
@group(0) @binding(4) var<storage, read> outlines: array<vec4f>;

struct VsOut {
    @builtin(position) pos: vec4f,
    // 1/z, u/z, v/z: linear in screen space, as the software fill does.
    // Centroid: inside the polygon even where a multisampled pixel is only
    // partly covered (the same as the pixel centre with one sample).
    @location(0) @interpolate(linear, centroid) attr: vec3f,
    @location(1) @interpolate(flat) poly: u32,
}

@vertex
fn vs(@location(0) xy: vec2f, @location(1) attr: vec3f, @location(2) poly: u32) -> VsOut {
    var o: VsOut;
    o.pos = vec4f(xy.x / screen.size.x * 2.0 - 1.0, 1.0 - xy.y / screen.size.y * 2.0, 0.0, 1.0);
    o.attr = attr;
    o.poly = poly;
    return o;
}

fn render8(o: u32) -> u32 {
    if (o >> 2u) >= arrayLength(&render_ram) {
        return 0u;
    }
    return (render_ram[o >> 2u] >> ((o & 3u) * 8u)) & 0xffu;
}

fn palette8(o: u32) -> u32 {
    if (o >> 2u) >= arrayLength(&palette) {
        return 0u;
    }
    return (palette[o >> 2u] >> ((o & 3u) * 8u)) & 0xffu;
}

// 4bpp texel at sheet coordinates (x, y) in `bank`.
fn texel(bank: u32, x_in: u32, y_in: u32) -> u32 {
    var x = x_in;
    var y = y_in;
    if x >= 1024u {
        x = x - 1024u;
        y = y ^ 1024u;
    }
    let word = ((y & 2047u) / 2u) * 512u + (x & 1023u) / 2u;
    let o = (bank & 1u) * 0x200000u + word * 2u;
    let w = render8(o) | (render8(o + 1u) << 8u);
    let shift = 12u - 4u * ((x & 1u) + 2u * (y & 1u));
    return (w >> shift) & 15u;
}

fn wrap(c: i32, size: u32, mirror: bool) -> u32 {
    let s = i32(size);
    if mirror {
        let m = ((c % (2 * s)) + 2 * s) % (2 * s);
        if m >= s {
            return u32(2 * s - 1 - m);
        }
        return u32(m);
    }
    return u32(((c % s) + s) % s);
}

// Bilinearly filtered texel scaled to 0-255, or -1 where a translucent
// texture is mostly transparent.
fn filtered_texel(p: Poly, u_in: f32, v_in: f32) -> i32 {
    let u = u_in - 0.5;
    let v = v_in - 0.5;
    let fu = u - floor(u);
    let fv = v - floor(v);
    let u0 = i32(floor(u));
    let v0 = i32(floor(v));
    let mx = (p.flags & 2u) != 0u;
    let my = (p.flags & 4u) != 0u;
    let translucent = (p.flags & 8u) != 0u;
    var sum = 0.0;
    var weight = 0.0;
    for (var k = 0; k < 4; k++) {
        let du = k & 1;
        let dv = k >> 1;
        var w: f32;
        switch k {
            case 0: { w = (1.0 - fu) * (1.0 - fv); }
            case 1: { w = fu * (1.0 - fv); }
            case 2: { w = (1.0 - fu) * fv; }
            default: { w = fu * fv; }
        }
        let tx = p.tex.x + wrap(u0 + du, p.tex.z, mx);
        let ty = p.tex.y + wrap(v0 + dv, p.tex.w, my);
        let t = texel(p.bank, tx, ty);
        if !(translucent && t == 15u) {
            sum += f32(t) * w;
            weight += w;
        }
    }
    if weight < 0.5 {
        return -1;
    }
    // Rust's round(): half away from zero (values here are never negative).
    return i32(floor(sum / weight * 17.0 + 0.5));
}

fn luma(table: u32, index: u32) -> u32 {
    return render8(0x400000u + ((table * 128u + index) & 0x7fffu) * 2u);
}

fn translate(ch: u32, c: u32, lum: u32) -> u32 {
    return palette8(0x10000u + ch * 0x4000u + (c * 256u + lum) * 2u);
}

fn shade(in: VsOut) -> vec4f {
    let p = polys[in.poly];
    let px = floor(in.pos.xy);
    // The software fill's clip: rows cy0 <= y < cy1, pixel centres
    // cx0 <= x + 0.5 < cx1.
    if px.y < p.clip.y || px.y >= p.clip.w || px.x + 0.5 < p.clip.x || px.x + 0.5 >= p.clip.z {
        discard;
    }
    let g = vec2u(floor(px / screen.scale));
    if (p.flags & 16u) != 0u && ((g.x ^ g.y) & 1u) != 0u {
        discard;
    }
    var lum: u32;
    if (p.flags & 1u) != 0u {
        let iz = in.attr.x;
        let t8 = filtered_texel(p, in.attr.y / iz, in.attr.z / iz);
        if t8 < 0 {
            discard;
        }
        lum = min(luma(p.luma_table, u32(t8) / 2u) * p.luma / 256u, 63u);
    } else {
        lum = p.luma >> 2u;
    }
    let rgb = vec3u(translate(0u, p.colour.x, lum), translate(1u, p.colour.y, lum), translate(2u, p.colour.z, lum));
    return vec4f(vec3f(rgb) / 255.0, 1.0);
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4f {
    return shade(in);
}

// The depth buffer's value, as model2::raster::depth_value: larger is
// nearer. Written by the shader because 1/z is the quantity that is
// linear across the screen.
const DEPTH_BIAS: f32 = @DEPTH_BIAS@;
const DEPTH_SCALE: f32 = @DEPTH_SCALE@;

// 1 when writing, 1 + DEPTH_TOLERANCE when testing (the Test pipeline):
// model2::raster::depth_test_value.
override DEPTH_FACTOR: f32 = 1.0;

struct DepthOut {
    @location(0) colour: vec4f,
    @builtin(frag_depth) depth: f32,
}

// 1/z at the centre of pixel `px` as the software fill has it: on the
// polygon's edges at the row's centre, then across the span. Not from the
// triangle fan's interpolation, which for a quad that isn't flat depends
// on the diagonal, so that two copies of one quad starting at different
// corners (a decal on its surface) would get different depths. `iz` where
// the row misses the outline (an edge pixel the GPU covers and the
// software doesn't).
fn outline_iz(poly: u32, px: vec2f, iz: f32) -> f32 {
    let r = bitcast<vec2u>(outlines[poly].xy);
    let yc = px.y + 0.5;
    var l = vec2f(3.4e38, 0.0);
    var rt = vec2f(-3.4e38, 0.0);
    for (var k = 0u; k < r.y; k++) {
        let a = outlines[r.x + k];
        let b = outlines[r.x + (k + 1u) % r.y];
        if (a.y <= yc && b.y > yc) || (b.y <= yc && a.y > yc) {
            let t = (yc - a.y) / (b.y - a.y);
            let e = vec2f(a.x + (b.x - a.x) * t, a.z + (b.z - a.z) * t);
            if e.x < l.x {
                l = e;
            }
            if e.x > rt.x {
                rt = e;
            }
        }
    }
    if l.x > rt.x {
        return iz;
    }
    let t = ((px.x + 0.5) - l.x) / max(rt.x - l.x, 1e-6);
    return l.y + (rt.y - l.y) * t;
}

// The depth of polygon `poly` at fragment position `pos`, as
// model2::raster::depth_value (times DEPTH_FACTOR).
fn depth_at(poly: u32, pos: vec4f, iz: f32) -> f32 {
    let z = outline_iz(poly, floor(pos.xy), iz) * (1.0 + f32(polys[poly].colour.w) * DEPTH_BIAS) * DEPTH_FACTOR;
    return z / (z + DEPTH_SCALE);
}

@fragment
fn fs_depth(in: VsOut) -> DepthOut {
    var o: DepthOut;
    o.colour = shade(in);
    o.depth = depth_at(in.poly, in.pos, in.attr.x);
    return o;
}

// Texture packs (docs/texture-packs.md): a polygon with flag bit 5 (32) is
// drawn from the replacement image in group 1 instead of its texels, as
// the software rasteriser's `shade_replaced` does. Only the pack pipelines
// (`fs_pack`, `fs_pack_depth`) use group 1; `vs`, `fs` and `fs_depth` are
// the same with or without it.

// The replacement image: Rgba8Unorm, straight alpha, one level.
@group(1) @binding(0) var pack_tex: texture_2d<f32>;

// enhance::Rgba::sample at (x, y) in image pixels: RGB 0-255 weighted by
// alpha (transparent texels don't darken edges), alpha 0-1.
fn pack_sample(p: Poly, x_in: f32, y_in: f32) -> vec4f {
    let dim = textureDimensions(pack_tex, 0);
    let x = x_in - 0.5;
    let y = y_in - 0.5;
    let fx = x - floor(x);
    let fy = y - floor(y);
    let x0 = i32(floor(x));
    let y0 = i32(floor(y));
    let mx = (p.flags & 2u) != 0u;
    let my = (p.flags & 4u) != 0u;
    var rgb = vec3f(0.0);
    var alpha = 0.0;
    for (var k = 0; k < 4; k++) {
        var w: f32;
        switch k {
            case 0: { w = (1.0 - fx) * (1.0 - fy); }
            case 1: { w = fx * (1.0 - fy); }
            case 2: { w = (1.0 - fx) * fy; }
            default: { w = fx * fy; }
        }
        let px = wrap(x0 + (k & 1), dim.x, mx);
        let py = wrap(y0 + (k >> 1), dim.y, my);
        // Back to the 8-bit values the software reads.
        let t = round(textureLoad(pack_tex, vec2u(px, py), 0) * 255.0);
        let wa = w * t.a / 255.0;
        rgb += wa * t.rgb;
        alpha += wa;
    }
    if alpha > 0.0 {
        rgb = rgb / alpha;
    }
    return vec4f(rgb, alpha);
}

// raster.rs `Light::apply`: the replacement colour `rgb` (0-255) stands for
// the texture at full luma, so each channel is scaled by what the original
// pipeline gives at the polygon's luma over what it gives at full luma, for
// the original texel; where that texel is black at full luma in a channel
// but not at this luma, the channel adds the original's value (nothing
// scales up from black). Before rounding.
fn pack_lit(p: Poly, rgb: vec3f, u: f32, v: f32) -> vec3f {
    if p.luma >= 255u {
        return rgb;
    }
    let c = p.colour.xyz;
    // Fallback where the original texel is transparent or black at full
    // luma: the ratio for the brightest luma.
    var fb: vec3f;
    for (var ch = 0u; ch < 3u; ch++) {
        let full = translate(ch, c[ch], 63u);
        if full > 0u {
            fb[ch] = f32(translate(ch, c[ch], min(64u * p.luma / 256u, 63u))) / f32(full);
        } else {
            fb[ch] = f32(p.luma) / 255.0;
        }
    }
    let t8 = filtered_texel(p, u, v);
    if t8 < 0 {
        return rgb * fb;
    }
    let l = luma(p.luma_table, u32(t8) / 2u);
    let lp = min(l * p.luma / 256u, 63u);
    let lf = min(l * 255u / 256u, 63u);
    var r: vec3f;
    for (var ch = 0u; ch < 3u; ch++) {
        let full = translate(ch, c[ch], lf);
        let here = f32(translate(ch, c[ch], lp));
        if full == 0u {
            r[ch] = rgb[ch] * fb[ch] + here;
        } else {
            r[ch] = rgb[ch] * (here / f32(full));
        }
    }
    return r;
}

// `shade`, or for a replaced polygon raster.rs `shade_replaced`.
fn shade_pack(in: VsOut) -> vec4f {
    let p = polys[in.poly];
    if (p.flags & 32u) == 0u {
        return shade(in);
    }
    let px = floor(in.pos.xy);
    if px.y < p.clip.y || px.y >= p.clip.w || px.x + 0.5 < p.clip.x || px.x + 0.5 >= p.clip.z {
        discard;
    }
    let g = vec2u(floor(px / screen.scale));
    if (p.flags & 16u) != 0u && ((g.x ^ g.y) & 1u) != 0u {
        discard;
    }
    let iz = in.attr.x;
    let u = in.attr.y / iz;
    let v = in.attr.z / iz;
    let dim = vec2f(textureDimensions(pack_tex, 0));
    let s = pack_sample(p, u * dim.x / f32(p.tex.z), v * dim.y / f32(p.tex.w));
    if s.a < 0.5 {
        discard;
    }
    // Rust's round() then clamp, as the software (values are never
    // negative).
    let rgb = min(floor(pack_lit(p, s.rgb, u, v) + 0.5), vec3f(255.0));
    return vec4f(rgb / 255.0, 1.0);
}

@fragment
fn fs_pack(in: VsOut) -> @location(0) vec4f {
    return shade_pack(in);
}

@fragment
fn fs_pack_depth(in: VsOut) -> DepthOut {
    var o: DepthOut;
    o.colour = shade_pack(in);
    o.depth = depth_at(in.poly, in.pos, in.attr.x);
    return o;
}

// Filtered pack images (Quality::tex_filter mip or anisotropic): the
// image's premultiplied mip chain, sampled through `pack_smp` (linear, mip
// levels blended, anisotropy as asked; repeat or mirror per axis as the
// polygon's flags) with the gradients of the texture coordinates, so the
// GPU picks the level for the pixel's footprint. `vs_pack`, `fs_packf` and
// `fs_packf_depth` only.
@group(1) @binding(1) var pack_smp: sampler;

struct VsPackOut {
    @builtin(position) pos: vec4f,
    @location(0) @interpolate(linear, centroid) attr: vec3f,
    @location(1) @interpolate(flat) poly: u32,
    // The same values at the pixel centre: their screen derivatives are the
    // triangle's plane gradients, also at partly covered multisampled
    // pixels.
    @location(2) @interpolate(linear, center) attr_c: vec3f,
}

@vertex
fn vs_pack(@location(0) xy: vec2f, @location(1) attr: vec3f, @location(2) poly: u32) -> VsPackOut {
    var o: VsPackOut;
    o.pos = vec4f(xy.x / screen.size.x * 2.0 - 1.0, 1.0 - xy.y / screen.size.y * 2.0, 0.0, 1.0);
    o.attr = attr;
    o.poly = poly;
    o.attr_c = attr;
    return o;
}

// `shade`, or for a replaced polygon a filtered sample of its image. `gx`
// and `gy`: screen derivatives of (1/z, u/z, v/z), taken by the caller
// before anything can discard.
fn shade_packf(in: VsPackOut, gx: vec3f, gy: vec3f) -> vec4f {
    let p = polys[in.poly];
    if (p.flags & 32u) == 0u {
        var o: VsOut;
        o.pos = in.pos;
        o.attr = in.attr;
        o.poly = in.poly;
        return shade(o);
    }
    let px = floor(in.pos.xy);
    if px.y < p.clip.y || px.y >= p.clip.w || px.x + 0.5 < p.clip.x || px.x + 0.5 >= p.clip.z {
        discard;
    }
    let g = vec2u(floor(px / screen.scale));
    if (p.flags & 16u) != 0u && ((g.x ^ g.y) & 1u) != 0u {
        discard;
    }
    let iz = in.attr.x;
    let u = in.attr.y / iz;
    let v = in.attr.z / iz;
    // Texture coordinates 0-1 over the original's texels; with u = (u/z) /
    // (1/z), both linear on screen, du/dx = (d(u/z)/dx - u d(1/z)/dx) / (1/z).
    let size = vec2f(f32(p.tex.z), f32(p.tex.w));
    let ddx = vec2f(gx.y - u * gx.x, gx.z - v * gx.x) / (iz * size);
    let ddy = vec2f(gy.y - u * gy.x, gy.z - v * gy.x) / (iz * size);
    let c = textureSampleGrad(pack_tex, pack_smp, vec2f(u, v) / size, ddx, ddy);
    // The cut-out after filtering; at least half opaque, so un-premultiplying
    // at most doubles the 8-bit rounding.
    if c.a < 0.5 {
        discard;
    }
    let rgb = min(floor(pack_lit(p, c.rgb / c.a * 255.0, u, v) + 0.5), vec3f(255.0));
    return vec4f(rgb / 255.0, 1.0);
}

@fragment
fn fs_packf(in: VsPackOut) -> @location(0) vec4f {
    let gx = dpdx(in.attr_c);
    let gy = dpdy(in.attr_c);
    return shade_packf(in, gx, gy);
}

@fragment
fn fs_packf_depth(in: VsPackOut) -> DepthOut {
    let gx = dpdx(in.attr_c);
    let gy = dpdy(in.attr_c);
    var o: DepthOut;
    o.colour = shade_packf(in, gx, gy);
    o.depth = depth_at(in.poly, in.pos, in.attr.x);
    return o;
}

// Empties the depth buffer (to the far value, 0) where a window draws: a
// triangle over the screen, limited by the scissor rectangle.
@vertex
fn vs_clear(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4f(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_clear() -> @location(0) vec4f {
    return vec4f(0.0);
}

// Temporal anti-aliasing (Quality::temporal, temporal.rs): the 3D layer is
// drawn at an internal size with a sub-pixel jitter, into the colour target
// and an aux target of motion and depth. The `_t` entry points below; the
// ones above are untouched. For a temporal frame the CPU gives the shared
// shading functions a clip rectangle that never discards and no checker
// bit (4): clipping and the checkerboard are done here, at the unjittered
// sample point in output pixels. The real clip rectangle is in `outlines`
// after the outlines (`clip_t`), the checker bit is flag bit 6 (64), and bit 7 (128) says the polygon has
// motion data (vertex attribute 3). Bit 8 (256): the polygon is thin (narrower than an
// internal pixel or so: `temporal::is_thin`), marked in the aux target.

struct ScreenT {
    // Internal size over output size, per axis.
    rs: vec2f,
    // The jitter in output pixels.
    jitter_d: vec2f,
    // Scale of texture derivatives for filtered pack images: rs, so they
    // are per output pixel (a LOD bias when upscaling).
    lod: f32,
    // Bit 0: keep the checkerboard (Stipple::Keep); else blend it at 50%.
    flags: u32,
    // Where the polygons' clip rectangles start in `outlines`.
    clip_base: u32,
}

@group(0) @binding(5) var<uniform> screen_t: ScreenT;

// Polygon `poly`'s clip rectangle x0, y0, x1, y1 in output pixels.
fn clip_t(poly: u32) -> vec4f {
    return outlines[screen_t.clip_base + poly];
}

struct VsTOut {
    @builtin(position) pos: vec4f,
    @location(0) @interpolate(linear, centroid) attr: vec3f,
    @location(1) @interpolate(flat) poly: u32,
    // As VsPackOut's, for the filtered pack images' derivatives.
    @location(2) @interpolate(linear, center) attr_c: vec3f,
    // The previous position, homogeneous (X, Y, W) in output pixels:
    // linear in screen space, like attr.
    @location(3) @interpolate(linear, center) prev: vec3f,
}

@vertex
fn vs_t(@location(0) xy: vec2f, @location(1) attr: vec3f, @location(2) poly: u32, @location(3) prev: vec3f) -> VsTOut {
    var o: VsTOut;
    let s = xy + screen_t.jitter_d;
    o.pos = vec4f(s.x / screen.size.x * 2.0 - 1.0, 1.0 - s.y / screen.size.y * 2.0, 0.0, 1.0);
    o.attr = attr;
    o.poly = poly;
    o.attr_c = attr;
    o.prev = prev;
    return o;
}

// The fragment's sample point without the jitter, in output pixels (the
// pixel centre at 100% without jitter).
fn sample_t(pos: vec4f) -> vec2f {
    return pos.xy / screen_t.rs - screen_t.jitter_d;
}

// `shade`'s clip and checkerboard at the sample point (as there, `px` is
// the sample less half a pixel: the pixel's corner at 100% unjittered).
// The coverage: 1, or 0.5 for a checkerboard polygon blended.
fn pre_t(in: VsTOut) -> f32 {
    let px = sample_t(in.pos) - 0.5;
    let c = clip_t(in.poly);
    if px.y < c.y || px.y >= c.w || px.x + 0.5 < c.x || px.x + 0.5 >= c.z {
        discard;
    }
    if (polys[in.poly].flags & 64u) != 0u {
        if (screen_t.flags & 1u) == 0u {
            return 0.5;
        }
        let g = vec2i(floor(px / screen.scale));
        if ((g.x ^ g.y) & 1) != 0 {
            discard;
        }
    }
    return 1.0;
}

// The aux target: motion in output pixels (previous position less the
// sample point), 1/z (negative: blended checkerboard), the expected
// previous 1/z (0: no motion data; negative: the polygon is thin, flag
// bit 8, for the resolve's thin-feature lock). Clamped so it stays finite
// in 16 bits.
fn aux_t(in: VsTOut, sheer: bool) -> vec4f {
    var b = clamp(in.attr.x, 6.2e-5, 6.0e4);
    if sheer {
        b = -b;
    }
    let w = in.prev.z;
    if (polys[in.poly].flags & 128u) == 0u || !(w > 1e-6) {
        return vec4f(0.0, 0.0, b, 0.0);
    }
    let mv = clamp(in.prev.xy / w - sample_t(in.pos), vec2f(-16384.0), vec2f(16384.0));
    let e = clamp(in.attr.x / w, 6.2e-5, 6.0e4);
    return vec4f(mv, b, select(e, -e, (polys[in.poly].flags & 256u) != 0u));
}

// A thin polygon's coverage (flag bits 16-23, 255 = 1): the CPU widened
// it to an internal pixel or so (temporal.rs, `thin`), so it is drawn at
// this share over what is behind (the colour target blends premultiplied).
fn cover_t(poly: u32) -> f32 {
    let f = polys[poly].flags;
    if (f & 256u) == 0u {
        return 1.0;
    }
    return f32((f >> 16u) & 255u) / 255.0;
}

fn plain_t(in: VsTOut) -> VsOut {
    var o: VsOut;
    o.pos = in.pos;
    o.attr = in.attr;
    o.poly = in.poly;
    return o;
}

// depth_at at the sample point.
fn depth_t(in: VsTOut) -> f32 {
    let z = outline_iz(in.poly, sample_t(in.pos) - 0.5, in.attr.x) * (1.0 + f32(polys[in.poly].colour.w) * DEPTH_BIAS)
        * DEPTH_FACTOR;
    return z / (z + DEPTH_SCALE);
}

struct TOut {
    @location(0) colour: vec4f,
    @location(1) aux: vec4f,
}

struct TDepthOut {
    @location(0) colour: vec4f,
    @location(1) aux: vec4f,
    @builtin(frag_depth) depth: f32,
}

@fragment
fn fs_t(in: VsTOut) -> TOut {
    let a = pre_t(in);
    var o: TOut;
    o.colour = shade(plain_t(in)) * (a * cover_t(in.poly));
    o.aux = aux_t(in, a < 1.0);
    return o;
}

@fragment
fn fs_depth_t(in: VsTOut) -> TDepthOut {
    let a = pre_t(in);
    var o: TDepthOut;
    o.colour = shade(plain_t(in)) * (a * cover_t(in.poly));
    o.aux = aux_t(in, a < 1.0);
    o.depth = depth_t(in);
    return o;
}

@fragment
fn fs_pack_t(in: VsTOut) -> TOut {
    let a = pre_t(in);
    var o: TOut;
    o.colour = shade_pack(plain_t(in)) * (a * cover_t(in.poly));
    o.aux = aux_t(in, a < 1.0);
    return o;
}

@fragment
fn fs_pack_depth_t(in: VsTOut) -> TDepthOut {
    let a = pre_t(in);
    var o: TDepthOut;
    o.colour = shade_pack(plain_t(in)) * (a * cover_t(in.poly));
    o.aux = aux_t(in, a < 1.0);
    o.depth = depth_t(in);
    return o;
}

fn packf_t(in: VsTOut) -> VsPackOut {
    var o: VsPackOut;
    o.pos = in.pos;
    o.attr = in.attr;
    o.poly = in.poly;
    o.attr_c = in.attr_c;
    return o;
}

@fragment
fn fs_packf_t(in: VsTOut) -> TOut {
    let gx = dpdx(in.attr_c) * screen_t.lod;
    let gy = dpdy(in.attr_c) * screen_t.lod;
    let a = pre_t(in);
    var o: TOut;
    o.colour = shade_packf(packf_t(in), gx, gy) * (a * cover_t(in.poly));
    o.aux = aux_t(in, a < 1.0);
    return o;
}

@fragment
fn fs_packf_depth_t(in: VsTOut) -> TDepthOut {
    let gx = dpdx(in.attr_c) * screen_t.lod;
    let gy = dpdy(in.attr_c) * screen_t.lod;
    let a = pre_t(in);
    var o: TDepthOut;
    o.colour = shade_packf(packf_t(in), gx, gy) * (a * cover_t(in.poly));
    o.aux = aux_t(in, a < 1.0);
    o.depth = depth_t(in);
    return o;
}

// Smooth shading (Quality::shading, model2::light): the `_l` entry points,
// for frames with a lit polygon. A lit polygon's outline entry is (first,
// count, flags, normals): flags bit 0 lit, bit 1 lit per pixel, bits 8-15
// and 16-23 the lumas a pixel is clamped to (raster.rs `Poly::lit_range`);
// its points carry luma/z in w. Per pixel, `normals` is the index of
// `count` entries (nx/z, ny/z, nz/z, 0), one per point, then its lighting
// in two (`lamp_eval`). Its luma is taken per pixel from those
// (`lit_luma`), every other polygon's is its own. The shading functions
// above stay as they were (the frames without a lit polygon draw exactly
// as before); `shade_l`, `shade_pack_l` and `shade_packf_l` are them with
// the polygon passed in.

// Where the row at height `yc` crosses an outline: the edges (from point
// la to lb at lt, and ra to rb at rt; indices from the first point) at the
// span's two ends, and their x. ok false where the row misses the outline.
struct Cross {
    la: u32,
    lb: u32,
    lt: f32,
    lx: f32,
    ra: u32,
    rb: u32,
    rt: f32,
    rx: f32,
    ok: bool,
}

fn outline_cross(first: u32, count: u32, yc: f32) -> Cross {
    var c: Cross;
    c.lx = 3.4e38;
    c.rx = -3.4e38;
    for (var k = 0u; k < count; k++) {
        let k1 = (k + 1u) % count;
        let a = outlines[first + k];
        let b = outlines[first + k1];
        if (a.y <= yc && b.y > yc) || (b.y <= yc && a.y > yc) {
            let t = (yc - a.y) / (b.y - a.y);
            let x = a.x + (b.x - a.x) * t;
            if x < c.lx {
                c.lx = x;
                c.la = k;
                c.lb = k1;
                c.lt = t;
            }
            if x > c.rx {
                c.rx = x;
                c.ra = k;
                c.rb = k1;
                c.rt = t;
            }
        }
    }
    c.ok = c.lx <= c.rx;
    return c;
}

fn lerp4(a: vec4f, b: vec4f, t: f32) -> vec4f {
    return a + (b - a) * t;
}

// An attribute of the outline's points (entries `base`..), interpolated
// along the edges of `c`, then across the span at `s`, as the software
// fill does.
fn cross_at(base: u32, c: Cross, s: f32) -> vec4f {
    let l = lerp4(outlines[base + c.la], outlines[base + c.lb], c.lt);
    let r = lerp4(outlines[base + c.ra], outlines[base + c.rb], c.rt);
    return lerp4(l, r, s);
}

// model2::light::Lamp::eval plus PixelLight's offset: lamp entries `i`
// (L, flags: bit 0 front, bits 1-2 the specular base (0 d, 1 max(d, 0),
// 2 n·L), bits 8-15 squarings, 255 none) and `i + 1` (ambient, diffuse,
// specular scale, offset), for unit normal `n`.
fn lamp_eval(i: u32, n: vec3f) -> f32 {
    let a = outlines[i];
    let m = outlines[i + 1u];
    let bits = bitcast<u32>(a.w);
    let l = a.xyz;
    let dotl = (n.x * l.x + n.y * l.y) + n.z * l.z;
    var d = dotl;
    if (bits & 1u) == 0u {
        d = -dotl;
    }
    let s0 = max(d + 0.0, 0.0);
    var lum = s0 * m.y + m.x;
    let squarings = (bits >> 8u) & 255u;
    if squarings != 255u {
        var b = d;
        let base = (bits >> 1u) & 3u;
        if base == 1u {
            b = s0;
        } else if base == 2u {
            b = dotl;
        }
        var sp = max((b * 2.0) * n.z - l.z, 0.0);
        for (var k = 0u; k < squarings; k++) {
            sp = sp * sp;
        }
        lum += sp * m.z;
    }
    return lum + m.w;
}

// raster.rs `pixel_luma`: the lighting of the normalised normal `n`, or
// the interpolated luma `luma` where `n` is (nearly) zero.
fn pixel_luma(i: u32, luma: f32, n: vec3f) -> f32 {
    let len = sqrt(dot(n, n));
    if !(len >= 1e-6) {
        return luma;
    }
    return lamp_eval(i, n / len);
}

// A lit polygon's luma (0-255, before truncation) at the centre of pixel
// `px`, as the software renderer's `lit_lumas`: luma/z and 1/z (and per
// pixel the normal/z) on the outline's edges at the row's centre, then
// across the span, divided. For a pixel the GPU covers and the software
// doesn't (at an edge), the row is moved into the outline and the span's t
// clamped to 0..1. < 0: not lit.
fn lit_luma(poly: u32, px: vec2f) -> f32 {
    let d = bitcast<vec4u>(outlines[poly]);
    if d.z == 0u {
        return -1.0;
    }
    let pixel = (d.z & 2u) != 0u;
    let lamp = d.w + d.y;
    var c = outline_cross(d.x, d.y, px.y + 0.5);
    if !c.ok {
        var lo = 3.4e38;
        var hi = -3.4e38;
        for (var k = 0u; k < d.y; k++) {
            lo = min(lo, outlines[d.x + k].y);
            hi = max(hi, outlines[d.x + k].y);
        }
        let m = 1e-3 * max(hi - lo, 1e-3);
        c = outline_cross(d.x, d.y, clamp(px.y + 0.5, lo + m, hi - m));
        if !c.ok {
            // No span at all (a polygon thinner than that): the corners'.
            var p = vec4f(0.0);
            var n = vec4f(0.0);
            for (var k = 0u; k < d.y; k++) {
                p += outlines[d.x + k];
                if pixel {
                    n += outlines[d.w + k];
                }
            }
            if pixel {
                return pixel_luma(lamp, p.w / p.z, n.xyz / p.z);
            }
            return p.w / p.z;
        }
    }
    let s = clamp(((px.x + 0.5) - c.lx) / max(c.rx - c.lx, 1e-6), 0.0, 1.0);
    let p = cross_at(d.x, c, s);
    if pixel {
        let n = cross_at(d.w, c, s);
        return pixel_luma(lamp, p.w / p.z, n.xyz / p.z);
    }
    return p.w / p.z;
}

// Polygon `i` as drawn at pixel `px`: `polys[i]`, with a smooth-shaded
// polygon's luma there (truncated, as light::quantise, then clamped to its
// range).
fn poly_px(i: u32, px: vec2f) -> Poly {
    var p = polys[i];
    let l = lit_luma(i, px);
    lit_fine = -1.0;
    toon_dark = 0.0;
    if l >= 0.0 {
        let flags = bitcast<u32>(outlines[i].z);
        lit_range = vec2u((flags >> 8u) & 255u, (flags >> 16u) & 255u);
        // Toon lighting (look.rs): the polygon's steps (flag bits 3-6: 0
        // follow the look, 1 none, else the count) or the look's.
        let own = (flags >> 3u) & 15u;
        var steps = u32(screen.toon.x);
        if own == 1u {
            steps = 0u;
        } else if own > 1u {
            steps = own;
        }
        if steps >= 2u {
            let n = f32(steps);
            let t = clamp((l / 255.0 - screen.toon.z) / (1.0 - screen.toon.z), 0.0, 0.9999);
            let k = floor(t * n);
            let level = screen.toon.y + (1.0 - screen.toon.y) * k / (n - 1.0);
            p.luma = clamp(u32(level * 255.0), lit_range.x, lit_range.y);
            if k == 0.0 {
                toon_dark = 1.0;
                toon_px = px;
            }
        } else {
            p.luma = clamp(u32(clamp(l, 0.0, 255.0)), lit_range.x, lit_range.y);
            if (flags & 4u) != 0u {
                lit_fine = max(l, 0.0);
            }
        }
    }
    return p;
}

// Toon lighting's halftone: 1 in the darkest band of a toon-lit pixel
// (set by `poly_px`), and where the pixel is.
var<private> toon_dark: f32 = 0.0;
var<private> toon_px: vec2f = vec2f(0.0);

// The darkest band's halftone: a rotated grid of dots of ink, a dot's area
// the coverage asked for, in screen space (so it doesn't swim with the
// surface). Anything else is returned as it is.
fn toon_ink(rgb: vec3f) -> vec3f {
    if toon_dark == 0.0 || screen.half.x <= 0.0 {
        return rgb;
    }
    let c = cos(screen.half.z);
    let s = sin(screen.half.z);
    let q = toon_px + 0.5;
    let r = vec2f(q.x * c + q.y * s, -q.x * s + q.y * c) / screen.half.y;
    let d = length(fract(r) - 0.5);
    let radius = sqrt(screen.half.x / 3.14159265);
    let aa = 0.75 / screen.half.y;
    return mix(rgb, screen.ink.xyz, smoothstep(radius + aa, radius - aa, d));
}

// Finer levels (outline flags bit 2; raster.rs `fine_level`): the pixel's
// luma before truncation, or < 0 (the game's levels), and its polygon's
// luma range. Set by `poly_px`.
var<private> lit_fine: f32 = -1.0;
var<private> lit_range: vec2u = vec2u(0u, 255u);

// raster.rs `fine_level`: the columns k, k1 to blend (in x, y) and the
// weight of k1 (in z, as bits) for luma table value `v` (textured) or the
// luma alone (untextured), at `lit_fine` within `lit_range`.
fn fine_level(textured: bool, v: u32) -> vec3u {
    var a: u32;
    var b: u32;
    var y: f32;
    if textured {
        a = min(v * lit_range.x / 256u, 63u);
        b = min(v * lit_range.y / 256u, 63u);
        y = f32(v) * lit_fine / 256.0 - 0.5;
    } else {
        a = min(lit_range.x >> 2u, 63u);
        b = min(lit_range.y >> 2u, 63u);
        y = lit_fine / 4.0 - 0.5;
    }
    let x = clamp(y, f32(a), f32(b));
    let k = min(u32(floor(x)), b);
    return vec3u(k, min(k + 1u, b), bitcast<u32>(x - f32(k)));
}

// raster.rs `fine_columns`: the columns `k` (`fine_level`) kept off pure
// black for colour `c`, whose game's column is `game`.
fn fine_columns(c: vec3u, game: u32, k: vec3u) -> vec3u {
    if fine_black(c, game) {
        return vec3u(game, game, 0u);
    }
    if fine_black(c, k.x) {
        return vec3u(k.y, k.y, 0u);
    }
    if fine_black(c, k.y) {
        return vec3u(k.x, k.x, 0u);
    }
    return k;
}

// Whether column `k` of colour `c` is pure black.
fn fine_black(c: vec3u, k: u32) -> bool {
    return translate(0u, c.x, k) == 0u && translate(1u, c.y, k) == 0u && translate(2u, c.z, k) == 0u;
}

// raster.rs `fine_translate`: channel `ch` of component `c` blended
// between the columns of `k` (`fine_level`), before rounding.
fn fine_translate(ch: u32, c: u32, k: vec3u) -> f32 {
    let a = f32(translate(ch, c, k.x));
    let b = f32(translate(ch, c, k.y));
    return a + (b - a) * bitcast<f32>(k.z);
}

// `pack_lit` of a lit pixel: with finer levels the original pipeline's
// colour at the pixel's luma is the blend of two columns (raster.rs
// `Light::at`).
fn pack_lit_l(p: Poly, rgb: vec3f, u: f32, v: f32) -> vec3f {
    if lit_fine < 0.0 {
        return pack_lit(p, rgb, u, v);
    }
    let c = p.colour.xyz;
    var fb: vec3f;
    for (var ch = 0u; ch < 3u; ch++) {
        let full = translate(ch, c[ch], 63u);
        if full > 0u {
            fb[ch] = fine_translate(ch, c[ch], fine_columns(c, min(64u * p.luma / 256u, 63u), fine_level(true, 64u))) / f32(full);
        } else {
            fb[ch] = f32(p.luma) / 255.0;
        }
    }
    let t8 = filtered_texel(p, u, v);
    if t8 < 0 {
        return rgb * fb;
    }
    let l = luma(p.luma_table, u32(t8) / 2u);
    let lf = min(l * 255u / 256u, 63u);
    let k = fine_columns(c, min(l * p.luma / 256u, 63u), fine_level(true, l));
    var r: vec3f;
    for (var ch = 0u; ch < 3u; ch++) {
        let full = translate(ch, c[ch], lf);
        let here = fine_translate(ch, c[ch], k);
        if full == 0u {
            r[ch] = rgb[ch] * fb[ch] + here;
        } else {
            r[ch] = rgb[ch] * (here / f32(full));
        }
    }
    return r;
}

// `shade` of polygon `p`.
fn shade_l(in: VsOut, p: Poly) -> vec4f {
    let px = floor(in.pos.xy);
    if px.y < p.clip.y || px.y >= p.clip.w || px.x + 0.5 < p.clip.x || px.x + 0.5 >= p.clip.z {
        discard;
    }
    let g = vec2u(floor(px / screen.scale));
    if (p.flags & 16u) != 0u && ((g.x ^ g.y) & 1u) != 0u {
        discard;
    }
    var lum: u32;
    var v = 0u;
    let textured = (p.flags & 1u) != 0u;
    if textured {
        let iz = in.attr.x;
        let t8 = filtered_texel(p, in.attr.y / iz, in.attr.z / iz);
        if t8 < 0 {
            discard;
        }
        v = luma(p.luma_table, u32(t8) / 2u);
        lum = min(v * p.luma / 256u, 63u);
    } else {
        lum = p.luma >> 2u;
    }
    if lit_fine >= 0.0 {
        // Finer levels: the blend, rounded as the software (Rust's round(),
        // never negative here).
        let k = fine_columns(p.colour.xyz, lum, fine_level(textured, v));
        let f = vec3f(fine_translate(0u, p.colour.x, k), fine_translate(1u, p.colour.y, k), fine_translate(2u, p.colour.z, k));
        return vec4f(floor(f + 0.5) / 255.0, 1.0);
    }
    let rgb = vec3u(translate(0u, p.colour.x, lum), translate(1u, p.colour.y, lum), translate(2u, p.colour.z, lum));
    return vec4f(toon_ink(vec3f(rgb) / 255.0), 1.0);
}

// `shade_pack` of polygon `p`.
fn shade_pack_l(in: VsOut, p: Poly) -> vec4f {
    if (p.flags & 32u) == 0u {
        return shade_l(in, p);
    }
    let px = floor(in.pos.xy);
    if px.y < p.clip.y || px.y >= p.clip.w || px.x + 0.5 < p.clip.x || px.x + 0.5 >= p.clip.z {
        discard;
    }
    let g = vec2u(floor(px / screen.scale));
    if (p.flags & 16u) != 0u && ((g.x ^ g.y) & 1u) != 0u {
        discard;
    }
    let iz = in.attr.x;
    let u = in.attr.y / iz;
    let v = in.attr.z / iz;
    let dim = vec2f(textureDimensions(pack_tex, 0));
    let s = pack_sample(p, u * dim.x / f32(p.tex.z), v * dim.y / f32(p.tex.w));
    if s.a < 0.5 {
        discard;
    }
    let rgb = min(floor(pack_lit_l(p, s.rgb, u, v) + 0.5), vec3f(255.0));
    return vec4f(toon_ink(rgb / 255.0), 1.0);
}

// `shade_packf` of polygon `p`.
fn shade_packf_l(in: VsPackOut, p: Poly, gx: vec3f, gy: vec3f) -> vec4f {
    if (p.flags & 32u) == 0u {
        var o: VsOut;
        o.pos = in.pos;
        o.attr = in.attr;
        o.poly = in.poly;
        return shade_l(o, p);
    }
    let px = floor(in.pos.xy);
    if px.y < p.clip.y || px.y >= p.clip.w || px.x + 0.5 < p.clip.x || px.x + 0.5 >= p.clip.z {
        discard;
    }
    let g = vec2u(floor(px / screen.scale));
    if (p.flags & 16u) != 0u && ((g.x ^ g.y) & 1u) != 0u {
        discard;
    }
    let iz = in.attr.x;
    let u = in.attr.y / iz;
    let v = in.attr.z / iz;
    let size = vec2f(f32(p.tex.z), f32(p.tex.w));
    let ddx = vec2f(gx.y - u * gx.x, gx.z - v * gx.x) / (iz * size);
    let ddy = vec2f(gy.y - u * gy.x, gy.z - v * gy.x) / (iz * size);
    let c = textureSampleGrad(pack_tex, pack_smp, vec2f(u, v) / size, ddx, ddy);
    if c.a < 0.5 {
        discard;
    }
    let rgb = min(floor(pack_lit_l(p, c.rgb / c.a * 255.0, u, v) + 0.5), vec3f(255.0));
    return vec4f(toon_ink(rgb / 255.0), 1.0);
}

@fragment
fn fs_l(in: VsOut) -> @location(0) vec4f {
    return shade_l(in, poly_px(in.poly, floor(in.pos.xy)));
}

@fragment
fn fs_depth_l(in: VsOut) -> DepthOut {
    var o: DepthOut;
    o.colour = shade_l(in, poly_px(in.poly, floor(in.pos.xy)));
    o.depth = depth_at(in.poly, in.pos, in.attr.x);
    return o;
}

@fragment
fn fs_pack_l(in: VsOut) -> @location(0) vec4f {
    return shade_pack_l(in, poly_px(in.poly, floor(in.pos.xy)));
}

@fragment
fn fs_pack_depth_l(in: VsOut) -> DepthOut {
    var o: DepthOut;
    o.colour = shade_pack_l(in, poly_px(in.poly, floor(in.pos.xy)));
    o.depth = depth_at(in.poly, in.pos, in.attr.x);
    return o;
}

@fragment
fn fs_packf_l(in: VsPackOut) -> @location(0) vec4f {
    let gx = dpdx(in.attr_c);
    let gy = dpdy(in.attr_c);
    return shade_packf_l(in, poly_px(in.poly, floor(in.pos.xy)), gx, gy);
}

@fragment
fn fs_packf_depth_l(in: VsPackOut) -> DepthOut {
    let gx = dpdx(in.attr_c);
    let gy = dpdy(in.attr_c);
    var o: DepthOut;
    o.colour = shade_packf_l(in, poly_px(in.poly, floor(in.pos.xy)), gx, gy);
    o.depth = depth_at(in.poly, in.pos, in.attr.x);
    return o;
}

// Temporal with smooth shading: the luma at the unjittered sample point
// (a widened thin polygon's pixels outside it take its edge's), drawn at a
// thin polygon's coverage as `fs_t` is.

@fragment
fn fs_l_t(in: VsTOut) -> TOut {
    let a = pre_t(in);
    var o: TOut;
    o.colour = shade_l(plain_t(in), poly_px(in.poly, sample_t(in.pos) - 0.5)) * (a * cover_t(in.poly));
    o.aux = aux_t(in, a < 1.0);
    return o;
}

@fragment
fn fs_depth_l_t(in: VsTOut) -> TDepthOut {
    let a = pre_t(in);
    var o: TDepthOut;
    o.colour = shade_l(plain_t(in), poly_px(in.poly, sample_t(in.pos) - 0.5)) * (a * cover_t(in.poly));
    o.aux = aux_t(in, a < 1.0);
    o.depth = depth_t(in);
    return o;
}

@fragment
fn fs_pack_l_t(in: VsTOut) -> TOut {
    let a = pre_t(in);
    var o: TOut;
    o.colour = shade_pack_l(plain_t(in), poly_px(in.poly, sample_t(in.pos) - 0.5)) * (a * cover_t(in.poly));
    o.aux = aux_t(in, a < 1.0);
    return o;
}

@fragment
fn fs_pack_depth_l_t(in: VsTOut) -> TDepthOut {
    let a = pre_t(in);
    var o: TDepthOut;
    o.colour = shade_pack_l(plain_t(in), poly_px(in.poly, sample_t(in.pos) - 0.5)) * (a * cover_t(in.poly));
    o.aux = aux_t(in, a < 1.0);
    o.depth = depth_t(in);
    return o;
}

@fragment
fn fs_packf_l_t(in: VsTOut) -> TOut {
    let gx = dpdx(in.attr_c) * screen_t.lod;
    let gy = dpdy(in.attr_c) * screen_t.lod;
    let a = pre_t(in);
    var o: TOut;
    o.colour = shade_packf_l(packf_t(in), poly_px(in.poly, sample_t(in.pos) - 0.5), gx, gy) * (a * cover_t(in.poly));
    o.aux = aux_t(in, a < 1.0);
    return o;
}

@fragment
fn fs_packf_depth_l_t(in: VsTOut) -> TDepthOut {
    let gx = dpdx(in.attr_c) * screen_t.lod;
    let gy = dpdy(in.attr_c) * screen_t.lod;
    let a = pre_t(in);
    var o: TDepthOut;
    o.colour = shade_packf_l(packf_t(in), poly_px(in.poly, sample_t(in.pos) - 0.5), gx, gy) * (a * cover_t(in.poly));
    o.aux = aux_t(in, a < 1.0);
    o.depth = depth_t(in);
    return o;
}

// Material maps (docs/texture-packs.md, "How material maps light";
// model2::raster `material_luma`, `material_normal`, `relieved`): the
// `fs_mat*` entry points, for frames where a drawn polygon has a material
// and a lamp (Variant::Mat, MatFiltered). Group 1 holds the polygon's image
// (or a 1x1 stand-in) in binding 0, with `fs_matf*` its sampler in binding
// 1, and its material in binding 2: nx, ny (stored 128 + 127 x, y, along
// increasing u and v), specular, gloss. Poly flag bit 9 (512): the polygon
// has a material bound; bit 10 (1024): its normal is interpolated from its
// corners (`count` entries of normal/z at the directory's `normals`). Its
// material block follows those normals and (lit per pixel) its two lamp
// entries: (n, out), (T, h), the lamp as `lamp_eval` reads it with offset
// 0, and (normal scale, relief strength (0: the faithful rule), bits(range
// lo | hi << 8), 0). A polygon without bit 9 draws exactly as the `_l`
// entry points draw it (as the plain ones, unlit); the functions above are
// unchanged.

@group(1) @binding(2) var mat_tex: texture_2d<f32>;

// The relief rule's factor for this pixel (< 0: none), set by `mat_px`;
// the luma it scales is `lit_fine`, per texel (`mat_at`).
var<private> mat_rel: f32 = -1.0;

// How `mat_px` left the pixel: 0 drawn as without materials, unlit; 1 the
// same, lit (smooth shading); 2 changed by its material. A pixel the
// material doesn't change is drawn by the very functions the entry points
// without materials call (`shade_pack`, `shade_pack_l`, ...), so that the
// shader compiler can't round it otherwise.
var<private> mat_state: u32 = 0u;

// Where polygon `i`'s material block is in `outlines`.
fn mat_index(i: u32) -> u32 {
    let d = bitcast<vec4u>(outlines[i]);
    var k = d.w;
    // Corner normals: interpolated for the material, or lit per pixel.
    if (polys[i].flags & 1024u) != 0u || (d.z & 2u) != 0u {
        k += d.y;
    }
    if (d.z & 2u) != 0u {
        k += 2u;
    }
    return k;
}

// Whether `c` (a whole number) is odd.
fn mat_odd(c: f32) -> bool {
    return c - 2.0 * floor(c / 2.0) == 1.0;
}

// The material's mirror signs: on a mirrored axis the image runs backwards
// in every second repeat, so the component along it changes sign there
// (`s`, `t`: the texture coordinates over the texture's size).
fn mat_mirror(p: Poly, n: vec2f, s: f32, t: f32) -> vec2f {
    var r = n;
    if (p.flags & 2u) != 0u && mat_odd(floor(s)) {
        r.x = -r.x;
    }
    if (p.flags & 4u) != 0u && mat_odd(floor(t)) {
        r.y = -r.y;
    }
    return r;
}

// enhance::Material::sample_normal at level 0 (tex_filter off): bilinear
// over the stored bytes with the polygon's addressing, at texture
// coordinates (u, v) in texels.
fn mat_normal(p: Poly, u: f32, v: f32) -> vec2f {
    let dim = textureDimensions(mat_tex, 0);
    let s = u / f32(p.tex.z);
    let t = v / f32(p.tex.w);
    let x = s * f32(dim.x) - 0.5;
    let y = t * f32(dim.y) - 0.5;
    let fx = x - floor(x);
    let fy = y - floor(y);
    let x0 = i32(floor(x));
    let y0 = i32(floor(y));
    let mx = (p.flags & 2u) != 0u;
    let my = (p.flags & 4u) != 0u;
    var n = vec2f(0.0);
    for (var k = 0; k < 4; k++) {
        var w: f32;
        switch k {
            case 0: { w = (1.0 - fx) * (1.0 - fy); }
            case 1: { w = fx * (1.0 - fy); }
            case 2: { w = (1.0 - fx) * fy; }
            default: { w = fx * fy; }
        }
        let px = wrap(x0 + (k & 1), dim.x, mx);
        let py = wrap(y0 + (k >> 1), dim.y, my);
        let b = round(textureLoad(mat_tex, vec2u(px, py), 0).xy * 255.0);
        n += w * (b - 128.0);
    }
    return mat_mirror(p, clamp(n / 127.0, vec2f(-1.0), vec2f(1.0)), s, t);
}

// The material filtered as the image is (tex_filter mip or anisotropic):
// its mip chain through the image's sampler, with the gradients `ddx`,
// `ddy` of (u, v) over the texture's size. The filtered bytes are kept to
// 1/256 of a step, so a flat map stays exactly flat.
fn mat_normal_f(p: Poly, u: f32, v: f32, ddx: vec2f, ddy: vec2f) -> vec2f {
    let size = vec2f(f32(p.tex.z), f32(p.tex.w));
    let c = textureSampleGrad(mat_tex, pack_smp, vec2f(u, v) / size, ddx, ddy);
    let b = round(c.xy * (255.0 * 256.0)) / 256.0;
    return mat_mirror(p, clamp((b - 128.0) / 127.0, vec2f(-1.0), vec2f(1.0)), u / size.x, v / size.y);
}

// `v` normalised (w 1), or zero where it is shorter than 1e-6 or not finite
// (raster.rs `material_luma`'s `unit`).
fn mat_unit(v: vec3f) -> vec4f {
    let len = sqrt((v.x * v.x + v.y * v.y) + v.z * v.z);
    if !(len >= 1e-6) {
        return vec4f(0.0);
    }
    return vec4f(v / len, 1.0);
}

// Polygon `i`'s normal as drawn at pixel `px` (any length): its corners'
// interpolated as `lit_luma` interpolates them (flag bit 10), else the
// face's (material block `k`).
fn mat_drawn_normal(i: u32, px: vec2f, k: u32) -> vec3f {
    if (polys[i].flags & 1024u) == 0u {
        return outlines[k].xyz;
    }
    let d = bitcast<vec4u>(outlines[i]);
    var c = outline_cross(d.x, d.y, px.y + 0.5);
    if !c.ok {
        var lo = 3.4e38;
        var hi = -3.4e38;
        for (var j = 0u; j < d.y; j++) {
            lo = min(lo, outlines[d.x + j].y);
            hi = max(hi, outlines[d.x + j].y);
        }
        let m = 1e-3 * max(hi - lo, 1e-3);
        c = outline_cross(d.x, d.y, clamp(px.y + 0.5, lo + m, hi - m));
        if !c.ok {
            var p = vec4f(0.0);
            var n = vec4f(0.0);
            for (var j = 0u; j < d.y; j++) {
                p += outlines[d.x + j];
                n += outlines[d.w + j];
            }
            return n.xyz / p.z;
        }
    }
    let s = clamp(((px.x + 0.5) - c.lx) / max(c.rx - c.lx, 1e-6), 0.0, 1.0);
    let p = cross_at(d.x, c, s);
    return cross_at(d.w, c, s).xyz / p.z;
}

// raster.rs `material_luma` for polygon `i` at pixel `px`, the map's normal
// there `xy` (mirror signs applied): (luma, factor, 1) where the material
// changes the pixel: the faithful rule's luma and -1, or the flat luma and
// the relief rule's factor; z 0 where it changes nothing (a flat texel, no
// normal, the same value), and the pixel is drawn as without it.
fn mat_luma(i: u32, px: vec2f, xy: vec2f) -> vec3f {
    let none = vec3f(0.0, -1.0, 0.0);
    let k = mat_index(i);
    let m0 = outlines[k];
    let m1 = outlines[k + 1u];
    let m4 = outlines[k + 4u];
    let kx = m4.x * xy.x;
    let ky = m4.x * xy.y;
    if kx == 0.0 && ky == 0.0 {
        return none;
    }
    let nu = mat_unit(mat_drawn_normal(i, px, k));
    let tu = mat_unit(vec3f(kx, ky, sqrt(max(1.0 - xy.x * xy.x - xy.y * xy.y, 0.0))));
    if nu.w == 0.0 || tu.w == 0.0 {
        return none;
    }
    let n = nu.xyz;
    let t = tu.xyz;
    // Toward the eye; the frame perpendicular to it.
    let o = n * m0.w;
    let ot = (o.x * m1.x + o.y * m1.y) + o.z * m1.z;
    let tpu = mat_unit(m1.xyz - o * ot);
    if tpu.w == 0.0 {
        return none;
    }
    let tp = tpu.xyz;
    let bp = vec3f(o.y * tp.z - o.z * tp.y, o.z * tp.x - o.x * tp.z, o.x * tp.y - o.y * tp.x) * m1.w;
    let o2 = mat_unit(t.z * o + t.x * tp + t.y * bp);
    if o2.w == 0.0 {
        return none;
    }
    let n2 = o2.xyz * m0.w;
    // The pixel's luma without the material: the lit value, or its own.
    var base = f32(polys[i].luma) + 0.5;
    if bitcast<u32>(outlines[i].z) != 0u {
        base = lit_luma(i, px);
    }
    if m4.y == 0.0 {
        // Faithful: the game's sum at the mapped normal.
        let e0 = clamp(lamp_eval(k + 2u, n), 0.0, 255.0);
        let e1 = clamp(lamp_eval(k + 2u, n2), 0.0, 255.0);
        if e1 == e0 {
            return none;
        }
        return vec3f(base + (e1 - e0), -1.0, 1.0);
    }
    // Relief: the change in diffuse light, relative.
    if outlines[k + 3u].y == 0.0 {
        return none;
    }
    let l = outlines[k + 2u];
    var d0 = (n.x * l.x + n.y * l.y) + n.z * l.z;
    var d1 = (n2.x * l.x + n2.y * l.y) + n2.z * l.z;
    if (bitcast<u32>(l.w) & 1u) == 0u {
        d0 = -d0;
        d1 = -d1;
    }
    let dd = max(d1, 0.0) - max(d0, 0.0);
    if dd == 0.0 {
        return none;
    }
    // At least a quarter (model2::raster::RELIEF_MIN), at most fourfold.
    let f = max(1.0 + m4.y * dd, 0.25);
    if f == 1.0 {
        return none;
    }
    return vec3f(base, min(f, 4.0), 1.0);
}

// Polygon `i` as drawn at pixel `px` with its material (map normal `xy`):
// `poly_px`'s, then where the material changes the pixel its luma, with
// `lit_fine`, `lit_range` and `mat_rel` as raster.rs's `PxLuma` has them:
// the two-column blend always, clamped to the material's range.
fn mat_px(i: u32, px: vec2f, xy: vec2f) -> Poly {
    var p = poly_px(i, px);
    mat_rel = -1.0;
    mat_state = select(0u, 1u, bitcast<u32>(outlines[i].z) != 0u);
    if (p.flags & 512u) == 0u {
        return p;
    }
    let m = mat_luma(i, px, xy);
    if m.z == 0.0 {
        return p;
    }
    mat_state = 2u;
    let r = bitcast<u32>(outlines[mat_index(i) + 4u].z);
    lit_range = vec2u(r & 255u, (r >> 8u) & 255u);
    p.luma = clamp(u32(clamp(m.x, 0.0, 255.0)), lit_range.x, lit_range.y);
    lit_fine = max(m.x, 0.0);
    mat_rel = m.y;
    return p;
}

// The map normal of polygon `i` at the fragment (tex_filter off), or zero
// without a material.
fn mat_xy(i: u32, attr: vec3f) -> vec2f {
    let p = polys[i];
    if (p.flags & 512u) == 0u {
        return vec2f(0.0);
    }
    return mat_normal(p, attr.y / attr.x, attr.z / attr.x);
}

// The same filtered, with the screen derivatives `gx`, `gy` of (1/z, u/z,
// v/z) as `shade_packf` takes them.
fn mat_xy_f(i: u32, attr: vec3f, gx: vec3f, gy: vec3f) -> vec2f {
    let p = polys[i];
    if (p.flags & 512u) == 0u {
        return vec2f(0.0);
    }
    let iz = attr.x;
    let u = attr.y / iz;
    let v = attr.z / iz;
    let size = vec2f(f32(p.tex.z), f32(p.tex.w));
    let ddx = vec2f(gx.y - u * gx.x, gx.z - v * gx.x) / (iz * size);
    let ddy = vec2f(gy.y - u * gy.x, gy.z - v * gy.x) / (iz * size);
    return mat_normal_f(p, u, v, ddx, ddy);
}

// raster.rs `relieved` for a texel of luma table value `v`: `l` lowered to
// where that texel's top column starts (at most the column of luma `hi`),
// times `f`; pure black stays black.
fn mat_relieved(l: f32, v: u32, hi: u32, f: f32) -> f32 {
    if v == 0u || !(l >= 1.0) {
        return l;
    }
    let top = 256.0 * (f32(min(v * hi / 256u, 63u)) + 0.5) / f32(v);
    return clamp(min(min(l, top), 255.0) * f, 0.0, 255.0);
}

// raster.rs `PxLuma::at`: the luma and finer level (< 0: the game's
// levels) a texel of luma table value `v` is drawn at, for polygon luma
// `luma`: with the relief rule, scaled per texel.
fn mat_at(v: u32, luma: u32) -> vec2f {
    if mat_rel < 0.0 {
        return vec2f(f32(luma), lit_fine);
    }
    let l = mat_relieved(lit_fine, v, lit_range.y, mat_rel);
    return vec2f(f32(clamp(u32(clamp(l, 0.0, 255.0)), lit_range.x, lit_range.y)), l);
}

// `fine_level` at finer level `lf`.
fn fine_level_at(textured: bool, v: u32, lf: f32) -> vec3u {
    var a: u32;
    var b: u32;
    var y: f32;
    if textured {
        a = min(v * lit_range.x / 256u, 63u);
        b = min(v * lit_range.y / 256u, 63u);
        y = f32(v) * lf / 256.0 - 0.5;
    } else {
        a = min(lit_range.x >> 2u, 63u);
        b = min(lit_range.y >> 2u, 63u);
        y = lf / 4.0 - 0.5;
    }
    let x = clamp(y, f32(a), f32(b));
    let k = min(u32(floor(x)), b);
    return vec3u(k, min(k + 1u, b), bitcast<u32>(x - f32(k)));
}

// `pack_lit_l`, with the relief rule per texel (`mat_at`).
fn pack_lit_m(p: Poly, rgb: vec3f, u: f32, v: f32) -> vec3f {
    if lit_fine < 0.0 {
        return pack_lit(p, rgb, u, v);
    }
    let c = p.colour.xyz;
    let a64 = mat_at(64u, p.luma);
    let k64 = fine_columns(c, min(64u * u32(a64.x) / 256u, 63u), fine_level_at(true, 64u, a64.y));
    var fb: vec3f;
    for (var ch = 0u; ch < 3u; ch++) {
        let full = translate(ch, c[ch], 63u);
        if full > 0u {
            fb[ch] = fine_translate(ch, c[ch], k64) / f32(full);
        } else {
            fb[ch] = f32(p.luma) / 255.0;
        }
    }
    let t8 = filtered_texel(p, u, v);
    if t8 < 0 {
        return rgb * fb;
    }
    let l = luma(p.luma_table, u32(t8) / 2u);
    let lf = min(l * 255u / 256u, 63u);
    let a = mat_at(l, p.luma);
    let k = fine_columns(c, min(l * u32(a.x) / 256u, 63u), fine_level_at(true, l, a.y));
    var r: vec3f;
    for (var ch = 0u; ch < 3u; ch++) {
        let full = translate(ch, c[ch], lf);
        let here = fine_translate(ch, c[ch], k);
        if full == 0u {
            r[ch] = rgb[ch] * fb[ch] + here;
        } else {
            r[ch] = rgb[ch] * (here / f32(full));
        }
    }
    return r;
}

// `shade_l`, with the relief rule per texel (`mat_at`).
fn shade_m(in: VsOut, p: Poly) -> vec4f {
    let px = floor(in.pos.xy);
    if px.y < p.clip.y || px.y >= p.clip.w || px.x + 0.5 < p.clip.x || px.x + 0.5 >= p.clip.z {
        discard;
    }
    let g = vec2u(floor(px / screen.scale));
    if (p.flags & 16u) != 0u && ((g.x ^ g.y) & 1u) != 0u {
        discard;
    }
    var lum: u32;
    var v = 0u;
    var a = vec2f(f32(p.luma), lit_fine);
    let textured = (p.flags & 1u) != 0u;
    if textured {
        let iz = in.attr.x;
        let t8 = filtered_texel(p, in.attr.y / iz, in.attr.z / iz);
        if t8 < 0 {
            discard;
        }
        v = luma(p.luma_table, u32(t8) / 2u);
        a = mat_at(v, p.luma);
        lum = min(v * u32(a.x) / 256u, 63u);
    } else {
        lum = p.luma >> 2u;
    }
    if a.y >= 0.0 {
        // Finer levels: the blend, rounded as the software.
        let k = fine_columns(p.colour.xyz, lum, fine_level_at(textured, v, a.y));
        let f = vec3f(fine_translate(0u, p.colour.x, k), fine_translate(1u, p.colour.y, k), fine_translate(2u, p.colour.z, k));
        return vec4f(floor(f + 0.5) / 255.0, 1.0);
    }
    let rgb = vec3u(translate(0u, p.colour.x, lum), translate(1u, p.colour.y, lum), translate(2u, p.colour.z, lum));
    return vec4f(vec3f(rgb) / 255.0, 1.0);
}

// `shade_pack_l`, with the relief rule per texel; a pixel the material
// leaves alone exactly as without materials (`mat_state`).
fn shade_pack_m(in: VsOut, p: Poly) -> vec4f {
    if mat_state == 0u {
        return shade_pack(in);
    }
    if mat_state == 1u {
        return shade_pack_l(in, p);
    }
    if (p.flags & 32u) == 0u {
        return shade_m(in, p);
    }
    let px = floor(in.pos.xy);
    if px.y < p.clip.y || px.y >= p.clip.w || px.x + 0.5 < p.clip.x || px.x + 0.5 >= p.clip.z {
        discard;
    }
    let g = vec2u(floor(px / screen.scale));
    if (p.flags & 16u) != 0u && ((g.x ^ g.y) & 1u) != 0u {
        discard;
    }
    let iz = in.attr.x;
    let u = in.attr.y / iz;
    let v = in.attr.z / iz;
    let dim = vec2f(textureDimensions(pack_tex, 0));
    let s = pack_sample(p, u * dim.x / f32(p.tex.z), v * dim.y / f32(p.tex.w));
    if s.a < 0.5 {
        discard;
    }
    let rgb = min(floor(pack_lit_m(p, s.rgb, u, v) + 0.5), vec3f(255.0));
    return vec4f(rgb / 255.0, 1.0);
}

// `shade_packf_l`, with the relief rule per texel; a pixel the material
// leaves alone exactly as without materials.
fn shade_packf_m(in: VsPackOut, p: Poly, gx: vec3f, gy: vec3f) -> vec4f {
    if mat_state == 0u {
        return shade_packf(in, gx, gy);
    }
    if mat_state == 1u {
        return shade_packf_l(in, p, gx, gy);
    }
    if (p.flags & 32u) == 0u {
        var o: VsOut;
        o.pos = in.pos;
        o.attr = in.attr;
        o.poly = in.poly;
        return shade_m(o, p);
    }
    let px = floor(in.pos.xy);
    if px.y < p.clip.y || px.y >= p.clip.w || px.x + 0.5 < p.clip.x || px.x + 0.5 >= p.clip.z {
        discard;
    }
    let g = vec2u(floor(px / screen.scale));
    if (p.flags & 16u) != 0u && ((g.x ^ g.y) & 1u) != 0u {
        discard;
    }
    let iz = in.attr.x;
    let u = in.attr.y / iz;
    let v = in.attr.z / iz;
    let size = vec2f(f32(p.tex.z), f32(p.tex.w));
    let ddx = vec2f(gx.y - u * gx.x, gx.z - v * gx.x) / (iz * size);
    let ddy = vec2f(gy.y - u * gy.x, gy.z - v * gy.x) / (iz * size);
    let c = textureSampleGrad(pack_tex, pack_smp, vec2f(u, v) / size, ddx, ddy);
    if c.a < 0.5 {
        discard;
    }
    let rgb = min(floor(pack_lit_m(p, c.rgb / c.a * 255.0, u, v) + 0.5), vec3f(255.0));
    return vec4f(rgb / 255.0, 1.0);
}

@fragment
fn fs_mat(in: VsOut) -> @location(0) vec4f {
    return shade_pack_m(in, mat_px(in.poly, floor(in.pos.xy), mat_xy(in.poly, in.attr)));
}

@fragment
fn fs_mat_depth(in: VsOut) -> DepthOut {
    var o: DepthOut;
    o.colour = shade_pack_m(in, mat_px(in.poly, floor(in.pos.xy), mat_xy(in.poly, in.attr)));
    o.depth = depth_at(in.poly, in.pos, in.attr.x);
    return o;
}

@fragment
fn fs_matf(in: VsPackOut) -> @location(0) vec4f {
    let gx = dpdx(in.attr_c);
    let gy = dpdy(in.attr_c);
    return shade_packf_m(in, mat_px(in.poly, floor(in.pos.xy), mat_xy_f(in.poly, in.attr, gx, gy)), gx, gy);
}

@fragment
fn fs_matf_depth(in: VsPackOut) -> DepthOut {
    let gx = dpdx(in.attr_c);
    let gy = dpdy(in.attr_c);
    var o: DepthOut;
    o.colour = shade_packf_m(in, mat_px(in.poly, floor(in.pos.xy), mat_xy_f(in.poly, in.attr, gx, gy)), gx, gy);
    o.depth = depth_at(in.poly, in.pos, in.attr.x);
    return o;
}

// Temporal with materials: luma and material at the unjittered sample
// point (as `fs_l_t`), the filtered map's derivatives per output pixel.

@fragment
fn fs_mat_t(in: VsTOut) -> TOut {
    let a = pre_t(in);
    let p = mat_px(in.poly, sample_t(in.pos) - 0.5, mat_xy(in.poly, in.attr));
    var o: TOut;
    o.colour = shade_pack_m(plain_t(in), p) * (a * cover_t(in.poly));
    o.aux = aux_t(in, a < 1.0);
    return o;
}

@fragment
fn fs_mat_depth_t(in: VsTOut) -> TDepthOut {
    let a = pre_t(in);
    let p = mat_px(in.poly, sample_t(in.pos) - 0.5, mat_xy(in.poly, in.attr));
    var o: TDepthOut;
    o.colour = shade_pack_m(plain_t(in), p) * (a * cover_t(in.poly));
    o.aux = aux_t(in, a < 1.0);
    o.depth = depth_t(in);
    return o;
}

@fragment
fn fs_matf_t(in: VsTOut) -> TOut {
    let gx = dpdx(in.attr_c) * screen_t.lod;
    let gy = dpdy(in.attr_c) * screen_t.lod;
    let a = pre_t(in);
    let p = mat_px(in.poly, sample_t(in.pos) - 0.5, mat_xy_f(in.poly, in.attr, gx, gy));
    var o: TOut;
    o.colour = shade_packf_m(packf_t(in), p, gx, gy) * (a * cover_t(in.poly));
    o.aux = aux_t(in, a < 1.0);
    return o;
}

@fragment
fn fs_matf_depth_t(in: VsTOut) -> TDepthOut {
    let gx = dpdx(in.attr_c) * screen_t.lod;
    let gy = dpdy(in.attr_c) * screen_t.lod;
    let a = pre_t(in);
    let p = mat_px(in.poly, sample_t(in.pos) - 0.5, mat_xy_f(in.poly, in.attr, gx, gy));
    var o: TDepthOut;
    o.colour = shade_packf_m(packf_t(in), p, gx, gy) * (a * cover_t(in.poly));
    o.aux = aux_t(in, a < 1.0);
    o.depth = depth_t(in);
    return o;
}

// The look's ID pass (look.rs; `encode_3d` with its ID targets): the same
// polygons, runs and depth rules as the 3D layer's pass, writing what is
// there instead of its colour: the polygon's texture ID and tag (a pair of
// words the CPU left in `outlines` at `screen.toon.w`, one per polygon) and
// its exact 1/z. The shading functions are called for their cut-outs only
// (a polygon's transparent texels and its checkerboard draw nothing here
// either); colours are never used.

struct IdOut {
    @location(0) id: vec4u,
    @location(1) iz: f32,
    @builtin(frag_depth) depth: f32,
}

fn id_out(poly: u32, pos: vec4f, iz: f32) -> IdOut {
    var o: IdOut;
    let e = bitcast<vec4u>(outlines[bitcast<u32>(screen.toon.w) + poly]);
    let z1 = outline_iz(poly, floor(pos.xy), iz);
    let z = z1 * (1.0 + f32(polys[poly].colour.w) * DEPTH_BIAS) * DEPTH_FACTOR;
    // SPIKE: z is the polygon's index + 1 (the lighting passes' table).
    o.id = vec4u(e.xy, poly + 1u, 0u);
    o.iz = z1;
    o.depth = z / (z + DEPTH_SCALE);
    return o;
}

// The cut-outs of the original texture alone (`shade` without its colour):
// the clip, the checkerboard, and a translucent texture's transparent
// texels; a polygon with none of them reads no texel at all.
fn id_cut(in: VsOut) {
    let p = polys[in.poly];
    let px = floor(in.pos.xy);
    if px.y < p.clip.y || px.y >= p.clip.w || px.x + 0.5 < p.clip.x || px.x + 0.5 >= p.clip.z {
        discard;
    }
    let g = vec2u(floor(px / screen.scale));
    if (p.flags & 16u) != 0u && ((g.x ^ g.y) & 1u) != 0u {
        discard;
    }
    if (p.flags & 9u) == 9u {
        let iz = in.attr.x;
        if filtered_texel(p, in.attr.y / iz, in.attr.z / iz) < 0 {
            discard;
        }
    }
}

@fragment
fn fs_id(in: VsOut) -> IdOut {
    id_cut(in);
    return id_out(in.poly, in.pos, in.attr.x);
}

@fragment
fn fs_id_pack(in: VsOut) -> IdOut {
    if (polys[in.poly].flags & 32u) == 0u {
        id_cut(in);
    } else {
        _ = shade_pack(in);
    }
    return id_out(in.poly, in.pos, in.attr.x);
}

@fragment
fn fs_id_packf(in: VsPackOut) -> IdOut {
    // Derivatives in uniform control flow.
    let gx = dpdx(in.attr_c);
    let gy = dpdy(in.attr_c);
    if (polys[in.poly].flags & 32u) == 0u {
        var o: VsOut;
        o.pos = in.pos;
        o.attr = in.attr;
        o.poly = in.poly;
        id_cut(o);
    } else {
        _ = shade_packf(in, gx, gy);
    }
    return id_out(in.poly, in.pos, in.attr.x);
}

// A window's depth reset (as `fs_clear`), for the ID pass's targets.
@fragment
fn fs_id_clear() -> IdOut {
    var o: IdOut;
    o.id = vec4u(0u);
    o.iz = 0.0;
    o.depth = 0.0;
    return o;
}
