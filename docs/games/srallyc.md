# `srallyc`

## What's known about `srallyc`

Sega Rally Championship (Sega; revision C; ©SEGA 1994 on the attract
screens, and the boot warning "used only in Japan"), the seventeenth game
manifest and the first Model 2A one. `srallyc.zip` also holds the older
program revision `b` and two chips named `ic12.bin`/`ic13.bin` (the program
pair again); they aren't used. **It plays** (`make play GAME=srallyc`,
the launcher's Model 2A group, the browser's list when its chips are
loose in `rom/srallyc/`): attract, car and course select, and races.

- *Confirmed (by running it):* `make run GAME=srallyc INPUT=coin-start
  STEPS=700000000 SNAP=100` boots the i960 program through the memory
  setup, uploads the TGP program at frame 79, passes the load check
  (the program, running, answers it) and runs the attract mode: the
  "WARNING ... used only in Japan" screen (frame 500), the "SEGA RALLY
  CHAMPIONSHIP 1995" title logo (frame 1,000), and from frame 1,350 **the
  3D**: a rally car and its rival jumping a dirt road through a forest
  with a cloudy sky, a rally car head on over an "INSERT COIN(S)" sign, the
  same road at a distance with dust at the cars' wheels, all textured and
  lit, behind `CREDIT 0/2`, `©SEGA 1994` and the logo; then the
  "CHAMPIONSHIP TOP 16" ranking table (frame 3,000). The default setting
  asks for **two coins** per credit: `coin-start` pays twice, presses start
  and gets to the CAR SELECT screen (frame 1,800: "select with the wheel,
  confirm with the accelerator", four cars, AT or MT), then the course map,
  the start line (a gate, crowd, banners, "SEGA RALLY CHAMPIONSHIP" boards,
  the dashboard and a rear-view mirror) and the countdown (frames 1,900 to
  2,790), all drawn. **Then the car drives** (milestone 5): released it
  stays put; held, the accelerator pulls it through the automatic's gears
  to 214 km/h; the wheel steers it through the town's first right-hand
  bend at 200 km/h, over the hills and onto the dirt road through a
  checkpoint (30 s of race with three scripted turns, `docs/hw/model2a.md`
  §9 has the script); barriers slow and deflect it, a hard turn into one
  spins it round ("wrong way"), and in the air it falls at 10 m/s². The
  sound board boots from `epr-17890.30` and plays.
- *Confirmed (captures, 2026-11-11):* the race **fades in from white
  through the colour translation tables** (`0x1810000`), not the polygons'
  luma: in a coin-start capture, 6061 of the 6144 entries the renderer
  reads (3 channels x 32 components x 64 lumas) differ at frame 2700 from
  every other captured frame, which all agree. Texture keys do not include
  those tables, so the race textures were dumped in the fade, near white,
  until the dump kept the most drawn look (`docs/texture-packs.md`,
  "Looks").
- *Confirmed (by the pictures):* the 2A polygon ROM, texture ROM and
  texture RAM layouts that make this draw: see `docs/hw/model2a.md`, "Findings", and
  its §5, §7. The pictures are the evidence
  (a wrong pairing gave scattered polygons, a right one the road and the
  cars); no picture or ROM data is in the repository.
- *Confirmed (running the race):* the TGP has a **data ROM** on its
  external bus, from word address `0x800000` (`.29/.28`): the
  course's ground, as cells of polygons that functions `7`, `0x52` and
  `0x53` search for the polygon under each wheel (the car fell through the
  road without it); its address registers are 24 bits wide
  (`docs/hw/model2a.md` §9, `docs/hw/mb86234.md`).
- *Confirmed (milestone 5):* why the car ran away after the start in
  milestone 4: the search's loop word `BF 00` is a **loop on a counter**
  (register `0x0C`, loaded with the cell's polygon count), not "jump if
  the zero flag is clear". The old reading ran the search past each
  cell's polygons into other cells' (16 passes in a cell of 6) and gave
  the game the ground of other places. The ROM's cells pin the count
  (their polygons and attribute words lie exactly count x 16 words
  apart), `make tgp-check` tells the two readings apart on a made-up
  course, and with the counter the race drives (`docs/hw/mb86234.md`).
- *Confirmed (from the game's input test, by holding each bit):* coin 1
  and 2 are port 0 bits `0x01`, `0x02`, test and service `0x04`, `0x08`, VR
  (the view button) `0x20` and start `0x40`; the wheel, accelerator and
  brake are analog channels 0, 1 and 2.
- *Confirmed (by driving):* the wheel is centred at `0x80`, lower steers
  left and higher right; the pedals are `0x00` released and `0xFF`
  pressed. The default keys are stcc's: arrows to steer, accelerate and
  brake, Enter to start, C to coin, V for the view.
- *Confirmed (the game's input polling, and driving a manual car):* the
  **shift lever** is an H gate, port 1 bits 4-6 read as a code the game
  turns into a gear with a table: 1st is `0x40`+`0x10`, 2nd `0x40`+`0x20`,
  3rd `0x10`, 4th `0x20`, anything else neutral. That is why the input
  test's SHIFT1 and SHIFT2 answered to no single bit. The manifest has
  them as `gear1`-`gear4`; they matter only with an MT car, which the
  wheel picks on the car select screen. A scripted input holds the gear
  button for as long as the gear is in. The players **latch** the lever
  (`romset::Shifter`, `model2::input`): a gear key (Q, W, E, R) puts it
  in that gear until another is chosen, A and Z step up and down through
  the gate (down from first is neutral, up from neutral first) and X is
  neutral; the pad's right and left shoulder buttons step. No other
  game's inputs or hashes change.
- *Confirmed:* the ROM chips' layout (`docs/hw/model2a.md` §5): the main
  data pairs are packed 4 MB apart (`.10/.11`, `.8/.9`, `.6/.7`), the
  texels the game copies to `0x12000000` come from `0x2200000`, the
  vertex data it reads from `0x2860000`.
- *Confirmed (by running it):* the game calls 33 of the TGP program's
  functions (numbers 1 to `0x5F`), each always with the same number of
  argument and result words. 3D calls begin at frame 1,352.
  `docs/hw/model2a.md` §4 lists them with call counts, §6 with what each
  does.
- *Confirmed:* the TGP's program has a 128-entry dispatch table, function
  F at word `0xAF + F` (`docs/hw/mb86234.md`; the first milestone's notes
  were one word off).
- *Confirmed (milestone 3):* the TGP **runs the uploaded program** on the
  MB86234 interpreter (`crates/mb86234`, `model2::tgp2a`): a 4,832-frame
  coin-start run executes 101 million TGP words, all of known forms. The
  functions: a matrix stack (push at depth under 7, pop, set, read, reset,
  identity), translate (`t += v M`), scale (rows), rotations about x, y, z
  by float angles and by 16-bit ones from the ROM table, transform a point,
  normalise, point along a direction, vector registers, a distance, a 2D
  cross product (`0x5C`), billboards, and function 5, which writes the
  current matrix into the geometrizer's display list. Milestone 2's model
  of them (`model2::tgp2a_hle`, `--tgp2a hle`) had the rotations' signs
  backwards and `0x5C` wrong, and didn't know `0x54`, `0x55` or the list
  output; `make tgp-check` compares it.
- *Unconfirmed:* the 2A geometrizer's own arithmetic, lighting and
  clipping (the 2C native geometrizer's stands in: the pictures look
  right); the cabinet's drive board (the game writes its command byte; it
  isn't modelled) and link; whether the car's quick acceleration and
  braking (100 km/h in 1.2 s, 2-3 g) are the cabinet's (the gravity and
  speedometer agree with the motion, so they are taken to be the game's
  tuning).
- *Confirmed (a scripted race):* `make run GAME=srallyc INPUT=coin-start,
  gas=0xff@2300-9000,wheel=0xa0@3085-3165,wheel=0x70@3360-3420,
  wheel=0xb0@3590-3640,wheel=0xa0@4760-4840 STEPS=1500000000 SNAP=250`
  (the accelerator held and five scripted turns, steering nothing else:
  the car crashes about its way) drives the Desert stage from the
  start line over the town, the hills and the dirt road to the goal:
  "DESERT STAGE CLEAR! RANK 15th" at 1'13"23, the car shown from behind
  on the result screen; then the Forest stage on tarmac and pines
  ("RETIRED" when the time runs out at
  about frame 9,500, then the stage table "DESERT 1'13"23, FOREST
  RETIRED"). The pictures hold up throughout: the HUD (time, lap time,
  position, speed, gear, tachometer, the rear-view mirror, the course
  bar), the turn-sign boards and the stage
  results; no drawing glitch was seen. *Not done:* the
  later stages, the second and third laps of a longer course and the
  finish of a whole championship, which a person has to steer.
- *Confirmed (by running it):* the audio of a race (`--audio`, the
  player's mix) was measured at -38 dBFS RMS (peak -21) with no gain while
  half the samples were missing (next item), so the manifest added 6 dB.
  With all four sample chips, a race at +6 dB mixes at -28 dBFS RMS, about
  -26 LUFS, peaks -10 to -11 dBFS, no clipping; the other games' gameplay
  sits at -16 to -28 LUFS (median about -21.5, measured the same way,
  coin-start runs), so the manifest now adds **10 dB**: about -22 LUFS,
  peaks about -7 dBFS. *Pending a listening check* (the 6 dB was tuned on
  the broken audio; `volume_db: 6.0` is the one to go back to).
- *Confirmed (by the sound program's own table, the chips' contents and
  running it; by ear, 2026-11-10):* **the samples are four 2 MB chips,
  `.31 .32 .36 .37`, back to back** at the sound 68000's
  `0x800000-0xFFFFFF`, each 16-bit chip word byte-swapped as the 68000
  sees it. The evidence, reproducible from the ROMs:
  - The sound program `epr-17890.30` has a **sample table** at 68000
    `0x609C26`: 20-byte entries (start, length, loop start, loop length,
    an envelope word), 304 samples. Every slot of the window begins a
    sample at its first byte and no sample crosses a 2 MB boundary: four
    separately packed chips. A coin-start run's driver (180 s captured,
    135 s rendered with `sound-render`) read 48.3 MB of sample ROM, all
    inside table entries (38.4 MB from `0x800000`, 2.6 MB from
    `0xA00000`, 0.4 MB from `0xC00000`, 7.0 MB from `0xE00000`), and read
    the same addresses whatever the chips held (logged with every order
    below and with all four blank): the addresses come from the table,
    the chips only supply data.
  - The driver streams in 512-byte copies and wraps a loop only after
    copying the whole 512 bytes, so the ROM must repeat each loop's start
    after its end. Of 110 looped samples, 78 carry that exact 512-byte
    repeat, all of them with `.31` in the first slot, `.32` the second,
    `.36` the third and `.37` the fourth, and **none** with any chip in
    any other slot (the other 32 repeat in no chip). Over the 24 orders,
    the next best explains 57.
  - Each chip's data (up to its trailing `0xFF` fill) ends where its
    slot's last sample does: `.31` 512 bytes after (that sample's loop
    repeat), `.32` 5 bytes before, `.36` and `.37` 1 byte after; every
    other chip and slot pair misses by 0x70B bytes to 61 KB.
  - The four chips are 8-bit PCM by their statistics (4-7% `0xFF`, mean
    step between bytes 5-8 in the 68000's byte order against 8-12
    swapped), unlike the TGP's `.29/.28` (53% `0xFF`, step 35 either way,
    both halves ending at the same offset, as one 32-bit bus).
  - `.31/.32` had been taken for a second bank of the TGP's data ROM (at
    its `0x900000`). The TGP never reads there: over a coin-start race
    (1.5G instructions) it reads only `0x800000-0x81FFFF` (2.5 million
    reads), and none in the attract mode, and the race's state hash is
    the same with either labelling. With only `.36/.37` (the old layout),
    174 of the 304 table samples were blank (18.9% of the bytes this run
    streams; 72% in an earlier, shorter capture), and the rest came from
    the wrong chips: most instruments and much of the music played
    silence, a DC level or other samples' data (wrong notes). With all
    four the music sounds right (by ear, 2026-11-10; not compared with a
    cabinet).
  - The driver uses no FM, no LFO depth and no DSP effects (its DSP
    program is all zeros, no EFSDL), only 8-bit PCM voices with loop
    modes 0 and 1 streamed through 8 KB buffers, which refill in time
    (0.07% of buffer reads overrun).
- *Confirmed (by running it, 2026-11-11; the cause is a sound chip
  fault, not this game's data):* **the co-driver's corner calls
  stuttered** (reported by ear) because the SCSP model restarted
  finished one-shot slots. A call is one table sample (e.g. entry 157,
  `0xC3DA24`, 12,806 bytes, 0.58 s at 22.05 kHz) streamed through an 8 KB
  buffer: LPCTL 1 and LEA `0x1FFF` while it streams, then LPCTL 0 with LEA
  at the sample's end in the buffer for the last part. The driver leaves
  KYONB set when the call ends and keys the slot off about 0.5-1 s later;
  in between, any other slot's KYONEX made the model key it on again, so
  it replayed from the buffer's start to that LEA: the call's last
  stretch (0.21 s for entry 157, 51 ms for entry 151, 2 to 10 times).
  Over a 108 s race with three scripted turns, all 20 complete calls
  were replayed (94 restarts); with key-on as an edge of the latched key
  (`docs/hw/sound.md` §2.1) there are none, and a soloed call's slot
  matches the ROM sample sample for sample to its end and then stays
  silent. Reproduce: `make run GAME=srallyc STEPS=900000000
  INPUT=coin-start,gas=0xff@2300-6000,wheel=0xa0@3085-3165,wheel=0x70@3360-3420,wheel=0xb0@3590-3640`,
  then `remodel --game srallyc sound-render` with `REMODEL_SOUND_TRACE=1`:
  the key trace lists each restart as a second `on` for a slot with no
  `off` between. *Not confirmed:* which syllable a listener hears
  repeated (the restarts replay each call's end), and a check by ear.
- *Speed (by running it):* **the game's code runs from work RAM**, not
  the program ROM: of the 14,584 distinct instructions a coin-start run
  executes, 14,502 are at `0x5A0000-0x5FCB3C` (put there at boot by the
  bootstrap in the ROM, `0x420-0x5E4`, which is the 82 others; where
  from was not traced).
  Two things follow. (1) The interpreter's decoded-instruction cache
  covered only the ROM, so every one of those instructions was decoded
  afresh and its result thrown away (about 290 cycles an instruction
  against about 90 for `stcc`, measured with the time stamp counter around
  `Cpu::step`). It now keeps decodes of code in writable memory too,
  checked against the words in memory on every use (so changed code is
  decoded again; a test covers it): a 600M-instruction coin-start run
  went from 1.42x to 2.3x real time, the state hash unchanged (7.3 ms
  a frame; the TGP interpreter and the geometrizer were not the
  cost, `--tgp2a hle` makes no difference). (2) The recompiler works
  from the program ROM image, so the recompiled `srallyc` has 839 blocks,
  all in that bootstrap, none of the game: recompiling gave the same speed
  as interpreting (1.41x vs 1.42x) because it covers no more than the
  82 instructions. Recompiling code that lives in RAM would need the
  code captured from a run, a guard that the RAM still holds it, and the
  block table over the RAM range; not done. Most of the instructions
  (86% of a 600M-instruction coin-start run's, 80% of each frame's
  in a race; an earlier note here said 18%, which is not what a
  coin-start run does) poll the video
  status register in a five-instruction loop at `0x5A379C`, which the
  idle skip now fast-forwards through ("Idle loop", below). The TGP interpreter is cheap: 1.2 s for the 4,141 frames (0.3 ms
  a frame) and 1.6 s for 6,212 frames of a race (21,000 TGP words a frame
  in the race, about 0.13 ms), the geometrizer thread 0.9 s of 31. The
  player needs 1.4x to hold 60 Hz and has 2.3x; run-ahead of 2 frames
  costs two more game frames per frame shown.
- *Input lag, measured (`make input-lag`):* the wheel, from frame 4007
  of a coin-start run with the accelerator held, shows after 1 frame
  (the same from frames 3907 and 4207), as `stcc`'s does.
- *State hash:* a 600M-instruction coin-start run gives `45fbec5e9d1215cf`
  (`2108cb7ccade31b3` before the timers' counts were read a word higher,
  2026-11-13, `docs/hw/peripherals.md`; the game reads timer 2 that way,
  and its pictures over the run look the same)
  with `--geo native` (before the fix, `56b5c12673c631b3` on the run that
  creates the EEPROM); the save state round trip (`make state-check GAME=srallyc`)
  and run-ahead (`make run-ahead-check GAME=srallyc`) keep it.
- *Not done:* the force feedback (drive board); the meaning of the
  course polygons' attribute words and last four words; the later
  stages and championship; a browser build tried with the chips loose
  (`remodel web-index` lists it).

## Lighting (smooth shading's side channel)

Measured with `remodel --geo native --light --input coin-start run 900000000` (6212 frames; `make lighting-capture` runs the same), reModel's smooth-shading side channel (docs/hw/geometrizer.md, "Lighting side channel"):

- Self-check: of 16,786,826 polygon records, the flat luma recomputed from the side channel is the stream's for 16,786,826 (100.00%), one level off for 0 and further off for 0.
- specular on in 100.00% of object blocks, 71.42% of polygons lit with a specular term; normals computed from the vertices in 0.00% of blocks; zero (unlit) normals on 15.89% of polygons.
- Specular highlights as drawn (`make spec-stats`, the 21 frames of a `make lighting-capture`, 960x720, 2026-11-12): 36% of the textured polygons shown have a specular term (scale 255, 96 or 64, squared twice), 32% of the textured pixels, and it brightens none of them by even a level: under a level on 82% of those pixels, the surface already at 255 on the rest. *Unconfirmed:* this is the formula the 2A geometrizer is taken to share (above).
- models drawn from polygon RAM in 27,173 blocks (1,393,987 polygons, 3.86% of blocks); no direct data.
- the light vector changes within a frame in 1653 of 6212 frames (up to 2 vectors a frame); lighting entries not changed within a frame.
- True focal length over the window width: median 0.625 (range 0.258-1.032); the stream-only layer guesses 1.0.
- The list scan's block facts equal the native geometrizer's on 703,322 blocks, differ on 0.
- The exact match is reModel's 2A model against itself (`Program::Fixed`,
  the 2C shared program's maths): it confirms the side channel, not the 2A
  hardware's lighting, which stays *unconfirmed*.

## Enhanced preset

What the player's Smooth modern picture style (the `enhanced` preset; `romset::Recommended` in the
manifest, `run_ahead_max` for run-ahead) turns on for `srallyc`, and why:
Temporal at full size; no depth buffer or smooth shading (little or no
change on mostly flat cars and scenery: `docs/play.md`); smooth motion;
run-ahead 1, which needs the machine to have twice the game's speed to
spare.

## Slow frames (`--perf-log`)

- *Confirmed (by running it, 2026-11-27):* the slow frame every 60 frames
  (1, 61, ..., 1381, 1441; 18 to 28 ms against a 0.4 ms median with the
  JIT) was **not** the geometry chip's program, and not the JIT. It was
  photo mode's history (`model2::photo::History`, on by default, `--photo-history`),
  which takes a save state of the machine every 60 frames on the game's
  thread. Sega Rally's state is 24 MB (its RAMs), and `save_state` grew a
  fresh buffer by doubling and faulted it in: 20.4 ms each (`photo
  history` line of `--perf-log`). `--photo-history 0` removed every slow
  redraw (0 of 1500); an earlier note here blamed the TGP's program
  because `--geo native` got rid of them (an unexplained side effect).
  Headless `run`, which has no history, shows no 60-frame period, and the
  TGP's own runs are 1 to 10 ms at worst, spread over the game (timed
  around each run of the chip). Fixed: the history reuses the buffers
  its compressing threads are done with (`Machine::save_state_into`), so
  a state costs a copy: 4.5 ms each, 0 slow redraws in 1500 frames, p99
  4.9 ms (from 19.4), worst 15.3 ms (from 28.2). The saved bytes are the
  same, so the state hash is unchanged. The same cause is behind the
  17.8 ms at frame 1381 (1381 = 23 x 60 + 1) with the interpreter.
  Games with smaller states paid less, so showed it less.

## Idle loop

- *Confirmed (from the code, then by running it):* the main program
  waits for the next frame in the loop at `0x5A379C-0x5A37B3` (the
  manifest's `idle_loops`; found as the hottest code of a coin-start
  run). It polls the video status register for vblank: `ld 0x98000c`,
  its frame bit (bit 2) compared with the one it read before the loop,
  branching back while they are equal, and `g0` added to on every pass (a
  count of passes nothing in the loop reads). It leaves the loop on the
  first read after vblank toggles the bit. The code is in work RAM,
  where the game copies it at boot, so it always runs on the
  interpreter.
- *Confirmed (by running it):* with the idle skip (`docs/idle-skip.md`)
  a 600M-instruction coin-start run (`make bench`) fast-forwards 85.6%
  of its instructions through it, and the state hash is unchanged.

## Linked play (two to four real machines in one process; in the player as Link play)

In the player it is **Link play** (`make play GAME=srallyc PLAYERS=2`, the
launcher's Link play button; two to four machines on one split screen, one
controller each: `docs/play.md`, "Link play"). The game's side of it is its
`romset::LinkPlay` entry (`crates/romset/src/games/link.rs`): up to four
cabinets, LINK TYPE at EEPROM byte `0x0B` (CAR 1 to CAR 4), the settings
record's defaults and CRC.

The full account, with evidence, is in `docs/hw/link-board.md`; the
headless harness is `make link-run GAME=srallyc` (off by default: N machines
joined by a link hub, run in lockstep; section 9 of that document). The
first part of this list was *confirmed* against `make link-trace
GAME=srallyc LINK=fake,0,2` (a scripted partner), the second against the
two-machine spike:

- The game links only if the board is present (`0x1A14000` bit 0 clear; today
  the slot reads "none", and the game then resets its link and cabinet type
  to 0) **and** the EEPROM's LINK TYPE (test mode, GAME ASSIGNMENTS;
  image byte `0x0B` of the 128-byte `eeprom.bin`, work RAM `0x20201B`) is
  not NOTLINK: CAR 1 is the
  master, CAR 2-4 slaves, RELAY a relay. No DIP or input bit is involved.
- Handshake: write 1 to `0x1A14000` (the master first writes 1 to
  `0x1A14002`), poll `0x1A10000` for 1, then read the node ID
  (`0x1A10002`, **1-based**: the master is 1) and node count (`0x1A10003`); two barriers on the nodes'
  state words follow, then attract runs with the link on.
- Per frame, to the end of the race: one 448-byte block written at
  `0x1A12000` (state word, then the car's position, orientation, wheel
  floats, the race clock), and reads of the received blocks from `0x1A121C0`
  (**the game's own block, echoed back, is the last one**, `N - 1`; the
  others come first, a node's ring successor in block 0). Nothing
  waits on a partner's data once the barriers pass; with the partner's
  blocks 3 or 8 frames late the race still ran, a silent partner changed
  nothing visible, and the board's ready byte dropping (more than 10
  polls) leaves a race running alone (`POSITION 1/1`) and restarts the
  setup and attract modes.
- Run this way, a mirror partner (the game's own block, echoed as the
  other node's) takes it through attract, car select and a race with
  `POSITION 1/2`.
- **Two real copies of the game, joined by `LinkHub` (`make link-run`), run
  the whole link together**: the handshake (ready at frame 926, the board
  screens "THIS IS MASTER CONTROLLER" and "THIS IS SLAVE MACHINE", the two
  barriers at 1228 and 1351), the link attract (the "2 players can battle"
  screens, and its `0x18C` ring: 1 at 1360, go 2 at 1362, 0 at 1363), coin-up
  (frame 1560 in the run) to car select, the Desert race starting at frame
  **3103**, each car in the other's picture, `POSITION 1/2` and `2/2`, the
  results ("RETIRED RANK 1st" and "2nd"), and back to attract (frame 9982).
  Three and four machines do the same (`x / 4` on the screens). The slave
  copies the master's settings to its EEPROM at the first handshake.
- *Confirmed (by running it): FREE PLAY is the game's own COIN ASSIGNMENTS
  setting #26.* Test menu (`test@1300-1330`, then the service button moves the cursor and the test button selects):
  COIN ASSIGNMENTS shows "COIN/CREDIT SETTING #12" (2 coins 1 credit, both
  chutes) at the defaults; the test button on the setting line steps the
  number through #13 to #25 (the ladders of coins per credit), then #26, whose chutes both
  read FREE PLAY, then #1 to #11 again. Leaving the page, then the menu (the
  settings are written when the test menu is left), and diffing the 128-byte
  `eeprom.bin` of that run against one that entered and left the page without changing
  anything: byte `0x1E` 0x0B -> 0x19 (the setting number, 0-based: #12 -> #26), byte `0x04` 0x02 -> 0x03, byte `0x14` 0xF2 -> 0xFA
  and byte `0x1A` 0x02 -> 0x01 (what the game derives from it: not told apart
  further), then the header checksum at `0x00` (the same CRC-16 as above, which
  the game itself recomputed). With that image as the EEPROM, a boot shows "FREE PLAY"
  at the bottom of the attract screens (where it reads "INSERT COIN(S)"
  otherwise), and one start press, no coin, goes to CAR SELECT. In reModel
  that is `romset::LinkPlay::free_play` (the four bytes and the checksum fixed; a
  test builds the image and compares it with these values), applied with the
  linked play settings: `make link-run GAME=srallyc LINK_OPTS=--free-play
  INPUT=start@1560-1564,gas=0xff@3430-99999` races two machines with no
  coins (`make link-free-play-test`; "Free play" in `docs/play.md`).
- It needed two corrections to the first spike's guesses: the ID register
  is **1-based** and a node's own block is the **last** received one; with
  a 0-based ID and the own block first, two real machines hang the link
  attract for ever (a mirror partner could not show it).
- The other nodes' blocks may arrive 1 to 12 frames after they were sent
  (the delay D) without any reset or visible fault; the combined run is
  reproducible (the same combined hash on the interpreter, the JIT and
  with one thread) and a save of both machines **and the hub** resumes
  exactly. The board's flags are part of the state: save the machines
  without them and the link drops.
