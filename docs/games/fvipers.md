# `fvipers`

## What's known about `fvipers`

Fighting Vipers (Sega; the title screen says ©SEGA 1995, the boot
warning "used only in Japan"), the fourteenth game and the sixth on
Model 2B, with early Sega 2B SHARC programs.

- *Confirmed (ROM, then the memory test, which shows every ROM, IC 5-16,
  and RAM GOOD):* program `.15/.16` at `0x0` and `.13/.14` at
  `0x40000` (128 KB chips); main data `.11/.12` and `.9/.10`; the 1 MB
  pairs `.7/.8` and `.5/.6` in the second data bank, each repeating
  through its 8 MB slot (the program reads them at `0x64xxxxx` and
  `0x6Cxxxxx`, 4 MB in, as `indy500` does; *unconfirmed* where they
  really sit). The SHARC programs are uploaded from `.7/.8` like any data.
  Polygons are three pairs of 2 MB chips, `.17/.21`, `.18/.22`,
  `.19/.23`, **packed** back to back every 4 MB (in 8 MB slots the
  fighters from the second and third pairs drew as large dark shards);
  textures `.27/.25` and `.28/.26` in 8 MB slots (packed, Bahn's coat drew
  with the wrong texture). The copro data ROMs `.29/.30` are the same
  chips as `schamp`'s.
- *Confirmed (from its programs, then by running it):* the geometrizer
  SHARC program is `indy500`'s, word for word (2,983 words of code; the
  same recompiled crate); the copro program is its own (4,267 words). The
  i960 doesn't write MSGR7, so the geometrizer checks every command
  word's tag, and both things that stopped it are now in the board model
  (Model 2B above): the copro writes parts of the display list into
  buffer RAM (fn `0x78`), and the i960 patches list jumps in buffer RAM.
- *Confirmed (from the code and the memory test):* its textures go to
  the upper half of each texture RAM bank (`0x11100000`, `0x11300000`);
  the manifest's `texture_ram_mirrored` makes each bank 1 MB, repeating
  through its window, and everything draws textured (Model 2B above).
- *Confirmed (by the input test, then by running it):* port `0x1C00002`
  bit 0 coin 1, 1 coin 2, 2 test, 3 service, 4 start 1, 5 start 2 (one
  coin a credit); ports `0x1C00004`/`0x1C00006` for players 1 and 2: bit
  0 punch, 1 kick, 2 guard, 4 down, 5 up, 6 right, 7 left.
- *Confirmed (by running it):* `make run GAME=fvipers INPUT=coin-start`
  boots, attracts (character introductions and a fight demo), coins up,
  takes Grace at the character select and fights Bahn in the first stage
  through rounds with sound. `make sound` of that run averages -32 dBFS
  RMS (peak -12), so the manifest adds no gain. A coin-start run of 600M
  instructions gives the state hash `96bd74a28b0455b6` recompiled and
  interpreted (the game never writes its EEPROM in it). `make
  state-check GAME=fvipers` round-trips, and `make lockstep GAME=fvipers
  RECOMP=1 INPUT=coin-start` runs 600M instructions identical (i960 and
  both SHARCs recompiled). Its geometrizer runs `indy500`'s crate, its
  copro program a crate of its own.
- *Speed (by running it):* that run plays at 2.83x real time recompiled
  natively (the geometrizer on its thread, slowest frame 10.1 ms), and at
  0.82x interpreted (slowest frame 34.7 ms).
- *Confirmed (from the raster stream):* character shadows are opaque
  black polygons. On the floor they are translucent 32×32 and 32×64
  textures (headers `6000`/`6008`, palette entry 247 = `0x8000`, black)
  whose texels are only 14 (the shadow) and 15 (transparent); the large
  silhouettes on the walls in the introductions use the plain texture
  `4009 0000 1818` with palette entry 1, black. Nothing in their attribute
  words, luma words or headers sets them apart, and the checker bit
  (which the game does set, on the arena fences) is clear, so they draw
  solid black, as sent. *Unconfirmed:* how they look on the hardware.
- *Confirmed (from the game's code):* the 128 halfwords at `0x12800000`
  are written at boot as the ramp (i + 1) / 2 (i960 `0x474`), and
  otherwise only by a test menu's "TRANS MAP" / "TRANS NUM" items (i960
  `0x57544`), which copy there one of the seven 128-byte luma tables in
  ROM at `0x064266E0` (the same tables the game copies to luma RAM at
  `0x11400000`, i960 `0x12024`). So it is a luma-table-shaped map, left
  at its boot value in play. *Unconfirmed* what the hardware does with
  it; the board model has nothing there, and frames look right without
  it.
- *Input lag, measured (`make input-lag`: pictures with and without the
  control compared pixel by pixel, frames counted after the first that
  reads it; `docs/play.md`, "Run-ahead"):* a punch (frame 3707 of a run
  whose punches and kicks stop at 3400) shows a frame after the first that
  reads it: input lag 1; from 3307 (they stop at 3000) eight, mid-move.
  Walking showed nothing for 16 frames while coin-start's punches carried
  on.
- *Depth buffer (2026-11-11; captured coin-start frames drawn with it off
  and on at 1440x1080 and 1920x1080 (`make depth-check`), compared with
  the build before the 2026-11-10 rules (`docs/enhancements.md`, "Depth
  buffer")):* 18 of 21 frames differ, by at most 0.05%: costume layers and
  limbs of the fighters by a few pixels (1-3% of 1/z), a wall base going
  under the arena floor (9%). Can't tell which is right; none looks
  broken. The 2026-11-10 rules changed at most 900 pixels a frame, nearly
  all back to the game's order.

## Lighting (smooth shading's side channel)

Measured with `remodel --geo native --light --input coin-start run 900000000` (6205 frames; `make lighting-capture` runs the same), reModel's smooth-shading side channel (docs/hw/geometrizer.md, "Lighting side channel"):

- Self-check: of 19,948,488 polygon records, the flat luma recomputed from the side channel is the stream's for 19,948,391 (100.00%), one level off for 97 and further off for 0 (the SHARC rounds toward zero).
- specular on in 72.27% of object blocks, 64.17% of polygons lit with a specular term; normals computed from the vertices in 29.63% of blocks; zero (unlit) normals on 9.20% of polygons.
- Specular highlights as drawn (`make spec-stats`, the 21 frames of a `make lighting-capture`, 960x720, 2026-11-12): 61% of the textured polygons shown have a specular term (scales 255 and 144, squared one to three times), 29% of the textured pixels; it brightens 4.8% of the textured pixels by a level or more and 3.6% by 4 or more, 38 levels on average, up to 187: the fighters' bodies, armour and clothes; the arena shows none.
- models drawn from polygon RAM in 9,450 blocks (179,550 polygons, 1.90% of blocks); no direct data.
- the light vector changes within a frame in 109 of 6205 frames (up to 2 vectors a frame); lighting entries not changed within a frame.
- True focal length over the window width: median 0.968 (range 0.968-0.968); the stream-only layer guesses 1.0.
- The list scan's block facts equal the native geometrizer's on 496,220 blocks, differ on 0.

## Enhanced preset

What the player's Enhanced picture preset (`romset::Recommended` in the
manifest, `run_ahead_max` for run-ahead) turns on for `fvipers`, and why:
Temporal at full size; smooth shading on (the fighters are clearly better
in the review pictures); no depth buffer (18 of 21 frames differ by a few
pixels and "can't tell which is right", above); smooth motion; run-ahead 1.
