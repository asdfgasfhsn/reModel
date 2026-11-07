# Static recompilation

Remodel can turn the game's own code into Rust and compile it into the
binary: the i960 main program, the MB86235 programs uploaded to the 2C
copro and geometrizer, and the ADSP-21062 (SHARC) programs uploaded to
Model 2B's. Nothing is reimplemented; the original instructions run, just
translated ahead of time instead of decoded at run time.

```sh
make run STEPS=900000000 INPUT=coin1@1100-1104,start@1250-1254   # trace what runs
make recomp               # generate out/recomp/<program>/ and build target/recomp
make bench RECOMP=1       # same run on the recompiled build: same state hash
make lockstep RECOMP=1    # recompiled beside interpreted, compared as they go
target/recomp/release/remodel --interp ...   # recompiled build, interpreter for the i960
```

The generated code is derived from the ROMs, so it lives in `out/recomp/`
and is never committed. Ordinary builds (and `make check`) leave it out. A
build with no recompiled code for a ROM image runs it on the interpreters,
so the same binary still runs other games and revisions.

## Layout and registration

Every recompiled program is a crate of its own, named by its hash:
`out/recomp/i960_<hash>/` for a game's i960 program, `out/recomp/tgp_<hash>/`
for a 2C copro or geometrizer program, `out/recomp/sharc_<hash>/` for a 2B
one. Games share them where the program is the same. A SHARC crate is
named after the words it compiles (`adsp2106x::cpu::code_hash` of their
addresses and contents), not the whole upload: `overrevb`, `sgt24h`,
`dynabb97` and `doa` upload the same geometrizer code with different words
past it, and the Jaleco games the same copro code, so the four games have
four SHARC crates between them rather than eight (one geometrizer crate,
and copro crates for the Jaleco games, `dynabb97` and `doa`). `indy500`'s
geometrizer program is an earlier version of that code and its copro
program its own, so it shares neither and adds two crates. `fvipers`
uploads `indy500`'s geometrizer code and `schamp` the shared one, so each
adds only its copro crate.
Each holds `program.rs` from the code generator, generic over the machine around the
processor, and a `lib.rs` that instantiates it for the Model 2 board
(`Model2Bus`, the TGP's external buses), so its machine code is compiled in
that crate and nowhere else. Program crates depend on `i960`, `mb86235`, `adsp2106x` and
`remodel-model2`; model2 depends on none of them. A SHARC crate's
`roles.txt` says which processor(s) the program was captured on (`copro`,
`geo`), and its glue instantiates it for those buses only.

model2 takes the programs at run time instead: `model2::recomp::register`
installs a table of (hash, i960 blocks), (hash, copro and geometrizer
`step`) and (hash, image, compiled words, copro and geometrizer `run`), and
the machine looks programs up there: i960 and TGP programs by hash, SHARC
programs by their compiled words being in the SHARC's memory. The front ends
(`remodel`, `remodel-play`, `remodel-web`) call `recomp::register()` first
thing. In the checkout's workspace `recomp` is `crates/recomp`, a stub that
registers nothing, so ordinary builds, `make check` and `make dist` never
see `out/`.

`remodel recomp` also writes a Cargo workspace in `out/recomp/`:
`registry/` is a crate called `remodel-recomp` that depends on every
program crate there and registers them all, and the workspace's `[patch]`
puts it in place of the stub; `front/` only brings `remodel` and
`remodel-play` into the workspace (under a `cfg` that never holds) so that
`cargo build -p` can build them. It has its own `Cargo.lock`, copied from
the checkout's, so the checkout's never changes. `make ... RECOMP=1`
builds from it (`--manifest-path out/recomp/Cargo.toml`) into
`target/recomp`. `make recomp-workspace` rewrites it and every program
crate's manifest without recompiling anything (after updating Remodel or
moving the checkout; it also moves loose `i960_<hash>.rs` / `tgp_<hash>.rs`
files from the older layout into crates). Files are only written when
their contents change, so Cargo rebuilds only programs that did.

## Memory and parallelism

Measured here (16 cores, 31 GB, seven 2C games; their program crates hold
4-24 MB of generated Rust each):

| build | before (one model2 crate with every game) | program crates |
|---|---|---|
| clean `target/recomp`, `remodel` + `remodel-play` | available memory 14 GB to 85 MB | 12.3 GB to 7.9 GB (largest rustc 4.8 GB, `hotd`); 10 min |
| one game's programs regenerated | everything rebuilt | 12.0 GB to 10.2 GB; 87 s (`topskatr`) |

What keeps it bounded:

- **One program crate at a time.** `make release RECOMP=1` (and so `play`,
  `bench`, `lockstep`, ...) runs one `cargo build -p` per program crate in
  `out/recomp/`, then builds the front end. Each is compiled with every
  core, but only one is in memory at once, so the peak is the largest
  program's (about 5 GB for `hotd`'s 24 MB i960 module), however many games
  there are. Cargo's `-j` would not do: rustc's codegen threads share its
  job tokens, so `-j1` also makes each crate's LLVM work serial. A crate
  whose sources didn't change is not rebuilt; the registry and the front
  ends relink when any program changes (a minute or so).
- **Submodules.** rustc never splits one module across codegen units, so
  the i960 code generator puts every 512 blocks in a submodule of their
  own. With the whole program in one module, `hotd`'s crate compiled on one
  thread in 217 s; in submodules it takes about 60 s on 16, with the same
  peak memory. An MB86235 program is still one `match` in one function and
  compiles on one thread (up to 100 s for `stcc`'s geometrizer program);
  splitting it would add a call per TGP instruction.
- **No debug info** for the recompiled build, as before.
- **SHARC programs** go 256 words to a submodule, each with the region
  functions that start in it. The geometrizer crate (2,987 words, 3.7 MB
  of Rust with its blocks) builds in about 150 s (56 s as single steps
  only, with available memory going from 18.6 GB to 8.6 GB on 16 codegen
  units); the copro crate (2,111 words) in about 60 s (40 s). Both are
  compiled for the bus they run on only; for both buses they took twice as
  long.

Splitting into crates costs nothing at run time: blocks are
`#[inline(never)]` anyway, and model2 marks what they call on every
instruction (`Model2Bus`'s `Bus` and `Host` methods, memory lookup, the
check in `run_tgp`, the TGP buses' `External` methods) `#[inline]`, so it
still inlines into each program crate. That also made the
recompiled builds 0-16% faster than the single crate (`make bench
RECOMP=1`) and the interpreters about as fast as before. More codegen
units made no difference: 64 against the default 16 took the same time
and memory on `hotd`'s crate (as one module, before the submodules).

## The oracle

The interpreters are the reference. `make bench` runs a fixed scripted
session into a race and prints a state hash over all RAM, the last
rasteriser stream and every value the i960 read from the copro. A
recompiled build must print the same hash. This is a strong check: one
wrong flag, one instruction retired at a different time, and the hash
differs within a few frames.

## Beyond recompiling: the native geometrizer

Recompiling keeps every instruction of a program. For the geometrizer
there is also a native implementation of the programs' behaviour, 2B
(`model2::geo_native`, `docs/hw/geometrizer-2b.md`) and 2C
(`model2::geo_native_2c`, `docs/hw/geometrizer-2c.md`): the default in
the player and the browser, `--geo native` for the `remodel` CLI, whose
default stays the chip (`--geo chip`, `remodel-play --geo chip`,
`?geo=chip` in the browser). It finds a program by the same hash this
page's crates are named by (`sharc_15393321f266b9cf` and
`sharc_1ee99ebc8a458237` on 2B; `tgp_29177553590496aa`,
`tgp_b850fb3484005f7b`, `tgp_a49bbffc5f8b1d2a` and
`tgp_c655943137753c1d` on 2C), so it needs no recompiled build, and gives
the same raster stream word for word: `make geo-check` runs the chip
(recompiled with `RECOMP=1`) and the native code side by side each frame
and compares them, as `make lockstep` does the recompiled code with the
interpreter. The state hash is the chip's. A 2C race frame takes 0.2-0.5
ms natively, about a twentieth of the recompiled TGP's time.

## i960 (`crates/i960/src/codegen.rs`)

**Which code.** All program code is in ROM, so it can't change at run time.
`remodel recomp` takes the static code discovery (`analyze`: recursive
descent from the boot record, interrupt table and system procedures) and
adds every address the interpreter executed in `make run` (`coverage.txt`),
including the targets of indirect calls and jumps (`indirect.txt`). Code
found only by guessing that an `lda` constant is a code pointer is left out
unless it ran, and so is anything at an address that isn't a multiple of 4
(the i960's instructions are word aligned; `schamp`'s discovery decoded
data at `0x20E`, and the block made there took the table slot of the one
at `0x20C`, so the recompiled build faulted at boot).

**Blocks.** Code is cut into basic blocks at branch and call targets, after
each control transfer, at known entry points, and wherever an instruction
isn't reached by falling through. Each block becomes one Rust function
(512 to a submodule, so the crate compiles in parallel), and the machine
keeps a table of them indexed by address. Execution enters at
any block start. Anywhere else, such as the point an interrupt returns to,
runs on the interpreter until it reaches one.

**Instructions.** Each instruction is either translated into plain Rust on
the register file (`c.r[17] = c.r[16].wrapping_add(0x4);`) or, for the rare
and floating-point ones, a call to the interpreter's `Cpu::exec` with the
decoded instruction as a constant. In `topskatr`, 96% of the 53,000
instructions are translated. Compares, condition tests, `call` and `ret`
use the interpreter's own helpers, so frame and flag behaviour can't
drift; that includes the on-chip local-register cache (`LocalCache`: the
KB's three cached caller frames, spilled on overflow and by `flushreg`),
which lives in `Cpu` and so is the same state for both.

Not every instruction that "falls through" really does. A `synmovq` to the
IAC port can reinitialise the processor at a new IP, and `modpc` can let an
interrupt in. So after any instruction left to the interpreter, the block
returns unless the IP is the next address. `topskatr` reinitialises this
way during boot.

**Timing.** The Model 2 board model runs the other processors, timers,
vblank and interrupt lines between i960 instructions (`Host::retire`).
Recompiled blocks call it after every instruction, exactly as the
interpreter's loop does, and return to the dispatcher as soon as an
interrupt is due, a frame ends or the instruction budget runs out. A
recompiled run therefore retires the same instructions at the same
emulated times as an interpreted one, which is why the state hash
matches.

**Checking.** `make lockstep RECOMP=1` boots two machines, one recompiled
and one interpreted, runs the first a block at a time and the second the
same number of instructions, and stops at the first block after which
registers, flags or cached register frames differ, printing the block and
the registers involved.

## MB86235 (`crates/mb86235/src/codegen.rs`)

Each program word the interpreter executed becomes a match arm calling
`Tgp::exec` with that word's decoding as a constant. `exec` is
`#[inline(always)]`, so the compiler specialises each arm. Words that
never ran fall back to the interpreter. The programs are uploaded by the
game at boot, so they are keyed by a hash of the uploaded image.

## ADSP-21062 (`crates/adsp2106x/src/codegen.rs`)

**Which code.** The programs are uploaded by the i960 at boot (`make run`
writes them to `out/GAME/sharc-copro.bin`, `sharc-geo.bin`). `remodel
recomp` compiles the words that
static discovery (`codegen::discover`, also behind `sharc-disasm`) reaches
from the reset and interrupt vectors, following jumps, calls, loops, delay
slots and the command dispatch tables (both programs store their handlers'
addresses as immediates at init and jump through them: discovery takes
immediates that point into the image as code), plus anything `make run`
saw execute (`sharc-*-coverage.txt`). For `overrevb` that is 2,111 of the
copro's 4,096 words and 2,987 of the geometrizer's; nothing executed in a
coin-start run is missing from the static set. The crate is named after
those words and holds only them (`IMAGE` is zero elsewhere), and a SHARC
attaches the program whose compiled words are all in its memory as
compiled (with that role, the one with the most words should several
match). Nothing else of the image is compiled in: data the program reads,
from wherever, is read from memory as it runs, so another upload of the
same code with other data runs the same crate correctly. A crate from
before (named after the whole image) is deleted when `recomp` writes its
replacement.

**Instructions.** Each compiled word is a function that runs one step of
the interpreter for that instruction, with its decoding as a constant
(`Sharc::run_compiled`). Everything it calls (execution, compute, ALU,
multiplier, shifter, conditions, DAGs, universal registers, memory) is
`#[inline(always)]`, so the compiler specialises each word: decode and
operand dispatch fold away, and so do the fetch stage's branch and loop
set-up paths for instructions that can't take them. What remains is the
interpreter's own cycle in the same order: the timer and latches tick, the
loop test of the fetch sees the status from before the instruction, the
two-slot pipeline moves on, delayed branches, loop ends tested at the last
word's fetch, PC/loop/status stacks, interrupts and their hold-off after
branches. The program's `step` is a `match` on the address in the execute
slot (after running any bubbles, as the interpreter's `step` does).

**Blocks.** That per-cycle bookkeeping (tick, fetch stage, interrupt
check, counters, the bus's `after_step`) cost more than most instructions'
own work. So runs of consecutive words that can be in one
(`codegen::in_block`: no DO, stack operation, IDLE, unconditional branch,
or sequencer or system register operand) also become a region function of
up to 128 words, entered at any of them through a `match` that breaks out
of nested labelled blocks, so execution falls through from word to word.
It runs them as one block (`Sharc::block_start`/`block_exec`/`block_end`)
and brings the pipeline, loop logic, counters and latches up to date at
the end, as single steps would have left them; `docs/hw/adsp2106x.md`
says why that is exact and when a block can't run (interrupts on, the
fetch reaching a loop end, a taken branch, IOP registers, overwritten
code). Then the word runs as its single step. The board says how far a
block may go before it must look (`Bus::block_budget`: the geometrizer's
check every 256 instructions or its FLAG0 pulse, the copro's step limit)
and replays its per-instruction work after each external access
(`Bus::after_access`), so its stopping rules (the copro waiting on a FLAG,
the geometrizer's end of frame) are kept exact. `run` loops over blocks
and single steps until `after_step` says stop.

The geometrizer program is float-heavy: with RND32 (both programs set it)
an add or multiply of two normal inputs is computed exactly in f64 and
rounded to 23 fraction bits in place (`float::single`), which the
interpreter uses too; anything else (zeros, specials, results that over-
or underflow, adds of very different magnitudes) takes the exact
significand path. A unit test checks the two agree bit for bit, flags
included.

**Fallback.** A word without code (not found, or the program's own code
loaded later) runs on the interpreter: `Sharc::step_uncompiled` counts it
per address, and `remodel run` reports any (`recompiled code missed N
instructions`); `overrevb` misses none. **Self-modifying code:** neither
program writes its code (95 geometrizer words store through the PM bus,
all to data), but that can't be proven statically, so the core watches
every compiled word: when attaching, any that doesn't hold what was
compiled, and later any written, runs on the interpreter from then on
(`Sharc::watch_code`; a unit test overwrites a compiled word). After a save
state load the program is found again by its compiled words being in
memory.

**Spinning.** `Sharc::spinning` (the board stops stepping a SHARC that
only waits on a FLAG input) decodes from memory when the decode cache,
which recompiled code doesn't fill, has no entry; it gives the same answer
either way.

**Checking.** `make lockstep GAME=overrevb RECOMP=1 INPUT=coin-start` runs
both SHARCs recompiled in one machine and interpreted in the other (beside
the i960 comparison): the copro's steps, PC and registers after every i960
block, and both SHARCs' whole state (memory, registers, stacks, pipeline,
counters) and the geometrizer's output after every frame. 600M i960
instructions (4,141 frames, into a race) are identical. The coin-start
state hash is the interpreter's (`ee2bcd89e419e428`).

**Speed** (`overrevb`, from a save state at frame 3000 of a coin-start
run, 658 race frames, native; WebAssembly in Node, 390 frames, as
`docs/web.md` times it):

| geometrizer per frame | native | WebAssembly |
|---|---|---|
| interpreter, before | 20.8 ms (spikes 25) | 33.1 ms |
| interpreter, now (inlining, f64 fast path) | 16.8 ms | |
| recompiled, single steps | 9.3 ms (slowest 12.4) | 10.7 ms (worst 12.5) |

With blocks, from a save state at frame 2000 of a coin-start run to the
600M-instruction mark (frame 4141 natively, single-threaded; 2,070 frames
in Node 26), mean per frame with the slowest (native) or the 95th
percentile (WebAssembly):

| geometrizer per frame | native, single steps | native, blocks | WebAssembly, single steps | WebAssembly, blocks |
|---|---|---|---|---|
| `overrevb` | 8.9 ms (slowest 12.5) | 3.6 ms (5.0) | 10.2 ms (p95 11.7) | 4.8 ms (5.5) |
| `sgt24h` | 14.6 ms (24.4) | 6.1 ms (10.3) | 16.5 ms (p95 24.2; 35% of frames over 17.4) | 8.1 ms (11.8; none) |
| `sgt24h`, race start (frames 2000-2899) | 16.4 ms (24.5) | 6.8 ms (9.1) | | 9.0 ms (11.9) |

About 13 ns an instruction natively, from 30. Over those race frames,
with the geometrizer on its thread, `overrevb` runs at 3.3 times real
time natively (1.9 before) and `sgt24h` at 2.7 (1.2 before). `doa`'s
whole 600M-instruction coin-start run (4,141 frames) runs at 2.69x real
time natively, the geometrizer on its thread (slowest frame 14.3 ms;
0.82x and 50.3 ms interpreted).
Every game's state hash is unchanged, recompiled and interpreted, and `make
lockstep` is identical for 600M instructions on `overrevb`, `sgt24h`,
`dynabb97` and `doa` (the last two run the geometrizer crate compiled from
the Jaleco uploads, with their own data).
