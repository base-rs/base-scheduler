#!/usr/bin/env python3
"""Validate the frozen binaries and matrices, then summarize the matched screens."""

import hashlib
import itertools
import json
from pathlib import Path
import statistics

ROOT = Path(__file__).resolve().parent
CLIENT = "a8c84568e0932f782179fbbb1c64272129c6af9a5f1344a8c84998b22527fc5f"
VARIANTS = {"owned-baseline", "owned-lazy", "borrowed-baseline", "borrowed-lazy", "tokio"}
BACKENDS = {"owned": "iouring", "borrowed": "iouring-monitor", "tokio": "tokio"}


def main():
    build = json.loads((ROOT / "build-manifest.json").read_text())
    baseline = build["baseline"]["variants"]["baseline"]["sha256"]
    candidate = build["candidate"]["sha256"]
    assert baseline != candidate
    assert hashlib.sha256((ROOT / "source.patch").read_bytes()).hexdigest() == build["candidate"]["patch_sha256"]
    summary = {}
    confirmation_cases = [(1, 1, 64), (1, 32, 64)]
    wide_cases = [case for case in itertools.product([1, 4], [1, 32], [64, 4096]) if case not in confirmation_cases]
    for name, cases, repeats in [
        ("confirmation", confirmation_cases, 5),
        ("wide", wide_cases, 3),
    ]:
        report = json.loads((ROOT / (name + ".json")).read_text())
        assert report["mode"] == "control" and report["repeats"] == repeats
        assert {tuple(case) for case in report["cases"]} == set(cases)
        assert report["client_sha256"] == CLIENT
        assert set(report["variants"]) == VARIANTS
        for variant, details in report["variants"].items():
            assert details["sha256"] == (candidate if variant.endswith("-lazy") else baseline)
            assert details["backend"] == BACKENDS[variant.split("-")[0]]
        keys = [(row["variant"], (row["workers"], row["connections"], row["bytes"]), row["repeat"]) for row in report["samples"]]
        assert len(keys) == len(set(keys))
        assert set(keys) == set(itertools.product(VARIANTS, cases, range(repeats)))
        cells = {}
        for case in sorted(cases):
            cell = {}
            for variant in sorted(VARIANTS):
                rows = [row for row in report["samples"] if row["variant"] == variant and (row["workers"], row["connections"], row["bytes"]) == case]
                assert all(row["seconds"] == 3 and row["warmup_seconds"] == 1 and row["requests"] > 0 for row in rows)
                assert all(row["backend"] == BACKENDS[variant.split("-")[0]] for row in rows)
                values = {}
                for metric in ["requests_per_second", "rtt_p99_us"]:
                    measured = [row[metric] for row in rows]
                    values[metric] = {"median": statistics.median(measured), "minimum": min(measured), "maximum": max(measured)}
                cell[variant] = values
            for path in ["owned", "borrowed"]:
                a = cell[path + "-baseline"]["requests_per_second"]["median"]
                b = cell[path + "-lazy"]["requests_per_second"]["median"]
                cell[path + "-change-percent"] = 100 * (b / a - 1)
            cells["/".join(map(str, case))] = cell
        summary[name] = cells
    (ROOT / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    for name, cells in summary.items():
        print(name)
        for case, cell in cells.items():
            print(case, "owned", f'{cell["owned-change-percent"]:+.2f}%', "borrowed", f'{cell["borrowed-change-percent"]:+.2f}%')


if __name__ == "__main__":
    main()
