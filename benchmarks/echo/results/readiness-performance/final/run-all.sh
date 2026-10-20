#!/usr/bin/env bash
set -euo pipefail
cd /home/samuel/Developer/core-rs/core-scheduler-perf-next-final
cache=/home/samuel/.cache/core-scheduler-next-final
flock -s /tmp/core-scheduler-perf.lock env CARGO_TARGET_DIR="$cache/target" cargo build --release --locked -p core-scheduler-echo-benchmark > "$cache/build-after.log" 2>&1
cp "$cache/target/release/core-scheduler-echo-benchmark" "$cache/after"
sha256sum "$cache/before" "$cache/after" > "$cache/binary-hashes.txt"
python3 "$cache/run.py" --output "$cache/final" --client /home/samuel/.cache/core-scheduler-owner-wake-final/client --repeats 3 \
 --case before,tokio,"$cache/before" --case after,tokio,"$cache/after" \
 --case before,tokio-adapter,"$cache/before" --case after,tokio-adapter,"$cache/after" \
 --case before,tokio-adapter-poll,"$cache/before" --case after,tokio-adapter-poll,"$cache/after" \
 --case before,iouring,"$cache/before" --case after,iouring,"$cache/after" \
 --case before,iouring-poll,"$cache/before" --case after,iouring-poll,"$cache/after" \
 --workload 1,1,64 --workload 1,32,64 --workload 1,32,4096 --workload 4,128,64 --workload 4,128,4096 > "$cache/final.log" 2>&1
coverage=/home/samuel/.cache/core-scheduler-next-final-coverage
mkdir -p "$coverage/tmp"
export CARGO_TARGET_DIR="$coverage/build"
export CARGO_LLVM_COV_TARGET_DIR="$coverage/coverage"
export TMPDIR="$coverage/tmp"
export LLVM_COV=/home/samuel/.cache/core-scheduler-perf-final-coverage/llvm-cov-system-exclude.py
export LLVM_PROFDATA=/usr/bin/llvm-profdata
flock -s /tmp/core-scheduler-perf.lock cargo bake --locked test:coverage --all-targets true --all-features true > "$cache/coverage.log" 2>&1
flock -s /tmp/core-scheduler-perf.lock cargo fmt --all -- --check > "$cache/fmt.log" 2>&1
flock -s /tmp/core-scheduler-perf.lock cargo clippy --workspace --all-targets --all-features --locked -- -D warnings > "$cache/clippy.log" 2>&1
RUSTDOCFLAGS=-Dwarnings flock -s /tmp/core-scheduler-perf.lock cargo doc --workspace --all-features --no-deps --locked > "$cache/docs.log" 2>&1
