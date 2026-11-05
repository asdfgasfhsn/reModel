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
BENCH_STEPS ?= 600000000
BENCH_INPUT ?= coin1@1100-1104,start@1250-1254
# Capture sessions for `make assets` (topskatr): menus stepping through every
# skater, then a Novice and an Expert race with some steering.
ASSET_MENUS ?= coin1@1100-1104,start@1250-1254,selright@2000-2004,selright@2070-2074,selright@2140-2144,selright@2210-2214,selright@2280-2284,selright@2350-2354,selright@2420-2424,selright@2490-2494,selright@2560-2564,selright@2630-2634
ASSET_STEER ?= curving=0x50@3000-3400,curving=0xb0@3600-3900,curving=0x60@4300-4700,curving=0xa0@5200-5500,curving=0x58@6200-6600,curving=0xa8@7300-7600
ASSET_RACE_STEPS ?= 1600000000

# Where `make m68k-tests` keeps the SingleStepTests 68000 suite.
M68K_TESTS     ?= out/m68k-tests
M68K_TESTS_API := https://api.github.com/repos/SingleStepTests/680x0/contents/68000/v1
M68K_TESTS_RAW := https://raw.githubusercontent.com/SingleStepTests/680x0/main/68000/v1

CARGO   ?= cargo
# RECOMP=1: use the build with recompiled i960 and TGP code (see `make recomp`).
RECOMP  ?=
ifneq ($(RECOMP),)
REMODEL := target/recomp/release/remodel
# No debug info: the generated code is large and only slows the build.
RELEASE_ENV := REMODEL_RECOMP_DIR=$(abspath out/recomp) CARGO_TARGET_DIR=target/recomp CARGO_PROFILE_RELEASE_DEBUG=false
else
REMODEL := target/release/remodel
RELEASE_ENV :=
endif
RUN     := $(REMODEL) --game $(GAME) --rom $(ROM)

.DEFAULT_GOAL := help
.PHONY: help build release test lint fmt fmt-check check ci games verify boot disasm analyze run copro-disasm m68k-tests clean recomp bench sound sound-disasm

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

verify: release ## Check every chip in ROM= against known-good hashes
	$(RUN) verify

boot: release ## Print the i960 boot record and system-table entry points
	$(RUN) boot

disasm: release ## Linear disassembly: make disasm ADDR=0x6f0 COUNT=64
	$(RUN) disasm $(ADDR) $(COUNT)

analyze: release ## Recursive-descent code discovery; listing goes to OUT=
	$(RUN) analyze $(OUT)/program.lst

run: release ## Boot and report; PNG every SNAP frames; scripted INPUT (see remodel usage)
	$(RUN) $(VIEW) $(if $(PACK),--texture-pack $(PACK)) $(if $(INPUT),--input $(INPUT)) run $(STEPS) $(SNAP)

.PHONY: assets
assets: release ## Extract the game's art for review: capture sessions, then distinct textures, screens and contact sheets in OUT/assets (about 15 min)
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

capture: release ## Run into a race saving every 300th frame's renderer inputs to out/GAME/capture (~200 MB)
	rm -rf $(OUT)/capture
	$(RUN) --capture $(OUT)/capture --input $(BENCH_INPUT) run 900000000 300 >/dev/null

gpu-check: release ## Draw the captured frames on the software and GPU rasterisers; time and compare them (needs a GPU)
	$(RUN) gpu-check

lockstep: ## Run the recompiled i960 code beside the interpreter, block by block, and stop at the first difference (RECOMP=1, STEPS)
	$(MAKE) release RECOMP=1
	target/recomp/release/remodel --game $(GAME) --rom $(ROM) lockstep $(STEPS)

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
