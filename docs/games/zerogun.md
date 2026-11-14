# Zero Gunner (`zerogun`, Model 2B-CRX, 1997)

Status: **plays** (listed; the launcher's Model 2B group, with a poster and
bezel), checked in scripted runs: boot, attract, coin-start, the fighter
select and the first mission (Asia) to game over, with sound. Not yet
played by hand. Psikyo's vertical helicopter shooter for two players,
"developed and distributed under licence by SEGA" (title screen, ©1997).
It needed two things from the board and CPU models: the i960 keeping the
interrupt table and stack it was initialised with, and an answer from the
protection device it checks in play.

`zerogun.zip` also holds the clones: `zerogunj` (the Japanese 2B program,
`epr-20290.15/.16`) and the 2A versions `zeroguna` (`epr-20437.12/.13`)
and `zerogunaj` (`epr-20288.12/.13`), with the 2A board's TGP and
rasteriser ROMs (`opr-14742a`-`14747`, `mpr-16310`-`16312`). Only the
parent is supported; the clones aren't listed.

## What's known

- *Confirmed (ROM, then by running it):* the manifest verifies (13
  chips). The board is **Model 2B-CRX**: the program writes both SHARCs'
  host windows (`0x8C0000` and `0x840000`) and the 2B sound UART
  (`0x9C0000`), and boots, uploads and runs both SHARC programs on the 2B
  board model. Program `epr-20439.15/.16` (1 MB, the only program pair);
  main data `.11/.12` at `0x2000000` and `.9/.10` at `0x2800000`;
  polygons `.17/.21`; textures `.27/.25` (2 MB each); sound program `.31`;
  samples `.32` then `.33` (2 MB each, *unconfirmed*: by analogy with the
  other 2B sets, not checked against the sound program's sample table).
  No copro data ROMs. The chips' roles came from the numbering of the
  other 2B sets and their contents; the emulator's .dat was used only as
  a list of file names. The test menu (Psikyo's own: DISPLAY CHECK, I/O
  CHECK, SOUND CHECK, CONFIGURING, BOOKKEEPING, SCROLL CHECK, POLYGON
  CHECK, BG CHECK, STAGE SELECT) has **no memory test**, so the layout is
  confirmed only by the game drawing right: textured 3D, the 2D screens
  and the attract's pictures all as they should look.
- *Confirmed (from the code, then by running it):* the boot copies ROM
  `0x40000-0xFFFFF` to RAM at `0x540000` and runs it there (so the code
  at ROM *x* runs at *x* + `0x500000`). Before that it copies its PRCB to
  `0x500000`, reinitialises the i960 with it (an IAC, ROM `0x63C`), and
  then clears `0x500000-0x53FFFF`, the new PRCB included, and enables
  interrupts. So the i960 **must keep the interrupt table and interrupt
  stack pointers it read at initialisation**: read live from the cleared
  PRCB, the first vblank interrupt jumped to 0 and faulted (frame 86).
  reModel's i960 now reads both at reset and at an IAC reinit and keeps
  them (`i960::Cpu::int_table`, `int_stack`), which changes no other
  game's state hash. The save state layout grew by those two words
  (`STATE_FORMAT` 2).
- *Confirmed (from the code, then by running it):* a **keyed protection
  device** on the expansion bus. At boot the game writes 0 to
  `0x1D9000A` and sends a 192-byte block (from ROM `0x6EA80`, each
  halfword byte-swapped) to the device's RAM at `0x1D80000`. In play (eleven
  places, ROM `0x4442C` the first), it picks one of sixteen requests at
  random: it writes the halfword offset of a 12-byte piece of the block
  (0, 6, ... `0x5A`) to `0x1D90008` and a 16-bit key (table at ROM
  `0x27A44`) to `0x1D9000C`, reads `0x1D9000E` once (discarded), then
  four halfwords, polling bit 0 of `0x1D90000` (busy) clear after each,
  and compares the 8 bytes with ROM `0x279A4` + 8 × the request's index.
  Two requests share a key (`0x234D`, offsets `0x18` and `0x4E`, with
  different answers), so the device is told them apart by the offset too.
  A mismatch adds 1 to a count (`0x513000`); with more than 10, the main
  loop counts on and after about 3,600 more frames calls `0x4B660`
  (*unconfirmed* what that does). What the device computes is
  *unknown*; the board model answers each (key, offset) with the bytes the
  game expects (`romset::ProtectionInterface::Keyed`,
  `docs/hw/model2b.md`). In a coin-start run the count stays 0.
- *Confirmed (by running it):* the copro and geometrizer SHARC programs
  are both its own: the geometrizer's code hash is `8c502dffb7ce9a1e`,
  which no native geometrizer implements, so it always runs on the SHARC
  (`make geo-check` compares nothing). `make run` writes both programs to
  `out/zerogun/sharc-*.bin`; both run with no fault and no host error.
- *Confirmed (by running it):* the attract draws in 3D, textured: the
  Psikyo logo, the title, the staff roll over a city and a hangar, the
  briefing screens (USA, Europe, Asia), helicopter fights over the sea and
  a city, and the score ranking.
- *Confirmed (from the game's own I/O CHECK, by running it):* port 0
  (`0x1C00002`) COIN-1 `0x01`, COIN-2 `0x02`, TEST `0x04`, SERVICE `0x08`,
  START 1P `0x10`, START 2P `0x20`; port 1 (`0x1C00004`) player 1 and
  port 2 (`0x1C00006`) player 2: SW1 `0x01`, SW2 `0x02`, DOWN `0x10`, UP
  `0x20`, RIGHT `0x40`, LEFT `0x80`, all active low; eight DIP switches
  at `0x1C0000C` (all OFF). SW1 shoots; SW2 is the bomb (the game's tips:
  "Use a bomb when it comes to a pinch"). The test menu: test enters, the
  stick moves, SW1 picks; RIGHT+SW1 leaves the I/O check.
- *Confirmed (by running it):* `coin-start` (one coin, start, then shot
  every 8 frames and a bomb every 400) picks the gunship on the fighter
  select (APACHE, GUNSHIP, COBRA) and plays mission 1, Asia: enemies hit,
  power-ups taken, then GAME OVER, CONTINUE? and back to attract.
  *Unconfirmed:* the fighter select's TIME counts down about one a
  hundred frames, slower than a second; whether that is the game's own
  pace isn't known.
- *Confirmed (by running it):* the idle loop is the wait for the next
  frame at `0x549050`-`0x549063` (ROM `0x49050`), a poll of the video
  status's frame bit (`0x98000C` bit 2) counting passes in r4:
  78.0% of a coin-start run's instructions are skipped (`make bench
  GAME=zerogun`: state hash `8dcba7059288171d`, with an `eeprom.bin` from an
  earlier run; the same with `--no-idle-skip` and after a save and load,
  `make state-check GAME=zerogun`). Interpreted it runs at 1.64x real
  time (always on the SHARC geometrizer).
- *Confirmed (by running it):* **recompiled** (`make recomp
  GAME=zerogun`): 3,371 of the copro's 4,096 uploaded words and 3,224 of
  the geometrizer's, but only 3,213 i960 instructions (898 blocks): as on
  `srallyc`, the recompiler reaches only the program ROM, and the game
  runs its code from the copy in RAM, which stays on the interpreter
  (`docs/recompiler.md`). `make lockstep GAME=zerogun RECOMP=1` runs 400M
  instructions of a coin-start run identical (both SHARCs recompiled),
  and the bench run gives the same state hash at 5.07x real time, the
  SHARCs being most of the time. On the i960 JIT (`make bench
  GAME=zerogun JIT=1`) the code in RAM is translated too (2,545 of its
  2,557 blocks) and the hash is the same, at 1.74x real time with the
  SHARCs interpreted.
- *Confirmed (by running it):* **input lag** (`make input-lag
  GAME=zerogun`, left held from frame 2107 of a coin-start run): the
  picture changes in the first frame that can read it, so `input_lag` and
  `run_ahead_max` are 0 (also from 1807, 2407 and 2507; from 2207 the
  picture doesn't change within 16 frames, *unconfirmed* why: the
  helicopter may not be under control then).
- *Confirmed (by running it):* **sound**: the 68000 and SCSP board runs
  as on the other 2B games; a coin-up and start run with shots only,
  recorded with `--audio` (72 s), is silent for the first 6 s, then plays: about -37 dBFS RMS in the
  attract, -28 in play, -29.7 over the run (peak -12.2), so `volume_db`
  is 0. *Unconfirmed:* by ear.

## Next

1. Play it by hand, and player 2.
2. A native geometrizer for its own SHARC program, for speed.
3. The sound by ear, and the sample order against the sound program's
   sample table.
4. What the protection device computes, and what `0x4B660` does.
5. The 2A and Japanese clones.
