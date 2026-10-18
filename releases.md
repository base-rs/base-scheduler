# Releases

## Unreleased

- Add shared conformance suites for clocks, tasks, address resolution, signals, typed socket options, TCP registration, file allocation, and advice, with controlled fixtures and rejection tests.
- Rename `Signals::signal_notify` to `signal_subscribe` and its associated `Registration` type to `Subscription`.

- Generate individually reported conformance suites from shared capability-specific case lists, with backend-owned runners and isolated fixtures. Direct now uses these suites; existing async assertion helpers remain available.

- Add reusable socket/message conformance tests and direct IPv4/IPv6 socket support, including prepared scatter/gather I/O, cancellation/retirement fixtures, nonblocking retry without occupied worker slots, and independent cleanup.

- Add owned `IoBuffers` and `IoBuffersMut` packages with reusable native I/O vectors and explicit advancement after partial transfers. Sequence traits now expose indexed byte ranges and optional prepared vectors instead of mutable source-owner slices.

- Reuse direct blocking-task completion state for handle abandonment, removing a separate shared allocation per blocking submission.
- Accept owned buffer sequences for message I/O and backend-associated address resolution results with borrowed entry views, allowing arrays, pooled collections, and native resolver lists without mandatory copied vectors.
- Keep cancellation and direct capacity listeners inline in waiting futures, avoiding per-wait allocations and unnecessary capacity registration during immediate admission.
- Extract runtime-independent scheduler capabilities and shared types from Socketry.
- Make operation context explicit, retain implementation-associated resources and owning task handles, and preserve owned-buffer and cooperative-cancellation contracts.
- Add `protocol-scheduler-conformance` with reusable file and blocking-work assertions.
- Add `protocol-scheduler-direct` with bounded file/blocking workers, independent cleanup, and explicit shutdown.
