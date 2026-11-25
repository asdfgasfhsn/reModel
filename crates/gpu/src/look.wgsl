// The look's screen-space pass (docs/enhancements.md, "Looks"; look.rs): ink
// outlines, rim light and speed lines drawn over the finished 3D layer from
// two images the ID pass made at the same size: each pixel's texture ID and
// tag (two words) and its exact 1/z.
//
// Outlines. In screen space 1/z is linear across a flat polygon, so its
// second difference across a pixel is zero on a flat surface however
// steeply it is tilted, and large only where the depth jumps (a silhouette)
// or bends (a crease): no threshold on slope is needed, and flat areas
// stay clean. Ink goes on the nearer side only (the pixel whose neighbours
// at the line's half width are further away), so a line is as thick as
// asked and never eats into the object behind. A line between two textures
// at the same depth is the same test on the IDs, again on one side (the
// nearer, or the larger ID for a tie), and only where the surface bends
// (never between coplanar pieces). Lines fade with distance, and a tag
// can thin a texture's lines or remove them.

struct Ink {
    // Output size in pixels, its height over 1080 (the weights' unit), and
    // the speed lines' amount.
    size: vec2f,
    scale: f32,
    speed: f32,
    // Line weight (pixels at 1080), opacity, depth sensitivity, texture
    // line strength.
    line: vec4f,
    // Ink colour, and the width of depth lines (silhouettes, creases) as a
    // multiple of the line weight.
    colour: vec4f,
    // Fade, as distances (1/z): where lines are at full strength (and
    // nearer), where they are gone (and further); w a seed for the speed
    // lines' hash.
    fade: vec4f,
}

@group(0) @binding(0) var<uniform> ink: Ink;
@group(0) @binding(1) var layer: texture_2d<f32>;
@group(0) @binding(2) var ids: texture_2d<u32>;
@group(0) @binding(3) var depth: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4f(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

fn clampi(p: vec2i) -> vec2i {
    return clamp(p, vec2i(0), vec2i(ink.size) - 1);
}

fn iz_at(p: vec2i) -> f32 {
    return textureLoad(depth, clampi(p), 0).x;
}

// Bit 17 of a pixel's tag: drawn by the game in its own order, without
// depth.
fn painter_of(tag: u32) -> bool {
    return ((tag >> 17u) & 1u) != 0u;
}

// The 1/z at `p` as it counts next to a pixel of 1/z `fallback` (`cp`: that
// pixel is a painter's-order polygon): `fallback` itself, which reads as
// flat, where `p` is a pixel the 3D layer drew but the ID pass has nothing
// for (a polygon drawn without depth, translucent: its depth and texture
// are unknown, so it is never taken for a step), or where both are
// painter's-order polygons (their depths where they meet are not the
// surfaces'); a fifth of the step where just one is (a ground quad's far
// edge hovers a tenth above the ground it meets: only a large step counts).
// 0 where nothing is drawn.
fn iz_or(p: vec2i, fallback: f32, cp: bool) -> f32 {
    let q = clampi(p);
    let e = textureLoad(ids, q, 0);
    if e.x == 0u {
        if textureLoad(layer, q, 0).a >= 0.5 {
            return fallback;
        }
        return 0.0;
    }
    let pp = painter_of(e.y);
    if cp && pp {
        return fallback;
    }
    let n = textureLoad(depth, q, 0).x;
    if cp != pp {
        return fallback + (n - fallback) * 0.2;
    }
    return n;
}

// How much nearer than its neighbours `iz0` is at `r` pixels along each
// axis: the largest relative drop in 1/z, one-sided (0 where the pixel is
// the further one, or flat). Where both neighbours on an axis are further
// away the pixel is on something thinner than twice `r` (a wire, a fence's
// mesh, a far tree's twigs): that is mostly noise as ink, so it is drawn
// at a third.
fn near_side(c: vec2i, iz0: f32, r: i32, cp: bool) -> f32 {
    let l = iz_or(c + vec2i(-r, 0), iz0, cp);
    let rt = iz_or(c + vec2i(r, 0), iz0, cp);
    let u = iz_or(c + vec2i(0, -r), iz0, cp);
    let d = iz_or(c + vec2i(0, r), iz0, cp);
    let lx = (l + rt - 2.0 * iz0) / iz0;
    let ly = (u + d - 2.0 * iz0) / iz0;
    // The nearer of the two neighbours' drop: large only where both are far.
    let bx = max((l - iz0) / iz0, (rt - iz0) / iz0);
    let by = max((u - iz0) / iz0, (d - iz0) / iz0);
    let kx = 1.0 - 0.67 * smoothstep(-0.1, -0.3, bx);
    let ky = 1.0 - 0.67 * smoothstep(-0.1, -0.3, by);
    return max(max(-lx * kx, -ly * ky), 0.0);
}

// How strongly a neighbour at `r` pixels in some direction is another
// texture that this pixel is the front of (the nearer, or the larger ID),
// across a surface that bends: where the 1/z of the pixel and the two
// pixels either side of it along that direction lie on one straight line
// the two textures are coplanar (a decal on a wall, the pieces of a floor)
// and no line is drawn, however steeply the plane is tilted: that is a
// painted edge, not a shape. `forced` (a tag that names an outline weight)
// draws it anyway.
fn id_side(c: vec2i, id0: u32, iz0: f32, r: i32, forced: bool, cp: bool) -> f32 {
    var hit = 0.0;
    for (var k = 0; k < 4; k++) {
        var o = vec2i(r, 0);
        if k == 1 {
            o = vec2i(-r, 0);
        } else if k == 2 {
            o = vec2i(0, r);
        } else if k == 3 {
            o = vec2i(0, -r);
        }
        let en = textureLoad(ids, clampi(c + o), 0);
        let n = en.x;
        if n == 0u || n == id0 || (cp && painter_of(en.y)) {
            continue;
        }
        let izn = iz_at(c + o);
        // Nearer than the neighbour by more than rounding, or equal and the
        // larger ID.
        let nearer = iz0 > izn * 1.002 || (iz0 >= izn * 0.998 && id0 > n);
        if !nearer {
            continue;
        }
        var bend = 1.0;
        if !forced {
            let back = iz_or(c - o, iz0, cp);
            let second = abs(back + izn - 2.0 * iz0) / iz0;
            bend = smoothstep(0.01, 0.04, second);
        }
        hit = max(hit, bend);
    }
    return hit;
}

fn hash(n: f32) -> f32 {
    return fract(sin(n * 12.9898 + ink.fade.w) * 43758.5453);
}

// Streaks radiating from the middle: a ring of thin wedges of random length
// and width, brighter towards the edges.
fn speed_lines(p: vec2f) -> f32 {
    let q = (p / ink.size - 0.5) * vec2f(ink.size.x / ink.size.y, 1.0) * 2.0;
    let r = length(q);
    let a = atan2(q.y, q.x) / 6.2831853 + 0.5;
    let n = 72.0;
    let cell = floor(a * n);
    let h = hash(cell);
    let f = abs(fract(a * n) - 0.5) * 2.0;
    let width = 0.12 + 0.3 * hash(cell + 31.0);
    let start = 0.55 + 0.35 * h;
    let streak = 1.0 - smoothstep(width * 0.6, width, f);
    return streak * smoothstep(start, start + 0.35, r);
}

@fragment
fn fs(@builtin(position) frag: vec4f) -> @location(0) vec4f {
    let c = vec2i(floor(frag.xy));
    var d = textureLoad(layer, c, 0);
    let e = textureLoad(ids, c, 0);
    if e.x != 0u {
        let tag = e.y;
        let cp = painter_of(tag);
        let wmul = f32(tag & 255u) / 16.0;
        let rim = f32((tag >> 8u) & 255u) / 255.0;
        let iz0 = iz_at(c);
        // Full strength up to distance `fade.x`, none from `fade.y`.
        let fade = 1.0 - smoothstep(ink.fade.x, ink.fade.y, 1.0 / max(iz0, 1e-9));
        let wpx = ink.line.x * ink.scale * wmul;
        if wpx > 0.0 && ink.line.y > 0.0 && fade > 0.002 {
            let r = max(i32(round(wpx)), 1);
            // Silhouettes and creases can be inked wider than texture lines.
            let rs = max(i32(round(wpx * ink.colour.w)), 1);
            // Under a pixel wide, thinner lines are fainter.
            let thin = min(wpx, 1.0);
            var k = 0.0;
            if ink.line.z > 0.0 {
                // Edge strength from a relative drop of 1/z (a fraction
                // of it): a tenth is a clear step.
                let t = 0.1 / ink.line.z;
                k = smoothstep(t * 0.5, t, near_side(c, iz0, rs, cp));
            }
            if ink.line.w > 0.0 {
                k = max(k, ink.line.w * id_side(c, e.x, iz0, r, ((tag >> 16u) & 1u) != 0u, cp));
            }
            k *= ink.line.y * fade * thin;
            let ink_rgb = ink.colour.xyz * d.a;
            d = vec4f(mix(d.rgb, ink_rgb, k), d.a);
            if rim > 0.0 {
                // A lighter band just inside the line.
                let r2 = r * 3;
                let s = smoothstep(0.05, 0.2, near_side(c, iz0, r2, cp)) * (1.0 - k) * fade;
                d = vec4f(d.rgb + (vec3f(d.a) - d.rgb) * (rim * s * 0.6), d.a);
            }
        }
    }
    if ink.speed > 0.0 {
        let s = speed_lines(frag.xy) * ink.speed * 0.7;
        d = vec4f(d.rgb * (1.0 - s) + vec3f(s), d.a + s * (1.0 - d.a));
    }
    return d;
}
