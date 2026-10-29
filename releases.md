# Releases

## Unreleased

- Reduce Native readiness overhead on established stream reads: skip redundant notifications for directions that are already ready, bypass cancellation wrappers for never-cancelled operations, and poll a compact receive future. Native sockets now register with the portable reactor only when a fallback wait first needs it, so `NativeSocket::retain_native` returns the new `RegisteredSocket`, which exposes the socket and its descriptor but not async-io's readiness methods. This is a breaking change for backend integrations, and a reactor registration failure surfaces at that wait.

## v0.2.0

- Correct all socket reader/writer factory lifetime bounds so connections can own sockets alongside their directional handles. Handles retain scheduler and backend access without borrowing the temporary socket reference. Backend implementors must add compatible precise capture bounds (`use<...>`); raise the workspace minimum Rust version to 1.87.

- Publish immediately available io\_uring completions after nonblocking submission, reducing delivery latency while preserving cancellation, notification handling and shutdown.

- Batch io\_uring completion task work on fresh thread-bound workers with deferred task running. Preserve cooperative and original-ring fallbacks, nonblocking completion progress, cancellation and movable drivers.

- Avoid an extra `WouldBlock` receive after short borrowed scalar stream reads on io\_uring when OOB support is disabled. Preserve EOF notifications, racing readiness, cancellation and registration retirement; opt-in OOB and message receives keep conservative retries.

- Add compile-time `SocketStreamReader::with_oob_support` selection, disabled by default. Native Linux epoll avoids an extra receive after ordinary short scalar reads and rejects reported urgent data; opt-in support preserves ordinary-byte progress across urgent marks. Other receive paths retain conservative behavior.

## v0.1.2

- Avoid retrying blocked fallback socket operations on unrelated task wakes, reducing io\_uring listener overhead while preserving immediate I/O progress.

## v0.1.1

- Reduce Linux worker-readiness overhead with native epoll event delivery and eventfd interruption, preserving precise timeouts and compatibility fallbacks.

- Drive Native socket readiness on executor workers, with lazy per-socket registration, independent concurrent waiters, and cached stream-read readiness.

- Reduce worker-readiness notification overhead while preserving independent waiters, cancellation, and shutdown wakeups.

- Return successful borrowed stream writes before constructing wait futures; retain lazy registration and ordered owned output.

- Dispatch macOS and Linux/Android readiness events through stable registration pointers, with deferred retirement and failed-registration quarantine.

- Decode macOS kqueue events directly from one reusable native-event vector, removing normalized event staging.

## v0.1.0

- Publish as `base-scheduler` in the `base-rs` organization, with package versions starting at 0.1.0.
- Rename the companion scheduler packages and Rust crate paths to the `base-scheduler` family.

Initial release of the runtime-independent scheduler interface and its seven companion crates. The workspace requires Rust 1.85 or later.

- Define separate capabilities for clocks, task spawning, bounded blocking work, address resolution, signals, process waits, and file/socket lifecycle operations. Protocol libraries depend on the capabilities they use; applications choose a backend.

- Provide scheduler-bound stream, packet, and file readers and writers. Borrowed socket operations reuse caller storage; owned operations retain stable storage through completion, including after their waiting future is dropped. Keep registration, routing, and retirement state inside each backend.

- Make cancellation explicit per operation. Provide owning task handles, bounded admission, independent resource retirement, and asynchronous cleanup. Add `Clock::timeout` with operation-first polling, distinct expiry and clock errors, and no timer registration for immediately ready operations.

- Add reusable buffer pools, unique writable chunks, retained immutable views, and owned scatter/gather storage. Support explicit read/write overlap, ordered owned stream submission and flushing, and separate file acceptance/completion waits.

- Include Direct reference and Tokio adapter backends, plus the Native work-stealing executor with worker-local event loops and timers. Provide independently driven readiness and Linux io\_uring selectors, with per-worker rings, reusable request state, and optional fixed-file registrations.

- Support child process status waits, Unix process-group selection and state-change flags, native process notifications, optional io\_uring `WAITID`, and bounded blocking fallbacks. A started blocking wait may finish after cancellation and delay shutdown.

- Supply reusable conformance suites, cross-backend I/O examples, and matched network, file, buffer, and timer benchmarks. CI checks Linux, macOS, Windows, the minimum Rust version, and complete measured source-region coverage.

- Keep the reusable timer capability and concrete io\_uring multishot receiver API experimental. Portable application code uses the bound reader/writer capabilities.
