#!/usr/bin/env python3
"""Summarize matched throughput ratios for worker-owned receive pools."""

import argparse
import collections
import json
import math
import statistics
from pathlib import Path


COMPARISONS = [
    ("worker-equal", "shared-provided"),
    ("worker-equal", "shared-monitor"),
    ("worker-equal", "tokio"),
    ("worker-scaled", "worker-equal"),
    ("shared-monitor", "shared-provided"),
]


def summarize(path):
    pairs = collections.defaultdict(dict)
    rows = [json.loads(line) for line in path.read_text().splitlines()]
    for row in rows:
        key = tuple(row[field] for field in ["workers", "connections", "transport", "size", "mode", "repeat"])
        if row["strategy"] in pairs[key]:
            raise ValueError(f"Duplicate sample: {key} {row['strategy']}")
        pairs[key][row["strategy"]] = row
    grouped = collections.defaultdict(list)
    for (*case, _repeat), strategies in pairs.items():
        expected = {"shared-provided", "shared-monitor", "worker-equal", "tokio"}
        if case[0] > 1:
            expected.add("worker-scaled")
        if set(strategies) != expected:
            raise ValueError(f"Unmatched sample: {case}")
        for numerator, denominator in COMPARISONS:
            if numerator in strategies:
                ratio = strategies[numerator]["messages_per_second"] / strategies[denominator]["messages_per_second"]
                grouped[(*case, numerator, denominator)].append(ratio)
    cells = []
    for (*case, numerator, denominator), ratios in sorted(grouped.items()):
        cell = dict(zip(["workers", "connections", "transport", "size", "mode"], case))
        cell.update(numerator=numerator, denominator=denominator, pairs=len(ratios),
                    median_ratio=statistics.median(ratios), minimum=min(ratios), maximum=max(ratios))
        cells.append(cell)
    totals = []
    for numerator, denominator in COMPARISONS:
        for workers in [None, *sorted({cell["workers"] for cell in cells})]:
            ratios = [cell["median_ratio"] for cell in cells
                      if cell["numerator"] == numerator and cell["denominator"] == denominator
                      and (workers is None or cell["workers"] == workers)]
            if ratios:
                totals.append(dict(numerator=numerator, denominator=denominator, workers=workers,
                                   cells=len(ratios), geometric_mean=math.exp(statistics.mean(map(math.log, ratios)))))
    return dict(samples=len(rows), totals=totals, cells=cells)


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
