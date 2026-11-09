# Virtua Fighter 2 (`vf2`, Model 2A-CRX, 1995)

Status: **in progress** (`romset::games::IN_PROGRESS`); not playable yet.

- *Confirmed:* the manifest verifies (33 chips, program set 2.1:
  `epr-18385..18388`). The i960 boots, uploads a 3802-word TGP program at
  frame 34 and runs the attract mode: with `--input coin-start` the screen
  shows the sky and desert backdrop, "INSERT COIN(S)" and the credit
  counter. No fighters or other 3D models appear (frames 1000, 2000).
- *Unconfirmed:* chip roles are by analogy with `srallyc` (main data
  `.10/.11 .8/.9 .6/.7 .4/.5`, polygons `.16/.20 .17/.21 .18/.22`, textures
  `.25/.24 .27/.26`, TGP data `.31/.32`); controls copied from `fvipers`.
- *Blocker:* the TGP runs words the interpreter does not know:
  `bf624019` (word 0x03E, the dispatch-table entry for function 0, which the
  game calls as an idle/main loop: 2.7 million executions in 300M
  instructions; the other entries are `bf60xxxx` jumps, so `62` is probably
  a call or jump variant), plus `4a000001`, `4a000003`, `1c1743a1` (once
  or twice each). The geometrizer also counts 243622 bad commands.
