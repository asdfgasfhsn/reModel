# Changelog

reModel uses [semantic versioning](https://semver.org/). Before 1.0 a minor
version marks a milestone (new games, boards or features) and a patch
version a fix. Saved formats (save states, the settings file, texture
packs) and the documented `make` targets and CLI may still change between
minor versions; 1.0 comes when they are stable. Versions up to 0.6.0 were
tagged after the fact on the commit that reached each milestone.

## 0.6.0 (2026-11-13): Virtua Fighter 2

- Virtua Fighter 2 (`vf2`, Model 2A) plays: the MB86234's circular
  indexed operands, the 2A's 512K-word texture banks, the geometrizer's
  window carried across frames, and the timer read fix (which also fixed
  Fighting Vipers' ponytail).
- Idle-loop skipping for 18 games, exact to the instruction count.

## 0.5.0 (2026-11-13): visual enhancements and the menu redesign

- Smooth shading, material maps and texture packs, depth buffer,
  anti-aliasing and temporal anti-aliasing, smooth-motion in-between
  frames, bloom.
- The in-game menu and the browser's settings panel in tabs, with picture
  presets (Original, Enhanced) and per-game run-ahead caps.

## 0.4.0 (2026-11-09): Model 2A and the TGP

- Sega Rally Championship (`srallyc`) plays: the Model 2A board, the
  MB86234 TGP running the game's uploaded program, the 2A's polygon ROM,
  textures and fixed-function geometrizer.

## 0.3.0 (2026-11-09): the Model 2B games

- Over Rev (2B), Super GT 24h, Dynamite Baseball '97, Dead or Alive,
  Indianapolis 500, Fighting Vipers, Sonic Championship, Virtua Striker and
  Last Bronx on the Model 2B board (two SHARCs), with native geometrizers.
- The launcher with live attract previews, run-ahead, smooth motion,
  bezels, key and gamepad remapping.

## 0.2.0 (2026-11-06): the Model 2C games

- Sega Ski Super G, Sega Touring Car Championship, Sega Water Ski, Wave
  Runner, Over Rev, The House of the Dead (light guns) and Behind Enemy
  Lines join Top Skater on the Model 2C board.
- The browser build: a game list and a page per game, posters.

## 0.1.0 (2026-11-05): Top Skater

- Top Skater (`topskatr`, Model 2C) plays, in a native window and in the
  browser (WebAssembly and WebGPU): the i960 interpreter and recompiler,
  the Model 2C board, the MB86235 geometrizer, live sound.
