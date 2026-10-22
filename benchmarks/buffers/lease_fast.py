#!/usr/bin/env python3
"""Compare portable pool implementations with randomized matched samples."""

import argparse
import hashlib
import itertools
import json
import random
import subprocess
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repeats", type=int, default=7)
    args = parser.parse_args()
    paths = {"mutex": args.before.resolve(), "bitmap": args.after.resolve()}
    hashes = {label: hashlib.sha256(path.read_bytes()).hexdigest()
              for label, path in paths.items()}
    randomizer = random.Random(52161)
    groups = list(itertools.product(range(args.repeats), [1, 2, 8],
                                   ["shared", "local"], ["mutable", "frozen"]))
    randomizer.shuffle(groups)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w") as output:
        for repeat, threads, pool, mode in groups:
            order = list(paths)
            randomizer.shuffle(order)
            for implementation in order:
                result = subprocess.run([
                    "taskset", "-c", "2-9", str(paths[implementation]),
                    str(threads), "1000000", pool, mode,
                ], check=True, text=True, capture_output=True, timeout=60)
                row = json.loads(result.stdout)
                row.update(repeat=repeat, implementation=implementation,
                           binary_sha256=hashes[implementation], cpus="2-9")
                output.write(json.dumps(row) + "\n")
                output.flush()


if __name__ == "__main__":
    main()
