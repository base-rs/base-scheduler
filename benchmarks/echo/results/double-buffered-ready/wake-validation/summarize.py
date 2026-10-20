#!/usr/bin/env python3
"""Validate complete runs and compare three worker-loop variants, without pooling p99s."""

import argparse
import itertools
import json
import statistics
from pathlib import Path


def summarize(report):
    assert report.get("finished_utc"), "benchmark did not finish"
    expected = {
        (case["label"], case["backend"], *workload, repeat)
        for case, workload, repeat in itertools.product(
            report["cases"], report["workloads"], range(1, report["repeats"] + 1)
        )
    }
    actual = [
        tuple(sample[key] for key in
              ("variant", "backend", "workers", "connections", "bytes", "repeat"))
        for sample in report["samples"]
    ]
    assert len(actual) == len(expected) and set(actual) == expected, "missing/duplicate runs"
    assert all(sample["requests"] > 0 and sample["seconds"] == 3
               and sample["warmup_seconds"] == 1 for sample in report["samples"])
    rows = []
    for workers, connections, size in report["workloads"]:
        for backend in dict.fromkeys(case["backend"] for case in report["cases"]):
            row = {"workers": workers, "connections": connections, "bytes": size,
                   "backend": backend, "variants": {}}
            for variant in dict.fromkeys(case["label"] for case in report["cases"]
                                         if case["backend"] == backend):
                samples = [sample for sample in report["samples"] if
                           (sample["variant"], sample["backend"], sample["workers"],
                            sample["connections"], sample["bytes"]) ==
                           (variant, backend, workers, connections, size)]
                assert len(samples) == report["repeats"]
                row["variants"][variant] = {}
                for metric in ("requests_per_second", "rtt_p99_us"):
                    values = [sample[metric] for sample in samples]
                    row["variants"][variant][metric] = {
                        "median": statistics.median(values), "min": min(values),
                        "max": max(values), "runs": len(values),
                    }
            fixed = row["variants"]["fixed"]["requests_per_second"]["median"]
            row["fixed_throughput_change_percent"] = {
                variant: 100 * (fixed / data["requests_per_second"]["median"] - 1)
                for variant, data in row["variants"].items() if variant != "fixed"
            }
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
        print("| Workers/connections/bytes | Backend | Original req/s [range] | "
              "Uncorrected req/s [range] | Fixed req/s [range] | Fixed/original | "
              "Fixed/uncorrected | Median per-run p99 µs, original → uncorrected → fixed |")
        print("| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |")
        for row in rows:
            def throughput(variant):
                if variant not in row["variants"]:
                    return "—"
                data = row["variants"][variant]["requests_per_second"]
                return f"{data['median']:,.0f} [{data['min']:,.0f}–{data['max']:,.0f}]"

            def change(variant):
                value = row["fixed_throughput_change_percent"].get(variant)
                return "—" if value is None else f"{value:+.2f}%"

            latencies = " → ".join(
                f"{row['variants'][variant]['rtt_p99_us']['median']:.3f}"
                if variant in row["variants"] else "—"
                for variant in ("original", "uncorrected", "fixed")
            )
            print(f"| {row['workers']}/{row['connections']}/{row['bytes']} | "
                  f"{row['backend']} | {throughput('original')} | "
                  f"{throughput('uncorrected')} | {throughput('fixed')} | "
                  f"{change('original')} | {change('uncorrected')} | {latencies} |")
