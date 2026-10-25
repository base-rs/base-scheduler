"""Validate the follow-up matrices and summarize every retained capacity sample."""

import csv
import hashlib
import itertools
import json
from pathlib import Path
import statistics


ROOT = Path(__file__).resolve().parent
BASE = "83433057fc469e076dcd7e7f42a61b8147a91ced"
BINARY = "8ea109bf21845f3fa0aeee9964726a242b65dce08fe4262fe83e664f38d6c12b"
CLIENT = "a8c84568e0932f782179fbbb1c64272129c6af9a5f1344a8c84998b22527fc5f"
EXPERIMENTS = {'tokio-no-short': [('tokio-no-short', 'tokio-no-short', 'tokio-no-short')],
 'permit': [('permit', 'reusable-permit', 'candidate')],
 'slot': [('slot', 'read-completion-slot', 'candidate')],
 'tcp-inq': [('tcp-inq', 'tcp-inq-readiness', 'candidate-owned')],
 'probe': [('probe', 'owned-read-probe', 'candidate')],
 'counter': [('counter', 'counter-control', 'candidate')],
 'retire': [('retire', 'batched-retirement', 'candidate')],
 'dispatch': [('dispatch', 'request-dispatch', 'candidate')],
 'dispatch-v2': [('dispatch-v2', 'request-dispatch-v2', 'candidate')],
 'flatten': [('flatten', 'flatten-owned-read', 'candidate')],
 'split': [('split', 'split-executor', 'split-native')],
 'fresh-state': [('async-task-fresh-state', 'async-task-fresh-state', 'candidate')],
 'admission': [('admission', 'unbounded-read-control', 'candidate')],
 'no-slp': [('no-slp', None, 'native-no-slp')],
 'batch': [('batch-4', 'batch-submission-4', 'batch-4'), ('batch-16', 'batch-submission-16', 'batch-16')],
 'boxed': [('boxed-ahead', 'boxed-ahead', 'boxed-ahead'),
           ('boxed-cold-paths', 'boxed-cold-paths', 'boxed-cold-paths')],
 'lazy-write': [('lazy-write', 'lazy-owned-write', 'candidate')],
 'inline-lane': [('inline-lane', 'inline-owned-lane', 'candidate')],
 'waker-slot': [('waker-slot', 'read-waker-slot', 'candidate')],
 'fused-release': [('fused-release', 'async-task-fused-release', 'candidate')]}


def read(name):
    return json.loads((ROOT / name).read_text())


def summarize(name, candidates, *, repeats=5, counter_mode=None):
    report = read(name + ("-cache.json" if counter_mode else "-screen.json"))
    binaries = {BINARY}
    for manifest_name, patch_name, candidate in candidates:
        manifest = read(manifest_name + "-build-manifest.json")
        assert manifest["base"] == BASE
        if patch_name is not None:
            patch = ROOT / (patch_name + ".patch")
            assert hashlib.sha256(patch.read_bytes()).hexdigest() == manifest["patch_sha256"]
        else:
            assert manifest["environment"]["RUSTFLAGS"] == "-C no-vectorize-slp"
        assert report["variants"][candidate]["sha256"] == manifest["binary_sha256"]
        binaries.add(manifest["binary_sha256"])
    assert report["client_sha256"] == CLIENT
    assert report["counter_mode"] == counter_mode
    warmup = 0 if counter_mode else 1
    variants = report["variants"]
    assert all(variant["sha256"] in binaries for variant in variants.values())
    assert report["settings"]["seconds"] == 3
    assert report["settings"]["warmup"] == warmup
    assert report["repeats"] == repeats
    if name == "concurrency":
        assert report["cases"] == [[1, count, 64] for count in [1, 8, 32, 128]]
        assert set(report["variants"]) == {"native", "tokio"}
    expected = set(itertools.product(map(tuple, report["cases"]), variants, range(report["repeats"])))
    observed = set()
    groups = {}
    for sample in report["samples"]:
        case = sample["workers"], sample["connections"], sample["bytes"]
        key = case, sample["variant"], sample["repeat"]
        assert key in expected and key not in observed, key
        observed.add(key)
        assert sample["server_cpus"] == list(range(sample["workers"]))
        assert sample["client_cpus"] == [4, 5, 6, 7]
        assert sample["client_workers"] == 4
        assert sample["seconds"] == 3 and sample["warmup_seconds"] == warmup
        assert sample["backend"] == variants[sample["variant"]]["backend"]
        assert sample["requests_per_second"] == sample["requests"] / sample["seconds"]
        if counter_mode:
            assert set(sample["counters"]) == set(report["events"])
            for event, count in sample["counters"].items():
                assert count > 0
                assert sample["per_request"][event] == count / sample["requests"]
            filename = f"concurrency-cache-{sample['variant']}-w{sample['workers']}-c{sample['connections']}-b{sample['bytes']}-r{sample['repeat']}.csv"
            rows = list(csv.reader((ROOT / "cache-counters" / filename).read_text().splitlines()))
            raw = {row[2]: row for row in rows if len(row) >= 5 and row[2] in report["events"]}
            assert set(raw) == set(report["events"])
            for event, count in sample["counters"].items():
                assert float(raw[event][0]) == count
                assert float(raw[event][4]) == 100.0
        groups.setdefault(case, {}).setdefault(sample["variant"], []).append(sample)
    assert observed == expected
    cases = []
    for case, values in sorted(groups.items()):
        summary = {}
        for variant, samples in sorted(values.items()):
            rates = [sample["requests_per_second"] for sample in samples]
            summary[variant] = {
                "median": statistics.median(rates),
                "minimum": min(rates),
                "maximum": max(rates),
                "median_sample_p99_us": statistics.median(sample["rtt_p99_us"] for sample in samples),
            }
            if counter_mode:
                summary[variant]["per_request"] = {
                    event: statistics.median(sample["per_request"][event] for sample in samples)
                    for event in report["events"]
                }
        if "baseline" in summary:
            for _, _, candidate in candidates:
                summary[candidate]["change_percent"] = 100 * (summary[candidate]["median"] / summary["baseline"]["median"] - 1)
        cases.append({"case": case, "variants": summary})
    return {"samples": len(observed), "cases": cases}


def task_polls():
    report = read("task-poll-current.json")
    assert report["base"] == BASE
    assert report["server_cpus"] == [0]
    source = ROOT / "scripts/task-poll.rs"
    assert hashlib.sha256(source.read_bytes()).hexdigest() == report["source_sha256"]
    expected = set(itertools.product(["native", "tokio"], [1, 32], ["borrowed", "owned"]))
    observed = set()
    result = {}
    for case in report["samples"]:
        key = case["backend"], case["tasks"], case["wake"]
        assert key in expected and key not in observed
        observed.add(key)
        assert case["workers"] == 1 and case["polls"] == 5_000_000
        assert case["warmup_samples"] == 1 and len(case["samples"]) == 5
        for sample in case["samples"]:
            assert sample["polls"] == 5_000_000
            assert sample["polls_per_second"] == sample["polls"] / sample["seconds"]
        rate = statistics.median(sample["polls_per_second"] for sample in case["samples"])
        result["/".join(map(str, key))] = {"median_polls_per_second": rate, "nanoseconds_per_poll": 1e9 / rate}
    assert observed == expected
    return {"samples": 40, "cases": result}


def main():
    summary = {
        name: summarize(name, configuration)
        for name, configuration in EXPERIMENTS.items()
    }
    rebuild = read("clean-rebuild-build-manifest.json")
    assert rebuild["base"] == BASE and rebuild["binary_sha256"] == BINARY
    summary["task-poll"] = task_polls()
    summary["concurrency"] = summarize("concurrency", [], repeats=3)
    summary["concurrency-cache"] = summarize("concurrency", [], repeats=2, counter_mode="cache")
    (ROOT / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    capacity_samples = sum(report["samples"] for name, report in summary.items() if name not in {"task-poll", "concurrency-cache"})
    print(f"Validated {capacity_samples} capacity samples, 16 cache-counter samples, and 40 task-only samples across {len(summary)} experiments.")


if __name__ == "__main__":
    main()
