# Core Scheduler io_uring

A Linux selector with independently driven completion I/O. File reads and writes, including positioned operations, use io_uring. Other file operations, sockets, resolution, blocking work, and signals use an explicitly owned `ReadinessSelector` fallback.

`IoUringSelector::new` returns the selector and an `IoUringDriver`. It starts no ring thread. An application can call `driver.turn(timeout)` from its event loop or move the driver into a dedicated thread and call `run`. Operation futures use ordinary wakers and can be polled on any executor.

The selector composes with `NativeScheduler::with_selector`. The driver must remain active throughout operation and asynchronous shutdown. Dropping a pending operation requests cancellation while the driver retains its buffer and file until original completion. Cancellation completion alone never releases original request storage.

Admission bounds queued plus in-flight original requests. This initial extraction uses one boxed request and completion channel per operation; it does not yet pool request records. Setup probes required kernel support and reports unavailable rings explicitly. Unexpected driver failure retains memory whose kernel access cannot be proven complete and reports failure to operation waiters; graceful shutdown drains instead.

The completion engine is adapted from Socketry revision `7a70586d5d37fb640d696d0c03a05b2ee3cbe06b`, preserving its MIT attribution.
