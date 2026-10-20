# Single-worker performance research

Fresh profiles of merged PR #17 distinguish the Tokio adapter's read policy from scheduler overhead. Across three workloads, the adapter is 1.2–2.6% below raw Tokio using comparable conservative polling reads. Native's borrowed path still pays for repeated readiness-request setup. This research retains no production changes; the smaller ring-policy experiments do not justify changing the implementation.

The [validated tables](results/single-worker-research/tables.md), [raw evidence and reproduction notes](results/single-worker-research/readme.md), and [summary generator](results/single-worker-research/summarize.py) preserve the experiments separately from earlier optimization results.

## Fresh baseline

These are three uninstrumented repetitions per backend and workload. They repeat the existing observation: Native is close to Tokio with four workers, while the one-worker gap remains substantial.

| Backend | 1 worker / 32 connections / 64 bytes | Relative to Tokio | 4 workers / 128 connections / 64 bytes | Relative to Tokio |
| --- | ---: | ---: | ---: | ---: |
| Pure Tokio | 702,831 | 100.00% | 2,116,772 | 100.00% |
| Tokio adapter, owned | 642,343 | 91.39% | 2,039,750 | 96.36% |
| Tokio adapter, borrowed | 649,979 | 92.48% | 2,039,618 | 96.36% |
| Native io_uring, owned | 630,368 | 89.69% | 2,085,611 | 98.53% |
| Native io_uring, borrowed | 567,467 | 80.74% | 2,120,796 | 100.19% |

## Matching the Tokio read policy

The original pure-Tokio control uses `AsyncReadExt::read`. The adapter retains readable readiness until a receive returns `WouldBlock`. Those policies do different work in this echo workload. As the [earlier urgent-data regression](readiness-performance.md#short-reads-and-tcp-urgent-data) demonstrated, clearing readable readiness after any short read can strand queued trailing bytes at a TCP urgent-data boundary. This research keeps the conservative policy.

A benchmark-only control screen uses the same executable to compare raw Tokio's ordinary reads with `readable().await` followed by `try_read`, with and without the adapter's task wrapper. In the one-worker/32-connection/64-byte workload, conservative raw Tokio reaches 651,113 requests/s, the owned adapter 649,269, and the borrowed adapter 648,354. The adapter is 0.28–0.42% below that conservative async-waiter control. This narrows the unexplained overhead; it does not isolate trait dispatch, since the readiness APIs, cancellation, and buffer handling still differ.

Directly spawning the original adapter I/O loop with `tokio::spawn` reaches 646,724 requests/s for owned I/O and 646,274 for borrowed I/O, providing no improvement over the corresponding adapter task handles. The raw-policy controls show task-wrapper differences below 1%. Removing task ownership or panic handling has no demonstrated useful benefit here.

A separate five-repetition matrix uses `poll_fn(TcpStream::poll_read_ready)` followed by `try_read`, matching the adapter's polling readiness policy more closely:

| Workers/connections/bytes | Ordinary raw Tokio requests/s | Conservative polling raw Tokio requests/s | Owned adapter relative to conservative control | Borrowed adapter relative to conservative control |
| --- | ---: | ---: | ---: | ---: |
| 1/32/64 | 702,727 | 663,549 | 98.18% | 98.76% |
| 1/32/4096 | 625,315 | 592,154 | 98.03% | 98.49% |
| 4/128/64 | 2,127,976 | 2,110,324 | 97.44% | 98.15% |

The same-policy comparison meets the 5% target in these three workloads. Against ordinary Tokio reads, the adapters still reach only 92.71–93.27% at one worker and 96.63–97.34% at four workers in this matrix. Both comparisons use controls in the same executable. The polling control preserves short-read readiness but does not equalize every detail of `AsyncFd`, `TcpStream`, cancellation, cooperative budgets, or ownership handling. The remaining 1.2–2.6% is consequently not an isolated measurement of interface dispatch.

## Counting the failed receives

Separate `perf stat` samples count receive entries and filter receive exits for kernel `-EAGAIN`. Each cell below is the median count per completed echo from two instrumented runs at one worker, 32 connections, and 64 bytes. Instrumented throughput is excluded from capacity comparisons.

| Backend | Receive syscalls / echo | Receive EAGAIN / echo | Send syscalls / echo | Ring enters / echo |
| --- | ---: | ---: | ---: | ---: |
| Pure Tokio `AsyncRead` | 1.000 | 0.000 | 1.000 | 0.000 |
| Conservative raw Tokio | 2.000 | 1.000 | 1.000 | 0.000 |
| Tokio adapter, owned | 2.000 | 1.000 | 1.000 | 0.000 |
| Tokio adapter, borrowed | 2.000 | 1.000 | 1.000 | 0.000 |
| Native io_uring, owned | 0.000 | 0.000 | 1.000 | 0.063 |
| Native io_uring, borrowed | 2.000 | 1.000 | 1.000 | 0.063 |

Owned Native receives use io_uring and therefore do not issue userspace receive syscalls. Borrowed Native receives perform the payload syscall directly and submit a one-shot readiness wait after `WouldBlock`. Both Native paths batch ring enters, approximately one per 16 echoes here. This count alone does not show the cost of individual SQEs or CQEs.

The earlier baseline stat samples report approximately 15,505 instructions per echo for pure Tokio, 17,499 for the owned Tokio adapter, 17,407 for its borrowed adapter, 16,324 for owned Native, and 18,531 for borrowed Native. These are whole-server instrumented counts, including small startup and shutdown contributions. They support a difference in work performed; they are not a measurement of the isolated cost of `EAGAIN`.

## Native costs and rejected candidates

The fresh cycles profiles include substantial networking and kernel locking work. Within Native userspace, request admission, route lookup, submission, and completion-channel send/drop remain visible. Borrowed operations additionally construct and retire a `ReadinessRequest` for each unsuccessful readiness cycle. Tokio's task-wrapper symbols include inlined application and I/O code, so their full sample share must not be interpreted as wrapper overhead.

Two benchmark-only alternatives test smaller changes before considering a persistent readiness design:

- Returning Native to the separate async-io readiness selector reaches only 413,164 owned or 416,864 borrowed requests/s in the focused control screen, versus 636,995 and 572,351 for the corresponding ring paths. Reintroducing that selector is a substantial regression for this workload.
- Setting `IORING_RECVSEND_POLL_FIRST` for nonempty owned receives changes throughput by +0.69% at 1/1/64, −8.11% at 1/32/64, and +0.07% at 4/128/64. The unchanged Tokio control varies by +1.05%, −0.23%, and −0.38%. The concurrent single-worker loss rejects this hint. The diagnostic runs on Linux 7.2 and does not implement an older-kernel compatibility fallback.

Deferred task work with single-issuer rings is also rechecked independently of the receive hint because PR #17 changed executor batching and self-wake handling. Only fresh worker-bound rings change; movable drivers and the existing setup fallback remain intact. The 54-sample screen remains mixed:

| Workers/connections/bytes | Owned Native change | Borrowed Native change | Unchanged Tokio control |
| --- | ---: | ---: | ---: |
| 1/1/64 | −1.44% | +2.48% | +0.11% |
| 1/32/64 | −0.18% | −0.39% | +0.01% |
| 4/128/64 | +1.21% | +0.76% | +0.41% |

The modest concurrent differences and opposing single-connection results do not justify changing ring policy. The broad single-connection ranges overlap. A separate [setup trace](results/single-worker-research/deferred/setup.strace) confirms the deferred and single-issuer flags were accepted on the worker ring. This diagnostic is retained as evidence, with the existing cooperative ring configuration unchanged.

## Next useful direction

Persistent per-socket readiness is the strongest remaining Native hypothesis. It could reuse the kernel registration and readiness state across reads, avoiding the per-wait admission, request storage, and completion reply. This research identifies the work it could remove but does not establish its throughput benefit.

Such an experiment needs explicit rules for concurrent waiters, cancellation, rearming, migration between worker rings, and socket close. Retaining a socket indefinitely in a cache would conflict with the current exclusive close contract. A multishot poll also needs to handle terminal completion, unsupported kernels, and readiness changes without stale-event or lost-wake shortcuts. The rejected allocation and hashing experiments in the [previous research index](results/idle-notification-performance/readme.md) show why a larger registration-lifetime change is more promising than another small container optimization.

The adapter's original comparison with Tokio `AsyncRead` remains useful as an application measurement. The same-policy controls answer a different question: how much overhead remains when both paths do comparable receive work. Neither should replace the other.

## Method and limits

All experiments run on Aiko's Ryzen 9950X3D using the locked dependencies and default optimized Cargo release profile, without explicit LTO or target-CPU overrides. Server workers use CPU 0 or CPUs 0–3; the fixed four-worker client uses CPUs 4–7. Builds, timings, and profiling serialize on one exclusive lock. Every sample uses a new server and persistent validated IPv4 loopback TCP connections with `TCP_NODELAY`, reused 64 KiB buffers, and one outstanding round trip per connection.

Uninstrumented samples use one second of warmup followed by three seconds of measurement. The final polling-policy matrix uses five randomized repetitions; other capacity screens use three. Whole-process stat samples use zero warmup so their request denominator closely matches the measured server lifetime. Cycles profiles use one warmup second and five measurement seconds. CPU counts and sampled costs remain separate from uninstrumented throughput.

The baseline executable was frozen at `9386f7c4f6cb0dccd7e528512476d2cf963f7a88`; its executable code matches merged `130cd4a67e526e80ba9a5e7e7ebbe81494bd903b`. Benchmark-only controls and diagnostic patches build from the latter. Raw evidence records executable hashes and source patches. These short loopback experiments do not prove equivalence at sub-percent precision or predict physical-network and general application performance.
