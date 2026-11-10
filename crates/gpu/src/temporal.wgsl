// Temporal anti-aliasing's resolve (Quality::temporal, temporal.rs): from
// this frame's jittered internal-size 3D layer and aux target, and the
// last frame's history, to this frame's output-size history, which compose
// reads in place of the 3D layer. Everything is premultiplied RGBA in
// display values, alpha accumulated like colour.
//
// Per output pixel P (its centre, output px):
// 1. The 3x3 internal samples around the one nearest P: a Gaussian
//    reconstruction at P (`cur`; weights by distance in output px, so
//    when upscaling only samples that land near P count much, `cover`
//    = the largest weight), their min/max, mean and deviation.
// 2. Motion: the nearest sample's surface's (aux). Its `status`: no data
//    (none, previous over current depth outside [0.5, 2], or |mv| over
//    half the output's width: zero motion, clip only); hidden (previous
//    position off the picture, or no texel around it in the last aux
//    target with the expected 1/z); seen. If not seen, a surface at least
//    DILATE_STEP nearer among the 3x3 is tried (dilation only then:
//    windows have depth scales of their own, so "nearer" is not "on top"
//    across them, e.g. a HUD line over the scene). Hidden: `fallback` (the
//    samples interpolated bilinearly).
// 3. History at P + mv: a Keys cubic (5 bilinear taps), or the texel as is
//    when the pixel hasn't moved.
// 4. History clipped (towards the mean) into the neighbourhood: min/max
//    for a still pixel, the mean +- gamma deviations within min/max for a
//    moving one or one without motion data.
// 5. Blend factor from the trust in this frame against the history: the
//    base, raised by fast motion, a change of motion since the last frame
//    (from the last aux target), blended stipple, no motion data, and a
//    running mean after a reset; weighed by how near this frame's sample
//    is (`cover`). When upscaling and the history is trusted less than
//    usual, this frame's estimate leans to `fallback` (smooth, whole at
//    every pixel) instead of the nearest sample alone.
// The result is checked finite (bit pattern) so nothing bad can enter the
// history. The first frame after a reset is `fallback`: at 100% and no
// jitter that is the samples as they are (the plain picture, exactly).

struct Resolve {
    // Output and internal sizes in pixels.
    display: vec2f,
    render: vec2f,
    // This frame's and the last frame's jitter in internal pixels:
    // internal pixel i holds the sample at i + 0.5 - jitter.
    jitter: vec2f,
    jitter_prev: vec2f,
    // Frames resolved since the last reset (0: no history).
    count: f32,
    // Bit 0: pass-through (test hook): the nearest sample, no history.
    flags: u32,
    pad: vec2f,
    // Tuning (temporal.rs `Tuning`):
    // x: base blend factor, y: kernel (1/px^2) at 100%, z, w: clip gamma at 100% and
    // at 50% and below.
    k0: vec4f,
    // x: blend factor at fast motion, y: its length (output px), z: without
    // motion data, w: on blended stipple.
    k1: vec4f,
    // x: blend factor at a motion change, y: change ignored (px), z: change
    // for the full factor (px), w: depth tolerance (share of 1/z).
    k2: vec4f,
    // x: kernel at 50% and below, y: sharpening after the resolve (0 off,
    // 1 the most; `fs_sharpen`), z: the history resampling cubic's C (Keys; 0.5
    // Catmull-Rom).
    k3: vec4f,
}

@group(0) @binding(0) var<uniform> rv: Resolve;

// Dilation: a neighbour's motion is tried when it is this much nearer
// (1/z) than the nearest sample's surface.
const DILATE_STEP: f32 = 1.1;
// This frame, internal size: premultiplied RGBA; aux (motion, 1/z,
// expected previous 1/z).
@group(0) @binding(1) var colour: texture_2d<f32>;
@group(0) @binding(2) var aux: texture_2d<f32>;
// The last frame's aux target and history (output size).
@group(0) @binding(3) var aux_prev: texture_2d<f32>;
@group(0) @binding(4) var history: texture_2d<f32>;
@group(0) @binding(5) var lin: sampler;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    // One triangle covering the screen.
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4f(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

fn finite4(v: vec4f) -> bool {
    let e = bitcast<vec4u>(v) & vec4u(0x7f800000u);
    return all(e != vec4u(0x7f800000u));
}

fn load_colour(i: vec2i) -> vec4f {
    return textureLoad(colour, clamp(i, vec2i(0), vec2i(rv.render) - 1), 0);
}

// Keys' cubic convolution kernel at distance x (in [0, 2]).
fn keys(x: vec2f, a: f32) -> vec2f {
    let near = ((a + 2.0) * x - (a + 3.0)) * x * x + 1.0;
    let far = ((a * x - 5.0 * a) * x + 8.0 * a) * x - 4.0 * a;
    return select(far, near, x <= vec2f(1.0));
}

// The history at `p` (output px): Keys' cubic (C = Tuning::cubic; 0.75, a
// little sharper than Catmull-Rom, which blurred slow motion as the history
// is resampled every frame) from five bilinear taps.
fn resample_history(p: vec2f) -> vec4f {
    let size = rv.display;
    let centre = floor(p - 0.5) + 0.5;
    let f = p - centre;
    // Keys' cubic with a = -C (C = 0.5: Catmull-Rom).
    let ka = -rv.k3.z;
    let w0 = keys(1.0 + f, ka);
    let w1 = keys(f, ka);
    let w2 = keys(1.0 - f, ka);
    let w3 = keys(2.0 - f, ka);
    let w12 = w1 + w2;
    let t12 = (centre + w2 / w12) / size;
    let t0 = (centre - 1.0) / size;
    let t3 = (centre + 2.0) / size;
    let a = w12.x * w0.y;
    let b = w0.x * w12.y;
    let c = w12.x * w12.y;
    let d = w3.x * w12.y;
    let e = w12.x * w3.y;
    let s = textureSampleLevel(history, lin, vec2f(t12.x, t0.y), 0.0) * a
        + textureSampleLevel(history, lin, vec2f(t0.x, t12.y), 0.0) * b
        + textureSampleLevel(history, lin, t12, 0.0) * c
        + textureSampleLevel(history, lin, vec2f(t3.x, t12.y), 0.0) * d
        + textureSampleLevel(history, lin, vec2f(t12.x, t3.y), 0.0) * e;
    return s / (a + b + c + d + e);
}

// Slack of the clip box per side: half an 8-bit level. Without it a channel
// whose box has no width (alpha 1 on every opaque sample, blue 0 on a red
// and yellow decal) holds the whole history to the mean when the history
// is a rounding step off there (alpha 0.99951, the half float just below
// 1): the history became the 3x3 mean at random pixels, dots along high
// contrast edges and blur inside textures.
const CLIP_SLACK: f32 = 0.5 / 255.0;

// `h` moved towards `mu` until it is inside [lo, hi] widened by CLIP_SLACK
// (mu is inside).
fn clip_towards(h: vec4f, mu: vec4f, lo_in: vec4f, hi_in: vec4f) -> vec4f {
    let lo = lo_in - CLIP_SLACK;
    let hi = hi_in + CLIP_SLACK;
    let d = h - mu;
    let up = (hi - mu) / max(d, vec4f(1e-7));
    let down = (lo - mu) / min(d, vec4f(-1e-7));
    let t = select(select(vec4f(1.0), down, d < vec4f(-1e-7)), up, d > vec4f(1e-7));
    let k = clamp(min(min(t.x, t.y), min(t.z, t.w)), 0.0, 1.0);
    return mu + d * k;
}

// What a sample's surface (its aux) says about the history.
const NO_DATA: u32 = 0u;
const HIDDEN: u32 = 1u;
const SEEN: u32 = 2u;

// NO_DATA: no motion data, or previous over current depth outside [0.5,
// 2], or |mv| over half the output's width (zero motion, clip only).
// HIDDEN: the previous position is off the picture, or no texel around it
// in the last aux target holds the expected 1/z (it was hidden, or
// nothing was there). SEEN: usable. `zrange`: the span of 1/z over the
// neighbourhood (a receding surface's tolerance).
fn status(a: vec4f, p: vec2f, rs: vec2f, zrange: f32) -> u32 {
    let z = abs(a.z);
    let e = a.w;
    if !(e > 0.0) || !(z > 0.0) {
        return NO_DATA;
    }
    let w = z / e;
    if w < 0.5 || w > 2.0 || length(a.xy) > 0.5 * rv.display.x {
        return NO_DATA;
    }
    let pp = p + a.xy;
    if any(pp < vec2f(0.0)) || any(pp >= rv.display) {
        return HIDDEN;
    }
    var tol = rv.k2.w * e;
    if zrange < 0.5 * e {
        tol += zrange;
    }
    let q0 = vec2i(floor(pp * rs + rv.jitter_prev - 0.5));
    for (var k = 0; k < 4; k++) {
        let i = clamp(q0 + vec2i(k & 1, k >> 1u), vec2i(0), vec2i(rv.render) - 1);
        let b = abs(textureLoad(aux_prev, i, 0).z);
        if b > 0.0 && abs(b - e) <= max(tol, rv.k2.w * b) {
            return SEEN;
        }
    }
    return HIDDEN;
}

// The resolve's result, and what it decided (`fs_inspect`):
// - x: rules: the nearest sample's status (0 no data, 1 hidden, 2 seen) +
//   4 x the status used + 16 if dilated + 32 on a reset + 64 on blended
//   stipple + 128 if the history was loaded without resampling + 256 if a
//   value was not finite;
// - y: this frame's share in the result (1: the fallback alone);
// - z: how far the clip moved the history (largest of RGBA);
// - w: which blend factor set the trust (0 the base, 1 after a reset, 2
//   fast motion, 3 a motion change, 4 blended stipple, 5 no data) + 8 x
//   the motion's length in quarter output px.
struct Res {
    out: vec4f,
    dbg: vec4f,
}

fn res(out: vec4f, rules: f32) -> Res {
    return Res(out, vec4f(rules, 1.0, 0.0, 0.0));
}

@fragment
fn fs_resolve(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    return resolve(pos.xy).out;
}

@fragment
fn fs_inspect(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    return resolve(pos.xy).dbg;
}

fn resolve(p: vec2f) -> Res {
    let rs = rv.render / rv.display;
    // Sample grid coordinates: the sample of internal pixel i is at g = i.
    let g = p * rs + rv.jitter - 0.5;
    if (rv.flags & 1u) != 0u {
        return res(load_colour(vec2i(floor(p * rs))), 0.0);
    }
    // The samples interpolated: the picture without history.
    let g0 = floor(g);
    let t = g - g0;
    let b0 = vec2i(g0);
    let fallback = mix(
        mix(load_colour(b0), load_colour(b0 + vec2i(1, 0)), t.x),
        mix(load_colour(b0 + vec2i(0, 1)), load_colour(b0 + vec2i(1, 1)), t.x),
        t.y
    );
    if rv.count < 0.5 {
        return res(fallback, 32.0);
    }

    // 1. The neighbourhood.
    let c0 = vec2i(floor(g + 0.5));
    let maxi = vec2i(rv.render) - 1;
    // How far into upscaling: 0 at 100%, 1 at 50% and below.
    let up = clamp(1.0 / rs.x - 1.0, 0.0, 1.0);
    let kernel = mix(rv.k0.y, rv.k3.x, up);
    var sum = vec4f(0.0);
    var wsum = 0.0;
    var cover = 0.0;
    var m1 = vec4f(0.0);
    var m2 = vec4f(0.0);
    var mn = vec4f(1e9);
    var mx = vec4f(-1e9);
    var near = vec4f(0.0);
    var centre = vec4f(0.0);
    var bestz = -1.0;
    var zmin = 1e9;
    var zmax = 0.0;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let i = clamp(c0 + vec2i(dx, dy), vec2i(0), maxi);
            let s = textureLoad(colour, i, 0);
            let a = textureLoad(aux, i, 0);
            let d = (vec2f(i) - g) / rs;
            let w = exp(-kernel * dot(d, d));
            sum += w * s;
            wsum += w;
            cover = max(cover, w);
            m1 += s;
            m2 += s * s;
            mn = min(mn, s);
            mx = max(mx, s);
            let z = abs(a.z);
            if z > bestz {
                bestz = z;
                near = a;
            }
            if dx == 0 && dy == 0 {
                centre = a;
            }
            if z > 0.0 {
                zmin = min(zmin, z);
                zmax = max(zmax, z);
            }
        }
    }
    let cur = sum / max(wsum, 1e-20);
    let mu = m1 / 9.0;
    let sigma = sqrt(max(m2 / 9.0 - mu * mu, vec4f(0.0)));

    // Motion: the nearest sample's surface's. Where that has no usable
    // history (no data, or it wasn't there: the trailing side of a moving
    // edge), a nearer surface's around it if that has (dilation, so the
    // edge brings its history along). Only then: windows have depth
    // scales of their own, so a nearer 1/z next to a HUD line drawn over
    // the scene is not in front of it.
    let zrange = zmax - zmin;
    var best = centre;
    var st = status(centre, p, rs, zrange);
    let st0 = f32(st);
    var dilated = 0.0;
    if st != SEEN && bestz > abs(centre.z) * DILATE_STEP && status(near, p, rs, zrange) == SEEN {
        best = near;
        st = SEEN;
        dilated = 16.0;
    }
    var rules = st0 + 4.0 * f32(st) + dilated + select(0.0, 64.0, best.z < 0.0);
    if st == HIDDEN {
        return res(fallback, rules);
    }
    let nodata = st == NO_DATA;
    var mv = best.xy;
    if nodata {
        mv = vec2f(0.0);
    }
    let len = length(mv);

    // 2. The history at the previous position.
    let pp = p + mv;
    var h: vec4f;
    if len < 0.01 {
        h = textureLoad(history, vec2i(p), 0);
        rules += 128.0;
    } else {
        h = resample_history(pp);
    }
    if !finite4(h) {
        return res(fallback, rules + 256.0);
    }
    h = clamp(h, vec4f(0.0), vec4f(1.0));
    var vel_prev = vec2f(0.0);
    if !nodata {
        let gp = pp * rs + rv.jitter_prev - 0.5;
        vel_prev = textureLoad(aux_prev, clamp(vec2i(floor(gp + 0.5)), vec2i(0), vec2i(rv.render) - 1), 0).xy;
    }

    // 4. Clip into the neighbourhood.
    var lo = mn;
    var hi = mx;
    if len >= 0.25 || nodata {
        let gamma = mix(rv.k0.z, rv.k0.w, up);
        lo = max(mn, mu - gamma * sigma);
        hi = min(mx, mu + gamma * sigma);
    }
    let unclipped = h;
    h = clip_towards(h, mu, lo, hi);
    let moved = abs(h - unclipped);

    // 5. Blend.
    var trust = rv.k0.x;
    var why = 0.0;
    let after_reset = 1.0 / (rv.count + 1.0);
    if after_reset > trust {
        trust = after_reset;
        why = 1.0;
    }
    let fast = rv.k1.x * clamp(len / rv.k1.y, 0.0, 1.0);
    if fast > trust {
        trust = fast;
        why = 2.0;
    }
    if !nodata {
        let change = length(mv - vel_prev);
        let vel = rv.k2.x * clamp((change - rv.k2.y) / rv.k2.z, 0.0, 1.0);
        if vel > trust {
            trust = vel;
            why = 3.0;
        }
    }
    if best.z < 0.0 && rv.k1.w > trust {
        trust = rv.k1.w;
        why = 4.0;
    }
    if nodata && rv.k1.z > trust {
        trust = rv.k1.z;
        why = 5.0;
    }
    // As a weight: `trust` is the current sample's share when it is at P;
    // one further off counts less. When upscaling and the history is
    // trusted less than usual (fast motion, after a reset, ...), this
    // frame's estimate leans to the samples interpolated (smooth, and
    // whole at every pixel) rather than the nearest sample alone.
    let wide = clamp((trust - rv.k0.x) / max(rv.k1.x - rv.k0.x, 1e-6), 0.0, 1.0)
        * clamp(2.0 * (1.0 / rs.x - 1.0), 0.0, 1.0);
    let est = mix(cur, fallback, wide);
    let weight = mix(cover, 1.0, wide);
    let n = (1.0 - trust) / max(trust, 1e-6);
    let alpha = weight / (weight + n);
    let out = h + (est - h) * alpha;
    if !finite4(out) {
        return res(fallback, rules + 256.0);
    }
    let clip = max(max(moved.x, moved.y), max(moved.z, moved.w));
    return Res(out, vec4f(rules, alpha, clip, why + 8.0 * floor(min(len, 16384.0) * 4.0)));
}

// Sharpening after the resolve (Tuning::sharpen > 0): the history is left
// as it is (sharpening it would compound frame after frame); this pass
// draws a sharpened copy that compose reads instead. Contrast adaptive, in
// the manner of AMD's CAS: a negative-lobed cross filter whose lobe
// shrinks where the neighbourhood already spans the range (strong edges),
// and the result kept within the cross's own min/max, so it cannot ring or
// draw halos past the colours that are there. Premultiplied RGBA, the same
// weights for every channel.
@group(0) @binding(6) var resolved: texture_2d<f32>;

fn load_resolved(i: vec2i) -> vec4f {
    return textureLoad(resolved, clamp(i, vec2i(0), vec2i(rv.display) - 1), 0);
}

@fragment
fn fs_sharpen(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let i = vec2i(pos.xy);
    let e = load_resolved(i);
    let b = load_resolved(i + vec2i(0, -1));
    let d = load_resolved(i + vec2i(-1, 0));
    let f = load_resolved(i + vec2i(1, 0));
    let h = load_resolved(i + vec2i(0, 1));
    let mn = min(min(min(b, d), min(f, h)), e);
    let mx = max(max(max(b, d), max(f, h)), e);
    // Headroom of the green channel (as CAS): 1 in flat areas, 0 where the
    // neighbourhood spans nothing to full (alpha).
    let room = clamp(min(mn.y, mx.w - mx.y) / max(mx.y, 1e-4), 0.0, 1.0);
    // The lobe: up to CAS's strongest (-1/5) at sharpen 1.
    let w = -sqrt(room) * 0.2 * clamp(rv.k3.y, 0.0, 1.0);
    let out = (e + w * (b + d + f + h)) / (1.0 + 4.0 * w);
    return clamp(out, mn, mx);
}
