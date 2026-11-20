# Asset identity and context bundles

Shared words, roles and rules: [README.md](README.md). Nothing here is
built into reModel; what was tried is in "Evidence" (spike code is on
unmerged branches, not for merging as it stands).

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

All 62 of those artworks differ by polygon colour only; none differs by
luma table or translucency (E1). Other games' counts are in E3.

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
from it, and any one of them can be replaced by hand. This replaces the
hand-run `studio variants` step. A pack's size is not a reason to avoid
it: in the showcase the extra colourings are 1.67 MB of 13.26 MB of 3D
images.

**A colouring is a ramp, not a tint.** The game's translation is small
and known: a texel value goes through a luma table into a row of the
palette chosen by the polygon's colour, which gives a ramp of 16 colours
(`crates/model2/src/raster.rs`). Both tables are in every capture, but
games rewrite them as they run, so one snapshot reproduces only part of
what was sighted (E2). The catalogue therefore records each key's
16-colour ramp (48 bytes) when it sights the key.

A polygon's colour is an index, not a strength: brightness does not rise
with the number, so any tint worked out from the colour values is wrong
(E1). The other games checked look the same (inferred, E3). The same artwork also
differs by luma table on some games, so the luma table is part of a
variant, and a tint must account for it.

**How the build bakes.** Place each pixel on the first colouring's ramp,
take the same place on the second's, and keep whatever part of the pixel
lay off the ramp. On the game's own art this is exact or nearly (E2).
Two things limit it:

- **Which colouring the author paints matters more than the method.**
  Baking from the colouring with the most distinct colours is best: a
  flatter ramp cannot give back detail it never had (E2).
- **It degrades with distance from the ramp, and it cannot predict a
  restyle** (E2). So `pack build` measures how far the authored image
  lies from the original ramp and chooses:

| Mean distance from the ramp | Build does |
|---|---|
| Under 4 levels (a faithful repaint or an upscale) | Bakes every colouring, silently |
| 4 to 12 | Bakes, and puts the result in the review list |
| 12 or more (a restyle) | Copies the image to every colouring and lists them as "may want its own art" |

It also copies when the two original ramps differ by under 4 levels. The
thresholds come from synthetic art only: no pack that stays near the
ramps exists yet to test them on.

**`follow` is the fallback for a colour never seen**, in place of
today's untinted stand-in. Its model: per channel, scale the image by
what the game's colour table gives for this variant's colour over what
it gives for the reference colour, at the original texel under the
pixel. That is the shape of the lighting ratio the renderer already
computes (`crates/model2/src/raster.rs:1609-1630`, `pack_lit` in
`crates/gpu/src/raster.wgsl`), with the same fallback where the
reference channel is zero (it happens: E1). A lookup per texel value is
nearly exact on the game's own art on all five games tried (E1, E3),
but on new art it carries the original's texel-by-texel differences
into the replacement, putting tones back into flat cel art (E1). It has
not been run on a colour the catalogue has never seen, which is the one
case it is now for: the real tables were not read (E2).

The policy is set per asset in the project (`asset set ID colour=...`)
and written to the pack's `material` line for that artwork at build time.
Both renderers resolve names the same way, so a snapshot and the player
agree.

**Splitting a shared key** (the sole and the banner) needs the renderer to
know which model a polygon belongs to. That identity is on the CPU side
only today, so the split waits for
[scene-understanding.md](scene-understanding.md). Until then a `shared`
artwork is reported, and the pack author decides.

### Palette packs: recolouring with no images

Proposed on 2026-11-20; nothing here is tried. A colouring is a ramp of
16 colours, and a variant's key already names its polygon colour and
luma table. So a pack can replace a variant's ramp and nothing else:
the game's own texels, drawn through 48 bytes the pack supplies.

```
ramp tex_<image key>_64x64   1a1020 2c1830 ... (16 colours)
ramp art:<surface>  colour=<n>  from kit-away.ramp
```

- **What it is for.** Team kits, car liveries, alternate costumes and
  colour-blind palettes. `vstriker`'s team kits are one artwork in 20
  to 28 colourings that differ by palette entry alone (E3), so on such
  a game a kit is a few ramps.
- **Why it should be exact.** It is the per-texel lookup, which is
  nearly exact on the game's own art on every game tried (E1, E3). The
  noise that rules it out for repainted art does not arise, because the
  texels are the game's.
- **Draw-only.** The renderer's translation is replaced for that key.
  Palette RAM is not written, so state hashes do not change (inferred
  from how pack images are drawn today; to be tested).
- **What is not known.** Games rewrite their tables as they run (E2):
  a fade or a flash changes the ramp a key draws through. A fixed
  replacement would freeze it, so the replacement may have to be
  applied as a change from the catalogued ramp. Whether untextured
  polygons, which take their colour from the same tables, can be
  recoloured the same way is not checked.
- **First step.** One `vstriker` kit and one `daytona` car recoloured
  by ramp, compared with the game's own other colourings, with state
  hashes checked. It needs the catalogue to record ramps (MVP item 4).

### Set aside

- **One ratio for the whole image**, which is what `studio variants`
  does today: it fails on large ground textures and across luma tables
  (E1, E3). The build's baking replaces it.
- **A tint worked out from the colour values**: wrong, as above (E1).
- **`follow` as the usual case**: authors rarely wanted a tint
  (inferred), since
  `fixed` matched 18 of the showcase's 40 extra colourings and no tint
  more than 2 (E1).

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
  of the showcase's 40 extra colourings are copies that `fixed` removes
  (E1; E2 counted 20 as the same image as the first).

## Testing

- **Without ROMs:** test vectors that freeze both keys; the lookup order;
  `follow` with the reference variant equals the image byte for byte;
  baking through two synthetic ramps, and each band of the distance
  rule; bundle contents from a synthetic catalogue.
- **With ROMs:** the pack log before and after on the showcase pack;
  software and GPU frames agree as closely as they do today.
- **Draw-only:** state hashes with a pack that uses `artworks` equal
  those without.

## Performance

`follow` adds table lookups of the kind the lighting ratio already makes
per pixel; expected to be within noise, not measured. Memory falls with
the number of colour copies, which matters most in the browser's 192 MB
budget. Nothing here was measured in the browser.

## Evidence

Spike code is on unmerged branches; none of it is in reModel. Each
entry says what was observed (measured or read) and what is inferred.

### E1. Colour variants offline (2026-11-19)

Top Skater's catalogue and the showcase pack, at commit `7d8f099`;
offline, on decoded images, without the game's real tables.

Observed:

- All 62 artworks with several colourings differ by polygon colour only.
  None differs by luma table or translucency. They carry 49% of the
  pixels sighted.
- Brightness does not rise with the polygon's colour number. Three
  models that worked a tint out from the colour values predicted the
  game's own variants to within 4 levels on 12 to 15 of the 62 artworks.
- A lookup per texel value is nearly exact on the game's own art: 0.15
  levels mean error over 148 pairs.
- One ratio for the whole image, which is what `studio variants` uses,
  serves 51 of 62 artworks within 4 levels but only 56% of the pixels:
  it fails on the large ground textures, where hue changes with
  brightness.
- On new art the per-texel model carries the original's texel-by-texel
  differences into the replacement, putting tones back into flat cel
  art that the artist took out.
- Of 40 extra colourings in the showcase pack, 18 are the same image as
  their reference, and most of the rest were deliberately redrawn (a
  ground in mint, lilac and burgundy). `fixed` matches 18 of the 40; no
  tint matches more than 2.
- In 17 of 148 pairs the reference colour has a channel at zero, where
  any ratio is undefined.

Inferred: the showcase's authors rarely wanted a tint.

### E2. Baking through ramps (2026-11-20)

Offline; four games' catalogues and the showcase pack; no code change.

Observed:

- One snapshot of the palette and luma tables reproduced 52% to 77% of
  sighted pixels on three games and all of Daytona's, because games
  rewrite the tables as they run.
- On the game's own art, baking through the ramps gave 0.00 mean error
  on Top Skater and `vf2`, 0.20 on Daytona, 2.0 on `fvipers` (0.00 on
  its luma-table pairs).
- Baking from the colouring with the most distinct colours cut Daytona's
  error from 1.06 to 0.20.
- On painted test art the error was 1.3 levels for pixels within 1 level
  of the ramp, 4.5 at 4 to 8, 9 at 8 to 20, and 15.6 beyond.
- In the showcase every authored image is 25 levels or more off the
  original ramp. Of its 40 extra colourings, 20 are the same image as
  the first, where a copy is exact and baking does harm (18 levels); the
  other 20 were redrawn to a new theme, and no method came near them
  (46.8 mean error at best, against 58.3 for a copy).

Not tested: the real tables were not read, so the `follow` model was not
run on a colour the catalogue has never seen. The distance thresholds
come from synthetic art only.

Conflict noted: E1 counts 18 of the 40 as the same image as their
reference, E2 counts 20 as the same as the first. The later count is
E2's; E1's 18 is what `fixed` matched.

### E3. Five games and two more (2026-11-20)

New catalogues of two or three scripted runs each (no pack exists for
any but Top Skater). The two Model 2B games were added from a coin
start and a run with no input each.

| | Top Skater | `vf2` | `daytona` | `gunblade` | `fvipers` |
|---|---|---|---|---|---|
| Board | 2C | 2A | Original | 2B | 2B |
| Image keys over artworks | 565 / 417 | 954 / 693 | 989 / 396 | 1,255 / 648 | 983 / 642 |
| Artworks with several colourings | 62 | 151 | 156 | 222 | 215 |
| Their share of pixels sighted | 49% | 70% | 88% | 41% | 36% |
| Differ by colour only | 62 | 142 | 141 | 222 | 157 |
| Differ by luma table, alone or with colour | 0 | 6 | 15 | 0 | 58 |
| Luma tables in use | 2 | 4 | About 38 | 2 | 4 |
| Largest set of colourings | 12 | 28 | 47 | 37 | 15 |
| Mean error, one ratio for the image | 7.0 | 4.2 | 15.3 | 7.0 | 21.7 |
| Mean error, lookup per texel value | 0.15 | 0.04 | 0.74 | 0.18 | 0.41 |

Observed:

- The lookup per texel value holds on all five. One ratio is worst on
  `fvipers` (21.7 levels), where 58 artworks also change luma table.
- `fvipers` has more luma-table variants than any other game tried,
  covering 19% of its pixels. Daytona uses about 38 tables where Top
  Skater uses 2; one ratio for the image is poor there (10 levels on
  those pairs).
- Two more 2B games agree (`vstriker`, `lastbrnx`). Artworks in several
  colourings carry 79% and 39% of pixels; the lookup per texel value
  errs by 1.2 and 0.0 levels, one ratio by 31.6 and 9.6. `vstriker`'s
  team kits are one artwork in 20 to 28 colourings: the teams differ by
  palette entry alone.
- `gunblade` is the one game where the plain ratio of the two polygon
  colours nearly works (88% of pixels within 4 levels). Its colour index
  is still not a brightness.
- Nearly nine pixels in ten of Daytona's textures are on artworks drawn
  in several colourings.
- Daytona's largest sets are 25 to 47 colourings of flat 32x32
  gradients; `vf2`'s are stage walls and floors in up to 28 colours.

Inferred: colour is an index on every game checked, and the per-texel
lookup is the only model that holds. `gunblade`'s ratio is that game's
palette, not a rule. Artwork identity matters more on `vf2` and
`daytona`. Variant sets can be large and plain, so baking by the build
suits them: nobody should draw 47 swatches.

Not tested: a fighter's second costumes, and text drawn as textures.

## Open questions

- **Which tint for an unseen colour on flat art:** the per-texel lookup,
  which is exact on the original and noisy on cel art, or one ratio for
  the image, which is smooth and wrong on large ground textures?
- **Baking across luma tables.** Daytona has variants that differ by
  luma table. The build's copies need the tone step as well as the
  colour step; neither was run with the game's real tables.
- **Do the build's bands hold on real repaints?** The 4 and 12 level
  thresholds were set on synthetic art.
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
4. Each key's 16-colour ramp recorded in the catalogue.
5. Variants baked by `pack build` through the ramps, or copied, by the
   distance rule, replacing the hand-run `studio variants` step; the
   thresholds checked on a real repaint.
6. `follow` for unseen colours, after trying both tints on a colour held
   out of the catalogue, with the game's real tables read.
