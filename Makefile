# reModel: build, test and analysis entry points.
#
# Every target with a `## description` shows up in `make help`, which is the
# default goal. Keep descriptions current when adding targets.

GAME  ?= topskatr
ROM   ?= rom
OUT   ?= out/$(GAME)
ADDR  ?= 0x5c0
COUNT ?= 32
STEPS ?= 50000000
SNAP  ?= 0
INPUT ?=
# Snapshot view options for `run`, e.g. VIEW="--height 1080 --aspect 16:9 --hud edges".
VIEW  ?=
# Texture pack directory for `run` snapshots and gpu-check (docs/texture-packs.md), e.g. PACK=out/topskatr/texpack.
PACK  ?=
# Fixed run for `make bench`: attract, coin-up, start, into a race.
# `coin-start` is the game's own coin-up and start script (its manifest's
# controls); for topskatr that is coin1@1100-1104,start@1250-1254.
BENCH_STEPS ?= 600000000
BENCH_INPUT ?= coin-start
# Capture sessions for `make assets` (topskatr only): menus stepping through every
# skater, then a Novice and an Expert race with some steering.
ASSET_MENUS ?= coin1@1100-1104,start@1250-1254,selright@2000-2004,selright@2070-2074,selright@2140-2144,selright@2210-2214,selright@2280-2284,selright@2350-2354,selright@2420-2424,selright@2490-2494,selright@2560-2564,selright@2630-2634
ASSET_STEER ?= curving=0x50@3000-3400,curving=0xb0@3600-3900,curving=0x60@4300-4700,curving=0xa0@5200-5500,curving=0x58@6200-6600,curving=0xa8@7300-7600
ASSET_RACE_STEPS ?= 1600000000

# Where `make m68k-tests` keeps the SingleStepTests 68000 suite.
M68K_TESTS     ?= out/m68k-tests
M68K_TESTS_API := https://api.github.com/repos/SingleStepTests/680x0/contents/68000/v1
M68K_TESTS_RAW := https://raw.githubusercontent.com/SingleStepTests/680x0/main/68000/v1

# ADSP-21062 (Model 2B SHARC) program images for `make sharc-test`, and the
# one `make sharc-disasm` lists. Captured from a 2B upload into out/ (never
# committed; docs/hw/adsp2106x.md says how).
SHARC_PROGRAMS ?= $(wildcard out/overrevb/sharc-copro.bin out/overrevb/sharc-geo.bin)
FILE ?= out/overrevb/sharc-copro.bin

# Where `make z80-test` looks for the zexdoc/zexall exercisers (never committed).
Z80_TESTS     ?= out/z80-tests
Z80_TESTS_URL := https://raw.githubusercontent.com/anotherlin/z80emu/master/testfiles

# With no `cargo` on PATH (no system Rust), fall back to the rustup below:
# its proxies then serve cargo, `cargo fmt` and clippy, here and in the
# scripts make runs.
ifeq ($(shell command -v cargo 2>/dev/null),)
export PATH := $(HOME)/.local/share/rustup-wasm/bin:$(PATH)
endif
CARGO   ?= cargo
# The browser build needs a toolchain with the wasm32 target and
# wasm-bindgen-cli matching Cargo.lock. Default: rustup's stable cargo, from
# ~/.local/share/rustup-wasm/bin (a rustup kept out of ~/.cargo/bin, where
# its proxies would shadow the system toolchain's `cargo fmt` and clippy)
# or ~/.cargo/bin.
WASM_RUSTUP  ?= $(firstword $(wildcard $(HOME)/.local/share/rustup-wasm/bin/rustup) $(HOME)/.cargo/bin/rustup)
WASM_CARGO   ?= RUSTC=$(shell $(WASM_RUSTUP) which rustc) $(shell $(WASM_RUSTUP) which cargo)
WASM_BINDGEN ?= $(HOME)/.cargo/bin/wasm-bindgen
WEB_OUT      ?= out/web
# RECOMP=1: use the build with recompiled i960 and TGP code (see `make recomp`).
RECOMP  ?=
ifneq ($(RECOMP),)
REMODEL := target/recomp/release/remodel
PLAY    := target/recomp/release/remodel-play
# Built from the generated workspace in out/recomp, which links the
# registry of recompiled programs in place of the crates/recomp stub. No
# debug info: the generated code is large and only slows the build.
RECOMP_CARGO := CARGO_TARGET_DIR=target/recomp CARGO_PROFILE_RELEASE_DEBUG=false $(CARGO) build --release --manifest-path out/recomp/Cargo.toml
# $(call release_build,PACKAGE): the program crates one at a time, each
# with every core (the largest take about 5 GB; docs/recompiler.md), then
# the rest.
release_build = @test -f out/recomp/Cargo.toml || { echo "no out/recomp/Cargo.toml: run make recomp (or make recomp-workspace) first"; exit 1; }; \
	for p in $$(cd out/recomp && ls -d i960_* tgp_* sharc_* 2>/dev/null); do \
		echo "recompiled program $$p"; \
		$(RECOMP_CARGO) --quiet -p remodel-recomp-$$(echo $$p | tr _ -) || exit 1; \
	done; \
	echo "$(RECOMP_CARGO) -p $(1)"; \
	$(RECOMP_CARGO) -p $(1)
else
REMODEL := target/release/remodel
PLAY    := target/release/remodel-play
release_build = $(CARGO) build --release -p $(1)
endif
RUN     := $(REMODEL) --game $(GAME) --rom $(ROM)

.DEFAULT_GOAL := help
.PHONY: help build release dist test lint fmt fmt-check check ci z80-test sharc-test sharc-disasm games inputs verify boot disasm analyze run copro-disasm tgp-disasm tgp-check m68k-tests clean recomp recomp-workspace bench latency input-lag run-ahead-check sound sound-disasm geo-check geo-fuzz smooth-frames temporal-capture temporal-check pace-sim install-desktop uninstall-desktop

##@ General

help: ## Show this help (variables: GAME, ROM, OUT, ADDR, COUNT, STEPS, SNAP, INPUT, VIEW, PACK, PACK_CHECK_OUT, TILE_CHECK_OUT, SCALE, UPSCALER, TILES, MIN_USES, MAX_SIDE, FORCE, TEXDUMP, TEXPACK, COLOUR, GPU_OPTS, LIGHT_OPTS, LIGHT_CAPTURE, LIGHT_IN, PROBE, AGAINST, SEQ_FROM, SEQ_FRAMES, STATE, TEMPORAL_SEQ, TEMPORAL_OPTS, RECOMP, PLAY_ARGS, FILE, SECONDS, SOUNDTRACK, SRC)
	@awk 'BEGIN {FS = ":.*## "; printf "usage: make <target> [VAR=value]\n"} \
		/^##@/ {printf "\n%s\n", substr($$0, 5)} \
		/^[a-zA-Z0-9_-]+:.*## / {printf "  %-12s %s\n", $$1, $$2}' $(MAKEFILE_LIST)
	@printf "\ncurrent: GAME=%s ROM=%s OUT=%s\n" "$(GAME)" "$(ROM)" "$(OUT)"

##@ Build and test

build: ## Debug build of the whole workspace
	$(CARGO) build --workspace

release: ## Optimised build of the remodel CLI (RECOMP=1: with the recompiled game code)
	$(call release_build,remodel)

recomp: ## Recompile the game's i960 code and the TGP programs captured by `make run` into crates in out/recomp and build them (only what changed); then use RECOMP=1
	$(CARGO) build --release -p remodel
	target/release/remodel --game $(GAME) --rom $(ROM) recomp
	$(MAKE) release RECOMP=1

recomp-workspace: ## Rewrite out/recomp's Cargo workspace and program crate manifests from the programs there (after updating reModel or moving the checkout)
	$(CARGO) build --release -p remodel
	target/release/remodel recomp-workspace

test: ## Run all unit tests (no ROMs needed)
	$(CARGO) test --workspace

lint: ## Clippy across all crates and targets, warnings are errors
	$(CARGO) clippy --workspace --all-targets -- -D warnings

fmt: ## Format all Rust code
	$(CARGO) fmt --all

fmt-check: ## Fail if any Rust code is unformatted
	$(CARGO) fmt --all -- --check

check: fmt-check lint test ## Everything a change must pass before commit

ci: check ## Alias for check

pace-sim: ## The browser's audio rate control on synthetic frame timings (steady, phone jitter, 30 Hz, slowdowns): pitch movement and underruns, before and after (docs/web.md)
	$(CARGO) test --release -p remodel-model2 --test pace_sim -- --ignored --nocapture

m68k-tests: $(M68K_TESTS)/.done ## Run the 68000 core against the SingleStepTests suite (downloaded to out/ on first use)
	M68K_TESTS=$(abspath $(M68K_TESTS)) $(CARGO) test --release -p m68k --test singlestep -- --ignored --nocapture

# Fetch the public SingleStepTests 68000 suite and decompress it; never committed.
$(M68K_TESTS)/.done:
	mkdir -p $(M68K_TESTS)/gz $(M68K_TESTS)/json
	curl -fsSL $(M68K_TESTS_API) | grep -o '"name": *"[^"]*\.json\.gz"' | sed 's/.*"\([^"]*\)"$$/\1/' \
		> $(M68K_TESTS)/files.txt
	while read -r f; do \
		curl -fsSL -o $(M68K_TESTS)/gz/$$f $(M68K_TESTS_RAW)/$$f || exit 1; \
		gzip -dc $(M68K_TESTS)/gz/$$f > $(M68K_TESTS)/json/$${f%.gz} || exit 1; \
	done < $(M68K_TESTS)/files.txt
	touch $@

z80-test: ## Run the Z80 core against the zexdoc and zexall exercisers in Z80_TESTS= (default out/z80-tests; see docs/hw/z80.md)
	@if [ -f $(Z80_TESTS)/zexdoc.com ] || [ -f $(Z80_TESTS)/zexall.com ]; then \
		Z80_TESTS=$(abspath $(Z80_TESTS)) $(CARGO) test --release -p remodel-z80 --test zex -- --ignored --nocapture --test-threads 2; \
	else \
		echo "no zexdoc.com or zexall.com in $(Z80_TESTS). They are Frank Cringle's freely redistributable"; \
		echo "Z80 exercisers (CP/M .com files); fetch them into $(Z80_TESTS), e.g.:"; \
		echo "  mkdir -p $(Z80_TESTS)"; \
		echo "  curl -fsSL -o $(Z80_TESTS)/zexdoc.com $(Z80_TESTS_URL)/zexdoc.com"; \
		echo "  curl -fsSL -o $(Z80_TESTS)/zexall.com $(Z80_TESTS_URL)/zexall.com"; \
		exit 1; \
	fi

sharc-test: ## ADSP-21062 (SHARC) unit tests; then decode, discovery and a boot of the 2B programs in SHARC_PROGRAMS= (default: those captured in out/overrevb)
	$(CARGO) test -p adsp2106x
	@if [ -n "$(SHARC_PROGRAMS)" ]; then \
		$(MAKE) --no-print-directory release && $(REMODEL) sharc-check $(SHARC_PROGRAMS); \
	else \
		echo "no SHARC program images in out/overrevb (docs/hw/adsp2106x.md): unit tests only"; \
	fi

##@ ROM tools (need a user-supplied dump in ROM=)

games: release ## List games with manifests
	$(REMODEL) games

inputs: release ## The game's inputs: names for INPUT=, ports and channels, default keys and pad bindings
	$(REMODEL) --game $(GAME) inputs

verify: release ## Check every chip in ROM= against known-good hashes
	$(RUN) verify

scan: release ## List the sets in ROM= (MAME zips or folders) and check the supported ones
	$(RUN) scan

boot: release ## Print the i960 boot record and system-table entry points
	$(RUN) boot

disasm: release ## Linear disassembly: make disasm ADDR=0x6f0 COUNT=64
	$(RUN) disasm $(ADDR) $(COUNT)

analyze: release ## Recursive-descent code discovery; listing goes to OUT=
	$(RUN) analyze $(OUT)/program.lst

run: release ## Boot and report; PNG every SNAP frames; scripted INPUT (see remodel usage; INPUT=coin-start coins up and starts)
	$(RUN) $(VIEW) $(if $(PACK),--texture-pack $(PACK)) $(if $(INPUT),--input $(INPUT)) run $(STEPS) $(SNAP)

.PHONY: assets
assets: release ## Extract topskatr's art for review: capture sessions, then distinct textures, screens and contact sheets in OUT/assets (about 15 min)
	rm -rf $(OUT)/assets && mkdir -p $(OUT)/assets/raw
	$(RUN) --dump-textures $(OUT)/assets/raw/attract run 900000000 20 > $(OUT)/assets/raw/attract.log
	cp -r $(OUT)/frames $(OUT)/assets/frames-attract
	$(RUN) --dump-textures $(OUT)/assets/raw/menus --input $(ASSET_MENUS) run 480000000 10 > $(OUT)/assets/raw/menus.log
	cp -r $(OUT)/frames $(OUT)/assets/frames-menus
	$(RUN) --dump-textures $(OUT)/assets/raw/novice --input coin1@1100-1104,start@1250-1254,$(ASSET_STEER) \
		run $(ASSET_RACE_STEPS) 15 > $(OUT)/assets/raw/novice.log & \
	$(RUN) --dump-textures $(OUT)/assets/raw/expert --input coin1@1100-1104,start@1250-1254,selright@1330-1336,$(ASSET_STEER) \
		run $(ASSET_RACE_STEPS) 15 > $(OUT)/assets/raw/expert.log & \
	wait
	python3 -I tools/asset-review.py $(OUT)/assets

.PHONY: dump-textures
dump-textures: release ## Coin-start-race run dumping every 3D texture and tile pattern drawn (each 60 frames) to OUT/texdump
	rm -rf $(OUT)/texdump
	$(RUN) $(VIEW) --dump-textures $(OUT)/texdump --input $(BENCH_INPUT) run $(BENCH_STEPS) 60

# Batch upscaling of OUT/texdump into a texture pack (docs/texture-packs.md,
# "Batch upscaling"). UPSCALER: an external tool's command with {in}, {out}
# and {scale} filled in, run once over the directory (empty: built-in
# bicubic), e.g.
#   UPSCALER='realesrgan-ncnn-vulkan -i {in} -o {out} -s {scale} -n realesrgan-x4plus -g 1 -f png'
# TILES: model | builtin | nearest | skip (how 8x8 tiles are scaled);
# COLOUR: match (put the dump's low-frequency colour back, keeping the
# upscaler's detail) | keep (the upscaler's colours as they come);
# MIN_USES: leave out dumps drawn fewer times; MAX_SIDE: largest pack image
# side; FORCE=1: redo everything (after changing settings).
SCALE    ?= 4
UPSCALER ?=
TILES    ?= model
COLOUR   ?= match
MIN_USES ?= 1
MAX_SIDE ?= 4096
FORCE    ?=
TEXDUMP  ?= $(OUT)/texdump
TEXPACK  ?= $(OUT)/texpack-$(SCALE)x
.PHONY: upscale-textures
# Through the environment, so the template's quotes reach the tool as written.
upscale-textures: export UPSCALER_CMD = $(UPSCALER)
upscale-textures: release ## Upscale TEXDUMP (default OUT/texdump, from dump-textures) into texture pack TEXPACK (OUT/texpack-SCALEx) with UPSCALER= (empty: built-in); wrap-aware padding, colour matched to the dump (COLOUR=keep: off), hard alpha, only new or changed dumps
	$(REMODEL) --game $(GAME) texpack-upscale $(TEXDUMP) $(TEXPACK) --scale $(SCALE) --tiles $(TILES) --colour $(COLOUR) \
		--min-uses $(MIN_USES) --max-side $(MAX_SIDE) --work out/scratch/texpack-upscale-$(GAME) \
		$(if $(UPSCALER),--upscaler "$$UPSCALER_CMD") $(if $(FORCE),--force)

play: ## Play in a native window: the launcher (game cards, live attract previews, settings), or straight into GAME= if given; GPU, live sound, gamepads, Esc menu, save states (RECOMP=1: recompiled code; PLAY_ARGS)
	$(call release_build,remodel-play)
	$(PLAY) $(if $(filter command line environment,$(origin GAME)),--game $(GAME)) --rom $(ROM) $(PLAY_ARGS)

# The launcher in the desktop's app menu, running this checkout's player
# (from the repo, so it finds rom/ and keeps its files in out/).
APPS_DIR ?= $(or $(XDG_DATA_HOME),$(HOME)/.local/share)/applications
ICONS_DIR ?= $(or $(XDG_DATA_HOME),$(HOME)/.local/share)/icons/hicolor/512x512/apps

install-desktop: ## Add reModel to the desktop's app menu: builds the player (RECOMP=1: recompiled) and writes a .desktop entry and icon that run it from this checkout
	$(call release_build,remodel-play)
	mkdir -p "$(APPS_DIR)" "$(ICONS_DIR)"
	cp assets/reModel-icon.png "$(ICONS_DIR)/remodel.png"
	printf '%s\n' '[Desktop Entry]' 'Type=Application' 'Name=reModel' \
		'GenericName=Arcade player' 'Comment=Sega Model 2 games, recompiled' \
		'Exec=$(CURDIR)/$(PLAY) --rom $(abspath $(ROM))' 'Path=$(CURDIR)' \
		'TryExec=$(CURDIR)/$(PLAY)' 'Icon=remodel' 'Terminal=false' \
		'Categories=Game;Emulator;' 'StartupWMClass=remodel' 'StartupNotify=true' \
		> "$(APPS_DIR)/remodel.desktop"
	@echo "install-desktop: $(APPS_DIR)/remodel.desktop runs $(CURDIR)/$(PLAY)"

uninstall-desktop: ## Remove the app-menu entry and icon that install-desktop wrote
	rm -f "$(APPS_DIR)/remodel.desktop" "$(ICONS_DIR)/remodel.png"

# `make dist`: interpreter only, so no game-derived recompiled code is
# linked in; a target dir of its own keeps the stripped profile apart.
DIST_TARGET_DIR := target/dist
DIST_OUT        ?= out/dist

dist: ## Distributable player for this platform (interpreter only, stripped) -> out/dist/remodel-VERSION-TARGET.tar.gz
	@test -z "$(RECOMP)" || { echo "make dist builds the interpreter only: recompiled code is derived from the game, so it isn't distributed (drop RECOMP=1)"; exit 1; }
	CARGO_TARGET_DIR=$(DIST_TARGET_DIR) CARGO_PROFILE_RELEASE_DEBUG=false CARGO_PROFILE_RELEASE_STRIP=true \
		$(CARGO) build --release --locked --manifest-path Cargo.toml -p remodel-play
	@$(CARGO) tree --locked --manifest-path Cargo.toml -p remodel-play -i remodel-recomp --prefix none | head -1 | grep -q "($(CURDIR)/crates/recomp)" \
		|| { echo "dist: remodel-recomp isn't the crates/recomp stub: recompiled code in the dist build"; exit 1; }
	@set -e; \
	version=$$($(CARGO) pkgid -p remodel-play | sed 's/.*[@#]//'); \
	rev=$$(git describe --tags --always --dirty 2>/dev/null || true); \
	target=$$(rustc -vV | sed -n 's/^host: //p'); \
	name=remodel-$$version$${rev:+-$$rev}-$$target; \
	dir=$(DIST_OUT)/$$name; \
	rm -rf $$dir $$dir.tar.gz; mkdir -p $$dir; \
	cp $(DIST_TARGET_DIR)/release/remodel-play $$dir/; \
	for l in $(wildcard LICENSE*); do cp $$l $$dir/; done; \
	python3 -I tools/notices.py remodel-play $$target $$dir/THIRD-PARTY-NOTICES.txt; \
	sed -e "s|@VERSION@|$$version$${rev:+ ($$rev)}|" -e "s|@TARGET@|$$target|" crates/play/dist-README.txt > $$dir/README.txt; \
	tar -C $(DIST_OUT) -czf $$dir.tar.gz $$name; \
	echo "built $$dir.tar.gz ($$(du -h $$dir.tar.gz | cut -f1))"

web: ## Browser build into out/web: game list, a page per game (model2/SET/), screenshots; RECOMP=1 gives each game its own module with its recompiled code (docs/web.md)
	WEB_OUT=$(WEB_OUT) ROM=$(ROM) RECOMP=$(RECOMP) CARGO="$(CARGO)" WASM_CARGO="$(WASM_CARGO)" WASM_BINDGEN=$(WASM_BINDGEN) sh tools/web.sh

capture: release ## Run into a race saving every 300th frame's renderer inputs to out/GAME/capture (~200 MB)
	rm -rf $(OUT)/capture
	$(RUN) --capture $(OUT)/capture --input $(BENCH_INPUT) run 900000000 300 >/dev/null

# Options for gpu-check, depth-check and pack-check: --aa N (multisampling;
# software supersamples to match), --depth, --tex-filter off|mip|2|4|8|16
# (gpu-check with PACK), and for depth-check and pack-check
# --height/--aspect/--hud; --shading flat|vertex|pixel, --crease auto|N and
# --shading-fine on|off (smooth shading, model2::light) for all of them.
GPU_OPTS ?=
gpu-check: release ## Draw the captured frames on the software and GPU rasterisers; time and compare them, and what GPU_OPTS="--aa 4 --depth" and PACK=DIR (a texture pack, both renderers) cost (needs a GPU)
	$(RUN) $(GPU_OPTS) $(if $(PACK),--texture-pack $(PACK)) gpu-check

# Where pack-check writes its pictures.
PACK_CHECK_OUT ?= $(OUT)/pack-check
pack-check: release ## Draw the captured frames with texture pack PACK= on the GPU unfiltered, mipmapped and 16x anisotropic, in software trilinear and as a supersampled reference; write the pictures to PACK_CHECK_OUT (default OUT/pack-check) and compare them (needs a GPU)
	$(if $(PACK),,$(error pack-check needs PACK=DIR, a texture pack))
	$(RUN) $(GPU_OPTS) --texture-pack $(PACK) pack-check $(OUT)/capture $(PACK_CHECK_OUT)

# Where tile-check writes its pictures.
TILE_CHECK_OUT ?= $(OUT)/tile-check
.PHONY: tile-check
tile-check: release ## Draw the captured frames with the tile images of texture pack PACK live on the GPU and as the software snapshots replace them; write FRAME-gpu, -sw and -orig.png to TILE_CHECK_OUT (default OUT/tile-check) and compare them (needs a GPU; VIEW= sets the view, default 1080p 16:9)
	$(if $(PACK),,$(error tile-check needs PACK=DIR, a texture pack))
	$(RUN) $(VIEW) --texture-pack $(PACK) tile-check $(OUT)/capture $(TILE_CHECK_OUT)

pack-identity: release ## Draw the captured frames with a pack standing for the original textures (PACK=DIR, normally the dump itself; empty: each frame's own textures) on the GPU and in software, unfiltered and mipmapped, against no pack; fails if they differ beyond the tolerance (needs a GPU)
	$(RUN) $(GPU_OPTS) $(if $(PACK),--texture-pack $(PACK)) pack-check --identity $(OUT)/capture $(OUT)/pack-identity

# depth-check: PROBE="X,Y@FRAME ..." follows those output pixels through the
# depth buffer (only those frames are drawn); AGAINST=DIR compares the
# depth-on frames with an earlier run's pictures (docs/enhancements.md).
depth-check: release ## Draw the captured frames on the GPU with the depth buffer off and on, report and picture the differences into OUT/depth-check (GPU_OPTS: --aa N, --height, --aspect; PROBE=X,Y@FRAME, AGAINST=DIR; needs a GPU)
	$(RUN) $(GPU_OPTS) depth-check $(OUT)/capture $(OUT)/depth-check $(foreach p,$(PROBE),--probe $(p)) $(if $(AGAINST),--against $(AGAINST))

# lighting-check: LIGHT_OPTS for the command, e.g. "--every 5" (pictures of
# every 5th frame), "--probe X,Y@FRAME" (repeatable), "--bench [N]";
# GPU_OPTS before it ("--crease N", "--height", "--aspect").
LIGHT_OPTS ?=
.PHONY: lighting-check
lighting-check: release ## Draw the captured frames flat and smooth-shaded (vertex, or GPU_OPTS="--shading pixel" for per pixel; "--shading-fine on" adds finer levels and K, against the game's levels; "--shading-balance off" checks without the brightness balance, -nobal pictures and reports, its mean brightness C report-only) on the software and GPU renderers; check the invariants (crease 0 is flat, unlit polygons unchanged, brightness of corners and pixels, software against GPU, hue kept, determinism; with LIGHT_IN=a lighting-capture: the side channel's self-check and list scan, L0 against it), report and pictures into out/scratch/lighting/GAME (LIGHT_OPTS: --every N, --probe X,Y@F, --bench [N], --no-side; GPU_OPTS: --crease N, --height/--aspect; needs a GPU)
	$(RUN) $(GPU_OPTS) lighting-check $(or $(LIGHT_IN),$(OUT)/capture) out/scratch/lighting/$(GAME) $(LIGHT_OPTS)

# Where lighting-capture saves (a directory of its own: the regression
# baseline hashes out/GAME/capture).
LIGHT_CAPTURE ?= out/scratch/lighting/$(GAME)/capture
.PHONY: lighting-capture
lighting-capture: release ## Save every 300th frame of a coin-start run with its lighting side channel (native geometrizer; light.bin, scan.bin) to LIGHT_CAPTURE (default out/scratch/lighting/GAME/capture), for lighting-check; prints the side channel's self-check over every frame of the run
	rm -rf $(LIGHT_CAPTURE)
	$(RUN) --geo native --light --capture $(LIGHT_CAPTURE) --input $(BENCH_INPUT) run 900000000 300 | grep -E '^light'

.PHONY: temporal-probe
temporal-probe: release ## Draw the captured frames with the GPU's temporal anti-aliasing (GPU_OPTS: --temporal, --stipple, --depth, --height, --aspect): its first frame against the frame with it off, jittered, blend and keep pictures into out/scratch/temporal-probe/GAME (needs a GPU)
	$(RUN) $(GPU_OPTS) temporal-probe $(OUT)/capture out/scratch/temporal-probe/$(GAME)

# Temporal anti-aliasing's test sequences: SEQ_FRAMES consecutive frames
# from frame SEQ_FROM of a run with INPUT (default coin-start; STATE=FILE
# starts from a save state), kept in TEMPORAL_SEQ (git-ignored scratch,
# about 0.3 MB a frame). TEMPORAL_OPTS for temporal-check: --every N,
# --gpu-only, --no-gpu, --crop X,Y,W,H, --at F,.., --tune K=V,.., --ref-cache DIR,
# --bench [N], --inspect (error by resolve rule), --dump X,Y,W,H (the
# resolve's decisions per pixel), --depth (before the command),
# --height/--aspect/--hud.
SEQ_FROM      ?= 3500
SEQ_FRAMES    ?= 300
STATE         ?=
TEMPORAL_SEQ  ?= out/scratch/temporal/$(GAME)/seq
TEMPORAL_OPTS ?=
temporal-capture: release ## Save SEQ_FRAMES consecutive frames from SEQ_FROM (INPUT, STATE) with their motion data to TEMPORAL_SEQ, for temporal-check (native geometrizer)
	$(RUN) --geo native --capture-seq $(TEMPORAL_SEQ) --seq $(SEQ_FROM),$(SEQ_FRAMES) $(if $(STATE),--load-state $(STATE)) --input $(or $(INPUT),$(BENCH_INPUT)) run 4000000000 0 >/dev/null

temporal-check: release ## Check TEMPORAL_SEQ's motion vectors (pose agreement, reprojection, the GPU's velocity target) and score plain, MSAA 4x, TAA and 67%/50% upscaled GPU frames against supersampled references over the sequence; report and pictures in TEMPORAL_SEQ/check (TEMPORAL_OPTS: --every N, --at F,.., --gpu-only, --no-gpu, --crop, --tune, --ref-cache DIR, --inspect, --dump X,Y,W,H, --depth, --bench [N]; needs a GPU)
	$(RUN) temporal-check $(TEMPORAL_SEQ) $(TEMPORAL_OPTS)

GEO_CHECK_STEPS ?= 750000000
GEO_CHECK_FROM  ?= 3000
FUZZ_FRAMES ?= 500
FUZZ_SEED   ?= 1
geo-check: release ## A coin-start run (GEO_CHECK_STEPS, INPUT) with the native geometrizer beside the chip (2B SHARC, 2C TGP), raster streams and state compared each frame; both timed (from frame GEO_CHECK_FROM too; RECOMP=1: the recompiled chip)
	$(RUN) --input $(or $(INPUT),$(BENCH_INPUT)) geo-check $(GEO_CHECK_STEPS) $(GEO_CHECK_FROM)

geo-fuzz: release ## Random display lists (FUZZ_FRAMES, FUZZ_SEED) through the game's geometrizer program on its chip (SHARC or TGP) and the native one, compared
	$(RUN) geo-fuzz $(FUZZ_FRAMES) $(FUZZ_SEED)

SMOOTH_T    ?= 0,0.25,0.5,0.75
SMOOTH_SNAP ?= 500
smooth-frames: release ## Smooth motion's in-betweens: a coin-start run (BENCH_STEPS, INPUT) on the native geometrizer; every SMOOTH_SNAP-th frame also drawn at SMOOTH_T of the way from the frame before (out/GAME/frames/FRAME-tTT.png, FRAME-prev.png), in-betweens timed
	$(RUN) --geo native --smooth $(SMOOTH_T) --input $(or $(INPUT),$(BENCH_INPUT)) run $(BENCH_STEPS) $(SMOOTH_SNAP)

lockstep: ## Run the recompiled code beside the interpreter (i960 block by block; Model 2B's SHARCs too) and stop at the first difference (RECOMP=1, STEPS, INPUT)
	$(MAKE) release RECOMP=1
	target/recomp/release/remodel --game $(GAME) --rom $(ROM) $(if $(INPUT),--input $(INPUT)) lockstep $(STEPS)

state-check: release ## Save state round trip: a run saved at frame 2000 and resumed must match an uninterrupted one (state hash and audio)
	@$(RUN) --audio $(OUT)/state-a.wav --save-state $(OUT)/state-check.save@2000 --input $(BENCH_INPUT) run $(BENCH_STEPS) | grep '^state hash' > $(OUT)/state-a.txt
	@$(RUN) --audio $(OUT)/state-b.wav --load-state $(OUT)/state-check.save --input $(BENCH_INPUT) run $(BENCH_STEPS) | grep '^state hash' > $(OUT)/state-b.txt
	@cmp -s $(OUT)/state-a.txt $(OUT)/state-b.txt && echo "state hash    same after save and load: $$(cut -c15- $(OUT)/state-a.txt)" || { echo "state hash DIFFERS:"; cat $(OUT)/state-a.txt $(OUT)/state-b.txt; exit 1; }
	@python3 -I -c 'import sys, wave; a, b = (wave.open(f).readframes(1 << 30) for f in sys.argv[1:]); print("audio         " + ("resumed run matches the uninterrupted one" if a.endswith(b) else "DIFFERS")); sys.exit(not a.endswith(b))' $(OUT)/state-a.wav $(OUT)/state-b.wav

LATENCY_FRAME  ?= 3100
LATENCY_CHANGE ?= wheel=0xF0
latency: release ## Input latency: frames from a held input (LATENCY_CHANGE, from frame LATENCY_FRAME of a coin-start run) to the game's RAM, display list and polygons
	$(RUN) --input $(BENCH_INPUT) latency $(LATENCY_FRAME) $(LATENCY_CHANGE)

# `make input-lag`: per game, GAME+FRAME+CHANGE[+INPUT]: a frame in play of
# a run with INPUT (default coin-start), and a control held from there.
# The fighters' and gun games' runs stop their periodic buttons first, so
# the control isn't lost in a move already under way. Each game's
# docs/games/<set>.md gives what these measured (romset's
# `Game::input_lag`).
FIGHT_INPUT := coin1@1100-1104,start@1250-1254,punch@1400-3400/30,kick@1415-3400/30
INPUT_LAG ?= topskatr+3507+curving=0xF0 skisuprg+3507+swing=0xF0 \
	stcc+4507+wheel=0xD0+coin-start,gas=0xff@4100-99999 segawski+3507+slide=0xF0 \
	waverunr+3507+handlebar=0xF0 overrev+3507+wheel=0xF0 hotd+3510+trigger1 \
	bel+3507+trigger1+coin1@1100-1104,start@1250-1254,gun1=248:190@2200-9000,trigger1@2260-3400/8 \
	overrevb+3507+wheel=0xF0 sgt24h+3507+wheel=0xF0 dynabb97+3507+bat=0xc0 \
	doa+3507+punch+$(FIGHT_INPUT) indy500+3507+wheel=0xF0 fvipers+3707+punch+$(FIGHT_INPUT) \
	schamp+3507+right+$(FIGHT_INPUT) vstriker+3507+right lastbrnx+3507+right+$(FIGHT_INPUT) \
	srallyc+4007+wheel=0xD0+coin-start,gas=0xff@2300-99999
input-lag: release ## Each game's input lag (INPUT_LAG, or just GAME='s): frames from a control held in play to the first change in the picture, which is how many run-ahead can take away
	@for c in $(if $(filter command line,$(origin GAME)),$(filter $(GAME)+%,$(INPUT_LAG)),$(INPUT_LAG)); do \
		set -- $$(echo $$c | tr + ' '); \
		printf '%-10s %-16s ' $$1 "$$3 at $$2"; \
		$(REMODEL) --game $$1 --rom $(ROM) --geo native --input $${4:-coin-start} latency $$2 $$3 16 \
			| sed -n 's/^input lag *//p'; \
	done

RUN_AHEAD_GAMES  ?= topskatr stcc hotd overrevb vstriker
RUN_AHEAD_FRAMES ?= 2
RUN_AHEAD_STEPS  ?= 300000000
run-ahead-check: release ## Run-ahead against plain runs (RUN_AHEAD_GAMES, or GAME=): coin-start runs (RUN_AHEAD_STEPS, smooth motion on) RUN_AHEAD_FRAMES ahead keep the real state hash, and the frames shown are the plain run's that many later (pictures, hashes, in-betweens); times the copy
	@for g in $(if $(filter command line,$(origin GAME)),$(GAME),$(RUN_AHEAD_GAMES)); do \
		$(REMODEL) --game $$g --rom $(ROM) --geo native --smooth 0.5 --input $(BENCH_INPUT) \
			run-ahead-check $(RUN_AHEAD_STEPS) $(RUN_AHEAD_FRAMES) 25 || exit 1; \
	done

bench: release ## Timed fixed run into a race: speed vs real time and the state hash (must not change with speed-ups)
	$(RUN) --input $(BENCH_INPUT) run $(BENCH_STEPS) | grep -E '^(instructions|emulated|host time|state hash)'

copro-disasm: release ## Disassemble the geometry program captured by `make run`
	$(RUN) copro-disasm

tgp-disasm: release ## List a Model 2A TGP (MB86234) program with mnemonics: header, dispatch table, functions, from the upload `make run` captured (to out/GAME/tgp.lst)
	$(RUN) tgp-disasm

tgp-check: release ## Run a Model 2A TGP program's functions on the MB86234 interpreter with random arguments, against their maths and the old function model (needs `make run` first)
	$(RUN) tgp-check

sharc-disasm: release ## Disassemble an ADSP-21062 program image: make sharc-disasm FILE=out/overrevb/sharc-geo.bin (listing to FILE.lst)
	$(REMODEL) sharc-disasm $(FILE)

sound: release ## Boot the sound board, replay the sound bytes captured by `make run`, write OUT/sound.wav
	$(RUN) sound-render $(OUT)/sound.wav

sound-disasm: release ## Disassemble the sound 68000 program: make sound-disasm ADDR=0x600120 COUNT=64
	$(RUN) sound-disasm $(ADDR) $(COUNT)
.PHONY: music mpeg-dump mpeg-test soundtrack-import
SECONDS ?= 10
# Replacement soundtrack for `make music` (docs/mods.md): a name under
# soundtracks/, or a path when it contains a `/`.
SOUNDTRACK ?=
SRC        ?=
SOUNDTRACK_DIR = $(if $(findstring /,$(SOUNDTRACK)),$(SOUNDTRACK),soundtracks/$(SOUNDTRACK))

music: release ## Music board (DSB2 or DSB1) music from the stream `make run` captured -> OUT/music.wav; SOUNDTRACK=name replaces tracks (OUT/music-name.wav)
	$(RUN) music-render $(if $(SOUNDTRACK),--soundtrack "$(SOUNDTRACK_DIR)") \
		$(OUT)/music$(if $(SOUNDTRACK),-$(notdir $(SOUNDTRACK))).wav

soundtrack-import: release ## Make soundtracks/SOUNDTRACK from the audio files in SRC= (ffmpeg converts non-WAV/MP2)
	@test -n "$(SRC)" -a -n "$(SOUNDTRACK)" || { echo "usage: make soundtrack-import SRC=dir SOUNDTRACK=name"; exit 1; }
	$(REMODEL) soundtrack-import "$(SRC)" "$(SOUNDTRACK_DIR)"

mpeg-dump: release ## Decode MPEG ROM frames directly: make mpeg-dump ADDR=0x11000 SECONDS=10
	$(RUN) mpeg-dump $(ADDR) $(SECONDS)

mpeg-test: ## MPEG decoder test against the MPEG ROMs in ROM=
	REMODEL_ROM=$(abspath $(ROM)) $(CARGO) test --release -p remodel-mpeg -- --ignored

##@ Housekeeping

clean: ## Remove build output and generated listings
	$(CARGO) clean
	rm -rf out
