# Stall rescue causes

These results explain the stuck-poll rescues that [rescue-counts](../rescue-counts/readme.md) recorded in workloads without long polls, listed as an open question in [stall rescue](../../../../design/stall-rescue.md#open-questions). They answer two questions: what each worker was doing when the monitor marked it stuck, and whether waking an idle worker ever fails to interrupt its wait.

The instrumentation is an experiment, not merged: [experiment.patch](experiment.patch) applies to `main` at `793cbbf`. With the `diagnostics` feature, it logs each stuck mark as one JSON line to the file named by `BASE_NATIVE_RESCUE_LOG`:

- **When a mark begins:** the worker's phase (timer pass, work search, idle, driver turn or tasks), how long it had been in that phase, how overdue its deadline was, and the timers rescued.
- **When the mark clears:** its duration, the worker thread's CPU time and involuntary context switches over it, from `/proc/self/task/{tid}/sched`.
- **When a worker leaves idle after a wake from another thread:** if that took more than 250 µs, the latency and its relation to the worker's own idle timeout.

The patch also adds `wake_latency`, a test of lost wakes described below.

Hana: Linux 7.2.14, Intel Xeon E3-1240 v6 (4 cores, 8 threads), `powersave` governor, `CONFIG_HZ=1000`, `kernel.sched_schedstats=0`, rustc 1.99.1, release builds, unpinned. Every run held `/tmp/base-scheduler-perf.lock`.

| File | Contents |
| --- | --- |
| [experiment.patch](experiment.patch) | The instrumentation and `wake_latency`, against `793cbbf`. |
| [periodic-run.sh](periodic-run.sh) | Runs `periodic {native,iouring} 4 1024 1000 {unaligned,aligned}` with logging, as `periodic-run.sh BINARY OUTPUT 6`. |
| [phases.jsonl](phases.jsonl) | 24 runs with the phase and CPU-time logging: each run's record and its events. |
| [wakes.jsonl](wakes.jsonl) | 24 further runs that also log slow wakes and each worker's wake count. |
| [wake-latency.jsonl](wake-latency.jsonl) | The lost-wake test's output, one line per configuration, labelled with whether the host was idle or loaded. |
| [summarize.py](summarize.py), [summary.md](summary.md) | The summary, generated from the three record files. |

## Result

Numbers below are from [summary.md](summary.md).

**No worker was stuck inside a poll.** All 37 marks labelled stuck poll, and all 26 labelled long batch, fall into these groups:
- **Idle code after a wake from another thread (26 marks):** the worker had been woken, so it was no longer sleeping, but had not yet resumed. The validation exempts this case.
- **Task batch or driver turn (28 marks):** 8 had been running tasks for more than 1 ms. 19 had entered the phase after the monitor read the clock, so their previous turn had run long.
- **Timer pass or work search (9 marks).**

The stuck-poll label compares task polls with the monitor's previous pass, which can be long before. Here it disagrees with the worker's actual phase in most marks. The label is wrong; the marks are not: in every one, a timer on that worker was more than *S* = 1 ms overdue, which is what rescue is for.

**Waking an idle worker works.** In the periodic workload:
- 495 of 673,457 wakes from other threads (0.074%) took more than 250 µs to resume the worker. They were most common with aligned deadlines, where every timer falls due at once on a 4-core host (0.35% and 0.74%).
- 60 matched a missed-wake signature: woken well before the worker's own idle timeout, but resumed within 100 µs of it.
- The longest delays, 2 to 20 ms, resumed long after both the wake and the worker's own timeout. A lost interrupt would still have ended at the timeout, so these were threads waiting for a CPU, not sleeping through a wake.

The `wake_latency` test settles this. It parks a task on an idle worker with no timers pending, so a missed wake would wait indefinitely, then wakes it from another thread after 50 to 500 µs. That ran 20,000 times for each selector and for 1 and 4 workers, on an idle host and again beside three busy-loop processes. No wake was lost and none took more than 10 ms. On an idle host the median wake was 75 to 80 µs on the readiness selector and 30 µs on io_uring. Under load it was 7 µs on both, consistent with idle cores entering deep sleep states under `powersave`.

## Limits

- **CPU-time fractions over a mark** are reliable only for long marks: the kernel updates a running thread's CPU time at ticks and context switches, every 1 ms here. The five marks longer than 10 ms show workers that barely ran. Without scheduler statistics, a thread waiting in the run queue looks like a sleeping one, so these do not separate preemption from run-queue delay; enabling `kernel.sched_schedstats` would.
- **The phase is read after the rescue,** a little after the overdue deadline was read, which is why 19 marks show a phase just entered.
- **The instrumentation reads the clock at every phase change,** which may change how often rescues happen.
- **One host, one workload.**
