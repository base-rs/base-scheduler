#!/usr/bin/env python3
"""Validate and summarize matched ordinary/fixed buffer samples."""

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
    assert rows, "empty sample file"
    identities = {row["binary_sha256"] for row in rows}
    assert len(identities) == 1, "comparison must use one binary"
    assert len({row["cpus"] for row in rows}) == 1, "CPU affinity changed"
    groups = defaultdict(dict)
    fields = ["kind", "direction", "size", "depth", "ownership", "repeat"]
    for row in rows:
        key = tuple(row[field] for field in fields)
        assert row["mode"] not in groups[key], f"duplicate sample: {key}"
        assert row["seconds"] > 0 and row["gib_per_second"] > 0
        assert row["bytes"] == row["iterations"] * row["depth"] * row["size"]
        groups[key][row["mode"]] = row
    repetitions = {row["repeat"] for row in rows}
    cases = defaultdict(list)
    for key, pair in groups.items():
        assert set(pair) == {"ordinary", "fixed"}, f"incomplete pair: {key}"
        assert pair["ordinary"]["iterations"] == pair["fixed"]["iterations"]
        cases[key[:-1]].append(pair)
    for key, pairs in cases.items():
        assert {pair["fixed"]["repeat"] for pair in pairs} == repetitions, f"missing repetitions: {key}"
    print(f"Samples: {len(rows)}; matched pairs: {len(groups)}; repeats: {len(repetitions)}")
    print(f"Binary SHA-256: {next(iter(identities))}")
    print()
    print("| Kind | Direction | Bytes | Depth | Ownership | Ordinary GiB/s | Fixed GiB/s | Matched fixed/ordinary | Pair range |")
    print("| --- | --- | ---: | ---: | --- | ---: | ---: | ---: | ---: |")
    for key, pairs in sorted(cases.items()):
        rates = [statistics.median(pair[mode]["gib_per_second"] for pair in pairs)
                 for mode in ["ordinary", "fixed"]]
        ratios = [pair["fixed"]["gib_per_second"] / pair["ordinary"]["gib_per_second"]
                  for pair in pairs]
        values = [*key, *(f"{rate:.3f}" for rate in rates),
                  f"{statistics.median(ratios):.3f}×", f"{min(ratios):.3f}–{max(ratios):.3f}"]
        print("| " + " | ".join(map(str, values)) + " |")
    print()
    print("| Bytes | Depth | Registered slots | Registration µs | Unregistration µs |")
    print("| ---: | ---: | ---: | ---: | ---: |")
    setup = defaultdict(list)
    for row in rows:
        if row["mode"] == "fixed":
            setup[row["size"], row["depth"]].append(row)
    for (size, depth), samples in sorted(setup.items()):
        times = [statistics.median(row[field] for row in samples) * 1e6
                 for field in ["registration_seconds", "unregistration_seconds"]]
        print(f"| {size} | {depth} | {depth * 2 + 1} | {times[0]:.2f} | {times[1]:.2f} |")


if __name__ == "__main__":
    main()
