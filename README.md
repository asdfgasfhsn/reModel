<p align="center"><img src="assets/reModel-logo.png" alt="Remodel" width="640"></p>

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
own thread. The sound board (68000 + SCSP) and the DSB2 MPEG music board
render offline from the command bytes a run captures (`make sound`, `make
music`). There is no live window yet.

| Stage | State |
|---|---|
| ROM set loading and verification | done |
| i960 disassembler | done |
| Static code discovery | done (~28% of `topskatr` program ROM mapped) |
| i960 interpreter (reference and tracer) | done; boots `topskatr` into its interrupt-driven init |
| Model 2C board model (memory, IRQs, timers, I/O, copro interface) | in progress |
| MB86235 geometry processor: disassembler and reference interpreter | done; runs the game's uploaded program |
| Geometrizer (second MB86235) running the game's own program | done; gameplay frames produce the rasteriser stream |
| Recompiler: i960 → native | done; opt-in (`make recomp`, then `RECOMP=1`), identical state hash; 96% of the game's instructions become plain Rust, the rest call the interpreter with the instruction pre-decoded; see `docs/recompiler.md` |
| Recompiler: MB86235 programs → Rust | done; built with the i960 code, geometrizer about 1.5x faster |
| Recompiler: ADSP-21062 (SHARC) programs → Rust | done (Model 2B); built with the i960 code, each word the interpreter's step specialised for its instruction, straight-line runs compiled as blocks with the pipeline and counters brought up to date at their end, interpreter fallback for anything not compiled or overwritten; crates named after their code, so games uploading the same code share them. Geometrizer per race frame: `overrevb` 3.6 ms native, 4.8 ms in WebAssembly (interpreted 16.8 ms); `sgt24h` 6.1 ms native, 8.1 ms in WebAssembly; identical state hashes and `make lockstep`; see `docs/recompiler.md` |
| Native 2B geometrizer (the geometrizer programs' behaviour in Rust, `model2::geo_native`) | done, opt-in (`--geo native`, `remodel-play --geo native`, `?geo=native`): the two programs every 2B game uploads, found by their code hash; raster streams bit-identical to the SHARC's over 5,000-frame coin-start runs of all seven 2B games and a fuzzer (`make geo-check`, `make geo-fuzz`), same state hashes; a race frame of `sgt24h` 1.5 ms against 5.7 recompiled (22.6 interpreted); see `docs/hw/geometrizer-2b.md` |
| 2D tile layer renderer (PNG snapshots) | done; boot, logo, title and attract text render |
| 3D rasteriser (software) | textured, bilinear-filtered gameplay frames (topskatr uses no mipmaps); the reference, but too slow for live races (15-27 ms a frame at native size) |
| 3D rasteriser (GPU, wgpu: Vulkan / Metal / DirectX 12) | off-screen preflight: about 3 ms a frame native and 7 ms at 1080p, matching the software renderer on 99.96% of coverage; see `docs/gpu.md` |
| Scripted input (headless) | done; names, ports and bindings per game in its manifest; every `topskatr` button and both analog channels confirmed |
| Speed | interpreters about 2.2x real time (`make bench`); recompiled build about 4.4x (`make bench RECOMP=1`), where the geometrizer thread becomes the limit |
| Sound board: 68000 + SCSP (slots, EG, LFO, FM, timers, MIDI, DSP) | done offline: replays the captured command stream to a WAV at about 12x real time |
| DSB2 music board (68000 firmware + MPEG Layer II decoder) | done; `make music` renders the game's music from a `run` capture |
| DSB1 music board (`stcc`: Z80 firmware + MPEG Layer II decoder) | done; live in `make play GAME=stcc` and the browser, `make music GAME=stcc` offline; modelled from the Z80 program (`docs/hw/dsb1.md`) |
| Z80 core (the DSB1's CPU) | done: full instruction set with undocumented flags, interrupts, T-state timing, save states; see `docs/hw/z80.md` |
| ADSP-21062 SHARC core (Model 2B's two geometry DSPs) | interpreter done and running both of `overrevb`'s programs on the 2B board model: every instruction group, 40-bit floating point, DAGs, loops, interrupts, host boot through DMA channel 6, save states (`make sharc-test`); written from Analog Devices' manual, see `docs/hw/adsp2106x.md`. Recompiler above |
| Model 2B-CRX board model | done for `overrevb`: the copro and geometrizer SHARCs booted through their host windows, their FIFOs and flags, the moved sound UART; the geometrizer program's raster stream has 2C's format, so both renderers take it unchanged. Worked out from the game's programs, see `docs/hw/model2b.md` |
| Enhanced views: any resolution, 16:9 / 21:9 with HUD moved to the edges | done for snapshots (`--height`, `--aspect`, `--hud`) |
| Custom soundtracks (replace any music track, game logic intact) | done offline: `make music SOUNDTRACK=name`; see `docs/mods.md` |
| Native window (`make play`) | playable: GPU renderer, live sound (cpal), keyboard and gamepad (gilrs), any window shape from 4:3 to 32:9, or 4:3 with the game's bezel beside it; about 2 ms of CPU per frame recompiled. Esc menu (sound mix, display, save slots, remapping keys and pad per game, game picker), save states; see `docs/play.md` |
| Browser build (WebAssembly + WebGPU) | playable (`make web`), with sound, keyboard, gamepad and touch, and a settings panel before each game (resolution, aspect, HUD, bezel, mix, remapping keys and pad, touch and tilt; kept in the browser); see `docs/web.md` |
| Second game: `skisuprg` (Sega Ski Super G, 2C) | playable (`make play GAME=skisuprg`): drive board modelled from the game's code, its own controls and coin-start script from its manifest; save states round-trip |
| Third game: `stcc` (Sega Touring Car Championship, 2C) | playable with sound effects and music (`make play GAME=stcc`): attract, car and gearbox select, name entry and races; its DSB1 music board (Z80) is modelled from its program. Needed geometrizer list jumps and mirrored ROM slots; recompiled build identical |
| Fourth game: `segawski` (Sega Water Ski, 2C) | playable with sound (`make play GAME=segawski`): attract, player and course select, runs. No extra boards; needed a second main data ROM bank at `0x6000000`; recompiled build identical |
| Fifth game: `waverunr` (Wave Runner, 2C) | playable (`make play GAME=waverunr`): attract, course select and races to game over, sound from the SCSP (no music board). Needed a data ROM window at `0x06000000` and the recompiler leaving code run from RAM to the interpreter; its ride-on cabinet's board isn't needed; recompiled build identical |
| Sixth game: `overrev` (Over Rev, 2C, Jaleco) | playable with sound (`make play GAME=overrev`): attract, mode, car and gearbox select, races. Jaleco's own sound driver runs on the SCSP board model unchanged (music streamed from sample ROM; no music board). Needed the link board's registers to read as absent; recompiled build identical |
| Seventh game: `hotd` (The House of the Dead, 2C) | playable (`make play GAME=hotd`) with light guns: mouse, touch, gamepad sticks or scripts aim, through generic gun controls in the manifest. Attract, coin-up and play through the first areas with shooting and off-screen reloads; sound from the SCSP. Needed a model of its gun interface (worked out from the game's code); recompiled build identical |
| Eighth game: `bel` (Behind Enemy Lines, 2C, EPL) | playable with sound (`make play GAME=bel`): two mounted guns aimed by mouse, touch, gamepad sticks or scripts. Attract, coin-up and the first mission with shooting and missiles to game over. Needed packed main data and polygon banks, the copro status read before its upload, and the sound UART's transmit interrupt; EPL's sound driver runs on the SCSP board model unchanged; recompiled build identical |
| Ninth game: `overrevb` (Over Rev, 2B, Jaleco) | playable with sound (`make play GAME=overrevb`): the 2B version of `overrev`, same data and controls, its own i960 program and two SHARC programs. Attract, menus and races look and sound as `overrev` does. Full speed recompiled (`RECOMP=1`: i960 and both SHARCs; races at 3.3x real time natively, and the browser's geometrizer keeps up), below real time interpreted; recompiled build identical |
| Tenth game: `sgt24h` (Super GT 24h, 2B, Jaleco) | playable with sound (`make play GAME=sgt24h`): attract, course, car and gearbox select, races to game over. The second 2B game, on the same SHARC code as `overrevb`; needed the i960's local-register cache and its I/O board type set in backup RAM (a manifest default); its link board's Z80 ROM isn't needed. Full speed recompiled; recompiled build identical |
| Eleventh game: `dynabb97` (Dynamite Baseball '97, 2B, Sega) | playable with sound (`make play GAME=dynabb97`): attract, team and pitcher select, batting with the bat lever, pitching and fielding. The first Sega 2B game: its own copro SHARC program, the same geometrizer SHARC code as Jaleco's games; needed nothing new from the board model. Full speed recompiled; recompiled build identical |
| Twelfth game: `doa` (Dead or Alive, 2B, Tecmo) | playable with sound (`make play GAME=doa`): attract, character select and fights through rounds, replays and the next opponent. Needed its protection device answered: the game's own check, with the answer it compares against declared in the manifest and read from the ROM (the device's algorithm is unknown); a bad dump of one polygon ROM, which drew Kasumi as shards, is repaired at load. Its geometrizer runs the shared Jaleco/Sega geometrizer crate, its copro program its own. Full speed recompiled; recompiled build identical |
| Thirteenth game: `indy500` (Indianapolis 500, 2B, Sega) | playable with sound (`make play GAME=indy500`): attract, course and gearbox select, rolling start and races. The earliest 2B game, with its own copro and geometrizer SHARC programs; needed the copro SHARC's external bus (the copro data ROMs, and buffer RAM) and the copro status its test menu reads. Full speed recompiled (4.2x real time natively); recompiled build identical |
| Fourteenth game: `fvipers` (Fighting Vipers, 2B, Sega) | playable with sound in scripted runs (`make play GAME=fvipers` to play it): attract, character select and fights through rounds. `indy500`'s geometrizer program and a copro program of its own that writes parts of the display list. Needed the copro SHARC's bank 2 as buffer RAM, list jumps the i960 patches in buffer RAM, 1 MB texture RAM banks (a manifest flag) and its polygon ROMs packed. Full speed recompiled (2.8x real time natively); recompiled build identical |
| Fifteenth game: `schamp` (Sonic Championship, 2B, Sega) | playable with sound in scripted runs (`make play GAME=schamp` to play it): attract, character select and fights through rounds and replays. The shared geometrizer program and a copro program of its own, on the same board work as `fvipers`; needed the SHARC core to ignore a reserved I/O register its copro writes, and the i960 recompiler to drop a block discovery found at an odd address. Full speed recompiled (2.7x real time natively); recompiled build identical |

## Quick start

Requires a Rust toolchain (stable) and `make`.

```sh
make                  # list every target
make check            # format check, clippy, unit tests (no ROMs needed)
make m68k-tests       # 68000 core vs the SingleStepTests suite (fetches ~200 MB, 1.1 GB unpacked, into out/)
make z80-test         # Z80 core vs the zexdoc/zexall exercisers in out/z80-tests (docs/hw/z80.md)
make sharc-test       # ADSP-21062 (SHARC) tests, plus the 2B programs if captured (docs/hw/adsp2106x.md)

# With ROM sets in ./rom, as MAME keeps them (rom/topskatr.zip, or unpacked in rom/topskatr/):
make verify           # check every chip
make scan             # list the sets in rom/ (MAME zips or folders) and check the supported ones
make boot             # boot record and system-table entry points
make analyze          # code discovery -> out/topskatr/program.lst
make disasm ADDR=0x6f0 COUNT=64
make run STEPS=50000000   # boot on the interpreter, report hardware use
make run SNAP=60          # also write the tile layer every 60 frames
make run STEPS=900000000 SNAP=115 INPUT=coin-start   # coin-start: the game's own coin-up and start
make inputs               # the game's input names for INPUT=, and their keys and pad bindings
                          # light guns: gun1=x:y@from-to aims (view pixels), gun1=off@... off screen;
                          # name@from-to/n holds an input for 2 frames every n (e.g. trigger1@2000-3000/20)
make sound                # render the sound commands of the last run -> out/topskatr/sound.wav
make sound-disasm ADDR=0x6004ea   # disassemble the sound 68000 program
                          # coin up, start, and play into a race
make run STEPS=485000000 INPUT=coin1@1100-1104,start@1250-1254 \
    VIEW="--height 1080 --aspect 21:9 --hud edges"   # enhanced snapshot
make dump-textures        # race run: every 3D texture and tile drawn -> out/topskatr/texdump/
make run STEPS=600000000 INPUT=coin1@1100-1104,start@1250-1254 \
    PACK=out/topskatr/texpack VIEW="--height 1080"   # snapshots with a texture pack (docs/texture-packs.md)
make assets               # extract the game's art for review -> out/topskatr/assets (needs ImageMagick)
make bench                # timed run into a race: speed and state hash
make latency              # frames from a wheel change to the game reacting and its polygons (LATENCY_CHANGE, LATENCY_FRAME)
make recomp               # recompile the game's i960 code and TGP or SHARC programs (after a `run`; builds only what changed)
make bench RECOMP=1       # same run on the recompiled build: same hash, about twice as fast
make play RECOMP=1        # play in a native window (Esc menu, F5/F9 quick save/load, F11 fullscreen)
make web RECOMP=1         # browser build -> out/web: game list, a page and module per game (WebGPU; serve over HTTPS, docs/web.md)
make dist                 # player to hand out (interpreter only, no ROMs) -> out/dist/remodel-VERSION-TARGET.tar.gz
make capture              # save renderer inputs from a run into a race -> out/topskatr/capture
make gpu-check            # draw them on the software and GPU rasterisers, time and compare
make lockstep RECOMP=1    # recompiled vs interpreted i960 (and 2B SHARCs), block by block, until they differ
make music                # after a `run` into a race: the music board's music -> out/topskatr/music.wav (GAME=stcc too)
make soundtrack-import SRC=~/my-mix SOUNDTRACK=my-mix   # your own music (ffmpeg converts)
make music SOUNDTRACK=my-mix      # same, with your tracks -> out/topskatr/music-my-mix.wav
```

Custom soundtracks, their `tracks.txt` format and how the game still drives
them are described in `docs/mods.md`.

`make bench` prints a hash of all RAM and the last rasteriser stream. A change
meant only to make things faster must leave it unchanged. The hash depends on
`out/GAME/eeprom.bin`, which `run` creates on first boot.

Every target takes `GAME=skisuprg` for Sega Ski Super G, `GAME=stcc` for
Sega Touring Car Championship, `GAME=segawski` for Sega Water Ski,
`GAME=waverunr` for Wave Runner, `GAME=overrev` for Over Rev,
`GAME=overrevb` for its Model 2B version (its chips in `overrev.zip`, as
MAME merges them), `GAME=hotd`
for The House of the Dead or `GAME=bel` for Behind Enemy Lines (`make play GAME=stcc`, `make capture
GAME=skisuprg`, ...), except `make assets`, which is `topskatr`'s, and
`make music`, which needs a music board (`topskatr`'s DSB2 or `stcc`'s DSB1).

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
  model2/    board model (2B and 2C): memory map, IRQs, timers, I/O,
             copro and geometrizer interfaces
  mb86235/   MB86235 (TGPx4) decoder, disassembler and interpreter
  adsp2106x/ ADSP-21062 (SHARC) decoder, disassembler and interpreter
             (Model 2B's geometry DSPs; generic)
  m68k/      68000 interpreter and disassembler (sound CPUs)
  z80/       Z80 interpreter (DSB1 music board CPU)   (generic)
  sound/     SCSP (YMF292) with its DSP, and the sound board (Model 2-generic)
  mpeg/      MPEG-1 Audio Layer II decoder          (generic)
  dsb/       DSB2 and DSB1 MPEG music boards: 68000 or Z80, UART,
             decoder chip model, replacement soundtracks (WAV/MP2
             reader, resampler)
  png/       PNG decode/encode with zlib inflate/deflate (generic)
  cli/       the `remodel` command
tools/
  asset-review.py   distinct textures, screens and contact sheets from capture runs (`make assets`)
```

## Reuse across Model 2

All Model 2 boards share the i960KB main CPU and the 68000/SCSP sound
system. The geometry processor is what differs between revisions:

| Component | Model 2 | 2A | 2B | 2C |
|---|---|---|---|---|
| i960 decoder, interpreter, recompiler | shared | shared | shared | shared |
| ROM loading, code analysis | shared | shared | shared | shared |
| Sound (68000 + SCSP) | shared | shared | shared | shared |
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
processor); `overrevb`, `sgt24h`, `dynabb97`, `doa` and `indy500` are Model
2B-CRX (ADSP-21062 SHARCs). The manifest's
`board` picks the board model: `crates/model2` keeps what differs between
the two (the copro and geometrizer cores, the 2B host windows, the UART
address) behind it, with nothing keyed on a game's name. A manifest's
`parent` marks a clone: its chips may sit in the parent set's zip or
folder, and it shares the parent's poster and bezel.

The board table is from public documentation (largely MAME's driver notes).
MAME does not run `topskatr` correctly, so its board details are treated as
**unverified hypotheses**: the game's own code is the ground truth, and the
board model logs every access it doesn't fully understand. Places where
MAME is demonstrably wrong are tracked in `docs/mame-defects.md`, written up
so they can become upstream patches.

## What's known about the Model 2C board

From the five games' own code and test menus; it holds for every 2C set
supported unless a game's section says otherwise.

- *Confirmed (from the code, then by running it):* the memory test in
  `segawski`, `skisuprg` and `waverunr` checks two more RAMs on the video
  side, IC55 at `0x10A00000` and IC54 at `0x10C00000`, 64 KB each. Its
  table entry gives each one byte-wide, 64 KB long, compared through mask
  `0x0F` (a 4-bit-wide RAM, presumably 64K x 4), next to IC81, the luma
  RAM at `0x11400000`, tested the same way. The test sets render mode bit
  0 (`0x10000000` = `0x4005`, host access) first, then for 16 rotations of
  `0x1234ABCD` saves each location, writes the pattern, reads it back and
  restores it. Remodel left both unmapped (reads 0), so the three games
  reported them BAD; they are now plain RAM (`bus.rs`). No game reads or
  writes them outside the test in a coin-start run, so their function is
  *unconfirmed*, as are mirrors past 64 KB and the upper four bits of each
  byte. `topskatr`'s and `stcc`'s memory tests don't list them.
- *Confirmed (from `bel`'s code and memory test):* ROM banks don't always
  give each chip pair an 8 MB slot. `bel`'s main data (four pairs of
  16 Mbit chips) sits every 4 MB, `.11/.12`, `.9/.10`, `.7/.8`, `.5/.6`
  from `0x2000000`, and so do its four polygon pairs (model addresses run
  to `0xBFFFFF`, 4M words), while its two texture pairs still take 8 MB
  slots. The manifest lists the packed banks (`Game::packed`). Why the
  banks differ (jumpers, the ROM board revision) is *unconfirmed*.
- *Confirmed (from `bel`'s code):* the copro status `0x980014` reads with
  bit 0 set before any program is uploaded, not only once an upload has
  started: `bel` uploads its copro program at boot only if bit 0 of
  `0x980010` or `0x980014` is set, and otherwise sends commands to a copro
  with no program. The board model now reads all ones whenever the copro
  holds no program. The other games start an upload unconditionally.
- *Confirmed (from `bel`'s code):* the sound UART raises I/O request bit
  10 (IRQ3) while its transmitter can take a byte, and a game may send
  its sound commands from that interrupt (`docs/hw/peripherals.md`,
  `docs/hw/sound.md` §7).
- *Unconfirmed:* `stcc`'s memory test reports IC29 and IC30 BAD; those are
  its copro data ROMs (`.29/.30`), which the i960 can only reach through
  the TGP. Not investigated.

## What's known about Model 2B

From `overrevb`'s i960 program and its two SHARC programs, run on the board
model and compared with `overrev` (the same game on 2C, which shares every
chip but the program pair). Details, addresses and evidence in
`docs/hw/model2b.md`.

- *Confirmed (`overrevb`, `sgt24h`):* 2B has **two SHARCs**, the copro and
  the geometrizer, each programmed by the i960 through its I/O-processor
  registers (host windows `0x8C0000` and `0x840000`): a DMA of 4,096 48-bit
  words, sent as 16-bit halves to `0x884000` / `0x804000` while bit 31 of
  `0x980000` (copro) or `0x980008` (geometrizer) is set. Both Jaleco games
  upload the same two SHARC programs: the same code, word for word, but
  not the same images, since the upload always sends 4,096 words and
  the tails past the code differ (one data word in the copro's; in the
  geometrizer's, from `0x20C31` on, whatever follows it in each i960 ROM,
  in `sgt24h` the i960 program's own strings).
- *Confirmed (`overrevb`, with the ADSP-21062 core):* each SHARC is booted
  as UM §11.6.3 describes for a 16-bit host. Its window maps IOP register
  *n* at window + 4*n*, each 32-bit register written as two 16-bit halves,
  low first: DMAC6 = 0, II6 = `0x20000`, IM6 = 1, C6 = `0x1000`, DMAC6 =
  `0xA1` (instruction words, 16-to-48 packing, LSW first), then 12,288 halves
  to EPB0 (the geometrizer first gets MSGR7 = `0x7378842E`). The DMA's end
  vectors through `0x20040` (`RTI`) to `0x20005`, `JUMP 0x20080 (DB)`. The
  copro program has 2,111 words of code, the geometrizer's 2,987, none with
  an undefined encoding (`docs/hw/adsp2106x.md`).
- *Confirmed (the upload code and UM §11.6.3):* bit 31 of `0x980000` /
  `0x980008` is not the SHARC's RESET: the i960 programs the SHARC while
  the bit is set (a SHARC in reset ignores its host) and the program runs
  once it is cleared. It behaves as the host bus request (HBR);
  *unconfirmed* that it is that pin, and what resets a SHARC for a second
  upload (each is uploaded once a boot). `0x980020` is written 0 after
  each bit 31 is set: *unconfirmed* what for.
- *Confirmed:* the copro keeps 2C's i960 interface: commands into a FIFO
  through `0x880000 + 16·fn` (tagged) and `0x884000`, results read from
  `0x884000`, `0x980004` bit 0 set while there are none. The SHARC reads
  commands from external `0x400000` (FLAG0: FIFO empty) and writes results
  to `0xC00000` (FLAG1: FIFO full).
- *Confirmed:* the geometrizer keeps 2C's display list ports, buffer RAM
  and frame protocol (FLAG0 is 2C's F0). The SHARC reads the list from
  external `0x400000`, writes the rasteriser stream to `0x410000`, and
  reads the polygon ROM through an address register at `0x430000` and a
  data port at `0x430001`; polygon RAM is written at `0x420000` and its
  fast part is the SHARC's own internal memory (neither is used by
  `overrevb`).
- *Confirmed:* the 2B geometrizer program emits **the same rasteriser
  stream format as the 2C programs**, so the renderers take it unchanged.
  Fed the same lists as `overrev`'s 2C geometrizer, it gives the same
  command words and lengths, vertices within float rounding, and a
  rendered frame within 4 pixels; the two programs differ in the luma word
  of back faces (`docs/hw/model2b.md`).
- *Confirmed:* the sound UART is at `0x9C0000` (data) and `0x9C0004`
  (mode, command, status), one register a 32-bit word (2C: `0x1C80000`,
  two a word). The 2B program also writes `0x40` and `0x4E` to `0x1C80004`
  at boot: *unconfirmed* what is there.
- *Unconfirmed:* the SHARC clock (40 MHz, general knowledge; Remodel runs
  5 SHARC instructions per i960 instruction), the FIFO depths and the wait
  states. The copro and geometrizer flags the programs drive (copro FLAG2
  error, FLAG3 waiting; geometrizer FLAG1-3) go nowhere in the model.
- *Confirmed (`overrevb`, by running them recompiled):* neither SHARC
  program writes its own code in a coin-start run into a race, and
  everything they execute is reached statically from the vectors and
  their dispatch tables (2,111 copro words, 2,987 geometrizer words);
  `docs/recompiler.md`. *Unconfirmed* for paths a coin-start run doesn't
  take (the geometrizer's code upload command, fn 0x1D, would load code;
  any such word runs on the interpreter).
- *Confirmed (`sgt24h`):* boot clears work RAM (`0x500000-0x57FFFF`,
  its callers' stack frames included) from inside the function called at
  `0x710`, then returns. That works only because the i960KB keeps recent
  callers' local registers on-chip: its data sheet says the cache "holds up
  to four local register frames" (the current one and three callers'), and
  the oldest is written to memory only when a fifth procedure is entered;
  `flushreg` writes the cached sets out (i960 Jx manual, and the 80960KB
  Programmer's Reference Manual, 270567-001, ch. 4). With the cache modelled (`i960::cpu::LocalCache`)
  the unpatched ROM boots past `0x710` to "CHECKING NETWORK NOW", "STAND
  ALONE" and "INSERT COIN" on the 2C model with the 2B stubs; without it,
  the return reloads the zeroed frame and faults at IP 0. That it is
  exactly three cached sets is the data sheet's, not tested by the game.
- *Confirmed (`dynabb97`, ROM and by running it):* Sega's own 2B game
  uploads its SHARCs the same way. Its geometrizer program is the Jaleco
  games' code word for word (2,987 words; the images differ only from
  `0x20C31` on, past the code), so the geometrizer SHARC program looks
  like the board's standard one (*unconfirmed* beyond these three games).
  Its copro program is its own (1,956 words of code, different from
  `0x20099` on) and uses only the two FIFOs and flags; nothing else on
  the board model needed changing.
- *Confirmed (`doa`, from the code):* on return the i960 takes the
  caller's frame from PFP with only bits 0-3 (return status and the
  prereturn-trace flag) cleared, not the six low bits the 80960KB manual
  says it ignores (270567-001, ch. 4 and appendix E: new frames are
  aligned on 64 bytes). `doa`'s task switch loads FP, PFP and SP by hand
  with 16-byte-aligned frames (the first at `0x5018D0`); clearing bits 4
  and 5 on return lands it on the wrong frame, and the game never gets as
  far as uploading its SHARC programs. Calls still open 64-byte-aligned
  frames (`i960::cpu::Cpu::ret`).
- *Confirmed (`indy500`, from its copro program, then by running it):*
  the copro SHARC's external bus has more than its two FIFOs on a board
  with copro data ROMs. With SYSCON's MSIZE 10 (every 2B copro program
  sets it) the SHARC's four external banks are 8M words each from
  `0x400000` (ADSP-2106x User's Manual §5.4.1): the command FIFO is bank
  0, the result FIFO bank 1, buffer RAM bank 2 (`0x1400000`; confirmed
  by `fvipers`, below: `indy500`'s runs never touch it) and the data ROMs
  `.29/.30` bank 3 (`0x1C00000`), read as 32-bit words as on 2C. Whether
  either repeats through its bank is *unconfirmed*; details in
  `docs/hw/model2b.md`, "The copro's external memory".
- *Confirmed (`indy500`, from the code):* its test menu's T.G.P. TEST
  reads `0x980014` once and shows IC31 (bit 1) and IC37 (bit 0) BAD for
  a set bit. Answered as on 2C (all ones until the programs are uploaded,
  then 0) both show GOOD; *unconfirmed* what the bits really report.
- *Confirmed (`fvipers`, from its programs and the i960 code, then by
  running it):* the copro SHARC's bank 2 is **buffer RAM**, the display
  list, word addressed. The i960 leaves an 18-word hole in the list by
  moving its write pointer and sends the copro fn `0x78` with the hole's
  offset; the copro writes a fn `0x0B` matrix and a fn 1 object there.
  Remodel had a separate RAM there for `indy500` (never touched by its
  runs); with it gone `indy500`'s state hash is `4a1b197bf00149d6`, its
  runs otherwise unchanged.
- *Confirmed (`fvipers`, `schamp`, from the programs):* Sega's 1995-96
  fighting games don't write MSGR7 when uploading the geometrizer, so its
  program checks the tag of every command word (fn in bits 27-23, 12-8
  and 4-0) and, on a word that fails, leaves it in MSGR3 and toggles
  FLAG2 forever: the 3D stops for good. That is where both stopped, on
  words from the two places above and below. They also upload with
  32-bit IOP writes, set SYSCON and WAIT through the host window, and
  send only the program's length.
- *Confirmed (`fvipers`, from the code):* the i960 writes fn 0
  placeholders through the command port and later patches them into list
  jumps, or retargets jumps, by writing buffer RAM directly. The list DMA
  now follows such words; what tells a jump from a data word on the board
  is still *unconfirmed* (the rule that fits every game is in
  `docs/hw/model2b.md`, "Sega's earlier games").
- *Confirmed (`fvipers`, `schamp`, from the code and the memory test):*
  they upload textures only to the upper half of each texture RAM bank's
  2 MB window (`0x11100000`, `0x11300000`), which their boards must read
  as the 1 MB the rasteriser uses (`indy500` and `doa` use the lower
  half; `overrevb` and `sgt24h` fill both with different data). A
  manifest flag, `texture_ram_mirrored`, maps those banks as 1 MB
  repeating; *unconfirmed* what makes their boards differ.
- *Confirmed (from the programs, the games' I/O and by running them):*
  the geometrizer is render-only. Its program writes only the rasteriser
  FIFO, its polygon RAM and polygon memory address, and a handshake
  strobe; no 2B game reads its host window (`0x840000`), and with `--geo
  native` (the SHARC runs no frame) coin-start runs of 600M instructions
  end with the SHARC's state hashes (`sgt24h` `282c8d6fd5151c21`, `doa`
  `78f308b4da95cefb`, `schamp` `8d2b7b4cd55d64a0`). Its command set,
  formats and arithmetic (32-bit floats rounded toward zero, `RSQRTS` and
  two Newton steps for computed normals) are in
  `docs/hw/geometrizer-2b.md`, with the native implementation that
  reproduces its stream word for word. `indy500`'s and `fvipers`'
  program is the same but for one bug: in the computed-normal modes it
  scales the first vertex's y by the x focal length.
- *Confirmed (by fuzzing the program, `make geo-fuzz`):* its code for
  models in its own memory ("fast" polygon RAM, which no game uses) sends
  the last word twice when an object ends by its count, and loses track of
  the previous edge after a computed-normal triangle with link type 0 or
  2. A board test (fn 0x0E) whose 32 pattern words aren't a walking one
  runs on through the list: the routine it calls sits in the last
  instructions of a loop (the ADSP-2106x manual, §3.5.1.1). *Unconfirmed*
  on hardware: these follow from the SHARC model.
- *Confirmed (`doa`, from the code, then by running it):* Tecmo's game
  checks for a protection device at `0x1D80000-0x1D87FFF` at boot. The
  board model answers it from the manifest (`romset::Protection`,
  `crates/model2/src/protection.rs`); the interface is in
  `docs/hw/model2b.md`, "The protection device".

## What's known about Model 2A (scoping, not yet supported)

From a throwaway boot of `srallyc` on the 2C board model, and from the
ROMs. Nothing here runs yet.

- *Confirmed (`srallyc`'s code):* the 2A TGP's program is **uploaded by the
  game**, not internal to the chip as general knowledge had it. The loader
  holds the copro in reset (`0x980000` bit 31), writes 1,664 32-bit words to
  `0x884000`, releases it, writes 0 to `0x880080` and requires the copro to
  answer 8, else "Coprocessor Load Error". `vcop`, `daytona` and `vf2` each
  carry their own TGP program (a 6-word vector and a dispatch table of 73,
  73 and 112 entries); the programs differ between games. So the TGP needs
  an interpreter for its (undocumented) instruction set, not a
  function-by-function model.
- *Confirmed (numerically):* the shared Model 2/2A chips are maths tables:
  `opr-14742a.45/.46` a quarter-wave sine (65,536 steps a circle, within
  3e-8), `.58/.59` reciprocal mantissas, `.62/.63` inverse square root
  mantissas for even and odd exponents.
- *Confirmed (`srallyc`):* no geometrizer program is uploaded; display lists
  go straight to `0x804000`. The 2A geometrizer is presumably fixed-function
  (unconfirmed). Writes to `0x12000000-0x128xxxxx` look like texture and
  luma RAM (unconfirmed).

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
  whose 3D scenes are not rendered yet. Its attract text scrolls out and
  back in through the split plane (`docs/hw/tilemap.md`).
- *Confirmed:* coin 1 is IN0 bit 0 and start is IN0 bit 6 (active low). A
  scripted coin and start go through course select and skater select into
  a race: the HUD timer, checkpoints and hints all run.
- *Confirmed:* the other controls, each from the game's response to a
  scripted press (all active low):
  - select right is IN0 bit 4 and select left is IN0 bit 7: they move the
    course-select highlight;
  - jump front is IN0 bit 5 (the skater presses the nose) and jump tail is
    IN1 bit 0 (presses the tail into a manual);
  - analog channel 0 is steering: 0x10 carves right, 0xF0 carves left, 0x80
    is centred;
  - analog channel 1 is slide: 0xF0 makes the skater crouch into a slide,
    with sparks off the board.
- *Confirmed:* gameplay frames render in 3D with textures from the game's own
  geometrizer output, composited between low- and high-priority tiles. This
  confirms:
  - vertex linking, the 24-bit float stream, and the projection with the sync
    offsets (xoff 0, yoff 128);
  - texture headers and points from the `.27`/`.25` ROM, and the 4bpp 2x2-packed
    texel sheets in texture RAM;
  - the luma tables and the colour translation tables (section 5 of
    `docs/hw/geometrizer.md`).
- *Confirmed (from the ROMs; see `docs/hw/sound.md`):* the sound program
  (`mpr-19759.31`, word-swapped) is Sega's "CRX_drv" Ver3.2.8 and runs from
  `0x600000`. It routes SCSP MIDI-in to 68k IRQ level 3, timer A to level 2
  and timer B to level 1, and parses its input as MIDI with running status.
  The i960 sends 3-byte messages (`An`/`Bn` statuses) from a 278-entry
  table, plus `0xFF` at init, and never reads the UART data register. Music
  is `AE 10 nn` on channel 14, which the DSB2 firmware (working title "Ollie
  Over") captures, while it forwards all other bytes out of its own UART.
  The MPEG ROMs hold MPEG-1 Layer II, 32 kHz, 128 kbit/s stereo, stored
  unswapped.
- *Confirmed (running the sound program on `crates/sound`):* the driver
  streams every sound: the 68000 copies 512-byte chunks of 8-bit PCM from the
  sample ROM into 8 KB double buffers in sound RAM (`0x60000-0x6FFFF`) and
  refills by polling the slot monitor's CA bit. Voices play at 22.05 kHz,
  stereo sounds as hard-panned slot pairs. It loads a 108-step DSP program
  (reverb into the ring at `0x70000`). A `0xFF` byte soft-restarts the
  driver. Channel 14 is not special to it: the game's `AE` commands hit empty
  table entries, so they make no SCSP sound whether or not the link runs
  through the DSB2 first.
- *Confirmed (DSB2 firmware and MPEG ROMs; `docs/hw/sound.md` §3):* the
  DSB2 firmware holds a 12-track table at its ROM `0x8000`. Each entry gives
  a start and end byte address in the 16 MB MPEG space (always on frame
  boundaries), a volume per side and an optional loop point. `AE 10 nn` plays
  track nn (0 = stop), `AE 02 vv` sets the master volume, `AE 03 vv` fades out
  and then stops. Remodel's decoder matches ffmpeg's to within 3e-6 of full
  scale on the game's MPEG data. Of the 12 tracks, 1-3, 5, 7-9 and 12 are
  played with looping armed (`64` before `74`) and 4, 6, 10 and 11 once.
  The firmware never reads decoder status beyond the always-clear busy bit,
  so it can't tell when a track ends. A replacement soundtrack of a
  different length therefore can't change the game's behaviour
  (`docs/mods.md`).
- *Confirmed (enhancement):* topskatr's courses hold up in wide views. At
  16:9 and 21:9 the game sends enough of the scene that the widened
  viewport fills to the edges with no visible pop-in across a race, so no
  game patch has been needed so far. The 3D HUD (speed gauge, time-limit
  ring) is in window 4, separate from the course (window 2) and sky
  (window 0).
- *Unverified:* the board map, IRQ controller semantics (`0xE80000`
  request/ack, `0xE80004` enable), timers and I/O chip layout in
  `crates/model2/src/bus.rs`.
- Regions the code reads and writes directly: work RAM `0x00500000`; a very
  hot port at `0x00800000` (about 4.9k accesses to 26 addresses, likely the
  geometry processor's input); `0x01C00000` and `0x01D00000` (probably I/O);
  `0x01000000` and `0x01800000` (probably tilemap and palette).

## What's known about `skisuprg`

Second 2C game, added to find what is still `topskatr`-specific.

- *Confirmed:* the program is `epr-19489.15`/`epr-19490.16` at `0x0` (`.15`
  = low half) and `epr-19551.13`/`epr-19552.14` at `0x100000` (`.13` low;
  the code loads a table at `0x195900` that only makes sense in that order).
  Boot record: PRCB `0xB0`, first IP `0x5F0`, interrupt table `0x46E20`,
  system-procedure table `0x1C0`. Nothing executes above `0x100000`.
- *Confirmed:* main data is `.11/.12` at `0x2000000` and `.9/.10` at
  `0x2800000`, as in `topskatr`: a table at `0x2E783E0` holds pointers into
  `0x2E77xxx`. Both MB86235 programs are uploaded from the program ROM
  (copro from `0x8D480`, geometrizer from `0x90708`).
- *Confirmed:* the polygon ROM is 16 MB (`.17/.21` then `.18/.22`), four
  times `topskatr`'s, so the geometrizer now sizes its ROM window from the
  ROM. Copro data (`.29/.30`) fills the whole 8 MB window.
- *Confirmed:* init enables IRQ mask `0x19` (`topskatr`: `0x215`).
- *Confirmed:* `epr-19491.31` is CRX_drv Ver3.2.8 (96 10/28); the sound board
  plays the attract's commands (`make sound GAME=skisuprg`).
- *Confirmed (from the code):* the game needs its drive board. Its vblank
  handler reads the board's reply from the I/O byte at `0x1C00006` and then
  writes the frame's command to `0x1C00008`. At boot it sends `0x00`, waits
  for the reply `0xCE`, sends a byte from its settings (the swing channel's
  calibrated centre, `0x80` by default), waits for `0xFF`, then sends
  `0x00`; after 479 frames it gives up and shows "DRIVE BOARD TROUBLE
  CODE: xx" with the last reply in hex (`FF` with nothing attached).
  Replies `0xB0`-`0xB7` are faults; `0xB8`/`0xB9` start a 120-frame
  warning. In the test menu, after command `0xDC` each nibble of the reply
  shows a switch (`0x0`/`0xF`: off/on), and after `0xA3`/`0xA4` bit 4 is a
  status the drive test shows and follows.
- *Confirmed (from the code):* commands seen: during a run `0x10`-`0x1F`
  (picked from tables by the ski inputs), `0x40` and `0xD1` in turn, one a
  frame; `0x80` at rest; `0xC2`/`0xC3` around test mode and attract;
  `0xA0`-`0xA6` and `0xD0` in the drive tests. *Unconfirmed:* what each
  does on the board, and what a real board replies to them.
- *Confirmed (by running it):* a drive board that replies `0xCE` while told
  `0x00` and `0xFF` to anything else (`crates/model2/src/drive.rs`, chosen
  by the manifest's `drive_board`) passes the boot check; the game runs its
  attract, takes coins and plays a full run (`make capture GAME=skisuprg`
  reaches the results screen).
- *Confirmed (by running it):* `make recomp GAME=skisuprg` recompiles its
  i960 code (keyed by its program hash, beside `topskatr`'s) and both
  MB86235 programs; `make lockstep GAME=skisuprg` runs 300M instructions
  identical, and a coin-start run reaches the same state hash on both.
- *Confirmed (from the code):* the boot code calls the per-frame input
  read, which reads all eight analog channels and uses channels 0-2, each
  through a calibration table built from its settings (min, centre, max per
  channel; defaults `0x00`/`0x80`/`0xFF`). Digital inputs are read at
  `0x1C00002` (twice: with I/O register 0 bit 0 clear and set),
  `0x1C00004`, `0x1C00006` (the drive board's reply) and `0x1C0000C` (the
  last inverted).
- *Confirmed (by scripted runs):* coins are port `0x1C00002` bits 0 and 1
  as in `topskatr`, and one credit is two coins by default. The three
  panel buttons (blue, red, green) are bits 6, 7 and 4 of that port: on
  the select screens they pick the left, middle and right choice, and blue
  also starts a game. Channels 0 and 1 both turn the skier, in opposite
  senses (right: channel 0 up, channel 1 down); the game's demo drives
  them with equal calibrated values. Remodel names them `edge` and
  `swing`; *unconfirmed* which ski angle each measures.
- *Unconfirmed:* channel 2 (`push`): the skier's physics adds its offset
  from `0x80` to the speed outside `0x70`-`0x8F`, the demo sets it at the
  start, but holding it had no visible effect in a run. Port `0x1C00002`
  bit 5 and `0x1C00004` bit 0 step a selection right and left, and
  `0x1C00004` bit 4 confirms, in a mode of the select screens the runs
  didn't reach (named `selright`, `selleft`, `decide`); which switches they
  are on the cabinet is unknown. The test menu uses port bits 2 and 3
  (test, service) and bits 6 and 7 of the second `0x1C00002` read.

## What's known about `stcc`

Third 2C game: the first with a DSB1 music board, a wheel and a gearbox.

- *Confirmed:* the set's three program pairs (`epr-19272a/19273a`,
  `epr-19272b/19273b`, `epr-19570/19571`) are revisions of the same program;
  the manifest uses `epr-19570.15`/`epr-19571.16` (`.15` low half), the one
  with the most added. Boot record: PRCB `0xB0`, first IP `0xD00`, interrupt
  table `0x689E0`, system-procedure table `0x1C0`.
- *Confirmed:* ROM banks are 8 MB slots and a pair smaller than its slot
  repeats through it. Main data `.11/.12` (8 MB) is at `0x2000000` and the
  1 MB `.9/.10` pair at `0x2800000`, but the game reads it at `0x2C4B210`
  ("Not CGM Data" until it mirrors; a CGM header sits at `0x4B210` of the
  pair). The 1 MB second polygon pair `.18/.22` is likewise read 4 MB into
  the second polygon slot (a model in the same format as the first pair's
  starts there), and the 1 MB second texture pair `.28/.26` at the start of
  the second texture slot. Byte order within each pair is confirmed by
  strings and headers, except `.28/.26` (low half `.28` by analogy with
  `.27/.25`; unconfirmed). Copro data `.29/.30` is 4 MB, repeated through
  its 8 MB window (unconfirmed whether the program reads past 4 MB).
- *Confirmed:* `epr-19274.31` is CRX_drv Ver3.2.5 (96 8/1); `make sound
  GAME=stcc` plays a run's commands, and the 68000 never touches anything
  outside the sound board.
- *Confirmed (from the code and by running it):* the geometrizer list uses
  **jump words**: the game builds each frame's list in pieces (buffer RAM
  `0x00000`/`0x10000` banks, pieces at `+0x4000`, `+0xC000`, ...) and links
  them with command-port writes with bit 31 set (`0x80004000`: continue at
  `0x4000`). The geometrizer program has no jump handling, so the list DMA
  must follow them. Data words with bit 31 set (negative floats) are not
  jumps, and the game also overwrites old jump words directly (the copro
  stores matrices into the list), so Remodel follows only jump words written
  through the command port and not stored over by the copro since
  (`fvipers` patches jumps in with the i960, which they survive; Model 2B
  above). *How the hardware tells them apart is unconfirmed.* Without this the
  geometrizer stops at its command-check error on the first linked list
  and no 3D is drawn.
- *Confirmed (from the code):* the wheel board is commanded like
  `skisuprg`'s drive board (reply read from `0x1C00006`, then a command
  written to `0x1C00008`, every frame), but the game only stores the reply
  and never reads it back: no handshake, no fault check, and it runs the
  same with nothing attached. Commands seen: `0x00` through boot, `0x98`,
  `0x30` (attract, menus), `0x34` (select screens), `0x38` (racing), and
  `0x5x`/`0x6x` while racing with steering and collisions. *Unconfirmed:*
  what each does; presumably force effects. The model (`Drive::Wheel`)
  replies `0xFF` and keeps the last commands.
- *Confirmed (from the code):* the link is decided by the settings: the
  network check runs only if the "LINK TYPE" setting isn't stand-alone
  (the default). It then reads the link board at `0x1A14000`, bit 0 set
  meaning "NO COMMUNICATION BOARD". Nothing needs answering for a single
  cabinet.
- *Confirmed (from the code):* the DSB1 needs nothing for the game to run.
  The i960 never reads the sound link's receive register, and its link
  bytes reach the sound board as in `topskatr`. The Z80 program
  (`epr-19275.2s`, "MPEG SOUND DRIVER Z80 VERSION 0.26 - 96/08/29") takes
  each byte in its interrupt handler (mode 1, `0x38`) from UART data port
  `0xF0`, waits for bit 0 of status port `0xF1` (transmitter ready),
  echoes the byte out of `0xF0`, and stores it in a 4 KB ring at
  `0x8000-0x8FFF` (write pointer at `0x9002`). So the DSB1 takes the same
  link bytes as the sound board and needs no reply path.
- *Confirmed (from the Z80 program, then by running it):* the DSB1 plays
  the music (`docs/hw/dsb1.md`). ROM `0000-7FFF` (8 KB used), RAM
  `8000-FFFF`, no banking. The decoder sits on ports `E0` (play mode: 0
  stop, 2 play and continue into the next queued segment, 1 play to the
  end), `E2-E4` start (read back: current address), `E5-E7` end (the
  last byte; confirmed by all 23 segments ending one frame short of
  end + 1), `E8` attenuation (`00` loud to `7F` silent). Channel 15
  commands: `AF 6F n` plays track `n` (0 stops), `AF 01 v` scales later
  volumes, `AF 02 v` sets the volume, `AF 03 r` fades out. The 17 tracks
  are lists of segments (start and end in the 8 MB MPEG space) that the
  program queues one ahead while the decoder plays. A coin-start session
  plays track 1, then track 11 looping until the race's fade-out.
  *Unconfirmed:* the Z80 clock (4 MHz used; it sets only the fade speed),
  the attenuation curve (linear used), and what ports `E9`, `EB` and `E8`
  values above `7F` set (ignored).
- *Confirmed (from the input test, then by scripted runs):* port
  `0x1C00002` bits 0/1 coins (one coin a credit by default), 2 test, 3
  service, 6 start; port `0x1C00004` bit 0 view 1 (the camera changes),
  bit 1 view 2 (not tried), bits 4/5 shift up/down; analog channel 0 the
  wheel (0x30 steers left, 0xD0 right), 1 gas and 2 brake (0x00 released,
  0xFF down). The gas pedal also confirms on the menus and the wheel moves
  the selection. Holding the brake while confirming the car picks the
  manual gearbox (MT), which the shift buttons then step through (1-4 and
  back in a scripted run).
- *Confirmed (by running it):* `make capture GAME=stcc` (coin-start)
  reaches the qualifying lap, where the car waits for gas; with gas held
  it laps, and the attract demos race in 3D. `make state-check GAME=stcc`
  round-trips, `make lockstep GAME=stcc` runs 300M instructions identical,
  and a coin-start run of 600M instructions gives the same state hash
  (`bbde4ebfb8d918d8`) recompiled and interpreted.
- *Unconfirmed:* `epr-18643a.7` (also in `sgt24h` and `von`) is Z80 code
  whose header names Sega part 837-10074; it belongs to another cabinet
  board, most likely the link board (see "What's known about
  `sgt24h`").

## What's known about `segawski`

Sega Water Ski, a different game from Ski Super G (water skiing behind a
boat, not snow), on the same 2C board. The set is the Japanese release
(the boot warning says "used only in Japan"; the program's build date is
Aug 22 1997).

- *Confirmed (from the code and the game's memory test):* program
  `epr-19963a.15`/`epr-19964a.16` at `0x0` (`.15` low); main data `.11/.12`
  at `0x2000000` and `.9/.10` at `0x2800000` (its pointer tables point
  into `0x2800000`), and a second data bank: `.7/.8` at `0x6000000` (the
  font is loaded from `0x622BF70`, and its own pointer tables point into
  `0x6000000`) and the 1 MB pair `.5/.6` at `0x6800000`, read 4 MB into
  its slot at `0x6C0xxxx` (sprite headers and palettes like `.7/.8`'s), so
  it repeats as `stcc`'s small pairs do. Nothing executes or reads above
  `0x100000` in program space. With this layout the memory test passes
  IC15, 16, 11, 12, 9, 10, 7, 8, 5 and 6, which it checks in that order.
  `romset::Role::MainDataHigh` holds the second bank; the bus maps it at
  `0x6000000`-`0x7FFFFFF` only for games that have it. *Unconfirmed:*
  what the board decodes at `0x4000000`-`0x5FFFFFF` and above
  `0x7000000` (nothing reads there).
- *Confirmed:* polygons are four pairs (`.17/.21`, `.18/.22`, `.19/.23`,
  `.20/.24`, 32 MB), textures two (`.27/.25`, `.28/.26`), copro data
  `.29/.30`, samples `.32/.34`; `epr-19967.31` is CRX_drv Ver3.4.8 (97
  8/20), and `make sound GAME=segawski` plays a run's commands.
- *Confirmed (by running it):* it needs no drive or music board. It never
  waits on the drive board registers; before a run it shows "the foot
  controller is being unlocked, take care", the output test's SLIDE LOCK.
- *Confirmed (from the input test, then by scripted runs):* port
  `0x1C00002` bits 0/1 coins (one credit is two coins by default; the test
  shows one COIN CHUTE), 2 test, 3 service, 6 SELECT(DOWN); port
  `0x1C00004` bit 0 SET (starts a game, confirms), 1 SELECT(UP), 2 PITCH
  LEFT, 3 PITCH RIGHT; analog channel 0 SLIDE (`0x10` swings the skier
  left, `0xF0` right). *Unconfirmed:* what the pitch switches do; the game
  shows a "jump!" prompt with a foot-tilt icon at ramps.
- *Confirmed (by running it):* coin-start reaches player select, course
  select (Palm Beach, Au-Pau River, Whale Park) and a run at Palm Beach;
  the attract demos ski in 3D. `make state-check GAME=segawski`
  round-trips, `make lockstep GAME=segawski` runs 300M instructions
  identical, and a coin-start run of 600M instructions gives the same
  state hash (`505a972b9e8a3c89`) recompiled and interpreted.
- *Confirmed (from the code, then by running it):* the memory test's
  IC55 and IC54 are 64 KB RAMs at `0x10A00000` and `0x10C00000` (see
  "What's known about the Model 2C board"); with them modelled every chip
  in the test reads GOOD.

## What's known about `waverunr`

Fifth game, 2C: a ride-on personal watercraft cabinet, SCSP sound only.

- *Confirmed:* one program pair, `epr-19282a.15`/`epr-19283a.16` (`.15`
  low half: the i960 fault strings read in order). Boot record: PRCB
  `0xB0`, first IP `0x5F0`, interrupt table `0x44C70`, system-procedure
  table `0x1C0`. `epr-19284.31` is CRX_drv Ver3.2.7 (96 9/10).
- *Confirmed (from the code):* main data is `.11/.12` at `0x2000000` and
  `.9/.10` at `0x2800000` (8 MB slots, as in `stcc`): the course tables
  at program `0x8E1B0` point into `0x2C00000-0x2C1FFFF`, and the record
  chains there (a count, that many 32-byte records, until a zero count)
  parse 4 MB into `.9/.10` and nowhere else.
- *Confirmed (from the code):* `.7/.8` (1 MB) is read through a window at
  **`0x06000000`**, the same second data bank as `segawski`'s. The tables at
  program `0x90940` point into `0x6480000-0x64EFFFF`; the pair is blank
  below `0x80000` and holds data at `0x80000-0xEFFFF`, and the same record
  chains parse there with the pair repeating in an 8 MB slot from
  `0x6000000` (as `stcc`'s 1 MB pairs do in theirs). Without it the game
  read nothing there (unmapped) and still ran. Remodel calls these chips
  `Role::MainDataHigh`. *Unconfirmed:* how the board decodes the rest of
  `0x04000000-0x07FFFFFF`; nothing else there was read in our runs.
- *Unconfirmed:* the polygon pairs are `.17/.21`, `.18/.22` and the 1 MB
  `.19/.23`; textures `.27/.25` and the 1 MB `.28/.26`; copro data the
  1 MB `.29/.30`. Roles and slot order follow the other 2C sets; the game
  draws correctly with them, but byte order within the 1 MB pairs is by
  analogy only.
- *Confirmed (from the code):* the game copies routines from program ROM
  `0xC0000-0xC7FFF` to work RAM `0x210000` and runs them there (calls to
  `0x217090`, `0x21E1F0`, ...). The recompiler leaves them to the
  interpreter.
- *Confirmed (from the code):* the cabinet board (the body's air
  compressor, valves and motion; boot messages "now cabinet initialize",
  "compressor starting", "marine jet home position", ...) is optional. At
  boot the game reads its status byte from `0x1C00006` and switches on
  `0xC6-0xCF`, `0xE0`, `0xE1`; `0xFF` (nothing attached) skips the whole
  sequence and marks the board absent. With the board absent the input
  test shows "SAFETY SENSOR ERROR" and "EMERGENCY" isn't checked (with a
  board, they'd be `0x1C00006` bits 4 and 6). Nothing is modelled.
  *Unconfirmed:* what `0x1C00008` writes drive (lamps or valves,
  presumably).
- *Confirmed (by running it):* no link reads in a default-settings run
  (two writes to `0x1A14000`). The program has a "net check command"
  string; whether a link setting exists is unconfirmed.
- *Confirmed (from the input test, then by scripted runs):* port
  `0x1C00002` bit 0 coin (three coins a credit by default), 2 test, 3
  service, 6 start, 7 view (the camera seemed to move closer once;
  unconfirmed). Analog channels: 0 handlebar (0x30 steers left, 0xD0
  right; it also moves the course selection), 2 throttle lever (0xFF
  released, 0x00 fully open, about 100 km/h; it also confirms the
  course), 1 the body's roll and 3 its pitch. The handlebar, roll and
  throttle go through calibration tables built from the test menu's
  settings (`0x45668` on). Pitch is read only by the test menus. Roll is
  read in play (a lean more than `0x20` from centre sets a state in the
  craft's physics), but scripted runs leaning both ways showed no
  effect. *Unconfirmed:* what roll does.
- *Confirmed (by running it):* `make capture GAME=waverunr` (coin-start:
  three coins, start, the throttle to pick the first course and held
  open) races the beginner course through a checkpoint until time runs
  out and "GAME OVER". `make sound GAME=waverunr` plays the run's sound
  commands on all 32 SCSP slots, about -30 dBFS through the race. `make
  state-check GAME=waverunr` round-trips, `make lockstep GAME=waverunr`
  runs 300M instructions identical, and a coin-start run of 600M
  instructions gives the same state hash (`97d498bed77503a6`, with an
  `eeprom.bin` from an earlier run) recompiled and interpreted.
- *Confirmed (from the raster stream):* under the rock arch on the
  beginner course the game draws its shadow as one flat dark-blue polygon
  (palette colour 5-bit (0, 0, 6), full luma) over the water and behind
  the craft, with texture header bit 15, the checker (screen-door) flag,
  set. Both renderers applied the checker only to textured polygons, so
  the shadow came out solid and the water flat dark blue; it now drops
  every other pixel and the water shows through. *Unconfirmed:* how it
  looks on the hardware's display (a half-tone, or blended by the
  monitor).

## What's known about `overrev`

Sixth game, 2C, and the first not by Sega: Jaleco's, with its own sound
driver. A sit-down driving cabinet; no extra boards are needed.

- *Confirmed (from the code):* `overrev.zip` holds two program pairs.
  `epr-20124a.15`/`epr-20125a.16` is the 2C one: it carries the MB86235
  geometrizer and copro code the other 2C programs share (17 of 37 and 22
  of 128 32-byte windows of `topskatr`'s uploaded programs match, as for
  the other 2C sets), while `epr-19992b`/`19993b` (the 2B clone
  `overrevb`'s) matches none. Both have the same strings and "Sep 10 1997"
  build dates. Boot record: PRCB `0xB0`, first IP `0x690`, interrupt table
  `0x1690`, system-procedure table `0x1C0`.
- *Confirmed (from the code):* main data is `.11/.12` at `0x2000000` and
  `.9/.10` at `0x2800000`: of the 2,826 program words pointing into
  `0x2000000-0x2FFFFFF`, 92 land on filler (`0xCC`/`0xFF` runs) in that
  order and 434 the other way round. No copro data ROMs, no second data
  bank. *Unconfirmed:* textures `.27/.25` and polygons `.17/.21` follow
  `topskatr`'s order; the game draws correctly with them.
- *Confirmed (from the code):* every boot runs a network check ("CHECKING
  NETWORK NOW"), whatever the link setting, as a state machine copied to
  work RAM (from program `0x6D6A0`). It reads the communication board at
  `0x1A14000`: bit 0 set means no board. With no board and the link
  setting stand-alone (the default: the setting byte at `0x590D16` is 0,
  from the EEPROM) it then waits for bit 7 of `0x1A14002` to be set and
  goes on "STAND ALONE"; with no board and a linked setting it stops on
  "Nothing communication board!". With a board it writes 1 to `0x1A14000`,
  waits for bit 7 of `0x1A14002` to clear, and starts the link. Remodel
  reads the board's registers as all ones (no board), which the game takes
  as a stand-alone cabinet; reading 0 (as before) left it waiting at "Link
  check." forever. *Unconfirmed:* that an empty slot reads as all ones on
  the real board (pull-ups); it's the reading the game's code expects.
- *Confirmed (by running it):* no drive board: `0x1C00006` is never read.
  The game writes `0x1C00008` every frame. *Unconfirmed:* what (lamps,
  presumably).
- *Confirmed (from the input test, then by scripted runs):* port
  `0x1C00002` bit 0 coin (one coin a credit by default), 2 test, 3
  service, 6 start; port `0x1C00004` bit 0 view (cycles chase, cockpit and
  bumper cameras), 4 shift up, 5 shift down (they change gear with the
  manual gearbox). VIEW2 is listed but unused ("---"). Analog channels: 0
  wheel (0x30 steers left, 0xD0 right; it also moves the menu selection),
  1 gas and 2 brake, both 0xFF released and 0x00 fully down (the input
  test shows the raw value). A gas press (release to down) confirms each
  menu: mode, car, gearbox (AT or MT, chosen with the wheel).
- *Confirmed (from the code, then by running it):* the sound program
  `epr-20002.31` is Jaleco's own 68000 driver, not CRX_drv; it needs
  nothing beyond what Sega's drivers use (`docs/hw/sound.md` §6). The
  i960 sends the same 3-byte `A0 xx yy` messages over the link, and one
  `0xFF` at init, which restarts the driver as it does CRX_drv. Music is
  streamed PCM from the sample ROMs like the effects; there is no music
  board.
- *Confirmed (by running it):* `make capture GAME=overrev` (coin-start:
  one coin, start, the gas pedal to confirm challenge cup, the first car
  and AT, then held down) races the first course until time runs out.
  `make sound GAME=overrev` plays the run's commands on all 32 slots,
  about -37 dBFS RMS through the race with the music and engine. `make
  state-check GAME=overrev` round-trips, `make lockstep GAME=overrev` runs
  300M instructions identical, and a coin-start run of 600M instructions
  gives the same state hash (`d41686f041683f4c`, with an `eeprom.bin` from
  an earlier run) recompiled and interpreted.
- *Unconfirmed:* steering, braking and the AI cars' behaviour were only
  checked as far as a scripted run shows; the car stays last when
  nothing steers it.

## What's known about `overrevb`

Ninth game and the first Model 2B one: Over Rev's 2B version, a clone of
`overrev` in MAME's naming. Its chips are in `overrev.zip` (or an
`overrevb` folder or zip of its own); the manifest's `parent` says so.

- *Confirmed (ROM):* only the program pair differs from `overrev`
  (`epr-19992b.15/.16`); data, polygon, texture and sound chips are the
  same files, in the same order, and the controls are the same.
- *Confirmed (from the code):* its i960 program is a 2B build of the same
  game (same strings, "Sep 10 1997"), with the 2B uploads, UART address and
  copro boot described under Model 2B. It reads the controls, EEPROM and
  link board as `overrev` does.
- *Confirmed (by running it):* `make run GAME=overrevb INPUT=coin-start`
  attracts, coins up, goes through the mode, car and gear menus and races
  the first course with textured 3D that matches `overrev`'s at the same
  frames (sampled every 300) through the menus and into the race; in the
  unsteered race the two drift apart after frame 3,300 (the 2B car loses
  speed in the next curve and falls behind), as two builds of the physics
  can.
  *Unconfirmed* whether the copro maths differs from 2C's anywhere it
  matters. The attract demo runs about 20 frames behind `overrev`'s.
- *Confirmed (by running it):* `make sound GAME=overrevb` plays the run's
  commands, about -37 dBFS RMS through the race as for `overrev`. `make
  state-check GAME=overrevb` round-trips (save states hold both SHARCs),
  `make lockstep GAME=overrevb RECOMP=1` runs 300M instructions
  identical, and a coin-start run of 600M instructions gives the same state
  hash (`ee2bcd89e419e428`, with an `eeprom.bin` from an earlier run)
  recompiled and interpreted.
- *Speed:* the interpreted geometrizer SHARC takes about 20 ms a race
  frame natively (33 ms in WebAssembly), so races run below real time
  (0.86x natively, about half in a browser) and the geometrizer is what
  limits it. The game slows to its pace rather than the polygons falling
  behind the inputs; full speed needs a SHARC recompiler
  (`docs/hw/model2b.md`, "Speed").
- *Confirmed (by running it, `make latency`):* the game reads the wheel
  at the start of each frame and its display list and polygons follow in
  the same frame, on 2B as on 2C: one frame from input to screen at full
  speed in the native player, two in the browser.

## What's known about `sgt24h`

Tenth game and the second on Model 2B: Jaleco's Super GT 24h, a sit-down
driving cabinet. Its i960 program is its own; its two SHARC programs are
`overrevb`'s code, and so are its board's needs, with two exceptions
below (the register cache and the I/O type).

- *Confirmed (ROM):* program `epr-19155.15`/`epr-19156.16` (`.15` low),
  build dates "Jul 5 1996" and "Jul 6 1996"; the title screen shows no
  copyright line, so the manifest's year 1996 is the build's. Main data
  `.11/.12` at `0x2000000` and `.9/.10` at `0x2800000`: of the 1,737
  program words pointing into `0x2000000-0x2FFFFFF`, 238 land on filler in
  that order and 1,394 the other way round. Textures `.27/.25` and
  polygons `.17/.21` (4 MB chips, a pair to a slot) draw correctly in that
  order. One sample ROM (`.32`), no copro data, no music board. The
  memory test passes ("ROM GOOD", "RAM GOOD").
- *Confirmed (ROM):* the sound program `epr-19157.31` is Jaleco's driver:
  its first 32 KB are byte for byte `overrev`'s `epr-20002.31`. It runs
  unchanged; a coin-start race's `make sound` averages -37.7 dBFS RMS
  (peak -19.1), as `overrev`'s does, so the manifest gives it the same
  +6 dB.
- *Confirmed (by running it):* the i960 uploads the copro and geometrizer
  SHARC programs exactly as `overrevb` does, and they reach the same code
  (2,111 and 2,987 words, `remodel sharc-check` on both games' images). The
  images differ past the code (see Model 2B), so the recompiled crates
  aren't shared: `sgt24h` gets its own two `sharc_<hash>` crates with the
  same code in them.
- *Confirmed (from the code):* boot needs the i960's local-register cache
  (Model 2B above); with it the game boots to "CHECKING NETWORK NOW",
  "STAND ALONE" and attract. The network check is `overrev`'s: it probes
  the communication board at `0x1A14000` (bit 0 set: no board) and with
  the default link setting ("NOT LINK") runs stand-alone.
- *Confirmed (from the code, then by running it):* the GAME ASSIGNMENTS
  menu's I/O TYPE (A to E) is a 16-bit word at backup SRAM `0x1D0000A`.
  Unlike the other settings, which the game resets when the SRAM's
  `0xAD85` signature is missing, it's never initialised, and the I/O code
  uses it unchecked as an index into an 8-byte-entry table of pedal
  scalings (program `0x5EB80`): type A (0) passes the gas and brake
  through, B-E (1-4) take `0x100 - raw`. Types B and D also list a VIEW 2
  button and send an output byte to I/O `0x1C00008` (in place of
  `0x1C0000A`). With the SRAM's power-on `0xFFFF` the index runs past the
  table into code and the pedals come out as `raw × 12 + 2` (the input test
  showed `00F6H` for a released pedal), so the gas never registered. The
  menu shows any value above 4 as A, so a fresh board looks set to A but
  isn't; the manifest's new `backup_ram` field puts 0 there at power-on,
  as an operator saving type A would. *Unconfirmed:* which cabinet each
  type is for, and that A is the standard one (it's the one whose pedals
  read as `overrev`'s do).
- *Confirmed (from the input test, then by scripted runs):* port
  `0x1C00002` bit 0 coin (one coin a credit), 2 test, 3 service, 6 start;
  port `0x1C00004` bit 0 view (cycles the camera), 4 shift up and 5 shift
  down (sequential: 1-2-3-4 and back with the manual gearbox). The game
  reads bits 4-7 of that port as a lever position (`0x10`, `0x20`,
  `0x40`, `0x80`, none: 0-4); the input test shows only the first two.
  Analog channels: 0 wheel (0x30 left, 0xD0 right; it moves the menu
  selection), 1 gas and 2 brake, 0xFF released and 0x00 down with type A.
  The game calibrates the pedals and wheel from the extremes it sees
  (and has a calibration screen, "MAX MIN CENTER"). A gas press confirms
  the course, car and gearbox (AT or MT, chosen with the wheel).
- *Confirmed (by running it):* no drive board. There is a STEERING FORCE
  setting, but with type A the game writes no drive command; the outputs
  it writes each frame go to `0x1C0000A` (*unconfirmed*: lamps).
- *Unconfirmed (ROM):* `epr-18643a.7`, on another board, is a Z80
  program (it starts with a jump, uses IN/OUT ports `0x00`-`0x70` and RAM
  at `0x8000` up; 1.8 KB of 128 KB used) whose header reads "Copyright
  SEGA ENTERPRISES, LTD. 1995", "$Id: 837-10074.s ...", "AM Hardware
  R&D". The same chip is in `stcc` and `von`, both linkable games, and
  the i960 program never reads it, so it's most likely the
  communication (link) board's firmware. The game needs nothing from it:
  with no board it runs stand-alone.
- *Confirmed (by running it):* `make run GAME=sgt24h INPUT=coin-start`
  attracts, coins up, goes through the course, car and gear menus and
  races until time runs out, then shows GAME OVER and returns to attract.
  `make state-check GAME=sgt24h` round-trips, `make lockstep GAME=sgt24h
  RECOMP=1` runs 300M instructions of a coin-start run identical (i960
  and both SHARCs recompiled), and a coin-start run of 600M instructions
  gives the same state hash (`282c8d6fd5151c21`, with or without an
  `eeprom.bin` from an earlier run) recompiled and interpreted.
- *Speed (by running it):* recompiled, `make bench GAME=sgt24h RECOMP=1`
  runs at 1.6x real time natively (the geometrizer SHARC is the limit,
  slowest frame 24.5 ms), so it plays at full speed as `overrevb` does;
  interpreted it races below real time.
- *Unconfirmed:* steering and braking were only checked as far as a
  scripted run shows (the unsteered car ends against the wall).
- *Confirmed (from the raster stream):* the shadow under the player's
  car in a race is a set of flat dark-grey polygons (palette colour
  (2, 2, 2)) with the checker bit set (header `80C0`), so they draw as a
  screen-door half-tone over the track, as `waverunr`'s arch shadow does.

## What's known about `dynabb97`

Eleventh game and the third on Model 2B, the first of Sega's own there:
Dynamite Baseball '97, a two-player baseball cabinet with a bat lever.

- *Confirmed (ROM, then the memory test):* program `epr-19833a.15/.16`
  then `.13/.14`; main data `.11/.12` at `0x2000000`, `.9/.10` at
  `0x2800000`, and a second bank `.7/.8` and `.5/.6` at `0x6000000`, as
  in `hotd`. The test menu's MEMORY TEST shows every ROM (IC 5-16) and
  RAM GOOD in that layout. Polygons `.17/.21` then `.18/.22` and
  textures `.27/.25` draw correctly. Four sample ROMs (`.32-.35`), no copro
  data, no music board. The boot warning says "used only in Japan"; the
  title screen says ©SEGA 1997.
- *Confirmed (by running it):* the board model runs it unchanged: the
  i960 uploads both SHARC programs as the Jaleco games do, the geometrizer
  code is theirs and the copro program is Sega's own (Model 2B above). No
  protection device: nothing is read outside the known map. Boot writes
  zeros once over `0x600000-0x62FFFF`, which Remodel doesn't map and the
  game never reads back (*unconfirmed* what is there).
- *Confirmed (from the input test, then by scripted runs):* port
  `0x1C00002` bit 0 coin 1, 1 coin 2, 2 test, 3 service, 4 start 1, 5
  start 2 (one coin a credit); port `0x1C00004` player 1 and `0x1C00006`
  player 2, each bit 0 SW #1, 1 SW #2, 4 down, 5 up, 6 right, 7 left.
  SW #1 confirms the team and the starting pitcher. BAT VOLUME is analog
  channel 0 (player 1) and 1 (player 2): the input test shows 0 for
  `0x50` and below, `0xFF` from about `0xC0`, `0x6D` at `0x80`. Pulling
  the bat to `0xC0` and letting it back to `0x50` swings; holding it
  doesn't. *Unconfirmed:* that the lever rests at the low end (inferred
  from that), and what SW #2 does.
- *Confirmed (by running it):* `make run GAME=dynabb97 INPUT=coin-start`
  attracts, coins up, goes through team and pitcher select and plays
  innings (batting with strikes, balls and outs, then pitching and
  fielding). `make sound` of the run averages -34 dBFS RMS (peak -13),
  between `hotd`'s and `skisuprg`'s (same driver, CRX_drv 3.2.8), so the
  manifest adds no gain. `make state-check GAME=dynabb97` round-trips,
  `make lockstep GAME=dynabb97 RECOMP=1` runs 300M instructions of a
  coin-start run identical (i960 and both SHARCs recompiled), and a
  coin-start run of 600M instructions gives the same state hash
  (`69e50b78363de101`, with an `eeprom.bin` from an earlier run)
  recompiled and interpreted.
- *Speed (by running it):* that run plays at 2.74x real time recompiled
  natively with the SHARC block compiler (1.86x before it, when the
  geometrizer SHARC was the limit), and at about real time interpreted
  (slowest frame 32 ms).

## What's known about `doa`

Dead or Alive (Tecmo, 2B, Japan), the twelfth game and the fourth on
Model 2B. It boots, attracts with 3D, coins up and fights on the board
model as it is, once the i960 returns keep PFP's address bits 4 and 5
(Model 2B above), and plays on once its protection device is answered:

- *Confirmed (from the code, then by running it):* at boot (i960
  `0x5A0`) the game polls `0x1D8400C` up to 10,000 times for `0xFFF0`.
  If it sees it, it sends a command, a block of data and two tables to
  the device at `0x1D80000` to `0x1D87FFF` (i960 `0x29F0`, its only code
  that touches the device), reads an answer back from `0x1D87FF8` and
  compares it with the 48 bytes at ROM `0x2810` (its copyright and
  version line), keeping the result at `0x54FABC`. With no device the
  result is 0, and starting a game (i960 `0x3558`) then arms a random
  countdown at `0x54F3B0` (300 to 1,199 frames; 892 in a coin-start
  run). When it runs out (i960 `0x2FA4`) the game kills its own tasks:
  the fight freezes with only the background drawn. A checksum of the
  boot code that makes the check (i960 `0x520`, result at `0x54FAB0`)
  arms the same countdown when it fails. The interface, step by step, is
  in `docs/hw/model2b.md`, "The protection device".
- *Confirmed (from the ROM data):* the two tables are a canonical Huffman
  code whose symbols are the characters of that ROM line, sorted by how
  often they occur in it; but the block isn't that code as it stands, so
  the device does something more to it first. *Unconfirmed* what: one
  request is all there is to go on. The board model therefore doesn't
  compute the answer. The manifest names the ROM bytes the game compares
  it with (command 6: 48 bytes at `0x2810`), read from the ROM at run
  time, and the device streams those back. That answers the game's check
  without patching it; it doesn't model the device for any other input.
- *Confirmed (by running it):* with the check answered `0x54FABC` is 1,
  the countdown never arms, and a coin-start run (`make run GAME=doa
  INPUT=coin-start`, punch and kick every half second) plays 6,200
  frames: character select, a won bout against the first opponent over
  several rounds with K.O.s, replays and "GET READY", then the next
  opponent, 4,700 frames past where it used to freeze. Sound plays
  throughout; `make sound` of that run averages -30 dBFS RMS (peak -11),
  louder than the games at 0 dB, so the manifest takes 3 dB off. `make
  state-check GAME=doa` round-trips, and a coin-start run of 600M
  instructions gives the state hash `78f308b4da95cefb` (with an
  `eeprom.bin` from an earlier run; `ba5d806c8790a9a2` before the
  polygon ROM repair below) recompiled and interpreted. `make
  lockstep GAME=doa RECOMP=1 INPUT=coin-start` runs 600M instructions
  identical (i960 and both SHARCs recompiled). Its geometrizer code is
  the other 2B games' (2,987 words, the same crate); its copro program is
  its own (3,261 words, a crate of its own).
- *Speed (by running it):* that run plays at 2.69x real time recompiled
  natively (the geometrizer on its thread, slowest frame 14.3 ms), and at
  0.82x interpreted (slowest frame 50.3 ms).
- *Confirmed (by running it):* the input test gives port `0x1C00002` bit
  0 coin 1, 1 coin 2, 2 test, 3 service, 4 start 1, 5 start 2 (one coin a
  credit), and ports `0x1C00004`/`0x1C00006` for players 1 and 2: bit 0
  hold, 1 punch, 2 kick, 4 down, 5 up, 6 right, 7 left. Laid out as
  `dynabb97`'s (program `.15/.16`; main data `.11/.12`, `.9/.10`; a
  second bank `.7/.8`, `.5/.6`; polygons `.17/.21`, `.18/.22`,
  `.19/.23`; textures `.27/.25`; four sample ROMs) its attract demo and
  fights draw correctly; the memory test wasn't run. The Model 2A chips
  also in `doa.zip` belong to the 2A clone `doaa`.
- *Confirmed (from the ROM data):* the set's dump of `mpr-19324.19`, the
  low half of the third polygon pair (`0xC00000-0xDFFFFF` to the
  geometrizer: Kasumi's models, and others'), is bad on data line 9: it
  reads about 60% of that bit's 1s as 0. Its last quarter holds the
  same data as the last quarters of `.22` and `.23`, and differs from
  them in 138,737 of 524,288 words, every one of them bit 9 read 0
  where the others have 1 (none the other way, no address pattern); in
  its first three quarters bit 9 is set in 38-45% of nonzero words,
  its other bits in 58-71%. Floats lose a
  mantissa bit (at most 2^-14 relative), but in a polygon's attribute
  word bit 9 is the link type's high bit: Kasumi drew as scattered
  shards with holes, in fights and on the character select screen (the
  "dark blocks over the face and hair" seen there before), while the
  geometrizer transformed every vertex as the ROM gives it (checked
  against a transform of the ROM data, object by object) and Zack, from
  the first pair, drew right. Bits 23-24 of an attribute word copy bits
  8-9 (`docs/hw/geometrizer.md` §3.3) and come from the intact `.23`:
  linked by them, her meshes are coherent (every polygon's stored normal
  agrees with its vertices' winding, 199 of 199 in her largest head
  model, against 145 of 197 linked by the dump's bits 8-9). The manifest
  declares the bad line (`romset::Dropout`) and loading restores bit 9
  of attribute-shaped words from bit 24 (45,916 words); Kasumi then
  draws whole in fights and on the character select screen. The chip on
  a real board is presumably good (*unconfirmed*: no other dump to
  compare). Hairline cracks of background pixels along some polygon
  edges remain on the large character select models, Jann Lee's from the
  intact first pair too, so they are the rasteriser's, not the dump's
  (*unconfirmed* why; not the near plane below: they stay with it at
  10^-6, on polygons well beyond it).
- *Confirmed (by running it):* the camera's close-ups (attract mode, the
  character select screen) put fighters' arms, heads and bodies less
  than one unit in front of the eye. Remodel's rasteriser clipped at z =
  1, so those parts vanished as if the camera had cut them away. It now
  clips to the view volume with a near plane at 10^-6
  (`docs/hw/geometrizer.md`, Remodel status), and they draw whole.
- *Confirmed (from the raster stream):* fighters' shadows are flat
  (untextured) black polygons, header `00C0 0000 0000 0000` (palette
  entry 0), checker bit clear, so they draw solid black, as sent.
  *Unconfirmed:* how they look on the hardware.

## What's known about `indy500`

Indianapolis 500 (Sega, 1995; the Twin cabinet, Rev A, the newer
revision), the thirteenth game and the fifth on Model 2B, and the
earliest 2B game here.

- *Confirmed (ROM, then the memory test):* program `epr-18598a.15/.16`;
  main data `.11/.12` at `0x2000000` and `.9/.10` at `0x2800000`; the 1
  MB pair `epr-18596.7/.8` in the second data bank at `0x6000000`, where
  it repeats through its 8 MB slot (the program reads it at
  `0x64F4D20`, `0x6488A90` and the like, 4 MB in, so the board decodes
  only the pair's address lines there; *unconfirmed* which address it
  really sits at). The test menu's MEMORY TEST shows every ROM (IC 7-16)
  and RAM GOOD in that layout. Polygons `.17/.21` then `.18/.22` and
  textures `.27/.25` draw correctly; four sample ROMs `.32-.35`. The copro
  data ROMs `.29/.30` are read by the copro SHARC (Model 2B above). The
  zip's other program, `.7/.8` and sound program chips belong to the
  clones `indy500d` (Deluxe) and `indy500to` (the older Twin revision),
  which aren't supported.
- *Confirmed (ROM):* both SHARC programs are its own. The geometrizer's
  (2,983 words of code) is an earlier version of the code the other 2B
  games share (its dispatch table and much of its code differ in
  address), and emits the same rasteriser stream: attract, menus and
  races draw correctly. The copro program (3,937 words) is the only one
  that reads its external bus beyond the FIFOs.
- *Confirmed (from the input test, then by scripted runs):* port
  `0x1C00002` bit 0 coin 1, 1 coin 2, 2 test, 3 service, 6 start (two
  coins a credit by default); port `0x1C00004` bit 0 VIEW1 (zoom in), 1
  VIEW2 (zoom out), 4 shift up, 5 shift down: a sequential shifter, not an
  H gate. Analog channel 0 is the wheel (lower is left), 1 the gas and 2
  the brake, both 0 released and `0xFF` fully down (a held `0xFF` gas
  takes the car to about 250 mph, `0xFF` brake slows it, and a wheel at
  `0x30` turns it left into the infield; the input test shows the raw
  values). The gas pedal confirms on the course and
  transmission screens.
- *Confirmed (from the code, then by running it):* the game writes a
  drive board command to I/O register 4 every frame (`0x01` at rest,
  `0x5x` in a race), and stores what it reads from registers 3 and 4
  without ever using it; the manifest gives it the wheel drive board
  model, which takes the commands. The link board registers at
  `0x1A14000` are written twice at boot and never read; the test menu's
  NETWORK TYPE defaults to STAND ALONE and the game runs stand-alone.
  Nothing else in its game assignments needs setting (no
  `backup_ram`).
- *Confirmed (by running it):* `make run GAME=indy500 INPUT=coin-start`
  boots (about 675 frames of black screen and checks), attracts with 3D
  and the course demos, coins up, picks the oval and AT, plays the
  rolling start and races with the gas held. `make sound` of that run
  averages -35 dBFS RMS (peak -16), with `dynabb97`'s and `hotd`'s, so
  the manifest adds no gain; no CRX_drv version string in its sound
  program (*unconfirmed* which driver it is). `make
  state-check GAME=indy500` round-trips, and a coin-start run of 600M
  instructions gives the state hash `4a1b197bf00149d6` (with an
  `eeprom.bin` from an earlier run; `c75add6c2ab149d6` while the copro's
  bank 2 was a RAM of its own, which the run never touched) recompiled
  and interpreted. `make
  lockstep GAME=indy500 RECOMP=1 INPUT=coin-start` runs 600M
  instructions identical (i960 and both SHARCs recompiled). Its
  geometrizer crate is shared with `fvipers` only, its copro crate its
  own (`docs/recompiler.md`).
- *Speed (by running it):* that run plays at 4.2x real time recompiled
  natively (the geometrizer on its thread, slowest frame 9.3 ms), and at
  1.1x interpreted (slowest frame 35 ms).
- *Confirmed (from the raster stream):* the dark shape under each car is
  an opaque polygon: header `60DA 0064 0808 0A40`, a translucent
  128×256 texture whose car silhouette is texel 0 (texel 15 around it
  transparent), luma table 100 (entry 0 is 1) and grey palette colour
  (22, 22, 22), so it draws at luma 0, close to black. Its attribute
  word (`0xA01`), luma word and header carry nothing the car and track
  polygons don't, and the checker bit is clear, so both renderers draw it
  solid, as sent. *Unconfirmed:* how it looks on the hardware (no shadow
  mechanism is visible in the data; `docs/hw/geometrizer.md` §5.4).

## What's known about `fvipers`

Fighting Vipers (Sega; the title screen says ©SEGA 1995, the boot
warning "used only in Japan"), the fourteenth game and the sixth on
Model 2B, with the earliest Sega 2B SHARC programs here.

- *Confirmed (ROM, then the memory test, which shows every ROM, IC 5-16,
  and RAM GOOD):* program `.15/.16` at `0x0` and `.13/.14` at
  `0x40000` (128 KB chips); main data `.11/.12` and `.9/.10`; the 1 MB
  pairs `.7/.8` and `.5/.6` in the second data bank, each repeating
  through its 8 MB slot (the program reads them at `0x64xxxxx` and
  `0x6Cxxxxx`, 4 MB in, as `indy500` does; *unconfirmed* where they
  really sit). The SHARC programs are uploaded from `.7/.8` like any data.
  Polygons are three pairs of 2 MB chips, `.17/.21`, `.18/.22`,
  `.19/.23`, **packed** back to back every 4 MB (in 8 MB slots the
  fighters from the second and third pairs drew as large dark shards);
  textures `.27/.25` and `.28/.26` in 8 MB slots (packed, Bahn's coat drew
  with the wrong texture). The copro data ROMs `.29/.30` are the same
  chips as `schamp`'s.
- *Confirmed (from its programs, then by running it):* the geometrizer
  SHARC program is `indy500`'s, word for word (2,983 words of code; the
  same recompiled crate); the copro program is its own (4,267 words). The
  i960 doesn't write MSGR7, so the geometrizer checks every command
  word's tag, and both things that stopped it are now in the board model
  (Model 2B above): the copro writes parts of the display list into
  buffer RAM (fn `0x78`), and the i960 patches list jumps in buffer RAM.
- *Confirmed (from the code and the memory test):* its textures go to
  the upper half of each texture RAM bank (`0x11100000`, `0x11300000`);
  the manifest's `texture_ram_mirrored` makes each bank 1 MB, repeating
  through its window, and everything draws textured (Model 2B above).
- *Confirmed (by the input test, then by running it):* port `0x1C00002`
  bit 0 coin 1, 1 coin 2, 2 test, 3 service, 4 start 1, 5 start 2 (one
  coin a credit); ports `0x1C00004`/`0x1C00006` for players 1 and 2: bit
  0 punch, 1 kick, 2 guard, 4 down, 5 up, 6 right, 7 left.
- *Confirmed (by running it):* `make run GAME=fvipers INPUT=coin-start`
  boots, attracts (character introductions and a fight demo), coins up,
  takes Grace at the character select and fights Bahn in the first stage
  through rounds with sound. `make sound` of that run averages -32 dBFS
  RMS (peak -12), so the manifest adds no gain. A coin-start run of 600M
  instructions gives the state hash `96bd74a28b0455b6` recompiled and
  interpreted (the game never writes its EEPROM in it). `make
  state-check GAME=fvipers` round-trips, and `make lockstep GAME=fvipers
  RECOMP=1 INPUT=coin-start` runs 600M instructions identical (i960 and
  both SHARCs recompiled). Its geometrizer runs `indy500`'s crate, its
  copro program a crate of its own.
- *Speed (by running it):* that run plays at 2.83x real time recompiled
  natively (the geometrizer on its thread, slowest frame 10.1 ms), and at
  0.82x interpreted (slowest frame 34.7 ms).
- *Confirmed (from the raster stream):* character shadows are opaque
  black polygons. On the floor they are translucent 32×32 and 32×64
  textures (headers `6000`/`6008`, palette entry 247 = `0x8000`, black)
  whose texels are only 14 (the shadow) and 15 (transparent); the large
  silhouettes on the walls in the introductions use the plain texture
  `4009 0000 1818` with palette entry 1, black. Nothing in their attribute
  words, luma words or headers sets them apart, and the checker bit
  (which the game does set, on the arena fences) is clear, so they draw
  solid black, as sent. *Unconfirmed:* how they look on the hardware.
- *Confirmed (from the game's code):* the 128 halfwords at `0x12800000`
  are written at boot as the ramp (i + 1) / 2 (i960 `0x474`), and
  otherwise only by a test menu's "TRANS MAP" / "TRANS NUM" items (i960
  `0x57544`), which copy there one of the seven 128-byte luma tables in
  ROM at `0x064266E0` (the same tables the game copies to luma RAM at
  `0x11400000`, i960 `0x12024`). So it is a luma-table-shaped map, left
  at its boot value in play. *Unconfirmed* what the hardware does with
  it; the board model has nothing there, and frames look right without
  it.

## What's known about `schamp`

Sonic Championship (Sega; the title screen says ©SEGA 1996), the
fifteenth game and the seventh on Model 2B. `schamp.zip` also holds the
program and `.7/.8` chips of its clone `sfight` (Sonic the Fighters, as
the dat names it), which isn't supported.

- *Confirmed (ROM, then the memory test: every ROM, IC 7-16, and RAM
  GOOD):* program `.15/.16`; main data `.11/.12` and `.9/.10`; the 1 MB
  pair `.7/.8` in the second data bank, repeating (read at `0x64xxxxx`);
  polygons `.17/.21`, `.18/.22` and textures `.27/.25`, `.28/.26` (4 MB
  chips) in 8 MB slots; copro data `.29/.30`, the same chips as
  `fvipers`'s.
- *Confirmed (from its programs, then by running it):* the geometrizer
  SHARC program is the code the Jaleco games, `dynabb97` and `doa`
  share (2,987 words; the same crate); the copro program is its own
  (4,870 words), built like `fvipers`'s: it reads and writes buffer RAM
  through bank 2 at the same places. Like `fvipers` it doesn't write
  MSGR7, relies on the i960's list jump patches, and uploads its textures
  to the upper half of each texture RAM bank (Model 2B above).
- *Confirmed (from its copro program):* the program clears its internal
  memory early on and then stores a debug word through an index register
  it hasn't set yet, into IOP register `0x20`, which the ADSP-21062
  manual marks reserved; the SHARC core now ignores writes there
  (`docs/hw/adsp2106x.md`).
- *Confirmed (by the input test, then by running it):* the same ports as
  `fvipers`: bit 0 punch, 1 kick, 2 barrier; directions, coins and starts
  as there (one coin a credit).
- *Confirmed (by running it):* `make run GAME=schamp INPUT=coin-start`
  boots, attracts (the story, the character line-up and a fight demo),
  coins up, takes Sonic and fights Knuckles through rounds and replays
  with sound. `make sound` of that run averages -28 dBFS RMS (peak -8),
  louder than the games at 0 dB, so the manifest takes 4 dB off. A
  coin-start run of 600M instructions gives the state hash
  `8d2b7b4cd55d64a0` recompiled and interpreted (the game never writes
  its EEPROM in it).
  `make state-check GAME=schamp` round-trips, and `make lockstep
  GAME=schamp RECOMP=1 INPUT=coin-start` runs 600M instructions identical
  (i960 and both SHARCs recompiled). Its geometrizer runs the shared
  crate, its copro program a crate of its own. The recompiled build first
  faulted at boot: code discovery had made an i960 block at `0x20E`, an
  odd address, which took the slot of the buffer RAM clear at `0x20C`;
  `remodel recomp` now drops unaligned instructions
  (`docs/recompiler.md`).
- *Speed (by running it):* that run plays at 2.71x real time recompiled
  natively (the geometrizer on its thread, slowest frame 10.1 ms), and at
  0.78x interpreted (slowest frame 59.1 ms).
- *Confirmed (from the raster stream):* character shadows are opaque
  black polygons that share the characters' own texture header (`4009
  0000 1A00`, an all-15 64×64 texture that the palette colour tints),
  with palette entry 1 (`0x8000`, black) and a fixed luma (205); the
  checker bit is clear and nothing else sets them apart, so they draw
  solid black, as sent. *Unconfirmed:* how they look on the hardware.
- *Confirmed (from its tile RAM):* the story's white captions ("REVENGE
  OF DR. ...") are one line of text longer than a tile map, on plane A in
  left/right split mode with line scroll: the game scrolls the line-scroll
  entries 8 pixels a frame across both maps. They drew with gaps
  ("REVENG...ONIC") because we took the entry as the split point only;
  the tile chip scrolls both maps by it too, as one 1024-pixel plane
  (`docs/hw/tilemap.md`). *Unconfirmed:* not compared with hardware.

## What's known about `hotd`

The House of the Dead, a two-player light-gun game on 2C. The set is the
Japanese release (the boot warning says "used only in Japan"); the title
screen says ©SEGA 1996.

- *Confirmed (from the game's memory test):* program `.15/.16` at `0x0`
  (`.15` low) and `.13/.14` at `0x100000`; main data `.11/.12` at
  `0x2000000` and `.9/.10` at `0x2800000`; the second data bank, `.7/.8`
  at `0x6000000` and the 1 MB pair `.5/.6` at `0x6800000` (as in
  `segawski`). The memory test checks IC15, 16, 13, 14, 11, 12, 9, 10, 7,
  8, 5 and 6 in that order and passes them all. Its RAM IC54 and IC55
  are the 64 KB RAMs the board model now maps (see "What's known about the
  Model 2C board"; it reported them BAD before they were mapped).
- *Confirmed (from the code):* both MB86235 programs come from `.9/.10`,
  not the program ROM: the boot uploads the geometrizer's from
  `0x2F40000` (code at `0xA1C34`) and the copro's from `0x2F48000`
  (`0xA36B8`), 4 MB into that pair. Nothing about the boot path is
  special beyond that; the shared upload code just reads a different
  address.
- *Confirmed (by running it):* polygons are four pairs (`.17/.21`,
  `.18/.22`, `.19/.23`, `.20/.24`), textures two (`.27/.25`, `.28/.26`),
  copro data the 512 KB `.29/.30`, samples `.32/.34`; `epr-19720.31` is
  the sound program (CRX_drv 3.2.8 per `docs/roms.md`). The game draws
  correctly and `make sound GAME=hotd` plays a run's commands on the SCSP.
  No music board.
- *Confirmed (from the code):* boot waits on a **gun interface** behind
  I/O registers 9-13 (the upper byte lanes of `0x1C00010`-`0x1C0001C`):
  the game writes a parameter to `0x1C00014`, a command to `0x1C00012`,
  waits for `0x1C0001A` bits 2 and 3, then reads the reply from
  `0x1C00018` (and reads `0x1C00016`, unused). Commands below `0x80` write
  a device register (low bits), `0x80` and up read one back; register 0
  selects a counter, command `0x87` reads it. At boot (`0xA3750`) the game
  sets registers 1 and 8, toggles bit 0 of register 1, then streams 5 KB
  from program ROM `0xA3A00-0xA4DFF` through command `0x07`: a
  configuration image, presumably for programmable logic that times the
  guns (*unconfirmed* what the part is). Without a device answering, boot
  spins at `0xA3728` forever. Remodel models only this (`model2::gun`),
  for games with guns in their manifest.
- *Confirmed (from the code, then the GUN SETTING test screen):* each frame
  `0xA38C0` reads counters 0-7 as four 10-bit values (a low byte, then two
  bits from the next counter): player 1's **Y** in 0/1 and **X** in 2/3,
  player 2's Y in 4/5 and X in 6/7 (the GUN MARK CHECK screen shows them as
  IP1X/IP1Y and IP2X/IP2Y). Counter 8 holds one bit a player (bit 0
  player 1, bit 1 player 2), read inverted into `0x51EF08` and `0x51EF24`:
  **1 means the gun sees no screen**. Reloading needs that bit: with the
  coordinates zeroed but the bit clear, an off-screen shot doesn't reload
  and the game shows its "shoot outside the screen" hint. *Unconfirmed:*
  what the real board does with the counters while a gun sees nothing
  (Remodel reports 0).
- *Confirmed (from the test screens):* the game calibrates the guns
  itself (GUN SETTING: adjustment, default adjustment, manual
  adjustment). The defaults in a fresh EEPROM are X 173-596 (player 2:
  163-596) and Y 87-380, which the game maps to points about 32 pixels in
  from the view's edges. Measured on GUN MARK CHECK, the raw ranges that
  put the mark on the aimed pixel (to within a pixel) are X 141-629
  (player 2: 131-630) and Y 56-410 over the whole 496x384 view; these are
  in the manifest. A cabinet recalibrated in the test menu would want
  other values.
- *Confirmed (from the input test, then by scripted runs):* port
  `0x1C00002` bits 0/1 coin chutes, 2 test, 3 service, 4 START1, 5
  START2; port `0x1C00004` bit 0 TRIGGER1, bit 1 TRIGGER2. Nothing else
  on the ports or analog channels is used (the shared input routine reads
  the analog channels but the game ignores them). Two coins a credit by
  default ("2 CREDITS TO START").
- *Confirmed (by running it):* the coin-start script plays: it aims at
  the middle and either side in turn, fires every 20 frames and reloads
  off screen every seventh shot; shots land where aimed (impact sparks at
  the aim point, zombies hit), and the run gets through the first two
  areas before the game over. `make state-check GAME=hotd` round-trips,
  `make lockstep GAME=hotd` runs 300M instructions identical, and a
  coin-start run of 600M instructions gives the same state hash
  (`0473295c9891af45`, with an `eeprom.bin` from an earlier run)
  recompiled and interpreted.
- Light gun input is generic (`romset::Gun`): a gun is a position in the
  view (or off screen) plus a trigger button; the manifest says where the
  board reads it (`GunSource::Counters` for this interface), its
  calibration and how it reloads. Players, the browser and `--input`
  (`gun1=x:y@from-to`, `gun1=off@...`) produce positions; see
  `docs/play.md` and `docs/web.md`.

## What's known about `bel`

Behind Enemy Lines, a two-player game on 2C by EPL Productions for Sega:
each player works a gun mounted on a vehicle that drives itself through
the mission. The title screen says ©1996, 1997 SEGA / REAL 3D (the dat
says 1998). The sound program is EPL's (`docs/hw/sound.md` §7).

- *Confirmed (from the code, then the memory test):* program `.15/.16` at
  `0x0` and `.13/.14` at `0x40000` (128 KB chips; the code's pointers run
  straight on); main data `.11/.12`, `.9/.10`, `.7/.8`, `.5/.6` packed at
  `0x2000000`, `0x2400000`, `0x2800000` and `0x2C00000` (tilemaps at
  `0x28xxxxx` and a pointer table at `0x2C00000` show it). With 8 MB slots
  boot read garbage tilemap sizes and copied 13 MB past texture RAM. The
  memory test then passes IC5-16 and every CPU board RAM.
- *Confirmed (by running it):* polygons are four pairs (`.17/.21`,
  `.18/.22`, `.19/.23`, `.20/.24`, low halves `.17`-`.20` by their float
  exponents), packed every 4 MB like the main data; with 8 MB slots
  enemies and buildings came out as huge flat shards. Textures are two
  pairs (`.27/.25`, `.28/.26`) in 8 MB slots (texture addresses reach
  `0x46xxxx` half-words, the second slot); copro data the 4 MB `.29/.30`;
  samples `.32`-`.35`; `.31` the sound program.
- *Confirmed (from the code):* the routine at `0x27030` uploads the copro
  program from program ROM `0x27090`, only when the status at `0x980010` or
  `0x980014` shows bit 0 (see "What's known about the Model 2C board").
  The geometrizer program is the same 4,096 words as `hotd`'s, `skisuprg`'s,
  `segawski`'s and `waverunr`'s.
- *Confirmed (from the code):* sound commands go out a byte per UART
  transmit interrupt (`0x6F930`); with no such interrupt the game ran
  silent.
- *Confirmed (from the input test, then by scripted runs):* port
  `0x1C00002` bits 0/1 coin chutes, 2 **service**, 3 **test** (the other
  way round from most games here), 4/5 START 1/2; port `0x1C00004` bits
  0/1 the guns' triggers, 4/5 their missile buttons ("launch button
  located at front of gun"). The guns are potentiometers on the ADC:
  channel 0 player 1's yaw, 1 player 2's yaw, 2 player 1's pitch, 3
  player 2's pitch (`GunSource::Analog`). One coin a credit by default.
- *Confirmed (from the test menu):* GUN CALIBRATION asks for each gun at
  its lower-left then upper-right stops: yaw grows to the right and pitch
  upwards. With the defaults of a fresh EEPROM the crosshair can't reach
  the view's edges; the manifest's ranges (X −19 to 273, Y 273 to −18,
  clamped to 0-255) put it on the aimed pixel to within a few pixels,
  measured from the crosshair lines at three aims.
- *Seen, unconfirmed:* an `eeprom.bin` written while probing the test
  menu left the guns not moving in game; a fresh one works. Small blue
  specks show through some terrain seams.
- *Confirmed (by running it):* the coin-start script plays the first
  mission to the game over, scoring hits; `make sound GAME=bel` renders
  the commands with music throughout. `make state-check GAME=bel`
  round-trips, `make lockstep GAME=bel` runs 300M instructions identical,
  and a coin-start run of 600M instructions gives the same state hash
  (`25f923c57d7362eb`, with an `eeprom.bin` from an earlier run)
  recompiled and interpreted.
- Players aim with the generic gun controls; the right mouse button (or
  the pad's left trigger / B) fires a missile, as the manifest makes the
  missile button stand in for reloading (`Reload::Button`).

## License

MIT for Remodel's own code ([LICENSE](LICENSE)). `make dist` and `make web`
also ship `THIRD-PARTY-NOTICES.txt` for the crates they link
(`tools/notices.py`). Game software and data remain the property of their
respective owners.
