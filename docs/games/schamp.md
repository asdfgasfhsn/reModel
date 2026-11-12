# `schamp`

## What's known about `schamp`

Sonic Championship (Sega; the title screen says ©SEGA 1996), the
fifteenth game and the seventh on Model 2B. `schamp.zip` also holds the
program and `.7/.8` chips of its clone `sfight` (Sonic the Fighters, as
the dat names it), which isn't supported.

- *Confirmed (ROM, then the memory test: every ROM, IC 7-16, and RAM
  GOOD):* program `.15/.16`; main data `.11/.12` and `.9/.10`; the 1 MB
  pair `.7/.8` in the second data bank, repeating (read at `0x64xxxxx`);
  polygons `.17/.21`, `.18/.22` and textures `.27/.25`, `.28/.26` (4 MB
  chips) in 8 MB slots; copro data `.29/.30`, the same chips as
  `fvipers`'s.
- *Confirmed (from its programs, then by running it):* the geometrizer
  SHARC program is the code the Jaleco games, `dynabb97` and `doa`
  share (2,987 words; the same crate); the copro program is its own
  (4,870 words), built like `fvipers`'s: it reads and writes buffer RAM
  through bank 2 at the same places. Like `fvipers` it doesn't write
  MSGR7, relies on the i960's list jump patches, and uploads its textures
  to the upper half of each texture RAM bank (Model 2B above).
- *Confirmed (from its copro program):* the program clears its internal
  memory early on and then stores a debug word through an index register
  it hasn't set yet, into IOP register `0x20`, which the ADSP-21062
  manual marks reserved; the SHARC core now ignores writes there
  (`docs/hw/adsp2106x.md`).
- *Confirmed (by the input test, then by running it):* the same ports as
  `fvipers`: bit 0 punch, 1 kick, 2 barrier; directions, coins and starts
  as there (one coin a credit).
- *Confirmed (by running it):* `make run GAME=schamp INPUT=coin-start`
  boots, attracts (the story, the character line-up and a fight demo),
  coins up, takes Sonic and fights Knuckles through rounds and replays
  with sound. `make sound` of that run averages -28 dBFS RMS (peak -8),
  louder than the games at 0 dB, so the manifest takes 4 dB off. A
  coin-start run of 600M instructions gives the state hash
  `0dd9ca5f3388771f` (`8d2b7b4cd55d64a0` before the timers' counts were read a word higher, 2026-11-13, `docs/hw/peripherals.md`) recompiled and interpreted (the game never writes
  its EEPROM in it).
  `make state-check GAME=schamp` round-trips, and `make lockstep
  GAME=schamp RECOMP=1 INPUT=coin-start` runs 600M instructions identical
  (i960 and both SHARCs recompiled). Its geometrizer runs the shared
  crate, its copro program a crate of its own. The recompiled build first
  faulted at boot: code discovery had made an i960 block at `0x20E`, an
  odd address, which took the slot of the buffer RAM clear at `0x20C`;
  `remodel recomp` now drops unaligned instructions
  (`docs/recompiler.md`).
- *Speed (by running it):* that run plays at 2.71x real time recompiled
  natively (the geometrizer on its thread, slowest frame 10.1 ms), and at
  0.78x interpreted (slowest frame 59.1 ms).
- *Confirmed (from the raster stream):* character shadows are opaque
  black polygons that share the characters' own texture header (`4009
  0000 1A00`, an all-15 64×64 texture that the palette colour tints),
  with palette entry 1 (`0x8000`, black) and a fixed luma (205); the
  checker bit is clear and nothing else sets them apart, so they draw
  solid black, as sent. *Unconfirmed:* how they look on the hardware.
- *Confirmed (from its tile RAM):* the story's white captions ("REVENGE
  OF DR. ...") are one line of text longer than a tile map, on plane A in
  left/right split mode with line scroll: the game scrolls the line-scroll
  entries 8 pixels a frame across both maps. They drew with gaps
  ("REVENG...ONIC") because we took the entry as the split point only;
  the tile chip scrolls both maps by it too, as one 1024-pixel plane
  (`docs/hw/tilemap.md`). *Unconfirmed:* not compared with hardware.
- *Input lag, measured (`make input-lag`: pictures with and without the
  control compared pixel by pixel, frames counted after the first that
  reads it; `docs/play.md`, "Run-ahead"):* walking right (frame 3507 of a
  run whose punches and kicks stop at 3400) shows in the first frame that
  reads it (input lag 0), a punch a frame later (1, also from 3907).

## Lighting (smooth shading's side channel)

Measured with `remodel --geo native --light --input coin-start run 900000000` (6196 frames; `make lighting-capture` runs the same), reModel's smooth-shading side channel (docs/hw/geometrizer.md, "Lighting side channel"):

- Self-check: of 19,574,803 polygon records, the flat luma recomputed from the side channel is the stream's for 19,574,684 (100.00%), one level off for 119 and further off for 0 (the SHARC rounds toward zero).
- specular on in 71.28% of object blocks, 53.64% of polygons lit with a specular term; normals computed from the vertices in 62.65% of blocks; zero (unlit) normals on 5.21% of polygons.
- Specular highlights as drawn (`make spec-stats`, the 21 frames of a `make lighting-capture`, 960x720, 2026-11-12): 48% of the textured polygons shown have a specular term (scales 96, 255, 144, squared one to three times), 17% of the textured pixels; it brightens 5.8% of the textured pixels by a level or more and 4.5% by 4 or more, 28 levels on average, up to 199: the fighters (heads, chests, gloves, shoes) and, in attract, the mecha and the space station's hull.
- models drawn from polygon RAM in 2,798 blocks (62,927 polygons, 0.53% of blocks); no direct data.
- the light vector changes within a frame in 258 of 6196 frames (up to 2 vectors a frame); lighting entries not changed within a frame.
- True focal length over the window width: median 0.565 (range 0.565-2.667); the stream-only layer guesses 1.0.
- The list scan's block facts equal the native geometrizer's on 530,910 blocks, differ on 0.

## Smooth shading

The manifest sets a crease angle of 25 degrees instead of the default 40
(`romset::Shading`; docs/enhancements.md, "Smooth shading"). Evidence
(*observed* in reModel's own pictures, `lighting-check --shading pixel
--crease 20/25/30/40/50` on frames 1200 and 2400 of a coin-start capture,
1440x1080): Sonic's quills and Knuckles' dreadlocks are spikes of a few
faces whose ridges meet at roughly 25-40 degrees. At 40 the ridges are
smoothed away and each spike reads as one blob of blue or red; at 30 the
larger ridges start to go; at 25 the ridges stay while the heads, cheeks,
gloves and shoes still shade smoothly; at 20 the faces look nearly flat
again. The angle is a choice by eye, not a property of the data.

## Enhanced preset

What the player's Enhanced picture preset (`romset::Recommended` in the
manifest, `run_ahead_max` for run-ahead) turns on for `schamp`, and why:
Temporal at full size; the depth buffer on (the tangled feet come out
right, nothing broken: `docs/enhancements.md`, "Depth buffer"); smooth
shading with the crease of 25 degrees chosen above; smooth motion;
run-ahead 0 (the stick has no lag; the menu offers 1 for the punches, with
a help line that it makes the stick judder).
