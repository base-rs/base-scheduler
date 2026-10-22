# `core-scheduler-iouring`

A Linux selector with independently driven completion I/O. Owned file reads and writes (including positioned operations), socket reads/writes, and socket message sends/receives use io_uring. Borrowed `_poll` operations try nonblocking syscalls directly and use ring readiness waits after `WouldBlock`. Other file operations, socket lifecycle, resolution, blocking work, and signals use an explicitly owned `ReadinessSelector` fallback.

## Usage

The workspace is not published yet. Add this package from Git with `cargo add core-scheduler-iouring core-scheduler-native --git https://github.com/core-rs/core-scheduler`.

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

## Driver ownership

`IoUringSelector::new` returns the selector and an `IoUringDriver`. It starts no ring thread. An application can call `driver.turn(timeout)` from its event loop or move the driver into a dedicated thread and call `run`. Operation futures use ordinary wakers and can be polled on any executor. Idle turns wait directly on the ring; cross-thread wakeups signal an eventfd watched by a single outstanding `PollAdd`. The poll retains only the descriptor, with no userspace destination buffer to keep alive. Setup requires `IORING_FEAT_EXT_ARG` for timed turns, available since Linux 5.11.

A separately driven selector composes with `NativeScheduler::with_selector`. The driver must remain active throughout operation and asynchronous shutdown. Dropping a pending operation requests cancellation while the driver retains its buffer and file until original completion. Cancellation completion alone never releases original request storage.

## Owned I/O

Admission bounds queued plus in-flight original ring requests. Synchronous socket attempts do not consume ring capacity. Ordinary operations use one boxed request and completion channel per ring request; file monitors reuse those allocations. Setup probes required kernel support and reports unavailable rings explicitly. Unexpected driver failure retains memory whose kernel access cannot be proven complete and causes affected operation futures to panic when polled, because their original buffers cannot safely be returned. Queued requests that never reached the kernel return errors and their buffers. Graceful shutdown drains instead.

Owned scalar socket writes first attempt a synchronous nonblocking send after checking cancellation and admission closure. Ready writes return their original buffer without allocating a ring request; `WouldBlock` and `Interrupted` fall through to completion I/O. Scalar reads submit directly to the ring. Ring socket transfers use `Recv`, `Send`, `RecvMsg`, and `SendMsg`. An `EAGAIN` completion retires that attempt before a readiness wait and retry on the original ring. Message requests retain their header, address, iovec array, and complete sequence owner until original completion. Prepared `IoBuffers` packages supply their existing vectors directly; ordinary sequences use request-owned vectors.

## Reusable file state

`FileMonitoring` retains typed request and completion storage for one sequential stream of owned reads or writes. Reusing the buffer-owner type avoids fresh request/channel allocations after warmup. Independent monitors can share a file for concurrent operations; use positioned operations for independent offsets.

Each monitor also caches an optional fixed-file registration for its current ring. `IoUringOptions::fixed_files` limits each ring's sparse table and defaults to 256; zero disables registration. Unsupported setup, insertion failure, or exhaustion falls back to ordinary descriptors. A future moving threads keeps its accepted operation on the original ring. After retirement, a later operation can use another worker, releasing the previous registration through its original ring and creating a new registration on the destination. Returning to a previous worker registers again. Pending and failed releases remain tracked for explicit close, so old registrations can temporarily coexist with the current one. This cache does not implement registered buffers.

Call `file_monitor_close` while the drivers remain active, then drop the monitor before exclusively closing the file. Close retires accepted I/O and waits for every cached ring registration to be released. Cancelling close stops waiting while release continues; another close observes its result. Drop queues release without waiting. See the [ring-state research](../../benchmarks/files/ring-state-research.md) for measured benefits, costs, and further opportunities.

## Borrowed I/O

Borrowed recv/send and message `_poll` calls use `MSG_DONTWAIT` through synchronous `NativeSocket::try_*` methods. Ready calls allocate no request, acquire no ring capacity, and continue in the same task poll. After `WouldBlock`, a one-shot `PollAdd` on the selected worker ring waits for readiness, then the task retries the syscall. Readiness is advisory. A retry keeps the original wait's ring even after task migration. `socket_wait` also uses `PollAdd`. These waits retain only the socket, never borrowed bytes or vector pointers. Cancellation waits for retirement before returning; dropping a future requests cancellation, while forgetting it can retain a readiness registration until an event or selector shutdown. Finish or drain outstanding waits before exclusive socket close.

## Receiving message sequences

`buffer_pool(options)` creates one bounded payload budget shared across connections and workers. `socket_receiver(socket, pool, flags, cancellation)` uses that budget for multishot receives, with one provided-buffer ring per worker that uses the pool. Workers publish exclusive leases from the shared pool; they do not allocate another payload budget. Completed `IoUringBuffer` leases are writable for in-place parsing or decryption and can be frozen into retained immutable views. Their last owner returns storage directly to the shared pool on any thread. Requested usable capacity and alignment exclude the additional receive metadata and ring storage.

This experimental shared receiver requires Linux 6.0 or later with provided-buffer ring and multishot receive support. Unsupported registration or receive operations return errors; this prototype does not automatically fall back. Applications can explicitly construct the portable `SocketReceiver` monitor composition instead. The [worker-pool measurements](../../benchmarks/buffers/worker-pools.md) show multi-connection regressions, so this path is not yet a demonstrated faster default.

Idle receivers initially wait for readability before asking for storage. Under pressure, a worker can reclaim another worker's unused published buffers by cancelling and draining that group's receives before unregistering its ring. Returned application leases are independent of this process and remain valid across redistribution and shutdown. Retaining every buffer pauses progress rather than growing capacity. Cancelling an individual receive wait leaves this shared receiver usable. Explicit `close(scheduler).await` retires the resource; drop requests cancellation without waiting. A receiver holds one admission slot for its lifetime, so leave room for sends and other operations. Pools and receivers must be used with the selector that created them.

`socket_recvmsg_multishot` starts one persistent receive using a bounded provided-buffer ring. This implements the portable `SocketRecvMsgMultishot` capability, also forwarded by Native; import `SocketRecvMsgMultishot` and `SocketRecvMsgReceiver` from `core_scheduler`. A receiver's `recv(scheduler, cancellation)` returns a message with `SocketRecvResult` metadata and an immutable `RecvMsgBuffer` lease. The lease exposes the payload through `AsRef<[u8]>` and implements `IoBuf`, so it can also be transferred directly into an owned write without copying. Dropping the lease returns its buffer to the original ring. Local returns publish directly; remote returns use that ring's command inbox.

The kernel rearms a multishot request across receives. Only a CQE without `IORING_CQE_F_MORE` retires that submission. Exhaustion pauses the receiver instead of spinning: returning a buffer resubmits the request. TCP EOF ends reception; empty datagrams remain messages, and truncation and sender metadata are preserved. Peeking is rejected. Linux 6.0 or later and provided-buffer ring support are required; setup or operation errors are explicit, without a fallback.

Each receiver retains one admission slot until EOF or close, even while its buffer pool is exhausted. Reserve additional capacity for sends and other operations which can wait for the ring. `RecvMsgMultishotOptions` defaults to eight 64 KiB payload buffers per receiver, with additional metadata storage. This is setup-time allocation; delivery reuses the request and reserved result queue. Call `close(scheduler).await` before exclusive socket close to cancel and await the final original completion. Drop requests cancellation without waiting. Returned leases remain valid after receiver or scheduler shutdown. Dropping or forgetting an individual `recv` future cannot expose its caller's memory to the kernel. A token already cancelled before `recv` leaves the receiver unchanged; cancellation of a pending wait closes the receiver before returning. Queued successful receives win concurrent cancellation.

Unexpected driver failure retains unknown kernel-accessible receive storage and reports an I/O error to the receiver. Already delivered leases remain safe to inspect. Unlike an owned single-shot operation, the receiver has no caller-supplied in-flight buffer to return.

The completion engine is adapted from Socketry revision `7a70586d5d37fb640d696d0c03a05b2ee3cbe06b`, preserving its MIT attribution.

## Worker routing and event processing

Each ring has its own request capacity, operation identifiers, pending queue, and in-flight storage. Local submissions bypass the synchronized inbox and notification eventfd. A private thread-local lookup selects only the current worker belonging to the explicitly supplied selector; it does not select an implicit scheduler. Root futures and other external callers use round-robin routing through per-ring inboxes. Submitted operations retain their original ring through completion and cancellation even when their tasks migrate.

Owner-thread wakes coalesce in a local flag, retained until the actual idle-wait decision. Cross-thread wakes continue to signal the eventfd. A completed notification poll drains its counter before rearming, so an idle worker does not repeatedly complete the same notification. Cancellations and payloads get SQ space before the notification poll, preventing starvation with a one-entry queue. Idle turns publish submissions and wait in a single ring entry; productive turns skip empty entries unless kernel overflow or deferred work requires progress.

Completion processing publishes results and wakes tasks without polling application futures. The owning worker publishes the entire available CQ batch before polling its ready queue; other workers may steal published tasks immediately. Bounded command processing and periodic driver turns prevent either commands or runnable tasks from starving I/O progress. Cross-queue cancellation preserves submission order after migration.

Drivers implement `SelectorDriver`: entering consumes the movable driver and returns a thread-bound `IoUringWorker`, implementing `SelectorWorker`. This keeps local routing and queue storage on the owning thread. Native workers keep driving accepted I/O after task abandonment. Stopping or failing any ring closes admission for the entire selector group, including its shared fallback; surviving drivers continue retiring their accepted work. Applications coordinate cancellation and await their tasks before shutting down the scheduler.

Ordinary request records and completion channels still allocate per ring request; file monitors reuse their storage. External notifications currently write to each ring's interrupt eventfd. These are implementation costs rather than interface requirements. Native task ownership and executor counters also remain shared; per-worker rings remove shared I/O admission and submission contention, rather than every source of executor contention.

## Releasing

This package releases with the workspace. Follow the [workspace release process](../../readme.md#releasing).

## Releases

See the shared [release notes](../../releases.md) for changes awaiting the first release.

## See Also

The [workspace overview](../../readme.md) describes the interface and companion backends.

## Contributing

Please open an issue or pull request on [GitHub](https://github.com/core-rs/core-scheduler).
