#!/usr/bin/env python3
"""Compare the readiness fallback with worker-ring waits using a fixed client."""

import argparse
import hashlib
import importlib.util
import itertools
import json
import os
from pathlib import Path
import platform
import random
import subprocess

# Reuse the existing fixed-client process runner and CPU placement.
spec = importlib.util.spec_from_file_location(
    "performance", Path(__file__).resolve().parents[1] / "performance" / "run.py"
)
performance = importlib.util.module_from_spec(spec)
spec.loader.exec_module(performance)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--client", type=Path, required=True)
    parser.add_argument("--source", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repeats", type=int, default=5)
    args = parser.parse_args()
    if args.repeats < 1:
        parser.error("repeats must be positive")
    binaries = {name: getattr(args, name).resolve() for name in ["baseline", "candidate", "client"]}
    report = dict(
        candidate_source=args.source,
        baseline_source="7cd6af8725fedebc2dffd52473380d51d7d7fc5e",
        client_source="5736301fded46c4e5f11d5387950ae2ee0807b94",
        binaries={name: {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
                  for name, path in binaries.items()},
        platform=platform.platform(), hostname=platform.node(),
        rustc=subprocess.check_output(["rustc", "--version"], text=True).strip(),
        cpu_topology=subprocess.check_output(["lscpu", "-e=CPU,CORE,SOCKET,NODE,CACHE"], text=True),
        server_cpus="0 through workers-1", client_cpus="4-7", client_workers=4,
        seed=20261022, load_average_before=os.getloadavg(), samples=[],
    )
    cases = list(itertools.product(
        range(1, args.repeats + 1), [(1, 1), (1, 32), (4, 32)], [64, 4096],
        [("baseline", "iouring-poll"), ("candidate", "iouring-poll"),
         ("candidate", "iouring"), ("candidate", "tokio")],
    ))
    random.Random(report["seed"]).shuffle(cases)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    for repeat, (workers, connections), size, (variant, backend) in cases:
        result = performance.sample(
            binaries[variant], binaries["client"], backend, workers, connections, size
        )
        report["samples"].append(dict(result, variant=variant, repeat=repeat))
        args.output.write_text(json.dumps(report, indent=2) + "\n")
        print(f"{len(report['samples'])}/{len(cases)} {variant} {backend} "
              f"w{workers} c{connections} b{size} r{repeat}: "
              f"{result['requests_per_second']:,.0f} req/s", flush=True)
    report["load_average_after"] = os.getloadavg()
    args.output.write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
