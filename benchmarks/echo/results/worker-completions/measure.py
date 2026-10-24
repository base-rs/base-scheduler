#!/usr/bin/env python3
"""Measure frozen echo variants, keeping profiles separate from throughput controls."""

import argparse
import fcntl
import hashlib
import importlib.util
import itertools
import json
from pathlib import Path
import platform
import random
import statistics
import subprocess
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parents[4]
HELPER = ROOT / "benchmarks/echo/results/single-worker-research/scripts/sample-runner.py"
spec = importlib.util.spec_from_file_location("sample_runner", HELPER)
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--variant", action="append", required=True, help="NAME=BACKEND=BINARY")
    parser.add_argument("--client", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--mode", choices=["control", "stat", "perf"], default="control")
    parser.add_argument("--cases", nargs="+", default=["1:32:64", "1:32:4096"])
    parser.add_argument("--repeats", type=int, default=3)
    args = parser.parse_args()
    variants = {}
    for value in args.variant:
        name, backend, binary = value.split("=", 2)
        if name in variants:
            parser.error("duplicate variant name")
        variants[name] = (backend, Path(binary).resolve())
    cases = [tuple(map(int, value.split(":"))) for value in args.cases]
    if args.repeats < 1 or any(len(case) != 3 or min(case) < 1 or case[0] > 4 for case in cases):
        parser.error("positive counts and at most four server workers are required")
    args.output.mkdir(parents=True, exist_ok=False)
    report = {
        "base": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "platform": platform.platform(),
        "rustc": subprocess.check_output(["rustc", "-Vv"], text=True),
        "topology": subprocess.check_output(["lscpu", "-e=CPU,CORE,SOCKET,NODE,CACHE"], text=True),
        "client": str(args.client.resolve()),
        "mode": args.mode, "cases": cases, "repeats": args.repeats, "seed": 20261025,
        "note": "Optimized release binaries; server CPUs 0 through workers-1; fixed four-worker client on CPUs 4-7. Instrumented throughput is not capacity evidence.",
        "samples": [],
    }
    jobs = list(itertools.product(range(args.repeats), cases, variants))
    random.Random(report["seed"]).shuffle(jobs)
    with open("/tmp/core-scheduler-perf.lock", "a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        report["variants"] = {name: {"backend": backend, "binary": str(binary), "sha256": hashlib.sha256(binary.read_bytes()).hexdigest()} for name, (backend, binary) in variants.items()}
        identities = [(value["backend"], value["sha256"]) for value in report["variants"].values()]
        if len(set(identities)) != len(identities):
            parser.error("variants must have distinct backend/executable identities")
        report["client_sha256"] = hashlib.sha256(args.client.read_bytes()).hexdigest()
        report["governors"] = {str(cpu): Path(f"/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor").read_text().strip() for cpu in range(8)}
        report["scripts"] = {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest() for path in [Path(__file__), HELPER, HELPER.with_name("supervise.py")]}
        for repeat, (workers, connections, size), name in jobs:
            backend, binary = variants[name]
            directory = args.output / name
            directory.mkdir(exist_ok=True)
            settings = SimpleNamespace(server=binary, client=args.client.resolve(), output=directory,
                privileged_perf=args.mode != "control", perf_event="cycles")
            sample = runner.sample(settings, args.mode, backend, workers, connections, size, repeat)
            sample["variant"] = name
            report["samples"].append(sample)
            (args.output / "aiko.json").write_text(json.dumps(report, indent=2) + "\n")
            print(name, workers, connections, size, repeat, round(sample["requests_per_second"]), flush=True)
    if args.mode == "perf":
        for sample in report["samples"]:
            directory = args.output / sample["variant"]
            data = directory / (sample["identifier"] + ".data")
            for name, options in [("self", ["--no-children", "-g", "none"]), ("callgraph", ["--children", "-g", "graph,1,caller"])]:
                with data.with_suffix("." + name + ".txt").open("w") as output:
                    subprocess.run(["sudo", "-n", "perf", "report", "--stdio", "--header", "--show-nr-samples", "--sort", "symbol", "--percent-limit", "0.3", *options, "-i", str(data)], stdout=output, check=True)
    for case in cases:
        for name in variants:
            selected = [sample for sample in report["samples"] if sample["variant"] == name and (sample["workers"], sample["connections"], sample["bytes"]) == case]
            print(case, name, "median", round(statistics.median(sample["requests_per_second"] for sample in selected)), flush=True)


if __name__ == "__main__":
    main()
