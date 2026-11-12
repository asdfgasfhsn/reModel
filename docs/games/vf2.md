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
- *Confirmed (by the pictures):* the stages and fighters are textured
  (stone towers, trees, brick floor, clothes and faces). The grey was
  not the chip roles, the polygons' attributes or the renderer: the
  texture headers and point lists (the `.25/.24` pair) decoded all
  along, and the polygons drew as textured with sensible headers (5,700
  distinct textures in a 200M-instruction run), but their texel sheets
  were empty. The game's boot copy fills Model 2A's video RAM only in
  the upper half of each bank's window (bank 0 words `0x80000`-`0xFFFFF`
  from `0x12200000`, bank 1 the same from `0x12600000`), where
  `srallyc` fills the lower half. A bank is 512K words (the sheet's
  1 MB), so its window's 2 MB of 32-bit words repeat, and the board model
  kept the upper half at an address the rasteriser never reads.
  `model2::bus::mirror_video_2a` now folds each bank's words to its 512K.
  Whether the hardware repeats the window or `vf2` addresses the bank
  differently is not told apart (both draw the same). Unconfirmed: the
  second texture pair `.27/.26` (nothing in the first seconds reads
  past the first pair's headers that was checked).
- *Unconfirmed (a TGP result, not drawing):* the 3D fighter on the
  player-select screen. The list at that screen (frame ~1,440) has no
  window command (the window of the frame before is the full screen, 7
  window commands a frame elsewhere), carries only two objects (a
  323-polygon one and a 14-polygon one), each under an identity matrix
  with zero translation, so every vertex lands at depth about 0 (z
  between -0.3 and 0.05, where the fight's same 323-polygon object sits
  at z 1.8-2.1) and is culled. The TGP calls in that frame are few (48:
  functions `0x01`, `0x02`, `0x03`, `0x06`, `0x0E` called with zeros,
  `0x16`, `0x24`, `0x26`, `0x29`, `0x2C`, `0x2F`, `0x32`, `0x34`,
  `0x35`, `0x5E`, `0x6C`-`0x6F`) and the matrices come out identity:
  the translation the game expects (and the other fighter parts) is
  most likely produced by functions the select screen calls that return
  zero or nothing here, `0x34`/`0x35` (the arguments are `0x8050`,
  `0x8098`; the reply of `0x34` is 0 each time). The TGP never reads its
  external data ROM (`0x800000` up) in `vf2` (checked: no reads in a
  coin-start run), and `vf2`'s set has no chip for it. Re-sending the
  previous frame's window for a list with none was tried and changes
  nothing (the polygons are then culled for their depth), so it is not
  kept. Next: trace `0x34`/`0x35` and the function that should leave a
  translation in the matrix on this screen.
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
