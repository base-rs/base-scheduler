# Releases

## Unreleased

- Drive Native socket readiness on executor workers, with lazy per-socket registration, independent concurrent waiters, and cached stream-read readiness.
- Reduce worker-readiness notification overhead while preserving independent waiters, cancellation, and shutdown wakeups.

## v0.1.0

- Publish as `base-scheduler` in the `base-rs` organization, with package versions starting at 0.1.0.
- Rename the companion scheduler packages and Rust crate paths to the `base-scheduler` family.

Initial release of the runtime-independent scheduler interface and its seven companion crates. The workspace requires Rust 1.85 or later.

- Define separate capabilities for clocks, task spawning, bounded blocking work, address resolution, signals, process waits, and file/socket lifecycle operations. Protocol libraries depend on the capabilities they use; applications choose a backend.

- Provide scheduler-bound stream, packet, and file readers and writers. Borrowed socket operations reuse caller storage; owned operations retain stable storage through completion, including after their waiting future is dropped. Keep registration, routing, and retirement state inside each backend.

- Make cancellation explicit per operation. Provide owning task handles, bounded admission, independent resource retirement, and asynchronous cleanup. Add `Clock::timeout` with operation-first polling, distinct expiry and clock errors, and no timer registration for immediately ready operations.

- Add reusable buffer pools, unique writable chunks, retained immutable views, and owned scatter/gather storage. Support explicit read/write overlap, ordered owned stream submission and flushing, and separate file acceptance/completion waits.

- Include Direct reference and Tokio adapter backends, plus the Native work-stealing executor with worker-local event loops and timers. Provide independently driven readiness and Linux io_uring selectors, with per-worker rings, reusable request state, and optional fixed-file registrations.

- Support child process status waits, Unix process-group selection and state-change flags, native process notifications, optional io_uring `WAITID`, and bounded blocking fallbacks. A started blocking wait may finish after cancellation and delay shutdown.

- Supply reusable conformance suites, cross-backend I/O examples, and matched network, file, buffer, and timer benchmarks. CI checks Linux, macOS, Windows, the minimum Rust version, and complete measured source-region coverage.

- Keep the reusable timer capability and concrete io_uring multishot receiver API experimental. Portable application code uses the bound reader/writer capabilities.
