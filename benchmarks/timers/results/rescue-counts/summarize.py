#!/usr/bin/env python3
"""Summarize the rescue counts recorded by `counts.py` as a Markdown table, one row per workload."""
import collections, json, sys

rows = collections.defaultdict(list)
for line in open(sys.argv[1]):
    record = json.loads(line)
    data = record["data"]
    rows[record["case"]].append(data.get("server_rescues", data.get("rescues")))

print("| Workload | Runs | Stuck polls | Long batches | Timers rescued |")
print("| --- | --- | --- | --- | --- |")
missing = []
for case in sorted(rows):
    counts = rows[case]
    if any(count is None for count in counts):
        missing.append(case)
        continue
    cell = lambda key: " / ".join(str(count[key]) for count in counts)
    print(f"| {case} | {len(counts)} | {cell('stuck_polls')} | {cell('long_batches')} | {cell('timers')} |")
if missing:
    print(f"\nNo counts recorded for: {', '.join(missing)}. Were the binaries built with `--features diagnostics`?")
