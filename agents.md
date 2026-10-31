# Agent instructions

Run `cargo bake agent:context:install` to install dependency context and skills. Use `cargo bake agent:context:list` and `cargo bake agent:context:show` to discover guidance. Follow `.agents/conventions.md` and the relevant installed Base skills.

Read `context/design.md` before changing the public capabilities or behavioral contracts.

The workspace tests cover runtime-independent support, generic composition, reusable conformance assertions, the Direct, Tokio, Native, readiness, and io_uring backends, and the reusable timers. CI requires 100% region coverage for the workspace with `cargo bake --locked test:coverage --all-targets true --all-features true`, and separately for `base-scheduler-timers` with default features and with each of the `driver`, `sharded`, and `local` features (`cargo bake --locked test:coverage --package base-scheduler-timers --all-targets true --features driver`, for example); see `.github/workflows/test.yml`.
