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
// nearer, or the larger ID for a tie). Lines fade with distance, and a tag
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
    // Ink colour, and the rim light's glow strength.
    colour: vec4f,
    // Fade, as 1/z: where lines are at full strength (and nearer), where
    // they are gone (and further); z a seed for the speed lines' hash.
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

fn id_at(p: vec2i) -> u32 {
    return textureLoad(ids, clampi(p), 0).x;
}

// How much nearer than its neighbours `iz0` is at `r` pixels along each
// axis: the largest relative drop in 1/z, one-sided (0 where the pixel is
// the further one, or flat).
fn near_side(c: vec2i, iz0: f32, r: i32) -> f32 {
    let l = iz_at(c + vec2i(-r, 0));
    let rt = iz_at(c + vec2i(r, 0));
    let u = iz_at(c + vec2i(0, -r));
    let d = iz_at(c + vec2i(0, r));
    let lx = (l + rt - 2.0 * iz0) / iz0;
    let ly = (u + d - 2.0 * iz0) / iz0;
    return max(max(-lx, -ly), 0.0);
}

// Whether a neighbour at `r` pixels in some direction is another texture
// that this pixel is the front of (the nearer, or the larger ID).
fn id_side(c: vec2i, id0: u32, iz0: f32, r: i32) -> f32 {
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
        let n = id_at(c + o);
        if n == 0u || n == id0 {
            continue;
        }
        let izn = iz_at(c + o);
        // Nearer than the neighbour by more than rounding, or equal and the
        // larger ID.
        let nearer = iz0 > izn * 1.002 || (iz0 >= izn * 0.998 && id0 > n);
        if nearer {
            hit = 1.0;
        }
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
        let wmul = f32(tag & 255u) / 16.0;
        let rim = f32((tag >> 8u) & 255u) / 255.0;
        let iz0 = iz_at(c);
        // Full strength at 1/z `fade.x` and nearer, none at `fade.y` and
        // further.
        let fade = smoothstep(ink.fade.y, ink.fade.x, iz0);
        let wpx = ink.line.x * ink.scale * wmul;
        if wpx > 0.0 && ink.line.y > 0.0 {
            let r = max(i32(round(wpx)), 1);
            // Under a pixel wide, thinner lines are fainter.
            let thin = min(wpx, 1.0);
            var k = 0.0;
            if ink.line.z > 0.0 {
                // Edge strength from a relative drop of 1/z (a fraction
                // of it): a tenth is a clear step.
                let t = 0.1 / ink.line.z;
                k = smoothstep(t * 0.5, t, near_side(c, iz0, r));
            }
            if ink.line.w > 0.0 {
                k = max(k, ink.line.w * id_side(c, e.x, iz0, r));
            }
            k *= ink.line.y * fade * thin;
            let ink_rgb = ink.colour.xyz * d.a;
            d = vec4f(mix(d.rgb, ink_rgb, k), d.a);
            if rim > 0.0 {
                // A lighter band just inside the line.
                let r2 = r * 3;
                let s = smoothstep(0.05, 0.2, near_side(c, iz0, r2)) * (1.0 - k) * fade;
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
