# Core Scheduler io_uring

A Linux selector with independently driven completion I/O. Owned file reads and writes (including positioned operations), socket reads/writes, and socket message sends/receives use io_uring. Borrowed `_poll` operations try nonblocking syscalls directly and use ring readiness waits after `WouldBlock`. Other file operations, socket lifecycle, resolution, blocking work, and signals use an explicitly owned `ReadinessSelector` fallback.

`IoUringSelector::new` returns the selector and an `IoUringDriver`. It starts no ring thread. An application can call `driver.turn(timeout)` from its event loop or move the driver into a dedicated thread and call `run`. Operation futures use ordinary wakers and can be polled on any executor.

A separately driven selector composes with `NativeScheduler::with_selector`. The driver must remain active throughout operation and asynchronous shutdown. Dropping a pending operation requests cancellation while the driver retains its buffer and file until original completion. Cancellation completion alone never releases original request storage.

Admission bounds queued plus in-flight original requests. This initial extraction uses one boxed request and completion channel per ring request; it does not yet pool request records. Setup probes required kernel support and reports unavailable rings explicitly. Unexpected driver failure retains memory whose kernel access cannot be proven complete and causes affected operation futures to panic when polled, because their original buffers cannot safely be returned. Queued requests that never reached the kernel return errors and their buffers. Graceful shutdown drains instead.

Socket transfers submit `Recv`, `Send`, `RecvMsg`, and `SendMsg`. An `EAGAIN` completion retires that attempt before a readiness wait and retry on the original ring. Message requests retain their header, address, iovec array, and complete sequence owner until original completion. Prepared `IoBuffers` packages supply their existing vectors directly; ordinary sequences use request-owned vectors.

Borrowed recv/send and message `_poll` calls use `MSG_DONTWAIT` through synchronous `NativeSocket::try_*` methods. Ready calls allocate no request, acquire no ring capacity, and continue in the same task poll. After `WouldBlock`, a one-shot `PollAdd` on the selected worker ring waits for readiness, then the task retries the syscall. Readiness is advisory. A retry keeps the original wait's ring even after task migration. `socket_wait` also uses `PollAdd`. These waits retain only the socket, never borrowed bytes or vector pointers. Cancellation waits for retirement before returning; dropping a future requests cancellation, while forgetting it can retain a readiness registration until an event or selector shutdown. Finish or drain outstanding waits before exclusive socket close.

The completion engine is adapted from Socketry revision `7a70586d5d37fb640d696d0c03a05b2ee3cbe06b`, preserving its MIT attribution.

`IoUringSelector::with_workers(options, workers)` creates one ring per worker, with one shared readiness fallback. `NativeScheduler::with_drivers` owns and drives those rings directly on its task workers:

```rust
# #[cfg(target_os = "linux")]
# {
use core_scheduler_iouring::{IoUringOptions, IoUringSelector};
use core_scheduler_native::{NativeExecutorOptions, NativeScheduler};

let workers = 2;
let (selector, drivers) = IoUringSelector::with_workers(IoUringOptions::default(), workers)?;
let scheduler = NativeScheduler::with_drivers(selector, drivers, NativeExecutorOptions {
    workers,
    timer_capacity: 128,
})?;
scheduler.block_on(scheduler.shutdown());
# }
# Ok::<(), std::io::Error>(())
```

Each ring has its own request capacity, operation identifiers, pending queue, and in-flight storage. Local submissions bypass the synchronized inbox and notification pipe. A private thread-local lookup selects only the current worker belonging to the explicitly supplied selector; it does not select an implicit scheduler. Root futures and other external callers use round-robin routing through per-ring inboxes. Submitted operations retain their original ring through completion and cancellation even when their tasks migrate.

An idle worker submits a one-shot ring readiness request for its notification eventfd and sleeps directly in `io_uring_enter`. The request contains no userspace buffer pointer and remains registered until its completion is observed. External wakes still write only to the notification eventfd, preserving single-owner access to SQ and CQ. On kernels without extended enter timeouts, bounded waits use `ppoll` on the ring and notification eventfd. Neither path uses separate epoll registrations, rearming, or timerfd. Notifications and completions racing the transition to sleep remain observable.

Completion processing publishes results and wakes tasks without polling application futures. The owning worker publishes the entire available CQ batch before polling its ready queue; other workers may steal published tasks immediately. Bounded command processing and periodic driver turns prevent either commands or runnable tasks from starving I/O progress. Cross-queue cancellation preserves submission order after migration.

Drivers implement `SelectorDriver`: entering consumes the movable driver and returns a thread-bound `IoUringWorker`, implementing `SelectorWorker`. This keeps local routing and queue storage on the owning thread. Native workers keep driving accepted I/O after task abandonment. Stopping or failing any ring closes admission for the entire selector group, including its shared fallback; surviving drivers continue retiring their accepted work. Applications coordinate cancellation and await their tasks before shutting down the scheduler.

Request records and completion channels still allocate per ring request, and external notifications currently increment each ring's interrupt counter. These are implementation costs rather than interface requirements. The native task registry and executor counters also remain shared; per-worker rings remove shared I/O admission and submission contention, rather than every source of executor contention.
