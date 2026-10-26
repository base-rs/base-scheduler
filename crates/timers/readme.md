# `base-scheduler-timers`

Runtime-independent monotonic timer queues with batched expiry and constant-time cancellation. Capacity can be reserved during setup so registration and cancellation reuse existing storage.

## Usage

Add this package with `cargo add base-scheduler-timers`.

```rust
use base_scheduler_timers::Timers;
use std::time::Instant;

let mut timers = Timers::with_capacity(16);
let deadline = Instant::now();
let handle = timers.schedule(deadline, "ready");
assert_eq!(timers.pop_expired(deadline), Some("ready"));
assert!(!handle.cancel());
```

`Timers<Value>` stores values until their deadlines. Dropping a timer handle does not cancel it; use `TimerHandle::cancel` explicitly. Collect expired values and process them outside queue operations.

## Queue ownership

The `local` feature provides an exclusively owned `local::TimerQueue<Value>` without synchronization or reference counting. It uses the same batching, heap, and reusable slots as `Timers<Value>`. Keys retain their reserved slots until explicitly released, including after expiry; updates and releases use the originating queue. Collect values before invoking callbacks or dropping them outside an outer queue borrow. Native uses this queue for its worker event loops.

The `sharded` feature provides worker-owned queues, assistance views, deadline notifications, and lazy sleep futures. The optional `driver` feature provides a bounded producer inbox. These features create no threads and select no executor.

Extracted from [Socketry](https://github.com/base-rs/socketry-rust/tree/7a70586d5d37fb640d696d0c03a05b2ee3cbe06b/crates/timers), retaining its MIT attribution.

## Releasing

This package releases with the workspace. Follow the [workspace release process](../../readme.md#releasing).

## Releases

See the shared [release notes](../../releases.md).

## See Also

The [workspace overview](../../readme.md) describes the interface and companion backends.

## Contributing

Please open an issue or pull request on [GitHub](https://github.com/base-rs/base-scheduler).
