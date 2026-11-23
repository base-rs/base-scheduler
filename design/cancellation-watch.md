# Cancellation watches

Proposed 2026-11-23, awaiting the user's review.

## Requirement

A socket operation given a cancellation that is not `Cancellation::never()` costs, in the steady state, what one given `never` does, plus an atomic load and a waker comparison: no lock, no waker clone and no allocation per operation. A connection waits on the same cancellation for every read and write, so its stream reader and writer keep one registration with it across operations, renewed only when the waiting task's waker changes, as Async ties stopping to the task rather than to each operation: `task.stop` raises in the task's fiber wherever it waits, and no I/O call registers anything to make that possible.

### Interface

`base-scheduler` gains `CancellationWatch`, a reusable registration with one cancellation at a time:

```rust
let mut watch = CancellationWatch::new();
// In a poll function, as often as the operation waits:
if watch.poll(cancellation, context).is_ready() {
    return Poll::Ready(Err(Cancelled.into()));
}
```

- `poll(&mut self, cancellation, context)` returns `Ready` once `cancellation` is cancelled. Otherwise it ensures that cancelling it wakes `context`'s waker, and returns `Pending`.
- The watch registers with the first cancellation it is given, and keeps the registration across polls and across operations. A later poll with the same cancellation and a waker that `will_wake` the registered one takes no lock. A different waker replaces the registered one under the cancellation's lock; a different cancellation moves the registration.
- `Cancellation::never()` is never registered: `poll` returns `Pending` at once.
- Dropping the watch removes its registration.
- A registration outlives the operation that made it, so cancelling may wake a task that is no longer waiting on it, which futures permit; cancellation happens once per cancellation, so each registration causes at most one such wake.

`Cancellation::cancelled()` keeps its signature and behaviour, implemented over the same registry as a watch for one wait.

### Backends

Each backend's socket stream reader and writer holds a watch for each direction and uses it in place of `cancellation.cancelled()` on its waiting path: the readiness selector's `read_with` and `write_with`, which today go through `cancellable`, io_uring's stream reader and writer, and Tokio's. Other operations, such as accepts, connects and timers, keep `cancelled()`: they wait once per call rather than once per request, so a registration per wait costs them little. Native's never-cancelled fast paths stay as they are.

### Cost

A `Cancellation`'s node replaces its `event_listener::Event` with a registry of wakers, a slot vector and free list under a `Mutex`, which cancelling takes once to wake every registered waker after setting the cancelled flag. A watch caches its node, its slot and a clone of the registered waker, so it compares wakers without the lock. Registration takes the lock once; checking the flag after registering preserves a racing cancellation, as `cancelled()` does now.

## Rationale

The M6 rerun of base-http's hello benchmark found serving through `Server::run` about 11% slower per request than the M5 baseline's own accept loop, because the loop gave each connection `Cancellation::never()` while `Server::run`, as M5's `serve` did, gives each a child of the server's cancellation (base-http `benchmarks/hello/connection-placement.md`). On hana, the same build with and without that child, profiled under the same load, cost 33.2 and 30.7 user-space samples per thousand requests. With the child, `event_listener`'s list took 12.5% of all samples, registering (4.4% and 3.6% for insertion) and removing (3.6% and 0.9%) a listener under its lock for each wait, and cloning and dropping the task's waker for each registration took 3.7%; none of these appear without it.

Every real server has a shutdown cancellation, and every client driver a lifetime, so this is the cost applications pay, not a benchmark artefact. Making `cancelled()` cheaper would still lock twice per wait; a watch removes the registration from the steady state altogether.

## Scope

- `base-scheduler`: `CancellationWatch`, the registry in `Cancellation`, and `cancelled()` over it.
- The readiness selector, io_uring and Tokio backends' socket stream readers and writers.
- Following, in base-http: the server's `cancellable` and the client's `checked` wait once or more per request on the connection's cancellation, and can hold a watch in the connection's state in the same way.

## Validation

- Unit tests of the watch: same cancellation and waker without the lock, a changed waker, a changed cancellation, `never`, cancellation before and after registering, a cancellation racing registration on another thread, cancellation of an ancestor, and dropping a watch before and after cancellation.
- The conformance suite's cancellation scenarios pass on every backend, with stream reads and writes cancelled while waiting and between operations.
- Miri over the registry and watch.
- The echo benchmark with a real cancellation against `never`, and base-http's hello benchmark through `Server::run` against the build that passes `never`, on hana: the difference within noise.

## Open questions

1. **Its name.** `CancellationWatch` says what it does and sits beside `Cancellation` and `Cancelled`. A shorter `Watch`, or a method `Cancellation::watch()` returning one bound to that cancellation, would lose the reuse across cancellations, which a reader needs because callers pass the cancellation per operation. Recommendation: `CancellationWatch`.
2. **Whether `cancelled()` uses the new registry or keeps `event_listener`.** One registry per cancellation is simpler, and cancelling then wakes one list rather than two. `event_listener` stays a dependency either way, for the buffer pool and the backends. Recommendation: one registry.
3. **Release.** The change is additive, so 0.6.4. Recommendation: 0.6.4, with base-http's follow-up after it.
