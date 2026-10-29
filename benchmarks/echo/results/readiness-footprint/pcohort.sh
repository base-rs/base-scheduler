#!/bin/bash
cd /tmp/claude-scripts
out=http-cohort-P.jsonl; : > $out
H=./hbench.sh
for r in 1 2 3 4 5; do
  for v in A P; do
    for c in 1 64 256; do
      WORKERS=1 $H bin/hello-$v base-native $c >> $out
      WORKERS=1 $H bin/hello-$v base-native-tasks $c >> $out
    done
    for c in 64 256; do
      SCPU=0,1 CCPU=2,3,6,7 WORKERS=2 $H bin/hello-$v base-native-tasks $c >> $out
    done
  done
  echo "repeat $r" >> p-cohort.log
done
echo finished >> p-cohort.log
