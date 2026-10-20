#!/usr/bin/env python3
"""Validate the experiment cells and executable identities; regenerate tables."""
import hashlib
import itertools
import json
import statistics
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent
VARIANTS = ["immediate", "ready", "batch", "ready-batch", "ready-local"]
BUILDS = {v: json.loads((ROOT / f"{v}-build.json").read_text()) for v in VARIANTS}
SHA = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
MEDIAN = statistics.median
summary = {}
lines = []


def binary(variant, suffix):
    return BUILDS[variant]["binaries"][f"{variant}-{suffix}"]["sha256"]


def timer_table(name, variants, workloads, script):
    report = json.loads((ROOT / name / "results.json").read_text())
    assert report["runner_sha256"] == SHA(ROOT / "scripts" / script)
    assert report["timer_slack_ns"] == 1
    samples = report["samples"]
    key = lambda s: (s["variant"], s["workers"], s["tasks"], s["delay_us"], s["aligned"], s["work"], s["repeat"])
    expected = Counter((v, *w, r) for v, w, r in itertools.product(variants, workloads, range(1, 4)))
    assert Counter(map(key, samples)) == expected
    for variant in variants:
        assert report["builds"][variant]["binaries"][f"{variant}-timers"]["sha256"] == binary(variant, "timers")
    rows = []
    lines.extend([f"## {name}", "", "| Workers / timers / delay µs / aligned / work | Variant | Expiries/s | CPU seconds | p50 µs | p99 µs |", "| --- | --- | ---: | ---: | ---: | ---: |"])
    for workload in workloads:
        for variant in variants:
            cell = [s for s in samples if key(s)[:-1] == (variant, *workload)]
            row = dict(variant=variant, workload=workload,
                       rate=MEDIAN(s["expiries_per_second"] for s in cell),
                       cpu=MEDIAN(s["user_seconds"] + s["system_seconds"] for s in cell),
                       p50=MEDIAN(s["lateness_p50_us"] for s in cell),
                       p99=MEDIAN(s["lateness_p99_us"] for s in cell))
            rows.append(row)
            lines.append(f"| {' / '.join(map(str, workload))} | {variant} | {row['rate']:,.0f} | {row['cpu']:.4f} | {row['p50']:.2f} | {row['p99']:.2f} |")
    lines.append("")
    summary[name] = dict(samples=len(samples), rows=rows)


def echo_table(name, variants, workloads, mixed=False):
    report = json.loads((ROOT / name / "aiko.json").read_text())
    assert report["runner"]["sha256"] == SHA(ROOT / "scripts" / "run.py")
    assert report["helper"]["sha256"] == SHA(ROOT / "scripts" / "sample-runner.py")
    assert report["client"]["sha256"] == "db6c914ab75011477ec6232e681046b1e75532cbaa1c81663bac984591675cf3"
    cases = [(v, "iouring-poll") for v in variants]
    if not mixed:
        cases.append(("control", "tokio"))
    assert [(c["label"], c["backend"]) for c in report["cases"]] == cases
    for case in report["cases"]:
        variant = "immediate" if case["label"] == "control" else case["label"]
        suffix = "mixed" if mixed else "core-scheduler-echo-benchmark"
        assert case["binary"]["sha256"] == binary(variant, suffix)
    key = lambda s: (s["variant"], s["backend"], s["workers"], s["connections"], s["bytes"], s["repeat"])
    expected = Counter((*c, *w, r) for c, w, r in itertools.product(cases, workloads, range(1, 4)))
    assert Counter(map(key, report["samples"])) == expected
    rows = []
    lines.extend([f"## {name}", "", "| Workers / connections / bytes | Variant | Roundtrips/s | Min–max | Timer p99 µs |", "| --- | --- | ---: | ---: | ---: |"])
    for workload in workloads:
        for variant, backend in cases:
            samples = [s for s in report["samples"] if key(s)[:-1] == (variant, backend, *workload)]
            rates = [s["requests_per_second"] for s in samples]
            p99 = None
            if mixed:
                histograms = [json.loads((ROOT / name / f"{s['identifier']}.stderr").read_text()) for s in samples]
                assert all(h["tasks"] == 32 and h["completed"] > 0 for h in histograms)
                p99 = MEDIAN(h["lateness_p99_us"] for h in histograms)
            row = dict(variant=variant, workload=workload, rate=MEDIAN(rates), minimum=min(rates), maximum=max(rates), timer_p99=p99)
            rows.append(row)
            tail = f"{p99:.2f}" if p99 is not None else "—"
            lines.append(f"| {' / '.join(map(str, workload))} | {variant} | {row['rate']:,.0f} | {min(rates):,.0f}–{max(rates):,.0f} | {tail} |")
    lines.append("")
    summary[name] = dict(samples=len(report["samples"]), rows=rows)


source = json.loads((ROOT / "source-verification.json").read_text())
provenance = json.loads((ROOT / "provenance.json").read_text())
assert source["candidate_sha256"] == SHA(ROOT.parent / "local-timers" / "candidate.patch")
for variant, build in BUILDS.items():
    assert build["source"] == "64eb1548e1371b067f0e7c8b85d5a37c2f745768"
    assert "--release" in build["build_command"] and "--locked" in build["build_command"]
    assert not build["environment_overrides"]
    assert build["files"]["benchmarks/echo/src/bin/timers.rs"] == SHA(ROOT / "scripts" / "timers.rs")
    assert binary(variant, "mixed") != binary(variant, "core-scheduler-echo-benchmark")
    assert build["rustc"] == provenance["compiler"]
    for name, expected in build["files"].items():
        if name != "benchmarks/echo/src/bin/timers.rs":
            assert source["source_files"][variant][name] == expected
    if variant != "immediate":
        assert source["patches"][variant] == SHA(ROOT / f"{variant}.patch")
    for name, executable in build["binaries"].items():
        assert provenance["binaries"][name] == executable["sha256"]

workloads = [(1, 1, 10, False, 0), (1, 32, 1000, False, 0), (4, 1024, 1000, False, 0), (4, 1024, 1000, True, 0)]
timer_table("timers", ["immediate", "ready", "batch"], workloads + [(4, 1024, 1000, True, 10000)], "measure.py")
timer_table("optimized-timers", ["immediate", "ready-batch", "ready-local"], workloads, "measure-optimized.py")
echo_table("echo", ["immediate", "ready", "batch"], [(1, 1, 64), (4, 128, 64)])
echo_table("mixed", ["immediate", "ready", "batch"], [(1, 32, 64), (4, 128, 64)], mixed=True)
echo_table("optimized-echo", ["immediate", "ready-batch", "ready-local"], [(1, 1, 64), (4, 128, 64)])
echo_table("optimized-mixed", ["immediate", "ready-batch", "ready-local"], [(1, 32, 64), (4, 128, 64)], mixed=True)

stat = json.loads((ROOT / "stat" / "results.json").read_text())
assert stat["runner_sha256"] == SHA(ROOT / "scripts" / "stat.py")
assert Counter((s["variant"], s["repeat"]) for s in stat["samples"]) == Counter(itertools.product(["immediate", "ready", "batch"], [1, 2]))
summary["stat"] = dict(samples=len(stat["samples"]), rows=[])
lines.extend(["## Instrumented relative timers: 4 workers / 1,024 timers / 1 ms", "", "| Variant | Instructions | Ring entries | Context switches |", "| --- | ---: | ---: | ---: |"])
for variant in ["immediate", "ready", "batch"]:
    rows = []
    for sample in stat["samples"]:
        if sample["variant"] != variant:
            continue
        counters = {}
        for line in (ROOT / "stat" / f"{sample['identifier']}.stat").read_text().splitlines():
            if not line or line.startswith("#"):
                continue
            fields = line.split(";")
            counters[fields[2]] = float(fields[0])
        rows.append(counters)
    row = dict(variant=variant, **{k: MEDIAN(r[k] for r in rows) for k in rows[0]})
    summary["stat"]["rows"].append(row)
    lines.append(f"| {variant} | {row['instructions']:,.0f} | {row['syscalls:sys_enter_io_uring_enter']:,.0f} | {row['context-switches']:,.0f} |")

summary["total_samples"] = sum(s["samples"] for s in summary.values() if isinstance(s, dict))
(ROOT / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
(ROOT / "tables.md").write_text("\n".join(lines) + "\n")
print(f"Validated {summary['total_samples']} samples and executable identities.")
