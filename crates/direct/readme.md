# Protocol Scheduler Direct

`protocol-scheduler-direct` implements file operations, IP sockets, scatter/gather message I/O, and bounded blocking work using dedicated worker threads. It works with any executor that polls standard Rust futures; it supplies no executor, public clock, or signal service.

```rust
use protocol_scheduler::{Cancellation, FileClose, FileOpen, FileOptions, FileRead};
use protocol_scheduler_direct::{DirectOptions, DirectScheduler};

# async fn example(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
let scheduler = DirectScheduler::new(DirectOptions::default())?;
let cancellation = Cancellation::new();
let file = scheduler.file_open(path, FileOptions::READ, &cancellation).await?;
let (result, buffer) = scheduler.file_read(&file, vec![0; 4096], &cancellation).await;
let count = result?;
assert!(count <= buffer.len());
scheduler.file_close(file).await?;
scheduler.shutdown().await;
# Ok(()) }
```

Each operation receives the scheduler and cancellation explicitly. `DirectFile` and `DirectSocket` contain no scheduler or registration affinity; successive operations can use different direct instances. Open files through `FileOpen`; native file import and access are deferred until descriptor flags and aliases can preserve positioned-write and exclusive-close guarantees.

Admission is bounded by queued plus running jobs. Cancellation prevents admission or queued I/O execution; a running syscall preserves its actual result. Dropping an I/O future does not release its resources while a worker still uses them. General blocking-job handles own queued execution; dropping one can skip queued work, but cannot interrupt a running closure.

Close has a separate worker and bounded queue, so ordinary work cannot exhaust cleanup capacity. Accepted cleanup survives waiter abandonment. Await all outstanding uses before explicit close; otherwise close panics rather than racing backend access. Ordinary destruction does not report OS close errors. Explicit close consumes the OS handle exactly once and reports errors without retrying.

`shutdown().await` closes admission, skips queued general blocking jobs, and waits for running work, accepted I/O, and accepted cleanup. Coordinate application cleanup first. A synchronous operation that never returns can prevent shutdown; do not wait for shutdown from a job it must drain. Dropping the scheduler closes admission without synchronously joining workers.

This initial backend supports Unix and Windows. Positioned I/O leaves the shared cursor unchanged on Unix; Windows uses the standard-library positioned methods, which change it. File operations retain platform error and durability semantics. Socket creation, bind, listen, connect, accept, scalar I/O, readiness, shutdown, close, and scatter/gather messages support both IPv4 and IPv6. Socket options and native resource import remain optional capabilities that direct does not implement.

Sockets stay nonblocking throughout their lifetime. Each syscall attempt uses the ordinary bounded pool. `WouldBlock` and interrupted attempts return their worker and admission slot before waiting for a scheduler-local retry pulse, so pending reads do not consume the capacity needed by writes. One lazily started retry thread wakes waiting operations approximately every two milliseconds and sleeps while none are waiting. This deliberately simple reference policy trades latency and repeated attempts for avoiding a reactor, a thread per socket operation, or blocking workers on network traffic. Capacity bounds queued/running attempts, not the number of caller-owned pending socket futures. General blocking closures and synchronous file calls can still occupy ordinary workers; applications must arrange their eventual completion.

`SocketSendMsg` and `SocketRecvMsg` use prepared `IoBuffers`/`IoBuffersMut` arrays directly during synchronous `sendmsg`/`recvmsg` or non-overlapped Winsock calls. Other sequences build native vectors in scratch storage reused across attempts. Each operation makes one transfer, preserves short counts and datagram boundaries, and returns the unchanged collection owner. Unix reports discarded control data when the OS supplies `MSG_CTRUNC`; the Windows payload/address path has no control metadata. No syscall retains vector or payload pointers after that attempt returns. Accepted attempts retain the entire owner even if the waiting future or scheduler is dropped. Between attempts, the future owns its input with no backend access remaining; dropping it safely ends retries. Scheduler shutdown wakes pending retry waiters and drains accepted attempts and cleanup, without requiring abandoned futures to be polled.

The workspace is unpublished. Behavioral tests use `protocol-scheduler-conformance` as a development dependency; the implementation never depends on it in production.
