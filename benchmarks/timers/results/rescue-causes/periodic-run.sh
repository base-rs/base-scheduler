#!/bin/bash
B=$1; out=$2; runs=$3
for i in $(seq 1 $runs); do
  for backend in native iouring; do for alignment in unaligned aligned; do
    log=$(mktemp)
    record=$(BASE_NATIVE_RESCUE_LOG=$log $B periodic $backend 4 1024 1000 $alignment)
    python3 -c "
import json,sys
r=json.loads(sys.argv[1]); events=[json.loads(l) for l in open(sys.argv[2])]
print(json.dumps({'case':f'periodic {r[\"backend\"]} 4/1024/1000us {r[\"alignment\"]}','repeat':int(sys.argv[3]),'record':r,'events':events}))" "$record" "$log" "$i" >> $out
    rm -f $log
  done; done
done
