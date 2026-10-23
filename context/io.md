# Writing efficient I/O

Start with the data's lifetime. Use **persistent monitors for borrowed I/O** and **pooled receivers for independently owned received data**. These are recommended compositions of the [scheduler interface](interface.md), not a ranking of syscalls. A pooled receiver can itself use a persistent monitor and borrowed nonblocking I/O. Pooling supplies ownership, retention, reuse, and backpressure; it does not inherently make a receive faster.

## Choose by application behavior

| Application requirement | Starting point |
| --- | --- |
| Consume stream bytes before reusing storage | One readable stream monitor and a reused borrowed slice. |
| Retain complete frames while receiving more bytes | A scheduler-created pool and socket receiver; freeze completed storage and retain ranges. |
| Process each datagram entirely inside its receive loop | One general readable socket monitor and borrowed message receive. |
| Hand packets to independent tasks | A pooled socket receiver and bounded task handoff; move the complete message when peer metadata is needed. |
| Send data that remains available until the awaited write finishes | One writable monitor and borrowed scalar or vectored sends, advancing after short writes. |
| Copy or transform files | Sequential file monitors, reusable owned buffers, and joined independent reads/writes. |

Borrowed storage may be inline or a reusable `Vec`; prefer the storage layout the parser needs. Stack placement alone is not the optimization. Construct monitors and storage outside repeated-operation loops. Use one monitor per sequential execution, with independent readable and writable monitors for concurrent tasks. Use stream monitors only for byte streams and general socket monitors for datagrams.

Owned operations remain useful for completion-based dispatch and buffers that must retire independently of their caller's future. One-off operations need not manufacture persistent state. The `_poll` suffix names the borrowed readiness contract; these are ordinary async methods called with `.await`, not raw `Poll` methods.

## Keep protocol state explicit

Pass scheduler capabilities and cancellation to operations. Retain resources, monitors, parsers, and storage in protocol objects without implicitly selecting a scheduler. Require the individual capabilities used by that protocol, rather than a universal backend or scheduler aggregate. A different scheduler argument still has to be compatible with the retained resources.

Use `scheduler.buffer_pool(options)` and `scheduler.socket_receiver(...)` for portable owned reception. Application code chooses a memory budget and required ownership; the backend chooses allocation, registration, publication, and receive strategy. Do not put worker IDs, ring IDs, buffer group IDs, multishot selection, or registration switches into ordinary protocol call sites. Concrete backend experiments belong in backend-specific benchmarks until measurements and semantics justify an application-visible choice.

## Bound retention and finish cleanup

Parse while a received lease is uniquely writable when mutation is useful. Freeze it before retaining immutable views. Completed frames wholly inside a chunk can retain ranges; copy or assemble only the incomplete frame crossing chunks when contiguous records are required. A small long-lived view can pin an entire slot, so selective copying can improve memory use. Framing, decryption, and compaction belong to the protocol.

Budget for application-held views and in-flight operations. Bound packet tasks and retained frames; keeping every slot while awaiting another receive deadlocks progress. `BufferPoolOptions::buffers` is the exact shared count, independent of worker count, and exhaustion backpressures rather than growing. Do not depend on worker-local allocation or particular recycling order.

Handle partial stream and file transfers. Zero bytes on a nonempty stream read means EOF; an empty datagram is a real message. Check packet truncation and preserve the peer address when dispatching. Borrowed address views cannot outlive their message; convert the endpoint before extracting payload ownership if it is needed afterwards.

For file pipelines, poll both independent operations with `zip`/join. Merely constructing a read and a write does not submit them. Two reusable buffers suffice for a one-read/one-write pipeline. If using pooled buffers, release a completed write's owner inside its branch so a capacity-waiting read can progress. Explicit submission remains available when acceptance itself is required; it is not the default pipeline optimization.

Cancellation requests progress toward shutdown, not rollback. Capture the loop's result, await uncancellable monitor/receiver close, then return the result. Await all accepted task handles, including on failure; dropping a handle requests task destruction. Close and drop every monitor/receiver before exclusively closing its resource. Only successful close proves retirement; stop using the handle once close begins, even if cleanup fails. Applications decide how to report an operation failure alongside a cleanup failure. The examples prioritize cleanup failures.

## Keep examples and measurements aligned

Use the [canonical I/O recipes](https://github.com/core-rs/core-scheduler/tree/main/examples) as starting points. The runnable driver, correctness tests, and benchmarks call the same recipe functions on Direct, Tokio, Native readiness, and Native io_uring. Extend these workloads when changing the recommendation rather than creating an unrelated favorable microbenchmark.

Use optimized builds, execute I/O on scheduler workers, validate received contents, and record setup, warmup, timing boundaries, payload sizes, pool budgets, worker counts, environment, and source revision. Compare equivalent ownership and parser behavior: immediate borrowed consumption and retaining frames solve different problems. A pooled receive is not universally faster than reused borrowed storage, and multishot, registered buffers, and zero-copy sends are backend mechanisms with workload-dependent benefits. Keep the recommended path simple until repeatable measurements support a change.
