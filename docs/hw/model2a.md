# Model 2A (2A-CRX): board map, TGP interface, ROM layout

What is known about the 2A board from `srallyc` (Sega Rally Championship,
revision C), the only 2A game here. Everything comes from the game's own
code and from running it on the board model (`model2::bus` with
`Board::Model2A`, `model2::tgp2a`); where a statement is a guess it says
so. The 2A's processor, the Fujitsu MB86234, is in `mb86234.md`.

**Status (milestone 5).** The board model boots `srallyc`, passes its
coprocessor load check, runs its tile layers and **draws its 3D, textured**:
the attract mode's rally scenes (cars, track, forest, sky) and, with the
coins in, the game's menus and races, and **the race is drivable** (§9). The TGP runs the uploaded program on
the MB86234 interpreter (`crates/mb86234`, `mb86234.md`), its external bus
on the geometrizer's buffer RAM: the program writes each object's matrix
into the display list (§7). The 2A geometrizer is fixed-function hardware
with no model of its own here: reModel draws with the 2C native
geometrizer's command handling (`Core::Fixed`, `Program::Fixed` in
`geo_native_2c.rs`) over the polygon ROM as §5 lays it out, and its texture
RAM is the video RAM of §3 (*confirmed* by the pictures: §7). `srallyc` is
in `romset::games::ALL` and plays (the launcher's Model 2A group; a race
through the Desert stage to its finish and on into the next is in
`docs/games/srallyc.md`).
`--tgp2a hle` swaps the chip for milestone 2's function model
(`model2::tgp2a_hle`, §6) for comparison.

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
hardware, unlike 2B's SHARC and 2C's second MB86235. The display lists
the game builds go to `0x804000` as on 2C, in the 2C geometrizer's command set
(§7: the 2B and 2C programs reimplement this hardware's interface). The
board model runs them through the 2C native geometrizer (§7).

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

*Confirmed (by the pictures, §7):* this is **texture RAM**, the 2C's
render RAM at `0x11000000` with its 16-bit words spread to 32 bits: the
rasteriser's sheet addressing (`geometrizer.md` §5.1) reads it unchanged
once each word `k` of the 2A block is taken as 16-bit word `k` of the 2C
one. Bank 0 is `0x12000000`, bank 1 `0x12400000` (2C: `0x11000000`,
`0x11200000`), which is why the game's two copies land where they do (bank
0 words 0-`0x5FFFF`, bank 1 words `0x60000`-`0x7FFFF`: the 1 MB source is
one 512K-word run split over the two). The 256-word table at `0x12800000`,
each value 0..127 twice (`0,0,1,1,2,2,...`), is the **luma table**: the
2C's render RAM `+0x400000` (table 0, linear), read as the low byte of each
word (`geometrizer.md` §5.1). Texels, luma and the colour translation
tables of the palette RAM all behave as on 2C.

The board model maps 8 MB at `0x12000000` and 64 KB at `0x12800000` as
plain RAM for 2A boards only (`Model2Bus::new`) and keeps each 16-bit word
written there also at its 2C place in the render RAM the rasteriser reads
(`mirror_video_2a`); `remodel run` dumps both blocks to
`out/GAME/vram2a-*.bin`.

## 4. The TGP's interface as the i960 uses it

### Upload and load check (*confirmed*, frame 79)

1. write `0x980000` = `0x80000000` (the TGP held in reset for loading);
2. 1,665 stores of 32-bit words to `0x884000`;
3. write `0x980000` = 0;
4. write 0 to `0x880080` (command port 8);
5. read `0x884000`: the game requires **8**, else it prints "Coprocessor
   Load Error." and stops. The program answers 8 (`mb86234.md`, load
   check).

The TGP program is in the game's program ROM at `0x5AE94`, not in a
separate chip. *Confirmed (`srallyc`'s code):* the 2A TGP's program is
**uploaded by the game**, not internal to the chip as general knowledge
had it. `vcop`, `daytona` and `vf2` each carry their own TGP program (a
6-word vector and a dispatch table of 73, 73 and 112 entries; from the
scoping pass); the programs differ between games. So the TGP needs an
interpreter for its (undocumented) instruction set, not a
function-by-function model.

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
  number. The call log recognises headers this way (`tgp2a::header`): a
  float argument could look like one by chance, so call counts are a
  reading of the log, not the chip's.
- Arguments are 32-bit words, mostly IEEE floats.
- A **reply** is that many reads of `0x884000`, which on the real board
  would wait for the TGP. The game does not poll the status register
  (`0x980004`, read once).
- A **command port** write (`0x880000 + 0x10*F`) reaches the TGP as
  `F << 23 | value & 0x7FFFFF` (*confirmed* for the load check: the
  program loops until a word's exponent field is 8, and the game's only
  such write is to port 8).
- In the model the TGP runs at once on each host write until it waits for
  input (no timing); a read finds the answer ready.
- The TGP's program has a **dispatch table of 128 entries** (words
  0xAF-0x12E of the image), indexed by function number F (word `0xAF + F`):
  every function the game called (31 of them) has a slot pointing into the
  program (a jump-like word `0xBF60xxxx`); unused numbers point at a common
  return. (Milestone 1 read it as 130 entries from `0xAE`, one off: the
  main loop's `3B0000AF` is the base. See `mb86234.md`.)

### The functions the game calls

From a 700 M-instruction coin-start run (4,832 frames) with the program
running on the interpreter, calls in the attract mode. Args and replies
are words per call (constant for each function in the whole run). Frames
are the first and last the function was called. §6 says what each does.
(Milestone 2's counts, with the function model answering, differ: wrong
answers sent the game down other paths, and the model's ignorance of
`0x55` turned its words into other "calls".)

| F | Args | Reads | Calls | Frames | What it is (§6) |
|---|---|---|---|---|---|
| 1 | 0 | 0 | 1 | 80 | clear the error counter |
| 2 | 0 | 1 | 4,752 | 80- | the error counter, once a frame |
| 3 | 1 (0) | 0 | 4,753 | 79- | set the command counter, once a frame |
| 4 | 0 | 1 | 3,907 | 925- | read the command counter |
| 5 | 0 | 0 | 85,563 | 1,352- | the current matrix into the display list |
| 8 | 0 | 1 | 1 | 79 | the load check |
| 0x20, 0x21 | 0 | 0 | 103,316 | 1,352- | push, pop |
| 0x22 | 12 | 0 | 3,359 | 1,352- | set the matrix |
| 0x23 | 0 | 0 | 3,360 | 79- | reset the stack |
| 0x25 | 0 | 0 | 18,890 | 1,352- | identity |
| 0x26 | 0 | 12 | 3,359 | 1,352- | read the matrix |
| 0x27 | 3 | 0 | 96,889 | 1,352- | translate |
| 0x28 | 3 | 0 | 11,458 | 1,352-4,294 | scale |
| 0x29-0x2B | 1 | 0 | 36,604, 48,241, 18,631 | 1,352- | rotate about x, y, z |
| 0x2C | 3 | 3 | 15,064 | 1,352- | transform a point |
| 0x2F | 3 | 3 | 1,522 | 1,352-4,650 | normalise |
| 0x32-0x34 | 1 | 0 | 36,012 each | 2,023-4,650 | rotate (16-bit angle) |
| 0x42-0x44 | 4, 1, 4 | 0, 3, 0 | 5,073 each | 2,280-4,650 | vector registers |
| 0x48 | 1 | 0 | 5,073 | 2,280-4,650 | transform a vector register |
| 0x54 | 3 | 0 | 3,358 | 1,352- | point along a direction |
| 0x55 | 10 | 0 | 79,586 | 1,352-4,629 | a scaled billboard into the list |
| 0x58 | 6 | 1 | 5,715 | 1,352- | distance |
| 0x5C | 6 | 1 | 15,372 | 1,352- | 2D cross product |
| 0x5D | 7 | 0 | 254,190 | 1,352- | a billboard into the list (the commonest call) |
| 0x5E | 7 | 0 | 1,198 | 2,023- | translate and face a point |
| 0x5F | 5 | 0 | 5,990 | 2,023- | translate, rotate z and x, into the list |

The attract mode's 3D scenes start at frame 1,352; before that only the
frame calls 2, 3, 4 and 0x23 occur.

The race (milestone 5: a 680 M-instruction coin-start run, 4,693 frames,
the accelerator held from the car select and the wheel turned at three
corners; the race starts at frame 2,790) adds these, all from frame 2,375
(the start line) on:

| F | Args | Reads | Calls | What it is (§6) |
|---|---|---|---|---|
| 7 | 0 (1 in the list) | 1 | 9,732 | a word of the data ROM |
| 0x15 | 1 | 1 | 8,602 | sine (the library's) |
| 0x50 | 6 | 1 | 581 | distance between two points (frames 4,364-4,394) |
| 0x51, 0x54 | 3 | 0 | 7, 14 | point along a direction (on the start line) |
| 0x52 | 0 (9 in the list) | 0 | 9,732 | find the course polygon under a point |
| 0x53 | 1 | 17 | 9,732 | that polygon's words |
| 0x59 | 6 | 1 | 2,318 | dot product |
| 0x5B | 6 | 3 | 3,068 | cross product |
| 0x60 | 0 | 0 | 14 | the current matrix into shared RAM |

Functions 7 and 0x52 take their arguments from the list in shared RAM
(below), not the FIFO. `remodel run` lists the table (`tgp
calls`; headers are recognised by their form, so a float argument could
be miscounted as a call, though none is known to be), writes
`out/GAME/tgp-calls.txt` (every call in order, with frame and i960
address, the first 200,000 events) and the program as `out/GAME/tgp.bin`.

## 5. ROM layout (`srallyc`)

| Chips | Role | Evidence |
|---|---|---|
| `epr-17888c.12` (low) / `epr-17889c.13` (high) | i960 program, 1 MB, 16-bit interleaved | boot record valid (PRCB `0xB0`, first IP `0x420`) |
| `mpr-17746.10` / `-17747.11` | main data at `0x2000000` (4 MB) | its first word is a count (300) and its next words pointers `0x2000010`, `0x2001C30`, ... into itself, then floats; texel source at `0x2200000` (3) |
| `mpr-17744.8` / `-17745.9` | main data at `0x2400000` | read in full by the game (1 MB); looks like image data; purpose unconfirmed |
| `mpr-17884.6` / `-17885.7` | main data at `0x2800000` | the game reads floats here (`0x2860000`-`0x28C0000`, `0x29E0000`): model vertex data |
| `mpr-17748.16` (low) / `-17750.20` (high), `mpr-17749.17` (low) / `-17751.21` (high) | **polygon ROM**: two banks of 1M 32-bit words, at model addresses `0x800000` and `0x900000` | §7: every polygon attribute word parses and the normals are unit length. **The pairs are not the numbers' neighbours**: `.16/.17` are the low halves, `.20/.21` the high halves, of banks 0 and 1 |
| `mpr-17753.25` (even words) / `-17752.24` (odd words) | **texture ROM**: texture headers and point lists as 16-bit words (2C: `.27/.25`), read at the address the display list's tpa/tha give (bit 23 clear) | §7: headers decode (texture, size, sheet position, luma table, colour), the pictures are right. **`.25` is the even half**, the reverse of the order the chips are numbered in |
| `mpr-17754.29` (low) / `-17755.28` (high) | **the TGP's data ROM**: a 32-bit bank at address `0x800000` of its external bus | §9: the course data its collision functions read; the first begins with a count (300) |
| `opr-14742a.45`/`.46`, `.58`/`.59`, `.62`/`.63` | the TGP's lookup tables (a sine table of floats, reciprocal and inverse-square-root mantissas) | numerically, see below |
| `epr-17890.30` | 68000 sound program | the sound board runs it |
| `mpr-17756.31`, `-17757.32`, `-17886.36`, `-17887.37` | SCSP samples, 8 MB back to back at the sound 68000's `0x800000`, in this order | the sound program's sample table (`0x609C26`) fills all four slots, and only this order puts each loop's repeat after its end and each chip's data end at its slot's last sample (docs/games/srallyc.md); `.31/.32` were once taken for a second TGP data bank, at `0x900000`, which the TGP never reads |
| `mpr-16310.15`, `-16311.16`, `-16312.14` | ramps (`00 E0 C0 A0 80 60 40 20`...): unconfirmed what reads them; not used | |

The main data pairs are **packed**, 4 MB each from `0x2000000`
(`Game::packed`), where 2B's and 2C's are 8 MB slots: the 8 MB layout
puts the vertex data's pair at `0x3000000`, where the program does not
read. The polygon and texture ROM are packed too (the bank at `0x900000`
is 1M words after `0x800000`, not 2M as in a 2C slot). The zip also holds the older program revision `b`
(`epr-17888b.12`, `epr-17889b.13`) and `ic12.bin`/`ic13.bin`, which are
the program pair again under other names; they are not used.

*Confirmed (numerically):* the shared Model 2/2A chips are maths tables:
`opr-14742a.45/.46` a quarter-wave sine (65,536 steps a circle, within
3e-8), an arctangent, and slope and intercept pairs for `1/x` and
`1/sqrt(x)` (the TGP's function unit reads these: one multiply and one
subtract give full precision); `.58/.59` reciprocal mantissas, `.62/.63`
inverse square root mantissas for even and odd exponents.

## 6. The functions (run on the interpreter, milestone 3)

What each function of `srallyc`'s program does, as the interpreter runs
it; `make tgp-check` checks each against the maths with random arguments
(`mb86234.md`). Word addresses are the code's (slot F = word `0xAF + F`).
The last column says where milestone 2's function model
(`model2::tgp2a_hle`, read from the code without running it, kept behind
`--tgp2a hle`) agrees.

*Confirmed (running `srallyc`'s program):* the MB86234's instruction
words: an ALU operation in parallel with a transfer (registers, two
RAMs, the external bus) or with loads of the multiplier's inputs, 24-bit
immediates, control registers with a repeat count, conditional jumps
and loads, calls, and a loop on a counter register (`BF00`, which the
course search closes its loop with; milestone 4's reading of it as a
jump on the zero flag made the race's physics run away). The
interpreter runs every function the game calls and its library (add to
tangent) to the maths for random arguments, and the course search on a
made-up course (`make tgp-check`); a few forms only uncalled code uses
remain unknown.

| F | Takes / gives | What it does | Code | Model |
|---|---|---|---|---|
| 1 | 0 / 0 | clears the error counter (the game calls it once, after function 2 reports the start-up value) | `0x13D` | not modelled |
| 2 | 0 / 1 | the error counter (RAM0 `0x7C`); start-up leaves `0x6000` in it | `0x13F` | gives 0 |
| 3 | 1 / 0 | sets the command counter; resets the display-list cursor to `0x6000` and clears flag bit 1 | `0x15E` | agrees (cursor not modelled) |
| 4 | 0 / 1 | the command counter (counting every command, 3 included) | `0x168` | agrees |
| 5 | 0 / 0 | **writes the current matrix into the display list**: a geometrizer Matrix command (fn `0x0B` in bits 23-27) and 12 words, at the byte offset the next entry of the list at shared RAM `0x6000` gives (§7) | `0x14E` | not modelled |
| `0x20`, `0x21` | 0 / 0 | push (copies below depth 7, counts above), pop (nothing at depth 0) | `0x25B`, `0x26F` | agrees |
| `0x22`, `0x26` | 12 / 0, 0 / 12 | set, read the current matrix (3x3 row by row, then the translation) | `0x27E`, `0x32E` | agrees |
| `0x23`, `0x25` | 0 / 0 | reset the stack (the matrix kept), identity | `0x282`, `0x327` | agrees |
| `0x27` | 3 / 0 | translate: `t += v M` | `0x332` | agrees |
| `0x28` | 3 / 0 | scale: row i times factor i | `0x348` | agrees |
| `0x29`-`0x2B` | 1 / 0 | rotate about x, y, z by a float angle (radians): `M = R M` with `R` for x `[[1,0,0],[0,c,-s],[0,s,c]]`, y `[[c,0,s],[0,1,0],[-s,0,c]]`, z `[[c,-s,0],[s,c,0],[0,0,1]]` (sine and cosine by the program's polynomial) | `0x363`, `0x384`, `0x3A4` | **opposite sign** (the model guessed the transposes) |
| `0x2C` | 3 / 3 | transform a point: `v M + t` | `0x3C2` | agrees |
| `0x2F` | 3 / 3 | normalise | `0x30E` | agrees (to 1 ulp) |
| `0x32`-`0x34` | 1 / 0 | the same rotations with a 16-bit angle (65,536 a turn), sine and cosine from the function unit's ROM | `0x35F`, `0x380`, `0x3A0` | not modelled |
| `0x42`-`0x44` | 4 / 0, 1 / 3, 4 / 0 | vector register i: set, read, add | `0x404`.. | agrees |
| `0x48` | 1 / 0 | transforms vector register i by the current matrix, in place | `0x44C` | not modelled |
| `0x54` | 3 / 0 | turns the matrix to point along a direction: two rotations after which `v M = (0, 0, \|v\|)` | `0x4B5` | not modelled |
| `0x51` | 3 / 0 | the same two rotations with the other signs: the matrix's third row becomes `v / \|v\|` | `0x48E` | not modelled |
| `0x50` | 6 / 1 | the distance from (w0, w1, w2) to (w3, w4, w5) | `0x46C` | not modelled |
| `0x59` | 6 / 1 | a dot product, `w0 w1 + w2 w3 + w4 w5` | `0x530` | not modelled |
| `0x5B` | 6 / 3 | a cross product, `u x v` with `u = (w5, w0, w2)`, `v = (w4, w3, w1)` | `0x55F` | not modelled |
| `7` | list / 1 | the data ROM's word at the offset the next word of the shared-RAM list gives (from `0x800000`) | `0x16D` | not modelled |
| `0x52` | list / 0 | the course search (§9): from nine words of the list (a point x, y, z, a block of the course data, a word not used, three cells of the block, a slot), the first polygon of the three cells (in the order 7th, 8th, 6th word) that contains the point in x and z; records where it is in a table at RAM0 104 + slot | `0x58A` | not modelled |
| `0x53` | 1 / 17 | the polygon the search found for a slot: its 16 words and its attribute word | `0x63C` | not modelled |
| `0x60` | 0 / 0 | copies the current matrix to shared RAM `0x7F00 +` the list's next word | `0x3DB` | not modelled |
| `0x55` | 10 / 0 | push, translate (3), face a point (4, as `0x5E`), scale (3), function 5, pop: a scaled billboard | `0x66E` | not modelled (milestone 2's table missed it: the model took the call as one with no words) |
| `0x58` | 6 / 1 | the distance between (w1, w3, w5) and (w0, w2, w4) | `0x51C` | agrees (to 1 ulp) |
| `0x5C` | 6 / 1 | a 2D cross product of differences, `(w5 - w0)(w4 - w3) - (w2 - w3)(w1 - w0)` | `0x577` | **wrong** (guessed a distance) |
| `0x5D` | 7 / 0 | push, `0x5E`, function 5, pop: an object's matrix into the display list (254,190 calls in the run) | `0x674` | not modelled |
| `0x5E` | 7 / 0 | translate by (w0, w1, w2), then a rotation about y by the direction (w3 - w4, w5 - w6), normalised (a billboard: the game passes its own x and z again as w4 and w6, and the point to face as w3 and w5) | `0x679` | not modelled |
| `0x5F` | 5 / 0 | push, translate (3), rotate z then x by 16-bit angles, function 5, pop | `0x67B` | not modelled |

`make tgp-check` checks every function in this table against its maths
(the course search on a made-up course, below).

Also in the program, not called by the game: the rest of the library at
`8`-`0x17` (add, subtract, multiply, divide, a residual `x - y (x / y)` at
`0x0C`, abs, negate, square root, integer/float conversions, cosine,
tangent; `make tgp-check` checks them), ROM sine and cosine (`0x35`,
`0x36`), an arctangent through the function unit (`0x3E`), the 3x3 product
(`0x24`), the stack depth (`0x2D`), more vector-register operations
(`0x40`, `0x41`, `0x45`-`0x47`), the matrix copy back from shared RAM
(`0x61`), a peek (`6`), the cosine of the angle between two vectors
(`0x5A`: their dot product over the product of their lengths) and others not examined
(`0x2E`, `0x56`, `0x57`).

### What the game does with it

With the chip the game's TGP traffic in a 4,832-frame coin-start run is
101 million TGP words; the 3D calls run from frame 1,352 (`0x2C` 15,064
times, `0x5C` 15,372: more than with the function model, whose wrong
answers sent the game down other paths). No unknown word runs, no read
finds the FIFO empty, no call fails to finish.

## 7. The geometrizer (milestone 4)

### What the TGP gives it (milestone 3, *confirmed*)

The TGP's only output to the geometrizer is function 5 (alone or inside
`0x55`, `0x5D`, `0x5F`): it writes a geometrizer **Matrix command** into
the display list in buffer RAM, which is the TGP's shared RAM (32K words,
word addresses). The handshake:

1. The i960 builds the frame's list through the geometrizer ports as on
   2C. Before each object it leaves 13 words free (it sets the write
   pointer `0x801008` past them) and appends the object command (fn
   `0x01`: tpa, tha, oba, obc).
2. It puts the byte offset of each gap in a table at buffer RAM byte
   `0x18000` (TGP word `0x6000`), in order (`0x4A0`, `0x4E8`, ... in one
   frame: 18 words apart, 13 for the matrix and 5 for the object).
3. Function 3 (once a frame) resets the TGP's cursor to `0x6000`; each
   function-5 call takes the next entry, shifts it right by 2 (bytes to
   words) and writes there the command word (the exponent field of R13
   set to `0x0B`: fn `0x0B` in bits 23-27; the low bits are whatever R13
   held, the calling function's command word when 5 is called alone) and
   the current matrix's 12 words.

The list of one attract frame then parses with the 2C command set: fn 4
(texture point RAM, 216 words at `0x800100`), `0x16` (LOD), 7 (mode), 8
(z-sort 8.0), 6 (22 lighting entries), 3 (window: (0, 128)-(496, 512),
centre (248, 320)), 9 (focal lengths 350, 350), `0x0A` (light), then the
matrices from the TGP and the objects, then END.

### The command words (*confirmed*)

The i960's command words reach buffer RAM with only the fn field set
(bits 23-27: it writes 0 to the command port, whose address gives the fn),
the TGP's matrix words carry whatever its register held in the low
bits, and the first word of a list is `0x02000000` (fn 4): none has the
2C's check pattern (fn repeated in bits 8-12 and 0-4). The hardware takes
the command from bits 23-27 alone, so the 2C native geometrizer's command
check is off for the 2A (`Program::Fixed`).

### The polygon ROM (*confirmed*)

A model is the 2C's standard format (`geometrizer.md` §3.1): two vertices
P0, P1, then polygons of attr, normal, Pa, Pb (10 words each, a triangle
padding Pb with zeros), as IEEE single floats, with the attribute word's
link bits repeated in bits 23-24. What was missing in milestone 3 was the
chip pairing. Each chip is 16 bits wide and holds **half** of the 32-bit
words, but **bank 0 is `.16` (low half) with `.20` (high half) and bank 1
is `.17` with `.21`**; `.24/.25` are not polygon data (§5). Evidence:

- At the model `oba = 0x80334C` (`obc = 0x279`, 633 polygons) the chips
  `.16` and `.20` read as one 32-bit word stream give an attribute word
  that is shaped as one (type in bits 0-1, bits 2-7 clear) in all 633
  polygons and a unit-length normal in 513 (the rest are polygons with no
  normal, zero); every other pairing of the six chips scores 0-96 of
  633 attribute words. Read as 16-bit streams on their own, `.20` and
  `.21` each show a model with plausible normals (the milestone 3
  reading): a float cut to its top half is a float with a random low
  half, and the **low halves are in a different chip**. The other pair
  of chips at the same word index holds a second, unrelated model, in
  step with the first because both banks are laid out polygon by polygon.
- Model addresses from `0x800000` index bank 0 and from `0x900000` bank 1
  by 32-bit word (`oba = 0x91119F`, `0x92144E` parse the same way in
  `.17`/`.21`: 316 of 316 and 40 of 40 attribute words).
- A native geometrizer reading the polygon ROM this way draws the cars,
  road and trees (below).

### The geometrizer

`Core::Fixed` (`model2::geo`) is a geometrizer with no program: the
2C native geometrizer's command handling (`geo_native_2c`, in its
`Program::Fixed` form: the shared program's maths with the squarings
table set at start and no command check) on the list as buffer RAM
gives it, with the polygon ROM above. It runs from the first frame with
whatever `--geo` says. The display list's matrices, windows, focal
lengths, lighting, mode and light vector are all the 2C commands. The 2A
chip's own arithmetic, lighting and clipping are *unconfirmed*: this is
the 2C programs' behaviour, which the pictures agree with (the cars are
lit from the side, the road and trees shaded by their normals). (Smooth
shading's self-check recomputes every polygon's luma from the side
channel exactly, 100% of 16.8 million in a run, but against reModel's own
model of the chip: `geometrizer.md`, "Lighting side channel"; it says
nothing about the hardware.)

### Textures and colour (*confirmed* by the pictures)

- Tpa and tha point at the texture ROM (`.25`/`.24`, 16-bit words,
  address bit 23 clear) or at texture point RAM (bit 23 set; fn 4 writes
  it, as on 2C).
- Texel sheets: the video RAM at `0x12000000` (§3), addressed as the 2C's
  render RAM.
- Luma tables: `0x12800000` (§3). Colours: palette RAM `0x1802000`+ and
  the three colour translation tables from `0x1810000`, as on 2C (the
  game fills them).

With these the rasteriser draws the attract mode's scenes as pictures
(headless renders at frames 1,400 to 1,900 of `make run GAME=srallyc`): a
road through a forest with a blue sky and clouds, rally cars in white and
red with their shadows, dust, and the game's own overlays; and a
"INSERT COIN(S)" screen over the same.

## 8. What is left

- The 2A geometrizer's own arithmetic (rounding, the lighting equation's
  details, specular), clipping and z-sort are the 2C program's, not
  checked against the 2A hardware.
- The `0x1A14000` link board, the drive board (the wheel's force
  feedback: the game writes its commands, nothing answers them) and the
  meaning of the course data's attribute words and of each polygon's last
  four words.
- `out/srallyc/tgp.bin` (`make run GAME=srallyc` writes it; `make
  tgp-disasm GAME=srallyc` lists it, `make tgp-check GAME=srallyc` checks
  it) and the call log (`tgp-calls.txt`).

## 9. The TGP's data ROM and the race (milestones 4 and 5)

*Confirmed (by the run):* the TGP's external bus has a **data ROM from
word address `0x800000`**, in both of the chip's bus modes (the bus
instructions `63400000`, shared RAM, and `63000000`, the function unit):
`.29` and `.28` as one 32-bit bank (`.29` the low half), as the 2C's TGP
has its `CoproData` at `0x800000`. (`.31` and `.32`, once taken as a second
bank at `0x900000`, are sample ROMs: over a coin-start race the TGP reads
only `0x800000-0x81FFFF`, in the attract mode nothing from the ROM, and the
race's state hash is the same either way; docs/games/srallyc.md.) The address registers are 24 bits wide: the program makes a
ROM address by adding `-0x800000` to the pointer it was given and, in the
function that reads a block, steps through it from there. (Milestone 3 had
them 16 bits and no ROM: nothing the attract mode calls reads it.)

**What the race does with it** (*confirmed*, from the i960's code, the
TGP's program and the ROM). The i960 keeps a table (RAM `0x5DCAC0`, 12
bytes an entry: `0xDE30, 0x1ADA0, 0x2003E00`; `0x272E0, 0x3C110,
0x200BA00`; ...: two word offsets into the data ROM and a main-data
address) and, four times a frame (once for each wheel of the player's
car; more when other things touch the ground), puts words into the TGP's
list in shared RAM (the list function 5 takes its offsets from, §7) and
calls function `7`, which reads the ROM word at the first offset (`0xDE30`
holds 300), then `0x52` with nine list words: the wheel's x, y and z, the
second offset (a **block** of the course data), a word the search doesn't
use, three **cells** of the block (word offsets into it: the cell under
the wheel and two neighbours, by the i960's reckoning) and a slot. The
course data:

- A **cell** is four words: the number of polygons, the byte offsets from
  the ROM's start of its polygons and of their attribute words, and 0.
  The polygons (16 words each) are followed directly by the attribute
  words (one each): the count times 16 is the distance between the two
  offsets in every cell.
- A **polygon** is four corners (x, y, z: course units, which are metres
  as far as the car shows: its wheels are 1.8 apart across and 2.75 along,
  and it falls at 10 a second squared) and four more words (*unknown*:
  presumably the plane for the height). Bit 2 of its attribute word says
  it has four corners (else three, and the fourth is not read); the other
  bits are *unknown* (the surface?).

`0x52` copies each polygon's corners in x and z and tests the point
against its edges (the cross products `(c_i - p) x (c_i+1 - p)`): all of
them non-negative (sign bits clear) is a hit, all negative ends that cell,
mixed goes on to the next polygon; after the cell's last polygon it goes
on to the next cell (in the order of the 7th, 8th and 6th list words). It
writes where the hit is (the attribute word's and the polygon's
addresses, `ROM - 0x800000`) at RAM0 104 + slot, or 104 in both words if
none of the three cells has one. `0x53` (the slot) gives the polygon's 16
words and its attribute word. **Without the ROM** (milestone 3), function
7 answered 0, `0x52` was never called, `0x53` answered zeros and the car
fell through the road at the start of every race.

**The race drives** (milestone 5, *confirmed* by running it). Milestone 4
had the search's loop word `BF00` as "jump if the zero flag is clear": the
car stood on the start line, but the search ran on past a cell's
polygons into the next cells' (16 passes for a cell of 6 under a rear
wheel on the start line, 47 for another) and took the first polygon there
that held the wheel in plan view, so the game got the ground of other
places. Once the car moved the physics ran away: the speedometer at
250 km/h within five seconds and the car turning left and tumbling with
the accelerator held, creeping at 25 km/h and veering off the road with
it released. `BF00` is a **loop on a counter** that the search loads with
the cell's polygon count (`mb86234.md` has the evidence). With it, in a
coin-start race with scripted inputs (`make run GAME=srallyc
INPUT=coin-start,gas=0xff@2300-6000,wheel=0xa0@3085-3165,wheel=0x70@3360-3420,wheel=0xb0@3590-3640
STEPS=680000000 SNAP=100`):

- released, the car stays at 0 km/h; held, it pulls away at the start
  (frame 2,790) through the gears of the automatic, 100 km/h after about
  1.2 s and 214 km/h, where it levels off, after 4.5 s; braking from
  207 km/h it is at 37 km/h 1.7 s later. The speedometer agrees with the
  distance the wheels cover per frame (60 frames a second) in course
  units, read as metres;
- with the wheel centred it runs straight down the first street of the
  course's town, between the "SEGA" and sponsor barriers, and into
  the barrier where the road bends right; turned right there (`0xA0`) it
  takes the bend at 200 km/h without touching, then left and right again
  over the hills onto the dirt road, through a checkpoint (the time
  extended from 35 to 54 s at 0'26"), past spectators and a lattice tower,
  hitting a barrier twice where the script steers late;
- hitting a barrier slows it (214 to 150 km/h) and turns it off the
  barrier; steered hard into one it spins, ends facing back and the game
  shows "逆走!" (wrong way); airborne over crests it falls at 10 m/s².

The acceleration and braking (2 to 3 g) are high for a real car but the
gravity and the speedometer are consistent with the motion, the top speed
is a flat limit, and the TGP's part is the ROM's data and functions that
`make tgp-check` checks, so this is taken to be the game's own tuning
(*unconfirmed* against a cabinet).

### Controls (*confirmed* on the game's input test)

The test menu (`test` held, `service` to move, `test` released and
pressed again to choose) has an INPUT TEST screen listing CHUTE1, CHUTE2,
SHIFT1-4, VR, START, TEST SW, SERVICE and the values HANDLE, ACCEL, BRAKE
and HANDLE(DRIVE). Holding each bit of the I/O ports in turn on that
screen, the game gives:

| Switch | Port, bit |
|---|---|
| CHUTE1 (coin 1) | 0, `0x01` |
| CHUTE2 (coin 2) | 0, `0x02` |
| TEST SW, SERVICE | 0, `0x04`, `0x08` |
| VR (the view button) | 0, `0x20` |
| START | 0, `0x40` |
| SHIFT3, SHIFT4 | 1, `0x10`, `0x20` |

SHIFT1 and SHIFT2 do not respond to any single bit, because the **shift
lever** is an H gate read as a 3-bit code (*confirmed* from the game's
input polling, and by driving a manual car): the game takes port 1 bits
4-6 (pressed = 1) as an index into an 8-byte table (`0, 3, 4, 0, 0, 1, 2,
0`), the gear:

| Gear | Port 1 bits held |
|---|---|
| 1 | `0x40` and `0x10` |
| 2 | `0x40` and `0x20` |
| 3 | `0x10` |
| 4 | `0x20` |
| neutral | none, or any other combination |

(`0x40` is the gate's left column, `0x10`/`0x20` up and down.) With the MT
Celica (the wheel turned left on the car select screen while the
accelerator confirms: the wheel's position picks the car) and the gears
held in turn (`gear1`...`gear4` in the manifest), the dashboard shows 1,
2, 3 and the car pulls through them. The game also reads port 0 bits 6
and 7 with port A bit 0 set (the EEPROM's mode, in which the board model
gives those bits as 1) into a pair of active-high flags with their
presses and releases, which nothing in the race was seen to need:
*unknown*.

HANDLE, ACCEL and BRAKE are the analog channels 0, 1 and 2 (the screen shows
the value given: `0x11`, `0x22`, `0x33`); HANDLE(DRIVE) reads `FFH`,
the drive board's answer, which is not modelled (the game writes
its command byte at register 4, 4,396 times in a 2,900-frame run). The
car select screen says "select with the wheel, confirm with the
accelerator": the accelerator at `0xFF` confirms. The game wants
**two coins** for a credit by default ("CREDIT 1/2" after the first), so
`coin-start` pays twice and then presses start: it reaches the car select
(frame 1,800), the countdown and the start (frame 2,000-2,790).
*Confirmed by driving:* the wheel at `0x80` runs straight, below it steers
left and above it right (`0x20` and `0xC0` turn the car within a second;
on the car select screen left and right pick the two MT cars); the pedals
are `0x00` released (the car stays still) and `0xFF` pressed (full
throttle, full braking).

