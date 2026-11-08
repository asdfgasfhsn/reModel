# Visual enhancements: roadmap

How reModel could make the games look better than the hardware did:
sharper edges, higher-resolution textures, better models, and modern
lighting up to ray tracing. This is a plan to track, not a description of
what exists. Each item says whether it is done, planned or an idea.
Written 2026-11-09.

## Principles

- **Render-only.** No enhancement may change what the game computes. The
  state hash of a coin-start run must stay the same with every
  enhancement on, as it does for widescreen, smooth motion and texture
  packs today. Enhancements change how a frame is drawn, never the frame
  the game asked for.
- **Original by default.** Every enhancement is opt-in or has an off
  switch, and the original look stays exact (the software and GPU
  renderers' bit-for-bit paths).
- **Generic code, separate content.** The hooks and renderer features
  live in this repo and work for every game. Packs (textures, models,
  materials) and any per-game patches are content: they are made and
  distributed separately, never committed here.
- **Gameplay stays authentic.** A change that alters what the player can
  see in a way that could affect play, such as a longer draw distance,
  is an optional patch, off by default and labelled as such.

## Why reModel is well placed

The native geometrizers (`model2::geo_native`, `geo_native_2c`, the 2A
fixed-function one) run the games' display lists in Rust, so for every
frame we know, with their meaning:

- every model drawn, by its ROM address;
- every object's matrix, and the previous frame's, matched by model
  address, place and position (smooth motion already does this,
  `model2::smooth`);
- every polygon, its normal, texture, colour and luma;
- the lights, fog, windows and camera.

Tools like RTX Remix have to reconstruct this from a game's DirectX
calls. We get it for free, and we own the renderer (software, and the
GPU one on wgpu: Vulkan, Metal, DirectX 12 and WebGPU in the browser).
RTX Remix itself doesn't apply: it only hooks Windows games drawing
through DirectX 8/9's fixed-function pipeline.

## Status

| Area | Item | Status |
|---|---|---|
| Output | Any resolution, 16:9 to 32:9, HUD to the edges | done (`docs/play.md`) |
| Output | Wide views fill the sides with scrolled tile skies | done (README, `doa`) |
| Output | Smooth motion: in-betweens on displays over 60 Hz | done, opt-in |
| Output | Run-ahead: less input lag | done (native player), opt-in |
| Textures | Texture packs: dump, replace at any size | done (`docs/texture-packs.md`) |
| Textures | Batch upscaling pipeline (dump → upscale → pack) | planned |
| Textures | Mipmaps from pack images, anisotropic filtering | planned |
| Image | Anti-aliasing (MSAA or supersampling) | planned, next |
| Image | Depth buffer | planned, next |
| Image | Temporal upscaler (motion vectors, jitter) | planned |
| Image | DLSS, optional, native RTX only | idea |
| Models | Model replacement packs, keyed by ROM address | planned |
| Models | Automatic smoothing (vertex normals, curved surfaces) | idea |
| Models | Level of detail and draw distance overrides | idea (optional patch) |
| Lighting | Per-vertex, then per-pixel lighting | planned |
| Lighting | Material maps in packs (normal, specular, roughness) | planned |
| Lighting | Shadows, bloom, HDR tonemapping, better fog | idea |
| Lighting | Hybrid ray tracing (shadows, reflections, AO) | idea |
| Lighting | Path tracing with denoising | idea (research) |
| Audio | Replacement soundtracks | done (`docs/mods.md`) |

## Order of work

1. **Anti-aliasing and a depth buffer.** Cheap, universal and
   groundwork for what follows.
2. **Batch texture upscaling.** The biggest visible gain for the least
   work.
3. **A temporal upscaler.** Shares the depth buffer and motion vectors.
4. **Smooth shading and material maps.**
5. **Model replacement.**
6. **Hybrid ray tracing**, then path tracing.

## The items

### Anti-aliasing

At the original resolution polygon edges are the ugliest part of the
picture. On the GPU, MSAA (4x or 8x) costs little; supersampling
(render at 2x and filter down) is simpler and also smooths texture
shimmer. The software renderer can supersample for snapshots. Both must
keep translucency and the checkerboard (`docs/hw/geometrizer.md` 5.4)
looking right at subsample level.

### Depth buffer

Model 2 draws without one: polygons are sorted far to near
(`model2::raster::place`) and the GPU keeps primitive order
(`docs/gpu.md`), the painter's algorithm. Where objects intersect, or a
large polygon's sort key is wrong for part of it, the wrong one shows.
A depth buffer from the vertices' depth fixes that without changing what
the game sends. Points to settle:

- windows drawn in order (HUD windows over the scene): a depth clear
  per window, or depth only within a window;
- translucent polygons: draw them after the opaque ones, in sorted order,
  testing depth but not writing it;
- coplanar decals (shadows, road markings): a depth bias, or keep the
  game's order where depths tie;
- a setting to keep the original order, for comparison.

The depth buffer is also an input to the temporal upscaler, ambient
occlusion and ray tracing.

### Batch texture upscaling

`make dump-textures` already writes every 3D texture and 2D tile a game
draws; a pack of replacements at any size is drawn in their place,
matched by content (`docs/texture-packs.md`). A target that runs a
game's dumps through an upscaler and writes a pack makes this one
command. The upscaler is an external tool (ESRGAN-style models suit
textures and pixel art); reModel only drives it. Points to settle:

- 4-bit indexed textures go through luma tables and colour translation:
  upscale the rendered texture at full luma (as dumped) and let the
  pack's existing per-channel mapping (`docs/texture-packs.md`) apply
  luma;
- translucent texel 15 becomes alpha; the upscaler must keep hard alpha
  edges;
- tiles that tile seamlessly (road, wallpaper) need wrap-aware upscaling.

### Mipmaps and anisotropic filtering

Packs are drawn with the game's bilinear filtering. High-resolution pack
images on small or distant polygons alias; mipmaps built from them and
anisotropic filtering on the GPU fix that. The original textures keep
their exact path.

### Temporal upscaling (and DLSS)

Rendering at a lower resolution and reconstructing at the display's
lets the expensive items (ray tracing) run where they otherwise
couldn't. A temporal upscaler needs:

- **depth**, from the depth buffer;
- **motion vectors per pixel**: exact for us, since smooth motion
  already matches each object's matrices between consecutive frames;
  each vertex's previous position is its model point through the
  previous frame's matrix;
- **a jittered projection**: a sub-pixel offset per frame, easy since we
  own the projection.

Choices:

| | DLSS (NVIDIA) | FSR 2/3 (AMD) | Our own |
|---|---|---|---|
| Hardware | RTX GPUs only | any | any |
| Licence | proprietary SDK; an optional native extra, kept apart | open source (MIT) | ours |
| wgpu / browser | native only, via Vulkan interop (wgpu doesn't expose it) | compute shaders: portable to WGSL, native and browser | yes |
| Quality | best; Ray Reconstruction also denoises ray tracing | good | simpler |

Plan: an FSR-style upscaler in WGSL as the base, everywhere; DLSS as an
optional native extra for RTX owners if the interop is worth it. (The
development machine's RTX 2080 Super supports DLSS upscaling and Ray
Reconstruction, not frame generation, which needs a 40-series card.)

### Model replacement

A pack can map a model's ROM address (and, to be robust to address
reuse, a hash of its polygon data) to a replacement mesh, drawn with the
object's matrix in place of the original's polygons. The native
geometrizers see each model by address, so the hook is where a model is
expanded into polygons. Points to settle: textures and UVs of the new
mesh; models the game builds from several pieces; collision is the
game's and stays unchanged (a much larger mesh can look like it floats
or clips).

### Automatic smoothing

Model 2 models carry a normal per polygon. Averaging the normals of
polygons that share a vertex (within an angle threshold, to keep hard
edges) gives vertex normals, enough for smooth shading. With them,
curved-surface tessellation (PN triangles and similar) can round off
silhouettes of cars and characters with no artist work. Suits some
models better than others; per-pack opt-out.

### Level of detail and draw distance

Games choose coarser models and cull objects by distance. The display
list shows those choices, so a renderer setting could draw the
finest model always, or further away, to remove pop-in. Because it
changes what a player can see, it is an optional per-game patch, off by
default.

### Lighting

The hardware lights each polygon with one luma value, through its luma
table: flat shading. Steps:

1. **Per-vertex lighting** from smoothed vertex normals and the game's
   own light direction and ambient, mapped through the same luma tables
   so colours stay the game's.
2. **Per-pixel lighting**, with **material maps** carried in packs
   (normal, specular, roughness).
3. **Shadows** (shadow maps from the game's light), **bloom**, **HDR**
   with tonemapping, **fog** with depth.

### Ray tracing

Scenes are small: a few thousand polygons a frame, tiny for a modern
GPU. That makes ray tracing realistic:

- **Hybrid first:** rasterise as now, then trace rays only for shadows,
  reflections (wet roads, car bodies) and ambient occlusion. Likely
  affordable at native resolution, with upscaling as a bonus.
- **Path tracing later:** full global illumination. Real-time path
  tracing uses one or two rays a pixel and needs a denoiser more than an
  upscaler (DLSS Ray Reconstruction does both).

Budget: the games run at 60 frames a second, so a frame has about 16 ms;
smooth motion draws the in-between frames on faster displays with the
same pipeline. Platform: wgpu's ray queries are experimental and
native-only; WebGPU has no ray tracing yet, so this would be native only
at first. Lights and materials have to be invented (the games define a
light direction and ambient, nothing physical), which is pack content.

## Open questions

- How packs carry models and materials alongside textures (one pack
  format, versioned).
- Whether the depth buffer can be on by default once it matches the
  original in every game (checked by comparing frames).
- How much of this the browser can have: WebGPU covers everything up to
  the temporal upscaler and per-pixel lighting; ray tracing waits for
  the web platform.
