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
- *Confirmed (by the pictures):* the 2A polygon ROM, texture ROM and
  texture RAM layouts that make this draw: see `docs/hw/model2a.md`, "Findings", and
  its §5, §7. The pictures are the evidence
  (a wrong pairing gave scattered polygons, a right one the road and the
  cars); no picture or ROM data is in the repository.
- *Confirmed (running the race):* the TGP has a **data ROM** on its
  external bus, from word address `0x800000` (`.29/.28`, `.31/.32`): the
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
- *Confirmed (by running it):* the audio of a race (`--audio`) averages
  -38 dBFS RMS (peak -21) with no gain: the engine and the music play
  (the boot is silent for 15 s, the attract and race hold -37 to -36
  dBFS), quieter than the games at 0 dB, so the manifest adds 6 dB
  (like `overrev` and `sgt24h`), about -32 RMS.
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
  block table over the RAM range; not done. Meanwhile 18% of the
  instructions poll the video status register in a five-instruction
  loop at `0x5A379C`, which an exact idle skip would have to account for
  (instruction count, bus time, the TGP and timers) to keep the state
  hash. The TGP interpreter is cheap: 1.2 s for the 4,141 frames (0.3 ms
  a frame) and 1.6 s for 6,212 frames of a race (21,000 TGP words a frame
  in the race, about 0.13 ms), the geometrizer thread 0.9 s of 31. The
  player needs 1.4x to hold 60 Hz and has 2.3x; run-ahead of 2 frames
  costs two more game frames per frame shown.
- *Input lag, measured (`make input-lag`):* the wheel, from frame 4007
  of a coin-start run with the accelerator held, shows after 1 frame
  (the same from frames 3907 and 4207), as `stcc`'s does.
- *State hash:* a 600M-instruction coin-start run gives `2108cb7ccade31b3`
  with `--geo native` (`56b5c12673c631b3` on the run that creates the
  EEPROM); the save state round trip (`make state-check GAME=srallyc`)
  and run-ahead (`make run-ahead-check GAME=srallyc`) keep it.
- *Not done:* the force feedback (drive board); the meaning of the
  course polygons' attribute words and last four words; the later
  stages and championship; a browser build tried with the chips loose
  (`remodel web-index` lists it).

