#!/usr/bin/env python3
# fsum.py FILE: medians of fixed-rate h2load runs per configuration, A against P.
import json, statistics, sys
from collections import defaultdict
runs = defaultdict(list)
for line in open(sys.argv[1]):
    r = json.loads(line)
    runs[(r["backend"], r["conns"], r["offered"], r["bin"])].append(r)
for backend, conns, offered in sorted({k[:3] for k in runs}):
    cells = []
    for b in "AP":
        rs = runs[(backend, conns, offered, b)]
        m = {k: statistics.median(r[k] for r in rs) for k in ("rps", "mean", "sd", "max")}
        cells.append(f"{b} n={len(rs)} rps={m['rps']:.0f} mean={m['mean']:.0f} sd={m['sd']:.0f} max={m['max']:.0f}")
    print(f"{backend:18} {conns:4} {offered:7}  " + "  ".join(cells))
