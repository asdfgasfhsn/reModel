# Using reModel: commands

The full command catalogue. `make help` is always current and is the authority; this page keeps the worked examples.

## Commands

Requires a Rust toolchain (stable) and `make`.

```sh
make                  # list every target
make check            # format check, clippy, unit tests, the browser build type-checked (no ROMs needed); `make ci` is the same
make changed          # what a change since origin/main (CHANGED_BASE=...) can affect: crates to test, web-check, lock-test, benchmarks (tools/changed.py)
make check-changed    # make check on the affected crates only (those touched and every crate depending on them); make check stays the full gate
make bench-changed    # bench-all and bench-variants, only when the change can move a state hash (emulator, CLI, renderer, recompiler, benchmark files)
make build            # debug build of the whole workspace
make release          # optimised build of the remodel CLI (RECOMP=1: with the recompiled code)
make lock-status      # who holds the build, test, render and emulate locks and who waits (tools/lock status; docs/usage.md, "Shared machine")
make agent-setup      # a new worktree: link rom/, copy the EEPROMs, make out/scratch/agent-NAME, write out/agent.env (ART=1: pinned binary); prints what is available as JSON
make agent-bin        # build once and keep a read-only remodel in out/bin/COMMIT/ for agents that never build
make lock-test        # tools/lock's own tests (a private lock folder; part of make check)
make agent-env-test   # slice A's acceptance test: beside a build and a test run holding their locks, img and studio answer in under 2 s, parseable, with the right exit codes (no ROM)
remodel img sheet OUT.png IMG.. | diff A.png B.png | stats IMG.png   # contact sheet, pixel difference, image numbers, with --json (no ROM; docs/uplift-studio.md, "Image tools")
remodel games         # the titles, then each variant (the other releases of a title: `--game topskatrj`, `make play GAME=srallycb`; a variant id works wherever a set id does; docs/variants.md)
make test             # unit tests only; make lint (clippy, warnings are errors); make fmt / make fmt-check
make pace-sim         # the browser's audio rate control on synthetic frame timings (docs/web.md)
make clean            # remove build output and everything in out/ (scratch included)
make clean-stale      # delete old crate builds Cargo leaves in target/ (unused 2 days; CLEAN_DAYS=, DRY_RUN=1); current builds kept
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
make link-trace GAME=srallyc LINK=fake,0,2   # experimental: a link board fitted, every access to the link RAM traced (docs/hw/link-board.md)
make link-run GAME=srallyc NODES=2 FRAMES=6000 SHOT_EVERY=100 INPUT0=coin1@1500-1504,coin1@1560-1564   # experimental: two to four real machines in one process joined by a link hub, pictures side by side and the handshake log in OUT (docs/hw/link-board.md, section 9)
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
make pack-log PACK=DIR INPUT=coin-start STEPS=1400000000   # flicker report: draw every frame with the pack at the arcade's pace, log hit or miss and why per texture, tile and region (PACK_LOG, PACK_FIXES=none|all|surface,hold,preload; the player's --pack-log FILE; docs/texture-packs.md, "Flicker"); `remodel pack-flicker LOG` reads one
make pack-lint PACK=out/topskatr/texpack-4x   # check a pack against the pack format, no ROM or GPU: pack.txt, names, index.txt, decoding, sizes, normal maps (specular and gloss maps: a note, no effect yet), memory (TEXDUMP=DIR: also against a dump; docs/texture-packs.md, Pack format)
make play PLAY_ARGS="--pack on --tex-filter 16"   # live in the player: pack from out/GAME/texpack (the browser: ?pack=1&texfilter=16, docs/web.md)
make play PLAY_ARGS="--texture-pack DIR --materials --shading pixel"   # with the pack's material maps (relief 2; --materials-relief off|S; the menu's "Material maps"; browser ?materials=1&relief=S; docs/texture-packs.md, Trying material maps; docs/enhancements.md, Material maps)
make assets               # extract the game's art for review -> out/topskatr/assets (needs ImageMagick)
make bench                # timed run into a race: speed, state hash and frame times (host ms per emulated frame: median, p90, p99, p99.9, worst; a JIT hitch shows here)
make latency              # frames from a wheel change to the game reacting and its polygons (LATENCY_CHANGE, LATENCY_FRAME)
make recomp               # recompile the game's i960 code and TGP or SHARC programs (after a `run`; builds only what changed)
make recomp-workspace     # rewrite out/recomp's Cargo workspace and crate manifests (after updating reModel or moving the checkout)
make bench RECOMP=1       # same run on the recompiled build: same hash, about twice as fast
make bench JIT=1          # same run with the i960 and the copro programs on their JITs (no recompiled build needed): same hash, the JITs' counters (docs/jit.md; --jit-copro off: the i960's alone)
make jit-translate        # the whole i960 program translated by the JIT ahead of time (on its workers), as `make recomp` cuts it, timed
make jit-warm             # warm the JITs' hot-block list without playing: a headless coin-start run writes out/topskatr/jit/hot.txt (addresses only), translated at the next boot (JIT_CACHE=DIR, JIT_WARM_STEPS)
make jit-hitch JIT_HITCH_OPTS="--geo native"   # frame-time hitches of the bench session: interpreter, JIT inline (--jit-sync), JIT cold and warm (docs/jit.md, Pre-flight)
./target/release/remodel --game srallyc --jit --jit-cache out/srallyc/jit --input coin-start run 600000000   # a run on the JITs with the hot-block list in out/srallyc/jit/hot.txt: its blocks translated at boot, the ones it translates added at the end (`jit list` line)
./target/release/remodel --game topskatr --jit --jit-sync --input coin-start run 600000000   # the JIT translating on the thread that needs it, as before the workers (frame times: the hitches it caused)
make state-check          # save state round trip: saved at frame 2000 and resumed must match an uninterrupted run (state hash and audio)
make state-roundtrip      # save state round trips for several sets (topskatr, daytona, srallyc, lastbrnx) saved at frames 700, 1900 and 3300: each must end on the uninterrupted run's state hash
make link-net NET="--host 47890 --nodes 2" GAME=srallyc   # networked link play without a window: this host runs one machine, `NET="--join HOST:PORT"` on the other host (docs/play.md, "Network link play")
make link-browse          # list the network link hosts found on this network (mDNS and broadcast; SECS=3): name, game, players, address, version, whether this machine could join
make link-free-play-test  # free play in linked play (srallyc, daytona, manxtt, indy500, motoraid, waverunr, sgt24h, overrev, overrevb, von, stcc, skisuprg): a two-node race started with start presses only, on the game's own FREE PLAY coin setting, in link-run and over loopback (docs/play.md, "Free play"); `link-run --free-play`, `link-net --free-play`, the player's `--free-play`
make link-record-test     # the settings records linked play writes against what manxtt, indy500, motoraid, waverunr, sgt24h, overrev, overrevb, von, stcc (and stcca, stccb) and skisuprg save themselves (docs/hw/eeprom-records.md); `remodel --game G link-settings FILE` checks one image
make link-pack-test       # packs in linked play (daytona; GAMES=): the split-screen player's combined hash equals link-run's with a synthetic pack on both views, on one and on none (`--link-pack`), and the picture shows it only where asked (opens a window briefly; docs/play.md, "Link play")
make link-net-test        # two processes over loopback, srallyc, daytona, manxtt, indy500, motoraid, waverunr, sgt24h, overrev, von, stcc and skisuprg: each host's state hash equals link-run's, under 5% loss and 50 ms jitter too, and across mid-session changes of the delay D (scripted, and adaptive under Wi-Fi-like faults: `link-run --delay-at` with the logged schedule); a named host is found by `--browse` over mDNS and joined; leave, kill, settings and desync cases end with a message
make link-state-check     # experimental: the same for two linked Daytonas (link-run), saved at 1900, 3100, 3200 and 3300
make bench-all            # `make bench` for every set in tools/expected-hashes.txt that has ROMs; lists any state hash that differs
make bench-variants       # the same for every variant in tools/expected-variant-hashes.txt (`--game topskatrj`, `GAME=srallycb`...: the other releases of a title, docs/variants.md) whose ROMs are there; kept apart from bench-all
make release-tag VERSION=0.8.0   # cut a release (below, "Versions and releases"); DRY_RUN=1 checks without tagging
make geo-check GAME=sgt24h   # native geometrizer beside the chip (2B SHARC, 2C TGP) on a coin-start run, raster streams compared each frame, both timed
make geo-fuzz GAME=sgt24h    # random display lists through the chip's program and the native geometrizer, compared (FUZZ_FRAMES, FUZZ_SEED)
make play                 # the launcher: pick a game (live attract previews, settings, Continue), then play; on the JITs (JIT=0: without them)
make play GAME=hotd       # straight into a game (Esc or pad Guide or Start+Select menu, F5/F9 quick save/load, F11 fullscreen)
make play GAME=srallyc PLAYERS=2   # link play: 2 to 4 linked machines of the game on one split screen, one controller each (keyboard P1, gamepads join with a button press; --players N, the launcher's Link play button; docs/play.md, "Link play")
make play GAME=srallyc PLAY_ARGS="--host"   # network link play: host (the lobby shows this machine's address; --net-players N starts by itself); PLAY_ARGS="--join HOST:PORT" on the other machines, or the launcher's Link play, Host or Join a network game (docs/play.md, "Network link play")
make stick-log             # play Daytona with a log of the gamepad's sticks (out/stick-log.csv), and a summary of how noisy each is at rest on quitting (docs/play.md, "Measuring a pad")
make play PLAY_ARGS=--smooth   # with in-between frames on a display over 60 Hz (a frame more latency)
make play GAME=sgt24h PLAY_ARGS="--run-ahead 3"   # show the game 3 frames ahead: less input lag, 4x the emulation
make play PLAY_ARGS="--bloom 50"   # bloom on the 3D layer's bright parts, 1 to 100 percent (off by default; docs/play.md)
make play PLAY_ARGS="--temporal 67"   # temporal anti-aliasing, 3D drawn at 67% and upscaled (taa: full size; --stipple keep; docs/play.md)
make input-lag            # each game's own input lag: frames from a control to the picture changing
make run-ahead-check      # run-ahead keeps the real state hash and shows the plain run's later frames
make run-ahead-profile GAME=srallyc RUN_AHEAD_FRAMES=1   # host time per shown frame: real frame, copy, frames ahead
make play GAME=topskatr PLAY_ARGS=--perf-log   # slow redraws (what the JITs did) and late ones (where the time went: surface, drawing, event loop, emulation, geometrizer), summaries on leaving the game (docs/play.md, Slowdowns); every 300 draws where a draw's CPU time went and what the renderer holds on the GPU (docs/play.md, "The renderer's time and memory")
make play GAME=vf2 PLAY_ARGS="--no-overlap"   # draw on the main thread after each frame instead of on the draw thread alongside the next (for comparison; docs/play.md, The frame loop)
make draw-overlap-check GAME=vf2   # the player with and without its draw thread (opens a window): same state hash, same picture for every frame both drew (--frame-log; DRAW_CHECK_FRAMES, DRAW_CHECK_OPTS)
make play PLAY_ARGS="--photo-history 30"   # photo mode (F8, docs/photo-mode.md) with a 30 s time scrub (default 10; 0: none); --photo-dir DIR for where photos go
make play GAME=stcc PLAY_ARGS='--input coin-start --frames 3000 --photo "2800:back=60,orbit=30,dof=1,res=4k"'   # a scripted photo session (keys in docs/photo-mode.md, "Scripted")
make camera-scan GAME=srallyc CAMERA_FRAME=3000 CAMERA_GAP=20   # a spike for photo mode's true camera: rotations in the i960's RAM that turned as the scenery says the camera did (docs/photo-mode.md, A true camera)
make photo-check GAME=stcc   # photo mode leaves the game alone (opens a window): the same state hash with and without a photo session, and photo mode's machine holds the game's frame (PHOTO_CHECK_FRAMES, PHOTO_CHECK_SPEC, PHOTO_CHECK_OPTS; RECOMP=1)
make install-desktop RECOMP=1   # reModel in the desktop's app menu, running this checkout's player
make uninstall-desktop    # remove that app-menu entry and its icon
make smooth-frames GAME=doa    # coin-start run drawing in-between frames beside the real ones -> out/doa/frames
make web RECOMP=1         # browser build -> out/web: game list, a page and module per game (WebGPU; serve over HTTPS, docs/web.md); missing screenshots made JOBS at a time (default 3)
make site-shots           # the promo website's screenshots, from your ROMs with default settings -> out/site-shots (site/README.md)
make site-serve           # assemble the website (site/ + posters + screenshots) into out/site and serve it on http://127.0.0.1:8800
make site-deploy          # publish it: out/site force-pushed as the single commit of origin's gh-pages branch (DRY_RUN=1: don't push)
make dist                 # player to hand out (interpreter and JIT, no recompiled code, no ROMs) -> out/dist/remodel-VERSION-TARGET.tar.gz (.zip on Windows; DIST_TARGET= another target)
make app                  # macOS: universal reModel.app, ad-hoc signed, in a .dmg -> out/dist/remodel-VERSION-macos-universal.dmg
make appimage             # Linux x86_64: the player as an AppImage (pinned appimagetool, fetched once into out/tools) -> out/dist/remodel-VERSION-x86_64.AppImage
make capture              # save renderer inputs from a run into a race -> out/topskatr/capture
make gpu-check            # draw them on the software and GPU rasterisers, time and compare (GPU_OPTS="--aa 4 --depth": with them on; "--bloom 50": bloom at 50% on the GPU frames, which the software renderer has none of, so the comparison then includes it; the cost table always adds bloom columns, at --bloom's strength or 50)
make look-render           # draw captured frames without and with a look (LOOK=comic or a look.txt, PACK=, LOOK_SET="outline_weight=3 toon_steps=2"; LOOK_OPTS="--overlay miss" with PACK= paints textured polygons with no pack image magenta; docs/texture-packs.md, "Looks" and "Draw rules and overlays")
make scene-make GAME=topskatr   # named scenes (docs/uplift-studio.md, "Scenes"): run each recipe once and keep its capture, state and hash in out/GAME/scenes (needs the ROM); make scene-list shows them, make scene-verify runs the recipes again and flags drift (exit 1)
make scene-render GAME=topskatr SCENE=race-course PACK=out/topskatr/texpack SCENE_OPTS="--against original --crop 0,0,248,192 --json"   # CAMERA="orbit=30,tilt=-8" sees the scene from another camera (photo mode's words, from the scene's display.bin; scenes made before it need scene-make SCENE_OPTS=--replace)   # --overlay miss: textured polygons with no pack image in magenta, with miss_share and miss_pixels in --json   # draw a scene from its capture in about 2 s: no boot, ROM, build or lock; every scene command's --json answer has ok, code, error, data, warnings, next and exits 0 done, 1 check failed, 2 bad request, 3 missing, 4 internal
make play GAME=topskatr PLAY_ARGS="--scene race-course --texture-pack DIR"   # start the player at a scene made with scene-make (a save state's way in), the pack watched: write a PNG in DIR and that image swaps in under 0.2 s (docs/play.md)
make scene-check          # the scene acceptance test on topskatr and vf2: make, verify, render against the original, time it (needs the ROMs and a GPU)
make gpu-check PACK=out/topskatr/texpack-4x GPU_OPTS="--tex-filter mip"   # the same with a texture pack on both renderers (docs/texture-packs.md)
make gpu-check PACK=DIR GPU_OPTS="--materials --shading pixel"   # also draw the pack's normal maps, GPU against software, from captures with light.bin (made with --geo native); counts and memory per frame; --light instead of --materials for the frame to compare with
# `run --texture-pack DIR --materials --geo native` snapshots draw its normal maps in software (a line per snapshot: polygons with a lamp, with a material, pixels changed)
make pack-check PACK=DIR LIGHT_IN=out/scratch/lighting/topskatr/capture GPU_OPTS="--materials --shading pixel"   # also the pack without and with its maps, on captures with light.bin (make lighting-capture): FRAME-mat-off/-mat-on (GPU), -mat-sw (software), -mat-diff (off | on | x8), -mat-cover (green applied, orange no lamp, red degenerate frame, grey none); share of pixels with a material and a lamp, mean and largest change, frame mean (flagged over 2 levels), GPU against software, ms, memory; PROBE=X,Y@FRAME explains a pixel
# --materials also records the side channel (as --light), so compare its snapshots with a run with --light, not a bare one: without either, vertex and pixel shading have no side channel and draw from the stream's lumas
make materials-identity PACK=DIR   # the pack with every normal map made flat must draw with --materials exactly as with --light, under both rules, in flat, vertex, pixel and mipmapped pixel shading (coin-start snapshots, JOBS runs at a time, default 3; MAT_STEPS, MAT_SNAP), and on the GPU (gpu-check and temporal-probe of captures; MAT_GPU_SNAP)
# `--materials-relief off|S` with --materials: how a normal map changes brightness: relief at strength S (default 2, 0.1 to 8; relative to how the flat surface is drawn, so it shows on fully lit scenery too; a quarter to 4 times) or off, the faithful rule (the map's normal in the game's own lighting sum)
make pack-check PACK=out/topskatr/texpack-4x   # draw them with the pack on the GPU unfiltered, mipmapped and 16x, in software and as a supersampled reference; compare -> out/topskatr/pack-check
make tile-check PACK=out/topskatr/texpack-4x   # the pack's tile images live on the GPU against the software snapshots' (1080p 16:9) -> out/topskatr/tile-check
make studio-init STUDIO=out/topskatr/studio/demo STUDIO_ARGS="--game topskatr --map out/topskatr/texmap"   # the uplift studio (docs/uplift-studio.md): a project of plain files
make studio-select STUDIO=... STUDIO_ARGS="--key 3356,3455 --category ground --reason 'track'"           # also --group, --surface, --frame F --region X,Y,W,H (make studio-region lists a region)
make studio-style STUDIO=... STUDIO_ARGS="--recipe recipes.txt --category ground"                         # local style processor: palette, despeckle, ink, halftone, wrap-aware
make studio-draft STUDIO=... STUDIO_ARGS="--brief briefs/b.md --group 4 --jobs 3"                         # batch generation through an external command (default Codex), resumable
make studio-artworks / studio-bake STUDIO=... STUDIO_ARGS="--all"   # the artworks (one surface key, many colourings) and one art_ image baked to every colouring through the colour ramps, or drawn as it is (docs/uplift-studio.md, "Artworks")
make studio-variants / studio-text / studio-seam / studio-status / studio-approve / studio-pack          # surface variants, text to tiles, seam check, status counts, approve drafts, build and lint pack/
make studio-region-capture STUDIO=... STUDIO_ARGS="logo --capture-dir out/topskatr/capture --frame 2100 --region 336,280,332,96"   # a 2D region override: tile layout and the original to redraw (the pack draws an image over the tiles' rectangle, wherever it scrolls to); make studio-regions lists them
make project-init STUDIO=out/GAME/studio/NAME PROJECT_ARGS="--from-studio OLD_WORKSPACE --approver lead"   # a workspace as a project that survives hand-over: assets.tsv, log.tsv, provenance/, hashes of what is approved, no private binaries or scratch (needs REMODEL_ACTOR; docs/uplift-studio.md, "Projects")
make project-status STUDIO=...   # counts by status, what waits, stale and rejected, the log; make project-approve STUDIO=... PROJECT_ARGS="NAME.. | --category C [--override 'REASON']" approves checked drafts after the asset gates
make pack-build STUDIO=...   # the pack to ship from the approved images, with the project's version, author, licence and look.txt; make pack-resolve PACKS="TOP BELOW" says which stacked pack wins each asset and what the stack costs
make asset-check STUDIO=... ASSET="tex_..._64x64"   # the asset gates on one image in under a second: file, alpha, stale (errors), seam, changed, noise (warnings), each with its number and limit; ASSET=--all for every image (docs/uplift-studio.md, "Gates")
make asset-bundle STUDIO=... ASSET=NAME   # an agent's whole briefing for one asset in cache/bundles/NAME/: bundle.json (identity, colourings and ramps, size, wrap, where it shows, status, gates), the original, a mask of the texels that ever show, two context crops; make asset-show is the same answer with no pictures; no ROM, no GPU (docs/uplift-studio.md, "Context bundles")
make pack-gates STUDIO=... GATE_ARGS="--release"      # every gate level on the project's pack/: asset gates, lint, budget, known, selected, scene coverage; writes STUDIO/reports/DATE-VERSION/ (report.json, gates.tsv, report.md); exit 1 at an error; `remodel pack-flicker LOG --limit PX` is the clip gate
make studio-compare STUDIO=... SCENES=all   # the same before and after from the project's named scenes in seconds, with no run (make scene-make first)
make studio-compare STUDIO=... STUDIO_ARGS="--run-args '--height 1080 --aspect 16:9'"                    # the game with and without the pack: side-by-side sheets in STUDIO/compare (remodel --out DIR run: frames and eeprom elsewhere than out/GAME)
make catalogue INPUT=coin-start LABEL=race   # observe a scripted run into out/GAME/catalogue (every distinct texture, tile, 2D layout and object, and the texture RAM loads), then the report and viewer; sessions add up (docs/uplift-studio.md, "Catalogue")
make catalogue-runs RUNS=FILE               # a file of runs (LABEL|STEPS|INPUT per line): attract, courses, characters, menus; each one session
make catalogue-report / catalogue-viewer    # coverage, new since last time, screens, loaded vs drawn -> report.txt; index.html: the gallery by kind and screen, where each entry appears
make catalogue-glyphs INPUT=coin-start      # which tile shows which digit, from the changing tile layer -> out/GAME/catalogue/glyphs/glyphs.tsv (remodel catalogue glyphs --counts down: again from the trace)
remodel catalogue merge SRC | export-map | captures | tsv   # merge another catalogue in; the catalogue as a studio map (studio init --map catalogue=DIR); tile data for studio region-capture; one row per entry
make tour-list / tour-run / coverage            # a game's tours (named input scripts from power-on) run with the polygon and texture ROM read maps on, and how much of each ROM they have read, by region, with the unread stretches by address (docs/uplift-studio.md, "Tours and coverage"); TOUR=name, TOUR_OPTS="--tours FILE --json", COVERAGE_OPTS="--top N --json"
make coverage-check                             # acceptance test: the bench tour's state hash with maps on equals the bench's, a tour run twice writes the same map, the answers have the six envelope fields
make tour-record GAME=topskatr TOUR_FILE=FILE   # play and write what you played as a tour line (remodel-play --record-tour FILE; from power-on, exact: it replays to the same state hash)
make ram-search GAME=topskatr TOUR=T RAM_ARGS="--at F1,F2,F3 --rule dec=300,dec=120 --width w"   # find a work-RAM variable by comparing RAM at frames of a tour (rules same, diff, inc[=N], dec[=N], eq/ne/lt/gt=V); make ram-values lists the game's named values, or with RAM_ARGS="--at F1,F2" reads them at those frames; make ram-pokes lists the game's pokes (poke=NAME@from-to in a tour holds one; never applied otherwise)
make crawl GAME=topskatr FROM=coin-start@1300 CRAWL_MINUTES=10   # explore menus from a point in a tour: tries from saved states, keeps what reads polygon or texture ROM no tour map had, writes branch tours to out/GAME/crawl/crawl.txt (CRAWL_TRIES=N, CRAWL_OPTS="--seed N --poke NAME"); make crawl-verify replays the deepest branches with tour run and compares state hashes (docs/uplift-studio.md, "The crawler")
remodel [--game G] tour list | tour run NAME|--all | coverage | ram search|show|pokes | crawl | crawl verify   # the same by hand; --json prints one {ok, code, error, data, warnings, next} object, exit 0 done, 2 bad request, 3 missing (no map yet)
make texmap                 # which texture is where: the captured frames drawn with an ID image, texel positions, contact sheet and inventory per frame, groups of textures that belong together, and a static viewer -> out/topskatr/texmap/index.html (CAPTURE=DIR, VIEW=, TEXMAP_OPTS="--every N --viewer-frames N --group-cap N"; docs/texture-packs.md, "Texture map")
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
make jit-lockstep GAME=srallyc   # the i960 JIT vs the interpreter, block by block, through a coin-start run (JIT_STEPS, default 300M)
make jit-copro-lockstep GAME=vf2 # the copro JITs (MB86234, MB86235) vs their interpreters, run by run, through a coin-start run
make jit-test                    # the JITs' unit tests: hand-made and random programs on each JIT and its interpreter
make music                # after a `run` into a race: the music board's music -> out/topskatr/music.wav (GAME=stcc too)
make soundtrack-import SRC=~/my-mix SOUNDTRACK=my-mix   # your own music (ffmpeg converts)
make music SOUNDTRACK=my-mix      # same, with your tracks -> out/topskatr/music-my-mix.wav
```

Custom soundtracks, their `tracks.txt` format and how the game still drives
them are described in `docs/mods.md`.

One debugging command has no `make` target:
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

Every target takes `GAME=<set>` for any of the thirty-four games (`make games`
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

**GPU tests.** Some unit tests need a GPU. `make check GPU_TESTS=0`
skips them (they return early, as on a machine without one, and the check
prints that it is incomplete), for when the card's memory is taken by other
programs; run them before pushing a change.

**Memory cap.** Every `remodel` and `remodel-play` run the Makefile starts
goes through `tools/capped`: on Linux with a systemd user session it runs in
its own scope with `MemoryMax=$MEMCAP` (default `8G`) and no swap, so a
runaway run is killed alone rather than the kernel's out-of-memory killer
picking something else on the machine. `MEMCAP=16G` raises it for one
command, `MEMCAP=` turns it off; elsewhere (macOS, Windows) runs aren't
capped. For a direct run: `tools/capped target/release/remodel ...`.

## ROM folder layouts

`--rom` (the player's ROM folder setting) names a folder of sets "as MAME
keeps them", but a chip is found by what it is, not by what it is called.
The loader takes each chip of a game by its SHA-1 (the manifest's), looking
first where the chip's name says (`<set>/NAME`, `<set>.zip`, then the
parent's, then loose in the folder) and, if that holds nothing or another
dump, in every file and zip member under the folder (subfolders down to four
levels; zips are indexed by their member list, only the members of the right
size are read). So merged, split and non-merged sets, a zip under any name, loose
chips in any folders and renamed members all work, and the launcher lists a
game when files with every chip's SHA-1 are anywhere in the folder.

- A chip found by name that is not the known-good dump is named: the player
  prints `warning: CHIP: FILE has sha1 X, not the known-good Y, and no file
  in ROM_FOLDER has it; using it anyway` once and plays (as before); `verify`
  and `scan` (checks on) fail with the same facts. If the right dump is
  anywhere else in the folder it is used and nothing is said.
- A manifest chip without a SHA-1 is taken by name only.
- Zips over 128 MB (an emulator's download bundling whole sets) are not
  searched.
- Hashes are cached, so a start does not decompress the folder again: in
  `$XDG_CACHE_HOME/remodel/rom-sha1.tsv` (`~/.cache/remodel/` by default),
  keyed by path, size and modification time (and a zip member's CRC-32).
  `REMODEL_ROM_CACHE=FILE` puts it elsewhere, `REMODEL_ROM_CACHE=off` keeps
  it in memory. It is only a speed-up: delete it any time. A cached hash for
  a zip member is shared with other members of the same CRC-32 and size; a
  chip picked by content is read and hashed again before it is used.

## Shared machine: locks, worktrees, a pinned binary

Several agents (or an agent and a person) share one machine. `tools/lock`
gives each kind of shared resource its own lock, so work that needs
different things does not queue:

| Class | At once | Taken by |
|---|---|---|
| `build` | 1 | `make build`, `release`, `lint`, `web-check`, `recomp`, `agent-bin`: memory and the cargo target directory |
| `test` | 1 | `make test` and `check`, `jit-test`, `sharc-test`, ...: the GPU tests and long runs (with `build`) |
| `render` | 3 | the GPU render targets (`look-render`, `gpu-check`, `pack-check`, `depth-check`, `lighting-check`, `temporal-probe`) |
| `emulate` | 3 | for callers that want it (`tools/lock emulate -- ...`); no target takes it yet |

`tools/lock CLASS[,CLASS] [--wait SECS] -- COMMAND` runs the command holding
a slot (a class already held by an ancestor is not taken again, so targets
nest); `tools/lock status [--json]` lists holders (process, how long, the
command) and waiters; `make lock-status` is the same. `--wait` (make:
`LOCK_WAIT`, default 1800) bounds the wait: it gives up with exit code 3
and names the holder. A holder that dies releases its lock by itself, the
locks being `flock(2)` files in `out/locks/` of the main checkout (shared by
its worktrees; `REMODEL_LOCK_DIR` moves them; `REMODEL_LOCK_<CLASS>_SLOTS`
sets a class's slots). Not set up for macOS or Windows: a released build has
no locks to take.

Until nobody types it any more, `flock out/heavy.lock COMMAND` is honoured:
`build` and `test` also take `out/heavy.lock` (first, so the order is always
heavy, build, test) unless a process above has it already, so
`flock out/heavy.lock make check` does not deadlock on itself, and a hand-typed
holder shows in `status` as "by hand". `REMODEL_LEGACY_LOCK=0` turns this
off, and the day it is not needed this goes.

`make agent-setup` (in a new worktree) links `rom/` to the main checkout's,
copies each game's `out/GAME/eeprom.bin` (a missing one gives false
benchmark mismatches), makes `out/scratch/agent-NAME` (`ACTOR=NAME`, default
the folder's name) and writes `out/agent.env`, which the Makefile reads:
`CARGO_TARGET_DIR` (the agent's own `target-agents/NAME/`), `REMODEL` and `PLAY`
pointing into it, `REMODEL_ACTOR`, `REMODEL_SCRATCH` and, where cargo is not
on the path, where it is. With `REMODEL_SCRATCH` set, `remodel run` with no
`--out` writes to `$REMODEL_SCRATCH/run/GAME/` instead of `out/GAME/`, with
a fresh copy of `out/GAME/eeprom.bin` to start from (the shared one stays
untouched), so two agents never write one output. It prints, as one JSON
object, the binary, the ROM sets, free GPU memory (where `nvidia-smi` is),
free disk and the scratch folder.

`make agent-bin` builds the CLI under the `build` lock and keeps a
read-only copy in `out/bin/COMMIT/remodel` (with a hash added when the tree
has changes; a folder is never overwritten). `make agent-setup ART=1`
then points `REMODEL` at it and marks it pinned: `make` targets that would
build `release` use it as it is. Art work (renders, `studio`, `img`) needs
nothing else. A fresh worktree with no build of its own takes the main
checkout's copy for its commit, so `make agent-bin` once in the main
checkout serves every worktree at that commit; with none for its commit
yet, it takes the newest one pinned for an earlier commit and says so.

## Versions and releases

`remodel --version` and `remodel-play --version` print the build's version:
`git describe --tags --dirty --always` (`v0.6.0-42-gabc1234`: 42 commits
after the tag, `-dirty` for uncommitted changes as of the last rebuild), or
the crate version where git or the history is missing (a source tarball).
The player shows it in the launcher's corner and at the foot of the Esc
menu; the browser's landing page footer and settings panel show it (`make
web` writes the build's version into the pages). It is display only: it
never enters emulation or a state hash. `REMODEL_VERSION=...` in the
environment overrides it at build time.

**Save states** carry a format number (`REMODEL-SAVE-<n>` at the head of
the file; the machine state inside has its own, `REMODEL-STATE-<n>`),
bumped when the layout changes, plus, from format 2, the version of the
reModel that wrote it and, from format 3, the set it was saved from (a
variant's own id: `game topskatrj`). Format 1 and 2 files are read as
before; a format 3 file saved from another variant than the one loading it is
refused with a message naming both ("this save state is from another version
of the game: it is for Top Skater (Japan), this is Top Skater (USA)"). A file with a newer format is refused with a message
("this save state was made by reModel X with format N; this build reads
format M"), not loaded. States only load into the same program ROMs (and the same variant). A state carries everything the
machine holds that later behaviour depends on, including the native
geometrizer's halt (a command it loops on for ever) when it is in one;
that is a trailing record, present only then, so a state without one reads
as before and the format number did not change. `make state-roundtrip`
checks that a save and load at several frames ends on the straight run's
hash.
Texture packs have had a `format` in their manifest since pack format 2
(docs/texture-packs.md, "Pack format"): a pack without one is format 1, and a
pack of a newer format than the build reads is refused with a message.

**Cutting a release** (the maintainer):

1. Move `CHANGELOG.md`'s entries under a `## x.y.z (date): title` heading
   and set `version` in the root `Cargo.toml`'s `[workspace.package]`.
   Commit.
2. `make release-tag VERSION=x.y.z` (`ROM=` if the ROMs aren't in `rom/`).
   It checks the tree is clean, the Cargo version and the CHANGELOG heading,
   runs `make check` and `make release`, benches every set in
   `tools/expected-hashes.txt` that has ROMs (`make bench-all`) and requires
   each state hash to match, then creates the annotated tag `vx.y.z` with
   the CHANGELOG section as its message. `DRY_RUN=1` does everything but
   create the tag.
3. It never pushes; it prints the command: `git push origin HEAD vx.y.z`.
4. Pushing the tag starts `.github/workflows/release.yml`: on Linux
   (x86_64, built on Ubuntu 22.04 for an older glibc) `make appimage`, the
   tarball and the AppImage; on Windows (x86_64) `make dist`, a zip; on
   macOS `make app`, one universal `reModel.app` (Apple silicon and Intel,
   bundle id `io.github.asdfgasfhsn.remodel`) in a `.dmg`; then a
   published GitHub release named after the tag's subject and marked the
   latest, with its notes, the four files and `SHA256SUMS.txt`, once all
   three builds pass (a failed build makes no release; fix it and tag the
   next patch version). The releases page is where people get builds.
   The workflow can
   also be run by hand (Actions, release, Run workflow): it then builds
   the archives as workflow artifacts and makes no release, a way to try a
   build before tagging.

**Continuous integration.** `.github/workflows/ci.yml` runs what `make
check` runs (fmt-check, lint, test, web-check) on Ubuntu for every push to
main and every pull request, except those that only change `docs/`, `site/`
or Markdown files. The runners have no GPU, so the GPU tests
skip (they print "no GPU adapter: skipped"); run `make check` here as
before. The runner is small (4 CPUs, 16 GB): CI builds without debug info
and gives up after 40 minutes. A test must never signal anything but its
own children: one that ran `kill -KILL -PID` on a process group took the
runner's agent down with it on CI, so the run died with no log (it now
calls `killpg` itself). The release builds aren't signed with a developer certificate (the
macOS app is ad-hoc signed, which Apple silicon requires, and not
notarised): the archives' README says how to open them past macOS's
Gatekeeper and Windows's SmartScreen.

When emulation changes on purpose, update that set's line in
`tools/expected-hashes.txt` in the same commit. `make bench` runs from a blank EEPROM
(`--fresh-eeprom`), so the hashes don't depend on what earlier runs saved in
`out/GAME/eeprom.bin`.

