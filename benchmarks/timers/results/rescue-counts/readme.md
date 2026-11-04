# Stall rescue counts

These results check the rescue classification counter in [stall rescue](../../../../design/stall-rescue.md)'s validation. The named benchmarks contain no long polls, so the design expects no stuck-poll rescues, and counts any as false positives. They are also the per-workload rescue counts that the design's budget section asks to be reported.

The binaries were built from commit `f59cba4`: the scheduler code of `main` at `0b4c540` (stall rescue, #93) with this pull request's harness, identical to its commit `37e9206`. They predate the monitor startup fix (`fc987a6`, #97), which changes no counting. Timing from these builds is not a measurement.

Hana: Linux 7.2.14, Intel Xeon E3-1240 v6 (4 cores, 8 threads), `powersave` governor, rustc 1.99.1, release builds. The echo server ran on CPUs 0, 4, 1 and 5 and the client on 2, 6, 3 and 7; the timer benchmark's runs were not pinned. Every run held `/tmp/base-scheduler-perf.lock`, and nothing else ran on the host.

```sh
cargo build --release --locked -p base-scheduler-benchmark-timers -p base-scheduler-benchmark-echo --features base-scheduler-benchmark-timers/diagnostics,base-scheduler-benchmark-echo/diagnostics --target-dir ../target-counts
T=../target-counts/release
flock /tmp/base-scheduler-perf.lock python3 benchmarks/timers/results/rescue-counts/counts.py --timers $T/base-scheduler-benchmark-timers --echo $T/base-scheduler-benchmark-echo --repeats 3 --server-cpus 0,4,1,5 --client-cpus 2,6,3,7 --output counts.jsonl
python3 benchmarks/timers/results/rescue-counts/summarize.py counts.jsonl > summary.md
```

| File | Contents |
| --- | --- |
| [counts.py](counts.py) | Runs every workload of the stage 2 acceptance run once per repeat, in a shuffled order, plus `scenario stall`, a control that must record rescues. Run here with `--repeats 3`. |
| [counts.jsonl](counts.jsonl) | 198 records, one per run, each with the run's full output. Timer records carry `rescues`; echo records carry the server's as `server_rescues`. |
| [summarize.py](summarize.py) | Generates the summary from the records. |
| [summary.md](summary.md) | Rescue counts per workload, one value per repeat, and rescued timers as a share of measured expiries. |
| [context-switches.py](context-switches.py), [context-switches.jsonl](context-switches.jsonl) | Six further unaligned native periodic runs (`periodic native 4 1024 1000 unaligned`), each record with its worker threads' involuntary context switches added as `worker_involuntary_switches`. |

## Result

Numbers below are from [summary.md](summary.md).

- **Echo matrix:** no rescues of any kind on any backend, configuration or rate, including the idle cases.
- **Scenarios:** none for spawning, with or without a root timer pending, idle, pulse or root sleeps. `task-sleep` recorded 3 stuck-poll rescues (10 timers) in one of its three runs. The `stall` control recorded 12 stuck-poll rescues and 12 rescued timers in every run, as designed: 4 per 200 ms stall.
- **Timer workloads:** none, except one io_uring `reuse expire 128/4` run with 1 stuck-poll rescue (39 timers).
- **Periodic timers, 1,024 tasks on 4 workers:** rescues in 8 of 12 runs, on both selectors, aligned and unaligned:

  | Workload | Stuck polls | Long batches | Timers rescued |
  | --- | --- | --- | --- |
  | native, unaligned | 10 / 0 / 3 | 6 / 0 / 0 | 1743 / 0 / 262 |
  | native, aligned | 2 / 0 / 3 | 5 / 3 / 3 | 1120 / 299 / 1256 |
  | io_uring, unaligned | 2 / 0 / 1 | 1 / 0 / 0 | 519 / 0 / 192 |
  | io_uring, aligned | 3 / 0 / 0 | 0 / 0 / 0 | 801 / 0 / 0 |

  The single-task, 10 µs periodic workload recorded none. Rescued timers were at most 0.061% of a run's measured expiries (1,743 of 2,838,325). That share is an upper bound: rescues are counted over the whole run, including the warmup, and expiries only inside the measurement window.

None of these workloads blocks inside a poll. By the design's [validation](../../../../design/stall-rescue.md#validation) and decision 13, their stuck-poll rescues are false positives: in the periodic workload, and in the single `task-sleep` and `reuse expire` runs. Long-batch rescues are an expected effect of load. The expectation of zero stuck-poll rescues therefore holds for the echo matrix, spawning, idle, pulse and root sleeps, but not for the periodic workload or those two runs.

## Cause: not determined

A stuck-poll rescue means that a timer stayed due for more than *S* = 1 ms on a worker that was not sleeping, with no task poll completed on that worker since the monitor's previous pass. Candidate mechanisms, none tested:

- the worker thread was descheduled for more than *S*;
- the worker was in its timer pass or driver turn, where no task poll completes, for more than *S*;
- the classification's comparison with the previous pass, which can be stale, as `RescueStatistics` documents.

[Six further unaligned native runs](context-switches.jsonl) recorded 39 to 171 involuntary context switches across the worker threads per run, without an evident relation to the rescues: runs with 39 and 69 switches rescued nothing, and runs with 79 and 171 rescued 2,059 and 1,483 timers. These are whole-run totals that are not aligned in time with the rescues, so they do not rule preemption in or out.

The rescues are too rare to explain the periodic workload's +26% p50 lateness in the [acceptance run](../../../echo/results/stall-rescue/readme.md).

## Not measured

- Rescue counts per registration, which decision 7 would use to revisit migrating registrations. `RescueStatistics` counts rescues and rescued timers only, so this needs a new diagnostic.
