#!/usr/bin/env python3
"""Measure fixed-buffer registration with identical pooled ownership."""

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
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--repeats", type=int, default=7)
    parser.add_argument("--cpus", default="2-5")
    parser.add_argument("--file-mib", type=int, default=256)
    parser.add_argument("--network-messages", type=int, default=20000)
    parser.add_argument("--validation", action="store_true",
                        help="Repeat representative gains, losses, and network cases with longer runs")
    args = parser.parse_args()
    binary = args.binary.resolve()
    identity = hashlib.sha256(binary.read_bytes()).hexdigest()
    args.directory.mkdir(parents=True, exist_ok=True)
    directory = args.directory.resolve()
    shapes = [(kind, direction, size) for kind, direction, size in
              itertools.product(["cached", "direct"], ["read", "write"], [4096, 65536, 131072])]
    shapes += [("tcp", "read", 64), ("tcp", "read", 65536), ("udp", "read", 1200)]
    configurations = [(kind, direction, size, depth) for (kind, direction, size), depth
                      in itertools.product(shapes, [1, 16])]
    if args.validation:
        configurations = [
            ("cached", "read", 65536, 16), ("cached", "write", 65536, 16),
            ("direct", "read", 65536, 1), ("direct", "read", 65536, 16),
            ("direct", "write", 65536, 1), ("direct", "write", 65536, 16),
            ("tcp", "read", 65536, 16), ("udp", "read", 1200, 1),
        ]
    groups = list(itertools.product(range(args.repeats), configurations, ["pooled", "retained"]))
    randomizer = random.Random(62147)
    randomizer.shuffle(groups)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w") as output:
        for repeat, (kind, direction, size, depth), ownership in groups:
            modes = ["ordinary", "fixed"]
            randomizer.shuffle(modes)
            iterations = (args.file_mib * 1024 * 1024 // size // depth
                          if kind in ["cached", "direct"] else max(2000, args.network_messages // depth))
            for mode in modes:
                command = [
                    "taskset", "-c", args.cpus, str(binary), mode, kind, direction,
                    str(size), str(depth), str(iterations), ownership, str(directory),
                ]
                result = subprocess.run(command, text=True, capture_output=True, timeout=120)
                if result.returncode:
                    raise RuntimeError(f"{command}: {result.stderr}")
                row = json.loads(result.stdout)
                row.update(repeat=repeat, binary_sha256=identity, cpus=args.cpus)
                output.write(json.dumps(row) + "\n")
                output.flush()


if __name__ == "__main__":
    main()
