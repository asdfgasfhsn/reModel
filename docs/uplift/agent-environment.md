# An environment where work does not queue

Shared words, roles and rules: [README.md](README.md). Nothing here is
built.

This spec is about agents working in a reModel checkout on a shared
machine. It does not concern a stranger with a released build: they have
one binary and nothing to queue behind. Keeping it that way is the first
design rule below.

## The problem

Waiting was the largest cost in the showcase work, and the cause was
partly a missing tool.

**From the agents' record.**

- One agent handed back unfinished six times because its build never
  started in over an hour.
- Art agents waited 7 to 20 minutes or more for a comparison render
  queued behind another agent's test run. Two ended with their final
  review sheets unrendered.
- Agents polled with sleep loops and left waiting processes behind.
- `cargo` was not on the path in agents' shells. Worktrees lacked `rom/`,
  the EEPROM files (15 false hash mismatches in one benchmark run) and
  `out/`.
- Other agents' builds overwrote the shared release binary mid-task, so
  agents kept private 300 MB copies.
- GPU memory held by other agents' processes caused out-of-memory
  failures at 4K.
- Python image libraries were missing in four agents' shells. Per-pixel
  work went through shell and ImageMagick, with syntax errors, and one
  `montage` command overwrote an original image.

**Observed in the repository.**

- **The heavy lock is not a mechanism.** Nothing in the `Makefile`,
  `tools/` or any script takes `out/heavy.lock`. It is a convention typed
  by hand, described in `CLAUDE.md` and a local note. Two `make` runs do
  not wait for each other unless each caller remembers.
- **`target-agents/` does not exist** yet, though `CLAUDE.md` tells agents
  to build into it.
- **Builds are not memory-capped.** `tools/capped` wraps runs of the two
  binaries (`Makefile:102-105`), not `cargo`. One `rustc` has been
  measured at 4.8 GB on this 31 GB machine (`docs/recompiler.md:89-105`).
- **Runs collide.** `run` deletes `out/GAME/frames` and reads
  `out/GAME/eeprom.bin` (`crates/cli/src/main.rs:3165-3172`), so two runs
  in one tree interfere unless both pass `--out`.
- **A still render is light.** One scene with the full showcase pack
  takes 1.75 s and no emulation (measured;
  [scenes-and-server.md](scenes-and-server.md)).

The last point is the way out. Art checks waited behind builds and test
runs because everything shared one lock, not because they are heavy.

## Design

### 1. Art work needs no build

An art agent uses a pinned, prebuilt binary and never runs `cargo`.

- `make agent-bin` builds once and places the binaries in
  `out/bin/<commit>/`, read-only. Nothing overwrites them; a new commit
  gets a new folder. A released build serves as well.
- Every art tool takes the binary from `REMODEL=`, which the set-up below
  points there. Runs still go through `tools/capped`, as every run does:
  the Makefile's `RUN` already wraps `REMODEL` in it.
- Art tools read scenes and the catalogue; none of them compiles
  anything.

### 2. A lock tool, by resource

`tools/lock CLASS -- command` replaces the hand-typed `flock`.

| Class | At once | Protects |
|---|---|---|
| `build` | 1 | Memory, and the cargo target directory |
| `test` | 1 | The GPU tests and the long test runs |
| `render` | 3 (to be measured) | GPU memory |
| `emulate` | `JOBS` | CPU cores |

- **The Makefile takes the lock itself.** `make check` takes `build` then
  `test`; `make look-render` takes `render`. Nobody types a lock.
- **Renders never wait behind builds.**
- **Holders are visible.** Each lock file records the process, the
  command and the start time. `tools/lock status` lists holders and
  waiters.
- **Waits are bounded.** `--wait 300` gives up with exit code 3 and says
  who holds the lock. An agent can then do other work or hand back with a
  reason, without a sleep loop.
- **Dead holders are cleared**, by checking the recorded process.
- **Builds are capped** through `tools/capped`, with their own limit.

### 3. A worktree that works

`make agent-setup` (run in a new worktree) does what agents did by hand:

- links `rom/`, and copies each game's EEPROM so benchmark hashes match;
- sets `CARGO_TARGET_DIR` to the shared `target-agents/` and puts `cargo`
  on the path, in one environment file the Makefile reads;
- points `REMODEL=` at the pinned binary;
- makes the agent's own scratch folder under `out/scratch/`;
- links the scene library and the catalogue read-only;
- prints, as JSON, what is available: binary, games with ROMs, scenes,
  free GPU memory, free disk.

### 4. Tools that do not collide

- Every command that writes takes `--out`, and defaults to the agent's
  scratch folder when run in a worktree.
- `run` deletes only a frames folder it made.
- The EEPROM a run starts from is read-only; a run writes its own copy.
- Projects hold no binaries, and copies inside a project are reflinks.

### 5. Image tools in the binary

`remodel img` gives the per-pixel verbs agents reached for, in Rust, with
`--json`: `sheet` (a labelled contact sheet), `diff`, `stats`, `crop`,
`resize`, `quantise`, `composite`, `roll`. The style processor already
shows the pattern: `studio style` replaced a shell script. No Python, no
ImageMagick, and the same tools for a stranger on Windows.

The brief suggested a container or virtual environment with an image
toolkit. That would help this machine only, and complicates GPU access.
Putting the verbs in the binary serves everyone.

### 6. Resources are checked before they run out

`tools/lock render` checks free GPU memory before starting and waits if a
render would not fit. A disk floor stops scratch-heavy commands early,
with a message, where today the limit is a note in a hand-over file.

### 7. A briefing a tool can enforce

`project brief --json` gives an agent its task as data: the assets
assigned, the scenes to check, the verbs it may use, and its budgets. The
limits are enforced by the tools (an art agent's `approve` is refused),
since guidance alone is ignored by an agent that never reads it.

## Data formats

A lock slot is a file `out/locks/<class>.<n>` held with `flock`, holding
one line: process, start time, command. The environment file is
`out/agent.env`, plain `KEY=value` lines (`CARGO_TARGET_DIR`, `PATH`,
`REMODEL`, `REMODEL_ACTOR`, the scratch folder), read by the Makefile.
`make agent-setup` ends by printing:

```json
{"binary":"out/bin/7d8f099/remodel","games":["topskatr","srallyc"],
 "scenes":{"topskatr":5},"gpu_free_mb":5088,"disk_free_gb":195,
 "scratch":"out/scratch/agent-3/"}
```

## Interfaces

| Command | Does |
|---|---|
| `tools/lock CLASS [--wait S] -- CMD` | Runs CMD holding a slot of the class |
| `tools/lock status [--json]` | Holders, waiters, how long |
| `make agent-setup`, `make agent-bin` | The worktree and the pinned binary |
| `remodel img VERB` | The image verbs |
| `project brief` | An agent's task, as data ([pack-project.md](pack-project.md)) |

## The roles

- **Art agent.** Starts with a working tree and a binary. Its render
  waits for at most two other renders. It never sees `cargo`.
- **Tool engineer.** `make check` queues behind other builds, visibly,
  and gives up cleanly on a limit. Its build cannot overwrite a binary an
  art agent is using.
- **Tester.** Benchmark hashes match in a fresh worktree on the first
  run.
- **Pack lead.** `tools/lock status` answers "why is nothing happening".
- **Artist and player.** Unaffected: a released build has no locks.

## Testing

- The lock: two holders of a one-slot class, a bounded wait that expires,
  a holder that died, three renders at once.
- The set-up: in a temporary worktree, `make agent-setup` then one game's
  benchmark matches its expected hash.
- The image verbs: synthetic images, as for the style processor.

## Performance

Targets: an art agent's still render starts within 10 s with three agents
rendering. No art task takes the `build` lock. A lock wait reports its
holder within a second.

## Open questions

- **How many renders fit at once** on an 8 GB GPU beside a desktop
  session is untested. GPU memory crowding has broken the GPU tests
  before.
- **What cap suits a build?** The largest compile measured used 4.8 GB.
- **Should `make check` have a fast subset** for changes that touch only
  pack tools or docs? Today every change pays for the full run.
- **A second machine** already pushes to the same branch. Locks are per
  machine; nothing here coordinates across them.

## MVP: a few days

1. `tools/lock` with `build`, `test` and `render`, wired into the
   Makefile's targets, with `status` and bounded waits.
2. `make agent-setup` and `make agent-bin`.
3. `run` and the studio tools defaulting to private output folders.
4. `remodel img sheet`, `diff` and `stats`.

This slice and the scene slice together remove the top-ranked friction.
Neither needs the other to start.
