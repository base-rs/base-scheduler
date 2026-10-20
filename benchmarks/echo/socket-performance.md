# Socket readiness and direct ring interrupts

This comparison measures lighter Tokio readiness waiting and direct io_uring idle waits against the implementation at `6690045`. Both controls use the same benchmark source and an unchanged client executable. The [raw results](results/socket-performance/aiko.json) retain every measurement; the [runner](results/socket-performance/run.py) serializes workloads with an exclusive host lock.

## Implementation and io-event review

The Tokio adapter now reserves one polling waker per socket direction, avoiding the independent readiness-waiter list for the usual single reader and writer. Concurrent operations use Tokio's existing independent waiters. Dropping a reservation releases it; forgetting one leaves later operations on the independent waiter path. Lazy registration remains necessary to avoid permanently cached hangup readiness on unconnected Linux sockets.

Every operation still attempts its nonblocking syscall before consulting readiness. An experimental readiness-first shortcut delayed valid lifecycle calls and immediate OS errors after a previous operation cleared readiness. Independent review reproduced this with repeated `listen` and invalid `accept` calls; the shortcut was removed and regression tests added. The optimization retains cancellation and concurrent-waiter semantics.

The [io-event implementation reviewed at c8550a8](https://github.com/socketry/io-event/blob/c8550a85e340941491ee4c09b73d54c5310fed1e/ext/io/event/selector/uring.c#L1726) arms an asynchronous read on its eventfd before releasing the GVL and waiting on the ring. Another thread writes the eventfd without touching SQ or CQ. The read consumes the counter, and its distinguished completion clears the registration flag. The pending read survives timeouts and interrupted waits, rather than being submitted twice.

Core adopts that direct-ring wake mechanism with a pointer-free `PollAdd` on an eventfd. The worker drains the event counter separately after completion. This retains no kernel pointer into wake storage, simplifying exceptional driver destruction. Only the owning worker accesses SQ/CQ; external wakeups increment the event counter. Extended enter timeouts bound finite waits when available, while older kernels use `ppoll` over the ring and eventfd. Neither path uses epoll rearming or timerfd. A fresh CQ check after submission avoids sleeping when submission itself publishes completions.

A direct asynchronous read also adds a lifetime obligation for its destination, including driver failure and destruction. The [upstream cancellation documentation](https://github.com/axboe/liburing/blob/master/man/io_uring_cancelation.7#L240) distinguishes ring shutdown cancellation from explicitly draining requests before freeing associated resources. A future direct-read variant would need to retain the wake storage until its original CQE or preserve it on an exceptional path, just as owned I/O requests do. The current `PollAdd` avoids that pointer-lifetime requirement.

Core does not reproduce Ruby's GVL release/reacquisition logic or add a separate nonblocking kernel poll before blocking. It retains its executor publication/parking fence and full-CQ-snapshot publication before polling local tasks. Ruby resumes fibers from its CQ loop; Core intentionally publishes runnable tasks for the executor to poll separately. The change does not enable additional ring setup flags: io-event's single-issuer/deferred-task-work configuration needs a separate review of ring creation and worker-thread ownership here.

Capacity waiters are notified when admission leaves full capacity; native task drain waiters are notified when the outstanding runnable count reaches zero. Closure retains its independent wakeups. Normal local worker wake publication remains unchanged.

## Results

The first complete matrix improves both Tokio adapter modes by about 1–5%. Four-worker adapter throughput is within 3–5% of pure Tokio, but one-worker/32-connection throughput still trails by 8–11%. The single-connection adapters are effectively level with the control. This meets the 5% goal in some configurations, not across the matrix.

Native borrowed throughput is within 1.5–2.2% of Tokio with four workers and improves the one-connection cases. Native owned throughput improves modestly in concurrent cases, but the one-worker/one-connection/64-byte case regresses from 236,431 to 201,327 round trips/second (14.8%), with p50/p99 increasing from 4.02/6.06 to 5.04/10.57 microseconds. Lower whole-lifecycle CPU use (3.37 to 2.44 seconds) does not make that latency regression acceptable. This candidate remains under investigation; the retained data must not be read as evidence of an across-the-board native improvement.

Median complete round trips per second across five repetitions:

| Workers | Connections | Bytes | Backend | Before | After | Change |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 1 | 64 | tokio | 271,450 | 271,073 | -0.1% |
| 1 | 1 | 64 | tokio-adapter | 261,202 | 270,296 | +3.5% |
| 1 | 1 | 64 | tokio-adapter-poll | 261,443 | 270,229 | +3.4% |
| 1 | 1 | 64 | iouring | 236,431 | 201,327 | -14.8% |
| 1 | 1 | 64 | iouring-poll | 181,333 | 213,784 | +17.9% |
| 1 | 1 | 4096 | tokio | 235,551 | 234,457 | -0.5% |
| 1 | 1 | 4096 | tokio-adapter | 235,927 | 238,195 | +1.0% |
| 1 | 1 | 4096 | tokio-adapter-poll | 236,518 | 239,806 | +1.4% |
| 1 | 1 | 4096 | iouring | 186,077 | 187,019 | +0.5% |
| 1 | 1 | 4096 | iouring-poll | 210,450 | 233,479 | +10.9% |
| 1 | 32 | 64 | tokio | 697,704 | 696,398 | -0.2% |
| 1 | 32 | 64 | tokio-adapter | 595,840 | 619,716 | +4.0% |
| 1 | 32 | 64 | tokio-adapter-poll | 599,707 | 623,483 | +4.0% |
| 1 | 32 | 64 | iouring | 518,275 | 528,008 | +1.9% |
| 1 | 32 | 64 | iouring-poll | 560,890 | 562,175 | +0.2% |
| 1 | 32 | 4096 | tokio | 613,107 | 612,759 | -0.1% |
| 1 | 32 | 4096 | tokio-adapter | 532,670 | 557,230 | +4.6% |
| 1 | 32 | 4096 | tokio-adapter-poll | 536,747 | 561,149 | +4.5% |
| 1 | 32 | 4096 | iouring | 467,853 | 478,305 | +2.2% |
| 1 | 32 | 4096 | iouring-poll | 505,289 | 505,006 | -0.1% |
| 4 | 32 | 64 | tokio | 1,748,875 | 1,746,579 | -0.1% |
| 4 | 32 | 64 | tokio-adapter | 1,624,488 | 1,663,298 | +2.4% |
| 4 | 32 | 64 | tokio-adapter-poll | 1,620,111 | 1,682,283 | +3.8% |
| 4 | 32 | 64 | iouring | 1,491,012 | 1,543,121 | +3.5% |
| 4 | 32 | 64 | iouring-poll | 1,707,642 | 1,720,244 | +0.7% |
| 4 | 32 | 4096 | tokio | 1,625,135 | 1,610,110 | -0.9% |
| 4 | 32 | 4096 | tokio-adapter | 1,502,440 | 1,551,043 | +3.2% |
| 4 | 32 | 4096 | tokio-adapter-poll | 1,506,315 | 1,560,062 | +3.6% |
| 4 | 32 | 4096 | iouring | 1,357,224 | 1,405,756 | +3.6% |
| 4 | 32 | 4096 | iouring-poll | 1,572,004 | 1,574,333 | +0.1% |

After-change throughput as a fraction of the same-build pure Tokio control:

| Workers | Connections | Bytes | Tokio owned | Tokio borrowed | Native owned | Native borrowed |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 1 | 64 | 99.7% | 99.7% | 74.3% | 78.9% |
| 1 | 1 | 4096 | 101.6% | 102.3% | 79.8% | 99.6% |
| 1 | 32 | 64 | 89.0% | 89.5% | 75.8% | 80.7% |
| 1 | 32 | 4096 | 90.9% | 91.6% | 78.1% | 82.4% |
| 4 | 32 | 64 | 95.2% | 96.3% | 88.4% | 98.5% |
| 4 | 32 | 4096 | 96.3% | 96.9% | 87.3% | 97.8% |

Minimum–maximum throughput of the unchanged pure Tokio controls:

| Workers | Connections | Bytes | Before control | After control |
| --- | --- | --- | --- | --- |
| 1 | 1 | 64 | 268,654–271,914 | 267,650–273,651 |
| 1 | 1 | 4096 | 233,661–237,258 | 234,202–242,029 |
| 1 | 32 | 64 | 693,766–702,193 | 693,730–699,360 |
| 1 | 32 | 4096 | 608,987–624,015 | 609,462–618,059 |
| 4 | 32 | 64 | 1,716,846–1,769,102 | 1,728,979–1,756,938 |
| 4 | 32 | 4096 | 1,604,599–1,627,844 | 1,563,066–1,635,382 |

Median per-run p50 / p99 round-trip latency, in microseconds:

| Workers | Connections | Bytes | Backend | Before | After |
| --- | --- | --- | --- | --- | --- |
| 1 | 1 | 64 | tokio | 3.56 / 4.35 | 3.54 / 4.33 |
| 1 | 1 | 64 | tokio-adapter | 3.60 / 5.64 | 3.57 / 4.73 |
| 1 | 1 | 64 | tokio-adapter-poll | 3.61 / 5.57 | 3.58 / 4.69 |
| 1 | 1 | 64 | iouring | 4.02 / 6.06 | 5.04 / 10.57 |
| 1 | 1 | 64 | iouring-poll | 5.33 / 7.10 | 4.87 / 9.46 |
| 1 | 1 | 4096 | tokio | 3.89 / 5.38 | 3.93 / 5.39 |
| 1 | 1 | 4096 | tokio-adapter | 3.92 / 5.41 | 3.88 / 5.31 |
| 1 | 1 | 4096 | tokio-adapter-poll | 3.92 / 5.38 | 3.86 / 5.33 |
| 1 | 1 | 4096 | iouring | 5.01 / 7.90 | 5.22 / 10.87 |
| 1 | 1 | 4096 | iouring-poll | 4.44 / 5.88 | 4.00 / 5.37 |
| 1 | 32 | 64 | tokio | 46.85 / 53.70 | 46.94 / 53.66 |
| 1 | 32 | 64 | tokio-adapter | 54.88 / 61.22 | 52.73 / 59.33 |
| 1 | 32 | 64 | tokio-adapter-poll | 54.56 / 60.83 | 52.45 / 59.04 |
| 1 | 32 | 64 | iouring | 61.22 / 69.76 | 60.06 / 68.48 |
| 1 | 32 | 64 | iouring-poll | 56.83 / 63.10 | 56.73 / 62.88 |
| 1 | 32 | 4096 | tokio | 53.38 / 60.70 | 53.38 / 60.70 |
| 1 | 32 | 4096 | tokio-adapter | 61.44 / 68.29 | 58.72 / 65.73 |
| 1 | 32 | 4096 | tokio-adapter-poll | 60.99 / 67.84 | 58.34 / 65.31 |
| 1 | 32 | 4096 | iouring | 67.84 / 76.61 | 66.37 / 75.26 |
| 1 | 32 | 4096 | iouring-poll | 63.07 / 69.69 | 63.13 / 69.57 |
| 4 | 32 | 64 | tokio | 17.39 / 52.19 | 17.45 / 52.19 |
| 4 | 32 | 64 | tokio-adapter | 18.59 / 53.31 | 18.16 / 52.99 |
| 4 | 32 | 64 | tokio-adapter-poll | 18.61 / 53.85 | 17.89 / 52.29 |
| 4 | 32 | 64 | iouring | 19.98 / 56.41 | 19.39 / 56.03 |
| 4 | 32 | 64 | iouring-poll | 17.45 / 54.59 | 17.52 / 53.79 |
| 4 | 32 | 4096 | tokio | 18.75 / 59.65 | 18.91 / 59.74 |
| 4 | 32 | 4096 | tokio-adapter | 20.06 / 60.35 | 19.44 / 59.68 |
| 4 | 32 | 4096 | tokio-adapter-poll | 19.93 / 60.38 | 19.30 / 59.58 |
| 4 | 32 | 4096 | iouring | 21.95 / 65.15 | 21.30 / 63.97 |
| 4 | 32 | 4096 | iouring-poll | 18.89 / 62.14 | 18.99 / 61.82 |

Median server process CPU seconds, before → after (whole lifecycle):

| Workers | Connections | Bytes | Pure Tokio | Tokio owned | Tokio borrowed | Native owned | Native borrowed |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 1 | 64 | 1.62 → 1.62 | 1.82 → 1.81 | 1.82 → 1.79 | 3.37 → 2.44 | 2.12 → 1.75 |
| 1 | 1 | 4096 | 1.60 → 1.60 | 1.82 → 1.77 | 1.80 → 1.77 | 3.03 → 2.40 | 2.51 → 2.04 |
| 1 | 32 | 64 | 2.54 → 2.55 | 2.72 → 2.67 | 2.70 → 2.66 | 2.88 → 2.86 | 2.80 → 2.79 |
| 1 | 32 | 4096 | 2.61 → 2.60 | 2.76 → 2.70 | 2.75 → 2.70 | 2.89 → 2.88 | 2.83 → 2.83 |
| 4 | 32 | 64 | 8.92 → 8.94 | 9.44 → 9.24 | 9.25 → 9.25 | 12.65 → 12.44 | 11.83 → 11.62 |
| 4 | 32 | 4096 | 9.49 → 9.46 | 9.78 → 9.71 | 9.65 → 9.70 | 12.55 → 12.39 | 11.83 → 11.68 |

## Rejected experiments and remaining costs

Trying a nonblocking owned send before ring submission improved concurrent traffic in screening, but reduced single-connection 64-byte throughput by about 22%. The [isolated notification experiment](results/socket-performance/explorations/notification-only.json) and its [provenance](results/socket-performance/explorations/notification-only-provenance.json) retain that rejection evidence; capacity/drain notification reductions alone preserved the baseline result within variation. Normal local worker wake publication also remains unchanged after screening an alternative.

The [wake-suppression candidate](results/socket-performance/explorations/wake-suppression.json) regressed the small single-connection owned case even without ready-owned-send changes. That candidate combined local wake suppression with capacity/drain notification reductions, so it does not isolate wake suppression alone. Its [provenance](results/socket-performance/explorations/wake-suppression-provenance.json) is explicitly retrospective; the later notification-only comparison isolates the retained changes.

Targeted helper inlining showed no meaningful improvement and was removed. Moving the benchmark's owned/borrowed dispatch outside the spawned future avoids carrying both state machines in one future. That change applies to both server builds. The [inlining/dispatch screening](results/socket-performance/explorations/inline-dispatch.json) is exploratory evidence rather than the final comparison: its frozen hashes identify the executables, but the temporary inlining edits were not committed as a reproducible source revision. The [idle experiment manifest](results/socket-performance/explorations/idle-provenance.json) records earlier frozen binary hashes and result paths; the complete comparison above is authoritative for this candidate.

[Fresh profiles and syscall summaries](results/socket-performance/profiling/analysis.md) confirm that the Tokio linked-list readiness-waiter hotspot fell below 0.5% each for polling and drop, from 13.72% and 13.81% in the before profile. These are sampled instruction locations, not predicted wall-clock savings. The task wrapper contains inlined application/socket work, so its samples do not isolate task bookkeeping. Owned serial io_uring still makes about four ring-enter calls per round trip in tracing; native borrowed operations use roughly one direct receive and send. Final epoll activity in those traces is limited to a small lifecycle/fallback component. Tracing changes scheduling substantially and supplies mechanism evidence, not throughput evidence.

The [earlier profiles](readiness-profiling.md) identified epoll rearming, readiness waiters, request retirement, and executor bookkeeping. These changes reduce specific parts of that work, rather than measuring pure trait-dispatch cost. The Tokio adapter still has explicit admission/cancellation checks and a task wrapper that contains user destructor panics. Native ring waits still allocate request records and completion channels, and the executor still has shared task bookkeeping. Their individual contribution to the remaining throughput gap is not established by this benchmark.

## Method and provenance

Measurements run on Aiko's Ryzen 9 9950X3D, Linux 7.2.6, and Rust 1.99.0. Each of six worker/connection/payload configurations runs five times for both builds and all five backends: 300 separate server/client samples. Each has one second of warmup and three seconds of measurement. Every repetition shuffles all combinations using seed `20261025`. Server workers use CPU 0 or CPUs 0–3; the fixed four-worker client uses CPUs 4–7, on separate physical cores sharing a last-level cache. The host lock prevents the cooperating experiments, builds, and tests from overlapping timings.

- Before source: `14c509acc1c8d8775aa802cbd5c84accd6d5fd3e`, constructed from `6690045baf43409466b5418a1c464b5ef9559329` plus the benchmark-only dispatch change in `75fcc9c`. Its scheduler implementation is unchanged from the baseline.
- After source: `4a061fbc4355738f39f5eb6ea1db1520db6a753e`.
- Fixed client source: `5736301fded46c4e5f11d5387950ae2ee0807b94`.

Both servers use `cargo build --release --locked -p core-scheduler-echo-benchmark` with `CARGO_PROFILE_RELEASE_DEBUG=1`: optimization level 3, default release code generation and LTO settings, and no `target-cpu=native`. The builds use the same target directory sequentially, with Cargo tracking the different source paths and dependency fingerprints; each executable is copied and hashed before the next build. No timed executable contains coverage instrumentation.

| Executable | SHA-256 |
| --- | --- |
| before | `002163e1acdf63624d03f8f2691e1e9de2b7f1c87f9e7150e2b763f2156fca11` |
| after | `2387876a4f7cab272524f62cd79aa5302931016b1d4f010ef9406d94f7eb35f5` |
| client | `db6c914ab75011477ec6232e681046b1e75532cbaa1c81663bac984591675cf3` |

All Core echo operations pass `Cancellation::never()` and reuse their per-connection buffer. The client verifies echoed bytes and includes only complete round trips within the measurement window. This closed-loop loopback test includes client scheduling and kernel networking; it does not isolate server-only capacity, measure connection churn, or establish physical-network performance. Reported ratios are ratios of medians, not confidence intervals. Per-run percentiles are summarized separately rather than pooled. CPU totals cover warmup and shutdown as well as the measured interval, so they are not exact per-request CPU costs. See the [benchmark methodology](readme.md).

To reproduce with the retained executables:

```sh
python3 benchmarks/echo/results/socket-performance/run.py \
  --before /tmp/core-perf-socket-control \
  --after /tmp/core-perf-socket-candidate \
  --client /tmp/core-scheduler-profile-baseline/release/core-scheduler-echo-benchmark \
  --before-source 14c509acc1c8d8775aa802cbd5c84accd6d5fd3e \
  --after-source 4a061fbc4355738f39f5eb6ea1db1520db6a753e \
  --client-source 5736301fded46c4e5f11d5387950ae2ee0807b94 \
  --output /tmp/core-scheduler-socket-performance-results
```

## Behavioral verification

Canonical whole-workspace Bake coverage with all targets and features passes 18,333/18,333 measured source regions on Linux and 15,817/15,817 on macOS, without unreachable-region exclusions. Linux's temporary LLVM wrapper excludes only distro compiler sources under `/usr/src/debug/rust/`; no project sources are exempted. Formatting, Clippy, and API documentation checks pass on both systems. GitHub Actions also passes its Linux, macOS, and Windows gates and Rust 1.85 compatibility checks. Independent adversarial review confirmed the revised runtime and design documentation.

Regression cases cover independent read/write wakers, concurrent waiters, dropped and forgotten reservations, runtime shutdown, syscall-first lifecycle/error results, eventfd wake/sleep races, interrupted waits, old-kernel finite-wait fallback, notification failure with full CQ-batch publication, and capacity/drain transitions.
