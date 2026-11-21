# Dynamite Cop (`dynamcop`, Model 2A-CRX, 1998)

Status: **plays** (listed; the launcher's Model 2A group, with a poster and
bezel), checked in scripted runs: boot, the attract mode in 3D (the story
scenes on the hijacked liner and the demo fights), coin-start through
PLAYER SELECT and MISSION SELECT into mission 1's fights on the ship's
deck, the continue screen, game over and back to the attract mode, with
sound. Not yet played by hand. The export release of Dynamite Deka 2; the
first game here to check the **keyed protection device** (below).

## What's known

- *Confirmed:* the manifest verifies (35 chips of `dynamcop.zip`). The
  emulator's .dat was used only as a catalogue of which file names and
  CRCs belong to the parent and which to its clones; the chips' roles come
  from the data and the run.
- *Confirmed (from the ROM):* the board is **Model 2A**: the set has the
  TGP's maths tables (`opr-14742a.45` ... `opr-14747.63`) that only
  TGP boards carry, the program uploads an MB86234 program (3,712 words)
  to the TGP's ports, and the program pair is `.12/.13` then `.14/.15` as
  on `vcop2` (PRCB `0xB0`, first IP `0x5F0`, interrupt table `0x8F4C0`).
  The ramps `mpr-16310.15`, `-16311.16`, `-16312.14` are in the zip and
  unused, as on every 2A set. The TGP program runs with no unknown word
  and no unknown external access.
- *Confirmed (from the program and the pictures):* **main data in two
  banks**, as `motoraid`: `.10/.11` and `.8/.9` (8 MB each) at
  `0x2000000`, `.6/.7` and `.4/.5` in the second bank at `0x6000000`
  (`Role::MainDataHigh`). The program holds about 4,100 words that point into
  `0x2000000-0x2FFFFFF` and 3,100 into `0x6000000-0x6FFFFFF`, against
  10-170 to each MB of `0x3000000-0x5FFFFFF` (floats and code words that
  happen to land there). With this layout everything draws textured.
- *Confirmed (by the pictures):* polygons `.16/.20 .17/.21 .18/.22
  .19/.23` (low half first, packed), textures `.25/.24` and `.27/.26` (the
  second pair in its own 8 MB slot, as `vf2`'s); no TGP data ROM.
  Each polygon pairing is the one whose 32-bit words read as sane floats
  most often (0.81-0.89, against 0.60-0.79 for the other pairings).
  Without `.27/.26` the attract and mission 1 draw differently from frame
  900 on; without `.19/.23` nothing changes in a 6,000-frame coin-start
  run, so that pair is used by later missions or not at all
  (*unconfirmed*).
- *Confirmed (from the code, `0x66A00`-`0x66BA8` and `0xB5EF0`-`0xB6080`,
  and by running it):* **a protection device at `0x1D80000`/`0x1D90000`**,
  checked twice at boot. Each check writes a 20-byte block from the
  program ROM (`0x1FFFC0`, then `0x1FFF80`) into the device's RAM at
  `0x1D80000` (halfwords, byte-swapped), its halfword offset (0) to
  `0x1D90008`, and a key to `0x1D9000C` (`0x1326`, then `0x7700`); it
  reads `0x1D9000E` once and discards it, then reads 8 halfwords from
  `0x1D9000E`, polling bit 0 of `0x1D90000` (busy) after each. The init
  writes `0x1D9000A` once (a reset, *unconfirmed*). The 16 bytes of each
  answer are compared with ROM `0xBC7F0` (binary) and `0xBC800` (an
  ASCII name). On a mismatch the game marks an error at `0x51F4B4`, fills
  64 KB of work RAM with `0xFF` and dies (an operation fault at IP 0 by
  frame 271). How the device turns a block and key into its answer is
  *not known*; the board model answers each key with the bytes the game
  compares it with (`romset::ProtectionInterface::Keyed`,
  `model2::protection`), as `doa`'s device is answered. Nothing else in
  the program touches the device. This is a different register layout
  from `doa`'s (`docs/hw/model2b.md`); whether it is the same chip is
  *unknown*.
- *Confirmed (from the game's own input test, by running it):* port 0
  (`0x1C00002`) COIN 1 `0x01`, COIN 2 `0x02`, TEST `0x04`, SERVICE `0x08`,
  1P START `0x10`, 2P START `0x20`; port 1 1P PUNCH `0x01`, KICK `0x02`,
  JUMP `0x04`, DOWN `0x10`, UP `0x20`, RIGHT `0x40`, LEFT `0x80`; port 2
  the same for player 2. Port 0 `0x40`/`0x80` and ports 1/2 `0x08` show
  nothing. The test menu (MEMORY TEST, T.G.P. TEST, INPUT TEST, SOUND
  TEST, C.R.T. TEST, GAME ASSIGNMENTS, COIN ASSIGNMENTS, BOOKKEEPING,
  BACKUP DATA CLEAR, EXIT): service moves the cursor, test selects. One
  coin a credit by default.
- *Confirmed (by running it):* `coin-start` (a coin, start, then punch
  every 20 frames and kick every 50) picks the first fighter (Bruno
  Delinger) on PLAYER SELECT, starts mission 1 ("parachute onto the ship
  from the air") and fights on the deck (score, POWER UP, enemies falling)
  until the lives run out, CONTINUE?, game over and the attract mode.
- *Confirmed (from the sound program's own table and the chips'
  contents):* **the samples are `.31 .32 .36 .37`, back to back** from the
  sound 68000's `0x800000`. The sound program `epr-20811.30` has a sample
  table at 68000 `0x60ECC0` of **12-byte** entries (start, length with
  flags in the top byte, loop start), 312 samples packed end to end
  across the 2 MB slots (three cross a slot boundary). Samples begin and
  end near silence: the mean absolute byte at the starts and ends that
  fall in each slot is 1.4-7.6 in this order's chip and 18-31 in any
  other chip, for all four slots.
- *Confirmed (by running it):* **sound**: a 180 s coin-start run renders
  with `make sound` (3,449 key-ons over all 32 slots, peak -14.4 dBFS,
  RMS -35 dBFS, no 5 s block below -47 dBFS, no clipping). *Unconfirmed:*
  by ear, and the level (`volume_db` 0).
- *Confirmed (by running it):* the idle loop is the wait for the next
  frame at `0x71194`-`0x711A7`, a poll of the video status's frame bit
  (`0x98000C` bit 2) counting passes in g0, as `motoraid`'s:
  81% of a coin-start run's instructions are skipped (`make bench
  GAME=dynamcop`: 7.2x real time interpreted; state hash
  `4fc9232bb702240c`, the same with `--no-idle-skip` and after a save and
  load, `make state-check GAME=dynamcop`, whose audio matches too).
- *Confirmed (by running it):* **input lag** (`make input-lag
  GAME=dynamcop`, held from frame 3507 of a fight): a punch shows in the
  first frame that can read it (0), a jump two frames later (2). So
  `input_lag` is 0 and `run_ahead_max` 1, as `schamp`'s and `lastbrnx`'s
  are set where one control lags and the main one doesn't.

## Clones

`dynamcop.zip` also holds its clones' program chips:

- `dyndeka2`: Dynamite Deka 2 (Japan, 2A), `epr-20922.12`/`epr-20923.13`
  in place of `.12/.13`: **supported, a variant** (`crates/romset/src/games/variants.rs`,
  docs/variants.md). *Confirmed (by running it):* its two protection answers
  (the 16-byte replies to commands `0x1326` and `0x7700`) are 0x10 lower in its
  program than the parent's, 0xBC7E0 and 0xBC7F0 (found by searching the program
  for their bytes): with the parent's addresses the game stops on "SCROLL
  ALLOCATION ERROR" with a black screen, with these it attracts and plays. Its idle
  loop is the parent's 0x10 lower (0x71184); 82.8% of a 600M coin-start run is
  skipped.
Not done:

- `dynmcopb`: Dynamite Cop (export, 2B), `progrom0.15`/`progrom1.16`.
- `dyndek2b`: Dynamite Deka 2 (Japan, 2B), `epr-20936.15`/`epr-20937.16`.
- `dynmcopc`: Dynamite Cop (2C), `epr-20952.15`/`epr-20953.16`.

The data chips are the parent's; on 2B and 2C they will need their own
layout and the 2B/2C copro programs, and whether the device check is the
same there is unknown.

## Next

1. Play it by hand (`make play GAME=dynamcop`), and a scripted run that
   walks and fights through mission 1 (the coin-start script stands
   still).
2. The sound by ear, and its level.
3. The other missions, two players, and the clones above.
