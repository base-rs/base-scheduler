"""Validate the complete final matrix and summarize every repetition."""

import itertools
import hashlib
import json
import statistics
from pathlib import Path

root = Path(__file__).resolve().parent
report = json.loads((root / "aiko.json").read_text())
provenance = json.loads((root / "provenance.json").read_text())
for name, filename in [("runner", "run.py"), ("helper", "sample-runner.py"), ("source_patch", "source.patch")]:
    assert hashlib.sha256((root / filename).read_bytes()).hexdigest() == provenance[name]["sha256"]
assert report["client"]["sha256"] == provenance["client"]["sha256"]
for case in report["cases"]:
    assert case["binary"]["sha256"] == provenance["binaries"][case["label"]]["sha256"]
backends = ["tokio", "tokio-adapter", "tokio-adapter-poll", "iouring", "iouring-poll"]
variants = ["before", "after"]
workloads = [(1, 1, 64), (1, 32, 64), (1, 32, 4096), (4, 128, 64), (4, 128, 4096)]
assert report["repeats"] == 3
assert report.get("finished_utc"), "The matrix must finish before it is summarized."
expected = set(itertools.product(variants, backends, workloads, range(1, 4)))
seen = set()
for sample in report["samples"]:
    key = (
        sample["variant"], sample["backend"],
        (sample["workers"], sample["connections"], sample["bytes"]), sample["repeat"],
    )
    assert key not in seen, key
    seen.add(key)
    assert json.loads((root / (sample["identifier"] + ".json")).read_text()) == sample
    assert not (root / (sample["identifier"] + ".stderr")).read_text()
assert seen == expected, (expected - seen, seen - expected)

rows = []
for workers, connections, size in workloads:
    row = {"workers": workers, "connections": connections, "bytes": size, "variants": {}}
    for variant in variants:
        data = {}
        for backend in backends:
            samples = [
                sample for sample in report["samples"]
                if (sample["variant"], sample["backend"], sample["workers"], sample["connections"], sample["bytes"])
                == (variant, backend, workers, connections, size)
            ]
            rates = [sample["requests_per_second"] for sample in samples]
            data[backend] = {
                "median_rps": statistics.median(rates),
                "min_rps": min(rates), "max_rps": max(rates),
                "median_p50_us": statistics.median(sample["rtt_p50_us"] for sample in samples),
                "median_p99_us": statistics.median(sample["rtt_p99_us"] for sample in samples),
            }
        for backend in backends:
            data[backend]["pct_of_tokio"] = 100 * data[backend]["median_rps"] / data["tokio"]["median_rps"]
        row["variants"][variant] = data
    for backend in backends:
        after = row["variants"]["after"][backend]
        before = row["variants"]["before"][backend]
        after["change_pct"] = 100 * (after["median_rps"] / before["median_rps"] - 1)
        after["relative_to_control_change_pct"] = 100 * (after["pct_of_tokio"] / before["pct_of_tokio"] - 1)
    rows.append(row)
(root / "summary.json").write_text(json.dumps(rows, indent=2) + "\n")

lines = [
    "# Final echo matrix", "",
    "Medians of three complete samples per cell; changes compare before/after medians. Throughput ratios use pure Tokio from the same build and workload. No samples are discarded.", "",
    "| Workers/connections/bytes | After Tokio requests/s | Tokio owned | Tokio borrowed | Native owned | Native borrowed |",
    "| --- | ---: | ---: | ---: | ---: | ---: |",
]
for row in rows:
    after = row["variants"]["after"]
    cells = " | ".join(f"{after[backend]['pct_of_tokio']:.1f}%" for backend in backends[1:])
    lines.append(f"| {row['workers']}/{row['connections']}/{row['bytes']} | {after['tokio']['median_rps']:,.0f} | {cells} |")
lines += [
    "", "## Throughput change from before", "",
    "| Workers/connections/bytes | Pure Tokio control | Tokio owned | Tokio borrowed | Native owned | Native borrowed |",
    "| --- | ---: | ---: | ---: | ---: | ---: |",
]
for row in rows:
    cells = " | ".join(f"{row['variants']['after'][backend]['change_pct']:+.1f}%" for backend in backends)
    lines.append(f"| {row['workers']}/{row['connections']}/{row['bytes']} | {cells} |")
lines += [
    "", "## Throughput ranges", "",
    "Minimum–maximum requests/s across all three repetitions. Latency medians remain in `summary.json`.", "",
    "| Workers/connections/bytes | Build | Pure Tokio | Tokio owned | Tokio borrowed | Native owned | Native borrowed |",
    "| --- | --- | ---: | ---: | ---: | ---: | ---: |",
]
for row in rows:
    for variant in variants:
        data = row["variants"][variant]
        cells = " | ".join(f"{data[backend]['min_rps']:,.0f}–{data[backend]['max_rps']:,.0f}" for backend in backends)
        lines.append(f"| {row['workers']}/{row['connections']}/{row['bytes']} | {variant} | {cells} |")
(root / "tables.md").write_text("\n".join(lines) + "\n")
print("\n".join(lines))
