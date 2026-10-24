#!/usr/bin/env python3
"""Count Native polls and ready-batch sizes; instrumented rates are diagnostic only."""

import fcntl
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[4]
BASE = "29d47375bf9a1b0239e9229335beb4d8beb0825b"
EXPERIMENT = Path.home() / ".cache/core-scheduler-single-worker-completions"
WORKER = ROOT / "crates/native/src/worker.rs"
REACTOR = ROOT / "crates/iouring/src/io_uring_selector.rs"


def replace_once(source, old, new):
    assert source.count(old) == 1, f"expected one replacement: {old[:80]!r}"
    return source.replace(old, new)


def main():
    originals = {path: subprocess.check_output(["git", "show", BASE + ":" + str(path.relative_to(ROOT))], cwd=ROOT, text=True) for path in [WORKER, REACTOR]}
    environment = os.environ | {"CARGO_PROFILE_RELEASE_DEBUG": "1", "CARGO_TARGET_DIR": str(EXPERIMENT / "target")}
    with open("/tmp/core-scheduler-perf.lock", "a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        assert all(path.read_text() == source for path, source in originals.items())
        try:
            for variant in ["baseline", "deferred"]:
                for path, source in originals.items():
                    path.write_text(source)
                if variant == "deferred":
                    subprocess.run(["git", "apply", str(EXPERIMENT / "deferred.patch")], cwd=ROOT, check=True)
                source = originals[WORKER]
                source = replace_once(source, "let mut iteration = 0usize;", "let mut iteration = 0usize; let mut batches = 0u64; let mut polls = 0u64; let mut waits = 0u64; let mut histogram = [0u64; 65];")
                source = replace_once(source, "run_ready_batch(&shared, runnable);", "let count = run_ready_batch(&shared, runnable); batches += 1; polls += count as u64; histogram[count.min(64)] += 1;")
                source = replace_once(source, "|| {\n                        drive(", "|| {\n                        waits += 1;\n                        drive(")
                source = replace_once(source, "CURRENT_WORKER.with(|current| current.take());", 'eprintln!("WORKER_METRICS worker={identifier} loops={iteration} batches={batches} polls={polls} waits={waits} histogram={histogram:?}");\n    CURRENT_WORKER.with(|current| current.take());')
                source = replace_once(source, "fn run_ready_batch(shared: &Shared, first: Runnable) {", "fn run_ready_batch(shared: &Shared, first: Runnable) -> usize { let mut count = 0;")
                source = replace_once(source, "while let Some(task) = runnable {", "while let Some(task) = runnable { count += 1;")
                source = replace_once(source, "    }\n}\n\n#[cfg(test)]\nmod tests;", "    }\n    count\n}\n\n#[cfg(test)]\nmod tests;")
                WORKER.write_text(source)
                subprocess.run(["cargo", "fmt", "--all"], cwd=ROOT, check=True)
                patch = subprocess.check_output(["git", "diff", "--", "crates/native/src/worker.rs", "crates/iouring/src/io_uring_selector.rs"], cwd=ROOT)
                (EXPERIMENT / (variant + "-diagnostic.patch")).write_bytes(patch)
                with (EXPERIMENT / "logs" / (variant + "-diagnostic-build.log")).open("w") as log:
                    subprocess.run(["cargo", "build", "--release", "--locked", "-p", "core-scheduler-echo-benchmark"], cwd=ROOT, env=environment, stdout=log, stderr=subprocess.STDOUT, check=True)
                shutil.copy2(EXPERIMENT / "target/release/core-scheduler-echo-benchmark", EXPERIMENT / "bin" / (variant + "-diagnostic"))
        finally:
            for path, source in originals.items():
                path.write_text(source)


if __name__ == "__main__":
    main()
