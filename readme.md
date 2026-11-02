# `base-scheduler`

Explicit scheduler capabilities for reusable Rust protocol libraries. Applications choose an execution environment; libraries ask for the operations they need.

## Usage

Add the interface with `cargo add base-scheduler`. Applications also add a backend crate, such as `base-scheduler-direct`, `base-scheduler-tokio`, or `base-scheduler-native`; reusable libraries depend only on the interface capabilities they use.

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

Prepare a release with `cargo bake cargo:version:patch` (or `minor`, `major`, or `bump --version X.Y.Z`), commit it with a `Release: vVERSION` trailer, then run `cargo bake cargo:release` and open a pull request. After review and merge, GitHub Actions publishes the release when the configured `crates-io` environment approves it, then creates or updates the matching GitHub Release from `releases.md`. Follow the shared [Releasing skill](https://github.com/base-rs/base-project/blob/main/context/releasing.md) for the standard process.

## Releases

<!-- bake-readme:releases:start -->

See [releases.md](releases.md) for the full release history.

### v0.4.0

- **Breaking:** add the required `SocketStreamReader::try_read_into`, which attempts one borrowed read now without waiting or consulting cached readiness. Polling `read_into` once with a no-op waker is not equivalent: Native, readiness and Tokio readers that have established progress wait on cached readiness first, so a single poll can miss input or EOF the kernel has already queued. Implementations of `SocketStreamReader` outside this workspace must add the method; wrappers should forward it.
- Use Native's own epoll readiness driver on Android, as on Linux, rather than the portable `polling` fallback. Android stream readers therefore follow the Linux readiness rules: without OOB support, a short receive clears readiness and reported urgent data returns `Unsupported`.
- Fix blocking pools on Apple platforms that could stop running jobs, permanently: the readiness selector's pool, which Native uses, and the Tokio adapter's pool. There, the default pthread condition variable stops waking anyone once one waiter has been passed over for 2^23 newer waits ([ioquatix/macos-pthread-cond-hang](https://github.com/ioquatix/macos-pthread-cond-hang)), which an idle worker in a busy pool can be. Idle workers now wait again every second. The direct scheduler's pool, which stopped that way in [#83](https://github.com/base-rs/base-scheduler/issues/83), is unchanged.
- Fix Tokio adapter socket waits on Windows that could hang although the socket was ready. async-io's IOCP reactor can miss interest registered from another thread ([smol-rs/polling#271](https://github.com/smol-rs/polling/issues/271)); each wait now wakes the reactor after registering interest and retries its syscall at least every 10 seconds.

### v0.3.1

- Add Unix `PipeReading` and `PipeWriting` capabilities which adopt an owned pipe descriptor, such as a child process's standard input or output, as a sequential reader or writer. Direct, readiness, Tokio and Native support them natively; io_uring uses its readiness fallback.

### v0.3.0

- Register child cancellation signals in reusable slots instead of a hash map. Creating and dropping a child no longer hashes, and steady churn does not grow the parent's registry. In the HTTP client benchmark at 32 concurrent requests, this saves about 245 user instructions per response.

- Reduce Native readiness overhead on established stream reads: skip redundant notifications for directions that are already ready, bypass cancellation wrappers for never-cancelled operations, and poll a compact receive future. Native sockets now register with the portable reactor only when a fallback wait first needs it, so `NativeSocket::retain_native` returns the new `RegisteredSocket`, which exposes the socket and its descriptor but not async-io's readiness methods. This is a breaking change for backend integrations, and a reactor registration failure surfaces at that wait.

<!-- bake-readme:releases:end -->

## See Also

The workspace also contains seven companion crates, released together with the interface:

- [`base-scheduler-conformance`](crates/conformance/readme.md): reusable capability-specific assertions for backend development dependencies.
- [`base-scheduler-direct`](crates/direct/readme.md): reference implementations of tasks, clocks, native address resolution, file/socket operations, and platform-supported signals and filesystem extensions, with explicit service ownership and bounded syscall admission.
- [`base-scheduler-tokio`](crates/tokio/readme.md): an adapter for an explicitly selected Tokio runtime, preserving owning task handles, owned I/O, cancellation, and bounded native-work admission.
- [`base-scheduler-native`](crates/native/readme.md): Socketry's work-stealing executor and native readiness I/O, with Base ownership and shutdown semantics.
- [`base-scheduler-selector`](crates/selector/readme.md): independent I/O lifecycle contracts and the portable readiness selector.
- [`base-scheduler-iouring`](crates/iouring/readme.md): Linux completion file and socket I/O with a separately owned event driver and explicit readiness fallback.
- [`base-scheduler-timers`](crates/timers/readme.md): runtime-independent batched timers, reusable sleep registrations, and deadline notifications.

See the [design and behavioral contracts](context/design.md) for resource compatibility, shutdown, cancellation boundaries, and extraction decisions.

The [TCP echo benchmark](benchmarks/echo/readme.md) compares pure Tokio, the Tokio scheduler adapter, and Native with per-worker io_uring rings using separate server and client processes.

## Contributing

Please open an issue or pull request on [GitHub](https://github.com/base-rs/base-scheduler).

### Agent Context

Run `cargo bake agent:context:install` to install shared context and skills. Use `cargo bake agent:context:list` and `cargo bake agent:context:show` to discover relevant guides, follow `agents.md` if present, and apply skills under `.agents/skills/`. The installer preserves repository-owned `agents.md`; it does not create or regenerate that file.
