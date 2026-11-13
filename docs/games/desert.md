# Desert Tank (`desert`, the original Model 2, 1994)

Status: **in progress** (`romset::games::IN_PROGRESS`, not in the
launcher). The third game on the original Model 2 board
(`Board::Model2`, [`docs/hw/model2.md`](../hw/model2.md)), after
[`daytona`](daytona.md) and [`vcop`](vcop.md). The title screen reads
"©1994 SEGA / MARTIN MARIETTA".

## What's known

- *Confirmed:* the manifest verifies (28 chips of `desert.zip`). The
  emulator's .dat was used only as a catalogue of the set's file names;
  the chips' roles come from the data and the run.
- *Confirmed (by running it):* the program is **512 KB** as `vcop`'s,
  `.12/.13` then `.14/.15` (the second pair holds 34 KB of code, run from
  `0x40000`); the boot record at 0 (PRCB `0xC0`, first IP `0x860`,
  interrupt table `0x3640`).
- *Confirmed (by running it):* main data `.10/.11`, `.8/.9`, `.6/.7`
  packed from `0x2000000` as `daytona`'s and `vcop`'s; the TGP program
  (2,024 words) is uploaded and runs with no unknown word or external
  access.
- *Confirmed (by running it):* boot shows the settings ("GAME SYSTEM":
  one coin a credit, COUNTRY JAPAN, ADVERTISE SOUND ON, DIFFICULTY DESERT
  EASY, CANYON EASY), then the title and attract.
- *Confirmed (from the data, then the pictures):* **the polygon ROMs pair
  `.16` with `.21` and `.17` with `.20`**, not as `daytona`'s (`.16/.20`,
  `.17/.21`). Joining two chips' 16-bit halves into 32-bit words, the share
  of words that read as floats of a sane size (exponent within 2^+-20) is
  0.89 for `.16/.21` and `.17/.20` against 0.74 and 0.30 for `daytona`'s
  pairing and about 0.1 for a pair the wrong way round (the same measure
  gives 0.90 for `daytona`'s own `.16/.20` and `.17/.21`, 0.89 for `vcop`'s
  `.16/.20`). With `daytona`'s pairing the attract and play drew the
  ground and sky but every model as slivers and streaks; paired so, the
  tank, the dunes, the pylons and buildings draw. The chips are as the
  catalogue names them; which pair holds the low 4 MB is *unconfirmed*
  beyond the pictures being right with `.16/.21` first.
- *Confirmed (from the data):* the TGP's data ROM is the two 512 KB
  EPROMs `epr-16981.28` (low half) and `epr-16980.29`: so paired it begins
  with a 4x4 identity matrix of floats, as `daytona`'s `.28/.29`.
- *Confirmed (from the game's own input test, by running it):* TEST MODE
  (BOOKKEEPING, GAME SYSTEM, INPUT TEST, OUTPUT TEST, SOUND TEST, TGP
  TEST, MEMORY TEST, BACKUP RAM CLEAR, EXIT: RED up, BLUE down, START
  selects; the service switch also moves down). INPUT TEST lists, and
  the scripts lit:
  - register 8 (port 0): CHUTE 1 `0x01`, CHUTE 2 `0x02`, TEST `0x04`,
    SERVICE `0x08`, START `0x10`, VR1 (BLUE) `0x20`, VR2 (GREEN) `0x40`,
    VR3 (RED) `0x80`;
  - register 9 (port 1): SHIFT `0x01`, MACHINE GUN `0x10`, CANNON `0x20`;
  - analog channel 0 HANDLE, 1 ACCEL, 2 TURRET (each shown raw, `80` at
    the model's rest).
  *Confirmed (by running it):* SELECT A MISSION takes the handle ("choose
  by steering": above `0x80` moves to EXPERT, the right) and the pedal
  selects ("SELECT!"); the start button, the triggers and the lever don't.
  **SHIFT is the drive lever**: with it clear the tank starts with
  "REVERSE" shown and makes no headway with the pedal down (the scene
  stays put); set, it drives forward along the track. The manifest gives it as a two-position latched lever
  (`Shifter`: Forward, and Reverse as neutral). In play the handle above
  `0x80` turns the tank right. *Unconfirmed:* the handle's and turret's
  travel, which way the turret turns, and the pedal's rest value (taken as
  0, as `daytona`'s).
- *Confirmed (by running it):* `coin-start` (one coin, start, the pedal on
  SELECT A MISSION, then the lever forward, the pedal down, the machine
  gun every 10 frames and the cannon every 60) plays the desert mission:
  the tank drives along the track, shoots, scores, reaches the first
  checkpoint ("TIME EXTENSION", 69,000 points by frame 4,500) and takes
  damage until its armour is low ("危険").
- *Confirmed (by running it):* the idle loop is the wait for the next
  frame at `0x2FC1C`-`0x2FC23` (a byte the vblank handler changes, polled
  until it differs). The game is busy: about a quarter of a run's
  instructions are skipped.
- *Confirmed (by running it):* **sound** on the original board's sound
  board (`sound::PcmBoard`, `epr-16985.7`, the same driver and addresses):
  the first MultiPCM, one 2 MB chip (`.32`), plays with bank 1 (its
  banked samples' loops fit `.32`'s second MB: a mean jump of 4.2 against
  12.5 in its first), the second (`.4`, `.5`) with banks 1 and 2. In a
  coin-start run the music starts at about 10 s and the effects follow
  the shooting. *Open:* at boot the host sends `F8 F8 FF` and then nine
  more bytes at once; `FF` restarts the sound program, which keeps its
  interrupts masked for about 93 ms while it clears its RAM, and eight of
  the nine bytes are overrun (`vcop`'s link has no overrun). Lost
  on the hardware too, or the UART or the restart is quicker than the
  model's: *unconfirmed*. Not checked by ear.
- *Confirmed (by running it):* **input lag** (`make input-lag GAME=desert`:
  the handle at `0xD0` from frames 2505 and 3505 of a coin-start run): the
  picture changes in the first frame that can read it, so `input_lag` and
  `run_ahead_max` are 0.

## Next

1. The whole desert mission to its end, and the canyon (EXPERT); a
   steering script.
2. The turret's direction and travel, the handle's travel.
3. The boot bytes the sound board overruns (above).
4. The OUTPUT TEST (lamps) and the motion cabinet (not needed to play).
