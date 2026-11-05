#!/usr/bin/env python3
"""Summarize the stall rescue cost measurements as Markdown: medians with ranges for each build, and the change from `main` before stall rescue."""
import collections, json, pathlib, statistics, sys

directory = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else pathlib.Path(__file__).parent)
names = {"m2": "before", "now": "on", "nowoff": "off"}

def table(rows, metrics, title, intro):
    print(f"## {title}\n\n{intro}\n")
    print("| Case | Metric | before | on | off | on vs before | off vs before |")
    print("| --- | --- | --- | --- | --- | --- | --- |")
    for case in sorted({key[0] for key in rows}):
        for metric, get in metrics.items():
            values = {build: [get(r) for r in rows.get((case, build), [])] for build in names}
            if not all(values.values()):
                continue
            median = {build: statistics.median(v) for build, v in values.items()}
            cell = lambda build: f"{median[build]:.4g} ({min(values[build]):.3g}–{max(values[build]):.3g})"
            change = lambda build: f"{(median[build] / median['m2'] - 1) * 100:+.1f}%" if median["m2"] else "n/a"
            print(f"| {case} | {metric} | {cell('m2')} | {cell('now')} | {cell('nowoff')} | {change('now')} | {change('nowoff')} |")
    print()

costs = collections.defaultdict(list)
for line in open(directory / "costs.jsonl"):
    r = json.loads(line); costs[(r["case"], r["build"])].append(r)
worker = lambda key: (lambda r: r["threads"].get("base-native-tas", {}).get(key, 0))
table({k: v for k, v in costs.items() if k[0].startswith("periodic")}, {
    "p50 lateness µs": lambda r: r["record"]["lateness_p50_us"],
    "p99 lateness µs": lambda r: r["record"]["lateness_p99_us"],
    "process CPU s": lambda r: r["record"]["cpu_seconds"],
    "worker CPU s": worker("cpu_seconds"),
    "worker sleeps": worker("voluntary"),
    "monitor wakes": lambda r: r["threads"].get("base-native-mon", {}).get("voluntary", 0),
}, "Periodic diagnostic and pulse", "From `costs.jsonl`: 15 repeats, builds in random order within each. Worker CPU and sleeps (voluntary context switches) are summed over the worker threads; monitor wakes are the monitor thread's voluntary context switches.")
print("### On against off, paired by repeat\n")
print("Each repeat ran all three builds back to back, so differences within a repeat share its conditions. The change is the median of the per-repeat changes from off to on.\n")
print("| Case | Metric | Median change, on vs off | Repeats with on higher |")
print("| --- | --- | --- | --- |")
paired = {
    "p50 lateness": lambda r: r["record"]["lateness_p50_us"],
    "process CPU": lambda r: r["record"]["cpu_seconds"],
    "worker CPU": worker("cpu_seconds"),
}
for case in sorted({key[0] for key in costs if key[0].startswith("periodic")}):
    on = {r["repeat"]: r for r in costs[(case, "now")]}; off = {r["repeat"]: r for r in costs[(case, "nowoff")]}
    for metric, get in paired.items():
        changes = [(get(on[k]) / get(off[k]) - 1) * 100 for k in sorted(on) if k in off and get(off[k])]
        higher = sum(1 for k in on if k in off and get(on[k]) > get(off[k]))
        print(f"| {case} | {metric} | {statistics.median(changes):+.1f}% | {higher} of {len(changes)} |")
print("\nP50 lateness against `main` before stall rescue, paired by repeat:\n")
print("| Case | Median change, on vs before | Repeats with on higher | Median change, off vs before | Repeats with off higher |")
print("| --- | --- | --- | --- | --- |")
p50 = paired["p50 lateness"]
for case in sorted({key[0] for key in costs if key[0].startswith("periodic")}):
    before = {r["repeat"]: r for r in costs[(case, "m2")]}
    cells = []
    for build in ["now", "nowoff"]:
        runs = {r["repeat"]: r for r in costs[(case, build)]}
        changes = [(p50(runs[k]) / p50(before[k]) - 1) * 100 for k in sorted(runs) if k in before]
        cells += [f"{statistics.median(changes):+.1f}%", f"{sum(1 for k in runs if k in before and p50(runs[k]) > p50(before[k]))} of {len(changes)}"]
    print(f"| {case} | " + " | ".join(cells) + " |")
monitor_cpu = {case: statistics.median(r["threads"].get("base-native-mon", {}).get("cpu_seconds", 0) for r in costs[(case, "now")]) for case in sorted({key[0] for key in costs if key[0].startswith("periodic")})}
print("\nThe monitor thread's own CPU, median per run with stall rescue on (10 ms resolution): " + "; ".join(f"{case}: {value:.2f} s" for case, value in monitor_cpu.items()) + ".\n")

table({k: v for k, v in costs.items() if k[0].startswith("pulse")}, {
    "process CPU ms": lambda r: r["record"]["cpu_ns_per_op"] * r["record"]["iterations"] / 1e6,
    "monitor wakes": lambda r: r["threads"].get("base-native-mon", {}).get("voluntary", 0),
}, "Pulse", "A task woken by a timer once a second, 3 times per run.")

over = collections.defaultdict(list); counts = collections.defaultdict(list)
for line in open(directory / "oversub.jsonl"):
    r = json.loads(line)
    key = (f"{r['condition']}, {r['backend']}", r["build"])
    (counts if r["build"] == "counts" else over)[key].append(r["data"])
table(over, {
    "p50 lateness µs": lambda d: d["lateness_p50_us"],
    "p99 lateness µs": lambda d: d["lateness_p99_us"],
    "process CPU s": lambda d: d["cpu_seconds"],
}, "Oversubscription", "From `oversub.jsonl`: the periodic diagnostic (1,024 tasks, 1 ms) with 8 workers on an idle host, 8 workers beside four busy-loop processes, and 4 workers pinned to CPUs 0 and 1; 9 repeats.")
print("Rescues in a `diagnostics` build, 3 runs per condition (stuck polls / long batches / timers rescued, and rescued timers as a share of expiries; labels from the classification before decision 16):\n")
print("| Condition | Runs |")
print("| --- | --- |")
for (case, _), runs in sorted(counts.items()):
    print(f"| {case} | " + "; ".join(f"{d['rescues']['stuck_polls']} / {d['rescues']['long_batches']} / {d['rescues']['timers']} ({d['rescues']['timers'] / d['samples'] * 100:.3f}%)" for d in runs) + " |")

print("\n## Timers fired per expiry pass\n")
print("From `batches.jsonl`: the readiness selector's periodic diagnostic with builds instrumented by `batches-old.patch` (before) and `batches-new.patch` (on); 9 repeats. Bucket *b* counts expiry passes that fired from 2^(b−1) to 2^b − 1 timers; the values are means per run.\n")
print("| Workers | Build | p50 lateness µs (range) | Passes per run (range) | 16–31 | 32–63 | 64–127 | 128–255 |")
print("| --- | --- | --- | --- | --- | --- | --- | --- |")
batches = collections.defaultdict(list)
for line in open(directory / "batches.jsonl"):
    r = json.loads(line); batches[(r["workers"], r["build"])].append(r["record"])
for (workers, build), runs in sorted(batches.items()):
    mean = [sum(r["batches"][i] for r in runs) / len(runs) for i in range(16)]
    label = {"m2h": "before", "nowh": "on"}[build]
    passes = [sum(r["batches"]) for r in runs]; p50 = [r["lateness_p50_us"] for r in runs]
    print(f"| {workers} | {label} | {statistics.median(p50):.1f} ({min(p50):.1f}–{max(p50):.1f}) | {sum(mean):,.0f} ({min(passes):,}–{max(passes):,}) | {mean[5]:,.0f} | {mean[6]:,.0f} | {mean[7]:,.0f} | {mean[8]:,.0f} |")
