# Working on Remodel

- Use the root `Makefile` for everything: `make help` lists targets. Run
  `make check` before considering a change done.
- When adding a build, test or tool entry point, add a `make` target with a
  `## description` comment so `make help` stays complete, and update the
  README's quick start if it's user-facing.
- Keep Model 2-generic code (CPU, ROM loading, analysis, sound, renderer)
  separate from game-specific code; per-game data lives in
  `crates/romset/src/games/`.
- Never commit ROM files, ROM-derived listings, or large excerpts of game
  code. `rom/` and `out/` are git-ignored.
- Record confirmed hardware or game findings in the README's "What's known"
  section, and mark anything unconfirmed as such.
