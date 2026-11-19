# 2D at cell level: tiles, regions and text

Shared words, roles and rules: [README.md](README.md). Nothing here is
built. What exists is in [../texture-packs.md](../texture-packs.md)
("Regions") and [../hw/tilemap.md](../hw/tilemap.md).

## The problem

A game's HUD, logos, menus and text are drawn from 8x8 tiles on four
64x64 maps. A pack can replace a tile by its pattern and colours, or a
whole arrangement with a region. Both were pushed to their limits by the
showcase.

**Observed in the code.**

- **A tile's key includes its colours**: the 64 pens, then the colour of
  each pen used (`crates/model2/src/video.rs:428-448`). It does not
  include the tile number, the palette bank or the priority.
- **A fade makes new keys.** The docs say so: a palette fade "makes new
  keys and the region stops matching until the colours come back"
  (`docs/texture-packs.md:469-472`).
- **A region is whole cells.** It matches tile keys per map cell
  (`crates/model2/src/regions.rs:387-443`) and draws per map cell
  (`crates/model2/src/video.rs:751-752`), so its edges are always on 8
  pixel boundaries. Cells outside it are untouched.
- **The browser does not draw regions**: its list of shown features is
  `materials` alone (`crates/model2/src/pack.rs:40`).

**Observed in Top Skater's catalogue.**

| Fact | Number |
|---|---|
| Tile keys | 24,186 |
| Tile numbers | 7,781 |
| Colour-free patterns | About 10,300 |
| Keys seen in exactly one sample | 31% |
| Most keys for one tile and bank | 32 (palette states of a fade) |
| Keys for one solid-fill pattern | 600 |
| Uses of one fill tile inside the showcase's six sponsor regions | 575 |

So most tile keys are the same picture in another colour, and some tiles
are shared by unrelated things.

**What it cost in the showcase.** The publisher's boot logo could only be
blanked, because it is built from fill tiles shared across the screen. A
replacement credit left blue fragments in "INSERT COIN(S)", which uses the
same glyph tiles. Sponsor logos broke where the old ones used shared
fills. Cutting a region left neighbouring fragments to patch by hand, with
"no option to clear tiles around a region". The score digits were never
restyled: "each tile needs mapping to a (digit, quarter) pair". The clock's
digits were inferred (31 rows, confidence 0.90); the score's were not,
because the score never changed in the run. The pack's 5,970 tile images
exceed the 4,096 cells of the tile atlas, so tiles were decoded again and
again.

Other Model 2 games add needs Top Skater does not have: Japanese HUD text,
light-gun crosshairs, and skies and backgrounds drawn on the tile layer
(from public descriptions of the games; unverified).

No emulator surveyed replaces composed 2D screens. The nearest, DuckStation,
names a source upload, a sub-rectangle and a palette range, and still needs
tuning per game. This is new design work.

## Design

Four steps, each useful alone.

### 1. Patterns, not colourings

Give each tile a colour-free identity, `pat:<hash of the 64 pens>`, with
its colourings as variants. This mirrors artwork and variant for 3D
([asset-identity.md](asset-identity.md)).

- A pack image may be named by pattern, `pat_<hash>.png`. By default it is
  `fixed`: drawn the same in every colouring. That is what the renderer's
  "hold" repair does today for a tile whose colours changed
  (`crates/gpu/src/tiles.rs:245`, `337-341`), made deliberate.
- Regions match cells by pattern unless told `colours=exact`.

Effect: a fade no longer drops a replacement, the showcase's tile count
falls towards the number of patterns, and the atlas stops overflowing.
The option noted in `docs/enhancements.md:412-413` (key on the pattern and
colour an indexed image) is the stricter form of this and stays open: see
the questions.

### 2. Regions that fit

- **Anchors.** A region names the few cells that make it distinctive as
  its anchor and treats the rest as "whatever is there". Content that
  changes inside it, such as digits, no longer breaks the match. Capturing
  from two scenes marks the differing cells automatically, as `--also`
  does today.
- **Clearing.** `clear=auto` blanks the cells around the region that
  belonged to the same arrangement when it was captured. The catalogue
  already finds an arrangement as a connected group of tiles
  (`crates/model2/src/catalogue.rs:264-333`); the cells of that group the
  region does not cover are the fragments to clear.
- **Pixel placement.** The image may sit at any pixel rectangle inside the
  cells it covers. The rest of each partly covered cell is drawn with a
  stated fill: a colour, transparent, or the original. Drawing stays one
  slice per cell, so the shader does not change.

A region already wins over a tile image inside its cells
(`crates/model2/src/video.rs:541-543`), so a shared fill tile is replaced
only where the region is. With anchors and clearing, the boot logo, the
credit and the sponsor boards need no hand patches.

### 3. Text fields

A **field** is a run of cells whose content is text the game writes:
a score, a clock, a name. The pack supplies a font and the renderer
writes the text.

- `glyphs.txt` maps a pattern and quarter to a character. The catalogue's
  inference proposes it; a person confirms it. Unconfirmed rows are not
  used.
- Each frame, the renderer reads the field's cells from tile RAM, decodes
  them to a string, and draws that string into the field's rectangle with
  the pack's font atlas, aligned as the field says.
- If any cell is not in the map, the whole field draws the original tiles.
  Never half.

This is draw-side: it reads what the tile layer already reads. It is the
general form of the plan in `docs/mods.md` to draw song titles over the
game's title bar, and it serves Japanese HUDs the same way. Labels that do
not change ("SCORE", "TOTAL TIME") are ordinary regions.

A simpler first form needs no renderer work: where every pattern in a font
is one quarter of one glyph, as the clock's 31 rows are, the build step
cuts a glyph image into those tiles. It fails for shared tiles, which is
what the field is for.

### 4. The browser, later

Regions and fields use the tile atlas the browser build already shares.
What is missing is fetching `regions.txt` and listing `regions` as shown.
Browser work is out of scope for now (decided 2026-11-19). Until it is
done, `pack check` warns when a pack relies on regions, so a HUD pack
does not fail there silently.

## Interfaces

| Verb | Does |
|---|---|
| `region capture SCENE X,Y,W,H --name N` | Cuts a region from a scene, with anchors and `clear=auto`; `--also SCENE` marks what varies |
| `region list`, `region check` | What matches where, on which scenes; warns when a region matches nowhere or twice |
| `asset show PATTERN` | For a pattern: every arrangement and scene that uses it, which is how shared it is |
| `glyphs infer`, `glyphs show`, `glyphs confirm` | Propose, show as a sheet, and accept a glyph map |
| `region field REGION X,Y,W,H --glyphs NAME --font NAME` | Declares a text field inside a region |

`scene what` returns, for a 2D point, the map, cell, pattern, colouring,
the arrangement it belongs to and the region or field that claims it.

A region is an asset of kind `layout`; its slug is its name. In the
project it is `regions/<name>.txt` and `art/<name>.png`. Coordinates are
in the game's 496x384 picture.

`regions.txt` grows three keys per region and a `field` line:

```
region sponsor-board 21x10 cells=... anchor=0,0-20,1 clear=auto rect=4,6,160,68 fill=transparent
field  score in=hud-top at=40,2 cells=8x2 glyphs=digits font=comic align=right
```

`glyphs.txt` has one confirmed row per pattern and quarter:

```
glyphs digits
<pattern>  7  tl
<pattern>  7  tr
```

No new `format` number is needed: the features are named `patterns` and
`fields`, and a pack that depends on them says `needs`
(`docs/texture-packs.md:273-285`).

## The roles

- **Art agent.** Is handed "the sponsor board on the character select".
  `region capture` from the scene gives it a size and an original. It
  paints one image at any size. Shared tiles are not its problem. Budget:
  2 calls and 1 picture per region.
- **Pack lead.** `asset show` on a pattern shows how shared it is before
  anyone paints it. `scene sheet --label layouts` is how they name a
  region for an agent. Confirming a glyph map is a judgement, and theirs.
- **Artist.** Paints a HUD as a few whole pictures and one font sheet, not
  as thousands of 8x8 squares.
- **Tester.** `region check` across every scene finds a region that stops
  matching.
- **Player.** Text stays live: the score shown is the game's score.

## Testing

- **Without ROMs:** pattern keys frozen by test vectors; anchors, clearing
  and pixel placement on synthetic maps (the region tests are the model:
  scrolled, half off screen, occluded); field decoding with a synthetic
  glyph map, including the fall-back; the software and GPU maps agree.
- **With ROMs:** the showcase's 13 regions redone with anchors and
  clearing, with no hand-patched tiles; a pack log showing no region
  misses through the title fade.

## Performance

Matching already tries every anchor on four maps each frame. Pattern keys
cost the same to compute as today's. A field decodes a few dozen cells a
frame. Fewer tile images means fewer atlas evictions; by how much is to be
measured. The browser shares this code, and fewer images helps its 192 MB
budget most; nothing was measured there.

## Open questions

- **Should a fixed image follow a fade?** Drawn `fixed`, a restyled logo
  stays at full strength while the game fades the screen around it.
  Scaling it by the palette's change would follow a fade to black; a fade
  to white needs something else. Indexed images would follow exactly and
  cannot hold smooth art.
- **How reliable is glyph inference on other games**, and on scores that
  rarely change? Cheats that drive counters would help, and are planned
  separately.
- **Sprites on the tile layer** (a sky, a crosshair) may need regions that
  follow line scroll per row; untested.
- **Wide views** add side columns that keep the original tiles today.
  Should a region extend into them?

## MVP: a few days

1. Pattern keys: `pat_` images drawn `fixed`, and regions matching by
   pattern.
2. Anchors and `clear=auto` in `region capture`.
3. Build-step glyph tiles for one clock, from a confirmed `glyphs.txt`.
4. **Spike:** a text field for Top Skater's score, and one Japanese HUD
   label on a second game.
