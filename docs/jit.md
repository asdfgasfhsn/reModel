# The JITs (spike): the i960 and the copros

The static recompiler (`docs/recompiler.md`) makes the game's code fast,
but only for someone with this repository, a Rust toolchain and the
patience for a build, and what it builds is derived from the game, so it
can't be handed out. The JIT does the same translation inside the player,
at run time, from the ROM the player was pointed at, so a downloaded
build could get recompiled speed with nothing game-derived in it. This page is the
spike that tests whether that is worth it: one back end (native code
through [Cranelift](https://cranelift.dev/)), the i960 first, then the
copro programs ("Copro JITs"), opt-in.

```sh
make bench JIT=1                  # the bench session on the JITs: same state hash, their counters
make jit-lockstep GAME=srallyc    # the JIT beside the interpreter, block by block (JIT_STEPS, default 300M)
make jit-translate                # the whole program translated ahead of time, timed
make jit-copro-lockstep GAME=vf2  # the copro JITs beside their interpreters, run by run
remodel --jit ...                 # --jit-hot N, --jit-opt speed, --jit-copro off|check
remodel-play --no-jit             # the player has them on by default; this turns them off (make play JIT=0)
```

No recompiled build is involved: `JIT=1` works on an ordinary `make
release`, and `RECOMP=1` and `JIT=1` together run the JITs (they take
the place of recompiled code; with `--jit-copro off` the copro and
geometrizer programs stay recompiled). The native player runs on the JITs by default (`--no-jit`,
`make play JIT=0` to turn them off; where Cranelift has no back end for
the host it falls back on its own); the CLI's default is still the
interpreter, the reference. The browser build
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
  effective addresses; reads and writes of plain memory ("Plain memory
  inline"); the step after an instruction while the machine is quiet ("A
  cheaper step").
- **Calls**: other memory accesses (`read8/16/32`, `write8/16/32`: I/O,
  unaligned, unmapped), `call` and `callx`, `ret`, and fallbacks. A
  memory helper first tells the machine the instruction's address
  (`Host::at`), as recompiled code does before every instruction (the bus
  only uses it to attribute I/O accesses). `call`, `ret` and the
  fallbacks run the interpreter's own code on the machine wrapped so that
  its plain memory is reached as the inline paths reach it (`Plain`: a
  call's frame, 16 words written through, no longer goes through the
  bus's lookups).
- **After each instruction**, when the machine isn't quiet, one call
  (`step`, or `step_last` for a block's last): count it, `Host::retire` it
  (the TGP, timers, vblank, interrupt lines run), and leave the block if
  the machine says stop (a frame ended, the budget ran out), an interrupt
  is due, or a fallback moved the IP (an IAC reinit, `modpc` letting an
  interrupt in). This is the recompiler's per-instruction sequence moved
  into one helper: it keeps the IR small (translation time is mostly
  Cranelift's, in proportion to the IR).
- **No block linking**: every block returns to the dispatcher
  (`Machine::dispatch`), which takes due interrupts, checks the idle loops
  and looks the next block up in a page table by address. The profile
  below puts the dispatcher and lookup at 2-3% of host time. Linking
  (a block jumping straight to its translated successor) would have to
  stop at idle-loop starts and keep the dispatcher's interrupt check; it
  isn't worth it before the bigger items below.
- **Hot threshold**: a block start is interpreted twice before it is
  translated (`--jit-hot N`), which leaves most of the boot code, run
  once, on the interpreter. Cranelift's optimiser is off, its full
  (backtracking) register allocator on (`--jit-opt speed` for the
  optimiser too). With memory and the step inline a block is many small
  CFG blocks, around which the single-pass allocator (the default until
  then, and still the copro JITs') kept every value in a stack slot; the
  full one made the blocks about 20% faster for 40% more translation time,
  which the workers hide (Results).

### Plain memory inline

The machine hands each run of a block a table (`JitHost::fast_pages`):
for each 64 KB page below `FAST_SPAN` (64 MB, which covers the program
ROM, the RAMs, buffer RAM, tile and palette RAM and the data ROM), where
its bytes are if all of it is plain memory, and for writes the memory's
write count and, in code RAM, the page's first write counter. An access
aligned to its width and below the span (one `band` against a mask) to a
page with an entry is made inline: a read is a load, a write bumps the
memory's write count (`Mem::writes`, the renderer's version numbers, in
save states) and, in code RAM, its KB's counter by exactly what the bus
adds (2: it counts the first and the last byte's KB, the same one for an
aligned access), then stores. Everything else goes through the bus as
before: I/O, unaligned accesses (which may cross a counter's KB or a
page), Model 2A's video RAM (whose writes are mirrored), memories
smaller than a page, ROM writes. `Model2Bus` makes the table when the JIT
starts counting code writes and again after a state load, and gives an
empty one while an idle-loop probe or a write watch is looking at
accesses. The blocks of code in RAM still check their counters after
each store, inline path or not ("Code in RAM").

### A cheaper step

Most retires do nothing but pass time: the copro is stopped or waiting
on its input, and no timer, vblank or UART event is due. The machine
says how many of the next retires are like that (`JitHost::quiet`, for
`Model2Bus` the idle-loop skip's own bound, `quiet_retires`, when the
copro is idle, no interrupt line is waiting to be passed on, and less
than the budget). While that count lasts, a block's step is inline: count
the instruction, take one off the count, go on. The retires are owed, and
made at once (`JitHost::retire_quiet`: time and the budget move on) before
anything reaches the machine: every helper call settles them first and
leaves the count at 0, so the step after it is the full one, which
retires and asks again. No interrupt can become due meanwhile (lines are
raised only by events and I/O, priorities change only in helpers), and
the dispatcher sees the machine exactly as after that many single
retires. The interrupt check itself (`Cpu::interrupt_due`, before every
instruction in the interpreter too) is one compare when nothing is
pending: the CPU keeps a byte saying which of its eight pending words are
nonzero, rebuilt from them on a state load (it isn't saved).

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

### Translation off the game's thread

Translating a block takes Cranelift 0.1-5 ms (25 ms on a loaded
machine), the geometrizer's first frames tens of milliseconds more, so
translating where the code is needed made the game hitch: in the bench
session the JIT's slowest frame was 71 ms (`topskatr`, native
geometrizer), 208 ms (`topskatr --geo chip`), 59 ms (`srallyc`) and 43 ms
(`vf2`), against 8-16 ms interpreted. So no JIT translates on the thread
that runs its chip any more (`crates/jit/src/pool.rs`):

- **A pool of workers** shared by every JIT in the process (the i960's,
  the MB86234's, both MB86235s', run-ahead's and the launcher's
  machines): the core count less three, one to eight
  (`REMODEL_JIT_WORKERS=N` to choose). Each has its own Cranelift
  context and builds and compiles a block's IR to bytes.
- **The JIT plans** a block on its own thread (it reads the code: ROM,
  RAM, the uploaded program), hands the plan to the pool and goes on
  interpreting. The bytes come back on the JIT's channel, and the JIT
  copies them into its own executable memory (its Cranelift module: the
  code still belongs to, and is freed with, the JIT) when it next reaches
  a start that is waiting, which is a block boundary: the dispatcher is
  between blocks. The game never waits.
- **The rest of a waiting block isn't a start.** Interpreted one
  instruction (or word) at a time, every address after a waiting
  block's start would reach the hot threshold and be queued too (14,584
  i960 blocks for `topskatr`'s 2,787); the addresses up to a waiting
  block's end, entered from its start, are run without being counted.
- **Exactly the same machine.** A block and the interpreter retire the
  same instructions with the same effects (above), so when a block is
  installed changes the host's time, not the state: hashes are the
  interpreter's (`make bench-all JIT=1`), and `make jit-lockstep` and
  `make jit-copro-lockstep` still compare after every block or run,
  whichever ran it.
- **Code written while it was translated.** An i960 block from RAM keeps
  the words and page counters it was planned from (as an installed one
  does): one whose code changed before it was installed is thrown away
  ("translations thrown away" in the report) and planned again on the
  next visit, and an installed one is still checked on every entry. A
  copro result for a program the chip no longer holds (another upload;
  a loaded state) is thrown away, as is one for a JIT that was dropped
  (its jobs not yet started are skipped).
- `--jit-sync` translates at once on the thread that needs it, as
  before (the unit tests do, so that every block they reach is
  translated code; `make jit-test` also runs the random programs with
  the workers, installing at random points). `make jit-translate` hands
  the whole program to the workers and waits.

### Hot-block list

The workers hide translation, but a cold boot still runs the game's first
seconds partly interpreted. So every JIT notes the blocks it translates
in a list (`crates/jit/src/hotlist.rs`), kept between runs, and at boot
hands the workers every block listed for the programs it is given,
behind any the game is waiting for: on 16 cores the whole list is
translated in a fraction of a second, before the game gets to most of it.

- **What is kept: addresses.** Block starts, by chip (`i960`, `mb86234`,
  `mb86235`) and program hash: the i960's program ROM hash (the one save
  states carry), the hash of an uploaded copro program (the MB86235's
  own, FNV-1a of the MB86234's words). No game code and no machine code:
  a few KB of text, derived from the user's ROM and written only on
  their machine. Nothing of it is in the repository.
- **The file**, one per game set: `hot.txt` in `out/SET/jit/` in a
  source tree, else in the user's cache folder
  (`~/.cache/remodel/jit/SET/` on Linux); the CLI's with `--jit-cache DIR`
  (none without it, so `make bench JIT=1` stays a cold boot). The first
  line is the format's version (`reModel JIT hot blocks 1`); another
  version, or a file that doesn't parse, is ignored with a warning and
  replaced on the next save. Saved (through a temporary file) only when
  it grew: by the player on leaving a game and every 30 s, by the CLI at
  the end of a `run`.
- **Entries are hints.** Each is planned from the code as it is now, as
  the game's own would be: an address that doesn't decode is skipped
  (left to the game: it isn't marked as untranslatable), one from
  another version of the program is a translation wasted at most, and a
  block from RAM keeps its words and is checked on every entry as
  always. Listed RAM starts (`srallyc` runs from work RAM) are planned
  when the game first runs code in RAM, when it has copied its code
  there. A block the game never reaches is never run, so the list can't
  change what the game does: `make bench-all JIT=1` hashes are the same
  cold and warm.
- **Shared**: a machine's JITs share one list (run-ahead's machine too,
  `Machine::enable_jit_with`).

### Pre-flight

```sh
make jit-warm GAME=topskatr      # a headless coin-start run (JIT_WARM_STEPS) writes out/topskatr/jit/hot.txt
make jit-hitch GAME=topskatr JIT_HITCH_OPTS="--geo native"   # frame times: interpreter, JIT inline, cold, warm
remodel-play --jit-warm          # a game with no list yet gets one from a quiet run in the background
```

`make jit-warm` warms a list without playing: the bench session (boot,
attract, coin, start, into the race) on the JITs with `--geo chip`, so
the geometrizer's MB86235 program is listed too, writing the file the
player reads in a source tree (`JIT_CACHE=DIR` for another). In the
player, `--jit-warm` does the same for a game that has no list yet, like
a shader-cache precompile: a second machine on its own thread
(`model2::jit_warm`, no window, sound or saves) runs the session and
writes the list for the next boot; the game being played isn't touched
(the two only share the workers). `make jit-hitch` is the measurement:
the bench session's frame-time distribution (`run`'s `frame times`
line: host time from one frame's end to the next) on the interpreter,
the JIT translating where it runs (`--jit-sync`), and the JIT cold and
warm, with the state hash of each (the same four).

**Hitches, before and after** (`make jit-hitch`: the bench session,
600M instructions, 4,141 frames, unthrottled, on the 16-core machine
shared with other builds; host milliseconds per emulated frame; "inline"
is the JIT as it was, translating where it runs, `--jit-sync`; "cold"
the workers with no list, "warm" with the list the cold run left):

| | interpreter | JIT inline | JIT cold | JIT warm |
|---|---|---|---|---|
| `topskatr` native: worst frame | 9.4 | 72.3 | 12.0 | 6.8 |
| p99.9 / p99 | 8.3 / 6.1 | 23.6 / 5.0 | 6.2 / 3.0 | 6.2 / 2.8 |
| frames over 16.7 ms / 33 ms | 0 / 0 | 11 / 3 | 0 / 0 | 0 / 0 |
| `topskatr --geo chip`: worst frame | 18.6 | 210.4 | 16.4 | 6.8 |
| p99.9 / p99 | 15.7 / 14.4 | 29.3 / 10.5 | 7.0 / 5.0 | 6.4 / 5.0 |
| frames over 16.7 ms / 33 ms | 2 / 0 | 10 / 4 | 0 / 0 | 0 / 0 |
| `srallyc`: worst frame | 11.3 | 60.4 | 8.5 | 10.5 |
| p99.9 / p99 | 11.3 / 8.3 | 14.2 / 3.8 | 4.8 / 3.3 | 5.9 / 3.3 |
| frames over 16.7 ms / 33 ms | 0 / 0 | 4 / 2 | 0 / 0 | 0 / 0 |
| `vf2`: worst frame | 8.9 | 44.5 | 9.7 | 7.7 |
| p99.9 / p99 | 8.9 / 8.8 | 18.3 / 5.5 | 6.3 / 4.2 | 5.7 / 4.2 |
| frames over 16.7 ms / 33 ms | 0 / 0 | 5 / 3 | 0 / 0 | 0 / 0 |
| host seconds (`topskatr` native, chip, `srallyc`, `vf2`) | 8.0, 21.5, 8.4, 16.5 | 5.6, 9.8, 3.9, 9.0 | 4.6, 7.7, 3.5, 8.6 | 4.3, 7.7, 3.5, 8.5 |

The JIT's worst frame is now no worse than the interpreter's: what is
left are the frames that are long on the interpreter too (the first
race frame on `topskatr`'s chip geometrizer is 14-18 ms however it
runs; `srallyc`'s frame 9 and 2,642 are its own), and warm, the boot's
first frames run translated (`topskatr`: 66,000 instructions
interpreted outside blocks in its first 100M against 165,000 cold).
Translating on 8 workers also makes the whole run faster than inline
(`topskatr` native 5.6 to 4.6 s cold, 4.3 s warm). The state hashes are
the same in every column.

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
to 25 ms on the loaded machine: a frame's worth, which is why it now
runs on the workers ("Translation off the game's thread"). The
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

**Memory and the step inline** ("Plain memory inline", "A cheaper step",
the full register allocator). `make bench`'s session with `JIT=1`, host
seconds, the faster of three runs alternating the build before and after
on the same machine (busy with another session's hash loop, so within
about 0.1 s):

| | `topskatr` (chip geometrizer) | `topskatr --geo native` | `srallyc` | `vf2` | `zerogun` (2B) | `dynamcop` (2B) |
|---|---|---|---|---|---|---|
| before | 7.41 | 3.72 | 2.95 | 7.88 | 10.95 | 4.62 |
| **after** | **6.78** | **2.62** | **2.21** | **6.06** | **10.49** | **3.18** |
| faster | 1.09x | 1.42x | 1.33x | 1.30x | 1.04x | 1.45x |

Where the i960 is the limit (`topskatr` native, `srallyc`, `vf2`,
`dynamcop`) the session is 1.3-1.45x faster; with `topskatr`'s chip
geometrizer and on `zerogun` the geometrizer's thread (its program on its
JIT) is the limit, so the gain is what the game's thread no longer adds.
Along the way: the inline paths and the inline step alone (single-pass
allocator) 1.13-1.18x; the frame writes of `call` through `Plain`, and the
full allocator, the rest. Translation time went up with the IR, from
0.4 to 0.6-0.9 s per session (on the workers, hidden by them and by the
hot-block list).

The game's thread, sampled as above (every 5 ms of wall time, gdb under
`SIGUSR2`), as shares of each run and in seconds of the session's time
(the share times the bench time); translated code here includes the
copros' (their code is in the same kind of memory, and the unwinder
can't always say whose):

| | `topskatr --geo native` before | after | `srallyc` before | after |
|---|---|---|---|---|
| translated code (i960 and copro) | 38% (1.40 s) | 52% (1.35 s) | 31% (0.91 s) | 43% (0.94 s) |
| per-instruction: `step`, `retire`, the interrupt check | 20% (0.74 s) | 13% (0.33 s) | 23% (0.69 s) | 11% (0.25 s) |
| bus: memory and I/O | 16% (0.59 s) | 9% (0.23 s) | 13% (0.39 s) | 8% (0.19 s) |
| copro on its JIT's helpers, interpreter | 5% | 7% | 5% | 6% |
| dispatcher, idle loops | 3% | 4% | 4% | 8% |
| i960 interpreter (fallbacks, single steps) | 3% | 3% | 5% | 6% |
| translating (linking, allocation), libc, other | 15% | 13% | 19% | 18% |

What is left of the step is mostly the full step's own work: asking the
machine again how long it stays quiet (`quiet_retires`, 3-4%) and the
retires while the copro runs; of the bus, I/O (the copro's FIFOs and
status, the frame registers) and the bus's own lookups for accesses the
inline paths don't take.

**Exactness.** `make bench-all` and `make bench-all JIT=1`: 22 of 22
hashes. `make jit-lockstep` identical for 300M instructions on `srallyc`,
`topskatr` and `zerogun`; `make jit-copro-lockstep` identical on
`topskatr`, `vf2` and `zerogun`. Random programs (`make jit-test`,
`native::fast_tests`: loads and stores of every width at the edges of
ROM, code RAM, data RAM, an I/O page that counts its reads, the top of
the span and a mirror just above it, unaligned ones, stores over the
block's own code, events every 37 retires) compared with the interpreter
after every block, then memory, write counts, code counters, I/O and
event times: they fail on a quiet stretch one retire too long, a code
counter bumped once instead of twice, no code counter, the I/O page
taken as plain memory, and a slow read that doesn't settle the owed
retires first.

## Copro JITs

After the i960 JIT the copro programs were the largest single cost
(above), so the copros have back ends of their own, on the same Cranelift
setup (`crates/jit`: `mb86234.rs`, `mb86235.rs`, the shared module and
helpers in `copro.rs`):

- **MB86234**, the TGP of the original Model 2 and 2A (`srallyc`, `vf2`,
  `daytona`, `vcop`, `desert`, `vcop2`, `skytargt`, `manxtt`,
  `motoraid`): `jit::mb86234::Mb86234Jit`, run by `model2::tgp2a`.
- **MB86235**, Model 2C's copro and its geometrizer TGP (`topskatr` and
  the other 2C sets): `jit::mb86235::Mb86235Jit`, run by the bus (copro)
  and the geometrizer's engine (`--geo chip`), through
  `model2::copro_jit`.

They are on with `--jit` (`make bench JIT=1`) and by default in the player (`remodel-play --no-jit` turns them off);
`--jit-copro off` leaves the copros on their interpreters (the i960's JIT
alone, as measured below), and `--jit-copro check` runs each beside its
interpreter (`make jit-copro-lockstep`). With `RECOMP=1` and `JIT=1` the
copro JITs take the place of recompiled copro and geometrizer programs
too.

```sh
make jit-copro-lockstep GAME=vf2   # each copro JIT beside its interpreter, every run compared
make jit-test                      # the JITs' unit tests (random programs on both, compared)
```

### Lowering: the interpreter's step, written out as IR

There is no shared `lower` here as there is for the i960: the static
recompiler's MB86235 code is each word's interpreter step specialised by
rustc, and the MB86234 has no recompiler, so each JIT has its own
emitter that mirrors the interpreter function by function (`op`, `alu`,
`write_regs`, `post` for the MB86234; `exec`, `alu`, `mul`, `ea`,
`reg_read`/`reg_write`, `loc_read`/`loc_write` for the MB86235). The
decoding is done at translation time, so what the IR holds is what rustc
leaves of the step for a constant word: registers and RAM are loads and
stores at fixed offsets in the chip's struct and its RAMs, the field
decisions (which register, which view, which addressing mode, which RAM
a transfer type fixes) are the emitter's, and only the data is computed
at run time.

- **Every word is translated.** Words the MB86234 interpreter counts as
  unknown call its counter and do nothing, as there. The MB86235's
  `Step::Unsupported` words (MOVI, MOV4, format 3) are left out of
  blocks, so the interpreter's step meets them and stops the chip as
  before.
- **Floats exactly as the interpreters compute them.** MB86234: the
  "special values" model (`docs/hw/mb86234.md`) inline, an exponent of 255
  read as an ordinary binade (`f`), the operation in `f64`, one rounding
  to single (`fdemote`), and only a result outside single precision's
  range (saturation, 2^128) through the interpreter's own `word`. MB86235:
  IEEE single inline, and a NaN result through the interpreter's
  `float::add`/`sub`/`mul`/`rcp`/`rsq` (they define which NaN); FLOG calls
  `float::log2`. Conversions use Cranelift's saturating ones, which match
  Rust's `as` (NaN to 0).
- **Calls into the interpreter's pieces** for what has state outside the
  registers and RAM: the FIFOs (`VecDeque`s), the MB86234's return stack
  and external bus (shared RAM, data ROM and the function unit's ports,
  through `Cpu::ext_read`/`ext_write`), the MB86235's external bus
  (`&mut dyn External`), CLRF, the live status flags (FIFO states) where a
  condition or an ST read needs them, and the diagnostics.

### Blocks and timing

The two chips are scheduled differently, and each JIT keeps its
interpreter's timing word for word:

- **MB86234: synchronous.** The board runs it after each i960 access to
  its ports until it waits for input (`Cpu::run`, at most a million
  words). A block runs from a start to the first control word (jumps,
  calls, returns, loops; at most 64 words), and never holds a word that
  reads the input FIFO except as its first: the dispatcher makes the
  interpreter's check (`reads_fifo` and an empty FIFO) before every
  block, so a run stops on the same word with the same state. A block
  runs only if its words fit in the step limit left. A repeat (`3C04 n`)
  and the word it repeats are one loop in the IR; a start under a repeat
  (a stop for input can leave one) is interpreted.
- **MB86235: interleaved.** The copro runs two words after every i960
  instruction (`Model2Bus::retire_insn`), the geometrizer 256 at a time.
  `Mb86235Jit::run` has `Tgp::run`'s contract (up to `max` words), and a
  block counts its words against that budget and leaves after whichever
  one spends it, with the state the interpreter would have there: the PC,
  a delayed branch pending (its delay slot not yet run), a repeat part
  done. Within a block the delay slot follows its branch, and the block
  leaves for the target if taken or goes on; the word after a REP checks
  the repeat count as it runs (with the interpreter's nop skip), and so
  does a block's first word, which may start under a repeat. A stall (an
  FI read with the FIFO empty) leaves before the word. A block start with
  a delayed branch pending, and a branch in a delay slot or under a
  repeat, are the interpreter's (a word at a time: 1.1 million of the copro's
  words in `topskatr`'s session, 0.17 million of the geometrizer's). Blocks are at most 64
  words for the geometrizer and 16 for the copro: run two words at a
  time, the copro's blocks start almost anywhere and rarely run long,
  and shorter ones cost less to translate.

Translation is queued at first use (no hot threshold: the programs are small; on the workers,
"Translation off the game's thread"),
for the program the chip holds; another upload starts again (MB86234: a
new translation; MB86235: the program found by its hash), and the code
of a program dropped is freed with its module (unlike the i960's). The
programs can't write their own code (the MB86235's MOVI is unsupported,
the MB86234 has no program store).

### Correctness

- `make jit-test`: each JIT against its interpreter on random programs
  (every word form and format, random registers, RAMs, external memory
  and FIFO input, special float values), compared in full after every
  run; for the MB86235 with runs of 1, 2, 3, 7, 50 and 256 words, so
  budgets cut blocks everywhere (release build: 400 and 40 MB86234
  programs, 300 and 30 MB86235 ones; a few in `make check`'s debug
  build). Breaking one detail of a lowering (an operand order, a return
  address, a delay slot's target, the ring of a circular operand) fails
  them at once.
- `make jit-copro-lockstep` (300M i960 instructions of a coin-start run):
  identical on every MB86234 game (`srallyc`, `vf2`, `daytona`, `desert`,
  `vcop`, `vcop2`, `skytargt`, `manxtt`, `motoraid`: 0.16 to 6 million runs
  compared each) and on `topskatr`'s copro and geometrizer (0.5 and 0.2
  million runs). For the MB86234 every run's whole state and its external
  writes are compared; for the MB86235 the registers, FIFOs and writes
  every run, its RAMs and execution counts every 64th run and at every
  stall.
- `make bench-all` and `make bench-all JIT=1`: all 18 expected hashes.
  `make tgp-check GAME=srallyc` (the interpreter's): 0 unknown words.

### Results

`make bench`'s session (600M i960 instructions, coin start), host
seconds, medians of two runs (which agreed within 1%). The 2A sets with
`--geo chip`, the CLI's default (their geometrizer is fixed-function,
the same either way); "recompiled" is the main checkout's `target/recomp`
of the same day (`RECOMP=1`: the i960 and the MB86235 programs
recompiled; the MB86234 has no recompiler, so on 2A it is the i960's
alone, which can't reach `srallyc`'s code in RAM).

| | `srallyc` | `vf2` | `daytona` | `topskatr` native | `topskatr` chip |
|---|---|---|---|---|---|
| interpreter | 7.02 | 14.95 | 10.30 | 7.94 | 21.4 |
| i960 JIT only (`--jit --jit-copro off`) | 4.90 | 11.32 | 7.13 | 5.80 | 20.6 |
| **i960 and copro JITs** (`--jit`) | **3.88** | **8.93** | **5.88** | **4.77** | **8.76** |
| recompiled build (`RECOMP=1`) | 7.01 | 9.35 | 6.13 | 5.06 | 13.7 |

- **MB86234**: the copro JIT takes 1.0-2.4 s off each 2A run (18-21% of
  the i960-JIT run), and the JIT build is 1.8x the interpreter on
  `srallyc`, 1.7x on `vf2` and `daytona`, and ahead of the recompiled build
  on all three. Translating costs 23 ms (`srallyc`, 208 blocks) to 64 ms
  (`vf2`, 497 blocks).
- **MB86235, the copro** (`topskatr` native, the player's configuration):
  1.0 s off (18%); the whole run is 1.66x the interpreter and 6% faster
  than the recompiled build. Its 730 blocks take 0.28 s to translate.
- **MB86235, the geometrizer** (`--geo chip`): its thread went from 19 s
  busy to 6.4 s, and the run from 20.6 s to 8.8 s (2.4x), 1.6x faster than
  the recompiled build. The native geometrizer is still far cheaper
  (0.8 s).

**Where the time goes now.** A sampling profile of the main
thread (every 10 ms, under gdb: the main thread's stack at each sample,
by function name; 430-940 samples a run) of the same session with
`--jit --geo native`, by part of the machine:

| share of the main thread | `srallyc` | `vf2` | `topskatr` |
|---|---|---|---|
| bus: memory and I/O accesses | 19% | 18% | 20% |
| translated code (both JITs; frames without a name, which gdb can't attribute) | 16% | 16% | 21% |
| translating (Cranelift; the i960's 0.4 s and the copro's) | 11% | 7% | 12% |
| i960 interpreter (the JIT's fallbacks: floats; `exec`) | 10% | 11% | 10% |
| per-instruction bookkeeping: `retire`, the interrupt check | 10% | 14% | 8% |
| i960 JIT helpers and dispatcher (and the translated code gdb names) | 11% | 13% | 8% |
| the copro: its dispatcher, board glue, interpreted words and the translated code gdb names | 6% | 7% | 6% |
| native geometrizer (its MB86235 helpers), rasteriser | 4% | 2% | 5% |

The copro, the largest part after the i960's JIT, is now a few percent
plus its share of the translated code. What is left is what every
instruction costs whoever runs it: the bus (the JIT's memory accesses are
calls), `retire` and the interrupt check, the i960's float fallbacks,
and translation, which is 7-12% of a 600M-instruction session and so
the next thing a short session (a boot, an attract loop) would notice.

### Not done, and next

- **Translation cost**: 40 µs a word for the MB86235 (VLIW words, each
  lowered with its budget exit: about 500 bytes of code a word), 20 µs for
  the MB86234. The geometrizer's first frames used to translate on its
  thread (slowest frame 210 ms against 16 ms interpreted); the workers
  hide that now ("Translation off the game's thread"), but a smaller IR
  per word (one shared budget exit instead of one per word) would still
  halve the work.
- **Block linking and longer copro slices**: every block returns to the
  dispatcher, and the 2C copro's runs are two words long by its schedule,
  so the dispatcher is much of its cost. Running the copro ahead in
  longer slices and catching up at the i960's accesses would be exact only
  if everything that can see it (its FIFOs and status, buffer RAM, the
  geometrizer's list) synchronised first; not attempted.
- **Delay-slot starts**: a run that ends between a branch and its delay
  slot resumes on the interpreter for one word; a block that starts in a
  delay slot would take those too.

### The SHARC (Model 2B), in progress

`jit::adsp2106x::SharcJit`, Model 2B's copro (`model2::sharc::Copro`) and
geometrizer (`Engine::process_sharc`, `--geo chip`) through
`model2::copro_jit::SharcJitSlot`; on with the other copro JITs (`--jit`,
the player's default), off with `--jit-copro off`, beside the interpreter
with `--jit-copro check` (`make jit-copro-lockstep`). It takes the place of
recompiled SHARC code, as the other copro JITs do.

**Shape: pipeline blocks.** The static recompiler runs a SHARC program's
straight-line words as blocks (docs/recompiler.md, "Blocks"): runs of
words that can be in one (`codegen::in_block`) become a region entered at
any of its words, run as one block when `Sharc::block_start` allows it
(the pipeline the plain sequence, no fetch reaching a loop's end,
interrupts and the timer off), the pipeline, counters and latches brought
up to date at its end (`block_end`). Milestones 1 and 2 were that, made at
run time. Since milestone 3 the JIT's blocks also hold the sequencer:
`SharcJit::run` has `Sharc::run_bus`'s contract; a region is a run of words
a block can hold (`emit::capable`: every word `in_block` takes, and jumps,
calls, returns, DOs, the loop and PC stack operations, CJUMP, BIT on
ASTAT and the user status registers), one Cranelift function entered at
the word in the execute slot, which runs each cycle as `Sharc::step` does:
the word, the fetch stage with the loop logic, the slots moved on, a
branch's redirect (delayed, or the two words fetched squashed into
bubbles, which it runs too), the interrupt hold-off. It goes on to
whatever word the pipeline brings to the execute slot while that is in the
region, so loops run round inside one block, and a block that leaves at a
word outside its region hands the pipeline straight to the region that
holds it (one block for the bus, the core brought up to date once).
`Sharc::jit_enter` says when that is exact (interrupts off, so none can be
taken; the timer off, so a tick only latches; no MODE1 write pending; not
inside a short conditional loop, where the board's `spinning` could stop
the core) and `jit_exit` writes the pipeline, counters and latches back.
The sequencer's stacks and loop logic stay the interpreter's (`jit_fetch`
for the fetch of a loop's end, `jit_do`, `jit_push_pc`, `jit_pop_pc`,
`jit_stacks`, `jit_abort_loop`); a word that would overflow or empty a
stack, a DO of a one-word loop (whose pipeline is rebuilt), RTI, a jump to
itself on a FLAG input (where the board stops stepping), and the status
stack's operations leave the block for the interpreter's single step.

Most of the time the pipeline is the plain sequence (the word in the
execute slot, the next two after it, no slot looped back, no hold-off), so
each word has plain code, whose fetch stage is a compare (two words on
isn't the innermost loop's end), and the words the pipeline reaches
otherwise (a branch's two delay slots, a loop's last two words, the
sequencer's own) also code for the pipeline in variables; a word without
that code leaves the block when it isn't plain. So the timing is the
interpreter's step for step: `make jit-copro-lockstep` compares every run.

**The program, and code it writes.** A JIT follows the program its core
holds, named by `code_hash` over the discovered words (so a state load,
or another upload of the same code with other data, keeps its
translations; another program starts again). The words of every region
translated are watched (`Sharc::watch_words`, the recompiler's
`watch_code` a region at a time): a write to one makes `jit_enter`
decline and the JIT drop that region (`Sharc::take_overwritten`), which
is planned again from the new words when it next runs. A state load or
an upload stops the watch, and run-ahead's copy of a core takes the other
core's: the JIT sets its own up again before its next run.

**Lockstep.** The SHARCs' external buses have state the interpreter can't
be given a copy of cheaply (the copro's FIFOs and buffer RAM, the
geometrizer's list DMA and polygon RAM), so `--jit-copro check` runs the
JIT first, on the board's bus, logging each read and the FLAG inputs the
bus set after each instruction it was asked about, then the interpreter
on a replay of that log (the same reads in the same order, its writes
compared with the JIT's, the FLAG inputs as the bus set them, stopping at
the step the JIT stopped at), and compares the two cores after every run:
`digest` (registers, stacks, pipeline, counters) every run, internal
memory, execution counts and IOP registers every 64th and at every stop.

**Milestones** (each exact before the next):

1. Plumbing (done): regions whose words each call the interpreter's
   `block_exec` (`h_exec`), the dispatcher, both SHARCs, the hot-block
   list (`adsp2106x` in `hot.txt`), the lockstep, random-program tests
   (`make jit-test`: random words of every type, registers, DAGs, modes
   and external memory, runs of 1 to 256 words, compared in full after
   every run).
2. The common compute and data-move forms lowered to IR (done; below).
3. Branches, the two delay slots, loops, the PC and loop stacks in
   blocks (pipeline blocks, above), inline internal writes, message
   registers, PX moves and BITREV (done).
4. More random-program coverage: float operands that round exactly
   halfway, near overflow and underflow, denormals, a third of the
   computes float adds and multiplies (done); the status stack and
   one-word loops in blocks still to come.
5. A cheaper word (done; "The cost of a word", below): fewer values live
   across a region, STKY and the fetch stage's status noted only by the
   words that need them, regions translated for their MODE1 and SYSCON,
   the compares, the float sign operations and the bit operations in IR.

**What the programs run** (`REMODEL_SHARC_PROFILE=1 remodel run` prints
it: each executed word by type and compute, from the execution counts;
`zerogun`, 200M i960 instructions): the geometrizer's words are 99.8%
ones a block can hold (NOPs 17%, register loads 14%, loads with a
multiply-add 14%, loads with a multiply 8%, stores with an increment 7%,
shifts, stores); the copro's 90% (conditional jumps 8%, multiplies,
loads, immediate loads, adds; returns, BIT, calls and DOs are single
steps).

**The lowering** (`adsp2106x/emit.rs`). A word does what `block_exec`
does, in its order: each transfer's address (under its condition)
classified first, leaving before the word for an IOP register (but the
message registers, plain storage) or a reserved address as `block_exec`
declines it; STKY noted for the latch at the end; the word's effects;
`Bus::after_access` after an external access; leaving after the word if a
write reached watched code. Within the word the effects keep `exec`'s
visible order: a transfer's stored register and a register move's source
are read before the compute, the compute's results land before a load's
(so the load wins), DM after PM. In IR: conditions (on ASTAT, and the FLAG
inputs as `Sharc::astat` shows them; BM and the loop counter through
`Sharc::cond`); the ALU's fixed-point add, subtract, increment, decrement,
pass and logic (with ALUSAT, the carry, overflow and AOS); COMP and FCOMP
(with the compare accumulation), FABS, -F and PASS F (NaN operands through
the interpreter's operation); FADD, FSUB and
FMUL on `float::single`'s path (RND32, two normal inputs, an add's
exponents at most 28 apart: the exact f64 result rounded to 23 fraction
bits in place, to nearest even or toward zero), and off it a zero operand
(exponent field 0: zero or a denormal) as the exact path treats it, else
the interpreter's own operation; the multifunction multiply with an ALU
operation; type 6's LSHIFT, ASHIFT, BSET, BCLR, BTGL and BTST; register-file, DAG and PX register
moves and immediates; the DAGs (pre- and post-modify, M registers and
immediates, circular buffers with I7/I15's wrap flags and interrupt
latches, BR0/BR8 bit reversal, BITREV); 32-bit reads and writes of
internal memory (a write calls `invalidate` only where `Sharc::jit_cached`
says a word it touches has something decoded or watched); the message
registers; branch targets (absolute, relative, indirect with BR8). The
other compute operations go through the interpreter's operation alone
(`jit_alu`, `jit_mul`, `jit_shift_imm`, `jit_compute`), other memory
through its `load`/`store` (`jit_read`, `jit_write`), and the words the
lowering doesn't have (PX transfers with memory, BIT) through `block_exec`
(`h_exec`). Regions are compiled with Cranelift's optimiser (the i960's
and the other copros' aren't): a word is many IR instructions and a region
runs many times; it took the geometrizer from 14.6 to 12.2 s of
`zerogun`'s session (milestone 2) for 0.1 s more translation on the
workers. A region is at most 256 words; the geometrizer programs are 6 to
18 regions, translated in 0.4-1.3 s on the workers (the slowest region
0.15-0.2 s), the copro programs' 50-335 in 0.3-1.8 s.

**The cost of a word** (milestone 5). Each word was being paid for in
values the register allocator had to keep: the pipeline's variables, the
counters, the budget, the core, the context and the bus pointers live
across a region of hundreds of blocks, each a target of the dispatch's
jump table, with helper calls everywhere that clobber the caller-saved
registers, so nearly all of them lived in stack slots, reloaded and stored
around every word. Keeping more of the core in variables (the register
file, the DAGs, ASTAT in parts, STKY: a cache written back at the exit and
around the helpers that read or write it) was tried and measured first,
and it was slower (`zerogun` 15.2 s against 12.4): with every word block a
merge point, all ~45 cached values are live everywhere, Cranelift's
allocator spills them all, and the merges add stack-to-stack moves. Calling
the helpers through `preserve_all` trampolines (so no call clobbers
anything) and keeping the core in the pinned register didn't change that
(14.8 and 11.9 s): the allocator still spilled the long-lived values. So
the state stays in the core, and milestone 5 makes each word need less:

- fewer values live across a region: the budget is counted down in one
  variable (a word that writes watched code makes it 1, the rest noted in
  `Ctx::cut`); a bubble's cycle is counted in `Ctx::cycles`; the loop end,
  the short-loop flag, MODE1 and the bus are read where they are used (the
  helpers that need the bus find it in `Ctx::bus`);
- STKY at the start of the cycle (for the interrupt latch at the end,
  which only looks at the exception sticky bits) is noted only by the
  words that can set one (the fixed-point adds, the interpreter's
  operations, `block_exec`), with `Ctx::st_pc` saying which word noted it;
  for any other last word STKY as it stands is the same;
- ASTAT and the FLAG inputs at the start of the cycle (the fetch stage's
  loop test at a loop's end) are noted only by a word that can change them
  (ASTAT: a compute; the FLAG inputs: an external access, whose
  `after_access` may set them); for the others they are read after the
  word, on the (cold) way to the loop logic;
- a region is translated for the MODE1 and SYSCON IMDW bits the core has
  (`emit::Mode`), which no word of a block changes: rounding, truncation,
  saturation and bit reversal are constants and an inline access tests no
  IMDW bit; `SharcJit::one` runs a region only while the core's modes are
  its own, and drops one whose aren't, to translate it again (counted
  with the translations thrown away: none in the bench sessions);
- the compares, the float sign operations and the type 6 bit operations
  (the copro's commonest words on `jit_alu` and `jit_shift_imm`) in IR.

`REMODEL_JIT_DISASM=DIR` writes the machine code of everything the JITs
translate to DIR (Cranelift's listing, after register allocation): how
the above was found. The code is 20-25% smaller (`zerogun`'s geometrizer
1018 to 778 KB, its copro 1175 to 976 KB).

**Results** (`make bench`'s session, 600M i960 instructions, coin start,
host seconds and the geometrizer thread's, which the run waits for;
`--jit` before is the i960's JIT alone, the SHARCs interpreted; milestones
3 and 5 the faster of runs alternating on the same machine, which was
busy, so within about 0.3 s):

| | `zerogun` | `pltkids` | `gunblade` | `fvipers` | `schamp` |
|---|---|---|---|---|---|
| interpreter | 40.6 (37.0) | 59.8 (57.9) | 85.8 (84.3) | 90.4 (85.7) | 88.0 (85.7) |
| `--jit`, before | 38.2 (36.2) | 58.7 (57.2) | | | |
| milestone 1 (regions on `h_exec`) | 29.4 (27.4) | 44.0 (42.5) | | | |
| milestone 2 (the lowering) | 14.2 (12.2) | 21.3 (19.9) | 29.2 (27.4) | 29.5 (25.2) | 30.4 (27.5) |
| milestone 3 (pipeline blocks) | 12.2 (10.4) | 17.5 (16.5) | 26.2 (25.2) | 27.3 (24.0) | 27.1 (25.7) |
| **milestone 5** (a cheaper word) | **10.9 (9.2)** | **15.4 (14.4)** | **24.7 (23.8)** | **24.7 (21.5)** | **24.7 (22.8)** |

That is 6.6 times real time for `zerogun` (1.77 interpreted), 4.7 for
`pltkids` (1.20), 2.9 for `gunblade`, `fvipers` and `schamp`
(0.80-0.84): 3.5-3.9 times the interpreter, and 6-12% faster than
milestone 3 (the geometrizer's thread 6-13%). (Milestone 2's `gunblade`,
`fvipers` and `schamp` were measured with another build running; the
differences there are within that noise.) Words left to the interpreter's
single step (`make bench JIT=1`'s `jit copro` lines, words outside blocks
against all the core ran): the geometrizer's 0.07-0.26% (3.6-5.8% at
milestone 2), the copro's 4.8% (`gunblade`), 9.0 (`pltkids`), 9.9
(`zerogun`), 26 (`fvipers`), 28 (`schamp`), from 27-58%: what is left is
mostly the copro's wait for its FIFO (a one-word `DO UNTIL NOT FLAG0_IN`
of a NOP, which the board stops stepping as `spinning`), and the first
runs of a region still being translated. Of the words in blocks, 0.1%
(the geometrizer) and 2-3% (the copro: BIT, PX transfers) run on
`block_exec`.

What the time goes on is still mostly the translated code itself: of
the game's and the geometrizer's threads' samples (gdb under `SIGUSR2`,
`zerogun`), translated code was 68% at milestone 3 and is 60% now (the
SHARCs' regions 33 and 31% from the geometrizer, 10 and 7.5% from the
copro, the rest the i960's). For `fvipers` and `schamp` the geometrizer's
words are the same kinds as `zerogun`'s (loads with a multiply-add,
loads, multiplies, stores with an increment, shifts with a move).

**Exactness.** `make jit-copro-lockstep` (300M) is identical on
`overrevb`, `zerogun`, `gunblade` and `fvipers` at every milestone
(3.0-8.6 million copro runs and about 4,000 geometrizer runs compared
each); `make bench-all` and `make bench-all JIT=1` give all 22 hashes. The
random-program tests (`make jit-test`; a debug build in `make check`)
broke at once on a deliberate error in the float add, the inline read and
(since milestone 4's operands) the rounding of exact ties, which they had
missed before; at milestone 5, on errors in FNEG's AN, the compare
accumulation, COMP's AZ, BTST's SZ, RND32's operand masking, the STKY
note and the ASTAT a loop test sees. (The compares and bit operations
needed operands of their own to be caught: a third of the computes now
include them, half of type 6's words the bit operations.)

**Not done, and next**

- The status stack (PUSH/POP STS) and one-word loops in blocks; RTI and
  interrupts (blocks need interrupts off, as the 2B programs mostly run).
  POP STS sets MODE1 at once (register files swapped, interrupts perhaps
  enabled and taken in the same step), so it would end a block.
- Keeping core state in registers needs a region shape the allocator can
  work with: plain code entered at fewer points (so that a run of words is
  straight-line code, its values in registers and written back at its
  exits), not every word a target of the dispatch. Regions of 1,024 words
  instead of 256 (fewer exits to another region: `zerogun`'s geometrizer
  leaves for another region about every 15 words) were no faster, and
  took 0.85 s to translate.
- FFLOAT, FFIX, the fixed-point multiplies and the shifter's field
  operations still go through the interpreter's operation.

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
| i960 native JIT, finished | floats translated (the largest fallback group in both games: `mulr`, `addr`, `subr`, `movr`, `cvtir`, `cmpr`); code memory reclaimed; `jit-lockstep` over every game. Done: plain memory inline (reads and writes, with the bus's write counts and code counters), the step inline while the machine is quiet and the interrupt check one compare, translation on workers with the hot-block list | a week |
| Copro JITs | MB86234 and MB86235: done ("Copro JITs"). ADSP-21062 (2B copro and geometrizer): in progress ("The SHARC"): pipeline blocks on Cranelift (branches, delay slots, loops, the PC and loop stacks), the compute, data-move and memory lowering, lockstep and random tests done (milestones 1-4), a cheaper word (milestone 5); the status stack in blocks left | a few days left |
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

1. a copro back end (or native copro programs, as the geometrizers have):
   done for the MB86234 and MB86235 ("Copro JITs"), a further 1.2-1.3x
   (2.4x with the chip geometrizer), so that the JIT build is 1.66-1.8x
   the interpreter and ahead of the recompiled build on every set
   measured; the SHARC is under way ("The SHARC"),
2. an inline fast path for plain memory and a cheaper per-instruction
   step (interrupt check, timers): done ("Memory and the step inline"),
   a further 1.3-1.45x where the i960 is the limit (`topskatr` native,
   `srallyc`, `vf2`, `dynamcop`),
3. floats in the i960 lowering (the largest fallback group; small in
   time), and the full step's own cost (asking how long the machine stays
   quiet, 3-4%).

The WebAssembly back end is worth doing on the same front end once the
native one has items 1 and 2, since the browser's interpreters are slower
still and the same shares apply.
