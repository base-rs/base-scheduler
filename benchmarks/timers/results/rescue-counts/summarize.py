#!/usr/bin/env python3
"""Summarize the rescue counts recorded by `counts.py` as a Markdown table, one row per workload."""
import collections, json, sys

rows = collections.defaultdict(list)
records = collections.defaultdict(list)
for line in open(sys.argv[1]):
    record = json.loads(line)
    data = record["data"]
    rows[record["case"]].append(data.get("server_rescues", data.get("rescues")))
    records[record["case"]].append(data)

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
shares = [(case, record) for case in sorted(rows) for record in records[case] if record.get("samples") and record.get("rescues") is not None]
if shares:
    print("\nRescued timers as a share of measured expiries, per run, for workloads that count expiries:\n")
    print("| Workload | Expiries | Timers rescued | Share |")
    print("| --- | --- | --- | --- |")
    for case, record in shares:
        rescued = record["rescues"]["timers"]
        if rescued:
            print(f"| {case} | {record['samples']:,} | {rescued:,} | {rescued / record['samples'] * 100:.3f}% |")
    print(f"\n{sum(1 for _, record in shares if record['rescues']['timers'] == 0)} of {len(shares)} such runs rescued no timers.")
if missing:
    print(f"\nNo counts recorded for: {', '.join(missing)}. Were the binaries built with `--features diagnostics`?")
