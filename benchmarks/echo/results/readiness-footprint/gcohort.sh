#!/bin/bash
# gcohort.sh REPEATS "BIN:BACKEND ..." "CONNS ..." -> echo-events lines
R=$1; VARIANTS=$2; CONNS=$3
for i in $(seq $R); do for c in $CONNS; do for v in $VARIANTS; do
  bash /tmp/claude-scripts/echo-events.sh ${v%%:*} ${v##*:} $c task-clock,instructions:u,cycles:u,mem_inst_retired.lock_loads:u,L1-dcache-load-misses:u
done; done; done
