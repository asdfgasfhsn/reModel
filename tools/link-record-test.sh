#!/bin/bash
# The settings records of linked play against what the games write (`make link-record-test`).
#
#   usage: tools/link-record-test.sh REMODEL ROM_DIR OUT_DIR [GAME...]
#
# For each game (manxtt, indy500, motoraid, waverunr by default; also von, stcc, stcca, stccb and
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
# Scratch output stays under OUT_DIR.
set -u
BIN=$1; ROM=$2; OUT=$3; shift 3
GAMES=${*:-manxtt indy500 motoraid waverunr}
mkdir -p "$OUT"
fail=0
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
	env ${dump:+"$dump"} "$cap" "$BIN" --game "$game" --rom "$ROM" --out "$d/written" --input 'test@1300-1330,test@1500-1505' run 300000000 100000 >"$d/written.log" 2>&1
	if msg=$("$cap" "$BIN" --game "$game" --rom "$ROM" link-settings "$d/written/$file" 2>&1); then ok "$game written" "$msg"; else bad "$game written" "$msg"; fi

	input="coin-start"
	[ "$game" = manxtt ] && input="coin1@1100-1104,start@1300-1304"
	"$cap" "$BIN" --game "$game" --rom "$ROM" --out "$d/node" --input "$input" link-run --nodes 2 --frames 2500 --delay 2 >"$d/node.log" 2>&1
	good=1; detail=""
	for i in 0 1; do
		want=$(echo $roles | cut -d' ' -f$((i + 1)))
		msg=$("$cap" "$BIN" --game "$game" --rom "$ROM" link-settings "$d/node/eeprom-$i.bin" 2>&1) || good=0
		[[ $msg =~ "role byte "0x[0-9a-f]+" = "$want($|,) ]] || good=0
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
done
exit $fail
