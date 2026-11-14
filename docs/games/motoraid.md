# Motor Raid (`motoraid`, Model 2A-CRX, 1997)

Status: **in progress** (in `IN_PROGRESS`: the CLI runs it, the launcher
and the browser don't list it yet). It boots to its attract mode in 3D,
textured, and a coin-start Championship rides stage 1 to the finish
(9th) and into stage 2, to the end of the clock, the ranking and back to
the attract mode, with sound. Not yet played by hand.

## What's known

- *Confirmed:* the manifest verifies (31 chips of `motoraid.zip`). The
  emulator's .dat was used only as a catalogue of the set's file names;
  the chips' roles come from the data and the run.
- *Confirmed (by running it):* the program is `.12/.13`, 1 MB. The TGP
  program (3,147 words) is uploaded and runs on the MB86234 interpreter;
  it uses **ALU code 3**, which no other program here does, 7,142 times in
  the first 1,346 frames (`docs/hw/mb86234.md`: taken as XOR, a
  hypothesis). With it, no unknown word or external access over a
  16,566-frame run.
- *Confirmed (from the program, the data and the pictures):* **the main
  data is in two banks.** `.10/.11` and `.8/.9` (8 MB each) are at
  `0x2000000`; `.6/.7` and then `.4/.5` (1 MB, repeating in its 8 MB slot)
  are in the second bank at `0x6000000` (`Role::MainDataHigh`, as
  `segawski` and `waverunr` on 2C). The evidence: the program holds
  pointers into `0x2000000-0x2FFFFFF` and `0x6000000-0x6CFFFFF` (fewer
  than 20 to any MB of `0x3000000-0x3FFFFFF`); `.6/.7` begins with floats
  (model data) and holds about 2,600 pointers into
  `0x6000000-0x66FFFFF`; the game copies textures into video RAM from
  `0x2000000`, `0x2100000`, `0x2300000`, `0x2700000` and `0x6700000`
  (routine `0x2F990`). `.6/.7` is model data (0.05-0.08 of its bytes have
  equal nibbles, a mark of 4-bit texels) except its last MB (0.56), which
  is `0x6700000` only with `.6/.7` at `0x6000000`; the other sources, in
  `.10/.11`, score 0.39-0.89. With the
  pairs packed after `.8/.9` (`srallyc`'s layout) the game's reads there
  were unmapped (278,859 addresses in a 10,354-frame attract), and the
  canyon walls, the tunnels, the tutorial's rider and the sky drew flat or
  not at all; in the second bank all of them draw textured, and nothing
  reads unmapped memory.
- *Confirmed (from the data):* the polygon ROMs pair `.16/.20`, `.17/.21`,
  `.18/.22` (sane floats 0.89, 0.90, 0.88 against at most 0.80 for the
  others). The TGP's data ROM is `.28` (low half) with `.29`, as
  `manxtt`'s: so its first words are increasing offsets (`0xD260`,
  `0x16850`, `0x1F130`, ...), the other way round they would be
  `0xD2600000`-like. Textures `.25` (even words) / `.24`.
- *Confirmed (from the game's own input test):* TEST MENU (`test` held;
  SERVICE moves, TEST chooses: MEMORY TEST, T.G.P. TEST, INPUT TEST,
  OUTPUT TEST, SOUND TEST, C.R.T. TEST, GAME ASSIGNMENTS, COIN
  ASSIGNMENTS, VOLUME SETTING, BOOKKEEPING, BACKUP DATA CLEAR, EXIT).
  INPUT TEST lists BANK, ACCELE, BRAKE, START/VIEW, PUNCH, KICK, COIN
  CHUTE, SERVICE, TEST; holding one port bit at a time lit port 0 `0x01`
  COIN CHUTE, `0x08` SERVICE, `0x40` START/VIEW, port 1 `0x10` PUNCH,
  `0x20` KICK, and nothing else of ports 0-2 (TEST, port 0 `0x04`, leaves
  the screen). ACCELE, BRAKE and BANK are analog channels 0, 1 and 2, as
  `manxtt`'s.
- *Confirmed (by riding):* BANK below `0x80` leans the bike and turns it
  right, above it left (as `manxtt`); the accelerator at `0xE0` reaches
  360 km/h on stage 1. The keys move the bank through a `KeyRamp` (12
  frames over, 8 back), as for `manxtt`.
- *Confirmed (by running it):* two coins a credit ("CREDIT 0/2");
  `coin-start` pays two and presses START: SELECT PLAYER (Robin), SELECT
  GAME (CHAMPIONSHIP, PRACTICE: the timers run out to the first), then
  CHAMPIONSHIP stage 1 (YENDAS). With the accelerator held from frame
  2,200 the bike rides stage 1 through a checkpoint ("CHECK") to
  "FINISH 9th" (01'13"75, 60 points), then stage 2 until the time runs
  out, the championship ranking and the attract mode (16,566 frames; no
  hang).
- *Confirmed (by running it; not yet by ear):* the sound board runs the
  game's driver: 7,599 host bytes delivered, 8,479 key-ons over all 32
  slots, about -31 dBFS RMS; the player's live mix over a 288 s run
  measures -21.0 LUFS, peak -5.5 dBFS, inside the other games' range, so
  the manifest adds no gain. The samples `.31 .32 .36 .37` back to back
  are **by analogy** with `srallyc`: this sound program (CRX_drv 3.2.8)
  has no table of `srallyc`'s form, so the order is *unconfirmed*.
- *Unknown:* the game writes 64 KB at `0x12810000-0x1281FFFF` (23,040
  writes at boot), just past the luma tables at `0x12800000`, which the
  board model doesn't map (`docs/hw/model2a.md` §2). A luma table
  number is 8 bits, so the rasteriser as modelled reads only the first
  64 KB; what the second block is for is not known. `manxtt` writes a
  few words there too.

## Idle loop

*Confirmed (from the code, by running it):* the frame wait at
`0x30204-0x30217` polls the video status's frame bit (`0x98000C` bit 2)
until vblank toggles it, counting passes in g0, as `srallyc`'s and
`manxtt`'s loops do. Over a 5,522-frame coin-start race it skips 78.8% of
the instructions (7.5x real time against 2.6x), the state hash the same as
with `--no-idle-skip`.

## Input lag

`make input-lag GAME=motoraid`: **0 frames** for the bank (`0x30` from
frame 2,900 of a race) and the brake; the punch shows 1 frame. As for
`schamp` (punches 1, the stick 0), `input_lag` is 0 and `run_ahead_max`
1. Save states round-trip (`make state-check`).

## Open

- Play by hand; a whole Championship.
- The sample ROM order, by the sound program and by ear.
- The `0x12810000` block.
- The ride-on cabinet's moving parts: no drive board is checked at boot
  and none is modelled; the I/O chip's register 4 is written
  (`0x1C00008`), what it drives is *unknown*.
