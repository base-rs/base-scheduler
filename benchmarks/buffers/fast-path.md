# Receive fast paths

The shared pool now uses atomic availability words instead of a mutex-protected free list. This reduces checkout/return costs while preserving bounded storage, writable ownership, retained immutable views, and zero steady-state allocation. It does not turn the portable pool into a kernel-registered buffer pool.

## Implementation

The ordinary pooled receive path is:

1. Clear one availability bit to acquire exclusive access to its preallocated slot.
2. Attempt a nonblocking receive through the persistent directional monitor. Wait and retry only after `WouldBlock`.
3. Return the completed owner to the parser. It may mutate the chunk, freeze it, retain views, or pass its owner to another operation or worker.
4. Set its availability bit after the final owner releases the chunk. Notify an additional capacity waiter for every return.

A successful acquire synchronizes with the previous owner's release. Atomic read-modify-write operations preserve release sequences when different slots share one availability word. Stable integer slot indices avoid a pointer free-list ABA problem. A partial final word exposes only real slots. Large pools can scan multiple words, and shared ownership and availability cache lines can still contend.

The io_uring multishot path additionally keeps a receive armed with provided buffers. Completed batches can be consumed without registering event listeners. Only an empty-to-nonempty queue transition wakes the consumer. A buffer returning on its ring worker is republished directly, without upgrading the controller's weak reference or cloning another pool owner. Remote returns retain ownership in a command and notify the original ring. The optional local lookup tolerates thread-local teardown; leases remain valid after receiver and scheduler shutdown.

These changes preserve `recvmsg` metadata, including ancillary-data truncation reporting. Replacing stream `RecvMsgMulti` with plain `RecvMulti` would lose that observable behavior, so that specialization was rejected.

## Measurement method

Runtime and benchmark implementation: `f9ea0bd1df983067183d872e25e9419a956cffbd`. The baseline uses runtime source from `6ba13d98663876d21ecf6751035b7e1552936297` with the same corrected timed benchmark implementation. Baseline binaries were built before applying runtime optimizations and retained separately. Every sample records its binary's SHA-256 hash. Rust smoke-test additions made between baseline builds and the final commit do not alter the timed loops.

All builds use Cargo's optimized release profile, the same compiler and lockfile, and no `RUSTFLAGS` override. The [host environment](results/environment.txt) records Aiko's Ryzen 9 9950X3D, Linux, and compiler. Timing runs do not overlap builds, tests, profiling, or each other. These are one-host point estimates; small differences remain sensitive to loopback batching and scheduling.

The original [buffer report](readme.md) incorrectly said Native work ran on workers. `NativeScheduler::block_on` polls its root on the calling thread. Both Native and Tokio measured work now executes in spawned worker tasks. Root execution is valid, but cross-thread dispatch and recycling are different costs from the worker-local path. A separate instrumented check of 32,000 received UDP datagrams reduced multishot eventfd writes from 32,005 outside the worker to four on the worker. Those syscall counts include warmup and instrumentation; their elapsed times are not throughput measurements.

## Pool measurements

The [lease benchmark](src/bin/leases.rs) measures one million checkout/return cycles per thread, with either independent pools or one shared pool. A cycle writes one byte; the retained-view variant also freezes the lease and creates and drops one view. Storage is initialized before timing. Threads synchronize at a barrier, and timing ends after all workers join. The shared pool has at least eight slots, so this isolates ownership bookkeeping without capacity waits. Affinity is CPUs 2–9, with seven matched repetitions in randomized order.

| Threads | Pool | Ownership | Mutex M cycles/s | Bitmap M cycles/s | Matched bitmap/mutex |
| ---: | --- | --- | ---: | ---: | ---: |
| 1 | Independent | Mutable | 23.107 | 26.059 | 1.115× |
| 1 | Independent | Frozen view | 14.751 | 15.975 | 1.086× |
| 1 | Shared | Mutable | 23.138 | 26.152 | 1.129× |
| 1 | Shared | Frozen view | 14.708 | 16.162 | 1.092× |
| 2 | Independent | Mutable | 44.753 | 51.270 | 1.146× |
| 2 | Independent | Frozen view | 29.176 | 31.854 | 1.089× |
| 2 | Shared | Mutable | 7.235 | 13.552 | 1.875× |
| 2 | Shared | Frozen view | 5.464 | 9.753 | 1.783× |
| 8 | Independent | Mutable | 121.418 | 137.646 | 1.125× |
| 8 | Independent | Frozen view | 73.290 | 83.758 | 1.145× |
| 8 | Shared | Mutable | 2.710 | 8.272 | 3.070× |
| 8 | Shared | Frozen view | 2.402 | 5.298 | 2.205× |

The [168 raw samples](results/leases-final.jsonl) show 8.6–14.6% higher cycle throughput without contention. With two threads sharing a pool, the improvement is 1.78–1.88×; with eight, it is 2.20–3.07×. The bitmap removes mutex serialization, while shared cache lines still limit scaling.

This test isolates pool costs; its throughput ratios are not network throughput ratios. It does not establish scaling for arbitrarily large pools or worker counts.

## Single-connection TCP and UDP measurements

The [socket benchmark](src/bin/sockets.rs) uses a standard-library peer on a separate thread. Every sample warms 1,000 batches and measures 10,000, with seven matched repetitions and randomized case and implementation order. Affinity is CPUs 2–5. One or two scheduler workers execute a single connection. This is not a many-connection shared-pool server benchmark.

Two traffic patterns distinguish round-trip and receive-heavy behavior. `echo` sends every payload and receives its echo. `receive` sends one byte of credit and receives a burst of payloads, reducing unrelated per-message sends and peer receives. TCP transfers are 64 bytes or 64 KiB with windows of one and 16 or four, respectively. UDP sends 1,200-byte datagrams with windows of one or 16. TCP boundaries are reconstructed from byte counts; the reported message rate counts fixed-size payload units, not kernel receive calls. UDP boundaries and truncation are checked.

The modes are a reused raw buffer, portable pool, retained frozen view, provided-buffer multishot, and multishot retaining the previous completed lease. All share the same writable monitor. The portable pool has eight slots. Multishot has `max(window + 2, 8)` slots to permit receive-ahead while one chunk is retained; it therefore has more storage in the window-16 cases. The Tokio comparison uses the scheduler adapter with a raw monitor, not bare Tokio. These tests do not include HTTP parsing, TLS, QUIC, NIC zero copy, or packet-loss behavior.

The [1,848 raw samples](results/receive-fast.jsonl) give these matched median ratios across all configurations:

| Comparison | Throughput ratio |
| --- | ---: |
| Portable pool, new/old | 1.000× |
| Retained frozen view, new/old | 0.999× |
| Multishot, new/old | 1.003× |
| Portable pool/native raw | 1.003× |
| Multishot/native raw | 0.991× |
| Multishot/Tokio adapter raw | 0.960× |

Pool bookkeeping is substantially faster in isolation, but ordinary single-connection pooled network I/O is effectively level with raw reuse overall. The multishot changes do not establish a general network speedup.

For 64-byte TCP with a window of one, multishot exceeds the Tokio adapter in each measured worker/traffic combination:

| Workers | Traffic | Multishot/Tokio adapter raw |
| ---: | --- | ---: |
| 1 | echo | 1.125× |
| 1 | receive | 1.142× |
| 2 | echo | 1.114× |
| 2 | receive | 1.174× |

This advantage does not generalize: with a window of 16, 64-byte TCP multishot/Tokio ratios are 0.788–0.966. Other transfer sizes and datagram configurations also frequently favor the adapter. These comparisons identify workload-dependent paths; they do not justify making multishot the universal default. Run the summarizer below for every configuration.

## Shared-pool network measurements

The socket benchmark also accepts a final connection count. Benchmark source: `1f6bc7e2cf2b83bc2bc98737acc2714849c9f6e3`; runtime source remains `f9ea0bd`. Each connection runs in a separate scheduler task with its own standard-library peer thread. All pooled receivers share one pool containing eight slots per connection. An asynchronous warmup barrier publishes one common start time; aggregate throughput counts every connection's messages and ends at the last measured completion. Peer threads share the workers' CPU affinity. Earlier-finishing connections may clean up while others finish timing. These tests include scheduling and loopback peer contention, not isolated server CPU cost.

Both binaries use this same benchmark and runtime source except that the mutex baseline replaces `src/buffer_pool.rs` with its version from `6ba13d9`. This isolates the pool changes from other multishot optimizations. There are no capacity waits in this generously sized pool. Binary identities are recorded in every row. The raw-buffer mode does not use pool checkout/return and serves as a noise control.

The [336-sample matrix](results/server-fast.jsonl) uses seven matched repetitions of one, eight, or 32 connections and one, two, or eight workers, CPU affinity 2–9, a receive window of eight, and 1,000 warmup batches per connection. Timed batches per connection are `max(1000, 10000 / connections)`. Ratios are medians of matched bitmap/mutex throughput:

| Workers | Connections | Transport | Raw control new/old | Pool new/old | Retained view new/old |
| ---: | ---: | --- | ---: | ---: | ---: |
| 1 | 1 | TCP | 0.991× | 1.011× | 1.003× |
| 1 | 1 | UDP | 1.003× | 0.991× | 0.996× |
| 2 | 8 | TCP | 0.982× | 0.976× | 1.061× |
| 2 | 8 | UDP | 1.026× | 0.971× | 1.027× |
| 8 | 8 | TCP | 0.985× | 1.017× | 1.069× |
| 8 | 8 | UDP | 1.032× | 1.044× | 1.038× |
| 8 | 32 | TCP | 0.993× | 1.228× | 1.448× |
| 8 | 32 | UDP | 1.032× | 1.060× | 1.321× |

The [longer contention validation](results/server-contention.jsonl) repeats only 32 connections on eight workers, with 5,000 timed batches per connection and 11 matched repetitions (132 samples).

| Transport | Receive mode | Mutex M messages/s | Bitmap M messages/s | Matched bitmap/mutex |
| --- | --- | ---: | ---: | ---: |
| TCP | Raw | 3.301 | 3.315 | 0.976× |
| TCP | Pool | 2.529 | 3.188 | 1.283× |
| TCP | Retained view | 2.315 | 2.914 | 1.291× |
| UDP | Raw | 4.303 | 3.864 | 0.981× |
| UDP | Pool | 2.298 | 3.544 | 1.474× |
| UDP | Retained view | 2.069 | 3.088 | 1.485× |

The longer run retains a 28–29% TCP throughput improvement and a 47–49% UDP improvement for pooled receives under contention. The raw controls are about 2% slower, so the measured pool gain is not explained by a faster run of the whole binary. Throughput columns are independent medians; the last column is the median of per-repeat ratios, which can differ from the ratio of those medians. These are point estimates from one loopback host, not promises for every connection count, machine, or protocol.

Together, the pool-cycle and concurrent-network measurements support the atomic availability bitmap as a useful default for shared pools. They do not establish that multishot is a better default: the single-connection matrix still shows workload-dependent wins and losses.

## Interpretation

The shared pool removes avoidable serialization from the ownership path without changing callers. The concurrent-connection tests exercise that improvement in network I/O, rather than assuming a pool-cycle gain automatically improves a single connection. A raw buffer already reused by one connection has no payload allocation to eliminate, so it remains an important control. Kernel receive-ahead can save receive syscalls, but adds completion processing and a queue; use measured workload behavior rather than assuming that registration or multishot always wins.

The portable associated receiver remains syscall-first. Its fast path is one available-slot acquisition, a direct nonblocking `recvmsg` through the persistent monitor, application access to the returned owner, and an atomic slot return after the last owner releases it. Readiness registration and capacity waiters are used only when needed. Explicit multishot receivers still own their separate provided-buffer pools. Sharing provided buffers between connections on a ring, integrating them with the writable/frozen lease model, and sharing a server's global budget across ring-owned pools remain implementation gaps. The current work does not silently allocate a second pool and call it the supplied shared pool.

## Reproduce

Build both runtime versions with identical benchmark sources. For the baseline, copy `benchmarks/buffers/src` from the implementation commit into a separate checkout at `6ba13d9`. Keep its release `sockets` and `leases` binaries before building the optimized version. No special compiler flags are needed.

```sh
cargo build --release --locked -p core-scheduler-buffer-benchmark
python3 benchmarks/buffers/receive_fast.py \
    --binary target/release/sockets --baseline /path/to/baseline/sockets \
    --repeats 7 --output benchmarks/buffers/results/receive-fast.jsonl
python3 benchmarks/buffers/lease_fast.py \
    --after target/release/leases --before /path/to/baseline/leases \
    --output benchmarks/buffers/results/leases-final.jsonl
python3 benchmarks/buffers/summarize_fast.py benchmarks/buffers/results/receive-fast.jsonl
python3 benchmarks/buffers/summarize_fast.py benchmarks/buffers/results/leases-final.jsonl
```

For the shared-connection comparison, build `1f6bc7e` twice, replacing only `src/buffer_pool.rs` with its `6ba13d9` version for the mutex binary. Keep the resulting binaries separately, then run:

```sh
python3 benchmarks/buffers/server_fast.py \
    --after /path/to/bitmap/sockets --before /path/to/mutex/sockets \
    --output benchmarks/buffers/results/server-fast.jsonl
python3 benchmarks/buffers/server_fast.py \
    --after /path/to/bitmap/sockets --before /path/to/mutex/sockets \
    --contention --repeats 11 --output benchmarks/buffers/results/server-contention.jsonl
python3 benchmarks/buffers/summarize_fast.py benchmarks/buffers/results/server-fast.jsonl
python3 benchmarks/buffers/summarize_fast.py benchmarks/buffers/results/server-contention.jsonl
```

Example individual cases:

```sh
target/release/sockets iouring multi 64 10000 1 tcp 1 receive
target/release/sockets iouring pool 1200 10000 2 udp 16 echo
target/release/leases 8 1000000 shared frozen
```

Linux and macOS workspace tests pass with 100% measured region coverage (27,541/27,541 on Linux and 21,779/21,779 on macOS). Pool ownership tests pass Miri, including concurrent checkout/return, frozen views, cross-thread release, exhaustion, and word boundaries. Clippy, documentation builds, and the macOS MSRV check pass. Independent review approved the implementation. Windows remains for CI.
