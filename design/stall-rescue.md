# Stall Rescue

**Status: proposed.** These requirements are not active yet. Requirement 1 applies once the staged implementation below lands. Requirement 2 is the interface's part and is reflected now in the context wording changes that accompany this document. Open questions are listed at the end and are not requirements.

## Requirement

### 1. Stranded ready work progresses while a worker is idle (Native quality of implementation)

In the Native backend, ready work held by a stuck worker must progress within a bound *T* while any other task worker of the same scheduler is idle, without the application doing anything. A stuck worker is one that has not reached a turn boundary (its timer pass and driver turn in `run_loop`) for longer than the detection threshold, and is not idle. Ready work means:

- **Expired timers** in the stuck worker's queue, including sleeps and reusable timers registered by root futures or other executors and routed there, and registrations made by tasks that were later stolen by other workers.
- **Readiness events** queued in the stuck worker's epoll or kqueue poller, for sockets registered on that worker by its own tasks, by tasks later stolen, or by root and external callers routed there.
- **Runnable tasks** in the stuck worker's ready queues and remote inbox.

"Progress" means the waker is invoked and the task becomes runnable and stealable, so an idle worker can poll it (or, for a root future, its `block_on` thread is woken). The poll that is stuck is never preempted. Runnable tasks already satisfy this through stealing, without a bound on detection (see [runnable tasks](#runnable-tasks)).

*T* is the worst-case delay from work becoming ready to its waker being invoked, assuming the rescuing thread gets scheduled. It is the sum of the stall threshold and the sampling period, plus the bounded rescue work itself. *T* has a documented default and may be configurable through backend options. Its value is an open question; this document does not set a numerical default.

`Blocking::spawn_blocking` remains the way to declare blocking work up front. Rescue is a fallback for blocking that wasn't declared, not a replacement for `spawn_blocking`, and it does not make blocking inside a task free. A task that blocks still holds its worker's thread and delays everything else that worker would have run.

This is a property of the Native backend's implementation. It is not part of the portable interface's contract, and it does not apply to the Tokio adapter, which has its own driver model. In-flight io_uring operations are excluded until the [io_uring decision](#io_uring) is made. Stage 1 must document that exclusion as backend-specific behaviour.

Rescue must preserve every existing ownership contract: cancellation, owned-buffer retention through original retirement, the rule that borrowed payloads are accessed only within a poll, registration reclamation at batch boundaries, and graceful shutdown. Rescue must never poll an application future from completion or timer processing ([worker progress](worker-progress.md)). It must call wakers outside every queue, table and poll lock.

### 2. The interface permits completion, re-routing and rescue on any worker or thread

The portable interface (the `base-scheduler` crate, its rustdoc, `context/interface.md`, the contract sections of `context/design.md`, and the conformance suite) must not require, imply, or test that a particular worker or thread services an operation, registration, timer or waker. A backend may publish completion, invoke a waker, fire a timer, drain readiness, cancel, re-route, or migrate in-flight work from any of its threads. It must still keep each operation bound to its selected scheduler and keep the ownership contracts in requirement 1. Where current backends do keep work on its original worker or ring, the documentation must present that as a backend implementation choice, not as a portable guarantee.

The selector hooks in `base-scheduler-selector` are public, but they are not the portable interface. They currently describe a driver as exclusively owned by one worker (`SelectorWorker`, `SelectorDriver::enter`). To implement requirement 1 for readiness, they need an optional addition that lets another thread make bounded nonblocking progress for a worker's driver. Selectors that can't support it (io_uring under its current flags) must be able to decline. This addition is described under [interface changes](#interface-changes) and is not made by this document.

## Rationale

Native workers each own a timer queue (`base_scheduler_timers::local`, `crates/native/src/worker_timers.rs`, "Keys never migrate"), a readiness poller (`crates/selector/src/readiness_driver.rs`: `DriverGroup::route` assigns external callers round robin, the route lasts for the socket's lifetime, and each worker's table is a `Mutex<Table>`), and, with io_uring, a ring (`crates/iouring/src/io_uring_selector.rs`: `route_ref` and `bind_worker` with `SINGLE_ISSUER | DEFER_TASKRUN`). Tasks are stolen, but timers and registrations are not. A worker stuck in a long poll therefore strands its timers and readiness events while other workers sit idle. On `main`, a 100 ms root sleep took 1.94 s behind a 2 s blocking task, and a root socket read took 1.95 s.

[PR #92](https://github.com/base-rs/base-scheduler/pull/92) fixed root timers only, with a shared root queue and a watcher role handed off before each batch. Review measured about +15% wall time and +28% CPU on spawning whenever any root timer is pending, which is common: `block_on(main)` holding a timeout is enough. The maintainer parked #92 in favour of a general design, which this document proposes.

The maintainer's position:

> Tasks represent units of work … work stealing/task migration is expensive, so we want to avoid it, but in the case where something blocks (which we can't know ahead of time), it's our only real option to reduce latency, if other workers are not already busy.

> Performance is king, but work stealing is part of that equation. If we know something is going to block, we can state that clearly, e.g. using something like `spawn_blocking`.

> From the user's point of view, I don't think they should need to care, but it should absolutely be possible for the implementation to steal blocked operations/timers/sockets/etc to a different worker if that would reduce latency overall (an internal decision).

On io_uring:

> this would be more of a performance measurement decision. However … we should ensure the interface supports this regardless of the implementation's choices.

So locality remains the steady-state design and its cost must stay close to zero (see [cost](#hot-path-cost-budget)). Rescue acts only after the fact, when a stall has actually happened, and only moves wakeups, not ownership. Ownership may move as well if measurements later justify it. The interface stays silent on placement so either choice remains available.

Earlier research constrains the mechanism. A shared timer queue driven by every worker was rejected for its CPU cost: about +35% process CPU with 4 workers and 1,024 timers, from deadline herds and lock contention ([worker-driven timer research](../benchmarks/echo/worker-timers-research.md)). That research recommended keeping "opportunistic assistance separate from idle-timeout ownership so assistance does not recreate the same wakeup herd". The worker-local queue then reduced process CPU by 64–71% compared with the timer-thread baseline ([worker-local timers](../benchmarks/echo/worker-local-timers.md)). That measurement does not isolate the cost of the uncontended mutex it removed. The `sharded` feature of `base-scheduler-timers` already provides worker-owned mutex queues with a `TimerAssist::try_poll` view that collects "while the owner is stalled outside queue operations", which is the shape proposed here.

## Scope

- **Requirement 1:** the Native executor (`crates/native`): its worker loop, timer queues, idle and park transitions, and shutdown. Its readiness selector drivers (`crates/selector/src/readiness_driver*`: epoll, kqueue and the `polling` compatibility backend). Its io_uring composition, for timers and runnable tasks; in-flight ring operations are governed by the io_uring decision.
- **Requirement 2:** the `base-scheduler` interface crate and rustdoc, `context/interface.md`, the contract sections of `context/design.md`, `context/io.md`, and `base-scheduler-conformance`. The `base-scheduler-selector` hooks are included as the integration point.
- **Out of scope:** preempting a poll. Interrupting a blocking syscall. A single long poll on the only worker, where no other worker is available to run the rescued tasks. Root futures still benefit if the rescuer is a dedicated thread. Also out of scope are the Tokio adapter, the standalone `async-io` fallback progress, and files, resolution and blocking jobs, which already run on independent pools.

## Validation

### Requirement 1

Regression tests (Native), each run with 2 and 4 workers, some workers held by a blocking poll of several seconds and at least one worker idle:

- `crates/native/tests/root_timers.rs` from PR #92. All three cases fail on `main`. Keep its bounds relative to the hold, and loosen `LATE` to a multiple of *T* rather than 1 s if *T* is configurable.
- A sleep and a reusable timer registered by a task on worker A, after which the task is stolen by B (force this with a hand-off between tasks), then A blocks. The timer must fire within *T*.
- A root `read_into` on a socket routed to a worker that then blocks: the scratch repro from #92, which took 1.945 s on `main`. Also a task-registered socket whose task is stolen before its registering worker blocks. Cover both epoll and kqueue, and the `polling` compatibility backend if rescue is enabled there.
- Runnable tasks queued behind a long poll, in both ready generations and in the remote inbox, run on other workers. This test should already pass on `main`; add it so the property is checked.
- io_uring: a test that documents the chosen behaviour for an in-flight operation on a stuck ring. If new operations are routed away from a stuck ring, test that too.
- Races and lifecycle: the owner returning from its long poll during a rescue; a stress test of repeated short stalls near the threshold; closure and `shutdown()` while a worker is stuck and rescue is active; driver retirement during rescue; and waker code that panics or reenters the scheduler during rescue. Shutdown must not complete while a rescuer still holds a queue or poller, and no record may be reclaimed while a rescuer's event batch can still refer to it.
- A false-positive counter (test or diagnostic build only). In the named benchmarks below, which contain no long polls, it should report zero rescues at the default *T*. Report any rescues it does record.

Inspection: no lock is held across a task poll or a waker call. Lock order is poll lock, then table lock, then timer queue, and never the reverse. Rescuers use only `try_lock` on owner state. Interrupt and notification records consumed by a rescuer are restored for the owner. The monitor holds only weak references to scheduler state.

Performance: follow [worker progress validation](worker-progress.md#validation). Keep the baseline revision and build settings, and match task topology, CPU placement and worker counts. Measure fixed-rate latency at equal offered rates separately from unrestricted throughput and process CPU, and report noise. The named workloads are listed under [cost](#hot-path-cost-budget). No numerical regression threshold has been agreed (see open questions).

### Requirement 2

Inspect the interface crate's rustdoc, `context/interface.md`, `context/design.md`, `context/io.md` and the conformance crate for any statement or assertion about which worker or thread services work, using the audit below as the baseline. Check that each remaining statement about original-worker or original-ring retention is explicitly described as a backend choice. Check that type bounds do not tie a future, handle or waker to a thread.

## Interface audit

This audit covers revision `4c36c8e` (`origin/main`). "Fine" means no change is needed. "Reword" means documentation states or implies affinity as a contract. "Change" means a trait or type must change.

### Portable interface and contract documents

| # | Item | Location | Verdict | Reason |
| --- | --- | --- | --- | --- |
| 1 | Operation futures `+ Send`; I/O handles `Send`; resources `Send + Sync` | `src/*` trait signatures, `context/design.md` "Future and thread bounds" | Fine | `Send` permits polling from any thread. Borrowed futures are not `'static`, which is fine: rescue moves wakeups, not futures. Handles need not be `Sync`, because a rescuer touches only backend-private registration state. |
| 2 | Spawned futures, blocking closures, outputs and handles `Send + 'static` | `src/spawn.rs`, `src/blocking.rs` | Fine | This is what makes stealing possible, and it is required for task migration. |
| 3 | `Timers::Timer: Send`; "without registering a deadline or selecting a worker" | `src/timers.rs` | Fine | Construction selects no worker; the trait says nothing about where a timer fires. |
| 4 | `Clock` sleep: "Dropping the future releases its timer registration" | `src/clock.rs` | Fine | No owner is named. Release must reach the queue that holds the key, which a lockable queue does directly. |
| 5 | Waker invocation thread | Interface: none; `Notify` "ordinary thread context"; signals "outside raw signal handlers and backend locks" | Fine, with a clarification added | Nothing promises which thread runs a waker. `context/interface.md` now says so explicitly, so callers don't infer it from current behaviour. |
| 6 | Operation futures polled from any executor | `context/design.md` "Explicit execution services"; Tokio and io_uring readmes | Fine | It is already permitted and exercised (root futures, other executors). |
| 7 | "Backend routing may use the current worker…, while an accepted operation retains its original worker or ring through retirement" | `context/design.md` "Explicit execution services" | **Reword (done)** | This is a contract section, and the clause reads as a guarantee. A completion, rescue or migration on another worker would violate it. It now describes the current Native selectors and states that the interface doesn't require it. |
| 8 | "Moving between workers leaves routing and registration migration internal" | `context/interface.md` conventions; `context/io.md`; `SocketStreamReader` rustdoc; readme | Fine, with an addition | It already leaves placement internal. A sentence now adds that backends may complete, wake, re-route or rescue in-flight work from any of their threads. |
| 9 | Cancellation of in-flight operations: best effort, retirement before release, success wins concurrent cancellation | `context/design.md` "Operation lifecycle"; `context/interface.md` | Fine | No thread or worker is named. A backend that re-routes must send cancellation to the operation's current owner, which is a backend obligation the interface already implies ("a completion backend must retire the original operation"). |
| 10 | Owned buffers retained until backend access retires, including after abandonment | same; `IoBuf` contracts | Fine | It doesn't matter which thread observes retirement. Rescue that enters a ring on another thread keeps the same storage. |
| 11 | Borrowed buffers accessed only synchronously inside a poll | `context/design.md` "Borrowed socket dispatch"; `context/interface.md` | Fine | Readiness rescue publishes readiness only; the payload syscall still happens in the task's own poll, wherever it runs. Rescue must not dispatch borrowed I/O itself. |
| 12 | `try_read_into` attempt semantics | `src/socket_stream_reader.rs`; [immediate reads](stream-immediate-read.md) | Fine | It bypasses readiness, so it is independent of any worker. See implementation item C for its Native test. |
| 13 | Registration lifetime: close and drop handles before exclusive close; final alias deregisters synchronously on any thread | `context/interface.md`; `context/design.md` | Fine | This already works from any thread. The batch-boundary reclamation rule must cover rescuer batches too (implementation item B). |
| 14 | Socket ownership and handles; scheduler-instance affinity | `src/socket_operation.rs`, `src/file_operation.rs`, [socket handle ownership](socket-handle-ownership.md) | Fine | Affinity is to a scheduler instance, not a worker. |
| 15 | Reusable timer `Registration`: "retained Native registrations keep their original worker, with waker updates and release routed to that worker" | `context/design.md` "Experimental reusable timers" | **Reword (done)** | This is a contract section describing Native's placement. It now says this is Native's current choice, that the key stays in its original queue, and that the interface doesn't say which worker fires it. |
| 16 | "Timers retain their existing worker-owned registration contract" | `context/design.md` "Directional socket monitor machinery" | **Reword (done)** | Calling worker ownership a "contract" contradicts requirement 2; it is now called placement. |
| 17 | "io_uring uses one ring per worker; active operations stay with their original ring through retirement" | `context/interface.md` "Backend composition" | **Reword (done)** | It is accurate for io_uring but reads as interface. It is now marked as a backend choice that the portable contract does not require. |
| 18 | `BufferPool`: "worker caches must make returned capacity available to acquisitions on other workers" | `src/buffer_pooling.rs` | Fine | This works against affinity. |
| 19 | `SocketRecvMessage` "may move between workers" | `src/socket_recv_message.rs` | Fine | |
| 20 | Conformance suites | `crates/conformance` | Fine | No assertion depends on a worker or thread identity (searched for worker/thread/affinity). The blocking fixture's "workers" are pool threads. |
| 21 | `SelectorWorker` "owned exclusively by an executor worker"; `turn(&mut self)`; `SelectorDriver::enter` "thread-bound routing" | `crates/selector/src/selector_worker.rs`, `selector_driver.rs`; `context/interface.md` selector table | **Change (described, not made)** | Exclusive ownership of the driver can stay. Another thread can't make progress for it without a new hook (see [interface changes](#interface-changes)). Reword the doc together with that change. |
| 22 | [Worker progress](worker-progress.md): "the owning worker finishes that snapshot before returning to application polling" | design document | **Reword on adoption** | A rescuer also collects snapshots. On adoption, change it to "the worker or rescuer that collected it". It is left unchanged here because this document is only a proposal. |

No `Send`/`'static` bound, waker contract, cancellation rule or retained-storage rule in the portable interface needs a type change.

### Backend implementation choices to keep backend-specific

These describe current behaviour and are not contract. When rescue lands, each must be updated or explicitly marked as backend-specific. None of them should move into the interface.

| | Statement | Location | Note |
| --- | --- | --- | --- |
| A | "An operation keeps its chosen ring through admission, completion, and cancellation, even if its task migrates." | `IoUringSelector::with_workers` rustdoc | This follows from `SINGLE_ISSUER`. It stays true under option 1 or 2. |
| B | Records "retire at a driver batch boundary"; "Only the sole driver calls" `Controller::stop` | `readiness_driver.rs`; `context/design.md` Native backend | The boundary must count rescuer batches too. `stop` must exclude rescuers. |
| C | `immediate_reads_observe_input_while_the_only_worker_is_busy`: "its driver publishes no readiness" | `crates/native/tests/worker_readiness.rs` | Its premise holds only without rescue. With a dedicated monitor, even one worker can be rescued after *T*. The test's immediate `poll_once` will probably still pass, but the comment and [immediate reads](stream-immediate-read.md) validation should say "within *T*". |
| D | "Each worker owns an unsynchronized timer queue"; "Keys never migrate"; "A full owning timer queue returns `WouldBlock`" | Native readme; `worker_timers.rs`; `NativeExecutorOptions::timer_capacity` | The queue becomes lockable. Keys still don't migrate, and capacity stays per queue (unlike #92's single root queue). |
| E | "Its first compatible worker owns that registration…; root and external callers select workers round-robin" | `context/design.md` Native backend | Placement stays as it is; under rescue, ownership no longer means that only the owner delivers events. |
| F | "The owning worker publishes the entire available CQ snapshot before polling any of those tasks"; "Timers and driver turns can both wait behind a long batch or a long individual poll" | `context/design.md` worker ownership; `context/execution.md` | Update both with the implementation: rescue bounds that wait. |
| G | Fixed-file and passive caches "a stalled original driver can delay their release" | `context/design.md` io_uring ring ownership | This stays true under options 1 and 2. |

### Interface changes

Only the selector hooks need one. A possible shape:

```rust,ignore
pub trait SelectorWorker {
    // existing methods …

    /// A shareable view through which another thread may make bounded, nonblocking
    /// progress on this driver's behalf while its owner is busy. `None` declines.
    fn assistant(&self) -> Option<Arc<dyn SelectorAssist>> { None }
}

pub trait SelectorAssist: Send + Sync {
    /// Publish readiness already queued for the owner without waiting, if the owner is
    /// not inside its own turn. Wakers run outside every lock; no application future
    /// is polled. `Busy` means the owner or another assistant holds the driver.
    fn try_assist(&self) -> io::Result<Assisted>; // Busy | Idle | Published(count)
    /// Hint that the owner is stalled, so new external routes may avoid it.
    fn set_stalled(&self, stalled: bool) {}
}
```

A provided method that returns `None` keeps existing selectors source-compatible. Dynamic dispatch is acceptable because it is used only when rescuing. The exact shape is an open question.

The portable `base-scheduler` crate needs no type or trait change.

## Proposed mechanism

This section is not a requirement. It describes the design to implement and measure.

### Detection

- **Heartbeat.** Each worker stores a per-worker turn counter on its own cache line, with a relaxed store once per `run_loop` iteration, next to `worker_timers::turn`. A turn boundary is exactly where timers and readiness are serviced, so a stalled counter means work there is waiting. A long batch of short tasks counts as stalled too, and rescuing in that case also reduces latency ([execution guide](../context/execution.md): "a large batch can delay I/O checks").
- **Idle.** The existing `sleeping` flag marks a worker in its idle wait. An idle worker services its own deadline and poller, so it is never treated as stuck.
- **Published deadline.** Each worker keeps its earliest local deadline in an atomic. Registration publishes a new deadline only if it is earlier than the published one (a load and a compare, and rarely a store), and the timer pass refreshes it. A rescuer reads it to skip lock attempts when nothing has expired. Unlike #92's shared deadline, no worker bounds its idle wait by another worker's deadline, so it cannot cause herds of wakeups.
- **Monitor.** The recommended monitor is a dedicated thread in the style of Go's sysmon. It samples every period *P* and treats a worker as stuck when its counter is unchanged across observations spanning the threshold *S*, while it is not idle. *T* = *S* + *P*. When it has seen every worker idle for several consecutive periods, the monitor goes into a deep sleep. The worker that leaves idle first wakes it, with one extra load on the idle exit path, which already performs `SeqCst` operations, and a wake only when the monitor is in deep sleep. That limits monitor wakes to about one per deep-sleep entry, which can't happen more often than the all-idle dwell allows. The alternative is a rescuer chosen among idle workers, which avoids a thread but needs a hand-off whenever that worker finds tasks; that is the cost that sank #92. Treat it as the comparison case to measure, not the default.
- **Threshold trade-offs.** A small *T* gives better worst-case latency but more false positives on long legitimate batches, more `try_lock` contention with owners that are about to return, and more lost locality as rescued tasks are stolen. A large *T* has fewer side effects but strands work for longer. Go uses roughly 10 ms for both its sysmon network poll and its preemption, from memory; this needs verifying. Candidates to measure are 1 ms and 10 ms.

### Timers

- Move each worker's `local::TimerQueue<Waker>` from TLS `RefCell` into `WorkerState` behind a mutex, or adopt `sharded::WorkerTimers`. The owner locks it, uncontended, for registration, update, release and its timer pass, and never holds it across a task poll or a waker call.
- A rescuer calls `try_lock`, pops expired entries into local storage, releases the lock, then calls the wakers. Each waker makes its task runnable in its preferred worker's inbox, and `wake_idle` falls back to any idle worker, which steals it. Keys stay in the original queue, and the slot keeps its capacity until `Release`.
- Remote commands (`Register`, `Update`, `Release`) currently go through a single-consumer `SegQueue` inbox that only the owner drains. A root `Register` sent to a stuck worker would never be acknowledged. Either have remote callers lock the target queue directly, which also removes the acknowledgement allocation, and interrupt the owner if they published an earlier deadline while it is idle, or have rescuers drain the inbox under the queue lock. The direct lock is simpler. Measure both.
- Closure is checked under the queue lock. A rescuer that observes closure pops all entries, as the owner does now. Shutdown waits for in-flight rescue waker calls, as it already does for timer callbacks.

### Readiness

- Add a per-controller poll lock that owns the driver's `Events` buffer. The owner holds it for its whole `turn`, including an idle `epoll_pwait`/`kevent`; holding it while idle is fine because an idle worker isn't stuck. It releases the lock before returning to tasks. A rescuer `try_lock`s it, waits with a zero timeout, dispatches, clears the buffer, then unlocks. Because waits never overlap on one poller, epoll and kqueue never split one wakeup between two waiters. Concurrent `epoll_wait` on one descriptor is legal in any case.
- **Edge semantics.** Registrations are persistent `EPOLLET` or `EV_CLEAR` with no oneshot flag, and each event publishes into the registration's shared `Direction` state (generation plus readiness bits) and wakes its listeners. Whoever collects an edge publishes it, so moving collection to a rescuer loses nothing and needs no re-arm. The rescuer must decode events exactly as the owner does, including the macOS byte count and EOF and the Linux EOF and urgent flags.
- **Interrupts.** Linux's interrupt eventfd is level-triggered, so the rescuer must skip `clear_notification` and leave the counter for the owner. macOS's `EVFILT_USER` is `EV_CLEAR`, so a rescuer that collects it must re-trigger it (`NOTE_TRIGGER`) or set a permit that the owner checks before waiting. Otherwise a wake racing the owner's idle transition would be lost. The timerfd is armed only during the owner's locked wait and is filtered already. The `polling` compatibility backend's notification semantics under a foreign wait need checking; until they are checked, decline assistance there.
- **Tables and reclamation.** `Mutex<Table>` still guards registration and retirement. Lock order is poll lock, then table lock. `Controller::release` (reclaim) runs only under the poll lock with an empty batch, which keeps the "retire at a batch boundary" invariant for every collector. `stop` acquires the poll lock and marks the controller retired, and rescuers check that mark under the lock.
- **Locality.** The registration stays on the stuck worker, so each later event for that socket needs rescuing again while the worker remains stuck. Moving a registration after repeated rescues is allowed by requirement 2 and is deferred.

### Runnable tasks

On `main`, both ready generations (`ready` and `current`, through `WorkerState::stealers`) and the remote `incoming` injector are stealable while their owner polls. There is no LIFO or next-task slot (`crates/native/src/worker.rs`, `search`). `schedule` calls `wake_idle`, which wakes any idle worker if the preferred one is busy, and every worker searches all victims before parking. Runnable tasks therefore don't need rescue. Only the task that is stuck in its poll is pinned, which is unavoidable. Stage 1 should add a regression test for this. Any future LIFO slot must stay stealable or be included in rescue.

### Root registrations and PR #92

Under rescue, round-robin placement of root and external registrations does no harm: if the chosen worker stalls, the rescuer fires its timers and drains its poller. #92's shared root queue, watcher role and hand-off are then unnecessary, and so are their per-batch cost with any root timer pending and the reduction of root capacity to a single queue. Rescue covers root sockets and stolen tasks' registrations as well as root timers, which #92 didn't.

Reuse from #92: `crates/native/tests/root_timers.rs`, which fails on `main` and should pass under rescue; its scratch spawn benchmark with a root timer pending, as the cost check this design must pass; the scratch root-read repro, turned into a test; and its closure ordering analysis (check closure under the queue lock, and drain without trusting the atomic deadline once closure has been observed).

### io_uring

Under `SINGLE_ISSUER | DEFER_TASKRUN` (`bind_worker`), only the owner thread may enter the ring, and deferred completion task work runs only when the owner enters with `GETEVENTS`. In-flight operations on a stuck ring therefore can't be rescued: no other thread can reap them or submit their cancellations. Borrowed waits use ring `PollAdd`, so their readiness is stranded too. Timers and runnable tasks are still rescued, because they are executor state, and fallback operations use the shared `async-io` selector. This is a measurement decision with three options, none of which the interface may preclude:

1. **Accept and document it.** In-flight ring operations wait for their owner; `spawn_blocking` is the remedy. New operations from stolen tasks already follow their new worker through TLS routing.
2. **Route new operations away from a stuck ring.** The monitor sets the stall hint, and `route_ref`'s external round robin and borrowed-wait ring selection skip rings marked stuck. This costs one load per external route. Rebinding an existing persistent monitor still needs the old ring to retire its poll, so it would stall as well. It should stay on the old ring, or be left for the owner to retire later; that needs designing.
3. **Drop the flags.** Use cooperative or default task running without a single issuer, with SQ, CQ and in-flight state behind a per-ring lock, so a rescuer can submit and reap with `GETEVENTS`. I am unsure whether completions are posted while the owner is CPU-bound or blocked in a syscall under cooperative task running. Task work is queued to the submitting task, and without an IPI it may wait for that task's next kernel transition. Under default task running it may interrupt the owner's syscall. A spike must establish this before the option is costed.

What to measure:

- The echo matrix for owned and borrowed I/O: 1/1/64, 1/32/64, 4/128/64 and 4/128/4096.
- Fixed-rate latency at equal offered rates.
- Process CPU, plus ring entries and context switches from `perf stat`, run separately.
- For each of: the current flags; option 3 without a lock; option 3 with the lock.
- The latency of an in-flight ring read behind a 2 s poll.

An earlier deferred-task-work screen was mixed, ranging from −1.44% to +2.48% ([single-worker research](../benchmarks/echo/single-worker-research.md)). The cost of dropping the flags may therefore be small, but that needs measuring again on the current executor.

### Hot-path cost budget

Steady-state additions, none of which should change the no-stall paths beyond noise:

- A relaxed heartbeat store per loop iteration.
- An uncontended lock and unlock in place of a `RefCell` borrow on each timer operation and timer pass.
- A conditional atomic store when a registration publishes an earlier deadline.
- A poll lock acquired and released around each driver turn.
- One load on idle exit to check whether the monitor is in deep sleep.
- Monitor wakes at 1/*P* while any worker is busy, and at most about one wake per deep-sleep entry otherwise.
- Nothing per spawn, and nothing that depends on whether a root timer is pending. That is what distinguishes this design from #92.

Named workloads, each with its baseline from `main` at the same revision and build settings:

- The timer benchmark: refresh, `sleep yield 64/4`, `reuse yield 64/4`, `expire 128/4`, cycle.
- The worker-local timer diagnostic: 1/1/10 µs, 4/1,024/1 ms, and 4/1,024/1 ms aligned, reporting lateness p50/p99 and process CPU.
- #92's spawn benchmark, with and without a pending root timer.
- The echo matrix above, on readiness and io_uring.
- Idle process CPU over 10 s with no load, and with 1 request per second.

Report rescue and false-positive counts alongside. Fixed-rate latency is reported separately from throughput and CPU, as [worker progress](worker-progress.md) requires.

### Risks

- **False positives:** see the threshold trade-offs.
- **Locality loss.** Rescued tasks are stolen, and they keep paying cross-worker costs while their registrations stay on a stuck worker.
- **Owner contention.** The owner can block briefly behind a rescuer's critical section. Keep it bounded: expired timers only, and one event batch.
- **Waker code.** Wakers run on the monitor thread. A foreign executor's slow or panicking waker can delay rescue of other workers. Contain panics, and consider capping each rescue pass.
- **Shutdown.** The monitor must hold only weak references and exit before drivers are destroyed. Retirement must wait for in-flight rescue passes. A rescue must never touch a stopped controller or a closed queue except to drain it.
- **Masking.** Rescue can hide blocking that should be `spawn_blocking`. Consider a diagnostic counter or log for repeated stalls of the same worker.

## Prior art

- **Go.** This is from memory of `runtime/proc.go` and `runtime/time.go`; verify the details before relying on them.
  - Since Go 1.14, timers live in per-P heaps. A P that runs out of work and tries to steal also runs the victim P's expired timers (`stealWork` calling `checkTimers` on the victim), so timers don't wait for their owning P. That is the same idea as the timer rescue here.
  - The network poller is process-wide rather than per-P. `sysmon`, a dedicated thread with no P, polls it without blocking if nobody has for about 10 ms.
  - `sysmon` also retakes Ps from threads blocked in syscalls, handing the P with its queues and timers to another thread, and asks goroutines running for more than about 10 ms to be preempted.
  - It backs off from about 20 µs to 10 ms when there is nothing to do, and sleeps deeply while all Ps are idle.
  - Go can retake a P because the P (scheduling state) is separate from the M (thread). Native's worker state is similarly separable for queues and timers, but not for an io_uring ring under single issuer.
- **Tokio** (multi-thread runtime). Again from memory; verify.
  - The I/O and time drivers are shared. A worker about to park tries to take the driver and parks inside it, while other parking workers wait on a condition variable. A busy or stuck worker doesn't hold the driver, so it doesn't strand I/O or timers, at the cost of a shared driver that every worker uses.
  - Local run queues are stealable, but I believe the LIFO slot is not, so a task in a stuck worker's LIFO slot waits for that worker.
  - `block_in_place` hands the worker's core to another thread. That is an explicit form of Go's hand-off, comparable to declaring blocking with `spawn_blocking`.
  - To my knowledge Tokio has no automatic stall detector.

## Staging

1. **This document and the interface wording.** Make the interface wording changes (requirement 2) and record the audit. No code changes.
2. **Timers.** Add the heartbeat, monitor and lockable timer queues with timer rescue. Bring over #92's tests. Document the io_uring exclusion (option 1). Measure against the cost budget.
3. **Readiness.** Add the selector assist hook, the poll lock, rescue for epoll and kqueue, and handling of interrupt records. Add the root-read and stolen-task socket tests. Amend [worker progress](worker-progress.md) for rescuer-collected snapshots.
4. **io_uring.** Measure options 2 and 3, including the spike on whether completions are posted without the owner, and choose among them.
5. **Possibly later.** Move registrations after repeated rescues, if measurements show that locality loss matters.

## Open questions

1. What should *T* default to, or *S* and *P* separately, and should it be configurable in `NativeExecutorOptions`? Can rescue be disabled?
2. Should the rescuer be a dedicated monitor thread (recommended) or an idle worker? With a dedicated thread, should it also rescue when no worker is idle? That still helps root futures, and makes tasks runnable for the next worker to become free.
3. Should remote timer commands lock the target queue directly, or keep the inbox with rescuers draining it?
4. What regression budget applies to the named workloads? #92's +15% wall time and +28% CPU on spawning was judged too high, but no threshold has been agreed.
5. Which io_uring option should be chosen, after measurement?
6. What shape should the selector assist hook take: a provided `dyn` method or an associated type, and the names?
7. Should repeated rescue trigger registration migration (stage 5)?
8. Should rescue run on the `polling` compatibility backend, or be declined there?
