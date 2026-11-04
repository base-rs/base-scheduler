# Stall rescue counts

These results check the rescue classification counter in [stall rescue](../../../../design/stall-rescue.md)'s validation. The named benchmarks contain no long polls, so the design expects no stuck-poll rescues, and counts any as false positives. They are also the per-workload rescue counts that the design's budget section asks to be reported.

The scheduler code is `main` at `0b4c540` (stall rescue, #93). The harness is this directory's pull request: the timer benchmark's `scenario` and `periodic` modes, and the timer and echo benchmarks built with `--features diagnostics`. Timing from these builds is not a measurement.

Hana: Linux 7.2.14, Intel Xeon E3-1240 v6 (4 cores, 8 threads), rustc 1.99.1, release builds. The echo server ran on CPUs 0, 4, 1 and 5 and the client on 2, 6, 3 and 7. Every run held `/tmp/base-scheduler-perf.lock`, and nothing else ran on the host.

| File | Contents |
| --- | --- |
| [counts.py](counts.py) | Runs every workload of the stage 2 acceptance run once per repeat, in a shuffled order, plus `scenario stall`, a control that must record rescues. Run here with `--repeats 3`. |
| [counts.jsonl](counts.jsonl) | 198 records, one per run, each with the run's full output. Timer records carry `rescues`; echo records carry the server's as `server_rescues`. |
| [summarize.py](summarize.py) | Generates the summary from the records. |
| [summary.md](summary.md) | Rescue counts per workload, one value per repeat. |
| [context-switches.py](context-switches.py), [context-switches.txt](context-switches.txt) | Six further unaligned native periodic runs, each with its rescue counts and its worker threads' involuntary context switches. |

## Result

- **Echo matrix:** no rescues of any kind on any backend, configuration or rate, including the idle cases.
- **Scenarios:** none for spawning, with or without a root timer pending, idle, pulse or root sleeps. `task-sleep` recorded 3 stuck-poll rescues (10 timers) in one of its three runs. The `stall` control recorded 12 stuck-poll rescues and 12 rescued timers in every run, as designed: 4 per 200 ms stall.
- **Timer workloads:** none, except one io_uring `reuse expire 128/4` run with 1 stuck-poll rescue (39 timers).
- **Periodic timers, 1,024 tasks on 4 workers:** rescues in most runs on both selectors, aligned and unaligned:

  | Workload | Stuck polls | Long batches | Timers rescued |
  | --- | --- | --- | --- |
  | native, unaligned | 10 / 0 / 3 | 6 / 0 / 0 | 1,743 / 0 / 262 |
  | native, aligned | 2 / 0 / 3 | 5 / 3 / 3 | 1,120 / 299 / 1,256 |
  | io_uring, unaligned | 2 / 0 / 1 | 1 / 0 / 0 | 519 / 0 / 192 |
  | io_uring, aligned | 3 / 0 / 0 | 0 / 0 / 0 | 801 / 0 / 0 |

  The single-task, 10 µs periodic workload recorded none.

So the stage 2 expectation of zero stuck-poll rescues holds for the echo matrix, the scenarios and the timer microbenchmarks, apart from two single runs. It does not hold for the 1,024-timer periodic workload, where they are false positives by the design's definition.

## Cause: not determined

None of these workloads blocks inside a poll. In the periodic workload, each of the 4 workers wakes about 256 tasks every millisecond, so a worker that stays in one batch, or is descheduled, for more than *S* = 1 ms has due timers taken over by the monitor. Rescued timers come in multiples of about 256, the size of one expiry batch.

[Six further unaligned native runs](context-switches.txt) recorded 53 to 92 involuntary context switches across the worker threads per run, without tracking the rescues: the run with 53 rescued 1,101 timers, and runs with 88 to 92 rescued 212 to 256. Preemption alone therefore does not explain them. The stuck-poll and long-batch split is approximate, as `RescueStatistics` documents, because it compares task polls with the monitor's previous pass.

They are rare: at most 0.06% of a run's 2.8 to 3.1 million measured expiries (1,743 of 2,838,325), and none in 4 of the 12 runs. That is far too few to explain the workload's +26% p50 lateness in the [acceptance run](../../../echo/results/stall-rescue/readme.md), which therefore has another cause.

## Not measured

- Rescue counts per registration, which decision 7 would use to revisit migrating registrations.
