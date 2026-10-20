# Serial echo slowdown diagnosis

The alternating ready queues introduce a redundant worker self-notification in the idle path. Suppressing that notification removes an extra driver turn per echo and recovers the observed serial throughput loss in the targeted experiment. Skipping singleton queue swaps or reducing external-queue admission frequency did not recover it in their separate screen.

These are diagnostic variants of `b03e582`, based on the measured candidate `d1306df`. The original single-queue reference remains `ddc994d`. This record describes the causal investigation before a production fix. The [subsequent validation](../wake-validation/readme.md) applies self-interrupt suppression with regression tests and measures it across the concurrent matrix. Its results remain separate from the diagnostic samples below.

## Mechanism

With one worker and one connection, the owned echo path normally completes its write synchronously, then suspends on an io_uring read. There is approximately one runnable poll and one read completion per echo.

The original loop probes the driver before advertising that its worker is asleep. A completion harvested in that probe schedules the runnable while the worker is still active, so it needs no self-notification. After publishing the sleeping flag, the worker rechecks the queue and discovers that runnable.

The alternating loop instead performs its nonblocking driver turn after each task batch. On this serial path, it submits the read and subsequently harvests the read completion in an idle `turn(None)`, while the worker is advertised as sleeping. Publishing the completion schedules a local runnable through `schedule` → `Shared::wake_idle` → `WorkerState::wake`. That calls `Thread::unpark` and the driver waker on the same worker that is already processing completions.

The io_uring waker sets the thread-local `LocalRoute::notified` flag. The productive driver turn leaves that flag intact because its `progressed` condition short-circuits the later notification check. At the next otherwise-idle turn, the driver consumes this stale local notification and returns without waiting. The executor then repeats its empty queue searches and sleeping-state bookkeeping before it can actually wait.

This is a local permit and bookkeeping cost. It does not add an eventfd write per echo, a kernel wakeup per echo, or additional `io_uring_enter` calls. The original and alternating binaries both issue approximately two ring enters and one synchronous send per request.

## Counts and CPU evidence

Temporary counters were compiled into separate binaries, then run twice per variant with no warmup and three seconds of measurement. Counts cover the server lifetime, including negligible startup/shutdown work, and are normalized by completed client requests. Counter-instrumented throughput is not used as performance evidence.

| Per completed echo | Original loop | Alternating queues | Alternating, skip self-notification |
| --- | ---: | ---: | ---: |
| Runnable polls | 1.00 | 1.00 | 1.00 |
| Driver turns | 3.02 | 4.00 | 3.00 |
| Queue searches | 3.98 | 7.00 | 5.00 |
| Sleeping-state cycles | 1.98 | 3.00 | 2.00 |
| Successful worker wake notifications | <0.0001 | 1.00 | 0.00 |
| Ring submissions | 1.00 | 1.00 | 1.00 |
| Kernel wait calls | 1.00 | 1.00 | 1.00 |

The driver-turn counts include calls that return without a syscall. Restoring the original total number of driver turns does not restore the exact original loop: the alternating variant still performs approximately one more queue search per echo and swaps its queues.

Separate `perf stat` runs collect whole-server instruction and cycle counts plus syscall tracepoints. Three repetitions per cell show:

- Original → alternating: median instructions per echo rise from 26,677 to 27,711, about 3.9%; cycles rise from 15,139 to 15,674.
- Alternating → skip self-notification, in a second interleaved comparison: instructions fall from 27,747 to 27,005, about 2.7%; cycles fall from 15,698 to 15,367.
- Both comparisons retain approximately two `io_uring_enter` calls, one `sendto`, and one context switch per echo. Eventfd read/write counts remain at startup/shutdown scale.

Removing self-notification accounts for most of the observed instruction increase, with some remaining queue and control-flow overhead. Do not interpret the cycle difference as an exact attribution of end-to-end latency: client scheduling and kernel execution overlap, and instrumentation substantially perturbs this workload. Four separate sampled profiles locate user-space time in the worker loop but are too coarse to attribute this small regression by themselves.

## Uninstrumented ablations

All timing samples use the original fixed client, release builds with default optimization, one worker on CPU 0, one connection, 64-byte messages, client CPUs 4–7, one second of warmup, and three seconds of measurement. Each block randomly interleaves five repetitions per variant, under the existing exclusive benchmark lock. The blocks are separate and are not pooled.

The [first 30 timing samples](ablation/aiko.json) test the initial queue-bookkeeping hypotheses:

| Variant | Median requests/s | Observed range |
| --- | ---: | ---: |
| Original loop | 239,570 | 238,916–245,833 |
| Alternating queues | 233,221 | 228,787–235,736 |
| Skip queue swaps for singleton batches | 233,113 | 228,535–237,877 |
| Check external queues when empty or every 61 batches | 233,292 | 228,881–235,275 |
| Both of those changes | 230,725 | 226,107–232,411 |
| Raw Tokio control | 268,586 | 267,757–270,315 |

Neither proposed bookkeeping shortcut recovers the loss in this screen. That does not prove they have zero cost; it means they do not explain the regression on their own.

The [next 25 timing samples](wake-ablation/aiko.json), selected after inspecting counters, test the idle/wakeup mechanism:

| Variant | Median requests/s | Observed range |
| --- | ---: | ---: |
| Original loop | 233,395 | 228,559–240,703 |
| Alternating queues | 231,169 | 226,133–234,527 |
| Alternating, restore a pre-idle nonblocking probe | 235,761 | 232,247–239,299 |
| Alternating, skip self-notification | 241,230 | 238,413–244,778 |
| Raw Tokio control | 272,095 | 268,837–275,648 |

Skipping self-notification improves median throughput by 4.35% over the alternating loop in this block; their observed ranges do not overlap. It also exceeds the original loop's median in this selected block, but that reference is variable and its range overlaps. Restoring an extra probe partially recovers throughput, with overlapping ranges, while adding work. The more direct fix is to avoid notifying the currently executing worker about work it has just published.

The causal variant keeps the sleeping-flag exchange and skips `unpark` and the driver-waker call when the target thread is the current thread. It leaves remote wakeups and both ready queues intact. It is not a recommendation to add thread-ID checks throughout the hot path; local routing already knows which worker is executing and can inform a final implementation.

## Evidence and limits

The [summary](summary.json) is generated by [summarize.py](summarize.py). Raw timing, counter stderr, syscall counters, sampled profile summaries, build logs, binary SHA-256 identities, and source patches are preserved here. Raw perf data and frozen executables remain on Aiko under `~/.cache/core-scheduler-batch-diagnosis/`; they are not committed. The frozen original/candidate binaries are the same ones identified in the parent performance report.

- [profile.py](profile.py) runs timing and instrumented phases separately, using [sample-runner.py](sample-runner.py) and [supervise.py](supervise.py).
- [build-variants.py](build-variants.py) builds the singleton/admission ablations.
- [build-wake-variants.py](build-wake-variants.py) builds the idle-probe and self-wake ablations.
- [count-loops.py](count-loops.py) builds temporary diagnostic counters; [counter-runner.py](counter-runner.py) disables warmup so the counts and client request denominator cover the same workload.

The additional work was 55 uninstrumented timing samples, 12 `perf stat` samples, four sampled profiles, and six counter samples. All phases were serialized with `/tmp/core-scheduler-perf.lock`. Instrumented results are used for event counts and code attribution, never as capacity measurements.

The result identifies a concrete avoidable cost and a targeted recovery. These diagnostic samples do not establish regression-free performance across the earlier concurrent matrix, borrowed I/O, readiness selectors, other platforms, or general applications. The [follow-up](../wake-validation/readme.md) supplies regression tests for local wake suppression, remote wake races, and shutdown, full coverage, and concurrent performance comparisons for the actual fix.
