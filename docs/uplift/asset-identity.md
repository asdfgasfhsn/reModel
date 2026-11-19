# Asset identity and context bundles

Shared words, roles and rules: [README.md](README.md). Nothing here is
built.

## The problem

A pack names what it replaces by a key. Today's key names one *colouring*
of an artwork, not the artwork, and nothing tells a person or a model what
an asset is, where it shows, or what it must match.

**How the key is made (observed).** `raster::tex_info`
(`crates/model2/src/raster.rs:1486-1525`) hashes the texels and the size,
and that state is the **surface key**. It then goes on through the luma
table, the translucent flag and the polygon's colour to give the **image
key**, the one in a pack file's name. Position, mirror, wrap and mip level
are in neither.

**Why colour is in it (inferred).** A dump image is the texture as drawn at
full brightness in that colour. The renderer scales a replacement by the
game's lighting but never re-colours it for another variant
(`crates/model2/src/raster.rs:1609-1630`). So the key answers
"which picture is in this file", not "which artwork is this".

**What it costs (observed, from Top Skater's catalogue of three sessions).**

| Fact | Number |
|---|---|
| Image keys | 565 |
| Artworks (surface keys) | 417 |
| Artworks under more than one key | 62, with up to 12 colours each |
| Share of drawn pixels on those 62 | 49% |
| Showcase pack images | 303, covering 263 artworks: 40 are colour copies |
| Keys with no image that share an artwork with one that has | 47 |

A quarter of the keys are repeats, and they cover half the screen.
"Colour is part of the key" was also the largest cause of replacement
flicker in the showcase: 450 M pixels over one run.

**Related gaps (observed).**

- **A pack image cannot be named by artwork.** A `surf_` name without a map
  suffix is refused (`crates/model2/src/pack.rs:469`). Only material maps
  and look tags can use the surface key.
- **The stand-in is a repair, not a feature.** The GPU draws another key's
  image for a key that has none, untinted
  (`crates/gpu/src/lib.rs:1914-1931`). The software renderer looks up exact
  names only (`crates/cli/src/texpack.rs:273-279`), so snapshots and the
  player can disagree.
- **Some keys are not one thing.** 11 of the 565 textures are a single
  flat colour and 22 more have two. The docs record a key shared by a
  sandal sole and a banner (`docs/texture-packs.md:664-667`). Texels alone
  cannot tell these apart.
- **The catalogue remembers the first sighting only**: its place, colour
  and thumbnail (`crates/model2/src/idmap.rs:76-84`). The dump tool elects
  the most-used look for its picture (`crates/cli/src/texpack.rs:155-176`);
  the catalogue does not.
- **Sheet position is not an identity.** 12 of 109 upload offsets held
  more than one content across scenes (counted from the catalogue's
  `loads.tsv`).

Other projects learned the same lessons. Packs broke when an emulator
changed its hashing without recording the scheme; palette wildcards are
their main source of false matches; and one alias table mapping many dumps
to one replacement is the usual fix. (Read from PPSSPP's, Azahar's and
DuckStation's documentation; not from code.)

## Design

### Two levels, already computed

- **Artwork**: `art:<surface key>` with its size. The texels.
- **Variant**: the image key. One colouring of the artwork.

Nothing new is hashed. The surface key is already in dumps, the catalogue,
the pack log and the GPU's stand-in table. The change is to make the
artwork the name people and packs use, and the variant a detail.

Around those two levels:

- **A name.** The project gives an artwork a slug (`wall-base`). People and
  agents use slugs; the build step writes loader names.
- **Links.** `same-as` joins artworks that differ by a few texels, such as
  a re-uploaded copy. A tool suggests them; a person confirms. Never
  automatic: this is where other projects' false matches come from.
- **Flags.** `flat` (one or two colours), `shared` (drawn on many unrelated
  objects), `rewritten` (its sheet place changes content while on screen).
  A flagged artwork carries a warning into its bundle and its gates.
- **A second anchor.** The ROM address a texture was uploaded from is the
  same across scenes and copies. 88% of Top Skater's uploads have one. A
  hand join gave 380 of 565 keys an address, with no address shared by two
  artworks (inferred; needs a proper run). The catalogue should store it
  and use it to merge duplicates. It is not a pack name.
- **A scheme version.** `pack.txt` already has one: `keys`, which a
  reader refuses when it is higher than it knows
  (`docs/texture-packs.md:273-277`). Scheme 1 is 64-bit FNV-1a
  (`crates/model2/src/enhance.rs:479-499`) over the texel bytes and size.
  Test vectors should freeze it. If it ever changes, `pack rekey` maps old
  names to new through the catalogue.

### Packs can name the artwork

A new file kind, `art_<surface>_<w>x<h>.png`. By the pack format's own
rule it needs no new `format` number: readers ignore unknown kinds of
file, and a pack that depends on it says `needs artworks`
(`docs/texture-packs.md:278-281`). The
loader looks for the variant's own `tex_` image first, then the artwork's
`art_` image. Each artwork states how its image meets the variants:

| `colour=` | Meaning | When |
|---|---|---|
| `baked` | One image per variant, as today | A variant that needs its own art |
| `fixed` | One image, drawn the same for every variant | Art that should ignore the game's tint. This is today's stand-in, made deliberate |
| `follow` | One image, made for a reference variant, re-tinted as it is drawn | A colour nobody catalogued, so no image could be baked for it |

The usual case is `baked`, with the copies made by the build. The
artist or agent makes one image. `pack build` makes the other colourings
from it, as `studio variants` does today, and any one of them can be
replaced by hand. A pack's size is not a reason to avoid this: in the
showcase the extra colourings are 1.67 MB of 13.26 MB of 3D images.

`follow` is the fallback for a colour never seen, in place of today's
untinted stand-in. Its model: per channel, scale the image by what the
game's colour table gives for this variant's colour over what it gives
for the reference colour, at the original texel under the pixel. That is
the shape of the lighting ratio the renderer already computes
(`crates/model2/src/raster.rs:1609-1630`, `pack_lit` in
`crates/gpu/src/raster.wgsl`), with the same fallback where the reference
channel is zero.

**Spiked on 2026-11-19** (Top Skater's catalogue and the showcase pack,
at commit `7d8f099`; offline, on decoded images, without the game's real
tables):

- All 62 artworks with several colourings differ by polygon colour only.
  None differs by luma table or translucency. They carry 49% of the
  pixels sighted.
- **A polygon's colour is an index, not a strength.** Brightness does not
  rise with the number. Any tint worked out from the colour values is
  wrong: three such models predicted the game's own variants to within
  4 levels on 12 to 15 of the 62 artworks.
- **A lookup per texel value is nearly exact** on the game's own art:
  0.15 levels mean error over 148 pairs. That is the model above.
- **One ratio for the whole image**, which is what `studio variants`
  uses, serves 51 of 62 artworks within 4 levels but only 56% of the
  pixels: it fails on the large ground textures, where hue changes with
  brightness.
- **On new art the per-texel model has a cost.** It carries the
  original's texel-by-texel differences into the replacement, putting
  tones back into flat cel art that the artist took out.
- **The showcase's authors rarely wanted a tint.** Of 40 extra
  colourings in the pack, 18 are the same image as their reference, and
  most of the rest were deliberately redrawn (a ground in mint, lilac
  and burgundy). `fixed` matches 18 of the 40; no tint matches more
  than 2.
- In 17 of 148 pairs the reference colour has a channel at zero, where
  any ratio is undefined.

So `follow` is demoted from the usual case to a fallback, and the build
bakes. Not done: the real tables were not read, so the model was not run
on a colour the catalogue has never seen, which is the one case it is now
for.

The policy is set per asset in the project (`asset set ID colour=...`)
and written to the pack's `material` line for that artwork at build time.

Both renderers resolve names the same way, so a snapshot and the player
agree.

**Splitting a shared key** (the sole and the banner) needs the renderer to
know which model a polygon belongs to. That identity is on the CPU side
only today, so the split waits for
[scene-understanding.md](scene-understanding.md). Until then a `shared`
artwork is reported, and the pack author decides.

### The catalogue becomes the index of artworks

- Record every variant and placement of an artwork, not the first.
- Elect each artwork's picture by most-used look, as the dump does.
- Store the upload's ROM address per artwork, and the flags above.
- Write `artworks.tsv`, the merged index the tools below read.

### The context bundle

Everything needed to repaint exactly one asset, in one small folder.
`asset bundle ID` writes:

| File | Holds |
|---|---|
| `bundle.json` | Identity, name, size, wrap per axis, alpha rule, variants with colours and screen share, flags, partners, the scenes it shows in with boxes, the style rules for its category, the output size, the gates it must pass, its status |
| `original.png` | The elected look, at 1x and enlarged without smoothing |
| `variants.png` | Every colouring, labelled |
| `uv-mask.png` | The texels ever visible on screen, over all scenes (from `raster::visible_uv`, `crates/model2/src/raster.rs:1989`) |
| `tiled.png` | A 3x3 repeat, when the game repeats or mirrors it |
| `context-N.png` | Crops of up to three scenes where it shows largest, boxed |
| `brief.txt` | The prompt text for an image model, filled from the above |

It is built from the catalogue and the scenes' facts, with no emulator.
`bundle.json` alone is enough to decide what to do; an agent should need
at most two of the pictures.

## Interfaces

| Verb | Does |
|---|---|
| `asset list` | Filter by `--kind`, `--scene`, `--status`, `--flag`, `--category`; ranked by screen share |
| `asset show ID` | The `bundle.json` content, without writing pictures |
| `asset bundle ID` | Writes the bundle |
| `asset name ID SLUG` | Names an artwork in the project |
| `asset same ID ID` | Links two artworks; `--suggest` lists candidates with a difference score |
| `pack rekey` | Rewrites names after a scheme change |

An ID is accepted as a slug, an `art:` key, or today's `tex_` name, which
resolves to its artwork.

## The roles

- **Pack lead.** Says "replace the wall base" and means one row, not six
  hashes. `asset list --flag shared` shows where judgement is needed.
- **Art agent.** `asset bundle wall-base` is its whole briefing. It writes
  one image and never learns the variants exist. Budget: 2 calls and 2
  pictures to understand an asset.
- **Artist.** Opens the bundle folder: the original, the mask and the
  context crops are ordinary PNGs.
- **Tester.** The flicker report's `colour` and `sibling` misses should
  fall to zero for artworks set to `follow` or `fixed`.
- **Player.** Fewer untinted stand-ins. The pack is hardly smaller: 18
  of the showcase's 40 extra colourings are copies that `fixed` removes.

## Testing

- **Without ROMs:** test vectors that freeze both keys; the lookup order;
  `follow` with the reference variant equals the image byte for byte;
  bundle contents from a synthetic catalogue.
- **With ROMs:** the pack log before and after on the showcase pack;
  software and GPU frames agree as closely as they do today.
- **Draw-only:** state hashes with a pack that uses `artworks` equal
  those without.

## Performance

`follow` adds table lookups of the kind the lighting ratio already makes
per pixel; expected to be within noise, not measured. Memory falls with
the number of colour copies, which matters most in the browser's 192 MB
budget. Nothing here was measured in the browser.

## Open questions

- **Which tint for an unseen colour on flat art:** the per-texel lookup,
  which is exact on the original and noisy on cel art, or one ratio for
  the image, which is smooth and wrong on large ground textures?
- **Do other games' variants differ by more than colour?** Top Skater's
  do not. Luma-table variants would need the tone step the spike could
  not test.
- **Is the ROM address safe to keep in shared files?** It is an offset, as
  a key is a hash; it is only needed locally.
- **How different may two artworks be and still be offered as `same-as`?**
  Needs real near-duplicates to set.
- **Catalogue size.** Recording every placement grows it; by how much is
  not known.

## MVP: a few days

1. `art_` images with `colour=fixed` and `colour=baked`, in both
   renderers.
2. `artworks.tsv` and `asset list` and `asset show` from the existing
   catalogue.
3. Bundle v0: `bundle.json`, `original.png`, `uv-mask.png` and two context
   crops.
4. Variants baked by `pack build`, replacing the hand-run `studio
   variants` step.
5. `follow` for unseen colours, after trying both tints on a colour held
   out of the catalogue.
