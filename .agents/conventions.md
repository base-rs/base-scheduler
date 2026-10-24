# Conventions

- Keep the root interface crate independent of concrete schedulers, executors, reactors, and platform registrations.

- Construct stateful I/O readers and writers from an explicit scheduler and resource. Retain those bound capabilities in protocol objects and pass cancellation to each cancellable operation; worker migration remains internal to the backend.

- Prefer individual capabilities and implementation-associated resource and task-handle types. Preserve the owned-buffer and cooperative-cancellation contracts.

- Follow the shared Core conventions, testing, pull request, and release skills. Use the private `bake/` package for project tooling.

- Record intentional differences from the Socketry extraction in `context/design.md`. Keep concrete backend conformance tests in their own packages; test runtime-independent support and public generic composition here.

- Companion crates live under `crates/`. The conformance crate depends only on the interface and test support; direct uses conformance only as a development dependency. Workspace packages share versions and release notes.

- Follow `context/io.md` when writing protocol I/O, examples, or benchmarks. Use bound stream/packet readers and writers as the default socket API, and bound file readers and writers for repeated file I/O. Monitors, receivers, raw operation hooks, and default bound implementations live in `core_scheduler::backend`. Keep application factory bounds independent of those hooks; generic protocols depend only on the resource capabilities they use. Keep borrowed-only paths free of receive-pool allocation.
