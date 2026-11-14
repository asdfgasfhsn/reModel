# Virtua Cop 2 (`vcop2`, Model 2A-CRX, 1995)

Status: **plays** in scripted runs (in `IN_PROGRESS`, not yet listed):
boot, attract, coin-start, the stage select and all three stages, light
guns with shots, hits and the off-screen reload, two players, sound. Not
yet played by hand. The third Model 2A game after
[`srallyc`](srallyc.md) and [`vf2`](vf2.md), and the second gun game on
the board model's gun interface after [`hotd`](hotd.md); it needed nothing
new from the board model.

## What's known

- *Confirmed:* the manifest verifies (27 chips of `vcop2.zip`; the ramps
  `mpr-16310.15`, `-16311.16`, `-16312.14` are in the zip and unused, as
  on every 2A set). The emulator's .dat was not needed; the chips' roles
  come from the numbering of the other 2A sets and were checked by
  running the game.
- *Confirmed (by running it):* the program is `.12/.13` (1 MB, the code)
  then `.14/.15` at `0x100000` (floats: data). The boot record is at 0
  (PRCB `0xC0`, first IP `0x860`, interrupt table `0x1A80`, in the program
  ROM).
- *Confirmed (by the pictures):* main data `.10/.11 .8/.9 .6/.7 .4/.5`
  packed from `0x2000000`, polygons the one pair `.16/.20`, textures the
  one pair `.25/.24`, the TGP's maths tables as every 2A set; no TGP data
  ROM. The attract (a street with buildings, trees and cars, "©SEGA 1995",
  the lock-on sight tutorial "ロックオンサイトが敵を自動探知します。")
  and the three stages draw in 3D, textured and lit.
- *Confirmed (by running it):* the TGP program (1,778 words, hash
  `36d89d4375697844`) runs on the MB86234 interpreter with **no unknown
  word**; its only unknown external address is port `0x2C`, read once, as
  in `vf2` (`docs/hw/mb86234.md`, "Not known"). The game calls 31
  functions in a coin-start run.
- *Confirmed (from the code, `0x97C`-`0x9B0`):* the program runs on either
  I/O board. At boot it writes 1 to `0x1C00024` (the original Model 2 I/O
  board's register 18, `docs/hw/model2.md` §2) and reads it back: if it
  reads 1 it takes the original board's register layout, else (flag
  `0x500F00` = 1) the 2A's 315-5649 layout, which is what the board model
  gives (the address is unmapped on 2A). It also writes "SEGA" to
  `0x1C00034`-`0x1C0003A` and commands to `0x1C00040`, which nothing on the
  2A answers; the game does not wait on them.
- *Confirmed (from the game's own input test, by running it):* port 0
  (`0x1C00002`) CHUTE #1 `0x01`, CHUTE #2 `0x02`, TEST-SW `0x04`,
  SERVICE-SW `0x08`, START1 `0x10`, START2 `0x20`; port 1 (`0x1C00004`)
  TRIGGER1 `0x01`, TRIGGER2 `0x02`. The test menu (MEMORY TEST, T.G.P.
  TEST, INPUT TEST, OUTPUT TEST, SOUND TEST, C.R.T. TEST, GAME
  ASSIGNMENTS, COIN ASSIGNMENTS, GUN SETTING, BOOKKEEPING, BACKUP DATA
  CLEAR, EXIT): service moves the cursor, test selects. One coin a credit
  by default.
- *Confirmed (from the code, `0x1380` and `0x37230`-`0x3741C`):* **the
  guns** are read through `hotd`'s gun interface on I/O registers 9-13
  (`docs/hw/model2c.md`, `model2::gun`): the game sets registers 1 and 8,
  streams a 5,120-byte configuration image from program ROM
  `0x5F050`-`0x6044F` through command `0x07`, then reads counters 0/1
  (player 1's Y), 2/3 (its X), 4-7 (player 2's) and 8 (the "sees no
  screen" bits) with command `0x87`. So the manifest uses
  `GunSource::Counters` as `hotd` does. This is the 2C gun interface
  answering on a 2A board's I/O chip (*confirmed* that the game needs it:
  without an answer it would spin at `0x135C`).
- *Confirmed (GUN SETTING, GUN MARK CHECK):* the defaults of a fresh
  backup RAM are player 1 X 137-630, Y 36-425 and player 2 X 134-627,
  Y 36-425 (the screen shows them as AD1/AD2 XMIN, YMIN, XMAX, YMAX), and
  the game maps them linearly onto the whole 496x384 view: with those
  ranges in the manifest the marks land on the aimed pixel at the corners
  and centre of the view (the mark's sprite centre half a pixel left and
  1.5 up), for both guns.
- *Confirmed (by running it):* **a shot's aim is taken two frames after
  the trigger**, as in `vcop`: an off-screen pulse on frames +1 and +2 or
  +2 and +3 after a trigger reloads, on frame +0 or +3 alone it does not.
  The game's default is manual reloading (shoot off the screen: the
  "RELOAD 画面外を撃て!" prompt when the cylinder is empty); the
  cylinder holds six.
- *Confirmed (by running it):* `coin-start` (one coin, start, a shot at
  BEGINNER on STAGE SELECT, then shots every 20 frames at the middle and
  either side with an off-screen reload every seventh) plays stage 1:
  hits (sparks on the enemies, score 1,649 by the continue), the lives
  running out, "CONTINUE?", game over, and attract carries on. MEDIUM
  (a pier, shot at the middle panel) and EXPERT (a subway station, shot at
  x 400, y 150) start and play.
- *Confirmed (by running it):* **two players**: two coins, START1 and
  START2 put both in (two cylinders and lives), and player 2's gun shoots
  and reloads off screen.
- *Confirmed (from the sound program's own table and the chips'
  contents):* **the samples are `.31 .32 .36 .37`, back to back** from the
  sound 68000's `0x800000`, as `srallyc`'s. The sound program
  `epr-18530.30` has a sample table at 68000 `0x609488` of **16-byte**
  entries (start, length, loop start, loop length; `srallyc`'s are 20
  bytes), 503 samples: every 2 MB slot starts a sample at its first byte,
  none crosses a slot, and each chip's data (to its trailing `0xFF` fill)
  ends 4-7 bytes before its slot's last sample does only in this order
  (`.31` slot 0, `.32` slot 1, `.36` slot 2, `.37` slot 3). Of the 91
  looped samples 24 repeat their loop's first 512 bytes after its end in
  this order, 19 in the next best.
- *Confirmed (by running it):* **sound**: the 68000 and SCSP board runs
  as on `srallyc` and `vf2`, and a coin-start run's commands render with
  `make sound` (110 s: never silent after the first seconds, peak 3,190 of
  32,767, -20 dBFS, RMS -36 to -43 dBFS a 5 s block, no clipping).
  *Unconfirmed:* by ear, and the level: `volume_db` is 0 (a listening
  choice, `srallyc` has 10).
- *Confirmed (by running it):* the idle loop is the wait for the next
  frame at `0x36FC`-`0x3707`, a poll of the work RAM byte `0x500000` until
  an interrupt handler changes it, as `vf2`'s: 84-85% of a coin-start
  run's instructions are skipped (`make bench GAME=vcop2`: 10x real time
  interpreted; state hash `e31f9a0f51649643`, the same with
  `--no-idle-skip` and after a save and load, `make state-check
  GAME=vcop2`).
- *Confirmed (by running it):* **input lag** (`make input-lag
  GAME=vcop2`, the trigger held from frames 2010, 2210 and 2510 of a
  coin-start run): the picture changes in the first frame that can read
  it, so `input_lag` and `run_ahead_max` are 0.

## Next

1. Play it by hand (`make play GAME=vcop2` once listed), then listing,
   poster, recompiled build and browser build.
2. The sound by ear, and its level (`volume_db`).
3. A stage shot through to its boss: the coin-start script is blind and
   loses its lives in the first areas.
4. The OUTPUT TEST (lamps) and GAME ASSIGNMENTS' settings (where they live
   in backup RAM).
