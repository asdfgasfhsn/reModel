#!/bin/bash
# Free play in linked play (`make link-free-play-test`): a two-node linked race of each game
# started with start presses only, no coins, on the game's own FREE PLAY coin setting.
#
#   usage: tools/link-free-play-test.sh REMODEL ROM_DIR OUT_DIR [GAME...]
#
# For each game (srallyc, daytona, manxtt, indy500 by default):
#   deterministic   `link-run --free-play` twice: the same combined hash
#   needed          the same inputs without --free-play (no coins: the game does not start)
#                   end on another hash
#   network         a `link-net` host (`--free-play`) and a joiner over loopback end on
#                   `link-run`'s hash for each node, so the setting travels with the
#                   master's settings
#   pictures        frames in the race, from the run, go to OUT_DIR/GAME/captures (read by eye)
# Everything is on 127.0.0.1; scratch output stays under OUT_DIR.
set -u
BIN=$1; ROM=$2; OUT=$3; shift 3
GAMES=${*:-srallyc daytona manxtt indy500}
mkdir -p "$OUT"
fail=0
port=$((47100 + RANDOM % 1500))
hashes() { sed -n 's/^node \([0-9]*\) *state hash \([0-9a-f]*\) .*/\1 \2/p' "$1"; }
comb() { sed -n 's/^combined hash *//p' "$1"; }
ok() { echo "link-free-play $1: ok${2:+ ($2)}"; }
bad() { echo "link-free-play $1: FAIL ${2:-}"; fail=1; }

for game in $GAMES; do
	JITARG=""
	case $game in
	srallyc) INPUT="start@1560-1564,gas=0xff@3430-99999"; FRAMES=4400; SHOTS=2500,3300,4300 ;;
	daytona) INPUT="start@1300-1304,gas=0xff@1500-1510,gas=0xff@1700-1710,gas=0xff@2400-99999"; FRAMES=4600; SHOTS=2000,3300,4500 ;;
	manxtt) INPUT="start@1300-1304,throttle=0xe0@3500-99999"; FRAMES=4600; SHOTS=2500,3400,4500 ;;
	indy500) INPUT="start@1250-1254,gas=0xff@1400-1410,gas=0xff@1600-1610,gas=0xff@1800-1810,gas=0xff@2000-99999"; FRAMES=4500; SHOTS=2200,3300,4400 ;;
	von) INPUT="start@1100-1104,start@1500-1504,start@1900-1904,start@2300-2304,lshot@3600-99999/6,rshot@3603-99999/9,lup@3600-4100"; FRAMES=5200; SHOTS=2000,3000,4400,5000; JITARG=--jit ;;
	stcc) INPUT="start@1250-1254,start@1700-1704,start@2100-2104,start@2500-2504,start@2900-2904,start@3300-3304,gas=0xff@4100-99999"; FRAMES=7200; SHOTS=5000,6600,7200; JITARG=--jit ;;
	skisuprg) INPUT="blue@1250-1254,blue@1500-1504,blue@1800-1804,blue@2100-2104,blue@2400-2404,blue@2700-2704"; FRAMES=4000; SHOTS=3000,3600,4000; JITARG=--jit ;;
	daytonas|daytonase) INPUT="start@1300-1304,gas=0xff@1500-1510,gas=0xff@1700-1710,gas=0xff@2400-99999"; FRAMES=4600; SHOTS=2000,3300,4500 ;;
	indy500d) INPUT="start@1250-1254,start@1400-1404,start@1600-1604,start@1800-1804,gas=0xff@2000-99999"; FRAMES=4500; SHOTS=2200,3300,4400 ;;
	indy500to) INPUT="start@1900-1904,start@2100-2104,start@2300-2304,gas=0xff@2500-99999"; FRAMES=4500; SHOTS=2200,3300,4400 ;;
	*) echo "no scripted race for $game"; exit 2 ;;
	esac
	run=("$(dirname "$0")/capped" "$BIN" --game "$game" --rom "$ROM" $JITARG)
	mkdir -p "$OUT/$game"
	"${run[@]}" --out "$OUT/$game/a" --input "$INPUT" link-run --nodes 2 --free-play --frames $FRAMES --delay 2 --shot-at $SHOTS >"$OUT/$game/a.log" 2>&1
	"${run[@]}" --out "$OUT/$game/b" --input "$INPUT" link-run --nodes 2 --free-play --frames $FRAMES --delay 2 >"$OUT/$game/b.log" 2>&1
	"${run[@]}" --out "$OUT/$game/c" --input "$INPUT" link-run --nodes 2 --frames $FRAMES --delay 2 >"$OUT/$game/c.log" 2>&1
	a=$(comb "$OUT/$game/a.log"); b=$(comb "$OUT/$game/b.log"); c=$(comb "$OUT/$game/c.log")
	if [ -n "$a" ] && [ "$a" = "$b" ]; then ok "$game deterministic" "combined hash $a"; else bad "$game deterministic" "$a vs $b"; fi
	if [ -n "$c" ] && [ "$a" != "$c" ]; then ok "$game needed" "without --free-play $c"; else bad "$game needed" "same hash $c without it"; fi
	mkdir -p "$OUT/$game/captures"
	cp "$OUT/$game/a/frames/"0*.png "$OUT/$game/captures/" 2>/dev/null

	port=$((port + 1))
	"${run[@]}" --out "$OUT/$game/nh" --input "$INPUT" link-net --host $port --nodes 2 --name Alex --free-play --frames $FRAMES --delay 2 >"$OUT/$game/nh.log" 2>&1 &
	hp=$!
	sleep 1.5
	"${run[@]}" --out "$OUT/$game/nj" --input "$INPUT" link-net --join 127.0.0.1:$port --name Sam --frames $FRAMES --delay 2 >"$OUT/$game/nj.log" 2>&1 &
	jp=$!
	wait $hp; hrc=$?; wait $jp; jrc=$?
	r0=$(hashes "$OUT/$game/a.log" | sed -n 1p | cut -d' ' -f2); r1=$(hashes "$OUT/$game/a.log" | sed -n 2p | cut -d' ' -f2)
	h0=$(hashes "$OUT/$game/nh.log" | cut -d' ' -f2); h1=$(hashes "$OUT/$game/nj.log" | cut -d' ' -f2)
	if [ $hrc = 0 ] && [ $jrc = 0 ] && [ "$h0" = "$r0" ] && [ "$h1" = "$r1" ] && [ -n "$r0" ]; then
		ok "$game network" "host $h0, joiner $h1 = link-run (the joiner was not given --free-play)"
	else
		bad "$game network" "exit $hrc/$jrc, host $h0 joiner $h1, link-run $r0 $r1"
	fi
done
exit $fail
