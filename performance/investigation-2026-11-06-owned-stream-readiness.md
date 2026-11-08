# Performance Investigation: Owned stream reads and writes on the readiness selector

## Identity and status

- Investigation ID: `performance/investigation-2026-11-06-owned-stream-readiness`.
- Date and author/reviewer: 6 November 2026; Claude for Samuel Williams; reviewed and merged in #104 (`cd51eee`).
- Evidence class: screening.
- Evidence validity: valid at 1 worker; inconclusive at 4 workers, where the gain is no larger than the noise of other screens on this host. Build layout was not controlled.
- Decision: adopted.
- Related investigations: [stream transfers](investigation-2026-11-06-stream-transfer-registration.md) extended the same path to vectored, flagged, and pooled transfers.

## Hypothesis and constraints

`read_owned` and `write_owned` on the readiness selector went through the generic operation. Each call looked up the socket's registration under a mutex, with a linear search and an `Arc` clone, and repeated the cancellation and open checks. `read_owned` also issued a `recv` before checking readiness, so each request paid one receive that failed with `EAGAIN`. Taking the bound borrowed path (`read_with`, `write_with`) should make owned transfers as cheap as borrowed ones.

The behaviour change this needed, rejecting urgent data with `Unsupported` when OOB support is disabled, as `read_into` already did, was accepted by the maintainer and documented in [design/socket-stream-oob.md](../design/socket-stream-oob.md).

## Frozen variants and preparation

`main` against the #104 branch, built by `quick.py echo` in each tree. Source identities are those of #104's branch head and its base.

## Predeclared method

`quick.py echo` on hana: 64-byte messages, 1 and 4 workers, 32 and 256 connections, 5 interleaved rounds per run, raw Tokio as the unchanged control. Changes are medians of per-round changes. These runs had no layout control.

## Evidence and results

- Raw bundles: [echo-owned-readiness-1](../benchmarks/echo/results/quick-screens/results/echo-owned-readiness-1-20261106-194757.jsonl) and [-2](../benchmarks/echo/results/quick-screens/results/echo-owned-readiness-2-20261106-200121.jsonl). #104's description has the full table.

| Metric and scope | Change, rounds improved | Validity and interpretation |
| --- | --- | --- |
| Owned echo requests/s, 1 worker, 32 and 256 connections | +19% to +23%, 5/5 in two runs | Valid: far beyond any noise seen on this host. |
| Owned echo CPU/request, 1 worker | −21% to −24% | Valid. |
| Owned echo requests/s, 4 workers | +2.9% to +4.0%, 5/5, one run | Not established: other screens moved 2–3% without a change. |
| Borrowed echo requests/s, unchanged path | −0.3% to +2.1% | The unchanged path stayed within noise. |

Owned echo at 1 worker then matched the borrowed path and raw Tokio, about 155,000 requests/s at 256 connections.

## Decision and code disposition

Adopted in #104. A test checks registration, clearing after a short read, the cached registration, cancellation returning the buffer, and EOF.

## Revisit conditions

A recorded campaign with salted layout replicas at 4 workers, if the multi-worker gain needs to be claimed.
