#!/usr/bin/env python3
"""Compare shared-budget receives, worker-owned receives, and the Tokio adapter."""

import argparse
import hashlib
import itertools
import json
import random
import subprocess
import sys
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--candidate-revision", required=True)
    parser.add_argument("--repeats", type=int, default=7)
    parser.add_argument("--iterations", type=int, default=10000)
    parser.add_argument("--cpus", default="2-9")
    args = parser.parse_args()
    binaries = {"before": args.baseline.resolve() / "sockets", "after": args.candidate.resolve() / "sockets"}
    hashes = {key: hashlib.sha256(path.read_bytes()).hexdigest() for key, path in binaries.items()}
    randomizer = random.Random(612704)
    cases = list(itertools.product(range(args.repeats), [(1, 1), (2, 8)],
                                  [("tcp", 64), ("tcp", 65536), ("udp", 1200)], ["pool", "shared"]))
    randomizer.shuffle(cases)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w") as output:
        for index, (repeat, (workers, connections), (transport, size), mode) in enumerate(cases):
            strategies = [("before", "iouring", "total", "shared-provided"),
                          ("after", "iouring", "total", "shared-monitor"),
                          ("after", "iouring", "worker-equal", "worker-equal"),
                          ("after", "tokio", "total", "tokio")]
            if workers > 1:
                strategies.append(("after", "iouring", "worker-scaled", "worker-scaled"))
            randomizer.shuffle(strategies)
            for version, backend, layout, strategy in strategies:
                parameters = [backend, mode, str(size), str(args.iterations), str(workers), transport,
                              "8", "receive", str(connections)]
                if version == "after":
                    parameters.append(layout)
                command = ["taskset", "-c", args.cpus, str(binaries[version]), *parameters]
                result = subprocess.run(command, text=True, capture_output=True, timeout=120)
                if result.returncode:
                    raise RuntimeError(f"{command!r} exited {result.returncode}:\n{result.stderr}")
                row = json.loads(result.stdout)
                row.update(strategy=strategy, repeat=repeat, parameters=parameters, cpus=args.cpus,
                           binary_sha256=hashes[version],
                           revision="fd47abe0c0ba939e64ad1538cbc50e343064897c" if version == "before" else args.candidate_revision,
                           payload_budget=8 * connections * size * (workers if layout == "worker-scaled" else 1))
                output.write(json.dumps(row) + "\n")
                output.flush()
            print(f"Completed {index + 1}/{len(cases)} matched cases.", file=sys.stderr, flush=True)


if __name__ == "__main__":
    main()
