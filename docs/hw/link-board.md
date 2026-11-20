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
(section 7). The 2A's slot is the same as the original board's
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
  blocks and the ready byte is common is *unconfirmed*; Virtual-On was not run past its handshake; Daytona was taken
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
| Settings | `link-run` builds each node's image: the default record, the role byte, the CRC | where they live (`SettingsStore`: the EEPROM for srallyc, the I/O board record for daytona), the default record, the role byte's offset and its value for node 0, 1, .. (`role_off`, `roles`), a cabinet-number byte (`car_off`), the CRC (`RecordCrc`: range, position, start value, complement) |
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
* **D** is the hub's delay as before (1: the others' blocks of the same
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
* **Session agreement** (the lobby): a joiner's `Hello` carries the game id, a
  hash of the program ROMs, the reModel version (the crate version) and the
  protocol version; the host answers `Welcome` or `Refuse` with a plain
  reason (another game, other ROMs, another version or protocol, lobby full,
  session already started). The host's `Start` carries D, the node
  assignment, the host table (addresses), the check interval, the timeout and
  the settings; the joiners answer with their settings hash; then `Go`.
  Every step is repeated until answered, so the setup works through loss.

### 12.2 The packets

UDP, `RMLN` + protocol version + kind; at most 1,400 bytes; everything
little-endian and bounds-checked, and **a malformed packet is dropped whole,
never trusted** (sizes, node ids, host indexes, frame ranges; the session id
and the source address must match the table). Kinds: Hello, Welcome, Refuse,
Lobby, Ping, Pong, Start, StartAck, Go, **Data**, Bye, Abort, Discover, Info.

A **Data** packet carries the sender's newest frame, an acknowledgement per
host (the frame up to which the sender holds all of that host's records), the
sender's last eight checkpoint hashes and **every record of the sender's that
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

### 12.3 Desync, disconnects, the end

* **State check.** Every N frames (default 60, `--check-every`) each host
  hashes **every node's record of that frame as it holds it** (flags, start
  frame, block) and sends the hash in its packets; a mismatch with another
  host's stops the session on both with `out of sync with player P at frame
  F: our state hash X, theirs Y`. Each host's *machine* differs (it is
  another node), so there is no machine hash to compare; what must agree is
  the records every host feeds its hub, and a damaged, misdecoded or
  misdelivered record shows there. (Checked: a record damaged in memory at
  frame 1200 is reported at frame 1200 on both hosts.) A different build is
  caught earlier by the version check, other ROMs by the ROM hash, other
  settings by the settings hash.
* **A host that goes silent** for T seconds (default 5, `--timeout`) ends the
  session on every other host with `player P has been silent for T s:
  connection lost`; a polite leave or finish sends `Bye` (three times) and
  ends it at once with the reason. Records a leaving host had already sent
  are used first: a host passes boundaries as long as it has what its view
  needs and ends when it stalls for lack of what will never come. **What the
  game would do** with a node that vanishes is not known beyond section 9.4
  (a node that stops sending leaves its last block in the others' receive
  area, and the games did not complain at any delay tried); a mid-race
  recovery (a replacement, a rejoin) is out of scope: the session ends, and
  the launcher shows why.
* **The end of a scripted run**: a host that reaches `--frames` waits (at
  most 3 s) until every other host has acknowledged its last record, then says
  `Bye finished`; the others pass the boundaries they have records for and
  stop.

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
| joiner killed (SIGKILL) | the host ends after the 2 s timeout, with the message | same |
| joiner leaves politely (`--leave-at 1500`) | the host ends at once: `player 2 left: left` | same |

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
