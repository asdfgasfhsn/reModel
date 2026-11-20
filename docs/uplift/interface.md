# The interface in one place

Shared words, roles and rules: [README.md](README.md). Nothing here is
built. This page gathers what the other specs each define a part of: every
verb, where files live, who needs what, the status of an asset, and three
tasks written out command by command. Where a spec and this page differ,
this page is the intent and the spec needs correcting.

## Verbs

The CLI is `remodel NOUN VERB`; the server and the agent API
use `noun.verb` with the same arguments. Every verb takes `--json` and
`--project DIR` (default: the current folder).

| Noun | Verbs | Spec |
|---|---|---|
| `scene` | `list`, `make`, `verify`, `render`, `what`, `where`, `sheet`, `clip` | [scenes-and-server](scenes-and-server.md) |
| `asset` | `list`, `show`, `bundle`, `name`, `set`, `same`, `make`, `normalise`, `check` | [asset-identity](asset-identity.md), [generation](generation.md), [quality-gates](quality-gates.md) |
| `region` | `capture`, `list`, `check`, `field` | [2d-cells](2d-cells.md) |
| `glyphs` | `infer`, `show`, `confirm` | [2d-cells](2d-cells.md) |
| `project` | `init`, `status`, `brief`, `review`, `approve`, `reject` | [pack-project](pack-project.md) |
| `pack` | `build`, `check`, `report`, `resolve`, `diff`, `doctor`, `rekey` | [pack-project](pack-project.md), [quality-gates](quality-gates.md) |
| `look` | `set`, `calibrate`, `suggest-tags` | [renderer-controls](renderer-controls.md) |

Content coverage adds `tour` and `coverage`, and distribution adds
verbs to `pack`. Pass two adds three nouns and a few verbs: `model export/check`
([model-interchange](model-interchange.md)), `place check`
([added-elements](added-elements.md)), `shader check/list`
([shader-packs](shader-packs.md)), and `scene camera`, `asset set ID
class=` and `asset list --suggest-class`
([scene-understanding](scene-understanding.md)).

An art agent's job uses eleven of them: `project brief`, `asset show`,
`asset bundle`, `asset set`, `asset make`, `asset normalise`,
`asset check`, `scene render`, `scene what`, `region capture` and
`region check`. `tools/lock`, `remodel img` and
`backend` are tools around these, not part of the set
([agent-environment](agent-environment.md)).

### Conventions

- **IDs.** An asset is named by its slug (`wall-base`), its key
  (`art:...`, `pat:...`, `layout:...`) or today's file name
  (`tex_..._WxH`). A region is an asset of kind `layout`, and its slug is
  the region's name.
- **Coordinates.** Every `X,Y,W,H` is in the game's own picture, 496x384,
  whatever the view. A wide view extends to negative and larger X.
- **`--view`** sets the size and aspect drawn. **`--overlay`** replaces
  the picture with a diagnostic one. They are independent.
- **Exit codes.** 0 done. 1 a gate at `error` failed. 2 the request was
  wrong. 3 something needed is missing. 4 an internal error. A `warn` or
  `review` verdict does not change the exit code.
- **The envelope.**

  ```json
  {"ok": false, "code": 3,
   "error": "scene 'char-select' has no local copy",
   "next": "remodel scene make char-select   (needs the ROM)",
   "data": null, "warnings": []}
  ```

## Reference: every verb's shape

A cold read of this page by a mid-tier model (2026-11-19) completed one
task of three: the examples showed some verbs and left the shape of the
rest to be guessed. This table is the fix. `ID` is a slug, a key or a
file name; `[...]` is optional; `A|B` is a choice. `data` fields are
listed in the last column.

| Command | Answers with |
|---|---|
| `scene list [--tag T]` | `scenes[]`: `name`, `tags[]`, `local`, `stale`, `tour`, `frame`; and `tags`, every tag in use. Tags come from a fixed list: `attract`, `menu`, `select`, `play`, `result`, `hud` |
| `scene make NAME\|--all [--replace]` | `made[]`, `failed[]` |
| `scene verify [NAME]` | `scenes[]`: `name`, `ok`, `why` |
| `scene render NAME [--asset ID]... [--against original\|REV] [--overlay CLASS] [--view V]` | `picture`, `changed_share`, `assets[]`: `name`, `drawn` (`replaced`, `original`, `absent`), `variants_drawn`, `misses` |
| `scene what NAME X,Y[,W,H]` | `things[]`, largest first: `id`, `name`, `kind` (`art`, `pattern`, `layout`, `field`, `model`), `status`, `share`, `box` |
| `scene where ID [--scene NAME]` | `scenes[]`: `scene`, `share`, `boxes[]` |
| `scene sheet NAME --label art\|layouts\|models` | `picture`, and `labels[]`: `n`, `id`, `name`, `box`, so the picture need not be read to get names and boxes. `layouts` covers everything 2D: tiles, regions and text |
| `scene clip NAME --frames N` | `clip`, `frames`, `flicker[]` |
| `asset list [--status S,...] [--category C] [--kind K] [--scene NAME]` | `assets[]`: `id`, `name`, `kind`, `status`, `category` |
| `asset show ID` | The row: `id`, `name`, `kind`, `status`, `category`, `colour`, `draw`, `size`, `variants`, `scenes[]`, `draft`, `art`, and `last`: `actor`, `verb`, `time`, `note` (a rejection's reason is here) |
| `asset bundle ID` | As in task A; `folder` holds the original, the mask and the crops |
| `asset name ID SLUG` | `id`, `name` |
| `asset set ID KEY=VALUE...` | The changed row |
| `asset same ID ID`, `asset same --suggest` | `merged`, or `pairs[]` with a likeness |
| `asset make ID --with BACKEND [--replace]` | `outcome` (`ok`, `repaired`, `refit`, `gate:NAME`, `backend:WHY`), `draft`, `status`, `gates[]`, `sheet` |
| `asset normalise ID FILE [--replace]` | `draft`, `status`, `changes[]`. `FILE` is any PNG anywhere; a corrected copy is written to `draft/<slug>.png` |
| `asset check ID` | `status`, `gates[]`: `gate`, `value`, `limit`, `verdict` (`pass`, `warn`, `review`, `error`), `fix`; `sheet` if a gate failed |
| `region capture SCENE X,Y,W,H --name NAME [--replace]` | As in task B |
| `region list` | `regions[]`: `name`, `cells`, `status` |
| `region check NAME` | `scenes[]`: `scene`, `matched`, `cells_matched`, `cells` |
| `project status [--status S,...] [--category C] [--scene NAME]` | `actor`, `approvers[]`, `counts` by status, `assets[]` (as `asset list`), `waiting`: `to_approve` (checked and clean) and `to_review` (checked, with a `review` item to accept), and `log[]`, the last twenty lines |
| `project brief [--actor NAME]` | `actor`, `assets[]`, `scenes[]`, `verbs[]`, `budget` |
| `project review [--category C]` | `sheets[]`: `category`, `picture`, `assets[]` |
| `project approve ID...\|--category C [--accept "REASON"]` | `approved[]`, `skipped[]`: `asset`, `why`. With `--category`, assets not yet `checked`, or with a `review` item and no `--accept`, are skipped with the reason, not errors. A named ID that is not `checked` is exit code 2 |
| `project reject ID --reason "TEXT"` | The changed row |
| `pack build [--archive] [--out DIR] [--replace]` | `out` (default `build/` in the project), `files`, `bytes`, and `archive`, the path of the file to publish. It builds whatever is approved and does not need `pack check` first |
| `pack check [--level asset\|scene\|clip\|pack]` | As in task C. `done` is true when nothing is at `error`, no required asset is short of `approved`, and every required scene meets its coverage. With no level it runs every level it can and lists the rest under `skipped` |
| `tour list`, `tour run NAME\|--all`, `coverage` | [content-coverage](content-coverage.md) |
| `pack find`, `pack install`, `pack update`, `pack publish --check` | [distribution](distribution.md) |

Rules the table leans on:

- **The envelope always has all six fields.** `code` equals the exit
  code, `error` is null on success, `warnings` may be empty, and `next`
  may be null: it is a hint, never something to depend on.
- **`--replace`** is taken by every verb marked with it. Without it, a
  verb that would overwrite a file stops with exit code 2 and says so.
- **`REMODEL_ACTOR`** is an environment variable. A writing verb with it
  unset stops with exit code 2.
- **The outcomes of `asset make`** are those of step 4 in
  [generation](generation.md): `repaired` is a pass that needed a stated
  repair, `refit` is an image whose subject does not fit the original's
  cut-out and must be made again, and `WHY` in `backend:WHY` is `refused`,
  `timeout`, `rate-limited` or `bad-output` (that page lists the four
  without the prefix). A `refused` call is retried
  once with the prompt lint's wording before it is reported.
- **A rejected asset** returns to `drafted` when a new draft is written
  with `--replace`. Its reason stays in `asset show` and in the bundle.
- **A region's art** is drafted like any asset: `asset normalise NAME
  FILE` writes `draft/<name>.png`, and approval copies it to `art/`.
- **A box** is `[x, y, w, h]` in game coordinates. `region capture`
  widens the box given to whole 8-pixel cells and reports the result.
- **Finding something on screen** with no box given: `scene list --tag`
  to choose a scene, `scene sheet --label layouts` for boxes and names
  as data, then `scene what` on a box.

This table is hand-written because nothing is built. Once slice A
exists it is generated from the code (`remodel help --json`), and the
cold read becomes a repeatable test of that output.

## What each command needs

| Needs | Commands |
|---|---|
| The binary and the project only | `asset list/show/set/name/normalise/check`, `project *`, `pack build --archive`, `pack resolve`, `pack check --level asset` and `--level pack`, `region list` |
| The catalogue as well | `asset bundle`, `asset same --suggest`, `asset show` on a pattern, `glyphs *` |
| The scene library as well (and a GPU) | `scene list/render/what/where/sheet`, `region capture/check`, `pack check --level scene`, `pack diff`, `look *` |
| The ROM as well | `scene make/verify/clip`, `pack build` (it bakes recipes), `pack check --level clip` |

No command runs the game unless its name says so. A command that lacks a
scene or a catalogue stops with exit code 3 and a `next` line. Whoever
holds the ROM (the pack lead, the tester, or the player's first run)
makes the scene library once; art agents and artists then read it.

## Where things live

| What | Where | Shared? |
|---|---|---|
| A game's common scene recipes | Its manifest in `crates/romset/src/games/` | In the repository |
| A project's own scene recipes, and which are `required` with what `coverage` | The project's `scenes.txt` | With the project |
| Scene copies: capture, state, facts | `<data dir>/<set>/scenes/<name>/` | Never; one library per game, used by every project |
| The catalogue | `<data dir>/<set>/catalogue/` | Never |
| Approved art, by slug | The project's `art/` | Yes |
| Work in progress | The project's `draft/<slug>.png` | With the project |
| Region definitions and their art | The project's `regions/<name>.txt` and `art/<name>.png` | Yes |
| Provenance, written by the tools | The project's `provenance/<slug>.txt` | Yes |
| Bundles, originals, raw generator output | The project's `cache/` | Never |
| Gate reports | The project's `reports/<date>-<commit>/` | Yes; they hold numbers and paths, and sheets stay in `cache/` |
| Recipes, backends, gate limits, style | `recipes.txt` ([../uplift-studio.md](../uplift-studio.md)), `backends.txt` ([generation](generation.md)), `gates.txt`, `style.txt` | Yes |

A revision is a commit of the project folder. `pack diff REV` takes a
commit.

## The status of an asset

| From | To | By |
|---|---|---|
| `todo` | `drafted` | `asset make` whenever it writes a draft, or `asset normalise`. Either writes `draft/<slug>.png` and its provenance |
| `drafted` | `checked` | `asset check` with no gate at `error`. Warnings are listed and do not block. `asset make` runs the same gates itself, so a draft it makes with outcome `ok` is `checked` at once |
| `rejected` | `drafted` | A new draft written with `--replace` |
| `checked` | `approved` | `project approve`, by a name listed under `approvers` in `project.txt`. It copies the draft to `art/` and records its hash. Any `review` items must be accepted in the same call with `--accept "reason"` |
| any | `rejected` | `project reject --reason` |
| `approved` | `stale` | Found by any command: the file's hash is no longer the approved one |

The actor is the `REMODEL_ACTOR` name, recorded in `log.tsv`. An agent's
brief names it, and `approve` refuses a name that is not an approver.
This is a guard against mistakes, not security.

Each writing verb takes a short lock on the project, changes one row of
`assets.tsv` and appends to `log.tsv`. Rows are one per asset and sorted,
so two agents on different assets merge cleanly in git.

`asset set ID KEY=VALUE` changes a row's `category`, `colour`, `draw`
(`selflit`, `filter=nearest`) or a recipe setting for that asset
(`recipe.seamless=8`). Nothing is edited by hand, and `pack build` writes
the pack's `material` lines from those columns.

## Three tasks, written out

Slugs, scenes and numbers below are examples.

### A. Restyle the wall base

The brief gives the asset. It tiles sideways and has six colour variants;
the pack lead has set `colour=baked`: the build makes the other five
colourings from the one image
([asset-identity](asset-identity.md), "Packs can name the artwork").

```sh
remodel project brief --json          # my assets, scenes, budgets
remodel asset bundle wall-base --json
```
```json
{"ok":true,"data":{"id":"art:...","name":"wall-base","size":[128,128],
 "wrap":["repeat","clamp"],"variants":6,"colour":"baked",
 "flags":[],"category":"wallbase","scenes":["course-start","course-sand"],
 "gates":["file","alpha","stale","seam","changed","noise"],
 "suggest":"recipe","folder":"cache/bundles/wall-base/"},
 "next":"remodel asset make wall-base --with recipe"}
```

`suggest` is `recipe` because the asset tiles and is not flagged as a
picture: recipes are the `recipe` backend.

```sh
remodel asset make wall-base --with recipe --json
```
```json
{"ok":true,"data":{"outcome":"ok","draft":"draft/wall-base.png",
 "status":"checked",
 "gates":[{"gate":"seam","value":14.2,"limit":12,"verdict":"warn",
   "fix":"asset set wall-base recipe.seamless=8, then make again"}]}}
```

`seam` is a warning in the first set of gates, so the draft is `checked`
and the warning is listed
([quality-gates](quality-gates.md), "Asset gates"). The agent chooses to
fix it. `asset make` runs the asset gates itself, so a separate `asset
check` is needed only for a file made by hand.

```sh
remodel asset set wall-base recipe.seamless=8
remodel asset make wall-base --with recipe --replace --json   # no warning
remodel scene render course-start --asset wall-base --against original --json
```
```json
{"ok":true,"data":{"picture":"cache/renders/course-start-wall-base.png",
 "changed_share":0.21,
 "assets":[{"name":"wall-base","drawn":"replaced","variants_drawn":4,
            "misses":0}]}}
```

It stops there: the asset is `checked`. Six calls and no pictures read.

### B. Replace the sponsor board

The brief gives the scene and the box, because finding "the sponsor
board" is a judgement the pack lead makes from one labelled sheet:

```sh
remodel scene sheet char-select --label layouts     # pack lead, once
```

The art agent then:

```sh
remodel region capture char-select 168,116,166,72 --name sponsor-board --json
```
```json
{"ok":true,"data":{"name":"sponsor-board","cells":[21,10],
 "size_px":[168,80],"anchors":14,"clear":"auto","cleared_cells":9,
 "shared_patterns":[{"pattern":"pat:...","used_in":6}],
 "original":"cache/regions/sponsor-board.png"},
 "next":"remodel asset normalise sponsor-board FILE"}
```
```sh
remodel asset normalise sponsor-board ~/work/logo-kelpjack.png --json
remodel asset check sponsor-board --json      # drafted -> checked
remodel region check sponsor-board --json     # matches on which scenes
remodel scene render char-select --asset sponsor-board --json
```

Five calls and at most one picture. The shared fill tiles are reported,
and are replaced only inside the region.

### C. Is the pack done?

```sh
remodel pack check --json
```
```json
{"ok":false,"code":1,"data":{"done":false,
 "assets":{"approved":281,"checked":14,"drafted":6,"todo":2,"stale":1},
 "errors":[{"asset":"lane-red","gate":"alpha","value":0.034,"limit":0.02}],
 "review":[{"asset":"cap-letter","gate":"text","found":"K"}],
 "scenes":[{"scene":"course-sand","coverage":0.91,"target":0.95,
            "largest_unreplaced":["stone-wall-low","cliff-top"]}],
 "skipped":[{"level":"clip","why":"needs the ROM"}]},
 "next":"remodel project status --status todo,stale"}
```

One call. Levels that cannot run (no ROM, no scene library, no GPU) are
listed as skipped, not failed.

## Budgets for one task

| Task | Calls | Pictures read |
|---|---|---|
| One 3D asset, from brief to `checked` | 6 | 0, plus 1 per failed attempt |
| One region | 5 | 1 |
| "Is it done" | 1 | 0 |

## Adding a kind of uplift

A tool engineer adding, say, real transparency in place of the
checkerboard touches these, in order:

1. **The rule's home.** A column in `assets.tsv` and a key on the pack's
   `draw` line, or a new file kind. A new pack feature name, so older
   readers skip it under `uses` and refuse it under `needs`.
2. **Both renderers**, software and GPU, with off being byte for byte
   today's picture.
3. **What it does without its data.** Which games lack the side channel
   it needs, and the stated fall-back (usually: the original picture).
4. **Per-game data**, if any, as a table in the game's manifest, read
   generically.
5. **An overlay class**, so it can be seen, and its share in `--json`.
6. **Gates**: what can go wrong, as a number.
7. **Tests** without ROMs, and a state-hash check with ROMs.
8. **The browser**: shown there or not, and the pack budget it costs.
9. **Docs**: the format table in `docs/texture-packs.md`, a line in
   `docs/usage.md`, and a `make` target.
