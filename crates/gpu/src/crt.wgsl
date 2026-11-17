// The CRT look (docs/enhancements.md, "CRT"): the finished picture drawn as
// an arcade monitor would show it, in one pass over the composed frame
// (the 3D layer, the 2D layers and the game's own HUD: not the player's
// bezel or menu, which are drawn after).
//
//   curvature   the picture bulges a little: a barrel warp, the middles of
//               the left and right edges kept in place
//   scanlines   one beam line for each of the game's 384 lines, whatever
//               the output's size: the picture is averaged across a line
//               (three taps), and a line's brightness falls off from its
//               centre as a gaussian; brighter lines are wider, so bright
//               parts fill the gap between lines and dark ones leave it
//   softness    a little blur along a line, the beam's width
//   glow        the bright parts bleed a little, in a few wide taps
//   mask        a red, green, blue stripe triad, staggered on alternate
//               lines (a slot mask)
//   vignette    the corners fall off softly
//
// Everything scales with `strength` (0 to 1), and the line structure and
// the mask fade out when a game line has under about 2 output pixels (they
// would only beat against the pixel grid).

struct Params {
    strength: f32,
    // The game's lines in the picture (384).
    lines: f32,
    // The picture's size in output pixels.
    size: vec2f,
    // Top-left of the picture in the target.
    origin: vec2f,
    pad: vec2f,
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

@fragment
fn fs(@builtin(position) frag: vec4f) -> @location(0) vec4f {
    let s = clamp(pr.strength, 0.0, 1.0);
    let px = frag.xy - pr.origin;
    let d0 = px / pr.size - 0.5;

    // Curvature: sample further out towards the corners; scaled so the
    // middles of the left and right edges stay where they are.
    let k = 0.06 * s;
    let q = d0 * vec2f(1.0, 0.75);
    let uv = 0.5 + d0 * (1.0 + k * dot(q, q)) / (1.0 + k * 0.25);
    let inside = min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y));
    let edge = smoothstep(0.0, 2.0 / pr.size.y, inside);

    // Where in its line this pixel is: -0.5 to 0.5 from the centre.
    let ppl = pr.size.y / pr.lines;
    let dens = smoothstep(1.4, 2.8, ppl);
    let ly = uv.y * pr.lines;
    let line = floor(ly);
    let f = ly - line - 0.5;
    let yc = (line + 0.5) / pr.lines;

    // The line's colour: three taps across it, three along it.
    let dy = 0.3 / pr.lines;
    let dx = 1.0 / pr.size.x;
    var c = vec3f(0.0);
    for (var j = -1; j <= 1; j++) {
        let wy = select(0.25, 0.5, j == 0);
        for (var i = -1; i <= 1; i++) {
            let wx = select(0.25, 0.5, i == 0);
            c += wx * wy * tap(vec2f(uv.x + f32(i) * dx, yc + f32(j) * dy));
        }
    }

    // Glow: what is brighter than 0.75 a few pixels away, a soft halo
    // rather than a lift (at most about +0.06 inside a white area).
    var g = vec3f(0.0);
    let reach = vec2f(3.0, 7.0) * dx;
    g += max(tap(uv + vec2f(reach.x, 0.0)) - 0.75, vec3f(0.0));
    g += max(tap(uv - vec2f(reach.x, 0.0)) - 0.75, vec3f(0.0));
    g += max(tap(uv + vec2f(reach.y, 0.0)) - 0.75, vec3f(0.0));
    g += max(tap(uv - vec2f(reach.y, 0.0)) - 0.75, vec3f(0.0));
    g += max(tap(vec2f(uv.x, yc + 1.5 / pr.lines)) - 0.75, vec3f(0.0));
    g += max(tap(vec2f(uv.x, yc - 1.5 / pr.lines)) - 0.75, vec3f(0.0));
    c += (0.25 * s / 6.0) * g;

    // The beam: gaussian across the line, wider for brighter ones, divided
    // by its mean over the line so a line keeps the picture's brightness
    // on average (brighter at its centre, darker at its edges) instead of
    // lifting it.
    let lum = clamp(dot(c, vec3f(0.299, 0.587, 0.114)), 0.0, 1.0);
    let sigma = mix(0.27, 0.46, lum);
    let inv = 1.0 / (2.0 * sigma * sigma);
    let beam = exp(-f * f * inv);
    var mean = 0.0;
    for (var i = 0; i < 8; i++) {
        let fi = (f32(i) + 0.5) / 8.0 - 0.5;
        mean += exp(-fi * fi * inv);
    }
    mean /= 8.0;
    let depth = 0.85 * s * dens;
    c *= mix(1.0, beam / mean, depth);

    // The mask: the triad's lit channel for this pixel, the others dimmer.
    let a = 0.35 * s * dens;
    let lit = (u32(max(floor(px.x), 0.0)) + (u32(max(line, 0.0)) & 1u)) % 3u;
    let mask = vec3f(
        select(1.0 - a, 1.0, lit == 0u),
        select(1.0 - a, 1.0, lit == 1u),
        select(1.0 - a, 1.0, lit == 2u),
    );
    c *= mask / (1.0 - 0.667 * a);

    // The vignette.
    let r = length(d0 * 2.0 * vec2f(1.0, 0.85));
    c *= (1.0 - 0.5 * s * smoothstep(0.55, 1.35, r)) * edge;
    return vec4f(clamp(c, vec3f(0.0), vec3f(1.0)), 1.0);
}

// The wipe of the picture menu's Compare: a picture of the same size,
// copied as is (the draw's scissor rectangle says where).
@fragment
fn fs_copy(@builtin(position) frag: vec4f) -> @location(0) vec4f {
    return vec4f(textureLoad(src, vec2i(frag.xy), 0).rgb, 1.0);
}
