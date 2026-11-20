// SPIKE: screen-space ambient occlusion over the 3D layer, from the ID
// pass's targets: the polygon index (ids.z, + 1) and 1/z; the polygon's
// view-space normal and its window's centre and focal lengths come from a
// per-polygon table. Two passes: `fs_raw` (16 spiral samples, one per
// pixel pattern) and `fs_apply` (a 4x4 surface-aware blur, then the term
// multiplies the picture where the polygon takes it). Every loop has a
// constant bound.

struct P {
    // Output size, output pixels per game pixel.
    size: vec2f,
    scale: vec2f,
    // pad (game pixels, left of the 496), radius (view units), strength, mode
    // (0 apply, 1 the term as grey, 2 the eligible pixels in magenta).
    a: vec4f,
}

@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var ids: texture_2d<u32>;
@group(0) @binding(2) var izt: texture_2d<f32>;
// Per polygon (placed index): (nx, ny, nz, takes) then (cx, cy, fx, fy).
@group(0) @binding(3) var<storage, read> table: array<vec4f>;
@group(0) @binding(4) var picture: texture_2d<f32>;
@group(0) @binding(5) var raw: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4f(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

struct Surf {
    ok: bool,
    pos: vec3f,
    n: vec3f,
    fx: f32,
}

fn surf(q: vec2i) -> Surf {
    var s: Surf;
    s.ok = false;
    let dims = vec2i(p.size);
    if q.x < 0 || q.y < 0 || q.x >= dims.x || q.y >= dims.y {
        return s;
    }
    let id = textureLoad(ids, q, 0).z;
    let iz = textureLoad(izt, q, 0).x;
    if id == 0u || !(iz > 0.0) {
        return s;
    }
    let a = table[(id - 1u) * 2u];
    if a.w < 0.5 {
        return s;
    }
    let b = table[(id - 1u) * 2u + 1u];
    let g = (vec2f(q) + 0.5) / p.scale - vec2f(p.a.x, 0.0);
    let z = 1.0 / iz;
    s.pos = vec3f((g.x - b.x) * z / b.z, (b.y - g.y) * z / b.w, z);
    s.n = select(a.xyz, -a.xyz, dot(a.xyz, s.pos) > 0.0);
    s.fx = b.z;
    s.ok = true;
    return s;
}

fn pat(q: vec2i) -> f32 {
    var m = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
    return m[(q.y & 3) * 4 + (q.x & 3)] / 16.0;
}

@fragment
fn fs_raw(@builtin(position) f: vec4f) -> @location(0) vec4f {
    let q = vec2i(f.xy);
    let s = surf(q);
    if !s.ok {
        return vec4f(1.0);
    }
    let radius = p.a.y;
    let rpx = clamp(radius * s.fx * p.scale.x / s.pos.z, 3.0 * p.scale.y, 60.0 * p.scale.y);
    let rot = pat(q) * 6.2831853;
    var occ = 0.0;
    for (var k = 0; k < 16; k++) {
        let a = f32(k) * 2.399963 + rot;
        let r = sqrt((f32(k) + 0.5) / 16.0) * rpx;
        let sq = vec2i(floor(vec2f(q) + vec2f(cos(a), sin(a)) * r));
        let o = surf(sq);
        if !o.ok {
            continue;
        }
        let v = o.pos - s.pos;
        let d = length(v);
        if d < 1e-6 || d >= radius {
            continue;
        }
        let c = dot(s.n, v) / d;
        occ += max(c - 0.15, 0.0) * (1.0 - d / radius);
    }
    return vec4f(clamp(1.0 - p.a.z * 2.0 * occ / 16.0, 0.0, 1.0), 0.0, 0.0, 1.0);
}

@fragment
fn fs_apply(@builtin(position) f: vec4f) -> @location(0) vec4f {
    let q = vec2i(f.xy);
    let px = textureLoad(picture, q, 0);
    let s = surf(q);
    if !s.ok {
        return px;
    }
    var sum = 0.0;
    var cnt = 0.0;
    for (var dy = -2; dy < 2; dy++) {
        for (var dx = -2; dx < 2; dx++) {
            let nq = q + vec2i(dx, dy);
            let o = surf(nq);
            if !o.ok || abs(o.pos.z - s.pos.z) > 0.03 * s.pos.z || dot(o.n, s.n) < 0.9 {
                continue;
            }
            sum += textureLoad(raw, nq, 0).x;
            cnt += 1.0;
        }
    }
    var t = textureLoad(raw, q, 0).x;
    if cnt > 0.0 {
        t = sum / cnt;
    }
    let mode = p.a.w;
    if mode > 1.5 {
        return vec4f(1.0, 0.0, 1.0, px.a);
    }
    if mode > 0.5 {
        return vec4f(t, t, t, px.a);
    }
    return vec4f(px.rgb * t, px.a);
}
