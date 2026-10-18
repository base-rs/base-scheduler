# Protocol Scheduler Conformance

Reusable, capability-specific assertions for implementations of `protocol-scheduler`. Add this crate as a **development dependency** of a backend, then call its async test functions from that backend's integration tests while its executor and services are driven.

The file tests accept backend-created resources and paths. Each function requires only the capabilities it exercises. The blocking tests accept a `Blocking` implementation; deterministic queued-work tests additionally use a backend fixture that holds execution at a known boundary. No test assumes one runtime, task order, concrete descriptor, or prompt cancellation of a running syscall.

Tests panic on contract violations. The initial suite covers file contents and cursor semantics, ownership, pre-cancellation and lazy operations, positioned I/O, metadata, truncation, synchronization, compatible execution contexts, and blocking admission/ownership. Backend fixtures remain responsible for deterministic running-operation races, retirement after abandonment, shutdown, and platform-specific behavior. Unsupported capabilities must not be presented as passing tests.

The direct backend provides the first integration. Future adapters can run the same functions without depending on direct. Fixtures and assertions live here; backend-specific internal checks belong in backend crates.
