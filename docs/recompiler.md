# Static recompilation

Remodel can turn the game's own code into Rust and compile it into the
binary: the i960 main program and the MB86235 programs uploaded to the
copro and geometrizer. Nothing is reimplemented; the original
instructions run, just translated ahead of time instead of decoded at run
time.

```sh
make run STEPS=900000000 INPUT=coin1@1100-1104,start@1250-1254   # trace what runs
make recomp               # generate out/recomp/*.rs and build target/recomp
make bench RECOMP=1       # same run on the recompiled build: same state hash
target/recomp/release/remodel --interp ...   # recompiled build, interpreter for the i960
```

The generated code is derived from the ROMs, so it lives in `out/recomp/`
and is never committed. Ordinary builds (and `make check`) leave it out.
`build.rs` in `crates/model2` picks up every `i960_*.rs` and `tgp_*.rs` there
when `REMODEL_RECOMP_DIR` is set and registers them by program hash. A
build whose registry has no entry for a ROM image runs it on the
interpreters, so the same binary still runs other games and revisions.

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
isn't reached by falling through. Each block becomes one Rust function, and
the machine keeps a table of them indexed by address. Execution enters at
any block start. Anywhere else, such as the point an interrupt returns to,
runs on the interpreter until it reaches one.

**Instructions.** Each instruction is either translated into plain Rust on
the register file (`c.r[17] = c.r[16].wrapping_add(0x4);`) or, for the rare
and floating-point ones, a call to the interpreter's `Cpu::exec` with the
decoded instruction as a constant. In `topskatr`, 96% of the 53,000
instructions are translated. Compares, condition tests, `call` and `ret`
use the interpreter's own helpers, so frame and flag behaviour can't
drift.

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
registers or flags differ, printing the block and the registers involved.

## MB86235 (`crates/mb86235/src/codegen.rs`)

Each program word the interpreter executed becomes a match arm calling
`Tgp::exec` with that word's decoding as a constant. `exec` is
`#[inline(always)]`, so the compiler specialises each arm. Words that
never ran fall back to the interpreter. The programs are uploaded by the
game at boot, so they are keyed by a hash of the uploaded image.
