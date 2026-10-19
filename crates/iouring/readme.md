# Core Scheduler io_uring

A Linux selector with independently driven completion I/O. File reads and writes, including positioned operations, use io_uring. Other file operations, sockets, resolution, blocking work, and signals use an explicitly owned `ReadinessSelector` fallback.

`IoUringSelector::new` returns the selector and an `IoUringDriver`. It starts no ring thread. An application can call `driver.turn(timeout)` from its event loop or move the driver into a dedicated thread and call `run`. Operation futures use ordinary wakers and can be polled on any executor.

The selector composes with `NativeScheduler::with_selector`. The driver must remain active throughout operation and asynchronous shutdown. Dropping a pending operation requests cancellation while the driver retains its buffer and file until original completion. Cancellation completion alone never releases original request storage.

Admission bounds queued plus in-flight original requests. This initial extraction uses one boxed request and completion channel per operation; it does not yet pool request records. Setup probes required kernel support and reports unavailable rings explicitly. Unexpected driver failure retains memory whose kernel access cannot be proven complete and causes affected operation futures to panic when polled, because their original buffers cannot safely be returned. Queued requests that never reached the kernel return errors and their buffers. Graceful shutdown drains instead.

The completion engine is adapted from Socketry revision `7a70586d5d37fb640d696d0c03a05b2ee3cbe06b`, preserving its MIT attribution.

The intended native integration gives each worker its own ring and local request storage. Local submissions should bypass shared channels and admission locks; remote submissions and cancellation use a per-worker inbox. An outstanding operation stays with its original ring if its task migrates. This prototype still uses a shared submission channel and admission lock to establish the driving and ownership contracts; it does not implement that local fast path yet.
