# Ordered stream write benchmark

This benchmark compares ordinary awaited writes with an owning ordered writer. Each connection sends a one-byte request and receives a configured number of fixed-size response chunks. Every server fills the same payload bytes using the same deterministic computation, and the client validates every byte against chunk-specific expected contents. Requests alternate between two payload patterns, catching reuse of the previous response as well as chunk reordering. The request/response boundary ensures a complete response, including the ordered writer's final `flush`, before the next request.

The server variants are raw Tokio, Tokio adapter with persistent borrowed monitors, Tokio adapter with an ordered writer, Native readiness with persistent monitors, Native io_uring with persistent monitors, and Native io_uring with an ordered writer. Every variant uses the same portable `HeapBufferPool` with two buffers per connection. Pool setup, task construction, socket registration, and expected-payload generation happen before measurement. No payload buffer allocation occurs in the timed loop.

One response chunk with zero extra generation work measures the immediate path without useful overlap. Multiple chunks allow the application to prepare the next buffer after handing the preceding one to the writer. `WORK` adds a deterministic dependency chain per cache line to model generation or transformation; its result affects the transmitted bytes. This is synchronous computation on the connection task, with no artificial yielding. An io_uring implementation that defers submission until the worker boundary will therefore not necessarily start its queued operation during this computation.

`SEND_BUFFER=0` preserves the operating system's default send-buffer size. `SEND_BUFFER=4096` requests a small socket send buffer to exercise short writes and backpressure. Linux may adjust this value; the server records the actual value. A small send buffer is a stress control, not a claim about typical HTTP deployment settings. All sockets enable `TCP_NODELAY` before registration. These are transport and ownership experiments, not HTTP or TLS implementations.

## Build and run

```sh
cargo build --release --locked -p core-scheduler-write-benchmark
target/release/core-scheduler-write-benchmark server iouring-writer 1 1 65536 8 8 4096
target/release/core-scheduler-write-benchmark client 127.0.0.1:PORT 4 1 65536 8 8 1 3
```

The server prints its bound address. Arguments after the backend/address are worker count, connection count, bytes per chunk, chunks per response, and generation work per cache line. The server's final argument is requested send-buffer size; the client's final two arguments are warmup and measurement durations in seconds.

Run `python3 benchmarks/writes/run.py --output RESULTS.json` on aiko.local for a randomized five-repetition screen. The script serializes work through `/tmp/core-scheduler-perf.lock`, pins servers to CPU 0 or CPUs 0–3 and the unchanged four-worker client to CPUs 4–7, and records all samples, source/binary identities, CPU information, and effective socket-buffer sizes. It does not change host power policy. Use `--suite immediate` or `--suite overlap` for focused comparisons, or repeat `--case workers,connections,bytes,chunks,work,send_buffer` to select exact cases. Freeze a client binary with `--client-binary` when comparing implementation revisions.

Results count only complete requests wholly within the shared measurement window. Payload throughput includes response bytes only. RTT includes request transmission, payload generation, all response chunks, and client byte validation. Report distributions and absolute throughput alongside relative changes; a reduction in application waiting does not by itself establish a throughput improvement. An ordered writer that queues work, wakes another task, or enters the kernel more often can lose to direct synchronous sends when the socket is immediately writable.
