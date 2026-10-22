# Common pool contract prototype

The common pool API works with backend-associated implementations and independently defined lease types. Acquisition, freezing, and retained views remain allocation-free after setup. This is an interface and ownership prototype: all current production backends select the portable heap pool. Fixed registration, shared provided-buffer rings, and zero-copy send selection remain separate backend integration work.

## Application interface

```rust,ignore
let pool = scheduler.buffer_pool(BufferPoolOptions::new(128, 64 * 1024))?;
let mut receiver = scheduler
    .socket_receiver(&socket, pool.clone(), SocketRecvFlags::default(), cancellation)
    .await?;
if let Some(message) = receiver.recv(scheduler, cancellation).await? {
    let frozen = message.buffer.freeze();
    let frame = frozen.clone().slice(..);
    // Retain or forward either owner; storage returns after its final owner retires.
    drop((frame, frozen));
}
receiver.close(scheduler).await?;
```

`BufferPool` describes bounded acquisition and owns an associated `ReceiveBuffer` representation. `BufferPooling` constructs an associated pool; Native forwards construction to its selected selector. Direct, Tokio, Readiness, and io_uring currently return `HeapBufferPool`. The previous concrete `BufferPool::new` constructor is now `HeapBufferPool::new`; portable application code can use `scheduler.buffer_pool` instead.

Options specify exact usable buffer count and size, plus minimum power-of-two alignment. The heap fallback uses stable initialized allocations with at most `alignment - 1` padding bytes per slot. Alignment does not require kernel registration. The options have no registered/unregistered switch; any later policy must distinguish fixed registration from provided-buffer publication and preserve portable ownership semantics during fallback.

`SocketReceiver` and `FileReceiver` are generic over the pool contract. They return the implementation's owned buffer, narrow its completed prefix through `ReceiveBuffer::truncate`, and allow in-place parsing or decryption before freezing. The generic paths use static dispatch. The existing bitmap, shared pool owner, per-slot reader counts, and retirement rules are retained.

## Correctness and coverage

- macOS workspace/all-target/all-feature coverage: **22,000 / 22,000** measured regions; 3 valid unreachable regions excluded.
- Linux workspace/all-target/all-feature coverage: **30,041 / 30,041** measured regions; 8 valid unreachable regions excluded.
- Workspace Clippy with all targets/features, formatting, and diff checks passed.
- Generic pool operations allocate zero times after setup, including with a distinct pool and lease implementation.
- Alignment, invalid sizes, padded allocation overflow, shortened/frozen views, cross-thread returns, cancellation under exhaustion, and backend shutdown admission are covered.
- Shared backend conformance creates pools through the selected scheduler and exercises bounded acquisition and retained views. Root file/socket receiver tests use an independently defined pool and lease representation.

Coverage uses the shared Bake gate. Linux uses the existing matching LLVM tools and wrapper for remapped standard-library sources, as in the preceding pipeline experiment. Tests and builds held the shared performance lock; all timings held it exclusively.

## Optimized measurements

**864 samples:** 600 broad comparisons, 220 longer validation samples, and 44 identical-binary control samples. Ratios below are contract / concrete-baseline throughput. Each cell uses randomized matched pairs; aggregate ratios are geometric means of cell median ratios. These are exploratory measurements, not statistical confidence bounds.

| Broad workload | Cells | Pairs per cell | Aggregate ratio |
| --- | ---: | ---: | ---: |
| files | 16 | 5 | 0.990× |
| leases | 8 | 5 | 0.992× |
| sockets | 36 | 5 | 0.954× |

The broad sweep covers one/eight-thread mutable and frozen pool operations; native, Tokio, and io_uring TCP/UDP receives with one connection/worker or eight connections/two workers; and Direct, Native, Tokio, and io_uring file pipelines at 64/128 KiB. The network harness uses eight-message bursts, including retained parser views. File measurements are page-cache-hot copies, not direct I/O or durability measurements.

Longer validation targeted noisy or slower cells, with eleven matched pairs each:

| Workload | Median ratio | Pair range |
| --- | ---: | ---: |
| `files tokio join 65536 536870912 1` | 1.009× | 0.763–1.303× |
| `files tokio submit 65536 536870912 1` | 0.936× | 0.594–1.550× |
| `leases 1 5000000 local mutable` | 0.986× | 0.972–1.000× |
| `leases 8 1000000 local frozen` | 1.014× | 0.652–1.400× |
| `sockets iouring pool 1200 20000 2 udp 8 receive 8` | 0.996× | 0.422–2.488× |
| `sockets iouring pool 65536 20000 2 tcp 8 receive 8` | 1.003× | 0.426–2.111× |
| `sockets iouring shared 1200 20000 2 udp 8 receive 8` | 1.040× | 0.419–2.220× |
| `sockets native shared 64 20000 1 tcp 8 receive 1` | 1.013× | 0.917–1.195× |
| `sockets native shared 64 20000 2 tcp 8 receive 8` | 0.816× | 0.427–2.367× |
| `sockets tokio shared 64 20000 2 tcp 8 receive 8` | 0.995× | 0.469–2.185× |

The single-thread mutable pool case measured about 1.4% lower throughput; the shared contract also added alignment handling to the heap implementation, so this comparison does not isolate trait abstraction alone. Longer io_uring receive medians range from approximately unchanged to 4% higher. Native multiconnection TCP and Tokio submitted file writes retain lower medians and very wide ranges; their performance neutrality is not established.

For those two cases, the control compares two invocations of the **same contract executable**, under the same two labels and randomized order. There is no source or binary difference:

| Identical-binary workload | Median ratio | Pair range |
| --- | ---: | ---: |
| `files tokio submit 65536 536870912 1` | 1.022× | 0.626–1.173× |
| `sockets native shared 64 20000 2 tcp 8 receive 8` | 1.037× | 0.436–2.309× |

These controls reproduce the large timing swings without a code change. They demonstrate substantial harness/runtime variability; they do not prove that every before/after difference is noise. The common contract is suitable for continued implementation experiments, but these data do not support a blanket claim of unchanged network performance or a networking speedup.

## Reproduction and provenance

Baseline source is commit `7593620`; remote Rust/Cargo sources were verified byte-for-byte against it before the baseline build. The measured contract source hashes, compiler, CPU/kernel details, and executable hashes are recorded in [the environment manifest](results/pool-contract-environment.json). All 466 measured Rust/Cargo/root-readme sources matched the local implementation. No Rust source changed after the measured build.

Both builds use `cargo build --release --locked -p core-scheduler-buffer-benchmark --bins`. Host: Aiko, AMD Ryzen 9 9950X3D, Linux 7.2.6, Rust 1.99.0 / LLVM 23.1.1. Each benchmark process and its threads use CPUs 2–9; peer and scheduler threads share that CPU set. There are no concurrent task-owned builds, tests, profiling sessions, or timed benchmarks.

```sh
flock -x /tmp/core-scheduler-perf.lock python3 benchmarks/buffers/pool_contract.py \
  --before /home/samuel/.cache/core-scheduler-pool-contract/before \
  --after /home/samuel/.cache/core-scheduler-pool-contract/after \
  --output benchmarks/buffers/results/pool-contract.jsonl
```

Long validation adds `--validation --repeats 11` and writes a separate results file. The identical-binary control uses the `after` directory for both arguments and filters with `--match '^(sockets native shared 64 20000 2|files tokio submit)'`.

Raw samples and regenerated summaries: [broad](results/pool-contract.jsonl), [validation](results/pool-contract-validation.jsonl), [control](results/pool-contract-control.jsonl), [broad summary](results/pool-contract-summary.json), [validation summary](results/pool-contract-validation-summary.json), [control summary](results/pool-contract-control-summary.json). Use `summarize_pool_contract.py INPUT --output OUTPUT` to regenerate each summary.
