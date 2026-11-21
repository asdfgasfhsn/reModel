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
  map/                what `make texmap` wrote (a link to it, or a copy), or a folder of
                      several, one per run: `map/course/`, `map/chars/` (below)
  regions/NAME/       2D region overrides: region.txt, original.png, notes.txt, replacement.png
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
studio` lists them. All are non-interactive, print short text or TSV, and
need no ROM except `compare`, which runs the game. They are strict and
scriptable (below, "Strict, with answers").

| command | what |
|---|---|
| `init` | the folders and files above; `--map DIR` links (or `--copy-map` copies) a texmap output; `--map` more than once (`--map course=DIR --map chars=DIR2`) makes `map/` a folder of them; `--capture-dir DIR` records the capture directory `region-capture` reads |
| `region` | what a frame's region draws: textures and tile cells (TSV), from the ID and position images (`--map NAME` with several maps) |
| `region-capture` | cut a 2D region override out of a captured frame's tile maps (below, "Regions") |
| `regions` | the project's regions and which have a replacement (`--tsv`) |
| `select` | add rows by `--key` (name, hash or ID), `--group`, `--surface`, `--frame F --region X,Y,W,H`, `--all`, with `--category` and `--reason`; copies the originals |
| `status` | counts by category, what has no draft, strays, images with no record; `--tsv` for rows |
| `approve` | copy drafts into `pack/` (`--drafted`, a group, a category, names; `--undo`; an approved image that differs from its draft is replaced only with `--replace`) |
| `import` | bring in images made elsewhere with a `--tool` record |
| `pack` | write `pack.txt` and `index.txt` from `pack/`, run pack-lint |
| `draft` | the batch generator runner (below) |
| `style` | the local style processor (below) |
| `variants` | one restyled image for every key of its surface, tinted per key |
| `text` | text to tiles (below) |
| `seam` | the seam check (below) |
| `compare` | before and after sheets (below) |

**Several maps.** A project may hold more than one texture map: a course
run and a character-select run, say (`init --map a=DIR --map b=DIR`, or
folders and links in `map/`, each holding an `inventory.tsv`). `select`,
`import`, `status` and the rest see the textures of all of them (a name in
several is listed once, from the first map in name order, and its original
copied from whichever map has it). IDs and frames belong to one map: a
frame (`--frame F`) found in several needs `--map NAME`, as does a numeric
`--key ID`; names and hashes need nothing.

**Strict, with answers.** An unknown subcommand, or an option the
subcommand does not read, is an error that names the near matches
(`unknown option --replce; did you mean --replace?`) instead of a value
quietly ignored. Nothing overwrites a draft or an approved image without
`--replace`: `import`, `style`, `variants`, `text` and `draft` keep what is
there and say so (a warning, and when nothing at all was written an error
that names the fix; `draft` skips finished drafts so a broken batch
resumes, as before). Exit codes: 0 done, 1 a check failed (`seam --strict`,
a failed pack-lint, a draft job that failed), 2 the request was wrong, 3
something needed is missing (the project, a file, the ROM), 4 an internal
error. `--json` on any of them prints one object instead of text:

```json
{"ok":false,"code":2,"error":"studio draft: unknown option `--replce`; did you mean --replace? ...",
 "data":{"lines":["..."]},"warnings":[],"next":null}
```

`data.lines` is what the text mode printed (TSV stays TSV), `warnings` and
`next` come from the command's `warning:` and `next:` lines on stderr. The
code is `studio_answer.rs`; the answer module the rest of the CLI shares
replaces it where `cmd_studio` and `cmd_img` in `main.rs` emit.

**Names are exact.** Every command that takes names, `--group` or
`--category` (`style`, `draft`, `approve`, `import`, ...) refuses what it
can't find: a name not in the selection (a name is a row's name or its
16-digit key, never a substring), a group with no rows, or a category nobody
has is an error naming it, never "nothing to narrow by, so every row".
Filters given together all apply, and with none, `--all` is required.

**Batch generation (`draft`).** For each selected row without a draft
(`--replace` redoes) it makes `work/draft/NAME/` holding `in1.png` (the
original enlarged with nearest-neighbour), the brief's reference images,
`ctx-frame.png` (the frame where the texture shows most, cropped around it)
and/or `ctx-group.png` (the group's other textures, their drafts where they
exist, so the halves match), and `prompt.txt`. It then runs the generator
command in that folder with the prompt on stdin, `--jobs` at a time,
retrying (`--retries`), with a timeout, appending to `run.log`; the output
`out.png` is cropped to the original's proportions, sized to the original
times `@scale` (`--keep-size`: left at the generator's size) and written to
`draft/`, and the prompt, command and inputs to `prompts/NAME.txt`. A
cut-out texture's transparent texels stay transparent: the original's alpha,
resampled to the draft's size, is applied to the opaque picture a generator
returns (a draft of an opaque original is left alone). A finished draft is skipped on the next run, so a
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
is a colour; a section given again continues the first, later settings
winning a clash):

| setting | meaning (default) |
|---|---|
| `palette` | control colours, dark to light, ramped to `tones` |
| `tones` | number of tones (3) |
| `scale`, `blur`, `median` | output size over the source (4); Gaussian sigma in source texels (0.6); median radius in output pixels (2) |
| `stretch`, `bias` | percent clipped at each end before quantising (2); added to the stretched brightness, shifting the mix of tones (0). A flat texture (its brightness range under 1/64) isn't stretched: it takes the tone its luminance has on the palette's scale (white the lightest, black the darkest) |
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
region; `--pad N` keeps the text N pixels inside it (`--pad X,Y`: left and right, top and bottom; `--pad L,T,R,B` per side); `--fit contain|stretch`, `--align`), optionally on a solid
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

**Image tools (`remodel img`).** The per-pixel jobs that went through
shell and ImageMagick, in the binary, with no ROM and `--json`:

| command | what |
|---|---|
| `img sheet OUT.png IMG.. [--cols N] [--cell PX] [--labels A,B,..] [--bg #rrggbb]` | a contact sheet: each image fitted in a PX square (default 256; small art enlarged by whole numbers, a checker under transparency) with its name under it; `data.cells[]` has `n`, `label`, `file`, `box` so the picture need not be read to learn what is where |
| `img diff A.png B.png [--threshold N] [--max-changed PCT] [--out DIFF.png]` | `changed_pct` (a pixel counts when its channels differ by more than N together, default 6), `mean_abs_diff`, `max_diff`, `bbox`, `identical`; `--out` writes a heat picture; `--max-changed` fails (exit 1) above PCT, and for pictures of different sizes |
| `img stats IMG.png [--region X,Y,W,H]` | size, `opaque`, `transparent` and `partial` shares, `alpha_levels`, mean colour, brightness mean, range and histogram, `colours` used and the top five, the `bbox` of what is not transparent |

Nothing is overwritten without `--replace`, and never an input. `crop`,
`resize`, `quantise`, `composite` and `roll` of the spec are not built.

**Before and after (`compare`).** Works from any directory (the repo root,
with `rom/` and `out/`, is found by walking up from the working directory
and the project) and renders from a snapshot of the pack copied when it
starts, so a pack being rebuilt meanwhile is never seen half written. Runs
the game twice (without and with the
pack, `--jobs` at once) with `remodel --out DIR run STEPS SNAP`, frames to
`compare/before/frames` and `after/frames`, then writes `sheet-FRAME.png`
(before beside after), `sheet-all.png`, and `compare.tsv` (share of pixels
changed, mean difference). It needs the ROMs and the `--input` and view
(`--run-args "--height 1080 --aspect 16:9"`) you would give `run`.
`remodel run` has `--out DIR` for it: where `run` writes `frames/`,
`eeprom.bin` and the captured programs (default `out/GAME` in the working
directory, as before; the eeprom is copied there from `out/GAME` first).

## Regions

Some 2D art can't be restyled tile by tile: the tiles are shared (a sponsor
logo built from the same fill tiles as the background, "©SEGA 1997" whose
letters are also "INSERT COIN(S)", the boot logo). A **region** names an
arrangement of tiles in a tile map and an image drawn over its rectangle
instead (docs/texture-packs.md, "Regions" is the pack side: matching is in
map space, so it follows scrolling).

`remodel studio region-capture PROJECT NAME --capture-dir DIR --frame F --region
X,Y,W,H` cuts one out of a captured frame (the capture directory of a
`--capture` run; `init --capture-dir DIR` remembers it). `--region` is in the
pixels of the frame picture the map holds (`--game-px`: game pixels, 496 x
384); the tool finds the tile cells the rectangle covers (cells it overlaps
by at least half), in the plane it draws in (`--plane a|b`), and writes:

- `regions/NAME/region.txt`, the line for `regions.txt`: the keys of those
  cells. `--also F,..` compares the same cells in other frames and makes
  the ones that differ `*` (animated parts); `--wild X,Y;X,Y` names more;
  `--layer high|low` and `--pin` (only at this map cell) narrow it.
- `original.png`, the game's picture of exactly that rectangle, enlarged
  (`--ref-scale`, default 4), for the artist or agent to redraw;
- `notes.txt`: where it was cut from and the size to draw it at (4x).

It says where the region matches in the frame (and the `--also` frames), and
warns when it matches more than once, when its tiles are all one pattern
(widen the rectangle), or when its cells aren't contiguous in the map. Put
the artwork at `regions/NAME/replacement.png` (or give `--image FILE`), then
`studio pack` writes `regions.txt`, the `region_NAME.png` images and `uses
regions` into `pack/`; remove the replacement and pack again to take a
region out. `studio regions` lists them.

## Catalogue

The map above comes from a few captured frames, so it shows what those
frames showed. The **catalogue** is the same idea grown by observing: every
distinct asset a game shows, found while it runs, kept per game in
`out/<game>/catalogue/` (ROM-derived, so ignored by git), added to by every
run and every play session, never rebuilt. It answers "have we seen all of
it?" (later course sections, other skaters, menus) and holds the
mappings the studio lacks (which HUD font tile is which digit).

Nothing in it is specific to one game or touches the emulation: it reads
what the renderers already read, and the state hash does not change.

### What is catalogued

| kind | what | key |
|---|---|---|
| `tex` | a 3D texture as drawn: size, sheet place, luma table, colour, wrapping (`rr`, `mr`, `rm`, `mm`), and its **surface key** (the artwork whatever colour it is drawn in) | the texture dump's key, so a name is a pack file |
| `tile` | an 8x8 2D tile pattern | the tile dump's key |
| `layout` | a connected arrangement of 4 to 600 tiles on one layer: a logo, a label, a number field, a panel. Its **shape** (cells only, no tile keys) is recorded too: a shape that recurs with different contents is a text or number field | hash of cells and tile keys |
| `object` | one model's polygon list: its model address when the native geometrizer's side channel knows it (`oba`), else the set of its polygon headers (an approximation: two models with the same headers collide); polygons, textures it shows | the address, or the headers |
| loads | texture RAM uploads (below) | frame, place, hash |

For each entry: first and last seen, how often (samples), the screens it
appears on, coverage (visible pixels, best sighting with its frame and
box), a thumbnail, and what it appears with: textures and tiles that touch
in one object or layer (the groups of `make texmap`), an object's textures,
a texture's objects. A **screen** is a recognisable state of the picture:
samples whose most covering textures (by surface) and tiles (by tile
number, so fades and palette changes do not split it) are alike (Jaccard
0.5 over the top 24); sessions' screens are merged the same way.

### Loading, and what it joins

`model2::texwatch` watches texture RAM. The game fills it with the CPU, so
there is no upload call to hook and a hook on every write would cost the
JIT its fast path. Instead, once a frame, the bus compares the texture
RAM's write counter (which the fast path keeps) and only when it moved
compares the two banks with a shadow copy, page by page; the changed pages
become loads: frame, byte range and sheet rows (`bank0 rows 64-127`), a hash of the
new bytes and, when the data is a verbatim copy of main data ROM (32-byte
windows looked up at any 4-byte shift), the ROM address it came from. It is
off by default, on with `--catalogue`, outside the machine's state (saving,
loading, hashing and run-ahead never see it).

Joined with what is drawn: a texture knows the load that fed it (when it
was loaded, per screen) and how many loads overlapped its texels; its
surface key says which keys are one artwork; the report counts the loaded
bytes that lie under textures seen drawn, and lists loads no drawn texture
touches (loaded, never drawn: or not yet observed). A frame number that
jumps or goes back (a loaded save state) makes the watch start from what
is there instead of calling it all a load, and the next sample is taken at
once; a session is one process, so a long run is one session and several
runs are several, merged when read.

### Store

`out/<game>/catalogue/`:

```
format.txt                 format number and game
sessions/<id>/             one per observing process (id: start time, pid)
  info.txt                 label, frames, samples, loads, cost counters
  entries.tsv              every entry the session saw: ident, name, size, surface, detail, loaded, loads, with
  seen.tsv                 per entry: samples, pixels, best sighting (frame, box), first, last, screens, wrap
  edges.tsv                touching pairs and weights
  screens.tsv              the session's screens: samples, frame image, top signatures
  samples.tsv              append-only: one line per sample (frame, screen, counts, new, frame image)
  loads.tsv                append-only: frame, offset, length, hash, destination, ROM source, screen
thumbs/                    texture, layout and object images; tiles.dat (all tile patterns, 8x8 RGBA records)
frames/                    saved frames (when something new showed, capped) and .tiles.z tile data of those showing a new layout
index.html, report.txt, index.tsv    written by the commands below
```

Entry ids are `kind:key` (`tex:0123456789abcdef`). Files are plain TSV,
appended or replaced whole (atomically, every 30 s and at the end), so a
killed session loses its last half minute and sessions never write each
other's files: **merging is copying directories** (`remodel catalogue
merge SRC`), and everything a report reads is computed from the sessions,
oldest first. Texture, tile and frame pictures are named by key or session
so they can be shared.

### Observing

- `make catalogue GAME=topskatr INPUT=coin-start` runs the scripted run
  (`INPUT` as `--input`, `CATALOGUE_STEPS=` its length, `LABEL=` its name) with `remodel --catalogue
  DIR run`, then the report and viewer. `make catalogue-runs RUNS=FILE` runs a list
  (lines `LABEL|STEPS|INPUT`): attract, every course, every character, the
  menus, each its own session. Scripted runs go faster than real time, so
  the worker is waited for (no sample is dropped).
- `remodel-play --catalogue` (the **Catalogue** setting, off by default, in
  the Esc menu's More page) observes while you play: docs/play.md,
  "Catalogue". A sample is taken every 60 frames, and sooner (at most every 8) when
  the tile layer was rewritten in bulk or textures were loaded.

### Reading it

- `make catalogue-report` (`remodel catalogue report`): per kind the entries, those new in
  the last session, those seen in one sample only, and a Good-Turing
  coverage estimate (1 minus the share of sightings that are single: the
  chance a new sample shows something the catalogue holds; a guide, not a
  measurement); sessions; the screens with their entries and how many were
  seen once (a screen with many is "thin: observe it longer"); what is new
  since the last session; loaded against drawn; how many keys are one
  artwork.
- `make catalogue-viewer`: `index.html` in the catalogue directory, one
  file that opens from `file://`. A gallery filtered by kind, screen and
  text and sorted by use, pixels or rarity; click an entry for its key,
  surface, detail, wrapping, first and last sighting, load, the frame it is
  best seen in with its box marked, the screens it is on (click to filter),
  what it touches, its textures or objects, and its other keys with the same
  artwork. Tabs for the screens (a frame of each), the loads and the
  report.

### The studio

- `studio init --map catalogue=out/GAME/catalogue` writes the catalogue as
  a map into the project (`catalogue-maps/`): `inventory.tsv` ranked by
  pixels, `groups.tsv` from what touches, `textures/` under pack names. `select
  --key/--group/--surface/--all`, `status`, `style`, `draft` and the rest then
  work from it, alongside a texmap map (give `--map` more than once). The
  catalogue has no per-frame ID images, so `select --frame --region` and
  `studio region` need a texmap map of the frame.
- `studio region-capture NAME --capture-dir out/GAME/catalogue --frame N
  --game-px --region X,Y,W,H` cuts a region from the tile data the catalogue
  saved with the frames that first showed a layout (`remodel catalogue
  captures` lists them: frame `1000000 * (session + 1) + frame`).
  `remodel catalogue export-map` and `captures` write both
  forms without a project.

### Glyph mapping

HUD numbers are tiles: a digit is one tile, or a block of them (the Top
Skater clock's are 2x2, the four **quarters** of the glyph). Their names say
nothing about which digit they show. `make catalogue-glyphs INPUT=... `
(`remodel --glyphs DIR run`) watches the tile layer every frame, keeps only
the cells whose tile changes, and reads the digits off how a counter
changes:

1. cells that change in the same frames and touch are one glyph block;
   its changes are replayed into states (the tiles of its cells), and a
   block whose states step through ten in a clean cycle is a digit place;
2. another block that changes only when that one wraps (the tens of
   seconds, the minutes) says which step is 9 to 0: the state after it is
   the zero of a counter that counts up, the nine of one that counts down;
3. the direction comes from laying both against a 3x5 template font (the
   better fit, when they differ by 0.03) else from `--glyph-counts up|down`;
   with no carry partner the template fit alone chooses, and says so.

`glyphs.tsv` has a row per tile: `tile key`, `glyph`, `quarter` (`-` one
tile; `tl tr bl br`; `rRcC` for bigger blocks), `evidence`, `confidence`; a
tile that serves two digits has two rows; `glyphs.txt` shows each glyph as
the tiles draw it for checking; `events.tsv` and `tiles.tsv` are the trace,
and `remodel catalogue glyphs --counts down` infers again from it without
playing. It is an inference from behaviour, not read from the ROM, so
unconfirmed until looked at: a score that never changes in the run gives
nothing (play, or script, something that scores).

## Scenes

A scene is a named moment of a game: a recipe (the game, an input script
and a frame) plus a local copy made from it. Checking an art change at a
scene needs no boot, no build, no lock and no ROM: `remodel scene render`
draws the copy's capture in about two seconds. Design and what is still to
come: [uplift/scenes-and-server.md](uplift/scenes-and-server.md).

**Recipes** are small text and hold no game data. A game's own scenes are
in its manifest (`crates/romset/src/games/`, next to `attract_frame`; Top
Skater has five, `vf2` three). A project adds its own in `scenes.txt`, and
its names win over the manifest's:

```ini
[scene course-start]
title = Novice course, first stretch
input = coin-start          # any input script; coin-start is the game's own
frame = 3300                # counted from power-on with a fresh EEPROM
tags  = play hud            # attract menu select play result hud
required = yes              # kept for pack checks (not used yet)
coverage = 0.95
```

`--project DIR` reads `DIR/scenes.txt`; without it, a `scenes.txt` in the
working directory is read if there is one. The file is strict: an unknown
setting or tag is an error that lists near matches.

**Local copies** are derived from the ROM and never shared or committed.
They live in `<data dir>/<set>/scenes/<name>/` (`out/<set>/scenes` in a
source tree, else the player's data folder; `REMODEL_DATA` overrides):

| File | Holds |
|---|---|
| `scene.txt` | The recipe as run, `eeprom = fresh`, the geometrizer mode, the reModel version, the state hash at the frame, and `stale` or `verified` after `scene verify` |
| `capture/` | The renderer's inputs for the frame (what `look-render`, `gpu-check` and `pack-check` read); about 19 MB on Top Skater and 28 MB on `vf2`, most of it the texture ROM, copied into every scene |
| `state.save` | A save state at the frame, for the live player and clips (not used yet) |
| `thumb.png` | A small picture |

**Commands** (all take `--json`, see below):

```sh
remodel --game topskatr scene list [--tag play] [--project DIR]
remodel --game topskatr scene make NAME | --all [--replace]    # needs the ROM
remodel --game topskatr scene verify [NAME]                    # needs the ROM
remodel --game topskatr scene render NAME [--pack DIR] [--crop X,Y,W,H] \
        [--against original] [--view native|H[,A:B]] [--out DIR]
```

- `list` shows each scene and its state: `ready`, `missing` (no local
  copy), `stale` (its recipe changed, or `verify` found the emulation
  changed) or `unreproducible` (a copy whose recipe is gone).
- `make` runs the recipe from power-on with a fresh EEPROM and keeps the
  copy. It refuses to overwrite a copy without `--replace`; `--all` makes
  the missing ones and lists the others as `skipped`.
- `verify` runs each recipe again and compares the state hash at its frame
  with the recorded one. A different hash means the emulation changed (runs
  are bit-exact across engines), so the scene is marked stale, the command
  exits 1, and `make --all --replace` rebuilds. Run it after a reModel
  update; a difference in pictures is then not blamed on a pack.
  `make scene-verify`.
- `render` draws the scene from its capture on the GPU with no emulator.
  `--pack DIR` draws a texture pack; `--against original` also draws the
  original and writes `NAME-original.png`, `NAME-pack.png` and a
  side-by-side `NAME-sheet.png`, and reports the share of pixels that
  differ. `--crop X,Y,W,H` is in the game's own 496x384 picture whatever
  the view (a wide view reaches negative X). `--view` is `native`, a height
  or `HEIGHT,A:B`; the default is 1080p 16:9 (the global `--height`,
  `--aspect` and `--hud` also work), and `--look` and the shading options
  apply as for `look-render`. Output goes to `out/<set>/scene-render`
  (`--out`); it is drawn again on every call, so nothing old is ever read
  back. Layered packs (several `--pack`) are not built.
- `studio compare PROJECT --scene all` (or `NAME,NAME`) writes the studio's
  before and after sheets from scenes instead of running the game twice.
  The run-based compare now refuses a folder that already holds a run
  unless `--force` runs it again; it used to keep the old frames silently.

**The answer.** Every scene command follows the uplift interface rules
([uplift/README.md](uplift/README.md)). With `--json` it prints one object:
`ok`, `code`, `error`, `data`, `warnings` and `next` (a suggested command).
Exit codes: 0 done, 1 a check failed (`verify` found a stale scene), 2 the
request was wrong (an unknown scene, option or tag lists near matches), 3
something missing (the ROM, a local copy), 4 an internal error. The
code is `crates/cli/src/answer.rs`, which later commands use too.

```sh
remodel --game topskatr scene render race-course --pack out/topskatr/texpack --against original --json
```

`data` has `picture`, `original`, `sheet`, `width`, `height`, `polygons`,
`changed_share` (0 to 1), `pack`, `crop` and the time in ms.

**Make targets:** `make scene-list`, `scene-make`, `scene-verify`,
`scene-render` (`SCENE=`, `PACK=`, `SCENE_OPTS=`) and `make scene-check`,
the acceptance test: it makes every scene of `topskatr` and `vf2` from a
fresh folder, verifies them, draws each against the original and times it.

**Acceptance test** (what the slice is judged on; `make scene-check` is its
repeatable half). With a prebuilt binary, no lock and no boot, an agent
that has a scene library and a pack must be able to check one still in at
most 3 calls and 1 picture: `scene list`, `scene render NAME --pack DIR
--against original --json` (the numbers say whether the pack drew and how
much changed), and one crop only if it needs to look. The draw must take
about 2 s or less cold at 1080p; a wrong name, an unknown option or a
missing copy must answer with exit 2, 2 and 3 and near matches; `verify`
must exit 1 when a hash differs. Run on 2026-11-21, on a machine with other
work running (load about 5):

| | `topskatr`, `race-course` | `vf2`, `fight-start` |
|---|---|---|
| Pack | 5,271 files (the demo pack) | 170 textures, colours inverted (made for the test) |
| `scene render --against original --json`, wall time, cold, 1080p | 1.83 s (1.82 to 2.22 over 4 runs) | 0.95 s (0.94 to 0.98 over 3) |
| of which GPU start, draw (both pictures), writing 3 PNGs | 0.22, 1.06, 0.45 s | 0.21, 0.13, 0.51 s |
| Pixels changed against the original | 62% of the top-left quarter crop (30% of `race-go`, whole frame) | 71.5% |
| `scene make --all` (JIT), per scene | 1 to 4 s; 11 s for the five | 2 to 4 s; 9 s for the three |
| `scene verify` | 10 s on the JIT; the interpreter reproduces the same hashes (`vf2`: 21 s) | same |
| Calls, pictures | 3 calls, 1 picture | 3 calls, 1 picture |

It has not been run by a separate mid-tier model that had read only
[uplift/interface.md](uplift/interface.md): the calls above were made by the
author of the tool. A slice's acceptance test calls for that run, and it
is still to do.

**Not built yet:** `scene what`, `where`, `sheet` and `clip`; `--asset` and
`--overlay` on `render`; the per-asset `assets[]` report (which assets the
pack replaced, and why not); `facts.tsv`; the side channel in captures
(each object's model key); layered packs; the player's `--scene` and the
server. Scenes use the default (chip) geometrizer; the design's choice of
native waits for the side channel.
## Tours and coverage

The catalogue holds what somebody happened to play, and its own
coverage figure only says how well the screens visited were sampled, not
how much of the game was never visited
([uplift/content-coverage.md](uplift/content-coverage.md)). **Tours** and
**ROM read maps** give a number to count against: how much of a game's
polygon ROM and texture ROM the tours have read.

### Tours

A tour is a named input script from power-on: one line of `LABEL|LENGTH|INPUT`.

- `LENGTH` is CPU steps (as `run`'s STEPS) or frames with an `f` (`6200f`).
- `INPUT` is the `--input` script (`name@from-to`, `name=value@from-to`,
  `coin-start`), plus two terms: `from=LABEL@FRAME` starts as another tour
  did up to that frame, then goes on with this tour's own inputs (which
  must start after it: a branch cannot rewrite the past), and
  `mark=NAME@FRAME` saves the state as that frame starts, to
  `out/GAME/coverage/states/LABEL-NAME.state`.

A game's own tours are in `crates/romset/src/games/tours/GAME.txt`: input
scripts and frame numbers, nothing taken from a ROM. A project adds its own
with `--tours FILE`. Top Skater, `vf2` and `daytona` have tours today; every
game has a `bench` tour (the bench's steps and input) so a tour run can be
checked against `tools/expected-hashes.txt`.

```sh
make tour-list GAME=topskatr             # the tours, their branches and marks, which have a map
make tour-run GAME=topskatr              # all of them (TOUR=coin-start for one)
make coverage GAME=topskatr              # the report
```

`remodel tour run` always starts from power-on in a machine of its own,
with the EEPROM as the board is made: it never reads an `eeprom.bin` left in
an output folder (that changed a state hash on its own: a tour that carried a
stale one did not reproduce). So a tour run twice gives the same state hash
and the same map. It uses the native geometrizer where the game has one
(`--geo chip` for the game's own program: the same state hashes) and the JIT
with `--jit`.

### Read maps

Each tour records, beside the run, which words of two ROMs it read:

- **Polygon ROM**: the words the geometrizer read from the ROM half of its
  bus. A model drawn from polygon RAM is not a ROM read and is not counted
  (the first spike's map counted up to two small ones a frame on `vf2` and
  `fvipers`). The hook is one place, the bus read, so it covers the native
  geometrizers (2A, 2B, 2C and the original Model 2) and also the game's
  own program on the chip (`--geo chip`).
- **Texture ROM**: the texture headers and UV lists the rasteriser read,
  noted where the raster stream is built (`raster::build_impl`, shared by
  the software and GPU renderers), on a copy of each frame the tour grabs.
  The texels are not here: they sit in texture RAM, loaded from the data
  ROM, which these maps do not follow yet.

Counts are **bytes** (the polygon ROM is read in 4-byte words, the texture
ROM in 2-byte words): nothing marks where a header or a model begins, so
"headers" cannot be counted. A map is a bitmap, one bit per ROM word,
written to and never read back by the machine: nothing in emulation sees it,
so state hashes, save states and run-ahead are unchanged with it on, and off
is the default (`make coverage-check` proves the first on three boards).
It is not on in the player.

**Boards.** Every board whose geometrizer reads the polygon ROM through the
shared bus is counted, which is all of them. The games with no native
geometrizer (`zerogun`, `pltkids`) run the chip path only, and that is
counted the same way; there is no native path to differ from. Texture
reads come from the rasteriser, which every game has.

### The report

`remodel coverage` (no ROM needed; it reads the maps) merges the maps of
the game's tours by union. For each ROM it gives the share of non-blank
bytes read (blank: a run of at least 256 bytes all zero or all ones), the
share of the whole ROM, the read share of each 1 MiB region, and the unread
stretches, largest first, by address range and size. It lists addresses and
sizes only; no ROM bytes. A tour that has no map yet, or whose line changed
since its map, is named in a warning.

`--json` on `tour list`, `tour run` and `coverage` prints one object,
`{ok, code, error, data, warnings, next}`, and exits with `code`: 0 done, 2 a
bad request (unknown tour, bad option), 3 something missing (no map yet:
`next` says which command makes it), 4 an internal error.

### What the shipped tours read (measured 2026-11-21)

| | Top Skater (2C) | `vf2` (2A) | `daytona` (original) |
|---|---|---|---|
| Tours | `coin-start`, `characters`, `expert` (and `bench`) | `coin-start`, `attract` (and `bench`) | `coin-start`, `attract` (and `bench`) |
| Polygon ROM | 8 MB, none blank | 12 MB, 1.60 MB blank | 16 MB, 4.20 MB blank |
| Read, of the whole ROM | 27.9% | 27.3% | 17.6% |
| Read, of what is not blank | 27.9% | 31.5% | 23.9% |
| Unread stretches | 40, the largest 4.66 MB | 16,082, the largest 1.57 MB | 18,396, the largest 1.59 MB |
| Texture ROM read (headers and UV lists) | 17.6% of 8 MB | 12.0% of 16 MB | 10.2% of 16 MB |

The polygon figures agree with the first spike's (28%, 28% and 18%;
[uplift/content-coverage.md](uplift/content-coverage.md), E1 and E3): the
`vf2` difference is the spike counting its RAM models. The Top Skater
tours are the three runs of its first catalogue (coin start, character
walk, a steered course), written as tours.

**The chip path reads the same words, plus one.** `--geo chip` runs the
game's own geometrizer program, which reads the word after each model; the
native geometrizer reads a model's words and stops. On a 2,500-frame
coin-start the two maps of Top Skater (2C) differ by 60 bytes in 15 ranges
(733,512 against 733,572) and `gunblade`'s (2B) by 480 of 1.63 MB; the
state hashes are equal. `pltkids`, which has no native geometrizer, reads
2.7% of its polygon ROM in the same run on either setting.

**The benchmark starts from a blank EEPROM.** `make bench` used to load
an `eeprom.bin` an earlier run had written, so 16 of the 25 expected hashes
were second-run hashes and a fresh folder gave different ones. It now runs
with `--fresh-eeprom` (no EEPROM loaded or kept), the same start as a tour, so
a tour's hash and the benchmark's agree (Top Skater `7f0275251269648e`).

### What it does not do yet

No tour recorder in the player, no `poke=`, no crawler, no model browser for
the unread stretches, no map of the data ROM or the tiles
([uplift/content-coverage.md](uplift/content-coverage.md), items 3 to 5 of
its MVP).

## What is built, what comes later

Built: the design above; named scenes and `scene render` (above); `remodel texmap` and `make texmap` (ID images,
texel positions, inventory, contact sheets, groups, JSON); the viewer
(phase 1); the project commands, the batch runner, the style processor,
text to tiles, surface variants, the seam check and before and after
(phase 1b: render side and tooling only, no emulation or state hash is
touched). The catalogue (below) is built too: observing while playing and in
scripted runs, loads, report, viewer, glyph mapping, and the studio reading it.

Not built, roughly in this order:

1. **Before/after page**: the sheets in the viewer, side by side per frame
   (`studio compare` writes the sheets, from a run or from scenes).
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
