# `lastbrnx`

## What's known about `lastbrnx`

Last Bronx (Sega; Export, Rev A; the title screen says ©SEGA 1996), the
seventeenth game and the ninth on Model 2B. `lastbrnx.zip` also holds the
program pairs of its Japanese and US revisions (`lastbrnxj`, `lastbrnxu`),
which aren't supported as sets of their own.

- *Confirmed (by running each pair; the names are the catalogue's):* the
  Export revision's program is `epr-19061a/19062a`; `epr-19046a/19047a`
  (Japan) shows its boot warning in Japanese ("このゲームはフィクションです",
  "this game is fiction", and a caution about weapons), the other two in
  English ("INSERT COIN(S)", "WAITING FOR A CHALLENGER"). The three
  differ in program only: every other chip is shared, so the manifest
  lists the Export pair.
- *Confirmed (from the code, then by running it):* program `.15/.16`
  (512 KB chips) at `0x0`; main data `.11/.12` at `0x2000000` and `.9/.10`
  at `0x2800000` (4 MB chips, 8 MB a pair); the character-pattern loader
  reads a table of (offset, tile) pairs at `0x2A00000` and the patterns
  after it; polygons `.17/.21` and textures `.27/.25` (2 MB chips); sound
  program `.31`, samples `.32` and `.34` (there is no `.33`). There is no
  `.7/.8` pair and no copro data ROM: **both SHARC programs are in the
  program ROM**, the geometrizer's (3,442 words) at `0x1B10` and the copro's
  (6,566 words) at `0x99F8`, uploaded by the code at `0x6BC8` and `0x98D0`
  (`docs/hw/model2b.md`, "Last Bronx"). Its textures go to the lower
  halves of the texture RAM banks (`0x11000000`, `0x11200000`), as
  `indy500`'s: the banks stay 2 MB. (The memory test wasn't run.)
- *Confirmed (by running it):* four things stopped it, none of them a
  ROM layout, and each was found from the game's own code
  (`docs/hw/model2b.md`):
  - its copro upload sets II6 = 0 (the geometrizer's sets `0x20000`):
    the host DMA's index is decoded by its low 17 bits, and the SHARC
    core now takes an index below `0x20000` as `0x20000 +` it; before,
    the program went nowhere and the copro faulted at its vector;
  - its interrupt dispatcher loads `0xE80000` and `0xE80004` as a pair
    (`ldl`) and services the bits of the second word, so that word holds
    the pending requests, not the enable mask reModel returned: the vblank
    handler ran 100 times a frame, never returned, and the program never
    got past its start (a black screen, 3,115 distinct instructions run in
    600M);
  - the copro program sets the secondary register file (`BIT SET MODE1
    0x400`) and uses the old R0 and R7 in the next instruction: a MODE1
    write takes effect a cycle late on the chip (UM §E.2.1), which the core
    now models (`docs/hw/adsp2106x.md`); without it the copro read random
    external addresses (703 distinct ones in 300M instructions, none now);
  - its life gauges drew empty: the gauge routine writes the fill
    patterns at `0x1181000` and the map draws them from tile `0x80`
    (`0x1081000`): tile RAM is 1 MB, repeating through its 2 MB window
    (`docs/hw/tilemap.md`).
  The 16 other games' 600M coin-start state hashes are unchanged by
  all of these (checked by running each: `topskatr` `400275251269648e`,
  `overrevb` `ee2bcd89e419e428`, `doa` `78f308b4da95cefb`, `vstriker`
  `a4af0c62acc656f3` and the other twelve, each the value its section or
  `docs/hw/geometrizer*.md` gives).
- *Confirmed (from its programs, then by running it):* the geometrizer
  program is the shared one (the one `overrevb`, `schamp` and the others
  upload) with three commands of its own, fn 0x1A-0x1C: a ring of 16-bit
  texture points it flushes into the rasteriser's texture point RAM at
  the end of every frame's list (`docs/hw/geometrizer-2b.md`, "Last
  Bronx's program"). It uploads like `fvipers` (SYSCON and WAIT first,
  C6 the length) and doesn't write MSGR7, so it checks every command
  word's tag; none failed. The copro program (6,566 words) is its own and
  the heaviest of the 2B copros: 425M instructions in a 600M run
  against the geometrizer's 1,618M (`vstriker`'s: 340M and 1,900M).
- *Confirmed (by running it beside the SHARC, `make geo-check
  GAME=lastbrnx`, and the fuzzer):* the native geometrizer
  (`Program::Lastbrnx`) gives the SHARC's stream and state word for word
  over a 4,101-frame coin-start run (108.7M raster words), and over 4,300
  frames of 5 fuzzer seeds; **fn 0x1B**, which transforms a polygon into
  the ring and continues into object drawing, is not implemented (the
  native geometrizer stops at it as at fn 0x1D), because no run of the
  game sends it. *Unconfirmed:* what the texture-point commands are for
  (dynamic texture coordinates for an effect, perhaps); the ring is empty
  in every frame seen, so they change state and emit nothing.
- *Confirmed (by the input test, then by running it):* the same ports as
  `fvipers`: `0x1C00002` bit 0 coin 1, 1 coin 2, 2 test, 3 service, 4
  start 1, 5 start 2 (one coin a credit); ports `0x1C00004` and
  `0x1C00006` for the players: bit 0 punch, 1 kick, 2 guard, 4 down, 5
  up, 6 right, 7 left. The input test showed player 1's up, left, punch,
  kick and guard, player 2's punch, down and right, coin 1, start 1 and
  test; *unconfirmed:* coin 2, start 2, service and player 2's other
  bits (assumed from the layout).
- *Confirmed (by running it):* `make run GAME=lastbrnx INPUT=coin-start`
  boots (the warning, the SEGA logo, "INSERT COIN(S)"), attracts (a fight
  demo in Cross Street with the title), coins up, takes the first fighter
  (Yusaku) and fights Tommy through round 1 (the "FIGHT" call, weapons,
  the life gauges filling, a K.O.) with sound. The audio of that run
  (`--audio`) averages -32.7 dBFS RMS (peak -13), like the games at 0
  dB, so the manifest adds no gain; the sound program's own string reads
  "CRX_drv Ver3.1.1". A coin-start
  run of 600M instructions gives the state hash `38e1f20d016dd508` (the first
  run, which creates the EEPROM, gives `700e5666e46dd508`). `make state-check
  GAME=lastbrnx` round-trips (state hash and audio), and `make lockstep GAME=lastbrnx RECOMP=1
  INPUT=coin-start` runs 600M instructions identical (i960 and both SHARCs recompiled). Its i960 program and both SHARC programs have crates of their own (`docs/recompiler.md`), and the recompiled run, with or without the native geometrizer, gives the same hash.
- *Speed (by running it):* that run plays at 2.88x real time recompiled natively (the
  geometrizer on its thread, slowest frame 10.0 ms), 3.34x with the native
  geometrizer, and at 0.77x interpreted (slowest frame 53 ms; 1.51x with
  the native geometrizer). The native geometrizer takes 1.20 ms a frame
  (worst 4.5) against the SHARC's 21.4 ms interpreted (worst 37.8) and
  5.5 ms recompiled (worst 10.1; `make geo-check`, also identical on the
  recompiled SHARC). The geometrizer is the limit, as on the other 2B
  games, but the copro is no light load.
- *Input lag, measured (`make input-lag`: pictures with and without the
  control compared pixel by pixel, frames counted after the first that
  reads it; `docs/play.md`, "Run-ahead"):* walking right (frame 3507 of a run whose punches and
  kicks stop at 3400) shows in the first frame that reads it (input lag
  0), a punch a frame later (1, also from 3907).
- *Unconfirmed:* the clones aren't supported: their program pairs run on
  the same data, but the manifest lists the Export pair. Which of the two
  halves of a gauge is which player's, and the details of the survival
  mode, haven't been looked at.
- *Unconfirmed (`make temporal-check`, frames 3300-3599 of a coin-start
  run, 2026-11-10):* in blocks that smooth motion's plan matched, 3% of
  the position words of frame N's list posed with frame N-1's transforms
  don't agree with frame N-1's own (96.9%; 100% on the other games
  checked), some moved as a whole and some reshaped. On screen only 0.2%
  of pixels' motion vectors are more than a pixel off. Probably limb parts
  paired with a different transform; not looked into.

## Lighting (smooth shading's side channel)

Measured with `remodel --geo native --light --input coin-start run 900000000` (6173 frames; `make lighting-capture` runs the same), reModel's smooth-shading side channel (docs/hw/geometrizer.md, "Lighting side channel"):

- Self-check: of 19,573,461 polygon records, the flat luma recomputed from the side channel is the stream's for 19,573,415 (100.00%), one level off for 46 and further off for 0 (the SHARC rounds toward zero).
- specular on in 99.51% of object blocks, 90.64% of polygons lit with a specular term; normals computed from the vertices in 64.15% of blocks; zero (unlit) normals on 9.29% of polygons.
- models drawn from polygon RAM in 356,492 blocks (1,209,732 polygons, 37.07% of blocks); 4,699 blocks of direct data (0.49%).
- the light vector changes within a frame in 186 of 6173 frames (up to 2 vectors a frame); lighting entries not changed within a frame.
- True focal length over the window width: median 1.017 (range 1.002-4.303); the stream-only layer guesses 1.0.
- The list scan's block facts equal the native geometrizer's on 961,794 blocks, differ on 0 (3 before the list first set the focal lengths).
- Over a third of its object blocks draw models from polygon RAM (the
  fighters' joint pieces the list rewrites every frame): smooth shading
  derives everything per frame from the frame's own polygons and keeps no
  per-model data.
