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

`SocketReceiving` constructs a portable receiver from a scheduler-created buffer pool. Receive and close take the scheduler explicitly. Completed messages expose mutable payloads which can be frozen into retained views. Close awaits retirement without cancellation; drop the receiver afterwards to release its socket owner before exclusive socket close. Explicit multishot kernel control remains a concrete io_uring API accessed through the selector.

## Releasing

This package releases with the workspace. Follow the [workspace release process](../../readme.md#releasing).

## Releases

See the shared [release notes](../../releases.md) for changes awaiting the first release.

## See Also

The [workspace overview](../../readme.md) describes the interface and companion backends.

## Contributing

Please open an issue or pull request on [GitHub](https://github.com/core-rs/core-scheduler).
