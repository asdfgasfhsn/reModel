# `skisuprg`

## What's known about `skisuprg`

Second 2C game, added to find what is still `topskatr`-specific.

- *Confirmed:* the program is `epr-19489.15`/`epr-19490.16` at `0x0` (`.15`
  = low half) and `epr-19551.13`/`epr-19552.14` at `0x100000` (`.13` low;
  the code loads a table at `0x195900` that only makes sense in that order).
  Boot record: PRCB `0xB0`, first IP `0x5F0`, interrupt table `0x46E20`,
  system-procedure table `0x1C0`. Nothing executes above `0x100000`.
- *Confirmed:* main data is `.11/.12` at `0x2000000` and `.9/.10` at
  `0x2800000`, as in `topskatr`: a table at `0x2E783E0` holds pointers into
  `0x2E77xxx`. Both MB86235 programs are uploaded from the program ROM
  (copro from `0x8D480`, geometrizer from `0x90708`).
- *Confirmed:* the polygon ROM is 16 MB (`.17/.21` then `.18/.22`), four
  times `topskatr`'s, so the geometrizer now sizes its ROM window from the
  ROM. Copro data (`.29/.30`) fills the whole 8 MB window.
- *Confirmed:* init enables IRQ mask `0x19` (`topskatr`: `0x215`).
- *Confirmed:* `epr-19491.31` is CRX_drv Ver3.2.8 (96 10/28); the sound board
  plays the attract's commands (`make sound GAME=skisuprg`).
- *Confirmed (from the code):* the game needs its drive board. Its vblank
  handler reads the board's reply from the I/O byte at `0x1C00006` and then
  writes the frame's command to `0x1C00008`. At boot it sends `0x00`, waits
  for the reply `0xCE`, sends a byte from its settings (the swing channel's
  calibrated centre, `0x80` by default), waits for `0xFF`, then sends
  `0x00`; after 479 frames it gives up and shows "DRIVE BOARD TROUBLE
  CODE: xx" with the last reply in hex (`FF` with nothing attached).
  Replies `0xB0`-`0xB7` are faults; `0xB8`/`0xB9` start a 120-frame
  warning. In the test menu, after command `0xDC` each nibble of the reply
  shows a switch (`0x0`/`0xF`: off/on), and after `0xA3`/`0xA4` bit 4 is a
  status the drive test shows and follows.
- *Confirmed (from the code):* commands seen: during a run `0x10`-`0x1F`
  (picked from tables by the ski inputs), `0x40` and `0xD1` in turn, one a
  frame; `0x80` at rest; `0xC2`/`0xC3` around test mode and attract;
  `0xA0`-`0xA6` and `0xD0` in the drive tests. *Unconfirmed:* what each
  does on the board, and what a real board replies to them.
- *Confirmed (by running it):* a drive board that replies `0xCE` while told
  `0x00` and `0xFF` to anything else (`crates/model2/src/drive.rs`, chosen
  by the manifest's `drive_board`) passes the boot check; the game runs its
  attract, takes coins and plays a full run (`make capture GAME=skisuprg`
  reaches the results screen).
- *Confirmed (by running it):* `make recomp GAME=skisuprg` recompiles its
  i960 code (keyed by its program hash, beside `topskatr`'s) and both
  MB86235 programs; `make lockstep GAME=skisuprg` runs 300M instructions
  identical, and a coin-start run reaches the same state hash on both.
- *Confirmed (from the code):* the boot code calls the per-frame input
  read, which reads all eight analog channels and uses channels 0-2, each
  through a calibration table built from its settings (min, centre, max per
  channel; defaults `0x00`/`0x80`/`0xFF`). Digital inputs are read at
  `0x1C00002` (twice: with I/O register 0 bit 0 clear and set),
  `0x1C00004`, `0x1C00006` (the drive board's reply) and `0x1C0000C` (the
  last inverted).
- *Confirmed (by scripted runs):* coins are port `0x1C00002` bits 0 and 1
  as in `topskatr`, and one credit is two coins by default. The three
  panel buttons (blue, red, green) are bits 6, 7 and 4 of that port: on
  the select screens they pick the left, middle and right choice, and blue
  also starts a game. Channels 0 and 1 both turn the skier, in opposite
  senses (right: channel 0 up, channel 1 down); the game's demo drives
  them with equal calibrated values. reModel names them `edge` and
  `swing`; *unconfirmed* which ski angle each measures.
- *Unconfirmed:* channel 2 (`push`): the skier's physics adds its offset
  from `0x80` to the speed outside `0x70`-`0x8F`, the demo sets it at the
  start, but holding it had no visible effect in a run. Port `0x1C00002`
  bit 5 and `0x1C00004` bit 0 step a selection right and left, and
  `0x1C00004` bit 4 confirms, in a mode of the select screens the runs
  didn't reach (named `selright`, `selleft`, `decide`); which switches they
  are on the cabinet is unknown. The test menu uses port bits 2 and 3
  (test, service) and bits 6 and 7 of the second `0x1C00002` read.
- *Input lag, measured (`make input-lag`: pictures with and without the
  control compared pixel by pixel, frames counted after the first that
  reads it; `docs/play.md`, "Run-ahead"):* swinging (frames 3507 and 3907
  of a coin-start run) shows in the first frame that reads it: input lag
  0.

## Lighting (smooth shading's side channel)

Measured with `remodel --geo native --light --input coin-start run 900000000` (6166 frames; `make lighting-capture` runs the same), reModel's smooth-shading side channel (docs/hw/geometrizer.md, "Lighting side channel"):

- Self-check: of 18,711,622 polygon records, the flat luma recomputed from the side channel is the stream's for 18,711,622 (100.00%), one level off for 0 and further off for 0.
- specular on in 6.98% of object blocks, 0.00% of polygons lit with a specular term; normals computed from the vertices in 6.98% of blocks; zero (unlit) normals on 12.89% of polygons.
- models drawn from polygon RAM in 38,656 blocks (444,557 polygons, 7.16% of blocks); no direct data.
- one light vector per frame; lighting entries not changed within a frame.
- True focal length over the window width: median 0.500 (range 0.500-248.000); the stream-only layer guesses 1.0.
- The list scan's block facts equal the native geometrizer's on 539,604 blocks, differ on 0.
