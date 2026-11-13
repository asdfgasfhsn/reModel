// Final frame: the 2D base layer, the 3D layer and the HUD, blended per
// output pixel as model2::video::compose does on the CPU (see
// `Layers2d` there for the coordinate rules).

struct Params {
    // Output pixels per game pixel.
    scale: vec2f,
    // Game pixels added at each side in wide views.
    pad: f32,
    has_3d: u32,
    ext_w: u32,
    // Columns the base layer has beyond the frame at each side.
    side: u32,
    // Top-left of the frame in the target (letterboxing), in pixels.
    origin: vec2f,
    backdrop: vec4f,
    // Bloom's strength (0: off, nothing is read) and the 3D layer's size in
    // output pixels.
    bloom: f32,
    size: vec2f,
}

@group(0) @binding(0) var<uniform> pr: Params;
@group(0) @binding(1) var base: texture_2d<f32>;
@group(0) @binding(2) var layer3d: texture_2d<f32>;
@group(0) @binding(3) var hud: texture_2d<f32>;
// Half-size blurred bright parts of the 3D layer (gpu/src/bloom.rs).
@group(0) @binding(4) var glow: texture_2d<f32>;
@group(0) @binding(5) var glow_sampler: sampler;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    // One triangle covering the screen.
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4f(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

// The glow added over the picture, before the HUD.
fn add_glow(c: vec3f, pos: vec2f) -> vec3f {
    if pr.bloom <= 0.0 {
        return c;
    }
    return c + pr.bloom * textureSampleLevel(glow, glow_sampler, pos / pr.size, 0.0).rgb;
}

@fragment
fn fs(@builtin(position) frag: vec4f) -> @location(0) vec4f {
    // pos.xy is the pixel centre in the frame, (ox + 0.5, oy + 0.5).
    let pos = vec4f(frag.xy - pr.origin, frag.zw);
    let gy = i32(floor(pos.y / pr.scale.y));
    if gy >= 384 {
        return vec4f(0.0, 0.0, 0.0, 1.0);
    }
    let ex = pos.x / pr.scale.x;
    let bx = i32(floor(ex - pr.pad)) + i32(pr.side);
    var c = vec3f(0.0);
    if bx >= 0 && bx < 496 + 2 * i32(pr.side) {
        c = textureLoad(base, vec2i(bx, gy), 0).rgb;
    } else if pr.has_3d != 0u {
        c = pr.backdrop.rgb;
    }
    // The 3D layer is premultiplied: its alpha is the fraction of the
    // pixel covered (1 or 0 without multisampling), so edges blend with
    // what is behind them.
    let d = textureLoad(layer3d, vec2i(floor(pos.xy)), 0);
    c = d.rgb + (1.0 - d.a) * c;
    c = add_glow(c, pos.xy);
    if i32(ex) < i32(pr.ext_w) {
        let h = textureLoad(hud, vec2i(i32(ex), gy), 0);
        if h.a > 0.0 {
            c = h.rgb;
        }
    }
    return vec4f(c, 1.0);
}

// Replaced tile patterns (texture packs, gpu/src/tiles.rs): the frame's
// tiles per game pixel and their replacements in an atlas, drawn at output
// resolution as model2::video::compose_enhanced does (`TileMap` there).

struct TileParams {
    // Atlas cell side in texels, and cells per row.
    cell: u32,
    per_row: u32,
    pad0: u32,
    pad1: u32,
}

// Per game pixel, planes B and A: palette index (bits 0-11), atlas cell
// (12-23, 0xfff none), row (24-26) and column (27-29) in the tile,
// priority (30), map enabled (31).
@group(1) @binding(0) var hits: texture_2d<u32>;
// Per extended HUD column: the game column its HUD pixel comes from.
@group(1) @binding(1) var hud_src: texture_2d<u32>;
@group(1) @binding(2) var atlas: texture_2d<f32>;
// The palette as RGBA8.
@group(1) @binding(3) var<storage, read> pal: array<u32>;
@group(1) @binding(4) var<uniform> tp: TileParams;

const HIT_PRESENT: u32 = 0x80000000u;
const HIT_PRIORITY: u32 = 0x40000000u;

// Colour of tile pixel `h` at `f` (0-1) within its game pixel; alpha 0 if
// transparent. A replacement's alpha decides transparency (bilinear,
// clamped to the tile, colour weighted by alpha, cut at 0.5), except on the
// opaque back plane, where the original pen shows through.
fn tile_colour(h: u32, f: vec2f, opaque: bool) -> vec4f {
    let cell = (h >> 12u) & 0xfffu;
    if cell != 0xfffu {
        let n = i32(tp.cell);
        let p = (vec2f(f32((h >> 27u) & 7u), f32((h >> 24u) & 7u)) + f) * f32(tp.cell) / 8.0 - 0.5;
        let i0 = vec2i(floor(p));
        let fr = p - floor(p);
        let origin = vec2i(i32(cell % tp.per_row), i32(cell / tp.per_row)) * n;
        var rgb = vec3f(0.0);
        var a = 0.0;
        for (var k = 0; k < 4; k++) {
            let d = vec2i(k & 1, k >> 1u);
            let w = select(1.0 - fr.x, fr.x, d.x == 1) * select(1.0 - fr.y, fr.y, d.y == 1);
            let t = textureLoad(atlas, origin + clamp(i0 + d, vec2i(0), vec2i(n - 1)), 0);
            rgb += w * t.a * t.rgb;
            a += w * t.a;
        }
        if a >= 0.5 {
            return vec4f(rgb / a, 1.0);
        }
        if !opaque {
            return vec4f(0.0);
        }
    }
    if (h & 0xfu) == 0u && !opaque {
        return vec4f(0.0);
    }
    return vec4f(unpack4x8unorm(pal[h & 0xfffu]).rgb, 1.0);
}

@fragment
fn fs_tiles(@builtin(position) frag: vec4f) -> @location(0) vec4f {
    let pos = vec4f(frag.xy - pr.origin, frag.zw);
    let gyf = pos.y / pr.scale.y;
    let gy = i32(floor(gyf));
    if gy >= 384 {
        return vec4f(0.0, 0.0, 0.0, 1.0);
    }
    let fy = gyf - floor(gyf);
    let ex = pos.x / pr.scale.x;
    let gx = ex - pr.pad;
    let bx = i32(floor(gx)) + i32(pr.side);
    var c = vec3f(0.0);
    if gx >= 0.0 && gx < 496.0 {
        // Low-priority tiles over the backdrop: plane B (opaque), then A.
        let hh = textureLoad(hits, vec2i(i32(gx), gy), 0).xy;
        c = pr.backdrop.rgb;
        for (var s = 0; s < 2; s++) {
            let h = hh[s];
            if (h & (HIT_PRESENT | HIT_PRIORITY)) == HIT_PRESENT {
                let t = tile_colour(h, vec2f(gx - floor(gx), fy), s == 0);
                if t.a > 0.0 {
                    c = t.rgb;
                }
            }
        }
    } else if bx >= 0 && bx < 496 + 2 * i32(pr.side) {
        // The sides of a wide view: original patterns.
        c = textureLoad(base, vec2i(bx, gy), 0).rgb;
    } else if pr.has_3d != 0u {
        c = pr.backdrop.rgb;
    }
    let d = textureLoad(layer3d, vec2i(floor(pos.xy)), 0);
    c = d.rgb + (1.0 - d.a) * c;
    c = add_glow(c, pos.xy);
    if i32(ex) < i32(pr.ext_w) {
        let sx = textureLoad(hud_src, vec2i(i32(ex), gy), 0).x;
        if sx != 0xffffffffu {
            // High-priority tiles: pen 0 transparent on both planes.
            let hh = textureLoad(hits, vec2i(i32(sx), gy), 0).xy;
            for (var s = 0; s < 2; s++) {
                let h = hh[s];
                if (h & (HIT_PRESENT | HIT_PRIORITY)) == (HIT_PRESENT | HIT_PRIORITY) {
                    let t = tile_colour(h, vec2f(ex - floor(ex), fy), false);
                    if t.a > 0.0 {
                        c = t.rgb;
                    }
                }
            }
        }
    }
    return vec4f(c, 1.0);
}
