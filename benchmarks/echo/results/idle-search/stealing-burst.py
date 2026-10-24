#!/usr/bin/env python3
"""Measure controlled uneven bursts using frozen Native unit-test executables."""

import argparse
import hashlib
import itertools
import json
import os
from pathlib import Path
import platform
import random
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--variant", action="append", required=True, help="NAME=TEST_BINARY")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--workers", nargs="+", type=int, default=[4, 16, 48])
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--rounds", type=int, default=11)
    parser.add_argument("--tasks", type=int, default=128)
    parser.add_argument("--work-us", type=int, default=100)
    parser.add_argument("--busy-us", type=int, default=20_000)
    args = parser.parse_args()
    if args.output.exists() or min(args.workers + [args.tasks, args.repeats]) < 1 or args.rounds < 2:
        parser.error("use a new output, positive counts, and at least two rounds")
    variants = dict(item.split("=", 1) for item in args.variant)
    if len(variants) != len(args.variant):
        parser.error("duplicate variant name")
    report = {
        "platform": platform.platform(), "hostname": platform.node(),
        "rustc": subprocess.check_output(["rustc", "-Vv"], text=True),
        "settings": vars(args) | {"output": str(args.output)},
        "variants": {name: {"binary": str(Path(binary).resolve()), "sha256": hashlib.sha256(Path(binary).read_bytes()).hexdigest()} for name, binary in variants.items()},
        "seed": 20261025, "samples": [],
        "method": "Private diagnostic reroutes suspended tasks to the long-poll worker before external oneshot wakes. Per-process round zero is warmup. No selector, timer, or I/O callback publication occurs in this fixture.",
    }
    hashes = [variant["sha256"] for variant in report["variants"].values()]
    if len(set(hashes)) != len(hashes):
        parser.error("duplicate test executable hash")
    jobs = list(itertools.product(range(args.repeats), args.workers, variants))
    random.Random(report["seed"]).shuffle(jobs)
    logs = args.output.with_suffix("")
    logs.mkdir()
    for repeat, workers, name in jobs:
        environment = os.environ | {
            "STEAL_WORKERS": str(workers), "STEAL_TASKS": str(args.tasks),
            "STEAL_WORK_US": str(args.work_us), "STEAL_BUSY_US": str(args.busy_us),
            "STEAL_ROUNDS": str(args.rounds),
        }
        command = ["taskset", "-c", f"0-{workers - 1}", str(Path(variants[name]).resolve()),
            "worker::stealing_benchmark::uneven_burst", "--ignored", "--exact", "--nocapture"]
        result = subprocess.run(command, env=environment, text=True, capture_output=True, timeout=60, check=True)
        (logs / f"{name}-w{workers}-r{repeat}.log").write_text(result.stdout + result.stderr)
        samples = [json.loads(line.removeprefix("STEAL_RESULT ")) for line in result.stdout.splitlines() if line.startswith("STEAL_RESULT ")]
        assert len(samples) == args.rounds
        for sample in samples:
            assert sample["workers"] == workers and sample["tasks"] == args.tasks
            assert sum(sample["per_worker"]) == args.tasks
            assert len(sample["start_ns"]) == len(sample["finish_ns"]) == args.tasks
            assert min(sample["start_ns"]) >= 0 and max(sample["start_ns"]) <= max(sample["finish_ns"])
            sample.update(variant=name, repeat=repeat, warmup=sample["round"] == 0)
            report["samples"].append(sample)
        args.output.write_text(json.dumps(report, indent=2) + "\n")
        print(name, workers, repeat, "rounds", len(samples), flush=True)


if __name__ == "__main__":
    main()
