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

// raster.rs `Light`: the replacement stands for the texture at full luma,
// so each channel is scaled by what the original pipeline gives at the
// polygon's luma over what it gives at full luma, for the original texel.
fn pack_light(p: Poly, u: f32, v: f32) -> vec3f {
    if p.luma >= 255u {
        return vec3f(1.0);
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
        return fb;
    }
    let l = luma(p.luma_table, u32(t8) / 2u);
    let lp = min(l * p.luma / 256u, 63u);
    let lf = min(l * 255u / 256u, 63u);
    var r: vec3f;
    for (var ch = 0u; ch < 3u; ch++) {
        let full = translate(ch, c[ch], lf);
        if full == 0u {
            r[ch] = fb[ch];
        } else {
            r[ch] = f32(translate(ch, c[ch], lp)) / f32(full);
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
    let r = pack_light(p, u, v);
    // Rust's round() then clamp, as the software (values are never
    // negative).
    let rgb = min(floor(s.rgb * r + 0.5), vec3f(255.0));
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
    let r = pack_light(p, u, v);
    let rgb = min(floor(c.rgb / c.a * 255.0 * r + 0.5), vec3f(255.0));
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
// motion data (vertex attribute 3).

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
// previous 1/z (0: no motion data). Clamped so it stays finite in 16 bits.
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
    return vec4f(mv, b, e);
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
    o.colour = shade(plain_t(in)) * a;
    o.aux = aux_t(in, a < 1.0);
    return o;
}

@fragment
fn fs_depth_t(in: VsTOut) -> TDepthOut {
    let a = pre_t(in);
    var o: TDepthOut;
    o.colour = shade(plain_t(in)) * a;
    o.aux = aux_t(in, a < 1.0);
    o.depth = depth_t(in);
    return o;
}

@fragment
fn fs_pack_t(in: VsTOut) -> TOut {
    let a = pre_t(in);
    var o: TOut;
    o.colour = shade_pack(plain_t(in)) * a;
    o.aux = aux_t(in, a < 1.0);
    return o;
}

@fragment
fn fs_pack_depth_t(in: VsTOut) -> TDepthOut {
    let a = pre_t(in);
    var o: TDepthOut;
    o.colour = shade_pack(plain_t(in)) * a;
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
    o.colour = shade_packf(packf_t(in), gx, gy) * a;
    o.aux = aux_t(in, a < 1.0);
    return o;
}

@fragment
fn fs_packf_depth_t(in: VsTOut) -> TDepthOut {
    let gx = dpdx(in.attr_c) * screen_t.lod;
    let gy = dpdy(in.attr_c) * screen_t.lod;
    let a = pre_t(in);
    var o: TDepthOut;
    o.colour = shade_packf(packf_t(in), gx, gy) * a;
    o.aux = aux_t(in, a < 1.0);
    o.depth = depth_t(in);
    return o;
}

// Smooth shading (Quality::shading, model2::light): the `_l` entry points,
// for frames with a lit polygon. A lit polygon's outline entry has `lit`
// (z) set and its points carry luma/z in w; its luma is taken per pixel
// from those (`lit_luma`), every other polygon's is its own. The shading
// functions above stay as they were (the frames without a lit polygon draw
// exactly as before); `shade_l`, `shade_pack_l` and `shade_packf_l` are
// them with the polygon passed in.

// The two ends of an outline's span on the row at height `yc`: (x, 1/z,
// luma/z) at each, interpolated along the edges as the software fill does;
// l.x > r.x where the row misses the outline.
struct Span {
    l: vec3f,
    r: vec3f,
}

fn outline_span(first: u32, count: u32, yc: f32) -> Span {
    var s: Span;
    s.l = vec3f(3.4e38, 0.0, 0.0);
    s.r = vec3f(-3.4e38, 0.0, 0.0);
    for (var k = 0u; k < count; k++) {
        let a = outlines[first + k];
        let b = outlines[first + (k + 1u) % count];
        if (a.y <= yc && b.y > yc) || (b.y <= yc && a.y > yc) {
            let t = (yc - a.y) / (b.y - a.y);
            let e = vec3f(a.x + (b.x - a.x) * t, a.z + (b.z - a.z) * t, a.w + (b.w - a.w) * t);
            if e.x < s.l.x {
                s.l = e;
            }
            if e.x > s.r.x {
                s.r = e;
            }
        }
    }
    return s;
}

// A lit polygon's luma (0-255, before truncation) at the centre of pixel
// `px`, as the software renderer's `lit_lumas`: luma/z and 1/z on the
// outline's edges at the row's centre, then across the span, divided. For
// a pixel the GPU covers and the software doesn't (at an edge), the row is
// moved into the outline and the span's t clamped to 0..1. < 0: not lit.
fn lit_luma(poly: u32, px: vec2f) -> f32 {
    let d = bitcast<vec4u>(outlines[poly]);
    if d.z == 0u {
        return -1.0;
    }
    var s = outline_span(d.x, d.y, px.y + 0.5);
    if s.l.x > s.r.x {
        var lo = 3.4e38;
        var hi = -3.4e38;
        for (var k = 0u; k < d.y; k++) {
            lo = min(lo, outlines[d.x + k].y);
            hi = max(hi, outlines[d.x + k].y);
        }
        let m = 1e-3 * max(hi - lo, 1e-3);
        s = outline_span(d.x, d.y, clamp(px.y + 0.5, lo + m, hi - m));
        if s.l.x > s.r.x {
            // No span at all (a polygon thinner than that): the corners'.
            var lz = 0.0;
            var iz = 0.0;
            for (var k = 0u; k < d.y; k++) {
                lz += outlines[d.x + k].w;
                iz += outlines[d.x + k].z;
            }
            return lz / iz;
        }
    }
    let t = clamp(((px.x + 0.5) - s.l.x) / max(s.r.x - s.l.x, 1e-6), 0.0, 1.0);
    let iz = s.l.y + (s.r.y - s.l.y) * t;
    let lz = s.l.z + (s.r.z - s.l.z) * t;
    return lz / iz;
}

// Polygon `i` as drawn at pixel `px`: `polys[i]`, with a smooth-shaded
// polygon's luma there (truncated, as light::quantise).
fn poly_px(i: u32, px: vec2f) -> Poly {
    var p = polys[i];
    let l = lit_luma(i, px);
    if l >= 0.0 {
        p.luma = u32(clamp(l, 0.0, 255.0));
    }
    return p;
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
    let r = pack_light(p, u, v);
    let rgb = min(floor(s.rgb * r + 0.5), vec3f(255.0));
    return vec4f(rgb / 255.0, 1.0);
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
    let r = pack_light(p, u, v);
    let rgb = min(floor(c.rgb / c.a * 255.0 * r + 0.5), vec3f(255.0));
    return vec4f(rgb / 255.0, 1.0);
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

// Temporal with smooth shading: the luma at the unjittered sample point.

@fragment
fn fs_l_t(in: VsTOut) -> TOut {
    let a = pre_t(in);
    var o: TOut;
    o.colour = shade_l(plain_t(in), poly_px(in.poly, sample_t(in.pos) - 0.5)) * a;
    o.aux = aux_t(in, a < 1.0);
    return o;
}

@fragment
fn fs_depth_l_t(in: VsTOut) -> TDepthOut {
    let a = pre_t(in);
    var o: TDepthOut;
    o.colour = shade_l(plain_t(in), poly_px(in.poly, sample_t(in.pos) - 0.5)) * a;
    o.aux = aux_t(in, a < 1.0);
    o.depth = depth_t(in);
    return o;
}

@fragment
fn fs_pack_l_t(in: VsTOut) -> TOut {
    let a = pre_t(in);
    var o: TOut;
    o.colour = shade_pack_l(plain_t(in), poly_px(in.poly, sample_t(in.pos) - 0.5)) * a;
    o.aux = aux_t(in, a < 1.0);
    return o;
}

@fragment
fn fs_pack_depth_l_t(in: VsTOut) -> TDepthOut {
    let a = pre_t(in);
    var o: TDepthOut;
    o.colour = shade_pack_l(plain_t(in), poly_px(in.poly, sample_t(in.pos) - 0.5)) * a;
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
    o.colour = shade_packf_l(packf_t(in), poly_px(in.poly, sample_t(in.pos) - 0.5), gx, gy) * a;
    o.aux = aux_t(in, a < 1.0);
    return o;
}

@fragment
fn fs_packf_depth_l_t(in: VsTOut) -> TDepthOut {
    let gx = dpdx(in.attr_c) * screen_t.lod;
    let gy = dpdy(in.attr_c) * screen_t.lod;
    let a = pre_t(in);
    var o: TDepthOut;
    o.colour = shade_packf_l(packf_t(in), poly_px(in.poly, sample_t(in.pos) - 0.5), gx, gy) * a;
    o.aux = aux_t(in, a < 1.0);
    o.depth = depth_t(in);
    return o;
}
