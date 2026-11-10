# `vstriker`

## What's known about `vstriker`

Virtua Striker (Sega; Rev A; the title screen says ©SEGA 1994, the boot
warning "used only in Japan"), the sixteenth game and the eighth on
Model 2B, and the earliest 2B code here. `vstriker.zip` also holds the
original revision's program and `.7/.8` chips (the names without the `a`
suffix), the clone `vstrikro`, which isn't supported.

- *Confirmed (from the code, then the memory test, which shows every
  ROM, IC 7-16, and RAM GOOD):* program `.15/.16` at `0x0` and `.13/.14`
  at `0x40000` (128 KB chips); main data **packed** from `0x2000000`:
  `.11/.12` and `.9/.10` (2 MB chips, 4 MB a pair), then the 1 MB pair
  `.7/.8` at `0x2800000`, where the program reads it (its SHARC uploads
  start at `0x2825F30`) and where the later Sega games have nothing
  (Model 2B above). Polygons `.17/.21`, `.18/.22`, packed every 4 MB:
  the game draws models from geometrizer address `0x900000` on (the
  second pair), and in 8 MB slots the stands, advertising boards, pitch
  lines and players' shadows were missing. Textures `.27/.25`; four
  sample ROMs `.32-.34`; no copro data ROMs.
- *Confirmed (from its programs, then by running it):* both SHARC
  programs are its own. The geometrizer's (2,910 words of code, 3,040
  uploaded) is an earlier version of `indy500`'s, which has 2,896 of its
  words in the same order; the copro's (2,199 words) is closest to the
  Jaleco games' (2,012 of its 2,283 uploaded words). It uploads them as
  `fvipers` does and doesn't write MSGR7, so the geometrizer checks every
  command word's tag; none failed in coin-start runs through a match
  (`docs/hw/model2b.md`, "Sega's earlier games").
- *Confirmed (from the render RAM, the raster stream and by running
  it):* the match scene was dark because its textures were blank. The
  game uploads its texels only to `0x11100000` and `0x11300000`, the upper
  half of each texture RAM bank's window, as `fvipers` and `schamp` do;
  with 2 MB banks the rasteriser read the empty lower halves. The pitch,
  for example, is a 256×256 texture at (1024, 0) in bank 0 with palette
  colour (5, 10, 0) and polygon luma 230: every texel read 0, and luma
  table 0's entry 0 is 0, so it drew black (the stands, untextured
  white). The geometrizer's luma words, the palette and the luma and
  colour tables were as the game wrote them. With `texture_ram_mirrored`
  everything draws textured and lit.
- *Confirmed (from the code):* the game builds its colour translation
  tables at boot (i960 `0x7F10`, then `0x93F0`): linear ramps, scaled,
  then rows 27-31 replaced by curves interpolated from ROM data at
  `0x282E340`. Rows 30 and 31 (colour components 30 and 31) come out 0 in
  every 3D column, from zeros in that data, so a fully saturated palette
  component draws black. That is the game's own table, and not what
  darkened the scene, which draws lit once its textures are there
  (*unconfirmed* what, if anything, uses those two rows).
- *Confirmed (by the input test, then by running it):* port `0x1C00002`
  bit 0 coin 1, 1 coin 2, 2 test, 3 service, 4 start 1, 5 start 2 (one
  coin a credit); ports `0x1C00004`/`0x1C00006` for players 1 and 2: bit
  0 switch 1, 1 switch 2, 2 switch 3, 4 down, 5 up, 6 right, 7 left. The
  input test also shows switch 3 for bit 3 (*unconfirmed* why; nothing
  else seen reads it). Which switch does what comes from the attract
  mode's how-to-play, which shows three buttons, green, cyan and magenta,
  for short pass (and the sliding tackle when defending), long pass and
  shoot (held, it charges a power bar), and from runs from save states:
  at the kick-off switch 1 passes to a team-mate and switch 2 fills the
  power bar and shoots; at a throw-in, where the on-screen key crosses
  out the third button, switch 2 does nothing while switches 1 and 3
  throw. So switch 1 is short pass, 2 shoot and 3 long pass.
- *Confirmed (by running it):* `make run GAME=vstriker INPUT=coin-start`
  boots (the warning, then the rankings), attracts (the
  stadium, players, the title and a how-to-play), coins up, takes the
  team under the cursor (Denmark) and plays a two-minute match against
  France with goals, a replay, the half-time score and loss time, with
  sound. `make sound` of that run averages -32 dBFS RMS (peak -13), so
  the manifest adds no gain; no driver version string in its sound
  program. A coin-start run of 600M instructions gives the state hash
  `a4af0c62acc656f3` recompiled and interpreted (the game never writes its EEPROM in it).
  `make state-check GAME=vstriker` round-trips, and `make lockstep
  GAME=vstriker RECOMP=1 INPUT=coin-start` runs 600M instructions
  identical (i960 and both SHARCs recompiled). Its geometrizer and copro
  programs each have a recompiled crate of their own (`docs/recompiler.md`).
- *Confirmed (by running it beside the SHARC, `make geo-check
  GAME=vstriker`, and the fuzzer):* the native geometrizer
  (`Program::Vstriker`, `docs/hw/geometrizer-2b.md`) gives the SHARC's
  stream and state word for word over a 5,177-frame coin-start run
  (172.7M raster words) on the interpreted and the recompiled SHARC, and
  the 600M-instruction run ends with the same state hash in native mode.
- *Speed (by running it):* that run plays at 1.8x real time
  recompiled natively (the geometrizer on its thread, slowest frame
  46.0 ms), and at 0.69x interpreted (slowest frame 137 ms). The
  geometrizer is the limit: 36 of the run's 40 s, about 1.9 billion
  SHARC instructions, against 0.34 billion for the copro. Remeasured on
  a quieter machine: 2.73x recompiled; with the native geometrizer (the
  player's default) 3.87x recompiled and 1.55x interpreted, a match
  frame's geometry 1.5 ms against 6.8 on the recompiled SHARC and 27.7
  interpreted.
- *Unconfirmed:* at boot it writes a byte 0 to `0x1400000` (i960
  `0x3B80`), which nothing reads back and the board model ignores.
- *Input lag, measured (`make input-lag`: pictures with and without the
  control compared pixel by pixel, frames counted after the first that
  reads it; `docs/play.md`, "Run-ahead"):* moving the player (right from
  frame 3507, left from 3907 of a coin-start run) and shooting show in the
  first frame that reads them: input lag 0.
- *Confirmed (`make temporal-check` on captured sequences, 2026-11-10):*
  two things defeat smooth motion's pairing for motion vectors. The
  players' bodies are pose models that change with the animation, and the
  ball intro (from about frame 1044 of a coin-start run) draws a trail of
  four balls, each copy a different model, that restarts every 8 frames
  (each restart is a camera cut to the cut test). The plan pairs these
  transforms across models; for in-betweens that is harmless, but frame
  N's model posed with frame N-1's transform gave wrong motion vectors on
  about 1% of 3D pixels (limbs, the front ball, up to 128 game pixels).
  Transforms paired across models now give no motion data: 1.7% of 3D
  pixels in a match, which temporal anti-aliasing draws nearer the plain
  frame (`docs/enhancements.md`). Its crowd textures also keep a still
  frame changing under temporal's jitter: 0.21 levels a frame in the
  attract sequence (frames 470-549) and 0.34 in a match (3300-3419),
  the one result over `temporal-check`'s 0.25 limit.

## Lighting (smooth shading's side channel)

Measured with `remodel --geo native --light --input coin-start run 900000000` (6212 frames; `make lighting-capture` runs the same), reModel's smooth-shading side channel (docs/hw/geometrizer.md, "Lighting side channel"):

- Self-check: of 25,234,925 polygon records, the flat luma recomputed from the side channel is the stream's for 25,234,849 (100.00%), one level off for 76 and further off for 0 (the SHARC rounds toward zero).
- no specular lighting; normals computed from the vertices in 0.00% of blocks; zero (unlit) normals on 13.85% of polygons.
- no models drawn from polygon RAM; no direct data.
- the light vector changes within a frame in 4749 of 6212 frames (up to 2 vectors a frame); lighting entries not changed within a frame.
- True focal length over the window width: median 2.018 (range 1.859-2.430); the stream-only layer guesses 1.0.
- The list scan's block facts equal the native geometrizer's on 748,040 blocks, differ on 0.
