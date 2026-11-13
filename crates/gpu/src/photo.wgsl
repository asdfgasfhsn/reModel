// Photo mode's passes (gpu/src/photo.rs, docs/photo-mode.md): the depth
// of the 3D layer as 1/z, motion blur's sum of images, depth of field and
// the colour look, and the finish (downsampling, chromatic aberration,
// grain, letterbox) into the output.

// Shared by every module (photo.rs adds it to each).

// ---- Full-screen passes. ----

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4f(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

fn hash(p: vec2u, s: u32) -> f32 {
    var h = (p.x * 0x8da6b343u) ^ (p.y * 0xd8163841u) ^ (s * 0xcb1ab31fu);
    h = (h ^ (h >> 16u)) * 0x7feb352du;
    h = (h ^ (h >> 15u)) * 0x846ca68bu;
    h = h ^ (h >> 16u);
    return f32(h) / 4294967296.0;
}

fn luma(c: vec3f) -> f32 {
    return dot(c, vec3f(0.2126, 0.7152, 0.0722));
}

//// split
// ---- Depth: each polygon's 1/z, the nearest kept. ----

struct DepthParams {
    size: vec2f,
    pad: vec2f,
}

@group(0) @binding(0) var<uniform> dp: DepthParams;

struct DepthOut {
    @builtin(position) pos: vec4f,
    @location(0) iz: f32,
}

@vertex
fn vs_depth(@location(0) v: vec3f) -> DepthOut {
    var o: DepthOut;
    // Output pixels to clip space; the depth test's value grows with 1/z
    // (nearer), as model2::raster::depth_value's does.
    let d = v.z / (v.z + 0.1);
    o.pos = vec4f(v.x / dp.size.x * 2.0 - 1.0, 1.0 - v.y / dp.size.y * 2.0, clamp(d, 0.0, 1.0), 1.0);
    o.iz = v.z;
    return o;
}

@fragment
fn fs_depth(i: DepthOut) -> @location(0) vec4f {
    return vec4f(i.iz, 0.0, 0.0, 1.0);
}

//// split
// ---- Motion blur: one image of several, weighted, added to the sum. ----

struct AccumParams {
    weight: f32,
    pad0: f32,
    pad1: f32,
    pad2: f32,
}

@group(0) @binding(0) var<uniform> ap: AccumParams;
@group(0) @binding(1) var accum_src: texture_2d<f32>;

@fragment
fn fs_accum(@builtin(position) p: vec4f) -> @location(0) vec4f {
    return textureLoad(accum_src, vec2i(floor(p.xy)), 0) * ap.weight;
}

//// split
// ---- Depth of field and the look. ----

struct Post {
    // The image's size in pixels.
    size: vec2f,
    // Focus: 1/z of the plane in focus; aperture: the blur radius in
    // pixels of something infinitely far (0: no depth of field); max_coc:
    // the largest blur radius.
    focus_iz: f32,
    aperture: f32,
    max_coc: f32,
    // Bright parts weigh more in the blur (bokeh highlights).
    bokeh: f32,
    // Exposure as a factor, contrast and saturation (1: as is), warmth
    // and tint (0: as is), vignette (0: none).
    exposure: f32,
    contrast: f32,
    saturation: f32,
    warmth: f32,
    tint: f32,
    vignette: f32,
    // 1: lay the HUD (hud where it differs from hudless) over the result.
    hud: u32,
    seed: u32,
    pad0: f32,
    pad1: f32,
}

@group(0) @binding(0) var<uniform> pp: Post;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var iz_tex: texture_2d<f32>;
@group(0) @binding(3) var tiles: texture_2d<f32>;
@group(0) @binding(4) var hud: texture_2d<f32>;
@group(0) @binding(5) var hudless: texture_2d<f32>;

const TILE: i32 = 16;

// Blur radius in pixels for a point at 1/z `iz`: the aperture's radius
// times how far its 1/z is from the focus plane's, relative to that
// (thin lens, focus far beyond the focal length).
fn coc(iz: f32) -> f32 {
    if pp.aperture <= 0.0 {
        return 0.0;
    }
    let f = max(pp.focus_iz, 1e-12);
    return min(pp.aperture * abs(iz - f) / f, pp.max_coc);
}

// The largest blur radius in each TILE x TILE block of the image.
@fragment
fn fs_tile(@builtin(position) p: vec4f) -> @location(0) vec4f {
    let base = vec2i(floor(p.xy)) * TILE;
    let dims = vec2i(textureDimensions(iz_tex));
    var m = 0.0;
    for (var y = 0; y < TILE; y++) {
        for (var x = 0; x < TILE; x++) {
            let q = base + vec2i(x, y);
            if q.x < dims.x && q.y < dims.y {
                m = max(m, coc(textureLoad(iz_tex, q, 0).r));
            }
        }
    }
    return vec4f(m, 0.0, 0.0, 1.0);
}

const SAMPLES: i32 = 96;
const PI: f32 = 3.14159265;

// Depth of field gathered as the scatter it stands for: every pixel within
// the radius spreads its colour over a disc of its own blur radius, as
// much per pixel as the disc is small; one behind this pixel can't spread
// over it further than this pixel's own blur (so a sharp object keeps its
// edge against a blurred background, while a blurred one in front spills
// over what is behind it).
fn depth_of_field(p: vec2i, c0: vec3f) -> vec3f {
    let dims = vec2i(textureDimensions(scene));
    let izc = textureLoad(iz_tex, p, 0).r;
    let cc = coc(izc);
    let t = p / TILE;
    let tdims = vec2i(textureDimensions(tiles));
    var r_max = cc;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let q = clamp(t + vec2i(x, y), vec2i(0), tdims - 1);
            r_max = max(r_max, textureLoad(tiles, q, 0).r);
        }
    }
    if r_max < 0.5 {
        return c0;
    }
    let area = PI * r_max * r_max / f32(SAMPLES);
    let a0 = max(cc, 0.5);
    let w0 = 1.0 / (PI * a0 * a0) * (1.0 + pp.bokeh * max(luma(c0) - 0.6, 0.0) * 6.0);
    var acc = c0 * w0;
    var wsum = w0;
    let rot = hash(vec2u(p), pp.seed) * 6.2831853;
    for (var i = 0; i < SAMPLES; i++) {
        let r = r_max * sqrt((f32(i) + 0.5) / f32(SAMPLES));
        let a = f32(i) * 2.3999632 + rot;
        let q = clamp(p + vec2i(round(r * vec2f(cos(a), sin(a)))), vec2i(0), dims - 1);
        let izs = textureLoad(iz_tex, q, 0).r;
        var cs = coc(izs);
        if izs < izc {
            cs = min(cs, cc);
        }
        let cover = clamp(cs - r + 1.0, 0.0, 1.0);
        if cover <= 0.0 {
            continue;
        }
        let col = textureLoad(scene, q, 0).rgb;
        let s = max(cs, 0.5);
        let w = cover * area / (PI * s * s) * (1.0 + pp.bokeh * max(luma(col) - 0.6, 0.0) * 6.0);
        acc += col * w;
        wsum += w;
    }
    return acc / wsum;
}

@fragment
fn fs_post(@builtin(position) frag: vec4f) -> @location(0) vec4f {
    let p = vec2i(floor(frag.xy));
    var c = textureLoad(scene, p, 0).rgb;
    if pp.aperture > 0.0 {
        c = depth_of_field(p, c);
    }
    // White balance, exposure, contrast about mid grey, saturation.
    c *= vec3f(1.0 + 0.12 * pp.warmth, 1.0 + 0.08 * pp.tint, 1.0 - 0.12 * pp.warmth) * pp.exposure;
    c = (c - 0.5) * pp.contrast + 0.5;
    c = mix(vec3f(luma(c)), c, pp.saturation);
    // Vignette: darker towards the corners.
    let uv = frag.xy / pp.size - 0.5;
    let d = length(uv * vec2f(pp.size.x / pp.size.y, 1.0)) / length(vec2f(0.5 * pp.size.x / pp.size.y, 0.5));
    c *= 1.0 - pp.vignette * smoothstep(0.35, 1.0, d);
    c = max(c, vec3f(0.0));
    if pp.hud != 0u {
        let h = textureLoad(hud, p, 0).rgb;
        let b = textureLoad(hudless, p, 0).rgb;
        if any(abs(h - b) > vec3f(0.5 / 255.0)) {
            c = h;
        }
    }
    return vec4f(c, 1.0);
}

//// split
// ---- Finish: the image down to the output's size, and the film. ----

struct Finish {
    src: vec2f,
    out: vec2f,
    origin: vec2f,
    // Width over height of the letterbox's opening (0: none).
    letterbox: f32,
    grain: f32,
    aberration: f32,
    // Samples a side per output pixel.
    n: u32,
    seed: u32,
    pad: f32,
}

@group(0) @binding(0) var<uniform> fp: Finish;
@group(0) @binding(1) var post: texture_2d<f32>;

fn load_post(q: vec2f) -> vec3f {
    let dims = vec2i(textureDimensions(post));
    return textureLoad(post, clamp(vec2i(floor(q)), vec2i(0), dims - 1), 0).rgb;
}

@fragment
fn fs_finish(@builtin(position) frag: vec4f) -> @location(0) vec4f {
    let o = frag.xy - fp.origin;
    if fp.letterbox > 0.0 {
        let aspect = fp.out.x / fp.out.y;
        if aspect > fp.letterbox {
            let w = fp.out.y * fp.letterbox;
            if abs(o.x - fp.out.x * 0.5) > w * 0.5 {
                return vec4f(0.0, 0.0, 0.0, 1.0);
            }
        } else {
            let h = fp.out.x / fp.letterbox;
            if abs(o.y - fp.out.y * 0.5) > h * 0.5 {
                return vec4f(0.0, 0.0, 0.0, 1.0);
            }
        }
    }
    let scale = fp.src / fp.out;
    let cell = floor(o);
    var c = vec3f(0.0);
    let n = max(fp.n, 1u);
    for (var j = 0u; j < n; j++) {
        for (var i = 0u; i < n; i++) {
            let q = (cell + (vec2f(f32(i), f32(j)) + 0.5) / f32(n)) * scale;
            if fp.aberration > 0.0 {
                // Red outwards, blue inwards, more towards the edges.
                let d = (q / fp.src - 0.5) * fp.aberration * 0.012 * fp.src.y;
                c += vec3f(load_post(q + d).r, load_post(q).g, load_post(q - d).b);
            } else {
                c += load_post(q);
            }
        }
    }
    c /= f32(n * n);
    if fp.grain > 0.0 {
        let g = hash(vec2u(cell), fp.seed) + hash(vec2u(cell) + vec2u(7u, 13u), fp.seed) - 1.0;
        c += g * fp.grain * 0.12 * (1.0 - abs(luma(c) - 0.5));
    }
    return vec4f(clamp(c, vec3f(0.0), vec3f(1.0)), 1.0);
}
