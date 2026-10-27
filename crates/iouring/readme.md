# `base-scheduler-iouring`

A Linux selector with independently driven completion I/O. Owned file reads and writes (including positioned operations), socket reads/writes, and socket message sends/receives use io\_uring. Borrowed `_poll` operations try nonblocking syscalls directly and use ring readiness waits after `WouldBlock`. Other file operations, socket lifecycle, resolution, blocking work, and signals use an explicitly owned `ReadinessSelector` fallback.

Application code constructs scheduler-bound stream and packet readers and writers, plus file readers and writers. Operations accept cancellation without another scheduler argument; registration, request reuse, and worker migration remain internal. See the [I/O usage guide](../../context/io.md).

## Usage

Add this package with `cargo add base-scheduler-iouring base-scheduler-native`.

`IoUringSelector::with_workers(options, workers)` creates one ring per worker, with one shared readiness fallback. `NativeScheduler::with_drivers` owns and drives those rings directly on its task workers:

```rust
# #[cfg(target_os = "linux")]
# {
use base_scheduler_iouring::{IoUringOptions, IoUringSelector};
use base_scheduler_native::{NativeExecutorOptions, NativeScheduler};

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

Bound `FileReader` and `FileWriter` handles retain typed request and completion storage for one sequential stream of owned reads or writes through internal file monitors. Reusing the buffer-owner type avoids fresh request/channel allocations after warmup. Independent monitors can share a file for concurrent operations; use positioned operations for independent offsets.

Each monitor also caches an optional fixed-file registration for its current ring. `IoUringOptions::fixed_files` limits each ring's sparse table and defaults to 256; zero disables registration. Unsupported setup, insertion failure, or exhaustion falls back to ordinary descriptors. A future moving threads keeps its accepted operation on the original ring. After retirement, a later operation can use another worker, releasing the previous registration through its original ring and creating a new registration on the destination. Returning to a previous worker registers again. Pending and failed releases remain tracked for explicit close, so old registrations can temporarily coexist with the current one. This cache does not implement registered buffers.

Call the bound handle’s uncancellable `close()` while the drivers remain active, then drop the handle before exclusively closing the file. Close retires accepted I/O and waits for every cached ring registration to be released. Dropping a close future does not establish retirement; call close again to finish and observe its result. Dropping the handle queues release without waiting. See the [ring-state research](../../benchmarks/files/ring-state-research.md) for measured benefits, costs, and further opportunities.

## Borrowed I/O

Bound readers and writers attempt nonblocking syscalls with `MSG_DONTWAIT`. Ready calls allocate no request, acquire no ring capacity, and continue in the same task poll. After `WouldBlock`, their private persistent directional registration waits on the current worker's ring, then retries the syscall. Readiness remains advisory. Ordinary scalar stream reads with OOB support disabled clear only their observed generation after a short positive read and wait before probing again; full reads keep draining. Closure notifications inhibit short-read clearing so the final data cannot hide EOF. OOB-enabled scalar reads, pooled reads, and flagged/message receives retain syscall-first retries and clear only after `WouldBlock`. Use `with_oob_support::<true>()` when normal reads must progress across urgent marks, including with an OOB-consuming alias. Ring poll completions can forward broad TCP wake hints containing `POLLPRI` for ordinary data, so disabled support rejects only unambiguous isolated priority notifications and does not guarantee urgent-data detection. Worker migration retires the previous registration before installing its replacement. No borrowed payload or vector pointer survives a poll; dropping a borrowed operation ends access immediately. Close retires the handle's registration before exclusive socket close.

## Receiving message sequences

`BufferPooling::buffer_pool(options)` creates one exact payload budget shared across connections and workers, independent of worker count. Pass the pool to `socket_stream_reader_with_pool` or `socket_packet_reader_with_pool`; ordinary pools use demand-driven reception. It allocates no new payload per message and composes with the portable polling path. Receive strategy and registration state remain private; applications use the same bound reader methods as other schedulers.

Readers receive on demand and do not retain a receive-ahead queue. Overlap input and output explicitly: read a chunk, submit owned output through the independent stream writer, then await the next read while accepted output progresses. Subsequent submissions await their predecessor, and `flush` completes the final output. Two reusable pool slots accommodate one read and one write; retained application views require additional capacity. Packet reads and writes can be polled together on independent handles.

`IoUringSelector::socket_recvmsg_multishot` starts one persistent receive using a bounded provided-buffer ring. This is a concrete experimental API; import `RecvMsgMultishotOptions` from `base_scheduler_iouring`. With Native, construct it through `scheduler.selector()`, then use `recv(&cancellation).await` and `close().await` on the bound sequence. Portable application code uses demand-driven bound stream or packet readers; this experiment is independent of those readers. The concrete receiver exposes immutable owned messages and retires reception on pending-wait cancellation. Returned payloads can outlive close or move to another worker; local returns publish directly and remote returns use the owning ring's inbox.

The kernel rearms a multishot request across receives. Only a CQE without `IORING_CQE_F_MORE` retires that submission. Exhaustion pauses the receiver instead of spinning: returning a buffer resubmits the request. TCP EOF ends reception; empty datagrams remain messages, and truncation and sender metadata are preserved. Peeking is rejected. Linux 6.0 or later and provided-buffer ring support are required; setup or operation errors are explicit, without a fallback.

Each receiver retains one admission slot until EOF or close, even while its buffer pool is exhausted. Reserve additional capacity for sends and other operations which can wait for the ring. `RecvMsgMultishotOptions` defaults to eight 64 KiB payload buffers per receiver, with additional metadata storage. This is setup-time allocation; delivery reuses the request and reserved result queue. Call `close().await` before exclusive socket close to cancel and await the final original completion. Drop requests cancellation without waiting. Returned leases remain valid after receiver or scheduler shutdown. Dropping or forgetting an individual `recv` future cannot expose its caller's memory to the kernel. A token already cancelled before `recv` leaves the receiver unchanged; cancellation of a pending wait closes the receiver before returning. Queued successful receives win concurrent cancellation.

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

`ProcessWait` uses `WAITID` when opcode probing reports support, retaining its output until the original completion. Older kernels use the shared readiness/pool fallback. Exact-PID, any-child, group, nonblocking, and stopped/continued waits share the [process-wait contract](../../context/interface.md#child-process-waits).

## Releases

See the shared [release notes](../../releases.md).

## See Also

The [workspace overview](../../readme.md) describes the interface and companion backends.

## Contributing

Please open an issue or pull request on [GitHub](https://github.com/base-rs/base-scheduler).
