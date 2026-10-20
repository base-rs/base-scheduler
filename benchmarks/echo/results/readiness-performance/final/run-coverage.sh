#!/usr/bin/env bash
set -euo pipefail
cd /home/samuel/Developer/core-rs/core-scheduler-perf-next-final
cache=/home/samuel/.cache/core-scheduler-next-final
coverage=/home/samuel/.cache/core-scheduler-next-final-coverage
export CARGO_TARGET_DIR="$coverage/build"
export CARGO_LLVM_COV_TARGET_DIR="$coverage/coverage"
export TMPDIR="$coverage/tmp"
export LLVM_COV=/home/samuel/.cache/core-scheduler-perf-final-coverage/llvm-cov-system-exclude.py
export LLVM_PROFDATA=/usr/bin/llvm-profdata
flock -s /tmp/core-scheduler-perf.lock cargo bake --locked test:coverage --all-targets true --all-features true > "$cache/coverage.log" 2>&1
flock -s /tmp/core-scheduler-perf.lock cargo fmt --all -- --check > "$cache/fmt.log" 2>&1
flock -s /tmp/core-scheduler-perf.lock cargo clippy --workspace --all-targets --all-features --locked -- -D warnings > "$cache/clippy.log" 2>&1
RUSTDOCFLAGS=-Dwarnings flock -s /tmp/core-scheduler-perf.lock cargo doc --workspace --all-features --no-deps --locked > "$cache/docs.log" 2>&1
