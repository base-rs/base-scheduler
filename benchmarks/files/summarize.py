#!/usr/bin/env python3
"""Report medians and matched throughput changes from run.py JSONL output."""
import collections
import json
import statistics
import sys
rows = [json.loads(line) for line in open(sys.argv[1])]
groups = collections.defaultdict(list)
for row in rows:
    groups[(row["backend"], row["workload"], row["size"], row["tasks"], row["workers"])].append(row)
print("| Backend | Operation | Bytes | Tasks/workers | Ordinary kops/s | Monitor kops/s | Paired change (range) |")
print("|---|---|---:|---:|---:|---:|---:|")
for (backend, workload, size, tasks, workers), samples in sorted(groups.items()):
    medians = {mode: statistics.median(r["operations_per_second"] for r in samples if r["mode"] == mode)/1000 for mode in ["ordinary", "monitor"]}
    pairs = collections.defaultdict(dict)
    for r in samples:
        pairs[r["pair"]][r["mode"]] = r["operations_per_second"]
    changes = [100*(p["monitor"]/p["ordinary"] - 1) for p in pairs.values()]
    print(f"| {backend} | {workload} | {size} | {tasks}/{workers} | {medians['ordinary']:.1f} | {medians['monitor']:.1f} | {statistics.median(changes):+.1f}% ({min(changes):+.1f}–{max(changes):+.1f}%) |")
