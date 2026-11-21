# Manx TT Superbike (`manxtt`, Model 2A-CRX, 1995)

Status: **plays** (listed; the launcher's Model 2A group, with a poster and
bezel). It boots past its motion platform's
check, runs its attract mode in 3D, and a coin-start race rides, on both
courses, with the automatic or the manual gearbox, to the end of the
clock and back to the attract mode, with sound. The deluxe set (revision
D) is `manxtt`; the twin-cabinet set (revision C) is `manxttc`, a clone
(below). Not yet played by hand; no race has been steered to a finished
lap.

## What's known

- *Confirmed:* the manifest verifies (35 chips of `manxtt.zip`). The
  emulator's .dat was used only as a catalogue of the set's file names;
  the chips' roles come from the data and the run.
- *Confirmed (by running it):* the program is **512 KB**, `.12/.13` then
  `.14/.15`, as `vf2`'s; the boot record at 0 (PRCB `0xB0`, first IP
  `0x440`). Main data `.10/.11 .8/.9 .6/.7 .4/.5` packed from `0x2000000`.
  The TGP program (1,782 words) is uploaded at frame 54 and runs on the
  MB86234 interpreter with no unknown word and no unknown external
  access, in attract and races.
- *Confirmed (from the data, then the pictures):* the polygon ROMs pair
  `.16/.20`, `.17/.21`, `.18/.22` (`srallyc`'s pairing). Joining two chips'
  16-bit halves, the share of words that read as floats of a sane size is
  0.92, 0.88 and 0.95 for these pairs against 0.76-0.89 for the other
  high halves and about 0.14 for two low halves together. The texture ROM
  is one pair, `.25` (even words) and `.24`, as `srallyc`'s. The attract
  and races draw textured and lit with them.
- *Confirmed (from the TGP's replies and by running it):* **the TGP's data
  ROM is `.28` (low half) with `.29`**, the order of `daytona`'s pair and
  the reverse of `srallyc`'s. With `srallyc`'s order function 7 (a word of
  the ROM) answered `0x00B40000` for the course data's first word where a
  count belongs (`0xB4` = 180 with the halves the other way), the course
  searches (functions `0x52`/`0x53`) found no polygon under any wheel, and
  the TGP read the function unit's unknown ports 1 to `0xC` (from
  pointers into the missing data). The attract's bike then stood on the
  start line and from about frame 1,800 the i960 never left a loop at
  `0x2CFF0` that takes 10.0 from a distance until it is below 10: the
  distance had grown past where subtracting 10 changes a float. Swapped,
  the TGP has no unknown access and the attract rides the course.
- *Confirmed (from the game's code, by running it):* **the "Active Motion
  Slider"** (the bike's moving platform) is checked at boot: "DANGER!
  ACTIVE MOTION SLIDER NOW CHECKING SYSTEM". The I/O routine reads a
  reply byte from `0x1C00006` (stored at RAM `0x514035`) and writes a
  command byte from RAM `0x514034` to `0x1C00008` each frame, as
  `skisuprg`'s platform does. The check (`0x5F530`-`0x5F868`): a reply
  `0xB0`-`0xB7` at its start is a named fault; it sends `0x3D` until the
  reply is `0xCE` (846 frames allowed), then `0xFF` minus a calibration
  value from the settings (RAM `0x51406F`; `0x7F` here) until the reply is
  `0xCD` (564 frames allowed), and the attract follows; otherwise it
  shows the fault and waits. With nothing answering (the port read as
  `0xFF`) the game stayed on that screen. The board model
  (`DriveBoard::MotionSlider`, `model2::drive::MotionSlider`) answers
  `0xCE` while the game sends `0x3D` and `0xCD` once it has sent the
  setting; with it the check passes in a few frames. What the board
  answers to the motion commands of a race (`0x5D`, `0xC9`, ... seen) is
  *unknown*; the game was not seen to check them.
- *Confirmed (from the game's own input test):* TEST MENU (`test` held at
  boot or in attract; SERVICE moves, TEST chooses: MEMORY TEST, INPUT
  TEST, SOUND TEST, C.R.T TEST, GAME ASSIGNMENTS, COIN ASSIGNMENTS,
  BOOKKEEPING, BACKUP DATA CLEAR, OUTPUT TEST, EXIT). INPUT TEST lists, and
  holding one port bit at a time lit:
  - port 0: COIN CHUTE #1 `0x01`, COIN CHUTE #2 `0x02`, TEST `0x04` (it
    leaves the screen), SERVICE `0x08`, START / VR `0x40` (one button);
  - port 1: SHIFT UP `0x10`, SHIFT DOWN `0x20`;
  - no other bit of ports 0-2 shows anything;
  - THROTTLE analog channel 0, BRAKE 1, BANK 2 (values set in a script
    show as given), each listed with a range `20H`-`E0H`, BANK with a
    middle of `80H`.
- *Confirmed (by riding):* BANK below `0x80` leans the bike right and
  turns it right, above `0x80` left. The game leans the bike (and rolls
  the view) straight from the bank's position: from level to a full
  35-degree roll within 4 frames of a full bank. So a key held would
  throw the bike over at once: the keys move the bank through a
  `KeyRamp` (12 frames to full lean, 8 back; a choice, the cabinet's
  slider takes a body's weight to move). THROTTLE at `0xE0` pulls away
  through the automatic's gears (170 km/h in about 5 s on Laxey Coast);
  at rest (`0x20`) the bike stays on the line.
- *Confirmed (by running it):* one coin is a credit and the game goes
  straight to ACTIVE MOTION SLIDER OPERATION BASICS ("PUSH START
  BUTTON"), then COURSE SELECT (LAXEY COAST, T.T.COURSE) and TRANSMISSION
  SELECT (AT, MT 6 SPEED). Both select by leaning ("バイクを倒して選択":
  right picks the right-hand choice) and the choice follows the lean,
  so the lean must be held until the screen's timer runs out or START
  confirms. Braking while confirming the course gives "TT mode"
  (the screen says; not tried). `coin-start` (one coin, START) takes the
  defaults: Laxey Coast with the automatic. With T.T. COURSE and MT the
  race is a "2-LAP RACE" from a grid of eight, and SHIFT UP steps the
  gears (five presses a second apart at speed; the dial read 1, 2, 4 and
  5 in snapshots between them, and the bike stays in 1st without them).
- *Confirmed (by running it):* a coin-start race with the throttle held
  and no steering runs into walls until the clock runs out, then "YOUR
  RANK IS 8th", and the attract mode; a 13,800-frame run (two credits:
  two races) had no hang and no unknown TGP access.
- *Confirmed (by running it; not yet by ear):* the sound board (68000 +
  SCSP, sound program `epr-18824a.30`) runs the game's driver: a 4,832-frame
  coin-start race sends 10,447 host bytes, all delivered; `sound-render`
  plays 2,073 key-ons over all 32 slots, about -32 dBFS RMS in the race.
  The player's live mix of the twin's race measures -24.1 LUFS, peak
  -8.8 dBFS, inside the other games' range, so the manifest adds no gain.
  The samples `.31 .32 .36 .37` back to back from the 68000's
  `0x800000` are **by analogy** with `srallyc` and `vf2`: this sound
  program has no sample table of their form (a search for theirs finds
  none), so the order is *unconfirmed*. Each chip's data runs to within
  110 KB of its end.
- *Unconfirmed:* the deluxe set's extra chips. `epr-18742.7` (128 KB)
  reads as a byte-swapped 68000 program (reset PC `0x300`) and
  `mpr-18743.32` (2 MB) as data; neither is in the twin set and nothing
  in a run reads them. Presumably a second sound board of the deluxe
  cabinet (as the original Model 2's sound board has its program at
  `.7` and samples at `.32`); not modelled. The main sound board's MIDI
  out (MOBUF) is never written.

## Idle loop

*Confirmed (from the code, by running it):* the main program waits for
the next frame in the loop at `0x3F1C-0x3F33`, polling the video
status's frame bit (`0x98000C` bit 2) until vblank toggles it and
counting passes in g0, as `srallyc`'s loop does. The manifest's
`idle_loops` names it; over a 4,832-frame coin-start race it skips 78.4%
of the instructions (8.4x real time against 2.6x), with the same state
hash as `--no-idle-skip`.

## Input lag

`make input-lag GAME=manxtt`: **2 frames** for the bank (`0x30` held in
the race from frame 3,300) and for the brake: the display list first
changes two frames after the first frame that can read the input. The
manifest's `input_lag` and `run_ahead_max` are 2.

## `manxttc`

The twin-cabinet release (revision C), in its own `manxttc.zip`; the
catalogue gives it no parent, but every chip is `manxtt`'s except the
program (`epr-18822c..18825c`), the sound program (`epr-18826.30`) and
the first sample ROM (`mpr-18827.31`), and it lacks the deluxe's
`epr-18742.7`/`mpr-18743.32`. It is a clone here (`parent: manxtt`: its
shared chips are also found in `manxtt.zip`), with `manxtt`'s controls.
*Confirmed (by running it):* it checks the motion slider at boot the same
way (so the twin cabinet has the slider too), its frame wait is at the
same address, and attract, coin-start and a race play as `manxtt`'s;
save states round-trip.

## Open

- Steer a race to a finished lap and a checkpoint; play by hand.
- The sample ROM order, by the sound program and by ear.
- The speedometer's needle may be 3D (it moves with the HUD only if it
  is named in `hud_element_windows`): not checked for wide views.
- The link board (`0x1A14000`) reads as absent; linked play not looked
  at.

## Link play (survey)

- *Confirmed (by running it):* GAME ASSIGNMENTS has **LINK TYPE** (`Not Link`, `Master`, `Slave`) and BIKE COLOR (No.). Found by driving the test menu with scripted input (`test@1300-1330` held to open it, `service@A-B/20` to step the cursor, `test@F` to select or change a value) and diffing the `eeprom.bin` of runs that changed the value, the menu left through EXIT so the game saved it.
  LINK TYPE is **byte `0x0B` of the 128-byte EEPROM image**: 0 Not Link, 1 Master, 2 Slave. The record is
  `srallyc`'s kind: a CRC-16 at bytes 0-1 (CCITT, start `0xFFFF`, complemented, stored low byte first),
  a length word `0x0038` at bytes 2-3, the settings after; the CRC covers bytes 2 to 55 (recomputed from two
  game-written images). Byte `0x04` also goes from 1 to 2 when the game is linked (derived, *unconfirmed* what it is).
  `manxttc` has the same page.
- *Confirmed (by running it):* with LINK TYPE Master and a link board fitted (`REMODEL_LINK=fake,0,2`) the game
  shows "CHECKING COMMUNICATION BOARD", "THIS IS MASTER CONTROLLER" and runs `srallyc`'s start: 0 to
  `0x1A14000`/`0x1A14002`, a read of `0x1A14000`, the master's 1 to `0x1A14002`, 1 to `0x1A14000`, then it reads
  `0x1A14002` every frame (daytona's wait for bit 7 to flip). See `docs/hw/link-board.md` section 14.
- *Confirmed (by running it):* a plain `LinkPlay` entry (the record above, role byte `0x0B`, roles 1 and 2, `0x1C0` blocks as `srallyc`) links two, three and four machines in `link-run` unchanged: the ring is up at frame 64, the game shows POSITION 1/2 (n/3, n/4) in the race. `docs/hw/link-board.md` section 14.5.
- *Built:* `romset::LinkPlay` for `manxtt` and `manxttc` (`crates/romset/src/games/link.rs`; the record is the 56 bytes
  the game's own test menu writes at the defaults, LINK TYPE 1 for the master and 2 for the slave, the CRC fixed). Up to four machines
  (the menu offers Master and Slave only; the hub ran four), in `link-run`, split-screen (`make play GAME=manxtt PLAYERS=2`) and over
  the network (`link-net`); the block's identity bytes for the players-leaving ghost are `srallyc`'s (*unconfirmed*).
  `make link-net-test GAMES=manxtt` (the clean and 5%-loss runs equal `link-run`'s hashes, the players-leaving cases, settings and
  desync) and `make link-free-play-test GAMES=manxtt` pass. A scripted race (`coin1@1100-1104,start@1300-1304,throttle=0xe0@3500-99999`):
  the course select at frame 2500, the race from about 3,380 with POSITION 1/2 and 2/2 and the other bike (the 1P marker) in view.
- *Confirmed (by running it): FREE PLAY is the game's own COIN ASSIGNMENTS setting #27.* Test menu (`test@1300-1330`; the cursor starts on EXIT):
  COIN ASSIGNMENTS is the sixth row (six service presses), its COIN/CREDIT SETTING starts at #1 (1 coin 1 credit, both chutes), the test
  button steps the number (#26 is "1 coin 6 credits" for chute 1) and **26 presses reach #27, whose chutes both read FREE PLAY**
  (the page then has no MANUAL SETTING row). Leaving the page and the menu (the game saves then) and comparing the 128-byte `eeprom.bin` with a run that
  left the menu without changing anything: byte `0x2E` 0x00 -> 0x1A (the setting number, 0-based), byte `0x04` 0x01 -> 0x02, byte `0x24`
  0xF2 -> 0xFA (derived: not told apart further), then the header CRC. As `srallyc`'s, in `romset::LinkPlay::free_play`; `--free-play`
  starts a race from a start press with no coin.
