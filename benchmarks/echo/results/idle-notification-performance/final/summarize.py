#!/usr/bin/env python3
"""Validate the frozen echo comparison and regenerate its summary tables."""

from collections import Counter
import hashlib
import itertools
import json
from pathlib import Path
import statistics
import subprocess

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[4]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    report = json.loads((ROOT / "aiko.json").read_text())
    provenance = json.loads((ROOT / "provenance.json").read_text())
    assert report["finished_utc"] >= report["started_utc"]
    assert report["repeats"] == 5
    assert report["warmup_seconds"] == 1
    assert report["measurement_seconds"] == 3
    assert len(report["samples"]) == 150
    assert report["server_cpus"] == "0 through workers-1"
    assert report["client_cpus"] == "4-7"

    for name in ["runner", "helper"]:
        path = ROOT / ("run.py" if name == "runner" else "sample-runner.py")
        assert digest(path) == report[name]["sha256"]
    assert report["client"]["sha256"] == provenance["client"]["sha256"]
    for case in report["cases"]:
        assert case["binary"]["sha256"] == provenance["binaries"][case["label"]]["sha256"]
    tree = subprocess.check_output(
        ["git", "rev-parse", provenance["after_commit"] + "^{tree}"],
        cwd=REPO, text=True,
    ).strip()
    assert tree == provenance["after_tree"]
    assert digest(ROOT / "build/source.patch") == provenance["source_patch_sha256"]
    assert subprocess.check_output(
        ["git", "diff", provenance["before_commit"], provenance["after_commit"]],
        cwd=REPO,
    ) == (ROOT / "build/source.patch").read_bytes()
    assert digest(ROOT / "build/baseline.log") == provenance["baseline_build_log_sha256"]
    assert digest(ROOT / "build/build.log") == provenance["after_build_log_sha256"]

    cases = {(c["label"], c["backend"]) for c in report["cases"]}
    workloads = [tuple(w) for w in report["workloads"]]
    assert cases == set(itertools.product(["before", "after"],
                                          ["tokio", "iouring", "iouring-poll"]))
    assert workloads == [(1, 1, 64), (1, 32, 64), (1, 32, 4096),
                         (4, 128, 64), (4, 128, 4096)]
    counts = Counter()
    identifiers = set()
    for sample in report["samples"]:
        identifier = sample["identifier"]
        assert identifier not in identifiers
        identifiers.add(identifier)
        assert json.loads((ROOT / (identifier + ".json")).read_text()) == sample
        assert (ROOT / (identifier + ".stderr")).read_bytes() == b""
        assert sample["seconds"] == 3 and sample["warmup_seconds"] == 1
        assert sample["requests_per_second"] == sample["requests"] / 3
        assert sample["requests"] > 0
        counts[(sample["variant"], sample["backend"], sample["workers"],
                sample["connections"], sample["bytes"], sample["repeat"])] += 1
    expected = {(label, backend, *workload, repeat)
                for (label, backend), workload, repeat in itertools.product(
                    cases, workloads, range(1, 6))}
    assert set(counts) == expected and set(counts.values()) == {1}

    summary = []
    fields = ["requests_per_second", "rtt_p50_us", "rtt_p99_us", "rtt_mean_us"]
    for workload in workloads:
        for backend in ["tokio", "iouring", "iouring-poll"]:
            row = dict(zip(["workers", "connections", "bytes"], workload), backend=backend)
            for label in ["before", "after"]:
                samples = [s for s in report["samples"] if
                           (s["variant"], s["backend"], s["workers"],
                            s["connections"], s["bytes"]) == (label, backend, *workload)]
                row[label] = {field: {"median": statistics.median(s[field] for s in samples),
                                      "min": min(s[field] for s in samples),
                                      "max": max(s[field] for s in samples)} for field in fields}
            row["throughput_change_percent"] = 100 * (
                row["after"]["requests_per_second"]["median"] /
                row["before"]["requests_per_second"]["median"] - 1)
            summary.append(row)

    for row in summary:
        for label in ["before", "after"]:
            control = next(r for r in summary if r["backend"] == "tokio" and
                           all(r[k] == row[k] for k in ["workers", "connections", "bytes"]))
            row[label]["percent_of_tokio"] = 100 * (
                row[label]["requests_per_second"]["median"] /
                control[label]["requests_per_second"]["median"])
    (ROOT / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    lines = ["# Validated final comparison", "",
             "Each range contains five independently measured repetitions.", "",
             "| Workers/connections/bytes | Backend | Before requests/s (range) | After requests/s (range) | Change | After % Tokio | After p50/p99 RTT (µs) |",
             "| --- | --- | ---: | ---: | ---: | ---: | ---: |"]
    for row in summary:
        rates = []
        for label in ["before", "after"]:
            rate = row[label]["requests_per_second"]
            rates.append(f'{rate["median"]:,.0f} ({rate["min"]:,.0f}–{rate["max"]:,.0f})')
        after = row["after"]
        lines.append(f'| {row["workers"]}/{row["connections"]}/{row["bytes"]} | '
                     f'{row["backend"]} | {rates[0]} | {rates[1]} | '
                     f'{row["throughput_change_percent"]:+.2f}% | '
                     f'{after["percent_of_tokio"]:.2f}% | '
                     f'{after["rtt_p50_us"]["median"]:.2f}/{after["rtt_p99_us"]["median"]:.2f} |')
    (ROOT / "tables.md").write_text("\n".join(lines) + "\n")
    print("Validated 150 samples, repeat coverage, individual files, stderr, and provenance.")


if __name__ == "__main__":
    main()
