#!/bin/bash
# Interleaved HTTP cohort. Output: /tmp/claude-scripts/http-cohort.jsonl
out=/tmp/claude-scripts/http-cohort.jsonl; : > $out
H=/tmp/claude-scripts/hbench.sh; BIN=/tmp/claude-scripts/bin
for r in 1 2 3; do
  for c in 1 64 256; do
    $H $BIN/hello-A hyper-tokio $c >> $out
    WORKERS=1 $H $BIN/hello-A hyper-spawn-tokio $c >> $out
    for v in A B C D; do
      bin=$BIN/hello-${v/D/C}; cap=; [ $v = D ] && cap=1
      for be in base-native base-iouring base-native-tasks; do
        CAP=$cap WORKERS=1 $H $bin $be $c >> $out
      done
    done
  done
  for c in 64 256; do
    SCPU=0,1 CCPU=2,3,6,7 WORKERS=2 $H $BIN/hello-A hyper-spawn-tokio $c >> $out
    for v in A B C D; do
      bin=$BIN/hello-${v/D/C}; cap=; [ $v = D ] && cap=1
      for be in base-native-tasks base-iouring-tasks; do
        SCPU=0,1 CCPU=2,3,6,7 CAP=$cap WORKERS=2 $H $bin $be $c >> $out
      done
    done
  done
  echo "repeat $r done" >> /tmp/claude-scripts/http-cohort.log
done
echo finished >> /tmp/claude-scripts/http-cohort.log
