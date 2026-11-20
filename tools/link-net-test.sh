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
#   drop2-*     two hosts, mid-race: the joiner says goodbye (polite), the joiner is
#               killed (SIGKILL: the host notices after the 2 s timeout), the host
#               leaves (the joiner is the master's board no more): the one left keeps
#               running to the last frame ("racing on alone"), its state hash equal to
#               `link-run --leave NODE:F` for the drop frame F it logged
#   drop3-*     three hosts, one leaves (politely, or killed with its newest records held
#               back from one survivor so the other must relay them): both survivors run
#               to the end, agree F, and end on `link-run --nodes 3 --leave NODE:F`'s hashes
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

# Players leaving mid-race. $1 case tag, $2 hosts, $3 the host that leaves (0 is the master's),
# $4 quit (says goodbye at frame LEAVE_AT) or kill (SIGKILL once its log shows frame LEAVE_AT).
# The survivors run to FRAMES_DROP and log the drop frame F they agreed; `link-run --leave NODE:F`
# with the same inputs must give each survivor's hash. A kill with 3 hosts also holds the
# leaver's records from LEAVE_AT - 3 back from host 1 (--starve), so host 1 can only go on with
# what host 0 relays.
drop_case() {
	local tag=$1 nodes=$2 lv=$3 how=$4 dl=${5:-2}
	local names=(Ann Bob Cy) pids=() i port logs=() ex
	n=$((n + 1)); port=$((base + n))
	local common="--frames $FRAMES_DROP --delay $dl --timeout 2 --hash-every 100 --shot-at $SHOTS_DROP"
	for ((i = 0; i < nodes; i++)); do
		ex=""
		[ $i = $lv ] && [ $how = quit ] && ex="--leave-at $LEAVE_AT"
		[ $i = $lv ] && [ $how = kill ] && [ $nodes = 3 ] && ex="--starve 1:$((LEAVE_AT - 3))"
		if [ $i = 0 ]; then
			timeout 900 "${BIN_RUN[@]}" --out "$OUT/$tag-$i" $INPUT_ARGS link-net --host $port --nodes $nodes --name ${names[0]} $common $ex >"$OUT/$tag-$i.log" 2>"$OUT/$tag-$i.err" &
			sleep 1.5
		else
			timeout 900 "${BIN_RUN[@]}" --out "$OUT/$tag-$i" $INPUT_ARGS link-net --join 127.0.0.1:$port --name ${names[$i]} $common $ex >"$OUT/$tag-$i.log" 2>"$OUT/$tag-$i.err" &
			sleep 0.5
		fi
		pids[$i]=$!
	done
	if [ $how = kill ]; then
		local t=0
		while ! grep -q "^frame *$LEAVE_AT: state hash" "$OUT/$tag-$lv.log" 2>/dev/null; do
			sleep 0.2; t=$((t + 1)); [ $t -gt 1500 ] && break
		done
		# `timeout` and the capped wrapper sit above the remodel process: kill the whole chain, by PID.
		local kids; kids=$(pgrep -P ${pids[$lv]} 2>/dev/null)
		for k in $kids $(for x in $kids; do pgrep -P $x; done) ${pids[$lv]}; do kill -9 $k 2>/dev/null; done
	fi
	local rc=() surv=() ok=1 F="" fs=""
	for ((i = 0; i < nodes; i++)); do
		wait ${pids[$i]} 2>/dev/null; rc[$i]=$?
		if [ $i != $lv ]; then
			surv+=($i)
			[ ${rc[$i]} = 0 ] || ok=0
			f=$(sed -n "s/^link-net      drop frame of ${names[$lv]}: \([0-9]*\) .*/\1/p" "$OUT/$tag-$i.log")
			[ -n "$f" ] || ok=0
			fs="$fs $f"
			[ -z "$F" ] && F=$f
			[ "$f" = "$F" ] || ok=0
		fi
	done
	if [ $ok = 0 ]; then bad "$tag" "survivors exit ${rc[*]}, drop frames:$fs: $(cat "$OUT/$tag-"*.err | head -3)"; return; fi
	"${BIN_RUN[@]}" --out "$OUT/$tag-lr" $INPUT_ARGS link-run --nodes $nodes --frames $FRAMES_DROP --delay $dl --leave $lv:$F >"$OUT/$tag-lr.log" 2>&1
	local good=1 detail=""
	for i in "${surv[@]}"; do
		ref=$(hashes "$OUT/$tag-lr.log" | sed -n "$((i + 1))p" | cut -d' ' -f2)
		got=$(hashes "$OUT/$tag-$i.log" | cut -d' ' -f2)
		[ -n "$ref" ] && [ "$ref" = "$got" ] || good=0
		detail="$detail node $i $got"
	done
	# The race goes on after the drop: the survivors' last hash lines are past LEAVE_AT, and no one ended early.
	for i in "${surv[@]}"; do
		grep -q "^frame *$FRAMES_DROP: state hash" "$OUT/$tag-$i.log" || good=0
		grep -q "link-net: " "$OUT/$tag-$i.err" && good=0
	done
	if [ $good = 1 ]; then
		ok "$tag" "${names[$lv]} ${how}s, drop frame $F, survivors$detail = link-run --leave $lv:$F"
		for i in "${surv[@]}"; do grep -h "relayed\|drop frame\|left\|lost" "$OUT/$tag-$i.log" | sed "s/^link-net      /    node $i: /"; done
		mkdir -p "$OUT/captures"
		for i in "${surv[@]}"; do for f in "$OUT/$tag-$i/frames/"*.png; do [ -f "$f" ] && cp "$f" "$OUT/captures/$(echo $tag | tr / -)-n$i-$(basename "$f")"; done; done
	else
		bad "$tag" "drop frame $F, survivors$detail, link-run $(hashes "$OUT/$tag-lr.log" | tr '\n' ' ')"
	fi
}

for game in $GAMES; do
	case $game in
	srallyc)
		INPUT="coin1@1500-1504,coin1@1560-1564,gas=0xff@3430-99999"; FRAMES=4400
		LEAVE_AT=4000; FRAMES_DROP=6000; SHOTS_DROP=4300,5000,6000 ;;
	daytona)
		INPUT="coin1@1100-1104,start@1300-1304,gas=0xff@1500-1510,gas=0xff@1700-1710,gas=0xff@2400-99999"; FRAMES=4600
		LEAVE_AT=5000; FRAMES_DROP=6600; SHOTS_DROP=5300,6000,6600 ;;
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

	# Players leaving mid-race: the others carry on (see drop_case).
	drop_case "$game/drop2-polite" 2 1 quit
	drop_case "$game/drop2-kill" 2 1 kill
	drop_case "$game/drop2-host" 2 0 quit
	drop_case "$game/drop3-polite" 3 1 quit
	drop_case "$game/drop3-kill" 3 2 kill 6

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
