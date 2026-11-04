# Remodel

**Static recompilation toolkit for Sega Model 2 arcade games.**

Remodel turns the program code of a Model 2 game into native code that runs
on a modern PC, with the custom hardware (geometry processor, renderer, sound)
replaced by high-level reimplementations. The first target is *Top Skater*
(`topskatr`); everything not specific to one game is built to be reused across
the Model 2 family.

> **Bring your own ROMs.** Remodel contains no game code or data. It reads a
> dump you supply at runtime and checks it against known-good hashes. Never
> commit ROM files or generated listings: `rom/` and `out/` are git-ignored.

## Status

Early tooling. The original program boots on the reference interpreter;
nothing renders yet.

| Stage | State |
|---|---|
| ROM set loading and verification | done |
| i960 disassembler | done |
| Static code discovery | done (~28% of `topskatr` program ROM mapped) |
| i960 interpreter (reference and tracer) | done; boots `topskatr` into its interrupt-driven init |
| Model 2C board model (memory, IRQs, timers, I/O, copro interface) | in progress |
| Recompiler (i960 → C) | planned |
| Geometry processor (high-level reimplementation) | planned |
| Renderer (OpenGL/Vulkan) | planned |
| Sound (68000 + SCSP) and input | planned |

## Quick start

Requires a Rust toolchain (stable) and `make`.

```sh
make                  # list every target
make check            # format check, clippy, unit tests (no ROMs needed)

# With an unpacked dump in ./rom:
make verify           # check every chip
make boot             # boot record and system-table entry points
make analyze          # code discovery -> out/topskatr/program.lst
make disasm ADDR=0x6f0 COUNT=64
make run STEPS=50000000   # boot on the interpreter, report hardware use
```

Variables: `GAME` (manifest id, default `topskatr`), `ROM` (chip directory,
default `rom`), `OUT`, `ADDR`, `COUNT`, `STEPS`. The `Makefile` is the supported
interface for humans and agents alike; `make help` is always current.

## Layout

```
crates/
  i960/      i960KB decoder and reference interpreter (CPU-generic)
  romset/    ROM manifests, verification, assembly (Model 2-generic)
    src/games/  one module per supported game
  analyze/   recursive-descent code discovery      (CPU-generic)
  model2/    board model: memory map, IRQs, timers, I/O, copro interface
  cli/       the `remodel` command
```

## Reuse across Model 2

All Model 2 boards share the i960KB main CPU and the 68000/SCSP sound
system. The geometry processor is what differs between revisions:

| Component | Model 2 | 2A | 2B | 2C |
|---|---|---|---|---|
| i960 decoder, interpreter, recompiler | shared | shared | shared | shared |
| ROM loading, code analysis | shared | shared | shared | shared |
| Sound (68000 + SCSP) | shared | shared | shared | shared |
| Geometry processor | Fujitsu TGP | Fujitsu TGP | ADSP-21062 SHARC | Fujitsu TGP (MB86235) |
| Renderer | shared | shared | shared | shared |

Supporting a new game should mean adding a manifest under
`crates/romset/src/games/`, plus whatever per-game patches turn out to be
needed. `topskatr` is Model 2C-CRX (MB86235 geometry processor).

The board table is from public documentation (largely MAME's driver notes).
MAME does not run `topskatr` correctly, so its board details are treated as
**unverified hypotheses**: the game's own code is the ground truth, and the
board model logs every access it doesn't fully understand.

## What's known about `topskatr`

Recorded as it's found, so nobody has to re-derive it. *Confirmed* means the
game's own code demonstrates it; *unverified* means it comes from external
documentation and hasn't been contradicted yet.

- The program ROM is `epr-19755a.15` and `epr-19756a.16`, joined as 16-bit
  halves of each 32-bit word (`.15` = low half), 1 MB at address `0x0`.
- Bootstrap tables at reset: PRCB `0xB0`, first IP `0x5C0`, interrupt table
  `0x5A2B0`, system-procedure table `0x1C0`.
- Reset copies a startup table from ROM `0x580` to `0xE00000`. It copies the
  PRCB to `0x511C40` and the interrupt table to `0x510000`, and `.data` from
  ROM `0xBF198` to RAM `0x500000–0x509FD8`. It then sends a reinitialise
  message (`0x93`) to the i960's control port `0xFF000010` and continues at
  `0x6F0`.
- *Confirmed:* the game programs the interrupt control register
  (`0xFF000004`) to `0x0F0E0D0C`, routing IRQ lines 0-3 to vectors 12-15.
  These are the four vectors with dedicated handlers in its interrupt table.
- *Confirmed:* boot uploads a 4,096-word (64-bit) program to the geometry
  coprocessor through `0x884000`, framed by bit 31 of `0x980000`.
- *Confirmed:* boot times a loop on bit 2 of `0x98000C` and only proceeds
  once that bit changes, consistent with a per-frame toggle.
- *Confirmed:* init enables IRQ mask `0x215` and programs timer 0
  (`0xF00000`) with 50,000 ticks (2 ms at 25 MHz).
- *Unverified:* the board map, IRQ controller semantics (`0xE80000`
  request/ack, `0xE80004` enable), timers and I/O chip layout in
  `crates/model2/src/bus.rs`.
- Regions the code reads and writes directly: work RAM `0x00500000`; a very
  hot port at `0x00800000` (about 4.9k accesses to 26 addresses, likely the
  geometry processor's input); `0x01C00000` and `0x01D00000` (probably I/O);
  `0x01000000` and `0x01800000` (probably tilemap and palette).

## License

MIT for Remodel's own code. Game software and data remain the property of
their respective owners.
