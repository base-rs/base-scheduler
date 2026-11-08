# Performance Investigation: Timer lateness after the timer queue rework

## Identity and status

- Investigation ID: `performance/investigation-2026-11-06-timer-batching`.
- Date and author/reviewer: 6 November 2026; Claude for Samuel Williams; the record was reviewed when it was committed.
- Evidence class: screening.
- Evidence validity: valid for the cause of the p50 rise; inconclusive for the expiry variants at 4 workers, which were within the layout control.
- Decision: rejected (deferring an expiry pass's idle wakes; the expiry variants). The p50 rise was found to be mostly an artefact of the diagnostic, not a regression to fix.
- Related investigations: [stall rescue monitor costs](investigation-2026-11-05-stall-rescue-monitor-costs.md). The measurements are recorded in [timer-batching](../benchmarks/timers/results/timer-batching/readme.md).

## Hypothesis and constraints

After the timer queue rework that came with stall rescue, the readiness selector's p50 lateness in the unaligned periodic diagnostic rose by 20–38%. Three questions:

1. Is the rise a real per-expiry cost, or the diagnostic's feedback? Tasks re-arm at run time plus 1 ms, so cheaper expiries can cluster deadlines more tightly.
2. Would leaving an expiry pass's idle wakes to the ready batch reduce lateness and CPU?
3. Would a different expiry pass do better: deadline-ordered under one lock (`seq`), the old one-pop-one-wake pass (`inter`), or 32 wakers per lock hold (`batch32`)?

## Frozen variants and preparation

The [record](../benchmarks/timers/results/timer-batching/readme.md) lists every tree, the [deferral.patch](../benchmarks/timers/results/timer-batching/deferral.patch), its scripts, and the probe harness. The timer diagnostic depends on each tree by absolute path, so its layout control was a different build.

## Predeclared method

The worker-local timer diagnostic on hana at 4 and 8 workers, unaligned and aligned, with a layout replicate, and a spread-phase variant of the diagnostic that removes the feedback. Details and commands are in the record.

## Evidence and results

- Raw bundles: the JSON files in [timer-batching](../benchmarks/timers/results/timer-batching), and the `quick.py timers` screens [timers-timer-batching-1](../benchmarks/echo/results/quick-screens/results).

| Question | Finding | Validity and interpretation |
| --- | --- | --- |
| The p50 rise | Mostly the diagnostic's feedback. With spread phases, the real cost is +4.5% to +6% (p50 0.14–0.28 µs later), with less CPU. | Valid. Not the monitor: see [monitor costs](investigation-2026-11-05-stall-rescue-monitor-costs.md). |
| Deferring idle wakes | Unaligned p50 −14% to −45% and steady-load CPU −14% to −41%, but steady-load p99 at 4 workers +49.5% (readiness) and +15.5% (io_uring), and io_uring CPU in the unaligned diagnostic +4% to +31%. | Valid; a trade-off, not clearly favourable. |
| Expiry variants at 4 workers | Not distinguishable from layout noise, which moved +12.6%. | Inconclusive. |
| `inter` at 8 workers | Recovered the gap in two runs. | Suggestive: the old pass may account for part of the difference. |

## Decision and code disposition

Deferral was rejected as not clearly favourable; its patch is kept in the record. The expiry variants were not pursued. The p50 rise was accepted as mostly an artefact, with a real cost of about 5% at that load and lower CPU.

## Revisit conditions

- A narrower deferral, keeping the wake for a single-timer pass straight out of idle, which is untried.
- The `inter` pass at 8 workers, with salted layout replicas and more rounds.
- A workload where steady-load p99 is not the constraint.
