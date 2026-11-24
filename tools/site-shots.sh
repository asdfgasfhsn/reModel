#!/bin/sh
# The website's screenshots (`make site-shots`; site/README.md), made from
# your ROMs into SITE_SHOTS/<name>.png by the player's own --shot, with
# the player's default settings and none of yours.
#
# Environment (the Makefile sets these):
#   CAP         the memory cap wrapper (tools/capped), run before the player
#   PLAY        absolute path of remodel-play
#   ROM         absolute ROM folder
#   SITE_SHOTS  absolute output folder
#   SHOT        optional: redo just this shot (its name without .png, e.g. seq/wide)
#   FORCE       1: redo shots whose PNG already exists (SHOT= always does)
#
# Why a clean directory: the player reads its settings (and each game's
# picture preset, look and flags) from out/ when the working directory has
# one, else from $XDG_DATA_HOME/remodel, and explicit flags don't override a
# saved preset. Every shot therefore runs in an empty temporary directory
# under out/scratch/ with its own XDG_DATA_HOME, deleted afterwards, so only
# defaults plus the flags below apply. A picture style (looks/) is chosen the
# way the player does, by a `preset = NAME` line in the game's settings
# file there, with no other picture flags.
#
# The shot list is the data at the end: one shot per line,
#   name|game|frame|input|size|flags
# input: @cs (the game's coin-start), @coins (coin-start and three more credits, so
# no INSERT COIN shows), @gas (coin-start, gas held), @race (the same with gear 1
# held, for games that have it),
# @link (start in free play, gas held: for --players), or a script as for
# `remodel run --input`. size: WxH. flags: remodel-play flags and these
# tokens: @enh (--aa 8 --depth on --shading pixel), preset=NAME (picture
# style), crop=WxH+X+Y and scale=N (cut the PNG and enlarge it N times,
# nearest neighbour, with ImageMagick), pack=DIR (a texture pack folder under
# the repository, drawn in real time so its images have decoded; the shot is
# skipped with a note when the folder isn't there, as packs are local studio
# projects), slow (real time instead of --fast), photo=KEYS (the picture is a
# scripted photo-mode photo taken at the frame instead of a --shot: KEYS are
# --photo's KEY=VALUE pairs, comma-separated (docs/photo-mode.md, "Scripted"),
# and the photo is size's height tall in the window's 16:9). Unless preset= is
# given, every picture option is first set to off, whatever the defaults.
#
# Launcher and menu shots: game -, frame is the run's length in seconds and
# input the --nav script (pad presses at seconds), e.g. right@2,accept@3.5.
# The clean profile matters doubly here: no player name, saves or recent games.
# Add a shot by adding a line; names are what site/ refers to.
set -eu
set -f
: "${CAP:=}"
: "${PLAY:?PLAY (the remodel-play path) is needed}"
: "${ROM:?ROM is needed}"
: "${SITE_SHOTS:?SITE_SHOTS is needed}"
: "${SHOT:=}"
: "${FORCE:=}"

root=$(cd "$(dirname "$0")/.." && pwd)
command -v magick >/dev/null || { echo "site-shots: needs ImageMagick (magick)"; exit 1; }
[ -x "$PLAY" ] || { echo "site-shots: no player at $PLAY"; exit 1; }

work=$root/out/scratch/site-shots-run.$$
trap 'rm -rf "$work"' EXIT INT TERM
mkdir -p "$work/cwd" "$SITE_SHOTS"

COINS='coin-start,coin1@1160-1164,coin1@1180-1184,coin1@1200-1204'
GAS='coin-start,gas=0xff@1500-9000'
RACE='coin-start,gas=0xff@1500-9000,gear1@1500-9000'
LINK='start@1560-1564,gas=0xff@2500-9000,gear1@2500-9000'
TO=
command -v timeout >/dev/null && TO='timeout 900' # a bad --input leaves the player waiting, not quitting
ENH='--aa 8 --depth on --shading pixel'
OFF='--aa off --temporal off --depth off --shading flat --bloom off --crt off --smooth off --tex-filter off --run-ahead 0'

shoot() { # name game frame input size flags
	name=$1 game=$2 frame=$3 input=$4 size=$5 flags=$6
	out=$SITE_SHOTS/$name.png
	if [ -z "$SHOT" ] && [ -z "$FORCE" ] && [ -f "$out" ]; then
		echo "site-shots: $name exists, skipped"
		return
	fi
	case $input in @cs) input=coin-start ;; @coins) input=$COINS ;; @gas) input=$GAS ;; @race) input=$RACE ;; @link) input=$LINK ;; esac
	preset= crop= scale=1 pack= slow= photo= args=
	for t in $flags; do
		case $t in
		@enh) args="$args $ENH" ;;
		preset=*) preset=${t#preset=} ;;
		photo=*) photo=${t#photo=} ;;
		crop=*) crop=${t#crop=} ;;
		scale=*) scale=${t#scale=} ;;
		pack=*) pack=${t#pack=} ;;
		slow) slow=1 ;;
		*) args="$args $t" ;;
		esac
	done
	[ -n "$preset" ] || args="$OFF $args"
	if [ -n "$pack" ]; then
		[ -d "$root/$pack" ] || { echo "site-shots: $name skipped: no pack at $pack (a local studio project)"; return; }
		args="$args --texture-pack $root/$pack"
		slow=1
	fi
	if [ "$game" = - ]; then
		run="--seconds $frame${input:+ --nav $input}" what="launcher, $frame s"
	else
		run="--game $game --input $input --frames $frame" what="$game, frame $frame"
		[ -n "$slow" ] || run="$run --fast"
	fi
	case $args in *--players*) args="$args --input2 $input --input3 $input --input4 $input" ;; esac

	rm -rf "$work/xdg" && mkdir -p "$work/xdg/remodel/$game" "$(dirname "$out")"
	[ -z "$preset" ] || printf 'preset = %s\n' "$preset" >"$work/xdg/remodel/$game/settings.txt"
	tmp=$work/shot.png
	rm -f "$tmp"
	if [ -n "$photo" ]; then
		# One photo session at the frame (a second session in the same run
		# would open a few frames later), and a few frames past it to finish.
		rm -rf "$work/photo"
		run="--game $game --input $input --frames $((frame + 5)) --fast" what="$game, photo at frame $frame"
		capture="--photo-dir $work/photo --photo $frame:$photo,res=${size#*x},name=shot"
		made=$work/photo/shot.png
	else
		capture="--shot-size $size --shot $tmp"
		made=$tmp
	fi
	echo "site-shots: $name ($what, $size${preset:+, style $preset}${pack:+, pack})"
	# shellcheck disable=SC2086
	(cd "$work/cwd" && XDG_DATA_HOME=$work/xdg $TO $CAP "$PLAY" --rom "$ROM" --no-sound --no-splash $run \
		$capture $args >"$work/log" 2>&1) || { tail -5 "$work/log"; echo "site-shots: $name FAILED"; echo "$name" >>"$work/failed"; return; }
	[ -f "$made" ] || { tail -5 "$work/log"; echo "site-shots: $name made no picture"; echo "$name" >>"$work/failed"; return; }
	[ "$made" = "$tmp" ] || mv "$made" "$tmp"
	if [ -n "$crop" ]; then
		magick "$tmp" -crop "$crop" +repage -filter point -resize "$((scale * 100))%" "$tmp"
	fi
	mv "$tmp" "$out"
}

list() {
	grep -v '^#' <<'LIST' | grep .
# name|game|frame|input|size|flags
# The sequence: one frame of Daytona USA, native to ultrawide. Chosen because
# its HUD (lap, speed, position, map) spreads furthest to the edges at 16:9.
seq/native-px|daytona|3600|@race|496x384|--aspect 4:3 --bezel off
seq/enhanced|daytona|3600|@race|1440x1080|--aspect 4:3 --bezel off @enh
seq/bezel|daytona|3600|@race|2560x1440|--aspect 4:3 --bezel on @enh
seq/wide|daytona|3600|@race|2560x1440|--aspect fit --hud edges @enh
seq/ultra|daytona|3600|@race|3440x1440|--aspect fit --hud edges @enh
# Picture styles, as the player offers them (Settings, Picture).
looks/original|srallyc|4000|@race|1440x1080|--aspect 4:3 --bezel off preset=original
looks/sharp|srallyc|4000|@race|1440x1080|--aspect 4:3 --bezel off preset=sharp
looks/enhanced|srallyc|4000|@race|1440x1080|--aspect 4:3 --bezel off --run-ahead 0 preset=enhanced
looks/crt|srallyc|4000|@race|1440x1080|--aspect 4:3 --bezel off preset=crt
looks/comic|srallyc|4000|@race|1440x1080|--aspect 4:3 --bezel off preset=comic
# Anti-aliasing: the same frame, a 640x400 crop at 2x.
aa/off|daytona|3600|@race|1440x1080|--aspect 4:3 --bezel off crop=640x400+380+300 scale=2
aa/on|daytona|3600|@race|1440x1080|--aspect 4:3 --bezel off @enh crop=640x400+380+300 scale=2
# Real same-screen link play, 2 and 4 machines.
link/2p|daytona|5000|@link|2560x1440|--players 2 --aa 8
link/4p|daytona|5000|@link|2560x1440|--players 4 --aa 8
# Hero: the most striking gameplay frame, for a big plate near the top.
hero|indy500|3000|@gas|2560x1440|--aspect fit --hud edges @enh
# The launcher on a clean profile: Daytona USA in the carousel (its versions
# and link line), and its game page's Version list, cut to the panel (the
# preview beside it shows the game's logo).
ui/carousel|-|16|right@2,right@2.6|2560x1440|
ui/version|-|36|right@2,right@2.6,accept@3.5,down@30,accept@31|2560x1440|crop=1000x1010+1480+66
# Texture packs: one Top Skater frame (the Novice course scene) without and
# with the studio's cel-shaded showcase pack, cut to the skater and the ground
# (the full frame has real brand boards and a song credit). Local only.
pack/off|topskatr|3300|@cs|1440x1080|--aspect 4:3 --bezel off --aa 4 crop=600x460+420+500
pack/on|topskatr|3300|@cs|1440x1080|--aspect 4:3 --bezel off --aa 4 pack=out/topskatr/studio/cel-pack4/pack crop=600x460+420+500
# Photo mode: the hero's Indy 500 moment as the game shows it (photo mode with
# nothing changed but the HUD on), then with the camera orbited about the car,
# depth of field and motion blur, cut to 1.85:1 clear of the course map (a
# window the game draws with its own camera, which the free camera moves too);
# and Virtua Fighter 2's fighter seen from 45 degrees round.
photo/live|indy500|3000|@gas|1920x1080|--aspect fit --hud edges @enh photo=hud=on
photo/indy|indy500|3000|@gas|1920x1080|--aspect fit @enh photo=dof=1,orbit=-35,blur=16 crop=1480x800+0+280
photo/vf2|vf2|4000|@coins|1920x1080|--aspect fit @enh photo=dof=1,orbit=45
# Gallery.
gallery/bel|bel|4000|@coins|2560x1440|--aspect fit --hud edges @enh
gallery/desert|desert|4000|@coins|2560x1440|--aspect fit --hud edges @enh
gallery/doa|doa|4000|@coins|2560x1440|--aspect fit --hud edges @enh
gallery/indy500|indy500|4000|@gas|2560x1440|--aspect fit --hud edges @enh
gallery/lastbrnx|lastbrnx|4000|@coins|2560x1440|--aspect fit --hud edges @enh
gallery/overrev|overrev|4000|@cs|2560x1440|--aspect fit --hud edges @enh
gallery/sgt24h|sgt24h|4000|@cs|2560x1440|--aspect fit --hud edges @enh
gallery/hotd|hotd|3600|@coins|2560x1440|--aspect fit --hud edges @enh
gallery/vcop2|vcop2|2200|@coins|2560x1440|--aspect fit --hud edges @enh
gallery/gunblade|gunblade|3600|@coins|2560x1440|--aspect fit --hud edges @enh
LIST
}

list | while IFS='|' read -r name game frame input size flags; do
	if [ -n "$SHOT" ] && [ "$SHOT" != "$name" ]; then continue; fi
	shoot "$name" "$game" "$frame" "$input" "$size" "$flags" </dev/null
done

# The picture styles' index, for whoever reads the folder.
if [ -z "$SHOT" ] || [ "${SHOT#looks/}" != "$SHOT" ]; then
	mkdir -p "$SITE_SHOTS/looks"
	cat >"$SITE_SHOTS/looks/README.txt" <<'TXT'
Picture styles (the player's Settings, Picture), one frame of Sega Rally
Championship at 1440x1080, 4:3. Each is the player's preset on a clean
profile (a `preset = NAME` line in the game's settings file), no other flags:
original.png   Arcade original (preset original): everything off, the arcade look
sharp.png      Sharp (preset sharp): 4x multisampling, mipmapped textures
enhanced.png   Smooth modern (preset enhanced): Sharp plus the game's recommended
               extras and smooth motion (run-ahead off here, so the frame
               matches the others)
crt.png        CRT (preset crt): Sharp plus the CRT look, 60 percent
comic.png      Comic (preset comic): Sharp, smooth-lit, ink outlines, toon bands, halftone
Custom is any other combination and is not shown.
TXT
fi
if [ -f "$work/failed" ]; then
	echo "site-shots: FAILED: $(tr '\n' ' ' <"$work/failed")"
	exit 1
fi
echo "site-shots: done ($SITE_SHOTS)"
