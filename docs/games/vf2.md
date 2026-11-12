# Virtua Fighter 2 (`vf2`, Model 2A-CRX, 1995)

Status: **in progress** (`romset::games::IN_PROGRESS`); a game starts and
fights, but not playably yet.

- *Confirmed:* the manifest verifies (33 chips, program set 2.1:
  `epr-18385..18388`). The i960 boots and uploads a 3,802-word TGP program
  at frame 34, which now runs on the MB86234 interpreter with no word of
  an unknown form (`docs/hw/mb86234.md`, "What `vf2`'s program added").
  Attract shows the fighters and stages in 3D, then the warning, the SEGA
  logo and "INSERT COIN(S)". The default setting is two coins a credit
  ("CREDIT 1/2" after one): `coin-start` inserts two, starts, and its
  punches pick Akira, and a round against Lau begins (a coin-start run
  of 600M instructions: player select at frame ~1,650, "SET 1" ~1,750).
- *Confirmed (by running its functions with chosen arguments on the
  interpreter):* the TGP's functions that the game calls most do the
  expected mathematics: arithmetic, 1/sqrt, sqrt, sine and cosine,
  atan2 and asin, push, pop, identity, translate, scale, the three
  rotations, transforming points, distances, normalising. Its function
  numbers dispatch through a table at `0x3F` + F; the argument and reply
  counts of every function the game calls match its code.
- *Blocker:* in the fight a fighter falls through the ring within two
  seconds, with no input, and the round ends "RING OUT" (the replay shows
  him dropping below the floor among the stage's buildings). The floor
  and collision work is in TGP functions `0x4A` (the object list: transform
  each object's points, test them against planes, push them out, write
  them back into shared RAM) and `0x3A`/`0x3B`/`0x3E` (sphere collision
  tables); their geometry is unconfirmed, and so is anything the i960
  does with the results. Next: a `tgp-check` for `vf2` and a trace of
  function `0x4A`'s inputs and outputs around the fall.
- *Unconfirmed:* the picture: fighters and stages draw untextured or flat
  grey and black (the stage's buildings as black slabs), so either the
  texture roles below or the 2A renderer's handling of this game's
  polygons is wrong; the 3D fighter on the player-select screen does not
  appear.
- *Unconfirmed:* chip roles are by analogy with `srallyc` (main data
  `.10/.11 .8/.9 .6/.7 .4/.5`, polygons `.16/.20 .17/.21 .18/.22`, textures
  `.25/.24 .27/.26`); controls copied from `fvipers`.
- *Confirmed (from the ROM data and the sound driver; not yet by ear):*
  the samples are `.31 .32 .36 .37`, 2 MB each, back to back at the sound
  68000's `0x800000-0xFFFFFF`, tested as for `srallyc`
  (docs/games/srallyc.md). The sound program `epr-17574.30` has the same
  kind of sample table, at 68000 `0x605B28` (20-byte entries, 400 samples
  among empty rows), with samples in all four 2 MB slots, none crossing a
  boundary. The loops' 512-byte repeats after their ends are found only
  with `.31` in the first slot (18 of 24 loops) and `.32` in the second
  (25 of 30), in no other chip; the third slot has 3 loops (one repeat,
  in `.36`) and the fourth none, so `.36` and `.37` are placed by where
  their data ends: exactly at the third slot's last sample for `.36`, one
  byte after the fourth's for `.37` (`.31` 0x66 bytes before the first's,
  `.32` exactly at the second's); swapped, four more samples would be
  blank and the end of `.37` unreferenced. All four chips are 8-bit PCM
  by their statistics. The driver runs in a coin-start run (363 bytes
  from the host in 120 s; `sound-render`: 5134 key-ons, 24.8 MB of sample
  reads, all inside table entries). The role rests on the sound program's
  table and the chips' contents.
- *Confirmed (by running it; not yet by ear):* the manifest adds no gain
  (`volume_db: 0.0`). The 6 dB it had before clipped: a 1.5G-step
  coin-start run with `--audio` (the player's mix) measured about -13
  LUFS after 30 s, peaks at 0 dBFS and 3265 clipped samples. With no gain
  the same run gives -19.0 LUFS, peaks -1.4 dBFS and nothing clipped,
  inside the other games' gameplay range (-16 to -28 LUFS, median about
  -21.5; `srallyc.md`).

## Enhanced preset

What the player's Enhanced picture preset (`romset::Recommended` in the
manifest, `run_ahead_max` for run-ahead) turns on for `vf2`, and why:
Not playable yet, so the defaults: temporal at full size, no depth buffer
or smooth shading, nothing recorded either way; run-ahead from its input lag
once it plays.
