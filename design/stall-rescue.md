# Stall Rescue

**Status.** The parts of this document take effect at different times:

- **Requirement 2 is active now.** It is the interface's part, and the context wording changes that accompany this document already reflect it.
- **Requirement 1's timer parts are implemented** (stage 2, in the same pull request as this document): expired timers, remote timer commands, the off switch and the stuck-preferred refinement. The mechanism below describes the implementation; [implementation notes](#stage-2-implementation-notes) record what measurement changed on the way, and [decision 15](#decisions) replaced the periodic sampling originally specified. Two independent pieces were split out: the runnable counter's cache isolation (#94) and the echo benchmark harness (#95). Its readiness parts take effect when stage 3 lands. In-flight io_uring operations stay excluded until stage 4 records a decision.
- **Decisions.** The maintainer has decided the questions this document raised, recorded under [Decisions](#decisions), except stage 3's detection, listed under [open questions](#open-questions).

## Requirement

### 1. Stranded ready work progresses while a worker is idle (Native quality of implementation)

In the Native backend, ready work held by a stuck worker must progress within a bound *T* while any other task worker of the same scheduler is idle, without the application doing anything.

A **stuck** worker has not reached a turn boundary (its timer pass and driver turn in `run_loop`) for longer than the stall threshold. It is also not idle and not closing.

**Ready work** means:

- **Expired timers in the stuck worker's queue.** This includes sleeps and reusable timers registered by root futures or other executors and routed there. It also includes registrations made by tasks that other workers later stole, with whatever waker was most recently set for each.
- **Readiness events queued in the stuck worker's epoll or kqueue poller.** This covers sockets registered on that worker by its own tasks, by tasks later stolen, or by root and external callers routed there.
- **Runnable tasks** in the stuck worker's ready queues and remote inbox.

**Progress** means the waker is invoked, and the woken task becomes runnable and stealable so that an idle worker can poll it. For a root future, the `block_on` thread is woken instead. The poll that is stuck is never preempted. Runnable tasks already make progress through stealing, with no detection needed (see [runnable tasks](#runnable-tasks)).

**The bound T.** *T* is the worst-case delay from work becoming ready to its waker being invoked, assuming the rescuing thread gets scheduled. For timers ([decision 15](#decisions)) it is the sum of:

- the stall threshold *S*, how long a timer may stay due and uncollected on a worker that is not sleeping;
- up to *P*, the monitor's shortest interval between passes, when its plan is within *P* of the deadline or when it is already sampling a stuck worker;
- the bounded rescue work itself.

Stage 3 must choose how readiness stalls are detected (see [open questions](#open-questions)).

**Decided:**

- The target is *T* = 2 ms.
- The split between *S* and *P* is chosen by measurement. Both are 1 ms; measured worst-case lateness behind a 200 ms stall is about 1.3 ms.
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

**Regression tests (Native).** Run each test with 2 and 4 workers, extending #92's tests where they use 2 only. In most, some workers are held by a blocking poll and at least one worker is idle. The one-worker tests hold every worker, so the monitor is the only rescuer: closure with a stuck worker, the panicking and reentrant waker, and **No idle worker** below. Rescue is on unless the test says otherwise. *T* itself is checked by the benchmark lateness measurements, not by tests.

GitHub Actions runners are oversubscribed, so tests run there must not assert wall-clock upper bounds (sleep for *X*, expect completion within *Y*). A test shows rescue by causality instead: the blocking poll holds its worker until the test releases it, after the stranded work has progressed, so that progress can only have come from rescue; the rescue counters confirm it, and rescue that never happens fails at the test watchdog, which names the step the test last reached. The watchdog only turns a hang into a failure, so it is generous and not tuned to expected speed. A rescued timer must still not fire before its deadline; that lower bound reads the clock before the code under test reads it to fix the deadline. A wait whose first poll comes after its deadline completes without registering, so it shows nothing about rescue; where a waiter's first poll can come late, the test repeats the attempt rather than assume it came in time. A property that truly needs real time is not asserted on CI, and its test documents where it is run instead. The scenarios in `crates/native/src/monitor/scenarios.rs` follow this. "Within the `LATE` bound" below means within the 250 ms bound described for #92's tests, the earlier form of these checks. These tests still have wall-clock checks and are to be converted:

- `crates/native/tests/root_timers.rs` uses the `LATE` bound, and `DELAY * 4 + LATE`. Its rescue-disabled test asserts `start.elapsed() >= hold / 2`, but the held worker fixes its hold deadline before the test reads `start`, so a delayed test thread fails it. The file also has no watchdog.
- `crates/iouring/tests/stall_rescue.rs` uses the `LATE` bound.

The tests:

- **#92's root timer tests.** These are in [`crates/native/tests/root_timers.rs`](https://github.com/base-rs/base-scheduler/blob/861d9ddff11c8f6d7e2ca9e3949de553553e81fc/crates/native/tests/root_timers.rs) on #92's branch, and all three fail on `main`. They were brought over with their bounds relative to the hold and a 250 ms `LATE` bound. Under the rule above, that bound is to be replaced by the causal check: hold the worker until the timer has fired, and fail at a watchdog if it never does.
- **Stolen timer.** A task on worker A registers a sleep and a reusable timer. Another worker, B, then steals the task (force this with a hand-off between tasks), and A blocks. The timer must fire while A is still held.
- **Waker update to a stuck owner.** A reusable timer is registered on A. A blocks, the task is polled on B, and B replaces the timer's waker. The timer must fire while A is still held and wake the *new* waker. On `main`, the update would sit in A's inbox, so a rescuer would fire the old waker and the wakeup would be lost.
- **Stuck readiness.** A root `read_into` on a socket routed to a worker that then blocks. This is #92's scratch reproduction, which took 1.945 s on `main`; its source is [recorded on this PR](https://github.com/base-rs/base-scheduler/pull/93#issuecomment-6596074954). Also a socket registered by a task that is stolen before its registering worker blocks. Run both on epoll, and on kqueue on macOS and FreeBSD.
- **Runnable tasks.** Tasks queued behind a long poll, in both ready generations and in the remote inbox, run on other workers. This should already pass on `main`; add it so the property is checked.
- **Stuck preferred worker.** Every worker but A is kept saturated with local work, and A blocks. A rescued waker schedules a task whose preferred worker is A. The task must run without waiting for A or for the other workers to drain their local queues. This covers the stuck-preferred refinement ([decision 9](#decisions), described under [runnable tasks](#runnable-tasks)) and is added with it.
- **No spin on interrupts.** A victim stays stuck with a pending interrupt (an eventfd count, or a re-triggered `EVFILT_USER`) and no registration events. An idle worker's pre-park check must find nothing published, and the idle worker must park rather than loop through `Retry` for the whole stall.
- **Pre-park retry.** Every worker but one is idle, and that one is stuck. A timer on the stuck worker is rescued, and its waker schedules a task whose preferred worker is an idle worker. The task must run while the stuck worker is still held; this is repeated. If an idle worker's pre-park check rescues the timer, it must not park with `sleeping == false`. The monitor usually rescues first, so the scenario reaches the pre-park path only incidentally. `a_rescue_turns_an_empty_idle_search_into_a_retry` in `crates/native/src/worker/tests.rs` checks that a rescue turns an empty search into a retry.
- **No idle worker.** Every worker is held, a root `block_on` is waiting on a sleep, and the sleep's waker runs within the `LATE` bound.
- **io_uring.** A test that documents the interim behaviour for an in-flight operation on a stuck ring: it completes after the hold, not within the `LATE` bound. When stage 4 decides, add a test that new operations are routed away from a stuck ring if option 2 is adopted.
- **Monitor wake race.** Repeat this many times: no deadline is pending, so the monitor sleeps until woken; a worker then registers a timer and at once blocks, and the timer must still be rescued while that worker is held.
- **Races and lifecycle:**
  - the owner returns from its long poll during a rescue;
  - a stress test of repeated short stalls near the threshold;
  - closure and `shutdown()` while a worker is stuck and rescue is active;
  - driver retirement during rescue;
  - a waker that panics during rescue;
  - a waker that reenters the scheduler (spawns, registers or deregisters on the same controller) during rescue.

  Shutdown must not complete while a rescuer still holds a queue or poller. No record may be reclaimed while any batch, the owner's or a rescuer's, can still refer to it.
- **Rescue switched off.** With rescue switched off, behaviour matches `main`. Once stage 3 rescues readiness, `immediate_reads_observe_input_while_the_only_worker_is_busy` (`crates/native/tests/worker_readiness.rs`) must use the off switch: its premise is that the busy worker's driver publishes no readiness, which readiness rescue would break intermittently. Stage 2 leaves it unchanged.
- **Rescue classification counter** (test or diagnostic builds only). Each rescue is counted as a stuck poll or a long batch, or as the rescue of a worker leaving idle ([decision 16](#decisions)); a poll stuck for longer than *P* can still be counted as a long batch. In the named benchmarks, which contain no long polls, it should report zero stuck-poll rescues at the default *T*; report any it does record as false positives. Long-batch rescues are a normal effect of load: report them, and judge their cost through the budgets rather than the counter. A worker woken remotely is not sleeping as soon as its flag is cleared, and a closing worker publishes its remaining timers as due, so either can be counted if it fails to collect them within *S*; such counts are rescues, not detection errors.

**Inspection:**

- No timer-queue, table or notification lock is held across a waker call or a task poll. The poll lock is held across readiness wakers only, and is never held across a task poll.
- Lock order is poll lock, then table lock, then a direction's notification mutex. Timer-queue locks are leaves: they may be taken under any of these, but they never take one, and no path holds two timer-queue locks at once.
- A pre-park check that fires any waker makes the search return `Retry`.
- A rescuer pops a victim's timers only with `try_lock`. The monitor's recheck of published deadlines takes each queue lock briefly; no queue lock holder runs user code.
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
             reaching turn boundaries.
             Data arrives on sock. The kernel queues the event in A's poller,
             and nobody collects it.

3. Detect    A is marked stuck. For timers, the monitor wakes S after A's
             earliest deadline and finds it still published while A is not
             sleeping (decision 15). How stage 3 detects a readiness stall
             on a worker with no due timer is an open question.

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

- **Turn boundaries and the published deadline.** A worker collects its due timers at every turn boundary (`run_loop`'s timer pass) and refreshes its published deadline, and an idle worker wakes for its own deadline. So a deadline still published *S* after it fell due, on a worker that is not sleeping, means the worker has not reached a turn boundary since: it is stuck. Detection needs no sampling of the worker itself.
- **Published deadline.** Each worker keeps a lower bound of its earliest local deadline in an atomic `u64` of clock nanoseconds since the scheduler's epoch. Every timer queue mutation happens under the queue lock, including remote commands ([timers](#timers)), so whoever holds the lock updates it with plain loads and stores, with no read-modify-write: registration lowers it, and every pass that flushes the queue (expiry, rescue and the idle timeout) refreshes it exactly. Release leaves it early, which can only cause a redundant check, and avoids flushing the batched queue on every release. Detection depends on this value never being later than the queue's earliest live deadline; `worker_timers` asserts it in debug builds whenever it refreshes the value. `sharded::Publication::lower` takes a mutex, so it is not reused here. Unlike #92's shared deadline, no worker bounds its idle wait by another worker's deadline, so it cannot cause herds of wakeups.
- **Monitor (decided, [decision 15](#decisions)).** A dedicated thread:
  - It sleeps until the earliest published deadline plus *S*. A pass marks every worker whose deadline has been due for *S* while it is not sleeping, rescues its timers, and then samples it every *P* until it recovers, rescuing timers that fall due during the stall without a further grace period. Passes are at least *P* apart. A sleeping worker with an overdue deadline keeps the monitor looking every *P*: it may leave idle straight into a batch, skipping its timer pass, and an unchanged deadline would never wake the monitor again.
  - With no pending deadline it never wakes. With a timer expiring once a second it wakes about once a second.
  - It records the earliest deadline its sleep covers. A worker publishing a deadline more than *P* earlier unparks it; a deadline within *P* of the plan is at most *P* late. A pass stores its plan before rereading every published deadline under its queue lock (with `try_lock`, looking again a period later if the lock is busy), so a deadline that changed during the pass, lower or higher, is never missed. A worker decides under its queue lock whether to unpark the monitor and unparks it after unlocking. Closing a non-empty queue publishes a deadline of zero, and keeps doing so while timers remain, so the monitor drains a stuck owner's queue at shutdown; an owner computing its idle wait after closure waits no longer.
  - It rescues whether or not any worker is idle. With no idle worker, rescue wakes root futures and queues tasks earlier, as described in requirement 1.
- **Heartbeat.** Each worker stores a turn counter on its own cache line, with a relaxed store once per `run_loop` iteration and once when it leaves idle. Detection does not use it: it clears a worker's stuck mark once the worker reaches a turn boundary, and in diagnostic builds it classifies stalls.

  A long batch of short tasks counts as stalled too ([decision 13](#decisions)). A batch is the frozen generation that `run_ready_batch` drains between two turn boundaries, and it has no length limit, so a batch of many short polls can outlast *S* without any poll misbehaving. Rescuing in that case also reduces latency, since `context/design.md` notes that "a large batch can delay I/O checks". Test and diagnostic builds classify each rescue by what the worker was doing ([decision 16](#decisions)): **leaving idle** (in idle code and no longer sleeping), **long batch** (a task poll completed within *P* of the mark), or **stuck poll** (none did).
- **Idle and closing.** The existing `sleeping` flag marks a worker in its idle wait. A remote wake clears `sleeping` before the owner leaves its wait (`WorkerState::wake`), and the shutdown drain also runs with `sleeping == false`. A worker that is closing is rescued like any other if its queue holds due timers; once closure has finished, the monitor exits.
- **Pre-park check (decided).** In the existing steal search before parking, a worker also checks each victim that is not sleeping. The search runs after the worker has stored `sleeping = true`. If the check fires any waker, the search returns `Retry` instead of `Empty`: the worker rescans every queue and advertises sleep again before it parks. This retry is required. A woken task can be scheduled back to this same worker. `WorkerState::wake` then clears `sleeping` but skips the unpark on the self-thread path, and the task may land in a queue the search has already passed. Without the retry, the worker would park with `sleeping == false` and later `wake_idle` calls would skip it. Running the check before sleep is advertised was considered and rejected. It would move the check off the path where the worker already scans its victims, and a later steal search would still have to repeat it.

  Both checks act only on a victim the monitor has marked stuck ([decision 14](#decisions)), so steady-state behaviour stays as it is on `main`:
  - **Timers:** if the victim is marked stuck and its published deadline has passed, it `try_lock`s the victim's queue and fires its expired timers. Unlike Go's `stealWork`, it does not fire a healthy victim's expired timers, which would move their tasks to the parking worker and make locality loss routine.
  - **Readiness:** if the victim is marked stuck, it calls `try_assist` on the victim's poller.

  Each check costs one load of the stuck mark, so this path adds no clock reads or syscalls unless a victim is actually stuck. It helps only for timers that fall due on a victim already marked stuck, because the monitor rescues a victim's due timers in the pass that marks it; it then shortens *T* for whichever worker is about to sleep anyway. Its end-to-end path is exercised only incidentally by the scenario tests, which the monitor usually wins; the retry and the check are unit-tested. The check runs as its own pass over the victims, after the worker advertises sleep and before its second steal search, so it does not depend on `search`'s `has_runnables()` early return.
- **Threshold trade-offs.** A small *S* gives better worst-case latency. It also costs more rescues of long legitimate batches, more rescues of workers that are leaving idle or are preempted (measured for [decision 16](#decisions)), more `try_lock` contention with owners about to return, and more lost locality as rescued tasks are stolen. The monitor's wake rate depends on how often deadlines fall due, at most 1/*P*, not on *S*. The 1 ms threshold is much tighter than Go's sysmon, which waits about 10 ms before polling the network or preempting.

### Timers

- **Lockable queues.** Each worker's `local::TimerQueue<Waker>` moves out of the TLS `RefCell` into `WorkerState`, behind a small spinlock whose unlock is a release store (`crates/native/src/spin_lock.rs`). The owner locks it, uncontended, for registration, update, release and its timer pass. No lock holder runs user code, blocks, or holds it across a task poll or a waker call; a contender spins briefly, then yields. The queue, its published deadline, a copy of the scheduler's epoch and a per-queue closed flag share their own 128-byte block, so timer operations touch no line that other workers write on every schedule.
- **Batch expiry.** The owner and rescuers lock once, pop up to 256 expired wakers into a reusable buffer with `local::TimerQueue::pop_expired_batch`, unlock, and then call the wakers.
- **Rescuer.** A rescuer uses `try_lock`, pops expired entries, releases the lock and calls the wakers. Each waker makes its task runnable in its preferred worker's inbox, and `wake_idle` falls back to any idle worker, which steals the task. Keys stay in their original queue, and each slot keeps its capacity until `Release`.
- **Remote commands (decided).** `Register`, `Update` and `Release` lock the target queue directly. They interrupt the owner only if they published an earlier deadline while it is idle. This replaces the single-consumer `SegQueue` inbox. That inbox would leave a root `Register` unacknowledged behind a stuck owner. It would also leave a waker `Update` undelivered, so a rescuer would fire the stale waker and lose the wakeup. The direct lock also removes the acknowledgement allocation for remote registration.

  **Ordering.** The owner stores `sleeping = true` (`SeqCst`) before it locks its queue to compute its wait timeout. A remote command inserts under the queue lock and checks `sleeping` after it unlocks, interrupting through `WorkerState::wake` if it lowered the head. The mutex orders the two, so either the owner's timeout computation sees the new entry or the command sees `sleeping`. This replaces the inbox recheck in `worker_timers::timeout`.
- **Closure.** Closure sets each queue's closed flag under its lock before publishing scheduler closure, so registration checks only the queue it locks. A pass that observes closure pops every entry, whether the owner's or a rescuer's. Shutdown waits for in-flight rescue waker calls, as it already does for timer callbacks: the monitor counts as a worker in `workers_remaining`.

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
2. **Route new operations away from a stuck ring.** The monitor sets the stall hint. `route_ref`'s external round robin and the ring selection for borrowed waits then skip rings marked stuck, at a cost of one load per external route. An existing persistent monitor stays on the old ring until its poll retires, so its events would still wait for the stuck owner; moving it would need the old ring to retire its poll, which would stall as well. This needs designing.
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
- An uncontended spinlock acquire and release in place of a `RefCell` borrow for each timer operation, and one lock per 256 expired timers for each timer pass.
- A plain store of the published deadline whenever the queue head changes. For per-request timeouts this means most registrations and releases, because the queue oscillates between empty and one timer.
- For remote timer commands, a lock on the target queue in place of an inbox push and acknowledgement allocation.
- An uncontended poll lock taken for each driver turn, with a `try_lock` on zero-timeout turns.
- One load per victim in the pre-park search.
- One load of the monitor's plan whenever a queue's published deadline changes, and a monitor wake only when a deadline is more than *P* earlier than its plan.
- Monitor wakes about once per deadline that falls due, at most 1/*P*, and none while no deadline is pending.
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

- Monitor wake rate and CPU, reported alongside the workers' CPU, not hidden in process totals.
- Fixed-rate p99 latency with every core occupied, where the monitor competes with workers.
- A net-benefit workload with undeclared short blocking, for example tasks that occasionally poll CPU-bound for 5 ms among I/O and timer traffic. Compare p99 and throughput with rescue on and off.
- Rescue counts for every workload, classified as stuck polls, long batches or workers leaving idle, with false positives identified, and rescue counts per registration. [Decision 7](#decisions) revisits migration from the per-registration counts.

Fixed-rate latency is reported separately from throughput and CPU, as [worker progress](worker-progress.md) requires.

### Lifetimes

- Each worker's state holds an erased `Option<Arc<dyn SelectorAssist>>` in a `OnceLock`, set by `run_worker` from `SelectorDriver::assistant` before it enters the driver.
- The monitor holds only a `Weak<Shared>`. It upgrades that reference at the start of each pass and drops it at the end. It exits when the upgrade fails, or when it observes that scheduler closure has finished.
- Workers that use assistants in the pre-park search are already scoped by `Shared`.
- Safety does not depend on the monitor exiting before drivers are destroyed. An assistant stays valid after its driver stops: it observes the retired mark under the poll lock and returns `Retired`.
- **Stopping the monitor.** Closure unparks the monitor through `wake_all`; there is no separate stop signal. The monitor never joins itself. While it holds an upgraded reference it can hold the last strong `Arc<Shared>`, so `Shared` may drop on the monitor thread. For that reason the join handle is not held in `Shared`, and `Shared::drop` never joins.
- **Joining.** Native has no synchronous shutdown path today: `NativeScheduler::drop` only calls `close()` (`crates/native/src/native_scheduler.rs`), `shutdown()` is asynchronous, and worker threads are already detached (`crates/native/src/thread.rs`). The monitor is therefore detached in the same way. It can park indefinitely when no deadline is pending; closure and the final retirement call `wake_all`, which unparks it, and it exits once closure has finished or its upgrade fails. If a synchronous shutdown path is added later, it may join the monitor from a thread that is not the monitor, outside `Shared::drop`. An asynchronous shutdown must not block an executor thread on a join. A join would be for tidiness, not memory safety.

### Risks

- **False positives.** See the threshold trade-offs. A worker woken remotely, or a closing worker, can be marked if it fails to collect due timers within *S*; that rescue is harmless. In the periodic diagnostic, the marked workers were leaving idle or had completed polls by the time their mark cleared ([decision 16](#decisions)).
- **Locality loss.** Rescued tasks are stolen. They keep paying cross-worker costs while their registrations stay on a stuck worker.
- **Owner contention.** An owner entering an idle wait can block behind one rescuer batch (readiness), or behind a rescuer's expired-timer pop (timers).
- **Waker code on the monitor thread.** Wakers run on the monitor thread. A foreign executor's slow or panicking waker can delay rescue of other workers, and can delay an idle owner of that poller. Contain panics, and consider capping each rescue pass.
- **The monitor's own CPU.** It wakes about once per deadline that falls due, up to 1,000 times a second when timers fall due continuously, and then competes with workers on fully loaded machines. It is measured explicitly.
- **The published-deadline invariant.** Detection assumes each queue's published deadline is never later than its earliest live deadline. A later value would delay rescue, and a stale earlier value can mark a worker that has not reached a turn boundary for *S* even though nothing was stranded, which only costs a redundant rescue pass. Debug builds assert the invariant.
- **Spinning.** The timer-queue spinlock has no fairness. A holder preempted on an oversubscribed machine makes contenders spin and yield until it runs again. Critical sections are short but not constant: a full queue's capacity reclamation, or a 256-entry batch pop.
- **Shutdown.** See [lifetimes](#lifetimes). A rescue must never touch a stopped controller or a closed queue except to drain it.
- **Untested platforms.** Once stage 3 adds the planned CI jobs, readiness tests run on Linux, macOS and FreeBSD only; today CI runs Linux, macOS and Windows. NetBSD will be build-checked, and OpenBSD and DragonFly are tier 3 targets with at most a build check, so a kqueue difference there, such as a missing `EVFILT_USER`, may surface only from users.
- **Masking.** Rescue can hide blocking that should be `spawn_blocking`. Consider a diagnostic counter or log for repeated stalls of the same worker.

## Stage 2 implementation notes

These record where the stage 2 code refines the proposed mechanism.

- **Detection.** The periodic sampling and deep-sleep handshake originally specified were implemented first and measured: about 1,000 monitor wakes a second whenever any worker was busy, which showed up as +2–5% server CPU in pinned four-worker echo and +40–150% CPU (about 0.1 ms per second) at one request or timer wake per second. A worker alternating between idle and busy every few microseconds also woke the monitor from deep sleep on nearly every exit from idle. The deadline-driven monitor in [detection](#detection) replaced both ([decision 15](#decisions)).
- **Timer queues.** With `std::sync::Mutex` the one-task `refresh` benchmark was +2.7% against `main`; with the spinlock it is −5%. Per-lock overhead, not hold time, dominated the 1,024-timer diagnostic: one waker per lock raised its p99 lateness from about 230 to 309 µs, 16 to 257 µs and 64 to 237 µs, while 256 matched `main` (229 µs) with 6% less CPU. Registration from any thread is synchronous, so `NativeTimer` no longer holds a pending acknowledgement, and root registrations allocate nothing.
- **Owner timer pass.** When the published deadline is `NO_DEADLINE` the pass returns without reading the clock or taking the lock, and when it is later than now it returns without the lock.
- **Timer cache lines.** An early measurement put the queue inline in `WorkerState` and checked `Shared`'s closed flag and epoch on each registration; both share cache lines with state that every worker writes on each schedule, and the 64-task, 4-worker `yield` refresh benchmark ran about 25–90% slower than `main` until they moved.
- **`Shared::outstanding` isolated (#94).** Every worker updates the runnable counter on each schedule and retirement, and `retire_runnable` then reads `closed`. Adding fields to `Shared` placed `closed` in the same 128-byte block, which Intel's adjacent-line prefetcher moves between cores as a pair; `perf` showed the `closed` load taking a quarter of `run_loop`'s cycles, and task-heavy workloads ran 5–12% slower with identical instruction counts. `main` already suffered part of this, so the fix is its own pull request, and stage 2 is measured against `main` with it.
- **Registrations borrow the scheduler.** A timer registration held an `Arc<Shared>` clone, as on `main`, so every registration and release updated one reference count shared by all workers; `perf annotate` put the `lock incq` and `lock decq` on it among the timer path's hottest instructions. A sleep future already borrows its scheduler, so `Registration` now borrows `Shared`, and `NativeTimer`, which owns an `Arc`, stores only the key. The 64-task, 4-worker `sleep yield` case went from about 3% slower than `main` to about 8% faster on io_uring and 10% faster on the readiness selector.
- **Inline hints.** `has_runnables`, `is_closed` and `finished` are `#[inline]`. Without them the branch executed 1–2% more instructions and about 1% more cycles in the `sleep yield` and `sleep refresh` cases (5 runs each); `main` does not mark them.
- **Unaligned diagnostic lateness.** Once registration became cheaper, the 4-worker, 1,024-timer diagnostic with unaligned deadlines showed p50 lateness about 21% above `main` over 30 paired runs (77 to 94 µs), with p99 within 4% and CPU 11% lower. Deadline-ordered expiry, larger batches and skipping the monitor's locked recheck did not change it, and with rescue off it was about 9%. With deadlines aligned to a millisecond grid, p50 was within 5% and p99 7% better. The likely cause is the benchmark's own dynamics: each task's next deadline is its run time plus 1 ms, so per-task cost changes how deadlines cluster. This is a hypothesis supported by those controls, not proven. [Later paired measurement](../benchmarks/timers/results/rescue-costs/readme.md) found the regression with stall rescue off as well (p50 higher than before in 14 of 15 repeats, on and off alike), superseding the 9% figure above, which came from one run on an earlier build: it comes from the rest of the change, not the monitor. At 4 workers the same expiries arrived in fewer, larger passes; at 8 workers pass counts overlapped completely. [Timer batching](../benchmarks/timers/results/timer-batching/readme.md) supported the hypothesis with one build: adding about 50 multiply-adds of work per expiry brought `main`'s p50 down to the earlier build's, and more work lowered it further, at about the same CPU with 4 workers. At 4 workers, pass order and batch size were not distinguishable from build-to-build noise; at 8 workers the old pass recovered the gap. With phases fixed so that deadlines do not depend on run times, p50 was 4.5–6% higher than before the rework (0.14–0.28 µs later), with a layout control within 1%. Leaving an expiry pass's idle wakes to the ready batch lowered the unaligned diagnostic's p50 by 14–45%, but raised steady-load p99 at 4 workers and io_uring CPU in the unaligned diagnostic, and the evidence did not clearly favour it, so it was not adopted.
- **io_uring `reuse refresh`.** This single-task case, which in steady state does not touch the queue, runs about 7% slower than `main` with the same instruction count. `ld_blocks_partial.address_alias` (4 KiB aliasing) rises from about 6 to 64 million events: an address-layout artefact of that build, absent from the same case on the readiness selector and smaller in earlier builds. In the final acceptance run it was 1.1% slower, within budget.
- **Classification.** The stuck mark records the classification, so that rescues by idle workers are classified too. A worker leaving idle is classified when it is marked; any other worker is unclassified until the first monitor pass at least *P* after the mark, and rescues made meanwhile are held until it is classified ([decision 16](#decisions)). The per-poll counter and the idle flag exist only with `cfg(test)` or the `diagnostics` feature, which also adds `NativeScheduler::rescue_statistics`.
- **Monitor lifetime.** It counts in `workers_remaining` through a registration that holds only a weak reference, so `shutdown()` waits for its last rescue. `wake_all` also unparks it. The thread that starts the monitor registers its handle before the scheduler is returned: when the monitor registered itself, a monitor scheduled late could miss every wake of a scheduler that closed and finished first, then park forever with shutdown waiting for it. CI hit this as a hang in a test that creates and shuts down schedulers in a loop.
- **Parameters.** *S* = *P* = 1 ms, as private constants in `crates/native/src/monitor.rs`. Measured worst-case lateness behind a 200 ms stall is about 1.2–1.5 ms on hana, against 185 ms on `main`.
- **Quick benchmark.** `benchmarks/echo/results/stall-rescue/scripts/quick.py` compares builds in about three minutes: timer refresh and yield, the worker-local timer diagnostic, fixed-rate and one-request-per-second echo, a one-hertz timer, and stalled-worker lateness. It was used to iterate on the design; the full budget run remains the acceptance check.
- **Tests.** #92's tests are in `crates/native/tests/root_timers.rs` with `LATE` = 250 ms, alongside the rescue-off and no-idle-worker cases. Scenarios that place work on particular workers are in `crates/native/src/monitor/scenarios.rs`; the monitor pass, its wake handshake, the pre-park check and the scheduling refinement also have deterministic unit tests.
- **Acceptance measurements.** [`benchmarks/echo/results/stall-rescue`](../benchmarks/echo/results/stall-rescue/readme.md) holds the final comparison with `main`, every row over budget, and the monitor's measured wake rate. The rows over budget include the unaligned diagnostic's p50 lateness (+26%), `pulse` CPU (+28%, one monitor wake per deadline), and sub-microsecond to tens-of-microseconds lateness shifts in the `cycle` cases, whose ranges on `main` are wider than the change. The worst-case stall lateness falls from 185 ms to 1.3 ms. The readme also names the unbudgeted measurements from [validation](#validation) that were not taken: p99 with every core occupied, occasional undeclared blocking, and rescue counts per workload. Rescue counts were taken later; see the next note. [`benchmarks/timers/results/rescue-costs`](../benchmarks/timers/results/rescue-costs/readme.md) separates the monitor's cost from the rest of the change for the unaligned diagnostic and `pulse` rows: the monitor costs a few percent of CPU on io_uring and about 0.1 ms per run for `pulse`, and the p50 regression is the same without it.
- **Rescue counts.** [`benchmarks/timers/results/rescue-counts`](../benchmarks/timers/results/rescue-counts/readme.md) records rescues for every acceptance workload, with `diagnostics` builds, 3 runs each. The echo matrix, spawning, idle, pulse and root sleeps record none. One `task-sleep` run recorded 3 stuck-poll rescues (10 timers), and one io_uring `reuse expire 128/4` run 1 (39 timers). The 1,024-timer periodic diagnostic recorded stuck-poll and long-batch rescues in 8 of 12 runs on both selectors, at most 0.061% of a run's expiries. Its long-batch rescues are an expected effect of load; its stuck-poll rescues, like the two single runs', count as false positives by the [validation](#validation)'s definition, since nothing in these workloads blocks, unless the validation's exemption for remotely woken or closing workers explains them. [`benchmarks/timers/results/rescue-causes`](../benchmarks/timers/results/rescue-causes/readme.md) later measured the periodic diagnostic again: its marked workers were leaving idle or had completed polls by the time the mark cleared ([decision 16](#decisions)). Per-registration rescue counts, which [decision 7](#decisions) would use, need a new diagnostic: `RescueStatistics` counts only rescues and rescued timers.

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
   - the heartbeat and the deadline-driven monitor ([decision 15](#decisions));
   - the off switch;
   - lockable timer queues with batch expiry;
   - remote commands that lock the target queue directly;
   - the published deadline;
   - timer rescue, both from the monitor and from the pre-park check;
   - the stuck-preferred-worker refinement in `schedule`.

   It lands in the same pull request as this document ([decision 12](#decisions)). The echo harness that the budgets need is #95, and its baseline is measured on `main` with the same harness ([decision 10](#decisions)).

   Bring over #92's tests and add every [requirement 1 test](#requirement-1) that concerns timers or scheduling: stolen timer, waker update, runnable tasks, stuck preferred worker, pre-park retry, no idle worker, the io_uring interim, the monitor wake race, the races and lifecycle cases that apply to timer queues, rescue switched off, and the rescue classification counter. The readiness tests (stuck readiness, no spin on interrupts, driver retirement) belong to stage 3. Document the io_uring exclusion as the interim option 1. Update the Native readme's timer and platform descriptions (audit item D; its line saying other edge platforms use `polling` is already stale for Linux and Android), and `context/design.md` (audit item H). Measure against the budgets.
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
  - **Why deferring costs little.** Migration moves where a socket's events are collected. Since wakes from a worker's own driver turn stay on that worker ([`driver-wakes`](../benchmarks/echo/results/driver-wakes/readme.md)), that also decides where a woken task is queued, though idle workers can still steal it. Rescued tasks already run on other workers, and with the stuck-preferred-worker refinement they skip the stuck worker's inbox. The most migration could save is one rescue delay per event, during stalls that are themselves bugs.
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
- **10. Measurement harnesses:** built for stage 2, because several budgets cannot be checked with the tools in the repository today. The echo changes are #95. The worker-local timer diagnostic and #92's spawn benchmark were first kept as scripts under `benchmarks/echo/results/stall-rescue/scripts/`, which is how the stage 2 acceptance results were produced. The maintainer chose Cargo targets instead: they are now the `periodic` and `scenario` modes of `base-scheduler-benchmark-timers`, tested under workspace coverage, and its `diagnostics` feature, like the echo benchmark's, reports rescue counts. The original list:
  - a Native readiness backend for the echo benchmark, starting from `benchmarks/echo/results/turn-interval/echo-native-backend.patch`;
  - an offered-rate option on the echo client, for fixed-rate latency and the 1 request per second idle-CPU case;
  - an idle-CPU script for the 10 s no-load measurement;
  - the worker-local timer diagnostic as a Cargo target in the private benchmark package.

  Each is added in the first stage 2 pull request that measures against it, and its baseline is measured on `main` with the same harness. The diagnostic and the spawn benchmark were first used as scripts, as above, and became package targets afterwards.
- **11. Noise and host:** budgets are checked on Linux on [hana](https://github.com/base-rs/base-benchmark/blob/main/hosts/hana.md), which the maintainer chose over Aiko for stability, using the CPU pinning procedure of the existing echo reports (for example [`benchmarks/echo/double-buffered-ready-performance.md`](../benchmarks/echo/double-buffered-ready-performance.md)) under the exclusive host lock `/tmp/base-benchmark-host.lock` that hana's document asks every measurement to take. Each case takes 9 paired, randomly ordered runs of `main` and the branch:
  - "within noise" means the branch's median lies inside `main`'s min–max range for that case;
  - "≤ ~3%" is the difference of the paired medians.

  macOS results are supplementary and do not decide a budget.
- **12. Stage 2 organisation:** stage 2 lands in the same pull request as this document, so that both are reviewed together. This replaces the earlier plan of three pull requests after the document merged. Independent pieces are split out: the runnable counter's cache isolation (#94) and the echo harness (#95).
- **13. Long batches:** a batch of short polls that outlasts *S* counts as a stall and is rescued, because rescue then bounds timer and readiness latency that the batch would otherwise delay. The release hot path stays at one heartbeat store per loop iteration. Test and diagnostic builds also count task polls, so each rescue is classified as a stuck poll or a long batch, and the validation expectation of zero false positives applies to stuck polls only. [Decision 16](#decisions) replaces that classification.
  - **Why rescue helps less under saturation.** When every worker is busy, `wake_idle` returns early, so rescued tasks are only queued sooner, while the monitor competes for CPU and the poll lock. The gain is mainly for root futures and timer wakeups.
  - **Conditional gate.** If the saturated echo runs (4/128/4096) miss their budget, rescue long batches only when `idle_workers > 0`, and always rescue stuck polls. Telling the two apart in release builds then needs the per-poll counter there too: one relaxed store per task poll, on the heartbeat's cache line. Under [decision 16](#decisions) a stuck poll is known only a period after the mark, so the gate would decide on that later sample rather than at the first rescue.
- **14. Pre-park timer rescue:** gated on the monitor's stuck mark, as readiness is. A pre-park check that fired a healthy victim's expired timers, as Go's `stealWork` does, would move those tasks to the parking worker in steady state, make the owner's timer pass wait behind the rescuer's pop, and add clock reads to the idle path. Rescue stays strictly after the fact, and the monitor already guarantees *T*. Gating on how overdue a timer is (`now − deadline > S`) was considered; it would save at most about one sampling period *P*, at the cost of those clock reads.
- **15. Detection pays only when a stall could strand work:** the maintainer asked that the cost be paid only under contention or when work has to move, with the goal of eliminating stalled-worker latency without affecting nominal latency, and left the mechanism to measurement. For timers, the monitor now sleeps until a deadline has been due for *S* and checks whether it is still published on a worker that is not sleeping ([detection](#detection)), instead of sampling every *P* while any worker is busy. This replaces the periodic sampling and deep-sleep handshake of decision 2's original form; the dedicated monitor and the pre-park check remain. The measurements that motivated it are in the [implementation notes](#stage-2-implementation-notes).
- **16. Classifying rescues:** the maintainer accepted the rescues measured in the periodic diagnostic, which contains no long polls, and chose to classify a rescue by what the worker is doing rather than by task polls since the monitor's previous pass. That label called 40 of 92 marks there stuck polls. In [that measurement](../benchmarks/timers/results/rescue-causes/readme.md), with the worker's phase read before rescuing, 50 marks were workers in idle code that were no longer sleeping (40 of them waiting in the run queue after a wake), and the other 42 had completed task polls by the time the mark cleared; no wake from another thread was lost in 560,000 tries. Polls were sampled at the monitor's next pass for only 5 of those 42; in the longest marks the monitor and the workers were starved together, so a poll stuck for longer than *P* during them cannot be ruled out. The classification is: a worker in idle code that is no longer sleeping is **leaving idle**, which the validation exempts; otherwise a worker that completes a task poll by the monitor's next pass, at least *P* after the mark, is in a **long batch**, and one that does not is in a **stuck poll**. A stuck poll is any stall with no completed poll for that long, including a worker preempted between polls or in a long timer pass or driver turn. A poll that ends soon after the mark, or a late pass, counts as a long batch, so a poll stuck for longer than *P* can go uncounted. Rescues made before a mark is classified are counted when it is. It applies to test and diagnostic builds only.

## Open questions

Questions 1–8 were decided, decisions 9–12 settle the questions raised when stage 2 was planned, decisions 13 and 14 settle the questions raised in review, decision 15 the detection change from measurement, and decision 16 the classification of the rescues measured in workloads without long polls. All are recorded under [Decisions](#decisions); questions 1–8 keep their original numbers so that earlier references stay valid.

- **Stage 3 detection.** Readiness events are not announced in advance, so the deadline-driven monitor cannot see a worker stuck with queued events but no due timer. Candidates, to measure in stage 3: an idle worker's pre-park check compares a victim's heartbeat with the value it saw on its previous check and assists a victim whose heartbeat has not moved for *S*, which costs nothing unless some worker is idle and so fits requirement 1's condition exactly; or the monitor samples only workers with registrations, at *P*, while at least one worker is idle.
