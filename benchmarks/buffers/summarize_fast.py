#!/usr/bin/env python3
"""Print matched median comparisons for pool and receive fast-path samples."""

import argparse
import json
import statistics
from collections import defaultdict


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("samples")
    args = parser.parse_args()
    with open(args.samples) as source:
        rows = [json.loads(line) for line in source]
    leases = "leases_per_second" in rows[0]
    servers = "connections" in rows[0]
    groups = defaultdict(dict)
    for row in rows:
        if leases:
            key = (row["threads"], row["shared"], row["frozen"], row["repeat"])
            label = row["implementation"]
            metric = "leases_per_second"
        elif servers:
            key = (row["workers"], row["connections"], row["transport"],
                   row["size"], row["mode"], row["repeat"])
            label = row["implementation"]
            metric = "messages_per_second"
        else:
            key = (row["workers"], row["traffic"], row["transport"],
                   row["size"], row["window"], row["repeat"])
            label = (row["implementation"], row["backend"], row["mode"])
            metric = "messages_per_second"
        assert label not in groups[key], f"duplicate sample: {key}, {label}"
        groups[key][label] = row[metric]
    print(f"Samples: {len(rows)}")
    if leases or servers:
        prefix = "threads shared frozen" if leases else "workers connections transport size mode"
        print(f"{prefix} mutex_M/s bitmap_M/s bitmap/mutex")
        pairs = [("bitmap", "mutex")]
    else:
        print("workers traffic transport size window pool_new/old retained_new/old multi_new/old multi/raw multi/tokio pool/raw")
        pairs = [
            (("after", "iouring", mode), ("before", "iouring", mode))
            for mode in ["pool", "shared", "multi"]
        ] + [
            (("after", "iouring", "multi"), ("after", backend, "raw"))
            for backend in ["iouring", "tokio"]
        ] + [(("after", "iouring", "pool"), ("after", "iouring", "raw"))]
    for key in sorted({key[:-1] for key in groups}):
        matched = [values for group, values in groups.items() if group[:-1] == key]
        ratios = [statistics.median(values[left] / values[right] for values in matched)
                  for left, right in pairs]
        rates = ([statistics.median(values[mode] for values in matched) / 1e6
                  for mode in ["mutex", "bitmap"]] if leases or servers else [])
        print(*key, *(f"{number:.3f}" for number in rates + ratios))
    print("Overall matched median ratios:", *(
        f"{statistics.median(values[left] / values[right] for values in groups.values()):.3f}"
        for left, right in pairs
    ))


if __name__ == "__main__":
    main()
