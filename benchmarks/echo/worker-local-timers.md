# Native worker-local timers

Native now owns a separate timer queue on each task worker. The event loop collects expiries before its next task batch and uses the earliest deadline to bound its interruptible I/O wait. There is no timer thread, including for root futures or callers on other executors.

This integrates the earlier [worker-driven](worker-timers-research.md), [worker-local](local-timers-research.md), and [wake-delivery](timer-ready-research.md) studies. The historical studies describe their own experimental baselines; their patches remain archived. The current implementation keeps immediate waker delivery, which already publishes stealable tasks, without another timer-action queue.

## Ownership and progress

A local sleep registers directly with the worker running its first poll. A root or external sleep selects a worker by round robin and submits a registration command with an owning acknowledgment. Dropping an unconsumed acknowledgment releases an accepted registration.

An existing timer stays on its original worker when its waiting future migrates. The registered waker determines which task to make runnable; it is replaced when the future is polled with a different waker. Updates, cancellation, and drop reach the original queue through its owner's inbox when needed. A later sleep can use the new worker. Protocol objects still receive their scheduler explicitly.

The loop checks local expiries and publishes runnable tasks, then runs a finite ready batch and a nonblocking I/O turn. An idle worker uses its local next deadline in the I/O wait. It rechecks timer commands after publishing its sleeping state, so a newly submitted earlier deadline cannot be missed. Without an active I/O driver, the same deadline bounds an interruptible thread park. Closing admission rejects late commands, drains accepted ones, and notifies timer waiters before worker retirement. Shutdown waits for callbacks and driver destruction too.

Timer capacity is reserved per worker. An exhausted owning queue returns `WouldBlock` even if another queue has capacity; outstanding keys retain their slots after expiry until released. Queue operations reuse reserved storage, including reclaiming a canceled non-root heap entry under capacity pressure. Root/external registration uses an acknowledgment allocation; local registration, repolling, and drop allocate nothing within existing capacity.

## Measurement

The Aiko comparison uses matching `cargo build --release --locked` binaries based on `1689f8d`, with the integrated implementation applied only to the candidate. Both use the checked-in echo workload and the same baseline client executable. Server workers occupy CPU 0 or CPUs 0–3; the client uses CPUs 4–7. Samples run in randomized order under the exclusive performance lock. Both versions inherit 1 ns timer slack, avoiding the coalescing-policy difference identified in the earlier study.

Each echo cell has three repetitions with one second of warmup and two seconds measured. The table reports median requests/s. These short loopback samples establish a regression screen; they do not prove equivalence at sub-percent precision.

| Workers / connections / bytes | Owned before → after | Change | Borrowed before → after | Change | Tokio control change |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 / 1 / 64 | 236,304 → 235,952 | -0.15% | 240,492 → 239,906 | -0.24% | +0.09% |
| 1 / 32 / 64 | 628,367 → 631,862 | +0.56% | 567,074 → 568,354 | +0.23% | +0.43% |
| 4 / 128 / 64 | 2,082,752 → 2,065,830 | -0.81% | 2,095,342 → 2,110,232 | +0.71% | +0.49% |
| 4 / 128 / 4,096 | 1,848,253 → 1,850,944 | +0.15% | 1,873,308 → 1,881,660 | +0.45% | +0.44% |

Owned and borrowed echo changes stay within −0.81% to +0.71%. The unchanged Tokio controls change by +0.09% to +0.49%; this screen shows no clear networking regression.

The timer diagnostic has three repetitions per cell, with one second of warmup and three seconds measured. Each task requests a fresh deadline after resuming; the aligned case selects a common 1 ms tick. Lateness is measured when the task resumes. CPU is whole-process user plus system time, including warmup, startup, and shutdown, rather than only the measurement window. This is a periodic workload, not maximum timer capacity or a fixed-arrival-rate comparison.

| Workers / timers / delay | p50 lateness before → after | p99 lateness before → after | Process CPU seconds before → after | CPU change |
| --- | ---: | ---: | ---: | ---: |
| 1 / 1 / 10 µs | 3.20 → 1.20 µs | 10.65 → 1.66 µs | 0.867 → 0.310 | -64.3% |
| 4 / 1,024 / 1,000 µs | 8.18 → 2.21 µs | 19.74 → 9.77 µs | 5.615 → 1.636 | -70.9% |
| 4 / 1,024 / 1,000 µs aligned | 139.90 → 50.24 µs | 282.88 → 82.81 µs | 3.689 → 1.324 | -64.1% |

The 10 µs case completes about 74,946 → 88,251 measured expiries/s (+17.8%). The unaligned four-worker case completes 1,015,240 → 1,021,365/s (+0.60%); the aligned case completes 1,024,000/s in both versions. CPU falls by about 64–71% in these cases while lateness also improves. These rates reflect requested periodic delays and task resumption costs, not a general timer throughput limit.

Raw samples, runner, diagnostic source, compiler details, binary hashes, and reproduction notes are in [the evidence directory](results/worker-local-timers/readme.md).

## Validation

The shared clock conformance suite runs both inside worker tasks and from root futures. Additional regressions cover migration, replacement wakers while registration is pending, cancellation and dropped acknowledgments, owner capacity, late commands, shutdown callback retirement, reentrant/panicking wakers, earlier external deadlines, and timer progress after I/O-driver retirement. Allocation checks cover local sleep lifecycles and reserved queue reuse and compaction.

On macOS, the complete workspace with all features passes the shared Bake region gate at 16,167/16,167 measured regions. Native passes at 1,524/1,524 on both macOS and Linux; the timer crate passes at 1,605/1,605 with all features and 698/698 with `local` alone on both hosts. The default, driver-only, and sharded-only timer gates also pass on macOS. Workspace tests pass on both hosts, and workspace Clippy with all features passes on macOS. Cross-platform CI retains Windows, macOS, Linux, and Rust 1.85 jobs and adds the local-only timer gate.
