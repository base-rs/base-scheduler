#!/usr/bin/env python3
"""Summarize throughput and matched ratios from measure.py JSON lines."""

import argparse
import json
import statistics
from collections import defaultdict


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("samples")
    arguments = parser.parse_args()
    with open(arguments.samples) as source:
        rows = [json.loads(line) for line in source]
    groups = defaultdict(dict)
    for row in rows:
        key = (row["binary"], row["backend"], row["workers"], row["size"], row.get("transport"), row["repeat"])
        metric = "gib_per_second" if row["binary"] == "files" else "round_trips_per_second"
        assert row["mode"] not in groups[key], f"duplicate sample: {key}"
        groups[key][row["mode"]] = row[metric]

    print(f"Samples: {len(rows)}; source: {sorted({row.get('source', 'unrecorded') for row in rows})}")
    print("64 KiB file throughput (GiB/s):")
    print("backend workers serial join submit join/serial submit/join join/vec-join")
    for workers in [1, 2]:
        for backend in ["direct", "native", "tokio", "iouring"]:
            samples = [values for key, values in groups.items() if key[:4] == ("files", backend, workers, 65536)]
            medians = [statistics.median(sample[mode] for sample in samples) for mode in ["serial", "join", "submit"]]
            ratios = [statistics.median(sample[left] / sample[right] for sample in samples) for left, right in [("join", "serial"), ("submit", "join"), ("join", "vec-join")]]
            print(backend, workers, *(f"{value:.3f}" for value in medians + ratios))
    print("Network matched ratios (pooled/raw, retained/raw):")
    for backend in ["native", "tokio", "iouring"]:
        for workers in [1, 2]:
            for transport, size in [("tcp", 64), ("tcp", 65536), ("udp", 1200)]:
                samples = [values for key, values in groups.items() if key[:5] == ("sockets", backend, workers, size, transport)]
                ratios = [statistics.median(sample[mode] / sample["raw"] for sample in samples) for mode in ["pool", "shared"]]
                print(backend, workers, transport, size, *(f"{value:.3f}" for value in ratios))
    samples = [values for key, values in groups.items() if key[0] == "sockets"]
    print("Overall network ratios:", *(f"{statistics.median(sample[mode] / sample['raw'] for sample in samples):.3f}" for mode in ["pool", "shared"]))
    print("4 KiB submitted/joined ratios:")
    for backend in ["direct", "native", "tokio", "iouring"]:
        for workers in [1, 2]:
            samples = [values for key, values in groups.items() if key[:4] == ("files", backend, workers, 4096)]
            print(backend, workers, f"{statistics.median(sample['submit'] / sample['join'] for sample in samples):.3f}")


if __name__ == "__main__":
    main()
