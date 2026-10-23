# Conventions

- Keep the root interface crate independent of concrete schedulers, executors, reactors, and platform registrations.
- Protocol operations receive scheduler capabilities and cancellation explicitly. Protocol objects retain resources and protocol state, without caching a scheduler.
- Prefer individual capabilities and implementation-associated resource and task-handle types. Preserve the owned-buffer and cooperative-cancellation contracts.
- Follow the shared Core conventions, testing, pull request, and release skills. Use the private `bake/` package for project tooling.
- Record intentional differences from the Socketry extraction in `context/design.md`. Keep concrete backend conformance tests in their own packages; test runtime-independent support and public generic composition here.

- Companion crates live under `crates/`. The conformance crate depends only on the interface and test support; direct uses conformance only as a development dependency. Workspace packages share versions and release notes.

- Follow `context/io.md` when writing protocol I/O, examples, or benchmarks. Reuse persistent monitors for borrowed I/O and use pooled receivers when completed data needs independent ownership.
