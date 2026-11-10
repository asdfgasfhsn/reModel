# Texture dumps and texture packs

An enhancement in the spirit of Dolphin's custom textures: `remodel run` can
write out every texture and tile pattern the game draws, and draw its
snapshots with edited or upscaled replacements; the native player and the
browser draw a pack live on the GPU, 3D textures and tiles (below, "Live
display"), optionally with mipmaps and anisotropic filtering of the 3D
textures (below). Nothing here models hardware; with no pack and the filter
off, rendering is unchanged (`make bench` hash, native snapshots and GPU
frames are identical; checked 2026-11-10).

```sh
make dump-textures                       # race run -> out/topskatr/texdump/
make upscale-textures                    # all of it, 4x -> out/topskatr/texpack-4x (below)
# or edit / upscale the PNGs you care about into a directory, same names, then:
make run STEPS=600000000 SNAP=60 INPUT=coin1@1100-1104,start@1250-1254 \
    PACK=out/topskatr/texpack-4x VIEW="--height 1080 --aspect 16:9 --hud edges"
```

`remodel run` options: `--dump-textures DIR` and `--texture-pack DIR` (and
`--tex-filter`, below). Both act at snapshots (every `SNAP` frames and the final frame), so a texture only
drawn between snapshots is not dumped; `make dump-textures` snapshots every
60 frames. Dumps and packs are ROM-derived: keep them out of git (`out/` is
ignored).

## What gets dumped

**3D textures.** One PNG per distinct texture drawn (at least one pixel),
named `tex_<16 hex>_<w>x<h>.png`, `w` x `h` being the texture's size in
texels. The image is the texture rendered through the game's own pipeline
(docs/hw/geometrizer.md 5.4) at **full polygon luma** (255), one pixel per
texel, unfiltered: texel -> luma table -> colour translation with the
polygon's colour. For a translucent texture, texel 15 has alpha 0.

The key (the 16 hex digits) is a 64-bit FNV-1a hash of:

- the texels of the texture's rectangle in render RAM (bank, x, y, w, h from
  the header, so moving a texture in RAM keeps its key), and
- the fields that change how it looks: width, height, luma table number
  (h1 bits 0-7), translucent flag (h0 bit 13), and the polygon colour value
  (palette entry 0x1000 + h3 >> 6, xBGR555).

The same texels drawn in another colour are therefore separate dumps.
Mirroring, wrap and the checker (screen-door) flag do not change the key;
they apply to the replacement as they do to the original.

**Looks.** The key does not include the luma table's and colour translation
tables' contents, which games rewrite: srallyc fades into the race from
white through the translation tables, and sgt24h tints its sky during the
race (`docs/games/srallyc.md`). The same key can therefore be drawn in
several looks (the 16 colours its texel values map to at full luma). The
dump shows the look drawn most (polygons, over the snapshots so far),
rewriting the image when another look overtakes it; the index marks such
textures `looks=N`. Until 2026-11-11 the first look seen was kept: srallyc's
race textures were dumped in the fade, near white, and a pack made from
them drew the race washed out (and sgt24h's sky without its tint). **Dumps
of srallyc and sgt24h made before then are wrong: dump again and remake
the packs.** topskatr's race textures have one look (its dumps and packs are
unchanged); other games are unchecked (`make pack-identity`, below, tells).
A texture drawn only in a fade during the dump run keeps that look, so dump
with a run that also sees it in its usual look (srallyc: `make
dump-textures BENCH_STEPS=900000000` covers the race captures).

**Tiles** (the 2D layer: HUD, menus, text). One 8x8 PNG per distinct tile
pattern on screen, `tile_<16 hex>.png`, keyed by the 64 pens plus the
palette colours of the pens it uses. Pen 0 has alpha 0 but keeps its palette
colour. Blank tiles (all pen 0) are skipped. A palette fade produces new keys
for the faded colours, which then draw as the original (no replacement).

`DIR/index.txt` lists every dump: name, first frame seen, uses (textures:
polygons drawn, over all snapshots; tiles: frames on screen), and what
produced it (texture header words, colour, bank and sheet position, luma
table, translucent, all from the first use; tile number and palette bank).
Textures end with `wrap=`: every way the texture was drawn, one pair per
way, u then v, `r` repeat or `m` mirror (`wrap=rr,mr`: repeated, and also
mirrored in u). The mirror bits are not part of the key, so one dump can be
drawn several ways; `make upscale-textures` pads by this field. Indexes
written before it have no `wrap=`. Sorting the index by `uses` is a quick
way to find the textures worth replacing first.

## Making a pack

A pack is a directory of PNGs named like the dumps. Any subset will do:
missing names draw as the original. Images may be any size; typically 2x or
4x upscales. Any non-interlaced PNG works (grey, RGB, palette, with alpha or
tRNS, 8 or 16 bits); interlaced files are rejected with a message and that
texture falls back to the original.

```sh
mkdir -p out/topskatr/texpack
cd out/topskatr/texdump
magick mogrify -path ../texpack -filter Lanczos -resize 400% tex_*.png tile_*.png
```

Images are loaded the first time their key is drawn and cached for the run
(live, see below, they are decoded in the background and the original shows
until an image is ready).

### 3D textures

The replacement covers the same texture area: it is sampled bilinearly at
the normalised coordinates (u / w, v / h), with the original's repeat or
mirrored-repeat per axis, so tiling works as before. Pixels with alpha < 0.5
are transparent (like texel 15 of a translucent texture), and colour is
weighted by alpha so transparent pixels' RGB does not bleed into edges.

**Lighting.** The replacement stands for the texture at full luma, as
dumped. For each pixel, the rasteriser takes the original texel under it
(filtered as usual) and computes per channel what the original pipeline
gives at this polygon's luma over what it gives at full luma:

    ratio = xlat(c, min(63, L(t) * luma / 256)) / xlat(c, min(63, L(t) * 255 / 256))
    out   = replacement * ratio

with `L` the luma table, `xlat` the colour translation table and `c` the
polygon colour component. That keeps the game's lighting curve, including
the cap at 63 (bright texels saturate before dark ones) and per-channel
tinting from the translation tables. Where the original texel is transparent
or black at full luma, the ratio for the brightest luma is used instead. At
full polygon luma the replacement is drawn as is. Because the ratio follows
the original texels, a replacement that changes the picture a lot (rather
than refining it) can show faint traces of the original's shading where the
original saturates.

Both sides of the ratio use the tables as they are now, so the replacement
follows the polygon's luma but not a change of the tables themselves: while
srallyc fades in from white, replaced textures show their usual look
(capture frame 2700: 88% of the frame's pixels off by more than 16 levels
from the original). Following that would need the look the image was made
from at draw time (not done).

### Tiles

Tile replacements should be N x 8 square (32x32 for 4x). In a scaled view
(`--height`), the tile layer is then sampled at output resolution: each
output pixel finds its game pixel and the position inside it, and samples
the replacement there (bilinear, clamped to the tile's edges, so tiles do not
blend into their neighbours; art that spans several tiles can show seams if
the upscaler treated each tile alone). Transparency follows the replacement's
alpha (< 0.5 is transparent), with the hardware rules kept: on the opaque
back plane (plane B, low priority) transparent replacement pixels show the
original pen, as pen 0 does there; elsewhere they show what is behind. HUD
placement in wide views (`--hud edges`) works as for the original tiles.
The columns a wide view adds at the sides (a scrolled tile sky carried on
past the frame) keep the original patterns. Snapshots, the player and the
browser draw tiles by these same rules (live: below).

## Live display

The native player and the browser draw a pack on the GPU
(`gpu::Renderer::set_pack`, `docs/gpu.md`), with the same replacement and
lighting rules as the snapshots. A pack is a directory of the same
`tex_<key>_<w>x<h>.png` and `tile_<key>.png` files.

**Tiles live.** Only a pack with tile images touches the 2D layers, and only
in frames where a pattern on screen has one: otherwise the layers are
drawn exactly as without a pack. When one does, the CPU works out for each
game pixel which tile each plane shows there and where in it, and the
GPU's compositor draws the replaced ones at output resolution
(`compose.wgsl` `fs_tiles`, `gpu/src/tiles.rs`), so a 4x image is as sharp
at 1080p or 4K as it is, not baked into the 496x384 layer. The rules are
the snapshots' (above): bilinear, clamped to each tile's own image, so
tiles never blend into their neighbours and need no padding; per-line
scroll, the split and window modes, priority and pen 0 are the game's own,
and scroll is in whole game pixels, as on the hardware. Keys are the dump's
(pattern and colours), so a tile drawn mid-fade or in a cycled colour has
another key and draws as the original for those frames. Images are kept in
one atlas texture of equal square cells, the size of the first tile image
(other sizes are scaled to it: keep a pack's tiles one size), 64 x 64 cells
for 4x images; when it is full the least recently drawn give way, and a
frame needing more than it holds draws the rest as the original. The
player's Esc menu and the browser's status line show how many patterns
were replaced in the last frame.

**Native player** (`docs/play.md`). The setting `pack` (default off; flag
`--pack on|off`) looks in `<data dir>/<set>/texpack/` (`out/<set>/texpack`
in a checkout), and for a clone without one in its parent's, since keys are
the textures' content. `--texture-pack DIR` uses DIR for every game that run
(and turns `pack` on). The directory is listed when the pack is switched on;
PNGs are decoded on two background threads the first time their texture is
drawn, and the original shows until the image is ready. The launcher's
previews never use packs. The Esc menu shows how many images the pack has
and how many polygons it replaced last frame; a missing directory gives a
notice.

**Browser** (`docs/web.md`). `?pack=1` or the panel's Display setting (applied
live) fetches the pack from `/texpack/<set>/` on the server, beside
`/rom/`, and for a clone without one from its parent's folder, as the
native player does (`romset::pack_sets`). A browser cannot list a folder, so the directory needs an
`index.txt`: one file name per line, `#` comments and blank lines ignored.
`make upscale-textures` writes one; for a hand-made pack,
`(cd DIR && ls *.png > index.txt)`. Only names in the index are requested,
and only when their texture is first drawn; a worker fetches and decodes
them (a 1024x1024 PNG takes 10-50 ms) and the page hands over at most one
image a frame. Without an `index.txt` the original textures stay, with a
console warning.

**GPU memory.** Images are uploaded when first drawn and kept; over a budget
(512 MB natively, 192 MB in the browser) the least recently drawn are
dropped and loaded again when next needed. With mipmaps on, an image costs
about a third more. Tile images are outside that budget: their atlas is
allocated whole when the first one is drawn, 16 MB for 4x tiles (2048x2048),
64 MB at most.

**Cost.** A frame that replaces textures draws a batch for each run of
polygons (in draw order) sharing an image instead of one for the whole frame: about 600 draws a frame
on topskatr with a full pack (198-1047), where there were 1-4. Frames upload
and decode new images as they first appear, so a large image's first
appearance can hitch: in the browser, a frame uploading a 1024x1024 image
with its mipmaps measured up to 13 ms of draw against about 3 ms normally.
The player's decoding stutter has not been measured. Live tiles add CPU
work in frames that replace some (the per-pixel tile map: on topskatr's
captures the 2D layers took 2.7-3.0 ms instead of 1.7-2.1 ms in race
frames, natively, `make tile-check` on a machine shared with other jobs)
and an upload of about 2.5 MB a frame at 1080p (the map and the HUD's
source columns); compose then samples up to four tile images per output
pixel instead of reading two textures. Whole GPU frames with and without
tiles were within the run-to-run noise there; the browser drew
`overrevb`'s attract mode with 4x tiles at 60 fps, 6-7 ms of draw a frame
(1413x1060).

## Mipmaps and anisotropic filtering

The setting `tex_filter` (player; flag `--tex-filter`, also `remodel
--tex-filter` for snapshots and `gpu-check`) or `texfilter` (browser,
`?texfilter=`) takes `off`, `mip`, `2`, `4`, `8` or `16`; default `off`. It
affects pack images only: the original textures are never filtered this way,
and with no pack it does nothing.

- **off**: the image is sampled bilinearly as it is, as in snapshots before
  this existed. A high-resolution image on a small or distant polygon
  aliases and shimmers (moiré on distant ground).
- **mip**: the image's mip chain is built (each level a box filter of the
  last, down to 1x1) and the two levels around the pixel's footprint are
  blended (trilinear). Mipmaps average display values, not linear light, and
  are premultiplied by alpha, so transparent texels' colour does not bleed
  into cut-out edges. Distant ground stops shimmering but blurs.
- **2 / 4 / 8 / 16**: mipmaps plus anisotropic filtering up to that many
  times, so ground at a grazing angle keeps its detail. Best effort: the GPU
  may allow less (`gpu::max_anisotropy` reports 16, or 1 where the backend
  has none, and the player then offers only `off` and `mip`).

**Software and GPU.** The software renderer (snapshots) has trilinear
filtering only: `2`..`16` draw as `mip`. Anisotropic filtering is GPU-only.
The GPU filters with its sampler and the polygon's screen-space gradients of
the texture coordinates; the software computes its level from gradients of
the same quantities. With filtering on they match more closely than the
exact path does (measured 2026-11-10, `topskatr`, a fine-checker 4x test
pack): pixels differing by more than 16 levels between the GPU's `mip` and
software trilinear were 1.85% at native size and 2.5% at 1080p, against
3.4% for the exact (filter off) path with the same pack.

**Cost.** Mip chains are built on the CPU when an image is first drawn and
uploaded with it. Filtering is the GPU's own and cheap; the draws per frame
(above) are the larger cost.

**Checking.** `make pack-check PACK=DIR` draws the frames `make capture` saved
on the GPU unfiltered, mipmapped and at 16x, in software trilinear and as a
4x4 supersampled reference, writes them to `PACK_CHECK_OUT` (default
`OUT/pack-check`) and reports each one's mean difference from the reference.
On topskatr (mean difference per channel, 0-255):

| pack | off | mip | 16x |
|---|---|---|---|
| a harsh fine-checker test pack | 9.66 | 4.70 | 4.84 |
| a realesrgan-x4plus pack | 2.09 | 2.06 | 1.62 |

By eye: `off` shows moiré on distant ground with a detailed pack, `mip`
removes it but blurs, `16` keeps the ground's detail at grazing angles, and
none shows halos at cut-out edges. The metric is against a supersampled
render, which is itself an approximation of an ideal filter. Other games and
other upscalers are untested.

`make tile-check PACK=DIR` draws the captured frames with the pack's tiles
live on the GPU and as the software snapshots draw them, over the same 3D
layer, writes both and the frame without the pack (`FRAME-gpu`, `-sw`,
`-orig.png` in `OUT/tile-check`) and reports where they differ. On
topskatr (realesrgan-x4plus tiles), srallyc and doa (built-in 4x), 21
frames each at 1080p 16:9: no pixel differs by more than 16 levels; 0.1-2%
differ at all, by rounding. `gpu-check` and `pack-check` compose with the
original tiles (they compare 3D textures).

**Identity.** `make pack-identity PACK=DIR` (`remodel pack-check --identity`)
checks that a pack stands for the textures as the captured frames draw
them: with `PACK` the dump itself (`OUT/texdump`), or with no `PACK` each
frame's own textures, it draws every captured frame on the GPU unfiltered
and mipmapped and in software exact and trilinear, against the same
renderer without a pack, and writes the pictures to `OUT/pack-identity`. It
fails if any frame has more than 1% of its pixels off by more than 16
levels unfiltered (the pack's colours are filtered where the original
filters texel values and looks them up, so they are not identical);
mipmapped drawing is a different filter and is only reported. Measured
2026-11-11 at native size, mean share over the captures, unfiltered (GPU /
software):

| pack | srallyc | topskatr | sgt24h |
|---|---|---|---|
| each frame's own textures | 0.30% / 0.31% | 0.50% / 0.50% | 0.71% / 0.71% |
| dump, first look kept (before the fix) | 59.4% / 59.4% | 0.49% / 0.49% | 13.1% / 13.2% |
| dump, most used look (`make dump-textures`) | 5.6% / 5.6% | 0.49% / 0.49% | 1.6% / 1.6% |
| dump, `BENCH_STEPS=900000000` | 5.2% / 5.2% | | |

With the fix, srallyc's race frames are 0.13-0.83% off; what is left is the
fade (frame 2700, above) and, with the default dump run (shorter than the
capture's), textures it saw only in the fade (frame 5700's start banner and
boards, 8%); sgt24h's are frames 2400-3000 (about 2%, tint) and 6212 (16%,
past the dump run). Mipmapped, every pack is 3-11% off at native size:
distant textures average.

`make gpu-check PACK=DIR GPU_OPTS="--tex-filter mip"` draws the captured
frames with the pack on both renderers and compares them: on topskatr the
pixels differing by more than 16 levels were 1.58% with the original
textures and 1.54% with a 1:1 pack, so the GPU's pack path matches the
software's about as closely as the originals do.

## Batch upscaling

`make upscale-textures` makes a pack from a whole dump in one go (`remodel
texpack-upscale SRC DEST`, no ROM needed):

```sh
make dump-textures GAME=topskatr
make upscale-textures GAME=topskatr                 # built-in bicubic, 4x
make upscale-textures GAME=topskatr TEXPACK=out/topskatr/texpack-esrgan \
    UPSCALER='realesrgan-ncnn-vulkan -i {in} -o {out} -s {scale} -n realesrgan-x4plus -g 1 -f png'
```

Variables: `TEXDUMP` (default `OUT/texdump`), `TEXPACK` (default
`OUT/texpack-SCALEx`), `SCALE` (4), `UPSCALER`, `TILES`, `COLOUR`, `MIN_USES` (1: dumps
drawn fewer times, often transient keys, are left out), `MAX_SIDE` (4096: a
larger result is scaled down to the largest whole factor that fits; a 4x
2048x1024 texture would be 128 MiB in memory, and packs stay loaded for the
run), `FORCE=1`. It ends by printing the `make run ... PACK=` and `make
pack-check ... PACK=` commands that show the result.

**The upscaler** is an external tool, run once over a directory, never part
of reModel: `UPSCALER` is its command with `{in}` (a directory of PNGs),
`{out}` (where it must write PNGs of the same names) and `{scale}` filled in,
run with `sh -c`. Empty, a built-in bicubic (Catmull-Rom) resampler is used,
which needs nothing installed. Its output must be exactly `SCALE` times its
input: models with a fixed scale (realesrgan-x4plus and -x4plus-anime are
4x only; realesr-animevideov3 has 2x, 3x and 4x) need the matching `SCALE`,
and any other size is reported per image as a failure. With
`realesrgan-ncnn-vulkan`, `-g` picks the GPU from the list it prints at
start: on a laptop with an integrated GPU as well, that is often `-g 1`
(the development laptop: 0 is the Intel Iris Xe, 1 the NVIDIA RTX A2000).
Check the list on your machine; without `-g` it may run on the slow one.
Small inputs: ncnn ESRGAN models give garbage below about 8 pixels a side
and smear edges up to 12, so every image handed to the upscaler is at least
16 pixels a side (padding, below, already makes tiles 24).

**Tiles** (`TILES=`): `model` (default) sends them through the upscaler
like textures; `builtin`, `nearest` (pixel copies, as sharp as the
original) or `skip` (left out, drawn as the original). On topskatr the
5030 tile dumps are most of the run time; realesrgan-x4plus gave crisper
text and outlines than the built-in.

What it does to each dump:

- **Alpha.** Transparent pixels (texel 15 of a translucent texture, tiles'
  pen 0) get the colour of their opaque neighbours, filled outwards, so the
  meaningless colour under them cannot bleed into edge pixels. The
  upscaler only ever sees opaque RGB. Alpha is scaled here: bilinear with
  the texture's addressing, cut at 0.5 (the renderer's own rule), so pack
  images have only alpha 0 or 255 and cut-outs keep the original's shape
  at the finer resolution, without halos. A tile's transparent pixels get
  pen 0's colour back, as in the dump. Fully transparent dumps are left out.
- **Seamless tiling.** Textures are padded by 16 texels on each side with
  the renderer's own addressing, repeated or mirrored per axis as `wrap=`
  says they are drawn, upscaled with that context and cropped, so a
  repeated texture's edges meet without a seam. An axis drawn both ways (or
  with no `wrap=`: older indexes give the first use's header) is padded
  mirrored: exact where it is mirrored, and where it repeats it still
  continues the texture's own edge instead of bringing in the far side's.
  The summary counts those. Tiles are padded by clamping (their neighbours
  differ from one place to the next).
- **Luma.** The full-luma dump is what gets upscaled; lighting is then the
  usual per-channel ratio (above), which assumes the image stands for the
  full-luma texture.
- **Colour** (`COLOUR=match`, the default; `--colour` on `remodel
  texpack-upscale`). The dump's low-frequency colour is put back and the
  upscaler's detail kept: each pixel gets lowpass(dump) - lowpass(result)
  added, both a Gaussian of 2 source texels, weighted by opacity so
  transparent pixels pull nothing, with the texture's addressing so the
  correction tiles like the texture (tiles: nothing past the edge). It is
  smooth (no halos or edges of its own); a flat shift comes back exactly.
  `COLOUR=keep` leaves the upscaler's colours. Images whose mean colour
  moved by more than 6/255 in a channel (`--drift N`) are reported, before
  and after matching, and listed in the pack's `drift.txt`. Matching fixes
  colour, not texture: a denoising model's lost grain stays lost (the
  road is still smooth, only its tint is back), and detail it smeared
  between colours (thin red/black tile art) stays smeared. The built-in
  resampler hardly changes (mean 0.05/255 a pixel).
- **Checks.** Every result must be exactly `SCALE` times the padded input;
  a missing or wrong-sized output fails that image and the run (the work
  directory, `out/scratch/texpack-upscale-GAME`, is then kept to look at,
  with the tool's output in `upscaler.log`). An upscaler that writes no file
  for 5 minutes (`--stall SECONDS`) is stopped; what it did write goes into
  the pack and a rerun retries the rest. realesrgan-ncnn-vulkan 0.2.0 once
  hung here on the last image of a run, spinning a core with the GPU idle.

**Incremental.** The pack's `upscale.txt` records the settings and, per
image, its dump's hash and the file's size and time. A rerun only does new
or changed dumps, keeps images edited or added by hand (anything not
matching the record), and refuses to mix settings: change `TEXPACK` or pass
`FORCE=1` to redo all. Images are written under a temporary name and renamed,
so an interrupted run never leaves a half-written file. The pack's
`index.txt` lists its image files, one name a line (`#` lines are comments).

On topskatr (coin-start race dump: 241 textures, 5030 tiles, an RTX A2000
laptop): built-in 1.3 s, 47 MiB; realesrgan-x4plus 2.5 minutes, 86 MiB
(textures alone, `TILES=skip`: 59 s).
What to expect from realesrgan-x4plus: crisper HUD text and outlines and
cleaner foliage, no seams on repeated ground and no halos on cut-outs, but
it is a denoising model: grainy textures (the snow-like road) come out
smooth and flat and the grass gains invented blades. Without matching
(`COLOUR=keep`) clouds turn whiter and 30 textures and 909 tiles drift in
colour (14/255 at most for a texture); with it none of the textures and 2
tiles do (6.2/255 at most), the clouds keep their blue-grey shading, and
the frames' low-frequency colour error against the original drops by a
quarter to a half (`pack-check` captures). Try `TILES=` and other
models per game; the built-in result is close to the original, only
softer.

Limitations:

- Art spanning several tiles (logos, big fonts) can show seams between
  tiles, since each tile is upscaled alone; on the opaque back plane the
  pen-0 area of an upscaled tile still draws the original pen, per game
  pixel.
- Textures drawn only between snapshots are not in the dump, so they stay
  original (topskatr: 57 of 4293 textured polygons in a coin-start run).
- Models that redraw a texture rather than refine it can show the
  original's shading traces under lighting (above); colour matching
  restores the mean colour over a few texels but not finer structure, and
  `drift.txt` lists what still drifts.

## Limitations

- Tile keys include colours: tiles drawn during a fade or palette cycle
  show the original (one image per colour would be needed). The sides a
  wide view adds (scrolled tile skies) keep the original patterns.
- Dumps happen at snapshots, so short-lived textures can be missed.
- Live, an image's first appearance can hitch (decode, upload, mip chain);
  the original shows until it is ready. A frame that replaces many textures
  makes hundreds of draws.
- Anisotropic filtering is GPU-only (snapshots draw it as `mip`), and
  mipmaps average display values, not linear light.
- Not verified live: smooth motion's in-between frames with a pack, F5/F9
  save slots with a pack, other browsers and phones, and eviction past the
  memory budget or the tile atlas.
- Textures still being uploaded when a snapshot is taken get transient keys
  (the game copies texture data across several frames).
- Keys include colours, so a texture or tile drawn in many colours (fades,
  flashing text) needs one replacement per colour. Textures change colour
  without changing key when the game rewrites the translation tables
  (`looks=` above): the replacement stands for the look drawn most, and
  fades and tints of the tables do not show on it.
