# Texture dumps and texture packs

An enhancement in the spirit of Dolphin's custom textures: `remodel run` can
write out every texture the game draws, and draw its snapshots with edited
or upscaled replacements. Nothing here models hardware; with neither option
given, rendering is unchanged (`make bench` hash and native snapshots are
identical).

```sh
make dump-textures                       # race run -> out/topskatr/texdump/
# edit / upscale the PNGs you care about into a directory, same names, then:
make run STEPS=600000000 SNAP=60 INPUT=coin1@1100-1104,start@1250-1254 \
    PACK=out/topskatr/texpack VIEW="--height 1080 --aspect 16:9 --hud edges"
```

`remodel run` options: `--dump-textures DIR` and `--texture-pack DIR`. Both
act at snapshots (every `SNAP` frames and the final frame), so a texture only
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

Images are loaded the first time their key is drawn and cached for the run.

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

## Limitations

- Only snapshots use packs; there is no live display yet.
- Dumps happen at snapshots, so short-lived textures can be missed.
- Mipmaps and microtextures are not modelled by the renderer, so packs do not
  have them either; a large upscale on a distant polygon can shimmer.
- Textures still being uploaded when a snapshot is taken get transient keys
  (the game copies texture data across several frames).
- Keys include colours, so a texture or tile drawn in many colours (fades,
  flashing text) needs one replacement per colour.
