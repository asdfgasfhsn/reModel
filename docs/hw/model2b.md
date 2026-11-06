# Model 2B-CRX: the board as `overrevb` uses it

Status: 2026-11-07. Remodel runs `overrevb` (Over Rev, the 2B version),
`sgt24h` (Super GT 24h), `dynabb97` (Dynamite Baseball '97) and `doa`
(Dead or Alive) from boot
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
below. This page
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
| `0x980020` | written 0 right after each bit 31 is set | *unconfirmed* what it does; Remodel ignores it |
| `0x884000` | with `0x980000` bit 31 set: the copro's EPB0 (the upload's data); otherwise writes feed the copro's command FIFO and reads take its results, as on 2C | upload loop; the boot's FIFO drain (i960 `0x23300`) |
| `0x880000 + 16·fn` | tagged copro commands, as on 2C: `value & 0x800FFFFF \| fn << 23` | the first command, `0x808` to `0x880080` (fn 8), passes the copro program's check |
| `0x980004` bit 0 | copro output FIFO empty, as on 2C | the drain loop reads `0x884000` until it is set |
| `0x804000` | with `0x980008` bit 31 set: the geometrizer's EPB0; otherwise the list data port, as on 2C | upload loop |
| `0x9C0000` / `0x9C0004` | sound UART (8251 type): data / mode, command and status, one register a 32-bit word, the byte in bits 7-0 (2C: `0x1C80000`, two registers a word) | init writes 0, 0, 0, `0xD0` (internal reset), `0x4E` (mode), `0x37` (command) to `0x9C0004`, then polls its bit 0 (TxRDY); the sound commands go to `0x9C0000` and the 68000 plays them |
| `0x1C80004` | written (16-bit) with `0x40` and `0x4E` during init: 2C's UART control address | *unconfirmed* what is there on 2B; nothing reads it, and treating it as the UART would only reset it again |

**The upload** (i960 `0x1FEC0` for the copro, `0x23330` for the
geometrizer): set bit 31, write 0 to `0x980020`, write the DMA channel 6
registers through the window as 16-bit halves (the geometrizer first gets
MSGR7 = `0x7378842E`), then 12,288 halves to EPB0, then clear bit 31. The
details and the boot that follows are in `adsp2106x.md`, "Model 2B: the
upload".

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
Remodel resets a SHARC into host boot mode when its bit 31 rises.

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
address is used: there are no copro data ROMs on this board.

The result FIFO's depth is *unconfirmed*; Remodel reports it full at 64
words, which the program only ever waits on.

## The geometrizer SHARC

| SHARC side | What |
|---|---|
| external `0x400000`, read | the list FIFO, fed by the list DMA from buffer RAM as on 2C. The program reads it without polling a flag, so the hardware must hold a read until a word is there (*unconfirmed*: Remodel hands it the next list word directly) |
| external `0x410000`, write | the rasteriser FIFO |
| external `0x420000 + a`, write | polygon RAM word `a` (fn 5 with address bit 24 clear; Remodel maps `a` below `0x10000`) |
| external `0x430000`, write | polygon memory address |
| external `0x430001`, read | the polygon memory word at that address: the polygon ROM from `0x800000` up (word addressed, as on 2C), polygon RAM below. Every model vertex and attribute word is read this way, one address write per word, the data read after it |
| external `0x430002`, write | written (any value) at each frame handshake, with FLAG1 set around it; *unconfirmed* what it signals (frame done?) |
| SHARC internal `0x30100+` | the "fast" polygon RAM: models at addresses with bit 24 set are read straight from internal memory (fn 5 writes there when bit 24 is set) |
| (none of the three rows above) | `overrevb` never writes polygon RAM or draws from it: in a coin-start run every model comes from the ROM through `0x430000`/`0x430001`, so those paths are the program's, not exercised |
| FLAG0 input | frame start, as F0 on the 2C geometrizer: the program polls it inside its loops and abandons the frame when it rises |
| FLAG1 output | raised around the `0x430002` write at each frame handshake |
| FLAG2 output | pulsed around each command word it reads; toggled forever after a failed command check (a check skipped when MSGR7 holds `0x7378842E`, which the upload writes). *Unconfirmed* what it drives |
| FLAG3 output | raised by END until the next frame starts (unless a test command ran since the last object); *unconfirmed* what it drives |

**Frame protocol** (*confirmed* from the program; Remodel's geometrizer
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
So Remodel's renderers (software and GPU) take it unchanged: there is no
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
  can't be told from this, and Remodel doesn't need to: each runs its own.

`overrevb` and `overrev` then render their own lists alike scene by scene
(attract, menus, races; compared from `make run ... SNAP=300` of both).

## Timing

The SHARCs are clocked at 40 MHz (general knowledge, *unconfirmed* for
this board) and execute an instruction a cycle; against the i960's
25 MHz at the model's 3 cycles an instruction, the copro runs 5
instructions per i960 instruction (`sharc::STEPS_PER_INSN`), the way the
2C copro runs 2 TGP words per i960 instruction for its 20 MHz. An i960
read of an empty result FIFO lets the copro run until it has a result, as
on 2C. The geometrizer runs a frame at a time on its worker, as on 2C.

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
  `doa`) do.
- What the device at `0x1D80000-0x1D87FFF` that `doa` checks for is
  (*unconfirmed* whether it sits on the game board or the ROM board),
  and how it turns the block into its answer (above).
