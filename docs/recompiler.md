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
one (games can share these: `overrevb` and `sgt24h` upload the same two
SHARC programs). Each holds
`program.rs` from the code generator, generic over the machine around the
processor, and a `lib.rs` that instantiates it for the Model 2 board
(`Model2Bus`, the TGP's external buses), so its machine code is compiled in
that crate and nowhere else. Program crates depend on `i960`, `mb86235`, `adsp2106x` and
`remodel-model2`; model2 depends on none of them. A SHARC crate's
`roles.txt` says which processor(s) the program was captured on (`copro`,
`geo`), and its glue instantiates it for those buses only.

model2 takes the programs at run time instead: `model2::recomp::register`
installs a table of (hash, i960 blocks), (hash, copro and geometrizer
`step`) and (hash, image, compiled words, copro and geometrizer `run`), and
the machine looks programs up there by hash. The front ends
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
- **SHARC programs** go 256 words to a submodule. `overrevb`'s
  geometrizer crate (2,987 words, 1.3 MB of Rust) builds in about 56 s
  with available memory going from 18.6 GB to 8.6 GB (16 codegen units in
  parallel); its copro crate (2,111 words) in about 40 s. Both are compiled
  for the bus they run on only; for both buses they took twice as long.

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

## i960 (`crates/i960/src/codegen.rs`)

**Which code.** All program code is in ROM, so it can't change at run time.
`remodel recomp` takes the static code discovery (`analyze`: recursive
descent from the boot record, interrupt table and system procedures) and
adds every address the interpreter executed in `make run` (`coverage.txt`),
including the targets of indirect calls and jumps (`indirect.txt`). Code
found only by guessing that an `lda` constant is a code pointer is left out
unless it ran.

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
writes them to `out/GAME/sharc-copro.bin`, `sharc-geo.bin`), so they are
keyed by a hash of the image. `remodel recomp` compiles the words that
static discovery (`codegen::discover`, also behind `sharc-disasm`) reaches
from the reset and interrupt vectors, following jumps, calls, loops, delay
slots and the command dispatch tables (both programs store their handlers'
addresses as immediates at init and jump through them: discovery takes
immediates that point into the image as code), plus anything `make run`
saw execute (`sharc-*-coverage.txt`). For `overrevb` that is 2,111 of the
copro's 4,096 words and 2,987 of the geometrizer's; nothing executed in a
coin-start run is missing from the static set.

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
slot (after running any bubbles, as the interpreter's `step` does); `run`
loops over it until the bus's `after_step` says stop, which is how the
board's stopping rules (the copro waiting on a FLAG, the geometrizer's end
of frame) are kept exact without a call per instruction.

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
state load the program is found by hash, or else by its compiled words
being in memory, and attached again.

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
| recompiled | 9.3 ms (slowest 12.4) | 10.7 ms (worst 12.5) |

The race runs at 1.8 times real time natively with the geometrizer on its
thread, so `overrevb` reaches full speed in both.
