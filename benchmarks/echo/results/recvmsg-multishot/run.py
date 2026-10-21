#!/usr/bin/env python3
"""Matched multishot echo samples. Run exclusively under /tmp/core-scheduler-perf.lock on Aiko."""

import argparse
import datetime
import hashlib
import importlib.util
import itertools
import json
import os
from pathlib import Path
import random
import statistics
import subprocess

ROOT = Path(__file__).resolve().parents[4]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--seconds", type=float, default=2)
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location("echo", ROOT / "benchmarks/echo/run.py")
    echo = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(echo)
    subprocess.run(["cargo", "build", "--release", "--locked", "-p", "core-scheduler-echo-benchmark"], cwd=ROOT, check=True)
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")).expanduser()
    binary = target / "release/core-scheduler-echo-benchmark"
    config = argparse.Namespace(server_cpus=[0, 1, 2, 3], client_cpus=[4, 5, 6, 7], client_workers=4,
                                client_binary=binary, warmup=0.5, seconds=args.seconds)
    def output(*command):
        return subprocess.check_output(command, cwd=ROOT, text=True).strip()
    report = {
        "created_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "commit": output("git", "rev-parse", "HEAD"),
        "dirty": output("git", "status", "--porcelain"),
        "diff_sha256": hashlib.sha256(output("git", "diff", "HEAD").encode()).hexdigest(),
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "rustc": output("rustc", "-Vv"), "kernel": output("uname", "-a"),
        "topology": output("lscpu", "-e=CPU,CORE,SOCKET,NODE,CACHE"),
        "cpu": output("lscpu"), "load_before": os.getloadavg(),
        "warmup_seconds": config.warmup, "seconds": config.seconds,
        "repeats": args.repeats, "random_seed": 20261021,
        "build_environment": {name: os.environ.get(name) for name in
                              ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_TARGET_DIR"]},
        "server_cpus": config.server_cpus, "client_cpus": config.client_cpus,
        "samples": [],
    }
    cases = list(itertools.product([1, 4], [1, 32], [64, 4096]))
    rng = random.Random(20261021)
    for repeat in range(args.repeats):
        rng.shuffle(cases)
        for workers, connections, size in cases:
            backends = ["tokio", "iouring-poll", "iouring-multishot"]
            rng.shuffle(backends)
            for backend in backends:
                row = dict(echo.sample(binary, backend, workers, connections, size, config), repeat=repeat + 1)
                report["samples"].append(row)
                args.output.parent.mkdir(parents=True, exist_ok=True)
                args.output.write_text(json.dumps(report, indent=2) + "\n")
                print(f"{len(report['samples']):3} {backend:18} w={workers} c={connections} b={size}: {row['requests_per_second']:.0f} req/s", flush=True)
    report["load_after"] = os.getloadavg()
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    for case in sorted(cases):
        values = {}
        for backend in ["tokio", "iouring-poll", "iouring-multishot"]:
            selected = [r["requests_per_second"] for r in report["samples"]
                        if (r["workers"], r["connections"], r["bytes"]) == case and r["backend"] == backend]
            values[backend] = statistics.median(selected)
        print(case, values, "multishot vs Tokio:", f"{100 * (values['iouring-multishot'] / values['tokio'] - 1):+.2f}%", flush=True)


if __name__ == "__main__":
    main()
