# `hotd`

## What's known about `hotd`

The House of the Dead, a two-player light-gun game on 2C. The set is the
Japanese release (the boot warning says "used only in Japan"); the title
screen says ©SEGA 1996.

- *Confirmed (from the game's memory test):* program `.15/.16` at `0x0`
  (`.15` low) and `.13/.14` at `0x100000`; main data `.11/.12` at
  `0x2000000` and `.9/.10` at `0x2800000`; the second data bank, `.7/.8`
  at `0x6000000` and the 1 MB pair `.5/.6` at `0x6800000` (as in
  `segawski`). The memory test checks IC15, 16, 13, 14, 11, 12, 9, 10, 7,
  8, 5 and 6 in that order and passes them all. Its RAM IC54 and IC55
  are the 64 KB RAMs the board model now maps (see `docs/hw/model2c.md`; it reported them BAD before they were mapped).
- *Confirmed (from the code):* both MB86235 programs come from `.9/.10`,
  not the program ROM: the boot uploads the geometrizer's from
  `0x2F40000` (code at `0xA1C34`) and the copro's from `0x2F48000`
  (`0xA36B8`), 4 MB into that pair. Nothing about the boot path is
  special beyond that; the shared upload code just reads a different
  address.
- *Confirmed (by running it):* polygons are four pairs (`.17/.21`,
  `.18/.22`, `.19/.23`, `.20/.24`), textures two (`.27/.25`, `.28/.26`),
  copro data the 512 KB `.29/.30`, samples `.32/.34`; `epr-19720.31` is
  the sound program (CRX_drv 3.2.8 per `docs/roms.md`). The game draws
  correctly and `make sound GAME=hotd` plays a run's commands on the SCSP.
  No music board.
- *Confirmed (from the code):* boot waits on a **gun interface** behind
  I/O registers 9-13 (the upper byte lanes of `0x1C00010`-`0x1C0001C`):
  the game writes a parameter to `0x1C00014`, a command to `0x1C00012`,
  waits for `0x1C0001A` bits 2 and 3, then reads the reply from
  `0x1C00018` (and reads `0x1C00016`, unused). Commands below `0x80` write
  a device register (low bits), `0x80` and up read one back; register 0
  selects a counter, command `0x87` reads it. At boot (`0xA3750`) the game
  sets registers 1 and 8, toggles bit 0 of register 1, then streams 5 KB
  from program ROM `0xA3A00-0xA4DFF` through command `0x07`: a
  configuration image, presumably for programmable logic that times the
  guns (*unconfirmed* what the part is). Without a device answering, boot
  spins at `0xA3728` forever. reModel models only this (`model2::gun`),
  for games with guns in their manifest.
- *Confirmed (from the code, then the GUN SETTING test screen):* each frame
  `0xA38C0` reads counters 0-7 as four 10-bit values (a low byte, then two
  bits from the next counter): player 1's **Y** in 0/1 and **X** in 2/3,
  player 2's Y in 4/5 and X in 6/7 (the GUN MARK CHECK screen shows them as
  IP1X/IP1Y and IP2X/IP2Y). Counter 8 holds one bit a player (bit 0
  player 1, bit 1 player 2), read inverted into `0x51EF08` and `0x51EF24`:
  **1 means the gun sees no screen**. Reloading needs that bit: with the
  coordinates zeroed but the bit clear, an off-screen shot doesn't reload
  and the game shows its "shoot outside the screen" hint. *Unconfirmed:*
  what the real board does with the counters while a gun sees nothing
  (reModel reports 0).
- *Confirmed (from the test screens):* the game calibrates the guns
  itself (GUN SETTING: adjustment, default adjustment, manual
  adjustment). The defaults in a fresh EEPROM are X 173-596 (player 2:
  163-596) and Y 87-380, which the game maps to points about 32 pixels in
  from the view's edges. Measured on GUN MARK CHECK, the raw ranges that
  put the mark on the aimed pixel (to within a pixel) are X 141-629
  (player 2: 131-630) and Y 56-410 over the whole 496x384 view; these are
  in the manifest. A cabinet recalibrated in the test menu would want
  other values.
- *Confirmed (from the input test, then by scripted runs):* port
  `0x1C00002` bits 0/1 coin chutes, 2 test, 3 service, 4 START1, 5
  START2; port `0x1C00004` bit 0 TRIGGER1, bit 1 TRIGGER2. Nothing else
  on the ports or analog channels is used (the shared input routine reads
  the analog channels but the game ignores them). Two coins a credit by
  default ("2 CREDITS TO START").
- *Confirmed (by running it):* the coin-start script plays: it aims at
  the middle and either side in turn, fires every 20 frames and reloads
  off screen every seventh shot; shots land where aimed (impact sparks at
  the aim point, zombies hit), and the run gets through the first two
  areas before the game over. `make state-check GAME=hotd` round-trips,
  `make lockstep GAME=hotd` runs 300M instructions identical, and a
  coin-start run of 600M instructions gives the same state hash
  (`38c08a3e3591af45`, with an `eeprom.bin` from an earlier run; `0473295c9891af45` before the timers' counts were read a word higher, 2026-11-13, `docs/hw/peripherals.md`)
  recompiled and interpreted.
- Light gun input is generic (`romset::Gun`): a gun is a position in the
  view (or off screen) plus a trigger button; the manifest says where the
  board reads it (`GunSource::Counters` for this interface), its
  calibration and how it reloads. Players, the browser and `--input`
  (`gun1=x:y@from-to`, `gun1=off@...`) produce positions; see
  `docs/play.md` and `docs/web.md`.
- *Input lag, measured (`make input-lag`: pictures with and without the
  control compared pixel by pixel, frames counted after the first that
  reads it; `docs/play.md`, "Run-ahead"):* a shot (the trigger from frame
  3510 of a coin-start run) shows in the first frame that reads it, in the
  2D layers only (input lag 0); from frame 3907 of a run whose trigger
  stops at 3400, a frame later (1). From 3910 and 4510 nothing changed for
  16 frames (between scenes).

## Lighting (smooth shading's side channel)

Measured with `remodel --geo native --light --input coin-start run 900000000` (6129 frames; `make lighting-capture` runs the same), reModel's smooth-shading side channel (docs/hw/geometrizer.md, "Lighting side channel"):

- Self-check: of 9,359,810 polygon records, the flat luma recomputed from the side channel is the stream's for 9,359,810 (100.00%), one level off for 0 and further off for 0.
- no specular lighting; normals computed from the vertices in 100.00% of blocks; zero (unlit) normals on 0.00% of polygons.
- models drawn from polygon RAM in 6,322 blocks (95,438 polygons, 1.58% of blocks); no direct data.
- the light vector changes within a frame in 3157 of 6129 frames (up to 3 vectors a frame); lighting entries not changed within a frame.
- True focal length over the window width: median 1.000 (range 1.000-1.000); the stream-only layer guesses 1.0.
- The list scan's block facts equal the native geometrizer's on 399,028 blocks, differ on 0.

## Enhanced preset

What the player's Enhanced picture preset (`romset::Recommended` in the
manifest, `run_ahead_max` for run-ahead) turns on for `hotd`, and why:
Temporal at full size; no depth buffer or smooth shading (nothing
recorded as better); smooth motion; run-ahead 0 (input lag 0, once 1).
