#!/usr/bin/env python3
"""Compare frozen server variants with the canonical validating echo client."""

import argparse
import hashlib
import importlib.util
import itertools
import json
from pathlib import Path
import random
from types import SimpleNamespace


ROOT = Path(__file__).resolve().parents[4]
spec = importlib.util.spec_from_file_location("echo_runner", ROOT / "benchmarks/echo/run.py")
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--variants", nargs="+", required=True, help="label:backend:commit:binary")
    parser.add_argument("--cases", nargs="+", default=["1/1/64", "1/32/64"])
    parser.add_argument("--client", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repeats", type=int, default=3)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output already exists")
    variants = [(label, backend, commit, Path(binary)) for label, backend, commit, binary in
                (value.split(":", 3) for value in args.variants)]
    cases = [tuple(map(int, value.split("/"))) for value in args.cases]
    sampling = SimpleNamespace(
        server_cpus=[0, 1, 2, 3], client_cpus=[4, 5, 6, 7], client_binary=args.client,
        client_workers=4, warmup=1, seconds=3,
    )
    report = {
        "variants": {label: {"backend": backend, "commit": commit, "path": str(binary),
                            "sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}
                     for label, backend, commit, binary in variants},
        "client": {"path": str(args.client), "sha256": hashlib.sha256(args.client.read_bytes()).hexdigest()},
        "cases": cases, "repeats": args.repeats, "seed": 20261025,
        "warmup": sampling.warmup, "seconds": sampling.seconds, "samples": [],
    }
    schedule = list(itertools.product(range(1, args.repeats + 1), cases, variants))
    random.Random(report["seed"]).shuffle(schedule)
    for index, (repeat, (workers, connections, size), (label, backend, _, binary)) in enumerate(schedule, 1):
        result = runner.sample(binary, backend, workers, connections, size, sampling)
        result.update(repeat=repeat, variant=label)
        report["samples"].append(result)
        args.output.write_text(json.dumps(report, indent=2) + "\n")
        print(index, len(schedule), label, workers, connections, size,
              round(result["requests_per_second"]), flush=True)


if __name__ == "__main__":
    main()
