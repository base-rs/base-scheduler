# Scoped spawning

Proposed 2026-11-21; not yet accepted or implemented.

## Requirement

A scheduler can run tasks that borrow from the code which created them, provided every such task has finished, and been destroyed, before that code's scope ends. This is structured concurrency, as `std::thread::scope` provides for threads: an application creates its scheduler, runs its program inside one scope, and the scope does not return until every task started in it is gone.

Interface:

- The interface crate defines a capability, `SpawnScoped`, with one entry point, `scope`. It takes an asynchronous root, which receives a `&Scope`, and blocks the calling thread until the root has returned and every task spawned through the scope has been destroyed, then returns the root's output:

  ```rust
  let output = scheduler.scope(async |scope| {
      // Tasks spawned here may borrow anything that outlives the call to
      // `scope`, including the scheduler itself.
      let task = scope.spawn(async { work(&borrowed).await })?;
      task.await
  });
  ```

- `Scope::spawn` takes a future that is `Send` and outlives the scope, not necessarily `'static`, whose output is also `Send` and outlives the scope. It registers the task before it becomes runnable and returns `Result<ScopedTask<'scope, T>, SpawnError>`, as `Spawn::spawn` does. A task can spawn further tasks into the same scope through a borrowed `&Scope`.

- `ScopedTask` follows the existing task-handle contract: it owns the task's execution, awaiting it returns the output or `TaskError`, and dropping it requests destruction once any active poll returns. It never detaches. The scope also waits for that destruction.

- The root runs on the calling thread, as Native's `block_on` runs its root, so it too may borrow local state. Spawned tasks run on the scheduler's workers, in parallel with the root and with each other.

- `scope` is synchronous and blocking. It must be called outside the scheduler's tasks and workers: a backend that can detect a call from one of its workers panics, as Native's `block_on` already does, and the documentation states that a call from inside asynchronous code elsewhere can deadlock. There is no asynchronous form, because Rust cannot make one sound: a caller can forget an asynchronous scope's future without running its cleanup, and borrowed data would then outlive the tasks using it.

- A closed scheduler rejects `Scope::spawn` with `SpawnError::SchedulerClosed`, dropping the future without polling it. `scope` itself still runs its root and waits for any tasks admitted before closure.

- Panics follow `std::thread::scope`. If the root panics, `scope` still waits for every task, then resumes the root's panic. A task's panic is reported to its handle as `TaskError::Panicked`, as now. If a task panicked and its handle was dropped without observing the panic, `scope` panics after every task has been destroyed, so a failure is never silent. A panic during a task's or output's destruction is contained as the existing guards contain it.

Soundness boundary:

- When `scope` returns, normally or by unwinding, no task's future or output remains, and nothing the backend still holds, such as a waker, queued runnable or I/O registration, can reach either. A backend that cannot guarantee this for some path, such as an abrupt runtime teardown, blocks `scope` until it can, rather than returning; a scope that waits forever is a bug, never unsoundness.
- The interface crate implements `scope`, `Scope` and `ScopedTask` once for every backend, on top of an `unsafe` backend primitive that spawns a future without the `'static` bound. That primitive's contract states exactly what the backend guarantees about destroying the future and its storage, and the interface crate's join, panic and output handling rely only on that contract. Each `unsafe` block carries a `// SAFETY:` comment naming the obligation it discharges.
- Outputs are delivered through storage the scope owns, never left in a runtime's own task storage after the task is destroyed, since a runtime may drop such storage later, from another thread. A task whose handle was dropped has its output destroyed before the scope's join completes.

Backends:

- Native implements the primitive with `async-task`'s `spawn_unchecked`, whose documented condition, that borrowed data outlive the task's `Runnable` and `Task`, the backend satisfies by tracking both for each scoped task.
- The Tokio adapter implements it by erasing the future's lifetime before `Runtime::spawn`, with a guard in the spawned future that reports its destruction, including when Tokio destroys it on abort or runtime shutdown. `scope` waits for that report, so a Tokio runtime destroyed while a scope is active cannot leave a borrowed future alive.
- Existing `Spawn::spawn`, its `'static` bounds and its handles are unchanged. Scoped tasks count as live task owners for graceful shutdown, as spawned tasks do.

## Rationale

The user's model of a program is the Ruby form `scheduler = Scheduler.new; begin; user_code(scheduler); ensure; scheduler.close; end`: everything inside may use the scheduler, including tasks it starts, because they all stop before the scheduler goes away. Async structures tasks the same way, with children belonging to their parent. Base's protocol stack already follows this model through borrowing: socket readers and writers borrow their scheduler, so an HTTP connection's request bodies, a server's handlers and a client all live for the scope in which the scheduler is borrowed. `Spawn::spawn` cannot run such a connection on a worker task, because nothing ties a spawned task to the scope, and so it requires `'static` futures, as [the interface design](../context/design.md#future-and-thread-bounds) records; that context anticipates a separately designed interface for futures that are not `'static`.

`base-http` needs one, decided with the user on 2026-11-21. Its M6 milestone adds an opt-in mode in which each connection of a client or server is its own task, so that connections progress on more than one worker, and a spawned server must be able to forward a request body through a client in the same program, as a load balancer does. With `Spawn` alone, the connection would need a scheduler borrowed for `'static`, by leaking it, or reader and writer handles that own their scheduler, a change to every backend's I/O factories, to `base-endpoint`'s TLS streams and to every protocol above them. A scoped spawn instead keeps every borrowing API as it is: a scoped connection task has the same lifetimes as the scoped mode, and only where its future runs changes. The user chose it over leaking the scheduler and over owning handles.

Blocking is what makes the scope sound, as for `std::thread::scope` and crossbeam's scopes; asynchronous scoped spawning without blocking is known to be unsound for the reason given above, which is why runtimes such as Tokio offer none. Implementing the join once, in the interface crate, over a narrow `unsafe` primitive keeps the reasoning that soundness depends on in one place, reviewed and tested once, instead of in each backend.

## Scope

The `SpawnScoped` capability, `Scope`, `ScopedTask` and the backend primitive in the interface crate; their implementations in `base-scheduler-native` and `base-scheduler-tokio`; the conformance crate's scoped-spawn cases; and the interface design context, which gains a section for scoped tasks. It ships in the 0.6 release planned with [packet ECN](packet-ecn.md). Executor-independent I/O, timers and selectors are unaffected.

## Validation

Inspect the interface crate's `scope` implementation and the backend primitive: every `unsafe` block has a `// SAFETY:` comment, and the join, panic and output paths rely only on the primitive's stated contract. The conformance crate generates the following cases for every backend that implements `SpawnScoped`, each borrowing data from the caller's stack:

- A task borrowing caller data completes, and its output reaches the root.
- Tasks run on more than one worker in parallel, observed by tasks that wait for each other across workers.
- A task whose handle is dropped while it is pending is destroyed before `scope` returns, observed through a destructor that records into borrowed data after the task's future would have run.
- A task whose handle is dropped after it completes has its output destroyed before `scope` returns, observed through the output's destructor.
- Tasks spawned by tasks, including after the root returns, are waited for.
- The root's panic is resumed after every task is destroyed; a task's unobserved panic makes `scope` panic after the join; an observed one does not.
- Spawning after the scheduler closes returns `SchedulerClosed` and drops the future unpolled, and `scope` still returns.
- On Native, calling `scope` from a worker panics.
- On Tokio, shutting the runtime down during a scope destroys its tasks, and `scope` returns only after their destruction is reported.

The workspace's coverage requirement applies. Running the interface crate's scope tests under Miri, with a test-only backend primitive, is desirable where the tools permit, and its result is recorded.
