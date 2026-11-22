# The link (network) board

The communication board some Model 2 cabinets carry in an expansion slot
at `0x1A10000`, as **the games use it**: what the i960 reads and writes,
and what it expects back. Everything here comes from the games' own code
and from running them under reModel (no emulator's link code was read; the
board's own firmware is not run anywhere here). Each point is marked
*confirmed* (with how to see it), *inferred* (read from code but not yet
exercised) or *unconfirmed*.

`srallyc` (Sega Rally Championship, Model 2A) is the worked example: it
was taken through boot, a link handshake, attract, car select and a race
against a scripted partner (sections 1 to 8), and then **between two to
four real copies of the game in one process** (section 9, the two-machine
spike). Section 9 corrects two points of the first spike that a mirror
partner could not show: the node IDs are 1-based and **a node's own block
is the last of its received blocks**, not the first. Where an earlier
section says otherwise, section 9 wins (the places are marked). `daytona` (the original Model 2, up
to **eight** cabinets) was then run the same way (section 10; what is generic
and what is per game: section 11); both were then taken **between
machines on a LAN**, over UDP in lockstep (section 12; what internet play
would need: section 13); `von` and `overrev` were only looked at
(section 7), and then a dozen more games' test menus were walked for their link
settings (section 14); of the harder ones `von` (section 15), `stcc` (section 16)
and `skisuprg` (section 17) were then taken to linked races and matches. The 2A's slot is the same as the original board's
(`model2.md` section 2) and the 2B's.

## 1. Reproducing it

The experimental hook is off by default and changes nothing then (state
hashes are unchanged; `make bench-all`).

```
make link-trace GAME=srallyc LINK=idle            # board fitted, nobody on the link
make link-trace GAME=srallyc LINK=fake,0,2 INPUT=... STEPS=520000000 SNAP=100
                                                  # a partner: node ID 0 of 2 nodes
```

`LINK=idle` fits `romset::LinkBoard::Idle` whatever the manifest says;
`LINK=latch` the `Latch` board (section 17, `skisuprg`'s check);
`LINK=fake,ID,TOTAL` additionally answers the board start (section 3) and
**mirrors the game's own block back** as every other node's, so the game
sees partners in exactly its own state (`LINK_DELAY=N` makes the partners'
copy N frames late, `LINK_FREEZE=F` stops it from frame F, `LINK_DROP=F` has
the board stop reporting ready from frame F). Every CPU access to
`0x1A00000`-`0x1A1FFFF` is written to `OUT/link-trace.txt` as `frame ip R|W
address size value`. Under the hood these are `REMODEL_LINK`,
`REMODEL_LINK_DELAY`, `REMODEL_LINK_FREEZE` and `REMODEL_LINK_DROP`
(`remodel` usage); `REMODEL_RAM_DUMP=0x500000:FILE` and
`REMODEL_DISASM_IMAGE=FILE:0x500000` with `disasm` read the game's code
where it runs from (`srallyc` copies most of it to RAM at boot, so the ROM
listing has little of it).

The EEPROM's link setting is edited in the 128-byte `eeprom.bin` in the
run's `--out` directory (section 2); `srallyc`'s record carries a CRC-16.

Frame numbers below are `srallyc` runs with a fresh EEPROM, `coin-start`
not used (coins and start were scripted late, see section 5).

## 2. How a game decides that link play is on (`srallyc`)

Three things, all *confirmed* by running:

1. **The board's presence bit.** At boot (frame 9) the game writes 0 to
   `0x1A14000` and `0x1A14002` (byte writes), then reads `0x1A14000`: **bit
   0 clear means a board is there**, set means none. The result goes to a
   flag at `0x20A554`. Today's slot reads all ones, so this is 0.
   *Confirmed:* with no board, the game **resets the settings it can't
   honour**: its main loop (the routine at `0x5A2260` in RAM) sets LINK TYPE and
   CABINET TYPE to 0 and saves them to the EEPROM, so a linked cabinet's
   settings do not survive a boot with no board (a run with a fresh EEPROM
   that wrote defaults with CABINET TYPE 1 and ended with 0).
2. **The EEPROM setting LINK TYPE**, in the test mode's GAME ASSIGNMENTS
   page (the menu text is in the program: "LINK TYPE": `NOTLINK`, `CAR 1`,
   `CAR 2`, `CAR 3`, `CAR 4`, `RELAY`; also "CABINET TYPE": `DELUXE`,
   `TWIN`). It is byte `0x0B` of the 128-byte `eeprom.bin` image (the game's work
   RAM from `0x202010` is the record from its first byte, header included,
   so this is `0x20201B`): 0 NOTLINK (the default), 1 CAR 1, 2-4 CAR 2-4, 5
   RELAY. *Confirmed:* set to 1 (with the record's CRC-16 fixed) the game
   shows "COMMUNICATION BOARD CHECKING", "THIS IS MASTER CONTROLLER" on the
   board check screen; 2 takes the other branch ("THIS IS SLAVE MACHINE",
   and it does not write `0x1A14002`, section 3). Values 2, 3 and 4 all
   take the slave branch and 5 a relay one ("THIS IS RELAY MACHINE");
   *inferred:* where the CAR number then matters beyond that (the byte is
   read at a dozen sites outside the network code: car colour or start
   grid, *unconfirmed*). **CABINET TYPE** (byte `0x0A`, work RAM
   `0x20201A`, default TWIN = 1) is read at 17 sites; what it changes is
   *unconfirmed*, and it is zeroed with the link type when no board is
   found.
3. **Nothing else.** No DIP switch, input bit or cabinet-ID pin was found:
   the I/O registers the game touches are the usual ones (`0x1C00000`
   analog and ports, the EEPROM block at `0x1C00200`), and the master/slave
   and car selection is the EEPROM byte alone. (*Unconfirmed* for code the
   runs didn't reach.)

The settings record: 4-byte header (a CRC-16 in image bytes 0-1, then
`0x0024` as a length in bytes 2-3), then 32 bytes of settings; the game's
own copy is at work RAM `0x202010` from the first header byte. The CRC covers
image bytes 2 to 35 (the length word and the 32 settings): the usual CCITT
polynomial `0x1021` taken the high bit first, initial value `0xFFFF`, result
complemented, stored low byte first (the game's routine at `0x5C87E0`;
reproduced and checked against a game-written EEPROM, `remodel link-run`'s
patch of the link type). CABINET TYPE is image byte `0x0A`. (The first
spike's remark that the link type is "byte `0x0B` of the settings record"
meant this, header included; counted after the header it is `0x0F`, which is
a different setting.)

How a player sets it up: test mode, GAME ASSIGNMENTS, LINK TYPE one cabinet
to CAR 1 (master) and the others to CAR 2 to 4, with a board fitted. The
test menu's own entry is *inferred* from its text; the byte it edits is
*confirmed* as above.

## 3. The handshake and what the game does with the board

*Confirmed* by tracing the game against the `fake` partner and against an
idle board. The registers (all byte accesses by `srallyc`):

| Address | Use |
|---|---|
| `0x1A14000` | write 0 / 1; read: bit 0 set = no board. Writing 1 **starts the board**. Written 0 at boot and at each link setup |
| `0x1A14002` | write 0 at setup; the **master writes 1** (slaves don't) just before starting the board. `srallyc` never reads it (`von` and `daytona` read bit 7 as busy, section 7) |
| `0x1A10000` | read: byte **1 = the link is up** (board ready); anything else, not |
| `0x1A10002` | read once the link is up: this node's ID, **1-based: the master is 1, the slave of two 2** (*corrected, section 9*: the first spike's partner answered 0 for the master, which a one-machine mirror cannot tell apart, and two real machines deadlock with it) |
| `0x1A10003` | read once the link is up: the number of nodes (a signed byte) |
| `0x1A12000`-`0x1A121BF` | **the game's own block, 448 bytes**: written by the game |
| `0x1A121C0`-... | **received blocks**, 448 bytes each, four at most (`0x1A121C0`, `0x1A12380`, `0x1A12540`, `0x1A12700`), read by the game |

The sequence (game mode 1, a small state machine; times in frames of one
`srallyc` boot with the master setting and the `fake` partner):

| Frame | What |
|---|---|
| 9 | presence probe (above) |
| 924 | "COMMUNICATION BOARD CHECKING": writes 0 to `0x1A14000`, `0x1A14002`; prints the role |
| 925 | reads `0x1A14000` (bit 0 clear), the master writes 1 to `0x1A14002`, both write **1 to `0x1A14000`**. With bit 0 set instead: "Nothing Communication Board!" for 300 frames, then on to stand-alone attract (and the settings reset, section 2) |
| 926 | polls `0x1A10000` **every frame until it reads 1** (no timeout: an idle board was polled for 2,000 frames, to the end of the run). Then reads `0x1A10003`, `0x1A10002`, prints "Node ID" and "Total Nodes", "Network Check Success !" and **holds that for 300 frames** |
| 926 on | writes its block **every frame** from now on (next section) |
| 1227-1229 | **barrier 1**: waits until the status word of every node's block shows the same state (*node state 1*, section 4) |
| 1350-1352 | **barrier 2**: waits until all show state 2 |
| 1353 | game mode 2: attract with the link on (the title demo shows the "2 players can battle" screen) |

With bit 0 of `0x1A14000` clear and `0x1A10000` never reaching 1 the game
waits in "Network Checking !" for good.

For a **non-master** node the game copies the master's settings (four words
at offsets `0x08`, `0x0C`, `0x10` and `0x14` of the master's block, stored
in the game's settings bytes `0x20201C`, `0x20201D`, `0x202018` and the
last two) into its own and saves them to
the EEPROM if they differ, just after barrier 1 (code at `0x5AB9CC`;
*inferred* from the code, not exercised: the partner mirrored the master's
own settings).

## 4. The shared RAM

`0x1A10000`-`0x1A13FFF` is plain 16 KB RAM on the CPU side (as modelled, and
all `srallyc` needs). The game uses:

- **Its own block** at `0x1A12000` (448 = `0x1C0` bytes), written by one
  block copy every frame from a local structure (*confirmed*: 112 word
  writes per frame, from frame 926 to the end, in attract, select and the
  race alike).
- **Received blocks** at `0x1A121C0 + 0x1C0 * k`. **Corrected (section 9,
  *confirmed* with two, three and four real machines):** with the 1-based ID
  register, received block `k` of a node with 0-based position `i` holds the
  block of node `(i + k + 1) mod N`: the node's **own block, echoed, is the
  last one (block `N - 1`)**, block 0 is its ring successor. The game's two
  constants say so: it computes `0x20A584 = TOTAL - 1` (its own block, which
  each node compares with "the node whose turn it is") and finds the master
  at block `(TOTAL - ID) mod TOTAL`. (The first spike had the own block
  first, `(i + k) mod N` with a 0-based ID, and a mirror partner could not
  tell: every block was the game's own. With two real machines that layout
  hangs the link attract, section 9.) The game reads its own car's state
  back from the own block (*confirmed* in the first spike: a partner that
  delayed the echo changed the physics: the car's speed and gears went
  wrong).

The block, as observed (a Desert-stage race, offsets in bytes from the
block start; all words little-endian as the i960 stores them):

| Offset | Observed |
|---|---|
| `0x00` | u16 node ID (1-based, section 9: 1 the master, 2 the slave of two); `0x02` u16 car number (the LINK TYPE: 1 CAR 1, 2 CAR 2, ..) |
| `0x04` | u32 changes at car select (2 at select) |
| `0x08`, `0x0C`, `0x10`, `0x14` | in attract the master's four settings words (section 3); at car select `0x13`, a number `0x1469` that stays put through the race (*unconfirmed*: a race seed or id; the game reads it from both nodes' blocks, `0x1A121DC` and `0x1A1239C`, every race frame), `1`, `1` |
| `0x18` | **node state**: 2 in attract, 3 at entry, 4 car select or waiting, 5 start, 6 racing (then 7 at the end, *inferred*). The game's own names for them (debug strings): `NS_None 0`, `Init 1`, `Adv 2`, `Entry 3`, `Wait 4`, `Start 5`, `Play 6`, `Finish 7` |
| `0x1C` | u32 copy of the `0x1469` number |
| `0x24` | u32 1 while racing |
| `0x28`, `0x2C`, `0x30` | float32 **position**: x, height (0.3 m), z (the start grid at -368, 0.3, -310) |
| `0x34`-`0x54` | floats: the car's orientation or velocity (*meaning unconfirmed*) |
| `0x58`-`0x64` | four floats, 0.1 at rest (*inferred*: suspension) |
| `0x68`-`0x74` | four small negative floats |
| `0x78` | u16 `0x2222` in `0x7A` once racing (*unconfirmed*: a car-type marker); byte `0x78` of the receive block is **written back** by the game (`0x80`) every race frame |
| `0x7C` | u16 `0x012B`, u16 `0xFFFF` while racing (*unconfirmed*: lap or checkpoint) |
| `0x188` | u32 0, then 3 (*inferred*: checkpoint or lap) |
| `0x19C` | u32 the **race clock in frames, counting down from 3000** (50 s at 60 Hz: it matches the on-screen countdown); the game compares it between nodes |

The remaining words stayed zero in the runs. The game reads of a received
block each frame run from about 12 words in attract to about 100 in a
race (the position, orientation, wheel floats, the flags and the clock of
both blocks).

## 5. What is exchanged, how often, and what must arrive

- **Direction and size.** Each node sends one 448-byte block per frame, and
  reads its own and the others'. That is 26.9 KB/s a node at 60 Hz
  (*confirmed*: one block copy per frame, in every game mode from the link
  handshake on).
- **Frequency.** Per frame: one poll of `0x1A10000` (and, in the setup
  modes, `0x1A10002`/`3`), one block write, the reads above. There is no
  other register traffic after the handshake (*confirmed*: no write to
  `0x1A14000`/`2` after frame 925 in a 5,500-frame run with a race).
- **What has to arrive.** Only the handshake gates progress: the board
  start (`0x1A10000` = 1), and the two barriers on the nodes' state words
  (state 1 for all, then 2 for all). During the race nothing waits for a
  partner's data: the game uses whatever is in the block (*confirmed*: with
  the partner silent from frame 3000 the race went on with stale data, the
  position showed 2/2 where it had been 1/2, and no message or restart
  followed).
- **Delay tolerance.** *Confirmed:* with the partner's blocks arriving 3 and
  8 frames late (on top of the one frame the block already takes) the game
  boots, passes both barriers (they simply take longer), gets through
  select and races to the end of the run without a restart. Only the
  opponent's car is late, as it should be; own state must **not** be
  delayed (see section 4 on receive block 0). Whether the game *looks* right
  with D = 8 was not judged: only that it doesn't abort.
- **Lost link.** The game's per-frame poll (`0x5ABCB0`) counts consecutive
  polls where `0x1A10000` is not 1. After **more than 10** it acts: in
  the setup and attract modes (game modes 1 and 2) it clears the link flag
  and **restarts the game mode logic** (back to the "board checking"
  screen); in a race it **carries on alone**: *confirmed* by dropping the
  ready byte at frame 2800 during a race, the access pattern changed at frame 2812 (11 polls
  later: the link flag cleared) and by frame 3000 the display read `1/1`,
  the race continuing. Any other
  node's state word showing 1 (a node that restarted) also resets
  attract (*inferred*, code at `0x5ABD5C`).
- **Timeouts.** No timeout while waiting for the board (above); the 300-frame
  holds are fixed display times. There is no heartbeat or checksum in the
  block that the game checks other than the shared seed and clock
  comparisons.

## 6. What is confirmed and what isn't

Confirmed: the presence check, the EEPROM link and cabinet bytes and the
CRC, the master/slave write difference, the register meanings in the table
(from `0x1A14000`, `0x1A14002`, `0x1A10000`, `0x1A10002`, `0x1A10003`), the
block geometry (448 bytes, own at `0x1A12000`, received from `0x1A121C0`),
one block write per frame, own-state echo needed, delay tolerance of the
race, behaviour on a silent partner and on a dropped ready byte, the
barriers as they appear against a mirror partner.

Inferred: the node state numbering past 6 (*section 9: 7 was never seen, 8
is the finish*); the meaning of most block fields. (*Settings copy to
slaves is confirmed in section 9.* The inference that received block `k` is
node `(ID + k) mod TOTAL` was **wrong**, section 4.)

Unconfirmed: what a *different* cabinet writes (its ID field and car
number are inferred from the slave's own writes, not from a partner), how
the real board assigns IDs and how long it takes to bring the ring up,
RELAY, 3 or 4 cabinets, TWIN's effect, whether the real board has any more
registers (`srallyc` touches only the five above), electrical details of
the ring, whether the real board echoes the own block with no delay, and
what happens when a node's block repeats or skips a frame.

## 7. Daytona, Virtual-On and the others

A quick look only (`REMODEL_LINK=idle`, traces of a 1,380-frame run each;
their docs have the code-level detail):

- **Same register interface.** `von`: writes 0 to `0x1A14000` and
  `0x1A14002`, reads `0x1A14000` (bit 0), writes 1, waits for bit 7 of
  `0x1A14002` to clear (*busy*), and for a linked ID writes its ID to
  `0x1A10001`, polls `0x1A10000` for 1 and reads the node total at
  `0x1A10003` (`von.md`); it also reads `0x1A14002` **every frame** later. It
  clears two **1,792-byte** buffers (`4 x 0x1C0`: the same four-block
  geometry as `srallyc`'s). `daytona`: reads `0x1A14002` each frame in its
  network check (bit 7 set = no board / stand-alone; the idle board's
  zero leaves it polling) and shows "NETWORK BOARD NOT PRESENT" for a MASTER
  with bit 0 set. `overrev`, `stcc`, `sgt24h` and `indy500` run the same
  start check (`overrev.md`).
- **What differs.** The games' network layers are different code: `srallyc`
  reads its ID from `0x1A10002` and writes none; `von` writes its ID; the
  strings and states differ (`srallyc`: `NC_`/`NS_`; `von`: "Checking
  Network Now"). Whether the shared RAM layout beyond the `0x1C0`-byte
  blocks and the ready byte is common is *unconfirmed*; Virtual-On was not run past its handshake here (section 15 did it); Daytona was taken
  through a linked race with up to eight machines (section 10).
- **One board model can serve all of them**: the registers (present, start,
  busy, ready/ID/count) and a block exchange of `0x1C0`-byte blocks are
  common. A game-specific part is only what each game puts in the blocks,
  which the board does not look at.

## 8. Design sketch for linked play

What the board does, as a model: the CPU-side RAM is shared memory; each
node's TX block is replicated to every node's receive blocks, in a fixed
node order relative to the receiver, with the receiver's own block echoed
in receive block 0. The board does not interpret the blocks.

### (a) Two cabinets in one process

- A `LinkHub` owning the exchange, and a `Link` per machine as now
  (`model2::link`), with a *connected* kind in `romset::LinkBoard`. Each
  machine keeps its own `Model2Bus`; the hub is touched only at the frame
  boundary.
- **Per frame boundary** (the existing vblank point of `Model2Bus`): for
  each machine, copy its TX block (`0x1A12000`, 448 bytes) out, then, once
  all machines have reached the boundary, copy into each machine's receive
  blocks: its own into block `N - 1`, node `(i + k + 1) mod N`'s into block
  `k` (*as built and corrected, section 9*). The machines run frame-by-frame
  in lockstep (one thread each or alternately on one).
- **Handshake model** (`Link` register behaviour): `0x1A14000` bit 0 reads
  0; a write of 1 starts the board; node IDs by master first (the cabinet
  that wrote 1 to `0x1A14002` is the first, the others in order of start;
  the register shows the ID plus one);
  `0x1A10003` = the number of cabinets that have started; `0x1A10000` = 1
  once all declared have; each cabinet's start waiting on the others is
  fine, they poll without a timeout.
- Seen with the `fake` partner: one machine plus a mirror gets through the
  handshake, attract, select and race in this build (`make link-trace`).
  Two real machines add what a mirror cannot test: different node IDs and
  settings copying, a slave following the master's `0x1469`-style values,
  real barrier timing.
- **Player** (headless first): `remodel link-run GAME --cabinets 2` style
  tool that steps two machines and writes both frames' PNGs; the player
  window for two cabinets (side by side) is a second step, and costs two
  renderers (the GPU memory warning in `CLAUDE.md` applies).
- **Built as a spike (section 9):** `model2::link::LinkHub` and
  `remodel link-run` / `make link-run`; two, three and four real machines
  run attract, a race and the results together. The player view and two sets
  of controls are still to do (section 9, "What the next steps need").

### (b) LAN, one cabinet per host, fixed-delay exchange

Each host runs its own cabinet. At the end of frame N each host sends its TX
block (448 bytes, about 27 KB/s) and applies, at the start of frame N + D,
the block that the other sent at frame N. Both then see the other's block
**at the same emulated frame**, so each machine is deterministic given its
own inputs and the received blocks, replayable (record the received blocks
with the inputs) and save-state-safe (the in-flight blocks and the delay
line are part of the state).

- **Can D be 1-3?** Yes by the evidence: the game tolerates the other
  node's blocks 3 and 8 frames late (section 5). D = 3 is 50 ms, which a
  LAN meets with room; D = 1-2 needs a wired LAN. The handshake and the
  barriers just take D frames longer per round trip.
- **What the emulation must guarantee.**
  1. Capture of TX and application of RX **only at frame boundaries**, at
     one fixed point in the frame, so a block never changes under the
     reading i960 (the game reads fields at different times and has no
     consistency check) and the result does not depend on host timing.
  2. Receive block 0 (the echo of the own block) is **not delayed** by D:
     it is the node's own state, local to the host, available from the next
     frame on. Only the other blocks are late.
  3. D and the node count fixed for a session; the first D frames' receive
     blocks are defined (zeros, as the board powers up, or the handshake's
     first blocks) on both sides, identically.
  4. Lockstep pacing: host A does not start frame N until B's block for N - D has
     arrived; otherwise stall (or run ahead and roll back: the existing
     run-ahead and rollback work must treat received blocks as an input,
     with TX **not sent for speculative frames**).
  5. Everything the game reads from the link comes through the blocks and
     the board registers; the registers are a function of the exchange
     (the ready byte becomes 1 when all hosts have started, at the same
     frame number on all; `0x1A10002`/`3` fixed by session setup).
  6. A state-hash exchange every N frames for desync detection (each side
     hashes the machine; a mismatch means the determinism guarantee was
     broken, e.g. by a different ROM set or settings).
  7. The two hosts' **EEPROMs and settings must agree**: LINK TYPE
     master/slave per host, same difficulty, etc. (a non-master copies the
     master's settings at the barrier; a mismatch of the others would be
     overwritten at the first boot).
- **Lost link on a LAN.** A silent partner is not noticed by the game
  during a race (section 5); the host must decide: pause, or drop the ready
  byte so the game carries on alone (it then shows 1/1 and keeps racing).
  Dropping `0x1A10000` for more than 10 frames in attract restarts the game
  there, which is the right behaviour for a quit.
- Effort: given (a), about **a week**: a socket transport with the frame
  numbering and stall, session set-up (ROM and settings agreement, delay
  choice), record and replay of the received blocks, the state-hash check,
  and rollback integration if wanted.

### (c) Open questions

1. ~~What a real second cabinet's blocks contain~~ *Answered in section
   9*: ID (1-based) and car number fields, both nodes' words, the
   `0x1469` seed the same on both after car select (it is in both blocks
   from entry on, the game derives it, not the board).
2. ~~How the real board numbers nodes~~ *Answered for 2, 3, 4 cabinets
   (section 9)*; RELAY and TWIN are still open (RELAY with a CAR 1 ran
   attract and the results, section 9).
3. Whether the real board's echo of the own block is immediate or one
   frame late; the model chose "visible next frame" and the game ran
   (two to four machines, section 9).
4. (Section 9 made the exchange frame-synchronous, so this stays open.) Whether the game gets a block mid-frame torn on hardware (the real board
   is asynchronous): the game seems to have no consistency check, so it
   probably copes; frame-boundary exchange removes the question.
5. Daytona and Virtual-On's own block layouts and handshake details; a
   common board model with per-game parameters seems enough but is
   unconfirmed.
6. Whether `srallyc`'s race result and ranking logic work between two
   different clocks (the race clock is in both blocks and compared).
7. Run-ahead and rollback with the link: the `srallyc` manifest has
   `run_ahead_max: 1`; the receive blocks must be part of the rolled-back
   state, and a rolled-back frame must not have sent its TX.
8. 2A TGP data: no link dependence seen, but a replay of the link needs
   the game's TGP state identical on both hosts (deterministic already).

## 9. Two to four real machines in one process (the two-machine spike)

Step (a) of the design in section 8, built as a spike, off by default, with
no player UI: **`model2::link::LinkHub`** (the board: ready byte, IDs,
exchange at the frame boundary, a delay line) and **`remodel link-run`**
(`crates/cli/src/link_run.rs`; `make link-run`) which runs N complete
machines of the game in one process, a frame each in turn (the machines are
deterministic; the hub is touched only between frames), writes a PNG of all
screens side by side and logs the handshake, the node states and every
change in a block. A machine's EEPROM is built per node (the game's default
settings record, LINK TYPE set, CRC fixed: section 2; the user's files are
never read or written), so `--nodes 2` runs a CAR 1 and a CAR 2.

```
make link-run GAME=srallyc NODES=2 FRAMES=6000 SHOT_EVERY=100 \
    INPUT0="coin1@1500-1504,coin1@1560-1564,gas=0xff@3430-99999" \
    INPUT1="coin1@1500-1504,coin1@1560-1564,gas=0xff@3600-99999"
    # pictures in out/link-run/frames, the log in out/link-run/link-run.log,
    # blocks.txt: both nodes' blocks every frame
make link-run NODES=4 INPUT="coin1@1500-1504,coin1@1560-1564,gas=0xff@3500-99999" LINK_OPTS="--shot-at 3600,4000"
```

Options for experiments: `--delay D`, `--delay-to NODE:D`, `--skew`,
`--ids`, `--layout`, `--drop-at F`, `--freeze NODE:F`, `--set-byte
NODE:OFFSET:VALUE`, `--save-at F FILE`, `--load FILE`, `--trace NODE`,
`--dump-at F`, `--watch NODE:ADDR`, `--ram-dump NODE:ADDR:FILE`
(`remodel` usage; `link-run` prints its own).

### 9.1 What the two machines showed (all *confirmed* by running)

- **The handshake completes between two genuine games**: both write 0 at
  frame 924 and 1 to `0x1A14000` at 926 (the master first writes 1 to
  `0x1A14002`); the hub, seeing both started, makes the ready byte 1 at
  the boundary of frame 926; the boards screens ("COMMUNICATION BOARD
  CHECKING", "THIS IS MASTER CONTROLLER" / "THIS IS SLAVE MACHINE", "Node ID
  : 1" / "2", "Total Nodes: 2") show on both, and from 927 both send their
  block (state 4). Barrier 1 passes at 1228 (state 1) and barrier 2 at 1351
  (state 2) on both at the same frame. A start 150 frames apart
  (`--skew 0,150`) has both passing the barriers together, 150 frames
  later.
- **The first spike's layout hangs real attract.** With a 0-based ID and
  the own block first, both screens went blank after frame 1351 for ever
  (state 2 in both blocks, nothing wrong visible in the handshake): the
  link attract's synchronising word, **block offset `0x18C`** (*confirmed*
  as the node's `0x20A770`: 0 idle, **1 ready**, **2 go**), is a ring of
  acknowledgements (*inferred* from the code at `0x5AEE40`-`0x5AF300` and
  watching the words): which node must answer first depends on the cars'
  order among the received blocks, and a node writes its 1 at once only if
  the block of that car is at index `TOTAL - 1` (`0x20A584`), the own
  block (section 4). With the own block first, the CAR 1 node was left
  waiting on its own block for ever. Three nodes with the old layout let the
  slaves through and hung only the master. The layout of section 4 makes
  two, three and four nodes pass: `0x18C` goes 1 (all, frames 1360-1361),
  the go 2 (frame 1362) and back to 0 (1363) and the attract plays
  ("3人まで対戦可能!!", the ranking table) on every screen.
- **Both players coin up and start a linked race, and each sees the
  other's car.** From the frames of one run (D = 1, a coin script at frames
  1500 and 1560): coin at 1560 takes both nodes to state 3 at 1567 and car
  select (state 4) at 1569, the `0x1469` seed appearing in both blocks at
  1569 (not the board's: the game's); the course select, then the race: the
  selection times out at 3021 (1452 frames after 1569, no inputs given)
  into state 5 and the race (state 6) at **frame 3103**, the start grid at
  `x = -369` and `-375` (z -310, the ID 1 car in front), the race clock 3000
  counting down. The screens show `POSITION 1/2` and `2/2` swapping as the
  cars pass, a green and a red dot on each minimap, and the other car:
  ahead on the road, in the rear-view mirror when it is behind, and next to
  the car with its number sign in a four-car race. Four nodes show
  `x / 4`. The nodes in one race, at frame 4400 and D = 1: own state
  and the other's position agree (`-203.6, -7.0, 271.7` here and in the
  other's block).
- **The ranking and the finish work between the machines**: the race ends
  at frames 9205 and 9259 (state **8**, not 7; both retired in that run) with
  "RETIRED RANK 1st" on one screen and "2nd" on the other, and the nodes
  return to attract (state 2 at frame 9982) and run the `0x18C` handshake
  again (1, 2, 0 at 9988-9991). `0x18C` also moves in a race, on the
  master alone, at 5232 (1, back to 0 at 5293) and 7898 (2, 0 at 7959):
  *unconfirmed* what the game decides there (a lap or checkpoint report).
- **Settings travel from the master to the slaves during the first handshake**
  (the code at `0x5AB9CC`, section 3; not timed), and the slave **writes them to its EEPROM**:
  five bytes of the image, `0x08`, `0x09`, `0x0C`, `0x0D`, `0x0E` (the
  game's `0x202018`, `19`, `1C`, `1D`, `1E`) are copied; in the test a slave
  with `0x08 = 2, 0x0C = 3, 0x0E = 5` ended with the master's `1, 0, 1`, its
  CRC fixed and its other bytes alone (`--set-byte` and `eeprom-N.bin`).
  They travel in the master's block at `0x08`-`0x17` (four words, the last
  carrying `0x0E` in its low byte and `0x09` in the next; *inferred* from
  one test). The link type (`0x0B`) and cabinet type (`0x0A`) are not
  copied; each cabinet keeps its own. The slave's block carries zeros in
  those words, and the master does not mind.
- **Different inputs per machine just work**: each node's own pedals and
  wheel in its own car; the hub carries nothing of the inputs.
- **The own block can be delayed no more** (as before): the own block is
  the echo, immediate.

### 9.2 Timing, D = 1 to 12 (the other nodes' blocks reach a node D frames
after they were sent; D = 1 is as soon as the echo)

Everything is deterministic, so the table is from one scripted run each
(the same inputs; the whole game, handshake to a race at frame 4400):

| D | race starts (state 6) | the other's clock at 4400 vs own | the other car's reported z against its own at 4400 |
|---|---|---|---|
| 1 | 3103 | same | same (271.7) |
| 2 | 3104 | 1 frame older | 0.4 m behind |
| 3 | 3105 | 2 older | 0.5 m behind |
| 6 | 3108 | 5 older | 1.6 m behind |
| 12 | 3114 | 11 older | 3 m behind |

(The table is from the hub as first built. Since section 12 each node also sees
the others' board starts D - 1 frames late, so for D > 1 the ready byte, and with
it every frame after, comes up D - 1 frames later than listed; D = 1 is unchanged.)

The game boots, passes both barriers, selects, races and finishes at every
D with no reset, message or visible glitch in the frames looked at (3525 and
4400 of each run; only D = 1 was looked at closely). The other car is where it was D - 1 frames ago, as it should be;
whether D = 12 *looks* right (a car appearing late at the start, a mirror
image behind) was not judged beyond that.

Each extra frame of D delays the barriers and so the race start by a
frame. The attract's `0x18C` ring takes D frames longer per round (1360 to
1363 at D = 1, 1368 to 1383 at D = 5): the game's own limit, the poll of the
ready byte, is not involved. **The game has no timeout on its handshake and
no complaint about a stale partner at any of these delays.** An asymmetric
delay (`--delay-to`, one node getting the other late and not the reverse)
behaves the same way.

### 9.3 Reproducibility and save states

- **The combined run is reproducible**: the same inputs give the same
  combined hash (an FNV of both machines' state hashes and the hub's state)
  across runs, on the interpreter, on the JIT and with `--single-thread`
  (`744c2f5f4a0c2da5` for a 4,800-frame run with a race, two machines, with the hub as first built: the combined number folds the hub's saved state, which changed in section 12, so it differs now while the machines' own hashes (at D = 1) do not;
  `make link-run` prints it). A different D gives a different hash, as it
  must.
- **Saved together, the two machines resume exactly**: `--save-at 3900 F`
  then `--load F` from the middle of a race ends on the same combined hash
  as the straight run. What is saved: both machines' `save_state`, the
  hub's state (ready flag, IDs, the delay lines) **and each bus's board flags**
  (`started`, `master`: the registers' record of what the game did, which is
  not part of a machine's own state). With only the machines saved and the
  board's state lost (`--fresh-hub`), the hub never sees "started" again, the
  ready byte falls to 0, and both games leave the link (a race goes on alone
  as `1/1`, attract restarts the board check): the board is part of the
  state, and the in-flight blocks must be saved with the nodes in any
  rollback or save scheme.

### 9.4 What breaks (and what does not)

- **Own-block placement and ID base** (above): the one real defect found,
  invisible to a mirror.
- **Dropping the ready byte**: in a race both carry on alone as `1/1`
  (`--drop-at 4000`), from frame 4000 on; in the attract (`--drop-at
  1700`) both restart at "Network Checking !" and wait for the board for
  ever. The hub as built keeps the board down; a hub that brought it up
  again would let them handshake again (*untried*).
- **A node that stops sending** (`--freeze 1:4000`, its blocks stale): the
  other shows `1/2` and carries on, and the frozen node's car just stops
  moving in its picture; nothing resets.
  *But not for the rest of a session* (section 12.3, from `link-run --leave`):
  with the leaver's last block repeated for ever, `srallyc`'s race clock (it
  follows the other nodes' clocks) stops and the race never ends, and
  `daytona` waits at its next circuit select for the leaver's entry. What the
  hub shows of a node that has left is a copy of the others' state with the
  leaver's own car (12.3).
- **A node booted late**: fine (the nodes just wait in the board check).
- **RELAY** with a CAR 1 (link types 1, 5): both run attract and the
  results, without the ring; RELAY with CARs was not tried.
- **Three and four nodes** (link types 1, 2, 3 and 1 to 4): the handshake, the
  attract's ring and a race run, each node seeing the others (`x / 4`).
- Host speed: a pair of machines runs about 700 frames a second on the JIT
  (a 4,800-frame run takes 7 s, a race to the finish 15 s), about ten times
  real time, with pictures drawn only at the frames asked for.
- **Not tried** (srallyc; Daytona is section 10): the real board's behaviour when its ring breaks; the TWIN
  cabinet type's effect; the 2-player screen layouts of the TWIN cabinet.

### 9.5 What the next steps need

- **Player view: done** (`remodel-play --players N`, `make play GAME=srallyc
  PLAYERS=2`, the launcher's Link play; `docs/play.md`, "Link play"): N
  machines and one `LinkHub` stepped in lockstep from the frame loop, each
  machine on its own thread for the frame (the frame is the barrier, the hub's
  exchange the only thing they share), one renderer each on one GPU device
  drawing into its view of the window, one controller each, the set's own
  EEPROMs (`out/srallyc-link2/eeprom-N.bin`; LINK TYPE set per machine from
  the manifest's `romset::LinkPlay`) and saves (every machine, the hub and
  the board flags in one file). The combined hash of a player run equals
  `remodel link-run`'s for the same inputs (section 9.3). What the LAN step
  can take from it: `link::LinkSet::step` is where the exchange sits (one
  frame boundary, all machines' blocks in, all received blocks out), and
  `LinkSet::save`/`load` are the state a rollback needs.
- **LAN**: each side runs one machine, sends its 448-byte block at the
  frame boundary and gets the other's `D` frames late into its block
  `(i + k + 1) mod N` with its own echo in the last. Two things from this
  spike matter: the **ID register is 1-based and the layout is own-last**
  (section 4), and the **board's flags** (started, master) are part of the
  state both sides hash and save. D = 1 to 12 all worked, so D can be
  chosen from the measured round trip. Everything else is section 8(b):
  about **a week** on top of the player view; the in-process hub is the
  model to copy (`LinkHub::exchange` with one remote source), and a
  save-state check (section 9.3) is the test.
- The rollback or run-ahead work needs the hub's delay lines in its state
  and no sending from a rolled-back frame (section 8(c) 7), which the
  `LinkHub::save` / `load` already shape.

## 10. Daytona USA (`daytona`, the original Model 2): two to eight machines

The same hub and `remodel link-run` run `daytona` (`make link-run GAME=daytona
NODES=8`), through the per-game data in `crates/romset/src/games/link.rs`
(section 11). All of it is from running the game and reading its program (no
emulator's code); each point is marked *confirmed* (seen in a run), *code*
(read from the program, not exercised) or *unconfirmed*. Frames are from the
scripted run `coin1@1100-1104,start@1300-1304,gas=0xff@1500-1510,
gas=0xff@1700-1710,gas=0xff@2400-99999` on every node, D = 1, with the
settings made by `link-run`; they are the same for 2, 4 and 8 nodes unless
said.

### 10.1 The board and how link play is enabled

- **Same slot, same registers** (*confirmed*, `link-run --trace`): the
  original board's slot is the 2A's (`model2.md` section 2). The game
  touches exactly the registers of section 3 (`0x1A14000`, `0x1A14002`,
  `0x1A10000`, `0x1A10002`, `0x1A10003`) and the blocks from `0x1A12000`.
- **The setting is on the I/O board**, not in an EEPROM like srallyc's:
  the 128-byte record the game fetches with command 3 at boot and saves with
  1 and 2 (`model2.md` section 2; the model keeps it in the bus at
  `0x1C00200`, byte N at word N). The record is "SEGA", a version, a
  CRC-16 at `0x08` (CCITT polynomial, start 0, **not** complemented, over
  bytes `0x0A`-`0x7F`: *confirmed* by recomputing the manifest's record),
  then:
  - **`0x0B` LINK ID**: 0 SINGLE (the network check is skipped), 1
    MASTER, 2 SLAVE (the game's own default is MASTER). *Confirmed:* the
    test screen at boot shows "LINK ID MASTER" or "SLAVE", and the board
    check "THIS IS MASTER CONTROLLER" or "THIS IS SLAVE MACHINE" (a third
    string, in the table at `0x1B948`, is for the value 0, *code*). With no
    board (`0x1A14000` bit 0 set) a linked cabinet shows "NETWORK BOARD
    NOT PRESENT" and restarts (`daytona.md`).
  - **`0x0C` CAR NUMBER**, 0-based, shown +1 (*confirmed*: setting 1 shows
    "CAR NUMBER 2"). Whether the game uses it beyond showing it is
    *unconfirmed*: with every node left at CAR NUMBER 1 two nodes
    link, play the selection and finish a race as with 1 and 2 (the first
    runs of this section), and the node ID decides what the exchange does.
    `link-run` sets it to the node's index all the same. In the 3D runs
    (below, CAR NUMBER = index) the cars take different colours (red, blue,
    yellow, green, pink, cyan, orange ..); whether it follows the ID or the
    CAR NUMBER was not separated.
  - Cabinet type (TWIN by default) and the rest: *unconfirmed*. No DIP
    switch or input bit for linking was found.
- **How many cabinets** (*confirmed*): **eight**. The program checks the ID
  register (`0x1A10002`) and the node count (`0x1A10003`) to be 1 to 8 (the
  routine at `0x1B5F4`) and reads eight received blocks each frame. Two,
  four and eight machines were run through to the results.

### 10.2 The handshake

The routine at `0x1B5F4`, *confirmed* by the access trace and the screens:

| Frame | What |
|---|---|
| 186 | writes 0 to `0x1A14000` and `0x1A14002` (byte writes) |
| 187 | reads `0x1A14000` (bit 0 set: "NETWORK BOARD NOT PRESENT", "CANCELLED"); clears its local 448-byte copy (`0x540C00`); **writes 1 to `0x1A14000`**; the **master then writes 1 to `0x1A14002`** (srallyc writes `0x1A14002` first; the order does not matter to the hub) |
| 188 on | every game frame: zeroes its own block (`0x1A12000`), waits for **bit 7 of `0x1A14002` to flip** (below), reads `0x1A10003` and while that is non-zero counts down **320 frames** ("NETWORK CHECKING", a blinking caption) |
| 528 | the count ends; `0x1A10000` must read 1, `0x1A10002` (ID) 1-8 and `0x1A10003` (count) 1-8, or an error ("CANCELLED") and the game leaves the link; else it sets its link variables and sends a block every frame from then on |

- **New register behaviour: bit 7 of `0x1A14002` flips every frame**
  (*confirmed*: with the idle board's constant 0 the game stays in "NETWORK
  CHECKING" for ever). The model flips it with the frame parity for boards
  joined to a hub (`Model2Bus`), one flip per vblank,
  which is what the loop's parity test needs; srallyc never reads the
  register, so nothing changes there. The vblank handler also reads
  `0x1A14002` every frame into a game variable (`0x540054`), whose use was
  not found. What the real bit is (a frame token, a ring token, busy) is
  *unconfirmed*. The 320-frame count only counts frames in which the node
  count is non-zero: a ring that has not formed just keeps the game
  waiting (no timeout, as srallyc).
- **Ready, ID, count**: `0x1A10000` = 1 is read once at frame 528 and then
  **every frame** (the link-loss check at `0x1B0D0`; the behaviour on a loss
  is *code* only, not exercised). IDs are **1-based** (master 1) and the
  count is the number of cabinets, as srallyc (section 9).
- **Barriers**: the game has no barrier words. Each node's state is in the
  header of its block (next section) and the nodes go on together because
  the master's block says so. A late-started node (`--skew`) was not tried
  on daytona.

### 10.3 The exchanged block

- **Size, place, rate** (*confirmed*): a **448-byte (`0x1C0`)** block at
  `0x1A12000` written **once a frame** from frame 528, from a local copy at
  `0x540C00` and the car table; **eight** received blocks at `0x1A121C0 +
  0x1C0 * k` are read every frame (the loop at `0x1A960` covers all eight;
  those past the node count are zeros). Eight blocks end at `0x1A12FC0`:
  the 16 KB holds them (a unit test pins it). The same geometry as srallyc.
- **Order of the received blocks** (*confirmed*: both screens show their
  own and the other cars, `POSITION 1st/2`, `2nd/2` and the cars' order on 4
  and 8): **the same own-last order as srallyc**. The game finds the block of
  the node with ID X at index `(TOTAL - 1 - MYID + X) mod TOTAL` (`0x1AA60`,
  `0x3FA8`, `0x1B760`), MYID the register's value. The node's own block is
  the last one (X = MYID), and only the difference X - MYID counts, so
  `LinkHub`'s layout 3 serves both games.
- **Header** (the first `0x40` bytes; the game keeps one copy per node at
  `0x540C40 + 0x40 * k`). From the dumps at frames 1200 and 3000
  (`link-run --dump-at`):
  - `+0x3C` a **checksum**: the sum (mod 2^32) of the other 15 words (*code*:
    the sender stores it, the receiver at `0x1A960` recomputes it and
    **keeps its previous copy of that header when it differs**, so a torn
    block is ignored, a stale one harmless). Not exercised with a bad block.
  - `+0x0A` the node's **state** (2 and 3 at 528/530; `0x24` to `0x28` at
    1108-1119; 6 to 9 at 3136-3142, ..); `+0x0B` the master's value is the
    **node count**, the others' 1 (*unconfirmed* reading); `+0x0C` the
    node's own ID; `+0x08` a 16-bit number that falls from `0x3BF` after the
    course select (*inferred*: a timer); `+0x2F` a mask of the nodes
    entered (3 on the master of two, 1 on the slave).
  - The **master's block carries the race entry** from frame 3140: `+0x00`
    to `+0x07` a grid order (7 6 5 4 3 2 1 0 for eight nodes) and from `+0x0C`
    the roster of IDs (1 2 3 4 ..); the other nodes' blocks have zeros there
    (*confirmed* on 2, 4 and 8; the exact meaning *unconfirmed*).
- **Body**: `+0x40` to `+0xE7` (`0xA8` bytes) the node's own car state copied
  from the game's car table (`0x1A8B8`): position, orientation and wheel
  numbers; each node recomputes every car's pose from the received ones
  through the TGP (`0x1AA18`, `0x1AB94`); from `+0x120` 4-byte entries per
  car slot (*code*). The rest is zero. There is no sequence number:
  staleness shows only as a car that lags.

### 10.4 What a linked Daytona does (*confirmed*, frames of the scripted run)

| Frame | Every screen |
|---|---|
| 188 | the hub makes the link ready (both nodes started the board at 187) |
| 190-528 | "NETWORK CHECKING" and the role line |
| from 528 | the **link attract**: a title card "communication system: up to 2 people can battle" (fixed text, also with 8 nodes), then the usual attract |
| 1100 / 1300 | coin; start: **CIRCUIT SELECT** (a 12 s timer) |
| 1500-2400 | **MISSION SELECT** (Beginner, Automatic, a 3 s timer) |
| about 2600 | "以上に決定しました" (decided as above) |
| about 3000 | **"GENTLEMEN START YOUR ENGINES"** |
| 3136-3142 | node states 6 to 9; the master's block carries the grid and roster |
| 3200-4400 | ROLLING START with "AUTO DRIVE", the 3 shown at about 3950 |
| from about 4400 | the race: `POSITION 1st/2`, `2nd/2` (`x/4`, `x/8`) from the nodes' positions; every node shows the others' cars (eight cars in eight colours in the 8-up picture) and, on the HUD, the other nodes' lap times |
| by 13500 | **RESULTS 1st / 2nd** with total time and best lap on each screen (1'09"25 and 1'09"29 on the two nodes of the two-machine run) |
| about 14500 | back to the Daytona USA title |

Speed: 4,000 frames of two machines take 10 s on the JIT (26 s on the
interpreter); 3,200 frames of eight take 34 s.

### 10.5 The race's 3D scene (was missing from a straight run; fixed)

**What was seen** (*confirmed*): in a straight run the race's HUD, timers,
positions, laps and results worked but the 3D scene was not drawn (the sky,
the grass and the HUD), on 1, 2, 4 and 8 nodes; a save at frame 3200 and a
load drew it from the next frame, a save at 3100 or earlier did not.
LINK ID SINGLE inside `link-run` was fine.

**The cause** (*confirmed* by bisecting a load: the same save loaded into a
fresh machine, one subsystem at a time swapped back from the straight run;
only the geometrizer's made it fail): the geometrizer hung **at frame 525**
(the network check) and again at 3137 (the race entry), for good.

- At the vblank of frame 525 the game is in the middle of rewriting the
  list in buffer RAM (the title card's list is one buffer, not the double
  buffer of a normal frame): the write pointer is `0x24`, nine of the
  header's fifteen words are new and the rest are the old header's
  (`0xFFFF0080`, a window word, among them). The geometrizer is started
  with a copy of buffer RAM at the vblank (it does not see later writes), reads
  the half-new list and meets that window word as a command. A list that is
  half written when the frame starts is the game's own timing (the check
  that fills the frame is the link's), and is *unconfirmed* as to what the
  real board's geometrizer does with it.
- The native geometrizer (`geo_native_2c`) answered a word that is no command
  with `Stop::Hang`: the program of a 2B/2C geometrizer loops for ever on
  one (the dispatch table's "error" entries), so nothing more is drawn until
  the next upload. That is right for a program, but the 2A and original
  Model 2 geometrizer is fixed-function (`Program::Fixed`) with no program to
  loop in, and it was given the same permanent hang. The picture stayed
  without 3D for the rest of the run, whatever the game then sent.
- That state, `Native::stop`, **was not in the save state**: a load made a
  new native geometrizer, which started without it. So a save taken **after**
  the hang "fixed" the scene: the loaded machine was simply not hung. The
  game's list is half written again at **frame 3137**, the race entry (the
  node states 6 to 9 at 3136-3142, when the check's code runs again), so a
  load at 3100 or earlier hung again there and ended on the straight run's
  hash, and a load at 3200 or later (after both) never did. Everything else
  (work RAM, link RAM, video RAMs, settings) was identical, as the
  comparison found: the machine's own state was complete, only this flag
  was missing.
- Why only linked: single-player Daytona never has a half-written list at a
  vblank (its list is double buffered and built early in the frame); the
  link check's code (the 320-frame count, the block exchange and checksums)
  fills the frame in link mode and the title card's single list was caught.

**The fix:** the fixed-function geometrizer **ends the frame's list at a word
that is no command** and reads the next frame's list afresh (*unconfirmed*
as hardware behaviour, but a program-less chip has nothing to hang), and a
program geometrizer's hang (and a failed command check) **is now carried by
the save state**. A linked race now draws in a straight run, 2 nodes at D = 1
from frame 3201 on, and a save at any frame (1900, 3100, 3200, 3300) loaded
ends on the straight run's combined hash (`make link-state-check`).

### 10.6 Delay, determinism and the state

- **D = 1, 2, 3, 8 and 16** (two machines; the other node's block arrives D
  frames after it was sent, the own block is the echo): the handshake, the
  selection screens and the race entry work at every D, each frame of D
  shifting the 320-frame count and the state steps (state 2 at frame 528,
  528, 532, 540, 556; race entry at 3136, 3136, 3138, 3142, 3150). In the
  rendered race (10.5) D = 8 shows both cars, `1st/2`, `2nd/2` and the
  lap times of both at frame 6000. No reset, no message: **no timeout and no
  complaint about a stale partner** at any D, as srallyc. D = 16 was only
  looked at by its states, not by its picture.
- **Determinism** (*confirmed*): two machines, 4,000 frames, the script above:
  the combined hash (an FNV of the machines' state hashes and the hub's
  state, as in section 9.3) is `f848578ec39435d3` on the interpreter, the
  JIT, with `--single-thread` and with both. A different D gives a
  different hash.
- **Save and load** (*confirmed*): `--save-at F FILE` then `--load FILE` ends
  on the straight run's combined hash for a save at any frame (checked at
  300, 1900, 2600, 3000, 3100, 3200 and 3300; `make link-state-check`). The
  hub's state
  (ready, IDs, delay lines), each bus's board flags and the settings (they
  are in the machine's memory) are saved; the heartbeat is the frame parity
  and needs no state. Without the hub's state (`--fresh-hub`) the ready byte
  falls and the game leaves the link, as srallyc.

### 10.7 What differs from srallyc

- The settings are in the **I/O board's record** at `0x0B` (LINK ID 0, 1, 2)
  with its own CRC, not the 2A's EEPROM LINK TYPE (1-4); a separate **CAR
  NUMBER** byte.
- **Eight** cabinets, not four.
- The handshake **needs `0x1A14002` bit 7 to flip every frame** and counts 320
  frames before it asks for the ready byte, the ID and the count; the
  master's `0x1A14002` write comes after the `0x1A14000` one (srallyc
  never reads `0x1A14002`).
- The block: the same size, address and own-last order; a checksummed
  **header** (state, count, ID, the master's grid and roster) instead of
  srallyc's plain fields, an own car section copied from the car table.
- No barrier words (srallyc: the `0x18C` acknowledgement ring and node
  states 1 and 2): the states are in the header and the master leads.
- A frame's list can be half written at the vblank (10.5), which the
  geometrizer must survive.

## 11. What is generic and what is per game

| | Generic (the board, `LinkHub`, `link-run`) | Per game (`romset::LinkPlay`, `crates/romset/src/games/link.rs`) |
|---|---|---|
| Registers | `0x1A14000` (present, start), `0x1A14002` (master flag; bit 7 flips with the frame parity on a hub, harmless where it is not read), `0x1A10000` ready, `0x1A10002` ID, `0x1A10003` node count | `id_base` (the register holds the 0-based node + this; 1 for both games) |
| Ready, IDs | ready when every node has written 1 to `0x1A14000`; the master first, the others in order of start; a fixed D for a session | nothing |
| Exchange | at the frame boundary: each node's own block out, the others' in D frames later, the own block echoed at once, in the **own-last** order `(ID + k + 1) mod N` | the block size (`block`), the own block's offset (`tx`), the first received block's (`rx`), the node limit (`max_nodes`: 4 srallyc, 8 daytona) |
| Settings | `link-run` builds each node's image: the default record, the role byte, the CRC | where they live (`SettingsStore`: the EEPROM for srallyc, the I/O board record for daytona, the backup SRAM for von), the default record, the role byte's offset and its value for node 0, 1, .. (`role_off`, `roles`), a cabinet-number byte (`car_off`), the CRC (`RecordCrc`: range, position, start value, complement) |
| Logging | handshake, state changes, block dumps | the state word and position in a block (`state_off`, `pos_off`; none known for daytona) |
| Left open | | what is in a block; what the game needs to see before it goes on (srallyc's two barriers, daytona's 320 flips); what a lost or stale link does; any further register |

What a LAN protocol must leave per game: the **block size and the number of
blocks**; the **settings** each host must set (the master's and slaves' role
byte, a number per cabinet) and **agree on** (srallyc copies the master's
settings, daytona's header carries the master's grid and roster); the **ID
base** and the **received-block order** (own last, in both so far); the
**register behaviour** a game waits on (daytona's flip); the **handshake
length** (about 1,350 frames to srallyc's link attract, 528 to daytona's);
and the block's contents, which the exchange does not look at, **except that a
daytona block has a checksum that the receiver uses to drop a torn one**, so
a block must arrive whole. In short, a block-exchange service with per-game
constants and a ready/ID/count board.

## 12. Linked play over a LAN (`remodel-linknet`; `remodel link-net`, `remodel-play --host/--join`)

Each machine on the network runs its own linked cabinet and the link
board's blocks travel between the hosts over UDP. Built as the lockstep
with a fixed delay of section 8, with the changes below; the numbers are
from loopback runs of two real processes (`make link-net-test`).

### 12.1 What was built

* **One node per host, any number of hosts** (2 to the game's `max_nodes`:
  4 srallyc, 8 daytona), a full mesh of UDP between them. One host hosts and
  owns node 0 (CAR 1, the master); the others join by `IP:port` and are
  numbered in the order they joined. The protocol carries a node list per
  host, so a host with several local nodes (split screen plus LAN) fits the
  wire and the hub (`LinkHub::apply` takes any set of local nodes); the
  command line and the player run one.
* **The hub changed to make it so** (`model2::link::LinkHub`). Section 9's hub
  made the ready byte and the IDs a global fact read from the buses at the
  boundary; over a network a node cannot read another node's board flags at
  the same boundary, so the flags travel with the blocks. What a node *shows
  the link* at a boundary is now a record, `NodeRec` (the `started` and
  `master` flags, the start frame, and the block while started), and each
  node sees **its own record of frame F and the others' records of frame
  F - (D - 1)**: the ready byte, the IDs and the received blocks all come
  from that view. With D = 1 this is section 9's behaviour to the bit
  (checked: `link-run` state hashes of three srallyc machines over 4,400
  frames are unchanged); with D > 1 the ready byte comes up D - 1 frames
  later than before (the other nodes' starts are seen late like their
  blocks), so section 9.2's race-start frames move by about that. A node
  also sees "ready" at its own frame, not a global one: two machines can
  come up one boundary apart and the game does not notice. `remodel
  link-run` runs the same hub, all nodes in one process, so **the same
  inputs and D give each node the same machine state over a network and in
  `link-run`**; this is an automated test (`tools/link-net-test.sh`: two
  processes, each node's state hash equal to `link-run`'s, srallyc and
  daytona, clean and under loss and jitter).
* **D** (protocol 5: it can change mid-session, 12.7) is the hub's delay as before (1: the others' blocks of the same
  frame, which a network cannot give except by waiting a trip every frame;
  2: of the frame before, and so on). The host picks it at the start from the
  measured round trips: `ceil(one-way / 16.7 ms) + 1`, the one-way time being
  half the worst round trip (between two joiners, the sum of their halves to
  the host), at least 1; `--delay N` forces it. On loopback it comes out as
  **2** (a round trip under a frame still costs one frame plus the one); the
  test runs use 2 and 6.
* **Lockstep and stalls.** A host runs a frame, makes its record and sends it,
  then passes the boundary if the others' records of frame F - (D - 1)
  are all in; if not it **waits** (no guessing: a stall shows as a held
  frame; `NetLink::boundary` returns `Stalled` and the player tries again
  on the next redraw). The first D - 1 boundaries need nothing. A host
  is never more than D - 1 frames ahead of the slowest, so the hosts run at
  the speed of the slowest (a slow machine slows everyone: lockstep's price).
* **Settings.** The master's settings record (srallyc's EEPROM image, where
  the player has one; daytona's lives on the I/O board and starts from the
  manifest's default) is sent at the start and every node builds its own
  image from it (role byte, cabinet number, CRC) exactly as `link-run`
  does. Each joiner reports a hash of the image it will run, and the host
  refuses to start if one is not what it expects (checked: a joiner with one
  settings byte changed is refused, with both hashes in the message).
* **Session agreement** (the lobby): a joiner's `Hello` carries the game id (a
  set id, or a variant's own id), a hash of every chip the game loads
  (protocol 4; below), the reModel version (the crate version) and the
  protocol version; the host answers `Welcome` or `Refuse` with a plain
  reason (another game, other ROMs, another version or protocol, lobby full,
  session already started). The host's `Start` carries D, the node
  assignment, the host table (addresses), the check interval, the timeout and
  the settings; the joiners answer with their settings hash; then `Go`.
  Every step is repeated until answered, so the setup works through loss.
  **The ROM hash** (`linknet::rom_hash`) is an FNV-1a over the set's id and then
  each chip in manifest order (so by socket and role) as its role, its length
  and its content: never a file name, which clones share and users rename. Two
  sets that differ in one data, polygon, texture or sound chip, or two variants
  of a title (whose ids differ), do not agree, where protocol 3's hash covered
  the program ROMs only. It is taken once when the session is configured
  (tens of milliseconds); the same value is the `rom` key of the mDNS TXT record,
  so a host is refused as "different ROMs" before joining. The protocol
  version is 4 for this change: a peer of version 3 gets the version refusal
  and never compares hashes of another meaning. Protocol 5 (12.7) adds the
  delay change to the record and the held share to the data packet: a peer of
  version 4 gets the same refusal.

* **Names and discovery (as built later).** The protocol is version 2: the
  `Lobby` packet carries every player's name (host first) so a joiner sees who
  is in, and the `Info` answer to a broadcast `Discover` carries the host's
  reModel version, the ROM hash and D beside the game, name, players and port.
  A host's name and every name in a packet is cleaned on receipt (printable,
  at most 16 characters) and two players with one name are told apart on
  display only ("Sam", "Sam (2)"); names are not in any state or hash. A
  `Hello` of another protocol version is refused with the two versions in the
  message, and a joiner that hears any packet of another version from its host
  says so instead of waiting for its 5 second timeout. Discovery has a second
  way beside the broadcast: DNS-SD over multicast DNS (`crates/linknet/src/mdns.rs`,
  the `mdns-sd` crate): service `_remodel._udp.local.` on the host's real
  port, instance `NAME GAME PORT`, TXT keys `proto`, `game`, `ver`, `rom` (the
  hash, 16 hex digits), `players`, `max`, `free`, `d`, `state` (`open`, `full`,
  `starting`) and `name`. The host re-registers (an update) when the TXT changes,
  withdraws the service when the lobby is full or starting and unregisters on
  `close` and on drop; a joiner browses and merges its answers with the
  broadcast's by address (a host heard at several addresses, or both ways, is
  one entry; the address on our own subnet is preferred). A host can be
  refused before joining (version, ROM hash, game, full) from the record alone.
  The responder shares UDP 5353 with other mDNS software; where it cannot
  start, the broadcast alone is used. *Unmeasured:* Wi-Fi access points that
  filter multicast, a second physical machine.

### 12.2 The packets

UDP, `RMLN` + protocol version + kind; at most 1,400 bytes; everything
little-endian and bounds-checked, and **a malformed packet is dropped whole,
never trusted** (sizes, node ids, host indexes, frame ranges; the session id
and the source address must match the table). Kinds: Hello, Welcome, Refuse,
Lobby, Ping, Pong, Start, StartAck, Go, **Data**, Bye, Abort, Discover, Info (protocol 2 as above), **Relay** (protocol 3: see 12.3).

A **Data** packet carries the sender's newest frame, an acknowledgement per
host (the frame up to which the sender holds all of that host's records), the
sender's last eight checkpoint hashes, **the leave reports it knows** (`(leaver, reporter, frame held)` triples, 10 bytes each, only while someone has left: 12.3) and **every record of the sender's that
the receiver has not acknowledged**, oldest first. Records are whole (the
block is never split: daytona's block checksum would drop a torn one), coded
against the same node's block of the frame before (a run of unchanged bytes
is two bytes; an unchanged block costs about 15). So loss costs nothing but a
repeat: the next packet carries the same record again. A packet is sent when
a record is made, again after 12 ms while unacknowledged, 6 ms after
receiving something (to carry the acknowledgement if no record of ours does)
and every 100 ms at least. A record that arrives twice or late is kept once
(the first); the 5% duplication of the test runs shows as "duplicate
records" in the stats and does nothing.

### 12.3 Desync, players leaving, the end

* **State check.** Every N frames (default 60, `--check-every`) each host
  hashes **every node's record of that frame as it holds it** (flags, start
  frame, block) and sends the hash in its packets; a mismatch with another
  host's stops the session on both with `out of sync with player P at frame
  F: our state hash X, theirs Y`. Each host's *machine* differs (it is
  another node), so there is no machine hash to compare; what must agree is
  the records every host feeds its hub, and a damaged, misdecoded or
  misdelivered record shows there. (Checked: a record damaged in memory at
  frame 1200 is reported at frame 1200 on both hosts.) For a node that has
  left, the records after its drop frame are the hub's synthesis, so the
  hash covers the records they are made from and **the drop event itself**
  (node and drop frame): two hosts that agreed different drop frames differ
  at the first checkpoint after it. A different build is caught earlier by
  the version check, other ROMs by the ROM hash, other settings by the
  settings hash. A real mismatch is the only thing that ends a running
  session; the host that finds it says `Bye` with the reason, which ends
  the others at once.

* **A player leaves: the others go on** (protocol 3). A host *leaves* when
  it says `Bye` (a quit, `finished`), when it is silent for T seconds
  (default 5, `--timeout`), or when another host says it has left (the
  reports below ride in every data packet, so a lost `Bye` does not leave
  one host waiting for the timeout). The survivors then agree on the leaver
  P's **drop frame F** and carry on with P's records from F + 1 made by the
  hub:

  1. *Report.* The moment a survivor S notices, it writes down `h_S`, the
     frame up to which it holds all of P's records, and **takes no more of
     P's records** (what P still sends is ignored; nothing of P's is
     compared in the state check any more). The report `(P, S, h_S)` is
     added to every data packet S sends, together with the reports S has
     been told of (gossip), so it is repeated until the end and a lost
     packet costs nothing.
  2. *Agree.* When every host still in the session has reported on P, F is
     the **largest report**. Every survivor computes the same F from the
     same set of reports. (A host that leaves while this is going on is
     dropped from the set that must report; its report still counts if it
     is known.)
  3. *Relay.* A survivor that holds P's records up to F sends the others
     what they lack as `Relay` packets (kind 15: origin node, first frame,
     the records coded as in `Data`, up to F, repeated until the receiver's
     acknowledgement of P's records reaches F; the acknowledgements in `Data`
     already say what each survivor holds of every host). A survivor takes
     relayed records only after it knows F and only up to F.
  4. *Synthesis.* From F + 1, the hub's record of P at a frame is made from
     P's record F and the records of the survivors at that frame (below).
     It does not depend on when anyone noticed or when F was agreed.

  *Why it agrees (the lockstep argument).* A survivor passes boundary b only
  if it holds the others' records of frame b - (D - 1), so it has consumed
  at most the records it holds: `consumed_S <= h_S <= F` (it takes no more of
  P's after reporting, so `h_S` is final). A record of P beyond F is held by
  no survivor and so was consumed by none, and the hub never asks for one:
  it uses record F instead, whatever arrives or is lost afterwards. The
  record F is held by the survivor whose report is the maximum and the hub
  needs, for any boundary, only the records up to F (relayed to any survivor
  that lacks them) and F itself. So the sequence of records each survivor's
  hub sees, and so its machine, is a function of the records and F alone.
  Lockstep also bounds how far apart the survivors' `h_S` can be: a host is
  at most D - 1 frames ahead of the slowest, so P's records can be D - 1
  frames ahead of the slowest survivor's view and the reports differ by about
  that many plus what loss held back: the relay is a few records in a normal
  drop (`link-net --starve` holds 3 frames back at D = 6 and the survivor
  gets 12 records relayed in `make link-net-test`).

  *The synthesis* (`LinkHub::leave`, `Ghost`; the same code for `link-run
  --leave NODE:F` and the player). **Freezing the record (`Ghost::Freeze`, what
  `--freeze` does: P's car stops and its last block stays) does not work over
  a whole session**, from runs of `link-run --leave 1:5000`, two nodes:
  `srallyc`'s race clock follows the other nodes' clocks (the survivor's own
  clock stopped at 1921, 1923 in the frozen block, from frame 5000 on) so the
  race never ended (the picture still raced at frame 11,000, the baseline run
  had the results at 9,726); and `daytona` ran to the results and the next
  circuit select but waited there for ever for the leaver's entry
  (`state 0x12000001` unchanged to frame 19,500) where a mirrored leaver went
  on to the next race at 18,440. Dropping the ready byte (`Ghost::Down`,
  `--drop-at`: the lone survivor carries on as `1/1`) lets `srallyc` finish
  its race (results at 9,740, the game restarts its board at 10,452) and then
  waits in "Network Checking" for ever (section 9.4); with three nodes it would
  split the survivors. What works is a **mirrored leaver** (`Ghost::Mirror`,
  the default): from F + 1 the leaver's block is a copy of the lowest-numbered
  node still playing (a node plays up to its own drop frame) **except for the
  bytes that are the leaver's own** (per game, `LinkPlay::ghost_keep`:
  `srallyc` the node and car numbers `0x00-0x03` and the car's pose and wheels
  `0x24-0x7F`; `daytona` the role and ID bytes `0x0B-0x0C` and the car body
  `0x40-0xE7`, with the header's checksum word `0x3C` recomputed) and its
  flags (started, master, start frame), which stay as they were. The leaver
  thus keeps the game's own state, clock and barriers in step with the
  survivors (every node's state in the barriers, the race clock, the finish),
  and its car stays parked where it was, the cabinet gone from the race. Copying the
  car's pose as well (`link-run --ghost-keep 0:4` only) puts the ghost car
  on the player's: the race then ended at 11,666 in the wrong state (7), so
  the pose is kept. With one survivor the leaver mirrors that one.
  *Runs* (two and three nodes, one leaving at frame 5,000 in the race, then
  on through the results, the attract, a second coin, the car select and the
  second race):

  | | leaver | race ends | next attract | next select | second race starts |
  |---|---|---|---|---|---|
  | `srallyc`, frozen | node 1 of 2 | never (clock stopped) | - | - | - |
  | `srallyc`, ready dropped | both of 2 | 9,727 / 9,740 | board restarts, waits for ever | - | - |
  | `srallyc`, mirrored | node 1 of 2 | 7,009 | 7,733 | 10,369 | 13,038 |
  | `srallyc`, mirrored | node 2 of 3 | 9,730 / 9,744 | 10,454 | 10,457 | 13,128 |
  | `srallyc`, mirrored | node 0 (the master) of 3 | 9,703 / 9,704 | 10,428 | **stays in the entry state 3** | - |
  | `daytona`, frozen | node 1 of 2 | 13,082 | 14,431 | 14,817, then waits | - |
  | `daytona`, mirrored | node 1 of 2 | 13,082 | 14,431 | 14,817 | 18,440 |
  | `daytona`, mirrored | node 2 of 3 | 16,932 | 18,281 | 18,330 | beyond the run |

  *The master leaving* (the host is node 0 and the master): the mirror makes
  the master's block a copy of a slave's, and the games' next car select is
  led by the master (the master goes to state 4 two frames after all are in
  3 and the slaves follow it): with the master mirroring a slave, each waits
  for the other. So in `srallyc` the survivors finish the race, the results
  and the attract, and then **wait at the next select for ever** (`link-run
  --leave 0:5000`, three nodes: state 3 from frame 10,429 to the end of the
  run at 15,000). The slaves cannot become masters (the role is the game's
  own setting). *Unconfirmed*: the same for `daytona` (the survivors of
  `--leave 0:5000` with three nodes were still in the race, both cars against
  a wall, at frame 19,500; the race had not ended). The host process itself
  is not special after the start: the mesh is the hosts' addresses from the
  `Start` packet, so the survivors never depend on it.

* **A lone survivor** carries on exactly as above (it mirrors itself): its
  game goes on with the leaver's parked car in the field (`POSITION 2/2`
  against the parked car in the run's picture), through the results and the
  next game, and the player shows "Sam left - racing on alone". There is nothing
  to wait for, and the traffic falls to the heartbeat. A lone survivor's
  scripted run (`--frames`) ends when it reaches the frame, without waiting
  for anyone. (Not the same as dropping the ready byte, which is what
  `--drop-at` does: there it plays `1/1` for the race and then waits in
  "Network Checking" for ever.)
* **What is not handled** (the session says so or stalls visibly, and a
  player can always leave from the Esc menu): a survivor that is lost
  *after* it reported on P, holding records of P that no one else has, which
  the others then agree as F (they wait for records that cannot come; after
  3 timeouts the session ends with `the records of P up to frame F are lost`);
  a network that splits (each side drops the other: two sessions that go on
  apart); rejoining; hosts that disagree on who left for the first frames
  (a survivor timing out P while the others still hear it: the gossip makes
  them all drop it).
* **The end of a scripted run**: a host that reaches `--frames` waits (at
  most 3 s) until every other host still in the session has acknowledged its
  last record, then says `Bye finished`; the others see a player leave whose
  last record they hold, pass the boundaries up to `--frames` and stop.

### 12.4 Save states, pacing, run-ahead

Save states are **disabled** in a network session (the player says so; the
resume state is not kept either; nothing of the player's own settings is
written). The design allows them later: a save would have to be taken at the
same frame on every host (agree a frame F; each saves its machine and its
hub's records at the boundary of F) and a load would start every host from
the matching files together; not built. Smooth motion and run-ahead are off,
as in same-screen link play. The window shows a status line (players, D,
round trip, how many frames were held and for how long, WAITING while
stalled more than a quarter second).

### 12.5 Measured (two processes, loopback, one shared machine at `nice` 10)

From `make link-net-test`: the scripted race to frame 4,400 (srallyc) or
4,600 (daytona), each host's hash equal to `link-run`'s for its node.

| | srallyc | daytona |
|---|---|---|
| each host's state hash equals `link-run`'s for its node: D = 2 clean, D = 6 under faults | yes, 4,400 frames | yes, 4,600 frames |
| D chosen by the host on loopback (round trips of 0.1 to 0.3 ms in the lobby: under a frame, so `ceil(..) + 1`) | **2** | **2** |
| paced at 60 frames/s (`--realtime`), clean, D = 2: boundaries that had to wait | 1 of 4,400 (0.5 ms) on the host, 0 on the joiner | 1 of 4,700 (6.5 ms), 0 |
| bandwidth each way, clean (one packet a frame, **191 bytes a frame**) | 11.2 KB/s at 60 frames/s | 11.2 KB/s |
| paced, 5% loss, 5% duplication, 5 ms + up to 50 ms jitter on what each host sends, D = 6: waits | host 48 of 4,400 (1.1%, 561 ms in all, longest 45 ms), joiner 0 | not run paced |
| bandwidth each way under those faults (every unacknowledged record rides in each packet: 5% duplicate records is 19,000 repeats in 4,400 frames) | 17 KB/s | 22 KB/s (unpaced run) |
| unpaced (the hosts run as fast as they can, so they wait for each other every few frames: the waits are the machines' own speeds, not the network), clean, D = 2 | 60-150 waits of 4,400, 12-31 ms in all, longest 1.4 ms | 97-316 of 4,600, 26-62 ms, longest 4 ms |
| unpaced, the fault run, D = 6 | about 1,100 waits (10 s in all, longest 72 ms) | about 900 (9 s, longest 88 ms) |
| 12 clean daytona runs in a row | every one equal to `link-run` (after the fix below) | |
| state check: a record damaged at frame 1200 | reported on both hosts with the frame and both hashes | same |
| a settings byte changed on a joiner | the host refuses to start, naming both hashes | same |
| a player leaves mid-race (polite, killed, the host; two and three hosts) | the others notice at once (`Bye`) or after the 2 s timeout (kill), agree the drop frame, relay what one lacks and run to the last frame, each survivor's hash equal to `link-run --leave NODE:F` for the drop frame it logged (section 12.3) | same |

The round trip shown in a paced run (16 to 19 ms) is mostly the other host's
own frame period: a ping is answered when the other host next pumps its
session, once a frame. The lobby's, polled every 2 ms, is the network's own.

One bug of this work, for the record: a host that finished its last frame
first said goodbye, and the other treated the goodbye as the end of the
session before using the records that had already arrived, so a run ended a
frame short (and its hash differed) about one time in five. A host now passes
every boundary its records allow and ends only when it stalls for want of
what will never come (a desync or a lost connection still stops at once).

### 12.6 What a two-PC test should check

* The D chosen (`remodel link-net` prints the round trips and D at the
  start) and whether the picture is held (stalls: the stats line, or the
  window's status line).
* A player leaving mid-race (close one machine's window, pull its cable):
  the other(s) should show "NAME left - racing on" or "NAME's connection was
  lost - racing on", the other car parked, and the race, results and (any
  player but the master) the next game should go on. The line `drop frame of
  NAME: F` in `remodel link-net`'s output is the frame to give `link-run
  --leave NODE:F` to compare hashes.
* A race to the finish between the two machines, then the results screens
  (the end of a race is the one place the game's behaviour has not been seen
  over a delayed link at all: section 10.4's results were seen at D = 1).
* Whether the hash line of a scripted run (`remodel link-net ... --frames N`
  on each; `remodel link-run --nodes 2 --delay D` for the same inputs) is
  equal node by node on real hardware: the clean test of determinism across
  two CPUs and two builds.
* Firewalls: the host's UDP port (default 47890) must be reachable; the joiner
  needs none opened (the host's replies come back to it); with three or more
  hosts every host's port must be reachable by the others (the joiners' ports
  are the ones their first packets came from).
* LAN discovery finds hosts on the default port only, and only where the
  network passes broadcast (guest Wi-Fi often does not).

### 12.7 A delay D that follows the network (protocol 5)

D is no longer fixed for the session. The session host (host 0) may change it
while the game runs, by an announcement that every host applies at the same
frame, so each machine's link view, and so the game, stays what `link-run
--delay-at` gives. What was built, and why it is safe:

**What a change is.** A record can carry `D becomes D' from boundary F`
(`NodeRec::change`; flag bit 3 in the record's wire flags, then `F` as u64 and
`D'` as u8; D' 1 to 60, F at least 2). It is the host's record of frame h, and
`F = h + D + 1` with D the delay in force at h. The announcement is *part of
the record*, so it travels by the same path as the block: acknowledged,
resent in every packet until the receiver holds it, relayed with the rest of a
leaver's records, covered by the state check (`hash_recs` hashes it when
present, so two hosts that disagree on a change differ at the next checkpoint).
Nothing else was added to the wire for it.

**Why no host reaches F too early.** A host passes boundary b only when it
holds the others' records of frame `b + 1 - D`. The announcer's record h is
therefore in every host's hands before it passes `h + D - 1`, and F = h + D + 1
is two boundaries later: every host learns of the change from the record it
needs anyway, with no timing assumption and no separate message to lose. The
hub schedules the change the moment it takes the record in
(`LinkHub::put`), before the next `has_all` asks what the boundary needs. One
change is in flight at a time (the host announces the next only once F is
passed), and F is always beyond the last scheduled one.

**What each node's view gets** (`LinkHub::schedule_delay`, `view_frame`). Before,
at boundary b the node sees its own record of b and the others' records of frame
`b + 1 - D`. Across a change at F (D to D'), the others' view frame is
`max(b + 1 - D', v)` for `b >= F`, with `v` the view frame of boundary F - 1:
a view never goes back. The ready byte, the node IDs and the started flags
come from the same view frame as the blocks, so they follow likewise. The
node's own record is unaffected.

| change | what the others' records do at F and after |
|---|---|
| raise by k (D' = D + k) | no new record for k boundaries: the view stays on the frame it had at F - 1 (the same blocks are written again), then moves a frame a boundary as before; the others' data is k frames older than it was, and never goes back in time |
| lower by k | the view jumps k + 1 frames at F: the k frames in between are never shown to the node (their records are held and checked, not used) |
| same D | nothing |

For example D 3 to 5 at boundary 20 gives the view frames 15, 16, 17, 17, 17,
18, 19 for boundaries 17 to 23; D 5 to 4 at 30 gives 24, 25, 27, 28 for 28 to 31
(frame 26 is skipped). `a_delay_raise_holds_the_view_and_a_lower_skips_a_frame`
(`crates/model2/src/link.rs`) checks both.

**Why the survivors agree after a player leaves** (the argument of 12.3). A
survivor that passed F had consumed the announcer's record of at least h, so
`h <= h_S <= F_drop` for the leaver if the announcer is the one leaving: the
record is among those every survivor holds (or is relayed one: a relayed
record carries the announcement) and all of them schedule the change; a record
beyond the agreed drop frame is held by no survivor, so no survivor applied it
(`crates/linknet/tests/delay.rs`: the announcer quitting two frames after its
record, with the next survivor starved of its records so that only a relay
brings the change; a player leaving between the announcement and F; a player
vanishing at F, under loss).

**The decision** (`crates/linknet/src/adapt.rs`; the host only, from a rule
that never touches the game). Every boundary adds to a one-second-bucket meter
of how many boundaries had to wait. Every host reports its held share of the
last four seconds in each data packet (a byte), so the host sees a stall that
only a joiner suffers. Also the round trips of the pings (the last 40, raw)
give a 95th-percentile one-way delay (half the round trip), asked for in
frames plus one.

* *Raise* when over 10% of the last four seconds' boundaries (at least 120)
  were held, by this host or by a peer: D + 1, or D + 2 when over 30% were held,
  and at least what the pings ask for up to two frames beyond; or when over 3%
  were held and the pings ask for more than D. Clamped to 8. Held boundaries
  are the evidence; the pings alone never raise D (a paced loopback link's
  ping reads one frame because the peer answers once a frame).
* *Lower* one frame at a time, only after 25 seconds with no change and 20
  seconds with at most 0.5% held, nothing held by any peer and the pings' D
  below the current one, never below 2 (a fixed D can be 1).
* *Hysteresis.* After a change nothing is decided for 4 seconds and the meters
  start empty. A raise within 60 seconds of a lowering doubles the wait before
  the next lowering (up to eight times; forgotten after 150 quiet seconds).
* Pinned: the `net_delay` setting (Input delay 1 to 6, default Automatic) and
  `REMODEL_NET_DELAY` keep D as it starts; `link-net --delay D` without
  `--auto-delay` does too.

**Test hooks, logs.** `link-net --delay-plan F:D,...` (host) announces D at the
host's frame F; `--auto-delay [MIN:MAX]` turns the rule on. Every host logs
`delay D -> D' from frame F` as it learns it, and at the end a line `delay
schedule (for link-run --delay-at): F:D,F:D`, which `remodel link-run
--delay-at F:D,...` takes to reproduce the run. The player prints the same
changes on its console and the status line shows `Delay 3 -> 5 frames` from the
announcement until F, then `Delay 5 frames`.

**What the game does across a change.** *Confirmed* (`make link-net-test`, daytona and
srallyc, two processes): D 3 to 6 to 4 to 5 to 3 at frames 1,804, 2,607, 3,305 and
3,906, across the boot, the link check and the race start, under 5% loss and 50 ms
jitter, and the adaptive rule's own raises and lowerings on a link with Wi-Fi-like faults:
no machine dropped out of the link, nobody was "waiting for the other
machines", the race clocks and positions kept going (pictures in
`out/link-net-test/captures`), and each host's state hash equals `link-run
--delay-at` with the schedule the hosts logged. The unchanged-D run's hash differs, so
the schedule is part of what the state is. Changes one boundary or six apart (the
startup of the wifi run raised twice in a row before the decision waited out the
last change) were fine too. Players leaving with a change in flight (announced three
frames before the leaving frame, with F at the drop frame; the announcer itself leaving
two frames after its record): the survivors' hashes equal `link-run --leave NODE:F
--delay-at`. *Not confirmed:* games other than the two, a human's feel for the
controls when D moves, and a raise or lower while a track-select or results screen is
timing the barriers (the changes above came during the boot, the link check and the race,
not the results and next-race screens).

**Measured** (daytona, two `link-net --realtime` players, 4,600 frames, the joiner behind
a UDP proxy: 10 ms + up to 30 ms each way, 3% loss, an 80 ms spike for 250 ms about every
8 s; both start at D 3 from the lobby's round trip):

| | fixed D 3 | `--auto-delay` |
|---|---|---|
| D schedule | 3 | 3 to 4 at frame 125, to 5 at 816, to 6 at 2,336 |
| boundaries held (host / joiner) | 992 / 1,067 (21.6% / 23.2%) | 93 / 128 (2.0% / 2.8%) |
| time lost to holds | 7.1 / 7.5 s | 1.4 / 1.5 s |
| longest hold | 95 / 81 ms | 77 / 64 ms |

On a clean loopback link D stays 2 with or without the rule (no boundary held in 4,600, no
change). The price is the controls: each frame of D is 16.7 ms more between a press and its
effect on the others' screens; the player's own car is not delayed (its own record is
immediate; the others' cars are shown D - 1 frames late).

## 13. Internet play, next (not built)

What the LAN design has that internet play can keep: lockstep with a fixed D
(only D grows: a 60 ms round trip needs D = 4, 120 ms D = 6, 200 ms D = 9;
the game's own timeouts did not complain at D = 12, section 9.2, and the
price is D - 1 frames of input-to-picture latency over a cabinet's own);
whole-record packets and redundancy against loss (each packet carries
everything unacknowledged, which copes with the 5% loss and 50 ms jitter the
loopback tests inject); the state check, the settings agreement and the
version and ROM checks; the clean end; deterministic machines (so a
spectator, or a replay, is a node that only listens).

What internet play would add:

* **NAT traversal.** Hosts behind NAT cannot be reached. UDP hole punching
  through a rendezvous server (each side sends to the other's public address
  and port as the server reports them) works for most home NATs; the
  protocol already identifies a peer by session id and address table, and the
  start packet carries the table, so the table would be filled with the
  *public* addresses the rendezvous gives. Symmetric NATs need a relay.
* **A relay fallback.** A small server that forwards a session's packets
  between hosts (they still only exchange records; the relay needs to know
  only whose session id is whose), at the cost of the relay's latency. The
  mesh becomes a star at the relay; the packet format does not change (the
  source-address check becomes a relay-authenticated host index).
* **A rendezvous point.** Where hosts register ("srallyc, 1 of 2, a name")
  and joiners find them: the lobby's `Info` already is the listing record
  (game, name, players, maximum, port); discovery by broadcast becomes
  discovery by asking the server. A short session code in place of an
  `IP:port` is the user-visible gain.
* **Security.** The LAN protocol trusts its network: the session id is random
  but travels in the clear and a bystander on the path could inject a packet
  (it would break a state check, not the host, but still end a session).
  Internet play wants an authenticated, encrypted channel (a session key from
  the rendezvous or an out-of-band code, a MAC on each packet), rate limits
  on the lobby (a host answers `Hello` and `Discover` from anyone), and care
  that **a block from another player is data fed to the game's own code**
  (the games read other nodes' blocks without much checking: daytona's
  checksum guards tearing, not malice), so hosts should only link with people
  they chose. The bounds checks here keep a packet from harming the host,
  not the game's memory.
* **Larger D and jitter handling.** D chosen from a longer measurement (the
  median round trip plus the jitter, re-measured) and perhaps changed between
  races; the 12 ms resend and 100 ms heartbeat timers are LAN-sized (resend
  after about one round trip instead); a late-join or rejoin story; and, at
  D above about 6, the case for **rollback** (the modernisation track) over a
  pure wait.
* **Pace.** Lockstep makes every host run at the slowest; over the internet
  one hiccup is everyone's, so a small buffer of frames (run ahead of the
  network by a frame or two, then catch up) and a visible "waiting for
  player 2" help; the status line already counts holds.

## 14. Other linked games: survey

Which other games here can be linked, what their own test menus say, and
how far each is from the `srallyc` and `daytona` support. Everything was
found by running the games (no emulator's code or link implementation was
read); each point is marked *confirmed (by running it)* or *inferred*. The
pictures of the test pages were kept out of the tree.

### 14.1 Method

- **The test menu.** `test@1300-1330` (TEST held) opens the test menu from
  attract (the menu is up by about frame 1400; its cursor starts on EXIT);
  `service@A-B/20` steps the cursor down every 20 frames (it wraps from
  EXIT to the first row, and the rows of a page count one a line, a
  multi-line setting such as `von`'s PLAY TIME 1P counting each of its
  lines); `test@F` selects a page or changes the value under the cursor
  (one press, one step; the values cycle). Counts differ per game and are
  in each game's doc. Frames are as for `make run INPUT=`; the pictures
  came from `remodel run` snapshots.
- **Where a setting lives.** Two runs from a fresh `eeprom.bin`: one that
  changes nothing, one that changes the value once, **both leaving the test
  menu through EXIT** (`manxtt` saved to the EEPROM only when the test
  menu was left, not when the value changed or the page was left: a first
  attempt that stopped before that saw no difference; the others were run
  the same way from the start) and compared byte by byte (`cmp -l` of the
  two `eeprom.bin` files); `von` keeps it in the backup SRAM instead, seen with
  `REMODEL_WATCH=0x1D00028` (it writes on each press).
- **Which interface.** The game's setting at a linked value, the test menu
  left (the game then restarts its mode logic and its start check), run with
  `REMODEL_LINK=fake,0,2` (`make link-trace`) and the accesses to
  `0x1A00000-0x1A1FFFF` read from `link-trace.txt`. The `fake` board answers
  the board start of section 3 but its `0x1A14002` never flips bit 7, so every
  game below stops at its first wait on that register: *what comes after it was
  not seen for any game but `srallyc` and `daytona`*, except where a hub run
  is mentioned.

### 14.2 The table

Max machines is what the game's own menu offers (*confirmed (by running it)*
for the menu's range) unless marked inferred.

| Game (board) | Link setting: name, where stored, range | Max machines | Interface | Effort | Confidence |
|---|---|---|---|---|---|
| `manxtt`, `manxttc` (2A) | LINK TYPE: EEPROM byte `0x0B` (`srallyc`'s record kind: CRC-16 at 0 (CCITT, start `0xFFFF`, complemented), length word `0x38`, CRC over bytes 2-55): 0 Not Link, 1 Master, 2 Slave. BIKE COLOR (No.) is a second setting | 2 by the menu (Master, Slave); the hub ran 2, 3 and 4 and the game showed POSITION n/N | Same board and start as `srallyc`/`daytona`, then reads `0x1A10000`/`2`/`3`, writes `0x1A10001`; blocks of `0x1C0` at `0x2000`, received from `0x21C0` | **Built** (data only: `LinkPlay` entry, with free play, COIN/CREDIT SETTING #27: bytes `0x04`, `0x24`, `0x2E`) | Setting and interface: confirmed (by running it); linked in `link-run`, split screen and `link-net` (14.7) |
| `von` (2B) | NETWORK LINK ATTRIBUTE: backup SRAM byte `0x1D00028`: 0 SLAVE, 1 MASTER, 2 NOLINK (default); the SRAM record has a table-driven CRC-16 (section 15.1) | 2 (the game rejects any other node total) | Same registers; writes its own ID to `0x1A10001`, polls `0x1A10000`, reads the total at `0x1A10003`, waits for bit 7 of `0x1A14002` to be set (the hub's flip does it); **one 1,792-byte block** (`0x700`) at `0x2000`, the other node's at `0x2700`. Z80 ROM `epr-18643a.7` in the set | **Built** (section 15): a third settings store (`SettingsStore::BackupRam`) and a checksum kind (`SumKind::Table`), no hub change | Setting, record, handshake, block and a two-player match: confirmed (by running it). Linked in `link-run`, split screen and `link-net` (section 15) |
| `stcc` (2C) | LINK TYPE: EEPROM byte `0x10` bits 5-7 (`SEGASTCb` record, CRC-16 at `0x08`, counter at `0x0C`): 0 STAND ALONE, 1-7 CAR 1-7 (all seen) | 7 (CAR 1-7; the hub ran 2, 4 and 7) | `von`'s wording ("THIS IS MASTER SITE"), then the start of `srallyc` and a poll of `0x1A14002`; writes its ID to `0x1A10001` and the sizes `0x0E00` and `0x01C0` to `0x1A10012`/`0x1A10014`; `0x1C0`-byte blocks at `0x2000`, received from `0x21C0` | **Built** (section 16): the record and its checksum (CCITT, start `0xFFFF`, complemented, bytes `0x10`-`0x7F`); no hub change | Setting, record and checksum, handshake, block and a race of 2, 4 and 7 machines: confirmed (by running it) (section 16) |
| `indy500` (2B) | NETWORK TYPE: EEPROM `S32A` record byte `0x1E` (copy `0x42`): 0 STAND ALONE, 1 MASTER, 2 SLAVE. CABINET ID byte `0x1F` (copy `0x43`), 0-based, 1 to 4 on the sister games. CABINET TYPE TWIN | 4 inferred (2 run) | Same start as `manxtt`; reads `0x1A10000`/`2`/`3`; blocks `0x1C0` at `0x2000`, received from `0x21C0` | **Built** (the `S32A` record's checksum is solved, `docs/hw/eeprom-records.md`; a node's record has NETWORK TYPE 1/2 and CABINET ID 0/1 in both copies) | Setting: confirmed (by running it). Linked in `link-run`, split screen and `link-net` (14.7); needs different CABINET IDs |
| `motoraid` (2A) | NETWORK TYPE: `S32A` byte `0x1E` (copy `0x42`): 0 / 1 MASTER / 2 SLAVE; CABINET ID byte `0x1F` (copy `0x43`): 1 to 4, cycling | 4 inferred (2 run) | as `indy500` plus a write of its ID to `0x1A10001` and `0x1A10014` | **Built** as `indy500` (data only) | Setting: confirmed (by running it). Linked in `link-run`: see 14.7 |
| `waverunr` (2C) | NETWORK TYPE: `S32A` byte `0x1E` (copy `0x42`): 0 STAND ALONE / 1 MASTER / 2 SLAVE; CABINET ID byte `0x1F` (copy `0x43`) | 4 inferred (2 run) | as `motoraid` (reads `0x1A10012`, writes `0x1A10001` and `0x1A10014`); own block about `0xB4` bytes | **Built** as `indy500` (data only) | Setting: confirmed (by running it). Linked in `link-run`: see 14.7 |
| `skisuprg` (2C) | NETWORK TYPE: `S32A` byte `0x20` (copy `0x48`; the record is longer): 0 NO LINK / 1 MASTER / 2 SLAVE; CABINET ID byte `0x21` (copy `0x49`): 1 to 4 | 4 (the hub ran 2 and 4) | A **board check by read-back**: 0, 0 then 1 to `0x1A14000` and the byte read back must be 1 (bit 0 set), then 0 written and read back 0 ("NETWORK BOARD IS NOT EXIST !" otherwise); then the ordinary start (0, 0, the master 1 to `0x1A14002`, 1 to `0x1A14000`), its ID and `0x380` to `0x1A10001`/`0x1A10014`, a poll of `0x1A10000`; `0x380`-byte blocks at `0x2000`, received from `0x2380` | **Built** (section 17): a board whose `0x1A14000` reads back what was written (`LinkBoard::Latch`) | Check, start, block geometry and a race of 2 and 4 machines: confirmed (by running it) (section 17) |
| `sgt24h` (2B) | LINK TYPE: EEPROM `S32A` record bytes `0x19`-`0x1C` (copies `0x20` later): `01 01 01 00` NOT LINK, `02 00 01 00` CAR NO1 Master, `02 00 00 01` CAR NO2 Slave; CAR NO3 and NO4 Slave also offered. LINK MAX (NOT LINK until a type is chosen, then 2) | 4 (LINK TYPE); 2 run | Same registers; writes its ID to `0x1A10001`; blocks of **`0x380`** (own `0x2000`-`0x237C`, received from `0x2380`); about 50 reads of `0x1A14002` a frame. Jaleco: the set holds the Z80 link board ROM `epr-18643a.7` | **Settings and data**: the `0x380` layout in `LinkPlay`, the `S32A` record per node | Setting: confirmed (by running it). Linked in `link-run` with `block 0x380, rx 0x2380`: two cars in one race (confirmed (by running it)); byte meanings and LINK MAX range unconfirmed |
| `overrev` (2C), `overrevb` (2B) | GAME ASSIGNMENTS MENU, FOUNDATIONAL SETTING: LINK MAX byte `0x36` (copy `0x6A`): 0 NOT LINK, 1 2 LINKS, 2 3 LINKS, 3 4 LINKS; LINK TYPE byte `0x35` (copy `0x69`): 0 MASTER CarNo.1 (the default), 1 SLAVE CarNo.2, 2 CarNo.3 ... (`S32A` record). LINK MAX is what turns it on | 4 (menu); 2 run | The `sgt24h` family: reads `0x1A10002`/`3`, writes `0x1A10001`, blocks of `0x380` (own `0x2000`, received from `0x2380`); the master does **not** write `0x1A14002` (the hub warned "not exactly one master" and went on). Jaleco: no link board ROM in the set | **Settings and data** (as `sgt24h`) | Setting: confirmed (by running it). `overrev` linked in `link-run` with the `0x380` layout: both nodes in one race (confirmed (by running it)); `overrevb` only the LINK MAX byte checked |
| `dynabb97` (2B) | none: GAME ASSIGNMENTS has GAME DIFFICULTY, ADVERTISE SOUND, CABINET TYPE (VS = two-player), FAVORITE, INNINGS | 1 | none: no access to `0x1A10000`-`0x1A14002` | not applicable | confirmed (by running it) |
| `desert` (Model 2) | none: GAME SYSTEM has COIN/CREDIT, COUNTRY, ADVERTISE SOUND, DIFFICULTY | 1 | none | not applicable | confirmed (by running it) |
| `srallyc`, `daytona` | built (sections 2 to 12) | 4, 8 | | done | |

The `S32A` record (every game above but `manxtt`, `stcc`, `von`, `dynabb97` and `desert`):
magic `53 33 32 41` ("S32A"), two copies of a record (36 bytes at `0x08` and
`0x2C`; `overrev` 52 bytes, `skisuprg` 40, `sgt24h` 32 bytes apart),
each a 16-bit checksum, a write counter that goes up by one at each
save, then the settings from `0x10`. The game saves both copies. **Its
checksum was found by reading the games' own code** (section 14.1 first
found it not to be a CRC-16 or a sum by black box): a shifting sum, the
same routine with a different start value per game family, a plain byte sum in
`sgt24h`; `docs/hw/eeprom-records.md` describes it and what the game does with
a record it finds. `romset::RecordCrc` computes it and writes the second copy.

### 14.3 The rest of the games

`topskatr`, `vf2`, `fvipers`, `lastbrnx`, `doa`, `zerogun`, `sfight`,
`pltkids`, `schamp`, `vstriker`, `segawski`, `bel`, `hotd`, `gunblade`,
`dynamcop`, `vcop`, `vcop2`, `skytargt`: none of their boot, attract and
coin-start runs access `0x1A10000`-`0x1A14002` (the runs' `io.txt` list of
unmapped accesses has no such address; the games that do link show
`0x1A14000` there at once, even when not linked). They have no link board
check, so no link play *(confirmed (by running it) for those runs; a check hidden
behind a setting or mode no run reached cannot be ruled out)*. Their test
menus were not walked. `rchase2` is the one with "checking network": that is its
ride drive board (`docs/games/rchase2.md`), not this board. `dynabb97` and
`desert` were walked (above): no setting.

### 14.4 What the survey found about the board

- **One register interface, two shared-RAM layouts.** Every game that has a
  link setting except `skisuprg` starts the board as `srallyc` does (0, 0 to
  `0x1A14000`/`0x1A14002`; read `0x1A14000`; the master 1 to `0x1A14002`; 1 to
  `0x1A14000`) and then reads `0x1A14002`, `0x1A10000`, `0x1A10002` and
  `0x1A10003`; several write their ID to `0x1A10001` (`von`, `sgt24h`, `overrev`,
  `motoraid`, `waverunr`; the hub accepts it). What differs is the layout of the
  shared RAM (section 14.5): `0x1C0`-byte blocks (`srallyc`, `daytona`,
  `manxtt`, `indy500`) or `0x380`-byte blocks received from `0x2380` (`sgt24h`,
  `overrev`), and the per-game setting store. The Z80 ROM `epr-18643a.7` (the link
  board's firmware by its header, `docs/games/sgt24h.md`) is in the sets of `von`,
  `stcc` and `sgt24h`, but nothing here runs it.
- **The start check runs at every boot, linked or not**, in the games that
  have a board slot: `0x1A14000` is written within the first 700 frames
  (`manxtt` at 62, `sgt24h` at 98, `skisuprg` at 148, `stcc` at 630), which
  is why the slot's all-ones value matters (section 7).
- **The setting is saved when the test menu is left**, not when a value is
  changed (`manxtt`), so a player UI or tool that sets it without the menu must
  also write the record's checksum (`srallyc`, `manxtt`: known; the `S32A` records:
  read from the games' code, `docs/hw/eeprom-records.md`; `SEGASTCb`: found by running the game,
  section 16.1; `von`'s SRAM record: read from the game's code, section 15.1).

### 14.5 Hub runs (what `link-run` did with them)

To see more than the first wait, `remodel link-run` was given the games with
**temporary, uncommitted** entries: for `manxtt` a plain `LinkPlay` (the
game's own default record, EEPROM store, role byte `0x0B`, roles 1 and 2);
for the `S32A` games a placeholder entry whose per-node settings were the
EEPROM images the game itself saved from its test menu (one MASTER or
CAR NO1, one SLAVE with CABINET ID 2), the role and checksum written to unused
bytes, and for `sgt24h` and `overrev` block `0x380`, received blocks from
`0x2380`. The hub itself is unchanged. Two machines, `coin-start`-like inputs on
both, 4,500 frames, pictures kept out of the tree.

- **`manxtt`**: the ring comes up at frame 64 (the boot start check), the game
  exchanges `0x1C0` blocks (word 1 of a block carries `0x0101` / `0x0202` in its high half,
  the ID and the car number as `srallyc`'s do), plays attract, the ACTIVE MOTION SLIDER screen, course
  select and a race showing POSITION 1/2 and /2; with 3 and 4 nodes POSITION
  n/3 and n/4 and the 1P markers. No setting needed but the entry, and the game's
  own record with byte `0x04` at 1 (not 2) did as well.
- **`indy500`**: "CHECKING NETWORK NOW Ver.3.01", "THIS IS MASTER CONTROLLER" /
  "THIS IS SLAVE MACHINE", "Node ID :1 / :2", "Total Nodes :2", "Cabinet ID
  :1 / :2", then the race with POSITION 1/2 and 2/2 and the other car in view.
  **With both cabinets at CABINET ID 1 the game shows NETWORK ERROR and
  restarts the check forever** (the first attempt), so each node's CABINET ID
  must differ: the per-node role needs both bytes, as `daytona`'s CAR NUMBER.
- **`motoraid`, `waverunr`**: linked (a RIVAL marker; 1st/2 and 2nd/2) with the
  `0x1C0` geometry; only a look at the first race.
- **`sgt24h`**: with the `0x1C0` geometry the board came up and both screens
  stayed on "CHECKING NETWORK NOW, LINKED, CAR No.1 MASTER / CAR No.2 SLAVE" for
  4,500 frames (the game's blocks are `0x380` bytes, which that exchange does not
  copy); with `0x380` and received blocks from `0x2380` both are in one race
  (POSITION 7/8 and 8/8, the cars side by side).
- **`overrev`**: the same, with the `0x380` layout, after the first run
  with `0x1C0` left one machine racing alone and the other on ROUND 1. The master
  writes nothing to `0x1A14002`, so the hub's "master first" rule had no
  master to find; node 0 was taken as the first and the ring was right.
- Not run in this survey: `von`, `stcc` and `skisuprg` (they were taken in sections 15 to 17),
  `overrevb`.

Two record families are visible in the traces (registers the same, shared RAM
different): **`0x1C0` blocks at `0x2000`, received from `0x21C0`** (`srallyc`,
`daytona`, `manxtt`, `indy500`; `motoraid` and `waverunr` write smaller blocks and
run on it) and **`0x380` blocks, received from `0x2380`**, with the game writing
its ID to `0x1A10001` (`sgt24h`, `overrev`; `von`'s two 1,792-byte buffers are the same
size, *inferred*).

### 14.6 Recommended order (steps 1 and 2 are done: section 14.7)

1. **`manxtt` (and its clone `manxttc`): data only, confirmed.** One `LinkPlay`
   entry (EEPROM store, role byte `0x0B`, roles Master 1 / Slave 2, the game's
   default record, CRC from byte 2 to 55) and the hub ran it unchanged with two,
   three and four machines. It needs no new code, only the entry and a
   `max_nodes` (the menu only offers Master and Slave; the game accepted four
   in the hub), so it is the cheapest proof that the board model is generic and gives
   two-bike head-to-head at once. Check the race to the flag and the
   player UI's controls before calling it done.
2. **The `S32A` settings record, once: then `indy500`, `motoraid`, `waverunr`,
   `sgt24h`, `overrev`/`overrevb`.** Six games (seven sets) keep their link
   setting in the same Sega record (a magic, two copies, a write counter and a
   16-bit checksum), and the hub already links them: the runs above did it with
   settings images the games saved themselves. The one piece of work is making a
   node's record: read the checksum routine out of one game's program (the
   library code is the same in the others, *inferred*), then give
   `LinkSettings` a "two copies, counter, checksum" form with two role bytes
   (NETWORK TYPE and CABINET ID, or LINK MAX and LINK TYPE; the IDs must
   differ, `indy500` shows a NETWORK ERROR loop otherwise). Order within it:
   `indy500` first (already linked in the hub, a clear race), then `motoraid` and
   `waverunr` (linked at first look; check a race), then `sgt24h` and `overrev`
   (they also need the `0x380`-byte layout, which is `LinkPlay` data: `block`,
   `rx`), `overrevb` last as `overrev`'s clone.
3. **`von`, then `stcc`: the `0x380` family with their own stores.** `von` is a
   two-node head-to-head fighter and the most wanted of them for play; its
   setting is a backup-SRAM byte (a third `SettingsStore`), it writes its ID to
   `0x1A10001` and waits for bit 7 of `0x1A14002` to be *set*, which `sgt24h` and
   `overrev` (step 2) will have settled in the hub; do it after them. `stcc`
   shows `von`'s wording ("THIS IS MASTER SITE"), so it follows `von`: it has its
   own record (`SEGASTCb`, LINK TYPE in the top three bits of byte `0x10`, a
   checksum not yet found) and offers CAR 1 to 7, which would be the largest ring
   after `daytona`.
4. **`skisuprg`: last.** Its start is a different handshake (the read-back of
   `0x1A14000` after the start, "NETWORK BOARD IS NOT EXIST !"), with no poll of
   `0x1A14002` or read of `0x1A10000` in the part that ran; what it expects back
   is unknown, so it needs code reading before any board model.

`dynabb97` and `desert` have nothing to add. For all of them the test menu's own
link setting is a start-up decision, so the player UI can set it the way it sets
`srallyc`'s (`LinkPlay` roles) without the user opening the test menu.

### 14.7 What was built from this survey

Built (data in `crates/romset/src/games/link.rs`, all *confirmed (by running it)*):

| Game | Where it links | Free play | Checks |
|---|---|---|---|
| `manxtt`, `manxttc` | `link-run` (2 to 4 machines), split screen, `link-net` | COIN/CREDIT SETTING #27 (bytes `0x04`, `0x24`, `0x2E` of the record) | `make link-net-test GAMES=manxtt`: clean D=2 and 5% loss D=6 hashes equal `link-run`'s, players leaving (2 and 3 hosts), settings, desync, mDNS; `link-free-play-test` |
| `indy500` | `link-run` (2 machines run; the menu range is 4, *inferred*), split screen, `link-net` | COIN/CREDIT SETTING #27 (bytes `0x10`, `0x11`, `0x15`) | `link-net-test GAMES=indy500` and `link-free-play-test GAMES=indy500`; `link-record-test` |
| `motoraid`, `waverunr` | `link-run` (2 machines), split screen | not looked for | `link-record-test`; a race in `link-run`: Motor Raid shows RIVAL 2 and RIVAL 1 and POSITION 10 and 9 of 10, Wave Runner 10th/10 and 9th/10 with 2nd/2 and 1st/2 on the link pair; not run over the network |

- **The role bytes.** `manxtt`: byte `0x0B` 1 / 2 in the `srallyc` record kind (a
  CCITT checksum). `indy500`, `motoraid` and `waverunr`: NETWORK TYPE
  (byte `0x1E`) 1 master / 2 slave and CABINET ID (byte `0x1F`) the node's index
  0 / 1, both in both copies of the `S32A` record, whose checksum
  is the one of `docs/hw/eeprom-records.md` (read from the games' own code); a
  record the game does not accept would be rebuilt from its defaults and the machine
  would run stand-alone, so the nodes' final EEPROMs keeping their roles is the
  check (`make link-record-test`). The records are the ones the game's own test
  menu saves at the defaults (an `eeprom.bin` of a run that held TEST and chose
  EXIT).
- **Blocks.** `manxtt`, `indy500`, `motoraid` and `waverunr` use the `0x1C0`
  blocks at `0x2000`, received from `0x21C0`, as `srallyc`, with node IDs 1-based.
  The players-leaving ghost's kept bytes are `srallyc`'s for all four
  (*unconfirmed*: the bytes were not mapped; the cases of `link-net-test`
  agree with `link-run --leave` whatever they are, but how well a leaver's car
  is parked was only looked at for `manxtt` and `indy500`, where the race went on).
- **Free play.** Found as for `srallyc` (`docs/games/manxtt.md`,
  `docs/games/indy500.md`): the test menu's COIN ASSIGNMENTS page, the setting's
  number stepped to FREE PLAY, the menu left, the two `eeprom.bin` files
  compared.

Not built in this step: `sgt24h` and `overrev`/`overrevb` (the
`0x380`-byte block layout is `LinkPlay` data, but their records differ: a
52-byte record in `overrev`, a byte-sum record in `sgt24h`; the checksum
is known, section 14.2). `von`, `stcc` (and `stcca`, `stccb`) and `skisuprg` were built
afterwards: sections 15 to 17.

## 15. Virtual-On (`von`, `vonj`): two machines

Taken from the survey's row (section 14.2) to a linked one-against-one match, in `link-run`, the
split screen and over the network. Everything below is *confirmed (by running it)* unless it says
otherwise; the game's own code was read for the SRAM record (the games' code is the source, as
everywhere here).

### 15.1 The setting and its record

NETWORK LINK ATTRIBUTE (GAME ASSIGNMENTS: NOLINK, SLAVE, MASTER in the order TEST steps it) is the byte
at `0x1D00028` of the **battery-backed SRAM**, which no other linked game uses (the original Model 2's I/O
board record and the 93C46 are the others): 0 SLAVE, 1 MASTER, 2 NOLINK (the default). The SRAM record
is checked at every boot (`remodel disasm`, ROM `0x2568`):

| Offset | Contents |
|---|---|
| `0x00` | the 16 bytes `SEGA.AM#3.VRON.` (with the NUL) |
| `0x10` | u32 `0x20C`, the record's length |
| `0x14` | u16 **checksum** (low byte first) over the 34 bytes `0x16`-`0x37` |
| `0x16`-`0x37` | the operator settings (20 bytes the game copies from its ROM defaults at `0x22A0`, then 14): NETWORK LINK ATTRIBUTE at `0x28`, the COIN/CREDIT SETTING number at `0x2A` |
| `0x38`-`0x20B` | bookkeeping counters, then the two ranking tables (`MSB` names and scores) |
| `0x20C` | **a second copy** of the 0x20C bytes |

A record whose header, length and checksum are right is kept; one whose are not is replaced by the copy,
and if that is bad too by the defaults (*read from the code*; the last step is *confirmed*: a first experiment
changed the role byte in both copies without fixing the checksum, and the game wrote its default role back at frame 426). The checksum routine (ROM `0x3120`)
is a table-driven CRC-16, the table being the CCITT one (entry 1 is `0x1021`): a 32-bit register starts at
`0xDEBDEB00`; for each byte the byte is added to the register, which is shifted left eight bits and
exclusive-ored with the table entry of its old top byte in bits 16-31; the result is the table entry of the
final top byte exclusive-ored with bits 8-23. `romset::SumKind::Table`. *Confirmed* against the record the
game wrote at its first boot and the one it wrote after FREE PLAY was chosen (`make link-record-test GAMES=von`).

**Free play.** COIN ASSIGNMENTS, COIN/CREDIT SETTING #27 FREE PLAY (service steps to the page, the third row,
26 presses of TEST from #1; the SRAM saved when the test menu is left): byte `0x2A` 1 -> `0x1B` and the
flags `0x34` and `0x36` 0 -> 1, the checksum and the second copy following
(`romset::games::link`, `VON_FREE_PLAY`). Both a start press with no coin and `make link-free-play-test GAMES=von`.

The SRAM is not kept between runs here (it is all ones at power on, and the game writes its defaults at
about frame 426), so `LinkPlay` carries the record as the game writes it (`SettingsStore::BackupRam`, 0x418
bytes loaded over the start of the SRAM, role byte and checksum set per node, the second copy rewritten) and
the player's own von settings are never touched.

### 15.2 The handshake (frames of `link-run`, delay 1)

| Frame | What |
|---|---|
| 426 | writes 0 to `0x1A14000` and `0x1A14002`; waits for bit 7 of `0x1A14002` to be clear, reading it once a frame |
| 487 | writes 1 to `0x1A14000` (the board is started); the hub's per-frame flip of bit 7 (section 11) is what both of its waits see (busy clear, then set), so the hub needed no change for it |
| 489 | writes its own ID to `0x1A10001` (**1 for the MASTER, 2 for the SLAVE**: it never writes `0x1A14002`, so the hub's "not exactly one master" warning appears and node 0 is first), reads `0x1A10012` (the hub gives 0) and writes half of it to `0x1A10014` |
| 519 | polls `0x1A10000` until 1; reads the total at `0x1A10003` (2) and its ID at `0x1A10002` (the same 1 or 2); any other total is "Illegal Nodes" |

### 15.3 The block

The game writes **0x700 bytes** (1,792, the "two 1,792-byte buffers" of section 7: the game's TX and RX
areas) at `0x1A12000` every frame from the frame the link is up, and reads `0x1A12700`-`0x1A12DFF`, the
**other node's block**: `tx 0x2000`, `rx 0x2700`, `block 0x700` (not the `0x380` layout of the survey's inference;
the own block echoed at `0x2E00` is never read). At every frame dumped (`link-run --dump-at`) node 0's first received
block was byte for byte node 1's own block. What the block holds, from dumps of an attract, a machine select and
a fight (*unconfirmed*: meanings): a u16 at `0x02` that changes every frame and is **the same in both nodes'
blocks** (a shared frame-seeded sequence?), a byte at `0x04` that tells the nodes apart (0x20 and 0x21 in a match),
a flag at `0x554`, then floats that are the robots' positions. A block is about 286 bytes a frame on the
network (`link-net`, delta coded).

### 15.4 What the machines do

`link-run`, two nodes, coin and start on both: after the link the attract shows "GO AGAINST OTHER PLAYER", the
machine select, and then **a match in which each machine's robot is the other's enemy**: in a
capture at about 5,000 frames one machine's PLAYER bar is full and its ENEMY bar low while the other's is the
other way round, and the robots are in the same place seen from opposite sides (colours: the first node's robot
violet, the second's red). The countdowns differ by 0.04 to 0.06 seconds between the machines (the game's own
clocks; they do not drift further). The block layout, the IDs and the unchanged hub were all it took.
`vonj` (Japan) has the same SRAM record (the dump of a fresh boot differs only by a run counter) and the same
link, with the prompt "乱入対戦しますか?" ("join the battle?") before a match: `VariantLink::Parent`.

The game accepts two nodes only (any other total is "Illegal Nodes"), so `max_nodes` is 2.

### 15.5 What was changed

- `SettingsStore::BackupRam`, `SumKind::Table` and `RecordCrc::table`; `Model2Bus::settings_load` and
  `settings_dump` (the three stores in one place); `RecordCrc::fix` copies a second copy up to the end of the image.
- `remodel-linknet`: a block was limited to 1,024 bytes; it is now 1,792 (`wire::MAX_BLOCK`) and a datagram is
  read up to 2,000 bytes. A record of a block that changed all over is as large as the block (about 1.85 KB), so
  the IP layer fragments that datagram on an Ethernet link; the delta coding sends far less than that in play
  (286 bytes a frame in a fight).
- No other hub change: other games' hashes are untouched (`make link-net-test GAMES="daytona srallyc"`).

### 15.6 What is untested

The game's own use of the block fields; a SLAVE-first start (the roles are fixed: node 0 MASTER); a real
link board's timing; the Z80 board program, which nothing here runs.

### 15.7 Checks

All run on the shared development machine, with `--jit` (which gives the interpreter's hashes: the first `von` run, without it, ended on the same ones).

- `remodel-play --game von --players 2 --frames 5000 --shot F` (no sound): the two machines side by side in one window, the violet and the red robot, the
  health bars mirrored.
- `make link-net-test GAMES=von`: clean D=2 and 5% loss D=6 give each host the hash of `link-run`'s node; the scripted and the Wi-Fi-like delay changes, the host found by
  mDNS, a polite leave, a kill and the host leaving with two hosts, a leave with a delay change in flight, a settings byte that differs and a damaged record all pass. (There is no
  three-host case: the game takes two nodes.) The strict form of `delay-adaptive-clean` (the adaptive rule must not move D on a clean link) needs a game that holds 60 Hz, and `von`
  does not: a single machine's frame times (`remodel run`, `--jit`, coin, start and shots, 3,106 frames) are 5.0 ms median but have five frames over 33 ms (worst 50 ms at frame 1041,
  the machine select's loads), so the rule raises D at about frames 1070, 1310 and 2820 even on a quiet link (144 stalls in 4,000 boundaries, longest 30 ms). For `von` that case therefore
  expects "any" change (`ADAPT_CLEAN` in the script) and still has to pass what matters: each host's hash equals `link-run --delay-at` with the schedule the hosts logged. (`stcc` and
  `skisuprg` pass the strict case, at load 16-17; `daytona`, untouched, failed it at load 23.)
- `make link-free-play-test GAMES=von`: deterministic, differs without free play, and a host with `--free-play` and a joiner without end on `link-run`'s hashes (the settings record
  goes with the session's parameters: `Params::base` is up to 1,280 bytes now, `wire::MAX_BASE`; `von`'s image is 1,048). A start press does not begin a game on free play before the
  "PRESS START BUTTON" screen: the script presses at 1100, 1500, 1900 and 2300; the next prompt, "ENGAGE OTHER PLAYER?", takes the default.
- `make link-record-test GAMES=von`: the record the game saves at first boot, and the one after FREE PLAY (it must be `VON_FREE_PLAY`'s bytes), pass the checksum; a two-node `link-run` leaves roles 1 and 0 in the nodes' SRAM.

## 16. Sega Touring Car Championship (`stcc`, `stcca`, `stccb`): up to seven machines

### 16.1 The setting, the record and its checksum

LINK TYPE (GAME ASSIGNMENTS, the fifth row: STAND ALONE, CAR 1 to CAR 7) is **bits 5-7 of EEPROM byte `0x10`**
(0 stand-alone, 1 to 7 for the CAR number; bit 0 of the byte is another setting, 1). The 128-byte image
(`docs/hw/eeprom-records.md` section 4):

| Offset | Contents |
|---|---|
| `0x00` | `SEGASTCb` (`stcca` and `stccb`: `SEGASTCa`) |
| `0x08` | u16 **checksum**, low byte first |
| `0x0A` | `FF FF` |
| `0x0C` | u32 write counter (0 in the game's first-boot record, 1 after the first save from the test menu) |
| `0x10` | the settings (to `0x29`), then `FF` |

**The checksum** is the CRC-16 of `srallyc`'s record kind (CCITT polynomial `0x1021`, start value `0xFFFF`,
taken high bit first, **complemented**) over **bytes `0x10` to `0x7F`**, the whole rest of the 128-byte image
including its `0xFF` fill, stored low byte first (`RecordCrc::ccitt(8, 0x10, 0x80, 0xffff, true)`). Found by running the game: its first-boot
record was written and a search over the checksums known here (CCITT, the S32A shifting sum and the table sum of
`von`, with their start values, and byte sums) over every start and end offset found one match with a plain meaning
(this one; the few others were ranges starting at byte 17 to 19, which are not fields of the record); then **confirmed against five
records the game wrote itself**, by `remodel --game stcc link-settings`: its first-boot defaults (checksum `0xA60F`),
LINK TYPE CAR 1 (`0x407B`), CAR 3 (`0x9CB2`) and CAR 7 (`0x3501`), each saved from the test menu by changing the row and leaving the
menu, and FREE PLAY (`0x71DE`). `stcca` and `stccb`: the same rule over the same range for their own records (their first-boot records and
their FREE PLAY ones, all passing), whose settings run four bytes shorter (`01 00 f1 50 02 00 fc ff ...` against the parent's
`01 00 f1 ff 50 ff ff ff 02 00 fc ff ...`); `stccb` differs from `stcca` in the byte at `0x12` (`d1`, not `f1`).

**Free play**: COIN ASSIGNMENTS has one row, COIN/CREDIT SETTING; #27 is FREE PLAY (26 presses of TEST from #1), and the
game's save changes the word at `0x18`-`0x19` (`02 00` -> `8A 06`; `stcca` and `stccb`: `0x14`-`0x15`),
the counter and the checksum.

### 16.2 The handshake (frames of `link-run`, delay 1)

| Frame | What |
|---|---|
| 630-631 | writes 0 to `0x1A14000`/`0x1A14002`, reads `0x1A14000` (0: a board), writes 1 (the board is started) |
| 631-633 | reads `0x1A14002` every frame until bit 7 comes (the hub's flip) |
| 633 | writes its ID to `0x1A10001`, **`0x0E00` to `0x1A10012` and `0x01C0` to `0x1A10014`** (*inferred*: the game tells the board the size of the received area, eight blocks, and of one block) |
| 694-815 | polls `0x1A10000` for 1; then reads `0x1A10003` and `0x1A10002` |

The wording is `von`'s ("THIS IS MASTER SITE"). It does not write `0x1A14002` as a master either; the hub warns and takes
node 0 as the first.

### 16.3 The block, and the races

`0x1C0` bytes at `0x2000` every frame, the other nodes' blocks from `0x21C0` (the `srallyc` geometry, IDs 1-based, own block last:
`LinkPlay` as `srallyc` with `max_nodes 7`). The CAR number of a node is the order it was started in.
`link-run` with coin, start and the select screens' start presses:

- **2 machines**: qualifying alone (the two cars appear as CAR 1 and CAR 2, an Alfa Romeo and an AMG), then
  Round 1 where the starting grid lists both as "A. 1" and "A. 2" and the race shows POSITION 6/8 and 7/8, the other
  machine's car on the road.
- **4 machines**: the four cars (Martini Mercedes, AMG, Opel, Castrol Toyota) in one Round 1, POSITION 6, 5, 8 and 7 of 8.
- **7 machines** (CAR 1 to 7, the most the menu offers): seven cars in one Round 1, POSITION 2, 3, 5, 6, 4, 7 and 8 of 8,
  each with its own distinct position (the eighth car is the game's).

### 16.4 Checks

- `remodel-play --game stcc --players 4 --frames 7200 --shot F`: four machines in the quadrants, each in Round 1 with the others on the road (POSITION 6, 5, 8 and 7 of 8).
- `make link-net-test GAMES=stcc`: every case as for `von` and the three-host leaves too (the game takes seven); `delay-adaptive-clean` passed at load 17 (the others as in 15.7).
- `make link-free-play-test GAMES=stcc` and `make link-record-test GAMES="stcc stcca stccb"` pass (the records and free play bytes, the roles `0x21` and `0x41` kept by a two-node `link-run`).
- `stcca` and `stccb` race two machines in `link-run` (POSITION 8 and 7 of 8, the other human car marked "A" on the road).

### 16.5 What was changed and what is untested

`LinkPlay` entries for `stcc`, `stcca` and `stccb` (the record, the role bytes `0x21, 0x41, .. 0xE1`, free play); the two clones
use `VariantLink::Own` because their record differs. No hub or linknet change. Untested: CAR numbers other than in the
order 1..N (the role bytes are fixed per node), what the block fields mean, and an eighth machine (the menu offers none).

## 17. Ski Super G (`skisuprg`): up to four machines

The survey (14.2) stopped at "NETWORK BOARD IS NOT EXIST !" with an idle board. What the game wanted was in its code
and is confirmed by a linked race.

### 17.1 The board check wants a register that reads back

With NETWORK TYPE not NO LINK the game (ROM `0x6D720`) writes 0 to `0x1A14000`, 0 to `0x1A14002`, then **1 to
`0x1A14000` and reads the byte back: bit 0 must be set**; then it writes 0 and reads again: **bit 0 must be clear**. An
empty slot reads all ones (the first half passes, the second fails), an idle board reads 0 (the first fails): both give
"NOT EXIST". The board it wants has a `0x1A14000` whose bit 0 follows what the CPU wrote: `romset::LinkBoard::Latch` (`model2::link::Link`
reads back the last byte written; hub, start and everything else as for the other games). With it the game goes on with
the *ordinary* start of the survey's family: frame 149 writes 0, 0 again, 150 the master writes 1 to `0x1A14002` and 1 to `0x1A14000`,
reads `0x1A14002` (the hub's bit 7 flip), 152 writes its network type (1 MASTER, 2 SLAVE) to `0x1A10001` and **`0x0380` to `0x1A10014`**
(the block size), polls `0x1A10000`, reads `0x1A10002`/`3`, `0x1A10012` (0) and `0x1A10014` (what it wrote).

### 17.2 What follows

The game shows "CHECK NETWORK (MASTER)" and "CABINET ID n" for 60 frames (state 5), then spends 300 frames in state 6 (seen: a watch on its state word),
which, *read from its code* (ROM `0x6C430`, not exercised with equal IDs), counts the **CABINET ID at offset 4 of every node's block** (the game writes `0x1A12004`
itself, from its setting) and fails if two blocks carry the same number, then goes on to attract with the link. The blocks are `0x380` bytes at `0x2000` (the game writes 41 bytes of it, per
frame, in attract; the received blocks it reads are at `0x2380` and `0x2700`): `tx 0x2000`, `rx 0x2380`, `block 0x380`, IDs 1-based, own block last. The
role bytes are NETWORK TYPE `0x20` (1 MASTER, 2 SLAVE) and CABINET ID `0x21` (the node's index, shown 1 to 4), the same in
both copies of the `S32A` record (`docs/hw/eeprom-records.md`, start value `0xDEBDEB00`); the record's checksum is `RecordCrc::s32a(0x08, 40, 39, 0xDEBDEB00)`.

### 17.3 Races

`link-run`, coin and the selects' button presses, 2 and 4 machines: each machine's skier starts in its own colours
(yellow, violet, white and red) and the others are on the slope with it: **1st/2 and 2nd/2** on two machines, **1st/4 and 4th/4**
(the two between have their rank flashing) on four. **Free play**: COIN ASSIGNMENTS #27 (15 presses from the default #12): bytes `0x10` 0x0B -> 0x1A, `0x11`
0 -> 1, `0x12` 2 -> 1.

### 17.4 Checks

- `remodel-play --game skisuprg --players 2 --frames 3600 --shot F`: two skiers side by side, 1st/2 and 2nd/2.
- `make link-net-test GAMES=skisuprg`: every case, the three-host leaves included; `delay-adaptive-clean` passed at load 16-17.
- `make link-free-play-test GAMES=skisuprg` and `make link-record-test GAMES=skisuprg` pass.

### 17.5 What was changed and what is untested

`LinkBoard::Latch` (the manifest of `skisuprg` now fits it; its default NO LINK never touches the register, so single-player
state hashes are unchanged), `link-run`/`link-net`/the player keep the manifest's board kind instead of making an idle one, the
`LinkPlay` entry. Untested: a CABINET ID other than the node's index, the meaning of the other block fields, a MASTER that is not node 0.
