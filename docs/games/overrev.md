# `overrev`

## What's known about `overrev`

Sixth game, 2C, and the first not by Sega: Jaleco's, with its own sound
driver. A sit-down driving cabinet; no extra boards are needed.

- *Confirmed (from the code):* `overrev.zip` holds two program pairs.
  `epr-20124a.15`/`epr-20125a.16` is the 2C one: it carries the MB86235
  geometrizer and copro code the other 2C programs share (17 of 37 and 22
  of 128 32-byte windows of `topskatr`'s uploaded programs match, as for
  the other 2C sets), while `epr-19992b`/`19993b` (the 2B clone
  `overrevb`'s) matches none. Both have the same strings and "Sep 10 1997"
  build dates. Boot record: PRCB `0xB0`, first IP `0x690`, interrupt table
  `0x1690`, system-procedure table `0x1C0`.
- *Confirmed (from the code):* main data is `.11/.12` at `0x2000000` and
  `.9/.10` at `0x2800000`: of the 2,826 program words pointing into
  `0x2000000-0x2FFFFFF`, 92 land on filler (`0xCC`/`0xFF` runs) in that
  order and 434 the other way round. No copro data ROMs, no second data
  bank. *Unconfirmed:* textures `.27/.25` and polygons `.17/.21` follow
  `topskatr`'s order; the game draws correctly with them.
- *Confirmed (from the code):* every boot runs a network check ("CHECKING
  NETWORK NOW"), whatever the link setting, as a state machine copied to
  work RAM (from program `0x6D6A0`). It reads the communication board at
  `0x1A14000`: bit 0 set means no board. With no board and the link
  setting stand-alone (the default: the setting byte at `0x590D16` is 0,
  from the EEPROM) it then waits for bit 7 of `0x1A14002` to be set and
  goes on "STAND ALONE"; with no board and a linked setting it stops on
  "Nothing communication board!". With a board it writes 1 to `0x1A14000`,
  waits for bit 7 of `0x1A14002` to clear, and starts the link. reModel
  reads the board's registers as all ones (no board), which the game takes
  as a stand-alone cabinet; reading 0 (as before) left it waiting at "Link
  check." forever. *Unconfirmed:* that an empty slot reads as all ones on
  the real board (pull-ups); it's the reading the game's code expects.
- *Confirmed (by running it):* no drive board: `0x1C00006` is never read.
  The game writes `0x1C00008` every frame. *Unconfirmed:* what (lamps,
  presumably).
- *Confirmed (from the input test, then by scripted runs):* port
  `0x1C00002` bit 0 coin (one coin a credit by default), 2 test, 3
  service, 6 start; port `0x1C00004` bit 0 view (cycles chase, cockpit and
  bumper cameras), 4 shift up, 5 shift down (they change gear with the
  manual gearbox). VIEW2 is listed but unused ("---"). Analog channels: 0
  wheel (0x30 steers left, 0xD0 right; it also moves the menu selection),
  1 gas and 2 brake, both 0xFF released and 0x00 fully down (the input
  test shows the raw value). A gas press (release to down) confirms each
  menu: mode, car, gearbox (AT or MT, chosen with the wheel).
- *Confirmed (from the code, then by running it):* the sound program
  `epr-20002.31` is Jaleco's own 68000 driver, not CRX_drv; it needs
  nothing beyond what Sega's drivers use (`docs/hw/sound.md` §6). The
  i960 sends the same 3-byte `A0 xx yy` messages over the link, and one
  `0xFF` at init, which restarts the driver as it does CRX_drv. Music is
  streamed PCM from the sample ROMs like the effects; there is no music
  board.
- *Confirmed (by running it):* `make capture GAME=overrev` (coin-start:
  one coin, start, the gas pedal to confirm challenge cup, the first car
  and AT, then held down) races the first course until time runs out.
  `make sound GAME=overrev` plays the run's commands on all 32 slots,
  about -37 dBFS RMS through the race with the music and engine. `make
  state-check GAME=overrev` round-trips, `make lockstep GAME=overrev` runs
  300M instructions identical, and a coin-start run of 600M instructions
  gives the same state hash (`d41686f041683f4c`, with an `eeprom.bin` from
  an earlier run) recompiled and interpreted.
- *Unconfirmed:* steering, braking and the AI cars' behaviour were only
  checked as far as a scripted run shows; the car stays last when
  nothing steers it.
- *Input lag, measured (`make input-lag`: pictures with and without the
  control compared pixel by pixel, frames counted after the first that
  reads it; `docs/play.md`, "Run-ahead"):* the wheel (frames 3507 and 3907
  of a coin-start run) shows in the first frame that reads it: input lag
  0.
- *Depth buffer (2026-11-11; captured coin-start frames drawn with it off
  and on at 1440x1080 and 1920x1080 (`make depth-check`), compared with
  the build before the 2026-11-10 rules (`docs/enhancements.md`, "Depth
  buffer")):* 18 of 21 frames differ, by up to 0.46%. Right with it on:
  crossed tree cards intersect. Can't tell: glass walkways against the
  building faces (11-52% of 1/z, frame 5700). The 2026-11-10 tolerance put
  one banner back to the game's look: a sponsor banner 0.02% in front of
  the wall drawn after it is half covered, as with the buffer off (the
  build before showed it whole).

## Lighting (smooth shading's side channel)

Measured with `remodel --geo native --light --input coin-start run 900000000` (6077 frames; `make lighting-capture` runs the same), reModel's smooth-shading side channel (docs/hw/geometrizer.md, "Lighting side channel"):

- Self-check: of 12,002,343 polygon records, the flat luma recomputed from the side channel is the stream's for 12,002,343 (100.00%), one level off for 0 and further off for 0.
- no specular lighting; normals computed from the vertices in 0.00% of blocks; zero (unlit) normals on 22.54% of polygons.
- no models drawn from polygon RAM; no direct data.
- the light vector changes within a frame in 4207 of 6077 frames (up to 3 vectors a frame); lighting entries not changed within a frame.
- True focal length over the window width: median 1.442 (range 0.706-2.734); the stream-only layer guesses 1.0.
- The list scan's block facts equal the native geometrizer's on 388,760 blocks, differ on 0.

## Enhanced preset

What the player's Smooth modern picture style (the `enhanced` preset; `romset::Recommended` in the
manifest, `run_ahead_max` for run-ahead) turns on for `overrev`, and why:
Temporal at full size; the depth buffer on (the crossed tree cards
intersect, and the one banner traded off is half covered as with the buffer
off: above); no smooth shading (no pictures recorded either way); smooth
motion; run-ahead 0 (input lag 0).

## Idle loop

- *Confirmed (from the code, then by running it):* the main program
  waits for the next frame in the loop at `0x12C44-0x12C57` (the
  manifest's `idle_loops`; found as the hottest code of a coin-start
  run). It polls the video status register for vblank: `ld 0x98000c`,
  its frame bit (bit 2) compared with the one it read before the loop,
  branching back while they are equal, and `g0` added to on every pass (a
  count of passes nothing in the loop reads). It leaves the loop on the
  first read after vblank toggles the bit.
- *Confirmed (by running it):* with the idle skip (`docs/idle-skip.md`)
  a 600M-instruction coin-start run (`make bench`) fast-forwards 79.7%
  of its instructions through it, and the state hash is unchanged.

## Link play (survey)

- *Confirmed (by running it):* GAME ASSIGNMENTS MENU, FOUNDATIONAL SETTING has **LINK MAX** (NOT LINK, 2 LINKS, 3 LINKS,
  4 LINKS) and **LINK TYPE** (MASTER CarNo.1, SLAVE CarNo.2, CarNo.3, ...; the default is MASTER CarNo.1 with LINK MAX NOT LINK,
  so LINK MAX is what turns linking on). Found by driving the test menu with scripted input (`test@1300-1330` held to open it, `service@A-B/20` to step the cursor, `test@F` to select or change a value) and diffing the `eeprom.bin` of runs that changed the value, the menu left through EXIT so the game saved it. The EEPROM is the `S32A` kind (copies `0x34` apart at `0x08` and
  `0x3C`): LINK MAX byte `0x36` (copy `0x6A`; 0 to 3), LINK TYPE byte `0x35` (copy `0x69`; 0, 1, 2 seen). The checksum algorithm
  was not determined.
- *Confirmed (by running it):* LINK MAX 2 LINKS with a board fitted: the start check at boot and after the menu is
  0 to `0x1A14000`/`0x1A14002`, a read of `0x1A14000`, 1 to `0x1A14000` (no write to `0x1A14002` as the default
  CarNo.1 master), then a read of `0x1A14002` every frame. Jaleco's set has no link board ROM. See `docs/hw/link-board.md` section 14.
- *Confirmed (by running it):* in `link-run` with the game's own images (LINK MAX 2 LINKS, MASTER CarNo.1 / SLAVE CarNo.2) and the `0x380` layout (as `sgt24h`) two machines race in one field; the hub warned that no node wrote 1 to `0x1A14002` as master (the game does not) and went on. `docs/hw/link-board.md` section 14.5.
