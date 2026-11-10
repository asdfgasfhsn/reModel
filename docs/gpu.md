# GPU renderer

`crates/gpu` draws the Model 2 3D layer on the GPU through
[wgpu](https://wgpu.rs): Vulkan on Linux and Android, Metal on macOS and
iOS, DirectX 12 on Windows, and OpenGL as a fallback. The same code and
shader run on all of them.

It is what the native player (`docs/play.md`) and the browser build
(`docs/web.md`) display: both present each frame through
`Renderer::present_bus`, which draws the 3D layer and composes it with the
2D tile layers and HUD (drawn on the CPU, `video::layers_2d`) into the
window's surface. It is checked against the software rasteriser, which
stays the reference, off-screen.

```sh
make capture      # run into a race, saving every 300th frame's renderer inputs
make gpu-check    # draw each captured frame both ways, time and compare
```

## How it works

Both renderers draw from the same polygon list, `model2::raster::place`.
It covers polygons parsed from the geometrizer's output stream, sorted
into draw order and placed in the output view (resolution, widescreen,
HUD placement).

The GPU draws each convex polygon as a triangle fan. Everything goes in a
single render pass, by default with no depth buffer: GPUs keep primitive
order for overlapping pixels, which gives the painter's algorithm the
software renderer uses. Two opt-in options (`gpu::Quality`, decided in
`docs/enhancements.md`) change that, and with both off the output is
bit-for-bit what it was:

- **Multisampling** (`msaa` 4, or 8 where the adapter has it): the pass
  renders into a multisampled image resolved into the 3D layer, whose
  colour is then premultiplied by coverage; the compose pass blends it
  over the 2D base by that coverage.
- **Depth buffer** (`depth`): a 32-bit float depth attachment written
  from the fragment shader (`depth_at`: `model2::raster::depth_value` of
  1/z and the polygon's draw order in its window), compare
  greater-or-equal. 1/z is not the triangle fan's interpolation but the
  software fill's (`outline_iz`: along the polygon's edges at the row's
  centre, then across the span, from an `outlines` buffer of each
  polygon's points): for a quad that isn't flat the fan's value depends
  on its diagonal, and a decal sent as the same quad from another corner
  lost to its surface between the two diagonals (`sgt24h`'s windscreen).
  Three pipeline modes: tested only (a polygon's colour, its depth raised
  by `DEPTH_TOLERANCE` through the `DEPTH_FACTOR` override,
  `raster::depth_test_value`); depth only (its depth, after the colour of
  its whole group); and none (backgrounds, shadows, translucent and very
  deep polygons stay in the game's order). A window starts with a
  scissored full-screen draw that empties the buffer where it draws.

- **Texture packs** (`Renderer::set_pack`; `Quality::tex_filter`): see
  "Texture packs" below. With no pack set the pipelines, shaders and output
  are the ones above, unchanged.
- **Temporal anti-aliasing and upscaling** (`Quality::temporal`,
  `Quality::stipple`): see "Temporal path" below. Off (0, the default) the
  pipelines, shaders and output are the ones above, unchanged.

The fragment shader (`crates/gpu/src/raster.wgsl`) is a line-for-line port
of the software shading. It reads from the game's own render RAM and
palette RAM, uploaded unchanged as storage buffers:

- 4-bit texels from the texture sheets;
- bilinear filtering with the translucency rule;
- the checkerboard;
- luma tables and colour translation.

Lookups are therefore exact, and the renderers can differ only in
coverage and float rounding.

## Captures

`remodel run --capture DIR` saves, at each snapshot, everything the
renderers read:

- the raster stream;
- texture point RAM, render RAM, palette RAM and tile RAM;
- the sync offsets;
- the texture ROM, once per directory.

The format is in `crates/model2/src/capture.rs`. Captures are derived from
the game, so they live under `out/`.

## Results (`topskatr`, RTX 2080 SUPER, Vulkan)

21 frames from boot to mid-race, up to 2,400 polygons per frame.

| | software | GPU |
|---|---|---|
| native 496×384, mean / worst | 10.4 / 26.7 ms | 3.1 / 4.9 ms |
| 1080p 16:9, mean / worst | 106 / 281 ms | 7.3 / 11.2 ms |

A frame is 17.4 ms, so the software rasteriser can't draw races live even
at native size. The GPU times include rebuilding every buffer, uploading
the 4 MB render RAM and reading the image back each frame. A live
renderer keeps buffers, uploads only what changed and presents without a
readback, so it will be faster.

How the GPU output compares with the software renderer:

- **Coverage:** the same pixels are drawn in 99.96% of the frame
  (native).
- **Colour:** 9% of pixels differ, 1.5% by more than 16 levels in a
  channel. The differences sit on distant, large polygons, mostly sky and
  far terrain. Small differences in interpolated texture coordinates
  there pick a neighbouring texel. Splitting quads into triangles may also
  contribute, since a non-planar texture mapping interpolates differently
  that way. The hardware's own interpolation is unknown, so neither result
  is the reference here. Each frame's `gpu-check.png` shows software, GPU
  and the differences side by side.

## Multisampling and depth cost

`make gpu-check GPU_OPTS="--aa 8 --depth"` prints, after the comparison,
the whole frame on the GPU (3D layer, compose and the readback, which
dominates at 1080p) for each configuration, the mean over the captured
frames. RTX 2080 SUPER, Vulkan, 21 frames each (ms; the first column is
`off`):

| | off | 4x | 8x | depth | 4x+depth | 8x+depth |
|---|---|---|---|---|---|---|
| `topskatr` native | 8.94 | 9.11 | 9.09 | 9.56 | 9.70 | 9.88 |
| `topskatr` 1080p 16:9 | 35.22 | 35.81 | 36.25 | 37.69 | 38.78 | 40.28 |
| `sgt24h` native | 8.96 | 9.06 | 9.06 | 9.77 | 9.89 | 9.99 |
| `sgt24h` 1080p 16:9 | 36.13 | 36.86 | 37.16 | 39.30 | 40.27 | 41.67 |

So 4x costs about 0.6 ms at 1080p and 8x about 1, and the depth buffer
about 1.5-3 (a fragment-shader depth write turns off early depth tests,
and each window is cleared), well inside a 17 ms frame on this GPU. A
live player skips the readback that most of these numbers are.

Agreement with the software renderer (`gpu-check --depth`): the same
as without it: for `topskatr`, 0.04% coverage and 8.9% colour (1.6% over
16 levels) either way; `hotd` 1.94% against 1.94%; `bel` 5.1%. With
`--aa 8` (GPU 8x against software 3x3 supersampling, whole frames)
26-28% of native pixels differ at all and 3-8% by more than 16 levels:
the texture-coordinate differences described above plus the two
resolve filters.

## Texture packs

`set_pack` takes a `model2::enhance::TexturePack` (the player's directory
pack, the browser's fetched one; `docs/texture-packs.md`). Each frame, for
every textured polygon the renderer computes the texture's key
(`raster::tex_info`, as the snapshots do), asks the pack once per key and
uploads a new image as an `Rgba8Unorm` texture (`Found::Pending`: the
original is drawn and the pack is asked again next frame; `Found::Missing`:
remembered, the original). An image larger than the GPU's texture limit is
refused with one warning.

- **Keys cached.** Hashing the texels of every texture each frame is
  avoided by keeping the hashes (`raster::TexKeys`) while render RAM's
  version is unchanged; on `topskatr` render RAM changed on 136 of 4140
  frames and on `srallyc` 42 of 4140, so the cache holds on about 97% of
  frames. Hashing everything uncached would cost 0.2-2.2 ms a frame.
- **Own shaders.** A replaced polygon sets bit 5 of its flags and its image
  is a second bind group (group 1), bound per run of polygons sharing it.
  Frames that replace nothing use the original `vs`/`fs`/`fs_depth`
  pipelines. Frames that do use separate entry points (`fs_pack`,
  `fs_pack_depth`; `vs_pack`, `fs_packf`, `fs_packf_depth`), so the pack-off
  pipelines are not touched: with no pack and the filter off, output is
  bit-for-bit unchanged (state hashes and `gpu-check` frames compared on
  `topskatr`, `srallyc`, `lastbrnx`, `doa`; `depth-check` also `sgt24h`,
  `segawski`).
- **Exact path** (`fs_pack`, filter off): the shader does the software's
  bilinear sample itself with `textureLoad`, straight alpha weighted by
  alpha, the original's repeat or mirror per axis, the cut-out at alpha 0.5
  and the same per-channel lighting ratio from the original texel
  (`shade_replaced`). On `topskatr`, `gpu-check` with a 1:1 pack: 1.54% of
  pixels differ by more than 16 levels from the software's, 1.58% with the
  originals.
- **Filtered path** (`fs_packf*`, `tex_filter` mip or 2..16): the image is
  uploaded as its premultiplied mip chain and sampled with `textureSampleGrad`
  through a linear sampler (anisotropy up to what `max_anisotropy` reports),
  repeat or mirrored-repeat per axis; four samplers cover the address-mode
  combinations. The gradients are those of 1/z, u/z and v/z at the pixel
  centre (a second interpolant, so partly covered multisampled pixels get
  the plane's gradients), turned into texture-coordinate gradients. Alpha
  below 0.5 after filtering is cut out; colour is un-premultiplied for the
  lighting ratio. On `topskatr`, `gpu-check` with a fine-checker 4x test
  pack, `mip` against software trilinear: 1.85% of pixels differ by more
  than 16 levels at native size, 2.5% at 1080p (3.4% for the exact path
  with the same pack).
- **Draws.** Runs of polygons are split at each change of image, so a full
  pack makes about 600 draws a frame on `topskatr` (198-1047), where there
  were 1-4. `PackStats` (`Renderer::pack_stats`) counts polygons looked up
  and replaced, images drawn, draws, images uploaded and held, and bytes.
- **Memory.** Images are kept after upload, a mip chain adding about a third,
  up to a budget (512 MB natively, 192 MB on wasm); over it the least
  recently drawn are dropped, except this frame's, and uploaded again when
  next asked for. A change of `tex_filter` rebuilds the images that change
  form (exact or mipmapped). Large images' first appearance can cost a frame
  (the browser measured up to 13 ms of draw uploading a 1024x1024 image
  with mipmaps, against about 3 ms).
- **Checking filtering.** `make pack-check PACK=DIR`: GPU off, mip and 16x,
  software trilinear, and a 4x4 supersampled reference, with the mean
  distance of each from it (`topskatr`: a fine-checker test pack 9.66, 4.70,
  4.84; realesrgan-x4plus 2.09, 2.06, 1.62).

Not measured: the cost of the filtered path against the exact one per
pixel, and eviction past the budget.

## Temporal path

`Quality::temporal` (0 off; 100 TAA; 50-99 a render scale in percent;
`set_quality` clamps it to 50..=100, or 0 without `temporal_supported`)
and `Quality::stipple` (`Blend` or `Keep`). What it is for, the resolve's
rules and what was measured are in `docs/enhancements.md`; here, how it
sits in the renderer (`crates/gpu/src/temporal.rs`, `temporal.wgsl`, the
`_t` entry points at the end of `raster.wgsl`).

- **Targets**, kept between frames by label; any re-creation (a new size
  or scale) resets the history:

  | Target | Format | Size |
  |---|---|---|
  | `3d layer t` | `Rgba8Unorm`, premultiplied | internal |
  | `3d aux a`, `3d aux b` (this frame's and the last's) | `Rgba16Float` | internal |
  | `3d depth t` (with `depth` only) | `Depth32Float` | internal |
  | `history a`, `history b` | `Rgba16Float`, premultiplied | output |
  | `3d sharpened` (with sharpening only) | `Rgba16Float`, premultiplied | output |
  | `resolve inspect` (with inspection only) | `Rgba32Float` | output |

  Internal size is the output size at 100%, else `max(1, round(w * s))`
  by `max(1, round(h * s))`, with `rs` the exact ratios per axis.
- **Pass order** (`encode_temporal`): the 3D pass (`encode_3d` with a
  `TemporalPass`) draws into the colour and this frame's aux target with
  one sample (MSAA is forced off), jittered; the resolve pass
  (`encode_resolve`, a fullscreen `fs_resolve`) reads the colour, both aux
  targets and the last history through a linear clamp sampler and writes
  the other history; compose, unchanged, binds the history view in place
  of the 3D layer (an `Rgba16Float` texture is valid there: it is read
  with `textureLoad`, one to one). The history is read back at the
  previous position with Keys' cubic (`TemporalTuning::cubic`, C = 0.75;
  0.5 would be Catmull-Rom) in 5 bilinear taps, and clipped to the
  neighbourhood's box widened by `CLIP_SLACK` (0.5/255, a WGSL constant)
  on every side, so half-float rounding in the history can't trip the
  clip in a channel whose box has no width (`docs/enhancements.md`).
  Where a thin polygon's sample (aux a < 0) is among the 3x3 and at least
  as near as the pixel's own surface, its motion is the one used
  (`TemporalTuning::thin_motion`, k3.w in the uniform). Two
  passes run only when asked for: with `TemporalTuning::sharpen` above 0,
  `fs_sharpen` draws the new history sharpened into `3d sharpened`, which
  compose reads instead (the history itself stays unsharpened); with
  `set_temporal_inspect`, a second resolve (`fs_inspect`, the same
  `resolve()` function) writes what it decided per pixel into `resolve
  inspect`, leaving the picture and the history alone. `ResolveIo` is the
  pass boundary: the only code that knows the algorithm, and what a
  library upscaler would take instead.
- **Pipelines.** Cached by `(samples, Mode, Variant, temporal)`; the
  temporal ones come from `raster_pipeline_t` (one sample, two colour
  targets, premultiplied-alpha blending on the colour for the blended
  checkerboard), so the off path's construction is the old code. Entry
  points: `vs_t`, `fs_t`, `fs_depth_t`, `fs_pack_t`, `fs_pack_depth_t`,
  `fs_packf_t`, `fs_packf_depth_t`; the window clear reuses
  `vs_clear`/`fs_clear` with both write masks empty. Each `_t` fragment
  shader works out the unjittered sample point `q = pos / rs - jitter`,
  tests the clip rectangle and (with Keep) the checkerboard there, then
  calls the existing `shade*` functions; no existing WGSL function was
  changed. For a temporal frame the CPU sets each polygon's own clip
  rectangle to ±3e38 and clears its checker bit, so those tests inside
  `shade*` never fire, and sets bit 6 (checker) and bit 7 (has motion).
  `fs_packf*_t` scale the texture derivatives by `min(rs)`.
- **Thin polygons** (`temporal::thin`, in `encode_3d`'s temporal branch
  only): a polygon whose width across (the least, over its edges, of the
  farthest point from the edge's line) is under `TemporalTuning::thin`
  (1) internal pixel and whose longest extent is at least `thin_len` (2)
  internal pixels, and none of whose long edges (half its longest or
  more) another polygon of the frame shares (`temporal::Edges`, edge
  ends rounded to 1/8 px; a mesh's slivers share theirs), gets flag bit
  8, and its points are moved apart across
  its middle line (`Thin::widen`; `prev` by the same offset times W) until
  it is `cover` (1) internal pixel wide. Bits 16-23 hold its coverage,
  width over widened width (255 = 1); `cover_t` multiplies the `_t`
  fragment's colour by it, and the colour target's premultiplied blending
  lays it over what is behind. So every sample row or column across the
  polygon hits it once whatever the jitter, at its share of the colour,
  where unwidened it was hit by some jitter phases and missed by others.
  The aux target is written as usual (blending is on the colour target
  only), with the thin mark.
- **Vertex format**: 36 bytes, `xy` (`Float32x2`), `(iz, uz, vz)`
  (`Float32x3`), the polygon index (`Uint32`) and `prev` (`Float32x3`,
  location 3): `Placed::prev[k]`, the previous position in output pixels
  as homogeneous [X, Y, W], or `(x, y, 1)` on a repeated frame or without
  data. The off path keeps 24 bytes.
- **Bindings.** `raster_layout_t` is the five bindings of `raster_layout`
  plus a uniform `ScreenT` at binding 5 (`rs`, the jitter in output
  pixels, the derivative scale, flags bit 0 Keep, and `clip_base`). The
  real clip rectangles ride in the `outlines` storage buffer (binding 4),
  after the outline data, from `clip_base`: a fifth storage buffer in the
  fragment stage is over `Limits::downlevel_defaults`'s four per stage,
  which `gpu::limits` asks for.
- **Aux target encoding**: r, g the motion in output pixels (previous
  position minus `q`), clamped to ±16384; b the fan-interpolated 1/z,
  clamped to [6.2e-5, 6.0e4], negated for a blended checker fragment, 0
  where nothing was drawn; a the expected previous 1/z (1/z over W), 0
  without motion data, negated on a thin polygon (flag bit 8; readers take
  its magnitude, `TemporalDebug` splits it into `aux[3]` and `thin`; a
  thin polygon without motion data isn't marked, which costs only its
  motion dilation). Every value is finite: the CPU drops a polygon's
  `prev` with a non-finite value, W ≤ 0 or the wrong length, and the
  shader rejects W ≤ 1e-6.
- **Jitter** (`temporal::jitter`): Halton(2,3) − 0.5 in internal pixels,
  index `1 + count % L`, `L = clamp(round(8 / s²), 8, 32)`; (0, 0) when
  `count` (frames resolved since the last reset) is 0. `vs_t` adds it in
  output pixels (`jitter / rs`).
- **Frame identity.** `FrameTag { frame, t }` (the smooth frame's number;
  `t` 1 for the game's own frame, less for an in-between) and
  `Step { Reset, Repeat, Blend { tp }, NoMotion }` from `step(last, now,
  cut)`: no last tag gives NoMotion; a cut on a new frame Reset; the same
  tag Repeat; else `tp = (last.frame - now.frame) + last.t` (the previous
  image's time on this frame's plan), Reset outside [-2, now.t).
  - `present_bus(bus, None, ..)` with temporal on takes the engine's
    `smooth_frame()` and its `motion()`, tagged `(frame, 1)`;
    `present_bus_last` the geometrizer's `last_smooth()` (filled only
    with `set_keep_last`); `present_bus_motion(.., Some(&BusMotion))`
    an in-between at its own tag (`BusMotion::of`); `present` and `frame`
    with explicit polygons whatever `set_temporal_frame` set, else
    NoMotion. `draw` (gpu-check's 3D read-back) always takes the off path.
  - `reset_history` (front ends call it on a state load, a game start, a
    machine switch); `set_quality` resets on any change of the normalised
    quality. Keep falls back to Blend per frame in `encode_temporal` when
    `view.sy * rs.y < 1.5`, so the returned quality still says Keep.
  - Tools and tests: `temporal_debug` (internal colour and aux read back),
    `temporal_history`, `set_temporal_jitter`, `set_temporal_passthrough`
    (a nearest copy instead of the resolve), `set_temporal_tuning`
    (`TemporalTuning`), `set_temporal_sharpen` (0 to 1; off by default,
    not offered by the player), `set_temporal_inspect` with
    `temporal_inspect` (per pixel: the rules that fired, this frame's
    share, the clip distance, the trust term), `frame_bus` (`present_bus`
    off-screen with an RGB read-back).
- **Tests.** `temporal::tests`: `steps`, `jitter_sequence`, `halves`,
  `identity_at_100_without_jitter` (the first temporal frame through the
  resolve equals the off frame byte for byte, depth off and on; Repeat and
  Blend frames with the pass-through too; exact aux values; the off path
  after temporal frames equals a fresh renderer's), `history_stays_finite`
  (NaN and infinities planted in both histories and aux targets),
  `still_scene_converges`, `alternating_colour_is_not_averaged`,
  `rounding_in_the_history_is_not_clipped` (every opaque history texel's
  alpha set one half-float step below 1 before the last frame: the
  picture within one level of the unpoisoned one; fails without
  `CLIP_SLACK`), `sharpening_stays_within_the_neighbourhood`,
  `thin_polygons` (classification and widening),
  `thin_line_stays_whole_when_upscaled` (a line half an output pixel
  thick under jitter at 50% and 67%: every column along it shows at least
  a sixth of its contrast on every frame from 24 to 40; without the
  thin-polygon handling some column drops to nothing) and
  `thin_line_leaves_no_trail` (at 50% and 100%, six frames after the line
  is gone, has jumped away, or an opaque polygon came in front, the
  picture where it was is the background or the occluder within 3
  levels); and
  `only_temporal_entry_points_use_binding_5` beside
  `shaders_parse_and_validate`, which validates `temporal.wgsl` too. The
  GPU tests print "skipped" and pass without an adapter.
  `remodel temporal-probe` and `make temporal-check` are the tools on
  captured frames (`docs/usage.md`).

Cost and memory (`temporal-check --bench`) are tabled in
`docs/enhancements.md`: on an RTX A2000 Laptop, TAA about 1.1 ms over off
at 1080p and 4.4 ms at 4K, 67% about the same as off, 50% less; 71 MB of
targets at 1080p and 100%. Measured before `CLIP_SLACK` and the C = 0.75
cubic, not since; neither adds work (the same 5 taps).

## Smooth shading

`Quality::shading` (`Flat`, `Vertex`, `Pixel`), `crease` and
`shading_fine` (docs/enhancements.md, "Smooth shading"). The
`present_bus*` calls shade the stream themselves (`build_polys`:
`light::shade_with` with the game's rules from `set_light_rules` and the
side channel from the engine or `set_light_frame`, then
`raster::build_with`); a repeat of the same stream, side channel and
options reuses the last result (`LightCache`). Polygons passed in are
drawn with whatever `Poly::lit` and `Poly::pixel` hold.

- **Data, no new bindings.** A lit polygon's corner values ride in the
  `outlines` storage buffer (binding 4) that the depth buffer uses:
  directory entry `(first, count, flags, normals)` with flags bit 0 lit,
  bit 1 per pixel, bit 2 finer levels, bits 8-15 and 16-23 the luma range
  that keeps the polygon's hue (`Poly::lit_range`); its points carry
  luma/z in w; per pixel, `count` normal/z entries and two lamp entries
  (`lamp_eval`). Vertices, vertex entry points and bind group layouts are
  unchanged.
- **Entry points.** Frames with a lit polygon draw with the `_l` twins of
  every fragment entry point (`lit_entry`; plain, depth, pack, filtered
  pack, and each `_t` for temporal); every other frame, and shading off,
  takes the old pipelines and shader text, so the off path is bit for bit
  what it was (the regression baseline). `poly_px` interpolates the luma
  (or normal) along the outline's edges at the row's centre and across the
  span, as the software fill does (`lit_luma`), truncates and clamps it.
- **Finer levels** (bit 2): `poly_px` also keeps the luma before
  truncation (`lit_fine`, `lit_range`, private variables), and `shade_l`
  and `pack_light_l` blend the two colour columns `fine_level` picks
  (`raster::fine_level`, the same arithmetic in f32), rounded as the
  software rounds.
- **Tests** (`temporal.rs`, `shading_off_path_and_lit_pipelines`): no lit
  polygon gives the off frame byte for byte with and without depth and
  temporal, finer levels on or off; corners at the polygon's own luma or
  normal give the off frame; varying corners, per-pixel normals, the hue
  clamp and finer levels agree with the software renderer as well as flat
  drawing does (finer levels within one step of rounding).

## Next

- The 2D tile layers and HUD on the GPU: they are drawn on the CPU and
  only composed on the GPU.
- Persistent buffers with dirty-range uploads of render RAM.
- Tile (2D layer) replacements live: only the 3D textures of a pack are
  used on the GPU so far.
