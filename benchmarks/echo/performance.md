# io_uring performance improvements

The first implementation follow-up to the [profiling investigation](profiling.md) moves wait preparation and capacity-listener registration off io_uring's ready path. It preserves owned buffers, per-worker rings, bounded command processing, whole completion-batch publication, and the existing work-stealing executor. The public interfaces and Tokio implementation are unchanged.

## What changed

`Reactor::turn` now returns after useful progress, a zero-timeout check, or while submissions remain to be flushed. It checks the completion queue directly; asking the outer poller to wait with a zero timeout cannot publish another completion to the executor. The idle path drains notifications and returns if any were observed, because a notification can represent new commands or executor work. Only a genuinely idle turn rearms the notification/ring sources and enters the poller wait.

Notifications are retained while the worker stays busy. Before sleeping, the reactor rearms level-triggered, one-shot sources, so a notification or ring completion arriving between inspection and waiting remains observable. Command processing and completion publication retain their existing bounds. No extra nonblocking `epoll` call or spin loop is introduced.

Capacity acquisition now checks and acquires available capacity before registering an event listener. When full, it registers and rechecks capacity and closure before waiting, preserving wakeups if a permit is returned or admission closes during that interval. Cancellation listeners are also constructed only on this waiting path.

## Relationship to io-event

The comparison used `socketry/io-event` at `c8550a8`. Its [epoll selector](https://github.com/socketry/io-event/blob/c8550a8/ext/io/event/selector/epoll.c#L1074) first polls without releasing Ruby's GVL, then releases the GVL for a blocking wait only when no work was found. That extra poll avoids GVL transitions; Rust has no corresponding reason to copy it.

Its [io_uring selector](https://github.com/socketry/io-event/blob/c8550a8/ext/io/event/selector/uring.c#L1827) flushes submissions and checks completions before preparing a blocking wait. That distinction between completion processing and idle waiting is useful here. Our executor already performs nonblocking driver turns and rechecks its runnable queues before sleeping; the implementation removes redundant poller work inside those turns. It retains our rule that completions enqueue tasks rather than directly resuming application futures.

## Repeated results

Median complete round trips per second across five repetitions. The [raw samples](results/performance/aiko.json) include every run and binary hashes.

| Workers | Connections | Payload | Original io_uring | Improved io_uring | Change | Pure Tokio |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 64 B | 163,054 | 231,231 | +41.8% | 270,222 |
| 1 | 1 | 4,096 B | 156,469 | 186,342 | +19.1% | 233,554 |
| 1 | 32 | 64 B | 481,697 | 515,304 | +7.0% | 695,411 |
| 1 | 32 | 4,096 B | 438,048 | 471,583 | +7.7% | 611,161 |
| 4 | 32 | 64 B | 1,302,641 | 1,496,058 | +14.8% | 1,746,934 |
| 4 | 32 | 4,096 B | 1,186,902 | 1,355,276 | +14.2% | 1,610,414 |

Median of each run's p50 / p99 round-trip latency, in microseconds; these are not pooled percentiles:

| Workers | Connections | Payload | Original p50 / p99 | Improved p50 / p99 |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 64 B | 6.10 / 6.70 | 4.02 / 6.48 |
| 1 | 1 | 4,096 B | 6.28 / 7.39 | 5.05 / 7.86 |
| 1 | 32 | 64 B | 65.79 / 75.58 | 61.53 / 70.02 |
| 1 | 32 | 4,096 B | 72.38 / 82.50 | 67.26 / 76.35 |
| 4 | 32 | 64 B | 23.15 / 57.15 | 19.98 / 56.26 |
| 4 | 32 | 4,096 B | 25.18 / 64.93 | 21.95 / 64.45 |

Throughput improves in all six cases, and each improved run exceeds every corresponding baseline run. Latency does not improve uniformly: with one worker, one connection, and 4-KiB messages, median p99 rises from 7.39 to 7.86 µs (about 6%), despite better median latency and throughput. All five repetitions show that p99 increase, so it should be treated as a measured tradeoff rather than dismissed as an isolated outlier.

Minimum–maximum throughput across the five repetitions:

| Workers | Connections | Payload | Original req/s range | Improved req/s range |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 64 B | 161,423–166,366 | 223,100–238,707 |
| 1 | 1 | 4,096 B | 155,671–158,391 | 166,554–188,243 |
| 1 | 32 | 64 B | 479,484–487,600 | 512,758–520,020 |
| 1 | 32 | 4,096 B | 437,282–441,590 | 468,713–473,389 |
| 4 | 32 | 64 B | 1,276,805–1,335,793 | 1,494,228–1,503,828 |
| 4 | 32 | 4,096 B | 1,179,784–1,202,311 | 1,345,974–1,373,972 |

These short runs establish the direction and approximate size of the combined improvement, not a confidence interval. The earlier isolated prototype percentages must not be added to these gains. Pure Tokio remains faster in this workload; completion channels and worker wakeups remain candidates for separate investigation.

## Validation

The Linux workspace passed all-feature/all-target tests and 18,019/18,019 measured source regions, with the existing single invariant `unreachable!` exclusion. The io_uring package alone covered 1,818/1,818 regions. Workspace Clippy and formatting passed. New regressions check command and completion progress without idle setup, retained notifications across nonblocking turns, notifications arriving while sources are rearmed or immediately before waiting, queued submissions, and capacity retirement/closure before listener registration. Existing tests cover cancellation, full submission queues, shutdown, task migration, and buffer retention.

Aiko's distro Rust embeds standard-library paths under `/usr/src/debug/rust/`, which the coverage tool does not automatically exclude. A temporary `cargo-llvm-cov` wrapper added `--ignore-filename-regex '^/usr/src/debug/rust/'`; no project sources were excluded. The coverage target was cleaned before validation to remove obsolete instrumented objects.

## Measurement method

The [comparison runner](results/performance/run.py) uses the same baseline client binary for both server builds. Both are release builds with optimization level 3, debug symbols, default codegen/LTO settings, and no `target-cpu=native`. The baseline is `5736301fded46c4e5f11d5387950ae2ee0807b94`; the improved Rust implementation is `ecb7a7a03f0bb16b32e8be7bf58fd4746493e87d`. The existing Aiko machine and CPU placement are described in the [initial results](results.md).

Each case uses one second of warmup and three seconds of measurement, five repetitions, and shuffled order. Server workers use CPU 0 or CPUs 0–3; four baseline-client workers use CPUs 4–7. The baseline pure-Tokio server is included as an unchanged comparison. Builds, coverage, and profiling finish before measurement. Results remain closed-loop, end-to-end loopback measurements, not isolated server capacity. CPU totals in the raw data cover entire process lifetimes, including warmup and shutdown.

Reproduce using separate checkouts at the two source revisions, building each with `CARGO_PROFILE_RELEASE_DEBUG=1 cargo build --release --locked -p core-scheduler-echo-benchmark` into distinct target directories, then run:

```sh
python3 benchmarks/echo/results/performance/run.py \
  --baseline /tmp/core-scheduler-profile-baseline/release/core-scheduler-echo-benchmark \
  --improved /tmp/core-scheduler-profile-improved/release/core-scheduler-echo-benchmark \
  --source ecb7a7a03f0bb16b32e8be7bf58fd4746493e87d \
  --output /tmp/core-echo-performance-results.json
```
