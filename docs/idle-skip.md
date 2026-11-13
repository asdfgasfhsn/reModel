# Idle-loop skipping

Every game spends most of each frame waiting: its main program finishes
the frame's work and then polls, in a loop of two to six instructions,
until vblank or an interrupt handler says the next frame may start. On
the interpreter those polls were 72 to 87% of the i960's instructions in
`make run-ahead-profile` (`docs/play.md`, "Run-ahead cost"). reModel
fast-forwards through them, exactly: a skipped poll leaves the machine
as running it would have, to the bit, so every game's state hash is the
same with skipping on or off.

The mechanism is in `crates/model2/src/idle.rs` (with the bus's half in
`bus.rs`); the loops are per game, in the manifests.

## Which loops

A game opts in by listing its loops in its manifest
(`romset::Game::idle_loops`): each `IdleLoop` is the address the loop
branches back to (`start`) and the address just past its backward branch
(`end`). Each was found as the hottest code in a coin-start run (`make run`'s
"hot spots") and read in the game's code; its game's page in
`docs/games/` says what it reads and what ends it. An entry can't make a
game run differently: everything below is checked as it runs, so a wrong
entry only costs the time of checking it.

## What is checked

When the i960 reaches a listed loop's start (`Machine::run_slice`, at an
instruction boundary, with no interrupt due), the machine runs one pass of
the loop as usual while the bus watches every access (a probe). Then it
credits more passes only if all of these hold:

1. **No event during the pass, none during the passes credited.** The
   bus says how many instructions from now retire with nothing happening
   but time passing (`Model2Bus::quiet_retires`): before vblank, before
   any running timer reaches its count, before the sound UART's transmit
   interrupt would be raised. Those are the only things an instruction's
   retire does besides advancing time and running the copro
   (`retire_insn`), and the only sources of interrupts. The pass must fit
   in that span, and the passes credited end inside it.
2. **The pass wrote nothing,** and read only what can't change in that
   span: memory that only the i960 writes (everything mapped but buffer
   RAM), and the registers that change only at those events or by the
   i960's own writes: the video status at `0x98000C` (its frame bit
   toggles at vblank), the interrupt request and enable words at
   `0xE80000`, copro control and status (`0x980000`, `0x980014`) and the
   board control byte (`0xF80000`). Buffer RAM and the copro's
   output-empty flag (`0x980004`) count only while the copro is idle
   (no program, stalled on its input FIFO, or the 2B SHARC spinning on a
   flag), since a running copro writes them. Anything else (timer counts,
   inputs, FIFO data, unmapped or protection addresses) refuses the pass.
3. **The CPU is back where it was.** At the end of the pass the IP is
   the loop's start again and the CPU's whole state (registers, floating
   point registers, AC, PC, TC, the pending interrupts, the cached
   register sets; `i960::cpu::ArchState`) equals what it was at the
   start, except for **counters**: registers that the loop's code only
   ever adds a literal to with `addo`/`subo` into themselves, and that no
   other instruction in the loop names (`idle::counters` decodes the
   loop and checks this; a loop with counters must also be made only of
   plain single-register instructions: loads, ALU operations, compares
   and branches, no calls, stores, multi-register moves or floating
   point). Several of the games count their polls this way (`addo g0,
   1, g0`); nothing in the loop reads the count, so it can be credited.

With those, every further pass does exactly what the probe's did: the
same state, the same reads returning the same values, the same branch
taken. So the machine credits `k` whole passes of `p` instructions, as
many as fit both before the span ends and in what the caller allowed
(`run_slice`'s `max`):

- the instruction count goes up by `k * p`, and time by `k * p` times
  the cycles per instruction;
- each counter gets `k` times what one pass added (wrapping, as `addo`
  does);
- the copro runs on as `k * p` retires would have run it, in a batch: the
  2C MB86235 for twice that many words (it stalls only on an empty input
  FIFO, which nothing fills meanwhile, so a long run stops where single
  ones do), the 2B SHARC copro through `Copro::run_calls`, which runs it
  as `k * p` calls of five steps would (one long run while it runs; if it
  stops early the calls left do nothing when it spins on a flag, and
  tick it once each when it went IDLE, so those are run one by one);
- diagnostics follow: the per-frame hot-spot profile counts each of the
  pass's addresses `k` more times, and the I/O access log counts the
  pass's register reads `k` more times.

What's left of the way to the event (less than one pass more) runs
normally, so a slice can end, and the event lands, in the middle of a
pass exactly where stepping puts it. Then the interrupt or vblank is taken
as usual and the loop is probed again when the game next comes back to it.

A pass that fails a check is simply run, as are passes too close to the
next event to be worth probing (fewer than three passes' worth left).
After two failed probes in a row a loop isn't probed again for 4,096
instructions. None of this is machine state: save states and run-ahead's
copies hold none of it, and nothing is ever half skipped between two
calls, so saving, loading and run-ahead behave the same with skipping on
or off (and a state saved with it on loads with it off).

## Where it runs

Everywhere `Machine::run_slice` runs: the `remodel` CLI, the player, the
launcher's previews, run-ahead's shadow machine (which skips if the real
one does: `Machine::configure_like`) and the browser build. Interpreted
or recompiled: with `RECOMP=1` the probe pass runs recompiled blocks (a
loop is one or two blocks; the dispatcher sees its start between them),
and the bus watches their accesses the same way. *Confirmed* on
`topskatr`, `vf2` and `lastbrnx` (2026-11-13): `make bench RECOMP=1` gives
the interpreter's hashes with the same instructions skipped (topskatr
400275251269648e 82.0%, vf2 23a2a5094a998773 60.6%, lastbrnx
c3ec82727bacd608 74.4%); recompiled with the skip, lastbrnx runs 600M
instructions in 23.9 s (3.0x real time) against 87.7 s interpreted. `srallyc`
runs its code from work RAM, so its loop is always interpreted.

## The off switch

`--no-idle-skip` on `remodel` (any command) and `remodel-play` runs every
pass instead (`model2::idle::set_default`; `Machine::set_idle_skip` per
machine). `remodel run` (and `make bench`) prints a line saying what was
skipped:

```
idle skip     513881810 instructions (85.6%) in 35981 skips, 1 loop; 4148 of 40129 probes refused
```

or `idle skip     off`, or `none` for a game with no loops listed.

## Checking it

- Unit tests (`make test`): `idle::tests` runs small poll loops (a
  vblank-toggle poll with a counter, a RAM byte poll, one that stores)
  on 2A, 2B and 2C machines with timers and vblank posting interrupts,
  skipping and not, in slices of 1 to 30,011 instructions and whole
  frames, and compares the whole saved state after every slice;
  `sharc::tests::batched_runs_are_single_runs` checks the SHARC batch
  against single calls (running, spinning part way through a call, and
  IDLE).
- `make bench GAME=g` for every game: the state hash must be what it was
  before (and what `--no-idle-skip` gives).

## Results

`make bench GAME=g` (600M instructions, a coin-start run into play, the
interpreted i960 and the geometrizer on the chip, on its thread; 16-core
desktop, 2026-11-13), before the skip existed and with it. Every state
hash is unchanged. Most refused probes are the first pass after the
loop is entered, which starts with registers from before the loop; the
second pass is the fixed point.

| Game | State hash (before = after) | Skipped | Host time before | after | Speed-up |
|---|---|---|---|---|---|
| `topskatr` | `400275251269648e` (same) | 82.0% | 39.7 s (1.81x) | 21.1 s (3.42x) | 1.88x |
| `skisuprg` | `188a5a44fb547532` (same) | 86.5% | 36.3 s (1.98x) | 26.8 s (2.69x) | 1.36x |
| `stcc` | `b42ea11201ac75c4` (same) | 74.6% | 31.5 s (2.29x) | 16.8 s (4.30x) | 1.88x |
| `segawski` | `505a972b9e8a3c89` (same) | 78.7% | 35.7 s (2.02x) | 27.6 s (2.61x) | 1.29x |
| `waverunr` | `97d498bed77503a6` (same) | 71.9% | 44.7 s (1.61x) | 33.6 s (2.14x) | 1.33x |
| `overrev` | `d41686f041683f4c` (same) | 79.7% | 32.8 s (2.20x) | 19.1 s (3.78x) | 1.72x |
| `hotd` | `38c08a3e3591af45` (same) | 76.4% | 31.9 s (2.25x) | 25.9 s (2.78x) | 1.24x |
| `bel` | `a634c029241973eb` (same) | 65.5% | 27.8 s (2.59x) | 25.0 s (2.88x) | 1.11x |
| `overrevb` | `ee2bcd89e419e428` (same) | 79.6% | 54.3 s (1.33x) | 47.5 s (1.52x) | 1.14x |
| `sgt24h` | `282c8d6fd5151c21` (same) | 56.1% | 76.2 s (0.95x) | 72.3 s (1.00x) | 1.05x |
| `dynabb97` | `69e50b78363de101` (same) | off | 67.7 s (1.06x) | 70.3 s (1.02x) | 0.96x |
| `doa` | `78f308b4da95cefb` (same) | 78.9% | 84.0 s (0.86x) | 80.8 s (0.89x) | 1.04x |
| `indy500` | `4a1b197bf00149d6` (same) | 71.7% | 61.2 s (1.18x) | 58.5 s (1.23x) | 1.05x |
| `fvipers` | `f2da97d89774d706` (same) | 60.3% | 87.0 s (0.83x) | 87.2 s (0.83x) | 1.00x |
| `schamp` | `0dd9ca5f3388771f` (same) | 65.7% | 88.7 s (0.81x) | 90.1 s (0.80x) | 0.98x |
| `vstriker` | `a4af0c62acc656f3` (same) | 51.3% | 101.5 s (0.71x) | 98.6 s (0.73x) | 1.03x |
| `lastbrnx` | `c3ec82727bacd608` (same) | 74.4% | 90.5 s (0.80x) | 90.5 s (0.80x) | 1.00x |
| `srallyc` | `45fbec5e9d1215cf` (same) | 85.6% | 31.1 s (2.31x) | 7.0 s (10.36x) | 4.48x |
| `vf2` | `23a2a5094a998773` (same) | 60.6% | 25.9 s (2.78x) | 14.8 s (4.87x) | 1.75x |

The 2B games gain least in this run because the geometrizer's chip
program (the SHARC on its thread) is the limit there: the i960 side
waits for it (`waited for` in the report), whatever the skip saves.
With the native geometrizer, as the player and the browser run, the
i960 side is the limit, and `lastbrnx` goes from 13.8 to 8.6 ms a frame
(`docs/play.md`, "Run-ahead cost"). `dynabb97` has no loop listed
(`docs/games/dynabb97.md`). Speed figures vary by about 10% from run to
run.

Longer runs agree too: 3.5G-instruction coin-start runs give the same state hash with the skip on and
off for `srallyc` (`5e5b61ae015148e9`, 46.5 s against 188.0 s),
`topskatr` (`964ed5b62a19c876`, 187 s against 218 s) and `indy500`
(`9b92c79e79c7dc49`, 401 s against 421 s).
