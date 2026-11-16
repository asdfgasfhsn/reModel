# `segawski`

## What's known about `segawski`

Sega Water Ski, a different game from Ski Super G (water skiing behind a
boat, not snow), on the same 2C board. The set is the Japanese release
(the boot warning says "used only in Japan"; the program's build date is
Aug 22 1997).

- *Confirmed (from the code and the game's memory test):* program
  `epr-19963a.15`/`epr-19964a.16` at `0x0` (`.15` low); main data `.11/.12`
  at `0x2000000` and `.9/.10` at `0x2800000` (its pointer tables point
  into `0x2800000`), and a second data bank: `.7/.8` at `0x6000000` (the
  font is loaded from `0x622BF70`, and its own pointer tables point into
  `0x6000000`) and the 1 MB pair `.5/.6` at `0x6800000`, read 4 MB into
  its slot at `0x6C0xxxx` (sprite headers and palettes like `.7/.8`'s), so
  it repeats as `stcc`'s small pairs do. Nothing executes or reads above
  `0x100000` in program space. With this layout the memory test passes
  IC15, 16, 11, 12, 9, 10, 7, 8, 5 and 6, which it checks in that order.
  `romset::Role::MainDataHigh` holds the second bank; the bus maps it at
  `0x6000000`-`0x7FFFFFF` only for games that have it. *Unconfirmed:*
  what the board decodes at `0x4000000`-`0x5FFFFFF` and above
  `0x7000000` (nothing reads there).
- *Confirmed:* polygons are four pairs (`.17/.21`, `.18/.22`, `.19/.23`,
  `.20/.24`, 32 MB), textures two (`.27/.25`, `.28/.26`), copro data
  `.29/.30`, samples `.32/.34`; `epr-19967.31` is CRX_drv Ver3.4.8 (97
  8/20), and `make sound GAME=segawski` plays a run's commands.
- *Confirmed (by running it):* it needs no drive or music board. It never
  waits on the drive board registers; before a run it shows "the foot
  controller is being unlocked, take care", the output test's SLIDE LOCK.
- *Confirmed (from the input test, then by scripted runs):* port
  `0x1C00002` bits 0/1 coins (one credit is two coins by default; the test
  shows one COIN CHUTE), 2 test, 3 service, 6 SELECT(DOWN); port
  `0x1C00004` bit 0 SET (starts a game, confirms), 1 SELECT(UP), 2 PITCH
  LEFT, 3 PITCH RIGHT; analog channel 0 SLIDE (`0x10` swings the skier
  left, `0xF0` right). *Unconfirmed:* what the pitch switches do; the game
  shows a "jump!" prompt with a foot-tilt icon at ramps.
- *Confirmed (by running it):* coin-start reaches player select, course
  select (Palm Beach, Au-Pau River, Whale Park) and a run at Palm Beach;
  the attract demos ski in 3D. `make state-check GAME=segawski`
  round-trips, `make lockstep GAME=segawski` runs 300M instructions
  identical, and a coin-start run of 600M instructions gives the same
  state hash (`505a972b9e8a3c89`) recompiled and interpreted.
- *Confirmed (from the code, then by running it):* the memory test's
  IC55 and IC54 are 64 KB RAMs at `0x10A00000` and `0x10C00000` (see
  `docs/hw/model2c.md`); with them modelled every chip
  in the test reads GOOD.
- *Input lag, measured (`make input-lag`: pictures with and without the
  control compared pixel by pixel, frames counted after the first that
  reads it; `docs/play.md`, "Run-ahead"):* leaning the board (`slide`,
  frames 3507 and 3907 of a coin-start run) shows in the first frame that
  reads it: input lag 0.

## Lighting (smooth shading's side channel)

Measured with `remodel --geo native --light --input coin-start run 900000000` (6071 frames; `make lighting-capture` runs the same), reModel's smooth-shading side channel (docs/hw/geometrizer.md, "Lighting side channel"):

- Self-check: of 16,812,062 polygon records, the flat luma recomputed from the side channel is the stream's for 16,812,062 (100.00%), one level off for 0 and further off for 0.
- no specular lighting; normals computed from the vertices in 75.92% of blocks; zero (unlit) normals on 0.00% of polygons.
- models drawn from polygon RAM in 49,574 blocks (2,178,788 polygons, 10.18% of blocks); 117,303 blocks of direct data (24.08%).
- the light vector changes within a frame in 212 of 6071 frames (up to 2 vectors a frame); a lighting entry's values change between objects in 2467 frames (16,685 polygons).
- True focal length over the window width: median 0.602 (range 0.602-4.861); the stream-only layer guesses 1.0.
- The list scan's block facts equal the native geometrizer's on 487,164 blocks, differ on 0.

## Enhanced preset

What the player's Smooth modern picture style (the `enhanced` preset; `romset::Recommended` in the
manifest, `run_ahead_max` for run-ahead) turns on for `segawski`, and why:
Temporal at full size; the depth buffer on (the skier's back and jersey
text and logo come out right after the 2026-11-10 rules, nothing broken:
`docs/enhancements.md`, "Depth buffer"); smooth shading (the life vest
is clearly better in the review pictures); smooth motion; run-ahead 0
(input lag 0).

## Idle loop

- *Confirmed (from the code, then by running it):* the main program
  waits for the next frame in the loop at `0x4FD24-0x4FD37` (the
  manifest's `idle_loops`; found as the hottest code of a coin-start
  run). It polls the video status register for vblank: `ld 0x98000c`,
  its frame bit (bit 2) compared with the one it read before the loop,
  branching back while they are equal, and `g0` added to on every pass (a
  count of passes nothing in the loop reads). It leaves the loop on the
  first read after vblank toggles the bit.
- *Confirmed (by running it):* with the idle skip (`docs/idle-skip.md`)
  a 600M-instruction coin-start run (`make bench`) fast-forwards 78.7%
  of its instructions through it, and the state hash is unchanged.
