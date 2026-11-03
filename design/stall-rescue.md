# Stall Rescue

**Status.** The parts of this document take effect at different times:

- **Requirement 2 is active now.** It is the interface's part, and the context wording changes that accompany this document already reflect it.
- **Requirement 1 is proposed.** Its timer parts (expired timers, remote timer commands, the off switch) take effect when stage 2 lands. Its readiness parts take effect when stage 3 lands. In-flight io_uring operations stay excluded until stage 4 records a decision.
- **Decisions.** The maintainer has decided every question this document raised. They are recorded under [Decisions](#decisions).

## Requirement

### 1. Stranded ready work progresses while a worker is idle (Native quality of implementation)

In the Native backend, ready work held by a stuck worker must progress within a bound *T* while any other task worker of the same scheduler is idle, without the application doing anything.

A **stuck** worker has not reached a turn boundary (its timer pass and driver turn in `run_loop`) for longer than the stall threshold. It is also not idle and not closing.

**Ready work** means:

- **Expired timers in the stuck worker's queue.** This includes sleeps and reusable timers registered by root futures or other executors and routed there. It also includes registrations made by tasks that other workers later stole, with whatever waker was most recently set for each.
- **Readiness events queued in the stuck worker's epoll or kqueue poller.** This covers sockets registered on that worker by its own tasks, by tasks later stolen, or by root and external callers routed there.
- **Runnable tasks** in the stuck worker's ready queues and remote inbox.

**Progress** means the waker is invoked, and the woken task becomes runnable and stealable so that an idle worker can poll it. For a root future, the `block_on` thread is woken instead. The poll that is stuck is never preempted. Runnable tasks already make progress through stealing, with no detection needed (see [runnable tasks](#runnable-tasks)).

**The bound T.** *T* is the worst-case delay from work becoming ready to its waker being invoked, assuming the rescuing thread gets scheduled. It is the sum of:

- the stall threshold *S*;
- the sampling period *P*;
- the bounded rescue work itself.

**Decided:**

- The target is *T* = 2 ms.
- The split between *S* and *P* is chosen by measurement. The starting point is *S* ≈ *P* ≈ 1 ms.
- *T* is a fixed internal default. There is no public tuning knob.
- Native provides an off switch for diagnostics, benchmarks and deterministic tests. One possible form is a `stall_rescue: bool` in `NativeExecutorOptions` that defaults to on, but the exact form belongs to the implementation.

When no other worker is idle, rescue still invokes wakers. A root future then makes progress, and a rescued task is queued earlier. However, `TaskService::wake_idle` returns early when `idle_workers == 0` (`crates/native/src/task_service.rs`), so the task runs only when some worker next searches for work. The bound applies only while a worker is idle.

**Declared blocking.** `Blocking::spawn_blocking` remains the way to declare blocking work up front. Rescue is a fallback for blocking that wasn't declared, not a replacement for `spawn_blocking`, and it does not make blocking inside a task free. A task that blocks still holds its worker's thread and delays everything else that worker would have run.

**Scope of the property.** This is a property of the Native backend's implementation, not part of the portable interface's contract. Its readiness parts cover every platform where Native has a worker readiness driver: Linux and Android (epoll), and macOS, FreeBSD, NetBSD, OpenBSD and DragonFly (kqueue). After stage 3, every such driver is base-rs code, so a rescuer can leave the owner's interrupts pending (see [decision 8](#decisions)). Two cases are excluded:

- **In-flight io_uring operations.** These are excluded until the [io_uring decision](#io_uring) is made. Stage 2 documents this as the interim, which is option 1.
- **Platforms without a worker readiness driver.** These are Windows, illumos, Solaris and Redox. Their socket waits use the standalone `async-io` fallback, which is not worker-owned and so is never stranded behind a worker.

Timers and runnable tasks are still rescued in both cases. The property does not apply to the Tokio adapter, which has its own driver model.

**Preserved contracts.** Rescue must preserve every existing ownership contract:

- cancellation;
- owned-buffer retention through original retirement;
- the rule that borrowed payloads are accessed only within a poll;
- registration reclamation at batch boundaries;
- graceful shutdown.

**Wakers and locks.** Rescue must never poll an application future from completion or timer processing ([worker progress](worker-progress.md)). It must call timer wakers outside every timer-queue lock. Readiness dispatch, whether by the owner or by a rescuer, calls wakers while it holds that poller's poll lock. It never holds a registration table lock or a notification mutex while doing so ([readiness](#readiness) explains why).

### 2. The interface permits completion, re-routing and rescue on any worker or thread

The portable interface consists of:

- the `base-scheduler` crate and its rustdoc;
- `context/interface.md`;
- the contract sections of `context/design.md`;
- the conformance suite.

It must not require, imply or test that a particular worker or thread services an operation, registration, timer or waker. The canonical permission, used in the context documents too, is:

> A backend may complete an operation, invoke a waker, fire a timer, drain readiness, cancel, re-route, or migrate in-flight work from any thread.

A backend must still keep each operation bound to its selected scheduler and keep the ownership contracts in requirement 1. Where current backends keep work on its original worker or ring, the documentation must present that as a backend implementation choice, not as a portable guarantee.

The selector hooks in `base-scheduler-selector` are public, but they are not the portable interface. They currently describe a driver as exclusively owned by one worker (`SelectorWorker`, `SelectorDriver::enter`). Requirement 1 needs an optional addition to them, so that another thread can make bounded nonblocking progress for a worker's driver on readiness. Selectors that cannot support it must be able to decline. Those are io_uring under its current flags, and readiness drivers retired at creation because the platform has no supported edge-triggered poller. The addition is described under [interface changes](#interface-changes) and is not made by this document.

## Rationale

### How it was found

The defect showed up while building the spawned server mode of `base-http-fixtures` ([base-rs/base-http#49](https://github.com/base-rs/base-http/pull/49)). That mode bounded its server join with a Native timer registered from the test thread. In the test where a blocking handler occupies the only worker, that timer was routed to the blocked worker and never fired, so the deadline meant to catch a stuck server was itself stuck behind it.

`base-http-fixtures` now runs that join deadline on its own thread. This is documented there as a temporary workaround to remove once rescue lands. Its test-phase and client-teardown deadlines still use the scheduler clock, so they are bounded only while that clock's timers can run.

### Where work is stranded

Each Native worker owns several pieces of state that are not stolen:

- **A timer queue.** See `base_scheduler_timers::local` and `crates/native/src/worker_timers.rs` ("Keys never migrate").
- **A readiness poller.** In `crates/selector/src/readiness_driver.rs`, `DriverGroup::route` assigns external callers round robin, and a socket keeps its route for its whole lifetime.
- **An io_uring ring**, when io_uring is in use. In `crates/iouring/src/io_uring_selector.rs`, see `route_ref`, and `bind_worker` with `SINGLE_ISSUER | DEFER_TASKRUN`.

Tasks are stolen, but timers and registrations stay where they are. A worker stuck in a long poll therefore strands its timers and readiness events while other workers sit idle. On `main`, behind a 2 s blocking task:

- a 100 ms root sleep took 1.94 s;
- a root socket read took 1.95 s.

### PR #92

[PR #92](https://github.com/base-rs/base-scheduler/pull/92), now closed as superseded by this design, fixed root timers only. Its branch, `root-timer-routing`, is kept at `861d9dd`. It does so with a shared root queue and a watcher role that is handed off before each batch.

Its own benchmark for spawning with a root timer pending reports:

- wall time 5,332 → 5,617 ns (+5.3%);
- CPU 5,646 → 6,568 ns (+16.3%);
- an earlier run reported about +11% wall and +25% CPU.

A separate local benchmark run while reviewing #92, which is not published, gave about +15% wall and +28% CPU. The cost applies whenever any root timer is pending, which is common: `block_on(main)` holding a timeout is enough. This document proposes a general design that supersedes #92.

### The maintainer's position

> Tasks represent units of work … work stealing/task migration is expensive, so we want to avoid it, but in the case where something blocks (which we can't know ahead of time), it's our only real option to reduce latency, if other workers are not already busy.

> Performance is king, but work stealing is part of that equation. If we know something is going to block, we can state that clearly, e.g. using something like `spawn_blocking`.

> From the user's point of view, I don't think they should need to care, but it should absolutely be possible for the implementation to steal blocked operations/timers/sockets/etc to a different worker if that would reduce latency overall (an internal decision).

On io_uring:

> this would be more of a performance measurement decision. However … we should ensure the interface supports this regardless of the implementation's choices.

So locality remains the steady-state design, and its cost must stay within the agreed [budget](#hot-path-cost-budget). Rescue acts only after the fact, when a stall has actually happened. It moves wakeups, not ownership. Ownership may move as well if measurements later justify it, and the interface says nothing about placement so either choice remains available.

### Constraints from earlier research

- **A shared timer queue was rejected for its CPU cost.** When every worker drove one queue, process CPU rose by about 35% with 4 workers and 1,024 timers, because of deadline herds and lock contention ([worker-driven timer research](../benchmarks/echo/worker-timers-research.md)). That research recommended keeping "opportunistic assistance separate from idle-timeout ownership so assistance does not recreate the same wakeup herd".
- **The worker-local queue was a large improvement.** It reduced process CPU by 64–71% compared with the timer-thread baseline ([worker-local timers](../benchmarks/echo/worker-local-timers.md)). That measurement does not isolate the cost of the uncontended mutex it removed.
- **The building block already exists.** The `sharded` feature of `base-scheduler-timers` provides worker-owned mutex queues. Its `TimerAssist::try_poll` view collects "while the owner is stalled outside queue operations", which is the shape proposed here.

## Scope

- **Requirement 1** covers the Native executor (`crates/native`): its worker loop, timer queues, idle and park transitions, and shutdown. It also covers the epoll readiness driver on Linux and Android, the kqueue readiness driver on macOS, FreeBSD, NetBSD, OpenBSD and DragonFly (`crates/selector/src/readiness_driver*`), and Native's io_uring composition for timers and runnable tasks.
- **Requirement 2** covers:
  - the `base-scheduler` interface crate and its rustdoc;
  - `context/interface.md`;
  - the contract sections of `context/design.md`;
  - `context/io.md`;
  - `base-scheduler-conformance`.

  The `base-scheduler-selector` hooks are included as the integration point.
- **Out of scope:**
  - preempting a poll;
  - interrupting a blocking syscall;
  - in-flight io_uring operations (pending the stage 4 decision);
  - running rescued tasks when no worker is idle (rescue still wakes root futures in that case);
  - the Tokio adapter;
  - progress of the standalone `async-io` fallback, which is the only readiness path on Windows, illumos, Solaris and Redox;
  - files, resolution and blocking jobs, which already run on independent pools.

## Validation

### Requirement 1

**Regression tests (Native).** Run each test with 2 and 4 workers, extending #92's tests where they use 2 only. "Within the `LATE` bound" means within the 250 ms bound described for #92's tests; *T* itself is checked by the benchmark lateness measurements, not by tests. In each, some workers are held by a blocking poll of several seconds and at least one worker is idle. Rescue is on unless the test says otherwise.

- **#92's root timer tests.** These are in [`crates/native/tests/root_timers.rs`](https://github.com/base-rs/base-scheduler/blob/861d9ddff11c8f6d7e2ca9e3949de553553e81fc/crates/native/tests/root_timers.rs) on #92's branch, and all three fail on `main`. Bring them over with their bounds relative to the hold. Set `LATE` to 250 ms rather than 1 s. That is large compared with *T*, so loaded CI runners don't make the test flaky, and it is still far below the multi-second hold. It distinguishes rescue from waiting for the hold without testing *T* itself. *T* is checked by the benchmark lateness measurements, not by tests.
- **Stolen timer.** A task on worker A registers a sleep and a reusable timer. Another worker, B, then steals the task (force this with a hand-off between tasks), and A blocks. The timer must fire within the `LATE` bound.
- **Waker update to a stuck owner.** A reusable timer is registered on A. A blocks, the task is polled on B, and B replaces the timer's waker. The timer must fire within the `LATE` bound and wake the *new* waker. On `main`, the update would sit in A's inbox, so a rescuer would fire the old waker and the wakeup would be lost.
- **Stuck readiness.** A root `read_into` on a socket routed to a worker that then blocks. This is #92's scratch reproduction, which took 1.945 s on `main`; its source is [recorded on this PR](https://github.com/base-rs/base-scheduler/pull/93#issuecomment-6596074954). Also a socket registered by a task that is stolen before its registering worker blocks. Run both on epoll, and on kqueue on macOS and FreeBSD.
- **Runnable tasks.** Tasks queued behind a long poll, in both ready generations and in the remote inbox, run on other workers. This should already pass on `main`; add it so the property is checked.
- **Stuck preferred worker.** Every worker but A is kept saturated with local work, and A blocks. A rescued waker schedules a task whose preferred worker is A. The task must run without waiting for A or for the other workers to drain their local queues. This covers the stuck-preferred refinement ([decision 9](#decisions), described under [runnable tasks](#runnable-tasks)) and is added with it.
- **No spin on interrupts.** A victim stays stuck with a pending interrupt (an eventfd count, or a re-triggered `EVFILT_USER`) and no registration events. An idle worker's pre-park check must find nothing published, and the idle worker must park rather than loop through `Retry` for the whole stall.
- **Pre-park retry.** Every worker but one is idle, and that one is stuck. A timer on the stuck worker is rescued by an idle worker's pre-park check, and its waker schedules a task whose preferred worker is the rescuing worker. The task must run promptly, and the rescuing worker must not park with `sleeping == false`. Repeat this to exercise the race.
- **No idle worker.** Every worker is held, a root `block_on` is waiting on a sleep, and the sleep's waker runs within the `LATE` bound.
- **io_uring.** A test that documents the interim behaviour for an in-flight operation on a stuck ring: it completes after the hold, not within the `LATE` bound. When stage 4 decides, add a test that new operations are routed away from a stuck ring if option 2 is adopted.
- **Deep-sleep handshake race.** Repeat this many times: all workers go idle, the monitor enters deep sleep, a worker wakes and at once blocks, and a timer on that worker must still be rescued within the `LATE` bound.
- **Races and lifecycle:**
  - the owner returns from its long poll during a rescue;
  - a stress test of repeated short stalls near the threshold;
  - closure and `shutdown()` while a worker is stuck and rescue is active;
  - driver retirement during rescue;
  - a waker that panics during rescue;
  - a waker that reenters the scheduler (spawns, registers or deregisters on the same controller) during rescue.

  Shutdown must not complete while a rescuer still holds a queue or poller. No record may be reclaimed while any batch, the owner's or a rescuer's, can still refer to it.
- **Rescue switched off.** With rescue switched off, behaviour matches `main`. `immediate_reads_observe_input_while_the_only_worker_is_busy` (`crates/native/tests/worker_readiness.rs`) uses the off switch. Its premise is that the busy worker's driver publishes no readiness, and a monitor would break that premise intermittently.
- **Rescue classification counter** (test or diagnostic builds only). Each rescue is counted as a stuck poll or a long batch (see [detection](#detection)). In the named benchmarks, which contain no long polls, it should report zero stuck-poll rescues at the default *T*; report any it does record as false positives. Long-batch rescues are a normal effect of load: report them, and judge their cost through the budgets rather than the counter. Workers that are closing, and workers whose `sleeping` flag was cleared by a remote wake before they left their wait, count as idle and are not counted.

**Inspection:**

- No timer-queue, table or notification lock is held across a waker call or a task poll. The poll lock is held across readiness wakers only, and is never held across a task poll.
- Lock order is poll lock, then table lock, then a direction's notification mutex. Timer-queue locks are leaves: they may be taken under any of these, but they never take one, and no path holds two timer-queue locks at once.
- A pre-park check that fires any waker makes the search return `Retry`.
- A rescuer uses only `try_lock` on owner state.
- Interrupt and notification records consumed by a rescuer are restored for the owner.
- The monitor holds only a weak reference to scheduler state, as described under [lifetimes](#lifetimes).

**Performance.** Follow [worker progress validation](worker-progress.md#validation):

- Keep the baseline revision and build settings, and match task topology, CPU placement and worker counts.
- Measure fixed-rate latency at equal offered rates separately from unrestricted throughput and process CPU.
- Report noise.

The workloads and the decided budgets are listed under [cost](#hot-path-cost-budget).

### Requirement 2

Inspect the interface crate's rustdoc, `context/interface.md`, `context/design.md`, `context/io.md` and the conformance crate. Look for any statement or assertion about which worker or thread services work, using the audit below as the baseline. Then check:

- that each remaining statement about original-worker or original-ring retention is explicitly described as a backend choice;
- that type bounds do not tie a future, handle or waker to a thread.

## Interface audit

This audit covers revision `4c36c8e` (`origin/main`). The verdicts mean:

- **Fine:** no change is needed.
- **Reword:** documentation states or implies affinity as a contract.
- **Change:** a trait or type must change.

### Portable interface and contract documents

| # | Item | Location | Verdict | Reason |
| --- | --- | --- | --- | --- |
| 1 | Operation futures `+ Send`; I/O handles `Send`; resources `Send + Sync` | `src/*` trait signatures; `context/design.md` "Future and thread bounds" | Fine | `Send` permits polling from any thread. Borrowed futures are not `'static`, which is fine: rescue moves wakeups, not futures. Handles need not be `Sync`, because a rescuer touches only backend-private registration state. |
| 2 | Spawned futures, blocking closures, outputs and handles `Send + 'static` | `src/spawn.rs`, `src/blocking.rs` | Fine | This is what makes stealing possible, and it is required for task migration. |
| 3 | `Timers::Timer: Send`; "without registering a deadline or selecting a worker" | `src/timers.rs` | Fine | Construction selects no worker, and the trait says nothing about where a timer fires. |
| 4 | `Clock` sleep: "Dropping the future releases its timer registration" | `src/clock.rs` | Fine | No owner is named. Release must reach the queue that holds the key, which a lockable queue does directly. |
| 5 | Waker invocation thread | Interface: none. `Notify`: "ordinary thread context". Signals: "outside raw signal handlers and backend locks". | Fine, with a clarification added | Nothing promises which thread runs a waker. `context/interface.md` now says so with the canonical permission. |
| 6 | Operation futures polled from any executor | `context/design.md` "Explicit execution services"; Tokio and io_uring readmes | Fine | This is already permitted and exercised, by root futures and other executors. |
| 7 | "Backend routing may use the current worker…, while an accepted operation retains its original worker or ring through retirement" | `context/design.md` "Explicit execution services" | **Reword (done)** | This is a contract section, and the clause reads as a guarantee. It now carries the canonical permission and describes retention as the current Native selectors' choice. |
| 8 | "Moving between workers leaves routing and registration migration internal" | `context/interface.md` conventions; `context/io.md`; `SocketStreamReader` rustdoc; readme | Fine, with an addition | This already leaves placement internal. The canonical permission is added after it. |
| 9 | Cancellation of in-flight operations: best effort, retirement before release, success wins over a concurrent cancellation | `context/design.md` "Operation lifecycle"; `context/interface.md` | Fine | No thread or worker is named. A backend that re-routes must send cancellation to the operation's current owner. The interface already implies this: "a completion backend must retire the original operation". |
| 10 | Owned buffers retained until backend access retires, including after abandonment | Same locations; `IoBuf` contracts | Fine | It doesn't matter which thread observes retirement. |
| 11 | Borrowed buffers accessed only synchronously inside a poll | `context/design.md` "Borrowed socket dispatch"; `context/interface.md` | Fine | Readiness rescue publishes readiness only. The payload syscall still happens in the task's own poll, wherever it runs. Rescue must not dispatch borrowed I/O itself. |
| 12 | `try_read_into` attempt semantics | `src/socket_stream_reader.rs`; [immediate reads](stream-immediate-read.md) | Fine | It bypasses readiness, so it doesn't depend on any worker. Its Native validation premise is item 23. |
| 13 | Registration lifetime: close and drop handles before exclusive close; the final alias deregisters synchronously on any thread | `context/interface.md`; `context/design.md` | Fine | This already works from any thread. The batch-boundary reclamation rule must cover rescuer batches too (item B). |
| 14 | Socket ownership and handles; scheduler-instance affinity | `src/socket_operation.rs`, `src/file_operation.rs`, [socket handle ownership](socket-handle-ownership.md) | Fine | Affinity is to a scheduler instance, not to a worker. |
| 15 | Reusable timer `Registration`: "retained Native registrations keep their original worker, with waker updates and release routed to that worker" | `context/design.md` "Experimental reusable timers" | **Reword (done)** | This is a contract section that describes Native's placement. It now says this is Native's current choice and that the interface doesn't say which worker fires a timer. |
| 16 | "Timers retain their existing worker-owned registration contract" | `context/design.md` "Directional socket monitor machinery" | **Reword (done)** | Calling worker ownership a "contract" contradicts requirement 2. It now says "placement". |
| 17 | "The executor manages … worker-local timers…; io_uring uses one ring per worker; active operations stay with their original ring through retirement" | `context/interface.md` "Backend composition" | **Reword (done)** | This is accurate for Native and io_uring, but it reads as interface. Both statements are now marked as backend choices. |
| 18 | `BufferPool`: "worker caches must make returned capacity available to acquisitions on other workers" | `src/buffer_pooling.rs` | Fine | This works against affinity. |
| 19 | `SocketRecvMessage` "may move between workers" | `src/socket_recv_message.rs` | Fine | |
| 20 | Conformance suites | `crates/conformance` | Fine | No assertion depends on worker or thread identity (searched for worker, thread and affinity). The blocking fixture's "workers" are pool threads. |
| 21 | `SelectorWorker` "owned exclusively by an executor worker"; `turn(&mut self)`; `SelectorDriver::enter` "thread-bound routing" | `crates/selector/src/selector_worker.rs`, `selector_driver.rs`; `context/interface.md` selector table | **Change (described, not made)** | Exclusive ownership of the driver can stay. Without a new hook, though, no other thread can make progress for the driver (see [interface changes](#interface-changes)). Reword this documentation together with that change. |
| 22 | [Worker progress](worker-progress.md): "the owning worker finishes that snapshot before returning to application polling" | Design document | **Reword when adopted** | A rescuer also collects snapshots. In stage 3, change this to "the worker or rescuer that collected it". |
| 23 | [Immediate reads](stream-immediate-read.md) validation: "keeps the only worker busy, so its driver cannot publish readiness" | Design document | **Reword when adopted** | With rescue on, the premise holds only for the first *T*. In stage 3, say that the test runs with rescue off. |

No `Send`/`'static` bound, waker contract, cancellation rule or retained-storage rule in the portable interface needs a type change.

### Backend implementation choices to keep backend-specific

These describe current behaviour and are not contract. When rescue lands, each must be updated or explicitly marked as backend-specific. None of them should move into the interface.

| | Statement | Location | Note |
| --- | --- | --- | --- |
| A | "An operation keeps its chosen ring through admission, completion, and cancellation, even if its task migrates." | `IoUringSelector::with_workers` rustdoc | This follows from `SINGLE_ISSUER`. It stays true under options 1 and 2. |
| B | Records "retire at a driver batch boundary"; "Only the sole driver calls" `Controller::stop` | `readiness_driver.rs`; `context/design.md` Native backend | The boundary must count rescuer batches too, and `stop` must exclude rescuers (see [readiness](#readiness)). |
| C | `immediate_reads_observe_input_while_the_only_worker_is_busy`: "its driver publishes no readiness" | `crates/native/tests/worker_readiness.rs` | The premise holds only without rescue, so the test uses the off switch. |
| D | "Each worker owns an unsynchronized timer queue"; "Keys never migrate"; "A full owning timer queue returns `WouldBlock`" | Native readme; `worker_timers.rs`; `NativeExecutorOptions::timer_capacity` | The queue becomes lockable. Keys still don't migrate, and capacity stays per queue (unlike #92's single root queue). |
| E | "Its first compatible worker owns that registration…; root and external callers select workers round-robin" | `context/design.md` Native backend | Placement stays the same. Under rescue, ownership no longer means that only the owner delivers events. |
| F | "The owning worker publishes the entire available CQ snapshot before polling any of those tasks"; "a large batch can delay I/O checks" | `context/design.md` worker ownership | Update both with the implementation, because rescue bounds the delay for readiness and timers. |
| G | Fixed-file and passive caches: "a stalled original driver can delay their release" | `context/design.md` io_uring ring ownership | This stays true under options 1 and 2. |
| H | "no dedicated timer thread"; "Each task worker owns a reserved local queue"; "The registration retains its original worker…; waker replacements… update that original queue locally or through its inbox" | `context/design.md` Native clock and timers | This is Native's description, now framed as Native's current choice. On adoption: the monitor is not a timer thread (it owns no queue); "or through its inbox" becomes "or under the queue lock". |
| I | `NativeTimer` rustdoc: "Pending registrations retain their original worker when the waiting task moves" | `crates/native/src/worker_timers/native_timer.rs` | It now says this is current Native placement and not an interface guarantee (doc comment only). |

### Interface changes

Only the selector hooks need one. The driver declares its assistant type statically on `SelectorDriver`, which is never used as a trait object, so `SelectorWorker` and Native's `&mut dyn SelectorWorker` run loop stay as they are (decision 6):

```rust,ignore
pub trait SelectorDriver: Send + 'static {
    // existing items …

    /// The view through which another thread may make bounded, nonblocking progress
    /// on this driver's behalf while its owner is busy.
    type Assist: SelectorAssist + Send + Sync + 'static;

    /// Called before `enter`. `None` when this driver instance cannot be rescued, such as
    /// a readiness driver retired at creation because the platform has no supported
    /// edge-triggered poller.
    fn assistant(&self) -> Option<Self::Assist>;
}

pub trait SelectorAssist {
    /// Publish readiness already queued for the owner without waiting, if the owner is
    /// not inside its own turn. No application future is polled. `Busy` means the owner
    /// or another assistant holds the driver; `Retired` means the driver has stopped.
    fn try_assist(&self) -> io::Result<Assisted>; // Busy | Retired | Idle | Published(count)
    /// Hint that the owner is stalled, so new external routes may avoid it.
    fn set_stalled(&self, stalled: bool) {}
}

/// For drivers that can never be rescued: io_uring for now, the non-Unix fallback, and test drivers.
pub enum NoAssist {}
```

`Published(count)` counts registration events published, which is the same as the readiness wakers invoked for them. It excludes interrupt, notification and timerfd records. The pre-park `Retry` is keyed on this count being non-zero, and so is the monitor's view of progress. If interrupt records counted, a stuck victim with a pending interrupt would report progress on every zero-timeout wait. The rescuer leaves the Linux eventfd set and re-triggers `EVFILT_USER`, so such an interrupt stays pending, and an idle worker would spin through `Retry` and `yield_now` for the whole stall.

Associated-type defaults are unstable, so each of the four implementers names its type. They are the readiness driver, its fallback, io_uring and the test driver, all in this workspace. Drivers that decline use the uninhabited `NoAssist`, so the compiler knows their rescue path is unreachable. The names are placeholders.

`SelectorAssist` stays dyn-compatible. Native's `run_worker<D>` calls `assistant()` while the driver type is still concrete and stores the result as `Option<Arc<dyn SelectorAssist>>` in the worker's state, beside the existing erased `driver_waker`. The only dynamic calls are on the rescue path, which is rare and dominated by its `epoll_wait` or `kevent`. If profiling ever shows a cost, Native can become generic internally (`run_loop` first) without changing the selector trait.

The portable `base-scheduler` crate needs no type or trait change.

## Proposed mechanism

This section is not a requirement. It describes the design to implement and measure.

### Overview

The diagram follows one readiness wakeup through a stall. Timer rescue takes the same path, with the victim's timer-queue lock in place of its poll lock. Rescue moves the wakeup, not the task: no thread ever blocks in another worker's poller, and the woken task runs wherever it is scheduled and stolen.

```text
Task R last ran on worker A and awaits a read on socket sock.
sock is registered in A's epoll or kqueue.

1. Wait      R polls read(sock), gets WouldBlock and stores its waker in
             sock's shared state. R is now only a waker; it is in no queue.

2. Stall     A polls another task X, which blocks for seconds. A stops
             reaching turn boundaries, so its heartbeat stops changing.
             Data arrives on sock. The kernel queues the event in A's poller,
             and nobody collects it.

3. Detect    The monitor samples every P and sees A's heartbeat unchanged
             across S while A is neither idle nor closing. It marks A
             stuck. (S and P are about 1 ms each, for T = 2 ms.)

4. Collect   A rescuer, either the monitor or worker C in its pre-park
             check, collects A's queued events:
                 try_lock(A's poll lock)        held: skip, it is in hand
                 epoll_wait / kevent on A's poller, zero timeout
                 publish sock readable and call R's waker
                 unlock
             If C fired any waker, its search returns Retry, not Empty.

5. Schedule  R's waker runs schedule(R). With the stuck-preferred
             refinement (see "Runnable tasks"), R goes to a worker that is
             not stuck. Without it, R goes to its preferred worker's
             incoming inbox, which here is A's. wake_idle then wakes a
             sleeping worker: A is busy, so it skips A.

6. Run       Whichever comes first:
             a. A returns from X, drains its incoming at the next batch
                boundary and runs R; or
             b. an idle worker, often C itself after Retry, searches its
                victims, steals R from A's incoming and runs it. R's
                preferred worker becomes that worker.

Latency is about T plus one wake and one steal, rather than the whole
blocking poll.
```

Step 6b needs a worker that is idle or runs out of local work. When every other worker is saturated, a task waiting in a stuck worker's inbox is searched last; [runnable tasks](#runnable-tasks) describes the refinement that keeps it out of that inbox.

### Detection

- **Heartbeat.** Each worker stores a per-worker turn counter on its own cache line, with a relaxed store once per `run_loop` iteration, next to `worker_timers::turn`. A turn boundary is exactly where timers and readiness are serviced, so a counter that stops changing means work there is waiting.

  A long batch of short tasks counts as stalled too ([decision 13](#decisions)). A batch is the frozen generation that `run_ready_batch` drains between two turn boundaries, and it has no length limit, so a batch of many short polls can outlast *S* without any poll misbehaving. Rescuing in that case also reduces latency, since `context/design.md` notes that "a large batch can delay I/O checks". Test and diagnostic builds also count task polls, so that each rescue can be classified as a **stuck poll** (neither counter moved) or a **long batch** (the poll counter moved but the turn counter did not).
- **Idle and closing.** The existing `sleeping` flag marks a worker in its idle wait, and an idle worker services its own deadline and poller. A remote wake clears `sleeping` before the owner leaves its wait (`WorkerState::wake`). The shutdown drain also runs with `sleeping == false`. A worker that is closing (`Shared::finished`) is treated as idle. Neither case is counted as a false positive, though a worker woken remotely can still be rescued if it truly fails to return within *S*.
- **Published deadline.** Each worker keeps its earliest local deadline in an atomic `u64` of clock nanoseconds. Every timer queue mutation now happens under the queue lock, including remote commands (decided, see [timers](#timers)). Whoever holds the lock can therefore refresh it with a plain store after changing the head, with no read-modify-write and no `watchers` mutex.

  `sharded::Publication::lower` takes such a mutex, so it is not reused here. For per-request timeouts the queue oscillates between empty and one timer, so this store happens on most registrations and releases (see [cost](#hot-path-cost-budget)). A rescuer reads the atomic to skip lock attempts when nothing has expired. Unlike #92's shared deadline, no worker bounds its idle wait by another worker's deadline, so it cannot cause herds of wakeups.
- **Monitor (decided).** A dedicated thread in the style of Go's sysmon:
  - It samples every period *P*. It treats a worker as stuck when the worker's counter is unchanged across observations spanning *S* and the worker is not idle or closing.
  - It rescues whether or not any worker is idle. With no idle worker, rescue wakes root futures and queues tasks earlier, as described in requirement 1. If that pass proves costly in measurement, gate it on `idle_workers > 0` or on a pending root waker; checking either costs one load. For long batches specifically, [decision 13](#decisions) records the gate to add if the saturated echo runs miss their budget.
  - Its CPU, and the CPU it runs on, are part of the measurement plan.
- **Deep-sleep handshake.** The monitor deep-sleeps after it has seen every worker idle for several consecutive periods. The handshake works in two parts:
  - The monitor stores `deep = true` (`SeqCst`). It then rechecks every worker's `sleeping` flag (`SeqCst`) and parks only if all of them are still set. If any flag is clear, it stores `deep = false` and keeps sampling.
  - A worker leaving idle already stores `sleeping = false` with `SeqCst` (`worker.rs`, after `park`). It then loads `deep` (`SeqCst`). If `deep` is set, the worker swaps it to `false` and unparks the monitor.

  Both sides use `SeqCst` on a store followed by a load, so at least one side sees the other: the monitor does not park while a worker is active and unseen. The worker's cost is one load on the idle exit path, which already performs `SeqCst` operations, plus a wake only when the monitor is in deep sleep.
- **Pre-park check (decided).** In the existing steal search before parking, a worker also checks each victim that is not sleeping. The search runs after the worker has stored `sleeping = true`. If the check fires any waker, the search returns `Retry` instead of `Empty`: the worker rescans every queue and advertises sleep again before it parks. This retry is required. A woken task can be scheduled back to this same worker. `WorkerState::wake` then clears `sleeping` but skips the unpark on the self-thread path, and the task may land in a queue the search has already passed. Without the retry, the worker would park with `sleeping == false` and later `wake_idle` calls would skip it. Running the check before sleep is advertised was considered and rejected. It would move the check off the path where the worker already scans its victims, and a later steal search would still have to repeat it.
  Both checks act only on a victim the monitor has marked stuck ([decision 14](#decisions)), so steady-state behaviour stays as it is on `main`:
  - **Timers:** if the victim is marked stuck and its published deadline has passed, it `try_lock`s the victim's queue and fires its expired timers. Unlike Go's `stealWork`, it does not fire a healthy victim's expired timers, which would move their tasks to the parking worker and make locality loss routine.
  - **Readiness:** if the victim is marked stuck, it calls `try_assist` on the victim's poller.

  Each check costs one load of the stuck mark, so this path adds no clock reads or syscalls unless a victim is actually stuck. It shortens *T* for whichever worker is about to sleep anyway.

  The check sits in the victim loop of `search`, which runs only while `has_runnables()` holds (`worker.rs`, the early return before the victim loop). That holds during a stall, because the stuck task stays counted in `outstanding` until `retire_runnable` runs after its poll. If that accounting changes, the check must move ahead of the early return.
- **Threshold trade-offs.** A small *T* gives better worst-case latency. It also costs more: more monitor wakes (about 1/*P*, so roughly 1,000 per second at *P* = 1 ms whenever any worker is busy), more false positives on long legitimate batches, more `try_lock` contention with owners about to return, and more lost locality as rescued tasks are stolen. A large *T* has fewer side effects but strands work for longer. The 2 ms target is much tighter than Go's sysmon, which waits about 10 ms before polling the network or preempting.

### Timers

- **Lockable queues.** Move each worker's `local::TimerQueue<Waker>` out of the TLS `RefCell` and into `WorkerState` behind a mutex, or adopt `sharded::WorkerTimers`. The owner locks it, uncontended, for registration, update, release and its timer pass. It never holds the lock across a task poll or a waker call.
- **Batch expiry.** The current expire loop borrows once per pop (`worker_timers::expire`). Under a mutex, the owner and rescuers instead lock once, pop with a batch pop into a reusable buffer, unlock, and then call the wakers. `local::TimerQueue` needs a batch pop; the heap and sharded queues already have `pop_expired_batch`.
- **Rescuer.** A rescuer uses `try_lock`, pops expired entries, releases the lock and calls the wakers. Each waker makes its task runnable in its preferred worker's inbox, and `wake_idle` falls back to any idle worker, which steals the task. Keys stay in their original queue, and each slot keeps its capacity until `Release`.
- **Remote commands (decided).** `Register`, `Update` and `Release` lock the target queue directly. They interrupt the owner only if they published an earlier deadline while it is idle. This replaces the single-consumer `SegQueue` inbox. That inbox would leave a root `Register` unacknowledged behind a stuck owner. It would also leave a waker `Update` undelivered, so a rescuer would fire the stale waker and lose the wakeup. The direct lock also removes the acknowledgement allocation for remote registration.

  **Ordering.** The owner stores `sleeping = true` (`SeqCst`) before it locks its queue to compute its wait timeout. A remote command inserts under the queue lock and checks `sleeping` after it unlocks, interrupting through `WorkerState::wake` if it lowered the head. The mutex orders the two, so either the owner's timeout computation sees the new entry or the command sees `sleeping`. This replaces the inbox recheck in `worker_timers::timeout`.
- **Closure.** Closure is checked under the queue lock. A rescuer that observes closure pops every entry, as the owner does now. Shutdown waits for in-flight rescue waker calls, as it already does for timer callbacks.

### Readiness

**Poll lock.** Add a per-controller poll lock that owns the driver's `Events` buffer and the right to wait on the poller.

- **Owner, idle wait.** It acquires the lock before the wait, holding it through an idle `epoll_pwait` or `kevent`. Holding the lock while idle is fine, because an idle worker isn't stuck.
- **Owner, zero-timeout turns.** For the turns between batches, it uses `try_lock` and skips the turn if a rescuer holds the lock. That rescuer is already publishing the same events. A skipped turn returns `Ok(false)`, meaning progress may be pending. It never reports the driver as closed and drained, even when closure has begun.
- **Release.** The owner releases the lock before it returns to tasks.
- **Rescuer.** A rescuer uses `try_lock`, waits with a zero timeout, dispatches, clears the buffer and unlocks.

Waits on one poller never overlap. Concurrent `epoll_wait` calls on one descriptor are legal in any case.

**Wakers run under the poll lock (resolution of the waker-locking question).** `DriverState::dispatch` decodes event keys as raw `Registration` pointers (on Linux and macOS). It publishes through `Direction::publish`, which calls `Notification::notify`, and that call invokes wakers inline. The notification's own mutex is released before the wakers run. Every key in the batch stays valid only because `Controller::release` runs after `events.clear()`, at the next batch boundary. If the poll lock were released before dispatch, a concurrent batch boundary could free records that are still being dispatched.

So dispatch, by either the owner or a rescuer, invokes readiness wakers under the poll lock. This is chosen over adding an in-flight batch count that `release` must see as zero, for these reasons:

- **The owner never blocks behind it on a hot path.** Its zero-timeout turns skip with `try_lock`, and only an idle wait blocks. By then the worker has no tasks, and the wakers being run publish tasks that idle workers will pick up.
- **Contention is bounded** to one batch of waker calls.
- **It needs no copying.** An in-flight count would require copying events out of the lock-owned buffer and adding an atomic to every owner turn. Its only benefit would be letting a blocked idle wait start a little earlier.
- **It is the owner's behaviour today.** Wakers already run inside `turn` with the batch alive.

**Reentrancy under the poll lock.** A waker or a dropped waker may reenter the scheduler. Every path it can take must avoid the poll lock:

- **Allowed:**
  - spawning or scheduling tasks;
  - interrupting a worker through its driver waker (`poller.notify`, with no poll lock);
  - registering or deregistering on the same controller (the table lock is taken after the poll lock, which matches the lock order).
- **Allowed:** registering, updating or releasing a timer. This takes a timer-queue lock under the poll lock.
- **Must not block on the poll lock:** `Controller::stop` (see "tables and reclamation" below).

**Lock order.** Poll lock, then table lock, then a direction's `Notification` mutex. The table lock is taken only on platforms without pointer tokens, where dispatch looks keys up through it. The `Notification` mutex is taken on every platform, and it is released before wakers run. Timer-queue locks are leaves: any of these locks may be held when one is taken, but no timer path ever takes the poll, table or `Notification` locks, and the timer path holds at most one queue lock at a time.

**Wakers must not block.** A rescuer holds the poll lock while readiness wakers run. A waker that blocks can therefore delay the stuck worker's idle wait without bound, along with that worker's own timer pass, because it has already advertised `sleeping`. This is documented rather than worked around. A blocking waker already blocks the owner's own turn today, so rescue moves where the problem bites rather than creating a new class of it. A timed `try_lock` with park-and-retry on the idle path was considered and rejected. It would need a second wake source for the parked owner (lock release, poller notification and timer deadline), and it would turn the idle transition into a polling loop, all to tolerate a waker that already breaks the `Waker` contract. Waker panics are contained per waker, as `worker_timers::expire` already does, so no panic poisons the poll lock.

**Edge semantics.** Registrations are persistent `EPOLLET` or `EV_CLEAR`, with no oneshot flag. Each event publishes into the registration's shared `Direction` state (generation plus readiness bits) and wakes its listeners. Whoever collects an edge publishes it, so moving collection to a rescuer loses nothing and needs no re-arm. The rescuer must decode events exactly as the owner does, including:

- the macOS byte count and EOF;
- the Linux EOF and urgent flags.

**Interrupts.**

- **Linux.** The interrupt eventfd is level-triggered, so the rescuer must skip `clear_notification` and leave the counter for the owner.
- **macOS.** `EVFILT_USER` is `EV_CLEAR`, so a rescuer that collects it must re-trigger it (`NOTE_TRIGGER`) or set a permit that the owner checks before waiting. Otherwise a wake racing the owner's idle transition would be lost.
- **Timerfd.** It is armed only during the owner's locked wait, and it is already filtered.

**Tables and reclamation.** `Mutex<Table>` still guards registration and retirement.

- `Controller::release` (reclamation) runs only under the poll lock and with an empty batch. This keeps the "retire at a batch boundary" invariant for every collector.
- `Controller::stop` sets a retired mark (atomic, or under the table lock) and then calls `try_lock` on the poll lock. If that succeeds, `stop` clears the batch and reclaims inline.
- If the lock is held, the current holder, whether owner or rescuer, checks the mark before it unlocks. If the mark is set, it clears its batch and reclaims before it returns or reports `Retired`.
- **Handoff race.** The holder can find the mark clear, then `stop` sets it and its `try_lock` fails, and then the holder unlocks; nobody would reclaim. To close this, the holder reloads the mark with `SeqCst` after it unlocks. `stop` stores the mark with `SeqCst` before its `try_lock`. If the reloaded mark is set, the holder calls `try_lock` again and reclaims if it succeeds. If that `try_lock` fails, the new holder is subject to the same rule. Either `stop`'s `try_lock` or one of these reloads observes the other's side, so some thread always reclaims.
- Retired records drop their `Waker`s and `Listener`s on whichever thread reclaims them: the stopping thread, the owner, the monitor, or a worker running the pre-park check. Those drops must not assume a particular thread.
- A rescuer that sees the mark when it acquires the lock reclaims if needed and returns `Retired`.

**Locality.** The registration stays on the stuck worker. Each later event for that socket therefore needs rescuing again while the worker remains stuck. Requirement 2 allows a registration to move after repeated rescues, but that is deferred ([decision 7](#decisions)).

### Runnable tasks

On `main`, these queues are all stealable while their owner polls:

- both ready generations, `ready` and `current`, through `WorkerState::stealers`;
- the remote `incoming` injector.

There is no LIFO or next-task slot (`crates/native/src/worker.rs`, `search`). `schedule` calls `wake_idle`, which wakes any idle worker if the preferred one is busy, and every worker searches all victims before parking. Runnable tasks therefore don't need rescue while any worker is idle, which is the condition requirement 1 sets. Only the task that is stuck in its poll is pinned, which is unavoidable. Stage 2 adds a regression test for this. Any future LIFO slot must stay stealable or be included in rescue.

**Stuck preferred worker ([decision 9](#decisions)).** A worker steals from other workers only once its own queues are empty (`search`). When every worker but the stuck one is saturated, a task woken into the stuck worker's `incoming` is searched last, behind every other worker's local work, even though the stuck worker cannot run it. This is outside requirement 1, which assumes an idle worker, but it makes rescued tasks wait longest exactly when the scheduler is loaded. The refinement: when `schedule` finds that the task's preferred worker is marked stuck, it treats the task as having no preference. It pushes the task to the current worker's local queue when called on a worker of the same scheduler, and otherwise to the inbox of a worker that is not stuck. This costs one relaxed load of the stuck mark per remote wake. It also removes the extra hop in step 5 of the [overview](#overview) in the common case. It belongs to stage 2, because the monitor's stuck mark arrives there, and it is measured against the same spawn and echo budgets.

### Root registrations and PR #92

With rescue in place, round-robin placement of root and external registrations does no harm. If the chosen worker stalls, a rescuer fires its timers and drains its poller. That makes #92's shared root queue, watcher role and hand-off unnecessary. It also removes their per-batch cost while any root timer is pending, and #92's reduction of root capacity to a single queue. Rescue also covers root sockets and stolen tasks' registrations, which #92 doesn't. This design is therefore proposed to supersede #92.

Parts of #92 to reuse:

- [`crates/native/tests/root_timers.rs`](https://github.com/base-rs/base-scheduler/blob/861d9ddff11c8f6d7e2ca9e3949de553553e81fc/crates/native/tests/root_timers.rs), which fails on `main` and should pass under rescue, and the root-queue cases in [`worker_timers/tests.rs`](https://github.com/base-rs/base-scheduler/blob/861d9ddff11c8f6d7e2ca9e3949de553553e81fc/crates/native/src/worker_timers/tests.rs) where they still apply;
- its scratch spawn benchmark with a root timer pending, as a cost check this design must pass;
- its scratch root-read reproduction, turned into a test;
- its closure ordering analysis: check closure under the queue lock, and once closure has been observed, drain without trusting the atomic deadline.

The two scratch programs were never committed. Their source, with build and run instructions, is [recorded on this PR](https://github.com/base-rs/base-scheduler/pull/93#issuecomment-6596074954).

### io_uring

**Why in-flight operations can't be rescued.** Under `SINGLE_ISSUER | DEFER_TASKRUN` (`bind_worker`), only the owner thread may enter the ring. Deferred completion task work runs only when the owner enters with `GETEVENTS`. So no other thread can reap a stuck ring's in-flight operations or submit their cancellations. Borrowed waits use ring `PollAdd`, so their readiness is stranded too.

Timers and runnable tasks are still rescued, because they are executor state, and fallback operations use the shared `async-io` selector.

**Decided:**

1. Option 1 is the interim, documented in stage 2.
2. Option 2 follows if it measures cheap.
3. Run the spike before costing option 3.

The interface must not preclude any of the options.

**The options:**

1. **Accept and document it (interim).** In-flight ring operations wait for their owner, and `spawn_blocking` is the remedy. New operations from stolen tasks already follow their new worker through TLS routing.
2. **Route new operations away from a stuck ring.** The monitor sets the stall hint. `route_ref`'s external round robin and the ring selection for borrowed waits then skip rings marked stuck, at a cost of one load per external route. Rebinding an existing persistent monitor still needs the old ring to retire its poll, so the rebind would stall as well. Such a monitor should stay on the old ring, or be left for the owner to retire later; this needs designing.
3. **Drop the flags.** Use task running without a single issuer, with SQ, CQ and in-flight state behind a per-ring lock, so that a rescuer can submit and reap with `GETEVENTS`.

   **Kernel semantics (to be verified by the spike).** The following is from memory and unverified:
   - Completion task work is queued to the submitting task.
   - Under `COOP_TASKRUN` (`TWA_SIGNAL_NO_IPI`), a CPU-bound owner runs it on its next return to user mode, but an owner asleep in a syscall is not woken to run it.
   - Default task running (`TWA_SIGNAL`) interrupts the owner's sleeping syscall.

   So option 3 probably needs default task running, or else the rescuer must submit cancellations or reissue operations itself.

   **A ready-made spike.** `bind_worker` already falls back to `COOP_TASKRUN | TASKRUN_FLAG` without `SINGLE_ISSUER` when deferred task running is unavailable. Movable drivers never bind at all, and keep default task running. Those two configurations are a ready-made spike.

**Earlier evidence:**

- A deferred-task-work screen was mixed, ranging from −1.44% to +2.48% ([single-worker research](../benchmarks/echo/single-worker-research.md)).
- [Tokio parity](../benchmarks/echo/tokio-parity.md) measured a 10.3% regression at 4 workers and 1 connection with deferred task running. That measurement combined it with a loop ordering that was rejected, and the page says the implementation then kept cooperative task work. [Single-worker research](../benchmarks/echo/single-worker-research.md) likewise records the "existing cooperative ring configuration unchanged".
- The current `bind_worker` tries deferred task running first, so that page and the code disagree. Measure the flag sets again on the current executor before choosing.

**What to measure:**

- the echo matrix for owned and borrowed I/O: 1/1/64, 1/32/64, 4/128/64 and 4/128/4096;
- fixed-rate latency at equal offered rates;
- process CPU, plus ring entries and context switches from `perf stat`, run separately;
- each of the above for: the current flags; cooperative task running without a single issuer; default task running; and option 3 with its lock;
- the latency of an in-flight ring read behind a 2 s poll under each.

### Hot-path cost budget

**Steady-state additions:**

- A relaxed heartbeat store on each loop iteration.
- An uncontended lock and unlock in place of a `RefCell` borrow for each timer operation, and one lock (not one per pop) for each timer pass.
- A plain store of the published deadline whenever the queue head changes. For per-request timeouts this means most registrations and releases, because the queue oscillates between empty and one timer.
- For remote timer commands, a lock on the target queue in place of an inbox push and acknowledgement allocation.
- An uncontended poll lock taken for each driver turn, with a `try_lock` on zero-timeout turns.
- One load on idle exit to check for monitor deep sleep, and one load per victim in the pre-park search.
- Monitor wakes at about 1/*P* (about 1,000 per second at *P* = 1 ms) while any worker is busy, and at most about one wake per deep-sleep entry otherwise.
- One relaxed load of the preferred worker's stuck mark for each remote wake in `schedule`, for the stuck-preferred-worker refinement.
- No change on the native epoll and kqueue paths from removing `polling`. The pipe and millisecond-timeout fallbacks run only when an eventfd, timerfd or `EVFILT_USER` is unavailable.
- Nothing per spawn, and nothing that depends on whether a root timer is pending. This is what distinguishes the design from #92.

**Budgets (decided).** Each workload is compared with its baseline from `main` at the same revision and build settings.

| Workload | Budget |
| --- | --- |
| Echo matrix (above), readiness and io_uring, owned and borrowed | Within noise |
| #92's spawn benchmark ([source](https://github.com/base-rs/base-scheduler/pull/93#issuecomment-6596074954)), `spawn` and `spawn-pending` | Within noise |
| Idle process CPU over 10 s with no load, and at 1 request per second | Within noise |
| Timer microbenchmarks: refresh, `sleep yield 64/4`, `reuse yield 64/4`, `expire 128/4`, cycle; worker-local timer diagnostic 1/1/10 µs, 4/1,024/1 ms and 4/1,024/1 ms aligned, with lateness p50 and p99 and process CPU | ≤ ~3% |

Any result over budget goes back to the maintainer with its numbers.

**Additional measurements (no budget yet; report them):**

- Monitor wake rate and CPU at each candidate *P* (for example 0.5, 1 and 2 ms), and which CPU it runs on. Report it alongside the workers' CPU, not hidden in process totals.
- Fixed-rate p99 latency with every core occupied, where the monitor competes with workers.
- A net-benefit workload with undeclared short blocking, for example tasks that occasionally poll CPU-bound for 5 ms among I/O and timer traffic. Compare p99 and throughput with rescue on and off.
- Rescue counts for every workload, classified as stuck polls or long batches, with false positives identified, and rescue counts per registration. [Decision 7](#decisions) revisits migration from the per-registration counts.

Fixed-rate latency is reported separately from throughput and CPU, as [worker progress](worker-progress.md) requires.

### Lifetimes

- Each worker's state holds an erased `Option<Arc<dyn SelectorAssist>>` in a `OnceLock`, set by `run_worker` from `SelectorDriver::assistant` before it enters the driver.
- The monitor holds only a `Weak<Shared>`. It upgrades that reference at the start of each pass and drops it at the end. It exits when the upgrade fails, or when it observes that scheduler closure has finished.
- Workers that use assistants in the pre-park search are already scoped by `Shared`.
- Safety does not depend on the monitor exiting before drivers are destroyed. An assistant stays valid after its driver stops: it observes the retired mark under the poll lock and returns `Retired`.
- **Stopping the monitor.** Closure signals the monitor to stop and unparks it. The monitor never joins itself. While it holds an upgraded reference it can hold the last strong `Arc<Shared>`, so `Shared` may drop on the monitor thread. For that reason the join handle is not held in `Shared`, and `Shared::drop` never joins.
- **Joining.** Native has no synchronous shutdown path today: `NativeScheduler::drop` only calls `close()` (`crates/native/src/native_scheduler.rs`), `shutdown()` is asynchronous, and worker threads are already detached (`crates/native/src/thread.rs`). The monitor is therefore detached in the same way. It exits after at most one period, once its upgrade fails or it observes the stop signal. If a synchronous shutdown path is added later, it may join the monitor from a thread that is not the monitor, outside `Shared::drop`. An asynchronous shutdown must not block an executor thread on a join. A join would be for tidiness, not memory safety.

### Risks

- **False positives.** See the threshold trade-offs. Closing workers, and workers woken remotely but not yet out of their wait, are excluded from the count.
- **Locality loss.** Rescued tasks are stolen. They keep paying cross-worker costs while their registrations stay on a stuck worker.
- **Owner contention.** An owner entering an idle wait can block behind one rescuer batch (readiness), or behind a rescuer's expired-timer pop (timers).
- **Waker code on the monitor thread.** Wakers run on the monitor thread. A foreign executor's slow or panicking waker can delay rescue of other workers, and can delay an idle owner of that poller. Contain panics, and consider capping each rescue pass.
- **The monitor's own CPU.** At about 1,000 wakes per second it is not free, and on fully loaded machines it competes with workers. It is measured explicitly.
- **Shutdown.** See [lifetimes](#lifetimes). A rescue must never touch a stopped controller or a closed queue except to drain it.
- **Untested platforms.** Once stage 3 adds the planned CI jobs, readiness tests run on Linux, macOS and FreeBSD only; today CI runs Linux, macOS and Windows. NetBSD will be build-checked, and OpenBSD and DragonFly are tier 3 targets with at most a build check, so a kqueue difference there, such as a missing `EVFILT_USER`, may surface only from users.
- **Masking.** Rescue can hide blocking that should be `spawn_blocking`. Consider a diagnostic counter or log for repeated stalls of the same worker.

## Prior art

### Go

- **Per-P timers.** Since Go 1.14, timers live in per-P heaps. A P that runs out of work and tries to steal also runs the victim P's expired timers (`stealWork` calls `checkTimers` on the victim), so timers don't wait for their owning P. This is the idea behind the pre-park check here.
- **The network poller and sysmon.** The network poller is process-wide rather than per-P. `sysmon` is a dedicated thread with no P. If nobody has polled the network for about 10 ms, it polls without blocking.
- **Retaking and preemption.** `sysmon` also retakes Ps from threads blocked in syscalls, handing the P with its queues and timers to another thread. It asks goroutines that have run for more than about 10 ms to be preempted.
- **Backoff.** When there is nothing to do, `sysmon` backs off from 20 µs to 10 ms, and it sleeps deeply while all Ps are idle.
- **Relevance to Native.** Go can retake a P because the P (scheduling state) is separate from the M (thread). Native's worker state is similarly separable for queues and timers, but not for an io_uring ring under single issuer.

### Tokio (multi-thread runtime)

- **Shared drivers.** The I/O and time drivers are shared. A worker about to park tries to take the driver and parks inside it, while other parking workers wait on a condition variable. A busy or stuck worker doesn't hold the driver, so it doesn't strand I/O or timers. The cost is a shared driver that every worker uses.
- **Run queues.** Local run queues are stealable, but the LIFO slot is not, so a task in a stuck worker's LIFO slot waits for that worker.
- **`block_in_place`.** It hands the worker's core to another thread. That is an explicit form of Go's hand-off, comparable to declaring blocking with `spawn_blocking`.
- **No stall detection.** Tokio has no automatic stall detector.

## Staging

1. **This document and the interface wording.** Make the requirement 2 wording changes and record the audit and decisions. No code changes.
2. **Timers.** This stage includes:
   - the heartbeat and the monitor, with its deep-sleep handshake;
   - the off switch;
   - lockable timer queues with batch expiry;
   - remote commands that lock the target queue directly;
   - the published deadline;
   - timer rescue, both from the monitor and from the pre-park check;
   - the stuck-preferred-worker refinement in `schedule`.

   It lands after this document merges, as three pull requests ([decision 12](#decisions)):
   1. lockable timer queues with batch expiry, remote commands that lock the target queue directly, and the published deadline;
   2. the heartbeat, the monitor with its deep-sleep handshake, the pre-park check and the off switch;
   3. the stuck-preferred-worker refinement.

   Each updates this document's status as it lands. The measurement harnesses that the budgets need are built inside stage 2, in the first pull request that measures against them, with the baseline taken on `main` using the same harness ([decision 10](#decisions)).

   Bring over #92's tests and add every [requirement 1 test](#requirement-1) that concerns timers or scheduling: stolen timer, waker update, runnable tasks, stuck preferred worker, pre-park retry, no idle worker, the io_uring interim, the deep-sleep handshake race, the races and lifecycle cases that apply to timer queues, rescue switched off, and the rescue classification counter. The readiness tests (stuck readiness, no spin on interrupts, driver retirement) belong to stage 3. Document the io_uring exclusion as the interim option 1. Update the Native readme's timer and platform descriptions (audit item D; its line saying other edge platforms use `polling` is already stale for Linux and Android), and `context/design.md` (audit item H). Measure against the budgets.
3. **Readiness.** This stage includes:
   - the selector assist hook;
   - the poll lock with `try_lock` on zero-timeout turns;
   - the retired mark;
   - epoll and kqueue rescue;
   - interrupt-record handling.

   It starts by removing the readiness driver's direct `polling` dependency, as its own pull request, so that rescue only ever runs against base-rs drivers ([decision 8](#decisions)):
   - On Linux and Android, `NativePoller` replaces the `Compatible` backend. If it cannot create an eventfd, it interrupts through a nonblocking pipe. If it cannot create a timerfd, it waits with `epoll_pwait`'s millisecond timeout, rounded up so that it never wakes early. Only timeout precision degrades, and only in that fallback.
   - `kqueue.rs` extends from macOS to FreeBSD, NetBSD, OpenBSD and DragonFly. Each platform's flags and filters are checked against its `kqueue(2)`. Where `EVFILT_USER` is unavailable, which includes NetBSD before 10.0 and possibly OpenBSD, the interrupt falls back to a pipe in the same way.
   - base-rs types replace `polling::Event`, `PollMode` and `Events`, and the test poller is rebuilt on the native drivers.
   - Redox loses its worker readiness driver and uses the standalone fallback, as illumos does today.

   Its validation:
   - fault-injection tests (`epoll/syscalls.rs`) for the pipe and millisecond-timeout fallbacks, with #58's fallback coverage re-run;
   - CI builds for the FreeBSD and NetBSD targets, and readiness tests run on FreeBSD in a virtual machine;
   - for OpenBSD and DragonFly, which are tier 3 targets, at least a build check in CI;
   - `cargo tree -p base-scheduler-selector -i polling` shows `polling` only through `async-io`.

   Add the root-read and stolen-task socket tests, and switch `immediate_reads_observe_input_while_the_only_worker_is_busy` to the off switch. Amend [worker progress](worker-progress.md) and [immediate reads](stream-immediate-read.md) (audit items 22 and 23).
4. **io_uring.** Measure option 2. Run the task-running spike, then cost option 3 if the spike makes it viable. Record the decision, replacing the interim.
5. **Possibly later.** Move registrations after repeated rescues, if the per-registration rescue counts show that locality loss matters ([decision 7](#decisions)).

Once stages 2 and 3 have landed, remove the own-thread join deadline in `base-http-fixtures` (see [how it was found](#how-it-was-found)).

## Decisions

These are the maintainer's decisions, recorded here and removed from the open questions:

- **1. *T*:** the target is 2 ms. The split between *S* and *P* is chosen by measurement, starting from about 1 ms each. It is a fixed internal default with no public knob. There is an off switch for diagnostics, benchmarks and deterministic tests.
- **2. Rescuer:** a dedicated monitor thread, plus a check of victims' expired timers and stuck pollers during the pre-park steal search. The monitor also rescues when no worker is idle; this mainly helps root futures, because of `wake_idle`.
- **3. Remote timer commands:** they lock the target queue directly.
- **4. Regression budgets:** within noise for echo, spawn and idle CPU, and ≤ ~3% for the timer microbenchmarks, measured as decision 11 describes. Anything over budget goes back to the maintainer with numbers.
- **5. io_uring:** option 1 is the interim during the timer stage, then option 2 if it measures cheap. Run the spike before costing option 3.
- **6. Selector hook:** an associated `SelectorDriver::Assist` type and `fn assistant(&self) -> Option<Self::Assist>`, as shown in [interface changes](#interface-changes), erased by Native at its boundary. The static declaration was preferred to a `dyn` hook on the trait for compile-time checking: each driver must state whether it can be rescued, and the `Send + Sync + 'static` bounds are checked against the concrete type. It sits on `SelectorDriver` rather than `SelectorWorker` because Native runs its loop over `&mut dyn SelectorWorker` and its `Shared` is not generic (it is also held by the public `NativeTimer`). `run_worker<D>` erases the assistant to `Arc<dyn SelectorAssist>` while the type is concrete. Dynamic calls happen only on the rescue path; if profiling shows a cost, Native can become generic internally without changing the trait. The `Option` remains because a readiness driver instance can be retired at creation when the platform has no supported edge-triggered poller.
- **7. Registration migration:** deferred. A registration stays on its worker. Revisit only if the per-registration rescue counts in the measurement plan show that the per-event delay of up to *P* matters during long stalls.
  - **Why deferring costs little.** Migration moves where a socket's events are collected, not where its task runs. Rescued tasks already run on other workers, and with the stuck-preferred-worker refinement they skip the stuck worker's inbox. The most migration could save is one rescue delay per event, during stalls that are themselves bugs.
  - **What it would cost.** Moving a live edge-triggered registration between pollers without losing or double-delivering an event already collected, plus two `epoll_ctl` or `kevent` changes per move.
  - **If it is ever adopted:** move a registration on its next use while its owner is marked stuck. That moves only sockets that are both stranded and active, and costs nothing when no worker is stuck. Moving on every steal is rejected, because it pays the move on ordinary steals unrelated to stalls. After *N* rescues is the remaining alternative. Requirement 2 already permits migration, so no interface change would be needed, and in-flight io_uring operations can't move either way.
- **8. The `polling` compatibility backend:** removed, not declined. The readiness driver drops its direct `polling` dependency entirely, as described in [stage 3](#staging). `async-io`, and `polling` through it, remain for the standalone socket fallback, process waits and the Tokio adapter on Windows. None of those is worker-owned, so none can be stranded behind a worker.
  - **Why it could not stay.** A rescuer cannot safely wait on a `polling::Poller`. In polling 3.11.0:
    - `Poller::wait_impl` swaps `notified` to false after every wait (`src/lib.rs`, around line 761).
    - The epoll backend clears and re-arms its notifier on every wait, including zero-timeout waits (`src/epoll.rs`, around lines 231–237).
    - `polling` filters its notifier event out of the results, so a caller can't tell that one was consumed.
    - A wait that is concurrent with another returns 0 immediately (`src/lib.rs`, around lines 770–774).

    So a rescuer's zero-timeout wait would consume an interrupt meant for the owner, which would then sleep through it. The native drivers avoid this by leaving the eventfd set or re-triggering `EVFILT_USER`.
  - **Alternatives considered.** Re-notifying unconditionally after every rescue pass would probably work, but it adds a syscall per pass and depends on `polling` internals. An upstream change, or a separate notification channel beside `polling`, would keep two drivers to maintain for the rarest configurations. Declining rescue there was the previous recommendation; the maintainer chose removal so that requirement 1 holds on every platform with a worker readiness driver.
  - **Why Linux needs no `polling`.** #58 selected it only when the native driver could not create its eventfd or timerfd. `NativePoller` now covers both failures itself.
- **9. Stuck preferred worker:** approved for stage 2, as described under [runnable tasks](#runnable-tasks), with its regression test, and with its cost of one relaxed load per remote wake measured against the budgets.
- **10. Measurement harnesses:** built inside stage 2, because several budgets cannot be checked with the tools in the repository today:
  - a Native readiness backend for the echo benchmark, starting from `benchmarks/echo/results/turn-interval/echo-native-backend.patch`;
  - an offered-rate option on the echo client, for fixed-rate latency and the 1 request per second idle-CPU case;
  - an idle-CPU script for the 10 s no-load measurement;
  - the worker-local timer diagnostic as a Cargo target in the private benchmark package.

  Each is added in the first stage 2 pull request that measures against it, and its baseline is measured on `main` with the same harness.
- **11. Noise and host:** budgets are checked on Linux on Aiko, using the CPU pinning and exclusive benchmark lock procedure of the existing echo reports (for example [`benchmarks/echo/double-buffered-ready-performance.md`](../benchmarks/echo/double-buffered-ready-performance.md)). Each case takes 9 paired, randomly ordered runs of `main` and the branch:
  - "within noise" means the branch's median lies inside `main`'s min–max range for that case;
  - "≤ ~3%" is the difference of the paired medians.

  macOS results are supplementary and do not decide a budget.
- **12. Stage 2 organisation:** stage 2 starts once this document merges and lands as the three pull requests listed under [staging](#staging).
- **13. Long batches:** a batch of short polls that outlasts *S* counts as a stall and is rescued, because rescue then bounds timer and readiness latency that the batch would otherwise delay. The release hot path stays at one heartbeat store per loop iteration. Test and diagnostic builds also count task polls, so each rescue is classified as a stuck poll or a long batch, and the validation expectation of zero false positives applies to stuck polls only.
  - **Why rescue helps less under saturation.** When every worker is busy, `wake_idle` returns early, so rescued tasks are only queued sooner, while the monitor competes for CPU and the poll lock. The gain is mainly for root futures and timer wakeups.
  - **Conditional gate.** If the saturated echo runs (4/128/4096) miss their budget, rescue long batches only when `idle_workers > 0`, and always rescue stuck polls. Telling the two apart in release builds then needs the per-poll counter there too: one relaxed store per task poll, on the heartbeat's cache line.
- **14. Pre-park timer rescue:** gated on the monitor's stuck mark, as readiness is. A pre-park check that fired a healthy victim's expired timers, as Go's `stealWork` does, would move those tasks to the parking worker in steady state, make the owner's timer pass wait behind the rescuer's pop, and add clock reads to the idle path. Rescue stays strictly after the fact, and the monitor already guarantees *T*. Gating on how overdue a timer is (`now − deadline > S`) was considered; it would save at most about one sampling period *P*, at the cost of those clock reads.

## Open questions

None. Questions 1–8 were decided, decisions 9–12 settle the questions raised when stage 2 was planned, and decisions 13 and 14 settle the questions raised in review. All are recorded under [Decisions](#decisions); questions 1–8 keep their original numbers so that earlier references stay valid.
