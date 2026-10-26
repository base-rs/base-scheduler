# Ordered stream write benchmark

This benchmark compares ordinary awaited writes with an owning ordered writer. Each connection sends a one-byte request and receives a configured number of fixed-size response chunks. Every server fills the same payload bytes using the same deterministic computation, and the client validates every byte against chunk-specific expected contents. Requests alternate between two payload patterns, catching reuse of the previous response as well as chunk reordering. The ordered server flushes before processing the next request. Receiving the complete payload does not establish that the server has collected its completion or finished that flush; RTT measures client-visible response completion.

The server variants are raw Tokio, Tokio adapter with persistent borrowed monitors, Tokio adapter with an ordered writer, Native readiness with persistent monitors, Native io_uring with persistent monitors, and Native io_uring with an ordered writer. Every variant uses the same portable `HeapBufferPool` with two buffers per connection. Pool setup, task construction, socket registration, and expected-payload generation happen before measurement. No payload buffer allocation occurs in the timed loop.

One response chunk with zero extra generation work measures the immediate path without useful overlap. Multiple chunks allow the application to prepare the next buffer after handing the preceding one to the writer. `WORK` adds a deterministic dependency chain per cache line to model generation or transformation; its result affects the transmitted bytes. This is synchronous computation on the connection task, with no artificial yielding. An io_uring implementation that defers submission until the worker boundary will therefore not necessarily start its queued operation during this computation.

Payload generation uses one non-inlined function shared by all backend futures. Initial measurements with inlined generation changed materially after adding a benchmark control, despite unchanged writer code. Those archived screens describe the original binaries; final comparisons use the shared helper to reduce backend-specific code-generation differences in the synthetic application work.

Monitor backends also accept an `-alternating` suffix. This control holds the last completed lease until after acquiring the next lease, forcing two-buffer alternation without overlapping any I/O. It distinguishes the cache cost of using both buffers from the cost of asynchronous write dispatch. Both controls still reserve exactly two buffers; the ordinary monitor can reacquire the same hot slot immediately.

`SEND_BUFFER=0` preserves the operating system's default send-buffer size. `SEND_BUFFER=65536` with 256 KiB chunks exercises short writes and backpressure. Linux may adjust this value; the server records the actual value. The initial 4 KiB send-buffer smoke test encountered delayed-ACK stalls and is unsuitable for measuring scheduler costs. A constrained send buffer is a stress control, not a claim about typical HTTP deployment settings. All sockets enable `TCP_NODELAY` before registration. These are transport and ownership experiments, not HTTP or TLS implementations.

## Build and run

```sh
cargo build --release --locked -p base-scheduler-write-benchmark
target/release/base-scheduler-write-benchmark server iouring-writer 1 1 262144 8 8 65536
target/release/base-scheduler-write-benchmark client 127.0.0.1:PORT 4 1 262144 8 8 1 3
```

The server prints its bound address. Arguments after the backend/address are worker count, connection count, bytes per chunk, chunks per response, and generation work per cache line. The server's final argument is requested send-buffer size; the client's final two arguments are warmup and measurement durations in seconds.

Run `python3 benchmarks/writes/run.py --output RESULTS.json` on aiko.local for a randomized five-repetition screen. The script serializes work through `/tmp/base-scheduler-perf.lock`, pins servers to CPU 0 or CPUs 0–3 and the unchanged four-worker client to CPUs 4–7, and records all samples, source/binary identities, CPU information, and effective socket-buffer sizes. It does not change host power policy. Use `--suite immediate` or `--suite overlap` for focused comparisons, or repeat `--case workers,connections,bytes,chunks,work,send_buffer` to select exact cases. Freeze a client binary with `--client-binary` when comparing implementation revisions.

`--variant LABEL=BACKEND=SERVER_BINARY` adds a named isolated server implementation; include the label in `--backends` to randomize it with ordinary controls. The client remains frozen, and the report records each variant's binary hash and actual backend argument.

`--perf-stat DIRECTORY` records diagnostic cycles, instructions, cache misses, ring entries, direct sends, and io_uring `SEND` submissions. The last counter filters `io_uring_submit_req` to opcode 26, distinguishing actual asynchronous writes from readiness operations used by the reader. Use zero warmup for these counter runs so normalization approximately matches the measured response count; startup and the final boundary-crossing response still contribute to counters. Instrumentation changes scheduling and throughput, so these samples are separate from capacity comparisons.

Results count only complete requests wholly within the shared measurement window. Payload throughput includes response bytes only. RTT includes request transmission, payload generation, all response chunks, and client byte validation. Report distributions and absolute throughput alongside relative changes; a reduction in application waiting does not by itself establish a throughput improvement. An ordered writer that queues work, wakes another task, or enters the kernel more often can lose to direct synchronous sends when the socket is immediately writable.

See [the prototype research report](results/readme.md) for retained measurements, implementation tradeoffs and the current recommendation.
