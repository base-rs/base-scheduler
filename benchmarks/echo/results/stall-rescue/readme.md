# Stall rescue acceptance measurements

These results check stage 2 of [stall rescue](../../../../design/stall-rescue.md) (timers) against its performance budget. The baseline is `main` at `7b158df`; the branch is `7adde39`, whose only later change is test code.

Hana: Linux 7.2.14, Intel Xeon E3-1240 v6 (4 cores, 8 threads), `powersave` governor, rustc 1.99.1, release builds. Nothing else ran on the host; every sample took `/tmp/base-scheduler-perf.lock`.

| File | Contents |
| --- | --- |
| [acceptance.jsonl](acceptance.jsonl) | 1,170 samples from [`scripts/budget.py`](scripts/budget.py) with 9 repeats, `main` and branch interleaved in random order within each repeat. Builds are named `main` and `branch` in the data; the revisions are recorded here only. |
| [acceptance.md](acceptance.md) | Its summary from [`scripts/summarize.py`](scripts/summarize.py): the median, minimum and maximum on `main`, the branch median, and a verdict against the budget for that row (≤ ~3% for timer cases; the branch median within `main`'s range for echo, spawn and idle cases, per decision 11). |
| [stall.md](stall.md) | [`scripts/quick.py`](scripts/quick.py), 9 pairs: a 10 ms root sleep behind a 200 ms stall on a 2-worker scheduler, and a 1 Hz pulse timer. |
| [spawn-idle.md](spawn-idle.md) | `quick.py`, 21 pairs: spawn, spawn with a root timer pending, and one request per second for 10 s (idle server CPU). |
| [monitor-threads.txt](monitor-threads.txt) | Per-thread CPU and voluntary context switches for branch timer workloads, 3 runs each, from [`scripts/threads.py`](scripts/threads.py). A monitor thread's voluntary switches approximate its wakes; `/proc` CPU times resolve to 10 ms, so "< 0.01 s" means below that. |

The Rust sources in [`scripts/`](scripts/) are as run. The same workloads are now the `scenario` and `periodic` modes of `base-scheduler-benchmark-timers`, whose `diagnostics` feature reports rescue counts.

## Result

- **Stall:** worst-case lateness falls from 185.2 ms to 1.33 ms (1.21–1.49 ms); median lateness is unchanged.
- **Improvements:** `sleep yield` and `sleep refresh` are 5–10% faster on both selectors, mostly from registrations borrowing the scheduler instead of cloning its `Arc`. Root sleeps use 27% less CPU, with 17% lower p50 lateness.
- **Echo matrix:** within noise. Six server-CPU or p50 rows have branch medians just above `main`'s maximum; their medians are 0.9–3.0% above `main`'s.

Over budget, for the maintainer's decision (design decision 4):

| Case | Metric | `main` | Branch | Change | Note |
| --- | --- | --- | --- | --- | --- |
| diag native 4/1024/1 ms, unaligned | late p50 | 64.0 µs | 80.5 µs | +25.8% | CPU −12%. See the design's implementation notes. |
| diag native 4/1024/1 ms, unaligned | late p99 | 234.8 µs | 242.7 µs | +3.4% | |
| diag native 4/1024/1 ms, aligned | late p50 | 414.5 µs | 427.8 µs | +3.2% | p99 −5.1%, CPU −4.7%. |
| `pulse` (1 Hz timer) | CPU | 1.05 ms | 1.35 ms | +28.5% | The monitor wakes once per deadline, *S* after it; inherent to deadline-driven detection (decision 15). |
| timers native reuse, native sleep, iouring sleep `cycle` 64/4 | late p50 | 2.41 / 3.66 / 4.96 µs | 2.76 / 4.00 / 5.74 µs | +14.4 / +9.1 / +15.7% | Sub-microsecond; `main`'s own ranges are wider than the change. |
| timers native reuse `cycle` 64/4 | late p99 | 37.1 µs | 84.8 µs | +128% | Bimodal: `main` ranges 9–129 µs. |
| timers iouring reuse, native sleep `cycle` 64/4 | CPU per op | 557 / 1,356 ns | 594 / 1,493 ns | +6.6 / +10.1% | Ranges span about 2×; the other two `cycle` cases are −11.9% and −10.5%. |
| diag native 1/1/10 µs | late p50 | 5.18 µs | 5.50 µs | +6.1% | |
| diag iouring 1/1/10 µs | late p99 | 14.86 µs | 15.67 µs | +5.4% | |
| diag iouring 4/1024/1 ms | late p50 | 20.8 µs | 21.6 µs | +3.5% | Range 6.8–34 µs. |

## Within noise, but with large median shifts

The verdict for spawn and idle rows is whether the branch median falls within `main`'s range. Several medians moved a lot inside wide ranges:

- **Spawn.** `rb spawn` +6% and `rb spawn-pending` +16% in the acceptance run; +6% and +10% over 21 further pairs ([spawn-idle.md](spawn-idle.md)), with every range spanning 1.9–2.5×. With 200,000 spawns (`perf stat -e instructions:u,cycles:u rb <workload> 4 200000`, 3 runs per build, not committed), user-space instructions differ by +0.7% (`spawn`) and −0.9% (`spawn-pending`), while cycles vary by up to 30% between runs of the same build, so the shifts come from cross-thread wake and park timing rather than added work. This run therefore neither confirms nor refutes the design's claim that spawning does not depend on whether a root timer is pending.
- **Idle server CPU, one request a second.** Native +103% and io_uring +31% in the acceptance run, while native-poll was −47% and io_uring-poll −19% (below `main`'s range). Over 21 further 10-second pairs, native was +51% (2.3–7.8 ms on `main`, 2.6–8.5 ms on the branch) and io_uring +11% (2.95–4.36 ms and 2.87–8.71 ms). The echo server holds no timers, so the monitor should not wake; this was not confirmed per thread.

## Monitor thread

From [monitor-threads.txt](monitor-threads.txt), on the branch:

| Workload | Monitor wakes | Monitor CPU | Worker CPU |
| --- | --- | --- | --- |
| diag native 4/1024/1 ms | 3,637–3,647 | 0.04–0.05 s | 8.4–8.7 s |
| `root-sleep`, 1,000 sleeps | 940–964 | ≤ 0.01 s | 0.02–0.03 s |
| `task-sleep-pending`, 300 iterations | 246–251 | < 0.01 s | 0.04–0.06 s |
| `pulse`, 10 pulses | 11–16 | < 0.01 s | < 0.01 s |
| timers native sleep `cycle` 64/4 | 184–187 | < 0.01 s | 0.37–0.58 s |
| timers native sleep `yield` 64/4 | 1–4 | < 0.01 s | 2.2 s |

The monitor wakes about once per distinct deadline that workers are collecting anyway, plus every millisecond while a deadline is overdue; in the 1,024-timer diagnostic it costs about 0.5% of worker CPU.

## Not measured

The design asks for further measurements without budgets. These were not taken:

- fixed-rate p99 with every core occupied;
- a workload with occasional undeclared blocking (the net benefit);
- rescue counts per workload, including stuck-poll rescues in the named benchmarks, which should be zero. They were measured later, in [`benchmarks/timers/results/rescue-counts`](../../../timers/results/rescue-counts/readme.md).
