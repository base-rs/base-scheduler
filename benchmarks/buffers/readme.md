# Shared buffers and overlapping I/O

Use bounded reusable storage and independently owned completed chunks. For a file pipeline, poll the output write and next input read concurrently. Separating submission from completion is useful when admission itself matters, but it is not a generally faster replacement for joining ordinary operations.

The prototype implements these choices across Direct, the Tokio adapter, Native readiness, and Native io_uring. It does not yet connect the shared pool to io_uring registered buffers or provided-buffer rings.

Subsequent experiments measure [receive fast paths and shared-pool contention](fast-path.md) and [fixed registered buffers](registered.md). The latter isolates ordinary versus fixed io_uring reads/writes with the same leases, rather than integrating registration into the scheduler.

## Ownership and use

`BufferPool::new(count, size)` reserves stable initialized storage. Acquisition returns a unique writable `BufferLease`; pool exhaustion waits with cancellation. A file or socket receiver returns completed leases independently of the receiver, allowing subsequent receives while application code retains earlier chunks.

Parse or decrypt in place while the lease is unique. `freeze()` consumes writable access and produces a cloneable immutable `FrozenBuffer`; `view(range)` retains a zero-copy range. The final lease or view returns the slot. Setup allocates storage and bookkeeping once; the allocation test checks that steady-state acquisition, freeze, views, and return allocate nothing.

A parser owns framing and compaction: retain views of complete frames and copy a partial tail when contiguous storage is needed. Retaining one small view keeps its entire slot unavailable, so selectively copying long-lived fragments can reduce memory pressure. The receiver does not hide a growing receive queue.

`SocketReceiving::socket_receiver` is the backend-associated construction interface using a supplied pool. Its completed buffers implement `ReceiveBuffer`, allowing a future backend to specialize storage without changing generic consumers. Current implementations use persistent readable monitors and syscall-first borrowed message receives. Existing explicit multishot receivers retain their own separate provided-buffer pools.

For a shared pool, drive both branches and release each completed write's returned owner inside that branch:

```rust,ignore
let (written, next) = futures_lite::future::zip(
    async {
        let (result, buffer) =
            file_write_all(scheduler, output, buffer, cancellation).await;
        drop(buffer);
        result
    },
    input.recv(scheduler, cancellation),
).await;
written?;
let next = next?;
```

Two slots suffice for an isolated read/write pipeline. A server-shared pool or retained parser views need an explicit capacity policy. Waiting for another input buffer before collecting an output completion can deadlock if that completion owns the only slot the read needs. Joining alone is insufficient if the completed write branch retains its owner until the read also finishes.

`FileSubmission` provides distinct acceptance and completion waits for owned reads/writes, including positioned variants:

```rust,ignore
let completion = scheduler
    .file_write_submit(output, buffer, cancellation)
    .await
    .map_err(|(error, _buffer)| error)?;

// Accepted work can progress while this completion is not being polled.
let (result, buffer) = completion.await;
```

Admission means bounded backend acceptance, not necessarily kernel submission or durability. The completion returns the original owner on success, error, or cancellation. Abandoning it never releases storage still accessed by a worker or the kernel. `file_write_all_submit` accepts the first transfer; after a short write, further retries require polling its completion. It does not eagerly run a whole write loop in a hidden task.

## Aiko measurements

**Execution-context correction:** The original samples below ran Native and Tokio operations in root futures polled by `block_on`, outside their task workers. An earlier version of this report incorrectly stated that Native/io_uring work ran as worker tasks. These measurements remain records of external-root execution, including its cross-thread dispatch and buffer returns; they do not establish the worker-local fast path. The corrected worker benchmarks, contention measurements, and resulting implementation changes are documented in [Receive fast paths](fast-path.md).

The [raw samples](results/aiko.jsonl) contain 1,218 measurements from runtime/benchmark commit `3b2e5418772db12f98a91524c0158549fdc70009`. Subsequent changes generalize conformance tests, format the test macro, and document the measurements; they do not change timed runtime paths. [Environment details](results/environment.txt) record the machine and compiler.

All binaries use Cargo's optimized release profile. Each matched group randomizes mode order, and group order is randomized with a fixed seed. Seven repeats use CPU affinity 2–5 on Aiko's Ryzen 9 9950X3D. No other builds or benchmark runs were started during timing. Ratios below are medians of per-repeat matched ratios, rather than ratios of independently aggregated medians. They remain point estimates from one host, not confidence bounds.

### Files

Each sample copies 64 MiB with 4, 64, or 128 KiB chunks. An untimed warm pass precedes the measured pass. Input and prepopulated output pages are cached; full contents are verified after timing. The output is not synchronized to durable storage. Two vector-buffer modes provide serial and joined controls; three pooled modes provide serial, joined, and explicitly submitted copies. Native/io_uring work in this original dataset runs on the calling thread and dispatches to the ring workers.

Median throughput at **64 KiB, one worker**, GiB/s:

| Backend | Pooled serial | Pooled joined | Pooled submitted | Matched joined/serial |
| --- | ---: | ---: | ---: | ---: |
| Direct | 4.642 | 6.034 | 6.104 | 1.286× |
| Native readiness | 4.969 | 8.443 | 8.577 | 1.699× |
| Tokio adapter | 5.121 | 8.807 | 8.671 | 1.706× |
| Native io_uring | 3.804 | 4.944 | 4.872 | 1.302× |

At **64 KiB, two workers**, GiB/s:

| Backend | Pooled serial | Pooled joined | Pooled submitted | Matched joined/serial |
| --- | ---: | ---: | ---: | ---: |
| Direct | 4.841 | 8.249 | 8.296 | 1.677× |
| Native readiness | 5.113 | 8.688 | 8.770 | 1.745× |
| Tokio adapter | 5.297 | 8.464 | 8.600 | 1.629× |
| Native io_uring | 3.613 | 6.365 | 5.889 | 1.752× |

The overlap is valuable, while the explicit acceptance boundary adds little at 64 KiB. It can hurt small-operation throughput: at 4 KiB, submitted/joined ratios range from 0.747 to 1.011 across these configurations. At 64 KiB, io_uring with two workers is 0.925. Joining starts both operations in one parent poll; awaiting submission and only polling the read's result changes batching and wakeup opportunities. These measurements establish the difference, not a profile attributing its exact cost.

Pool ownership also has a measurable cost in some cases. Across file configurations, matched pooled-joined/vector-joined median ratios range from 0.936 to 1.044. At 64 KiB the ratios are 0.946 for one-worker io_uring and 0.936 for two-worker Direct. Do not infer universally zero overhead from the allocation-free payload lifecycle. The pool adds a free-list mutex and ownership atomics; the benchmark does not isolate each instruction's contribution.

### TCP and UDP

The peer is a standard-library socket on a separate thread. TCP exchanges are 64 bytes or 64 KiB; UDP datagrams are 1,200 bytes. Each sample warms 1,000 round trips, then measures 10,000. All modes use the same persistent writable monitor. The receive choices are a raw readable monitor, a pooled receiver, or a pooled receiver that freezes and retains a view until the next receive. TCP partial transfers are completed; UDP message boundaries are preserved.

Across matched network samples, median pooled/raw throughput is **0.992**, and retained-view/raw throughput is **0.997**. Individual configuration medians vary: pooled/raw ranges from 0.944 to 1.103, and retained/raw from 0.949 to 1.044. The largest apparent speedups illustrate loopback noise and batching sensitivity; they do not establish that pool bookkeeping makes a syscall faster. Two-worker Native 64-byte TCP is around 5% slower for both pooled modes. The initial exploratory run showed a similar small-message gap.

The transport patterns exercise reusable storage and retained chunks. They do not implement H1/H2 framing, QUIC, TLS, or actual HTTP traffic. This run does not compare bare Tokio, test packet loss, or measure many connections contending for one shared pool. The pool's contended scaling and backend-specific kernel registration remain important follow-up measurements.

## Recommendation

Keep the simple pool and owned-chunk interface, and make joined file pipelines the ordinary example. Keep explicit submission optional for control over bounded acceptance, lifecycle, or polling order. Avoid exposing registration decisions in protocol code before measuring a concrete specialized backend.

The next performance experiment should connect the backend-associated receiver to a ring-owned shared provided-buffer strategy, preserving independent writable leases and delayed cross-worker returns. File fixed-buffer registration additionally requires retaining backing allocations through registration retirement. Neither optimization follows merely from reusing a stable address.

## Reproduce

On Linux with io_uring enabled, from the repository root:

```sh
cargo build --release --locked -p core-scheduler-buffer-benchmark
python3 benchmarks/buffers/measure.py --repeats 7 --cpus 2-5 \
    --source "$(git rev-parse HEAD)" --output benchmarks/buffers/results/local.jsonl
python3 benchmarks/buffers/summarize.py benchmarks/buffers/results/local.jsonl
```

Example individual cases:

```sh
target/release/files iouring join 65536 67108864 1
target/release/files iouring submit 65536 67108864 1
target/release/sockets iouring pool 65536 10000 1 tcp
target/release/sockets iouring shared 1200 10000 2 udp
```

Direct, Native readiness, and the Tokio adapter are also supported on macOS and Windows by the Rust binaries. The matched measurement script uses Linux `taskset` and includes io_uring. Binary smoke tests verify copying partial tails and TCP/UDP behavior on each available backend. Conformance tests verify separate admission, independent progress before polling completion, retained owners, cancellation, and safe abandonment; deterministic blocking-pool tests additionally exercise forgotten completions and capacity retirement. Miri checks the pool's unsafe aliasing and reuse model. Linux and macOS workspace coverage reached 100% of measured source regions; Windows validation remains for CI.
