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
- When behaviour contradicts MAME (confirmed from the game, or found in
  MAME's code), add or update an entry in `docs/mame-defects.md` with
  evidence and a way to reproduce it, so it can become an upstream patch.
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
- Agents in worktrees build into the shared `target-agents/` directory at
  the repo root (`CARGO_TARGET_DIR=<repo>/target-agents`, under the heavy
  lock), never the main checkout's `target/`: cargo judges freshness by
  file times, so a build of older code from another tree can leave stale
  crates that the main tree's next build keeps (it broke `make play` once).
  Before an agent's final `make check`, `touch` the workspace crates'
  `src/lib.rs`/`src/main.rs` so they rebuild from its own sources.
