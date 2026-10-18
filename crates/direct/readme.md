# Protocol Scheduler Direct

`protocol-scheduler-direct` implements file operations and bounded blocking work using dedicated worker threads. It works with any executor that polls standard Rust futures; it supplies no executor, sockets, timers, or signal service.

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

Each operation receives the scheduler and cancellation explicitly. `DirectFile` contains no scheduler or registration affinity; successive operations can use different direct instances. Open files through `FileOpen`; native file import and access are deferred until descriptor flags and aliases can preserve positioned-write and exclusive-close guarantees.

Admission is bounded by queued plus running jobs. Cancellation prevents admission or queued I/O execution; a running syscall preserves its actual result. Dropping an I/O future does not release its resources while a worker still uses them. General blocking-job handles own queued execution; dropping one can skip queued work, but cannot interrupt a running closure.

Close has a separate worker and bounded queue, so ordinary work cannot exhaust cleanup capacity. Accepted cleanup survives waiter abandonment. Await all outstanding uses before explicit close; otherwise close panics rather than racing backend access. Ordinary destruction does not report OS close errors. Explicit close consumes the OS handle exactly once and reports errors without retrying.

`shutdown().await` closes admission, skips queued general blocking jobs, and waits for running work, accepted I/O, and accepted cleanup. Coordinate application cleanup first. A synchronous operation that never returns can prevent shutdown; do not wait for shutdown from a job it must drain. Dropping the scheduler closes admission without synchronously joining workers.

This initial backend supports Unix and Windows. Positioned I/O leaves the shared cursor unchanged on Unix; Windows uses the standard-library positioned methods, which change it. File operations retain platform error and durability semantics. Socket I/O needs a separate progress/capacity design before inclusion.

The workspace is unpublished. Behavioral tests use `protocol-scheduler-conformance` as a development dependency; the implementation never depends on it in production.
