#!/usr/bin/env python3
"""Summarize the stall rescue cause experiment as Markdown: each stuck mark's worker phase, the slow wake-to-resume latencies, and the lost-wake test."""
import collections, json, pathlib, sys

directory = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else pathlib.Path(__file__).parent)
load = lambda name: [json.loads(line) for line in open(directory / name)]

print("## Stuck marks by worker phase\n")
print("Both periodic runs, 24 runs each: `phases.jsonl` (marks only) and `wakes.jsonl` (marks and wake latencies).\n")
marks = [event | {"case": run["case"]} for name in ["phases.jsonl", "wakes.jsonl"] for run in load(name) for event in run["events"] if event["event"] == "mark"]
counts = collections.Counter((mark["phase"], mark["kind"]) for mark in marks)
print("| Phase when marked | Marks | Labelled stuck poll | Labelled long batch |")
print("| --- | --- | --- | --- |")
for phase in ["idle", "tasks", "driver", "search", "timers"]:
    poll, batch = counts[(phase, "poll")], counts[(phase, "batch")]
    if poll + batch:
        print(f"| {phase} | {poll + batch} | {poll} | {batch} |")
print(f"| all | {len(marks)} | {sum(v for (_, k), v in counts.items() if k == 'poll')} | {sum(v for (_, k), v in counts.items() if k == 'batch')} |")
just_entered = sum(1 for mark in marks if mark["phase"] in ("tasks", "driver") and mark["phase_ns"] == 0)
long_tasks = sum(1 for mark in marks if mark["phase"] == "tasks" and mark["phase_ns"] > 1_000_000)
print(f"\nOf the marks in the tasks or driver phase, {just_entered} had entered it after the monitor read the clock (time in phase 0), and {long_tasks} had been running tasks for more than 1 ms.")
long_marks = [run_event for name in ["phases.jsonl", "wakes.jsonl"] for run in load(name) for run_event in run["events"] if run_event["event"] == "clear" and run_event["marked_ns"] > 10_000_000]
for clear in long_marks:
    print(f"\nA mark lasting {clear['marked_ns'] / 1e6:.1f} ms: the worker ran for {clear['run_ns'] / clear['marked_ns'] * 100:.1f}% of it, with {clear['involuntary']} involuntary switches.", end="")
print("\n")

print("## Slow wakes in the periodic workload\n")
print("From `wakes.jsonl`. A slow wake took more than 250 µs from another thread's wake to the worker leaving idle. The missed-wake signature is a wake more than 200 µs before the worker's own idle timeout, followed by a resume within 100 µs of that timeout.\n")
print("| Backend, deadlines | Wakes | Slow | Missed-wake signature | Late after both |")
print("| --- | --- | --- | --- | --- |")
totals = collections.Counter(); slow = collections.Counter(); signature = collections.Counter(); late = collections.Counter()
for run in load("wakes.jsonl"):
    words = run["case"].split()
    key = f"{words[1]}, {words[3]}"
    for event in run["events"]:
        if event["event"] == "wake_summary":
            totals[key] += event["wakes"]
        elif event["event"] == "slow_wake":
            slow[key] += 1
            after = event["resumed_minus_timeout_ns"]
            if after is None:
                continue
            timeout = event["idle_ns"] - after
            if event["woken_after_ns"] < timeout - 200_000:
                if abs(after) <= 100_000:
                    signature[key] += 1
                else:
                    late[key] += 1
for key in sorted(totals):
    print(f"| {key} | {totals[key]:,} | {slow[key]} ({slow[key] / totals[key] * 100:.3f}%) | {signature[key]} | {late[key]} |")
print(f"| all | {sum(totals.values()):,} | {sum(slow.values())} ({sum(slow.values()) / sum(totals.values()) * 100:.3f}%) | {sum(signature.values())} | {sum(late.values())} |\n")

print("## Lost-wake test\n")
print("From `wake-latency.jsonl`: `wake_latency BACKEND WORKERS 20000`, a task woken from another thread on an idle worker with no timers pending, so a missed wake would wait indefinitely.\n")
print("| Host | Backend | Workers | Wakes | Lost (over 2 s) | Over 10 ms | Over 1 ms | p50 µs | p99 µs | Max µs |")
print("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |")
for line in load("wake-latency.jsonl"):
    r = line["record"]
    print(f"| {line['condition']} | {r['backend']} | {r['workers']} | {r['wakes']:,} | {r['lost_over_2s']} | {r['over_10ms']} | {r['over_1ms']} | {r['p50_us']:.1f} | {r['p99_us']:.1f} | {r['max_us']:.0f} |")
