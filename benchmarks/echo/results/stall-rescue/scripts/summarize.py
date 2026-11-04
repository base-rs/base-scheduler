#!/usr/bin/env python3
"""Summarize paired budget runs: medians per build, `main`'s range, and the budget verdict."""
import collections, json, statistics, sys

rows = [json.loads(line) for line in open(sys.argv[1])]
samples = collections.defaultdict(lambda: collections.defaultdict(list))
for row in rows:
    samples[row["case"]][row["build"]].append(row["data"])

def metrics(case, data):
    if case.startswith("rb "):
        workload = case.split()[1]
        if workload in ("idle", "pulse"):
            return {"cpu_ms": data["cpu_ns_per_op"] * data["iterations"] / 1e6}
        result = {"ns/op": data["ns_per_op"], "cpu ns/op": data["cpu_ns_per_op"]}
        if "sleep" in workload:
            result.update({"late p50 us": data["p50"] / 1e3, "late p99 us": data["p99"] / 1e3})
        return result
    if case.startswith("timers "):
        result = {"ns/op": data["ns_per_operation"], "cpu ns/op": data["cpu_ns_per_operation"]}
        if "expire" in case or "cycle" in case:
            result.update({"late p50 us": data["overshoot_p50_ns"] / 1e3, "late p99 us": data["overshoot_p99_ns"] / 1e3})
        return result
    if case.startswith("diag "):
        return {"cpu s": data["cpu_seconds"], "late p50 us": data["lateness_p50_us"], "late p99 us": data["lateness_p99_us"]}
    if case.endswith(" idle"):
        return {"server cpu ms": data["server_cpu_seconds"] * 1e3}
    if "@" in case:
        return {"p50 us": data["rtt_p50_us"], "p99 us": data["rtt_p99_us"], "server cpu s": data["server_cpu_seconds"]}
    return {"req/s": data["requests_per_second"], "p99 us": data["rtt_p99_us"], "server cpu us/req": data["server_cpu_seconds"] * 1e6 / data["requests"]}

budget3 = lambda case: case.startswith(("timers ", "diag "))
higher_is_better = {"req/s"}
print("| Case | Metric | `main` median (min–max) | Branch median | Change | Verdict |")
print("| --- | --- | --- | --- | --- | --- |")
over = []
for case in samples:
    builds = samples[case]
    names = metrics(case, builds["main"][0]).keys()
    for name in names:
        main = [metrics(case, d)[name] for d in builds["main"]]
        branch = [metrics(case, d)[name] for d in builds["branch"]]
        m, b = statistics.median(main), statistics.median(branch)
        change = (b - m) / m * 100 if m else float("nan")
        if budget3(case):
            worse = -change if name in higher_is_better else change
            verdict = "within 3%" if worse <= 3 else "over 3%"
        else:
            verdict = "within noise" if min(main) <= b <= max(main) else ("better" if (b > max(main)) == (name in higher_is_better) else "outside range")
        if verdict in ("over 3%", "outside range"):
            over.append((case, name))
        print(f"| {case} | {name} | {m:.4g} ({min(main):.4g}–{max(main):.4g}) | {b:.4g} | {change:+.1f}% | {verdict} |")
print(f"\n{len(rows)} samples; {len(over)} metrics outside budget:")
for case, name in over:
    print(f"- {case}: {name}")
