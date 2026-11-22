# The pack as a project: what is not built

Shared words, roles and rules: [README.md](README.md). The manifest, the
status and provenance files, `project init/status/approve/reject`,
`pack build`, layers and `pack resolve` are built (2026-11-23, roadmap item 6)
and are described in [../uplift-studio.md](../uplift-studio.md), "Projects",
and [../texture-packs.md](../texture-packs.md), "Layers". This page keeps
what is not built, and the two other specs that live here (items 26 and 29).
The pack file format that exists today is in
[../texture-packs.md](../texture-packs.md).

## What the first slice did differently from the design

- **The folder is the studio workspace**, not a new tree: images stay in
  `draft/` and `pack/` (the design said `art/`), and the manifest is
  `assets.tsv` beside `selection.tsv`, so every `studio` command keeps
  working on a project. `project.txt` is tab-separated as it was (the design
  showed `key = value`).
- **Status is derived** from the files and rewritten into `assets.tsv`, not
  set by verbs on the manifest: there is nothing to fall out of step.
- **`cache/`** is `original/`, `map/` and `catalogue-maps/`, which keep their
  names because the studio reads them there; `.gitignore` lists them.
- **`pack build`** builds the approved images; it does not bake recipes
  (`studio style` and `studio bake` do) and writes no archive.

## Not built

- **`project brief`**: one agent's task as data (its assets, scenes, verbs and
  budgets). **`project review`**: one sheet per category of everything
  `checked`, original beside new, and the `review` verdict with recorded
  decisions (`approve --accept "reason"`), which asset gates do not yet use.
- **A project's list of required scenes** with a coverage target each
  (`scenes.txt`); `pack check --release` uses the default 10% for every scene.
  **`pack report`**.
- **`pack build --archive`**: the file to publish (the project's `cache/`
  left out), and the player's install and bake steps.
- **`pack diff REV`**: before and after sheets per scene between two commits;
  **`pack doctor`**: assets that no longer match anything the catalogue has
  seen, scenes gone stale, gates that changed verdict; **`pack rekey`** for a
  new key scheme.
- **Names and artwork links** ([asset-identity.md](asset-identity.md)): the
  `name` column of `assets.tsv` is empty until `asset name` exists; no
  same-as links, flat or shared flags, ROM address anchor; `asset list` and
  `asset show` over `status/artworks.tsv`; `follow` as a colour mode.
- **Recipes carried in a pack**: baking on the user's machine from their ROM.
- **Layers in the player and the browser** (one pack there today), a layer
  switched off by the user, region sets merged across layers, and the
  player refusing an order that breaks `depends` or `conflicts` (the CLI does).
- **`asset set` / `asset make`**, a rejected-to-drafted note in `asset show`,
  `restyle` or `faithful` recorded in `project.txt` (the style gates apply
  only to a faithful pack, [quality-gates.md](quality-gates.md)), a signed
  archive.
- **A remaining weakness**: `stale` is found from a hash recorded by
  `studio approve` or `project init`; an approval written by hand into
  `pack/` before `project init` is trusted as it stands.

## Packs in linked play and size tiers

### Packs in linked play

Read from the code at `541d8ac` on 2026-11-20 (linked play, 0.12.0);
nothing was run. Citations in this section are at that commit, not at
`7d8f099`.

- **Nothing a pack or a look does reaches the link.** Machines exchange
  each node's link record and nothing else
  (`crates/model2/src/link.rs:120-131`). The state hash covers RAM, the
  geometrizer's command stream and the coprocessor's results
  (`crates/model2/src/bus.rs:1566-1576`); no renderer or pack state is
  in it. The settings carried from the master are the game's own
  (`crates/cli/src/link_net.rs:211-225`).
- **Over a LAN, each player already has their own pack and look**: each
  host runs one machine and one renderer.
- **On the split screen, packs are off.** `sync_pack` loads one pack
  into one renderer and only when no link is active
  (`crates/play/src/main.rs:2292`). Each view has a renderer of its own
  (`crates/play/src/draw.rs:52-63`) and the look is copied to all of
  them.
- **The change for a pack per view** is in the player only: pack state
  per view, `sync_pack` applied per node, and a per-node setting. With
  it the split screen can show one node as the game drew it and one
  with a pack, live. The pack budget is per renderer, so each packed
  view can use up to the full 512 MiB
  (`crates/gpu/src/lib.rs:330`).
- **One setting is not draw-only**: the choice of chip or native
  geometrizer changes what produces the command stream, which is in the
  hash. Whether the two give equal streams was not determined.
- **The test**: the same linked session, same inputs, with and without
  a pack, comparing each node's state hash and the combined hash. It
  needs the player change first, since `link-run` and `link-net` take
  no pack.

### Size tiers and compressed images

Proposed on 2026-11-20; nothing here is tried. What a pack costs is its
decoded size, not its download: the showcase is 18.8 MB on disk and 197
MB decoded, over the browser's 192 MB budget.

- **A half-size tier, made by the build.** `pack build --tier half`
  writes a second set of images at half the scale. The loader takes the
  tier that fits the budget it has. A quarter of the pixels is about a
  quarter of the memory (arithmetic, not measured).
- **Images compressed for the GPU.** A block format such as BC7 holds a
  pixel in one byte where a decoded PNG takes four, and is not decoded
  on load, which is where most of a reload's time went on a large image
  ([authoring.md](authoring.md), "Evidence"). The build would write
  them; a pack still ships PNGs as its source.
- **What is not known.** What flat cel art with hard ink lines looks
  like after block compression. Which formats the browser's GPU offers,
  since they differ by device and a transcoding step would be a new
  dependency. Whether the build should make these on the user's machine
  or the author's.
- **First step.** The showcase pack built both ways, with decoded
  memory, load time and a difference picture for ten images.

### Size tiers and compressed images

Proposed on 2026-11-20; nothing here is tried. What a pack costs is its
decoded size, not its download: the showcase is 18.8 MB on disk and 197
MB decoded, over the browser's 192 MB budget.

- **A half-size tier, made by the build.** `pack build --tier half`
  writes a second set of images at half the scale. The loader takes the
  tier that fits the budget it has. A quarter of the pixels is about a
  quarter of the memory (arithmetic, not measured).
- **Images compressed for the GPU.** A block format such as BC7 holds a
  pixel in one byte where a decoded PNG takes four, and is not decoded
  on load, which is where most of a reload's time went on a large image
  ([authoring.md](authoring.md), "Evidence"). The build would write
  them; a pack still ships PNGs as its source.
- **What is not known.** What flat cel art with hard ink lines looks
  like after block compression. Which formats the browser's GPU offers,
  since they differ by device and a transcoding step would be a new
  dependency. Whether the build should make these on the user's machine
  or the author's.
- **First step.** The showcase pack built both ways, with decoded
  memory, load time and a difference picture for ten images.

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
  code that runs on the CPU. A shader pack is the exception that is code:
  WGSL run on the GPU, which a loop can hang
  ([shader-packs.md](shader-packs.md), "Safety"; the unpacking limits are
  in [distribution.md](distribution.md), "Installing").

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

## Open questions

- **Licence.** Optional free text, shown where a pack is listed. No default and no check is proposed.
- **Signatures** for archives: later, or not at all?
- **One repository per pack**, or many packs in one?
- **Restyle or faithful**: where it is recorded (`project.txt` or `gates.txt`).
