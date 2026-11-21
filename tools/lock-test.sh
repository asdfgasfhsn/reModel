#!/bin/sh
# Tests of tools/lock (`make lock-test`): a private lock folder and a private
# stand-in for out/heavy.lock, so nothing here touches a real lock.
set -u
here=$(cd "$(dirname "$0")" && pwd)
lock="$here/lock"
work=${REMODEL_LOCK_TEST_DIR:-$here/../out/scratch/lock-test}
rm -rf "$work"; mkdir -p "$work"
export REMODEL_LOCK_DIR="$work/locks" REMODEL_HEAVY_LOCK="$work/heavy.lock"
unset REMODEL_LOCKS_HELD
bad=0
ok()   { echo "ok    $1"; }
fail() { echo "FAIL  $1"; bad=1; }
check() { # check NAME CONDITION-COMMAND...
    n=$1; shift; if "$@"; then ok "$n"; else fail "$n"; fi
}
contains() { grep -q -- "$1" "$2"; }

# 1. two holders of a one-slot class: the second's bounded wait expires with
# exit 3 and names the first.
"$lock" build -- sleep 3 & h=$!
sleep 0.7
"$lock" build --wait 1 -- true 2> "$work/e1"; rc=$?
check "bounded wait gives up with exit 3" test $rc = 3
check "the wait names the holder" contains "sleep 3" "$work/e1"
# 2. status shows the holder, as text and as JSON.
"$lock" status > "$work/s1"
check "status lists the holder" contains "held     build" "$work/s1"
"$lock" status --json > "$work/s1j"
check "status --json has the holder" python3 -I -c '
import json,sys
d=json.load(open(sys.argv[1]))
assert any(h["class"]=="build" for h in d["holders"]), d' "$work/s1j"
# 3. another class does not wait behind it (render never behind a build).
start=$(date +%s)
"$lock" render --wait 5 -- true; rc=$?
check "a render does not wait for a build" test $rc = 0
check "...and starts at once" test $(( $(date +%s) - start )) -le 1
wait $h
# 4. the wait that does fit: second starts when the first ends, exit passes through.
"$lock" test -- sleep 1 & h=$!
sleep 0.4
"$lock" test --wait 10 -- sh -c 'exit 5'; rc=$?
check "a wait that fits runs the command, its exit code passes through" test $rc = 5
wait $h
# 5. a holder that died (killed hard) frees the lock at once, and status does
# not list it.
"$lock" build -- sleep 30 & h=$!
sleep 0.5
python3 -I - "$h" <<'EOF'
import os, signal, subprocess, sys
# kill the lock tool and its child: the lock goes with the process
pid = int(sys.argv[1])
subprocess.run(["pkill", "-KILL", "-P", str(pid)])
os.kill(pid, signal.SIGKILL)
EOF
wait $h 2>/dev/null
"$lock" build --wait 3 -- true; rc=$?
check "a killed holder's lock is free again" test $rc = 0
"$lock" status > "$work/s2"
check "a dead holder is not listed" sh -c "! grep -q 'held     build' '$work/s2'"
# 6. render has three slots: three at once, a fourth waits and gives up.
for i in 1 2 3; do "$lock" render -- sleep 3 & done
sleep 0.7
"$lock" render --wait 1 -- true 2> "$work/e6"; rc=$?
check "three renders at once, the fourth waits (exit 3)" test $rc = 3
"$lock" status > "$work/s6"
check "status shows three render holders" test "$(grep -c 'held     render' "$work/s6")" = 3
wait
# 7. nesting: a class already held is not taken again (make targets nest).
"$lock" build -- "$lock" build --wait 1 -- sh -c 'exit 0'; rc=$?
check "a nested take of a held class does not deadlock" test $rc = 0
"$lock" build -- "$lock" build,test --wait 1 -- sh -c 'echo $REMODEL_LOCKS_HELD' > "$work/n"
check "a nested class adds to what is held" contains "build" "$work/n"
# 8. the hand-typed lock: build waits for it, and runs under it without deadlock.
flock "$REMODEL_HEAVY_LOCK" sleep 3 & h=$!
sleep 0.5
"$lock" build --wait 1 -- true 2> "$work/e8"; rc=$?
check "build honours a hand-typed flock on heavy.lock (waits, exit 3)" test $rc = 3
"$lock" status > "$work/s8"
check "status shows the hand-typed holder" contains "by hand" "$work/s8"
"$lock" render --wait 1 -- true; rc=$?
check "a render does not wait for heavy.lock" test $rc = 0
wait $h
flock "$REMODEL_HEAVY_LOCK" "$lock" build,test --wait 2 -- true; rc=$?
check "flock heavy.lock tools/lock build does not deadlock on itself" test $rc = 0
REMODEL_LEGACY_LOCK=0 flock "$REMODEL_HEAVY_LOCK" "$lock" build --wait 2 -- true; rc=$?
check "REMODEL_LEGACY_LOCK=0 ignores heavy.lock" test $rc = 0
# 9. a wrong request is exit 2.
"$lock" nonsense -- true 2>/dev/null; rc=$?
check "an unknown class is exit 2" test $rc = 2
"$lock" build 2>/dev/null; rc=$?
check "no command is exit 2" test $rc = 2

rm -rf "$work"
if [ $bad = 0 ]; then echo "lock-test: all passed"; else echo "lock-test: FAILED"; fi
exit $bad
