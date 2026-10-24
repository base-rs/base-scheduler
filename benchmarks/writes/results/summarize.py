#!/usr/bin/env python3
"""Summarize retained successful samples without choosing favorable repetitions."""

import collections
import json
from pathlib import Path
import statistics


def summarize(path):
    report = json.loads(path.read_text())
    groups = collections.defaultdict(list)
    for sample in report["samples"]:
        result = sample.get("result", sample)
        key = (
            sample["workers"],
            result["connections"],
            result["bytes"],
            result.get("chunks", 1),
            result.get("work", 0),
            sample.get("send_buffer", 0),
            sample.get("variant", sample["backend"]),
            sample.get("server_backend", sample["backend"]),
        )
        groups[key].append(result["requests_per_second"])
    print(f"\n{path.name}: {len(report['samples'])} samples")
    for key, values in sorted(groups.items()):
        print(
            key,
            f"median={statistics.median(values):.1f}",
            f"range={min(values):.1f}–{max(values):.1f}",
            f"n={len(values)}",
        )


if __name__ == "__main__":
    root = Path(__file__).resolve().parent
    for name in ["shared-prepare.json", "final-controls.json", "echo-control.json"]:
        path = root / name
        if path.exists():
            summarize(path)
