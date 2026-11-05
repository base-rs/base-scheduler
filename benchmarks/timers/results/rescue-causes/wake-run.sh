#!/bin/bash
# Usage: wake-run.sh BINARY OUTPUT
# Runs wake_latency for every backend, worker count and mode, first on an otherwise idle host and then beside three busy-loop processes, appending one JSON line per run: the condition and the program's record.
set -euo pipefail
B=$1; out=$2
run() {
  for mode in sleep spin; do for workers in 1 4; do for backend in thread tokio native iouring; do
    [ "$backend" = thread ] && [ "$workers" = 4 ] && continue
    record=$(timeout 600 "$B" $backend $workers 20000 $mode)
    echo "{\"condition\": \"$1\", \"record\": $record}" >> "$out"
  done; done; done
}
run idle
spinners=()
for i in 1 2 3; do sh -c 'while :; do :; done' & spinners+=($!); done
trap 'kill "${spinners[@]}" 2>/dev/null || true' EXIT
run loaded
