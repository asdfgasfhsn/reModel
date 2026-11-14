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
# WASM_CARGO (a command, may start with VAR=value), WASM_BINDGEN, JOBS (how
# many screenshots are made at once; 3 by default), WEB_GAME (optional: rebuild
# only that game's page and module; the rest of the live site is kept).
#
# A module is rebuilt only when what it's built from changed: each pkg/ keeps
# a stamp of reModel's sources (tracked, modified and new files under crates/,
# Cargo.toml, Cargo.lock), the toolchain commands and, for a game's own
# module, its recompiled program crates. The game manifests
# (crates/romset/src/games/) count only for the game and its parent, so
# adding a game doesn't rebuild the others; the shared module counts them
# all. Delete a pkg/ to force a rebuild.
set -eu
: "${JOBS:=3}"
: "${WEB_GAME:=}"

# Built beside the live site (WEB_OUT.next, starting from a copy of it, so
# screenshots and unchanged files carry over) and swapped in at the end, so
# a page loaded mid-build never mixes old and new files. The swap is per
# top-level entry, inside WEB_OUT itself: a server may bind-mount WEB_OUT,
# and replacing the directory would leave it serving the old one.
live=$WEB_OUT
WEB_OUT=$live.next
rm -rf "$WEB_OUT"
if [ -d "$live" ]; then
    cp -a --reflink=auto "$live" "$WEB_OUT"
else
    mkdir -p "$WEB_OUT"
fi

remodel=target/release/remodel
model2=$WEB_OUT/model2
target=target/web
wasm=$target/wasm32-unknown-unknown/release/remodel_web.wasm
started=$(date +%s)

$CARGO build --release -p remodel
version=$($remodel --version | cut -d' ' -f2)
games=$($remodel --rom "$ROM" web-index)
if [ -z "$games" ]; then
    echo "web: no supported game has all its chips in $ROM/<set>/" >&2
fi
if [ -n "$WEB_GAME" ]; then
    if ! echo "$games" | grep -qx "$WEB_GAME"; then
        echo "web: $WEB_GAME is not a listed game with its chips in $ROM/<set>/" >&2
        exit 1
    fi
    games=$WEB_GAME
fi

# The stamp of reModel's sources and the toolchain (see the top).
manifests=crates/romset/src/games
src_key=$({
    git ls-files -s -- crates Cargo.toml Cargo.lock ":!$manifests"
    git diff HEAD -- crates Cargo.toml Cargo.lock ":!$manifests"
    git ls-files -o --exclude-standard -z -- crates ":!$manifests" | xargs -0 -r sha256sum
    echo "$WASM_CARGO"
    $WASM_BINDGEN --version
} | sha256sum | cut -c1-32)
# The stamp for a module: the sources' plus the manifests named by $1 (a
# game's: its own and its parent's; empty: all of them) and its program
# crates' (the rest).
stamp_for() {
    m=$1
    shift
    {
        echo "$src_key"
        if [ -n "$m" ]; then
            for f in $m; do cat "$manifests/$f.rs" 2>/dev/null || true; done
        else
            cat "$manifests"/*.rs
        fi
        for p in "$@"; do
            find "out/recomp/$p" -type f -not -path '*/target/*' | LC_ALL=C sort | xargs -r sha256sum
        done
    } | sha256sum | cut -c1-32
}
# Whether pkg dir $1 was built with stamp $2.
fresh() {
    [ -s "$1/remodel_web_bg.wasm" ] && [ "$(cat "$1/.stamp" 2>/dev/null)" = "$2" ]
}
mkdir -p "$model2"

# Screenshots from the attract mode, made once (delete shot.png to remake
# one). A failure leaves the card without a picture.
running=0
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
    running=$((running + 1))
    if [ "$running" -ge "$JOBS" ]; then
        wait
        running=0
    fi
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
    echo "$stamp" >"$1/.stamp"
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
        parent=$($remodel games | awk -v g="$g" '$1 == g { print $NF }')
        # shellcheck disable=SC2086 # one argument per program
        stamp=$(stamp_for "$g $parent" $files)
        if fresh "$model2/$g/pkg" "$stamp"; then
            echo "web: $g's module is up to date"
        else
            # A workspace registering only this game's program crates.
            dir=target/web-recomp/$g
            # shellcheck disable=SC2086
            $remodel recomp-workspace --web "$dir" $files
            # shellcheck disable=SC2086
            build "$dir/Cargo.toml" "$model2/$g/pkg" $files
        fi
        pkg=./pkg/
    fi
    # Its chips' folder under rom/: its own, or a clone's parent's.
    romset=$($remodel --game "$g" --rom "$ROM" rom-folder)
    sed -e "s|@PKG@|$pkg|" -e "s|@ROMSET@|$romset|" -e "s|@VERSION@|$version|" crates/web/game.html >"$model2/$g/index.html"
done
if [ -n "$shared" ]; then
    stamp=$(stamp_for "")
    if fresh "$model2/pkg" "$stamp"; then
        echo "web: the shared module is up to date"
    else
        build Cargo.toml "$model2/pkg"
    fi
elif [ -z "$WEB_GAME" ]; then
    rm -rf "$model2/pkg"
fi

cp crates/web/audio-worklet.js crates/web/sound-worker.js crates/web/geo-worker.js crates/web/pack-worker.js \
    crates/web/touch.js crates/web/gun.js crates/web/settings.js crates/web/panel.js "$model2/"
sed "s|@VERSION@|$version|" crates/web/index.html >"$WEB_OUT/index.html"
cp LICENSE "$WEB_OUT/"
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
# Bezels beside a 4:3 picture (assets/bezels/README.md); a clone uses its
# parent's, as with posters.
mkdir -p "$WEB_OUT/bezels"
cp assets/bezels/*.webp "$WEB_OUT/bezels/"
# Left from the single-page layout (before model2/).
rm -rf "$WEB_OUT/pkg" "$WEB_OUT/audio-worklet.js" "$WEB_OUT/sound-worker.js" "$WEB_OUT/geo-worker.js" "$WEB_OUT/touch.js" "$WEB_OUT/gun.js" "$WEB_OUT/settings.js" "$WEB_OUT/panel.js"
$remodel --rom "$ROM" web-index "$WEB_OUT" >/dev/null
# model2/ first: a game page, its module and scripts all live there. rom/
# and texpack/ stay: they're where a server mounts the ROMs and the texture
# packs.
mkdir -p "$live"
for e in model2 $(ls -A "$WEB_OUT" | grep -vx -e model2 -e rom -e texpack); do
    [ -e "$WEB_OUT/$e" ] || continue
    if [ -e "$live/$e" ] || [ -L "$live/$e" ]; then
        mv -T --exchange "$WEB_OUT/$e" "$live/$e"
    else
        mv -T "$WEB_OUT/$e" "$live/$e"
    fi
done
rm -rf "$WEB_OUT"
WEB_OUT=$live
echo "web: built $WEB_OUT in $(($(date +%s) - started)) s: serve it over HTTPS (WebGPU needs a secure page) with $ROM/ at $WEB_OUT/rom/"
