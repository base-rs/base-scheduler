#!/usr/bin/env python3
"""Compare the concrete pool baseline with the generic pool contract in optimized binaries."""

import argparse
import hashlib
import itertools
import json
import random
import re
import subprocess
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--validation", action="store_true", help="Longer samples for noisy or slower first-pass cases")
    parser.add_argument("--match", help="Keep cases whose binary and arguments match this regular expression")
    args = parser.parse_args()
    directories = {"concrete": args.before.resolve(), "contract": args.after.resolve()}
    hashes = {(version, binary): hashlib.sha256((directory / binary).read_bytes()).hexdigest()
              for version, directory in directories.items()
              for binary in ["leases", "sockets", "files"]}
    cases = []
    for threads, sharing, ownership in itertools.product([1, 8], ["local", "shared"], ["mutable", "frozen"]):
        cases.append(("leases", [str(threads), str(1000000 if threads == 1 else 250000), sharing, ownership]))
    for backend, (workers, connections), (transport, size), mode in itertools.product(
        ["native", "tokio", "iouring"], [(1, 1), (2, 8)],
        [("tcp", 64), ("tcp", 65536), ("udp", 1200)], ["pool", "shared"],
    ):
        cases.append(("sockets", [backend, mode, str(size), str(max(2000, 20000 // connections)),
                                  str(workers), transport, "8", "receive", str(connections)]))
    for backend, size, mode in itertools.product(
        ["direct", "native", "tokio", "iouring"], [65536, 131072], ["join", "submit"],
    ):
        cases.append(("files", [backend, mode, str(size), str(64 * 1024 * 1024), "1"]))
    if args.validation:
        cases = [
            ("leases", ["1", "5000000", "local", "mutable"]),
            ("leases", ["8", "1000000", "local", "frozen"]),
            ("files", ["tokio", "join", "65536", str(512 * 1024 * 1024), "1"]),
            ("files", ["tokio", "submit", "65536", str(512 * 1024 * 1024), "1"]),
        ]
        for backend, mode, transport, size, workers, connections in [
            ("native", "shared", "tcp", 64, 2, 8),
            ("iouring", "pool", "udp", 1200, 2, 8),
            ("iouring", "shared", "udp", 1200, 2, 8),
            ("iouring", "pool", "tcp", 65536, 2, 8),
            ("native", "shared", "tcp", 64, 1, 1),
            ("tokio", "shared", "tcp", 64, 2, 8),
        ]:
            cases.append(("sockets", [backend, mode, str(size), "20000", str(workers),
                                      transport, "8", "receive", str(connections)]))
    if args.match:
        cases = [(binary, parameters) for binary, parameters in cases
                 if re.search(args.match, " ".join([binary, *parameters]))]
    randomizer = random.Random(648971)
    groups = list(itertools.product(range(args.repeats), cases))
    randomizer.shuffle(groups)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w") as output:
        for repeat, (binary, parameters) in groups:
            order = list(directories)
            randomizer.shuffle(order)
            for version in order:
                command = ["taskset", "-c", "2-9", str(directories[version] / binary), *parameters]
                result = subprocess.run(command, check=True, text=True, capture_output=True, timeout=120)
                row = json.loads(result.stdout)
                row.update(binary=binary, parameters=parameters, repeat=repeat, version=version,
                           binary_sha256=hashes[version, binary], cpus="2-9")
                output.write(json.dumps(row) + "\n")
                output.flush()


if __name__ == "__main__":
    main()
