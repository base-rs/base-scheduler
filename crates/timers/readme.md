# Core Scheduler Timers

Runtime-independent monotonic timer queues with batched expiry and constant-time cancellation. Capacity can be reserved during setup so registration and cancellation reuse existing storage.

`Timers<Value>` stores values until their deadlines. Dropping a timer handle does not cancel it; use `TimerHandle::cancel` explicitly. Collect expired values and process them outside queue operations.

The `local` feature provides an exclusively owned `local::TimerQueue<Value>` without synchronization or reference counting. It uses the same batching, heap, and reusable slots as `Timers<Value>`. Keys retain their reserved slots until explicitly released, including after expiry; updates and releases use the originating queue. Collect values before invoking callbacks or dropping them outside an outer queue borrow. Native uses this queue for its worker event loops.

The `sharded` feature provides worker-owned queues, assistance views, deadline notifications, and lazy sleep futures. The optional `driver` feature provides a bounded producer inbox. These features create no threads and select no executor.

Extracted from [Socketry](https://github.com/core-rs/socketry-rust/tree/7a70586d5d37fb640d696d0c03a05b2ee3cbe06b/crates/timers), retaining its MIT attribution.
