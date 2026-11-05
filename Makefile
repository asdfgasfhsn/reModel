# Remodel: build, test and analysis entry points.
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
# No debug info: the generated code is large and only slows the build.
RELEASE_ENV := REMODEL_RECOMP_DIR=$(abspath out/recomp) CARGO_TARGET_DIR=target/recomp CARGO_PROFILE_RELEASE_DEBUG=false
WEB_ENV := REMODEL_RECOMP_DIR=$(abspath out/recomp)
else
REMODEL := target/release/remodel
PLAY    := target/release/remodel-play
RELEASE_ENV :=
WEB_ENV :=
endif
RUN     := $(REMODEL) --game $(GAME) --rom $(ROM)

.DEFAULT_GOAL := help
.PHONY: help build release dist test lint fmt fmt-check check ci games inputs verify boot disasm analyze run copro-disasm m68k-tests clean recomp bench sound sound-disasm

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
	$(RELEASE_ENV) $(CARGO) build --release -p remodel

recomp: ## Recompile the game's i960 code and the TGP programs captured by `make run` into out/recomp and build them; then use RECOMP=1
	$(CARGO) build --release -p remodel
	target/release/remodel --game $(GAME) --rom $(ROM) recomp
	$(MAKE) release RECOMP=1

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

play: ## Play in a native window: GPU, live sound, gamepads, Esc menu, save states (RECOMP=1: recompiled code; PLAY_ARGS, e.g. --game topskatr)
	$(RELEASE_ENV) $(CARGO) build --release -p remodel-play
	$(PLAY) --game $(GAME) --rom $(ROM) $(PLAY_ARGS)

# `make dist`: interpreter only, so no game-derived recompiled code is
# linked in; a target dir of its own keeps the stripped profile apart.
DIST_TARGET_DIR := target/dist
DIST_OUT        ?= out/dist

dist: ## Distributable player for this platform (interpreter only, stripped) -> out/dist/remodel-VERSION-TARGET.tar.gz
	@test -z "$(RECOMP)" || { echo "make dist builds the interpreter only: recompiled code is derived from the game, so it isn't distributed (drop RECOMP=1)"; exit 1; }
	env -u REMODEL_RECOMP_DIR CARGO_TARGET_DIR=$(DIST_TARGET_DIR) CARGO_PROFILE_RELEASE_DEBUG=false CARGO_PROFILE_RELEASE_STRIP=true \
		$(CARGO) build --release --locked -p remodel-play
	@for f in $(DIST_TARGET_DIR)/release/build/remodel-model2-*/out/recomp_registry.rs; do \
		if grep -q '::HASH =>' $$f; then echo "$$f: recompiled code in the dist build"; exit 1; fi; \
	done
	@set -e; \
	version=$$($(CARGO) pkgid -p remodel-play | sed 's/.*[@#]//'); \
	rev=$$(git describe --tags --always --dirty 2>/dev/null || true); \
	target=$$(rustc -vV | sed -n 's/^host: //p'); \
	name=remodel-$$version$${rev:+-$$rev}-$$target; \
	dir=$(DIST_OUT)/$$name; \
	rm -rf $$dir $$dir.tar.gz; mkdir -p $$dir; \
	cp $(DIST_TARGET_DIR)/release/remodel-play $$dir/; \
	for l in $(wildcard LICENSE*); do cp $$l $$dir/; done; \
	sed -e "s|@VERSION@|$$version$${rev:+ ($$rev)}|" -e "s|@TARGET@|$$target|" crates/play/dist-README.txt > $$dir/README.txt; \
	tar -C $(DIST_OUT) -czf $$dir.tar.gz $$name; \
	echo "built $$dir.tar.gz ($$(du -h $$dir.tar.gz | cut -f1))"

web: ## Build the browser version (WebAssembly + WebGPU) into out/web; needs the wasm32 target and wasm-bindgen (docs/web.md); RECOMP=1 includes the recompiled game code
	CARGO_TARGET_DIR=target/web CARGO_PROFILE_RELEASE_DEBUG=false $(WEB_ENV) $(WASM_CARGO) build --release --target wasm32-unknown-unknown -p remodel-web
	$(WASM_BINDGEN) --target web --no-typescript --out-dir $(WEB_OUT)/pkg target/web/wasm32-unknown-unknown/release/remodel_web.wasm
	cp crates/web/index.html crates/web/audio-worklet.js crates/web/sound-worker.js crates/web/geo-worker.js crates/web/touch.js $(WEB_OUT)/
	cp assets/reModel-logo-1086.png $(WEB_OUT)/reModel-logo.png
	@echo "built $(WEB_OUT): serve it over HTTPS (WebGPU needs a secure page) with the ROM files at $(WEB_OUT)/rom/"

capture: release ## Run into a race saving every 300th frame's renderer inputs to out/GAME/capture (~200 MB)
	rm -rf $(OUT)/capture
	$(RUN) --capture $(OUT)/capture --input $(BENCH_INPUT) run 900000000 300 >/dev/null

gpu-check: release ## Draw the captured frames on the software and GPU rasterisers; time and compare them (needs a GPU)
	$(RUN) gpu-check

lockstep: ## Run the recompiled i960 code beside the interpreter, block by block, and stop at the first difference (RECOMP=1, STEPS)
	$(MAKE) release RECOMP=1
	target/recomp/release/remodel --game $(GAME) --rom $(ROM) lockstep $(STEPS)

state-check: release ## Save state round trip: a run saved at frame 2000 and resumed must match an uninterrupted one (state hash and audio)
	@$(RUN) --audio $(OUT)/state-a.wav --save-state $(OUT)/state-check.save@2000 --input $(BENCH_INPUT) run $(BENCH_STEPS) | grep '^state hash' > $(OUT)/state-a.txt
	@$(RUN) --audio $(OUT)/state-b.wav --load-state $(OUT)/state-check.save --input $(BENCH_INPUT) run $(BENCH_STEPS) | grep '^state hash' > $(OUT)/state-b.txt
	@cmp -s $(OUT)/state-a.txt $(OUT)/state-b.txt && echo "state hash    same after save and load: $$(cut -c15- $(OUT)/state-a.txt)" || { echo "state hash DIFFERS:"; cat $(OUT)/state-a.txt $(OUT)/state-b.txt; exit 1; }
	@python3 -I -c 'import sys, wave; a, b = (wave.open(f).readframes(1 << 30) for f in sys.argv[1:]); print("audio         " + ("resumed run matches the uninterrupted one" if a.endswith(b) else "DIFFERS")); sys.exit(not a.endswith(b))' $(OUT)/state-a.wav $(OUT)/state-b.wav

bench: release ## Timed fixed run into a race: speed vs real time and the state hash (must not change with speed-ups)
	$(RUN) --input $(BENCH_INPUT) run $(BENCH_STEPS) | grep -E '^(instructions|emulated|host time|state hash)'

copro-disasm: release ## Disassemble the geometry program captured by `make run`
	$(RUN) copro-disasm

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

music: release ## DSB2 music from the stream `make run` captured -> OUT/music.wav; SOUNDTRACK=name replaces tracks (OUT/music-name.wav)
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
