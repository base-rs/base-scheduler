#!/usr/bin/env python3
"""Summarize the stall rescue cause experiment as Markdown: every stuck mark and rescue, the proposed classification, and the wake latency test."""
import collections, json, pathlib, sys

directory = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else pathlib.Path(__file__).parent)
runs = [json.loads(line) for line in open(directory / "events.jsonl")]

def pairs(run):
    # Each mark with the clear that ends it, if the run logged one.
    open_marks, out = {}, []
    for event in run["events"]:
        if event["event"] == "mark":
            open_marks[event["worker"]] = event
        elif event["event"] == "clear" and event["worker"] in open_marks:
            out.append((open_marks.pop(event["worker"]), event))
    out.extend((mark, None) for mark in open_marks.values())
    return out

def classify(mark, clear):
    # The proposed classification: a worker in idle code that is no longer sleeping is leaving idle, which is exempt; otherwise poll progress by the next pass, or by the clear, separates a long batch from a stuck poll.
    if mark["phase"] == "idle":
        return "leaving idle"
    if clear is None:
        return "unknown (no clear logged)"
    if clear["progressed_next"] is True or (clear["progressed_next"] is None and clear["polls"] > 0):
        return "long batch or turn"
    if mark["phase"] != "tasks":
        return f"no progress, in {mark['phase']}"
    return "stuck poll"

marks = [(run["case"], mark, clear) for run in runs for mark, clear in pairs(run)]
print(f"## Stuck marks\n\n{len(runs)} runs of the periodic workload (`events.jsonl`), {len(marks)} marks. The phase is read when the monitor marks the worker, before it rescues anything.\n")
print("| Phase | Marks | Labelled stuck poll | Labelled long batch |")
print("| --- | --- | --- | --- |")
phases = collections.Counter((mark["phase"], mark["kind"]) for _, mark, _ in marks)
for phase in ["idle", "tasks", "driver", "search", "timers"]:
    poll, batch = phases[(phase, "poll")], phases[(phase, "batch")]
    if poll + batch:
        print(f"| {phase} | {poll + batch} | {poll} | {batch} |")
print(f"| all | {len(marks)} | {sum(v for (_, k), v in phases.items() if k == 'poll')} | {sum(v for (_, k), v in phases.items() if k == 'batch')} |\n")

print("### Proposed classification\n")
print("| Class | Marks | Labelled stuck poll today |")
print("| --- | --- | --- |")
classes = collections.Counter(); labelled = collections.Counter()
for _, mark, clear in marks:
    c = classify(mark, clear); classes[c] += 1; labelled[c] += mark["kind"] == "poll"
for c, n in classes.most_common():
    print(f"| {c} | {n} | {labelled[c]} |")

progressing = [clear for _, mark, clear in marks if clear and classify(mark, clear) == "long batch or turn"]
sampled = sum(1 for clear in progressing if clear["progressed_next"] is not None)
long_marks = sum(1 for clear in progressing if clear["progressed_next"] is None and clear["marked_ns"] > 5_000_000)
print(f"\nOf the {len(progressing)} long batches or turns, {sampled} were sampled at a later monitor pass while still marked; the other {len(progressing) - sampled} cleared first and are classified by the polls completed by the clear, {long_marks} of them after more than 5 ms.")

print("\n### Marks without poll progress\n")
print("Scheduler counters over each mark: CPU time (`run`), run-queue delay (`delay`), and voluntary and involuntary switches.\n")
print("| Case | Phase | Marked µs | Run µs | Delay µs | Voluntary | Involuntary |")
print("| --- | --- | --- | --- | --- | --- | --- |")
for case, mark, clear in marks:
    if clear and classify(mark, clear) not in ("leaving idle", "long batch or turn"):
        print(f"| {case[9:]} | {mark['phase']} | {clear['marked_ns'] / 1e3:.0f} | {clear['run_ns'] / 1e3:.0f} | {clear['delay_ns'] / 1e3:.0f} | {clear['voluntary']} | {clear['involuntary']} |")

print("\n### Workers leaving idle\n")
woken = [(mark, clear) for _, mark, clear in marks if mark["phase"] == "idle" and clear]
if woken:
    import statistics
    median = lambda key: statistics.median(clear[key] for _, clear in woken) / 1e3
    queued = sum(1 for _, clear in woken if clear["delay_ns"] >= clear["marked_ns"] / 2)
    running = sum(1 for _, clear in woken if clear["delay_ns"] < 50_000)
    over = sum(1 for _, clear in woken if clear["delay_ns"] > clear["marked_ns"])
    print(f"{len(woken)} of these marks have a logged clear. Over each mark, the median worker was marked for {median('marked_ns'):.0f} µs, ran for {median('run_ns'):.0f} µs and accrued {median('delay_ns'):.0f} µs of run-queue delay. In {queued} of them the run-queue delay was at least half the mark, and in {running} it was under 50 µs: those workers were probably running, in their pre-park checks or just resumed. Run-queue delay is credited when a thread reaches a CPU, so it includes waiting that began before the mark; in {over} marks it exceeds the mark itself.")
print("\n## Rescues\n")
logged = collections.Counter(); counted = collections.Counter()
for run in runs:
    for event in run["events"]:
        if event["event"] == "rescue":
            logged[(event["by"], event["kind"])] += 1
    rescues = run["record"]["rescues"]
    counted["poll"] += rescues["stuck_polls"]; counted["batch"] += rescues["long_batches"]
print(f"Every rescue that fired a timer is logged: {sum(logged.values())} logged against {counted['poll'] + counted['batch']} counted by `RescueStatistics`.\n")
print("| Rescuer | Labelled stuck poll | Labelled long batch |")
print("| --- | --- | --- |")
for by in ["monitor", "worker"]:
    print(f"| {by} | {logged[(by, 'poll')]} | {logged[(by, 'batch')]} |")

print("\n## Wake latency\n")
print("From `wake-latency.jsonl`: `wake_latency BACKEND WORKERS 20000 MODE`. `thread` is a plain thread woken with `park` and `unpark`, with no scheduler. `sleep` waits 50 to 500 µs before each wake; `spin` busy-waits 0 to 30 µs, so wakes race the worker's transition to idle. A lost wake would hang the program, so every completed run lost none.\n")
print("| Host | Mode | Backend | Workers | Over 1 ms | Over 10 ms | p50 µs | p99 µs | p99.9 µs | Max µs |")
print("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |")
for line in map(json.loads, open(directory / "wake-latency.jsonl")):
    r = line["record"]
    print(f"| {line['condition']} | {r['mode']} | {r['backend']} | {r['workers']} | {r['over_1ms']} | {r['over_10ms']} | {r['p50_us']:.1f} | {r['p99_us']:.1f} | {r['p999_us']:.0f} | {r['max_us']:.0f} |")
