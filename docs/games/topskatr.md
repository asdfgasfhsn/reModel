# `topskatr`

## What's known about `topskatr`

Recorded as it's found, so nobody has to re-derive it. *Confirmed* means the
game's own code demonstrates it; *unverified* means it comes from external
documentation and hasn't been contradicted yet.

- The program ROM is `epr-19755a.15` and `epr-19756a.16`, joined as 16-bit
  halves of each 32-bit word (`.15` = low half), 1 MB at address `0x0`.
- Bootstrap tables at reset: PRCB `0xB0`, first IP `0x5C0`, interrupt table
  `0x5A2B0`, system-procedure table `0x1C0`.
- Reset copies a startup table from ROM `0x580` to `0xE00000`. It copies the
  PRCB to `0x511C40` and the interrupt table to `0x510000`, and `.data` from
  ROM `0xBF198` to RAM `0x500000–0x509FD8`. It then sends a reinitialise
  message (`0x93`) to the i960's control port `0xFF000010` and continues at
  `0x6F0`.
- *Confirmed:* the game programs the interrupt control register
  (`0xFF000004`) to `0x0F0E0D0C`, routing IRQ lines 0-3 to vectors 12-15.
  These are the four vectors with dedicated handlers in its interrupt table.
- *Confirmed:* boot uploads a 4,096-word (64-bit) program to the geometry
  coprocessor through `0x884000`, framed by bit 31 of `0x980000`.
- *Confirmed:* boot times a loop on bit 2 of `0x98000C` and only proceeds
  once that bit changes, consistent with a per-frame toggle.
- *Confirmed:* init enables IRQ mask `0x215` and programs timer 0
  (`0xF00000`) with 50,000 ticks (2 ms at 25 MHz).
- *Confirmed:* the game reads coprocessor results at 369 distinct sites.
  With the coprocessor stubbed, it fails after about 27 s on a divide by
  zero. With the MB86235 interpreter running the uploaded program, 300M
  instructions (36 s of game time) run with no faults and no unanswered
  result reads; the coprocessor executes 53M words.
- *Confirmed:* the coprocessor program is a command dispatcher (see
  `docs/hw/mb86235.md` section 6). Hardware notes with sources are in
  `docs/hw/`.
- *Confirmed:* every instruction the interpreter executed was already found
  by static analysis. The trace (`make run`, then `make analyze`) resolves
  the indirect jumps it observes.
- *Confirmed:* with a blank settings EEPROM the game writes 38 words of
  defaults, and accepts them on the next boot.
- *Confirmed:* the 2D tile layer shows the full boot sequence ("Downloading",
  "Initializing"), the publisher logo, the title screen and attract mode,
  whose 3D scenes are not rendered yet. Its attract text scrolls out and
  back in through the split plane (`docs/hw/tilemap.md`).
- *Confirmed:* coin 1 is IN0 bit 0 and start is IN0 bit 6 (active low). A
  scripted coin and start go through course select and skater select into
  a race: the HUD timer, checkpoints and hints all run.
- *Confirmed:* the other controls, each from the game's response to a
  scripted press (all active low):
  - select right is IN0 bit 4 and select left is IN0 bit 7: they move the
    course-select highlight;
  - jump front is IN0 bit 5 (the skater presses the nose) and jump tail is
    IN1 bit 0 (presses the tail into a manual);
  - analog channel 0 is steering: 0x10 carves right, 0xF0 carves left, 0x80
    is centred;
  - analog channel 1 is slide: 0xF0 makes the skater crouch into a slide,
    with sparks off the board.
- *Confirmed:* gameplay frames render in 3D with textures from the game's own
  geometrizer output, composited between low- and high-priority tiles. This
  confirms:
  - vertex linking, the 24-bit float stream, and the projection with the sync
    offsets (xoff 0, yoff 128);
  - texture headers and points from the `.27`/`.25` ROM, and the 4bpp 2x2-packed
    texel sheets in texture RAM;
  - the luma tables and the colour translation tables (section 5 of
    `docs/hw/geometrizer.md`).
- *Confirmed (from the ROMs; see `docs/hw/sound.md`):* the sound program
  (`mpr-19759.31`, word-swapped) is Sega's "CRX_drv" Ver3.2.8 and runs from
  `0x600000`. It routes SCSP MIDI-in to 68k IRQ level 3, timer A to level 2
  and timer B to level 1, and parses its input as MIDI with running status.
  The i960 sends 3-byte messages (`An`/`Bn` statuses) from a 278-entry
  table, plus `0xFF` at init, and never reads the UART data register. Music
  is `AE 10 nn` on channel 14, which the DSB2 firmware (working title "Ollie
  Over") captures, while it forwards all other bytes out of its own UART.
  The MPEG ROMs hold MPEG-1 Layer II, 32 kHz, 128 kbit/s stereo, stored
  unswapped.
- *Confirmed (running the sound program on `crates/sound`):* the driver
  streams every sound: the 68000 copies 512-byte chunks of 8-bit PCM from the
  sample ROM into 8 KB double buffers in sound RAM (`0x60000-0x6FFFF`) and
  refills by polling the slot monitor's CA bit. Voices play at 22.05 kHz,
  stereo sounds as hard-panned slot pairs. It loads a 108-step DSP program
  (reverb into the ring at `0x70000`). A `0xFF` byte soft-restarts the
  driver. Channel 14 is not special to it: the game's `AE` commands hit empty
  table entries, so they make no SCSP sound whether or not the link runs
  through the DSB2 first.
- *Confirmed (DSB2 firmware and MPEG ROMs; `docs/hw/sound.md` §3):* the
  DSB2 firmware holds a 12-track table at its ROM `0x8000`. Each entry gives
  a start and end byte address in the 16 MB MPEG space (always on frame
  boundaries), a volume per side and an optional loop point. `AE 10 nn` plays
  track nn (0 = stop), `AE 02 vv` sets the master volume, `AE 03 vv` fades out
  and then stops. reModel's decoder matches ffmpeg's to within 3e-6 of full
  scale on the game's MPEG data. Of the 12 tracks, 1-3, 5, 7-9 and 12 are
  played with looping armed (`64` before `74`) and 4, 6, 10 and 11 once.
  The firmware never reads decoder status beyond the always-clear busy bit,
  so it can't tell when a track ends. A replacement soundtrack of a
  different length therefore can't change the game's behaviour
  (`docs/mods.md`).
- *Confirmed (enhancement):* topskatr's courses hold up in wide views. At
  16:9 and 21:9 the game sends enough of the scene that the widened
  viewport fills to the edges with no visible pop-in across a race, so no
  game patch has been needed so far. The 3D HUD (speed gauge, time-limit
  ring) is in window 4, separate from the course (window 2) and sky
  (window 0).
- *Unverified:* the board map, IRQ controller semantics (`0xE80000`
  request/ack, `0xE80004` enable), timers and I/O chip layout in
  `crates/model2/src/bus.rs`.
- Regions the code reads and writes directly: work RAM `0x00500000`; a very
  hot port at `0x00800000` (about 4.9k accesses to 26 addresses, likely the
  geometry processor's input); `0x01C00000` and `0x01D00000` (probably I/O);
  `0x01000000` and `0x01800000` (probably tilemap and palette).
- *Input lag, measured (`make input-lag`: pictures with and without the
  control compared pixel by pixel, frames counted after the first that
  reads it; `docs/play.md`, "Run-ahead"):* curving (from frame 3507 of a
  coin-start run) shows in the first frame that reads it: input lag 0,
  nothing for run-ahead to hide. Curving the other way from frame 3907
  changed nothing on screen for 16 frames (the skater mid-trick,
  presumably).

