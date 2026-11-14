# Model 2B-CRX: the board as `overrevb` uses it

Status: 2026-11-09. reModel runs `overrevb` (Over Rev, the 2B version),
`sgt24h` (Super GT 24h), `dynabb97` (Dynamite Baseball '97), `doa`
(Dead or Alive) and `indy500` (Indianapolis 500) from boot
through attract into play, with both SHARCs running the programs the
games upload. `sgt24h` needed nothing more from the board model: its
SHARC code is `overrevb`'s (the uploaded images differ only past the
code), and it uses the same ports. Neither did `dynabb97`, Sega's own:
its geometrizer code is the Jaleco games' too (2,987 words, identical up
to `0x20C30`), its copro program its own (1,956 words), which uses only
the command and result FIFOs and FLAG0/FLAG1 described below. `doa`
(Tecmo) also runs on this model, with the same geometrizer code and a
copro program of its own (3,261 words); it checks for a protection device at
`0x1D80000`, answered as described under "The protection device"
below. `indy500` (Sega, 1995, the earliest) runs a geometrizer program
of its own (2,983 words, an earlier version of the shared one) and a
copro program (3,937 words) that also reads data ROMs on its external
bus, which needed that bus modelled ("The copro's external memory"
below), and it reads the copro status at `0x980014` in its test menu.
`fvipers` (Fighting Vipers, 1995) and `schamp` (Sonic Championship,
1996) run `indy500`'s geometrizer code and the shared one respectively,
and copro programs of their own (4,267 and 4,870 words) that write parts
of the display list into buffer RAM, the copro's external bank 2. They
needed that bank, list jumps the i960 patches in buffer RAM, and texture
RAM banks of 1 MB ("Sega's earlier games" below). `vstriker` (Virtua
Striker, 1994) runs geometrizer and copro programs of its own (2,910 and
2,199 words of code), needed nothing new from the board model, and takes
the 1 MB texture RAM banks too; its main data layout is its own
("Sega's earlier games" below). `lastbrnx` (Last Bronx, 1996) runs the
shared geometrizer code with three commands added and a copro program of
its own (6,566 words) that depends on the SHARC's MODE1 effect latency;
it needed the host DMA's low-index alias, the interrupt register's second
word and tile RAM's 1 MB repeat ("Last Bronx" below).
`gunblade` (Gunblade NY, 1995) uploads a copro program of its own (6,958
words as uploaded) and the shared geometrizer code (3,117 words, code hash
`15393321f266b9cf`), and needed nothing new. It has copro data ROMs
(`.29/.30`) mapped on the copro's external bus as `indy500`'s are
(*unconfirmed* that its program reads them).
This page
is what the board model (`crates/model2`: `bus.rs`, `sharc.rs`, `geo.rs`)
is built on. The SHARC core itself is
described in `adsp2106x.md`; the parts 2B shares with 2C in
`geometrizer.md` and `peripherals.md`.

**Sources.** `overrevb`'s i960 program (`make analyze GAME=overrevb`), its
two SHARC programs (`make run GAME=overrevb` writes them to
`out/overrevb/sharc-copro.bin` and `sharc-geo.bin`; `make sharc-disasm
FILE=...` lists them), Analog Devices' ADSP-2106x User's Manual ([UM],
`adsp2106x.md`), and runs compared with `overrev`, the 2C version of the
same game. All chips but the two program ROMs are shared between the two
sets, so `overrev` on the 2C model is a reference for every scene. No
emulator's source was read. *Confirmed* means the game's code says so and
running it agrees; *unconfirmed* marks inference.

## What differs from 2C, as the i960 sees it

Everything not listed here (memory, video, tiles, I/O chip, EEPROM, timers,
interrupts, buffer RAM and the geometrizer's list ports) is as on 2C:
`overrevb` boots, attracts and races on the 2C map with only these changes.

| i960 address | 2B | Evidence |
|---|---|---|
| `0x8C0000-0x8C03FF` | copro SHARC's host window: IOP register *n* at `+4n` | upload routine (i960 `0x1FEC0`) |
| `0x840000-0x8403FF` | geometrizer SHARC's host window, the same way | upload routine (i960 `0x23330`) |
| `0x980000` bit 31 | set while the i960 programs the copro SHARC (see below) | brackets the copro upload |
| `0x980008` bit 31 | the same for the geometrizer SHARC | brackets the geometrizer upload |
| `0x980020` | written 0 right after each bit 31 is set | *unconfirmed* what it does; reModel ignores it |
| `0x884000` | with `0x980000` bit 31 set: the copro's EPB0 (the upload's data); otherwise writes feed the copro's command FIFO and reads take its results, as on 2C | upload loop; the boot's FIFO drain (i960 `0x23300`) |
| `0x880000 + 16·fn` | tagged copro commands, as on 2C: `value & 0x800FFFFF \| fn << 23` | the first command, `0x808` to `0x880080` (fn 8), passes the copro program's check |
| `0x980004` bit 0 | copro output FIFO empty, as on 2C | the drain loop reads `0x884000` until it is set |
| `0x804000` | with `0x980008` bit 31 set: the geometrizer's EPB0; otherwise the list data port, as on 2C | upload loop |
| `0x9C0000` / `0x9C0004` | sound UART (8251 type): data / mode, command and status, one register a 32-bit word, the byte in bits 7-0 (2C: `0x1C80000`, two registers a word) | init writes 0, 0, 0, `0xD0` (internal reset), `0x4E` (mode), `0x37` (command) to `0x9C0004`, then polls its bit 0 (TxRDY); the sound commands go to `0x9C0000` and the 68000 plays them |
| `0x1C80004` | written (16-bit) with `0x40` and `0x4E` during init: 2C's UART control address | *unconfirmed* what is there on 2B; nothing reads it, and treating it as the UART would only reset it again |
| `0x980014` | copro status: `indy500`'s T.G.P. TEST (i960 `0xC7B50`) reads it once and shows IC31 BAD if bit 1 is set, IC37 BAD if bit 0 is | reModel answers as on 2C: all ones until both SHARCs have programs, then 0, and the test shows both GOOD. *Unconfirmed* what the bits are. `fvipers` reads it every frame (i960 `0x11914`) and tests bit 1, with both branches going to the same place |
| `0x11100000`, `0x11300000` | texture RAM, the upper half of each bank's 2 MB window: `fvipers`, `schamp` and `vstriker` upload their texels there and nowhere else ("Sega's earlier games" below) | the games' texture base pointers (`0x500254`, `0x500258`, set at i960 `0x144`); the memory test |
| `0x12800000-0x128000FF` | written by `fvipers` and `schamp`: 128 halfwords, the ramp (i + 1) / 2 (0, 1, 1, 2, ... 64) at boot (`fvipers` i960 `0x474`). Otherwise only a test menu writes it: its "TRANS MAP" / "TRANS NUM" items copy one of the seven 128-byte luma tables in ROM (`0x064266E0`, the ones the game copies to luma RAM) there (`fvipers` i960 `0x57544`) | the code; *unconfirmed* what the hardware does with it (a luma-shaped map, perhaps for translucency); reModel maps nothing, and nothing reads it back |

**The upload** (i960 `0x1FEC0` for the copro, `0x23330` for the
geometrizer): set bit 31, write 0 to `0x980020`, write the DMA channel 6
registers through the window as 16-bit halves (the geometrizer first gets
MSGR7 = `0x7378842E`), then 12,288 halves to EPB0, then clear bit 31. The
details and the boot that follows are in `adsp2106x.md`, "Model 2B: the
upload".

*Confirmed (`overrevb`, `sgt24h`, with the ADSP-21062 core):* each SHARC is
booted as UM §11.6.3 describes for a 16-bit host: a DMA of 4,096 48-bit
words, sent as 16-bit halves to `0x884000` / `0x804000` while bit 31 of
`0x980000` (copro) or `0x980008` (geometrizer) is set. The window maps IOP
register *n* at window + 4*n*, each 32-bit register written as two 16-bit
halves, low first: DMAC6 = 0, II6 = `0x20000`, IM6 = 1, C6 = `0x1000`,
DMAC6 = `0xA1` (instruction words, 16-to-48 packing, LSW first), then
12,288 halves to EPB0. The DMA's end vectors through `0x20040` (`RTI`) to
`0x20005`, `JUMP 0x20080 (DB)`. The copro program has 2,111 words of code,
the geometrizer's 2,987, none with an undefined encoding
(`docs/hw/adsp2106x.md`). Both Jaleco games upload the same two SHARC
programs: the same code, word for word, but not the same images, since the
upload always sends 4,096 words and the tails past the code differ (one
data word in the copro's; in the geometrizer's, from `0x20C31` on,
whatever follows it in each i960 ROM, in `sgt24h` the i960 program's own
strings).

`fvipers` and `schamp` upload differently (*confirmed* from the host
writes, by running them): each IOP register is one 32-bit write, not two
halves; they write SYSCON (`0xA110` copro, `0x3110` geometrizer) and
WAIT (`0xC9400`, `0xC400`) through the window before the DMA, set C6 to
the program's length rather than `0x1000` (`fvipers` `0x10FF` and
`0xC29` words, `schamp` `0x135A` and `0xC2D`), and never write MSGR7.
The SHARC core takes them as they are.

**What bit 31 is.** [UM §11.6.3]: a SHARC set up for host booting idles
after reset; the host asserts HBR (host bus request), waits for HBG, then
writes the IOP registers and EPB0; the DMA's end vectors through EP0I and
the program runs. A SHARC held in RESET doesn't answer the host at all, so
bit 31 set can't be RESET asserted (the writes made under it work), and if
clearing it asserted RESET the program the game just loaded would never run
(it does: the game waits for the copro's replies). Bit 31 therefore behaves
as the host bus request: *confirmed* that it is not RESET in either sense,
*unconfirmed* that it drives the HBR pin itself. What resets the SHARCs
before a second upload isn't known: the game uploads each once per boot.
reModel resets a SHARC into host boot mode when its bit 31 rises.

## The protection device (`doa`)

`doa` talks to a device at `0x1D80000-0x1D87FFF` from one routine (i960
`0x29F0`, called once at boot from `0x5B8`), all 16-bit accesses. The
interface below is the game's side of it; *confirmed* from the code, and
by running it with the board model (`crates/model2/src/protection.rs`)
answering as the game expects.

| Offset from `0x1D80000` | Access | What the game does |
|---|---|---|
| `0x400C` | read | status: polled for `0xFFF0` before the request (up to 10,000 reads; anything else means "no device") and after each step below |
| `0x4000` | write | a command: `6` |
| `0x0000 + 2·n` | write | a data block, from halfword offset *n* (here 0): 12 halfwords, after a 4-byte header that isn't sent (output length `0x30`, then 0) |
| `0x7FF4` | write | a table address: 0, then `0x80` |
| `0x7FF6` | write | table words at that address: 24 halfwords at 0, 128 at `0x80`, waiting for the status after each |
| `0x7FF0` | write | 0 |
| `0x7FF2` | write | *n*, the block's offset: starts the device |
| `0x7FF8` | read | the answer: one read that the game discards, then 24 halfwords, the earlier byte of each pair in its high half |

The game byte-swaps the answer into RAM (`0x54B390`) and compares its 48
bytes with ROM `0x2810`, which reads `TECMO LTD.  DEAD OR ALIVE
1996.10.22  VER. 1.00`. It keeps the result (1 for a match) at
`0x54FABC`.

*Confirmed (from the ROM data):* the two tables describe a canonical
Huffman code. The first holds twelve entries of two halfwords, by code
length: (length, index of the first symbol of that length) and the first
code of that length left-justified in 12 bits. Lengths 2 to 6 are used
(the length-6 entry repeats to fill the rest); the counts they give (1,
1, 6, 6 and 4 symbols) make a complete code (the Kraft sum is exactly
1). The second table is the symbols, two to a halfword, high byte first:
the 18 characters of the string above, from the most frequent (space, 10
times) to the least. *Confirmed (by decoding it):* the data block isn't
that code read straight: decoded with the tables, in either byte order
and either bit order, it gives the wrong characters, and the string's
code (187 bits) doesn't start the block in any of those orders, nor
inverted. So the device does more than decode it (presumably it
descrambles the block first, perhaps keyed by the command);
*unconfirmed* how. With one request in the whole program there isn't
enough to work that out.

So the board model doesn't compute the answer: a game's manifest names,
per command, the ROM bytes the game compares the answer with
(`romset::Protection`), read from the ROM at run time, and the device
streams them back one halfword behind (the discarded first read) with
the status always `0xFFF0`. That answers `doa`'s check and nothing more.

What the check guards (*confirmed*, from the code): with `0x54FABC` 0,
starting a game (i960 `0x3558`) sets a countdown at `0x54F3B0` to 300
plus a random number below 900 frames (from the game's random number
generator at `0x8E640`); when it runs out (i960 `0x2FA4`) the game kills
its own tasks and the fight freezes. The same countdown arms when a
checksum of the boot code (30 halfwords at `0x570`, every other one,
against the value at `0x8FCE0`, i960 `0x520`) fails, so patching the
boot check would trip it too. With the check answered the countdown
never arms.

The other 2B games that touch `0x1D80000` use another interface (*seen*
in throwaway boots, with no device answering): `zerogun` writes 0 to
`0x1D9000A` and a block of 96 halfwords to `0x1D80000`, from code it runs
in RAM, then faults at IP 0 without reading anything back. `dynmcopb` and
`dyndek2b` write 0 to `0x1D9000A`, 10 halfwords to `0x1D80000`, then
`0x1D90008` and `0x1D9000C`, read `0x1D9000E` and `0x1D90000` nine times
each and fault at IP 0. `pltkids` faults at IP 0 before touching it.
None of them compares an answer with its ROM the way `doa` does, so the
`doa` device model doesn't apply to them; *unconfirmed* whether their
faults come from the missing device.

## The copro SHARC

Its program's view of the board (*confirmed* from the program, then by the
game running on it):

| SHARC side | What |
|---|---|
| external `0x400000`, read | the command FIFO (what the i960 writes to `0x880000+` and `0x884000`) |
| external `0xC00000`, write | the result FIFO (what the i960 reads at `0x884000`) |
| FLAG0 input | command FIFO empty: polled before every read (`IF FLAG0_IN JUMP (PC,0)`), and the idle loop is `DO ... UNTIL NOT FLAG0_IN` |
| FLAG1 input | result FIFO full: polled before every write |
| FLAG2 output | set on a command word that fails the check below, before it stops in a `DO ... UNTIL FOREVER` |
| FLAG3 output | set while it waits for a command, cleared when one arrives; *unconfirmed* what it drives |

Commands are dispatched on bits 30-23 of the first word (a table of 80
routine addresses at DM `0x30000`); the word must have the same value in
bits 7-0 and 15-8, as the 2C copro checks. After the upload it discards
words until a fn 8 command, which the i960's boot sends (`0x808` to
`0x880080`) once it has drained the result FIFO. No other external
address is used by the Jaleco games', `dynabb97`'s or `doa`'s programs:
those sets have no copro data ROMs.

### The copro's external memory (`indy500`, `fvipers`, `schamp`)

`indy500`'s copro program sets SYSCON to `0xA100`, as the others do:
MSIZE 10, so its four external memory banks are 2^(10+13) = 8M words
each from `0x400000` [UM §5.4.1, "External Memory Banks": bank 0 starts
at `0x400000`, banks 1-3 follow; each asserts its own MS*x* select line].
The command FIFO (`0x400000`) is bank 0 and the result FIFO (`0xC00000`)
bank 1; `indy500`, `fvipers` and `schamp` use the other two (*confirmed*
from their programs, then by running them):

| SHARC side | What |
|---|---|
| `0x1C00000+` (bank 3), read | the copro data ROMs `.29/.30` as 32-bit words, low half from `.29` (1 MB, `0x40000` words, the same interleave as 2C's copro data). fn 13 (`0x20256`) reads six pointers at `0x1C00000 + n + 0x10·k` and adds `0x1C00000` to each; fn 62 (`0x20BCB`) walks a table at `0x1C00000 +` an offset the i960 sends. Before the bus was modelled, 600M instructions of attract mode read it about 5,700 times (zeros) |
| `0x1400000+` (bank 2), read/write | **buffer RAM**, the display list the i960 builds at `0x900000`, word *n* at `0x1400000 + n` (see below). `fvipers`' fn `0x78` (`0x20908`) writes a part of the list there; both `fvipers`' and `schamp`'s programs also read tables at words `0x3E80` and `0x7E80` (bytes `0xFA00` and `0x1FA00`, near the top of each 64 KB half). `indy500`'s fn 54 (`0x206CB`) addresses it as `0x1400000 +` a 16-bit offset from the command plus up to 127, writes `0xFFFFFFFF` there and then reads and writes records through it; no run of `indy500` reaches it |

**Bank 2 is buffer RAM** (*confirmed* from `fvipers`' programs, then by
running it). The i960's routine at `0x5A48` reads the list's write pointer
(`0x802008`), moves it on by `0x48` bytes (`0x801008`), leaving a hole of
18 words, and sends the copro fn `0x78` (`0x3C007878`) with the hole's
byte offset and five more words. The copro handler (`0x20908`) writes 18
words to `0x1400000 +` offset / 4: a fn `0x0B` command (`0x05800B0B`)
with a 12-word matrix, a fn 1 command (`0x00800101`) with its operands,
and the words the i960 sent. The list then runs through the hole; with
nothing there it ran into buffer RAM's fill pattern and the geometrizer
stopped on a bad command (below). reModel maps the bank onto buffer RAM,
128 KB repeating through the bank (*unconfirmed* how many address lines
it decodes). Earlier reModel gave `indy500` a separate RAM there, which
none of its runs touched; that is now buffer RAM too, and nothing about
`indy500`'s runs changed but its state hash, which no longer hashes the
unused RAM (`4a1b197bf00149d6`, against `c75add6c2ab149d6` with it).

The data ROM bank is modelled only for sets with copro data ROMs
(`indy500`, `fvipers`, `schamp`); how many address lines it decodes
(whether the ROM repeats through its bank) is *unconfirmed*; reModel
repeats it. The other 2B programs never address banks 2 and 3, so their
save states and hashes are unchanged.

The result FIFO's depth is *unconfirmed*; reModel reports it full at 64
words, which the program only ever waits on.

## The geometrizer SHARC

| SHARC side | What |
|---|---|
| external `0x400000`, read | the list FIFO, fed by the list DMA from buffer RAM as on 2C. The program reads it without polling a flag, so the hardware must hold a read until a word is there (*unconfirmed*: reModel hands it the next list word directly) |
| external `0x410000`, write | the rasteriser FIFO |
| external `0x420000 + a`, write | polygon RAM word `a` (fn 5 with address bit 24 clear; reModel maps `a` below `0x10000`) |
| external `0x430000`, write | polygon memory address |
| external `0x430001`, read | the polygon memory word at that address: the polygon ROM from `0x800000` up (word addressed, as on 2C), polygon RAM below. Every model vertex and attribute word is read this way, one address write per word, the data read after it |
| external `0x430002`, write | written (any value) at each frame handshake, with FLAG1 set around it; *unconfirmed* what it signals (frame done?) |
| SHARC internal `0x30100+` | the "fast" polygon RAM: models at addresses with bit 24 set are read straight from internal memory (fn 5 writes there when bit 24 is set) |
| (none of the three rows above) | `overrevb` never writes polygon RAM or draws from it: in a coin-start run every model comes from the ROM through `0x430000`/`0x430001`, so those paths are the program's, not exercised |
| FLAG0 input | frame start, as F0 on the 2C geometrizer: the program polls it inside its loops and abandons the frame when it rises |
| FLAG1 output | raised around the `0x430002` write at each frame handshake |
| FLAG2 output | pulsed around each command word it reads; toggled forever after a failed command check (a check skipped when MSGR7 holds `0x7378842E`, which the upload writes). *Unconfirmed* what it drives |
| FLAG3 output | raised by END until the next frame starts (unless a test command ran since the last object); *unconfirmed* what it drives |

**Frame protocol** (*confirmed* from the program; reModel's geometrizer
runs it per frame like 2C's, `geo.rs`): at boot it waits for FLAG0 high,
signals (FLAG1, `0x430002`), waits for it low, and does that twice. Then
each frame: FLAG0 rises, the program emits NOP and END (the last frame's
terminator), signals, waits for FLAG0 to fall, and reads the list until
END, after which it emits NOPs until FLAG0 rises again. That is the 2C
program's protocol (`geometrizer.md` §1.2), so the same list DMA model
feeds it.

**Commands.** The dispatch table (32 entries at DM `0x30000`) has the 2C
command set: fn 0x00-0x10 with 0x11-0x13, 0x15, 0x17-0x1C and 0x1F
aliasing 0x01-0x03, 0x05, 0x07-0x0C and END, and 0x14 log data, 0x16 a
parameter store,
0x1D a code upload into internal memory (and a call to it unless bit 10 of
its first word is set), 0x1E a call into uploaded code. fn 0x0E is the
2C test command (FIFO pattern and polygon ROM checksums through
`0x430000`/`0x430001`). Unlike topskatr's 2C program, fn 0x11 is plain
object data (the same handler as fn 1). A coin-start run of `overrevb`
uses fn 0x01/0x11, 0x03, 0x04, 0x06-0x0B, 0x0F, 0x10, 0x14 and 0x16; never
0x02, 0x05, 0x0C-0x0E, 0x1D or 0x1E.

### What it emits: the rasteriser stream

**The same format as the 2C geometrizer's** (`geometrizer.md` §4.2):
command words `0xFF000000 | list word >> 23` (fn, fn bit 5 and the eye
select), data as IEEE singles shifted right 8 (the program's `LSHIFT BY
-8` before each write), windows repacked as `0x00XXXYYY`, NOP
`0xFF000000`, END `0xFF00000F`, polygon blocks as tpa, tha, two start
vertices, then attr, luma word, distance, one or two vertices per polygon.
So reModel's renderers (software and GPU) take it unchanged: there is no
translation step.

*Confirmed* by feeding the same display lists (captured from `overrev` on
the 2C model) to both geometrizer programs, the 2C one on the MB86235 core
and the 2B one on the SHARC core, and comparing the streams word by word
(a scratch harness, not kept):

- attract frames 600-610 (the logo) and 1700-1703 (the grandstand demo):
  identical lengths (up to 31,731 words) and identical command words;
  92-94% of words bit-identical; vertex coordinates differ only by float
  rounding (relative 3e-5 at most).
- The luma word differs on about 40% of polygons: the 2C program sends 0
  luma with the back-face bit (bit 23) set, where the 2B program sends a
  luma value with it, and the two disagree on the back-face bit for some
  polygons (all of them link type 0, which aren't drawn, or double-sided).
  The distance word sometimes differs in sign at huge magnitudes. Drawn
  with the same renderer, frame 1700 differs in 4 pixels of 190,464 (1,383
  against 1,384 polygons). Which program's lighting is right for its board
  can't be told from this, and reModel doesn't need to: each runs its own.

`overrevb` and `overrev` then render their own lists alike scene by scene
(attract, menus, races; compared from `make run ... SNAP=300` of both).

**Its program.** What the geometrizer program does, command by command,
and the native implementation of it that `--geo native` runs (bit for bit
the SHARC's stream, and render-only: nothing it does reaches the i960) are
in `geometrizer-2b.md`.

## Sega's earlier games (`fvipers`, `schamp`, `vstriker`)

Both games stopped with the geometrizer SHARC in its command-check loop:
the 3D vanished a few seconds into the attract mode and never came back.
What it waits for, and what was missing (*confirmed* from the programs
and the i960 code, then by running them):

- **The check.** Without MSGR7 = `0x7378842E` the geometrizer program
  (`0x2010C`) checks each command word: its fn (bits 27-23) must equal
  bits 4-0 and bits 12-8, as the tagged command port writes it. A word
  that fails goes into MSGR3 (IOP `0x0B`, readable by the host), and the
  program toggles FLAG2 forever (`0x2014B`; `DO ... UNTIL LCE` delay
  loops of 3,000,000) until a reset. Nothing on the board answers it, so
  it stops drawing for good. The Jaleco games, `dynabb97`, `doa` and
  `indy500` write MSGR7 and skip the check; these two don't, so any word
  that isn't a command reaching a command position stops them. MSGR3
  showed two kinds.
- **Copro list parts** (`fvipers`: the fill pattern `0x57575757`): the
  list ran into a hole the copro should have filled. The copro's bank 2
  is buffer RAM ("The copro's external memory" above).
- **Jumps patched in buffer RAM** (`fvipers`: `0x80018D78`,
  `0x80018DC4`): its i960 (`0x787C8`) writes a fn 0 word 0 through the
  command port as a placeholder and keeps its offset; once the next piece
  of the list is in place it writes a jump to that piece (bit 31 | the
  piece's offset) over the placeholder directly in buffer RAM (`st r13,
  0x900000(r14)`), and it retargets jumps it wrote through the port the
  same way. reModel's list DMA followed only jump words written through
  the port and unchanged since (stcc's copro stores matrices over old jump
  words, so a changed word couldn't stay a jump), so it fed the jump word
  to the geometrizer. Now a word the port wrote as a jump or a fn 0
  command stays a command word through the i960's own writes and is
  followed when bit 31 is set; the port appending over it or a copro
  store ends it. A copro store must end it: `fvipers`' copro writes a
  matrix element of -0.0 (`0x80000000`) into such a slot (an earlier
  rule, keeping any word rewritten into a bare jump, sent its list to
  offset 0 there). How the hardware tells jumps from data is still
  *unconfirmed*: this is the narrowest rule all the games agree with
  (`geo.rs`, `Geometrizer::jumps`).
- **Texture RAM.** With the list fixed, both drew with blank (all
  index 0) textures: they upload their texels only to `0x11100000` and
  `0x11300000`, the upper half of each texture bank's 2 MB window
  (their base pointers, set at boot), where `indy500` and `doa` use the
  lower half and `overrevb` and `sgt24h` fill both halves with
  different data. The rasteriser reads 1 MB of each bank. On these two
  games' boards the banks must be 1 MB, repeating through their windows:
  so mapped (`Game::texture_ram_mirrored`), they draw fully textured and
  their memory tests show every RAM GOOD. A board difference the i960
  can't see, so a manifest flag; *unconfirmed* whether it is a board
  revision, a jumper or RAM chips.
- `schamp`'s copro program writes the reserved IOP register `0x20`
  (`adsp2106x.md`), which faulted the core.

`vstriker` (1994), the earliest of Sega's 2B games here, *confirmed*
from the i960 code and its programs, then by running it:

- **Main data.** Its 1 MB pair `.7/.8` is the third pair of the first
  main data bank, packed after the two 4 MB pairs: `.11/.12` at
  `0x2000000`, `.9/.10` at `0x2400000`, `.7/.8` at `0x2800000`. The copro
  upload (i960 `0x8270`) reads its DMA header from `0x2825F30` (DMAC6
  `0xA1`, II6 `0x20000`, IM6 1, at offset `0x25F30` of `.7/.8`) and then
  the program words after it; the program refers to some 380 addresses in
  `0x28xxxxx` and none from `0x6000000` up; the memory test shows IC 7-16
  GOOD. With `.7/.8` in the second bank (`0x6000000`) instead, as the later
  games (`indy500`, `fvipers`, `schamp`) have it, the uploads read zeros (II6 0), both SHARCs faulted at `0x25000`
  and the 3D never started (the i960 sat in a loop at `0x9EF0`).
- **Its programs.** The geometrizer program is an earlier version of
  `indy500`'s: 2,896 of its 3,040 uploaded words appear in the same
  order in `indy500`'s, identical up to `0x20086`. The copro program is
  closest to the Jaleco games' (2,012 of 2,283 words), with the same
  loop at `0x2059A` whose end points back into an earlier identical
  loop (`adsp2106x.md`). Neither reads the copro's external bus beyond
  the FIFOs (the set has no copro data ROMs). The upload is
  `fvipers`' kind (32-bit IOP writes, SYSCON `0xA110`/`0x3110` and WAIT
  first, C6 the length: `0x8ED` and `0xBE0` words) without MSGR7, so the
  geometrizer checks every command word's tag; no word failed it in
  coin-start runs through a match.
- **Texture RAM.** Its texels go only to `0x11100000` and `0x11300000`,
  the upper halves, as `fvipers`' and `schamp`'s: with 2 MB banks the
  rasteriser read blank (all index 0) texels and the match drew dark
  (docs/games/vstriker.md).
- *Unconfirmed:* at boot it writes a byte 0 to `0x1400000` (i960
  `0x3B80`), which no other game here touches; the board model ignores it
  and nothing reads it back.

## Last Bronx (`lastbrnx`)

Sega, 1996, the latest of the 2B games here. Everything that stopped it
was in the SHARC core or in registers every Model 2 game shares, and
each was *confirmed* from its code and by running it:

- **Its SHARC programs are in the program ROM.** There is no `.7/.8`
  pair and no copro data ROM. The geometrizer upload (i960 `0x6BC8`,
  the program at `0x1B10`) is `fvipers`' kind: SYSCON `0x3100` then
  `0x3110`, WAIT, MSGR7 untouched, II6 = `0x20000`, IM6 = 1, C6 = 3,442
  words (`0x2856` halves at `0x1B10`), DMAC6 = `0xA1`, then the halves to
  EPB0. The copro upload (`0x98D0`) writes the registers as halves like
  `overrevb`'s, without SYSCON or WAIT: MSGR7 = `0x6716` (written twice,
  so the register holds `0x67166716`; *unconfirmed* what it is for: no
  instruction of the copro program addresses MSGR7 directly), DMAC6 = 0, **II6 = 0**, IM6 = 1, C6 =
  6,566 words, DMAC6 = `0xA1`, then 19,698 halves (the count is the word
  at `0x133DC`) from `0x99F8`. The program starts with zero words
  (NOPs) in both.
- **II6 = 0 loads the program.** The host DMA took the index as it was,
  below the SHARC's internal memory, and the words went nowhere:
  the copro faulted at the vector after the DMA and the 3D never ran.
  The program runs on the real chip, so the DMA's index must be decoded
  by its low 17 bits (`adsp2106x.md`, "Model 2B: the upload"); the core now
  takes an index below `0x20000` as `0x20000 +` it.
- **The interrupt dispatcher reads `0xE80000` as a pair.** Its handler for
  i960 vector 240 (`0xC30`) loads the pair with `ldl` and services the
  lowest set bit of the *second* word (`0xE80004`) until it is empty.
  reModel returned the enable mask there, which stays set: the vblank
  handler (`0x920`) was entered over and over (20,808 times in the first
  207 frames, 100 a frame), the interrupt never returned and the main
  program never ran (3,115 distinct instructions in 600M, a black
  screen). The word must hold the pending requests; reModel returns the
  requests that are enabled (`peripherals.md`).
- **The copro program depends on MODE1's effect latency.** Its
  collision and distance routine (copro `0x20B4E`) sets SRRFL
  (`BIT SET MODE1 0x400`, the secondary R0-R7) and in the next
  instruction multiplies R0 by R7 as they were: with the swap taking
  effect at once it multiplied the new, stale registers and the result
  indexed memory with random addresses (703 distinct stray external
  reads in 300M instructions, none after the fix). A write to MODE1 now
  takes effect after one more instruction (UM §E.2.1; `adsp2106x.md`).
  Nothing else's behaviour changed: the 16 other games' state hashes are
  the same.
- **Tile RAM repeats through its 2 MB window.** The gauge routine
  (i960 `0x13520`, `0x139D0`) builds the life gauge's fill patterns as
  tile `0x8080` and up, the map entries including their priority bit, and
  writes pattern *n* at `0x1080000 + n * 32`, i.e. at `0x1181000` for the
  first, then draws tile `0x80`. Only the low 20 address bits reach the
  RAM (`tilemap.md`), so reModel maps `0x1100000-0x11FFFFF` onto the lower
  megabyte, for every game, which no other game's behaviour disagrees
  with; before, the gauges drew empty (nothing was ever written at
  tile `0x80`).
- **Its geometrizer program** is the shared code (the one `overrevb`,
  `schamp` and the others upload) with three commands of its own, 0x1A-0x1C
  (`geometrizer-2b.md`, "Last Bronx's program"). The copro program
  (6,566 words) uses the FIFOs and buffer RAM only (nothing in a
  coin-start run touches another external address) and is the heaviest of
  the 2B copros here: 425M instructions in a 600M-instruction coin-start
  run (`vstriker`'s: 340M).
- **Texture RAM** is as `indy500`'s: it uploads to the lower halves
  (`0x11000000`, `0x11200000`), so the banks stay 2 MB.

## Timing

The SHARCs are clocked at 40 MHz (general knowledge, *unconfirmed* for
this board) and execute an instruction a cycle; against the i960's
25 MHz at the model's 3 cycles an instruction, the copro runs 5
instructions per i960 instruction (`sharc::STEPS_PER_INSN`), the way the
2C copro runs 2 TGP words per i960 instruction for its 20 MHz. An i960
read of an empty result FIFO lets the copro run until it has a result, as
on 2C. The geometrizer runs a frame at a time on its worker, as on 2C. The
flags the programs drive (copro FLAG2 error, FLAG3 waiting; geometrizer
FLAG1-3) go nowhere in the model.

Both stop being stepped while their program spins on a flag input
(`Sharc::spinning`: a `JUMP` to itself or a loop of NOPs ending on a
FLAG input) until the input changes: the copro spends most of its time
waiting for commands that way. This changes nothing the program can see;
the state hash is identical with it off.

## Speed

The geometrizer SHARC is the cost: about 310,000 instructions a race
frame (frames 3000-3313 of a coin-start run). Measured from a save state
in the race (`--load-state`, 658 frames), native:

| | `overrev` (2C) | `overrevb` (2B) |
|---|---|---|
| i960 + copro, i960 recompiled | 4.8 ms | 6.7 ms (the copro SHARC about 2 ms, interpreted) |
| i960 + copro, interpreted | 8.1 ms | 9.9 ms |
| geometrizer, interpreted | | 16.8 ms (20.8 before the interpreter's inlining and f64 fast path) |
| geometrizer, recompiled | 5.9 ms (TGP) | 9.3 ms (slowest frame 12.4) |
| race, all recompiled, geometrizer on its thread | 15x real time | 1.8x |

Since the SHARC programs run as blocks (`docs/recompiler.md`), the
recompiled geometrizer takes 3.6 ms a frame (slowest 5.0; frames
2000-4141) and the race runs at 3.3x real time; `sgt24h`'s, the heavier
one, 6.1 ms (slowest 10.3). In WebAssembly (Node, `docs/web.md`) it took
10.7 ms a frame (worst 12.5) as single steps where the interpreter took
33 ms; with blocks 4.8 ms, and 8.1 ms for `sgt24h`, inside the 17.4 ms of
a frame. `doa`'s whole coin-start run (600M instructions, 4,141 frames)
runs at 2.69x real time recompiled natively (geometrizer slowest frame
14.3 ms) and 0.82x interpreted (slowest 50.3 ms).
So `overrevb` runs at full speed recompiled,
natively and in the browser; the SHARC recompiler is described in
`docs/recompiler.md`. Interpreted it still races below real time, and the
front ends then slow the game to what the geometrizer manages
(`docs/play.md`, `docs/web.md`).

Input latency is the same on both boards: `make latency GAME=overrevb`
shows the game reading the wheel at the start of the frame after the
change and the display list and polygons changing in that same frame, so
at full speed the player draws it one frame later (the browser two).

The copro is much lighter: 121 million instructions in a 600M-instruction
coin-start run against the geometrizer's 900 million, once its waits
aren't stepped (stepping them, it ran 2,000 million in the first half of
that run alone, and the run took nearly three times as long).

## Open questions

- What `0x980020`, `0x430002` and FLAG1/FLAG3 of the geometrizer and
  FLAG3 of the copro drive, and what is at `0x1C80004` on 2B.
- What resets the SHARCs (only one upload each happens per boot).
- The FIFO depths, and how long a geometrizer read of an empty list FIFO
  waits on the real board.
- The SHARC clock and wait states (`WAIT` = `0xC9400` copro,
  `0xB400` geometrizer, both written by the programs) aren't modelled.
- Sega's 2B games upload their own copro programs and may use more of
  the board than the ones running (`overrevb`, `sgt24h`, `dynabb97`,
  `doa`, `indy500`, `fvipers`, `schamp`, `vstriker`, `lastbrnx`) do.
- Whether the copro's data ROM and buffer RAM repeat through their
  banks; what the bits of `0x980014` are; what is at `0x12800000`.
- How the list DMA tells jumps from data words, and what makes the
  texture RAM banks 1 MB on `fvipers`' and `schamp`'s boards.
- What `lastbrnx` writes to the copro's MSGR7 (`0x6716`) is for, what the
  interrupt controller's second word really returns, and what the three
  texture-point commands of its geometrizer program are for (`fn 0x1B`
  in particular, which no run sends).
- What the device at `0x1D80000-0x1D87FFF` that `doa` checks for is
  (*unconfirmed* whether it sits on the game board or the ROM board),
  and how it turns the block into its answer (above).

## Other findings

- *Confirmed (`overrevb`, by running them recompiled):* neither SHARC
  program writes its own code in a coin-start run into a race, and
  everything they execute is reached statically from the vectors and
  their dispatch tables (2,111 copro words, 2,987 geometrizer words);
  `docs/recompiler.md`. *Unconfirmed* for paths a coin-start run doesn't
  take (the geometrizer's code upload command, fn 0x1D, would load code;
  any such word runs on the interpreter).
- *Confirmed (`sgt24h`):* boot clears work RAM (`0x500000-0x57FFFF`,
  its callers' stack frames included) from inside the function called at
  `0x710`, then returns. That works only because the i960KB keeps recent
  callers' local registers on-chip: its data sheet says the cache "holds up
  to four local register frames" (the current one and three callers'), and
  the oldest is written to memory only when a fifth procedure is entered;
  `flushreg` writes the cached sets out (i960 Jx manual, and the 80960KB
  Programmer's Reference Manual, 270567-001, ch. 4). With the cache modelled (`i960::cpu::LocalCache`)
  the unpatched ROM boots past `0x710` to "CHECKING NETWORK NOW", "STAND
  ALONE" and "INSERT COIN" on the 2C model with the 2B stubs; without it,
  the return reloads the zeroed frame and faults at IP 0. That it is
  exactly three cached sets is the data sheet's, not tested by the game.
- *Confirmed (`dynabb97`, ROM and by running it):* its copro program
  differs from the other games' from `0x20099` on; its geometrizer
  program is the Jaleco games' code word for word, so that program looks
  like the board's standard one (*unconfirmed* beyond these three games).
- *Confirmed (`doa`, from the code):* on return the i960 takes the
  caller's frame from PFP with only bits 0-3 (return status and the
  prereturn-trace flag) cleared, not the six low bits the 80960KB manual
  says it ignores (270567-001, ch. 4 and appendix E: new frames are
  aligned on 64 bytes). `doa`'s task switch loads FP, PFP and SP by hand
  with 16-byte-aligned frames (the first at `0x5018D0`); clearing bits 4
  and 5 on return lands it on the wrong frame, and the game never gets as
  far as uploading its SHARC programs. Calls still open 64-byte-aligned
  frames (`i960::cpu::Cpu::ret`).
- *Confirmed (from the programs, the games' I/O and by running them):*
  the geometrizer is render-only. Its program writes only the rasteriser
  FIFO, its polygon RAM and polygon memory address, and a handshake
  strobe; no 2B game reads its host window (`0x840000`), and with `--geo
  native` (the SHARC runs no frame) coin-start runs of 600M instructions
  end with the SHARC's state hashes (`sgt24h` `282c8d6fd5151c21`, `doa`
  `78f308b4da95cefb`, `schamp` `0dd9ca5f3388771f`). Its command set,
  formats and arithmetic (32-bit floats rounded toward zero, `RSQRTS` and
  two Newton steps for computed normals) are in
  `docs/hw/geometrizer-2b.md`, with the native implementation that
  reproduces its stream word for word. `indy500`'s and `fvipers`'
  program is the same but for one bug: in the computed-normal modes it
  scales the first vertex's y by the x focal length. `vstriker`'s is
  `indy500`'s without its repeated stores and with its data words in
  program memory at `0x20C00`: the same output.
- *Confirmed (by fuzzing the program, `make geo-fuzz`):* its code for
  models in its own memory ("fast" polygon RAM, which no game uses) sends
  the last word twice when an object ends by its count, and loses track of
  the previous edge after a computed-normal triangle with link type 0 or
  2. A board test (fn 0x0E) whose 32 pattern words aren't a walking one
  runs on through the list: the routine it calls sits in the last
  instructions of a loop (the ADSP-2106x manual, §3.5.1.1). *Unconfirmed*
  on hardware: these follow from the SHARC model.
