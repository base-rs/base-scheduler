#!/usr/bin/env python3
"""Summarize matched throughput ratios for the common pool contract experiment."""

import argparse
import collections
import json
import math
import statistics
from pathlib import Path


def summarize(path):
    pairs = collections.defaultdict(dict)
    for row in map(json.loads, path.read_text().splitlines()):
        key = (row["binary"], tuple(row["parameters"]), row["repeat"])
        if row["version"] in pairs[key]:
            raise ValueError(f"Duplicate sample: {key}")
        pairs[key][row["version"]] = row
    grouped = collections.defaultdict(list)
    for (binary, parameters, _repeat), versions in pairs.items():
        if set(versions) != {"concrete", "contract"}:
            raise ValueError(f"Unmatched sample: {binary} {parameters}")
        metric = {"leases": "leases_per_second", "sockets": "messages_per_second", "files": "gib_per_second"}[binary]
        grouped[(binary, parameters)].append(versions["contract"][metric] / versions["concrete"][metric])
    cells = []
    for (binary, parameters), ratios in sorted(grouped.items()):
        cells.append(dict(binary=binary, parameters=list(parameters), pairs=len(ratios),
                          median_ratio=statistics.median(ratios), minimum=min(ratios), maximum=max(ratios)))
    totals = []
    for binary in sorted({cell["binary"] for cell in cells}):
        ratios = [cell["median_ratio"] for cell in cells if cell["binary"] == binary]
        totals.append(dict(binary=binary, cells=len(ratios),
                           geometric_mean=math.exp(statistics.mean(map(math.log, ratios)))))
    return dict(samples=len(pairs) * 2, totals=totals, cells=cells)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = summarize(args.input)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report["totals"], indent=2))


if __name__ == "__main__":
    main()
