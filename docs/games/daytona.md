# Daytona USA (`daytona`, the original Model 2, 1994)

Status: **in progress** (`romset::games::IN_PROGRESS`, not in the
launcher): it boots to its attract mode, draws it in 3D, textured, and
`coin-start` reaches a race that drives; it is silent. The
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
  (*hypothesis*).
- *Confirmed (by running it):* `coin-start` (one coin, start, the
  accelerator on the CIRCUIT SELECT screen and again on MISSION SELECT)
  starts a Beginner race with an automatic: the rolling start, then with
  the accelerator held the car pulls away (95 km/h four seconds in, the
  lap timer running, "POSITION 40/40"), runs into the wall on the first
  bend with the wheel centred, and turned (`wheel=0x40` for a second)
  spins round to face the wrong way ("逆走"). Without the RAM0 call fix
  the race stayed in its rolling start at 50 km/h.
- *Confirmed (by the pictures):* attract draws the AM2 logo, "©SEGA
  1994", the attract races in 3D (the oval's banking and stands, a
  mountain pass with rock walls, a forest road, cars with their liveries,
  sky with clouds), the "V.R." view-button screen and the ranking tables,
  textured and lit, with the 2A layouts of polygon and texture ROM and
  video RAM.
- *Confirmed (from the input polling code):* register 8 holds coin 1
  `0x01`, coin 2 `0x02`, test `0x04`, service `0x08` (the code acts on
  those bits), start `0x10`, and the view buttons `0x20`, `0x40`, `0x80`
  with register 9 bit 0 the fourth; analog channels are registers 0-7.
  *Confirmed by running it:* the accelerator is channel 1 (it confirms
  the select screens and drives the car), the wheel channel 0 (it turns
  the car). *Unconfirmed:* the brake (channel 2 assumed), the wheel's
  direction and the shifter's bits (to read off the test menu's input
  test).
- *Confirmed (from the sound program):* the sound board is not the
  2A/2B/2C one: a 68000 with RAM at `0xF00000`, two MultiPCM-type sample
  chips and an OPN2-type FM chip (`docs/hw/model2.md` §5). It is not
  modelled, so the game is silent.
- *Confirmed (by running it):* the idle loops are the waits on the
  frame counter byte at work RAM `0x500000`, in the main loop
  (`0x1394`-`0x13A0`) and the boot screens' (`0x1B8B8`-`0x1B8C4`): 74 to
  78% of the i960's instructions are skipped in coin-start runs.

## Next

1. The test menu's input test: the brake, the wheel's direction, the
   shifter's bits; then a whole race (steering script) and its end.
2. The sound board (`docs/hw/model2.md` §5): a MultiPCM and an OPN2 model.
3. `vcop` and `desert` on the same board model (`vcop`'s chips have
   daytona's layout, sound board included).
4. The drive board (register 17's queue) and the link board.
