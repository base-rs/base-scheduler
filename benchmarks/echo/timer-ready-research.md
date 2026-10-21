# Publishing expired timers into ready queues

This archived experiment predates the [production worker-local timer integration](worker-local-timers.md). Its baseline and candidate descriptions apply to the measurements below.

Queuing stealable timer-delivery actions costs more than immediately waking their tasks in the tested Aiko workloads. Batching publication and allowing the stealing worker to keep the resulting task reduce the penalty, but do not produce a win. Keep timer queues worker-owned and immediately publish the resulting runnable tasks through their registered wakers.

This extends the [worker-owned timer experiment](local-timers-research.md). The baseline here is that archived worker-local prototype, rather than the production shared timer service. The [patches, scripts, raw data, and generated tables](results/timer-ready/readme.md) preserve five implementations and 171 samples. Production source remains unchanged.

## What can be stolen

The worker-local baseline already does:

```text
owner pops expired timer
  → invokes registered waker
  → async-task publishes Runnable into a ready queue
  → owner or stealing worker polls the task
```

The proposed additional stage does:

```text
owner pops expired timer
  → publishes Ready::Timer(waker) into a ready queue
  → owner or stealing worker invokes that waker
  → async-task publishes Runnable into a ready queue
  → owner or stealing worker polls the task
```

The additional stage makes *wake delivery* stealable. It is unnecessary for stealing the actual resumed task: both baseline ready queue generations are already stealable, as are preferred-worker incoming queues.

Invoking the registered waker is necessary. A timer polled inside a Native task can have a combinator's waker, such as a child waker supplied by `FuturesUnordered`. Waking the enclosing task directly can bypass the combinator's readiness bookkeeping. Even for a task's own waker, async-task's scheduling transition coalesces wakes and preserves single-poller execution. These experiments retain that transition and never poll application futures directly from timer collection.

## Refined timer comparison

All builds use release optimization, locked dependencies, Rust 1.99.0 / LLVM 23.1.1, and matching 1 ns Linux timer slack. The following are medians of three randomized repetitions from the fresh confirmation screen. CPU seconds cover the entire process, including one second of warmup and three measured seconds.

| Workload | Immediate wake CPU | Batched ready publication CPU | Batched publication with local task routing CPU |
| --- | ---: | ---: | ---: |
| 1 worker / 1 timer / 10 µs | 0.3365 | 0.3724 (+10.7%) | 0.3736 (+11.0%) |
| 1 worker / 32 timers / 1 ms | 0.0452 | 0.0515 (+14.0%) | 0.0527 (+16.8%) |
| 4 workers / 1,024 relative timers / 1 ms | 1.6245 | 1.6610 (+2.2%) | 1.6881 (+3.9%) |
| 4 workers / 1,024 aligned timers / 1 ms | 1.2915 | 1.6814 (+30.2%) | 1.6197 (+25.4%) |

The synchronized-expiry result is the clearest penalty: immediate wake p99 lateness is **79.68 µs**, versus **104.70 µs** with batched ready publication and **98.62 µs** with local task routing. Every variant maintains the configured 1,024,000 expiries/s in that case. The immediate CPU samples span 1.286–1.306 seconds, versus 1.602–1.750 and 1.616–1.647 seconds for the alternatives, so this difference exceeds the observed repetition spread.

The relative 1,024-timer CPU samples overlap; the small median differences there do not establish a reliable percentage penalty. Low-load CPU overhead and synchronized-expiry lateness are more consistent evidence against the additional stage. Very short-delay p99 values also vary sharply between repetitions and should not be treated as a stable latency ratio.

The optimized ready variant collects due wakers under one timer borrow, uses one outstanding-count increment for the batch, publishes under one worker borrow, and notifies an idle worker once. The local-routing variant additionally routes same-scheduler tasks synchronously awakened during timer delivery onto the delivering worker, avoiding a return to their previous preferred worker. Its scoped routing state is restored even if a waker panics. Neither refinement eliminates the extra ready-queue pass.

## Network checks

Without application timers, the optimized screen has no consistent improvement. Single-connection throughput is 236,570/s for immediate waking, 232,255/s for batched ready publication (−1.82%), and 237,038/s with local routing (+0.20%). Four-worker / 128-connection medians differ by +0.09% and −0.47%. The repetition ranges overlap. This screen does not isolate queue width versus dispatch costs or resolve the earlier worker-local prototype's empty-loop regression.

Mixed echo with 32 relative 1 ms timers shows no material throughput improvement from the added stage. In the optimized screen, batched publication changes throughput by +0.18% with one worker / 32 connections and −0.08% with four workers / 128 connections. Local routing changes those medians by +0.66% and +0.23%, respectively; the repetition ranges overlap.

Timer latency is worse in that screen: with one worker, p99 lateness is 46.11 µs for immediate waking, 58.14 µs for batched ready publication, and 57.47 µs with local routing. With four workers, those values are 37.15, 41.22, and 42.30 µs. The initial individual-publication screen also shows essentially unchanged echo throughput and higher timer tail latency.

## Initial screen and simpler batching

The initial implementation published individual `Ready::Timer` entries. With four workers and 1,024 relative timers it used 1.886 versus 1.577 CPU seconds (+19.6%). With aligned deadlines it used 2.009 versus 1.304 seconds (+54.1%), with p99 lateness 126.14 versus 80.45 µs. The optimized comparison above materially improves on this straightforward implementation, but still favors immediate wake delivery.

A separate variant collected due timers under one borrow into reusable storage and then woke them immediately. It retained the original ready queue element type and ordinary scheduling. It was mixed: relative 1,024-timer CPU fell from 1.577 to 1.478 seconds, but aligned-timer CPU rose from 1.304 to 1.459 seconds and p99 lateness rose from 80.45 to 89.66 µs. It did not establish a general improvement worth adopting by itself.

The CPU-heavy aligned workload performs 10,000 black-boxed integer recurrence steps after each expiry. All three initial variants saturate the four CPUs, using approximately 16 process CPU seconds across the four-second lifecycle. Rates are 530,245/s for immediate wake, 528,263/s for queued delivery (−0.37%), and 532,230/s for collection followed by immediate wake (+0.37%). There is little benefit from distributing the cheap wake callbacks once application work dominates. This is a synthetic load, not evidence about arbitrary applications.

## Why the extra stage costs

The queued implementation adds a ready-queue pass before the ordinary task scheduling pass, plus accounting to keep timer actions alive through shutdown. Its `Ready::Task(Runnable) | Ready::Timer(Waker)` enum enlarges every ready entry from 8 to 24 bytes on the measured target. The batched variants also reserve another 16 bytes per timer slot per worker for reusable waker storage. No per-timer box or auxiliary task is allocated.

The initial preferred-worker routing can send a task awakened by a stolen timer action back to its previous worker. Local routing addresses that handoff, but the confirmation screen shows that it does not remove the synchronized-expiry penalty. The original immediate wake path also lets other workers steal tasks while the owner is still processing the rest of its due timers.

Two separate `perf stat` repetitions of the relative 1,024-timer workload show about 13.15 billion whole-process instructions for immediate delivery, 14.00 billion for individual queued delivery, and 12.27 billion for collection followed by immediate delivery. The queued variant has fewer median ring entries and context switches in those runs, despite consuming more CPU in the uninstrumented screen. These diagnostics do not support attributing its regression simply to extra blocking syscalls. They do not isolate the individual contributions of wider queue entries, atomics, cache effects, and dispatch.

## Scope

The changes are archived experiments. The added tests verify stealing, cancellation/drop after expiry collection, shutdown accounting, allocation-free reusable batching, callbacks outside queue borrows, and task routing. Native and io_uring suites pass for all additional variants. Full source-region coverage and the original worker-local prototype's adoption work remain outstanding; see the [evidence notes](results/timer-ready/readme.md).

The useful design is still an owner-local timer queue whose earliest deadline bounds that worker's interruptible I/O wait. Remove due timers before delivering wakeups, and let the ordinary stealable ready queues distribute the resumed tasks. Adding another stealable delivery stage is unnecessary for that model.
