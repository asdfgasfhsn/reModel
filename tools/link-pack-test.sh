#!/bin/bash
# Packs in linked play (`make link-pack-test`): the state hashes of a split-screen linked
# session do not depend on the texture pack, whichever views draw it.
#
#   usage: tools/link-pack-test.sh REMODEL PLAY ROM_DIR OUT_DIR [GAME...]
#
# For each game (daytona by default; srallyc, manxtt, indy500 also work) with a two-node
# scripted race:
#   reference   `remodel link-run` (no pack: it has none) gives the combined hash
#   pack        a synthetic pack made here: a dump of the game's textures with every
#               colour inverted (tools/pack-tint.py), so it replaces what is drawn with
#               something plainly different; nothing of it is kept in the repository
#   player      `remodel-play --players 2` runs the same script to the same frame with no
#               pack, and with the pack on every view, on view 1 only, on view 2 only, and
#               with the pack on but no view drawing it (`--link-pack`); each run's combined
#               hash must equal link-run's
#   pictures    each run's split-screen picture (--shot): the packed view differs from the
#               original and the other view does not (view 1 only: only the left half
#               differs from the no-pack picture, view 2 only: only the right half), so
#               the pack was really drawn in the view it was asked for
# The player opens a window briefly (it needs a GPU); its data folder and working folder
# are under OUT_DIR, never the player's own. Scratch output stays under OUT_DIR.
set -u
BIN=$1; PLAY=$2; ROM=$3; OUT=$4; shift 4
GAMES=${*:-daytona}
HERE=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)
ROM=$(cd "$ROM" && pwd)
fail=0
ok() { echo "link-pack $1: ok${2:+ ($2)}"; }
bad() { echo "link-pack $1: FAIL ${2:-}"; fail=1; }
comb() { sed -n 's/^combined hash *//p' "$1"; }
pcomb() { sed -n 's/^link play  *[0-9]* machines, combined hash \([0-9a-f]*\).*/\1/p' "$1"; }

for game in $GAMES; do
	case $game in
	daytona) INPUT="start@1300-1304,gas=0xff@1500-1510,gas=0xff@1700-1710,gas=0xff@2400-99999"; FRAMES=3400 ;;
	srallyc) INPUT="start@1560-1564,gas=0xff@3430-99999"; FRAMES=3800 ;;
	manxtt) INPUT="start@1300-1304,throttle=0xe0@3500-99999"; FRAMES=3800 ;;
	indy500) INPUT="start@1250-1254,gas=0xff@1400-1410,gas=0xff@1600-1610,gas=0xff@1800-1810,gas=0xff@2000-99999"; FRAMES=3400 ;;
	*) echo "no scripted race for $game"; exit 2 ;;
	esac
	d="$OUT/$game"
	rm -rf "$d"; mkdir -p "$d/work"
	cap="$HERE/capped"

	# The reference: the headless link run (free play on, as the player has it in link play).
	"$cap" "$BIN" --game "$game" --rom "$ROM" --out "$d/ref" --input "$INPUT" link-run --nodes 2 --free-play \
		--frames $FRAMES --delay 1 >"$d/ref.log" 2>&1
	ref=$(comb "$d/ref.log")
	if [ -n "$ref" ]; then ok "$game reference" "link-run combined hash $ref"; else bad "$game reference" "no hash: see $d/ref.log"; continue; fi

	# The synthetic pack: the game's own textures, colours inverted.
	"$cap" "$BIN" --game "$game" --rom "$ROM" --out "$d/dump" --dump-textures "$d/texdump" --input coin-start \
		run 900000000 60 >"$d/dump.log" 2>&1
	python3 -I "$HERE/pack-tint.py" tint "$d/texdump" "$d/pack" >"$d/tint.log" 2>&1 || { bad "$game pack" "see $d/tint.log"; continue; }
	ok "$game pack" "$(cat "$d/tint.log")"

	# The player, five ways: a data folder and a working folder of its own (no out/ here).
	for mix in off all 1 2 none; do
		args=(--game "$game" --rom "$ROM" --players 2 --input "$INPUT" --frames $FRAMES --shot-size 1280x720 --shot "$d/$mix.png")
		[ "$mix" != off ] && args+=(--texture-pack "$d/pack" --link-pack "$mix")
		(cd "$d/work" && XDG_DATA_HOME="$d/work/data" "$cap" "$PLAY" "${args[@]}") >"$d/$mix.log" 2>&1
		h=$(pcomb "$d/$mix.log")
		if [ -n "$h" ] && [ "$h" = "$ref" ]; then ok "$game player pack on views: $mix" "combined hash $h"
		else bad "$game player pack on views: $mix" "hash '${h:-none}', link-run $ref: see $d/$mix.log"; fi
	done

	# The pictures: the pack shows where it was asked for, and only there.
	expect() { # MIX LEFT RIGHT
		got=$(python3 -I "$HERE/pack-tint.py" halves "$d/off.png" "$d/$1.png" 2>&1)
		if [ "$got" = "left $2 right $3" ]; then ok "$game picture views: $1" "$got"; else bad "$game picture views: $1" "got '$got', wanted left $2 right $3"; fi
	}
	expect all differs differs
	expect 1 differs same
	expect 2 same differs
	expect none same same
done
exit $fail
