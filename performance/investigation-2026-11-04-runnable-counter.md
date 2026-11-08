# Performance Investigation: Native's runnable counter

## Identity and status

- Investigation ID: `performance/investigation-2026-11-04-runnable-counter`.
- Date and author/reviewer: 4–6 November 2026; Claude for Samuel Williams. The padding was reviewed and merged in #94; the per-thread experiment was not reviewed.
- Evidence class: screening.
- Evidence validity: valid against run-to-run noise for the padding's timer result; build layout was not controlled, and #94's raw samples were not kept. For the per-thread variants: valid as a screen against run-to-run noise; build layout was not controlled.
- Decision: adopted (padding, #94, `b628fb9`); rejected (per-thread counters).
- Related investigations: [driver wakes](investigation-2026-11-06-driver-wakes.md), which also targeted the 4-worker CPU overhead.

## Hypothesis and constraints

`Shared::outstanding`, the global count of runnable tasks, is written on every schedule and retirement. It shared a 64-byte line with `closed`, which every retirement reads, so cross-core traffic on one invalidated the other (#94: `outstanding` at offset 80, `closed` at 96). Two hypotheses followed:

1. Padding the counter to its own 128-byte block removes the false sharing.
2. The counter itself, a contended atomic, explains Native's extra CPU per request with 4 workers. Per-thread counts would remove it.

Every ordering on the counter had to stay the same for the padding. The per-thread variant needed a new argument that drain and closure are still detected, made at the idle fences.

## Frozen variants and preparation

- **Padding:** `main` at `4c36c8e` against the branch at `29cc0de`, both with the echo harness from #95 (`1a1e11c`) copied into each tree. No layout control.
- **Per-thread counters,** each against `140cc25`, recovered from the screened trees:
  - [runnable-counter-1.diff](../benchmarks/echo/results/quick-screens/runnable-counter-1.diff) (executable `5a968bab…`): per-thread counts, with drain and closure checked at every retirement.
  - [runnable-counter-2.diff](../benchmarks/echo/results/quick-screens/runnable-counter-2.diff) (`e5cfafc1…`): per-thread counts that only grow, written with plain single-writer stores, checked at the idle fences.
  - [runnable-counter-3.diff](../benchmarks/echo/results/quick-screens/runnable-counter-3.diff) (`10bc0ec6…`): the same, counting a runnable before publishing it; committed on the local branch `perf/runnable-counter` as `c309117` and reverted.

## Predeclared method

- **Padding (#94):** 9 paired runs in random order on hana, echo servers pinned to CPUs 0, 4, 1, 5 and clients to 2, 6, 3, 7, timer benchmarks unpinned. Medians with min–max.
- **Per-thread counters:** `quick.py echo` with 4 workers, 32 and 256 connections, 5 interleaved rounds, raw Tokio as the unchanged control. Runs 1 and 2 had no copy control; run 3's copy was the same executable as its base ([controls](../benchmarks/echo/results/quick-screens/readme.md#controls)).

## Evidence and results

- Raw bundles: #94's description holds the padding tables; its samples were not kept. The per-thread screens are [echo-runnable-counter-1–3](../benchmarks/echo/results/quick-screens/results), run *n* screening candidate *n*; the changes below are from `quick.py`'s summary of each file.

| Metric and scope | Baseline | Candidate | Change | Validity and interpretation |
| --- | --- | --- | --- | --- |
| `reuse yield`, 64 tasks on 4 workers, ns/op | 114.4 (113.3–116.3) | 106.0 (105.1–111.1) | −7.3% | Valid screen; outside the baseline's whole range. |
| Other timer cases (#94): `sleep refresh`, `cycle`, the periodic diagnostic | | | −1.9% to +7.4% | Within their ranges; the diagnostic's p50 is bimodal ([notes](index.md#measurement-notes)). |
| Echo throughput and CPU/request (#94) | | | within ±1% | Unchanged. |
| Echo p99 (#94) | | | −1.4% to +2.7%; fixed-rate p99 +19% (286 → 341 µs) | #94 called the fixed-rate p99 inconclusive. |
| Echo CPU/request, 4 workers, candidate 1 (checked at every retirement) | | | +0.4% to +2.7%; 0 of 5 rounds better in three of the four cases | Worse, one run. |
| Echo CPU/request, 4 workers, candidates 2 and 3 (checked at idle fences) | | | −0.5% to +1.3% | No effect; run 3's same-executable control moved at most 0.5%. |

The atomics' apparent 3.7% of cycles in profiles was mostly memory stalls, which moved elsewhere when the counter changed (net about −1%).

## Decision and code disposition

The padding was adopted in #94: a clear gain on contended yields, at 152 bytes of `Shared` per scheduler. The per-thread counters were rejected: one variant cost more CPU and the other changed nothing, against a correctness argument that had grown more delicate. They were reverted; the diffs above preserve them.

## Revisit conditions

A profile on a different host showing contention on the counter itself rather than memory stalls, or a design change that needs per-worker counts anyway, such as per-worker drain detection.
