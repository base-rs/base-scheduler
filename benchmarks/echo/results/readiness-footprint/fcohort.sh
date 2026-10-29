#!/bin/bash
# fcohort.sh: fixed-rate h2load latency, main (A) vs this change (P), interleaved
cd /tmp/claude-scripts; out=fixed-rate-P.jsonl; : > $out
run() { # BIN BACKEND CONNS PER_CLIENT_RPS
  d=$(mktemp -d)
  WORKERS=1 taskset -c 0 bin/hello-$1 $2 > $d/addr 2>/dev/null & pid=$!
  while [ ! -s $d/addr ]; do sleep 0.05; done; a=$(head -1 $d/addr)
  t=$(( $3 < 4 ? $3 : 4 ))
  taskset -c 1-3,5-7 h2load --h1 -c $3 -t $t --rps $4 -D 4 --warm-up-time 1 http://$a/ > $d/out 2>&1
  kill $pid; wait $pid 2>/dev/null
  python3 - $d/out $1 $2 $3 $4 >> $out <<'PY'
import sys,re,json
f,b,be,c,r=sys.argv[1:]; t=open(f).read()
def us(v): return float(v[:-2])*{'us':1,'ms':1e3}[v[-2:]] if v.endswith(('us','ms')) else float(v[:-1])*1e6
m=re.search(r'time for request:\s+(\S+)\s+(\S+)\s+(\S+)\s+(\S+)',t)
rps=float(re.search(r'finished in \S+, ([\d.]+) req/s',t).group(1))
print(json.dumps(dict(bin=b,backend=be,conns=int(c),offered=int(c)*int(r),rps=round(rps),min=us(m.group(1)),max=us(m.group(2)),mean=us(m.group(3)),sd=us(m.group(4)))))
PY
  rm -rf $d
}
for i in 1 2 3 4 5; do for v in A P; do for be in base-native base-native-tasks; do
  run $v $be 1 30000; run $v $be 64 2000; run $v $be 256 500
done; done; done
echo finished
