# Uplift specs: shared ground

These specs describe how reModel could let people and agents restyle and
improve a game's look. Only the first slice of
[scenes-and-server.md](scenes-and-server.md) is built (and the shared answer
every new command uses, `crates/cli/src/answer.rs`), and the first part of
[quality-gates.md](quality-gates.md) (asset gates and `pack check`), and the first part of
[asset-identity.md](asset-identity.md) (artworks in packs), and the project part of
[pack-project.md](pack-project.md) (manifest, provenance, layers); parts of the rest were
tried in spikes, on branches that are not for merging. The vision, the roadmap and the
order of work are in [../uplift-vision.md](../uplift-vision.md); what exists
today is in [../uplift-studio.md](../uplift-studio.md),
[../texture-packs.md](../texture-packs.md) and
[../enhancements.md](../enhancements.md).

This page holds what every spec shares: the words, the people, the rules
for interfaces, and the template. Read it before any one spec.

| Spec | Covers |
|---|---|
| [interface.md](interface.md) | Every verb, where files live, an asset's status, and three tasks written out |
| [scenes-and-server.md](scenes-and-server.md) | Named scenes, drawing them without the emulator, live reload, the long-running server |
| [asset-identity.md](asset-identity.md) | One identity per artwork, its variants, and the context bundle for each asset |
| [quality-gates.md](quality-gates.md) | Checks that give numbers, and what "done" means |
| [pack-project.md](pack-project.md) | The pack as a versioned project: manifest, status, provenance, layers |
| [2d-cells.md](2d-cells.md) | Tiles, regions and text at cell level |
| [renderer-controls.md](renderer-controls.md) | What a pack can tell the renderer, and the views that show it |
| [generation.md](generation.md) | An adapter for any image model, with guarantees made by post-steps |
| [agent-environment.md](agent-environment.md) | Locks, worktrees, binaries and tools, so work does not queue |
| [content-coverage.md](content-coverage.md) | Reaching all of a game: tours, ROM read maps, the crawler, gaps |
| [distribution.md](distribution.md) | Finding and installing packs by a convention on GitHub, with no curated list |
| [scene-understanding.md](scene-understanding.md) | Model identity per pixel, the buffers every feature shares, space beyond one frame, classes |
| [model-interchange.md](model-interchange.md) | Replacement models out and back through glTF |
| [lighting.md](lighting.md) | Occlusion, shadows, real transparency, added lights |
| [added-elements.md](added-elements.md) | Grass, props, decals and weather that stay put |
| [authoring.md](authoring.md) | The Studio page in the player: the path with no terminal |
| [shader-packs.md](shader-packs.md) | Shaders that apply to any game, in the spirit of RetroArch's |

The first ten specs after the interface page are pass one: what
unblocks the work now. The last six are pass two: what uplift could do
next. Eleven of the specs have had something tried, and say what in
their own "Evidence" section.


## Words

- **Asset**: one thing a pack can replace or tag. Kinds: `art` (a 3D
  texture's artwork), `tile` (an 8x8 pattern), `layout` (an arrangement of
  tiles), `model` (a drawn object), later `light` and `scatter`.
- **Artwork** and **variant**: the artwork is the texels; a variant is one
  way the game colours them. Today's pack key names a variant
  ([asset-identity.md](asset-identity.md)).
- **Scene**: a named moment of a game, kept as a recipe (game, input script,
  frame) plus what was drawn there ([scenes-and-server.md](scenes-and-server.md)).
- **Capture**: the renderer's inputs for one frame, saved so the frame can
  be drawn again with no emulator (`crates/model2/src/capture.rs:1-30`).
- **Gate**: a check with a number and a threshold
  ([quality-gates.md](quality-gates.md)).
- **Pack** and **project**: the pack is what a player loads; the project is
  the pack plus its status, provenance, scenes and reports
  ([pack-project.md](pack-project.md)).
- **Draw side**: everything after the machine has produced a frame. All of
  uplift lives here.

## Who it is for

Six roles. One person or agent may hold several. Each spec walks the roles
it touches through a task and says what they judge it on.

| Role | Does | Judges a spec on |
|---|---|---|
| **Pack lead** (a person or a top-tier agent) | Sets the style sheet and brand list, assigns work, approves | Can they see status and review a change without rebuilding context? |
| **Art agent** (a mid-tier model) | One asset or one batch, from a bundle | Can they finish without a build, without playing the game and without reading reModel's code? |
| **Artist** (a person, no terminal) | Edits in Krita, Photoshop or Blender | Is it seconds from saving a file to seeing it in the scene, in motion? |
| **Tool engineer** | Adds a kind of uplift to the framework | Does it work for every game, can it be tested, is it draw-only? |
| **Tester** (a person or an agent) | Runs gates, reads reports, signs off | Is "done" a number, and is the emulation provably untouched? |
| **Player** | Installs and layers packs | Does it hold frame rate, native and in the browser, and can it be switched off? |

Only one person works on reModel today. The artist and the player are
strangers who will arrive with a released build and their own ROMs: no
Rust toolchain, no checkout, perhaps no image model. A spec that needs
this machine's layout to work has failed them.

The art agent and the tester are drawn from the record of the Top Skater
showcase (14 agents, about 2,300 tool calls). The others are assumptions
until real people use the tools.

## Rules for every interface

These exist so a mid-tier model can do the work reliably, and they make the
tools better for people too.

1. **One asset is one job.** An asset and its bundle are a complete unit of
   work that fits a small context. No job needs the whole catalogue.
2. **One set of verbs, the same everywhere.** The CLI, the server and the
   agent API share names and fields. An art agent's job uses eleven
   verbs ([interface.md](interface.md)).
3. **Structured output.** Every command takes `--json` and prints one
   object (written with a JSON library, not by hand): `ok`, `code`,
   `error`, `data`, `warnings`, `next` (a suggested next command).
   Tables stay TSV. Prose reports are for people and are made from the
   same data.
4. **Exit codes mean something.** 0 done, 1 a gate failed, 2 the request
   was wrong, 3 something needed is missing (ROM, scene, capture), 4 an
   internal error.
5. **Strict names, no silent work.** An unknown name, key or option is an
   error that lists near matches. Nothing overwrites a draft or an approved
   image without `--replace`. Nothing reuses old output without saying so.
6. **Idempotent.** Running a command twice gives the same result and the
   same files.
7. **Numbers before pictures.** A check answers with numbers. A picture is
   offered only when a number fails, as one small labelled sheet.
8. **State lives in the project.** What was done, by what, and what is left
   is in the project's files, never only in a report or a conversation.
9. **Limits are enforced by the tool**, not by guidance a model may not
   have read.
10. **Judgement goes up a tier.** Style, brands and approval belong to the
    pack lead. Each spec says which steps a mid-tier model should not be
    trusted with.

Each spec gives a budget for the art agent: tool calls, pictures read and
wall time per asset. The baseline to beat is the showcase: 20 to 57
pictures read per agent, and checks that booted the game each time.

## Constraints

Every design here is checked against these.

- **Draw-only.** No uplift changes emulation state, state hashes, save
  states, run-ahead or replays. It copes with in-between frames and with
  30 Hz games.
- **Original by default.** Everything is opt-in, and off is bit-for-bit the
  original picture.
- **Every game.** Generic code reads per-game data from
  `crates/romset/src/games/` and never tests a game's identity. The
  `Shading` table is the pattern to follow.
- **Clean-room.** No other emulator's code is consulted. Nothing derived
  from ROMs enters the repository.
- **Native first.** The brief asked for native and browser alike. Decided
  2026-11-19: uplift tooling stays out of the browser unless a piece is
  nearly free there. Packs still draw in the browser through the shared
  renderer where they do today, within its 192 MB pack budget against
  512 MB native (`crates/gpu/src/lib.rs:325-331`). No design here should
  rule the browser out later.
- **One game per pack.** Keys, scenes and layers are per game. Only looks
  without tags, and shader packs, cross games.
- **Not every game has every side channel.** Per-polygon normals, lighting
  entries and matched motion need a native geometrizer. The chip path has
  none, and `zerogun` and `pltkids` have no native geometrizer. A feature
  that needs them says what it does without them.
- **Original art only** in what reModel itself ships, such as the
  showcase pack, with invented brands in place of real ones. What other
  authors share is their business (decided 2026-11-19).
- **Three games.** Every slice is proven on Top Skater and `vf2`, and
  `daytona` joins wherever the board could change the answer, so nothing
  hardens around one title.

## How firm each spec is

These pages were written ahead of the code, and will go stale. Five
rules keep that from mattering.

1. **Citations are evidence, not upkeep.** Each `path:line` was true at
   commit `7d8f099`. Nobody maintains them, and reModel has already
   moved on (linked play, 0.12.0). The day a slice starts, its spec's
   "Observed" list is checked again against the code, by an agent,
   before anything is built on it.
2. **A spec carries a status**, and nothing is built on a sketch:

   | Status | Means |
   |---|---|
   | Sketch | Reasoned from code and documents. Its risky assumptions are untried |
   | Grounded | Its claims about today's code were checked by a second reader |
   | Spiked | Its riskiest assumption was tried in code, and the result is in the spec's "Evidence" section with its date and branch |
   | Built | Gone: the spec is deleted and what is true moves to the user documentation |

3. **Only the next slice is a commitment.** Later specs are a menu, and
   are expected to change when the slice before them lands.
4. **Each slice names its acceptance test first**, and the test is run
   on the games above and by an agent on a mid-tier model that has read only
   [interface.md](interface.md), with its calls and pictures counted.
5. **Whoever builds a slice deletes its spec in the same commit**, and
   moves what is true into the user documentation. A spec left beside
   the code it described is the stale page these rules guard against.

Where each spec stands is in the vision's table of spikes
([../uplift-vision.md](../uplift-vision.md), "Spikes").

## Template

Each spec has: the problem (with evidence), the design, interfaces (CLI,
files, agent API), data formats, the roles walked through it, how to test
it, performance, open questions, and an MVP slice of a few days' work.

A spec that has been spiked also has an "Evidence" section before its
open questions: one numbered entry (E1, E2, ...) per spike, with its
date, what was run, the results, and what was not tested. The design
states the outcome and points to the entry by its tag. The design is
what to build; the evidence is why.

Claims about today's behaviour cite code as `path:line` at commit
`7d8f099`, or a run. **Observed** means read in the code or measured;
**inferred** means a conclusion. Prior art from other projects was read
from their documentation only, and is marked unverified where the page
could not be read in full.
