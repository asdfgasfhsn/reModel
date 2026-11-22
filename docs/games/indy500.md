# `indy500`

## What's known about `indy500`

Indianapolis 500 (Sega, 1995; the Twin cabinet, Rev A, the newer
revision), the thirteenth game and the fifth on Model 2B, and the
earliest 2B game here
before `vstriker`.

- *Confirmed (ROM, then the memory test):* program `epr-18598a.15/.16`;
  main data `.11/.12` at `0x2000000` and `.9/.10` at `0x2800000`; the 1
  MB pair `epr-18596.7/.8` in the second data bank at `0x6000000`, where
  it repeats through its 8 MB slot (the program reads it at
  `0x64F4D20`, `0x6488A90` and the like, 4 MB in, so the board decodes
  only the pair's address lines there; *unconfirmed* which address it
  really sits at). The test menu's MEMORY TEST shows every ROM (IC 7-16)
  and RAM GOOD in that layout. Polygons `.17/.21` then `.18/.22` and
  textures `.27/.25` draw correctly; four sample ROMs `.32-.35`. The copro
  data ROMs `.29/.30` are read by the copro SHARC (Model 2B above). The
  zip's other program, `.7/.8` and sound program chips belong to the
  clones `indy500d` (Deluxe) and `indy500to` (the older Twin revision),
  which aren't supported.
- *Confirmed (ROM):* both SHARC programs are its own. The geometrizer's
  (2,983 words of code) is an earlier version of the code the other 2B
  games share (its dispatch table and much of its code differ in
  address), and emits the same rasteriser stream: attract, menus and
  races draw correctly. The copro program (3,937 words) is the only one
  that reads its external bus beyond the FIFOs.
- *Confirmed (from the input test, then by scripted runs):* port
  `0x1C00002` bit 0 coin 1, 1 coin 2, 2 test, 3 service, 6 start (two
  coins a credit by default); port `0x1C00004` bit 0 VIEW1 (zoom in), 1
  VIEW2 (zoom out), 4 shift up, 5 shift down: a sequential shifter, not an
  H gate. Analog channel 0 is the wheel (lower is left), 1 the gas and 2
  the brake, both 0 released and `0xFF` fully down (a held `0xFF` gas
  takes the car to about 250 mph, `0xFF` brake slows it, and a wheel at
  `0x30` turns it left into the infield; the input test shows the raw
  values). The gas pedal confirms on the course and
  transmission screens.
- *Confirmed (from the code, then by running it):* the game writes a
  drive board command to I/O register 4 every frame (`0x01` at rest,
  `0x5x` in a race), and stores what it reads from registers 3 and 4
  without ever using it; the manifest gives it the wheel drive board
  model, which takes the commands. The link board registers at
  `0x1A14000` are written twice at boot and never read; the test menu's
  NETWORK TYPE defaults to STAND ALONE and the game runs stand-alone.
  Nothing else in its game assignments needs setting (no
  `backup_ram`).
- *Confirmed (by running it):* `make run GAME=indy500 INPUT=coin-start`
  boots (about 675 frames of black screen and checks), attracts with 3D
  and the course demos, coins up, picks the oval and AT, plays the
  rolling start and races with the gas held. `make sound` of that run
  averages -35 dBFS RMS (peak -16), with `dynabb97`'s and `hotd`'s, so
  the manifest adds no gain; no CRX_drv version string in its sound
  program (*unconfirmed* which driver it is). `make
  state-check GAME=indy500` round-trips, and a coin-start run of 600M
  instructions gives the state hash `4a1b197bf00149d6` (with an
  `eeprom.bin` from an earlier run; `c75add6c2ab149d6` while the copro's
  bank 2 was a RAM of its own, which the run never touched) recompiled
  and interpreted. `make
  lockstep GAME=indy500 RECOMP=1 INPUT=coin-start` runs 600M
  instructions identical (i960 and both SHARCs recompiled). Its
  geometrizer crate is shared with `fvipers` only, its copro crate its
  own (`docs/recompiler.md`).
- *Speed (by running it):* that run plays at 4.2x real time recompiled
  natively (the geometrizer on its thread, slowest frame 9.3 ms), and at
  1.1x interpreted (slowest frame 35 ms).
- *Confirmed (from the raster stream):* the dark shape under each car is
  an opaque polygon: header `60DA 0064 0808 0A40`, a translucent
  128×256 texture whose car silhouette is texel 0 (texel 15 around it
  transparent), luma table 100 (entry 0 is 1) and grey palette colour
  (22, 22, 22), so it draws at luma 0, close to black. Its attribute
  word (`0xA01`), luma word and header carry nothing the car and track
  polygons don't, and the checker bit is clear, so both renderers draw it
  solid, as sent. *Unconfirmed:* how it looks on the hardware (no shadow
  mechanism is visible in the data; `docs/hw/geometrizer.md` §5.4).
- *Input lag, measured (`make input-lag`: pictures with and without the
  control compared pixel by pixel, frames counted after the first that
  reads it; `docs/play.md`, "Run-ahead"):* the wheel (frames 3507 and 3907
  of a coin-start run) shows a frame after the first that reads it: input
  lag 1.
- *Depth buffer (2026-11-11; captured coin-start frames drawn with it off
  and on at 1440x1080 and 1920x1080 (`make depth-check`), compared with
  the build before the 2026-11-10 rules (`docs/enhancements.md`, "Depth
  buffer")):* 16 of 21 frames differ, by up to 0.3%. The 2026-11-10 rules
  helped: the lettering on the player car's sidepods, partly lost before,
  is whole (seen at frame 4200; about 1,000 pixels change in each race
  frame). Right with it on: the fence rail goes in front of the spectators
  who stand 25% of 1/z behind it.

## Lighting (smooth shading's side channel)

Measured with `remodel --geo native --light --input coin-start run 900000000` (6073 frames; `make lighting-capture` runs the same), reModel's smooth-shading side channel (docs/hw/geometrizer.md, "Lighting side channel"):

- Self-check: of 14,207,273 polygon records, the flat luma recomputed from the side channel is the stream's for 14,207,273 (100.00%), one level off for 0 and further off for 0.
- no specular lighting; normals computed from the vertices in 0.00% of blocks; zero (unlit) normals on 21.40% of polygons.
- models drawn from polygon RAM in 8,958 blocks (143,215 polygons, 2.50% of blocks); no direct data.
- the light vector changes within a frame in 229 of 6073 frames (up to 2 vectors a frame); lighting entries not changed within a frame.
- True focal length over the window width: median 0.402 (range 0.341-200.000); the stream-only layer guesses 1.0.
- The list scan's block facts equal the native geometrizer's on 358,611 blocks, differ on 0.

## Enhanced preset

What the player's Smooth modern picture style (the `enhanced` preset; `romset::Recommended` in the
manifest, `run_ahead_max` for run-ahead) turns on for `indy500`, and why:
Temporal at full size; the depth buffer on (the fence rail goes in front
of the spectators and the sidepod lettering is whole, above); no smooth
shading (the game lights its cars evenly: `docs/play.md`); smooth motion;
run-ahead 1.

## Idle loop

- *Confirmed (from the code, then by running it):* the main program
  waits for the next frame in the loop at `0x34914-0x34927` (the
  manifest's `idle_loops`; found as the hottest code of a coin-start
  run). It polls the video status register for vblank: `ld 0x98000c`,
  its frame bit (bit 2) compared with the one it read before the loop,
  branching back while they are equal, and `g0` added to on every pass (a
  count of passes nothing in the loop reads). It leaves the loop on the
  first read after vblank toggles the bit.
- *Confirmed (by running it):* with the idle skip (`docs/idle-skip.md`)
  a 600M-instruction coin-start run (`make bench`) fast-forwards 71.7%
  of its instructions through it, and the state hash is unchanged.

## Link play (survey)

- *Confirmed (by running it):* GAME ASSIGNMENTS has CABINET TYPE (TWIN), **NETWORK TYPE** (STAND ALONE, MASTER, SLAVE)
  and **CABINET ID**. Found by driving the test menu with scripted input (`test@1300-1330` held to open it, `service@A-B/20` to step the cursor, `test@F` to select or change a value) and diffing the `eeprom.bin` of runs that changed the value, the menu left through EXIT so the game saved it. The EEPROM record is `motoraid`'s `S32A` kind with the same offsets (NETWORK TYPE byte
  `0x1E`, copy `0x42`; CABINET ID byte `0x1F`, copy `0x43`).
- *Confirmed (by running it):* NETWORK TYPE MASTER with a board fitted gives `srallyc`'s start, then a read of
  `0x1A14002` every frame. See `docs/hw/link-board.md` section 14.
- *Confirmed (by running it):* in `link-run` (the hub as it is; per-node EEPROM images saved by the game) two machines show "CHECKING NETWORK NOW Ver.3.01", "THIS IS MASTER CONTROLLER" / "THIS IS SLAVE MACHINE", Node ID, Total Nodes and Cabinet ID, then race (POSITION 1/2 and 2/2). With both at CABINET ID 1 the game shows NETWORK ERROR and restarts the check forever. Blocks `0x1C0` at `0x2000`, received from `0x21C0`. `docs/hw/link-board.md` section 14.5.
- *Built:* `romset::LinkPlay` for `indy500` (`crates/romset/src/games/link.rs`): the `S32A` record's checksum was read out of the game's
  own code (`docs/hw/eeprom-records.md`), so a node's image has NETWORK TYPE 1 (master) or 2 (slave) and CABINET ID 0 or 1 in both copies of the
  record and a good checksum, which the game accepts (the nodes' final EEPROMs keep the roles and the checksum; `make link-record-test`). Linked
  race in `link-run` (the check screens "THIS IS MASTER CONTROLLER" / "THIS IS SLAVE MACHINE", then `1/2` and `2/2` with the other car in view),
  split-screen (`make play GAME=indy500 PLAYERS=2`) and over the network; `make link-net-test GAMES=indy500` and `make link-free-play-test GAMES=indy500`.
  The CABINET IDs differ per node (the node's index); two at one ID show NETWORK ERROR.
- *Confirmed (by running it): FREE PLAY is the game's own COIN ASSIGNMENTS setting #27.* Test menu: COIN ASSIGNMENTS is the eighth row (eight
  service presses from EXIT), COIN/CREDIT SETTING starts at 12 (2 coins 1 credit), **15 presses of the test button reach 27, whose chutes read
  FREE PLAY** (26 is "1 coin 6 credits"). Leaving through EXIT (the game saves) and comparing with a run that did not change it: byte `0x10`
  0x0B -> 0x1A (the setting, 0-based), byte `0x11` 0x02 -> 0x01 and byte `0x15` 0x00 -> 0x01 (derived), the counter at `0x0C` raised by one, the
  checksum, all in both copies. In `romset::LinkPlay::free_play`; `--free-play` starts a race from a start press.

## Variants

`indy500d` (Deluxe) and `indy500to` (older Twin) are supported variants
(docs/variants.md): own program pair, `.7/.8` pair and sound program. Idle loops are at
0x345D4 and 0x2E1C4. *Confirmed (by running it):* `indy500to` is on its warning screen until
about frame 1500 and the ranking screen takes a start press only after that: the parent's
`coin-start` presses start at 1250, which credits the coins and then waits on PRESS START
BUTTON, so the variant has its own script (start at 1900), which reaches a race. *Confirmed:*
the written settings records: `indy500to`'s has the parent's layout and checksum (the copy
at 0x2C, role and cabinet bytes 0 at 0x1E/0x1F, `remodel link-settings` accepts it) with other
default values (byte 0x1C 1 where the parent's is 2); `indy500d`'s has another version
(0x11 0x07), the checksum over the parent's range fails and its bytes 0x1E/0x1F hold other
values.

- *Confirmed (by running it):* **link play and free play of both variants** (`games/link_variants.rs`).
  Both have the parent's test menu, with NETWORK TYPE (STAND ALONE, MASTER, SLAVE) and CABINET ID
  on GAME ASSIGNMENTS (`indy500to` adds ENGINE VOLUME and DEFAULT VIEW after CABINET ID) and the
  COIN ASSIGNMENTS page. The cursor wants presses well apart (about 100 frames), and a held test
  press repeats: every 3 or 4 frames of hold moved COIN/CREDIT SETTING two or three steps, which is
  probably why the parent's script did not end on #27 in `indy500to` (*unconfirmed*: that script was not kept); one
  frame of press, 100 frames apart, moves it by two (12 to 27 in 8 presses, `indy500to`; 17 to 27
  in 6, `indy500d`, whose default is #17, 3 coins 1 credit). FREE PLAY is #27 in both and changes
  the same bytes as the parent: 0x10 (to 0x1A), 0x11 (to 1, from 3 in `indy500d` and 2 in
  `indy500to`), 0x15 (to 1), the write counter and the checksums.
  - `indy500to`: the parent's layout (36-byte record, copy at 0x2C), NETWORK TYPE 0x1E and
    CABINET ID 0x1F (copies 0x42, 0x43) as the parent; MASTER and SLAVE are accepted.
  - `indy500d`: the record is **44 bytes** (the copy at 0x34, so the copies of the role bytes are
    at +44), the checksum the same routine and start value over 42 bytes, NETWORK TYPE at **0x28**
    and CABINET ID at **0x29** (the bytes 0x1E and 0x1F hold other settings, 18 and 17), format word
    0x0711. Found by changing each setting in the menu and diffing the saved EEPROM, and by trying the
    checksum over the lengths the image allows (its code was not read).
  - Races: `link-run` with 2 machines (1/2 and 2/2, the other car in view) and 4 (1/4 to 4/4) in
    both; `make link-free-play-test GAMES="indy500d indy500to"` and `make link-net-test` (hashes equal
    `link-run`'s, with loss and jitter, drops and delay changes); `make link-record-test`: the game's
    own save of NETWORK TYPE MASTER with CABINET ID 2, and of FREE PLAY, is what romset writes, byte
    for byte except the checksums and the write counter. With the default #17 (3 coins 1 credit)
    `indy500d` needs three coins a credit, so its races use free play or three coins.
