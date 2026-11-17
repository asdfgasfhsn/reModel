# Cyber Troopers Virtual-On (`von`, Model 2B-CRX, 1996)

Status: **plays** (listed; the launcher's Model 2B group, with a poster and
bezel), checked in scripted runs: boot, the attract mode in 3D, textured
(the anti-drug notice, the Sega logo, the title, the demo fights), coin-start
into the machine select, the first mission (a one-on-one fight on a
platform over the sea) to a draw at time-up, the continue countdown and
back to the boot notice, with sound. Not yet played by hand. Sega's
robot-versus-robot fighter, played with two sticks. On the board model it
needed one new thing: a **communication (link) board**, which the game
checks for on every boot and, without one, refuses to start a game on (a
minimal generic stub, `romset::LinkBoard::Idle`, answers it).

`von.zip` also holds its clone `vonj`, the Japanese version: its own
program pair `epr-18664b.15/epr-18665b.16` (the US pair is `vo-prog0.usa`
and `vo-prog1.usa`), the rest of the chips shared. Only the parent (US) is
supported; the clone isn't listed.

## What's known

- *Confirmed (ROM, then by running it):* the manifest verifies (22
  chips). The board is **Model 2B-CRX**: the screen at boot prints "MODEL2
  BCRX", downloads both SHARC programs ("Downloading COPRO prog", "GEO
  prog") and loads texture banks 0 and 1, and the program writes both
  SHARCs' host windows. The layout: program `vo-prog0.usa/.prog1.usa` (1
  MB) then `epr-18666.13/.14` (1 MB, which holds the copro SHARC program;
  both pairs are `Role::Program`, back to back); main data `.11/.12` then
  `.9/.10` at `0x2000000` and `0x2800000`; polygons `.17/.21`, `.18/.22`
  (4 MB chips fill their 8 MB slots); textures `.27/.25`, `.28/.26` (the
  pairings the other 2B games use; the 3D draws fully textured, other
  pairings weren't tried);
  copro data `.29/.30` (2 MB each) on the copro SHARC's bus; sound
  program `.31` and samples `.32`, `.34`. `epr-18643a.7` is the link
  board's Z80 program (below), never read by the i960. The emulator's .dat
  was used only as a catalogue of which names and CRCs belong to the
  parent and which to the clone.
- *Confirmed (by running it):* the geometrizer SHARC program (3,113 words,
  code hash `1ee99ebc8a458237`) is the one `indy500` and `fvipers` upload,
  so the native geometrizer implements it (`Program::Indy500`) and
  `make geo-check GAME=von` compares it with the chip; the copro program
  (3,679 words) is its own. Both run with no fault and no host error.
- *Confirmed (from the code, then by running it):* the **link board
  check**, a routine at ROM `0xC64A0` called each time the game's mode
  logic restarts (ROM `0x1898C`):
  - It writes 0 to `0x1A14000` and `0x1A14002`, waits a frame and reads
    `0x1A14000`: bit 0 set means **no board**. The game then prints "THERE
    IS NO COMMUNICATION BOARD!" and records `0xFF` at `0x5770B0`, and
    from then on neither the test switch nor a coin starts anything: the
    first coin blanks the screen for good (the sound goes on), and the
    test button does nothing. With the board absent the slot reads as all
    ones on the board model, which is that.
  - With bit 0 clear it waits 60 frames, writes 1 to `0x1A14000` and
    waits for bit 7 of `0x1A14002` to clear, clears two 1,792-byte
    buffers at `0x5024F0` and `0x5032F0`, and prints which site it is
    from the link ID setting (`0x5770B1`): 0 "THIS IS RELAY SITE", 1
    "MASTER", 2 "SLAVE", 3 "STANDALONE MACHINE". The setting is a byte
    in the battery-backed SRAM (`0x1D00028`): the game's own defaults
    (ROM `0x22C0`) set it to 2, which the check at ROM `0x18920` turns
    into 3, stand-alone (0 gives slave, 1 master). For **stand-alone**
    it stops there: no other register is touched and `0x5770B0` stays 0,
    the single-cabinet state (*confirmed by running it:* the game then
    runs as it did on a cabinet with no link).
  - For a linked ID it goes on: waits for bit 7 of `0x1A14002` to be set,
    writes the ID to `0x1A10001`, copies half of `0x1A10012` to
    `0x1A10014`, polls `0x1A10000` for the byte 1 ("Checking Network
    Now"), reads the node total at `0x1A10003` and succeeds only on 2
    ("Network Total Nodes: 2", then "Result : Node ID = ..."); any other
    count gives "Illegal Nodes". A second cabinet is the other node; a
    button (ROM `0xEBA28`) aborts the wait.
  - The model (`model2::link`, `romset::LinkBoard::Idle`): `0x1A14000`
    reads 0 (board fitted: bit 0 clear) and `0x1A14002` reads 0 (bit 7
    clear: not busy). That is all the stand-alone path reads. A game set to
    a linked ID would wait for the board's bit 7 and a partner that never
    come, as a real cabinet with nothing plugged in. *Unconfirmed:* what
    each register bit means beyond what the code tests; the board's own
    program is the Z80 chip `epr-18643a.7`, which no code here runs (the
    i960 program never reads it; `docs/games/sgt24h.md`).
  - Other games with a link slot (`overrev`, `stcc`, `sgt24h`) are
    stand-alone with the slot empty; `von` is the first that needs the
    board fitted.
- *Confirmed (from the game's own Input Test, by running it):* port 0
  (`0x1C00002`): COIN CHUTE #1 `0x01`, #2 `0x02`, TEST `0x04`, SERVICE
  `0x08`, START `0x10` (one start button: the test has no second); port 1
  (`0x1C00004`) the **left stick** and port 2 (`0x1C00006`) the **right
  stick**, each lit by its bit: SHOT `0x01`, DASH `0x02`, DOWN `0x10`, UP
  `0x20`, RIGHT `0x40`, LEFT `0x80` (bits `0x04` and `0x08` light nothing;
  port 3, `0x1C0000C`, is read and lights nothing). The sticks are
  **digital**: 8-way levers, not analogue (*unconfirmed:* the game also
  reads all eight analogue channels, `0x1C0001E`, into RAM at `0x502490`
  every frame, but its Input Test doesn't show them and no use was found).
  The test menu (TEST MENU: MEMORY TEST, T.G.P. TEST, INPUT TEST, OUTPUT
  TEST, SOUND TEST, C.R.T. TEST, GAME ASSIGNMENTS, COIN ASSIGNMENTS,
  BOOKKEEPING, BACKUP DATA CLEAR, EXIT) opens with TEST (a press in
  attract), "SELECT WITH SERVICE BUTTON AND PRESS TEST BUTTON": service
  steps the cursor, test enters.
- *Confirmed (by running it):* `coin-start` (one coin, start, then both
  levers forward, shots and a dash) goes through the machine select and
  the mission briefing (NEXT ENEMY) into the first fight, READY, the
  timer running down from 90 against a CPU opponent, a draw at time-up,
  CONTINUE?, and back to the boot notice. The attract mode's "GO AGAINST
  OTHER PLAYER!" banner is the game's challenge prompt for a cabinet
  that could be linked. *Unconfirmed:* the outcome of the fight (the
  script doesn't aim); head-to-head play needs a second cabinet on the
  link, which isn't modelled.
- *Confirmed (by running it):* the attract draws in 3D, textured. One
  frame of mission loading shows a screen of noise (a texture bank
  uploading; *unconfirmed*, one frame).
- *Confirmed (by running it):* the idle loop is the wait for the next
  frame at `0x29A3C`-`0x29A4F`, a poll of the video status's frame bit
  (`0x98000C` bit 2) counting passes in g0 (the game stores the count in
  `0x504C80`): about 64-71% of a coin-start run's instructions are
  skipped (`make bench GAME=von`: state hash `e0e990f07a540fe0`, with an
  `eeprom.bin` from an earlier run; the same with `--no-idle-skip` and
  after a save and load, `make state-check GAME=von`, whose audio matches
  too).
- *Confirmed (by running it):* **recompiled** (`make recomp GAME=von`):
  158,115 i960 instructions in 27,894 blocks (all it found in the program
  ROM and a coin-start run), 3,580 of the copro's 3,679 uploaded words
  and 2,983 of the geometrizer's 3,113. `make lockstep GAME=von RECOMP=1`
  runs 400M instructions of a coin-start run identical (the i960 and the
  copro SHARC recompiled; the lockstep reports the geometrizer as
  interpreted, the native one standing in for it), and the bench run gives
  the same state hash at 3.97x real time. Interpreted it runs at about
  1.2x real time. `make jit-lockstep GAME=von` (the i960 JIT beside the
  interpreter, with the copro JITs) runs 120M instructions identical.
- *Confirmed (by running it):* **input lag** (`make input-lag GAME=von`,
  the left stick held left from frame 2507 of a coin-start run in the
  first fight; also from 2907): the picture changes in the first frame
  that can read it, so `input_lag` and `run_ahead_max` are 0.
- *Confirmed (by running it):* **sound**: the 68000 and SCSP board runs as
  on the other 2B games; a coin-start run recorded with `--audio` (54 s)
  averages -26 dBFS RMS (peak -2.1), so `volume_db` is 0. The sound
  program `epr-18670.31` carries no driver version string (the CRX_drv
  string the later games have). *Unconfirmed:* by ear.

## Controls

On the keyboard the left stick is WASD and the right stick the arrow keys;
Q and E are the left stick's shot and dash, `.` and `/` the right
stick's; C is the coin and Enter the start. On a gamepad the d-pad is the
left stick and the face buttons (Y up, A down, X left, B right) the right
stick, the shoulders the shots (L1, R1) and the triggers the dashes (L2,
R2); Select is the coin and Start the start. **A gap:** the manifest's
controls bind a stick's four directions to buttons only (`PadButton`); an
analogue stick can't stand for a digital lever, so the gamepad's own
sticks aren't used. Binding a pad stick's directions to buttons (with a
dead zone) would fit these two levers better, and the Controls page has no
cabinet picture for twin levers (its cabinet pictures are the wheel and
pedals and the light gun): it shows the pad or the keyboard with the 17
buttons labelled.

## Next

1. Play it by hand; the twin-stick controls on a pad (the gap above).
2. Head-to-head play: the link board's protocol (shared RAM at
   `0x1A10000`, the node count at `0x1A10003`) and the Z80 program
   `epr-18643a.7`, which would let two instances be linked (the plan for
   linked multiplayer).
3. The analogue channels the game reads, and whether anything uses them.
4. The sound program's version, and the sample order against it.
5. The Japanese clone `vonj`.
