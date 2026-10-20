#!/usr/bin/env python3
"""Validate the completed experiments and regenerate their numerical summary."""

from collections import Counter, defaultdict
import hashlib
import itertools
import json
from pathlib import Path
from statistics import median


ROOT = Path(__file__).resolve().parent
BACKENDS = ["tokio", "tokio-adapter", "tokio-adapter-poll", "iouring", "iouring-poll"]


def read(path):
    return json.loads((ROOT / path).read_text())


def digest(path):
    return hashlib.sha256((ROOT / path).read_bytes()).hexdigest()


def matrix(report, expected):
    assert report.get("finished_utc"), "Experiment has not finished"
    actual = Counter(
        (s["mode"], s["backend"], s["workers"], s["connections"], s["bytes"], s["repeat"])
        for s in report["samples"]
    )
    assert actual == Counter(expected), (actual - Counter(expected), Counter(expected) - actual)
    for sample in report["samples"]:
        assert sample["requests"] > 0
        assert abs(sample["requests"] / sample["seconds"] - sample["requests_per_second"]) < 0.001


def summarize_samples(samples):
    groups = defaultdict(list)
    for s in samples:
        groups[(s["mode"], s["backend"], s["workers"], s["connections"], s["bytes"])].append(s)
    return [
        dict(
            mode=mode, backend=backend, workers=workers, connections=connections, bytes=size,
            samples=len(values), median_requests_per_second=median(s["requests_per_second"] for s in values),
            min_requests_per_second=min(s["requests_per_second"] for s in values),
            max_requests_per_second=max(s["requests_per_second"] for s in values),
            median_rtt_p50_us=median(s["rtt_p50_us"] for s in values),
            median_rtt_p99_us=median(s["rtt_p99_us"] for s in values),
        )
        for (mode, backend, workers, connections, size), values in sorted(groups.items())
    ]


profile = read("profile/aiko.json")
assert profile["merged_source"] == "130cd4a67e526e80ba9a5e7e7ebbe81494bd903b"
assert profile["measured_source"] == "9386f7c4f6cb0dccd7e528512476d2cf963f7a88"
expected = []
for mode, repeats, workloads in [("control", 3, [(1, 32, 64), (4, 128, 64)]),
                                  ("stat", 2, [(1, 32, 64)]), ("perf", 1, [(1, 32, 64)])]:
    expected.extend((mode, backend, *workload, repeat)
                    for backend, workload, repeat in itertools.product(BACKENDS, workloads, range(1, repeats + 1)))
matrix(profile, expected)
for name, sha in profile["scripts"].items():
    assert digest("scripts/" + name) == sha

reports = {}
for name, cases, workloads, repeats in [
    ("semantic-controls", [("diagnostic", backend) for backend in [
        *BACKENDS, "tokio-policy-native", "tokio-policy-conservative",
        "tokio-policy-native-adapter-task", "tokio-policy-conservative-adapter-task",
        "tokio-adapter-raw-task", "tokio-adapter-poll-raw-task", "native-readiness", "native-readiness-poll",
    ]], [(1, 32, 64)], 3),
    ("poll-first", list(itertools.product(["before", "poll-first"], ["tokio", "iouring"])),
     [(1, 1, 64), (1, 32, 64), (4, 128, 64)], 3),
    ("deferred", list(itertools.product(["before", "deferred"], ["tokio", "iouring", "iouring-poll"])),
     [(1, 1, 64), (1, 32, 64), (4, 128, 64)], 3),
    ("matched-policy", [("matched", backend) for backend in
                        ["tokio-policy-native", "tokio-policy-poll", "tokio-adapter", "tokio-adapter-poll"]],
     [(1, 32, 64), (1, 32, 4096), (4, 128, 64)], 5),
]:
    report = read(name + "/aiko.json")
    matrix(report, [(label, backend, *workload, repeat)
                    for (label, backend), workload, repeat in itertools.product(cases, workloads, range(1, repeats + 1))])
    assert report["runner"]["sha256"] == digest("scripts/run.py")
    assert report["helper"]["sha256"] == digest("scripts/sample-runner.py")
    assert report["client"]["sha256"] == profile["binaries"]["client"]["sha256"]
    build = read(name + "/build.json")
    assert build["base_source"] == profile["merged_source"]
    patch_key = "production_patch_sha256" if "production_patch_sha256" in build else "patch_sha256"
    assert digest(name + "/change.patch") == build[patch_key]
    assert report["build"] == build
    for case in report["cases"]:
        expected_binary = build["baseline_binary_sha256"] if case["label"] == "before" else build["binary_sha256"]
        assert case["binary"]["sha256"] == expected_binary
    reports[name] = report

for name in ["poll-first", "deferred"]:
    assert read(name + "/build.json")["baseline_binary_sha256"] == read("semantic-controls/build.json")["binary_sha256"]

errors = read("receive-errors/aiko.json")
matrix(errors, [("stat", backend, 1, 32, 64, repeat)
                for backend, repeat in itertools.product([*BACKENDS, "tokio-policy-conservative"], [1, 2])])
assert errors["helper_sha256"] == digest("scripts/exit-sample-runner.py")
assert errors["binary_sha256"] == read("semantic-controls/build.json")["binary_sha256"]


def stat_summary(report, directory):
    groups = defaultdict(lambda: defaultdict(list))
    for sample in report["samples"]:
        if sample["mode"] != "stat":
            continue
        for line in (ROOT / directory / (sample["identifier"] + ".stat")).read_text().splitlines():
            fields = line.split(";")
            if len(fields) > 2 and fields[0].strip().isdigit():
                groups[sample["backend"]][fields[2]].append(int(fields[0]) / sample["requests"])
    return {backend: {event: median(values) for event, values in events.items()}
            for backend, events in sorted(groups.items())}


summary = {
    "baseline": summarize_samples([s for s in profile["samples"] if s["mode"] == "control"]),
    **{name: summarize_samples(report["samples"]) for name, report in reports.items()},
    "baseline_stat_per_echo": stat_summary(profile, "profile/stat"),
    "receive_errors_per_echo": stat_summary(errors, "receive-errors"),
}


def throughput(name, mode, backend, workload):
    rows = [s for s in summary[name]
            if (s["mode"], s["backend"], s["workers"], s["connections"], s["bytes"]) == (mode, backend, *workload)]
    assert len(rows) == 1
    return rows[0]["median_requests_per_second"]


summary["matched_policy_ratios"] = [
    dict(workers=w, connections=c, bytes=b, backend=backend,
         relative_to_ordinary_tokio=throughput("matched-policy", "matched", backend, (w, c, b))
         / throughput("matched-policy", "matched", "tokio-policy-native", (w, c, b)),
         relative_to_conservative_polling_tokio=throughput("matched-policy", "matched", backend, (w, c, b))
         / throughput("matched-policy", "matched", "tokio-policy-poll", (w, c, b)))
    for w, c, b in [(1, 32, 64), (1, 32, 4096), (4, 128, 64)]
    for backend in ["tokio-adapter", "tokio-adapter-poll"]
]
(ROOT / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")

tables = ["# Validated research tables", "", "Generated by `python3 summarize.py`. Throughput uses uninstrumented samples only.", ""]
for name in ["baseline", *reports]:
    tables += ["## " + name, "", "| Variant | Backend | Workers/connections/bytes | Repeats | Median requests/s | Range |",
               "| --- | --- | --- | ---: | ---: | --- |"]
    for s in summary[name]:
        tables.append(f"| {s['mode']} | {s['backend']} | {s['workers']}/{s['connections']}/{s['bytes']} | {s['samples']} | {s['median_requests_per_second']:,.0f} | {s['min_requests_per_second']:,.0f}–{s['max_requests_per_second']:,.0f} |")
    tables.append("")
tables += ["## Matched-policy adapter ratios", "", "| Workers/connections/bytes | Backend | Relative to ordinary Tokio | Relative to conservative polling Tokio |",
           "| --- | --- | ---: | ---: |"]
for s in summary["matched_policy_ratios"]:
    tables.append(f"| {s['workers']}/{s['connections']}/{s['bytes']} | {s['backend']} | {100*s['relative_to_ordinary_tokio']:.2f}% | {100*s['relative_to_conservative_polling_tokio']:.2f}% |")
tables.append("")
tables += ["## Receive syscall counts", "", "Instrumented whole-server counts divided by completed echoes; startup and shutdown contribute small residuals.", "",
           "| Backend | recvfrom/echo | recvfrom EAGAIN/echo | sendto/echo | io_uring_enter/echo |",
           "| --- | ---: | ---: | ---: | ---: |"]
for backend, events in summary["receive_errors_per_echo"].items():
    keys = ["syscalls:sys_enter_recvfrom", "syscalls:sys_exit_recvfrom", "syscalls:sys_enter_sendto", "syscalls:sys_enter_io_uring_enter"]
    tables.append("| " + backend + " | " + " | ".join(f"{events[key]:.4f}" for key in keys) + " |")
(ROOT / "tables.md").write_text("\n".join(tables) + "\n")
print("Validated all completed matrices, recorded script/patch identities, and executable provenance; wrote summary.json and tables.md.")
