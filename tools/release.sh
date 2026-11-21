#!/bin/sh
# Release helpers (`make bench-all`, `make release-tag`; docs/usage.md,
# "Releases").
#
#   tools/release.sh bench-all             bench every set in tools/expected-hashes.txt
#                                          that has ROMs, compare the state hashes
#   tools/release.sh bench-variants        the same for the variants in
#                                          tools/expected-variant-hashes.txt (kept
#                                          apart: not part of a release's gate)
#   tools/release.sh tag X.Y.Z             check everything, then tag vX.Y.Z (never pushes)
#
# Environment (the Makefile sets these): ROM (default rom), MAKE, REMODEL
# (the release binary), DRY_RUN (tag: do all the checks but create no tag),
# SKIP_BUILD (tag: skip `make check` and `make release`; for testing this
# script).
#
# tools/expected-hashes.txt: one `set hash` per line, the `state hash` of
# `make bench GAME=set` (the default fixed run). The hashes change when
# emulation changes on purpose: update the line in the same commit.
set -eu
cd "$(dirname "$0")/.."
: "${ROM:=rom}"
: "${MAKE:=make}"
: "${DRY_RUN:=}"
: "${SKIP_BUILD:=}"
expected=tools/expected-hashes.txt
expected_variants=tools/expected-variant-hashes.txt

# Sets in the list (default: the parents') whose ROMs are there. A clone's
# ROMs may be in its parent's folder, so `remodel rom-folder` says where to
# look; a variant's own chips may be in subfolders of its parent's zip, so
# for those (`variants`) the answer is whether `remodel verify` loads it.
bench_all() {
    list=${1:-$expected} label=${2:-bench-all}
    bad=0 ran=0 skipped=
    while read -r set want; do
        case $set in '' | '#'*) continue ;; esac
        if [ "$label" = bench-variants ]; then
            "${REMODEL:-target/release/remodel}" --game "$set" --rom "$ROM" verify >/dev/null 2>&1 || {
                skipped="$skipped $set"
                continue
            }
        else
            folder=$("${REMODEL:-target/release/remodel}" --game "$set" --rom "$ROM" rom-folder 2>/dev/null) || folder=$set
            if [ ! -e "$ROM/$folder" ] && [ ! -e "$ROM/$folder.zip" ]; then
                skipped="$skipped $set"
                continue
            fi
        fi
        got=$($MAKE --no-print-directory bench GAME="$set" ROM="$ROM" | sed -n 's/^state hash *//p' | grep -o '^[0-9a-f]\{16\}' | head -n 1)
        ran=$((ran + 1))
        if [ "$got" = "$want" ]; then
            echo "$label: $set $got ok"
        else
            echo "$label: $set MISMATCH: got ${got:-nothing}, expected $want" >&2
            bad=$((bad + 1))
        fi
    done <"$list"
    [ -n "$skipped" ] && echo "$label: no ROMs for:$skipped (not checked)"
    if [ "$bad" -ne 0 ]; then
        echo "$label: $bad of $ran state hashes differ from $list" >&2
        return 1
    fi
    echo "$label: $ran state hashes match $list"
}

die() {
    echo "release-tag: $*" >&2
    exit 1
}

tag() {
    version=${1:-}
    case $version in
        [0-9]*.[0-9]*.[0-9]*) ;;
        *) die "usage: make release-tag VERSION=x.y.z" ;;
    esac
    case $version in *[!0-9.]*) die "VERSION is x.y.z, digits only (no leading v)" ;; esac
    tag=v$version
    git rev-parse -q --verify "refs/tags/$tag" >/dev/null && die "tag $tag already exists"

    # Cheap checks first.
    [ -z "$(git status --porcelain --untracked-files=no)" ] || die "the working tree isn't clean (commit or stash first)"
    cargo_version=$(awk '/^\[workspace.package\]/ { w = 1; next } /^\[/ { w = 0 } w && $1 == "version" { gsub(/"/, "", $3); print $3 }' Cargo.toml)
    [ "$cargo_version" = "$version" ] || die "Cargo.toml's workspace version is ${cargo_version:-missing}, not $version"
    notes=$(awk -v v="$version" '
        /^## / { if (on) exit; split($2, a, " "); on = (a[1] == v); next }
        on { print }' CHANGELOG.md)
    [ -n "$(printf '%s' "$notes" | tr -d ' \n')" ] || die "CHANGELOG.md has no \`## $version\` section with text"

    if [ -z "$SKIP_BUILD" ]; then
        $MAKE check
        $MAKE release
    fi
    REMODEL=${REMODEL:-target/release/remodel}
    export REMODEL
    bench_all || die "state hashes changed: update $expected if that was intended, or fix the change"
    [ -z "$(git status --porcelain --untracked-files=no)" ] || die "the working tree changed during the checks"

    # Subject: the heading's title (`## 0.6.0 (date): title`), as earlier tags have it.
    title=$(sed -n "s/^## $version ([^)]*): *//p" CHANGELOG.md | head -n 1)
    message=$(printf 'reModel v%s%s\n\n%s\n' "$version" "${title:+: $title}" "$(printf '%s\n' "$notes" | sed -e '/./,$!d')")
    if [ -n "$DRY_RUN" ]; then
        echo "release-tag: dry run: would tag $tag at $(git rev-parse --short HEAD) with:"
        printf '%s\n' "$message" | sed 's/^/    /'
        return 0
    fi
    git tag -a "$tag" -m "$message"
    echo "release-tag: tagged $tag at $(git rev-parse --short HEAD). Nothing was pushed; to publish:"
    echo "    git push origin HEAD $tag"
}

case ${1:-} in
    bench-all) bench_all ;;
    bench-variants) bench_all "$expected_variants" bench-variants ;;
    tag) tag "${2:-}" ;;
    *) die "usage: tools/release.sh bench-all | bench-variants | tag X.Y.Z" ;;
esac
