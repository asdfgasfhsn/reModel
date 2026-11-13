# Architecture: layout and reuse across Model 2

## Layout

```
crates/
  i960/      i960KB decoder and reference interpreter (CPU-generic)
  romset/    ROM manifests, verification, assembly (Model 2-generic)
    src/games/  one module per supported game
  analyze/   recursive-descent code discovery      (CPU-generic)
  model2/    board model (Model 2, 2A, 2B and 2C): memory map, IRQs, timers, I/O,
             copro and geometrizer interfaces
  mb86234/   MB86234 (Model 2A TGP) interpreter and disassembler
  mb86235/   MB86235 (TGPx4) decoder, disassembler and interpreter
  adsp2106x/ ADSP-21062 (SHARC) decoder, disassembler and interpreter
             (Model 2B's geometry DSPs; generic)
  m68k/      68000 interpreter and disassembler (sound CPUs)
  z80/       Z80 interpreter (DSB1 music board CPU)   (generic)
  sound/     SCSP (YMF292) with its DSP, and the sound board; the original
             Model 2's board: MultiPCM, OPN2 (Model 2-generic)
  mpeg/      MPEG-1 Audio Layer II decoder          (generic)
  dsb/       DSB2 and DSB1 MPEG music boards: 68000 or Z80, UART,
             decoder chip model, replacement soundtracks (WAV/MP2
             reader, resampler)
  png/       PNG decode/encode with zlib inflate/deflate (generic)
  state/     save states: a small binary serialisation   (generic)
  gpu/       3D rasteriser on the GPU (wgpu) and the 2D layers' composition
  recomp/    registry of recompiled programs (a stub; `make recomp` builds
             the real one in out/recomp/)
  cli/       the `remodel` command
  play/      the native player, `remodel-play`
  web/       the browser build (WebAssembly and WebGPU, `make web`)
tools/
  asset-review.py   distinct textures, screens and contact sheets from capture runs (`make assets`)
  notices.py        THIRD-PARTY-NOTICES.txt for `make dist` and `make web`
  web.sh            the browser build (`make web`)
```

## Reuse across Model 2

All Model 2 boards share the i960KB main CPU, and all but the original
board the 68000/SCSP sound system. The geometry processor is what differs between revisions:

| Component | Model 2 | 2A | 2B | 2C |
|---|---|---|---|---|
| i960 decoder, interpreter, recompiler | shared | shared | shared | shared |
| ROM loading, code analysis | shared | shared | shared | shared |
| Sound (68000 + SCSP) | no: 68000 with two MultiPCMs and an OPN2 FM chip (`sound::PcmBoard`, `docs/hw/model2.md` §5, `docs/hw/multipcm.md`) | shared | shared | shared |
| Geometry processor | Fujitsu TGP (MB86234), program uploaded | as Model 2; fixed-function geometrizer | two ADSP-21062 SHARCs (copro, geometrizer), programs uploaded; core in `crates/adsp2106x`, board in `crates/model2` | Fujitsu TGP (MB86235) |
| Renderer | shared | shared | shared | shared |

Supporting a new game should mean adding a manifest under
`crates/romset/src/games/`, plus whatever per-game patches turn out to be
needed. A manifest holds the chips, the game's controls (input names,
ports and analog channels, light guns with where the board reads them and
their calibration, default keys and pad bindings, and its coin-start
script) and which drive board it needs, if any; the board model, scripted
input and both players read them from there. Drive board protocols live
in `crates/model2/src/drive.rs`, one per kind; the 2C gun interface in
`crates/model2/src/gun.rs`. `topskatr`, `skisuprg`, `stcc`, `segawski`,
`waverunr`, `overrev`, `hotd` and `bel` are Model 2C-CRX (MB86235 geometry
processor); `overrevb`, `sgt24h`, `dynabb97`, `doa`, `indy500`, `fvipers`,
`schamp`, `vstriker` and `lastbrnx` are Model 2B-CRX (ADSP-21062 SHARCs);
`srallyc` is Model 2A-CRX (MB86234 TGP); `daytona` (in progress) the original
Model 2, on the 2A board model with its differences (`Board::Model2`). The manifest's
`board` picks the board model: `crates/model2` keeps what differs between
the boards (the copro and geometrizer cores, the 2B host windows, the UART
address) behind it, with nothing keyed on a game's name. A manifest's
`parent` marks a clone: its chips may sit in the parent set's zip or
folder, and it shares the parent's poster and bezel.

A manifest's `idle_loops` lists the poll loops the game waits for the
next frame in; the board model fast-forwards through them exactly, with
every pass checked as it runs (`crates/model2/src/idle.rs`,
`docs/idle-skip.md`), so the mechanism stays generic and only the
addresses are per game.

The board table is from public documentation (largely MAME's driver notes).
MAME does not run `topskatr` correctly, so its board details are treated as
**unverified hypotheses**: the game's own code is the ground truth, and the
board model logs every access it doesn't fully understand. Places where
MAME is demonstrably wrong are tracked in `docs/mame-defects.md`, written up
so they can become upstream patches.

