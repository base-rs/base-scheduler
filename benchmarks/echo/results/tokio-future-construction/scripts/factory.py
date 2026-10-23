#!/usr/bin/env python3
"""Compare constructing the Tokio operation in its pinned storage with moving it."""

import fcntl
import hashlib
import itertools
import json
import os
from pathlib import Path
import random
import subprocess
import types

from sample import sample


root = Path.home() / ".cache/core-scheduler-echo-gap-factory"
root.mkdir(exist_ok=False)
checkouts = {name: Path.home() / f"Developer/core-rs/core-scheduler-echo-gap{suffix}" for name, suffix in [("baseline", ""), ("factory", "-factory")]}
client = Path.home() / "Developer/core-rs/core-scheduler-monoio-echo/target/release/core-scheduler-echo-benchmark"
report = dict(client_sha256=hashlib.sha256(client.read_bytes()).hexdigest(), builds={}, samples=[])
for name, checkout in checkouts.items():
    (root / name).mkdir()
    binary = checkout / "target/release/core-scheduler-echo-benchmark"
    patch = subprocess.check_output(["git", "diff"], cwd=checkout)
    (root / name / "change.patch").write_bytes(patch)
    report["builds"][name] = dict(source=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=checkout, text=True).strip(), binary=str(binary), binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), patch_sha256=hashlib.sha256(patch).hexdigest())
with open("/tmp/core-scheduler-perf.lock", "a") as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    report["load_average_before"] = os.getloadavg()
    cases = list(itertools.product(range(1, 4), checkouts, ["tokio", "tokio-adapter-monitor"], [64, 4096]))
    random.Random(20261024).shuffle(cases)
    for repeat, name, backend, size in cases:
        args = types.SimpleNamespace(server=Path(report["builds"][name]["binary"]), client=client, output=root / name, privileged_perf=True, perf_event="cycles")
        result = sample(args, "control", backend, 1, 32, size, repeat)
        report["samples"].append(dict(result, variant=name))
        (root / "aiko.json").write_text(json.dumps(report, indent=2) + "\n")
        print(f"{len(report['samples'])}/27 {name} {result['identifier']}: {result['requests_per_second']:,.0f} req/s", flush=True)
    args = types.SimpleNamespace(server=Path(report["builds"]["factory"]["binary"]), client=client, output=root / "factory", privileged_perf=True, perf_event="cycles")
    for mode, repeat in [("stat", 1), ("stat", 2), ("perf", 1)]:
        result = sample(args, mode, "tokio-adapter-monitor", 1, 32, 64, repeat)
        report["samples"].append(dict(result, variant="factory"))
        (root / "aiko.json").write_text(json.dumps(report, indent=2) + "\n")
        print(f"{len(report['samples'])}/27 factory {result['identifier']}", flush=True)
    report["load_average_after"] = os.getloadavg()
    (root / "aiko.json").write_text(json.dumps(report, indent=2) + "\n")
    with (root / "factory/profile.txt").open("w") as output:
        subprocess.run(["sudo", "-n", "perf", "report", "--stdio", "--header", "--no-children", "-g", "none", "--sort", "dso,symbol", "--percent-limit", "0.1", "-i", str(root / "factory/perf-tokio-adapter-monitor-w1-c32-b64-r1.data")], stdout=output, check=True)
