# Releases

## Unreleased

- **Breaking:** remove the Direct backend, `base-scheduler-direct`; 0.4.0 is its final release. Its blocking pool could stop running jobs on Apple platforms ([#83](https://github.com/base-rs/base-scheduler/issues/83)), and its socket retry pulse spun instead of waiting ([#85](https://github.com/base-rs/base-scheduler/issues/85)). Applications should use `base-scheduler-native` as the general backend, or `base-scheduler-tokio` to run on an existing Tokio runtime.
- Migrate from `DirectScheduler::new(DirectOptions { threads, capacity })` to `NativeScheduler::with_options(NativeOptions { threads, capacity, ..Default::default() })`, or `NativeScheduler::new()` for the defaults. `threads` and `capacity` remain blocking-work limits, but socket I/O no longer counts against them. Direct also used `threads` as its number of task workers, whereas Native defaults `workers` to the available parallelism, so set `workers: threads` to keep the old count. Native's workers drive I/O readiness and timers, so its futures can be polled with `NativeScheduler::block_on`, as spawned tasks, or from another executor; `block_on` panics on a Native worker. `shutdown().await` still retires the scheduler. The published `base-scheduler-direct` 0.4.0 remains available on crates.io but will receive no further releases.
- **Breaking:** add `stall_rescue` to `NativeOptions` and `NativeExecutorOptions`, enabled by default. Construct them with `..Default::default()` to keep compiling when fields are added. With stall rescue, timers no longer wait for a worker that is stuck in a long poll: a monitor thread, or a worker about to go idle, fires them within about two milliseconds on Linux while another worker is idle. A worker counts as stuck once one of its timers stays due for about a millisecond without being collected, and wakes then avoid it. Previously a root sleep routed to a worker blocked for two seconds took the whole two seconds. Readiness events behind a stuck worker are not yet rescued. Blocking work should still use `spawn_blocking`. See [`design/stall-rescue.md`](design/stall-rescue.md).
- Register Native timers from root futures and other executors by locking the chosen worker's queue directly, rather than sending it a command and awaiting an acknowledgment. Such registrations no longer allocate, and waker updates and release no longer wait for that worker.
- Add the `diagnostics` feature to `base-scheduler-native`, with `NativeScheduler::rescue_statistics` counting stall rescues by kind. It is not a stable interface.
- Make bound owned stream reads and writes on the readiness selector (`read_owned`, `write_owned`) as cheap as borrowed ones: they now reuse the reader's or writer's cached registration, wait on cached readiness first once established, and clear readiness after a short read, instead of looking up the registration under a lock and issuing a receive that would fail before every wait. Owned-buffer echo with one worker on Linux epoll handles about 20% more requests per second with about 22% less CPU per request, matching the borrowed path. On Linux, with OOB support disabled (the default), `read_owned` now returns `Unsupported` when epoll reports urgent data, as `read_into` already did.
- Reuse the bound reader's or writer's cached registration on the readiness selector for the remaining stream transfers: pooled `read`, `read_vectored`, `read_vectored_with_flags`, `read_into_with_flags`, `read_owned_vectored` and `write_owned_vectored`. They no longer look up the socket's registration under a lock or repeat admission checks on every call. Their readiness rules are unchanged: each receive still reaches the socket first and keeps readiness until `WouldBlock`. A pooled `read` that would block now returns its slot and waits on cached readiness, instead of polling the socket again before and after the wait. Echo with one worker on Linux epoll handles about 24–32% more requests per second with pooled `read` (25–29% less CPU per request), and about 3–6% more with vectored and owned vectored transfers.
- Keep a Native task woken by a worker's own driver turn on that worker, which owns the readiness registration or ring operation that woke it, instead of sending it to the worker that last polled it. After an idle worker stole a task, each later readiness event of that task was pushed into the thief's inbox, often with a wake. Borrowed-buffer echo on Linux epoll with four workers uses about 2–3% less CPU per request. Wakes from task polls and other threads still prefer the last polling worker. See [`benchmarks/echo/results/driver-wakes`](benchmarks/echo/results/driver-wakes/readme.md).
- Make the Tokio adapter's bound stream reads and writes cheaper. On Linux and Android, a stream reader with OOB support disabled (the default) now clears its observed readiness after a short read, as Tokio's own streams do, instead of issuing a receive that fails with `WouldBlock` before every wait. Its registration also requests priority readiness, and reported urgent data or closure keeps readiness, so bytes after an urgent mark stay readable; a socket that has reported urgent data keeps draining through `WouldBlock`. Reads and writes with `Cancellation::never()` skip the generic cancellable operation: established reads wait in one compact future, and writes make their syscall directly. 64-byte echo with one worker handles about 15–17% more requests per second with about 19% less CPU per request, borrowed and owned, about 4–7% behind raw Tokio; with four workers, CPU per request falls about 9–13%.
- Keep an io_uring socket's persistent readiness poll on the ring that issued it when its task moves to another worker, instead of cancelling it there and registering a new poll on the new worker's ring. With two or more workers, idle workers steal connection tasks often enough that a diagnostic build counted about 0.3–0.5 rebinds per HTTP request, each rebind costing a cross-ring cancellation, an eventfd write, a wake from the old ring and a new poll. The poll's notification now wakes the task from its original ring, which keeps it on that worker, as the readiness selector already does. A hello-world HTTP server with one task per connection on two or four io_uring workers serves 14–25% more requests per second with 15–24% less CPU per request, as fast as the epoll backend. One worker is unaffected.
- Pad Native's runnable counter to its own cache line. Whichever thread wakes a task updates it, and each runnable retirement updates it and then reads the scheduler's closed flag, which shared its line. The timer benchmark's `reuse yield` case with 64 tasks on 4 workers runs about 7% faster (114 to 106 ns per operation), with about the same instruction count.

## v0.4.0

- **Breaking:** add the required `SocketStreamReader::try_read_into`, which attempts one borrowed read now without waiting or consulting cached readiness. Polling `read_into` once with a no-op waker is not equivalent: Native, readiness and Tokio readers that have established progress wait on cached readiness first, so a single poll can miss input or EOF the kernel has already queued. Implementations of `SocketStreamReader` outside this workspace must add the method; wrappers should forward it.
- Use Native's own epoll readiness driver on Android, as on Linux, rather than the portable `polling` fallback. Android stream readers therefore follow the Linux readiness rules: without OOB support, a short receive clears readiness and reported urgent data returns `Unsupported`.
- Fix blocking pools on Apple platforms that could stop running jobs, permanently: the readiness selector's pool, which Native uses, and the Tokio adapter's pool. There, the default pthread condition variable stops waking anyone once one waiter has been passed over for 2^23 newer waits ([ioquatix/macos-pthread-cond-hang](https://github.com/ioquatix/macos-pthread-cond-hang)), which an idle worker in a busy pool can be. Idle workers now wait again every second. The direct scheduler's pool, which stopped that way in [#83](https://github.com/base-rs/base-scheduler/issues/83), is unchanged.
- Fix Tokio adapter socket waits on Windows that could hang although the socket was ready. async-io's IOCP reactor can miss interest registered from another thread ([smol-rs/polling#271](https://github.com/smol-rs/polling/issues/271)); each wait now wakes the reactor after registering interest and retries its syscall at least every 10 seconds.

## v0.3.1

- Add Unix `PipeReading` and `PipeWriting` capabilities which adopt an owned pipe descriptor, such as a child process's standard input or output, as a sequential reader or writer. Direct, readiness, Tokio and Native support them natively; io_uring uses its readiness fallback.

## v0.3.0

- Register child cancellation signals in reusable slots instead of a hash map. Creating and dropping a child no longer hashes, and steady churn does not grow the parent's registry. In the HTTP client benchmark at 32 concurrent requests, this saves about 245 user instructions per response.

- Reduce Native readiness overhead on established stream reads: skip redundant notifications for directions that are already ready, bypass cancellation wrappers for never-cancelled operations, and poll a compact receive future. Native sockets now register with the portable reactor only when a fallback wait first needs it, so `NativeSocket::retain_native` returns the new `RegisteredSocket`, which exposes the socket and its descriptor but not async-io's readiness methods. This is a breaking change for backend integrations, and a reactor registration failure surfaces at that wait.

## v0.2.0

- Correct all socket reader/writer factory lifetime bounds so connections can own sockets alongside their directional handles. Handles retain scheduler and backend access without borrowing the temporary socket reference. Backend implementors must add compatible precise capture bounds (`use<...>`); raise the workspace minimum Rust version to 1.87.

- Publish immediately available io_uring completions after nonblocking submission, reducing delivery latency while preserving cancellation, notification handling and shutdown.

- Batch io_uring completion task work on fresh thread-bound workers with deferred task running. Preserve cooperative and original-ring fallbacks, nonblocking completion progress, cancellation and movable drivers.

- Avoid an extra `WouldBlock` receive after short borrowed scalar stream reads on io_uring when OOB support is disabled. Preserve EOF notifications, racing readiness, cancellation and registration retirement; opt-in OOB and message receives keep conservative retries.

- Add compile-time `SocketStreamReader::with_oob_support` selection, disabled by default. Native Linux epoll avoids an extra receive after ordinary short scalar reads and rejects reported urgent data; opt-in support preserves ordinary-byte progress across urgent marks. Other receive paths retain conservative behavior.

## v0.1.2

- Avoid retrying blocked fallback socket operations on unrelated task wakes, reducing io_uring listener overhead while preserving immediate I/O progress.

## v0.1.1

- Reduce Linux worker-readiness overhead with native epoll event delivery and eventfd interruption, preserving precise timeouts and compatibility fallbacks.

- Drive Native socket readiness on executor workers, with lazy per-socket registration, independent concurrent waiters, and cached stream-read readiness.

- Reduce worker-readiness notification overhead while preserving independent waiters, cancellation, and shutdown wakeups.

- Return successful borrowed stream writes before constructing wait futures; retain lazy registration and ordered owned output.

- Dispatch macOS and Linux/Android readiness events through stable registration pointers, with deferred retirement and failed-registration quarantine.

- Decode macOS kqueue events directly from one reusable native-event vector, removing normalized event staging.

## v0.1.0

- Publish as `base-scheduler` in the `base-rs` organization, with package versions starting at 0.1.0.
- Rename the companion scheduler packages and Rust crate paths to the `base-scheduler` family.

Initial release of the runtime-independent scheduler interface and its seven companion crates. The workspace requires Rust 1.85 or later.

- Define separate capabilities for clocks, task spawning, bounded blocking work, address resolution, signals, process waits, and file/socket lifecycle operations. Protocol libraries depend on the capabilities they use; applications choose a backend.

- Provide scheduler-bound stream, packet, and file readers and writers. Borrowed socket operations reuse caller storage; owned operations retain stable storage through completion, including after their waiting future is dropped. Keep registration, routing, and retirement state inside each backend.

- Make cancellation explicit per operation. Provide owning task handles, bounded admission, independent resource retirement, and asynchronous cleanup. Add `Clock::timeout` with operation-first polling, distinct expiry and clock errors, and no timer registration for immediately ready operations.

- Add reusable buffer pools, unique writable chunks, retained immutable views, and owned scatter/gather storage. Support explicit read/write overlap, ordered owned stream submission and flushing, and separate file acceptance/completion waits.

- Include Direct reference and Tokio adapter backends, plus the Native work-stealing executor with worker-local event loops and timers. Provide independently driven readiness and Linux io_uring selectors, with per-worker rings, reusable request state, and optional fixed-file registrations.

- Support child process status waits, Unix process-group selection and state-change flags, native process notifications, optional io_uring `WAITID`, and bounded blocking fallbacks. A started blocking wait may finish after cancellation and delay shutdown.

- Supply reusable conformance suites, cross-backend I/O examples, and matched network, file, buffer, and timer benchmarks. CI checks Linux, macOS, Windows, the minimum Rust version, and complete measured source-region coverage.

- Keep the reusable timer capability and concrete io_uring multishot receiver API experimental. Portable application code uses the bound reader/writer capabilities.
