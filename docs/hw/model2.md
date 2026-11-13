# Model 2 (the original board): map, I/O board, ROM layout, sound

What is known about the original Model 2 board (1993-94: `daytona`,
`vcop`, `desert`) from `daytona` (Daytona USA, revision A) and `vcop`
(Virtua Cop, revision B). Everything comes from the games' own code and
data and from running them on the board model (`model2::bus` with
`Board::Model2`); where a statement is a guess it says so. The games'
pages are [`docs/games/daytona.md`](../games/daytona.md),
[`docs/games/vcop.md`](../games/vcop.md) and
[`docs/games/desert.md`](../games/desert.md) (Desert Tank, which needed
nothing new from the board model).

**Status.** The board model boots `daytona` to its attract mode (the
settings screen, the AM2 logo, the 3D attract races, textured and lit,
the "V.R." screen and the ranking tables), and a coin-start run reaches a
race that drives, with sound (§5). `vcop` boots to its attract mode and
a coin-start run plays its first stage with the light gun (shots, the
off-screen reload), with sound; `desert` plays its first mission. All
three are in `romset::games::IN_PROGRESS`, not yet in the launcher.

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
| 9, 10 | `0x1C00012`, `0x1C00014` | switches (the fourth view button is reg 9 bit 0; the gear lever is reg 9 bits 4-6, read through a table at `0x230D1C`: a 4-position H gate as on `srallyc`; reg 10 is read as 16 bits in the test menu: DIP switches, *unconfirmed*). `vcop`: reg 9 bits 0 and 1 are the two guns' triggers (its input test) |
| 11 | `0x1C00016` | **DIP switches** (`vcop`, *confirmed* from the code, `0x25E0`): bit 7 clear sets automatic reloading, bit 6 the enemies' look (GAME SYSTEM shows both, neither can be changed there), bit 5 a flag at `0x501700` (*unknown*). The model reads `0xFF`, every switch off (active low, as the inputs): manual reloading, which the off-screen shot needs. *Unconfirmed:* the factory setting |
| 14 | `0x1C0001C` | switches, read with reg 8 as one 16-bit input word |
| 15 | `0x1C0001E` | an output latch the game sets and clears bits of (lamps, *unconfirmed*) |
| 17 | `0x1C00022` | a byte queue the game feeds from a 256-byte ring (the drive board's commands, *unconfirmed*) |
| 18 | `0x1C00024` | written 0 and 1 at start-up |
| 26-29 | `0x1C00034`-`0x1C0003A` | written "S", "E", "G", "A" at start-up |
| 12, 13 | `0x1C00018`, `0x1C0001A` | read by `vcop`'s input routine with 11 (`0x1800`); not used further (*unknown*) |
| 32 | `0x1C00040` | **command**: the game waits for it to read 0, writes a command, and waits for 0 again |
| 33 | `0x1C00042` | **status**: the game waits for `0x40` around the buffer's commands |

**The light gun registers** (`vcop`, *confirmed* from the code and its
GUN MARK CHECK screen): register N is the low byte of the 16-bit word at
`0x1C00100 + 2N`, read with `ldos` once a frame (`0x1500`-`0x16FC`).
Registers 0/1 are player 1's Y (low byte, then the high byte), 2/3 its X,
4/5 and 6/7 player 2's Y and X, and register 8 one bit a player (bit 0
player 1, bit 1 player 2) that is **set while the gun sees no screen**: the
gun mark isn't drawn and a shot with it set reloads. That is the layout of
the 2C gun interface's counters (`model2c.md`, `hotd`), read directly here
rather than through commands, so the model serves both from `model2::gun`.
The game maps a position through a calibration in its settings (bytes
`0x24`-`0x33` of the record below; the defaults X 131-630 and Y 36-425
for player 1, X 128-627 and Y 39-425 for player 2) to the 496x384 view.
On a shot it fills the tile layer for four frames (the flash the gun
sees) and takes the position latched in the middle of it (`0x585xx`,
`0x15xx`). *Unconfirmed:* what the registers hold while a gun sees nothing
(the model reports 0), and the timing hardware behind them.

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
| `epr-16721.8` | sound board, *unknown* (not 68000 code at its start; the sound program never addresses it) | |
| `mpr-16491.32` then `-16492.33` | MultiPCM 1's samples, 4 MB | `.32` starts with the 12-byte header table; its banked samples' loops match `.32`'s second MB and `.33`'s two ([`multipcm.md`](multipcm.md)) |
| `mpr-16493.4` then `-16494.5` | MultiPCM 2's samples, 4 MB | likewise |
| `mpr-16310.15`, `-16311.16`, `-16312.14` | ramps, as on 2A: not used | |

The zip also holds the clones' chips (`dayton93`, `daytonas`, `daytonase`,
`daytonat`, `daytonata`, `daytonagtx`, `daytonam`); none is used.

`vcop` (*confirmed* by running it, [`vcop.md`](../games/vcop.md)) differs:
its program is **512 KB**, `.12/.13` then `.14/.15`, linked where it lies
(code runs from `0x40000`-`0x7FFFF`; nothing from the `0x220000` window);
one polygon pair (`.16/.20`) and one texture pair (`.25/.24`); no TGP data
ROM; its first MultiPCM's ROMs are 1 MB chips in 2 MB sockets
([`multipcm.md`](multipcm.md), "Sockets"). Main data, the TGP's tables and
the sound program's sockets are `daytona`'s. The board model maps the
`0x220000` window for both; *unconfirmed* what the hardware has there for
a 512 KB program.

`desert` (*confirmed* by running it, [`desert.md`](../games/desert.md)):
a 512 KB program as `vcop`'s; main data as `daytona`'s; polygons paired
**`.16/.21` and `.17/.20`** (found from the data); textures `.25/.24`; the
TGP's data ROM two 512 KB EPROMs, `.28` low and `.29` high, as
`daytona`'s pair.

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

**Not the 2A/2B/2C sound board** (*confirmed* from the program): a 68000
with two Sega MultiPCM sample chips and a Yamaha OPN2 (YM3438) FM chip,
fed the i960's MIDI-style bytes on a UART. Modelled as
`sound::PcmBoard` (`crates/sound/src/pcm_board.rs`), the chips as
`sound::multipcm` ([`multipcm.md`](multipcm.md)) and `sound::opn2`. The
map is from `daytona`'s program (`epr-16720.7`); `vcop`'s (`epr-17170.7`)
and `desert`'s (`epr-16985.7`) use the same addresses (checked: the
same absolute addresses throughout).

| 68000 address | device | evidence |
|---|---|---|
| `000000-01FFFF` | program ROM `epr-16720.7` (16-bit, byte-swapped like the other sound programs) | reset vector: stack `0xF0FFFE`, PC `0x300` |
| `C00001` | an input: bit 0 picks one of two velocity curves (`0x5D16`/`0x5D96`) at each note-on (`0x9CA`) | *unknown* source (a jumper?); the model reads `FF` |
| `C10001` | a latch the program steps through `1, 2, 4 ... 80` on each received byte (`0x7FC`): LEDs | ignored |
| `C20001`/`C20003` | UART data / control and status, 8251-like: reset with `00 00 00 40`, mode `4E`, command `37` (`0x6AC`) | the level-2 handler reads status (bits 3-5, the error flags, must be 0) then data |
| `C40001`/`3`/`5` | MultiPCM 1: data (read: status, bit 0 busy), slot, register | `0x74C` |
| `C40013` | written `3` once at reset | *unknown*; ignored |
| `C50001` | MultiPCM 1's sample bank (0-3) | set per tune (`0x458`, `0x1482`) |
| `C60001`/`3`/`5`, `C60013`, `C70001` | MultiPCM 2, likewise | `0x77A` |
| `D00001`/`D00003` | FM part 1: address (read: status) / data | `0x7B0`; the busy flag (bit 7) is polled before each write |
| `D00005`/`D00007` | FM part 2: address / data | `0x7D6` |
| `F00000-F0FFFF` | 64 KB work RAM | cleared at reset (`0x338`); stack at the top |

- **Interrupts** (*confirmed*): only level 2 (autovector 26, `0x120`) has
  a handler; every other vector points at `RTE` (`0x100`). The handler
  takes one received byte: `FF` restarts the program (MIDI reset; the host
  sends `F8 F8 FF` at boot), `F8` is dropped, and the rest are assembled
  into MIDI messages (status `80`-`BF`, `E0`-`EF`: three bytes; `C0`-`DF`:
  two; running status). So the UART's receive line raises level 2 while a
  byte is unread. The main loop runs at mask 1.
- **Timing** (*confirmed*): nothing else interrupts. The main loop
  (`0x522`-`0x5E0`) polls the FM chip's status bit 1, **timer B**, between
  its jobs, and on an overflow (`0x5F2`) re-arms it (register `0x27` =
  `0x2A`: reset B's flag, enable it, keep B running) and runs the
  sequencer's tick (`0x608`, `0x27EE`). Timer B is loaded with `0xFC`
  (`0x860`): 4 counts, 64 FM samples, a tick. Timer A is loaded (`0x3F0`)
  but never started.
- **The FM chip is only a timer in `daytona`** (*confirmed*): the
  program writes it only from the reset tables at `0x860`/`0x876` (LFO
  on, the timers, all six channels keyed off, total levels `7F`, release
  rates `F`, both parts) and register `0x27`; nothing keys a channel or
  sets a frequency. `vcop` and `desert` write the FM chip from the same
  few places. The OPN2 model is complete enough for a program that does
  play it (see `crates/sound/src/opn2.rs` for what is approximate), but
  no game checks that yet.
- **The i960 side**: the host's sound link UART (`0x1C80000`), as on 2A;
  the board has no reply path that the program uses (it never writes the
  UART's data port).
- **Sample ROMs** (*confirmed*, [`multipcm.md`](multipcm.md)): MultiPCM 1
  reads `mpr-16491.32` then `mpr-16492.33`, MultiPCM 2 `mpr-16493.4` then
  `mpr-16494.5`, each as 4 MB, its first MB fixed and the bank register
  choosing the MB seen at `0x100000-0x1FFFFF`. MultiPCM 1 plays the music,
  MultiPCM 2 the effects and voices (a channel's chip is bit 0 of its
  settings byte, `7(a3)`, e.g. `0x14C8`). `epr-16721.8` is not referenced by the program
  (*unknown*).
- **Clocks** (*unconfirmed*): the model runs the 68000 at 10 MHz and the
  FM chip at 8 MHz (timer B's tick 1.152 ms, so the music's tempo: the
  race music's sixteenth notes come out 0.15 s apart, 100 beats a minute);
  the MultiPCMs' base rate is inferred as 44.1 kHz from the samples'
  tuning ([`multipcm.md`](multipcm.md)); rendered music is within about 5
  cents of A440 tuning by its spectral peaks. Nothing on the CPU side
  depends on its clock except how much it gets done between ticks.
- **Mix** (*unconfirmed*): the two MultiPCMs summed at equal level and the
  FM chip's channels added; the board's analog stage is not known.

## 6. What is left

- The shifter in play (its bits are from the code), the drive board and
  the link board's protocol.
- The sound board's clocks and analog mix, the MultiPCM's envelope and
  LFO timings, `C00001` and `C40013` (§5, [`multipcm.md`](multipcm.md)).
- The I/O board's other registers and its store (§2); registers 12 and
  13, which `vcop` reads; the gun registers while a gun sees nothing.
