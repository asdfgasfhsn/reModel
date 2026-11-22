# Working on reModel

- The project is spelled `reModel` in prose, titles and UI text (never
  "Remodel" or "ReModel"); lowercase `remodel` stays for commands, crates
  and file names.
- Use the root `Makefile` for everything: `make help` lists targets. Run
  `make check` before considering a change done.
- When adding a build, test or tool entry point, add a `make` target with a
  `## description` comment so `make help` stays complete, and update the
  README's quick start if it's user-facing (the full command catalogue is
  `docs/usage.md`).
- Keep Model 2-generic code (CPU, ROM loading, analysis, sound, renderer)
  separate from game-specific code; per-game data lives in
  `crates/romset/src/games/`.
- Never commit ROM files, ROM-derived listings, or large excerpts of game
  code. `rom/` and `out/` are git-ignored.
- Record confirmed hardware or game findings in `docs/games/<set>.md` (a
  game's) or the board docs in `docs/hw/` (`model2a.md`, `model2b.md`,
  `model2c.md`, ...), and mark anything unconfirmed as such. Game progress
  goes in the status table in `docs/status.md`. The README stays a short
  front page: keep findings and tracking out of it, and link new docs from
  its documentation map.
- Work clean-room: never copy or consult other emulators' code (MAME,
  Supermodel, ElSemi's or others). Facts come from running the games,
  the hardware and public chip manuals.
- When the game's behaviour contradicts MAME (confirmed by running the
  game, or from the hardware), add or update an entry in
  `docs/mame-defects.md` with evidence and a way to reproduce it, so it
  can become an upstream patch.
- When a game starts working, give it a poster in the same step: original
  artwork made with the Codex CLI's image generation, following
  `assets/posters/README.md` (no text, logos, characters or real brands),
  with its prompt recorded there.
- Scratch files (traces, test renders, unpacked data, agent work) go in
  `out/scratch/`, which is on disk and git-ignored, not in `/tmp`: here
  `/tmp` is RAM-backed and filling it starves the machine of memory.
  Delete scratch output once a task is done.
- Every `remodel` and `remodel-play` run is memory-capped: the Makefile
  runs them through `tools/capped` (`MEMCAP`, default 8G), so a runaway
  run is killed on its own instead of the machine's other work. Run the
  binaries through `make`, or prefix a direct run with `tools/capped`;
  raise `MEMCAP` for a run that genuinely needs more, never drop the cap.
- Agents render with the CLI (`make look-render`, `make studio-compare`,
  `run` snapshots, `--shot`), not long-lived visible player windows: each
  window also costs the desktop compositor GPU memory, which has crowded
  out the GPU tests. When the GPU is full anyway, `make check GPU_TESTS=0`
  skips the GPU tests with a warning; run them before pushing.
- Agents in worktrees build into their own `target-agents/NAME/` at the
  repo root (`make agent-setup` sets `CARGO_TARGET_DIR`), never the main
  checkout's `target/` and never a target dir another agent uses: cargo
  judges freshness by file times, so a build of another tree can leave
  stale crates (it broke `make play` once), and with one shared dir one
  agent's build replaced another's release binary between its build and
  its link tests (2026-11-23). Delete your `target-agents/NAME/` when done.
  Before an agent's final `make check`, `touch` the workspace crates'
  `src/lib.rs`/`src/main.rs` so they rebuild from its own sources.
- Locks are `tools/lock`, not a hand-typed `flock out/heavy.lock`: `make`
  takes them itself (builds, `make test`/`check`: `build`, plus `test` for
  test runs; the GPU render targets: `render`, three at once), so a render
  does not wait behind a build. `make lock-status` (or `tools/lock status`)
  says who holds what and who waits; `LOCK_WAIT=SECS` bounds a wait (exit
  3, naming the holder: do other work or hand back, never a sleep loop).
  To run something else heavy by hand: `tools/lock build -- COMMAND`. The
  old `flock out/heavy.lock make check` still works, since build and test
  also honour that file for now (`REMODEL_LEGACY_LOCK=0` turns it off).
- A new worktree: run `make agent-setup` first (links `rom/`, copies each
  game's EEPROM so benchmark hashes match, makes your scratch folder, writes
  `out/agent.env` with the shared target dir; `run` then writes to a private
  folder in your scratch). An agent that only checks pictures and never
  builds runs `make agent-bin` once in a tree (a read-only copy of the CLI
  in `out/bin/COMMIT/`) and then `make agent-setup ART=1`, after which
  `make` uses that binary and does not build.
