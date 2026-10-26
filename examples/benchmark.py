#!/usr/bin/env python3
"""Build once and measure the canonical recipes in shuffled, reproducible order."""

import argparse
import datetime
import hashlib
import itertools
import json
import os
from pathlib import Path
import platform
import random
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.repeats < 1:
        parser.error("repeats must be positive")
    root = Path(__file__).resolve().parent.parent

    def command(*arguments):
        return subprocess.check_output(arguments, cwd=root, text=True).strip()

    artifacts = command("cargo", "build", "--release", "--locked", "-p", "base-scheduler-examples", "--message-format=json")
    binary = next(Path(artifact["executable"]) for line in artifacts.splitlines()
                  if (artifact := json.loads(line)).get("executable")
                  and artifact["target"]["name"] == "base-scheduler-examples")
    metadata = {
        "timestamp": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "platform": platform.platform(),
        "cpu": platform.processor(),
        "logical_cpus": os.cpu_count(),
        "rustc": command("rustc", "--version", "--verbose"),
        "revision": command("git", "rev-parse", "HEAD"),
        "status": command("git", "status", "--porcelain"),
        "lock_sha256": hashlib.sha256((root / "Cargo.lock").read_bytes()).hexdigest(),
        "rustflags": os.environ.get("RUSTFLAGS", ""),
        "repeats": args.repeats,
        "seed": 0,
    }
    backends = ["direct", "tokio", "native"]
    if platform.system() == "Linux":
        backends.append("iouring")
    workloads = [("stream", 65536, 4096), ("frames", 4093, 32768), ("datagrams", 1200, 16384), ("files", 65536, 4096)]
    generator = random.Random(0)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.with_suffix(".metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    with args.output.open("w") as output:
        for repetition in range(args.repeats):
            cases = list(itertools.product(backends, workloads, [1, 4]))
            generator.shuffle(cases)
            for backend, (recipe, size, count), workers in cases:
                sample = json.loads(subprocess.check_output([str(binary), backend, recipe, str(size), str(count), str(workers)], cwd=root, text=True, timeout=120))
                sample["repetition"] = repetition
                output.write(json.dumps(sample) + "\n")
                output.flush()
                print(f"{repetition + 1}/{args.repeats} {backend:7} {recipe:9} {workers} workers: {sample['mib_per_second']:.1f} MiB/s", flush=True)


if __name__ == "__main__":
    main()
