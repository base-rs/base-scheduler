# `core-scheduler`

Explicit scheduler capabilities for reusable Rust protocol libraries. Applications choose an execution environment; libraries ask for the operations they need.

The root crate defines interfaces and runtime-independent support. It contains no executor, reactor, operating-system I/O implementation, or runtime adapter. The initial API is extracted from [Socketry](https://github.com/core-rs/socketry-rust), and is not yet published to crates.io.

The workspace also contains seven companion crates, released together with the interface:

- [`core-scheduler-conformance`](crates/conformance/readme.md): reusable capability-specific assertions for backend development dependencies.
- [`core-scheduler-direct`](crates/direct/readme.md): reference implementations of tasks, clocks, native address resolution, file/socket operations, and platform-supported signals and filesystem extensions, with explicit service ownership and bounded syscall admission.
- [`core-scheduler-tokio`](crates/tokio/readme.md): an adapter for an explicitly selected Tokio runtime, preserving owning task handles, owned I/O, cancellation, and bounded native-work admission.
- [`core-scheduler-native`](crates/native/readme.md): Socketry's work-stealing executor and native readiness I/O, with Core ownership and shutdown semantics.
- [`core-scheduler-selector`](crates/selector/readme.md): independent I/O lifecycle contracts and the portable readiness selector.
- [`core-scheduler-iouring`](crates/iouring/readme.md): Linux completion file and socket I/O with a separately owned event driver and explicit readiness fallback.
- [`core-scheduler-timers`](crates/timers/readme.md): runtime-independent batched timers, reusable sleep registrations, and deadline notifications.

## Usage

The workspace is not published yet. Add the interface from Git with `cargo add core-scheduler --git https://github.com/core-rs/core-scheduler`. Applications select a companion backend; reusable libraries depend on the interface capabilities they use.

Construct readers and writers from the scheduler once, then pass cancellation to their operations. A protocol object can retain these bound capabilities alongside its parser and protocol state:

```rust
use core_scheduler::{Cancellation, OperationResult, SocketStreamReader};

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

## Socket and file I/O

The four socket handles are the normal application interface:

| Resource | Operations |
| --- | --- |
| `SocketStreamReader` | `read` returns an owned chunk; `read_into` and `read_vectored` use borrowed parser storage. |
| `SocketStreamWriter` | `write` and `write_vectored` make borrowed transfers; `submit` accepts owned output that progresses independently, and `flush` waits for all accepted bytes. |
| `SocketPacketReader` | `read` returns an owned message with source metadata; borrowed forms fill caller storage and report truncation. |
| `SocketPacketWriter` | Send one packet from borrowed scalar/vector storage or an owned buffer, with an optional destination. |

Construct them through `scheduler.socket_stream_reader`, `socket_stream_writer`, `socket_packet_reader`, and `socket_packet_writer`. Reuse one handle per sequential execution, with separate readers and writers for concurrent tasks. Borrowed-only readers do not allocate a receive pool. Owned reads initialize a default pool lazily, or use a shared pool supplied to `socket_stream_reader_with_pool` or `socket_packet_reader_with_pool`. Retained chunks can freeze into immutable views and move between tasks; their final owner returns storage for reuse.

`scheduler.file_reader(&file)` and `scheduler.file_writer(&file)` provide the same construction pattern for reusable owned-buffer file I/O. `read` and `write` use the OS cursor; `read_at` and `write_at` take explicit offsets. Handles have no private logical file position. File operations return the original owner alongside success, error, or cancellation. Join independent read and write futures to overlap a file pipeline.

Call `close().await` on each reader and writer before dropping it and exclusively closing its resource. Flush successful stream output first. Close has no cancellation argument and establishes backend retirement; dropping a handle is not an awaited cleanup barrier.

See the [I/O usage guide](context/io.md), [interface reference](context/interface.md), and [runnable examples](examples/readme.md) for complete loops, buffer retention, partial transfers, and cleanup. The backend selects persistent readiness, request reuse, and buffer registration behind these handles.

## Other capabilities

File and socket lifecycle operations remain explicit scheduler capabilities: open, bind, listen, connect, accept, options, metadata, synchronization, and close. Protocol libraries require the individual capabilities they use; there is no universal scheduler requirement.

| Capability | Contract |
| --- | --- |
| `Clock` | Monotonic observations and cancellable relative or absolute waits. |
| `Spawn` | Task dispatch with implementation-associated, owning completion handles. |
| `Blocking` | Bounded admission of owned synchronous work. |
| `Signals` | Notification subscriptions with implementation-associated guards and signal identifiers. |
| `BufferPooling` | Shareable bounded storage with retained immutable views and cancellable capacity waits. |
| Extension capabilities | Typed socket options, address resolution, standard TCP resource registration, file advice, and allocation. |

Operation futures use static dispatch and are `Send`; no boxed future or runtime-selection feature is required. Spawned futures and outputs are `Send + 'static`. Backend resource types and construction establish compatibility.

`Cancellation` is a runtime-independent cooperative signal. Clones share state, children inherit ancestor cancellation, and dropping a signal does not cancel it. Pre-cancellation prevents cancellable operation submission; successful progress can win a concurrent request. Cancellation does not roll back bytes or cursor changes. A task handle owns its execution lifetime: await it to observe completion, or drop it to request destruction after any active poll returns. Keep the scheduler and its drivers alive while accepted work retires.

## Development

Install the launcher with `cargo install bake --locked`. The private `bake/` workspace member supplies the shared Core development tasks:

```sh
cargo bake --locked test:coverage --all-targets true --all-features true
```

The tests cover runtime-independent support, generic composition, reusable conformance assertions, and the Direct, Tokio, Native, readiness, and io\_uring backends.

## Releasing

Prepare a release with `cargo bake cargo:version:patch` (or `minor`, `major`, or `bump --version X.Y.Z`), then run `cargo bake cargo:release` and open a pull request. After review and merge, GitHub Actions publishes the release when the configured `crates-io` environment approves it. Follow the shared [Releasing skill](https://github.com/core-rs/core-project/blob/main/context/releasing.md) for the standard process.

## Releases

The workspace has not been released. See [release notes](releases.md) for changes awaiting the first release.

## See Also

See the [design and behavioral contracts](context/design.md) for resource compatibility, shutdown, cancellation boundaries, and extraction decisions.

The [TCP echo benchmark](benchmarks/echo/readme.md) compares pure Tokio, the Tokio scheduler adapter, and Native with per-worker io\_uring rings using separate server and client processes.

## Contributing

Please open an issue or pull request on [GitHub](https://github.com/core-rs/core-scheduler).

### Agent Context

Run `cargo bake agent:context:install` to install shared context and skills. Use `cargo bake agent:context:list` and `cargo bake agent:context:show` to discover guidance. Follow `agents.md` and the relevant skills under `.agents/skills/`.
