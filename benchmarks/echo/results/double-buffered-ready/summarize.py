#!/usr/bin/env python3
"""Summarize the paired ready-queue experiment without pooling RTT percentiles."""

import argparse
import json
import statistics
from pathlib import Path


def summarize(report):
    rows = []
    for workers, connections, size in report["workloads"]:
        for backend in dict.fromkeys(case["backend"] for case in report["cases"]):
            groups = {
                variant: [
                    sample
                    for sample in report["samples"]
                    if (
                        sample["variant"], sample["backend"], sample["workers"],
                        sample["connections"], sample["bytes"],
                    ) == (variant, backend, workers, connections, size)
                ]
                for variant in ["before", "after"]
            }
            row = {"workers": workers, "connections": connections, "bytes": size,
                   "backend": backend}
            for variant, samples in groups.items():
                assert len(samples) == report["repeats"], (variant, row, len(samples))
                row[variant] = {}
                for metric in ["requests_per_second", "rtt_p99_us"]:
                    values = [sample[metric] for sample in samples]
                    row[variant][metric] = {
                        "median": statistics.median(values),
                        "min": min(values), "max": max(values),
                    }
            row["throughput_change_percent"] = 100 * (
                row["after"]["requests_per_second"]["median"]
                / row["before"]["requests_per_second"]["median"] - 1
            )
            rows.append(row)
    return rows


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()
    rows = summarize(json.loads(args.report.read_text()))
    if args.json:
        print(json.dumps(rows, indent=2))
    else:
        print("| Workers/connections/bytes | Backend | Before requests/s [range] | "
              "After requests/s [range] | Change | Median per-run p99 µs, before → after |")
        print("| --- | --- | ---: | ---: | ---: | ---: |")
        for row in rows:
            def throughput(variant):
                data = row[variant]["requests_per_second"]
                return f"{data['median']:,.0f} [{data['min']:,.0f}–{data['max']:,.0f}]"

            print(f"| {row['workers']}/{row['connections']}/{row['bytes']} | "
                  f"{row['backend']} | {throughput('before')} | {throughput('after')} | "
                  f"{row['throughput_change_percent']:+.2f}% | "
                  f"{row['before']['rtt_p99_us']['median']:.3f} → "
                  f"{row['after']['rtt_p99_us']['median']:.3f} |")
