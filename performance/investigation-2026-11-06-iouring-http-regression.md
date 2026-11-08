# Performance Investigation: An apparent io_uring HTTP regression since v0.4.0

## Identity and status

- Investigation ID: `performance/investigation-2026-11-06-iouring-http-regression`.
- Date and author/reviewer: 6 November 2026; Claude for Samuel Williams; not reviewed.
- Evidence class: screening.
- Evidence validity: valid as a negative result: the apparent regression is within the build-layout control.
- Decision: rejected (no regression; nothing to change).
- Related investigations: none.

## Hypothesis and constraints

io_uring HTTP hello measured 1.7–3.8% slower on `main` than on `v0.4.0`. Stall rescue, merged in between, was the suspect.

## Frozen variants and preparation

`v0.4.0`, `main` of 6 November, and a copy of `v0.4.0` at another path, each with `quick.py http`'s private copy of base-http's hello benchmark, which depends on the tree by absolute path. The copy was therefore a different build of the same source, a genuine layout control ([why](../benchmarks/echo/results/quick-screens/readme.md#the-layout-control)).

## Predeclared method

`quick.py http` on hana: io_uring and readiness backends, 16, 64, and 256 connections, 5 interleaved rounds, Hyper as the unchanged control; `perf stat` on one run of each build.

## Evidence and results

- Raw bundles: [http-iouring-regression-1](../benchmarks/echo/results/quick-screens/results/http-iouring-regression-1-20261106-194627.jsonl), [http-iouring-regression-aa](../benchmarks/echo/results/quick-screens/results/http-iouring-regression-aa-20261106-203252.jsonl) (`v0.4.0` against its copy), and [iouring-regression-multi-1](../benchmarks/echo/results/quick-screens/results/iouring-regression-multi-1.jsonl).

| Metric and scope | Change | Validity and interpretation |
| --- | --- | --- |
| io_uring requests/s, `v0.4.0` → `main` | −1.7% to −3.8% | Apparent regression. |
| io_uring requests/s, `v0.4.0` → its copy at another path | as large | The same source "regressed" as much. |
| `perf stat` instructions and cycles per request | unchanged | No added work. |

The server registers no timers, so stall rescue's monitor never runs there.

## Decision and code disposition

No regression: the difference is build-layout noise. Nothing was changed.

## Revisit conditions

A difference beyond salted layout replicas in a `base-benchmark` campaign, or a change in instructions per request.
