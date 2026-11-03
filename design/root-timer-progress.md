# Root Timer Progress

## Requirement

A Native timer registered outside this scheduler's workers must not depend on one particular worker to fire. Such root registrations include sleeps and reusable timers polled by `NativeScheduler::block_on`, by another executor such as `futures_lite::future::block_on`, or by another scheduler's workers. Their expiry must be collected promptly while any task worker of the scheduler is idle or between task batches, even if other workers are each held by a single long poll, such as a blocking call or a CPU-bound future.

An idle worker must not wait past the earliest pending root deadline unless another idle worker is bounded by it. A worker which leaves its idle wait to poll tasks must first ensure that a pending root deadline still bounds some other idle worker's wait, or is left for the next worker to go idle. Registering an earlier root deadline must interrupt a waiting worker, so it can shorten its wait. Registration, waker replacement, cancellation and release of a root timer must not wait for any worker to run.

Timers registered by a task on its own worker may stay in that worker's local queue, which only that worker collects. A long poll on a worker therefore still delays the timers of tasks registered on that worker, until the poll returns. A root timer is also delayed while every worker is held by a long poll; no worker policy can preempt a poll. This document requires no numerical latency bound.

## Rationale

Root registrations were assigned to a worker round robin and never migrated. A root thread sleeping for 100 ms while that worker ran a 2 s blocking poll waited for about 1.9 s, although another worker was idle. Callers outside a worker cannot choose a worker, and its long polls are not their own, so their timers must be served by whichever worker can run. The user directed that the defect be fixed rather than worked around.

Worker-local registration remains a deliberate locality property: a task's sleep registers in its worker's queue without synchronization or allocation, and that worker fires it between its own batches.

## Scope

The Native backend's `Clock` and `Timers` implementations, its worker loop and idle waits, with every selector it composes, including readiness and io_uring. The Tokio adapter keeps Tokio's timer. Root I/O readiness registrations are outside this document.

## Validation

Run `crates/native/tests/root_timers.rs`, which blocks some workers in a long poll and checks that root sleeps, reusable root timers and concurrent sleeps from other threads complete well before the poll returns. Its hand-off case lets an idle worker adopt a pending root deadline before a long poll starts, repeated so that the poll usually lands on the watcher; disabling the resignation makes it fail. Run it repeatedly, including under CPU load, since the property concerns progress rather than one interleaving.

Inspect the root queue's registration, expiry and closure under its lock, including that a closed worker drains without trusting the atomic deadline; the watcher claim before an idle wait; the resignation before a task batch; and the ordering between publishing a root deadline and checking the watcher, and between claiming the watcher and loading the deadline. The shared clock conformance suite runs both inside worker tasks and from root futures. Measure the task timer path and the idle/busy transition against the previous revision when changing this mechanism.
