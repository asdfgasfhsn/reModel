#!/bin/sh
# `make materials-identity`: a pack whose normal maps are all flat must draw
# with --materials exactly as the same run with --light and no materials, in
# every shading mode and under both rules, relief (the default) and
# faithful (--materials-relief off) (docs/texture-packs.md, "How material
# maps light").
#
# --materials records the lighting side channel, as --light does; without
# either, `run` has none, and vertex and pixel shading fall back to the
# stream's lumas. So the run without materials to compare with is the one
# with --light, not the bare one: that comparison differs by smooth
# shading's side channel, not by anything a material does.
#
# Environment (the Makefile sets these): REMODEL, GAME, ROM, PACK, WORK,
# STEPS, SNAP.
set -eu
: "${REMODEL:?}" "${GAME:?}" "${ROM:?}" "${PACK:?}" "${WORK:?}" "${STEPS:=900000000}" "${SNAP:=300}"
remodel=$(readlink -f "$REMODEL")
rom=$(readlink -f "$ROM")
rm -rf "$WORK"
mkdir -p "$WORK"
work=$(readlink -f "$WORK")
python3 -I tools/flat-maps.py "$PACK" "$work/pack"

# one LABEL ARGS...: a coin-start run's snapshots into $work/LABEL.
one() {
    label=$1
    shift
    mkdir -p "$work/$label/run"
    (cd "$work/$label/run" && "$remodel" --game "$GAME" --rom "$rom" --geo native --input coin-start \
        --texture-pack "$work/pack" "$@" run "$STEPS" "$SNAP" > "$work/$label/log.txt" 2>&1) ||
        echo "$label: the run failed ($work/$label/log.txt)"
}

# NAME:ARGS, = for a space in ARGS.
modes="flat: vertex:--shading=vertex pixel:--shading=pixel mip:--shading=pixel=--tex-filter=mip"
for m in $modes; do
    name=${m%%:*}
    args=$(echo "${m#*:}" | tr = ' ')
    # shellcheck disable=SC2086
    one "$name-light" --light $args &
    # shellcheck disable=SC2086
    one "$name-relief" --materials $args &
    # shellcheck disable=SC2086
    one "$name-faithful" --materials --materials-relief off $args &
done
wait

fail=0
for m in $modes; do
    name=${m%%:*}
    for rule in relief faithful; do
        a=$work/$name-light/run/out/$GAME/frames
        b=$work/$name-$rule/run/out/$GAME/frames
        n=0
        differ=0
        for f in "$a"/*.png; do
            [ -e "$f" ] || continue
            n=$((n + 1))
            cmp -s "$f" "$b/$(basename "$f")" || differ=$((differ + 1))
        done
        log=$work/$name-$rule/log.txt
        drawn=$(sed -n 's/^materials .* \([0-9][0-9]*\) drawn with a material.*/\1/p' "$log" | awk '{s += $1} END {print s + 0}')
        changed=$(sed -n 's/^materials .*material, \([0-9][0-9]*\) pixels changed.*/\1/p' "$log" | awk '{s += $1} END {print s + 0}')
        echo "$name, $rule: $n snapshots, $differ differ from --light without materials; $drawn polygons drawn with a material, $changed pixels changed"
        if [ "$n" -eq 0 ] || [ "$differ" -ne 0 ] || [ "$changed" -ne 0 ]; then
            fail=1
        fi
        if [ "$drawn" -eq 0 ]; then
            echo "$name, $rule: no polygon drew with a material: the check proves nothing (does the pack map what the game draws?)"
            fail=1
        fi
    done
done
if [ $fail -ne 0 ]; then
    echo "materials-identity: FAIL (pictures and logs in $work)"
    exit 1
fi
echo "materials-identity: ok"
rm -rf "$work"
