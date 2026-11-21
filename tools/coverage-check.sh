#!/bin/sh
# The acceptance test for tours and coverage (`make coverage-check`;
# docs/uplift-studio.md, "Tours and coverage").
#
#   tools/coverage-check.sh REMODEL ROM OUTDIR STEPS GAME...
#
# For each game it needs the ROMs for (a game without them is skipped):
#   1. The `bench` tour (STEPS steps of `coin-start`, run with the read
#      maps on) ends in the same state hash as a plain `remodel run` of
#      the same steps and input, in a fresh folder, which has no maps: the
#      maps change nothing. (Not tools/expected-hashes.txt: those are the
#      bench's second run, which starts with the `eeprom.bin` the first
#      left; a tour and a fresh run start with the board's EEPROM.)
#   2. Running it again writes a byte-identical map (a tour is from
#      power-on with a known EEPROM, so it reproduces).
#   3. `tour run --all --json` and `coverage --json` answer with the six
#      envelope fields, exit 0, and a map with polygon and texture bytes.
#   4. `coverage` with nothing run says so with exit code 3.
# Uses `remodel` through tools/capped, as the Makefile does.
set -eu
cd "$(dirname "$0")/.."
remodel=$1 rom=$2 out=$3 steps=$4
shift 4
fail=0 ran=0
check() { # check DESCRIPTION CONDITION-COMMAND...
    desc=$1
    shift
    if "$@"; then echo "coverage-check: $desc: ok"; else echo "coverage-check: $desc: FAILED" >&2; fail=1; fi
}
for game in "$@"; do
    folder=$("$remodel" --game "$game" --rom "$rom" rom-folder 2>/dev/null) || folder=$game
    if [ ! -e "$rom/$folder" ] && [ ! -e "$rom/$folder.zip" ]; then
        echo "coverage-check: no ROMs for $game (skipped)"
        continue
    fi
    ran=$((ran + 1))
    dir=$out/$game
    rm -rf "$dir"
    mkdir -p "$dir"
    run="tools/capped $remodel --game $game --rom $rom --out $dir"
    want=$(tools/capped "$remodel" --game "$game" --rom "$rom" --out "$dir/plain" --input coin-start run "$steps" | sed -n 's/^state hash *\([0-9a-f]*\).*/\1/p' | head -n 1)
    $run tour run bench --json >"$dir/bench.json"
    got=$(sed -n 's/.*"state_hash": "\([0-9a-f]*\)".*/\1/p' "$dir/bench.json" | head -n 1)
    check "$game: the bench tour's state hash with maps on is a plain run's ($want)" test "$got" = "$want"
    cp "$dir/coverage/bench.map" "$dir/bench-1.map"
    $run tour run bench >/dev/null
    check "$game: running a tour again writes the same map" cmp -s "$dir/coverage/bench.map" "$dir/bench-1.map"
    $run tour run --all --json >"$dir/tours.json"
    check "$game: tour run --all answers ok" grep -q '"code": 0' "$dir/tours.json"
    $run coverage --json >"$dir/coverage.json"
    for field in ok code error data warnings next; do
        check "$game: the answer has $field" grep -q "^  \"$field\":" "$dir/coverage.json"
    done
    check "$game: the answer is ok, exit 0" grep -q '"code": 0' "$dir/coverage.json"
    check "$game: polygon ROM bytes were read" grep -Eq '"read_bytes": [1-9]' "$dir/coverage.json"
    $run coverage | sed -n '1,4p'
    code=0
    $run coverage --dir "$dir/none" --json >"$dir/none.json" 2>/dev/null || code=$?
    check "$game: coverage with no maps is exit 3" test "$code" = 3
done
[ "$ran" -gt 0 ] || { echo "coverage-check: no game had ROMs: nothing checked" >&2; exit 1; }
[ "$fail" = 0 ] || exit 1
echo "coverage-check: $ran games ok"
