# Renderer controls for packs

Shared words, roles and rules: [README.md](README.md). Nothing here is
built. What exists is in [../texture-packs.md](../texture-packs.md)
("Looks") and [../enhancements.md](../enhancements.md).

## The problem

The renderer shapes a replacement in ways its author cannot see or steer.
The showcase agents learned the rules by trial and error.

**Observed in the code and docs.**

- **A replacement is always re-lit.** Its colour is scaled per channel by
  what the game's tables give at the polygon's brightness over what they
  give at full brightness; it is drawn as painted only where the polygon
  is at full brightness (`pack_lit`, `crates/gpu/src/raster.wgsl:296-312`;
  `crates/model2/src/raster.rs:1609-1630`).
- **The one way out is a look tag.** `emissive` means "full brightness,
  ignores the game's lighting" (`docs/texture-packs.md:1224`): the
  renderer draws such a polygon at brightness 255
  (`crates/gpu/src/lib.rs:2368`), where the ratio leaves a replacement
  untouched. It
  lives in `look.txt`, so a pack with no look cannot use it, and
  `pack-lint` does not treat `look.txt` as a pack file. The pack manifest
  reserves a `selflit` key on its `material` line and does not implement
  it (`docs/texture-packs.md:265`).
- **Tags are per texture, one line each.** They name an image or a surface
  (`docs/texture-packs.md:1218-1224`). Nothing names a category.
- **The look's ID target knows textures only.** A pixel carries a 23-bit
  fold of the texture key, or the colour for an untextured polygon
  (`crates/gpu/src/look.rs:43-50`). Which model a pixel belongs to is on
  the CPU side only.
- **The browser draws no look.** The passes are shared WGSL and would run,
  but the web build exposes no look setting or file
  (`docs/enhancements.md`, "Looks").
- **Per-game renderer data has a home.** `romset::Shading` gives each game
  a crease angle and exclusions, read generically
  (`crates/romset/src/lib.rs:380-392`).

**What it cost in the showcase.**

- The first comic defaults fitted no game: an ink weight of 2.25 "read as
  invisible at 1080p", and a fade distance in fixed game units when Top
  Skater's typical distance is 45 and other games' is about 1. Both were
  reworked.
- Before tags were tuned "the skater was covered in black halftone dots".
  On another game "the sky goes dark blue with dots".
- Which surfaces need `outline=none` or `toon=off` (crowd, sky, banners,
  flat HUD pieces) was found one render at a time.
- Tools disagreed: one look render "draws the character select without
  the 2D tile and region overrides".

## Design

### 1. Draw rules in the pack

A pack states, per asset or per category, how the renderer treats it.
These are not look settings: they hold whether a look is on or not.

They go on the manifest's existing per-texture `material` line, using the
key it already reserves:

```
material art_<key>_128x128  colour=follow          # re-lit, the default
material art_<key>_256x128  selflit                # drawn exactly as painted
material @hud-3d            selflit filter=nearest
```

| Key | Values | Meaning |
|---|---|---|
| `selflit` | present or absent | Drawn as painted at any brightness; absent, re-lit by the game's ratio |
| `colour` | `baked`, `fixed`, `follow` | How one image meets the colour variants ([asset-identity.md](asset-identity.md)) |
| `filter` | `linear`, `nearest` | Sampling for this asset; pixel art wants `nearest` |

`@name` is a category from the project's `assets.tsv`, written into the
pack at build time as a list of assets, so the loader needs no project.
Nobody writes these lines by hand: `asset set ID draw=selflit` records
the rule, and `pack build` writes the manifest.

Look tags gain the same `@category` form: `tag @banner outline=none
toon=off`. One line replaces a dozen.

### 2. Views that show what the renderer is doing

`scene render --overlay NAME`, and the same in the player:

| Overlay | Shows |
|---|---|
| `ids` | Each artwork in its own flat colour, with a legend |
| `variants` | Colour variants of one artwork, to see where each is used |
| `miss` | Replaced, original, and stand-in, coloured by the pack log's reason |
| `tags` | Outline, toon and `selflit` classes as colours: what a tag line did |
| `light` | The game's lighting layer alone, without textures |
| `order` | Polygons drawn in the game's order without depth (skies, ground quads), where ink behaves differently |

Each overlay also returns numbers with `--json`: the share of pixels per
class. An agent can confirm a tag took effect without opening a picture.

### 3. Calibrating a look per game

Two tools replace trial and error.

- **`look calibrate`** draws a game's scenes with a look and reports: the
  share of pixels inked, the share of lit pixels in each toon band, the
  halftone's share, and ink on assets tagged `outline=none` (which should
  be zero). It proposes parameter values that bring those inside target
  ranges. The values it settles on can be stored per game as a
  recommended look, in the manifest beside `Shading`, so a look starts
  from something that fits the game.
- **`look suggest-tags`** proposes tags from what the renderer knows:
  polygons in HUD windows, skies and other painter's-order polygons,
  single flat quads that face the camera (signs and banners), and textures
  dense with edges (text). It proposes; the pack lead confirms.

Both are guesses about what makes a look read well. The target ranges
come from the showcase's tuned look and need a second game to check them.

### 4. One draw path for every tool

`look-render`, `studio compare`, `pack-check` and the player each compose
a frame their own way. `scene render` becomes the one path: 3D pack,
tiles, regions, fields, look, in the player's order. The older tools call
it. A picture from a tool is then the picture the player shows.

### 5. Model identity in the ID target

Specified in [scene-understanding.md](scene-understanding.md). Adding
each polygon's model, window and flags to the ID target lets
overlays, picking and tags work per model as well as per texture. The
data exists per polygon on the CPU. It can ride in the buffer that
already carries ID words. The renderer runs under wgpu's downlevel
limits, which allow four storage buffers per stage, so a new buffer is
not free (inferred; the shader's bindings were not counted). It is listed here
because three features above improve once it lands.

### 6. The browser, later

Out of scope for now (decided 2026-11-19). When it is wanted: expose the
look in the web build: a `set_look` beside `set_pack`, a
fetched `look.txt`, and a row in the settings panel. Draw rules are read
by the shared pack code, but the web build must list them as shown, and
`filter=nearest` needs a second sampler there as it does natively.

## Interfaces

| Where | What |
|---|---|
| `pack.txt` | `material` lines with `selflit`, `colour=`, `filter=`; categories as asset lists |
| `look.txt` | `tag @category ...` |
| `scene render --overlay NAME` | The views above, with class shares in `--json` |
| `look calibrate [--scenes ...]` | The report and proposed values |
| `look suggest-tags` | Proposed tag lines, as TSV with the reason for each |
| `look set KEY=VALUE` | Live in the player and the server (the CLI has `--look-set` today) |

## The roles

- **Art agent.** Paints a logo and marks it `selflit`. The colours on
  screen are the colours it painted. `--overlay miss --json` tells it the
  file was used. Budget: no pictures for this.
- **Pack lead.** Reads `look suggest-tags`, accepts most, and writes five
  category tags, not sixty texture tags.
- **Artist.** Presses a key in the player and sees which surfaces are
  inked, which are replaced, and which are not theirs yet.
- **Tool engineer.** A new effect adds its class to `tags` and its numbers
  to `look calibrate`.
- **Tester.** Off is exact: with no draw rules and no look, frames are
  byte for byte today's.
- **Player.** A game's recommended look works without tuning.

## Testing

- **Without ROMs:** `selflit` gives the replacement's exact pixel at any
  polygon brightness, in both renderers; `@category` expands as the asset
  list says; overlays are deterministic; with nothing set, the GPU tests'
  frames are unchanged.
- **With ROMs:** state hashes equal with every rule and overlay on;
  `look calibrate` numbers recorded per game in its `docs/games/` page.

## Performance

`selflit` removes work per pixel. Overlays reuse the ID pass, which
costs 0.7 ms at 1080p and 3.0 ms at 4K as documented in
`docs/enhancements.md` (not re-measured), and run only when asked for.
`filter=nearest` needs a second sampler, not a second pass. The browser
gets the same costs through the shared renderer; none were measured
there, and overlays are a tool, not something a player's frame pays for.

## Open questions

- **`selflit` and `emissive` are the same switch** in two places. Should
  the look tag remain once the pack has it?
- **`selflit` beside lit neighbours** may look pasted on. Is a middle
  setting wanted, such as lit but never darker than a floor?
- **Stipple.** The games fake transparency with a checkerboard. A real
  blend exists only under temporal anti-aliasing
  (`docs/enhancements.md`, "The checkerboard"). A per-asset
  `stipple=blend` with a chosen opacity is a likely request for fighters'
  shadows and gun-game effects; it is specified in [lighting.md](lighting.md).
- **Target ranges** for calibration are unproven beyond one pack.

## MVP: a few days

1. `selflit` on the `material` line, in both renderers.
2. `scene render --overlay ids`, `miss` and `tags`, with class shares.
3. `@category` in tags and `material` lines.
4. **Spike:** `look calibrate` numbers for the comic look on four games'
   scenes, to see whether one set of target ranges fits them.
