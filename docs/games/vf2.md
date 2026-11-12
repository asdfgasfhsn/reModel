# Virtua Fighter 2 (`vf2`, Model 2A-CRX, 1995)

Status: **in progress** (`romset::games::IN_PROGRESS`); not playable yet.

- *Confirmed:* the manifest verifies (33 chips, program set 2.1:
  `epr-18385..18388`). The i960 boots, uploads a 3802-word TGP program at
  frame 34 and runs the attract mode: with `--input coin-start` the screen
  shows the sky and desert backdrop, "INSERT COIN(S)" and the credit
  counter. No fighters or other 3D models appear (frames 1000, 2000).
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
  reads, all inside table entries). The TGP made no external-bus reads in
  that run (it runs into unknown words, below), so its side is not yet
  checked: the role rests on the sound program's table and the chips'
  contents.
- *Confirmed (by running it; not yet by ear):* the manifest adds no gain
  (`volume_db: 0.0`). The 6 dB it had before clipped: a 1.5G-step
  coin-start run with `--audio` (the player's mix) measured about -13
  LUFS after 30 s, peaks at 0 dBFS and 3265 clipped samples. With no gain
  the same run gives -19.0 LUFS, peaks -1.4 dBFS and nothing clipped,
  inside the other games' gameplay range (-16 to -28 LUFS, median about
  -21.5; `srallyc.md`).
- *Blocker:* the TGP runs words the interpreter does not know:
  `bf624019` (word 0x03E, the dispatch-table entry for function 0, which the
  game calls as an idle/main loop: 2.7 million executions in 300M
  instructions; the other entries are `bf60xxxx` jumps, so `62` is probably
  a call or jump variant), plus `4a000001`, `4a000003`, `1c1743a1` (once
  or twice each). The geometrizer also counts 243622 bad commands.

## Enhanced preset

What the player's Enhanced picture preset (`romset::Recommended` in the
manifest, `run_ahead_max` for run-ahead) turns on for `vf2`, and why:
Not playable yet, so the defaults: temporal at full size, no depth buffer
or smooth shading, nothing recorded either way; run-ahead from its input lag
once it plays.
