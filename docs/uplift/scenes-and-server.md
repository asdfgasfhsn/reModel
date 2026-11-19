# Scenes, and drawing them fast

Shared words, roles and rules: [README.md](README.md). Nothing here is
built.

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
- **Packs do not reload in the native player.** The directory is listed
  once (`crates/model2/src/dirpack.rs:79-104`) and `sync_pack` returns early
  when the directory is unchanged (`crates/play/src/main.rs:1906-1916`).
  Only `look.txt` reloads, by a check every 400 ms
  (`crates/play/src/main.rs:1854-1890`).
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
| `scene.txt` | The recipe as run, the state hash at that frame, the reModel version, the geometrizer mode | Checking it is still valid |
| `capture/` | The renderer's inputs for the frame (about 10 MB; `out/topskatr/capture` is 221 MB for 22 frames) | Still pictures, with no emulator |
| `state.save` | A save state at the frame | Motion clips, the live player |
| `facts.tsv` | Every asset drawn there: pixels, box, variant | "What is here" and "where is this" |
| `thumb.png` | A small picture | Browsing |

Scenes also come from play. The catalogue already notes each screen's best
frame (97 screens for Top Skater). With a state and a capture saved at
those frames, playing a game with `--catalogue` would fill the scene
library. Such a scene has no recipe, so it is marked `unreproducible`: it
works until a state-format change and cannot be regenerated.

**Staying valid.** Any change to the state format refuses every older state
(`crates/model2/src/lib.rs:349-352`). So the state is a cache and the
recipe is the truth. `scene verify` runs each recipe again and compares the
hash at the frame with the recorded one. Runs are bit-exact across the
interpreter, the JITs and the recompiler (the engine rows of
`docs/status.md`; `make state-check`), so a
different hash means the emulation changed and the scene is rebuilt.

### Three ways to draw a scene

| | Needs | Answers in | For |
|---|---|---|---|
| **1. One command**, from the capture | The released binary | 1 to 2 s (measured) | Agents, scripts, every still check |
| **2. The player**, watching the pack | The binary and the ROM | Under 1 s after a save (target) | The artist: in motion, hands on the controls |
| **3. The server**, long-running | The binary and the ROM | Under 100 ms for a still (target) | Clips, many agents on one GPU, a web studio |

Build them in that order. The first removes the build, the boot and the
lock from art work. The second gives the artist a loop. The third is the
same verbs, kept warm.

**The player** gains `--scene NAME` (load the scene's state and play from
there, which is the user loading a state, as from a save slot) and a watch on the pack directory, made the way the look is watched.
On a change it opens the pack again and calls `Renderer::set_pack`, which
already clears every pack cache (`crates/gpu/src/lib.rs:1249-1258`).
Reloading single files comes later, if the full reset proves slow. Live
reload and picking are already the next two items in
`docs/uplift-studio.md`.

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
| Player sees a saved file | Never | Under 1 s |
| Clip of 120 frames | Not built | Under 5 s |
| Making a scene | 2 to 19 s to reach an attract frame (`docs/play.md:942-945`); a race scene near frame 3500 is about a minute of game time, so longer (inferred) | Once per scene per reModel version |
| Browser | No scene tools | None in pass one; see the open question |

The server runs under `tools/capped`. A machine is 50 to 85 MB
(`docs/photo-mode.md:333-336`); the renderer holds 91 to 483 MiB of GPU
memory plus the pack, up to 512 MB (`docs/play.md:656-668`). One server for
several agents avoids each holding a pack of its own.

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
   three for a second game on another board.
2. `scene make` and `scene list`, saving capture, state and hash.
3. `scene render` with `--pack`, `--crop`, `--against original` and
   `--json`, built on what `look-render` does today.
4. Point `studio compare` at scenes, and make it refuse to reuse old
   output.

That removes the boot, the build and the lock from every still check. The
player's watch is the next slice; the server follows once clips are needed.
