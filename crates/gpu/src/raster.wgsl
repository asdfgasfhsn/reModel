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
    // Bit 0 textured, 1 mirror x, 2 mirror y, 3 translucent, 4 checker.
    flags: u32,
    bank: u32,
    luma_table: u32,
    luma: u32,
    // 5-bit colour components; w unused.
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

struct VsOut {
    @builtin(position) pos: vec4f,
    // 1/z, u/z, v/z: linear in screen space, as the software fill does.
    @location(0) @interpolate(linear) attr: vec3f,
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

@fragment
fn fs(in: VsOut) -> @location(0) vec4f {
    let p = polys[in.poly];
    let px = floor(in.pos.xy);
    // The software fill's clip: rows cy0 <= y < cy1, pixel centres
    // cx0 <= x + 0.5 < cx1.
    if px.y < p.clip.y || px.y >= p.clip.w || px.x + 0.5 < p.clip.x || px.x + 0.5 >= p.clip.z {
        discard;
    }
    var lum: u32;
    if (p.flags & 1u) != 0u {
        let g = vec2u(floor(px / screen.scale));
        if (p.flags & 16u) != 0u && ((g.x ^ g.y) & 1u) != 0u {
            discard;
        }
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
