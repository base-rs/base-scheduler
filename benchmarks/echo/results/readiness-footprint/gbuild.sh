#!/bin/bash
# gbuild.sh NAME: build sched-gap echo benchmark into /tmp/claude-scripts/bin/NAME
cd ~/Developer/claude-bench/sched-gap && CARGO_TARGET_DIR=~/Developer/claude-bench/gap-target cargo build --release --locked -q -p base-scheduler-echo-benchmark 2>&1 | grep -E '^(error|warning)' -A7
cp ~/Developer/claude-bench/gap-target/release/base-scheduler-echo-benchmark /tmp/claude-scripts/bin/$1 && echo built $1
