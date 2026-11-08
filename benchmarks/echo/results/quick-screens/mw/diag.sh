#!/bin/bash
# diag.sh BINARY BACKEND WORKERS CONNECTIONS [env...]: one 4 s wrk sample after 1 s warmup; prints req/s, server CPU µs/request and the diagnostic monitor counters per request. Call under the perf lock.
binary=$1 backend=$2 workers=$3 connections=$4; shift 4
case $workers in 1) server=0 client=2,3,6,7;; 2) server=0,1 client=2,3,6,7;; 4) server=0-3 client=4-7;; esac
fifo=$(mktemp -u); mkfifo $fifo
env "$@" taskset -c $server $binary $backend $workers > $fifo 2>/dev/null &
pid=$!
read address < $fifo
wrk=/home/samuel/Developer/ioquatix/wrk/wrk
taskset -c $client $wrk -t4 -c$connections -d1s http://$address/ > /dev/null
sleep 0.3; read r0 s0 < /tmp/mw-diag-$pid 2>/dev/null
c0=$(awk '{print $14+$15}' /proc/$pid/stat)
out=$(taskset -c $client $wrk -t4 -c$connections -d4s --latency http://$address/)
c1=$(awk '{print $14+$15}' /proc/$pid/stat)
sleep 0.3; read r1 s1 < /tmp/mw-diag-$pid 2>/dev/null
kill $pid; wait $pid 2>/dev/null; rm -f $fifo /tmp/mw-diag-$pid
requests=$(echo "$out" | awk '/requests in/ {print $1}')
rps=$(echo "$out" | awk '/Requests\/sec/ {print $2}')
p99=$(echo "$out" | awk '$1=="99%" {print $2}')
python3 -c "
import sys; c0,c1,req,r0,r1,s0,s1=map(float,sys.argv[1:8])
print(f'$backend N=$workers c=$connections $* rps=$rps p99=$p99 cpu_us/req={(c1-c0)*1e4/req:.3f} rebinds/req={(r1-r0)/req:.4f} submits/req={(s1-s0)/req:.4f}')
" $c0 $c1 $requests ${r0:-0} ${r1:-0} ${s0:-0} ${s1:-0}
