# `overrevb`

## What's known about `overrevb`

Ninth game and the first Model 2B one: Over Rev's 2B version, a clone of
`overrev` in MAME's naming. Its chips are in `overrev.zip` (or an
`overrevb` folder or zip of its own); the manifest's `parent` says so.

- *Confirmed (ROM):* only the program pair differs from `overrev`
  (`epr-19992b.15/.16`); data, polygon, texture and sound chips are the
  same files, in the same order, and the controls are the same.
- *Confirmed (from the code):* its i960 program is a 2B build of the same
  game (same strings, "Sep 10 1997"), with the 2B uploads, UART address and
  copro boot described under Model 2B. It reads the controls, EEPROM and
  link board as `overrev` does.
- *Confirmed (by running it):* `make run GAME=overrevb INPUT=coin-start`
  attracts, coins up, goes through the mode, car and gear menus and races
  the first course with textured 3D that matches `overrev`'s at the same
  frames (sampled every 300) through the menus and into the race; in the
  unsteered race the two drift apart after frame 3,300 (the 2B car loses
  speed in the next curve and falls behind), as two builds of the physics
  can.
  *Unconfirmed* whether the copro maths differs from 2C's anywhere it
  matters. The attract demo runs about 20 frames behind `overrev`'s.
- *Confirmed (by running it):* `make sound GAME=overrevb` plays the run's
  commands, about -37 dBFS RMS through the race as for `overrev`. `make
  state-check GAME=overrevb` round-trips (save states hold both SHARCs),
  `make lockstep GAME=overrevb RECOMP=1` runs 300M instructions
  identical, and a coin-start run of 600M instructions gives the same state
  hash (`ee2bcd89e419e428`, with an `eeprom.bin` from an earlier run)
  recompiled and interpreted.
- *Speed:* the interpreted geometrizer SHARC takes about 20 ms a race
  frame natively (33 ms in WebAssembly), so races run below real time
  (0.86x natively, about half in a browser) and the geometrizer is what
  limits it. The game slows to its pace rather than the polygons falling
  behind the inputs; full speed needs a SHARC recompiler
  (`docs/hw/model2b.md`, "Speed").
- *Confirmed (by running it, `make latency`):* the game reads the wheel
  at the start of each frame and its display list and polygons follow in
  the same frame, on 2B as on 2C: one frame from input to screen at full
  speed in the native player, two in the browser.
- *Input lag, measured (`make input-lag`: pictures with and without the
  control compared pixel by pixel, frames counted after the first that
  reads it; `docs/play.md`, "Run-ahead"):* the wheel (frames 3507 and 3907
  of a coin-start run) shows in the first frame that reads it: input lag
  0, as on 2C.

