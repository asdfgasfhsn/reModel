# Uplift: vision and roadmap

How reModel can let anyone, person or agent, restyle and improve a
Model 2 game: its textures, 2D art, models, lighting and the scene itself.
This page is the vision, the map of capabilities and the order of work.
The specs are in [uplift/](uplift/README.md).

Status, 2026-11-19: a design, written from the code at `7d8f099`, the
record of the Top Skater showcase, and a survey of other projects'
documentation. Nothing described here is built unless it says so. Pass
one (this page and nine spec pages) covers what unblocks the work now. Pass
two (model interchange, scene understanding, lighting, added elements,
authoring) is outlined under "Horizon" and not yet specified.

## The outcome

reModel ships as one binary. With it and their own ROMs:

- **An artist** who has never opened a terminal sees every asset a game
  has and where it appears, pulls one out as an ordinary image, edits it
  in their own tools, and sees it in the running game within a second.
- **An agent**, including a mid-tier model, does the same through a small
  set of verbs: eight for an art job. It is handed one asset and everything about it, makes or
  edits the art, and learns from numbers whether it is right. It never
  builds reModel or plays the game.
- **A pack lead** sees what is done, what failed and what needs a
  decision, and reviews a revision as before-and-after sheets.
- **A tool engineer** adds a kind of uplift once, for every game, with
  tests, and cannot disturb the emulation.
- **A player** installs a small archive, stacks packs, and switches any
  of them off.

A pack is a project: versioned, reviewable, with a status, a source and
an approval for every asset.

**What pass one does not reach.** The artist's path. Every verb specified
so far is a command, and the page in the player that would wrap them
(pick a scene, export an asset to edit, see it reload, build the pack) is
named but not designed. An independent read of these specs found the
artist the worst-served role. Item 8 below is the start, and the
authoring spec leads pass two.

## What the investigation changed

The brief this work started from listed eleven wishes. Most stand. These
findings change their shape or order.

1. **Still checks need no server and no emulator.** A captured frame can
   already be drawn again without the game. Measured: one Top Skater
   scene at 1080p with the full showcase pack, cold, in 1.75 s. What is
   missing is named scenes and one command. The long-running server is
   for motion and for sharing a GPU, and comes later.
2. **The lock that everything queued behind is not a mechanism.** Nothing
   in the Makefile or `tools/` takes it; agents type it by hand. Art
   checks waited behind builds because they shared a convention, not
   because they are heavy.
3. **The stable identity already exists.** The renderer computes a
   colour-free surface key beside every pack key. The work is letting a
   pack name it, and deciding how one image meets several colours. In Top
   Skater 565 keys are 417 artworks, and the repeats cover 49% of drawn
   pixels.
4. **Silent failure cost as much as slowness.** A stray argument restyled
   4,204 rows and overwrote drafts. A compare run reused old frames.
   Checks that find problems exit 0. The interface rules in
   [uplift/README.md](uplift/README.md) come from this.
5. **Image generation was a small part of the showcase.** Of 6,496
   entries in its ledger, 30 came from a model and 5,591 from local
   recipes; the rest are typeset text, variants and imports. The batch
   runner was mostly bypassed for hand scripts. The adapter is
   worth having, and recipes are the workhorse.
6. **Image tools belong in the binary**, not in a container. A container
   helps one machine; `remodel img` helps every user.
7. **The renderer can do more than the showcase's "can't do" list says,
   and knows less than `docs/enhancements.md` suggests.** Smooth shading,
   normal maps lit by the game's light, bloom, a depth buffer, temporal
   anti-aliasing and ink outlines exist. But there is no camera or world
   space anywhere: the game hands over geometry already in view space
   (`crates/model2/src/photo.rs:13-17`). Model identity is on the CPU
   side only. That makes placing new things
   in a scene the hard part of the horizon.
8. **2D has no prior art to copy.** No emulator surveyed replaces composed
   2D screens. reModel's map-space regions are ahead; the next step is
   colour-free patterns and live text fields.

## Map of capabilities

| Layer | Capability | Spec | Exists today |
|---|---|---|---|
| **Find** | Every asset, where it shows | [asset-identity](uplift/asset-identity.md) | Catalogue, texmap |
| | Named scenes | [scenes-and-server](uplift/scenes-and-server.md) | Save states and captures, unnamed |
| **Understand** | One identity per artwork; context bundle | [asset-identity](uplift/asset-identity.md) | Surface key; no bundle |
| **Make** | Recipes; any image model, with guarantees | [generation](uplift/generation.md) | `studio style`, `studio draft` |
| | 2D: patterns, regions that fit, text fields | [2d-cells](uplift/2d-cells.md) | Tile images, regions |
| | Draw rules: as painted, filtering, tags by category | [renderer-controls](uplift/renderer-controls.md) | Look tags per texture |
| **See** | Draw a scene in seconds; live in the player; overlays | [scenes-and-server](uplift/scenes-and-server.md), [renderer-controls](uplift/renderer-controls.md) | `look-render` on captures |
| **Check** | Gates with numbers; "done" defined | [quality-gates](uplift/quality-gates.md) | Seam, lint, flicker, each separate |
| **Keep** | Manifest, status, provenance, history | [pack-project](uplift/pack-project.md) | Loose files |
| **Share** | Build, layers, archive, discovery | [pack-project](uplift/pack-project.md) | One pack; plans only |
| **Work** | Locks by resource, worktrees, pinned binary | [agent-environment](uplift/agent-environment.md) | A convention |
| **Horizon** | Models, scene knowledge, lighting, added elements, authoring | Pass two | Parts: see below |

## Roadmap

Ranked by how much each unblocks, with the first step named. Effort: S is
days, M is one to two weeks, L is more. The first column marks what helps
agents soonest.

| Soon | Item | Value | Effort | Risk | Needs | First step |
|---|---|---|---|---|---|---|
| ★ | **1. Scenes and `scene render`** | Every still check without boot, build or lock | S | Low | Nothing | Five scenes for Top Skater, three for a game on another board |
| ★ | **2. Lock tool, worktree set-up, pinned binary, `remodel img`** | Ends the queue; no shell image scripts | S | Low | Nothing | `tools/lock` wired into the Makefile |
| ★ | **3. Interface rules on today's tools, and the three defects** | No silent work; output a model can parse | S to M | Low | Nothing | `--json`, exit codes, refuse to reuse or overwrite |
| ★ | **4. Asset gates and `pack check`** | "Done" is a number | M | Medium: limits need calibrating | 1 for scene coverage | Run the gates over the showcase's images |
| | **5. Artwork identity in packs** | A pack replaces "the wall base" | M | Medium; `follow` tint is high | Nothing | `art_` images, fixed and baked; then the tint spike |
| | **6. Project manifest and provenance** | Work survives hand-over | M | Low | 5 for names | Import the showcase workspace |
| | **7. Context bundles** | An agent's whole briefing in one call | M | Low | 1, 5 | `bundle.json`, original, mask, two crops |
| | **8. Player: `--scene`, pack watch, then a pack page** | The artist's loop, and the first step away from the terminal | S to M, then L | Low, then unknown | 1 | Reload on change, as the look does |
| | **9. 2D: patterns, anchors, clearing** | HUDs and logos without hand patches | M | Medium | 1 for capturing from scenes | Pattern keys; redo the showcase's 13 regions |
| | **10. Draw rules and overlays** | Colours as painted; see what a tag did | M | Low | 1 | `selflit`; `--overlay miss` |
| | **11. Generation adapter** | Size, alpha, seams guaranteed | M | Low | 4, 7 | Normalise and classify around `studio draft` |
| | **12. Text fields** | Live scores and clocks in a pack font; Japanese HUDs | L | Medium to high | 9 | The score in Top Skater |
| | **13. Server and clips** | Motion checks; many agents on one GPU | M to L | Medium: GPU sharing untested | 1 | MCP over a local socket, one verb |
| | **14. Layers, archive, `pack doctor`** | Sharing | M | Low to medium | 6 | Two packs layered in the CLI |
| | **15. Look calibration** | A look that fits each game | M | Medium: targets unproven | 1, 10 | Numbers for four games |

### Suggested order

- **Slice A: stop waiting.** Items 1, 2 and 3. Independent of each other;
  three small pieces of work. After it, an art agent can check a still in
  two seconds with a prebuilt binary and no lock.
- **Slice B: know what you have.** Items 4, 5 and 6. The asset gates need
  nothing but scenes and can start at once; identity comes before the
  manifest, which names assets by it. Item 5 ships `fixed` and `baked`
  first; `follow` waits for its spike.
- **Slice C: the loop.** Items 7, 8 and 10. The bundle for agents, the
  live player for the artist, overlays for both.
- **Slice D: the hard art.** Items 9 and 11, then 12.
- **Slice E: scale and share.** Items 13, 14 and 15.

Redoing part of the showcase after each slice is the test: the same
assets, with calls, pictures read and wall time counted.

### What it should cost an agent

| Per asset | Showcase | Target |
|---|---|---|
| Time to see a change in the scene | About 40 s a side from boot, plus 7 to 20 minutes queued | 2 s |
| Pictures read | 20 to 57 per agent | None on a pass; one sheet on a failure |
| Calls to understand an asset | Many: montages, TSVs, frames | 2 |
| Knowing the file was used | By eye | A field in the answer |
| Hand-over | A prose report | The project's log and status |

## Horizon (pass two, provisional)

Each needs its own spec. What is known so far:

| Idea | Needs | Generic? | Known cost | Smallest proof |
|---|---|---|---|---|
| **Model and window IDs per pixel** | Per-polygon data already on the CPU | Yes, with a native geometrizer | The ID pass: 0.7 ms at 1080p, 3.0 ms at 4K (documented) | Pick a model under the cursor in a scene |
| **Camera and world space** | Recovering one camera from per-object view matrices | Unknown; a spike exists and is unconfirmed (`docs/photo-mode.md`, "A true camera") | Not measured | Hold a point still on the ground across 120 frames |
| **Model replacement (glTF)** | Model identity; the plan exists | Rigid parts only; not geometry rewritten each frame | Not measured | One prop and one character's parts exported and swapped |
| **Shadows from the game's light** | Depth, the light direction (native path) | Largely | Unmeasured; scenes are a few thousand polygons | A shadow map on one scene, multiplied in |
| **Ambient occlusion** | Depth and normals | Yes | Unmeasured | Half-resolution pass on one scene |
| **Real transparency for stipple** | The checker flag per polygon | Yes | Small | Per-asset blend on a fighter's shadow |
| **Added elements (grass, crowds, decals)** | World space, surface classes per game | Rules generic, data per game | Unmeasured | Scatter on one recognised ground, steady in motion |
| **Authoring** | Items 1, 7, 8 and IDs | Yes | None at run time | Point at a thing in the player and open its bundle |
| **Shader packs, for any game** | A stated set of inputs after the frame is composed | Yes: this is the one kind of pack that is not per game | Per pass; the CRT pass is the model | A user-supplied pass in place of the built-in CRT one |

**Shader packs** were added by the user on 2026-11-19: let people bring
shaders that apply to any number of games, as RetroArch does. reModel
already runs passes of this kind after the frame is composed
(`crates/gpu/src/crt.wgsl`, `bloom.wgsl`, `look.wgsl`), so the host
exists. It can also offer a shader more than the finished picture: depth,
the texture ID and the game's lighting layer. Two routes, to be weighed
in pass two: reModel's own shader packs in WGSL against a stated
contract, and importing RetroArch's preset format, which brings a large
existing library and needs its shaders translated.

Two cautions from the survey. Lighting added on top must multiply, as
occlusion does, or it double-counts shading the game has already baked
in; many surfaces are lit to full brightness. And ray tracing in the
browser should not be designed for: it does not exist there yet.

### The browser

Out of scope for uplift tooling unless a piece is nearly free there
(decided 2026-11-19). Packs keep drawing in the browser through the
shared renderer where they do today. Two facts to remember when that
changes: the browser draws no regions and no look yet, and the showcase
pack is over its 192 MB budget.

Audio is out of scope. The one hook it will need from this work is a pack
kind for sound, which the manifest's `kind` column allows.

## Decisions

Taken by the user on 2026-11-19:

| Question | Decision |
|---|---|
| May a shared pack hold images derived from game art? | Not the tools' business. The tools build packs and do not rule on what is shared |
| Agents or the artist first? | Agents first, as the showcase was made. Cutting friction and waste matters more than the artist's path for now |
| JSON output: by hand or a library? | A library, weighed by what it costs to carry. `serde` is already in the dependency tree; `serde_json` is the one addition |
| The server's transport | As agent-native as possible: a local socket and MCP |
| Is the browser an authoring target? | No, unless a piece is nearly free or highly efficient there |
| The three defects found in current code | Fix them with slice A |
| Pass two's order | Model and window IDs per pixel first |

The defects: `studio compare` reusing old frames without `--force`
(`crates/cli/src/studio.rs:1834`); generated drafts not inheriting a
cut-out's alpha (inferred from `finish_draft`, `studio.rs:1761`; not
run); `--keep-size` read at `studio.rs:1730` but missing from the
switches at `studio.rs:84-98`, so it would take the next argument as its
value.

Still open, with a recommendation:

1. **Licence and AI label.** Recommendation: both optional free text in
   the manifest, shown where a pack is listed, with no default and no
   check. The tools only help people make and fetch packs.
2. **Scene lists in the repository.** A recipe is an input script and a
   frame number, the same kind of fact as the `coin-start` scripts and
   attract frames the manifests hold now, so there is no clean-room
   concern. The real costs are upkeep: frame numbers shift when emulation
   timing is corrected and differ between a game's revisions, a scene's
   name becomes something packs depend on, and someone has to choose
   scenes for 34 games. Recommendation: yes, but small and optional. A
   handful per game where someone has done the work, a project's own
   `scenes.txt` always taking precedence, and `scene verify` in the
   benchmark run to catch drift.

## How this was made

- **Code and docs** read at `7d8f099` by four mapping passes, each claim
  cited by file and line. An independent pass then checked 72 citations
  and "does not exist" claims against the code; the 12 it faulted were
  corrected.
- **Review.** Three more independent reads: against the project's
  constraints, as four of the roles, and as an agent given only the specs
  and three tasks. Their findings are folded in;
  [uplift/interface.md](uplift/interface.md) exists because of the last.
- **The showcase record**: 14 agents' reports and timelines.
- **One measurement**: the scene render time above. Every other timing is
  quoted from reModel's docs and marked so.
- **Other projects**: documentation, wikis and write-ups only; no
  emulator source. Several pages were read through summaries or in part,
  and the specs mark those claims unverified.
- **The hardware and games survey was thin.** Its sources blocked
  automated reading, and much of it rests on general knowledge. It
  informed which needs beyond Top Skater to cover (Japanese HUDs,
  crosshairs, stipple, 2D skies) and nothing else. It disagrees with
  `docs/status.md` on three games' boards; the repository stands.
- **Not done**: no thresholds were calibrated, no tint model was tried,
  and nothing was tested on a second game.
