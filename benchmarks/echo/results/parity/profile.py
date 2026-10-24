#!/usr/bin/env python3
"""Profile matched Tokio, Native monitor, and Native owned echo servers."""

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import subprocess
from types import SimpleNamespace


ROOT = Path(__file__).resolve().parents[4]
runner_path = ROOT / "benchmarks/echo/results/readiness-profiling/run.py"
spec = importlib.util.spec_from_file_location("profile_runner", runner_path)
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, required=True)
    parser.add_argument("--client", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.server = args.server.resolve()
    args.client = args.client.resolve()
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=False)
    report = {
        "source": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "platform": platform.platform(),
        "taskwork_policy": os.environ.get("CORE_SCHEDULER_TASKWORK", "normal"),
        "stream_rearm": os.environ.get("CORE_SCHEDULER_STREAM_REARM"),
        "binaries": {
            name: {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
            for name, path in [("server", args.server), ("client", args.client)]
        },
        "server_cpus": [0],
        "client_cpus": [4, 5, 6, 7],
        "samples": [],
    }
    sampling = SimpleNamespace(**vars(args), perf_event="cycles:u", privileged_perf=True)
    for mode in ["stat", "perf"]:
        for backend in ["tokio", "iouring-monitor", "iouring"]:
            result = runner.sample(sampling, mode, backend, 1, 32, 64, 1)
            report["samples"].append(result)
            (args.output / "samples.json").write_text(json.dumps(report, indent=2) + "\n")
            print(result["identifier"], round(result["requests_per_second"]), flush=True)
    for result in report["samples"]:
        if result["mode"] != "perf":
            continue
        identifier = result["identifier"]
        data = args.output / (identifier + ".data")
        for suffix, options in [
            ("self", ["--no-children", "-g", "none", "--percent-limit", "0.1"]),
            ("callgraph", ["--children", "-g", "graph,1,caller", "--percent-limit", "2"]),
        ]:
            with (args.output / f"{identifier}.{suffix}.txt").open("w") as output:
                subprocess.run([
                    "sudo", "-n", "perf", "report", "--stdio", "--header",
                    "--show-nr-samples", "--sort", "symbol", *options, "-i", str(data),
                ], stdout=output, check=True)


if __name__ == "__main__":
    main()
