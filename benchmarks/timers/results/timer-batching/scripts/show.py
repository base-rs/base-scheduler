#!/usr/bin/env python3
"""show.py FILE [FIELDS...]: medians per (workers, build), and per-round paired change against the first build named 'before' or the first build."""
import json, statistics, sys
rows = [json.loads(l) for l in open(sys.argv[1])]
fields = sys.argv[2:] or ["lateness_p50_us", "lateness_p99_us", "cpu_seconds", "spread", "passes", "fired", "collect_p50_us", "queue_p50_us", "idle_collect_p50_us", "busy_collect_p50_us", "overshoot_p50_us", "idle_queue_p50_us", "busy_queue_p50_us", "idle_share", "moved_share", "passes", "idle_passes", "sleeps"]
fields = [f for f in fields if f in rows[0]]
builds = list(dict.fromkeys(r["build"] for r in rows))
for w in sorted({r["workers"] for r in rows}):
    print(f"\n## {w} workers ({rows[0]['backend']})\n")
    print("| field | " + " | ".join(builds) + " |")
    print("| --- | " + " | ".join("---:" for _ in builds) + " |")
    for f in fields:
        cells = []
        for b in builds:
            v = [r[f] for r in rows if r["workers"] == w and r["build"] == b]
            cells.append(f"{statistics.median(v):.4g} ({min(v):.3g}–{max(v):.3g})")
        print(f"| {f} | " + " | ".join(cells) + " |")
    base = builds[0]
    for b in builds[1:]:
        for f in ["lateness_p50_us", "cpu_seconds"]:
            by = {}
            for r in rows:
                if r["workers"] == w:
                    by.setdefault(r["round"], {})[r["build"]] = r[f]
            ch = [v[b] / v[base] - 1 for v in by.values() if b in v and base in v]
            if ch:
                print(f"{b} vs {base} {f}: median {statistics.median(ch)*100:+.1f}%, lower in {sum(c < 0 for c in ch)}/{len(ch)}")
