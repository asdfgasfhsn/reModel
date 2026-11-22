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
# How many emulator runs a target that starts several (materials-identity,
# web's screenshots) has going at once.
JOBS  ?= 3
# `make release-tag`: the version to tag (x.y.z); DRY_RUN=1 does the checks only.
VERSION ?=
DRY_RUN ?=
SKIP_BUILD ?=
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
# tools/lock (docs/uplift/agent-environment.md): builds, test runs and GPU
# renders each take their own lock, so a render does not queue behind a
# build. Nested takes are free. LOCK_WAIT: seconds to wait before giving up
# (exit 3, naming the holder; tools/lock status shows who). Hand-typed
# `flock out/heavy.lock` is still honoured by build and test.
LOCK        := $(CURDIR)/tools/lock
LOCK_WAIT   ?= 1800
LOCK_BUILD  := $(LOCK) build --wait $(LOCK_WAIT) --
LOCK_TEST   := $(LOCK) build,test --wait $(LOCK_WAIT) --
LOCK_RENDER := $(LOCK) render --wait $(LOCK_WAIT) --
# The browser build needs a toolchain with the wasm32 target and
# wasm-bindgen-cli matching Cargo.lock. Default: rustup's stable cargo, from
# ~/.local/share/rustup-wasm/bin (a rustup kept out of ~/.cargo/bin, where
# its proxies would shadow the system toolchain's `cargo fmt` and clippy)
# or ~/.cargo/bin.
WASM_RUSTUP  ?= $(firstword $(wildcard $(HOME)/.local/share/rustup-wasm/bin/rustup) $(HOME)/.cargo/bin/rustup)
WASM_CARGO   ?= RUSTC=$(shell $(WASM_RUSTUP) which rustc) $(shell $(WASM_RUSTUP) which cargo)
WASM_BINDGEN ?= $(HOME)/.cargo/bin/wasm-bindgen
WEB_OUT      ?= out/web
RECOMP_REMODEL := target/recomp/release/remodel
# RECOMP=1: use the build with recompiled i960 and TGP code (see `make recomp`).
RECOMP  ?=
ifneq ($(RECOMP),)
REMODEL := $(RECOMP_REMODEL)
PLAY    := target/recomp/release/remodel-play
# Built from the generated workspace in out/recomp, which links the
# registry of recompiled programs in place of the crates/recomp stub. No
# debug info: the generated code is large and only slows the build.
RECOMP_CARGO := $(LOCK_BUILD) env CARGO_TARGET_DIR=target/recomp CARGO_PROFILE_RELEASE_DEBUG=false $(CARGO) build --release --manifest-path out/recomp/Cargo.toml
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
release_build = $(LOCK_BUILD) $(CARGO) build --release -p $(1)
endif
# `make agent-setup` writes out/agent.env (CARGO_TARGET_DIR, REMODEL_ACTOR,
# a PATH to cargo, and with ART=1 REMODEL= a pinned prebuilt binary).
-include out/agent.env
# Exported only when set: an empty CARGO_TARGET_DIR stops cargo.
$(foreach v,CARGO_TARGET_DIR REMODEL_ACTOR REMODEL_SCRATCH,$(if $($(v)),$(eval export $(v))))
ifneq ($(CARGO_BIN_DIR),)
export PATH := $(CARGO_BIN_DIR):$(PATH)
endif
# A binary given as REMODEL= (or pinned by agent.env) is used as it is: the
# targets that need `release` do not build.
PINNED := $(or $(filter command line,$(origin REMODEL)),$(REMODEL_PINNED))
ifneq ($(PINNED),)
$(eval release_build_real = $(value release_build))
release_build = $(if $(filter remodel,$(1)),@echo "using the given binary $(REMODEL) (no build)",$(call release_build_real,$(1)))
endif
# JIT=1: run the CLI's i960 on the JIT, translated to machine code as it
# runs (docs/jit.md); needs no recompiled build. The player has the JIT on
# by default; JIT=0 turns it off there.
JIT     ?=
# Instructions `make jit-lockstep` compares.
JIT_STEPS ?= 300000000
# Every remodel and remodel-play run goes through tools/capped: its own
# memory cap (MEMCAP, systemd's MemoryMax, default 8G; empty: none), so a
# runaway run is stopped alone instead of starving the machine. No cap
# where there's no systemd user session (macOS, Windows).
MEMCAP  ?= 8G
export MEMCAP
CAP     := $(CURDIR)/tools/capped
RUN     := $(CAP) $(REMODEL) --game $(GAME) --rom $(ROM) $(if $(JIT),--jit)

# $(call compare_hashes,A,B,WHAT): the state hash lines in files A and B must
# be the same, or fail showing both.
compare_hashes = @cmp -s $(1) $(2) && echo "state hash    $(3): $$(cut -c15- $(1))" || { echo "state hash DIFFERS:"; cat $(1) $(2); exit 1; }

.DEFAULT_GOAL := help
# Every target with a `## description` is phony (none of them makes a file).
.PHONY: $(shell sed -n 's/^\([a-zA-Z0-9_-]*\):.*## .*/\1/p' $(firstword $(MAKEFILE_LIST)))

##@ General

help: ## Show this help (the variables a target takes are in its description)
	@awk 'BEGIN {FS = ":.*## "} \
		/^##@/ {rows[n++] = "\n" substr($$0, 5)} \
		/^[a-zA-Z0-9_-]+:.*## / {rows[n++] = $$1 FS $$2; if (length($$1) > w) w = length($$1)} \
		END {printf "usage: make <target> [VAR=value]\n"; \
			for (i = 0; i < n; i++) {if (index(rows[i], FS)) {split(rows[i], f, FS); printf "  %-*s %s\n", w, f[1], f[2]} else print rows[i]}}' $(MAKEFILE_LIST)
	@printf "\ncurrent: GAME=%s ROM=%s OUT=%s\n" "$(GAME)" "$(ROM)" "$(OUT)"

##@ Build and test

build: ## Debug build of the whole workspace
	$(LOCK_BUILD) $(CARGO) build --workspace

release: ## Optimised build of the remodel CLI (RECOMP=1: with the recompiled game code)
	$(call release_build,remodel)

recomp: ## Recompile the game's i960 code and the TGP programs captured by `make run` into crates in out/recomp and build them (only what changed); then use RECOMP=1
	$(LOCK_BUILD) $(CARGO) build --release -p remodel
	$(CAP) target/release/remodel --game $(GAME) --rom $(ROM) recomp
	$(MAKE) release RECOMP=1

recomp-workspace: ## Rewrite out/recomp's Cargo workspace and program crate manifests from the programs there (after updating reModel or moving the checkout)
	$(LOCK_BUILD) $(CARGO) build --release -p remodel
	$(CAP) target/release/remodel recomp-workspace

# GPU_TESTS=0 skips the tests that need a GPU (they return early, as on a
# machine without one), for when the card's memory is full; never for a
# change to the GPU code, and run the full check before pushing.
GPU_TESTS ?= 1
test: ## Run all unit tests (no ROMs needed); GPU_TESTS=0 skips the GPU ones, with a warning (for a busy GPU; run them before pushing)
	$(LOCK_TEST) env REMODEL_GPU_TESTS=$(GPU_TESTS) $(CARGO) test --workspace
	@if [ "$(GPU_TESTS)" = 0 ]; then echo "*** GPU tests SKIPPED (GPU_TESTS=0): this check is incomplete; run them before pushing ***"; fi

lint: ## Clippy across all crates and targets, warnings are errors
	$(LOCK_BUILD) $(CARGO) clippy --workspace --all-targets -- -D warnings

fmt: ## Format all Rust code
	$(CARGO) fmt --all

fmt-check: ## Fail if any Rust code is unformatted
	$(CARGO) fmt --all -- --check

check: fmt-check lint lock-test test web-check ## Everything a change must pass before commit

web-check: ## Type-check the browser build (remodel-web for wasm32: its wasm-only code, which `lint` and `test` don't compile); skipped with a note where the wasm toolchain isn't installed
	@if [ -x "$(WASM_RUSTUP)" ] && $(WASM_RUSTUP) target list --installed 2>/dev/null | grep -qx wasm32-unknown-unknown; then \
		$(LOCK_BUILD) env $(WASM_CARGO) check --quiet --release --target wasm32-unknown-unknown --target-dir target/web -p remodel-web 2>&1 | tee target/web-check.log; \
		! grep -qE '^(warning|error)' target/web-check.log; \
	else \
		echo "web-check: no wasm32 toolchain ($(WASM_RUSTUP)), skipped"; \
	fi

ci: check ## Alias for check

pace-sim: ## The browser's audio rate control on synthetic frame timings (steady, phone jitter, 30 Hz, slowdowns): pitch movement and underruns, before and after (docs/web.md)
	$(LOCK_TEST) $(CARGO) test --release -p remodel-model2 --test pace_sim -- --ignored --nocapture

m68k-tests: $(M68K_TESTS)/.done ## Run the 68000 core against the SingleStepTests suite (downloaded to out/ on first use)
	$(LOCK_TEST) env M68K_TESTS=$(abspath $(M68K_TESTS)) $(CARGO) test --release -p m68k --test singlestep -- --ignored --nocapture

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
		$(LOCK_TEST) env Z80_TESTS=$(abspath $(Z80_TESTS)) $(CARGO) test --release -p remodel-z80 --test zex -- --ignored --nocapture --test-threads 2; \
	else \
		echo "no zexdoc.com or zexall.com in $(Z80_TESTS). They are Frank Cringle's freely redistributable"; \
		echo "Z80 exercisers (CP/M .com files); fetch them into $(Z80_TESTS), e.g.:"; \
		echo "  mkdir -p $(Z80_TESTS)"; \
		echo "  curl -fsSL -o $(Z80_TESTS)/zexdoc.com $(Z80_TESTS_URL)/zexdoc.com"; \
		echo "  curl -fsSL -o $(Z80_TESTS)/zexall.com $(Z80_TESTS_URL)/zexall.com"; \
		exit 1; \
	fi

jit-test: ## Unit tests of the JITs (crates/jit: the i960's and the copros', each against its interpreter on hand-made and random programs; release build, as the random runs are long)
	$(LOCK_TEST) $(CARGO) test --release -p remodel-jit

sharc-test: ## ADSP-21062 (SHARC) unit tests; then decode, discovery and a boot of the 2B programs in SHARC_PROGRAMS= (default: those captured in out/overrevb)
	$(LOCK_TEST) $(CARGO) test -p adsp2106x
	@if [ -n "$(SHARC_PROGRAMS)" ]; then \
		$(MAKE) --no-print-directory release && $(CAP) $(REMODEL) sharc-check $(SHARC_PROGRAMS); \
	else \
		echo "no SHARC program images in out/overrevb (docs/hw/adsp2106x.md): unit tests only"; \
	fi

##@ Agents and shared-machine locks (docs/uplift/agent-environment.md)

lock-status: ## Who holds the build, test, render and emulate locks, who waits and for how long (tools/lock status; --json: LOCK_STATUS_ARGS=--json); a hand-typed `flock out/heavy.lock` shows too
	@$(LOCK) status $(LOCK_STATUS_ARGS)

lock-test: ## Test tools/lock in a private lock folder: two holders of a one-slot class, a bounded wait that expires (exit 3, naming the holder), a dead holder cleared, three renders at once and a fourth waiting, nesting, status, the hand-typed heavy.lock honoured and not deadlocked
	@sh tools/lock-test.sh

agent-env-test: release ## Slice A's acceptance test (docs/uplift/agent-environment.md), no ROM or GPU: with a build and a test run holding their locks, a pinned binary answers img stats, diff and sheet in under 2 s without waiting, make builds nothing for a given REMODEL, and studio and img commands give parseable {ok, code, error, data, warnings, next} answers with the exit codes 0, 1, 2 and 3; AGENT_ENV_BIN=FILE tests another binary
	@sh tools/agent-env-test.sh $(or $(AGENT_ENV_BIN),$(CURDIR)/$(REMODEL))

agent-setup: ## Set up this checkout or worktree for an agent: link rom/, copy each game's EEPROM from the main checkout, make out/scratch/agent-ACTOR, write out/agent.env (its own target-agents/ACTOR/, REMODEL, a private output folder for run); ART=1 pins a prebuilt binary (agent-bin) so no build is needed; prints what is available as JSON
	@tools/agent-setup $(if $(ART),--art) $(if $(ACTOR),--actor $(ACTOR))

agent-bin: ## Build the remodel CLI once and keep a read-only copy in out/bin/COMMIT/ (never overwritten; a changed tree adds a hash to the folder name); art agents run it with REMODEL=out/bin/COMMIT/remodel and never build
	$(call release_build,remodel)
	@tools/agent-setup bin

##@ ROM tools (need a user-supplied dump in ROM=)

games: release ## List games with manifests
	$(CAP) $(REMODEL) games

inputs: release ## The game's inputs: names for INPUT=, ports and channels, default keys and pad bindings
	$(CAP) $(REMODEL) --game $(GAME) inputs

verify: release ## Check every chip in ROM= against known-good hashes
	$(RUN) verify

scan: release ## List the sets in ROM= (MAME zips or folders) and check the supported ones
	$(RUN) scan

boot: release ## Print the i960 boot record and system-table entry points
	$(RUN) boot

link-run: release ## Experimental, off by default: NODES (default 2; up to 4 srallyc, manxtt, indy500, motoraid, waverunr, sgt24h, overrev, skisuprg, 7 stcc, 8 daytona, 2 von) real machines of GAME (srallyc, daytona, manxtt, indy500, motoraid, waverunr, sgt24h, overrev, von, stcc, skisuprg: games with an entry in crates/romset/src/games/link.rs) in one process joined by a link hub, run in lockstep, the game's link role setting patched per node (master first); writes side-by-side pictures and the handshake and block log to OUT (FRAMES, INPUT0, INPUT1 or INPUT, DELAY = frames the other nodes' blocks take, default 1, SHOT_EVERY, LINK_OPTS="--save-at F FILE --load FILE --drop-at F --freeze NODE:F --trace NODE --shot-at F,F"; docs/hw/link-board.md)
	$(RUN) --out $(OUT) $(if $(INPUT),--input $(INPUT)) link-run --nodes $(or $(NODES),2) --frames $(or $(FRAMES),6000) --delay $(or $(DELAY),1) $(if $(SHOT_EVERY),--shot-every $(SHOT_EVERY)) $(if $(INPUT0),--input0 $(INPUT0)) $(if $(INPUT1),--input1 $(INPUT1)) $(LINK_OPTS)

link-net: release ## Networked link play without a window: one machine of GAME (srallyc, daytona, manxtt, indy500, motoraid, waverunr, sgt24h, overrev, von, stcc, skisuprg) here, the others on other hosts, the blocks over UDP in lockstep: NET="--host 47890 --nodes 2 --delay 3" on one host, NET="--join HOST:PORT" on the others (FRAMES, INPUT; fault injection --loss --latency --jitter --dup; prints this node's state hash as link-run does; docs/play.md, "Network link play")
	$(RUN) --out $(OUT) $(if $(INPUT),--input $(INPUT)) link-net $(NET) --frames $(or $(FRAMES),3000)

link-browse: release ## List the hosts of a network link game found on this network (mDNS and broadcast; SECS to look, default 3; GAME to judge which could be joined): name, game, players, address, version, ROM hash, delay and whether this machine can join each (docs/play.md, "Network link play")
	$(RUN) link-net --browse $(or $(SECS),3)

link-net-test: release ## Networked link play over loopback, two processes per case, srallyc, daytona, manxtt, indy500, motoraid, waverunr, sgt24h and overrev, and von, stcc and skisuprg (GAMES; also the variants daytonas, daytonase, indy500d, indy500to): each host's state hash equals link-run's for its node, clean and under 5% loss with 50 ms jitter; player names in the lobby and the messages, the host found by mDNS (--browse) with its TXT record before it is joined; a polite leave, a kill, a settings byte that differs and a damaged record end cleanly with a message; the delay D changing mid-session (a scripted plan under faults, and the adaptive rule paced with Wi-Fi-like faults, where D rises and on a clean link does not; each host's hash equals `link-run --delay-at` with the logged schedule), and players leaving mid-race with two and three hosts (also with a delay change in flight) (a polite leave, a kill, the host leaving; with the leaver's newest records held back from one survivor so the other must relay them): the survivors run to the last frame and end on the hashes of `link-run --leave NODE:F` for the drop frame they logged (needs the ROMs; about 5 minutes a game)
	tools/link-net-test.sh $(CURDIR)/$(REMODEL) $(ROM) out/link-net-test $(GAMES)

link-free-play-test: release ## Free play in linked play, srallyc, daytona, manxtt, indy500, motoraid, waverunr, sgt24h, overrev, overrevb, von, stcc and skisuprg (GAMES; also the variants daytonas, daytonase, indy500d, indy500to): a two-node race started with start presses only (no coins) on the game's own FREE PLAY setting: deterministic, differs from the same inputs without it, and over loopback a host with --free-play and a joiner without end on link-run's hashes; race pictures in out/link-free-play-test/GAME/captures (needs the ROMs; about a minute a game)
	tools/link-free-play-test.sh $(CURDIR)/$(REMODEL) $(ROM) out/link-free-play-test $(GAMES)

link-record-test: release ## The settings records linked play writes against what the games write, manxtt, indy500, motoraid, waverunr, sgt24h, overrev and overrevb, and von, stcc, stcca, stccb and skisuprg (GAMES; also the variants daytonas, daytonase, indy500d, indy500to): the checksum of each copy of the record the game saves from its test menu, the role bytes of a two-node link-run surviving in the game's own EEPROM, and a changed setting's save (docs/hw/eeprom-records.md; needs the ROMs; about 2 minutes)
	tools/link-record-test.sh $(CURDIR)/$(REMODEL) $(ROM) out/link-record-test $(GAMES)

link-trace: release ## Experimental, off by default: run GAME with a link board fitted (LINK=idle, or LINK=fake,ID,TOTAL: a partner that mirrors the game's own block; LINK_DELAY, LINK_FREEZE, LINK_DROP: frames) and write every access to 0x1A00000-0x1A1FFFF to OUT/link-trace.txt (INPUT, STEPS, SNAP as `run`; docs/hw/link-board.md)
	REMODEL_LINK=$(or $(LINK),idle) REMODEL_LINK_DELAY=$(LINK_DELAY) REMODEL_LINK_FREEZE=$(LINK_FREEZE) REMODEL_LINK_DROP=$(LINK_DROP) $(RUN) $(if $(INPUT),--input $(INPUT)) run $(STEPS) $(SNAP)

disasm: release ## Linear disassembly: make disasm ADDR=0x6f0 COUNT=64
	$(RUN) disasm $(ADDR) $(COUNT)

analyze: release ## Recursive-descent code discovery; listing goes to OUT=
	$(RUN) analyze $(OUT)/program.lst

run: release ## Boot and report; PNG every SNAP frames; scripted INPUT (see remodel usage; INPUT=coin-start coins up and starts)
	$(RUN) $(VIEW) $(if $(PACK),--texture-pack $(PACK)) $(if $(INPUT),--input $(INPUT)) run $(STEPS) $(SNAP)

assets: release ## Extract topskatr's art for review: capture sessions, then distinct textures, screens and contact sheets in OUT/assets (about 15 min)
	rm -rf $(OUT)/assets && mkdir -p $(OUT)/assets/raw
	$(RUN) --out $(OUT) --dump-textures $(OUT)/assets/raw/attract run 900000000 20 > $(OUT)/assets/raw/attract.log
	cp -r $(OUT)/frames $(OUT)/assets/frames-attract
	$(RUN) --out $(OUT) --dump-textures $(OUT)/assets/raw/menus --input $(ASSET_MENUS) run 480000000 10 > $(OUT)/assets/raw/menus.log
	cp -r $(OUT)/frames $(OUT)/assets/frames-menus
	$(RUN) --dump-textures $(OUT)/assets/raw/novice --input coin1@1100-1104,start@1250-1254,$(ASSET_STEER) \
		run $(ASSET_RACE_STEPS) 15 > $(OUT)/assets/raw/novice.log & \
	$(RUN) --dump-textures $(OUT)/assets/raw/expert --input coin1@1100-1104,start@1250-1254,selright@1330-1336,$(ASSET_STEER) \
		run $(ASSET_RACE_STEPS) 15 > $(OUT)/assets/raw/expert.log & \
	wait
	python3 -I tools/asset-review.py $(OUT)/assets

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
# Through the environment, so the template's quotes reach the tool as written.
upscale-textures: export UPSCALER_CMD = $(UPSCALER)
upscale-textures: release ## Upscale TEXDUMP (default OUT/texdump, from dump-textures) into texture pack TEXPACK (OUT/texpack-SCALEx) with UPSCALER= (empty: built-in); wrap-aware padding, colour matched to the dump (COLOUR=keep: off), hard alpha, only new or changed dumps
	$(CAP) $(REMODEL) --game $(GAME) texpack-upscale $(TEXDUMP) $(TEXPACK) --scale $(SCALE) --tiles $(TILES) --colour $(COLOUR) \
		--min-uses $(MIN_USES) --max-side $(MAX_SIDE) --work out/scratch/texpack-upscale-$(GAME) \
		$(if $(UPSCALER),--upscaler "$$UPSCALER_CMD") $(if $(FORCE),--force)

# Normal maps for texture pack TEXPACK (docs/texture-packs.md, "Material
# maps"). NORMALGEN: an external tool's command with {in} and {out} filled
# in, run once over the directory (empty: built-in, from luminance);
# NORMAL_Y: which way its green points (up|down); STRENGTH, DETAIL: the
# built-in's slope gain and the width in texels of brightness changes kept
# out of the relief (empty: the tool's defaults, 2 and 8). TEXDUMP= (empty: the
# pack's own images) gives wrap=, surf= and uses=.
NORMALGEN ?=
NORMAL_Y  ?= up
STRENGTH  ?=
DETAIL    ?=
pack-materials: export NORMALGEN_CMD = $(NORMALGEN)
pack-materials: release ## Make normal maps for texture pack TEXPACK from its images (or TEXDUMP's where it has none) with NORMALGEN= (empty: built-in, from luminance); STRENGTH (default 2), DETAIL (default 8), MIN_USES, FORCE; wrap-aware, neutral under cut-outs, one per surface, keeps hand-made and edited maps, skips the textures listed in the pack's materials-exclude.txt; a starting point to edit by hand
	$(CAP) $(REMODEL) --game $(GAME) texpack-materials $(TEXPACK) $(if $(TEXDUMP),--dump $(TEXDUMP)) \
		--min-uses $(MIN_USES) --work out/scratch/texpack-materials-$(GAME) \
		$(if $(STRENGTH),--strength $(STRENGTH)) $(if $(DETAIL),--detail $(DETAIL)) \
		$(if $(NORMALGEN),--normalgen "$$NORMALGEN_CMD" --normal-y $(NORMAL_Y)) $(if $(FORCE),--force)

# pack-lint checks against a dump only when TEXDUMP is given.
pack-lint: release ## Check texture pack PACK= against the pack format, no ROM or GPU: pack.txt, file names, index.txt, every PNG decodes within MAX_SIDE, image and map proportions, unit normals, maps under cut-outs, materials-exclude.txt entries, specular and gloss maps (a note: loaded, no effect yet), GPU memory against the budgets (TEXDUMP= adds mirror and repeat seams, never-drawn and image-or-surface checks); fails on errors
	$(if $(PACK),,$(error pack-lint needs PACK=DIR, a texture pack))
	$(CAP) $(REMODEL) pack-lint $(PACK) --max-side $(MAX_SIDE) \
		$(if $(filter command line environment,$(origin TEXDUMP)),--dump $(TEXDUMP))

play: ## Play in a native window: the launcher (carousel or grid of the games, live attract previews, quick resume, saves, settings), or straight into GAME= if given; GPU, live sound, gamepads, Esc menu, save states (on the JITs; JIT=0: without them; RECOMP=1 JIT=0: recompiled code; PLAY_ARGS); PLAYERS=2..4 with GAME= (srallyc, daytona, manxtt, indy500, motoraid, waverunr, sgt24h, overrev): that many linked machines on one split screen, one controller each (docs/play.md, "Link play")
	$(call release_build,remodel-play)
	$(CAP) $(PLAY) $(if $(filter command line environment,$(origin GAME)),--game $(GAME)) $(if $(PLAYERS),--players $(PLAYERS)) --rom $(ROM) $(if $(filter 0,$(JIT)),--no-jit) $(PLAY_ARGS)

stick-log: ## Play with a log of the gamepad's sticks, to measure how noisy a pad is: hold the stick still for 5 s, then steer; a CSV line a frame to STICK_LOG (default out/stick-log.csv) and, on quitting, each stick's mean, spread and largest offset at rest (GAME= to pick the game, default daytona; docs/play.md, "Measuring a pad")
	$(call release_build,remodel-play)
	$(CAP) $(PLAY) --game $(if $(filter command line environment,$(origin GAME)),$(GAME),daytona) --rom $(ROM) $(if $(filter 0,$(JIT)),--no-jit) --stick-log $(or $(STICK_LOG),out/stick-log.csv) $(PLAY_ARGS)

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
# DIST_TARGET= builds for another target (x86_64-apple-darwin on an Apple
# silicon Mac, its std from `rustup target add`). Windows gets a .zip with
# remodel-play.exe (7-Zip packs it), everything else a .tar.gz.
DIST_TARGET_DIR := target/dist
DIST_OUT        ?= out/dist
DIST_TARGET     ?=
PYTHON          ?= python3

dist: ## Distributable player for this platform (interpreter only, stripped; DIST_TARGET= another target) -> out/dist/remodel-VERSION-TARGET.tar.gz (.zip on Windows)
	@test -z "$(RECOMP)" || { echo "make dist builds the interpreter only: recompiled code is derived from the game, so it isn't distributed (drop RECOMP=1)"; exit 1; }
	CARGO_TARGET_DIR=$(DIST_TARGET_DIR) CARGO_PROFILE_RELEASE_DEBUG=false CARGO_PROFILE_RELEASE_STRIP=true \
		$(CARGO) build --release --locked --manifest-path Cargo.toml -p remodel-play $(if $(DIST_TARGET),--target $(DIST_TARGET))
	@$(CARGO) tree --locked --manifest-path Cargo.toml -p remodel-play -i remodel-recomp --prefix none | head -1 | grep -q "crates[/\\]recomp)" \
		|| { echo "dist: remodel-recomp isn't the crates/recomp stub: recompiled code in the dist build"; exit 1; }
	@set -e; \
	version=$$($(CARGO) pkgid -p remodel-play | sed 's/.*[@#]//'); \
	rev=$$(git describe --tags --always --dirty 2>/dev/null || true); \
	[ "$$rev" != "v$$version" ] || rev=; \
	target=$(DIST_TARGET); target=$${target:-$$(rustc -vV | sed -n 's/^host: //p')}; \
	case $$target in *windows*) exe=.exe ;; *) exe= ;; esac; \
	name=remodel-$$version$${rev:+-$$rev}-$$target; \
	dir=$(DIST_OUT)/$$name; \
	rm -rf $$dir $$dir.tar.gz $$dir.zip; mkdir -p $$dir; \
	cp $(DIST_TARGET_DIR)/$(if $(DIST_TARGET),$(DIST_TARGET)/)release/remodel-play$$exe $$dir/; \
	for l in $(wildcard LICENSE*); do cp $$l $$dir/; done; \
	$(PYTHON) -I tools/notices.py remodel-play $$target $$dir/THIRD-PARTY-NOTICES.txt; \
	sed -e "s|@VERSION@|$$version$${rev:+ ($$rev)}|" -e "s|@TARGET@|$$target|" crates/play/dist-README.txt > $$dir/README.txt; \
	case $$target in \
	*windows*) archive=$$dir.zip; (cd $(DIST_OUT) && 7z a -tzip -bso0 $$name.zip $$name) ;; \
	*) archive=$$dir.tar.gz; tar -C $(DIST_OUT) -czf $$archive $$name ;; \
	esac; \
	echo $$name > $(DIST_OUT)/$$target.name; \
	echo "built $$archive ($$(du -h $$archive | cut -f1))"

# `make appimage`'s tools, pinned and checked (fetched into out/tools once).
APPIMAGETOOL_VERSION := 1.9.1
APPIMAGETOOL_SHA256  := ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
APPIMAGE_RUNTIME_VERSION := 20251108
APPIMAGE_RUNTIME_SHA256  := 2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d
APPIMAGETOOL     := out/tools/appimagetool-$(APPIMAGETOOL_VERSION)-x86_64.AppImage
APPIMAGE_RUNTIME := out/tools/runtime-$(APPIMAGE_RUNTIME_VERSION)-x86_64

app: ## macOS: reModel.app, universal (Apple silicon and Intel; `rustup target add aarch64-apple-darwin x86_64-apple-darwin`), ad-hoc signed, in a .dmg -> out/dist/remodel-VERSION-macos-universal.dmg
	@test "$$(uname -s)" = Darwin || { echo "make app needs macOS (lipo, iconutil, codesign, hdiutil)"; exit 1; }
	$(MAKE) dist DIST_TARGET=aarch64-apple-darwin
	$(MAKE) dist DIST_TARGET=x86_64-apple-darwin
	DIST_OUT=$(DIST_OUT) sh tools/package.sh app

appimage: dist $(APPIMAGETOOL) $(APPIMAGE_RUNTIME) ## Linux x86_64: the player as an AppImage (pinned appimagetool and runtime, fetched into out/tools) -> out/dist/remodel-VERSION-x86_64.AppImage
	DIST_OUT=$(DIST_OUT) APPIMAGETOOL=$(APPIMAGETOOL) APPIMAGE_RUNTIME=$(APPIMAGE_RUNTIME) sh tools/package.sh appimage

$(APPIMAGETOOL):
	@mkdir -p $(@D)
	curl -fsSLo $@.part https://github.com/AppImage/appimagetool/releases/download/$(APPIMAGETOOL_VERSION)/appimagetool-x86_64.AppImage
	echo "$(APPIMAGETOOL_SHA256)  $@.part" | sha256sum -c --quiet
	chmod +x $@.part && mv $@.part $@

$(APPIMAGE_RUNTIME):
	@mkdir -p $(@D)
	curl -fsSLo $@.part https://github.com/AppImage/type2-runtime/releases/download/$(APPIMAGE_RUNTIME_VERSION)/runtime-x86_64
	echo "$(APPIMAGE_RUNTIME_SHA256)  $@.part" | sha256sum -c --quiet
	mv $@.part $@

web: ## Browser build into out/web: game list, a page per game (model2/SET/), screenshots (JOBS at once, default 3); RECOMP=1 gives each game its own module with its recompiled code; GAME= rebuilds just that game; unchanged modules are skipped (docs/web.md)
	CAP=$(CAP) WEB_GAME=$(if $(filter command line environment,$(origin GAME)),$(GAME)) WEB_OUT=$(WEB_OUT) ROM=$(ROM) RECOMP=$(RECOMP) CARGO="$(CARGO)" WASM_CARGO="$(WASM_CARGO)" WASM_BINDGEN=$(WASM_BINDGEN) JOBS=$(JOBS) sh tools/web.sh

capture: release ## Run into a race saving every 300th frame's renderer inputs to out/GAME/capture (~200 MB)
	rm -rf $(OUT)/capture
	$(RUN) --capture $(OUT)/capture --input $(BENCH_INPUT) run 900000000 300 >/dev/null

# Where texmap reads captures (default OUT/capture, made by `make capture` if missing) and writes.
CAPTURE     ?=
TEXMAP_OUT  ?= $(OUT)/texmap
TEXMAP_OPTS ?=
texmap: release ## Which texture is where: draw the captured frames (CAPTURE=DIR, default OUT/capture, captured first if missing) and write to TEXMAP_OUT (default OUT/texmap) per-frame ID images, texel-position images, contact sheets and inventories, run-wide inventory.tsv, groups.tsv, texmap.json and a static viewer index.html (VIEW="--height 1080 --aspect 16:9", TEXMAP_OPTS="--every N --viewer-frames N --group-cap N"; docs/uplift-studio.md)
	$(if $(CAPTURE),,@[ -d $(OUT)/capture ] || $(MAKE) capture)
	$(RUN) $(VIEW) texmap $(or $(CAPTURE),$(OUT)/capture) $(TEXMAP_OUT) $(TEXMAP_OPTS)

# The catalogue of everything a game shows (docs/uplift-studio.md,
# "Catalogue"): out/GAME/catalogue, grown by sessions.
CATALOGUE_DIR   ?= $(OUT)/catalogue
CATALOGUE_STEPS ?= $(BENCH_STEPS)
# Tours and the ROM read maps (docs/uplift-studio.md, "Tours and coverage").
TOUR_OPTS ?=
COVERAGE_OPTS ?=
COVERAGE_GAMES ?= topskatr vf2 daytona
OUT_ROOT ?= out/scratch
# `make catalogue-runs`: a file of runs, one per line: LABEL|STEPS|INPUT
# (INPUT as for --input; `#` lines and blank lines are skipped).
RUNS ?=
catalogue: release ## Observe a scripted run into the catalogue (INPUT=script, default coin-start; CATALOGUE_STEPS=N, default BENCH_STEPS; LABEL=text; CATALOGUE_OPTS="--catalogue-every N"), then write the report and viewer; sessions add up, so run it again with another INPUT to grow the catalogue (docs/uplift-studio.md)
	$(RUN) --catalogue $(CATALOGUE_DIR) --catalogue-label "$(or $(LABEL),$(or $(INPUT),$(BENCH_INPUT)))" $(CATALOGUE_OPTS) --input "$(or $(INPUT),$(BENCH_INPUT))" run $(CATALOGUE_STEPS) 0 | grep -E '^catalogue'
	$(RUN) catalogue report --dir $(CATALOGUE_DIR) | head -30
	$(RUN) catalogue viewer --dir $(CATALOGUE_DIR)
catalogue-runs: release ## Observe every run listed in RUNS=FILE (lines LABEL|STEPS|INPUT) into the catalogue, one session each, then report and viewer: attract, every course, every character, the menus
	@test -n "$(RUNS)" || { echo "usage: make catalogue-runs RUNS=FILE (lines LABEL|STEPS|INPUT)"; exit 1; }
	@grep -vE '^[[:space:]]*(#|$$)' $(RUNS) | while IFS='|' read -r label steps input; do \
		echo "== $$label"; \
		$(RUN) --catalogue $(CATALOGUE_DIR) --catalogue-label "$$label" $(CATALOGUE_OPTS) --input "$$input" run $$steps 0 | grep -E '^catalogue' || exit 1; \
	done
	$(RUN) catalogue report --dir $(CATALOGUE_DIR) | head -30
	$(RUN) catalogue viewer --dir $(CATALOGUE_DIR)
tour-list: release ## List GAME's tours (input scripts from power-on, crates/romset/src/games/tours/; TOUR_OPTS="--tours FILE --json"): length, branch point, marks, whether each has a read map (docs/uplift-studio.md, "Tours and coverage")
	$(RUN) tour list $(TOUR_OPTS)
tour-run: release ## Run GAME's tours (TOUR=name, default all; TOUR_OPTS="--tours FILE --dir DIR --json") from power-on with the ROM read maps on, writing out/GAME/coverage/LABEL.map and a state at each mark; same state hashes as without maps (the default is the native geometrizer; --geo chip runs the game's own)
	$(RUN) tour run $(or $(TOUR),--all) $(TOUR_OPTS)
coverage: release ## How much of GAME's polygon and texture ROM its tours have read, by region, with the largest unread stretches by address (COVERAGE_OPTS="--top N --json"; needs no ROM, run `make tour-run` first)
	$(RUN) coverage $(COVERAGE_OPTS)
coverage-check: release ## Acceptance test for tours and coverage on COVERAGE_GAMES (default topskatr vf2 daytona): the bench tour's state hash with the maps on is a plain run's (BENCH_STEPS of coin-start), a tour run twice writes the same map, the answers have the six envelope fields, coverage with no maps exits 3
	sh tools/coverage-check.sh $(REMODEL) $(ROM) $(OUT_ROOT)/coverage-check $(BENCH_STEPS) $(COVERAGE_GAMES)
catalogue-report: release ## The catalogue's coverage report: what is seen, what is new since the last session, screens and how complete they look, loaded vs drawn (also CATALOGUE_DIR/report.txt)
	$(RUN) catalogue report --dir $(CATALOGUE_DIR)
catalogue-viewer: release ## Write CATALOGUE_DIR/index.html, the static browser of the catalogue (gallery by kind and screen, where each entry appears, what it appears with, loads)
	$(RUN) catalogue viewer --dir $(CATALOGUE_DIR)
catalogue-glyphs: release ## Infer which tile shows which glyph (digits) from a run's changing tile layer: INPUT=script, CATALOGUE_STEPS=N, GLYPH_OPTS="--glyph-counts up|down"; writes CATALOGUE_DIR/glyphs/glyphs.tsv (tile key, glyph, quarter) and glyphs.txt (docs/uplift-studio.md, "Glyph mapping")
	$(RUN) --glyphs $(CATALOGUE_DIR)/glyphs $(GLYPH_OPTS) --input "$(or $(INPUT),$(BENCH_INPUT))" run $(CATALOGUE_STEPS) 0 | grep -E '^glyphs'

# The uplift studio (docs/uplift-studio.md): a project is a directory
# (STUDIO, default OUT/studio/default); STUDIO_ARGS is the subcommand's
# options, e.g. STUDIO_ARGS="--key 2970 --category wall --reason 'big wall'".
STUDIO      ?= $(OUT)/studio/default
STUDIO_ARGS ?=
studio-init: release ## Start a studio project in STUDIO: folders, project.txt, selection.tsv, style.md; STUDIO_ARGS="--game G --map TEXMAP_DIR [--copy-map] --style FILE --name N --capture-dir DIR" (the map links or copies what `make texmap` wrote; give --map several times, NAME=DIR, for a project over several maps)
	$(RUN) studio init $(STUDIO) $(STUDIO_ARGS)
studio-region: release ## What a frame's region draws: textures and tile cells with sizes, groups, surfaces (STUDIO_ARGS="--frame 3000 --region X,Y,W,H [--scale S]")
	$(RUN) studio region $(STUDIO) $(STUDIO_ARGS)
studio-select: release ## Add textures or tiles to STUDIO's selection.tsv with a reason, idempotently, copying their originals (STUDIO_ARGS="--key K,.. | --group N | --surface S | --frame F --region X,Y,W,H | --all  --kind tex|tile --min-pixels N --category C --reason TEXT")
	$(RUN) studio select $(STUDIO) $(STUDIO_ARGS)
studio-status: release ## STUDIO's counts (original, drafted, approved) by category, what is missing, strays and unrecorded images (STUDIO_ARGS="--tsv [--missing]" for rows an agent reads)
	$(RUN) studio status $(STUDIO) $(STUDIO_ARGS)
studio-approve: release ## Copy drafts into STUDIO's pack/ (STUDIO_ARGS="NAME.. | --drafted | --group N | --category C", --undo removes them; --replace lets a changed draft replace an approved image)
	$(RUN) studio approve $(STUDIO) $(STUDIO_ARGS)
studio-pack: release ## Build STUDIO's pack/ manifest and index.txt from the approved images and run pack-lint (STUDIO_ARGS="--approve-all --no-lint")
	$(RUN) studio pack $(STUDIO) $(STUDIO_ARGS)
studio-draft: release ## Batch generation: run an external generator (default `codex exec {images} -`, the prompt on stdin) over selected rows with a brief and reference images, JOBS at once, retried, logged, resumable (STUDIO_ARGS="--brief FILE --group N|--all|NAME.. --cmd TEMPLATE --ref IMG --jobs N --retries N --scale N --keep-size --dry-run --replace"; every studio-* target takes --json in STUDIO_ARGS: one object, with exit codes 0 done, 1 a check failed, 2 wrong request, 3 missing, 4 internal)
	$(RUN) studio draft $(STUDIO) $(STUDIO_ARGS)
studio-style: release ## Local style processor: restyle selected originals with a recipe file (palette ramp, despeckle, edge ink, halftone; wrap-aware) into draft/ (STUDIO_ARGS="--recipe FILE --group N|--category C|--all|NAME..")
	$(RUN) studio style $(STUDIO) $(STUDIO_ARGS)
studio-variants: release ## One restyled image feeding every key of its surface, tinted per key from the originals' colours (STUDIO_ARGS="--of KEY --mode luma|ratio|mean --strength S")
	$(RUN) studio variants $(STUDIO) $(STUDIO_ARGS)
studio-artworks: release ## The artworks of STUDIO's maps (a surface key at a size, with every colouring seen of it): colourings, pixels drawn, share of the screen, colours, ramps recorded, and whether pack/ has its art_ image and in which mode (STUDIO_ARGS="--tsv --min-variants 2"; docs/uplift-studio.md, "Artworks")
	$(RUN) studio artworks $(STUDIO) $(STUDIO_ARGS)
studio-bake: release ## One art_ image for an artwork's colourings: write each colouring's tex_ image from STUDIO/pack/art_*.png through the catalogue's colour ramps (baked; a copy where the image is a restyle), or leave them to be drawn as the art_ image is (fixed); a rebuild keeps hand-made tex_ images (STUDIO_ARGS="ART.. | --all [--from DIR] [--mode baked|fixed] [--dry-run] [--replace]"; then make studio-pack)
	$(RUN) studio bake $(STUDIO) $(STUDIO_ARGS)
studio-text: release ## Text to tiles: typeset text (--image FILE, or --text/--font/--size/--colour/--stroke through ImageMagick) laid over a frame region and cut into the tile_*.png replacements (STUDIO_ARGS="--frame F --region X,Y,W,H --box X,Y,W,H --fit contain --alpha original")
	$(RUN) studio text $(STUDIO) $(STUDIO_ARGS)
studio-region-capture: release ## A 2D region override cut from a captured frame's tile maps into STUDIO/regions/NAME (STUDIO_ARGS="NAME --capture-dir DIR --frame F --region X,Y,W,H [--also F,..] [--wild X,Y;..] [--plane a|b] [--pin] [--image FILE]"): its tile layout and the original to redraw at Nx (docs/uplift-studio.md, "Regions")
	$(RUN) studio region-capture $(STUDIO) $(STUDIO_ARGS)
studio-regions: release ## STUDIO's regions and which have a replacement (STUDIO_ARGS="--tsv"); `make studio-pack` puts those into pack/
	$(RUN) studio regions $(STUDIO) $(STUDIO_ARGS)
studio-seam: release ## Seam check of STUDIO's pack/ (STUDIO_ARGS="--from draft", --strict fails on suspects): per wrap mode, repeat edges against each other and edge lines on mirrored ones, against the originals
	$(RUN) studio seam $(STUDIO) $(STUDIO_ARGS)
img: release ## Image tools, no ROM (docs/uplift-studio.md, "Image tools"): IMG_ARGS="sheet OUT.png A.png B.png --cols 2" (labelled contact sheet) | "diff A.png B.png [--max-changed PCT] [--out HEAT.png]" | "stats A.png [--region X,Y,W,H]"; add --json for one object; nothing is overwritten without --replace
	$(CAP) $(REMODEL) img $(IMG_ARGS)

studio-compare: release ## Before and after (without and with STUDIO's pack/), written as side-by-side sheets to STUDIO/compare: SCENES=all (or NAME,NAME) draws the project's named scenes from their captures in seconds, with no run and no ROM (make scene-make first); without SCENES it runs the game twice (BENCH_STEPS, INPUT default BENCH_INPUT, JOBS at once) and refuses a folder that already holds a run (STUDIO_ARGS="--into DIR --frames F,.. --run-args '--height 1080 --aspect 16:9' --pack DIR --force")
	$(RUN) studio compare $(STUDIO) $(if $(SCENES),--scene $(SCENES),--steps $(BENCH_STEPS) --snap 300 --input $(or $(INPUT),$(BENCH_INPUT)) --jobs $(JOBS)) $(STUDIO_ARGS)

GATE_ARGS ?=
PROJECT_ARGS ?=
PACKS ?=
asset-check: release ## Asset gates (docs/uplift-studio.md, "Gates") on images of STUDIO: ASSET="NAME NAME" (a name, 16-digit key or file stem) or ASSET=--all; GATE_ARGS="--from pack|draft --json"; errors (file, alpha, stale) exit 1, warnings (seam, changed, noise) do not; each result states its number and limit; no ROM, no GPU, milliseconds each
	$(if $(ASSET),,$(error asset-check needs ASSET="NAME ..." or ASSET=--all))
	$(CAP) $(REMODEL) asset check --project $(STUDIO) $(ASSET) $(GATE_ARGS)
pack-gates: release ## Every gate level on STUDIO's pack/ and a report in STUDIO/reports/DATE-VERSION (report.json, gates.tsv, report.md): asset gates on each image, lint, budget, known, selected, and scene coverage when the scenes are made (a GPU, no ROM); GATE_ARGS="--level asset|pack|scene --asset NAME --release --json" (--release makes selected and coverage errors); limits are STUDIO/gates.txt; exit 1 when a gate is at error
	$(LOCK_RENDER) $(RUN) pack check --project $(STUDIO) $(GATE_ARGS)
project-init: release ## Make STUDIO a project (docs/uplift-studio.md, "Projects"), or import a workspace: PROJECT_ARGS="--from-studio WORKSPACE [--with-cache] --approver NAME --pack-version V"; writes assets.tsv, log.tsv, provenance/ and records the approved images' hashes; needs REMODEL_ACTOR (make agent-setup writes it); no ROM
	$(CAP) $(REMODEL) project init --project $(STUDIO) $(PROJECT_ARGS)
project-status: release ## Counts by status, what waits (checked, stale, rejected) and the log of STUDIO as one answer: PROJECT_ARGS="--status checked --category wall --limit 50 --json"; writes nothing
	$(CAP) $(REMODEL) project status --project $(STUDIO) $(PROJECT_ARGS)
project-approve: release ## Approve checked drafts of STUDIO after the asset gates: PROJECT_ARGS="NAME.. | --category C [--override 'REASON']"; a gate at error exits 1 unless overridden with a reason; needs REMODEL_ACTOR, in project.txt's approvers if it lists any
	$(CAP) $(REMODEL) project approve --project $(STUDIO) $(PROJECT_ARGS)
pack-build: release ## Build STUDIO's shippable pack from its approved images into STUDIO/build (stale images left out): PROJECT_ARGS="--out DIR --replace"; pack.txt carries the project's version, author, licence, ai, depends, conflicts, and its look.txt is copied
	$(CAP) $(REMODEL) pack build --project $(STUDIO) $(PROJECT_ARGS)
pack-resolve: release ## Which of several stacked packs wins each asset and the stack's memory against the native and browser budgets: PACKS="TOP BELOW" (highest first); exit 1 when a depends or conflicts is broken
	$(if $(PACKS),,$(error pack-resolve needs PACKS="TOP BELOW"))
	$(CAP) $(REMODEL) pack resolve $(PACKS) $(PROJECT_ARGS)

LOOK      ?= comic
LOOK_OUT  ?= $(OUT)/look-render
LOOK_OPTS ?=
look-render: release ## Draw the captured frames (CAPTURE=DIR, default OUT/capture) on the GPU without and with a look: LOOK=comic (the built-in Comic defaults) or a look.txt or directory with one; PACK=DIR adds a texture pack, LOOK_SET="outline_weight=3 toon_steps=2" sets parameters, VIEW sets the view (default 1080p 16:9), LOOK_OPTS="--frames 2100,3300 --bench 20" selects frames and times the cost; writes FRAME-plain/-look/-sheet.png to LOOK_OUT (default OUT/look-render); for agents tuning a look (docs/texture-packs.md, "Looks"; needs a GPU)
	$(LOCK_RENDER) $(RUN) $(VIEW) $(if $(PACK),--texture-pack $(PACK)) $(GPU_OPTS) --look $(LOOK) $(foreach s,$(LOOK_SET),--look-set $(s)) look-render $(or $(CAPTURE),$(OUT)/capture) $(LOOK_OUT) $(LOOK_OPTS)

# Named scenes (docs/uplift-studio.md, "Scenes"): SCENE=NAME, SCENE_OPTS="--json --project DIR".
SCENE      ?=
SCENE_OPTS ?=
scene-list: release ## List GAME's scenes (the manifest's and a project's scenes.txt) and whether each has a local copy: SCENE_OPTS="--json --tag play --project DIR" (docs/uplift-studio.md, "Scenes")
	$(RUN) scene list $(SCENE_OPTS)
scene-make: release ## Make GAME's scene SCENE (default: every scene without a local copy): run its recipe from power-on and keep the capture, state, hash and thumbnail in out/GAME/scenes/NAME; SCENE_OPTS="--replace --json" (needs the ROM)
	$(RUN) scene make $(or $(SCENE),--all) $(SCENE_OPTS)
scene-verify: release ## Run scenes' recipes again (SCENE=NAME, default all with a copy) and compare the state hash at their frames with the recorded one: a difference means the emulation changed and marks the scene stale (exit 1); run after a reModel update (needs the ROM)
	$(RUN) scene verify $(SCENE) $(SCENE_OPTS)
scene-render: release ## Draw scene SCENE from its capture with no emulator, ROM or boot, about 2 s: PACK=DIR a texture pack, VIEW the view (default 1080p 16:9), SCENE_OPTS="--against original --crop X,Y,W,H --json --out DIR" (game pixels, 496x384); writes NAME[-pack/-original/-sheet].png to out/GAME/scene-render (needs a GPU)
	$(RUN) $(VIEW) scene render $(SCENE) $(if $(PACK),--pack $(PACK)) $(SCENE_OPTS)
# Games `make scene-check` tries, and where it keeps their scenes.
SCENE_CHECK_GAMES ?= topskatr vf2
SCENE_CHECK_OUT   ?= out/scratch/scene-check
scene-check: release ## The scene acceptance test, per SCENE_CHECK_GAMES (default topskatr vf2; JIT=1 runs the games on the JIT): make every scene from a fresh folder (SCENE_CHECK_OUT), verify them (the hashes must match), draw each from its capture against the original (under the Comic look) with a crop, and time the draw (needs the ROMs and a GPU); the folder is deleted after
	@rm -rf $(SCENE_CHECK_OUT)
	@set -e; for g in $(SCENE_CHECK_GAMES); do \
		echo "== $$g"; \
		export REMODEL_DATA=$(SCENE_CHECK_OUT); \
		$(CAP) $(REMODEL) --game $$g --rom $(ROM) $(if $(JIT),--jit) scene make --all; \
		$(CAP) $(REMODEL) --game $$g --rom $(ROM) $(if $(JIT),--jit) scene verify; \
		for n in $$($(CAP) $(REMODEL) --game $$g --rom $(ROM) $(if $(JIT),--jit) scene list --json | sed -n 's/.*"name": "\([a-z0-9-]*\)".*/\1/p'); do \
			$(CAP) $(REMODEL) --game $$g --rom $(ROM) $(if $(JIT),--jit) --look comic scene render $$n --against original --crop 0,0,248,192 | grep -E '^(scene|changed|time)'; \
		done; \
	done
	@rm -rf $(SCENE_CHECK_OUT)

# Options for gpu-check, depth-check and pack-check: --bloom N (gpu-check, temporal-probe: bloom), --aa N (multisampling;
# software supersamples to match), --depth, --tex-filter off|mip|2|4|8|16
# (gpu-check with PACK), --materials (the pack's material maps: gpu-check draws them from captures with light.bin;
# --materials-relief off|S), --light (gpu-check: shade with light.bin), and for depth-check and pack-check
# --height/--aspect/--hud; --shading flat|vertex|pixel, --crease auto|N and
# --shading-fine on|off (smooth shading, model2::light) for all of them.
GPU_OPTS ?=
gpu-check: release ## Draw the captured frames on the software and GPU rasterisers; time and compare them, and what GPU_OPTS="--aa 4 --depth" and PACK=DIR (a texture pack, both renderers) cost (needs a GPU)
	$(LOCK_RENDER) $(RUN) $(GPU_OPTS) $(if $(PACK),--texture-pack $(PACK)) gpu-check

# Where pack-check writes its pictures.
PACK_CHECK_OUT ?= $(OUT)/pack-check
pack-check: release ## Draw the captured frames with texture pack PACK= on the GPU unfiltered, mipmapped and 16x anisotropic, in software trilinear and as a supersampled reference; write the pictures to PACK_CHECK_OUT (default OUT/pack-check) and compare them; GPU_OPTS="--materials --shading pixel" with LIGHT_IN= captures that have light.bin (make lighting-capture) adds the pack without and with its material maps on the GPU and in software, where they apply and a report, PROBE=X,Y@FRAME what a material does at a pixel (needs a GPU)
	$(if $(PACK),,$(error pack-check needs PACK=DIR, a texture pack))
	$(LOCK_RENDER) $(RUN) $(GPU_OPTS) --texture-pack $(PACK) pack-check $(or $(LIGHT_IN),$(OUT)/capture) $(PACK_CHECK_OUT) $(foreach p,$(PROBE),--probe $(p))

# Where pack-log writes its log.
PACK_LOG ?= $(OUT)/pack.log
pack-log: release ## Flicker report of texture pack PACK=: run INPUT (default coin-start) for STEPS, drawing every frame on the GPU with the pack at the arcade's pace (its background decoding, uploads and eviction), log what each frame's lookups gave to PACK_LOG (default OUT/pack.log) and report the textures, tiles and regions that alternate between replacement and original, ranked by pixels (PACK_FIXES=none|all|surface,hold,preload compares the stabilisations; PACK_LOG_OPTS="--pack-pace off" runs flat out; needs a GPU; docs/texture-packs.md, "Flicker")
	$(if $(PACK),,$(error pack-log needs PACK=DIR, a texture pack))
	$(RUN) --input $(or $(INPUT),coin-start) \
		--texture-pack $(PACK) --pack-log $(PACK_LOG) $(if $(PACK_FIXES),--pack-fixes $(PACK_FIXES)) $(PACK_LOG_OPTS) run $(STEPS) 0
	$(CAP) $(REMODEL) pack-flicker $(PACK_LOG)

# Where tile-check writes its pictures.
TILE_CHECK_OUT ?= $(OUT)/tile-check
tile-check: release ## Draw the captured frames with the tile images of texture pack PACK live on the GPU and as the software snapshots replace them; write FRAME-gpu, -sw and -orig.png to TILE_CHECK_OUT (default OUT/tile-check) and compare them (needs a GPU; VIEW= sets the view, default 1080p 16:9)
	$(if $(PACK),,$(error tile-check needs PACK=DIR, a texture pack))
	$(RUN) $(VIEW) --texture-pack $(PACK) tile-check $(OUT)/capture $(TILE_CHECK_OUT)

pack-identity: release ## Draw the captured frames with a pack standing for the original textures (PACK=DIR, normally the dump itself; empty: each frame's own textures) on the GPU and in software, unfiltered and mipmapped, against no pack; fails a frame on the differences that mean a fault (one texel value, another polygon), not on filter order up to a ceiling (needs a GPU)
	$(LOCK_RENDER) $(RUN) $(GPU_OPTS) $(if $(PACK),--texture-pack $(PACK)) pack-check --identity $(OUT)/capture $(OUT)/pack-identity

# materials-identity: the coin-start run's length and snapshot interval,
# and the interval of its captures for the GPU checks (its 12 runs go JOBS
# at a time).
MAT_STEPS    ?= 900000000
MAT_SNAP     ?= 300
MAT_GPU_SNAP ?= 900
materials-identity: release ## Check that texture pack PACK with all its normal maps made flat draws with --materials exactly as with --light and no materials (the comparison to make: --materials records the side channel), under both rules (relief and --materials-relief off), in flat, vertex, pixel and mipmapped pixel shading, from coin-start run snapshots; and the same on the GPU (gpu-check of captures, unfiltered, mip, 16x and --aa 4 --depth; temporal-probe) (MAT_STEPS, MAT_SNAP, MAT_GPU_SNAP; JOBS runs at once, default 3; needs a GPU; scratch in out/scratch/materials-identity/GAME, deleted when it passes)
	@test -n "$(PACK)" || { echo "materials-identity wants PACK=DIR (a pack with normal maps)"; exit 1; }
	CAP=$(CAP) REMODEL=$(REMODEL) GAME=$(GAME) ROM=$(ROM) PACK=$(PACK) WORK=out/scratch/materials-identity/$(GAME) \
		STEPS=$(MAT_STEPS) SNAP=$(MAT_SNAP) GPU_SNAP=$(MAT_GPU_SNAP) JOBS=$(JOBS) sh tools/materials-identity.sh

# depth-check: PROBE="X,Y@FRAME ..." follows those output pixels through the
# depth buffer (only those frames are drawn); AGAINST=DIR compares the
# depth-on frames with an earlier run's pictures (docs/enhancements.md).
depth-check: release ## Draw the captured frames on the GPU with the depth buffer off and on, report and picture the differences into OUT/depth-check (GPU_OPTS: --aa N, --height, --aspect; PROBE=X,Y@FRAME, AGAINST=DIR; needs a GPU)
	$(LOCK_RENDER) $(RUN) $(GPU_OPTS) depth-check $(OUT)/capture $(OUT)/depth-check $(foreach p,$(PROBE),--probe $(p)) $(if $(AGAINST),--against $(AGAINST))

# lighting-check: LIGHT_OPTS for the command, e.g. "--every 5" (pictures of
# every 5th frame), "--probe X,Y@FRAME" (repeatable), "--bench [N]";
# GPU_OPTS before it ("--crease N", "--height", "--aspect").
LIGHT_OPTS ?=
lighting-check: release ## Draw the captured frames flat and smooth-shaded (vertex, or GPU_OPTS="--shading pixel" for per pixel; "--shading-fine on" adds finer levels and K, against the game's levels; "--shading-balance off" checks without the brightness balance, -nobal pictures and reports, its mean brightness C report-only) on the software and GPU renderers; check the invariants (crease 0 is flat, unlit polygons unchanged, brightness of corners and pixels, software against GPU, hue kept, determinism; with LIGHT_IN=a lighting-capture: the side channel's self-check and list scan, L0 against it), report and pictures into out/scratch/lighting/GAME (LIGHT_OPTS: --every N, --probe X,Y@F, --bench [N], --no-side; GPU_OPTS: --crease N, --height/--aspect; needs a GPU)
	$(LOCK_RENDER) $(RUN) $(GPU_OPTS) lighting-check $(or $(LIGHT_IN),$(OUT)/capture) out/scratch/lighting/$(GAME) $(LIGHT_OPTS)

spec-stats: release ## Count the game's own specular highlights on a lighting-capture (LIGHT_IN, default LIGHT_CAPTURE): textured polygons with a specular term, the pixels it brightens by 1 and 4 levels or more (face and smooth normals); spec-stats.txt and frame | flat term | smooth term pictures into out/scratch/lighting/GAME/spec (LIGHT_OPTS: --every N; GPU_OPTS: --height/--aspect; no GPU needed)
	$(LOCK_RENDER) $(RUN) $(GPU_OPTS) lighting-check $(or $(LIGHT_IN),$(LIGHT_CAPTURE)) out/scratch/lighting/$(GAME)/spec --spec-stats $(LIGHT_OPTS)

# Where lighting-capture saves (a directory of its own: the regression
# baseline hashes out/GAME/capture).
LIGHT_CAPTURE ?= out/scratch/lighting/$(GAME)/capture
lighting-capture: release ## Save every 300th frame of a coin-start run with its lighting side channel (native geometrizer; light.bin, scan.bin) to LIGHT_CAPTURE (default out/scratch/lighting/GAME/capture), for lighting-check, spec-stats and pack-check with material maps (LIGHT_IN=); prints the side channel's self-check over every frame of the run
	rm -rf $(LIGHT_CAPTURE)
	$(RUN) --geo native --light --capture $(LIGHT_CAPTURE) --input $(BENCH_INPUT) run 900000000 300 | grep -E '^light'

temporal-probe: release ## Draw the captured frames with the GPU's temporal anti-aliasing (GPU_OPTS: --temporal, --stipple, --depth, --height, --aspect): its first frame against the frame with it off, jittered, blend and keep pictures into out/scratch/temporal-probe/GAME (needs a GPU)
	$(LOCK_RENDER) $(RUN) $(GPU_OPTS) temporal-probe $(OUT)/capture out/scratch/temporal-probe/$(GAME)

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
	$(RECOMP_REMODEL) --game $(GAME) --rom $(ROM) $(if $(INPUT),--input $(INPUT)) lockstep $(STEPS)

state-check: release ## Save state round trip: a run saved at frame 2000 and resumed must match an uninterrupted one (state hash and audio)
	@$(RUN) --audio $(OUT)/state-a.wav --save-state $(OUT)/state-check.save@2000 --input $(BENCH_INPUT) run $(BENCH_STEPS) | grep '^state hash' > $(OUT)/state-a.txt
	@$(RUN) --audio $(OUT)/state-b.wav --load-state $(OUT)/state-check.save --input $(BENCH_INPUT) run $(BENCH_STEPS) | grep '^state hash' > $(OUT)/state-b.txt
	$(call compare_hashes,$(OUT)/state-a.txt,$(OUT)/state-b.txt,same after save and load)
	@python3 -I -c 'import sys, wave; a, b = (wave.open(f).readframes(1 << 30) for f in sys.argv[1:]); print("audio         " + ("resumed run matches the uninterrupted one" if a.endswith(b) else "DIFFERS")); sys.exit(not a.endswith(b))' $(OUT)/state-a.wav $(OUT)/state-b.wav

# `make state-roundtrip`: per set, the input that gets it into play (daytona
# needs the coin, start and gas of its selection screens).
STATE_RT_GAMES ?= topskatr daytona srallyc lastbrnx
STATE_RT_AT    ?= 700 1900 3300
STATE_RT_STEPS ?= 600000000
STATE_RT_INPUT_daytona ?= coin1@1100-1104,start@1300-1304,gas=0xff@1500-1510,gas=0xff@1700-1710,gas=0xff@2400-99999
state-roundtrip: release ## Save state round trips for several sets at several frames (STATE_RT_GAMES, STATE_RT_AT, STATE_RT_STEPS): each run saved at a frame and resumed must end on the uninterrupted run's state hash
	@set -e; mkdir -p $(OUT)/state-rt; bad=0; \
	for g in $(STATE_RT_GAMES); do \
	  folder=$$($(REMODEL) --game $$g --rom $(ROM) rom-folder 2>/dev/null) || folder=$$g; \
	  if [ ! -e $(ROM)/$$folder ] && [ ! -e $(ROM)/$$folder.zip ]; then echo "state-rt      $$g: no ROMs (not checked)"; continue; fi; \
	  eval "in=\$${STATE_RT_INPUT_$$g:-$(BENCH_INPUT)}"; \
	  run="$(CAP) $(REMODEL) --game $$g --rom $(ROM) --out $(OUT)/state-rt --input $$in"; \
	  $$run run $(STATE_RT_STEPS) >/dev/null; \
	  want=$$($$run run $(STATE_RT_STEPS) | sed -n 's/^state hash *\([0-9a-f]*\).*/\1/p'); \
	  for f in $(STATE_RT_AT); do \
	    $$run --save-state $(OUT)/state-rt/$$g.save@$$f run $(STATE_RT_STEPS) >/dev/null; \
	    got=$$($$run --load-state $(OUT)/state-rt/$$g.save run $(STATE_RT_STEPS) | sed -n 's/^state hash *\([0-9a-f]*\).*/\1/p'); \
	    if [ "$$got" = "$$want" ]; then echo "state-rt      $$g saved at $$f: $$got ok"; else echo "state-rt      $$g saved at $$f: DIFFERS (got $$got, straight run $$want)"; bad=1; fi; \
	  done; \
	  rm -f $(OUT)/state-rt/$$g.save; \
	done; [ $$bad = 0 ]

LINK_STATE_GAME  ?= daytona
LINK_STATE_AT    ?= 1900 3100 3200 3300
LINK_STATE_INPUT ?= coin1@1100-1104,start@1300-1304,gas=0xff@1500-1510,gas=0xff@1700-1710,gas=0xff@2400-99999
link-state-check: release ## Experimental: save state round trips of linked machines (LINK_STATE_GAME daytona, NODES 2, FRAMES 4600, LINK_STATE_AT): a run saved at a frame and resumed must end on the straight run's combined hash (3200 and 3300 are in the 3D race)
	@set -e; mkdir -p $(OUT)/link-state; bad=0; \
	run="$(CAP) $(REMODEL) --game $(LINK_STATE_GAME) --rom $(ROM) --out $(OUT)/link-state --input $(LINK_STATE_INPUT) link-run --nodes $(or $(NODES),2) --frames $(or $(FRAMES),4600)"; \
	want=$$($$run | sed -n 's/^combined hash *//p'); \
	for f in $(LINK_STATE_AT); do \
	  $$run --save-at $$f $(OUT)/link-state/save >/dev/null; \
	  got=$$($$run --load $(OUT)/link-state/save | sed -n 's/^combined hash *//p'); \
	  if [ "$$got" = "$$want" ]; then echo "link-state    saved at $$f: $$got ok"; else echo "link-state    saved at $$f: DIFFERS (got $$got, straight run $$want)"; bad=1; fi; \
	done; rm -f $(OUT)/link-state/save; [ $$bad = 0 ]

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
	vf2+3707+punch+coin1@1100-1104,coin1@1130-1134,start@1250-1254,punch@1400-3400/30,kick@1415-3400/30 \
	schamp+3507+right+$(FIGHT_INPUT) vstriker+3507+right lastbrnx+3507+right+$(FIGHT_INPUT) \
	srallyc+4007+wheel=0xD0+coin-start,gas=0xff@2300-99999 \
	daytona+3300+wheel=0xD0+coin-start,gas=0xff@2300-99999 vcop+2010+trigger1 \
	desert+2505+handle=0xD0 vcop2+2010+trigger1 \
	skytargt+2507+stickx=0x10 skytargt+2507+gun+coin1@1100-1104,coin1@1130-1134,start@1300-1304 \
	manxtt+3300+bank=0x30+coin-start,throttle=0xe0@2300-99999 \
	motoraid+2900+bank=0x30+coin-start,throttle=0xe0@2200-99999 \
	gunblade+2407+trigger1+coin1@1100-1104,start@1250-1254,gun1=140:180@1260-1500,trigger1@1300-1400/20,gun1=248:190@1500-9000 \
	dynamcop+3507+punch+$(FIGHT_INPUT) dynamcop+3507+jump+$(FIGHT_INPUT) \
	zerogun+2107+left \
	pltkids+2107+up \
	rchase2+2400+stickx=0xc0 \
	von+2507+lleft+coin1@1100-1104,start@1250-1254,lshot@2000-99999/6
input-lag: release ## Each game's input lag (INPUT_LAG, or just GAME='s): frames from a control held in play to the first change in the picture, which is how many run-ahead can take away
	@for c in $(if $(filter command line,$(origin GAME)),$(filter $(GAME)+%,$(INPUT_LAG)),$(INPUT_LAG)); do \
		set -- $$(echo $$c | tr + ' '); \
		printf '%-10s %-16s ' $$1 "$$3 at $$2"; \
		$(CAP) $(REMODEL) --game $$1 --rom $(ROM) --geo native --input $${4:-coin-start} latency $$2 $$3 16 \
			| sed -n 's/^input lag *//p'; \
	done

RUN_AHEAD_GAMES  ?= topskatr stcc hotd overrevb vstriker
RUN_AHEAD_FRAMES ?= 2
RUN_AHEAD_STEPS  ?= 300000000
run-ahead-check: release ## Run-ahead against plain runs (RUN_AHEAD_GAMES, or GAME=): coin-start runs (RUN_AHEAD_STEPS, smooth motion on) RUN_AHEAD_FRAMES ahead keep the real state hash, and the frames shown are the plain run's that many later (pictures, hashes, in-betweens); times the copy
	@for g in $(if $(filter command line,$(origin GAME)),$(GAME),$(RUN_AHEAD_GAMES)); do \
		$(CAP) $(REMODEL) --game $$g --rom $(ROM) --geo native --smooth 0.5 --input $(BENCH_INPUT) \
			run-ahead-check $(RUN_AHEAD_STEPS) $(RUN_AHEAD_FRAMES) 25 || exit 1; \
	done

RUN_AHEAD_WARMUP ?= 3000
RUN_AHEAD_PROFILE_FRAMES ?= 1000
run-ahead-profile: release ## Host time per shown frame with run-ahead RUN_AHEAD_FRAMES (0: off) for GAME=: real frame, copy, frames ahead, memory copied, input change rate (docs/play.md, "Run-ahead cost")
	$(RUN) --geo native --input $(BENCH_INPUT) run-ahead-profile $(RUN_AHEAD_WARMUP) $(RUN_AHEAD_PROFILE_FRAMES) $(RUN_AHEAD_FRAMES)

DRAW_CHECK_FRAMES ?= 1500
DRAW_CHECK_OPTS   ?= --run-ahead 1 --aa 8 --shading pixel --bloom 50
DRAW_CHECK_OUT    ?= out/scratch/draw-overlap-check/$(GAME)
draw-overlap-check: ## The player with its draw thread and with --no-overlap (opens a window; needs a GPU): coin-start runs of GAME to frame DRAW_CHECK_FRAMES (DRAW_CHECK_OPTS, default run-ahead 1 and heavy settings; smooth motion and temporal anti-aliasing off whatever the game's saved settings, as both make a picture depend on when and how often frames were drawn) must end on the same state hash and present the same picture of every frame both drew (--frame-log, read back; DRAW_CHECK_OUT)
	$(call release_build,remodel-play)
	@mkdir -p $(DRAW_CHECK_OUT)
	@for m in overlap no-overlap; do \
		$(CAP) $(PLAY) --game $(GAME) --rom $(ROM) --no-splash --no-sound --smooth off --temporal off --bezel off \
			--input $(BENCH_INPUT) --frames $(DRAW_CHECK_FRAMES) $(DRAW_CHECK_OPTS) \
			--frame-log $(DRAW_CHECK_OUT)/$$m.log $$([ $$m = no-overlap ] && echo --no-overlap) \
			| grep '^frames' > $(DRAW_CHECK_OUT)/$$m.txt || exit 1; \
	done
	$(call compare_hashes,$(DRAW_CHECK_OUT)/overlap.txt,$(DRAW_CHECK_OUT)/no-overlap.txt,the same)
	@python3 -I tools/frame-log-diff.py $(DRAW_CHECK_OUT)/overlap.log $(DRAW_CHECK_OUT)/no-overlap.log

PHOTO_CHECK_FRAMES ?= 2400
PHOTO_CHECK_SPEC   ?= 2200:back=100,orbit=40,tilt=-12,z=0.2,dof=1,blur=8,hud=on,vignette=0.3,res=1080,name=photo-check
PHOTO_CHECK_OPTS   ?=
PHOTO_CHECK_OUT    ?= out/scratch/photo-check/$(GAME)
photo-check: ## Photo mode leaves the game alone (opens a window; needs a GPU): a coin-start run of GAME to frame PHOTO_CHECK_FRAMES with a scripted photo session (PHOTO_CHECK_SPEC: open at a frame, scrub back, move the camera, take a photo into PHOTO_CHECK_OUT) must end on the same state hash as one without, and photo mode's machine, run again from its history, must hold the game's frame (when it shows the game's own: not with run-ahead or --photo-history 0) (PHOTO_CHECK_OPTS; RECOMP=1: recompiled code)
	$(call release_build,remodel-play)
	@mkdir -p $(PHOTO_CHECK_OUT)
	@for m in plain photo; do \
		$(CAP) $(PLAY) --game $(GAME) --rom $(ROM) --no-splash --no-sound --bezel off --fast \
			--input $(BENCH_INPUT) --frames $(PHOTO_CHECK_FRAMES) $(PHOTO_CHECK_OPTS) \
			$$([ $$m = photo ] && echo --photo-dir $(PHOTO_CHECK_OUT) --photo "$(PHOTO_CHECK_SPEC)") \
			> $(PHOTO_CHECK_OUT)/$$m.log || exit 1; \
		grep '^frames' $(PHOTO_CHECK_OUT)/$$m.log > $(PHOTO_CHECK_OUT)/$$m.txt; \
	done
	@grep '^photo' $(PHOTO_CHECK_OUT)/photo.log
	@! grep -q 'studio state hash .* DIFFERS' $(PHOTO_CHECK_OUT)/photo.log || { echo "photo mode's frame is not the game's"; exit 1; }
	$(call compare_hashes,$(PHOTO_CHECK_OUT)/plain.txt,$(PHOTO_CHECK_OUT)/photo.txt,the same with and without photo mode)

CAMERA_FRAME ?= 3000
CAMERA_GAP   ?= 20
camera-scan: release ## Photo mode's true camera, a spike (docs/photo-mode.md): run a coin-start run (INPUT) to CAMERA_FRAME and CAMERA_GAP frames on, and list the rotations in the i960's RAM that turned as the scenery says the camera did (candidates for the game's camera)
	$(RUN) --input $(or $(INPUT),$(BENCH_INPUT)) camera-scan $(CAMERA_FRAME) $(CAMERA_GAP)

bench: release ## Timed fixed run into a race from a blank EEPROM: speed vs real time and the state hash (must not change with speed-ups, nor with what earlier runs saved; JIT=1: on the i960 JIT, with its counters)
	$(RUN) --fresh-eeprom --input $(BENCH_INPUT) run $(BENCH_STEPS) | grep -E '^(instructions|emulated|host time|frame times|idle skip|jit|state hash)'

jit-lockstep: release ## The i960 JIT beside the interpreter, block by block, through a coin-start run (JIT_STEPS, default 300M; INPUT), stopping at the first difference; no recompiled build needed (docs/jit.md)
	$(CAP) $(REMODEL) --game $(GAME) --rom $(ROM) --jit --input $(or $(INPUT),$(BENCH_INPUT)) lockstep $(JIT_STEPS)

jit-copro-lockstep: release ## The copro JITs (Model 2A's MB86234, 2C's MB86235 copro and geometrizer) each beside its interpreter through a coin-start run (JIT_STEPS i960 instructions, default 300M; INPUT), compared after every run of the chip, stopping at the first difference (docs/jit.md)
	$(CAP) $(REMODEL) --game $(GAME) --rom $(ROM) --jit --jit-copro check --input $(or $(INPUT),$(BENCH_INPUT)) run $(JIT_STEPS) | grep -E '^(instructions|jit copro|state hash)'

jit-translate: release ## Translate the game's whole i960 program on the JIT ahead of time, cut as `make recomp` cuts it (discovery plus `make run`'s trace), and time it
	$(RUN) jit-translate

# Where `make jit-warm` writes the hot-block list: the player's folder for
# it in a source tree (docs/jit.md, "Hot-block list").
JIT_CACHE      ?= out/$(GAME)/jit
JIT_WARM_STEPS ?= $(BENCH_STEPS)
jit-warm: release ## Warm the JITs' hot-block list without playing: a headless coin-start run (JIT_WARM_STEPS, default BENCH_STEPS) on the JITs, the blocks they translate written to JIT_CACHE/hot.txt (default out/GAME/jit, where the player looks), translated at the next boot; addresses only (docs/jit.md)
	$(CAP) $(REMODEL) --game $(GAME) --rom $(ROM) --jit --jit-cache $(JIT_CACHE) --input $(or $(INPUT),$(BENCH_INPUT)) run $(JIT_WARM_STEPS) | grep -E '^(frame times|jit list|state hash)'

# A scratch hot-block list for `make jit-hitch`.
JIT_HITCH_DIR  := out/scratch/jit-hitch/$(GAME)
JIT_HITCH_OPTS ?=
jit-hitch: release ## Frame-time hitches: the bench session (BENCH_STEPS; JIT_HITCH_OPTS, e.g. --geo native) on the interpreter, on the JIT translating where it runs (--jit-sync, as before the workers), on the JIT cold (no hot-block list) and warm (the list the cold run left); each run's host time per frame (median to worst, frames over 8, 16.7 and 33 ms) and state hash (scratch list in out/scratch/jit-hitch/GAME, deleted after)
	@rm -rf $(JIT_HITCH_DIR)
	@for run in "interpreter|" "jit inline|--jit --jit-sync" "jit cold|--jit --jit-cache $(JIT_HITCH_DIR)" "jit warm|--jit --jit-cache $(JIT_HITCH_DIR)"; do \
		echo "== $(GAME) $(JIT_HITCH_OPTS): $${run%%|*}"; \
		$(CAP) $(REMODEL) --game $(GAME) --rom $(ROM) $${run#*|} $(JIT_HITCH_OPTS) --input $(BENCH_INPUT) run $(BENCH_STEPS) | grep -E '^(host time|frame times|state hash)' || exit 1; \
	done
	@rm -rf $(JIT_HITCH_DIR)

bench-all: release ## `make bench` for every set in tools/expected-hashes.txt that has ROMs; report any state hash that differs
	@ROM=$(ROM) MAKE="$(MAKE)" REMODEL=$(REMODEL) sh tools/release.sh bench-all

bench-variants: release ## `make bench` for every variant in tools/expected-variant-hashes.txt whose ROMs are there (the other releases of a title: `--game topskatrj`...; docs/variants.md); report any state hash that differs. Kept apart from bench-all, which release-tag gates on
	@ROM=$(ROM) MAKE="$(MAKE)" REMODEL=$(REMODEL) sh tools/release.sh bench-variants

release-tag: ## Cut a release: VERSION=x.y.z; check, release build, bench-all, clean tree, CHANGELOG and Cargo.toml versions, then an annotated tag vX.Y.Z (never pushes; DRY_RUN=1 only checks)
	@test -n "$(VERSION)" || { echo "usage: make release-tag VERSION=x.y.z"; exit 1; }
	@ROM=$(ROM) MAKE="$(MAKE)" REMODEL=$(REMODEL) DRY_RUN=$(DRY_RUN) SKIP_BUILD=$(SKIP_BUILD) sh tools/release.sh tag $(VERSION)

copro-disasm: release ## Disassemble the geometry program captured by `make run`
	$(RUN) copro-disasm

tgp-disasm: release ## List a Model 2A TGP (MB86234) program with mnemonics: header, dispatch table, functions, from the upload `make run` captured (to out/GAME/tgp.lst)
	$(RUN) tgp-disasm

tgp-check: release ## Run a Model 2A TGP program's functions on the MB86234 interpreter with random arguments, against their maths and the old function model (needs `make run` first)
	$(RUN) tgp-check

sharc-disasm: release ## Disassemble an ADSP-21062 program image: make sharc-disasm FILE=out/overrevb/sharc-geo.bin (listing to FILE.lst)
	$(CAP) $(REMODEL) sharc-disasm $(FILE)

sound: release ## Boot the sound board, replay the sound bytes captured by `make run`, write OUT/sound.wav
	$(RUN) sound-render $(OUT)/sound.wav

sound-disasm: release ## Disassemble the sound 68000 program: make sound-disasm ADDR=0x600120 COUNT=64
	$(RUN) sound-disasm $(ADDR) $(COUNT)
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
	$(CAP) $(REMODEL) soundtrack-import "$(SRC)" "$(SOUNDTRACK_DIR)"

mpeg-dump: release ## Decode MPEG ROM frames directly: make mpeg-dump ADDR=0x11000 SECONDS=10
	$(RUN) mpeg-dump $(ADDR) $(SECONDS)

mpeg-test: ## MPEG decoder test against the MPEG ROMs in ROM=
	REMODEL_ROM=$(abspath $(ROM)) $(CARGO) test --release -p remodel-mpeg -- --ignored

##@ Website (the promo site; site/README.md)

# The promo site: site/ holds the page and original art only; the game
# screenshots are made from your ROMs by `make site-shots` into
# SITE_SHOTS and joined with the page by `make site` into SITE_OUT, so
# nothing ROM-derived is ever committed.
SITE_SHOTS ?= out/site-shots
SITE_OUT   ?= out/site
SITE_PORT  ?= 8800

site-shots: ## Website screenshots from ROM=: every shot in tools/site-shots.sh's list, captured with the player's default settings (never your saved ones) into SITE_SHOTS (default out/site-shots); SHOT=NAME redoes one
	$(call release_build,remodel-play)
	CAP=$(CAP) PLAY=$(CURDIR)/$(PLAY) ROM=$(abspath $(ROM)) SITE_SHOTS=$(abspath $(SITE_SHOTS)) SHOT=$(SHOT) sh tools/site-shots.sh

site: ## Assemble the website into SITE_OUT (default out/site): site/, the posters and the screenshots from `make site-shots`, as WebP
	SITE_SHOTS=$(SITE_SHOTS) SITE_OUT=$(SITE_OUT) sh tools/site-build.sh

site-serve: site ## Assemble the website and serve it on http://127.0.0.1:SITE_PORT (default 8800) until Ctrl-C
	@echo "http://127.0.0.1:$(SITE_PORT)/"
	cd $(SITE_OUT) && $(PYTHON) -m http.server $(SITE_PORT) --bind 127.0.0.1

site-deploy: site ## Publish the website: assemble it, then force-push SITE_OUT as the single commit of origin's gh-pages branch, which GitHub Pages serves (DRY_RUN=1: commit, don't push)
	SITE_OUT=$(SITE_OUT) DRY_RUN=$(DRY_RUN) sh tools/site-deploy.sh

##@ Housekeeping

CLEAN_DAYS ?= 2
clean-stale: ## Delete build output newer builds replaced: in target/, crate builds and incremental caches unused for CLEAN_DAYS days (default 2), always keeping each crate's newest build, so current builds aren't redone (DRY_RUN=1: report the total only)
	python3 -I tools/clean-stale.py target --days $(CLEAN_DAYS) $(if $(DRY_RUN),--dry-run)

clean: ## Remove build output and generated listings
	$(CARGO) clean
	rm -rf out
