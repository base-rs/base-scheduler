# Performance Investigation: Native CPU varies between processes in expiring workloads

## Identity and status

- Investigation ID: `performance/investigation-2026-11-08-native-cpu-variation`.
- Date and author/reviewer: 8 November 2026; Claude for Samuel Williams; not reviewed.
- Evidence class: screening.
- Evidence validity: valid as an observation of run-to-run variation; the cause is not assessed.
- Decision: open.
- Related investigations: [timer batching](investigation-2026-11-06-timer-batching.md), which found the periodic diagnostic bimodal and feedback-driven.

## Hypothesis and constraints

While checking the timer benchmark's move to `base-benchmark` (#117), Native's CPU per expiry varied widely between processes of the same build in workloads where timers expire. io_uring's did not. Native's thread-park idle path is the suspect: io_uring parks workers in the ring instead.

## Frozen variants and preparation

The timer benchmark of #117's branch at `e7dce78`, prepared on hana with `base-benchmark` 0.1 with one layout replica. Both artifacts were the same executable (`da9be882…`): 0.1's replicas rebuilt a workspace member at another path, which Cargo identifies by its relative path, so the replica was byte-identical. Every "replica" difference below is therefore the run-to-run variation of one executable, not build layout.

## Predeclared method

`base-benchmark run` in screening mode, 9 rounds, CPUs 0–3, stack padding on, scenarios `periodic`, `expire-contended`, `refresh`, `spawn`, and `yield-contended`.

## Evidence and results

- Raw bundles: the campaigns in [native-cpu-variation](../benchmarks/timers/results/native-cpu-variation), without their staged executables. `base-benchmark summarize` regenerates their summaries; 0.2 also warns that each replica is identical to its baseline.

| Metric and scope | `current` → `current-replica` per round, median [min, max] | Validity and interpretation |
| --- | --- | --- |
| `periodic` (4 workers, 1,024 tasks), Native CPU per expiry | +7.4% [−53.6%, +111.1%] | One executable. Median 2.3 µs per expiry, minimum 0.7 µs. |
| `periodic`, io_uring CPU per expiry | −4.5% [−13.1%, +1.7%] | Narrow by comparison. |
| `expire-contended` (128 tasks, 4 workers), Native CPU | +50.8% [−66.0%, +203.3%] | One executable. |

## Decision and code disposition

Open; nothing changed.

## Revisit conditions

Compare each process's CPU with its workers' park and unpark counts, for example with the `diagnostics` feature extended to count parks, to test whether the slow processes are those whose workers park and unpark more.
