# Sky Target (`skytargt`, Model 2A-CRX, 1995)

Status: **plays** in scripted runs (in `IN_PROGRESS`, not yet listed):
boot, attract with its 3D flight demo and weapon screens, coin-start, the
fighter select, the carrier launch and mission 1 to its boss, continue and
game over, sound. Not yet played by hand. The fourth Model 2A game.

## What's known

- *Confirmed:* the manifest verifies (27 chips of `skytargt.zip`; the
  ramps `mpr-16310.15`, `-16311.16`, `-16312.14` are unused, as on every
  2A set).
- *Confirmed (by running it):* the program is the one pair `.12/.13`
  (1 MB). The boot record is at 0 (PRCB `0xB0`, first IP `0x5F0`,
  interrupt table `0x40BC0`).
- *Confirmed (by the pictures):* main data `.10/.11` and `.8/.9` (4 MB
  chips, a pair filling its 8 MB slot) from `0x2000000`; polygons
  `.16/.20` and `.17/.21` packed from `0x800000` as on the other 2A sets;
  textures the one pair `.25/.24`; the TGP's maths tables as every 2A
  set.
- *Confirmed (from the code and the data):* **`.6/.7` (two 512 KB EPROMs)
  are the second main data bank, at `0x6000000`** (`Role::MainDataHigh`,
  repeating through its slot), not after `.8/.9`: the game reads
  `0x6400000`-`0x646xxxx` (6,202 addresses unmapped in a 600M-instruction
  run before), and its palette code (`0x232D0`) copies the 3D polygon
  colours to palette RAM `0x1802000` from a count and list at `0x640FB5C`,
  which in `.6/.7` (offset `0xFB5C`) is a count of `0x112` and 15-bit
  colours (`0x8000`, `0xFC00`, `0x83E0`, ...). Without it every 3D polygon
  took colour 0 and drew black (the attract's flight and the weapon
  screens' fighter were missing on a black background). The other
  pairs hold `0xFFFF` and zeros there. `segawski` and `waverunr` (2C) use
  the same bank; this is the first 2A game seen to.
- *Confirmed (from the data, then by running it):* **the TGP's data ROM is
  `.28/.29` with `.28` the low half**, as `daytona`'s and unlike
  `srallyc`'s: so paired its first words are counts and floats (2, 6, 5.0,
  10.0, -10.0, -5.0, ..., 1.0, 10.0); the other way round they are float
  halves swapped. Paired as `srallyc`'s, the TGP's function `0x55` (a
  collision test over the ROM's polygon lists) read a count from garbage,
  ran a million words without waiting at every call (8,583 "stuck"
  accesses) and the attract slowed to a crawl from frame 1,362.
- *Confirmed (by running it):* the TGP program (2,859 words, hash
  `68f47ba0e7a7f9a8`) runs on the MB86234 interpreter with no unknown word
  once two encodings were added (`docs/hw/mb86234.md`): `38 imm` loads P
  (the product register) and ALU code 3 (a hypothesis: XOR). The game
  calls 27 functions in a coin-start run.
- *Confirmed (from the game's own input test, by running it):* port 0
  (`0x1C00002`) COIN #1 `0x01`, COIN #2 `0x02`, TEST `0x04`, SERVICE
  `0x08`, VIEW CHANGE `0x20`, START `0x40` (`srallyc`'s layout); port 1
  (`0x1C00004`) MACHINE GUN `0x10`, MISSILE `0x20`; STICK LR is analog
  channel 2 and STICK UD channel 0 (the screen shows the values given).
  Nothing else on the ports responds. The test menu (MEMORY TEST, T.G.P.
  TEST, INPUT TEST, OUTPUT TEST, SOUND TEST, C.R.T. TEST, GAME
  ASSIGNMENTS, COIN ASSIGNMENTS, VOLUME SETTING, BOOKKEEPING, BACKUP DATA
  CLEAR, EXIT): service moves the cursor, test selects.
- *Confirmed (by running it):* the stick at `0x80` is centred; below it
  moves the plane left (banking) and down the screen, above it right and
  up (`stickx`, and `sticky` with its pad axis and keys inverted so that
  up moves the plane up). Held left during the fighter select it picks
  another fighter. The machine gun fires while held (pulsed every ten
  frames it scored nothing; held, three hits and 4,760 points by frame
  2,500).
- *Confirmed (by running it):* two coins a credit by default ("2 CREDITS
  to 1PLAY"). `coin-start` (two coins, start, the machine gun held, a
  missile every second) reaches the fighter select, the carrier launch,
  "MISSION 1" (frame ~1,500) and its boss (a bomber, "DANGER", a time
  limit), then CONTINUE with the mission chart, game over and attract.
- *Confirmed (from the sound program's own table and the chips' contents):*
  the samples are `.31 .32 .36 .37` back to back from the sound 68000's
  `0x800000`, as `srallyc`'s and `vcop2`'s: the sample table at 68000
  `0x60AA02` (16-byte entries, 737 samples) puts samples in all four 2 MB
  slots, none crossing one, and 45 of its 121 loops repeat their first
  512 bytes after their end in this order, 28 in the next best. Sound runs
  on the SCSP board: a coin-start run's commands render (`make sound`) at
  a peak of -14.4 dBFS, RMS -29 to -36 dBFS a 5 s block, no clipping.
  *Unconfirmed:* by ear.
- *Confirmed (by running it):* the idle loop is the wait for the next
  frame at `0x379D4`-`0x379E7`, a poll of the video status's frame bit
  (`0x98000C` bit 2) counting passes in g0, as `hotd`'s: 56% of a
  coin-start run's instructions are skipped (`make bench GAME=skytargt`,
  5x real time interpreted, state hash `9a102fc85df71fbc`, the same with
  `--no-idle-skip` and after a save and load, `make state-check`). The
  game keeps settings in the EEPROM (44 words programmed), so a run's
  hash depends on `out/skytargt/eeprom.bin`.
- *Confirmed (by running it):* **input lag** (`make input-lag
  GAME=skytargt`): the stick shows in the first frame that reads it (from
  frames 2207, 2507 and 2707 of a coin-start run), the machine gun a frame
  later (from frame 2507), so `input_lag` and `run_ahead_max` are 1.

## Next

1. Play it by hand; listing, poster, recompiled and browser builds.
2. A mission flown to its end (the script flies straight and loses to
   mission 1's boss), and the other fighters.
3. The sound by ear; OUTPUT TEST (lamps); what VIEW CHANGE does in play.
4. ALU code 3 (XOR) checked against more of the program than the one
   use (`0xB09`).
