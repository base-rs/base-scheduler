#!/bin/bash
# usage: hbuild.sh VARIANT  -> /tmp/claude-scripts/bin/hello-VARIANT
set -e
v=$1; s=~/Developer/claude-bench/variants/sched-$v
cd ~/Developer/claude-bench/base-http/benchmarks/hello
sed -i 's/features = \["rt", "net"/features = ["rt", "rt-multi-thread", "net"/' Cargo.toml
args=()
for c in base-scheduler:. base-scheduler-direct:crates/direct base-scheduler-native:crates/native base-scheduler-selector:crates/selector base-scheduler-tokio:crates/tokio base-scheduler-iouring:crates/iouring base-scheduler-timers:crates/timers; do
  args+=(--config "patch.crates-io.${c%%:*}.path=\"$s/${c#*:}\"")
done
CARGO_TARGET_DIR=target-$v cargo build -q --release "${args[@]}" 2>&1 | grep -v '^warning: Patch' | tail -5
mkdir -p /tmp/claude-scripts/bin && cp target-$v/release/base-http-hello-benchmark /tmp/claude-scripts/bin/hello-$v
