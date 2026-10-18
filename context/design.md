# Scheduler interface and behavioral contracts

## Scope and provenance

The root `protocol-scheduler` crate defines the interaction between protocol libraries and their execution environment. It implements runtime-independent cancellation and buffer support, but contains no scheduler, executor, reactor, operating-system I/O, or runtime adapter.

The initial extraction follows the Core ecosystem design and the [original interface proposal](proposal.md). Its source is [Socketry revision `b54b8d61bf80b71a1a5b97c3f5b80bae8a5ebabd`](https://github.com/core-rs/socketry-rust/tree/b54b8d61bf80b71a1a5b97c3f5b80bae8a5ebabd), specifically `crates/executor/src/scheduler/*.rs`, capability and shared-value modules in `crates/executor/src/`, and their runtime-independent tests. Existing MIT attribution is retained.

Preserve the existing operation-level traits. `FileOperation` and `SocketOperation` associate resources with a scheduler; `FileRead`, `SocketWrite`, and the other operation traits supply independent capabilities. `FileScheduler` and `SocketScheduler` are blanket convenience aggregates, not requirements imposed on every protocol library. There is no universal scheduler trait requiring unrelated facilities.

The original proposal remains design history. The public Rust traits and this document describe the implemented initial contract. No crate publication or migration of Socketry is part of this extraction.

## Explicit execution services

Protocol values, parsing, and state transitions do not inherently need scheduler capabilities. Operations which make asynchronous progress receive the capabilities and cancellation they need explicitly. An operation future retains those arguments until completion or abandonment. A protocol object must not cache a scheduler from construction or a previous operation, or find an implicit current scheduler.

Moving a `Send` operation future to another polling thread retains its original scheduler and token. Moving a protocol object between completed operations lets its next call select another compatible scheduler and token. The latter does not promise arbitrary resource migration.

Implementations choose their own file, socket, metadata, signal-registration, and task-handle types. The interface imposes no `Arc<std::fs::File>`, native descriptor, Tokio handle, readiness registration, or ring-operation representation. A resource may retain backend state needed for safety and registration lifetime, but it must not silently select a different scheduler than the operation's explicit argument.

Backends document scheduler/resource compatibility, including instance affinity. Compatible scheduler instances can reuse a resource between operations. Backends must reject incompatible pairings with `io::ErrorKind::InvalidInput` before operation submission where the type system cannot reject them. Rebinding or conversion, where supported, is an explicit backend operation. Application code finishes outstanding operations before resource transfer or close.

## Future and thread bounds

The extraction preserves Socketry's `Send` operation futures and `Send + Sync` file/socket resources and I/O capabilities. Static dispatch uses return-position `impl Future`; there is no mandatory allocation or dynamic dispatch for operations. Task and blocking-work handles are generic associated types, allowing different backends to retain their own representations.

Spawned futures, blocking closures, their outputs, and their handles are `Send + 'static`. This prevents borrowing caller stack storage into independently scheduled work. `Spawn` and `Blocking` do not require the scheduler object itself to be `Send + Sync`; an implementation may provide a thread-local submission endpoint for `Send` work. I/O and clock capabilities are shared services. Local-only futures and resources need a separate, deliberately designed interface later; they are not silently accepted by this API.

The production dependency is `event-listener`, used only for runtime-independent cancellation notifications. There are no backend feature switches. Protocol libraries and backends can coexist as different concrete types in one program.

## Operation lifecycle, cancellation, and buffers

Cancellable operations construct lazy futures. Constructing an unpolled future submits no operation. An already-cancelled input prevents submission. After submission, cancellation is cooperative and best effort: an already-started blocking call may have to finish, and a completion backend must retire the original operation before releasing its resources. Successful progress or an established operation result wins concurrent cancellation, as specified by the capability.

`Cancellation` has persistent shared state. Child cancellation is isolated from ancestors and siblings; ancestor cancellation propagates to descendants. Dropping a token does not request cancellation. Cancelling is synchronous notification, not an assertion that work has stopped. Waiting for cancellation registers before checking state to preserve racing wakeups, and dropping a wait unregisters only that listener. `defer_cancel` notifies protected work once, then continues polling it so asynchronous draining remains possible.

`IoBuf` and `IoBufMut` are unsafe contracts for stable, owned initialized storage. Their exposed allocation, range, and length stay stable while an operation owns them. Inline storage which moves with the owner cannot implement the traits safely. `BufferSlice` retains the owner and exposes a validated subrange without copying. All normally completed buffer operations return that owner, including errors and cancellation.

Dropping an I/O future forfeits the buffer to backend retirement; it does not establish that kernel or worker access ended. Cancellation and future abandonment do not undo transferred bytes, changed file state, connection attempts, or cursor changes. A protocol must define when continuation is valid, rather than treating cancellation as rollback. Explicit offsets follow the documented platform cursor rules; protocol code requiring portable cursor ordering serializes related operations.

There is no global ordering between concurrent operations. Await related writes, synchronization, shutdown, or close in the order the protocol requires. Buffer reads and writes are single transfers, preserve short counts, and never grow the initialized buffer length. Datagram operations preserve message boundaries and report truncation where the capability supports it.

## Cleanup, tasks, and shutdown

File/socket close, socket shutdown, and file advice intentionally have no cancellation input. These operations remain available while ordinary operation tokens are cancelled. Their completion forms await backend execution and report its result; submission forms await bounded admission only. Accepted cleanup survives future abandonment and must be drained by graceful scheduler shutdown. Close does not cancel or join other operations; callers establish exclusive ownership and completion first. Dropping an unpolled owning close releases the resource using its ordinary destructor.

`Spawn::spawn` registers synchronously before a task becomes runnable. It has no hidden parent or task group. A caller passes cancellation into the future's closure explicitly. A closed scheduler rejects and drops the future without polling it. Task handles own execution lifetime: moving transfers ownership; awaiting returns normal output or `TaskError`; dropping requests destruction once any active poll returns. Destruction runs synchronous Rust destructors and does not execute the remainder of an asynchronous body. Host handles which detach on drop need an owning adapter. No independent forced-abort capability is added.

Graceful shutdown stops admission of new work, lets live owners coordinate cooperative cancellation and await completion, drains accepted cleanup, and retains all outstanding backend-accessed storage until retirement. An abrupt runtime teardown is a separate lifecycle event and must still preserve memory safety. This crate does not prescribe a runtime owner or a universal shutdown method; each backend documents how applications drive and shut it down.

## Clocks and optional services

`Clock::now` is monotonic within one clock domain. Use it rather than `Instant::now` or `Instant::elapsed` in generic consumers so deterministic and paused clocks remain substitutable. Deadlines from independently advancing domains are not interchangeable. Relative sleep starts on its first poll; overflow is an `InvalidInput` operation error. Pre-cancellation wins even for an elapsed deadline. Once waiting, expiry wins if observed together with cancellation. Dropping the wait releases its registration.

`Blocking` is an independent capability with lazy, cancellable, bounded admission. Running closures cannot be forcibly interrupted; they may capture a token to cooperate. Capacity counts queued and running work until execution ends, even after handle abandonment. Blocking job handles own queued work and report completion separately from admission errors. Thread-pool configuration and workers remain backend implementation details.

`Signals` is independently implementable and uses an associated signal identifier and registration guard. Subscription is synchronous and fallible. Notifications run in ordinary thread context, outside raw signal handlers and backend locks. A target can request cancellation or implement application policy. Dropping a registration stops new notifications without synchronously joining an in-progress callback; the backend retains its target until dispatch ends. This crate installs no signal handlers.

Address resolution, typed socket options, message I/O, file allocation/advice, and standard-library TCP resource registration remain separate extension capabilities. Portable consumers require them explicitly. File allocation/advice retain their Linux-style semantic contracts without requiring an implementation on unsupported platforms. There are no system resolver, socket-option syscalls, or signal constants in the interface crate.

## Extraction decisions and follow-up

Concrete task types, native descriptors and metadata wrappers, blocking-pool options, workers, task-local lookup, timers, system resolution, selector code, runtime adapters, and OS feature flags stay in their implementation crates. The optional `Notify` integration for `futures-channel` stays out of the production dependency graph; notification implementations for cancellation, `Arc`, and standard unbounded channels are runtime-independent support.

Tests cover cancellation propagation and wakeups, buffer ownership and invalid ranges, error preservation, default relative-clock behavior, capability-limited consumers, and successive operations using compatible scheduler instances. Test-only implementations establish public API composition; they do not certify any real backend.

## Companion crates

The same workspace now contains `protocol-scheduler-conformance` and `protocol-scheduler-direct`. All public packages share a version, root release notes, and release tag. Keep production interface dependencies independent of both companions. Direct depends on conformance only for development; conformance depends on the interface and runtime-independent test helpers, never a backend.

Conformance supplies async capability-specific assertions and backend-controlled fixtures. Its initial file tests accept resources and paths supplied by the backend, respect partial transfers, and cover ownership, pre-cancellation, laziness, cursor operations, positioned I/O, metadata, truncation, synchronization, and errors. Blocking tests cover lazy/cancelled admission, outputs, panic payloads, queued cancellation, bounded admission, and handle abandonment. The blocking queue fixture holds workers at a known execution boundary and releases them on guard destruction; assertions do not require global FIFO execution. Backends drive their own executors and remain responsible for test deadlines and implementation-specific retirement/shutdown tests. No passing aggregate claims support for capabilities that were not tested.

Direct supplies the core file capability aggregate and `Blocking` on Unix and Windows, without implementing `Spawn`, `Clock`, sockets, address resolution, or signals. Ordinary work uses a bounded pool; cleanup uses a separate worker with its own bounded admission. Counts include both queued and running work. Admission is lazy and cancellation-aware. Accepted file operations retain their resource and owned input until retirement, even after waiter abandonment or scheduler destruction. A worker checks cancellation before executing queued I/O and returns the original input on cancellation. Started calls preserve their actual result. General blocking handles may abandon queued work, whereas accepted I/O and cleanup survive pool closure. Worker panic payloads from file operations are resumed in the waiting future, not disguised as I/O errors.

`DirectFile` owns a standard-library file with private reference retention and no scheduler affinity. Every direct instance accepts the same resource type; successive completed operations can select a different pool. Explicit close requires exclusive ownership, consumes the OS handle exactly once, reports errors without retrying, and uses the cleanup pool. This includes cleanup surviving abandoned completion waiters. Submitted cleanup admission reports only admission errors.

`DirectScheduler::close` stops admission to both pools, discards queued general blocking jobs, and leaves accepted I/O and cleanup to retire. `shutdown().await` additionally waits for all admitted jobs to finish. Applications coordinate cooperative cancellation and asynchronous cleanup before closing admission. Destruction closes admission without synchronously joining workers, so destructors can safely run in worker callbacks. A never-returning synchronous operation can prevent graceful shutdown. Waiting for a scheduler's shutdown from one of its own jobs is a caller-created dependency cycle. Direct uses no process-wide fallback pool or implicit current scheduler.

Regular files are the initial focus. Blocking sockets could exhaust all workers with reads while writes needed for progress remain queued, so they require a deliberate progress and capacity policy before inclusion. Timers must likewise not consume ordinary blocking workers for their entire wait.

Extending conformance to clocks, tasks, sockets, and richer deterministic cancellation/retirement fixtures, then migrating the native and Tokio implementations and adding a smol adapter, remain follow-up work. Keep backend code in companion or adapter crates rather than the interface package.
