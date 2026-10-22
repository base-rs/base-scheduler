#!/usr/bin/env python3
"""Compare syscall-first and multishot receives with matched randomized samples."""

import argparse
import hashlib
import itertools
import json
import random
import subprocess
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repeats", type=int, default=9)
    parser.add_argument("--iterations", type=int, default=10000)
    parser.add_argument("--cpus", default="2-5")
    args = parser.parse_args()
    binaries = {"after": args.binary.resolve()}
    if args.baseline:
        binaries["before"] = args.baseline.resolve()
    identities = {
        label: hashlib.sha256(path.read_bytes()).hexdigest()
        for label, path in binaries.items()
    }
    randomizer = random.Random(57932)
    groups = list(itertools.product(
        range(args.repeats), [1, 2], ["echo", "receive"],
        [("udp", 1200, 1), ("udp", 1200, 16), ("tcp", 64, 1),
         ("tcp", 64, 16), ("tcp", 65536, 1), ("tcp", 65536, 4)],
    ))
    randomizer.shuffle(groups)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w") as output:
        for repeat, workers, traffic, (transport, size, window) in groups:
            cases = [("after", "iouring", mode)
                     for mode in ["raw", "pool", "shared", "multi", "multi-retain"]]
            cases += [("after", "tokio", mode) for mode in ["raw", "pool"]]
            if args.baseline:
                cases += [("before", "iouring", mode)
                          for mode in ["pool", "shared", "multi", "multi-retain"]]
            randomizer.shuffle(cases)
            for label, backend, mode in cases:
                command = [
                    "taskset", "-c", args.cpus, str(binaries[label]), backend, mode,
                    str(size), str(args.iterations), str(workers), transport,
                    str(window), traffic,
                ]
                result = subprocess.run(command, text=True, capture_output=True,
                                        timeout=60, check=True)
                row = json.loads(result.stdout)
                row.update(repeat=repeat, implementation=label, cpus=args.cpus,
                           binary_sha256=identities[label])
                output.write(json.dumps(row) + "\n")
                output.flush()


if __name__ == "__main__":
    main()
