# Stall rescue causes

These results explain the stuck-poll rescues that [rescue-counts](../rescue-counts/readme.md) recorded in workloads without long polls. They were listed as an open question in [stall rescue](../../../../design/stall-rescue.md), now decision 16. They answer three questions:

- what each worker was doing when the monitor marked it stuck;
- why workers leaving idle were slow to resume;
- whether waking an idle worker ever fails to interrupt its wait.

The instrumentation is an experiment, not merged. [experiment.patch](experiment.patch) applies to `main` at `793cbbf`. With the `diagnostics` feature, it logs one JSON line per event to the file named by `BASE_NATIVE_RESCUE_LOG`:

- **Mark:** when the monitor marks a worker stuck. It records the worker's phase (timer pass, work search, idle, driver turn or tasks), read before anything is rescued; how long the worker had been in that phase; and how overdue its deadline was.
- **Clear:** when the mark clears. It records the mark's duration; the worker thread's CPU time and run-queue delay over it, from `/proc/self/task/{tid}/schedstat`; its voluntary and involuntary context switches, from `.../sched`; the task polls it completed; and whether any had completed by the monitor's next pass.
- **Rescue:** every rescue that fires a timer, by the monitor or by an idle worker, with the label `RescueStatistics` gives it.

The patch also adds `wake_latency`. It parks a task on an idle worker with no timers pending, so a missed wake would wait indefinitely, then wakes it from another thread. `thread` runs the same loop with a plain thread and `park`/`unpark`, and `tokio` under Tokio's runtime, as baselines.

Hana: Linux 7.2.14, Intel Xeon E3-1240 v6 (4 cores, 8 threads), `powersave` governor, `CONFIG_HZ=1000`, rustc 1.99.1, release builds, unpinned. Every run held `/tmp/base-scheduler-perf.lock`, and nothing else of ours ran on the host.

```sh
git apply benchmarks/timers/results/rescue-causes/experiment.patch
cargo build --release --locked -p base-scheduler-benchmark-timers --features diagnostics --target-dir target-diagnostics
cargo build --release --locked -p base-scheduler-benchmark-timers --target-dir target-plain
D=benchmarks/timers/results/rescue-causes
flock /tmp/base-scheduler-perf.lock $D/periodic-run.sh target-diagnostics/release/base-scheduler-benchmark-timers events.jsonl 12
flock /tmp/base-scheduler-perf.lock $D/wake-run.sh target-plain/release/wake_latency wake-latency.jsonl
python3 $D/summarize.py $D > $D/summary.md
```

The wake test uses the build without `diagnostics`, since its latencies are timing.

| File | Contents |
| --- | --- |
| [experiment.patch](experiment.patch) | The instrumentation and `wake_latency`, against `793cbbf`. |
| [periodic-run.sh](periodic-run.sh) | Runs `periodic {native,iouring} 4 1024 1000 {unaligned,aligned}` with logging, writing each run's record and events as one line. |
| [events.jsonl](events.jsonl) | 48 runs, 12 of each backend and alignment. |
| [wake-run.sh](wake-run.sh) | Runs `wake_latency` 20,000 times for each backend, worker count and mode, on an otherwise idle host and then beside three busy-loop processes. |
| [wake-latency.jsonl](wake-latency.jsonl) | One line per run: the host condition and the program's record. |
| [summarize.py](summarize.py), [summary.md](summary.md) | The summary, generated from the two record files. |

## Result

Numbers below are from [summary.md](summary.md).

**Every marked worker was leaving idle or had completed task polls by the time its mark cleared.** The 48 runs recorded 92 marks:

- **Leaving idle (50 marks):** the worker was in idle code and no longer sleeping. The validation exempts this case.
- **Long batch or turn (42 marks):** task polls had completed by the monitor's next pass while the worker was still marked (5 marks), or by the time the mark cleared (37 marks).
- **No progress (none):** no mark cleared without the worker completing a poll.

Today's label called 40 of the 92 marks stuck polls: 28 workers leaving idle and 12 long batches. All 196 rescues that fired a timer were logged, matching `RescueStatistics` exactly: 67 by the monitor and 129 by idle workers' pre-park checks.

This does not show that no poll was stuck for longer than a period. Eleven of the 37 marks classified at the clear lasted more than 5 ms. In the longest, the monitor and the workers were starved together: one pass marked all four workers, and the next came 25 ms later. A poll stuck during such a stretch, followed by completed polls before the clear, counts as a long batch, so stuck polls can be undercounted.

**Most workers leaving idle were waiting for a CPU.** Over the 50 marks, the median worker was marked for 1,963 µs, ran for 843 µs, and accrued 2,202 µs of run-queue delay.
- **Waiting (40 marks):** the run-queue delay was at least half the mark. The longest marks, up to 38 ms, show the threads were runnable but not run. What occupied the CPUs then was not recorded: it could have been other load on the host, or a cgroup CPU limit.
- **Running (8 marks):** run-queue delay was under 50 µs. Those workers were probably running: in their pre-park checks, where a rescue can schedule a task onto the rescuing worker itself and clear its sleeping flag, or just resumed and not yet past their next phase marker.
- **In between (2 marks):** some run-queue delay, but less than half the mark.
- **The comparison is approximate:** run-queue delay is credited when a thread reaches a CPU, so it includes waiting that began before the mark, and in 22 marks it exceeds the mark itself.

**No wake is lost, and Native's wake latency is in the same range as a plain thread's and Tokio's.**
- **Lost wakes:** none of the 560,000 wakes was lost, in either mode, on an idle host or beside three busy-loop processes. A lost wake would hang the program, so every completed run lost none.
- **Spin mode:** it busy-waits 0 to 30 µs before each wake, intended to land many wakes during the worker's transition to idle; how many did was not counted.
- **Medians:**
  - On an idle host in sleep mode, 64 µs for the plain thread, 74 to 85 µs for Tokio and Native's readiness selector, and about 32 µs on io_uring. Cores enter deep sleep states under `powersave` while nothing runs.
  - On a busy host, or in spin mode, 3 to 8 µs for every backend.
- **Tail:** wakes over 1 ms are rare, at most 2 in 20,000 per run. They occur in Tokio and the plain thread as well as Native.

## Limits

- One host, one workload: the 1,024-timer periodic diagnostic, which collects about 256 timers per worker per millisecond, so long batches are frequent. The single stuck-poll rescues in other workloads in [rescue-counts](../rescue-counts/readme.md) were not instrumented.
- The instrumentation reads the clock at every phase change, which may change how often rescues happen.
- What occupied the CPUs during the longest run-queue delays was not recorded. `cpu.stat`'s `nr_throttled` would show a cgroup CPU limit.
- The wake test records no process CPU time.
