# Final socket performance profiling

Run on Aiko on 2026-10-20, 00:02:13–00:02:44 UTC, after the exclusive benchmark matrix released `/tmp/core-scheduler-perf.lock`. All profiling and report export held that lock. No builds or concurrent benchmark work ran in this session.

Frozen server sources and executable SHA-256 hashes:

- Before: `14c509acc1c8d8775aa802cbd5c84accd6d5fd3e`; `002163e1acdf63624d03f8f2691e1e9de2b7f1c87f9e7150e2b763f2156fca11`.
- After: `4a061fbc4355738f39f5eb6ea1db1520db6a753e`; `2387876a4f7cab272524f62cd79aa5302931016b1d4f010ef9406d94f7eb35f5`.
- Fixed client: `5736301fded46c4e5f11d5387950ae2ee0807b94`; `db6c914ab75011477ec6232e681046b1e75532cbaa1c81663bac984591675cf3`.

## Tokio profiles

All four profiles use 1 worker, 32 connections, 64-byte messages, a 1-second warmup and 5-second measurement, `cycles:u` at 999 Hz, and DWARF call stacks. Server CPU 0; client CPUs 4–7. All report zero lost samples. See each `self.txt` and `callgraph.txt` for full output.

| Self sample location | Before owned adapter | After owned adapter | After adapter poll | After pure Tokio |
|---|---:|---:|---:|---:|
| `Readiness::poll` | 13.72% | <0.5% | <0.5% | <0.5% |
| `Readiness::drop` | 13.81% | <0.5% | <0.5% | <0.5% |
| `TaskFuture::poll` | 13.07% | 12.33% | 14.01% | N/A |
| Nested receive/read operation `PollFn::poll` | 1.33% in old cancellable wrapper | 7.40% | 8.82% | N/A |
| `ScheduledIo::poll_readiness` | <0.5% | 4.91% | 5.46% | 17.48% |

The significant linked-list waiter polling/drop hotspot has disappeared. The remaining adapter samples are concentrated in nested task/operation futures and Tokio task/readiness machinery. `TaskFuture::poll` contains inlined echo and socket operation work: its percentage does not measure the cost of task bookkeeping alone. The old and new operation wrappers also differ in inlining and symbol attribution.

These are sampled instruction locations, not predicted wall-clock savings. Although the requested event excludes kernel execution, the reports contain unresolved kernel-address sample locations; no conclusions are drawn from those addresses. The matrix, rather than instrumented throughput, establishes capacity.

## Native syscall traces

Each trace uses 64-byte messages, no warmup, and a 0.5-second measurement. Whole-process totals include startup and shutdown.

| Backend/case | Before measured requests | After measured requests | Before `io_uring_enter` | After `io_uring_enter` | Before `epoll_ctl` / `epoll_pwait` | After `epoll_ctl` / `epoll_pwait` |
|---|---:|---:|---:|---:|---:|---:|
| Owned, 1w/1c | 15,098 | 15,328 | 60,406 | 61,329 | 19 / 1 | 7 / 0 |
| Poll, 1w/1c | 42,621 | 41,132 | 54 | 39 | 38 / 6 | 7 / 0 |
| Owned, 4w/32c | 325,492 | 390,281 | 137,868 | 172,294 | 172 / 18 | 69 / 0 |
| Poll, 4w/32c | 87,291 | 93,398 | 151 | 372 | 169 / 18 | 72 / 1 |

Owned serial I/O still uses about four ring-enter calls per request in this trace, consistent with submission and completion-wait overhead for the read and write. Poll variants execute approximately one direct `recvfrom` plus one direct `sendto` per request and use the ring when waiting is necessary. Epoll calls in the final implementation are a small lifecycle/fallback component in these traces.

The after notifier uses `write`; the old notifier shares `sendto` with network data sends. Aggregate syscall summaries therefore cannot isolate old notification counts. For owned 1w/1c, before has 29,228 `sendto` calls (28,670 errors), while after has 29,673 `write` calls (zero errors). These totals suggest a change in notification mechanism, not elimination of notification frequency on this serial workload.

Strace substantially changes scheduling and observed throughput, especially for 4-worker poll workloads. Do not use its throughput as capacity evidence, compare absolute syscall totals without request accounting, or interpret summed traced syscall time as CPU time. The exclusive, uninstrumented matrix supplies performance results.
