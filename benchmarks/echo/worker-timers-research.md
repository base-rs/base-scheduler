# Worker-driven timer research

Driving the existing timer queue from Native’s I/O workers removes one thread and can improve timer latency. Having every worker drive the **same queue** also introduces a substantial CPU cost under concurrent timer load. This experiment does not justify adopting that shared-queue design. Production behavior remains unchanged; the next useful implementation experiment is one timer queue per worker, paired with its ring.

The [validated measurements](results/worker-timers/tables.md), [raw evidence and reproduction instructions](results/worker-timers/readme.md), and [diagnostic patch](results/worker-timers/candidate.patch) preserve the experiment. The patch is not applied to the workspace.

## How the timeout fits into the loop

Native already uses `core-scheduler-timers`, but through a lazy dedicated timer thread. That thread collects expired registrations and wakes their tasks. Native’s task workers collect I/O completions after each ready batch and wait indefinitely when idle; the timer thread interrupts them when a timer expires.

The candidate reuses the same queue and registration semantics. Workers collect due timers at batch boundaries and drop the collected wake values outside queue locks, publishing tasks through the normal ready queues. The I/O turn after a ready batch stays nonblocking. Only an idle worker derives a timeout from the queue’s next published deadline and passes it to `SelectorWorker::turn`. With no timers, that remains an indefinite wait.

For io_uring, the timeout bounds the ring’s completion wait using extended wait arguments. I/O and external notifications can end that wait before the deadline. Registering an earlier timer interrupts an idle worker so it can recompute its timeout. Cancelled timers publish an immediate cleanup hint, allowing reserved capacity to be reclaimed before returning to a long wait.

The candidate’s `thread::park_timeout(timeout)` branch runs only **without an active selector driver**, including after driver retirement. It waits for a thread notification or the deadline; `unpark` can end it early. It does not replace the live ring’s I/O wait. Zero timeout remains a no-op in this fallback, preserving the ordinary executor’s behavior after productive batches.

The sleeping-state publication and final work recheck still surround the idle wait. Deadline notification must participate in that handshake: an earlier registration must either be visible to the timeout calculation or retain an interrupt for the impending wait. Shutdown also waits for already-claimed timer wake delivery outside the queue lock.

## Timer behavior

With the host’s default timer slack, the median of three repeated samples gives:

| Workers / timers / requested delay | Before p50 lateness | Integrated p50 lateness | Before / integrated process CPU seconds |
| --- | ---: | ---: | ---: |
| 1 / 1 / 1 ms | 52.51 µs | 1.35 µs | 0.013 / 0.005 |
| 1 / 32 / 1 ms | 53.15 µs | 1.58 µs | 0.058 / 0.053 |
| 1 / 1,024 / 1 ms | 67.26 µs | 1.76 µs | 1.171 / 1.398 |
| 4 / 1 / 1 ms | 53.89 µs | 1.66 µs | 0.017 / 0.028 |
| 4 / 1,024 / 1 ms | 58.75 µs | 2.91 µs | 4.100 / 7.698 |

CPU totals cover the whole process: one second of warmup, three measured seconds, startup and shutdown. Lateness is measured when the task resumes. Each iteration requests a fresh deadline relative to its current time, so lower lateness itself increases the expiry rate. For example, the last row completes approximately 967,212 versus 1,020,311 measured expiries/s; that 5.49% rate increase does not explain a roughly 88% CPU increase. It is not a fixed-arrival-rate comparison or a maximum queue-capacity measurement.

The thread snapshots confirm one fewer thread: two versus three with one worker, and five versus six with four workers. Unchanged Native readiness controls retain their dedicated timer thread and reproduce the baseline’s latency behavior. Tokio controls use a different timer precision policy and are recorded separately; their expiry rates are not equal-precision capacity comparisons.

## Timer precision control

The inherited Linux timer slack on Aiko is 50,000 ns. Linux can coalesce thread timer wakes within their slack allowance; the policy is inherited by new threads and across execution. Consequently the initial roughly 50 µs difference must not be attributed entirely to removing a thread and its cross-thread wake. See the [Linux API documentation](https://man7.org/linux/man-pages/man2/pr_set_timerslack.2const.html).

With **both versions inheriting 1 ns slack**, a single worker’s 1 ms timer improves from 2.50 µs to 1.36 µs median lateness. Thus most of the original 52.51 → 1.35 µs difference was coalescing policy, while a smaller integration benefit remains. Whole-process CPU is 0.013 → 0.005 seconds. At a requested 10 µs delay, median lateness falls from 3.25 to 1.32 µs and CPU falls from 0.878 to 0.347 seconds, despite more completed iterations. Single-worker integration has a useful benefit in this workload.

The four-worker shared-queue cost remains after matching slack. With 1,024 timers, median lateness improves from 8.41 to 2.76 µs, but process CPU rises from **5.700 to 7.705 seconds**—about 35%—for only about 0.53% more measured expiries. With one timer and four workers, CPU also rises from 0.018 to 0.028 seconds. Matching precision reduces the apparent penalty without removing the shared-worker coordination problem.

This control does not propose changing an application’s timer-slack policy. It establishes that precision/coalescing and integration overhead need separate comparisons.

## Echo throughput

The echo screen contains no application timers. It tests whether checking the empty timer queue affects ordinary I/O, using identical optimized before/after builds and the same fixed client. These are median requests/s from three randomized repetitions per cell:

| Workers / connections / bytes | Owned before → integrated | Change | Borrowed before → integrated | Change | Unchanged Tokio control change |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 / 1 / 64 | 238,126 → 238,440 | +0.13% | 234,440 → 234,035 | −0.17% | −1.73% |
| 1 / 32 / 64 | 636,348 → 633,676 | −0.42% | 574,496 → 568,487 | −1.05% | +2.03% |
| 4 / 128 / 64 | 2,090,038 → 2,096,315 | +0.30% | 2,124,696 → 2,115,931 | −0.41% | −0.68% |

This does not demonstrate a useful networking improvement. Native differences are small relative to the control variation; three short repetitions do not prove equivalence at sub-percent precision. The candidate’s empty-queue path adds only the deadline check at worker-loop boundaries, without calling `Instant::now` or taking the timer lock when no deadline is published.

## Where the shared design costs more

Every idle worker observes the same published deadline. Several workers can therefore time out together, attempt to collect from the same mutex-protected queue, and recompute their waits. When a deadline is due but collection loses the queue lock, the conservative hint can also keep the next turn nonblocking until another worker refreshes it. The candidate additionally counts concurrent wake-delivery batches for shutdown. These are implementation costs, not requirements of the public clock interface.

Separate whole-process counter runs support the extra-wakeup concern. These are medians of two instrumented runs, kept separate from the uninstrumented rates above:

| Workers / timers / delay | Before → integrated ring enters | Before → integrated context switches | Before → integrated write syscalls |
| --- | ---: | ---: | ---: |
| 1 / 1 / 1 ms | 7,600 → 3,989 | 11,410 → 3,994 | 3,802 → 3 |
| 4 / 1 / 1 ms | 7,585 → 26,860 | 11,410 → 18,123 | 3,799 → 7,721 |
| 4 / 1,024 / 1 ms | 213,104 → 1,076,819 | 295,780 → 832,803 | 107,158 → 308,364 |

With one worker and one timer, integration removes almost all interrupt writes and substantially reduces waits and context switches. With four workers, those savings reverse. The 1,024-timer case makes about five times as many ring entries. In addition to workers waiting on the same deadline, publishing an earlier deadline can interrupt an idle peer even when the registering worker is active. These counts are consistent with redundant worker coordination; they do not identify every write’s destination or isolate individual mutex costs. Instructions also rise from about 16.43 billion to 29.41 billion in the large four-worker case.

The CPU increase is sufficient to reject the shared-worker prototype. Shared-deadline wakeups and queue contention are the leading explanations from its structure and measurements; the experiment does not isolate the cost of each lock or atomic operation.

## Next implementation experiment

Pair each worker’s ring with its own `WorkerTimers` queue. Only its owning worker uses that queue’s deadline to bound its idle wait. Normal registration and expiry then avoid a shared queue and shared deadline. Keep opportunistic assistance separate from idle-timeout ownership so assistance does not recreate the same wakeup herd.

A sleep should choose and retain its timer queue on first registration, just as an I/O request retains its chosen ring. Moving its future or task between workers must not invalidate or migrate the existing registration. Expiry publishes a runnable task, which remains stealable. External callers choose a queue explicitly inside the scheduler implementation, and an earlier deadline or cancellation cleanup notifies that queue’s owning worker. This requires no public scheduler-interface change and no implicit scheduler lookup by protocol objects.

Before proposing that implementation, establish the reservation/capacity policy across queues, verify registration-versus-parking races and driver retirement, preserve cancellation and shutdown delivery guarantees, and test mixed timer/I/O load. The current echo screen contains no application timers, so it cannot establish timer latency under sustained network load.

The prototype passes 57 Native unit tests on both Linux and macOS, including new earlier-deadline and shutdown-delivery tests. Its macOS coverage remains incomplete, so it is research evidence rather than a production-ready patch. After restoring the production implementation, the shared Bake Native coverage task passes **1,335/1,335 measured regions**. [Verification details](results/worker-timers/verification.md) distinguish those results.
