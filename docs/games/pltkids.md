# Pilot Kids (`pltkids`, Model 2B-CRX, 1998)

Status: **plays** (listed; the launcher's Model 2B group, with a poster and
bezel), checked in scripted runs: boot, the attract mode in 3D, textured
(the Psikyo logo, the story scenes in the toy room, the demo play, the
title and the score ranking), coin-start into stage 1 (Dad's Room) to its
boss, game over, the continue countdown, the ranking and back to the
attract mode, with sound. Not yet played by hand. Psikyo's horizontal
shooter for two players, "developed and distributed under licence by
SEGA" (title screen, ©1998). On the board model it needed nothing new
after `zerogun`: the i960 keeping the interrupt table it was initialised
with, and an answer from the keyed protection device, checked once at
boot.

`pltkids.zip` also holds its clone `pltkidsa`, the Model 2A version:
its own program pair `epr-21281.pr0/.pr1`, with the 2A board's TGP and
rasteriser ROMs (`opr-14742a`-`14747`, `mpr-16310`-`16312`). Only the
parent is supported; the clone isn't listed.

## What's known

- *Confirmed (ROM, then by running it):* the manifest verifies (21
  chips). The board is **Model 2B-CRX**: the program writes both SHARCs'
  host windows (`0x8C0000` and `0x840000`) and the 2B sound UART
  (`0x9C0000`), and boots, uploads and runs both SHARC programs on the 2B
  board model. The chips have Psikyo's names: program `epr-21285a.15/.16`
  (1 MB, the only 2B program pair); main data `.da0/.da1` at `0x2000000`;
  polygons `.tp0/.tp1`, `.tp2/.tp3`, `.tp4/.tp5`, `.tp6/.tp7` (even chip
  the low half: those pairings make by far the most sane floats, 0.88-0.89
  against 0.18-0.85 for any other, and `.tp6/.tp7` 0.55, against at most
  0.53); textures `.tx1/.tx0` and `.tx3/.tx2` (**the odd chip the low
  half**, as `dynamcop`'s higher-numbered chip is: with `.tx0` low the 3D
  draws in flat colours, with `.tx1` low it is fully textured); sound
  program `.sd0`; samples `.sd1` to `.sd4` (2 MB each). No copro data
  ROMs. The emulator's .dat was used only as a catalogue of which file
  names and CRCs belong to the parent and which to the clone.
- *Confirmed (from the game's own test menu):* Psikyo's test menu
  (Display Check, Input Check, Sound Check, Game Setting, Exit) has **no
  memory test**, so the layout is confirmed only by the game drawing and
  playing right: textured 3D, the 2D screens and the attract all as they
  should look.
- *Unconfirmed:* the sample order. It is the chips' numbering, and the
  data agrees: read as 8-bit samples (the bytes of each 16-bit word
  swapped, as the sound board reads them), the end of `.sd1` runs on
  smoothly into the start of `.sd2` (from -1, 1 to 3, 3, 3), `.sd2` into
  `.sd3` and `.sd3` into `.sd4`, and `.sd1` starts with silence. The
  sound program's sample table wasn't found to check it against.
- *Confirmed (from the code, then by running it):* the boot copies ROM
  `0x50000-0xFFFFF` to RAM at `0x550000` and runs it there (so the code
  at ROM *x* runs at *x* + `0x500000`), as `zerogun` does from `0x40000`.
  Before that it copies its PRCB to `0x500000`, reinitialises the i960
  with it (an IAC, ROM `0x62C`, the message at `0x570`) and then clears
  `0x500000-0x54FFFF`, the new PRCB included (ROM `0x708`). Before the
  i960 kept the interrupt table it was initialised with (`zerogun`'s
  change, `i960::Cpu::int_table`), the first vblank interrupt read a
  table address of 0 from the cleared PRCB and jumped to 0 (frame 120).
- *Confirmed (from the code, then by running it):* the **keyed protection
  device** `dynamcop` and `zerogun` use, checked **once, at boot**. The
  routines are in the program ROM (`0x15660` writes 0 to `0x1D9000A`,
  `0x156A0` sends a block, `0x15750` makes a request), called from the
  code in RAM at `0x5518A8`-`0x5518E8`: a 12-byte block from RAM
  `0x5822F0` (each halfword byte-swapped) to `0x1D80000`, then offset 0
  to `0x1D90008` and key `0x7140` to `0x1D9000C`, one discarded read of
  `0x1D9000E`, then four halfwords, polling bit 0 of `0x1D90000` (busy)
  clear after each. The 8 bytes are compared with RAM `0x5518C8` (ROM
  `0x518C8`), which reads `98-PILOT`. A mismatch sets `0x5092D4`, which
  four places in the game test (ROM `0x158B0`, `0x6DA44`, `0x6E4F4`,
  `0x6E8C8`), the middle two drawing a random number with it
  (*unconfirmed* what each spoils). Nothing else calls the request
  routine. What the device computes is *unknown*; the board model answers
  the key with the bytes the game expects
  (`romset::ProtectionInterface::Keyed`). In coin-start runs `0x5092D4`
  is written only with 0.
- *Confirmed (from the game's own Input Check, by running it):* port 0
  (`0x1C00002`) COIN 1 `0x01`, COIN 2 `0x02`, TEST `0x04`, SERVICE
  `0x08`, START 1 `0x10`, START 2 `0x20`; port 1 (`0x1C00004`) player 1
  and port 2 (`0x1C00006`) player 2: SW-1 `0x01`, SW-2 `0x02`, DOWN
  `0x10`, UP `0x20`, RIGHT `0x40`, LEFT `0x80`, each lit on the screen
  by its bit. Ports 1/2 `0x04` and `0x08` light nothing. SW-1 shoots;
  SW-2 drops a bomb (one of the stars under the score goes). The test
  menu: test enters, the stick moves, SW-1 picks; RIGHT+SW-1 leaves the
  Input Check.
- *Confirmed (by running it):* `coin-start` (one coin, start, then a shot
  every 4 frames and a bomb every 500) starts stage 1 at once (no
  character select for one player): over the desk,
  the bookshelves and cupboards, the brick wall with the toy train, the
  boss helicopter, then GAME OVER, CONTINUE?, the score ranking and the
  attract mode. Standing still, the boss wins.
- *Confirmed (by running it):* the attract draws in 3D, textured, and its
  demo plays.
- *Confirmed (by running it):* the copro and geometrizer SHARC programs
  are its own copro program and `zerogun`'s geometrizer code (code hash
  `8c502dffb7ce9a1e`), which no native geometrizer implements, so it
  always runs on the SHARC (`make geo-check` compares nothing). `make
  run` writes both programs to `out/pltkids/sharc-*.bin`; both run with
  no fault and no host error.
- *Confirmed (by running it):* the idle loop is the wait for the next
  frame at `0x55CF3C`-`0x55CF4F` (ROM `0x5CF3C`), a poll of the video
  status's frame bit (`0x98000C` bit 2) counting passes in r4: 82.1% of a
  coin-start run's instructions are skipped (`make bench GAME=pltkids`:
  state hash `9372c575ce6c52d6`, with an `eeprom.bin` from an earlier
  run; the same with `--no-idle-skip` and after a save and load, `make
  state-check GAME=pltkids`, whose audio matches too). Interpreted it
  runs at about 1.1x real time, the SHARC geometrizer being most of it.
- *Confirmed (by running it):* **recompiled** (`make recomp
  GAME=pltkids`): 2,013 of the copro's 4,096 uploaded words and 3,224 of
  the geometrizer's, but only 6,287 i960 instructions (1,845 blocks): as
  on `zerogun`, the recompiler reaches only the program ROM, and the game
  runs most of its code from the copy in RAM, which stays on the
  interpreter (`docs/recompiler.md`). `make lockstep GAME=pltkids
  RECOMP=1` runs 400M instructions of a coin-start run identical (both
  SHARCs recompiled), and the bench run gives the same state hash at
  4.17x real time.
- *Confirmed (by running it):* **input lag** (`make input-lag
  GAME=pltkids`, up held from frame 2107 of a coin-start run; also from
  1807 and 2407): the picture changes in the first frame that can read
  it, so `input_lag` and `run_ahead_max` are 0. (The shot can't be
  measured that way: the coin-start script already fires every 4 frames.)
- *Confirmed (by running it):* **sound**: the 68000 and SCSP board runs
  as on the other 2B games; a coin-start run recorded with `--audio` (72
  s) is silent for the first 15 s (the Psikyo logo and the first story
  scenes), then plays: -47 to -29 dBFS RMS in 5 s windows, -31.7 over
  the run (peak -13.4), so `volume_db` is 0. The attract alone sends few
  sound commands (18 bytes in its first 70 s); play sends many.
  *Unconfirmed:* by ear.

## Next

1. Play it by hand, and player 2.
2. The sound by ear, and the sample order against the sound program.
3. Later stages: the coin-start script stands still and loses to stage 1's
   boss.
4. What the protection device computes, and what `0x5092D4` spoils.
5. (Built as a variant: `pltkidsa`, `docs/variants.md`. Its stall on the survey's manifest was the MB86234's register-count repeat; it plays stage 1 to game over. The brown banding in some frames is not checked against the 2A board.)
