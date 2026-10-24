#!/usr/bin/env python3
"""Compare ring task-work policies within one optimized echo executable."""

import argparse
import hashlib
import importlib.util
import itertools
import json
import os
from pathlib import Path
import random
import subprocess
from types import SimpleNamespace


ROOT = Path(__file__).resolve().parents[4]
spec = importlib.util.spec_from_file_location("echo_runner", ROOT / "benchmarks/echo/run.py")
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, required=True)
    parser.add_argument("--client", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--source-root", type=Path, default=ROOT)
    parser.add_argument("--compare-rearm", action="store_true")
    args = parser.parse_args()
    args.server, args.client = args.server.resolve(), args.client.resolve()
    if args.output.exists():
        parser.error("output already exists")
    sampling = SimpleNamespace(
        server_cpus=[0, 1, 2, 3], client_cpus=[4, 5, 6, 7],
        client_binary=args.client, client_workers=4, warmup=1, seconds=3,
    )
    report = {
        "source": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=args.source_root, text=True).strip(),
        "patch": subprocess.check_output(["git", "diff", "b70f46c", "--", "crates/iouring/src"], cwd=args.source_root, text=True),
        "binaries": {
            name: {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
            for name, path in [("server", args.server), ("client", args.client)]
        },
        "warmup": sampling.warmup, "seconds": sampling.seconds, "seed": 20261024,
        "samples": [],
        "compare_rearm": args.compare_rearm,
    }
    cases = list(itertools.product(
        range(1, 4), ["normal", "cooperative"],
        [False, True] if args.compare_rearm else [False],
        ["tokio", "iouring-monitor"] if args.compare_rearm else ["tokio", "iouring-monitor", "iouring"], [1, 32],
    ))
    random.Random(report["seed"]).shuffle(cases)
    for index, (repeat, policy, rearm, backend, connections) in enumerate(cases, 1):
        os.environ["CORE_SCHEDULER_TASKWORK"] = policy
        os.environ.pop("CORE_SCHEDULER_STREAM_REARM", None)
        if rearm:
            os.environ["CORE_SCHEDULER_STREAM_REARM"] = "1"
        result = runner.sample(args.server, backend, 1, connections, 64, sampling)
        result.update(repeat=repeat, policy=policy, rearm=rearm)
        report["samples"].append(result)
        args.output.write_text(json.dumps(report, indent=2) + "\n")
        print(index, len(cases), policy, rearm, backend, connections, round(result["requests_per_second"]), flush=True)


if __name__ == "__main__":
    main()
