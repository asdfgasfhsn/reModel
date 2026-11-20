# The pack as a project

Shared words, roles and rules: [README.md](README.md). Nothing here is
built. The pack file format that exists today is in
[../texture-packs.md](../texture-packs.md).

## The problem

A pack today is a folder of PNGs with an optional `pack.txt`. The studio
workspace around it is loose files. Work on the showcase was carried
between agents in prose reports and rebuilt by hand-written scripts.

**Observed in the studio code and the showcase workspace.**

- **No manifest for the work.** State is spread over `selection.tsv`,
  `status/status.txt`, `prompts/`, `project.txt`, and recipe and brief
  files kept wherever the author chose, none with a format version
  (`crates/cli/src/studio.rs:353`, `389-404`).
- **Status is where a file sits.** An image in `pack/` is approved, in
  `draft/` drafted, otherwise original (`crates/cli/src/studio.rs:353`).
  There is no rejected state, no approver, and no hash, so a file edited
  after approval is still "approved".
- **Thin provenance.** A draft records the first word of the command, the
  brief's name, the date and a prompt file. No model or tool version, seed
  or input hash.
- **No revisions.** A new draft overwrites the old one. The showcase's
  history is whole-folder copies (`cel-pack2`, `cel-pack4`, a `cel-pack3`
  that is gone) with notes in prose.
- **Not relocatable.** `project.txt` stores the capture folder as an
  absolute path (`crates/cli/src/studio.rs:539-541`); in the showcase it
  points into a worktree that no longer exists, and `map` there is a link
  to a folder outside the workspace.
- **Heavy.** Of `cel-pack4`'s 806 MB, 291 MB is a private copy of the
  `remodel` binary, 219 MB is scratch, and the images exist three times.
- **The look is outside the pack.** `pack-lint` does not know `look.txt`
  as a pack file (inferred from `crates/cli/src/pack_lint.rs:23-31`).
- **One pack at a time.** There is no layering.

What should be kept: the strict names, `select` and `approve` being
idempotent, the recipe file, and the pack format's `uses` and `needs`,
which already separate features a reader may skip from ones it must have.

## Design

### Two things, kept apart

- **The pack** is what a player loads: images, `pack.txt`, `look.txt`,
  `regions.txt`. It is built, never edited by hand.
- **The project** is what people and agents work in. It is a folder of
  text and original art that fits in a git repository, and it builds the
  pack.

Keeping authoring apart from shipping is what the recompilation projects
do: author in loose files, ship something built.

### The project folder

```
project.txt        format, name, game, what it was made against
assets.tsv         one row per asset: the manifest
scenes.txt         the scenes that matter to this pack
style.md           the style sheet, for people and prompts
style.txt          its checkable part: palettes per category, line weights
gates.txt          gate limits changed from the defaults, with reasons
recipes.txt        the local style recipes (today's format)
art/               approved images, named by slug
draft/             work in progress
provenance/        one small text file per image
log.tsv            every status change, appended
reports/           gate reports per revision
build/             the built pack (not committed)
cache/             originals, bundles, sheets (ROM-derived, not committed)
```

Every path is relative. Nothing links outside the folder. No binaries.

`project.txt`:

```ini
format     = 1
name       = Top Skater, cel comic
game       = topskatr
program    = <hash>           # program ROM hash, as save states record it
keys       = 1                # the identity scheme, as pack.txt has it
approvers  = lead
made-with  = reModel 0.12.0
version    = 0.4.0
licence    = CC-BY-4.0        # optional, free text
depends    = other-pack >= 1.2  # optional: a pack this one must sit above
```

`assets.tsv`, one row per asset:

| Column | Holds |
|---|---|
| `id`, `name`, `kind`, `category` | Identity ([asset-identity.md](asset-identity.md)) and the recipe group |
| `status` | `todo`, `drafted`, `checked`, `approved`, `rejected`, `stale` |
| `colour` | `baked`, `fixed` or `follow` |
| `draw` | Draw rules: `selflit`, `filter=nearest` ([renderer-controls.md](renderer-controls.md)) |
| `source` | `hand`, `recipe`, `generated` or `imported` |
| `hash` | The approved image's hash |
| `by`, `date` | Who approved it |
| `note` | A reason, required for `rejected` |

Status is recorded and then verified: if the image's hash is not the
approved one, the asset is `stale`. The table of which verb moves which
status is in [interface.md](interface.md). Rows change only through
verbs (`asset set`, `asset make`, `project approve`), each under a short
lock, one sorted row per asset, so several agents on different assets
merge cleanly in git.

`provenance/<slug>.txt` holds the tool, the model and its version, the
prompt file, the hashes of every input, the post-steps in order, the date,
the author and, if the author wants them, a licence and an AI label
(`none`, `assisted` or `generated`, which some mod sites now ask for).
Neither is required or checked. Prompts and
seeds are a record, not a recipe: image models do not reproduce reliably
across machines.

`log.tsv` is append-only: time, actor, verb, asset, from, to, note. It is
the hand-over between agents, and with git it is the history. A revision
is a commit of the project folder; nothing else defines one.

### One game per pack

A pack belongs to one game: its keys are that game's texels and tiles, and
its scenes are that game's moments. Layers stack within a game. The one
thing that is naturally shared across games is how the finished picture is
treated: a look without per-texture tags, and the shader packs proposed
in [shader-packs.md](shader-packs.md).

### What a pack carries

The tools build packs; they do not rule on what may be shared. That is the
author's business (decided 2026-11-19). A pack can carry its art two ways,
and may mix them:

| Way | Holds | Good for |
|---|---|---|
| **Images** | The finished PNGs | Anything; the simplest |
| **Recipes** | The recipe and its settings, baked on the user's machine from their ROM | Small downloads; art that is a mechanical restyle of the game's own |

`pack build` writes the pack. `pack build --archive` writes the file to
publish. The project's `cache/` is left out because it is working files,
not because of a rule. The rule in `CLAUDE.md` that nothing ROM-derived
enters reModel's own repository is unchanged, and is about this
repository only.

### The shipped manifest

`pack.txt` gains keywords: `version`, `author`, `licence`, `ai`, `program`
(the ROM hashes it was made against, never ROM data), `depends` and
`conflicts`. `look.txt` becomes a pack file. None of this needs a new
`format` number. The format's rule is that `format` rises only when the
meaning of something a reader already understands changes; readers
ignore unknown keywords and unknown names in `uses`, and refuse a pack
that `needs` a feature they cannot show
(`docs/texture-packs.md:273-285`). New abilities here are feature names:
`artworks`, `patterns`, `fields`.

### Layers

The player takes an ordered list of packs. For each asset the highest pack
that has it wins. The renderer already takes a pack as one trait object
(`crates/gpu/src/lib.rs:1249`), so a layered pack is a wrapper around
several directory packs, and the renderer does not change. Each pack can
be switched off. The order is the user's. A pack's `depends` says what
must be present beneath it and `conflicts` what must not be loaded with
it; the player refuses an order that breaks either. `pack resolve` says
which pack won each asset and what it hid, and checks the layers'
combined size against the memory budget, native and browser. It warns
when a layer `uses` something the browser does not show, such as regions
today.

### Surviving new versions

- **Pack format and identity scheme** are versioned, and `pack rekey`
  exists for the second.
- **`pack doctor`** runs after a reModel update: assets that no longer
  match anything the catalogue has seen, scenes that went stale, gates
  that changed verdict.
- **Discovery** stays as planned: a repository with a topic and a release
  archive, no curated list. The client checks the manifest and the
  archive's hash itself, since a topic proves nothing. Packs never contain
  code.

## Interfaces

| Verb | Does |
|---|---|
| `project init` | Makes the folder; `--from-studio DIR` imports a studio workspace |
| `project status` | Counts and lists by status, category and scene |
| `project brief` | One agent's task as data: its assets, scenes, verbs and budgets |
| `project review` | One sheet per category of everything `checked`, original beside new |
| `project approve ID` or `--category C`, `project reject ID --reason` | Change status, after the gates; only for a listed approver |
| `pack build` | Bakes recipes, resolves names, writes the pack; `--archive` makes the file to publish |
| `pack resolve` | Which layer wins each asset |
| `pack diff REV` | Before and after sheets per scene between two revisions |
| `pack doctor` | What a reModel update broke |

## The roles

- **Pack lead.** `project status --json` is the hand-over. `pack diff` is
  the review. Nothing has to be re-derived from a report.
- **Art agent.** Is given asset IDs. It writes `draft/<slug>.png` and its
  provenance, and the log records it. It cannot approve.
- **Artist.** Works in `art/` and `draft/`: ordinary images with readable
  names.
- **Tester.** Approval is tied to a hash, so "approved" means the file
  that was checked.
- **Player.** Installs a small archive, bakes once, stacks packs, and can
  switch each one off. The install and bake steps have no interface yet:
  the page that would do it is in [authoring.md](authoring.md), and until
  then this is a command.
- **Tool engineer.** A new asset kind is a new `kind` and file kind, with
  no new manifest.

## Testing

- **Without ROMs:** parsers; the status machine, including `stale`;
  the archive holding the built pack and nothing from `cache/`; layer resolution; a build
  from recipes over synthetic originals that gives the same bytes twice.
- **With ROMs:** import the showcase workspace, build, and compare with
  the pack it has today.

## Performance

A build bakes recipes: seconds per hundred textures, by the showcase's
figures of 2 to 5 s per local texture in shell scripts and far less in
`studio style`. Layer lookup is per asset on first draw, not per frame.
The built showcase pack is 18.8 MB of PNG files on disk (measured
2026-11-20) and 197 MB once decoded; a shared archive of recipes and
original art would be smaller still. Decoded, it is over the browser's
192 MB budget, so a pack meant for the browser needs smaller images or fewer
colour copies; compressed textures are an open question.

## Open questions

- **Licence.** Optional free text, shown where a pack is listed. No
  default and no check is proposed, since the tools only help people
  make and fetch packs.
- **Signatures** for archives: later, or not at all?
- **Compressed textures** in the built pack (KTX2) would cut memory and
  load time, and need a decoder in the browser build.
- **One repository per pack**, or many packs in one?

## MVP: a few days

1. `project.txt`, `assets.tsv`, `log.tsv` and provenance files, with
   hashes and `stale`.
2. `project init --from-studio` on the showcase.
3. `pack build` writing the new `pack.txt`, with `look.txt` accepted.
4. Two packs layered in the CLI renderer, with `pack resolve`.
