#!/usr/bin/env python3
"""Run randomized matched samples without concurrent builds or benchmarks."""

import argparse
import itertools
import json
import random
import subprocess
from pathlib import Path


def groups(repeats):
    randomizer = random.Random(22361)
    cases = []
    for backend, workers, size in itertools.product(
        ["direct", "native", "tokio", "iouring"], [1, 2], [4096, 65536, 131072]
    ):
        for repeat in range(repeats):
            modes = ["vec-serial", "vec-join", "serial", "join", "submit"]
            randomizer.shuffle(modes)
            cases.append([
                ("files", backend, mode, size, workers, repeat, None)
                for mode in modes
            ])
    for backend, workers, (transport, size) in itertools.product(
        ["native", "tokio", "iouring"],
        [1, 2],
        [("tcp", 64), ("tcp", 65536), ("udp", 1200)],
    ):
        for repeat in range(repeats):
            modes = ["raw", "pool", "shared"]
            randomizer.shuffle(modes)
            cases.append([
                ("sockets", backend, mode, size, workers, repeat, transport)
                for mode in modes
            ])
    randomizer.shuffle(cases)
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repeats", type=int, default=7)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cpus", default="2-5")
    parser.add_argument("--source", required=True, help="Git commit used to build the binaries")
    arguments = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    with arguments.output.open("w") as output:
        for group in groups(arguments.repeats):
            for binary, backend, mode, size, workers, repeat, transport in group:
                amount = 64 * 1024 * 1024 if binary == "files" else 10000
                command = [
                    "taskset", "-c", arguments.cpus,
                    str(root / "target/release" / binary), backend, mode,
                    str(size), str(amount), str(workers),
                ]
                if transport is not None:
                    command.append(transport)
                result = subprocess.run(
                    command, text=True, capture_output=True, timeout=45, check=True
                )
                row = json.loads(result.stdout)
                row.update(
                    binary=binary, repeat=repeat, cpus=arguments.cpus,
                    source=arguments.source,
                )
                output.write(json.dumps(row) + "\n")
                output.flush()


if __name__ == "__main__":
    main()
