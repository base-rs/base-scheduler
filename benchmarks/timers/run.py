#!/usr/bin/env python3
"""Run the timer matrix in randomized, paired order; invoke under Aiko's performance flock."""
import argparse
import json
import random
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--binary", default="target/release/core-scheduler-timer-benchmark")
parser.add_argument("--output", required=True)
parser.add_argument("--repeats", type=int, default=7)
parser.add_argument("--backends", nargs="+", default=["tokio", "native", "iouring", "direct"])
args = parser.parse_args()
configs = [
    ("refresh", 1, 3_000_000, 1, 1000),
    ("yield", 1, 1_500_000, 1, 1000),
    ("yield", 64, 75_000, 4, 1000),
    ("expire", 1, 300, 1, 1000),
    ("expire", 128, 200, 4, 1000),
    ("cycle", 64, 200, 4, 1000),
    ("earlier", 1, 3_000_000, 1, 1000),
    ("fixed", 1, 3_000_000, 1, 1000),
]
random.seed(271828)
with open(args.output, "w") as output:
    for repeat in range(args.repeats):
        cases = [(config, backend) for config in configs for backend in args.backends]
        random.shuffle(cases)
        for (workload, tasks, iterations, workers, delay), backend in cases:
            modes = ["sleep", "reuse", "after"]
            random.shuffle(modes)
            for mode in modes:
                command = [args.binary, backend, mode, workload, str(tasks), str(iterations), str(workers), str(delay)]
                result = subprocess.run(command, capture_output=True, text=True, check=True, timeout=90)
                row = json.loads(result.stdout)
                row["repeat"] = repeat
                output.write(json.dumps(row) + "\n")
                output.flush()
        print(f"repeat {repeat + 1}/{args.repeats} complete", flush=True)
