#!/bin/bash
cd /tmp/claude-scripts
for t in t0 t3 tE; do
  taskset -c 0-3 python3 ~/Developer/claude-bench/sched/benchmarks/timers/run.py --binary bin/$t --output timers-native-$t.jsonl --repeats 3 --backends native > /dev/null 2>&1
done
echo timers done >> e-cohort.log
out=http-cohort-E.jsonl; : > $out
H=./hbench.sh
for r in 1 2 3 4 5; do
  for v in A E; do
    for c in 1 64 256; do
      WORKERS=1 $H bin/hello-$v base-native $c >> $out
      WORKERS=1 $H bin/hello-$v base-native-tasks $c >> $out
    done
    for c in 64 256; do
      SCPU=0,1 CCPU=2,3,6,7 WORKERS=2 $H bin/hello-$v base-native-tasks $c >> $out
    done
  done
  echo "repeat $r" >> e-cohort.log
done
echo finished >> e-cohort.log
