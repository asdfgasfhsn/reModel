# Virtua Fighter 2 (`vf2`, Model 2A-CRX, 1995)

Status: **plays** (`make play GAME=vf2`; the launcher's Model 2A group): a
game starts, the player-select screen draws its fighter, and its fights
play out round after round.

- *Confirmed:* the manifest verifies (33 chips, program set 2.1:
  `epr-18385..18388`). The i960 boots and uploads a 3,802-word TGP program
  at frame 34, which now runs on the MB86234 interpreter with no word of
  an unknown form (`docs/hw/mb86234.md`, "What `vf2`'s program added").
  Attract shows the fighters and stages in 3D, then the warning, the SEGA
  logo and "INSERT COIN(S)". The default setting is two coins a credit
  ("CREDIT 1/2" after one): `coin-start` inserts two, starts, and its
  punches pick Akira, and a round against Lau begins (a coin-start run
  of 600M instructions: player select at frame ~1,650, "SET 1" ~1,750).
- *Confirmed (by running its functions with chosen arguments on the
  interpreter):* the TGP's functions that the game calls most do the
  expected mathematics: arithmetic, 1/sqrt, sqrt, sine and cosine,
  atan2 and asin, push, pop, identity, translate, scale, the three
  rotations, transforming points, distances, normalising. Its function
  numbers dispatch through a table at `0x3F` + F; the argument and reply
  counts of every function the game calls match its code.
- *Confirmed (by running it):* fights play out. A coin-start run of
  1.5G instructions (10,354 frames) plays round 1 against Lau to a K.O.
  (frame ~2,700), "WINNER", then the next opponents (Sarah, then Shun)
  through rounds 2 and 3, with no ring-out. Before, a fighter fell
  through the ring 1.7 s into round 1 ("RING OUT"). The cause was the
  TGP's functions `0x6C`-`0x6F`, which read the skeleton's rotation
  angles and translations from a 64-word ring the i960 fills in shared
  RAM at `0x3800` (buffer RAM `0x90E000`-`0x90E0FF`): the MB86234 steps
  their pointer round the ring (a circular operand,
  `docs/hw/mb86234.md`), and the interpreter let it run on past the ring
  into other data, so the bone matrices filled with denormals and NaNs
  (from frame 979, in attract too) and the game's floor and ring-edge
  tests, which use the bones, saw a fighter outside the ring. Unconfirmed
  for the floor work itself: functions `0x4A`, `0x3A`/`0x3B`/`0x3E` were
  not changed and now give finite results throughout (they had passed
  the NaNs on).
- *Confirmed (by the pictures):* the stages and fighters are textured
  (stone towers, trees, brick floor, clothes and faces). The grey was
  not the chip roles, the polygons' attributes or the renderer: the
  texture headers and point lists (the `.25/.24` pair) decoded all
  along, and the polygons drew as textured with sensible headers (5,700
  distinct textures in a 200M-instruction run), but their texel sheets
  were empty. The game's boot copy fills Model 2A's video RAM only in
  the upper half of each bank's window (bank 0 words `0x80000`-`0xFFFFF`
  from `0x12200000`, bank 1 the same from `0x12600000`), where
  `srallyc` fills the lower half. A bank is 512K words (the sheet's
  1 MB), so its window's 2 MB of 32-bit words repeat, and the board model
  kept the upper half at an address the rasteriser never reads.
  `model2::bus::mirror_video_2a` now folds each bank's words to its 512K.
  Whether the hardware repeats the window or `vf2` addresses the bank
  differently is not told apart (both draw the same).
- *Confirmed (by the pictures, 2026-11-13):* the second texture pair
  `.27` (even words) / `.26` (odd) sits in its own 8 MB slot, at 16-bit
  word `0x400000` of the texture ROM, not straight after the first pair:
  the manifest no longer lists `Role::Textures` as packed. The display
  lists address texture points and headers at `0`-`0x1FFFFF` and
  `0x400000`-`0x5FFFFF` and nowhere between (`docs/hw/model2a.md` §5).
  Packed, the bank ended at `0x3FFFFF` and every read of the second pair
  gave 0. That was the "odd 3D on faces, hair and things attached to
  fighters": a fighter's head is one model drawn with the textures of
  its expression, and the base expression's points are in the first
  pair but the blinking and shouting ones in the second, so in those
  frames every face polygon sampled texel (0, 0) of its sheet (a flat
  skin colour where the face should be, flickering as the expression
  changed; the polygons, matrices and sort order were the same in good
  and bad frames). Lau's vest embroidery and sash, Akira's chest, the
  floor beyond the ring and the shadows' headers were flat or missing
  the same way. With the slot, faces animate (blinks, open mouths) and
  those surfaces are textured.
- *Confirmed (by running it and from the game's code, 2026-11-13):* the
  "flickering hair" was the fighters' headband tails, hair and braids
  (Akira's headband, Lau's braid and sash: objects with second-pair
  textures) missing from the display list in 3 frames of 5, in every
  renderer and with every enhancement off: the game left them out.
  It draws them in a task per fighter that has a time budget: each frame
  it reads how long the frame has run (timer 2, started at vblank) and
  skips a fighter's task if that plus the task's measured cost would
  pass 17,000 us (`0x4268`), counting the skips in RAM `0x500164`. When a
  task runs, it appends its objects at the end of the list and links them
  in with a jump the fighter's own objects left (buffer RAM `0x1960` into
  the list, patched to the appended block), so a skipped frame has the
  fighter without them. The game reads timer counts one word above where
  it writes them (`docs/hw/peripherals.md`, "Timers"), and the board
  model returned the timer at the address read, so the elapsed time came
  from a timer just restarted or one not running and the budget looked
  spent: 1,308 skips by frame 2,250 of a coin-start run. With the read
  moved a word, none in a 10,354-frame run (fights, game over, attract),
  the tails and braid are drawn every frame, and the game plays as before
  (rounds against Lau and Sarah, K.O.s, "GAME OVER"). The fights differ
  from earlier runs from the first frame the game reads a timer, so the
  frame numbers of older notes here no longer match exactly.
- *Confirmed (by running it):* the player-select screen draws its 3D
  fighter (the chosen fighter's head turning in the box on the left).
  Its lists (frames ~1,265-1,463 of a coin-start run) have no window
  command at all, only objects, so the renderers, which took each
  frame's raster stream on its own, had no window for them and dropped
  every polygon (none was built from the 2,837-word stream). The
  rasteriser keeps its window from frame to frame, so these objects draw
  in the window the last frame that set one left it with (the full
  screen); `model2::geo::carry_window` puts that window at the start of
  a stream that doesn't begin with its own (*hypothesis* for the
  hardware: that the rasteriser's window registers are simply kept;
  the picture is what the screen is for). It was not the TGP: the
  i960's ring writes and the TGP's ring pointer agree on every frame of
  a coin-start run (the first slot the i960 writes in a frame is RAM0
  `0x3D` at the frame's first `0x6C`-`0x6F` call, 2,632 frames, select
  screen included), the ring holds the pose (translation about
  (-0.22, -0.07, 1.01), then the angles), and function `0x34` copies
  the posed matrix, RAM1 at `[0x3F]` after a push/pop around `0x29`, to
  the list (byte offsets `0x18050`/`0x18098`, `0x50`/`0x98`, ...; the
  lists rotate round four 32 KB buffers at `0x900000`, and the i960
  leaves 12 words for it after each `0xB` command). Earlier notes here
  that the matrices came out identity were wrong.
- *Unconfirmed:* chip roles are by analogy with `srallyc` (main data
  `.10/.11 .8/.9 .6/.7 .4/.5`, polygons `.16/.20 .17/.21 .18/.22`, textures
  `.25/.24 .27/.26`).
- *Confirmed (from the ROM data and the sound driver; not yet by ear):*
  the samples are `.31 .32 .36 .37`, 2 MB each, back to back at the sound
  68000's `0x800000-0xFFFFFF`, tested as for `srallyc`
  (docs/games/srallyc.md). The sound program `epr-17574.30` has the same
  kind of sample table, at 68000 `0x605B28` (20-byte entries, 400 samples
  among empty rows), with samples in all four 2 MB slots, none crossing a
  boundary. The loops' 512-byte repeats after their ends are found only
  with `.31` in the first slot (18 of 24 loops) and `.32` in the second
  (25 of 30), in no other chip; the third slot has 3 loops (one repeat,
  in `.36`) and the fourth none, so `.36` and `.37` are placed by where
  their data ends: exactly at the third slot's last sample for `.36`, one
  byte after the fourth's for `.37` (`.31` 0x66 bytes before the first's,
  `.32` exactly at the second's); swapped, four more samples would be
  blank and the end of `.37` unreferenced. All four chips are 8-bit PCM
  by their statistics. The driver runs in a coin-start run (363 bytes
  from the host in 120 s; `sound-render`: 5134 key-ons, 24.8 MB of sample
  reads, all inside table entries). The role rests on the sound program's
  table and the chips' contents.
- *Confirmed (by running it; not yet by ear):* the manifest adds no gain
  (`volume_db: 0.0`). The 6 dB it had before clipped: a 1.5G-step
  coin-start run with `--audio` (the player's mix) measured about -13
  LUFS after 30 s, peaks at 0 dBFS and 3265 clipped samples. With no gain
  the same run gives -19.0 LUFS, peaks -1.4 dBFS and nothing clipped,
  inside the other games' gameplay range (-16 to -28 LUFS, median about
  -21.5; `srallyc.md`).
- *Confirmed (by measurement; the pitch complaint, 2026-11-13):* "odd
  slow downs that sound like pitch changes" in the native player are the
  live playback pacing, not the emulated sound. The emulated sound is
  fixed in emulated time (the live path feeds each host byte at its exact
  sample, so host speed can't move it): in a 70 s coin-start run (fight
  included) the 68000's timer A interrupt, the driver's tempo clock, came
  every 49 samples (the manual's `0xFF - 0xCE`) 60,361 times of 60,886,
  50 to 52 samples 24 times, and the rest were the driver's boot, before
  3 s; timer B every 504 samples likewise. None of the 1,808 key-ons use
  the pitch LFO, FM or a non-looping/reverse mode (OCT -3 to 0 only), so
  no modulation can bend a note. In the player, on a quiet machine at
  default settings, the audio step stays within 0.5% of the display's
  (1.043 on a 60 Hz display) for a minute (0 underruns). With the
  settings the author's `out/play-settings.txt` has (8x MSAA, pixel
  shading, temporal 50%, smooth motion, run-ahead 1) a fight's draw alone
  takes 9-12 ms (3 ms in attract) on an RTX 2080 SUPER, and with about
  7 ms of emulation (the i960 interpreter, the most of any game here)
  that is the 16.7 ms a refresh gives: the player drops from "synced to
  60 Hz" (104% speed) to "paced by the clock" (100%), a 70-cent slide,
  and frames come in bursts of two and three refreshes. The rate control
  (`model2::audio::Pace`) then bent the pitch by up to 100 cents over a
  second with the game's true speed unchanged, and a single 0.2 s hitch
  (2 per 110 s here) pulled it down by up to 40% for seconds, since the
  clock's 11 frames due against 3 run read as a slow game. Fixed in the
  player (`docs/play.md`): hitches over 0.1 s aren't counted as slowness,
  and the queue's target deepens to 150 ms while redraws are uneven. On a
  replay of the measured frame timings that cut underruns (12 to 3 in
  one trace) and the worst pitch dip (1491 to 529 cents in another);
  steady slides from the player changing mode and a game truly below
  full speed are still followed, by design. What would help most is
  cheaper frames: `make play RECOMP=1` takes the i960 interpreter out,
  and the draw settings are the other half. Not confirmed by ear.

- *Confirmed (by the game's input test, driven by `make run` scripts):*
  the test menu (hold `test`; `service` moves the cursor, a fresh `test`
  press selects) lists "INPUT TEST" with every switch of both players.
  Each of the 19 inputs, held one at a time, lit exactly its own line:
  port `0x1C00002` bit 0 coin 1, 1 coin 2, 2 test, 3 service, 4 start 1,
  5 start 2; ports `0x1C00004` and `0x1C00006` (players 1 and 2): bit 0
  punch, 1 kick, 2 guard, 4 down, 5 up, 6 right, 7 left. The manifest's
  controls match (the same layout as `fvipers`; Z guard, X punch, V kick).
  The DIP-like settings (two coins a credit, "CREDIT 1/2") are the game's
  backup RAM defaults, not DIP switches; nothing needs setting.
- *Confirmed (by running it, 2026-11-13: a coin-start run of 3.5G
  instructions, 24,160 frames):* the whole game loop with no hang, crash
  or garbage frame: rounds against Lau, Sarah, Shun and Pai (stages 1-4,
  "SET 1"/"SET 2", K.O., "REPLAY", "WINNER", "TRY NEXT STAGE"), then the
  scripted player loses to Pai: "YOU LOSE", "CONTINUE?" counting down,
  "GAME OVER", the SEGA logo, and back to attract (fighters in 3D, title,
  the ranking screens, "INSERT COIN(S)"). From a save state at the
  continue screen, two coins and start continue into round 4 again with
  "CREDIT 0/2". With four coins, start and start 2 together: two-player
  "PLAYER SELECT" (Akira against Lion, picked by the scripted buttons)
  and a versus round in which both sides' buttons act. A second player
  joining mid-game (two coins, start 2) gets the challenger's player
  select, a "CHAMPION 1 WIN"/"CHALLENGER" fight and "WINNER", then
  "TRY NEXT STAGE" and the first player's next stage. The player-select
  fighter was missing in each (the window, above; with the window
  carried over it is checked on the one-player screen only).
- *Unconfirmed (not checked against the game):* in the close "REPLAY"
  views large black shapes lie on the floor near the camera. With the
  second texture pair read (above) they are the fighters' own shadows:
  their silhouettes (limbs, a falling fighter), drawn with a shadow
  matrix that flattens each body part onto the floor (the display list's
  matrices for those objects have determinant 0), large because the
  camera is low. The flat grey areas that crossed the floor beyond the
  ring in those views were second-pair textures and are now textured.
- *Input lag, measured (`make input-lag GAME=vf2`: frame 3707 of a
  two-coin coin-start run whose punches and kicks stop at 3400; earlier
  frames of that run are K.O. and round transitions that show nothing):*
  a punch shows a frame after the first that reads it: input lag 1;
  walking right shows in the first frame: 0 (as `lastbrnx`).
  `run_ahead_max` is 1.
- *Depth buffer (2026-11-13; 21 captured coin-start frames, 1440x1080,
  `make depth-check GAME=vf2`):* 15 differ, by at most 0.028% of the
  picture (a few pixels of limb and costume layers); nothing looks
  broken either way, so no depth buffer.
- *Lighting (`make lighting-capture lighting-check GAME=vf2`, 21
  frames):* the side channel is exact on all 63,029 polygons (L2 100%,
  no mismatch, hue kept, software and GPU the same); about 54% of the
  drawn polygons are lit. Smooth shading makes the fighters rounder with
  no seams (review pictures), so the Enhanced preset turns it on.

## Enhanced preset

What the player's Enhanced picture preset (`romset::Recommended` in the
manifest, `run_ahead_max` for run-ahead) turns on for `vf2`, and why:
Temporal at full size; smooth shading on (the fighters are rounder, the
side channel is exact); no depth buffer (nothing recorded as fixed);
smooth motion; run-ahead up to 1 (punches lag a frame, the stick none).

## Idle loop

- *Confirmed (from the code, then by running it):* the main program
  waits for the next frame in the loop at `0x10F98-0x10FA3` (the
  manifest's `idle_loops`; found as the hottest code of a coin-start
  run). It polls a byte at `0x500000` (`ldob`) in work RAM, branching
  back while it equals the value it had before the loop. Nothing in the
  loop writes it, so something else must (an interrupt handler,
  *unconfirmed*: the writer was not traced), and between interrupts
  every pass is the same.
- *Confirmed (by running it):* with the idle skip (`docs/idle-skip.md`)
  a 600M-instruction coin-start run (`make bench`) fast-forwards 60.6%
  of its instructions through it, and the state hash is unchanged.
