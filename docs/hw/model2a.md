# Model 2A (2A-CRX): board map, TGP interface, ROM layout

What is known about the 2A board from `srallyc` (Sega Rally Championship,
revision C), the only 2A game here. Everything comes from the game's own
code and from running it on the board model (`model2::bus` with
`Board::Model2A`, `model2::tgp2a`); where a statement is a guess it says
so. The 2A's processor, the Fujitsu MB86234, is in `mb86234.md`.

**Status (milestone 1).** The board model boots `srallyc`, passes its
coprocessor load check against a **stand-in** (the TGP's program is
uploaded and recorded, never run), and runs the attract mode's tile
layers: the export warning, the title logo, the credit and © line, the ranking table. No
3D is drawn: every 3D call the game makes goes unanswered. `srallyc` is in
`romset::games::IN_PROGRESS`, not `ALL`, so no front end lists it.

## 1. What the 2A shares with 2B and 2C (confirmed by the run)

i960KB at 25 MHz, the program ROM at 0, the same I/O chip at `0x1C00000`,
interrupt controller `0xE80000`, timers `0xF00000`, tile RAM `0x1000000`,
palette `0x1800000`, backup SRAM `0x1D00000`, the geometrizer's command
ports at `0x800000` (eye mode, pointers `0x801008` / `0x802008` /
`0x803008`, data appends `0x804000`: the same traffic as 2C's), buffer RAM
`0x900000`, and the sound link UART at `0x1C80000`, as 2C (the 2B moves
it). The 2A's sound board (68000 + SCSP) boots from `epr-17890.30` with the
sample ROMs `mpr-17886.36/.37` on the 2B/2C sound model unchanged, and
plays in attract (*confirmed*: sound link bytes at frames 79, 925 on, and a
mix with a peak of about 2,500).

The 2A geometrizer takes no uploaded program: nothing but the TGP's is
written to the upload ports (*confirmed*), so it is fixed-function
hardware, unlike 2B's SHARC and 2C's second MB86235. The display lists the
game builds go to `0x804000` as on 2C. Nothing here models the
geometrizer's output; the model's geometrizer, never booted, takes the
words and draws nothing.

## 2. Where the 2A differs

| Address | What | How known |
|---|---|---|
| `0x12000000`-`0x127FFFFF` | **Video RAM of 32-bit words holding 16 bits each** (the high half is written as 0). Most likely texture RAM, see 3. | the game copies main data into it; |
| `0x12800000`-`0x1280FFFF` | a small table, 256 words | written once at boot |
| `0x884000`-`0x887FFF` | the TGP's FIFO port, **one 32-bit word per store** | 4 |
| `0x880000`-`0x883FFF` | the TGP's command ports (function = address bits 4-11), used once, for the load check | 4 |
| `0x980000` | TGP control: bit 31 set while the program is uploaded | as 2C |
| `0x980004`, `0x980014` | TGP status; `0x980014` reads all ones until a program is in | as 2C |

On 2C the same upload is 64-bit words (two stores each, low half first);
2A's program ROM image is 1,665 words of 32 bits (*confirmed*: the stores
equal the ROM's words from `0x5AE94` on).

Not seen, so not mapped: the 2C's `0x11000000` render RAM is still mapped
from the board model's common map, but nothing was found to use it on 2A;
reads of `0x10800000` once a frame in attract (from frame 925, returning 0
without effect), `0x1A14000` (the link board's status, as on 2C),
`0x1C00200`. Their purpose is unknown.

## 3. The `0x12000000` region

*Confirmed by the code's copy loop:* the game fills it at boot (from frame 13) with a loop at an i960 address in RAM, 32-bit stores of 16-bit
values, from main data:

- Source: `0x2200000`-`0x22FFFFF` (1 MB of the first main data pair).
  The first 768 KB goes to `0x12000000`-`0x12180000` and the last 256 KB to
  `0x12580000`-`0x12600000`, one 16-bit value per 32-bit word, in source
  order (verified against the ROM bytes: the RAM's 16-bit stream equals
  the ROM's low-half, high-half order from `0x2200000`).
- Values like `0x1111`, `0x1133`, `0xFFFF`, `0xEEEE`, `0x8888`, `0xAAAA`:
  runs of equal nibbles, as 4-bit texels would be.

*Unconfirmed:* that this is texture RAM (the 2B and 2C games copy their
texel sheets from main data to `0x11000000` the same way,
`geometrizer.md` §3), and how the rasteriser addresses it. The 256-word
table at `0x12800000` holds each value 0..127 twice in a row
(`0,0,1,1,2,2,...`); the game writes it once and never reads it back
(function unknown: a luma or fade ramp is a guess).

The board model maps 8 MB at `0x12000000` and 64 KB at `0x12800000` as
plain RAM for 2A boards only (`Model2Bus::new`), and `remodel run` dumps
both to `out/GAME/vram2a-*.bin`.

## 4. The TGP's interface as the i960 uses it

### Upload and load check (*confirmed*, frame 79)

1. write `0x980000` = `0x80000000` (the TGP held in reset for loading);
2. 1,665 stores of 32-bit words to `0x884000`;
3. write `0x980000` = 0;
4. write 0 to `0x880080` (command port 8);
5. read `0x884000`: the game requires **8**, else it prints "Coprocessor
   Load Error." and stops. The stand-in answers exactly this.

The TGP program is in the game's program ROM at `0x5AE94`, not in a
separate chip.

### Function calls

After the load the game talks to the TGP only through `0x884000`
(*confirmed*; 300,000 words in a 2,000-frame run):

- A **call** is a header word, then its argument words, all stored to
  `0x884000` (a burst of stores to consecutive words `0x884000`-`0x88402C`
  carries one call's 12 matrix words: the window is a single FIFO).
- The header word has the function number *F* in bits 23-30 and the same
  byte in both low bytes: `F << 23 | F << 8 | F` (for example `0x01800303`
  is function 3, `0x2E005C5C` function 0x5C). Every header in a run has
  this form; it is also what the hardware makes of a store to command port
  `0x880000 + 0x10*F` on 2C (the tag in bits 23-30), so F is that port's
  number. The stand-in recognises headers this way (`tgp2a::header`): a
  float argument could look like one by chance, so call counts are a
  reading of the log, not the chip's.
- Arguments are 32-bit words, mostly IEEE floats.
- A **reply** is that many reads of `0x884000`, which on the real board
  would wait for the TGP. The game does not poll the status register
  (`0x980004`, read once).
- The TGP's program has a **dispatch table of 130 entries** (words
  0xAE-0x12F of the image), indexed by function number: every function the
  game called (31 of them) has a slot pointing into the program (a jump-like
  word `0xBF60xxxx`), and slots 0 and 129 point nowhere. See `mb86234.md`.

### The functions the game calls

From a 700 M-instruction coin-start run (4,832 frames), calls in the
attract mode. Args and replies are words per call (constant for each
function in the whole run). Frames are the first and last the function was
called. The meaning column is what the argument counts, the ordering of
calls and the values suggest: **all guesses**, none confirmed.

| F | Args | Reads | Calls | Frames | What it may be |
|---|---|---|---|---|---|
| 2 | 0 | 1 | 4,752 | 80- | a status or frame-sync request, once a frame |
| 3 | 1 (0) | 0 | 4,753 | 79- | once a frame, the argument is 0 |
| 4 | 0 | 1 | 3,907 | 925- | once a frame in attract, from frame 925 |
| 5 | 0 | 0 | 85,563 | 1,352- | follows each draw sequence (a "pop"?) |
| 8 | 0 | 1 | 1 | 79 | the load check's ping: answered with 8 |
| 0x20 | 0 | 0 | 89,620 | 1,352- | start of a draw sequence (a "push"?) |
| 0x21 | 0 | 0 | 89,620 | 1,352- | pairs with 0x20 |
| 0x22 | 12 | 0 | 3,359 | 1,352- | sets a 3x4 matrix (12 words) |
| 0x23 | 0 | 0 | 3,360 | 79- | |
| 0x25 | 0 | 0 | 8,432 | 1,352- | |
| 0x26 | 0 | 12 | 3,359 | 1,352- | reads a 3x4 matrix back (12 words) |
| 0x27 | 3 | 0 | 88,422 | 1,352- | a vector: translate? |
| 0x28 | 3 | 0 | 11,458 | 1,352-4,294 | a vector |
| 0x29 | 1 | 0 | 31,375 | 1,352- | an angle: rotate? |
| 0x2A | 1 | 0 | 37,780 | 1,352- | an angle |
| 0x2B | 1 | 0 | 13,402 | 1,352- | an angle |
| 0x2C | 3 | 3 | 3,359 | 1,352- | transforms a vector (3 in, 3 out)? |
| 0x2F | 3 | 3 | 1,522 | 1,352-4,650 | transforms a vector (3 in, 3 out)? |
| 0x32-0x34 | 1 | 0 | 36,012 each | 2,023-4,650 | |
| 0x42 | 4 | 0 | 5,073 | 2,280-4,650 | |
| 0x43 | 1 | 3 | 5,073 | 2,280-4,650 | 1 in, 3 out |
| 0x44 | 4 | 0 | 5,073 | 2,280-4,650 | |
| 0x48 | 1 | 0 | 5,073 | 2,280-4,650 | |
| 0x54 | 3 | 0 | 3,358 | 1,352- | a vector |
| 0x58 | 6 | 1 | 5,229 | 1,352- | 6 in, 1 out |
| 0x5C | 6 | 1 | 13,436 | 1,352- | 6 in, 1 out |
| 0x5D | 7 | 0 | 254,190 | 1,352- | the commonest call: 7 words, mostly floats |
| 0x5E | 7 | 0 | 1,198 | 2,023- | |
| 0x5F | 5 | 0 | 5,990 | 2,023- | |

The attract mode's 3D scenes start at frame 1,352; before that only the
frame calls 2, 3, 4 and 0x23 occur. The sequence shape is: 0x20, a
translation (0x27), three angle calls (0x2A, 0x29, 0x2B), 0x05, then 0x21:
what a matrix stack would look like (*a guess*, to be checked against the
interpreted program).

The stand-in answers none of these: reads return 0 (the game carries on
with zeros: an empty FIFO read is not a stall in the model), so every 3D
object the game draws is built from zeros. `remodel run` lists the table
(`tgp calls`), writes `out/GAME/tgp-calls.txt` (every call in order, with
frame and i960 address) and the program as `out/GAME/tgp.bin`.

## 5. ROM layout (`srallyc`)

| Chips | Role | Evidence |
|---|---|---|
| `epr-17888c.12` (low) / `epr-17889c.13` (high) | i960 program, 1 MB, 16-bit interleaved | boot record valid (PRCB `0xB0`, first IP `0x420`) |
| `mpr-17746.10` / `-17747.11` | main data at `0x2000000` (4 MB) | its first word is a count (300) and its next words pointers `0x2000010`, `0x2001C30`, ... into itself, then floats; texel source at `0x2200000` (3) |
| `mpr-17744.8` / `-17745.9` | main data at `0x2400000` | read in full by the game (1 MB); looks like image data; purpose unconfirmed |
| `mpr-17884.6` / `-17885.7` | main data at `0x2800000` | the game reads floats here (`0x2860000`-`0x28C0000`, `0x29E0000`): model vertex data |
| `mpr-17748.16`/`.17`, `.20`/`.21`, `.24`/`.25` | polygon ROM pairs (the geometrizer's) | float-like data in `.20/.21`; order and layout unconfirmed (nothing reads them yet) |
| `mpr-17754.29`/`.28`, `mpr-17756.31`/`.32` | texture ROM pairs | the first pair starts with the count 300; unconfirmed |
| `opr-14742a.45`/`.46`, `.58`/`.59`, `.62`/`.63` | the TGP's lookup tables (a sine table of floats, reciprocal and inverse-square-root mantissas) | numerically, README "What's known about Model 2A" |
| `epr-17890.30` | 68000 sound program | the sound board runs it |
| `mpr-17886.36`, `-17887.37` | SCSP samples | the sound plays |
| `mpr-16310.15`, `-16311.16`, `-16312.14` | ramps (`00 E0 C0 A0 80 60 40 20`...): unconfirmed what reads them; not used | |

The main data pairs are **packed**, 4 MB each from `0x2000000`
(`Game::packed`), where 2B's and 2C's are 8 MB slots: the 8 MB layout
puts the vertex data's pair at `0x3000000`, where the program does not
read. The zip also holds the older program revision `b`
(`epr-17888b.12`, `epr-17889b.13`) and `ic12.bin`/`ic13.bin`, which are
the program pair again under other names; they are not used.

## 6. What milestone 2 starts from

- `out/srallyc/tgp.bin` (`make run GAME=srallyc` writes it; `make
  tgp-disasm GAME=srallyc` splits it): 1,665 words, 130-entry dispatch at
  word 0xAE, 81 distinct entry points.
- The call log (`tgp-calls.txt`): each call's arguments and, for 3D, the
  i960 code that makes them, in order; an interpreter's replies can be
  compared with what the game then does with them (the 3D objects drawn).
- The reply words the game reads: function 2, 4 (1 each), 0x26 (12),
  0x2C, 0x2F, 0x43 (3), 0x58, 0x5C (1).
