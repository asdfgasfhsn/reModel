#!/bin/bash
# Networked link play over loopback, two processes per case (`make link-net-test`).
#
#   usage: tools/link-net-test.sh REMODEL ROM_DIR OUT_DIR [GAME...]
#
# For each game (srallyc, daytona by default), with the game's scripted race:
#   clean       a host and a joiner at delay D: each process's state hash must
#               equal `remodel link-run`'s for its node with the same D
#   faults      the same with 5% loss, 5% duplication, 50 ms jitter and 5 ms
#               added latency on both sides: the same hashes, no desync, only stalls
#   leave       the joiner says goodbye at frame 1500: the host ends cleanly
#   disconnect  the joiner is killed (SIGKILL) mid-run: the host ends on its
#               silence timeout with a message
#   discover    a named host is found by `link-net --browse` over mDNS (the TXT record: name, game,
#               players, version, ROM hash, delay), then joined by a named player; the
#               names are in both logs (skipped with a message where multicast is unavailable)
#   settings    the joiner's settings differ by a byte: the host refuses to start
#   desync      a record received for frame 1200 is damaged: both hosts report
#               the frame and the hashes
# Everything is on 127.0.0.1; scratch output stays under OUT_DIR.
set -u
BIN=$1; ROM=$2; OUT=$3; shift 3
GAMES=${*:-srallyc daytona}
mkdir -p "$OUT"
fail=0
base=$((47100 + RANDOM % 1500))
n=0

hashes() { sed -n 's/^node \([0-9]*\) *state hash \([0-9a-f]*\) .*/\1 \2/p' "$1"; }
ok() { echo "link-net-test $1: ok${2:+ ($2)}"; }
bad() { echo "link-net-test $1: FAIL ${2:-}"; fail=1; }
stat() { grep -h "link-net      stalls\|link-net      sent" "$1" | sed 's/^link-net      //'; }

# Run host and joiner; $1 = case tag, $2 = common args, $3 = host extras, $4 = joiner extras
pair() {
	local tag=$1 common=$2 hx=$3 jx=$4
	n=$((n + 1)); port=$((base + n))
	"${BIN_RUN[@]}" --out "$OUT/$tag-h" $INPUT_ARGS link-net --host $port --nodes 2 --name Alex $common $hx >"$OUT/$tag-h.log" 2>"$OUT/$tag-h.err" &
	HPID=$!
	sleep 1.5
	[ -z "${MID:-}" ] || eval "$MID"
	"${BIN_RUN[@]}" --out "$OUT/$tag-j" $INPUT_ARGS link-net --join 127.0.0.1:$port --name Sam $common $jx >"$OUT/$tag-j.log" 2>"$OUT/$tag-j.err" &
	JPID=$!
}

for game in $GAMES; do
	case $game in
	srallyc)
		INPUT="coin1@1500-1504,coin1@1560-1564,gas=0xff@3430-99999"; FRAMES=4400 ;;
	daytona)
		INPUT="coin1@1100-1104,start@1300-1304,gas=0xff@1500-1510,gas=0xff@1700-1710,gas=0xff@2400-99999"; FRAMES=4600 ;;
	*) echo "no scripted race for $game"; exit 2 ;;
	esac
	BIN_RUN=("$(dirname "$0")/capped" "$BIN" --game "$game" --rom "$ROM")
	INPUT_ARGS="--input $INPUT"
	mkdir -p "$OUT/$game"

	for D in 2 6; do
		# The reference: both nodes in one process.
		"${BIN_RUN[@]}" --out "$OUT/$game/lr$D" $INPUT_ARGS link-run --nodes 2 --frames $FRAMES --delay $D >"$OUT/$game/lr$D.log" 2>&1
		ref0=$(hashes "$OUT/$game/lr$D.log" | sed -n 1p | cut -d' ' -f2)
		ref1=$(hashes "$OUT/$game/lr$D.log" | sed -n 2p | cut -d' ' -f2)
		[ -n "$ref0" ] || { bad "$game link-run D=$D" "no reference"; continue; }

		if [ $D = 2 ]; then tag=clean; extra=""; else tag=faults; extra="--latency 5 --jitter 50 --loss 5 --dup 5"; fi
		pair "$game/$tag" "--frames $FRAMES --delay $D --seed $D" "$extra" "$extra --seed 9"
		wait $HPID; hrc=$?; wait $JPID; jrc=$?
		h0=$(hashes "$OUT/$game/$tag-h.log" | cut -d' ' -f2); h1=$(hashes "$OUT/$game/$tag-j.log" | cut -d' ' -f2)
		if [ $hrc = 0 ] && [ $jrc = 0 ] && [ "$h0" = "$ref0" ] && [ "$h1" = "$ref1" ]; then
			ok "$game $tag D=$D" "node 0 $h0, node 1 $h1 = link-run"
			stat "$OUT/$game/$tag-h.log" | sed "s/^/    host:   /"; stat "$OUT/$game/$tag-j.log" | sed "s/^/    joiner: /"
		else
			bad "$game $tag D=$D" "exit $hrc/$jrc, hashes host $h0 joiner $h1, link-run $ref0 $ref1"
		fi
	done

	# A host found over mDNS, then joined; names in the logs.
	if [ "$game" = "${GAMES%% *}" ]; then
		MID='"${BIN_RUN[@]}" link-net --browse 4 >"$OUT/$game/browse.log" 2>&1'
		pair "$game/discover" "--frames 600 --delay 2" "" ""
		MID=
		wait $HPID; hrc=$?; wait $JPID; jrc=$?
		if grep -q "mDNS unavailable" "$OUT/$game/browse.log"; then
			echo "link-net-test $game discover: skipped (multicast is not available here: $(grep 'mDNS unavailable' "$OUT/$game/browse.log" | head -1))"
		elif grep -q "host \"Alex\" game $game players 1/[0-9]* free [0-9]* addr [0-9.]*:$port version [0-9.]* rom [0-9a-f]\{16\} delay [0-9]* via mdns[a-z+]*: can join" "$OUT/$game/browse.log" \
			&& [ $hrc = 0 ] && [ $jrc = 0 ] \
			&& grep -q "lobby: Alex, Sam" "$OUT/$game/discover-h.log" && grep -q "you are Sam, player 2" "$OUT/$game/discover-j.log" \
			&& grep -q "you are Alex, player 1" "$OUT/$game/discover-h.log"; then
			ok "$game discover" "$(grep 'host "Alex"' "$OUT/$game/browse.log" | sed 's/^link-net *//' | cut -c1-150)"
		else
			bad "$game discover" "exit $hrc/$jrc: $(cat "$OUT/$game/browse.log") / $(grep -h 'lobby\|you are' "$OUT/$game/discover-h.log" "$OUT/$game/discover-j.log")"
		fi
	fi

	# The joiner leaves politely.
	pair "$game/leave" "--frames 20000 --delay 2 --timeout 2" "" "--leave-at 1500"
	wait $HPID; hrc=$?; wait $JPID; jrc=$?
	if [ $hrc = 2 ] && grep -q "Sam left" "$OUT/$game/leave-h.err"; then ok "$game leave" "$(head -1 "$OUT/$game/leave-h.err")"; else bad "$game leave" "host exit $hrc: $(cat "$OUT/$game/leave-h.err")"; fi

	# The joiner is killed.
	pair "$game/disconnect" "--frames 20000 --delay 2 --timeout 2" "" ""
	sleep 4
	kill -9 $JPID 2>/dev/null
	t0=$SECONDS; wait $HPID; hrc=$?
	if [ $hrc = 2 ] && grep -q "silent for" "$OUT/$game/disconnect-h.err"; then ok "$game disconnect" "$(head -1 "$OUT/$game/disconnect-h.err"), ended $((SECONDS - t0)) s after the kill"; else bad "$game disconnect" "host exit $hrc: $(cat "$OUT/$game/disconnect-h.err")"; fi

	# A different settings byte on the joiner.
	pair "$game/settings" "--frames 600 --delay 2" "" "--tamper-byte 0x0c:5"
	wait $HPID; hrc=$?; wait $JPID; jrc=$?
	if [ $hrc != 0 ] && grep -q "settings differ" "$OUT/$game/settings-h.err" && [ $jrc != 0 ]; then ok "$game settings" "$(head -1 "$OUT/$game/settings-h.err" | cut -c1-110)"; else bad "$game settings" "exit $hrc/$jrc: $(cat "$OUT/$game/settings-h.err")"; fi

	# A damaged record.
	pair "$game/desync" "--frames 3000 --delay 2 --check-every 60" "" "--corrupt-rx-at 1200"
	wait $HPID; hrc=$?; wait $JPID; jrc=$?
	if [ $jrc = 2 ] && [ $hrc = 2 ] && grep -q "out of sync with [A-Za-z]* at frame 1200: our state hash [0-9a-f]*, theirs [0-9a-f]*" "$OUT/$game/desync-j.err" \
		&& grep -q "out of sync with [A-Za-z]* at frame 1200" "$OUT/$game/desync-h.err"; then
		ok "$game desync" "$(head -1 "$OUT/$game/desync-j.err")"
	else
		bad "$game desync" "exit $hrc/$jrc: $(cat "$OUT/$game/desync-j.err") / $(cat "$OUT/$game/desync-h.err")"
	fi
done
exit $fail
