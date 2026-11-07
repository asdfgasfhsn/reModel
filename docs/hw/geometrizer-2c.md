# The 2C geometrizer programs, and a native implementation of them

Status: 2026-11-08. What the display-list programs that the Model 2C games
upload to their geometrizer MB86235 (the TGP) do, worked out from their
behaviour and their disassembly (`remodel copro-disasm out/GAME/geo.bin`),
and how reModel implements them natively
(`crates/model2/src/geo_native_2c.rs`, the default in the player and the
browser). The board side (the geo ports, buffer RAM, the list DMA, the
frame handshake) is in `geometrizer.md` §1; the stream they produce and
the rasteriser that draws it in `geometrizer.md` §4-§6; the 2B
counterpart in `geometrizer-2b.md`; the chip in `mb86235.md`.

**Clean room.** Written from the programs' behaviour and disassembly. No
emulator's code was read. The native code is Rust written from this
description: it doesn't follow the programs instruction by instruction,
holds no table from a ROM or a program (the state the start-up code sets
up is taken from the TGP at run time), and finds a program by hashing the
words the game uploads. Everything below marked *confirmed* was checked by
running the TGP and the native implementation on the same input and
comparing their output and state word for word (`make geo-check`, `make
geo-fuzz`, below).

## Which programs

A program is recognised by `mb86235::cpu::program_hash` of the uploaded
words: the hash the recompiler names the program's crate by
(`docs/recompiler.md`), so `tgp_<hash>` in `out/recomp` is the same
program. Never by game.

| Program hash | Games | Native |
|---|---|---|
| `29177553590496aa` | `skisuprg`, `segawski`, `waverunr`, `hotd`, `bel` (identical uploads) | `Program::Shared` |
| `b850fb3484005f7b` | `stcc`: the shared program with one more status-port write at the start of its test command (fn 0x0E), which nothing sees | `Program::Shared` |
| `a49bbffc5f8b1d2a` | `overrev` | `Program::Overrev` |
| `c655943137753c1d` | `topskatr` | `Program::TopSkater` |

All four are one program in different versions: the command dispatch,
the handlers up to fn 0x0E and the object code sit at the same addresses
in each; `overrev`'s and `topskatr`'s differ from the shared one in the
places below (*confirmed*, comparing the disassemblies with branch targets
normalised). For any other program `--geo native` runs the TGP.

## Render-only

*Confirmed*: the geometrizer feeds nothing back to the i960 or the copro.

- **From the programs.** Their external writes are the output FIFO (the
  rasteriser stream), polygon RAM (fn 0x05, and A-bus addresses from
  `0x400`, which only a corrupt lighting index reaches) and the PDR port
  (status outputs: error blinks, frame-handshake bits). They read only the
  input FIFO and polygon memory.
- **From the games.** In every 2C game's I/O log (`out/GAME/io.txt` of a
  coin-start run) the i960 reads only the write pointer (`0x802008`) and
  read pointer (`0x803008`), which are its own, the frame parity
  (`0x98000C`, vblank's) and the copro's status (`0x980004`, `0x980010`,
  `0x980014`, `0x980030`): nothing the geometrizer drives. reModel's board
  model wires PDR to nothing.
- **From running it.** With `--geo native` the TGP never runs a frame and
  600M-instruction coin-start runs end with the TGP's state hashes
  (below).

So the geometrizer is a pure function of (the frame's list, polygon
memory, its own state) to (the raster stream, its new state). Its state is
its A and B RAMs (the transform, lights, lighting entries, the previous
edge, a few counters), the PR register queue (the rotation matrices) and,
in `overrev`, the AR3 register (its command log pointer).

## Arithmetic

As `mb86235.md` §1.4 and the interpreter have it (the chip's own rounding
is *unconfirmed*): IEEE single precision rounded to nearest, denormals
kept; `FRSQ` is `1/sqrt(x)` with two roundings; `CFIB` truncates toward
zero and clamps to 0-255 (a NaN gives 0); the `Z` forms (`FADDZ`, `FSUBZ`)
clamp a negative result to +0 and keep −0 and NaNs. A NaN result is the
first NaN operand quieted, else the default NaN `0xFFC00000`, in the
instruction's operand order (`mb86235::float`): left to the host, two NaNs
meeting gave different bits depending on which way the compiler ordered an
addition (the fuzzer found it), and WebAssembly may give any NaN. The
games' state hashes are the same with and without the rule.

The native code calls the same `mb86235::float` functions as the
interpreter and the recompiled code, in the programs' order and
association, so every value is the TGP's by construction. Values are kept
as 32-bit patterns.

**Stream format.** Command words are `0xFF000000 | list word >> 23`;
floats go out as their bits shifted right 8 (`geometrizer.md` §4.2).

## Frame protocol

The upload's start-up code fills the dispatch table and constants in A
RAM (and `topskatr`'s B RAM), then waits for F0. Each frame (F0 pulsed at
vblank, `geometrizer.md` §1.2) the program sends NOP and END (the last
frame's terminator; the board model discards both), clears its input
FIFO, waits for F0 to fall and reads the list until END. The first frame
after the upload goes through the start-up code's own wait instead, so it
reads a list too. Handlers poll F0 to abandon a frame that runs into the
next; the board model always finishes a frame first, so that never
happens.

`overrev`'s frame start also counts down a counter at A RAM `0x3F1-0x3F2`,
reloaded from `0x3F0`, which sets a status bit; nothing starts it (its
increment is unreachable code) and the native code keeps it anyway.

**The command check.** Each command word must have its fn (bits 27-23)
in bits 4-0 and 12-8 too (there is no switch to turn it off, unlike 2B).
A word that fails, a fn the program has no handler for, or operands that
fail a handler's check (below) send the program into a loop that blinks
the status port forever: nothing more is drawn until the next upload.

**Command log (`overrev`).** Before each command its fn goes into B RAM at
AR3, then `0xFFFFFFFF` after it; AR3 wraps to 0 at `0x3EF`.

## Memory layout

| State | Shared, `overrev` | `topskatr` |
|---|---|---|
| Dispatch table (fn 0x00-0x1F) | A `0x00` | A `0x40` |
| Constants (masks 3, `0x3FFFF`, `0x7C0000`, `0xF8000000`, `0x20000`, `0x100`, `0x200`, `0xFF000000`, `0x1F`) | A `0xD8` | A `0x10` |
| Translation (and `topskatr`'s second) | A `0x20` | B `0x30` (B `0x33`) |
| Focal lengths fx, fy | A `0x23` | A `0x33` |
| Light vector L | A `0x26` | A `0x36` |
| Previous edge P0', P1' | A `0x29`, A `0x2C` | A `0x2C`, B `0x2C` |
| Tested flag (fn 0x0E sets, objects clear; `overrev` doesn't keep it) | A `0x2F` | A `0x3F` |
| Lighting entries (4 words each), distance coefficients (32 each) | A `0x30`, A `0xB0` | A `0x60`, B `0x40` |
| Squarings per specular control | A `0xD0` | A `0xE0` |
| Attribute bits 17-0 of the polygon being sent | A `0xE3` | A `0x1B` |
| Mode: computed normals (bit 1 and up), specular mask (`0xF` or 0) | A `0xE4`, A `0xE5` | A `0x1C`, A `0x1D` |
| LOD | A `0xE6` | A `0x2F` |
| Polygon count (counted down) | A `0xE7` | A `0x31` |
| Rotation matrix (9 words) | PR 0-8 | PR 0-8, second PR 9-17 |

## Commands

fn is bits 27-23 of the command word; `cmd` is the raster command word.
"Hang" means the loop forever above.

| fn | Operands | Out | State |
|---|---|---|---|
| 0x00 NOP | — | `cmd` | — |
| 0x01 Object | tpa, tha, oba, obc | `cmd`, tpa & 0xFFFFFF, tha & 0xFFFFFF, the polygons (below) | count = obc |
| 0x02 Direct | tpa, tha, 6 words, then polygons: attr, 5 words [+3 if attr bit 0] until an attr with bits 1-0 clear | `(cmd & 0xFF00001C) \| 1` (eye bits dropped), tpa & 0xFFFFFF, tha & 0xFFFFFF, each word >> 8, each attr & 0xFFFFFF (the first attr is tested too) | — |
| 0x03 Window | 6 words | `cmd`, each `(v & 0x0FFF0000) >> 4 \| v & 0xFFF` | — |
| 0x04 Raster memory | addr, n, n words | `cmd`, addr, n, the words | — |
| 0x05 Polygon RAM | addr, n, max(n & 0x3FFF, 1) words | — | written at `EB + addr` (EB from the constant word, 0); hang if addr ≥ 2^24 (n isn't read then) or n ≥ 0x8000 |
| 0x06 Lighting | index, n, n × (bytes, coef) | — | entry `index + k`: byte 1 (ambient), byte 0 (diffuse), byte 2 (specular scale) as floats, byte 3 (specular control) as an integer; coefficient `index + k` (the coefficient first). Unchecked: 32 entries, then on into the next tables |
| 0x07 Mode | v | — | computed = v >> 1; specular mask = 0xF if bit 0 else 0 |
| 0x08 Z-sort | 1 float | `cmd`, the float >> 8 | — |
| 0x09 Focal | fx, fy | — | focal lengths |
| 0x0A Light | 3 floats | — | L |
| 0x0B Matrix | 12 floats | — | PR 0-8 (rotation), translation |
| 0x0C Translation | 3 floats | — | translation |
| 0x0D Data push | addr, n | max(n & 0x3FFF, 1) polygon memory words from addr, raw | hang if `0x1000000 − addr` or `0x8000 − n` (`overrev`: `0x1000000 − n`) is negative as a signed number |
| 0x0E Test | 32 words, nblocks, blocks of (addr, count, sum) | — | below |
| 0x0F END | — | — | the frame's end |
| 0x10 Dummy | 1 word | — | — |
| 0x14 Log data | addr, n, n words | `cmd`, addr, 4n, each word's bytes, low first | — |
| 0x16 LOD | 1 float | — | LOD |
| 0x11 (`topskatr`) | as 0x01 | as 0x01 | the compact skinned format (below) |
| 0x1B (`topskatr`) | 12 floats | — | the second matrix: PR from the index B RAM `0x0C` holds (9), the second translation |
| 0x1C (`topskatr`) | — | — | the first matrix and translation copied to the second |
| others | — | — | hang |

Loop counts of 0 are 2^32 (the loops count down after their body), which
only a broken list sends; the native code stops on loops of more than 4M.

**The test command.** The sums only set status bits, so only its reads
and stores matter:

- shared: each of the 32 words is compared with a walking one; a mismatch
  calls a routine whose flags then keep the loop going whatever its count
  (the games' tests always match). The blocks are stored in B RAM from 0
  (3 words each) and then summed; nothing if nblocks is 0. Sets the
  tested flag.
- `topskatr`: the words are compared with 1 (so every one after the
  first "fails", which only sets a status bit); the blocks are summed
  straight from the list. Sets the tested flag first.
- `overrev`: as `topskatr`, but its block loop runs at least once: an
  nblocks of 0 runs it 2^32 times.

## Object data

A model in polygon memory (ROM from `0x800000`, polygon RAM below) is two
vertices P0, P1, then polygons. The stream gets the two transformed
vertices, then per polygon `attr & 0x3FFFF`, the luma word, the distance
`>> 8`, Pa and, for a quad (attr bit 0), Pb. obc polygons are drawn (0:
until a terminator), and an attr with bits 1-0 clear ends the model
before that.

Per vertex, view space is `v' = ((t + x·m0) + y·m3) + z·m6` (and y', z'
with m1, m4, m7 and m2, m5, m8): translation first. It goes out as
`x'·fx`, `y'·fy`, `z'·1`.

**Given normals** (mode bit 1 clear): 10 words a polygon: attr, normal,
Pa, Pb (a triangle's unused). The normal is rotated: `n' = ((n.x·m0 +
n.y·m3) + n.z·m6)` and so on. With `dotp = (Pa'·n')` and `dotl = (n'·L)`
(each `(x·x + y·y) + z·z`):

- back face: `dotp < 0` (−0 and NaN count as front); `overrev`'s quads
  instead count as front only when `0 − dotp` is negative, so a zero
  `dotp` is a back face there;
- distance: `(coef·LOD)·|dotp|` (a triangle: `|dotp|·(coef·LOD)`), coef
  from attr bits 31-27, the lighting entry from bits 22-18;
- a front face is lit from `dotl`; the shared program first clamps a
  triangle's at 0 and leaves a quad's, `overrev` clamps both;
- a back face with attr bit 17 (double-sided) is lit from `−dotl` and
  sent as a front face (no back-face bit);
- any other back face sends the luma word `0x800000` (the back-face bit,
  luma 0).

**Computed normals** (mode bit 1 set; always for `topskatr`'s fn 0x11):
the model's normals are skipped. The program keeps the previous edge as
the rasteriser will link it (`geometrizer.md` §3.4): P0', P1' start as
the model's two vertices; after each polygon the link type (attr bits
9-8) gives the next edge: 0 or 2 (Pa, Pb), 1 (P0' kept, Pa), 3 (Pb, P1'
kept), with Pb = Pa for a triangle. With `a = Pa' − P0'` and `b = P1' −
P0'`, `c = b × a` (`c.x = b.y·a.z − a.y·b.z`, `c.y = b.z·a.x − a.z·b.x`,
`c.z = b.x·a.y − a.x·b.y`), `len² = (c.x² + c.y²) + c.z²`, `y = FRSQ(len²)`
then twice `y ← (3 − len²·(y·y))·(y·0.5)`, and `n = c·y`.

- back face: `Pa'·c < 0`, the dot product with the normal before
  normalising;
- distance: `(coef·LOD)·|Pa'·n|` (the shared triangle code multiplies
  `LOD·coef`); a single-sided back face sends `coef·LOD` alone, with the
  luma word `0x800000`;
- lit from `n·L`, or `−(n·L)` for a double-sided back face.

**The lighting routine.** From `d`, the value above, and the polygon's
entry (ambient, diffuse, specular scale, control):

- the diffuse term `s0`: shared, `d + 0` clamped at 0; `overrev`, its
  clamped value is passed in (so `s0` is `d` clamped, or `−dotl + 0`
  clamped);
- `lum = s0·diffuse + ambient`;
- with the specular mask and the control sharing a bit: `s = (2·d)·n.z −
  L.z`, clamped at 0 (`overrev` uses `s0` for `d`); unless the control is
  1, squared k times, k the squarings table's entry for the control (14
  bits of it, at least once): 1, 1, 1, 1, 2, 2, 2, 3 for controls 0-7
  (`overrev`'s first two entries are 0, which also squares once). The
  table isn't bounded: larger controls read the words after it (in the
  shared program a control of 15 squares 16,383 times). Then `lum = lum + s·scale`;
- the luma word is `CFIB(lum) << 15`.

**`topskatr`'s compact format** (fn 0x11, `geometrizer.md` §3.2): P0, P1,
then per polygon attr, Pa and for a quad Pb (7 or 4 words), no normals.
Each vertex picks its matrix by bit 0 of its x word (which stays in the
value): 0 the first matrix and translation, 1 the second (fn 0x1B). Two-
bone skinning of the skaters. Normals are computed as above; the
coefficient times LOD and the attribute word are kept in B `0x2F` and A
`0x30` for the polygon.

## The native implementation

`model2::geo_native_2c::Native` keeps the program's state as the TGP does:
A and B RAM, the PR queue, EB and `overrev`'s AR3. It takes a TGP's
state between frames (`Native::import`; a freshly uploaded TGP is first
run through its start-up code on a copy, `geo_native_2c::started`, so the
constants and tables are the program's own) and hands it back
(`Native::export`). A save state made in native mode stores the TGP with
the native state written in, in the format the TGP path saves, so it
loads in either mode; switching `--geo` mid-run does the same.

`GeoMode` (`geo.rs`) picks what runs a frame, for 2B and 2C alike: `Chip`
(the uploaded program on the SHARC or TGP; the `remodel` CLI's default),
`Native` (the native code when it implements the program, else the chip;
the player's and the browser's default), `Check` (the chip's output, with
the native one computed beside it from the same buffer RAM, its own copy
of polygon RAM, and compared: the stream word by word, the list words
read, and every word of A and B RAM, PR, AR3 and polygon RAM after the
frame; after a difference the native side takes the chip's state again).

Front ends: `remodel --geo chip|native|check` (`sharc` and `tgp` name the
chip too), `remodel-play --geo chip`, the browser's `?geo=chip` (also
`?geo=sharc`, `?geo=tgp`; docs/web.md).

**Smooth motion** (`geometrizer.md`, "Smooth motion"): the matrix (fn
0x0B), translation (0x0C), focal-length (0x09) and `topskatr`'s second
matrix (0x1B) handlers pass their operands through `Io::xform` before
using them, and objects (0x01, `topskatr`'s 0x11) their model address
through `Io::object`, as on 2B. fn 0x1C copies the first matrix, so it
follows the blended one. `topskatr`'s skinned skaters blend both bones.

## Verification

`make geo-check GAME=...` (750M instructions of a coin-start run, past
5,000 frames) and `make geo-fuzz GAME=...` (random lists through the
game's own upload: every command, odd floats, models in polygon RAM, ROM
garbage as models, the compact format and both matrices, the test
command, all four modes).

Results (*confirmed*, interpreted TGP; the recompiled TGP gives the same):

| Game | Program | Frames compared | Raster words | Differences |
|---|---|---|---|---|
| `topskatr` | TopSkater | 5,173 | 92.4M | none (stream, list words read, state) |
| `skisuprg` | Shared | 5,130 | 126.5M | none |
| `stcc` | Shared (`stcc`'s) | 5,173 | 82.5M | none |
| `segawski` | Shared | 5,035 | 115.5M | none |
| `waverunr` | Shared | 5,031 | 139.2M | none |
| `overrev` | Overrev | 5,041 | 87.0M | none |
| `hotd` | Shared | 5,093 | 78.1M | none |
| `bel` | Shared | 5,119 | 107.3M | none |
| fuzz, each of the four uploads | | 4,000 (8 seeds) each | 2.8-4.0M each | none |

`segawski`, `waverunr` and `hotd` draw with computed normals, the others
with given normals; `topskatr` both, and its skaters in the compact
format. Bit-exact everywhere, so there are no visual differences to
describe.

The fuzzer keeps lighting entries within the 32 and specular controls
below 15: past those the program squares thousands of times without
input or output, which the board model takes for a finished frame and
cuts short (`IDLE_STEPS` in `geo.rs`), something the hardware doesn't do;
the native code computes what the program does. The games stay far
inside both.

**State hashes.** 600M-instruction coin-start runs (with the EEPROM a
first run writes) give the same state hash on the TGP and natively:
`topskatr` `400275251269648e`, `skisuprg` `188a5a44fb547532`, `stcc`
`bbde4ebfb8d918d8`, `segawski` `505a972b9e8a3c89`, `waverunr`
`97d498bed77503a6`, `overrev` `d41686f041683f4c`, `hotd`
`0473295c9891af45`, `bel` `25f923c57d7362eb`. Runs of `topskatr`,
`overrev` and `segawski` saved at frame 2000 in either mode and resumed in
either mode end with the uninterrupted run's hash.

## Speed

Per-frame geometrizer time on the worker thread, native build (`make
geo-check`, which times both sides every frame; frames 3000 on are races):

| | TGP interpreted | TGP recompiled | native |
|---|---|---|---|
| `topskatr`, frames 3000-5173 | 11.95 ms (worst 16.0) | 8.17 ms (worst 10.7) | 0.45 ms (worst 1.3) |
| `stcc`, frames 3000-5173 | 5.87 ms (worst 13.7) | 3.93 ms (worst 6.4) | 0.19 ms (worst 0.5) |
| all eight, frames 3000 on | 4.6-12.4 ms | | 0.17-0.45 ms |

Native is about 20 times the recompiled TGP and 30 times the interpreted
one. The whole machine, interpreted i960, `topskatr` 600M-instruction
coin-start run: 2.11x real time on the TGP (18.3 s of geometrizer time,
6.2 s waited for), 2.58x natively (0.7 s, none waited for).
