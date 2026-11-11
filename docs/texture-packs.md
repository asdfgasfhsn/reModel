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
Textures then have `surf=`, the texture's surface key (16 hex digits; below,
"Pack format": what material maps can be named by to serve every colour of
the texture; dumps made before 2026-11-11 have none), and `wrap=` (and after it `looks=N`, above, where there is
more than one look): every way the texture was drawn, one pair per
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

`make pack-lint PACK=out/topskatr/texpack` checks the result (names, the
index, that every file loads; below, "Pack format", which also defines
the optional manifest `pack.txt` and material maps).

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
tinting from the translation tables. Where the original texel is transparent,
the ratio for the brightest luma is used instead. Where it is black at full
luma in a channel, nothing scales up from black, so that channel is
`replacement * (ratio for the brightest luma) + xlat(c, min(63, L(t) * luma
/ 256))`: it adds what the original gives at this luma. That matters for
translation tables that are not monotonic: topskatr draws one texel pure
red at level 63 but brown at lower levels (a skater's collar), which a
pack drew red until 2026-11-11. At full polygon luma the replacement is
drawn as is. Because the ratio follows
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

## Pack format

A pack is a directory; this section is its definition, the same for every
board and game (names are content keys the renderers compute; nothing in a
pack names a board). `model2::pack` implements it: the manifest, the
compatibility rules and the file-name grammar, used by the CLI, the player
and the browser. **Status (2026-11-11):** the format, the surface key and
`make pack-lint` exist; material maps are named, checked and loaded (below,
"Loading material maps") but not drawn yet, so a pack's pictures are what
the images alone give.

**Format 1** is everything above: `tex_<key>_<w>x<h>.png`,
`tile_<key>.png`, any subset, `index.txt` for the browser, and tool records
(`upscale.txt`, `drift.txt`) that readers ignore. It has no manifest, and
**a pack without a manifest is read exactly as before manifests existed**
(same lookups, same pictures; checked 2026-11-11 by `make pack-identity`,
`make tile-check`, `gpu-check` and `pack-check` with a pack, all
unchanged). **Format 2** adds, all optional:

| File | What |
|---|---|
| `pack.txt` | the manifest (below) |
| `tex_<key>_<w>x<h>.n.png`, `.s.png`, `.g.png` | material maps for that image in that colour: normal, specular scale, gloss |
| `surf_<skey>_<w>x<h>.n.png`, `.s.png`, `.g.png` | the same maps keyed by surface, serving every colour the texels are drawn in |
| `materials.txt` | tool record (reserved for a map-making tool) |

Keys are 16 lowercase hex digits and sizes plain decimals, exactly as the
renderers write them; any other spelling is never opened. Tiles have no
material maps (the 2D layer has no lighting): `tile_<key>.n.png` is not a
pack name. Reserved, not to be used for anything else: map suffixes `.e`
(self-lit) and `.m` (maps packed in one file), file prefixes `mdl_`
(replacement models) and `img_` (pack-own images), the directory
`models/`, the manifest keyword `model` and the features `models` and
`selflit`. Readers only open names they construct, so files of a kind
they don't know are never touched, and an `index.txt` line may name any
file (`models/...` included); the browser fetches only image names.

**Surface key.** The image key (above) covers the polygon colour, luma table
and translucent flag, so one set of texels drawn in five colours is five
images. A material map describes shape, which is the texels', so it can
also be named by the surface key: the same 64-bit FNV-1a stream as the
image key, stopped after the texels and the size. Exactly, with all
numbers little-endian:

    texels  = FNV-1a 64 over the rectangle's texel values (0-15), one byte each, rows top to bottom
    surface = FNV-1a 64 over texels (8 bytes), w (4 bytes), h (4 bytes)
    key     = the same stream continued over luma table, translucent (0 or 1), colour (4 bytes each)

so mirroring, colour, luma table and translucency never change it, and the
image key is unchanged by its arrival. The dump's index gives it per
texture (`surf=`). A reader will look for a texture's map of each kind as
`<image name>.<k>.png` first (this image in this colour), then
`<surface name>.<k>.png` (these texels in any colour); a material needs no
replacement image (a normal map on the original texels is valid).

**Maps** are data, not colour: 8-bit values as stored, no gamma, any
non-interlaced PNG, alpha ignored. `.n`: x and y in red and green,
`(v - 128) / 127`, so 128 is 0 and the flat normal is (128, 128, 255); x
points to the image's right, y to its top with `normal_y up` (the
default) or its bottom with `normal_y down`; blue is not used by the
renderer (z is reconstructed). `.s`: specular scale from red, `R / 128`
(128 is the game's own highlight, 0 none). `.g`: gloss from red, the
game's specular exponent times `2^((R - 128) / 32)`. A map covers the
texture's `w` x `h` texels like an image, at any size; the tools expect the
size of the pack image. Maps modulate the game's own lighting, which only
the native geometrizer's exact lighting data carries: where that is
missing (the chip path, recompiled browser builds, captures without
lighting data), a material will have no effect.

**The manifest, `pack.txt`.** Line-based UTF-8 text: a keyword, a tab (or
spaces), its value; `#` comments and blank lines skipped; `format` first.

```
# reModel pack
format	2
keys	1
name	topskatr 4x, hand-tuned materials
game	topskatr
uses	materials
normal_y	up
normal_scale	1.0
spec_scale	1.0
material	tex_0123456789abcdef_64x64	normal=0.5	spec=1.5
material	surf_fedcba9876543210_64x64	normal=0
```

| Keyword | Meaning | Default |
|---|---|---|
| `format N` | the format's major number; the first line | no manifest: 1 |
| `keys N` | the key scheme of the file names: 1 is the image, tile and surface keys defined here | 1 |
| `name`, `game` | free text: what the pack is, the set it was made from; never used for lookup | none |
| `uses a,b` | optional features the pack contains (`materials`); names a reader doesn't know are ignored | none |
| `needs a,b` | features without which the pack must not be shown at all | none |
| `normal_y up\|down` | green direction of every normal map | `up` |
| `normal_scale X`, `spec_scale X` | pack-wide strength of normal and specular maps, 0 to 4 | 1 |
| `material STEM k=v ...` | per material (`tex_..` or `surf_..` stem, no suffix): `normal=X`, `spec=X` multiply the pack-wide strengths; other keys are ignored (reserved: `spec_base`, `spec_power`, `selflit`) | none |
| `model ...` | reserved for replacement models | |

A malformed manifest (a missing or misplaced `format`, a keyword given
twice, a value out of range) refuses the pack, with the line it is on.

**Compatibility.**

1. No manifest is format 1, readable forever.
2. `format` rises only when the meaning of something a reader already
   understands changes. A reader refuses a pack whose `format` is higher
   than it knows: it draws the original textures and says the pack needs
   a newer reModel. It never guesses. The same for a higher `keys`.
3. Everything else is added without a new format: readers ignore unknown
   keywords (the player prints one line about them, `pack-lint` lists
   them), unknown kinds of file and unknown names in `uses`. A pack that
   must not be shown without a feature puts it in `needs`, and a reader
   that can't show it refuses the whole pack. No reader draws material
   maps yet, so today `needs materials` refuses the pack everywhere (they
   load, but showing the pack without them is what `needs` forbids) and
   `uses materials` is informative.
4. Tools write `pack.txt` only when the pack holds something format 1
   can't say; a plain upscaled pack gets none (`make upscale-textures`
   writes none) and stays readable by older builds, which show its images
   and nothing of the rest.

Where a refused pack shows: `run` and the checks print the reason and draw
without the pack; the player's Esc menu shows it as it does a missing
directory; the browser logs it to the console and keeps the originals.

**Loading material maps.** Only a pack with a manifest of format 2 has
material maps: without `pack.txt` (format 1) its `.n`, `.s` and `.g` files
are never opened, so an older pack behaves as before whatever it holds.
Readers find a texture's map of each kind by image name, then surface name
(`pack::resolve_maps`); the files found make one material
(`enhance::Material::assemble`), whether or not the texture has a
replacement image. It is one RGBA8 image, the size of the normal map (else
the specular's, else the gloss's; the others are resampled to it
bilinearly with repeat addressing): red and green the normal's x and y,
encoded as in the file, with y turned to point along increasing v under
`normal_y up` and any x, y longer than 1 scaled back to 1; blue the
specular scale; alpha the gloss; 128 where the pack has no map of that
kind. The pack-wide strengths times the `material` line of the stem each
map was found under go with it. Mip levels are made at load, never read
from a pack: normals averaged as unit vectors and renormalised, the rest
box-averaged with the images' weights. The CLI (`--texture-pack`) loads a
material on first use; the player decodes and assembles it on its pack
threads; the browser fetches its files like images (`pack_wanted`) and
assembles them when all have arrived. The GPU renderer asks for a texture's
material only with `gpu::Quality::materials` on (default off; nothing in
the player or the browser turns it on yet; `remodel --materials` does for
`gpu-check`, `depth-check` and `pack-check`), uploads it as one
`Rgba8Unorm` texture (with mips when `tex_filter` has them) and keeps it
under the same budget as the images (below). Nothing samples it yet: frames
are identical with materials on or off (checked 2026-11-11 with a synthetic
pack on `topskatr`'s captures, `gpu-check` unfiltered, mipmapped and 16x).

**Models.** Replacement models are not built; the format only reserves
their names. They will key by content (a hash of the model's polygon data
as the geometrizer reads it), not address. Models a game rewrites every
frame cannot be replaced by a static mesh: `lastbrnx` rewrites its
fighters' joint models in polygon RAM each frame, so their content never
repeats, and direct data has no model. They keep the game's polygons.

**`make pack-lint PACK=DIR`** (`remodel pack-lint DIR [--dump DIR]
[--max-side N]`, no ROM, no GPU) checks a pack against all this and exits
non-zero on errors:

| Check | Level |
|---|---|
| `pack.txt` parses; its `format` and `keys` are known and its `needs` shown (else readers refuse it); unknown keywords and features, reserved ones, `material` lines whose stem has no files, a `game` reModel doesn't know | error / warning / note |
| every PNG's name parses as a pack name (a reserved name is a note) | warning |
| `index.txt` lists every file and nothing missing (the browser needs it) | warning |
| every PNG decodes (not interlaced) and is within `MAX_SIDE` pixels a side (default 4096, as `upscale-textures`) | error |
| images and maps have the proportions of their texture's `w` x `h` (within a pixel); tile images are square multiples of 8; a map is its image's size and all maps of one stem agree | warning |
| with `TEXDUMP=DIR`: files whose key the dump never drew; a map for an image whose surface also has one (the image's would win) | note |
| GPU memory with everything drawn, images and materials, plain and mipmapped, against the 512 MB native and 192 MB browser budgets | note |

On the 4x ESRGAN pack of `topskatr` (241 images, 5,030 tiles): clean, one
note (182 MB of images, 242 MB mipmapped: past the browser's budget, so
the browser re-fetches the least recently drawn).

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
about a third more. Material maps, when loaded, share that budget and that
order with the images (a material is dropped on its own; one the size of
its image doubles that texture's cost), and `PackStats` counts them apart
(`materials`, `material_held`, `material_bytes`). Tile images are outside that budget: their atlas is
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
renderer without a pack, and writes the pictures to `OUT/pack-identity`.
The pack is not pixel-identical to the original: its colours are filtered
where the original filters texel values and looks them up. The last column
splits the software's differing 3D pixels (off by more than 16 levels) by
cause, as shares of the frame: the texels under the bilinear filter hold
different values (*filter order*: the original filters texel values and
then looks the result up, the pack filters the looked-up colours), one of
them is a cut-out (texel 15 of a translucent texture), they hold one value
(*same*: a pack standing for the texture must match there, so these are
real gaps: a wrong look, lighting the replacement can't express), or
another polygon is on top in one of the two (*other*: a cut-out edge
decided differently).

It fails (exits non-zero, naming each frame and why) if any single frame,
unfiltered (`identity_faults` in `crates/cli/src/pack_check.rs`):

- has more than **0.003%** of its pixels *same* + *other*
  (`IDENTITY_FAULT_LIMIT`; 5 pixels at 496x384). Without a fault these were
  at most 0.001% on every frame of srallyc, topskatr and sgt24h; the collar
  gap below, with its fix reverted, is 0.003-0.007% of topskatr's frames
  2100, 2400, 2700 and 5100, and fails four frames (0.005% would have
  caught it on 2700 alone);
- has more than **5%** of its pixels off on the GPU or in software, or 5% of
  them filter order + cut-out (`IDENTITY_CEILING`, about twice the worst
  frame, topskatr 3600 at 2.6%): a sanity ceiling, so a pack that looks
  wrong across much of a frame fails even where the split puts it down to
  filter order;
- has the GPU's share more than **0.1** percentage points from the
  software's (`IDENTITY_GPU_SW`; at most 0.025 measured): the split is the
  software's, so the GPU is held to agreeing with it.

Below these, filter order and cut-out pixels are reported, not held to a
limit, as are the means and mipmapped drawing (a different filter). With
the depth buffer (`GPU_OPTS=--depth`) the split isn't computed, so no
pixel can be put down to filter order and the check falls back to holding
each renderer's whole share to 1% a frame (`IDENTITY_TOLERANCE`), which
topskatr 3300 and 3600 fail by filter order alone: strict, not silent.

Why filter order is not a fault (2026-11-11): the frames where it is over
1% (topskatr 3300, 3600: 1.7%, 2.6%; sgt24h 2400, 2700, 3000, 6212:
1.9-2.1%, with each frame's own textures or topskatr's dump) have
99.6-100% of their differing pixels filter order, 0.0-0.4% cut-out edges,
none *same* and almost none another polygon; and drawing the original with
its colours filtered instead of its texel values gives what the pack draws
in 98-100% of them (the rest: lighting at luma below 255, also between
texels). Own-frame packs rule out looks, textures rewritten after a dump
and textures never dumped. They are the frames with stadium crowds and
sponsor boards in view, drawn small (91-98% of the differing pixels are
minified, the texture's texels smaller than a pixel): crowds use the
translation tables as a palette, texel value to an unrelated colour, so
the original's in-between values show colours of neither neighbour (24-42%
of the differing pixels are between texels whose colours don't rise or
fall with the value), and lettering edges fall on the tables' non-linear
curve. Neither filter is wrong for a pack (the hardware's own filter is
unconfirmed, docs/hw/geometrizer.md 5.4), and a pack cannot filter texel
values it doesn't have. The one gap the split found (topskatr, *same*, a
texel black at full luma in a channel but not below it) is fixed: see
"Lighting" above. `make pack-identity` passes on srallyc, topskatr and
sgt24h with each frame's own textures, and on topskatr with its dump.

Measured 2026-11-11 at native size, mean share over the captures,
unfiltered (GPU / software):

| pack | srallyc | topskatr | sgt24h |
|---|---|---|---|
| each frame's own textures | 0.30% / 0.31% | 0.50% / 0.50% | 0.71% / 0.71% |
| dump, first look kept (before the fix) | 59.4% / 59.4% | 0.49% / 0.49% | 13.1% / 13.2% |
| dump, most used look (`make dump-textures`) | 5.6% / 5.6% | 0.49% / 0.49% | 1.6% / 1.6% |
| dump, `BENCH_STEPS=900000000` | 5.2% / 5.2% | | |

With the fix, srallyc's race frames are 0.13-0.83% off; what is left is the
fade (frame 2700, above) and, with the default dump run (shorter than the
capture's), textures it saw only in the fade (frame 5700's start banner and
boards, 8%); sgt24h's are frames 2400-3000 (about 2%, as much as with each frame's own textures: the filter order, above) and 6212 (16%,
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
