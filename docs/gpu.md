# GPU renderer

`crates/gpu` draws the Model 2 3D layer on the GPU through
[wgpu](https://wgpu.rs): Vulkan on Linux and Android, Metal on macOS and
iOS, DirectX 12 on Windows, and OpenGL as a fallback. The same code and
shader run on all of them.

This is a preflight. The renderer works off-screen and is checked against
the software rasteriser, but nothing displays it yet: the live window
comes next.

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

## Next

- A window and swapchain (winit) that presents the GPU layer.
- The 2D tile layer and HUD composition on the GPU. These still run on
  the CPU, after the 3D layer.
- Persistent buffers with dirty-range uploads of render RAM.
- Tile (2D layer) replacements live: only the 3D textures of a pack are
  used on the GPU so far.
