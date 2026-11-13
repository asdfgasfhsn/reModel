# Using reModel: commands

The full command catalogue. `make help` is always current and is the authority; this page keeps the worked examples.

## Commands

Requires a Rust toolchain (stable) and `make`.

```sh
make                  # list every target
make check            # format check, clippy, unit tests (no ROMs needed); `make ci` is the same
make build            # debug build of the whole workspace
make release          # optimised build of the remodel CLI (RECOMP=1: with the recompiled code)
make test             # unit tests only; make lint (clippy, warnings are errors); make fmt / make fmt-check
make pace-sim         # the browser's audio rate control on synthetic frame timings (docs/web.md)
make clean            # remove build output and everything in out/ (scratch included)
make m68k-tests       # 68000 core vs the SingleStepTests suite (fetches ~200 MB, 1.1 GB unpacked, into out/)
make z80-test         # Z80 core vs the zexdoc/zexall exercisers in out/z80-tests (docs/hw/z80.md)
make sharc-test       # ADSP-21062 (SHARC) tests, plus the 2B programs if captured (docs/hw/adsp2106x.md)

# With ROM sets in ./rom, as MAME keeps them (rom/topskatr.zip, or unpacked in rom/topskatr/):
make games            # the games with manifests
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
make copro-disasm         # disassemble the geometry program the last `run` captured (MB86235)
make tgp-disasm GAME=srallyc   # Model 2A: the TGP (MB86234) program the last `run` captured -> out/srallyc/tgp.lst
make tgp-check GAME=srallyc    # its functions on the MB86234 interpreter with random arguments, against their maths
make sharc-disasm FILE=out/overrevb/sharc-geo.bin   # Model 2B: an ADSP-21062 program image -> FILE.lst
make mpeg-dump ADDR=0x11000 SECONDS=10   # decode MPEG ROM frames directly -> out/topskatr/mpeg-ADDR.wav
make mpeg-test            # MPEG decoder test against the MPEG ROMs in rom/
                          # coin up, start, and play into a race
make run STEPS=485000000 INPUT=coin1@1100-1104,start@1250-1254 \
    VIEW="--height 1080 --aspect 21:9 --hud edges"   # enhanced snapshot
make dump-textures        # race run: every 3D texture and tile drawn -> out/topskatr/texdump/
make upscale-textures     # dump -> texture pack out/topskatr/texpack-4x (built-in resampler; only new dumps)
make upscale-textures UPSCALER='realesrgan-ncnn-vulkan -i {in} -o {out} -s {scale} -n realesrgan-x4plus -g 1 -f png' \
    TEXPACK=out/topskatr/texpack-esrgan   # through an external upscaler (docs/texture-packs.md, Batch upscaling);
                                          # its colour drift is matched back to the dump (COLOUR=keep: off)
make run STEPS=600000000 INPUT=coin1@1100-1104,start@1250-1254 \
    PACK=out/topskatr/texpack VIEW="--height 1080"   # snapshots with a texture pack (docs/texture-packs.md)
make run STEPS=600000000 INPUT=coin1@1100-1104,start@1250-1254 PACK=out/topskatr/texpack \
    VIEW="--height 1080 --tex-filter mip"   # mipmapped pack images (--tex-filter off|mip|2|4|8|16; snapshots draw 2..16 as mip)
make pack-materials TEXPACK=out/topskatr/texpack-4x   # normal maps for a pack from its images, built-in from brightness (STRENGTH 2, DETAIL 8) or NORMALGEN=CMD; a starting point to edit by hand, edits kept on reruns, textures listed in the pack's materials-exclude.txt skipped (docs/texture-packs.md, Material maps)
make pack-lint PACK=out/topskatr/texpack-4x   # check a pack against the pack format, no ROM or GPU: pack.txt, names, index.txt, decoding, sizes, normal maps (specular and gloss maps: a note, no effect yet), memory (TEXDUMP=DIR: also against a dump; docs/texture-packs.md, Pack format)
make play PLAY_ARGS="--pack on --tex-filter 16"   # live in the player: pack from out/GAME/texpack (the browser: ?pack=1&texfilter=16, docs/web.md)
make play PLAY_ARGS="--texture-pack DIR --materials --shading pixel"   # with the pack's material maps (relief 2; --materials-relief off|S; the menu's "Material maps"; browser ?materials=1&relief=S; docs/texture-packs.md, Trying material maps; docs/enhancements.md, Material maps)
make assets               # extract the game's art for review -> out/topskatr/assets (needs ImageMagick)
make bench                # timed run into a race: speed and state hash
make latency              # frames from a wheel change to the game reacting and its polygons (LATENCY_CHANGE, LATENCY_FRAME)
make recomp               # recompile the game's i960 code and TGP or SHARC programs (after a `run`; builds only what changed)
make recomp-workspace     # rewrite out/recomp's Cargo workspace and crate manifests (after updating reModel or moving the checkout)
make bench RECOMP=1       # same run on the recompiled build: same hash, about twice as fast
make state-check          # save state round trip: saved at frame 2000 and resumed must match an uninterrupted run (state hash and audio)
make geo-check GAME=sgt24h   # native geometrizer beside the chip (2B SHARC, 2C TGP) on a coin-start run, raster streams compared each frame, both timed
make geo-fuzz GAME=sgt24h    # random display lists through the chip's program and the native geometrizer, compared (FUZZ_FRAMES, FUZZ_SEED)
make play RECOMP=1        # the launcher: pick a game (live attract previews, settings, Continue), then play
make play GAME=hotd RECOMP=1   # straight into a game (Esc or pad Guide or Start+Select menu, F5/F9 quick save/load, F11 fullscreen)
make play PLAY_ARGS=--smooth   # with in-between frames on a display over 60 Hz (a frame more latency)
make play GAME=sgt24h PLAY_ARGS="--run-ahead 3"   # show the game 3 frames ahead: less input lag, 4x the emulation
make play PLAY_ARGS="--temporal 67"   # temporal anti-aliasing, 3D drawn at 67% and upscaled (taa: full size; --stipple keep; docs/play.md)
make input-lag            # each game's own input lag: frames from a control to the picture changing
make run-ahead-check      # run-ahead keeps the real state hash and shows the plain run's later frames
make run-ahead-profile GAME=srallyc RUN_AHEAD_FRAMES=1   # host time per shown frame: real frame, copy, frames ahead
make install-desktop RECOMP=1   # reModel in the desktop's app menu, running this checkout's player
make uninstall-desktop    # remove that app-menu entry and its icon
make smooth-frames GAME=doa    # coin-start run drawing in-between frames beside the real ones -> out/doa/frames
make web RECOMP=1         # browser build -> out/web: game list, a page and module per game (WebGPU; serve over HTTPS, docs/web.md); missing screenshots made JOBS at a time (default 3)
make dist                 # player to hand out (interpreter only, no ROMs) -> out/dist/remodel-VERSION-TARGET.tar.gz
make capture              # save renderer inputs from a run into a race -> out/topskatr/capture
make gpu-check            # draw them on the software and GPU rasterisers, time and compare (GPU_OPTS="--aa 4 --depth": with them on)
make gpu-check PACK=out/topskatr/texpack-4x GPU_OPTS="--tex-filter mip"   # the same with a texture pack on both renderers (docs/texture-packs.md)
make gpu-check PACK=DIR GPU_OPTS="--materials --shading pixel"   # also draw the pack's normal maps, GPU against software, from captures with light.bin (made with --geo native); counts and memory per frame; --light instead of --materials for the frame to compare with
# `run --texture-pack DIR --materials --geo native` snapshots draw its normal maps in software (a line per snapshot: polygons with a lamp, with a material, pixels changed)
make pack-check PACK=DIR LIGHT_IN=out/scratch/lighting/topskatr/capture GPU_OPTS="--materials --shading pixel"   # also the pack without and with its maps, on captures with light.bin (make lighting-capture): FRAME-mat-off/-mat-on (GPU), -mat-sw (software), -mat-diff (off | on | x8), -mat-cover (green applied, orange no lamp, red degenerate frame, grey none); share of pixels with a material and a lamp, mean and largest change, frame mean (flagged over 2 levels), GPU against software, ms, memory; PROBE=X,Y@FRAME explains a pixel
# --materials also records the side channel (as --light), so compare its snapshots with a run with --light, not a bare one: without either, vertex and pixel shading have no side channel and draw from the stream's lumas
make materials-identity PACK=DIR   # the pack with every normal map made flat must draw with --materials exactly as with --light, under both rules, in flat, vertex, pixel and mipmapped pixel shading (coin-start snapshots, JOBS runs at a time, default 3; MAT_STEPS, MAT_SNAP), and on the GPU (gpu-check and temporal-probe of captures; MAT_GPU_SNAP)
# `--materials-relief off|S` with --materials: how a normal map changes brightness: relief at strength S (default 2, 0.1 to 8; relative to how the flat surface is drawn, so it shows on fully lit scenery too; a quarter to 4 times) or off, the faithful rule (the map's normal in the game's own lighting sum)
make pack-check PACK=out/topskatr/texpack-4x   # draw them with the pack on the GPU unfiltered, mipmapped and 16x, in software and as a supersampled reference; compare -> out/topskatr/pack-check
make tile-check PACK=out/topskatr/texpack-4x   # the pack's tile images live on the GPU against the software snapshots' (1080p 16:9) -> out/topskatr/tile-check
make pack-identity PACK=out/topskatr/texdump   # a pack of the dump itself must draw the captured frames as without one (no PACK: each frame's own textures); splits the differing pixels by cause and fails a frame over 0.003% one-texel/other-polygon pixels, not on filter order (up to 5%) -> out/topskatr/pack-identity
                          # known open item: it exits non-zero on topskatr and sgt24h, where single frames go over the 1% per-frame limit though the means are within it
make depth-check          # draw them on the GPU with the depth buffer off and on, picture the differences -> out/topskatr/depth-check
make depth-check PROBE="672,666@3600" AGAINST=DIR   # follow a pixel through the depth buffer; compare depth-on with an earlier run's (docs/enhancements.md)
make lighting-check       # smooth shading (vertex; GPU_OPTS="--shading pixel": per pixel; add "--shading-fine on" for finer levels, K; "--shading-balance off" without the brightness balance, -nobal, C report-only) on the captured frames: invariants, flat | vertex or pixel | differences and layer pictures -> out/scratch/lighting/topskatr (LIGHT_OPTS="--crease 30 --probe X,Y@F --bench"; GPU_OPTS="--shading vertex" turns it on in gpu-check, depth-check, pack-check; LIGHT_IN=DIR: a lighting-capture, checked with its side channel)
make lighting-capture     # every 300th frame of a native coin-start run with smooth shading's side channel (light.bin, scan.bin) -> out/scratch/lighting/topskatr/capture, and the side channel's self-check over every frame (LIGHT_CAPTURE=DIR)
make spec-stats           # how much specular highlight the game casts, on a lighting-capture (no GPU): textured polygons with a specular term, pixels it brightens by 1 and 4 levels or more, frame | flat term | smooth term pictures -> out/scratch/lighting/topskatr/spec (LIGHT_IN=DIR, LIGHT_OPTS="--every N")
make temporal-probe GPU_OPTS="--temporal 67"   # the GPU's temporal path on the captured frames: the first frame at 100% (stipple keep) must equal the frame with it off, byte for byte; jittered, blend and keep pictures -> out/scratch/temporal-probe/topskatr (default --temporal taa)
./target/release/remodel --game sgt24h --input coin-start temporal-probe live 3000 6 DIR   # live run: the motion vectors from smooth motion, per frame
make temporal-capture GAME=sgt24h SEQ_FROM=2850   # SEQ_FRAMES (300) consecutive frames with their motion data -> out/scratch/temporal/sgt24h/seq (TEMPORAL_SEQ; native geometrizer; INPUT, default coin-start; STATE=FILE to start from a save state; about 0.3 MB a frame)
make temporal-check GAME=sgt24h   # motion vectors (CPU and the GPU's velocity target), then plain, MSAA 4x, TAA and 67%/50% upscaled frames through the sequence against supersampled references (software and the GPU's own), with frame-to-frame change and a still frame -> SEQ/check (report.txt, pictures); fails if the vectors miss or temporal loses to plain/naive (default 1080p 16:9, every 10th frame sampled after a 16-frame warm-up)
make temporal-check GAME=sgt24h TEMPORAL_OPTS="--every 20 --gpu-only --crop 800,400,200,120 --at 3001,3002 --ref-cache DIR --tune fast=0.5"   # every 20th frame; GPU columns only; enlarged crops (gref off msaa4 taa / up67 up50 naive67 naive50, and each column's mean difference from gref inside the crop over the sampled frames in the report); frames 3001 and 3002 sampled too; references kept between runs; resolve constants (gpu::TemporalTuning) for tuning (--no-gpu: CPU half only; --height/--aspect/--hud: another view)
make temporal-check GAME=sgt24h TEMPORAL_OPTS="--inspect --tune cubic=0.5,sharpen=0.3"   # also the error above the plain frame (TAA) or naive (67%/50%) split by the resolve's rule and by whether the clip moved the history, with FRAME-COL-rules.png and FRAME-COL-excess.png (red worse, green better); here with Catmull-Rom history resampling and the optional sharpening (both off the defaults). The report also scores texture interiors ("inside polygons") and their sharpness ("detail": 1 as sharp as the reference, below 1 softer)
make temporal-check GAME=sgt24h TEMPORAL_OPTS="--dump 770,842,16,4"   # the resolve's decisions per pixel in that rectangle on every frame (rules, whether a thin polygon's motion was taken, this frame's share, clip, trust, motion, the colours and the history written, the nearest sample and whether it is on a thin polygon) -> SEQ/check/dump.txt; a fifth number, 67 or 50, dumps that column instead of TAA
./target/release/remodel --game sgt24h --depth temporal-check SEQ   # with the depth buffer in every GPU column and the reference (--depth goes before the command)
./target/release/remodel --game sgt24h [--height 2160] temporal-check SEQ --bench 30   # GPU time a frame: plain, MSAA 4x, TAA, 67%, 50%, and the temporal targets' memory
make lockstep RECOMP=1    # recompiled vs interpreted i960 (and 2B SHARCs), block by block, until they differ
make music                # after a `run` into a race: the music board's music -> out/topskatr/music.wav (GAME=stcc too)
make soundtrack-import SRC=~/my-mix SOUNDTRACK=my-mix   # your own music (ffmpeg converts)
make music SOUNDTRACK=my-mix      # same, with your tracks -> out/topskatr/music-my-mix.wav
```

Custom soundtracks, their `tracks.txt` format and how the game still drives
them are described in `docs/mods.md`.

One debugging command has no `make` target and isn't in the CLI's usage:
`remodel --game ID geo-trace-words WORD...` boots the Model 2C geometrizer
program the last `run` captured (`out/GAME/geo.bin`) on the MB86235
interpreter, raises and drops its flag 0, feeds the hex WORDs into its
input FIFO, and prints the next 60 instructions with the AA0, AA1, AA3,
AR0 and AR1 registers and the FIFO's length after each, to follow how the
program reads a command.

`make bench` prints a hash of all RAM and the last rasteriser stream, and
an `idle skip` line: how many of the instructions the game's idle loops
were fast-forwarded through (`docs/idle-skip.md`). Skipping is exact, so
`--no-idle-skip` (an option of every `remodel` command, and of
`remodel-play`), which runs every pass of those loops, gives the same
hash, more slowly: `./target/release/remodel --game srallyc --no-idle-skip
--input coin-start run 600000000`.

The hash covers all RAM and the last rasteriser stream. A change
meant only to make things faster must leave it unchanged. The hash depends on
`out/GAME/eeprom.bin`, which `run` creates on first boot: a run in a
directory where an earlier run left one loads it (`eeprom loaded` in the
output) and boots differently, so compare runs from the same starting
EEPROM (an empty working directory for a fresh one). For example a
600M-step `topskatr` coin-start run gives `7f0275251269648e` (ip 0x48210)
from a fresh EEPROM and `400275251269648e` (ip 0x48218) when repeated in the
same directory. The geometrizer's thread doesn't change the hash: it runs
each frame on its own copy of the inputs and nothing reads its results
before `sync()`, so the default run and `--single-thread` agree.

Every target takes `GAME=<set>` for any of the eighteen games (`make games`
lists them; the sets are in the README's table and [games/](games/README.md)),
e.g. `make play GAME=stcc`, `make capture GAME=skisuprg`. `GAME=overrevb` is
Over Rev's Model 2B version, its chips in `overrev.zip` as MAME merges them.
The exceptions: `make assets` is `topskatr`'s; `make music` needs a music
board (`topskatr`'s DSB2 or `stcc`'s DSB1); `make tgp-disasm` and
`make tgp-check` are for the Model 2A game (`srallyc`), `make geo-check` and
`make geo-fuzz` for the 2B and 2C games with a native geometrizer.

Variables: `GAME` (manifest id, default `topskatr`), `ROM` (chip directory,
default `rom`), `OUT` (default `out/GAME`), `ADDR`, `COUNT`, `STEPS`, and the
per-target ones `make help` lists (`VIEW`, `PACK`, `GPU_OPTS`, `PLAY_ARGS`,
...). The `Makefile` is the supported interface for humans and agents alike;
`make help` is always current.

