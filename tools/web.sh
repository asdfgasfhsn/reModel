#!/bin/sh
# The browser build (`make web`, docs/web.md): a landing page listing the
# supported games whose chips are in ROM/<set>/, and a page per game at
# WEB_OUT/model2/<set>/ with a screenshot and its WebAssembly module.
#
# With RECOMP set, each game gets its own module with only its recompiled
# code (`remodel recomp-files` names the files in out/recomp/), built in
# turn from one target directory. Without, all games share one interpreter
# module at WEB_OUT/model2/pkg/.
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

# Build the module the page is going to load, and bind it into $1.
build() {
    t=$(date +%s)
    env $WASM_CARGO build --release --target wasm32-unknown-unknown -p remodel-web
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
        # Only this game's files, as links, for model2's build script.
        dir=target/web-recomp/$g
        rm -rf "$dir"
        mkdir -p "$dir"
        for f in $files; do
            ln -s "$PWD/out/recomp/$f" "$dir/$f"
        done
        REMODEL_RECOMP_DIR=$PWD/$dir build "$model2/$g/pkg"
        pkg=./pkg/
    fi
    sed "s|@PKG@|$pkg|" crates/web/game.html >"$model2/$g/index.html"
done
if [ -n "$shared" ]; then
    (unset REMODEL_RECOMP_DIR && build "$model2/pkg")
else
    rm -rf "$model2/pkg"
fi

cp crates/web/audio-worklet.js crates/web/sound-worker.js crates/web/geo-worker.js crates/web/touch.js "$model2/"
cp crates/web/index.html "$WEB_OUT/"
cp assets/reModel-logo-1086.png "$WEB_OUT/reModel-logo.png"
# Card posters (assets/posters/README.md).
mkdir -p "$WEB_OUT/posters"
cp assets/posters/*.webp "$WEB_OUT/posters/"
for g in $games; do
    [ -f "assets/posters/$g.webp" ] || echo "web: warning: no poster for $g (assets/posters/README.md); using the placeholder" >&2
done
# Left from the single-page layout (before model2/).
rm -rf "$WEB_OUT/pkg" "$WEB_OUT/audio-worklet.js" "$WEB_OUT/sound-worker.js" "$WEB_OUT/geo-worker.js" "$WEB_OUT/touch.js"
$remodel --rom "$ROM" web-index "$WEB_OUT" >/dev/null
echo "web: built $WEB_OUT in $(($(date +%s) - started)) s: serve it over HTTPS (WebGPU needs a secure page) with $ROM/ at $WEB_OUT/rom/"
