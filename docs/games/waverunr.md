# `waverunr`

## What's known about `waverunr`

Fifth game, 2C: a ride-on personal watercraft cabinet, SCSP sound only.

- *Confirmed:* one program pair, `epr-19282a.15`/`epr-19283a.16` (`.15`
  low half: the i960 fault strings read in order). Boot record: PRCB
  `0xB0`, first IP `0x5F0`, interrupt table `0x44C70`, system-procedure
  table `0x1C0`. `epr-19284.31` is CRX_drv Ver3.2.7 (96 9/10).
- *Confirmed (from the code):* main data is `.11/.12` at `0x2000000` and
  `.9/.10` at `0x2800000` (8 MB slots, as in `stcc`): the course tables
  at program `0x8E1B0` point into `0x2C00000-0x2C1FFFF`, and the record
  chains there (a count, that many 32-byte records, until a zero count)
  parse 4 MB into `.9/.10` and nowhere else.
- *Confirmed (from the code):* `.7/.8` (1 MB) is read through a window at
  **`0x06000000`**, the same second data bank as `segawski`'s. The tables at
  program `0x90940` point into `0x6480000-0x64EFFFF`; the pair is blank
  below `0x80000` and holds data at `0x80000-0xEFFFF`, and the same record
  chains parse there with the pair repeating in an 8 MB slot from
  `0x6000000` (as `stcc`'s 1 MB pairs do in theirs). Without it the game
  read nothing there (unmapped) and still ran. reModel calls these chips
  `Role::MainDataHigh`. *Unconfirmed:* how the board decodes the rest of
  `0x04000000-0x07FFFFFF`; nothing else there was read in our runs.
- *Unconfirmed:* the polygon pairs are `.17/.21`, `.18/.22` and the 1 MB
  `.19/.23`; textures `.27/.25` and the 1 MB `.28/.26`; copro data the
  1 MB `.29/.30`. Roles and slot order follow the other 2C sets; the game
  draws correctly with them, but byte order within the 1 MB pairs is by
  analogy only.
- *Confirmed (from the code):* the game copies routines from program ROM
  `0xC0000-0xC7FFF` to work RAM `0x210000` and runs them there (calls to
  `0x217090`, `0x21E1F0`, ...). The recompiler leaves them to the
  interpreter.
- *Confirmed (from the code):* the cabinet board (the body's air
  compressor, valves and motion; boot messages "now cabinet initialize",
  "compressor starting", "marine jet home position", ...) is optional. At
  boot the game reads its status byte from `0x1C00006` and switches on
  `0xC6-0xCF`, `0xE0`, `0xE1`; `0xFF` (nothing attached) skips the whole
  sequence and marks the board absent. With the board absent the input
  test shows "SAFETY SENSOR ERROR" and "EMERGENCY" isn't checked (with a
  board, they'd be `0x1C00006` bits 4 and 6). Nothing is modelled.
  *Unconfirmed:* what `0x1C00008` writes drive (lamps or valves,
  presumably).
- *Confirmed (by running it):* no link reads in a default-settings run
  (two writes to `0x1A14000`). The program has a "net check command"
  string; whether a link setting exists is unconfirmed.
- *Confirmed (from the input test, then by scripted runs):* port
  `0x1C00002` bit 0 coin (three coins a credit by default), 2 test, 3
  service, 6 start, 7 view (the camera seemed to move closer once;
  unconfirmed). Analog channels: 0 handlebar (0x30 steers left, 0xD0
  right; it also moves the course selection), 2 throttle lever (0xFF
  released, 0x00 fully open, about 100 km/h; it also confirms the
  course), 1 the body's roll and 3 its pitch. The handlebar, roll and
  throttle go through calibration tables built from the test menu's
  settings (`0x45668` on). Pitch is read only by the test menus. Roll is
  read in play (a lean more than `0x20` from centre sets a state in the
  craft's physics), but scripted runs leaning both ways showed no
  effect. *Unconfirmed:* what roll does.
- *Confirmed (by running it):* `make capture GAME=waverunr` (coin-start:
  three coins, start, the throttle to pick the first course and held
  open) races the beginner course through a checkpoint until time runs
  out and "GAME OVER". `make sound GAME=waverunr` plays the run's sound
  commands on all 32 SCSP slots, about -30 dBFS through the race. `make
  state-check GAME=waverunr` round-trips, `make lockstep GAME=waverunr`
  runs 300M instructions identical, and a coin-start run of 600M
  instructions gives the same state hash (`97d498bed77503a6`, with an
  `eeprom.bin` from an earlier run) recompiled and interpreted.
- *Confirmed (from the raster stream):* under the rock arch on the
  beginner course the game draws its shadow as one flat dark-blue polygon
  (palette colour 5-bit (0, 0, 6), full luma) over the water and behind
  the craft, with texture header bit 15, the checker (screen-door) flag,
  set. Both renderers applied the checker only to textured polygons, so
  the shadow came out solid and the water flat dark blue; it now drops
  every other pixel and the water shows through. *Unconfirmed:* how it
  looks on the hardware's display (a half-tone, or blended by the
  monitor).
- *Input lag, measured (`make input-lag`: pictures with and without the
  control compared pixel by pixel, frames counted after the first that
  reads it; `docs/play.md`, "Run-ahead"):* the handlebar (frames 3507 and
  3907 of a coin-start run) shows in the first frame that reads it: input
  lag 0.
- *Depth buffer (2026-11-11; captured coin-start frames drawn with it off
  and on at 1440x1080 and 1920x1080 (`make depth-check`), compared with
  the build before the 2026-11-10 rules (`docs/enhancements.md`, "Depth
  buffer")):* 15 of 21 frames differ, by up to 1.1%. The 2026-11-10 rules
  helped: a distant banner's lettering and a tree's branches under its
  foliage, which the depth buffer had covered, draw in the game's order
  again. Can't tell: on a cliff-hugging stretch (frame 3300) a tree on the
  cliff top is hidden by the cliff face, which is 3.3x nearer there; the
  rider's body parts move by a few pixels (1.5-2% of 1/z).

## Lighting (smooth shading's side channel)

Measured with `remodel --geo native --light --input coin-start run 900000000` (6067 frames; `make lighting-capture` runs the same), reModel's smooth-shading side channel (docs/hw/geometrizer.md, "Lighting side channel"):

- Self-check: of 17,257,546 polygon records, the flat luma recomputed from the side channel is the stream's for 17,257,546 (100.00%), one level off for 0 and further off for 0.
- no specular lighting; normals computed from the vertices in 92.04% of blocks; zero (unlit) normals on 0.00% of polygons.
- models drawn from polygon RAM in 37,069 blocks (2,224,140 polygons, 7.77% of blocks); 37,973 blocks of direct data (7.96%).
- one light vector per frame; a lighting entry's values change between objects in 103 frames (206 polygons).
- True focal length over the window width: median 0.602 (range 0.000-0.886); the stream-only layer guesses 1.0.
- The list scan's block facts equal the native geometrizer's on 477,012 blocks, differ on 0.

## Enhanced preset

What the player's Enhanced picture preset (`romset::Recommended` in the
manifest, `run_ahead_max` for run-ahead) turns on for `waverunr`, and why:
Temporal at full size; no depth buffer (a tree on a cliff top is hidden
by the cliff face, which is 3.3x nearer: can't tell, above) and no smooth
shading (no pictures recorded as better); smooth motion; run-ahead 0
(input lag 0).

## Idle loop

- *Confirmed (from the code, then by running it):* the main program
  waits for the next frame in the loop at `0x353C4-0x353D7` (the
  manifest's `idle_loops`; found as the hottest code of a coin-start
  run). It polls the video status register for vblank: `ld 0x98000c`,
  its frame bit (bit 2) compared with the one it read before the loop,
  branching back while they are equal, and `g0` added to on every pass (a
  count of passes nothing in the loop reads). It leaves the loop on the
  first read after vblank toggles the bit.
- *Confirmed (by running it):* with the idle skip (`docs/idle-skip.md`)
  a 600M-instruction coin-start run (`make bench`) fast-forwards 71.9%
  of its instructions through it, and the state hash is unchanged.
