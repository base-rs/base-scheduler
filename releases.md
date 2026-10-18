# Releases

## Unreleased

- Add owned `IoBuffers` and `IoBuffersMut` packages with reusable native I/O vectors and explicit advancement after partial transfers. Sequence traits now expose indexed byte ranges and optional prepared vectors instead of mutable source-owner slices.

- Reuse direct blocking-task completion state for handle abandonment, removing a separate shared allocation per blocking submission.
- Accept owned buffer sequences for message I/O and backend-associated address resolution results with borrowed entry views, allowing arrays, pooled collections, and native resolver lists without mandatory copied vectors.
- Keep cancellation and direct capacity listeners inline in waiting futures, avoiding per-wait allocations and unnecessary capacity registration during immediate admission.
- Extract runtime-independent scheduler capabilities and shared types from Socketry.
- Make operation context explicit, retain implementation-associated resources and owning task handles, and preserve owned-buffer and cooperative-cancellation contracts.
- Add `protocol-scheduler-conformance` with reusable file and blocking-work assertions.
- Add `protocol-scheduler-direct` with bounded file/blocking workers, independent cleanup, and explicit shutdown.
