#!/usr/bin/env python3
"""Compare preserved release binaries before/after hiding buffer policies.

Run on Aiko under flock /tmp/core-scheduler-perf.lock. Arguments: result
directory, baseline executable, candidate executable. Output is exclusive so a
rerun cannot silently append duplicate matched samples.
"""

import hashlib
import json
import os
from pathlib import Path
import random
import subprocess
import sys


out = Path(sys.argv[1])
binaries = {"baseline": Path(sys.argv[2]).resolve(), "candidate": Path(sys.argv[3]).resolve()}
hashes = {name: hashlib.sha256(path.read_bytes()).hexdigest() for name, path in binaries.items()}
assert os.environ.get("PERF_LOCK_HELD"), "Hold the shared benchmark lock for the entire run"
seed = "portable-buffer-policy-20261022"
cases = [(transport, size, workers, layout)
         for transport, size in [("tcp", 65536), ("udp", 8192)]
         for workers in [1, 4]
         for layout in ["total", "worker-equal"]]
with (out / "samples.jsonl").open("x") as result:
    for repeat in range(1, 8):
        order = list(cases)
        rng = random.Random(f"{seed}-{repeat}")
        rng.shuffle(order)
        for transport, size, workers, layout in order:
            pair = list(binaries)
            rng.shuffle(pair)
            for variant in pair:
                args = ["taskset", "-c", "2-15", str(binaries[variant]), "iouring", "pool",
                        str(size), "10000", str(workers), transport, "8", "receive", "8", layout]
                run = subprocess.run(args, capture_output=True, text=True, check=True, timeout=60)
                row = json.loads(run.stdout)
                assert row["bytes_received"] == 8 * 10000 * 8 * size
                assert row["pool_capacity"] == 64
                assert row["seconds"] > 0 and row["messages_per_second"] > 0
                row.update(variant=variant, repeat=repeat, seed=seed, args=args,
                           binary_sha256=hashes[variant], diagnostics=run.stderr)
                result.write(json.dumps(row) + "\n")
                result.flush()
                print(repeat, transport, size, workers, layout, variant,
                      round(row["messages_per_second"]), flush=True)
