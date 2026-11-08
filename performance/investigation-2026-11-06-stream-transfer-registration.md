# Performance Investigation: Vectored, flagged, and pooled stream transfers, and one receive rule

## Identity and status

- Investigation ID: `performance/investigation-2026-11-06-stream-transfer-registration`.
- Date and author/reviewer: 6–7 November 2026; Claude for Samuel Williams; reviewed and merged in #108 and #110.
- Evidence class: screening.
- Evidence validity: valid at 1 worker against run-to-run noise; build layout was not controlled ([controls](../benchmarks/echo/results/quick-screens/readme.md#controls)).
- Decision: adopted (#108, then the receive rule in #110).
- Related investigations: [owned stream readiness](investigation-2026-11-06-owned-stream-readiness.md), [Tokio adapter reads](investigation-2026-11-06-tokio-adapter-reads.md). #110's measurements are recorded in [unified-receive](../benchmarks/echo/results/unified-receive/readme.md).

## Hypothesis and constraints

After #104, pooled `read`, `read_vectored`, `read_vectored_with_flags`, `read_owned_vectored`, and `write_owned_vectored` on the readiness selector still took the generic path: a registration lookup under a mutex and repeated admission checks on every call. Pooled `read` also waited through `socket_wait`, which issues a zero-timeout `poll(2)` before and after each wait. Two steps were measured:

1. **#108:** reuse the cached registration while keeping each transfer's readiness rules: reach the socket first, clear only on `WouldBlock`, do not reject urgent data.
2. **The scalar rule:** apply `read_into`'s rule to these receives too: wait on readiness first once established, clear after a short read, and reject urgent data when OOB support is disabled. This changes behaviour, so it needed the maintainer's decision.

## Frozen variants and preparation

- **#108:** `main` plus only the commit adding the `native-vectored`, `native-owned-vectored`, and `native-pooled` echo backends, against the change, on `perf/owned-vectored`.
- **The scalar rule:** [owned-vectored-scalar-rule.patch](../benchmarks/echo/results/quick-screens/results/owned-vectored-scalar-rule.patch), applied to the third `owned-vectored` tree, for the screen; the adopted form is #110, on `perf/unified-receive`.

## Predeclared method

`quick.py echo` on hana: 64-byte messages, 1 worker unless stated, 32 and 256 connections, 5 interleaved rounds per run, with confirmation runs.

## Evidence and results

- Raw bundles: [echo-owned-vectored-1–4](../benchmarks/echo/results/quick-screens/results) for #108 and the scalar rule (run 3); for #110, the runs in [unified-receive](../benchmarks/echo/results/unified-receive), and three earlier screens of the same work, `echo-unified-receive-1–3`, in [quick-screens](../benchmarks/echo/results/quick-screens/results). #108's description and #110's record have the full tables.

| Metric and scope | Change, rounds improved | Validity and interpretation |
| --- | --- | --- |
| Pooled `read` requests/s and CPU/request, 1 worker (#108) | +24% to +32%, −25% to −29%, 5/5 in three runs | Valid. |
| Vectored and owned vectored, 1 worker (#108) | +3% to +6%, −4% to −7% | Valid against noise; small. |
| Pooled `read`, 4 workers (#108) | +16% to +25% throughput | Valid; vectored within noise. |
| Scalar rule on the #108 paths, CPU/request, 1 worker | a further −14% to −18%, to about the scalar cost | Valid screen. |
| #110, converted paths on every backend, 1 worker | +11% to +36% requests/s, −14% to −33% CPU/request, 5/5 in two runs | Valid against run-to-run noise. |
| #110, converted paths, 4 workers | +2% to +16% requests/s, −6% to −21% CPU/request | Valid against run-to-run noise for CPU/request and the Tokio adapter; the selector's +1.8% to +3.7% throughput is within the scalar backends' ±3.3% spread. |
| #110, scalar paths | within the control | Unchanged, as intended. |

The same-executable control stayed within 1.7% in runs 1–2 and 3.6% in run 4 of #108.

## Decision and code disposition

#108 was adopted first, keeping the old rules. The scalar rule was not rejected on performance: it cost less, and the maintainer decided on 7 November 2026 to apply it on every backend if performance was the same or better, preferring the simpler single rule. #110 did so, with `try_read_vectored` added, at a net +127 production lines. Peeks keep the conservative rule. No current downstream user calls these paths.

## Revisit conditions

Owned vectored transfers whose buffers lack prepared I/O vectors allocate scratch per call; packet and pipe transfers still use the generic path. Both are in the [open ideas](index.md#open-ideas).
