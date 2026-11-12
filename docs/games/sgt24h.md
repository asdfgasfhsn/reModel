# `sgt24h`

## What's known about `sgt24h`

Tenth game and the second on Model 2B: Jaleco's Super GT 24h, a sit-down
driving cabinet. Its i960 program is its own; its two SHARC programs are
`overrevb`'s code, and so are its board's needs, with two exceptions
below (the register cache and the I/O type).

- *Confirmed (ROM):* program `epr-19155.15`/`epr-19156.16` (`.15` low),
  build dates "Jul 5 1996" and "Jul 6 1996"; the title screen shows no
  copyright line, so the manifest's year 1996 is the build's. Main data
  `.11/.12` at `0x2000000` and `.9/.10` at `0x2800000`: of the 1,737
  program words pointing into `0x2000000-0x2FFFFFF`, 238 land on filler in
  that order and 1,394 the other way round. Textures `.27/.25` and
  polygons `.17/.21` (4 MB chips, a pair to a slot) draw correctly in that
  order. One sample ROM (`.32`), no copro data, no music board. The
  memory test passes ("ROM GOOD", "RAM GOOD").
- *Confirmed (ROM):* the sound program `epr-19157.31` is Jaleco's driver:
  its first 32 KB are byte for byte `overrev`'s `epr-20002.31`. It runs
  unchanged; a coin-start race's `make sound` averages -37.7 dBFS RMS
  (peak -19.1), as `overrev`'s does, so the manifest gives it the same
  +6 dB.
- *Seen in captures (2026-11-11):* the game rewrites part of its colour
  translation tables during the race: 384 of the 6144 entries the renderer
  reads differ in the coin-start captures from frame 2400 to 5400 against
  frames 600-2100 and 5700-6212, and the sky is drawn warmer then. What it stands for (a time of
  day, a lighting change) is *unconfirmed*. It is why texture dumps keep the
  most drawn look (`docs/texture-packs.md`, "Looks").
- *Confirmed (by running it):* the i960 uploads the copro and geometrizer
  SHARC programs exactly as `overrevb` does, and they reach the same code
  (2,111 and 2,987 words, `remodel sharc-check` on both games' images). The
  images differ past the code (see Model 2B), so the recompiled crates
  aren't shared: `sgt24h` gets its own two `sharc_<hash>` crates with the
  same code in them.
- *Confirmed (from the code):* boot needs the i960's local-register cache
  (Model 2B above); with it the game boots to "CHECKING NETWORK NOW",
  "STAND ALONE" and attract. The network check is `overrev`'s: it probes
  the communication board at `0x1A14000` (bit 0 set: no board) and with
  the default link setting ("NOT LINK") runs stand-alone.
- *Confirmed (from the code, then by running it):* the GAME ASSIGNMENTS
  menu's I/O TYPE (A to E) is a 16-bit word at backup SRAM `0x1D0000A`.
  Unlike the other settings, which the game resets when the SRAM's
  `0xAD85` signature is missing, it's never initialised, and the I/O code
  uses it unchecked as an index into an 8-byte-entry table of pedal
  scalings (program `0x5EB80`): type A (0) passes the gas and brake
  through, B-E (1-4) take `0x100 - raw`. Types B and D also list a VIEW 2
  button and send an output byte to I/O `0x1C00008` (in place of
  `0x1C0000A`). With the SRAM's power-on `0xFFFF` the index runs past the
  table into code and the pedals come out as `raw × 12 + 2` (the input test
  showed `00F6H` for a released pedal), so the gas never registered. The
  menu shows any value above 4 as A, so a fresh board looks set to A but
  isn't; the manifest's new `backup_ram` field puts 0 there at power-on,
  as an operator saving type A would. *Unconfirmed:* which cabinet each
  type is for, and that A is the standard one (it's the one whose pedals
  read as `overrev`'s do).
- *Confirmed (from the input test, then by scripted runs):* port
  `0x1C00002` bit 0 coin (one coin a credit), 2 test, 3 service, 6 start;
  port `0x1C00004` bit 0 view (cycles the camera), 4 shift up and 5 shift
  down (sequential: 1-2-3-4 and back with the manual gearbox). The game
  reads bits 4-7 of that port as a lever position (`0x10`, `0x20`,
  `0x40`, `0x80`, none: 0-4); the input test shows only the first two.
  Analog channels: 0 wheel (0x30 left, 0xD0 right; it moves the menu
  selection), 1 gas and 2 brake, 0xFF released and 0x00 down with type A.
  The game calibrates the pedals and wheel from the extremes it sees
  (and has a calibration screen, "MAX MIN CENTER"). A gas press confirms
  the course, car and gearbox (AT or MT, chosen with the wheel).
- *Confirmed (by running it):* no drive board. There is a STEERING FORCE
  setting, but with type A the game writes no drive command; the outputs
  it writes each frame go to `0x1C0000A` (*unconfirmed*: lamps).
- *Unconfirmed (ROM):* `epr-18643a.7`, on another board, is a Z80
  program (it starts with a jump, uses IN/OUT ports `0x00`-`0x70` and RAM
  at `0x8000` up; 1.8 KB of 128 KB used) whose header reads "Copyright
  SEGA ENTERPRISES, LTD. 1995", "$Id: 837-10074.s ...", "AM Hardware
  R&D". The same chip is in `stcc` and `von`, both linkable games, and
  the i960 program never reads it, so it's most likely the
  communication (link) board's firmware. The game needs nothing from it:
  with no board it runs stand-alone.
- *Confirmed (by running it):* `make run GAME=sgt24h INPUT=coin-start`
  attracts, coins up, goes through the course, car and gear menus and
  races until time runs out, then shows GAME OVER and returns to attract.
  `make state-check GAME=sgt24h` round-trips, `make lockstep GAME=sgt24h
  RECOMP=1` runs 300M instructions of a coin-start run identical (i960
  and both SHARCs recompiled), and a coin-start run of 600M instructions
  gives the same state hash (`282c8d6fd5151c21`, with or without an
  `eeprom.bin` from an earlier run) recompiled and interpreted.
- *Speed (by running it):* recompiled, `make bench GAME=sgt24h RECOMP=1`
  runs at 1.6x real time natively (the geometrizer SHARC is the limit,
  slowest frame 24.5 ms), so it plays at full speed as `overrevb` does;
  interpreted it races below real time.
- *Unconfirmed:* steering and braking were only checked as far as a
  scripted run shows (the unsteered car ends against the wall).
- *Confirmed (from the raster stream):* the shadow under the player's
  car in a race is a set of flat dark-grey polygons (palette colour
  (2, 2, 2)) with the checker bit set (header `80C0`), so they draw as a
  screen-door half-tone over the track, as `waverunr`'s arch shadow does.
- *Input lag, measured (`make input-lag`: pictures with and without the
  control compared pixel by pixel, frames counted after the first that
  reads it; `docs/play.md`, "Run-ahead"):* the wheel (frames 3507 and 3907
  of a coin-start run) shows four frames after the first that reads it:
  input lag 4, the most of the sixteen games; run-ahead of 3 frames hides
  three of them.

## Lighting (smooth shading's side channel)

Measured with `remodel --geo native --light --input coin-start run 900000000` (6163 frames; `make lighting-capture` runs the same), reModel's smooth-shading side channel (docs/hw/geometrizer.md, "Lighting side channel"):

- Self-check: of 16,714,724 polygon records, the flat luma recomputed from the side channel is the stream's for 16,714,087 (100.00%), one level off for 637 and further off for 0 (the SHARC rounds toward zero).
- no specular lighting; normals computed from the vertices in 0.00% of blocks; zero (unlit) normals on 17.91% of polygons.
- no models drawn from polygon RAM; no direct data.
- the light vector changes within a frame in 139 of 6163 frames (up to 3 vectors a frame); lighting entries not changed within a frame.
- True focal length over the window width: median 0.605 (range 0.605-1.210); the stream-only layer guesses 1.0.
- The list scan's block facts equal the native geometrizer's on 279,408 blocks, differ on 0.

## Enhanced preset

What the player's Enhanced picture preset (`romset::Recommended` in the
manifest, `run_ahead_max` for run-ahead) turns on for `sgt24h`, and why:
Temporal at full size; no depth buffer or smooth shading (little or no
change); smooth motion; run-ahead 3: the lag measured above is 4, the player
offers 3 at most. With temporal and run-ahead 1 on an interpreted
machine it fell behind once (`docs/play.md`, "Temporal anti-aliasing"); the
player then drops run-ahead and says so.
