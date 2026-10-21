#!/usr/bin/env python3
"""Print paired timer comparisons from run.py JSON lines (wall-clock throughput, not individual operation latency)."""
import json
import statistics
import sys
from collections import defaultdict

cases = defaultdict(dict)
for line in open(sys.argv[1]):
    row = json.loads(line)
    cases[(row["workload"], row["tasks"], row["workers"], row["backend"])].setdefault(row["mode"], {})[row["repeat"]] = row
print("| Workload | Tasks/workers | Backend | Sleep ns/op | Reuse ns/op | After ns/op | Reuse cost reduction (paired median; range) |")
print("| --- | --- | --- | ---: | ---: | ---: | --- |")
for (workload, tasks, workers, backend), modes in sorted(cases.items()):
    values = [statistics.median(row["ns_per_operation"] for row in modes[mode].values()) for mode in ["sleep", "reuse", "after"]]
    paired = [100 * (1 - modes["reuse"][repeat]["ns_per_operation"] / modes["sleep"][repeat]["ns_per_operation"]) for repeat in modes["reuse"]]
    print(f"| {workload} | {tasks}/{workers} | {backend} | {values[0]:,.1f} | {values[1]:,.1f} | {values[2]:,.1f} | {statistics.median(paired):+.1f}% ({min(paired):+.1f}% to {max(paired):+.1f}%) |")
