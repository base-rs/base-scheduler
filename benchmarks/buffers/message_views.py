#!/usr/bin/env python3
"""Compare received-message accessors with the previous result records."""

import argparse
import hashlib
import itertools
import json
from pathlib import Path
import random
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repeats", type=int, default=7)
    parser.add_argument("--iterations", type=int, default=3000)
    parser.add_argument("--cpus", default="2-5")
    parser.add_argument(
        "--backends", nargs="+", choices=("tokio", "iouring"),
        default=("tokio", "iouring"),
    )
    parser.add_argument("--workers", nargs="+", type=int, default=(1, 2))
    workloads = {
        "tcp-small": ("tcp", 64, 1),
        "tcp-burst": ("tcp", 64, 16),
        "tcp-large": ("tcp", 65536, 4),
        "udp": ("udp", 1200, 16),
    }
    parser.add_argument(
        "--workloads", nargs="+", choices=workloads, default=tuple(workloads),
    )
    args = parser.parse_args()
    binaries = {"before": args.before.resolve(), "after": args.after.resolve()}
    hashes = {
        label: hashlib.sha256(path.read_bytes()).hexdigest()
        for label, path in binaries.items()
    }
    cases = list(itertools.product(
        range(args.repeats), args.backends, args.workers, ("raw", "pool", "multi", "provided"),
        (workloads[name] for name in args.workloads),
    ))
    randomizer = random.Random(232810)
    randomizer.shuffle(cases)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x") as output:
        for repeat, backend, workers, mode, (transport, size, window) in cases:
            if mode == "provided" and backend != "iouring":
                continue
            order = list(binaries)
            randomizer.shuffle(order)
            for implementation in order:
                command = [
                    "taskset", "-c", args.cpus, str(binaries[implementation]),
                    backend, "pool" if mode == "provided" else mode, str(size), str(args.iterations), str(workers),
                    transport, str(window), "receive", "1",
                    "worker-equal" if mode == "provided" else "total",
                ]
                result = subprocess.run(
                    command, text=True, capture_output=True, timeout=45, check=True,
                )
                row = json.loads(result.stdout)
                row.update(
                    repeat=repeat, implementation=implementation, receive_path=mode,
                    binary_sha256=hashes[implementation], cpus=args.cpus,
                )
                output.write(json.dumps(row) + "\n")
                output.flush()


if __name__ == "__main__":
    main()
