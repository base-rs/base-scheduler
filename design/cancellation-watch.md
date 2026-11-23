# Watched cancellations

Accepted 2026-11-23, implemented in `base-scheduler` 0.6.4, adopted by base-http, and validated on 2026-11-24. The user chose to declare that a task's waits share a cancellation, rather than have each reader infer it, and the shape measured below as B2, with a release as 0.6.4.

## Requirement

A task that waits on one cancellation many times, such as a connection whose every read, write and handler wait shares its server's shutdown, declares it once, and then each wait costs what a wait on `Cancellation::never()` does, plus an atomic load and a waker comparison: no lock, no waker clone and no allocation per wait. As Async ties stopping to the task rather than to each operation, so that `task.stop` reaches the fiber wherever it waits, a watched cancellation ties its registration to the task rather than to each wait.

### Interface

```rust
let cancellation = cancellation.watch();       // once, in the task that waits
reader.read_into(&mut buffer, &cancellation).await?;
writer.write_all(&response, &cancellation).await?;
```

- `Cancellation::watch(&self) -> Cancellation` returns a cancellation with the same signal and one registration, the watch, that every wait on it shares. Operations keep taking `&Cancellation`: a watched cancellation is an ordinary `Cancellation`, told apart at run time by an optional field, so no operation, trait or backend gains a parameter, and nothing is compiled twice.
- A watched cancellation belongs to one task. Its watch registers the first waiting task's waker, the owner's, and keeps it across waits. A wait whose waker has the owner's data and vtable addresses, as `will_wake` compares them, reads them from a set-once cell without a lock. A wait with a different waker, from another task, is added to the watch's other wakers under its lock, once each, so it stays correct at about today's cost and leaves the owner's registration in place; its first wait may allocate the slot.
- Cloning a watched cancellation, or taking its `child`, gives a plain cancellation sharing the signal, so a watch is never shared by accident. `Cancellation::never().watch()` is `never`.
- `cancelled()` on a watched cancellation waits through the watch, so existing code such as `pin!(cancellation.cancelled())` becomes cheaper without change.
- `Cancellation::poll_watched(&self, context) -> Option<Poll<()>>` lets a library's own waits take the same path without a future: `Some(Poll::Pending)` for `never`, the watch's result for a watched cancellation, and `None` for a plain one, whose waits keep a registration in their own state through `cancelled()`.
- The watch is removed when the watched cancellation is dropped, or once it is cancelled, since cancelling wakes and discards every registration.

A `Cancellation` grows from one pointer to two.

### Registration

Plain waits keep `event_listener`'s listener, whose storage is already held in the wait's future, so they allocate nothing; replacing it with another registry saved nothing in the prototypes measured below. A watch registers with its signal's node once, when `watch()` is called, in slots the node reuses as watches are dropped. Cancelling sets the flag, then takes each watch's wakers under its lock and wakes them. A wait registers before it checks the flag, preserving a racing cancellation: either cancelling finds its waker, or it observes the flag.

### Compact paths

Where a backend or library gives `never` a compact path and measurement shows the general path costs a watched wait, a watched cancellation takes the compact path too, checking the watch only when the operation would wait:

- the readiness selector's stream reader, whose compact receive future a watched read awaits directly, and its scheduler's `cancellable`, through which its writer and its reader's first read wait;
- in base-http, the server's `cancellable`, the client's `checked` and request lending, through `poll_watched`.

Each is one more case beside `never` in a place that already special-cases it, as `base-project-performance` asks of fast paths. io_uring's stream writer polls its cancellation with a waker of its own rather than the task's, so a task's watch cannot serve it, and Tokio's waits go through `cancelled()`, which a watched cancellation already makes cheap; neither needs a compact path unless a measurement shows otherwise.

## Rationale

### The cost

The M6 rerun of base-http's hello benchmark found serving through `Server::run` about 11% slower per request than the M5 baseline's own accept loop, because the loop gave every connection `Cancellation::never()` while `Server::run`, as M5's `serve` did, gives each a child of the server's cancellation (base-http `benchmarks/hello/connection-placement.md`). On hana, the same build with and without that child, profiled under the same load, took 33.2 and 30.7 user-space samples per thousand requests. With the child, registering and removing an `event_listener` listener under its lock for each wait took 12.5% of all samples, and cloning and dropping the task's waker for each registration 3.7%. Every real server has a shutdown cancellation, and every client driver a lifetime, so this is the cost applications pay.

### The options measured

Prototypes on hana, each a build of base-http's hello benchmark serving `base-native` with `hyper-tokio` as a control, compared with `main` over 10 rounds, the server on CPU 3 and the client on CPUs 1 and 2:

| Prototype | What it changes | Wall time | User CPU |
| --- | --- | ---: | ---: |
| Registry | The waker registry in place of `event_listener`, a registration per wait | +0.8% | −1.5% |
| A | A `CancellationWatch` that the readiness reader and writer keep, and the server's connection loop another | −1.6% | −4.0% |
| B | `Cancellation::watch()`, called once per connection | −2.6% | −9.6% |
| B2 | B, with watched cancellations taking the reader's compact path | −4.8% | −10.2% |
| `never` | Connections given `Cancellation::never()` | −6.8% to −7.6% | −13.6% to −17.1% |

B and B2 come from a second campaign, with B at −3.3% and −8.4% there; `never`'s two figures are from each campaign. The Hyper control, unchanged in every build, moved by up to 5% between builds, so differences of a few percent are at this host's noise. Profiled under the same load, B2 and `never` took 28.5 user-space samples per thousand requests each; the one difference was more 144-byte copies in B2, 1.4 points of the samples in `memmove`, from a wrapper future that the implementation removes.

- **The lock was not the cost**: the registry alone, still registering per wait, saved nothing. Registering and removing a registration for every wait, and the waker clones it takes, was.
- **B over A**: a watched cancellation covers every wait in the task, the socket's and the library's alike, with one registration, and needs one call per connection; A covered only the waits that were given their own watch, and changed every reader, writer and library loop.
- **B2 over B**: the general path's wrapper futures cost what remained; the compact path removes them.

### The implementation measured

Two screening campaigns of 10 rounds on hana compared base-http's hello benchmark built with `base-scheduler` 0.6.3, `main`, and with this implementation, `never-new` and `watched`, against the 0.6.3 build that passes `never`. Watched connections came to −4.6% wall time and −10.0% user CPU against `main`, as the prototype had, and within noise of `never`: +1.3% in wall time against it, and +1.4% against `never-new`, where the Hyper control, unchanged in every build, read +1.4% and +1.7%. A first version shared one compact path between `never` and watched reads in the readiness reader; it put `never-new` 1.3% behind `never` in wall time, worse in 8 of 10 rounds while the control was 0.7% ahead, so a never-cancelled read awaits its receive alone again, which measured −1.3% against `never`.

### Declared rather than inferred

The user preferred declaring that a task's waits share a cancellation to having each reader infer it from repeated arguments: the relationship belongs to the task, and one declaration serves every operation and library it uses. A trait making operations generic over plain and watched cancellations would declare it in the types, but compile every operation twice and change every backend's traits; the runtime field declares it at no cost to compile time and no change to signatures, as base-project's conventions prefer.

## Scope

- `base-scheduler`: `Cancellation::watch`, `is_watched`, `poll_watched`, the watches' registration, and `cancelled()` through a watch.
- The readiness selector's stream reader and its `cancellable`.
- base-http: the server watches each connection's cancellation, the client driver each connection's lifetime, and `cancellable`, `checked` and request lending take the watched path, after `base-scheduler` 0.6.4.

## Validation

- Unit tests of the registry and watch: a repeated wait takes no lock, a second task's wait registers itself and both are woken, cancellation before and after registering, a cancellation racing registration on another thread, an ancestor's cancellation, clones and children are plain, `never().watch()`, and dropping a watched cancellation before and after cancellation.
- Allocation tests: waits on plain cancellations, and the owner's waits on a watched one, do not allocate.
- The conformance suite's stream scenarios on every backend with watched cancellations, cancelled while waiting and between operations.
- Miri over the registry, the watch and their races.
- A campaign of 20 rounds on hana of base-http's hello benchmark through `Server::run` watching each connection, against the same build passing `never` with a layout replica of it, and the echo benchmark with a watched cancellation against `never`: the difference within noise. Done on 2026-11-24: watching came to +1.0% in wall time against `never`, while the unchanged Hyper control moved +2.9%, and 6.4% faster and 14.9% lower in user CPU per request than base-http before it, in 19 and 20 of 20 rounds (base-http's `benchmarks/hello/connection-placement.md`); in the [echo benchmark](../benchmarks/echo/watched-cancellation.md), a watched cancellation was within noise of `never` at one connection and 32, and a plain one cost about 10% more user CPU per request.
