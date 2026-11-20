# Uplift: vision and roadmap

How reModel can let anyone, person or agent, restyle and improve a
Model 2 game: its textures, 2D art, models, lighting and the scene itself.
This page is the vision, the map of capabilities and the order of work.
The specs are in [uplift/](uplift/README.md).

Status, 2026-11-19: a design, written from the code at `7d8f099`, the
record of the Top Skater showcase, and a survey of other projects'
documentation. Nothing described here is built unless it says so. Pass
one (this page and nine spec pages) covers what unblocks the work now. Pass
two (six more specs) covers what uplift could do next.

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
artist the worst-served role. Item 8 below is the start, and
[uplift/authoring.md](uplift/authoring.md) designs the page.

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
| **Know the scene** | Model per pixel, shared buffers, classes | [scene-understanding](uplift/scene-understanding.md) | On the CPU side only |
| **Reshape and add** | Replacement models, added elements | [model-interchange](uplift/model-interchange.md), [added-elements](uplift/added-elements.md) | Planned only |
| **Light** | Occlusion, shadows, real transparency | [lighting](uplift/lighting.md) | Smooth shading, normal maps, bloom |
| **Author** | The Studio page in the player | [authoring](uplift/authoring.md) | Two static viewers |
| **Any game** | Shader packs | [shader-packs](uplift/shader-packs.md) | Built-in CRT, bloom, look |

## Roadmap

Ranked by how much each unblocks, with the first step named. Effort: S is
days, M is one to two weeks, L is more. The first column marks what helps
agents soonest.

| Soon | Item | Value | Effort | Risk | Needs | First step |
|---|---|---|---|---|---|---|
| ★ | **1. Scenes and `scene render`** | Every still check without boot, build or lock | S | Low | Nothing | Five scenes for Top Skater, three for a game on another board |
| ★ | **2. Lock tool, worktree set-up, pinned binary, `remodel img`** | Ends the queue; no shell image scripts | S | Low | Nothing | `tools/lock` wired into the Makefile |
| ★ | **3. Interface rules on today's tools, and the three defects** | No silent work; output a model can parse | S to M | Low | Nothing | `--json`, exit codes, refuse to reuse or overwrite |
| ★ | **22. [Content coverage](uplift/content-coverage.md)**: tours, ROM read maps, `coverage`; then the tour recorder, pokes and the crawler | A catalogue that holds the whole game, and a number for how much it holds | S, then M to L | Medium: the read-map figure is unknown | Nothing; cheats for pokes | The read-map spike on Top Skater's three runs |
| ★ | **4. Asset gates and `pack check`** | "Done" is a number | M | Medium: limits need calibrating | 1 for scene coverage | Run the gates over the showcase's images |
| | **5. Artwork identity in packs** | A pack replaces "the wall base" | M | Medium | Nothing | `art_` images, fixed and baked, with the build making the colour copies |
| | **6. Project manifest and provenance** | Work survives hand-over | M | Low | 5 for names | Import the showcase workspace |
| | **7. Context bundles** | An agent's whole briefing in one call | M | Low | 1, 5 | `bundle.json`, original, mask, two crops |
| | **8. Player: `--scene`, pack watch, then a pack page** | The artist's loop, and the first step away from the terminal | S to M, then L | Low, then unknown | 1 | Reload on change, as the look does |
| | **9. 2D: patterns, anchors, clearing** | HUDs and logos without hand patches | M | Medium | 1 for capturing from scenes | Pattern keys; redo the showcase's 13 regions |
| | **10. Draw rules and overlays** | Colours as painted; see what a tag did | M | Low | 1 | `selflit`; `--overlay miss` |
| | **11. Generation adapter** | Size, alpha, seams guaranteed | M | Low | 4, 7 | Normalise and classify around `studio draft` |
| | **12. Text fields** | Live scores and clocks in a pack font; Japanese HUDs | L | Medium to high | 9 | The score in Top Skater |
| | **13. Server and clips** | Motion checks; many agents on one GPU | M to L | Medium: GPU sharing untested | 1 | MCP over a local socket, one verb |
| | **14. Layers, archive, `pack doctor`** | Sharing | M | Low to medium | 6 | Two packs layered in the CLI |
| | **23. [Distribution](uplift/distribution.md)**: find and install by a GitHub convention | Players find packs; nobody keeps a list | M | Low; adds an HTTP client | 14 | `pack install` from a file, then from a repository |
| | **15. Look calibration** | A look that fits each game | M | Medium: targets unproven | 1, 10 | Numbers for four games |

### Suggested order

- **Slice A: stop waiting.** Items 1, 2 and 3, and the first part of 22
  (tours and read maps), since a scene is a point in a tour. Independent
  of each other; small pieces of work. After it, an art agent can check a
  still in two seconds with a prebuilt binary and no lock, and the pack
  lead knows how much of the game the catalogue holds.
- **Slice B: know what you have.** Items 4, 5 and 6, and the rest of 22
  as the cheats work allows. The asset gates need
  nothing but scenes and can start at once; identity comes before the
  manifest, which names assets by it. Item 5 ships `fixed` and `baked`;
  `follow` was spiked and is now only a fallback for unseen colours.
- **Slice C: the loop.** Items 7, 8 and 10. The bundle for agents, the
  live player for the artist, overlays for both.
- **Slice D: the hard art.** Items 9 and 11, then 12.
- **Slice E: scale and share.** Items 13, 14, 15 and 23.

**Three games** (decided 2026-11-19). The friction that led here was
all found on Top Skater, and so was every design. The tooling is proven
on three titles from three boards as it is built:

| Game | Board | Why | Used for |
|---|---|---|---|
| Top Skater (`topskatr`) | 2C | The showcase; a pack exists | Every slice: redo part of the showcase |
| Virtua Fighter 2 (`vf2`) | 2A | A fighter: characters built from moving parts, a text-heavy HUD, little scenery | Every slice: make the same kinds of asset |
| Daytona USA (`daytona`) | Original | The oldest board, and the game most people would want uplifted | Every spike, and every slice that touches the geometrizer, the renderer or identity |

A slice is done when its test passes on the first two, with calls,
pictures read and wall time counted. Daytona joins wherever the board
could change the answer. Running all three for every slice would cost
more than it finds.

**Two Model 2B games for spikes** (added 2026-11-20). 2B runs different
geometry code from the other three boards, in four versions. Spikes that
touch the geometrizer, the renderer or identity are also run on:

| Game | Why |
|---|---|
| Gunblade NY (`gunblade`) | The shared 2B geometry program, which 6 of the 14 2B games use. A rail shooter: city scenery and many enemies |
| Fighting Vipers (`fvipers`) | The earlier program, which 3 games use. Its second DSP writes parts of the display list, the hardest case for naming models |

The other two of 2B's four programs were tried the same day on
`vstriker` and `lastbrnx`, for picking, the read map, reshaping and
colour variants only. `zerogun` and `pltkids` have no native geometry
code, so picking and reshaping do not reach them at all.

### What it should cost an agent

| Per asset | Showcase | Target |
|---|---|---|
| Time to see a change in the scene | About 40 s a side from boot, plus 7 to 20 minutes queued | 2 s |
| Pictures read | 20 to 57 per agent | None on a pass; one sheet on a failure |
| Calls to understand an asset | Many: montages, TSVs, frames | 2 |
| Knowing the file was used | By eye | A field in the answer |
| Hand-over | A prose report | The project's log and status |

## Pass two: what uplift could do next

Six specs, less grounded than pass one: most costs are unmeasured, and
each leads with spikes. In the order they depend on each other:

| # | Spec | What it gives | Needs | Generic? | First proof |
|---|---|---|---|---|---|
| 16 | [Scene understanding](uplift/scene-understanding.md) | A model's identity per pixel; one stated set of buffers; exact positions and normals from each polygon's plane; camera movement | The ID pass (0.7 ms at 1080p, documented) | Planes, flags and normals for every game; models and movement need a native geometrizer | Pick a model under the cursor |
| 17 | [Model interchange](uplift/model-interchange.md) | Reshaped models (spiked); any glTF mesh (spiked in both renderers) | 16 | Rigid parts; not geometry rewritten each frame | Done for reshaping: unchanged data gives the same frame |
| 18 | [Lighting](uplift/lighting.md) | Real transparency, ambient occlusion, shadows from the game's light | 16 | First two for every game | Blended stipple; occlusion timed on three games |
| 19 | [Added elements](uplift/added-elements.md) | Grass, props, decals, weather | 16, 17 | Native geometrizer only | One prop held still through a moving clip |
| 20 | [Authoring](uplift/authoring.md) | The Studio page: the path with no terminal | Slices A and C, 16 for picking | Yes | Scene grid, pick, edit, reload |
| 21 | [Shader packs](uplift/shader-packs.md) | Shaders for any game | Nothing | Yes: the one pack kind that is not per game | The built-in CRT pass as a pack, identical |

Three findings carry most of the weight.

- **Things stay put by living in a model's own space.** An added prop or
  a tuft of grass is placed in the coordinates of a model the game draws,
  and rides that object's matrix. No camera or world space has to be
  recovered. That removes the hardest dependency this page listed after
  pass one.
- **Position and normal come from each polygon's plane, not from
  depth.** Depth is unreliable on the large ground polygons the games
  draw in their own order, which is exactly where occlusion, shadows and
  grass matter. A plane per polygon is exact, and works for every game.
- **New geometry never enters the machine.** The state hash covers the
  geometrizer's output, so replacement models and added elements are
  drawn from a second pass the machine never sees. The spike found the
  cheap way to make that pass: re-run the game's own geometry code with
  a model's data swapped, as smooth motion already re-runs it. A free
  mesh with its own textures still has to be built in the renderer.
- **New lighting takes light away; it never adds on top.** Many surfaces
  are already at full brightness, so occlusion and shadows multiply.
  Lights a pack adds, and a different time of day, collide with that and
  are left undesigned.

The main risk is what the game culls. It does not send objects off
screen or faces turned from the camera, so a shadow map made from one
frame has holes, and anything added to a model disappears when the game
culls that model at the screen's edge. Only spikes will say how much
either shows.

**Shader packs** were added by the user on 2026-11-19, with RetroArch's
shader system as the inspiration, not something to import. reModel
already runs passes of this kind (CRT, bloom), and can offer a shader
real depth and the game's lighting layer, which no emulator surveyed
does. The spec proposes reModel's own shader packs in WGSL, proven by
moving the built-in CRT pass into one and getting an identical picture.

Where pass two fits the order of work: shader packs depend on nothing
and can be done at any time. Scene understanding's first step (item 16)
can start alongside slice C. The rest follow it.

Ray tracing in the browser should not be designed for: it does not exist
there yet.

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
| Is reaching all of a game's content part of this? | Yes. A full uplift is not possible without it ([content-coverage](uplift/content-coverage.md)) |
| How are packs found? | By a convention on GitHub (topics, a manifest at the root, a release), with no curated list ([distribution](uplift/distribution.md)) |
| One game or more? | Three: Top Skater, `vf2` and `daytona`, proven as the tooling is built |
| Who judges taste? | Nobody. The tools make comparing cheap and let packs sit side by side; players choose |
| What crosses games beyond shader packs? | Left open until more than one game has a pack |

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

## Spikes

What has been tried, as against reasoned. Each row's detail is in the
spec named; scratch output stays out of the repository.

| Date | Question | Result | Changes |
|---|---|---|---|
| 2026-11-19 | How long does a still check take with no emulator? | 1.2 s without a pack, 1.75 s with the showcase pack, at 1080p from a cold start | [scenes-and-server](uplift/scenes-and-server.md): scenes before any server |
| 2026-11-19 | Can one image be tinted to follow an artwork's colour variants? | Not by any simple tint: a polygon's colour is an index, not a strength. A lookup per texel is exact on the game's art and noisy on flat new art. The showcase's authors mostly wanted the same image or a redrawn one | [asset-identity](uplift/asset-identity.md): the build bakes colour copies; `follow` is only a fallback for unseen colours |
| 2026-11-19 | Can a mid-tier model do three tasks from [interface.md](uplift/interface.md) alone? | First read: one of three. After a table of every verb's shape: all three command sequences right, with small gaps reported and then closed. A paper test can go no further | The verb table; the real test waits for slice A's commands |
| 2026-11-19 | Can every 3D pixel be traced to the model that drew it? | Yes on Top Skater, in software: every 3D pixel in 21 frames, state hashes unchanged, cost negligible. A model is a part (a limb, a board), so picking needs grouping. Existing captures hold no identity | [scene-understanding](uplift/scene-understanding.md): captures must record the side channel; a grouping step is needed |
| 2026-11-19 | Do Top Skater's findings on colour variants hold on `vf2` and `daytona`? | Yes, and more strongly: artworks drawn in several colourings carry 70% and 88% of textured pixels. Daytona also varies by luma table. One catalogue run misses 40 to 50% of textures while reporting 100% | [asset-identity](uplift/asset-identity.md), [content-coverage](uplift/content-coverage.md): identity ranks higher; coverage cannot trust the catalogue's own figure |
| 2026-11-19 | How much of the polygon ROM do Top Skater's three runs read? | 28%. The unread part is whole regions, and at least 1.72 MB of it parses as models nobody has seen | [content-coverage](uplift/content-coverage.md): the crawler and a model browser matter; the catalogue is far from complete |
| 2026-11-19 | Can a different mesh be drawn in a model's place without touching the machine? | Yes, by re-running the game's own geometry code with one model's data swapped: identical when unchanged, a scaled torso and a box held through a moving camera, 0.5 to 1.1 ms. The game's format limits it to strips, flat lighting and the original's texture sheet | [model-interchange](uplift/model-interchange.md): two tiers. Reshaping is cheap and proven; a free glTF mesh still needs its own spike |
| 2026-11-19 | Do picking, the read map and reshaping carry to `vf2` (2A) and `daytona` (original)? | Yes, with about 45 lines and no new hooks: every 3D pixel keyed, unchanged data gives the identical frame, a scaled part stays attached, state hashes unchanged. Read so far: 28% and 18% of the model data. `vf2` rewrites two models in RAM every pose, which a hash cannot name. Model 2B is untested | [scene-understanding](uplift/scene-understanding.md): RAM models are named by address. [content-coverage](uplift/content-coverage.md): the same gap on every game |
| 2026-11-20 | Do picking, the read map and reshaping carry to Model 2B (`gunblade`, `fvipers`)? | Yes, with 43 lines in the 2B geometry code: every 3D pixel keyed, unchanged data gives the identical frame, state hashes unchanged. Read so far: 14% of each game's model data. A `gunblade` enemy is a different model in every pose | [scene-understanding](uplift/scene-understanding.md): grouping must cover poses as well as parts. [model-interchange](uplift/model-interchange.md): reshaping suits rigid things, not pose-swapped ones |
| 2026-11-20 | Do the colour-variant findings hold on 2B? | Yes. The lookup per texel value is the only model that holds on all five games. `fvipers` has the most luma-table variants yet (58 artworks) | [asset-identity](uplift/asset-identity.md): no change, more evidence |
| 2026-11-20 | Can the build bake an artwork's other colourings from one authored image? | Exactly, when the art stays near the game's own colour ramp. Not at all for a restyle: the showcase's redrawn colourings are new themes no method predicts, and half its colourings are plain copies | [asset-identity](uplift/asset-identity.md): the build bakes or copies by how far the art lies from the ramp; the catalogue must record each colouring's ramp |
| 2026-11-20 | Can a new mesh be drawn in the standard polygon record, and does any of this work on the GPU? | Yes to both. A box replaced a fighter's leg on `vf2` and the car on `daytona`; the GPU draws re-run frames the same and picks the same model on 96% to 99.8% of pixels. The stored normal is one per polygon, and a new mesh can only use the original's UV entries | [model-interchange](uplift/model-interchange.md): tier 1's limit is textures, not shape. [scene-understanding](uplift/scene-understanding.md): picking fits the existing ID target |
| 2026-11-20 | Can a crawler find a game's screens without a person? | On menus, yes: with no new code it reached most of Top Skater's menu screens and found 51 textures and 17 screens the hand-made catalogue lacks. It cannot play a course, and a greedy policy starved the Expert branch | [content-coverage](uplift/content-coverage.md): the crawler is for breadth; recorded play and pokes for depth. A chained state did not hash the same as a straight run, cause not found |
| 2026-11-20 | Can a shader file brought by a user run on the finished picture, safely? | It runs, an identity shader is byte-exact, mistakes are reported, and it costs under 0.1 ms. An endless loop cannot be caught: it cost reModel its GPU device | [shader-packs](uplift/shader-packs.md): a loop rule, recovery from a lost device, and plain words that a shader pack is code |
| 2026-11-20 | How much texture data do the scripted runs read? | 18% of Top Skater's texture ROM, 13.5% of `gunblade`'s, 3.6% of `daytona`'s. Headers cannot be counted, only bytes. A leftover EEPROM file changes a run's state hash | [content-coverage](uplift/content-coverage.md): report bytes, not headers; a tour carries its EEPROM |
| 2026-11-20 | Can the renderer draw a free mesh, with its own normals, UVs and image, in a model's place or anchored to one? | Yes, in the software renderer: steady to under 0.01 of a pixel over 37 frames, smooth-shaded by the game's formula, own image, state hashes unchanged. Draw order must be per triangle and is still off on big objects | [model-interchange](uplift/model-interchange.md): tier 2 is real; the block must record matrices and material; the GPU path is the main work left. [added-elements](uplift/added-elements.md): an anchored prop holds |
| 2026-11-20 | Do ambient occlusion and shadows from the game's light give a picture worth shipping? | Occlusion is safe on three games and leaves HUD and sky untouched, but is faint. Shadows are not ready: cut-out textures cast solid rectangles, a fighter's window casts nothing on the floor, and blob shadows double up. Back-face holes fill in 72 lines. CPU only | [lighting](uplift/lighting.md): occlusion judged on a GPU pass; shadows wait on four named pieces |
| 2026-11-20 | Which asset gates separate good images from bad on the showcase? | `file` and `alpha` do; `alpha` found three textures in the approved pack that had lost their cut-out, and a fourth that may be meant. `seam` is weak. The style gates (`kept`, `palette`, `luma-order`, `category`, a halo test) cannot tell a good restyle from a bad one | [quality-gates](uplift/quality-gates.md): a first set of three errors and three warnings, with limits; style gates only for faithful packs |
| 2026-11-20 | Does GitHub behave as the pack convention assumes, and can an archive be unpacked safely? | Yes on every point checked, signed out. Forks never list; the release API already gives a hash, so the `.sha256` file goes; file addresses must use `HEAD`. Fifteen hostile archives were refused. Not tested: a real repository | [distribution](uplift/distribution.md): three rules changed; the unpacker counts real bytes |
| 2026-11-20 | Do the adapter's fixed steps turn a raw image-model output into one that passes the gates? | Yes for the 30 raw outputs on disk: 3 pass as returned, 30 after size and alpha are normalised, against 26 as hand-finished. One of the 30 passes and is still wrong: the subject does not fit the cut-out. No model was called | [generation](uplift/generation.md): a fit test, and two outcomes, `repaired` and `refit` |
| 2026-11-20 | Does the free mesh work on the GPU and on Model 2B? | Yes. The wgpu renderer draws it with no new pipeline, agreeing with software except at edges, at 0 to 3 ms at 1080p. An `fvipers` part was replaced, steady to 0.005 of a pixel. The block now records its material | [model-interchange](uplift/model-interchange.md): depth-testing for big objects; mesh images need IDs of their own |
| 2026-11-20 | Do the geometry hooks hold on 2B's other two programs (`vstriker`, `lastbrnx`)? | Yes, unchanged. `lastbrnx` is the first game with polygons sent directly (99.1% of pixels keyed). `vstriker` keeps copies of a model at up to 7 addresses, and its teams differ only by palette | [scene-understanding](uplift/scene-understanding.md): a model is named by content hash in ROM and by address in RAM |
| 2026-11-20 | Is ambient occlusion worth having as a GPU pass, and can the shadow blockers be cleared? | Occlusion: no. It matches the CPU result and touches nothing it should not, but a player would not see it and it cost over 10 ms at 1080p. Shadows: cut-outs and windows are solved; flicker, self-shadowing and a pale shadow term are not | [lighting](uplift/lighting.md): occlusion set aside; shadows stay a spike; blended stipple is the only lighting item left in the first set |
| 2026-11-20 | Can a saved texture reach the picture within a second, and is pointing cheap? | Yes: 0.2 s for a 512 texture and 0.6 s for a 2048 one, if only the saved image is swapped. Reading the pixel under the cursor takes 0.07 ms. The player has no reload at all today | [authoring](uplift/authoring.md): a one-image swap, decoding on a worker, and the ID pass without a look |

Status of each spec, by the ladder in [uplift/README.md](uplift/README.md):
the interface page, agent-environment, pack-project, 2d-cells and
renderer-controls are grounded; every other spec is spiked in part.
Generation is spiked offline only, and distribution without a real
repository.

Spike code is on two unmerged branches, `uplift-spikes` and
`uplift-spikes-b`, about 7,800 lines in all. None of it is for merging
as it stands.

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
- **Pass two** was written from the same code maps, the rendering survey
  and one more survey of shader systems. An independent review of its six
  specs checked about 55 citations and the designs against the code. It
  found that depth could not carry what three specs asked of it, and
  that new geometry could not go where two specs put it; both designs
  were changed. Its cost figures are quoted from
  reModel's docs or are other engines' rough numbers, and are marked so.
- **Not done**: no thresholds were calibrated, no tint model was tried,
  nothing was tested on a second game, and none of pass two's spikes was
  run.
