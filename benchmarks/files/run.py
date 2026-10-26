#!/usr/bin/env python3
"""Run randomized matched pairs; build once with cargo build --release first."""
import argparse
import itertools
import json
import random
import subprocess
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("--binary", default="target/release/base-scheduler-file-benchmark")
parser.add_argument("--output", type=Path, required=True)
parser.add_argument("--repeats", type=int, default=7)
parser.add_argument("--iterations", type=int, default=50000)
parser.add_argument("--backends", nargs="+", default=["direct", "tokio", "native", "iouring"])
parser.add_argument("--affinity", action="store_true", help="Linux: pin one worker to CPU 2, four workers to CPUs 0-7")
args = parser.parse_args()
randomizer = random.Random(20261022)
pairs = list(itertools.product(range(args.repeats), args.backends, [4096, 65536], ["read-at", "write-at"], [(1, 1), (16, 4)]))
randomizer.shuffle(pairs)
args.output.parent.mkdir(parents=True, exist_ok=True)
with args.output.open("w") as output:
    for pair, (repeat, backend, size, workload, (tasks, workers)) in enumerate(pairs):
        modes = ["ordinary", "monitor"]
        randomizer.shuffle(modes)
        for mode in modes:
            command = [args.binary, backend, mode, workload, str(tasks), str(args.iterations), str(workers), str(size)]
            affinity = "2" if workers == 1 else "0-7"
            if args.affinity:
                command = ["taskset", "-c", affinity, *command]
            row = json.loads(subprocess.check_output(command, text=True, timeout=60))
            row.update(repeat=repeat, pair=pair, affinity=affinity if args.affinity else None)
            print(json.dumps(row), file=output, flush=True)
