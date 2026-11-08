# Performance Investigation: Tokio adapter stream reads and writes

## Identity and status

- Investigation ID: `performance/investigation-2026-11-06-tokio-adapter-reads`.
- Date and author/reviewer: 6–7 November 2026; Claude for Samuel Williams; reviewed and merged in #107.
- Evidence class: screening.
- Evidence validity: valid at 1 and 4 workers for CPU per request, against run-to-run noise; build layout was not controlled ([why](../benchmarks/echo/results/quick-screens/readme.md#the-layout-control)).
- Decision: adopted.
- Related investigations: [stream transfers](investigation-2026-11-06-stream-transfer-registration.md), whose receive rule later applied the same clearing to every consuming receive.

## Hypothesis and constraints

With one worker, 64-byte echo on the Tokio adapter trailed raw Tokio by about 18%. A syscall count over one 2-second run at 1 worker and 32 connections found the cause: the adapter issued 4.02 million `recv` calls, half of them failing with `EAGAIN`, against raw Tokio's 2.21 million. Raw Tokio clears readiness after a short read; the adapter cleared only on `WouldBlock`, because a short read can stop at a TCP urgent mark with more bytes queued. Two changes followed:

1. Clear readiness after a short read for OOB-disabled stream readers on Linux and Android, requesting `EPOLLPRI` so that reported urgent data or closure keeps readiness instead.
2. A compact path for `Cancellation::never()` reads and writes.

Bytes after an urgent mark must stay readable, and a notification racing the clear must survive; Tokio's readiness tick provides the latter.

## Frozen variants and preparation

`main`, change 1, and change 1 plus change 2, on the `perf/tokio-adapter` branch, built by `quick.py echo` in each tree.

## Predeclared method

`quick.py echo` on hana: 64-byte messages, 1 and 4 workers, 32 and 256 connections, 5 interleaved rounds per run, raw Tokio as the unchanged control. Each change was screened, then confirmed by a second run.

## Evidence and results

- Raw bundles: [echo-tokio-adapter-1–4](../benchmarks/echo/results/quick-screens/results). #107's description has the full table.

| Metric and scope | Change, rounds improved | Validity and interpretation |
| --- | --- | --- |
| CPU/request, 1 worker, change 1 | −14% to −16%, 5/5 in two runs | Valid. |
| CPU/request, 1 worker, change 2 on change 1 | −3% to −6%, 5/5 in two runs | Valid. |
| CPU/request, 4 workers | −8% to −10% (change 1), then −1% to −3% | Valid for change 1; change 2 is small. |
| Gap to raw Tokio, 1 worker | from about 18% to 4–7% | Raw Tokio 6.03–6.04 µs/request, the adapter 6.36–6.57. |

The same-executable control stayed within about ±2%.

## Decision and code disposition

Both changes were adopted in #107, with tests for each OOB policy, full reads, a racing notification, bytes after an urgent mark, closure, EOF, reset, and blocking writes.

## Revisit conditions

The remaining 5–9% CPU gap to raw Tokio at 1 worker. [Untested ideas](index.md#open-ideas) for it are listed in the index.
