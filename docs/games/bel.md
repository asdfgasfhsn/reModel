# `bel`

## What's known about `bel`

Behind Enemy Lines, a two-player game on 2C by EPL Productions for Sega:
each player works a gun mounted on a vehicle that drives itself through
the mission. The title screen says ©1996, 1997 SEGA / REAL 3D (the dat
says 1998). The sound program is EPL's (`docs/hw/sound.md` §7).

- *Confirmed (from the code, then the memory test):* program `.15/.16` at
  `0x0` and `.13/.14` at `0x40000` (128 KB chips; the code's pointers run
  straight on); main data `.11/.12`, `.9/.10`, `.7/.8`, `.5/.6` packed at
  `0x2000000`, `0x2400000`, `0x2800000` and `0x2C00000` (tilemaps at
  `0x28xxxxx` and a pointer table at `0x2C00000` show it). With 8 MB slots
  boot read garbage tilemap sizes and copied 13 MB past texture RAM. The
  memory test then passes IC5-16 and every CPU board RAM.
- *Confirmed (by running it):* polygons are four pairs (`.17/.21`,
  `.18/.22`, `.19/.23`, `.20/.24`, low halves `.17`-`.20` by their float
  exponents), packed every 4 MB like the main data; with 8 MB slots
  enemies and buildings came out as huge flat shards. Textures are two
  pairs (`.27/.25`, `.28/.26`) in 8 MB slots (texture addresses reach
  `0x46xxxx` half-words, the second slot); copro data the 4 MB `.29/.30`;
  samples `.32`-`.35`; `.31` the sound program.
- *Confirmed (from the code):* the routine at `0x27030` uploads the copro
  program from program ROM `0x27090`, only when the status at `0x980010` or
  `0x980014` shows bit 0 (see `docs/hw/model2c.md`).
  The geometrizer program is the same 4,096 words as `hotd`'s, `skisuprg`'s,
  `segawski`'s and `waverunr`'s.
- *Confirmed (from the code):* sound commands go out a byte per UART
  transmit interrupt (`0x6F930`); with no such interrupt the game ran
  silent.
- *Confirmed (from the input test, then by scripted runs):* port
  `0x1C00002` bits 0/1 coin chutes, 2 **service**, 3 **test** (the other
  way round from most games here), 4/5 START 1/2; port `0x1C00004` bits
  0/1 the guns' triggers, 4/5 their missile buttons ("launch button
  located at front of gun"). The guns are potentiometers on the ADC:
  channel 0 player 1's yaw, 1 player 2's yaw, 2 player 1's pitch, 3
  player 2's pitch (`GunSource::Analog`). One coin a credit by default.
- *Confirmed (from the test menu):* GUN CALIBRATION asks for each gun at
  its lower-left then upper-right stops: yaw grows to the right and pitch
  upwards. With the defaults of a fresh EEPROM the crosshair can't reach
  the view's edges; the manifest's ranges (X −19 to 273, Y 273 to −18,
  clamped to 0-255) put it on the aimed pixel to within a few pixels,
  measured from the crosshair lines at three aims.
- *Seen, unconfirmed:* an `eeprom.bin` written while probing the test
  menu left the guns not moving in game; a fresh one works. Small blue
  specks show through some terrain seams.
- *Confirmed (by running it):* the coin-start script plays the first
  mission to the game over, scoring hits; `make sound GAME=bel` renders
  the commands with music throughout. `make state-check GAME=bel`
  round-trips, `make lockstep GAME=bel` runs 300M instructions identical,
  and a coin-start run of 600M instructions gives the same state hash
  (`25f923c57d7362eb`, with an `eeprom.bin` from an earlier run)
  recompiled and interpreted.
- Players aim with the generic gun controls; the right mouse button (or
  the pad's left trigger / B) fires a missile, as the manifest makes the
  missile button stand in for reloading (`Reload::Button`).
- *Input lag, measured (`make input-lag`: pictures with and without the
  control compared pixel by pixel, frames counted after the first that
  reads it; `docs/play.md`, "Run-ahead"):* a shot (the trigger from frame
  3507 of a run whose shooting stops at 3400) reaches the display list two
  frames after the first that reads it and the picture three: input lag 3.
  From 3907 the list changed but nothing visible for 16 frames; the
  missile button and the aim alone (frames 2810-4010) never showed (the
  aim shows only through hits). *Unconfirmed* beyond the one frame.
- *Depth buffer (2026-11-11; captured coin-start frames drawn with it off
  and on at 1440x1080 and 1920x1080 (`make depth-check`), compared with
  the build before the 2026-11-10 rules (`docs/enhancements.md`, "Depth
  buffer")):* the 2026-11-10 GPU 1/z brings back the ivy on the cliff that
  the build before drew the rock face over (frames 5100-6212). Right with
  it on: a hanging soldier's legs in front of a beam (the beam is 3.6% of
  1/z behind them).

## Lighting (smooth shading's side channel)

Measured with `remodel --geo native --light --input coin-start run 900000000` (6155 frames; `make lighting-capture` runs the same), reModel's smooth-shading side channel (docs/hw/geometrizer.md, "Lighting side channel"):

- Self-check: of 16,248,589 polygon records, the flat luma recomputed from the side channel is the stream's for 16,248,589 (100.00%), one level off for 0 and further off for 0.
- specular on in 100.00% of object blocks, 36.17% of polygons lit with a specular term; normals computed from the vertices in 0.00% of blocks; zero (unlit) normals on 18.18% of polygons.
- Specular highlights as drawn (`make spec-stats`, the 21 frames of a `make lighting-capture`, 960x720, 2026-11-12): 30% of the textured polygons shown have a specular term (scales 255, 43, 16, not squared), 21% of the textured pixels; it brightens 6.1% of the textured pixels by a level or more and 5.9% by 4 or more, 25 levels on average, up to 111: large faces of props and vehicles (a boxcar's side, a gun turret), each face brightened evenly as the game lights per polygon.
- no models drawn from polygon RAM; no direct data.
- the light vector changes within a frame in 4204 of 6155 frames (up to 2 vectors a frame); lighting entries not changed within a frame.
- True focal length over the window width: median 0.664 (range 0.664-0.805); the stream-only layer guesses 1.0.
- The list scan's block facts equal the native geometrizer's on 312,574 blocks, differ on 0.

## Enhanced preset

What the player's Enhanced picture preset (`romset::Recommended` in the
manifest, `run_ahead_max` for run-ahead) turns on for `bel`, and why:
Temporal at full size; the depth buffer on (the ivy on the cliff and a
hanging soldier's legs in front of a beam come out right, above); no
smooth shading (no pictures recorded as better); smooth motion; run-ahead 3,
the lag measured above (one measurement).
