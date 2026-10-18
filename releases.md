# Releases

## Unreleased

- Accept owned buffer sequences for message I/O and backend-associated address resolution results with borrowed entry views, allowing arrays, pooled collections, and native resolver lists without mandatory copied vectors.
- Keep cancellation and direct capacity listeners inline in waiting futures, avoiding per-wait allocations and unnecessary capacity registration during immediate admission.
- Extract runtime-independent scheduler capabilities and shared types from Socketry.
- Make operation context explicit, retain implementation-associated resources and owning task handles, and preserve owned-buffer and cooperative-cancellation contracts.
- Add `protocol-scheduler-conformance` with reusable file and blocking-work assertions.
- Add `protocol-scheduler-direct` with bounded file/blocking workers, independent cleanup, and explicit shutdown.
