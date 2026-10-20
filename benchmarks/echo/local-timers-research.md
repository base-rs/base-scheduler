# Worker-owned timer experiment

Worker-owned timers reduce process CPU by about 73% in the large periodic-timer test and improve mixed timer/echo throughput by 5–13% on Aiko. Single-connection echo without timers still needs investigation before production adoption.

The prototype gives each Native task worker its own unsynchronized timer queue, paired with its I/O driver. Registration, cancellation, and expiry on that worker manipulate ordinary local state. Its earliest deadline bounds its own idle wait. Expiry removes the timer before waking the task, and wakeups enter the executor's ordinary ready queues.

The implementation and measurements are preserved in [the research evidence](results/local-timers/readme.md). The [candidate patch](results/local-timers/candidate.patch) is archived, not applied to production source.

A [follow-up experiment](timer-ready-research.md) compares immediate wake delivery with publishing expired timer wakers as stealable ready actions, including batched publication and local task routing. Immediate delivery already makes the resulting tasks stealable; the extra delivery stage adds measurable cost in synchronized-expiry workloads.

## Timer results

Optimized Aiko builds, matching 1 ns Linux timer slack, and medians of three randomized repetitions give:

| Workers / timers / delay | Before → local p50 lateness | Before → local CPU seconds |
| --- | ---: | ---: |
| 1 / 1 / 10 µs | 3.28 → 1.30 µs | 0.885 → 0.341 |
| 1 / 1 / 1 ms | 2.50 → 1.31 µs | 0.0128 → 0.0048 |
| 4 / 1,024 / 1 ms | 8.46 → 2.10 µs | 5.702 → 1.541 |

The large four-worker case uses **73.0% less process CPU** while completing about 0.64% more measured expiries: 1,015,030 → 1,021,492 per second. Its p99 lateness falls from 20.48 → 13.09 µs. Readiness-backed Native shows a similar reduction. Thread snapshots show one fewer thread, confirming that these worker-local workloads do not start the fallback timer thread.

CPU covers the whole process: one second of warmup, three measured seconds, startup and shutdown. Each task requests a new relative deadline after its previous wait completes, so the rate partly reflects lateness. This is not a maximum-throughput or fixed-arrival-rate comparison. Matching slack isolates this experiment from the kernel wake-coalescing policy that dominated the first timer experiment.

## Echo without application timers

The final no-timer echo screen shows essentially unchanged throughput at higher concurrency: −0.27% / +0.03% for owned/borrowed I/O with one worker and 32 connections, and +0.14% / −0.18% with four workers and 128 connections. The unchanged Tokio control varies by −0.85% and −0.36% in those cells, respectively.

Single-worker, single-connection echo remains slower: −2.82% owned and −5.21% borrowed, versus −0.82% in the Tokio control. Three short repetitions do not isolate every source of variation, but this is a material regression to investigate before adoption. Removing the empty queue's clock read was insufficient; the queue still enters its flush machinery when empty. A separate 24-sample follow-up added a direct empty check before that machinery. It did not establish an improvement: owned I/O medians were 236,722 → 236,608 requests/s, and borrowed medians were 239,290 → 234,648. The unchanged Tokio control differed by +0.19%. The baseline and candidate also varied between the two screens, particularly for borrowed I/O. This guard neither isolates nor reliably fixes the single-connection cost; its [additional patch](results/local-timers/empty-fast.patch) and measurements remain separate diagnostics. Profiling the empty worker loop is still needed.

## Echo with active timers

Adding 32 periodic 1 ms timers alongside the echo tasks improves network throughput in every tested cell:

| Workers / connections | Owned I/O throughput | Borrowed I/O throughput | Owned timer p99 lateness | Borrowed timer p99 lateness |
| --- | ---: | ---: | ---: | ---: |
| 1 / 32 | +6.56% | +5.37% | 646 → 46 µs | 622 → 54 µs |
| 4 / 128 | +13.16% | +6.62% | 3,799 → 26 µs | 3,162 → 43 µs |

Both variants inherit 1 ns slack. Timer histograms cover deadlines after the server's first second until echo completes, approximating the client's measured window. These results support worker-local timer integration under mixed load; they do not establish a general application speedup or maximum timer capacity.

Separate instrumented runs also show the expected reduction in coordination. With four workers and 1,024 periodic timers, whole-process median futex calls fall from about 1.48 million to 680, ring entries from 1.62 million to 299 thousand, and context switches from 1.18 million to 179 thousand. These are two-run `perf stat` diagnostics, separate from throughput measurements. They are consistent with removing shared queue contention and timer-thread handoffs, without isolating each mechanism's contribution to CPU savings.

## Implementation

The experiment separates the existing `core-scheduler-timers` slot/heap algorithm from its shared `Arc<Mutex<_>>` wrapper. An experimental `local::TimerQueue` owns that state directly and returns reusable registration keys. Native installs one queue per worker in private thread-local state. Sleep futures retain a key and owning worker identity, without retaining thread-local references. Registration, repolling with the same waker, cancellation, and expiry need no queue mutex or per-timer allocation within reserved capacity.

A future may move to another thread while registered. Its existing registration stays on the original worker. Waker changes and cancellation/drop from another worker use the original owner's command inbox. Only these remote commands acquire the scheduler's admission lock, so shutdown cannot strand a command behind worker retirement. If expiry or closure overtakes a waker replacement, the replacement must also be woken; dropping that replacement silently would lose progress.

The worker processes a snapshot of remote commands, removes due timers, releases the local queue borrow, and wakes tasks. Application futures still run through `async-task::Runnable`, preserving single-poller safety and work stealing. Expiry does not poll arbitrary application futures inside a queue borrow. New timers scheduled by those resumed tasks enter a later queue pass.

Only the owning worker uses its next deadline to limit an idle I/O turn. The turn after a ready batch remains nonblocking. With no timers, checking the queue does not read the clock. With an active io_uring driver, the timeout bounds the interruptible ring wait. Without an active driver, `thread::park_timeout` waits for an unpark notification or deadline. Closure is rechecked before waiting, and shutdown waits for workers to finish delivering local timer wakeups.

The correspondence with Ruby is direct: `Async::Scheduler` owns `IO::Event::Timers`, derives `wait_interval` before `select`, and fires removed timers afterward. Rust publishes runnable tasks through their wakers instead of transferring fibers inline. The finite ready batches and I/O completion batches remain unchanged.

## Scope and adoption work

This is an opt-in experiment using Native's `worker-timers` feature and Timers' `local` feature. It is not a complete replacement for the production timer service:

- Sleeps first polled outside a matching Native worker still use the original lazy timer thread and shared queue. Routing those registrations through a worker inbox remains to be implemented. Moving an already-registered sleep is supported.
- Each worker reserves `timer_capacity`, in addition to the external fallback queue. This increases reserved memory and changes the capacity accounting from one shared limit to a limit per queue. A production change must document and test that policy.
- `Clock::now` retains the original clock wrapper; ordinary production time reads can use `Instant::now` directly once paused-clock support is separated.
- The local queue's public experimental API needs review before becoming supported API. Cancellation reclamation and capacity pressure need stronger tests, including non-earliest cancellations and exhausted queues.
- The existing deterministic Clock conformance fixture still drives the original paused queue. New tests exercise the local path, but a worker-aware deterministic fixture and full local-path conformance coverage remain necessary.
- The prototype has incomplete source-region coverage. Production source remains unchanged, with its coverage checked separately.

These measurements support finishing worker-local timer integration. They establish a useful reduction in timer coordination and mixed-load latency, while preserving the remaining lifecycle, capacity, conformance, and empty-loop performance work explicitly.
