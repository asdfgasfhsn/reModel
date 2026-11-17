# Rail Chase 2 (`rchase2`, Model 2B-CRX, 1995)

Status: **plays** (listed; the launcher's Model 2B group, with a poster and
bezel), checked in scripted runs: boot, the "checking network" screen
(main board and drive board), the attract mode in 3D, textured, the mode
select, and coin-start into the first stage (the Ekkel empire's secret
weapons factory, then the mines) with the lock-on reticle, damage and
explosions, with sound. Not yet played by hand. Sega's on-rails mine-cart
shooter for one or two riders on a motion cabinet. On the board model it
needed one new device: a minimal model of its ride drive board, answering
the wire test the game runs at boot.

## What's known

- *Confirmed (ROM, then the memory test):* the manifest verifies (all 21
  chips of `rchase2.zip`). The board is **Model 2B-CRX**: the program
  writes both SHARCs' host windows (`0x8C0000`, `0x840000`) and the 2B
  sound UART (`0x9C0000`). Program `epr-18045a.15/.16` then
  `epr-18074a.13/.14` (1 MB each, 2 MB in all); main data (2 MB chips)
  `.11/.12`, `.9/.10`, `.7/.8`, `.5/.6` **packed** back to back from
  `0x2000000`, so the pairs sit at `0x2000000`, `0x2400000`, `0x2800000`,
  `0x2C00000` (the program's pointers, e.g. `0x2600000` and `0x2664540`,
  land inside the second pair and need that; in 8 MB slots they would read
  repeats of the first); polygons `.17/.21` then `.18/.22`, packed;
  textures `.27/.25`; sound program `epr-18047.31`; samples `.32` then
  `.34`. The test menu's MEMORY TEST shows IC 5-16 and IC 79-89 all GOOD in
  this layout (it doesn't tell layouts apart much: gunblade passes it with
  empty sockets). The chips' roles came from the numbering of the other 2B
  sets and their contents; the emulator's .dat was used only as a list of
  file names. *Unconfirmed:* the samples' order against the sound
  program's table (by analogy with `gunblade`).
- *Confirmed (by running it):* it has **no copro data ROMs** and no
  protection device. Both SHARC programs are its own uploads: the copro
  program (3,112 words) and a geometrizer program (3,040 words, code hash
  `9b6fb0e1100bf66e`) the native geometrizer runs (it is the Virtua
  Striker variant): `make geo-check GAME=rchase2` compares 5,164 frames of
  a coin-start run, 0 differ (native 1.2 ms a frame against 14.9 ms
  interpreted).
- *Confirmed (by running it):* boot shows a warning ("used only in
  Japan"), the Sega logo, then a **"CHECKING NET_WORK NOW"** screen: MAIN
  BD and DRIVE BD boxes with arrows between them, ending NET_GOOD or
  NET_BAD. With nothing answering the drive board it ends NET_BAD and the
  game stays on that screen for good.
- *Confirmed (from the game's code, by running it):* **the drive board**
  ("by network"). The vblank I/O routine (i960 `0x1B68`) writes a command
  byte from RAM `0x501D8C` to `0x1C00008` and keeps the low nibble of
  `0x1C00006`, inverted, in RAM `0x501D8D`. The check (i960 `0x16610`)
  sends eight commands in turn, three frames apart (`0x07 0x0B 0x0D 0x0E
  0x70 0xB0 0xD0 0xE0`: one nibble with a single clear bit, the other
  nibble 0) and compares each reply with `08 04 02 01 08 04 02 01`, the
  low nibble of the reply read back being the lit nibble of the command;
  it repeats this twelve times and any miss makes NET_BAD. The board model
  (`DriveBoard::RideLink`, `model2::drive::RideLink`) answers
  `(cmd | cmd >> 4) & 0x0F`; with it the check passes (NET_GOOD, about
  frame 700) and the game goes on with `0x3B`, then a command byte each
  frame (`0x2F`, `0xAF`, `0xB8`, `0xC0`, `0xC8`, `0xF8` seen in the attract
  and in play, so presumably the ride's motion state). The output test
  shows reply bits 0 and 3 as ON/OFF (i960 `0x10404`, `0x10484`).
  *Unknown:* what the real board replies to anything but the wire test
  and what the commands mean; nothing but this check was seen to read the
  reply.
- *Confirmed (by running it):* the game probes for the original Model 2 I/O
  board's buffer at boot (writes `0x4D` to `0x1C00202`, reads it back; i960
  `0x6F8`) and, if it isn't there, for a four-word ID at `0x980030`
  (`HAT`, `OKA`, `KAZ`, `KTM`, i960 `0x6A0`); neither is present here and it
  uses the 2B I/O chip path (type 2 in RAM `0x501D80`); the game runs
  the same either way.
- *Confirmed (from the game's own input test, by running it):* port 0
  (`0x1C00002`) COIN CHUTE #1 `0x01`, #2 `0x02`, TEST `0x04`, SERVICE
  `0x08`, START 1P `0x10`, START 2P `0x20`; port 1 (`0x1C00004`) SHOT 1P
  `0x01`, SHOT 2P `0x02`; bits `0x40` and `0x80` of port 0 light nothing.
  **Not a mounted gun**: each rider has a **stick** on the ADC: 1P X is
  channel 1 and Y channel 3, 2P X channel 0 and Y channel 2 (the input
  test shows each raw value, 80H at rest; it lists LEFT/RIGHT/UP/DOWN MAX as
  C0H/40H/40H/C0H). In play the lock-on reticle follows the stick: X
  high is **left**, X low right, Y high **up** (the test screen's UP MAX
  label notwithstanding), reaching the screen corners at 40H and C0H. The
  test menu (MEMORY TEST, INPUT TEST, SOUND TEST, C.R.T. TEST, GAME
  ASSIGNMENTS, COIN ASSIGNMENTS, BOOKKEEPING, BACKUP DATA CLEAR, OUTPUT
  TEST): test enters, service moves the cursor (it starts below the
  last item), test selects. A one-player ride costs 2 credits and a
  two-player ride 4 at one credit a coin, so the coin-start script puts
  in two coins. (Holding ports 0's bits 0, 1, 4, 5, 6 and 7 all at once in
  the input test left the screen frozen on the last values in one run; each
  alone is fine. *Unconfirmed* why; not looked into.)
- *Confirmed (by running it):* `coin-start` (two coins, start, then a shot
  every 20 frames, stick centred) goes through the mode select into the
  first stage: the factory yard, the mine tunnels and the collapsing
  girders, taking damage as it stands still, with FLINT's health bar; a
  second rider can join ("COMRADE WANTED" in Japanese). The attract draws
  in 3D, textured, demos and the title over the cliff-side track.
- *Confirmed (by running it):* the idle loop is the wait for the next frame
  at `0xD798`-`0xD7AF`, a poll of the video status's frame bit
  (`0x98000C` bit 2) counting passes in g0: 80.8% of a coin-start run's
  instructions are skipped (`make bench GAME=rchase2`: state hash
  `d7abe9d43f16ed78`, with an `eeprom.bin` from an earlier run; the same
  with `--no-idle-skip`, on the JIT, and after a save and load, `make
  state-check GAME=rchase2`, whose audio matches too).
- *Confirmed (by running it):* **input lag** (`make input-lag
  GAME=rchase2`, the stick held from frame 2400 of a coin-start run): the
  picture changes in the first frame that can read it, so `input_lag` and
  `run_ahead_max` are 0.
- *Confirmed (by running it):* **sound**: the 68000 and SCSP board run as
  on the other 2B games; `make sound` averages -28.5 dBFS RMS (peak -6,
  none clipped), so `volume_db` is -4 (as `schamp`). *Unconfirmed:* by ear.
- *Confirmed (by running it):* **recompiled** (`make recomp GAME=rchase2`;
  both SHARC programs too): `make lockstep GAME=rchase2 RECOMP=1` runs
  400M instructions of a coin-start run identical, and the recompiled
  bench gives the same state hash at 4.6x real time. `make jit-lockstep`
  is not needed: the JIT bench gives the same hash too.

## Next

1. Play it by hand, and player 2.
2. The sound by ear, and the sample order against the sound program.
3. Later stages: the coin-start script stands still and takes damage.
4. What the real drive board answers, and what the command bytes mean
   (the motion).
5. Mouse aiming (the stick drives a reticle; a `Gun`-style pointer would
   need the reticle's mapping measured, as `gunblade`'s).
