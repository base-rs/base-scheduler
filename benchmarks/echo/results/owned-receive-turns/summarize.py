#!/usr/bin/env python3
"""Validate the owned-receive matrix and summarize matched throughput and P99."""

import itertools
import json
import math
from pathlib import Path
import statistics


ROOT = Path(__file__).resolve().parent
VARIANTS = {
    "owned-baseline": ("iouring", "baseline"),
    "owned-candidate": ("iouring", "candidate"),
    "borrowed-baseline": ("iouring-monitor", "baseline"),
    "borrowed-candidate": ("iouring-monitor", "candidate"),
    "tokio": ("tokio", "baseline"),
}
CASES = list(itertools.product([1, 4], [1, 32], [64, 4096]))


def summarize(filename, cases, variants, repeats, build):
    report = json.loads((ROOT / filename).read_text())
    if report["base"] != build["candidate"]:
        raise ValueError("Unexpected checkout revision")
    if report["mode"] != "control" or report["repeats"] != repeats:
        raise ValueError("Unexpected measurement mode or repetition count")
    if set(map(tuple, report["cases"])) != set(cases):
        raise ValueError("Incomplete case matrix")
    if set(report["variants"]) != set(variants):
        raise ValueError("Unexpected variants")
    for name, (backend, binary) in variants.items():
        identity = report["variants"][name]
        if identity["backend"] != backend or identity["sha256"] != build["variants"][binary]["sha256"]:
            raise ValueError("Executable identity mismatch: " + name)
    if report["client_sha256"] != "a8c84568e0932f782179fbbb1c64272129c6af9a5f1344a8c84998b22527fc5f":
        raise ValueError("Unexpected frozen validating client")
    expected = set(itertools.product(variants, cases, range(repeats)))
    observed = set()
    cells = {}
    for sample in report["samples"]:
        case = (sample["workers"], sample["connections"], sample["bytes"])
        key = (sample["variant"], case, sample["repeat"])
        if key not in expected or key in observed:
            raise ValueError("Unexpected or duplicate sample: " + repr(key))
        if sample["mode"] != "control" or sample["backend"] != variants[sample["variant"]][0]:
            raise ValueError("Unexpected sample backend or instrumentation")
        for metric in ["requests_per_second", "rtt_p99_us"]:
            if not math.isfinite(sample[metric]) or sample[metric] <= 0:
                raise ValueError("Invalid " + metric)
        observed.add(key)
        cells.setdefault((sample["variant"], case), []).append(sample)
    if observed != expected:
        raise ValueError("Incomplete sample matrix")
    summary = {"samples": len(observed), "repeats": repeats, "cases": []}
    for case in cases:
        medians = {}
        for name in variants:
            medians[name] = {}
            for metric in ["requests_per_second", "rtt_p99_us"]:
                values = [sample[metric] for sample in cells[name, case]]
                medians[name][metric] = {"median": statistics.median(values), "min": min(values), "max": max(values)}
        comparisons = {}
        for path in ["owned", "borrowed"]:
            if path + "-candidate" not in variants:
                continue
            comparisons[path] = {
                metric + "_change_percent": 100 * (
                    medians[path + "-candidate"][metric]["median"] / medians[path + "-baseline"][metric]["median"] - 1
                )
                for metric in ["requests_per_second", "rtt_p99_us"]
            }
            comparisons[path]["candidate_vs_tokio_percent"] = 100 * (
                medians[path + "-candidate"]["requests_per_second"]["median"] / medians["tokio"]["requests_per_second"]["median"] - 1
            )
        summary["cases"].append({"workers": case[0], "connections": case[1], "bytes": case[2], "variants": medians, "comparisons": comparisons})
    return summary


def main():
    build = json.loads((ROOT / "build-manifest.json").read_text())
    summary = {
        "base": build["base"], "candidate": build["candidate"],
        "wide": summarize("aiko.json", CASES, VARIANTS, 3, build),
        "serial_confirmation": summarize("serial-confirmation.json", [(1, 1, 64)], {
            name: value for name, value in VARIANTS.items() if not name.startswith("borrowed-")
        }, 7, build),
    }
    (ROOT / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    for screen in ["wide", "serial_confirmation"]:
        for cell in summary[screen]["cases"]:
            print(screen, cell["workers"], cell["connections"], cell["bytes"], json.dumps(cell["comparisons"]))


if __name__ == "__main__":
    main()
