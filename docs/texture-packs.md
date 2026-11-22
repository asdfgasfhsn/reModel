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
and the browser. `make pack-materials` makes normal maps and `make
pack-lint` checks a pack (below). Normal maps are drawn by the software
renderer (`run` snapshots, `pack-check --materials`; below, "How material
maps light") and the GPU renderer (docs/gpu.md, "Material maps"), which
the player and the browser use behind their `materials` setting (off by
default; below, "Trying material maps"; docs/enhancements.md, "Material
maps"). Specular (`.s`) and gloss (`.g`) maps are part of the format and
are loaded, but have no effect yet (`pack-lint` notes them; the roadmap is
in docs/enhancements.md).

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
| `regions.txt`, `region_<name>.png` | 2D region overrides (below, "Regions") |
| `tex_<key>_<w>x<h>.n.png`, `.s.png`, `.g.png` | material maps for that image in that colour: normal, specular scale, gloss |
| `surf_<skey>_<w>x<h>.n.png`, `.s.png`, `.g.png` | the same maps keyed by surface, serving every colour the texels are drawn in |
| `art_<skey>_<w>x<h>.png` | one image for the artwork: drawn for every colouring of the texels (every `tex_` key of that surface) that has no `tex_` image of its own (below, "Artwork images") |
| `materials.txt` | tool record of `make pack-materials` |
| `materials-exclude.txt` | the author's list of textures `make pack-materials` makes no map for (below, "Material maps") |

Keys are 16 lowercase hex digits and sizes plain decimals, exactly as the
renderers write them; any other spelling is never opened. Tiles have no
material maps (the 2D layer has no lighting): `tile_<key>.n.png` is not a
pack name. File prefix `region_` is region images (below, "Regions"). Reserved, not to be used for anything else: map suffixes `.e`
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
game's specular exponent times `2^((R - 128) / 32)`. Readers accept and
load `.s` and `.g` maps, but nothing draws them yet: they have no effect.
A map covers the
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
| `version`, `author`, `licence`, `ai`, `program` | what the pack says about itself, free text and never used for lookup, except as `depends` reads `version`: `ai` is `none`, `assisted` or `generated` (any other value refuses the manifest); `program` the ROM hashes it was made against, never ROM data | none |
| `depends NAME [>= VERSION]`, `conflicts NAME` | the pack that must sit below this one in a stack (at least that dotted version), and one that must not be in the stack at all; given any number of times. `NAME` is the other pack's `name` line, else its folder name | none |
| `uses a,b` | optional features the pack contains (`materials`, `regions`); names a reader doesn't know are ignored | none |
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
   that can't show it refuses the whole pack. Every reader (the CLI, the
   player, the browser) draws material maps, so `needs materials` is
   accepted everywhere (`model2::pack::SHOWN`). Artwork images
   (`art_`) are drawn by the CLI and the player, not the browser: `needs
   artworks` is refused there and `uses artworks` ignored. Regions are drawn by the
   CLI and the player, not yet the browser (`SHOWN_WEB`), so `needs regions` is
   refused there, `uses regions` ignored; whether the maps are drawn
   stays the user's `materials` setting, and with it off such a pack shows
   its images without the maps. `uses materials` is informative.
4. Tools write `pack.txt` only when the pack holds something format 1
   can't say; a plain upscaled pack gets none (`make upscale-textures`
   writes none) and stays readable by older builds, which show its images
   and nothing of the rest.

**Layers.** The CLI's `scene render` takes `--pack` several times,
highest first, and draws the stack (`model2::layered::LayeredPack`; the
renderer sees one pack). For each texture, tile or material map the highest
pack that has it wins; an image still decoding holds the answer, so a lower
layer is never shown for a frame. A pack's region set hides every lower
pack's (region sets are not merged). `depends` must name a pack beneath, at
the version asked, and `conflicts` a pack not in the stack: a stack that
breaks either is refused, with the reason. `remodel pack resolve DIR...`
(highest first) says which layer won each asset and what it hid, and the
stack's GPU memory with mipmaps against the 512 MB native and 192 MB browser
budgets, warning where a layer uses something the browser does not show.
The player and the browser still load one pack; each layer is a plain pack
and can be used alone.

**`look.txt`** is a pack file: `pack-lint` knows it (and warns when it does not
parse), and `pack build` copies the project's. The player already loads
it with the pack.

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
material only with `gpu::Quality::materials` on (default off; the player's
and the browser's `materials` setting turns it on, `remodel --materials`
for `gpu-check`, `depth-check` and `pack-check`), uploads it as one
`Rgba8Unorm` texture (with mips when `tex_filter` has them) and keeps it
under the same budget as the images (below), for polygons that have a lamp
(as the software renderer asks). It draws them as the software renderer
does (next paragraph; docs/gpu.md, "Material maps").

**How material maps light** (the software renderer; the GPU renderer
draws the same). A map changes a pixel's luma and nothing else: the colour
still comes from the game's luma table and colour tables. Only polygons
with the exact lighting layer have a lamp to perturb (`--materials`
records the side channel, as `--light`; the native geometrizer; a block
the self-check demoted, the chip path or a capture without `light.bin` get
none, and draw exactly as without materials). With materials on, the
lighting pass hands over the lamp of every such textured polygon whose
lighting depends on its normal (`light::Options::lamps`), lit or not and
whatever `shading` is; per polygon the renderer works out a tangent frame
from its corners and texture coordinates (least squares, so constant over
the polygon; none where the coordinates are degenerate). Per pixel, with N
the normal as drawn (the interpolated corner normals of a smooth-shaded
polygon, else the face's) and t the map's normal there (times the pack's
strength; its x component negated in every second repeat of a mirrored
axis), N is tilted by t in that frame about the side the eye sees. Then
one of two rules (`gpu::Quality::materials_relief`, the CLI's
`--materials-relief off|S`):

- **Relief** (the default, strength S = 2; 0.1 to 8): the pixel is drawn
  at `1 + S (max(d', 0) - max(d, 0))` times the brightness the flat
  surface is drawn at, `d` and `d'` the game's facing-signed `N . L` at N
  and at the tilted normal (ambient and specular left out; the factor is
  at least a quarter and at most 4, so a map's steepest texels leave a lit
  surface dark, not black). The factor is applied per texel, to the luma lowered to where
  that texel's top colour column starts (`raster::relieved`), so a surface
  the game lights to full luma, or past where its texels stop brightening,
  still shows relief, mostly as darkening on the slopes facing away from
  the light (the mean brightness drops a level or two: 0.5 to 3 levels of
  255 in the review frames, more on the strongest maps). Nothing where no
  light reaches the flat normal or the lamp has no diffuse term.
- **Faithful** (`off`): the luma becomes `base + E1 - E0`, base what the
  pixel has without materials (the stream's luma + 0.5 for a polygon that
  isn't lit), `E0` and `E1` the game's formula at N and at the tilted
  normal, each clamped to 0-255: the map's normal in the game's own sum.
  A polygon lit to full luma whatever its normal (below) shows nothing.

Either way a flat texel changes nothing and the pixel takes the path
without materials bit for bit, so a pack of flat maps draws exactly the
pack without them, in every shading mode, given the same side channel: the run to compare with
is the one with `--light` in place of `--materials`. Without either, `run`
records no side channel, and vertex and pixel shading fall back to the
stream's lumas; that picture differs from the one with the side channel
whatever the pack holds (`topskatr`, 900M coin-start, every 300th frame,
`--shading pixel`: 15 of 21 snapshots, frame 4800 8% of pixels, mean +0.28;
flat shading identical). `make materials-identity PACK=DIR` makes the
pack's maps flat and checks this comparison, in software and on the GPU
(`topskatr`, both rules: flat, vertex, pixel and mipmapped
pixel, 21 snapshots each identical, 14334 polygons drawn with a material
and no pixel changed; `gpu-check` of 7 captures identical in vertex and
pixel shading, mip, 16x, 4x MSAA with depth and temporal; with shading
off a handful of pixels one level apart, docs/gpu.md, "Material maps").
A changed pixel always blends the two colour columns around its luma (the
arithmetic of `shading_fine`, pure black kept), whatever `shading_fine`
says, and its luma is clamped to the lumas over which the polygon keeps
its hue and its texels' columns get brighter with the luma
(`raster::ramp_range_with`): a game may paint with a colour's top column
(`topskatr`'s sign lettering is the black top column of a grey), and a
relief that darkens must not pick the grey below it. A map can only change
the light the game casts: a polygon with an ambient-only lighting entry
shows nothing of it, and under the faithful rule neither does one lit to
full luma whatever its normal (`topskatr` lights much of its scenery with
ambient 255), which is why relief is the default.

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
| specular (`.s`) and gloss (`.g`) maps: loaded, but no effect yet | note |
| `index.txt` lists every file and nothing missing (the browser needs it) | warning |
| every PNG decodes (not interlaced) and is within `MAX_SIDE` pixels a side (default 4096, as `upscale-textures`) | error |
| images and maps have the proportions of their texture's `w` x `h` (within a pixel); tile images are square multiples of 8; a map is its image's size and all maps of one stem agree | warning |
| normal maps: red, green and blue a unit vector (0.9 to 1.1 long) on 99% of texels where blue is used at all; x² + y² at most 1 on 99%; blue not under 128 (pointing into the surface); a map that is entirely flat | warning |
| normal map texels under the image's transparent texels (the pack image's alpha, or the dump's) near the mean of their opaque neighbours (they show in mip levels and at cut-out edges) | warning |
| with `TEXDUMP=DIR`: on an axis the texture is drawn mirrored (`wrap=`), the normal's component across the edge near 0 there; on an axis drawn only repeated, no jump from the last texels to the first larger than the steps beside it (not checked where the image itself doesn't tile or has a line along the edge) | warning |
| with `TEXDUMP=DIR`: files whose key the dump never drew; a map for an image whose surface also has one (the image's would win) | note |
| `materials-exclude.txt` entries that aren't a `tex_`/`surf_` stem, or name nothing in the pack or the dump (without `TEXDUMP=` only the pack's own files are known) | warning |
| GPU memory with everything drawn, images and materials, plain and mipmapped, against the 512 MB native and 192 MB browser budgets | note |

On the 4x ESRGAN pack of `topskatr` (241 images, 5,030 tiles): clean, one
note (182 MB of images, 242 MB mipmapped: past the browser's budget, so
the browser re-fetches the least recently drawn).

## Artwork images

A `tex_` key names one colouring of an artwork: the same texels drawn in
another polygon colour or luma table are another key, and Top Skater has 62
artworks under up to 12 keys each, which carry half the drawn pixels. An
`art_<skey>_<w>x<h>.png` names the artwork instead (the surface key, which
the dump index, the catalogue and the pack log already record as `surf=`).

A reader looks for the colouring's own `tex_` image first and draws the
artwork's `art_` image when the pack has none, in the CLI's software and GPU
renderers and in the player, lit as any replacement is (the game's
brightness for the polygon scales it; its colour does not re-tint it).
Every colouring then shows the one image: this is mode `fixed`.
The stand-in the GPU draws for a key without an image (another key of the
surface) is never needed for such an artwork. A pack whose colourings are
all drawn from `art_` images says `needs artworks`; `uses artworks` marks
a pack that only has `art_` files besides complete `tex_` copies.

`remodel studio bake` makes the other mode, `baked`: it writes each
colouring's `tex_` image from the artwork's, through the colour ramps the
catalogue recorded (docs/uplift-studio.md, "Artworks"), so any reader draws
it, the browser too, and any one of them can be replaced by hand. `art_`
files have no material maps (name those by `tex_` or `surf_`).

## Regions

Tiles are 8x8 and a pattern is one key wherever it is drawn, so a tile image
can't reshape art that spans several tiles when its tiles are shared: a logo
built from the same fill tiles as the background, a credit line whose
letters are also another word. A **region** says "where this arrangement of
tiles is in a tile map, draw this image over that rectangle instead".
Nothing about it is game-specific; it is part of format 2 (`uses regions`).

**Files.** `regions.txt` (below) and one image `region_<name>.png` per
region, any size at least 8 pixels per tile in each direction (4x, 32 pixels
a tile, is typical) and in the proportions of the rectangle (an uneven size is
cut at integer fractions; `pack-lint` notes it). `pack.txt` says `uses
regions` (a pack made by `remodel studio pack` does); the file is only opened
for a pack with a format 2 manifest that does, so a pack that merely holds a
`regions.txt` is read as it always was.

```
# reModel regions
region	logo-a	3x2	cells=0123456789abcdef,*,_/fedcba9876543210,0000000000000001,*	plane=a	layer=high
```

`region NAME WxH cells=ROW/ROW...` then optional `k=v` fields, separated by
tabs or spaces. `NAME` is lowercase letters, digits, `-` and `_`. `W` x `H`
is the size in tiles (1 to 64 each way). `cells` has `H` rows of `W`
comma-separated cells, each a **tile key** (the 16 hex digits of
`tile_<key>.png`, which includes the pen colours), `*` (any tile: animated or
irrelevant parts) or `_` (a blank tile: all pen 0, whatever its colours). A
region of nothing but wildcards is an error. Fields: `plane=a|b` (plane A is
maps 0 and 1, B maps 2 and 3; default either), `layer=high|low` (every tile
that isn't `*` has, or hasn't, the entry's priority bit; default either),
`at=MAP:X,Y` (only at this cell, map 0-3, cells 0-63; default anywhere).
Unknown fields are ignored (`pack-lint` notes them), unknown keywords and `#`
comments too. A malformed line, or a name given twice, refuses the file with
its line number (the pack then draws without regions).

**Matching.** In **map space**, not on the screen: each frame, for every
enabled tile map (the four 64x64-cell maps), the layout of each region is
looked for at every cell, wrapping at the map's edges, with each cell's key
being the pattern the map entry names (the tile and its palette bank, as
`tile_<key>` does). This is why it is found wherever the game scrolls it, with
line scroll, plane splits and window masks (a pixel's cell is where the map
puts it), why a region half off the screen draws what shows, and why one
the game draws in a higher plane over a part of it simply hides that part. The
alternatives were rejected: a fixed screen position breaks with any scroll
(`at=` pins a *map* cell for the few cases that want that), and matching
screen pixels would have to undo each line's scroll. A region matches
wherever its arrangement is (a logo drawn twice is replaced twice); where
two regions overlap the one earlier in the file takes the cell. Take
enough of the picture that the arrangement is distinctive: a
rectangle of one repeated tile matches any run of it (`studio region-capture`
warns). Pen colours are part of a key, so a palette fade makes new keys and
the region stops matching until the colours come back (mark such cells `*`,
or keep to the cells whose colours hold).

**Drawing.** Each cell of a matched region shows its piece of the image
(the image cut at integer fractions into `W` x `H` pieces) in place of the
tile pattern: same plane, same priority bit, same transparency rules as
tile images (alpha < 0.5 is transparent; on the opaque back plane the
original pen shows through; elsewhere what is behind), sampled at output
resolution, bilinear and clamped to the piece. Everything else in the 2D
layer is as before, and a region cell wins over a tile image for the same
pattern. Wide views' added side columns keep the original patterns, as for
tiles. It is render-only: no emulation, state hash or capture changes, and a
pack without regions draws exactly as before (checked: software
snapshots and GPU frames of a pack with tile images, with and without an
empty or non-matching region set, are identical).

**Readers.** The CLI (`--texture-pack`: snapshots, `gpu-check`,
`pack-check`) and the native player draw regions, in software and on the GPU
(each piece takes a cell of the tile atlas, found per map cell, so
`compose.wgsl` is unchanged; the atlas's cells are sized for the largest
image, regions' included). The images load when the pack opens. The
**browser does not draw regions yet** (its pack loader fetches images by
name from `index.txt` and has no `regions.txt`): it shows the pack's tile and
texture images, ignores `uses regions`, and refuses a pack that
`needs regions` (`pack::SHOWN_WEB`).

**Making them.** `remodel studio region-capture` (docs/uplift-studio.md,
"Regions") cuts a region's layout out of a captured frame and writes the
original for an artist to redraw at Nx.

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

**GPU memory.** Images are uploaded ahead of their first draw when there is
room (`preload`, "Flicker") or when first drawn, and kept; over a budget
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

## Flicker: when replacements come and go

A replacement shows only while its key is in the pack and its image is on
the GPU, so anything that changes a key or delays an image makes the
original show for a frame or many: a flash, or a texture that jitters
between two looks. `make pack-log PACK=DIR` finds out which, per frame.

**The diagnostic.** `remodel run ... --texture-pack DIR --pack-log FILE`
(`make pack-log`) draws every frame of the run on the GPU with the pack as
the player does (its decoding threads, uploads and eviction), at the
arcade's pace (`--pack-pace off` for flat out), and the player has the same
`--pack-log FILE`. The log has a block per frame drawn and a line for each
texture (by key, with its surface, polygons, pixels and where on screen),
tile pattern (by tile and colour bank) and region looked up: `hit`, `up` (a
hit whose image was uploaded that frame), `held` (stood in for, below) or
`miss:REASON`: `not-in-pack` (key not in the pack and nothing like it),
`colour` (the same texels, another colour: a fade or colour-table change),
`texels` (another texture in a sheet rectangle that had a replacement a
moment ago: a transient upload), `sibling` (another key of its surface has
an image), `pending` (decoding), `evicted` (given up over the budget),
`full` (tile atlas), `unmatched` (a region that matched a moment ago).
Nothing is gathered without the flag. `remodel pack-flicker LOG` (`--top N`)
reads it: totals, misses by reason, and the subjects that change between
replacement and original over the run, ranked by the pixels either side of
the changes, with frames, positions and reasons.

**What it found** (Top Skater, the cel showcase pack, `coin-start` to frame
9664, 3D by surface; tiles by pattern and bank). Before, 9564 changes
between replacement and original (20.9 M px involved), of which:

- Colour (the largest by pixels): a texture drawn in another colour has
  another key (the colour is part of it), so the original showed whenever
  the game changed a polygon's colour: one 128x128 surface alternated every
  frame (282 changes), 450 M px of 3D and 3.7 M px of tiles missed that way
  in the run, and 265 M px more were another key of a surface the pack did
  replace (`sibling`).
- First appearance: the original shows until the image is decoded and
  uploaded (`pending`): 18 M px of 3D and 0.6 M of tiles, at the first
  frames of the attract, coin-up and the character select (frames 1012,
  1254, 2893).
- Tile atlas: the pack has 5970 tile images and the atlas 4096 cells; a
  tile given up for room had to be decoded again (`evicted`, `pending`).
- Not the cause here: regions matched steadily (each region's changes are
  scene starts and ends, 30 in all, none while on screen); transient keys
  (`texels`) did not occur in these runs; memory (197 MB held of 512).

**Fixes** (render side; the game's state is untouched, `state hash`
identical; all on by default, `--pack-fixes none|all|NAMES` with `surface`,
`hold`, `preload` to compare):

- `surface`: a key not in the pack whose surface (the same texels in
  another colour or luma table) has an image under another key draws that
  image, as is: the author's art in the original's other colour, not a
  flash of the original. The renderer learns which key stands for a surface
  as they are drawn. Not tinted: a tint would need the game's colour tables
  (they map texel to colour per colour column), so a faded or tinted
  surface shows the image at its own colours, lit as any replacement is. A
  pack that deliberately leaves one colour of a texture original cannot
  have that with this on: turn it off (`--pack-fixes hold,preload`).
  Also, for up to 8 frames, a sheet rectangle that just drew a replacement
  and now shows other texels (a texture being copied in across frames)
  keeps its image.
- `hold`: an image being decoded or given up (`pending`, `evicted`) is
  stood in for by the same surface's or rectangle's image, if any; a tile
  pattern whose colours changed, or that is not loaded, keeps the last image
  drawn for it (the cell is checked to still hold it). The pack reader also
  keeps decoded tile images (4 KB each), so a tile asked for again after the
  atlas gave it up is there at once.
- `preload`: the pack's images are decoded on its threads from the start and
  uploaded ahead of their first draw: up to 2 textures (1 M texels) a frame
  while under 75% of the budget, and tiles 64 a frame into free atlas cells
  (256 kept free; preloaded images are the first to be given up). Everything
  of this pack is on the GPU a few seconds in.

**After**, the same runs: 1091 changes (263 k px), 3D 2898 -> 4 changes,
20.1 M -> 0.12 M px; tiles 6636 -> 1057, 0.79 M -> 0.14 M px; `colour`,
`sibling` and `evicted` misses gone, `pending` down from 18.2 M px (3D) and
0.59 M (tiles) to none and 0.09 M. What is left are single-frame changes where a
key appears that is in nothing the renderer has seen (`not-in-pack`: the
first frame of a new texture before its sheet rectangle ever drew a
replacement; a tile at a scene start), which needs a pack-side image for
that key. The character-select run (3727 frames): 6403 -> 5935 changes of
which 5896 tiles, 20.4 M -> 0.83 M px; the player on a real window (1500
frames, 120 Hz) agreed: 4567 -> 884 changes, 53 M -> 0.6 M px.

Other cases: a key shared by unrelated content (the same texels, e.g. a
flat colour or a sole and a banner) is replaced the same everywhere; the log
cannot tell, the pack author can (`make texmap`'s groups). Textures that
change colour *without* changing key (`looks=`, above) don't flicker, they
show the look drawn most.

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

## Material maps

`make pack-materials` makes normal maps (`.n`, "Pack format" above) for a
pack from its images (`remodel texpack-materials PACK`, no ROM needed):

```sh
make pack-materials GAME=topskatr TEXPACK=out/topskatr/texpack-esrgan TEXDUMP=out/topskatr/texdump
make pack-lint PACK=out/topskatr/texpack-esrgan TEXDUMP=out/topskatr/texdump
```

**The result is a starting point to edit by hand.** The built-in generator
reads relief from brightness, and brightness is not height: dark paint
reads as a groove, a light one as a ridge, and shading painted into the
texture as slopes. Lettering and logos come out engraved, a face's painted
shading bands as contour lines, the shadows painted beside raised ridges
(a tread plate) as grooves. It does well on brick, stone, paving, bark,
foliage and concrete, where light and dark do follow the relief. Edit or
replace the maps where it is wrong, or delete those that do harm: reruns
keep every map it didn't make (below).

Variables: `TEXPACK` (the pack, written into), `TEXDUMP` (the dump the
pack was made from, default `OUT/texdump`: which textures, `wrap=`, `surf=`,
`uses=`; empty: the pack's own `tex_` images, one map each, padded
mirrored), `NORMALGEN`, `NORMAL_Y`, `STRENGTH`, `DETAIL`, `MIN_USES`,
`FORCE=1`. Per texture in the dump's index (tiles never: the 2D layer has
no lighting):

- **Source:** the pack's image, else the dump's; the map is the source's
  size. With `surf=` in the index one map serves every colour of the
  texels (`surf_<key>_<w>x<h>.n.png`), made from the colour that has a
  pack image and is drawn most; without, one per image (`tex_..n.png`).
- **Cut-outs:** transparent texels get their opaque neighbours' colour
  first, so an outline makes no slope; afterwards the map texels under
  them get the mean of their opaque neighbours' normals (never drawn at
  full size, but they enter the mip levels).
- **Edges:** relief is computed as the texture wraps: on a repeated axis
  the far edge is the neighbour (no seam where it repeats), on a mirrored
  one (or drawn both ways, or unknown) the image is reflected about its
  edge texels, so the normal's component across the mirror line is exactly
  0 and the mirrored copy meets it without a crease. Many textures are
  marked repeated but never drawn past their edge (signs, faces, panels);
  where an image's far edges don't meet (the brightness step across them
  is over twice the mean step inside, plus 2%), the axis is taken as
  mirrored, so the border gets no ridge (topskatr: 76 axes).
- **Built-in maths:** height = luminance (0.299 R + 0.587 G + 0.114 B, 0
  to 1) minus its Gaussian blur of `DETAIL` texels (so broad shading and
  colour areas don't tilt the surface; 0: off); slopes by a 3x3 Sobel
  operator, scaled to slope per texel of the texture (a 4x image gets the
  same relief as the original, only sharper); normal = normalise(-STRENGTH
  dh/dx, -STRENGTH dh/dy, 1) with y pointing up; stored as `128 + 127 v`,
  so a flat source gives exactly (128, 128, 255). Maps with no relief at
  all (a flat colour) are left out. Defaults `STRENGTH=2`, `DETAIL=8`
  (chosen by eye with the relief rule at strength 2, below
  "How material maps light": median per-map mean tilt about 5 degrees on
  topskatr and srallyc). `STRENGTH=1 DETAIL=4` is milder (on topskatr's
  brick wall half the texels lean under 5 degrees, one in twenty over 19,
  the steepest 42: mortar edges); stronger maps give brick and stone more
  depth, and painted detail more false relief too.
- **`NORMALGEN`:** an external tool's command with `{in}` (a directory of
  the prepared images: filled, opaque RGB, padded by 16 texels with the
  same edge rule) and `{out}` (where it writes normal maps of the same
  names and sizes), run once with `sh -c`, its output in
  `out/scratch/texpack-materials-GAME/normalgen.log`, stopped if it writes
  nothing for 5 minutes. `NORMAL_Y=up|down` says which way its green
  points. Its maps are cropped, read as the renderer reads them (x and y
  from red and green, scaled back to length 1, blue ignored), given the
  cut-out rule above, and written in the pack's convention. reModel only
  drives the tool; none is named or shipped. Without `NORMALGEN` no work
  directory is used.
- **The manifest:** `uses materials` is added to `pack.txt` (only that
  line changes); a pack without one gets `format 2`, `keys 1`, `game` and
  `uses materials`. Maps are written in the manifest's `normal_y`
  direction. `index.txt` is rewritten to list every file.

**Leaving textures out: `materials-exclude.txt`.** Put this file in the
pack (an author's file, never written by a tool: `materials.txt` is the
tool's record and is rewritten) with one texture per line, by the stem
the dump and the pack name it, `tex_<key>_<w>x<h>` (that image, in that
colour) or `surf_<key>_<w>x<h>` (the texels in every colour); `#` starts a
comment, blank lines are skipped:

```
# signs and faces: printed paint is not relief
tex_01612c313cc7bfdb_128x128   # a sign
surf_15014456e2d5d506_128x128  # a face, every colour
```

An excluded texture gets no generated map. If an earlier run made one, the
rerun removes it, but a map the tool didn't make (hand-made, edited) is
never deleted: the run reports it as kept and says the exclusion has no
effect on it. With `surf=` in the dump one map serves all the colours of
a surface, so a `tex_` entry excludes the map only when every colour of
that surface is listed; otherwise the run warns and still makes it
(`material <tex stem> normal=0` in `pack.txt` switches one colour's
normal map off in the reader). Entries that name nothing in the pack or
the dump, and lines that aren't stems, are warned about by the tool and
by `pack-lint` (check l).

**Finding a texture's stem.** The dump (`make dump-textures`) writes each
texture as `<stem>.png`, so look at the dump's pictures (or the pack's
`tex_*.png`) for the sign or face; the file name is its `tex_` stem, and
that texture's `surf=` field in the dump's `index.txt` (and the tool's
`surf_<surf>_<w>x<h>`) gives the surface stem. Its `frame=` says when it
was first seen. The tool prints `excluded <stem>` per entry that did
something.

**Incremental.** `materials.txt` records the settings and, per map, its
source's hash (with how it wraps) and the map file's size and time. A
rerun makes only maps that are missing or whose source changed, and
**keeps every map that doesn't match the record**: made or edited by hand.
A deleted map is made again. Other settings are refused; `FORCE=1` remakes
all the maps the tool made with the new ones and still keeps the
hand-made and edited ones (delete a map to have it remade).

On topskatr's 4x ESRGAN pack with a coin-start dump (241 textures, 194
surfaces): 186 maps (8 flat colours left out) in 4.3 s on 20 cores, 37 MiB
on disk, 116 MB of GPU memory if all were drawn (154 MB mipmapped);
`pack-lint` clean on the result. With the older dump in `out/topskatr/texdump`
(no `surf=`), one map per image: 234 maps (7 flat left out) in 8 s, 73 MiB.

### Trying material maps

Make the maps into a copy of a pack (the tool writes into `TEXPACK`; keep
the pack you made by hand or upscaled as it is), check it, then play with
them on (about 10 s for `topskatr`'s 234 maps):

```sh
cp -r out/topskatr/texpack-4x out/topskatr/texpack-maps
make pack-materials GAME=topskatr TEXPACK=out/topskatr/texpack-maps TEXDUMP=out/topskatr/texdump
make pack-lint PACK=out/topskatr/texpack-maps TEXDUMP=out/topskatr/texdump
make play GAME=topskatr PLAY_ARGS="--texture-pack out/topskatr/texpack-maps --materials --shading pixel"
```

A pack needs no images for maps to show. For a game without a pack, dump
its textures and make the maps into an empty folder; the game's own
textures are drawn with them:

```sh
make dump-textures GAME=srallyc
mkdir -p out/srallyc/texpack-maps
make pack-materials GAME=srallyc TEXPACK=out/srallyc/texpack-maps TEXDUMP=out/srallyc/texdump
make play GAME=srallyc PLAY_ARGS="--texture-pack out/srallyc/texpack-maps --materials --shading pixel"
```

`PLAY_ARGS` hold for that run only. To compare, press **Esc**, then
Settings, Picture (choose the **Custom** preset to see the rows): turn **Material maps**
on or off and pick a **Material relief**
(1, 2, 4, 8 or faithful); they apply at once on the paused frame, and the
help line says what the last frame did. Menu changes are saved, as
all are. Try `Shading: flat` with the maps on too: faces stay flat but
keep the maps' detail. Copying a pack resets file times, so
`pack-materials` takes every map in a copy as hand-edited: delete the
`.n.png` files and `materials.txt` in the copy to have them remade.

**The settings** (docs/play.md, docs/web.md): `materials` (off by default;
player `--materials on|off`, the menu's and launcher's "Material maps";
browser `?materials=1`) needs the texture pack on and is independent of
`shading`: with `shading = flat` the faces stay flat but carry the maps'
detail; with `pixel` the maps tilt the interpolated normal. Its rule,
`materials_relief` (player `--materials-relief off|S`, menu and launcher
"Material relief" 1, 2, 4, 8 or faithful; browser `?relief=off|S`): relief
at strength 2 by default, 0.1 to 8 in the settings file and on the command
line, or `off`, the faithful rule ("How material maps light" above). A
strength off the list (1.5) shows as one more choice, "custom (1.5)" in
the menu, "Custom (1.5)" in the launcher and the browser's panel.

**Where they have no effect,** and the player's menu line and window
title, the launcher's hint and the browser's status line say so: no texture
pack, a pack without maps (or without a format 2 `pack.txt`), the chip
geometrizer (`--geo chip`, `?geo=chip`: no exact lighting layer), and
polygons without a lamp in the frame (unlit, HUD windows, shadows, blocks
the side channel's self-check demoted; "no polygon had the game's exact
lighting" when that is the whole frame). The launcher's previews draw
without them.

**What to expect** (`topskatr`, `srallyc`, `lastbrnx`):

- Brick, stone, paving, wood planks and road get grooves and pits; the
  wood floor and brick walls of topskatr's courses show it best.
- The maps are made from brightness, so painted detail becomes bumps:
  lettering looks engraved, logos and signs embossed, faces get contour
  lines. List such textures in `materials-exclude.txt` (above) and rerun,
  or edit the maps.
- Scenes darken by a level or two: relief mostly darkens the slopes
  facing away from the light (0.5 to 3 levels of 255 on average in the
  review frames).
- Good results need hand-made or edited maps: the generated ones are a
  starting point.
- Relief needs lit surfaces: a polygon the game lights with ambient only,
  or one no light reaches, shows nothing; under the faithful rule neither
  does scenery lit to full brightness (much of topskatr's), which is why
  relief is the default.
- Small dark notches can show where a curved object's polygons meet (each
  polygon has its own tangent frame).

**Preview without playing:** with captures that carry the lighting side
channel (`make lighting-capture GAME=topskatr` saves them with `light.bin`
into `out/scratch/lighting/topskatr/capture`), `make pack-check GAME=topskatr
PACK=DIR LIGHT_IN=out/scratch/lighting/topskatr/capture GPU_OPTS="--materials
--shading pixel"` (add `--materials-relief off` for the faithful rule)
draws each frame with the pack's images without and with its maps on the
GPU and in software: `FRAME-mat-off.png` and `-mat-on.png` (GPU),
`-mat-sw.png` (software), `-mat-diff.png` (GPU off | on | difference x8)
and `-mat-cover.png` (green: a material the exact lighting layer lit;
orange: a material but no lamp; red: a lamp but a degenerate texture
frame; grey: no material; it follows the polygon on top in draw order, so
2D layers and cut-out texels over a material show the colour of what is
under them). Per frame it reports the share of textured pixels with a
material and of those with a lamp, the mean and largest brightness
change, the frame's mean change (flagged over 2 levels: a normal map
should move it little), the GPU against the software where materials
apply, GPU ms off and on, and the material memory held. `PROBE=X,Y@FRAME`
explains one pixel: texture and map files, lamp, normal, tangent frame,
the map's normal there, base and resulting luma.

## Looks

A **look** is a render profile that goes with a pack (or stands alone):
ink outlines, toon lighting with halftone dots, speed lines, and
per-texture tags. It is the file `look.txt`, in the pack directory or on
its own (`<data dir>/<set>/look.txt` in the player, `--look FILE` in the
CLI). Render only: nothing in it reaches the machine, and with its effects
off the picture is byte for byte the one drawn without it. The design is
in docs/uplift-studio.md, "Looks"; how the GPU draws it in
docs/enhancements.md, "Looks".

**Format 1.** Line-based UTF-8 like `pack.txt`: a keyword, a tab (or
spaces), its value; `#` comments and blank lines skipped; `format` first.
Every parameter has a default, so a file names only what it changes. A
`format` above 1 refuses the whole look (said, the look not drawn); unknown
keywords and unknown `k=v` fields of a tag are ignored and listed (the
player prints one line), so formats rise only when the meaning of
something already defined changes. Values out of range are errors that name
the line (a typo is never clamped away). `model2::look` is the reader.

```
# reModel look
format	1
name	Top Skater comic
game	topskatr
outline_weight	2.25
outline_colour	#120f1a
toon_steps	3
halftone	0.4
tag	tex_0123456789abcdef_64x64	outline=none
tag	surf_fedcba9876543210_32x32	toon=off emissive
```

| Parameter | Range (default) | Meaning |
|---|---|---|
| `outline` | on/off (on) | ink lines at depth jumps and creases, and between textures |
| `outline_weight` | 0-8 (1.5; Comic 5) | thickness in pixels on a 1080-line picture (a line is as wide as asked, on the nearer side of an edge only); scales with the output height |
| `outline_opacity` | 0-1 (0.9) | how solid |
| `outline_depth` | 0-4 (1; Comic 1.5) | sensitivity to depth jumps and creases (0: none from depth); a thing narrower than twice the line's weight (a wire, a fence's mesh) is drawn at a third |
| `outline_ids` | 0-1 (0.6; Comic 0.8) | strength of lines between two textures where the surface bends (0: none). Two textures on one flat surface (a floor of many pieces, a banner on a wall) never get one, however steeply the surface is tilted: that is paint, not a shape; a tag that names an `outline=` weight lines its texture there anyway |
| `outline_fade_near`, `outline_fade_far` | game units (0, 0) | distance where lines start to thin, and where they are gone, so far scenery isn't ink noise. 0 leaves it to the scene: full strength up to 100 times its typical distance (where half the picture's polygon area is further away), gone at 800 times, since games' units and depth ranges differ by orders of magnitude (Top Skater's median surface is at 45, its stands at 15,000) |
| `outline_colour` | `#rrggbb` (`#120f19`) | ink |
| `rim` | 0-1 (0) | a light band inside silhouettes (per texture with a `rim=` tag) |
| `toon_steps` | 0 or 2-6 (3) | the game's smooth lighting in this many hard bands (0: smooth). Needs smooth shading (the Comic style turns it on; `look-render` does for a look with steps) |
| `toon_floor` | 0-1 (0.45; Comic 0.35) | the darkest band's brightness, of the lit one |
| `toon_cut` | 0-0.9 (0.35; Comic 0.3) | brightness below which everything is the darkest band |
| `halftone` | 0-1 (0; Comic 0.6) | dots of ink in the darkest band, this share of its area |
| `halftone_pitch`, `halftone_angle`, `halftone_colour` | 2-24 px at 1080 (6), 0-90 (45), `#rrggbb` | the dot grid |
| `speed_lines` | 0-1 (0) | streaks from the middle of the picture; approximate: a fixed amount, not driven by speed |

**The Comic defaults** (`Look::comic`, the built-in look with no pack):
`outline_weight 5`, `outline_depth 1.5`, `outline_ids 0.8`, three toon
steps with `toon_floor 0.35` and `toon_cut 0.3`, `halftone 0.6`; the other
parameters are the defaults above. Tuned by eye on Top Skater (with and
without the cel restyle pack), Virtua Fighter 2, Daytona USA and Super GT
24h, 1080p. Toon bands follow the game's lighting, so a game's
smooth-shading layer matters: captured with `--geo native --light`
(`make lighting-capture`) the bands follow the game's own lighting exactly;
without it the stream's lumas are smoothed (also fine for the look, and
what the player does on the chip geometrizer). Native-geometrizer games
(2B, 2C) and `vf2`/`daytona` all give a lighting layer under `--light`.

**Tags.** `tag STEM k=v ...` names a texture by its image stem
(`tex_<key>_<w>x<h>`, this colour) or surface stem (`surf_<key>_<w>x<h>`,
these texels in any colour), as `material` lines do; the inventory of
`make texmap` is where to decide them (a group gets one line per texture).
Fields: `outline=W` (a multiple of the look's weight, 0.. 4, or `none`: HUD
pieces, signs, text), `toon=N` (2-6) or `toon=off`, `rim=X` (0-1),
`emissive` (self-lit: full brightness, ignores the game's lighting).
2D-layer HUD tiles are never outlined: the look works on the 3D layer.

**Where it is loaded.** The player: the pack's `look.txt` with the pack on,
else `<data dir>/<set>/look.txt`, else the built-in Comic defaults
(`Look::comic`); the file is watched and read again when it changes
(an edit shows within half a second; a bad file is reported and the
last good look stays). The Advanced page's *Comic look* row turns it on and
shows its parameters; what is changed there is kept as `look_tuning` over
the file. The CLI: `remodel --look FILE|comic [--look-set NAME=VALUE ...]
look-render` (`make look-render`) draws captured frames without and with
it for agents:

```
make look-render CAPTURE=out/topskatr/capture PACK=out/topskatr/studio/cel-pack/pack \
    LOOK=comic LOOK_SET="outline_weight=3 toon_steps=2" \
    VIEW="--height 1080 --aspect 16:9 --hud edges --aa 4" LOOK_OPTS="--frames 2100,3300"
```

It writes `FRAME-plain.png`, `FRAME-look.png` and `FRAME-sheet.png` (both)
and, with `--bench N`, the GPU time of each. `run` snapshots are software
and draw no look. The browser draws looks with the rest of the GPU
renderer (below, docs/enhancements.md); it has no look picker yet.

## Texture map

`make texmap` answers "which texture is where on screen, and what belongs
together" for any game that captures (`remodel texmap CAPTURE OUT`; no
GPU, render side only: emulation and state hashes are untouched). It draws
each captured frame with the software renderer, 1024x768 by default
(`VIEW="--height 1080 --aspect 16:9"`), and writes to `OUT` (default
`out/GAME/texmap`; the files are ROM-derived, keep them out of git):

| file | what |
|---|---|
| `frames/FRAME.png` | the frame |
| `frames/FRAME-ids.png` | the ID image: a pixel's RGB is `r + 256 g + 65536 b` = the ID of what draws it (0 none, 1 an untextured polygon, 2 and up a texture or tile pattern, the same in every frame of the run). What shows is what `compose` shows: HUD tiles over the 3D layer over the low-priority tiles |
| `frames/FRAME-uv.png` | where in its texture the pixel samples: red, green the position across it (0-255, wrapped; tiles 8x8), blue 255 where there is one |
| `frames/FRAME-contact.png` | the frame beside the textures it shows, biggest coverage first, each labelled with its ID (tiles on blue) |
| `frames/FRAME.tsv` | per ID in the frame: pixels, share of the frame, output bounding box, visible polygons, share of the texels that show and their box (`uv_*`), wrap (`rr`, `mr`, ... as `index.txt`), `span_u/v` (how many texture sizes the coordinates run over: above 1, it repeats), group |
| `ids.tsv`, `inventory.tsv` | the ID table; the run's textures and tiles ranked by coverage (frames, pixels, best frame, union of the texels shown, wrap, span, group, and what `index.txt` calls the source) |
| `groups.tsv`, `texmap.json` | the groups; everything above for tools |
| `textures/NAME.png` | each texture as the game draws it at full luma, under the dump and pack names (`tex_<key>_<w>x<h>`, `tile_<key>`): a restyled image goes into a pack under the same name |
| `index.html` | the static viewer: one file, works from `file://` (below) |

**Groups.** Polygons carry the object block of the display list they came
in (one model's polygons: a skater, a billboard). Two textures are linked
when pixels of different textures touch in the frame (4-neighbours) and
both are from the same block of the same window; tiles link likewise within
a layer (a line of text's letters). The touching pixel pairs add up over
the run, links under about one pair per 96 rows of the view are dropped
(a corner is not a link), and the rest are merged strongest first into
groups of at most `--group-cap` (default 24) members: an object block of
dozens of textures that all touch (a course) falls apart at its weakest
links. It is a heuristic: it finds parts that touch (a billboard's halves,
a ramp's faces), not parts of one model that are far apart on screen, and
a wide enough run can chain unrelated textures that share a block.

**Viewer.** `index.html` holds the frames (`--viewer-frames N` of them,
spread over the run), their ID and position images and every texture
embedded, so it opens by double click and needs no server or library.
Hovering a pixel shows its texture (preview, size, coverage, the part of it
that shows as a box, and a marker where this pixel is in it); a click lists
the group and tints its members; the gallery below ranks every texture and
tile of the run by coverage (filter by kind, id or name; clicking one
jumps to a frame that shows it). See `docs/uplift-studio.md` for how it
fits the restyling workflow.

## Limitations

- Tile keys include colours: tiles drawn during a fade or palette cycle
  keep the image of the same pattern (not tinted; `hold`, "Flicker"), and
  one never replaced in any colour shows the original. The sides a
  wide view adds (scrolled tile skies) keep the original patterns.
- Dumps happen at snapshots, so short-lived textures can be missed.
- Live, an image's decode, upload and mip chain are spread over the first
  seconds (`preload`, "Flicker"); one drawn before then shows the original
  (or a stand-in) until it is ready, and can hitch a frame. A frame that replaces many textures
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
