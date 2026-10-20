# Remove speculative io_uring notification reads

This follow-up to merged PR #14 removes one unnecessary syscall from the io_uring idle path. The [150-sample comparison](results/idle-notification-performance/final/aiko.json), [validated tables](results/idle-notification-performance/final/tables.md), and [build provenance](results/idle-notification-performance/final/provenance.json) preserve the selected implementation separately from exploratory candidates.

## Why the read was unnecessary

The driver uses an eventfd for remote interrupts and a thread-local permit for local notifications. One outstanding ring `PollAdd` watches the eventfd. Previously, every otherwise-idle turn also attempted a nonblocking eventfd read before entering the ring wait. With no remote notification, that read simply returned `WouldBlock`.

The selected implementation consumes the local permit at that boundary and leaves remote notifications to the armed poll. A write before the poll is submitted leaves the counter readable; a write after publication completes the outstanding poll. After collecting its CQE, the driver still drains the counter before rearming, preventing the same notification from repeatedly completing replacement polls. A full submission queue still publishes queued work and returns without parking when the notification poll cannot be armed.

The regression tests exercise first publication through both nonblocking submission and blocking wait, already-armed waits, standalone drivers, entered workers, and an interrupt arriving at the wait boundary. Another test injects an interrupted eventfd read: repeated unnotified idle turns leave that failure untouched, and an actual notification completion consumes it through the retrying drain. The optimization changes neither the wake contract nor buffer ownership, cancellation, shutdown, or resource affinity.

## Final comparison

The final matrix shows a 3.03% single-connection gain for owned Native I/O and 4.87% for borrowed Native I/O. The owned before/after ranges do not overlap; the borrowed ranges do, so the borrowed effect size remains less certain. The pure-Tokio control changes by +0.31% in that workload. This is smaller than the initial pilot's 5–7% result.

Concurrent Native medians change between −0.35% and +0.16%, with overlapping ranges: there is no demonstrated concurrent throughput improvement. Four-worker Native remains at 96.9–98.1% of pure Tokio; one-worker Native still trails it substantially. This optimization does not establish parity.

| Workers/connections/bytes | Before owned requests/s | After owned requests/s | Change | Before borrowed requests/s | After borrowed requests/s | Change |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1/1/64 | 236,923 | 244,101 | +3.03% | 229,479 | 240,659 | +4.87% |
| 1/32/64 | 627,998 | 628,389 | +0.06% | 566,493 | 567,143 | +0.11% |
| 1/32/4096 | 563,836 | 561,877 | −0.35% | 513,201 | 511,754 | −0.28% |
| 4/128/64 | 2,063,474 | 2,066,779 | +0.16% | 2,062,771 | 2,055,581 | −0.35% |
| 4/128/4096 | 1,819,469 | 1,816,904 | −0.14% | 1,830,384 | 1,828,615 | −0.10% |

These are medians of five repetitions. The [complete tables](results/idle-notification-performance/final/tables.md) include all ranges, pure-Tokio controls, relative throughput, and median p50/p99 RTT. The [summary JSON](results/idle-notification-performance/final/summary.json) retains before/after latency ranges. The 4 KiB single-worker pure-Tokio control falls 2.13% despite having no relevant source change; the higher relative Native percentages in that cell should not be interpreted as an additional Native improvement.

Single-connection median p99 RTT changes from 7.30 to 5.77 µs for owned I/O and 6.22 to 6.43 µs for borrowed I/O. The throughput improvement is not a uniform tail-latency improvement.

The Tokio adapter is unchanged. This final matrix therefore measures the two affected Native modes and pure Tokio; the [preceding five-backend comparison](readiness-performance.md) remains the default-build adapter baseline.

## Other experiments

The [exploration index](results/idle-notification-performance/readme.md) records the patches and raw screens. Removing or combining the early nonblocking reactor turn regressed serial traffic. Collecting completions immediately after the blocking wait also regressed serial traffic without improving concurrency. Request-ID hashing and capturing the local request route once per turn were effectively flat. Deferred task work traded a small concurrent gain for a serial reduction. These changes are not retained.

Removing the readiness request's separate box was also flat. Combining its storage and completion reply into one shared allocation added substantially more code and changed borrowed throughput by −0.68% at one worker and −0.06% at four workers. Neither allocation prototype justified retention.

Suppressing owner wakeups also needs care beyond throughput: the generic selector contract allows a selector to invoke callbacks and then block, relying on the task waker to interrupt it. The selected change preserves that contract.

Tokio waiter/future simplification and explicit cancellation or syscall-wrapper inlining did not produce a useful consistent gain. A separate [30-sample thin-LTO diagnostic](results/idle-notification-performance/explorations/tokio/thin-lto-analysis.md) does not reveal a large throughput improvement in its one-worker/32-connection/64-byte workload: thin LTO removes the relevant out-of-line symbols, but changes backend medians by only +0.09% to +1.21%. The owned adapter remains at 92.87% of same-profile raw Tokio and the borrowed adapter at 92.31%. Neither the project release profile nor the adapter implementation changes.

## Reproduction and limits

Before source is `a98f172f8b06f23b34a354a4b6f6c294aa5859ab`; selected production source is `9e005c5061c26bed937953d1e540a3ddde5cc6ed`. The subsequent commit `1547fa6c7f915637b215acc87de7174e68e160db` expands race tests and updates a comment only. Both benchmark executables use locked dependencies and the default optimized Cargo release profile, without explicit LTO, codegen-unit, or target-CPU overrides. The fixed client source and executable hash, source patch, compiler, host, CPU topology, and governors are recorded in the [provenance](results/idle-notification-performance/final/provenance.json).

The final matrix uses five randomized repetitions per cell, with one second of warmup and three seconds of measurement. Server workers occupy CPU 0 or CPUs 0–3; the four-worker fixed client occupies physical cores 4–7 in the same last-level cache. Timed runs hold an exclusive performance lock; builds and tests use its shared mode. The [frozen runner](results/idle-notification-performance/final/run.py) and [sample helper](results/idle-notification-performance/final/sample-runner.py) retain the exact measurement procedure. Run `python3 benchmarks/echo/results/idle-notification-performance/final/summarize.py` to validate and regenerate the summary.

The workload is persistent IPv4 loopback TCP echo with `TCP_NODELAY`, a reusable 64 KiB server buffer, payload validation, and one outstanding request per connection. Throughput counts completed round trips inside the measurement window. RTT includes client scheduling; a median of per-run p99 values is not a pooled p99. CPU values cover the server lifecycle, including setup and warmup. Five repetitions do not establish sub-percent differences reliably, and these results do not predict physical-network or general application performance.

Linux and macOS checks pass, including 100% measured source-region coverage for the selected production behavior. The [validation record](results/idle-notification-performance/final/validation.md) identifies tested commits and logs. The native path still performs per-wait admission, request storage, completion replies, and one-shot readiness registration. Persistent readiness is a larger potential improvement, requiring its own resource-lifetime and cancellation design; this experiment does not measure its benefit.
