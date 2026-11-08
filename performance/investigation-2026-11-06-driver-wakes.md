# Performance Investigation: Wakes from a worker's own driver turn

## Identity and status

- Investigation ID: `performance/investigation-2026-11-06-driver-wakes`.
- Date and author/reviewer: 6 November 2026; Claude for Samuel Williams; reviewed and merged in #105 (`3738ff6`).
- Evidence class: screening.
- Evidence validity: valid for CPU per request at 4 workers on the readiness selector, against run-to-run noise. Build layout was not controlled: the echo "layout" control was the same executable ([controls](../benchmarks/echo/results/quick-screens/readme.md#controls)).
- Decision: adopted (driver-local wakes); rejected (the other wake and steal policies below).
- Related investigations: [runnable counter](investigation-2026-11-04-runnable-counter.md); [record of the measurements](../benchmarks/echo/results/driver-wakes/readme.md).

## Hypothesis and constraints

A readiness registration belongs to the epoll controller of the worker that first waited on the socket. After an idle worker steals a task, that worker's later events are still dispatched by the original worker, which pushed the task into the thief's inbox, often with a wake. With 4 workers, three quarters of borrowed-echo schedules were such remote pushes, and the inbox push and steal paths were about 9% of user samples, against about 4% for raw Tokio's whole queue. Keeping a task woken by a worker's own driver turn on that worker should remove most of them.

The wake publication, fence and recheck protocol, stealing, initial placement, and shutdown had to stay unchanged.

## Frozen variants and preparation

Runs 1 and 2 compared `140cc25` with the change; run 3 compared `cd51eee`, after #104, with the change rebased onto it. The [record](../benchmarks/echo/results/driver-wakes/readme.md) holds the diagnostics patch that counted migrations and remote schedules, and the screening script.

## Predeclared method

`quick.py echo` on hana: 64-byte messages, 4 workers (and 1 worker as a check), 32 and 256 connections, borrowed and owned readiness and io_uring, 5 interleaved rounds per run.

## Evidence and results

- Raw bundles: [driver-wakes](../benchmarks/echo/results/driver-wakes) (`quick-1w.jsonl`, `quick-4w-{1,2,3}.jsonl`; run 3 is `echo-pr105` on hana). Run 3's base and its copy were both `84e0bb4b…` ([artifacts](../benchmarks/echo/results/quick-screens/artifacts.txt)).

| Metric and scope | Change, rounds improved | Same-executable control | Validity and interpretation |
| --- | --- | --- | --- |
| Borrowed readiness CPU/request, 4 workers | −1.6% to −3.3% in three runs; 5/5 except 4/5 at 256 connections in run 2 | −0.8% to +1.4% | Valid against run-to-run noise. |
| Owned readiness CPU/request, 4 workers | −0.6% to −2.4% | −0.6% to +0.5% | Smaller; consistent in run 3. |
| io_uring, 4 workers | −1.5% to +0.7% | | Unchanged, as expected: rings already completed on the submitting worker. |
| 1 worker, CPU and throughput | within ±1.1% | up to 2.7% | No effect. |

Rejected variants, measured the same way (details in the record):

| Variant | Finding |
| --- | --- |
| No idle wake on local pushes | The wakes moved to the batch fan-out. Tokio has more voluntary context switches per request than Native (0.18 against 0.11–0.13). |
| Limit searching workers to half, as Tokio does | No change: 9.63/8.62 → 9.70/8.58 µs/request at 32/256 connections. |
| Keep every same-scheduler wake on the waking worker | No better than driver-only wakes, and channel wakes would move tasks, making io_uring borrowed waits re-create ring registrations. |
| Steal only from queues holding two or more tasks | At most 1% beyond driver-local wakes, and a single task behind a long poll would wait for that poll. |

## Decision and code disposition

Driver-local wakes were adopted in #105 with a regression test. Trade-off: a task's home is now, in effect, the worker that registered its socket, so balance relies on stealing at each event. Latency under uneven load was not measured. The rejected variants were not merged.

## Revisit conditions

Latency measurements under uneven load or saturated workers; an application pattern where the first read happens on a different worker from the rest.
