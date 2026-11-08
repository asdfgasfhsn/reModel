# Model 2A (2A-CRX): board map, TGP interface, ROM layout

What is known about the 2A board from `srallyc` (Sega Rally Championship,
revision C), the only 2A game here. Everything comes from the game's own
code and from running it on the board model (`model2::bus` with
`Board::Model2A`, `model2::tgp2a`); where a statement is a guess it says
so. The 2A's processor, the Fujitsu MB86234, is in `mb86234.md`.

**Status (milestone 3).** The board model boots `srallyc`, passes its
coprocessor load check and runs the attract mode's tile layers: the export
warning, the title logo, the credit and (c) line, the ranking table. The
TGP **runs the uploaded program** on the MB86234 interpreter
(`crates/mb86234`, `mb86234.md`), its external bus on the geometrizer's
buffer RAM: every function the game calls runs, the answers are the
program's own, and the program writes each object's matrix into the
display list (§7). No 3D is drawn yet: the 2A geometrizer is
fixed-function hardware with no model here, and its polygon ROM format is
not the 2C one (§7). `srallyc` is in `romset::games::IN_PROGRESS`, not
`ALL`, so no front end lists it. `--tgp2a hle` swaps the chip for
milestone 2's function model (`model2::tgp2a_hle`, §6) for comparison.

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
game builds go to `0x804000` as on 2C, in the 2C geometrizer's command set
(§7: the 2B and 2C programs reimplement this hardware's interface).
Nothing here models the geometrizer's output; the model's geometrizer,
never booted, takes the words and draws nothing.

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
   Load Error." and stops. The program answers 8 (`mb86234.md`, load
   check).

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
frame calls 2, 3, 4 and 0x23 occur. `remodel run` lists the table (`tgp
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

## 6. The functions (run on the interpreter, milestone 3)

What each function of `srallyc`'s program does, as the interpreter runs
it; `make tgp-check` checks each against the maths with random arguments
(`mb86234.md`). Word addresses are the code's (slot F = word `0xAF + F`).
The last column says where milestone 2's function model
(`model2::tgp2a_hle`, read from the code without running it, kept behind
`--tgp2a hle`) agrees.

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
| `0x55` | 10 / 0 | push, translate (3), face a point (4, as `0x5E`), scale (3), function 5, pop: a scaled billboard | `0x66E` | not modelled (milestone 2's table missed it: the model took the call as one with no words) |
| `0x58` | 6 / 1 | the distance between (w1, w3, w5) and (w0, w2, w4) | `0x51C` | agrees (to 1 ulp) |
| `0x5C` | 6 / 1 | a 2D cross product of differences, `(w5 - w0)(w4 - w3) - (w2 - w3)(w1 - w0)` | `0x577` | **wrong** (guessed a distance) |
| `0x5D` | 7 / 0 | push, `0x5E`, function 5, pop: an object's matrix into the display list (254,190 calls in the run) | `0x674` | not modelled |
| `0x5E` | 7 / 0 | translate by (w0, w1, w2), then a rotation about y by the direction (w3 - w4, w5 - w6), normalised (a billboard: the game passes its own x and z again as w4 and w6, and the point to face as w3 and w5) | `0x679` | not modelled |
| `0x5F` | 5 / 0 | push, translate (3), rotate z then x by 16-bit angles, function 5, pop | `0x67B` | not modelled |

Also in the program, not called by the game: the library at `8`-`0x17`
(add, subtract, multiply, divide, a residual `x - y (x / y)` at `0x0C`, abs,
negate, square root, integer/float conversions, sine, cosine, tangent;
`make tgp-check` checks them), ROM sine and cosine (`0x35`, `0x36`), an
arctangent through the function unit (`0x3E`), the 3x3 product (`0x24`),
the stack depth (`0x2D`), more vector-register operations (`0x40`,
`0x41`, `0x45`-`0x47`), matrix copies to and from shared RAM (`0x60`,
`0x61`), peeks (`6`, `7`) and others not examined (`0x2E`, `0x50`-`0x53`,
`0x56`, `0x57`, `0x59`-`0x5B`).

### What the game does with it

With the chip the game's TGP traffic in a 4,832-frame coin-start run is
101 million TGP words; the 3D calls run from frame 1,352 (`0x2C` 15,064
times, `0x5C` 15,372: more than with the function model, whose wrong
answers sent the game down other paths). No unknown word runs, no read
finds the FIFO empty, no call fails to finish.

## 7. What the TGP gives the geometrizer (milestone 3)

*Confirmed (by running the program and decoding the frame's list):* the
TGP's only output to the geometrizer is function 5 (alone or inside `0x55`,
`0x5D`, `0x5F`): it writes a geometrizer **Matrix command** into the
display list in buffer RAM, which is the TGP's shared RAM (32K words,
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

The list of one attract frame (2,145 words) then parses cleanly with the
2C command set: fn 4 (texture point RAM), `0x16` (LOD), 7 (mode), 8 (z-sort
8.0), 6 (22 lighting entries), 3 (window: (0, 128)-(496, 512), centre (248,
320)), 9 (focal lengths 350, 350), `0x0A` (light), then 100 matrices from
the TGP and 109 objects, then END. The matrices are rotations (rows of
length 1) with translations of a few units.

*What a 2A geometrizer model needs*, beyond this command set:

- **The polygon ROM's format** (*not known*). The objects' model addresses
  (oba, from `0x800000`) land in the `mpr-17750.20`/`.21` pair, not the
  first pair as the 2C bank order puts them, and the data there are not
  2C models read as 32-bit words: each chip's 16-bit words form a stream
  of their own that looks like the 2C standard format with every value
  cut to its top 16 bits (`.21` at oba `0x80334C`: P0 (-328, -10.2, 20.5),
  P1 (0, 0, 0), attr `0x30A4`, normal (-0.53, -0.71, 0.46), Pa, ...), but
  the offsets don't always line up with oba (`0x80848B` points at an
  attribute word with the first edge six words before). How the two chips
  and the three pairs make up the address space is the next thing to
  find. Putting the 2C native geometrizer on the 2A lists (tried, not
  kept) draws a handful of polygons and garbage vertices because of this.
- **Textures**: the texel data the game copies to `0x12000000` (§3) is
  presumably what the rasteriser reads (2B/2C: `0x11000000`).
- The arithmetic, lighting and clipping of the fixed-function chip, which
  the 2B/2C programs only imitate.

## 8. Where milestone 4 starts

- The polygon ROM's layout and value format (above), then a 2A
  geometrizer: most likely the 2C native geometrizer's command handling
  with a 2A model reader, fed the lists as they are, then the textures.
- The TGP's unknowns (`mb86234.md`, "Not known") don't block anything the
  game calls.
- `out/srallyc/tgp.bin` (`make run GAME=srallyc` writes it; `make
  tgp-disasm GAME=srallyc` lists it, `make tgp-check GAME=srallyc` checks
  it) and the call log (`tgp-calls.txt`).
