#!/bin/bash
# The settings records of linked play against what the games write (`make link-record-test`).
#
#   usage: tools/link-record-test.sh REMODEL ROM_DIR OUT_DIR [GAME...]
#
# For each game (manxtt, indy500, motoraid, waverunr, sgt24h, overrev, overrevb by default; also von, stcc, stcca, stccb and
# skisuprg; docs/hw/eeprom-records.md):
#   written   a run from an empty EEPROM through the game's own test menu (TEST held, then EXIT)
#             makes the game save its settings record; `remodel link-settings` recomputes the
#             checksum of each copy (romset::RecordCrc) and it must be the one the game wrote
#   node      a two-node `link-run` loads a node image made by romset (role bytes, checksum, second
#             copy) and the game must keep it: the nodes' final EEPROMs still hold the master's and
#             the slave's role (a record the game does not accept is rebuilt from its defaults, role 0)
#             and still pass the checksum
#   changed   (indy500, von, stcc, stcca, stccb) the game's own save of a changed setting (FREE PLAY from
#             its COIN ASSIGNMENTS page, so a new value, counter and checksum) passes the check as well;
#             for von, stcc, stcca and stccb it must be the free play bytes romset::LinkPlay::free_play sets
#   von keeps its settings in the backup SRAM, not the EEPROM: the written record is a dump of it
#   (REMODEL_RAM_DUMP) and a node's is what `link-run` saves of it (eeprom-N.bin, from its settings store)
#   changed   (indy500) the game's own save of a changed setting (FREE PLAY from its COIN
#             ASSIGNMENTS page, so a new value, counter and checksum) passes the check as well
#   menu      (indy500d, indy500to, daytonas, daytonase: the variants, whose games write their records
#             from their own menus) the variant's test menu changes the settings (NETWORK TYPE or
#             LINK ID to MASTER with CABINET ID or CAR NUMBER 2; and, apart, FREE PLAY from its COIN
#             ASSIGNMENT page) and the game saves; the record romset writes for the same settings
#             (`link-run --link-types 1,1` node 1, and `--nodes 1 --link-types 0 --free-play`) must
#             be the game's, byte for byte except the checksums and the write counter (the games'
#             own save raises it: the S32A counter at 0x0C and its copy, the I/O board record's
#             byte 0x7C where the role page was the only change)
# Scratch output stays under OUT_DIR.
set -u
BIN=$1; ROM=$2; OUT=$3; shift 3
GAMES=${*:-manxtt indy500 motoraid waverunr sgt24h overrev overrevb}
mkdir -p "$OUT"
fail=0
# pr NAME START COUNT GAP [LEN]: COUNT presses of an input, GAP frames apart, each held LEN (3) frames.
pr() { local n=$1 s=$2 k=$3 gap=$4 l=${5:-3} o="" i; for ((i=0;i<k;i++)); do o="$o$n@$((s+i*gap))-$((s+i*gap+l)),"; done; echo -n "${o%,}"; }
# The inputs of each variant's test menu (found by running it; the cursor wants presses spaced well apart).
# The GAME ASSIGNMENTS (GAME SYSTEM) page: NETWORK TYPE MASTER, CABINET ID 2 (indy); LINK ID MASTER, CAR NUMBER 2 (daytona).
menu_role() {
	local base="test@1300-1330,$(pr service 1500 7 20),test@1750-1753,$(pr service 2000 7 20),test@2200-2203,service@2300-2303,test@2400-2403"
	case $1 in
	indy500d) echo "$base,service@2500-2503,test@2600-2603,$(pr service 2800 5 20),test@3100-3103" ;;
	indy500to) echo "$base,$(pr service 2500 3 20),test@2700-2703,$(pr service 2800 5 20),test@3100-3103" ;;
	daytonas | daytonase)
		local up=9
		[ "$1" = daytonase ] && up=8
		echo "test@1300-1330,$(pr view4 1600 9 20),start@1900-1903,$(pr view4 2000 $up 20),view3@2300-2303,service@2400-2403,view3@2500-2503,view4@2600-2603,view4@2700-2703,start@2800-2803,$(pr service 3000 9 20),start@3300-3303" ;;
	esac
}
# The COIN ASSIGNMENT page: COIN/CREDIT SETTING #27, FREE PLAY.
menu_free() {
	case $1 in
	indy500d | indy500to)
		local n=6
		[ "$1" = indy500to ] && n=8
		echo "test@1300-1330,$(pr service 1500 8 20),test@1800-1803,service@2000-2003,$(pr test 2100 $n 100 0),service@3000-3003,service@3100-3103,test@3200-3203,$(pr service 3400 4 100 0),test@3900-3903" ;;
	daytonas | daytonase)
		echo "test@1300-1330,$(pr view4 1600 8 20),start@1900-1903,view4@2100-2103,view2@2200-2203,service@2400-2403,start@2500-2503,$(pr service 2700 9 20),start@3000-3003" ;;
	esac
}
# The 1-based byte offsets (as cmp -l prints) at which files $1 and $2 differ, except those listed in $3.
diff_but() { cmp -l "$1" "$2" | awk -v skip=" $3 " '{ if (index(skip, " " $1 " ") == 0) print $1 }' | tr '\n' ' '; }
ok() { echo "link-record $1: ok${2:+ ($2)}"; }
bad() { echo "link-record $1: FAIL ${2:-}"; fail=1; }
cap="$(dirname "$0")/capped"
for game in $GAMES; do
	d="$OUT/$game"; rm -rf "$d"; mkdir -p "$d/written"
	# The role byte each node must still hold after a link-run, and the file the game's settings are in.
	roles="1 2"; file=eeprom.bin; dump=""
	case $game in
	von) roles="1 0"; file=sram.bin; dump="REMODEL_RAM_DUMP=0x1D00000:$d/written/sram.bin" ;;
	stcc | stcca | stccb) roles="33 65" ;;
	esac
	case $game in
	daytonas | daytonase) ;; # its record is the I/O board's, not an EEPROM the game writes unprompted: see `menu`
	*)
		env ${dump:+"$dump"} "$cap" "$BIN" --game "$game" --rom "$ROM" --out "$d/written" --input 'test@1300-1330,test@1500-1505' run 300000000 100000 >"$d/written.log" 2>&1
		if msg=$("$cap" "$BIN" --game "$game" --rom "$ROM" link-settings "$d/written/$file" 2>&1); then ok "$game written" "$msg"; else bad "$game written" "$msg"; fi ;;
	esac

	input="coin-start"
	[ "$game" = manxtt ] && input="coin1@1100-1104,start@1300-1304"
	"$cap" "$BIN" --game "$game" --rom "$ROM" --out "$d/node" --input "$input" link-run --nodes 2 --frames 2500 --delay 2 >"$d/node.log" 2>&1
	good=1; detail=""
	for i in 0 1; do
		want=$(echo $roles | cut -d' ' -f$((i + 1)))
		msg=$("$cap" "$BIN" --game "$game" --rom "$ROM" link-settings "$d/node/eeprom-$i.bin" 2>&1) || good=0
		case $game in
		# Their role byte is not the node number plus one (sgt24h: the master flag; overrev: LINK MAX,
		# the same for every node): the ID byte, the node's index, is what tells the nodes apart.
		sgt24h | overrev | overrevb) [[ $msg =~ "ID byte "0x[0-9a-f]+" = "$i($|,) ]] || good=0 ;;
		*) [[ $msg =~ "role byte "0x[0-9a-f]+" = "$want($|,) ]] || good=0
		esac
		detail="$detail node $i: ${msg#*checksums ok, };"
	done
	if [ $good = 1 ]; then ok "$game node" "$detail"; else bad "$game node" "$detail"; fi

	# FREE PLAY from the COIN ASSIGNMENTS page, saved by the game: von (service steps to the page at row 8,
	# the setting at row 3, #27 after 26 presses), stcc and its clones (the setting is row 1).
	changed=""
	case $game in
	von) changed='test@1300-1330,service@1500-1641/20,test@1750-1755,service@1850-1891/20,test@1950-2200/10,service@2300-2321/20,test@2400-2405,service@2500-2541/20,test@2600-2605' ;;
	stcc | stcca | stccb) changed='test@1300-1330,service@1500-1641/20,test@1750-1755,service@1850-1851,test@1950-2200/10,service@2300-2301,test@2400-2405,service@2500-2541/20,test@2650-2655' ;;
	esac
	if [ -n "$changed" ]; then
		mkdir -p "$d/changed"
		env ${dump:+"REMODEL_RAM_DUMP=0x1D00000:$d/changed/sram.bin"} "$cap" "$BIN" --game "$game" --rom "$ROM" --out "$d/changed" --input "$changed" run 480000000 100000 >"$d/changed.log" 2>&1
		msg=$("$cap" "$BIN" --game "$game" --rom "$ROM" link-settings "$d/changed/$file" 2>&1)
		case $msg in
		*"checksums ok"*", free play"*) ok "$game changed" "$msg" ;;
		*) bad "$game changed" "$msg" ;;
		esac
	fi

	if [ "$game" = indy500 ]; then
		mkdir -p "$d/changed"
		"$cap" "$BIN" --game "$game" --rom "$ROM" --out "$d/changed" --input 'test@1300-1330,service@1500-1641/20,test@1700-1705,service@1750-1751,test@1800-1940/10,service@2000-2021/20,test@2100-2105,service@2200-2261/20,test@2400-2405' run 400000000 100000 >"$d/changed.log" 2>&1
		msg=$("$cap" "$BIN" --game "$game" --rom "$ROM" link-settings "$d/changed/eeprom.bin" 2>&1)
		case $msg in
		*"checksums ok"*"write counter 102"*) ok "$game changed" "$msg" ;;
		*) bad "$game changed" "$msg" ;;
		esac
	fi

	case $game in
	indy500d | indy500to | daytonas | daytonase)
		# The game's own saves: the role page, and the coin page (each on a fresh store), against romset's records.
		store=eeprom.bin fresh="" steps=1000000000 skip="9 10 13 53 54 57"
		[ "$game" = indy500to ] && skip="9 10 13 45 46 49"
		skipr="$skip"
		case $game in daytonas | daytonase) store=iostore.bin fresh="--fresh-eeprom" steps=700000000 skip="9 10" skipr="9 10 125" ;; esac
		mkdir -p "$d/mr" "$d/mf" "$d/lr" "$d/lf"
		"$cap" "$BIN" --game "$game" --rom "$ROM" --out "$d/mr" $fresh --input "$(menu_role "$game")" run $steps 100000 >"$d/mr.log" 2>&1 &
		"$cap" "$BIN" --game "$game" --rom "$ROM" --out "$d/mf" $fresh --input "$(menu_free "$game")" run $steps 100000 >"$d/mf.log" 2>&1 &
		wait
		"$cap" "$BIN" --game "$game" --rom "$ROM" --out "$d/lr" link-run --nodes 2 --link-types 1,1 --frames 60 --delay 2 >"$d/lr.log" 2>&1
		"$cap" "$BIN" --game "$game" --rom "$ROM" --out "$d/lf" link-run --nodes 1 --link-types 0 --free-play --frames 60 --delay 2 >"$d/lf.log" 2>&1
		for c in "mr:lr/eeprom-1.bin:$skipr:role" "mf:lf/eeprom-0.bin:$skip:free play"; do
			IFS=: read -r g f sk what <<<"$c"
			if [ -s "$d/$g/$store" ] && [ -s "$d/$f" ] && msg=$("$cap" "$BIN" --game "$game" --rom "$ROM" link-settings "$d/$g/$store" 2>&1) && diffs=$(diff_but "$d/$g/$store" "$d/$f" "$sk") && [ -z "$diffs" ]; then
				ok "$game menu $what" "the game's own save is the record romset writes: $msg"
			else
				bad "$game menu $what" "$msg; differing bytes (1-based) ${diffs:-?}"
			fi
		done ;;
	esac
done
exit $fail
