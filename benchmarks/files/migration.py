#!/usr/bin/env python3
"""Compare retained registrations with invalidation during controlled worker migration."""
import argparse
import itertools
import json
import random
import subprocess
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("--retain", required=True)
parser.add_argument("--invalidate", required=True)
parser.add_argument("--output", required=True, type=Path)
parser.add_argument("--repeats", type=int, default=7)
parser.add_argument("--workers", type=int, nargs="+", default=[1, 2, 48])
parser.add_argument("--bursts", type=int, nargs="+", default=[1, 16, 256, 4096])
parser.add_argument("--sizes", type=int, nargs="+", default=[4096, 65536])
parser.add_argument("--workloads", nargs="+", default=["read", "write"])
parser.add_argument("--operations", type=int, default=65536)
args = parser.parse_args()
randomizer = random.Random(20261022)
cases = list(itertools.product(range(args.repeats), args.workers, args.bursts, args.sizes, args.workloads))
randomizer.shuffle(cases)
args.output.parent.mkdir(parents=True, exist_ok=True)
with args.output.open("w") as output:
    for pair, (repeat, workers, burst, size, workload) in enumerate(cases):
        rounds = max(64, min(8192, (args.operations + burst - 1) // burst))
        variants = [("retain", args.retain), ("invalidate", args.invalidate)]
        randomizer.shuffle(variants)
        for variant, binary in variants:
            affinity = "0-7" if workers <= 2 else "0-31"
            row = json.loads(subprocess.check_output(
                ["taskset", "-c", affinity, binary, str(workers), str(burst), str(rounds), str(size), workload],
                text=True, timeout=60,
            ))
            row.update(pair=pair, repeat=repeat, variant=variant, affinity=affinity)
            print(json.dumps(row), file=output, flush=True)
