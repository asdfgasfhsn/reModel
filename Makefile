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
# Texture pack directory for `run` snapshots (docs/texture-packs.md), e.g. PACK=out/topskatr/texpack.
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
.PHONY: help build release dist test lint fmt fmt-check check ci z80-test sharc-test sharc-disasm games inputs verify boot disasm analyze run copro-disasm m68k-tests clean recomp recomp-workspace bench latency sound sound-disasm geo-check geo-fuzz smooth-frames pace-sim

##@ General

help: ## Show this help (variables: GAME, ROM, OUT, ADDR, COUNT, STEPS, SNAP, INPUT, VIEW, PACK, RECOMP, SOUNDTRACK)
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

play: ## Play in a native window: the launcher (game cards, live attract previews, settings), or straight into GAME= if given; GPU, live sound, gamepads, Esc menu, save states (RECOMP=1: recompiled code; PLAY_ARGS)
	$(call release_build,remodel-play)
	$(PLAY) $(if $(filter command line environment,$(origin GAME)),--game $(GAME)) --rom $(ROM) $(PLAY_ARGS)

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

gpu-check: release ## Draw the captured frames on the software and GPU rasterisers; time and compare them (needs a GPU)
	$(RUN) gpu-check

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

bench: release ## Timed fixed run into a race: speed vs real time and the state hash (must not change with speed-ups)
	$(RUN) --input $(BENCH_INPUT) run $(BENCH_STEPS) | grep -E '^(instructions|emulated|host time|state hash)'

copro-disasm: release ## Disassemble the geometry program captured by `make run`
	$(RUN) copro-disasm

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
