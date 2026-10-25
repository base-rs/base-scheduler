# `core-scheduler-selector`

I/O capabilities independent of an executor. `ReadinessSelector` supplies socket operations through `async-io`, file and resolver operations through bounded native workers, and platform-supported signals. It can be polled from any executor while its services remain alive.

## Usage

The workspace is not published yet. Add this package from Git with `cargo add core-scheduler core-scheduler-selector --git https://github.com/core-rs/core-scheduler`.

```rust
use core_scheduler::{Cancellation, SocketClose, SocketDomain, SocketOpen, SocketType};
use core_scheduler_selector::ReadinessSelector;

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

`Selector` describes admission and asynchronous retirement. Individual operations use the existing `core-scheduler` capability traits. Concrete selectors document how their event driver is run; the lifecycle interface does not create threads or require a universal set of operations.

Call `shutdown().await` after coordinating application cancellation and cleanup. Dropping a selector closes admission without synchronously waiting; backend workers retain accepted I/O until access ends.

`SelectorDriver` and `SelectorWorker` support executor-owned event loops. A movable driver enters one worker, consuming itself into a possibly thread-bound owner. That owner provides interruptible waits and bounded completion batches. Completion processing wakes tasks; the executor polls its ready queue only after the driver returns. Selectors with independent progress, such as `ReadinessSelector`, do not need to implement this optional driving contract.

The readiness selector supplies bound stream, packet, and file readers and writers. Ready stream submissions complete inline; stalled owned output progresses through the existing bounded worker service, with cancellation and retirement independent of the caller polling its completion. Operations take cancellation without another scheduler argument. Borrowed-only readers allocate no payload pool; owned reads use lazy default storage or an explicit shared pool. Close and drop handles before exclusively closing their underlying resources. See the [I/O usage guide](../../context/io.md).

## Releasing

This package releases with the workspace. Follow the [workspace release process](../../readme.md#releasing).

`ProcessWait` uses pidfd readiness on Linux and `EVFILT_PROC` on macOS/FreeBSD for individual exit waits. Group/state-change waits and unsupported platforms use the existing bounded worker pool. A started blocking fallback can still reap after cancellation and delay shutdown until the child changes state. See the [process-wait contract](../../context/interface.md#child-process-waits) for ownership and cancellation semantics.

## Releases

See the shared [release notes](../../releases.md) for changes awaiting the first release.

## See Also

The [workspace overview](../../readme.md) describes the interface and companion backends.

## Contributing

Please open an issue or pull request on [GitHub](https://github.com/core-rs/core-scheduler).
