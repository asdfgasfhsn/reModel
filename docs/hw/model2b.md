# Model 2B-CRX: the board as `overrevb` uses it

Status: 2026-11-06. Remodel runs `overrevb` (Over Rev, the 2B version) from
boot through attract into races, with both SHARCs running the programs the
game uploads. This page is what the board model (`crates/model2`:
`bus.rs`, `sharc.rs`, `geo.rs`) is built on. The SHARC core itself is
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
frame (frames 3000-3313 of a coin-start run), 20 ms on the interpreter
(65 ns an instruction; 21.5 ms before the tick, ALU flag and rounding
paths were trimmed), with spikes to 24 ms. Measured from a save state
in the race (`--load-state`, 658 frames), native:

| | `overrev` (2C) | `overrevb` (2B) |
|---|---|---|
| i960 + copro, i960 recompiled | 4.8 ms | 6.7 ms (the copro SHARC about 2 ms) |
| i960 + copro, interpreted | 8.1 ms | 9.9 ms |
| geometrizer | 5.9 ms (TGP recompiled) | 20 ms |
| race, geometrizer on its thread | 15x real time | 0.86x |

So `overrevb` races below real time even natively, and in WebAssembly
the geometrizer takes 33 ms (`docs/web.md`, "Timing without a browser").
Input latency is the same on both boards: `make latency GAME=overrevb`
shows the game reading the wheel at the start of the frame after the
change and the display list and polygons changing in that same frame, so
at full speed the player draws it one frame later (the browser two).
What made `overrevb` feel unresponsive was speed: the native player
dropped to half the refresh rate (main thread and geometrizer, 27 ms, in
series), and the browser let its geometry worker fall ever further
behind. Both are fixed in the front ends (`docs/play.md`, `docs/web.md`):
now the game slows to what the geometrizer manages, the polygons at most
a frame (native) or two (browser) behind. Full speed needs the
geometrizer at about 12 ms natively and 14 ms in WebAssembly: 1.7 and
2.4 times faster, more than tuning the interpreter gives (its profile is
flat: dispatch, condition and flag work, the ALU and float paths each a
few percent). That takes a SHARC recompiler. The copro is much lighter: 121 million
instructions in a 600M-instruction coin-start run against the
geometrizer's 900 million, once its waits aren't stepped (stepping them,
it ran 2,000 million in the first half of that run alone, and the run
took nearly three times as long). A SHARC
recompiler, like the MB86235 one, is the way to go faster.

## Open questions

- What `0x980020`, `0x430002` and FLAG1/FLAG3 of the geometrizer and
  FLAG3 of the copro drive, and what is at `0x1C80004` on 2B.
- What resets the SHARCs (only one upload each happens per boot).
- The FIFO depths, and how long a geometrizer read of an empty list FIFO
  waits on the real board.
- The SHARC clock and wait states (`WAIT` = `0xC9400` copro,
  `0xB400` geometrizer, both written by the programs) aren't modelled.
- Other 2B games (`sgt24h`, Jaleco's other one, boots the same two SHARC
  programs; Sega's 2B games have their own) may use more of the board.
