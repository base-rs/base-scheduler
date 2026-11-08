# Performance Investigation: Native's runnable counter

## Identity and status

- Investigation ID: `performance/investigation-2026-11-04-runnable-counter`.
- Date and author/reviewer: 4–6 November 2026; Claude for Samuel Williams. The padding was reviewed and merged in #94; the per-thread experiment was not reviewed.
- Evidence class: screening.
- Evidence validity: valid for the padding's timer result. The per-thread variants are inconclusive at the scale claimed: their echo screens had no build-layout control.
- Decision: adopted (padding, #94, `b628fb9`); rejected (per-thread counters).
- Related investigations: [driver wakes](investigation-2026-11-06-driver-wakes.md), which also targeted the 4-worker CPU overhead.

## Hypothesis and constraints

`Shared::outstanding`, the global count of runnable tasks, is written on every schedule and retirement. It shared a 64-byte line with `closed`, which every retirement reads, so cross-core traffic on one invalidated the other (#94: `outstanding` at offset 80, `closed` at 96). Two hypotheses followed:

1. Padding the counter to its own 128-byte block removes the false sharing.
2. The counter itself, a contended atomic, explains Native's extra CPU per request with 4 workers. Per-thread counts would remove it.

Every ordering on the counter had to stay the same for the padding. The per-thread variant needed a new argument that drain and closure are still detected, made at the idle fences.

## Frozen variants and preparation

- **Padding:** `main` at `4c36c8e` against the branch at `29cc0de`, both with the echo harness from #95 (`1a1e11c`) copied into each tree. No layout control.
- **Per-thread counters:** [runnable-counter.patch](../benchmarks/echo/results/quick-screens/runnable-counter.patch), two variants on the local branch `perf/runnable-counter`: per-thread counters updated with locked read-modify-writes, then plain single-writer stores with the checks moved to the idle fences. The branch ends with the revert.

## Predeclared method

- **Padding (#94):** 9 paired runs in random order on hana, echo servers pinned to CPUs 0, 4, 1, 5 and clients to 2, 6, 3, 7, timer benchmarks unpinned. Medians with min–max.
- **Per-thread counters:** `quick.py echo` with 4 workers, 5 interleaved rounds, and its default "layout" control, which for echo was the same executable ([why](../benchmarks/echo/results/quick-screens/readme.md#the-layout-control)).

## Evidence and results

- Raw bundles: #94's description holds the padding tables. The per-thread screens are [echo-runnable-counter-1–3](../benchmarks/echo/results/quick-screens/results).

| Metric and scope | Baseline | Candidate | Change | Validity and interpretation |
| --- | --- | --- | --- | --- |
| `reuse yield`, 64 tasks on 4 workers, ns/op | 114.4 (113.3–116.3) | 106.0 (105.1–111.1) | −7.3% | Valid screen; outside the baseline's whole range. |
| Other timer cases (#94): `sleep refresh`, `cycle`, the periodic diagnostic | | | −1.9% to +7.4% | Within their ranges; the diagnostic's p50 is bimodal ([notes](index.md#measurement-notes)). |
| Echo throughput, CPU/request, and p99 (#94) | | | within ±1% | Unchanged. |
| Echo CPU/request, 4 workers, per-thread locked counters | | | +1.0% to +2.7% | Worse; same direction across runs. |
| Echo CPU/request, 4 workers, plain stores at idle fences | | | within run-to-run noise | No effect found. |

The atomics' apparent 3.7% of cycles in profiles was mostly memory stalls, which moved elsewhere when the counter changed (net about −1%).

## Decision and code disposition

The padding was adopted in #94: a clear gain on contended yields, at 152 bytes of `Shared` per scheduler. The per-thread counters were rejected: one variant cost more CPU and the other changed nothing, against a correctness argument that had grown more delicate. They were reverted; the patch above preserves them.

## Revisit conditions

A profile on a different host showing contention on the counter itself rather than memory stalls, or a design change that needs per-worker counts anyway, such as per-worker drain detection.
