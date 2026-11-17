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
ignored: it is ROM-derived):

```
studio/<project>/
  style.md            the style sheet (step 1)
  selection.tsv       id, name, group, coverage, status, note (step 3)
  status/             one line per image: name, status, tool, prompt file, date
  prompts/            the prompt or command behind each generated image
  original/           copies of the textures as dumped (tex_*.png, tile_*.png)
  draft/              replacements not yet approved (same names)
  pack/               the pack proper: pack.txt, approved images, index
  map/                what `make texmap` wrote (texmap.json, inventory.tsv, frames/)
```

`status` is one of `original` (untouched), `drafted` (an image exists in
`draft/`), `approved` (copied into `pack/`). The files are plain text so an
agent edits them with ordinary tools and a person with an editor; the
status of an image is the directory it is in plus its line in `status/`,
so nothing has a second source of truth to fall out of step. The
phase-1 `make texmap` writes `map/` (default `out/<game>/texmap`; point
`TEXMAP_OUT` at `studio/<project>/map`); the rest is convention until the
later phases give it commands.

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

Later phases add subcommands in the same style (`select`, `status`,
`draft`) with TSV or JSON output.

## How people use it

Phase 1: the static viewer `index.html` in the map directory (one file,
opens from `file://`): hover a pixel for its texture and where in it the
pixel is, click for its group, a gallery of every texture ranked by
coverage. Later: the same page next to a live player.

## What phase 1 builds, what comes later

Built (phase 1): the design above; `remodel texmap` and `make texmap`
(ID images, texel positions, inventory, contact sheets, groups, JSON);
the viewer. Render side only; no emulation or state hash is touched.

Later, roughly in this order:

1. **Project commands**: `studio init/select/status` over the layout above,
   keeping `selection.tsv` and `status/` consistent and building `pack/`
   from the approved images; a before/after page (the same frames with and
   without the pack, side by side in the viewer).
2. **Batch hooks**: `studio draft` running the Codex CLI image generation
   or an upscaler over a selection (a group at a time, with the group's
   other textures as context so the halves match), writing the prompt and
   tool next to each image.
3. **Live reload**: the player watching the pack directory and swapping
   textures as files change, so an edit shows in the running game.
4. **Pick in the player**: a mode that names the texture (and group) under
   the cursor in the running game, with the ID image drawn by the same
   renderer, adding it to the selection.
5. **Interactive editing** in the viewer: select, mark status, write a note,
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

**Phase 2 is the "comic" look for the Top Skater showcase**: ink outlines
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
