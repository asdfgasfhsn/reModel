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
# The same on the GPU: captures of the run (with the side channel), drawn
# by `gpu-check` (the GPU's material pipelines and the software renderer
# beside them, both pictures compared) with --light and with --materials,
# in flat, vertex and pixel shading, unfiltered, mipmapped, 16x
# anisotropic and multisampled with the depth buffer; and `temporal-probe`
# (temporal anti-aliasing: its pictures with --light and with --materials).
#
# Environment (the Makefile sets these): REMODEL, GAME, ROM, PACK, WORK,
# STEPS, SNAP, GPU_SNAP (the captures' interval).
set -eu
: "${REMODEL:?}" "${GAME:?}" "${ROM:?}" "${PACK:?}" "${WORK:?}" "${STEPS:=900000000}" "${SNAP:=300}"
: "${GPU_SNAP:=900}"
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

# The GPU. compare A B LABEL: picture directories A (without materials)
# and B (with flat maps); prints the result and sets fail. Exact is the
# aim, and what vertex and pixel shading give. Without smooth shading the
# frame without materials is drawn by the plain entry points (fs_pack),
# the material polygons here by fs_mat, which compute the pixels their
# material leaves alone with the very same functions; the shader compiler
# rounds a handful of them the other way (topskatr: up to 112 pixels of a
# 1080p frame, one level in one channel), and temporal's first mipmapped
# frame one pixel likewise. So a picture passes exact, or within one
# level on at most 1 in 10,000 pixels (counted); more is a failure.
# gpu-check.png: its software and GPU panels.
compare() {
    n=0
    exact=0
    near=0
    worst=""
    for f in "$1"/*.png; do
        [ -e "$f" ] || continue
        n=$((n + 1))
        if cmp -s "$f" "$2/$(basename "$f")"; then
            exact=$((exact + 1))
            continue
        fi
        # Pictures made from others (gpu-check's amplified differences,
        # temporal-probe's marks) are judged by those others.
        case $f in
            *-identity.png | *-stipple.png)
                n=$((n - 1))
                continue
                ;;
            *gpu-check.png) panels="3 2" ;;
            *) panels="" ;;
        esac
        # shellcheck disable=SC2086
        set -- "$1" "$2" "$3" $(python3 -I tools/png-diff.py "$f" "$2/$(basename "$f")" $panels)
        if [ "$6" -le 1 ] && [ $(($5 * 10000)) -le "$4" ]; then
            near=$((near + 1))
            worst="$worst $(basename "$f"):$5px"
        else
            echo "  $(basename "$f"): $5 of $4 pixels differ, by up to $6"
            fail=1
        fi
        set -- "$1" "$2" "$3"
    done
    echo "gpu $3: $n pictures, $exact exact, $near within one level on a few pixels${worst:+ (}${worst# }${worst:+)}"
    if [ "$n" -eq 0 ] || [ $((exact + near)) -ne "$n" ]; then
        fail=1
    fi
}
mkdir -p "$work/capture-run"
(cd "$work/capture-run" && "$remodel" --game "$GAME" --rom "$rom" --geo native --input coin-start \
    --capture "$work/capture" run "$STEPS" "$GPU_SNAP" > "$work/capture.log" 2>&1) ||
    echo "capture: the run failed ($work/capture.log)"
# gpu LABEL ARGS...: gpu-check of the captures, its pictures into $work/gpu-LABEL.
gpu() {
    label=$1
    shift
    "$remodel" --game "$GAME" --rom "$rom" --texture-pack "$work/pack" "$@" gpu-check "$work/capture" \
        > "$work/gpu-$label.txt" 2>&1 || echo "$label: gpu-check failed ($work/gpu-$label.txt)"
    mkdir -p "$work/gpu-$label"
    for d in "$work/capture"/*/; do
        for f in gpu-check gpu-frame; do
            if [ -e "$d$f.png" ]; then
                mv "$d$f.png" "$work/gpu-$label/$(basename "$d")-$f.png"
            fi
        done
    done
}
gmodes="flat: vertex:--shading=vertex=--tex-filter=mip pixel:--shading=pixel pixel-mip:--shading=pixel=--tex-filter=mip \
pixel-16:--shading=pixel=--tex-filter=16 pixel-aa4-depth:--shading=pixel=--aa=4=--depth"
for m in $gmodes; do
    name=${m%%:*}
    args=$(echo "${m#*:}" | tr = ' ')
    # shellcheck disable=SC2086
    gpu "$name-light" --light $args
    for rule in relief faithful; do
        if [ $rule = relief ]; then
            # shellcheck disable=SC2086
            gpu "$name-$rule" --materials $args
        else
            # shellcheck disable=SC2086
            gpu "$name-$rule" --materials --materials-relief off $args
        fi
        drawn=$(sed -n 's/^  materials: \([0-9][0-9]*\) drawn.*/\1/p' "$work/gpu-$name-$rule.txt" | awk '{s += $1} END {print s + 0}')
        compare "$work/gpu-$name-light" "$work/gpu-$name-$rule" "$name, $rule ($drawn materials drawn, per frame and view)"
        if [ "$drawn" -eq 0 ]; then
            fail=1
        fi
    done
done
# temporal-probe LABEL ARGS...: its pictures (off, the first temporal
# frame, jittered ones, stipple) and table into $work/temporal-LABEL. Its
# own exit status is not the check: on real frames the first temporal frame
# differs from the plain one where thin polygons are widened, with or
# without materials.
tprobe() {
    label=$1
    shift
    "$remodel" --game "$GAME" --rom "$rom" --texture-pack "$work/pack" "$@" --shading pixel \
        temporal-probe "$work/capture" "$work/temporal-$label" > "$work/temporal-$label.txt" 2>&1 || true
}
for t in off:--tex-filter=off mip:--tex-filter=mip; do
    name=${t%%:*}
    args=$(echo "${t#*:}" | tr = ' ')
    # shellcheck disable=SC2086
    tprobe "$name-light" --light $args
    for rule in relief faithful; do
        if [ $rule = relief ]; then
            # shellcheck disable=SC2086
            tprobe "$name-$rule" --materials $args
        else
            # shellcheck disable=SC2086
            tprobe "$name-$rule" --materials --materials-relief off $args
        fi
        compare "$work/temporal-$name-light" "$work/temporal-$name-$rule" "temporal $name, $rule"
    done
done

if [ $fail -ne 0 ]; then
    echo "materials-identity: FAIL (pictures and logs in $work)"
    exit 1
fi
echo "materials-identity: ok"
rm -rf "$work"
