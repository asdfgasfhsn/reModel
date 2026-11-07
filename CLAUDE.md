# Working on reModel

- The project is spelled `reModel` in prose, titles and UI text (never
  "Remodel" or "ReModel"); lowercase `remodel` stays for commands, crates
  and file names.
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
