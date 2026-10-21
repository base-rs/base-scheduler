# `core-scheduler-native`

A work-stealing executor composed with native I/O selectors for Core's explicit scheduler capabilities. Applications select this implementation; libraries depend only on `core-scheduler`.

Tasks run on worker-local queues with remote inboxes and work stealing. The default `ReadinessSelector` supplies I/O independently of those workers. Nonblocking sockets use `async-io` readiness. Files, address resolution, and blocking closures use a separate bounded pool; accepted cleanup has an independent pool. Monotonic sleeps use reserved timer storage from `core-scheduler-timers`.

## Usage

The workspace is not published yet. Add this package from Git with `cargo add core-scheduler core-scheduler-native --git https://github.com/core-rs/core-scheduler`.

```rust
use core_scheduler::{Cancellation, Clock, Spawn};
use core_scheduler_native::NativeScheduler;
use std::time::Duration;

let scheduler = NativeScheduler::new()?;
scheduler.block_on(async {
    let task = scheduler.spawn(async { 42 }).unwrap();
    assert_eq!(task.await.unwrap(), 42);
    scheduler.sleep(Duration::from_millis(1), &Cancellation::never()).await.unwrap();
    scheduler.shutdown().await;
});
# Ok::<(), std::io::Error>(())
```

## Ownership and shutdown

Operation futures receive the scheduler and cancellation explicitly. Task handles own execution: dropping a handle requests destruction after any active poll returns. Close stops admission and wakes socket and timer waits, while preserving live task owners. Coordinate cancellation and await tasks before `shutdown().await`, which drains accepted work. Dropping the scheduler closes it without blocking the dropping thread. A spawned task or blocking job must not await shutdown of its own scheduler. A task, syscall, or closure that never returns can prevent graceful shutdown.

Sockets have no scheduler-instance affinity. Finish outstanding operations before exclusive close. TCP resources imported from the standard library must remain nonblocking through any external aliases.

`NativeOptions` configures executor workers, blocking workers, admitted blocking jobs, and timer slots per worker. A full owning timer queue returns `WouldBlock`, even if another worker has capacity; a closed scheduler returns `BrokenPipe` for pending clock and socket operations.

## Worker timers

Each worker owns an unsynchronized timer queue and collects expiries in its event loop. The earliest deadline bounds its interruptible I/O wait, or its thread park when there is no active I/O driver. There is no timer thread. Expired timers wake tasks through their registered wakers, placing runnable tasks into the ordinary queues where they can be stolen. Native never polls application futures during timer collection.

A sleep first polled on a worker registers directly with that worker. Root futures and other executors choose a worker by round robin and send it a registration command. A pending sleep retains its original owner when its future moves; waker changes, cancellation, and drop update that owner's queue. A subsequent sleep can choose the task's new worker. Shutdown wakes pending sleepers and waits for worker timer callbacks and driver retirement.

The executor and readiness design come from [Socketry](https://github.com/core-rs/socketry-rust/tree/7a70586d5d37fb640d696d0c03a05b2ee3cbe06b/crates/executor). This adapter follows Core's current ownership, cancellation, and asynchronous shutdown contracts. It does not include Socketry's io_uring backend.

## Selector composition

`NativeScheduler<S>` forwards the capabilities implemented by its selector, preserving the selector's resource types. Select I/O limits separately from executor configuration:

```rust
use core_scheduler_native::{NativeExecutorOptions, NativeScheduler};
use core_scheduler_selector::{ReadinessSelector, SelectorOptions};

let selector = ReadinessSelector::with_options(SelectorOptions {
    threads: 2,
    capacity: 128,
})?;
let scheduler = NativeScheduler::with_selector(selector, NativeExecutorOptions {
    workers: 2,
    timer_capacity: 128,
})?;
scheduler.block_on(scheduler.shutdown());
# Ok::<(), std::io::Error>(())
```

Selectors may supply their own progress mechanism. The readiness selector is independently driven by `async-io`. `with_drivers(selector, drivers, options)` instead assigns one driver to each task worker, with no separate ring thread. Each `SelectorDriver` enters its worker and supplies `SelectorWorker` event waits, interruptible wakeups, and bounded completion processing. Native polls ready tasks only after a complete driver turn publishes its completions. Workers continue retiring accepted I/O after tasks are abandoned, and can keep executing tasks after a driver has retired. See the [io_uring selector](../iouring/readme.md) for per-worker ring construction and routing.

Native forwards the optional `SocketRecvMsgMultishot` capability when its selector implements it. The returned resource accepts the Native scheduler explicitly for `recv` and `close`, forwarding to that scheduler's selector without retaining a scheduler instance.

## Releasing

This package releases with the workspace. Follow the [workspace release process](../../readme.md#releasing).

## Releases

See the shared [release notes](../../releases.md) for changes awaiting the first release.

## See Also

The [workspace overview](../../readme.md) describes the interface and companion backends.

## Contributing

Please open an issue or pull request on [GitHub](https://github.com/core-rs/core-scheduler).
