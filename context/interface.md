# Scheduler interface reference

This is an inventory of the portable `core-scheduler` capabilities and the separate selector integration hooks. The linked Rust traits define the exact signatures; [design](design.md) records the behavioral contracts and implementation decisions. Applications choose concrete schedulers. Protocol libraries request only the capabilities they use. Start with [I/O usage guidance](io.md) when choosing a composition; this reference inventories the available hooks.

## Reading the signatures

Tables omit `&self` and spell an asynchronous method's **awaited result**, not its enclosing `impl Future + Send`. `S` is the explicitly supplied scheduler, `C` is `&Cancellation`, `F` is `&S::File`, and `K` is `&S::Socket`. `B` is an owned stable buffer; `V` is an owned sequence of such buffers. An `&mut` argument is exclusively borrowed for the operation future's lifetime.

- `OperationResult<T>` is `Result<T, OperationError>`, where `OperationError` is `Cancelled(Cancelled)` or `Io(io::Error)`. Its `kind()` maps cancellation to `Interrupted`; match the variant to distinguish cancellation from an OS interruption. `raw_os_error()` preserves native I/O codes when present.
- `BufferResult<B>` is `(OperationResult<usize>, B)`. The original owner is returned on every normally completed result, including error and cancellation. It is outside the `Result` deliberately.
- Uncancellable cleanup uses `io::Result<T>`. This reports failure and does not itself guarantee successful retirement when `Err` is returned.
- Operations are lazy unless explicitly documented otherwise. Constructing or dropping an unpolled operation submits no work. Synchronous factories, `spawn`, and signal subscription are explicit exceptions.
- Pre-cancellation prevents cancellable submission. After submission, cancellation is cooperative: progress can win, and an already-running blocking syscall may have to finish. Cancellation is not rollback.
- Owned operations retain buffers and resources until backend access retires, even if a caller drops or forgets its future. Abandonment forfeits recovery of the owner; it must never free memory still in use.
- Borrowed socket operations access buffers only synchronously inside a poll. A pending future never leaves an independent kernel or worker operation accessing those borrowed bytes.
- Operation futures are `Send`; file/socket resources and their capabilities are `Send + Sync`. Spawned work and owned buffers are `Send + 'static`. Local-only execution is outside this interface. Generic associated types and return-position `impl Future` support static dispatch without mandatory boxed futures.
- Scheduler arguments remain explicit. Resource types and instance affinity still constrain compatibility; passing a different scheduler does not rebind outstanding work. Backends reject incompatible pairings before submission with `InvalidInput` where types cannot reject them.

## Capability composition and resource types

[`FileOperation`](https://github.com/core-rs/core-scheduler/blob/main/src/file_operation.rs) supplies `type File: Send + Sync`. [`SocketOperation`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_operation.rs) supplies `type Socket: Send + Sync`. These types can retain native ownership and registration state; the interface does not require a particular descriptor or reference-counted wrapper.

[`FileScheduler`](https://github.com/core-rs/core-scheduler/blob/main/src/file_scheduler.rs) aggregates open, close, stat, truncate, sync, seek, read/write, and positioned read/write. [`SocketScheduler`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_scheduler.rs) aggregates open, close, shutdown, bind, listen, local/peer addresses, connect, accept, read/write, and readiness wait. They are blanket conveniences, not universal requirements. [`SocketMessages`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_messages.rs) combines owned send/receive messages; [`SocketOptions<O>`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_options.rs) combines get/set for one option type. Optional capabilities remain explicit bounds.

## Cancellation, tasks, blocking work, and signals

| Hook | Arguments → result | Semantics |
| --- | --- | --- |
| `Cancellation::new`, `default` | `() → Cancellation` | Independent uncancelled signal. |
| `Cancellation::never` | `() → Cancellation` | Allocation-free signal that cannot be cancelled. |
| `clone`, `child` | `&Cancellation → Cancellation` | Clone shares the signal; a child receives ancestor cancellation but does not cancel ancestors or siblings. A child of `never` is independently cancellable. |
| `cancel` | `() → bool` | Synchronously request cancellation of this signal and descendants; true only on its first request. Does not await their work. |
| `is_cancelled`, `check` | `() → bool` / `Result<(), Cancelled>` | Observe the persistent request. Dropping a token never requests cancellation. |
| `cancelled` | `() → ()` asynchronously | Wait for notification; dropping the wait removes its listener only. |
| `defer_cancel` | `(C, future, on_cancel: FnOnce()) → future output` | Invoke the synchronous callback once when cancellation is observed, then keep awaiting the protected future. Does not prevent the wrapper being dropped. |
| [`Spawn::spawn`](https://github.com/core-rs/core-scheduler/blob/main/src/spawn.rs) | `(future) → Result<Handle<T>, SpawnError>` synchronously | Register `Send + 'static` work before making it runnable. Handle owns execution lifetime; dropping it requests destruction after an active poll returns. Await yields `Result<T, TaskError>`. Closed admission rejects the future without polling it. |
| [`Blocking::spawn_blocking`](https://github.com/core-rs/core-scheduler/blob/main/src/blocking.rs) | `(FnOnce() → T, C) → Result<Handle<T>, BlockingError>` asynchronously | Await bounded admission, then return an owning completion handle. Queued plus running jobs count against capacity. Dropping a handle cannot interrupt a running closure. A worker checks cancellation before starting; running code can capture a token to cooperate. |
| [`Signals::signal_subscribe`](https://github.com/core-rs/core-scheduler/blob/main/src/signals.rs) | `(&[Signal], target: Notify<Signal>) → io::Result<Subscription>` synchronously | Associated signal identifiers and owning subscription. Dropping the subscription stops new notifications; in-progress dispatch can finish. Notifications can coalesce. |
| [`Notify::notify`](https://github.com/core-rs/core-scheduler/blob/main/src/notify.rs) | `(value) → ()` synchronously | Shared notification hook (`Notify<Value>: Send + Sync`); `Arc` targets, standard-library channel senders, and cancellation signals have implementations. Signal callbacks run outside raw signal handlers and scheduler locks. |

`TaskError` distinguishes cancellation from a retained panic payload. `SpawnError` reports closed admission or exhausted task identifiers. `BlockingError` distinguishes cancellation, closed admission, and worker-start I/O failure. Task ownership and cancellation signals are separate: background work needs an owner that remains alive, and the primitive interface supplies no implicit parent-child task group.

## Time

| Capability/hook | Arguments → result | Semantics |
| --- | --- | --- |
| [`Clock::now`](https://github.com/core-rs/core-scheduler/blob/main/src/clock.rs) | `() → Instant` synchronously | Monotonic time within this scheduler's clock domain; use it in generic code instead of the system clock. |
| `sleep_until` | `(deadline: Instant, C) → OperationResult<()>` | Wait for an absolute deadline. Pre-cancellation wins even if elapsed; once waiting, observed expiry wins a simultaneous cancellation. No yield is promised for an elapsed deadline. |
| `sleep` | `(duration: Duration, C) → OperationResult<()>` | Relative deadline starts at first poll. Unrepresentable deadlines return `InvalidInput`. |
| [`Timers::timer`](https://github.com/core-rs/core-scheduler/blob/main/src/timers.rs) | `() → Timer` synchronously | Infallible construction of scheduler-compatible reusable state, without registering a deadline or selecting a worker. |
| `timer_wait_until` | `(&mut Timer, deadline: Instant, C) → OperationResult<()>` | Reuse timer state with one waiter. Incompatible scheduler pairing is rejected. Cancellation after admission disarms; pre-cancellation leaves state unchanged. |

Dropping an ordinary clock wait releases its registration. Reusable timer waits deliberately retain state for later reuse, potentially retaining capacity and an old waker. Dropping the timer retires it; forgetting it can leak capacity. Reusable timers remain experimental. Deadlines from independently advancing clock domains are not interchangeable.

## Files

| Capability/hook | Arguments → awaited result | Semantics |
| --- | --- | --- |
| [`FileOpen::file_open`](https://github.com/core-rs/core-scheduler/blob/main/src/file_open.rs) | `(&Path, FileOptions, C) → OperationResult<File>` | Options expose read, write, create, create-new, and truncate; append is omitted. |
| [`FileRead::file_read`](https://github.com/core-rs/core-scheduler/blob/main/src/file_read.rs) | `(F, B: IoBufMut, C) → BufferResult<B>` | One read into existing initialized bytes. Short counts are preserved; nonempty reads returning zero indicate EOF. |
| [`FileWrite::file_write`](https://github.com/core-rs/core-scheduler/blob/main/src/file_write.rs) | `(F, B: IoBuf, C) → BufferResult<B>` | One potentially partial write. Success does not establish durability. |
| [`FileReadAt::file_read_at`](https://github.com/core-rs/core-scheduler/blob/main/src/file_read_at.rs) / [`FileWriteAt::file_write_at`](https://github.com/core-rs/core-scheduler/blob/main/src/file_write_at.rs) | `(F, B, offset: u64, C) → BufferResult<B>` | Positioned variants; read requires `IoBufMut`, write requires `IoBuf`. Shared-cursor effects follow the platform rules documented by these traits. |
| [`FileSeek::file_seek`](https://github.com/core-rs/core-scheduler/blob/main/src/file_seek.rs) | `(F, SeekFrom, C) → OperationResult<u64>` | Set the shared cursor and return its new position. Serialize dependent cursor operations. |
| [`FileStat::file_stat`](https://github.com/core-rs/core-scheduler/blob/main/src/file_stat.rs) | `(F, C) → OperationResult<Metadata>` | Associated owned metadata; an observation rather than a globally atomic snapshot. |
| [`FileTruncate::file_truncate`](https://github.com/core-rs/core-scheduler/blob/main/src/file_truncate.rs) | `(F, length: u64, C) → OperationResult<()>` | Change logical size without changing the cursor. |
| [`FileSync::file_sync`](https://github.com/core-rs/core-scheduler/blob/main/src/file_sync.rs) | `(F, SyncMode, C) → OperationResult<()>` | Synchronize data or data plus metadata. Await prior writes first. Does not flush application buffers or synchronize a parent directory. |
| [`FileAllocate::file_allocate`](https://github.com/core-rs/core-scheduler/blob/main/src/file_allocate.rs) | `(F, offset: u64, length: u64, AllocationMode, C) → OperationResult<()>` | Reserve backing storage, optionally preserving logical size. Filesystem support and errors remain observable; not a durability barrier. |
| [`FileAdvice::file_advise`](https://github.com/core-rs/core-scheduler/blob/main/src/file_advice.rs) | `(F, offset: u64, length: u64, AdviceMode) → io::Result<()>` | Uncancellable hint, awaited through OS completion. Zero length means through EOF. |
| `file_advise_submit` | same arguments → `io::Result<()>` | Await admission only; execution errors are not returned. Accepted work survives abandonment. |
| [`FileClose::file_close`](https://github.com/core-rs/core-scheduler/blob/main/src/file_close.rs) | `(File by value) → io::Result<()>` | Consume exclusive ownership and await admission plus OS close. Finish operations and drop monitors first. Not retried after close errors. |
| `file_close_submit` | `(File by value) → io::Result<()>` | Admission only; accepted cleanup is drained by graceful shutdown. |

[`FileMetadata`](https://github.com/core-rs/core-scheduler/blob/main/src/file_metadata.rs) exposes synchronous `file_type() → Option<FileType>`, `size() → Option<u64>`, and `accessed()`, `modified()`, `created() → Option<SystemTime>`. Absence represents unavailable information. Backends can retain native metadata and compute accessors without eagerly copying every field.

[`file_write_all(S, F, B, C)`](https://github.com/core-rs/core-scheduler/blob/main/src/file_write_all.rs) is a portable helper returning `BufferResult<B>`. It sequences partial writes, advances owned views, and reports zero progress as `WriteZero`. Errors may follow partial effects; the original owner is always returned when the helper completes.

## File monitors and explicit submission

[`FileMonitoring`](https://github.com/core-rs/core-scheduler/blob/main/src/file_monitoring.rs) associates `FileRegistration: Send` and provides:

| Hook | Arguments → result |
| --- | --- |
| `file_monitor` | `(F) → io::Result<FileMonitor<File, FileRegistration>>` synchronously |
| `file_monitor_read`, `file_monitor_write` | `(&mut monitor, B, C) → BufferResult<B>` asynchronously |
| `file_monitor_read_at`, `file_monitor_write_at` | `(&mut monitor, B, offset: u64, C) → BufferResult<B>` asynchronously |
| `file_monitor_close` | `(&mut monitor) → io::Result<()>` asynchronously |

One monitor describes one sequential operation stream. Different monitors can share a file; use positioned operations for independent offsets. Abandoned operations can outlive their futures, so reuse returns `WouldBlock` until retirement. Close stops admission and awaits retirement; drop the monitor afterwards to release its file owner.

[`FileSubmission`](https://github.com/core-rs/core-scheduler/blob/main/src/file_submission.rs) separates admission from completion. `file_read_submit(F, B, C)`, `file_write_submit(F, B, C)`, and the `_at_submit(F, B, offset, C)` variants await to:

```rust,ignore
Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)>
```

Admission failure returns the buffer immediately. After successful admission the operation can progress without polling its returned completion future. Admission does not mean the kernel has started or finished the operation. Await completion to recover the owner. `file_write_all_submit(S, F, B, C)` admits the first write and returns a completion future that handles subsequent partial writes when polled. Joining ordinary read/write futures is the ordinary file-pipeline choice; use submission when the acceptance boundary matters.

## Socket creation, control, and resolution

| Capability/hook | Arguments → result | Semantics |
| --- | --- | --- |
| [`SocketOpen::socket_open`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_open.rs) | `(SocketDomain, SocketType, Option<SocketProtocol>, C) → OperationResult<Socket>` asynchronously | Open nonblocking, without explicit bind/listen/connect. `None` selects the OS default protocol. |
| [`SocketBind::socket_bind`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_bind.rs) | `(K, SocketAddress, C) → OperationResult<()>` asynchronously | Bind an existing socket. |
| [`SocketListen::socket_listen`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_listen.rs) | `(K, backlog: i32, C) → OperationResult<()>` asynchronously | Begin listening; OS backlog and implicit-bind rules remain observable. |
| [`SocketConnect::socket_connect`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_connect.rs) | `(K, SocketAddress, C) → OperationResult<()>` asynchronously | Connect; cancellation does not undo an attempted connection. |
| [`SocketAccept::socket_accept`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_accept.rs) | `(K, C) → OperationResult<(Socket, SocketAddress)>` asynchronously | Return a new socket and peer address. |
| [`SocketLocalAddress::socket_local_address`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_local_address.rs) / [`SocketPeerAddress::socket_peer_address`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_peer_address.rs) | `(K) → io::Result<SocketAddress>` synchronously | Inspect local or connected peer endpoint. |
| [`SocketWait::socket_wait`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_wait.rs) | `(K, Interest, C) → OperationResult<()>` asynchronously | Advisory readable/writable notification; consume no data and retry nonblocking I/O afterwards. |
| [`SocketShutdown::socket_shutdown`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_shutdown.rs) | `(K, Shutdown) → io::Result<()>` asynchronously | Shutdown read, write, or both directions through completion; no cancellation token. |
| `socket_shutdown_submit` | same arguments → `io::Result<()>` | Await admission only. |
| [`SocketClose::socket_close`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_close.rs) | `(Socket by value) → io::Result<()>` asynchronously | Close exclusive ownership through completion; no cancellation token. |
| `socket_close_submit` | `(Socket by value) → io::Result<()>` asynchronously | Await admission only. |
| [`SocketRegistration::register_socket`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_registration.rs) / `register_listener` | `(TcpStream)` / `(TcpListener) → io::Result<Socket>` synchronously | Transfer an existing standard-library resource into the backend. |
| [`SocketGetOption<O>::socket_get_option`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_get_option.rs) | `(K, option: O, C) → OperationResult<O::Value>` asynchronously | Query a typed option. |
| [`SocketSetOption<O>::socket_set_option`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_set_option.rs) | `(K, option: O, value: O::Value, C) → OperationResult<()>` asynchronously | Set a typed option. |
| [`AddressResolution::address_resolve`](https://github.com/core-rs/core-scheduler/blob/main/src/address_resolution.rs) | `(name: Option<&str>, AddressService, AddressHints, C) → OperationResult<Addresses>` asynchronously | Owned associated resolution result; independent of connect policy. Preserve ordering, duplicates, socket type, protocol, and IPv6 scope. |

`SocketAddress` is the standard-library IP `SocketAddr`; this is not a Unix-domain or arbitrary native-address interface. `SocketOption` associates a `Value: Send`; supplied boolean options are `TcpNoDelay`, `SocketReuseAddress`, and `SocketKeepalive`. Implementations may support additional typed options independently.

`AddressService` selects a port or named service. Resolver errors are `AddressResolveError` carried inside `OperationError::Io`, preserving a portable category and native code when available. [`AddressInfoCollection::entries`](https://github.com/core-rs/core-scheduler/blob/main/src/address_info_collection.rs) returns borrowed `AddressInfoView` entries exposing `address`, `socket_type`, `protocol`, and explicit `to_address_info` materialization. The result owner can retain the original native address list.

## Owned and borrowed socket I/O

| Hook | Arguments → awaited result |
| --- | --- |
| [`SocketRead::socket_read`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_read.rs) | `(K, B: IoBufMut, C) → BufferResult<B>` |
| [`SocketWrite::socket_write`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_write.rs) | `(K, B: IoBuf, C) → BufferResult<B>` |
| [`SocketRecvMsg::socket_recvmsg`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_recv_msg.rs) | `(K, V: IoBufSequenceMut, SocketRecvFlags, C) → (OperationResult<SocketRecvResult>, V)` |
| [`SocketSendMsg::socket_sendmsg`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_send_msg.rs) | `(K, SocketMessage<V: IoBufSequence>, SocketSendFlags, C) → (OperationResult<usize>, SocketMessage<V>)` |
| [`SocketRecvPoll::socket_recv_poll`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_recv_poll.rs) | `(K, &mut [u8], SocketRecvFlags, C) → OperationResult<usize>` |
| [`SocketSendPoll::socket_send_poll`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_send_poll.rs) | `(K, &[u8], SocketSendFlags, C) → OperationResult<usize>` |
| [`SocketRecvMsgPoll::socket_recvmsg_poll`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_recv_msg_poll.rs) | `(K, &mut [IoSliceMut], SocketRecvFlags, C) → OperationResult<SocketRecvResult>` |
| [`SocketSendMsgPoll::socket_sendmsg_poll`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_send_msg_poll.rs) | `(K, &[IoSlice], Option<SocketAddress>, SocketSendFlags, C) → OperationResult<usize>` |

`SocketRecvFlags::peek` requests `MSG_PEEK`: inspect queued bytes without consuming or reserving them. Receivers reject peeking because they consume a sequence. `SocketSendFlags::dont_route` requests `MSG_DONTROUTE`; the protocol may reject or ignore it. Both flags default to false.

These are single transfers, not read-exact or write-all operations. Owned reads fill initialized ranges without resizing; writes may be partial. `SocketMessage` owns both its vector collection and optional destination (`None` uses the connected peer). `SocketRecvResult` contains copied byte count, optional source address, payload truncation, and ancillary-data truncation. Ancillary bytes themselves are not exposed. Zero bytes distinguish EOF only for streams with nonempty receive capacity; empty datagrams are real messages.

The `_poll` methods are ordinary awaitable futures, not raw `Context → Poll` hooks. They enforce nonblocking attempts: Unix uses `MSG_DONTWAIT`; Windows sets `FIONBIO` and leaves the socket nonblocking. External aliases must not concurrently restore blocking mode. Message receive attempts I/O on the first poll after validation; consumed bytes, EOF, and empty datagrams are returned in that same poll. `Pending` consumes nothing. Scalar receive can use readiness-first optimizations. This permits stack buffers while retaining safe abandonment.

## Persistent socket monitors

[`SocketMonitoring`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_monitoring.rs) associates `MonitorRegistration: MonitorState`. Construction takes `(K, Interest)` synchronously and returns `io::Result`:

- `socket_monitor_state` returns embeddable `SocketMonitor<Socket, Registration>`.
- `socket_monitor` allocates `MonitorStorage<SocketMonitor<...>>`.
- `socket_stream_monitor_state` and `socket_stream_monitor` return the corresponding stream-specific state/storage, allowing proven stream-only optimizations.

`socket_monitor_wait(&mut monitor, C)` returns `OperationResult<()>`. `socket_monitor_recv_poll`, `socket_monitor_send_poll`, `socket_monitor_recvmsg_poll`, and `socket_monitor_sendmsg_poll` take the same remaining arguments and return the same results as their borrowed socket counterparts, replacing `K` with `&mut MonitorStorage<M>`. One monitor describes one direction. Separate reader/writer monitors allow concurrent tasks; cloned storage aliases a registration and does not create independent waiter capacity.

`socket_monitor_close(&mut monitor) → io::Result<()>` retires registration without cancellation. No waiter may still be active on that monitor. Once close is first polled, callers stop I/O through it and its aliases; backends without persistent registration need not reject later misuse. Await successful close, drop every monitor handle, then exclusively close the socket. Dropping the last application handle requests background retirement; it is not an awaited retirement barrier.

## Buffer pools and receives

[`BufferPooling::buffer_pool`](https://github.com/core-rs/core-scheduler/blob/main/src/buffer_pooling.rs) takes `BufferPoolOptions { buffers, buffer_size, alignment }` and synchronously returns `io::Result<Pool>`. `buffers` remains an exact shared budget independent of worker count. Alignment is a nonzero power of two; metadata and padding can require additional memory. No registration or per-worker policy is exposed by the portable constructor.

[`BufferPool`](https://github.com/core-rs/core-scheduler/blob/main/src/buffer_pool.rs) is `Clone + Send + Sync + 'static` with `Buffer: ReceiveBuffer`. Its synchronous `capacity`, `buffer_size`, and `alignment` describe the whole pool, not current free capacity. `try_acquire() → Option<Buffer>` never waits. `acquire(C) → Result<Buffer, Cancelled>` waits asynchronously at exhaustion and does not silently grow. Clones share capacity. Acquired buffers expose initialized storage with unspecified previous contents. Application leases, frozen views, and published I/O all count against the budget. Idle demand-driven receivers return their storage before waiting for readiness.

[`SocketReceiving::socket_receiver`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_receiving.rs) takes `(K, Pool, SocketRecvFlags, C)` and asynchronously returns `OperationResult<PooledReceiver>`. Construction is lazy; successful admission permits receive-ahead but does not require it. Peeking is rejected. The associated receiver implements `SocketRecvMsgReceiver<S>` with writable `ReceiveBuffer` payloads.

| Receiver/message hook | Arguments → result | Semantics |
| --- | --- | --- |
| [`SocketRecvMsgReceiver::recv`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_recv_msg_receiver.rs) | `(&mut receiver, &S, C) → OperationResult<Option<Message>>` asynchronously | One owned chunk/datagram; `None` is terminal stream EOF or closed reception. Empty datagrams are `Some`. Stream order and datagram boundaries are preserved. |
| `close` | `(&mut receiver, &S) → io::Result<()>` asynchronously | Stop reception, discard unread queued messages, and await original-request retirement. Delivered messages remain valid. |
| [`SocketRecvMessage::address`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_recv_message.rs) | `(&message) → Option<impl SocketAddressView>` | Borrow source metadata in the backend's representation. Stream chunks may omit it. |
| `payload` | `(&message) → &[u8]` | Exactly completed bytes; excludes metadata and unused capacity. |
| `truncated`, `control_truncated` | `(&message) → bool` | Describe receive truncation, independently of later payload edits. |
| `into_payload` | `(message by value) → Payload: IoBuf` | Transfer ownership of exactly the visible payload range. Address views cannot outlive the transfer. |
| [`SocketRecvMessageMut::payload_mut`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_recv_message_mut.rs) | `(&mut message) → &mut [u8]` | Exclusive parsing/decryption access; separate capability so immutable concrete receivers remain supported. |
| [`SocketAddressView::to_socket_address`](https://github.com/core-rs/core-scheduler/blob/main/src/socket_address_view.rs) | `(&view) → SocketAddress` | Explicit portable endpoint conversion, preserving IPv6 scope and flow information. |
| [`ReceiveBuffer::truncate`](https://github.com/core-rs/core-scheduler/blob/main/src/receive_buffer.rs) | `(&mut buffer, length: usize) → ()` | Narrow the initialized visible prefix without moving storage; an oversized length panics. |
| `freeze` | `(buffer by value) → Frozen: IoBuf + Clone` | End mutable access and create independently retained immutable storage. Clones prevent reuse until the last owner retires. |

Pre-cancellation and cancellation while waiting for pool capacity preserve a pooled receiver. After I/O starts, cancellation may end reception or only the wait; it does not universally establish retirement or guarantee continuation. Successful delivery can win. Once close is first polled, callers stop receiving even if it fails or its wait is dropped. Only close retries remain valid; an incompatible scheduler performs no backend work, but does not authorize resuming reception. For portable cleanup, cancel and await the pending receive, handle any delivered result, await close, then drop the receiver before exclusively closing the socket. Retained messages survive that sequence. A terminal receive error is returned once, followed by `None`; `InvalidInput` and `WouldBlock` reject the attempt without making it terminal.

`SocketReceiver` is the reusable monitor-plus-pool fallback; `OwnedSocketRecvMessage` pairs portable metadata with a payload. `FileReceiver::new(file_monitor, pool)` constructs a sequential file composition. `FileReceiver::recv(&mut self, &S, C) → OperationResult<Option<Buffer>>` returns completed chunks independently of later reads; `close(&mut self, &S) → io::Result<()>` retires accepted work. Drop it before exclusive file close. Returned chunks allow a write to overlap the next read.

The fixed pool policy is unchanged. Worker caches and storage representation can evolve internally while respecting the shared budget; elastic growth would need an explicit future construction/capacity policy.

## Buffer and registration implementation hooks

[`IoBuf`](https://github.com/core-rs/core-scheduler/blob/main/src/io_buf.rs) is an unsafe trait extending `AsRef<[u8]> + Send + 'static`: the exposed initialized allocation, range, and length stay stable while an operation owns it. Moving an owner must not move its bytes. [`IoBufMut`](https://github.com/core-rs/core-scheduler/blob/main/src/io_buf_mut.rs) adds `AsMut<[u8]>` and exclusive writable access to exactly that range. Implementing these for inline byte arrays is invalid. An application-specific lease can retain storage internally without imposing its ownership mechanism on the scheduler.

`IoBuf::slice(self, range)` creates an owned `BufferSlice` without copying bytes; `BufferSlice::into_inner` returns its original owner. `HeapBufferPool` supplies concrete `BufferLease` and `FrozenBuffer` types; `FrozenBuffer::view(range)` retains a subrange. A small view can keep a whole slab alive, so parser compaction/copying policy belongs above the scheduler.

[`IoBufSequence`](https://github.com/core-rs/core-scheduler/blob/main/src/io_buf_sequence.rs) exposes `buffer_count()`, `buffer(index) → &[u8]`, and optional `io_slices() → Option<&[IoSlice]>`. Its unsafe contract preserves membership, order, initialized ranges, and allocation lifetime. [`IoBufSequenceMut`](https://github.com/core-rs/core-scheduler/blob/main/src/io_buf_sequence_mut.rs) adds `buffer_mut(index) → &mut [u8]` and optional `io_slices_mut() → Option<&[IoSliceMut]>`; writable ranges must not overlap. A completion backend must keep any borrowed vector array stable through retirement as well as retain the source owners.

`IoBuffers::new(sequence)` and `IoBuffersMut::new(sequence)` prepare a heap-stable I/O-vector array once. `advance(bytes)` adjusts active ranges without copying payloads; `into_inner()` destroys the vectors and returns the original sequence. Backends can use their own reusable vectors when these optional hooks return `None`.

[`MonitorState::acquire/release`](https://github.com/core-rs/core-scheduler/blob/main/src/monitor_state.rs) track application-handle lifetime separately from backend retention. Hooks must tolerate aliases and concurrent handle creation/destruction. [`MonitorStorage::new`](https://github.com/core-rs/core-scheduler/blob/main/src/monitor_storage.rs) allocates stable storage; `from_arc(owner, projection)` embeds a monitor in application-owned stable connection state. `clone` preserves the handle role. `retained` and `retained_project` retain allocation lifetime without extending the application registration lifetime; `as_ptr` exposes a stable state pointer while that storage owner is retained. These are backend implementation hooks, not permission to dereference stale completion pointers.

`SocketMonitor` exposes its socket, direction, registration, `check_interest`, and `into_parts`; `SocketMonitorState` is the sealed common view accepted by general and stream monitors. `FileMonitor` exposes `file`, `registration`, `registration_mut`, and `map_registration` for backend composition. Such state never replaces the scheduler explicitly supplied to operations.

## Selector integration and concrete runtime lifecycle

These hooks live in `core-scheduler-selector`, not the portable interface. They let the Native executor drive independently selected I/O machinery.

| Hook | Arguments → result | Semantics |
| --- | --- | --- |
| [`Selector::close`](https://github.com/core-rs/core-scheduler/blob/main/crates/selector/src/selector.rs) | `() → ()` synchronously | Stop admission and notify waiters; retain accepted cleanup and kernel-accessible storage. Repeatable. |
| `Selector::shutdown` | `() → ()` asynchronously | Close admission and await all accepted work retiring, including abandoned futures. Keep drivers progressing while waiting. |
| [`SelectorDriver::enter`](https://github.com/core-rs/core-scheduler/blob/main/crates/selector/src/selector_driver.rs) | `(driver by value) → Worker` synchronously | Consume a movable `Send + 'static` driver into its owning worker; thread-bound routing is restored on worker destruction. |
| [`SelectorWorker::waker`](https://github.com/core-rs/core-scheduler/blob/main/crates/selector/src/selector_worker.rs) | `() → Waker` | Interrupt an event wait, including wake/sleep races; harmless after driver retirement. |
| `SelectorWorker::close` | `(&mut worker) → ()` | Begin retirement without synchronously waiting. |
| `SelectorWorker::turn` | `(&mut worker, Option<Duration>) → io::Result<bool>` | Publish a bounded completion batch and wait at most the timeout. Zero is nonblocking; `None` permits indefinite waiting. True means closed and drained. After waking tasks, return so the executor can poll them. |

Drivers publish results and wake tasks; they never poll application futures from completion processing. The executor manages runnable tasks, stealing, worker-local timers, and the timeout passed to `turn`. io_uring uses one ring per worker; active operations stay with their original ring through retirement, while subsequent operations can follow a migrated task.

Constructors, `block_on`, worker counts, `selector()`, concrete admission `close`, and runtime `shutdown` belong to the chosen backend, not a universal scheduler trait. Tokio's adapter borrows an application-owned runtime which must remain alive and driven. Native owns its executor and composes selectors. Direct supplies a reference implementation. An application must keep execution services running while awaiting graceful cleanup.

Concrete io_uring experiments expose `RecvMsgMultishotOptions` and `IoUringSelector::socket_recvmsg_multishot`. Native applications access the selector through `scheduler.selector()` and advance and close that resource with the same selector. The worker-provided pool factory is also backend-specific; neither mechanism is required of other schedulers.
