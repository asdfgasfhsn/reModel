# Virtua Cop (`vcop`, the original Model 2, 1994)

Status: **plays** (listed; the launcher's Model 2 group, with a poster and
bezel). The second game on the original Model 2 board
(`Board::Model2`, [`docs/hw/model2.md`](../hw/model2.md)) after
[`daytona`](daytona.md).

## What's known

- *Confirmed:* the manifest verifies (26 chips of `vcop.zip`: revision B's
  program `epr-17166b.12`/`-17167b.13`; `vcopa`'s revision A pair
  `epr-17166a.12`/`-17167a.13` is in the zip too, not added). The
  emulator's .dat was used only as a catalogue of which file names belong
  to which set; the chips' roles below come from the data and the run.
- *Confirmed (by running it):* **the program is 512 KB**, two pairs back
  to back: `.12/.13` at 0 and `.14/.15` at `0x40000` (code runs from both
  halves, `0x40000`-`0x7FFFF` included: the gun routines, the stage
  scripts). Unlike `daytona` its code is linked where it lies; nothing
  runs from the `0x220000` window (one stray constant). The boot record is
  at 0 (PRCB `0xB0`, first IP `0x5D0`, interrupt table `0xD00`, in the
  program ROM).
- *Confirmed (by the pictures):* main data `.10/.11`, `.8/.9`, `.6/.7`
  packed from `0x2000000` (as `daytona`'s; `.6/.7` carry the TGP program
  and the text), polygons the one pair `.16/.20`, textures the one pair
  `.25/.24`, the TGP's maths tables as every 2A set. There is no TGP data
  ROM (no `.28/.29`): the external bus's ROM is empty.
- *Confirmed (by running it):* the TGP program (2,024 words, uploaded at
  boot from `.6/.7`) runs on the MB86234 interpreter with **no unknown
  word or external access**; the game calls about 20 functions.
- *Confirmed (by the pictures):* boot shows the Japan warning, then
  attract: the AM2 logo, the demo play in 3D (the wharf, the lock-on
  sight tutorial "ロックオンサイトが敵を自動探知します。"), the title and
  rankings, textured and lit; "©SEGA 1994".
- *Confirmed (by running it):* the TGP program is `daytona`'s but for its
  last word (8,096 bytes, 4 bytes differ).
- *Confirmed (from the code, and by running it):* **the settings live on
  the I/O board** as in `daytona` (`docs/hw/model2.md` §2): a "SEGA"
  record of 128 bytes copied to SRAM `0x1D00000`. With an empty store the
  game takes its defaults, which run stand-alone (no link setting to
  change): ADVERTISE SOUND ON, COUNTRY JPN, CABINET DX, DIFFICULTY NORMAL,
  LIFE 6. Byte `0x19` (advertise sound, 0 on), `0x1B` (country, 1 JPN, 2
  EXP) and `0x21` (lives - 1) are GAME SYSTEM items, seen changing in the
  test menu; `0x1A` and `0x20` are the menu's other two (cabinet and
  difficulty by their order, *unconfirmed*); `0x24`-`0x33` the guns'
  calibration.
- *Confirmed (from the game's own input test, by running it):* register 8
  (port 0) coin 1 `0x01` (CHUTE #1), coin 2 `0x02`, test `0x04`, service
  `0x08`, START1 `0x10`, START2 `0x20`; register 9 (port 1) TRIGGER1
  `0x01`, TRIGGER2 `0x02`. The test menu: TEST MODE (BOOKKEEPING, GAME
  SYSTEM, COIN ASSIGNMENT, INPUT TEST, OUTPUT TEST, GUN SETTING, SOUND
  TEST, TGP TEST, MEMORY TEST, BACKUP RAM CLEAR, EXIT), service moves the
  cursor, test selects; test and service together leave the input test.
- *Confirmed (from the code, then GUN MARK CHECK):* **the guns** are read
  from their own registers at `0x1C00100` (`docs/hw/model2.md` §2, "The
  light gun registers"): player 1's Y and X, player 2's Y and X, 16 bits
  each, and a "sees no screen" bit a player, the layout of `hotd`'s gun
  counters, so the manifest uses the same `GunSource::Counters`. The game
  maps raw positions through the calibration in its settings to the
  496x384 view; the manifest's ranges are the defaults (player 1 X
  131-630, Y 36-425; player 2 X 128-627, Y 39-425): GUN MARK CHECK puts
  the two marks where the guns aim (raw `E8`/`7D` for player 1 at
  100, 80) and draws no mark for a gun off the screen.
- *Confirmed (from the code, by running it):* **a shot's aim is taken two
  frames after the trigger**: on the trigger the game fills the tile layer
  for four frames (the flash a light gun sees) and latches the guns'
  registers in the second (`0x585xx`, `0x151C`). A shot reloads when the
  "sees no screen" bit is set then, so a reload needs the gun kept off the
  screen for those frames (the coin-start script's aims and off-screen
  pulses come two frames after their triggers; a player's reload control
  holds it while pressed).
- *Confirmed (from the code, `0x25E0`, and the GAME SYSTEM screen):* the
  reload type and the enemies' look are **DIP switches** (I/O register 11,
  `0x1C00016`, bits 7 and 6), not settings: GAME SYSTEM shows them and
  skips them. Read as 0 (every switch on), the game reloads by itself
  ("AUTO RELOAD", bullets refilled about 15 frames after each shot) and
  shows "ROBOT"; the board model reads `0xFF` (every switch off, as idle
  inputs read), which gives manual reloading by shooting off the screen.
  *Unconfirmed:* how a cabinet leaves the factory.
- *Confirmed (by running it):* `coin-start` (one coin, start, a shot at
  STAGE 1 on STAGE SELECT, then shots every 20 frames at the middle and
  either side with an off-screen reload every seventh) plays stage 1: the
  bullets go down a shot at a time and come back on the off-screen shot,
  enemies fall, the score rises (8,200 by frame 2,700), the lives run out
  ("CONTINUE?", "GAME OVER") and attract carries on (the tutorial "サイト
  が赤に変わると敵に撃たれます。", the title, the ranking).
- *Confirmed (by running it):* **two players**: two coins, START1 and
  START2 put both in the game (two HUDs, each with its own cylinder and
  lives), and player 2's gun and trigger shoot and reload as player 1's.
- *Confirmed (by running it):* **sound** on the original board's sound
  board (`sound::PcmBoard`, `epr-17170.7`): the program runs as
  `daytona`'s, the first MultiPCM plays the music with bank 2 and the
  second the effects and voices with bank 1. The first MultiPCM's ROMs are
  two 1 MB chips in 2 MB sockets ([`multipcm.md`](../hw/multipcm.md),
  "Sockets"): packed back to back, bank 2 read past their end and the
  music's banked instruments were silent. `epr-17171.8` is *unknown* (as
  `daytona`'s `.8`: the model maps nothing to it, and the sound program
  runs with no unmapped access). *Unconfirmed:*
  checked by the key-ons and levels, not by ear against a cabinet.
- *Confirmed (by running it):* the idle loop is the wait for the next
  frame at `0x9124`-`0x9137`, a poll of the video status's frame bit
  (`0x98000C` bit 2) as `hotd`'s: 77% of a coin-start run's instructions
  are skipped.
- *Confirmed (by running it):* **input lag** (`make input-lag GAME=vcop`,
  the trigger held from frames 1810, 2010 and 2510 of a coin-start run):
  the picture changes in the first frame that can read it (the flash, in
  the 2D layer), so `input_lag` and `run_ahead_max` are 0.
- *Clones:* `vcopa` (revision A: `epr-17166a.12`/`-17167a.13`, the rest
  shared) is in the zip; not added.

- *Confirmed (by running it):* **stages 2 and 3** (MEDIUM, a quarry, and
  EXPERT, a tower block with a car park) start from STAGE SELECT by shooting
  their panels (the panels' centres are about x 98, 250 and 400 at y 190 of
  the 496x384 view) and draw and play (enemies, the sights, hostages: "MISS!
  LIFE" for a shot hostage). Sound over a 6-minute run of several credits:
  never silent, peak 18,000 of 32,767, no clipping.
- *Confirmed (by running it):* the enemies carry the game's own sights, a
  circle around each that shrinks and goes green, yellow, orange, red and
  then the enemy fires (the glass cracks, a life goes). A throwaway script
  that read the sight's circle off each rendered frame and aimed at it (not
  in the tree) got through the first areas of stage 1 and kept the game
  progressing across continues; a blind sweep of aims does not.

## Next

1. A whole stage shot through to its boss and end (the aimed script above
   was crude: it misses enemies on roofs and ducked behind cover); no fault
   was seen, but no stage has been cleared in a scripted run.
2. The sound by ear against a cabinet (as `daytona`'s).
3. The OUTPUT TEST (lamps, `0x1C0001E`) and the gun's recoil, if any.

## Cheats

Both are work-RAM bytes of player 1's state, found with `make ram-search`
on tours (`stand-still`, `fire` in `crates/romset/src/games/tours/vcop.txt`)
and listed in `crates/romset/src/games/pokes/vcop.txt`. The HUD shows 6
lives (hearts, and "06") and a six-chamber cylinder at the start of play.

| Poke | Address | Width | Value | Label |
|---|---|---|---|---|
| `p1-lives` | `0x50ee70` | byte | 6 | Player 1 never loses a life |
| `p1-ammo` | `0x50ee38` | byte | 6 | Player 1 infinite ammo |

- *Confirmed (by running it):* lives. Standing still in stage 1, the byte
  read 5, 3, 2, 1 at frames 1800, 1920, 2040, 2100, where the HUD showed
  5, 3, 2, 1 hearts (the only byte that did). Held at 6, `lives-held` (the
  poke from frame 0) keeps it at 6 from frame 1500 to 12000 (unpoked, the
  game is over by 2400 and attract resumes), the picture at 3000-9000 is
  stage 1 with the HUD at 06 and the enemy still aiming at the gun. Holding
  it from power-on does not disturb attract or the start.
- *Confirmed (by running it):* ammo. Firing every 40 frames, the byte read
  6, 5, 4, 3, 2 (the only byte); the cylinder shows the same, and with no
  reload a "RELOAD" prompt appears. `ammo-held` reads 6 at frames
  1590-6000 firing every 20 frames, and the picture at 2000 shows a full
  cylinder and no RELOAD. With both held together a render at frame 4500 is
  still stage 1, HUD 06, cylinder full.
- Only meaningful in play (the values are the in-play HUD's). Player 2's
  copies were not looked for (*unconfirmed*; the struct's other player is
  probably a fixed distance away, not tested).
