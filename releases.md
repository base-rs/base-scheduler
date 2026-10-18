# Releases

## Unreleased

- Extract runtime-independent scheduler capabilities and shared types from Socketry.
- Make operation context explicit, retain implementation-associated resources and owning task handles, and preserve owned-buffer and cooperative-cancellation contracts.

- Add `protocol-scheduler-conformance` with reusable file and blocking-work assertions.
- Add `protocol-scheduler-direct` with bounded file/blocking workers, independent cleanup, and explicit shutdown.
