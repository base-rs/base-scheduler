#!/bin/bash
# profile.sh BINARY BACKEND WORKERS CONNECTIONS OUT [perf args...]: run wrk against a server and attach perf to the server for the measured interval.
# Call under flock /tmp/base-scheduler-perf.lock.
set -e
binary=$1 backend=$2 workers=$3 connections=$4 out=$5; shift 5
case $workers in 1) server=0 client=2,3,6,7;; 2) server=0,1 client=2,3,6,7;; 4) server=0-3 client=4-7;; esac
fifo=$(mktemp -u); mkfifo $fifo
taskset -c $server $binary $backend $workers > $fifo 2>/dev/null &
pid=$!
read address < $fifo
wrk=/home/samuel/Developer/ioquatix/wrk/wrk
taskset -c $client $wrk -t4 -c$connections -d1s http://$address/ > /dev/null
taskset -c $client $wrk -t4 -c$connections -d6s --latency http://$address/ > $out.wrk &
w=$!
sleep 1
perf "$@" -p $pid -o $out -- sleep 4 2> $out.perf-stderr || true
wait $w
kill $pid; wait $pid 2>/dev/null || true
rm -f $fifo
grep -E "Requests/sec|requests in" $out.wrk
