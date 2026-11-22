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
- *Confirmed (by looking at the frames, 2026-11-10):* the attract
  montage switches shots every 27 frames (frames 1037, 1064 and 1091 of a
  coin-start run: each a different shot from the frame before), and smooth
  motion's cut test flags each as a camera cut, rightly; temporal
  anti-aliasing starts its history again there.
- *Confirmed (by looking at the frames):* a coin-start run with the
  default input (no steering) never reaches the FINISH gantry: the race
  ends TIME UP at about frame 9000. Palette 0x1009 toggles every frame
  during the race (from at least frame 4600 to 7600), but the gantry isn't
  on screen. It is on screen in the attract demo that follows (stadium
  course), frames about 19590-19607 of the same run (a camera cut at
  19608), with 0x1009 alternating 0xD3FF and 0x8000: `remodel --game
  topskatr --geo native --capture-seq DIR --seq 19540,100 --input
  coin-start run 4200000000 0`.
- *Confirmed (temporal anti-aliasing, by looking at the frames):* on those
  frames the sign flickers with temporal on exactly as the game draws it:
  at TAA and 67% the word shows on even frames and the empty board on odd
  ones, like the plain frames, with no averaging to a mid colour and no
  ghost of the letters on the empty frames (`docs/enhancements.md`). The
  waving chequered flags beside it, which have no motion data, look soft.
- *Confirmed (probing the frames, 2026-11-11):* the large solid-black floor
  areas in the attract demo's street course (frames about 4400-4490 of a
  run with no input, about 75 s; also around 4240-4300) are texel 0 of a
  floor texture the game draws, not a lost polygon. The floor there is one
  big near-clipped polygon, header `5409 0000 5f10 3140`: a 64x64 texture
  at (512, 896) in bank 1 that uses only texels 0-3, mostly 2, with a
  jagged vertical stripe of 0 (a crack or stain), luma table 0 (a ramp,
  0 at index 0), palette colour (31, 31, 26). Texel 0 gives level 0, and
  the game's colour tables map level 0 of every ramp to black (level 1 is
  already 76 of 255 for component 31), so the stripe is black as sent;
  with the camera low on the road the polygon is magnified and the stripe
  fills half the screen. Software and GPU renderers, chip and native
  geometrizer, threaded and single-threaded runs all draw the same frame,
  so nothing in the path loses or misreads data. Rewriting that texture's
  texel 0 as 1 in a capture turns every black pixel of frame 4440 grey.
  Black blobs in the stadium montage (frames about 1050-1530) are the
  skater's shadow instead: a translucent texture of texels 14/15 in
  palette black, drawn solid like the other shadows in
  `docs/hw/geometrizer.md` 5.4.
- *Unconfirmed:* whether the hardware draws that stripe this dark. The
  polygon sets microtexture (h0 bit 12, the microtexture at (0, 128) in
  bank 0, texels 0-7: the asphalt slot) and reModel doesn't draw
  microtextures. What the games send for level of detail, and the
  candidate blends, are in `docs/hw/texture-lod.md`: with the likeliest
  one (a linear blend of the two filtered texels below level 0) the
  magnified stripe becomes asphalt, not solid black. Mipmaps don't come
  into it: topskatr uploads none. That page lists the footage that would
  settle it (this scene, about 75 s after power-on).

## Pokes (2026-11-23)

Found by running the game with `remodel ram search` (docs/uplift-studio.md,
"Pokes"); listed in `crates/romset/src/games/pokes/topskatr.txt`, applied only
when a tour says `poke=NAME@from-to`.

- *Confirmed (by running it):* **the course's time limit** is a 16-bit word
  at `0x5830B0`, counting frames (60 a second). In a coin start that picks
  the first skater (`from=coin-start@1300,selright@1330-1336`) the course is
  running at frame 5000 with "20 LIMIT" on screen and the word reads `0x4B3`
  (1203 frames, 20.05 s); at 5300 the screen says 15 and the word reads
  `0x387` (903); at 5420 it reads `0x30F` (783). Searching those three
  snapshots for a word that fell by exactly 300 and then 120 leaves two
  candidates in the two work RAMs, `0x5830B0` and a signed word at
  `0x5CDDDC` that falls with it; the first is the one the screen follows.
  Writing `0x0E10` (60 s) to it every frame from frame 3300 keeps the limit
  on screen at 59 at frame 9500 (1'45" total time), past where the same run
  ended with TIME UP at about frame 9000. The run still ended: by frame 16000 the
  game was back in the attract loop, with the skater given no steering (why
  was not looked into). The
  elapsed total time is a separate word at `0x509E70` counting frames
  (`0x73F` at frame 5000, 30"91 on screen).
- *Unconfirmed:* whether the address holds for the other skaters and
  courses (the search used one skater on the first course), and what the
  other word, `0x5CDDDC`, is.

## Pack gates on the Top Skater showcase pack (2026-11-22)

`remodel pack check` on the showcase's last pack (6,286 files; the gates are
in docs/uplift-studio.md, "Gates"; the evidence and its caveats in
docs/uplift/quality-gates.md, E2). Numbers only; no image is kept here.

- Asset gates, 2.2 s for the lot: `file` passes all; `alpha` is an error on 7
  of 303 textures (2.3%) and a warning on 552 of 5,970 tiles; `alpha.soft`
  warns on 34; `seam` on 1 of 219 checked; `noise` on 167 (2.7%); `changed`
  on none (the lowest texture still differs over 11.5% of its texels).
- Pack gates: `lint` 0 errors and 0 warnings; `budget` 263.1 MB mipmapped, over
  the browser's 192 MB; 227 of 6,500 selected rows have no approved image.
- `coverage` of pixels that differ from the original, per scene at the native
  view: coin-up 0.0%, course-select 0.0%, skater-select 6.8%, race-go 88.0%,
  race-course 90.9%. *Unconfirmed:* the two 0.0% scenes are 2D screens; whether
  the pack draws nothing there or `scene render` does not draw the 2D
  replacements was not checked.
- `flicker`: a 1,725-frame attract run with the pack (`make pack-log`)
  gives 983 changes and 10,304 pixels involved per 100 frames.

## Lighting (smooth shading's side channel)

Measured with `remodel --geo native --light --input coin-start run 900000000` (6209 frames; `make lighting-capture` runs the same), reModel's smooth-shading side channel (docs/hw/geometrizer.md, "Lighting side channel"):

- Self-check: of 14,125,713 polygon records, the flat luma recomputed from the side channel is the stream's for 14,125,713 (100.00%), one level off for 0 and further off for 0.
- specular on in 100.00% of object blocks, 67.06% of polygons lit with a specular term; normals computed from the vertices in 100.00% of blocks; zero (unlit) normals on 0.00% of polygons.
- Specular highlights as drawn (`make spec-stats`, the 21 frames of a `make lighting-capture`, 960x720, 2026-11-12): 45% of the textured polygons shown have a lighting entry with a specular term (scale 255, squared twice) and they cover 80% of the textured pixels (mostly the floor), but it brightens 0.01% of them by a level and none by 4: where the term is a level or more the surface is already at 255 (36% of those pixels: ambient and diffuse alone saturate), elsewhere it is under a level (64%). Another 8,908 polygons have a specular control with a zero scale. Doubling the term would change almost nothing more (0.02%).
- no models drawn from polygon RAM; no direct data.
- the light vector changes within a frame in 3321 of 6209 frames (up to 3 vectors a frame); lighting entries not changed within a frame.
- True focal length over the window width: median 1.210 (range 0.502-1.210); the stream-only layer guesses 1.0.
- The list scan's block facts equal the native geometrizer's on 303,700 blocks, differ on 0.

## Track-title pop-up and billboard logo (for live soundtracks)

Investigated 2026-11-13 for the live-soundtracks plan (`docs/mods.md`, parts 3
and 4). Found by running a coin-start race, snapshotting it frame by frame,
watching the tile RAM and char RAM writes, and reading the i960 code and ROM
data. Frame numbers are of a `coin-start` run. No ROM data is reproduced here.

### The title bar is not a pop-up: it is a standing 2D HUD bar

- *Confirmed:* it is drawn on the **2D tile layer**, plane A, map 0
  (`0x01000000`), with the priority bit set (in front of the 3D), not as
  polygons. It occupies tile rows 43-46 and columns 19-60: 42 x 4 tiles,
  336 x 32 pixels, at screen (152, 344) of the 496 x 384 frame. Layout of
  columns: 19-21 a cap and a pause or play icon, 22 a divider, **23-43 the
  title (21 x 2 tiles, 168 x 16 px)**, 44 a divider, **45-60 the "by ARTIST"
  image (16 x 2 tiles, 128 x 16 px)**. Rows 43 and 46 are the frame.
- *Confirmed:* it is **there for the whole race, not for a few seconds**: it
  appears with the race scene (frame 2891, one frame after the stop
  `AE 10 00` of the course-select track at 2890), shows a pause icon through
  the 3-2-1 countdown, and switches to the play icon when the game sends
  `AE 10 07` (frame 3144, the GO). It stays to the end of the race (TIME UP,
  stop at 8903). There is no slide, fade or timed hide: appearance and the
  icon change are instant, and the whole bar is rewritten into tile RAM every
  frame.
- *Confirmed:* the title changes **mid-track, with no music command**. In a
  default race the bar shows "WOULDN'T IT BE NICE" from 2891 and cuts to
  "IT'S WHAT YOU DO WITH IT" at frame 7490, 4346 frames (75.6 s at 57.52 Hz)
  after `AE 10 07`; nothing is sent to the DSB2 then. The game keeps its own
  schedule. *Unconfirmed:* the rule. DSB2 track 7 (`docs/hw/sound.md` 3.7)
  is about 235 s long and tracks 8 and 9 are later entry points into it, at
  77.8 s and 157.4 s: it looks like three songs back to back, and the cut at
  75.6 s is near the first boundary (2 s early). Tracks 1-3 look the same.
  What the game keys the cut on, and which title goes with tracks other than
  7, is not found.
- *Confirmed:* the same song ("Wouldn't It Be Nice") starts the race on both
  the Novice and Expert courses (track 7 both times). Every title seen is
  "by PENNYWISE"; only one artist image exists in the ROM data (see below).

### Where it comes from

- *Confirmed:* **there is no font.** Titles and artist are pre-rendered
  4-bits-per-pixel bitmaps in the ROM's main data region, hand-lettered
  italic capitals ("by" in script), not glyph codes. Nothing builds them
  from characters, so there is no character set, case or punctuation list to
  report: a replacement title has to be rasterised in our own font.
- *Confirmed:* title pixels: pens 1-14 only (no transparent pen), pen 1 the
  background green and pens 2-14 a ramp to black, so black antialiased
  letters on a flat green; palette bank `0x16` (xBGR555 entries `0x83E0` to
  `0x8000`). Artist: pens 1-15, white lettering on black, bank `0x17`. The
  field is a fixed size, so a long title must be cut or scaled to 168 x 16.
- *Confirmed:* the data is loaded once per race, at about frame 2878 (a
  scene change, while the screen is white), by the char-set loader at
  `0xA9420`, called from the graphics-set selector `0xA9640` (set 0, the
  in-race set, uses the table at ROM `0x02400000`). The block holding the
  titles is the entry at table offset `0xF0E0` (data from ROM `0x0240F120`),
  which fills char RAM tiles `0xA00-0xB58` (345 tiles, about eight titles of
  variable width packed together; "WOULDN'T IT BE NICE" is the last, tiles
  `0xB2F-0xB58` at ROM `0x02411700`, "IT'S WHAT YOU DO WITH IT" is tiles
  `0xA23-0xA4C`). The play and pause icons follow at `0x02411C40`, and the
  one "by PENNYWISE" image is tiles `0xB9F-0xBBE` at ROM `0x02412060`.
- *Confirmed:* which tiles the bar uses comes from tile-map pieces in the
  same region, run-length coded. The decoder is at `0xA88E0` (it writes
  the map entries; the runs of "next tile number" that lay out a title are
  the 3-byte op at `0xA8A20`) and it runs every frame. The pieces are listed
  by a pointer table at ROM `0x0240009C` (offsets from `0x02400000`).
  Entry 42 (ROM `0x0246CEBB`) is the "WOULDN'T IT BE NICE" bar and entry 35
  (`0x0246CD96`) the "IT'S WHAT YOU DO WITH IT" one; entries 36 and 39-41
  hold other titles' runs (*unconfirmed*: which song each is, and the code
  that picks the entry and times the cut, which was not traced). The icon
  tiles differ for pause (`0xB8A-0xB8D`) and play (`0xB83-0xB87`).

### Could the text be swapped at render time without writing game memory?

- *Judged yes (not built):* the title and artist are two fixed rectangles of
  tile-map cells (map 0, rows 44-45, columns 23-43 and 45-60), always drawn
  with tiles from the pool above. A renderer that draws plane A could, when
  it meets those cells with those tile numbers, draw a 168 x 16 or 128 x 16
  image of our own in the same pens' colours (green background and black
  lettering for the title) instead, in the same pass and priority. Game
  memory is not written. Because the game never changes the map to signal a
  new song except at its own cuts, the overlay should be keyed on "race
  scene with the bar present" (the divider tile `0x8A00` and the frame
  tiles), not on the game's text; to show a new song at once, draw our text
  whenever the source says so, since the game's bar needs no animation to
  play. The pause and play icons can be kept as the game draws them.
- The existing texture-pack tile replacement is per 8 x 8 tile by content, so
  it can restyle the fixed frame and icons but cannot carry changing text;
  live text needs the render-time overlay above.

### The band's logo on billboards

- *Confirmed (by looking at the dump):* it is Pennywise's logo, a black
  ring and "P" on an orange disc, texture
  **`tex_a3b00416296da378_128x128`** (header `6012,0001,1d20,2680`, bank 1 at
  (1024, 640), luma table 1, translucent, drawn with repeat wrap). It fills
  its whole 128 x 128 texture (a square, edge to edge circle), so it is a
  **whole texture, not part of a sheet**, and needs no sub-rectangle.
- *Confirmed:* there is **one size only**: no other 3D texture in the dump
  (about 240 textures of a race run; also an older dump of 2,600) matches a
  downscale of it. topskatr uploads no mipmaps (see above), so there are no
  LOD variants.
- *Confirmed:* it is drawn on the red banners of the Novice course's start
  area (visible at frame 2940 on a red and white banner beside the start
  ramp). It was drawn in only two polygons in the sampled frames, so a
  billboard of it is rare in view. *Unconfirmed:* other courses and where
  else it appears; only a coin-start (Novice) race was dumped.
- The same dump holds the other billboards for reference (an airwalk, a
  warp magazine, Coca-Cola, a stunttrackers sign); only the Pennywise logo is
  the band's.

## Enhanced preset

What the player's Smooth modern picture style (the `enhanced` preset; `romset::Recommended` in the
manifest, `run_ahead_max` for run-ahead) turns on for `topskatr`, and why:
Temporal anti-aliasing at full size; the depth buffer on (the skater's leg
and the tree billboards come out right in the 21 captured frames, nothing
broken: "Depth buffer" in `docs/enhancements.md`); smooth shading, vertex
with the game's crease (the skaters are clearly better in the review
pictures: `docs/play.md`, "Smooth shading"); smooth motion; run-ahead 0 (no
lag to hide). The FINISH sign flickers under temporal as the game draws it
(confirmed above).

## Idle loop

- *Confirmed (from the code, then by running it):* the main program
  waits for the next frame in the loop at `0x4820C-0x48223` (the
  manifest's `idle_loops`; found as the hottest code of a coin-start
  run). It polls the video status register for vblank: `ld 0x98000c`,
  its frame bit (bit 2) compared with the one it read before the loop,
  branching back while they are equal, and `g0` added to on every pass (a
  count of passes nothing in the loop reads). It leaves the loop on the
  first read after vblank toggles the bit. The register is read through
  `g6`, which holds `0x98000C`.
- *Confirmed (by running it):* with the idle skip (`docs/idle-skip.md`)
  a 600M-instruction coin-start run (`make bench`) fast-forwards 82.0%
  of its instructions through it, and the state hash is unchanged.
