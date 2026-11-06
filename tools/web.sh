#!/bin/sh
# The browser build (`make web`, docs/web.md): a landing page listing the
# supported games whose chips are in ROM/<set>/, and a page per game at
# WEB_OUT/model2/<set>/ with a screenshot and its WebAssembly module.
#
# With RECOMP set, each game gets its own module with only its recompiled
# code (`remodel recomp-files` names its program crates in out/recomp/),
# built in turn from one target directory, each from a workspace of its own
# (`remodel recomp-workspace --web`) that registers just those programs.
# Without, all games share one interpreter module at WEB_OUT/model2/pkg/.
#
# Environment (the Makefile sets these): WEB_OUT, ROM, RECOMP, CARGO,
# WASM_CARGO (a command, may start with VAR=value), WASM_BINDGEN.
set -eu

remodel=target/release/remodel
model2=$WEB_OUT/model2
target=target/web
wasm=$target/wasm32-unknown-unknown/release/remodel_web.wasm
started=$(date +%s)

$CARGO build --release -p remodel
games=$($remodel --rom "$ROM" web-index)
if [ -z "$games" ]; then
    echo "web: no supported game has all its chips in $ROM/<set>/" >&2
fi
mkdir -p "$model2"

# Screenshots from the attract mode, made once (delete shot.png to remake
# one). A failure leaves the card without a picture.
for g in $games; do
    shot=$model2/$g/shot.png
    [ -s "$shot" ] && continue
    mkdir -p "$model2/$g"
    (
        if $remodel --game "$g" --rom "$ROM" --height 360 --aspect 4:3 shot "$shot.tmp" >/dev/null; then
            mv "$shot.tmp" "$shot" && echo "web: $shot"
        else
            rm -f "$shot.tmp"
            echo "web: warning: no screenshot for $g" >&2
        fi
    ) &
done
wait

# Build the module the page is going to load from the workspace whose
# manifest is $1, with the program crates named after it ($3...) built one
# at a time first, as natively (docs/recompiler.md), and bind it into $2.
build() {
    t=$(date +%s)
    manifest=$1 pkg_dir=$2
    shift 2
    for p in "$@"; do
        echo "web: recompiled program $p"
        env $WASM_CARGO build --quiet --release --target wasm32-unknown-unknown --manifest-path "$manifest" -p "remodel-recomp-$(echo "$p" | tr _ -)"
    done
    set -- "$pkg_dir"
    env $WASM_CARGO build --release --target wasm32-unknown-unknown --manifest-path "$manifest" -p remodel-web
    $WASM_BINDGEN --target web --no-typescript --out-dir "$1" "$wasm"
    echo "web: $1 built in $(($(date +%s) - t)) s, $(du -h "$1/remodel_web_bg.wasm" | cut -f1)"
}
export CARGO_TARGET_DIR=$target CARGO_PROFILE_RELEASE_DEBUG=false

shared=
for g in $games; do
    files=
    if [ -n "$RECOMP" ]; then
        files=$($remodel --game "$g" --rom "$ROM" recomp-files)
    fi
    if [ -z "$files" ]; then
        [ -n "$RECOMP" ] && echo "web: $g has no recompiled code in out/recomp: interpreter only"
        shared=1
        pkg=../pkg/
        rm -rf "$model2/$g/pkg"
    else
        # A workspace registering only this game's program crates.
        dir=target/web-recomp/$g
        # shellcheck disable=SC2086 # one argument per program
        $remodel recomp-workspace --web "$dir" $files
        # shellcheck disable=SC2086
        build "$dir/Cargo.toml" "$model2/$g/pkg" $files
        pkg=./pkg/
    fi
    # Its chips' folder under rom/: its own, or a clone's parent's.
    romset=$($remodel --game "$g" --rom "$ROM" rom-folder)
    sed -e "s|@PKG@|$pkg|" -e "s|@ROMSET@|$romset|" crates/web/game.html >"$model2/$g/index.html"
done
if [ -n "$shared" ]; then
    build Cargo.toml "$model2/pkg"
else
    rm -rf "$model2/pkg"
fi

cp crates/web/audio-worklet.js crates/web/sound-worker.js crates/web/geo-worker.js crates/web/touch.js crates/web/gun.js "$model2/"
cp crates/web/index.html LICENSE "$WEB_OUT/"
python3 -I tools/notices.py remodel-web wasm32-unknown-unknown "$WEB_OUT/THIRD-PARTY-NOTICES.txt"
cp assets/reModel-logo-1086.png "$WEB_OUT/reModel-logo.png"
# Card posters (assets/posters/README.md).
mkdir -p "$WEB_OUT/posters"
cp assets/posters/*.webp "$WEB_OUT/posters/"
# A clone uses its parent's poster: `games` lists each set's parent.
for g in $games; do
    parent=$($remodel games | awk -v g="$g" '$1 == g { print $NF }')
    [ -f "assets/posters/$g.webp" ] || [ -f "assets/posters/$parent.webp" ] ||
        echo "web: warning: no poster for $g (assets/posters/README.md); using the placeholder" >&2
done
# Left from the single-page layout (before model2/).
rm -rf "$WEB_OUT/pkg" "$WEB_OUT/audio-worklet.js" "$WEB_OUT/sound-worker.js" "$WEB_OUT/geo-worker.js" "$WEB_OUT/touch.js" "$WEB_OUT/gun.js"
$remodel --rom "$ROM" web-index "$WEB_OUT" >/dev/null
echo "web: built $WEB_OUT in $(($(date +%s) - started)) s: serve it over HTTPS (WebGPU needs a secure page) with $ROM/ at $WEB_OUT/rom/"
