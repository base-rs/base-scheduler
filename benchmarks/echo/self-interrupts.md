# Avoiding io_uring self-interrupts

The native executor can publish a completed task while its worker is preparing to sleep. Previously, publishing that task could write an interrupt from the ring's own worker. The [interrupt-frequency investigation](results/socket-performance/selected/interrupt-frequency/analysis.md) measured roughly one successful owner-thread eventfd write per lifecycle echo in the rejected direct-ring prototype, against two writes from other threads per run.

## Runtime changes

An io_uring worker now records its own wake in a coalescing thread-local flag. Matching uses the exact controller identity, including nested selectors and different workers of the same selector. Productive turns retain the flag until the actual wait decision. Other threads continue using an eventfd; its retained descriptor and outstanding ring poll preserve notifications racing that decision. The native executor's publication and parking handshake is unchanged.

Idle workers wait directly on io_uring, with one outstanding `PollAdd` watching the eventfd. A notification CQE drains the counter before rearming. Payloads and cancellations get submission slots first, so repeated notifications cannot starve a one-entry queue. Cancellation backlog prevents a blocking wait even after transient submission failures. The whole available completion snapshot is published before returning to the executor, including when the notification completion contains an error.

Idle turns submit and wait in one kernel entry. Productive turns skip empty submissions unless CQ overflow or deferred task work requires kernel progress. Finite waits require extended wait arguments, probed at setup. This intentionally adds a kernel feature requirement compared with the former polling-based ring wait.

Owned scalar socket writes first attempt a synchronous nonblocking send, after cancellation and selector-admission checks. Ready writes return the original owner without request allocation or a capacity permit; blocked or interrupted attempts retain the completion path. Scalar reads submit directly to the ring. Owned retry readiness stays on the operation's original ring. The public capability and buffer-ownership contracts are unchanged.

## Measurement and provenance

All timing runs use optimized release builds, the same fixed client, CPU affinity, and an exclusive host benchmark lock. Builds and correctness checks use the corresponding shared lock. Counter instrumentation is measured separately from ordinary throughput. Request throughput and latency describe the client's measurement window; CPU and interrupt counts cover the server lifecycle, including warmup and shutdown. Samples from separate screens are not pooled.

The [historical 180-sample comparison](socket-performance.md) remains a measurement of `938e7e6`; it has not been relabelled as this runtime. The [original direct-ring prototype](direct-ring-experiment.md) and its 14.8% regression are also retained separately.

## Final confirmation

The [fresh 60-sample comparison](results/socket-performance/owner-wake-final/analysis.md) measures production runtime `3af77be0` against the original native backend at `63da755`. Each table cell has three randomized repetitions. All samples retain complete latency/CPU metadata and separate JSON files; [provenance](results/socket-performance/owner-wake-final/provenance.json) records release settings and executable/source hashes. This follow-up does not compare against pure Tokio.

Median throughput change:

| Workers | Connections | Bytes | Owned | Borrowed |
| --- | --- | --- | --- | --- |
| 1 | 1 | 64 | −0.01% | +24.67% |
| 1 | 32 | 64 | +17.58% | +0.11% |
| 1 | 32 | 4096 | +14.55% | −0.54% |
| 4 | 128 | 64 | +10.27% | −0.43% |
| 4 | 128 | 4096 | +10.17% | −0.15% |

Concurrent owned throughput improves 10–18%; concurrent borrowed throughput remains within 0.6% of its control. Serial owned 64-byte throughput is effectively unchanged in this run (234,402 → 234,387 requests/s), and borrowed throughput rises from 184,665 to 230,231 requests/s. A separate predecessor screen with five repetitions measured a 2.9% owned serial drop. That screen remains separate: the final three-repeat neutral result does not establish that a small loss is impossible in every run. The earlier gaps against pure Tokio are not resolved by this comparison.

All eight control/candidate × owned/borrowed × one/four-worker idle samples record zero CPU ticks over two seconds. Ring workers sleep in `io_cqring_wait`.

[Separate source counters](results/socket-performance/explorations/direct-ring-write-fastpath/counted/samples/aiko.json) measured zero owner-controller eventfd writes and positive remote writes in all four owned/borrowed × one/four-worker cases. Those counters cover the immediate predecessor, before the final cancellation-backlog guard and test seam. They demonstrate the wake mechanism separately from final throughput; their instrumented rates are excluded from every performance table. The final source also has deterministic owner-wake and remote-wake regression tests.

## Rejected and intermediate candidates

Removing a syscall can alter scheduling. The selector-local flag removed all owner-thread notification writes but increased voluntary context switches and reduced serial owned throughput by 24.6%. The removed write/read cycle apparently gave network completions more time to arrive before a blocking wait; this is an interpretation of the timing evidence rather than proof of an exact kernel interleaving.

A direct-ring candidate initially appeared faster, but an idle test found a busy loop: it rearmed the eventfd poll before draining the readable counter. Its apparent gains are invalid as evidence of an acceptable improvement. The corrected runtime drains before rearming and sleeps when idle. Moving retry readiness onto the original ring did not, by itself, recover owned serial throughput. Combining submission and waiting helped borrowed I/O but still left owned I/O slower. The synchronous owned-send path recovered most of that loss and improved concurrent owned throughput. Trying a direct receive first was rejected because it usually added an unsuccessful syscall before the ring read.

The [exploration archive](results/socket-performance/explorations) retains source patches, runners, provenance, samples, and rejection notes for owner suppression, retained permits, selector-local flags, level waits, CQ ordering, direct-ring waiting, original-ring retries, combined entry, and scalar fast paths. One preliminary write-fastpath matrix accidentally had its aggregate report overwritten by the serial runner. Its recovered throughput and switch counts are explicitly labelled, and its lost latency/CPU fields have not been reconstructed or invented. The final confirmation uses a separate output directory and fresh measurements.

## Correctness

Runtime commit: `3af77be0f8b8a9303c8680a7f8aab49b1c3d304c`.

All 70 io_uring unit tests, 180 shared conformance cases, and the backend doctest pass. Regression cases cover local wake coalescing, exact-controller matching, remote wake races, timeouts, eventfd saturation, notification rearming, mixed completion/error batches, one-entry submission queues, deferred cancellation after submission errors, original-ring retry affinity after migration, original-buffer identity, full socket send queues, and empty datagrams.

Canonical workspace coverage passes on Linux at 17,930/17,930 measured regions, with one invariant-only region excluded. macOS passes at 15,343/15,343 measured regions. The io_uring package alone passes at 2,119/2,119. The Linux implementation is platform-gated; the macOS gate checks the remaining workspace composition. These are measured source-region counts rather than exhaustive scheduling-state or branch coverage.
