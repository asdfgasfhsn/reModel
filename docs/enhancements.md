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
| Output | Wide views fill the sides with scrolled tile skies | done (docs/games/doa.md) |
| Output | Smooth motion: in-betweens on displays over 60 Hz | done, opt-in |
| Output | Run-ahead: less input lag | done (native player), opt-in |
| Textures | Texture packs: dump, replace at any size; live on the GPU in the player and the browser | done (`docs/texture-packs.md`); live display 2026-11-10 (tiles still snapshots only) |
| Textures | Batch upscaling pipeline (dump → upscale → pack) | done (2026-11-10, `docs/texture-packs.md`) |
| Textures | Mipmaps from pack images, anisotropic filtering | done, opt-in (2026-11-10) |
| Image | Anti-aliasing: MSAA 4x/8x on the GPU, supersampling in software | done, opt-in (2026-11-09) |
| Image | Depth buffer | done, opt-in, partial (below); current rules checked on all 18 games 2026-11-11 |
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

Done, off by default. Settings: the player's `aa` (menu, launcher,
`--aa off|4|8`, settings files), the browser's `aa` (panel, `?aa=4`),
`remodel --aa N` for snapshots and `gpu-check`. Render-only: state hashes
unchanged.

- **GPU: MSAA** on the 3D layer, 4x everywhere (WebGPU has only 4x), 8x
  where the adapter reports it for both the colour and depth formats
  (`gpu::max_msaa`; native, the device is asked for
  `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES`); other requests round down.
  The layer is resolved before the compose pass, and the resolve is
  *premultiplied by coverage*: the cleared layer is transparent black and
  each sample a polygon covers is opaque, so a resolved pixel holds
  `colour * coverage` and `coverage` in alpha. The compose pass blends
  `layer + (1 - alpha) * what's behind`, so an edge against a tile sky or
  the backdrop is a coverage-weighted mix, not covered or uncovered. With
  one sample alpha is 1 or 0 and the blend is bit-for-bit the old
  replace (checked: `gpu-check` frames identical to before the change).
- Texture sampling is still once per pixel, so MSAA smooths silhouettes
  and intersections, not texture shimmer; attributes are interpolated
  at the centroid so a partly covered pixel never samples outside its
  polygon.
- **The checkerboard** (`docs/hw/geometrizer.md` 5.4) is a per-pixel
  pattern, by game pixel, decided once for the pixel in the fragment
  shader: dropped pixels cover none of their samples and kept ones
  cover whatever the polygon covers. It stays a pattern of whole pixels
  (it does not average to grey), and the edges of a checker polygon are
  anti-aliased like any other. The software supersampler gets the same
  from computing the pattern per game pixel, not per subsample.
  Cut-out texels (translucent texture, texel 15) are alpha-tested per
  pixel, so those edges aren't smoothed.
- **Software: supersampling** for snapshots: `remodel --aa N` draws and
  composes at `k` times the size in each direction, `k` the smallest with
  `k * k >= N` (4 gives 2x2, 8 gives 3x3), and averages blocks
  (`View::supersampled`, `video::downsample`). `gpu-check --aa 4` compares
  the GPU's multisampled frame with the software's supersampled one.
- Cost on the GPU (`make gpu-check GPU_OPTS="--aa 8"` prints it):
  see `docs/gpu.md`.

### Depth buffer

Done, off by default, and deliberately partial. Settings: the player's
`depth` (menu, launcher, `--depth on|off`), the browser's `depth` (panel,
`?depth=1`), `remodel --depth` for snapshots and `gpu-check`; `make
depth-check` draws captured frames with it off and on and pictures every
difference. Render-only: state hashes unchanged.

Model 2 draws without one: polygons are sorted far to near
(`model2::raster::place`) and the painter's algorithm shows the wrong one
where objects intersect, or a polygon's sort key is wrong for part of it.
With the setting on, each polygon's depth is its interpolated 1/z
(`SVtx::iz`, linear in screen space, so exact per pixel), tested per pixel
in a 32-bit float depth buffer (software: the same arithmetic,
`raster::depth_value`). What was decided, from comparing frames of all 18
games with it off and on:

- **Windows:** the buffer is emptied inside each window's clip rectangle
  when the window starts (a scissored clear on the GPU), so HUD windows
  and the 3D layers drawn after the scene stay over it, as in the game's
  order.
- **Order is kept where depth can't be trusted** (`Placed::background`):
  polygons sorted behind everything (skies); **flat black shadows**
  (hotd's are screen-parallel slices at depths unrelated to the floor they
  look laid on); **translucent (checkerboard) polygons**, whose layering
  is the game's (skisuprg's whole scene is dithered layers; reordering
  them after the opaque ones changed it wholesale, and shadows under
  cars vanished when tested); and **polygons whose 1/z spans more than 8x**
  (ground and floor quads sorted by their far edge: topskatr's sand
  quad is above a ramp and a crate that the game shows over it).
  They draw first-come as before, testing and writing nothing.
- **Decals and layers:** a later polygon wins a tie, and by up to 1e-5
  of its depth per polygon between (`DEPTH_BIAS`), which covers rounding
  between a road marking and the road; and it is *tested* with its 1/z
  raised by 0.5% (`DEPTH_TOLERANCE`, `raster::depth_test_value`) while
  it *writes* its own depth. That keeps the game's order for a decal
  laid on a faceted surface of its model, which can lie a little behind
  the facets (measured: segawski's jersey text and logo 0.03-0.27%
  behind the jersey, lastbrnx's "109" sign 0.41% behind its tower, with
  5-78 polygons between, a rank allowance of 5e-5 to 7.8e-4). Polygons
  the game sorted within 5% of one depth of the group's first
  (`GROUP_SPAN`, including the header's "same depth as the previous
  polygon") form a group (`Placed::group`): a marker and its outline, a
  jacket over a body. They draw in the game's order among themselves,
  tested against what came before, and write depth when the group ends
  (software: pending writes; GPU: a colour draw that tests, then a
  depth-only draw; every tested polygon is drawn that way, since the
  test and the write differ).
- **1/z per pixel** is the software fill's: along the polygon's edges at
  the row's centre, then across the span. The GPU computes the same in
  the fragment shader (`docs/gpu.md`) instead of interpolating over its
  triangle fan, whose value on a quad that isn't flat depends on the
  diagonal.
- **Cut-out texels** that aren't drawn don't write depth.
- **Original order:** the setting off, the default, is bit-for-bit the
  old renderers.
- **Checked** (2026-11-09) by drawing 21 captured frames of a coin-start
  run (attract to play) of each of the 18 games at 1440x1080 with it off and
  on (`make depth-check`; 12 to 19 frames of 21 differ per game, by
  0.04% to 5% of the pixels, the most in `skisuprg`'s dithered scenes)
  and looking at the pictures of the most different frames of each of
  `topskatr`, `skisuprg`, `stcc`, `segawski`, `waverunr`, `hotd`, `bel`,
  `overrevb`, `sgt24h`, `dynabb97`, `indy500`, `schamp`, `lastbrnx` and
  `srallyc`, and the numbers of the rest (`overrev`, `doa`, `fvipers`,
  `vstriker`). Seen fixed: characters' limbs and clothes (topskatr's
  skater's leg, segawski's skier's back, schamp's tangled feet), a ladder
  and beam crossing (bel), overlapping tree billboards (topskatr). Seen
  as glitches while designing it, each answered by a rule above: a ramp
  and crate hidden by the sand quad (deep polygons), hotd's black shadows
  and the dither shadow under `overrevb`'s car vanishing (shadows,
  translucent), skisuprg's flag fills lost to their outlines and
  segawski's logo lost to the jacket (groups).
- **Loose ends found and fixed** (2026-11-10, `depth-check --probe`):
  - `sgt24h`'s grey band on the windscreen (and rear window) was the
    GPU's alone: the glass is the body's quad sent again from the next
    corner, so the fan split the two along different diagonals, and the
    body won by up to 0.6% of 1/z between them (a rank allowance of
    5.9e-4). The software drew the glass. Fixed by the GPU's 1/z above.
  - segawski's jersey text and the bottom edge of its logo lost whole
    rows to the jersey (both renderers): they are later quads of the
    same object block, 0.03-0.27% behind the jersey's facets, in the
    next group. Fixed by `DEPTH_TOLERANCE`. The "darkened logo" was this
    and nothing else: the logo's own pixels were the same off and on.
  - The same rule brings back decals the depth buffer had hidden:
    lastbrnx's "109" sign, hotd's building windows and a zombie's eye,
    segawski's logo on the skier's back and a roof lattice, banners in
    overrevb; and the GPU's 1/z brings back bel's ivy over its cliff,
    which only the GPU had hidden (the ivy is 1.85x nearer than the
    rock there). Confirmed on 2026-11-11 (`depth-check --against` a
    build of the commit before): that build's GPU drew the rock face
    over its ivy in frames 5100-6212 of a `bel` capture (up to 22,000
    pixels a frame), and now the ivy shows as with the buffer off. That
    the "rock face drawn over its neighbour" recorded as a fix on
    2026-11-09 was this artefact is likely but unconfirmed (no frame was
    recorded).
  - Checked on 2026-11-10 by drawing the captures of `topskatr`,
    `srallyc`, `lastbrnx`, `doa`, `sgt24h`, `segawski`, `bel`, `hotd`,
    `overrevb`, `schamp` and `skisuprg` against the earlier depth-on
    frames (`depth-check --against`) and looking at the most changed
    frames of each: the skater's leg, the tree billboards, the skier,
    schamp's feet and bel's vehicle in its hill stay fixed, with
    one-pixel shifts along intersection lines; the background rules are
    unchanged. Depth off is unchanged (state hashes, snapshots and
    `gpu-check` of `topskatr`, `srallyc`, `lastbrnx` and `doa` identical
    to before), and the software and GPU agree as well or better with it
    on (`gpu-check --depth`, native, pixels over 16 levels: `bel` 2.00% to
    1.95%, `sgt24h` 1.64% to 1.54%, `hotd`, `topskatr`, `lastbrnx`,
    `segawski`, `schamp`, `skisuprg`, `overrevb` within 0.01%).
- **Checked on all 18 games** (2026-11-11): a coin-start capture of
  each (21 frames; `stcc` sits on the starting grid from frame 3600),
  `depth-check` at 1440x1080 and at 1920x1080 16:9, depth-on frames
  compared with a build of the commit before the 2026-11-10 changes
  (`--against`), the most changed frames of `stcc`, `waverunr`,
  `dynabb97`, `indy500`, `overrev`, `fvipers`, `vstriker` and `bel`
  looked at, the differences probed. The 2026-11-10 changes moved
  depth-on towards the game's picture in every game: of the pixels
  they changed, 96-100% per game now equal depth off (`skisuprg` 92%)
  (decals and layers back: `indy500`'s sidepod lettering, `vstriker`'s
  shirt print, a `waverunr` banner and tree branches under their
  foliage, slivers through `dynabb97`'s clothes and hands, `bel`'s
  ivy). Those that moved away from depth off, where probed (`waverunr`),
  are cut-out foliage whose GPU colours differ off and on while the
  software's agree (unconfirmed for the other games). The 16:9
  view changes the same frames, give or take three (`vstriker`), and
  where looked at (`stcc`) shows the same kinds of difference in the
  wider sides. Seen as fixes with it on (the hidden polygon is 3-84% of 1/z
  behind, so not a matter of rounding): `stcc`'s roof struts behind
  a sign's pillar, `indy500`'s fence rail in front of the spectators,
  `overrev`'s crossed tree cards, `bel`'s soldier's legs in front of a
  beam (3.6%). Can't tell: `waverunr`'s tree on a cliff top hidden by
  the cliff face (3.3x nearer), `overrev`'s glass walkways against its
  building faces, `fvipers`' wall bases under the arena floor (9%) and
  costume layers (1-3%), `dynabb97`'s home plate (whole with it on,
  clipped by the dirt with it off; 0.9%).
- **Seen wrong with it on** (2026-11-11), none of them from the
  tolerance and none with a small generic fix: `stcc`'s pennants in
  the grandstands are planted in the crowd's flat card, 7.5% of
  1/z behind it, so the card cuts them in half (the game shows them
  whole); `dynabb97`'s close-up batter shows his black undershirt's
  collar through the jersey (the jersey 2.9% behind there; the two are sorted 0.4%
  apart but fall in different groups). The tolerance's trade-off
  showed once: an `overrev` banner 0.02% in front of the wall drawn
  after it is half covered again, as with the buffer off (the build
  before showed it whole).
- **Not verified:** against a real machine, any of it; and the
  trade-off of the tolerance: surfaces that cross within 0.5% of 1/z
  of each other keep the game's order there, so an intersection line
  can move by a pixel or two. The depth buffer isn't z-fight free: a
  decal more than 0.5% behind its surface, not sorted with it, still
  loses.

**Probing a pixel.** `make depth-check PROBE="X,Y@FRAME ..."` (`remodel
depth-check --probe X,Y@FRAME`, repeatable; output pixels of the
1440x1080 pictures, captured frame numbers; only those frames are drawn)
prints, for each polygon covering the pixel centre (after its clip
rectangle, the checkerboard and cut-out texels, each polygon once): its
place in draw order, window, rank, group, object block (the stream's
`0x1` command: one model's polygon list), sort key, why it keeps the
game's order if it does (`sky`, `shadow`, `checker`, `spread`), 1/z, its
depth, the buffer it is tested against and which polygon wrote it, and
pass or fail; then which polygon shows with the buffer off and on, the
GPU's and the software's colours there, and for a polygon the game drew
that lost, how much nearer the winner was against the rank allowance.
It follows `raster::probe_depth`, the software's rules with the original
textures. `AGAINST=DIR` (`--against`) compares each depth-on frame with
an earlier run's `depth-FRAME-on.png` in DIR (a frame without one was
drawn the same as depth off) and writes `against-FRAME.png` (before |
now | differences): what a change of the rules did.

### Batch texture upscaling

Done (2026-11-10). `make upscale-textures` turns a game's
`make dump-textures` output into a pack in one command: `remodel
texpack-upscale` (no ROM needed) prepares every dump, runs an external
upscaler once over the directory (`UPSCALER=`, a command template; ESRGAN
models through `realesrgan-ncnn-vulkan` are the tested example) or a
built-in bicubic resampler, and puts the results together. reModel only
drives the tool. Usage, options and limitations: `docs/texture-packs.md`,
"Batch upscaling". How the three open points were settled:

- **Luma.** The full-luma dump is upscaled and the renderer's existing
  per-channel lighting ratio applies luma, unchanged. That ratio assumes
  the replacement stands for the full-luma texture, so by default
  (`COLOUR=match`) the tool puts the dump's low-frequency colour back:
  lowpass(dump) - lowpass(result) added to each pixel, an opacity-weighted
  Gaussian of 2 source texels with the texture's addressing, which keeps
  the upscaler's detail and tiling. It reports images whose mean colour
  moved by more than `--drift` (6/255 per channel), before and after, in
  the pack's `drift.txt`. On topskatr realesrgan-x4plus moves 30 of 241
  textures (up to 14/255: it denoises speckled textures, which darkens or
  flattens them) and 909 of 5030 tiles; after matching, no textures and 2
  tiles (6.2/255). The built-in resampler moves none either way. Matching
  does not bring back what the model denoised away (the road's grain).
- **Alpha.** The upscaler never sees alpha: transparent pixels (texel 15,
  tiles' pen 0) get the colour of their opaque neighbours first, so no
  halo of their meaningless colour reaches the edge, and the image goes
  out as opaque RGB. Alpha is scaled here, bilinearly with the texture's
  addressing, and cut at 0.5: the rule the renderer uses for original
  translucent texels and pack images, so cut-outs keep their shape and
  stay hard-edged. Tiles' transparent pixels get pen 0's colour back.
- **Seamless tiling.** Each texture is padded by 16 texels with the
  renderer's own addressing (`enhance::address`), repeated or mirrored per
  axis as it is drawn, upscaled, and cropped. The dump index now records
  every way a texture is drawn (`wrap=`), not only the first; an axis drawn
  both ways, or unknown, is padded mirrored. Tiles are clamped (their
  neighbours vary), so art spanning several tiles can still show seams.

### Mipmaps and anisotropic filtering

Done, opt-in, off by default (2026-11-10), for texture pack images only; the
original textures keep their exact path. Settings: the player's `tex_filter`
(`--tex-filter off|mip|2|4|8|16`), the browser's `texfilter`
(`?texfilter=`), `remodel --tex-filter` for snapshots and `gpu-check`;
usage in `docs/texture-packs.md`. Render-only.

What exists:

- **Live packs on the GPU** came with it (the filter needs them): pack
  images are GPU textures, drawn with the software path's replacement and
  lighting rules (`docs/gpu.md`).
- **Mipmaps.** Each image's chain is built when it is first drawn:
  premultiplied by alpha, each level a box filter of the one above
  (`enhance::Rgba::mip_chain`; odd sizes weight by overlap), averaged in
  display values, not linear light. Cut-outs are decided after filtering
  (alpha under 0.5 is transparent).
- **Anisotropic filtering** is the GPU sampler's, up to 16x where the
  backend has it (`gpu::max_anisotropy`), driven by gradients of the texture
  coordinates. The software renderer has trilinear filtering only and
  draws 2..16 as `mip`.

How it was checked (2026-11-10):

- With no pack and the filter off, output is bit-for-bit unchanged: state
  hashes, snapshot frames, `gpu-check` and `depth-check` frames identical
  before and after, for `topskatr`, `srallyc`, `lastbrnx` and `doa`
  (`depth-check` also `sgt24h`, `segawski`).
- GPU pack frames match the software pack snapshot about as closely as the
  originals do: `topskatr` `gpu-check`, pixels differing by more than 16
  levels 1.58% with the originals, 1.54% with a 1:1 pack. With filtering
  on, GPU `mip` against software trilinear (a fine-checker 4x test pack):
  1.85% native, 2.5% at 1080p, against 3.4% for the exact path with the
  same pack.
- `make pack-check` against a 4x4 supersampled reference on `topskatr`
  (mean difference per channel): a harsh fine-checker test pack off 9.66,
  mip 4.70, 16x 4.84; a realesrgan-x4plus pack off 2.09, mip 2.06, 16x
  1.62. By eye: off shows moiré on distant ground, mip removes it but
  blurs, 16x keeps the ground's detail at grazing angles; no halos at
  cut-out edges.
- Cached texture keys are reused on about 97% of frames (render RAM changed
  on 136 of 4140 `topskatr` frames and 42 of 4140 `srallyc` frames); hashing
  every key uncached would cost 0.2-2.2 ms a frame. A full pack makes about
  600 draws a frame on `topskatr` (198-1047), where there were 1-4.
- Native player, scripted run on `topskatr`; browser in headed Chrome on
  Linux (a 1024x1024 image uploaded with its mipmaps cost a frame up to
  13 ms of draw against about 3 ms).

*Unconfirmed:* other games and upscalers; other browsers and phones;
`RECOMP=1` web builds; smooth motion's in-between frames and F5/F9 save
slots with a pack; the player's decode stutter; eviction past the memory
budget; a clone's fallback to its parent's pack. Tiles are still replaced
only in snapshots. Mips average display values, not linear light.

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
- Whether the depth buffer can be on by default: it looks right in
  the frames compared (see "Depth buffer") but changes pictures
  in every game, and the games' own layering tricks (large floors,
  dither, flat shadows) had to be left on the game's order. More
  games and long plays first.
- How much of this the browser can have: WebGPU covers everything up to
  the temporal upscaler and per-pixel lighting; ray tracing waits for
  the web platform.
