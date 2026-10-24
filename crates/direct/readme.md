# `core-scheduler-direct`

`core-scheduler-direct` is a reference backend for `core-scheduler`, with task execution, clocks, native address resolution, file operations, IP sockets, scatter/gather I/O, and bounded blocking work. Operation futures work with any executor that polls standard Rust futures. Services belong to the explicit scheduler instance; there is no implicit current scheduler or global fallback executor.

## Usage

The workspace is not published yet. Add this package from Git with `cargo add core-scheduler core-scheduler-direct --git https://github.com/core-rs/core-scheduler`.

```rust
use core_scheduler::{Cancellation, FileClose, FileOpen, FileOptions, FileRead};
use core_scheduler_direct::{DirectOptions, DirectScheduler};

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

## Ownership and shutdown

Each operation receives the scheduler and cancellation explicitly. `DirectFile` and `DirectSocket` contain no scheduler or registration affinity; successive operations can use different direct instances. Open files through `FileOpen`; native file import and access are deferred until descriptor flags and aliases can preserve positioned-write and exclusive-close guarantees.

Admission is bounded by queued plus running jobs. Cancellation prevents admission or queued I/O execution; a running syscall preserves its actual result. Dropping an I/O future does not release its resources while a worker still uses them. General blocking-job handles own queued execution; dropping one can skip queued work, but cannot interrupt a running closure.

Close has a separate worker and bounded queue, so ordinary work cannot exhaust cleanup capacity. Accepted cleanup survives waiter abandonment. Await all outstanding uses before explicit close; otherwise close panics rather than racing backend access. Ordinary destruction does not report OS close errors. Explicit close consumes the OS handle exactly once and reports errors without retrying.

`shutdown().await` closes admission, skips queued general blocking jobs, stops timers and signal delivery, and waits for live tasks, running work, accepted I/O, and accepted cleanup. Coordinate task cancellation, await task handles, and finish application cleanup before shutdown. A task that never completes or a synchronous operation that never returns can prevent shutdown; do not wait for shutdown from a task or job it must drain. Dropping the scheduler closes admission without synchronously joining workers; live task handles continue owning their tasks.

## Capabilities

This backend supports Unix and Windows. Positioned I/O leaves the shared cursor unchanged on Unix; Windows uses the standard-library positioned methods, which change it. File operations retain platform error and durability semantics. Socket creation, bind, listen, connect, accept, scalar I/O, readiness, shutdown, close, and scatter/gather messages support both IPv4 and IPv6. Typed options include `TcpNoDelay`, `SocketKeepalive`, and `SocketReuseAddress`. `SocketRegistration` imports standard-library TCP streams and listeners, preserving their endpoints and connections while selecting nonblocking mode; do not change that mode through external aliases.

| Capabilities | Supported platforms |
| --- | --- |
| Core file/socket I/O, message I/O, blocking work, tasks, clocks, address resolution, socket options, TCP registration | Unix and Windows |
| Signal subscriptions | Unix |
| File allocation and advice with the interface's Linux semantics | Linux |

`Spawn` uses separate task-polling workers and owning `DirectTask` handles, backed by `async-task`. Registration precedes runnable admission, panic payloads are returned as `TaskError`, and handle drop requests destruction after an active poll returns. Future and unconsumed-output destructor panics are contained so abandonment cannot abort the executor. An unobserved panic payload may be leaked to contain a payload destructor that also panics. `DirectOptions::threads` sizes the syscall pool and, separately, the task-polling pool. Polling workers start during construction so startup errors can be returned before the infallible-registration error vocabulary of `Spawn` is needed. Task admission is synchronous and does not share bounded syscall capacity.

`Clock` uses monotonic `Instant` time and one lazy timer-driver thread. Waiting timers do not occupy syscall or task workers. Cancellation and drop remove registrations, and pending waits fail when the scheduler closes. `AddressResolution` calls the system resolver through bounded ordinary workers, retaining inputs and capacity until a started lookup returns. `SystemAddresses` owns the unchanged native `addrinfo` list and exposes borrowed `SystemAddressInfo` views; iteration and field conversion allocate no additional collection. Native ordering, duplicates, scoped addresses, parameters, and resolver errors are preserved.

Unix `Signals` uses `signal-hook` to bridge OS handlers to a scheduler-owned dispatch thread. Callbacks run outside raw handlers and registry locks; subscriptions are independent and dropping a `DirectSubscription` does not join an active callback. Closing the service stops notification admission and shutdown waits for dispatch to finish. Callback panics are contained so other subscriptions continue. Signal dispositions follow `signal-hook`'s process-wide policy and are not restored by unsubscribe; applications coordinate handler ownership and choose termination policy explicitly. Windows omits this capability.

Linux `FileAllocate` invokes `fallocate64`, including `KEEP_SIZE`, without emulating reservation by resizing or writing zeros. `FileAdvice` invokes `posix_fadvise64` through independent cleanup admission; completion reports execution errors and submission reports admission only. Accepted advice retains its file through execution after waiter abandonment. Filesystem-specific support and errors remain observable. Other platforms omit these traits rather than substitute weaker guarantees.

## Socket I/O

Sockets stay nonblocking throughout their lifetime. Each syscall attempt uses the ordinary bounded pool. `WouldBlock` and interrupted attempts return their worker and admission slot before waiting for a scheduler-local retry pulse, so pending reads do not consume the capacity needed by writes. One lazily started retry thread wakes waiting operations approximately every two milliseconds and sleeps while none are waiting. This deliberately simple reference policy trades latency and repeated attempts for avoiding a reactor, a thread per socket operation, or blocking workers on network traffic. Capacity bounds queued/running attempts, not the number of caller-owned pending socket futures. General blocking closures and synchronous file calls can still occupy ordinary workers; applications must arrange their eventual completion.

`SocketSendMsg` and `SocketRecvMsg` use prepared `IoBuffers`/`IoBuffersMut` arrays directly during synchronous `sendmsg`/`recvmsg` or non-overlapped Winsock calls. Other sequences build native vectors in scratch storage reused across attempts. Each operation makes one transfer, preserves short counts and datagram boundaries, and returns the unchanged collection owner. Unix reports discarded control data when the OS supplies `MSG_CTRUNC`; the Windows payload/address path has no control metadata. No syscall retains vector or payload pointers after that attempt returns. Accepted attempts retain the entire owner even if the waiting future or scheduler is dropped. Between attempts, the future owns its input with no backend access remaining; dropping it safely ends retries. Scheduler shutdown wakes pending retry waiters and drains accepted attempts and cleanup, without requiring abandoned futures to be polled.

Construct scheduler-bound stream and packet readers and writers for application I/O, and file readers and writers for repeated file operations. Their operations take cancellation without another scheduler argument. Borrowed-only readers allocate no payload pool; owned reads use lazy default storage or an explicit shared pool. Close and drop every handle before exclusively closing its underlying resource. See the [I/O usage guide](../../context/io.md) for the recommended paths.

## Development

Behavioral tests use the capability-specific `core-scheduler-conformance` macros as a development dependency. Each generated case has its own fixture and ordinary Rust test name, including separate IPv4/IPv6 suites and scalar/prepared I/O lifecycle suites. Backend runners enforce deadlines, drain accepted work on assertion failure, and preserve panics for test reporting. The implementation never depends on conformance in production.

## Releasing

This package releases with the workspace. Follow the [workspace release process](../../readme.md#releasing).

## Releases

See the shared [release notes](../../releases.md) for changes awaiting the first release.

## See Also

The [workspace overview](../../readme.md) describes the interface and companion backends.

## Contributing

Please open an issue or pull request on [GitHub](https://github.com/core-rs/core-scheduler).
