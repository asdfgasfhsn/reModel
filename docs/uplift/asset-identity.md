# Asset identity and context bundles

Shared words, roles and rules: [README.md](README.md). **Partly built
(2026-11-23, roadmap item 5):** artworks in packs (`art_` images, the
colour ramp in the catalogue, `fixed` and `baked`, `studio artworks` and
`studio bake`, the `known` and `selected` gates by identity) are user
documentation now: [../uplift-studio.md](../uplift-studio.md), "Artworks",
and [../texture-packs.md](../texture-packs.md), "Artwork images". What is
left here is not built: names, links, flags, the ROM anchor, `follow`,
palette packs, the rest of the context bundle (below) and `pack rekey`. What was tried is in
"Evidence" (spike code is on unmerged branches, not for merging as it
stands).

### Re-grounded on 2026-11-23

Before item 5 was built, the "Observed" claims were checked against the
code and three catalogues (Top Skater, `vf2`, `daytona`, one `coin-start`
session each). What held: the key is made as described
(`raster::tex_info`: one FNV-1a stream, the surface key its state after
texels and size, the image key after luma table, translucency and colour);
the renderer scales a replacement by lighting and never re-colours it
(`raster::Light`); a bare `surf_` name was refused; the GPU stand-in exists
and the software reader looked up exact names only; the catalogue kept a
key's first sighting only. What moved or was wrong: line numbers (all of
them); `art_` needed a feature name too, not only a file kind (`artworks`
in `pack::FEATURES` and `SHOWN`: a reader refuses a pack that `needs` an
unknown feature, so the spec's "no change at all" held only for `uses`);
counts on one session are smaller but of the same shape (Top Skater 243 keys
over 193 artworks, 30 under several keys carrying 47% of pixels, against
565, 417, 62 and 49% over three sessions; `vf2` 72% against 70%; `daytona`
83% against 88%); a ramp is the first sighting's, not the most used look's,
so a key first seen in a fade records the fade (not yet elected); the
manifest `material` line for `colour=` was not needed, since the reader's
order (own `tex_`, then `art_`) is the policy and `baked` is just files.

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

### Packs can name the artwork (built)

Built as described in [../uplift-studio.md](../uplift-studio.md),
"Artworks", with these differences from the first design: the colour mode is
not a manifest line (a pack with `tex_` copies is `baked`, one with only the
`art_` image `fixed`); the reference colouring is found by measuring which
ramp the image lies nearest, not assumed; baking from the richest ramp is
advice (`paint_over`), not a rule. The thresholds held on the game's own
art and a few synthetic repaints; no real restyle has passed through them.

**`follow` is the fallback for a colour never seen**, in place of
today's untinted stand-in. It is not built: an unseen colouring gets the
`fixed` image. Its model: per channel, scale the image by
what the game's colour table gives for this variant's colour over what
it gives for the reference colour, at the original texel under the
pixel. That is the shape of the lighting ratio the renderer already
computes (`crates/model2/src/raster.rs`, `Light`, and `pack_lit` in
`crates/gpu/src/raster.wgsl`), with the same fallback where the
reference channel is zero (it happens: E1). A lookup per texel value is
nearly exact on the game's own art on all five games tried (E1, E3),
but on new art it carries the original's texel-by-texel differences
into the replacement, putting tones back into flat cel art (E1). It has
not been run on a colour the catalogue has never seen, which is the one
case it is now for: the real tables were not read (E2).

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
  (Built: each key's ramp at its first sighting; `studio artworks` is the
  index over the project's maps. Not built: the most-used look's ramp, the
  placements, and an `artworks.tsv` of the catalogue itself.)
- Elect each artwork's picture by most-used look, as the dump does.
- Store the upload's ROM address per artwork, and the flags above.

### The context bundle: what is left

`asset bundle` and `asset show` are built (2026-11-23,
[../uplift-studio.md](../uplift-studio.md), "Context bundles"): identity,
colourings and ramps, size, wrap, where it shows in the map's frames,
status and gates, the original, a mask and two crops. Not built:

| File or field | Holds |
|---|---|
| `variants.png` | Every colouring, labelled |
| `tiled.png` | A 3x3 repeat, when the game repeats or mirrors it |
| `brief.txt` | The prompt text for an image model, filled from the bundle |
| `scenes[]` | The scenes it shows in with boxes in the game's 496x384 (needs `scene where`); the built bundle gives the texture map's frames |
| flags, partners, a name | `shared` is derived; the rest wait for names, links and the ROM anchor |
| the style rules for its category | The built bundle points at `style.md` |
| a mask over all scenes | The built mask is over the map's frames |

## Interfaces

| Verb | Does |
|---|---|
| `asset list` | Filter by `--kind`, `--scene`, `--status`, `--flag`, `--category`; ranked by screen share |
| `asset name ID SLUG` | Names an artwork in the project |
| `asset same ID ID` | Links two artworks; `--suggest` lists candidates with a difference score |
| `pack rekey` | Rewrites names after a scheme change |

An ID is accepted as a slug, an `art:` key, or today's `tex_` name, which
resolves to its artwork.

## The roles

- **Pack lead.** Says "replace the wall base" and means one row, not six
  hashes. `asset list --flag shared` shows where judgement is needed.
- **Art agent.** `asset bundle wall-base` is its whole briefing (built). It writes
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
  thresholds held on the game's own art, a 2x upscale and two edits of it
  (2.2, 2.7 and 11.6 levels); no real restyle has been through them.
- **Is the ROM address safe to keep in shared files?** It is an offset, as
  a key is a hash; it is only needed locally.
- **How different may two artworks be and still be offered as `same-as`?**
  Needs real near-duplicates to set.
- **Catalogue size.** Recording every placement grows it; by how much is
  not known.

## MVP: what is left

Built on 2026-11-23: `art_` images with `fixed` and `baked`, in the CLI's
renderers and the player (not the browser); each key's ramp in the
catalogue and texture maps; baking and copying by the distance rule
(`studio bake`), which replaces the hand-run `studio variants` where ramps
exist; an artwork list (`studio artworks`). The remaining steps:

1. `asset list` and `asset show` over `artworks.tsv`, with names (item 6).
2. (Built: bundle v0.)
3. The ramp elected by most-used look, and `follow` for unseen colours,
   after trying both tints on a colour held out of the catalogue, with the
   game's real tables read.
4. `art_` in the browser (its pack index and fetch).
