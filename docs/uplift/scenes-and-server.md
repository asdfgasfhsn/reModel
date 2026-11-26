# Scenes, and drawing them fast

Shared words, roles and rules: [README.md](README.md). Status: the MVP
slice at the end is built (2026-11-21) and its user-facing parts are in
[../uplift-studio.md](../uplift-studio.md), "Scenes". The server and the
live session's slice 2 are built (2026-11-26; "Slice 2 as built" below,
[../uplift-studio.md](../uplift-studio.md), "Live sessions"); spines from a person's recording (slice 4) the same day, "Slice 4 as built"; what the first real use missed (watch, disasm, following, reads, until, presets) the same day, "After the first real use". The rest
(`scene what/where/sheet/clip`, `--asset`, `--overlay`, facts, the side
channel, `session.what`) is still a design.

**Re-check of "Observed" before building (2026-11-21, main at `32abfdc`).**
Line numbers have all moved. What changed in substance: the `studio compare`
defect is fixed (it now refuses to reuse a run's folder); a capture can now
also hold `light.bin` and `scan.bin` (the lighting side channel), but still
no model key per object, so the "captures hold only the finished stream"
finding stands for identity; `Frame::grab` waits for the geometrizer
itself. Unchanged: `remodel-play` has no state-loading flag and `sync_pack`
still returns early when the pack directory is unchanged; the CLI's options
are still process-wide statics (scene commands read them, and build each
machine themselves); `set_pack` still clears the pack caches; any
state-format change still refuses older states. The default geometrizer is
the chip, not native, so scenes record the mode and use the default.

## The problem

Checking an art change means seeing it in the game, at the place that
matters. Today that is slow, and the reason is not the renderer.

- **Checks replay the game from power-on.** `studio compare` runs the game
  twice from boot with a scripted input, 600M instructions by default
  (`Makefile:465-466`, `crates/cli/src/studio.rs:1826-1840`). The showcase
  agents ran 900M to 1.4B instructions per check.
- **Save states exist but no art tool uses them.** A state is about 1.2 MB
  and loads in about 100 ms including building the machine
  (`docs/play.md:934-947`, `docs/play.md:1792-1796`). Only `run`, `latency`,
  `geo-check` and `run-ahead-check` load one
  (`crates/cli/src/main.rs:3177-3190`). `remodel shot` boots from power-on,
  and `remodel-play` has no flag to load a state
  (`crates/play/src/main.rs:444-690`).
- **Packs reloaded in no player until 2026-11-23**; now `DirPack` watches
  its folder ([../play.md](../play.md), "Texture packs"). `look.txt` still
  reloads by its own check every 400 ms.
- **Scenes have no names.** Frame numbers, input scripts and region
  coordinates travelled between agents as prose.
- **Old output is reused silently.** `studio compare` keeps the `after`
  frames of an earlier run unless `--force` is given
  (`crates/cli/src/studio.rs:1834`). One agent "lost a lot of time reading
  stale remnants".

What already works, and is the base of this design:

- **A captured frame can be drawn again with no emulator.** A capture holds
  the renderer's inputs (`crates/model2/src/capture.rs:1-30`). `look-render`,
  `texmap`, `pack-check` and `gpu-check` all draw from captures.
- **It is fast.** Measured on 2026-11-19 on this machine (8 GB NVIDIA GPU):
  one captured Top Skater frame at 1080p, drawn plain and with a look and
  written as three PNGs, in a cold process, takes 1.16 s with no pack and
  1.75 s with the showcase pack of 6,290 files.
- **The renderer needs no window** (`crates/gpu/src/lib.rs:810-834`) and
  reads the machine without writing it (`crates/gpu/src/board.rs:1-60`).

So the brief's wish for a long-running server is right about the goal and
wrong about the first step. Still pictures need no server and no emulation:
they need named scenes and one command. The server earns its place later,
for motion, for sub-second loops and for sharing one GPU.

## Design

### A scene

A scene is a named moment, kept in two parts.

**The recipe** is small text and can be shared:

```ini
[scene course-start]
title = Novice course, first stretch
input = coin-start
frame = 3600
tags  = course hud skater
clip  = 120            # frames of motion worth checking from here
```

The proposal is that a game's common scenes (title, selects, each course
point, results) belong in its manifest in `crates/romset/src/games/`, next to `attract_frame`
(`crates/romset/src/lib.rs:255`) and the `coin-start` script. A project
adds its own in `scenes.txt`, and marks which are `required` and what
`coverage` each must reach. A recipe is an input script and a frame
number, not game data; whether that belongs in the repository is a
decision listed in the vision.

**The local copy** is made from the recipe and never shared, since it is
derived from the ROM. It lives in `<data dir>/<set>/scenes/<name>/`:

| File | Holds | Needed for |
|---|---|---|
| `scene.txt` | The recipe as run, the EEPROM it started from, the state hash at that frame, the reModel version, the geometrizer mode | Checking it is still valid |
| `capture/` | The renderer's inputs for the frame (about 10 MB; `out/topskatr/capture` is 221 MB for 22 frames) | Still pictures, with no emulator |
| `display.bin` | The frame's display list and the geometrizer's state before it (about 4.3 MB, mostly the polygon ROM) | The free camera (`scene render --camera`) |
| `state.save` | A save state at the frame | Motion clips, the live player |
| `facts.tsv` | Every asset drawn there: pixels, box, variant | "What is here" and "where is this" |
| `thumb.png` | A small picture | Browsing |

A scene's capture always records the geometrizer's side channel as well
(each object's model key, at least). A spike on 2026-11-19 found that
today's captures hold only the finished stream, so a scene made from one
cannot say which model a pixel belongs to, and the polygon data needed
to work it out later is not in the capture
([scene-understanding.md](scene-understanding.md)).

Scenes also come from play. The catalogue already notes each screen's best
frame (97 screens for Top Skater). With a state and a capture saved at
those frames, playing a game with `--catalogue` would fill the scene
library. Such a scene has no recipe, so it is marked `unreproducible`: it
works until a state-format change and cannot be regenerated.

**Staying valid.** Any change to the state format refuses every older state
(`crates/model2/src/lib.rs:349-352`). So the state is a cache and the
recipe is the truth. `scene verify` runs each recipe again, from power-on
and from the EEPROM `scene.txt` names, and compares the hash at the
frame with the recorded one. Runs are bit-exact across the
interpreter, the JITs and the recompiler (the engine rows of
`docs/status.md`; `make state-check`), so a
different hash means the emulation changed and the scene is rebuilt.
One case to design for. A reused output folder carries `eeprom.bin`
over from the last run and changes the hash by itself, so a recipe must
start from a stated EEPROM and a fresh folder. With that, a state
reached by chained state loads is byte-equal to the same frame reached
in one run ([content-coverage.md](content-coverage.md), E4), so a scene
reached by chaining (a `from=` branch) is checked by hash like any
other.

### Three ways to draw a scene

| | Needs | Answers in | For |
|---|---|---|---|
| **1. One command**, from the capture | The released binary | 1 to 2 s (measured) | Agents, scripts, every still check |
| **2. The player**, watching the pack | The binary and the ROM | 0.2 to 0.6 s after a save, if only the saved image is swapped (measured in a spike) | The artist: in motion, hands on the controls |
| **3. The server**, long-running | The binary and the ROM | Under 100 ms for a still (target) | Clips, many agents on one GPU, a web studio |

Build them in that order. The first removes the build, the boot and the
lock from art work. The second gives the artist a loop. The third is the
same verbs, kept warm.

**The player** has `--scene NAME` and a watch on the pack directory (built
2026-11-23: [../play.md](../play.md)). The watch does not reopen the pack: a
thread in `DirPack` settles file changes, and `Renderer::apply_pack_changes`
asks again for just the changed images (the old one stays until the new has
decoded). Measured 0.13 to 0.15 s from a write to the upload, in line with the
spike's 0.2 to 0.6 s. Picking is the next item in
`docs/uplift-studio.md`. Still to do: keep the old image when a settled file
does not decode (today the original is drawn until a good file is written),
a watch on `look.txt` through the same thread, and the server's use of it.

**The server** cannot be a loop around the CLI: the CLI keeps its options
in process-wide statics set once (`crates/cli/src/main.rs:806-838`). It is
a new front end over `model2` and `gpu`. It holds one headless renderer and
a cache of opened packs, so many clients share one GPU device and one copy
of each pack in memory. For a clip it forks the scene's machine with
`Machine::copy_from`, the 1 ms copy run-ahead uses
(`crates/model2/src/runahead.rs:1-31`), so the base state is never
advanced.

It is agent-native from the start (decided 2026-11-19): it speaks MCP,
which is JSON-RPC, over standard input and output and over a local socket
(a Unix socket, a named pipe on Windows). reModel has no network code or
dependency today, so this is new, and it stays behind a native-only
feature so the browser build's check is untouched. JSON is written and
read with a library, not by hand: `serde` is already in the dependency
tree and `serde_json` is the one addition. Whether to use an MCP library
or implement the few messages needed over `serde_json` is a choice to
make when building it, by what each costs to carry. The MCP tools are
generated from the same verb table as the CLI, so the two cannot drift.

## Interfaces

The same verbs for the CLI (`remodel scene ...`), the server and the agent
API. All take `--json`.

| Verb | Does |
|---|---|
| `scene list` | Names, titles, tags, state (`ready`, `stale`, `missing`, `unreproducible`) |
| `scene make NAME` or `--all` | Runs the recipe once, saves the state, capture, facts and thumbnail |
| `scene verify` | Runs recipes again and compares hashes; marks stale scenes |
| `scene render NAME` | Draws it. Options: `--pack DIR` (repeatable, layered), `--look`, `--view`, `--crop X,Y,W,H`, `--asset ID` (crop to where the asset shows), `--against original` or another pack, `--overlay ids`, `tags` or `miss` |
| `scene what NAME X,Y[,W,H]` | The assets under a point or box; `--kind layout` with no box lists every arrangement in the scene |
| `scene sheet NAME --label KIND` | One labelled picture of the scene, for a person to name things from |
| `scene where ASSET` | The scenes an asset appears in, with pixels and boxes |
| `scene clip NAME --frames N` | Plays on from the state and draws N frames; returns a strip and the flicker numbers |

Coordinates are in the game's own 496x384 picture, whatever the view.
No verb but `make`, `verify` and `clip` runs the game: one that lacks a
scene stops with exit code 3 and says which `scene make` to run. Whoever
holds the ROM makes the library once. Every run goes through
`tools/capped`.

`scene render --json` returns the paths written, the time taken, the share
of pixels changed against the baseline, and for each asset in view whether
the pack replaced it and why not if it did not. The reasons are the ones
the pack log records (`crates/model2/src/packlog.rs`; they are produced
in `crates/gpu/src/lib.rs` and `crates/gpu/src/tiles.rs`). An agent that changed one
asset learns from one call whether its file was used.

To an MCP client each verb is a tool (`scene.render`) whose arguments are
the CLI's options and whose result is the shared envelope.

## The roles

- **Art agent.** `scene where art:...` names two scenes. It writes its PNG,
  calls `scene render course-start --asset art:... --json`, and reads
  numbers. It opens the one crop only if a gate fails. No build, no ROM, no
  lock. Budget: 3 calls and at most 1 picture per asset.
- **Artist.** Starts `remodel-play --scene course-start` from the launcher's
  pack page, saves a file in Krita, and sees it within a second while
  skating.
- **Pack lead.** `scene render --all --against original` makes the review
  sheets for a revision in one call per scene.
- **Tester.** `scene verify` after a reModel update says which scenes
  changed, so a difference in the pictures is not blamed on the pack.
- **Tool engineer.** A new draw feature gets its before and after on every
  game's scenes, not on Top Skater alone.

## Testing

- **Without ROMs:** the scene file parser; a render of a synthetic capture
  (test fixtures exist in `crates/model2/src/capture.rs`); the server's
  protocol against that capture; strict-name and exit-code tests.
- **With ROMs:** `make scene-check`, modelled on `photo-check`
  (`Makefile:656-672`): a scripted run ends on the same state hash with the
  scene tools active and without. `scene verify` joins `bench-all`.
- **Draw-only.** Loading a state changes a machine. `scene render`,
  `scene clip` and the server therefore use a machine of their own and
  never a running player's. That is the rule the check enforces.

## Performance

| | Today | Target |
|---|---|---|
| Still, cold command, 1080p, full pack | 1.75 s (measured) | Keep under 2 s |
| Still, warm server | Not built | Under 100 ms (the GPU draw is 7.3 ms with upload and read-back, `docs/gpu.md:92-105`) |
| Player sees a saved file | Never | Under 1 s (0.2 to 0.6 s measured for a swapped image) |
| Clip of 120 frames | Not built | Under 5 s |
| Jumping to a scene from its state | 82 ms on Top Skater, 105 ms on Daytona (measured) | Keep |
| Making a scene | 2 to 19 s to reach an attract frame (`docs/play.md:942-945`); a race scene near frame 3500 is about a minute of game time, so longer (inferred) | Once per scene per reModel version |
| Browser | No scene tools | None in pass one; see the open question |

The server runs under `tools/capped`. A machine is 50 to 85 MB
(`docs/photo-mode.md:333-336`); the renderer holds 91 to 483 MiB of GPU
memory plus the pack, up to 512 MB (`docs/play.md:656-668`). One server for
several agents avoids each holding a pack of its own.

## The live session (design, 2026-11-26; slice 2 built the same day)

The server above draws stills of scenes that were made beforehand. The
**session** extends it to a running game: an agent opens a machine, sets
its inputs, steps it, moves photo mode's camera, reads named values, keeps
and returns to points, and draws what it sees, all over the same
connection. It is the server's third role, beside stills and clips, and
uses the same process, verb table and answers.

**Lockstep.** A session advances only when it is told to step. Nothing
runs in real time, so an agent can look at a frame, think as long as it
needs and send the next 30 frames of input. A language model can play
this way. It cannot keep up with a game running at 60 frames a second.

### Where it starts and what it keeps

`session.open` starts a machine from power-on, from a scene's state, or
from a tour at a frame (`from=LABEL@FRAME`, as a branch tour does). Each
session has:

- the machine, which nothing else uses (the draw-only rule under
  "Testing");
- its **log**, which is every input from the start point onwards. The log
  is a branch tour by construction, so `session.export` writes it as one
  line (`LABEL|LENGTH|from=SPINE@F INPUTS`) that replays to the same state
  hash. Pokes are logged as `poke=` terms and mark the tour as the tour
  format already does;
- **marks**, which are named copies of the machine (`Machine::copy_from`,
  about 1 ms; `crates/model2/src/runahead.rs`). `session.restore` returns
  to a mark and cuts the log back to that mark's frame. A mark costs one
  machine (50 to 85 MB), so a session holds at most `--marks N` of them
  (default 16) and refuses more, naming the oldest;
- the **camera**, the pack and the look used for its pictures. These are
  render state only, so they never reach the log.

### Verbs

| Verb | Does | Answer's data |
|---|---|---|
| `session.open` | `from: power-on \| scene:NAME \| tour:LABEL@FRAME`, `game` | session id, frame, state hash |
| `session.input` | Sets buttons (by the game's input names: `start`, `punch2`, ...) and analog channels (`steer=0x80`), held until changed | the inputs now held |
| `session.step` | `frames: N`, optional `inputs` for just those frames, `until` (below) | frame, state hash, the frames run, why it stopped |
| `session.mark` / `restore` / `forget` | Keeps, returns to, or drops a named point | marks and their frames |
| `session.read` | Named values (below), or raw `ADDR[:b\|w\|l]` | values |
| `session.poke` | Holds a value as a tour's `poke=` does | the poke, logged |
| `session.camera` | Photo mode's camera: `back`, `orbit`, `tilt`, `yaw`, `pitch`, `x`/`y`/`z`, `fov`, `reset` (the words of `--photo`, `docs/photo-mode.md`) | camera now |
| `session.render` | Draws the current frame: `view`, `pack`, `look`, `hud`, `against original`, `crop` | picture paths, the scene render answer's per-asset report |
| `session.what` | The assets under a point or box of the current frame (`scene what`) | assets |
| `session.probe` | What draws each game pixel of `at` (X,Y[,W,H]) in the current frame, and which `pack` file supplies it (`scene probe`; also `scene.probe` for a made scene) | layers per pixel, `sources` |
| `session.export` | Writes the log as a tour line, optionally `scene: NAME` (a scene at the current frame) | tour line, file |
| `session.close` | Frees the machine and its marks | |

`until` stops a step early when a condition holds: `value NAME OP V`
(`speed gt 40`), `screen NAME` (a named value that says which screen
shows), `frames N`. A step never runs more than `--max-step` frames
(default 3,600, one minute of game time), so a wrong condition cannot
hang the server.

Coordinates are the game's 496x384, as for scenes. Inputs use the names of
the game's input table (`crates/romset/src/games/`), and so does
`--record-tour`.

### Named values

`session.read` answers from a per-game table of named work-RAM values:
`crates/romset/src/games/values/GAME.txt`, one line per value,
`NAME|ADDR:WIDTH[:signed]|what it is and how it was found`. The format is
the same as the poke tables. Values are found with `ram search` on a tour,
and the table records how, as the poke table does. For Top Skater the
first values would be the skater's position, speed and heading, airborne
or not, the clock, the score, the course and the current screen. A game
without a table still answers raw addresses. Like the poke tables, these
are facts about the game, so they are per-game data, separate from the
generic code.

### The free camera from a scene

A capture holds the geometrizer's finished raster stream
(`crates/model2/src/capture.rs`). That stream is already projected for
the game's camera, so a capture alone cannot be seen from elsewhere.
Photo mode's free camera works on the display list
(`crates/model2/src/photo.rs`, "Free camera"). It applies one more rigid
transform to every object transform and runs the list through the native
geometrizer again. To use it from scenes, `scene make` also stores the
frame's display list and lighting taps (`display.bin`,
`crate::smooth::Frame`). `scene render --camera SPEC` and
`session.camera` then re-project that list. Neither needs the ROM or a
running machine. The limit is photo mode's: what the game culled for its
own camera is missing, and polygons sent already in view space stay where
the game's camera saw them. For a wall the game does not draw, the agent
has to drive the session to where the game does draw it.

### Exploring from a recording

A person's recording of a course (`--record-tour`) becomes a **spine**.
An agent opens a session at any frame of it, branches with its own inputs
to look behind a wall or at a far corner, renders, and restores. It never
has to play the hard parts itself, and everything it reaches can be
exported as a branch tour that replays exactly. `coverage` and the
catalogue already read tours, so what the branches reach counts toward a
course's coverage like any other tour.

**Moves.** `remodel moves learn TOUR...` cuts recordings into short input
spans that recur, such as a push, a turn, an ollie or a trick. It measures
what each span does to the named values from the same starting conditions
(speed, heading change, airtime). The result is a per-game **move table**:
`NAME|INPUTS over N frames|measured effect, range, sample count`. A step can
then use moves instead of raw inputs (`step {move: "ollie"}`), so an
agent picks among actions whose effects it knows. This is plain
measurement over the person's own play, with no model training. Learning
a policy from a few runs (imitation or reinforcement learning) is left
until spines and moves stop being enough.

### Transport and safety

The protocol is MCP (JSON-RPC) over standard input and output, and over a
Unix socket at `out/serve/GAME.sock` with mode 0600. Every verb is also a
CLI command with the same answer envelope (`crates/cli/src/answer.rs`),
generated from one table so the two cannot drift. It listens on nothing
but the local socket, and nothing in reModel opens a network port for it.
The server runs under `tools/capped` and holds the render lock only while
it draws. The browser build has no session, so its check is unaffected
(native-only feature).

### Testing

- **Replaying a tour through the session.** Opening a session at power-on
  and stepping a tour's inputs ends on the tour's state hash (the
  `scene-check` pattern).
- **Export round trip.** `session.export` followed by `tour run` gives the
  session's hash.
- **Marks.** `restore` after any number of steps gives the mark's hash
  exactly, and the log is cut to match.
- **Camera.** `scene render --camera` from `display.bin` matches photo
  mode's picture of the same frame and camera, within the GPU check's
  tolerance.
- **Without ROMs:** the protocol and the verb table against a synthetic
  capture, plus name and limit errors (`--max-step`, `--marks`).

### Slices

1. `display.bin` in scenes and `scene render --camera`. This is useful on
   its own for asset reviews, and it needs no server. **Built**
   (2026-11-26): `scene make` records the last frames on the native
   geometrizer (the state hash does not change), and the camera render
   matches photo mode's picture.
2. `remodel serve` with stills (the server above), then `session.open`,
   `input`, `step`, `mark`, `restore`, `render` and `export`. **Built**
   (2026-11-26), with `read`, `poke`, `camera`, `forget`, `close`,
   `until value` and `scene.render` as well: "Slice 2 as built" below.
3. Top Skater's value table, `read`, and `until`. **Built**: the
   table (`crates/romset/src/games/values/topskatr.txt`), its read
   from the command line (`remodel ram values TOUR --at F1,F2,...`,
   docs/uplift-studio.md, "Named values"), and `session.read` and
   `until value NAME OP V` in the server. `until screen NAME` is not
   built (the screen word does not name every screen).
4. Spines from the person's recordings, and the `coverage` report counting
   the branches. **Built** (2026-11-26): "Slice 4 as built" below.
5. `moves learn` and steps by move. **Built** (2026-11-26): "Slice 5 as
   built" below.

### Slice 2 as built (2026-11-26)

`remodel serve --game G [--socket [PATH]] [--marks N] [--max-step N]
[--sessions N] [--tours FILE] [--no-stdio]` and `remodel call VERB name=value
...`; how to connect is in [../uplift-studio.md](../uplift-studio.md), "Live
sessions". The code is `crates/cli/src/session.rs` (the verb table and the
sessions) and `crates/cli/src/serve.rs` (MCP, the transports, the command
line); `crates/cli/src/scene.rs` gained `draw_frame` (a frame in hand, not a
capture on disk) and `render_scene`, which `scene render` and the server both
use. It is behind the `serve` cargo feature of the `remodel` crate (on by
default, unix only); the browser build does not use that crate.

**The MCP library question.** The messages are written over `serde_json`, not
taken from a library. A server needs `initialize`, `notifications/initialized`,
`ping`, `tools/list` and `tools/call`: about 100 lines. The Rust MCP libraries
bring an async runtime and an HTTP stack (tokio, hyper or axum) that reModel
has nowhere else, and follow a protocol that has had three dated revisions in
two years, of which the part used here did not change; carrying them would cost
build time and a dependency set for no function this server uses (no HTTP, no
resources, no prompts, no sampling). The server answers with the client's
protocol revision when it knows it (2025-06-18, 2025-03-26, 2024-11-05) and
otherwise the newest. One line is one message, on standard input and output and
on the socket; requests are handled one at a time, in order, whichever
connection sends them.

**One table.** `session::VERBS` lists each verb's name, text and arguments
(name, type, required). `tools/list` is generated from it (the JSON schema of
each tool), argument checks run from it (unknown names with the near ones,
wrong types, missing required ones, all as answers with code 2), and
`remodel call` builds its arguments from it (`name=value`, a list by repeating
the name). A test calls every verb through the command line's path and the
tools' and checks they validate the same. Every tool answers with the shared
envelope (`ok`, `code`, `error`, `data`, `warnings`, `next`) as
`structuredContent`, and the same text as `content`; `isError` is `!ok`. A
wrong call is a JSON-RPC error only when it is not a tool call (parse error
-32700, bad message -32600, unknown method -32601, unknown tool or malformed
`params` -32602); a refused argument is an answer with code 2, as for every
other command.

**Deviations from the design above.**

- **When inputs apply.** The design left it open. What `session.input` (or a
  step's `inputs`) holds applies to the frame the session is on, the one about
  to run, and every frame after, as a tour's script does. The frame the session
  *started* on keeps the inputs of what it started from: a scene recipe's, a
  tour's up to its branch point, none at power-on (all released). That is what
  makes the log a branch tour: a session's own terms start after the branch
  point, which the tour format requires. A session that holds `start` and
  steps 30 frames runs 30 frames with `start`.
- **The state hash.** It is the machine's state with the current frame's
  inputs applied, as `tour run` ends. It equals a scene's recorded hash at the
  scene's frame (a scene's is taken a moment before that frame's inputs; only a
  poke covering the frame can tell them apart).
- **`from: scene:NAME`** loads the scene's `state.save` (about 110 ms) and
  checks it against the recorded hash (a mismatch is a warning: the scene is
  stale). A scene is not a tour, so the exported line carries the recipe's own
  inputs up to the scene's frame instead of a `from=`; it replays from
  power-on to the same hash.
- **Marks** keep their own copy of the log, so any mark can be restored at any
  time, not only the newest ones (the design cut the log to the mark; the
  result is the same for the mark restored). A mark's machine is made on
  first use and kept for the next mark after a `forget`.
- **Limits.** `--sessions N` (default 4) is new: the design left a cap open.
  `--marks` names the oldest mark when it refuses. `session.step` refuses over
  `--max-step`, not truncating.
- **`session.camera`** takes the words of `scene render --camera` as one
  string (`orbit=30,tilt=-8`; `reset` clears). A picture with a camera needs
  the frame's display list: a step records it for its last three frames (as
  `scene make` does), so it is there after any step and missing after `open`
  or `restore` until the next step (the render says so, code 3). The game's
  geometrizer needs a native implementation (the chip-run games have none).
- **Not built:** `session.what`; `session.export`'s
  `scene: NAME`; a cache of opened packs (each render builds its pack layers
  again; the GPU, the looks and the machines are kept warm); light-gun aim
  in the log (the export says so in a warning); a named pipe on Windows.
- **Answers the open questions.** `step` does not draw: drawing is its own
  verb, so an agent draws when it wants a picture. A cap on sessions per server
  exists (`--sessions`); how many agents share one server is for later.

**Measured** (2026-11-26, Top Skater, interpreter, release build, this machine
with an 8 GB NVIDIA GPU; `make serve-check MEASURE=1`):

| | |
|---|---|
| Step, no drawing | 1,630 frames/s (27 times game speed), in 600-frame steps |
| Step with a 1080p picture after every 10 frames | 55 frames/s, 18 ms a frame, 180 ms a cycle |
| `session.render`, 1080p, warm (GPU started) | 152 ms round trip: draw 32 ms (including upload and read-back), PNG write 126 ms |
| `scene.render`, 1080p, warm | 158 ms: draw 36 ms, write 122 ms |
| `scene.render`, 540p, warm | 47 ms |
| First render (GPU start) | adds the GPU's start, once per server (it ran inside the 267 ms of the first 540p picture) |
| `session.open`, power-on / from a scene | 7 ms / 110 ms |
| `session.mark` | 51 to 64 ms: the first copy into a machine of its own moves all its memory; `copy_from` skips what neither machine wrote when a copy repeats into the same one |
| `session.restore` | 5 ms |
| Memory | a session 76 MB, each mark 73 MB; the server at the end of the run (GPU, two sessions' worth, eight marks) 1,084 MB |

The warm still misses the under 100 ms target at 1080p because of the PNG
(126 of 152 ms: the encoder, not the GPU); at 540p it meets it. A smaller or
faster encode, or a crop, is the lever.

### Slice 4 as built (2026-11-26)

`crates/cli/src/spine.rs` (the cache, the index, the queries), the verbs
`spine.index` and `spine.where` in `session.rs`, and `remodel spine
index|where LABEL` (`serve::spine_main`); user-facing description and the
measured numbers in [../uplift-studio.md](../uplift-studio.md), "Spines".

- **Checkpoints.** `out/GAME/spines/LABEL/ckpt-FRAME.state` every 600 frames
  (`--every`), each a `save_session` taken after the frame's inputs are applied,
  which is the state `session.open` hands out, so a load needs no fix-up and the
  state hash equals the replay's at every frame (a test checks frames at, before
  and after a checkpoint, and 1). `meta.txt` (the key, the interval, the frames
  and the list) is written last, so a build that stopped half-way is no cache.
  The key is the FNV of the file layout, the game, the geometrizer and the tour's
  normalised line followed by its `from=` parents'. `tour::parse` now puts that
  chain in `Tour::spec`, so a branch's read map goes stale when its spine's
  line changes, which it did not before.
- **Opening.** `session.open from: tour:LABEL@F` builds the cache on first use
  (one replay, warned), loads the nearest checkpoint at or before F and steps the
  rest; the answer says `checkpoint` and `stepped`. A tour whose length is in steps
  has no checkpoints and replays as before.
- **Index.** `index.tsv`: a header with the key and a hash of the value table,
  the column names, and a row of numbers per sample (`-` for a value that is not
  readable or not finite). It is made in the same replay as the checkpoints. A
  changed key or table is no index. The query is a scan: `near` is the Euclidean
  distance over the named values, results at least `gap` frames apart; with no
  `near` the answer is the spans of consecutive matching rows.
- **Branches and coverage.** `session.export` already wrote `from=LABEL@F`
  branches; its `file` now holds every `--tours` file's text, not one. `--tours`
  repeats in `tour run`, `coverage`, `ram`, `crawl`, `spine` and `serve`
  (`tour::Common::tours` is a list), so a branch whose spine is in another file
  runs and counts. A branch's map is a replay from power-on, so its reads include
  its spine's, and `coverage` unions them.
- **Not built:** spans over a derived value (a speed from
  positions), a nearest-neighbour structure (the scan is 645 rows for a 19,000
  frame run), and a cache shared between worktrees.

### After the first real use (2026-11-26): what it missed, built

The field notes ([session-field-notes.md](session-field-notes.md)) listed what a
long task against the server lacked; this slice builds it. The verb table
(`session.rs`) grew `session.watch`, `session.disasm` and `session.list`, and
`step`, `open`, `read`, `camera` gained arguments; the command line gained
`remodel call VERB --help`. Nothing in the code names a game: the screens are
lines of the game's value table.

- **Watch.** `Model2Bus::watch` (was a single address for `REMODEL_WATCH`) is an
  `Option<Box<Watch>>`: ranges, a cap and the hits, tested once in `mem_write`,
  where the old test was. Each hit is (cycle, ip, address, value, size, frame);
  the frame is `bus.frames` when the write happens. `ip` is `cur_ip`, which the
  interpreter sets before each instruction and recompiled code and the JIT set
  through `Host::at` on the paths that reach the bus (the JIT's inline memory path
  never reaches it, so the bus turns it off while a watch is set:
  `fast_pages`). `Model2Bus::poke` takes the watch out for its write, so a
  session's pokes are not the program's writes. The watch is in the session, put
  on the bus for the frames of a step and taken off after (so it is not in a
  mark or a state, and `restore` does not touch it). **Measured**
  (2026-11-26, Top Skater, release, this machine, medians of five and three
  runs, frames a second): boot from power-on, base build 437 and 440, this build
  439 and 441, a watch on the clock 436; the race (frames 3300 to 6300 of the
  coin-start tour), interpreter, 305 and 313 without a watch, 309 with one;
  `--jit`, 804 without and 580 with (the fast path off). The state hash was the
  same with and without, on the interpreter and under `--jit` (a test and a
  one-off check).
- **Disassembly.** `session.disasm` uses the i960 crate's `decode` (it had a
  disassembler already, in `Insn`'s `Display`) over `bus.peek`, so it reads ROM
  or RAM code as the machine holds it, up to 256 instructions; the listing is an
  answer (the repo never holds listings). Starting before the address can land
  inside an eight-byte instruction (the answer says so).
- **Following.** `step follow` builds one `InputScript` per step from the
  recording's terms (`tour::inputs`, named pokes resolved), the held `inputs` as
  `NAME@first-last` and the session's pokes, and applies it each frame, the log
  recording what was applied (so the export is explicit terms and replays). The
  recording's pokes become the session's. `open overlay` runs the recording plus
  the overlay terms from a checkpoint before the overlay's first frame; the
  export is `from=LABEL@(A-1)` with the recording's terms for frames A to the open
  frame (`tour::cut` and the new `tour::tail`) and the overlay as the branch's
  own, since the format wants a branch's terms after its branch point; an overlay
  from frame 1 exports with no `from=`. An overlay past the open frame is not
  carried (a warning). A repeating term that began before A cannot be restated
  and the export says so (it cannot start late).
- **Reads.** `parse_raw` takes `ADDR[:LEN][:b|w|l][:signed|float]` and
  `ADDR..END`. `when=before_poke` works from `Session::pre`: each time the
  pokes of a frame apply, the bytes they cover are noted first, and a read
  substitutes them. It is kept in a mark and restored. A read warns when a poke of the current frame covers
  a value it returns.
- **`until`.** `distance`, `changes` (compares raw bytes with the step's start,
  armed at the step) and `screen` (a `NAME|VALUE=NUMBER|note` line of the value
  table, `tour::state_table`; Top Skater names six screens). The step's answer has
  `until_reading`.
- **Spine granularity.** `serve --index-every K --checkpoint-every N` (the
  `Config` fields existed) and `open`'s `index_every` and `checkpoint_every`.
- **Camera presets.** `scene::CAMERA_PRESETS`, expanded wherever a camera is
  parsed: `chase` (`tilt=14,z=-0.3`), `side` (`orbit=90,tilt=6,z=-0.2`), `top`
  (`tilt=55,z=-0.25`), shares of the scene's scale. Photo mode's tilt is positive
  for an eye above (checked on a race frame: `tilt=-30` puts the eye under the
  floor, looking up, which is what the notes found). The game draws little behind
  its own camera, so a view from further back shows a bare floor there; in a hall
  the eye can end inside a wall or ceiling (no geometry is tested: the render
  warns for `side` and `top`).
- **The restore question.** The notes' one unexplained observation (a task
  missing after a restore in a session with pokes) did not reproduce. A test
  runs two sessions, restores one to a mark after pokes, steps and restores
  again, and compares the whole saved machine state (`Machine::save_state`,
  not just the state hash) with a fresh replay of the same inputs: right after the restore,
  200 frames later, and after a mark taken with a poke pending. It is equal
  except for three bytes of the render and palette RAM *version* counters, which a copy only
  ever grows (so a renderer re-uploads) and which are no part of the state hash;
  the test allows those two counters and nothing else. So `restore` is not the
  cause; the notes' own explanation (every later result came from a fresh
  session) stands. An unconfirmed guess: the walk read the list from a mark
  taken before the task existed. The pokes' own state (their log and the
  bytes they overwrote) is part of a mark; the machine's is a full copy
  (`copy_from` with fresh marks, so nothing is skipped).
- **Not built:** `spine.where --refine` (re-stepping around the best rows),
  `remodel call --batch`, an append mode for `session.export file=` (the shared
  tours file the notes lost lines in), and the `ip` of writes the copro makes.
### Slice 5 as built (2026-11-26)

`crates/cli/src/moves.rs` (segmenting, measuring, the table, `moves learn` and
`moves check`), the verb `session.moves` and the `move` argument of
`session.step` in `session.rs`, `romset::games::moves` (the shipped table), `make
moves-learn`. How to use it, and the table's format: [../uplift-studio.md](../uplift-studio.md),
"Moves".

**Deviations and choices.**

- **One pass, forks.** The design says to replay from a spine checkpoint just
  before each occurrence. The build replays once from the checkpoint before the
  race stretch and, at each occurrence's first frame, copies the machine
  (`Machine::copy_from`) and runs the window without the move's inputs; the
  recording's own run is read from the same pass. The same two runs, for the cost of
  one pass and a window each (a replay per occurrence would cost about 300 frames
  more each).
- **Clusters, not single presses.** Runs that overlap are one move (a jump while
  steering is `steer-left-hi-5f+jumptail-tap`). The move is measured as recorded;
  the table's inputs are the median of its samples (an analog's strength and
  digital hold lengths, each part's offset). Steering in the recordings is almost
  always full (the keyboard sets 0x10 or 0xf0), so every steer is `hi`; `lo`/`mid`
  exist for a pad's analog stick and in `secret-park-ride` (0xc0).
- **The effect is against doing nothing.** Removing a steer in a corridor often
  means a wall, so `steer-left-hi-20f` shows a median speed gain of about 1.6 on the
  ground: that is what the steer did in the situations the person used it, not
  what any steer does. `dlat` (sideways) is the effect the table gets right.
- **Heading.** `heading-x`/`heading-z` (`dyaw`) barely move under steering (the
  skater turns its path, not its matrix: median 0 to 3 degrees); the direction of
  travel from `vel-x`/`vel-z` (`dpath`) and the position change along and across
  the old direction (`dfwd`, `dlat`) were added. Trick points have no value of
  their own; `dscore` (the score) carries them (a hop with the tail is about 3,500).
- **Window and screens.** The window is the move's frames plus 60 (at most 240).
  The race screen's word reads 0 for a few frames now and then and for 30 to 60
  frames while a section loads; a window may hold up to a tenth of such frames and
  no other screen. 7 of 356 clusters were dropped as off the race.
- **Waits.** `step move=wait-N` (not in the table) is N frames of nothing: a move
  repeated under `until` is otherwise that input held.
- **`until`** is checked every frame, so a sequence stops mid-move. `frames`
  becomes the cap (default `--max-step`).
- **The shipped table is derived from play, not the ROM** (the recordings are the
  user's, not committed; the table keeps no frames or listings).
- **The option `--out`** is the table's file for `moves` (elsewhere it is the output
  folder); `main.rs` leaves it alone for `moves`.

**Measured** (2026-11-26, release, `--jit`, this machine; learning twice from
the same recordings gave a byte-identical table).

| | |
|---|---|
| Learning, Novice (19,368 frames) and Expert (13,602) | 65 s with the spines cached (33 s and 31 s; 10,290 and 9,330 race frames replayed), 275 s cold under load (it also builds the two spines, on the interpreter, while other builds ran) |
| Clusters | 356 (8 too long, 7 off the race), 292 measured |
| Moves | 70 seen, 18 kept (at least 3 samples), 52 dropped (61 samples); 95 table rows (a row for all samples and for each condition combination with at least 3) |
| Top moves (samples) | steer-left-hi-20f 27, steer-right-hi-5f 26, steer-left-hi-5f 23, steer-left-hi-10f 21, steer-right-hi-10f 18, steer-right-hi-20f 18, steer-left-hi-40f 16, jumpfront-tap 12 |

**Prediction** (`remodel moves check`: measure the moves in recordings the table
was not made from, predict each from the best matching row (most conditions that
hold, else the overall row), compare). "Skill" is 1 less the table's mean absolute
error over the error of predicting no effect; "in range" is how often the measured
effect lay inside the row's min..max. `secret-park-ride` (held out; 81 clusters, 58
measured; 46 of them moves the table has, 12 not) is another course, a bowl:

| effect | mean abs actual | MAE table | skill | in range |
|---|---|---|---|---|
| dspeed | 0.218 | 0.636 | -1.92 | 76% |
| dyaw (deg) | 14.0 | 14.0 | 0.00 | 65% |
| dpath (deg) | 25.1 | 21.6 | 0.14 | 68% |
| dair (frames) | 3.0 | 2.7 | 0.10 | 87% |
| dheight | 2.88 | 3.00 | -0.04 | 59% |
| dfwd | 21.1 | 27.5 | -0.31 | 71% |
| dlat | 63.6 | 28.9 | 0.55 | 76% |

Learning from Novice alone and predicting Expert (133 measured, 86 predicted):
dspeed -0.71, dyaw -0.14, dpath -0.19, dair -0.06, dheight 0.18, dscore 0.57
(the hop's trick points), dfwd -0.58, dlat -0.10; in range 61 to 97%.

Honestly: the table tells an agent which way a move moves the skater (the sign
and rough size of `dlat`, the airtime and score of a hop) well on a new course in
the same recordings' style, and nothing reliable about speed or forward distance,
where it does worse than assuming no effect. Effects depend on the situation
(a wall, a slope, the stage of a jump) far more than on the move; the airborne and
speed conditions do not capture that. More recordings would help the rare moves
(52 dropped); a state-dependent model (the effect given the skater's heading and
the walls near) is where it would have to go, beyond measurement of this kind.

**Demo** (a session on the Novice course, the shipped table): from
`tour:novice-full@4230` (skater at -2187, -2105, speed 4.1) to the point (-2465,
-2659): 620 units from the start and 208 from the polyline of the recording's own path
(212 from its nearest sample), which the run never visited. The search was
`session.step move=M move=wait-W until="distance pos-x,pos-z to -2465,-2659 lt 40"
frames=600` for six steer moves and two waits (12 sequences): two reached it,
`steer-right-hi-40f` + `wait-10` in 205 frames and `steer-right-hi-80f` +
`wait-30` in 219 (ending 39.9 from the point, frame 4449, state hash
`c46556e97ffd6b24`). `session.export` wrote `moves-demo|4449f|from=novice-full@4230,
curving=0x10@4231-4310,curving=0x10@4341-4420`, and `tour run` replayed it to the
same hash on the interpreter and on `--jit`. What did not work: a greedy
controller that picked the move whose table effect pointed at the target each
step never arrived (the skater runs past; the table's forward effects are not
reliable), a start at frame 3600 (the skater is almost stopped there, steering
does nothing), and repeating a move with no wait (that is holding the input).

**Tests.** Without ROMs: the table's parser and its refusals and round trip, the
shipped tables' inputs against the game's input table, segmenting synthetic input
streams (clusters, long chains, naming, shifted in time), bucketing (holds,
durations, analog strengths, speed terciles), aggregation (medians, ranges,
condition rows, dropped moves), the effects arithmetic, the command line. With the ROM
(skipped without it): measuring is deterministic, a hop's measured effect is airtime
and height and a steer's is sideways; `step move=` equals the raw inputs
(same state hash and exported line), including a move with a gap and `wait-N`;
`until` repeats a move, stops mid-move, on a distance after two repeats, at the
`frames` cap and at `--max-step`; refusals; `session.moves` and its filters.

**Not built:** a state-dependent effect model; moves from pad recordings with
real analog strengths (none was available); `moves learn` over several games at
once; `session.moves` ranking by a goal; a stop that waits for the end of the move
(`until` checks every frame).

### Open questions (session)

- **Step speed.** Unthrottled stepping with no drawing should run well
  above game speed (`--fast` in `photo-check` does), but a session that
  draws every step will not. How often agents render will decide whether
  `step` draws by default.
- **Several sessions in one server.** Each holds a machine (50 to 85 MB)
  plus marks. Whether to cap sessions per server, or run one server per
  agent, depends on measurements.
- **Analog inputs while stepping.** Steering that changes every frame
  makes long logs. The tour format already keeps them exact. Moves may
  compress them for the agent's side only.

## Open questions

- **Capture size.** About 10 MB a frame, mostly texture RAM. Captures of
  one game could share that data. Not measured.
- **Capture format changes.** Whether captures carry a version, and how
  often they break, was not checked.
- **Geometrizer mode.** The player keeps separate preview states for the
  native and the chip geometrizer (`docs/play.md:939-941`), which suggests
  a state belongs to its mode (inferred, not checked). Scenes should fix
  the mode to native and say so.
- **Several GPU processes at once** is untested. GPU memory crowding has
  broken the GPU tests before.
- **30 Hz games.** `vstriker` repeats each stream; a clip there needs the
  two-vblank step.
- **The browser** is out of scope for these tools (decided 2026-11-19).
  The web module already exports a headless machine and a live `set_pack`
  (`crates/web/src/lib.rs:990-1034`, `1426-1456`), should that change.

## MVP: a few days

1. `scenes.txt` and manifest scenes, with five recipes for Top Skater and
   three for a second game on another board. **Built** (`vf2`, board 2A).
2. `scene make` and `scene list`, saving capture, state and hash. **Built**,
   with `scene verify` (and `make scene-verify`).
3. `scene render` with `--pack`, `--crop`, `--against original` and
   `--json`, built on what `look-render` does today. **Built**.
4. Point `studio compare` at scenes, and make it refuse to reuse old
   output. **Built** (`--scene`).

That removes the boot, the build and the lock from every still check. The
player's watch is the next slice; the server follows once clips are needed.
