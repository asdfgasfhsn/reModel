# Using reModel: commands

The full command catalogue. `make help` is always current and is the authority; this page keeps the worked examples.

## Commands

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
make play RECOMP=1        # the launcher: pick a game (live attract previews, settings, Continue), then play
make play GAME=hotd RECOMP=1   # straight into a game (Esc or pad Guide or Start+Select menu, F5/F9 quick save/load, F11 fullscreen)
make play PLAY_ARGS=--smooth   # with in-between frames on a display over 60 Hz (a frame more latency)
make play GAME=sgt24h PLAY_ARGS="--run-ahead 3"   # show the game 3 frames ahead: less input lag, 4x the emulation
make input-lag            # each game's own input lag: frames from a control to the picture changing
make run-ahead-check      # run-ahead keeps the real state hash and shows the plain run's later frames
make install-desktop RECOMP=1   # reModel in the desktop's app menu, running this checkout's player
make smooth-frames GAME=doa    # coin-start run drawing in-between frames beside the real ones -> out/doa/frames
make web RECOMP=1         # browser build -> out/web: game list, a page and module per game (WebGPU; serve over HTTPS, docs/web.md)
make dist                 # player to hand out (interpreter only, no ROMs) -> out/dist/remodel-VERSION-TARGET.tar.gz
make capture              # save renderer inputs from a run into a race -> out/topskatr/capture
make gpu-check            # draw them on the software and GPU rasterisers, time and compare (GPU_OPTS="--aa 4 --depth": with them on)
make depth-check          # draw them on the GPU with the depth buffer off and on, picture the differences -> out/topskatr/depth-check
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

