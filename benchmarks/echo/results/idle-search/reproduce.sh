#!/usr/bin/env bash
set -euo pipefail

if (( $# < 2 || $# > 3 )); then
    echo "Usage: $0 DISPOSABLE_CHECKOUT OUTPUT_DIRECTORY [FROZEN_CLIENT]" >&2
    exit 2
fi

base=e3b67a578cf8b7e57b0b3e8623a0f40439fe7c33
checkout="$(cd "$1" && pwd)"
harness="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$2"
experiment="$(cd "$2" && pwd)"
client="${3:-$experiment/bin/current}"
if [[ "$(git -C "$checkout" rev-parse HEAD)" != "$base" ]]; then
    echo "Use a disposable worktree at $base; this script changes its Native worker sources." >&2
    exit 2
fi
if [[ -n "$(git -C "$checkout" status --porcelain)" ]]; then
    echo "The disposable worktree must be clean before reproduction." >&2
    exit 2
fi
if [[ -e "$experiment/confirm.json" ]]; then
    echo "Use a new output directory." >&2
    exit 2
fi

exec 9>/tmp/core-scheduler-perf.lock
flock -x 9
export CARGO_TARGET_DIR="$experiment/target"
export CARGO_PROFILE_RELEASE_DEBUG=1
mkdir -p "$experiment/bin" "$experiment/logs"
cd "$checkout"

for variant in current no-steal empty quiescent; do
    git restore --source "$base" -- crates/native/src/worker.rs crates/native/src/task_service.rs crates/native/src/worker/tests.rs
    git apply "$harness/prototypes/$variant.patch"
    cp "$harness/stealing-benchmark.rs" crates/native/src/worker/stealing_benchmark.rs
    cargo fmt --all
    cargo clean --release -p core-scheduler-native -p core-scheduler-iouring -p core-scheduler-echo-benchmark \
        > "$experiment/logs/$variant-clean.log" 2>&1
    cargo build --release --locked -p core-scheduler-echo-benchmark \
        > "$experiment/logs/$variant-build.log" 2>&1
    cp "$CARGO_TARGET_DIR/release/core-scheduler-echo-benchmark" "$experiment/bin/$variant"
    cargo test --release --locked -p core-scheduler-native --lib --no-run --message-format=json \
        > "$experiment/logs/$variant-test-build.json" 2> "$experiment/logs/$variant-test-build.log"
    python3 - "$experiment/logs/$variant-test-build.json" "$experiment/bin/test-$variant" <<'PY'
import json
from pathlib import Path
import shutil
import sys

artifacts = [json.loads(line) for line in Path(sys.argv[1]).read_text().splitlines()]
executables = [artifact['executable'] for artifact in artifacts
    if artifact.get('reason') == 'compiler-artifact'
    and artifact.get('executable')
    and artifact['target']['name'] == 'core_scheduler_native'
    and artifact['profile']['test']]
assert len(executables) == 1, executables
shutil.copy2(executables[0], sys.argv[2])
PY
done

python3 "$harness/stealing-echo.py" \
    --variant "current=$experiment/bin/current" \
    --variant "no-steal=$experiment/bin/no-steal" \
    --variant "quiescent=$experiment/bin/quiescent" \
    --tokio "$experiment/bin/current" --client "$client" --root "$checkout" \
    --repeats 3 --output "$experiment/confirm.json" | tee "$experiment/confirm.log"

for workers in "4 16 32" "32"; do
    repeats=3
    name=confirm-burst
    if [[ "$workers" == 32 ]]; then
        repeats=10
        name=many-workers-burst
    fi
    read -ra worker_counts <<< "$workers"
    python3 "$harness/stealing-burst.py" \
        --variant "current=$experiment/bin/test-current" \
        --variant "no-steal=$experiment/bin/test-no-steal" \
        --variant "quiescent=$experiment/bin/test-quiescent" \
        --workers "${worker_counts[@]}" --repeats "$repeats" \
        --output "$experiment/$name.json" | tee "$experiment/$name.log"
done

python3 "$harness/wait-affinity.py" \
    --variant "current=iouring-monitor=$experiment/bin/current" \
    --variant "no-steal=iouring-monitor=$experiment/bin/no-steal" \
    --variant "empty=iouring-monitor=$experiment/bin/empty" \
    --variant "quiescent=iouring-monitor=$experiment/bin/quiescent" \
    --variant "tokio=tokio=$experiment/bin/current" \
    --client "$client" --cases 4:1:64 4:32:64 --repeats 3 --counters cpu \
    --output "$experiment/cpu.json" | tee "$experiment/cpu.log"

python3 "$harness/wait-affinity.py" \
    --variant "current=iouring-monitor=$experiment/bin/current" \
    --variant "no-steal=iouring-monitor=$experiment/bin/no-steal" \
    --variant "quiescent=iouring-monitor=$experiment/bin/quiescent" \
    --client "$client" --cases 4:1:64 --repeats 3 --counters work \
    --output "$experiment/work.json" | tee "$experiment/work.log"
