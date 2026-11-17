# The 2B geometrizer program, and a native implementation of it

Status: 2026-11-09. What the display-list program that most Model 2B
games upload to their geometrizer SHARC does, worked out from its
behaviour and its disassembly (`make sharc-disasm
FILE=out/overrevb/sharc-geo.bin`), and how reModel implements it natively
(`crates/model2/src/geo_native.rs`, `--geo native`). The board side (the
SHARC's ports, the list DMA, the frame protocol) is in `model2b.md`; the
stream it produces and the rasteriser that draws it in `geometrizer.md`
§4-§6.

**Clean room.** Written from the program's behaviour and disassembly and
Analog Devices' manual ([UM], `adsp2106x.md`). No emulator's code was
read. The native code is Rust written from this description: it doesn't
follow the program instruction by instruction, holds no table from the ROM
or the program, and finds the program by hashing the code the game
uploads. Everything below marked *confirmed* was checked by running the
SHARC and the native implementation on the same input and comparing their
output word by word (`make geo-check`, `remodel geo-fuzz`, below).

## Which programs

The program is recognised by `adsp2106x::cpu::code_hash` of the words
static code discovery finds in the upload: the hash the recompiler names
the program's crate by (`docs/recompiler.md`), so `sharc_<hash>` in
`out/recomp` is the same program. Never by game.

| Code hash | Games | Native |
|---|---|---|
| `15393321f266b9cf` | `overrevb`, `sgt24h`, `dynabb97`, `doa`, `schamp` (2,987 words of code; the uploads differ only past `0x20C30`, and `schamp`'s upload is shorter) | `Program::Shared2B` |
| `1ee99ebc8a458237` | `indy500`, `fvipers`, `von` (an earlier version: see "The earlier version" below) | `Program::Indy500` |
| `9b6fb0e1100bf66e` | `vstriker` (earlier still, 2,910 words of code: see "The earliest version" below) | `Program::Vstriker` |
| `90822719cbee82cd` | `lastbrnx` (3,312 words of code: the shared program with three commands added, see "Last Bronx's program" below) | `Program::Lastbrnx`, except its fn 0x1B |

For any other program (none is known on 2B) `--geo native` runs the
SHARC. So does it for `lastbrnx`'s fn 0x1B, the one command not implemented
(below): the native geometrizer stops there like it does at fn 0x1D.

## Render-only

*Confirmed*: the geometrizer feeds nothing back to the i960 or the copro.

- **From the program.** Its only external writes are the rasteriser FIFO
  (`0x410000`), its own polygon RAM (`0x420000+`), the polygon memory
  address (`0x430000`) and the handshake strobe (`0x430002`, with FLAG1).
  It reads only the list FIFO and polygon memory. Its other outputs are
  FLAG1-3 and the message registers (MSGR0 the last command word, MSGR1
  and MSGR2 the NOPs sent after END this frame and last frame, MSGR3 a
  command that failed the check, MSGR6 frames cut short by the next
  frame's start), which the host could read through the SHARC's host
  interface at `0x840000`.
- **From the games.** No 2B game reads `0x840000-0x8403FF`, the
  geometrizer's host window: every game's I/O log (`out/GAME/io.txt` of a
  coin-start run) shows only the upload's writes there. Nothing the i960
  reads depends on the geometrizer: the list pointers (`0x802008`,
  `0x803008`) are the i960's own, the frame parity (`0x98000C`) is
  vblank's, the copro status (`0x980014`) only says both programs are
  uploaded. reModel's board model answers `0x840000` reads with 0 and
  wires FLAG1-3 and `0x430002` to nothing, and the games run.
- **From running it.** With `--geo native` the SHARC never runs a frame
  and the machine's state hash after 600M-instruction coin-start runs is
  the one the SHARC gives (below).

So the geometrizer is a pure function of (the frame's display list, the
polygon memory, its own state from earlier frames) to (the raster stream,
its new state). Its state is the transform, lights, lighting parameters
and so on that commands set, and polygon RAM.

## Arithmetic

The program sets MODE1 to RND32 | TRUNC | ALUSAT and never changes it, so:

- every float result is IEEE single, **rounded toward zero**; float inputs
  lose the register's low 8 bits; denormal inputs and results are zero;
  overflow gives the largest finite value (toward zero); NaN results are
  all ones. Results are exact roundings of the exact result, so `a + b` is
  `b + a` (the program's operand order there doesn't matter), but
  `(a + b) + c` is not `a + (b + c)`: the native code keeps the program's
  association everywhere;
- `FIX` (float to integer) under TRUNC rounds toward minus infinity in the
  core (`adsp2106x.md`), saturating (ALUSAT); a NaN gives all ones;
- integer adds saturate;
- `RSQRTS` gives a 4-bit seed, the core's (`float::rsqrts`, whose seed
  table is computed, *unconfirmed* against the chip's ROM); the program
  refines it with two Newton steps, `y ← (0.5·y)·(3 − (y·y)·x)`.

The native code calls the `adsp2106x::float` functions the interpreter
and the recompiled code use (`mul`, `add`, `sign_op`, `select`, `to_int`,
`rsqrts`), with the same mode, so every value is the SHARC's by
construction. Values are kept as the 32-bit patterns the program stores.

**Stream format.** Command words are `0xFF000000 | list word >> 23`;
floats go out as their IEEE bits shifted right 8 (sign, exponent, 15
fraction bits); see `geometrizer.md` §4.2.

## Frame protocol

As `model2b.md` describes: after the upload the program waits for two
frame handshakes, so the first frame after an upload reads no list and the
second reads one; then one list a frame, from the read pointer until END.
At each frame start it sends NOP and END (the previous frame's
terminator, which reModel's frame model discards) and resets the
fast-RAM attribute word below. The list DMA's cap (`MAX_LIST_WORDS`) ends a
list with END words, as on the SHARC.

**The command check.** If MSGR7 doesn't hold `0x7378842E` after the upload
(`fvipers` and `schamp` don't write it), each command word must have its
fn (bits 27-23) in bits 4-0 and 12-8 too. A word that fails stops the
program for good (it toggles FLAG2 forever): nothing more is drawn until
the next upload. Only command words are checked, not operands.

## Commands

Each command is a word (fn in bits 27-23, fn bit 5 and the eye select in
bits 31-28 go to the raster command) and its operands from the list. fn
0x11-0x1F are fn 0x01-0x0F again except 0x14 (log data), 0x16 (LOD), 0x1D
and 0x1E. "Out" is what goes to the rasteriser; `cmd` is the raster
command word.

| fn | Operands | Out | State |
|---|---|---|---|
| 0x00 NOP | — | `cmd` (which the board model filters) | — |
| 0x01 Object | tpa, tha, oba, obc | `cmd`, tpa & 0xFFFFFF, tha & 0xFFFFFF, the polygons (below) | — |
| 0x02 Direct | tpa, tha, P0, P1 (6), then polygons: attr, luma, distance, Pa (3) [, Pb (3) if attr bit 0] until an attr with bits 1-0 clear | `cmd` − 1, tpa, tha, the six vertex words >> 8, then per polygon attr & 0xFFFFFF and the other words >> 8 (the closing attr isn't sent; the first is never tested) | — |
| 0x03 Window | 6 words | `cmd`, each as `0x00XXXYYY` (bits 11-0 and 27-16 of the word) | — |
| 0x04 Raster memory | addr, n, n words | `cmd`, addr, n, the words | — |
| 0x05 Polygon RAM | addr, n, n words | — | addr with bit 24 set: the SHARC's own memory from `0x30100` ("fast" polygon RAM, `addr − 0x1000000` words in); else polygon RAM word `addr` (`0x420000 + addr` on its bus) |
| 0x06 Lighting | index, n, n × (bytes, coef) | — | entry `index + k`: diffuse = byte 0, ambient = byte 1, specular scale = byte 2 (as floats), specular control = byte 3 (an integer); distance coefficient = coef. 32 entries; the index isn't scaled |
| 0x07 Mode | 1 word | — | bit 0: specular; bit 1: normals computed from the vertices |
| 0x08 Z-sort | 1 float | `cmd`, the float >> 8 | — |
| 0x09 Focal | fx, fy | — | focal lengths |
| 0x0A Light | 3 floats | — | light vector L |
| 0x0B Matrix | 12 floats | — | rotation m0-m8, translation t |
| 0x0C Translation | 3 floats | — | t |
| 0x0D Data push | addr, n | n words read from the SHARC's address space from addr, raw | — |
| 0x0E Test | 32 words, n, n × (addr, count, sum) | — | the blocks are stored where fast polygon RAM starts; each block's polygon memory is summed (only FLAG outputs show the result); sets a "tested" flag that the next object clears |
| 0x0F END | — | — | the frame's end |
| 0x10 Dummy | 1 word | — | — |
| 0x14 Log data | addr, n, n words | `cmd`, addr, 4n, then each word's four bytes, low first | — |
| 0x16 LOD | 1 float | — | the distance scale |
| 0x1D, 0x1E | code upload and call | (not implemented natively: the native geometrizer stops) | — |

A count of 0 is 2^32 (the SHARC's loop counter), which only a broken list
would send; the native code stops on loops of more than 4M words.

**The test command's loop.** The 32 words should be a walking one
(`1 << i`). A word that isn't calls a routine from the second-last
instruction of its loop, where the SHARC tests the loop's end twice
([UM] §3.5.1.1, the reason `RTS (LR)` exists, which this program doesn't
use): the count goes wrong and the loop runs on through the list. The
games' tests always pass, and the native code stops if one doesn't.

## Object data

A model in polygon memory (ROM from `0x800000`, polygon RAM below; or fast
polygon RAM) is two vertices P0, P1, then polygons of 10 words each:
attr, a normal (3), Pa (3), Pb (3). A triangle has the Pb words too and
they are skipped. obc polygons are drawn (0: until a terminator), and an
attr with bits 1-0 clear ends the model before that. The stream gets the
two transformed vertices, then per polygon attr & 0x3FFFF, the luma word,
the distance, Pa and, for a quad (attr bit 0), Pb: the rasteriser links
them (`geometrizer.md` §3.4).

Per vertex, view space is `v' = ((t + x·m0) + y·m3) + z·m6` (and the same
for y', z' with m1, m4, m7 and m2, m5, m8): translation first. It goes out
as `fx·x'`, `fy·y'`, `z'`.

Per polygon, with n the polygon's view-space normal and L the light:

- `dotl = (n.x·L.x + n.y·L.y) + n.z·L.z`, `dotp` the same with Pa' for L;
- back face: `dotp < 0` (a negative zero counts as front) adds 0x100;
- the diffuse factor is `|dotl|`, or 0 when `dotl·dotp` is negative (the
  sign of the product: −0 counts), computed as the product minus itself
  (NaN for an infinite product);
- `base = factor·diffuse + ambient` of the polygon's lighting entry (attr
  bits 22-18);
- with specular: `s = max((dotl + dotl)·n.z − L.z, +0)`, set to `s − s` if
  the control is 0, squared once if control ≥ 2, again if ≥ 4, again if
  control + 1 ≥ 8; then `luma = base + s·scale`;
- the luma is `FIX`ed, clipped to ±255, the back-face bit added, and sent
  shifted to bits 15-23 (as `value << 15` in the given-normal modes,
  `(value << 23) >> 8` in the computed-normal ones; they differ only for
  negative values, which a NaN gives);
- `distance = (coef·|dotp|)·LOD`, coef from attr bits 31-27.

**Given normals** (mode bit 1 clear): n is the model's normal rotated:
`((x·m0 + y·m3) + z·m6)` and so on.

**Computed normals** (mode bit 1 set): the program tracks the previous
edge (P0', P1') as the rasteriser will link it: P0' and P1' start as the
model's two vertices, and after each polygon the link type (attr bits 9-8)
sets the next edge: 0 or 2 (Pa, Pb), 1 (P0' kept, Pa), 3 (Pb, P1' kept),
with Pb = Pa for a triangle. With `a = Pa' − P0'`, `b = P1' − P0'`, the
normal is `b × a` normalised: `cx = a.z·b.y − a.y·b.z`, `cy = a.x·b.z −
a.z·b.x`, `cz = a.y·b.x − a.x·b.y`, `len² = cx² + (cz² + cy²)`, then
`RSQRTS` and two Newton steps as above. `dotp` uses `Pa'.x + 0` and
`Pa'.z + 0` (turning a −0 into +0). The program keeps the three edge
vertices in a ring of three slots and the first in program memory; the
native code keeps them the same way, because of the fast-RAM quirk below.

**The address latch.** Polygon memory is read through an address register
(`0x430000`) that the program writes before each word. It reads a
polygon's first words while it finishes the one before, so after a model
from polygon memory the register holds the address 4 (given normals) or 6
(computed normals) words into the polygon after the last one drawn. That
matters only to a later data push (fn 0x0D) of the data port, which the
native code also models.

**Fast polygon RAM** (oba bit 24: the model is in the SHARC's own memory,
written by fn 0x05) runs separate code, which no game's runs reach (none
writes fast RAM). It computes the same values, but *confirmed* by the
fuzzer it has two faults the native code reproduces:

- an object that ends by its count (not a terminator) sends its last word
  (the last vertex's z) twice: the code sends it from the loop's last
  instruction and again after the loop;
- in the computed-normal modes a triangle with link type 0 or 2 moves only
  P0'.x (the other two moves sit past a jump that isn't delayed), and
  leaves the ring and the edge pointer one word on: the following normals
  come from misaligned vertices.

It also keeps the last attribute word it read in a word of its own (reset
each frame), which nothing reads.

## The earlier version (`indy500`, `fvipers`)

`indy500`'s upload, which `fvipers` also sends (identical up to the end of
the code), is the same program with three differences (*confirmed*,
comparing the two disassemblies with addresses normalised, 3,256 lines):

- in the computed-normal modes the first vertex's y goes out scaled by
  **fx** instead of fy (four places, one per computed-normal path), a bug
  the later version fixes;
- at a frame start it doesn't send the NOP before the END (which the board
  model discards anyway);
- its test command doesn't rewrite the last stored word (no effect).

`Program::Indy500` is the shared implementation with the first
difference.

## The earliest version (`vstriker`)

`vstriker`'s upload (3,040 words, 2,910 of them code) is `indy500`'s
program before two changes (*confirmed*, comparing the two disassemblies
with addresses normalised; 2,896 of its words appear in order in
`indy500`'s):

- `indy500`'s writes the last of a run of stores to internal memory three
  times over (the vertex ring and the edge vertex after each polygon, the
  transform and light words its commands set, the MSGR1 and MSGR6
  counts, the "tested" flag); `vstriker`'s writes each once. They are
  stores of the same value to the same internal address, so they change
  nothing the program computes; a branch's delay slots move with them. Presumably a
  workaround for something on the board or the chip (*unconfirmed*: what).
- its program-memory data words (the matrix's first row, translation and
  light copies, the computed-normal modes' first edge vertex) start at
  `0x20C00`, just past its shorter code, rather than `0x21000`. (Its code
  upload command, fn 0x1D, writes to `0x20C00` too, over those words; no
  game sends it, and the native code doesn't implement it.)
- two words of the text strings embedded in the code area differ.

Everything else, including the first vertex's y scaled by fx in the
computed-normal modes, the frame start without the NOP, the test command
and the dispatch table, is `indy500`'s. `Program::Vstriker` is
`Program::Indy500` with its data words at `0x20C00`
(`Program::pm_base`, which `Native::import`/`export` and `geo-check`'s
state comparison use).

## The native implementation

`model2::geo_native::Native` holds the program's state in the SHARC's
data memory layout (block 1 from `0x30000`, 32K words) and the few
program-memory words it uses as data (`0x21000` up): the dispatch table
the start-up code writes, translation, matrix, focal lengths, mode, LOD,
light, the vertex ring, the lighting entries and coefficients, fast
polygon RAM. So a SHARC's state can be taken over between frames
(`Native::import`, after a save state made with the SHARC is loaded, or
when `--geo` switches mid-run) and handed back (`Native::export`: a save
state made in native mode stores the SHARC with the native state written
into its memory, in the format the SHARC path saves, so it loads in
either mode; loaded on the SHARC it repeats the two boot handshakes).

`GeoMode` (`geo.rs`) picks what runs a frame: `Chip` (the SHARC; the
`remodel` CLI's default: nothing changes), `Native` (the native code when
it implements the uploaded program, else the SHARC; the player's and the
browser's default), `Check` (the SHARC's output, with the
native one computed beside it from the same buffer RAM copy, its own
polygon RAM and address latch, and compared word by word; after a
difference the native side takes the SHARC's state again, so every frame
is compared from the same start). The engine runs on its worker thread as
before, or in the browser's geometrizer worker.

Front ends: `remodel --geo chip|native|check` (`sharc` names the chip
too), `remodel-play --geo chip`, and the browser's `?geo=chip` for the
SHARC (the geometrizer worker runs the native code by default;
docs/web.md). The 2C counterpart is `geometrizer-2c.md`.

**Smooth motion** (`geometrizer.md`, "Smooth motion"): the matrix (fn
0x0B and 0x1B), translation (0x0C, 0x1C) and focal-length (0x09, 0x19)
handlers pass their operands through `Io::xform` before using them, and
objects their model address through `Io::object`. With no tap (every
real frame unless smooth motion is on, and then it only reads them) that
changes nothing: the state hashes above are the same with in-betweens
drawn every frame. In-betweens rerun a frame's list on a copy of this
state with the transforms replaced.

## Verification

The values the program computes with are IEEE singles in its 32-bit
registers (*confirmed* by smooth shading's self-check, `geometrizer.md`,
"Lighting side channel": each polygon's luma recomputed in IEEE single
precision from the normal, light and lighting entry the native code used
is the program's for 99.996-100% of polygons in all nine 2B games, the
rest one level off by the rounding toward zero).

`make geo-check GAME=...` runs a coin-start run (`geo-check STEPS FROM`:
750M instructions, past 5,000 frames, by default) in `Check` mode and
reports the frames compared, the first difference with the words around
it, and the program's state words after each frame (the transform, the
ring, lighting entries; `0x30020-0x300FF` and the edge vertex). `remodel
geo-fuzz FRAMES SEED` drives the game's own uploaded program with random
display lists: every command, random and odd floats (zeros, denormals,
infinities, NaNs), models in polygon RAM and fast polygon RAM, ROM models
at random addresses, direct data, data pushes, tests, all four modes.

Results (*confirmed*, interpreted SHARC):

| Game | Frames compared | Raster words | Differences |
|---|---|---|---|
| `overrevb` | 5,040 | 87.0M | none (stream, list words read, state words, latch) |
| `sgt24h` | 5,127 | 128.6M | none |
| `dynabb97` | 5,155 | 77.5M | none |
| `doa` | 5,113 | 118.6M | none |
| `schamp` | 5,160 | 137.7M | none |
| `indy500` | 5,037 | 100.9M | none |
| `fvipers` | 5,169 | 142.5M | none |
| `vstriker` | 5,177 | 172.7M | none (interpreted and recompiled SHARC) |
| `lastbrnx` | 4,101 | 108.7M | none (its fn 0x1B not implemented, never sent; `docs/games/lastbrnx.md`) |
| fuzz, `overrevb`'s upload | 8,500 (19 seeds) | 8.2M | none |
| fuzz, `sgt24h`'s, `doa`'s and `dynabb97`'s uploads | 1,000 (3 seeds) | 0.9M | none |
| fuzz, `schamp`'s upload (command check on) | 1,400 (3 seeds) | 1.1M | none |
| fuzz, `indy500`'s and `fvipers`' uploads | 1,700 (4 seeds) | 1.7M | none |
| fuzz, `vstriker`'s upload (command check on) | 5,300 (11 seeds; 3 also on the recompiled SHARC) | 4.4M | none |

The coin-start runs go through attract, menus and into races or fights
(`geo-check`'s default 750M instructions, 5,177 frames). The fuzzer found
the fast-RAM faults and the latch behaviour above, and the start-up
code's dispatch table in data memory (a data push can read it), before
the native code reproduced them.

Bit-exact everywhere, so there are no visual differences to describe.

**State hashes.** With `--geo native` the SHARC runs no frame, so its
registers and memory in a save state differ (they are the start-up's,
with the native state written in on save); "identical" therefore means the
machine state hash (all RAM, the last raster stream, the copro results:
not the geometrizer SHARC), and the raster stream of every frame. Both
are the same as the SHARC's: coin-start runs of 600M instructions gave
(before the timer read fix of 2026-11-13, which changes the hashes of the
games that read the timers, `docs/hw/peripherals.md`)
the same state hash on the SHARC and natively for `sgt24h`
(`282c8d6fd5151c21`), `doa` (`78f308b4da95cefb`), `schamp`
(`0dd9ca5f3388771f`), `vstriker` (`a4af0c62acc656f3`, interpreted and
recompiled), `lastbrnx` (`c3ec82727bacd608`, interpreted and recompiled),
`overrevb` and `dynabb97` (with the `eeprom.bin` of
an earlier run), and a run saved at frame 2000 in either mode and resumed
in either mode ends with the uninterrupted run's hash (for `vstriker`
too, recompiled: saved natively and resumed on the SHARC, and the
reverse).

## Last Bronx's program

*Confirmed* from its disassembly, then by `make geo-check GAME=lastbrnx`
and the fuzzer. `lastbrnx` (3,442 words uploaded, 3,312 reached as code)
uploads the shared program (`15393321f266b9cf`) with 325 words added.
Aligned with `schamp`'s upload word by word, everything is the same except:

- the start-up code writes eight more words of data memory, after the 32
  command entries: `0x30100` three times, `0x80C000` four times and 0 to
  `0x30050-0x30057` (nine more words of code with the `I0 = 0x30050`
  that points at them), which shifts every address after it by 9: the 133
  other one-word differences are all jump distances and absolute targets
  that moved (compared with the distances normalised, nothing else
  differs);
- three entries of the command table (the other 29 are as in the shared
  program): fn 0x1A, 0x1B and 0x1C, which the shared program aliases to
  0x0A-0x0C (light, matrix, translation), are routines of its own, and the
  code past the shared program's end (`0x20C35` on) is theirs. The code
  upload (0x1D) and code call (0x1E) are the shared program's, in code
  that now sits where its scratch area was (`0x20C00`), so fn 0x1D would
  overwrite the new commands; the game doesn't send it.

What they do: a *ring of 16-bit texture points*, in data memory from
`0x30100` (the "fast" polygon RAM's place) with its state in
`0x30050-0x30057`: DM[0x50] the ring's base, DM[0x51] the read pointer
(a word address), DM[0x52] the write pointer, DM[0x53-0x56] the start and
end of what is queued in texture point RAM terms (their address, from
`0x80C000`: bit 23 set is point RAM, `geometrizer.md` §2.1), DM[0x57] a
recording flag.

| fn | Operands | What it does (*confirmed* by the SHARC and the native code agreeing) |
|---|---|---|
| 0x1A | flag | DM[0x57] = flag. With flag set and the old flag clear: DM[0x56] = DM[0x55] (end = start); with flag 0: read pointer = base, DM[0x54] = DM[0x53]. Nothing is sent |
| 0x1C | an address | Sends what the ring holds as one memory write for the rasteriser: command 4 (`0xFF000004`), DM[0x54]'s old value, a count n = (DM[0x56] - DM[0x54]) rounded down to even, then n 16-bit values read as short words from `2 * DM[0x51]` (the low half of each 32-bit ring word first); then the ring restarts: DM[0x53-0x56] = the operand, read and write pointers = base, flag 0. With nothing queued (n = 0) it only restarts the ring. The operand the game sends is `0x80C000`, once at the end of every frame's list |
| 0x1B | `R15, R0, R1, R13` and polygon memory | Not implemented natively. It transforms a polygon's vertices through the current matrix into 16-bit (x, y) texture points (fixed point, 9 fraction bits) which it appends to the ring (from DM[0x52]), and then continues into the object code with a polygon block of raster command 1 for them. Its code is the largest of the three (about 170 words). No run of the game sends it |

The rasteriser's cmd 4 write (`apply_raster_writes`) already puts those
values in texture point RAM. *Unconfirmed* what the points are for: the
operand `0x80C000` and 0x1B's transform suggest dynamic texture
coordinates (an effect or a trail), which the 0x1B routine would produce;
the i960 doesn't send any in the runs seen, so the ring is empty and 0x1A
and 0x1C change state and emit nothing.

**Check.** `make geo-check GAME=lastbrnx` over a 4,101-frame coin-start run
(108.7M raster words): 0 frames differ, 0 read the list differently, 0 end
with other state; the SHARC interpreted took 21.4 ms a frame (worst 37.8),
native 1.20 ms (worst 4.5), 17.8x. `geo-fuzz` (its lists use 0x0A-0x0C for
the matrix and leave 0x1B out for this program, and add random 0x1A and
0x1C) over 4,300 frames of 5 seeds: no difference. The emitting branch of
0x1C needs a ring only 0x1B fills, so only a unit test covers it
(`ring_flag_and_flush`).

## Speed

Per-frame geometrizer time on the worker thread, native build (`make
geo-check`, which times both sides on every frame; a desktop CPU shared
with other builds, so worst cases are noisy):

| | SHARC interpreted | SHARC recompiled | native |
|---|---|---|---|
| `sgt24h` race (frames 3000-4141, from a save) | 22.6 ms (worst 26.7) | 5.70 ms (worst 8.2) | 1.49 ms (worst 2.7) |
| `doa` fights (frames 3000-4486, from a save) | 35.1 ms (worst 88.6) | 7.32 ms (worst 11.5) | 1.83 ms (worst 3.0) |
| all seven games, frames 3000-5177 of a coin-start run | 17.6-37.2 ms | | 0.97-2.29 ms |
| `vstriker`, frames 3000-5177 of a coin-start run (a match) | 27.7 ms (worst 86.5) | 6.78 ms (worst 12.8) | 1.49-1.53 ms (worst 3.4) |
| `lastbrnx`, all 4,101 frames of a coin-start run (attract and a fight) | 21.4 ms (worst 37.8) | 5.53 ms (worst 10.1) | 1.20 ms (worst 4.5); 1.19 (worst 2.2) beside the recompiled SHARC |

(The native column is from the recompiled-SHARC runs; with the
interpreted SHARC beside it, it measured 1.57 and 2.24 ms. Native is about
4x the recompiled SHARC and 15x the interpreted one; it calls the core's
exact float functions for every operation, so there is room left.)

The whole machine, interpreted i960, 600M-instruction coin-start runs:
`sgt24h` runs at 0.80x real time on the SHARC and 2.59x natively, `doa`
0.81x and 2.03x, `schamp` 0.63x and 1.57x, `vstriker` 0.69x and 1.55x:
the 2B games reach full speed without recompiled code. Recompiled
(i960 and copro), `vstriker` runs at 2.73x on the recompiled SHARC and
3.87x natively (the geometrizer thread busy 24.1 s and 5.6 s of the run). Not measured in WebAssembly yet (the browser
build is a heavy one); that it keeps its lead there is *unconfirmed*.

## Next steps (scoping)

**Model 2B: done, with one command missing.** The four native programs
cover every 2B game reModel runs, all but `lastbrnx`'s fn 0x1B (above), which
no run sends. Another 2B game with a new geometrizer program would run on
the SHARC; `make geo-check GAME=...` says so ("no native implementation").

**Model 2C: done.** The four TGP programs the 2C games upload are
implemented natively the same way (`geometrizer-2c.md`): bit-exact over
coin-start runs of all eight 2C games, about twenty times the recompiled
TGP.

**GPU compute (WebGPU too), sketched.** The program's work splits cleanly:

- *Sequential, cheap, on the CPU*: walking the list (a few thousand words a
  frame) and applying the state commands, into an object table: for each
  object its transform, light, focal lengths, LOD, mode and a snapshot
  index into the lighting entries, its model address and count, and where
  its output goes. The models in polygon ROM never change, so per model
  (cached by address) the CPU knows once and for all each polygon's kind
  (quad or triangle), where the model ends and, for the computed-normal
  modes, which model vertices form each polygon's previous edge: the link
  types are in the ROM too, so the edge tracking reduces to vertex indices.
  Prefix sums over the objects' polygon kinds give every polygon's offset
  in the output.
- *Parallel, on the GPU*: one invocation per polygon (and per object for
  P0, P1): transform its vertices, rotate or compute its normal, light it,
  write its words (or vertex-buffer entries) at its offset. Nothing
  depends on another polygon's results once the edge indices are known.
  Fast polygon RAM models (and their two faults) stay on the CPU path.
- *Output*: either the raster stream (for the existing renderers, which
  parse it on the CPU), or better, the GPU renderer's own polygon buffers
  directly, with the z-sort keys computed in the same pass and sorted on
  the GPU, so nothing comes back to the CPU (a readback costs a frame of
  latency in the browser).
- *Exactness*: WGSL floats round to nearest (or as the GPU likes: no
  rounding-mode control, possible fused multiply-adds, denormals
  implementation-defined), so bit-exact output needs the SHARC's
  round-toward-zero done in integer arithmetic: a 24×24-bit product from
  16-bit halves in `u32`, alignment and truncation for adds, the `RSQRTS`
  seed as a 128-entry table computed on the CPU (WebGPU has no `f64` and
  no 64-bit integers). That is maybe ten times the cost of plain `f32`,
  still a small fraction of a frame for tens of thousands of polygons. A
  plain-`f32` (or full-precision) variant is the "enhanced" mode, checked
  against the exact one for size of difference rather than equality.
- *WebGPU limits*: 128 MB storage buffer bindings by default (polygon ROMs
  are 4-32 MB), 65,535 workgroups a dimension (dispatch in 2D for big
  frames), no `f64`; the browser's geometrizer worker can own the device
  queue if the renderer moves there too.

What it enables: per-object matrices are explicit in the object table, so
interpolated frames (now done on the CPU: `geometrizer.md`, "Smooth
motion"; a GPU pass would make each in-between nearly free), run-ahead and rollback (the geometrizer state is a few kilobytes,
saved and restored with the machine), full-precision geometry, and MSAA in
the renderer, without the SHARC.
