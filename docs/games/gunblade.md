# Gunblade NY (`gunblade`, Model 2B-CRX, 1995)

Status: **plays** (listed; the launcher's Model 2B group, with a poster and
bezel), checked in scripted runs: boot, attract, coin-start, the mission
select and the first area of the EASY mission with the mounted gun,
sound. Not yet played by hand. Sega's helicopter door-gun game for two
players; on the 2B board model it needed nothing new.

## What's known

- *Confirmed (ROM, then the memory test):* the manifest verifies (all 17
  chips of `gunblade.zip`, Rev A). The board is **Model 2B-CRX**: the
  program writes both SHARCs' host windows (`0x8C0000` and `0x840000`)
  and the 2B sound UART (`0x9C0000`), and boots, uploads and runs both
  SHARC programs on the 2B board model. Program `epr-18988a.15/.16`
  (1 MB, the only program pair); main data `.11/.12` at `0x2000000` and
  `.9/.10` at `0x2800000`; polygons `.17/.21` then `.18/.22`; textures
  `.27/.25`; copro data `.29/.30` on the copro SHARC's external bus, as
  `indy500`'s (32-bit floats so paired, `.29` low, e.g. 1.0, 500.0, -30.0,
  1000.0 at the start; *unconfirmed* that the copro program reads them);
  sound program `.31`; samples `.32` then `.34` (8 MB from the 68000's
  `0x800000`, as `topskatr`'s two 4 MB chips; *unconfirmed*: by analogy,
  not checked against the sound program's sample table). The test menu's MEMORY TEST shows IC 9-14,
  79-84, 88 and 89 all GOOD in this layout. The chips' roles came from the
  numbering of the other 2B sets and their contents; the emulator's .dat
  was used only as a list of file names.
  *Unconfirmed:* why IC 13 and 14 (the second program pair's sockets,
  empty in this set) test GOOD.
- *Confirmed (by running it):* the copro SHARC program is its own (6,958
  words as uploaded); the geometrizer's (3,117 words, code hash
  `15393321f266b9cf`) is the shared 2B one, so the native geometrizer
  runs it: `make geo-check GAME=gunblade` compares 5,164 frames of a
  coin-start run with the SHARC, 0 differ (native 1.6 ms a frame against
  23.2 ms interpreted). `make run` writes both programs to
  `out/gunblade/sharc-*.bin`; both run with no fault and no host error.
  The boot warning says "used only in Japan"; the title screen says
  ©SEGA 1995.
- *Confirmed (by running it):* the attract draws in 3D, textured: the
  New York streets, the S.A.A.F. helicopter on its pad, the harbour, the
  rankings over gameplay. Nothing is read outside the known map but one
  write and one read of `0x1C00200` at boot (the original Model 2 I/O
  board's buffer, `docs/hw/model2.md` §2; unmapped on 2B), which it
  doesn't wait on (*unconfirmed* what it is probing for).
- *Confirmed (from the game's own input test, by running it):* port 0
  (`0x1C00002`) COIN CHUTE #1 `0x01`, #2 `0x02`, TEST `0x04`, SERVICE
  `0x08`, START 1P `0x10`, START 2P `0x20`; port 1 (`0x1C00004`) TRIGGER
  1P `0x01`, 2P `0x02`. The guns are **potentiometers on the ADC**, as on
  `bel`: GUN X is analog channel 0 for 1P and 1 for 2P, GUN Y channel 2
  for 1P and 3 for 2P (the input test shows each channel's raw value). The test menu (MEMORY TEST, INPUT TEST, OUTPUT TEST, SOUND
  TEST, C.R.T. TEST, GAME ASSIGNMENTS, COIN ASSIGNMENTS, AIM SET,
  BOOKKEEPING, BACKUP DATA CLEAR, EXIT): service moves the cursor, test
  selects. One coin a credit by default.
- *Confirmed (by running it):* **aiming.** With a fresh EEPROM's
  calibration the crosshair moves right as X grows and down as Y grows;
  its centre, measured from pictures with the gun at 17 positions, is
  linear in Y (about 2.87 pixels a step) and nearly so in X (about 4.0
  pixels a step near the left edge, 3.5 near the right). The manifest's
  ranges (X 113-242, Y 30-164 across the 496x384 view, a least-squares
  fit) put it within 4 pixels of the aimed point. Player 2's gun takes the
  same ranges (*unconfirmed*: not measured). AIM SET recalibrates (pull a
  trigger to start); not tried.
- *Confirmed (by running it):* there is **no reloading**: the gun fires
  while the trigger is pulsed, with no ammunition shown, so the manifest's
  reload controls only fire.
- *Confirmed (by running it):* `coin-start` (one coin, start, a shot at
  EASY on SHOOT TO SELECT YOUR MISSION, then shots every 4 frames at the
  middle and either side in turn) flies in by helicopter and plays the
  first area, Times Square: enemies hit and downed, the clock past 76
  seconds by frame 6,450 with all three lives. With the gun left at rest
  (aimed low left) the first area ends in "DAMAGED", CONTINUE? and game
  over, and attract carries on. Without a shot the mission select picks
  after its countdown.
- *Confirmed (by running it):* the idle loop is the wait for the next
  frame at `0x6A65C`-`0x6A673`, a poll of the video status's frame bit
  (`0x98000C` bit 2) counting passes in g0, as `hotd`'s and `indy500`'s:
  82.3% of a coin-start run's instructions are skipped (`make bench
  GAME=gunblade`: state hash `8f1a6b60319ffe13`, with an `eeprom.bin` from
  an earlier run; the same with `--no-idle-skip` and after a save and
  load, `make state-check GAME=gunblade`). Interpreted it runs at 0.8x
  real time on the SHARC geometrizer, 6.8x with the native one (the
  player's default).
- *Confirmed (by running it):* **recompiled** (`make recomp
  GAME=gunblade`): 96,146 i960 instructions in 15,529 blocks, 6,842 of the
  copro's 6,958 words and 2,987 of the geometrizer's 3,117. `make lockstep
  GAME=gunblade RECOMP=1` runs 400M instructions of a coin-start run
  identical (i960 and both SHARCs recompiled), and the bench run gives
  the same state hash at 2.36x real time on the SHARC geometrizer.
- *Confirmed (by running it):* **input lag** (`make input-lag
  GAME=gunblade`, the trigger held from frames 2207, 2407 and 2807 of a
  coin-start run, the aim held still): the picture changes in the first
  frame that can read it, so `input_lag` and `run_ahead_max` are 0.
- *Confirmed (by running it):* **sound**: the 68000 and SCSP board runs
  as on the other 2B games; a coin-start run recorded with `--audio` (114
  s) is silent through the boot and the first attract scenes (18 s), then
  never silent: mean -20.7 dBFS, peak -4.8 dBFS, so `volume_db` is 0.
  *Unconfirmed:* by ear.

## Next

1. Play it by hand, and player 2.
2. A mission played through to its end: the coin-start script is blind.
3. The sound by ear, and the sample order against the sound program's
   sample table.
4. AIM SET, and what `0x1C00200` is for.
