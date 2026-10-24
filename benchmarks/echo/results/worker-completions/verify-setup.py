#!/usr/bin/env python3
"""Trace setup and entry flags separately from performance measurements."""

import fcntl
import hashlib
import importlib.util
import json
from pathlib import Path
import shlex
import tempfile
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parents[4]
EXPERIMENT = Path.home() / ".cache/core-scheduler-single-worker-completions"
spec = importlib.util.spec_from_file_location("echo_runner", ROOT / "benchmarks/echo/run.py")
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def main():
    output = EXPERIMENT / "results/setup"
    output.mkdir(parents=True, exist_ok=False)
    report = {}
    settings = SimpleNamespace(server_cpus=[0], client_cpus=[4, 5, 6, 7], client_binary=EXPERIMENT / "bin/baseline", client_workers=4, warmup=0, seconds=0.02)
    with open("/tmp/core-scheduler-perf.lock", "a") as lock, tempfile.TemporaryDirectory() as directory:
        fcntl.flock(lock, fcntl.LOCK_EX)
        for name in ["baseline", "deferred", "deferred-no-iowait", "coop-no-iowait", "deferred-collect"]:
            binary = EXPERIMENT / "bin" / name
            wrapper = Path(directory) / name
            trace = output / (name + ".strace")
            wrapper.write_text("#!/bin/sh\nexec " + shlex.join(["strace", "-f", "-e", "io_uring_setup,io_uring_enter", "-o", str(trace), "--", str(binary)]) + ' "$@"\n')
            wrapper.chmod(0o700)
            sample = runner.sample(wrapper, "iouring-monitor", 1, 1, 64, settings)
            report[name] = {"sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "sample": sample}
            (output / "aiko.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
