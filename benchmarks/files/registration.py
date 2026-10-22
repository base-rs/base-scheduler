#!/usr/bin/env python3
"""Compare fixed-file caching against the same implementation with registration disabled."""
import argparse
import itertools
import json
import random
import subprocess
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("--binary", default="target/release/core-scheduler-file-benchmark")
parser.add_argument("--baseline-binary", help="Optional executable from the original FileMonitor prototype")
parser.add_argument("--output", type=Path, required=True)
parser.add_argument("--repeats", type=int, default=7)
parser.add_argument("--iterations", type=int, default=50000)
args = parser.parse_args()
randomizer = random.Random(20261022)
cases = list(itertools.product(range(args.repeats), [4096, 65536], ["read-at", "write-at"], [("local", 1, 1), ("local", 16, 4), ("external", 1, 1), ("external", 1, 2)]))
randomizer.shuffle(cases)
args.output.parent.mkdir(parents=True, exist_ok=True)
with args.output.open("w") as output:
    for pair, (repeat, size, workload, (routing, tasks, workers)) in enumerate(cases):
        variants = ["unregistered", "registered"]
        if routing == "local" and args.baseline_binary:
            variants.append("original")
        randomizer.shuffle(variants)
        for variant in variants:
            backend = "iouring-unregistered" if variant == "unregistered" else "iouring"
            if routing == "external":
                backend += "-migration"
            binary = args.baseline_binary if variant == "original" else args.binary
            affinity = "2" if workers == 1 else "0-7"
            command = ["taskset", "-c", affinity, binary, backend, "monitor", workload, str(tasks), str(args.iterations), str(workers), str(size)]
            row = json.loads(subprocess.check_output(command, text=True, timeout=60))
            row.update(repeat=repeat, pair=pair, variant=variant, routing=routing, affinity=affinity)
            print(json.dumps(row), file=output, flush=True)
