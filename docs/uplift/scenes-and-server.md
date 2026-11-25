# Scenes, and drawing them fast

Shared words, roles and rules: [README.md](README.md). Status: the MVP
slice at the end is built (2026-11-21) and its user-facing parts are in
[../uplift-studio.md](../uplift-studio.md), "Scenes". The rest (the
server, `scene what/where/sheet/clip`, `--asset`,
`--overlay`, facts, the side channel) is still a design, as is the live
session ("The live session", 2026-11-26).

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

## The live session (design, 2026-11-26; not built)

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
   its own for asset reviews, and it needs no server.
2. `remodel serve` with stills (the server above), then `session.open`,
   `input`, `step`, `mark`, `restore`, `render` and `export`.
3. Top Skater's value table, `read`, and `until`.
4. Spines from the person's recordings, and the `coverage` report counting
   the branches.
5. `moves learn` and steps by move.

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
