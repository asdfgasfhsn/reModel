# `doa`

## What's known about `doa`

Dead or Alive (Tecmo, 2B, Japan), the twelfth game and the fourth on
Model 2B. It boots, attracts with 3D, coins up and fights on the board
model as it is, once the i960 returns keep PFP's address bits 4 and 5
(Model 2B above), and plays on once its protection device is answered:

- *Confirmed (from the code, then by running it):* at boot (i960
  `0x5A0`) the game polls `0x1D8400C` up to 10,000 times for `0xFFF0`.
  If it sees it, it sends a command, a block of data and two tables to
  the device at `0x1D80000` to `0x1D87FFF` (i960 `0x29F0`, its only code
  that touches the device), reads an answer back from `0x1D87FF8` and
  compares it with the 48 bytes at ROM `0x2810` (its copyright and
  version line), keeping the result at `0x54FABC`. With no device the
  result is 0, and starting a game (i960 `0x3558`) then arms a random
  countdown at `0x54F3B0` (300 to 1,199 frames; 892 in a coin-start
  run). When it runs out (i960 `0x2FA4`) the game kills its own tasks:
  the fight freezes with only the background drawn. A checksum of the
  boot code that makes the check (i960 `0x520`, result at `0x54FAB0`)
  arms the same countdown when it fails. The interface, step by step, is
  in `docs/hw/model2b.md`, "The protection device".
- *Confirmed (from the ROM data):* the two tables are a canonical Huffman
  code whose symbols are the characters of that ROM line, sorted by how
  often they occur in it; but the block isn't that code as it stands, so
  the device does something more to it first. *Unconfirmed* what: one
  request is all there is to go on. The board model therefore doesn't
  compute the answer. The manifest names the ROM bytes the game compares
  it with (command 6: 48 bytes at `0x2810`), read from the ROM at run
  time, and the device streams those back. That answers the game's check
  without patching it; it doesn't model the device for any other input.
- *Confirmed (by running it):* with the check answered `0x54FABC` is 1,
  the countdown never arms, and a coin-start run (`make run GAME=doa
  INPUT=coin-start`, punch and kick every half second) plays 6,200
  frames: character select, a won bout against the first opponent over
  several rounds with K.O.s, replays and "GET READY", then the next
  opponent, 4,700 frames past where it used to freeze. Sound plays
  throughout; `make sound` of that run averages -30 dBFS RMS (peak -11),
  louder than the games at 0 dB, so the manifest takes 3 dB off. `make
  state-check GAME=doa` round-trips, and a coin-start run of 600M
  instructions gives the state hash `78f308b4da95cefb` (with an
  `eeprom.bin` from an earlier run; `ba5d806c8790a9a2` before the
  polygon ROM repair below) recompiled and interpreted. `make
  lockstep GAME=doa RECOMP=1 INPUT=coin-start` runs 600M instructions
  identical (i960 and both SHARCs recompiled). Its geometrizer code is
  the other 2B games' (2,987 words, the same crate); its copro program is
  its own (3,261 words, a crate of its own).
- *Speed (by running it):* that run plays at 2.69x real time recompiled
  natively (the geometrizer on its thread, slowest frame 14.3 ms), and at
  0.82x interpreted (slowest frame 50.3 ms).
- *Confirmed (by running it):* the input test gives port `0x1C00002` bit
  0 coin 1, 1 coin 2, 2 test, 3 service, 4 start 1, 5 start 2 (one coin a
  credit), and ports `0x1C00004`/`0x1C00006` for players 1 and 2: bit 0
  hold, 1 punch, 2 kick, 4 down, 5 up, 6 right, 7 left. Laid out as
  `dynabb97`'s (program `.15/.16`; main data `.11/.12`, `.9/.10`; a
  second bank `.7/.8`, `.5/.6`; polygons `.17/.21`, `.18/.22`,
  `.19/.23`; textures `.27/.25`; four sample ROMs) its attract demo and
  fights draw correctly; the memory test wasn't run. The Model 2A chips
  also in `doa.zip` belong to the 2A clone `doaa`.
- *Confirmed (from the ROM data):* the set's dump of `mpr-19324.19`, the
  low half of the third polygon pair (`0xC00000-0xDFFFFF` to the
  geometrizer: Kasumi's models, and others'), is bad on data line 9: it
  reads about 60% of that bit's 1s as 0. Its last quarter holds the
  same data as the last quarters of `.22` and `.23`, and differs from
  them in 138,737 of 524,288 words, every one of them bit 9 read 0
  where the others have 1 (none the other way, no address pattern); in
  its first three quarters bit 9 is set in 38-45% of nonzero words,
  its other bits in 58-71%. Floats lose a
  mantissa bit (at most 2^-14 relative), but in a polygon's attribute
  word bit 9 is the link type's high bit: Kasumi drew as scattered
  shards with holes, in fights and on the character select screen (the
  "dark blocks over the face and hair" seen there before), while the
  geometrizer transformed every vertex as the ROM gives it (checked
  against a transform of the ROM data, object by object) and Zack, from
  the first pair, drew right. Bits 23-24 of an attribute word copy bits
  8-9 (`docs/hw/geometrizer.md` §3.3) and come from the intact `.23`:
  linked by them, her meshes are coherent (every polygon's stored normal
  agrees with its vertices' winding, 199 of 199 in her largest head
  model, against 145 of 197 linked by the dump's bits 8-9). The manifest
  declares the bad line (`romset::Dropout`) and loading restores bit 9
  of attribute-shaped words from bit 24 (45,916 words); Kasumi then
  draws whole in fights and on the character select screen. The chip on
  a real board is presumably good (*unconfirmed*: no other dump to
  compare). Hairline cracks of background pixels along some polygon
  edges remain on the large character select models, Jann Lee's from the
  intact first pair too, so they are the rasteriser's, not the dump's
  (*unconfirmed* why; not the near plane below: they stay with it at
  10^-6, on polygons well beyond it).
- *Confirmed (by running it):* the camera's close-ups (attract mode, the
  character select screen) put fighters' arms, heads and bodies less
  than one unit in front of the eye. reModel's rasteriser clipped at z =
  1, so those parts vanished as if the camera had cut them away. It now
  clips to the view volume with a near plane at 10^-6
  (`docs/hw/geometrizer.md`, reModel status), and they draw whole.
- *Confirmed (from the raster stream):* fighters' shadows are flat
  (untextured) black polygons, header `00C0 0000 0000 0000` (palette
  entry 0), checker bit clear, so they draw solid black, as sent.
  *Unconfirmed:* how they look on the hardware.
- *Confirmed (from tile RAM; enhancement):* the stage skies (sunset,
  clouds) are tiles on plane B (map 2), not 3D, so wide views first
  drew them only across the original 4:3 frame. The map is a 512-pixel
  panorama the game pans with the camera, streaming new columns in
  ahead of the scroll (a seam moves across it), and holds a constant
  vertical scroll (`0x40`). Wide views now carry scrolled plane-B rows
  on past the frame's edges, round the map, which fills the sides; the
  sky repeats beyond the 16 columns the game keeps off screen
  (`model2::video` `widen_base`). The same fills fvipers', dynabb97's,
  vstriker's and bel's skies and waverunr's water. Unscrolled rows are
  left alone: on select screens (topskatr, skisuprg, segawski, overrev)
  plane B is a page laid out for the frame and would repeat its titles.
- *Input lag, measured (`make input-lag`: pictures with and without the
  control compared pixel by pixel, frames counted after the first that
  reads it; `docs/play.md`, "Run-ahead"):* a punch, and walking right
  (frame 3507 of a run whose punches and kicks stop at 3400), show a frame
  after the first that reads them: input lag 1. While coin-start's punches
  carry on, walking showed nothing for 16 frames (the moves under way take
  it).

## Lighting (smooth shading's side channel)

Measured with `remodel --geo native --light --input coin-start run 900000000` (6149 frames; `make lighting-capture` runs the same), reModel's smooth-shading side channel (docs/hw/geometrizer.md, "Lighting side channel"):

- Self-check: of 18,038,390 polygon records, the flat luma recomputed from the side channel is the stream's for 18,038,385 (100.00%), one level off for 5 and further off for 0 (the SHARC rounds toward zero).
- specular on in 99.14% of object blocks, 0.00% of polygons lit with a specular term; normals computed from the vertices in 99.14% of blocks; zero (unlit) normals on 0.07% of polygons.
- Specular highlights as drawn (`make spec-stats`, the 21 frames of a `make lighting-capture`, 960x720, 2026-11-12): no textured polygon shown has a specular term (the lighting entries have none, as above).
- no models drawn from polygon RAM; no direct data.
- one light vector per frame; lighting entries not changed within a frame.
- True focal length over the window width: median 0.960 (range 0.960-1.921); the stream-only layer guesses 1.0.
- The list scan's block facts equal the native geometrizer's on 525,081 blocks, differ on 0.
