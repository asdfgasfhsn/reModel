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
    _a: u32,
    // Top-left of the frame in the target (letterboxing), in pixels.
    origin: vec2f,
    backdrop: vec4f,
}

@group(0) @binding(0) var<uniform> pr: Params;
@group(0) @binding(1) var base: texture_2d<f32>;
@group(0) @binding(2) var layer3d: texture_2d<f32>;
@group(0) @binding(3) var hud: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    // One triangle covering the screen.
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4f(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
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
    let gx = ex - pr.pad;
    var c = vec3f(0.0);
    if gx >= 0.0 && i32(gx) < 496 {
        c = textureLoad(base, vec2i(i32(gx), gy), 0).rgb;
    } else if pr.has_3d != 0u {
        c = pr.backdrop.rgb;
    }
    let d = textureLoad(layer3d, vec2i(floor(pos.xy)), 0);
    if d.a > 0.0 {
        c = d.rgb;
    }
    if i32(ex) < i32(pr.ext_w) {
        let h = textureLoad(hud, vec2i(i32(ex), gy), 0);
        if h.a > 0.0 {
            c = h.rgb;
        }
    }
    return vec4f(c, 1.0);
}
