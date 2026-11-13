# `dynabb97`

## What's known about `dynabb97`

Eleventh game and the third on Model 2B, the first of Sega's own there:
Dynamite Baseball '97, a two-player baseball cabinet with a bat lever.

- *Confirmed (ROM, then the memory test):* program `epr-19833a.15/.16`
  then `.13/.14`; main data `.11/.12` at `0x2000000`, `.9/.10` at
  `0x2800000`, and a second bank `.7/.8` and `.5/.6` at `0x6000000`, as
  in `hotd`. The test menu's MEMORY TEST shows every ROM (IC 5-16) and
  RAM GOOD in that layout. Polygons `.17/.21` then `.18/.22` and
  textures `.27/.25` draw correctly. Four sample ROMs (`.32-.35`), no copro
  data, no music board. The boot warning says "used only in Japan"; the
  title screen says ©SEGA 1997.
- *Confirmed (by running it):* the board model runs it unchanged: the
  i960 uploads both SHARC programs as the Jaleco games do, the geometrizer
  code is theirs and the copro program is Sega's own (Model 2B above). No
  protection device: nothing is read outside the known map. Boot writes
  zeros once over `0x600000-0x62FFFF`, which reModel doesn't map and the
  game never reads back (*unconfirmed* what is there).
- *Confirmed (from the input test, then by scripted runs):* port
  `0x1C00002` bit 0 coin 1, 1 coin 2, 2 test, 3 service, 4 start 1, 5
  start 2 (one coin a credit); port `0x1C00004` player 1 and `0x1C00006`
  player 2, each bit 0 SW #1, 1 SW #2, 4 down, 5 up, 6 right, 7 left.
  SW #1 confirms the team and the starting pitcher. BAT VOLUME is analog
  channel 0 (player 1) and 1 (player 2): the input test shows 0 for
  `0x50` and below, `0xFF` from about `0xC0`, `0x6D` at `0x80`. Pulling
  the bat to `0xC0` and letting it back to `0x50` swings; holding it
  doesn't. *Unconfirmed:* that the lever rests at the low end (inferred
  from that), and what SW #2 does.
- *Confirmed (by running it):* `make run GAME=dynabb97 INPUT=coin-start`
  attracts, coins up, goes through team and pitcher select and plays
  innings (batting with strikes, balls and outs, then pitching and
  fielding). `make sound` of the run averages -34 dBFS RMS (peak -13),
  between `hotd`'s and `skisuprg`'s (same driver, CRX_drv 3.2.8), so the
  manifest adds no gain. `make state-check GAME=dynabb97` round-trips,
  `make lockstep GAME=dynabb97 RECOMP=1` runs 300M instructions of a
  coin-start run identical (i960 and both SHARCs recompiled), and a
  coin-start run of 600M instructions gives the same state hash
  (`69e50b78363de101`, with an `eeprom.bin` from an earlier run)
  recompiled and interpreted.
- *Speed (by running it):* that run plays at 2.74x real time recompiled
  natively with the SHARC block compiler (1.86x before it, when the
  geometrizer SHARC was the limit), and at about real time interpreted
  (slowest frame 32 ms).
- *Input lag, measured (`make input-lag`: pictures with and without the
  control compared pixel by pixel, frames counted after the first that
  reads it; `docs/play.md`, "Run-ahead"):* it depends on the moment:
  pulling the bat (frame 3507 of a coin-start run) shows a frame later
  (input lag 1), from 3700 six; the stick up or left from 3507 two, right
  from 4507 six. Button 1 from 3507 showed nothing for 16 frames. The
  least, 1, is the manifest's.
- *Depth buffer (2026-11-11; captured coin-start frames drawn with it off
  and on at 1440x1080 and 1920x1080 (`make depth-check`), compared with
  the build before the 2026-11-10 rules (`docs/enhancements.md`, "Depth
  buffer")):* 13 of 21 frames differ, by up to 0.4%. The 2026-11-10 rules
  helped: slivers of polygons that showed through clothes and hands are
  gone. Wrong with it on: in the close-up of a batter the black
  undershirt's collar shows through the jersey (the jersey is 2.9% of 1/z
  behind it there; the two are sorted 0.4% apart but fall in different
  groups). Can't tell: home plate is whole with it on and clipped by the
  dirt drawn after it with it off (0.9%).

## Lighting (smooth shading's side channel)

Measured with `remodel --geo native --light --input coin-start run 900000000` (6191 frames; `make lighting-capture` runs the same), reModel's smooth-shading side channel (docs/hw/geometrizer.md, "Lighting side channel"):

- Self-check: of 10,713,176 polygon records, the flat luma recomputed from the side channel is the stream's for 10,713,135 (100.00%), one level off for 41 and further off for 0 (the SHARC rounds toward zero).
- no specular lighting; normals computed from the vertices in 1.78% of blocks; zero (unlit) normals on 11.54% of polygons.
- models drawn from polygon RAM in 9,086 blocks (145,094 polygons, 1.78% of blocks); no direct data.
- the light vector changes within a frame in 1244 of 6191 frames (up to 3 vectors a frame); lighting entries not changed within a frame.
- True focal length over the window width: median 4.032 (range 0.414-12.097); the stream-only layer guesses 1.0.
- The list scan's block facts equal the native geometrizer's on 510,023 blocks, differ on 0.
- Smooth shading: the players' colour translation tables hold coloured entries in the dark columns (0-11) below a grey ramp, so most lit polygons (over 90% in the 21 captured frames) are shaded only within the levels of their own hue (docs/hw/geometrizer.md, "Colour tables are not always brightness ramps"); before that refinement 2,234 of 3,758 lit polygons were left flat.

## Enhanced preset

What the player's Enhanced picture preset (`romset::Recommended` in the
manifest, `run_ahead_max` for run-ahead) turns on for `dynabb97`, and why:
Temporal at full size; no depth buffer (the batter's undershirt collar
shows through the jersey, above) and no smooth shading (over 90% of the lit
polygons are shaded only within their own levels, below); smooth motion;
run-ahead 1.

## Idle loop

- *Confirmed (from the code):* the main program waits for the next frame
  at `0x82B18-0x82B2F`, a poll of the video status's frame bit
  (`ld 0x98000c`) like the other games', but each pass also calls
  `0x64C60` (which returns at once while the word at `0x5D4C08` is 0) and
  counts passes in `r5`, a local register. The call saves the caller's
  locals, the count among them, to its stack frame in work RAM, so every
  pass writes memory with a new value and none is the fixed point the
  idle skip needs (`docs/idle-skip.md`). The game is left without an
  idle loop: it runs every pass.
