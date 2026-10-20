#!/usr/bin/env python3
"""Compare optimized socket implementations against unchanged Tokio controls."""

import argparse
import datetime
import fcntl
import hashlib
import importlib.util
import itertools
import json
import os
from pathlib import Path
import platform
import random
import subprocess


spec = importlib.util.spec_from_file_location(
    "profile_runner", Path(__file__).resolve().parents[1] / "readiness-profiling" / "run.py",
)
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["before", "after", "client"]:
        parser.add_argument("--" + name, type=Path, required=True)
        parser.add_argument("--" + name + "-source", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--lock", type=Path, default=Path("/tmp/core-scheduler-perf.lock"))
    args = parser.parse_args()
    if args.repeats < 1:
        parser.error("repeats must be positive")
    for name in ["before", "after", "client", "output"]:
        setattr(args, name, getattr(args, name).resolve())
    args.output.mkdir(parents=True, exist_ok=False)
    args.privileged_perf = False
    # Builds/tests on this host take a shared lock; timed workloads take it exclusively.
    # This keeps experiments in separate worktrees from interfering with each other.
    with args.lock.open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        compare(args)


def compare(args):
    backends = ["tokio", "tokio-adapter", "tokio-adapter-poll", "iouring", "iouring-poll"]
    cases = [(1, 1, 64), (1, 1, 4096), (1, 32, 64), (1, 32, 4096),
             (4, 32, 64), (4, 32, 4096)]
    report = dict(
        created_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        sources={name: getattr(args, name + "_source") for name in ["before", "after", "client"]},
        binaries={name: dict(path=str(path), sha256=hashlib.sha256(path.read_bytes()).hexdigest())
                  for name in ["before", "after", "client"] for path in [getattr(args, name)]},
        platform=platform.platform(), hostname=platform.node(),
        rustc=subprocess.check_output(["rustc", "--version"], text=True).strip(),
        cargo=subprocess.check_output(["cargo", "--version"], text=True).strip(),
        cpu_topology=subprocess.check_output(["lscpu", "-e=CPU,CORE,SOCKET,NODE,CACHE"], text=True),
        cpu_model=subprocess.check_output(["lscpu"], text=True),
        build="cargo build --release --locked -p core-scheduler-echo-benchmark; CARGO_PROFILE_RELEASE_DEBUG=1",
        server_cpus="0 through workers-1", client_cpus="4-7", client_workers=4,
        warmup_seconds=1, measurement_seconds=3, seed=20261025, repeats=args.repeats,
        lock=str(args.lock), load_average_before=os.getloadavg(), samples=[],
    )
    rng = random.Random(report["seed"])
    for repeat in range(1, args.repeats + 1):
        combinations = list(itertools.product(cases, ["before", "after"], backends))
        rng.shuffle(combinations)
        for (workers, connections, size), variant, backend in combinations:
            args.server = getattr(args, variant)
            result = runner.sample(args, variant, backend, workers, connections, size, repeat)
            result["variant"] = variant
            report["samples"].append(result)
            errors = args.output / (result["identifier"] + ".stderr")
            if errors.stat().st_size == 0:
                errors.unlink()
            (args.output / "aiko.json").write_text(json.dumps(report, indent=2) + "\n")
            print(f"{len(report['samples']):3} {result['identifier']}: "
                  f"{result['requests_per_second']:,.0f} req/s", flush=True)
    report["load_average_after"] = os.getloadavg()
    (args.output / "aiko.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
