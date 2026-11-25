#!/bin/bash
# CI only: run a command in the background and watch the runner while it
# runs: every 5 s the free disk and memory, the largest processes and the
# command's last output line; if the disk or memory falls under 1.5 GB the
# command is killed, so the step ends and its log is kept (a runner that
# runs out dies with no log). usage: tools/ci-watchdog.sh LOG COMMAND...
log=$1; shift
"$@" > "$log" 2>&1 &
pid=$!
while kill -0 "$pid" 2>/dev/null; do
    disk=$(df -Pm / | awk 'NR==2{print $4}')
    mem=$(awk '/MemAvailable/{print int($2/1024)}' /proc/meminfo)
    echo "watchdog $(date +%T) disk free ${disk}M, memory available ${mem}M, last: $(tail -n1 "$log" | cut -c1-120)"
    ps -eo rss=,comm= --sort=-rss | head -3 | awk '{printf "  %6d MB %s\n", $1/1024, $2}'
    if [ "$disk" -lt 1500 ] || [ "$mem" -lt 1500 ]; then
        echo "watchdog: under 1.5 GB free, killing the run"
        du -xsh /tmp "$HOME" "$PWD"/target "$PWD"/out 2>/dev/null
        pkill -KILL -P "$pid"; kill -KILL "$pid"
        tail -n 40 "$log"
        exit 99
    fi
    sleep 5
done
wait "$pid"; code=$?
tail -n 60 "$log"
exit $code
