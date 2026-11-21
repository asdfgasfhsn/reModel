#!/bin/sh
# Slice A's acceptance test (docs/uplift/agent-environment.md, item 2/3):
# with someone else's build and test run holding their locks, an art agent
# on a pinned binary checks a picture and drives the studio without
# waiting, every answer is a parseable object with the right exit code, and
# nothing it does overwrites what is there.
#
#   tools/agent-env-test.sh [REMODEL_BINARY]     (make agent-env-test)
#
# No ROM, no GPU. Private lock folder; scratch in out/scratch/agent-env-test.
set -u
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
bin=${1:-$root/target/release/remodel}
[ -x "$bin" ] || { echo "agent-env-test: $bin is not built (make release, or give a binary)"; exit 3; }
work="$root/out/scratch/agent-env-test"
rm -rf "$work"; mkdir -p "$work"
export REMODEL_LOCK_DIR="$work/locks" REMODEL_HEAVY_LOCK="$work/heavy.lock"
unset REMODEL_LOCKS_HELD
bad=0
ok()   { echo "ok    $1"; }
fail() { echo "FAIL  $1"; bad=1; }
check() { n=$1; shift; if "$@"; then ok "$n"; else fail "$n"; fi; }
now() { python3 -I -c 'import time; print(time.time())'; }
secs() { python3 -I -c 'import sys; print("%.2f" % (float(sys.argv[2]) - float(sys.argv[1])))' "$1" "$2"; }
under() { python3 -I -c 'import sys; sys.exit(0 if float(sys.argv[1]) < float(sys.argv[2]) else 1)' "$1" "$2"; }
json() { python3 -I -c 'import json,sys; d=json.load(open(sys.argv[2])); sys.exit(0 if eval(sys.argv[1]) else 1)' "$1" "$2"; }

# Another agent builds and tests: both locks held, and the hand-typed one too.
"$here/lock" build,test -- sleep 25 & holder=$!
sleep 1

# 1. a still check needs no build and no lock: a picture, its numbers, a diff.
python3 -I - "$work" <<'EOF'
import struct, sys, zlib
d = sys.argv[1]
def png(path, w, h, f):
    raw = b"".join(b"\x00" + b"".join(bytes(f(x, y)) for x in range(w)) for y in range(h))
    def ch(t, b): return struct.pack(">I", len(b)) + t + b + struct.pack(">I", zlib.crc32(t + b))
    open(path, "wb").write(b"\x89PNG\r\n\x1a\n" + ch(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + ch(b"IDAT", zlib.compress(raw)) + ch(b"IEND", b""))
png(d + "/a.png", 64, 64, lambda x, y: (200, 100, 50))
png(d + "/b.png", 64, 64, lambda x, y: (200, 100, 50) if x < 32 else (10, 10, 10))
EOF
t0=$(now)
"$here/lock" render --wait 5 -- "$bin" img stats "$work/a.png" --json > "$work/stats.json"; rc1=$?
"$here/lock" render --wait 5 -- "$bin" img diff "$work/a.png" "$work/b.png" --json > "$work/diff.json"; rc2=$?
"$here/lock" render --wait 5 -- "$bin" img sheet "$work/sheet.png" "$work/a.png" "$work/b.png" --json > "$work/sheet.json"; rc3=$?
t1=$(now)
el=$(secs "$t0" "$t1")
check "stats, diff and sheet run at once beside a build and a test run ($el s)" test "$rc1$rc2$rc3" = 000
check "...in under 2 s" under "$el" 2
check "stats says the picture is opaque" json 'd["ok"] and d["data"]["opaque"] == 1.0' "$work/stats.json"
check "diff says half changed" json 'd["data"]["changed_pct"] == 50.0' "$work/diff.json"
check "the sheet has two labelled cells" json 'len(d["data"]["cells"]) == 2 and d["data"]["cells"][0]["label"] == "a"' "$work/sheet.json"
"$bin" img diff "$work/a.png" "$work/b.png" --max-changed 10 > /dev/null 2>&1; rc=$?
check "a gate that fails is exit 1" test $rc = 1
"$bin" img stats "$work/nope.png" --json > "$work/missing.json" 2>/dev/null; rc=$?
check "a missing file is exit 3 and says so in the object" test $rc = 3
json 'd["code"] == 3 and not d["ok"] and "not found" in d["error"]' "$work/missing.json" && ok "...as {ok, code, error}" || fail "...as {ok, code, error}"
"$bin" img sheet "$work/sheet.png" "$work/a.png" > /dev/null 2>&1; rc=$?
check "a sheet will not overwrite one (exit 2)" test $rc = 2
"$bin" img diff "$work/a.png" "$work/b.png" --thresold 3 > "$work/e.txt" 2>&1; rc=$?
check "an unknown option is exit 2 naming the near one" sh -c "[ $rc = 2 ] && grep -q 'did you mean --threshold' '$work/e.txt'"

# 2. make does not build a pinned binary, and does not wait for the build lock.
t0=$(now)
make -s -C "$root" release REMODEL="$bin" > "$work/make.txt" 2>&1; rc=$?
t1=$(now)
check "make release with a given binary builds nothing and does not queue ($(secs "$t0" "$t1") s)" sh -c "[ $rc = 0 ] && grep -q 'no build' '$work/make.txt'"

# 3. the studio, strict, with answers.
p="$work/proj"
"$bin" studio init "$p" --name t --game g > /dev/null; rc=$?
check "studio init" test $rc = 0
"$bin" studio status "$p" --json > "$work/st.json"; rc=$?
check "studio status --json: ok, code 0, a data object" sh -c "[ $rc = 0 ]"
json 'd["ok"] and d["code"] == 0 and "lines" in d["data"] and d["warnings"] == [] and d["next"] is None and d["error"] is None' "$work/st.json" && ok "...with every field of the answer" || fail "...with every field of the answer"
"$bin" studio status "$work/not-a-project" --json > "$work/st2.json" 2>/dev/null; rc=$?
check "a project that is not there is exit 3" test $rc = 3
"$bin" studio draft "$p" --replce > "$work/e2.txt" 2>&1; rc=$?
check "studio: an unknown option is exit 2 naming the near one" sh -c "[ $rc = 2 ] && grep -q 'did you mean --replace' '$work/e2.txt'"
"$bin" studio drat "$p" --json > "$work/e3.json" 2>/dev/null; rc=$?
check "studio: an unknown subcommand is exit 2" test $rc = 2

kill $holder 2>/dev/null; wait $holder 2>/dev/null
rm -rf "$work"
if [ $bad = 0 ]; then echo "agent-env-test: all passed"; else echo "agent-env-test: FAILED"; fi
exit $bad
