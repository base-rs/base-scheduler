#!/bin/bash
# echo-events.sh BIN BACKEND CONNS EVENTS -> perf stat lines per round trip
BIN=/tmp/claude-scripts/bin/$1 B=$2 C=$3 EV=$4
exec 3< <(taskset -c 0 $BIN server $B 1 $C)
read -r ADDR <&3
PID=$(pgrep -n -f "$BIN server $B")
taskset -c 1-3 $BIN client $ADDR 3 $C 64 1 5 > /tmp/echo-client.json &
CL=$!
sleep 1.2
perf stat -x, -e $EV -p $PID -- sleep 4.5 2> /tmp/echo-perf.txt
wait $CL; wait
python3 - "$1 $B $C" <<'PY'
import json,sys
c=json.load(open('/tmp/echo-client.json')); n=c['requests_per_second']*4.5
out=[]
for l in open('/tmp/echo-perf.txt'):
    f=l.strip().split(',')
    if len(f)>3 and f[0].replace('.','').isdigit(): out.append(f"{f[2]}={float(f[0])*1e3/n:.3f}us" if f[2].startswith("task-clock") else f"{f[2]}={float(f[0])/n:.1f}")
print(sys.argv[1], round(c['requests_per_second']), ' '.join(out))
PY
