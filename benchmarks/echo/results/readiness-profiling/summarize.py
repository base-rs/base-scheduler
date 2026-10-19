#!/usr/bin/env python3
"""Summarize the retained samples; percentages are self rows, never callgraph sums."""

import json
from pathlib import Path
import re
import statistics


ROOT = Path(__file__).resolve().parent
GROUPS = {
    "oneshot": lambda symbol: "futures_channel::oneshot" in symbol,
    "admission": lambda symbol: "Capacity>::acquire" in symbol or "Permit as core::ops::drop::Drop>::drop" in symbol,
    "event_listener": lambda symbol: "event_listener::" in symbol,
    "worker_queues": lambda symbol: "crossbeam_deque::" in symbol or "crossbeam_epoch::" in symbol,
    "wake_idle": lambda symbol: "Shared>::wake_idle" in symbol,
    "route": lambda symbol: "IoUringSelector>::route" in symbol,
    "reactor_turn": lambda symbol: "Reactor>::turn" in symbol,
    "wait_ready": lambda symbol: "IoUringSelector>::wait_ready" in symbol,
    "worker_loop": lambda symbol: symbol.startswith("core_scheduler_native::worker::run_loop"),
}


def spread(values):
    return dict(median=statistics.median(values), min=min(values), max=max(values))


def summarize(directory):
    samples = json.loads((directory / "aiko.json").read_text())["samples"]
    groups = {}
    for sample in samples:
        key = f"{sample['mode']}-{sample['backend']}-w{sample['workers']}-c{sample['connections']}-b{sample['bytes']}"
        groups.setdefault(key, []).append(sample)
    result = {}
    for key, values in sorted(groups.items()):
        item = {field: spread([sample[field] for sample in values])
                for field in ["requests_per_second", "rtt_mean_us"]}
        item["server_cpu"] = {field: spread([sample["server_cpu"][field] for sample in values])
                              for field in ["user", "system"]}
        if values[0]["mode"] == "perf":
            profiles, lost = [], []
            for sample in values:
                content = (directory / (sample["identifier"] + ".self.txt")).read_text()
                rows = [(float(match[1]), match[2], match[3]) for line in content.splitlines()
                        if (match := re.match(r"\s*([\d.]+)%\s+\d+\s+\[([k.])\]\s+(.*)", line))]
                profile = {name: sum(percent for percent, mode, symbol in rows if predicate(symbol))
                           for name, predicate in GROUPS.items()}
                profile["kernel_rows"] = sum(percent for percent, mode, symbol in rows if mode == "k")
                profile["fdget_fget_fput"] = sum(percent for percent, mode, symbol in rows
                                               if mode == "k" and symbol.split()[0] in ["fdget", "fget", "fput"])
                profiles.append(profile)
                lost.append(int(re.search(r"Total Lost Samples: (\d+)", content)[1]))
            item["self_percent"] = {name: spread([profile[name] for profile in profiles]) for name in profiles[0]}
            item["lost_samples"] = lost
        if values[0]["mode"] == "stat":
            counts, coverage = [], []
            for sample in values:
                counters = {}
                for line in (directory / (sample["identifier"] + ".stat")).read_text().splitlines():
                    if not line or line.startswith("#"):
                        continue
                    fields = line.split(";")
                    counters[fields[2]] = float(fields[0]) / sample["requests"]
                    coverage.append(float(fields[4]))
                counts.append(counters)
            item["per_request"] = {name: spread([count[name] for count in counts]) for name in counts[0]}
            item["counter_running_percent"] = spread(coverage)
        result[key] = item
    return result


if __name__ == "__main__":
    print(json.dumps({name: summarize(ROOT / name) for name in [".", "kernel", "syscalls"]}, indent=2))
