# The i960 JIT (spike)

The static recompiler (`docs/recompiler.md`) makes the game's code fast,
but only for someone with this repository, a Rust toolchain and the
patience for a build, and what it builds is derived from the game, so it
can't be handed out. The JIT does the same translation inside the player,
at run time, from the ROM the player was pointed at, so a downloaded
build could get recompiled speed with nothing game-derived in it. This page is the
spike that tests whether that is worth it: one back end (native code
through [Cranelift](https://cranelift.dev/)), the i960 only, opt-in.

```sh
make bench JIT=1                  # the bench session on the JIT: same state hash, its counters
make jit-lockstep GAME=srallyc    # the JIT beside the interpreter, block by block (JIT_STEPS, default 300M)
make jit-translate                # the whole program translated ahead of time, timed
remodel --jit ... / remodel-play --jit   # --jit-hot N, --jit-opt speed
```

No recompiled build is involved: `JIT=1` works on an ordinary `make
release`, and `RECOMP=1` and `JIT=1` together run the JIT (it takes the
place of recompiled i960 code; the copro and geometrizer programs stay
recompiled). The default paths are unchanged, and the browser build
doesn't contain it (`crates/jit` is empty on `wasm32` and `crates/model2`
depends on it only off `wasm32`).

## Design

### Front end: the recompiler's, shared

- **Decoding and control flow** are the recompiler's: `i960::decode` and
  its `Flow`. A block is cut by the same rule as `codegen::program_module`
  cuts one (it ends after any control transfer), but where the static
  recompiler cuts at every leader of a whole-program analysis, the JIT
  starts a block wherever execution arrives (`Jit::plan`): from there to
  the first control transfer, at most 128 instructions, stopping before
  the game's idle-loop starts (the machine looks for those between blocks,
  `docs/idle-skip.md`) and where memory changes from ROM to RAM. Blocks may
  overlap; each instruction still retires exactly as interpreted. For the
  ahead-of-time measurement (`jit-translate`) the program is cut at
  `codegen::leaders` exactly as `make recomp` cuts it.
- **Lowering** is shared: `i960::lower` says what each translated
  instruction does as an `Op` on the register file (`Alu`, `Load`,
  `CmpBranch`, `Call`, ...), and both back ends consume it:
  `codegen.rs` prints it as Rust, the JIT emits it as Cranelift IR. The
  recompiler's output is unchanged by the split (checked: the old
  `translate` and the new printer give identical text for 4.4 million
  random instruction words that translate). Anything `lower` leaves out
  (floating point, the rare and the system instructions) runs through the
  interpreter's `Cpu::exec` with the decoded instruction, as recompiled
  code does: in `topskatr` 2,184 of the 53,431 instructions (4%), in
  `srallyc`'s hot code about a fifth (it is float-heavy).

### Back end: Cranelift

- **One function per block**, `extern "C" fn(cpu, host, ctx) -> u32` (0
  ran, 1 faulted). It is generic over the machine (`Jit<H: JitHost>`):
  every helper it calls is a Rust function monomorphised for `H`, so for
  `Model2Bus` the bus's `read32`, `retire` and so on are inlined into the
  helpers as they are into recompiled code.
- **Registers** stay in `Cpu` (`r`, `ac`, `ip`, `icount`): each operand
  is a load from, each result a store to, a fixed offset in the CPU
  (`offset_of!`). Nothing is cached in host registers across helper
  calls, so the interpreter's helpers (`Cpu::call`, `ret`, `exec`) always
  see the true state.
- **Inline**: integer and logical operations, shifts with the i960's
  rules (0 from a count of 32, `shri` capped at 31), compares and the
  condition code, condition tests and branches, `lda`, `mov` to `movq`,
  effective addresses.
- **Calls**: memory accesses (`read8/16/32`, `write8/16/32`), `call` and
  `callx`, `ret`, and fallbacks. A memory helper first tells the machine
  the instruction's address (`Host::at`), as recompiled code does before
  every instruction (the bus only uses it to attribute accesses).
- **After each instruction**, one call (`step`, or `step_last` for a
  block's last): count it, `Host::retire` it (the TGP, timers, vblank,
  interrupt lines run), and leave the block if the machine says stop (a
  frame ended, the budget ran out), an interrupt is due, or a fallback
  moved the IP (an IAC reinit, `modpc` letting an interrupt in). This is
  the recompiler's per-instruction sequence moved into one helper: it
  keeps the IR small (translation time is mostly Cranelift's, in
  proportion to the IR).
- **No block linking**: every block returns to the dispatcher
  (`Machine::dispatch`), which takes due interrupts, checks the idle loops
  and looks the next block up in a page table by address. The profile
  below puts the dispatcher and lookup at 2-3% of host time. Linking
  (a block jumping straight to its translated successor) would have to
  stop at idle-loop starts and keep the dispatcher's interrupt check; it
  isn't worth it before the bigger items below.
- **Hot threshold**: a block start is interpreted twice before it is
  translated (`--jit-hot N`), which leaves most of the boot code, run
  once, on the interpreter. Cranelift's optimiser is off and its
  single-pass register allocator on (`--jit-opt speed` for the optimiser
  and the full allocator): that halves the translation time, and the
  blocks run as fast either way (Results).

### Timing stays exact

The machine sees what it sees from the interpreter: one `retire` per
instruction, in order, with the same instruction count; interrupts taken
at the same instruction boundary (a block leaves as soon as one is due,
the dispatcher takes it); a frame ends after the same instruction; idle
loops are entered from the dispatcher, so idle skipping probes and skips
the same passes (the skip counts in `make bench` are the same). Faults
stop where the interpreter stops. So state hashes are the interpreter's,
and `jit-lockstep` compares registers, flags, IP, process controls and
cached register frames after every block.

### Code in RAM

`srallyc` copies its program to work RAM (`0x5A0000-0x5FCB3C`) and runs it
there, which the static recompiler can't reach (`docs/recompiler.md`).
The JIT translates it like ROM code, and keeps it honest:

- The bus counts writes (`Model2Bus::count_code_writes`, on only with the
  JIT): one counter per KB of the two RAMs code can run from (0x200000
  and work RAM), indexed by the physical memory, so a write through a
  mirror counts too. Code anywhere else stays on the interpreter,
  including the original Model 2's mirror of the program ROM at 0x220000,
  which the bus doesn't call ROM (`daytona` runs 0.5% of its
  instructions there).
- A RAM block keeps the words it was translated from and the counters of
  the (at most two) pages they are on. On entry, unchanged counters mean
  unchanged code; moved counters mean a word-by-word compare, and the
  block is kept (counters noted) or dropped and translated again from
  what is there now.
- After each of its own stores (and fallbacks) a RAM block checks its
  counters inline and, if they moved, leaves after that instruction, so a
  block that overwrites itself never runs stale code.
- A loaded save state, or run-ahead's copy, replaces RAM without the bus
  seeing writes: the machine then moves every counter
  (`Model2Bus::code_written_everywhere`), so each RAM block compares its
  words on its next entry (dropping them would mean translating them
  again every frame with run-ahead on).
- Not done: the code memory of a dropped block isn't freed (Cranelift's
  `JITModule` frees only everything at once). The measured games never
  drop one (`srallyc`: 0 rechecks, 0 drops over 600M instructions), and
  a unit test covers the path.

### Cache (designed, not built)

Translation is cheap enough (below) that the spike has no cache. If one is
wanted, the first step is not machine code: keep the list of hot block
starts per program hash (a few KB) and translate those on a worker
thread at boot, before the game reaches them. A machine-code cache would
need the helper addresses as relocations instead of constants
(`cranelift-module` imports rather than `iconst`), a key of program hash,
block words and the JIT's version, and invalidation on any reModel
update. Either is derived from the user's ROM and lives only on their
machine.

## Results

Measured on the development machine (16 cores, shared with other work,
so runs of the same thing vary by up to 10%).

**Correctness.** Bench state hashes (`make bench`, 600M instructions of a
coin-start run) are the interpreter's on the JIT for every set in
`tools/expected-hashes.txt` (`make bench-all JIT=1`: `srallyc`,
`lastbrnx`, `topskatr`, `vf2`, `daytona`, `hotd`, `vcop`, `desert`,
`sfight`, `schamp`, `fvipers`, `stcc`, `bel`; the first five also with
the optimiser on). `make jit-lockstep` is
identical for 300M instructions of a coin-start run (into the race) on
`topskatr` and `srallyc`, translating at the default threshold and at
`--jit-hot 0` (every block on its first visit, boot code included), with
the optimiser on and off. Unit tests (`crates/jit`) run hand-assembled
code (loads, stores, calls, compares, bit tests) on both and compare, and
check that a block in RAM is translated again when overwritten, by other
code or by its own store.

**Speed.** `make bench`'s session (600M instructions, 4,141 frames, 72 s
emulated), host seconds, lower is better, with the player's geometrizer
(`--geo native`) and the CLI's default (`--geo chip`, the game's own
geometrizer program on its thread). Medians of two or three runs (three for the native geometrizer). The
recompiled build is the main checkout's `target/recomp` of the same
day (`RECOMP=1`: its i960, copro and geometrizer programs recompiled);
`--interp` on it runs its i960 on the interpreter, which separates the
i960's part from the copro's and geometrizer's.

| | `topskatr` native | `topskatr` chip | `srallyc` native | `srallyc` chip |
|---|---|---|---|---|
| interpreter | 8.17 | 23.6 | 7.42 | 7.44 |
| **JIT** (default) | **5.62** | **21.0** | **4.97** | **4.94** |
| of which translating | 0.38 | 0.40 | 0.44 | 0.45 |
| JIT, Cranelift optimiser on | 5.97 | 21.3 | 5.43 | 5.45 |
| of which translating | 0.80 | 0.84 | 0.97 | 0.98 |
| recompiled build (`RECOMP=1`) | 5.12 | 14.1 | 6.97 | 6.95 |
| recompiled build, i960 interpreted | 8.22 | 16.6 | 7.34 | 7.14 |

- **`srallyc`**: the JIT is 1.5x the interpreter and 1.4x the recompiled
  build, which can't reach its code in RAM (2,362 of the 2,372 blocks it
  translates are from RAM).
- **`topskatr`, native geometrizer** (as the player runs it): 1.45x the
  interpreter, within 10% of the recompiled build. Without translation
  the JIT's run is 5.24 s against the recompiled 5.12: the same speed,
  from Cranelift with no optimiser against LLVM with the bus inlined
  into every block.
- **`topskatr`, chip geometrizer**: the geometrizer thread (the
  geometrizer's MB86235 program, interpreted, about 20 s busy) is the
  limit, so the i960 hardly matters; the recompiled build is faster
  because its geometrizer program is recompiled.
- The optimiser doubles the translation time and makes the blocks no
  faster (run time without translation: `topskatr` 5.24 s off, 5.17 s
  on; `srallyc` 4.53 and 4.46), so it is off by default.

**Translation time.** In the bench session (default settings) the JIT
translates `topskatr`'s 2,789 hot blocks (27,409 instructions) and
`srallyc`'s 2,372 (26,752) in about 0.4 s each: 140-190 µs a block, 14-17
µs an instruction, almost all of it Cranelift compiling (making the code
executable is 2-4%). The slowest block (128 instructions) took 3-5 ms, up
to 25 ms on the loaded machine: a frame's worth, so a worker thread
should take translation off the emulation thread before this ships. The
whole program translated ahead of time (`make jit-translate`: 53,431
instructions in 11,061 blocks, as `make recomp` cuts `topskatr`) takes
0.78 s (1.69 s with the optimiser), where rebuilding one game's
recompiled crates takes 87 s (`docs/recompiler.md`).

**Where the time goes.** A sampling profile (the interrupted instruction
address every 0.5 ms of CPU time, symbolised with the build's debug
info) of the same session, single-threaded with the native geometrizer,
by part of the machine:

| share of host time | `topskatr` interpreter | `topskatr` JIT | `srallyc` interpreter | `srallyc` JIT |
|---|---|---|---|---|
| i960: interpreter, or JIT code, its helpers and the dispatcher | 38% | 11% (5% in the translated code) | 24% (and most of the bus line: fetching its code from RAM) | 15% (4%) |
| translating (Cranelift; with its allocations, most of the libc line) | | 5% | | 8% |
| copro program on its interpreter (MB86235; `srallyc` MB86234) | 26% | 35% | 21% | 26% |
| bus: memory and I/O accesses | 12% | 18% | 29% | 15% |
| per-instruction bookkeeping: `retire` (timers, vblank, IRQ lines) and the interrupt check | 10% | 14% | 9% | 11% |
| geometrizer (native) and rasteriser | 9% | 11% | 13% | 19% |
| libc, other | 4% | 6% | 4% | 7% |

In seconds, `topskatr`'s i960 goes from 3.4 s to 0.8 s of an 8.9 s and a
6.9 s run (4.4x), but everything else stays, so the run is 1.3-1.45x
faster. After the JIT the copro is the largest part, then the bus (the
same calls the recompiled code makes, but not inlined into the block),
then the work every instruction costs whoever executes it: `retire` and
the interrupt check (`Cpu::interrupt_due` scans eight words per
instruction). The split between copro and geometrizer is by function
name; on 2A the native geometrizer's MB86235 helpers count as
geometrizer.

## What is missing for the copro programs

The JIT covers the i960 only. After it, the copro is the largest single
cost in both games measured (a quarter to a third of host time, above),
and in the chip geometrizer mode the geometrizer program is too. What a
JIT for those programs would need, by chip:

- **Front ends partly exist**: all three have a decoder, a disassembler
  and an interpreter. The SHARC also has the static recompiler's code
  discovery (`adsp2106x::codegen::discover`: vectors, jumps, calls, loops,
  delay slots, the command dispatch tables) and its region rules
  (`codegen::in_block`, `Sharc::block_start`/`block_end`), which say which
  runs of words can execute as one block and how the pipeline, loop logic
  and counters are brought up to date at its end; that carries over to a
  JIT as the i960's block rules did. The MB86235's recompiled words are
  the ones a run executed (no discovery, no blocks), and the MB86234 has
  no recompiler.
- **The lowering doesn't**: the recompiled MB86235 and SHARC code is
  each word's *interpreter step* with its decoding as a constant, and
  rustc does the specialising (decode, operand dispatch and unused paths
  fold away). A JIT has no rustc at run time, so each instruction's
  semantics must be written out as IR: for the MB86235 a VLIW word's ALU,
  multiplier, transfer and control fields with their parallel reads,
  FIFO stalls and the float formats; for the SHARC compute (ALU,
  multiplier, shifter, with `float::single`'s exact rounding), the DAGs,
  delayed branches and the loop and status stacks. That is an
  `i960::lower` per chip, several times larger than the i960's, and each
  needs the same lockstep (`make lockstep` already compares the SHARCs).
- **A cheaper first step**: blocks that call the interpreter's step for
  each word with its decoded form passed in, as the i960 JIT does for its
  fallbacks. That removes fetch and dispatch but not the decoded form's
  interpretation; worth measuring before writing full lowerings.
- **Or no JIT at all**: the native geometrizers (`docs/hw/geometrizer-2b.md`,
  `-2c.md`) replace the geometrizer programs outright and are already the
  player's default, 20 times faster than the recompiled TGP. The copro
  programs could get the same treatment per program family; that is per
  game family work, where a JIT is once per chip.
- **Model 2A's MB86234** (`srallyc`) has no recompiler at all; its
  interpreter is about a quarter of `srallyc`'s host time after the JIT.

## A WebAssembly back end (sketch)

The browser can't take native code, but it can compile WebAssembly the
page makes at run time, which is the same idea one level up:

- **Shared**: `plan` (block cutting), `i960::lower`, the per-instruction
  helper ABI (`step`, memory, `call`/`ret`/`exec`) and the RAM-code
  checks carry over unchanged. Only the emitter is new: `Op` to wasm
  instructions (with `wasm-encoder`) instead of to Cranelift IR, a
  straight translation (wasm is a stack machine with the same integer
  operations; the i960's shift rules need the same `select`s).
- **Linking to the running module**: the blocks run in their own module,
  instantiated by the page with the main module's exports as imports: its
  linear memory (so `Cpu` fields are loads and stores at the same
  offsets, the CPU's address passed in) and the helpers, exported from
  `remodel-web` as plain functions taking pointers. The dispatcher calls a
  block through a shared function table (`call_indirect` by slot), so
  `Jit::run` stays as it is with a table index for a code pointer.
- **Batching**: compiling and instantiating a module costs milliseconds,
  so blocks can't be modules one by one. Collect the hot blocks a frame
  finds, emit them as one module between frames (on a worker, with
  `WebAssembly.compile`), and install them when it is ready; until then
  they run on the interpreter as now. That also makes the hot-block list
  the natural cache (above).
- **Checking**: the same lockstep, in Node (as `docs/web.md` times the
  browser build).

## Effort

Rough, for one person who knows this codebase, with lockstep and the
bench hashes as the gate at each step:

| Piece | What | Estimate |
|---|---|---|
| i960 native JIT, finished | floats translated (the largest fallback group in both games: `mulr`, `addr`, `subr`, `movr`, `cvtir`, `cmpr`); an inline fast path for plain RAM and ROM reads (and writes, with the bus's write count and code counters); the per-instruction step cheaper (the interrupt check is a scan of eight words each time); translation on a worker thread with the hot-block list kept between runs; code memory reclaimed; a player setting; `jit-lockstep` over every game | 2-3 weeks |
| Copro JITs | MB86235 (2C copro), ADSP-21062 (2B copro and geometrizer), MB86234 (2A): a lowering each, blocks from the existing discovery and region rules, lockstep per chip | 2-4 weeks each (SHARC the most) |
| WebAssembly back end | the emitter (`Op` to wasm), helper exports and a shared table in `remodel-web`, batched modules compiled off the main thread, Node lockstep | 3-4 weeks for the i960; each copro lowering then a further week or so, as the emitter is shared |

## Verdict

Go, with the scope corrected. The JIT is exact, small (one crate, one
shared lowering), cheap to translate (well under a second, against minutes
of rustc) and needs nothing game-derived in the binary, so a downloaded
player can have it. It already matches the recompiled build's i960 on
`topskatr` and beats it on `srallyc`, whose code the recompiler can't
reach. But the i960 is no longer where the time goes: after the JIT it is
11-15% of host time (4-5% in the translated code itself), and the copro,
the bus and the per-instruction bookkeeping are half to two thirds. The real-world gain of
the i960 JIT alone is 1.45-1.5x on these two games in the player's
configuration; more needs, in order of what the profile says they buy:

1. a copro back end (or native copro programs, as the geometrizers have),
2. an inline fast path for plain memory and a cheaper per-instruction
   step (interrupt check, timers),
3. floats in the i960 lowering (the largest fallback group; small in time).

The WebAssembly back end is worth doing on the same front end once the
native one has items 1 and 2, since the browser's interpreters are slower
still and the same shares apply.
