# Model 2A (2A-CRX): board map, TGP interface, ROM layout

What is known about the 2A board from `srallyc` (Sega Rally Championship,
revision C), the only 2A game here. Everything comes from the game's own
code and from running it on the board model (`model2::bus` with
`Board::Model2A`, `model2::tgp2a`); where a statement is a guess it says
so. The 2A's processor, the Fujitsu MB86234, is in `mb86234.md`.

**Status (milestone 2).** The board model boots `srallyc`, passes its
coprocessor load check and runs the attract mode's tile layers: the export
warning, the title logo, the credit and (c) line, the ranking table. The
TGP's program is uploaded and recorded but **not run**: its instruction
set is only partly known (`mb86234.md`). In its place `model2::tgp2a_hle`
does what each of the program's functions was *read* to do (§6), so the
game now gets answers (a matrix it sets reads back, the stack pushes and
pops, a point transforms) where milestone 1 returned zeros. No 3D is drawn
yet: the geometrizer model for 2A is missing (§1), and the functions that
output to it (`0x05`, `0x5D`...) are only counted. `srallyc` is in
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
- The TGP's program has a **dispatch table of 128 entries** (words
  0xAF-0x12E of the image), indexed by function number F (word `0xAF + F`):
  every function the game called (31 of them) has a slot pointing into the
  program (a jump-like word `0xBF60xxxx`); unused numbers point at a common
  return. (Milestone 1 read it as 130 entries from `0xAE`, one off: the
  main loop's `3B0000AF` is the base. See `mb86234.md`.)

### The functions the game calls

From a 700 M-instruction coin-start run (4,832 frames), calls in the
attract mode. Args and replies are words per call (constant for each
function in the whole run). Frames are the first and last the function was
called. The meaning column is what milestone 1's argument counts suggested; §6
has what reading the program's code found for each, with how sure that is
(it replaces this column where they differ).

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

## 6. The function model (milestone 2)

`model2::tgp2a_hle` (recognised by the program's hash, so another 2A
program gets none) models each function by what its code was read to do.
**It is not the chip**: the program is not run. For each function: how
sure, and where it is in the program (word address; slot F = word
`0xAF + F`; `mb86234.md` for the cells).

| F | Takes / gives | What it does | Sure | Code |
|---|---|---|---|---|
| 2 | 0 / 1 | the unknown-function error count (cell `0x7C`, written only by the error path) | confirmed | `0x13F` |
| 3 | 1 / 0 | sets the command counter (cell `0x65`) to the word | confirmed | `0x15E` |
| 4 | 0 / 1 | reads the command counter; the main loop adds 1 after every command, `3` itself included | confirmed | `0x168` |
| 5 | 0 / 0 | writes the current matrix (12 words) to an output buffer in external memory (the geometrizer's list?) | structure confirmed, destination unknown | `0x14E` |
| 0x20 | 0 / 0 | **push**: copy the current matrix to the next stack slot; at depth 7 and over it only counts | confirmed | `0x25B` |
| 0x21 | 0 / 0 | **pop**: nothing at depth 0; a pop of an uncopied push (depth 8 and over) only counts | confirmed | `0x26F` |
| 0x22 | 12 / 0 | set the current matrix (3x3 rotation row by row, then the translation) | confirmed | `0x27E` |
| 0x23 | 0 / 0 | reset: stack pointer to the base, depth 0; the matrix is left as it was | confirmed | `0x282` |
| 0x25 | 0 / 0 | identity: 12 zeros, then 1.0 at words 0, 4, 8 | confirmed | `0x327` |
| 0x26 | 0 / 12 | read the current matrix | confirmed | `0x32E` |
| 0x27 | 3 / 0 | translate: `t += v M` (three chains of three multiplies) | maths read, convention assumed | `0x332` |
| 0x28 | 3 / 0 | scale (the game sends 40, 1, 40 for a flat disc): each row by its factor | guessed (rows vs columns) | `0x348` |
| 0x29-0x2B | 1 / 0 | rotate about x, y, z by the angle (radians; the game passes unwrapped angles like -5489): `M = R M`, right-handed, row vectors | **signs and axis order assumed** | `0x363`, `0x384`, `0x3A4` |
| 0x2C | 3 / 3 | transform a point: `v M + t` (the nine multiplies take elements `3i + j`) | maths confirmed, layout read | `0x3C2` |
| 0x2F | 3 / 3 | normalise (sum of squares, inverse square root, three multiplies) | confirmed | `0x30E` |
| 0x42 / 0x43 / 0x44 | 4 / 0, 1 / 3, 4 / 0 | the four vector registers: set `(i, x, y, z)`, read `i`, add `(i, x, y, z)` | confirmed | `0x404`, `0x40C`, `0x414` |
| 0x58 | 6 / 1 | pairs the words (0, 1), (2, 3), (4, 5), takes each pair's difference, returns the root of the sum of squares: a distance, *if* the pairs are what the code makes them | operation guessed | `0x51C` |
| 0x5C | 6 / 1 | **guess** (the same as 0x58) | guessed | `0x577` |
| 0x32-0x34, 0x48, 0x54, 0x5D-0x5F | take words | output to the geometrizer; not modelled (the words are consumed) | | |

Not called by the game and not modelled: the library of two-input and
one-input float functions at `8`-`0x17`, the 3x3 concatenation `0x24`, and
the vector operations around `0x40`-`0x48` other than the three above.
The game's own test menu has no coprocessor test with known answers, so
nothing here was checked against expected values: the checks are the
round trips and the identities in `model2::tgp2a_hle`'s tests.

### What the game does with it

With the model the game's per-frame TGP traffic changes (a run of 4,832
frames: `0x20`/`0x21` 102,913 times, 13,000 more than with zeros; `0x25`
18,890 times, twice the zeros' 8,432; the geometrizer still counts 518,683
commands it cannot use): the matrices it reads back are real, so its own
bookkeeping goes further. The picture is the same blue attract screen: the
geometrizer has no model for 2A.

## 7. Where milestone 3 starts

- Decode the multiply group and the ALU codes (`mb86234.md`, "Not
  known") by executing the short functions with known mathematics, then
  run the program in place of the model; the model's tests then become
  checks of the interpreter.
- What the geometrizer takes from the TGP: functions `0x05` and `0x5D` are
  the heavy traffic (254,190 calls of seven words); the display lists the
  game sends to `0x804000` are the same traffic as 2C's, but the 2A
  geometrizer has no uploaded program and no model.
- `out/srallyc/tgp.bin` (`make run GAME=srallyc` writes it; `make
  tgp-disasm GAME=srallyc` splits it) and the call log
  (`tgp-calls.txt`).
