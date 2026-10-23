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

The workspace is not published yet. Add the interface from Git with `cargo add core-scheduler --git https://github.com/core-rs/core-scheduler`. Applications also select a companion backend; reusable libraries require only the interface capabilities they use.

Keep resources and protocol state in an object, and pass the scheduler and cancellation to each operation. The resulting future borrows that operation's execution context:

```rust
use core_scheduler::{BufferResult, Cancellation, SocketRead};

struct Body<Socket> {
    socket: Socket,
}

impl<Socket> Body<Socket> {
    async fn read<S>(
        &self,
        scheduler: &S,
        cancellation: &Cancellation,
        buffer: Vec<u8>,
    ) -> BufferResult
    where
        S: SocketRead<Socket = Socket>,
    {
        scheduler.socket_read(&self.socket, buffer, cancellation).await
    }
}
```

This small example illustrates owned-buffer dispatch. The [runnable I/O examples](examples/readme.md) show complete loops that reuse monitors and storage, retain parsed frames, hand off packets, and overlap file operations.

The body retains no scheduler. Its next operation can use a different compatible scheduler. Associated resource types and backend registration rules determine compatibility; passing a new scheduler does not rebind an outstanding future or an incompatible resource.

## Capabilities

Require individual operations, such as `FileRead`, `SocketRead + SocketWrite`, or `Clock`. `FileScheduler` and `SocketScheduler` aggregate their core operations for consumers that need the complete capability set.

| Capability | Contract |
| --- | --- |
| File operations | Open, close, metadata, cursor and positioned I/O, truncation, and synchronization, using an associated file type. |
| Socket operations | Open, bind, listen, accept, connect, close, shutdown, readiness, and I/O, using an associated socket type. |
| `Clock` | Monotonic observations and cancellable relative or absolute waits. |
| `Spawn` | Primitive task dispatch with implementation-associated, owning completion handles. |
| `Blocking` | Optional bounded admission of owned synchronous work. |
| `Signals` | Optional notification subscriptions with implementation-associated guards and signal identifiers. |
| Borrowed socket I/O | Optional `SocketRecvPoll`, `SocketSendPoll`, `SocketRecvMsgPoll`, and `SocketSendMsgPoll`, accepting borrowed slices and I/O vectors. |
| `BufferPooling` and `SocketReceiving` | Experimental bounded reusable storage and owned received chunks, with retained parser views and backend-selected receive strategies. |
| Extension capabilities | Typed socket options, scatter/gather messages, address resolution, standard TCP resource registration, file advice, and allocation. |

Operation futures use static dispatch and are `Send`. File and socket resources are `Send + Sync`; spawned futures and outputs are `Send + 'static`. Local-only execution is outside this initial contract. Independent capabilities can be implemented separately without runtime-selection features.

Buffer consumers choose a shared payload budget and alignment through `scheduler.buffer_pool(options)`, then pass that pool to `scheduler.socket_receiver(socket, pool, flags, cancellation)`. Completed chunks are uniquely writable and can freeze into immutable views that outlive the receiver and move between workers. Retained views prevent reuse; exhaustion waits with cancellation. The backend owns allocation, hot-buffer reuse, registration, publication depth, and receive strategy. Changing worker count does not multiply the portable pool's capacity. Admitted receivers may receive ahead within the budget, so close them explicitly before exclusively closing their sockets. See the [buffer ownership design](context/design.md#experimental-shared-buffer-pools-and-receivers) and [buffer experiments](benchmarks/buffers/readme.md).

Received messages expose `address()` and `payload()` while owning their backend's storage and metadata. Convert an address view with `to_socket_address()` when a portable endpoint is needed. Pooled messages support `payload_mut()` for parsing or decryption and `into_payload()` to transfer an owned buffer into a write or freeze it into retained views. These accessors specify contents and ownership; allocation, copying, and native metadata representation remain implementation choices.

## Reusable file monitors

`FileMonitoring` is an experimental capability for sequential owned-buffer reads and writes. Construct one monitor per sequential execution; separate monitors support concurrent operations on the same file. Retain it across operations to reuse backend request state. io_uring also caches an optional fixed-file registration for the current ring. A subsequent operation on another worker releases and replaces that registration while reusing request storage:

```rust
use core_scheduler::{Cancellation, FileMonitoring, OperationResult};

async fn read_file<S: FileMonitoring>(
    scheduler: &S,
    file: &S::File,
    cancellation: &Cancellation,
    mut consume: impl FnMut(&[u8]),
) -> OperationResult<()> {
    let mut reader = scheduler.file_monitor(file)?;
    let mut buffer = vec![0; 65536];
    let result = async {
        loop {
            let (read, returned) = scheduler
                .file_monitor_read(&mut reader, buffer, cancellation)
                .await;
            buffer = returned;
            let count = read?;
            if count == 0 {
                return Ok(());
            }
            consume(&buffer[..count]);
        }
    }.await;
    scheduler.file_monitor_close(&mut reader).await?;
    result
}
```

A monitor can alternate reads and writes. Independent monitors share the underlying file cursor; use positioned operations for independent offsets. Close and drop every monitor before exclusively closing the file. After an operation future is dropped, reuse can return `WouldBlock` until backend access retires. The ordinary file operations remain available without monitor state. See the [file benchmark](benchmarks/files/readme.md) for measured costs and benefits.

## Reusable socket monitors

`SocketMonitoring` supplies reusable directional readiness state. Use `socket_monitor` for general socket I/O, including UDP, or `socket_stream_monitor` to enable backend-specific byte-stream optimizations. Both use the same operations:

```rust
use core_scheduler::{Cancellation, Interest, OperationResult, SocketMonitoring, SocketRecvFlags};

async fn receive<S: SocketMonitoring>(
    scheduler: &S,
    socket: &S::Socket,
    cancellation: &Cancellation,
    mut consume: impl FnMut(&[u8]),
) -> OperationResult<()>
where
    S::Socket: 'static,
{
    let mut reader = scheduler.socket_stream_monitor(socket, Interest::Readable)?;
    let mut buffer = vec![0; 65536];
    let result = async {
        loop {
            let count = scheduler
                .socket_monitor_recv_poll(&mut reader, &mut buffer, SocketRecvFlags::default(), cancellation)
                .await?;
            if count == 0 {
                return Ok(());
            }
            consume(&buffer[..count]);
        }
    }.await;
    scheduler.socket_monitor_close(&mut reader).await?;
    result
}
```

Retain the monitor across operations to reuse its registration. Create separate readable and writable monitors for concurrent tasks. `SocketStreamMonitor` wraps the common state without another allocation; `socket_stream_monitor_state` and `MonitorStorage::from_arc` also let you embed it in shared connection state. It does not imply connectedness or a universal rule for short reads. Scheduler and cancellation remain explicit operation arguments.

## Receiving message sequences

`SocketReceiving` constructs a receiver from an explicit scheduler-created pool. `recv(scheduler, cancellation)` returns owned messages exposing payload and address views; completed storage can be parsed, frozen, and retained independently of further receives. The backend chooses demand-driven or receive-ahead behavior. For shutdown, cancel and await the pending receive, await `receiver.close(scheduler)`, then drop the receiver before exclusively closing its socket. Monitor and receiver close methods take no cancellation token. The concrete io_uring multishot constructor remains a backend experiment.

The [scheduler interface reference](context/interface.md) lists every portable capability, its arguments and result semantics, and the separate backend driver hooks.

## Cancellation, ownership, and completion

`Cancellation` is a runtime-independent cooperative signal. Clones share state, children inherit ancestor cancellation, and dropping a signal does not cancel it. Cancellation requests and completed cleanup are separate events.

Owned I/O buffers keep a stable, initialized allocation. Operations return the original owner with success, error, or cancellation. A backend must retain buffers and resources until access ends, including when a caller drops the future. Reads and writes can be partial; there is no implicit read-exact or write-all behavior.

Borrowed socket `_poll` operations are also ordinary async methods: callers `.await` them with explicit cancellation. Each payload access finishes before a poll returns; pending readiness waits retain no payload pointers. Their buffers and `IoSlice` arrays can therefore live on the stack, and dropping or forgetting a pending future leaves no outstanding buffer access. Windows implementations can enable nonblocking mode and leave it enabled; aliases must not concurrently restore blocking mode. Owned operations remain available for completion-based dispatch.

Operation futures are lazy unless explicitly documented otherwise. Pre-cancellation prevents cancellable operation submission. Successful I/O progress wins a concurrent cancellation request; abandoning an operation cannot roll back its effects. Cleanup operations intentionally take no cancellation token so they can finish during graceful shutdown. `Spawn::spawn` and signal subscription register synchronously.

A task handle owns its task: moving transfers ownership, awaiting observes completion, and dropping requests destruction after any active poll returns. Runtime adapters must preserve this contract, including when their host runtime normally detaches on drop. The interface exposes no forced-abort method or implicit task hierarchy.

## Development

Install the launcher with `cargo install bake --locked`. The private `bake/` workspace member supplies the shared Core development tasks:

```sh
cargo bake --locked test:coverage --all-targets true --all-features true
```

The tests cover runtime-independent support, generic composition, reusable conformance assertions, and the Direct, Tokio, Native, readiness, and io_uring backends.

## Releasing

Prepare a release with `cargo bake cargo:version:patch` (or `minor`, `major`, or `bump --version X.Y.Z`), then run `cargo bake cargo:release` and open a pull request. After review and merge, GitHub Actions publishes the release when the configured `crates-io` environment approves it. Follow the shared [Releasing skill](https://github.com/core-rs/core-project/blob/main/context/releasing.md) for the standard process.

## Releases

The workspace has not been released. See [release notes](releases.md) for changes awaiting the first release.

## See Also

See the [design and behavioral contracts](context/design.md) for resource compatibility, shutdown, cancellation boundaries, and extraction decisions.

The [TCP echo benchmark](benchmarks/echo/readme.md) compares pure Tokio, the Tokio scheduler adapter, and Native with per-worker io_uring rings using separate server and client processes.

## Contributing

Please open an issue or pull request on [GitHub](https://github.com/core-rs/core-scheduler).

### Agent Context

Run `cargo bake agent:context:install` to install shared context and skills. Use `cargo bake agent:context:list` and `cargo bake agent:context:show` to discover guidance. Follow `agents.md` and the relevant skills under `.agents/skills/`.
