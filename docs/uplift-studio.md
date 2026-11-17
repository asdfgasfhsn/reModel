# The uplift studio

Tooling, usable by agents and by people, for restyling a game's art through
texture packs ([texture-packs.md](texture-packs.md)). Nothing in it is
specific to one game: it works on what every game's renderer already has
(captured frames, texture keys, tile keys, the pack format). The first
target is a showcase restyle of one Top Skater course, its UI tiles and its
characters; the studio is built so the next game starts from the same
commands.

## The problem

A game has thousands of small textures (32-256 px), and many are pieces of
a larger picture: one ramp face, half a billboard, a skater's UV layout.
Whoever restyles them, an artist or an agent, has to know **which texture
is where on screen** and **which ones belong together**, or the halves of a
billboard come back in two styles. The names in a dump (`tex_<hash>_WxH`)
say neither.

## Workflow

1. **Concept.** One page: the style sheet (palette, materials, mood, what
   stays recognisable, what must not change such as readability of signs
   and the HUD) and a few reference images. Written by hand or drafted by an
   agent; the point is that every later step reads the same sheet.
2. **Map.** `make texmap` (built): captured frames of the scene drawn with
   an ID image, an inventory and groups ([texture-packs.md](texture-packs.md),
   "Texture map"). Answers where, how big and what goes together.
3. **Select.** From the inventory pick what the showcase restyles:
   biggest coverage in the chosen frames first, then the rest of each
   group (a group is restyled as a unit). The selection is a list in the
   project, each entry with a status.
4. **Restyle.** For each selected texture (or group) produce a replacement
   PNG under the same name: by hand, by an image model (Codex CLI image
   generation), or by the upscaler (`make upscale-textures`) for a faithful
   upscale as the base. Each image records its provenance.
5. **Check.** `make pack-lint` for the format; `make pack-check` and
   `tile-check` for how the pack draws on the GPU and in software; a before
   and after of the same frames (the texmap frames drawn with and without
   the pack) to compare in the viewer.
6. **Show.** The pack in the player, a poster set, a short capture.
   Approved images become the pack that ships.

## Workspace on disk

Everything of a project lives under `out/<game>/studio/<project>/` (git
ignored: it is ROM-derived). `remodel studio init` makes it:

```
studio/<project>/
  project.txt         name, game, created (key, tab, value)
  style.md            the style sheet (step 1)
  selection.tsv       name, kind, group, surface, pixels, category, reason (step 3)
  recipes.txt         recipes of the local style processor (below)
  briefs/             briefs for the batch generator (below)
  status/status.txt   one line per drafted image: name, tool, prompt file, date
  prompts/            the prompt or parameters behind each generated image
  original/           copies of the textures as dumped (tex_*.png, tile_*.png)
  draft/              replacements not yet approved (same names)
  pack/               the pack proper: pack.txt, approved images, index.txt
  map/                what `make texmap` wrote (a link to it, or a copy)
  work/               the runner's per-job folders, typeset images (scratch)
  compare/            before and after sheets
```

The status of an image is where it is: `original` (selected, no image),
`drafted` (an image in `draft/`), `approved` (copied into `pack/`). It is
never stored a second time, so nothing falls out of step; `status/status.txt`
only says how a draft was made. The files are plain text so an agent edits
them with ordinary tools and a person with an editor. `selection.tsv`'s
`category` is the material a recipe is chosen by (`ash-shirt`, `ground`, ...)
and `reason` says why the row is there. `make texmap` writes `map/` (default
`out/<game>/texmap`; `init --map` links it).

## How agents use it

Everything is a command with a text output.

- `remodel texmap CAPTURE OUT` (`make texmap`): `inventory.tsv` and
  `groups.tsv` rank and cluster; `texmap.json` holds the same plus every
  frame's per-texture box, polygons and visible texel box; per frame
  `FRAME.tsv`.
- To find "the texture at pixel (x, y) of frame F": read
  `frames/F-ids.png` (RGB = `r + 256 g + 65536 b`, a few lines of code in
  any language) and look the ID up in `ids.tsv`; `frames/F-uv.png` gives the
  position inside the texture.
- To look: `frames/F-contact.png` is one image with the frame and the
  textures it uses, labelled with IDs, which a vision model can read in a
  single view; `textures/NAME.png` are the originals under their pack
  names.
- To produce: write a PNG of any size under the original's name; `make
  pack-lint PACK=...` says whether it is valid.

The studio commands (below) are the same kind: text or TSV in and out.

## How people use it

Phase 1: the static viewer `index.html` in the map directory (one file,
opens from `file://`): hover a pixel for its texture and where in it the
pixel is, click for its group, a gallery of every texture ranked by
coverage. Later: the same page next to a live player.

## The studio commands

`remodel studio SUB PROJECT [options]`, one `make studio-SUB` target each
(`STUDIO=` the project directory, `STUDIO_ARGS=` the options); `remodel
studio` lists them. All are non-interactive and idempotent (a second run
changes nothing), print short text or TSV, and need no ROM except
`compare`, which runs the game.

| command | what |
|---|---|
| `init` | the folders and files above; `--map DIR` links (or `--copy-map` copies) a texmap output |
| `region` | what a frame's region draws: textures and tile cells (TSV), from the ID and position images |
| `select` | add rows by `--key` (name, hash or ID), `--group`, `--surface`, `--frame F --region X,Y,W,H`, `--all`, with `--category` and `--reason`; copies the originals |
| `status` | counts by category, what has no draft, strays, images with no record; `--tsv` for rows |
| `approve` | copy drafts into `pack/` (`--drafted`, a group, a category, names; `--undo`) |
| `import` | bring in images made elsewhere with a `--tool` record |
| `pack` | write `pack.txt` and `index.txt` from `pack/`, run pack-lint |
| `draft` | the batch generator runner (below) |
| `style` | the local style processor (below) |
| `variants` | one restyled image for every key of its surface, tinted per key |
| `text` | text to tiles (below) |
| `seam` | the seam check (below) |
| `compare` | before and after sheets (below) |

**Batch generation (`draft`).** For each selected row without a draft
(`--force` redoes) it makes `work/draft/NAME/` holding `in1.png` (the
original enlarged with nearest-neighbour), the brief's reference images,
`ctx-frame.png` (the frame where the texture shows most, cropped around it)
and/or `ctx-group.png` (the group's other textures, their drafts where they
exist, so the halves match), and `prompt.txt`. It then runs the generator
command in that folder with the prompt on stdin, `--jobs` at a time,
retrying (`--retries`), with a timeout, appending to `run.log`; the output
`out.png` is cropped to the original's proportions, sized to the original
times `@scale` and written to `draft/`, and the prompt, command and inputs
to `prompts/NAME.txt`. A finished draft is skipped on the next run, so a
broken batch resumes. The command is a template run by `sh -c`
(`--cmd`), default `codex exec {images} -` (`{images}` becomes
`-i in1.png -i ref1.png ...`; `{files}` the names alone, `{prompt_file}`,
`{out}`); nothing depends on Codex. The generator must leave `out.png`.

A brief is plain text: the prompt, with `{name} {w} {h} {aspect} {scale}
{category} {group} {reason}` and `{images}` (a sentence saying which image
is which) replaced, and directives on lines of their own: `@ref FILE`
(relative to the brief; any number), `@scale N` (draft size over the
original's, default 4), `@context none|frame|group|both` (default `frame`).

**Style processor (`style`).** `remodel studio style P --recipe FILE`
restyles the originals of the chosen rows into `draft/`, in Rust with no
dependencies. The recipe file is `[default]`, `[material CATEGORY]` and
`[key NAME]` sections of `name = value` lines, each layer overriding the
last (`#` starts a comment at a line's start or before a space; `#rrggbb`
is a colour):

| setting | meaning (default) |
|---|---|
| `palette` | control colours, dark to light, ramped to `tones` |
| `tones` | number of tones (3) |
| `scale`, `blur`, `median` | output size over the source (4); Gaussian sigma in source texels (0.6); median radius in output pixels (2) |
| `stretch`, `bias` | percent clipped at each end before quantising (2); added to the stretched brightness, shifting the mix of tones (0) |
| `despeckle` | regions of one tone under this many source texels squared join the neighbour they touch most (0) |
| `ink`, `ink_colour`, `ink_side`, `ink_step` | edge ink width in output pixels (0: none), colour, `light`/`dark`/`both` side of a tone step, minimum step in tones (1) |
| `border_ink` | hairline width on the edges where the texture is clamped; never drawn on wrapped edges (0) |
| `halftone`, `pitch`, `angle` | 0 hard steps, else the share of a step that is dotted around it (1 = the whole ramp); dot distance in output pixels; screen angle (45; 0 and 45 repeat exactly over the texture) |
| `wrap` | `auto` (from the map's inventory), `repeat`, `mirror`, `clamp` |

Every step runs on a copy padded the way the texture wraps (repeat,
mirror or clamp, per axis from the inventory's `wrap`) and cropped
afterwards, so a tone step or dot pattern runs through the seam. Edge ink
comes only from tone steps, never from the texture's border: the first
slice's dark diagonal on the ground was a hairline drawn on two wrapped
edges. The halftone dot period is snapped to a divisor of the texture's
size.

**Surface variants (`variants`).** `--of KEY` takes KEY's restyle and
writes one image for every other key of the same surface (the inventory's
`surface` column: the same texels in another colour), tinted per key by
the ratio of the mean colours of the originals. `--mode ratio` scales each
channel (darker originals give darker variants), `luma` does the same and
then puts the restyle's brightness back so only the hue moves, `mean` is
one brightness gain; `--strength` goes part of the way; pixels in the ink
colour stay ink. `--also KEY,..` adds keys of other surfaces.

**Text to tiles (`text`).** `--frame F --region X,Y,W,H` finds the 8x8
tile cells drawn there (the ID image says which tile, the position image
where in the tile each pixel is, so the cell's corner follows). A typeset
image (`--image`, or `--text` with `--font`, `--size`, `--colour`,
`--stroke`, `--stroke-width`, typeset by ImageMagick's `magick` or by
`--typesetter`, a shell command reading `$STUDIO_TEXT`, `$STUDIO_FONT`, ...
and writing `$STUDIO_OUT`) is trimmed, fitted into `--box` (default the
region; `--pad N` keeps the text N pixels inside it; `--fit contain|stretch`, `--align`), optionally on a solid
`--bg` (`auto`: the box's commonest colour in the frame, which covers the
old text) and cut into `tile_*.png` of `--tile-scale` times 8 pixels.
`--alpha original` keeps each tile's own shape (the text is set into it),
`typeset` takes the text's alpha. A tile drawn in several places of the
region has one image: the biggest cell's wins, and the command says so.
A tile pattern is one key wherever it is drawn, so text on a ticker that
changes (song titles) is replaced for the tiles of that one title only.

**Seam check (`seam`).** For each image in `pack/` (or `--from draft`), per
axis by the wrap in the inventory: repeat compares the two edge lines
(and each against its neighbour inside) with the typical difference of
neighbouring lines inside; mirror, where the edge is its own reflection,
looks for an edge line unlike the next one (a doubled ink line); clamped
axes, and axes whose coordinates the map saw stay within one texture size
(`once`: a skater's UV map), are not checked. An axis is suspect above 2.5 times the typical step
and 12 levels, unless the original is just as suspect (the game's own
seam). `--strict` fails on suspects.

**Before and after (`compare`).** Runs the game twice (without and with the
pack, `--jobs` at once) with `remodel --out DIR run STEPS SNAP`, frames to
`compare/before/frames` and `after/frames`, then writes `sheet-FRAME.png`
(before beside after), `sheet-all.png`, and `compare.tsv` (share of pixels
changed, mean difference). It needs the ROMs and the `--input` and view
(`--run-args "--height 1080 --aspect 16:9"`) you would give `run`.
`remodel run` has `--out DIR` for it: where `run` writes `frames/`,
`eeprom.bin` and the captured programs (default `out/GAME` in the working
directory, as before; the eeprom is copied there from `out/GAME` first).

## What is built, what comes later

Built: the design above; `remodel texmap` and `make texmap` (ID images,
texel positions, inventory, contact sheets, groups, JSON); the viewer
(phase 1); the project commands, the batch runner, the style processor,
text to tiles, surface variants, the seam check and before and after
(phase 1b: render side and tooling only, no emulation or state hash is
touched).

Not built, roughly in this order:

1. **Before/after page**: the sheets in the viewer, side by side per frame.
2. **Live reload**: the player watching the pack directory and swapping
   textures as files change, so an edit shows in the running game.
3. **Pick in the player**: a mode that names the texture (and group) under
   the cursor in the running game, with the ID image drawn by the same
   renderer, adding it to the selection.
4. **Interactive editing** in the viewer: select, mark status, write a note,
   paste a prompt; stored in the project files.

## Looks: a pack plus a render profile

Textures alone only go so far; a style such as comic ink needs the picture
processed too. A **look** is a texture pack and a render profile shipped
together (a `look` file beside `pack.txt`, plain text, one parameter a
line), so "the Top Skater comic look" is one directory a player loads.

**Per-texture tags** in the pack, keyed by texture name like images and
material maps: toon steps (how many lighting bands), outline weight (0: no
outline, for HUD tiles, signs and text), rim light, emissive (self-lit:
ignores the game's lighting). The map's inventory is where an agent or
person decides the tags (a group gets one).

**Screen-space effect chain**, built from data the renderers already have:

- the **depth buffer** (`--depth`): silhouette outlines, fog, ambient
  occlusion, depth of field (photo mode has the last);
- the **separate lighting layer** of smooth shading (`model2::light`):
  toon-quantised lighting in 2-3 steps, halftone dots in the shadow step;
- the **per-pixel texture ID**: object-boundary outlines (an edge where
  the ID changes, thicker for a group boundary than inside a group) and
  per-texture effects (the tags above). This is the ID image `texmap`
  writes from captures; it is written so the same encoding and ID table
  can be produced **live by the GPU path** (an extra integer target with
  the texture key's ID, assigned the way `texmap`'s `Registry` does), so
  phase 2's outline pass reads it directly, in captures and in the running
  game;
- **motion vectors** from the temporal pass: speed lines and motion
  streaks scaled by screen-space velocity;
- the existing **live picture-style chain** (CRT, bloom, grading), which
  comes after.

**Tuning.** Live in the player (an Advanced page; the look file hot
reloads, like textures in phase 3) and from the CLI for agents (the same
parameters as `--look-set name=value`, drawn on captures so an agent can
render before/after without a window).

**Constraints.** Render only: no emulation or state-hash change, nothing
reads back into the game. Browser parity: effects are WGSL passes, so the
WebGPU build draws the same look. Depth-dependent effects need depth on;
without it a look falls back to the ID and lighting effects and says so.

**Out of scope for now**: model replacement (the pack format reserves
room for it, but games that rewrite geometry each frame make it hard) and
new geometry or particles.

**Phase 2 is built** (the look file, live ID target, ink outlines, toon
lighting with halftone, minimal speed lines, `make look-render`, live tuning
and hot reload in the player, the Comic picture style): see
docs/texture-packs.md, "Looks" and docs/enhancements.md, "Looks". Not
done: motion-driven speed lines, a browser look picker, tags set from the
viewer. Original plan: **the "comic" look for the Top Skater showcase**: ink outlines
from depth and texture-ID edges, toon lighting in 2-3 steps with halftone
in the shadow step, optional motion-driven speed lines, all as look-file
parameters. Phase 2 starts by producing the ID target live on the GPU and
the outline pass over it; the texmap inventory feeds the per-texture tags.

## Limits

- The map covers what the captured frames show. Textures drawn only between
  captures are not in it; capture more frames (`make capture`, or a
  `--capture` run with a smaller step) for more coverage.
- Groups are a heuristic (touching in one object block); they do not find
  parts of one model that are apart on screen. A seed to refine them by
  hand belongs in `selection.tsv`.
- Textures keep their identity by colour: the same texels drawn in another
  colour are another key, as in dumps.
