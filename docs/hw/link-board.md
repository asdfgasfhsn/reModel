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
against a scripted partner. `daytona`, `von` and `overrev` were only
looked at (section 7). The 2A's slot is the same as the original board's
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
   `TWIN`). It is the byte at offset `0x0B` of the settings record, the game's
   work RAM `0x20201B`: 0 NOTLINK (the default), 1 CAR 1, 2-4 CAR 2-4, 5
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

The settings record: 4-byte header (CRC-16 over the next 34 bytes, `0x24`
as a length), then 32 bytes of settings (the game's own copy is at work RAM
`0x202010`); the CRC is the usual table-driven CCITT polynomial, initial
value `0xFFFF`, result complemented, stored low byte first (the game's
routine at `0x5C87E0`; reproduced and checked against two EEPROMs).

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
| `0x1A10002` | read once the link is up: this node's ID (0 for the master) |
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
- **Received blocks** at `0x1A121C0 + 0x1C0 * k`. Receive block 0 is *this
  node's own* block as the board echoes it; the game reads its own car's
  state back from there (*confirmed*: a partner that delayed the *echo* too
  changed the physics: the car's speed and gears went wrong). Block `k` is
  *inferred* to hold node `(ID + k) mod TOTAL`: the game finds the master
  in block `(TOTAL - ID) mod TOTAL` (the master reads block 0, the slave
  with ID 1 of 2 block 1: *confirmed* for both, with the slave setting).
  For two nodes then: receive 0 = me, receive 1 = the other.

The block, as observed (a Desert-stage race, offsets in bytes from the
block start; all words little-endian as the i960 stores them):

| Offset | Observed |
|---|---|
| `0x00` | u16 node ID (0 master, 1 the slave of two); `0x02` u16 car number, ID + 1 |
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

Inferred: that received block `k` is node `(ID + k) mod TOTAL`; the node
state numbering past 6; settings copy to slaves; the meaning of most
block fields.

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
  blocks and the ready byte is common is *unconfirmed*; Daytona and Virtual-On
  were not run past their handshakes.
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
  blocks: block 0 its own, block `k` node `(ID + k) mod TOTAL`'s. The
  machines run frame-by-frame in lockstep (one thread each or alternately
  on one).
- **Handshake model** (`Link` register behaviour): `0x1A14000` bit 0 reads
  0; a write of 1 starts the board; node IDs by master first (the cabinet
  that wrote 1 to `0x1A14002` is 0, the others in order of start, 1..);
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
- Effort: the model and a headless two-machine harness with a race to the
  same finish line is about **3 days** if the mirror results hold; the
  player view and two sets of controls about 2 more.

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

1. What a real second cabinet's blocks contain: ID and car number fields,
   what the slave puts in the settings words, the `0x1469`-like seed
   (does the master pick it and do slaves adopt it?). A mirror can't
   show this; two machines in (a) will.
2. How the real board numbers nodes and orders receive blocks for 3 and 4
   cabinets, RELAY, and TWIN.
3. Whether the real board's echo of the own block is immediate or one
   frame late; the model chose "visible next frame" and the game ran.
4. Whether the game gets a block mid-frame torn on hardware (the real board
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
