#!/bin/bash
# usage: [SCPU= CCPU= WORKERS= CAP=1] hbench.sh BIN BACKEND CONNS -> one JSON line
B=$1; be=$2; c=$3
scpu=${SCPU:-0}; ccpu=${CCPU:-1-3,5-7}; W=~/Developer/ioquatix/wrk/wrk
d=$(mktemp -d)
if [ -n "${CAP:-}" ]; then export BASE_NATIVE_BATCH_CAP=1; else unset BASE_NATIVE_BATCH_CAP; fi
export WORKERS=${WORKERS:-1}
taskset -c $scpu $B $be > $d/addr 2>/dev/null & pid=$!
while [ ! -s $d/addr ]; do sleep 0.05; done; url=http://$(head -1 $d/addr)/
t=$(( c < 4 ? c : 4 ))
taskset -c $ccpu $W -t$t -c$c -d1s $url >/dev/null
perf stat -x, -e instructions:u,cycles:u,task-clock -p $pid -o $d/stat -- sleep 3.2 &
sp=$!
taskset -c $ccpu $W -t$t -c$c -d3s --latency $url > $d/wrk
wait $sp; kill $pid; wait $pid 2>/dev/null
python3 - "$d" "$be" "$c" "$B" "$WORKERS" "${CAP:-}" <<'PY'
import sys,re,json
d,be,c,b,w,cap=sys.argv[1:]
txt=open(d+'/wrk').read()
req=int(re.search(r'(\d+) requests in',txt).group(1))
def us(p):
    m=re.search(r'\s'+p+r'%\s+([\d.]+)(us|ms|s)\b',txt)
    v=float(m.group(1)); return v*{'us':1,'ms':1e3,'s':1e6}[m.group(2)]
st={}
for l in open(d+'/stat'):
    f=l.split(',')
    if len(f)>2 and f[0] not in('','<not counted>'):
        st[f[2].split(':')[0]]=float(f[0])
print(json.dumps(dict(bin=b.rsplit('-',1)[-1]+('+cap' if cap else ''),backend=be,workers=int(w),conns=int(c),rps=round(req/3),
  ipr=round(st['instructions']/req),cpr=round(st['cycles']/req),cpu_us=round(st['task-clock']*1000/req,3),p50=us('50'),p99=us('99'))))
PY
rm -rf $d
