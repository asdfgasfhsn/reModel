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

The original program runs on the reference interpreters from boot through
attract, coin-up, course and skater select into a race, with textured 3D
frames written as PNG snapshots. The interpreters run the bench (attract into
a race) at about 2x real time on a desktop CPU, with the geometrizer on its
own thread. There is no live window or sound yet.

| Stage | State |
|---|---|
| ROM set loading and verification | done |
| i960 disassembler | done |
| Static code discovery | done (~28% of `topskatr` program ROM mapped) |
| i960 interpreter (reference and tracer) | done; boots `topskatr` into its interrupt-driven init |
| Model 2C board model (memory, IRQs, timers, I/O, copro interface) | in progress |
| MB86235 geometry processor: disassembler and reference interpreter | done; runs the game's uploaded program |
| Geometrizer (second MB86235) running the game's own program | done; gameplay frames produce the rasteriser stream |
| Recompiler (i960 → C) | planned |
| Geometry processor fast path (high-level per-command) | planned |
| 2D tile layer renderer (PNG snapshots) | done; boot, logo, title and attract text render |
| 3D rasteriser (software) | textured gameplay frames render (point-sampled, no mipmaps yet) |
| Scripted input (headless) | done; coin and start confirmed, other buttons unverified |
| Speed | interpreters about 2x real time (`make bench`); recompiler not needed to reach full speed |
| Live window and sound (68000 + SCSP) | planned |

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
make run SNAP=60          # also write the tile layer every 60 frames
make run STEPS=900000000 SNAP=115 INPUT=coin1@1100-1104,start@1250-1254
                          # coin up, start, and play into a race
make bench                # timed run into a race: speed and state hash
```

`make bench` prints a hash of all RAM and the last rasteriser stream. A change
meant only to make things faster must leave it unchanged. The hash depends on
`out/GAME/eeprom.bin`, which `run` creates on first boot.

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
  mb86235/   MB86235 (TGPx4) decoder, disassembler and interpreter
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
board model logs every access it doesn't fully understand. Places where
MAME is demonstrably wrong are tracked in `docs/mame-defects.md`, written up
so they can become upstream patches.

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
- *Confirmed:* the game reads coprocessor results at 369 distinct sites.
  With the coprocessor stubbed, it fails after about 27 s on a divide by
  zero. With the MB86235 interpreter running the uploaded program, 300M
  instructions (36 s of game time) run with no faults and no unanswered
  result reads; the coprocessor executes 53M words.
- *Confirmed:* the coprocessor program is a command dispatcher (see
  `docs/hw/mb86235.md` section 6). Hardware notes with sources are in
  `docs/hw/`.
- *Confirmed:* every instruction the interpreter executed was already found
  by static analysis. The trace (`make run`, then `make analyze`) resolves
  the indirect jumps it observes.
- *Confirmed:* with a blank settings EEPROM the game writes 38 words of
  defaults, and accepts them on the next boot.
- *Confirmed:* the 2D tile layer shows the full boot sequence ("Downloading",
  "Initializing"), the publisher logo, the title screen and attract mode,
  whose 3D scenes are not rendered yet. See `docs/hw/tilemap.md` for the
  split-mode finding, which differs from MAME.
- *Confirmed:* coin 1 is IN0 bit 0 and start is IN0 bit 6 (active low). A
  scripted coin and start go through course select and skater select into
  a race: the HUD timer, checkpoints and hints all run, with no 3D drawn yet.
- *Confirmed:* gameplay frames render in 3D with textures from the game's own
  geometrizer output, composited between low- and high-priority tiles. This
  confirms:
  - vertex linking, the 24-bit float stream, and the projection with the sync
    offsets (xoff 0, yoff 128);
  - texture headers and points from the `.27`/`.25` ROM, and the 4bpp 2x2-packed
    texel sheets in texture RAM;
  - the luma tables and the colour translation tables (section 5 of
    `docs/hw/geometrizer.md`).
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
