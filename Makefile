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

CARGO   ?= cargo
REMODEL := target/release/remodel
RUN     := $(REMODEL) --game $(GAME) --rom $(ROM)

.DEFAULT_GOAL := help
.PHONY: help build release test lint fmt fmt-check check ci games verify boot disasm analyze run copro-disasm clean

##@ General

help: ## Show this help (variables: GAME, ROM, OUT, ADDR, COUNT, STEPS, SNAP, INPUT)
	@awk 'BEGIN {FS = ":.*## "; printf "usage: make <target> [VAR=value]\n"} \
		/^##@/ {printf "\n%s\n", substr($$0, 5)} \
		/^[a-zA-Z0-9_-]+:.*## / {printf "  %-12s %s\n", $$1, $$2}' $(MAKEFILE_LIST)
	@printf "\ncurrent: GAME=%s ROM=%s OUT=%s\n" "$(GAME)" "$(ROM)" "$(OUT)"

##@ Build and test

build: ## Debug build of the whole workspace
	$(CARGO) build --workspace

release: ## Optimised build of the remodel CLI
	$(CARGO) build --release -p remodel

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
	$(RUN) $(if $(INPUT),--input $(INPUT)) run $(STEPS) $(SNAP)

copro-disasm: release ## Disassemble the geometry program captured by `make run`
	$(RUN) copro-disasm

##@ Housekeeping

clean: ## Remove build output and generated listings
	$(CARGO) clean
	rm -rf out
