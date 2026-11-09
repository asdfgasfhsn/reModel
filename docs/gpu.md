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
  from the fragment shader (`fs_depth`: `model2::raster::depth_value` of the
  interpolated 1/z and the polygon's draw order in its window), compare
  greater-or-equal. Four pipeline modes: depth tested and written;
  tested only (the colour pass of a group of polygons sorted together);
  depth only (that group's depth, after its colour); and none
  (backgrounds, shadows, translucent and very deep polygons stay in the
  game's order). A window starts with a scissored full-screen draw that
  empties the buffer where it draws.

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

## Next

- A window and swapchain (winit) that presents the GPU layer.
- The 2D tile layer and HUD composition on the GPU. These still run on
  the CPU, after the 3D layer.
- Persistent buffers with dirty-range uploads of render RAM.
- Texture packs in the shader: replacements as GPU textures, using the
  same lighting model as the software path.
