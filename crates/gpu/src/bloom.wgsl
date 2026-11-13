// Bloom (docs/enhancements.md, "Bloom"): the 3D layer's bright parts,
// blurred through a chain of halvings and added back in the compose pass.
//
//   fs_first  the layer to half size, keeping only what is above the
//             threshold (soft knee), per tap before they are averaged
//   fs_down   one level to the next, half size
//   fs_up     one level up into the next larger one, added to it by
//             additive blending, at a fixed weight
//
// A 13-tap downsample and a 3x3 tent upsample, all bilinear taps.

struct Params {
    // Brightness (largest channel, 0-1) above which a colour glows, and the
    // width of the soft knee around it.
    threshold: f32,
    knee: f32,
    // Weight of a level's blur when added to the next larger one.
    up: f32,
    pad: f32,
    // The size of the target being drawn.
    dst: vec2f,
    pad2: vec2f,
}

@group(0) @binding(0) var<uniform> pr: Params;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var smp: sampler;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4f(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

fn tap(uv: vec2f) -> vec3f {
    return textureSampleLevel(src, smp, uv, 0.0).rgb;
}

// What of `c` glows: its excess brightness over the threshold, the
// transition spread over the knee so the glow's edge isn't hard.
fn bright(c: vec3f) -> vec3f {
    let b = max(c.r, max(c.g, c.b));
    let k = clamp(b - pr.threshold + pr.knee, 0.0, 2.0 * pr.knee);
    let soft = k * k / (4.0 * pr.knee + 1e-5);
    let amount = max(soft, b - pr.threshold) / max(b, 1e-5);
    return c * amount;
}

fn down13(uv: vec2f, first: bool) -> vec3f {
    let t = 1.0 / vec2f(textureDimensions(src));
    var o = array<vec2f, 13>(
        vec2f(-2.0, -2.0), vec2f(0.0, -2.0), vec2f(2.0, -2.0),
        vec2f(-1.0, -1.0), vec2f(1.0, -1.0),
        vec2f(-2.0, 0.0), vec2f(0.0, 0.0), vec2f(2.0, 0.0),
        vec2f(-1.0, 1.0), vec2f(1.0, 1.0),
        vec2f(-2.0, 2.0), vec2f(0.0, 2.0), vec2f(2.0, 2.0),
    );
    var w = array<f32, 13>(
        0.03125, 0.0625, 0.03125,
        0.125, 0.125,
        0.0625, 0.125, 0.0625,
        0.125, 0.125,
        0.03125, 0.0625, 0.03125,
    );
    var sum = vec3f(0.0);
    for (var i = 0; i < 13; i++) {
        var c = tap(uv + o[i] * t);
        if first {
            c = bright(c);
        }
        sum += c * w[i];
    }
    return sum;
}

@fragment
fn fs_first(@builtin(position) p: vec4f) -> @location(0) vec4f {
    return vec4f(down13(p.xy / pr.dst, true), 1.0);
}

@fragment
fn fs_down(@builtin(position) p: vec4f) -> @location(0) vec4f {
    return vec4f(down13(p.xy / pr.dst, false), 1.0);
}

@fragment
fn fs_up(@builtin(position) p: vec4f) -> @location(0) vec4f {
    let uv = p.xy / pr.dst;
    let t = 1.0 / vec2f(textureDimensions(src));
    var sum = tap(uv) * 4.0;
    sum += (tap(uv + vec2f(-t.x, 0.0)) + tap(uv + vec2f(t.x, 0.0))
        + tap(uv + vec2f(0.0, -t.y)) + tap(uv + vec2f(0.0, t.y))) * 2.0;
    sum += tap(uv + vec2f(-t.x, -t.y)) + tap(uv + vec2f(t.x, -t.y))
        + tap(uv + vec2f(-t.x, t.y)) + tap(uv + vec2f(t.x, t.y));
    return vec4f(sum / 16.0 * pr.up, 0.0);
}
