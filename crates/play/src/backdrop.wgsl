// The launcher's backdrop, in the logo's style: a neon cyan wireframe grid
// floor scrolling slowly towards the viewer under a horizon of low-poly
// wireframe mountains, on black. One full-screen triangle; colours are
// display values (the surface isn't sRGB).

struct Params {
    // Target size in pixels.
    size: vec2f,
    // Where the horizon is, as a fraction of the height from the top.
    horizon: f32,
    // Pixels per point times the launcher's scale: line widths follow it.
    scale: f32,
    // The floor's scroll, 0..1 (one grid square), and the mountains' drift.
    phase: f32,
    drift: f32,
    // Overall brightness, 0..1.
    level: f32,
    _pad: f32,
}

@group(0) @binding(0) var<uniform> pr: Params;

const CYAN: vec3f = vec3f(0.184, 0.902, 0.937);
const BLUE: vec3f = vec3f(0.141, 0.467, 1.0);

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4f(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

// Coverage of a line at every integer of `v`, `width` pixels wide.
fn grid_line(v: f32, width: f32) -> f32 {
    let d = abs(fract(v - 0.5) - 0.5) / max(fwidth(v), 1e-5);
    return 1.0 - clamp(d - width * 0.5 + 0.5, 0.0, 1.0);
}

// 0..1 triangle wave: straight slopes, sharp peaks, as low-poly hills.
fn tri(x: f32) -> f32 {
    return 1.0 - abs(fract(x) * 2.0 - 1.0);
}

// Height of the mountains above the horizon at `x` (units of the height).
fn ridge(x: f32) -> f32 {
    let h = tri(x * 0.9 + 0.13) * 0.55 + tri(x * 2.3 + 0.41) * 0.3 + tri(x * 5.1 + 0.7) * 0.15;
    // Lower in the middle, where the picture is.
    let side = smoothstep(0.15, 1.1, abs(x));
    return 0.035 + h * mix(0.025, 0.11, side);
}

@fragment
fn fs(@builtin(position) frag: vec4f) -> @location(0) vec4f {
    let h = pr.size.y;
    let p = vec2f((frag.x - pr.size.x * 0.5) / h, (frag.y - pr.horizon * h) / h);
    let width = 1.2 * pr.scale;
    var c = vec3f(0.0);

    // Sky: black, a faint blue band down to the horizon.
    let above = max(-p.y, 0.0);
    c += BLUE * 0.10 * exp(-above * 9.0);

    // Derivatives (in `grid_line`) need uniform control flow: compute the
    // floor and the mountains everywhere, then pick.
    // The floor: depth from the height below the horizon.
    let z = 0.35 / max(p.y, 1e-4);
    let wx = p.x * z * 2.2;
    let wz = z + pr.phase;
    let line = max(grid_line(wx, width), grid_line(wz, width));
    // Fade into the distance (lines crowd and shimmer there).
    let far = exp(-z * 0.12) * smoothstep(0.0, 0.06, p.y);
    let floor = CYAN * (line * far * 0.55 + 0.06 * exp(-p.y * 14.0));

    // Mountains: a ridge line, faint vertical wire under each slope, and
    // their body a shade darker than the sky.
    let x = p.x + pr.drift;
    let r = ridge(x);
    let edge = 1.0 - clamp(abs(above - r) * h / width - 0.5, 0.0, 1.0);
    let wire = grid_line(x * 14.0, width * 0.8) * clamp(above / r, 0.0, 1.0);
    let inside = select(1.0, 0.35, above < r);
    let hills = c * inside + CYAN * (edge * 0.55 + wire * 0.10 * f32(above < r));

    c = select(hills, c + floor, p.y > 0.0);
    // The horizon line itself.
    c += CYAN * 0.5 * exp(-abs(p.y) * h / (1.5 * pr.scale));

    // Vignette towards the corners.
    let q = frag.xy / pr.size - 0.5;
    c *= 1.0 - dot(q, q) * 1.1;
    return vec4f(clamp(c * pr.level, vec3f(0.0), vec3f(1.0)), 1.0);
}
