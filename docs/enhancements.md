# Visual enhancements: roadmap

How reModel could make the games look better than the hardware did:
sharper edges, higher-resolution textures, better models, and modern
lighting up to ray tracing. A plan to track: each item says whether it is
done, planned or an idea, and the done ones are described as built.
Started 2026-11-09.

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
| Textures | Texture packs: dump, replace at any size; live on the GPU in the player and the browser | done (`docs/texture-packs.md`); live display 2026-11-10, live tiles 2026-11-11 (wide views' added sides keep original tiles) |
| Textures | Batch upscaling pipeline (dump → upscale → pack) | done (2026-11-10, `docs/texture-packs.md`) |
| Textures | Mipmaps from pack images, anisotropic filtering | done, opt-in (2026-11-10), player and browser; anisotropic GPU only (snapshots draw it as mipmaps) |
| Image | Anti-aliasing: MSAA 4x/8x on the GPU, supersampling in software | done, opt-in (2026-11-09); 8x native only, where the GPU has it (the browser has 4x) |
| Image | Depth buffer | done, opt-in, partial (below), player and browser; current rules checked on all 18 games 2026-11-11 |
| Image | Temporal anti-aliasing and upscaling (motion vectors, jitter) | done, opt-in, native player only (2026-11-10): needs the native geometrizer, replaces MSAA; thin polygons (wires) stay whole at 67% and 50% (2026-11-11), animated objects without motion data look soft (below) |
| Image | FSR 2 or DLSS in place of the built-in resolve, native only | idea; FSR checked and parked (2026-11-11, below) |
| Models | Model replacement packs, keyed by ROM address | planned |
| Models | Automatic smoothing (vertex normals, curved surfaces) | vertex normals done (smooth shading, below); curved-surface tessellation an idea |
| Models | Level of detail and draw distance overrides | idea (optional patch) |
| Lighting | Smooth shading: per-vertex and per-pixel lighting with the game's own formula, finer brightness levels | done, opt-in (2026-11-11), player and browser; 2A formula unconfirmed (below) |
| Lighting | Material maps in packs (normal, specular, gloss) | planned; pack format with manifest, surface keys, map names and `make pack-lint` done (2026-11-11, `docs/texture-packs.md`, "Pack format"); loading onto the GPU (shared budget, `--materials`) done, drawing not built |
| Lighting | Pack-declared highlights where the game casts none | idea, deferred (2026-11-11): maps only scale the game's own specular; manifest keys reserved (below) |
| Lighting | Light data on the chip geometry path (materials and smooth shading there) | idea, not investigated (below) |
| Lighting | Shadows, bloom, HDR tonemapping, better fog | idea |
| Lighting | Hybrid ray tracing (shadows, reflections, AO) | idea |
| Lighting | Path tracing with denoising | idea (research) |
| Audio | Replacement soundtracks | done (`docs/mods.md`) |

## Order of work

1. **Anti-aliasing and a depth buffer** (done). Cheap, universal and
   groundwork for what follows.
2. **Batch texture upscaling** (done). The biggest visible gain for the
   least work.
3. **A temporal upscaler** (done as temporal anti-aliasing with a render
   scale; its open problems are below). Its inputs (jitter, motion
   vectors, depth, render scale) are what FSR 2 or DLSS would take.
4. **Smooth shading** (done) **and material maps.**
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
- With temporal anti-aliasing on (below), multisampling is off.

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
- **Off by default** (the project's "original by default"); whether it
  could be on is an open question, with the evidence so far under
  "Open questions" below. Measured per game (2026-11-11, 1440x1080, 21
  frames): frames changed 10-19 of 21, mean share of pixels changed
  `vstriker` about 0.001%, `fvipers`, `doa`, `srallyc`, `sgt24h`,
  `stcc` 0.01-0.03%, `dynabb97`, `indy500`, `overrevb`, `overrev`,
  `lastbrnx` 0.04-0.07%, `schamp`, `topskatr`, `waverunr`, `hotd`,
  `segawski` 0.1-0.17%, `bel` 0.5%, `skisuprg` 0.7%.
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
budget. Mips average display values, not linear light. A clone's fallback
to its parent's pack: checked in the player and the browser (`overrevb`).

**Live tiles** (2026-11-11). A pack's `tile_<key>.png` images are drawn live
too, at output resolution, by the snapshots' rules (`docs/texture-packs.md`,
"Tiles live"; `docs/gpu.md`). Decided:

- Keys stay the dump's (pattern and the colours of the pens it uses), so
  dumps, packs and `make upscale-textures`' `TILES=` modes (which upscale
  the coloured image) are unchanged; a fade or palette cycle draws the
  original for those frames. Keying on the pattern alone and colouring an
  indexed image would survive fades but needs another image format.
- The GPU composes the replaced tiles itself from a per-pixel tile map and
  an atlas; the 496x384 CPU layers stay as they were for everything else,
  so packs without tile images, and frames with nothing replaced, compose
  exactly as before.
- Sampling is per tile (clamped, no padding, no bleeding); seams can only
  come from the images (art spanning tiles upscaled tile by tile).
- The sides a wide view adds keep the original patterns, as in snapshots.
- The software renderer already did this in snapshots; the player and the
  browser are GPU only, so nothing more was needed there.

### Temporal anti-aliasing and upscaling

Done, opt-in, off by default (2026-11-10), in the native player only.
Settings: the player's `temporal` (`off`, `taa` for full size, or a render
scale from 50 to 100; the menu and launcher offer off, TAA, 75%, 67% and
50%; `--temporal`) and `stipple` (`blend` or `keep`, below; `--stipple`);
usage in `docs/play.md`. Tools: `remodel temporal-probe` and `make
temporal-capture` / `temporal-check` (`docs/usage.md`). Render-only: state
hashes unchanged. With it off the renderer takes the old code, pipelines
and 24-byte vertices, and its output is bit-for-bit what it was (state
hashes, `gpu-check` default, `--aa 4`, `--depth` and `--aa 8 --depth`, pack
`gpu-check` off/mip/16x, `depth-check` and `pack-check` frames of four
games identical to before, checked after the raster side and again after
the resolve went in). The
browser does not get it (decided); it keeps MSAA.

Rendering at a lower resolution and reconstructing at the display's
lets expensive items (ray tracing, later) run where they otherwise
couldn't; at full size the same machinery is anti-aliasing.

**How it works.**

- **Jitter.** Each frame the 3D layer is drawn offset by a sub-pixel
  amount, a Halton(2,3) sequence in internal pixels whose length grows as
  the scale drops (`round(8 / s²)`, 8 to 32 positions: 8 at 100%, 32 at
  50%). Only the 3D pass's vertex shader moves; tiles, the 2D HUD and
  compose aren't jittered, and the clip rectangles, the checkerboard and the
  depth buffer's 1/z are evaluated at the unjittered sample point. The first
  frame after a reset isn't jittered, so a one-off frame (`--shot`, the
  frame after a cut) is the plain picture: at 100% it equals the frame with
  temporal off, byte for byte (a GPU unit test through the resolve;
  `temporal-probe` checks it on captures).
- **Motion vectors, exact per vertex.** Smooth motion's plan
  (`model2::smooth`, `docs/hw/geometrizer.md`, "Smooth motion") already
  matches each of frame N's transforms with frame N-1's. Running frame N's
  list once more through the native geometrizer with the matched previous
  transforms (`smooth::Frame::motion`, once per game frame, cached) gives
  the same list in the previous pose: a stream with the same layout, word
  for word except positions (checked on every non-cut frame of 4,142-frame
  runs of `topskatr`, `srallyc`, `sgt24h` and `vstriker`). Each vertex's
  previous position is its position in that stream (for an in-between, a
  mix of the two streams by its time), projected with the previous frame's
  window centres, so `topskatr`'s camera bob is exact.
  `raster::build_motion` carries it through clipping as a homogeneous
  [X, Y, W] per point (`Poly::prev`, `Placed::prev`), which interpolates
  perspective-correctly across the polygon. No motion data (and the
  resolve's no-data path) for: objects the plan didn't match or snapped;
  objects matched with a transform keyed by another model (a pose model
  swapped in, `vstriker`'s players and its ball trail: posed with the old
  transform, they gave wrong vectors on about 1% of its 3D pixels before
  this rule); direct polygon data; a polygon with a vertex whose previous
  position was behind the eye; and frames with a camera cut. The replay
  reads polygon RAM as frame N-1 left it, so a model the list rewrites
  every frame (`lastbrnx`'s fighters' joint pieces) is frame N-1's shape
  too; one whose words the list writes twice in a frame, or after
  reading them, gets no motion data (`docs/hw/geometrizer.md`, "Motion
  vectors").
- **The aux target.** The 3D pass writes a second colour target,
  `Rgba16Float` at the internal size: the motion in output pixels (r, g),
  the visible surface's 1/z (b; negative for a blended checkerboard
  fragment, 0 where nothing was drawn) and the 1/z it had in the previous
  frame (a; 0 without motion data). The last polygon drawn at a pixel
  writes both targets, as for colour. Against the CPU's own vectors
  (`raster::visible`) the GPU's agree within 0.25 px on 99.48-99.999% of
  pixels showing the same surface, and within 0.05 px (or half-float
  precision) on 97.7-99.995%. Its a channel is negated on a thin polygon
  (next item); readers take its magnitude.
- **Thin polygons.** A polygon narrower than an internal pixel (wires,
  rails, poles far off) is hit by the samples of some jitter phases and
  missed by others. Measured on `topskatr`'s overhead wires at 50% (each
  wire two flat crossed quads, 0.25-1 output px wide): the clip hardly
  moved the history on the wire's pixels; what broke the lines was that
  this frame's share there is tiny when upscaling (0.003-0.08: the nearest
  sample is up to an output pixel away), so the picture is nearly all
  history, and that history was carried by the motion of the sky behind
  (the nearest sample's), 0.6 px a frame off the wire's own: the wire's
  accumulated colour drifted off it and smeared into broken bands 4-6 px
  thick. reModel projects on the CPU, so it knows which polygons are thin:
  in the temporal pass (`temporal::thin`), a polygon whose width across
  (the least, over its edges, of the farthest point from the edge's line)
  is under 1 internal pixel, at least 2 internal pixels long, and none of
  whose long edges (half its longest or more) another polygon shares is
  thin. That last test keeps out slivers of meshes seen edge-on, which
  the mesh around them already covers: `srallyc` draws tens of thousands
  of them a frame in its distant road and terrain, and treating them as
  thin cost it 0.01-0.03 (TAA 1.67, 67% 2.10, 50% 2.53 against 1.66,
  2.08, 2.50); with the test its scores are as before. A thin polygon is
  - drawn widened across its middle line to 1 internal pixel at its
    coverage (its width over 1, blended over what is behind), so every
    sample row or column across it hits it exactly once whatever the
    jitter, with its share of the colour: nothing is ever missed, the clip
    box always holds it, and the line converges like any edge;
  - marked in the aux target (the expected previous 1/z negated), and
    where such a sample is among the 3x3, at least as near as the pixel's
    own surface, the resolve takes its motion for the pixel, so the history
    around a wire moves with the wire.
  Only this frame's samples count, so nothing outlives the polygon: a
  thin-feature lock that kept the history of pixels where a thin polygon
  had been seen (unclipped, for up to 16 frames) was built and measured
  first, and dropped. It barely helped on its own (the clip wasn't the
  cause), it can't tell a polygon missed this frame from one that is gone,
  so a line that vanished or jumped left a fading copy behind, and with the
  widening it only cost (wires crop, 50%, 10.25 with it against 10.03
  without, before thin polygons were limited to those with no long shared
  edge; the final handling gives 10.01, below). No extra targets or memory; on the CPU a hash of the frame's
  polygon edges in the temporal pass. Off with `TemporalTuning::thin` 0
  (`--tune thin=0`; `cover`, `thin_len`, `isolated` and `thin_motion` are
  its parts).
- **The resolve** (`crates/gpu/src/temporal.wgsl`, `fs_resolve`), per
  output pixel:
  - this frame's estimate, a Gaussian reconstruction from the 3x3 internal
    samples around the pixel, with their minimum, maximum, mean and
    deviation;
  - the nearest sample's motion, classed as *no data* (none; previous over
    current 1/z outside [0.5, 2]; or longer than half the output's width:
    zero motion, clip only, this frame weighted 0.8), *hidden* (the
    previous position off the picture, or no texel of the last aux target
    around it within 10% of the expected 1/z: the history is dropped for
    this frame's samples, interpolated) or *seen*. Only when it isn't seen
    is a surface at least 1.1x nearer among the 3x3 tried: windows have
    depth scales of their own, so "nearer" isn't "on top" across them, and
    always taking the nearest pulled the pitch's motion into `vstriker`'s
    3D minimap and eroded it;
  - the history read at the previous position (Keys' cubic with C = 0.75,
    5 bilinear taps; the texel as it is when the pixel hasn't moved), then
    clipped towards the mean into the neighbourhood: the min/max box for a
    still pixel, the mean ± 1.25 deviations (2.0 at 50%) within it for a
    moving one, each widened by half a level (`CLIP_SLACK`, 0.5/255) on
    every side. Without the slack, a channel whose box has no width (alpha
    on opaque samples, blue on a red and yellow decal) and a history one
    half-float rounding step outside it gave a clip factor of 0, so the
    whole RGBA history was replaced by the 3x3 mean: dots along
    high-contrast edges wherever rounding had left alpha at 0.9995, and
    blur at random pixels inside textures. Catmull-Rom (C = 0.5), used
    first, softened slow motion, where the history is resampled every frame
    and the blend factor is low; sharper kernels for this frame's
    reconstruction scored worse (jitter noise);
  - the blend: this frame's weight is 0.12 at rest, raised by fast
    motion (to 0.6 at 8 output px a frame), by a change of motion since
    the last frame (to 0.8; catches disocclusion and objects resting on
    surfaces), by blended checkerboard (0.25), without motion data (0.8),
    and to a running mean after a reset; scaled by how near the sample is.
    When upscaling with the history trusted less than usual, the estimate
    leans to the interpolated samples instead of the nearest one.
  - Non-finite values are caught by their bit pattern and the history is
    clamped to [0, 1], so nothing can poison it (a test fills both
    histories and aux targets with NaN and infinities).
  - Optional sharpening (`fs_sharpen`): a contrast-adaptive cross filter on
    the resolved 3D layer before compose, clamped to the cross's minimum
    and maximum (no halos or ringing); the history itself is not
    sharpened. Off by default: at 0.3 it gained at most 0.01 on
    `topskatr` 4520-4619, lost 0.02 on `srallyc` and made still frames
    less steady. Reachable only as `Renderer::set_temporal_sharpen` and
    `temporal-check --tune sharpen=`; the player has no setting for it.
  The constants are `gpu::TemporalTuning` (`temporal-check --tune`;
  `cubic` and `sharpen` among them). The design's (0.25 at 32 px for fast
  motion, 0.3 without data) were worse than no anti-aliasing in fast
  motion (repeated resampling blur) and smeared `vstriker`'s animated
  players.
- **History.** Two `Rgba16Float` targets at the output size, ping-pong,
  premultiplied RGBA with alpha accumulated like colour; compose reads the
  history in place of the 3D layer, so 3D edges stay anti-aliased against
  tile skies that scroll on their own.
- **Frames and resets.** Each image is tagged with its game frame and its
  time (1 for the game's own, less for an in-between). A redraw of the same
  frame (the menu open, a repeat) adds a new jitter sample with zero
  motion and converges further; an in-between or a skipped frame
  reprojects by the matching fraction (up to two frames back,
  extrapolated). The history starts again on a camera cut (smooth
  motion's cut test), a save state load, a game start, a resize, any change
  of the quality settings and a switch between run-ahead's machine and the
  game's; the frame after is the plain one and the next average in.
- **Render scale.** At a percentage the 3D pass's targets (colour, aux,
  depth) are that much smaller in each direction; history, compose, tiles
  and the 2D HUD stay at the output size. Filtered pack images (`tex_filter`
  mip or anisotropic) choose their mip level as at the output size (a
  bias of log2 of the scale); the original textures have no mips.
- **The checkerboard** (`stipple`). Accumulated with exact jitter, the
  games' screen-door translucency does not melt into a blend: at two or
  more output pixels per game pixel the result is an anti-aliased checker
  pattern, and when upscaling it can't be reconstructed in motion (moiré,
  uneven melting). So there are two choices. `blend`, the default: a
  checker polygon is drawn as an even 50% blend, steady at every scale.
  That changes the look (smooth see-through instead of a screen door), and
  two stacked checker layers give 75% where the original shows the top
  one only. `keep`: the pattern as the game draws it, decided at the
  unjittered sample point; only while the 3D layer has at least 1.5
  internal pixels per game pixel vertically (at 1080p: 100%, 75% and 67%,
  not 50%; at the native 384 lines never), else that frame blends.
- **MSAA is off** while temporal is on (`set_quality` forces one sample):
  jittered accumulation does its work, and a multisampled aux target would
  resolve to averaged motion and depth at exactly the edges where they
  matter. The depth buffer, texture packs and their filtering combine with
  it.
- **Needs the native geometrizer** (the motion data): with `--geo chip`
  the player turns it off and says so. And 16-bit float targets that
  render and filter (`gpu::temporal_supported`; WebGPU and Vulkan have
  them, the OpenGL fallback may not).

**What it costs.** GPU time per frame (`temporal-check SEQ --bench 30`:
30 frames with 3D, each drawn 8 times, wall time of a `present` into an
offscreen target, waited for; medians in ms; NVIDIA RTX A2000 Laptop,
Vulkan). Measured before the clip slack and the C = 0.75 cubic went in
and not measured again since; neither adds work to the default path (the
cubic uses the same 5 taps, and the sharpening and inspection passes run
only when asked for):

| | off | MSAA 4x | TAA | 67% | 50% |
|---|---|---|---|---|---|
| `sgt24h` 1080p | 2.81 | 3.00 | 3.95 | 2.84 | 2.42 |
| `srallyc` 1080p | 2.88 | 3.06 | 4.05 | 2.96 | 2.54 |
| `sgt24h` 4K | 8.60 | 9.14 | 13.03 | 8.59 | 6.87 |
| `srallyc` 4K | 8.57 | 9.10 | 13.13 | 8.60 | 6.92 |

TAA costs about 1.1 ms over off at 1080p and 4.4 ms at 4K (the second
target, 36-byte vertices, the resolve); 67% about pays for itself and 50%
is cheaper than off. Memory for the temporal targets (colour 4 bytes and
two aux targets 8 each per internal pixel, two histories 8 each per output
pixel; the depth target is 4 more per internal pixel with `depth`):

| | 100% | 67% | 50% |
|---|---|---|---|
| 1080p | 71 MB | 49 MB | 42 MB |
| 4K | 285 MB | 198 MB | 166 MB |

On the CPU, temporal turns smooth motion's recording on (0.05-0.5 ms a
game frame) and adds the replay for the previous pose, once per game
frame: 0.2-0.4 ms on 2C, 1.4-1.8 ms on 2B (smooth motion's in-between
costs, `docs/hw/geometrizer.md`), whether or not in-betweens are shown.

**How good it is.** `make temporal-check` on sequences of 80-120 frames
at 1080p 16:9, every 20th frame against `gref`, the GPU's own 4x4
supersampled plain frame (mean difference per channel; lower is better;
in brackets the share of pixels over 16 levels). `naive` is the plain
frame drawn at that scale and enlarged bilinearly. The last column is the
frame-to-frame change of still 3D pixels (reprojected, outside blended
checkerboard, over every frame), off against TAA; lower is steadier. GPU: the RTX A2000. Measured with the clip slack and the
C = 0.75 cubic, and again with the thin-polygon handling (2026-11-11:
`topskatr`'s race gained 0.03-0.06, TAA's still-pixel change there rose
1.33 to 1.37, the other sequences moved by at most 0.01; `topskatr`
4520-4619 was not measured again); `topskatr`'s attract sequence was captured again for
this, so its numbers differ from an earlier capture of the same frames
(off 1.07 there, 1.23 here).

| Sequence (frames) | off | MSAA 4x | TAA | 67% | naive 67% | 50% | naive 50% | still-pixel change, off / TAA |
|---|---|---|---|---|---|---|---|---|
| `topskatr` attract 1000-1119 (3 cuts, 135 px pans) | 1.23 (2.65) | 1.10 (2.26) | **1.09** (2.29) | 1.59 (3.52) | 1.63 (3.62) | 2.09 (4.73) | 2.22 (5.05) | 1.00 / 1.40 |
| `topskatr` race 3500-3619 (overhead wires) | 2.98 (6.12) | 2.69 (5.57) | **2.18** (5.37) | 3.04 (7.27) | 3.46 (7.57) | 3.87 (8.96) | 4.48 (9.60) | 1.89 / 1.37 |
| `topskatr` 4520-4619 (shadow, grass) | 1.21 (2.41) | **1.08** (2.04) | 1.17 (2.38) | 1.51 (3.10) | 1.52 (3.19) | 1.87 (3.84) | 2.01 (4.18) | 2.59 / 1.24 |
| `srallyc` driving 3450-3569 | 1.95 (3.15) | 1.87 (2.81) | **1.66** (2.35) | 2.08 (3.07) | 2.25 (3.60) | 2.50 (3.94) | 2.86 (4.72) | 3.19 / 2.72 |
| `sgt24h` race 2850-2969 | 4.33 (11.64) | 4.11 (10.98) | **3.61** (10.07) | 4.63 (13.12) | 4.93 (13.54) | 5.53 (15.65) | 6.12 (16.93) | 1.35 / 0.70 |
| `vstriker` attract 470-549 | 2.78 (7.62) | 2.61 (7.04) | **2.31** (5.07) | 3.11 (7.69) | 3.39 (9.09) | 3.86 (10.71) | 4.27 (12.84) | 3.85 / 2.18 |
| `vstriker` match 3300-3419 | 3.76 (12.50) | 3.38 (11.31) | **2.98** (7.26) | 4.19 (11.59) | 4.63 (13.98) | 5.26 (15.28) | 6.05 (19.55) | 4.84 / 3.01 |

- In motion, TAA beats off on all seven sequences (by 3-27%) and MSAA 4x
  on five; it ties MSAA 4x on `topskatr`'s attract (1.09 against 1.10)
  and loses to it on `topskatr` 4520-4619 (1.17 against 1.08).
- Upscaling beats the naive enlargement on all seven, at 67% and at 50%;
  on `topskatr` 4520-4619 at 67% only just (1.51 against 1.52).
- Still (the first sampled frame drawn 40 times, against `gref`): TAA
  beats MSAA 4x by 20-40% on every sequence (`sgt24h` 2.58 against 3.99,
  `srallyc` 1.40 against 1.79, `vstriker` match 2.11 against 3.45).
- Stability: TAA is steadier than off on every sequence but `topskatr`'s
  attract, whose frames right after its three cuts restart from jittered
  samples (1.40 against 1.00); on its sampled frames alone (those paired
  with the reference's) TAA is 0.44 against off's 0.57.
- Sharpness inside polygons (mean |Laplacian| of luma where a pixel's 3x3
  shows one polygon, over `gref`'s): plain frames are 1.38-1.61 times the
  reference's (point samples of the games' bilinear textures, and texture
  aliasing), TAA 0.96-1.14: about as sharp as the supersampled reference
  without the aliasing. 67% is 0.76-1.03, 50% 0.63-0.97.
- `temporal-check`'s pass criteria (TAA no worse than off, upscaled no
  worse than naive, a still frame no worse than 1.1 times MSAA 4x and its
  change under 0.25 levels) pass on six of the seven sequences. The one
  failure is `vstriker`'s match: the still frame's change is 0.338
  (high-frequency crowd textures under jitter). The criteria are kept as
  set, not loosened.

By eye (crops of the sequences above against `gref`):

- **100% (TAA).** Moving silhouettes and road edges match the reference
  closely; still frames are the cleanest of any setting; cuts start clean
  (the frame after shows no trace of the previous shot); blended shadows are
  a steady 50%; the 3D HUD stays solid. Hard texel edges the game leaves
  (the skater's cut-out shadow, laces, a shirt print on `topskatr`) are
  kept. Animated objects without motion data (`topskatr`'s waving
  chequered flags, `vstriker`'s players' limbs) look soft. Fast driving
  softened grass, rock and distant trees visibly before the clip slack and
  the sharper cubic; not looked at again since.
- **67%.** Edges clean on roads, and still frames better than MSAA 4x.
  Thin geometry (`topskatr`'s overhead wires, rails) is drawn as whole,
  even lines, a little softer and darker-edged than the reference; it
  used to break into speckled, dotted bands. In fast motion edges fall
  back to the interpolated, internal-resolution look (by design, rather
  than smear). Animated players without motion data look as blocky as
  naive.
- **50%.** The same, more so: wires are whole lines but visibly softer,
  and where several wires run close together the lower ones still show
  some grain; fast pans are blocky. About MSAA 4x's quality when still,
  at less than plain rendering's cost.

**`topskatr`'s FINISH sign**, whose colour the game toggles every frame
(`docs/games/topskatr.md`), flickers exactly as the game draws it: in the
attract demo, frames about 19590-19607 of a coin-start run, TAA and 67%
show the word on even frames and the empty board on odd ones like the
plain frames, with no averaging to a mid colour and no ghost of the
letters on the empty frames (the clip holds the inside of the sign to
this frame's colour; a GPU unit test checks the same synthetically). That
sequence (frames 19540-19639) fails other `temporal-check` checks: 97.15%
of the GPU's vectors within 0.25 px of the CPU's (the limit is 99%), TAA
2.73 against off's 2.28 in motion, 67% 3.14 against naive's 2.95, and the
still frame 4.97 against MSAA 4x's 3.81. It is an attract scene with a
washed-out fade and a cut; not investigated.

**Fixed.**

- Dots along high-contrast edges at 100% (a 1-pixel orange and red row on
  the top edge of the yellow label on `sgt24h`'s car) and random blurred
  pixels inside textures: half-float rounding in the history against a
  clip box with no width in one channel (above). Fixed by `CLIP_SLACK`;
  a GPU unit test sets every opaque history texel's alpha one step below
  1 and fails without it. It was also half of TAA's loss on `topskatr`
  4520-4619 (1.32 before, worse than off).
- Softness in slow motion and on magnified texels (the skater's cut-out
  shadow on `topskatr` rounded off): mostly the clip collapse above, the
  rest the Catmull-Rom resampling of the history, compounded frame after
  frame (now Keys' cubic with C = 0.75). Together they took TAA on
  `topskatr` 4520-4619 from 1.32 to 1.17 (off 1.21).
- Thin geometry speckling at 67% and 50% (`topskatr`'s overhead wires):
  the thin-polygon handling above (2026-11-11). On `topskatr`'s race
  3500-3619, in a crop of the wires (600,330 240x100, the sampled frames
  and 3556, against `gref`): TAA 6.57 to 6.19 (MSAA 4x 6.54), 67% 9.17
  to 8.25 (naive 9.82), 50% 10.59 to 10.01 (naive 12.46); a crop of a
  pole and its wires and railings (330,250 160x110): 7.28 to 7.04, 10.63
  to 10.21, 13.62 to 13.24. The whole sequence: TAA 2.21 to 2.18, 67%
  3.10 to 3.04, 50% 3.93 to 3.87, and the still frame steadier when
  upscaling (change 0.130 to 0.115 at 67%, 0.155 to 0.105 at 50%); the
  one cost, TAA's still-pixel change over the moving frames, 1.33 to
  1.37 (off 1.89). The same gains with the depth buffer on. At 100% the
  wires, slightly grey and doubled before, now match the reference. The
  two parts measured apart: widening alone 67% 8.97, 50% 10.65 on the
  wires crop; the dropped lock with the thin polygon's motion and no
  widening 8.60 and 10.94, with a bright fringe beside the wires (the
  unclipped history's cubic overshoot building up) that needed an extra
  clamp. Two GPU unit tests: a line half an output pixel thick at
  50% and 67% stays whole on every frame (without the handling some
  column of it drops to nothing), and nothing is left behind when it
  goes, jumps away or is covered.
- `lastbrnx`'s motion data (2026-11-11): 3% of position words in blocks
  the plan called valid didn't match frame N-1's (96.9%): models the list
  rewrites in polygon RAM every frame, replayed with frame N's vertices.
  The replay now reads polygon RAM as frame N-1 left it: 100.00%, no
  coverage lost, the other sequences' motion data byte for byte the same.

**Open problems.**

- `vstriker`'s match: a still frame's change 0.338 against the 0.25
  limit (crowd textures under jitter).
- The motion-change rule fires on legitimate camera shake: `sgt24h`'s car
  bounces about 2 px up and down on alternate frames, which drops the
  history's weight on the car (this frame's share 0.43-0.6) and costs
  anti-aliasing there. Telling shake from disocclusion needs more than the
  last aux target.
- Animated objects without motion data look soft (above).
- Thin features built from several polygons that share their long edges
  (a pole or rail modelled as a box, a fence of strips) are not thin by
  the test above, so they get none of the thin-polygon handling and break
  up at 67% and 50% as before. Probably the case of the grandstand's
  railings on `topskatr`'s race (frame 3596: faint and broken at 50%,
  unchanged by the handling; how they are modelled is not confirmed).
  Telling such features from mesh slivers needs more than edge sharing
  (the width of the connected group).
- A thin polygon without motion data isn't marked in the aux target (the
  sign lives in the expected-1/z channel, 0 there); it is still widened.
- Smooth motion's in-betweens still draw those `lastbrnx` models with
  frame N's vertices under the blended transforms, so between frames
  they lead the limbs around them by (1 - t) of their own motion (see
  `docs/hw/geometrizer.md`, "Seeing it"). Blending the list's polygon RAM
  words as well would need pairing them between frames.
- Objects re-paired to another model's transform get no motion data:
  0.1-1.7% of 3D pixels (`sgt24h` 0.1%, `doa` 1.1%, `topskatr` race 1.3%,
  `vstriker` 1.7%). They fall back towards this frame's samples, blocky
  when upscaling.
- GPU cost not measured again after the fixes above (the table is from
  before them).
- In-betweens were driven only by forcing them on a 60 Hz display (every
  aux read-back finite, the steps as expected); not yet seen on a display
  over 60 Hz.
- Measured on one NVIDIA GPU; the Intel GPU in the same machine, and an
  adapter without 16-bit float targets, are untested.

### FSR 2 and DLSS

reModel does not port FSR 2 by hand; the resolve above is its own, small
one. The real FSR 2 library, or DLSS, could later replace that resolve in
the native player only: wgpu's `as_hal` gives unsafe access to the raw
Vulkan device, images and command buffers, so a library can record into
the same frame. The inputs they take now exist: the jitter, per-pixel
motion vectors, depth (the aux target's 1/z) and the render scale; the
sign of the aux 1/z could serve as a reactive mask. Their output has no
coverage alpha, which compose needs for 3D edges over tiles, so alpha
would be resolved separately. An idea, not started.

| | The built-in resolve | FSR 2 (AMD) | DLSS (NVIDIA) |
|---|---|---|---|
| Hardware | any with 16-bit float targets | any | RTX GPUs only |
| Licence | ours | open source (MIT); a C++ SDK to build | proprietary SDK; an optional extra, kept apart |
| wgpu | WGSL, native | native only, through `as_hal` (unsafe) | native only, through `as_hal` (unsafe) |
| Browser | no (decided) | no | no |
| Quality | simple: thin polygons drawn widened at their coverage, no locks | thin-feature locks, reactive masks | best; Ray Reconstruction also denoises ray tracing |

Either costs a C++ SDK as a build dependency and an unsafe interop layer.

*FSR: checked 2026-11-11, parked by decision.* Feasible on Vulkan only,
through AMD's official library (the FSR 3.1 upscaler of the FidelityFX
SDK, MIT; no hand port): wgpu 30's `as_hal` gives the raw device, images
and command buffers, but raw and wgpu commands can't share an encoder, so
the frame would split into three command buffers, with the image layouts
FSR leaves restored by hand. No backend exists for Metal, GL or WebGPU,
so it would be native only, behind an optional cargo feature with the
built-in resolve kept as the fallback; DX12 would need a second interop
layer. Whether the SDK builds on Linux at all is undocumented and the
largest risk. Estimated at 3-4.5 weeks (a 1-3 day build spike first),
plus 0.5-2 days a wgpu major. Parked because it would mainly fix thin
features, which the resolve's own thin-polygon handling addresses, and
it fits the output poorly: no coverage alpha, colours from the games'
lookup tables rather than linear light, and several 3D windows with no
single camera.

(The development machine's RTX 2080 Super supports DLSS upscaling and Ray
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
edges) gives vertex normals: done, per frame, for smooth shading (below).
With them, curved-surface tessellation (PN triangles and similar) could
round off silhouettes of cars and characters with no artist work (an
idea; smooth shading changes brightness only, never outlines). Suits some
models better than others; per-pack opt-out.

### Level of detail and draw distance

Games choose coarser models and cull objects by distance. The display
list shows those choices, so a renderer setting could draw the
finest model always, or further away, to remove pop-in. Because it
changes what a player can see, it is an optional per-game patch, off by
default.

### Smooth shading

Done, opt-in, off by default (2026-11-11), in the native player, the
browser and the CLI's snapshots and tools. Render-only: nothing the game
computes changes (state hashes equal with it on), and with it off the
renderers take the old code (the regression baseline is identical).

The hardware lights each polygon with one luma, so curved surfaces show
their facets. Smooth shading gives each polygon corner its own value and
blends it across the polygon, through the same luma tables and colour
translation tables, so colours stay the game's.

**Settings.**

| | Player | Browser | CLI |
|---|---|---|---|
| Level | `shading = flat \| vertex \| pixel` (menu, launcher, `--shading`) | `shading` (panel: Flat, Smooth, Per pixel; `?shading=`) | `--shading` (snapshots, `gpu-check`, `depth-check`, `pack-check`, `temporal-probe`, `lighting-check`) |
| Crease | `crease = auto \| 0-180` (settings file, `--crease`) | `crease` (panel: the game's, 30, 40, 60; `?crease=`) | `--crease` |
| Finer levels | `shading_fine = true \| false` (menu checkbox "Blend brightness levels", launcher "Brightness levels", `--shading-fine on\|off`) | `shadefine` (panel, `?shadefine=1`) | `--shading-fine on\|off` |
| Brightness balance | `shading_balance = true \| false` (menu checkbox "Balance brightness", launcher "Brightness balance", `--shading-balance on\|off`) | `shadebalance` (panel "Balance brightness", `?shadebalance=0`) | `--shading-balance on\|off` |

All apply live; `gpu::Quality::{shading, crease, shading_fine,
shading_balance}` underneath. The balance is on by default (the picture
is unchanged from before it was an option).

**The three levels.**

- `flat`: the game's picture (the default).
- `vertex`: each corner is lit with the game's own formula (on 2A,
  `srallyc`, unconfirmed: "Limitations" below) for the
  normalised, angle-weighted mean of the normals of the faces around it
  that belong to the same smooth surface, and the value is blended across
  the polygon (perspective-correct, along its edges and across each span,
  as the software fill interpolates), then truncated as the hardware
  truncates.
- `pixel`: the game's formula is worked out at every pixel for the
  normal blended across the polygon, so highlights (the games with
  specular light: `topskatr`, `srallyc`, `stcc`, `bel`, `lastbrnx`,
  `fvipers`, `schamp`) sit where the surface faces them instead of being
  smeared between corners.

What the corners are lit with depends on what is known (`model2::light`;
the record and its self-check in `docs/hw/geometrizer.md`, "Lighting
side channel"):

| Level | Source | Used for |
|---|---|---|
| L2 | the native geometrizer's side channel: per polygon the normal and lighting entry it was lit with, per block the light and focal lengths | the exact layer: `vertex` and `pixel` as above. Every block is checked each frame: its polygons' flat lumas recomputed from the side channel must be the stream's (within one level), else the block falls to L1 |
| L1 | block facts only (the chip path's scan of the display list) | `vertex` from means of the stream's lumas around each corner, with the true focal lengths; `pixel` drawn as `vertex` |
| L0 | the polygon stream alone (old captures; a list the scan doesn't know) | as L1 with a guessed focal length |

So `--geo chip` (and the browser's `geo=chip`) gets `vertex`, never
`pixel`: the menu, launcher, panel and status line say "drawn as
vertex"/"pixel (as vertex)". A failed self-check falls back the same way,
block by block, and is counted (`lighting-check` B). A game with no
native geometrizer (none so far) would get L1 or L0 with nothing written
for it.

**Crease.** Faces meeting at more than the crease angle keep a hard
edge; `auto` (the setting's default) is the game's own
(`romset::Shading::crease`), 0 smooths nothing (the original picture, byte
for byte). A game's own angle is 40 degrees unless its manifest says
otherwise (`schamp`: 25, below): at 40 boxes, 8-sided wheels and limbs
stay faceted, bodies and 10+-sided cylinders smooth. Separately modelled pieces (a fighter's limbs, a car's
panels) are never welded together, so joints keep their seams.

Per-game table, from a survey of all 18 working games (pixel at creases
20, 30, 40 and 50, and vertex at 40, on the two frames with the most lit
polygons of each game's capture, looked at in crops):

| Game | Crease | Why |
|---|---|---|
| `schamp` | 25 | Sonic's quills and Knuckles' dreadlocks lose their ridges at 30 and above (`docs/games/schamp.md`) |
| the other 17 | 40 (default) | no frame looked worse at 40 than at 30 or 50: fighters (`lastbrnx`, `doa`, `fvipers`), cars (`sgt24h`, `stcc`, `overrev`, `srallyc`, `indy500`), `dynabb97`'s octagonal bats (smooth at 40, faceted at 30), `vstriker`'s small players (a little rounder at 40, not worse; stage 4's "rubbery" was not seen again at 1440x1080), `topskatr`'s skaters, `segawski`, `skisuprg`, `hotd`, `bel`, `waverunr` |

A game without an entry, including any title not yet working, gets
`Shading::DEFAULT` (40 degrees, nothing excluded); the generic code reads
only the table's fields, never a game's identity, and the renderers'
default rules are `DEFAULT` until a front end sets the game's. To give a
game its own, write `shading: crate::Shading { crease: N,
..crate::Shading::DEFAULT }` in its manifest (`crates/romset/src/games/`)
with a one-line reason, put the pictures behind it in its
`docs/games/<set>.md`, and check it with `make lighting-check
GAME=<set> GPU_OPTS="--crease N"`. The table also has `skip_windows`,
`skip_models`, `model_crease` and `skip_checker` (no game uses them yet)
and reserves `weld_eps` and `weld_models` (not implemented).

**Finer levels** (`shading_fine`). Brightness reaches the screen through
64 columns of the game's colour tables, so a gradient across a large
smooth surface steps in visible bands. With finer levels on, a lit pixel
takes the blend of the two columns around its brightness instead of one
(`model2::raster::fine_level`; the GPU's `fine_level` is the same). Two
choices were open in the design; both were taken to keep colours the
game's:

- The two columns are always adjacent ones the polygon itself shows with
  the game's levels at the lumas that keep its hue (`Poly::lit_range`), so
  a blended pixel lies between two of the game's own colours of that hue
  and can't leave it (the alternative, blending past the end of that
  range, could mix in a colour of another hue on tables that change
  colour by brightness).
- The blend is centred: column k alone at level k + 0.5, so across a
  gradient the mean column is the game's (the design's "fraction of the
  level", uncentred, would brighten every lit pixel by half a column).
- Pure black is never blended (`raster::fine_columns`, the GPU's
  `fine_columns`): where the game's own column for the pixel (or texel)
  is pure black (all three channels 0) the pixel is that black, and a
  pair of columns one of which is pure black gives the other column
  alone. So black ink stays black, and the pixels next to it keep the
  game's colour instead of a dark fringe blending towards black; the
  only step there is the game's own. Blending elsewhere is unchanged.
  Keeping only the black pixels black (the narrower rule) leaves the
  next column's darker half blending down towards black, a dark ring
  round the ink.

Only lit pixels change; unlit polygons and everything at crease 0 stay
the game's. Pack images follow it (their lighting ratio uses the blend).
No dither. Blending is done on the colour values as displayed, not in
linear light.

By eye (review crops of all 18 games): the bands on untextured car
bodies go (`overrev`'s white car roof, where the 64 levels step visibly
with `pixel`); on textured surfaces the difference is hard to see.
Before pure black was kept, texels the game draws in column 0 as black
ink (the outlines on `fvipers`' armour, frame 1500, whose next column is
(70, 65, 60)) blended up to half a column towards that next one and came
out dark brown; they are black again. Nothing else looked worse than the
game's levels.

**What it does not do.** Outlines never change (no tessellation).
Brightness only: no new lights, shadows or materials. Decals floating
inside a larger smoothed polygon stay flat (counted, not fixed). HUD
windows, shadows, direct polygon data, zero-normal (unlit) and
ambient-only polygons are left flat.

**Brightness and colour.** A lit pixel's luma is clamped to the lumas
over which its colour tables keep the polygon's hue (some games' tables
pick other colours at other levels: `topskatr`'s ramp boards,
`dynabb97`'s players, `lastbrnx`): smooth shading changes level, never
hue. Each smooth surface is moved back by one offset to the brightness
the game gave it (a departure from design decision 6, which accepted the
normalised mean normal's brightening: on bumpy surfaces lit at a grazing
angle, `skisuprg`'s snow, it made lit pixels 23.6 levels brighter on
average). Per-pixel results over all lit pixels of each game's capture:
mean luma shift between -0.8 and +1.7 levels (stage 4). The offset is
the **brightness balance** (`shading_balance`, `light::Options::balance`,
on by default): with it off, each corner (and per pixel, the lamp's
offset) takes the game's formula at the smoothed normal alone, so a
surface whose facets scatter round the light comes out brighter than the
game drew it (others a little darker); the shape of the shading is the
same, only its level moves (one offset per surface, the unit test
`exact_layer_keeps_a_surface_s_brightness`). It
is computed once on the CPU, so the software and GPU renderers agree
either way. `make lighting-check GPU_OPTS="--shading-balance off"`
checks that mode (pictures and reports `-nobal`; the mean-brightness
figures C are reported, not warned about, as they measure exactly what
the balance corrects; everything else must pass).
*Measured* with it off (18 games' captures, 21 frames each, mean luma
shift over all lit pixels against their own): `skisuprg` +23.6 levels
with `vertex`, +13.9 with `pixel`; `schamp` -4.9 and -3.1; every other
game within -1.0 to +2.1 (with the balance on, every game -1.2 to +1.7).
On screen the difference is small even in `skisuprg`: only frames 4500
and 6000 of its capture change visibly (the snow round the skier, a few
levels lighter or darker in patches; review crops by eye); most of that
luma is clamped away by the snow's colour range, and much of the snow is
drawn as dithered layers.

**How it is verified.** `make lighting-capture` (every 300th frame of a
coin-start run with the side channel) and `make lighting-check`
(software and GPU, 1440x1080; `GPU_OPTS="--shading pixel"`,
`"--shading-fine on"`, `"--crease N"`): A crease 0 is the flat frame byte
for byte on both renderers; B the side channel lines up with the stream
and its recomputed lumas match (mismatch at most 0.1%); C brightness of
corners and pixels against the polygon's own luma (reported); D pixels of
polygons not lit are the flat frame's; E software against GPU no worse
than flat; F the stream-only layer against the side channel; G
determinism; J no pixel changes hue; K (finer levels) against the game's
levels: pixels changed, mean shift and the steps between neighbouring
pixels of untextured lit polygons; H, I cost and counts. Unit tests
without ROMs in `model2::light`, `model2::raster` (including
`fine_levels_blend_adjacent_columns`, `fine_levels_draw_between_columns`)
and the GPU's `shading_off_path_and_lit_pipelines`. Results of the last
run (2026-11-11, all 18 games, 21 frames each, RTX A2000 Laptop):

- `vertex`, `pixel`, `vertex` with finer levels and `pixel` with finer
  levels: 72 of 72 runs pass (A, B, D, E, G, J; J 0 pixels everywhere).
- E (software against GPU, pixels over 16 levels) with `pixel` and finer
  levels: within 0.01 points of the flat frames' (0.14-2.9%, the same
  edge pixels) on every game; `schamp` 1.11% against 1.16% flat.
- C pixels (`pixel`): over all lit pixels the luma moves -0.95
  (`schamp`) to +1.68 (`skisuprg`) levels on average, most within ±0.6.
- K (finer levels against the game's levels, `pixel`): 2-65% of lit
  pixels change (`srallyc` 2%, most 15-65%); the mean colour of lit
  pixels moves by at most 0.37 of 255 a channel (`dynabb97` -0.35, the
  rest within ±0.21). On untextured lit polygons, steps between
  neighbouring pixels become about three times as many and a quarter to a
  third the size (`sgt24h` 16,509 steps of 3.8 levels against 43,756 of
  1.4; `overrev` 914 of 3.9 against 4,911 of 1.0; `waverunr` 15,454 of
  3.3 against 44,271 of 1.2): the bands are gone. Some lit pixels end
  more than 16 levels from the game's level where a colour table steps
  far between two columns (`fvipers`: black to (70, 65, 60)); that is
  reported, not failed.
- State hashes after 600M coin-start instructions with `--light
  --shading pixel --shading-fine on --crease 180` (fresh EEPROM):
  `topskatr` 7f0275251269648e, `srallyc` 56b5c12673c631b3, `lastbrnx`
  700e5666e46dd508, `doa` f136114af6b8a55c, the same as with it off.
- The regression baseline (gpu-check, depth-check, pack-check, state, four
  games) is identical with the option off.

**Costs.** CPU: the shading pass (with the side channel's check) about
1-2 ms a new frame, up to 6 ms on busy frames on a loaded machine;
building the lit polygons 0.1-2 ms more; a repeated frame costs nothing.
GPU: within the noise at 1080p (lastbrnx 39.0 / 39.1 / 39.9 ms whole
frame with readback, flat / vertex / pixel) and at 4K; per lit fragment a
loop over the polygon's 3-9 outline points. Finer levels add two colour
lookups per lit pixel. Browser: the shading runs on the page, 1.1-1.5 ms
median a frame (the final build, finer levels on). Side channel: 16 bytes a polygon.

**Limitations and open points.**

- The 2A (`srallyc`) lighting formula is *unconfirmed*: it is the 2C
  program's maths, which reModel's 2A model uses; no 2A stream from
  hardware exists to compare with.
- Per-pixel lighting needs the native geometrizer; the chip path is
  vertex-only (L1). The shadow run of a native program beside the chip,
  for exact lighting of a chip-only program, is designed and not built
  (no title needs it yet).
- In-betweens with smooth motion on a display over 60 Hz were not seen
  live (covered by the code path and tests on a 60 Hz display).
- Finer levels next to a pure black column fall back to the game's step
  (above); a colour table whose dark end is not quite black (say (1, 1,
  1)) is blended as any other.
- Tested on one NVIDIA GPU (Vulkan) and Chrome on Linux.

### Lighting

The hardware lights each polygon with one luma value, through its luma
table: flat shading. Steps:

1. **Per-vertex and per-pixel lighting** from smoothed vertex normals
   and the game's own lights and materials: done ("Smooth shading",
   above).
2. **Material maps** carried in packs (normal, specular, roughness).
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

- How packs carry models and materials alongside textures: one pack
  format, versioned (decided 2026-11-11, `docs/texture-packs.md`, "Pack
  format"): an optional manifest, material maps as suffix files keyed by
  image or surface, model names reserved. The mesh format for models is
  still open.
- Pack-declared highlights (deferred 2026-11-11). Material maps will scale
  the specular highlight a game's lighting entry already has; a polygon
  whose entry casts none gets none, however shiny its pack paints it.
  Letting a pack add one (`spec_base=`, `spec_power=` on a `material`
  line, reserved in the manifest so it needs no new format) invents light
  the game doesn't cast. First measure how many polygons have a highlight
  at all, per game; then decide.
- Whether light data can be recovered on the chip geometry path (not
  investigated, unconfirmed either way). Material maps and the exact
  lighting layer need the native geometrizer's per-polygon light data;
  the chip path (`--geo chip`, recompiled browser builds) has none, so
  materials will do nothing there. Captures made without light data
  don't need recovering: record them again with it.
- Whether the depth buffer can be on by default (undecided). The
  evidence so far (2026-11-11, 21 captured frames per game; "Depth
  buffer" above): it changes 10-19 of 21 frames in every one of the 18
  games, a mean of 0.001-0.7% of the pixels a frame (`skisuprg` up to
  4.8%), mostly crossing geometry the painter's algorithm can't draw.
  Against it: two regressions (`stcc`'s pennants, `dynabb97`'s collar)
  and one tradeoff (`overrev`'s banner) that no generic rule found so
  far handles. Deep floors, dither, flat shadows and skies already keep
  the game's order. Keeping the game's order inside an object block
  would fix none of the three (by the probes, they are in different
  blocks or already in the game's order) and lose fixes that are inside
  one block (`stcc`'s struts, `indy500`'s fence rail, `overrev`'s tree
  cards). On by default would also change every default-settings
  picture check, though not state hashes. Open: long plays (`stcc`'s
  capture never leaves the grid), timings in the browser and on weak
  GPUs, and whether players keep it on.
- How much of this the browser can have: WebGPU covers per-pixel
  lighting (done); ray tracing waits for the web platform. Temporal
  anti-aliasing and upscaling stay native-only (decided), and so would
  FSR 2 or DLSS.
