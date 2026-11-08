# Performance Investigation: Timer registrations borrow the scheduler

## Identity and status

- Investigation ID: `performance/investigation-2026-11-05-timer-registration-borrow`.
- Date and author/reviewer: 5 November 2026, during stall rescue stage 2; Claude for Samuel Williams; reviewed with the stall rescue change.
- Evidence class: screening.
- Evidence validity: valid for the stated cases; no layout control.
- Decision: adopted.
- Related investigations: [stall rescue monitor costs](investigation-2026-11-05-stall-rescue-monitor-costs.md).

## Hypothesis and constraints

A Native timer registration held an `Arc<Shared>` clone, so every registration and release updated one reference count shared by all workers. `perf annotate` put the `lock incq` and `lock decq` among the timer path's hottest instructions. A sleep future already borrows its scheduler, so the registration can borrow `Shared` too, and `NativeTimer`, which owns an `Arc`, can store only the key. Stall rescue's budget for timer microbenchmarks was a regression of at most about 3% ([design](../design/stall-rescue.md)).

## Frozen variants and preparation

The stall rescue stage 2 branch, with and without the change, against `main` of that day. The [implementation notes](../design/stall-rescue.md) describe the change; the source identities of the screen were not recorded separately.

## Predeclared method

The timer benchmark's `sleep yield` case, 64 tasks on 4 workers, on both Native selectors, against `main`, on hana. The `#[inline]` hints on `has_runnables`, `is_closed`, and `finished` were measured the same way, with 5 runs each.

## Evidence and results

- Raw bundles: not retained; the design's implementation notes hold the result.

| Metric and scope | Change against `main` | Validity and interpretation |
| --- | --- | --- |
| `sleep yield` 64/4, ns/op, readiness selector | from about 3% slower to about 10% faster | Valid screen. |
| `sleep yield` 64/4, ns/op, io_uring | from about 3% slower to about 8% faster | Valid screen. |
| Without the inline hints, `sleep yield` and `sleep refresh` | 1–2% more instructions, about 1% more cycles | Small; kept as cheap. |

## Decision and code disposition

Adopted with stall rescue stage 2: it brought the branch within its budget and past `main`.

## Revisit conditions

None expected; only if registrations must outlive the scheduler borrow.
