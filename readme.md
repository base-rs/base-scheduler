# `base-scheduler`

Explicit scheduler capabilities for reusable Rust protocol libraries. Applications choose an execution environment; libraries ask for the operations they need.

## Usage

Add the interface with `cargo add base-scheduler`. Applications also add a backend crate, such as `base-scheduler-native` or `base-scheduler-tokio`; reusable libraries depend only on the interface capabilities they use.

Construct readers and writers from the scheduler once, then pass cancellation to their operations. A protocol object can retain these bound capabilities alongside its parser and protocol state:

```rust
use base_scheduler::{Cancellation, OperationResult, SocketStreamReader};

struct Body<Reader> {
    reader: Reader,
}

impl<Reader: SocketStreamReader> Body<Reader> {
    async fn read_into(
        &mut self,
        buffer: &mut [u8],
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.reader.read_into(buffer, cancellation).await
    }
}
```

Create the reader with `scheduler.socket_stream_reader(&socket)`. Its scheduler is fixed at construction; cancellation belongs to each call. Moving the protocol object or its future between workers leaves registration, routing, and retirement to the selected backend.

See the [I/O usage guide](context/io.md), [interface reference](context/interface.md), and [runnable examples](examples/readme.md) for complete loops, buffer retention, partial transfers, cleanup, and the other scheduler capabilities.

## Releasing

Prepare a release from a clean checkout of `main` with `cargo bake cargo:release:patch` (or `minor`, `major`, or `bump --version X.Y.Z`). It bumps the version, commits it with a `Release: vVERSION` trailer, validates the candidate, pushes a release branch and opens the pull request. Its license update derives copyright lines from Git history and from `.contributors.toml`, which records work from before this repository, such as the original authors of `crates/timers/src/timers.rs`; add an entry there rather than editing a header by hand. After review and merge, GitHub Actions publishes the release when the configured `crates-io` environment approves it, then creates or updates the matching GitHub Release from `releases.md`. Follow the shared [Releasing skill](https://github.com/base-rs/base-project/blob/main/context/releasing.md) for the standard process.

## Releases

<!-- bake-readme:releases:start -->

See [releases.md](releases.md) for the full release history.

### v0.6.3

- Rename `Placement` to `Parent`, as Async's `parent:` names what owns the tasks a library starts, keeping `Placement` as an alias until 0.7. ([#145](https://github.com/base-rs/base-scheduler/pull/145))

### v0.6.2

- Add `Placement`, so a library can write its supervision once and let its caller choose whether work runs in place or as scope tasks. ([#143](https://github.com/base-rs/base-scheduler/pull/143))

### v0.6.1

- Add `Scope::scheduler`, so code given only a scope can use its scheduler's other capabilities. ([#141](https://github.com/base-rs/base-scheduler/pull/141))
- Generate release notes from pull request titles and `Breaking:` paragraphs. ([#140](https://github.com/base-rs/base-scheduler/pull/140))

<!-- bake-readme:releases:end -->

## See Also

The workspace also contains six companion crates, released together with the interface:

- [`base-scheduler-conformance`](crates/conformance/readme.md): reusable capability-specific assertions for backend development dependencies.
- [`base-scheduler-tokio`](crates/tokio/readme.md): an adapter for an explicitly selected Tokio runtime, preserving owning task handles, owned I/O, cancellation, and bounded native-work admission.
- [`base-scheduler-native`](crates/native/readme.md): Socketry's work-stealing executor and native readiness I/O, with Base ownership and shutdown semantics.
- [`base-scheduler-selector`](crates/selector/readme.md): independent I/O lifecycle contracts and the portable readiness selector.
- [`base-scheduler-iouring`](crates/iouring/readme.md): Linux completion file and socket I/O with a separately owned event driver and explicit readiness fallback.
- [`base-scheduler-timers`](crates/timers/readme.md): runtime-independent batched timers, reusable sleep registrations, and deadline notifications.

See the [design and behavioral contracts](context/design.md) for resource compatibility, shutdown, cancellation boundaries, and extraction decisions.

The [TCP echo benchmark](benchmarks/echo/readme.md) compares pure Tokio, the Tokio scheduler adapter, and Native with per-worker io_uring rings using separate server and client processes.

The [performance investigations](performance/index.md) record measured, rejected, and open optimization attempts, so that they are not retried without new evidence.

## Contributing

Please open an issue or pull request on [GitHub](https://github.com/base-rs/base-scheduler).

### Agent Context

Run `cargo bake agent:context:install` to install shared context and skills. Use `cargo bake agent:context:list` and `cargo bake agent:context:show` to discover relevant guides, follow `agents.md` if present, and apply skills under `.agents/skills/`. The installer preserves repository-owned `agents.md`; it does not create or regenerate that file.
