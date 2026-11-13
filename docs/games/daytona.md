# Daytona USA (`daytona`, the original Model 2, 1994)

Status: **plays** (`make play GAME=daytona`; the launcher's Model 2
group): it boots to its attract mode, draws it in 3D, textured, and
`coin-start` reaches a race on each course that drives (and runs out its
clock), with music and effects (the sound board model, not yet checked by
ear against a cabinet). The keys steer
through a ramp, as the game wants a wheel that can't jump to full lock
(below, "the handling"). Known issues: no whole 8-lap race has been
steered to the flag, and the clones are not added. The
first game on the original Model 2 board (`Board::Model2`,
[`docs/hw/model2.md`](../hw/model2.md)); `vcop` and `desert` are on the
same board.

## What's known

- *Confirmed:* the manifest verifies (34 chips of the merged
  `daytona.zip`: revision A's program `epr-16722a.12`/`-16723a.13` and
  main data `epr-16724a.6`/`-16725a.7`, the rest shared with the clones).
  The chips' roles were worked out from the data (`docs/hw/model2.md` §3);
  the emulator's .dat was used only as a catalogue of which file names
  belong to which set.
- *Confirmed (by running it):* the i960 boots on the 2A board model with
  three changes (`docs/hw/model2.md` §2): the program ROM's second half at
  `0x220000`, where the code runs; the I/O registers at `0x1C00000 + 2N`;
  and the I/O board's command register and 128-byte buffer at
  `0x1C00040`/`0x1C00042`/`0x1C00200`. Without the last the boot waits
  forever for status `0x40` (`0x228D6C`).
- *Confirmed (from the code and the run):* **the settings live on the I/O
  board**: at boot the game asks the board for its 128-byte record (command
  3), copies it to the SRAM at `0x1D00000`, checks it (a "SEGA" header, a
  version, a CRC-16) and otherwise takes the program's defaults, which
  are a **linked** cabinet (link ID MASTER, cabinet TWIN). With no
  network board (`0x1A14000` bit 0 set) a MASTER shows "NETWORK BOARD NOT
  PRESENT", "CANCELLED" and restarts, over and over. The manifest's
  `backup_ram` is the defaults with the link ID at byte `0x0B` set to
  SINGLE and the CRC to match; with it the network check is skipped and
  attract runs. (The settings screen at boot then reads LINK ID SINGLE,
  CAR NUMBER 1, COUNTRY JPN, DIFFICULTY NORMAL, ADVERTISE SOUND ON, GAME
  MODE NORMAL, RIVAL ARROW ON, one coin a credit.)
- *Confirmed (by running it):* the TGP program (2,024 words, in main data
  `.6/.7`) is uploaded at frame 14 and runs on the MB86234 interpreter
  with **no word of an unknown form** and no unknown external access, in
  attract and a race (a 500M-instruction coin-start run, 92 million TGP
  words); the game calls about 40 functions, all through the command
  ports (the load check only polls the status register, `0x980004` bit 0,
  then writes command port 8: no answer is read). Three findings made it
  run (`docs/hw/mb86234.md`): its data ROM pair is `.28` low, `.29` high
  (it starts with an identity matrix); it reads that ROM by putting the
  address in the bus register (`docs/hw/model2.md` §4), so the
  interpreter now takes the external address's high bits from there; and
  its function `0x36` calls routines through addresses it keeps in RAM0
  (`BF66 0035`), which the interpreter took as registers: the TGP jumped
  to 0, restarted, and attract froze on its first 3D frame. A function
  unit port `0x22` (read after an angle is written) is taken to be -sin
  (*hypothesis*, which its one use supports: below).
- *Confirmed (by running it):* `coin-start` (one coin, start, the
  accelerator on the CIRCUIT SELECT screen and again on MISSION SELECT)
  starts a Beginner race with an automatic: the rolling start, then with
  the accelerator held the car pulls away (206 km/h on the speedometer
  under the rev gauge at frame 2,780; the big number above the gauge is
  the time left, which an earlier note here took for the speed; the lap
  timer running, "POSITION 40/40"), runs into the wall on the first
  bend with the wheel centred, and turned (`wheel=0x40` for a second)
  spins round to face the wrong way ("逆走"). Without the RAM0 call fix
  the race stayed in its rolling start at 50 km/h.
- *Confirmed (by the pictures):* attract draws the AM2 logo, "©SEGA
  1994", the attract races in 3D (the oval's banking and stands, a
  mountain pass with rock walls, a forest road, cars with their liveries,
  sky with clouds), the "V.R." view-button screen and the ranking tables,
  textured and lit, with the 2A layouts of polygon and texture ROM and
  video RAM.
- *Confirmed (from the game's own input test, test menu, by running it):*
  hold the test switch in attract (`test@1300-1330`) for the **TEST MODE**
  menu (BOOKKEEPING, GAME SYSTEM, COIN ASSIGNMENT, INPUT TEST, OUTPUT TEST,
  DRIVE BD TEST, SOUND TEST, TGP TEST, MEMORY TEST, BACKUP RAM CLEAR,
  EXIT; the screen says GREEN cursor up, RED cursor down, START select;
  with the model, the service switch moved the cursor down and the fourth
  view button up). INPUT TEST lists CHUTE1/2, SHIFT1-4, VR1-4, START,
  TEST-SW, SERVICE-SW and the raw bytes of HANDLE, ACCEL and BRAKE (a 0
  prints as a bare `H`). Driven from the scripts it showed:
  - register 8 (port 0): coin 1 `0x01` (CHUTE1), coin 2 `0x02` (CHUTE2),
    test `0x04`, service `0x08`, start `0x10`, VR1 `0x20`, VR2 `0x40`,
    VR3 `0x80`; register 9 bit 0 (port 1 `0x01`) VR4;
  - analog channel 0 = HANDLE, 1 = ACCEL, 2 = BRAKE, each read raw: the
    wheel reads `80H` at rest, the pedals `0`; `wheel=0x40` read `40H`,
    `gas=0x80`/`0xFF`, `brake=0x80`/`0xFF` read back the same.
  *Confirmed (from the code, `0x57A8`-`0x582C`):* the wheel is an
  **absolute position**: `v` is the byte, `d = v - 128`; `|d| <= 5` is a
  dead zone (steering 0), otherwise steering = `(|d| - 5)`, clamped to 75,
  times 255/150, signed: **below `0x80` is left, above is right**, full
  lock at `0x30`/`0xD0`. (A step of one unit past the dead zone is a
  steering value of 1 of 127; `0x7C` and `0x84` drive exactly as the
  centre.) The same value picks the course on CIRCUIT SELECT ("select
  course by steer"; the accelerator chooses): centre and left Beginner
  (8 laps), about `0x98`-`0xB4` Advanced (4 laps), `0xC8` and more Expert
  (2 laps), as a position that has to be held when the accelerator is
  pressed; MISSION SELECT takes the wheel the same way (Automatic at the
  centre, Manual-4 to the right).
  *Confirmed (from the code, `0x1D950`, `0x10830`):* the gear lever is
  register 9 bits 4-6 (port 1, as `srallyc`'s): a table at `0x230D1C` turns
  the pressed bits into a gear (bits 4+6 gear 1, 5+6 gear 2, 4 gear 3,
  5 gear 4; none, 4+5 and 6 alone neutral), the same bit patterns as
  `srallyc`, so the manifest gives it the same `Shifter`. *Confirmed by
  running it:* with the gear buttons held, the input test's SHIFT1, SHIFT2,
  SHIFT3 and SHIFT4 light for gears 1 to 4. *Not driven:* a Manual-4 race.
- *Confirmed (from the sound program, and by running it):* the sound
  board is not the 2A/2B/2C one: a 68000 (program `epr-16720.7`, RAM at
  `0xF00000`) taking the i960's MIDI-style bytes on a UART in its level-2
  interrupt, two MultiPCMs (`.32`+`.33` the music's samples, `.4`+`.5`
  the effects'; banked by MB) and an OPN2 FM chip that the program uses
  only for timer B, its tempo (`docs/hw/model2.md` §5,
  [`docs/hw/multipcm.md`](../hw/multipcm.md)). Modelled as
  `sound::PcmBoard`: `make sound` renders the captured link, and the
  player and the browser play it live. In a coin-start run the music
  starts 3.3 s after boot; the attract tune is about 100 beats a minute
  and the Beginner race's about 139 (with the FM chip's clock *assumed* 8
  MHz), in tune with A440 within about 5 cents; attract plays at a lower
  level than the race (the host sets it). *Unconfirmed:* the clocks, the
  envelope and LFO timings, the board's mix.
- *Inferred (from the game's samples):* **the MultiPCM plays a sample at
  its stored rate at octave 1**, not 0: the model played every sample an
  octave high and twice as fast ("too fast, pitch too high" in play).
  The announcer's "GENTLEMEN, START YOUR ENGINES" (second chip, sample
  47, keyed at octave -1 at frame 1852 of a coin-start run) now lasts 2.6
  s, the time the game shows its title card (frames 1852-2012), at a
  man's pitch (185-200 Hz; it was 1.3 s at 370-400 Hz), and 77 of the
  samples the game plays are keyed mostly at exactly octave 1, F 0
  ([`multipcm.md`](../hw/multipcm.md), "Base rate").
- *Confirmed (by running it):* the idle loops are the waits on the
  frame counter byte at work RAM `0x500000`, in the main loop
  (`0x1394`-`0x13A0`) and the boot screens' (`0x1B8B8`-`0x1B8C4`): 74 to
  78% of the i960's instructions are skipped in coin-start runs.

- *Confirmed (by running it):* **a race ends and returns to attract.**
  Driven with the accelerator held and the wheel centred (the car runs into
  the wall at the first bend and sits there), Beginner counts its clock
  (the number above the rev gauge, about 99 seconds) down in about 6,000 frames,
  shows "RESULTS 40th", "GAME OVER" and goes through the title and the
  attract races again: no hang, crash or misdraw on that path. (40th, so
  no name entry.) Advanced (20 cars by the HUD's POSITION, `wheel=0xA4` held through the
  accelerator at CIRCUIT SELECT) and Expert (`0xE8`) end the same way
  (RESULTS 20th and 30th). Not done: a whole
  8-lap Beginner race to the flag; the car wants steering through the
  bends (a steering script, below).
- *Confirmed (by running it):* **the Advanced and Expert races draw**
  (fixed; they used to show only sky and sea, with the HUD, lap and time
  running and the marker moving on the mini map, from the start lights
  on). The cause was the TGP model's special values: the car code at
  i960 `0xFFE8` divides (function 3, `a * 1/b` through the reciprocal
  port) by the sine of an angle difference that is sometimes 0, and then
  multiplies the result by the cosine of a quarter turn, 0. With IEEE
  rules that was infinity times 0, a NaN (from frame 2,170 of an
  Advanced coin-start run), which reached the position of the car the
  view follows (`0x170`/`0x174` of the object at `0x511400`, i960
  `0x102A4`-`0x102AC`) when the race started at frame 2,400; the view
  was then lost. The MB86234 model now takes exponent 255 as an ordinary
  binade and saturates beyond it (no infinities or NaNs: a *hypothesis*
  for the chip, [`mb86234.md`](../hw/mb86234.md), "Special values"), and
  both courses draw road, scenery, cars and the player's car all race
  (coin-start runs with `wheel=0xa4@1450-1520` or `wheel=0xe8@1450-1520`
  and the accelerator held, to frame 6,200). Beginner met the same
  infinity but drew. *Unconfirmed:* what the hardware does with a
  division by 0 (the reciprocal port's output for 0 is this model's).
- *Confirmed (from the code and by running it):* **the handling**: why
  the car was twitchy from the keyboard ("very jerky", snapping left and
  right). The wheel's path, traced per frame (logs of work RAM and of the
  TGP calls from a local debugging build, in a coin-start race with the
  accelerator held, steering from frame 2,780 at 206 km/h):
  - The ADC is read sanely: the four samples the game averages (I/O
    registers 0-7, `docs/hw/model2.md`) give the scripted byte every
    frame, with one in-between value on the frame it changes (`0xD0`
    pressed: steering 25, then 127). No noise, spikes or jumps.
  - `0x57A8`: steering -127..127 (above) into `0x501050`. The car's
    routine (`0xF048`, object `0x505180` for the player's car in these
    runs) keeps `(steering * 256 + last) / 2` at `0xBC`: a one-pole
    filter that **halves the gap each frame**, the game's only smoothing.
    `0xFD50` makes the front wheels' angle from it, `-(it >> 3)` at
    `0x128` (in 1/65536 turns): full lock is `0xFE0`, 22 degrees.
  - `0xFFE8`-`0x10214` turns that into yaw through the TGP: a bicycle
    model, the turning radius from the wheelbase (2.75) by the law of
    sines over the front and rear angles (functions 29, 27, 3, 30, 10:
    `x sin a`, `sin a`, `a / b`, `x cos a`, an atan2), then the yaw step
    as speed (`0x34`, 0.895 a frame) over the radius times 10430.4
    (65536 / 2π) (functions 36, 41, 40, 37), added to the heading at
    `0x58`/`0x48`. The values check by hand: at frame 2,781 the angle
    difference is -10.6 degrees, the radius 2.75 / sin 10.6° = 14.9, and
    0.8965 / 14.75 radians is the 634 units (3.5 degrees) the heading
    moves; the speed byte at `0x18` (206) is the speedometer's km/h and
    0.895 m a frame is 54 m/s, the same speed. Nothing out of range, no
    saturated values on this path while turning (the 2^128 of a straight
    line only gives a yaw step of 0, as it should).
  So the emulation does what the code says; the cause is the input. A
  key holds the wheel at full lock (`0xD0`) from one frame to the next,
  which a cabinet's wheel can't do; the game's filter gets the front
  wheels to 22 degrees in two frames, and at 200 km/h the heading moves
  7.5 degrees in one frame, swings back past where it started when the
  key is let go (+2.9 degrees the other way) and settles over 20 frames:
  a 6-frame tap moved the heading 30 degrees all told for a net 2. The
  fix (`KeyRamp` in the manifest, docs/play.md "Digital steering"): the
  keys and d-pad turn the wheel to full lock over 12 frames and let it
  back in 6; a stick goes straight through. The same tap then peaks at
  1.5 degrees a frame with no swing back (heading 0 to -3.7 degrees,
  30 degrees of movement down to 11); four alternating 6-frame taps
  (left, right, ...) went from 107 degrees of heading movement and 7.7
  a frame to 42 and 1.6. (Measured with scripts holding the values the
  ramp gives, frames 2,780-2,850 of `coin-start,gas=0xff@2300-99999`.)
  *Unconfirmed:* how fast the cabinet's wheel can really be turned (the
  12 and 6 frames are chosen to feel right, not measured), and the
  slip terms at `0x18C`/`0x18E` (the heading rebounds by about half
  within 3 frames of a step, with the wheel still held), which look
  like the game's own tyre model. `srallyc` handles the keys' switch
  well (span 112) without a ramp; its steering code was not traced.
- *Confirmed (by running it):* **input lag** (`make input-lag GAME=daytona`,
  `wheel=0xD0` held from frame 3300 of a coin-start run with the
  accelerator held, and from 2900): the picture changes in the first frame
  that can read it, so `input_lag` and `run_ahead_max` are 0.
- *Clones:* the merged `daytona.zip` carries the chips of `dayton93`
  ('93 Edition: `epr-16530a/16531a.12/.13`, `epr-16534a.6`/`16535.7`, other
  main-data chips), `daytonas` (Saturn Ads: `epr-17965/17966`,
  `epr17967/17968`), `daytonase` (Special Edition: `epr-17369a/17370a`,
  `epr-13731/13732`), `daytonat` and `daytonata` (Turbo: `turbo1/2.12/.13`,
  `dayturbo.12/.13`), `daytonagtx` (GTX2004: `gtx.12/.13`, `bank0/1.bin`)
  and `daytonam` (To The Maxx: `maxx.12/.13/.14`), by the emulator's
  catalogue. None added: most replace the program and part of the data,
  so each needs a layout check and the Turbo, GTX and Maxx sets are
  patches on other hardware revisions (unconfirmed).

## Next

1. A steering script for a whole Beginner race (a tracking script; the
   car's position, `0x170`/`0x174` of the object at `0x511400` by the
   above, is not read from the game yet).
2. A Manual-4 race, to try the gears in play.
3. The sound board by ear against a recording of the cabinet: tempo (the
   FM chip's clock), envelopes, the level of attract against the race.
4. `vcop` and `desert` on the same board model (`vcop`'s chips have
   daytona's layout, sound board included).
5. The drive board (register 17's queue) and the link board.
