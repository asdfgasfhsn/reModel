# Texture level of detail: mipmaps, the log table and microtextures

What the games' own data and raster streams show about the rasteriser's
level-of-detail inputs. reModel draws none of this yet: every texture is
drawn from level 0, and the microtexture bit is ignored. Written
2026-11-11 from runs of all 18 games (attract and coin-start, 900M
instructions each, every 20th frame's polygons examined); no other
project's code or documentation was consulted. The rasteriser is fixed
hardware with no program to read, so what it *does* with these inputs is
marked **unconfirmed** throughout; what the games *send* is confirmed.

## Summary

| Input | Where | State |
|---|---|---|
| Mip levels in texture RAM | level 1 at (1024 + x/2, 512 + y/2) in the other bank, level 2 at (1536 + x/4, 768 + y/4) in the same bank | **confirmed** from texture RAM, 16 games |
| Log table | 32,768 bytes, raster command 4 with address bit 23 clear (list fn 0x14), written once at boot | **confirmed**: it is `128·log2(1 + i/32768)` or a straight-line stand-in for it |
| Per-polygon distance word | stream word 2 of each polygon: `coef·LOD·|n·Pa|` | **confirmed** (the programs, `geometrizer-2b.md`, `geometrizer-2c.md`) |
| Level = `2·log2(z) − log2(distance)` + a constant | per pixel, from the pixel's depth z | the shape is **confirmed** as what the games author for; the constant and the rounding are **unconfirmed** |
| Microtexture enable | header word 0 bit 12 | **confirmed** as a per-texture flag naming a second texture; 4 games use it |
| Microtexture location | header word 2 bit 13 (x/128), bits 14-15 (y/128), in the bank the base texture is *not* in | **confirmed** by content |
| Header word 0 bits 10-11 | 0, 1 or 2 on microtextured polygons; 1 on some plain ones in three games | meaning **unconfirmed** |
| How base and microtexture combine | — | **unconfirmed**; candidates below |

## Mipmaps

Compared in texture RAM, for the textures each game draws around frame
4,500 of a coin-start run: the texture's level 0 averaged down 2x2 against
the rectangle where level 1 would be (correlation above 0.8 counts as a
match).

- Level 1 of a texture at (x, y) in bank b is at (1024 + x/2, 512 + y/2)
  in bank b ^ 1; level 2 at (1536 + x/4, 768 + y/4) in bank b. The same
  rectangle in the *same* bank never matches for level 1 (0 of about 1,000
  textures). So each bank's lower right quadrant holds the other bank's
  half-size copy, with the quarter-size copy of its own nested in that
  quadrant's lower right, and so on (**confirmed**; `geometrizer.md` §5.3
  had this as an unchecked claim).
- **16 of the 18 games upload mip chains**: `bel`, `dynabb97`, `fvipers`,
  `hotd`, `indy500`, `lastbrnx`, `overrev`, `overrevb`, `schamp`,
  `segawski`, `sgt24h`, `skisuprg`, `srallyc`, `stcc`, `vstriker`,
  `waverunr` (all or nearly all textures checked match at levels 1 and 2;
  `bel` about 80% at level 1).
- **`topskatr` and `doa` upload none** (0 of 70-80 textures). Both send
  distances that keep the level far below 0 everywhere (see the offsets
  below), so they never ask for a level above 0.
- A microtextured texture's base may have no usable chain: `segawski`'s
  and `waverunr`'s water (256x256) doesn't match at level 1.
- reModel draws level 0 only, so distant textures in the 16 games shimmer
  where the hardware would read a smaller level. How the hardware filters
  between levels is **unconfirmed**.

## The log table

Every game sends list fn 0x14 at boot: four blocks of 8,192 words at
addresses 0, 0x2000, 0x4000, 0x6000 (some send a block twice), which the
geometrizer passes on as raster command 4 with one byte per word. Nothing
writes it again. The 32,768 bytes, by game:

| Table | Games |
|---|---|
| exactly `round(128·log2(1 + i/32768))` (0 to 128) | `fvipers`, `schamp` (and `vf2`) |
| exactly `floor(128·log2(1 + i/32768))` (0 to 127) | `lastbrnx` |
| a straight line of slope 1/258 (0 to 127; within 1 of `i >> 8`) | `dynabb97`, `hotd`, `indy500`, `overrev`, `overrevb`, `segawski`, `sgt24h`, `skisuprg`, `srallyc`, `stcc`, `topskatr`, `vstriker`, `waverunr` |
| the straight line with dips (not monotonic, within 3 of `i >> 8`) | `doa` |
| its own rising curve, 0 to 146 | `bel` |

So the table's index is the 15 mantissa bits of a 24-bit stream float and
its value the fraction of that float's base-2 logarithm in 1/128ths
(**confirmed** by the three exact tables; most games settle for the
chord). With the exponent that gives `log2` of a stream float to 7
fraction bits, which is what a level with a 7-bit blend fraction needs.
Which stream values the rasteriser looks up (the distance word, the depth,
both) is **unconfirmed**.

## The distance word and the level

The geometrizer programs send, per polygon, `distance = coef·LOD·|n·Pa|`:
the polygon's distance coefficient (attribute bits 27-31 pick one of 32,
set by fn 0x06), the frame's LOD (fn 0x16) and the eye's perpendicular
distance to the polygon's plane (`geometrizer-2b.md`, `geometrizer-2c.md`;
a 2C single-sided back face sends `coef·LOD` alone). Nothing per vertex,
and nothing else per polygon, carries level-of-detail information.

For a flat polygon at perpendicular distance d, texels per pixel along the
steepest direction are `T·z²/(f·d)` (T texels per unit, f the focal
length, z the pixel's depth). So `2·log2(z) − log2(distance)` is that
density's logarithm less a per-surface constant, **if** the game authors
its coefficients that way. Measured, with ρ the texel density at each
polygon's on-screen centroid (largest singular value of the UV Jacobian)
and `offset = log2(ρ) − (2·log2(z) − log2(distance))`:

- Within one texture in one frame the offset varies far less than the
  density does: median standard deviation 0.18-0.45 levels against
  0.37-1.15 for `log2(ρ)`, in every game (groups of 8 or more polygons of
  50 pixels or more). A single surface holds it to a few hundredths over
  two octaves of depth (`hotd`'s microtextured floors: 2.23 to 2.35).
  **Confirmed**: the distance word is authored as the per-polygon half of
  `2·log2(z) − log2(distance)`.
- The offset itself is the game's choice, per polygon and per scene:

  | Game | LOD values seen | Common offsets |
  |---|---|---|
  | `fvipers`, `schamp` | 2560; 1493.33 | 2.5-3.0 (70-80% of polygons) |
  | `vstriker` | 128 | 1.5-3.0 |
  | `dynabb97`, `indy500` | 8 (10000 once) | 4.0-5.0 |
  | `overrev`, `overrevb` | 15 in races, 33-39 changing by shot in attract, 8000 once | 3.5-4.0, and 10-10.5 |
  | `bel` | 128 | 2.5-4.0 |
  | `hotd` | 4 | 5.0-6.0; microtextured surfaces 2.2-2.4 |
  | `segawski` | 0.25, 0.5, 0.05, 0.021-0.023 | 1.5-7; water 1.0-1.7 |
  | `waverunr` | 0.25 | 2.0-3.0; water 2.0-2.6 |
  | `srallyc` | 200, 1024, 400000 | 3-4 and 7-8 |
  | `sgt24h`, `skisuprg`, `lastbrnx` | 4; 20, 50; 128, 256, 1024, 65536 | 7-10 and others |
  | `stcc` | 0.5, 10, 1e7, 1e8 | 33 (and 9.5-10) |
  | `doa` (no mips) | 40, 1000 | 12-13.5 |
  | `topskatr` (no mips) | 65536 | 11-13; microtextured surfaces 5-8 |

  The two games without mip chains sit 8 or more levels below the rest,
  which fits a rasteriser that reads level 0 while the value is negative.
  `topskatr` gives its microtextured polygons coefficients about 2^6
  smaller than its other polygons': they are authored, not left at a
  default.
- The LOD value is a per-scene constant in most games; `overrev`'s and
  `segawski`'s attract modes change it from shot to shot. Whether that
  follows the focal length is **unconfirmed** (not measured).
- **Unconfirmed**: the constant between this value and the level the
  hardware picks, i.e. at what texel density level 0 gives way to level 1
  and to the microtexture. If the value were used as it stands, `fvipers`
  would leave level 0 only above 6 texels a pixel. The games with mip
  chains cluster at offsets of 2.5 to 4.5, so a constant of that order (or
  a deliberate sharpening by the games) is involved. It cannot be read
  from the stream.

## Microtextures

**Who uses the bit** (header word 0 bit 12), in 310 sampled frames a run:

| Game | Frames with any | Polygons a frame (median, max) | Share of the screen (median, max) | On |
|---|---|---|---|---|
| `segawski` | 56-71% | 208-237, 355 | 0.49-0.67, 1.0 | all water |
| `waverunr` | 68-76% | 202-232, 361 | 0.8, all | all water |
| `topskatr` | 50-60% | 41-54, 199 | 0.25-0.46, all | floors, roads, grass, walls, ramps |
| `hotd` | 2% in play, 31% in attract | 1-7, 18 | 0.2-0.76, 0.97 | floors and walls in close-ups |
| the other 14 | none | | | |

By screen area the microtextured polygons are the near ones: their median
depth is 1.3 to 13 times nearer than the other textured polygons', and 41%
(`waverunr`) to 93% (`hotd`) of their area is magnified (under one base
texel a pixel).

**Where the microtexture is** (**confirmed** by content). Header word 2
bit 13 and bits 14-15 give x/128 and y/128 of a 128x128 texture in the
bank the base texture is not in (header word 2 bit 12 is the base's bank).
The eight slots are the 256x512 rectangle at the top left of that bank.
What the games keep there:

- `topskatr` (bank 0): wood planks, bricks, asphalt, a sand relief,
  studded metal, two grasses, a tile grid. Its polygons point at the
  matching one: green grass polygons at the grass, a brown wall at the
  bricks, the sand floor at the sand relief, the street at the asphalt.
- `hotd` (bank 0, per stage): eight ornamental floor and wall tiles
  (reliefs, stars, medallions).
- `segawski` and `waverunr` (bank 1): eight frames of the same water
  ripple. All water polygons of a frame name one frame, and the game
  steps through the eight in order, one about every four game frames:
  **the microtexture animates the water** (**confirmed** from the
  headers; what it looks like on screen depends on the unconfirmed
  blend).

The same rectangle in the base's own bank holds unrelated textures, so
"the other bank" is right, not a fixed bank (`topskatr` and `hotd` have
their bases in bank 1, the water games in bank 0).

**What they are.** Whole materials in ordinary texel values, not maps
centred on a mid value: means run from 2.1 to 11.8 of 15 (`topskatr`'s
bricks 2.1 with 30% of texels 0, its asphalt 3.1 in 0-7, the water 2.4 in
0-8 beside a base of 3.1 in 0-9, a `hotd` tile 11.8). A base and its
microtexture often have similar means (water 3.1 and 2.4, `topskatr`'s
wood 8.4 and 8.5) but not always (`hotd`: 13.5 and 5.9).

**Other header bits.**

- Word 0 bits 10-11 on microtextured polygons: 0 (both water games;
  some of `topskatr`'s), 1 or 2 (`hotd`, `topskatr`), with no fixed
  relation to the base texture's size (64 to 256 texels). Also 1 on plain textured polygons in `bel`,
  `fvipers` and `vstriker`, whose textures have ordinary mip chains.
  `geometrizer.md` §5.3 called it a microtexture minimum level;
  **unconfirmed**, and the plain polygons show it isn't only that.
- Word 2 bit 11 is set on every textured polygon of every game: not a
  y bit, not a microtexture bit. Purpose unknown.
- Word 1's high byte is non-zero on some polygons of `bel`, `fvipers` and
  `vstriker`; word 3's low 6 bits are always 0.

**The level on microtextured polygons.** With the value
`2·log2(z) − log2(distance)` as it stands, 61-69% of the water games'
microtextured area, and 99% of `hotd`'s and `topskatr`'s, is below −1;
`topskatr`'s median is −9. A microtexture that takes over one level below
0 would therefore hide `topskatr`'s base textures almost everywhere they
are drawn, although the game authored those polygons' coefficients
separately. Either the hardware's constant differs, the blend never
reaches the microtexture alone, or it fades in over several levels:
**unconfirmed**.

**The blend: unconfirmed.** Candidates, mocked up in the software
rasteriser on `topskatr`, `hotd`, `segawski` and `waverunr` frames (a
throwaway probe, not in the tree) and looked at:

1. *Linear blend of the two filtered texels*, weight rising as the level
   falls below 0, then through the base's luma table and colour as usual.
   Plausible on all four games whether the weight stops at a half or
   reaches the microtexture alone: `hotd`'s close-up floors and walls gain
   their tile reliefs, the water its ripples, `topskatr`'s flat floors a
   wood or sand grain, and its magnified black stain (`topskatr`'s
   attract, about 75 s in: texel 0 of a floor texture, black as sent,
   `docs/games/topskatr.md`) becomes asphalt. The best candidate; the
   weight's ceiling and the fade's width are open.
2. *Base plus the microtexture's deviation from its own mean.* Keeps
   brightness, but leaves the base's black areas black with holes (`topskatr`),
   and the hardware has no mean to subtract.
3. *Signed or multiplicative detail around a fixed mid value* (7.5).
   Ruled out as far as pictures can: the water and `topskatr`'s near
   ground go dark or black, because these textures aren't centred there.

How many microtexels span a base texel is also **unconfirmed** (2 looked
right; 1, 2 or 4 by word 0 bits 10-11 looked no worse).

**What would settle it**: footage or photos of the hardware (not an
emulator) showing

- `segawski` or `waverunr`, the water just in front of the player: does a
  ripple pattern change about 14 times a second there (a cycle of eight,
  about half a second) and not further out
  (how far out it reaches gives the constant; whether the static pattern
  shows through gives the weight's ceiling);
- `topskatr`'s attract, the street course with the camera low by the
  wall, about 75 s after power-on: is the road in front of the camera
  black (no microtexture drawn), asphalt grey, or grey with a darker
  stain; and whether the wall beside it shows bricks;
- `topskatr`'s stadium floor in the attract close-ups of the skaters
  (about 23 s): flat cream, or wood grain;
- `hotd`'s attract close-ups of a zombie's feet and of the wall behind
  the agent (about 28 s and 42 s): embossed tiles or a blurred stone
  texture, and how far from the camera the tiles stop.
