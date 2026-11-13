# `stcc`

## What's known about `stcc`

Third 2C game: the first with a DSB1 music board, a wheel and a gearbox.

- *Confirmed:* the set's three program pairs (`epr-19272a/19273a`,
  `epr-19272b/19273b`, `epr-19570/19571`) are revisions of the same program;
  the manifest uses `epr-19570.15`/`epr-19571.16` (`.15` low half), the one
  with the most added. Boot record: PRCB `0xB0`, first IP `0xD00`, interrupt
  table `0x689E0`, system-procedure table `0x1C0`.
- *Confirmed:* ROM banks are 8 MB slots and a pair smaller than its slot
  repeats through it. Main data `.11/.12` (8 MB) is at `0x2000000` and the
  1 MB `.9/.10` pair at `0x2800000`, but the game reads it at `0x2C4B210`
  ("Not CGM Data" until it mirrors; a CGM header sits at `0x4B210` of the
  pair). The 1 MB second polygon pair `.18/.22` is likewise read 4 MB into
  the second polygon slot (a model in the same format as the first pair's
  starts there), and the 1 MB second texture pair `.28/.26` at the start of
  the second texture slot. Byte order within each pair is confirmed by
  strings and headers, except `.28/.26` (low half `.28` by analogy with
  `.27/.25`; unconfirmed). Copro data `.29/.30` is 4 MB, repeated through
  its 8 MB window (unconfirmed whether the program reads past 4 MB).
- *Confirmed:* `epr-19274.31` is CRX_drv Ver3.2.5 (96 8/1); `make sound
  GAME=stcc` plays a run's commands, and the 68000 never touches anything
  outside the sound board.
- *Confirmed (from the code and by running it):* the geometrizer list uses
  **jump words**: the game builds each frame's list in pieces (buffer RAM
  `0x00000`/`0x10000` banks, pieces at `+0x4000`, `+0xC000`, ...) and links
  them with command-port writes with bit 31 set (`0x80004000`: continue at
  `0x4000`). The geometrizer program has no jump handling, so the list DMA
  must follow them. Data words with bit 31 set (negative floats) are not
  jumps, and the game also overwrites old jump words directly (the copro
  stores matrices into the list), so reModel follows only jump words written
  through the command port and not stored over by the copro since
  (`fvipers` patches jumps in with the i960, which they survive; Model 2B
  above). *How the hardware tells them apart is unconfirmed.* Without this the
  geometrizer stops at its command-check error on the first linked list
  and no 3D is drawn.
- *Confirmed (from the code):* the wheel board is commanded like
  `skisuprg`'s drive board (reply read from `0x1C00006`, then a command
  written to `0x1C00008`, every frame), but the game only stores the reply
  and never reads it back: no handshake, no fault check, and it runs the
  same with nothing attached. Commands seen: `0x00` through boot, `0x98`,
  `0x30` (attract, menus), `0x34` (select screens), `0x38` (racing), and
  `0x5x`/`0x6x` while racing with steering and collisions. *Unconfirmed:*
  what each does; presumably force effects. The model (`Drive::Wheel`)
  replies `0xFF` and keeps the last commands.
- *Confirmed (from the code):* the link is decided by the settings: the
  network check runs only if the "LINK TYPE" setting isn't stand-alone
  (the default). It then reads the link board at `0x1A14000`, bit 0 set
  meaning "NO COMMUNICATION BOARD". Nothing needs answering for a single
  cabinet.
- *Confirmed (from the code):* the DSB1 needs nothing for the game to run.
  The i960 never reads the sound link's receive register, and its link
  bytes reach the sound board as in `topskatr`. The Z80 program
  (`epr-19275.2s`, "MPEG SOUND DRIVER Z80 VERSION 0.26 - 96/08/29") takes
  each byte in its interrupt handler (mode 1, `0x38`) from UART data port
  `0xF0`, waits for bit 0 of status port `0xF1` (transmitter ready),
  echoes the byte out of `0xF0`, and stores it in a 4 KB ring at
  `0x8000-0x8FFF` (write pointer at `0x9002`). So the DSB1 takes the same
  link bytes as the sound board and needs no reply path.
- *Confirmed (from the Z80 program, then by running it):* the DSB1 plays
  the music (`docs/hw/dsb1.md`). ROM `0000-7FFF` (8 KB used), RAM
  `8000-FFFF`, no banking. The decoder sits on ports `E0` (play mode: 0
  stop, 2 play and continue into the next queued segment, 1 play to the
  end), `E2-E4` start (read back: current address), `E5-E7` end (the
  last byte; confirmed by all 23 segments ending one frame short of
  end + 1), `E8` attenuation (`00` loud to `7F` silent). Channel 15
  commands: `AF 6F n` plays track `n` (0 stops), `AF 01 v` scales later
  volumes, `AF 02 v` sets the volume, `AF 03 r` fades out. The 17 tracks
  are lists of segments (start and end in the 8 MB MPEG space) that the
  program queues one ahead while the decoder plays. A coin-start session
  plays track 1, then track 11 looping until the race's fade-out.
  *Unconfirmed:* the Z80 clock (4 MHz used; it sets only the fade speed),
  the attenuation curve (linear used), and what ports `E9`, `EB` and `E8`
  values above `7F` set (ignored).
- *Confirmed (from the input test, then by scripted runs):* port
  `0x1C00002` bits 0/1 coins (one coin a credit by default), 2 test, 3
  service, 6 start; port `0x1C00004` bit 0 view 1 (the camera changes),
  bit 1 view 2 (not tried), bits 4/5 shift up/down; analog channel 0 the
  wheel (0x30 steers left, 0xD0 right), 1 gas and 2 brake (0x00 released,
  0xFF down). The gas pedal also confirms on the menus and the wheel moves
  the selection. Holding the brake while confirming the car picks the
  manual gearbox (MT), which the shift buttons then step through (1-4 and
  back in a scripted run).
- *Confirmed (by running it):* `make capture GAME=stcc` (coin-start)
  reaches the qualifying lap, where the car waits for gas; with gas held
  it laps, and the attract demos race in 3D. `make state-check GAME=stcc`
  round-trips, `make lockstep GAME=stcc` runs 300M instructions identical,
  and a coin-start run of 600M instructions gives the same state hash
  (`b42ea11201ac75c4`; `bbde4ebfb8d918d8` before the timers' counts were read a word higher, 2026-11-13, `docs/hw/peripherals.md`) recompiled and interpreted.
- *Unconfirmed:* `epr-18643a.7` (also in `sgt24h` and `von`) is Z80 code
  whose header names Sega part 837-10074; it belongs to another cabinet
  board, most likely the link board (see `docs/games/sgt24h.md`).
- *Input lag, measured (`make input-lag`: pictures with and without the
  control compared pixel by pixel, frames counted after the first that
  reads it; `docs/play.md`, "Run-ahead"):* steering (frames 4507 and 4907,
  the gas held from 4100 of a coin-start run) shows a frame after the
  first that reads it: input lag 1.
- *Depth buffer (2026-11-11; captured coin-start frames drawn with it off
  and on at 1440x1080 and 1920x1080 (`make depth-check`), compared with
  the build before the 2026-11-10 rules (`docs/enhancements.md`, "Depth
  buffer")):* 12 of 21 frames differ, by up to 0.08% of the pixels. Right
  with it on: the roof struts go behind a sign's pillar
  (they are 73-84% of 1/z behind it; the game draws them over it). Wrong
  with it on: the pennants in the grandstands stand in the crowd's flat
  card, 7.5% of 1/z behind it at the pixel probed, and lose their lower
  halves (the game shows them whole); no generic fix. The 2026-11-10 rules
  changed only edge pixels here.

## Lighting (smooth shading's side channel)

Measured with `remodel --geo native --light --input coin-start run 900000000` (6209 frames; `make lighting-capture` runs the same), reModel's smooth-shading side channel (docs/hw/geometrizer.md, "Lighting side channel"):

- Self-check: of 10,683,455 polygon records, the flat luma recomputed from the side channel is the stream's for 10,683,455 (100.00%), one level off for 0 and further off for 0.
- specular on in 100.00% of object blocks, 39.22% of polygons lit with a specular term; normals computed from the vertices in 0.00% of blocks; zero (unlit) normals on 10.59% of polygons.
- Specular highlights as drawn (`make spec-stats`, the 21 frames of a `make lighting-capture`, 960x720, 2026-11-12): 43% of the textured polygons shown have a specular term (scales 255, 160, 96, 64, 32, squared twice), 62% of the textured pixels; it brightens 4.6% of the textured pixels by a level or more and 3.8% by 4 or more, 28 levels on average where it shows, up to 103. Nearly all of it is the cars' bodywork (the car ahead of the camera, the car select's car); the track and stands have the term but it adds nothing.
- no models drawn from polygon RAM; no direct data.
- the light vector changes within a frame in 2633 of 6209 frames (up to 2 vectors a frame); lighting entries not changed within a frame.
- True focal length over the window width: median 0.806 (range 0.484-6.667); the stream-only layer guesses 1.0.
- The list scan's block facts equal the native geometrizer's on 682,939 blocks, differ on 0.

## Enhanced preset

What the player's Enhanced picture preset (`romset::Recommended` in the
manifest, `run_ahead_max` for run-ahead) turns on for `stcc`, and why:
Temporal at full size; smooth shading on (the cars are clearly better in
the review pictures); no depth buffer (it cuts the grandstand pennants in
half, above); smooth motion; run-ahead 1.

## Idle loop

- *Confirmed (from the code, then by running it):* the main program
  waits for the next frame in the loop at `0x763D4-0x763EB` (the
  manifest's `idle_loops`; found as the hottest code of a coin-start
  run). It polls the video status register for vblank: `ld 0x98000c`,
  its frame bit (bit 2) compared with the one it read before the loop,
  branching back while they are equal, and `r4` added to on every pass (a
  count of passes nothing in the loop reads). It leaves the loop on the
  first read after vblank toggles the bit.
- *Confirmed (by running it):* with the idle skip (`docs/idle-skip.md`)
  a 600M-instruction coin-start run (`make bench`) fast-forwards 74.6%
  of its instructions through it, and the state hash is unchanged.
