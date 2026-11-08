# Performance Investigation: What stall rescue's monitor costs

## Identity and status

- Investigation ID: `performance/investigation-2026-11-05-stall-rescue-monitor-costs`.
- Date and author/reviewer: 5–6 November 2026; Claude for Samuel Williams; the record was reviewed when it was committed.
- Evidence class: screening.
- Evidence validity: valid for the paired monitor-on and monitor-off comparisons; the unrecorded monitor variants are invalid as evidence (output printed, not kept).
- Decision: adopted as designed (the monitor stays); rejected or not pursued: the monitor variants, skipping deadlines of sleeping workers.
- Related investigations: [timer batching](investigation-2026-11-06-timer-batching.md), [timer registrations](investigation-2026-11-05-timer-registration-borrow.md). The measurements are recorded in [rescue-costs](../benchmarks/timers/results/rescue-costs/readme.md); [design/stall-rescue.md](../design/stall-rescue.md) is the design.

## Hypothesis and constraints

Stall rescue added a monitor thread that wakes for published deadlines and rescues stalled workers. Two suspicions: that the monitor caused the readiness selector's p50 rise in the periodic diagnostic, and that it cost lateness on io_uring (+34% p50 in one early run). Separately, could the monitor skip deadlines of sleeping workers, whose idle timeout covers them?

## Frozen variants and preparation

Builds before stall rescue, with it on, and with the monitor off, listed with their build commands in the [record](../benchmarks/timers/results/rescue-costs/readme.md).

## Predeclared method

The periodic diagnostic, unaligned and aligned, and pulse, on hana, paired by repeat (15 repeats), with per-thread CPU from `/proc`, and oversubscription runs. Details are in the record.

## Evidence and results

- Raw bundles and summary: [rescue-costs](../benchmarks/timers/results/rescue-costs), generated [summary.md](../benchmarks/timers/results/rescue-costs/summary.md).

| Question | Finding | Validity and interpretation |
| --- | --- | --- |
| Readiness p50 rise | Higher than before stall rescue in 14 of 15 repeats with the monitor on and in 14 of 15 with it off. | Valid: not the monitor. See [timer batching](investigation-2026-11-06-timer-batching.md). |
| Monitor CPU | io_uring unaligned +5.0% process CPU (on higher in 12 of 15), aligned +2.7%; readiness −1% to +2%. Total CPU still below before stall rescue. | Valid. |
| Monitor lateness on io_uring | p50 not consistently higher (on higher in 7 and 3 of 15); p99 within 3%. | Valid: the early +34% was noise. |
| Oversubscription | Rescued timers 0.31–0.54% of expiries; p50 1–4%, CPU about 1%. | Valid; p99 ranges overlap, no p99 claim. |
| Monitor variants: never mark, skip the locked recheck, never make a pass, wake only on a timer | Printed, not recorded, too noisy to attribute. Variants without a plan unparked the monitor tens of thousands of times per run. | Invalid as evidence; no conclusion. |
| Skip deadlines of sleeping workers | Reasoning only: a worker leaving idle would then unpark the monitor whenever its deadline beats the plan, every tick for `pulse`, moving a futex wake onto the worker's path. | Not measured. |

## Decision and code disposition

The monitor stays as designed. The variants were not kept; skipping sleeping workers' deadlines was not pursued.

## Revisit conditions

- A lock-free SeqCst handshake between the monitor's plan store and published deadlines, replacing the locked recheck, if the monitor's io_uring CPU matters; it was proposed when that cost was suspected and turned out small.
- The unmeasured cases in the record: the echo matrix, spawning, and idle CPU with the monitor off, fixed-rate p99 with every core busy, and occasional undeclared blocking.
