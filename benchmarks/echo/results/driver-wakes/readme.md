# Keep driver wakes on the dispatching worker

With four workers, Native echo used more CPU per request than raw Tokio. This experiment measured where Native's work distribution moves tasks, and changed one routing rule: a task woken during a worker's own driver turn is queued on that worker, rather than sent to the worker that last polled it. Wakes from task polls and from other threads keep the last-poller rule. Initial placement, stealing, idle wakes and the publication, fence and recheck protocol are unchanged.

## Why tasks crossed workers

A readiness registration belongs to the epoll controller of the worker that first waited on the socket, and a pending io_uring operation to the ring it was submitted on. After an idle worker stole a task, the task's later events were still dispatched by the original worker. That worker then pushed the task into the thief's inbox (`Injector::push`), often woke it, and the task was polled again on the thief's core. So once a task had been stolen, every later event crossed workers, until the task was stolen back.

A temporary diagnostics build ([diagnostics.patch](diagnostics.patch)) counted polls, migrations (polls on a worker other than the previous one), remote and local schedules, idle wakes by source, parks and steals. It also switched routing policies with an environment variable, so all policies were compared in one binary, which avoids build-layout differences. Build it with `cargo build --release -p base-scheduler-benchmark-echo --features diagnostics`; `BASE_WD_POLICY` selects the policy bits: 1 keeps every same-scheduler wake on the waking worker, 2 skips the idle wake after a local push, 4 steals only from queues holding two or more tasks, 8 limits searchers to half the workers, and 16 keeps driver-turn wakes local. [screen.py](screen.py) interleaves policies and reports medians of five samples per cell (three for the second table). Values are per completed poll, from Hana (Xeon E3-1240 v6), with 4 workers on CPUs 0–3 and the client on CPUs 4–7.

| 4 workers, native-poll | 32c baseline | 32c driver-local | 256c baseline | 256c driver-local |
| --- | ---: | ---: | ---: | ---: |
| CPU µs per request | 9.53 | 9.24 | 8.52 | 8.16 |
| Remote schedules from a worker | 0.75 | 0 | 0.77 | 0 |
| Migrations | 0.41 | 0.68 | 0.24 | 0.64 |
| Steals (ready queues + inboxes) | 0.41 | 0.54 | 0.22 | 0.36 |
| Idle wakes / parks | 0.22 / 0.23 | 0.28 / 0.27 | 0.06 / 0.07 | 0.09 / 0.09 |

On main, three quarters of native-poll schedules were remote pushes from another worker's driver. A user-space `perf record -e cycles:u` profile of main (native-poll, 4 workers, 256 connections) attributes about 9% of samples to `Injector::push` and the inbox `steal_batch_with_limit` paths, plus about 2% to `Stealer` steals and 1% to epoch pinning. Raw Tokio's whole queue, steal and notification cost is about 4%. With driver-local routing, all remote pushes from worker drivers disappear. Migrations and steals increase, because a stolen task now returns to its controller on its next event, but those moves are local pushes without a wake, and CPU per request falls. With io_uring, every schedule was already local, because a ring completes operations for the worker that submitted them, so its counters and CPU did not change.

Context switches do not explain the gap with Tokio: raw Tokio had more voluntary context switches per request (0.18 at 32 connections) than Native (0.11–0.13). The gap is mostly user time (2.5 against 1.8 µs per request at 256 connections), with about 0.5 µs more system time.

## Rejected variants

Native-poll CPU µs per request at 32 / 256 connections, each against the baseline of the same screen run in the same binary:

| Variant | Baseline | Variant | Outcome |
| --- | ---: | ---: | --- |
| No idle wake on a local push, relying on the batch fan-out wake (3 samples) | 9.59 / 8.56 | 9.61 / 8.63 | Wakes moved to the batch fan-out; steals, parks and migrations unchanged |
| At most half the workers searching at once, Tokio's limit (3 samples) | 9.63 / 8.62 | 9.70 / 8.58 | No change |
| Every same-scheduler wake stays on the waking worker (5 samples) | 9.53 / 8.52 | 9.35 / 8.29 | No better than driver-local (9.24 / 8.16), and would also move tasks on cross-worker channel wakes, where io_uring borrowed waits must retire and re-create their ring registration |
| Driver-local plus stealing only from queues holding two or more tasks (5 samples) | 9.53 / 8.52 | 9.14 / 8.20 | At most 1% beyond driver-local, with a single task queued behind a long poll left to wait for that poll; not adopted |

So the extra idle-worker wakes in hypothesis 1 were not the cost: most steals come from workers that have run out of local work, not from workers that were woken.

## Comparison

`quick.py echo` on Hana compares main (`140cc25`) with the change, with raw Tokio and a layout control (main's source built at another path) in the same rounds. Values are medians of five rounds; "better" counts rounds that improved on main.

[Run 1](quick-4w-1.jsonl):

| Case | Backend | CPU µs/req main → change | Change (better) | Layout | Requests/s change | Layout |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 4w 32c | native-poll | 9.503 → 9.31 | −1.6% (5/5) | +1.4% (1/5) | +0.8% (3/5) | −2.1% (0/5) |
| 4w 32c | iouring | 9.701 → 9.636 | −0.5% (5/5) | −0.8% (4/5) | +1.2% (3/5) | +1.8% (4/5) |
| 4w 256c | native-poll | 8.614 → 8.364 | −2.9% (5/5) | −0.8% (3/5) | +1.3% (5/5) | +0.9% (4/5) |
| 4w 256c | iouring | 8.17 → 8.201 | +0.4% (0/5) | +1.1% (0/5) | −0.5% (1/5) | −1.5% (1/5) |

[Run 2](quick-4w-2.jsonl), with the owned readiness backend added:

| Case | Backend | CPU µs/req main → change | Change (better) | Layout | Requests/s change | Layout |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 4w 32c | native-poll | 9.554 → 9.261 | −3.1% (5/5) | −0.3% (4/5) | +1.7% (4/5) | +0.4% (3/5) |
| 4w 32c | native | 10.35 → 10.29 | −0.6% (3/5) | −0.6% (3/5) | −0.8% (1/5) | −0.7% (1/5) |
| 4w 32c | iouring | 9.642 → 9.636 | −0.5% (3/5) | −0.3% (4/5) | +0.1% (3/5) | +0.7% (4/5) |
| 4w 256c | native-poll | 8.478 → 8.356 | −2.5% (4/5) | +1.1% (1/5) | +0.4% (3/5) | −1.2% (1/5) |
| 4w 256c | native | 9.034 → 8.882 | −1.7% (5/5) | −0.3% (4/5) | +1.1% (5/5) | +0.1% (3/5) |
| 4w 256c | iouring | 8.104 → 8.15 | +0.6% (1/5) | +0.9% (0/5) | −0.3% (2/5) | −0.8% (0/5) |

Native-poll CPU per request falls by 1.6–3.1% in both runs, beyond the layout control and in a direction the layout control did not share. Throughput changes are within layout noise, except owned Native at 256 connections in run 2. io_uring does not change. Raw Tokio measured 7.32–7.33 µs per request at 256 connections and 8.41–8.60 at 32, so most of the gap remains.

[One worker](quick-1w.jsonl), where routing cannot change, as a cross-check: every CPU and throughput change is within ±1.1%, and the layout control varies by up to 2.7%.

## Correctness

The flag is thread-local, set only around `SelectorWorker::turn`, and takes effect only when the waking thread is a worker of the task's own scheduler. Such a wake takes the existing local path: the task is pushed to this worker's ready queue, which this worker searches before it next waits, and `wake_idle` runs as before. In an idle turn, that wake clears the worker's own sleeping flag without an interrupt; between batches, it may wake another idle worker to steal. External threads, task polls and stall rescues keep the last-poller and stuck-worker rules. A waker callback that spawns during a driver turn would leave the new task's first runnable on the dispatching worker instead of its admitted worker; the scheduler's own drivers do not do this.
