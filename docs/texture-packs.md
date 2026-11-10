# Texture dumps and texture packs

An enhancement in the spirit of Dolphin's custom textures: `remodel run` can
write out every texture the game draws, and draw its snapshots with edited
or upscaled replacements; the native player and the browser draw a pack live
on the GPU (below, "Live display"), optionally with mipmaps and anisotropic
filtering (below). Nothing here models hardware; with no pack and the filter
off, rendering is unchanged (`make bench` hash, native snapshots and GPU
frames are identical; checked 2026-11-10).

```sh
make dump-textures                       # race run -> out/topskatr/texdump/
# edit / upscale the PNGs you care about into a directory, same names, then:
make run STEPS=600000000 SNAP=60 INPUT=coin1@1100-1104,start@1250-1254 \
    PACK=out/topskatr/texpack VIEW="--height 1080 --aspect 16:9 --hud edges"
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

**Tiles** (the 2D layer: HUD, menus, text). One 8x8 PNG per distinct tile
pattern on screen, `tile_<16 hex>.png`, keyed by the 64 pens plus the
palette colours of the pens it uses. Pen 0 has alpha 0 but keeps its palette
colour. Blank tiles (all pen 0) are skipped. A palette fade produces new keys
for the faded colours, which then draw as the original (no replacement).

`DIR/index.txt` lists every dump: name, first frame seen, uses (textures:
polygons drawn, over all snapshots; tiles: frames on screen), and what
produced it (texture header words, colour, bank and sheet position, luma
table, translucent; tile number and palette bank). Sorting it by `uses` is a
quick way to find the textures worth replacing first.

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

## Live display

The native player and the browser draw a pack on the GPU
(`gpu::Renderer::set_pack`, `docs/gpu.md`), with the same replacement and
lighting rules as the snapshots (3D textures only: see Limitations). A pack
is a directory of the same `tex_<key>_<w>x<h>.png` files; tiles in it are
ignored live.

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
`/rom/`. A browser cannot list a folder, so the directory needs an
`index.txt`: one file name per line, `#` comments and blank lines ignored.
`(cd DIR && ls *.png > index.txt)` writes one. Only names in the index are requested,
and only when their texture is first drawn; a worker fetches and decodes
them (a 1024x1024 PNG takes 10-50 ms) and the page hands over at most one
image a frame. Without an `index.txt` the original textures stay, with a
console warning.

**GPU memory.** Images are uploaded when first drawn and kept; over a budget
(512 MB natively, 192 MB in the browser) the least recently drawn are
dropped and loaded again when next needed. With mipmaps on, an image costs
about a third more.

**Cost.** A frame that replaces textures draws a batch for each run of
polygons (in draw order) sharing an image instead of one for the whole frame: about 600 draws a frame
on topskatr with a full pack (198-1047), where there were 1-4. Frames upload
and decode new images as they first appear, so a large image's first
appearance can hitch: in the browser, a frame uploading a 1024x1024 image
with its mipmaps measured up to 13 ms of draw against about 3 ms normally.
The player's decoding stutter has not been measured.

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

`make gpu-check PACK=DIR GPU_OPTS="--tex-filter mip"` draws the captured
frames with the pack on both renderers and compares them: on topskatr the
pixels differing by more than 16 levels were 1.58% with the original
textures and 1.54% with a 1:1 pack, so the GPU's pack path matches the
software's about as closely as the originals do.

## Limitations

- Tiles (the 2D layer) are replaced only in software snapshots, not in the
  live GPU display.
- Dumps happen at snapshots, so short-lived textures can be missed.
- Live, an image's first appearance can hitch (decode, upload, mip chain);
  the original shows until it is ready. A frame that replaces many textures
  makes hundreds of draws.
- Anisotropic filtering is GPU-only (snapshots draw it as `mip`), and
  mipmaps average display values, not linear light.
- Not verified live: smooth motion's in-between frames with a pack, F5/F9
  save slots with a pack, a clone's fallback to its parent's pack in the
  player, other browsers and phones, and eviction past the memory budget.
- Textures still being uploaded when a snapshot is taken get transient keys
  (the game copies texture data across several frames).
- Keys include colours, so a texture or tile drawn in many colours (fades,
  flashing text) needs one replacement per colour.
