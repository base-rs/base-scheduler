# `base-scheduler-selector`

I/O capabilities independent of an executor. `ReadinessSelector` supplies independently driven or worker-owned socket readiness, file and resolver operations through bounded native workers, and platform-supported signals. It can be polled from any executor while its services remain alive.

## Usage

Add this package with `cargo add base-scheduler base-scheduler-selector`.

```rust
use base_scheduler::{Cancellation, SocketClose, SocketDomain, SocketOpen, SocketType};
use base_scheduler_selector::ReadinessSelector;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let selector = ReadinessSelector::new()?;
let socket = selector.socket_open(
    SocketDomain::Ipv4, SocketType::Stream, None, &Cancellation::never(),
).await?;
selector.socket_close(socket).await?;
selector.shutdown().await;
# Ok(()) }
```

## Ownership and driving

`Selector` describes admission and asynchronous retirement. Individual operations use the existing `base-scheduler` capability traits. Concrete selectors document how their event driver is run; the lifecycle interface does not create threads or require a universal set of operations.

Call `shutdown().await` after coordinating application cancellation and cleanup. Dropping a selector closes admission without synchronously waiting; backend workers retain accepted I/O until access ends.

`SelectorDriver` and `SelectorWorker` support executor-owned event loops. A movable driver enters one worker, consuming itself into a possibly thread-bound owner. That owner provides interruptible waits and bounded completion batches. Completion processing wakes tasks; the executor polls its ready queue only after the driver returns. `ReadinessSelector::new` and `with_options` retain independent `async-io` progress. `with_workers(options, workers)` returns the selector and one `ReadinessDriver` per worker, suitable for `NativeScheduler::with_drivers`. On macOS these drivers own kqueues directly, using persistent `EV_CLEAR` filters and `EVFILT_USER` interruption. Other edge-capable platforms use `polling`; platforms without edge support retain independent progress and return retired drivers. Windows compiles only this retired-driver facade, without allocating worker-registration state or an unused poller. Kqueue writes into one reusable native-event vector; dispatch interprets its initialized entries directly, without a normalized event buffer or a separate conversion pass. Clearing the batch still precedes registration reclamation.

Worker-driven sockets register lazily after `WouldBlock`. Routing uses the explicitly selected selector's current worker, or round-robin for root and external callers. Each socket keeps its first owner within that selector group even when a future moves. Independent listeners support concurrent waits in either direction. Event generations prevent a racing `WouldBlock` from clearing a newer notification. The final socket alias deregisters synchronously before releasing its descriptor, including on another thread. Driver closure wakes pending operations; it never retains application payload pointers.

For registered sockets, the generation is observed before the syscall, so `WouldBlock` leads directly to a wait rather than a redundant immediate retry. Initial registration still retries to cover the registration boundary. Worker-owned direction waits already receive closure notifications and do not register a second shutdown listener; independently driven I/O retains its separate shutdown wait.

On macOS and Linux/Android epoll, an event token directly addresses driver-owned registration state. Kernel filters are deleted before successfully retired records are released at a subsequent batch boundary. Failed additions/deletions remain quarantined until the driver stops; group closure alone never releases a live batch. Registration state retains no native socket, so event delivery cannot delay remote exclusive close. Other polling backends retain numeric-key routing.

Worker readiness registers each wait and its waker in one step. Notification detaches all waiters before waking them, allowing notified guards to retire without another lock. Cancellation removes only its own waiter; monotonic IDs protect recycled slots from older guards. Waker callbacks run outside the notifier lock, and lazy registration rechecks readiness and closure to preserve racing notifications.

Bound scalar stream readers retain their worker registration and, after establishing stream progress, wait before another receive when readiness is clear. On macOS, a short successful receive clears only its observed generation and only after consuming at least the byte count reported by kqueue. A short receive at an urgent-data boundary can leave more bytes queued, so it retains readiness when it falls short of that count. Readable EOF is preserved through the final data read. Other readiness backends retain readiness until `WouldBlock` because their event wrappers do not expose the same byte-count and directional EOF information. Message receives keep their syscall-first contract. Borrowed scalar reads still allocate no receive pool.

Bound borrowed stream writes attempt the syscall before looking up a worker registration. Successful writes return before constructing wait futures and need no table lookup. On backpressure, the writer retains the shared registration and retries across lazy registration; later retries snapshot the notification generation before sending. Cancellation and scheduler closure retain their existing behavior. Owned submissions keep their independent progress and retirement contracts.

The readiness selector supplies bound stream, packet, and file readers and writers. Ready stream submissions complete inline; stalled owned output progresses through the existing bounded worker service, with cancellation and retirement independent of the caller polling its completion. Operations take cancellation without another scheduler argument. Borrowed-only readers allocate no payload pool; owned reads use lazy default storage or an explicit shared pool. Close and drop handles before exclusively closing their underlying resources. See the [I/O usage guide](../../context/io.md).

## Releasing

This package releases with the workspace. Follow the [workspace release process](../../readme.md#releasing).

`ProcessWait` uses pidfd readiness on Linux and `EVFILT_PROC` on macOS/FreeBSD for individual exit waits. Group/state-change waits and unsupported platforms use the existing bounded worker pool. A started blocking fallback can still reap after cancellation and delay shutdown until the child changes state. See the [process-wait contract](../../context/interface.md#child-process-waits) for ownership and cancellation semantics.

## Releases

See the shared [release notes](../../releases.md).

## See Also

The [workspace overview](../../readme.md) describes the interface and companion backends.

## Contributing

Please open an issue or pull request on [GitHub](https://github.com/base-rs/base-scheduler).
