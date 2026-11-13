# Model 2 (the original board): map, I/O board, ROM layout, sound

What is known about the original Model 2 board (1993-94: `daytona`,
`vcop`, `desert`) from `daytona` (Daytona USA, revision A), the only one
brought up so far. Everything comes from the game's own code and data and
from running it on the board model (`model2::bus` with `Board::Model2`);
where a statement is a guess it says so. The game's page is
[`docs/games/daytona.md`](../games/daytona.md).

**Status.** The board model boots `daytona` to its attract mode (the
settings screen, the AM2 logo, the 3D attract races, textured and lit,
the "V.R." screen and the ranking tables), and a coin-start run reaches a
race that drives. No sound (§5). It is in `romset::games::IN_PROGRESS`,
not yet in the launcher.

## 1. What it shares with Model 2A (confirmed by the run)

`daytona` runs on the 2A board model nearly unchanged, so the two boards
share at least what the game touches (`Board::has_tgp_2a`):

- the i960KB, the program ROM at 0, work RAM at `0x500000`, interrupt
  controller `0xE80000`, timers `0xF00000`, tile RAM `0x1000000`, palette
  `0x1800000`, backup SRAM `0x1D00000`, buffer RAM `0x900000`, the
  geometrizer's ports at `0x800000` and the sound link UART at `0x1C80000`;
- the **MB86234 TGP**, its program **uploaded by the game** (2,024 words,
  from main data `.6/.7`, as on 2A; `mb86234.md`), its FIFO at `0x884000`,
  command ports `0x880000 + 0x10*F` (which `daytona` uses for every call:
  the header word is what the hardware makes of the port's address), and
  control and status at `0x980000`/`0x980004`;
- the **video RAM at `0x12000000`** (texels) and the luma table at
  `0x12800000`, the polygon ROM, texture ROM and colour tables as 2A lays
  them out: the pictures are right with 2A's model (§3);
- the **fixed-function geometrizer**: nothing is uploaded to it; the 2C
  native geometrizer's command handling over the polygon ROM draws the
  attract scenes (`model2a.md` §7). Its own arithmetic is *unconfirmed*,
  as on 2A.

## 2. Where it differs (confirmed from `daytona`'s code and the run)

| Address | What | How known |
|---|---|---|
| `0x200000`-`0x21FFFF` | RAM, which the boot code fills with the program ROM's first 128 KB | the boot's copy loop (it then runs from ROM at 0) |
| `0x220000`-`0x23FFFF` | **the program ROM's second 128 KB again** (ROM `0x20000`-`0x3FFFF`): the code there is linked to run at `0x22xxxx` | relative calls from the ROM's first half land on function entries only at `0x220000 +` their ROM offset (`call 0x229440` reaches ROM `0x29440`), and the code there calls back with negative displacements; 908 references to `0x22xxxx`/`0x23xxxx` in the program |
| `0x1C00000`-`0x1C0007F` | **I/O registers**, a byte each at `0x1C00000 + 2N` (not the 315-5649's layout) | below |
| `0x1C00200`-`0x1C002FF` | the **I/O board's buffer**: 128 bytes, one per 16-bit word | below |
| `0x1A10000`-`0x1A14003` | the **network (link) board**: shared RAM from `0x1A10000`, status at `0x1A14000` (bit 0 set: no board) | the game's network check; with the link ID set to MASTER it shows "NETWORK BOARD NOT PRESENT" and restarts when bit 0 is set |

`0x200000`'s RAM (as on 2A/2C, 256 KB there) is cut to 128 KB on this
board in the model; whether the hardware has RAM, or the ROM again, at
`0x200000`-`0x21FFFF` is *unconfirmed* (nothing reads it after the copy).

### I/O registers (*confirmed* from the code, except where marked)

Register N is the low byte of the 16-bit word at `0x1C00000 + 2N` (a
32-bit read of word k gives registers 2k and 2k+1 in byte lanes 0 and 2,
as the 2C's chip does).

| Reg | Address | Use |
|---|---|---|
| 0-7 | `0x1C00000`-`0x1C0000E` | **analog channels 0-7**, read directly (no channel select register); the game sums four reads of each. Daytona's input test (*confirmed*): channel 0 the wheel (an absolute position, `0x80` centre, dead zone +-5, full lock at +-80, below `0x80` left), 1 the accelerator, 2 the brake |
| 8 | `0x1C00010` | switches (active low): coin 1 `0x01`, coin 2 `0x02`, test `0x04`, service `0x08`, start `0x10`, view buttons `0x20`, `0x40`, `0x80` |
| 9, 10 | `0x1C00012`, `0x1C00014` | switches (the fourth view button is reg 9 bit 0; the gear lever is reg 9 bits 4-6, read through a table at `0x230D1C`: a 4-position H gate as on `srallyc`; reg 10 is read as 16 bits in the test menu: DIP switches, *unconfirmed*) |
| 14 | `0x1C0001C` | switches, read with reg 8 as one 16-bit input word |
| 15 | `0x1C0001E` | an output latch the game sets and clears bits of (lamps, *unconfirmed*) |
| 17 | `0x1C00022` | a byte queue the game feeds from a 256-byte ring (the drive board's commands, *unconfirmed*) |
| 18 | `0x1C00024` | written 0 and 1 at start-up |
| 26-29 | `0x1C00034`-`0x1C0003A` | written "S", "E", "G", "A" at start-up |
| 32 | `0x1C00040` | **command**: the game waits for it to read 0, writes a command, and waits for 0 again |
| 33 | `0x1C00042` | **status**: the game waits for `0x40` around the buffer's commands |

The buffer commands (*confirmed*, the routines at ROM `0x28D40` and
`0x28E00`): **3** fills the buffer from the board's store, after which the
game copies the 128 bytes into the backup SRAM at `0x1D00000`; to save, it
writes **1**, copies the SRAM's 128 bytes into the buffer and writes **2**.
The vblank handler also writes 1 when the register reads 0. The record is
the operator settings: "SEGA", a version, a CRC-16 (CCITT, polynomial
`0x1021`, over bytes `0x0A`-`0x7F`) at `0x08`, the settings (link ID at
`0x0B`: 0 SINGLE, 1 MASTER, 2 SLAVE; cabinet, country, difficulty, ...)
and counters; a second copy at SRAM `0x80`; when neither checks, the
program's defaults (link ID MASTER, cabinet TWIN, country JPN). The store
itself (an EEPROM on the I/O board?) is *unconfirmed*: the model keeps the
buffer as plain RAM, completes every command at once and reads `0x40` as
the status, which is all the game needs; `Game::backup_ram` fills it on
this board (not the SRAM, which the game overwrites from it).

## 3. ROM layout (`daytona`)

| Chips | Role | Evidence |
|---|---|---|
| `epr-16722a.12` (low) / `-16723a.13` (high) | i960 program, 256 KB | boot record valid (PRCB `0xC0`, first IP `0x860`); second half linked at `0x220000` (§2) |
| `mpr-16528.10` / `-16529.11`, `mpr-16808.8` / `-16809.9`, `epr-16724a.6` / `-16725a.7` | main data, **packed** 4 MB, 4 MB and 1 MB from `0x2000000` | the boot record's interrupt table is at `0x2802040`, inside `.6/.7` with this order (its vectors point at the handlers in the program ROM); `.6/.7` (EPROMs of the program's revision) also hold the TGP program and the program's text and tables; the game reads `0x2800000`-`0x28FFFFF` 2,000+ times in its code |
| `mpr-16523.16`/`-16518.20`, `.17/.21`, `.18/.22`, `.19/.23` | polygon ROM, by analogy with 2A (low half first), packed | the attract scenes draw right; *unconfirmed* chip by chip |
| `mpr-16522.25` (even) / `-16521.24`, `mpr-16770.27` / `-16769.26` | texture ROM, as 2A (`.25`, `.27` the even words; the second pair in its own 8 MB slot) | the attract scenes' textures are right; *unconfirmed* chip by chip |
| `mpr-16537.28` (low) / `-16536.29` (high) | the TGP's data ROM from `0x800000` | **`.28` is the low half**, the reverse of `srallyc`'s pair: so paired, it begins with a 4x4 identity matrix of floats (`1.0` = `0x3F800000`) and its tables hold word offsets into itself that the program turns into ROM addresses (§4); the other way round they are halves of floats swapped |
| `opr-14742a.45` ... `-14747.63` | the TGP's maths tables (as 2A) | shared with every 2A set |
| `epr-16720.7` | sound 68000 program (§5) | its vectors and code |
| `epr-16721.8` | sound board, *unknown* (tables; not 68000 code at its start) | |
| `mpr-16491.32`, `-16492.33`, `mpr-16493.4`, `-16494.5` | sound samples, *unconfirmed* (8 MB) | |
| `mpr-16310.15`, `-16311.16`, `-16312.14` | ramps, as on 2A: not used | |

The zip also holds the clones' chips (`dayton93`, `daytonas`, `daytonase`,
`daytonat`, `daytonata`, `daytonagtx`, `daytonam`); none is used.

## 4. The TGP's external bus: the bus register's high bits

*Confirmed (by the run):* `daytona`'s program reads its data ROM by
putting a whole ROM address (`0x80xxxx`) in the bus register (`63`, which
the 2A programs only set to `0x400000` or 0) and its low 16 bits in an
address register. Taken alone, those accesses went to the function unit's
ports `0x00`-`0x3C`, which don't exist. The interpreter now completes an
external address with the bus register's bits 16-21 and 23 (bit 22 stays
the mode: shared RAM or function unit), which changes nothing for the 2A
programs (`mb86234.md`).

## 5. Sound board

**Not the 2A/2B/2C sound board** (*confirmed* from the program; the
chips' identities are inferred, with good confidence for the interfaces
below): the 68000 program (`epr-16720.7`) runs
from address 0 (reset vector: stack `0xF0FFFE`, PC `0x300`) with 64 KB of
RAM at `0xF00000`, takes the i960's bytes from a UART-like port at
`0xC20001` (status `0xC20003`) in its level-2 interrupt, and drives:

- **two sample chips**, at `0xC40000` and `0xC60000`, each with a busy
  flag (bit 0) and data at `+1`, a slot select at `+3` and a register
  select at `+5`, set up at reset slot by slot from a table of slot
  numbers 0-27 skipping every eighth (`0`-`6`, `8`-`0xE`, ...), with
  registers 0 (`0x80`), 4 (0) and 5 (`0xFE`) written for each: the
  28-voice layout and register interface of Sega's **MultiPCM** (315-5560)
  (*inferred*: identified by that interface, not by running it), each
  with a bank byte at `0xC50001`, `0xC70001`;
- an **FM chip** at `0xD00000`: busy flag bit 7 at `0xD00001` and two
  address/data pairs (`0xD00001`/`0xD00003`, `0xD00005`/`0xD00007`), a
  Yamaha OPN2 (YM3438/YM2612) interface (*inferred*);
- `0xC10001`, a byte the program steps through `1, 2, 4, ... 0x80` (LEDs or
  a watchdog, *unconfirmed*).

reModel's sound board is a 68000 with an SCSP at `0x100000` and its
program at `0x600000`; none of that carries over except the 68000 core.
The board model does not run this program (`model2::audio` leaves the
original Model 2 silent). What it would take: the board's memory map, a
MultiPCM model and an OPN2 model, and the sample ROMs' layout. That is new
work of about the size of the DSB1 board, not a variant of the SCSP board.

## 6. What is left

- The shifter in play (its bits are from the code), the drive board, the
  link board's protocol, and the sound board (§5).
- The I/O board's other registers and its store (§2).
